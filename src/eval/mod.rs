mod event_bonus;
mod live_score;
mod mysekai;
mod power;
mod skill;

use crate::types::{
    CardId, CardSpec, DeckContext, DeckScore, ScoreTarget, Unit, DECK_SIZE, SCORE_MAX, UNIT_COUNT,
};

pub(crate) use event_bonus::{resolve_event_bonus, resolve_support_deck_bonus};
pub(crate) use live_score::{calc_event_point, calc_live_score};
pub(crate) use mysekai::calc_mysekai_points;
pub(crate) use power::{fold_deck_power, resolve_card_power, DeckPower};
pub(crate) use skill::{materialize_permutation, prepare_skills};

/// Infallible evaluate entry point. All validation is handled by DeckContext::new();
/// this function is a pure hot-path with zero heap allocation.
pub fn evaluate(cards: &[CardId; DECK_SIZE], ctx: &DeckContext<'_>) -> DeckScore {
    let selected = lookup_cards(cards, ctx);
    let attr_counts = count_attrs(&selected);
    let unit_counts = count_units(&selected);
    let unit_kind_count = distinct_unit_count(&unit_counts);

    let card_power = resolve_card_power(&selected, &attr_counts, &unit_counts);
    let mut power = fold_deck_power(&card_power, ctx.honor_bonus);
    if let Some(world_bloom) = ctx
        .event
        .as_ref()
        .and_then(|event| event.world_bloom.as_ref())
    {
        if let Some(cap) = world_bloom.power_total_cap {
            // moe deck-information/deck-calculator.cpp:175-178 applies the WL power cap after folding deck power.
            power.total = power.total.min(cap);
        }
    }

    let (card_event_bonus, diff_attr_bonus_rate, event_bonus_rate) =
        resolve_event_bonus(&selected, ctx.event.as_ref());
    let support_deck_bonus_rate = resolve_support_deck_bonus(cards, &selected, ctx.event.as_ref());
    let total_bonus = event_bonus_rate + support_deck_bonus_rate;
    let (mysekai_event_point, mysekai_internal_point) = if ctx.is_mysekai {
        calc_mysekai_points(power.total, total_bonus)
    } else {
        (0, 0)
    };

    let prepared = prepare_skills(
        &selected,
        &unit_counts,
        unit_kind_count,
        ctx.keep_after_training_state,
        ctx.event
            .as_ref()
            .and_then(|event| event.skill_score_up_limit),
        ctx.is_mysekai,
    );

    let mut best_score = empty_deck_score();
    let mut best_value = f64::NEG_INFINITY;
    let mut best_leader = CardId::MAX;
    let mut mask = prepared.enumerate_mask;
    loop {
        let permutation = materialize_permutation(
            cards,
            &selected,
            &prepared,
            mask,
            ctx.effective_best_skill_as_leader,
            ctx.skill_reference_strategy,
        );
        let live_score = if ctx.is_mysekai {
            0
        } else {
            calc_live_score(
                &permutation.skills,
                &permutation.order,
                power.total,
                &ctx.music,
                ctx.effective_live_type,
                ctx.live_skill_order,
                ctx.specific_skill_order.as_ref(),
                ctx.multi_teammate_score_up,
                ctx.multi_teammate_power,
            )
        };
        let event_point = if ctx.is_mysekai {
            0
        } else if let Some(event) = ctx.event.as_ref() {
            calc_event_point(
                ctx.effective_live_type,
                event.event_type,
                live_score,
                ctx.music.event_rate,
                total_bonus,
                event.boost_rate,
                event.other_score,
                event.life,
            )
        } else {
            live_score
        };

        let target_value = match ctx.target {
            ScoreTarget::Mysekai => mysekai_internal_point as f64,
            ScoreTarget::Power => power.total as f64 + event_point as f64 / SCORE_MAX,
            ScoreTarget::Skill => permutation.multi_live_score_up + event_point as f64 / SCORE_MAX,
            ScoreTarget::Score => event_point as f64 + live_score as f64 / SCORE_MAX,
        };
        let permutation_value = match ctx.target {
            ScoreTarget::Power => event_point as f64 + live_score as f64 / SCORE_MAX,
            _ => target_value,
        };
        let leader = cards[permutation.order[0]];
        if permutation_value > best_value
            || (permutation_value == best_value && leader < best_leader)
        {
            best_value = permutation_value;
            best_leader = leader;
            best_score = build_deck_score(
                cards,
                &card_power,
                &card_event_bonus,
                power,
                ctx.honor_bonus,
                diff_attr_bonus_rate,
                support_deck_bonus_rate,
                event_bonus_rate,
                live_score,
                event_point,
                mysekai_event_point,
                mysekai_internal_point,
                target_value,
                &permutation,
            );
        }

        if mask == 0 {
            break;
        }
        mask = (mask - 1) & prepared.enumerate_mask;
    }

    best_score
}

pub fn card_value(_card: CardId, _ctx: &DeckContext<'_>) -> f64 {
    todo!()
}

pub fn upper_bound(_selected: &[CardId], _ctx: &DeckContext<'_>) -> f64 {
    todo!()
}

fn lookup_cards<'a>(
    cards: &[CardId; DECK_SIZE],
    ctx: &'a DeckContext<'_>,
) -> [&'a CardSpec; DECK_SIZE] {
    [
        &ctx.pool.cards[cards[0] as usize],
        &ctx.pool.cards[cards[1] as usize],
        &ctx.pool.cards[cards[2] as usize],
        &ctx.pool.cards[cards[3] as usize],
        &ctx.pool.cards[cards[4] as usize],
    ]
}

fn count_attrs(cards: &[&CardSpec; DECK_SIZE]) -> [i32; crate::types::ATTR_COUNT] {
    let mut counts = [0_i32; crate::types::ATTR_COUNT];
    for card in cards {
        counts[card.attr as usize] += 1;
    }
    counts
}

fn count_units(cards: &[&CardSpec; DECK_SIZE]) -> [i32; UNIT_COUNT] {
    let mut counts = [0_i32; UNIT_COUNT];
    for card in cards {
        let mut index = 0;
        while index < card.unit_count as usize {
            counts[card.units[index] as usize] += 1;
            index += 1;
        }
    }
    counts
}

fn distinct_unit_count(unit_counts: &[i32; UNIT_COUNT]) -> i32 {
    let mut count = 0;
    let mut index = 0;
    while index < UNIT_COUNT {
        let unit = match index {
            value if value == Unit::None as usize => false,
            value if value == Unit::Any as usize => false,
            value if value == Unit::Ref as usize => false,
            value if value == Unit::Diff as usize => false,
            _ => unit_counts[index] > 0,
        };
        if unit {
            count += 1;
        }
        index += 1;
    }
    count
}

fn build_deck_score(
    card_ids: &[CardId; DECK_SIZE],
    card_power: &[crate::types::PowerDetail; DECK_SIZE],
    card_event_bonus: &[f64; DECK_SIZE],
    power: DeckPower,
    honor_bonus: i32,
    diff_attr_bonus_rate: f64,
    support_deck_bonus_rate: f64,
    event_bonus_rate: f64,
    live_score: i32,
    event_point: i32,
    mysekai_event_point: i32,
    mysekai_internal_point: i32,
    target_value: f64,
    permutation: &skill::EvaluatedPermutation,
) -> DeckScore {
    let mut ordered_ids = [0; DECK_SIZE];
    let mut ordered_event_bonus = [0.0; DECK_SIZE];
    let mut ordered_skill_score_up = [0.0; DECK_SIZE];
    let mut ordered_skill_life_recovery = [0.0; DECK_SIZE];
    let mut ordered_power_total = [0; DECK_SIZE];

    let mut index = 0;
    while index < DECK_SIZE {
        let source = permutation.order[index];
        ordered_ids[index] = card_ids[source];
        ordered_event_bonus[index] = card_event_bonus[source];
        ordered_skill_score_up[index] = permutation.skills[source].score_up;
        ordered_skill_life_recovery[index] = permutation.skills[source].life_recovery;
        ordered_power_total[index] = card_power[source].total;
        index += 1;
    }

    DeckScore {
        card_ids: ordered_ids,
        card_event_bonus_rates: ordered_event_bonus,
        card_skill_score_up: ordered_skill_score_up,
        card_skill_life_recovery: ordered_skill_life_recovery,
        card_power_total: ordered_power_total,
        total_power: power.total,
        base_power: power.base,
        area_item_bonus_power: power.area_item_bonus,
        character_bonus_power: power.character_bonus,
        honor_bonus_power: honor_bonus,
        fixture_bonus_power: power.fixture_bonus,
        gate_bonus_power: power.gate_bonus,
        event_bonus_rate,
        support_deck_bonus_rate,
        diff_attr_bonus_rate,
        multi_live_score_up: permutation.multi_live_score_up,
        live_score,
        event_point,
        mysekai_event_point,
        mysekai_internal_point,
        target_value,
        chosen_mask: permutation.chosen_mask,
    }
}

fn empty_deck_score() -> DeckScore {
    DeckScore {
        card_ids: [0; DECK_SIZE],
        card_event_bonus_rates: [0.0; DECK_SIZE],
        card_skill_score_up: [0.0; DECK_SIZE],
        card_skill_life_recovery: [0.0; DECK_SIZE],
        card_power_total: [0; DECK_SIZE],
        total_power: 0,
        base_power: 0,
        area_item_bonus_power: 0,
        character_bonus_power: 0,
        honor_bonus_power: 0,
        fixture_bonus_power: 0,
        gate_bonus_power: 0,
        event_bonus_rate: 0.0,
        support_deck_bonus_rate: 0.0,
        diff_attr_bonus_rate: 0.0,
        multi_live_score_up: 0.0,
        live_score: 0,
        event_point: 0,
        mysekai_event_point: 0,
        mysekai_internal_point: 0,
        target_value: 0.0,
        chosen_mask: 0,
    }
}
