//! Cross-family contracts for observable roles and fixed-slot feasibility.
use super::*;

#[test]
fn challenge_specific_order_matches_full_ordered_oracle() {
    for top_k in [1, 4] {
        for fixed in [false, true] {
            let pool = build_pool(&randomized_exact_cards(0x0A55_1611, 8, 1));
            let mut ctx = ready_ctx(&pool, ScoreTarget::Score);
            ctx.live_type = LiveType::Challenge;
            ctx.enforce_char_uniqueness = false;
            ctx.live_skill_order = LiveSkillOrder::Specific;
            ctx.specific_skill_order = Some([4, 2, 0, 3, 1]);
            ctx.skill_scores[0] = [0.21, 0.17, 0.13, 0.11, 0.07, 0.23];
            if fixed {
                ctx.fixed_card_ids = vec![pool.game_id(CardIdx::new(3))];
            }
            let params = SearchParams {
                top_k,
                timeout_ms: 0,
            };
            let (expected, _) = ExactOracle::new(&pool, &ctx).search(&params);
            let (actual, stats) = search_instrumented(&pool, &ctx, &params);
            assert!(!stats.deadline_hit);
            assert_property_results(&pool, &actual, &expected, "Challenge Specific ordered role");
        }
    }
}

#[test]
fn numeric_skill_search_selects_the_resolved_not_metadata_leader() {
    let mut weak = skill_card(700, 1, 1000, 100);
    weak.skill = SkillSlot {
        skill_type: 1,
        value: 1,
    };
    weak.unit_mask = 1;
    let cards = [
        weak,
        skill_card(701, 2, 1000, 60),
        skill_card(702, 3, 1000, 40),
        skill_card(703, 4, 1000, 40),
        skill_card(704, 5, 1000, 40),
    ];
    let pool = build_pool(&cards);
    let mut ctx = ready_ctx(&pool, ScoreTarget::Skill);
    ctx.best_skill_as_leader = false;
    let params = SearchParams {
        top_k: 1,
        timeout_ms: 0,
    };
    let (expected, _) = ExactOracle::new(&pool, &ctx).search(&params);
    let (actual, _) = search_instrumented(&pool, &ctx, &params);
    assert_property_results(&pool, &actual, &expected, "Skill selectable leader");
}

#[test]
fn final_chapter_multiple_fixed_slots_remain_required() {
    let mut cards = randomized_exact_cards(0x0F1A_1122, 12, 6);
    cards[1].power = 10;
    cards[1].power_max = 10;
    let pool = build_pool(&cards);
    for fixed_characters in [false, true] {
        let mut ctx = final_chapter_ctx(&pool);
        ctx.fixed_card_ids = vec![pool.game_id(CardIdx::new(0))];
        if fixed_characters {
            ctx.fixed_character_ids = vec![2, 3];
        } else {
            ctx.fixed_card_ids.push(pool.game_id(CardIdx::new(1)));
        }
        let params = SearchParams {
            top_k: 4,
            timeout_ms: 0,
        };
        let (expected, _) = ExactOracle::new(&pool, &ctx).search(&params);
        for (label, tuning) in [
            ("default", tuning::SearchTuning::default()),
            (
                "no bounds",
                tuning::SearchTuning {
                    bounds: false,
                    ..Default::default()
                },
            ),
            (
                "no dominance",
                tuning::SearchTuning {
                    dominance: false,
                    ..Default::default()
                },
            ),
            (
                "neither",
                tuning::SearchTuning {
                    bounds: false,
                    dominance: false,
                    ..Default::default()
                },
            ),
        ] {
            let (rows, stats) =
                tuning::with_tuning(tuning, || search_instrumented(&pool, &ctx, &params));
            assert!(!stats.deadline_hit);
            assert_property_results(&pool, &rows, &expected, label);
            eprintln!(
                "FINAL_FIXED_ABLATION fixed_chars={fixed_characters} {label} score={} stats={stats:?}",
                rows[0].score
            );
        }
        let (actual, stats) = search_instrumented(&pool, &ctx, &params);
        assert!(!stats.deadline_hit);
        for row in &actual {
            assert!(
                deck_matches_fixed_slots(&pool, &ctx, &row.cards),
                "Final returned an illegal fixed-slot assignment: {row:?}"
            );
        }
        assert_property_results(
            &pool,
            &actual,
            &expected,
            "Final multiple fixed constraints",
        );
    }
}
