//! Final chapter leader honor: one main honor per deck, chosen per leader character.
use super::*;
use crate::pool::{CardIdx, CardPool};
use crate::search::context::LeaderHonor;
use crate::search::{SearchContext, leaf_evaluate, summarize_deck};

const MARATHON_EVENT_ID: i32 = 42;

fn honor_row(
    event_id: i32,
    honor_id: i32,
    leader_game_character_id: i32,
    bonus_rate: i32,
) -> types::EventHonorBonus {
    types::EventHonorBonus {
        event_id,
        honor_id,
        leader_game_character_id,
        bonus_rate,
    }
}

fn final_row(honor_id: i32, leader: i32, bonus_rate: i32) -> types::EventHonorBonus {
    honor_row(FINAL_CHAPTER_EVENT_ID, honor_id, leader, bonus_rate)
}

fn final_params() -> BuildParams {
    BuildParams {
        target: ScoreTarget::Score,
        live_type: LiveType::Multi,
        event_id: Some(FINAL_CHAPTER_EVENT_ID),
        event_type: Some("world_bloom".to_string()),
        ..BuildParams::default()
    }
}

/// Five cards, one per character 1..=5, with identical power and skill.
fn five_card_fixture() -> BonusTierFixture {
    pool_constraint_fixture(&[
        (1, 4, 100),
        (2, 4, 100),
        (3, 4, 100),
        (4, 4, 100),
        (5, 4, 100),
    ])
}

fn user_owning(fixture: &BonusTierFixture, honors: &[(i32, i32)]) -> UserProfile {
    UserProfile {
        user_honors: honors
            .iter()
            .map(|&(honor_id, level)| types::UserHonor { honor_id, level })
            .collect(),
        ..pool_constraint_user(fixture)
    }
}

fn owned_ids(fixture: &BonusTierFixture, ids: &[i32]) -> UserProfile {
    user_owning(fixture, &ids.iter().map(|&id| (id, 1)).collect::<Vec<_>>())
}

fn build_with(
    fixture: &BonusTierFixture,
    rows: &[types::EventHonorBonus],
    user: &UserProfile,
    params: &BuildParams,
) -> Result<(CardPool, SearchContext), BuildError> {
    let game = GameData {
        event_honor_bonuses: rows,
        ..bonus_tier_game(fixture)
    };
    build_card_pool(user, &game, params)
}

fn dense(pool: &CardPool, game_card_id: u16) -> CardIdx {
    pool.indices()
        .find(|&card| pool.game_id(card) == game_card_id)
        .expect("card is in the pool")
}

/// The five fixture cards with `leader` in slot 0.
fn deck_led_by(pool: &CardPool, leader: u16) -> [CardIdx; 5] {
    let mut deck = [dense(pool, leader); 5];
    for (slot, id) in (1..=5u16).filter(|&id| id != leader).enumerate() {
        deck[slot + 1] = dense(pool, id);
    }
    deck
}

fn honor(honor_id: i32, bonus_x10: u16) -> Option<LeaderHonor> {
    Some(LeaderHonor {
        honor_id,
        bonus_x10,
    })
}

/// Every per-card leader honor value equals the chosen honor of its character.
fn assert_column_matches_table(pool: &CardPool, ctx: &SearchContext) {
    assert_eq!(ctx.leader_honor_bonus_x10.len(), pool.count());
    for card in pool.indices() {
        let expected = ctx
            .leader_honor_for_character(pool.char_id(card))
            .map_or(0, |honor| u32::from(honor.bonus_x10));
        assert_eq!(
            ctx.leader_honor_bonus_x10_at(card.raw()),
            expected,
            "card {}",
            pool.game_id(card)
        );
    }
}

#[test]
fn owned_matching_honors_do_not_stack() {
    let fixture = five_card_fixture();
    let rows = [final_row(11, 1, 50), final_row(12, 1, 50)];
    let user = owned_ids(&fixture, &[11, 12]);
    let (pool, ctx) = build_with(&fixture, &rows, &user, &final_params()).unwrap();

    assert_eq!(ctx.leader_honor_for_character(1), honor(11, 500));
    assert_eq!(ctx.leader_honor_bonus_x10_at(dense(&pool, 1).raw()), 500);
    assert_column_matches_table(&pool, &ctx);

    let summary = summarize_deck(&pool, &ctx, &deck_led_by(&pool, 1)).unwrap();
    assert_eq!(summary.event_bonus_total, Some(50.0));
    assert_eq!(summary.card_event_bonus_rates[0], 50.0);
    assert_eq!(summary.main_honor_id, Some(11));
}

#[test]
fn highest_single_owned_honor_wins() {
    let fixture = five_card_fixture();
    let rows = [
        final_row(21, 1, 30),
        final_row(22, 1, 70),
        final_row(23, 1, 50),
    ];
    let user = owned_ids(&fixture, &[21, 22, 23]);
    let (pool, ctx) = build_with(&fixture, &rows, &user, &final_params()).unwrap();

    assert_eq!(ctx.leader_honor_for_character(1), honor(22, 700));
    let summary = summarize_deck(&pool, &ctx, &deck_led_by(&pool, 1)).unwrap();
    assert_eq!(summary.event_bonus_total, Some(70.0));
    assert_eq!(summary.main_honor_id, Some(22));
}

#[test]
fn equal_bonus_prefers_the_smallest_honor_id_in_any_row_order() {
    let fixture = five_card_fixture();
    let user = owned_ids(&fixture, &[30, 31, 33, 35]);
    let mut rows = vec![
        final_row(35, 1, 50),
        final_row(31, 1, 50),
        final_row(30, 1, 40),
        final_row(33, 1, 50),
    ];
    for _ in 0..2 {
        let (pool, ctx) = build_with(&fixture, &rows, &user, &final_params()).unwrap();
        assert_eq!(ctx.leader_honor_for_character(1), honor(31, 500));
        let summary = summarize_deck(&pool, &ctx, &deck_led_by(&pool, 1)).unwrap();
        assert_eq!(summary.main_honor_id, Some(31));
        assert_eq!(summary.event_bonus_total, Some(50.0));
        rows.reverse();
    }
}

#[test]
fn unowned_honors_are_excluded() {
    let fixture = five_card_fixture();
    let rows = [
        final_row(41, 1, 100),
        final_row(42, 1, 20),
        final_row(43, 2, 60),
    ];
    let user = owned_ids(&fixture, &[42]);
    let (pool, ctx) = build_with(&fixture, &rows, &user, &final_params()).unwrap();

    assert_eq!(ctx.leader_honor_for_character(1), honor(42, 200));
    assert_eq!(ctx.leader_honor_for_character(2), None);
    assert_column_matches_table(&pool, &ctx);
    let led_by_2 = summarize_deck(&pool, &ctx, &deck_led_by(&pool, 2)).unwrap();
    assert_eq!(led_by_2.main_honor_id, None);
    assert_eq!(led_by_2.event_bonus_total, Some(0.0));

    let nobody = owned_ids(&fixture, &[]);
    let (pool, ctx) = build_with(&fixture, &rows, &nobody, &final_params()).unwrap();
    assert!(
        (0..=26).all(|character| ctx.leader_honor_for_character(character).is_none()),
        "no owned honor means no assumed main honor"
    );
    assert!(ctx.leader_honor_bonus_x10.iter().all(|&value| value == 0));
    let summary = summarize_deck(&pool, &ctx, &deck_led_by(&pool, 1)).unwrap();
    assert_eq!(summary.main_honor_id, None);
}

fn two_leader_rows() -> [types::EventHonorBonus; 4] {
    [
        final_row(51, 1, 50),
        final_row(52, 1, 30),
        final_row(61, 2, 20),
        final_row(62, 2, 40),
    ]
}

#[test]
fn main_honor_follows_the_leader_character() {
    let fixture = five_card_fixture();
    let rows = two_leader_rows();
    let user = owned_ids(&fixture, &[51, 52, 61, 62]);
    let params = final_params();
    let (pool, ctx) = build_with(&fixture, &rows, &user, &params).unwrap();

    assert_eq!(ctx.leader_honor_for_character(1), honor(51, 500));
    assert_eq!(ctx.leader_honor_for_character(2), honor(62, 400));
    assert_eq!(ctx.leader_honor_for_character(3), None);
    assert_column_matches_table(&pool, &ctx);

    let led = |leader: u16| {
        let deck = deck_led_by(&pool, leader);
        (
            summarize_deck(&pool, &ctx, &deck).unwrap(),
            leaf_evaluate(&pool, &ctx, &deck),
        )
    };
    let (by_1, score_1) = led(1);
    let (by_2, score_2) = led(2);
    let (by_3, score_3) = led(3);
    assert_eq!(
        (by_1.main_honor_id, by_1.event_bonus_total),
        (Some(51), Some(50.0))
    );
    assert_eq!(
        (by_2.main_honor_id, by_2.event_bonus_total),
        (Some(62), Some(40.0))
    );
    assert_eq!(
        (by_3.main_honor_id, by_3.event_bonus_total),
        (None, Some(0.0))
    );
    assert_eq!(by_1.live_score, by_2.live_score);
    assert!(by_1.event_point > by_2.event_point && by_2.event_point > by_3.event_point);
    assert!(score_1 > score_2 && score_2 > score_3);

    // Automatic leader: the searched Top-1 is led by character 1 and reports its honor.
    let decks = pool_constraint_search(&pool, &ctx, &params);
    assert_eq!(decks.len(), 1);
    assert_eq!(pool.char_id(decks[0].cards[0]), 1);
    assert_eq!(decks[0].score, score_1);
    assert_eq!(summarize_deck(&pool, &ctx, &decks[0].cards), Some(by_1));

    // Forced leader character 2: the result switches to that leader's honor.
    let forced = BuildParams {
        forced_leader_character_id: Some(2),
        ..final_params()
    };
    let (pool, ctx) = build_with(&fixture, &rows, &user, &forced).unwrap();
    let decks = pool_constraint_search(&pool, &ctx, &forced);
    assert_eq!(decks.len(), 1);
    assert_eq!(pool.char_id(decks[0].cards[0]), 2);
    let summary = summarize_deck(&pool, &ctx, &decks[0].cards).unwrap();
    assert_eq!(summary.main_honor_id, Some(62));
    assert_eq!(summary.event_bonus_total, Some(40.0));
    assert_eq!(
        summary,
        summarize_deck(&pool, &ctx, &deck_led_by(&pool, 2)).unwrap()
    );
}

#[test]
fn summed_honors_no_longer_decide_the_leader() {
    // Two 30% honors for character 1 would outrank character 2's 50% honor
    // only if they stacked.
    let fixture = five_card_fixture();
    let rows = [
        final_row(71, 1, 30),
        final_row(72, 1, 30),
        final_row(81, 2, 50),
    ];
    let user = owned_ids(&fixture, &[71, 72, 81]);
    let params = final_params();
    let (pool, ctx) = build_with(&fixture, &rows, &user, &params).unwrap();

    let decks = pool_constraint_search(&pool, &ctx, &params);
    assert_eq!(decks.len(), 1);
    assert_eq!(pool.char_id(decks[0].cards[0]), 2);
    let summary = summarize_deck(&pool, &ctx, &decks[0].cards).unwrap();
    assert_eq!(summary.main_honor_id, Some(81));
    assert_eq!(summary.event_bonus_total, Some(50.0));
    assert_eq!(
        decks[0].score,
        leaf_evaluate(&pool, &ctx, &deck_led_by(&pool, 2))
    );
}

#[test]
fn fixed_leader_card_reports_its_main_honor() {
    let fixture = five_card_fixture();
    let rows = two_leader_rows();
    let user = owned_ids(&fixture, &[51, 52, 61, 62]);
    for (leader, expected_id, expected_bonus) in [(1, 51, 50.0), (2, 62, 40.0)] {
        let params = BuildParams {
            fixed_cards: vec![leader],
            ..final_params()
        };
        let (pool, ctx) = build_with(&fixture, &rows, &user, &params).unwrap();
        let decks = pool_constraint_search(&pool, &ctx, &params);
        assert_eq!(decks.len(), 1);
        let summary = summarize_deck(&pool, &ctx, &decks[0].cards).unwrap();
        assert_eq!(pool.game_id(summary.ordered_cards[0]), leader as u16);
        assert_eq!(summary.main_honor_id, Some(expected_id));
        assert_eq!(summary.event_bonus_total, Some(expected_bonus));
    }
}

#[test]
fn non_final_events_have_no_leader_honor() {
    let mut fixture = five_card_fixture();
    fixture.events = vec![types::Event {
        id: MARATHON_EVENT_ID,
        event_type: "marathon".to_string(),
    }];
    let rows = [
        honor_row(MARATHON_EVENT_ID, 91, 1, 50),
        final_row(92, 1, 50),
    ];
    let user = owned_ids(&fixture, &[91, 92]);
    let marathon = BuildParams {
        target: ScoreTarget::Score,
        live_type: LiveType::Multi,
        event_id: Some(MARATHON_EVENT_ID),
        ..BuildParams::default()
    };
    let no_event = BuildParams {
        live_type: LiveType::Multi,
        ..BuildParams::default()
    };
    for params in [marathon, no_event] {
        let (pool, ctx) = build_with(&fixture, &rows, &user, &params).unwrap();
        assert!(!ctx.is_final_chapter);
        assert!(ctx.leader_honors.is_empty());
        assert!(ctx.leader_honor_bonus_x10.iter().all(|&value| value == 0));
        assert_eq!(ctx.leader_honor_for_character(1), None);
        let summary = summarize_deck(&pool, &ctx, &deck_led_by(&pool, 1)).unwrap();
        assert_eq!(summary.main_honor_id, None);
        assert_eq!(
            summary.event_bonus_total,
            params.event_id.map(|_| 0.0),
            "event {:?}",
            params.event_id
        );
    }

    // The per-character table is only consulted under final chapter rules.
    let (_, final_ctx) = build_with(&fixture, &rows, &user, &final_params()).unwrap();
    assert_eq!(final_ctx.leader_honor_for_character(1), honor(92, 500));
    let mut not_final = final_ctx.clone();
    not_final.is_final_chapter = false;
    assert_eq!(not_final.leader_honor_for_character(1), None);
}

#[test]
fn honor_power_bonus_still_counts_every_owned_honor() {
    let fixture = five_card_fixture();
    let rows = [final_row(11, 1, 50), final_row(12, 1, 50)];
    let honors = [
        types::Honor {
            id: 11,
            levels: vec![
                types::HonorLevel {
                    level: 1,
                    bonus: 100,
                },
                types::HonorLevel {
                    level: 2,
                    bonus: 250,
                },
            ],
            asset_bundle_name: None,
        },
        types::Honor {
            id: 12,
            levels: vec![types::HonorLevel {
                level: 1,
                bonus: 40,
            }],
            asset_bundle_name: None,
        },
        types::Honor {
            id: 13,
            levels: vec![types::HonorLevel {
                level: 1,
                bonus: 999,
            }],
            asset_bundle_name: None,
        },
    ];
    let user = user_owning(&fixture, &[(11, 2), (12, 1)]);
    let game = GameData {
        event_honor_bonuses: &rows,
        honors: &honors,
        ..bonus_tier_game(&fixture)
    };
    for params in [final_params(), BuildParams::default()] {
        let (pool, ctx) = build_card_pool(&user, &game, &params).unwrap();
        assert_eq!(ctx.honor_bonus, 290, "event {:?}", params.event_id);
        let summary = summarize_deck(&pool, &ctx, &deck_led_by(&pool, 1)).unwrap();
        let cards_power = summary.card_power_total.iter().sum::<i32>();
        assert_eq!(summary.total_power, cards_power + 290);
    }
    let (_, ctx) = build_card_pool(&user, &game, &final_params()).unwrap();
    assert_eq!(ctx.leader_honor_for_character(1), honor(11, 500));
}

#[test]
fn every_build_entry_point_makes_the_same_honor_choice() {
    let fixture = five_card_fixture();
    let rows = two_leader_rows();
    let user = owned_ids(&fixture, &[51, 52, 61, 62]);
    let game = GameData {
        event_honor_bonuses: &rows,
        ..bonus_tier_game(&fixture)
    };
    let params = final_params();

    let (pool, ctx) = build_card_pool(&user, &game, &params).unwrap();
    let (detail_pool, detail_ctx, details) =
        build_card_pool_with_details(&user, &game, &params).unwrap();
    let shared_indexes = PreparedGameIndexes::new(&game);
    let prepared = PreparedGameData::with_indexes(game, &shared_indexes);
    let (prepared_pool, prepared_ctx) =
        build_card_pool_prepared(&user, &prepared, &params).unwrap();
    let (prepared_detail_pool, prepared_detail_ctx, prepared_details) =
        build_card_pool_with_details_prepared(&user, &prepared, &params).unwrap();
    let prepared_build = PreparedPoolBuild::new(&user, &prepared, &params).unwrap();
    let (fully_pool, fully_ctx) =
        build_card_pool_fully_prepared(&prepared, &prepared_build).unwrap();
    let (fully_again_pool, fully_again_ctx) =
        build_card_pool_fully_prepared(&prepared, &prepared_build).unwrap();
    let (fully_detail_pool, fully_detail_ctx, fully_details) =
        build_card_pool_with_details_fully_prepared(&prepared, &prepared_build).unwrap();

    let game_ids = |pool: &CardPool| {
        pool.indices()
            .map(|card| pool.game_id(card))
            .collect::<Vec<_>>()
    };
    for (other_pool, other_ctx) in [
        (&detail_pool, &detail_ctx),
        (&prepared_pool, &prepared_ctx),
        (&prepared_detail_pool, &prepared_detail_ctx),
        (&fully_pool, &fully_ctx),
        (&fully_again_pool, &fully_again_ctx),
        (&fully_detail_pool, &fully_detail_ctx),
    ] {
        assert_eq!(game_ids(other_pool), game_ids(&pool));
        assert_eq!(other_ctx, &ctx);
    }
    assert_eq!(prepared_details, details);
    assert_eq!(fully_details, details);
    assert_eq!(ctx.leader_honor_for_character(1), honor(51, 500));
    assert_eq!(ctx.leader_honor_for_character(2), honor(62, 400));
    assert_column_matches_table(&pool, &ctx);
    for (index, detail) in details.iter().enumerate() {
        assert_eq!(
            u32::from(detail.leader_honor_bonus_x10),
            ctx.leader_honor_bonus_x10_at(index)
        );
    }
    let deck = deck_led_by(&pool, 2);
    let expected = summarize_deck(&pool, &ctx, &deck).unwrap();
    assert_eq!(expected.main_honor_id, Some(62));
    assert_eq!(
        summarize_deck(&fully_pool, &fully_ctx, &deck_led_by(&fully_pool, 2)),
        Some(expected)
    );
}

#[test]
fn leader_honor_width_is_checked_per_honor() {
    let fixture = five_card_fixture();
    // Each honor fits in 16-bit tenths; only their (invalid) sum would not.
    let rows = [final_row(1, 1, 4_000), final_row(2, 1, 4_000)];
    let user = owned_ids(&fixture, &[1, 2]);
    let (_, ctx) = build_with(&fixture, &rows, &user, &final_params()).unwrap();
    assert_eq!(ctx.leader_honor_for_character(1), honor(1, 40_000));

    let too_wide = [final_row(3, 1, 7_000)];
    let user = owned_ids(&fixture, &[3]);
    assert!(matches!(
        build_with(&fixture, &too_wide, &user, &final_params()),
        Err(BuildError::CapacityExceeded { .. })
    ));
}

#[test]
fn engine_recommendations_carry_the_main_honor_id() {
    let fixture = five_card_fixture();
    let rows = two_leader_rows();
    let user = owned_ids(&fixture, &[51, 52, 61, 62]);
    let game = GameData {
        event_honor_bonuses: &rows,
        ..bonus_tier_game(&fixture)
    };
    let params = BuildParams {
        limit: 1,
        ..final_params()
    };
    let outcome = crate::engine::recommend(&user, &game, &params).unwrap();
    assert_eq!(
        outcome.completion(),
        crate::search::SearchCompletion::Complete
    );
    assert_eq!(outcome.results.len(), 1);
    assert_eq!(outcome.results[0].cards[0], 1);
    assert_eq!(outcome.results[0].main_honor_id, Some(51));
    let json = serde_json::to_value(outcome.results[0]).unwrap();
    assert_eq!(json["main_honor_id"], 51);

    let plain = BuildParams {
        limit: 1,
        live_type: LiveType::Multi,
        ..BuildParams::default()
    };
    let outcome = crate::engine::recommend(&user, &game, &plain).unwrap();
    assert_eq!(outcome.results[0].main_honor_id, None);
    let json = serde_json::to_value(outcome.results[0]).unwrap();
    assert!(json.get("main_honor_id").is_none());

    let legacy: crate::engine::Recommendation =
        serde_json::from_str(r#"{"cards":[1,2,3,4,5],"score":7}"#).unwrap();
    assert_eq!(legacy.main_honor_id, None);
}
