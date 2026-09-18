//! Exact tenth-percent bonus feasibility must not use rounded-up partial sums.
use super::*;

#[test]
fn fractional_bonus_tier_keeps_a_reachable_five_percent_deck() {
    let mut builder = PoolBuilder::new(5);
    for (dense, bonus) in [11u16, 11, 11, 11, 6].into_iter().enumerate() {
        let dense = dense as u16;
        let (values, lut) = encode_power(1000);
        builder.set_power_values(dense, values);
        builder.set_power_lut(dense, lut);
        builder.set_power_max(dense, 1000);
        builder.set_game_id(dense, 800 + dense);
        builder.set_char_id(dense, dense as u8 + 1);
        builder.set_attr(dense, 0);
        builder.set_unit_mask(dense, 1);
        builder.set_event_bonus(dense, EventBonusExact::from_x10(bonus, 0));
        builder.mark_char(dense as u8 + 1, dense);
        builder.mark_attr(0, dense);
        builder.mark_unit(0, dense);
    }
    let pool = builder.freeze();
    let mut ctx = ready_ctx(&pool, ScoreTarget::Bonus);
    ctx.event_type = Some(EventType::Marathon);
    let params = SearchParams {
        top_k: 1,
        timeout_ms: 0,
    };
    let (expected, _) = ExactOracle::new(&pool, &ctx).search(&params);
    assert_eq!(expected[0].score >> 32, 10);
    let (actual, stats) = search_bonus_targets(&pool, &ctx, &params, &[5]);
    assert!(!stats.deadline_hit);
    assert_property_results(&pool, &actual, &expected, "fractional bonus reachable tier");
}

fn tier_pool(bonuses: &[(u16, u16)]) -> CardPool {
    let mut builder = PoolBuilder::new(bonuses.len() as u16);
    for (dense, &(base, limited)) in bonuses.iter().enumerate() {
        let dense = dense as u16;
        let (values, lut) = encode_power(1_000);
        builder.set_power_values(dense, values);
        builder.set_power_lut(dense, lut);
        builder.set_power_max(dense, 1_000);
        builder.set_game_id(dense, 900 + dense);
        builder.set_char_id(dense, dense as u8 + 1);
        builder.set_attr(dense, 0);
        builder.set_unit_mask(dense, 1);
        builder.set_event_bonus(dense, EventBonusExact::from_x10(base, limited));
        builder.mark_char(dense as u8 + 1, dense);
        builder.mark_attr(0, dense);
        builder.mark_unit(0, dense);
    }
    builder.freeze()
}

#[test]
fn exact_tier_rejects_rounded_fractional_neighbors() {
    // The public request is exactly 5%, not the nearest half-percent bucket.
    // The general Bonus objective may still quantize its ranking key.
    for total_x10 in 45..=55 {
        let pool = tier_pool(&[(0, 0), (0, 0), (0, 0), (0, 0), (total_x10, 0)]);
        let mut ctx = ready_ctx(&pool, ScoreTarget::Bonus);
        ctx.event_type = Some(EventType::Marathon);
        let params = SearchParams {
            top_k: 1,
            timeout_ms: 0,
        };
        let (actual, stats) = search_bonus_targets(&pool, &ctx, &params, &[5]);
        assert!(!stats.deadline_hit);
        assert_eq!(
            actual.len(),
            usize::from(total_x10 == 50),
            "displayed bonus {} must not be rounded into exact target 5",
            total_x10 as f64 / 10.0
        );
    }
}

#[test]
fn exact_tier_enumerates_limited_bonus_role_assignments() {
    // All five cards are mandatory. Only the first limited card contributes;
    // all five tiers are legal, even though the card set is identical.
    let pool = tier_pool(&[(0, 10), (0, 20), (0, 30), (0, 40), (0, 50)]);
    let mut ctx = ready_ctx(&pool, ScoreTarget::Bonus);
    ctx.event_type = Some(EventType::Marathon);
    ctx.card_bonus_count_limit = 1;
    let params = SearchParams {
        top_k: 1,
        timeout_ms: 0,
    };
    let (actual, stats) = search_bonus_targets(&pool, &ctx, &params, &[1, 2, 3, 4, 5]);
    assert!(!stats.deadline_hit);
    assert_eq!(
        actual.len(),
        5,
        "deduplication and placement optimization must be per tier"
    );
    for (result, target) in actual.iter().zip((1..=5).rev()) {
        let summary = summarize_deck(&pool, &ctx, &result.cards).unwrap();
        assert_eq!(summary.event_bonus_total, Some(target as f64));
        assert_eq!(result.score >> 32, target as u64 * 2);
    }
}

#[test]
fn exact_reachability_does_not_clamp_an_empty_interval_to_a_reachable_sum() {
    let pool = tier_pool(&[(10, 0); 5]);
    let reach = bonus_reach::BonusReach::build(&pool);
    assert!(reach.any_in_range(0, 5, 50, 50));
    assert!(!reach.any_in_range(0, 5, 51, 60));
    assert!(!reach.any_in_range(0, 5, 51, 50));
    assert!(!reach.any_in_range(0, 5, u32::MAX, u32::MAX));
}

#[test]
fn exact_tier_challenge_rejects_mixed_character_decks() {
    let pool = tier_pool(&[(0, 0); 5]);
    let mut ctx = ready_ctx(&pool, ScoreTarget::Bonus);
    ctx.live_type = LiveType::Challenge;
    ctx.enforce_char_uniqueness = false;
    let params = SearchParams {
        top_k: 1,
        timeout_ms: 0,
    };
    assert!(
        search_bonus_targets(&pool, &ctx, &params, &[0])
            .0
            .is_empty()
    );
}

#[test]
fn exact_tier_final_enumerates_nonuniform_limited_members() {
    let pool = tier_pool(&[(0, 10), (0, 20), (0, 30), (0, 40), (0, 50)]);
    let mut ctx = ready_ctx(&pool, ScoreTarget::Bonus);
    ctx.event_type = Some(EventType::WorldBloom);
    ctx.is_world_bloom = true;
    ctx.is_final_chapter = true;
    ctx.card_bonus_count_limit = 2;
    let params = SearchParams {
        top_k: 1,
        timeout_ms: 0,
    };
    let targets = (3..=9).collect::<Vec<_>>();
    let (actual, stats) = search_bonus_targets(&pool, &ctx, &params, &targets);
    assert!(!stats.deadline_hit);
    assert_eq!(actual.len(), targets.len());
    for (result, target) in actual.iter().zip(targets.iter().rev()) {
        assert_eq!(
            summarize_deck(&pool, &ctx, &result.cards)
                .unwrap()
                .event_bonus_total,
            Some(*target as f64)
        );
    }
}

#[test]
fn exact_tier_final_sums_card_tenths_before_display_conversion() {
    // 4.6 + .1 + .1 + .1 + .1 used to become 4.999999999999998.
    let pool = tier_pool(&[(1, 0), (1, 0), (1, 0), (1, 0), (46, 0)]);
    let mut ctx = ready_ctx(&pool, ScoreTarget::Bonus);
    ctx.is_final_chapter = true;
    ctx.is_world_bloom = true;
    ctx.event_type = Some(EventType::WorldBloom);
    ctx.fixed_character_ids = vec![5];
    let params = SearchParams {
        top_k: 1,
        timeout_ms: 0,
    };
    let (actual, _) = search_bonus_targets(&pool, &ctx, &params, &[5]);
    assert_eq!(actual.len(), 1);
    assert_eq!(
        summarize_deck(&pool, &ctx, &actual[0].cards)
            .unwrap()
            .event_bonus_total,
        Some(5.0)
    );
}

#[test]
fn exact_tier_oracle_keeps_each_cultivation_variant_before_per_tier_dedup() {
    let mut cards = randomized_exact_cards(0xB001, 6, 5);
    for card in &mut cards {
        card.base_bonus = 0;
        card.limited_bonus = 0;
    }
    cards[0].base_bonus = 1;
    cards[5].base_bonus = 2;
    cards[5].game_id = cards[0].game_id;
    cards[5].char_id = cards[0].char_id;
    cards[5].attr = cards[0].attr;
    cards[5].unit_mask = cards[0].unit_mask;
    let pool = build_pool(&cards);
    let ctx = ready_ctx(&pool, ScoreTarget::Bonus);
    let params = SearchParams {
        top_k: 1,
        timeout_ms: 0,
    };
    let (expected, _) = ExactOracle::new(&pool, &ctx).search_bonus_targets(&params, &[1, 2]);
    let (actual, _) = search_bonus_targets(&pool, &ctx, &params, &[1, 2]);
    assert_eq!(expected.len(), 2);
    assert_property_results(
        &pool,
        &actual,
        &expected,
        "tier-specific cultivation variants",
    );
    assert_eq!(
        actual[0].game_card_set_key(&pool),
        actual[1].game_card_set_key(&pool)
    );
}

#[test]
fn exact_tier_fractional_limited_and_support_matrix_matches_ordered_oracle() {
    for case in 0..8usize {
        let bonuses = (0..8usize)
            .map(|i| {
                (
                    ((case * 13 + i * 7) % 21) as u16,
                    ((case + i * 3) % 4) as u16 * 10,
                )
            })
            .collect::<Vec<_>>();
        let pool = tier_pool(&bonuses);
        for scene in 0..4 {
            let mut ctx = ready_ctx(&pool, ScoreTarget::Bonus);
            ctx.event_type = Some(EventType::Marathon);
            ctx.card_bonus_count_limit = if scene == 0 { 5 } else { 2 };
            if scene >= 2 {
                ctx.is_world_bloom = true;
                ctx.event_type = Some(EventType::WorldBloom);
                ctx.support_deck = SupportDeck {
                    cards: vec![(901, 1.3), (903, 1.2), (2000, 0.5)],
                    count: 2,
                };
            }
            if scene == 3 {
                ctx.is_final_chapter = true;
                ctx.fixed_character_ids = vec![3];
            }
            let params = SearchParams {
                top_k: 3,
                timeout_ms: 0,
            };
            let targets = (0..=15).collect::<Vec<_>>();
            let (expected, _) =
                ExactOracle::new(&pool, &ctx).search_bonus_targets(&params, &targets);
            let (actual, stats) = search_bonus_targets(&pool, &ctx, &params, &targets);
            assert!(!stats.deadline_hit);
            assert_property_scores(
                &pool,
                &ctx,
                &actual,
                &expected,
                &format!("tier case {case} scene {scene}"),
            );
            assert_property_results(
                &pool,
                &actual,
                &expected,
                &format!("tier case {case} scene {scene}"),
            );
        }
    }
}

#[test]
fn fractional_final_leader_bounds_and_dominance_match_ordered_oracle() {
    for case in 0..16u64 {
        let pool = build_pool(&randomized_exact_cards(0xF10A_0000 + case, 9, 6));
        let mut ctx = final_chapter_ctx(&pool);
        ctx.is_world_bloom = true;
        ctx.event_type = Some(EventType::WorldBloom);
        for card in pool.indices() {
            ctx.leader_honor_bonus_x10[card.raw()] = ((card.raw() * 3 + case as usize) % 13) as u16;
            ctx.leader_limit_bonus_x10[card.raw()] = ((card.raw() * 7 + case as usize) % 17) as u16;
        }
        for fixed in [false, true] {
            ctx.fixed_character_ids = if fixed { vec![2] } else { vec![] };
            for top_k in [1, 3, 8] {
                let params = SearchParams {
                    top_k,
                    timeout_ms: 0,
                };
                let expected = ExactOracle::new(&pool, &ctx).search(&params).0;
                let actual = search(&pool, &ctx, &params);
                assert_property_scores(
                    &pool,
                    &ctx,
                    &actual,
                    &expected,
                    "Final fractional leader bonus",
                );
                assert_property_results(
                    &pool,
                    &actual,
                    &expected,
                    "Final fractional canonical results",
                );
            }
        }
    }
}
