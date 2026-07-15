use crate::pool::{CardIdx, CardPool, DiffSkill, RefSkill, SkillSlot, UnitCountSkill};
use crate::types::{LiveSkillOrder, LiveType, ScoreTarget, SkillReferenceStrategy, DECK_SIZE};

use super::context::SearchContext;
use super::types::DeckResultSummary;

const SKILL_SCALE: f64 = 10.0;

#[derive(Clone, Copy, Debug, Default)]
struct LiveSkillValue {
    score_up: f64,
    score_up_to_reference: f64,
    ref_rate: f64,
    ref_max: f64,
    has_ref: bool,
}

#[derive(Clone, Copy, Debug, Default)]
struct PreparedSkills {
    skills: [[LiveSkillValue; 2]; DECK_SIZE],
    enumerate_mask: u32,
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
    let prepared = prepare_skills(pool, ctx, deck);

    let mut best: Option<DeckResultSummary> = None;
    let mut best_key = u64::MIN;
    let mut mask = prepared.enumerate_mask;
    loop {
        let permutation = materialize_permutation(pool, deck, ctx, &prepared, mask);
        if permutation_satisfies_lower_bound(ctx, &permutation) {
            let live_score = if ctx.is_mysekai() {
                0
            } else {
                calc_live_score(total_power, &permutation, ctx)
            };
            let event_point = if ctx.is_mysekai() {
                None
            } else if ctx.has_event() {
                Some(calc_event_point(live_score, total_bonus, ctx))
            } else {
                None
            };
            let key = summarize_key(
                ctx,
                total_power,
                total_bonus,
                live_score,
                event_point,
                &permutation,
            );
            if best.is_none() || key > best_key {
                best_key = key;
                best = Some(build_summary(
                    pool,
                    ctx,
                    deck,
                    &card_power_total,
                    total_power,
                    total_bonus,
                    live_score,
                    event_point,
                    &permutation,
                ));
            }
        }

        if mask == 0 {
            break;
        }
        mask = (mask - 1) & prepared.enumerate_mask;
    }

    best
}

/// 精确计算叶子节点排序值；若额外约束不满足则返回 `None`。
#[inline(always)]
pub(crate) fn leaf_evaluate_checked(
    pool: &CardPool,
    ctx: &SearchContext,
    deck: &[CardIdx; 5],
) -> Option<u64> {
    let power_total = ctx.clamp_power_total(resolve_power_target(pool, deck) + ctx.honor_bonus);
    match ctx.target {
        ScoreTarget::Power => {
            if !has_valid_permutation(pool, ctx, deck) {
                return None;
            }
            Some(power_total as u64)
        }
        ScoreTarget::Mysekai => {
            let total_bonus = resolve_total_bonus(pool, ctx, deck);
            if !has_valid_permutation(pool, ctx, deck) {
                return None;
            }
            Some(calc_mysekai_internal(power_total, total_bonus) as u64)
        }
        ScoreTarget::Skill => {
            let prepared = prepare_skills(pool, ctx, deck);
            let mut best = 0u64;
            let mut found = false;
            let mut mask = prepared.enumerate_mask;
            loop {
                let permutation = materialize_permutation(pool, deck, ctx, &prepared, mask);
                if !permutation_satisfies_lower_bound(ctx, &permutation) {
                    if mask == 0 {
                        break;
                    }
                    mask = (mask - 1) & prepared.enumerate_mask;
                    continue;
                }
                let encoded = encode_skill_target(permutation.multi_live_score_up);
                if encoded > best {
                    best = encoded;
                }
                found = true;
                if mask == 0 {
                    break;
                }
                mask = (mask - 1) & prepared.enumerate_mask;
            }
            found.then_some(best)
        }
        ScoreTarget::Bonus => {
            let total_bonus = resolve_total_bonus(pool, ctx, deck);
            let prepared = prepare_skills(pool, ctx, deck);
            let mut best = 0u64;
            let mut found = false;
            let mut mask = prepared.enumerate_mask;
            loop {
                let permutation = materialize_permutation(pool, deck, ctx, &prepared, mask);
                if permutation_satisfies_lower_bound(ctx, &permutation) {
                    let live_score = calc_live_score(power_total, &permutation, ctx);
                    let encoded = encode_bonus_target(total_bonus, live_score);
                    if encoded > best {
                        best = encoded;
                    }
                    found = true;
                }
                if mask == 0 {
                    break;
                }
                mask = (mask - 1) & prepared.enumerate_mask;
            }
            found.then_some(best)
        }
        ScoreTarget::Score => {
            let total_bonus = resolve_total_bonus(pool, ctx, deck);
            let prepared = prepare_skills(pool, ctx, deck);
            let mut best = 0u64;
            let mut found = false;
            let mut mask = prepared.enumerate_mask;
            loop {
                let permutation = materialize_permutation(pool, deck, ctx, &prepared, mask);
                if !permutation_satisfies_lower_bound(ctx, &permutation) {
                    if mask == 0 {
                        break;
                    }
                    mask = (mask - 1) & prepared.enumerate_mask;
                    continue;
                }
                let live_score = calc_live_score(power_total, &permutation, ctx);
                let event_point = if ctx.has_event() {
                    calc_event_point(live_score, total_bonus, ctx)
                } else {
                    live_score
                };
                let encoded = ((event_point as u64) << 32) | (live_score as u32 as u64);
                if encoded > best {
                    best = encoded;
                }
                found = true;
                if mask == 0 {
                    break;
                }
                mask = (mask - 1) & prepared.enumerate_mask;
            }
            found.then_some(best)
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
    let prepared = prepare_skills(pool, ctx, deck);
    let mut best = 0u64;
    let mut found = false;
    let mut mask = prepared.enumerate_mask;
    loop {
        let permutation = materialize_permutation(pool, deck, ctx, &prepared, mask);
        if !permutation_satisfies_lower_bound(ctx, &permutation) {
            if mask == 0 {
                break;
            }
            mask = (mask - 1) & prepared.enumerate_mask;
            continue;
        }
        let live_score = calc_live_score(power_total, &permutation, ctx);
        let event_point = if ctx.has_event() {
            calc_event_point(live_score, 0.0, ctx)
        } else {
            live_score
        };
        let encoded = ((event_point as u64) << 32) | (live_score as u32 as u64);
        if encoded > best {
            best = encoded;
        }
        found = true;
        if mask == 0 {
            break;
        }
        mask = (mask - 1) & prepared.enumerate_mask;
    }
    found.then_some(best)
}

#[inline(always)]
fn summarize_key(
    ctx: &SearchContext,
    total_power: u32,
    total_bonus: f64,
    live_score: i32,
    event_point: Option<i32>,
    permutation: &EvaluatedPermutation,
) -> u64 {
    match ctx.target {
        ScoreTarget::Power => ((total_power as u64) << 32) | (live_score.max(0) as u32 as u64),
        ScoreTarget::Skill => encode_skill_target(permutation.multi_live_score_up),
        ScoreTarget::Mysekai => total_power as u64,
        ScoreTarget::Bonus => encode_bonus_target(total_bonus, live_score),
        ScoreTarget::Score => {
            ((event_point.unwrap_or(live_score).max(0) as u64) << 32)
                | (live_score.max(0) as u32 as u64)
        }
    }
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
    let mut total = pool.event_bonus(card).total_rate();
    if ctx.is_final_chapter && is_leader {
        total += ctx.leader_honor_bonus_at(card.raw()) as f64;
        total += ctx.leader_limit_bonus_at(card.raw()) as f64;
    }
    total
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
        let total_x10 = unsafe {
            pool.event_bonus(*deck.get_unchecked(0)).total_x10() as u32
                + pool.event_bonus(*deck.get_unchecked(1)).total_x10() as u32
                + pool.event_bonus(*deck.get_unchecked(2)).total_x10() as u32
                + pool.event_bonus(*deck.get_unchecked(3)).total_x10() as u32
                + pool.event_bonus(*deck.get_unchecked(4)).total_x10() as u32
        };
        return total_x10 as f64 * 0.1;
    }

    let mut attr_set = 0u8;
    let mut game_ids = [0u16; DECK_SIZE];
    let mut total = 0.0_f64;
    let mut total_x10 = 0u32;
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
            total += bonus.base_rate();
            if bonus.limited_x10() == 0 {
                total += bonus.limited_rate();
            } else if limited_count < ctx.card_bonus_count_limit {
                total += bonus.limited_rate();
                limited_count += 1;
            }
        } else {
            total_x10 += pool.event_bonus(card).total_x10() as u32;
        }

        if ctx.is_final_chapter && pos == 0 {
            total += ctx.leader_honor_bonus_at(card.raw()) as f64;
            total += ctx.leader_limit_bonus_at(card.raw()) as f64;
        }
        pos += 1;
    }

    if !ctx.is_final_chapter {
        total = total_x10 as f64 * 0.1;
    }

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
        total += ctx.leader_honor_bonus_at(card.raw());
        total += ctx.leader_limit_bonus_at(card.raw());
    }
    total
}

#[inline(always)]
fn prepare_skills(pool: &CardPool, ctx: &SearchContext, deck: &[CardIdx; 5]) -> PreparedSkills {
    let unit_counts = count_units(pool, deck);
    let unit_kind_count = distinct_unit_count(&unit_counts);
    let diff_count = unit_kind_count.saturating_sub(1).min(2) as u32;
    let mut prepared = PreparedSkills::default();
    let mut enumerate_mask = 0u32;
    let mut card_index = 0usize;
    while card_index < DECK_SIZE {
        let card = unsafe { *deck.get_unchecked(card_index) };
        let slot = pool.skill(card);
        let skill_dense = card.raw();
        let mut secondary = LiveSkillValue::default();
        let mut primary = LiveSkillValue::default();
        let mut need_enumerate = false;

        match slot.skill_type {
            0 => {
                primary.score_up = slot.value as f64;
            }
            1 => {
                primary.score_up =
                    resolve_unit_count_skill(pool.special().unit_count(), slot, &unit_counts)
                        as f64;
            }
            2 => {
                primary.score_up = pool.skill_min(card) as f64;
                secondary.score_up = resolve_diff_skill(pool.special().diff(), slot, diff_count)
                    .min(pool.skill_max(card) as u32) as f64;
            }
            3 => {
                let base = pool.skill_min(card) as f64;
                let (ref_rate, ref_max) = resolve_ref_skill(pool.special().ref_skills(), slot);
                primary.score_up = base;
                if ref_rate != 0 && ref_max != 0 {
                    secondary = LiveSkillValue {
                        score_up: base + ref_max as f64,
                        ref_rate: ref_rate as f64,
                        ref_max: ref_max as f64,
                        has_ref: true,
                        ..LiveSkillValue::default()
                    };
                    need_enumerate = true;
                }
            }
            _ => {}
        }

        if ctx.keep_after_training_state {
            if !ctx.trained_to_special_image_at(skill_dense)
                && ctx.skill_is_after_training_at(skill_dense)
            {
                primary = secondary;
            }
        } else if need_enumerate && secondary.score_up > 0.0 {
            enumerate_mask |= 1u32 << card_index;
        } else if secondary.score_up > primary.score_up {
            primary = secondary;
        }

        unsafe {
            *prepared.skills.get_unchecked_mut(card_index) = [secondary, primary];
        }
        card_index += 1;
    }

    if ctx.is_mysekai() {
        prepared.enumerate_mask = 0;
    } else {
        prepared.enumerate_mask = enumerate_mask;
    }
    prepared
}

#[inline(always)]
fn materialize_permutation(
    pool: &CardPool,
    deck: &[CardIdx; 5],
    ctx: &SearchContext,
    prepared: &PreparedSkills,
    mask: u32,
) -> EvaluatedPermutation {
    let mut skills = [LiveSkillValue::default(); DECK_SIZE];
    let mut index = 0usize;
    while index < DECK_SIZE {
        let mut skill = if mask & (1u32 << index) != 0 {
            unsafe { *prepared.skills.get_unchecked(index).get_unchecked(0) }
        } else {
            unsafe { *prepared.skills.get_unchecked(index).get_unchecked(1) }
        };
        skill.score_up_to_reference = skill.score_up;
        unsafe {
            *skills.get_unchecked_mut(index) = skill;
        }
        index += 1;
    }

    let mut ref_index = 0usize;
    while ref_index < DECK_SIZE {
        if unsafe { skills.get_unchecked(ref_index).has_ref } {
            let mut reference_scores = [0.0_f64; DECK_SIZE - 1];
            let mut reference_len = 0usize;
            unsafe {
                skills.get_unchecked_mut(ref_index).score_up -=
                    skills.get_unchecked(ref_index).ref_max;
            }
            let mut other = 0usize;
            while other < DECK_SIZE {
                if other != ref_index {
                    unsafe {
                        *reference_scores.get_unchecked_mut(reference_len) =
                            (skills.get_unchecked(other).score_up_to_reference
                                * skills.get_unchecked(ref_index).ref_rate
                                / 100.0)
                                .floor()
                                .min(skills.get_unchecked(ref_index).ref_max);
                    }
                    reference_len += 1;
                }
                other += 1;
            }
            let chosen = choose_reference_score(
                &reference_scores,
                reference_len,
                ctx.skill_reference_strategy,
            );
            unsafe {
                skills.get_unchecked_mut(ref_index).score_up += chosen;
            }
        }
        ref_index += 1;
    }

    let mut order = [0usize, 1, 2, 3, 4];
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
        sort_tail_by_card_raw(pool, &mut order, deck);
    }

    let mut multi_live_score_up = unsafe { skills.get_unchecked(*order.get_unchecked(0)).score_up };
    let mut score_index = 1usize;
    while score_index < DECK_SIZE {
        multi_live_score_up += unsafe {
            skills
                .get_unchecked(*order.get_unchecked(score_index))
                .score_up
        } * 0.2;
        score_index += 1;
    }

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
        let mut self_score_up = unsafe {
            permutation
                .skills
                .get_unchecked(*permutation.order.get_unchecked(0))
                .score_up
        };
        let mut index = 1usize;
        while index < DECK_SIZE {
            self_score_up += unsafe {
                permutation
                    .skills
                    .get_unchecked(*permutation.order.get_unchecked(index))
                    .score_up
            } / DECK_SIZE as f64;
            index += 1;
        }
        let self_skill = LiveSkillValue {
            score_up: self_score_up,
            ..LiveSkillValue::default()
        };
        let other_skill = ctx
            .multi_teammate_score_up
            .map(|score_up| LiveSkillValue {
                score_up: score_up as f64,
                ..LiveSkillValue::default()
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
    if matches!(ctx.live_skill_order, LiveSkillOrder::Specific)
        && ctx.specific_skill_order.is_none()
    {
        return buffer;
    }
    buffer
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
            let mut total = 0.0;
            let mut index = 0usize;
            while index < DECK_SIZE {
                total += unsafe { slots.get_unchecked(index).score_up };
                index += 1;
            }
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
    (score_up * SKILL_SCALE + 1e-6).floor() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::context::SupportDeck;
    use crate::types::EventType;

    fn ctx(live_type: LiveType) -> SearchContext {
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

#[inline(always)]
fn distinct_unit_count(unit_counts: &[u8; 6]) -> u8 {
    let mut count = 0u8;
    let mut index = 0usize;
    while index < 6 {
        if unsafe { *unit_counts.get_unchecked(index) } > 0 {
            count += 1;
        }
        index += 1;
    }
    count
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
fn resolve_diff_skill(table: &[DiffSkill], skill: SkillSlot, diff_count: u32) -> u32 {
    let index = skill.value.saturating_sub(1) as usize;
    let Some(entry) = table.get(index) else {
        return 0;
    };
    entry.base as u32 + entry.increment as u32 * diff_count
}

#[inline(always)]
fn resolve_ref_skill(table: &[RefSkill], skill: SkillSlot) -> (u8, u8) {
    let index = skill.value.saturating_sub(1) as usize;
    let Some(entry) = table.get(index) else {
        return (0, 0);
    };
    (entry.rate, entry.max)
}

#[inline(always)]
fn choose_reference_score(
    reference_scores: &[f64; DECK_SIZE - 1],
    reference_len: usize,
    strategy: SkillReferenceStrategy,
) -> f64 {
    match strategy {
        SkillReferenceStrategy::Max => {
            let mut best = 0.0;
            let mut index = 0usize;
            while index < reference_len {
                if unsafe { *reference_scores.get_unchecked(index) } > best {
                    best = unsafe { *reference_scores.get_unchecked(index) };
                }
                index += 1;
            }
            best
        }
        SkillReferenceStrategy::Min => {
            let mut best = unsafe { *reference_scores.get_unchecked(0) };
            let mut index = 1usize;
            while index < reference_len {
                if unsafe { *reference_scores.get_unchecked(index) } < best {
                    best = unsafe { *reference_scores.get_unchecked(index) };
                }
                index += 1;
            }
            best
        }
        SkillReferenceStrategy::Average => {
            let mut total = 0.0;
            let mut index = 0usize;
            while index < reference_len {
                total += unsafe { *reference_scores.get_unchecked(index) };
                index += 1;
            }
            total / reference_len as f64
        }
    }
}

#[inline(always)]
fn has_valid_permutation(pool: &CardPool, ctx: &SearchContext, deck: &[CardIdx; 5]) -> bool {
    if ctx.multi_live_score_up_lower_bound.is_none() {
        return true;
    }
    let prepared = prepare_skills(pool, ctx, deck);
    let mut mask = prepared.enumerate_mask;
    loop {
        let permutation = materialize_permutation(pool, deck, ctx, &prepared, mask);
        if permutation_satisfies_lower_bound(ctx, &permutation) {
            return true;
        }
        if mask == 0 {
            break;
        }
        mask = (mask - 1) & prepared.enumerate_mask;
    }
    false
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
