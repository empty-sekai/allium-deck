//! exact mysekai contracts.
use super::*;

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

    let best = dfs_search_exact(&pool, &search_ctx, &suffix, &params)
        .first()
        .map(|result| result.score)
        .unwrap_or(0);

    assert_eq!(best, brute_force_best(&pool, &search_ctx));
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
        search_exact(
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
