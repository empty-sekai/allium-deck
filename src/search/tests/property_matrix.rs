//! property matrix contracts.
use super::*;

#[test]
fn exact_power_skill_and_world_bloom_match_bruteforce_randomized() {
    // This is a deterministic property matrix, not a benchmark.  Every case is
    // small enough for the independent unpruned combination oracle.
    for case in 0..64u64 {
        let cards = randomized_exact_cards(0xA11E_0000 + case, 13, 8);
        let pool = build_pool(&cards);
        let params = SearchParams {
            top_k: 3,
            timeout_ms: 0,
        };

        // Constrained Power maximize exercises the new exact B&B rather than the
        // unconstrained 49-scenario DP.
        let mut power_ctx = ready_ctx(&pool, ScoreTarget::Power);
        power_ctx.fixed_card_ids = vec![pool.game_id(CardIdx::new(0))];
        power_ctx.fixed_character_ids = vec![2];
        let got = search(&pool, &power_ctx, &params);
        let (expected, _) = brute_force_search(&pool, &power_ctx, &params);
        assert_property_results(&pool, &got, &expected, &format!("case {case} power-max"));

        let mut minimize_ctx = power_ctx.clone();
        minimize_ctx.minimize = true;
        let got = search(&pool, &minimize_ctx, &params);
        let (expected, _) = brute_force_search(&pool, &minimize_ctx, &params);
        assert_property_results(&pool, &got, &expected, &format!("case {case} power-min"));

        let mut skill_ctx = ready_ctx(&pool, ScoreTarget::Skill);
        skill_ctx.fixed_character_ids = vec![1];
        let got = search(&pool, &skill_ctx, &params);
        let (expected, _) = brute_force_search(&pool, &skill_ctx, &params);
        assert_property_results(&pool, &got, &expected, &format!("case {case} skill"));

        // Ordinary World Bloom exercises the dense-suffix attr/character
        // matching bound plus support exclusion and event score ordering.
        let mut wl_ctx = ready_ctx(&pool, ScoreTarget::Score);
        wl_ctx.live_type = LiveType::Multi;
        wl_ctx.event_type = Some(EventType::WorldBloom);
        wl_ctx.is_world_bloom = true;
        wl_ctx.skill_scores[1] = [0.17, 0.13, 0.11, 0.07, 0.05, 0.19];
        wl_ctx.diff_attr_bonus = [0, 0, 7, 19, 41, 83];
        wl_ctx.support_deck = support_deck_for_property(&pool, case as usize);
        let got = search(&pool, &wl_ctx, &params);
        let (expected, _) = brute_force_search(&pool, &wl_ctx, &params);
        if got.iter().map(|result| result.score).collect::<Vec<_>>()
            != expected
                .iter()
                .map(|result| result.score)
                .collect::<Vec<_>>()
        {
            let suffix = SuffixBound::build(&pool, &wl_ctx);
            let direct = dfs_search(&pool, &wl_ctx, &suffix, &params);
            eprintln!("WL diagnostic case={case}");
            for (name, results) in [
                ("search", &got),
                ("direct-dfs", &direct),
                ("brute", &expected),
            ] {
                eprintln!("  {name}:");
                for result in results.iter().take(3) {
                    let ids = result.cards.map(|card| pool.game_id(card));
                    let rescored = evaluate::leaf_evaluate_checked(&pool, &wl_ctx, &result.cards);
                    eprintln!(
                        "    score={} rescored={rescored:?} cards={ids:?}",
                        result.score
                    );
                }
            }
        }
        assert_property_results(&pool, &got, &expected, &format!("case {case} world-bloom"));
    }
}

#[test]
fn exact_power_skill_special_skills_and_variants_match_bruteforce() {
    let pool = build_special_exact_pool();
    let params = SearchParams {
        top_k: 5,
        timeout_ms: 0,
    };

    for reference in [
        SkillReferenceStrategy::Max,
        SkillReferenceStrategy::Min,
        SkillReferenceStrategy::Average,
    ] {
        let mut skill_ctx = ready_ctx(&pool, ScoreTarget::Skill);
        skill_ctx.skill_reference_strategy = reference;
        skill_ctx.fixed_card_ids = vec![pool.game_id(CardIdx::new(0))];
        skill_ctx.fixed_character_ids = vec![2];
        let got = search(&pool, &skill_ctx, &params);
        let (expected, _) = brute_force_search(&pool, &skill_ctx, &params);
        assert_property_scores(
            &pool,
            &skill_ctx,
            &got,
            &expected,
            &format!("special skill {reference:?}"),
        );
    }

    let mut power_max = ready_ctx(&pool, ScoreTarget::Power);
    power_max.fixed_card_ids = vec![pool.game_id(CardIdx::new(1))];
    power_max.fixed_character_ids = vec![3];
    let got = search(&pool, &power_max, &params);
    let (expected, _) = brute_force_search(&pool, &power_max, &params);
    assert_property_scores(&pool, &power_max, &got, &expected, "special power-max");

    let mut power_min = power_max.clone();
    power_min.minimize = true;
    let got = search(&pool, &power_min, &params);
    let (expected, _) = brute_force_search(&pool, &power_min, &params);
    assert_property_scores(&pool, &power_min, &got, &expected, "special power-min");
}

#[test]
#[ignore = "controlled exhaustive all-scene property matrix; pin to one CPU"]
fn long_exact_all_scene_property_matrix() {
    let mut checks = 0u64;

    fn check(pool: &CardPool, ctx: &SearchContext, params: &SearchParams, label: &str) {
        let (got, stats) = search_instrumented(pool, ctx, params);
        assert!(!stats.deadline_hit, "{label}: unexpected timeout");
        let (expected, _) = brute_force_search(pool, ctx, params);
        if got.iter().map(|r| r.score).collect::<Vec<_>>()
            != expected.iter().map(|r| r.score).collect::<Vec<_>>()
        {
            eprintln!("{label}: production/oracle mismatch stats={stats:?}");
            for (name, rows) in [("production", &got), ("brute", &expected)] {
                eprintln!("  {name}:");
                for result in rows.iter().take(params.top_k.max(1)) {
                    let ids = result.cards.map(|card| pool.game_id(card));
                    let chars = result.cards.map(|card| pool.char_id(card));
                    let attrs = result.cards.map(|card| pool.attr(card));
                    eprintln!(
                        "    score={} ids={ids:?} chars={chars:?} attrs={attrs:?} rescored={:?}",
                        result.score,
                        evaluate::leaf_evaluate_checked(pool, ctx, &result.cards),
                    );
                }
            }
        }
        assert_property_scores(pool, ctx, &got, &expected, label);
    }

    for case in 0..256u64 {
        let cards = randomized_exact_cards(0xE7AC_0000 + case, 12, 7);
        let pool = build_pool(&cards);
        let params = SearchParams {
            top_k: 1 + (case as usize % 4),
            timeout_ms: 0,
        };

        let order = match case % 4 {
            0 => LiveSkillOrder::Best,
            1 => LiveSkillOrder::Worst,
            2 => LiveSkillOrder::Average,
            _ => LiveSkillOrder::Specific,
        };

        let mut solo = ready_ctx(&pool, ScoreTarget::Score);
        solo.live_type = LiveType::Solo;
        solo.live_skill_order = order;
        solo.specific_skill_order = (order == LiveSkillOrder::Specific).then_some([4, 2, 0, 3, 1]);
        solo.skill_scores[0] = [0.21, 0.17, 0.13, 0.11, 0.07, 0.23];
        check(&pool, &solo, &params, &format!("case {case} score-solo"));
        checks += 1;

        let mut auto = solo.clone();
        auto.live_type = LiveType::Auto;
        auto.base_score_auto = 0.83;
        auto.skill_scores[2] = [0.19, 0.13, 0.11, 0.07, 0.05, 0.17];
        check(&pool, &auto, &params, &format!("case {case} score-auto"));
        checks += 1;

        let mut multi = solo.clone();
        multi.live_type = LiveType::Multi;
        multi.event_type = Some(EventType::Marathon);
        multi.skill_scores[1] = [0.23, 0.17, 0.13, 0.11, 0.07, 0.29];
        multi.multi_teammate_power = Some(182_000 + case as i32 % 9000);
        multi.multi_teammate_score_up = Some(85 + case as i32 % 25);
        check(
            &pool,
            &multi,
            &params,
            &format!("case {case} score-multi-event"),
        );
        checks += 1;

        let mut cheerful = multi.clone();
        cheerful.live_type = LiveType::Cheerful;
        cheerful.event_type = Some(EventType::CheerfulCarnival);
        cheerful.life = 500 + (case as i32 % 501);
        cheerful.other_score = 600_000 + (case as i32 * 7919 % 400_000);
        check(
            &pool,
            &cheerful,
            &params,
            &format!("case {case} score-cheerful"),
        );
        checks += 1;

        let mut mysekai = ready_ctx(&pool, ScoreTarget::Mysekai);
        mysekai.live_type = LiveType::Mysekai;
        check(&pool, &mysekai, &params, &format!("case {case} mysekai"));
        checks += 1;

        let power = ready_ctx(&pool, ScoreTarget::Power);
        check(&pool, &power, &params, &format!("case {case} power-dp"));
        checks += 1;

        let mut power_fixed = power.clone();
        power_fixed.fixed_card_ids = vec![pool.game_id(CardIdx::new(0))];
        power_fixed.fixed_character_ids = vec![2];
        check(
            &pool,
            &power_fixed,
            &params,
            &format!("case {case} power-fixed"),
        );
        checks += 1;

        let mut power_min = power_fixed.clone();
        power_min.minimize = true;
        check(
            &pool,
            &power_min,
            &params,
            &format!("case {case} power-min"),
        );
        checks += 1;

        let mut skill = ready_ctx(&pool, ScoreTarget::Skill);
        skill.fixed_character_ids = vec![1];
        skill.skill_reference_strategy = match case % 3 {
            0 => SkillReferenceStrategy::Max,
            1 => SkillReferenceStrategy::Min,
            _ => SkillReferenceStrategy::Average,
        };
        check(&pool, &skill, &params, &format!("case {case} skill"));
        checks += 1;

        let mut wl = multi.clone();
        wl.event_type = Some(EventType::WorldBloom);
        wl.is_world_bloom = true;
        wl.diff_attr_bonus = [0, 0, 9, 27, 57, 103];
        wl.support_deck = support_deck_for_property(&pool, case as usize);
        check(&pool, &wl, &params, &format!("case {case} world-bloom"));
        checks += 1;

        let mut final_fixed = final_chapter_ctx(&pool);
        final_fixed.is_world_bloom = true;
        final_fixed.event_type = Some(EventType::WorldBloom);
        final_fixed.skill_scores[1] = [0.19, 0.17, 0.13, 0.11, 0.07, 0.23];
        final_fixed.diff_attr_bonus = [0, 0, 11, 31, 61, 107];
        final_fixed.fixed_character_ids = vec![1];
        final_fixed.support_decks_by_character = vec![SupportDeck::default(); 27];
        for character in 1usize..=7 {
            final_fixed.support_decks_by_character[character] =
                support_deck_for_property(&pool, character + case as usize);
        }
        check(
            &pool,
            &final_fixed,
            &params,
            &format!("case {case} final-fixed"),
        );
        checks += 1;

        let mut final_auto = final_fixed.clone();
        final_auto.fixed_character_ids.clear();
        let (got, stats) = search_instrumented(&pool, &final_auto, &params);
        assert!(
            !stats.deadline_hit,
            "case {case} final-auto: unexpected timeout"
        );
        let expected = final_chapter_auto_oracle(&pool, &final_auto, params.top_k);
        assert_property_scores(
            &pool,
            &final_auto,
            &got,
            &expected,
            &format!("case {case} final-auto"),
        );
        checks += 1;

        // Exact bonus buckets: enumerate every public card set, choose up to two
        // reachable bonus tiers, and compare the dedicated per-tier search.
        let mut bonus = ready_ctx(&pool, ScoreTarget::Bonus);
        bonus.event_type = Some(EventType::Marathon);
        let all_params = SearchParams {
            top_k: 1000,
            timeout_ms: 0,
        };
        let (all_bonus, _) = brute_force_search(&pool, &bonus, &all_params);
        let mut targets = Vec::new();
        for result in &all_bonus {
            let encoded = result.score >> 32;
            if encoded % 2 == 0 {
                let target = (encoded / 2) as i32;
                if !targets.contains(&target) {
                    targets.push(target);
                    if targets.len() == 2 {
                        break;
                    }
                }
            }
        }
        if !targets.is_empty() {
            let (got, stats) = search_bonus_targets(&pool, &bonus, &params, &targets);
            assert!(
                !stats.deadline_hit,
                "case {case} bonus-tiers: unexpected timeout"
            );
            // BonusBucketTracker returns target buckets in descending target
            // order. targets is sampled from brute-force Bonus results, which
            // are already ranked by the encoded bonus descending.
            let expected = targets
                .iter()
                .flat_map(|target| {
                    all_bonus
                        .iter()
                        .filter(move |result| (result.score >> 32) == (*target as u64 * 2))
                        .take(params.top_k)
                        .copied()
                })
                .collect::<Vec<_>>();
            if got.iter().map(|r| r.score).collect::<Vec<_>>()
                != expected.iter().map(|r| r.score).collect::<Vec<_>>()
            {
                eprintln!("case {case} bonus-tiers mismatch targets={targets:?} stats={stats:?}");
                for (name, rows) in [("production", &got), ("brute", &expected)] {
                    eprintln!("  {name}:");
                    for result in rows.iter().take(8) {
                        eprintln!(
                            "    bonus_x2={} live={} ids={:?}",
                            result.score >> 32,
                            result.score as u32,
                            result.cards.map(|card| pool.game_id(card)),
                        );
                    }
                }
            }
            assert_property_scores(
                &pool,
                &bonus,
                &got,
                &expected,
                &format!("case {case} bonus-tiers"),
            );
            checks += 1;
        }

        // Challenge paths use a one-character pool so the independent generic
        // brute-force enumerator has the same five-of-one-character feasible set.
        let challenge_cards = randomized_exact_cards(0xC1A1_0000 + case, 9, 1);
        let challenge_pool = build_pool(&challenge_cards);
        for live in [LiveType::Challenge, LiveType::ChallengeAuto] {
            let mut challenge = ready_ctx(&challenge_pool, ScoreTarget::Score);
            challenge.enforce_char_uniqueness = false;
            challenge.live_type = live;
            challenge.skill_scores[0] = [0.17; 6];
            challenge.skill_scores[2] = [0.13; 6];
            check(
                &challenge_pool,
                &challenge,
                &params,
                &format!("case {case} challenge-{live:?}"),
            );
            checks += 1;
        }
    }

    eprintln!("ALL_SCENE_EXACT_MATRIX checks={checks} cases=256");
    assert!(
        checks >= 3_500,
        "matrix should exercise thousands of exact comparisons"
    );
}
