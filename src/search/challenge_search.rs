use crate::pool::{CardIdx, CardPool};
use crate::types::DECK_SIZE;

use super::context::SearchContext;
use super::dfs::TopKTracker;
use super::evaluate::leaf_evaluate_checked;
use super::suffix::SuffixBound;
use super::types::{DeckResult, SearchParams};

/// challenge 模式专用搜索。
///
/// challenge 模式下 pool 全部为同角色卡，不要求角色唯一性，
/// 但仍需保证同一 game_id 不重复出现（单卡多个技能变体只取其一）。
pub fn search(
    pool: &CardPool,
    ctx: &SearchContext,
    suffix: &SuffixBound,
    params: &SearchParams,
) -> (Vec<DeckResult>, super::SearchStats) {
    if params.top_k == 0 || pool.count() < DECK_SIZE {
        return (Vec::new(), super::SearchStats::default());
    }

    let mut tracker = TopKTracker::new(params.top_k, pool);
    let mut deck = [CardIdx::new(0); DECK_SIZE];
    let mut stats = super::SearchStats::default();

    challenge_recurse(pool, ctx, suffix, 0, 0, &mut deck, &mut tracker, &mut stats);

    (tracker.into_vec(), stats)
}

fn challenge_recurse(
    pool: &CardPool,
    ctx: &SearchContext,
    suffix: &SuffixBound,
    depth: usize,
    start: usize,
    deck: &mut [CardIdx; DECK_SIZE],
    tracker: &mut TopKTracker,
    stats: &mut super::SearchStats,
) {
    if depth == DECK_SIZE {
        stats.leaf_nodes += 1;
        if let Some(score) = leaf_evaluate_checked(pool, ctx, deck) {
            if score > tracker.threshold() {
                tracker.insert(DeckResult::new(*deck, score));
            }
        }
        return;
    }

    let remaining = DECK_SIZE - depth;
    let mut dense = start;
    while dense < pool.count() {
        let card = CardIdx::new(dense as u16);
        dense += 1;

        // game_id 去重：同卡多技能变体只取其一
        if game_id_in_deck(pool, deck, depth, card) {
            continue;
        }

        // 剩余卡不够填满槽位时提前退出
        if pool.count() - dense < remaining - 1 {
            break;
        }

        deck[depth] = card;
        challenge_recurse(pool, ctx, suffix, depth + 1, dense, deck, tracker, stats);
    }
}

#[inline(always)]
fn game_id_in_deck(
    pool: &CardPool,
    deck: &[CardIdx; DECK_SIZE],
    depth: usize,
    card: CardIdx,
) -> bool {
    let gid = pool.game_id(card);
    let mut i = 0;
    while i < depth {
        if pool.game_id(deck[i]) == gid {
            return true;
        }
        i += 1;
    }
    false
}
