pub mod bruteforce;
pub mod challenge_search;
pub mod context;
pub mod dfs;
pub mod dominance;
pub mod evaluate;
mod final_chapter;
pub mod suffix;
pub mod types;
pub mod warm_start;

pub use bruteforce::{brute_force_search, BruteForceStats};
pub use context::{SearchContext, SupportDeck};
pub use dfs::{dfs_search, SearchStats};
pub use dominance::eliminate_dominated;
pub use evaluate::{
    calc_event_point, decode_u18, leaf_evaluate, resolve_power_for_cards, summarize_deck,
};
pub use suffix::{PartialDeck, SuffixBound, UsedSet};
pub use types::{DeckResult, DeckResultSummary, SearchParams};
pub use warm_start::warm_start;

use crate::pool::{CardIdx, CardPool};
use crate::types::{ScoreTarget, DECK_SIZE};

/// Reusable immutable search data for one `CardPool` / `SearchContext` pair.
///
/// Preparing performs dominance compaction, suffix-table construction and warm
/// seeding once. Callers that already cache the pool can keep this beside it and
/// execute repeated exact searches without rebuilding those structures.
pub struct PreparedSearch {
    pool: CardPool,
    ctx: SearchContext,
    original_indices: Vec<CardIdx>,
    alternatives: Vec<Vec<CardIdx>>,
    suffix: SuffixBound,
    warm_seeds: Vec<DeckResult>,
    max_top_k: usize,
}

impl PreparedSearch {
    /// Prepares the standard character-unique DFS path.
    ///
    /// Specialized Power/Skill, challenge and Final Chapter searches keep their
    /// existing entry points and return `None` here.
    pub fn build(pool: &CardPool, ctx: &SearchContext, max_top_k: usize) -> Option<Self> {
        if max_top_k == 0
            || pool.count() < DECK_SIZE
            || matches!(ctx.target, ScoreTarget::Power | ScoreTarget::Skill)
            || !ctx.enforce_char_uniqueness
            || ctx.is_final_chapter
        {
            return None;
        }

        let dominance = eliminate_dominated(pool, ctx);
        let suffix = SuffixBound::build_prepared(&dominance.pool, &dominance.ctx);
        let warm_seeds = warm_start::warm_start_seeds(&dominance.pool, &dominance.ctx, max_top_k);
        Some(Self {
            pool: dominance.pool,
            ctx: dominance.ctx,
            original_indices: dominance.original_indices,
            alternatives: dominance.alternatives,
            suffix,
            warm_seeds,
            max_top_k,
        })
    }

    /// Executes an exact search when `params.top_k` is covered by this plan.
    pub fn search_instrumented(
        &self,
        original_pool: &CardPool,
        original_ctx: &SearchContext,
        params: &SearchParams,
    ) -> Option<(Vec<DeckResult>, SearchStats)> {
        if params.top_k == 0 || params.top_k > self.max_top_k {
            return None;
        }
        let seeds = self.warm_seeds.iter().copied().take(params.top_k).collect();
        let (compacted_results, stats) = dfs::dfs_search_instrumented_with_seeds(
            &self.pool,
            &self.ctx,
            &self.suffix,
            params,
            seeds,
        );
        let remapped = remap_results(compacted_results, &self.original_indices);
        let expanded = expand_dominated_alternatives(
            original_pool,
            original_ctx,
            &self.alternatives,
            params,
            remapped,
        );
        Some((expanded, stats))
    }
}

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
    let alternatives = dominance.alternatives;
    if search_ctx.is_final_chapter {
        let member = dominance::compute_member_dominance(&search_pool, &search_ctx);
        // member 裁剪的替代记录映射回原始索引，并与第一轮 alternatives 做跨轮链闭包：
        // 真实次优卡组的 member 位可能是第一轮就被裁的卡（根 x），而 x 又被 member 轮
        // 裁掉（根 r）——从 r 出发必须能一步回换到它们（issue #7）。
        let mut member_alternatives = vec![Vec::new(); pool.count()];
        for (dense, alts) in member.alternatives.iter().enumerate() {
            if alts.is_empty() {
                continue;
            }
            let root = original_indices[dense].raw();
            for &alt_dense in alts {
                let alt = original_indices[alt_dense.raw()];
                member_alternatives[root].push(alt);
                member_alternatives[root].extend_from_slice(&alternatives[alt.raw()]);
            }
        }
        let member_keep = member.keep;
        if let Some(leader_char) = search_ctx.final_chapter_leader_character() {
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
        let (compacted_results, stats) = if search_ctx.final_chapter_leader_character().is_some() {
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
        let expanded = expand_alternatives(
            pool,
            ctx,
            &alternatives,
            &member_alternatives,
            params,
            remapped,
        );
        return (expanded, stats);
    }
    let suffix = SuffixBound::build(&search_pool, &search_ctx);
    let seeds = warm_start::warm_start_seeds(&search_pool, &search_ctx, params.top_k);
    let (compacted_results, stats) =
        dfs::dfs_search_instrumented_with_seeds(&search_pool, &search_ctx, &suffix, params, seeds);
    let remapped = remap_results(compacted_results, &original_indices);
    let expanded = expand_dominated_alternatives(pool, ctx, &alternatives, params, remapped);
    (expanded, stats)
}

/// 在一次 DFS 中为每个指定活动加成档位返回独立 Top-K。
pub fn search_bonus_targets(
    pool: &CardPool,
    ctx: &SearchContext,
    params: &SearchParams,
    targets: &[i32],
) -> (Vec<DeckResult>, SearchStats) {
    if params.top_k == 0
        || pool.count() < DECK_SIZE
        || targets.is_empty()
        || !matches!(ctx.target, ScoreTarget::Bonus)
    {
        return (Vec::new(), SearchStats::default());
    }
    let suffix = SuffixBound::build(pool, ctx);
    dfs::dfs_search_bonus_targets(pool, ctx, &suffix, params, targets)
}

/// Top-K 支配替代展开。
///
/// dominance 裁剪对 Top-1 无损（被裁卡换成支配者分数不降），但 Top-K 下被裁卡参与的
/// 组合本身可能是合法的次优解（issue #2）。设真实 Top-K 中存在含被裁卡的卡组 D，把
/// 其中每张被裁卡换成其支配根得到 D'，则 score(D') >= score(D) >= 第 K 名阈值，故 D'
/// 必在裁剪池的精确 Top-K 结果里。因此对每个搜索结果按槽位做替代回换（含多槽组合）、
/// 重新评估并合并，即可还原全部丢失的次优解。
///
/// 回换方向是支配的逆向，分数单调不升，按当前第 K 名阈值剪枝；`top_k <= 1` 直接跳过，
/// 主搜索路径零开销。
fn expand_dominated_alternatives(
    pool: &CardPool,
    ctx: &SearchContext,
    alternatives: &[Vec<CardIdx>],
    params: &SearchParams,
    results: Vec<DeckResult>,
) -> Vec<DeckResult> {
    expand_alternatives(pool, ctx, alternatives, &[], params, results)
}

/// `member_alternatives` 仅在 member 槽位（slot >= 1）参与回换：终章 member 裁剪
/// 忽略队长专属加成，被裁卡作队长仍可能更优，不能回换进队长槽。
///
/// 终章额外从每个结果的队长轮换出发展开：Top-K tracker 按卡集合去重、只保留最优
/// 排列，若某替代根恰是自身集合的最佳队长，它在结果里只出现在队长槽，直接回换
/// 永远不触发；轮换把根移回 member 槽后再回换，并顺带修正集合在其它队长下的
/// 最优排列分数。轮换按固定槽约束过滤，逐一精确评估后并入 tracker。
fn expand_alternatives(
    pool: &CardPool,
    ctx: &SearchContext,
    alternatives: &[Vec<CardIdx>],
    member_alternatives: &[Vec<CardIdx>],
    params: &SearchParams,
    results: Vec<DeckResult>,
) -> Vec<DeckResult> {
    if params.top_k <= 1 {
        return results;
    }
    let rotate_leader = ctx.is_final_chapter;
    let has_alternatives = results.iter().any(|result| {
        result.cards.iter().enumerate().any(|(slot, card)| {
            !alternatives[card.raw()].is_empty()
                || ((slot > 0 || rotate_leader)
                    && member_alternatives
                        .get(card.raw())
                        .is_some_and(|alts| !alts.is_empty()))
        })
    });
    if !has_alternatives && !rotate_leader {
        return results;
    }

    let mut tracker = dfs::TopKTracker::new(params.top_k, pool);
    for result in &results {
        tracker.insert(*result);
    }
    for result in &results {
        let mut deck = result.cards;
        expand_substitutions(
            pool,
            ctx,
            alternatives,
            member_alternatives,
            &mut deck,
            result.score,
            0,
            &mut tracker,
        );
        if !rotate_leader {
            continue;
        }
        let mut slot = 1usize;
        while slot < DECK_SIZE {
            let mut rotated = result.cards;
            rotated.swap(0, slot);
            slot += 1;
            if !deck_matches_fixed_slots(pool, ctx, &rotated) {
                continue;
            }
            let Some(score) = evaluate::leaf_evaluate_checked(pool, ctx, &rotated) else {
                continue;
            };
            tracker.insert(DeckResult::new(rotated, score));
            expand_substitutions(
                pool,
                ctx,
                alternatives,
                member_alternatives,
                &mut rotated,
                score,
                0,
                &mut tracker,
            );
        }
    }
    tracker.into_vec()
}

/// 判断卡组每个槽位是否满足固定卡/固定角色约束（队长轮换用）。
fn deck_matches_fixed_slots(
    pool: &CardPool,
    ctx: &SearchContext,
    deck: &[CardIdx; DECK_SIZE],
) -> bool {
    let mut slot = 0usize;
    while slot < DECK_SIZE {
        if ctx
            .fixed_card_at(slot)
            .is_some_and(|game_id| pool.game_id(deck[slot]) != game_id)
        {
            return false;
        }
        if ctx
            .fixed_character_at(slot)
            .is_some_and(|character_id| pool.char_id(deck[slot]) != character_id)
        {
            return false;
        }
        slot += 1;
    }
    true
}

/// 自 `from_slot` 起逐槽尝试把支配者回换成其支配的卡（多槽组合经递归覆盖）。
/// `node_score` 是当前替换组合的分数；再多换任何一张分数不会更高，因此 tracker
/// 满且 node_score 严格低于阈值时整棵子树可剪（同分仍展开，保住 tie-break 名次）。
/// 两轮支配都含支援惩罚维度（issue #23/#7），该单调性在 WL 下同样成立。
#[allow(clippy::too_many_arguments)]
fn expand_substitutions(
    pool: &CardPool,
    ctx: &SearchContext,
    alternatives: &[Vec<CardIdx>],
    member_alternatives: &[Vec<CardIdx>],
    deck: &mut [CardIdx; DECK_SIZE],
    node_score: u64,
    from_slot: usize,
    tracker: &mut dfs::TopKTracker,
) {
    let threshold = tracker.threshold();
    if threshold != 0 && node_score < threshold {
        return;
    }
    let mut slot = from_slot;
    while slot < DECK_SIZE {
        // 固定卡槽位按 game_id 锁死，被支配的替代卡 game_id 必不同（固定卡不参与裁剪），跳过。
        if ctx.fixed_card_at(slot).is_some() {
            slot += 1;
            continue;
        }
        let original = deck[slot];
        let member_alts: &[CardIdx] = if slot > 0 {
            member_alternatives
                .get(original.raw())
                .map(Vec::as_slice)
                .unwrap_or(&[])
        } else {
            &[]
        };
        for &alt in alternatives[original.raw()].iter().chain(member_alts) {
            deck[slot] = alt;
            // 支配卡与被支配卡同角色，角色唯一性与固定角色槽位约束自然保持。
            let Some(score) = evaluate::leaf_evaluate_checked(pool, ctx, deck) else {
                continue;
            };
            tracker.insert(DeckResult::new(*deck, score));
            expand_substitutions(
                pool,
                ctx,
                alternatives,
                member_alternatives,
                deck,
                score,
                slot + 1,
                tracker,
            );
        }
        deck[slot] = original;
        slot += 1;
    }
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

    if matches!(ctx.target, ScoreTarget::Power)
        && !ctx.minimize
        && ctx.enforce_char_uniqueness
        && ctx.fixed_card_ids.is_empty()
        && ctx.fixed_character_ids.is_empty()
    {
        return search_power_scenarios(pool, ctx, params);
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
        let ordering = if minimize { ka.cmp(&kb) } else { kb.cmp(&ka) };
        ordering.then_with(|| a.raw().cmp(&b.raw()))
    });

    let mut prefix = Vec::with_capacity(prefix_len + 8);
    let mut in_prefix = vec![false; pool.count()];
    let mut char_counts = [0u8; 27];

    for &card in &cards {
        let gid = pool.game_id(card);
        let cid = pool.char_id(card);
        if (ctx.fixed_card_ids.contains(&gid) || ctx.fixed_character_ids.contains(&cid))
            && !in_prefix[card.raw()]
        {
            in_prefix[card.raw()] = true;
            char_counts[(cid as usize).min(26)] += 1;
            prefix.push(card);
        }
    }

    for &card in &cards {
        if prefix.len() >= prefix_len {
            break;
        }
        if in_prefix[card.raw()] {
            continue;
        }
        let ch = (pool.char_id(card) as usize).min(26);
        if (char_counts[ch] as usize) >= per_char_cap {
            continue;
        }
        char_counts[ch] += 1;
        in_prefix[card.raw()] = true;
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

#[derive(Clone, Copy)]
struct PowerPartial {
    cards: [CardIdx; DECK_SIZE],
    len: usize,
    additive_power: u32,
}

fn search_power_scenarios(
    pool: &CardPool,
    ctx: &SearchContext,
    params: &SearchParams,
) -> (Vec<DeckResult>, SearchStats) {
    // For an additive scenario, keeping the best K partial states at each
    // cardinality is exact: every future choice is independent of the cards
    // already processed. A discarded partial state can therefore never re-enter
    // the final top K.
    let state_limit = params.top_k.max(1);
    let mut tracker = SimpleTopKTracker::new(params.top_k, false, pool);
    let mut stats = SearchStats::default();

    let mut scenarios = Vec::with_capacity(49);
    scenarios.push((None, None));
    for attr in 0u8..6 {
        scenarios.push((None, Some(attr)));
    }
    for unit in 0usize..6 {
        scenarios.push((Some(unit), None));
        for attr in 0u8..6 {
            scenarios.push((Some(unit), Some(attr)));
        }
    }

    for (unit_all, attr_all) in scenarios {
        let mut by_character = vec![Vec::<(u32, CardIdx)>::new(); 27];
        for card in pool.indices() {
            if unit_all.is_some_and(|unit| pool.unit_mask_raw(card) & (1u8 << unit) == 0) {
                continue;
            }
            if attr_all.is_some_and(|attr| pool.attr(card) != attr) {
                continue;
            }
            let character = usize::from(pool.char_id(card)).min(26);
            let power =
                evaluate::resolve_card_power_scenario(pool, card, unit_all, attr_all.is_some());
            by_character[character].push((power, card));
        }
        for cards in &mut by_character {
            cards.sort_unstable_by(|left, right| {
                right
                    .0
                    .cmp(&left.0)
                    .then_with(|| left.1.raw().cmp(&right.1.raw()))
            });
            // 同一 game_id 的养成变体互斥（同一张卡），只保留场景值最高的一个：
            // 变体占多个名额会在每角色候选与 DP 状态里挤出真正不同的次优集合，
            // 令 Top-K 丢解（issue #24 的 mass_099712 案例）。
            let mut seen_game_ids = Vec::with_capacity(cards.len());
            cards.retain(|(_, card)| {
                let game_id = pool.game_id(*card);
                if seen_game_ids.contains(&game_id) {
                    false
                } else {
                    seen_game_ids.push(game_id);
                    true
                }
            });
            cards.truncate(state_limit);
        }

        let seed = PowerPartial {
            cards: [CardIdx::new(0); DECK_SIZE],
            len: 0,
            additive_power: 0,
        };
        let mut states = vec![Vec::<PowerPartial>::new(); DECK_SIZE + 1];
        states[0].push(seed);
        for choices in by_character.into_iter().skip(1) {
            if choices.is_empty() {
                continue;
            }
            let mut count = DECK_SIZE;
            while count > 0 {
                count -= 1;
                if states[count].is_empty() {
                    continue;
                }
                let previous = states[count].clone();
                for state in previous {
                    for &(power, card) in &choices {
                        let mut next = state;
                        next.cards[count] = card;
                        next.len = count + 1;
                        next.additive_power = next.additive_power.saturating_add(power);
                        states[count + 1].push(next);
                    }
                }
                states[count + 1].sort_unstable_by(|left, right| {
                    right
                        .additive_power
                        .cmp(&left.additive_power)
                        .then_with(|| left.cards.cmp(&right.cards))
                });
                states[count + 1].truncate(state_limit);
            }
        }

        for state in &states[DECK_SIZE] {
            stats.leaf_nodes += 1;
            if let Some(score) = evaluate::leaf_evaluate_checked(pool, ctx, &state.cards) {
                tracker.insert(DeckResult::new(state.cards, score));
            }
        }
    }
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
        if let Some(game_id) = ctx.fixed_card_at(depth)
            && pool.game_id(card) != game_id {
                idx += 1;
                continue;
            }
        if let Some(character_id) = fixed_char_at_depth
            && char_id != character_id {
                idx += 1;
                continue;
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
mod tests;
