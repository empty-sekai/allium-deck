use super::*;
use crate::power::{DeckComposition, MultiUnitBonusMode};

fn pool_with_multi(mode: MultiUnitBonusMode) -> CardPool {
    pool_with_multi_shape(mode, 0)
}

fn pool_with_multi_shape(mode: MultiUnitBonusMode, shape: u8) -> CardPool {
    let masks = match shape {
        1 => [1, 1, 1, 1, 33, 33, 33, 2, 33],
        2 => [33; 9],
        3 => [32, 33, 34, 36, 40, 48, 33, 34, 32],
        _ => [1, 1, 2, 4, 8, 16, 32, 33, 33],
    };
    let chars = match shape {
        1 => [1, 2, 3, 4, 21, 22, 23, 9, 24],
        2 => [21, 22, 23, 24, 25, 26, 21, 22, 23],
        3 => [21; 9],
        _ => [1, 2, 5, 9, 13, 17, 21, 22, 22],
    };
    let mut builder = PoolBuilder::new(9);
    builder.set_multi_unit_bonus_mode(mode);
    for dense in 0..9u16 {
        let base = 1_000 + u32::from(dense) * 27;
        let (values, lut) = encode_power(base);
        builder.set_power_values(dense, values);
        builder.set_power_lut(
            dense,
            lut | if masks[usize::from(dense)] == 33 {
                1 << 21
            } else {
                0
            },
        );
        // Non-monotone ALL_MATCH states exercise bounds without assuming a
        // positive all-match increment or that multi always increases power.
        let mut multi = std::array::from_fn(|key| {
            base + ((key * 37 + usize::from(dense) * 113) % 509) as u32 - 350
        });
        if dense == 0 {
            multi[0] = 300_123;
        }
        builder.set_multi_power_values(dense, multi);
        builder.set_power_max(dense, base.max(*multi.iter().max().unwrap()));
        builder.set_char_id(dense, chars[usize::from(dense)]);
        builder.set_attr(dense, if shape == 0 { (dense % 3) as u8 } else { 0 });
        builder.set_unit_mask(dense, masks[usize::from(dense)]);
        builder.set_game_id(dense, 100 + dense);
        builder.set_skill(
            dense,
            SkillSlot {
                skill_type: 0,
                value: 50,
            },
        );
        builder.set_skill_min(dense, 50);
        builder.set_skill_max(dense, 50);
        builder.set_event_bonus(dense, EventBonusExact::from_whole(10, 0));
    }
    builder.freeze()
}

fn pool_with_empty_membership(mode: MultiUnitBonusMode, with_multi: bool) -> CardPool {
    let mut builder = PoolBuilder::new(10);
    builder.set_multi_unit_bonus_mode(mode);
    for dense in 0..10u16 {
        let empty = dense >= 5;
        let power = if empty { 100 } else { 10 };
        let (values, lut) = encode_power(power);
        builder.set_power_values(dense, values);
        builder.set_power_lut(dense, lut);
        if with_multi {
            builder.set_multi_power_values(dense, [power * 2; 8]);
        }
        builder.set_power_max(dense, power * 2);
        builder.set_char_id(dense, (dense % 5 + 1) as u8);
        builder.set_attr(dense, (dense % 3) as u8);
        builder.set_unit_mask(dense, if empty { 0 } else { 1 << (dense % 5) });
        builder.set_game_id(dense, 100 + dense);
    }
    builder.freeze()
}

#[test]
fn empty_membership_power_minimum_bounds_the_effective_power() {
    for mode in [
        MultiUnitBonusMode::ByDeck,
        MultiUnitBonusMode::ForceOn,
        MultiUnitBonusMode::ForceOff,
    ] {
        for with_multi in [false, true] {
            let pool = pool_with_empty_membership(mode, with_multi);
            let cards = std::array::from_fn::<_, 5, _>(|i| pool.card_idx(i as u16 + 5).unwrap());
            let composition = DeckComposition::from_cards(&pool, &cards);
            for card in cards {
                let effective = crate::power::effective_power(&pool, card, composition);
                assert_eq!(pool.power_min(card), 0);
                assert!(pool.power_min(card) <= effective);
            }
        }
    }
}

#[test]
fn empty_membership_minimization_keeps_zero_power_improvements() {
    for mode in [
        MultiUnitBonusMode::ByDeck,
        MultiUnitBonusMode::ForceOn,
        MultiUnitBonusMode::ForceOff,
    ] {
        for with_multi in [false, true] {
            let pool = pool_with_empty_membership(mode, with_multi);
            let mut context = ready_ctx(&pool, ScoreTarget::Power);
            context.minimize = true;
            for top_k in [1, 7] {
                let params = SearchParams {
                    top_k,
                    timeout_ms: 0,
                };
                let (expected, _) = brute_force_search(&pool, &context, &params);
                let actual = search(&pool, &context, &params);
                assert_eq!(actual.completion(), SearchCompletion::Complete);
                assert_results_match_bruteforce(&pool, &actual.results, &expected);
                let minimum = if with_multi && mode == MultiUnitBonusMode::ForceOn {
                    100
                } else {
                    0
                };
                let summary =
                    crate::search::summarize_deck(&pool, &context, &actual.results[0].cards)
                        .unwrap();
                assert_eq!(summary.total_power, minimum);
            }
        }
    }
}

#[test]
fn multi_composition_distinguishes_shared_membership_and_original_shuffle() {
    for (masks, expected_multi, expected_common, expected_shuffle) in [
        ([32; 5], false, 32, 0),
        ([33; 5], true, 33, 0),
        ([1, 1, 1, 1, 33], true, 1, 0),
        ([1, 2, 4, 8, 32], true, 0, 50),
        ([1, 2, 4, 33, 34], true, 0, 30),
    ] {
        let mut builder = PoolBuilder::new(5);
        for (dense, mask) in masks.into_iter().enumerate() {
            builder.set_unit_mask(dense as u16, mask);
        }
        let pool = builder.freeze();
        let c = DeckComposition::from_cards(&pool, &collect_first_five(&pool));
        assert_eq!(
            (c.is_multi_unit, c.shared_units, c.shuffle_bonus()),
            (expected_multi, expected_common, expected_shuffle)
        );
    }
}

#[test]
fn multi_sidecar_survives_compaction_and_resolves_nonmonotone_minimum() {
    let pool = pool_with_multi(MultiUnitBonusMode::ForceOn);
    let keep = [false, true, false, true, true, true, true, false, true];
    let compact = pool.compact(&keep);
    for (new, old) in compact
        .indices()
        .zip(pool.indices().filter(|card| keep[card.raw()]))
    {
        assert_eq!(
            compact.multi_power_values(new),
            pool.multi_power_values(old)
        );
        assert_eq!(compact.power_min(new), pool.power_min(old));
        assert_eq!(compact.game_id(new), pool.game_id(old));
    }
    assert_eq!(compact.multi_unit_bonus_mode(), MultiUnitBonusMode::ForceOn);
}

#[test]
fn multi_exact_topk_all_numeric_and_event_paths() {
    let params = SearchParams {
        top_k: 7,
        timeout_ms: 0,
    };
    for mode in [
        MultiUnitBonusMode::ByDeck,
        MultiUnitBonusMode::ForceOn,
        MultiUnitBonusMode::ForceOff,
    ] {
        let pool = pool_with_multi(mode);
        for target in [
            ScoreTarget::Power,
            ScoreTarget::Skill,
            ScoreTarget::Score,
            ScoreTarget::Bonus,
            ScoreTarget::Mysekai,
        ] {
            for event in [false, true] {
                let mut context = ready_ctx(&pool, target);
                context.is_world_bloom = event;
                context.event_type = event.then_some(EventType::WorldBloom);
                context.diff_attr_bonus = [0, 0, 10, 20, 30, 50];
                context.extra_bonus_ub = if event { 50 } else { 0 };
                let (expected, _) = brute_force_search(&pool, &context, &params);
                let actual = search(&pool, &context, &params);
                assert_eq!(actual.completion(), SearchCompletion::Complete);
                assert_results_match_bruteforce(&pool, &actual.results, &expected);
            }
        }
        let mut context = ready_ctx(&pool, ScoreTarget::Power);
        context.minimize = true;
        let (expected, _) = brute_force_search(&pool, &context, &params);
        let actual = search(&pool, &context, &params);
        assert_eq!(actual.completion(), SearchCompletion::Complete);
        assert_results_match_bruteforce(&pool, &actual.results, &expected);
    }
}

#[test]
fn multi_and_shared_membership_topk_cover_both_original_and_support_bits() {
    let params = SearchParams {
        top_k: 7,
        timeout_ms: 0,
    };
    for shape in [1, 2] {
        for mode in [
            MultiUnitBonusMode::ByDeck,
            MultiUnitBonusMode::ForceOn,
            MultiUnitBonusMode::ForceOff,
        ] {
            let pool = pool_with_multi_shape(mode, shape);
            for target in [ScoreTarget::Power, ScoreTarget::Score, ScoreTarget::Mysekai] {
                let context = ready_ctx(&pool, target);
                let (expected, _) = brute_force_search(&pool, &context, &params);
                let actual = search(&pool, &context, &params);
                assert_eq!(actual.completion(), SearchCompletion::Complete);
                assert_results_match_bruteforce(&pool, &actual.results, &expected);
            }
        }
    }
}

#[test]
fn multi_challenge_topk_keeps_all_same_character_cards() {
    let params = SearchParams {
        top_k: 7,
        timeout_ms: 0,
    };
    for mode in [
        MultiUnitBonusMode::ByDeck,
        MultiUnitBonusMode::ForceOn,
        MultiUnitBonusMode::ForceOff,
    ] {
        let pool = pool_with_multi_shape(mode, 3);
        for target in [ScoreTarget::Power, ScoreTarget::Score, ScoreTarget::Skill] {
            let mut context = ready_ctx(&pool, target);
            context.enforce_char_uniqueness = false;
            context.live_type = LiveType::Challenge;
            let expected = exhaustive_challenge_results(&pool, &context);
            let actual = search(&pool, &context, &params);
            assert_eq!(actual.completion(), SearchCompletion::Complete);
            assert_results_match_bruteforce(&pool, &actual.results, &expected[..params.top_k]);
        }
    }
}

#[test]
fn multi_wl3_final_and_exact_bonus_tiers_keep_shuffle_reachable() {
    let pool = pool_with_multi(MultiUnitBonusMode::ByDeck);
    let params = SearchParams {
        top_k: 5,
        timeout_ms: 0,
    };
    let mut context = ready_ctx(&pool, ScoreTarget::Score);
    context.is_world_bloom = true;
    context.is_final_chapter = true;
    context.is_wl3_finale = true;
    context.event_type = Some(EventType::WorldBloom);
    context.live_type = LiveType::Multi;
    context.live_skill_order = LiveSkillOrder::Average;
    context.best_skill_as_leader = false;
    context.power_total_cap = Some(336_000);
    context.extra_bonus_ub = 50;
    let expected = final_chapter_auto_oracle(&pool, &context, params.top_k);
    let actual = search(&pool, &context, &params);
    assert_eq!(actual.completion(), SearchCompletion::Complete);
    assert_results_match_bruteforce(&pool, &actual.results, &expected);
    // Five 10% cards plus a 0/10/30/50 original-unit bonus.
    let tiers = [50, 60, 80, 100];
    context.target = ScoreTarget::Bonus;
    let outcome = search_targets(&pool, &context, &params, &tiers);
    assert_eq!(outcome.completion(), SearchCompletion::Complete);
    let all = final_chapter_auto_oracle(&pool, &context, 512);
    for tier in tiers {
        let matches = |deck: &&DeckResult| {
            (evaluate::resolve_total_bonus(&pool, &context, &deck.cards) - f64::from(tier)).abs()
                < 1e-9
        };
        let expected = all
            .iter()
            .filter(matches)
            .copied()
            .take(params.top_k)
            .collect::<Vec<_>>();
        let actual = outcome
            .results
            .iter()
            .filter(matches)
            .copied()
            .collect::<Vec<_>>();
        assert_results_match_bruteforce(&pool, &actual, &expected);
    }
}
