use std::time::Duration;
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;
#[cfg(target_arch = "wasm32")]
use web_time::Instant;

use crate::pool::{CardIdx, CardPool};
use crate::types::{DECK_SIZE, LiveSkillOrder, LiveType, ScoreTarget};

use super::context::{SearchContext, SupportDeck};
use super::dfs::SearchStats;
use super::evaluate::{calc_event_point, decode_u18, leaf_evaluate_checked, resolve_power_target};
use super::suffix::SuffixBound;
use super::types::{DeckResult, SearchParams};

const MEMBER_COUNT: usize = 4;
const FINAL_CHAPTER_AUTO_LEADERS_PER_CHAR: usize = 3;
const FINAL_CHAPTER_SEED_GROUP_PREFIX: usize = 6;
/// 每层最多按上界降序保留的候选卡数。
const RANKED_CAP: usize = 32;

/// `recurse_cards` 排序候选缓冲的单槽：(上界, 卡, 落子后的局部状态)。
type RankedSlot = (u64, CardIdx, CardPartial);
/// 递归热路径每 1024 个节点才真正读一次时钟：`Instant::now()` 在 wasm 上是
/// `performance.now()` 的 JS 调用，逐节点读会主导终章搜索耗时。一旦过线即置粘性
/// 标志，之后所有调用立刻返回 true，整棵递归逐层退出（与 `dfs.rs` 同一约定）。
struct DeadlineGuard {
    deadline: Option<Instant>,
    node_count: u32,
    hit: bool,
}

impl DeadlineGuard {
    fn new(deadline: Option<Instant>) -> Self {
        Self {
            deadline,
            node_count: 0,
            hit: false,
        }
    }

    /// 逐次读时钟：只用于每队长 / 每 job 的外层循环（调用量为个位数量级）。
    #[inline]
    fn expired(&mut self) -> bool {
        if self.hit {
            return true;
        }
        let Some(deadline) = self.deadline else {
            return false;
        };
        if Instant::now() >= deadline {
            self.hit = true;
            return true;
        }
        false
    }

    /// 采样读时钟：用于递归内部的高频调用点。
    #[inline(always)]
    fn expired_sampled(&mut self) -> bool {
        if self.hit {
            return true;
        }
        if self.deadline.is_none() {
            return false;
        }
        self.node_count = self.node_count.wrapping_add(1);
        if self.node_count & 1023 != 0 {
            return false;
        }
        self.expired()
    }
}

#[derive(Clone)]
struct CharGroup {
    char_id: u8,
    cards: Vec<CardIdx>,
    best_power: u32,
    best_skill: u32,
    best_base_bonus: u32,
    best_limited_bonus: u32,
    sort_key: u64,
}

#[derive(Clone, Copy)]
struct LeaderConst {
    leader: CardIdx,
    power: u32,
    skill: u32,
    base_bonus_const: u32,
    limited_bonus: u32,
    limited_count: u8,
    extra_bonus_ub: u32,
}

#[derive(Clone)]
struct AutoLeaderJob {
    group_set: usize,
    leader: LeaderConst,
    ceiling: u64,
}

struct AutoLeaderGroupSet {
    groups: Vec<CharGroup>,
    suffix: Vec<GroupCeilingTail>,
}

#[derive(Clone, Copy)]
struct CardPartial {
    power: u32,
    skill: u32,
    base_bonus: u32,
    limited_values: [u32; MEMBER_COUNT + 1],
    attr_set: u8,
    selected: [u16; DECK_SIZE],
    selected_len: usize,
    support_bonus_sum: f64,
    support_next_scan: usize,
    support_bonus_ceil: u32,
}

#[derive(Clone, Copy)]
struct CardGroupPlan {
    rem_power: [u32; MEMBER_COUNT + 1],
    rem_skill: [u32; MEMBER_COUNT + 1],
    rem_base_bonus: [u32; MEMBER_COUNT + 1],
    rem_limited_values: [[u32; MEMBER_COUNT + 1]; MEMBER_COUNT + 1],
}

#[derive(Clone, Copy, Default)]
struct GroupCeilingTail {
    top_power: [u32; MEMBER_COUNT + 1],
    top_skill: [u32; MEMBER_COUNT + 1],
    top_base_bonus: [u32; MEMBER_COUNT + 1],
    top_limited_bonus: [u32; MEMBER_COUNT + 1],
}

impl CardPartial {
    #[inline(always)]
    fn for_leader(pool: &CardPool, ctx: &SearchContext, leader: &LeaderConst) -> Self {
        let mut limited_values = [0u32; MEMBER_COUNT + 1];
        limited_values[0] = leader.limited_bonus;
        let mut selected = [0u16; DECK_SIZE];
        selected[0] = pool.game_id(leader.leader);
        let leader_char = pool.char_id(leader.leader);
        let (support_bonus_sum, support_next_scan) =
            initial_final_chapter_support_state(ctx, leader_char, &selected, 1);
        let support_bonus_ceil = support_bonus_sum.ceil() as u32;
        Self {
            power: leader.power,
            skill: leader.skill,
            base_bonus: leader.base_bonus_const,
            limited_values,
            attr_set: 1u8 << pool.attr(leader.leader),
            selected,
            selected_len: 1,
            support_bonus_sum,
            support_next_scan,
            support_bonus_ceil,
        }
    }

    #[inline(always)]
    fn with_card(
        &self,
        pool: &CardPool,
        is_world_bloom: bool,
        support: &SupportDeck,
        card: CardIdx,
    ) -> Self {
        let eb = pool.event_bonus_exact(card);
        let mut next = *self;
        next.power += pool.power_max(card);
        next.skill += pool.skill_max(card) as u32;
        next.base_bonus += eb.base_ceil();
        insert_topk_u32(&mut next.limited_values, eb.limited_ceil());
        next.attr_set |= 1u8 << pool.attr(card);
        let game_id = pool.game_id(card);
        let (support_bonus_sum, support_next_scan) = advance_final_chapter_support_state(
            is_world_bloom,
            support,
            self.support_bonus_sum,
            self.support_next_scan,
            &self.selected,
            self.selected_len,
            game_id,
        );
        next.selected[next.selected_len] = game_id;
        next.selected_len += 1;
        next.support_bonus_sum = support_bonus_sum;
        next.support_next_scan = support_next_scan;
        next.support_bonus_ceil = support_bonus_sum.ceil() as u32;
        next
    }
}

fn build_card_group_plan(groups: &[CharGroup], selected: &[usize; MEMBER_COUNT]) -> CardGroupPlan {
    let mut plan = CardGroupPlan {
        rem_power: [0; MEMBER_COUNT + 1],
        rem_skill: [0; MEMBER_COUNT + 1],
        rem_base_bonus: [0; MEMBER_COUNT + 1],
        rem_limited_values: [[0; MEMBER_COUNT + 1]; MEMBER_COUNT + 1],
    };
    let mut depth = MEMBER_COUNT;
    while depth > 0 {
        depth -= 1;
        let next = depth + 1;
        let group = &groups[selected[depth]];
        plan.rem_power[depth] = plan.rem_power[next] + group.best_power;
        plan.rem_skill[depth] = plan.rem_skill[next] + group.best_skill;
        plan.rem_base_bonus[depth] = plan.rem_base_bonus[next] + group.best_base_bonus;
        plan.rem_limited_values[depth] = plan.rem_limited_values[next];
        insert_topk_u32(
            &mut plan.rem_limited_values[depth],
            group.best_limited_bonus,
        );
    }
    plan
}

fn build_group_ceiling_suffix(groups: &[CharGroup]) -> Vec<GroupCeilingTail> {
    let mut suffix = vec![GroupCeilingTail::default(); groups.len() + 1];
    let mut idx = groups.len();
    while idx > 0 {
        idx -= 1;
        let group = &groups[idx];
        let mut tail = suffix[idx + 1];
        insert_topk_u32(&mut tail.top_power, group.best_power);
        insert_topk_u32(&mut tail.top_skill, group.best_skill);
        insert_topk_u32(&mut tail.top_base_bonus, group.best_base_bonus);
        insert_topk_u32(&mut tail.top_limited_bonus, group.best_limited_bonus);
        suffix[idx] = tail;
    }
    suffix
}

pub(crate) fn search_fixed_leader(
    pool: &CardPool,
    ctx: &SearchContext,
    params: &SearchParams,
) -> (Vec<DeckResult>, SearchStats) {
    if params.top_k == 0 || pool.count() < DECK_SIZE {
        return (Vec::new(), SearchStats::default());
    }
    let Some(leader_char) = ctx.final_chapter_leader_character() else {
        return (Vec::new(), SearchStats::default());
    };
    search_leaders(pool, ctx, params, Some(leader_char))
}

pub(crate) fn search_auto_leader(
    pool: &CardPool,
    ctx: &SearchContext,
    params: &SearchParams,
) -> (Vec<DeckResult>, SearchStats) {
    search_leaders(pool, ctx, params, None)
}

fn search_leaders(
    pool: &CardPool,
    ctx: &SearchContext,
    params: &SearchParams,
    leader_char_filter: Option<u8>,
) -> (Vec<DeckResult>, SearchStats) {
    if params.top_k == 0 || pool.count() < DECK_SIZE {
        return (Vec::new(), SearchStats::default());
    }

    let deadline = if params.timeout_ms == 0 {
        None
    } else {
        Some(Instant::now() + Duration::from_millis(params.timeout_ms))
    };
    let mut guard = DeadlineGuard::new(deadline);
    let suffix = SuffixBound::build(pool, ctx);
    // member 位图由 search_instrumented 统一计算（含支援惩罚维度与替代记录），
    // 经 ctx 透传；空位图等价全保留。
    let member_keep = ctx.final_chapter_member_keep.clone();
    let mut tracker = TopKTracker::new(params.top_k, pool);
    let mut stats = SearchStats::default();
    if leader_char_filter.is_none() {
        return search_auto_leaders_two_phase(
            pool,
            ctx,
            params,
            &suffix,
            &member_keep,
            &mut guard,
            tracker,
            stats,
        );
    }
    let mut leader_chars = Vec::new();
    if let Some(leader_char) = leader_char_filter {
        leader_chars.push(leader_char);
    } else {
        for character_id in 1..=26 {
            leader_chars.push(character_id);
        }
    }

    for leader_char in leader_chars {
        let groups = build_char_groups(pool, ctx, leader_char, &member_keep, params.top_k);
        if groups.len() < MEMBER_COUNT {
            continue;
        }
        let group_suffix = build_group_ceiling_suffix(&groups);
        let mut leaders = pool
            .indices()
            .filter(|card| pool.char_id(*card) == leader_char)
            .collect::<Vec<_>>();
        leaders.sort_unstable_by(|left, right| {
            final_chapter_card_key(pool, *right)
                .cmp(&final_chapter_card_key(pool, *left))
                .then_with(|| left.raw().cmp(&right.raw()))
        });
        let mut leaders = filter_leader_variants(pool, ctx, leaders);
        if leader_char_filter.is_none() && leaders.len() > FINAL_CHAPTER_AUTO_LEADERS_PER_CHAR {
            leaders.truncate(FINAL_CHAPTER_AUTO_LEADERS_PER_CHAR);
        }

        for leader in leaders {
            if guard.expired() {
                break;
            }
            let leader_const = build_leader_const(pool, ctx, leader);
            seed_leader_groups(pool, ctx, &groups, &leader_const, &mut tracker);
            let threshold = tracker.threshold();
            if threshold != 0 {
                let ub =
                    character_ceiling(&suffix, ctx, &groups, &group_suffix, 0, &[], &leader_const);
                if ub <= threshold {
                    stats.leader_prunes += 1;
                    continue;
                }
            }
            let mut selected = [0usize; MEMBER_COUNT];
            let mut state = CharacterSearchState {
                pool,
                ctx,
                suffix: &suffix,
                groups: &groups,
                group_suffix: &group_suffix,
                support: ctx.support_deck_for_leader(leader_char),
                tracker: &mut tracker,
                stats: &mut stats,
                deadline: &mut guard,
                leader: leader_const,
            };
            state.recurse_chars(0, 0, &mut selected);
        }
    }

    (tracker.into_vec(), stats)
}

fn search_auto_leaders_two_phase(
    pool: &CardPool,
    ctx: &SearchContext,
    params: &SearchParams,
    suffix: &SuffixBound,
    member_keep: &[bool],
    guard: &mut DeadlineGuard,
    mut tracker: TopKTracker,
    mut stats: SearchStats,
) -> (Vec<DeckResult>, SearchStats) {
    seed_auto_leader_beam(pool, ctx, member_keep, &mut tracker, &mut stats);

    let mut jobs = Vec::new();
    let mut group_sets = Vec::new();
    for leader_char in 1..=26 {
        let groups = build_char_groups(pool, ctx, leader_char, member_keep, params.top_k);
        if groups.len() < MEMBER_COUNT {
            continue;
        }
        let group_suffix = build_group_ceiling_suffix(&groups);
        let group_set = group_sets.len();
        let mut leaders = pool
            .indices()
            .filter(|card| pool.char_id(*card) == leader_char)
            .collect::<Vec<_>>();
        leaders.sort_unstable_by(|left, right| {
            final_chapter_card_key(pool, *right)
                .cmp(&final_chapter_card_key(pool, *left))
                .then_with(|| left.raw().cmp(&right.raw()))
        });
        let mut leaders = filter_leader_variants(pool, ctx, leaders);
        if leaders.len() > FINAL_CHAPTER_AUTO_LEADERS_PER_CHAR {
            leaders.truncate(FINAL_CHAPTER_AUTO_LEADERS_PER_CHAR);
        }
        for leader in leaders {
            let leader_const = build_leader_const(pool, ctx, leader);
            seed_leader_groups(pool, ctx, &groups, &leader_const, &mut tracker);
            let ceiling =
                character_ceiling(suffix, ctx, &groups, &group_suffix, 0, &[], &leader_const);
            jobs.push(AutoLeaderJob {
                group_set,
                leader: leader_const,
                ceiling,
            });
        }
        group_sets.push(AutoLeaderGroupSet {
            groups,
            suffix: group_suffix,
        });
    }

    jobs.sort_unstable_by(|left, right| {
        right
            .ceiling
            .cmp(&left.ceiling)
            .then_with(|| left.leader.leader.raw().cmp(&right.leader.leader.raw()))
    });
    for job in jobs {
        if guard.expired() {
            break;
        }
        if tracker.threshold() != 0 && job.ceiling <= tracker.threshold() {
            stats.leader_prunes += 1;
            continue;
        }
        let group_set = &group_sets[job.group_set];
        let mut selected = [0usize; MEMBER_COUNT];
        let mut state = CharacterSearchState {
            pool,
            ctx,
            suffix,
            groups: &group_set.groups,
            group_suffix: &group_set.suffix,
            support: ctx.support_deck_for_leader(pool.char_id(job.leader.leader)),
            tracker: &mut tracker,
            stats: &mut stats,
            deadline: guard,
            leader: job.leader,
        };
        state.recurse_chars(0, 0, &mut selected);
    }

    (tracker.into_vec(), stats)
}

#[derive(Clone, Copy)]
struct MemberBeamState {
    cards: [CardIdx; DECK_SIZE],
    len: u8,
    start: usize,
    used_chars: u32,
    key: u64,
}

fn seed_auto_leader_beam(
    pool: &CardPool,
    ctx: &SearchContext,
    member_keep: &[bool],
    tracker: &mut TopKTracker,
    stats: &mut SearchStats,
) {
    const LEADER_LIMIT: usize = 16;
    const MEMBER_LIMIT: usize = 96;
    const BEAM_WIDTH: usize = 256;

    let mut leaders = pool.indices().collect::<Vec<_>>();
    leaders.sort_unstable_by(|left, right| {
        final_chapter_card_key(pool, *right)
            .cmp(&final_chapter_card_key(pool, *left))
            .then_with(|| left.raw().cmp(&right.raw()))
    });
    let leaders = filter_leader_variants(pool, ctx, leaders)
        .into_iter()
        .take(LEADER_LIMIT)
        .collect::<Vec<_>>();

    for leader in leaders {
        seed_auto_leader_beam_for_leader(
            pool,
            ctx,
            member_keep,
            tracker,
            stats,
            leader,
            MEMBER_LIMIT,
            BEAM_WIDTH,
        );
    }
    improve_final_chapter_results(pool, ctx, tracker, stats);
}

#[allow(clippy::too_many_arguments)]
fn seed_auto_leader_beam_for_leader(
    pool: &CardPool,
    ctx: &SearchContext,
    member_keep: &[bool],
    tracker: &mut TopKTracker,
    stats: &mut SearchStats,
    leader: CardIdx,
    member_limit: usize,
    beam_width: usize,
) {
    let leader_char = pool.char_id(leader);
    let candidates =
        final_chapter_beam_candidates(pool, ctx, leader_char, member_keep, member_limit);
    if candidates.len() < MEMBER_COUNT {
        return;
    }
    let mut beam = vec![MemberBeamState {
        cards: [leader; DECK_SIZE],
        len: 0,
        start: 0,
        used_chars: 1u32 << leader_char,
        key: final_chapter_card_key(pool, leader),
    }];
    let mut depth = 0usize;
    while depth < MEMBER_COUNT {
        let mut next = Vec::with_capacity(beam_width.min(beam.len() * candidates.len()));
        for state in &beam {
            let mut idx = state.start;
            while idx < candidates.len() {
                let card = candidates[idx];
                idx += 1;
                let char_id = pool.char_id(card);
                if state.used_chars & (1u32 << char_id) != 0 {
                    continue;
                }
                let mut cards = state.cards;
                cards[state.len as usize + 1] = card;
                next.push(MemberBeamState {
                    cards,
                    len: state.len + 1,
                    start: idx,
                    used_chars: state.used_chars | (1u32 << char_id),
                    key: state.key + final_chapter_member_key(pool, ctx, leader_char, card),
                });
            }
        }
        if next.is_empty() {
            break;
        }
        next.sort_unstable_by(|left, right| {
            right
                .key
                .cmp(&left.key)
                .then_with(|| left.cards.cmp(&right.cards))
        });
        if next.len() > beam_width {
            next.truncate(beam_width);
        }
        beam = next;
        depth += 1;
    }
    if depth != MEMBER_COUNT {
        return;
    }
    for state in beam {
        stats.leaf_nodes += 1;
        if let Some(score) = leaf_evaluate_checked(pool, ctx, &state.cards) {
            tracker.insert(DeckResult::new(state.cards, score));
        }
    }
}

fn improve_final_chapter_results(
    pool: &CardPool,
    ctx: &SearchContext,
    tracker: &mut TopKTracker,
    stats: &mut SearchStats,
) {
    let mut pass = 0usize;
    while pass < 1 {
        let seeds = tracker.results.clone();
        let mut changed = false;
        for seed in seeds {
            changed |= insert_one_swap_variants(pool, ctx, seed, tracker, stats);
        }
        if !changed {
            break;
        }
        pass += 1;
    }
}

fn insert_one_swap_variants(
    pool: &CardPool,
    ctx: &SearchContext,
    seed: DeckResult,
    tracker: &mut TopKTracker,
    stats: &mut SearchStats,
) -> bool {
    const SWAP_CANDIDATE_LIMIT: usize = 128;

    let mut changed = false;
    let mut deck = seed.cards;
    let leader = deck[0];
    let leader_char = pool.char_id(leader);
    let member_keep = vec![true; pool.count()];
    let candidates =
        final_chapter_beam_candidates(pool, ctx, leader_char, &member_keep, SWAP_CANDIDATE_LIMIT);
    let mut slot = 1usize;
    while slot < DECK_SIZE {
        let original = deck[slot];
        for &candidate in &candidates {
            if candidate == leader || pool.char_id(candidate) == leader_char {
                continue;
            }
            let cand_char = pool.char_id(candidate);
            let mut conflict = false;
            let mut idx = 1usize;
            while idx < DECK_SIZE {
                if idx != slot {
                    let current = deck[idx];
                    if current == candidate || pool.char_id(current) == cand_char {
                        conflict = true;
                        break;
                    }
                }
                idx += 1;
            }
            if conflict {
                continue;
            }
            deck[slot] = candidate;
            stats.leaf_nodes += 1;
            if let Some(score) = leaf_evaluate_checked(pool, ctx, &deck) {
                if score > seed.score {
                    changed = true;
                }
                tracker.insert(DeckResult::new(deck, score));
            }
        }
        deck[slot] = original;
        slot += 1;
    }
    changed
}

fn final_chapter_beam_candidates(
    pool: &CardPool,
    ctx: &SearchContext,
    leader_char: u8,
    member_keep: &[bool],
    limit: usize,
) -> Vec<CardIdx> {
    let mut keep = vec![false; pool.count()];
    let mut out = Vec::with_capacity(limit);
    push_final_chapter_candidates(
        pool,
        ctx,
        leader_char,
        member_keep,
        limit,
        &mut keep,
        &mut out,
        final_chapter_member_key,
    );
    push_final_chapter_candidates(
        pool,
        ctx,
        leader_char,
        member_keep,
        limit / 2,
        &mut keep,
        &mut out,
        |pool, _ctx, _leader_char, card| pool.power_max(card) as u64,
    );
    push_final_chapter_candidates(
        pool,
        ctx,
        leader_char,
        member_keep,
        limit / 2,
        &mut keep,
        &mut out,
        |pool, _ctx, _leader_char, card| {
            let eb = pool.event_bonus_exact(card);
            eb.total_x10() as u64 * 1_000_000 + pool.power_max(card) as u64
        },
    );
    push_final_chapter_candidates(
        pool,
        ctx,
        leader_char,
        member_keep,
        limit / 3,
        &mut keep,
        &mut out,
        |pool, _ctx, _leader_char, card| {
            pool.skill_max(card) as u64 * 1_000_000 + pool.power_max(card) as u64
        },
    );
    out
}

fn push_final_chapter_candidates(
    pool: &CardPool,
    ctx: &SearchContext,
    leader_char: u8,
    member_keep: &[bool],
    take: usize,
    keep: &mut [bool],
    out: &mut Vec<CardIdx>,
    key_fn: impl Fn(&CardPool, &SearchContext, u8, CardIdx) -> u64,
) {
    let mut ranked = pool
        .indices()
        .filter(|card| {
            pool.char_id(*card) != leader_char
                && member_keep.get(card.raw()).copied().unwrap_or(true)
        })
        .map(|card| (key_fn(pool, ctx, leader_char, card), card))
        .collect::<Vec<_>>();
    ranked.sort_unstable_by(|left, right| {
        right
            .0
            .cmp(&left.0)
            .then_with(|| left.1.raw().cmp(&right.1.raw()))
    });
    for (_, card) in ranked.into_iter().take(take) {
        let idx = card.raw();
        if keep.get(idx).copied().unwrap_or(false) {
            continue;
        }
        if let Some(slot) = keep.get_mut(idx) {
            *slot = true;
        }
        out.push(card);
    }
}

fn seed_leader_groups(
    pool: &CardPool,
    ctx: &SearchContext,
    groups: &[CharGroup],
    leader: &LeaderConst,
    tracker: &mut TopKTracker,
) {
    let prefix_len = groups.len().min(FINAL_CHAPTER_SEED_GROUP_PREFIX);
    if prefix_len < MEMBER_COUNT {
        return;
    }
    let mut a = 0usize;
    while a + 3 < prefix_len {
        let mut b = a + 1;
        while b + 2 < prefix_len {
            let mut c = b + 1;
            while c + 1 < prefix_len {
                let mut d = c + 1;
                while d < prefix_len {
                    let indices = [a, b, c, d];
                    let mut deck = [leader.leader; DECK_SIZE];
                    let mut slot = 0usize;
                    while slot < MEMBER_COUNT {
                        deck[slot + 1] = groups[indices[slot]].cards[0];
                        slot += 1;
                    }
                    if let Some(score) = exact_final_chapter_leaf(pool, ctx, &deck) {
                        tracker.insert(DeckResult::new(deck, score));
                    }
                    let mut variant = 0usize;
                    while variant < MEMBER_COUNT {
                        let group = &groups[indices[variant]];
                        if group.cards.len() > 1 {
                            let mut alt = deck;
                            alt[variant + 1] = group.cards[1];
                            if let Some(score) = exact_final_chapter_leaf(pool, ctx, &alt) {
                                tracker.insert(DeckResult::new(alt, score));
                            }
                        }
                        variant += 1;
                    }
                    d += 1;
                }
                c += 1;
            }
            b += 1;
        }
        a += 1;
    }
}

fn filter_leader_variants(
    pool: &CardPool,
    ctx: &SearchContext,
    leaders: Vec<CardIdx>,
) -> Vec<CardIdx> {
    let mut keep = vec![true; leaders.len()];
    let mut left = 0usize;
    while left < leaders.len() {
        if !keep[left] {
            left += 1;
            continue;
        }
        let lhs = leaders[left];
        let mut right = 0usize;
        while right < leaders.len() {
            if left != right && keep[right] {
                let rhs = leaders[right];
                if leader_dominates(pool, ctx, lhs, rhs) {
                    keep[right] = false;
                }
            }
            right += 1;
        }
        left += 1;
    }
    leaders
        .into_iter()
        .zip(keep)
        .filter_map(|(leader, keep)| keep.then_some(leader))
        .collect()
}

fn leader_dominates(pool: &CardPool, ctx: &SearchContext, lhs: CardIdx, rhs: CardIdx) -> bool {
    let lhs_values = pool.power_values(lhs);
    let rhs_values = pool.power_values(rhs);
    let lhs_lut = pool.power_lut(lhs);
    let rhs_lut = pool.power_lut(rhs);
    let mut idx = 0usize;
    while idx < 8 {
        if decode_u18(lhs_values, lhs_lut, idx) < decode_u18(rhs_values, rhs_lut, idx) {
            return false;
        }
        idx += 1;
    }

    let lhs_skill = pool.skill(lhs);
    let rhs_skill = pool.skill(rhs);
    if lhs_skill.skill_type != rhs_skill.skill_type || lhs_skill.value < rhs_skill.value {
        return false;
    }

    let lhs_bonus = pool.event_bonus_exact(lhs);
    let rhs_bonus = pool.event_bonus_exact(rhs);
    if lhs_bonus.base_x10() < rhs_bonus.base_x10()
        || lhs_bonus.limited_x10() < rhs_bonus.limited_x10()
    {
        return false;
    }
    if ctx.leader_honor_bonus_at(lhs.raw()) < ctx.leader_honor_bonus_at(rhs.raw())
        || ctx.leader_limit_bonus_at(lhs.raw()) < ctx.leader_limit_bonus_at(rhs.raw())
    {
        return false;
    }
    if pool.attr(lhs) != pool.attr(rhs) {
        return false;
    }
    let lhs_mask = pool.unit_mask_raw(lhs);
    let rhs_mask = pool.unit_mask_raw(rhs);
    (rhs_mask & lhs_mask) == rhs_mask
}

fn build_char_groups(
    pool: &CardPool,
    ctx: &SearchContext,
    leader_char: u8,
    member_keep: &[bool],
    top_k: usize,
) -> Vec<CharGroup> {
    let mut by_char = vec![Vec::<CardIdx>::new(); 27];
    for card in pool.indices() {
        let char_id = pool.char_id(card);
        if char_id == leader_char {
            continue;
        }
        if !member_keep.get(card.raw()).copied().unwrap_or(true) {
            continue;
        }
        by_char[char_id as usize].push(card);
    }

    let mut groups = Vec::new();
    for (char_id, cards) in by_char.into_iter().enumerate() {
        if cards.is_empty() {
            continue;
        }
        let mut best_power = 0u32;
        let mut best_skill = 0u32;
        let mut best_base_bonus = 0u32;
        let mut best_limited_bonus = 0u32;
        // 逐队长过滤不记录替代（支援惩罚使其可裁掉全局 member 轮保留的卡），
        // Top-K 下禁用，否则被裁卡的次优卡组无法回换。
        let member_cards = if top_k > 1 {
            cards
        } else {
            filter_member_variants_for_leader(pool, ctx, leader_char, cards)
        };
        let mut keyed_cards = member_cards
            .into_iter()
            .map(|card| (final_chapter_member_key(pool, ctx, leader_char, card), card))
            .collect::<Vec<_>>();
        keyed_cards.sort_unstable_by(|left, right| {
            right
                .0
                .cmp(&left.0)
                .then_with(|| left.1.raw().cmp(&right.1.raw()))
        });
        let sorted_cards = keyed_cards
            .into_iter()
            .map(|(_, card)| card)
            .collect::<Vec<_>>();
        for card in &sorted_cards {
            let eb = pool.event_bonus_exact(*card);
            best_power = best_power.max(pool.power_max(*card));
            best_skill = best_skill.max(pool.skill_max(*card) as u32);
            best_base_bonus = best_base_bonus.max(eb.base_ceil());
            best_limited_bonus = best_limited_bonus.max(eb.limited_ceil());
        }
        let sort_key =
            final_chapter_group_key(best_power, best_skill, best_base_bonus, best_limited_bonus);
        groups.push(CharGroup {
            char_id: char_id as u8,
            cards: sorted_cards,
            best_power,
            best_skill,
            best_base_bonus,
            best_limited_bonus,
            sort_key,
        });
    }

    groups.sort_unstable_by(|left, right| {
        right
            .sort_key
            .cmp(&left.sort_key)
            .then_with(|| left.char_id.cmp(&right.char_id))
    });
    groups
}

fn filter_member_variants_for_leader(
    pool: &CardPool,
    ctx: &SearchContext,
    leader_char: u8,
    cards: Vec<CardIdx>,
) -> Vec<CardIdx> {
    let mut keep = vec![true; cards.len()];
    let support_penalties = cards
        .iter()
        .map(|card| support_penalty_x100(ctx, leader_char, pool.game_id(*card)))
        .collect::<Vec<_>>();
    let mut left = 0usize;
    while left < cards.len() {
        if !keep[left] {
            left += 1;
            continue;
        }
        let lhs = cards[left];
        let mut right = 0usize;
        while right < cards.len() {
            if left != right && keep[right] {
                let rhs = cards[right];
                if !ctx.is_fixed_game_id(pool.game_id(rhs))
                    && member_dominates_for_leader(
                        pool,
                        lhs,
                        rhs,
                        support_penalties[left],
                        support_penalties[right],
                    )
                {
                    keep[right] = false;
                }
            }
            right += 1;
        }
        left += 1;
    }
    cards
        .into_iter()
        .zip(keep)
        .filter_map(|(card, keep)| keep.then_some(card))
        .collect()
}

fn member_dominates_for_leader(
    pool: &CardPool,
    lhs: CardIdx,
    rhs: CardIdx,
    lhs_support_penalty_x100: i32,
    rhs_support_penalty_x100: i32,
) -> bool {
    debug_assert_eq!(pool.char_id(lhs), pool.char_id(rhs));

    let lhs_values = pool.power_values(lhs);
    let rhs_values = pool.power_values(rhs);
    let lhs_lut = pool.power_lut(lhs);
    let rhs_lut = pool.power_lut(rhs);
    let mut idx = 0usize;
    while idx < 8 {
        if decode_u18(lhs_values, lhs_lut, idx) < decode_u18(rhs_values, rhs_lut, idx) {
            return false;
        }
        idx += 1;
    }

    if !skill_dominates(pool, lhs, rhs) {
        return false;
    }

    let lhs_bonus = pool.event_bonus_exact(lhs);
    let rhs_bonus = pool.event_bonus_exact(rhs);
    if lhs_bonus.base_x10() < rhs_bonus.base_x10()
        || lhs_bonus.limited_x10() < rhs_bonus.limited_x10()
    {
        return false;
    }
    if pool.attr(lhs) != pool.attr(rhs) {
        return false;
    }
    let lhs_mask = pool.unit_mask_raw(lhs);
    let rhs_mask = pool.unit_mask_raw(rhs);
    if (rhs_mask & lhs_mask) != rhs_mask {
        return false;
    }
    lhs_support_penalty_x100 <= rhs_support_penalty_x100
}

fn skill_dominates(pool: &CardPool, lhs: CardIdx, rhs: CardIdx) -> bool {
    let lhs_skill = pool.skill(lhs);
    let rhs_skill = pool.skill(rhs);
    if lhs_skill.skill_type != rhs_skill.skill_type {
        return false;
    }

    match lhs_skill.skill_type {
        0 => lhs_skill.value >= rhs_skill.value,
        1 => {
            let left = pool
                .special()
                .unit_count()
                .get(lhs_skill.value.saturating_sub(1) as usize);
            let right = pool
                .special()
                .unit_count()
                .get(rhs_skill.value.saturating_sub(1) as usize);
            let (Some(left), Some(right)) = (left, right) else {
                return false;
            };
            left.unit == right.unit
                && left
                    .score_up
                    .iter()
                    .zip(right.score_up.iter())
                    .all(|(l, r)| l >= r)
        }
        2 => {
            let left = pool
                .special()
                .diff()
                .get(lhs_skill.value.saturating_sub(1) as usize);
            let right = pool
                .special()
                .diff()
                .get(rhs_skill.value.saturating_sub(1) as usize);
            let (Some(left), Some(right)) = (left, right) else {
                return false;
            };
            left.base >= right.base && left.increment >= right.increment
        }
        3 => {
            let left = pool
                .special()
                .ref_skills()
                .get(lhs_skill.value.saturating_sub(1) as usize);
            let right = pool
                .special()
                .ref_skills()
                .get(rhs_skill.value.saturating_sub(1) as usize);
            let (Some(left), Some(right)) = (left, right) else {
                return false;
            };
            left.rate >= right.rate && left.max >= right.max
        }
        _ => false,
    }
}

fn build_leader_const(pool: &CardPool, ctx: &SearchContext, leader: CardIdx) -> LeaderConst {
    let eb = pool.event_bonus_exact(leader);
    let limited_count = (eb.limited_x10() > 0 && ctx.card_bonus_count_limit > 0) as u8;
    LeaderConst {
        leader,
        power: pool.power_max(leader),
        skill: pool.skill_max(leader) as u32,
        base_bonus_const: eb.base_ceil()
            + ctx.leader_honor_bonus_at(leader.raw())
            + ctx.leader_limit_bonus_at(leader.raw()),
        limited_bonus: eb.limited_ceil(),
        limited_count,
        extra_bonus_ub: final_chapter_extra_bonus_bound(pool, ctx, leader, &[], MEMBER_COUNT),
    }
}

struct CharacterSearchState<'a> {
    pool: &'a CardPool,
    ctx: &'a SearchContext,
    suffix: &'a SuffixBound,
    groups: &'a [CharGroup],
    group_suffix: &'a [GroupCeilingTail],
    support: &'a SupportDeck,
    tracker: &'a mut TopKTracker,
    stats: &'a mut SearchStats,
    deadline: &'a mut DeadlineGuard,
    leader: LeaderConst,
}

impl CharacterSearchState<'_> {
    fn recurse_chars(&mut self, depth: usize, start: usize, selected: &mut [usize; MEMBER_COUNT]) {
        if self.deadline.expired_sampled() {
            return;
        }
        if depth == MEMBER_COUNT {
            let mut ordered = *selected;
            order_card_groups(self.groups, &mut ordered);
            let mut deck = [self.leader.leader; DECK_SIZE];
            let partial = CardPartial::for_leader(self.pool, self.ctx, &self.leader);
            let plan = build_card_group_plan(self.groups, &ordered);
            // 排序缓冲按层预留一份：放在递归里会让每个节点付一次 ~3KB 栈清零，
            // 终章单次搜索的节点量在千万级，这项开销会主导整棵树。
            let mut scratch = [[(0u64, CardIdx::new(0), partial); RANKED_CAP]; MEMBER_COUNT];
            self.recurse_cards(&ordered, &plan, 0, &mut deck, partial, &mut scratch);
            return;
        }

        let mut threshold = self.tracker.threshold();
        if threshold != 0 {
            let ub = character_ceiling(
                self.suffix,
                self.ctx,
                self.groups,
                self.group_suffix,
                start,
                &selected[..depth],
                &self.leader,
            );
            if ub <= threshold {
                self.stats.ub_prunes += 1;
                return;
            }
        }

        let mut idx = start;
        while idx < self.groups.len() {
            if self.deadline.expired_sampled() {
                return;
            }
            if threshold != 0 {
                let ub = character_ceiling(
                    self.suffix,
                    self.ctx,
                    self.groups,
                    self.group_suffix,
                    idx,
                    &selected[..depth],
                    &self.leader,
                );
                if ub <= threshold {
                    self.stats.ub_prunes += 1;
                    break;
                }
            }
            selected[depth] = idx;
            self.stats.ep_candidates += 1;
            idx += 1;
            self.recurse_chars(depth + 1, idx, selected);
            threshold = self.tracker.threshold();
        }
    }

    fn recurse_cards(
        &mut self,
        selected: &[usize; MEMBER_COUNT],
        plan: &CardGroupPlan,
        depth: usize,
        deck: &mut [CardIdx; DECK_SIZE],
        partial: CardPartial,
        scratch: &mut [[RankedSlot; RANKED_CAP]],
    ) {
        if self.deadline.expired_sampled() {
            return;
        }
        if depth == MEMBER_COUNT {
            self.stats.leaf_nodes += 1;
            if let Some(score) = exact_final_chapter_leaf(self.pool, self.ctx, deck) {
                self.tracker.insert(DeckResult::new(*deck, score));
            }
            return;
        }

        let mut threshold = self.tracker.threshold();
        if threshold != 0 {
            let ub = selected_card_ceiling_from_partial(
                self.suffix,
                self.ctx,
                plan,
                depth,
                &partial,
                self.leader.skill,
            );
            if ub <= threshold {
                self.stats.ep_continue_prunes += 1;
                return;
            }
        }

        let group = &self.groups[selected[depth]];
        if threshold != 0 {
            let Some((ranked, scratch_tail)) = scratch.split_first_mut() else {
                return;
            };
            let mut ranked_len = 0usize;
            for &card in group.cards.iter().take(ranked.len()) {
                let optimistic_ub = selected_card_ceiling_with_candidate_support_ub(
                    self.pool,
                    self.suffix,
                    self.ctx,
                    plan,
                    depth + 1,
                    &partial,
                    card,
                    self.leader.skill,
                );
                if optimistic_ub <= threshold {
                    continue;
                }
                deck[depth + 1] = card;
                let next_partial =
                    partial.with_card(self.pool, self.ctx.is_world_bloom, self.support, card);
                let ub = selected_card_ceiling_from_partial(
                    self.suffix,
                    self.ctx,
                    plan,
                    depth + 1,
                    &next_partial,
                    self.leader.skill,
                );
                if ub <= threshold {
                    continue;
                }
                if ranked_len < ranked.len() {
                    let mut pos = ranked_len;
                    while pos > 0
                        && (ranked[pos - 1].0 < ub
                            || (ranked[pos - 1].0 == ub && card.raw() < ranked[pos - 1].1.raw()))
                    {
                        ranked[pos] = ranked[pos - 1];
                        pos -= 1;
                    }
                    ranked[pos] = (ub, card, next_partial);
                    ranked_len += 1;
                }
            }
            let mut ranked_idx = 0usize;
            while ranked_idx < ranked_len {
                let (ub, card, next_partial) = ranked[ranked_idx];
                if ub <= threshold {
                    self.stats.ep_continue_prunes += 1;
                    break;
                }
                deck[depth + 1] = card;
                self.recurse_cards(selected, plan, depth + 1, deck, next_partial, scratch_tail);
                threshold = self.tracker.threshold();
                ranked_idx += 1;
            }
        } else {
            for &card in &group.cards {
                deck[depth + 1] = card;
                let next_partial =
                    partial.with_card(self.pool, self.ctx.is_world_bloom, self.support, card);
                self.recurse_cards(
                    selected,
                    plan,
                    depth + 1,
                    deck,
                    next_partial,
                    &mut scratch[1..],
                );
            }
        }
    }
}

#[inline(always)]
fn order_card_groups(groups: &[CharGroup], selected: &mut [usize; MEMBER_COUNT]) {
    let mut idx = 1usize;
    while idx < MEMBER_COUNT {
        let current = selected[idx];
        let mut pos = idx;
        while pos > 0 && group_card_order_before(groups, current, selected[pos - 1]) {
            selected[pos] = selected[pos - 1];
            pos -= 1;
        }
        selected[pos] = current;
        idx += 1;
    }
}

#[inline(always)]
fn group_card_order_before(groups: &[CharGroup], left: usize, right: usize) -> bool {
    let lhs = &groups[left];
    let rhs = &groups[right];
    lhs.sort_key > rhs.sort_key
        || (lhs.sort_key == rhs.sort_key && lhs.cards.len() < rhs.cards.len())
}

fn character_ceiling(
    suffix: &SuffixBound,
    ctx: &SearchContext,
    groups: &[CharGroup],
    group_suffix: &[GroupCeilingTail],
    start: usize,
    selected: &[usize],
    leader: &LeaderConst,
) -> u64 {
    let mut selected_power = 0u32;
    let mut selected_skill = 0u32;
    let mut selected_base = 0u32;
    let mut selected_limited = [0u32; MEMBER_COUNT + 1];
    let mut idx = 0usize;
    while idx < selected.len() {
        let group = &groups[selected[idx]];
        selected_power += group.best_power;
        selected_skill += group.best_skill;
        selected_base += group.best_base_bonus;
        insert_topk_u32(&mut selected_limited, group.best_limited_bonus);
        idx += 1;
    }
    let tail = &group_suffix[start];

    let remaining = MEMBER_COUNT - selected.len();
    let mut power_sum = leader.power + selected_power;
    let mut skill_sum = leader.skill + selected_skill;
    let mut bonus_sum = leader.base_bonus_const + leader.limited_bonus + selected_base;
    let mut slot = 0usize;
    while slot < remaining {
        power_sum += tail.top_power[slot];
        skill_sum += tail.top_skill[slot];
        bonus_sum += tail.top_base_bonus[slot];
        slot += 1;
    }

    let limited_limit = ctx
        .card_bonus_count_limit
        .saturating_sub(leader.limited_count as usize);
    let limited_sum = merged_limited_sum(
        &selected_limited,
        &tail.top_limited_bonus,
        limited_limit.min(MEMBER_COUNT),
    );
    suffix.ceiling(
        power_sum,
        bonus_sum + limited_sum + leader.extra_bonus_ub,
        skill_sum,
        leader.skill,
    )
}

fn selected_card_ceiling_from_partial(
    suffix: &SuffixBound,
    ctx: &SearchContext,
    plan: &CardGroupPlan,
    chosen: usize,
    partial: &CardPartial,
    leader_skill: u32,
) -> u64 {
    let power_sum = partial.power + plan.rem_power[chosen];
    let skill_sum = partial.skill + plan.rem_skill[chosen];
    let bonus_sum = partial.base_bonus + plan.rem_base_bonus[chosen];
    let limited_sum = merged_limited_sum(
        &partial.limited_values,
        &plan.rem_limited_values[chosen],
        ctx.card_bonus_count_limit.min(MEMBER_COUNT + 1),
    );
    let extra_bonus_ub =
        final_chapter_extra_bonus_bound_from_partial(ctx, partial, MEMBER_COUNT - chosen);
    suffix.ceiling(
        power_sum,
        bonus_sum + limited_sum + extra_bonus_ub,
        skill_sum,
        leader_skill,
    )
}

#[allow(clippy::too_many_arguments)]
fn selected_card_ceiling_with_candidate_support_ub(
    pool: &CardPool,
    suffix: &SuffixBound,
    ctx: &SearchContext,
    plan: &CardGroupPlan,
    chosen: usize,
    partial: &CardPartial,
    card: CardIdx,
    leader_skill: u32,
) -> u64 {
    let eb = pool.event_bonus_exact(card);
    let power_sum = partial.power + pool.power_max(card) + plan.rem_power[chosen];
    let skill_sum = partial.skill + pool.skill_max(card) as u32 + plan.rem_skill[chosen];
    let bonus_sum = partial.base_bonus + eb.base_ceil() + plan.rem_base_bonus[chosen];
    let mut limited_values = partial.limited_values;
    insert_topk_u32(&mut limited_values, eb.limited_ceil());
    let limited_sum = merged_limited_sum(
        &limited_values,
        &plan.rem_limited_values[chosen],
        ctx.card_bonus_count_limit.min(MEMBER_COUNT + 1),
    );
    let extra_bonus_ub = final_chapter_extra_bonus_bound_after_candidate_support_ub(
        pool,
        ctx,
        partial,
        card,
        MEMBER_COUNT - chosen,
    );
    suffix.ceiling(
        power_sum,
        bonus_sum + limited_sum + extra_bonus_ub,
        skill_sum,
        leader_skill,
    )
}

fn final_chapter_extra_bonus_bound_after_candidate_support_ub(
    pool: &CardPool,
    ctx: &SearchContext,
    partial: &CardPartial,
    card: CardIdx,
    rest: usize,
) -> u32 {
    if !ctx.is_world_bloom {
        return ctx.extra_bonus_ub;
    }

    let attr_set = partial.attr_set | (1u8 << pool.attr(card));
    let current_attrs = attr_set.count_ones() as usize;
    let max_attrs = (current_attrs + rest).min(DECK_SIZE);
    let mut diff_ub = 0u32;
    let mut count = current_attrs;
    while count <= max_attrs {
        diff_ub = diff_ub.max(ctx.diff_attr_bonus[count] as u32);
        count += 1;
    }

    diff_ub + partial.support_bonus_ceil
}

#[inline(always)]
fn merged_limited_sum(
    left: &[u32; MEMBER_COUNT + 1],
    right: &[u32; MEMBER_COUNT + 1],
    cap: usize,
) -> u32 {
    let mut sum = 0u32;
    let mut li = 0usize;
    let mut ri = 0usize;
    let mut picked = 0usize;
    while picked < cap {
        let lv = left.get(li).copied().unwrap_or(0);
        let rv = right.get(ri).copied().unwrap_or(0);
        if lv >= rv {
            sum += lv;
            li += 1;
        } else {
            sum += rv;
            ri += 1;
        }
        picked += 1;
    }
    sum
}

fn final_chapter_extra_bonus_bound(
    pool: &CardPool,
    ctx: &SearchContext,
    leader: CardIdx,
    chosen_members: &[CardIdx],
    rest: usize,
) -> u32 {
    if !ctx.is_world_bloom {
        return ctx.extra_bonus_ub;
    }

    let mut attr_set = 1u8 << pool.attr(leader);
    let mut selected = [0u16; DECK_SIZE];
    selected[0] = pool.game_id(leader);
    let mut selected_len = 1usize;
    for &card in chosen_members {
        attr_set |= 1u8 << pool.attr(card);
        selected[selected_len] = pool.game_id(card);
        selected_len += 1;
    }

    let current_attrs = attr_set.count_ones() as usize;
    let max_attrs = (current_attrs + rest).min(DECK_SIZE);
    let mut diff_ub = 0u32;
    let mut count = current_attrs;
    while count <= max_attrs {
        diff_ub = diff_ub.max(ctx.diff_attr_bonus[count] as u32);
        count += 1;
    }

    let leader_char = pool.char_id(leader);
    let support = ctx.support_deck_for_leader(leader_char);
    let mut support_sum = 0.0_f64;
    let mut picked = 0usize;
    for &(game_id, bonus) in &support.cards {
        if picked >= support.count as usize {
            break;
        }
        if selected_contains(&selected, selected_len, game_id) {
            continue;
        }
        support_sum += bonus;
        picked += 1;
    }

    diff_ub + support_sum.ceil() as u32
}

fn final_chapter_extra_bonus_bound_from_partial(
    ctx: &SearchContext,
    partial: &CardPartial,
    rest: usize,
) -> u32 {
    if !ctx.is_world_bloom {
        return ctx.extra_bonus_ub;
    }

    let current_attrs = partial.attr_set.count_ones() as usize;
    let max_attrs = (current_attrs + rest).min(DECK_SIZE);
    let mut diff_ub = 0u32;
    let mut count = current_attrs;
    while count <= max_attrs {
        diff_ub = diff_ub.max(ctx.diff_attr_bonus[count] as u32);
        count += 1;
    }

    diff_ub + partial.support_bonus_ceil
}

fn initial_final_chapter_support_state(
    ctx: &SearchContext,
    leader_char: u8,
    selected: &[u16; DECK_SIZE],
    selected_len: usize,
) -> (f64, usize) {
    if !ctx.is_world_bloom {
        return (0.0, 0);
    }

    let support = ctx.support_deck_for_leader(leader_char);
    let mut support_sum = 0.0_f64;
    let mut picked = 0usize;
    let mut idx = 0usize;
    while idx < support.cards.len() {
        if picked >= support.count as usize {
            break;
        }
        let (game_id, bonus) = support.cards[idx];
        idx += 1;
        if selected_contains(selected, selected_len, game_id) {
            continue;
        }
        support_sum += bonus;
        picked += 1;
    }

    (support_sum, idx)
}

fn advance_final_chapter_support_state(
    is_world_bloom: bool,
    support: &SupportDeck,
    current_sum: f64,
    current_next_scan: usize,
    selected: &[u16; DECK_SIZE],
    selected_len: usize,
    new_game_id: u16,
) -> (f64, usize) {
    if !is_world_bloom {
        return (0.0, 0);
    }

    let mut support_sum = current_sum;
    let mut next_scan = current_next_scan;
    let scan_end = next_scan.min(support.cards.len());
    let mut replaced = false;
    let mut idx = 0usize;
    while idx < scan_end {
        let (game_id, bonus) = support.cards[idx];
        if game_id == new_game_id && !selected_contains(selected, selected_len, game_id) {
            support_sum -= bonus;
            replaced = true;
            break;
        }
        idx += 1;
    }

    if replaced {
        while next_scan < support.cards.len() {
            let (game_id, bonus) = support.cards[next_scan];
            next_scan += 1;
            if game_id == new_game_id || selected_contains(selected, selected_len, game_id) {
                continue;
            }
            support_sum += bonus;
            break;
        }
    }

    (support_sum, next_scan)
}

#[inline(always)]
fn selected_contains(selected: &[u16; DECK_SIZE], selected_len: usize, game_id: u16) -> bool {
    selected[0] == game_id
        || (selected_len > 1 && selected[1] == game_id)
        || (selected_len > 2 && selected[2] == game_id)
        || (selected_len > 3 && selected[3] == game_id)
        || (selected_len > 4 && selected[4] == game_id)
}

#[inline(always)]
fn final_chapter_card_key(pool: &CardPool, card: CardIdx) -> u64 {
    let power = pool.power_max(card) as u64;
    let skill = pool.skill_max(card) as u64;
    let eb = pool.event_bonus_exact(card);
    let bonus_x10 = eb.total_x10() as u64;
    power * (256 + skill) * (1000 + bonus_x10)
}

#[inline(always)]
fn final_chapter_member_key(
    pool: &CardPool,
    ctx: &SearchContext,
    leader_char: u8,
    card: CardIdx,
) -> u64 {
    let power = pool.power_max(card) as u64;
    let skill = pool.skill_max(card) as u64;
    let eb = pool.event_bonus_exact(card);
    let card_bonus_x100 = eb.total_x10() as i64 * 10;
    let support_penalty_x100 = support_penalty_x100(ctx, leader_char, pool.game_id(card)) as i64;
    let net_bonus_x100 = (card_bonus_x100 - support_penalty_x100).max(0) as u64;
    power * (256 + skill) * (10_000 + net_bonus_x100)
}

fn support_penalty_x100(ctx: &SearchContext, leader_char: u8, game_id: u16) -> i32 {
    let support = ctx.support_deck_for_leader(leader_char);
    let count = support.count as usize;
    if count == 0 {
        return 0;
    }
    let replacement = support
        .cards
        .get(count)
        .map(|(_, bonus)| *bonus)
        .unwrap_or(0.0);
    let mut idx = 0usize;
    while idx < count.min(support.cards.len()) {
        let (support_id, bonus) = support.cards[idx];
        if support_id == game_id {
            return ((bonus - replacement).max(0.0) * 100.0).round() as i32;
        }
        idx += 1;
    }
    0
}

fn final_chapter_group_key(
    best_power: u32,
    best_skill: u32,
    best_base_bonus: u32,
    best_limited_bonus: u32,
) -> u64 {
    let power = best_power as u64;
    let skill = best_skill as u64;
    let bonus = (best_base_bonus + best_limited_bonus) as u64;
    power * (256 + skill) * (100 + bonus)
}

#[inline(always)]
fn insert_topk_u32(values: &mut [u32], value: u32) {
    let mut slot = 0usize;
    while slot < values.len() {
        if value > values[slot] {
            let mut shift = values.len() - 1;
            while shift > slot {
                values[shift] = values[shift - 1];
                shift -= 1;
            }
            values[slot] = value;
            break;
        }
        slot += 1;
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct ExactSkillValue {
    score_up: f64,
    score_up_to_reference: f64,
    ref_rate: f64,
    ref_max: f64,
    has_ref: bool,
}

fn exact_final_chapter_leaf(
    pool: &CardPool,
    ctx: &SearchContext,
    deck: &[CardIdx; DECK_SIZE],
) -> Option<u64> {
    if !matches!(
        ctx.effective_live_type(),
        LiveType::Multi | LiveType::Cheerful
    ) || !matches!(ctx.live_skill_order, LiveSkillOrder::Average)
        || ctx.keep_after_training_state
        || !matches!(
            ctx.skill_reference_strategy,
            crate::types::SkillReferenceStrategy::Average
        )
        || ctx.multi_teammate_score_up.is_some()
    {
        return leaf_evaluate_checked(pool, ctx, deck);
    }

    let power_total = ctx.clamp_power_total(resolve_power_target(pool, deck) + ctx.honor_bonus);
    let total_bonus = final_chapter_leaf_total_bonus(pool, ctx, deck);
    let unit_counts = count_units(pool, deck);
    let diff_count = distinct_unit_count(&unit_counts).saturating_sub(1).min(2) as u32;
    let mut skills = [ExactSkillValue::default(); DECK_SIZE];
    let mut idx = 0usize;
    while idx < DECK_SIZE {
        let card = deck[idx];
        let slot = pool.skill(card);
        match slot.skill_type {
            0 => {
                skills[idx].score_up = slot.value as f64;
                skills[idx].score_up_to_reference = skills[idx].score_up;
            }
            1 => {
                let value = resolve_unit_count_skill(pool, slot, &unit_counts) as f64;
                skills[idx].score_up = value;
                skills[idx].score_up_to_reference = value;
            }
            2 => {
                let value = resolve_diff_skill(pool, slot, diff_count) as f64;
                skills[idx].score_up = value;
                skills[idx].score_up_to_reference = value;
            }
            3 => {
                let base = pool.skill_min(card) as f64;
                let (ref_rate, ref_max) = resolve_ref_skill(pool, slot);
                skills[idx] = ExactSkillValue {
                    score_up: base,
                    score_up_to_reference: base + ref_max as f64,
                    ref_rate: ref_rate as f64,
                    ref_max: ref_max as f64,
                    has_ref: ref_rate != 0 && ref_max != 0,
                };
            }
            _ => {}
        }
        idx += 1;
    }

    let mut ref_idx = 0usize;
    while ref_idx < DECK_SIZE {
        if skills[ref_idx].has_ref {
            let mut total = 0.0_f64;
            let mut count = 0usize;
            let mut other = 0usize;
            while other < DECK_SIZE {
                if other != ref_idx {
                    total += (skills[other].score_up_to_reference * skills[ref_idx].ref_rate
                        / 100.0)
                        .floor()
                        .min(skills[ref_idx].ref_max);
                    count += 1;
                }
                other += 1;
            }
            skills[ref_idx].score_up += total / count as f64;
        }
        ref_idx += 1;
    }

    let self_skill = skills[0].score_up
        + skills[1].score_up / 5.0
        + skills[2].score_up / 5.0
        + skills[3].score_up / 5.0
        + skills[4].score_up / 5.0;
    let rate_sum = ctx.skill_scores[1][0]
        + ctx.skill_scores[1][1]
        + ctx.skill_scores[1][2]
        + ctx.skill_scores[1][3]
        + ctx.skill_scores[1][4]
        + ctx.skill_scores[1][5];
    let base_rate = ctx.base_score + ctx.fever_score * 0.5;
    let power_sum = if let Some(tp) = ctx.multi_teammate_power {
        power_total as i32 + tp * (DECK_SIZE as i32 - 1)
    } else {
        DECK_SIZE as i32 * power_total as i32
    };
    let live_score = ((base_rate + self_skill * rate_sum / 100.0) * power_total as f64 * 4.0
        + DECK_SIZE as f64 * 0.015 * power_sum as f64) as i32;
    let event_point = calc_event_point(live_score, total_bonus, ctx);

    Some(match ctx.target {
        ScoreTarget::Score => ((event_point as u64) << 32) | (live_score as u32 as u64),
        _ => {
            let mut ordered_deck = *deck;
            reorder_member_deck(pool, &mut ordered_deck);
            return leaf_evaluate_checked(pool, ctx, &ordered_deck);
        }
    })
}

fn final_chapter_leaf_total_bonus(
    pool: &CardPool,
    ctx: &SearchContext,
    deck: &[CardIdx; DECK_SIZE],
) -> f64 {
    let mut attr_set = 0u8;
    let mut game_ids = [0u16; DECK_SIZE];
    let mut total_bonus_x10 = 0u32;
    let mut member_limited_x10 = [0u32; MEMBER_COUNT + 1];
    let mut limited_slots_left = ctx.card_bonus_count_limit.min(DECK_SIZE);
    let mut pos = 0usize;
    while pos < DECK_SIZE {
        let card = deck[pos];
        attr_set |= 1u8 << pool.attr(card);
        game_ids[pos] = pool.game_id(card);

        let bonus = pool.event_bonus_exact(card);
        total_bonus_x10 += bonus.base_x10();

        if pos == 0 {
            if bonus.limited_x10() > 0 && limited_slots_left > 0 {
                total_bonus_x10 += bonus.limited_x10();
                limited_slots_left -= 1;
            }
            total_bonus_x10 += 10 * ctx.leader_honor_bonus_at(card.raw());
            total_bonus_x10 += 10 * ctx.leader_limit_bonus_at(card.raw());
        } else {
            insert_topk_u32(&mut member_limited_x10, bonus.limited_x10());
        }
        pos += 1;
    }

    let mut limited_slot = 0usize;
    while limited_slot < limited_slots_left {
        total_bonus_x10 += member_limited_x10[limited_slot];
        limited_slot += 1;
    }

    let mut total_bonus = total_bonus_x10 as f64 / 10.0;
    if ctx.is_world_bloom {
        total_bonus += ctx.diff_attr_bonus[attr_set.count_ones() as usize] as f64;
        total_bonus += final_chapter_support_bonus_exact(ctx, pool.char_id(deck[0]), &game_ids);
    }
    total_bonus
}

fn final_chapter_support_bonus_exact(
    ctx: &SearchContext,
    leader_char: u8,
    game_ids: &[u16; DECK_SIZE],
) -> f64 {
    let support = ctx.support_deck_for_leader(leader_char);
    let mut total = 0.0_f64;
    let mut picked = 0u8;
    for &(game_id, bonus) in &support.cards {
        if picked >= support.count {
            break;
        }
        if game_ids[0] == game_id
            || game_ids[1] == game_id
            || game_ids[2] == game_id
            || game_ids[3] == game_id
            || game_ids[4] == game_id
        {
            continue;
        }
        total += bonus;
        picked += 1;
    }
    total
}

fn reorder_member_deck(pool: &CardPool, deck: &mut [CardIdx; DECK_SIZE]) {
    let mut indices = [1usize, 2, 3, 4];
    indices.sort_unstable_by(|left, right| {
        let left_card = deck[*left];
        let right_card = deck[*right];
        let left_bonus = pool.event_bonus_exact(left_card).limited_x10();
        let right_bonus = pool.event_bonus_exact(right_card).limited_x10();
        right_bonus
            .cmp(&left_bonus)
            .then_with(|| right_card.raw().cmp(&left_card.raw()))
    });
    let original = *deck;
    let mut slot = 0usize;
    while slot < MEMBER_COUNT {
        deck[slot + 1] = original[indices[slot]];
        slot += 1;
    }
}

fn count_units(pool: &CardPool, deck: &[CardIdx; DECK_SIZE]) -> [u8; 6] {
    let mut unit_counts = [0u8; 6];
    let mut pos = 0usize;
    while pos < DECK_SIZE {
        let card = deck[pos];
        let unit_mask = pool.unit_mask_raw(card);
        let mut unit = 0usize;
        while unit < 6 {
            if unit_mask & (1u8 << unit) != 0 {
                unit_counts[unit] += 1;
            }
            unit += 1;
        }
        pos += 1;
    }
    unit_counts
}

fn distinct_unit_count(unit_counts: &[u8; 6]) -> u8 {
    let mut count = 0u8;
    let mut index = 0usize;
    while index < 6 {
        if unit_counts[index] > 0 {
            count += 1;
        }
        index += 1;
    }
    count
}

fn resolve_unit_count_skill(
    pool: &CardPool,
    skill: crate::pool::SkillSlot,
    unit_counts: &[u8; 6],
) -> u32 {
    let index = skill.value.saturating_sub(1) as usize;
    let Some(entry) = pool.special().unit_count().get(index) else {
        return 0;
    };
    let unit = entry.unit as usize;
    if unit >= unit_counts.len() {
        return 0;
    }
    let member_count = unit_counts[unit].clamp(1, 5) as usize;
    entry.score_up[member_count - 1] as u32
}

fn resolve_diff_skill(pool: &CardPool, skill: crate::pool::SkillSlot, diff_count: u32) -> u32 {
    let index = skill.value.saturating_sub(1) as usize;
    let Some(entry) = pool.special().diff().get(index) else {
        return 0;
    };
    entry.base as u32 + entry.increment as u32 * diff_count
}

fn resolve_ref_skill(pool: &CardPool, skill: crate::pool::SkillSlot) -> (u8, u8) {
    let index = skill.value.saturating_sub(1) as usize;
    let Some(entry) = pool.special().ref_skills().get(index) else {
        return (0, 0);
    };
    (entry.rate, entry.max)
}

struct TopKTracker {
    top_k: usize,
    game_ids: Vec<u16>,
    results: Vec<DeckResult>,
}

impl TopKTracker {
    fn new(top_k: usize, pool: &CardPool) -> Self {
        Self {
            top_k,
            game_ids: pool.indices().map(|card| pool.game_id(card)).collect(),
            results: Vec::with_capacity(top_k),
        }
    }

    fn threshold(&self) -> u64 {
        if self.results.len() < self.top_k {
            0
        } else {
            self.results.last().map(|result| result.score).unwrap_or(0)
        }
    }

    fn insert(&mut self, candidate: DeckResult) {
        if let Some(existing_pos) = self
            .results
            .iter()
            .position(|existing| self.same_game_card_set(existing, &candidate))
        {
            let existing = self.results[existing_pos];
            let candidate_is_better = existing.score < candidate.score
                || (existing.score == candidate.score && candidate.cards < existing.cards);
            if !candidate_is_better {
                return;
            }
            self.results.remove(existing_pos);
        }
        let pos = self
            .results
            .iter()
            .position(|existing| {
                existing.score < candidate.score
                    || (existing.score == candidate.score && candidate.cards < existing.cards)
            })
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
