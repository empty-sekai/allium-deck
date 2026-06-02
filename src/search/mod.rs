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
pub use evaluate::{calc_event_point, decode_u18, leaf_evaluate, summarize_deck};
pub use suffix::{PartialDeck, SuffixBound, UsedSet};
pub use types::{DeckResult, DeckResultSummary, SearchParams};
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
        let (compacted_results, stats) = if search_ctx.fixed_character_at(0).is_some() {
            final_chapter::search_fixed_leader(&search_pool, &search_ctx, params)
        } else if !search_ctx.has_fixed_leader() {
            final_chapter::search_auto_leader(&search_pool, &search_ctx, params)
        } else {
            let suffix = SuffixBound::build(&search_pool, &search_ctx);
            let seeds = warm_start::warm_start_best(&search_pool, &search_ctx)
                .into_iter()
                .collect();
            dfs::dfs_search_instrumented_with_seeds(
                &search_pool,
                &search_ctx,
                &suffix,
                params,
                seeds,
            )
        };
        let remapped = remap_results(compacted_results, &original_indices);
        return (remapped, stats);
    }
    let suffix = SuffixBound::build(&search_pool, &search_ctx);
    let seeds = warm_start::warm_start_best(&search_pool, &search_ctx)
        .into_iter()
        .collect();
    let (compacted_results, stats) =
        dfs::dfs_search_instrumented_with_seeds(&search_pool, &search_ctx, &suffix, params, seeds);
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
    let minimize = ctx.minimize && matches!(ctx.target, ScoreTarget::Power);
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
        // minimize 时按质量升序取最弱前缀；否则降序取最强。
        let ordering = if minimize {
            ka.cmp(&kb)
        } else {
            kb.cmp(&ka)
        };
        ordering.then_with(|| a.raw().cmp(&b.raw()))
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

    let mut tracker = SimpleTopKTracker::new(params.top_k, minimize, pool);
    let mut deck = [prefix[0]; DECK_SIZE];
    let mut stats = SearchStats::default();
    simple_target_recurse(
        pool,
        ctx,
        &prefix,
        0,
        0,
        0,
        0,
        &mut deck,
        &mut tracker,
        &mut stats,
    );

    (tracker.into_vec(), stats)
}

fn simple_target_recurse(
    pool: &CardPool,
    ctx: &SearchContext,
    prefix: &[CardIdx],
    depth: usize,
    min_free_idx: usize,
    used_cards: u64,
    used_chars: u32,
    deck: &mut [CardIdx; DECK_SIZE],
    tracker: &mut SimpleTopKTracker,
    stats: &mut SearchStats,
) {
    if depth == DECK_SIZE {
        stats.leaf_nodes += 1;
        if let Some(score) = evaluate::leaf_evaluate_checked(pool, ctx, deck) {
            tracker.insert(DeckResult::new(*deck, score));
        }
        return;
    }

    let is_fixed = ctx.is_fixed_slot(depth);
    let scan_from = if is_fixed { 0 } else { min_free_idx };

    let mut idx = scan_from;
    while idx < prefix.len() {
        if used_cards & (1u64 << idx) != 0 {
            idx += 1;
            continue;
        }
        let card = prefix[idx];
        let char_id = pool.char_id(card);
        let fixed_char_at_depth = ctx.fixed_character_at(depth);
        if used_chars & (1u32 << char_id) != 0 {
            // 固定角色槽位允许同一角色的另一张卡入队
            if fixed_char_at_depth != Some(char_id) {
                idx += 1;
                continue;
            }
        }
        if let Some(game_id) = ctx.fixed_card_at(depth) {
            if pool.game_id(card) != game_id {
                idx += 1;
                continue;
            }
        }
        if let Some(character_id) = fixed_char_at_depth {
            if char_id != character_id {
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
            used_cards | (1u64 << idx),
            used_chars | (1u32 << char_id),
            deck,
            tracker,
            stats,
        );
        idx += 1;
    }
}

struct SimpleTopKTracker {
    top_k: usize,
    minimize: bool,
    game_ids: Vec<u16>,
    results: Vec<DeckResult>,
}

impl SimpleTopKTracker {
    fn new(top_k: usize, minimize: bool, pool: &CardPool) -> Self {
        Self {
            top_k,
            minimize,
            game_ids: pool.indices().map(|card| pool.game_id(card)).collect(),
            results: Vec::with_capacity(top_k),
        }
    }

    /// candidate 是否比 incumbent 更优。minimize 时「更优」= 分数更小。
    #[inline(always)]
    fn is_better(&self, candidate: &DeckResult, incumbent: &DeckResult) -> bool {
        let cmp = deck_result_cmp(candidate, incumbent);
        if self.minimize {
            cmp.is_gt()
        } else {
            cmp.is_lt()
        }
    }

    fn insert(&mut self, candidate: DeckResult) {
        if let Some(existing_pos) = self
            .results
            .iter()
            .position(|existing| self.same_game_card_set(existing, &candidate))
        {
            if !self.is_better(&candidate, &self.results[existing_pos]) {
                return;
            }
            self.results.remove(existing_pos);
        }
        let pos = self
            .results
            .iter()
            .position(|existing| self.is_better(&candidate, existing))
            .unwrap_or(self.results.len());
        self.results.insert(pos, candidate);
        if self.results.len() > self.top_k {
            self.results.pop();
        }
    }

    fn into_vec(self) -> Vec<DeckResult> {
        self.results
    }

    fn same_game_card_set(&self, left: &DeckResult, right: &DeckResult) -> bool {
        self.game_card_set_key(left) == self.game_card_set_key(right)
    }

    fn game_card_set_key(&self, result: &DeckResult) -> [u16; 5] {
        let mut cards = result.cards.map(|card| self.game_ids[card.raw()]);
        cards.sort_unstable();
        cards
    }
}

#[inline(always)]
fn deck_result_cmp(left: &DeckResult, right: &DeckResult) -> std::cmp::Ordering {
    right
        .score
        .cmp(&left.score)
        .then_with(|| left.cards.cmp(&right.cards))
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
                EventBonusHot::from_whole(card.base_bonus, card.limited_bonus),
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
            bonus: pool.event_bonus(selected).base_ceil(),
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
}
