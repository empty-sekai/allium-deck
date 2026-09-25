//! The log-linear regime test against the exact scores of every deck of each
//! regime.
use super::*;
use crate::search::composition::{Regime, RegimePlan};
use crate::search::objective::ObjectiveBound;

const UNIT_BITS: u8 = 0x3f;

/// Whether the shared attribute and units of `deck` put it in `regime`.
fn in_regime(pool: &CardPool, regime: Regime, deck: &[CardIdx; DECK_SIZE]) -> bool {
    let attr = pool.attr(deck[0]);
    let shared_attr = deck.iter().all(|&card| pool.attr(card) == attr);
    let units = deck
        .iter()
        .fold(UNIT_BITS, |units, &card| units & pool.unit_mask_raw(card));
    match regime {
        Regime::Mixed => !shared_attr && units == 0,
        Regime::SharedAttr(a) => shared_attr && attr == a && units == 0,
        Regime::SharedUnit(u) => !shared_attr && units & (1 << u) != 0,
        Regime::SharedUnitAttr(u, a) => shared_attr && attr == a && units & (1 << u) != 0,
    }
}

/// The best exact score of each deck of five distinct characters and public
/// cards, over every card of it as the leader.
fn deck_scores(pool: &CardPool, ctx: &SearchContext) -> Vec<([CardIdx; DECK_SIZE], u64)> {
    let cards: Vec<CardIdx> = pool.indices().collect();
    let count = cards.len();
    let mut decks = Vec::new();
    for a in 0..count {
        for b in a + 1..count {
            for c in b + 1..count {
                for d in c + 1..count {
                    for e in d + 1..count {
                        let deck = [cards[a], cards[b], cards[c], cards[d], cards[e]];
                        let distinct = (0..DECK_SIZE).all(|left| {
                            (left + 1..DECK_SIZE).all(|right| {
                                pool.char_id(deck[left]) != pool.char_id(deck[right])
                                    && pool.game_id(deck[left]) != pool.game_id(deck[right])
                            })
                        });
                        if !distinct {
                            continue;
                        }
                        let best = (0..DECK_SIZE)
                            .filter_map(|lead| {
                                let mut ordered = deck;
                                ordered.swap(0, lead);
                                crate::search::placement::evaluate_candidate(pool, ctx, &ordered)
                                    .map(|result| result.score)
                            })
                            .max();
                        if let Some(best) = best {
                            decks.push((deck, best));
                        }
                    }
                }
            }
        }
    }
    decks
}

fn regime_cards(case: u64) -> Vec<TestCard> {
    let mut rng = ExactLcg(0x7e61_0000 + case);
    let mut cards = randomized_exact_cards(0x7e62_0000 + case, 13, 6);
    for card in &mut cards {
        // Few attributes and overlapping units, so that the shared regimes
        // hold decks.
        card.attr = rng.range(0, 2) as u8;
        let unit = rng.range(0, 3) as u8;
        card.unit_mask = 1 << unit;
        if rng.range(0, 3) == 0 {
            card.unit_mask |= 1 << ((unit + 1) % 3);
        }
    }
    cards
}

fn regime_ctx(pool: &CardPool, case: u64) -> SearchContext {
    let mut rng = ExactLcg(0x7e63_0000 + case);
    let mut ctx = ready_ctx(pool, ScoreTarget::Score);
    ctx.live_type = [LiveType::Solo, LiveType::Auto, LiveType::Multi][(case % 3) as usize];
    ctx.live_skill_order = [LiveSkillOrder::Best, LiveSkillOrder::Average][(case / 3 % 2) as usize];
    ctx.music_rate_pct = 100 + rng.range(0, 60);
    ctx.boost_rate_pct = [100, 500, 1000][rng.range(0, 3) as usize];
    ctx.other_score = [0, 900_000][rng.range(0, 2) as usize];
    ctx.skill_scores = [[0.1, 0.2, 0.3, 0.4, 0.5, 0.6]; 3];
    ctx.multi_teammate_score_up = (rng.range(0, 2) == 0).then(|| rng.range(0, 150) as i32);
    match case / 6 % 3 {
        0 => {
            ctx.event_type = Some(EventType::Marathon);
        }
        1 => {
            ctx.event_type = Some(EventType::WorldBloom);
            ctx.is_world_bloom = true;
            ctx.best_skill_as_leader = false;
            ctx.diff_attr_bonus = [0, 5, 15, 30, 45, 60];
            ctx.support_deck = support_deck_for_property(pool, case as usize);
        }
        _ => {
            ctx.event_type = Some(EventType::WorldBloom);
            ctx.is_world_bloom = true;
            ctx.is_final_chapter = true;
            ctx.best_skill_as_leader = false;
            ctx.diff_attr_bonus = [0, 5, 15, 30, 45, 60];
            ctx.support_decks_by_character = vec![SupportDeck::default(); 27];
            for character in 1usize..=6 {
                ctx.support_decks_by_character[character] =
                    support_deck_for_property(pool, character + case as usize);
            }
            for dense in 0..pool.count() {
                ctx.leader_honor_bonus_x10[dense] = (((dense * 3 + case as usize) % 9) as u16) * 10;
                ctx.leader_limit_bonus_x10[dense] = (((dense * 5 + case as usize) % 7) as u16) * 10;
            }
        }
    }
    if case % 4 == 3 {
        ctx.forced_leader_character_id = Some((case % 6) as u8 + 1);
    }
    ctx
}

#[test]
fn regime_log_linear_test_keeps_every_regime_that_reaches_the_threshold() {
    let mut checked = 0usize;
    let mut excluded = 0usize;
    for case in 0..36u64 {
        let pool = build_pool(&regime_cards(case));
        let ctx = regime_ctx(&pool, case);
        let objective = ObjectiveBound::from_context(&ctx);
        let decks = deck_scores(&pool, &ctx);
        for regime in Regime::all() {
            let Some(plan) = RegimePlan::new(&pool, &ctx, &objective, regime, 0) else {
                continue;
            };
            let best = decks
                .iter()
                .filter(|(deck, _)| {
                    deck.iter().all(|card| plan.keep[card.raw()]) && in_regime(&pool, regime, deck)
                })
                .filter(|(deck, _)| {
                    ctx.forced_leader_character_id.is_none_or(|character| {
                        deck.iter().any(|&card| pool.char_id(card) == character)
                    })
                })
                .map(|&(_, score)| score)
                .max();
            let Some(best) = best else {
                continue;
            };
            assert!(plan.ceiling >= best, "case {case} {regime:?}");
            assert!(
                !plan.log_linear_excludes(&pool, &ctx, &objective, best),
                "case {case} {regime:?}: a deck scores {best}"
            );
            checked += 1;
            // Between the best deck and the regime ceiling the test may
            // rule the regime out.
            let (low, high) = (best >> 32, plan.ceiling >> 32);
            for step in 1..4 {
                let threshold = (low + (high - low) * step / 4) << 32;
                excluded +=
                    usize::from(plan.log_linear_excludes(&pool, &ctx, &objective, threshold));
            }
        }
    }
    assert!(checked >= 100, "{checked} regimes checked");
    assert!(excluded > 0, "the test never excluded a regime");
}
