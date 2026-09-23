use std::mem::size_of;

use crate::pool::{CardIdx, CardPool};
use crate::types::{DECK_SIZE, ScoreTarget};

use super::context::SearchContext;
use super::evaluate::calc_mysekai_internal;
use super::objective::{LIVE_SCORE_BOUND_SCALE, ObjectiveBound};

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
    /// 已选卡的综合力合计。
    pub power: u32,
    /// 已选卡的技能加成合计。
    pub skill: u32,
    /// 已选卡的活动加成合计，单位为 0.1%。
    pub bonus: u32,
    /// 已选卡里最高的单卡技能值，用于队长位取值。
    pub max_skill: u8,
    /// 已选卡里享受 limited bonus 的张数，用于对照计入上限。
    pub limited_count: u8,
}

/// 角色感知后缀上界。
#[derive(Clone, Debug, PartialEq)]
pub struct SuffixBound {
    objective: ObjectiveBound,
    is_world_bloom: bool,
    attr_matching: bool,
    is_final_chapter: bool,
    limited_bonus_cap: usize,
    extra_bonus_ub: u32,
    diff_attr_bonus: [u16; 6],
    support_cards: Vec<(u16, f64)>,
    support_count: usize,
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
    /// World Bloom dense suffix: for each attr, bitset of characters having at
    /// least one card of that attr at/after the dense index.  This feeds an
    /// exact 5x27 bipartite matching relaxation for reachable attribute count.
    dense_attr_char_tail: Vec<[u32; 5]>,
    /// Score/no-event 场景表：[allowed_unit_subset(64) * 7 + attr_opt] -> per-char max。
    /// attr_opt: 0..6 = 全同属性 attr id，6 = 无全同属性。空表示未启用。
    noev_tables: Vec<[u32; CHAR_MASK_COUNT]>,
}

const _: () = assert!(size_of::<SuffixBound>() <= 736);

/// One support relaxation for every possible leader. For every game ID the
/// envelope keeps the largest profile bonus, and its count is at least every
/// profile's count. Removing any set of main-deck IDs preserves that pointwise
/// dominance, so the remaining top-count sum bounds every real support deck.
/// The envelope is constructed once; DFS never scans per-leader profiles.
fn support_upper_envelope(pool: &CardPool, ctx: &SearchContext) -> (Vec<(u16, f64)>, usize) {
    if !ctx.is_final_chapter {
        return (
            ctx.support_deck.cards.clone(),
            ctx.support_deck.count as usize,
        );
    }
    let fixed_leader = ctx.final_chapter_leader_character().or_else(|| {
        ctx.fixed_card_at(0).and_then(|game_id| {
            pool.indices()
                .find(|&card| pool.game_id(card) == game_id)
                .map(|card| pool.char_id(card))
        })
    });
    if let Some(character) = fixed_leader {
        let profile = ctx.support_deck_for_leader(character);
        return (profile.cards.clone(), profile.count as usize);
    }
    let mut bonuses = std::collections::BTreeMap::<u16, f64>::new();
    let mut count = 0usize;
    let mut include = |profile: &super::context::SupportDeck| {
        count = count.max(profile.count as usize);
        for &(game_id, bonus) in &profile.cards {
            let maximum = bonuses.entry(game_id).or_default();
            *maximum = maximum.max(bonus);
        }
    };
    let mut seen = UsedSet::new();
    for card in pool.indices() {
        let character = pool.char_id(card);
        if !seen.contains(character) {
            include(ctx.support_deck_for_leader(character));
            seen.insert(character);
        }
    }
    let mut cards: Vec<_> = bonuses.into_iter().collect();
    cards.sort_unstable_by(|left, right| {
        right
            .1
            .total_cmp(&left.1)
            .then_with(|| left.0.cmp(&right.0))
    });
    (cards, count)
}

fn world_bloom_extra_bonus_fallback(
    ctx: &SearchContext,
    support_cards: &[(u16, f64)],
    support_count: usize,
) -> u32 {
    if !ctx.is_world_bloom {
        return ctx.extra_bonus_ub;
    }
    let diff = ctx.diff_attr_bonus.iter().copied().max().unwrap_or(0) as u32;
    let support = support_cards
        .iter()
        .take(support_count)
        .map(|(_, bonus)| *bonus)
        .sum::<f64>()
        .ceil()
        .clamp(0.0, u32::MAX as f64) as u32;
    ctx.extra_bonus_ub.max(diff.saturating_add(support))
}

impl SuffixBound {
    /// The objective relaxation shared by every aggregate ceiling.
    #[inline(always)]
    pub(crate) fn objective(&self) -> &ObjectiveBound {
        &self.objective
    }
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

        let (support_cards, support_count) = support_upper_envelope(pool, ctx);
        let extra_bonus_ub = world_bloom_extra_bonus_fallback(ctx, &support_cards, support_count);

        Self {
            objective: ObjectiveBound::from_context(ctx),
            is_world_bloom: ctx.is_world_bloom,
            attr_matching: super::tuning::SearchTuning::load().world_bloom_attr_matching,
            is_final_chapter: ctx.is_final_chapter,
            limited_bonus_cap: ctx.card_bonus_count_limit,
            extra_bonus_ub,
            diff_attr_bonus: ctx.diff_attr_bonus,
            support_cards,
            support_count,
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
            dense_attr_char_tail: if ctx.is_world_bloom {
                build_dense_attr_char_tail(pool)
            } else {
                Vec::new()
            },
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
    pub(crate) fn upper_bound_score_noevent_numerator(
        &self,
        pool: &CardPool,
        chosen: &[CardIdx],
        used_chars: &UsedSet,
        partial: &PartialDeck,
        slots_left: usize,
    ) -> i64 {
        debug_assert!(matches!(self.objective.target, ScoreTarget::Score));
        debug_assert!(!self.objective.has_event);
        if self.noev_tables.is_empty() {
            let packed = self.upper_bound_for_slots(slots_left, used_chars, partial);
            let live = packed as u32;
            debug_assert_eq!(packed >> 32, live as u64);
            return live as i64 * LIVE_SCORE_BOUND_SCALE;
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

        let mut best = self.noev_scenario_live_numerator(
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
                let ub = self.noev_scenario_live_numerator(
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
            let ub = self.noev_scenario_live_numerator(
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
    fn noev_scenario_live_numerator(
        &self,
        pool: &CardPool,
        chosen: &[CardIdx],
        allowed: u8,
        attr_opt: usize,
        used: u32,
        slots_left: usize,
        total_skill: u32,
        leader_ub: u32,
    ) -> i64 {
        let attr_full = attr_opt < 6;
        let mut power = 0u32;
        let mut idx = 0usize;
        while idx < chosen.len() {
            power += card_scenario_power(pool, chosen[idx], allowed, attr_full);
            idx += 1;
        }
        power += self.noev_tail(allowed, attr_opt, used, slots_left);
        self.objective
            .score_noevent_live_numerator_ceiling(power, total_skill, leader_ub)
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
        match self.objective.target {
            ScoreTarget::Power => self.objective.clamp_power_total(
                partial.power
                    + suffix_sum_u32(
                        &self.power_order,
                        &self.power_vals,
                        used_chars.bits(),
                        slots_left,
                    )
                    + self.objective.honor_bonus,
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
                let total_power = self.objective.clamp_power_total(
                    partial.power
                        + suffix_sum_u32(
                            &self.power_order,
                            &self.power_vals,
                            used_chars.bits(),
                            slots_left,
                        )
                        + self.objective.honor_bonus,
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
                let live_score =
                    self.objective
                        .calc_live_score_bound(total_power, total_skill, leader_ub);
                (((total_bonus.saturating_mul(2)) as u64) << 32) | (live_score.max(0) as u32 as u64)
            }
            ScoreTarget::Score => {
                let total_power = self.objective.clamp_power_total(
                    partial.power
                        + suffix_sum_u32(
                            &self.power_order,
                            &self.power_vals,
                            used_chars.bits(),
                            slots_left,
                        )
                        + self.objective.honor_bonus,
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
                let live_score =
                    self.objective
                        .calc_live_score_bound(total_power, total_skill, leader_ub);
                let event_point = self
                    .objective
                    .calc_event_point_bound(live_score, total_bonus);
                ((event_point as u64) << 32) | (live_score as u32 as u64)
            }
            ScoreTarget::Mysekai => {
                let total_power = self.objective.clamp_power_total(
                    partial.power
                        + suffix_sum_u32(
                            &self.power_order,
                            &self.power_vals,
                            used_chars.bits(),
                            slots_left,
                        )
                        + self.objective.honor_bonus,
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

    /// Score/no-event dense-aware suffix ceiling in pre-division numerator units.
    #[inline(always)]
    pub(crate) fn score_noevent_dense_live_numerator_ceiling(
        &self,
        dense_start: usize,
        partial: &PartialDeck,
        slots: usize,
    ) -> i64 {
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
        self.objective.score_noevent_live_numerator_ceiling(
            partial.power + tail_power,
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
        self.objective.ceiling(
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
        self.objective.ceiling_multi_score_event(
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
        self.objective.ceiling(
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
        self.objective.ceiling(
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
        self.objective.ceiling_multi_score_event(
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
            .saturating_add(self.objective.honor_bonus)
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
            .saturating_add(self.objective.honor_bonus)
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
            .saturating_add(self.objective.honor_bonus)
            .saturating_add(bonus_weight.saturating_mul(self.extra_bonus_ub));
        let max_power = self.objective.clamp_power_total(
            self.dense_power_tail
                .first()
                .map(|tail| tail[DECK_SIZE])
                .unwrap_or(0)
                .saturating_add(self.objective.honor_bonus),
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
            table.push(self.objective.joint_event_point_upper(
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
        self.objective.ceiling(
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
        dense_start: usize,
        used_chars: u32,
    ) -> u32 {
        if !self.is_world_bloom {
            return self.extra_bonus_ub;
        }

        let current_attrs = attr_set.count_ones() as usize;
        let novel_ub = self.reachable_novel_attr_ub(attr_set, dense_start, used_chars, rest);
        let max_attrs = (current_attrs + novel_ub).min(DECK_SIZE);
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
        dense_start: usize,
        used_chars: u32,
    ) -> u32 {
        if !self.is_world_bloom {
            return self.extra_bonus_ub;
        }

        let current_attrs = attr_set.count_ones() as usize;
        let novel_ub = self.reachable_novel_attr_ub(attr_set, dense_start, used_chars, rest);
        let max_attrs = (current_attrs + novel_ub).min(DECK_SIZE);
        let mut diff_ub = 0u32;
        let mut count = current_attrs;
        while count <= max_attrs {
            diff_ub = diff_ub.max(self.diff_attr_bonus[count] as u32);
            count += 1;
        }

        let support_sum = self.support_sum_excluding(selected, selected_len);

        diff_ub + support_sum.ceil() as u32
    }

    /// Maximum number of NEW attributes that any legal completion can add,
    /// relaxed only by constraints unrelated to (character, attr, dense start).
    /// Every feasible completion induces a matching from its novel attributes to
    /// distinct unused characters; therefore maximum bipartite matching is an
    /// admissible upper bound on attribute diversity.
    #[inline]
    fn reachable_novel_attr_ub(
        &self,
        attr_set: u8,
        dense_start: usize,
        used_chars: u32,
        rest: usize,
    ) -> usize {
        if !self.attr_matching {
            return rest;
        }
        if rest == 0 || self.dense_attr_char_tail.is_empty() {
            return if self.dense_attr_char_tail.is_empty() {
                rest
            } else {
                0
            };
        }
        let Some(masks) = self.dense_attr_char_tail.get(dense_start) else {
            return 0;
        };
        let mut owner = [u8::MAX; 27];
        let mut matched = 0usize;
        for attr in 0..5usize {
            if attr_set & (1u8 << attr) != 0 {
                continue;
            }
            let available = masks[attr] & !used_chars;
            if available == 0 {
                continue;
            }
            let mut seen = 0u32;
            if augment_attr_matching(attr as u8, masks, used_chars, &mut owner, &mut seen) {
                matched += 1;
                if matched >= rest {
                    return rest;
                }
            }
        }
        matched.min(rest)
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
            if (selected_len > 0 && selected[0] == game_id)
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
                || (selected_len > 0 && selected[0] == game_id)
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

fn build_dense_attr_char_tail(pool: &CardPool) -> Vec<[u32; 5]> {
    let n = pool.count();
    let mut tail = vec![[0u32; 5]; n + 1];
    let mut dense = n;
    while dense > 0 {
        dense -= 1;
        tail[dense] = tail[dense + 1];
        let card = CardIdx::new(dense as u16);
        let attr = pool.attr(card) as usize;
        let char_id = pool.char_id(card);
        if attr < 5 && char_id < 27 {
            tail[dense][attr] |= 1u32 << char_id;
        }
    }
    tail
}

#[inline]
fn augment_attr_matching(
    attr: u8,
    masks: &[u32; 5],
    used_chars: u32,
    owner: &mut [u8; 27],
    seen_chars: &mut u32,
) -> bool {
    let mut available = masks[attr as usize] & !used_chars & !*seen_chars;
    while available != 0 {
        let char_id = available.trailing_zeros() as usize;
        let bit = 1u32 << char_id;
        available &= available - 1;
        *seen_chars |= bit;
        let previous = owner[char_id];
        if previous == u8::MAX
            || augment_attr_matching(previous, masks, used_chars, owner, seen_chars)
        {
            owner[char_id] = attr;
            return true;
        }
    }
    false
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

#[cfg(test)]
mod support_envelope_tests {
    use super::*;
    use crate::search::objective::ceil_div_positive;
    use crate::types::{LiveSkillOrder, LiveType};
    use crate::pool::PoolBuilder;
    use crate::search::SupportDeck;
    use crate::types::{EventType, SkillReferenceStrategy};

    fn fixture() -> (CardPool, SearchContext) {
        let mut builder = PoolBuilder::new(3);
        for dense in 0..3u16 {
            builder.set_game_id(dense, 101 + dense);
            builder.set_char_id(dense, dense as u8 + 1);
            builder.set_attr(dense, dense as u8);
        }
        let pool = builder.freeze();
        let mut profiles = vec![SupportDeck::default(); 27];
        profiles[1] = SupportDeck {
            cards: vec![(1, 15.25), (2, 12.5), (3, 9.75), (7, 1.0)],
            count: 2,
        };
        profiles[2] = SupportDeck {
            cards: vec![(2, 18.5), (5, 7.25), (6, 6.0), (1, 3.0)],
            count: 3,
        };
        // Character 3 has no active profile and therefore uses the fallback.
        // Character 26 is not a possible leader in this pool.
        profiles[26] = SupportDeck {
            cards: vec![(99, 10_000.0)],
            count: 5,
        };
        let ctx = SearchContext {
            target: ScoreTarget::Score,
            fixed_card_ids: Vec::new(),
            fixed_character_ids: Vec::new(),
            forced_leader_character_id: None,
            music_rate_pct: 100,
            boost_rate_pct: 100,
            base_score: 1.0,
            base_score_auto: 1.0,
            fever_score: 0.0,
            skill_scores: [[0.0; 6]; 3],
            other_score: 0,
            life: 1000,
            diff_attr_bonus: [0; 6],
            support_deck: SupportDeck {
                cards: vec![(7, 11.0), (4, 4.0), (6, 2.0)],
                count: 2,
            },
            support_decks_by_character: profiles,
            is_world_bloom: true,
            is_final_chapter: true,
            enforce_char_uniqueness: true,
            minimize: false,
            live_type: LiveType::Multi,
            event_type: Some(EventType::WorldBloom),
            keep_after_training_state: true,
            skill_reference_strategy: SkillReferenceStrategy::Average,
            best_skill_as_leader: false,
            live_skill_order: LiveSkillOrder::Average,
            specific_skill_order: None,
            multi_teammate_score_up: None,
            multi_teammate_power: None,
            multi_live_score_up_lower_bound: None,
            extra_bonus_ub: 0,
            w_power: 2.0,
            w_bonus: 1.0,
            skill_ub_global: 0,
            card_bonus_count_limit: 4,
            honor_bonus: 0,
            power_total_cap: None,
            leader_honor_bonus_x10: vec![0; 3],
            leader_limit_bonus_x10: vec![0; 3],
            final_chapter_member_keep: vec![true; 3],
            skill_is_after_training: vec![false; 3],
            trained_to_special_image: vec![false; 3],
        };
        (pool, ctx)
    }

    fn profile_sum(profile: &SupportDeck, excluded: &[u16]) -> f64 {
        profile
            .cards
            .iter()
            .filter(|(id, _)| !excluded.contains(id))
            .take(profile.count as usize)
            .map(|(_, bonus)| bonus)
            .sum()
    }

    #[test]
    fn final_support_envelope_dominates_every_profile_after_every_exclusion() {
        let (pool, ctx) = fixture();
        let suffix = SuffixBound::build(&pool, &ctx);
        assert_eq!(suffix.support_count, 3);
        assert!(!suffix.support_cards.iter().any(|(id, _)| *id == 99));
        // Every selected subset of the support-ID universe, including empty
        // prefixes with a nonzero unused array slot, and every next candidate.
        for mask in 0..128u32 {
            let excluded: Vec<u16> = (1..=7u16)
                .filter(|id| mask & (1 << (id - 1)) != 0)
                .collect();
            if excluded.len() > DECK_SIZE {
                continue;
            }
            let mut selected = [7; DECK_SIZE];
            selected[..excluded.len()].copy_from_slice(&excluded);
            for leader in 1..=3 {
                let profile = ctx.support_deck_for_leader(leader);
                let expected = profile_sum(profile, &excluded);
                assert!(suffix.support_sum_excluding(&selected, excluded.len()) >= expected);
                assert!(
                    f64::from(suffix.world_bloom_extra_bonus_bound_from_parts(
                        0,
                        &selected,
                        excluded.len(),
                        0,
                        0,
                        0,
                    )) >= expected
                );
                if excluded.len() == DECK_SIZE {
                    continue;
                }
                for candidate in 1..=7 {
                    let mut with_candidate = excluded.clone();
                    with_candidate.push(candidate);
                    let expected = profile_sum(profile, &with_candidate);
                    assert!(
                        suffix.support_sum_excluding_candidate(
                            &selected,
                            excluded.len(),
                            candidate,
                        ) >= expected
                    );
                    assert!(
                        f64::from(suffix.world_bloom_extra_bonus_bound_for_candidate_parts(
                            0,
                            &selected,
                            excluded.len(),
                            candidate,
                            0,
                            0,
                            0,
                        )) >= expected
                    );
                }
            }
        }
    }

    #[test]
    fn fixed_leader_and_ordinary_support_keep_the_effective_single_profile() {
        let (pool, base) = fixture();
        for leader in 1..=3u8 {
            for constraint in 0..3 {
                let mut ctx = base.clone();
                match constraint {
                    0 => ctx.forced_leader_character_id = Some(leader),
                    1 => ctx.fixed_character_ids = vec![leader],
                    _ => ctx.fixed_card_ids = vec![100 + u16::from(leader)],
                }
                let suffix = SuffixBound::build(&pool, &ctx);
                let actual = ctx.support_deck_for_leader(leader);
                assert_eq!(suffix.support_cards, actual.cards);
                assert_eq!(suffix.support_count, actual.count as usize);
            }
        }
        let mut ordinary = base;
        ordinary.is_final_chapter = false;
        let suffix = SuffixBound::build(&pool, &ordinary);
        assert_eq!(suffix.support_cards, ordinary.support_deck.cards);
        assert_eq!(suffix.support_count, ordinary.support_deck.count as usize);
    }

    #[test]
    fn world_bloom_fallback_uses_the_envelope_and_preserves_explicit_hint() {
        let (pool, mut ctx) = fixture();
        ctx.diff_attr_bonus = [0, 0, 2, 4, 8, 10];
        let suffix = SuffixBound::build(&pool, &ctx);
        let support = suffix.support_sum_excluding(&[7; DECK_SIZE], 0).ceil() as u32;
        assert_eq!(suffix.extra_bonus_ub, support + 10);
        ctx.extra_bonus_ub = suffix.extra_bonus_ub + 100;
        assert_eq!(
            SuffixBound::build(&pool, &ctx).extra_bonus_ub,
            ctx.extra_bonus_ub
        );
    }

    #[test]
    fn signed_rate_division_rounds_toward_positive_infinity() {
        for denominator in [1i64, 100, 500] {
            for numerator in [
                i64::MIN,
                -1001,
                -501,
                -500,
                -499,
                -1,
                0,
                1,
                499,
                500,
                501,
                1001,
                i64::MAX,
            ] {
                let quotient = ceil_div_positive(numerator, denominator);
                let numerator = i128::from(numerator);
                let denominator = i128::from(denominator);
                let quotient = i128::from(quotient);
                assert!(quotient * denominator >= numerator);
                assert!((quotient - 1) * denominator < numerator);
            }
        }
    }
}
