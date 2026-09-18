//! Shared fixtures and helpers for independently named exactness contracts.
mod constraints;
mod exact_bonus;
mod exact_challenge;
mod exact_dominance;
mod exact_final_chapter;
mod exact_mysekai;
mod exact_power;
mod exact_score;
mod exact_world_bloom;
mod performance;
mod prepared_search;
mod property_bounds;
mod property_matrix;

mod case7_audit;
mod dominance_contract;
mod fractional_bonus;
mod role_constraints;

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

/// Exhaustive challenge oracle: no search bounds, one result per game-card set.
fn exhaustive_challenge_results(pool: &CardPool, search_ctx: &SearchContext) -> Vec<DeckResult> {
    let mut results = Vec::new();
    let card = |dense: usize| CardIdx::new(dense as u16);
    for a in 0..pool.count() {
        for b in a + 1..pool.count() {
            for c in b + 1..pool.count() {
                for d in c + 1..pool.count() {
                    for e in d + 1..pool.count() {
                        let mut deck = [card(a), card(b), card(c), card(d), card(e)];
                        if deck
                            .iter()
                            .any(|&card| pool.char_id(card) != pool.char_id(deck[0]))
                        {
                            continue;
                        }
                        let mut game_ids = deck.map(|card| pool.game_id(card));
                        game_ids.sort_unstable();
                        if game_ids.windows(2).any(|pair| pair[0] == pair[1]) {
                            continue;
                        }
                        // Challenge slots follow fixed-card groups, then the
                        // public search's descending power/skill candidate order.
                        deck.sort_unstable_by_key(|&card| {
                            (
                                search_ctx
                                    .fixed_card_ids
                                    .iter()
                                    .position(|&id| id == pool.game_id(card))
                                    .unwrap_or(usize::MAX),
                                std::cmp::Reverse(pool.power_max(card)),
                                std::cmp::Reverse(pool.skill_max(card)),
                                pool.game_id(card),
                            )
                        });
                        if !deck_matches_fixed_slots(pool, search_ctx, &deck) {
                            continue;
                        }
                        if let Some(score) =
                            evaluate::leaf_evaluate_checked(pool, search_ctx, &deck)
                        {
                            results.push(DeckResult::new(deck, score));
                        }
                    }
                }
            }
        }
    }
    let minimize = search_ctx.minimize && matches!(search_ctx.target, ScoreTarget::Power);
    results.sort_unstable_by(|a, b| {
        let order = deck_result_cmp(a, b);
        if minimize { order.reverse() } else { order }
    });
    let mut seen = std::collections::HashSet::new();
    results.retain(|result| seen.insert(result.game_card_set_key(pool)));
    results
}

fn check_challenge_ranking(target: ScoreTarget, skill: u8, minimize: bool) {
    for character_count in [1, 2] {
        let mut cards = Vec::new();
        for char_id in 1..=character_count {
            for variant in 0..8u16 {
                // The last entry is a second cultivation of the first game card.
                let game_id = 100 + u16::from(char_id) * 10 + variant % 7;
                cards.push(skill_card(
                    game_id,
                    char_id,
                    100 + u32::from(char_id) * 100 + u32::from(variant) * 3,
                    skill,
                ));
            }
        }
        for shuffled in [false, true] {
            if shuffled {
                cards.reverse();
                cards.rotate_left(3);
            }
            let pool = build_pool(&cards);
            for fixed in [false, true] {
                let mut search_ctx = ready_ctx(&pool, target);
                search_ctx.enforce_char_uniqueness = false;
                search_ctx.live_type = LiveType::Challenge;
                search_ctx.minimize = minimize;
                if fixed {
                    search_ctx.fixed_card_ids = vec![111];
                }
                let expected = exhaustive_challenge_results(&pool, &search_ctx);
                assert!(!expected.is_empty());
                for top_k in [0, 1, 3, 64] {
                    let params = SearchParams {
                        top_k,
                        timeout_ms: 0,
                    };
                    let actual = search(&pool, &search_ctx, &params);
                    assert_eq!(
                        actual,
                        expected[..top_k.min(expected.len())],
                        "target={target:?}, skill={skill}, minimize={minimize}, characters={character_count}, shuffled={shuffled}, fixed={fixed}, top_k={top_k}"
                    );
                }
            }
        }
    }
}

/// Exact auto-leader oracle for Final Chapter tests: enumerate every leader card
/// and every four-member combination, keeping the best arrangement for each
/// distinct five-card set. Unlike `brute_force_search`, this explicitly explores
/// all possible leader placements and is therefore a valid oracle for auto leader.
fn final_chapter_auto_oracle(
    pool: &CardPool,
    ctx: &SearchContext,
    top_k: usize,
) -> Vec<DeckResult> {
    let mut best_by_set: Vec<([u16; DECK_SIZE], DeckResult)> = Vec::new();
    for leader in pool.indices() {
        let leader_char = pool.char_id(leader);
        let mut deck = [leader; DECK_SIZE];
        fn rec(
            pool: &CardPool,
            ctx: &SearchContext,
            leader: CardIdx,
            leader_char: u8,
            start: usize,
            depth: usize,
            used_chars: u32,
            deck: &mut [CardIdx; DECK_SIZE],
            best_by_set: &mut Vec<([u16; DECK_SIZE], DeckResult)>,
        ) {
            if depth == DECK_SIZE {
                let Some(score) = evaluate::leaf_evaluate_checked(pool, ctx, deck) else {
                    return;
                };
                let candidate = DeckResult::new(*deck, score);
                let mut key = candidate.cards.map(|card| pool.game_id(card));
                key.sort_unstable();
                if let Some((_, existing)) = best_by_set.iter_mut().find(|(seen, _)| *seen == key) {
                    if candidate.score > existing.score
                        || (candidate.score == existing.score && candidate.cards < existing.cards)
                    {
                        *existing = candidate;
                    }
                } else {
                    best_by_set.push((key, candidate));
                }
                return;
            }
            let need = DECK_SIZE - depth;
            let mut dense = start;
            while dense < pool.count() {
                if pool.count() - dense < need {
                    break;
                }
                let card = CardIdx::new(dense as u16);
                dense += 1;
                if card == leader {
                    continue;
                }
                let char_id = pool.char_id(card);
                if char_id == leader_char || used_chars & (1u32 << char_id) != 0 {
                    continue;
                }
                deck[depth] = card;
                rec(
                    pool,
                    ctx,
                    leader,
                    leader_char,
                    dense,
                    depth + 1,
                    used_chars | (1u32 << char_id),
                    deck,
                    best_by_set,
                );
            }
        }
        rec(
            pool,
            ctx,
            leader,
            leader_char,
            0,
            1,
            1u32 << leader_char,
            &mut deck,
            &mut best_by_set,
        );
    }
    let mut results = best_by_set
        .into_iter()
        .map(|(_, result)| result)
        .collect::<Vec<_>>();
    results.sort_unstable_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| left.cards.cmp(&right.cards))
    });
    results.truncate(top_k);
    results
}

fn assert_property_scores(
    pool: &CardPool,
    ctx: &SearchContext,
    results: &[DeckResult],
    expected: &[DeckResult],
    label: &str,
) {
    assert_eq!(
        results.len(),
        expected.len(),
        "{label}: result length differs"
    );
    for (rank, (actual, oracle)) in results.iter().zip(expected.iter()).enumerate() {
        assert_eq!(
            actual.score, oracle.score,
            "{label}: score differs at rank {rank}"
        );
        assert_eq!(
            evaluate::leaf_evaluate_checked(pool, ctx, &actual.cards),
            Some(actual.score),
            "{label}: returned deck at rank {rank} must exactly re-evaluate",
        );
    }
}

fn assert_property_results(
    pool: &CardPool,
    results: &[DeckResult],
    expected: &[DeckResult],
    label: &str,
) {
    assert_eq!(
        results.len(),
        expected.len(),
        "{label}: result length differs"
    );
    for (rank, (actual, oracle)) in results.iter().zip(expected.iter()).enumerate() {
        assert_eq!(
            actual.score, oracle.score,
            "{label}: score differs at rank {rank}"
        );
        assert_eq!(
            actual.game_card_set_key(pool),
            oracle.game_card_set_key(pool),
            "{label}: card set differs at rank {rank}",
        );
    }
}

#[derive(Clone, Copy)]
struct ExactLcg(u64);

impl ExactLcg {
    fn next(&mut self) -> u32 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 32) as u32
    }

    fn range(&mut self, lo: u32, hi: u32) -> u32 {
        debug_assert!(lo < hi);
        lo + self.next() % (hi - lo)
    }
}

fn randomized_exact_cards(seed: u64, count: usize, characters: u8) -> Vec<TestCard> {
    let mut rng = ExactLcg(seed);
    (0..count)
        .map(|idx| {
            let char_id = (idx as u8 % characters) + 1;
            let power = 700 + rng.range(0, 1900) + idx as u32 * 3;
            let skill_value = 20 + rng.range(0, 100) as u8;
            let attr = rng.range(0, 5) as u8;
            let unit = rng.range(0, 6) as u8;
            TestCard {
                char_id,
                attr,
                unit_mask: 1u8 << unit,
                game_id: 20_000 + idx as u16,
                power,
                skill: SkillSlot {
                    skill_type: 0,
                    value: skill_value,
                },
                base_bonus: rng.range(0, 31) as u8,
                limited_bonus: rng.range(0, 11) as u8,
                power_max: power,
                skill_max: skill_value,
            }
        })
        .collect()
}

fn support_deck_for_property(pool: &CardPool, offset: usize) -> SupportDeck {
    let mut cards = pool
        .indices()
        .enumerate()
        .filter(|(idx, _)| (idx + offset).is_multiple_of(3))
        .map(|(idx, card)| (pool.game_id(card), 13.0 - (idx % 7) as f64))
        .collect::<Vec<_>>();
    cards.sort_unstable_by(|left, right| right.1.total_cmp(&left.1));
    SupportDeck {
        count: cards.len().min(5) as u8,
        cards,
    }
}

fn build_special_exact_pool() -> CardPool {
    let count = 16u16;
    let mut builder = PoolBuilder::new(count);
    builder.add_unit_count_skill(UnitCountSkill {
        unit: 0,
        score_up: [10, 20, 30, 40, 50],
    });
    builder.add_diff_skill(DiffSkill {
        base: 12,
        increment: 6,
    });
    builder.add_ref_skill(RefSkill { rate: 50, max: 30 });

    for dense in 0..count {
        let idx = dense as usize;
        // Cultivation / before-after variants of one public card keep identity
        // dimensions identical.  Only numeric state / skill representation may
        // differ between the two dense variants.
        let public = idx / 2;
        let char_id = (public % 8 + 1) as u8;
        let attr = (public % 5) as u8;
        let unit = (public % 6) as u8;
        let base = 850 + public as u32 * 71 + (idx % 2) as u32 * 9;
        let profile = [
            base,
            base + (idx as u32 % 4) * 11,
            base + (idx as u32 % 3) * 17,
            base + (idx as u32 % 5) * 13,
            base + 7,
            base + 19,
            base + 23,
            base + 29,
        ];
        let mut values = [0u16; 8];
        let mut lut = 0u32;
        for (slot, value) in profile.into_iter().enumerate() {
            values[slot] = value as u16;
            lut |= ((value >> 16) & 3) << (slot * 2);
        }
        builder.set_power_values(dense, values);
        builder.set_power_lut(dense, lut);

        let (skill, skill_min, skill_max) = match idx % 4 {
            0 => (
                SkillSlot {
                    skill_type: 0,
                    value: 25 + (idx % 20) as u8,
                },
                25 + (idx % 20) as u8,
                25 + (idx % 20) as u8,
            ),
            1 => (
                SkillSlot {
                    skill_type: 1,
                    value: 1,
                },
                10,
                50,
            ),
            2 => (
                SkillSlot {
                    skill_type: 2,
                    value: 1,
                },
                12,
                24,
            ),
            _ => (
                SkillSlot {
                    skill_type: 3,
                    value: 1,
                },
                15,
                45,
            ),
        };
        builder.set_skill(dense, skill);
        builder.set_skill_min(dense, skill_min);
        builder.set_skill_max(dense, skill_max);
        builder.set_event_bonus(
            dense,
            EventBonusExact::from_whole((idx % 17) as u16, (idx % 7) as u16),
        );
        builder.set_char_id(dense, char_id);
        builder.set_attr(dense, attr);
        builder.set_unit_mask(dense, 1u8 << unit);
        // Two cultivation variants share game ids; exact search must not select
        // both and Top-K must deduplicate by the public card set.
        builder.set_game_id(dense, 31_000 + (idx / 2) as u16);
        builder.set_power_max(dense, *profile.iter().max().unwrap());
        builder.mark_char(char_id, dense);
        builder.mark_unit(unit, dense);
        builder.mark_attr(attr, dense);
    }
    builder.freeze()
}

fn longtail_cards(characters: u8, per_character: u8) -> Vec<TestCard> {
    let mut cards = Vec::new();
    for char_id in 1..=characters {
        for variant in 0..per_character {
            let idx = (char_id as u32 - 1) * per_character as u32 + variant as u32;
            let mut attr = variant % 2;
            if variant == 2 && char_id <= 3 {
                attr = char_id + 1; // scarce attrs 2/3/4 live in distinct groups
            }
            let power = 4100u32
                .saturating_sub(variant as u32 * 360)
                .saturating_add(char_id as u32 * 7);
            let skill = 35 + variant * 14;
            cards.push(TestCard {
                char_id,
                attr,
                unit_mask: 1u8 << ((char_id + variant) % 6),
                game_id: 40_000 + idx as u16,
                power,
                skill: SkillSlot {
                    skill_type: 0,
                    value: skill,
                },
                base_bonus: 8 + variant * 8,
                limited_bonus: if variant >= 3 { 5 } else { 0 },
                power_max: power,
                skill_max: skill,
            });
        }
    }
    cards
}

fn longtail_support(pool: &CardPool, salt: usize) -> SupportDeck {
    let mut cards = pool
        .indices()
        .enumerate()
        .filter(|(idx, _)| (idx + salt).is_multiple_of(5))
        .map(|(idx, card)| (pool.game_id(card), 35.0 - (idx % 13) as f64 * 0.7))
        .collect::<Vec<_>>();
    cards.sort_unstable_by(|left, right| right.1.total_cmp(&left.1));
    SupportDeck {
        count: cards.len().min(10) as u8,
        cards,
    }
}

fn median_f64(values: &mut [f64]) -> f64 {
    values.sort_by(f64::total_cmp);
    values[(values.len() - 1) / 2]
}
