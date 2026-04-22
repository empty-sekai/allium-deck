use crate::pool::{CardIdx, CardPool};

use super::context::SearchContext;
use super::evaluate::decode_u18;

/// dominance 裁剪后的卡池、上下文和原索引映射。
pub struct DominanceResult {
    pub pool: CardPool,
    pub ctx: SearchContext,
    pub original_indices: Vec<CardIdx>,
    pub before: usize,
    pub after: usize,
}

/// 在安全场景下执行逐角色支配裁剪并返回压缩后的卡池。
pub fn eliminate_dominated(pool: &CardPool, ctx: &SearchContext) -> DominanceResult {
    let keep = if ctx.is_world_bloom || ctx.is_final_chapter {
        vec![true; pool.count()]
    } else {
        compute_keep_mask(pool, ctx)
    };
    let before = pool.count();
    let after = keep.iter().copied().filter(|keep| *keep).count();

    let original_indices = keep
        .iter()
        .copied()
        .enumerate()
        .filter_map(|(dense, keep)| keep.then_some(CardIdx::new(dense as u16)))
        .collect::<Vec<_>>();
    let compacted = pool.compact(&keep);
    let remapped_ctx = ctx.remap(&keep);
    assert_eq!(
        remapped_ctx.skill_is_after_training.len(),
        compacted.count(),
        "remapped context must match compacted pool",
    );

    DominanceResult {
        pool: compacted,
        ctx: remapped_ctx,
        original_indices,
        before,
        after,
    }
}

fn compute_keep_mask(pool: &CardPool, ctx: &SearchContext) -> Vec<bool> {
    let mut keep = vec![true; pool.count()];
    let mut char_id = 0u8;
    while (char_id as usize) < 27 {
        let cards: Vec<CardIdx> = pool
            .indices()
            .filter(|&idx| pool.char_id(idx) == char_id)
            .collect();
        let mut left = 0usize;
        while left < cards.len() {
            let a = unsafe { *cards.get_unchecked(left) };
            if !keep[a.raw()] {
                left += 1;
                continue;
            }
            let mut right = 0usize;
            while right < cards.len() {
                if left != right {
                    let b = unsafe { *cards.get_unchecked(right) };
                    if keep[b.raw()]
                        && !ctx.is_fixed_game_id(pool.game_id(b))
                        && dominates(pool, a, b)
                    {
                        keep[b.raw()] = false;
                    }
                }
                right += 1;
            }
            left += 1;
        }
        char_id += 1;
    }
    keep
}

fn dominates(pool: &CardPool, lhs: CardIdx, rhs: CardIdx) -> bool {
    debug_assert_eq!(pool.char_id(lhs), pool.char_id(rhs));

    let lhs_values = pool.power_values(lhs);
    let rhs_values = pool.power_values(rhs);
    let lhs_lut = pool.power_lut(lhs);
    let rhs_lut = pool.power_lut(rhs);
    let mut idx = 0usize;
    while idx < 8 {
        if decode_u18(lhs_values, lhs_lut, idx) < decode_u18(rhs_values, rhs_lut, idx) {
            return false;
        }
        idx += 1;
    }

    if !skill_dominates(pool, lhs, rhs) {
        return false;
    }

    let lhs_bonus = pool.event_bonus(lhs);
    let rhs_bonus = pool.event_bonus(rhs);
    if lhs_bonus.base_bonus < rhs_bonus.base_bonus
        || lhs_bonus.limited_bonus < rhs_bonus.limited_bonus
    {
        return false;
    }
    if pool.attr(lhs) != pool.attr(rhs) {
        return false;
    }

    let lhs_mask = pool.unit_mask_raw(lhs);
    let rhs_mask = pool.unit_mask_raw(rhs);
    (rhs_mask & lhs_mask) == rhs_mask
}

fn skill_dominates(pool: &CardPool, lhs: CardIdx, rhs: CardIdx) -> bool {
    let lhs_skill = pool.skill(lhs);
    let rhs_skill = pool.skill(rhs);
    if lhs_skill.skill_type != rhs_skill.skill_type {
        return false;
    }

    match lhs_skill.skill_type {
        0 => lhs_skill.value >= rhs_skill.value,
        1 => {
            let left = pool
                .special()
                .unit_count()
                .get(lhs_skill.value.saturating_sub(1) as usize);
            let right = pool
                .special()
                .unit_count()
                .get(rhs_skill.value.saturating_sub(1) as usize);
            let (Some(left), Some(right)) = (left, right) else {
                return false;
            };
            left.unit == right.unit
                && left
                    .score_up
                    .iter()
                    .zip(right.score_up.iter())
                    .all(|(l, r)| l >= r)
        }
        2 => {
            let left = pool
                .special()
                .diff()
                .get(lhs_skill.value.saturating_sub(1) as usize);
            let right = pool
                .special()
                .diff()
                .get(rhs_skill.value.saturating_sub(1) as usize);
            let (Some(left), Some(right)) = (left, right) else {
                return false;
            };
            left.base >= right.base && left.increment >= right.increment
        }
        3 => {
            let left = pool
                .special()
                .ref_skills()
                .get(lhs_skill.value.saturating_sub(1) as usize);
            let right = pool
                .special()
                .ref_skills()
                .get(rhs_skill.value.saturating_sub(1) as usize);
            let (Some(left), Some(right)) = (left, right) else {
                return false;
            };
            left.rate >= right.rate && left.max >= right.max
        }
        _ => false,
    }
}
