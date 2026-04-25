pub mod challenge_search;
pub mod context;
pub mod dfs;
pub mod dominance;
pub mod evaluate;
mod final_chapter;
pub mod suffix;
pub mod types;
pub mod warm_start;

pub use context::{SearchContext, SupportDeck};
pub use dfs::{dfs_search, SearchStats};
pub use dominance::eliminate_dominated;
pub use evaluate::{calc_event_point, decode_u18, leaf_evaluate};
pub use suffix::{PartialDeck, SuffixBound, UsedSet};
pub use types::{DeckResult, SearchParams};
pub use warm_start::warm_start;

use crate::pool::{CardIdx, CardPool};
use crate::types::{ScoreTarget, DECK_SIZE};

/// 执行完整搜索流水线：dominance 裁剪、上界构建、热启动、DFS/B&B。
pub fn search(pool: &CardPool, ctx: &SearchContext, params: &SearchParams) -> Vec<DeckResult> {
    let (results, _) = search_instrumented(pool, ctx, params);
    results
}

/// 带统计信息的搜索。
pub fn search_instrumented(
    pool: &CardPool,
    ctx: &SearchContext,
    params: &SearchParams,
) -> (Vec<DeckResult>, SearchStats) {
    if params.top_k == 0 || pool.count() < DECK_SIZE {
        return (Vec::new(), SearchStats::default());
    }
    if ctx.is_final_chapter && ctx.fixed_character_at(0).is_some() {
        return final_chapter::search_fixed_leader(pool, ctx, params);
    }
    if matches!(ctx.target, ScoreTarget::Power | ScoreTarget::Skill) {
        return search_simple_target(pool, ctx, params);
    }

    if !ctx.enforce_char_uniqueness {
        let suffix = SuffixBound::build(pool, ctx);
        return challenge_search::search(pool, ctx, &suffix, params);
    }

    let dominance = eliminate_dominated(pool, ctx);
    let mut search_pool = dominance.pool;
    let mut search_ctx = dominance.ctx;
    let mut original_indices = dominance.original_indices;
    if search_ctx.is_final_chapter {
        let member_keep = dominance::compute_member_keep(&search_pool);
        if let Some(leader_char) = search_ctx.fixed_character_at(0) {
            let keep = search_pool
                .indices()
                .map(|card| {
                    search_pool.char_id(card) == leader_char
                        || (member_keep.get(card.raw()).copied().unwrap_or(true)
                            && search_pool.char_id(card) != leader_char)
                })
                .collect::<Vec<_>>();
            original_indices = original_indices
                .into_iter()
                .zip(keep.iter().copied())
                .filter_map(|(idx, keep)| keep.then_some(idx))
                .collect();
            search_pool = search_pool.compact(&keep);
            search_ctx = search_ctx.remap(&keep);
            search_ctx.final_chapter_member_keep = vec![true; search_pool.count()];
        } else {
            search_ctx.final_chapter_member_keep = member_keep;
        }
    }
    let suffix = SuffixBound::build(&search_pool, &search_ctx);
    let seed = warm_start::warm_start_best(&search_pool, &search_ctx);
    let (compacted_results, stats) =
        dfs::dfs_search_instrumented(&search_pool, &search_ctx, &suffix, params, seed);
    let remapped = remap_results(compacted_results, &original_indices);
    (remapped, stats)
}

fn search_simple_target(
    pool: &CardPool,
    ctx: &SearchContext,
    params: &SearchParams,
) -> (Vec<DeckResult>, SearchStats) {
    const POWER_PREFIX: usize = 28;
    const POWER_PER_CHAR: usize = 6;
    const SKILL_PREFIX: usize = 20;
    const SKILL_PER_CHAR: usize = 3;
    const SCORE_NOEV_PREFIX: usize = 30;
    const SCORE_NOEV_PER_CHAR: usize = 6;

    if params.top_k == 0 || pool.count() < DECK_SIZE {
        return (Vec::new(), SearchStats::default());
    }

    let (prefix_len, per_char_cap) = match ctx.target {
        ScoreTarget::Power => (POWER_PREFIX, POWER_PER_CHAR),
        ScoreTarget::Skill => (SKILL_PREFIX, SKILL_PER_CHAR),
        _ => (SCORE_NOEV_PREFIX, SCORE_NOEV_PER_CHAR),
    };

    let mut cards: Vec<CardIdx> = pool.indices().collect();
    cards.sort_unstable_by(|a, b| {
        let (ka, kb) = match ctx.target {
            ScoreTarget::Power => (pool.power_max(*a) as u64, pool.power_max(*b) as u64),
            ScoreTarget::Skill => (pool.skill_max(*a) as u64, pool.skill_max(*b) as u64),
            _ => {
                let ka = pool.power_max(*a) as u64 * (256 + pool.skill_max(*a) as u64);
                let kb = pool.power_max(*b) as u64 * (256 + pool.skill_max(*b) as u64);
                (ka, kb)
            }
        };
        kb.cmp(&ka).then_with(|| a.raw().cmp(&b.raw()))
    });

    let mut prefix = Vec::with_capacity(prefix_len + 8);
    let mut in_prefix = vec![false; pool.count()];
    let mut char_counts = [0u8; 27];

    for &card in &cards {
        let gid = pool.game_id(card);
        let cid = pool.char_id(card);
        if ctx.fixed_card_ids.contains(&gid) || ctx.fixed_character_ids.contains(&cid) {
            if !in_prefix[card.raw() as usize] {
                in_prefix[card.raw() as usize] = true;
                char_counts[(cid as usize).min(26)] += 1;
                prefix.push(card);
            }
        }
    }

    for &card in &cards {
        if prefix.len() >= prefix_len {
            break;
        }
        if in_prefix[card.raw() as usize] {
            continue;
        }
        let ch = (pool.char_id(card) as usize).min(26);
        if (char_counts[ch] as usize) >= per_char_cap {
            continue;
        }
        char_counts[ch] += 1;
        in_prefix[card.raw() as usize] = true;
        prefix.push(card);
    }

    if prefix.len() < DECK_SIZE {
        return (Vec::new(), SearchStats::default());
    }

    let mut best: Option<DeckResult> = None;
    let mut deck = [prefix[0]; DECK_SIZE];
    let mut stats = SearchStats::default();
    simple_target_recurse(
        pool, ctx, &prefix, 0, 0, 0, 0, &mut deck, &mut best, &mut stats,
    );

    (best.into_iter().collect(), stats)
}

fn simple_target_recurse(
    pool: &CardPool,
    ctx: &SearchContext,
    prefix: &[CardIdx],
    depth: usize,
    min_free_idx: usize,
    used_cards: u32,
    used_chars: u32,
    deck: &mut [CardIdx; DECK_SIZE],
    best: &mut Option<DeckResult>,
    stats: &mut SearchStats,
) {
    if depth == DECK_SIZE {
        stats.leaf_nodes += 1;
        if let Some(score) = evaluate::leaf_evaluate_checked(pool, ctx, deck) {
            match best {
                Some(ref current) if score <= current.score => {}
                _ => *best = Some(DeckResult::new(*deck, score)),
            }
        }
        return;
    }

    let is_fixed = ctx.is_fixed_slot(depth);
    let scan_from = if is_fixed { 0 } else { min_free_idx };

    let mut idx = scan_from;
    while idx < prefix.len() {
        if used_cards & (1u32 << idx) != 0 {
            idx += 1;
            continue;
        }
        let card = prefix[idx];
        let char_id = pool.char_id(card);
        if used_chars & (1u32 << char_id) != 0 {
            idx += 1;
            continue;
        }
        if let Some(game_id) = ctx.fixed_card_at(depth) {
            if pool.game_id(card) != game_id {
                idx += 1;
                continue;
            }
        }
        if let Some(character_id) = ctx.fixed_character_at(depth) {
            if pool.char_id(card) != character_id {
                idx += 1;
                continue;
            }
        }
        deck[depth] = card;
        let next_min_free = if is_fixed { min_free_idx } else { idx + 1 };
        simple_target_recurse(
            pool,
            ctx,
            prefix,
            depth + 1,
            next_min_free,
            used_cards | (1u32 << idx),
            used_chars | (1u32 << char_id),
            deck,
            best,
            stats,
        );
        idx += 1;
    }
}

fn remap_results(
    results: Vec<DeckResult>,
    original_indices: &[crate::pool::CardIdx],
) -> Vec<DeckResult> {
    results
        .into_iter()
        .map(|mut result| {
            for card in &mut result.cards {
                let dense = card.raw();
                debug_assert!(
                    dense < original_indices.len(),
                    "compacted search result index must have original mapping",
                );
                if let Some(original) = original_indices.get(dense).copied() {
                    *card = original;
                }
            }
            result
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pool::{DiffSkill, EventBonusHot, PoolBuilder, RefSkill, SkillSlot, UnitCountSkill};
    use crate::types::{LiveSkillOrder, LiveType, ScoreTarget, SkillReferenceStrategy};

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
                EventBonusHot {
                    base_bonus: card.base_bonus,
                    limited_bonus: card.limited_bonus,
                },
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
            is_world_bloom: false,
            is_final_chapter: false,
            enforce_char_uniqueness: true,
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
            bonus: pool.event_bonus(selected).base_bonus as u32,
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
        let (results, stats) =
            dfs::dfs_search_instrumented(&pool, &search_ctx, &suffix, &params, seed);
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
}
