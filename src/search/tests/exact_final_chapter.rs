//! exact final chapter contracts.
use super::*;

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

    let results = search_exact(&pool, &search_ctx, &params);
    assert_eq!(results.len(), 1);
    assert!(!search_ctx.has_fixed_leader());
}

#[test]
fn search_final_chapter_fixed_leader_top_k_recovers_member_pruned_alternatives() {
    // Preserve the original numeric counterexample and also exercise a
    // canonical-order-preserving deletion (W's public ID precedes X's).
    for canonical_prunable in [false, true] {
        let mut cards = final_chapter_member_cards();
        if canonical_prunable {
            cards[2].game_id = 899;
        }
        let pool = build_pool(&cards);
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
        assert_eq!(member.keep[0], !canonical_prunable);
        if canonical_prunable {
            assert_eq!(member.alternatives[1], vec![CardIdx::new(0)]);
        }

        let results = search_exact(&pool, &search_ctx, &params);
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
}

#[test]
fn search_final_chapter_auto_leader_top_k_recovers_member_pruned_alternatives() {
    let pool = build_pool(&final_chapter_member_cards());
    let search_ctx = final_chapter_ctx(&pool);
    let params = SearchParams {
        top_k: 3,
        timeout_ms: 0,
    };

    let results = search_exact(&pool, &search_ctx, &params);
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

    let results = search_exact(&pool, &search_ctx, &params);
    let (brute, _) = brute_force_search(&pool, &search_ctx, &params);
    assert_results_match_bruteforce(&pool, &results, &brute);
}

#[test]
fn search_final_chapter_top_k_restores_member_alternative_behind_leader_dedup() {
    // W 在数值上 member 支配 X；同时验证原始 ID 和满足 canonical 替换的 ID。
    // W 是自身集合的最佳队长（技能 90）：
    // tracker 按集合去重后 W 只出现在队长槽，member 替代必须经队长轮换才能触发。
    // 集合 B={Y,X,fillers} 的最优排列是 Y 作队长（称号 6）、X 作队员。
    for canonical_prunable in [false, true] {
        let cards = [
            skill_card(920, 2, 300, 10),
            skill_card(921, 1, 300, 10),
            skill_card(if canonical_prunable { 919 } else { 922 }, 1, 305, 90),
            skill_card(923, 3, 400, 10),
            skill_card(924, 4, 410, 10),
            skill_card(925, 5, 420, 10),
        ];
        let pool = build_pool(&cards);
        let mut search_ctx = final_chapter_ctx(&pool);
        search_ctx.leader_honor_bonus_x10[0] = (6) * 10;
        search_ctx.leader_honor_bonus_x10[1] = (5) * 10;
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
        assert_eq!(member.keep[1], !canonical_prunable);

        let results = search_exact(&pool, &search_ctx, &params);
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
    search_ctx.leader_limit_bonus_x10[2] = 10;
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

    let results = search_exact(&pool, &search_ctx, &params);
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
fn final_chapter_auto_leader_must_not_truncate_support_safe_variant_set() {
    // Five characters each expose four mutually non-dominating variants. The
    // first three have higher leader-key power and are therefore the only ones
    // retained by the historical 3-per-character truncation. All of those 15
    // high-key cards are also in every Final Chapter support deck. The fourth
    // variant of every character is slightly weaker but absent from support.
    // Selecting the five fourth variants preserves the full support sum, making
    // that card set globally optimal. Because every card in that set is rank 4
    // for its own character, no retained leader can generate the set; post-search
    // leader rotation cannot recover a card set that was never visited.
    let mut cards = Vec::new();
    let mut top_three_ids = Vec::new();
    for character in 1u8..=5 {
        for variant in 0u8..4 {
            let game_id = 1100 + character as u16 * 10 + variant as u16;
            let power = match variant {
                0 => 1500,
                1 => 1480,
                2 => 1460,
                _ => 1400,
            };
            let mut card = skill_card(game_id, character, power, 20);
            card.attr = 0;
            // Disjoint unit masks prevent same-attribute leader dominance from
            // collapsing the four variants before the truncation under test.
            card.unit_mask = 1u8 << variant;
            card.base_bonus = 10;
            cards.push(card);
            if variant < 3 {
                top_three_ids.push(game_id);
            }
        }
    }
    // Add four very high-key decoy leaders from a sixth character. They are all
    // support-listed, so they are intentionally poor final choices, but they
    // occupy the global beam's leader prefix and prevent any rank-4 target card
    // from entering the heuristic seed.
    for variant in 0u8..4 {
        let game_id = 1160 + variant as u16;
        let mut card = skill_card(game_id, 6, 3000 - variant as u32 * 10, 20);
        card.attr = 0;
        card.unit_mask = 1u8 << variant;
        card.base_bonus = 10;
        cards.push(card);
        top_three_ids.push(game_id);
    }
    let pool = build_pool(&cards);
    let mut search_ctx = final_chapter_ctx(&pool);
    search_ctx.is_world_bloom = true;
    search_ctx.event_type = Some(EventType::WorldBloom);
    search_ctx.skill_scores[1] = [0.2; 6];
    search_ctx.support_decks_by_character = vec![SupportDeck::default(); 27];
    let support = SupportDeck {
        cards: top_three_ids
            .iter()
            .copied()
            .map(|id| (id, 100.0))
            .collect(),
        count: top_three_ids.len() as u8,
    };
    for character in 1usize..=5 {
        search_ctx.support_decks_by_character[character] = support.clone();
    }
    let params = SearchParams {
        top_k: 1,
        timeout_ms: 0,
    };

    let oracle = final_chapter_auto_oracle(&pool, &search_ctx, 1);
    assert_eq!(oracle.len(), 1);
    let mut oracle_ids = oracle[0].cards.map(|card| pool.game_id(card));
    oracle_ids.sort_unstable();
    let expected = [1113, 1123, 1133, 1143, 1153];
    assert_eq!(
        oracle_ids, expected,
        "oracle should prefer the five support-safe rank-4 variants"
    );

    let got = search_exact(&pool, &search_ctx, &params);
    assert_results_match_bruteforce(&pool, &got, &oracle);
}

#[test]
fn exact_final_chapter_auto_matches_exhaustive_leader_oracle_randomized() {
    for case in 0..48u64 {
        let cards = randomized_exact_cards(0xF1A1_0000 + case, 12, 6);
        let pool = build_pool(&cards);
        let mut final_ctx = final_chapter_ctx(&pool);
        final_ctx.is_world_bloom = true;
        final_ctx.event_type = Some(EventType::WorldBloom);
        final_ctx.skill_scores[1] = [0.19, 0.17, 0.13, 0.11, 0.07, 0.23];
        final_ctx.diff_attr_bonus = [0, 0, 11, 29, 59, 101];
        final_ctx.support_decks_by_character = vec![SupportDeck::default(); 27];
        for character in 1usize..=6 {
            final_ctx.support_decks_by_character[character] =
                support_deck_for_property(&pool, character + case as usize);
        }
        for dense in 0..pool.count() {
            final_ctx.leader_honor_bonus_x10[dense] =
                (((dense * 3 + case as usize) % 9) as u16) * 10;
            final_ctx.leader_limit_bonus_x10[dense] =
                (((dense * 5 + case as usize) % 7) as u16) * 10;
        }
        let params = SearchParams {
            top_k: 3,
            timeout_ms: 0,
        };
        let got = search_exact(&pool, &final_ctx, &params);
        let expected = final_chapter_auto_oracle(&pool, &final_ctx, params.top_k);
        assert_property_results(&pool, &got, &expected, &format!("case {case} final-auto"));
    }
}

#[test]
fn final_seed_traversal_never_changes_unlimited_canonical_results() {
    for case in 0..16u64 {
        let cards = randomized_exact_cards(0x5EED_0000 + case, 12, 6);
        let pool = build_pool(&cards);
        let mut final_ctx = final_chapter_ctx(&pool);
        final_ctx.is_world_bloom = true;
        final_ctx.event_type = Some(EventType::WorldBloom);
        final_ctx.diff_attr_bonus = [0, 0, 11, 29, 59, 101];
        final_ctx.support_decks_by_character = vec![SupportDeck::default(); 27];
        for character in 1usize..=6 {
            final_ctx.support_decks_by_character[character] =
                support_deck_for_property(&pool, character + case as usize);
        }
        let params = SearchParams {
            top_k: 4,
            timeout_ms: 0,
        };
        let seeded = search_exact(&pool, &final_ctx, &params);
        let unseeded = crate::search::tuning::with_tuning(
            crate::search::tuning::SearchTuning {
                final_seeds: false,
                ..Default::default()
            },
            || search_exact(&pool, &final_ctx, &params),
        );
        assert_property_results(
            &pool,
            &seeded,
            &unseeded,
            &format!("Final seed equivalence case {case}"),
        );
        assert_property_scores(
            &pool,
            &final_ctx,
            &seeded,
            &unseeded,
            &format!("Final seed objective case {case}"),
        );
    }
}

#[test]
fn final_chapter_mysekai_matches_exhaustive_oracle_on_power_ties() {
    // Deck powers straddle several 45k steps of the MySekai value, so many
    // decks tie on it and resolved power decides, with and without a forced
    // leader character.
    for case in 0..16u64 {
        let mut cards = randomized_exact_cards(0x3E5C_0000 + case, 12, 6);
        for card in &mut cards {
            card.power = 40_000 + card.power * 10;
            card.power_max = card.power;
        }
        let pool = build_pool(&cards);
        let mut ctx = ready_ctx(&pool, ScoreTarget::Mysekai);
        ctx.is_final_chapter = true;
        ctx.live_type = LiveType::Mysekai;
        ctx.best_skill_as_leader = false;
        ctx.is_world_bloom = true;
        ctx.event_type = Some(EventType::WorldBloom);
        ctx.support_decks_by_character = vec![SupportDeck::default(); 27];
        for character in 1usize..=6 {
            ctx.support_decks_by_character[character] =
                support_deck_for_property(&pool, character + case as usize);
        }
        for dense in 0..pool.count() {
            ctx.leader_honor_bonus_x10[dense] = (((dense * 3 + case as usize) % 9) as u16) * 10;
            ctx.leader_limit_bonus_x10[dense] = (((dense * 5 + case as usize) % 7) as u16) * 10;
        }
        if case % 2 == 1 {
            ctx.forced_leader_character_id = Some((case % 6) as u8 + 1);
        }
        for top_k in [1, 3, 30] {
            let params = SearchParams {
                top_k,
                timeout_ms: 0,
            };
            let got = search_exact(&pool, &ctx, &params);
            let expected = final_chapter_auto_oracle(&pool, &ctx, top_k);
            let label = format!("case {case} K={top_k}");
            assert_property_results(&pool, &got, &expected, &label);
            assert_property_scores(&pool, &ctx, &got, &expected, &label);
        }
    }
}

#[test]
fn final_chapter_specific_order_keeps_members_seated_by_public_id() {
    // Twins of one character, attribute and skill: the first has more power,
    // the second a larger public id. Members are seated by public id, so the
    // weaker twin can take a better-weighted seat under a specific order.
    let card = |char_id: u8, game_id: u16, power: u32, skill: u8| TestCard {
        char_id,
        attr: char_id % 3,
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
    };
    let cards = [
        card(1, 100, 1_200, 30),
        card(2, 40, 3_000, 40),
        card(3, 60, 1_100, 30),
        card(4, 70, 1_100, 30),
        card(5, 50, 6_510, 80),
        card(5, 90, 6_500, 80),
    ];
    let pool = build_pool(&cards);
    let mut observed = false;
    for live_type in [LiveType::Solo, LiveType::Auto] {
        for specific in [
            [0, 1, 3, 4, 2],
            [2, 0, 1, 3, 4],
            [0, 1, 2, 3, 4],
            [4, 2, 0, 3, 1],
        ] {
            let mut ctx = final_chapter_ctx(&pool);
            ctx.is_world_bloom = true;
            ctx.event_type = Some(EventType::WorldBloom);
            ctx.live_type = live_type;
            ctx.live_skill_order = LiveSkillOrder::Specific;
            ctx.specific_skill_order = Some(specific);
            ctx.skill_scores = [
                [0.3, 0.05, 0.2, 0.1, 0.9, 0.4],
                [0.19, 0.17, 0.13, 0.11, 0.07, 0.23],
                [0.25, 0.05, 0.15, 0.1, 0.8, 0.35],
            ];
            ctx.support_decks_by_character = vec![SupportDeck::default(); 27];
            let strong = [0, 1, 2, 3, 4].map(CardIdx::new);
            let weak = [0, 1, 2, 3, 5].map(CardIdx::new);
            observed |= evaluate::leaf_evaluate_checked(&pool, &ctx, &weak)
                > evaluate::leaf_evaluate_checked(&pool, &ctx, &strong);
            for top_k in [1, 2, 8] {
                let params = SearchParams {
                    top_k,
                    timeout_ms: 0,
                };
                let got = search_exact(&pool, &ctx, &params);
                let expected = final_chapter_auto_oracle(&pool, &ctx, top_k);
                assert_property_results(
                    &pool,
                    &got,
                    &expected,
                    &format!("live {live_type:?} order {specific:?} K={top_k}"),
                );
            }
        }
    }
    assert!(
        observed,
        "a weaker twin must outscore the stronger one somewhere"
    );
}

#[test]
fn final_chapter_all_skill_orders_match_explicit_oracle() {
    for case in 0..12u64 {
        let cards = randomized_exact_cards(0xF0AD_1000 + case, 11, 6);
        let pool = build_pool(&cards);
        for (live_type, order, specific, best_as_leader) in
            [LiveType::Multi, LiveType::Solo, LiveType::Auto]
                .into_iter()
                .flat_map(|live_type| {
                    [
                        (LiveSkillOrder::Best, None),
                        (LiveSkillOrder::Worst, None),
                        (LiveSkillOrder::Average, None),
                        (LiveSkillOrder::Specific, Some([4, 2, 0, 3, 1])),
                        (LiveSkillOrder::Specific, Some([0, 1, 3, 4, 2])),
                        (LiveSkillOrder::Specific, Some([2, 0, 1, 3, 4])),
                    ]
                    .into_iter()
                    .flat_map(move |(order, specific)| {
                        [false, true].map(|best| (live_type, order, specific, best))
                    })
                })
        {
            let mut search_ctx = final_chapter_ctx(&pool);
            search_ctx.is_world_bloom = true;
            search_ctx.event_type = Some(EventType::WorldBloom);
            search_ctx.live_type = live_type;
            search_ctx.live_skill_order = order;
            search_ctx.specific_skill_order = specific;
            search_ctx.best_skill_as_leader = best_as_leader;
            // Distinct slot rates make the member order observable under a
            // specific order.
            search_ctx.skill_scores = [
                [0.21, 0.09, 0.17, 0.05, 0.13, 0.25],
                [0.19, 0.17, 0.13, 0.11, 0.07, 0.23],
                [0.12, 0.2, 0.06, 0.16, 0.1, 0.22],
            ];
            search_ctx.diff_attr_bonus = [0, 0, 11, 29, 59, 101];
            search_ctx.support_decks_by_character = vec![SupportDeck::default(); 27];
            for character in 1usize..=6 {
                search_ctx.support_decks_by_character[character] =
                    support_deck_for_property(&pool, character + case as usize);
            }

            let params = SearchParams {
                top_k: 3,
                timeout_ms: 0,
            };
            let got = search_exact(&pool, &search_ctx, &params);
            let expected = final_chapter_auto_oracle(&pool, &search_ctx, params.top_k);
            assert_property_scores(
                &pool,
                &search_ctx,
                &got,
                &expected,
                &format!(
                    "final-auto case {case} live {live_type:?} order {order:?} {specific:?} best-as-leader {best_as_leader}"
                ),
            );

            let mut fixed = search_ctx.clone();
            fixed.fixed_character_ids = vec![pool.char_id(CardIdx::new(0))];
            let got = search_exact(&pool, &fixed, &params);
            let (expected, _) = brute_force_search(&pool, &fixed, &params);
            assert_property_scores(
                &pool,
                &fixed,
                &got,
                &expected,
                &format!(
                    "final-fixed case {case} live {live_type:?} order {order:?} {specific:?} best-as-leader {best_as_leader}"
                ),
            );
        }
    }
}
