pub mod context;
pub mod dfs;
pub mod dominance;
pub mod evaluate;
pub mod suffix;
pub mod types;
pub mod warm_start;

pub use context::{SearchContext, SupportDeck};
pub use dfs::dfs_search;
pub use dominance::eliminate_dominated;
pub use evaluate::{calc_event_point, decode_u18, leaf_evaluate};
pub use suffix::{PartialDeck, SuffixBound, UsedSet};
pub use types::{DeckResult, SearchParams};
pub use warm_start::warm_start;

use crate::pool::CardPool;
use crate::types::DECK_SIZE;

/// 执行完整搜索流水线：dominance 裁剪、上界构建、热启动、DFS/B&B。
pub fn search(pool: &CardPool, ctx: &SearchContext, params: &SearchParams) -> Vec<DeckResult> {
    if params.top_k == 0 || pool.count() < DECK_SIZE {
        return Vec::new();
    }

    let dominance = eliminate_dominated(pool, ctx);
    let suffix = SuffixBound::build(&dominance.pool, &dominance.ctx);
    let seed = warm_start::warm_start_best(&dominance.pool, &dominance.ctx);
    let compacted_results =
        dfs::dfs_search_seeded(&dominance.pool, &dominance.ctx, &suffix, params, seed);
    let remapped = remap_results(compacted_results, &dominance.original_indices);
    if matches!(ctx.target, crate::types::ScoreTarget::Bonus) && !ctx.bonus_targets.is_empty() {
        remapped
            .into_iter()
            .filter(|result| result.score != 0)
            .collect()
    } else {
        remapped
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
            bonus_targets: Vec::new(),
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
            live_type: LiveType::Solo,
            event_type: None,
            keep_after_training_state: false,
            skill_reference_strategy: SkillReferenceStrategy::Average,
            best_skill_as_leader: true,
            live_skill_order: LiveSkillOrder::Best,
            specific_skill_order: None,
            multi_teammate_score_up: None,
            multi_teammate_power: None,
            extra_bonus_ub: 0,
            w_power: 2.0,
            w_bonus: 1.0,
            skill_ub_global: 0,
            card_bonus_count_limit: DECK_SIZE,
            leader_honor_bonus: Vec::new(),
            leader_limit_bonus: Vec::new(),
            skill_is_after_training: Vec::new(),
            trained_to_special_image: Vec::new(),
        }
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
        let results = dfs::dfs_search_power_len_for_test(&pool, &suffix, 2, 1);
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
