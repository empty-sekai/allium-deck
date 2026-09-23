//! Exact Top-K of the unconstrained maximizing Power target.
//!
//! Pools carry per-unit power profiles for every member key, so decks that
//! share a unit, an attribute, both, or two units at once (Virtual Singer
//! cards with the same support unit) resolve different powers. Some cards are
//! cultivation variants that share a public game id.
use super::*;

const PIAPRO: u8 = 1 << 5;

#[derive(Clone, Copy)]
struct PowerCard {
    char_id: u8,
    attr: u8,
    unit_mask: u8,
    game_id: u16,
    /// Power by profile, then member key `unit_all * 2 + attr_all`.
    profiles: [[u32; 4]; 2],
    /// Bit `u` set: unit `u` resolves through the second profile.
    second_profile_units: u8,
}

fn power_pool(cards: &[PowerCard]) -> CardPool {
    let mut builder = PoolBuilder::new(cards.len() as u16);
    for (index, card) in cards.iter().enumerate() {
        let dense = index as u16;
        let mut values = [0u16; 8];
        let mut lut = u32::from(card.second_profile_units) << 16;
        for (slot, value) in values.iter_mut().enumerate() {
            let power = card.profiles[slot / 4][slot % 4];
            *value = power as u16;
            lut |= ((power >> 16) & 3) << (slot * 2);
        }
        builder.set_power_values(dense, values);
        builder.set_power_lut(dense, lut);
        builder.set_power_max(
            dense,
            card.profiles.iter().flatten().copied().max().unwrap(),
        );
        builder.set_skill(
            dense,
            SkillSlot {
                skill_type: 0,
                value: 100,
            },
        );
        builder.set_skill_min(dense, 100);
        builder.set_skill_max(dense, 100);
        builder.set_skill_reference(dense, 100);
        builder.set_event_bonus(dense, EventBonusExact::from_whole(0, 0));
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

fn top_k_params(top_k: usize) -> SearchParams {
    SearchParams {
        top_k,
        timeout_ms: 0,
    }
}

/// Five Virtual Singer cards that all carry the piapro unit and support unit
/// 0 share two units. Two of them gain more from the piapro unit, three from
/// unit 0, so the deck of all five resolves 1000 while no scenario that shares
/// a single unit values it above 800. Decks of the single-unit scenarios reach
/// 900 and 850.
#[test]
fn a_deck_sharing_two_units_is_ranked_by_its_own_power() {
    let virtual_singer = |char_id: u8, piapro_first: bool| PowerCard {
        char_id,
        attr: char_id % 5,
        unit_mask: PIAPRO | 1,
        game_id: 800 + u16::from(char_id),
        profiles: if piapro_first {
            [[50, 50, 100, 100], [50, 50, 200, 200]]
        } else {
            [[50, 50, 200, 200], [50, 50, 100, 100]]
        },
        second_profile_units: PIAPRO,
    };
    let single = |char_id: u8, unit_mask: u8, shared: u32| PowerCard {
        char_id,
        attr: char_id % 5,
        unit_mask,
        game_id: 800 + u16::from(char_id),
        profiles: [[50, 50, shared, shared]; 2],
        second_profile_units: 0,
    };
    let cards = [
        virtual_singer(21, true),
        virtual_singer(22, true),
        virtual_singer(23, false),
        virtual_singer(24, false),
        virtual_singer(25, false),
        single(1, 1, 200),
        single(26, PIAPRO, 250),
    ];
    let pool = power_pool(&cards);
    let context = ready_ctx(&pool, ScoreTarget::Power);
    for top_k in [1, 3] {
        let params = top_k_params(top_k);
        let (expected, _) = ExactOracle::new(&pool, &context).search(&params);
        assert_eq!(expected[0].score, 1_000);
        assert_eq!(
            search_exact(&pool, &context, &params),
            expected,
            "top_k={top_k}"
        );
    }
}

/// Three members carry units 0 and 1, two carry units 0 and 2, so the deck
/// of all five shares exactly unit 0, which is no card's own mask.
#[test]
fn a_shared_unit_set_that_is_no_card_mask_is_searched() {
    let card = |char_id: u8, unit_mask: u8, neither: u32, shared: u32| PowerCard {
        char_id,
        attr: char_id % 5,
        unit_mask,
        game_id: 850 + u16::from(char_id),
        profiles: [[neither, neither, shared, shared]; 2],
        second_profile_units: 0,
    };
    let cards = [
        card(1, 0b011, 100, 300),
        card(2, 0b011, 100, 300),
        card(3, 0b011, 100, 300),
        card(4, 0b101, 100, 300),
        card(5, 0b101, 100, 300),
        card(6, 0b1000, 250, 250),
    ];
    let pool = power_pool(&cards);
    let context = ready_ctx(&pool, ScoreTarget::Power);
    for top_k in [1, 2] {
        let params = top_k_params(top_k);
        let (expected, _) = ExactOracle::new(&pool, &context).search(&params);
        assert_eq!(expected[0].score, 1_500);
        assert_eq!(
            search_exact(&pool, &context, &params),
            expected,
            "top_k={top_k}"
        );
    }
}

/// Most decks of eight characters with equal card powers tie on the
/// objective, and public ids run against pool order, so public sets decide
/// the Top-K; a power cap below the best total makes more decks tie.
#[test]
fn tied_decks_are_ranked_by_public_set() {
    let cards: Vec<_> = (0..24u16)
        .map(|index| {
            let char_id = (index % 8) as u8 + 1;
            let power = if index % 7 == 3 { 1_100 } else { 1_000 };
            PowerCard {
                char_id,
                attr: char_id % 5,
                unit_mask: 1 << (char_id % 3),
                game_id: 100 + (index * 7) % 24,
                profiles: [[power; 4]; 2],
                second_profile_units: 0,
            }
        })
        .collect();
    let pool = power_pool(&cards);
    for cap in [None, Some(5_100)] {
        let mut context = ready_ctx(&pool, ScoreTarget::Power);
        context.power_total_cap = cap;
        for top_k in [1, 5, 30, 100] {
            let params = top_k_params(top_k);
            let (expected, _) = ExactOracle::new(&pool, &context).search(&params);
            assert_eq!(
                search_exact(&pool, &context, &params),
                expected,
                "cap={cap:?} top_k={top_k}"
            );
        }
    }
}

fn random_profile(rng: &mut ExactLcg, base: u32) -> [u32; 4] {
    let attr = rng.range(0, 4) * 997;
    let unit = rng.range(0, 4) * 1_231;
    let both = attr + unit + rng.range(0, 3) * 311;
    let profile = [base, base + attr, base + unit, base + both];
    if rng.range(0, 6) == 0 {
        // Member keys need not be ordered.
        [profile[3], profile[1], profile[0], profile[2]]
    } else {
        profile
    }
}

/// Characters `1..=6` carry one of units `0..=2`; `21..=23` are Virtual
/// Singers, alone or with support unit 0 or 1. Attributes come from `0..=1`
/// so decks share units and attributes together.
fn random_power_cards(rng: &mut ExactLcg, count: usize) -> Vec<PowerCard> {
    const CHARACTERS: [u8; 9] = [1, 2, 3, 4, 5, 6, 21, 22, 23];
    let mut cards: Vec<PowerCard> = Vec::with_capacity(count);
    for index in 0..count {
        let base = 40_000 + rng.range(0, 40) * 1_009 + if index % 4 == 0 { 70_000 } else { 0 };
        let profiles = [random_profile(rng, base), random_profile(rng, base)];
        let card = if index >= CHARACTERS.len() && rng.range(0, 3) == 0 {
            // A cultivation variant: same public card, other powers or the
            // same ones.
            let original = cards[rng.range(0, index as u32) as usize];
            if rng.range(0, 2) == 0 {
                original
            } else {
                PowerCard {
                    profiles,
                    ..original
                }
            }
        } else {
            let char_id = if index < CHARACTERS.len() {
                CHARACTERS[index]
            } else {
                CHARACTERS[rng.range(0, CHARACTERS.len() as u32) as usize]
            };
            let unit_mask = if char_id >= 21 {
                [PIAPRO, PIAPRO | 1, PIAPRO | 2][rng.range(0, 3) as usize]
            } else {
                1 << ((char_id - 1) % 3)
            };
            PowerCard {
                char_id,
                attr: rng.range(0, 2) as u8,
                unit_mask,
                game_id: 900 + index as u16,
                profiles,
                second_profile_units: if rng.range(0, 2) == 0 { PIAPRO } else { 0 },
            }
        };
        cards.push(card);
    }
    cards
}

#[test]
fn power_top_k_matches_oracle_across_scenarios() {
    let mut rng = ExactLcg(0x90e2_7a11);
    let mut compared = 0usize;
    for round in 0..24 {
        let cards = random_power_cards(&mut rng, 10 + round % 3);
        let pool = power_pool(&cards);
        let mut context = ready_ctx(&pool, ScoreTarget::Power);
        context.live_type = [LiveType::Solo, LiveType::Multi, LiveType::Auto][round % 3];
        context.honor_bonus = [0, 1_234, 7_777][rng.range(0, 3) as usize];
        if round % 4 == 3 {
            // Every card has the same bonus, so MySekai ranks decks by power.
            context.target = ScoreTarget::Mysekai;
            context.live_type = LiveType::Mysekai;
        }
        // The canonical order is total, so every smaller Top-K is a prefix.
        let (expected, _) = ExactOracle::new(&pool, &context).search(&top_k_params(100));
        assert!(
            expected.len() >= 30,
            "round={round}: {} decks",
            expected.len()
        );
        for top_k in [1, 5, 30, 100] {
            assert_eq!(
                search_exact(&pool, &context, &top_k_params(top_k)),
                expected[..top_k.min(expected.len())],
                "round={round} target={:?} top_k={top_k}",
                context.target
            );
            compared += 1;
        }
    }
    assert_eq!(compared, 96);
}
