use crate::pool::{CardIdx, CardPool};

use super::context::SearchContext;
use super::evaluate::decode_u18;

/// dominance 裁剪后的卡池、上下文和原索引映射。
pub struct DominanceResult {
    /// 裁剪后的卡池。
    pub pool: CardPool,
    /// 与 `pool` 对应、已重映射的搜索上下文。
    pub ctx: SearchContext,
    /// 裁剪后的 dense 索引 -> 原卡池 dense 索引。
    pub original_indices: Vec<CardIdx>,
    /// 原 dense 索引 -> 被该卡（直接或经支配链传递）支配而裁掉的原索引列表。
    /// 仅存活卡的条目非空，供 Top-K 搜索后的替代展开使用。
    pub alternatives: Vec<Vec<CardIdx>>,
    /// 裁剪前的卡数。
    pub before: usize,
    /// 裁剪后的卡数。
    pub after: usize,
}

/// 执行逐角色支配裁剪并返回压缩后的卡池。
///
/// WL 同样走支配裁剪：被裁的卡仍在独立的 support_cards 里参与支援计算（支援与主搜索池解耦），
/// 且 `dominates` 要求 attr 相同，异色变体全部保留，diff_attr_bonus 无损。
pub fn eliminate_dominated(pool: &CardPool, ctx: &SearchContext) -> DominanceResult {
    // WL 下支配还需支援维度可承担（issue #23）：支援表内的卡编入队伍会让出支援位，
    // 支援盲的支配会裁掉真实最优卡组里的卡，Top-1 都可能出错。
    // 差额不再一票否决，而是允许用支配者多出的活动加成抵扣（见 support_deficit_affordable）。
    // Position-sensitive objectives do not inherit the exchangeable-slot
    // dominance proof. Preserve their complete candidate frontier.
    let position_sensitive = super::problem::DeckProblem::from_context(ctx)
        .needs_placement_search()
        || !super::tuning::SearchTuning::load().dominance;
    let support = (!position_sensitive)
        .then(|| support_dimension(pool, ctx))
        .flatten();
    let (keep, dominated_by) = if position_sensitive {
        (vec![true; pool.count()], vec![0u16; pool.count()])
    } else {
        compute_keep_mask_with_winners(pool, ctx, support.as_ref())
    };
    let before = pool.count();
    let after = keep.iter().copied().filter(|keep| *keep).count();
    let alternatives = chain_compress_alternatives(&keep, &dominated_by);

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
        alternatives,
        before,
        after,
    }
}

/// 链压缩：被裁卡沿「被谁裁掉」链走到存活根。支配关系逐维度比较、可传递，
/// 因此根支配它链上的每一张被裁卡。
fn chain_compress_alternatives(keep: &[bool], dominated_by: &[u16]) -> Vec<Vec<CardIdx>> {
    let mut alternatives = vec![Vec::new(); keep.len()];
    let mut dense = 0usize;
    while dense < keep.len() {
        if !keep[dense] {
            let mut root = dominated_by[dense] as usize;
            while !keep[root] {
                root = dominated_by[root] as usize;
            }
            alternatives[root].push(CardIdx::new(dense as u16));
        }
        dense += 1;
    }
    alternatives
}

/// 终章 member 位支配裁剪的保留位图与替代记录。
pub struct MemberDominance {
    /// 按原 dense 索引标记该卡是否在 member 位保留。
    pub keep: Vec<bool>,
    /// member 位存活根 -> 被其（直接或经链传递）member 位支配裁掉的索引列表。
    pub alternatives: Vec<Vec<CardIdx>>,
}

/// Completion-safe support opportunity cost, including reserve entries and all
/// cultivation variants of each public card identity.
///
/// Let R be the other four main cards and t the q-th support value after removing
/// R, A and B. Replacing B by A loses at most
/// `(a - t)+ - (b - t)+ <= max(0, a - max(b, floor))`, where floor is the global
/// support value at one-based rank q+5. Removing at most five cards cannot push
/// the q-th remaining value below that rank. Outward rounding preserves the
/// inequality: a is rounded up, b and floor down. Taking the worst leader
/// profile is safe for every legal completion.
struct SupportDimension {
    upper_x100: Vec<[i32; 27]>,
    lower_x100: Vec<[i32; 27]>,
    replacement_floor_x100: [i32; 27],
}

impl SupportDimension {
    #[inline(always)]
    fn deficit_x100(&self, lhs: CardIdx, rhs: CardIdx) -> i32 {
        (0..27)
            .map(|character| {
                self.upper_x100[lhs.raw()][character]
                    - self.lower_x100[rhs.raw()][character]
                        .max(self.replacement_floor_x100[character])
            })
            .max()
            .unwrap_or(0)
            .max(0)
    }
}

fn support_dimension(pool: &CardPool, ctx: &SearchContext) -> Option<SupportDimension> {
    if !ctx.is_world_bloom {
        return None;
    }
    let mut dense_by_game_id = std::collections::HashMap::<u16, Vec<usize>>::new();
    for card in pool.indices() {
        dense_by_game_id
            .entry(pool.game_id(card))
            .or_default()
            .push(card.raw());
    }
    let mut upper_x100 = vec![[0i32; 27]; pool.count()];
    let mut lower_x100 = vec![[0i32; 27]; pool.count()];
    let mut replacement_floor_x100 = [0i32; 27];
    let mut any = false;
    let profile_count = if ctx.is_final_chapter { 27 } else { 1 };
    for character in 0u8..profile_count {
        let support = ctx.support_deck_for_leader(character);
        let count = support.count as usize;
        if count == 0 {
            continue;
        }
        replacement_floor_x100[character as usize] = support
            .cards
            .get(count + crate::types::DECK_SIZE - 1)
            .map(|(_, value)| (value * 100.0).floor() as i32)
            .unwrap_or(0);
        // A reserve may enter the counted prefix after another main card is
        // excluded. Recording only the original q entries is not admissible.
        for &(game_id, value) in &support.cards {
            if let Some(variants) = dense_by_game_id.get(&game_id) {
                for &dense in variants {
                    upper_x100[dense][character as usize] = (value * 100.0).ceil() as i32;
                    lower_x100[dense][character as usize] = (value * 100.0).floor() as i32;
                }
                any = true;
            }
        }
    }
    any.then_some(SupportDimension {
        upper_x100,
        lower_x100,
        replacement_floor_x100,
    })
}

/// 终章 member 位支配裁剪：用中性 ctx（忽略队长专属称号/当期加成）逐角色比较，
/// 裁掉仅剩队长价值的卡的 member 用途。固定卡从真实 ctx 继承、永不被裁；
/// WL 支援惩罚从真实 ctx 计入支配维度。被裁卡记录到存活根的 alternatives，
/// 供 Top-K 搜索后按 member 槽位回换（issue #7）。
pub fn compute_member_dominance(pool: &CardPool, ctx: &SearchContext) -> MemberDominance {
    let support = support_dimension(pool, ctx);
    // Ignore leader-only numeric benefits, not the actual skill/training state.
    let mut member_context = ctx.clone();
    member_context.is_final_chapter = false;
    let (keep, dominated_by) =
        compute_keep_mask_with_winners(pool, &member_context, support.as_ref());
    let alternatives = chain_compress_alternatives(&keep, &dominated_by);
    MemberDominance { keep, alternatives }
}

/// The same completion-safe member proof specialized to one leader's support
/// profile. Unlike the global member pass this need not protect other leaders.
pub(super) fn compute_member_dominance_for_leader(
    pool: &CardPool,
    ctx: &SearchContext,
    leader_character: u8,
) -> MemberDominance {
    let mut member_context = ctx.clone();
    member_context.support_deck = ctx.support_deck_for_leader(leader_character).clone();
    member_context.support_decks_by_character.clear();
    member_context.is_final_chapter = false;
    compute_member_dominance(pool, &member_context)
}

/// 返回保留位图与「被谁裁掉」映射：dominated_by[dense] 仅在 keep[dense]=false 时有意义，
/// 记录裁掉该卡的卡的 dense 索引（裁剪者之后仍可能被裁，使用前需链压缩到存活根）。
/// `support` 存在时（WL），支配额外要求支援加成差额能被活动加成盈余抵扣。
fn compute_keep_mask_with_winners(
    pool: &CardPool,
    ctx: &SearchContext,
    support: Option<&SupportDimension>,
) -> (Vec<bool>, Vec<u16>) {
    let mut keep = vec![true; pool.count()];
    let mut dominated_by = vec![0u16; pool.count()];
    if !super::tuning::SearchTuning::load().dominance {
        return (keep, dominated_by);
    }
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
                        && dominates(pool, ctx, a, b)
                        && support_deficit_affordable(pool, support, a, b)
                    {
                        keep[b.raw()] = false;
                        dominated_by[b.raw()] = a.raw() as u16;
                    }
                }
                right += 1;
            }
            left += 1;
        }
        char_id += 1;
    }
    (keep, dominated_by)
}

/// 支援差额是否付得起：`lhs` 顶替 `rhs` 少收的支援加成，必须被它多出来的
/// 卡面活动加成补上。两者在活动加成总和里同为百分比、同为加项（见
/// `evaluate::calc_support_bonus`），可直接相抵。
///
/// 只用 base 盈余作预算：limited 加成受 `card_bonus_count_limit` 约束，未必计入总和。
/// `dominates` 已保证 base 不劣，故盈余非负。
#[inline(always)]
fn support_deficit_affordable(
    pool: &CardPool,
    support: Option<&SupportDimension>,
    lhs: CardIdx,
    rhs: CardIdx,
) -> bool {
    let Some(support) = support else {
        return true;
    };
    let deficit = support.deficit_x100(lhs, rhs);
    if deficit <= 0 {
        return true;
    }
    let surplus_x10 = pool.event_bonus_exact(lhs).base_x10() as i32
        - pool.event_bonus_exact(rhs).base_x10() as i32;
    deficit <= surplus_x10 * 10
}

fn dominates(pool: &CardPool, ctx: &SearchContext, lhs: CardIdx, rhs: CardIdx) -> bool {
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

    if ctx.keep_after_training_state
        && (ctx.skill_is_after_training_at(lhs.raw()) != ctx.skill_is_after_training_at(rhs.raw())
            || ctx.trained_to_special_image_at(lhs.raw())
                != ctx.trained_to_special_image_at(rhs.raw()))
    {
        return false;
    }
    if !skill_dominates(pool, lhs, rhs) {
        return false;
    }

    let lhs_bonus = pool.event_bonus_exact(lhs);
    let rhs_bonus = pool.event_bonus_exact(rhs);
    if lhs_bonus.base_x10() < rhs_bonus.base_x10()
        || lhs_bonus.limited_x10() < rhs_bonus.limited_x10()
    {
        return false;
    }
    if ctx.is_final_chapter
        && (ctx.leader_honor_bonus_at(lhs.raw()) < ctx.leader_honor_bonus_at(rhs.raw())
            || ctx.leader_limit_bonus_at(lhs.raw()) < ctx.leader_limit_bonus_at(rhs.raw()))
    {
        return false;
    }
    if pool.attr(lhs) != pool.attr(rhs) {
        return false;
    }

    // Identical membership preserves the completion's power contexts and every
    // other card's unit-count/different-unit skill state.
    pool.unit_mask_raw(lhs) == pool.unit_mask_raw(rhs)
}

fn skill_dominates(pool: &CardPool, lhs: CardIdx, rhs: CardIdx) -> bool {
    let lhs_skill = pool.skill(lhs);
    let rhs_skill = pool.skill(rhs);
    if lhs_skill.skill_type != rhs_skill.skill_type
        || pool.skill_min(lhs) < pool.skill_min(rhs)
        || pool.skill_max(lhs) < pool.skill_max(rhs)
    {
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
