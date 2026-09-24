use core::cell::Cell;

use crate::search::budget::SearchBudget as DeadlineGuard;

use crate::pool::{CardIdx, CardPool};
use crate::types::{DECK_SIZE, LiveSkillOrder, LiveType};

use crate::search::context::{SearchContext, SupportDeck};
use crate::search::dfs::SearchStats;
use crate::search::log_linear::{FeatureBox, LogLinearBound};
use crate::search::objective::{CeilingInputs, ScoreCutoff};
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
    attr: u8,
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

    /// Log-linear weight of these terms; the limited bonus is not capped.
    #[inline(always)]
    fn weight(&self, bound: &LogLinearBound) -> f64 {
        bound.weigh(self.power, self.skill, self.base_bonus + self.limited_bonus)
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
    /// Attribute union of the leader and the selected groups.
    attr_set: u8,
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
            attr_set: leader.leader_attr_set,
        }
    }

    #[inline(always)]
    fn with_group(mut self, group: &CharGroup) -> Self {
        self.power += group.best_power;
        self.skill += group.best_skill;
        self.max_skill = self.max_skill.max(group.best_skill);
        self.base_bonus += group.best_base_bonus;
        insert_topk_u32(&mut self.limited_values, group.best_limited_bonus);
        self.attr_set |= 1u8 << group.attr;
        self
    }
}

#[derive(Clone)]
struct AutoLeaderJob {
    leader: LeaderConst,
    ceiling: u64,
}

/// The member groups of one leader character, their ceiling tables and
/// their log-linear weights.
struct GroupSet {
    groups: Vec<CharGroup>,
    suffix: Vec<GroupCeilingTail>,
    weights: GroupWeightCache,
}

impl GroupSet {
    fn build(
        pool: &CardPool,
        ctx: &SearchContext,
        buckets: &AttributeBuckets,
        leader_char: u8,
        member_keep: &[bool],
        top_k: usize,
        leaders: Option<LeaderRange>,
    ) -> Self {
        let groups = build_char_groups(pool, ctx, buckets, leader_char, member_keep, top_k);
        let suffix = build_group_ceiling_suffix(&groups, &ctx.diff_attr_bonus);
        let feature_box = leaders
            .filter(|_| groups.len() >= MEMBER_COUNT)
            .map(|leaders| leaders.feature_box(&suffix[0]));
        Self {
            groups,
            suffix,
            weights: GroupWeightCache {
                feature_box,
                attempted: 0,
                weights: None,
            },
        }
    }
}

/// Maxima over the leaders of one character, for a [`FeatureBox`].
#[derive(Clone, Copy)]
struct LeaderRange {
    power: u32,
    skill_min: u32,
    skill_max: u32,
    /// Base and limited bonus.
    bonus: u32,
    /// Attribute and support bonus, as [`extra_bonus_ceiling`] reads it.
    extra: u32,
}

impl LeaderRange {
    fn of<'a>(
        ctx: &SearchContext,
        leaders: impl IntoIterator<Item = &'a LeaderConst>,
    ) -> Option<Self> {
        let diversity = ctx.diff_attr_bonus.iter().copied().max().unwrap_or(0);
        leaders
            .into_iter()
            .map(|leader| Self {
                power: leader.power,
                skill_min: leader.skill,
                skill_max: leader.skill,
                bonus: leader.base_bonus_const + leader.limited_bonus,
                extra: if leader.use_group_attr_dp {
                    u32::from(diversity) + leader.support_bonus_ub
                } else {
                    leader.extra_bonus_ub
                },
            })
            .reduce(|left, right| Self {
                power: left.power.max(right.power),
                skill_min: left.skill_min.min(right.skill_min),
                skill_max: left.skill_max.max(right.skill_max),
                bonus: left.bonus.max(right.bonus),
                extra: left.extra.max(right.extra),
            })
    }

    /// Features of every deck of these leaders and the groups behind `tail`.
    fn feature_box(&self, tail: &GroupCeilingTail) -> FeatureBox {
        let sum = |values: &[u32; MEMBER_COUNT + 1]| values[..MEMBER_COUNT].iter().sum::<u32>();
        FeatureBox {
            power: self.power + sum(&tail.top_power),
            skill: self.skill_max + sum(&tail.top_skill),
            card_skill: self.skill_max.max(tail.top_skill[0]),
            leader_skill_min: self.skill_min,
            leader_skill_max: self.skill_max,
            bonus: self.bonus
                + self.extra
                + sum(&tail.top_base_bonus)
                + sum(&tail.top_limited_bonus),
        }
    }
}

/// Log-linear weights of a group set (pruning-proof Section 18.8), rebuilt
/// as the event-point threshold rises.
struct GroupWeightCache {
    /// `None` when the group set has too few groups for a deck.
    feature_box: Option<FeatureBox>,
    /// Event-point threshold of the last build attempt.
    attempted: u64,
    weights: Option<GroupWeights>,
}

impl GroupWeightCache {
    /// Rebuilds the weights once the event-point threshold has risen by
    /// 1/128 since the last attempt; returns whether it tried. A higher
    /// threshold narrows the chord and moves the tangent point closer to the
    /// decks that can still reach it.
    fn refresh(&mut self, suffix: &SuffixBound, groups: &[CharGroup], threshold_ep: u64) -> bool {
        let Some(feature_box) = self.feature_box else {
            return false;
        };
        if threshold_ep == 0 || threshold_ep <= self.attempted + self.attempted / 128 {
            return false;
        }
        self.attempted = threshold_ep;
        self.weights = LogLinearBound::new(suffix.objective(), &feature_box, threshold_ep)
            .map(|bound| GroupWeights::build(bound, groups));
        true
    }
}

/// Group weights under one [`LogLinearBound`].
struct GroupWeights {
    bound: LogLinearBound,
    /// The largest card weight of each group.
    group: Vec<f64>,
    /// From each suffix start, the largest group weights of distinct
    /// characters, best first.
    tail: Vec<[f64; MEMBER_COUNT]>,
}

impl GroupWeights {
    fn build(bound: LogLinearBound, groups: &[CharGroup]) -> Self {
        let group: Vec<f64> = groups
            .iter()
            .map(|group| {
                group
                    .scan
                    .iter()
                    .map(|entry| entry.terms.weight(&bound))
                    .fold(0.0, f64::max)
            })
            .collect();
        let mut tail = vec![[0.0; MEMBER_COUNT]; groups.len() + 1];
        let mut top = CharacterTop::default();
        for idx in (0..groups.len()).rev() {
            top.raise(groups[idx].char_id, group[idx]);
            tail[idx].copy_from_slice(&top.values()[..MEMBER_COUNT]);
        }
        Self { bound, group, tail }
    }

    /// The leader's own terms.
    fn leader(&self, leader: &LeaderConst) -> f64 {
        self.bound.constant
            + self.bound.leader_skill * f64::from(leader.skill)
            + self.bound.weigh(
                leader.power,
                leader.skill,
                leader.base_bonus_const + leader.limited_bonus,
            )
    }
}

#[derive(Clone, Copy)]
struct CardPartial {
    power: u32,
    skill: u32,
    max_skill: u32,
    base_bonus: u32,
    limited_values: [u32; MEMBER_COUNT + 1],
    limited_sum: u32,
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
    /// Diversity bonus of the deck, whose attribute union the leader and the
    /// selected groups fix.
    diversity_bonus: u32,
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
    leader_attr_set: u8,
    diversity: &[u16; 32],
    uniform_limited_cap: Option<u32>,
) -> CardGroupPlan {
    let attr_set = selected.iter().fold(leader_attr_set, |set, &group| {
        set | (1u8 << groups[group].attr)
    });
    let mut plan = CardGroupPlan {
        rem_power: [0; MEMBER_COUNT + 1],
        rem_skill: [0; MEMBER_COUNT + 1],
        rem_max_skill: [0; MEMBER_COUNT + 1],
        rem_base_bonus: [0; MEMBER_COUNT + 1],
        rem_limited_values: [[0; MEMBER_COUNT + 1]; MEMBER_COUNT + 1],
        rem_limited_sum: [0; MEMBER_COUNT + 1],
        uniform_limited_cap,
        diversity_bonus: u32::from(diversity[usize::from(attr_set)]),
    };
    let mut depth = MEMBER_COUNT;
    while depth > 0 {
        depth -= 1;
        let next = depth + 1;
        let group = &groups[selected[depth]];
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
/// with attribute `attr` joins the attribute union `set`, given the best bonus
/// `next` of every union after that card.
#[inline(always)]
fn attr_step(next: &[u16; 32], attr: u8, best: &mut [u16; 32]) {
    debug_assert!(attr < 5, "attributes index five-bit unions");
    let bit = 1usize << attr;
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

fn build_group_ceiling_suffix(
    groups: &[CharGroup],
    diff_attr_bonus: &[u16; 6],
) -> Vec<GroupCeilingTail> {
    let mut suffix = vec![GroupCeilingTail::default(); groups.len() + 1];
    suffix[groups.len()].attr_bonus[0] = diversity_bonus(diff_attr_bonus);
    // A deck takes at most one group of each character, so the top lists
    // rank each character's best value over its groups in the suffix.
    let mut tops = [CharacterTop::<u32>::default(); 4];
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
                group.attr,
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
struct CharacterTop<T = u32> {
    entries: [(T, u8); MEMBER_COUNT + 1],
    len: usize,
}

impl<T: Copy + Default + PartialOrd> CharacterTop<T> {
    fn raise(&mut self, char_id: u8, value: T) {
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

    fn values(&self) -> [T; MEMBER_COUNT + 1] {
        self.values_without(u8::MAX)
    }

    /// The values of every character but `skip`, best first. The first
    /// `MEMBER_COUNT` are the exact largest maxima of the other characters.
    fn values_without(&self, skip: u8) -> [T; MEMBER_COUNT + 1] {
        let mut values = [T::default(); MEMBER_COUNT + 1];
        let kept = self.entries[..self.len]
            .iter()
            .filter(|&&(_, owner)| owner != skip);
        for (slot, &(value, _)) in values.iter_mut().zip(kept) {
            *slot = value;
        }
        values
    }
}

/// Job ceilings of every leader character from one pass over the pool. The
/// top lists skip the leader's character; the attribute rows keep the groups
/// of every character, which can only raise them.
struct LeaderCeilingTails {
    tops: [CharacterTop<u32>; 4],
    attr_bonus: [[u16; 32]; MEMBER_COUNT + 1],
}

impl LeaderCeilingTails {
    fn build(
        pool: &CardPool,
        buckets: &AttributeBuckets,
        member_keep: &[bool],
        diff_attr_bonus: &[u16; 6],
    ) -> Self {
        let mut tops = [CharacterTop::<u32>::default(); 4];
        let mut attr_bonus = [[0u16; 32]; MEMBER_COUNT + 1];
        attr_bonus[0] = diversity_bonus(diff_attr_bonus);
        for &((char_id, attr), ref cards) in buckets {
            let Some(best) = cards
                .iter()
                .filter(|card| member_keep.get(card.raw()).copied().unwrap_or(true))
                .map(|&card| MemberTerms::of(pool, card))
                .reduce(MemberTerms::max)
            else {
                continue;
            };
            let values = [best.power, best.skill, best.base_bonus, best.limited_bonus];
            for (top, value) in tops.iter_mut().zip(values) {
                top.raise(char_id, value);
            }
            let next = attr_bonus;
            for picked in 1..=MEMBER_COUNT {
                attr_step(&next[picked - 1], attr, &mut attr_bonus[picked]);
            }
        }
        Self { tops, attr_bonus }
    }

    /// A table for [`character_ceiling`] with every member slot open.
    fn for_leader(&self, leader_char: u8) -> GroupCeilingTail {
        GroupCeilingTail {
            top_power: self.tops[0].values_without(leader_char),
            top_skill: self.tops[1].values_without(leader_char),
            top_base_bonus: self.tops[2].values_without(leader_char),
            top_limited_bonus: self.tops[3].values_without(leader_char),
            attr_bonus: self.attr_bonus,
            ..GroupCeilingTail::default()
        }
    }
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
    let Some(leader_char) = leader_char_filter else {
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
    };
    let mut leaders = pool
        .indices()
        .filter(|card| pool.char_id(*card) == leader_char)
        .collect::<Vec<_>>();
    leaders.sort_unstable_by(|left, right| {
        final_chapter_card_key(pool, *right)
            .cmp(&final_chapter_card_key(pool, *left))
            .then_with(|| left.raw().cmp(&right.raw()))
    });
    let leaders: Vec<LeaderConst> = leaders
        .into_iter()
        .map(|leader| build_leader_const(pool, ctx, leader))
        .collect();
    let mut group_set = GroupSet::build(
        pool,
        ctx,
        &attribute_buckets(pool),
        leader_char,
        &member_keep,
        params.top_k,
        LeaderRange::of(ctx, &leaders),
    );
    if group_set.groups.len() >= MEMBER_COUNT {
        let mut cutoff = ScoreCutoff::new(suffix.objective());
        // Exact path: every leader variant must remain reachable.  Heuristic
        // per-character caps are unsound under Final Chapter support occupancy,
        // leader-only bonuses and Top-K set identity.  Job/character ceilings
        // below are the only mechanism allowed to discard a leader.
        for leader_const in leaders {
            if guard.expired() {
                break;
            }
            stats.diagnostics.leader_jobs += 1;
            let leader_ceiling = character_ceiling(
                &suffix,
                ctx,
                &group_set.suffix,
                0,
                0,
                &CharacterPrefix::for_leader(&leader_const),
                &leader_const,
            );
            let threshold = tracker.rank_threshold(ctx.target);
            if threshold != 0 && leader_ceiling < threshold {
                stats.leader_prunes += 1;
                continue;
            }
            // Seed only a leader whose admissible ceiling survives the current
            // incumbent. Seeding is heuristic ordering work, never proof work.
            seed_leader_groups(
                pool,
                ctx,
                &group_set.groups,
                &leader_const,
                &mut tracker,
                &mut stats,
                guard,
            );
            let threshold = tracker.rank_threshold(ctx.target);
            if threshold != 0 && leader_ceiling < threshold {
                stats.leader_prunes += 1;
                continue;
            }
            CharacterSearchState::new(
                pool,
                ctx,
                &suffix,
                &mut group_set,
                leader_const,
                &mut tracker,
                &mut stats,
                guard,
                &mut cutoff,
            )
            .run();
        }
    }

    stats.deadline_hit = guard.hit;
    stats.finalize();
    (tracker.into_vec(), stats)
}

#[allow(clippy::too_many_arguments)]
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
    let leader_tails = LeaderCeilingTails::build(pool, &buckets, member_keep, &ctx.diff_attr_bonus);
    let mut jobs = Vec::new();
    let mut ranges: [Option<LeaderRange>; 27] = [None; 27];
    for leader_char in 0..=26 {
        if guard.expired() {
            break;
        }
        let tail = [leader_tails.for_leader(leader_char)];
        let mut leaders = pool
            .indices()
            .filter(|card| pool.char_id(*card) == leader_char)
            .collect::<Vec<_>>();
        leaders.sort_unstable_by(|left, right| {
            final_chapter_card_key(pool, *right)
                .cmp(&final_chapter_card_key(pool, *left))
                .then_with(|| left.raw().cmp(&right.raw()))
        });
        let leaders: Vec<LeaderConst> = leaders
            .into_iter()
            .map(|leader| build_leader_const(pool, ctx, leader))
            .collect();
        ranges[usize::from(leader_char)] = LeaderRange::of(ctx, &leaders);
        // Exact auto-leader jobs cover every surviving card; only an
        // admissible job ceiling below the threshold may discard one.
        for leader_const in leaders {
            if guard.expired() {
                break;
            }
            stats.diagnostics.leader_jobs += 1;
            let ceiling = character_ceiling(
                suffix,
                ctx,
                &tail,
                0,
                0,
                &CharacterPrefix::for_leader(&leader_const),
                &leader_const,
            );
            jobs.push(AutoLeaderJob {
                leader: leader_const,
                ceiling,
            });
        }
    }

    jobs.sort_unstable_by(|left, right| {
        right
            .ceiling
            .cmp(&left.ceiling)
            .then_with(|| left.leader.leader.raw().cmp(&right.leader.leader.raw()))
    });
    // A leader character's group set is built when its first job runs.
    let mut group_sets: [Option<GroupSet>; 27] = Default::default();
    let mut cutoff = ScoreCutoff::new(suffix.objective());
    for job in jobs {
        if guard.expired() {
            break;
        }
        let threshold = tracker.rank_threshold(ctx.target);
        if threshold != 0 && job.ceiling < threshold {
            stats.leader_prunes += 1;
            continue;
        }
        let leader_char = pool.char_id(job.leader.leader);
        let group_set = group_sets[usize::from(leader_char)].get_or_insert_with(|| {
            GroupSet::build(
                pool,
                ctx,
                &buckets,
                leader_char,
                member_keep,
                params.top_k,
                ranges[usize::from(leader_char)],
            )
        });
        if group_set.groups.len() < MEMBER_COUNT {
            continue;
        }
        // The character's own table reads a subset of the cards and groups
        // behind the job ceiling, so it bounds the job at least as tightly.
        let ceiling = character_ceiling(
            suffix,
            ctx,
            &group_set.suffix,
            0,
            0,
            &CharacterPrefix::for_leader(&job.leader),
            &job.leader,
        );
        let threshold = tracker.rank_threshold(ctx.target);
        if threshold != 0 && ceiling < threshold {
            stats.leader_prunes += 1;
            continue;
        }
        seed_leader_groups(
            pool,
            ctx,
            &group_set.groups,
            &job.leader,
            &mut tracker,
            &mut stats,
            guard,
        );
        let threshold = tracker.rank_threshold(ctx.target);
        if threshold != 0 && ceiling < threshold {
            stats.leader_prunes += 1;
            continue;
        }
        CharacterSearchState::new(
            pool,
            ctx,
            suffix,
            group_set,
            job.leader,
            &mut tracker,
            &mut stats,
            guard,
            &mut cutoff,
        )
        .run();
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
    // Seeds only fill a tracker that has no threshold yet; once it holds K
    // decks, later leaders start from that threshold.
    if !seeds_enabled() || guard.expired() || tracker.rank_threshold(ctx.target) != 0 {
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
            attr: best.attr,
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
            .then_with(|| left.attr.cmp(&right.attr))
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
    weights: &'a mut GroupWeightCache,
    /// The leader's log-linear terms under the current weights.
    leader_weight: f64,
    support: &'a SupportDeck,
    uniform_limited_cap: Option<u32>,
    diversity: [u16; 32],
    tracker: &'a mut TopKTracker,
    stats: &'a mut SearchStats,
    deadline: &'a mut DeadlineGuard,
    leader: LeaderConst,
    /// Decides the Score ceilings against the threshold; shared by the
    /// leaders of one search, whose objective is the same.
    cutoff: &'a mut ScoreCutoff,
    /// The event-point threshold of the last log-linear test and its
    /// [`LogLinearBound::log_cutoff`].
    log_cutoff: Cell<(u64, f64)>,
}

impl<'a> CharacterSearchState<'a> {
    #[allow(clippy::too_many_arguments)]
    fn new(
        pool: &'a CardPool,
        ctx: &'a SearchContext,
        suffix: &'a SuffixBound,
        group_set: &'a mut GroupSet,
        leader: LeaderConst,
        tracker: &'a mut TopKTracker,
        stats: &'a mut SearchStats,
        deadline: &'a mut DeadlineGuard,
        cutoff: &'a mut ScoreCutoff,
    ) -> Self {
        let leader_weight = group_set
            .weights
            .weights
            .as_ref()
            .map_or(0.0, |weights| weights.leader(&leader));
        Self {
            pool,
            ctx,
            suffix,
            groups: &group_set.groups,
            group_suffix: &group_set.suffix,
            weights: &mut group_set.weights,
            leader_weight,
            support: ctx.support_deck_for_leader(pool.char_id(leader.leader)),
            uniform_limited_cap: uniform_limited_cap(pool, ctx.card_bonus_count_limit),
            diversity: diversity_bonus(&ctx.diff_attr_bonus),
            tracker,
            stats,
            deadline,
            leader,
            cutoff,
            log_cutoff: Cell::new((0, f64::NEG_INFINITY)),
        }
    }
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
            let threshold = self.tracker.rank_threshold(self.ctx.target);
            // The four groups fix every member attribute, so this ceiling
            // reads the exact union before the card plan is built.
            if threshold != 0
                && !self.reaches(
                    character_ceiling_inputs(
                        self.ctx,
                        self.group_suffix,
                        self.groups.len(),
                        MEMBER_COUNT,
                        &prefix,
                        &self.leader,
                    ),
                    threshold,
                )
            {
                self.stats.ub_prunes += 1;
                return;
            }
            if threshold != 0 {
                self.refresh_weights(threshold);
                if self.groups_excluded(self.groups.len(), &selected[..], &prefix, threshold) {
                    self.stats.correlated_prunes += 1;
                    return;
                }
            }
            let mut ordered = *selected;
            order_card_groups(self.groups, &mut ordered);
            let mut deck = [self.leader.leader; DECK_SIZE];
            let plan = build_card_group_plan(
                self.groups,
                &ordered,
                self.leader.leader_attr_set,
                &self.diversity,
                self.uniform_limited_cap,
            );
            self.recurse_cards(&ordered, &plan, 0, &mut deck, *initial_partial, scratch);
            return;
        }

        let mut threshold = self.tracker.rank_threshold(self.ctx.target);
        // The ceiling reads the prefix, which is fixed here, and the suffix
        // table, so an unchanged table that reached a threshold still does.
        let mut reached: Option<(u32, u64)> = None;
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
                if reached != Some((version, threshold)) {
                    let inputs = character_ceiling_inputs(
                        self.ctx,
                        self.group_suffix,
                        idx,
                        depth,
                        &prefix,
                        &self.leader,
                    );
                    if !self.reaches(inputs, threshold) {
                        self.stats.ub_prunes += 1;
                        break;
                    }
                    reached = Some((version, threshold));
                }
                self.refresh_weights(threshold);
                if self.groups_excluded(idx, &selected[..depth], &prefix, threshold) {
                    self.stats.correlated_prunes += 1;
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
            threshold = self.tracker.rank_threshold(self.ctx.target);
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

        let mut threshold = self.tracker.rank_threshold(self.ctx.target);
        if threshold != 0 {
            let inputs =
                selected_card_inputs(self.ctx, plan, depth, &partial, None, self.leader.skill);
            if !self.reaches(inputs, threshold) {
                self.stats.ep_continue_prunes += 1;
                return;
            }
            self.refresh_weights(threshold);
            if self.cards_excluded(selected, plan, depth, &partial, None, threshold) {
                self.stats.correlated_prunes += 1;
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
                    self.candidate_ceiling(selected, plan, depth, &partial, entry, threshold)
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
                threshold = self.tracker.rank_threshold(self.ctx.target);
                ranked_idx += 1;
            }

            // 排序缓冲只覆盖组内扫描序的前 RANKED_CAP 张：它决定的是访问顺序，
            // 不是候选集。组更大时余下的卡仍要逐张过同一个上界——只有被上界
            // 拒绝才能不展开，按缓冲容量截断会把没被任何界否定过的卡静默丢掉。
            // 这一段在组不超过 RANKED_CAP 时不产生任何迭代。
            for entry in &group.scan[tail_start..] {
                threshold = self.tracker.rank_threshold(self.ctx.target);
                let mut optimistic_ub = 0;
                if threshold != 0 {
                    let Some(ub) =
                        self.candidate_ceiling(selected, plan, depth, &partial, entry, threshold)
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
    /// Whether the ceiling with `inputs` reaches `threshold`.
    #[inline(always)]
    fn reaches(&mut self, inputs: CeilingInputs, threshold: u64) -> bool {
        self.cutoff
            .reaches(self.suffix.objective(), inputs, threshold)
    }

    /// Candidate ceiling of `entry`, or `None` when a bound over the rest of
    /// its group is already below `threshold`: every later card of the group
    /// has the same attribute and no larger term, and both the ceiling and
    /// the log-linear weight are non-decreasing in every term. A candidate
    /// the weight rules out has ceiling zero.
    #[inline(always)]
    fn candidate_ceiling(
        &mut self,
        selected: &[usize; MEMBER_COUNT],
        plan: &CardGroupPlan,
        depth: usize,
        partial: &CardPartial,
        entry: &ScanCard,
        threshold: u64,
    ) -> Option<u64> {
        let leader_skill = self.leader.skill;
        let objective = self.suffix.objective();
        let rest = selected_card_inputs(
            self.ctx,
            plan,
            depth + 1,
            partial,
            Some(&entry.rest),
            leader_skill,
        );
        // The ranked buffer orders by the ceiling itself, so a card whose
        // own terms are the rest maxima computes it once for both tests.
        if entry.rest == entry.terms {
            let ceiling = objective.ceiling_of(rest);
            if ceiling < threshold
                || self.cards_excluded(selected, plan, depth, partial, Some(&entry.rest), threshold)
            {
                return None;
            }
            return Some(ceiling);
        }
        if !self.reaches(rest, threshold)
            || self.cards_excluded(selected, plan, depth, partial, Some(&entry.rest), threshold)
        {
            return None;
        }
        if self.cards_excluded(
            selected,
            plan,
            depth,
            partial,
            Some(&entry.terms),
            threshold,
        ) {
            return Some(0);
        }
        Some(objective.ceiling_of(selected_card_inputs(
            self.ctx,
            plan,
            depth + 1,
            partial,
            Some(&entry.terms),
            leader_skill,
        )))
    }

    /// Rebuilds the group weights for a risen threshold and the leader's
    /// terms under them.
    #[inline(always)]
    fn refresh_weights(&mut self, threshold: u64) {
        if self
            .weights
            .refresh(self.suffix, self.groups, threshold >> 32)
        {
            self.leader_weight = self
                .weights
                .weights
                .as_ref()
                .map_or(0.0, |weights| weights.leader(&self.leader));
        }
    }

    /// Whether the log-linear bound rules out every completion of the
    /// `selected` groups with groups from `start` on.
    #[inline(always)]
    fn groups_excluded(
        &self,
        start: usize,
        selected: &[usize],
        prefix: &CharacterPrefix,
        threshold: u64,
    ) -> bool {
        let Some(weights) = self.weights.weights.as_ref() else {
            return false;
        };
        let remaining = MEMBER_COUNT - selected.len();
        let extra = extra_bonus_ceiling(&self.group_suffix[start], remaining, prefix, &self.leader);
        let value = self.leader_weight
            + weights.bound.bonus * f64::from(extra)
            + selected
                .iter()
                .map(|&group| weights.group[group])
                .sum::<f64>()
            + weights.tail[start][..remaining].iter().sum::<f64>();
        let threshold_ep = threshold >> 32;
        weights
            .bound
            .excludes(value, threshold_ep, self.log_cutoff(threshold_ep))
    }

    /// Whether the log-linear bound rules out every completion of the card
    /// prefix `partial` at `depth`, with `candidate` in the next slot when
    /// given.
    #[inline(always)]
    fn cards_excluded(
        &self,
        selected: &[usize; MEMBER_COUNT],
        plan: &CardGroupPlan,
        depth: usize,
        partial: &CardPartial,
        candidate: Option<&MemberTerms>,
        threshold: u64,
    ) -> bool {
        let Some(weights) = self.weights.weights.as_ref() else {
            return false;
        };
        let bound = &weights.bound;
        let extra = if self.ctx.is_world_bloom {
            plan.diversity_bonus + partial.support_bonus_ceil
        } else {
            self.ctx.extra_bonus_ub
        };
        let open = depth + usize::from(candidate.is_some());
        let value = bound.constant
            + bound.leader_skill * f64::from(self.leader.skill)
            + bound.weigh(
                partial.power,
                partial.skill,
                partial.base_bonus + partial.limited_sum + extra,
            )
            + candidate.map_or(0.0, |terms| terms.weight(bound))
            + selected[open..]
                .iter()
                .map(|&group| weights.group[group])
                .sum::<f64>();
        let threshold_ep = threshold >> 32;
        bound.excludes(value, threshold_ep, self.log_cutoff(threshold_ep))
    }

    /// [`LogLinearBound::log_cutoff`] of `threshold_ep`, kept while the
    /// threshold holds.
    #[inline(always)]
    fn log_cutoff(&self, threshold_ep: u64) -> f64 {
        let (seen, cutoff) = self.log_cutoff.get();
        if seen == threshold_ep {
            return cutoff;
        }
        let cutoff = LogLinearBound::log_cutoff(threshold_ep);
        self.log_cutoff.set((threshold_ep, cutoff));
        cutoff
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
    suffix.objective().ceiling_of(character_ceiling_inputs(
        ctx,
        group_suffix,
        start,
        chosen,
        prefix,
        leader,
    ))
}

/// The inputs of [`character_ceiling`].
#[inline(always)]
fn character_ceiling_inputs(
    ctx: &SearchContext,
    group_suffix: &[GroupCeilingTail],
    start: usize,
    chosen: usize,
    prefix: &CharacterPrefix,
    leader: &LeaderConst,
) -> CeilingInputs {
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
    CeilingInputs {
        power: power_sum,
        bonus: bonus_sum + limited_sum + extra_bonus_ceiling(tail, remaining, prefix, leader),
        skill: skill_sum,
        leader: final_chapter_ceiling_skill(
            ctx,
            leader.skill,
            prefix
                .max_skill
                .max(if remaining == 0 { 0 } else { tail.top_skill[0] }),
        ),
    }
}

/// Attribute and support bonus of every completion of `prefix` that takes
/// `remaining` more groups from the suffix behind `tail`.
#[inline(always)]
fn extra_bonus_ceiling(
    tail: &GroupCeilingTail,
    remaining: usize,
    prefix: &CharacterPrefix,
    leader: &LeaderConst,
) -> u32 {
    if leader.use_group_attr_dp {
        u32::from(tail.attr_bonus[remaining][usize::from(prefix.attr_set)])
            + leader.support_bonus_ub
    } else {
        leader.extra_bonus_ub
    }
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
    suffix.objective().ceiling_of(selected_card_inputs(
        ctx,
        plan,
        chosen,
        partial,
        None,
        leader_skill,
    ))
}

/// The inputs of [`selected_card_ceiling_from_partial`]. With a
/// `candidate`, the inputs of every child of `partial` that adds a member
/// with at most its terms, before that member's support exclusion; the
/// ceiling is non-decreasing in every numeric term.
#[inline(always)]
fn selected_card_inputs(
    ctx: &SearchContext,
    plan: &CardGroupPlan,
    chosen: usize,
    partial: &CardPartial,
    candidate: Option<&MemberTerms>,
    leader_skill: u32,
) -> CeilingInputs {
    let (power, skill, base_bonus, limited_bonus) = candidate.map_or((0, 0, 0, 0), |terms| {
        (
            terms.power,
            terms.skill,
            terms.base_bonus,
            terms.limited_bonus,
        )
    });
    let power_sum = partial.power + power + plan.rem_power[chosen];
    let skill_sum = partial.skill + skill + plan.rem_skill[chosen];
    let bonus_sum = partial.base_bonus + base_bonus + plan.rem_base_bonus[chosen];
    let limited_sum = plan.limited_sum(partial, chosen, ctx.card_bonus_count_limit, limited_bonus);
    let extra_bonus_ub = if ctx.is_world_bloom {
        plan.diversity_bonus + partial.support_bonus_ceil
    } else {
        ctx.extra_bonus_ub
    };
    CeilingInputs {
        power: power_sum,
        bonus: bonus_sum + limited_sum + extra_bonus_ub,
        skill: skill_sum,
        leader: final_chapter_ceiling_skill(
            ctx,
            leader_skill,
            partial.max_skill.max(skill).max(plan.rem_max_skill[chosen]),
        ),
    }
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
                        1u8 << pool.attr(CardIdx::new(0)),
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
                    1u8 << pool.attr(deck[0]),
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
                    let upper = suffix.objective().ceiling_of(selected_card_inputs(
                        &ctx,
                        &plan,
                        chosen + 1,
                        &partial,
                        Some(&MemberTerms::of(&pool, card)),
                        leader.skill,
                    ));
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

    fn groups_of(attrs: &[u8]) -> Vec<CharGroup> {
        attrs
            .iter()
            .enumerate()
            .map(|(index, &attr)| CharGroup {
                char_id: index as u8,
                scan: Vec::new(),
                best_power: (index as u32 + 1) * 10_003,
                best_skill: (index as u32 * 7) % 31,
                best_base_bonus: (index as u32 * 13) % 47,
                best_limited_bonus: [0, 5, 5, 20, 11, 2][index],
                attr,
                sort_key: 0,
            })
            .collect()
    }

    #[test]
    fn character_prefix_matches_selected_group_reductions() {
        for attrs in [[0, 1, 2, 3, 4, 0], [1, 1, 0, 3, 4, 2], [4; 6]] {
            let groups = groups_of(&attrs);
            for leader_attr in 0..5u8 {
                let leader = LeaderConst {
                    leader: CardIdx::new(0),
                    power: 0,
                    skill: 0,
                    base_bonus_const: 0,
                    limited_bonus: 0,
                    limited_count: 0,
                    extra_bonus_ub: 0,
                    support_bonus_ub: 0,
                    leader_attr_set: 1u8 << leader_attr,
                    use_group_attr_dp: true,
                };
                for selection in 0..(1u32 << groups.len()) {
                    if selection.count_ones() as usize > MEMBER_COUNT {
                        continue;
                    }
                    let selected: Vec<_> = (0..groups.len())
                        .filter(|index| selection & (1u32 << index) != 0)
                        .collect();
                    let mut prefix = CharacterPrefix::for_leader(&leader);
                    for &index in &selected {
                        prefix = prefix.with_group(&groups[index]);
                    }
                    let picked = || selected.iter().map(|&index| &groups[index]);
                    assert_eq!(prefix.power, picked().map(|g| g.best_power).sum::<u32>());
                    assert_eq!(prefix.skill, picked().map(|g| g.best_skill).sum::<u32>());
                    assert_eq!(
                        prefix.max_skill,
                        picked().map(|g| g.best_skill).max().unwrap_or(0)
                    );
                    assert_eq!(
                        prefix.base_bonus,
                        picked().map(|g| g.best_base_bonus).sum::<u32>()
                    );
                    // Reference uses full sorting, not the incremental insertion helper.
                    let mut limited: Vec<_> = picked().map(|g| g.best_limited_bonus).collect();
                    limited.resize(MEMBER_COUNT + 1, 0);
                    limited.sort_unstable_by(|left, right| right.cmp(left));
                    assert_eq!(prefix.limited_values.as_slice(), limited.as_slice());

                    let mut present = [false; 5];
                    present[usize::from(leader_attr)] = true;
                    for group in picked() {
                        present[usize::from(group.attr)] = true;
                    }
                    let expected = (0..5)
                        .filter(|&attr| present[attr])
                        .fold(0u8, |set, attr| set | (1 << attr));
                    assert_eq!(
                        prefix.attr_set, expected,
                        "attrs={attrs:?} selection={selection}"
                    );

                    if let Ok(four) = <[usize; MEMBER_COUNT]>::try_from(selected.as_slice()) {
                        let bonuses = [0, 0, 10, 20, 30, 50];
                        let plan = build_card_group_plan(
                            &groups,
                            &four,
                            1u8 << leader_attr,
                            &diversity_bonus(&bonuses),
                            None,
                        );
                        let count = present.iter().filter(|&&yes| yes).count();
                        assert_eq!(plan.diversity_bonus, u32::from(bonuses[count]));
                    }
                }
            }
        }
    }

    #[test]
    fn suffix_bonus_table_matches_exhaustive_attribute_choices() {
        fn enumerate(attrs: &[u8], chosen: usize, set: usize, bonuses: &[u16; 6]) -> u16 {
            if chosen == 0 {
                return bonuses[set.count_ones() as usize];
            }
            if attrs.len() < chosen {
                return 0;
            }
            enumerate(&attrs[1..], chosen, set, bonuses).max(enumerate(
                &attrs[1..],
                chosen - 1,
                set | (1 << attrs[0]),
                bonuses,
            ))
        }

        for attrs in [[0, 1, 2, 3, 4, 0], [1, 1, 0, 3, 4, 2], [4; 6]] {
            let groups = groups_of(&attrs);
            // Nonmonotone tables must maximize the bonus itself, not the count.
            for bonuses in [[0, 0, 10, 20, 30, 50], [99, 70, 200, 3, 150, 0], [0; 6]] {
                let suffix = build_group_ceiling_suffix(&groups, &bonuses);
                for start in 0..=attrs.len() {
                    for chosen in 0..=MEMBER_COUNT {
                        for set in 0..32 {
                            assert_eq!(
                                suffix[start].attr_bonus[chosen][set],
                                enumerate(&attrs[start..], chosen, set, &bonuses),
                                "attrs={attrs:?} bonuses={bonuses:?} start={start} chosen={chosen} set={set}"
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
                let skip = next(9) as u8;
                let mut others = best;
                others[usize::from(skip)] = 0;
                others.sort_unstable_by(|left, right| right.cmp(left));
                assert_eq!(
                    top.values_without(skip)[..MEMBER_COUNT],
                    others[..MEMBER_COUNT]
                );
            }
        }
    }
}
