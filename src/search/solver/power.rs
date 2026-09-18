//! Additive top-K dynamic programming over unit/attribute power scenarios.
use crate::pool::{CardIdx, CardPool};
use crate::search::{
    DeckResult, SearchContext, SearchParams, SearchStats, TopKTracker, evaluate, placement,
};
use crate::types::DECK_SIZE;

#[derive(Clone, Copy)]
struct PowerPartial {
    cards: [CardIdx; DECK_SIZE],
    len: usize,
    additive_power: u32,
}

pub(super) fn search_power_scenarios(
    pool: &CardPool,
    ctx: &SearchContext,
    params: &SearchParams,
) -> (Vec<DeckResult>, SearchStats) {
    // For an additive scenario, keeping the best K partial states at each
    // cardinality is exact: every future choice is independent of the cards
    // already processed. A discarded partial state can therefore never re-enter
    // the final top K.
    let state_limit = params.top_k.max(1);
    let mut tracker = TopKTracker::new(params.top_k);
    let mut stats = SearchStats::default();

    let mut scenarios = Vec::with_capacity(49);
    scenarios.push((None, None));
    for attr in 0u8..6 {
        scenarios.push((None, Some(attr)));
    }
    for unit in 0usize..6 {
        scenarios.push((Some(unit), None));
        for attr in 0u8..6 {
            scenarios.push((Some(unit), Some(attr)));
        }
    }

    for (unit_all, attr_all) in scenarios {
        let mut by_character = vec![Vec::<(u32, CardIdx)>::new(); 27];
        for card in pool.indices() {
            if unit_all.is_some_and(|unit| pool.unit_mask_raw(card) & (1u8 << unit) == 0) {
                continue;
            }
            if attr_all.is_some_and(|attr| pool.attr(card) != attr) {
                continue;
            }
            let character = usize::from(pool.char_id(card)).min(26);
            let power =
                evaluate::resolve_card_power_scenario(pool, card, unit_all, attr_all.is_some());
            by_character[character].push((power, card));
        }
        for cards in &mut by_character {
            cards.sort_unstable_by(|left, right| {
                right
                    .0
                    .cmp(&left.0)
                    .then_with(|| pool.game_id(left.1).cmp(&pool.game_id(right.1)))
                    .then_with(|| left.1.raw().cmp(&right.1.raw()))
            });
            // 同一 game_id 的养成变体互斥（同一张卡），只保留场景值最高的一个：
            // 变体占多个名额会在每角色候选与 DP 状态里挤出真正不同的次优集合，
            // 令 Top-K 丢解（issue #24 的 mass_099712 案例）。
            let mut seen_game_ids = Vec::with_capacity(cards.len());
            cards.retain(|(_, card)| {
                let game_id = pool.game_id(*card);
                if seen_game_ids.contains(&game_id) {
                    false
                } else {
                    seen_game_ids.push(game_id);
                    true
                }
            });
            cards.truncate(state_limit);
        }

        let seed = PowerPartial {
            cards: [CardIdx::new(0); DECK_SIZE],
            len: 0,
            additive_power: 0,
        };
        let mut states = vec![Vec::<PowerPartial>::new(); DECK_SIZE + 1];
        states[0].push(seed);
        for choices in by_character {
            if choices.is_empty() {
                continue;
            }
            let mut count = DECK_SIZE;
            while count > 0 {
                count -= 1;
                if states[count].is_empty() {
                    continue;
                }
                let previous = states[count].clone();
                for state in previous {
                    for &(power, card) in &choices {
                        let mut next = state;
                        next.cards[count] = card;
                        next.len = count + 1;
                        next.additive_power = next.additive_power.saturating_add(power);
                        states[count + 1].push(next);
                    }
                }
                states[count + 1].sort_unstable_by(|left, right| {
                    right
                        .additive_power
                        .cmp(&left.additive_power)
                        .then_with(|| {
                            partial_public_key(pool, left).cmp(&partial_public_key(pool, right))
                        })
                        .then_with(|| left.cards.cmp(&right.cards))
                });
                states[count + 1].truncate(state_limit);
            }
        }

        for state in &states[DECK_SIZE] {
            stats.leaf_nodes += 1;
            if let Some(candidate) = placement::evaluate_candidate(pool, ctx, &state.cards) {
                tracker.insert(pool, ctx, candidate);
            }
        }
    }
    (tracker.into_vec(), stats)
}

fn partial_public_key(pool: &CardPool, state: &PowerPartial) -> [u16; DECK_SIZE] {
    let mut ids = [u16::MAX; DECK_SIZE];
    for (id, &card) in ids.iter_mut().zip(&state.cards[..state.len]) {
        *id = pool.game_id(card);
    }
    ids.sort_unstable();
    ids
}
