use crate::pool::{CardIdx, CardPool};

use super::context::SearchContext;
use super::evaluate::decode_u18;

/// dominance 裁剪后的卡池、上下文和原索引映射。
pub struct DominanceResult {
    pub pool: CardPool,
    pub ctx: SearchContext,
    pub original_indices: Vec<CardIdx>,
    /// 原 dense 索引 -> 被该卡（直接或经支配链传递）支配而裁掉的原索引列表。
    /// 仅存活卡的条目非空，供 Top-K 搜索后的替代展开使用。
    pub alternatives: Vec<Vec<CardIdx>>,
    pub before: usize,
    pub after: usize,
}

/// 执行逐角色支配裁剪并返回压缩后的卡池。
///
/// WL 同样走支配裁剪：被裁的卡仍在独立的 support_cards 里参与支援计算（支援与主搜索池解耦），
/// 且 `dominates` 要求 attr 相同，异色变体全部保留，diff_attr_bonus 无损。
pub fn eliminate_dominated(pool: &CardPool, ctx: &SearchContext) -> DominanceResult {
    // WL 下支配还需支援惩罚不劣（issue #23）：支援表内的卡编入队伍会损失支援加成，
    // 支援盲的支配会裁掉真实最优卡组里的卡，Top-1 都可能出错。
    let penalties = support_penalties(pool, ctx);
    let (keep, dominated_by) = compute_keep_mask_with_winners(pool, ctx, penalties.as_deref());
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
    pub keep: Vec<bool>,
    /// member 位存活根 -> 被其（直接或经链传递）member 位支配裁掉的索引列表。
    pub alternatives: Vec<Vec<CardIdx>>,
}

/// WL 支配裁剪的支援惩罚维度：支援表内的卡编入队伍会损失其支援加成
/// （评估把在队卡从支援总和中排除），惩罚更大的卡作支配者时裁剪不再分数安全、
/// 「被裁卡换成支配根」也不再分数单调。逐队长角色取惩罚（终章支援表按队长角色
/// 独立；非终章回落到全局支援表，各角色列相同），支配要求对每个队长角色都不劣。
/// 第一轮 `eliminate_dominated`（issue #23）与终章 member 轮（issue #7）共用。
fn support_penalties(pool: &CardPool, ctx: &SearchContext) -> Option<Vec<[i32; 27]>> {
    if !ctx.is_world_bloom {
        return None;
    }
    let mut dense_by_game_id = std::collections::HashMap::new();
    for card in pool.indices() {
        dense_by_game_id.insert(pool.game_id(card), card.raw());
    }
    let mut penalties = vec![[0i32; 27]; pool.count()];
    let mut any = false;
    let mut char_id = 0u8;
    while (char_id as usize) < 27 {
        let support = ctx.support_deck_for_leader(char_id);
        let count = support.count as usize;
        if count > 0 {
            let replacement = support
                .cards
                .get(count)
                .map(|(_, bonus)| *bonus)
                .unwrap_or(0.0);
            for &(game_id, bonus) in support.cards.iter().take(count) {
                let penalty = ((bonus - replacement).max(0.0) * 100.0).round() as i32;
                if penalty == 0 {
                    continue;
                }
                if let Some(&dense) = dense_by_game_id.get(&game_id) {
                    penalties[dense][char_id as usize] = penalty;
                    any = true;
                }
            }
        }
        char_id += 1;
    }
    any.then_some(penalties)
}

/// 终章 member 位支配裁剪：用中性 ctx（忽略队长专属称号/当期加成）逐角色比较，
/// 裁掉仅剩队长价值的卡的 member 用途。固定卡从真实 ctx 继承、永不被裁；
/// WL 支援惩罚从真实 ctx 计入支配维度。被裁卡记录到存活根的 alternatives，
/// 供 Top-K 搜索后按 member 槽位回换（issue #7）。
pub fn compute_member_dominance(pool: &CardPool, ctx: &SearchContext) -> MemberDominance {
    let penalties = support_penalties(pool, ctx);
    let (keep, dominated_by) = compute_keep_mask_with_winners(
        pool,
        &SearchContext {
            target: crate::types::ScoreTarget::Power,
            fixed_card_ids: ctx.fixed_card_ids.clone(),
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
            support_deck: super::context::SupportDeck::default(),
            support_decks_by_character: Vec::new(),
            is_world_bloom: false,
            is_final_chapter: false,
            enforce_char_uniqueness: true,
            minimize: false,
            live_type: crate::types::LiveType::Solo,
            event_type: None,
            keep_after_training_state: false,
            skill_reference_strategy: crate::types::SkillReferenceStrategy::Average,
            best_skill_as_leader: false,
            live_skill_order: crate::types::LiveSkillOrder::Average,
            specific_skill_order: None,
            multi_teammate_score_up: None,
            multi_teammate_power: None,
            multi_live_score_up_lower_bound: None,
            extra_bonus_ub: 0,
            w_power: 1.0,
            w_bonus: 1.0,
            skill_ub_global: 0,
            card_bonus_count_limit: crate::types::DECK_SIZE,
            honor_bonus: 0,
            power_total_cap: None,
            leader_honor_bonus: vec![0; pool.count()],
            leader_limit_bonus: vec![0; pool.count()],
            final_chapter_member_keep: vec![true; pool.count()],
            skill_is_after_training: vec![false; pool.count()],
            trained_to_special_image: vec![false; pool.count()],
        },
        penalties.as_deref(),
    );
    let alternatives = chain_compress_alternatives(&keep, &dominated_by);
    MemberDominance { keep, alternatives }
}

/// 返回保留位图与「被谁裁掉」映射：dominated_by[dense] 仅在 keep[dense]=false 时有意义，
/// 记录裁掉该卡的卡的 dense 索引（裁剪者之后仍可能被裁，使用前需链压缩到存活根）。
/// `support_penalties` 存在时，支配额外要求逐队长角色的支援惩罚不劣。
fn compute_keep_mask_with_winners(
    pool: &CardPool,
    ctx: &SearchContext,
    support_penalties: Option<&[[i32; 27]]>,
) -> (Vec<bool>, Vec<u16>) {
    let mut keep = vec![true; pool.count()];
    let mut dominated_by = vec![0u16; pool.count()];
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
                        && support_penalty_not_worse(support_penalties, a, b)
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

#[inline(always)]
fn support_penalty_not_worse(
    support_penalties: Option<&[[i32; 27]]>,
    lhs: CardIdx,
    rhs: CardIdx,
) -> bool {
    let Some(penalties) = support_penalties else {
        return true;
    };
    let lhs_pen = &penalties[lhs.raw()];
    let rhs_pen = &penalties[rhs.raw()];
    let mut char_id = 0usize;
    while char_id < 27 {
        if lhs_pen[char_id] > rhs_pen[char_id] {
            return false;
        }
        char_id += 1;
    }
    true
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
