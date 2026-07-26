use std::mem::size_of;

use crate::pool::{CardIdx, CardPool};
use crate::types::{LiveSkillOrder, LiveType, ScoreTarget, DECK_SIZE};

use super::context::SearchContext;
use super::evaluate::calc_mysekai_internal;

const JOINT_SUPPORT_BUCKET: u32 = 1024;

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
    /// Multi/Cheerful: 75_000 (= 0.075 × 1M), 其他: 0。
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
    dense_power_bonus_512_tail: Vec<[u32; DECK_SIZE + 1]>,
    dense_power_bonus_1024_tail: Vec<[u32; DECK_SIZE + 1]>,
    joint_ep_512: Vec<u32>,
    joint_ep_1024: Vec<u32>,
    /// Score/no-event 场景表：[allowed_unit_subset(64) * 7 + attr_opt] -> per-char max。
    /// attr_opt: 0..6 = 全同属性 attr id，6 = 无全同属性。空表示未启用。
    noev_tables: Vec<[u32; CHAR_MASK_COUNT]>,
}

const _: () = assert!(size_of::<SuffixBound>() <= 736);

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
        ) = build_dense_suffix_tails(pool, ctx.is_final_chapter);

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
            active_1m_coeff: if matches!(
                ctx.effective_live_type(),
                LiveType::Multi | LiveType::Cheerful
            ) {
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
            dense_power_bonus_512_tail: Vec::new(),
            dense_power_bonus_1024_tail: Vec::new(),
            joint_ep_512: Vec::new(),
            joint_ep_1024: Vec::new(),
            noev_tables: if matches!(ctx.target, ScoreTarget::Score) && !ctx.has_event() {
                build_noev_tables(pool)
            } else {
                Vec::new()
            },
        }
    }

    /// Score/no-event：场景感知上界。`chosen` 为已选卡（deck 前缀）。
    ///
    /// 场景 = (allowed, attr_opt)：allowed 为仍可能全员同 unit 的 unit 集合
    /// （已选卡 unit_mask 的 AND），attr_opt 为仍可能全同的属性。对每个场景，
    /// 已选卡取该场景下的精确综合力，剩余槽取每角色场景最大值 top-k。
    /// 任意补全的真实 full-unit 集合是 allowed 的子集且场景值单调，故可采纳。
    #[inline(always)]
    pub(crate) fn upper_bound_score_noevent(
        &self,
        pool: &CardPool,
        chosen: &[CardIdx],
        used_chars: &UsedSet,
        partial: &PartialDeck,
        slots_left: usize,
    ) -> u64 {
        if self.noev_tables.is_empty() {
            return self.upper_bound_for_slots(slots_left, used_chars, partial);
        }
        let mut allowed = 0x3fu8;
        let mut attr_uniform = 0xffu8;
        let mut idx = 0usize;
        while idx < chosen.len() {
            let card = chosen[idx];
            allowed &= pool.unit_mask_raw(card);
            let attr = pool.attr(card);
            if idx == 0 {
                attr_uniform = attr;
            } else if attr_uniform != attr {
                attr_uniform = 0xff;
            }
            idx += 1;
        }

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

        let mut best = self.noev_scenario_ceiling(
            pool,
            chosen,
            allowed,
            6,
            used_chars.bits(),
            slots_left,
            total_skill,
            leader_ub,
        );
        if chosen.is_empty() {
            let mut attr = 0usize;
            while attr < 6 {
                let ub = self.noev_scenario_ceiling(
                    pool,
                    chosen,
                    allowed,
                    attr,
                    used_chars.bits(),
                    slots_left,
                    total_skill,
                    leader_ub,
                );
                if ub > best {
                    best = ub;
                }
                attr += 1;
            }
        } else if attr_uniform != 0xff {
            let ub = self.noev_scenario_ceiling(
                pool,
                chosen,
                allowed,
                attr_uniform as usize,
                used_chars.bits(),
                slots_left,
                total_skill,
                leader_ub,
            );
            if ub > best {
                best = ub;
            }
        }
        best
    }

    #[allow(clippy::too_many_arguments)]
    #[inline(always)]
    fn noev_scenario_ceiling(
        &self,
        pool: &CardPool,
        chosen: &[CardIdx],
        allowed: u8,
        attr_opt: usize,
        used: u32,
        slots_left: usize,
        total_skill: u32,
        leader_ub: u32,
    ) -> u64 {
        let attr_full = attr_opt < 6;
        let mut power = 0u32;
        let mut idx = 0usize;
        while idx < chosen.len() {
            power += card_scenario_power(pool, chosen[idx], allowed, attr_full);
            idx += 1;
        }
        power += self.noev_tail(allowed, attr_opt, used, slots_left);
        self.ceiling(power, 0, total_skill, leader_ub)
    }

    #[inline(always)]
    fn noev_tail(&self, allowed: u8, attr_opt: usize, used: u32, slots_left: usize) -> u32 {
        if slots_left == 0 {
            return 0;
        }
        let vals = &self.noev_tables[allowed as usize * 7 + attr_opt];
        let mut top = [0u32; DECK_SIZE];
        let mut ch = 0usize;
        while ch < CHAR_MASK_COUNT {
            if used & (1u32 << ch) == 0 {
                insert_topk_u32_n(&mut top, vals[ch], slots_left);
            }
            ch += 1;
        }
        let mut sum = 0u32;
        let mut slot = 0usize;
        while slot < slots_left {
            sum += top[slot];
            slot += 1;
        }
        sum
    }

    pub(crate) fn build_prepared(pool: &CardPool, ctx: &SearchContext) -> Self {
        let mut bound = Self::build(pool, ctx);
        bound.dense_power_bonus_512_tail = build_dense_power_bonus_tail(pool, 512);
        bound.dense_power_bonus_1024_tail = build_dense_power_bonus_tail(pool, 1024);
        bound.joint_ep_512 = bound.build_joint_ep_table(512);
        bound.joint_ep_1024 = bound.build_joint_ep_table(1024);
        bound
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
            ScoreTarget::Bonus => {
                let total_bonus = partial.bonus
                    + suffix_sum_u16_as_u32(
                        &self.bonus_order,
                        &self.bonus_vals,
                        used_chars.bits(),
                        slots_left,
                    )
                    + self.extra_bonus_ub;
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
                (((total_bonus.saturating_mul(2)) as u64) << 32) | (live_score.max(0) as u32 as u64)
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
            ScoreTarget::Bonus => {
                let live = self.calc_live_score_bound(power_ub, skill_ub, leader_ub);
                (((bonus_total.saturating_mul(2)) as u64) << 32) | (live.max(0) as u32 as u64)
            }
            ScoreTarget::Score => {
                let live = self.calc_live_score_bound(power_ub, skill_ub, leader_ub);
                let ep = self.calc_event_point_bound(live, bonus_total);
                ((ep as u64) << 32) | (live as u32 as u64)
            }
            ScoreTarget::Mysekai => calc_mysekai_internal(power_ub, bonus_total as f64) as u64,
        }
    }

    #[inline(always)]
    pub(crate) fn ceiling_multi_score_event(
        &self,
        power_ub: u32,
        bonus_total: u32,
        skill_ub: u32,
        leader_ub: u32,
    ) -> u64 {
        let power_total = self.clamp_power_total(power_ub + self.honor_bonus);
        let max_slot_5x = (4 * leader_ub as i64 + skill_ub as i64).max(self.teammate_su_5x);
        let rate_1m = self.base_rate_1m + max_slot_5x * self.srs_div500_1m;
        let power_sum = if let Some(teammate_power) = self.multi_teammate_power {
            power_total as i64 + teammate_power as i64 * (DECK_SIZE as i64 - 1)
        } else {
            DECK_SIZE as i64 * power_total as i64
        };
        let active_1m = self.active_1m_coeff * power_sum;
        let live_score = ((rate_1m * power_total as i64 * 4 + active_1m) / 1_000_000) as i32;
        let other_score = if self.other_score == 0 {
            (live_score as i64).saturating_mul(4)
        } else {
            self.other_score as i64
        };
        let base_score = 110 + live_score as i64 / 17_000 + (other_score / 340_000).min(13);
        let inner = base_score * self.music_rate_pct as i64 * (bonus_total as i64 + 100) / 10_000;
        let event_point = (inner * self.boost_rate_pct as i64 / 100) as i32;
        ((event_point as u64) << 32) | (live_score as u32 as u64)
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
            _ => {
                // 每个技能槽的 score_up 不超过全队最大技能 L（含 leader 复发槽），
                // 因此 Σ su_i·r_i ≤ L·Σr_i = L·srs。旧值 5*skill_total(=S·srs/100)
                // 对 Solo/Auto 高估约 5 倍。
                self.base_rate_1m + 5 * (leader_ub as i64) * self.srs_div500_1m
            }
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
    pub(crate) fn dense_suffix_ceiling_multi_score_event(
        &self,
        dense_start: usize,
        partial: &PartialDeck,
        slots: usize,
    ) -> u64 {
        let tail_bonus = self
            .dense_bonus_tail
            .get(dense_start)
            .map(|tail| tail[slots])
            .unwrap_or(0);
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
        self.ceiling_multi_score_event(
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

    #[inline(always)]
    pub(crate) fn dense_candidate_ceiling_multi_score_event(
        &self,
        next_start: usize,
        partial: &PartialDeck,
        card_power: u32,
        card_bonus: u32,
        card_skill: u32,
        slots: usize,
    ) -> u64 {
        let rest = slots.saturating_sub(1);
        let tail_bonus = self
            .dense_bonus_tail
            .get(next_start)
            .map(|tail| tail[rest])
            .unwrap_or(0);
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
        self.ceiling_multi_score_event(
            partial.power + card_power + tail_power,
            partial.bonus + card_bonus + tail_bonus + self.extra_bonus_ub,
            partial.skill + card_skill + tail_skill,
            (partial.max_skill as u32).max(card_skill).max(tail_leader),
        )
    }

    #[inline(always)]
    pub(crate) fn dense_candidate_joint_ceiling_multi_score_event(
        &self,
        next_start: usize,
        partial: &PartialDeck,
        card_power: u32,
        card_bonus: u32,
        _card_skill: u32,
        slots: usize,
    ) -> u64 {
        if self.joint_ep_512.is_empty() || self.joint_ep_1024.is_empty() {
            return u64::MAX;
        }
        let rest = slots.saturating_sub(1);
        let support_512 = partial
            .power
            .saturating_add(card_power)
            .saturating_add(
                self.dense_power_bonus_512_tail
                    .get(next_start)
                    .map(|tail| tail[rest])
                    .unwrap_or(0),
            )
            .saturating_add(self.honor_bonus)
            .saturating_add(
                512u32.saturating_mul(
                    partial
                        .bonus
                        .saturating_add(card_bonus)
                        .saturating_add(self.extra_bonus_ub),
                ),
            );
        let support_1024 = partial
            .power
            .saturating_add(card_power)
            .saturating_add(
                self.dense_power_bonus_1024_tail
                    .get(next_start)
                    .map(|tail| tail[rest])
                    .unwrap_or(0),
            )
            .saturating_add(self.honor_bonus)
            .saturating_add(
                1024u32.saturating_mul(
                    partial
                        .bonus
                        .saturating_add(card_bonus)
                        .saturating_add(self.extra_bonus_ub),
                ),
            );
        let ep_512 = joint_ep_lookup(&self.joint_ep_512, support_512);
        let ep_1024 = joint_ep_lookup(&self.joint_ep_1024, support_1024);
        ((ep_512.min(ep_1024) as u64) << 32) | u32::MAX as u64
    }

    fn build_joint_ep_table(&self, bonus_weight: u32) -> Vec<u32> {
        let support_tail = if bonus_weight == 512 {
            &self.dense_power_bonus_512_tail
        } else {
            &self.dense_power_bonus_1024_tail
        };
        let max_support = support_tail
            .first()
            .map(|tail| tail[DECK_SIZE])
            .unwrap_or(0)
            .saturating_add(self.honor_bonus)
            .saturating_add(bonus_weight.saturating_mul(self.extra_bonus_ub));
        let max_power = self.clamp_power_total(
            self.dense_power_tail
                .first()
                .map(|tail| tail[DECK_SIZE])
                .unwrap_or(0)
                .saturating_add(self.honor_bonus),
        );
        let max_bonus = self
            .dense_bonus_tail
            .first()
            .map(|tail| tail[DECK_SIZE])
            .unwrap_or(0)
            .saturating_add(self.extra_bonus_ub);
        let max_skill = self
            .dense_skill_tail
            .first()
            .map(|tail| tail[DECK_SIZE])
            .unwrap_or(0);
        let max_leader = self.dense_leader_tail.first().copied().unwrap_or(0) as u32;
        let bucket_count = max_support.div_ceil(JOINT_SUPPORT_BUCKET) as usize;
        let mut table = Vec::with_capacity(bucket_count + 1);
        let mut bucket = 0usize;
        while bucket <= bucket_count {
            let support = (bucket as u32).saturating_mul(JOINT_SUPPORT_BUCKET);
            table.push(self.joint_event_point_upper(
                max_power.min(support),
                max_bonus.min(support / bonus_weight),
                max_skill,
                max_leader,
                support,
                bonus_weight,
            ));
            bucket += 1;
        }
        table
    }

    #[inline(always)]
    fn joint_event_point_upper(
        &self,
        power_ub: u32,
        bonus_ub: u32,
        skill_ub: u32,
        leader_ub: u32,
        support_ub: u32,
        bonus_weight: u32,
    ) -> u32 {
        let max_slot_5x = (4 * leader_ub as i64 + skill_ub as i64).max(self.teammate_su_5x);
        let rate_1m = self.base_rate_1m + max_slot_5x * self.srs_div500_1m;
        let (power_multiplier, power_constant) = match self.multi_teammate_power {
            Some(teammate_power) => (1i128, teammate_power as i128 * (DECK_SIZE as i128 - 1)),
            None => (DECK_SIZE as i128, 0),
        };
        let live_power_coeff =
            4i128 * rate_1m as i128 + self.active_1m_coeff as i128 * power_multiplier;
        let live_constant = self.active_1m_coeff as i128 * power_constant;

        let capped_power = self
            .power_total_cap
            .map_or(power_ub, |cap| power_ub.min(cap));
        let support_ub = self.power_total_cap.map_or(support_ub, |cap| {
            support_ub.min(cap.saturating_add(bonus_weight.saturating_mul(bonus_ub)))
        });

        let capped_other = if self.other_score == 0 {
            13i128
        } else {
            (self.other_score as i128 / 340_000).min(13)
        };
        let capped_bound = maximize_joint_event_numerator(
            capped_power,
            bonus_ub,
            support_ub,
            bonus_weight,
            123,
            17_000_000_000,
            live_power_coeff,
            live_constant,
            1,
        );
        let capped_ep = ceil_div_i128(
            capped_bound * self.music_rate_pct as i128 * self.boost_rate_pct as i128,
            17_000_000_000i128 * 1_000_000,
        );

        let selected_ep = if self.other_score == 0 {
            let uncapped_bound = maximize_joint_event_numerator(
                capped_power,
                bonus_ub,
                support_ub,
                bonus_weight,
                110,
                85_000_000_000,
                live_power_coeff,
                live_constant,
                6,
            );
            let uncapped_ep = ceil_div_i128(
                uncapped_bound * self.music_rate_pct as i128 * self.boost_rate_pct as i128,
                85_000_000_000i128 * 1_000_000,
            );
            capped_ep.min(uncapped_ep)
        } else {
            let fixed_other_bound = maximize_joint_event_numerator(
                capped_power,
                bonus_ub,
                support_ub,
                bonus_weight,
                110 + capped_other as i64,
                17_000_000_000,
                live_power_coeff,
                live_constant,
                1,
            );
            ceil_div_i128(
                fixed_other_bound * self.music_rate_pct as i128 * self.boost_rate_pct as i128,
                17_000_000_000i128 * 1_000_000,
            )
        };
        selected_ep.clamp(0, u32::MAX as i128) as u32
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

type SuffixTail = Vec<[u32; DECK_SIZE + 1]>;

fn build_dense_suffix_tails(
    pool: &CardPool,
    split_limited_bonus: bool,
) -> (
    SuffixTail,
    SuffixTail,
    SuffixTail,
    SuffixTail,
    SuffixTail,
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
        let hot = *pool.event_bonus(card);
        let char_id = pool.char_id(card) as usize;
        let total_ceil = hot.total_ceil();
        best_bonus_by_char[char_id] = best_bonus_by_char[char_id].max(total_ceil);
        if split_limited_bonus {
            let exact = pool.event_bonus_exact(card);
            best_base_by_char[char_id] = best_base_by_char[char_id].max(exact.base_ceil());
            best_limited_by_char[char_id] = best_limited_by_char[char_id].max(exact.limited_ceil());
        } else {
            best_base_by_char[char_id] = best_base_by_char[char_id].max(total_ceil);
        }
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

fn build_dense_power_bonus_tail(pool: &CardPool, bonus_weight: u32) -> Vec<[u32; DECK_SIZE + 1]> {
    let count = pool.count();
    let mut tails = vec![[0u32; DECK_SIZE + 1]; count + 1];
    let mut best_by_char = [0u32; CHAR_MASK_COUNT];
    let mut dense = count;
    while dense > 0 {
        dense -= 1;
        let card = crate::pool::CardIdx::new(dense as u16);
        let char_id = pool.char_id(card) as usize;
        let support = pool
            .power_max(card)
            .saturating_add(bonus_weight.saturating_mul(pool.event_bonus(card).total_ceil()));
        best_by_char[char_id] = best_by_char[char_id].max(support);

        let mut top = [0u32; DECK_SIZE];
        let mut ch = 0usize;
        while ch < CHAR_MASK_COUNT {
            insert_topk_u32(&mut top, best_by_char[ch]);
            ch += 1;
        }
        let mut slot = 0usize;
        while slot < DECK_SIZE {
            tails[dense][slot + 1] = tails[dense][slot].saturating_add(top[slot]);
            slot += 1;
        }
    }
    tails
}

#[inline(always)]
fn joint_ep_lookup(table: &[u32], support: u32) -> u32 {
    let bucket = support.div_ceil(JOINT_SUPPORT_BUCKET) as usize;
    table.get(bucket).copied().unwrap_or(u32::MAX)
}

#[allow(clippy::too_many_arguments)]
#[inline(always)]
fn maximize_joint_event_numerator(
    power_ub: u32,
    bonus_ub: u32,
    support_ub: u32,
    bonus_weight: u32,
    base_constant: i64,
    base_denominator: i128,
    live_power_coeff: i128,
    live_constant: i128,
    live_multiplier: i128,
) -> i128 {
    let max_bonus = bonus_ub.min(support_ub / bonus_weight);
    let linear_power = live_multiplier * live_power_coeff;
    let linear_constant =
        base_constant as i128 * base_denominator + live_multiplier * live_constant;

    let evaluate = |bonus: u32| -> i128 {
        let supported_power = support_ub.saturating_sub(bonus_weight.saturating_mul(bonus));
        let power = power_ub.min(supported_power) as i128;
        (linear_constant + linear_power * power) * (bonus as i128 + 100)
    };

    let mut best = evaluate(0).max(evaluate(max_bonus));
    if support_ub > power_ub {
        let flat_end = ((support_ub - power_ub) / bonus_weight).min(max_bonus);
        best = best.max(evaluate(flat_end));
        if flat_end < max_bonus {
            best = best.max(evaluate(flat_end + 1));
        }
    }

    let quadratic = linear_power * bonus_weight as i128;
    if quadratic > 0 {
        let vertex_numerator =
            linear_constant + linear_power * support_ub as i128 - quadratic * 100;
        if vertex_numerator > 0 {
            let vertex = vertex_numerator / (2 * quadratic);
            for candidate in [vertex - 1, vertex, vertex + 1] {
                if candidate >= 0 && candidate <= max_bonus as i128 {
                    best = best.max(evaluate(candidate as u32));
                }
            }
        }
    }
    best
}

#[inline(always)]
fn ceil_div_i128(numerator: i128, denominator: i128) -> i128 {
    debug_assert!(numerator >= 0 && denominator > 0);
    numerator.saturating_add(denominator - 1) / denominator
}

/// 单卡在场景 (allowed_full_units, attr_full) 下的综合力上界（对该场景精确）。
#[inline(always)]
pub(crate) fn card_scenario_power(
    pool: &CardPool,
    card: CardIdx,
    allowed: u8,
    attr_full: bool,
) -> u32 {
    let mask = pool.unit_mask_raw(card);
    let lut = pool.power_lut(card);
    let values = pool.power_values(card);
    let mut best = 0u32;
    let mut unit = 0usize;
    while unit < 6 {
        if mask & (1u8 << unit) != 0 {
            let slot = ((lut >> (16 + unit)) & 1) as usize;
            let unit_all = (allowed & (1u8 << unit) != 0) as usize;
            let key = unit_all * 2 + attr_full as usize;
            let value = super::evaluate::decode_u18(values, lut, slot * 4 + key);
            if value > best {
                best = value;
            }
        }
        unit += 1;
    }
    best
}

fn build_noev_tables(pool: &CardPool) -> Vec<[u32; CHAR_MASK_COUNT]> {
    let mut tables = vec![[0u32; CHAR_MASK_COUNT]; 64 * 7];
    for card in pool.indices() {
        let ch = pool.char_id(card) as usize;
        let card_attr = pool.attr(card) as usize;
        for allowed in 0..64usize {
            for attr_opt in 0..7usize {
                if attr_opt < 6 && card_attr != attr_opt {
                    // 全同属性 attr_opt 的卡组不可能包含此卡
                    continue;
                }
                let value = card_scenario_power(pool, card, allowed as u8, attr_opt < 6);
                let entry = &mut tables[allowed * 7 + attr_opt][ch];
                if value > *entry {
                    *entry = value;
                }
            }
        }
    }
    tables
}

#[inline(always)]
fn insert_topk_u32_n(values: &mut [u32; DECK_SIZE], value: u32, len: usize) {
    let mut slot = 0usize;
    while slot < len {
        if value > values[slot] {
            let mut shift = len - 1;
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
