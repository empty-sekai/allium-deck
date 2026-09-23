use crate::search::budget::{Instant, SearchBudget as ChallengeDeadline};
use crate::search::{SearchOutcome, SearchStats};
use std::time::Duration;

use crate::pool::{CardIdx, CardPool};
use crate::types::DECK_SIZE;

use crate::search::TopKTracker;
use crate::search::context::SearchContext;
use crate::search::evaluate::{leaf_evaluate_challenge_score_checked, leaf_evaluate_checked};
use crate::search::suffix::{PartialDeck, SuffixBound};
use crate::search::types::{DeckResult, SearchParams};
use crate::types::{LiveType, ScoreTarget};

#[cfg(test)]
mod deadline_tests {
    use super::*;

    #[test]
    fn challenge_deadline_samples_and_stays_expired() {
        let start = Instant::now();
        let end = start + Duration::from_secs(1);
        let mut guard = ChallengeDeadline::new(Some(end));
        assert!(!guard.expired_sampled_with(|| start));
        for _ in 0..1023 {
            assert!(!guard.expired_sampled_with(|| panic!("unexpected clock read")));
        }
        assert!(guard.expired_sampled_with(|| end));
        for _ in 0..2048 {
            assert!(guard.expired_sampled_with(|| panic!("expired guard read the clock")));
        }
    }

    fn equal_bound_pool(count: u16) -> CardPool {
        let mut builder = crate::pool::PoolBuilder::new(count);
        for index in 0..count {
            builder.set_game_id(index, index + 1000);
            builder.set_char_id(index, 1);
            builder.set_power_max(index, 100);
            builder.set_skill_max(index, 20);
        }
        builder.freeze()
    }

    #[test]
    fn challenge_bounds_deduplicate_equal_states_for_every_suffix() {
        let pool = equal_bound_pool(100);
        let candidates: Vec<_> = pool.indices().collect();
        let mut deadline = ChallengeDeadline::new(None);
        let bounds = ChallengeBounds::build_with_clock(&pool, &candidates, &mut deadline, || {
            panic!("disabled deadline read the clock")
        })
        .unwrap();
        for position in 0..=pool.count() {
            for slots in 0..=DECK_SIZE {
                let frontier = &bounds.frontiers[position][slots];
                if slots > pool.count() - position {
                    assert!(frontier.is_empty());
                } else {
                    assert_eq!(
                        frontier,
                        &[BoundState {
                            power: 100 * slots as u32,
                            skill: 20 * slots as u32,
                            leader: if slots == 0 { 0 } else { 20 },
                        }]
                    );
                }
            }
        }
    }

    #[test]
    fn challenge_bounds_preserve_distinct_nondominated_states() {
        let a = BoundState {
            power: 100,
            skill: 10,
            leader: 10,
        };
        let b = BoundState {
            power: 90,
            skill: 20,
            leader: 10,
        };
        let mut states = vec![
            a,
            b,
            a,
            BoundState {
                power: 80,
                skill: 5,
                leader: 5,
            },
            BoundState {
                power: 100,
                skill: 10,
                leader: 9,
            },
            b,
        ];
        let mut deadline = ChallengeDeadline::new(None);
        prune_dominated(&mut states, &mut deadline, &mut || {
            panic!("disabled deadline read the clock")
        })
        .unwrap();
        assert_eq!(states, vec![a, b]);
    }

    #[test]
    fn challenge_bounds_abort_when_clock_expires_during_build() {
        let pool = equal_bound_pool(100);
        let candidates: Vec<_> = pool.indices().collect();
        let start = Instant::now();
        let end = start + Duration::from_secs(1);
        let mut deadline = ChallengeDeadline::new(Some(end));
        let mut reads = 0;
        let bounds = ChallengeBounds::build_with_clock(&pool, &candidates, &mut deadline, || {
            reads += 1;
            if reads == 1 { start } else { end }
        });
        assert!(bounds.is_none());
        assert!(deadline.hit);
        assert_eq!(reads, 2);
    }

    #[test]
    fn challenge_bounds_abort_during_pairwise_pruning() {
        let mut states: Vec<_> = (0..64)
            .map(|value| BoundState {
                power: value,
                skill: 64 - value,
                leader: 0,
            })
            .collect();
        let original = states.clone();
        let start = Instant::now();
        let end = start + Duration::from_secs(1);
        let mut deadline = ChallengeDeadline::new(Some(end));
        let mut reads = 0;
        let result = prune_dominated(&mut states, &mut deadline, &mut || {
            reads += 1;
            if reads == 1 { start } else { end }
        });
        assert!(result.is_none());
        assert!(deadline.hit);
        assert_eq!(reads, 2);
        assert_eq!(states, original);
    }

    #[test]
    fn challenge_deadline_checks_initial_expiry_and_skips_disabled_clock() {
        let now = Instant::now();
        let mut expired = ChallengeDeadline::new(Some(now));
        assert!(expired.expired_sampled_with(|| now));
        let mut disabled = ChallengeDeadline::new(None);
        for _ in 0..2048 {
            assert!(!disabled.expired_sampled_with(|| panic!("disabled deadline read the clock")));
        }
    }
}

/// challenge 模式专用搜索。
///
/// challenge 模式下 pool 全部为同角色卡，不要求角色唯一性，
/// 但仍需保证同一 game_id 不重复出现（单卡多个技能变体只取其一）。
pub fn search(
    pool: &CardPool,
    ctx: &SearchContext,
    suffix: &SuffixBound,
    params: &SearchParams,
) -> (Vec<DeckResult>, crate::search::SearchStats) {
    let mut deadline = ChallengeDeadline::from_params(params);
    search_with_budget(pool, ctx, suffix, params, &mut deadline)
}

pub(crate) fn search_with_budget(
    pool: &CardPool,
    ctx: &SearchContext,
    suffix: &SuffixBound,
    params: &SearchParams,
    deadline: &mut ChallengeDeadline,
) -> (Vec<DeckResult>, crate::search::SearchStats) {
    let (results, mut stats) =
        search_with_character_filter(pool, ctx, suffix, params, None, deadline);
    stats.deadline_hit |= deadline.hit;
    stats.finalize();
    (results, stats)
}

/// 在一个共享 challenge pool 中只搜索指定角色。
///
/// `build_card_pool` 不传 `challenge_live_character_id` 时会保留全角色候选；
/// challenge_all 可复用该 pool，并在搜索入口按角色过滤，避免为 26 个角色重复建池。
pub fn search_character(
    pool: &CardPool,
    ctx: &SearchContext,
    suffix: &SuffixBound,
    params: &SearchParams,
    character_id: u8,
) -> (Vec<DeckResult>, crate::search::SearchStats) {
    let mut deadline = ChallengeDeadline::from_params(params);
    let (results, mut stats) =
        search_with_character_filter(pool, ctx, suffix, params, Some(character_id), &mut deadline);
    stats.deadline_hit |= deadline.hit;
    stats.finalize();
    (results, stats)
}

/// challenge_all：逐角色搜索后按分数归并出全局 Top-K。
///
/// 挑战 live 的队伍必须五张同角色，所以答案集是各角色最优解的并集，而不是
/// 在混角色池上做一次无约束搜索——后者既会产出非法卡组，组合数也高数个量级。
/// All characters share one deadline, including each character's inner search.
/// Expiry returns the best complete decks found so far.
pub fn search_all_characters(
    pool: &CardPool,
    ctx: &SearchContext,
    suffix: &SuffixBound,
    params: &SearchParams,
) -> (Vec<DeckResult>, crate::search::SearchStats) {
    let mut deadline = ChallengeDeadline::from_params(params);
    search_all_characters_with_budget(pool, ctx, suffix, params, &mut deadline)
}

pub(crate) fn search_all_characters_with_budget(
    pool: &CardPool,
    ctx: &SearchContext,
    suffix: &SuffixBound,
    params: &SearchParams,
    deadline: &mut ChallengeDeadline,
) -> (Vec<DeckResult>, crate::search::SearchStats) {
    let mut present = [false; 27];
    for card in pool.indices() {
        present[(pool.char_id(card) as usize).min(26)] = true;
    }

    let mut merged = Vec::new();
    let mut stats = crate::search::SearchStats::default();
    for (character_id, present) in present.iter().copied().enumerate() {
        if !present {
            continue;
        }
        if deadline.expired() {
            break;
        }
        let (results, character_stats) = search_with_character_filter(
            pool,
            ctx,
            suffix,
            params,
            Some(character_id as u8),
            deadline,
        );
        stats.accumulate(&character_stats);
        merged.extend(results);
    }

    merged.sort_unstable_by(|left, right| crate::search::deck_result_cmp(pool, ctx, left, right));
    merged.truncate(params.top_k);
    stats.deadline_hit |= deadline.hit;
    stats.finalize();
    (merged, stats)
}

/// One character's decks, or those of the whole pool, searched per area-item
/// composition regime so that each regime reads power bounds valid for its
/// decks; see [`crate::search::composition`].
fn search_with_character_filter(
    pool: &CardPool,
    ctx: &SearchContext,
    suffix: &SuffixBound,
    params: &SearchParams,
    character_id: Option<u8>,
    deadline: &mut ChallengeDeadline,
) -> (Vec<DeckResult>, crate::search::SearchStats) {
    if params.top_k == 0 || pool.count() < DECK_SIZE || deadline.expired_sampled() {
        return (Vec::new(), crate::search::SearchStats::default());
    }
    let keep = pool
        .indices()
        .map(|card| character_id.is_none_or(|character_id| pool.char_id(card) == character_id))
        .collect::<Vec<_>>();
    let original = pool
        .indices()
        .filter(|card| keep[card.raw()])
        .collect::<Vec<_>>();
    let character_pool = pool.compact(&keep);
    let character_ctx = ctx.remap(&keep);
    let (mut results, stats) = crate::search::composition::search_regimes(
        &character_pool,
        &character_ctx,
        params,
        deadline,
        crate::search::tuning::SearchTuning::load().bounds,
        |_, _| Vec::new(),
        |pool, ctx, floor, _, deadline| search_regime(pool, ctx, suffix, params, floor, deadline),
    );
    for result in &mut results {
        for card in &mut result.cards {
            *card = original[card.raw()];
        }
    }
    (results, stats)
}

/// Exact Top-K of `pool`, whose power maxima are admissible for every deck
/// the caller needs found; `floor` is an objective K known decks reach.
fn search_regime(
    pool: &CardPool,
    ctx: &SearchContext,
    suffix: &SuffixBound,
    params: &SearchParams,
    floor: u64,
    deadline: &mut ChallengeDeadline,
) -> (Vec<DeckResult>, crate::search::SearchStats) {
    if pool.count() < DECK_SIZE || deadline.expired_sampled() {
        return (Vec::new(), crate::search::SearchStats::default());
    }

    let mut tracker = TopKTracker::with_floor(params.top_k, floor);
    let mut deck = [CardIdx::new(0); DECK_SIZE];
    let mut stats = crate::search::SearchStats::default();
    let candidates = ordered_candidates(pool, ctx);
    if candidates.len() < DECK_SIZE {
        return (Vec::new(), crate::search::SearchStats::default());
    }
    // Maximization ceilings cannot prune a minimum-power search.
    let minimize = ctx.minimize && matches!(ctx.target, ScoreTarget::Power);
    let bounds = if minimize
        || !crate::search::tuning::SearchTuning::load().bounds
        || ctx.has_event()
        || matches!(ctx.target, ScoreTarget::Bonus | ScoreTarget::Mysekai)
    {
        None
    } else {
        let Some(bounds) = ChallengeBounds::build(pool, &candidates, deadline) else {
            return (Vec::new(), stats);
        };
        Some(bounds)
    };

    challenge_recurse(
        pool,
        ctx,
        suffix,
        &candidates,
        bounds.as_ref(),
        0,
        0,
        &mut deck,
        PartialDeck::default(),
        &mut tracker,
        &mut stats,
        deadline,
    );

    (tracker.into_vec(), stats)
}

#[inline(always)]
fn leaf_evaluate_challenge(
    pool: &CardPool,
    ctx: &SearchContext,
    deck: &[CardIdx; DECK_SIZE],
) -> Option<DeckResult> {
    if crate::search::problem::DeckProblem::from_context(ctx).needs_placement_search() {
        return crate::search::placement::evaluate_candidate(pool, ctx, deck);
    }
    let deck = &crate::search::placement::exchangeable_order(pool, ctx, deck);
    let score = if matches!(
        ctx.effective_live_type(),
        LiveType::Challenge | LiveType::ChallengeAuto
    ) && matches!(ctx.target, ScoreTarget::Score)
        && !ctx.has_event()
    {
        leaf_evaluate_challenge_score_checked(pool, ctx, deck)
    } else {
        leaf_evaluate_checked(pool, ctx, deck)
    }?;
    Some(DeckResult::new(*deck, score))
}

#[allow(clippy::too_many_arguments)]
fn challenge_recurse(
    pool: &CardPool,
    ctx: &SearchContext,
    suffix: &SuffixBound,
    candidates: &[CardIdx],
    bounds: Option<&ChallengeBounds>,
    depth: usize,
    start: usize,
    deck: &mut [CardIdx; DECK_SIZE],
    partial: PartialDeck,
    tracker: &mut TopKTracker,
    stats: &mut crate::search::SearchStats,
    deadline: &mut ChallengeDeadline,
) {
    stats.visited_nodes += 1;
    if deadline.expired_sampled() {
        return;
    }
    if depth == DECK_SIZE {
        stats.leaf_nodes += 1;
        if let Some(candidate) = leaf_evaluate_challenge(pool, ctx, deck) {
            tracker.insert(pool, ctx, candidate);
        }
        return;
    }

    let remaining = DECK_SIZE - depth;
    let threshold = tracker.cutoff();
    // Equal-score branches can still improve the tracker's card-order tie-break.
    if let (Some(bounds), Some(threshold)) = (bounds, threshold)
        && bounds.below(
            suffix,
            ctx,
            pool,
            &deck[..depth],
            start,
            &partial,
            threshold,
        )
    {
        stats.ub_prunes += 1;
        return;
    }

    let mut dense = start;
    while dense < candidates.len() {
        if deadline.expired_sampled() {
            return;
        }
        let card = candidates[dense];
        dense += 1;

        // game_id 去重：同卡多技能变体只取其一
        if game_id_in_deck(pool, deck, depth, card) {
            continue;
        }

        // 剩余卡不够填满槽位时提前退出
        if candidates.len() - dense < remaining - 1 {
            break;
        }
        if !slot_matches(ctx, pool, depth, card) {
            continue;
        }

        let next_partial = PartialDeck {
            power: partial.power + pool.power_max(card),
            skill: partial.skill + pool.skill_max(card) as u32,
            bonus: partial.bonus,
            max_skill: partial.max_skill.max(pool.skill_max(card)),
        };
        deck[depth] = card;
        if let (Some(bounds), Some(threshold)) = (bounds, threshold)
            && bounds.below(
                suffix,
                ctx,
                pool,
                &deck[..=depth],
                dense,
                &next_partial,
                threshold,
            )
        {
            stats.ep_continue_prunes += 1;
            continue;
        }

        challenge_recurse(
            pool,
            ctx,
            suffix,
            candidates,
            bounds,
            depth + 1,
            dense,
            deck,
            next_partial,
            tracker,
            stats,
            deadline,
        );
    }
}

fn ordered_candidates(pool: &CardPool, ctx: &SearchContext) -> Vec<CardIdx> {
    let mut all = pool.indices().collect::<Vec<_>>();
    if ctx.fixed_card_ids.is_empty() {
        sort_candidates(pool, &mut all);
        return all;
    }

    let mut ordered = Vec::with_capacity(all.len());
    for fixed_gid in &ctx.fixed_card_ids {
        let mut group = all
            .iter()
            .copied()
            .filter(|card| pool.game_id(*card) == *fixed_gid)
            .collect::<Vec<_>>();
        sort_candidates(pool, &mut group);
        ordered.extend(group);
    }

    all.retain(|card| !ctx.fixed_card_ids.contains(&pool.game_id(*card)));
    sort_candidates(pool, &mut all);
    ordered.extend(all);
    ordered
}

fn sort_candidates(pool: &CardPool, cards: &mut [CardIdx]) {
    cards.sort_unstable_by(|left, right| {
        candidate_key(pool, *right)
            .cmp(&candidate_key(pool, *left))
            .then_with(|| pool.game_id(*left).cmp(&pool.game_id(*right)))
    });
}

#[inline(always)]
fn candidate_key(pool: &CardPool, card: CardIdx) -> (u32, u8) {
    (pool.power_max(card), pool.skill_max(card))
}

#[inline(always)]
fn slot_matches(ctx: &SearchContext, pool: &CardPool, depth: usize, card: CardIdx) -> bool {
    ctx.fixed_card_at(depth)
        .is_none_or(|fixed_gid| pool.game_id(card) == fixed_gid)
}

struct ChallengeBounds {
    frontiers: Vec<Vec<Vec<BoundState>>>,
    /// The largest powers and score-ups among the candidates from each
    /// position, largest first.
    top_power: Vec<[u32; DECK_SIZE]>,
    top_skill: Vec<[u32; DECK_SIZE]>,
}

impl ChallengeBounds {
    fn build(
        pool: &CardPool,
        candidates: &[CardIdx],
        deadline: &mut ChallengeDeadline,
    ) -> Option<Self> {
        Self::build_with_clock(pool, candidates, deadline, Instant::now)
    }

    fn build_with_clock(
        pool: &CardPool,
        candidates: &[CardIdx],
        deadline: &mut ChallengeDeadline,
        mut now: impl FnMut() -> Instant,
    ) -> Option<Self> {
        if deadline.expired_sampled_with(&mut now) {
            return None;
        }
        let count = candidates.len();
        let mut frontiers = vec![vec![Vec::<BoundState>::new(); DECK_SIZE + 1]; count + 1];
        frontiers[count][0].push(BoundState::default());

        for dense in (0..count).rev() {
            if deadline.expired_sampled_with(&mut now) {
                return None;
            }
            let card = candidates[dense];
            let card_state = BoundState {
                power: pool.power_max(card),
                skill: pool.skill_max(card) as u32,
                leader: pool.skill_max(card) as u16,
            };

            for slot in 0..=DECK_SIZE {
                let mut states = Vec::new();
                for &state in &frontiers[dense + 1][slot] {
                    if deadline.expired_sampled_with(&mut now) {
                        return None;
                    }
                    states.push(state);
                }
                if slot > 0 {
                    for &state in &frontiers[dense + 1][slot - 1] {
                        if deadline.expired_sampled_with(&mut now) {
                            return None;
                        }
                        states.push(state.add(card_state));
                    }
                }
                prune_dominated(&mut states, deadline, &mut now)?;
                frontiers[dense][slot] = states;
            }
        }

        let mut top_power = vec![[0u32; DECK_SIZE]; count + 1];
        let mut top_skill = vec![[0u32; DECK_SIZE]; count + 1];
        for dense in (0..count).rev() {
            let card = candidates[dense];
            top_power[dense] = top_power[dense + 1];
            insert_descending(&mut top_power[dense], pool.power_max(card));
            top_skill[dense] = top_skill[dense + 1];
            insert_descending(&mut top_skill[dense], u32::from(pool.skill_max(card)));
        }
        Some(Self {
            frontiers,
            top_power,
            top_skill,
        })
    }

    /// Whether every completion of the `chosen` members with the rest drawn
    /// from the candidates from `start` stays below `threshold`.
    #[allow(clippy::too_many_arguments)]
    #[inline(always)]
    fn below(
        &self,
        suffix: &SuffixBound,
        ctx: &SearchContext,
        pool: &CardPool,
        chosen: &[CardIdx],
        start: usize,
        partial: &PartialDeck,
        threshold: u64,
    ) -> bool {
        let slots = DECK_SIZE - chosen.len();
        (matches!(ctx.target, ScoreTarget::Score)
            && !matches!(
                ctx.effective_live_type(),
                LiveType::Multi | LiveType::Cheerful | LiveType::Mysekai
            )
            && self.ranked_score_ceiling(suffix, pool, chosen, start, partial.power) < threshold)
            || self.ceiling(suffix, start, partial, slots) < threshold
    }

    /// Score ceiling from the chosen members' score-ups and, rank by rank,
    /// the largest powers and score-ups among the candidates from `start`.
    /// The members' score-ups, largest first, are at most these values rank
    /// by rank, and the leader, one of them, fills the sixth slot, so the
    /// six slots are at most the largest value twice followed by the rest.
    #[inline(always)]
    fn ranked_score_ceiling(
        &self,
        suffix: &SuffixBound,
        pool: &CardPool,
        chosen: &[CardIdx],
        start: usize,
        chosen_power: u32,
    ) -> u64 {
        let slots = DECK_SIZE - chosen.len();
        let (Some(top_power), Some(top_skill)) =
            (self.top_power.get(start), self.top_skill.get(start))
        else {
            return 0;
        };
        let power = chosen_power + top_power[..slots].iter().sum::<u32>();
        let mut values = [0u32; DECK_SIZE];
        for (value, &card) in values.iter_mut().zip(chosen) {
            *value = u32::from(pool.skill_max(card));
        }
        values[chosen.len()..].copy_from_slice(&top_skill[..slots]);
        values.sort_unstable_by(|left, right| right.cmp(left));
        let slot_values = [
            values[0], values[0], values[1], values[2], values[3], values[4],
        ];
        suffix
            .objective()
            .score_ceiling_from_slots(power, 0, &slot_values)
    }

    #[inline(always)]
    fn ceiling(
        &self,
        suffix: &SuffixBound,
        start: usize,
        partial: &PartialDeck,
        slots: usize,
    ) -> u64 {
        let Some(states) = self
            .frontiers
            .get(start)
            .and_then(|by_slot| by_slot.get(slots))
        else {
            return 0;
        };
        let mut best = 0u64;
        for state in states {
            let ceiling = suffix.objective().ceiling(
                partial.power + state.power,
                0,
                partial.skill + state.skill,
                (partial.max_skill as u32).max(state.leader as u32),
            );
            best = best.max(ceiling);
        }
        best
    }
}

fn insert_descending(values: &mut [u32; DECK_SIZE], value: u32) {
    let mut at = DECK_SIZE;
    while at > 0 && values[at - 1] < value {
        at -= 1;
    }
    if at < DECK_SIZE {
        values[at..].rotate_right(1);
        values[at] = value;
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct BoundState {
    power: u32,
    skill: u32,
    leader: u16,
}

impl BoundState {
    #[inline(always)]
    fn add(self, other: BoundState) -> Self {
        Self {
            power: self.power + other.power,
            skill: self.skill + other.skill,
            leader: self.leader.max(other.leader),
        }
    }
}

fn prune_dominated(
    states: &mut Vec<BoundState>,
    deadline: &mut ChallengeDeadline,
    now: &mut impl FnMut() -> Instant,
) -> Option<()> {
    let mut pruned = Vec::with_capacity(states.len());
    'candidate: for (idx, candidate) in states.iter().copied().enumerate() {
        for (other_idx, other) in states.iter().copied().enumerate() {
            if deadline.expired_sampled_with(&mut *now) {
                return None;
            }
            // Identical upper-bound states are interchangeable; keep the first.
            // This deduplicates only the bound frontier, never actual card sets.
            if (other_idx < idx && other == candidate)
                || (idx != other_idx && dominates(other, candidate))
            {
                continue 'candidate;
            }
        }
        pruned.push(candidate);
    }
    *states = pruned;
    Some(())
}

#[inline(always)]
fn dominates(left: BoundState, right: BoundState) -> bool {
    left.power >= right.power
        && left.skill >= right.skill
        && left.leader >= right.leader
        && (left.power > right.power || left.skill > right.skill || left.leader > right.leader)
}

#[inline(always)]
fn game_id_in_deck(
    pool: &CardPool,
    deck: &[CardIdx; DECK_SIZE],
    depth: usize,
    card: CardIdx,
) -> bool {
    let gid = pool.game_id(card);
    let mut i = 0;
    while i < depth {
        if pool.game_id(deck[i]) == gid {
            return true;
        }
        i += 1;
    }
    false
}

/// One character's independent Top-K, including its actual completion record.
#[derive(Clone, Debug)]
pub struct CharacterSearchOutcome {
    /// The requested character ID.
    pub character_id: u8,
    /// Legal incumbents and work for this character, not a fresh request budget.
    pub outcome: SearchOutcome<Vec<DeckResult>>,
    /// Diagnostic elapsed time; never used to infer completion.
    pub search_time: Duration,
}

/// Search every requested character under ONE operation deadline.
///
/// The budget starts before suffix preparation. Unvisited feasible characters
/// after expiry are explicitly `TimedOut`, not mislabeled as infeasible. Entries
/// with fewer than five candidates or `top_k == 0` are trivially complete.
pub fn search_characters_outcome(
    pool: &CardPool,
    ctx: &SearchContext,
    params: &SearchParams,
    characters: &[u8],
) -> SearchOutcome<Vec<CharacterSearchOutcome>> {
    let mut deadline = ChallengeDeadline::from_params(params);
    let suffix = SuffixBound::build(pool, ctx);
    let mut entries = Vec::with_capacity(characters.len());
    let mut total = SearchStats::default();
    for &character_id in characters {
        let started = Instant::now();
        let candidate_count = pool
            .indices()
            .filter(|&c| pool.char_id(c) == character_id)
            .count();
        let (results, mut stats) = if params.top_k == 0 || candidate_count < DECK_SIZE {
            (Vec::new(), SearchStats::default())
        } else if deadline.expired() {
            (
                Vec::new(),
                SearchStats {
                    deadline_hit: true,
                    ..Default::default()
                },
            )
        } else {
            let (results, mut stats) = search_with_character_filter(
                pool,
                ctx,
                &suffix,
                params,
                Some(character_id),
                &mut deadline,
            );
            stats.deadline_hit |= deadline.hit;
            (results, stats)
        };
        stats.finalize();
        total.accumulate(&stats);
        entries.push(CharacterSearchOutcome {
            character_id,
            outcome: SearchOutcome::new(results, stats),
            search_time: started.elapsed(),
        });
    }
    SearchOutcome::new(entries, total)
}
