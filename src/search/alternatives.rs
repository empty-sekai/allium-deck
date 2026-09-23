//! Exact reconstruction of card sets removed by certified dominance.
use super::{DeckResult, SearchContext, SearchParams, placement, tracker::TopKTracker};
use super::{SearchStats, budget::SearchBudget};
use crate::pool::{CardIdx, CardPool};
use crate::types::DECK_SIZE;

/// Top-K 支配替代展开。
///
/// dominance 裁剪对 Top-1 无损（被裁卡换成支配者分数不降），但 Top-K 下被裁卡参与的
/// 组合本身可能是合法的次优解（issue #2）。设真实 Top-K 中存在含被裁卡的卡组 D，把
/// 其中每张被裁卡换成其支配根得到 D'，则 score(D') >= score(D) >= 第 K 名阈值，故 D'
/// 的公共卡集合必在裁剪池的精确 Top-K 结果里。同一集合只保留一个最优培养态组合，
/// 故回换前先枚举这个集合的全部原培养态组合，再各自按槽位做替代回换（含多槽组合）。
/// 每个完整培养态组合独立求值；不同组合之间不能沿用实际分数作单调剪枝上界。
///
/// 回换方向是支配的逆向，分数单调不升，按当前第 K 名阈值剪枝；`top_k <= 1` 直接跳过，
/// 主搜索路径零开销。
pub(super) fn expand_dominated_alternatives(
    pool: &CardPool,
    ctx: &SearchContext,
    alternatives: &[Vec<CardIdx>],
    params: &SearchParams,
    results: Vec<DeckResult>,
    budget: &mut SearchBudget,
    stats: &mut SearchStats,
) -> Vec<DeckResult> {
    expand_alternatives(pool, ctx, alternatives, &[], params, results, budget, stats)
}

/// `member_alternatives` 仅在 member 槽位（slot >= 1）参与回换：终章 member 裁剪
/// 忽略队长专属加成，被裁卡作队长仍可能更优，不能回换进队长槽。
///
/// 终章额外从每个结果的队长轮换出发展开：Top-K tracker 按卡集合去重、只保留最优
/// 排列，若某替代根恰是自身集合的最佳队长，它在结果里只出现在队长槽，直接回换
/// 永远不触发；轮换把根移回 member 槽后再回换，并顺带修正集合在其它队长下的
/// 最优排列分数。轮换按固定槽约束过滤，逐一精确评估后并入 tracker。
pub(super) fn expand_alternatives(
    pool: &CardPool,
    ctx: &SearchContext,
    alternatives: &[Vec<CardIdx>],
    member_alternatives: &[Vec<CardIdx>],
    params: &SearchParams,
    results: Vec<DeckResult>,
    budget: &mut SearchBudget,
    stats: &mut SearchStats,
) -> Vec<DeckResult> {
    if params.top_k <= 1 {
        return results;
    }
    let rotate_leader = ctx.is_final_chapter;
    let variants = cultivation_variants(pool);
    let has_alternatives = results.iter().any(|result| {
        result.cards.iter().enumerate().any(|(slot, card)| {
            let options = variants
                .as_ref()
                .map(|variants| variants[card.raw()].as_slice())
                .filter(|options| !options.is_empty())
                .unwrap_or(core::slice::from_ref(card));
            options.iter().any(|variant| {
                !alternatives[variant.raw()].is_empty()
                    || ((slot > 0 || rotate_leader)
                        && member_alternatives
                            .get(variant.raw())
                            .is_some_and(|alts| !alts.is_empty()))
            })
        })
    });
    if !has_alternatives && !rotate_leader {
        return results;
    }

    let mut tracker = TopKTracker::new(params.top_k);
    for result in &results {
        tracker.insert(pool, ctx, *result);
    }
    for result in &results {
        if budget.expired() {
            break;
        }
        if let Some(variants) = &variants
            && result
                .cards
                .iter()
                .any(|card| !variants[card.raw()].is_empty())
        {
            let mut deck = result.cards;
            expand_cultivation_variants(
                pool,
                ctx,
                alternatives,
                member_alternatives,
                variants,
                &mut deck,
                0,
                &mut tracker,
                budget,
                stats,
            );
        } else {
            expand_root_placements(
                pool,
                ctx,
                alternatives,
                member_alternatives,
                *result,
                &mut tracker,
                budget,
                stats,
            );
        }
    }
    tracker.into_vec()
}

/// Empty entries mean one state. Avoid variant traversal in the common case.
fn cultivation_variants(pool: &CardPool) -> Option<Vec<Vec<CardIdx>>> {
    let mut by_game: Vec<_> = pool.indices().collect();
    by_game.sort_unstable_by_key(|&card| (pool.game_id(card), card.raw()));
    if !by_game
        .windows(2)
        .any(|pair| pool.game_id(pair[0]) == pool.game_id(pair[1]))
    {
        return None;
    }
    let mut variants = vec![Vec::new(); pool.count()];
    let mut start = 0;
    while start < by_game.len() {
        let mut end = start + 1;
        while end < by_game.len() && pool.game_id(by_game[end]) == pool.game_id(by_game[start]) {
            end += 1;
        }
        if end - start > 1 {
            for &card in &by_game[start..end] {
                variants[card.raw()].extend_from_slice(&by_game[start..end]);
            }
        }
        start = end;
    }
    Some(variants)
}

/// Enumerate before score pruning: another member's best cultivation state can
/// change after an inverse substitution, even if its own public ID is retained.
#[allow(clippy::too_many_arguments)]
fn expand_cultivation_variants(
    pool: &CardPool,
    ctx: &SearchContext,
    alternatives: &[Vec<CardIdx>],
    member_alternatives: &[Vec<CardIdx>],
    variants: &[Vec<CardIdx>],
    deck: &mut [CardIdx; DECK_SIZE],
    slot: usize,
    tracker: &mut TopKTracker,
    budget: &mut SearchBudget,
    stats: &mut SearchStats,
) {
    if budget.expired_sampled() {
        return;
    }
    stats.diagnostics.alternative_states += 1;
    if slot == DECK_SIZE {
        stats.leaf_nodes += 1;
        stats.diagnostics.alternative_leaves += 1;
        if let Some(candidate) = placement::evaluate_candidate(pool, ctx, deck) {
            tracker.insert(pool, ctx, candidate);
            expand_root_placements(
                pool,
                ctx,
                alternatives,
                member_alternatives,
                candidate,
                tracker,
                budget,
                stats,
            );
        }
        return;
    }
    let original = deck[slot];
    let options = &variants[original.raw()];
    if options.is_empty() {
        expand_cultivation_variants(
            pool,
            ctx,
            alternatives,
            member_alternatives,
            variants,
            deck,
            slot + 1,
            tracker,
            budget,
            stats,
        );
    } else {
        for &variant in options {
            if budget.expired_sampled() {
                break;
            }
            deck[slot] = variant;
            expand_cultivation_variants(
                pool,
                ctx,
                alternatives,
                member_alternatives,
                variants,
                deck,
                slot + 1,
                tracker,
                budget,
                stats,
            );
        }
        deck[slot] = original;
    }
}

#[allow(clippy::too_many_arguments)]
fn expand_root_placements(
    pool: &CardPool,
    ctx: &SearchContext,
    alternatives: &[Vec<CardIdx>],
    member_alternatives: &[Vec<CardIdx>],
    result: DeckResult,
    tracker: &mut TopKTracker,
    budget: &mut SearchBudget,
    stats: &mut SearchStats,
) {
    let mut deck = result.cards;
    expand_substitutions(
        pool,
        ctx,
        alternatives,
        member_alternatives,
        &mut deck,
        result.score,
        0,
        tracker,
        budget,
        stats,
    );
    if !ctx.is_final_chapter {
        return;
    }
    for slot in 1..DECK_SIZE {
        if budget.expired_sampled() {
            break;
        }
        let mut rotated = result.cards;
        rotated.swap(0, slot);
        if !deck_matches_fixed_slots(pool, ctx, &rotated) {
            continue;
        }
        stats.leaf_nodes += 1;
        stats.diagnostics.alternative_leaves += 1;
        let Some(candidate) = placement::evaluate_candidate(pool, ctx, &rotated) else {
            continue;
        };
        let score = candidate.score;
        tracker.insert(pool, ctx, candidate);
        expand_substitutions(
            pool,
            ctx,
            alternatives,
            member_alternatives,
            &mut rotated,
            score,
            0,
            tracker,
            budget,
            stats,
        );
    }
}

/// 判断卡组每个槽位是否满足固定卡/固定角色约束（队长轮换用）。
pub(super) fn deck_matches_fixed_slots(
    pool: &CardPool,
    ctx: &SearchContext,
    deck: &[CardIdx; DECK_SIZE],
) -> bool {
    ctx.deck_matches_slots(pool, deck)
}

/// 自 `from_slot` 起逐槽尝试把支配者回换成其支配的卡（多槽组合经递归覆盖）。
/// `node_score` 是当前替换组合的分数；再多换任何一张分数不会更高，因此 tracker
/// 满且 node_score 严格低于阈值时整棵子树可剪（同分仍展开，保住 tie-break 名次）。
/// 两轮支配都含支援惩罚维度（issue #23/#7），该单调性在 WL 下同样成立。
#[allow(clippy::too_many_arguments)]
fn expand_substitutions(
    pool: &CardPool,
    ctx: &SearchContext,
    alternatives: &[Vec<CardIdx>],
    member_alternatives: &[Vec<CardIdx>],
    deck: &mut [CardIdx; DECK_SIZE],
    node_score: u64,
    from_slot: usize,
    tracker: &mut TopKTracker,
    budget: &mut SearchBudget,
    stats: &mut SearchStats,
) {
    if budget.expired_sampled() {
        return;
    }
    stats.diagnostics.alternative_states += 1;
    let threshold = tracker.threshold();
    if threshold != 0 && node_score < threshold {
        stats.ub_prunes += 1;
        return;
    }
    let mut slot = from_slot;
    while slot < DECK_SIZE {
        // 固定卡槽位按 game_id 锁死，被支配的替代卡 game_id 必不同（固定卡不参与裁剪），跳过。
        if ctx.fixed_card_at(slot).is_some() {
            slot += 1;
            continue;
        }
        let original = deck[slot];
        let member_alts: &[CardIdx] = if slot > 0 {
            member_alternatives
                .get(original.raw())
                .map(Vec::as_slice)
                .unwrap_or(&[])
        } else {
            &[]
        };
        for &alt in alternatives[original.raw()].iter().chain(member_alts) {
            if budget.expired_sampled() {
                break;
            }
            deck[slot] = alt;
            // 支配卡与被支配卡同角色，角色唯一性与固定角色槽位约束自然保持。
            stats.leaf_nodes += 1;
            stats.diagnostics.alternative_leaves += 1;
            let Some(candidate) = placement::evaluate_candidate(pool, ctx, deck) else {
                continue;
            };
            let score = candidate.score;
            tracker.insert(pool, ctx, candidate);
            expand_substitutions(
                pool,
                ctx,
                alternatives,
                member_alternatives,
                deck,
                score,
                slot + 1,
                tracker,
                budget,
                stats,
            );
        }
        deck[slot] = original;
        slot += 1;
    }
}
