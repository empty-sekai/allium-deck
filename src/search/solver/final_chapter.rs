use crate::search::budget::SearchBudget as DeadlineGuard;

use crate::pool::{CardIdx, CardPool};
use crate::types::{DECK_SIZE, LiveSkillOrder, LiveType};

use crate::search::context::{SearchContext, SupportDeck};
use crate::search::dfs::SearchStats;
use crate::search::suffix::SuffixBound;
use crate::search::types::{DeckResult, SearchParams};
use crate::search::{placement, tracker::TopKTracker};

const MEMBER_COUNT: usize = 4;
const FINAL_CHAPTER_SEED_GROUP_PREFIX: usize = 6;
/// 每层最多按上界降序保留的候选卡数。
const RANKED_CAP: usize = 32;

/// `recurse_cards` 排序候选缓冲的单槽：(上界, 卡, 落子后的局部状态)。
type RankedSlot = (u64, CardIdx, CardPartial);
/// The member cards of one character and one attribute.
#[derive(Clone)]
struct CharGroup {
    char_id: u8,
    /// Best first by the member key; see [`ScanCard`].
    scan: Vec<ScanCard>,
    best_power: u32,
    best_skill: u32,
    best_base_bonus: u32,
    best_limited_bonus: u32,
    attr_mask: u8,
    sort_key: u64,
}

/// The terms a member card contributes to a card-stage ceiling, or their
/// maxima over several cards of one attribute.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct MemberTerms {
    power: u32,
    skill: u32,
    base_bonus: u32,
    limited_bonus: u32,
    attr: u8,
}

impl MemberTerms {
    #[inline(always)]
    fn of(pool: &CardPool, card: CardIdx) -> Self {
        let eb = pool.event_bonus_exact(card);
        Self {
            power: pool.power_max(card),
            skill: pool.skill_max(card) as u32,
            base_bonus: eb.base_ceil(),
            limited_bonus: eb.limited_ceil(),
            attr: pool.attr(card),
        }
    }

    #[inline(always)]
    fn max(self, other: Self) -> Self {
        debug_assert_eq!(self.attr, other.attr);
        Self {
            power: self.power.max(other.power),
            skill: self.skill.max(other.skill),
            base_bonus: self.base_bonus.max(other.base_bonus),
            limited_bonus: self.limited_bonus.max(other.limited_bonus),
            attr: self.attr,
        }
    }
}

/// A group card in scan order. `rest` holds the maxima of the terms from
/// this card to the end of its group.
#[derive(Clone, Copy, Debug)]
struct ScanCard {
    card: CardIdx,
    terms: MemberTerms,
    rest: MemberTerms,
}

/// `cards` share one attribute and are best first by the member key.
fn build_scan(pool: &CardPool, cards: &[CardIdx]) -> Vec<ScanCard> {
    let mut scan: Vec<ScanCard> = cards
        .iter()
        .map(|&card| {
            let terms = MemberTerms::of(pool, card);
            ScanCard {
                card,
                terms,
                rest: terms,
            }
        })
        .collect();
    for idx in (1..scan.len()).rev() {
        scan[idx - 1].rest = scan[idx - 1].terms.max(scan[idx].rest);
    }
    scan
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
    support_bonus_ub: u32,
    leader_attr_set: u8,
    use_group_attr_dp: bool,
}

/// The reductions of the selected character-group prefix. Updated once per
/// descent and reused by every suffix ceiling queried at that depth.
#[derive(Clone, Copy)]
struct CharacterPrefix {
    power: u32,
    skill: u32,
    max_skill: u32,
    base_bonus: u32,
    limited_values: [u32; MEMBER_COUNT + 1],
    attr_states: u32,
}

impl CharacterPrefix {
    #[inline(always)]
    fn for_leader(leader: &LeaderConst) -> Self {
        Self {
            power: 0,
            skill: 0,
            max_skill: 0,
            base_bonus: 0,
            limited_values: [0; MEMBER_COUNT + 1],
            attr_states: if leader.use_group_attr_dp {
                1u32 << leader.leader_attr_set
            } else {
                0
            },
        }
    }

    #[inline(always)]
    fn with_group(mut self, group: &CharGroup) -> Self {
        self.power += group.best_power;
        self.skill += group.best_skill;
        self.max_skill = self.max_skill.max(group.best_skill);
        self.base_bonus += group.best_base_bonus;
        insert_topk_u32(&mut self.limited_values, group.best_limited_bonus);
        if self.attr_states != 0 {
            self.attr_states = extend_attr_union_states(self.attr_states, group.attr_mask);
        }
        self
    }
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
    max_skill: u32,
    base_bonus: u32,
    limited_values: [u32; MEMBER_COUNT + 1],
    limited_sum: u32,
    attr_set: u8,
    selected: [u16; DECK_SIZE],
    selected_len: usize,
    /// End of the support entries summed into `support_bonus_ceil`.
    support_next_scan: usize,
    support_bonus_ceil: u32,
}

#[derive(Clone, Copy)]
struct CardGroupPlan {
    rem_power: [u32; MEMBER_COUNT + 1],
    rem_skill: [u32; MEMBER_COUNT + 1],
    rem_max_skill: [u32; MEMBER_COUNT + 1],
    rem_base_bonus: [u32; MEMBER_COUNT + 1],
    rem_limited_values: [[u32; MEMBER_COUNT + 1]; MEMBER_COUNT + 1],
    rem_limited_sum: [u32; MEMBER_COUNT + 1],
    uniform_limited_cap: Option<u32>,
    /// Exact maximum diversity bonus for the fixed remaining groups and each
    /// incoming five-bit attribute union.
    attr_bonus: [[u16; 32]; MEMBER_COUNT + 1],
}

#[derive(Clone, Copy, Default)]
struct GroupCeilingTail {
    /// `versions[open]` is equal for two suffixes exactly when the fields a
    /// ceiling with `open` member slots left reads are equal; see
    /// [`Self::read_by`].
    versions: [u32; MEMBER_COUNT + 1],
    top_power: [u32; MEMBER_COUNT + 1],
    top_skill: [u32; MEMBER_COUNT + 1],
    top_base_bonus: [u32; MEMBER_COUNT + 1],
    top_limited_bonus: [u32; MEMBER_COUNT + 1],
    /// Best diversity bonus for each starting attribute set after selecting
    /// exactly k groups from this suffix. Zero also represents infeasibility.
    attr_bonus: [[u16; 32]; MEMBER_COUNT + 1],
}

impl GroupCeilingTail {
    /// The fields [`character_ceiling`] reads with `open` member slots left.
    fn read_by(&self, open: usize) -> ([&[u32]; 4], &[u16; 32]) {
        (
            [
                &self.top_power[..open],
                &self.top_skill[..open],
                &self.top_base_bonus[..open],
                &self.top_limited_bonus[..open],
            ],
            &self.attr_bonus[open],
        )
    }
}

impl CardGroupPlan {
    #[inline(always)]
    fn limited_sum(&self, partial: &CardPartial, chosen: usize, cap: usize, candidate: u32) -> u32 {
        if let Some(limit) = self.uniform_limited_cap {
            return (partial.limited_sum + self.rem_limited_sum[chosen] + candidate).min(limit);
        }
        let mut values = partial.limited_values;
        if candidate != 0 {
            insert_topk_u32(&mut values, candidate);
        }
        merged_limited_sum(
            &values,
            &self.rem_limited_values[chosen],
            cap.min(DECK_SIZE),
        )
    }
}

/// Equal nonzero contributions turn a top-N sum into a capped additive sum.
/// Inspect rounded upper-bound values; mixed amounts retain the general merge.
fn uniform_limited_cap(pool: &CardPool, count: usize) -> Option<u32> {
    let mut common = None;
    for card in pool.indices() {
        let value = pool.event_bonus_exact(card).limited_ceil();
        if value == 0 {
            continue;
        }
        if common.is_some_and(|previous| previous != value) {
            return None;
        }
        common = Some(value);
    }
    Some(common.unwrap_or(0) * count.min(DECK_SIZE) as u32)
}

impl CardPartial {
    #[inline(always)]
    fn for_leader(pool: &CardPool, ctx: &SearchContext, leader: &LeaderConst) -> Self {
        let mut limited_values = [0u32; MEMBER_COUNT + 1];
        limited_values[0] = leader.limited_bonus;
        let mut selected = [0u16; DECK_SIZE];
        selected[0] = pool.game_id(leader.leader);
        let (support_bonus_ceil, support_next_scan) = if ctx.is_world_bloom {
            let support = ctx.support_deck_for_leader(pool.char_id(leader.leader));
            let (sum, next_scan) = remaining_support(support, &selected, 1);
            (sum.ceil() as u32, next_scan)
        } else {
            (0, 0)
        };
        Self {
            power: leader.power,
            skill: leader.skill,
            max_skill: leader.skill,
            base_bonus: leader.base_bonus_const,
            limited_values,
            limited_sum: leader.limited_bonus,
            attr_set: 1u8 << pool.attr(leader.leader),
            selected,
            selected_len: 1,
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
        next.max_skill = next.max_skill.max(pool.skill_max(card) as u32);
        next.base_bonus += eb.base_ceil();
        insert_topk_u32(&mut next.limited_values, eb.limited_ceil());
        next.limited_sum += eb.limited_ceil();
        next.attr_set |= 1u8 << pool.attr(card);
        let game_id = pool.game_id(card);
        next.selected[next.selected_len] = game_id;
        next.selected_len += 1;
        // A card outside the summed entries leaves the evaluator-order sum
        // unchanged; otherwise it is summed again, never updated in place.
        if is_world_bloom
            && support.cards[..self.support_next_scan]
                .iter()
                .any(|&(id, _)| id == game_id)
        {
            let (sum, next_scan) = remaining_support(support, &next.selected, next.selected_len);
            next.support_bonus_ceil = sum.ceil() as u32;
            next.support_next_scan = next_scan;
        }
        next
    }
}

/// Diversity bonus of every five-bit attribute union.
fn diversity_bonus(diff_attr_bonus: &[u16; 6]) -> [u16; 32] {
    core::array::from_fn(|set: usize| diff_attr_bonus[set.count_ones() as usize])
}

fn build_card_group_plan(
    groups: &[CharGroup],
    selected: &[usize; MEMBER_COUNT],
    diversity: &[u16; 32],
    uniform_limited_cap: Option<u32>,
) -> CardGroupPlan {
    let mut plan = CardGroupPlan {
        rem_power: [0; MEMBER_COUNT + 1],
        rem_skill: [0; MEMBER_COUNT + 1],
        rem_max_skill: [0; MEMBER_COUNT + 1],
        rem_base_bonus: [0; MEMBER_COUNT + 1],
        rem_limited_values: [[0; MEMBER_COUNT + 1]; MEMBER_COUNT + 1],
        rem_limited_sum: [0; MEMBER_COUNT + 1],
        uniform_limited_cap,
        attr_bonus: [[0; 32]; MEMBER_COUNT + 1],
    };
    plan.attr_bonus[MEMBER_COUNT] = *diversity;
    let mut depth = MEMBER_COUNT;
    while depth > 0 {
        depth -= 1;
        let next = depth + 1;
        let group = &groups[selected[depth]];
        let (head, tail) = plan.attr_bonus.split_at_mut(next);
        attr_step(&tail[0], group.attr_mask, &mut head[depth]);
        plan.rem_power[depth] = plan.rem_power[next] + group.best_power;
        plan.rem_skill[depth] = plan.rem_skill[next] + group.best_skill;
        plan.rem_max_skill[depth] = plan.rem_max_skill[next].max(group.best_skill);
        plan.rem_base_bonus[depth] = plan.rem_base_bonus[next] + group.best_base_bonus;
        plan.rem_limited_values[depth] = plan.rem_limited_values[next];
        plan.rem_limited_sum[depth] = plan.rem_limited_sum[next] + group.best_limited_bonus;
        insert_topk_u32(
            &mut plan.rem_limited_values[depth],
            group.best_limited_bonus,
        );
    }
    plan
}

/// Raises `best[set]` to the best bonus reachable when one card of a group
/// with `attr_mask` joins the attribute union `set`, given the best bonus
/// `next` of every union after that card.
#[inline(always)]
fn attr_step(next: &[u16; 32], attr_mask: u8, best: &mut [u16; 32]) {
    if attr_mask & !31 != 0 {
        // An attribute outside the five-bit union leaves every union as is.
        for (best, &next) in best.iter_mut().zip(next) {
            *best = (*best).max(next);
        }
    }
    let mut attrs = attr_mask & 31;
    while attrs != 0 {
        let bit = 1usize << attrs.trailing_zeros();
        attrs &= attrs - 1;
        // In every block of 2*bit unions the lower half lacks `bit` and joins
        // the upper half, which already holds it.
        for (best, next) in best
            .chunks_exact_mut(2 * bit)
            .zip(next.chunks_exact(2 * bit))
        {
            let (best_lo, best_hi) = best.split_at_mut(bit);
            let next_hi = &next[bit..];
            for ((lo, hi), &joined) in best_lo.iter_mut().zip(best_hi).zip(next_hi) {
                *lo = (*lo).max(joined);
                *hi = (*hi).max(joined);
            }
        }
    }
}

fn build_group_ceiling_suffix(
    groups: &[CharGroup],
    diff_attr_bonus: &[u16; 6],
) -> Vec<GroupCeilingTail> {
    let mut suffix = vec![GroupCeilingTail::default(); groups.len() + 1];
    suffix[groups.len()].attr_bonus[0] = diversity_bonus(diff_attr_bonus);
    // A deck takes at most one group of each character, so the top lists
    // rank each character's best value over its groups in the suffix.
    let mut tops = [CharacterTop::default(); 4];
    let mut idx = groups.len();
    while idx > 0 {
        idx -= 1;
        let group = &groups[idx];
        let values = [
            group.best_power,
            group.best_skill,
            group.best_base_bonus,
            group.best_limited_bonus,
        ];
        for (top, value) in tops.iter_mut().zip(values) {
            top.raise(group.char_id, value);
        }
        let next = suffix[idx + 1];
        let mut tail = GroupCeilingTail {
            versions: next.versions,
            top_power: tops[0].values(),
            top_skill: tops[1].values(),
            top_base_bonus: tops[2].values(),
            top_limited_bonus: tops[3].values(),
            attr_bonus: next.attr_bonus,
        };
        for picked in 1..=MEMBER_COUNT {
            attr_step(
                &next.attr_bonus[picked - 1],
                group.attr_mask,
                &mut tail.attr_bonus[picked],
            );
        }
        // Every field only grows as the suffix does, so a version never
        // returns to an earlier table.
        for open in 1..=MEMBER_COUNT {
            if tail.read_by(open) != next.read_by(open) {
                tail.versions[open] += 1;
            }
        }
        suffix[idx] = tail;
    }
    suffix
}

/// The largest per-character maxima seen so far, best first, each
/// character at most once. Maxima only grow, so a character that falls off
/// the list re-enters with its current maximum.
#[derive(Clone, Copy, Default)]
struct CharacterTop {
    entries: [(u32, u8); MEMBER_COUNT + 1],
    len: usize,
}

impl CharacterTop {
    fn raise(&mut self, char_id: u8, value: u32) {
        let mut at = match self.entries[..self.len]
            .iter()
            .position(|&(_, owner)| owner == char_id)
        {
            Some(at) if value > self.entries[at].0 => at,
            Some(_) => return,
            None if self.len < self.entries.len() => {
                self.len += 1;
                self.len - 1
            }
            None if value > self.entries[self.len - 1].0 => self.len - 1,
            None => return,
        };
        self.entries[at] = (value, char_id);
        while at > 0 && self.entries[at - 1].0 < self.entries[at].0 {
            self.entries.swap(at - 1, at);
            at -= 1;
        }
    }

    fn values(&self) -> [u32; MEMBER_COUNT + 1] {
        core::array::from_fn(|slot| {
            if slot < self.len {
                self.entries[slot].0
            } else {
                0
            }
        })
    }
}

#[inline]
fn extend_attr_union_states(states: u32, attr_mask: u8) -> u32 {
    let mut out = 0u32;
    let mut pending_states = states;
    while pending_states != 0 {
        let union = pending_states.trailing_zeros() as u8;
        pending_states &= pending_states - 1;
        let mut attrs = attr_mask;
        while attrs != 0 {
            let attr = attrs.trailing_zeros() as u8;
            attrs &= attrs - 1;
            out |= 1u32 << (union | (1u8 << attr));
        }
    }
    out
}

pub(crate) fn search_fixed_leader(
    pool: &CardPool,
    ctx: &SearchContext,
    params: &SearchParams,
    floor: u64,
    guard: &mut DeadlineGuard,
) -> (Vec<DeckResult>, SearchStats) {
    if params.top_k == 0 || pool.count() < DECK_SIZE {
        return (Vec::new(), SearchStats::default());
    }
    let Some(leader_char) = ctx.final_chapter_leader_character() else {
        return (Vec::new(), SearchStats::default());
    };
    search_leaders(pool, ctx, params, Some(leader_char), floor, guard)
}

pub(crate) fn search_auto_leader(
    pool: &CardPool,
    ctx: &SearchContext,
    params: &SearchParams,
    floor: u64,
    guard: &mut DeadlineGuard,
) -> (Vec<DeckResult>, SearchStats) {
    search_leaders(pool, ctx, params, None, floor, guard)
}

fn search_leaders(
    pool: &CardPool,
    ctx: &SearchContext,
    params: &SearchParams,
    leader_char_filter: Option<u8>,
    floor: u64,
    guard: &mut DeadlineGuard,
) -> (Vec<DeckResult>, SearchStats) {
    if params.top_k == 0 || pool.count() < DECK_SIZE {
        return (Vec::new(), SearchStats::default());
    }

    let suffix = SuffixBound::build(pool, ctx);
    // member 位图由 search_instrumented 统一计算（含支援惩罚维度与替代记录），
    // 经 ctx 透传；空位图等价全保留。
    let member_keep = ctx.final_chapter_member_keep.clone();
    let mut tracker = TopKTracker::with_floor(params.top_k, floor);
    tracker.set_bounds_enabled(crate::search::tuning::SearchTuning::load().bounds);
    let mut stats = SearchStats::default();
    if leader_char_filter.is_none() {
        return search_auto_leaders_two_phase(
            pool,
            ctx,
            params,
            &suffix,
            &member_keep,
            guard,
            tracker,
            stats,
        );
    }
    let mut leader_chars = Vec::new();
    if let Some(leader_char) = leader_char_filter {
        leader_chars.push(leader_char);
    } else {
        for character_id in 0..=26 {
            leader_chars.push(character_id);
        }
    }

    let buckets = attribute_buckets(pool);
    for leader_char in leader_chars {
        if guard.expired() {
            break;
        }
        let groups =
            build_char_groups(pool, ctx, &buckets, leader_char, &member_keep, params.top_k);
        if groups.len() < MEMBER_COUNT {
            continue;
        }
        let group_suffix = build_group_ceiling_suffix(&groups, &ctx.diff_attr_bonus);
        let mut leaders = pool
            .indices()
            .filter(|card| pool.char_id(*card) == leader_char)
            .collect::<Vec<_>>();
        leaders.sort_unstable_by(|left, right| {
            final_chapter_card_key(pool, *right)
                .cmp(&final_chapter_card_key(pool, *left))
                .then_with(|| left.raw().cmp(&right.raw()))
        });
        // Exact path: every leader variant must remain reachable.  Heuristic
        // per-character caps are unsound under Final Chapter support occupancy,
        // leader-only bonuses and Top-K set identity.  Job/character ceilings
        // below are the only mechanism allowed to discard a leader.
        for leader in leaders {
            if guard.expired() {
                break;
            }
            stats.diagnostics.leader_jobs += 1;
            let leader_const = build_leader_const(pool, ctx, leader);
            let leader_ceiling = character_ceiling(
                &suffix,
                ctx,
                &group_suffix,
                0,
                0,
                &CharacterPrefix::for_leader(&leader_const),
                &leader_const,
            );
            let threshold = tracker.threshold();
            if threshold != 0 && leader_ceiling < threshold {
                stats.leader_prunes += 1;
                continue;
            }
            // Seed only a leader whose admissible ceiling survives the current
            // incumbent. Seeding is heuristic ordering work, never proof work.
            seed_leader_groups(
                pool,
                ctx,
                &groups,
                &leader_const,
                &mut tracker,
                &mut stats,
                guard,
            );
            if tracker.threshold() != 0 && leader_ceiling < tracker.threshold() {
                stats.leader_prunes += 1;
                continue;
            }
            let mut state = CharacterSearchState {
                pool,
                ctx,
                suffix: &suffix,
                groups: &groups,
                group_suffix: &group_suffix,
                support: ctx.support_deck_for_leader(leader_char),
                uniform_limited_cap: uniform_limited_cap(pool, ctx.card_bonus_count_limit),
                diversity: diversity_bonus(&ctx.diff_attr_bonus),
                tracker: &mut tracker,
                stats: &mut stats,
                deadline: guard,
                leader: leader_const,
            };
            state.run();
        }
    }

    stats.deadline_hit = guard.hit;
    stats.finalize();
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
    let buckets = attribute_buckets(pool);
    let mut jobs = Vec::new();
    let mut group_sets = Vec::new();
    for leader_char in 0..=26 {
        if guard.expired() {
            break;
        }
        let groups = build_char_groups(pool, ctx, &buckets, leader_char, member_keep, params.top_k);
        if groups.len() < MEMBER_COUNT {
            continue;
        }
        let group_suffix = build_group_ceiling_suffix(&groups, &ctx.diff_attr_bonus);
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
        // Exact auto-leader jobs cover every surviving card; only an
        // admissible job ceiling below the threshold may discard one.
        for leader in leaders {
            if guard.expired() {
                break;
            }
            stats.diagnostics.leader_jobs += 1;
            let leader_const = build_leader_const(pool, ctx, leader);
            let ceiling = character_ceiling(
                suffix,
                ctx,
                &group_suffix,
                0,
                0,
                &CharacterPrefix::for_leader(&leader_const),
                &leader_const,
            );
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
        if tracker.threshold() != 0 && job.ceiling < tracker.threshold() {
            stats.leader_prunes += 1;
            continue;
        }
        let group_set = &group_sets[job.group_set];
        seed_leader_groups(
            pool,
            ctx,
            &group_set.groups,
            &job.leader,
            &mut tracker,
            &mut stats,
            guard,
        );
        if tracker.threshold() != 0 && job.ceiling < tracker.threshold() {
            stats.leader_prunes += 1;
            continue;
        }
        let mut state = CharacterSearchState {
            pool,
            ctx,
            suffix,
            groups: &group_set.groups,
            group_suffix: &group_set.suffix,
            support: ctx.support_deck_for_leader(pool.char_id(job.leader.leader)),
            uniform_limited_cap: uniform_limited_cap(pool, ctx.card_bonus_count_limit),
            diversity: diversity_bonus(&ctx.diff_attr_bonus),
            tracker: &mut tracker,
            stats: &mut stats,
            deadline: guard,
            leader: job.leader,
        };
        state.run();
    }

    stats.deadline_hit = guard.hit;
    stats.finalize();
    (tracker.into_vec(), stats)
}

fn seed_leader_groups(
    pool: &CardPool,
    ctx: &SearchContext,
    groups: &[CharGroup],
    leader: &LeaderConst,
    tracker: &mut TopKTracker,
    stats: &mut SearchStats,
    guard: &mut DeadlineGuard,
) {
    if !seeds_enabled() || guard.expired() {
        return;
    }
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
                    if guard.expired_sampled() {
                        return;
                    }
                    let indices = [a, b, c, d];
                    let chars = indices.map(|index| groups[index].char_id);
                    if (1..MEMBER_COUNT).any(|slot| chars[..slot].contains(&chars[slot])) {
                        d += 1;
                        continue;
                    }
                    stats.diagnostics.seed_states += 1;
                    let mut deck = [leader.leader; DECK_SIZE];
                    let mut slot = 0usize;
                    while slot < MEMBER_COUNT {
                        deck[slot + 1] = groups[indices[slot]].scan[0].card;
                        slot += 1;
                    }
                    stats.leaf_nodes += 1;
                    stats.diagnostics.seed_leaves += 1;
                    if let Some(candidate) = placement::evaluate_candidate(pool, ctx, &deck) {
                        tracker.insert(pool, ctx, candidate);
                    }
                    let mut variant = 0usize;
                    while variant < MEMBER_COUNT {
                        let group = &groups[indices[variant]];
                        if group.scan.len() > 1 {
                            let mut alt = deck;
                            alt[variant + 1] = group.scan[1].card;
                            stats.leaf_nodes += 1;
                            stats.diagnostics.seed_leaves += 1;
                            if let Some(candidate) = placement::evaluate_candidate(pool, ctx, &alt)
                            {
                                tracker.insert(pool, ctx, candidate);
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

/// Pool cards grouped by (character, attribute), built once per search.
type AttributeBuckets = Vec<((u8, u8), Vec<CardIdx>)>;

fn attribute_buckets(pool: &CardPool) -> AttributeBuckets {
    let mut cards = pool.indices().collect::<Vec<_>>();
    cards.sort_unstable_by_key(|&card| (pool.char_id(card), pool.attr(card), card.raw()));
    cards
        .chunk_by(|&left, &right| {
            (pool.char_id(left), pool.attr(left)) == (pool.char_id(right), pool.attr(right))
        })
        .map(|bucket| {
            (
                (pool.char_id(bucket[0]), pool.attr(bucket[0])),
                bucket.to_vec(),
            )
        })
        .collect()
}

fn build_char_groups(
    pool: &CardPool,
    ctx: &SearchContext,
    buckets: &AttributeBuckets,
    leader_char: u8,
    member_keep: &[bool],
    top_k: usize,
) -> Vec<CharGroup> {
    let leader_member_keep = (top_k == 1).then(|| {
        crate::search::dominance::compute_member_dominance_for_leader(pool, ctx, leader_char).keep
    });
    // One group per character and attribute: a deck takes at most one group
    // of each character, and a group fixes the attribute its card adds.
    let mut groups = Vec::new();
    let mut keyed = Vec::new();
    for &((char_id, _), ref cards) in buckets {
        if char_id == leader_char {
            continue;
        }
        keyed.clear();
        keyed.extend(
            cards
                .iter()
                .copied()
                .filter(|&card| {
                    member_keep.get(card.raw()).copied().unwrap_or(true)
                        && leader_member_keep
                            .as_ref()
                            .is_none_or(|keep| keep[card.raw()])
                })
                .map(|card| (final_chapter_member_key(pool, ctx, leader_char, card), card)),
        );
        if keyed.is_empty() {
            continue;
        }
        // Best first by the member key.
        keyed.sort_unstable_by(|left, right| {
            right
                .0
                .cmp(&left.0)
                .then_with(|| left.1.raw().cmp(&right.1.raw()))
        });
        let sorted_cards = keyed.iter().map(|&(_, card)| card).collect::<Vec<_>>();
        let scan = build_scan(pool, &sorted_cards);
        let best = scan[0].rest;
        groups.push(CharGroup {
            char_id,
            scan,
            best_power: best.power,
            best_skill: best.skill,
            best_base_bonus: best.base_bonus,
            best_limited_bonus: best.limited_bonus,
            attr_mask: 1u8 << best.attr,
            sort_key: final_chapter_group_key(
                best.power,
                best.skill,
                best.base_bonus,
                best.limited_bonus,
            ),
        });
    }

    groups.sort_unstable_by(|left, right| {
        right
            .sort_key
            .cmp(&left.sort_key)
            .then_with(|| left.char_id.cmp(&right.char_id))
            .then_with(|| left.attr_mask.cmp(&right.attr_mask))
    });
    groups
}

fn build_leader_const(pool: &CardPool, ctx: &SearchContext, leader: CardIdx) -> LeaderConst {
    let eb = pool.event_bonus_exact(leader);
    let limited_count = (eb.limited_x10() > 0 && ctx.card_bonus_count_limit > 0) as u8;
    LeaderConst {
        leader,
        power: pool.power_max(leader),
        skill: pool.skill_max(leader) as u32,
        base_bonus_const: eb.base_ceil() + ctx.leader_bonus_upper_at(leader.raw()),
        limited_bonus: eb.limited_ceil(),
        limited_count,
        extra_bonus_ub: final_chapter_extra_bonus_bound(pool, ctx, leader, &[], MEMBER_COUNT),
        support_bonus_ub: final_chapter_support_bonus_bound_for_leader(pool, ctx, leader),
        leader_attr_set: 1u8 << pool.attr(leader),
        use_group_attr_dp: ctx.is_world_bloom
            && crate::search::tuning::SearchTuning::load().final_attr_dp,
    }
}

struct CharacterSearchState<'a> {
    pool: &'a CardPool,
    ctx: &'a SearchContext,
    suffix: &'a SuffixBound,
    groups: &'a [CharGroup],
    group_suffix: &'a [GroupCeilingTail],
    support: &'a SupportDeck,
    uniform_limited_cap: Option<u32>,
    diversity: [u16; 32],
    tracker: &'a mut TopKTracker,
    stats: &'a mut SearchStats,
    deadline: &'a mut DeadlineGuard,
    leader: LeaderConst,
}

impl CharacterSearchState<'_> {
    fn run(&mut self) {
        // The initial support state depends only on this leader. Reuse it and
        // the ranked work buffers across every character-group combination.
        // Each card recursion reads only the buffer entries it has just filled.
        let initial_partial = CardPartial::for_leader(self.pool, self.ctx, &self.leader);
        let mut scratch = [[(0u64, CardIdx::new(0), initial_partial); RANKED_CAP]; MEMBER_COUNT];
        let mut selected = [0usize; MEMBER_COUNT];
        self.recurse_chars(
            0,
            0,
            0,
            &mut selected,
            CharacterPrefix::for_leader(&self.leader),
            &initial_partial,
            &mut scratch,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn recurse_chars(
        &mut self,
        depth: usize,
        start: usize,
        used_chars: u32,
        selected: &mut [usize; MEMBER_COUNT],
        prefix: CharacterPrefix,
        initial_partial: &CardPartial,
        scratch: &mut [[RankedSlot; RANKED_CAP]],
    ) {
        if self.deadline.expired_sampled() {
            return;
        }
        self.stats.visited_nodes += 1;
        if depth == MEMBER_COUNT {
            let threshold = self.tracker.threshold();
            // The four groups fix every member attribute, so this ceiling
            // reads the exact union before the card plan is built.
            if threshold != 0
                && character_ceiling(
                    self.suffix,
                    self.ctx,
                    self.group_suffix,
                    self.groups.len(),
                    MEMBER_COUNT,
                    &prefix,
                    &self.leader,
                ) < threshold
            {
                self.stats.ub_prunes += 1;
                return;
            }
            let mut ordered = *selected;
            order_card_groups(self.groups, &mut ordered);
            let mut deck = [self.leader.leader; DECK_SIZE];
            let plan = build_card_group_plan(
                self.groups,
                &ordered,
                &self.diversity,
                self.uniform_limited_cap,
            );
            self.recurse_cards(&ordered, &plan, 0, &mut deck, *initial_partial, scratch);
            return;
        }

        let mut threshold = self.tracker.threshold();
        // The ceiling reads the prefix, which is fixed here, and the suffix
        // table, so an unchanged table keeps the last ceiling.
        let mut last_ceiling: Option<(u32, u64)> = None;
        let mut idx = start;
        while idx < self.groups.len() {
            if self.deadline.expired_sampled() {
                return;
            }
            let group = &self.groups[idx];
            let char_bit = 1u32 << group.char_id;
            if used_chars & char_bit != 0 {
                idx += 1;
                continue;
            }
            // The first ceiling read is that of this node itself.
            if threshold != 0 {
                let version = self.group_suffix[idx].versions[MEMBER_COUNT - depth];
                let ub = match last_ceiling {
                    Some((seen, ub)) if seen == version => ub,
                    _ => {
                        let ub = character_ceiling(
                            self.suffix,
                            self.ctx,
                            self.group_suffix,
                            idx,
                            depth,
                            &prefix,
                            &self.leader,
                        );
                        last_ceiling = Some((version, ub));
                        ub
                    }
                };
                if ub < threshold {
                    self.stats.ub_prunes += 1;
                    break;
                }
            }
            selected[depth] = idx;
            self.stats.ep_candidates += 1;
            let next_prefix = prefix.with_group(group);
            idx += 1;
            self.recurse_chars(
                depth + 1,
                idx,
                used_chars | char_bit,
                selected,
                next_prefix,
                initial_partial,
                scratch,
            );
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
        self.stats.visited_nodes += 1;
        if depth == MEMBER_COUNT {
            self.stats.leaf_nodes += 1;
            if let Some(candidate) = placement::evaluate_candidate(self.pool, self.ctx, deck) {
                self.tracker.insert(self.pool, self.ctx, candidate);
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
            if ub < threshold {
                self.stats.ep_continue_prunes += 1;
                return;
            }
        }

        let group = &self.groups[selected[depth]];
        if threshold != 0 {
            let Some((ranked, scratch_tail)) = scratch.split_first_mut() else {
                return;
            };
            let head = group.scan.len().min(ranked.len());
            let mut ranked_len = 0usize;
            // A rest ceiling below the threshold also rules out the tail scan,
            // since the threshold never decreases.
            let mut tail_start = head;
            for entry in &group.scan[..head] {
                let Some(optimistic_ub) =
                    self.candidate_ceiling(plan, depth, &partial, entry, threshold)
                else {
                    self.stats.ep_continue_prunes += 1;
                    tail_start = group.scan.len();
                    break;
                };
                if optimistic_ub < threshold {
                    continue;
                }
                let card = entry.card;
                deck[depth + 1] = card;
                let next_partial =
                    partial.with_card(self.pool, self.ctx.is_world_bloom, self.support, card);
                let ub = self.child_ceiling(plan, depth, &partial, &next_partial, optimistic_ub);
                if ub < threshold {
                    continue;
                }
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
            let mut ranked_idx = 0usize;
            while ranked_idx < ranked_len {
                let (ub, card, next_partial) = ranked[ranked_idx];
                if ub < threshold {
                    self.stats.ep_continue_prunes += 1;
                    break;
                }
                deck[depth + 1] = card;
                self.recurse_cards(selected, plan, depth + 1, deck, next_partial, scratch_tail);
                threshold = self.tracker.threshold();
                ranked_idx += 1;
            }

            // 排序缓冲只覆盖组内扫描序的前 RANKED_CAP 张：它决定的是访问顺序，
            // 不是候选集。组更大时余下的卡仍要逐张过同一个上界——只有被上界
            // 拒绝才能不展开，按缓冲容量截断会把没被任何界否定过的卡静默丢掉。
            // 这一段在组不超过 RANKED_CAP 时不产生任何迭代。
            for entry in &group.scan[tail_start..] {
                threshold = self.tracker.threshold();
                let mut optimistic_ub = 0;
                if threshold != 0 {
                    let Some(ub) = self.candidate_ceiling(plan, depth, &partial, entry, threshold)
                    else {
                        self.stats.ep_continue_prunes += 1;
                        break;
                    };
                    if ub < threshold {
                        self.stats.ep_continue_prunes += 1;
                        continue;
                    }
                    optimistic_ub = ub;
                }
                let card = entry.card;
                let next_partial =
                    partial.with_card(self.pool, self.ctx.is_world_bloom, self.support, card);
                if threshold != 0 {
                    let ub =
                        self.child_ceiling(plan, depth, &partial, &next_partial, optimistic_ub);
                    if ub < threshold {
                        self.stats.ep_continue_prunes += 1;
                        continue;
                    }
                }
                deck[depth + 1] = card;
                self.recurse_cards(selected, plan, depth + 1, deck, next_partial, scratch_tail);
            }
        } else {
            for entry in &group.scan {
                let card = entry.card;
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

impl CharacterSearchState<'_> {
    /// Candidate ceiling of `entry`, or `None` when the ceiling over the rest
    /// of its group is already below `threshold`: every later card of the
    /// group has the same attribute and no larger term, and the ceiling is
    /// non-decreasing in every term.
    #[inline(always)]
    fn candidate_ceiling(
        &self,
        plan: &CardGroupPlan,
        depth: usize,
        partial: &CardPartial,
        entry: &ScanCard,
        threshold: u64,
    ) -> Option<u64> {
        let ceiling = |terms: &MemberTerms| {
            selected_card_ceiling_with_candidate_support_ub(
                self.suffix,
                self.ctx,
                plan,
                depth + 1,
                partial,
                terms,
                self.leader.skill,
            )
        };
        let rest_ub = ceiling(&entry.rest);
        if rest_ub < threshold {
            return None;
        }
        Some(if entry.rest == entry.terms {
            rest_ub
        } else {
            ceiling(&entry.terms)
        })
    }

    /// Ceiling of `next`, the child of `partial` that adds one card, given the
    /// candidate ceiling computed from `partial` with that card. The two differ
    /// only in the support ceiling, so an unchanged support sum reuses it.
    #[inline(always)]
    fn child_ceiling(
        &self,
        plan: &CardGroupPlan,
        depth: usize,
        partial: &CardPartial,
        next: &CardPartial,
        candidate_ub: u64,
    ) -> u64 {
        if next.support_bonus_ceil == partial.support_bonus_ceil {
            return candidate_ub;
        }
        selected_card_ceiling_from_partial(
            self.suffix,
            self.ctx,
            plan,
            depth + 1,
            next,
            self.leader.skill,
        )
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
    lhs.sort_key > rhs.sort_key || (lhs.sort_key == rhs.sort_key && lhs.scan.len() < rhs.scan.len())
}

fn character_ceiling(
    suffix: &SuffixBound,
    ctx: &SearchContext,
    group_suffix: &[GroupCeilingTail],
    start: usize,
    chosen: usize,
    prefix: &CharacterPrefix,
    leader: &LeaderConst,
) -> u64 {
    let tail = &group_suffix[start];

    let remaining = MEMBER_COUNT - chosen;
    let mut power_sum = leader.power + prefix.power;
    let mut skill_sum = leader.skill + prefix.skill;
    let mut bonus_sum = leader.base_bonus_const + leader.limited_bonus + prefix.base_bonus;
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
    // The open slots add at most `remaining` limited values.
    let limited_sum = merged_limited_sum(
        &prefix.limited_values,
        &tail.top_limited_bonus[..remaining],
        limited_limit.min(MEMBER_COUNT),
    );
    let extra_bonus_ub = if leader.use_group_attr_dp {
        final_chapter_character_attr_bonus_bound(tail, remaining, prefix.attr_states)
            + leader.support_bonus_ub
    } else {
        leader.extra_bonus_ub
    };
    suffix.objective().ceiling(
        power_sum,
        bonus_sum + limited_sum + extra_bonus_ub,
        skill_sum,
        final_chapter_ceiling_skill(
            ctx,
            leader.skill,
            prefix
                .max_skill
                .max(if remaining == 0 { 0 } else { tail.top_skill[0] }),
        ),
    )
}

/// Selected groups are mandatory. The suffix table already maximizes over
/// every remaining group and attribute choice for each selected union, including
/// nonmonotone diversity bonuses. Other score dimensions remain independent.
#[inline]
fn final_chapter_character_attr_bonus_bound(
    tail: &GroupCeilingTail,
    remaining: usize,
    selected_states: u32,
) -> u32 {
    let mut best = 0u32;
    let mut left = selected_states;
    while left != 0 {
        let selected_union = left.trailing_zeros() as usize;
        left &= left - 1;
        best = best.max(tail.attr_bonus[remaining][selected_union] as u32);
    }
    best
}

/// The generic live bound uses its fourth input as the whole-deck skill peak
/// for Solo/Auto non-Average orders. Multi and Average still use the real leader.
#[inline(always)]
fn final_chapter_ceiling_skill(ctx: &SearchContext, leader_skill: u32, skill_peak: u32) -> u32 {
    if matches!(ctx.effective_live_type(), LiveType::Solo | LiveType::Auto)
        && ctx.live_skill_order != LiveSkillOrder::Average
    {
        leader_skill.max(skill_peak)
    } else {
        leader_skill
    }
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
    let limited_sum = plan.limited_sum(partial, chosen, ctx.card_bonus_count_limit, 0);
    let extra_bonus_ub = if ctx.is_world_bloom {
        u32::from(plan.attr_bonus[chosen][partial.attr_set as usize]) + partial.support_bonus_ceil
    } else {
        ctx.extra_bonus_ub
    };
    suffix.objective().ceiling(
        power_sum,
        bonus_sum + limited_sum + extra_bonus_ub,
        skill_sum,
        final_chapter_ceiling_skill(
            ctx,
            leader_skill,
            partial.max_skill.max(plan.rem_max_skill[chosen]),
        ),
    )
}

/// Ceiling of every child of `partial` that adds a member with at most
/// `terms`, before that member's support exclusion. The ceiling is
/// non-decreasing in every numeric term.
fn selected_card_ceiling_with_candidate_support_ub(
    suffix: &SuffixBound,
    ctx: &SearchContext,
    plan: &CardGroupPlan,
    chosen: usize,
    partial: &CardPartial,
    terms: &MemberTerms,
    leader_skill: u32,
) -> u64 {
    let power_sum = partial.power + terms.power + plan.rem_power[chosen];
    let skill_sum = partial.skill + terms.skill + plan.rem_skill[chosen];
    let bonus_sum = partial.base_bonus + terms.base_bonus + plan.rem_base_bonus[chosen];
    let limited_sum = plan.limited_sum(
        partial,
        chosen,
        ctx.card_bonus_count_limit,
        terms.limited_bonus,
    );
    let extra_bonus_ub = if ctx.is_world_bloom {
        let attr_set = partial.attr_set | (1u8 << terms.attr);
        u32::from(plan.attr_bonus[chosen][attr_set as usize]) + partial.support_bonus_ceil
    } else {
        ctx.extra_bonus_ub
    };
    suffix.objective().ceiling(
        power_sum,
        bonus_sum + limited_sum + extra_bonus_ub,
        skill_sum,
        final_chapter_ceiling_skill(
            ctx,
            leader_skill,
            partial
                .max_skill
                .max(terms.skill)
                .max(plan.rem_max_skill[chosen]),
        ),
    )
}

#[inline(always)]
fn merged_limited_sum(left: &[u32], right: &[u32], cap: usize) -> u32 {
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

fn final_chapter_support_bonus_bound_for_leader(
    pool: &CardPool,
    ctx: &SearchContext,
    leader: CardIdx,
) -> u32 {
    if !ctx.is_world_bloom {
        return 0;
    }
    let mut selected = [0u16; DECK_SIZE];
    selected[0] = pool.game_id(leader);
    let support = ctx.support_deck_for_leader(pool.char_id(leader));
    remaining_support(support, &selected, 1).0.ceil() as u32
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

    let support = ctx.support_deck_for_leader(pool.char_id(leader));
    diff_ub + remaining_support(support, &selected, selected_len).0.ceil() as u32
}

/// Exact attribute-state DP over the already chosen character groups.  Each
/// remaining group must contribute one card whose attr is in its group mask;
/// tracking all 5-bit attr sets therefore gives an admissible (indeed exact for
/// the isolated attribute dimension) upper bound for the WL diversity bonus.
#[inline]
#[cfg(test)]
fn final_chapter_diff_attr_bound(
    ctx: &SearchContext,
    initial_attr_set: u8,
    future_group_attr_masks: &[u8],
) -> u32 {
    let mut states = 1u32 << initial_attr_set;
    for &group_mask in future_group_attr_masks {
        let mut next = 0u32;
        let mut state_bits = states;
        while state_bits != 0 {
            let set = state_bits.trailing_zeros() as u8;
            state_bits &= state_bits - 1;
            let mut attrs = group_mask;
            while attrs != 0 {
                let attr = attrs.trailing_zeros() as u8;
                attrs &= attrs - 1;
                next |= 1u32 << (set | (1u8 << attr));
            }
        }
        if next != 0 {
            states = next;
        }
    }
    let mut best = 0u32;
    let mut state_bits = states;
    while state_bits != 0 {
        let set = state_bits.trailing_zeros() as u8;
        state_bits &= state_bits - 1;
        best = best.max(ctx.diff_attr_bonus[set.count_ones() as usize] as u32);
    }
    best
}

/// Sum of the first `count` support entries whose game ids are not selected,
/// accumulated left to right in the evaluator's order, and the index after the
/// last scanned entry. The selected prefix excludes a subset of the final deck,
/// so the sum dominates the evaluator's (pruning-proof Lemma N5).
fn remaining_support(
    support: &SupportDeck,
    selected: &[u16; DECK_SIZE],
    selected_len: usize,
) -> (f64, usize) {
    let mut sum = 0.0_f64;
    let mut picked = 0usize;
    let mut next_scan = 0usize;
    while next_scan < support.cards.len() && picked < support.count as usize {
        let (game_id, bonus) = support.cards[next_scan];
        next_scan += 1;
        if selected_contains(selected, selected_len, game_id) {
            continue;
        }
        sum += bonus;
        picked += 1;
    }
    (sum, next_scan)
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

#[inline]
fn seeds_enabled() -> bool {
    crate::search::tuning::SearchTuning::load().final_seeds
}

#[cfg(test)]
mod limited_sum_tests {
    use super::*;
    use crate::pool::{EventBonusExact, PoolBuilder};

    fn pool(values: [u16; DECK_SIZE]) -> CardPool {
        let mut builder = PoolBuilder::new(DECK_SIZE as u16);
        for (dense, value) in values.into_iter().enumerate() {
            let index = dense as u16;
            builder.set_game_id(index, index + 1);
            builder.set_char_id(index, dense as u8);
            builder.set_event_bonus(index, EventBonusExact::from_x10(0, value));
        }
        builder.freeze()
    }

    #[test]
    fn uniform_limit_matches_independent_sorted_contributions() {
        let (_, mut ctx) = super::skill_ceiling_tests::fixture();
        for amount in [0, 1, 30, 175] {
            for pattern in 0..32 {
                let pool = pool(core::array::from_fn(|slot| {
                    if pattern & (1 << slot) == 0 {
                        0
                    } else {
                        amount
                    }
                }));
                for cap in 0..=DECK_SIZE {
                    ctx.card_bonus_count_limit = cap;
                    let groups =
                        build_char_groups(&pool, &ctx, &attribute_buckets(&pool), 0, &[], 1);
                    let plan = build_card_group_plan(
                        &groups,
                        &[0, 1, 2, 3],
                        &diversity_bonus(&ctx.diff_attr_bonus),
                        uniform_limited_cap(&pool, cap),
                    );
                    assert!(plan.uniform_limited_cap.is_some());
                    let leader = build_leader_const(&pool, &ctx, CardIdx::new(0));
                    let mut partial = CardPartial::for_leader(&pool, &ctx, &leader);
                    let mut values: Vec<_> = pool
                        .indices()
                        .map(|card| pool.event_bonus_exact(card).limited_ceil())
                        .collect();
                    values.sort_unstable_by(|a, b| b.cmp(a));
                    let expected: u32 = values.iter().take(cap).sum();
                    for (chosen, group) in groups.iter().enumerate() {
                        assert_eq!(plan.limited_sum(&partial, chosen, cap, 0), expected);
                        let card = group.scan[0].card;
                        let candidate = pool.event_bonus_exact(card).limited_ceil();
                        assert_eq!(
                            plan.limited_sum(&partial, chosen + 1, cap, candidate),
                            expected
                        );
                        partial =
                            partial.with_card(&pool, true, ctx.support_deck_for_leader(0), card);
                    }
                    assert_eq!(plan.limited_sum(&partial, MEMBER_COUNT, cap, 0), expected);
                }
            }
        }
        assert_eq!(uniform_limited_cap(&pool([0, 11, 19, 20, 0]), 4), Some(8));
        assert_eq!(uniform_limited_cap(&pool([0, 11, 21, 0, 0]), 4), None);
    }
}

#[cfg(test)]
mod skill_ceiling_tests {
    use super::*;
    use crate::pool::{EventBonusExact, PoolBuilder, SkillSlot};
    use crate::types::{EventType, ScoreTarget, SkillReferenceStrategy};

    pub(super) fn fixture() -> (CardPool, SearchContext) {
        let mut builder = PoolBuilder::new(5);
        for dense in 0..5u16 {
            let power = 67_200u32;
            let skill = if dense == 0 { 0 } else { 100 };
            let mut power_lut = 0u32;
            for slot in 0..8 {
                power_lut |= (power >> 16) << (slot * 2);
            }
            builder.set_game_id(dense, 100 + dense);
            builder.set_char_id(dense, dense as u8);
            builder.set_attr(dense, 0);
            builder.set_unit_mask(dense, 1);
            builder.set_power_values(dense, [power as u16; 8]);
            builder.set_power_lut(dense, power_lut);
            builder.set_power_max(dense, power);
            builder.set_skill(
                dense,
                SkillSlot {
                    skill_type: 0,
                    value: skill,
                },
            );
            builder.set_skill_min(dense, skill);
            builder.set_skill_max(dense, skill);
            builder.set_event_bonus(dense, EventBonusExact::from_whole(0, 0));
            builder.mark_char(dense as u8, dense);
            builder.mark_unit(0, dense);
        }
        let pool = builder.freeze();
        let ctx = SearchContext {
            target: ScoreTarget::Score,
            fixed_card_ids: Vec::new(),
            fixed_character_ids: Vec::new(),
            forced_leader_character_id: Some(0),
            music_rate_pct: 100,
            boost_rate_pct: 100,
            base_score: 1.0,
            base_score_auto: 1.0,
            fever_score: 0.0,
            skill_scores: [[1.0; 6]; 3],
            other_score: 0,
            life: 1_000,
            diff_attr_bonus: [0; 6],
            support_deck: SupportDeck::default(),
            support_decks_by_character: vec![SupportDeck::default(); 27],
            is_world_bloom: true,
            is_final_chapter: true,
            enforce_char_uniqueness: true,
            minimize: false,
            live_type: LiveType::Solo,
            event_type: Some(EventType::WorldBloom),
            skill_reference_strategy: SkillReferenceStrategy::Average,
            best_skill_as_leader: false,
            live_skill_order: LiveSkillOrder::Best,
            specific_skill_order: None,
            multi_teammate_score_up: None,
            multi_teammate_power: None,
            multi_live_score_up_lower_bound: None,
            extra_bonus_ub: 0,
            w_power: 2.0,
            w_bonus: 1.0,
            skill_ub_global: 0,
            card_bonus_count_limit: 4,
            honor_bonus: 0,
            power_total_cap: Some(336_000),
            leader_honor_bonus_x10: vec![0; 5],
            leader_honors: Vec::new(),
            leader_limit_bonus_x10: vec![0; 5],
            final_chapter_member_keep: vec![true; 5],
        };
        (pool, ctx)
    }

    #[test]
    fn final_skill_ceiling_preserves_real_leader_for_multi_and_average() {
        let (_, mut ctx) = fixture();
        for live in [
            LiveType::Multi,
            LiveType::Cheerful,
            LiveType::Solo,
            LiveType::Auto,
            LiveType::Challenge,
            LiveType::ChallengeAuto,
        ] {
            ctx.live_type = live;
            for order in [
                LiveSkillOrder::Average,
                LiveSkillOrder::Best,
                LiveSkillOrder::Worst,
                LiveSkillOrder::Specific,
            ] {
                ctx.live_skill_order = order;
                let effective = ctx.effective_live_type();
                let expected = if matches!(effective, LiveType::Solo | LiveType::Auto)
                    && order != LiveSkillOrder::Average
                {
                    100
                } else {
                    0
                };
                assert_eq!(final_chapter_ceiling_skill(&ctx, 0, 100), expected);
                assert_eq!(final_chapter_ceiling_skill(&ctx, 120, 100), 120);
            }
        }
    }

    #[test]
    fn final_skill_ceiling_three_call_sites_cover_stronger_members() {
        let (pool, mut ctx) = fixture();
        let deck = core::array::from_fn(|dense| CardIdx::new(dense as u16));
        for live in [LiveType::Solo, LiveType::Auto] {
            ctx.live_type = live;
            for order in [
                LiveSkillOrder::Average,
                LiveSkillOrder::Best,
                LiveSkillOrder::Worst,
                LiveSkillOrder::Specific,
            ] {
                ctx.live_skill_order = order;
                ctx.specific_skill_order =
                    (order == LiveSkillOrder::Specific).then_some([4, 1, 3, 0, 2]);
                let actual = crate::search::evaluate::leaf_evaluate_checked(&pool, &ctx, &deck)
                    .expect("the five-character fixture is legal");
                assert_eq!(actual as u32, 6_720_000);
                let suffix = SuffixBound::build(&pool, &ctx);
                if order != LiveSkillOrder::Average {
                    let invalid = suffix.objective().ceiling(336_000, 0, 400, 0);
                    assert_eq!(invalid as u32, 1_344_000);
                    assert!(
                        invalid < actual,
                        "old argument underestimates the legal leaf"
                    );
                }
                let groups = build_char_groups(&pool, &ctx, &attribute_buckets(&pool), 0, &[], 100);
                let selected = [0, 1, 2, 3];
                let group_suffix = build_group_ceiling_suffix(&groups, &ctx.diff_attr_bonus);
                let plan = build_card_group_plan(
                    &groups,
                    &selected,
                    &diversity_bonus(&ctx.diff_attr_bonus),
                    uniform_limited_cap(&pool, ctx.card_bonus_count_limit),
                );
                let leader = build_leader_const(&pool, &ctx, deck[0]);
                let mut prefix = CharacterPrefix::for_leader(&leader);
                let mut partial = CardPartial::for_leader(&pool, &ctx, &leader);
                for chosen in 0..=MEMBER_COUNT {
                    assert_eq!(
                        plan.rem_max_skill[chosen],
                        groups[chosen..]
                            .iter()
                            .map(|group| group.best_skill)
                            .max()
                            .unwrap_or(0),
                    );
                    let upper = character_ceiling(
                        &suffix,
                        &ctx,
                        &group_suffix,
                        chosen,
                        chosen,
                        &prefix,
                        &leader,
                    );
                    assert!(
                        upper >= actual,
                        "character stage live={live:?} order={order:?} chosen={chosen}"
                    );
                    let upper = selected_card_ceiling_from_partial(
                        &suffix,
                        &ctx,
                        &plan,
                        chosen,
                        &partial,
                        leader.skill,
                    );
                    assert!(
                        upper >= actual,
                        "card stage live={live:?} order={order:?} chosen={chosen}"
                    );
                    if chosen == MEMBER_COUNT {
                        break;
                    }
                    let card = groups[chosen].scan[0].card;
                    let upper = selected_card_ceiling_with_candidate_support_ub(
                        &suffix,
                        &ctx,
                        &plan,
                        chosen + 1,
                        &partial,
                        &MemberTerms::of(&pool, card),
                        leader.skill,
                    );
                    assert!(
                        upper >= actual,
                        "candidate stage live={live:?} order={order:?} chosen={chosen}"
                    );
                    prefix = prefix.with_group(&groups[chosen]);
                    partial = partial.with_card(&pool, true, ctx.support_deck_for_leader(0), card);
                    assert_eq!(partial.max_skill, 100);
                }
            }
        }
    }
}

#[cfg(test)]
mod attribute_bound_tests {
    use super::*;

    #[test]
    fn character_prefix_matches_selected_group_reductions() {
        for masks in [[1, 2, 4, 8, 16, 31], [3, 3, 5, 9, 17, 0], [31; 6]] {
            let groups: Vec<_> = masks
                .iter()
                .enumerate()
                .map(|(index, &attr_mask)| CharGroup {
                    char_id: index as u8,
                    scan: Vec::new(),
                    best_power: (index as u32 + 1) * 10_003,
                    best_skill: (index as u32 * 7) % 31,
                    best_base_bonus: (index as u32 * 13) % 47,
                    best_limited_bonus: [0, 5, 5, 20, 11, 2][index],
                    attr_mask,
                    sort_key: 0,
                })
                .collect();
            for use_group_attr_dp in [false, true] {
                for leader_attr_set in 0..32u8 {
                    let leader = LeaderConst {
                        leader: CardIdx::new(0),
                        power: 0,
                        skill: 0,
                        base_bonus_const: 0,
                        limited_bonus: 0,
                        limited_count: 0,
                        extra_bonus_ub: 0,
                        support_bonus_ub: 0,
                        leader_attr_set,
                        use_group_attr_dp,
                    };
                    for selection in 0..(1u32 << groups.len()) {
                        if selection.count_ones() as usize > MEMBER_COUNT {
                            continue;
                        }
                        let selected: Vec<_> = groups
                            .iter()
                            .enumerate()
                            .filter(|(index, _)| selection & (1u32 << *index) != 0)
                            .map(|(_, group)| group)
                            .collect();
                        let mut prefix = CharacterPrefix::for_leader(&leader);
                        for group in &selected {
                            prefix = prefix.with_group(group);
                        }
                        assert_eq!(
                            prefix.power,
                            selected.iter().map(|g| g.best_power).sum::<u32>()
                        );
                        assert_eq!(
                            prefix.skill,
                            selected.iter().map(|g| g.best_skill).sum::<u32>()
                        );
                        assert_eq!(
                            prefix.max_skill,
                            selected.iter().map(|g| g.best_skill).max().unwrap_or(0)
                        );
                        assert_eq!(
                            prefix.base_bonus,
                            selected.iter().map(|g| g.best_base_bonus).sum::<u32>()
                        );
                        // Reference uses full sorting, not the incremental insertion helper.
                        let mut limited: Vec<_> =
                            selected.iter().map(|g| g.best_limited_bonus).collect();
                        limited.resize(MEMBER_COUNT + 1, 0);
                        limited.sort_unstable_by(|left, right| right.cmp(left));
                        assert_eq!(prefix.limited_values.as_slice(), limited.as_slice());

                        // Reference enumerates boolean union states without using the
                        // production bitset transition or prefix implementation.
                        let mut reachable = [false; 32];
                        reachable[leader_attr_set as usize] = use_group_attr_dp;
                        for group in &selected {
                            let mut next = [false; 32];
                            for (set, &present) in reachable.iter().enumerate() {
                                if present {
                                    for attr in 0..5 {
                                        if group.attr_mask & (1u8 << attr) != 0 {
                                            next[set | (1 << attr)] = true;
                                        }
                                    }
                                }
                            }
                            reachable = next;
                        }
                        let expected = reachable
                            .iter()
                            .enumerate()
                            .fold(0u32, |bits, (set, &yes)| {
                                bits | if yes { 1u32 << set } else { 0 }
                            });
                        assert_eq!(
                            prefix.attr_states, expected,
                            "masks={masks:?} leader_attr_set={leader_attr_set} selection={selection} enabled={use_group_attr_dp}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn fixed_group_attr_table_matches_independent_union_enumeration() {
        for masks in [[1, 2, 4, 8], [3, 3, 5, 9], [31; MEMBER_COUNT]] {
            let groups = masks
                .iter()
                .map(|&attr_mask| CharGroup {
                    char_id: 0,
                    scan: Vec::new(),
                    best_power: 0,
                    best_skill: 0,
                    best_base_bonus: 0,
                    best_limited_bonus: 0,
                    attr_mask,
                    sort_key: 0,
                })
                .collect::<Vec<_>>();
            for bonuses in [[0, 0, 10, 20, 30, 50], [99, 70, 200, 3, 150, 0]] {
                let (_, mut ctx) = super::skill_ceiling_tests::fixture();
                ctx.diff_attr_bonus = bonuses;
                let plan =
                    build_card_group_plan(&groups, &[0, 1, 2, 3], &diversity_bonus(&bonuses), None);
                for chosen in 0..=MEMBER_COUNT {
                    for set in 0..32 {
                        assert_eq!(
                            u32::from(plan.attr_bonus[chosen][set]),
                            final_chapter_diff_attr_bound(&ctx, set as u8, &masks[chosen..]),
                            "masks={masks:?} bonuses={bonuses:?} chosen={chosen} set={set}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn suffix_bonus_table_matches_exhaustive_attribute_choices() {
        fn enumerate(masks: &[u8], chosen: usize, set: usize, bonuses: &[u16; 6]) -> u16 {
            if chosen == 0 {
                return bonuses[set.count_ones() as usize];
            }
            if masks.len() < chosen {
                return 0;
            }
            let mut best = enumerate(&masks[1..], chosen, set, bonuses);
            for attr in 0..5 {
                if masks[0] & (1 << attr) != 0 {
                    best = best.max(enumerate(
                        &masks[1..],
                        chosen - 1,
                        set | (1 << attr),
                        bonuses,
                    ));
                }
            }
            best
        }

        for masks in [[1, 2, 4, 8, 16, 31], [3, 3, 5, 9, 17, 0], [31; 6]] {
            let groups: Vec<_> = masks
                .iter()
                .map(|&attr_mask| CharGroup {
                    char_id: 0,
                    scan: Vec::new(),
                    best_power: 0,
                    best_skill: 0,
                    best_base_bonus: 0,
                    best_limited_bonus: 0,
                    attr_mask,
                    sort_key: 0,
                })
                .collect();
            // Nonmonotone tables must maximize the bonus itself, not the count.
            for bonuses in [[0, 0, 10, 20, 30, 50], [99, 70, 200, 3, 150, 0], [0; 6]] {
                let suffix = build_group_ceiling_suffix(&groups, &bonuses);
                for start in 0..=masks.len() {
                    for chosen in 0..=MEMBER_COUNT {
                        for set in 0..32 {
                            assert_eq!(
                                suffix[start].attr_bonus[chosen][set],
                                enumerate(&masks[start..], chosen, set, &bonuses),
                                "masks={masks:?} bonuses={bonuses:?} start={start} chosen={chosen} set={set}"
                            );
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod scan_tests {
    use super::*;
    use crate::pool::{EventBonusExact, PoolBuilder};

    #[test]
    fn scan_keeps_the_member_order_and_carries_rest_maxima() {
        // (power, skill, base_x10, limited_x10), best first.
        let cards = [
            (50u32, 10u8, 100u16, 0u16),
            (40, 30, 50, 100),
            (45, 20, 200, 0),
            (60, 5, 50, 0),
            (10, 40, 0, 50),
        ];
        let mut builder = PoolBuilder::new(cards.len() as u16);
        for (dense, &(power, skill, base_x10, limited_x10)) in cards.iter().enumerate() {
            let dense = dense as u16;
            builder.set_game_id(dense, 10 + dense);
            builder.set_char_id(dense, 1);
            builder.set_attr(dense, 2);
            builder.set_power_max(dense, power);
            builder.set_skill_max(dense, skill);
            builder.set_event_bonus(
                dense,
                EventBonusExact {
                    base_x10,
                    limited_x10,
                },
            );
        }
        let pool = builder.freeze();
        let order = [4, 0, 3, 1, 2].map(CardIdx::new);
        let summary: Vec<_> = build_scan(&pool, &order)
            .iter()
            .map(|entry| {
                (
                    entry.card.raw(),
                    entry.rest.power,
                    entry.rest.skill,
                    entry.rest.base_bonus,
                    entry.rest.limited_bonus,
                )
            })
            .collect();
        assert_eq!(
            summary,
            [
                (4, 60, 40, 20, 10),
                (0, 60, 30, 20, 10),
                (3, 60, 30, 20, 10),
                (1, 45, 30, 20, 10),
                (2, 45, 20, 20, 0),
            ]
        );
    }

    #[test]
    fn character_top_ranks_each_character_once_by_its_maximum() {
        let mut state = 0x2545_f491_u32;
        let mut next = move |bound: u32| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state % bound
        };
        for _ in 0..200 {
            let mut top = CharacterTop::default();
            let mut best = [0u32; 27];
            for _ in 0..40 {
                let (char_id, value) = (next(9) as u8, next(50));
                top.raise(char_id, value);
                best[usize::from(char_id)] = best[usize::from(char_id)].max(value);
                let mut expected = best;
                expected.sort_unstable_by(|left, right| right.cmp(left));
                assert_eq!(top.values()[..], expected[..MEMBER_COUNT + 1]);
            }
        }
    }
}
