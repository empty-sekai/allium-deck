use std::mem::size_of;

use crate::pool::CardPool;
use crate::types::{LiveType, ScoreTarget, DECK_SIZE};

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
}

/// 角色感知后缀上界。
#[derive(Clone, Debug, PartialEq)]
pub struct SuffixBound {
    target: ScoreTarget,
    effective_live_type: LiveType,
    has_event: bool,
    music_rate_pct: u32,
    boost_rate_pct: u32,
    base_rate: f64,
    skill_rate_sum: f64,
    other_score: i32,
    life: i32,
    multi_teammate_score_up: Option<i32>,
    multi_teammate_power: Option<i32>,
    extra_bonus_ub: u32,
    power_order: [u8; CHAR_MASK_COUNT],
    power_vals: [u32; CHAR_MASK_COUNT],
    skill_order: [u8; CHAR_MASK_COUNT],
    skill_vals: [u16; CHAR_MASK_COUNT],
    bonus_order: [u8; CHAR_MASK_COUNT],
    bonus_vals: [u16; CHAR_MASK_COUNT],
    bonus_targets: Vec<u32>,
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
            let bonus = pool.event_bonus(card).base_bonus as u16
                + pool.event_bonus(card).limited_bonus as u16;
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

        Self {
            target: ctx.target,
            effective_live_type: ctx.effective_live_type(),
            has_event: ctx.has_event(),
            music_rate_pct: ctx.music_rate_pct,
            boost_rate_pct: ctx.boost_rate_pct,
            base_rate: match ctx.effective_live_type() {
                LiveType::Auto | LiveType::ChallengeAuto => ctx.base_score_auto,
                LiveType::Multi | LiveType::Cheerful => ctx.base_score + ctx.fever_score * 0.5,
                _ => ctx.base_score,
            },
            skill_rate_sum: ctx.skill_scores[match ctx.effective_live_type() {
                LiveType::Multi | LiveType::Cheerful => 1,
                LiveType::Auto | LiveType::ChallengeAuto => 2,
                _ => 0,
            }]
            .iter()
            .sum(),
            other_score: ctx.other_score,
            life: ctx.life,
            multi_teammate_score_up: ctx.multi_teammate_score_up,
            multi_teammate_power: ctx.multi_teammate_power,
            extra_bonus_ub: ctx.extra_bonus_ub,
            power_order,
            power_vals: power_order.map(|char_id| power_per_char[char_id as usize]),
            skill_order,
            skill_vals: skill_order.map(|char_id| skill_per_char[char_id as usize]),
            bonus_order,
            bonus_vals: bonus_order.map(|char_id| bonus_per_char[char_id as usize]),
            bonus_targets: ctx.bonus_targets.clone(),
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
            ScoreTarget::Power => {
                partial.power as u64
                    + suffix_sum_u32(
                        &self.power_order,
                        &self.power_vals,
                        used_chars.bits(),
                        slots_left,
                    ) as u64
            }
            ScoreTarget::Skill => {
                let total_skill = partial.skill
                    + suffix_sum_u16_as_u32(
                        &self.skill_order,
                        &self.skill_vals,
                        used_chars.bits(),
                        slots_left,
                    );
                let best_unused = first_unused_val_u16(
                    &self.skill_order, &self.skill_vals, used_chars.bits(),
                );
                let leader_ub = (partial.max_skill as u32).max(best_unused as u32);
                (2 * total_skill + 8 * leader_ub) as u64
            }
            ScoreTarget::Bonus => {
                let max_bonus = partial.bonus
                    + suffix_sum_u16_as_u32(
                        &self.bonus_order,
                        &self.bonus_vals,
                        used_chars.bits(),
                        slots_left,
                    )
                    + self.extra_bonus_ub;
                let target_bonus = if self.bonus_targets.is_empty() {
                    max_bonus
                } else {
                    let Some(target_bonus) = self
                        .bonus_targets
                        .iter()
                        .copied()
                        .filter(|target| *target >= partial.bonus && *target <= max_bonus)
                        .max()
                    else {
                        return 0;
                    };
                    target_bonus
                };
                let total_power = partial.power
                    + suffix_sum_u32(
                        &self.power_order,
                        &self.power_vals,
                        used_chars.bits(),
                        slots_left,
                    );
                let total_skill = partial.skill
                    + suffix_sum_u16_as_u32(
                        &self.skill_order,
                        &self.skill_vals,
                        used_chars.bits(),
                        slots_left,
                    );
                let best_unused = first_unused_val_u16(
                    &self.skill_order, &self.skill_vals, used_chars.bits(),
                );
                let leader_ub = (partial.max_skill as u32).max(best_unused as u32);
                let live_score = self.calc_live_score_bound(total_power, total_skill, leader_ub);
                let event_point = self.calc_event_point_bound(live_score, target_bonus);
                ((target_bonus as u64) << 48)
                    | ((event_point as u32 as u64) << 24)
                    | ((live_score as u32 as u64) & 0x00ff_ffff)
            }
            ScoreTarget::Score => {
                let total_power = partial.power
                    + suffix_sum_u32(
                        &self.power_order,
                        &self.power_vals,
                        used_chars.bits(),
                        slots_left,
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
                let best_unused = first_unused_val_u16(
                    &self.skill_order, &self.skill_vals, used_chars.bits(),
                );
                let leader_ub = (partial.max_skill as u32).max(best_unused as u32);
                let live_score = self.calc_live_score_bound(total_power, total_skill, leader_ub);
                let event_point = self.calc_event_point_bound(live_score, total_bonus);
                ((event_point as u64) << 32) | (live_score as u32 as u64)
            }
            ScoreTarget::Mysekai => {
                let total_power = partial.power
                    + suffix_sum_u32(
                        &self.power_order,
                        &self.power_vals,
                        used_chars.bits(),
                        slots_left,
                    );
                let total_bonus = partial.bonus
                    + suffix_sum_u16_as_u32(
                        &self.bonus_order,
                        &self.bonus_vals,
                        used_chars.bits(),
                        slots_left,
                    )
                    + self.extra_bonus_ub;
                calc_mysekai_internal(total_power, total_bonus) as u64
            }
        }
    }

    /// 预计算同层 suffix 分量，供 DFS 循环内廉价 ceiling 使用。
    #[inline(always)]
    pub(crate) fn precompute_layer(
        &self,
        used: &UsedSet,
        slots: usize,
    ) -> LayerPrecomputed {
        LayerPrecomputed {
            suffix_power: suffix_sum_u32(
                &self.power_order, &self.power_vals, used.bits(), slots,
            ),
            suffix_bonus: suffix_sum_u16_as_u32(
                &self.bonus_order, &self.bonus_vals, used.bits(),
                slots.saturating_sub(1),
            ),
            extra_bonus_ub: self.extra_bonus_ub,
            skill_ub: suffix_sum_u16_as_u32(
                &self.skill_order, &self.skill_vals, used.bits(), slots,
            ),
            best_unused_skill: first_unused_val_u16(
                &self.skill_order, &self.skill_vals, used.bits(),
            ),
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
        match self.target {
            ScoreTarget::Power => power_ub as u64,
            ScoreTarget::Skill => {
                (2 * skill_ub + 8 * leader_ub) as u64
            }
            ScoreTarget::Score => {
                let live = self.calc_live_score_bound(power_ub, skill_ub, leader_ub);
                let ep = self.calc_event_point_bound(live, bonus_total);
                ((ep as u64) << 32) | (live as u32 as u64)
            }
            ScoreTarget::Bonus => {
                let live = self.calc_live_score_bound(power_ub, skill_ub, leader_ub);
                let ep = self.calc_event_point_bound(live, bonus_total);
                ((bonus_total as u64) << 48)
                    | ((ep as u32 as u64) << 24)
                    | ((live as u32 as u64) & 0x00ff_ffff)
            }
            ScoreTarget::Mysekai => {
                calc_mysekai_internal(power_ub, bonus_total) as u64
            }
        }
    }

    #[inline(always)]
    fn calc_live_score_bound(&self, power_total: u32, skill_total: u32, leader_ub: u32) -> i32 {
        let max_slot_score = match self.effective_live_type {
            LiveType::Multi | LiveType::Cheerful => {
                let self_bound = 0.8 * leader_ub as f64 + 0.2 * skill_total as f64;
                self_bound.max(self.multi_teammate_score_up.unwrap_or(0) as f64)
            }
            _ => skill_total as f64,
        };
        let rate = self.base_rate + max_slot_score * self.skill_rate_sum / 100.0;
        let total_power_i32 = power_total as i32;
        let power_sum = if let Some(teammate_power) = self.multi_teammate_power {
            total_power_i32 + teammate_power * (DECK_SIZE as i32 - 1)
        } else {
            DECK_SIZE as i32 * total_power_i32
        };
        let active_bonus = if matches!(self.effective_live_type, LiveType::Multi) {
            DECK_SIZE as f64 * 0.015 * power_sum as f64
        } else {
            0.0
        };
        match self.effective_live_type {
            LiveType::Mysekai => 0,
            _ => (rate * power_total as f64 * 4.0 + active_bonus) as i32,
        }
    }

    #[inline(always)]
    fn calc_event_point_bound(&self, live_score: i32, total_bonus: u32) -> i32 {
        if !self.has_event {
            return live_score;
        }
        let music_rate = self.music_rate_pct as f64 / 100.0;
        let deck_rate = total_bonus as f64 / 100.0 + 1.0;
        let boost_rate = self.boost_rate_pct as f64 / 100.0;

        match self.effective_live_type {
            LiveType::Challenge | LiveType::ChallengeAuto => (100 + live_score / 20_000) * 120,
            LiveType::Solo | LiveType::Auto => {
                let base_score = 100 + live_score / 20_000;
                ((base_score as f64 * music_rate * deck_rate) as i32 as f64 * boost_rate) as i32
            }
            LiveType::Multi => {
                let other_score = if self.other_score == 0 {
                    live_score.saturating_mul(4)
                } else {
                    self.other_score
                };
                let base_score =
                    110 + (live_score as f64 / 17_000.0) as i32 + (other_score / 340_000).min(13);
                ((base_score as f64 * music_rate * deck_rate) as i32 as f64 * boost_rate) as i32
            }
            LiveType::Cheerful => {
                let other_score = if self.other_score == 0 {
                    live_score.saturating_mul(4)
                } else {
                    self.other_score
                };
                let base_score =
                    110 + (live_score as f64 / 17_000.0) as i32 + (other_score / 340_000).min(13);
                let life_rate = 1.15 + (self.life as f64 / 5000.0).clamp(0.1, 0.2);
                let inner = (base_score as f64 * music_rate * deck_rate) as i32;
                ((inner as f64 * life_rate) as i32 as f64 * boost_rate) as i32
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

/// 同层预计算的 suffix 分量，用于廉价 ceiling 计算。
#[derive(Clone, Copy, Debug)]
pub(crate) struct LayerPrecomputed {
    /// 全 slots 的 power suffix sum（松弛上界）。
    pub suffix_power: u32,
    /// 剩余 slots-1 的 bonus suffix sum（不含候选卡的 bonus，不含 extra）。
    pub suffix_bonus: u32,
    /// WL 等额外 bonus 上界。
    pub extra_bonus_ub: u32,
    /// 全 slots 的 skill suffix sum。
    pub skill_ub: u32,
    /// 未使用角色中最大 skill。
    pub best_unused_skill: u16,
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

const CHAR_MASK_COUNT: usize = 27;
