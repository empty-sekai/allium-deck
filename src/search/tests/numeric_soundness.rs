//! Numeric admissibility of the integer and fixed-point search ceilings.
//!
//! The leaf evaluator works in `f64` and truncates; every search ceiling is an
//! integer or fixed-point expression. These deterministic property tests use
//! decimal constants that are not binary-exact and enumerate every complete
//! deck of small generated pools. For each deck the aggregate ceiling is built
//! from that deck's own exact features (the power the evaluator resolves, the
//! smallest integer not below the evaluator's bonus, the skill sum and the skill
//! peak), which is the tightest input any admissible search relaxation can
//! pass. The ceiling must dominate the leaf value as a packed key and in every
//! component.
use super::*;
use crate::pool::{CardIdx, EventBonusExact};
use crate::search::correlated::CorrelatedBound;
use crate::search::evaluate::{leaf_evaluate_checked, resolve_power_target, resolve_total_bonus};
use crate::search::objective::{LIVE_SCORE_BOUND_SCALE, ObjectiveBound};

const LOW: u64 = u32::MAX as u64;

/// SplitMix64: deterministic, dependency-free sampling.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    fn below(&mut self, bound: usize) -> usize {
        (self.next() % bound as u64) as usize
    }

    fn pick<T: Copy>(&mut self, values: &[T]) -> T {
        values[self.below(values.len())]
    }

    fn chance(&mut self, numerator: u64, denominator: u64) -> bool {
        self.next() % denominator < numerator
    }
}

/// Decimal music constants: real rows, short decimals and non-dyadic values.
const BASE_SCORES: [f64; 10] = [
    1.0,
    1.1,
    0.95,
    1.2345678,
    1.005249294449671,
    1.2666868323070202,
    1.1099,
    0.7,
    0.8155,
    1.0 / 3.0,
];
const FEVER_SCORES: [f64; 5] = [0.0, 0.1, 0.22803386641580425, 0.25672890541976606, 0.3];
const SKILL_RATES: [[f64; 6]; 7] = [
    [
        0.028222013170272814,
        0.04421448730009407,
        0.05644402634054563,
        0.03574788334901223,
        0.057008466603951084,
        0.03230479774223896,
    ],
    [
        0.028222013170272814,
        0.04421448730009407,
        0.05644402634054563,
        0.03574788334901223,
        0.08551269990592664,
        0.03230479774223896,
    ],
    [0.1, 0.2, 0.3, 0.1, 0.2, 0.1],
    [0.1; 6],
    [0.05; 6],
    [
        0.14503225105087683,
        0.21435835559711452,
        0.0003486,
        0.07,
        0.11,
        0.013,
    ],
    [1.0 / 3.0, 0.0, 1.0 / 7.0, 0.0, 0.01, 1.0 / 9.0],
];
const MUSIC_RATES: [u32; 6] = [100, 107, 110, 114, 120, 130];
const BOOST_RATES: [u32; 5] = [100, 500, 1500, 2700, 3500];
const LIVES: [i32; 8] = [0, 499, 500, 777, 999, 1000, 1001, 1500];
const OTHER_SCORES: [i32; 6] = [0, 0, 339_999, 340_000, 4_420_000, 5_000_000];
const SUPPORT_LISTS: [&[f64]; 7] = [
    &[2.7, 0.2, 0.1],
    &[2.2, 2.2, 0.6, 0.2, 0.2],
    &[0.3, 0.3, 0.2, 0.1, 0.1],
    &[3.3, 2.2, 1.1, 0.4],
    &[0.7, 0.7, 0.7, 0.7, 0.2],
    &[1.25, 0.5, 0.25],
    &[4.1, 2.9, 0.6, 0.3, 0.1],
];

#[derive(Clone, Copy)]
struct NumCard {
    char_id: u8,
    attr: u8,
    unit_mask: u8,
    game_id: u16,
    /// Power by `unit_all * 2 + attr_all`.
    powers: [u32; 4],
    skill: SkillSlot,
    skill_min: u8,
    skill_max: u8,
    base_x10: u16,
    limited_x10: u16,
}

fn numeric_pool(cards: &[NumCard]) -> CardPool {
    let mut builder = PoolBuilder::new(cards.len() as u16);
    builder.add_unit_count_skill(UnitCountSkill {
        unit: 0,
        score_up: [10, 20, 30, 40, 50],
    });
    builder.add_diff_skill(DiffSkill {
        base: 12,
        increment: 6,
    });
    builder.add_ref_skill(RefSkill { rate: 50, max: 30 });
    builder.add_ref_skill(RefSkill { rate: 33, max: 41 });
    for (index, card) in cards.iter().enumerate() {
        let dense = index as u16;
        let mut values = [0u16; 8];
        let mut high_bits = 0u32;
        for (slot, value) in values.iter_mut().enumerate() {
            let power = card.powers[slot % 4];
            *value = power as u16;
            high_bits |= ((power >> 16) & 3) << (slot << 1);
        }
        builder.set_power_values(dense, values);
        builder.set_power_lut(dense, high_bits);
        builder.set_power_max(dense, card.powers.into_iter().max().unwrap_or(0));
        builder.set_skill(dense, card.skill);
        builder.set_skill_min(dense, card.skill_min);
        builder.set_skill_max(dense, card.skill_max);
        builder.set_event_bonus(
            dense,
            EventBonusExact::from_x10(card.base_x10, card.limited_x10),
        );
        builder.set_char_id(dense, card.char_id);
        builder.set_attr(dense, card.attr);
        builder.set_unit_mask(dense, card.unit_mask);
        builder.set_game_id(dense, card.game_id);
        builder.mark_char(card.char_id, dense);
        for unit in 0..6u8 {
            if card.unit_mask & (1 << unit) != 0 {
                builder.mark_unit(unit, dense);
            }
        }
        builder.mark_attr(card.attr, dense);
    }
    builder.freeze()
}

/// Skill shapes covering fixed, unit-count, different-unit and reference
/// skills. Reference skills keep `skill_min + reference max <= skill_max`,
/// which pool construction enforces.
fn random_skill(rng: &mut Rng) -> (SkillSlot, u8, u8) {
    match rng.below(8) {
        0 => (
            SkillSlot {
                skill_type: 1,
                value: 1,
            },
            10,
            50,
        ),
        1 => (
            SkillSlot {
                skill_type: 2,
                value: 1,
            },
            12,
            24,
        ),
        2 => {
            let base = rng.pick(&[40u8, 57, 80, 99]);
            (
                SkillSlot {
                    skill_type: 3,
                    value: 1,
                },
                base,
                base + 30,
            )
        }
        3 => {
            let base = rng.pick(&[33u8, 61, 100]);
            (
                SkillSlot {
                    skill_type: 3,
                    value: 2,
                },
                base,
                base + 41,
            )
        }
        _ => {
            let value = rng.pick(&[7u8, 13, 50, 60, 99, 100, 101, 120, 150, 255]);
            (
                SkillSlot {
                    skill_type: 0,
                    value,
                },
                value,
                value,
            )
        }
    }
}

fn random_pool(rng: &mut Rng, size: usize) -> CardPool {
    let cards: Vec<_> = (0..size)
        .map(|index| {
            let base_power = 60_000 + rng.below(200_000) as u32;
            let attr_step = rng.pick(&[0u32, 1, 7, 997]);
            let unit_step = rng.pick(&[0u32, 3, 11, 1231]);
            let (skill, skill_min, skill_max) = random_skill(rng);
            let base_x10 = rng.pick(&[0u16, 1, 3, 25, 27, 100, 125, 199, 250, 405]);
            let limited_x10 = rng.pick(&[0u16, 0, 0, 5, 100]);
            NumCard {
                char_id: index as u8 + 1,
                attr: rng.below(2) as u8,
                unit_mask: rng.pick(&[1u8, 1, 2, 3]),
                game_id: 300 + index as u16,
                powers: [
                    base_power,
                    base_power + attr_step,
                    base_power + unit_step,
                    base_power + attr_step + unit_step,
                ],
                skill,
                skill_min,
                skill_max,
                base_x10,
                limited_x10,
            }
        })
        .collect();
    numeric_pool(&cards)
}

fn random_support(rng: &mut Rng, pool: &CardPool) -> SupportDeck {
    let list = rng.pick(&SUPPORT_LISTS);
    // Owned support cards overlap the main pool, so occupancy exclusions and
    // substitutions both occur.
    let mut cards: Vec<(u16, f64)> = list
        .iter()
        .enumerate()
        .map(|(index, &bonus)| {
            let game_id = if index < pool.count() && rng.chance(1, 2) {
                pool.game_id(CardIdx::new(index as u16))
            } else {
                900 + index as u16
            };
            (game_id, bonus)
        })
        .collect();
    cards.sort_by(|left, right| right.1.total_cmp(&left.1));
    cards.dedup_by_key(|entry| entry.0);
    SupportDeck {
        count: rng.pick(&[1u8, 2, 3]),
        cards,
    }
}

#[derive(Clone, Copy, Debug)]
enum Mode {
    Solo,
    Auto,
    Multi,
    Cheerful,
    Challenge,
    ChallengeAuto,
    Mysekai,
}

const MODES: [Mode; 7] = [
    Mode::Solo,
    Mode::Auto,
    Mode::Multi,
    Mode::Cheerful,
    Mode::Challenge,
    Mode::ChallengeAuto,
    Mode::Mysekai,
];

fn random_context(rng: &mut Rng, pool: &CardPool) -> SearchContext {
    let mut context = ready_ctx(pool, ScoreTarget::Score);
    let mode = rng.pick(&MODES);
    let target = rng.pick(&[
        ScoreTarget::Score,
        ScoreTarget::Score,
        ScoreTarget::Bonus,
        ScoreTarget::Mysekai,
        ScoreTarget::Skill,
        ScoreTarget::Power,
    ]);
    let event = !matches!(mode, Mode::Cheerful) && rng.chance(1, 2);
    context.target = target;
    context.live_type = match mode {
        Mode::Solo => LiveType::Solo,
        Mode::Auto => LiveType::Auto,
        Mode::Multi | Mode::Cheerful => LiveType::Multi,
        Mode::Challenge => LiveType::Challenge,
        Mode::ChallengeAuto => LiveType::ChallengeAuto,
        Mode::Mysekai => LiveType::Mysekai,
    };
    context.event_type = if matches!(mode, Mode::Cheerful) {
        Some(EventType::CheerfulCarnival)
    } else if event || matches!(target, ScoreTarget::Bonus) {
        Some(rng.pick(&[EventType::Marathon, EventType::WorldBloom]))
    } else {
        None
    };
    context.base_score = rng.pick(&BASE_SCORES);
    context.base_score_auto = rng.pick(&BASE_SCORES);
    context.fever_score = rng.pick(&FEVER_SCORES);
    context.skill_scores = [
        rng.pick(&SKILL_RATES),
        rng.pick(&SKILL_RATES),
        rng.pick(&SKILL_RATES),
    ];
    context.music_rate_pct = rng.pick(&MUSIC_RATES);
    context.boost_rate_pct = rng.pick(&BOOST_RATES);
    context.life = rng.pick(&LIVES);
    context.other_score = rng.pick(&OTHER_SCORES);
    context.live_skill_order = rng.pick(&[
        LiveSkillOrder::Average,
        LiveSkillOrder::Best,
        LiveSkillOrder::Worst,
        LiveSkillOrder::Specific,
    ]);
    if context.live_skill_order == LiveSkillOrder::Specific {
        context.specific_skill_order = Some(rng.pick(&[[4, 2, 0, 1, 3], [0, 1, 2, 3, 4]]));
    }
    context.skill_reference_strategy = rng.pick(&[
        SkillReferenceStrategy::Max,
        SkillReferenceStrategy::Min,
        SkillReferenceStrategy::Average,
    ]);
    context.best_skill_as_leader = rng.chance(3, 4);
    if matches!(mode, Mode::Multi | Mode::Cheerful) {
        context.multi_teammate_score_up =
            rng.pick(&[None, Some(0), Some(123), Some(250), Some(700)]);
        context.multi_teammate_power = rng.pick(&[None, Some(0), Some(123_457), Some(350_001)]);
    }
    context.honor_bonus = rng.pick(&[0, 0, 1_234, 7_777]);
    context.power_total_cap = rng.pick(&[None, None, Some(336_000)]);
    if matches!(context.event_type, Some(EventType::WorldBloom)) {
        context.is_world_bloom = true;
        context.diff_attr_bonus = rng.pick(&[[0; 6], [0, 1, 2, 3, 4, 5], [0, 0, 3, 7, 11, 13]]);
        context.support_deck = random_support(rng, pool);
        if rng.chance(1, 3) && !matches!(target, ScoreTarget::Bonus) {
            context.is_final_chapter = true;
            context.card_bonus_count_limit = 4;
            context.best_skill_as_leader = false;
            context.support_decks_by_character =
                (0..27).map(|_| random_support(rng, pool)).collect();
            context.leader_honor_bonus_x10 = (0..pool.count())
                .map(|_| rng.pick(&[0u16, 5, 13, 100]))
                .collect();
            context.leader_limit_bonus_x10 = (0..pool.count())
                .map(|_| rng.pick(&[0u16, 1, 27, 150]))
                .collect();
        }
    }
    context
}

fn decks(count: usize) -> Vec<[CardIdx; DECK_SIZE]> {
    let mut all = Vec::new();
    let idx = |value: usize| CardIdx::new(value as u16);
    for a in 0..count {
        for b in a + 1..count {
            for c in b + 1..count {
                for d in c + 1..count {
                    for e in d + 1..count {
                        all.push([idx(a), idx(b), idx(c), idx(d), idx(e)]);
                    }
                }
            }
        }
    }
    all
}

/// Exact aggregate features of one deck, as the tightest admissible search
/// relaxation would pass them to [`ObjectiveBound`].
struct Features {
    power: u32,
    bonus_exact: f64,
    bonus: u32,
    skill: u32,
    leader: u32,
}

fn features(pool: &CardPool, context: &SearchContext, deck: &[CardIdx; DECK_SIZE]) -> Features {
    let bonus_exact = resolve_total_bonus(pool, context, deck);
    Features {
        power: resolve_power_target(pool, deck),
        bonus_exact,
        bonus: bonus_exact.ceil() as u32,
        skill: deck
            .iter()
            .map(|&card| u32::from(pool.skill_max(card)))
            .sum(),
        leader: deck
            .iter()
            .map(|&card| u32::from(pool.skill_max(card)))
            .max()
            .unwrap_or(0),
    }
}

#[derive(Default)]
struct Tally {
    leaves: usize,
    tight: usize,
    near_integer_bonus: usize,
    multi_event: usize,
    noevent: usize,
}

fn assert_dominates(label: &str, upper: u64, leaf: u64, target: ScoreTarget) {
    assert!(
        upper >= leaf,
        "{label}: packed ceiling {upper:#x} < leaf {leaf:#x}"
    );
    if matches!(target, ScoreTarget::Score | ScoreTarget::Bonus) {
        assert!(
            upper >> 32 >= leaf >> 32,
            "{label}: high component {} < {}",
            upper >> 32,
            leaf >> 32
        );
        assert!(
            upper & LOW >= leaf & LOW,
            "{label}: live component {} < {}",
            upper & LOW,
            leaf & LOW
        );
    }
}

fn check_context(pool: &CardPool, context: &SearchContext, label: &str, tally: &mut Tally) {
    let bound = ObjectiveBound::from_context(context);
    let live_type = context.effective_live_type();
    for deck in decks(pool.count()) {
        let Some(leaf) = leaf_evaluate_checked(pool, context, &deck) else {
            continue;
        };
        tally.leaves += 1;
        let features = features(pool, context, &deck);
        let label = format!("{label} deck={deck:?}");
        let upper = bound.ceiling(
            features.power,
            features.bonus,
            features.skill,
            features.leader,
        );
        assert_dominates(&label, upper, leaf, context.target);
        tally.tight += usize::from(upper == leaf);

        // Decimal support lists can land the evaluator's bonus a few ulps
        // above an integer; the directly summed ceiling rounds past it.
        let floor = features.bonus_exact.floor();
        tally.near_integer_bonus +=
            usize::from(features.bonus_exact > floor && features.bonus_exact - floor < 1e-9);

        let multi_score_event = matches!(context.target, ScoreTarget::Score)
            && context.has_event()
            && matches!(live_type, LiveType::Multi)
            && !context.is_world_bloom
            && !context.is_final_chapter;
        if multi_score_event {
            tally.multi_event += 1;
            let upper = bound.ceiling_multi_score_event(
                features.power,
                features.bonus,
                features.skill,
                features.leader,
            );
            assert_dominates(&format!("{label} multi"), upper, leaf, context.target);
        }

        if matches!(context.target, ScoreTarget::Score) && !context.has_event() {
            tally.noevent += 1;
            let numerator = bound.score_noevent_live_numerator_ceiling(
                features.power,
                features.skill,
                features.leader,
            );
            assert!(
                numerator >= (leaf & LOW) as i64 * LIVE_SCORE_BOUND_SCALE,
                "{label} no-event numerator {numerator} < live {}",
                leaf & LOW
            );
        }
    }
}

#[test]
fn aggregate_ceiling_dominates_float_leaf_for_decimal_constants() {
    let mut rng = Rng(0x5eed_0bad_cafe_f00d);
    let mut tally = Tally::default();
    for round in 0..6_000 {
        let pool = random_pool(&mut rng, 7);
        let context = random_context(&mut rng, &pool);
        let label = format!(
            "round={round} target={:?} live={:?} event={:?} order={:?} final={}",
            context.target,
            context.effective_live_type(),
            context.event_type,
            context.live_skill_order,
            context.is_final_chapter
        );
        check_context(&pool, &context, &label, &mut tally);
    }
    assert!(tally.leaves > 50_000, "leaves={}", tally.leaves);
    assert!(tally.tight > 0, "no deck met its aggregate ceiling exactly");
    assert!(tally.multi_event > 1_000, "multi={}", tally.multi_event);
    assert!(tally.noevent > 1_000, "no-event={}", tally.noevent);
}

/// Support lists whose decimal sums are integers make the evaluator's bonus
/// land a few ulps above that integer. The ceiling's own support sum rounds up
/// past it for every consumer: event point, Bonus key and MySekai score.
#[test]
fn near_integer_support_sums_are_dominated_by_every_consumer() {
    let mut rng = Rng(0x051b_1e55);
    let mut tally = Tally::default();
    for x10 in 0..60u16 {
        for list in SUPPORT_LISTS {
            let cards: Vec<_> = (0..6)
                .map(|index| NumCard {
                    char_id: index as u8 + 1,
                    attr: index as u8 % 2,
                    unit_mask: 1,
                    game_id: 400 + index as u16,
                    powers: [110_000 + 17 * index as u32; 4],
                    skill: SkillSlot {
                        skill_type: 0,
                        value: 60 + 20 * index as u8,
                    },
                    skill_min: 60 + 20 * index as u8,
                    skill_max: 60 + 20 * index as u8,
                    base_x10: if index == 0 { x10 } else { 0 },
                    limited_x10: 0,
                })
                .collect();
            let pool = numeric_pool(&cards);
            for target in [ScoreTarget::Score, ScoreTarget::Bonus, ScoreTarget::Mysekai] {
                let mut context = random_context(&mut rng, &pool);
                context.target = target;
                context.event_type = Some(EventType::WorldBloom);
                context.is_world_bloom = true;
                context.is_final_chapter = false;
                context.card_bonus_count_limit = DECK_SIZE;
                context.support_decks_by_character.clear();
                context.diff_attr_bonus = [0; 6];
                if matches!(context.effective_live_type(), LiveType::Cheerful) {
                    context.live_type = LiveType::Solo;
                }
                context.support_deck = SupportDeck {
                    cards: list
                        .iter()
                        .enumerate()
                        .map(|(index, &bonus)| (950 + index as u16, bonus))
                        .collect(),
                    count: list.len() as u8,
                };
                check_context(
                    &pool,
                    &context,
                    &format!("x10={x10} list={list:?}"),
                    &mut tally,
                );
            }
        }
    }
    assert!(
        tally.near_integer_bonus > 100,
        "near-integer={}",
        tally.near_integer_bonus
    );
}

/// Every live type, skill order and target at once, on the same pools.
#[test]
fn aggregate_ceiling_covers_every_mode_order_and_target() {
    let mut rng = Rng(0x0ddb_a110);
    let mut tally = Tally::default();
    for (pool_round, size) in [6usize, 7, 8].into_iter().enumerate() {
        let pool = random_pool(&mut rng, size);
        for mode in MODES {
            for order in [
                LiveSkillOrder::Average,
                LiveSkillOrder::Best,
                LiveSkillOrder::Worst,
                LiveSkillOrder::Specific,
            ] {
                for target in [
                    ScoreTarget::Score,
                    ScoreTarget::Bonus,
                    ScoreTarget::Mysekai,
                    ScoreTarget::Skill,
                    ScoreTarget::Power,
                ] {
                    for event in [false, true] {
                        if matches!(target, ScoreTarget::Bonus) && !event {
                            continue;
                        }
                        let mut context = random_context(&mut rng, &pool);
                        context.target = target;
                        context.live_skill_order = order;
                        context.specific_skill_order =
                            (order == LiveSkillOrder::Specific).then_some([3, 0, 4, 2, 1]);
                        context.is_world_bloom = false;
                        context.is_final_chapter = false;
                        context.card_bonus_count_limit = DECK_SIZE;
                        context.support_decks_by_character.clear();
                        context.live_type = match mode {
                            Mode::Solo => LiveType::Solo,
                            Mode::Auto => LiveType::Auto,
                            Mode::Multi | Mode::Cheerful => LiveType::Multi,
                            Mode::Challenge => LiveType::Challenge,
                            Mode::ChallengeAuto => LiveType::ChallengeAuto,
                            Mode::Mysekai => LiveType::Mysekai,
                        };
                        context.event_type = match (mode, event) {
                            (Mode::Cheerful, _) => Some(EventType::CheerfulCarnival),
                            (_, true) => Some(EventType::Marathon),
                            (_, false) => None,
                        };
                        let label = format!(
                            "pool={pool_round} mode={mode:?} order={order:?} target={target:?} event={event}"
                        );
                        check_context(&pool, &context, &label, &mut tally);
                    }
                }
            }
        }
    }
    assert!(tally.leaves > 10_000, "leaves={}", tally.leaves);
}

/// Deliberate floor boundaries: choose the total power so that the fixed-point
/// live ceiling is an exact integer, or one to three millionths below one.
/// Decimal constants make the exact real value an integer while the stored
/// `f64` lies on either side of it.
/// Smallest `x >= 0` with `a * x + c ≡ r (mod m)` and its period, if any.
fn solve_affine_residue(a: i64, c: i64, r: i64, m: i64) -> Option<(i64, i64)> {
    fn extended_gcd(a: i64, b: i64) -> (i64, i64, i64) {
        if b == 0 {
            (a, 1, 0)
        } else {
            let (g, x, y) = extended_gcd(b, a % b);
            (g, y, x - (a / b) * y)
        }
    }
    let (g, inverse, _) = extended_gcd(a.rem_euclid(m), m);
    let wanted = (r - c).rem_euclid(m);
    if wanted % g != 0 {
        return None;
    }
    let period = m / g;
    let x = ((wanted / g) as i128 * inverse as i128).rem_euclid(period as i128) as i64;
    Some((x, period))
}

#[test]
fn live_ceiling_survives_exact_integer_and_near_integer_products() {
    let mut rng = Rng(0x00b0_0da2);
    let mut exact_hits = 0usize;
    let mut near_hits = 0usize;
    let mut equal = 0usize;
    const CARD_POWER_MAX: i64 = (1 << 18) - 1;
    for round in 0..400 {
        let mode = rng.pick(&[Mode::Solo, Mode::Auto, Mode::Multi, Mode::Cheerful]);
        let skills: [u8; DECK_SIZE] =
            core::array::from_fn(|_| rng.pick(&[0u8, 7, 20, 50, 99, 100, 120, 150]));
        let build = |deck_power: u32| {
            let cards: Vec<_> = (0..DECK_SIZE)
                .map(|index| {
                    let power = deck_power / DECK_SIZE as u32
                        + u32::from((index as u32) < deck_power % DECK_SIZE as u32);
                    NumCard {
                        char_id: index as u8 + 1,
                        attr: 0,
                        unit_mask: 1,
                        game_id: 500 + index as u16,
                        powers: [power; 4],
                        skill: SkillSlot {
                            skill_type: 0,
                            value: skills[index],
                        },
                        skill_min: skills[index],
                        skill_max: skills[index],
                        base_x10: 0,
                        limited_x10: 0,
                    }
                })
                .collect::<Vec<_>>();
            numeric_pool(&cards)
        };
        let probe = build(1);
        let mut context = random_context(&mut rng, &probe);
        context.target = ScoreTarget::Score;
        context.is_world_bloom = false;
        context.is_final_chapter = false;
        context.support_decks_by_character.clear();
        context.power_total_cap = None;
        context.live_type = match mode {
            Mode::Auto => LiveType::Auto,
            Mode::Multi | Mode::Cheerful => LiveType::Multi,
            _ => LiveType::Solo,
        };
        context.event_type = match mode {
            Mode::Cheerful => Some(EventType::CheerfulCarnival),
            _ if rng.chance(1, 2) => Some(EventType::Marathon),
            _ => None,
        };
        let bound = ObjectiveBound::from_context(&context);
        let skill: u32 = skills.iter().map(|&value| u32::from(value)).sum();
        let leader = u32::from(skills.into_iter().max().unwrap_or(0));
        // The numerator is affine in the (honor-inclusive) total power.
        let constant = bound.calc_live_score_bound_numerator(0, skill, leader);
        let slope = bound.calc_live_score_bound_numerator(1, skill, leader) - constant;
        let honor = i64::from(context.honor_bonus);
        let highest = DECK_SIZE as i64 * CARD_POWER_MAX + honor;
        // The exact residue, and the two closest reachable residues below the
        // next integer (the slope is a multiple of four, so not every residue is).
        let below: Vec<_> = (1..=64)
            .map(|gap| LIVE_SCORE_BOUND_SCALE - gap)
            .filter_map(|residue| {
                solve_affine_residue(slope, constant, residue, LIVE_SCORE_BOUND_SCALE)
                    .map(|solution| (residue, solution))
            })
            .take(2)
            .collect();
        let exact = solve_affine_residue(slope, constant, 0, LIVE_SCORE_BOUND_SCALE)
            .map(|solution| (0, solution));
        for (residue, (first, period)) in exact.into_iter().chain(below) {
            let mut total = first;
            while total < 100_000 + honor {
                total += period;
            }
            let mut taken = 0;
            while total <= highest && taken < 3 {
                let numerator = bound.calc_live_score_bound_numerator(total as u32, skill, leader);
                assert_eq!(numerator % LIVE_SCORE_BOUND_SCALE, residue);
                taken += 1;
                exact_hits += usize::from(residue == 0);
                near_hits += usize::from(residue != 0);
                let pool = build((total - honor) as u32);
                let deck = collect_first_five(&pool);
                let leaf = leaf_evaluate_checked(&pool, &context, &deck).expect("legal deck");
                let power = resolve_power_target(&pool, &deck);
                assert_eq!(i64::from(power) + honor, total);
                let upper = bound.ceiling(power, 0, skill, leader);
                assert_dominates(
                    &format!("round={round} mode={mode:?} total={total} residue={residue}"),
                    upper,
                    leaf,
                    ScoreTarget::Score,
                );
                equal += usize::from(upper & LOW == leaf & LOW);
                total += period * ((1_000 + period - 1) / period);
            }
        }
    }
    assert!(exact_hits > 100, "exact={exact_hits}");
    assert!(near_hits > 100, "near={near_hits}");
    assert!(equal > 0, "no boundary deck met its live ceiling");
}

/// Deliberate event-point boundaries: choose the bonus so that the first
/// integer event stage `base * music * (bonus + 100) / 10^4`, and for
/// Cheerful also the life stage, lands exactly on an integer or one grid step
/// below one, with non-binary music, boost and life rates.
#[test]
fn event_ceiling_survives_exact_integer_and_near_integer_stages() {
    let mut rng = Rng(0xe7e7_0001);
    let mut hits = 0usize;
    let mut equal = 0usize;
    for round in 0..600 {
        let mode = rng.pick(&[Mode::Solo, Mode::Auto, Mode::Multi, Mode::Cheerful]);
        let power = rng.pick(&[20_000u32, 55_555, 120_001, 250_000]);
        let cards: Vec<_> = (0..DECK_SIZE)
            .map(|index| NumCard {
                char_id: index as u8 + 1,
                attr: 0,
                unit_mask: 1,
                game_id: 600 + index as u16,
                powers: [power; 4],
                skill: SkillSlot {
                    skill_type: 0,
                    value: 100,
                },
                skill_min: 100,
                skill_max: 100,
                base_x10: 0,
                limited_x10: 0,
            })
            .collect();
        let template = numeric_pool(&cards);
        let mut context = random_context(&mut rng, &template);
        context.target = ScoreTarget::Score;
        context.is_world_bloom = false;
        context.is_final_chapter = false;
        context.support_decks_by_character.clear();
        context.live_type = match mode {
            Mode::Auto => LiveType::Auto,
            Mode::Multi | Mode::Cheerful => LiveType::Multi,
            _ => LiveType::Solo,
        };
        context.event_type = Some(if matches!(mode, Mode::Cheerful) {
            EventType::CheerfulCarnival
        } else {
            EventType::Marathon
        });
        let bound = ObjectiveBound::from_context(&context);
        let deck_power = resolve_power_target(&template, &collect_first_five(&template));
        let live = bound.calc_live_score_bound(
            bound.clamp_power_total(deck_power + context.honor_bonus),
            500,
            100,
        );
        for bonus in 0..=2_000u32 {
            let event_base = match context.effective_live_type() {
                LiveType::Solo | LiveType::Auto => 100 + i64::from(live) / 20_000,
                _ => {
                    let other = if context.other_score == 0 {
                        i64::from(live) * 4
                    } else {
                        i64::from(context.other_score)
                    };
                    110 + i64::from(live) / 17_000 + (other / 340_000).min(13)
                }
            };
            let first = event_base * i64::from(context.music_rate_pct) * (i64::from(bonus) + 100);
            let first_residue = first % 10_000;
            let inner = first / 10_000;
            let life_residue = inner * i64::from(bound.life_rate_num) % 5000;
            let on_grid = first_residue == 0 || first_residue == 9_999;
            let on_life = matches!(context.effective_live_type(), LiveType::Cheerful)
                && (life_residue == 0 || life_residue == 4_999);
            if !on_grid && !on_life {
                continue;
            }
            hits += 1;
            let mut cards = cards.clone();
            // Spread the bonus in tenths over the deck; the total stays integral.
            let mut remaining = bonus * 10;
            for card in &mut cards {
                let share = remaining.min(4_000);
                card.base_x10 = share as u16;
                remaining -= share;
            }
            let pool = numeric_pool(&cards);
            let deck = collect_first_five(&pool);
            let leaf = leaf_evaluate_checked(&pool, &context, &deck).expect("legal deck");
            let total = resolve_total_bonus(&pool, &context, &deck);
            assert_eq!(total, f64::from(bonus));
            let upper = bound.ceiling(resolve_power_target(&pool, &deck), bonus, 500, 100);
            assert_dominates(
                &format!("round={round} mode={mode:?} bonus={bonus}"),
                upper,
                leaf,
                ScoreTarget::Score,
            );
            equal += usize::from(upper >> 32 == leaf >> 32);
        }
    }
    assert!(hits > 1_000, "hits={hits}");
    assert!(equal > 0, "no boundary deck met its event ceiling");
}

/// The correlated no-event bound: every prefix of every deck, including the
/// last level where only one card remains to be chosen.
#[test]
fn correlated_bound_dominates_float_leaf_at_every_prefix() {
    let mut rng = Rng(0x00c0_22e1);
    let mut built = 0usize;
    let mut checked = 0usize;
    for round in 0..300 {
        // Power and skill trade off, which is what makes the correlated plane
        // tighter than independent maxima (otherwise it disables itself).
        let cards: Vec<_> = (0..8)
            .map(|index| {
                let strong = index % 2 == 0;
                let power = if strong {
                    230_000 + rng.below(30_000) as u32
                } else {
                    90_000 + rng.below(30_000) as u32
                };
                let skill = if strong {
                    rng.pick(&[0u8, 5, 10])
                } else {
                    rng.pick(&[120u8, 150, 200, 255])
                };
                NumCard {
                    char_id: index as u8 + 1,
                    attr: rng.below(2) as u8,
                    unit_mask: 1,
                    game_id: 700 + index as u16,
                    powers: [power; 4],
                    skill: SkillSlot {
                        skill_type: 0,
                        value: skill,
                    },
                    skill_min: skill,
                    skill_max: skill,
                    base_x10: 0,
                    limited_x10: 0,
                }
            })
            .collect();
        let pool = numeric_pool(&cards);
        let mut context = ready_ctx(&pool, ScoreTarget::Score);
        context.live_type = rng.pick(&[LiveType::Solo, LiveType::Auto]);
        context.live_skill_order = LiveSkillOrder::Average;
        context.base_score = rng.pick(&BASE_SCORES);
        context.base_score_auto = rng.pick(&BASE_SCORES);
        let rates = rng.pick(&SKILL_RATES);
        context.skill_scores = [rates, rates, rates];
        if rng.chance(1, 3) {
            context.power_total_cap = Some(336_000);
        }
        let Some(correlated) = CorrelatedBound::build(&pool, &context, None, 1, 0) else {
            continue;
        };
        built += 1;
        for deck in decks(pool.count()) {
            let Some(leaf) = leaf_evaluate_checked(&pool, &context, &deck) else {
                continue;
            };
            for depth in 0..DECK_SIZE {
                let mut used = UsedSet::new();
                let mut partial = PartialDeck::default();
                for &card in &deck[..depth] {
                    used.insert(pool.char_id(card));
                    partial.power += pool.power_max(card);
                    partial.skill += u32::from(pool.skill_max(card));
                    partial.max_skill = partial.max_skill.max(pool.skill_max(card));
                }
                let start = if depth == 0 {
                    0
                } else {
                    deck[depth - 1].raw() + 1
                };
                let upper = correlated.upper_bound(start, DECK_SIZE - depth, &used, &partial);
                assert!(
                    upper & LOW >= leaf & LOW,
                    "round={round} deck={deck:?} depth={depth}: {} < {}",
                    upper & LOW,
                    leaf & LOW
                );
                checked += 1;
            }
        }
    }
    assert!(built > 20, "correlated bound built only {built} times");
    assert!(checked > 10_000, "checked={checked}");
}

/// World Bloom support sums: the ceiling of the directly summed remaining
/// support list dominates the evaluator's sum for every completion of every
/// prefix, including lists whose decimal sums are exact integers.
#[test]
fn world_bloom_support_ceiling_dominates_evaluator_bonus_at_every_prefix() {
    let mut rng = Rng(0x05a9_90e7);
    let mut checked = 0usize;
    for round in 0..800 {
        let pool = random_pool(&mut rng, 7);
        let mut context = random_context(&mut rng, &pool);
        context.target = ScoreTarget::Score;
        context.event_type = Some(EventType::WorldBloom);
        context.is_world_bloom = true;
        context.support_deck = random_support(&mut rng, &pool);
        let final_chapter = rng.chance(1, 2);
        context.is_final_chapter = final_chapter;
        if final_chapter {
            context.best_skill_as_leader = false;
            context.card_bonus_count_limit = 4;
            context.support_decks_by_character =
                (0..27).map(|_| random_support(&mut rng, &pool)).collect();
        } else {
            context.support_decks_by_character.clear();
            context.card_bonus_count_limit = DECK_SIZE;
        }
        let suffix = SuffixBound::build(&pool, &context);
        for deck in decks(pool.count()) {
            let total = resolve_total_bonus(&pool, &context, &deck);
            let cards: u32 = deck
                .iter()
                .enumerate()
                .map(|(slot, &card)| {
                    let exact = pool.event_bonus_exact(card);
                    let leader = if final_chapter && slot == 0 {
                        context.leader_bonus_upper_at(card.raw())
                    } else {
                        0
                    };
                    exact.base_ceil() + exact.limited_ceil() + leader
                })
                .sum();
            for depth in 0..=DECK_SIZE {
                let mut selected = [0u16; DECK_SIZE];
                let mut attr_set = 0u8;
                let mut used = UsedSet::new();
                for (slot, &card) in deck[..depth].iter().enumerate() {
                    selected[slot] = pool.game_id(card);
                    attr_set |= 1 << pool.attr(card);
                    used.insert(pool.char_id(card));
                }
                let start = if depth == 0 {
                    0
                } else {
                    deck[depth - 1].raw() + 1
                };
                let extra = suffix.world_bloom_extra_bonus_bound(
                    attr_set,
                    suffix.support_ceiling(&selected, depth),
                    DECK_SIZE - depth,
                    start,
                    used.bits(),
                );
                assert!(
                    f64::from(cards + extra) >= total,
                    "round={round} final={final_chapter} deck={deck:?} depth={depth}: {} < {total}",
                    cards + extra
                );
                checked += 1;
            }
        }
    }
    assert!(checked > 100_000, "checked={checked}");
}

/// End-to-end: tricky decimal constants and support lists, compared with the
/// independent exhaustive oracle for every Top-K size.
#[test]
fn search_matches_oracle_under_decimal_constants() {
    let mut rng = Rng(0x0e2e_0ac1);
    let mut compared = 0usize;
    for round in 0..160 {
        let pool = random_pool(&mut rng, 8);
        let mut context = random_context(&mut rng, &pool);
        if matches!(context.target, ScoreTarget::Power | ScoreTarget::Skill) {
            context.target = ScoreTarget::Score;
        }
        if matches!(
            context.effective_live_type(),
            LiveType::Challenge | LiveType::ChallengeAuto
        ) {
            context.live_type = LiveType::Solo;
        }
        // Pool construction maps a MySekai live Score request to the MySekai
        // target. The ordered-slot placement model covers single-player lives.
        if context.live_type == LiveType::Mysekai {
            if context.target == ScoreTarget::Score {
                context.target = ScoreTarget::Mysekai;
            }
            if context.live_skill_order == LiveSkillOrder::Specific {
                context.live_skill_order = LiveSkillOrder::Best;
            }
        }
        if context.is_final_chapter {
            context.forced_leader_character_id = rng.chance(1, 2).then_some(1);
        }
        for top_k in [1, 5, 30] {
            let params = SearchParams {
                top_k,
                timeout_ms: 0,
            };
            let outcome = search(&pool, &context, &params);
            assert_eq!(outcome.completion(), SearchCompletion::Complete);
            let (expected, _) = ExactOracle::new(&pool, &context).search(&params);
            assert_eq!(
                outcome.results,
                expected,
                "round={round} K={top_k} target={:?} live={:?} event={:?} order={:?} final={}",
                context.target,
                context.effective_live_type(),
                context.event_type,
                context.live_skill_order,
                context.is_final_chapter
            );
            compared += 1;
        }
    }
    assert_eq!(compared, 480);
}

/// A MySekai live has no live-score formula of its own; the leaf evaluator
/// scores it with the solo constants. The Bonus key keeps that live score as
/// its tie-break, so its ceiling must too.
#[test]
fn mysekai_live_bonus_ceiling_keeps_the_live_tiebreak() {
    let cards: Vec<_> = (0..6)
        .map(|index| NumCard {
            char_id: index as u8 + 1,
            attr: 0,
            unit_mask: 1,
            game_id: 800 + index as u16,
            powers: [100_000 + index as u32 * 1_000; 4],
            skill: SkillSlot {
                skill_type: 0,
                value: 100,
            },
            skill_min: 100,
            skill_max: 100,
            base_x10: 100,
            limited_x10: 0,
        })
        .collect();
    let pool = numeric_pool(&cards);
    let mut context = ready_ctx(&pool, ScoreTarget::Bonus);
    context.live_type = LiveType::Mysekai;
    context.event_type = Some(EventType::Marathon);
    context.base_score = 1.1;
    context.skill_scores[0] = [0.1; 6];
    // Every deck has the same bonus total, so the Top-K order is decided by
    // the live-score tie-break alone.
    for top_k in [1, 2, 6] {
        let params = SearchParams {
            top_k,
            timeout_ms: 0,
        };
        let outcome = search(&pool, &context, &params);
        assert_eq!(outcome.completion(), SearchCompletion::Complete);
        let (expected, _) = ExactOracle::new(&pool, &context).search(&params);
        assert_eq!(outcome.results, expected, "K={top_k}");
    }
    let deck = collect_first_five(&pool);
    let leaf = leaf_evaluate_checked(&pool, &context, &deck).expect("legal deck");
    assert!(leaf & LOW > 0, "the leaf key carries a live score");
    let features = features(&pool, &context, &deck);
    let upper = ObjectiveBound::from_context(&context).ceiling(
        features.power,
        features.bonus,
        features.skill,
        features.leader,
    );
    assert_dominates("mysekai bonus", upper, leaf, ScoreTarget::Bonus);
}

/// For the profile below, a deck holding the second 2.2 and the first 0.2
/// leaves `2.2 + 0.6 + 0.2`, which the evaluator sums to 3.0000000000000004.
/// The grouped Final Chapter solvers sum the remaining entries in the same
/// order and return the oracle Top-K.
#[test]
fn final_chapter_support_sum_rounding_keeps_oracle_top_k() {
    let profile = SupportDeck {
        cards: vec![(401, 2.2), (402, 2.2), (403, 0.6), (404, 0.2), (405, 0.2)],
        count: 3,
    };
    let cards: Vec<_> = (0..8)
        .map(|index| NumCard {
            char_id: index as u8 + 1,
            attr: index as u8 % 3,
            unit_mask: 1,
            game_id: 401 + index as u16,
            powers: [120_000 + 3_001 * index as u32; 4],
            skill: SkillSlot {
                skill_type: 0,
                value: 80 + 10 * index as u8,
            },
            skill_min: 80 + 10 * index as u8,
            skill_max: 80 + 10 * index as u8,
            base_x10: 0,
            limited_x10: 0,
        })
        .collect();
    let pool = numeric_pool(&cards);
    let mut compared = 0usize;
    for live_type in [LiveType::Solo, LiveType::Auto, LiveType::Multi] {
        for music_rate_pct in [100, 107, 114, 130] {
            for forced_leader in [None, Some(1), Some(3)] {
                let mut context = ready_ctx(&pool, ScoreTarget::Score);
                context.live_type = live_type;
                context.event_type = Some(EventType::WorldBloom);
                context.is_world_bloom = true;
                context.is_final_chapter = true;
                context.best_skill_as_leader = false;
                context.card_bonus_count_limit = 4;
                context.forced_leader_character_id = forced_leader;
                context.base_score = 1.1;
                context.base_score_auto = 0.95;
                context.fever_score = 0.1;
                context.skill_scores = [SKILL_RATES[2]; 3];
                context.music_rate_pct = music_rate_pct;
                context.support_deck = profile.clone();
                context.support_decks_by_character = vec![profile.clone(); 27];
                let deck = [1usize, 3, 5, 6, 7].map(|index| CardIdx::new(index as u16));
                let bonus = resolve_total_bonus(&pool, &context, &deck);
                assert!(bonus > 3.0 && bonus < 3.0 + 1e-12, "bonus={bonus}");
                for top_k in [1, 5, 20] {
                    let params = SearchParams {
                        top_k,
                        timeout_ms: 0,
                    };
                    let outcome = search(&pool, &context, &params);
                    assert_eq!(outcome.completion(), SearchCompletion::Complete);
                    let (expected, _) = ExactOracle::new(&pool, &context).search(&params);
                    assert_eq!(
                        outcome.results, expected,
                        "live={live_type:?} music={music_rate_pct} leader={forced_leader:?} K={top_k}"
                    );
                    compared += 1;
                }
            }
        }
    }
    assert_eq!(compared, 108);
}
