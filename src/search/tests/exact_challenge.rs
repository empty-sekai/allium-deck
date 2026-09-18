//! exact challenge contracts.
use super::*;

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
fn challenge_search_keeps_zero_score_decks() {
    check_challenge_ranking(ScoreTarget::Skill, 0, false);
}

#[test]
fn challenge_search_preserves_equal_score_tie_breaks() {
    check_challenge_ranking(ScoreTarget::Skill, 10, false);
    // Non-power targets continue to ignore minimize.
    check_challenge_ranking(ScoreTarget::Skill, 10, true);
}

#[test]
fn challenge_search_minimizes_power() {
    check_challenge_ranking(ScoreTarget::Power, 10, true);
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
