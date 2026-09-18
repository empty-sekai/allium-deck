//! Independent ordered enumeration for the Specific-order counterexample.
use super::*;
use std::collections::{BTreeMap, BTreeSet};

fn ordered_exhaustive(pool: &CardPool, ctx: &SearchContext, top_k: usize) -> Vec<DeckResult> {
    fn visit(
        pool: &CardPool,
        ctx: &SearchContext,
        deck: &mut [CardIdx; 5],
        depth: usize,
        best: &mut BTreeMap<[u16; 5], DeckResult>,
    ) {
        if depth == 5 {
            let Some(score) = evaluate::leaf_evaluate_checked(pool, ctx, deck) else {
                return;
            };
            let candidate = DeckResult::new(*deck, score);
            let mut key = deck.map(|card| pool.game_id(card));
            key.sort_unstable();
            best.entry(key)
                .and_modify(|old| {
                    if score > old.score || (score == old.score && candidate.cards < old.cards) {
                        *old = candidate;
                    }
                })
                .or_insert(candidate);
            return;
        }
        for card in pool.indices() {
            if deck[..depth].iter().any(|&chosen| {
                pool.game_id(chosen) == pool.game_id(card)
                    || (ctx.enforce_char_uniqueness && pool.char_id(chosen) == pool.char_id(card))
            }) {
                continue;
            }
            if ctx
                .fixed_card_at(depth)
                .is_some_and(|id| id != pool.game_id(card))
                || ctx
                    .fixed_character_at(depth)
                    .is_some_and(|id| id != pool.char_id(card))
            {
                continue;
            }
            deck[depth] = card;
            visit(pool, ctx, deck, depth + 1, best);
        }
    }
    let mut best = BTreeMap::new();
    visit(pool, ctx, &mut [CardIdx::new(0); 5], 0, &mut best);
    let mut results: Vec<_> = best.into_values().collect();
    results.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.cards.cmp(&b.cards)));
    results.truncate(top_k);
    results
}

#[test]
#[ignore = "explicit diagnostic export before changing Specific-order semantics"]
fn audit_case7_ordered_feasible_set() {
    let pool = build_pool(&randomized_exact_cards(0xE7AC_0007, 12, 7));
    let mut context = ready_ctx(&pool, ScoreTarget::Score);
    context.live_type = LiveType::Solo;
    context.live_skill_order = LiveSkillOrder::Specific;
    context.specific_skill_order = Some([4, 2, 0, 3, 1]);
    context.skill_scores[0] = [0.21, 0.17, 0.13, 0.11, 0.07, 0.23];
    let params = SearchParams {
        top_k: 4,
        timeout_ms: 0,
    };
    let (production, stats) = search_instrumented(&pool, &context, &params);
    let (combination, _) = brute_force_search(&pool, &context, &params);
    let suffix = SuffixBound::build(&pool, &context);
    let unseeded = dfs_search(&pool, &context, &suffix, &params);
    let ordered = ordered_exhaustive(&pool, &context, 4);
    let raw_seeds = warm_start::warm_start_seeds(&pool, &context, 4);
    let mut results = serde_json::Map::new();
    for (name, rows) in [
        ("production", &production),
        ("independent_ordered_oracle", &combination),
        ("no_dominance_unseeded_dfs", &unseeded),
        ("ordered_exhaustive", &ordered),
        ("raw_warm_seeds", &raw_seeds),
    ] {
        results.insert(
            name.into(),
            serde_json::Value::Array(
                rows.iter()
                    .map(|row| {
                        let ids = row.cards.map(|c| pool.game_id(c));
                        let chars = row.cards.map(|c| pool.char_id(c));
                        serde_json::json!({"score":row.score,"dense_ids":row.cards.map(|c| c.raw()),
                "game_ids":ids,"character_ids":chars,
                "distinct_game_ids":ids.iter().collect::<BTreeSet<_>>().len()==5,
                "distinct_characters":chars.iter().collect::<BTreeSet<_>>().len()==5,
                "direct_leaf_score":evaluate::leaf_evaluate_checked(&pool,&context,&row.cards),
                "summary":format!("{:?}",summarize_deck(&pool,&context,&row.cards))})
                    })
                    .collect(),
            ),
        );
    }
    let report = serde_json::json!({"schema_version":1,"name":"case-7-score-solo-specific",
        "generator_seed":0xE7AC_0007u64,
        "cards":pool.indices().map(|c| serde_json::json!({"dense_id":c.raw(),"game_id":pool.game_id(c),
            "character_id":pool.char_id(c),"attribute":pool.attr(c),"unit_mask":pool.unit_mask_raw(c),
            "power_contexts":(0..8).map(|s|decode_u18(pool.power_values(c),pool.power_lut(c),s)).collect::<Vec<_>>(),
            "power_max":pool.power_max(c),"skill_type":pool.skill(c).skill_type,"skill_value":pool.skill(c).value,
            "skill_min":pool.skill_min(c),"skill_max":pool.skill_max(c),
            "base_bonus_x10":pool.event_bonus_exact(c).base_x10(),"limited_bonus_x10":pool.event_bonus_exact(c).limited_x10(),
            "dynamic_skill_metadata":null})).collect::<Vec<_>>(),
        "fixed_cards":context.fixed_card_ids,"fixed_characters":context.fixed_character_ids,
        "excluded_cards":[],"forced_leader":null,"objective":"Score","live":"Solo","event":null,
        "best_skill_as_leader":true,"live_skill_order":"Specific","specific_skill_order":context.specific_skill_order,
        "base_score":context.base_score,"skill_coefficients":context.skill_scores,
        "context_debug":format!("{context:#?}"),"production_stats":format!("{stats:?}"),"results":results});
    if let Some(path) = std::env::var_os("ALLIUM_CASE7_REPORT") {
        std::fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    }
    eprintln!("{}", serde_json::to_string_pretty(&report).unwrap());
    assert!(!stats.deadline_hit);
    assert!(ordered[0].score >= production[0].score);
}

#[test]
fn specific_order_fixture_preserves_the_true_ordered_optimum() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/case7_specific_order.json")).unwrap();
    let cards = fixture["cards"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| TestCard {
            char_id: c["character_id"].as_u64().unwrap() as u8,
            attr: c["attribute"].as_u64().unwrap() as u8,
            unit_mask: c["unit_mask"].as_u64().unwrap() as u8,
            game_id: c["game_id"].as_u64().unwrap() as u16,
            power: c["power_contexts"][0].as_u64().unwrap() as u32,
            power_max: c["power_max"].as_u64().unwrap() as u32,
            skill: SkillSlot {
                skill_type: c["skill_type"].as_u64().unwrap() as u8,
                value: c["skill_value"].as_u64().unwrap() as u8,
            },
            skill_max: c["skill_max"].as_u64().unwrap() as u8,
            base_bonus: (c["base_bonus_x10"].as_u64().unwrap() / 10) as u8,
            limited_bonus: (c["limited_bonus_x10"].as_u64().unwrap() / 10) as u8,
        })
        .collect::<Vec<_>>();
    let pool = build_pool(&cards);
    let mut context = ready_ctx(&pool, ScoreTarget::Score);
    context.live_type = LiveType::Solo;
    context.live_skill_order = LiveSkillOrder::Specific;
    context.specific_skill_order =
        serde_json::from_value(fixture["specific_skill_order"].clone()).unwrap();
    context.skill_scores = serde_json::from_value(fixture["skill_coefficients"].clone()).unwrap();
    let params = SearchParams {
        top_k: 4,
        timeout_ms: 0,
    };
    let expected = ordered_exhaustive(&pool, &context, 4);
    assert_eq!(expected[0].score, 288_690_521_835_152);
    for (label, configuration) in [
        ("all pruning", tuning::SearchTuning::default()),
        (
            "all score bounds disabled",
            tuning::SearchTuning {
                bounds: false,
                ..Default::default()
            },
        ),
        (
            "dominance disabled",
            tuning::SearchTuning {
                dominance: false,
                ..Default::default()
            },
        ),
        (
            "all bounds and dominance disabled",
            tuning::SearchTuning {
                bounds: false,
                dominance: false,
                ..Default::default()
            },
        ),
    ] {
        let (actual, stats) = tuning::with_tuning(configuration, || {
            search_instrumented(&pool, &context, &params)
        });
        assert!(!stats.deadline_hit);
        assert!(stats.visited_nodes > 0);
        if !configuration.bounds {
            assert_eq!(
                stats.ub_prunes + stats.correlated_prunes + stats.mono_break_prunes,
                0
            );
        }
        assert_property_results(&pool, &actual, &expected, label);
        assert_property_scores(&pool, &context, &actual, &expected, label);
        eprintln!(
            "CASE7_ABLATION {label}: score={} stats={stats:?}",
            actual[0].score
        );
    }
    let (actual, stats) = search_instrumented(&pool, &context, &params);
    assert!(!stats.deadline_hit);
    assert_property_results(&pool, &actual, &expected, "Specific ordered feasible set");
    assert_property_scores(
        &pool,
        &context,
        &actual,
        &expected,
        "Specific direct evaluation",
    );
    let (oracle, _) = ExactOracle::new(&pool, &context).search(&params);
    assert_property_results(&pool, &oracle, &expected, "shared oracle ordered coverage");
}
