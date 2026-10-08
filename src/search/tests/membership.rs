use super::*;
use crate::handler::FixedConstraintMode;

fn membership_fixture() -> CardPool {
    let mut cards = eight_cards_for_leader_tests();
    for (index, card) in cards.iter_mut().enumerate() {
        card.char_id += 1;
        card.power = 12_000 + index as u32 * 1_700;
        card.power_max = card.power;
        card.skill = SkillSlot {
            skill_type: 0,
            value: 30 + index as u8 * 7,
        };
        card.skill_max = 30 + index as u8 * 7;
        card.base_bonus = 15 + index as u8;
        card.limited_bonus = (index as u8 % 3 + 1) * 10;
    }
    let mut cards = cards.to_vec();
    let mut alternative = cards[0];
    alternative.game_id = 450;
    alternative.power += 9_000;
    alternative.power_max = alternative.power;
    cards.push(alternative);
    build_pool(&cards)
}

fn member_context(pool: &CardPool, target: ScoreTarget) -> SearchContext {
    let mut ctx = ready_ctx(pool, target);
    ctx.fixed_constraint_mode = FixedConstraintMode::Members;
    ctx.is_final_chapter = true;
    ctx.is_world_bloom = true;
    ctx.event_type = Some(EventType::WorldBloom);
    ctx.best_skill_as_leader = false;
    ctx.card_bonus_count_limit = 3;
    ctx.skill_scores = [[0.03, 0.09, 0.02, 0.13, 0.05, 0.04]; 3];
    ctx.support_decks_by_character = (0..=26)
        .map(|leader| SupportDeck {
            cards: vec![(200, 20.0 + f64::from(leader)), (450, 10.0)],
            count: 1,
        })
        .collect();
    ctx.extra_bonus_ub = 50;
    ctx.leader_honor_bonus_x10 = pool
        .indices()
        .map(|card| u16::from(pool.char_id(card)) * 15)
        .collect();
    ctx
}

#[test]
fn final_members_all_targets_match_ordered_oracle() {
    let pool = membership_fixture();
    let params = SearchParams {
        top_k: 8,
        timeout_ms: 0,
    };
    for target in [
        ScoreTarget::Score,
        ScoreTarget::Mysekai,
        ScoreTarget::Bonus,
        ScoreTarget::Power,
        ScoreTarget::Skill,
    ] {
        for order in [
            LiveSkillOrder::Best,
            LiveSkillOrder::Average,
            LiveSkillOrder::Specific,
        ] {
            for forced in [None, Some(8)] {
                let mut ctx = member_context(&pool, target);
                ctx.live_skill_order = order;
                ctx.specific_skill_order = Some([4, 1, 3, 0, 2]);
                ctx.fixed_card_ids = vec![200];
                ctx.fixed_character_ids = vec![3, 5];
                ctx.forced_leader_character_id = forced;
                let (expected, _) = ExactOracle::new(&pool, &ctx).search(&params);
                let actual = search(&pool, &ctx, &params);
                assert_eq!(actual.completion(), SearchCompletion::Complete);
                assert_eq!(
                    actual.results, expected,
                    "target={target:?} order={order:?} leader={forced:?}"
                );
                assert!(!actual.results.is_empty());
                assert!(
                    actual
                        .results
                        .iter()
                        .all(|row| row.cards.iter().any(|&card| pool.game_id(card) == 200))
                );
            }
        }
    }
}

#[test]
fn final_members_five_pins_leave_leader_and_input_order_free() {
    let pool = membership_fixture();
    let params = SearchParams {
        top_k: 8,
        timeout_ms: 0,
    };
    for target in [
        ScoreTarget::Score,
        ScoreTarget::Bonus,
        ScoreTarget::Power,
        ScoreTarget::Skill,
    ] {
        for forced in [None, Some(5)] {
            let mut ctx = member_context(&pool, target);
            ctx.live_skill_order = LiveSkillOrder::Specific;
            ctx.specific_skill_order = Some([4, 1, 3, 0, 2]);
            ctx.fixed_card_ids = vec![200, 201, 202, 203, 204];
            ctx.forced_leader_character_id = forced;
            let (expected, _) = ExactOracle::new(&pool, &ctx).search(&params);
            let actual = search(&pool, &ctx, &params);
            assert_eq!(actual.completion(), SearchCompletion::Complete);
            assert_eq!(
                actual.results, expected,
                "target={target:?} leader={forced:?}"
            );
            assert_eq!(actual.results.len(), 1);
            ctx.fixed_card_ids.reverse();
            assert_eq!(search(&pool, &ctx, &params).results, expected);
            if let Some(forced) = forced {
                assert_eq!(pool.char_id(actual.results[0].cards[0]), forced);
            }
        }
    }
}

#[test]
fn final_members_minimum_power_and_character_pin_order_are_exact() {
    let pool = membership_fixture();
    let mut ctx = member_context(&pool, ScoreTarget::Power);
    ctx.minimize = true;
    ctx.fixed_character_ids = vec![5, 3];
    let params = SearchParams {
        top_k: 8,
        timeout_ms: 0,
    };
    let (expected, _) = ExactOracle::new(&pool, &ctx).search(&params);
    let actual = search(&pool, &ctx, &params);
    assert_eq!(actual.completion(), SearchCompletion::Complete);
    assert_eq!(actual.results, expected);
    ctx.fixed_character_ids.reverse();
    assert_eq!(search(&pool, &ctx, &params).results, expected);
}

#[test]
fn final_members_share_one_expired_budget_and_never_certify_partial_search() {
    let pool = membership_fixture();
    let mut ctx = member_context(&pool, ScoreTarget::Score);
    ctx.fixed_card_ids = vec![200];
    let params = SearchParams {
        top_k: 8,
        timeout_ms: 0,
    };
    let mut budget =
        super::super::budget::SearchBudget::new(Some(super::super::budget::Instant::now()));
    let (results, stats) = super::super::search_with_budget(&pool, &ctx, &params, &mut budget);
    assert!(results.is_empty());
    assert!(budget.hit);
    assert_eq!(stats.completion(), SearchCompletion::TimedOut);
}

#[test]
fn default_slots_still_reject_conflicting_final_leader() {
    let pool = membership_fixture();
    let mut ctx = member_context(&pool, ScoreTarget::Score);
    ctx.fixed_constraint_mode = FixedConstraintMode::Slots;
    ctx.fixed_card_ids = vec![200];
    ctx.forced_leader_character_id = Some(8);
    let actual = search(
        &pool,
        &ctx,
        &SearchParams {
            top_k: 3,
            timeout_ms: 0,
        },
    );
    assert_eq!(actual.completion(), SearchCompletion::Complete);
    assert!(actual.results.is_empty());
}

#[test]
fn final_members_without_pins_preserve_existing_search_results() {
    let pool = membership_fixture();
    let params = SearchParams {
        top_k: 8,
        timeout_ms: 0,
    };
    for forced in [None, Some(5)] {
        let mut ctx = member_context(&pool, ScoreTarget::Score);
        ctx.card_bonus_count_limit = DECK_SIZE;
        ctx.forced_leader_character_id = forced;
        ctx.fixed_character_ids = forced.into_iter().collect();
        let members = search(&pool, &ctx, &params);
        ctx.fixed_constraint_mode = FixedConstraintMode::Slots;
        let slots = search(&pool, &ctx, &params);
        assert_eq!(members.completion(), SearchCompletion::Complete);
        assert_eq!(members.results, slots.results);
    }
}
