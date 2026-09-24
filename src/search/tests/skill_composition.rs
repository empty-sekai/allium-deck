//! Composition-aware Skill ceilings of the numeric solver.
//!
//! Pools mix fixed, unit-count, different-unit and reference skills across
//! several characters. Unit-count tables cover a single-unit table, a table
//! over the Virtual Singer unit and a non-monotone table; Virtual Singer cards
//! carry the piapro unit alone or together with a support unit, and some cards
//! are cultivation variants that share a public game id.
use super::*;
use crate::search::solver::numeric::audit_skill_path;

const PIAPRO: u8 = 1 << 5;

const UNIT_COUNT_SKILLS: [UnitCountSkill; 5] = [
    UnitCountSkill {
        unit: 0,
        score_up: [80, 90, 100, 110, 130],
    },
    UnitCountSkill {
        unit: 1,
        score_up: [85, 95, 105, 115, 135],
    },
    UnitCountSkill {
        unit: 3,
        score_up: [100, 110, 120, 130, 150],
    },
    UnitCountSkill {
        unit: 5,
        score_up: [60, 70, 80, 90, 120],
    },
    UnitCountSkill {
        unit: 2,
        score_up: [30, 100, 80, 190, 140],
    },
];

const DIFF_SKILLS: [DiffSkill; 2] = [
    DiffSkill {
        base: 70,
        increment: 30,
    },
    DiffSkill {
        base: 90,
        increment: 25,
    },
];

const REF_SKILLS: [RefSkill; 2] = [
    RefSkill { rate: 50, max: 60 },
    RefSkill { rate: 33, max: 41 },
];

/// Characters `1..=6` belong to one unit each; `21` and `22` are Virtual
/// Singers.
const CHARACTERS: [u8; 8] = [1, 2, 3, 4, 5, 6, 21, 22];

#[derive(Clone, Copy)]
struct SkillCard {
    char_id: u8,
    unit_mask: u8,
    game_id: u16,
    skill: SkillSlot,
    skill_min: u8,
    skill_max: u8,
    reference: u16,
}

fn unit_mask_of(rng: &mut ExactLcg, char_id: u8) -> u8 {
    if char_id >= 21 {
        match rng.range(0, 4) {
            0 => PIAPRO,
            support => PIAPRO | 1 << [0u8, 1, 3][support as usize - 1],
        }
    } else {
        1 << ((char_id - 1) % 5)
    }
}

/// A skill with the `skill_min` / `skill_max` / reference summary pool
/// construction produces for it.
fn random_skill(rng: &mut ExactLcg) -> (SkillSlot, u8, u8, u16) {
    match rng.range(0, 7) {
        0..=2 => {
            let index = rng.range(0, UNIT_COUNT_SKILLS.len() as u32) as usize;
            let table = UNIT_COUNT_SKILLS[index].score_up;
            let min = *table.iter().min().unwrap();
            let max = *table.iter().max().unwrap();
            (
                SkillSlot {
                    skill_type: 1,
                    value: index as u8 + 1,
                },
                min,
                max,
                u16::from(max),
            )
        }
        3 => {
            let index = rng.range(0, DIFF_SKILLS.len() as u32) as usize;
            let DiffSkill { base, increment } = DIFF_SKILLS[index];
            let full = base + 2 * increment;
            // Some caps clamp the second counted unit.
            let max = if rng.range(0, 2) == 0 {
                full
            } else {
                base + increment + 10
            };
            (
                SkillSlot {
                    skill_type: 2,
                    value: index as u8 + 1,
                },
                base,
                max,
                u16::from(full),
            )
        }
        4 => {
            let index = rng.range(0, REF_SKILLS.len() as u32) as usize;
            let base = [40u8, 60, 90][rng.range(0, 3) as usize];
            let max = base + REF_SKILLS[index].max;
            (
                SkillSlot {
                    skill_type: 3,
                    value: index as u8 + 1,
                },
                base,
                max,
                u16::from(max),
            )
        }
        _ => {
            let value = 60 + 10 * rng.range(0, 10) as u8;
            (
                SkillSlot {
                    skill_type: 0,
                    value,
                },
                value,
                value,
                u16::from(value),
            )
        }
    }
}

fn random_cards(rng: &mut ExactLcg, count: usize) -> Vec<SkillCard> {
    let mut cards: Vec<SkillCard> = Vec::with_capacity(count);
    for index in 0..count {
        let (skill, skill_min, skill_max, reference) = random_skill(rng);
        let card = if index >= CHARACTERS.len() && rng.range(0, 3) == 0 {
            // A cultivation variant of an earlier card, with the same skill
            // (an exact tie) or another one.
            let original = cards[rng.range(0, index as u32) as usize];
            if rng.range(0, 2) == 0 {
                original
            } else {
                SkillCard {
                    skill,
                    skill_min,
                    skill_max,
                    reference,
                    ..original
                }
            }
        } else {
            // The first cards cover every character, so decks always exist.
            let char_id = if index < CHARACTERS.len() {
                CHARACTERS[index]
            } else {
                CHARACTERS[rng.range(0, CHARACTERS.len() as u32) as usize]
            };
            SkillCard {
                char_id,
                unit_mask: unit_mask_of(rng, char_id),
                game_id: 700 + index as u16,
                skill,
                skill_min,
                skill_max,
                reference,
            }
        };
        cards.push(card);
    }
    cards
}

fn composition_pool(cards: &[SkillCard]) -> CardPool {
    let mut builder = PoolBuilder::new(cards.len() as u16);
    for skill in UNIT_COUNT_SKILLS {
        builder.add_unit_count_skill(skill);
    }
    for skill in DIFF_SKILLS {
        builder.add_diff_skill(skill);
    }
    for skill in REF_SKILLS {
        builder.add_ref_skill(skill);
    }
    for (index, card) in cards.iter().enumerate() {
        let dense = index as u16;
        let power = 20_000 + u32::from(dense) * 37;
        let (values, high_bits) = encode_power(power);
        builder.set_power_values(dense, values);
        builder.set_power_lut(dense, high_bits);
        builder.set_power_max(dense, power);
        builder.set_skill(dense, card.skill);
        builder.set_skill_min(dense, card.skill_min);
        builder.set_skill_max(dense, card.skill_max);
        builder.set_skill_reference(dense, card.reference);
        builder.set_event_bonus(dense, EventBonusExact::from_whole(0, 0));
        builder.set_char_id(dense, card.char_id);
        builder.set_attr(dense, 0);
        builder.set_unit_mask(dense, card.unit_mask);
        builder.set_game_id(dense, card.game_id);
        builder.mark_char(card.char_id, dense);
        for unit in 0..6u8 {
            if card.unit_mask & (1 << unit) != 0 {
                builder.mark_unit(unit, dense);
            }
        }
        builder.mark_attr(0, dense);
    }
    builder.freeze()
}

const LIVES: [LiveType; 3] = [LiveType::Solo, LiveType::Multi, LiveType::Auto];

const STRATEGIES: [SkillReferenceStrategy; 3] = [
    SkillReferenceStrategy::Average,
    SkillReferenceStrategy::Max,
    SkillReferenceStrategy::Min,
];

/// Request constraints of the oracle comparison, by `variant`.
fn skill_context(
    rng: &mut ExactLcg,
    pool: &CardPool,
    live: LiveType,
    variant: usize,
) -> SearchContext {
    let mut context = ready_ctx(pool, ScoreTarget::Skill);
    context.live_type = live;
    context.skill_reference_strategy = STRATEGIES[rng.range(0, 3) as usize];
    let card = |rng: &mut ExactLcg| CardIdx::new(rng.range(0, pool.count() as u32) as u16);
    match variant {
        0 => {}
        1 => context.fixed_card_ids = vec![pool.game_id(card(rng))],
        2 => context.fixed_character_ids = vec![pool.char_id(card(rng))],
        3 => context.forced_leader_character_id = Some(pool.char_id(card(rng))),
        4 => {
            context.multi_live_score_up_lower_bound =
                Some([150.0, 175.5, 190.0][rng.range(0, 3) as usize]);
        }
        5 => {
            context.fixed_card_ids = vec![pool.game_id(card(rng)), pool.game_id(card(rng))];
            context.fixed_character_ids = vec![pool.char_id(card(rng))];
        }
        6 => context.best_skill_as_leader = false,
        _ => {
            let fixed = card(rng);
            context.fixed_card_ids = vec![pool.game_id(fixed)];
            context.forced_leader_character_id = Some(pool.char_id(card(rng)));
            context.multi_live_score_up_lower_bound = Some(160.0);
        }
    }
    context
}

#[test]
fn skill_target_matches_oracle_for_composition_skills() {
    let mut rng = ExactLcg(0x5c11_c0de);
    let mut compared = 0usize;
    for round in 0..96 {
        let cards = random_cards(&mut rng, 10 + round % 2);
        let pool = composition_pool(&cards);
        let live = LIVES[round % 3];
        let variant = (round / 3) % 8;
        let context = skill_context(&mut rng, &pool, live, variant);
        for top_k in [1, 5, 30] {
            let params = SearchParams {
                top_k,
                timeout_ms: 0,
            };
            let actual = search_exact(&pool, &context, &params);
            let (expected, _) = ExactOracle::new(&pool, &context).search(&params);
            assert_eq!(
                actual, expected,
                "round={round} K={top_k} live={live:?} variant={variant}"
            );
            compared += 1;
        }
    }
    assert_eq!(compared, 288);
}

/// The depth-first Score search bounds complete decks with the same
/// composition-aware ceilings.
#[test]
fn score_target_matches_oracle_for_composition_skills() {
    let mut rng = ExactLcg(0x5c0_2e11);
    let mut compared = 0usize;
    for round in 0..48 {
        let cards = random_cards(&mut rng, 11 + round % 3);
        let pool = composition_pool(&cards);
        let mut context = ready_ctx(&pool, ScoreTarget::Score);
        context.live_type = LIVES[round % 3];
        context.skill_reference_strategy = STRATEGIES[rng.range(0, 3) as usize];
        context.base_score = 1.1;
        context.skill_scores = [[0.12, 0.02, 0.04, 0.06, 0.09, 0.03]; 3];
        if round % 2 == 1 {
            context.event_type = Some(EventType::Marathon);
        }
        for top_k in [1, 5, 30] {
            let params = SearchParams {
                top_k,
                timeout_ms: 0,
            };
            let actual = search_exact(&pool, &context, &params);
            let (expected, _) = ExactOracle::new(&pool, &context).search(&params);
            assert_eq!(
                actual, expected,
                "round={round} K={top_k} live={:?}",
                context.live_type
            );
            compared += 1;
        }
    }
    assert_eq!(compared, 144);
}

#[test]
fn skill_path_bounds_dominate_every_deck() {
    let mut rng = ExactLcg(0xd0_5c11);
    let mut decks = 0usize;
    for round in 0..18 {
        let cards = random_cards(&mut rng, 13);
        let pool = composition_pool(&cards);
        for live in LIVES {
            let mut context = ready_ctx(&pool, ScoreTarget::Skill);
            context.live_type = live;
            context.skill_reference_strategy = STRATEGIES[rng.range(0, 3) as usize];
            if rng.range(0, 3) == 0 {
                context.forced_leader_character_id = Some(CHARACTERS[round % CHARACTERS.len()]);
            }
            let n = pool.count() as u16;
            for a in 0..n {
                for b in a + 1..n {
                    for c in b + 1..n {
                        for d in c + 1..n {
                            for e in d + 1..n {
                                let deck = [a, b, c, d, e].map(CardIdx::new);
                                let mut chars = deck.map(|card| pool.char_id(card));
                                chars.sort_unstable();
                                let mut ids = deck.map(|card| pool.game_id(card));
                                ids.sort_unstable();
                                if chars.windows(2).any(|pair| pair[0] == pair[1])
                                    || ids.windows(2).any(|pair| pair[0] == pair[1])
                                {
                                    continue;
                                }
                                let Some(key) =
                                    evaluate::leaf_evaluate_checked(&pool, &context, &deck)
                                else {
                                    continue;
                                };
                                check_path(&pool, &context, &deck, key, round);
                                decks += 1;
                            }
                        }
                    }
                }
            }
        }
    }
    assert!(decks > 1_000, "only {decks} decks checked");
}

/// Two cultivation variants of one card resolve to the same value, but the
/// unit-count variant has the larger `skill_max` and is searched first. The
/// other variant has the smaller dense index, so it is the canonical
/// representative; its branch reaches the incumbent objective and public set
/// exactly and must still be searched.
#[test]
fn equal_objective_variant_replaces_the_first_representative() {
    let fixed = |char_id: u8, game_id: u16, value: u8| SkillCard {
        char_id,
        unit_mask: 1 << (char_id - 1),
        game_id,
        skill: SkillSlot {
            skill_type: 0,
            value,
        },
        skill_min: value,
        skill_max: value,
        reference: u16::from(value),
    };
    // Table 4 counts the piapro unit, which no member carries: it resolves to
    // its one-member entry, 60.
    let counted = SkillCard {
        skill: SkillSlot {
            skill_type: 1,
            value: 4,
        },
        skill_min: 60,
        skill_max: 120,
        reference: 120,
        ..fixed(1, 900, 60)
    };
    let cards = [
        fixed(1, 900, 60),
        fixed(2, 901, 90),
        fixed(3, 902, 90),
        fixed(4, 903, 90),
        fixed(5, 904, 90),
        counted,
    ];
    let pool = composition_pool(&cards);
    for live in LIVES {
        let mut context = ready_ctx(&pool, ScoreTarget::Skill);
        context.live_type = live;
        let params = SearchParams {
            top_k: 1,
            timeout_ms: 0,
        };
        let actual = search_exact(&pool, &context, &params);
        let (expected, _) = ExactOracle::new(&pool, &context).search(&params);
        assert_eq!(actual, expected, "live={live:?}");
        assert!(
            expected[0].cards.contains(&CardIdx::new(0)),
            "live={live:?}: the smaller dense variant is canonical"
        );
    }
}

fn check_path(
    pool: &CardPool,
    context: &SearchContext,
    deck: &[CardIdx; DECK_SIZE],
    key: u64,
    round: usize,
) {
    let summary = summarize_deck(pool, context, deck).expect("a scored deck has a summary");
    let resolved = |card: CardIdx| {
        let slot = summary
            .ordered_cards
            .iter()
            .position(|&other| other == card)
            .expect("summary holds every member");
        summary.card_skill_score_up[slot]
    };
    let audit = audit_skill_path(pool, context, deck);
    let label = format!("round={round} live={:?} deck={deck:?}", context.live_type);
    for depth in 0..DECK_SIZE {
        let frontier = audit.frontier[depth]
            .unwrap_or_else(|| panic!("{label}: depth {depth} has the deck as a completion"));
        assert!(
            frontier >= key,
            "{label}: frontier {frontier} < {key} at {depth}"
        );
        assert!(
            audit.global[depth] >= key,
            "{label}: global {} < {key} at {depth}",
            audit.global[depth]
        );
        assert!(
            audit.child[depth] >= key,
            "{label}: child {} < {key} at {depth}",
            audit.child[depth]
        );
        for (member, &card) in audit.order.iter().enumerate() {
            let ceiling = f64::from(audit.ceilings[depth][member]);
            assert!(
                ceiling >= resolved(card),
                "{label}: member {member} ceiling {ceiling} < {} at {depth}",
                resolved(card)
            );
        }
    }
}
