use std::mem::size_of;

use crate::pool::CardPool;
use crate::types::{LiveSkillOrder, LiveType, ScoreTarget, DECK_SIZE};

use super::context::SearchContext;
use super::evaluate::calc_mysekai_internal;

/// 已选角色集合。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UsedSet {
    bits: u32,
}

impl UsedSet {
    /// 创建空集合。
    #[inline(always)]
    pub const fn new() -> Self {
        Self { bits: 0 }
    }

    /// 判断角色是否已使用。
    #[inline(always)]
    pub fn contains(&self, char_id: u8) -> bool {
        self.bits & (1u32 << char_id) != 0
    }

    /// 插入一个角色。
    #[inline(always)]
    pub fn insert(&mut self, char_id: u8) {
        self.bits |= 1u32 << char_id;
    }

    #[inline(always)]
    pub(crate) const fn bits(&self) -> u32 {
        self.bits
    }
}

/// DFS 中间节点的可加分量摘要。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PartialDeck {
    pub power: u32,
    pub skill: u32,
    pub bonus: u32,
    pub max_skill: u8,
    pub limited_count: u8,
}

/// 角色感知后缀上界。
#[derive(Clone, Debug, PartialEq)]
pub struct SuffixBound {
    target: ScoreTarget,
    effective_live_type: LiveType,
    has_event: bool,
    music_rate_pct: u32,
    boost_rate_pct: u32,
    /// base_rate × 1_000_000, ceil。
    base_rate_1m: i64,
    /// (skill_rate_sum / 500) × 1_000_000, ceil。
    srs_div500_1m: i64,
    /// Solo/Auto + Average：前 5 个 rate 的和 × 1_000_000, ceil。
    avg_sum5_1m: i64,
    /// Solo/Auto + Average：leader 追加 slot 的 rate × 1_000_000, ceil。
    avg_leader_rate_1m: i64,
    /// 5 × multi_teammate_score_up（Multi/Cheerful 专用）。
    teammate_su_5x: i64,
    /// Multi: 75_000 (= 0.075 × 1M), 其他: 0。
    active_1m_coeff: i64,
    other_score: i32,
    /// Cheerful: 5750 + clamp(life, 500, 1000), 其他: 0。
    life_rate_num: i32,
    multi_teammate_power: Option<i32>,
    live_skill_order: LiveSkillOrder,
    is_world_bloom: bool,
    is_final_chapter: bool,
    limited_bonus_cap: usize,
    extra_bonus_ub: u32,
    diff_attr_bonus: [u16; 6],
    support_cards: Vec<(u16, f64)>,
    support_count: usize,
    honor_bonus: u32,
    power_total_cap: Option<u32>,
    power_order: [u8; CHAR_MASK_COUNT],
    power_vals: [u32; CHAR_MASK_COUNT],
    skill_order: [u8; CHAR_MASK_COUNT],
    skill_vals: [u16; CHAR_MASK_COUNT],
    bonus_order: [u8; CHAR_MASK_COUNT],
    bonus_vals: [u16; CHAR_MASK_COUNT],
    dense_bonus_tail: Vec<[u32; DECK_SIZE + 1]>,
    dense_base_bonus_tail: Vec<[u32; DECK_SIZE + 1]>,
    dense_limited_bonus_tail: Vec<[u32; DECK_SIZE + 1]>,
    dense_power_tail: Vec<[u32; DECK_SIZE + 1]>,
    dense_skill_tail: Vec<[u32; DECK_SIZE + 1]>,
    dense_leader_tail: Vec<u16>,
}

const _: () = assert!(size_of::<SuffixBound>() <= 600);

impl SuffixBound {
    /// 基于卡池构建一次性后缀上界数据。
    pub fn build(pool: &CardPool, ctx: &SearchContext) -> Self {
        let mut power_per_char = [0u32; CHAR_MASK_COUNT];
        let mut skill_per_char = [0u16; CHAR_MASK_COUNT];
        let mut bonus_per_char = [0u16; CHAR_MASK_COUNT];

        for card in pool.indices() {
            let ch = pool.char_id(card) as usize;
            debug_assert!(ch < CHAR_MASK_COUNT);
            power_per_char[ch] = power_per_char[ch].max(pool.power_max(card));
            skill_per_char[ch] = skill_per_char[ch].max(pool.skill_max(card) as u16);
            let bonus = pool.event_bonus(card).total_ceil() as u16;
            bonus_per_char[ch] = bonus_per_char[ch].max(bonus);
        }

        let mut power_order = core::array::from_fn(|idx| idx as u8);
        power_order
            .sort_unstable_by_key(|&char_id| std::cmp::Reverse(power_per_char[char_id as usize]));
        let mut skill_order = core::array::from_fn(|idx| idx as u8);
        skill_order
            .sort_unstable_by_key(|&char_id| std::cmp::Reverse(skill_per_char[char_id as usize]));
        let mut bonus_order = core::array::from_fn(|idx| idx as u8);
        bonus_order
            .sort_unstable_by_key(|&char_id| std::cmp::Reverse(bonus_per_char[char_id as usize]));
        let (
            dense_bonus_tail,
            dense_base_bonus_tail,
            dense_limited_bonus_tail,
            dense_power_tail,
            dense_skill_tail,
            dense_leader_tail,
        ) = build_dense_suffix_tails(pool);

        let base_rate: f64 = match ctx.effective_live_type() {
            LiveType::Auto | LiveType::ChallengeAuto => ctx.base_score_auto,
            LiveType::Multi | LiveType::Cheerful => ctx.base_score + ctx.fever_score * 0.5,
            _ => ctx.base_score,
        };
        let skill_rate_sum: f64 = ctx.skill_scores[match ctx.effective_live_type() {
            LiveType::Multi | LiveType::Cheerful => 1,
            LiveType::Auto | LiveType::ChallengeAuto => 2,
            _ => 0,
        }]
        .iter()
        .sum();
        let active_skill_rates = ctx.skill_scores[match ctx.effective_live_type() {
            LiveType::Multi | LiveType::Cheerful => 1,
            LiveType::Auto | LiveType::ChallengeAuto => 2,
            _ => 0,
        }];
        let avg_sum5 = active_skill_rates[..DECK_SIZE].iter().sum::<f64>();
        let avg_leader_rate = active_skill_rates[DECK_SIZE];

        Self {
            target: ctx.target,
            effective_live_type: ctx.effective_live_type(),
            has_event: ctx.has_event(),
            music_rate_pct: ctx.music_rate_pct,
            boost_rate_pct: ctx.boost_rate_pct,
            base_rate_1m: (base_rate * 1_000_000.0).ceil() as i64,
            srs_div500_1m: (skill_rate_sum / 500.0 * 1_000_000.0).ceil() as i64,
            avg_sum5_1m: (avg_sum5 * 1_000_000.0).ceil() as i64,
            avg_leader_rate_1m: (avg_leader_rate * 1_000_000.0).ceil() as i64,
            teammate_su_5x: ctx
                .multi_teammate_score_up
                .map(|v| v as i64 * 5)
                .unwrap_or(0),
            active_1m_coeff: if matches!(ctx.effective_live_type(), LiveType::Multi) {
                75_000
            } else {
                0
            },
            other_score: ctx.other_score,
            life_rate_num: if matches!(ctx.effective_live_type(), LiveType::Cheerful) {
                5750 + ctx.life.clamp(500, 1000)
            } else {
                0
            },
            multi_teammate_power: ctx.multi_teammate_power,
            live_skill_order: ctx.live_skill_order,
            is_world_bloom: ctx.is_world_bloom,
            is_final_chapter: ctx.is_final_chapter,
            limited_bonus_cap: ctx.card_bonus_count_limit,
            extra_bonus_ub: ctx.extra_bonus_ub,
            diff_attr_bonus: ctx.diff_attr_bonus,
            support_cards: ctx.support_deck.cards.clone(),
            support_count: ctx.support_deck.count as usize,
            honor_bonus: ctx.honor_bonus,
            power_total_cap: ctx.power_total_cap,
            power_order,
            power_vals: power_order.map(|char_id| power_per_char[char_id as usize]),
            skill_order,
            skill_vals: skill_order.map(|char_id| skill_per_char[char_id as usize]),
            bonus_order,
            bonus_vals: bonus_order.map(|char_id| bonus_per_char[char_id as usize]),
            dense_bonus_tail,
            dense_base_bonus_tail,
            dense_limited_bonus_tail,
            dense_power_tail,
            dense_skill_tail,
            dense_leader_tail,
        }
    }

    /// 对标准 5 卡搜索计算上界。
    pub fn upper_bound(&self, depth: usize, used_chars: &UsedSet, partial: &PartialDeck) -> u64 {
        self.upper_bound_for_slots(DECK_SIZE.saturating_sub(depth), used_chars, partial)
    }

    #[inline(always)]
    pub(crate) fn upper_bound_with_depth(
        &self,
        depth: usize,
        used_chars: &UsedSet,
        partial: &PartialDeck,
    ) -> u64 {
        self.upper_bound_for_slots(DECK_SIZE.saturating_sub(depth), used_chars, partial)
    }

    #[inline(always)]
    pub(crate) fn upper_bound_for_slots(
        &self,
        slots_left: usize,
        used_chars: &UsedSet,
        partial: &PartialDeck,
    ) -> u64 {
        match self.target {
            ScoreTarget::Power => self.clamp_power_total(
                partial.power
                    + suffix_sum_u32(
                        &self.power_order,
                        &self.power_vals,
                        used_chars.bits(),
                        slots_left,
                    )
                    + self.honor_bonus,
            ) as u64,
            ScoreTarget::Skill => {
                let total_skill = partial.skill
                    + suffix_sum_u16_as_u32(
                        &self.skill_order,
                        &self.skill_vals,
                        used_chars.bits(),
                        slots_left,
                    );
                let best_unused =
                    first_unused_val_u16(&self.skill_order, &self.skill_vals, used_chars.bits());
                let leader_ub = (partial.max_skill as u32).max(best_unused as u32);
                (2 * total_skill + 8 * leader_ub) as u64
            }
            ScoreTarget::Score => {
                let total_power = self.clamp_power_total(
                    partial.power
                        + suffix_sum_u32(
                            &self.power_order,
                            &self.power_vals,
                            used_chars.bits(),
                            slots_left,
                        )
                        + self.honor_bonus,
                );
                let total_bonus = partial.bonus
                    + suffix_sum_u16_as_u32(
                        &self.bonus_order,
                        &self.bonus_vals,
                        used_chars.bits(),
                        slots_left,
                    )
                    + self.extra_bonus_ub;
                let total_skill = partial.skill
                    + suffix_sum_u16_as_u32(
                        &self.skill_order,
                        &self.skill_vals,
                        used_chars.bits(),
                        slots_left,
                    );
                let best_unused =
                    first_unused_val_u16(&self.skill_order, &self.skill_vals, used_chars.bits());
                let leader_ub = (partial.max_skill as u32).max(best_unused as u32);
                let live_score = self.calc_live_score_bound(total_power, total_skill, leader_ub);
                let event_point = self.calc_event_point_bound(live_score, total_bonus);
                ((event_point as u64) << 32) | (live_score as u32 as u64)
            }
            ScoreTarget::Mysekai => {
                let total_power = self.clamp_power_total(
                    partial.power
                        + suffix_sum_u32(
                            &self.power_order,
                            &self.power_vals,
                            used_chars.bits(),
                            slots_left,
                        )
                        + self.honor_bonus,
                );
                let total_bonus = partial.bonus
                    + suffix_sum_u16_as_u32(
                        &self.bonus_order,
                        &self.bonus_vals,
                        used_chars.bits(),
                        slots_left,
                    )
                    + self.extra_bonus_ub;
                calc_mysekai_internal(total_power, total_bonus as f64) as u64
            }
        }
    }

    /// 预计算同层 suffix 分量，供 Power/Skill monotonic break 使用。
    #[inline(always)]
    pub(crate) fn precompute_layer(&self, used: &UsedSet, slots: usize) -> LayerPrecomputed {
        let rest = slots.saturating_sub(1);
        LayerPrecomputed {
            suffix_power_rest: suffix_sum_u32(
                &self.power_order,
                &self.power_vals,
                used.bits(),
                rest,
            ),
            suffix_bonus: suffix_sum_u16_as_u32(
                &self.bonus_order,
                &self.bonus_vals,
                used.bits(),
                rest,
            ),
            extra_bonus_ub: self.extra_bonus_ub,
            skill_ub_rest: suffix_sum_u16_as_u32(
                &self.skill_order,
                &self.skill_vals,
                used.bits(),
                rest,
            ),
        }
    }

    /// Score/no-event 专用预计算：只保留 power/skill/leader 所需字段。
    #[inline(always)]
    pub(crate) fn precompute_layer_score_noevent(
        &self,
        used: &UsedSet,
        slots: usize,
    ) -> LayerPrecomputedScoreNoEvent {
        let rest = slots.saturating_sub(1);
        let (suffix_power_rest, pwr_set, pwr_excl) =
            suffix_compact_u32(&self.power_order, &self.power_vals, used.bits(), rest);
        let (skill_ub_rest, skl_set, skl_excl) =
            suffix_compact_u16(&self.skill_order, &self.skill_vals, used.bits(), rest);
        let (best_skill, second_best, best_char) =
            first_two_unused_skill(&self.skill_order, &self.skill_vals, used.bits());
        LayerPrecomputedScoreNoEvent {
            suffix_power_rest,
            skill_ub_rest,
            best_unused_skill: best_skill,
            second_best_skill: second_best,
            best_skill_char: best_char,
            pwr_set,
            skl_set,
            pwr_excl,
            skl_excl,
        }
    }

    /// EP target 专用预计算：含 per-character exclusion delta。
    #[inline(always)]
    pub(crate) fn precompute_layer_ep(&self, used: &UsedSet, slots: usize) -> LayerPrecomputedEp {
        let rest = slots.saturating_sub(1);
        let (suffix_power_rest, pwr_set, pwr_excl) =
            suffix_compact_u32(&self.power_order, &self.power_vals, used.bits(), rest);
        let (suffix_bonus, bns_set, bns_excl) =
            suffix_compact_u16(&self.bonus_order, &self.bonus_vals, used.bits(), rest);
        let (skill_ub_rest, skl_set, skl_excl) =
            suffix_compact_u16(&self.skill_order, &self.skill_vals, used.bits(), rest);
        let (best_skill, second_best, best_char) =
            first_two_unused_skill(&self.skill_order, &self.skill_vals, used.bits());
        LayerPrecomputedEp {
            suffix_power_rest,
            suffix_bonus,
            skill_ub_rest,
            extra_bonus_ub: self.extra_bonus_ub,
            best_unused_skill: best_skill,
            second_best_skill: second_best,
            best_skill_char: best_char,
            pwr_set,
            bns_set,
            skl_set,
            pwr_excl,
            bns_excl,
            skl_excl,
        }
    }

    /// 廉价 ep ceiling：接受动态 power_ub 和 bonus_total，一次浮点 ep 计算。
    #[inline(always)]
    pub(crate) fn ceiling(
        &self,
        power_ub: u32,
        bonus_total: u32,
        skill_ub: u32,
        leader_ub: u32,
    ) -> u64 {
        let power_ub = self.clamp_power_total(power_ub + self.honor_bonus);
        match self.target {
            ScoreTarget::Power => power_ub as u64,
            ScoreTarget::Skill => (2 * skill_ub + 8 * leader_ub) as u64,
            ScoreTarget::Score => {
                let live = self.calc_live_score_bound(power_ub, skill_ub, leader_ub);
                let ep = self.calc_event_point_bound(live, bonus_total);
                ((ep as u64) << 32) | (live as u32 as u64)
            }
            ScoreTarget::Mysekai => calc_mysekai_internal(power_ub, bonus_total as f64) as u64,
        }
    }

    #[inline(always)]
    fn calc_live_score_bound(&self, power_total: u32, skill_total: u32, leader_ub: u32) -> i32 {
        let rate_1m = match self.effective_live_type {
            LiveType::Multi | LiveType::Cheerful => {
                let max_slot_5x =
                    (4 * leader_ub as i64 + skill_total as i64).max(self.teammate_su_5x);
                self.base_rate_1m + max_slot_5x * self.srs_div500_1m
            }
            LiveType::Solo | LiveType::Auto
                if matches!(self.live_skill_order, LiveSkillOrder::Average) =>
            {
                self.base_rate_1m
                    + skill_total as i64 * self.avg_sum5_1m / 500
                    + leader_ub as i64 * self.avg_leader_rate_1m / 100
            }
            _ => self.base_rate_1m + 5 * skill_total as i64 * self.srs_div500_1m,
        };
        let power_sum: i64 = if let Some(tp) = self.multi_teammate_power {
            power_total as i64 + tp as i64 * (DECK_SIZE as i64 - 1)
        } else {
            DECK_SIZE as i64 * power_total as i64
        };
        let active_1m = self.active_1m_coeff * power_sum;
        match self.effective_live_type {
            LiveType::Mysekai => 0,
            _ => ((rate_1m * power_total as i64 * 4 + active_1m) / 1_000_000) as i32,
        }
    }

    #[inline(always)]
    fn clamp_power_total(&self, power_total: u32) -> u32 {
        self.power_total_cap
            .map_or(power_total, |cap| power_total.min(cap))
    }

    /// Score/no-event 的 dense-aware suffix ceiling。
    #[inline(always)]
    pub(crate) fn score_noevent_dense_ceiling(
        &self,
        dense_start: usize,
        partial: &PartialDeck,
        slots: usize,
    ) -> u64 {
        let tail_power = self
            .dense_power_tail
            .get(dense_start)
            .map(|tail| tail[slots])
            .unwrap_or(0);
        let tail_skill = self
            .dense_skill_tail
            .get(dense_start)
            .map(|tail| tail[slots])
            .unwrap_or(0);
        let tail_leader = self
            .dense_leader_tail
            .get(dense_start)
            .copied()
            .unwrap_or(0) as u32;
        self.ceiling(
            partial.power + tail_power,
            0,
            partial.skill + tail_skill,
            (partial.max_skill as u32).max(tail_leader),
        )
    }

    /// 当前 dense suffix 的 target-aware ceiling。
    #[inline(always)]
    pub(crate) fn dense_suffix_ceiling(
        &self,
        dense_start: usize,
        partial: &PartialDeck,
        slots: usize,
    ) -> u64 {
        let tail_bonus = self.dense_bonus_from_start(dense_start, slots, partial.limited_count);
        let tail_power = self
            .dense_power_tail
            .get(dense_start)
            .map(|tail| tail[slots])
            .unwrap_or(0);
        let tail_skill = self
            .dense_skill_tail
            .get(dense_start)
            .map(|tail| tail[slots])
            .unwrap_or(0);
        let tail_leader = self
            .dense_leader_tail
            .get(dense_start)
            .copied()
            .unwrap_or(0) as u32;
        self.ceiling(
            partial.power + tail_power,
            partial.bonus + tail_bonus + self.extra_bonus_ub,
            partial.skill + tail_skill,
            (partial.max_skill as u32).max(tail_leader),
        )
    }

    #[inline(always)]
    pub(crate) fn dense_suffix_ceiling_with_extra(
        &self,
        dense_start: usize,
        partial: &PartialDeck,
        slots: usize,
        extra_bonus_ub: u32,
    ) -> u64 {
        let tail_bonus = self.dense_bonus_from_start(dense_start, slots, partial.limited_count);
        let tail_power = self
            .dense_power_tail
            .get(dense_start)
            .map(|tail| tail[slots])
            .unwrap_or(0);
        let tail_skill = self
            .dense_skill_tail
            .get(dense_start)
            .map(|tail| tail[slots])
            .unwrap_or(0);
        let tail_leader = self
            .dense_leader_tail
            .get(dense_start)
            .copied()
            .unwrap_or(0) as u32;
        self.ceiling(
            partial.power + tail_power,
            partial.bonus + tail_bonus + extra_bonus_ub,
            partial.skill + tail_skill,
            (partial.max_skill as u32).max(tail_leader),
        )
    }

    /// 当前候选 + dense suffix 的廉价 ceiling。
    #[inline(always)]
    pub(crate) fn dense_candidate_ceiling(
        &self,
        next_start: usize,
        partial: &PartialDeck,
        card_power: u32,
        card_bonus: u32,
        card_base_bonus: u32,
        card_limited_bonus: u32,
        card_skill: u32,
        slots: usize,
    ) -> u64 {
        let rest = slots.saturating_sub(1);
        let card_bonus = if self.is_final_chapter {
            card_base_bonus
                + if partial.limited_count as usize >= self.limited_bonus_cap {
                    0
                } else {
                    card_limited_bonus
                }
        } else {
            card_bonus
        };
        let next_limited_count = partial.limited_count.saturating_add(
            (self.is_final_chapter
                && card_limited_bonus > 0
                && (partial.limited_count as usize) < self.limited_bonus_cap) as u8,
        );
        let tail_bonus = self.dense_bonus_from_start(next_start, rest, next_limited_count);
        let tail_power = self
            .dense_power_tail
            .get(next_start)
            .map(|tail| tail[rest])
            .unwrap_or(0);
        let tail_skill = self
            .dense_skill_tail
            .get(next_start)
            .map(|tail| tail[rest])
            .unwrap_or(0);
        let tail_leader = self.dense_leader_tail.get(next_start).copied().unwrap_or(0) as u32;
        self.ceiling(
            partial.power + card_power + tail_power,
            partial.bonus + card_bonus + tail_bonus + self.extra_bonus_ub,
            partial.skill + card_skill + tail_skill,
            (partial.max_skill as u32).max(card_skill).max(tail_leader),
        )
    }

    /// 当前候选 + dense suffix 的 ceiling，调用方传入更紧的额外 bonus 上界。
    #[inline(always)]
    pub(crate) fn dense_candidate_ceiling_with_extra(
        &self,
        next_start: usize,
        partial: &PartialDeck,
        card_power: u32,
        card_bonus: u32,
        card_base_bonus: u32,
        card_limited_bonus: u32,
        card_skill: u32,
        slots: usize,
        extra_bonus_ub: u32,
    ) -> u64 {
        let rest = slots.saturating_sub(1);
        let card_bonus = if self.is_final_chapter {
            card_base_bonus
                + if partial.limited_count as usize >= self.limited_bonus_cap {
                    0
                } else {
                    card_limited_bonus
                }
        } else {
            card_bonus
        };
        let next_limited_count = partial.limited_count.saturating_add(
            (self.is_final_chapter
                && card_limited_bonus > 0
                && (partial.limited_count as usize) < self.limited_bonus_cap) as u8,
        );
        let tail_bonus = self.dense_bonus_from_start(next_start, rest, next_limited_count);
        let tail_power = self
            .dense_power_tail
            .get(next_start)
            .map(|tail| tail[rest])
            .unwrap_or(0);
        let tail_skill = self
            .dense_skill_tail
            .get(next_start)
            .map(|tail| tail[rest])
            .unwrap_or(0);
        let tail_leader = self.dense_leader_tail.get(next_start).copied().unwrap_or(0) as u32;
        self.ceiling(
            partial.power + card_power + tail_power,
            partial.bonus + card_bonus + tail_bonus + extra_bonus_ub,
            partial.skill + card_skill + tail_skill,
            (partial.max_skill as u32).max(card_skill).max(tail_leader),
        )
    }

    #[inline(always)]
    pub(crate) fn dense_bonus_from_start(
        &self,
        dense_start: usize,
        slots: usize,
        limited_count: u8,
    ) -> u32 {
        if !self.is_final_chapter {
            return self
                .dense_bonus_tail
                .get(dense_start)
                .map(|tail| tail[slots])
                .unwrap_or(0);
        }
        let tail_base = self
            .dense_base_bonus_tail
            .get(dense_start)
            .map(|tail| tail[slots])
            .unwrap_or(0);
        let remaining_limit = self
            .limited_bonus_cap
            .saturating_sub(limited_count as usize)
            .min(slots);
        let tail_limited = self
            .dense_limited_bonus_tail
            .get(dense_start)
            .map(|tail| tail[remaining_limit])
            .unwrap_or(0);
        tail_base + tail_limited
    }

    #[inline(always)]
    pub(crate) fn world_bloom_extra_bonus_bound_for_candidate_parts(
        &self,
        attr_set: u8,
        selected: &[u16; DECK_SIZE],
        selected_len: usize,
        candidate_game_id: u16,
        rest: usize,
    ) -> u32 {
        if !self.is_world_bloom {
            return self.extra_bonus_ub;
        }

        let current_attrs = attr_set.count_ones() as usize;
        let max_attrs = (current_attrs + rest).min(DECK_SIZE);
        let mut diff_ub = 0u32;
        let mut count = current_attrs;
        while count <= max_attrs {
            diff_ub = diff_ub.max(self.diff_attr_bonus[count] as u32);
            count += 1;
        }

        let support_sum =
            self.support_sum_excluding_candidate(selected, selected_len, candidate_game_id);

        diff_ub + support_sum.ceil() as u32
    }

    #[inline(always)]
    pub(crate) fn world_bloom_extra_bonus_bound_from_parts(
        &self,
        attr_set: u8,
        selected: &[u16; DECK_SIZE],
        selected_len: usize,
        rest: usize,
    ) -> u32 {
        if !self.is_world_bloom {
            return self.extra_bonus_ub;
        }

        let current_attrs = attr_set.count_ones() as usize;
        let max_attrs = (current_attrs + rest).min(DECK_SIZE);
        let mut diff_ub = 0u32;
        let mut count = current_attrs;
        while count <= max_attrs {
            diff_ub = diff_ub.max(self.diff_attr_bonus[count] as u32);
            count += 1;
        }

        let support_sum = self.support_sum_excluding(selected, selected_len);

        diff_ub + support_sum.ceil() as u32
    }

    #[inline(always)]
    fn support_sum_excluding(&self, selected: &[u16; DECK_SIZE], selected_len: usize) -> f64 {
        let mut support_sum = 0.0_f64;
        let mut picked = 0usize;
        let mut idx = 0usize;
        while idx < self.support_cards.len() {
            if picked >= self.support_count {
                break;
            }
            let (game_id, bonus) = unsafe { *self.support_cards.get_unchecked(idx) };
            if selected[0] == game_id
                || (selected_len > 1 && selected[1] == game_id)
                || (selected_len > 2 && selected[2] == game_id)
                || (selected_len > 3 && selected[3] == game_id)
                || (selected_len > 4 && selected[4] == game_id)
            {
                idx += 1;
                continue;
            }
            support_sum += bonus;
            picked += 1;
            idx += 1;
        }
        support_sum
    }

    #[inline(always)]
    fn support_sum_excluding_candidate(
        &self,
        selected: &[u16; DECK_SIZE],
        selected_len: usize,
        candidate_game_id: u16,
    ) -> f64 {
        let mut support_sum = 0.0_f64;
        let mut picked = 0usize;
        let mut idx = 0usize;
        while idx < self.support_cards.len() {
            if picked >= self.support_count {
                break;
            }
            let (game_id, bonus) = unsafe { *self.support_cards.get_unchecked(idx) };
            if game_id == candidate_game_id
                || selected[0] == game_id
                || (selected_len > 1 && selected[1] == game_id)
                || (selected_len > 2 && selected[2] == game_id)
                || (selected_len > 3 && selected[3] == game_id)
                || (selected_len > 4 && selected[4] == game_id)
            {
                idx += 1;
                continue;
            }
            support_sum += bonus;
            picked += 1;
            idx += 1;
        }
        support_sum
    }

    pub(crate) fn mono_precompute(
        &self,
        used: &UsedSet,
        partial: &PartialDeck,
        slots: usize,
    ) -> Option<MonoBreakState> {
        if !self.has_event {
            return None;
        }
        match self.target {
            ScoreTarget::Score => self.mono_precompute_score(used, partial, slots),
            _ => None,
        }
    }

    fn mono_precompute_score(
        &self,
        used: &UsedSet,
        partial: &PartialDeck,
        slots: usize,
    ) -> Option<MonoBreakState> {
        if matches!(
            self.effective_live_type,
            LiveType::Challenge | LiveType::ChallengeAuto | LiveType::Mysekai
        ) {
            return None;
        }

        let max_power = partial.power
            + suffix_sum_u32(&self.power_order, &self.power_vals, used.bits(), slots)
            + self.honor_bonus;
        let max_skill = partial.skill
            + suffix_sum_u16_as_u32(&self.skill_order, &self.skill_vals, used.bits(), slots);
        let max_leader = (partial.max_skill as u32).max(first_unused_val_u16(
            &self.skill_order,
            &self.skill_vals,
            used.bits(),
        ) as u32);
        let max_live = self.calc_live_score_bound(max_power, max_skill, max_leader);

        let base_score: i64 = match self.effective_live_type {
            LiveType::Solo | LiveType::Auto => (100 + max_live / 20_000) as i64,
            LiveType::Multi | LiveType::Cheerful => {
                let other = if self.other_score == 0 {
                    (max_live as i64).saturating_mul(4)
                } else {
                    self.other_score as i64
                };
                110 + max_live as i64 / 17_000 + (other / 340_000).min(13)
            }
            _ => return None,
        };
        if base_score <= 0 {
            return None;
        }

        let bm = base_score * self.music_rate_pct as i64;
        if bm <= 0 {
            return None;
        }

        Some(MonoBreakState::Score {
            bm,
            boost: self.boost_rate_pct as i64,
            is_cheerful: matches!(self.effective_live_type, LiveType::Cheerful),
            life: self.life_rate_num as i64,
        })
    }

    #[inline(always)]
    fn calc_event_point_bound(&self, live_score: i32, total_bonus: u32) -> i32 {
        if !self.has_event {
            return live_score;
        }
        match self.effective_live_type {
            LiveType::Challenge | LiveType::ChallengeAuto => (100 + live_score / 20_000) * 120,
            LiveType::Solo | LiveType::Auto => {
                let base_score = (100 + live_score / 20_000) as i64;
                let inner =
                    base_score * self.music_rate_pct as i64 * (total_bonus as i64 + 100) / 10_000;
                (inner * self.boost_rate_pct as i64 / 100) as i32
            }
            LiveType::Multi => {
                let other_score = if self.other_score == 0 {
                    (live_score as i64).saturating_mul(4)
                } else {
                    self.other_score as i64
                };
                let base_score = 110 + live_score as i64 / 17_000 + (other_score / 340_000).min(13);
                let inner =
                    base_score * self.music_rate_pct as i64 * (total_bonus as i64 + 100) / 10_000;
                (inner * self.boost_rate_pct as i64 / 100) as i32
            }
            LiveType::Cheerful => {
                let other_score = if self.other_score == 0 {
                    (live_score as i64).saturating_mul(4)
                } else {
                    self.other_score as i64
                };
                let base_score = 110 + live_score as i64 / 17_000 + (other_score / 340_000).min(13);
                let inner = (base_score * self.music_rate_pct as i64 * (total_bonus as i64 + 100)
                    / 10_000) as i32;
                let with_life = inner as i64 * self.life_rate_num as i64 / 5000;
                (with_life * self.boost_rate_pct as i64 / 100) as i32
            }
            LiveType::Mysekai => 0,
        }
    }
}

#[inline(always)]
fn suffix_sum_u32(
    order: &[u8; CHAR_MASK_COUNT],
    vals: &[u32; CHAR_MASK_COUNT],
    used: u32,
    slots_left: usize,
) -> u32 {
    let mut sum = 0u32;
    let mut count = 0usize;
    let mut idx = 0usize;
    while idx < CHAR_MASK_COUNT {
        if count >= slots_left {
            break;
        }
        let char_id = unsafe { *order.get_unchecked(idx) };
        if used & (1u32 << char_id) == 0 {
            sum += unsafe { *vals.get_unchecked(idx) };
            count += 1;
        }
        idx += 1;
    }
    sum
}

#[inline(always)]
fn suffix_sum_u16_as_u32(
    order: &[u8; CHAR_MASK_COUNT],
    vals: &[u16; CHAR_MASK_COUNT],
    used: u32,
    slots_left: usize,
) -> u32 {
    let mut sum = 0u32;
    let mut count = 0usize;
    let mut idx = 0usize;
    while idx < CHAR_MASK_COUNT {
        if count >= slots_left {
            break;
        }
        let char_id = unsafe { *order.get_unchecked(idx) };
        if used & (1u32 << char_id) == 0 {
            sum += unsafe { *vals.get_unchecked(idx) } as u32;
            count += 1;
        }
        idx += 1;
    }
    sum
}

/// Power/Skill monotonic break 专用预计算。
#[derive(Clone, Copy, Debug)]
pub(crate) struct LayerPrecomputed {
    /// 剩余 slots-1 的 power suffix sum。
    pub suffix_power_rest: u32,
    /// 剩余 slots-1 的 bonus suffix sum（不含 extra）。
    pub suffix_bonus: u32,
    /// WL 等额外 bonus 上界。
    pub extra_bonus_ub: u32,
    /// 剩余 slots-1 的 skill suffix sum。
    pub skill_ub_rest: u32,
}

/// Score/no-event 专用预计算：紧凑 exclusion delta。
#[derive(Clone, Copy, Debug)]
pub(crate) struct LayerPrecomputedScoreNoEvent {
    pub suffix_power_rest: u32,
    pub skill_ub_rest: u32,
    pub best_unused_skill: u16,
    pub second_best_skill: u16,
    pub best_skill_char: u8,
    pwr_set: u32,
    skl_set: u32,
    pwr_excl: [u32; DECK_SIZE],
    skl_excl: [u32; DECK_SIZE],
}

impl LayerPrecomputedScoreNoEvent {
    #[inline(always)]
    pub(crate) fn power_delta(&self, char_id: u8) -> u32 {
        compact_excl(self.pwr_set, &self.pwr_excl, char_id)
    }
    #[inline(always)]
    pub(crate) fn skill_delta(&self, char_id: u8) -> u32 {
        compact_excl(self.skl_set, &self.skl_excl, char_id)
    }
}

/// EP target 专用预计算：紧凑 exclusion delta via popcount 索引。
#[derive(Clone, Copy, Debug)]
pub(crate) struct LayerPrecomputedEp {
    pub suffix_power_rest: u32,
    pub suffix_bonus: u32,
    pub skill_ub_rest: u32,
    pub extra_bonus_ub: u32,
    pub best_unused_skill: u16,
    pub second_best_skill: u16,
    pub best_skill_char: u8,
    pwr_set: u32,
    bns_set: u32,
    skl_set: u32,
    pwr_excl: [u32; DECK_SIZE],
    bns_excl: [u32; DECK_SIZE],
    skl_excl: [u32; DECK_SIZE],
}

impl LayerPrecomputedEp {
    #[inline(always)]
    pub(crate) fn power_delta(&self, char_id: u8) -> u32 {
        compact_excl(self.pwr_set, &self.pwr_excl, char_id)
    }
    #[inline(always)]
    pub(crate) fn bonus_delta(&self, char_id: u8) -> u32 {
        compact_excl(self.bns_set, &self.bns_excl, char_id)
    }
    #[inline(always)]
    pub(crate) fn skill_delta(&self, char_id: u8) -> u32 {
        compact_excl(self.skl_set, &self.skl_excl, char_id)
    }
}

#[inline(always)]
fn compact_excl(set: u32, excl: &[u32; DECK_SIZE], char_id: u8) -> u32 {
    let bit = 1u32 << char_id;
    if set & bit == 0 {
        return 0;
    }
    let pos = (set & (bit - 1)).count_ones() as usize;
    unsafe { *excl.get_unchecked(pos) }
}

/// 两阶段 mono break 预计算状态。
pub(crate) enum MonoBreakState {
    Score {
        bm: i64,
        boost: i64,
        is_cheerful: bool,
        life: i64,
    },
}

impl MonoBreakState {
    #[inline(always)]
    pub(crate) fn min_bonus(&self, threshold: u64) -> u32 {
        match self {
            MonoBreakState::Score {
                bm,
                boost,
                is_cheerful,
                life,
            } => {
                let threshold_ep = (threshold >> 32) as i64;
                if threshold_ep <= 0 {
                    return 0;
                }
                let min_inner = if *is_cheerful {
                    let min_wl = ((threshold_ep + 1) * 100 + boost - 1) / boost;
                    (min_wl * 5000 + life - 1) / life
                } else {
                    ((threshold_ep + 1) * 100 + boost - 1) / boost
                };
                let min_bp100 = (min_inner * 10_000 + bm - 1) / bm;
                (min_bp100 - 100).max(0) as u32
            }
        }
    }
}

#[inline(always)]
fn first_unused_val_u16(
    order: &[u8; CHAR_MASK_COUNT],
    vals: &[u16; CHAR_MASK_COUNT],
    used: u32,
) -> u16 {
    let mut idx = 0usize;
    while idx < CHAR_MASK_COUNT {
        let char_id = unsafe { *order.get_unchecked(idx) };
        if used & (1u32 << char_id) == 0 {
            return unsafe { *vals.get_unchecked(idx) };
        }
        idx += 1;
    }
    0
}

#[inline(always)]
fn first_two_unused_skill(
    order: &[u8; CHAR_MASK_COUNT],
    vals: &[u16; CHAR_MASK_COUNT],
    used: u32,
) -> (u16, u16, u8) {
    let mut best = 0u16;
    let mut second = 0u16;
    let mut best_char = 0u8;
    let mut count = 0usize;
    let mut idx = 0usize;
    while idx < CHAR_MASK_COUNT {
        let char_id = unsafe { *order.get_unchecked(idx) };
        if used & (1u32 << char_id) == 0 {
            let v = unsafe { *vals.get_unchecked(idx) };
            if count == 0 {
                best = v;
                best_char = char_id;
            } else if count == 1 {
                second = v;
                return (best, second, best_char);
            }
            count += 1;
        }
        idx += 1;
    }
    (best, second, best_char)
}

const CHAR_MASK_COUNT: usize = 27;

#[inline(always)]
fn suffix_compact_u32(
    order: &[u8; CHAR_MASK_COUNT],
    vals: &[u32; CHAR_MASK_COUNT],
    used: u32,
    slots_left: usize,
) -> (u32, u32, [u32; DECK_SIZE]) {
    let mut sum = 0u32;
    let mut set = 0u32;
    let mut sel_chars = [0u8; DECK_SIZE];
    let mut sel_vals = [0u32; DECK_SIZE];
    let mut count = 0usize;
    let mut replacement = 0u32;
    let mut has_repl = false;

    let mut idx = 0usize;
    while idx < CHAR_MASK_COUNT {
        let char_id = unsafe { *order.get_unchecked(idx) };
        if used & (1u32 << char_id) == 0 {
            if count < slots_left {
                let v = unsafe { *vals.get_unchecked(idx) };
                set |= 1u32 << char_id;
                sel_chars[count] = char_id;
                sel_vals[count] = v;
                sum += v;
                count += 1;
            } else if !has_repl {
                replacement = unsafe { *vals.get_unchecked(idx) };
                has_repl = true;
                break;
            }
        }
        idx += 1;
    }
    let mut raw = [0u32; DECK_SIZE];
    let mut i = 0usize;
    while i < count {
        let c = sel_chars[i];
        let pos = (set & ((1u32 << c) - 1)).count_ones() as usize;
        raw[pos] = if has_repl {
            sel_vals[i] - replacement
        } else {
            sel_vals[i]
        };
        i += 1;
    }
    (sum, set, raw)
}

#[inline(always)]
fn suffix_compact_u16(
    order: &[u8; CHAR_MASK_COUNT],
    vals: &[u16; CHAR_MASK_COUNT],
    used: u32,
    slots_left: usize,
) -> (u32, u32, [u32; DECK_SIZE]) {
    let mut sum = 0u32;
    let mut set = 0u32;
    let mut sel_chars = [0u8; DECK_SIZE];
    let mut sel_vals = [0u32; DECK_SIZE];
    let mut count = 0usize;
    let mut replacement = 0u32;
    let mut has_repl = false;

    let mut idx = 0usize;
    while idx < CHAR_MASK_COUNT {
        let char_id = unsafe { *order.get_unchecked(idx) };
        if used & (1u32 << char_id) == 0 {
            if count < slots_left {
                let v = unsafe { *vals.get_unchecked(idx) } as u32;
                set |= 1u32 << char_id;
                sel_chars[count] = char_id;
                sel_vals[count] = v;
                sum += v;
                count += 1;
            } else if !has_repl {
                replacement = unsafe { *vals.get_unchecked(idx) } as u32;
                has_repl = true;
                break;
            }
        }
        idx += 1;
    }
    let mut raw = [0u32; DECK_SIZE];
    let mut i = 0usize;
    while i < count {
        let c = sel_chars[i];
        let pos = (set & ((1u32 << c) - 1)).count_ones() as usize;
        raw[pos] = if has_repl {
            sel_vals[i] - replacement
        } else {
            sel_vals[i]
        };
        i += 1;
    }
    (sum, set, raw)
}

fn build_dense_suffix_tails(
    pool: &CardPool,
) -> (
    Vec<[u32; DECK_SIZE + 1]>,
    Vec<[u32; DECK_SIZE + 1]>,
    Vec<[u32; DECK_SIZE + 1]>,
    Vec<[u32; DECK_SIZE + 1]>,
    Vec<[u32; DECK_SIZE + 1]>,
    Vec<u16>,
) {
    let count = pool.count();
    let mut dense_bonus_tail = vec![[0u32; DECK_SIZE + 1]; count + 1];
    let mut dense_base_bonus_tail = vec![[0u32; DECK_SIZE + 1]; count + 1];
    let mut dense_limited_bonus_tail = vec![[0u32; DECK_SIZE + 1]; count + 1];
    let mut dense_power_tail = vec![[0u32; DECK_SIZE + 1]; count + 1];
    let mut dense_skill_tail = vec![[0u32; DECK_SIZE + 1]; count + 1];
    let mut dense_leader_tail = vec![0u16; count + 1];
    let mut best_bonus_by_char = [0u32; CHAR_MASK_COUNT];
    let mut best_base_by_char = [0u32; CHAR_MASK_COUNT];
    let mut best_limited_by_char = [0u32; CHAR_MASK_COUNT];
    let mut best_power_by_char = [0u32; CHAR_MASK_COUNT];
    let mut best_skill_by_char = [0u16; CHAR_MASK_COUNT];
    let mut best_skill = 0u16;

    let mut dense = count;
    while dense > 0 {
        dense -= 1;
        let card = crate::pool::CardIdx::new(dense as u16);
        let eb = pool.event_bonus(card);
        let char_id = pool.char_id(card) as usize;
        best_bonus_by_char[char_id] = best_bonus_by_char[char_id].max(eb.total_ceil());
        best_base_by_char[char_id] = best_base_by_char[char_id].max(eb.base_ceil());
        best_limited_by_char[char_id] = best_limited_by_char[char_id].max(eb.limited_ceil());
        best_power_by_char[char_id] = best_power_by_char[char_id].max(pool.power_max(card));
        best_skill_by_char[char_id] = best_skill_by_char[char_id].max(pool.skill_max(card) as u16);
        best_skill = best_skill.max(pool.skill_max(card) as u16);

        let mut top_bonuses = [0u32; DECK_SIZE];
        let mut top_base_bonuses = [0u32; DECK_SIZE];
        let mut top_limited_bonuses = [0u32; DECK_SIZE];
        let mut top_powers = [0u32; DECK_SIZE];
        let mut top_skills = [0u16; DECK_SIZE];
        let mut ch = 0usize;
        while ch < CHAR_MASK_COUNT {
            insert_topk_u32(&mut top_bonuses, best_bonus_by_char[ch]);
            insert_topk_u32(&mut top_base_bonuses, best_base_by_char[ch]);
            insert_topk_u32(&mut top_limited_bonuses, best_limited_by_char[ch]);
            insert_topk_u32(&mut top_powers, best_power_by_char[ch]);
            insert_topk_u16(&mut top_skills, best_skill_by_char[ch]);
            ch += 1;
        }
        let mut slot = 0usize;
        while slot < DECK_SIZE {
            dense_bonus_tail[dense][slot + 1] = dense_bonus_tail[dense][slot] + top_bonuses[slot];
            dense_base_bonus_tail[dense][slot + 1] =
                dense_base_bonus_tail[dense][slot] + top_base_bonuses[slot];
            dense_limited_bonus_tail[dense][slot + 1] =
                dense_limited_bonus_tail[dense][slot] + top_limited_bonuses[slot];
            dense_power_tail[dense][slot + 1] = dense_power_tail[dense][slot] + top_powers[slot];
            dense_skill_tail[dense][slot + 1] =
                dense_skill_tail[dense][slot] + top_skills[slot] as u32;
            slot += 1;
        }
        dense_leader_tail[dense] = best_skill;
    }

    (
        dense_bonus_tail,
        dense_base_bonus_tail,
        dense_limited_bonus_tail,
        dense_power_tail,
        dense_skill_tail,
        dense_leader_tail,
    )
}

#[inline(always)]
fn insert_topk_u32(values: &mut [u32; DECK_SIZE], value: u32) {
    let mut slot = 0usize;
    while slot < DECK_SIZE {
        if value > values[slot] {
            let mut shift = DECK_SIZE - 1;
            while shift > slot {
                values[shift] = values[shift - 1];
                shift -= 1;
            }
            values[slot] = value;
            break;
        }
        slot += 1;
    }
}

#[inline(always)]
fn insert_topk_u16(values: &mut [u16; DECK_SIZE], value: u16) {
    let mut slot = 0usize;
    while slot < DECK_SIZE {
        if value > values[slot] {
            let mut shift = DECK_SIZE - 1;
            while shift > slot {
                values[shift] = values[shift - 1];
                shift -= 1;
            }
            values[slot] = value;
            break;
        }
        slot += 1;
    }
}
