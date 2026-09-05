//! search 模块测试。

use super::*;
use crate::pool::{DiffSkill, EventBonusExact, PoolBuilder, RefSkill, SkillSlot, UnitCountSkill};
use crate::types::{EventType, LiveSkillOrder, LiveType, ScoreTarget, SkillReferenceStrategy};

#[derive(Clone, Copy)]
struct TestCard {
    char_id: u8,
    attr: u8,
    unit_mask: u8,
    game_id: u16,
    power: u32,
    skill: SkillSlot,
    base_bonus: u8,
    limited_bonus: u8,
    power_max: u32,
    skill_max: u8,
}

fn encode_power(value: u32) -> ([u16; 8], u32) {
    let low = value as u16;
    let high = (value >> 16) & 3;
    let mut values = [0u16; 8];
    let mut high_bits = 0u32;
    let mut idx = 0usize;
    while idx < values.len() {
        values[idx] = low;
        high_bits |= high << (idx << 1);
        idx += 1;
    }
    (values, high_bits)
}

fn build_pool(cards: &[TestCard]) -> CardPool {
    let mut builder = PoolBuilder::new(cards.len() as u16);
    builder.add_unit_count_skill(UnitCountSkill {
        unit: 0,
        score_up: [10, 20, 30, 40, 50],
    });
    builder.add_diff_skill(DiffSkill {
        base: 12,
        increment: 6,
    });
    builder.add_ref_skill(RefSkill { rate: 50, max: 30 });

    let mut idx = 0usize;
    while idx < cards.len() {
        let card = unsafe { *cards.get_unchecked(idx) };
        let dense = idx as u16;
        let (values, high_bits) = encode_power(card.power);
        builder.set_power_values(dense, values);
        builder.set_power_lut(dense, high_bits);
        builder.set_skill(dense, card.skill);
        builder.set_event_bonus(
            dense,
            EventBonusExact::from_whole(card.base_bonus as u16, card.limited_bonus as u16),
        );
        builder.set_char_id(dense, card.char_id);
        builder.set_attr(dense, card.attr);
        builder.set_unit_mask(dense, card.unit_mask);
        builder.set_game_id(dense, card.game_id);
        builder.set_power_max(dense, card.power_max);
        builder.set_skill_min(dense, card.skill_max);
        builder.set_skill_max(dense, card.skill_max);
        builder.mark_char(card.char_id, dense);
        let mut unit = 0u8;
        while unit < 6 {
            if card.unit_mask & (1u8 << unit) != 0 {
                builder.mark_unit(unit, dense);
            }
            unit += 1;
        }
        builder.mark_attr(card.attr, dense);
        idx += 1;
    }

    builder.freeze()
}

fn ctx(target: ScoreTarget) -> SearchContext {
    SearchContext {
        target,
        fixed_card_ids: Vec::new(),
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
        support_deck: SupportDeck::default(),
        support_decks_by_character: Vec::new(),
        is_world_bloom: false,
        is_final_chapter: false,
        enforce_char_uniqueness: true,
        minimize: false,
        live_type: LiveType::Solo,
        event_type: None,
        keep_after_training_state: false,
        skill_reference_strategy: SkillReferenceStrategy::Average,
        best_skill_as_leader: true,
        live_skill_order: LiveSkillOrder::Best,
        specific_skill_order: None,
        multi_teammate_score_up: None,
        multi_teammate_power: None,
        multi_live_score_up_lower_bound: None,
        extra_bonus_ub: 0,
        w_power: 2.0,
        w_bonus: 1.0,
        skill_ub_global: 0,
        card_bonus_count_limit: DECK_SIZE,
        honor_bonus: 0,
        power_total_cap: None,
        leader_honor_bonus: Vec::new(),
        leader_limit_bonus: Vec::new(),
        final_chapter_member_keep: Vec::new(),
        skill_is_after_training: Vec::new(),
        trained_to_special_image: Vec::new(),
    }
}

fn ready_ctx(pool: &CardPool, target: ScoreTarget) -> SearchContext {
    let mut ctx = ctx(target);
    ctx.leader_honor_bonus = vec![0; pool.count()];
    ctx.leader_limit_bonus = vec![0; pool.count()];
    ctx.skill_is_after_training = vec![false; pool.count()];
    ctx.trained_to_special_image = vec![false; pool.count()];
    ctx
}

fn five_unique_cards() -> [TestCard; 5] {
    [
        TestCard {
            char_id: 0,
            attr: 0,
            unit_mask: 1,
            game_id: 100,
            power: 100,
            skill: SkillSlot {
                skill_type: 0,
                value: 10,
            },
            base_bonus: 10,
            limited_bonus: 0,
            power_max: 100,
            skill_max: 10,
        },
        TestCard {
            char_id: 1,
            attr: 0,
            unit_mask: 1,
            game_id: 101,
            power: 200,
            skill: SkillSlot {
                skill_type: 0,
                value: 20,
            },
            base_bonus: 10,
            limited_bonus: 0,
            power_max: 200,
            skill_max: 20,
        },
        TestCard {
            char_id: 2,
            attr: 0,
            unit_mask: 1,
            game_id: 102,
            power: 300,
            skill: SkillSlot {
                skill_type: 0,
                value: 30,
            },
            base_bonus: 10,
            limited_bonus: 0,
            power_max: 300,
            skill_max: 30,
        },
        TestCard {
            char_id: 3,
            attr: 0,
            unit_mask: 1,
            game_id: 103,
            power: 400,
            skill: SkillSlot {
                skill_type: 0,
                value: 40,
            },
            base_bonus: 10,
            limited_bonus: 0,
            power_max: 400,
            skill_max: 40,
        },
        TestCard {
            char_id: 4,
            attr: 0,
            unit_mask: 1,
            game_id: 104,
            power: 500,
            skill: SkillSlot {
                skill_type: 0,
                value: 50,
            },
            base_bonus: 10,
            limited_bonus: 0,
            power_max: 500,
            skill_max: 50,
        },
    ]
}

fn collect_first_five(pool: &CardPool) -> [crate::pool::CardIdx; 5] {
    let mut deck = [crate::pool::CardIdx::new(0); 5];
    let mut idx = 0usize;
    for card in pool.indices() {
        deck[idx] = card;
        idx += 1;
        if idx == 5 {
            break;
        }
    }
    deck
}

fn brute_force_best(pool: &CardPool, search_ctx: &SearchContext) -> u64 {
    let mut brute = 0u64;
    let mut a = 0usize;
    while a < pool.count() {
        let c0 = crate::pool::CardIdx::new(a as u16);
        let mut b = a + 1;
        while b < pool.count() {
            let c1 = crate::pool::CardIdx::new(b as u16);
            let mut c = b + 1;
            while c < pool.count() {
                let c2 = crate::pool::CardIdx::new(c as u16);
                let mut d = c + 1;
                while d < pool.count() {
                    let c3 = crate::pool::CardIdx::new(d as u16);
                    let mut e = d + 1;
                    while e < pool.count() {
                        let c4 = crate::pool::CardIdx::new(e as u16);
                        let score = leaf_evaluate(pool, search_ctx, &[c0, c1, c2, c3, c4]);
                        if score > brute {
                            brute = score;
                        }
                        e += 1;
                    }
                    d += 1;
                }
                c += 1;
            }
            b += 1;
        }
        a += 1;
    }
    brute
}

#[test]
fn search_leaf_evaluate_encodes_targets() {
    let pool = build_pool(&five_unique_cards());
    let deck = collect_first_five(&pool);

    let power_value = leaf_evaluate(&pool, &ctx(ScoreTarget::Power), &deck);
    assert_eq!(power_value, 1500);

    let skill_value = leaf_evaluate(&pool, &ctx(ScoreTarget::Skill), &deck);
    assert_eq!(skill_value, 700);

    let score_value = leaf_evaluate(&pool, &ctx(ScoreTarget::Score), &deck);
    assert_eq!(score_value, ((6000u64) << 32) | 6000u64);
}

#[test]
fn prepared_multi_event_search_matches_standard_pipeline() {
    let mut cards = five_unique_cards().to_vec();
    cards.extend([
        TestCard {
            char_id: 5,
            attr: 1,
            unit_mask: 1,
            game_id: 150,
            power: 650,
            skill: SkillSlot {
                skill_type: 0,
                value: 65,
            },
            base_bonus: 20,
            limited_bonus: 0,
            power_max: 650,
            skill_max: 65,
        },
        TestCard {
            char_id: 6,
            attr: 2,
            unit_mask: 1,
            game_id: 151,
            power: 625,
            skill: SkillSlot {
                skill_type: 0,
                value: 72,
            },
            base_bonus: 35,
            limited_bonus: 0,
            power_max: 625,
            skill_max: 72,
        },
        TestCard {
            char_id: 0,
            attr: 3,
            unit_mask: 1,
            game_id: 152,
            power: 620,
            skill: SkillSlot {
                skill_type: 0,
                value: 60,
            },
            base_bonus: 45,
            limited_bonus: 0,
            power_max: 620,
            skill_max: 60,
        },
    ]);
    let pool = build_pool(&cards);
    let mut search_ctx = ready_ctx(&pool, ScoreTarget::Score);
    search_ctx.live_type = LiveType::Multi;
    search_ctx.event_type = Some(EventType::Marathon);
    search_ctx.skill_scores[1] = [10.0; DECK_SIZE + 1];
    let params = SearchParams {
        top_k: 3,
        timeout_ms: 0,
    };

    let expected = search(&pool, &search_ctx, &params);
    let prepared = PreparedSearch::build(&pool, &search_ctx, params.top_k).unwrap();
    let (actual, _) = prepared
        .search_instrumented(&pool, &search_ctx, &params)
        .unwrap();

    assert_eq!(actual, expected);
}

#[test]
fn search_leaf_evaluate_score_path_consumes_music_skill_tables() {
    let pool = build_pool(&five_unique_cards());
    let deck = collect_first_five(&pool);
    let mut search_ctx = ctx(ScoreTarget::Score);
    search_ctx.skill_scores[0] = [10.0; 6];

    let score_value = leaf_evaluate(&pool, &search_ctx, &deck);
    assert_eq!(score_value, ((126000u64) << 32) | 126000u64);
}

#[test]
fn search_leaf_evaluate_applies_power_cap() {
    let cards = five_unique_cards().map(|mut card| {
        card.power = 1000;
        card.power_max = 1000;
        card
    });
    let pool = build_pool(&cards);
    let deck = collect_first_five(&pool);
    let mut search_ctx = ctx(ScoreTarget::Power);
    search_ctx.power_total_cap = Some(3_500);

    assert_eq!(leaf_evaluate(&pool, &search_ctx, &deck), 3_500);
}

#[test]
fn search_fixed_card_constraint_is_respected() {
    let mut cards = five_unique_cards().to_vec();
    cards.push(TestCard {
        char_id: 5,
        attr: 0,
        unit_mask: 1,
        game_id: 150,
        power: 1,
        skill: SkillSlot::default(),
        base_bonus: 0,
        limited_bonus: 0,
        power_max: 1,
        skill_max: 0,
    });
    let pool = build_pool(&cards);
    let mut search_ctx = ready_ctx(&pool, ScoreTarget::Power);
    search_ctx.fixed_card_ids = vec![150];
    let results = search(
        &pool,
        &search_ctx,
        &SearchParams {
            top_k: 1,
            timeout_ms: 0,
        },
    );

    assert_eq!(results.len(), 1);
    assert_eq!(pool.game_id(results[0].cards[0]), 150);
}

#[test]
fn search_fixed_character_constraint_is_respected() {
    let pool = build_pool(&five_unique_cards());
    let mut search_ctx = ready_ctx(&pool, ScoreTarget::Power);
    search_ctx.fixed_character_ids = vec![1, 3];
    let results = search(
        &pool,
        &search_ctx,
        &SearchParams {
            top_k: 1,
            timeout_ms: 0,
        },
    );

    assert_eq!(results.len(), 1);
    assert_eq!(pool.char_id(results[0].cards[0]), 1);
    assert_eq!(pool.char_id(results[0].cards[1]), 3);
}

#[test]
fn search_fixed_card_and_character_can_combine() {
    // 放开 fixed_cards ⊕ fixed_characters 互斥后：两者同时非空应被接受，
    // 引擎按「卡在前、角色在后」前缀填槽——槽0=固定卡(队长)，槽1=固定角色。
    let pool = build_pool(&five_unique_cards());
    let mut search_ctx = ready_ctx(&pool, ScoreTarget::Power);
    search_ctx.fixed_card_ids = vec![102]; // game_id 102 = char 2（见 five_unique_cards）
    search_ctx.fixed_character_ids = vec![4];
    let results = search(
        &pool,
        &search_ctx,
        &SearchParams {
            top_k: 1,
            timeout_ms: 0,
        },
    );

    assert_eq!(results.len(), 1);
    // 槽0 = 固定卡 102（队长）
    assert_eq!(pool.game_id(results[0].cards[0]), 102);
    // 槽1 = 固定角色 4
    assert_eq!(pool.char_id(results[0].cards[1]), 4);
}

/// 指定队长的六卡池：char 0 最弱，无约束时最优解会把它排除在外。
fn six_cards_with_weak_first() -> [TestCard; 6] {
    let base = five_unique_cards();
    [
        base[0],
        base[1],
        base[2],
        base[3],
        base[4],
        TestCard {
            char_id: 5,
            attr: 0,
            unit_mask: 1,
            game_id: 105,
            power: 600,
            skill: SkillSlot {
                skill_type: 0,
                value: 60,
            },
            base_bonus: 10,
            limited_bonus: 0,
            power_max: 600,
            skill_max: 60,
        },
    ]
}

#[test]
fn forced_leader_character_takes_the_leader_slot() {
    let pool = build_pool(&five_unique_cards());
    let mut search_ctx = ready_ctx(&pool, ScoreTarget::Score);
    search_ctx.live_type = LiveType::Multi;
    // 默认「最高技能作队长」会把 char 4 摆到队长位；指定队长必须压过它。
    search_ctx.best_skill_as_leader = true;
    search_ctx.forced_leader_character_id = Some(1);

    assert!(!search_ctx.effective_best_skill_as_leader());

    let deck = collect_first_five(&pool);
    let summary = summarize_deck(&pool, &search_ctx, &deck).expect("summary");
    assert_eq!(pool.char_id(summary.ordered_cards[0]), 1);
}

#[test]
fn forced_leader_character_is_ignored_when_unset() {
    let pool = build_pool(&five_unique_cards());
    let mut search_ctx = ready_ctx(&pool, ScoreTarget::Score);
    search_ctx.live_type = LiveType::Multi;
    search_ctx.best_skill_as_leader = true;

    let deck = collect_first_five(&pool);
    let summary = summarize_deck(&pool, &search_ctx, &deck).expect("summary");
    // 技能最高的是 char 4。
    assert_eq!(pool.char_id(summary.ordered_cards[0]), 4);
}

#[test]
fn forced_leader_character_keeps_that_character_in_the_deck() {
    let pool = build_pool(&six_cards_with_weak_first());
    let unconstrained = search(
        &pool,
        &ready_ctx(&pool, ScoreTarget::Power),
        &SearchParams {
            top_k: 1,
            timeout_ms: 0,
        },
    );
    assert_eq!(unconstrained.len(), 1);
    assert!(
        !unconstrained[0]
            .cards
            .iter()
            .any(|card| pool.char_id(*card) == 0),
        "最弱角色本不该出现在无约束最优解里",
    );

    let mut search_ctx = ready_ctx(&pool, ScoreTarget::Power);
    search_ctx.forced_leader_character_id = Some(0);
    let results = search(
        &pool,
        &search_ctx,
        &SearchParams {
            top_k: 3,
            timeout_ms: 0,
        },
    );
    assert!(!results.is_empty());
    for result in &results {
        assert!(
            result.cards.iter().any(|card| pool.char_id(*card) == 0),
            "指定队长后每条结果都必须包含该角色",
        );
    }
}

#[test]
fn forced_leader_character_wins_over_a_fixed_card_in_another_slot() {
    // 固定一张别的角色的卡 + 指定队长：队长位归指定角色，固定卡退到其他槽位。
    let pool = build_pool(&five_unique_cards());
    let mut search_ctx = ready_ctx(&pool, ScoreTarget::Score);
    search_ctx.live_type = LiveType::Multi;
    search_ctx.fixed_card_ids = vec![104]; // game_id 104 = char 4
    search_ctx.forced_leader_character_id = Some(1);

    let results = search(
        &pool,
        &search_ctx,
        &SearchParams {
            top_k: 1,
            timeout_ms: 0,
        },
    );
    assert_eq!(results.len(), 1);
    let summary = summarize_deck(&pool, &search_ctx, &results[0].cards).expect("summary");
    assert_eq!(pool.char_id(summary.ordered_cards[0]), 1);
    assert!(
        summary
            .ordered_cards
            .iter()
            .any(|card| pool.game_id(*card) == 104),
        "固定卡仍须留在队内",
    );
}

/// 指定队长的八卡池：角色 0..7 各一张卡，power / skill / 加成都取 2 的幂，
/// 任意 5 张的和都唯一——暴力对拍不会因并列分数而在名次内换序。
fn eight_cards_for_leader_tests() -> [TestCard; 8] {
    let mut cards = [TestCard {
        char_id: 0,
        attr: 0,
        unit_mask: 1,
        game_id: 200,
        power: 64,
        skill: SkillSlot {
            skill_type: 0,
            value: 1,
        },
        base_bonus: 1,
        limited_bonus: 0,
        power_max: 64,
        skill_max: 1,
    }; 8];
    for (index, card) in cards.iter_mut().enumerate() {
        let char_id = index as u8;
        card.char_id = char_id;
        card.attr = char_id % 4;
        card.unit_mask = 1 << (char_id % 3);
        card.game_id = 200 + char_id as u16;
        card.power = 64 << char_id;
        card.power_max = 64 << char_id;
        card.skill = SkillSlot {
            skill_type: 0,
            value: 1 << char_id,
        };
        card.skill_max = 1 << char_id;
        card.base_bonus = 1 << char_id;
    }
    cards
}

fn leader_character_of(
    pool: &CardPool,
    ctx: &SearchContext,
    deck: &[crate::pool::CardIdx; 5],
) -> u8 {
    let summary = summarize_deck(pool, ctx, deck).expect("summary");
    pool.char_id(summary.ordered_cards[0])
}

#[test]
fn forced_leader_search_matches_bruteforce_for_every_target() {
    let pool = build_pool(&eight_cards_for_leader_tests());
    let params = SearchParams {
        top_k: 4,
        timeout_ms: 0,
    };
    for target in [
        ScoreTarget::Score,
        ScoreTarget::Power,
        ScoreTarget::Skill,
        ScoreTarget::Bonus,
    ] {
        for leader in [0u8, 3, 6] {
            let mut search_ctx = ready_ctx(&pool, target);
            search_ctx.live_type = LiveType::Multi;
            search_ctx.event_type = Some(EventType::Marathon);
            search_ctx.forced_leader_character_id = Some(leader);

            let results = search(&pool, &search_ctx, &params);
            let (brute, _) = brute_force_search(&pool, &search_ctx, &params);
            assert_results_match_bruteforce(&pool, &results, &brute);
            assert!(
                !results.is_empty(),
                "target={target:?} leader={leader} 应有结果",
            );
            for result in &results {
                assert_eq!(
                    leader_character_of(&pool, &search_ctx, &result.cards),
                    leader,
                    "target={target:?} leader={leader} 队长位不对",
                );
            }
        }
    }
}

#[test]
fn forced_leader_holds_for_every_top_k_deck() {
    let pool = build_pool(&eight_cards_for_leader_tests());
    let mut search_ctx = ready_ctx(&pool, ScoreTarget::Score);
    search_ctx.live_type = LiveType::Multi;
    search_ctx.event_type = Some(EventType::Marathon);
    search_ctx.forced_leader_character_id = Some(2);

    let results = search(
        &pool,
        &search_ctx,
        &SearchParams {
            top_k: 8,
            timeout_ms: 0,
        },
    );
    assert!(results.len() > 1);
    for result in &results {
        assert_eq!(leader_character_of(&pool, &search_ctx, &result.cards), 2);
        assert!(result.cards.iter().any(|card| pool.char_id(*card) == 2));
    }
}

#[test]
fn forced_leader_survives_minimize_and_fixed_characters() {
    let pool = build_pool(&eight_cards_for_leader_tests());

    // 反向搜索（最弱综合力）。
    let mut minimize_ctx = ready_ctx(&pool, ScoreTarget::Power);
    minimize_ctx.minimize = true;
    minimize_ctx.forced_leader_character_id = Some(7);
    let params = SearchParams {
        top_k: 2,
        timeout_ms: 0,
    };
    let results = search(&pool, &minimize_ctx, &params);
    let (brute, _) = brute_force_search(&pool, &minimize_ctx, &params);
    assert_results_match_bruteforce(&pool, &results, &brute);
    for result in &results {
        assert_eq!(leader_character_of(&pool, &minimize_ctx, &result.cards), 7);
    }

    // 与固定角色共存：两个约束都要满足，队长位归指定角色。
    let mut combined_ctx = ready_ctx(&pool, ScoreTarget::Score);
    combined_ctx.live_type = LiveType::Multi;
    combined_ctx.event_type = Some(EventType::Marathon);
    combined_ctx.fixed_character_ids = vec![1, 5];
    combined_ctx.forced_leader_character_id = Some(5);
    let results = search(&pool, &combined_ctx, &params);
    assert!(!results.is_empty());
    for result in &results {
        assert!(result.cards.iter().any(|card| pool.char_id(*card) == 1));
        assert_eq!(leader_character_of(&pool, &combined_ctx, &result.cards), 5);
    }
}

#[test]
fn forced_leader_for_an_absent_character_returns_nothing() {
    let pool = build_pool(&eight_cards_for_leader_tests());
    let mut search_ctx = ready_ctx(&pool, ScoreTarget::Score);
    search_ctx.live_type = LiveType::Multi;
    search_ctx.event_type = Some(EventType::Marathon);
    search_ctx.forced_leader_character_id = Some(20); // 池里没有的角色

    let results = search(
        &pool,
        &search_ctx,
        &SearchParams {
            top_k: 3,
            timeout_ms: 0,
        },
    );
    assert!(results.is_empty(), "队长角色不在池里时不得产出卡组");
}

#[test]
fn forced_leader_does_not_change_results_when_it_matches_the_natural_leader() {
    // 指定的队长恰好就是默认（最高技能）队长时，结果集必须与不指定完全一致。
    let pool = build_pool(&eight_cards_for_leader_tests());
    let params = SearchParams {
        top_k: 3,
        timeout_ms: 0,
    };
    let mut free_ctx = ready_ctx(&pool, ScoreTarget::Score);
    free_ctx.live_type = LiveType::Multi;
    free_ctx.event_type = Some(EventType::Marathon);
    let free = search(&pool, &free_ctx, &params);
    assert!(!free.is_empty());
    let natural_leader = leader_character_of(&pool, &free_ctx, &free[0].cards);

    let mut forced_ctx = free_ctx.clone();
    forced_ctx.forced_leader_character_id = Some(natural_leader);
    let forced = search(&pool, &forced_ctx, &params);

    assert_eq!(forced[0].score, free[0].score);
    assert_eq!(
        forced[0].game_card_set_key(&pool),
        free[0].game_card_set_key(&pool),
    );
}

#[test]
fn search_multi_score_up_lower_bound_filters_invalid_decks() {
    let pool = build_pool(&five_unique_cards());
    let mut search_ctx = ready_ctx(&pool, ScoreTarget::Power);
    search_ctx.live_type = LiveType::Multi;
    search_ctx.multi_live_score_up_lower_bound = Some(1_000.0);
    let results = search(
        &pool,
        &search_ctx,
        &SearchParams {
            top_k: 1,
            timeout_ms: 0,
        },
    );

    assert!(results.is_empty());
}

#[test]
fn search_power_scenarios_matches_bruteforce_with_unit_and_attr_bonuses() {
    let mut builder = PoolBuilder::new(8);
    for dense in 0..8u16 {
        let grouped = dense < 5;
        let values = if grouped {
            [100, 200, 300, 1_000, 100, 200, 300, 1_000]
        } else {
            [500; 8]
        };
        builder.set_power_values(dense, values);
        builder.set_power_lut(dense, 0);
        builder.set_char_id(dense, (dense + 1) as u8);
        builder.set_attr(dense, if grouped { 1 } else { (dense - 4) as u8 });
        builder.set_unit_mask(dense, if grouped { 1 << 1 } else { 1 << (dense - 4) });
        builder.set_game_id(dense, dense + 100);
        builder.set_power_max(dense, if grouped { 1_000 } else { 500 });
        builder.mark_char((dense + 1) as u8, dense);
        builder.mark_attr(if grouped { 1 } else { (dense - 4) as u8 }, dense);
        builder.mark_unit(if grouped { 1 } else { (dense - 4) as u8 }, dense);
    }
    let pool = builder.freeze();
    let search_ctx = ready_ctx(&pool, ScoreTarget::Power);
    let expected = brute_force_best(&pool, &search_ctx);
    let actual = search(
        &pool,
        &search_ctx,
        &SearchParams {
            top_k: 3,
            timeout_ms: 0,
        },
    );

    assert_eq!(actual.first().map(|result| result.score), Some(expected));
    assert_eq!(expected, 5_000);
}

/// 暴力枚举 5 角色互异的最小 power deck（与 minimize 搜索对照）。
fn brute_force_worst_power(pool: &CardPool, search_ctx: &SearchContext) -> u64 {
    let mut worst = u64::MAX;
    let n = pool.count();
    let idx = |i: usize| crate::pool::CardIdx::new(i as u16);
    for a in 0..n {
        for b in (a + 1)..n {
            for c in (b + 1)..n {
                for d in (c + 1)..n {
                    for e in (d + 1)..n {
                        let deck = [idx(a), idx(b), idx(c), idx(d), idx(e)];
                        // 5 角色互异约束（与 enforce_char_uniqueness 一致）。
                        let mut chars = deck.map(|card| pool.char_id(card));
                        chars.sort_unstable();
                        if chars.windows(2).any(|w| w[0] == w[1]) {
                            continue;
                        }
                        let score = leaf_evaluate(pool, search_ctx, &deck);
                        if score < worst {
                            worst = score;
                        }
                    }
                }
            }
        }
    }
    worst
}

#[test]
fn search_minimize_power_matches_bruteforce_worst() {
    // 最弱组卡：minimize=true 应返回 power 最小的 5 角色互异 deck，
    // 与暴力枚举一致。卡池跨 7 角色、power 各异，确保有真正的「最弱」组合。
    let mut cards = Vec::new();
    let powers = [120u32, 90, 250, 60, 400, 30, 180];
    for (i, &p) in powers.iter().enumerate() {
        cards.push(TestCard {
            char_id: i as u8,
            attr: (i % 4) as u8,
            unit_mask: 1,
            game_id: 200 + i as u16,
            power: p,
            skill: SkillSlot {
                skill_type: 0,
                value: 10,
            },
            base_bonus: 0,
            limited_bonus: 0,
            power_max: p,
            skill_max: 10,
        });
    }
    let pool = build_pool(&cards);
    let mut search_ctx = ready_ctx(&pool, ScoreTarget::Power);
    search_ctx.minimize = true;

    let results = search(
        &pool,
        &search_ctx,
        &SearchParams {
            top_k: 1,
            timeout_ms: 0,
        },
    );
    assert_eq!(results.len(), 1, "minimize 应返回 1 个结果");

    let expected = brute_force_worst_power(&pool, &search_ctx);
    assert_eq!(
        results[0].score, expected,
        "minimize 搜索结果应等于暴力最弱 power"
    );

    // 最弱解应为 5 张最小 power 卡：30+60+90+120+180 = 480。
    assert_eq!(results[0].score, 480);

    // 反向验证：同池 maximize 应严格更大（取最强 5 张）。
    let mut max_ctx = ready_ctx(&pool, ScoreTarget::Power);
    max_ctx.minimize = false;
    let max_results = search(
        &pool,
        &max_ctx,
        &SearchParams {
            top_k: 1,
            timeout_ms: 0,
        },
    );
    assert!(
        max_results[0].score > results[0].score,
        "maximize({}) 应大于 minimize({})",
        max_results[0].score,
        results[0].score
    );
}

#[test]
fn search_suffix_bound_is_sound_and_zero_pool_is_zero() {
    let mut cards = five_unique_cards().to_vec();
    cards.push(TestCard {
        char_id: 5,
        attr: 1,
        unit_mask: 1,
        game_id: 105,
        power: 50,
        skill: SkillSlot {
            skill_type: 0,
            value: 5,
        },
        base_bonus: 0,
        limited_bonus: 0,
        power_max: 50,
        skill_max: 5,
    });
    let pool = build_pool(&cards);
    let search_ctx = ctx(ScoreTarget::Score);
    let suffix = SuffixBound::build(&pool, &search_ctx);

    let selected = pool.card_idx(0).unwrap_or(crate::pool::CardIdx::new(0));
    let mut used = UsedSet::new();
    used.insert(pool.char_id(selected));
    let partial = PartialDeck {
        power: pool.power_max(selected),
        skill: pool.skill_max(selected) as u32,
        bonus: pool.event_bonus_exact(selected).base_ceil(),
        max_skill: pool.skill_max(selected),
        limited_count: 0,
    };

    let upper = suffix.upper_bound_with_depth(1, &used, &partial);
    let mut best_real = 0u64;
    let mut i = 1usize;
    while i < pool.count() {
        let c1 = crate::pool::CardIdx::new(i as u16);
        let mut j = i + 1;
        while j < pool.count() {
            let c2 = crate::pool::CardIdx::new(j as u16);
            let mut k = j + 1;
            while k < pool.count() {
                let c3 = crate::pool::CardIdx::new(k as u16);
                let mut l = k + 1;
                while l < pool.count() {
                    let c4 = crate::pool::CardIdx::new(l as u16);
                    let deck = [selected, c1, c2, c3, c4];
                    let score = leaf_evaluate(&pool, &search_ctx, &deck);
                    if score > best_real {
                        best_real = score;
                    }
                    l += 1;
                }
                k += 1;
            }
            j += 1;
        }
        i += 1;
    }
    assert!(upper >= best_real);

    let zero_cards = [
        TestCard {
            char_id: 0,
            attr: 0,
            unit_mask: 1,
            game_id: 200,
            power: 0,
            skill: SkillSlot {
                skill_type: 0,
                value: 0,
            },
            base_bonus: 0,
            limited_bonus: 0,
            power_max: 0,
            skill_max: 0,
        },
        TestCard {
            char_id: 1,
            attr: 0,
            unit_mask: 1,
            game_id: 201,
            power: 0,
            skill: SkillSlot {
                skill_type: 0,
                value: 0,
            },
            base_bonus: 0,
            limited_bonus: 0,
            power_max: 0,
            skill_max: 0,
        },
        TestCard {
            char_id: 2,
            attr: 0,
            unit_mask: 1,
            game_id: 202,
            power: 0,
            skill: SkillSlot {
                skill_type: 0,
                value: 0,
            },
            base_bonus: 0,
            limited_bonus: 0,
            power_max: 0,
            skill_max: 0,
        },
        TestCard {
            char_id: 3,
            attr: 0,
            unit_mask: 1,
            game_id: 203,
            power: 0,
            skill: SkillSlot {
                skill_type: 0,
                value: 0,
            },
            base_bonus: 0,
            limited_bonus: 0,
            power_max: 0,
            skill_max: 0,
        },
        TestCard {
            char_id: 4,
            attr: 0,
            unit_mask: 1,
            game_id: 204,
            power: 0,
            skill: SkillSlot {
                skill_type: 0,
                value: 0,
            },
            base_bonus: 0,
            limited_bonus: 0,
            power_max: 0,
            skill_max: 0,
        },
    ];
    let zero_pool = build_pool(&zero_cards);
    let zero_suffix = SuffixBound::build(&zero_pool, &ctx(ScoreTarget::Power));
    assert_eq!(
        zero_suffix.upper_bound_with_depth(0, &UsedSet::new(), &PartialDeck::default()),
        0
    );
}

#[test]
fn search_dfs_matches_bruteforce_for_best_deck() {
    let cards = [
        TestCard {
            char_id: 0,
            attr: 0,
            unit_mask: 1,
            game_id: 300,
            power: 100,
            skill: SkillSlot {
                skill_type: 0,
                value: 0,
            },
            base_bonus: 0,
            limited_bonus: 0,
            power_max: 100,
            skill_max: 0,
        },
        TestCard {
            char_id: 1,
            attr: 0,
            unit_mask: 1,
            game_id: 301,
            power: 200,
            skill: SkillSlot {
                skill_type: 0,
                value: 0,
            },
            base_bonus: 0,
            limited_bonus: 0,
            power_max: 200,
            skill_max: 0,
        },
        TestCard {
            char_id: 2,
            attr: 0,
            unit_mask: 1,
            game_id: 302,
            power: 300,
            skill: SkillSlot {
                skill_type: 0,
                value: 0,
            },
            base_bonus: 0,
            limited_bonus: 0,
            power_max: 300,
            skill_max: 0,
        },
        TestCard {
            char_id: 3,
            attr: 0,
            unit_mask: 1,
            game_id: 303,
            power: 400,
            skill: SkillSlot {
                skill_type: 0,
                value: 0,
            },
            base_bonus: 0,
            limited_bonus: 0,
            power_max: 400,
            skill_max: 0,
        },
        TestCard {
            char_id: 4,
            attr: 0,
            unit_mask: 1,
            game_id: 304,
            power: 500,
            skill: SkillSlot {
                skill_type: 0,
                value: 0,
            },
            base_bonus: 0,
            limited_bonus: 0,
            power_max: 500,
            skill_max: 0,
        },
        TestCard {
            char_id: 5,
            attr: 0,
            unit_mask: 1,
            game_id: 305,
            power: 50,
            skill: SkillSlot {
                skill_type: 0,
                value: 0,
            },
            base_bonus: 0,
            limited_bonus: 0,
            power_max: 50,
            skill_max: 0,
        },
    ];
    let pool = build_pool(&cards);
    let mut search_ctx = ctx(ScoreTarget::Power);
    search_ctx.leader_honor_bonus = vec![0; pool.count()];
    search_ctx.leader_limit_bonus = vec![0; pool.count()];
    search_ctx.skill_is_after_training = vec![false; pool.count()];
    search_ctx.trained_to_special_image = vec![false; pool.count()];
    let suffix = SuffixBound::build(&pool, &search_ctx);
    let params = SearchParams {
        top_k: 1,
        timeout_ms: 0,
    };

    let results = dfs_search(&pool, &search_ctx, &suffix, &params);
    let best = results.first().map(|result| result.score).unwrap_or(0);

    let mut brute = 0u64;
    let mut a = 0usize;
    while a < pool.count() {
        let c0 = crate::pool::CardIdx::new(a as u16);
        let mut b = a + 1;
        while b < pool.count() {
            let c1 = crate::pool::CardIdx::new(b as u16);
            let mut c = b + 1;
            while c < pool.count() {
                let c2 = crate::pool::CardIdx::new(c as u16);
                let mut d = c + 1;
                while d < pool.count() {
                    let c3 = crate::pool::CardIdx::new(d as u16);
                    let mut e = d + 1;
                    while e < pool.count() {
                        let c4 = crate::pool::CardIdx::new(e as u16);
                        let score = leaf_evaluate(&pool, &search_ctx, &[c0, c1, c2, c3, c4]);
                        if score > brute {
                            brute = score;
                        }
                        e += 1;
                    }
                    d += 1;
                }
                c += 1;
            }
            b += 1;
        }
        a += 1;
    }

    assert_eq!(best, brute);
}

#[test]
fn search_dfs_score_noevent_does_not_break_before_higher_skill_same_power_state() {
    let cards = [
        TestCard {
            char_id: 0,
            attr: 0,
            unit_mask: 1,
            game_id: 500,
            power: 100,
            skill: SkillSlot {
                skill_type: 0,
                value: 10,
            },
            base_bonus: 0,
            limited_bonus: 0,
            power_max: 100,
            skill_max: 10,
        },
        TestCard {
            char_id: 0,
            attr: 0,
            unit_mask: 1,
            game_id: 500,
            power: 100,
            skill: SkillSlot {
                skill_type: 0,
                value: 20,
            },
            base_bonus: 0,
            limited_bonus: 0,
            power_max: 100,
            skill_max: 20,
        },
        TestCard {
            char_id: 1,
            attr: 0,
            unit_mask: 1,
            game_id: 501,
            power: 100,
            skill: SkillSlot {
                skill_type: 0,
                value: 0,
            },
            base_bonus: 0,
            limited_bonus: 0,
            power_max: 100,
            skill_max: 0,
        },
        TestCard {
            char_id: 2,
            attr: 0,
            unit_mask: 1,
            game_id: 502,
            power: 100,
            skill: SkillSlot {
                skill_type: 0,
                value: 0,
            },
            base_bonus: 0,
            limited_bonus: 0,
            power_max: 100,
            skill_max: 0,
        },
        TestCard {
            char_id: 3,
            attr: 0,
            unit_mask: 1,
            game_id: 503,
            power: 100,
            skill: SkillSlot {
                skill_type: 0,
                value: 0,
            },
            base_bonus: 0,
            limited_bonus: 0,
            power_max: 100,
            skill_max: 0,
        },
        TestCard {
            char_id: 4,
            attr: 0,
            unit_mask: 1,
            game_id: 504,
            power: 100,
            skill: SkillSlot {
                skill_type: 0,
                value: 0,
            },
            base_bonus: 0,
            limited_bonus: 0,
            power_max: 100,
            skill_max: 0,
        },
    ];
    let pool = build_pool(&cards);
    let mut search_ctx = ctx(ScoreTarget::Score);
    search_ctx.live_type = LiveType::Multi;
    search_ctx.skill_scores[1] = [10.0; 6];
    search_ctx.leader_honor_bonus = vec![0; pool.count()];
    search_ctx.leader_limit_bonus = vec![0; pool.count()];
    search_ctx.skill_is_after_training = vec![false; pool.count()];
    search_ctx.trained_to_special_image = vec![false; pool.count()];
    let suffix = SuffixBound::build(&pool, &search_ctx);
    let params = SearchParams {
        top_k: 1,
        timeout_ms: 0,
    };

    let results = dfs_search(&pool, &search_ctx, &suffix, &params);
    let best = results.first().map(|result| result.score).unwrap_or(0);

    let lower = leaf_evaluate(
        &pool,
        &search_ctx,
        &[
            crate::pool::CardIdx::new(0),
            crate::pool::CardIdx::new(2),
            crate::pool::CardIdx::new(3),
            crate::pool::CardIdx::new(4),
            crate::pool::CardIdx::new(5),
        ],
    );
    let higher = leaf_evaluate(
        &pool,
        &search_ctx,
        &[
            crate::pool::CardIdx::new(1),
            crate::pool::CardIdx::new(2),
            crate::pool::CardIdx::new(3),
            crate::pool::CardIdx::new(4),
            crate::pool::CardIdx::new(5),
        ],
    );

    assert!(higher > lower);
    assert_eq!(best, higher);
}

#[test]
fn search_dfs_score_noevent_matches_bruteforce_with_monotonic_break() {
    let cards = [
        TestCard {
            char_id: 0,
            attr: 0,
            unit_mask: 1,
            game_id: 600,
            power: 220,
            skill: SkillSlot {
                skill_type: 0,
                value: 30,
            },
            base_bonus: 0,
            limited_bonus: 0,
            power_max: 220,
            skill_max: 30,
        },
        TestCard {
            char_id: 1,
            attr: 0,
            unit_mask: 1,
            game_id: 601,
            power: 210,
            skill: SkillSlot {
                skill_type: 0,
                value: 28,
            },
            base_bonus: 0,
            limited_bonus: 0,
            power_max: 210,
            skill_max: 28,
        },
        TestCard {
            char_id: 2,
            attr: 0,
            unit_mask: 1,
            game_id: 602,
            power: 205,
            skill: SkillSlot {
                skill_type: 0,
                value: 25,
            },
            base_bonus: 0,
            limited_bonus: 0,
            power_max: 205,
            skill_max: 25,
        },
        TestCard {
            char_id: 3,
            attr: 0,
            unit_mask: 1,
            game_id: 603,
            power: 190,
            skill: SkillSlot {
                skill_type: 0,
                value: 24,
            },
            base_bonus: 0,
            limited_bonus: 0,
            power_max: 190,
            skill_max: 24,
        },
        TestCard {
            char_id: 4,
            attr: 0,
            unit_mask: 1,
            game_id: 604,
            power: 180,
            skill: SkillSlot {
                skill_type: 0,
                value: 22,
            },
            base_bonus: 0,
            limited_bonus: 0,
            power_max: 180,
            skill_max: 22,
        },
        TestCard {
            char_id: 5,
            attr: 0,
            unit_mask: 1,
            game_id: 605,
            power: 80,
            skill: SkillSlot {
                skill_type: 0,
                value: 3,
            },
            base_bonus: 0,
            limited_bonus: 0,
            power_max: 80,
            skill_max: 3,
        },
        TestCard {
            char_id: 6,
            attr: 0,
            unit_mask: 1,
            game_id: 606,
            power: 70,
            skill: SkillSlot {
                skill_type: 0,
                value: 2,
            },
            base_bonus: 0,
            limited_bonus: 0,
            power_max: 70,
            skill_max: 2,
        },
    ];
    let pool = build_pool(&cards);
    let mut search_ctx = ready_ctx(&pool, ScoreTarget::Score);
    search_ctx.live_type = LiveType::Multi;
    search_ctx.skill_scores[1] = [10.0; 6];
    let suffix = SuffixBound::build(&pool, &search_ctx);
    let params = SearchParams {
        top_k: 1,
        timeout_ms: 0,
    };
    let seed = warm_start::warm_start_best(&pool, &search_ctx);
    let (results, stats) = dfs::dfs_search_instrumented(&pool, &search_ctx, &suffix, &params, seed);
    let best = results.first().map(|result| result.score).unwrap_or(0);

    assert_eq!(best, brute_force_best(&pool, &search_ctx));
    let _ = stats;
}

#[test]
fn search_dfs_bonus_noevent_matches_bruteforce_with_suffix_max_break() {
    let cards = [
        TestCard {
            char_id: 0,
            attr: 0,
            unit_mask: 1,
            game_id: 520,
            power: 100,
            skill: SkillSlot {
                skill_type: 0,
                value: 0,
            },
            base_bonus: 10,
            limited_bonus: 0,
            power_max: 100,
            skill_max: 0,
        },
        TestCard {
            char_id: 1,
            attr: 1,
            unit_mask: 1,
            game_id: 521,
            power: 120,
            skill: SkillSlot {
                skill_type: 0,
                value: 0,
            },
            base_bonus: 5,
            limited_bonus: 0,
            power_max: 120,
            skill_max: 0,
        },
        TestCard {
            char_id: 2,
            attr: 2,
            unit_mask: 1,
            game_id: 522,
            power: 90,
            skill: SkillSlot {
                skill_type: 0,
                value: 0,
            },
            base_bonus: 40,
            limited_bonus: 0,
            power_max: 90,
            skill_max: 0,
        },
        TestCard {
            char_id: 3,
            attr: 3,
            unit_mask: 1,
            game_id: 523,
            power: 110,
            skill: SkillSlot {
                skill_type: 0,
                value: 0,
            },
            base_bonus: 20,
            limited_bonus: 0,
            power_max: 110,
            skill_max: 0,
        },
        TestCard {
            char_id: 4,
            attr: 4,
            unit_mask: 1,
            game_id: 524,
            power: 95,
            skill: SkillSlot {
                skill_type: 0,
                value: 0,
            },
            base_bonus: 35,
            limited_bonus: 0,
            power_max: 95,
            skill_max: 0,
        },
        TestCard {
            char_id: 5,
            attr: 0,
            unit_mask: 1,
            game_id: 525,
            power: 105,
            skill: SkillSlot {
                skill_type: 0,
                value: 0,
            },
            base_bonus: 15,
            limited_bonus: 0,
            power_max: 105,
            skill_max: 0,
        },
    ];
    let pool = build_pool(&cards);
    let search_ctx = ctx(ScoreTarget::Score);
    let suffix = SuffixBound::build(&pool, &search_ctx);
    let params = SearchParams {
        top_k: 1,
        timeout_ms: 0,
    };

    let best = dfs_search(&pool, &search_ctx, &suffix, &params)
        .first()
        .map(|result| result.score)
        .unwrap_or(0);

    assert_eq!(best, brute_force_best(&pool, &search_ctx));
}

#[test]
fn search_dfs_mysekai_matches_bruteforce_with_suffix_max_break() {
    let cards = [
        TestCard {
            char_id: 0,
            attr: 0,
            unit_mask: 1,
            game_id: 540,
            power: 200,
            skill: SkillSlot {
                skill_type: 0,
                value: 0,
            },
            base_bonus: 5,
            limited_bonus: 0,
            power_max: 200,
            skill_max: 0,
        },
        TestCard {
            char_id: 1,
            attr: 1,
            unit_mask: 1,
            game_id: 541,
            power: 250,
            skill: SkillSlot {
                skill_type: 0,
                value: 0,
            },
            base_bonus: 10,
            limited_bonus: 0,
            power_max: 250,
            skill_max: 0,
        },
        TestCard {
            char_id: 2,
            attr: 2,
            unit_mask: 1,
            game_id: 542,
            power: 240,
            skill: SkillSlot {
                skill_type: 0,
                value: 0,
            },
            base_bonus: 15,
            limited_bonus: 0,
            power_max: 240,
            skill_max: 0,
        },
        TestCard {
            char_id: 3,
            attr: 3,
            unit_mask: 1,
            game_id: 543,
            power: 180,
            skill: SkillSlot {
                skill_type: 0,
                value: 0,
            },
            base_bonus: 40,
            limited_bonus: 0,
            power_max: 180,
            skill_max: 0,
        },
        TestCard {
            char_id: 4,
            attr: 4,
            unit_mask: 1,
            game_id: 544,
            power: 210,
            skill: SkillSlot {
                skill_type: 0,
                value: 0,
            },
            base_bonus: 30,
            limited_bonus: 0,
            power_max: 210,
            skill_max: 0,
        },
        TestCard {
            char_id: 5,
            attr: 0,
            unit_mask: 1,
            game_id: 545,
            power: 260,
            skill: SkillSlot {
                skill_type: 0,
                value: 0,
            },
            base_bonus: 0,
            limited_bonus: 0,
            power_max: 260,
            skill_max: 0,
        },
    ];
    let pool = build_pool(&cards);
    let search_ctx = ctx(ScoreTarget::Mysekai);
    let suffix = SuffixBound::build(&pool, &search_ctx);
    let params = SearchParams {
        top_k: 1,
        timeout_ms: 0,
    };

    let best = dfs_search(&pool, &search_ctx, &suffix, &params)
        .first()
        .map(|result| result.score)
        .unwrap_or(0);

    assert_eq!(best, brute_force_best(&pool, &search_ctx));
}

#[test]
fn search_bonus_targets_matches_single_pass_bruteforce_buckets() {
    let cards = [
        TestCard {
            char_id: 0,
            attr: 0,
            unit_mask: 1,
            game_id: 600,
            power: 100,
            skill: SkillSlot::default(),
            base_bonus: 10,
            limited_bonus: 0,
            power_max: 100,
            skill_max: 0,
        },
        TestCard {
            char_id: 1,
            attr: 1,
            unit_mask: 1,
            game_id: 601,
            power: 110,
            skill: SkillSlot::default(),
            base_bonus: 20,
            limited_bonus: 0,
            power_max: 110,
            skill_max: 0,
        },
        TestCard {
            char_id: 2,
            attr: 2,
            unit_mask: 1,
            game_id: 602,
            power: 120,
            skill: SkillSlot::default(),
            base_bonus: 30,
            limited_bonus: 0,
            power_max: 120,
            skill_max: 0,
        },
        TestCard {
            char_id: 3,
            attr: 3,
            unit_mask: 1,
            game_id: 603,
            power: 130,
            skill: SkillSlot::default(),
            base_bonus: 40,
            limited_bonus: 0,
            power_max: 130,
            skill_max: 0,
        },
        TestCard {
            char_id: 4,
            attr: 4,
            unit_mask: 1,
            game_id: 604,
            power: 140,
            skill: SkillSlot::default(),
            base_bonus: 50,
            limited_bonus: 0,
            power_max: 140,
            skill_max: 0,
        },
        TestCard {
            char_id: 5,
            attr: 0,
            unit_mask: 1,
            game_id: 605,
            power: 150,
            skill: SkillSlot::default(),
            base_bonus: 60,
            limited_bonus: 0,
            power_max: 150,
            skill_max: 0,
        },
        TestCard {
            char_id: 6,
            attr: 1,
            unit_mask: 1,
            game_id: 606,
            power: 160,
            skill: SkillSlot::default(),
            base_bonus: 70,
            limited_bonus: 0,
            power_max: 160,
            skill_max: 0,
        },
    ];
    let pool = build_pool(&cards);
    let mut search_ctx = ready_ctx(&pool, ScoreTarget::Bonus);
    search_ctx.event_type = Some(crate::types::EventType::Marathon);
    let params = SearchParams {
        top_k: 2,
        timeout_ms: 0,
    };
    let targets = [150, 250];

    let default_actual = search(&pool, &search_ctx, &params);
    let (default_expected, _) = brute_force_search(&pool, &search_ctx, &params);
    assert_eq!(default_actual, default_expected);

    let actual = search_bonus_targets(&pool, &search_ctx, &params, &targets).0;
    let brute_params = SearchParams {
        top_k: 100,
        timeout_ms: 0,
    };
    let (all, _) = brute_force_search(&pool, &search_ctx, &brute_params);
    let expected = targets
        .iter()
        .rev()
        .flat_map(|target| {
            all.iter()
                .filter(move |result| (result.score >> 32) == (*target as u64 * 2))
                .take(params.top_k)
                .copied()
        })
        .collect::<Vec<_>>();

    assert_eq!(actual, expected);
}

#[test]
fn search_dfs_core_matches_bruteforce_for_three_cards_choose_two() {
    let cards = [
        TestCard {
            char_id: 0,
            attr: 0,
            unit_mask: 1,
            game_id: 360,
            power: 100,
            skill: SkillSlot {
                skill_type: 0,
                value: 0,
            },
            base_bonus: 0,
            limited_bonus: 0,
            power_max: 100,
            skill_max: 0,
        },
        TestCard {
            char_id: 1,
            attr: 0,
            unit_mask: 1,
            game_id: 361,
            power: 200,
            skill: SkillSlot {
                skill_type: 0,
                value: 0,
            },
            base_bonus: 0,
            limited_bonus: 0,
            power_max: 200,
            skill_max: 0,
        },
        TestCard {
            char_id: 2,
            attr: 0,
            unit_mask: 1,
            game_id: 362,
            power: 300,
            skill: SkillSlot {
                skill_type: 0,
                value: 0,
            },
            base_bonus: 0,
            limited_bonus: 0,
            power_max: 300,
            skill_max: 0,
        },
    ];
    let pool = build_pool(&cards);
    let power_ctx = ctx(ScoreTarget::Power);
    let suffix = SuffixBound::build(&pool, &power_ctx);
    let results = dfs::dfs_search_power_len_for_test(&pool, &suffix, 2, 1, &power_ctx);
    let best = results.first().map(|result| result.score).unwrap_or(0);

    let mut brute = 0u64;
    let mut left = 0usize;
    while left < pool.count() {
        let mut right = left + 1;
        while right < pool.count() {
            let score = pool.power_max(crate::pool::CardIdx::new(left as u16)) as u64
                + pool.power_max(crate::pool::CardIdx::new(right as u16)) as u64;
            if score > brute {
                brute = score;
            }
            right += 1;
        }
        left += 1;
    }

    assert_eq!(best, brute);
}

#[test]
fn search_warm_start_returns_non_zero_incumbent() {
    let pool = build_pool(&five_unique_cards());
    assert!(warm_start(&pool, &ctx(ScoreTarget::Power)) > 0);
}

#[test]
fn search_final_chapter_auto_leader_small_pool_returns_result() {
    let mut cards = five_unique_cards().to_vec();
    cards.push(TestCard {
        char_id: 5,
        attr: 0,
        unit_mask: 1,
        game_id: 105,
        power: 450,
        skill: SkillSlot {
            skill_type: 0,
            value: 45,
        },
        base_bonus: 10,
        limited_bonus: 0,
        power_max: 450,
        skill_max: 45,
    });
    let pool = build_pool(&cards);
    let mut search_ctx = ready_ctx(&pool, ScoreTarget::Score);
    search_ctx.is_final_chapter = true;
    search_ctx.live_type = LiveType::Multi;
    search_ctx.live_skill_order = LiveSkillOrder::Average;
    search_ctx.best_skill_as_leader = false;
    let params = SearchParams {
        top_k: 1,
        timeout_ms: 0,
    };

    let results = search(&pool, &search_ctx, &params);
    assert_eq!(results.len(), 1);
    assert!(!search_ctx.has_fixed_leader());
}

#[test]
fn search_dominance_preserves_best_score() {
    let cards = [
        TestCard {
            char_id: 0,
            attr: 0,
            unit_mask: 1,
            game_id: 400,
            power: 300,
            skill: SkillSlot {
                skill_type: 0,
                value: 30,
            },
            base_bonus: 10,
            limited_bonus: 0,
            power_max: 300,
            skill_max: 30,
        },
        TestCard {
            char_id: 0,
            attr: 0,
            unit_mask: 1,
            game_id: 401,
            power: 200,
            skill: SkillSlot {
                skill_type: 0,
                value: 20,
            },
            base_bonus: 5,
            limited_bonus: 0,
            power_max: 200,
            skill_max: 20,
        },
        TestCard {
            char_id: 1,
            attr: 0,
            unit_mask: 1,
            game_id: 402,
            power: 250,
            skill: SkillSlot {
                skill_type: 0,
                value: 25,
            },
            base_bonus: 0,
            limited_bonus: 0,
            power_max: 250,
            skill_max: 25,
        },
        TestCard {
            char_id: 2,
            attr: 0,
            unit_mask: 1,
            game_id: 403,
            power: 240,
            skill: SkillSlot {
                skill_type: 0,
                value: 24,
            },
            base_bonus: 0,
            limited_bonus: 0,
            power_max: 240,
            skill_max: 24,
        },
        TestCard {
            char_id: 3,
            attr: 0,
            unit_mask: 1,
            game_id: 404,
            power: 230,
            skill: SkillSlot {
                skill_type: 0,
                value: 23,
            },
            base_bonus: 0,
            limited_bonus: 0,
            power_max: 230,
            skill_max: 23,
        },
        TestCard {
            char_id: 4,
            attr: 0,
            unit_mask: 1,
            game_id: 405,
            power: 220,
            skill: SkillSlot {
                skill_type: 0,
                value: 22,
            },
            base_bonus: 0,
            limited_bonus: 0,
            power_max: 220,
            skill_max: 22,
        },
    ];
    let pool = build_pool(&cards);
    let mut search_ctx = ctx(ScoreTarget::Power);
    search_ctx.leader_honor_bonus = vec![0; pool.count()];
    search_ctx.leader_limit_bonus = vec![0; pool.count()];
    search_ctx.skill_is_after_training = vec![false; pool.count()];
    search_ctx.trained_to_special_image = vec![false; pool.count()];
    let suffix = SuffixBound::build(&pool, &search_ctx);
    let params = SearchParams {
        top_k: 1,
        timeout_ms: 0,
    };
    let before = dfs_search(&pool, &search_ctx, &suffix, &params)
        .first()
        .map(|result| result.score)
        .unwrap_or(0);

    let dominance = eliminate_dominated(&pool, &search_ctx);
    let compacted_suffix = SuffixBound::build(&dominance.pool, &dominance.ctx);
    let after = dfs_search(&dominance.pool, &dominance.ctx, &compacted_suffix, &params)
        .first()
        .map(|result| result.score)
        .unwrap_or(0);

    assert_eq!(before, after);
}

fn dominance_pair_card(game_id: u16, char_id: u8, power: u32) -> TestCard {
    TestCard {
        char_id,
        attr: 0,
        unit_mask: 1,
        game_id,
        power,
        skill: SkillSlot {
            skill_type: 0,
            value: 10,
        },
        base_bonus: 0,
        limited_bonus: 0,
        power_max: power,
        skill_max: 10,
    }
}

fn assert_results_match_bruteforce(pool: &CardPool, results: &[DeckResult], brute: &[DeckResult]) {
    assert_eq!(results.len(), brute.len(), "result length differs");
    for (rank, (exact, brute)) in results.iter().zip(brute.iter()).enumerate() {
        assert_eq!(exact.score, brute.score, "score differs at rank {rank}");
        assert_eq!(
            exact.game_card_set_key(pool),
            brute.game_card_set_key(pool),
            "card set differs at rank {rank}",
        );
    }
}

#[test]
fn search_top_k_recovers_dominated_alternatives() {
    // char0 的 B(295) 被 A(300) 支配裁掉，但 {B,1,2,3,4} 是全局第 2 名：
    // Top-K 替代展开应把它找回来，与暴力枚举一致（issue #2）。
    let cards = [
        dominance_pair_card(700, 0, 300),
        dominance_pair_card(701, 0, 295),
        dominance_pair_card(702, 1, 400),
        dominance_pair_card(703, 2, 410),
        dominance_pair_card(704, 3, 420),
        dominance_pair_card(705, 4, 430),
        dominance_pair_card(706, 5, 200),
    ];
    let pool = build_pool(&cards);
    let search_ctx = ready_ctx(&pool, ScoreTarget::Score);
    let params = SearchParams {
        top_k: 3,
        timeout_ms: 0,
    };

    // 前提：B 确实被支配裁掉。
    let dominance = eliminate_dominated(&pool, &search_ctx);
    assert_eq!(dominance.after, dominance.before - 1);

    let results = search(&pool, &search_ctx, &params);
    let (brute, _) = brute_force_search(&pool, &search_ctx, &params);
    assert_results_match_bruteforce(&pool, &results, &brute);
    assert!(
        results[1]
            .cards
            .iter()
            .any(|card| pool.game_id(*card) == 701),
        "rank 1 should contain the dominated card 701",
    );
}

#[test]
fn search_top_k_recovers_multi_slot_dominated_alternatives() {
    // 两个角色各有一张被支配卡，第 4 名 {B,D,...} 需要同时回换两个槽位。
    let cards = [
        dominance_pair_card(800, 0, 300),
        dominance_pair_card(801, 0, 296),
        dominance_pair_card(802, 1, 400),
        dominance_pair_card(803, 1, 395),
        dominance_pair_card(804, 2, 500),
        dominance_pair_card(805, 3, 510),
        dominance_pair_card(806, 4, 520),
    ];
    let pool = build_pool(&cards);
    let search_ctx = ready_ctx(&pool, ScoreTarget::Score);
    let params = SearchParams {
        top_k: 4,
        timeout_ms: 0,
    };

    let dominance = eliminate_dominated(&pool, &search_ctx);
    assert_eq!(dominance.after, dominance.before - 2);

    let results = search(&pool, &search_ctx, &params);
    let (brute, _) = brute_force_search(&pool, &search_ctx, &params);
    assert_results_match_bruteforce(&pool, &results, &brute);
    assert_eq!(results.len(), 4);
    let last_game_ids = results[3].cards.map(|card| pool.game_id(card)).to_vec();
    assert!(
        last_game_ids.contains(&801) && last_game_ids.contains(&803),
        "rank 3 should substitute both dominated cards, got {last_game_ids:?}",
    );
}

/// 终章 member 裁剪回归测试共用卡池（issue #7）：
/// char0 三张变体 —— X(900, 300, 队长称号加成 5) 第一轮靠称号幸存但 member 轮被
/// W(902, 305) 支配；Y(901, 295) 第一轮就被 X 支配。真实 Top-3 是 W/X/Y 各自成队。
fn final_chapter_member_cards() -> [TestCard; 7] {
    [
        dominance_pair_card(900, 0, 300),
        dominance_pair_card(901, 0, 295),
        dominance_pair_card(902, 0, 305),
        dominance_pair_card(903, 1, 400),
        dominance_pair_card(904, 2, 410),
        dominance_pair_card(905, 3, 420),
        dominance_pair_card(906, 5, 200),
    ]
}

fn final_chapter_ctx(pool: &CardPool) -> SearchContext {
    let mut search_ctx = ready_ctx(pool, ScoreTarget::Score);
    search_ctx.is_final_chapter = true;
    search_ctx.live_type = LiveType::Multi;
    search_ctx.live_skill_order = LiveSkillOrder::Average;
    search_ctx.best_skill_as_leader = false;
    search_ctx.leader_honor_bonus[0] = 5;
    search_ctx
}

#[test]
fn search_final_chapter_fixed_leader_top_k_recovers_member_pruned_alternatives() {
    let pool = build_pool(&final_chapter_member_cards());
    let mut search_ctx = final_chapter_ctx(&pool);
    search_ctx.fixed_character_ids = vec![5];
    let params = SearchParams {
        top_k: 3,
        timeout_ms: 0,
    };

    // 前提：Y 第一轮被裁；X 第一轮幸存、member 轮被 W 支配。
    let dominance = eliminate_dominated(&pool, &search_ctx);
    assert_eq!(dominance.after, dominance.before - 1);
    let member = dominance::compute_member_dominance(&dominance.pool, &dominance.ctx);
    assert!(!member.keep[0], "X should be member-dominated by W");
    assert_eq!(member.alternatives[1], vec![CardIdx::new(0)]);

    let results = search(&pool, &search_ctx, &params);
    let (brute, _) = brute_force_search(&pool, &search_ctx, &params);
    assert_results_match_bruteforce(&pool, &results, &brute);
    assert!(
        results[1]
            .cards
            .iter()
            .any(|card| pool.game_id(*card) == 900),
        "rank 1 should contain the member-pruned card 900",
    );
    assert!(
        results[2]
            .cards
            .iter()
            .any(|card| pool.game_id(*card) == 901),
        "rank 2 should contain the chained first-pass card 901",
    );
}

#[test]
fn search_final_chapter_auto_leader_top_k_recovers_member_pruned_alternatives() {
    let pool = build_pool(&final_chapter_member_cards());
    let search_ctx = final_chapter_ctx(&pool);
    let params = SearchParams {
        top_k: 3,
        timeout_ms: 0,
    };

    let results = search(&pool, &search_ctx, &params);
    let (brute, _) = brute_force_search(&pool, &search_ctx, &params);
    assert_results_match_bruteforce(&pool, &results, &brute);
}

#[test]
fn search_final_chapter_fixed_leader_card_top_k_recovers_member_pruned_alternatives() {
    // 固定队长卡 + 固定成员角色走 DFS 子路径（member 裁剪经 ctx 位图生效）。
    let pool = build_pool(&final_chapter_member_cards());
    let mut search_ctx = final_chapter_ctx(&pool);
    search_ctx.fixed_card_ids = vec![906];
    search_ctx.fixed_character_ids = vec![1];
    let params = SearchParams {
        top_k: 3,
        timeout_ms: 0,
    };

    let results = search(&pool, &search_ctx, &params);
    let (brute, _) = brute_force_search(&pool, &search_ctx, &params);
    assert_results_match_bruteforce(&pool, &results, &brute);
}

fn skill_card(game_id: u16, char_id: u8, power: u32, skill: u8) -> TestCard {
    TestCard {
        char_id,
        attr: 0,
        unit_mask: 1,
        game_id,
        power,
        skill: SkillSlot {
            skill_type: 0,
            value: skill,
        },
        base_bonus: 0,
        limited_bonus: 0,
        power_max: power,
        skill_max: skill,
    }
}

#[test]
fn search_final_chapter_top_k_restores_member_alternative_behind_leader_dedup() {
    // W(922) member 位支配 X(921)，且 W 是自身集合的最佳队长（技能 90）：
    // tracker 按集合去重后 W 只出现在队长槽，member 替代必须经队长轮换才能触发。
    // 集合 B={Y,X,fillers} 的最优排列是 Y 作队长（称号 6）、X 作队员。
    let cards = [
        skill_card(920, 2, 300, 10),
        skill_card(921, 1, 300, 10),
        skill_card(922, 1, 305, 90),
        skill_card(923, 3, 400, 10),
        skill_card(924, 4, 410, 10),
        skill_card(925, 5, 420, 10),
    ];
    let pool = build_pool(&cards);
    let mut search_ctx = final_chapter_ctx(&pool);
    search_ctx.leader_honor_bonus[0] = 6;
    search_ctx.leader_honor_bonus[1] = 5;
    search_ctx.event_type = Some(EventType::Marathon);
    search_ctx.skill_scores[1] = [10.0; 6];
    let params = SearchParams {
        top_k: 2,
        timeout_ms: 0,
    };

    // 前提：X 第一轮靠称号幸存，member 轮被 W 支配。
    let dominance = eliminate_dominated(&pool, &search_ctx);
    assert_eq!(dominance.after, dominance.before);
    let member = dominance::compute_member_dominance(&dominance.pool, &dominance.ctx);
    assert!(!member.keep[1], "X should be member-dominated by W");

    let results = search(&pool, &search_ctx, &params);
    assert_eq!(results.len(), 2);
    let expected_deck = [
        CardIdx::new(0),
        CardIdx::new(1),
        CardIdx::new(3),
        CardIdx::new(4),
        CardIdx::new(5),
    ];
    let expected = evaluate::leaf_evaluate_checked(&pool, &search_ctx, &expected_deck)
        .expect("expected arrangement must evaluate");
    let rank1_game_ids = {
        let mut ids = results[1].cards.map(|card| pool.game_id(card));
        ids.sort_unstable();
        ids
    };
    assert_eq!(rank1_game_ids, [920, 921, 923, 924, 925]);
    assert_eq!(
        results[1].score, expected,
        "rank 1 must carry the best arrangement score (Y leader, X member)",
    );
}

#[test]
fn search_final_chapter_world_bloom_support_penalty_keeps_member_candidates() {
    // A(900) 在队长支援表内（编入队伍损失 4.0 支援加成）：member 裁剪若不比较
    // 支援惩罚会裁掉 B(901)，而 A 卡组因支援损失跌出 Top-K，B 的真实次优卡组
    // 无从回换。支配加入支援惩罚维度后 B 保留在候选池，与暴力枚举一致。
    let cards = [
        skill_card(906, 7, 430, 10),
        skill_card(900, 0, 300, 10),
        skill_card(901, 0, 295, 10),
        skill_card(902, 1, 400, 10),
        skill_card(903, 2, 410, 10),
        skill_card(904, 3, 420, 10),
        skill_card(905, 4, 296, 10),
        skill_card(907, 5, 294, 10),
        skill_card(908, 6, 293, 10),
    ];
    let pool = build_pool(&cards);
    let mut search_ctx = final_chapter_ctx(&pool);
    search_ctx.is_world_bloom = true;
    search_ctx.event_type = Some(EventType::WorldBloom);
    search_ctx.fixed_character_ids = vec![7];
    search_ctx.leader_limit_bonus[2] = 1;
    let support = SupportDeck {
        cards: vec![(900, 5.0), (998, 1.0)],
        count: 1,
    };
    search_ctx.support_decks_by_character = vec![SupportDeck::default(); 8];
    search_ctx.support_decks_by_character[7] = support;
    let params = SearchParams {
        top_k: 3,
        timeout_ms: 0,
    };

    // 前提：B 第一轮靠当期加成幸存；member 轮因支援惩罚不得裁 B。
    let dominance = eliminate_dominated(&pool, &search_ctx);
    assert_eq!(dominance.after, dominance.before);
    let member = dominance::compute_member_dominance(&dominance.pool, &dominance.ctx);
    assert!(
        member.keep[2],
        "support-listed A must not member-dominate B",
    );

    let results = search(&pool, &search_ctx, &params);
    let (brute, _) = brute_force_search(&pool, &search_ctx, &params);
    assert_results_match_bruteforce(&pool, &results, &brute);
    assert!(
        results
            .iter()
            .any(|result| result.cards.iter().any(|card| pool.game_id(*card) == 901)),
        "top-k must contain a deck with B (901)",
    );
}

#[test]
fn search_power_top_k_dedups_cultivation_variants() {
    // 同一 game_id 的两个养成变体不得挤占每角色候选/DP 状态名额：
    // 否则 701 进不了候选，含它的真实次优卡组从 Top-K 消失（issue #24）。
    let cards = [
        dominance_pair_card(700, 1, 300),
        dominance_pair_card(700, 1, 300),
        dominance_pair_card(701, 1, 295),
        dominance_pair_card(702, 2, 400),
        dominance_pair_card(703, 3, 410),
        dominance_pair_card(704, 4, 420),
        dominance_pair_card(705, 5, 430),
    ];
    let pool = build_pool(&cards);
    let search_ctx = ready_ctx(&pool, ScoreTarget::Power);
    let params = SearchParams {
        top_k: 2,
        timeout_ms: 0,
    };

    let results = search(&pool, &search_ctx, &params);
    let (brute, _) = brute_force_search(&pool, &search_ctx, &params);
    assert_results_match_bruteforce(&pool, &results, &brute);
    assert!(
        results[1]
            .cards
            .iter()
            .any(|card| pool.game_id(*card) == 701),
        "rank 1 must contain 701, not a duplicate variant of 700",
    );
}

#[test]
fn search_world_bloom_support_penalty_blocks_first_pass_domination() {
    // 非终章 WL（issue #23）：A(900) 在支援表内，编入队伍损失 4.0 支援加成；
    // 第一轮支配若支援盲会裁掉 B(901)，而真实 Top-1 是 B 卡组。
    let cards = [
        skill_card(900, 0, 300, 10),
        skill_card(901, 0, 295, 10),
        skill_card(902, 1, 400, 10),
        skill_card(903, 2, 410, 10),
        skill_card(904, 3, 420, 10),
        skill_card(905, 4, 296, 10),
        skill_card(906, 5, 294, 10),
    ];
    let pool = build_pool(&cards);
    let mut search_ctx = ready_ctx(&pool, ScoreTarget::Score);
    search_ctx.is_world_bloom = true;
    search_ctx.event_type = Some(EventType::WorldBloom);
    search_ctx.live_type = LiveType::Multi;
    search_ctx.live_skill_order = LiveSkillOrder::Average;
    search_ctx.best_skill_as_leader = false;
    search_ctx.support_deck.cards = vec![(900, 5.0), (998, 1.0)];
    search_ctx.support_deck.count = 1;
    let params = SearchParams {
        top_k: 1,
        timeout_ms: 0,
    };

    // 前提：支援惩罚阻止 A 支配 B，第一轮不得裁任何卡。
    let dominance = eliminate_dominated(&pool, &search_ctx);
    assert_eq!(dominance.after, dominance.before);

    let results = search(&pool, &search_ctx, &params);
    let (brute, _) = brute_force_search(&pool, &search_ctx, &params);
    assert_results_match_bruteforce(&pool, &results, &brute);
    assert!(
        results[0]
            .cards
            .iter()
            .any(|card| pool.game_id(*card) == 901),
        "top-1 must contain B (901): the support-listed dominator forfeits its bonus in deck",
    );
}

#[test]
fn compute_member_dominance_protects_fixed_cards() {
    let cards = [
        dominance_pair_card(950, 0, 300),
        dominance_pair_card(951, 0, 295),
        dominance_pair_card(952, 1, 400),
    ];
    let pool = build_pool(&cards);
    let mut search_ctx = ready_ctx(&pool, ScoreTarget::Score);

    let member = dominance::compute_member_dominance(&pool, &search_ctx);
    assert!(!member.keep[1]);
    assert_eq!(member.alternatives[0], vec![CardIdx::new(1)]);

    search_ctx.fixed_card_ids = vec![951];
    let member = dominance::compute_member_dominance(&pool, &search_ctx);
    assert!(member.keep[1], "fixed cards must survive member pruning");
}

#[test]
fn search_top_k_dominated_alternatives_respect_fixed_slots() {
    // 固定卡槽位不参与回换：固定 game_id 800 时，被支配卡 801 不得顶掉它。
    let cards = [
        dominance_pair_card(800, 0, 300),
        dominance_pair_card(801, 0, 296),
        dominance_pair_card(802, 1, 400),
        dominance_pair_card(803, 2, 500),
        dominance_pair_card(804, 3, 510),
        dominance_pair_card(805, 4, 520),
        dominance_pair_card(806, 5, 210),
    ];
    let pool = build_pool(&cards);
    let mut search_ctx = ready_ctx(&pool, ScoreTarget::Score);
    search_ctx.fixed_card_ids = vec![800];
    let params = SearchParams {
        top_k: 3,
        timeout_ms: 0,
    };

    let results = search(&pool, &search_ctx, &params);
    let (brute, _) = brute_force_search(&pool, &search_ctx, &params);
    assert_results_match_bruteforce(&pool, &results, &brute);
    for result in &results {
        assert_eq!(pool.game_id(result.cards[0]), 800);
        assert!(result.cards.iter().all(|card| pool.game_id(*card) != 801));
    }
}

#[test]
fn challenge_all_searches_each_character_and_merges() {
    // 挑战 live 要求五张同角色。池里留着多个角色时（challenge_all），
    // search 必须逐角色搜索后归并，而不是在混角色池上做一次无约束搜索。
    let mut cards = Vec::new();
    for char_id in 0..3u8 {
        for slot in 0..6u16 {
            cards.push(skill_card(
                900 + char_id as u16 * 10 + slot,
                char_id,
                300 + u32::from(slot) * 7 + u32::from(char_id) * 3,
                10,
            ));
        }
    }
    let pool = build_pool(&cards);
    let mut search_ctx = ready_ctx(&pool, ScoreTarget::Score);
    search_ctx.enforce_char_uniqueness = false;
    let params = SearchParams {
        top_k: 5,
        timeout_ms: 0,
    };

    let results = search(&pool, &search_ctx, &params);
    assert!(!results.is_empty(), "challenge_all 必须给出结果");

    // 每个卡组都必须是同角色的合法挑战队伍。
    for result in &results {
        let leader_char = pool.char_id(result.cards[0]);
        assert!(
            result
                .cards
                .iter()
                .all(|card| pool.char_id(*card) == leader_char),
            "challenge 卡组不得跨角色",
        );
    }

    // 与逐角色搜索后归并的参考实现逐位一致。
    let suffix = SuffixBound::build(&pool, &search_ctx);
    let mut reference = Vec::new();
    for char_id in 0..3u8 {
        let (found, _) =
            challenge_search::search_character(&pool, &search_ctx, &suffix, &params, char_id);
        reference.extend(found);
    }
    reference.sort_unstable_by(deck_result_cmp);
    reference.truncate(params.top_k);
    assert_eq!(results, reference, "challenge_all 应等于逐角色归并");
}

#[test]
fn challenge_single_character_pool_keeps_direct_search() {
    // 池里只有一个角色（调用方已指定 challenge_live_character_id）时不走归并路径。
    let cards = (0..6u16)
        .map(|slot| skill_card(950 + slot, 1, 300 + u32::from(slot) * 5, 10))
        .collect::<Vec<_>>();
    let pool = build_pool(&cards);
    let mut search_ctx = ready_ctx(&pool, ScoreTarget::Score);
    search_ctx.enforce_char_uniqueness = false;
    let params = SearchParams {
        top_k: 3,
        timeout_ms: 0,
    };

    let results = search(&pool, &search_ctx, &params);
    let suffix = SuffixBound::build(&pool, &search_ctx);
    let (direct, _) = challenge_search::search(&pool, &search_ctx, &suffix, &params);
    assert_eq!(results, direct);
}

#[test]
fn challenge_live_power_and_skill_targets_search_same_character_decks() {
    // 回归：challenge live × target=power|skill 曾被 Power/Skill 通用路径截胡，
    // 该路径的递归无条件要求角色唯一，challenge 永远凑不齐五张同角色，
    // 静默返回空集。challenge 约束对所有 target 生效，必须走 challenge 搜索。
    for target in [ScoreTarget::Power, ScoreTarget::Skill] {
        let mut cards = Vec::new();
        for char_id in 0..2u8 {
            for slot in 0..6u16 {
                cards.push(skill_card(
                    970 + u16::from(char_id) * 10 + slot,
                    char_id,
                    300 + u32::from(slot) * 11 + u32::from(char_id) * 40,
                    u8::try_from(10 + slot).unwrap(),
                ));
            }
        }
        let pool = build_pool(&cards);
        let mut search_ctx = ready_ctx(&pool, target);
        search_ctx.enforce_char_uniqueness = false;
        let params = SearchParams {
            top_k: 3,
            timeout_ms: 0,
        };

        let results = search(&pool, &search_ctx, &params);
        assert!(!results.is_empty(), "challenge × {target:?} 必须给出结果");
        for result in &results {
            let leader_char = pool.char_id(result.cards[0]);
            assert!(
                result
                    .cards
                    .iter()
                    .all(|card| pool.char_id(*card) == leader_char),
                "challenge × {target:?} 卡组不得跨角色",
            );
        }

        // 与「逐角色挑战搜索 + 归并」的参考实现一致。
        let suffix = SuffixBound::build(&pool, &search_ctx);
        let mut reference = Vec::new();
        for char_id in 0..2u8 {
            let (found, _) =
                challenge_search::search_character(&pool, &search_ctx, &suffix, &params, char_id);
            reference.extend(found);
        }
        reference.sort_unstable_by(deck_result_cmp);
        reference.truncate(params.top_k);
        assert_eq!(
            results, reference,
            "challenge × {target:?} 应等于逐角色归并"
        );
    }
}

#[test]
fn mysekai_top_k_is_monotone_across_limits() {
    // mysekai 分值把总战力按 45k 一档量化，桶内大量并列。并列内的去留与
    // 次序曾被 tracker 插入序决定：limit=1 的第一名可以在 limit=3 缺席。
    // 规范次序 = 分值降序、总战力降序、队长 cardId 升序，对任意 limit 一致。
    let mut cards = Vec::new();
    for char_id in 0..5u8 {
        for variant in 0..2u16 {
            let power = 70_000 + u32::from(char_id) * 2_000 - u32::from(variant) * 1_000;
            cards.push(TestCard {
                char_id,
                attr: 0,
                unit_mask: 1,
                game_id: 1200 + u16::from(char_id) * 2 + variant,
                power,
                skill: SkillSlot::default(),
                base_bonus: 0,
                limited_bonus: 0,
                power_max: power,
                skill_max: 0,
            });
        }
    }
    let pool = build_pool(&cards);
    let search_ctx = ready_ctx(&pool, ScoreTarget::Mysekai);

    let run = |top_k: usize| {
        search(
            &pool,
            &search_ctx,
            &SearchParams {
                top_k,
                timeout_ms: 0,
            },
        )
    };
    let limits = [1usize, 2, 3, 5, 10];
    let by_limit: Vec<_> = limits.iter().copied().map(run).collect();

    // 所有 limit 的第一名都是同一个（战力最高的并列卡组）。站位序不参与比较。
    let first_key = by_limit[0][0].game_card_set_key(&pool);
    assert!(first_key.contains(&1208));
    for (index, results) in by_limit.iter().enumerate() {
        assert_eq!(
            results[0].game_card_set_key(&pool),
            first_key,
            "limit={} 的第一名偏离 top-1",
            limits[index]
        );
        // 结果集随 limit 单调增长。
        for (rank, result) in results.iter().enumerate() {
            assert!(
                by_limit[by_limit.len() - 1]
                    .iter()
                    .take(rank + 1)
                    .any(|top| top.game_card_set_key(&pool) == result.game_card_set_key(&pool)),
                "limit={} 第 {} 名不在 top-10 前缀里",
                limits[index],
                rank,
            );
        }
    }

    // 并列分值内按总战力降序输出。
    let top10 = &by_limit[by_limit.len() - 1];
    let mut prev_power = u32::MAX;
    for result in top10.iter().filter(|r| r.score == by_limit[0][0].score) {
        let power: u32 = result.cards.iter().map(|c| pool.power_max(*c)).sum();
        assert!(
            power <= prev_power,
            "并列分值内战力未按降序排列: {power} > {prev_power}"
        );
        prev_power = power;
    }
}
