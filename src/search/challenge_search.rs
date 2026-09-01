use std::time::Duration;
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;
#[cfg(target_arch = "wasm32")]
use web_time::Instant;

use crate::pool::{CardIdx, CardPool};
use crate::types::DECK_SIZE;

use super::context::SearchContext;
use super::dfs::TopKTracker;
use super::evaluate::{leaf_evaluate_challenge_score_checked, leaf_evaluate_checked};
use super::suffix::{PartialDeck, SuffixBound};
use super::types::{DeckResult, SearchParams};
use crate::types::{LiveType, ScoreTarget};

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
    search_with_character_filter(pool, ctx, suffix, params, None)
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
    search_with_character_filter(pool, ctx, suffix, params, Some(character_id))
}

/// challenge_all：逐角色搜索后按分数归并出全局 Top-K。
///
/// 挑战 live 的队伍必须五张同角色，所以答案集是各角色最优解的并集，而不是
/// 在混角色池上做一次无约束搜索——后者既会产出非法卡组，组合数也高数个量级。
/// `timeout_ms` 在角色之间检查，超时后返回已搜完角色的结果。
pub fn search_all_characters(
    pool: &CardPool,
    ctx: &SearchContext,
    suffix: &SuffixBound,
    params: &SearchParams,
) -> (Vec<DeckResult>, super::SearchStats) {
    let deadline =
        (params.timeout_ms != 0).then(|| Instant::now() + Duration::from_millis(params.timeout_ms));

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
        if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            break;
        }
        let (results, character_stats) =
            search_character(pool, ctx, suffix, params, character_id as u8);
        accumulate_stats(&mut stats, &character_stats);
        merged.extend(results);
    }

    merged.sort_unstable_by(super::deck_result_cmp);
    merged.truncate(params.top_k);
    (merged, stats)
}

fn accumulate_stats(total: &mut super::SearchStats, part: &super::SearchStats) {
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
) -> (Vec<DeckResult>, super::SearchStats) {
    if params.top_k == 0 || pool.count() < DECK_SIZE {
        return (Vec::new(), super::SearchStats::default());
    }

    let mut tracker = TopKTracker::new(params.top_k, pool);
    let mut deck = [CardIdx::new(0); DECK_SIZE];
    let mut stats = super::SearchStats::default();
    let candidates = ordered_candidates(pool, ctx, character_id);
    if candidates.len() < DECK_SIZE {
        return (Vec::new(), super::SearchStats::default());
    }
    if params.top_k == 1 && ctx.fixed_card_ids.is_empty() {
        return search_combo_top1(pool, ctx, &candidates);
    }
    let bounds = ChallengeBounds::build(pool, &candidates);

    challenge_recurse(
        pool,
        ctx,
        suffix,
        &candidates,
        &bounds,
        0,
        0,
        &mut deck,
        PartialDeck::default(),
        &mut tracker,
        &mut stats,
    );

    (tracker.into_vec(), stats)
}

fn search_combo_top1(
    pool: &CardPool,
    ctx: &SearchContext,
    candidates: &[CardIdx],
) -> (Vec<DeckResult>, super::SearchStats) {
    let mut best_score = 0u64;
    let mut best_deck = None;
    let mut stats = super::SearchStats::default();
    let game_ids = candidates
        .iter()
        .map(|card| pool.game_id(*card))
        .collect::<Vec<_>>();
    let len = candidates.len();

    for a in 0..len - 4 {
        let gid_a = game_ids[a];
        for b in a + 1..len - 3 {
            let gid_b = game_ids[b];
            if gid_b == gid_a {
                continue;
            }
            for c in b + 1..len - 2 {
                let gid_c = game_ids[c];
                if gid_c == gid_a || gid_c == gid_b {
                    continue;
                }
                for d in c + 1..len - 1 {
                    let gid_d = game_ids[d];
                    if gid_d == gid_a || gid_d == gid_b || gid_d == gid_c {
                        continue;
                    }
                    for e in d + 1..len {
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
                        if let Some(score) = leaf_evaluate_challenge(pool, ctx, &deck)
                            && score > best_score
                        {
                            best_score = score;
                            best_deck = Some(deck);
                        }
                    }
                }
            }
        }
    }

    let results = best_deck
        .map(|deck| vec![DeckResult::new(deck, best_score)])
        .unwrap_or_default();
    (results, stats)
}

#[inline(always)]
fn leaf_evaluate_challenge(
    pool: &CardPool,
    ctx: &SearchContext,
    deck: &[CardIdx; DECK_SIZE],
) -> Option<u64> {
    if matches!(
        ctx.effective_live_type(),
        LiveType::Challenge | LiveType::ChallengeAuto
    ) && matches!(ctx.target, ScoreTarget::Score)
    {
        leaf_evaluate_challenge_score_checked(pool, ctx, deck)
    } else {
        leaf_evaluate_checked(pool, ctx, deck)
    }
}

#[allow(clippy::too_many_arguments)]
fn challenge_recurse(
    pool: &CardPool,
    ctx: &SearchContext,
    suffix: &SuffixBound,
    candidates: &[CardIdx],
    bounds: &ChallengeBounds,
    depth: usize,
    start: usize,
    deck: &mut [CardIdx; DECK_SIZE],
    partial: PartialDeck,
    tracker: &mut TopKTracker,
    stats: &mut super::SearchStats,
) {
    if depth == DECK_SIZE {
        stats.leaf_nodes += 1;
        if let Some(score) = leaf_evaluate_challenge(pool, ctx, deck)
            && score > tracker.threshold()
        {
            tracker.insert(DeckResult::new(*deck, score));
        }
        return;
    }

    let remaining = DECK_SIZE - depth;
    let threshold = tracker.threshold();
    if threshold != 0 && bounds.ceiling(suffix, start, &partial, remaining) <= threshold {
        stats.ub_prunes += 1;
        return;
    }

    let mut dense = start;
    while dense < candidates.len() {
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
        if threshold != 0
            && bounds.ceiling(suffix, dense, &next_partial, remaining - 1) <= threshold
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
    fn build(pool: &CardPool, candidates: &[CardIdx]) -> Self {
        let count = candidates.len();
        let mut frontiers = vec![vec![Vec::<BoundState>::new(); DECK_SIZE + 1]; count + 1];
        frontiers[count][0].push(BoundState::default());

        let mut dense = count;
        while dense > 0 {
            dense -= 1;
            let card = candidates[dense];
            let card_state = BoundState {
                power: pool.power_max(card),
                skill: pool.skill_max(card) as u32,
                leader: pool.skill_max(card) as u16,
            };

            let mut slot = 0usize;
            while slot <= DECK_SIZE {
                frontiers[dense][slot] = frontiers[dense + 1][slot].clone();
                slot += 1;
            }

            slot = 1;
            while slot <= DECK_SIZE {
                let additions = frontiers[dense + 1][slot - 1]
                    .iter()
                    .map(|state| state.add(card_state))
                    .collect::<Vec<_>>();
                frontiers[dense][slot].extend(additions);
                prune_dominated(&mut frontiers[dense][slot]);
                slot += 1;
            }
        }

        Self { frontiers }
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

fn prune_dominated(states: &mut Vec<BoundState>) {
    let mut pruned = Vec::with_capacity(states.len());
    'candidate: for (idx, candidate) in states.iter().copied().enumerate() {
        for (other_idx, other) in states.iter().copied().enumerate() {
            if idx != other_idx && dominates(other, candidate) {
                continue 'candidate;
            }
        }
        pruned.push(candidate);
    }
    *states = pruned;
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
