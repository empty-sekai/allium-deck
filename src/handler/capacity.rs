//! Checked boundaries of the compact search representation.
//! These errors occur before lossy casts, saturation, or arena construction.
use super::{BuildError, gather::CardIntermediate};
use crate::pool::{CardPool, EventBonusHot};
use crate::search::SearchContext;
use crate::types::{DECK_SIZE, LiveType, ScoreTarget};

/// Largest a priori aggregate power: five card maxima plus the honor bonus.
pub(crate) const NUMERIC_POWER_MAX: u64 = 1 << 24;
/// Largest a priori live-score rate `base + score_up * sum(rates) / 100`.
pub(crate) const NUMERIC_RATE_MAX: u64 = 1 << 16;
/// Largest a priori live-score ceiling. The fixed-point live bound dominates
/// the floating-point evaluator while `28 * 2^-53 * live < 10^-6`, that is for
/// live scores below about `3.2e8`.
pub(crate) const NUMERIC_LIVE_SCORE_MAX: u64 = 1 << 27;
/// Largest a priori total event bonus, in percent.
pub(crate) const NUMERIC_BONUS_MAX: u64 = 1 << 20;
/// Largest a priori value of `base * music_rate * (bonus + 100) / 10^4`.
pub(crate) const NUMERIC_EVENT_INNER_MAX: u64 = 1 << 27;
/// Largest a priori event point; every integer event step stays inside `i32`.
pub(crate) const NUMERIC_EVENT_POINT_MAX: u64 = 1 << 30;
/// Largest admitted floating-point excess of an event-point first stage over
/// its `1/10^4` rational grid. The event-point floor lemma needs `< 10^-4`.
const NUMERIC_EVENT_SLACK_MAX: f64 = 1e-5;

/// Rejects inputs outside the numeric domain on which every integer or
/// fixed-point search ceiling provably dominates the floating-point leaf
/// evaluator (`docs/pruning-proof.md`, "Numeric admissibility").
///
/// All checked quantities are a priori maxima over every deck of the pool, so
/// every ceiling the search evaluates for this request is covered. Request
/// parameters (teammate values, opponent score) are validated before pool
/// construction.
pub(crate) fn numeric_domain(pool: &CardPool, ctx: &SearchContext) -> Result<(), BuildError> {
    let nonnegative = |field: &str, value: f64| {
        if value.is_finite() && value >= 0.0 {
            Ok(())
        } else {
            Err(BuildError::InvalidConfig(format!(
                "{field} must be finite and non-negative"
            )))
        }
    };
    let at_most = |field: &'static str, value: f64, max: u64| {
        if value <= max as f64 {
            Ok(())
        } else {
            Err(BuildError::CapacityExceeded {
                field,
                value: value.ceil().min(u64::MAX as f64) as u64,
                max,
            })
        }
    };

    nonnegative("base score", ctx.base_score)?;
    nonnegative("auto base score", ctx.base_score_auto)?;
    nonnegative("fever score", ctx.fever_score)?;
    for &rate in ctx.skill_scores.iter().flatten() {
        nonnegative("skill score rate", rate)?;
    }
    let profiles =
        || std::iter::once(&ctx.support_deck).chain(ctx.support_decks_by_character.iter());
    for profile in profiles() {
        for &(_, bonus) in &profile.cards {
            nonnegative("support deck bonus", bonus)?;
        }
    }

    let mut powers: Vec<u64> = pool
        .indices()
        .map(|card| u64::from(pool.power_max(card)))
        .collect();
    let mut bonuses: Vec<u64> = pool
        .indices()
        .map(|card| {
            let bonus = pool.event_bonus_exact(card);
            u64::from(bonus.base_ceil() + bonus.limited_ceil())
        })
        .collect();
    powers.sort_unstable_by(|left, right| right.cmp(left));
    bonuses.sort_unstable_by(|left, right| right.cmp(left));
    let uncapped_power = powers.iter().take(DECK_SIZE).sum::<u64>() + u64::from(ctx.honor_bonus);
    at_most(
        "deck power ceiling",
        uncapped_power as f64,
        NUMERIC_POWER_MAX,
    )?;
    if matches!(ctx.target, ScoreTarget::Power | ScoreTarget::Skill) {
        return Ok(());
    }
    let power = ctx
        .power_total_cap
        .map_or(uncapped_power, |cap| uncapped_power.min(u64::from(cap))) as f64;
    let skill_peak = pool
        .indices()
        .map(|card| pool.skill_max(card))
        .max()
        .unwrap_or(0) as f64;

    let profile_sum = |profile: &crate::search::SupportDeck, extra: usize| {
        let mut values: Vec<f64> = profile.cards.iter().map(|&(_, bonus)| bonus).collect();
        values.sort_unstable_by(|left, right| right.total_cmp(left));
        values
            .iter()
            .take(profile.count as usize + extra)
            .sum::<f64>()
    };
    let leader_bonus = (0..pool.count())
        .map(|dense| ctx.leader_bonus_upper_at(dense))
        .max()
        .unwrap_or(0) as f64;
    let support_bonus = profiles()
        .map(|profile| profile_sum(profile, 0))
        .sum::<f64>();
    let bonus = bonuses.iter().take(DECK_SIZE).sum::<u64>() as f64
        + leader_bonus
        + f64::from(ctx.diff_attr_bonus.iter().copied().max().unwrap_or(0))
        + support_bonus.ceil()
        + f64::from(ctx.extra_bonus_ub);
    at_most("event bonus ceiling", bonus, NUMERIC_BONUS_MAX)?;

    if matches!(ctx.target, ScoreTarget::Mysekai) {
        return Ok(());
    }

    let live_type = ctx.effective_live_type();
    let multi = matches!(live_type, LiveType::Multi | LiveType::Cheerful);
    let (base_rate, rates) = match live_type {
        LiveType::Auto | LiveType::ChallengeAuto => (ctx.base_score_auto, ctx.skill_scores[2]),
        LiveType::Multi | LiveType::Cheerful => {
            (ctx.base_score + ctx.fever_score * 0.5, ctx.skill_scores[1])
        }
        _ => (ctx.base_score, ctx.skill_scores[0]),
    };
    let slot_peak = if multi {
        (skill_peak * 9.0 / 5.0).max(f64::from(ctx.multi_teammate_score_up.unwrap_or(0)))
    } else {
        skill_peak
    };
    let rate = base_rate + slot_peak * rates.iter().sum::<f64>() / 100.0;
    at_most("live score rate", rate, NUMERIC_RATE_MAX)?;
    let active = if !multi {
        0.0
    } else if let Some(teammate) = ctx.multi_teammate_power {
        0.075 * (power + 4.0 * f64::from(teammate))
    } else {
        0.075 * 5.0 * power
    };
    let live = 4.0 * power * rate + active;
    at_most("live score ceiling", live, NUMERIC_LIVE_SCORE_MAX)?;

    if !matches!(ctx.target, ScoreTarget::Score) || !ctx.has_event() {
        return Ok(());
    }
    let event_base = match live_type {
        LiveType::Solo | LiveType::Auto => 100.0 + live / 20_000.0,
        LiveType::Multi | LiveType::Cheerful => 123.0 + live / 17_000.0,
        _ => return Ok(()),
    };
    let music = f64::from(ctx.music_rate_pct);
    let inner = event_base * music * (bonus + 100.0) / 10_000.0;
    at_most("event point inner ceiling", inner, NUMERIC_EVENT_INNER_MAX)?;
    let life = if matches!(live_type, LiveType::Cheerful) {
        f64::from(5750 + ctx.life.clamp(500, 1000)) / 5000.0
    } else {
        1.0
    };
    let event_point = inner * life * f64::from(ctx.boost_rate_pct) / 100.0;
    at_most("event point ceiling", event_point, NUMERIC_EVENT_POINT_MAX)?;

    // A support sum maintained incrementally may exceed the evaluator's direct
    // sum by at most (2 * count + 2 * DECK_SIZE) roundings of its magnitude.
    let unit = f64::EPSILON / 2.0;
    let count = profiles()
        .map(|profile| profile.count as usize)
        .max()
        .unwrap_or(0);
    let magnitude = profiles()
        .map(|profile| profile_sum(profile, DECK_SIZE))
        .fold(0.0, f64::max);
    let bonus_excess = (2 * count + 2 * DECK_SIZE) as f64 * unit * magnitude + unit * bonus;
    let slack = 5.1 * unit * inner + 1.01 * event_base * music / 10_000.0 * bonus_excess;
    if slack > NUMERIC_EVENT_SLACK_MAX {
        return Err(BuildError::InvalidConfig(format!(
            "event point rounding slack {slack:e} exceeds {NUMERIC_EVENT_SLACK_MAX:e}"
        )));
    }
    Ok(())
}

pub(super) fn ensure(field: &'static str, value: u64, max: u64) -> Result<(), BuildError> {
    if value > max {
        Err(BuildError::CapacityExceeded { field, value, max })
    } else {
        Ok(())
    }
}

pub(super) fn score(value: i64, limit: Option<u32>, field: &'static str) -> Result<u8, BuildError> {
    let value = value.max(0) as u64;
    let value = limit.map_or(value, |limit| value.min(u64::from(limit)));
    ensure(field, value, u64::from(u8::MAX))?;
    Ok(value as u8)
}

pub(super) fn validate_cards(cards: &[CardIntermediate]) -> Result<(), BuildError> {
    // Dense indices are u16 because CardIdx is intentionally compact.  The
    // 512-bit metadata mask is only a ZMM fast-path view; overflow cards stay
    // in the full SoA columns, including the AVX-512 block traversal.
    if cards.len() > u16::MAX as usize {
        return Err(BuildError::TooManyCards(cards.len()));
    }
    let mut limited_values = Vec::new();
    for card in cards {
        if card.game_card_id < 0 {
            return Err(BuildError::InvalidConfig(
                "card identity must be nonnegative".to_string(),
            ));
        }
        ensure(
            "public card id",
            card.game_card_id as u64,
            u64::from(u16::MAX),
        )?;
        ensure("character id", u64::from(card.character_id), 26)?;
        ensure("attribute id", u64::from(card.attr), 5)?;
        ensure("unit mask", u64::from(card.unit_mask_raw), 63)?;
        ensure(
            "per-card power unit profiles",
            u64::from(card.unit_mask_raw.count_ones()),
            2,
        )?;
        for unit in 0..6 {
            for members in 0..4 {
                ensure(
                    "card power",
                    card.power.detail(unit, members).total.max(0) as u64,
                    (1 << 18) - 1,
                )?;
            }
        }
        let total =
            u64::from(card.event_bonus.base_x10()) + u64::from(card.event_bonus.limited_x10());
        ensure(
            "card event bonus (tenths)",
            total,
            u64::from(EventBonusHot::MAX_TOTAL_X10),
        )?;
        let limited = card.event_bonus.limited_x10();
        if limited != 0 && !limited_values.contains(&limited) {
            limited_values.push(limited);
        }
        if let Some(reference) = card.skill.ref_skill {
            let upper = u64::from(card.skill.skill_min) + u64::from(reference.max);
            ensure(
                "reference skill upper bound",
                upper,
                u64::from(card.skill.skill_max),
            )?;
        }
    }
    ensure(
        "distinct limited bonus values",
        limited_values.len() as u64,
        15,
    )
}

/// One-based indices retain the existing zero sentinel. Identical content never
/// consumes another slot, even when many public cards share the same skill.
pub(super) fn intern<T: Copy + PartialEq>(
    values: &mut Vec<T>,
    value: T,
    field: &'static str,
) -> Result<(u8, bool), BuildError> {
    if let Some(index) = values.iter().position(|old| *old == value) {
        return Ok(((index + 1) as u8, false));
    }
    ensure(field, (values.len() + 1) as u64, u64::from(u8::MAX))?;
    values.push(value);
    Ok((values.len() as u8, true))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pool::{EventBonusExact, PoolBuilder};
    use crate::search::SupportDeck;
    use crate::types::{EventType, LiveSkillOrder, SkillReferenceStrategy};

    fn pool() -> CardPool {
        let mut builder = PoolBuilder::new(6);
        for dense in 0..6u16 {
            builder.set_power_max(dense, 90_000 + u32::from(dense));
            builder.set_skill_min(dense, 150);
            builder.set_skill_max(dense, 150);
            builder.set_event_bonus(dense, EventBonusExact::from_x10(4_000, 95));
            builder.set_char_id(dense, dense as u8 + 1);
            builder.mark_char(dense as u8 + 1, dense);
        }
        builder.freeze()
    }

    /// Real-magnitude constants: the largest music row values, Cheerful at
    /// full life with the largest boost, and a strong teammate.
    fn context(target: ScoreTarget) -> SearchContext {
        let rates = [
            0.028222013170272814,
            0.04421448730009407,
            0.05644402634054563,
            0.03574788334901223,
            0.21435835559711452,
            0.03230479774223896,
        ];
        SearchContext {
            target,
            fixed_card_ids: Vec::new(),
            fixed_character_ids: Vec::new(),
            forced_leader_character_id: None,
            music_rate_pct: 130,
            boost_rate_pct: 3500,
            base_score: 1.2666868323070202,
            base_score_auto: 0.8155,
            fever_score: 0.25672890541976606,
            skill_scores: [rates; 3],
            other_score: 0,
            life: 1000,
            diff_attr_bonus: [0, 1, 2, 3, 4, 50],
            support_deck: SupportDeck {
                cards: vec![(1, 12.5), (2, 10.0), (3, 7.5)],
                count: 25,
            },
            support_decks_by_character: Vec::new(),
            is_world_bloom: true,
            is_final_chapter: false,
            enforce_char_uniqueness: true,
            minimize: false,
            live_type: LiveType::Multi,
            event_type: Some(EventType::CheerfulCarnival),
            keep_after_training_state: false,
            skill_reference_strategy: SkillReferenceStrategy::Average,
            best_skill_as_leader: true,
            live_skill_order: LiveSkillOrder::Best,
            specific_skill_order: None,
            multi_teammate_score_up: Some(459),
            multi_teammate_power: Some(450_000),
            multi_live_score_up_lower_bound: None,
            extra_bonus_ub: 80,
            w_power: 1.0,
            w_bonus: 1.0,
            skill_ub_global: 750,
            card_bonus_count_limit: DECK_SIZE,
            honor_bonus: 20_000,
            power_total_cap: None,
            leader_honors: Vec::new(),
            leader_honor_bonus_x10: vec![0; 6],
            leader_limit_bonus_x10: vec![0; 6],
            final_chapter_member_keep: vec![true; 6],
            skill_is_after_training: vec![false; 6],
            trained_to_special_image: vec![false; 6],
        }
    }

    type Mutation = fn(&mut SearchContext);

    #[test]
    fn real_magnitudes_are_inside_the_numeric_domain() {
        let pool = pool();
        for target in [
            ScoreTarget::Score,
            ScoreTarget::Bonus,
            ScoreTarget::Mysekai,
            ScoreTarget::Power,
            ScoreTarget::Skill,
        ] {
            for live_type in [
                LiveType::Solo,
                LiveType::Auto,
                LiveType::Multi,
                LiveType::Challenge,
                LiveType::Mysekai,
            ] {
                let mut ctx = context(target);
                ctx.live_type = live_type;
                assert_eq!(
                    numeric_domain(&pool, &ctx),
                    Ok(()),
                    "{target:?} {live_type:?}"
                );
            }
        }
    }

    #[test]
    fn signs_and_non_finite_constants_are_rejected() {
        let pool = pool();
        let cases: [Mutation; 5] = [
            |ctx| ctx.base_score = -0.5,
            |ctx| ctx.fever_score = f64::NAN,
            |ctx| ctx.skill_scores[1][4] = -1e-9,
            |ctx| ctx.base_score_auto = f64::INFINITY,
            |ctx| ctx.support_deck.cards[1].1 = -2.0,
        ];
        for mutate in cases {
            let mut ctx = context(ScoreTarget::Power);
            mutate(&mut ctx);
            assert!(matches!(
                numeric_domain(&pool, &ctx),
                Err(BuildError::InvalidConfig(_))
            ));
        }
    }

    #[test]
    fn a_priori_ceilings_outside_the_proven_range_are_rejected() {
        let pool = pool();
        let cases: [(Mutation, &str); 6] = [
            (|ctx| ctx.honor_bonus = 1 << 24, "deck power ceiling"),
            (|ctx| ctx.base_score = 70_000.0, "live score rate"),
            (|ctx| ctx.base_score = 100.0, "live score ceiling"),
            (
                |ctx| ctx.multi_teammate_score_up = Some(20_000_000),
                "live score rate",
            ),
            (
                |ctx| ctx.support_deck.cards[0].1 = 2e6,
                "event bonus ceiling",
            ),
            (
                |ctx| ctx.music_rate_pct = 2_000_000,
                "event point inner ceiling",
            ),
        ];
        for (mutate, expected) in cases {
            let mut ctx = context(ScoreTarget::Score);
            mutate(&mut ctx);
            match numeric_domain(&pool, &ctx) {
                Err(BuildError::CapacityExceeded { field, value, max }) => {
                    assert_eq!(field, expected);
                    assert!(value > max);
                }
                other => panic!("{expected}: {other:?}"),
            }
        }
        let mut ctx = context(ScoreTarget::Score);
        ctx.music_rate_pct = 400_000;
        assert!(matches!(
            numeric_domain(&pool, &ctx),
            Err(BuildError::CapacityExceeded {
                field: "event point ceiling",
                ..
            })
        ));
    }
}
