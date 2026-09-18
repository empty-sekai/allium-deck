use std::time::Duration;
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;
#[cfg(target_arch = "wasm32")]
use web_time::Instant;

use crate::pool::{CardIdx, CardPool};
use crate::types::DECK_SIZE;

use super::SimpleTopKTracker;
use super::context::SearchContext;
use super::evaluate::{leaf_evaluate_challenge_score_checked, leaf_evaluate_checked};
use super::suffix::{PartialDeck, SuffixBound};
use super::types::{DeckResult, SearchParams};
use crate::types::{LiveType, ScoreTarget};

struct ChallengeDeadline {
    expires_at: Option<Instant>,
    checks: u16,
    hit: bool,
}

impl ChallengeDeadline {
    fn new(expires_at: Option<Instant>) -> Self {
        Self {
            expires_at,
            checks: 1023,
            hit: false,
        }
    }

    fn from_params(params: &SearchParams) -> Self {
        Self::new(
            (params.timeout_ms != 0)
                .then(|| Instant::now() + Duration::from_millis(params.timeout_ms)),
        )
    }

    #[inline]
    fn expired(&mut self) -> bool {
        self.expired_with(Instant::now)
    }

    #[inline]
    fn expired_with(&mut self, now: impl FnOnce() -> Instant) -> bool {
        if self.hit {
            return true;
        }
        let Some(expires_at) = self.expires_at else {
            return false;
        };
        self.checks = self.checks.wrapping_add(1);
        if self.checks & 1023 == 0 {
            self.hit = now() >= expires_at;
        }
        self.hit
    }
}

#[cfg(test)]
mod deadline_tests {
    use super::*;

    #[test]
    fn challenge_deadline_samples_and_stays_expired() {
        let start = Instant::now();
        let end = start + Duration::from_secs(1);
        let mut guard = ChallengeDeadline::new(Some(end));
        assert!(!guard.expired_with(|| start));
        for _ in 0..1023 {
            assert!(!guard.expired_with(|| panic!("unexpected clock read")));
        }
        assert!(guard.expired_with(|| end));
        for _ in 0..2048 {
            assert!(guard.expired_with(|| panic!("expired guard read the clock")));
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
        assert!(expired.expired_with(|| now));
        let mut disabled = ChallengeDeadline::new(None);
        for _ in 0..2048 {
            assert!(!disabled.expired_with(|| panic!("disabled deadline read the clock")));
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
) -> (Vec<DeckResult>, super::SearchStats) {
    let mut deadline = ChallengeDeadline::from_params(params);
    let (results, mut stats) =
        search_with_character_filter(pool, ctx, suffix, params, None, &mut deadline);
    stats.deadline_hit |= deadline.hit;
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
) -> (Vec<DeckResult>, super::SearchStats) {
    let mut deadline = ChallengeDeadline::from_params(params);
    let (results, mut stats) =
        search_with_character_filter(pool, ctx, suffix, params, Some(character_id), &mut deadline);
    stats.deadline_hit |= deadline.hit;
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
) -> (Vec<DeckResult>, super::SearchStats) {
    let mut deadline = ChallengeDeadline::from_params(params);

    let mut present = [false; 27];
    for card in pool.indices() {
        present[(pool.char_id(card) as usize).min(26)] = true;
    }

    let mut merged = Vec::new();
    let mut stats = super::SearchStats::default();
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
            &mut deadline,
        );
        accumulate_stats(&mut stats, &character_stats);
        merged.extend(results);
    }

    let minimize = ctx.minimize && matches!(ctx.target, ScoreTarget::Power);
    merged.sort_unstable_by(|left, right| {
        let ordering = super::deck_result_cmp(left, right);
        if minimize {
            ordering.reverse()
        } else {
            ordering
        }
    });
    merged.truncate(params.top_k);
    stats.deadline_hit |= deadline.hit;
    (merged, stats)
}

fn accumulate_stats(total: &mut super::SearchStats, part: &super::SearchStats) {
    total.visited_nodes += part.visited_nodes;
    total.deadline_hit |= part.deadline_hit;
    total.leaf_nodes += part.leaf_nodes;
    total.ub_prunes += part.ub_prunes;
    total.leader_prunes += part.leader_prunes;
    total.ep_candidates += part.ep_candidates;
    total.ep_break_prunes += part.ep_break_prunes;
    total.ep_continue_prunes += part.ep_continue_prunes;
    total.ep_explored += part.ep_explored;
    total.mono_break_prunes += part.mono_break_prunes;
}

fn search_with_character_filter(
    pool: &CardPool,
    ctx: &SearchContext,
    suffix: &SuffixBound,
    params: &SearchParams,
    character_id: Option<u8>,
    deadline: &mut ChallengeDeadline,
) -> (Vec<DeckResult>, super::SearchStats) {
    if params.top_k == 0 || pool.count() < DECK_SIZE || deadline.expired() {
        return (Vec::new(), super::SearchStats::default());
    }

    let minimize = ctx.minimize && matches!(ctx.target, ScoreTarget::Power);
    let mut tracker = SimpleTopKTracker::new(params.top_k, minimize, pool);
    let mut deck = [CardIdx::new(0); DECK_SIZE];
    let mut stats = super::SearchStats::default();
    let candidates = ordered_candidates(pool, ctx, character_id);
    if candidates.len() < DECK_SIZE {
        return (Vec::new(), super::SearchStats::default());
    }
    if params.top_k == 1 && ctx.fixed_card_ids.is_empty() {
        return search_combo_top1(pool, ctx, &candidates, tracker, deadline);
    }
    // Maximization ceilings cannot prune a minimum-power search.
    let bounds = if minimize
        || !super::tuning::SearchTuning::load().bounds
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

fn search_combo_top1(
    pool: &CardPool,
    ctx: &SearchContext,
    candidates: &[CardIdx],
    mut tracker: SimpleTopKTracker,
    deadline: &mut ChallengeDeadline,
) -> (Vec<DeckResult>, super::SearchStats) {
    let mut stats = super::SearchStats::default();
    let game_ids = candidates
        .iter()
        .map(|card| pool.game_id(*card))
        .collect::<Vec<_>>();
    let len = candidates.len();

    'search: for a in 0..len - 4 {
        if deadline.expired() {
            break;
        }
        let gid_a = game_ids[a];
        for b in a + 1..len - 3 {
            if deadline.expired() {
                break 'search;
            }
            let gid_b = game_ids[b];
            if gid_b == gid_a {
                continue;
            }
            for c in b + 1..len - 2 {
                if deadline.expired() {
                    break 'search;
                }
                let gid_c = game_ids[c];
                if gid_c == gid_a || gid_c == gid_b {
                    continue;
                }
                for d in c + 1..len - 1 {
                    if deadline.expired() {
                        break 'search;
                    }
                    let gid_d = game_ids[d];
                    if gid_d == gid_a || gid_d == gid_b || gid_d == gid_c {
                        continue;
                    }
                    for e in d + 1..len {
                        if deadline.expired() {
                            break 'search;
                        }
                        let gid_e = game_ids[e];
                        if gid_e == gid_a || gid_e == gid_b || gid_e == gid_c || gid_e == gid_d {
                            continue;
                        }
                        let deck = [
                            candidates[a],
                            candidates[b],
                            candidates[c],
                            candidates[d],
                            candidates[e],
                        ];
                        stats.leaf_nodes += 1;
                        stats.visited_nodes += 1;
                        if let Some(candidate) = leaf_evaluate_challenge(pool, ctx, &deck) {
                            tracker.insert(candidate);
                        }
                    }
                }
            }
        }
    }

    (tracker.into_vec(), stats)
}

#[inline(always)]
fn leaf_evaluate_challenge(
    pool: &CardPool,
    ctx: &SearchContext,
    deck: &[CardIdx; DECK_SIZE],
) -> Option<DeckResult> {
    if super::problem::DeckProblem::from_context(ctx).needs_placement_search() {
        return super::placement::evaluate_candidate(pool, ctx, deck);
    }
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
    tracker: &mut SimpleTopKTracker,
    stats: &mut super::SearchStats,
    deadline: &mut ChallengeDeadline,
) {
    stats.visited_nodes += 1;
    if deadline.expired() {
        return;
    }
    if depth == DECK_SIZE {
        stats.leaf_nodes += 1;
        if let Some(candidate) = leaf_evaluate_challenge(pool, ctx, deck) {
            tracker.insert(candidate);
        }
        return;
    }

    let remaining = DECK_SIZE - depth;
    let threshold = tracker.threshold();
    // Equal-score branches can still improve the tracker's card-order tie-break.
    if let (Some(bounds), Some(threshold)) = (bounds, threshold)
        && bounds.ceiling(suffix, start, &partial, remaining) < threshold
    {
        stats.ub_prunes += 1;
        return;
    }

    let mut dense = start;
    while dense < candidates.len() {
        if deadline.expired() {
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
            limited_count: partial.limited_count,
        };
        if let (Some(bounds), Some(threshold)) = (bounds, threshold)
            && bounds.ceiling(suffix, dense, &next_partial, remaining - 1) < threshold
        {
            stats.ep_continue_prunes += 1;
            continue;
        }

        deck[depth] = card;
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

fn ordered_candidates(
    pool: &CardPool,
    ctx: &SearchContext,
    character_id: Option<u8>,
) -> Vec<CardIdx> {
    let mut all = pool
        .indices()
        .filter(|card| character_id.is_none_or(|character_id| pool.char_id(*card) == character_id))
        .collect::<Vec<_>>();
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
        if deadline.expired_with(&mut now) {
            return None;
        }
        let count = candidates.len();
        let mut frontiers = vec![vec![Vec::<BoundState>::new(); DECK_SIZE + 1]; count + 1];
        frontiers[count][0].push(BoundState::default());

        for dense in (0..count).rev() {
            if deadline.expired_with(&mut now) {
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
                    if deadline.expired_with(&mut now) {
                        return None;
                    }
                    states.push(state);
                }
                if slot > 0 {
                    for &state in &frontiers[dense + 1][slot - 1] {
                        if deadline.expired_with(&mut now) {
                            return None;
                        }
                        states.push(state.add(card_state));
                    }
                }
                prune_dominated(&mut states, deadline, &mut now)?;
                frontiers[dense][slot] = states;
            }
        }

        Some(Self { frontiers })
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
            let ceiling = suffix.ceiling(
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
            if deadline.expired_with(&mut *now) {
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
