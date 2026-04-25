use std::time::{Duration, Instant};

use crate::pool::{CardIdx, CardPool};
use crate::types::{LiveSkillOrder, LiveType, ScoreTarget, DECK_SIZE};

use super::context::SearchContext;
use super::dfs::SearchStats;
use super::dominance;
use super::evaluate::{
    calc_event_point, decode_u18, leaf_evaluate_checked, resolve_power_target, resolve_total_bonus,
};
use super::suffix::SuffixBound;
use super::types::{DeckResult, SearchParams};

const MEMBER_COUNT: usize = 4;
const FINAL_CHAPTER_SEED_GROUP_PREFIX: usize = 6;

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
}

pub(crate) fn search_fixed_leader(
    pool: &CardPool,
    ctx: &SearchContext,
    params: &SearchParams,
) -> (Vec<DeckResult>, SearchStats) {
    if params.top_k == 0 || pool.count() < DECK_SIZE {
        return (Vec::new(), SearchStats::default());
    }
    let Some(leader_char) = ctx.fixed_character_at(0) else {
        return (Vec::new(), SearchStats::default());
    };

    let deadline = if params.timeout_ms == 0 {
        None
    } else {
        Some(Instant::now() + Duration::from_millis(params.timeout_ms))
    };
    let suffix = SuffixBound::build(pool, ctx);
    let member_keep = dominance::compute_member_keep(pool);
    let groups = build_char_groups(pool, ctx, leader_char, &member_keep);
    if groups.len() < MEMBER_COUNT {
        return (Vec::new(), SearchStats::default());
    }

    let mut tracker = TopKTracker::new(params.top_k);
    let mut stats = SearchStats::default();
    let mut leaders = pool
        .indices()
        .filter(|card| pool.char_id(*card) == leader_char)
        .collect::<Vec<_>>();
    leaders.sort_unstable_by(|left, right| {
        final_chapter_card_key(pool, *right)
            .cmp(&final_chapter_card_key(pool, *left))
            .then_with(|| left.raw().cmp(&right.raw()))
    });
    let leaders = filter_leader_variants(pool, ctx, leaders);

    for leader in leaders {
        if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            break;
        }
        let leader_const = build_leader_const(pool, ctx, leader);
        seed_leader_groups(pool, ctx, &groups, &leader_const, &mut tracker);
        seed_top_group_combinations(pool, ctx, &groups, &leader_const, &mut tracker);
        let threshold = tracker.threshold();
        if threshold != 0 {
            let ub = character_ceiling(&suffix, ctx, &groups, 0, &[], &leader_const);
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
            tracker: &mut tracker,
            stats: &mut stats,
            deadline,
            leader: leader_const,
        };
        state.recurse_chars(0, 0, &mut selected);
    }

    (tracker.into_vec(), stats)
}

fn seed_top_group_combinations(
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
                    let mut selected = [a, b, c, d];
                    selected.sort_unstable_by(|left, right| {
                        groups[*left]
                            .cards
                            .len()
                            .cmp(&groups[*right].cards.len())
                            .then_with(|| groups[*right].sort_key.cmp(&groups[*left].sort_key))
                    });
                    seed_exact_group_combo(pool, ctx, groups, leader, &selected, tracker);
                    d += 1;
                }
                c += 1;
            }
            b += 1;
        }
        a += 1;
    }
}

fn seed_exact_group_combo(
    pool: &CardPool,
    ctx: &SearchContext,
    groups: &[CharGroup],
    leader: &LeaderConst,
    selected: &[usize; MEMBER_COUNT],
    tracker: &mut TopKTracker,
) {
    let mut deck = [leader.leader; DECK_SIZE];
    seed_exact_group_combo_recurse(pool, ctx, groups, selected, 0, &mut deck, tracker);
}

fn seed_exact_group_combo_recurse(
    pool: &CardPool,
    ctx: &SearchContext,
    groups: &[CharGroup],
    selected: &[usize; MEMBER_COUNT],
    depth: usize,
    deck: &mut [CardIdx; DECK_SIZE],
    tracker: &mut TopKTracker,
) {
    if depth == MEMBER_COUNT {
        if let Some(score) = exact_final_chapter_leaf(pool, ctx, deck) {
            tracker.insert(DeckResult::new(*deck, score));
        }
        return;
    }
    let group = &groups[selected[depth]];
    let take = group.cards.len().min(3);
    let mut idx = 0usize;
    while idx < take {
        deck[depth + 1] = group.cards[idx];
        seed_exact_group_combo_recurse(pool, ctx, groups, selected, depth + 1, deck, tracker);
        idx += 1;
    }
}

fn seed_leader_groups(
    pool: &CardPool,
    ctx: &SearchContext,
    groups: &[CharGroup],
    leader: &LeaderConst,
    tracker: &mut TopKTracker,
) {
    let prefix_len = groups.len().min(6);
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

    let lhs_bonus = pool.event_bonus(lhs);
    let rhs_bonus = pool.event_bonus(rhs);
    if lhs_bonus.base_bonus < rhs_bonus.base_bonus
        || lhs_bonus.limited_bonus < rhs_bonus.limited_bonus
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
    _ctx: &SearchContext,
    leader_char: u8,
    member_keep: &[bool],
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
        let mut sorted_cards = cards;
        sorted_cards.sort_unstable_by(|left, right| {
            final_chapter_card_key(pool, *right)
                .cmp(&final_chapter_card_key(pool, *left))
                .then_with(|| left.raw().cmp(&right.raw()))
        });
        for card in &sorted_cards {
            let eb = pool.event_bonus(*card);
            best_power = best_power.max(pool.power_max(*card));
            best_skill = best_skill.max(pool.skill_max(*card) as u32);
            best_base_bonus = best_base_bonus.max(eb.base_bonus as u32);
            best_limited_bonus = best_limited_bonus.max(eb.limited_bonus as u32);
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

fn build_leader_const(pool: &CardPool, ctx: &SearchContext, leader: CardIdx) -> LeaderConst {
    let eb = pool.event_bonus(leader);
    let limited_count = (eb.limited_bonus > 0 && ctx.card_bonus_count_limit > 0) as u8;
    LeaderConst {
        leader,
        power: pool.power_max(leader),
        skill: pool.skill_max(leader) as u32,
        base_bonus_const: eb.base_bonus as u32
            + ctx.leader_honor_bonus_at(leader.raw())
            + ctx.leader_limit_bonus_at(leader.raw()),
        limited_bonus: eb.limited_bonus as u32,
        limited_count,
    }
}

struct CharacterSearchState<'a> {
    pool: &'a CardPool,
    ctx: &'a SearchContext,
    suffix: &'a SuffixBound,
    groups: &'a [CharGroup],
    tracker: &'a mut TopKTracker,
    stats: &'a mut SearchStats,
    deadline: Option<Instant>,
    leader: LeaderConst,
}

impl CharacterSearchState<'_> {
    fn recurse_chars(&mut self, depth: usize, start: usize, selected: &mut [usize; MEMBER_COUNT]) {
        if self
            .deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            return;
        }
        if depth == MEMBER_COUNT {
            let mut ordered = *selected;
            ordered.sort_unstable_by(|left, right| {
                self.groups[*left]
                    .cards
                    .len()
                    .cmp(&self.groups[*right].cards.len())
                    .then_with(|| {
                        self.groups[*right]
                            .sort_key
                            .cmp(&self.groups[*left].sort_key)
                    })
            });
            let mut deck = [self.leader.leader; DECK_SIZE];
            self.recurse_cards(&ordered, 0, &mut deck);
            return;
        }

        let mut threshold = self.tracker.threshold();
        if threshold != 0 {
            let ub = character_ceiling(
                self.suffix,
                self.ctx,
                self.groups,
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
            if self
                .deadline
                .is_some_and(|deadline| Instant::now() >= deadline)
            {
                return;
            }
            if threshold != 0 {
                let ub = character_ceiling(
                    self.suffix,
                    self.ctx,
                    self.groups,
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
        depth: usize,
        deck: &mut [CardIdx; DECK_SIZE],
    ) {
        if self
            .deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
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
            let ub = selected_card_ceiling(
                self.pool,
                self.suffix,
                self.ctx,
                self.groups,
                selected,
                depth,
                deck,
                &self.leader,
            );
            if ub <= threshold {
                self.stats.ep_continue_prunes += 1;
                return;
            }
        }

        let group = &self.groups[selected[depth]];
        if threshold != 0 {
            let mut ranked = Vec::with_capacity(group.cards.len());
            for &card in &group.cards {
                deck[depth + 1] = card;
                let ub = selected_card_ceiling(
                    self.pool,
                    self.suffix,
                    self.ctx,
                    self.groups,
                    selected,
                    depth + 1,
                    deck,
                    &self.leader,
                );
                ranked.push((ub, card));
            }
            ranked.sort_unstable_by(|left, right| {
                right
                    .0
                    .cmp(&left.0)
                    .then_with(|| left.1.raw().cmp(&right.1.raw()))
            });
            for (ub, card) in ranked {
                if ub <= threshold {
                    self.stats.ep_continue_prunes += 1;
                    break;
                }
                deck[depth + 1] = card;
                self.recurse_cards(selected, depth + 1, deck);
                threshold = self.tracker.threshold();
            }
        } else {
            for &card in &group.cards {
                deck[depth + 1] = card;
                self.recurse_cards(selected, depth + 1, deck);
            }
        }
    }
}

fn character_ceiling(
    suffix: &SuffixBound,
    ctx: &SearchContext,
    groups: &[CharGroup],
    start: usize,
    selected: &[usize],
    leader: &LeaderConst,
) -> u64 {
    let mut top_power = [0u32; MEMBER_COUNT];
    let mut top_skill = [0u32; MEMBER_COUNT];
    let mut top_base = [0u32; MEMBER_COUNT];
    let mut top_limited = [0u32; MEMBER_COUNT];
    let mut selected_power = 0u32;
    let mut selected_skill = 0u32;
    let mut selected_base = 0u32;
    let mut idx = 0usize;
    while idx < selected.len() {
        let group = &groups[selected[idx]];
        selected_power += group.best_power;
        selected_skill += group.best_skill;
        selected_base += group.best_base_bonus;
        insert_topk_u32(&mut top_limited, group.best_limited_bonus);
        idx += 1;
    }

    let mut group_index = start;
    while group_index < groups.len() {
        let group = &groups[group_index];
        insert_topk_u32(&mut top_power, group.best_power);
        insert_topk_u32(&mut top_skill, group.best_skill);
        insert_topk_u32(&mut top_base, group.best_base_bonus);
        insert_topk_u32(&mut top_limited, group.best_limited_bonus);
        group_index += 1;
    }

    let remaining = MEMBER_COUNT - selected.len();
    let mut power_sum = leader.power + selected_power;
    let mut skill_sum = leader.skill + selected_skill;
    let mut bonus_sum = leader.base_bonus_const + leader.limited_bonus + selected_base;
    let mut slot = 0usize;
    while slot < remaining {
        power_sum += top_power[slot];
        skill_sum += top_skill[slot];
        bonus_sum += top_base[slot];
        slot += 1;
    }

    let limited_limit = ctx
        .card_bonus_count_limit
        .saturating_sub(leader.limited_count as usize);
    let mut limited_sum = 0u32;
    let mut limited_slot = 0usize;
    while limited_slot < limited_limit.min(MEMBER_COUNT) {
        limited_sum += top_limited[limited_slot];
        limited_slot += 1;
    }
    suffix.ceiling(
        power_sum,
        bonus_sum + limited_sum + ctx.extra_bonus_ub,
        skill_sum,
        leader.skill,
    )
}

fn selected_card_ceiling(
    pool: &CardPool,
    suffix: &SuffixBound,
    ctx: &SearchContext,
    groups: &[CharGroup],
    selected: &[usize; MEMBER_COUNT],
    chosen: usize,
    deck: &[CardIdx; DECK_SIZE],
    leader: &LeaderConst,
) -> u64 {
    let mut power_sum = leader.power;
    let mut skill_sum = leader.skill;
    let mut bonus_sum = leader.base_bonus_const;
    let mut limited_values = [0u32; MEMBER_COUNT + 1];
    limited_values[0] = leader.limited_bonus;

    let mut idx = 0usize;
    while idx < MEMBER_COUNT {
        let group = &groups[selected[idx]];
        if idx < chosen {
            let card = deck[idx + 1];
            let eb = pool.event_bonus(card);
            power_sum += pool.power_max(card);
            skill_sum += pool.skill_max(card) as u32;
            bonus_sum += eb.base_bonus as u32;
            insert_topk_u32(&mut limited_values, eb.limited_bonus as u32);
        } else {
            power_sum += group.best_power;
            skill_sum += group.best_skill;
            bonus_sum += group.best_base_bonus;
            insert_topk_u32(&mut limited_values, group.best_limited_bonus);
        }
        idx += 1;
    }

    let mut limited_sum = 0u32;
    let limited_cap = ctx.card_bonus_count_limit.min(MEMBER_COUNT + 1);
    let mut limited_idx = 0usize;
    while limited_idx < limited_cap {
        limited_sum += limited_values[limited_idx];
        limited_idx += 1;
    }
    suffix.ceiling(
        power_sum,
        bonus_sum + limited_sum + ctx.extra_bonus_ub,
        skill_sum,
        leader.skill,
    )
}

#[inline(always)]
fn final_chapter_card_key(pool: &CardPool, card: CardIdx) -> u64 {
    let power = pool.power_max(card) as u64;
    let skill = pool.skill_max(card) as u64;
    let eb = pool.event_bonus(card);
    let bonus = eb.base_bonus as u64 + eb.limited_bonus as u64;
    power * (256 + skill) * (100 + bonus)
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

    let mut ordered_deck = *deck;
    reorder_member_deck(pool, &mut ordered_deck);
    let power_total =
        ctx.clamp_power_total(resolve_power_target(pool, &ordered_deck) + ctx.honor_bonus);
    let total_bonus = resolve_total_bonus(pool, ctx, &ordered_deck);
    let unit_counts = count_units(pool, &ordered_deck);
    let diff_count = distinct_unit_count(&unit_counts).saturating_sub(1).min(2) as u32;
    let mut skills = [ExactSkillValue::default(); DECK_SIZE];
    let mut idx = 0usize;
    while idx < DECK_SIZE {
        let card = ordered_deck[idx];
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
    let rate_sum = ctx.skill_scores[1].iter().sum::<f64>();
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
        _ => return leaf_evaluate_checked(pool, ctx, &ordered_deck),
    })
}

fn reorder_member_deck(pool: &CardPool, deck: &mut [CardIdx; DECK_SIZE]) {
    let mut indices = [1usize, 2, 3, 4];
    indices.sort_unstable_by(|left, right| {
        let left_card = deck[*left];
        let right_card = deck[*right];
        let left_bonus = pool.event_bonus(left_card).limited_bonus;
        let right_bonus = pool.event_bonus(right_card).limited_bonus;
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
    results: Vec<DeckResult>,
}

impl TopKTracker {
    fn new(top_k: usize) -> Self {
        Self {
            top_k,
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

    fn is_full(&self) -> bool {
        self.results.len() >= self.top_k
    }

    fn insert(&mut self, candidate: DeckResult) {
        if self
            .results
            .iter()
            .any(|existing| existing.cards == candidate.cards)
        {
            return;
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
}
