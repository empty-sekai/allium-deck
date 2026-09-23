use crate::pool::{CardIdx, CardPool, DiffSkill, RefSkill, SkillSlot, UnitCountSkill};
use crate::types::{DECK_SIZE, LiveSkillOrder, LiveType, ScoreTarget, SkillReferenceStrategy};

use super::context::SearchContext;
use super::types::DeckResultSummary;

const SKILL_SCALE: f64 = 10.0;

/// Pool unit bit of Virtual Singer (piapro) cards.
const PIAPRO_UNIT_BIT: u8 = 1 << 5;

#[derive(Clone, Copy, Debug, Default)]
struct LiveSkillValue {
    score_up: f64,
}

#[derive(Clone, Copy, Debug)]
struct EvaluatedPermutation {
    order: [usize; DECK_SIZE],
    skills: [LiveSkillValue; DECK_SIZE],
    multi_live_score_up: f64,
}

/// 解码压缩后的 u18 综合力值。
#[inline(always)]
pub fn decode_u18(values: &[u16; 8], high_bits: u32, idx: usize) -> u32 {
    debug_assert!(idx < values.len());
    values[idx] as u32 | (((high_bits >> (idx << 1)) & 3) << 16)
}

/// 精确计算叶子节点的排序值。
#[inline(always)]
pub fn leaf_evaluate(pool: &CardPool, ctx: &SearchContext, deck: &[CardIdx; 5]) -> u64 {
    leaf_evaluate_checked(pool, ctx, deck).unwrap_or(0)
}

/// 汇总单个搜索结果的展示指标。
pub fn summarize_deck(
    pool: &CardPool,
    ctx: &SearchContext,
    deck: &[CardIdx; 5],
) -> Option<DeckResultSummary> {
    let card_power_total = resolve_card_power_totals(pool, deck);
    let total_power = ctx.clamp_power_total(
        card_power_total
            .iter()
            .map(|value| (*value).max(0) as u32)
            .sum::<u32>()
            + ctx.honor_bonus,
    );
    let total_bonus = resolve_total_bonus(pool, ctx, deck);
    let permutation = evaluate_permutation(pool, ctx, deck);
    if !permutation_satisfies_lower_bound(ctx, &permutation) {
        return None;
    }
    let live_score = calc_live_score(total_power, &permutation, ctx);
    let event_point = if !ctx.is_mysekai() && ctx.has_event() {
        Some(calc_event_point(live_score, total_bonus, ctx))
    } else {
        None
    };
    Some(build_summary(
        pool,
        ctx,
        deck,
        &card_power_total,
        total_power,
        total_bonus,
        live_score,
        event_point,
        &permutation,
    ))
}

/// 精确计算叶子节点排序值；若额外约束不满足则返回 `None`。
#[inline(always)]
pub(crate) fn leaf_evaluate_checked(
    pool: &CardPool,
    ctx: &SearchContext,
    deck: &[CardIdx; 5],
) -> Option<u64> {
    if !ctx.deck_matches_forced_leader(pool, deck) {
        return None;
    }
    let power_total = || ctx.clamp_power_total(resolve_power_target(pool, deck) + ctx.honor_bonus);
    match ctx.target {
        ScoreTarget::Power => {
            if !meets_skill_lower_bound(pool, ctx, deck) {
                return None;
            }
            Some(power_total() as u64)
        }
        ScoreTarget::Mysekai => {
            let total_bonus = resolve_total_bonus(pool, ctx, deck);
            if !meets_skill_lower_bound(pool, ctx, deck) {
                return None;
            }
            Some(calc_mysekai_internal(power_total(), total_bonus) as u64)
        }
        ScoreTarget::Skill => {
            let permutation = evaluate_permutation(pool, ctx, deck);
            permutation_satisfies_lower_bound(ctx, &permutation)
                .then(|| encode_skill_target(permutation.multi_live_score_up))
        }
        ScoreTarget::Bonus => {
            let total_bonus = resolve_total_bonus(pool, ctx, deck);
            let permutation = evaluate_permutation(pool, ctx, deck);
            if !permutation_satisfies_lower_bound(ctx, &permutation) {
                return None;
            }
            let live_score = calc_live_score(power_total(), &permutation, ctx);
            Some(encode_bonus_target(total_bonus, live_score))
        }
        ScoreTarget::Score => {
            // 无活动时加成合计不参与计分，跳过逐卡加成解析。
            let total_bonus = if ctx.has_event() {
                resolve_total_bonus(pool, ctx, deck)
            } else {
                0.0
            };
            let permutation = evaluate_permutation(pool, ctx, deck);
            if !permutation_satisfies_lower_bound(ctx, &permutation) {
                return None;
            }
            let live_score = calc_live_score(power_total(), &permutation, ctx);
            let event_point = if ctx.has_event() {
                calc_event_point(live_score, total_bonus, ctx)
            } else {
                live_score
            };
            Some(((event_point as u64) << 32) | (live_score as u32 as u64))
        }
    }
}

/// Challenge live score evaluation skips event bonus aggregation because
/// challenge event points only depend on live score.
#[inline(always)]
pub(crate) fn leaf_evaluate_challenge_score_checked(
    pool: &CardPool,
    ctx: &SearchContext,
    deck: &[CardIdx; 5],
) -> Option<u64> {
    debug_assert!(matches!(
        ctx.effective_live_type(),
        LiveType::Challenge | LiveType::ChallengeAuto
    ));
    debug_assert!(matches!(ctx.target, ScoreTarget::Score));

    let power_total = ctx.clamp_power_total(resolve_power_target(pool, deck) + ctx.honor_bonus);
    let permutation = evaluate_permutation(pool, ctx, deck);
    if !permutation_satisfies_lower_bound(ctx, &permutation) {
        return None;
    }
    let live_score = calc_live_score(power_total, &permutation, ctx);
    let event_point = if ctx.has_event() {
        calc_event_point(live_score, 0.0, ctx)
    } else {
        live_score
    };
    Some(((event_point as u64) << 32) | (live_score as u32 as u64))
}

#[inline(always)]
fn encode_bonus_target(total_bonus: f64, live_score: i32) -> u64 {
    let bonus_x2 = (total_bonus * 2.0).round().clamp(0.0, u32::MAX as f64) as u32;
    ((bonus_x2 as u64) << 32) | (live_score.max(0) as u32 as u64)
}

fn build_summary(
    pool: &CardPool,
    ctx: &SearchContext,
    deck: &[CardIdx; 5],
    card_power_total: &[i32; 5],
    total_power: u32,
    total_bonus: f64,
    live_score: i32,
    event_point: Option<i32>,
    permutation: &EvaluatedPermutation,
) -> DeckResultSummary {
    let mut ordered_cards = [deck[0]; DECK_SIZE];
    let mut ordered_event_bonus = [0.0; DECK_SIZE];
    let mut ordered_skill_score_up = [0.0; DECK_SIZE];
    let mut ordered_power_total = [0; DECK_SIZE];
    let mut pos = 0usize;
    while pos < DECK_SIZE {
        let source = permutation.order[pos];
        let card = deck[source];
        ordered_cards[pos] = card;
        ordered_event_bonus[pos] = card_event_bonus_for_display(pool, ctx, card, pos == 0);
        ordered_skill_score_up[pos] = permutation.skills[source].score_up;
        ordered_power_total[pos] = card_power_total[source];
        pos += 1;
    }

    DeckResultSummary {
        ordered_cards,
        card_event_bonus_rates: ordered_event_bonus,
        card_skill_score_up: ordered_skill_score_up,
        card_power_total: ordered_power_total,
        total_power: total_power.min(i32::MAX as u32) as i32,
        live_score,
        event_point,
        multi_live_score_up: permutation.multi_live_score_up,
        event_bonus_total: (ctx.has_event() || total_bonus > 0.0).then_some(total_bonus),
        main_honor_id: ctx
            .leader_honor_for_character(pool.char_id(ordered_cards[0]))
            .map(|honor| honor.honor_id),
    }
}

fn resolve_card_power_totals(pool: &CardPool, deck: &[CardIdx; 5]) -> [i32; 5] {
    let mut attr_counts = [0u8; 6];
    let mut unit_counts = [0u8; 6];
    let mut pos = 0usize;
    while pos < DECK_SIZE {
        let card = unsafe { *deck.get_unchecked(pos) };
        let attr = pool.attr(card) as usize;
        debug_assert!(attr < attr_counts.len());
        unsafe {
            *attr_counts.get_unchecked_mut(attr) += 1;
        }
        let unit_mask = pool.unit_mask_raw(card);
        let mut unit = 0usize;
        while unit < 6 {
            if unit_mask & (1u8 << unit) != 0 {
                unsafe {
                    *unit_counts.get_unchecked_mut(unit) += 1;
                }
            }
            unit += 1;
        }
        pos += 1;
    }

    let mut totals = [0; DECK_SIZE];
    pos = 0;
    while pos < DECK_SIZE {
        let card = unsafe { *deck.get_unchecked(pos) };
        let attr = pool.attr(card) as usize;
        let attr_member = unsafe { *attr_counts.get_unchecked(attr) };
        totals[pos] =
            resolve_card_power(pool, card, &unit_counts, attr_member).min(i32::MAX as u32) as i32;
        pos += 1;
    }
    totals
}

#[inline(always)]
fn card_event_bonus_for_display(
    pool: &CardPool,
    ctx: &SearchContext,
    card: CardIdx,
    is_leader: bool,
) -> f64 {
    let mut total_x10 = u32::from(pool.event_bonus(card).total_x10());
    if ctx.is_final_chapter && is_leader {
        total_x10 += ctx.leader_honor_bonus_x10_at(card.raw());
        total_x10 += ctx.leader_limit_bonus_x10_at(card.raw());
    }
    f64::from(total_x10) / 10.0
}

#[inline(always)]
pub(crate) fn resolve_power_target(pool: &CardPool, deck: &[CardIdx; 5]) -> u32 {
    let mut attr_counts = [0u8; 6];
    let mut unit_counts = [0u8; 6];
    let mut pos = 0usize;
    while pos < DECK_SIZE {
        let card = unsafe { *deck.get_unchecked(pos) };
        let attr = pool.attr(card) as usize;
        debug_assert!(attr < attr_counts.len());
        unsafe {
            *attr_counts.get_unchecked_mut(attr) += 1;
        }
        let unit_mask = pool.unit_mask_raw(card);
        let mut unit = 0usize;
        while unit < 6 {
            if unit_mask & (1u8 << unit) != 0 {
                unsafe {
                    *unit_counts.get_unchecked_mut(unit) += 1;
                }
            }
            unit += 1;
        }
        pos += 1;
    }

    let mut total = 0u32;
    pos = 0;
    while pos < DECK_SIZE {
        let card = unsafe { *deck.get_unchecked(pos) };
        let attr = pool.attr(card) as usize;
        let attr_member = unsafe { *attr_counts.get_unchecked(attr) };
        total += resolve_card_power(pool, card, &unit_counts, attr_member);
        pos += 1;
    }
    total
}

/// Resolve total card power for a partial or complete fixed deck.
///
/// This helper is used by auxiliary calculations only. The fixed-size DFS
/// evaluator above remains unchanged.
pub fn resolve_power_for_cards(pool: &CardPool, deck: &[CardIdx]) -> u32 {
    let mut attr_counts = [0u8; 6];
    let mut unit_counts = [0u8; 6];
    for &card in deck {
        let attr = pool.attr(card) as usize;
        if attr < attr_counts.len() {
            attr_counts[attr] = attr_counts[attr].saturating_add(1);
        }
        let unit_mask = pool.unit_mask_raw(card);
        for (unit, count) in unit_counts.iter_mut().enumerate() {
            if unit_mask & (1u8 << unit) != 0 {
                *count = count.saturating_add(1);
            }
        }
    }
    deck.iter().fold(0u32, |total, &card| {
        let attr = pool.attr(card) as usize;
        total.saturating_add(resolve_card_power(
            pool,
            card,
            &unit_counts,
            attr_counts.get(attr).copied().unwrap_or(0),
        ))
    })
}

/// 由 live 分数与加成合计算出活动 PT。
///
/// 无活动上下文时原样返回 `live_score`。
#[inline(always)]
pub fn calc_event_point(live_score: i32, total_bonus: f64, ctx: &SearchContext) -> i32 {
    if !ctx.has_event() {
        return live_score;
    }

    let music_rate = ctx.music_rate_pct as f64 / 100.0;
    let deck_rate = total_bonus / 100.0 + 1.0;
    let boost_rate = ctx.boost_rate_pct as f64 / 100.0;

    match ctx.effective_live_type() {
        LiveType::Challenge | LiveType::ChallengeAuto => (100 + live_score / 20_000) * 120,
        LiveType::Solo | LiveType::Auto => {
            let base_score = 100 + live_score / 20_000;
            ((base_score as f64 * music_rate * deck_rate) as i32 as f64 * boost_rate) as i32
        }
        LiveType::Multi => {
            let other_score = if ctx.other_score == 0 {
                live_score.saturating_mul(4)
            } else {
                ctx.other_score
            };
            let base_score =
                110 + (live_score as f64 / 17_000.0) as i32 + (other_score / 340_000).min(13);
            ((base_score as f64 * music_rate * deck_rate) as i32 as f64 * boost_rate) as i32
        }
        LiveType::Cheerful => {
            let other_score = if ctx.other_score == 0 {
                live_score.saturating_mul(4)
            } else {
                ctx.other_score
            };
            let base_score =
                110 + (live_score as f64 / 17_000.0) as i32 + (other_score / 340_000).min(13);
            let life_rate = 1.15 + (ctx.life as f64 / 5000.0).clamp(0.1, 0.2);
            let inner = (base_score as f64 * music_rate * deck_rate) as i32;
            ((inner as f64 * life_rate) as i32 as f64 * boost_rate) as i32
        }
        LiveType::Mysekai => 0,
    }
}

#[inline(always)]
fn calc_live_score(
    power_total: u32,
    permutation: &EvaluatedPermutation,
    ctx: &SearchContext,
) -> i32 {
    // MySekai has no live: its objective depends on power and bonus only.
    if ctx.is_mysekai() {
        return 0;
    }
    let mut slots = sorted_live_skills(permutation, ctx);
    let skill_score_index = skill_score_index(ctx.effective_live_type());
    let mut skill_rates = unsafe { *ctx.skill_scores.get_unchecked(skill_score_index) };
    apply_live_skill_order(
        &mut slots,
        &mut skill_rates,
        ctx.live_skill_order,
        ctx.specific_skill_order.as_ref(),
    );

    let base_rate = match ctx.effective_live_type() {
        LiveType::Auto | LiveType::ChallengeAuto => ctx.base_score_auto,
        LiveType::Multi | LiveType::Cheerful => ctx.base_score + ctx.fever_score * 0.5,
        _ => ctx.base_score,
    };

    let mut rate = base_rate;
    let mut index = 0usize;
    while index < DECK_SIZE + 1 {
        rate += unsafe { slots.get_unchecked(index).score_up }
            * unsafe { *skill_rates.get_unchecked(index) }
            / 100.0;
        index += 1;
    }

    let total_power = power_total as i32;
    let power_sum = if let Some(teammate_power) = ctx.multi_teammate_power {
        total_power + teammate_power * (DECK_SIZE as i32 - 1)
    } else {
        DECK_SIZE as i32 * total_power
    };
    let active_bonus = if matches!(
        ctx.effective_live_type(),
        LiveType::Multi | LiveType::Cheerful
    ) {
        DECK_SIZE as f64 * 0.015 * power_sum as f64
    } else {
        0.0
    };

    (rate * total_power as f64 * 4.0 + active_bonus) as i32
}

#[inline(always)]
pub(crate) fn calc_mysekai_internal(power_total: u32, total_bonus: f64) -> u32 {
    let power_bonus_x10 = 10 + (power_total as u64 * 10) / 450_000;
    ((power_bonus_x10 as f64 * (100.0 + total_bonus) * 500.0) / 1000.0) as u32
}

#[inline(always)]
pub(crate) fn resolve_total_bonus(
    pool: &CardPool,
    ctx: &SearchContext,
    deck: &[CardIdx; 5],
) -> f64 {
    if !ctx.is_final_chapter && !ctx.is_world_bloom {
        // 无张数上限（默认）：热路径直出整卡加成之和。
        if ctx.card_bonus_count_limit >= DECK_SIZE {
            let total_x10 = unsafe {
                pool.event_bonus(*deck.get_unchecked(0)).total_x10() as u32
                    + pool.event_bonus(*deck.get_unchecked(1)).total_x10() as u32
                    + pool.event_bonus(*deck.get_unchecked(2)).total_x10() as u32
                    + pool.event_bonus(*deck.get_unchecked(3)).total_x10() as u32
                    + pool.event_bonus(*deck.get_unchecked(4)).total_x10() as u32
            };
            return total_x10 as f64 / 10.0;
        }
        // 有张数上限：按 base/limited 拆分计账，超额 limited 张不计入。
        let mut total_x10 = 0u32;
        let mut limited_count = 0usize;
        for pos in 0..DECK_SIZE {
            let card = unsafe { *deck.get_unchecked(pos) };
            let exact = pool.event_bonus_exact(card);
            total_x10 += exact.base_x10();
            if exact.limited_x10() > 0 && limited_count < ctx.card_bonus_count_limit {
                total_x10 += exact.limited_x10();
                limited_count += 1;
            }
        }
        return total_x10 as f64 / 10.0;
    }

    let mut attr_set = 0u8;
    let mut game_ids = [0u16; DECK_SIZE];
    let mut total_x10 = 0u64;
    let mut limited_count = 0usize;
    let mut pos = 0usize;
    while pos < DECK_SIZE {
        let card = unsafe { *deck.get_unchecked(pos) };
        attr_set |= 1u8 << pool.attr(card);
        unsafe {
            *game_ids.get_unchecked_mut(pos) = pool.game_id(card);
        }

        if ctx.is_final_chapter {
            let bonus = pool.event_bonus_exact(card);
            total_x10 += u64::from(bonus.base_x10());
            if bonus.limited_x10() > 0 && limited_count < ctx.card_bonus_count_limit {
                total_x10 += u64::from(bonus.limited_x10());
                limited_count += 1;
            }
        } else {
            total_x10 += u64::from(pool.event_bonus(card).total_x10());
        }

        if ctx.is_final_chapter && pos == 0 {
            total_x10 += u64::from(ctx.leader_honor_bonus_x10_at(card.raw()));
            total_x10 += u64::from(ctx.leader_limit_bonus_x10_at(card.raw()));
        }
        pos += 1;
    }

    let mut total = total_x10 as f64 / 10.0;

    if ctx.is_world_bloom {
        total += ctx.diff_attr_bonus[attr_set.count_ones() as usize] as f64;
        total += calc_support_bonus(pool, ctx, deck, &game_ids);
    }
    total
}

#[inline(always)]
pub(crate) fn card_proxy_bonus(
    pool: &CardPool,
    ctx: &SearchContext,
    card: CardIdx,
    is_leader: bool,
) -> u32 {
    let mut total = pool.event_bonus(card).total_ceil();
    if ctx.is_final_chapter && is_leader {
        total += ctx.leader_bonus_upper_at(card.raw());
    }
    total
}

/// Resolves each member's skill value inside one five-card deck.
///
/// Composition-dependent skills are resolved exactly for the deck:
/// - a unit-count skill reads how many members carry the skill's unit;
/// - a different-unit skill counts the distinct units of the other members
///   that differ from the card's own unit ([`different_unit_count`]);
/// - a reference skill adds `min(reference * rate / 100, max)` of another
///   member's static skill maximum ([`CardPool::skill_reference`]), selected
///   among the other members by the reference strategy and not rounded.
///
/// Admissibility of `skill_max`: every resolved value lies in
/// `[skill_min, skill_max]` of its card for every deck. Unit-count values are
/// table entries and `skill_max` is the largest entry; different-unit values
/// are clamped to `skill_max`; a reference skill adds at most `RefSkill::max`
/// to `skill_min`, and the builder guarantees `skill_min + max <= skill_max`.
///
/// A reference skill is always resolved with its reference; no base-only
/// alternative is evaluated. Referenced values are static per card, so no
/// member's value depends on another member's resolved value, the added share
/// is nonnegative, and every objective is non-decreasing in each member's
/// value. The base-only alternative can therefore never score higher. It is
/// also not a state the game produces: a deck of five always has another
/// member to reference.
#[inline(always)]
fn resolve_skills(
    pool: &CardPool,
    ctx: &SearchContext,
    deck: &[CardIdx; 5],
) -> [LiveSkillValue; DECK_SIZE] {
    let unit_counts = count_units(pool, deck);
    let mut skills = [LiveSkillValue::default(); DECK_SIZE];
    let mut index = 0usize;
    while index < DECK_SIZE {
        let card = unsafe { *deck.get_unchecked(index) };
        let slot = pool.skill(card);
        let score_up = match slot.skill_type {
            0 => slot.value as f64,
            1 => resolve_unit_count_skill(pool.special().unit_count(), slot, &unit_counts) as f64,
            2 => resolve_diff_skill(
                pool.special().diff(),
                slot,
                different_unit_count(pool, deck, index),
            )
            .min(u32::from(pool.skill_max(card))) as f64,
            3 => resolve_reference_skill(pool, ctx, deck, index, slot),
            _ => 0.0,
        };
        unsafe {
            skills.get_unchecked_mut(index).score_up = score_up;
        }
        index += 1;
    }
    skills
}

#[inline(always)]
fn evaluate_permutation(
    pool: &CardPool,
    ctx: &SearchContext,
    deck: &[CardIdx; 5],
) -> EvaluatedPermutation {
    let skills = resolve_skills(pool, ctx, deck);

    let mut order = [0usize, 1, 2, 3, 4];
    // 默认路径（最高技能作队长）保持原样；指定队长时 effective_best_skill_as_leader
    // 恒为 false，额外的槽位查找只落在 else 分支上。
    if ctx.effective_best_skill_as_leader() {
        let mut best_pos = 0usize;
        let mut pos = 1usize;
        while pos < DECK_SIZE {
            let left = unsafe { *order.get_unchecked(pos) };
            let right = unsafe { *order.get_unchecked(best_pos) };
            let left_score = unsafe { skills.get_unchecked(left).score_up };
            let right_score = unsafe { skills.get_unchecked(right).score_up };
            if left_score > right_score
                || (left_score == right_score
                    && pool.game_id(unsafe { *deck.get_unchecked(left) })
                        < pool.game_id(unsafe { *deck.get_unchecked(right) }))
            {
                best_pos = pos;
            }
            pos += 1;
        }
        order.swap(0, best_pos);
    } else {
        // 指定队长：该角色的卡固定占 order[0]（队长位），其余按卡 ID 排序。
        if let Some(leader_slot) = ctx.forced_leader_slot(pool, deck) {
            order.swap(0, leader_slot);
        }
        // A fully fixed lineup defines every skill slot, not just its leader.
        // Keep those positions when automatic leader selection is disabled.
        if ctx.fixed_card_ids.len() != DECK_SIZE {
            sort_tail_by_card_raw(pool, &mut order, deck);
        }
    }

    let score_up =
        |slot: usize| unsafe { skills.get_unchecked(*order.get_unchecked(slot)).score_up };
    let multi_live_score_up = add_ascending(
        score_up(0),
        [
            score_up(1) * 0.2,
            score_up(2) * 0.2,
            score_up(3) * 0.2,
            score_up(4) * 0.2,
        ],
    );

    EvaluatedPermutation {
        order,
        skills,
        multi_live_score_up,
    }
}

#[inline(always)]
fn sorted_live_skills(
    permutation: &EvaluatedPermutation,
    ctx: &SearchContext,
) -> [LiveSkillValue; DECK_SIZE + 1] {
    let mut buffer = [LiveSkillValue::default(); DECK_SIZE + 1];
    if matches!(
        ctx.effective_live_type(),
        LiveType::Multi | LiveType::Cheerful
    ) {
        let score_up = |slot: usize| unsafe {
            permutation
                .skills
                .get_unchecked(*permutation.order.get_unchecked(slot))
                .score_up
        };
        let member = DECK_SIZE as f64;
        let self_score_up = add_ascending(
            score_up(0),
            [
                score_up(1) / member,
                score_up(2) / member,
                score_up(3) / member,
                score_up(4) / member,
            ],
        );
        let self_skill = LiveSkillValue {
            score_up: self_score_up,
        };
        let other_skill = ctx
            .multi_teammate_score_up
            .map(|score_up| LiveSkillValue {
                score_up: score_up as f64,
            })
            .unwrap_or(self_skill);
        buffer[0] = self_skill;
        let mut slot = 1usize;
        while slot < DECK_SIZE {
            buffer[slot] = other_skill;
            slot += 1;
        }
        buffer[DECK_SIZE] = self_skill;
        return buffer;
    }

    let mut index = 0usize;
    while index < DECK_SIZE {
        unsafe {
            *buffer.get_unchecked_mut(index) = *permutation
                .skills
                .get_unchecked(*permutation.order.get_unchecked(index));
        }
        index += 1;
    }
    buffer[DECK_SIZE] = unsafe {
        *permutation
            .skills
            .get_unchecked(*permutation.order.get_unchecked(0))
    };
    buffer
}

/// Adds `terms` to `init` from the smallest to the largest, so members that
/// exchange positions produce the same floating-point value.
#[inline(always)]
fn add_ascending<const N: usize>(init: f64, mut terms: [f64; N]) -> f64 {
    let mut index = 1usize;
    while index < N {
        let mut cursor = index;
        while cursor > 0 && terms[cursor - 1] > terms[cursor] {
            terms.swap(cursor - 1, cursor);
            cursor -= 1;
        }
        index += 1;
    }
    let mut sum = init;
    for term in terms {
        sum += term;
    }
    sum
}

#[inline(always)]
fn apply_live_skill_order(
    slots: &mut [LiveSkillValue; DECK_SIZE + 1],
    skill_rates: &mut [f64; DECK_SIZE + 1],
    live_skill_order: LiveSkillOrder,
    specific_skill_order: Option<&[usize; DECK_SIZE]>,
) {
    match live_skill_order {
        LiveSkillOrder::Best => {
            sort_slots_ascending(slots);
            sort_rates_ascending(skill_rates);
        }
        LiveSkillOrder::Worst => {
            sort_slots_descending(slots);
            sort_rates_ascending(skill_rates);
        }
        LiveSkillOrder::Average => {
            let total = add_ascending(
                0.0,
                [
                    slots[0].score_up,
                    slots[1].score_up,
                    slots[2].score_up,
                    slots[3].score_up,
                    slots[4].score_up,
                ],
            );
            let average = total / DECK_SIZE as f64;
            let mut slot = 0usize;
            while slot < DECK_SIZE {
                unsafe {
                    slots.get_unchecked_mut(slot).score_up = average;
                }
                slot += 1;
            }
        }
        LiveSkillOrder::Specific => {
            let Some(order) = specific_skill_order else {
                return;
            };
            let original = *slots;
            let mut index = 0usize;
            while index < DECK_SIZE {
                unsafe {
                    *slots.get_unchecked_mut(index) =
                        *original.get_unchecked(*order.get_unchecked(index));
                }
                index += 1;
            }
            slots[DECK_SIZE] = original[DECK_SIZE];
        }
    }
}

#[inline(always)]
fn sort_slots_ascending(slots: &mut [LiveSkillValue; DECK_SIZE + 1]) {
    let mut left = 1usize;
    while left < DECK_SIZE {
        let mut cursor = left;
        while cursor > 0 {
            if unsafe { slots.get_unchecked(cursor - 1).score_up }
                <= unsafe { slots.get_unchecked(cursor).score_up }
            {
                break;
            }
            slots.swap(cursor - 1, cursor);
            cursor -= 1;
        }
        left += 1;
    }
}

#[inline(always)]
fn sort_slots_descending(slots: &mut [LiveSkillValue; DECK_SIZE + 1]) {
    let mut left = 1usize;
    while left < DECK_SIZE {
        let mut cursor = left;
        while cursor > 0 {
            if unsafe { slots.get_unchecked(cursor - 1).score_up }
                >= unsafe { slots.get_unchecked(cursor).score_up }
            {
                break;
            }
            slots.swap(cursor - 1, cursor);
            cursor -= 1;
        }
        left += 1;
    }
}

#[inline(always)]
fn sort_rates_ascending(skill_rates: &mut [f64; DECK_SIZE + 1]) {
    let mut left = 1usize;
    while left < DECK_SIZE {
        let mut cursor = left;
        while cursor > 0 {
            if unsafe { *skill_rates.get_unchecked(cursor - 1) }
                <= unsafe { *skill_rates.get_unchecked(cursor) }
            {
                break;
            }
            skill_rates.swap(cursor - 1, cursor);
            cursor -= 1;
        }
        left += 1;
    }
}

#[inline(always)]
fn skill_score_index(live_type: LiveType) -> usize {
    match live_type {
        LiveType::Multi | LiveType::Cheerful => 1,
        LiveType::Auto | LiveType::ChallengeAuto => 2,
        _ => 0,
    }
}

#[inline(always)]
fn encode_skill_target(score_up: f64) -> u64 {
    // Truncation is the floor for the non-negative value.
    (score_up * SKILL_SCALE + 1e-6) as u64
}

#[inline(always)]
fn count_units(pool: &CardPool, deck: &[CardIdx; 5]) -> [u8; 6] {
    let mut unit_counts = [0u8; 6];
    let mut pos = 0usize;
    while pos < DECK_SIZE {
        let card = unsafe { *deck.get_unchecked(pos) };
        let unit_mask = pool.unit_mask_raw(card);
        let mut unit = 0usize;
        while unit < 6 {
            if unit_mask & (1u8 << unit) != 0 {
                unsafe {
                    *unit_counts.get_unchecked_mut(unit) += 1;
                }
            }
            unit += 1;
        }
        pos += 1;
    }
    unit_counts
}

/// Unit a member contributes to different-unit counting, as a pool unit bit.
///
/// A Virtual Singer card with a support unit carries the piapro bit and its
/// support unit bit and counts as the support unit. Every other card carries a
/// single bit: its character's unit, or piapro for a Virtual Singer card
/// without a support unit.
#[inline(always)]
fn member_unit(unit_mask: u8) -> u8 {
    let non_piapro = unit_mask & !PIAPRO_UNIT_BIT;
    if non_piapro == 0 {
        unit_mask
    } else {
        non_piapro & non_piapro.wrapping_neg()
    }
}

/// Number of distinct units among the other members whose unit differs from
/// the unit of the member at `index`. A legal deck holds each card once, so
/// the position identifies the member itself.
#[inline(always)]
fn different_unit_count(pool: &CardPool, deck: &[CardIdx; 5], index: usize) -> u32 {
    let own = member_unit(pool.unit_mask_raw(unsafe { *deck.get_unchecked(index) }));
    let mut others = 0u8;
    let mut pos = 0usize;
    while pos < DECK_SIZE {
        if pos != index {
            let unit = member_unit(pool.unit_mask_raw(unsafe { *deck.get_unchecked(pos) }));
            if unit != own {
                others |= unit;
            }
        }
        pos += 1;
    }
    others.count_ones()
}

#[inline(always)]
fn resolve_card_power(
    pool: &CardPool,
    card: CardIdx,
    unit_counts: &[u8; 6],
    attr_member: u8,
) -> u32 {
    let unit_mask = pool.unit_mask_raw(card);
    let lut = pool.power_lut(card);
    let values = pool.power_values(card);
    let mut best = 0u32;
    let mut unit = 0usize;
    while unit < 6 {
        if unit_mask & (1u8 << unit) != 0 {
            let slot = ((lut >> (16 + unit)) & 1) as usize;
            let unit_member = unsafe { *unit_counts.get_unchecked(unit) };
            let key = member_key(unit_member, attr_member);
            let idx = slot * 4 + key;
            let value = decode_u18(values, lut, idx);
            if value > best {
                best = value;
            }
        }
        unit += 1;
    }
    best
}

/// Resolve one card's additive power inside a fixed all-unit/all-attribute scenario.
/// `unit_all` and `attr_all` describe deck-wide conditions, so callers can optimize
/// power exactly without enumerating every five-card combination.
pub(crate) fn resolve_card_power_scenario(
    pool: &CardPool,
    card: CardIdx,
    unit_all: Option<usize>,
    attr_all: bool,
) -> u32 {
    let mut unit_counts = [0u8; 6];
    if let Some(unit) = unit_all.filter(|unit| *unit < unit_counts.len()) {
        unit_counts[unit] = DECK_SIZE as u8;
    }
    resolve_card_power(
        pool,
        card,
        &unit_counts,
        if attr_all { DECK_SIZE as u8 } else { 0 },
    )
}

#[inline(always)]
fn member_key(unit_member: u8, attr_member: u8) -> usize {
    let unit_all = (unit_member == DECK_SIZE as u8) as usize;
    let attr_all = (attr_member == DECK_SIZE as u8) as usize;
    unit_all * 2 + attr_all
}

#[inline(always)]
fn resolve_unit_count_skill(
    table: &[UnitCountSkill],
    skill: SkillSlot,
    unit_counts: &[u8; 6],
) -> u32 {
    let index = skill.value.saturating_sub(1) as usize;
    let Some(entry) = table.get(index) else {
        return 0;
    };
    let unit = entry.unit as usize;
    if unit >= unit_counts.len() {
        return 0;
    }
    let member_count = unit_counts[unit].clamp(1, 5) as usize;
    entry.score_up[member_count - 1] as u32
}

#[inline(always)]
fn resolve_diff_skill(table: &[DiffSkill], skill: SkillSlot, unit_count: u32) -> u32 {
    let index = skill.value.saturating_sub(1) as usize;
    let Some(entry) = table.get(index) else {
        return 0;
    };
    entry.base as u32
        + entry.increment as u32 * unit_count.min(u32::from(DiffSkill::MAX_COUNTED_UNITS))
}

#[inline(always)]
fn resolve_reference_skill(
    pool: &CardPool,
    ctx: &SearchContext,
    deck: &[CardIdx; 5],
    index: usize,
    slot: SkillSlot,
) -> f64 {
    let base = pool.skill_min(unsafe { *deck.get_unchecked(index) }) as f64;
    let (rate, max) = resolve_ref_skill(pool.special().ref_skills(), slot);
    if rate == 0 || max == 0 {
        return base;
    }
    let rate = f64::from(rate);
    let max = f64::from(max);
    let mut shares = [0.0_f64; DECK_SIZE - 1];
    let mut len = 0usize;
    let mut other = 0usize;
    while other < DECK_SIZE {
        if other != index {
            let reference = f64::from(pool.skill_reference(unsafe { *deck.get_unchecked(other) }));
            unsafe {
                *shares.get_unchecked_mut(len) = (reference * rate / 100.0).min(max);
            }
            len += 1;
        }
        other += 1;
    }
    base + choose_reference_score(&shares, ctx.skill_reference_strategy)
}

#[inline(always)]
fn resolve_ref_skill(table: &[RefSkill], skill: SkillSlot) -> (u8, u8) {
    let index = skill.value.saturating_sub(1) as usize;
    let Some(entry) = table.get(index) else {
        return (0, 0);
    };
    (entry.rate, entry.max)
}

/// Reference share taken from the other four members' `shares`.
#[inline(always)]
fn choose_reference_score(shares: &[f64; DECK_SIZE - 1], strategy: SkillReferenceStrategy) -> f64 {
    match strategy {
        SkillReferenceStrategy::Max => shares.iter().copied().fold(0.0, f64::max),
        SkillReferenceStrategy::Min => shares.iter().copied().fold(f64::INFINITY, f64::min),
        // Ascending accumulation: the value is the same for every member order.
        SkillReferenceStrategy::Average => add_ascending(0.0, *shares) / shares.len() as f64,
    }
}

/// Whether the deck meets the optional lower bound on its effective skill value.
#[inline(always)]
fn meets_skill_lower_bound(pool: &CardPool, ctx: &SearchContext, deck: &[CardIdx; 5]) -> bool {
    ctx.multi_live_score_up_lower_bound.is_none()
        || permutation_satisfies_lower_bound(ctx, &evaluate_permutation(pool, ctx, deck))
}

#[inline(always)]
fn permutation_satisfies_lower_bound(
    ctx: &SearchContext,
    permutation: &EvaluatedPermutation,
) -> bool {
    ctx.multi_live_score_up_lower_bound
        .is_none_or(|lower_bound| permutation.multi_live_score_up + 1e-9 >= lower_bound)
}

#[inline(always)]
fn sort_tail_by_card_raw(pool: &CardPool, order: &mut [usize; DECK_SIZE], deck: &[CardIdx; 5]) {
    let mut left = 2usize;
    while left < DECK_SIZE {
        let mut cursor = left;
        while cursor > 1 {
            let prev = unsafe { *order.get_unchecked(cursor - 1) };
            let current = unsafe { *order.get_unchecked(cursor) };
            if pool.game_id(unsafe { *deck.get_unchecked(prev) })
                <= pool.game_id(unsafe { *deck.get_unchecked(current) })
            {
                break;
            }
            order.swap(cursor - 1, cursor);
            cursor -= 1;
        }
        left += 1;
    }
}

#[inline(always)]
fn calc_support_bonus(
    pool: &CardPool,
    ctx: &SearchContext,
    deck: &[CardIdx; 5],
    deck_game_ids: &[u16; 5],
) -> f64 {
    let mut total = 0.0_f64;
    let mut picked = 0u8;
    let support_deck = ctx.support_deck_for_leader(pool.char_id(deck[0]));
    for &(game_id, bonus) in &support_deck.cards {
        if picked >= support_deck.count {
            break;
        }
        let mut found = false;
        let mut idx = 0usize;
        while idx < DECK_SIZE {
            if unsafe { *deck_game_ids.get_unchecked(idx) } == game_id {
                found = true;
                break;
            }
            idx += 1;
        }
        if found {
            continue;
        }
        total += bonus;
        picked += 1;
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::context::SupportDeck;
    use crate::types::EventType;

    pub(super) fn ctx(live_type: LiveType) -> SearchContext {
        SearchContext {
            target: ScoreTarget::Score,
            fixed_card_ids: Vec::new(),
            fixed_character_ids: Vec::new(),
            forced_leader_character_id: None,
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
            live_type,
            event_type: None,
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
            leader_honor_bonus_x10: Vec::new(),
            leader_honors: Vec::new(),
            leader_limit_bonus_x10: Vec::new(),
            final_chapter_member_keep: Vec::new(),
        }
    }

    fn empty_permutation() -> EvaluatedPermutation {
        EvaluatedPermutation {
            order: [0, 1, 2, 3, 4],
            skills: [LiveSkillValue::default(); DECK_SIZE],
            multi_live_score_up: 0.0,
        }
    }

    #[test]
    fn cheerful_live_score_includes_coop_active_bonus() {
        let permutation = empty_permutation();

        assert_eq!(
            calc_live_score(1_000, &permutation, &ctx(LiveType::Solo)),
            4_000
        );
        assert_eq!(
            calc_live_score(1_000, &permutation, &ctx(LiveType::Multi)),
            4_375
        );
        assert_eq!(
            calc_live_score(1_000, &permutation, &ctx(LiveType::Cheerful)),
            4_375
        );

        let mut cheerful_event_ctx = ctx(LiveType::Multi);
        cheerful_event_ctx.event_type = Some(EventType::CheerfulCarnival);
        assert_eq!(
            calc_live_score(1_000, &permutation, &cheerful_event_ctx),
            4_375
        );
    }
}

#[cfg(test)]
mod dynamic_bounds;
