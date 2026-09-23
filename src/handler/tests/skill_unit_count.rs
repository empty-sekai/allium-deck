//! Composition-dependent skills resolved inside a concrete deck.
//!
//! Expected values are computed by hand from the game rules:
//! - a different-unit skill (`score_up_unit_count`) adds the value of the
//!   largest matched row, where the count is the number of distinct units
//!   among the other members that differ from the card's own unit, and a
//!   Virtual Singer card with a support unit counts as that unit;
//! - a reference skill (`other_member_score_up_reference_rate`) adds
//!   `min(target * rate / 100, max)`, unrounded, where `target` is the static
//!   maximum of the referenced member's skill at its skill level.

use serde_json::{Value, json};

use crate::engine::{MasterdataSources, OwnedGameData};
use crate::handler::types::{BuildParams, UserCard, UserProfile};
use crate::pool::{CardIdx, CardPool};
use crate::search::{SearchContext, summarize_deck};
use crate::types::{ScoreTarget, SkillReferenceStrategy};

const PLAIN_100: i32 = 1;
const PLAIN_105: i32 = 2;
const SAME_UNIT_ENHANCE: i32 = 15;
const CHARACTER_RANK: i32 = 22;
const REFERENCE: i32 = 23;
const DIFFERENT_UNIT: i32 = 24;
const WIDE_REFERENCE: i32 = 30;

fn detail(value: i32) -> Value {
    json!([{ "level": 1, "activateEffectValue": value }])
}

fn skills() -> Value {
    json!([
        { "id": PLAIN_100, "skillEffects": [
            { "skillEffectType": "score_up", "skillEffectDetails": detail(100) } ] },
        { "id": PLAIN_105, "skillEffects": [
            { "skillEffectType": "score_up", "skillEffectDetails": detail(105) } ] },
        { "id": SAME_UNIT_ENHANCE, "skillEffects": [
            { "skillEffectType": "score_up", "skillEffectDetails": detail(80),
              "skillEnhance": { "activateEffectValue": 10,
                                "skillEnhanceCondition": { "unit": "light_sound" } } } ] },
        { "id": CHARACTER_RANK, "skillEffects": [
            { "skillEffectType": "score_up", "skillEffectDetails": detail(90) },
            { "skillEffectType": "score_up_character_rank", "activateCharacterRank": 2,
              "skillEffectDetails": detail(1) },
            { "skillEffectType": "score_up_character_rank", "activateCharacterRank": 100,
              "skillEffectDetails": detail(50) } ] },
        { "id": REFERENCE, "skillEffects": [
            { "skillEffectType": "score_up", "skillEffectDetails": detail(60) },
            { "skillEffectType": "other_member_score_up_reference_rate",
              "skillEffectDetails": [{ "level": 1, "activateEffectValue": 50,
                                       "activateEffectValue2": 60 }] } ] },
        { "id": DIFFERENT_UNIT, "skillEffects": [
            { "skillEffectType": "score_up", "skillEffectDetails": detail(70) },
            { "skillEffectType": "score_up_unit_count", "activateUnitCount": 1,
              "conditionType": "equals_or_over", "skillEffectDetails": detail(30) },
            { "skillEffectType": "score_up_unit_count", "activateUnitCount": 2,
              "conditionType": "equals_or_over", "skillEffectDetails": detail(60) } ] },
        { "id": WIDE_REFERENCE, "skillEffects": [
            { "skillEffectType": "score_up", "skillEffectDetails": detail(60) },
            { "skillEffectType": "other_member_score_up_reference_rate",
              "skillEffectDetails": [{ "level": 1, "activateEffectValue": 50,
                                       "activateEffectValue2": 100 }] } ] }
    ])
}

/// `(card id, character id, support unit, skill id, after-training skill id)`.
const CARDS: &[(i32, i32, &str, i32, Option<i32>)] = &[
    // Virtual Singers without a support unit.
    (100, 21, "none", DIFFERENT_UNIT, None),
    (101, 22, "none", PLAIN_100, None),
    (102, 23, "none", PLAIN_100, None),
    (103, 24, "none", PLAIN_100, None),
    (104, 26, "none", PLAIN_100, None),
    // light_sound, idol, street members.
    (110, 1, "none", PLAIN_100, None),
    (111, 2, "none", PLAIN_100, None),
    (112, 3, "none", PLAIN_100, None),
    (113, 4, "none", PLAIN_100, None),
    (120, 5, "none", PLAIN_100, None),
    (121, 6, "none", PLAIN_100, None),
    (130, 9, "none", PLAIN_100, None),
    // Virtual Singers with a support unit.
    (140, 22, "light_sound", PLAIN_100, None),
    (141, 23, "idol", PLAIN_100, None),
    // Different-unit skill on a supported Virtual Singer and on a member.
    (150, 24, "light_sound", DIFFERENT_UNIT, None),
    (151, 1, "none", DIFFERENT_UNIT, None),
    // Reference targets with distinct static maxima.
    (160, 7, "none", PLAIN_105, None),
    (170, 10, "none", CHARACTER_RANK, None),
    (180, 26, "light_sound", SAME_UNIT_ENHANCE, None),
    // Cards whose skill follows the art state.
    (200, 25, "none", DIFFERENT_UNIT, Some(CHARACTER_RANK)),
    (300, 17, "none", REFERENCE, Some(CHARACTER_RANK)),
    (310, 18, "none", WIDE_REFERENCE, None),
];

fn game() -> OwnedGameData {
    let units = (1..=26)
        .map(|id| {
            let unit = match id {
                1..=4 => "light_sound",
                5..=8 => "idol",
                9..=12 => "street",
                13..=16 => "theme_park",
                17..=20 => "school_refusal",
                _ => "piapro",
            };
            json!({ "id": id, "gameCharacterId": id, "unit": unit })
        })
        .collect::<Vec<_>>();
    let cards = CARDS
        .iter()
        .map(|&(id, character, support, skill, after)| {
            let mut card = json!({
                "id": id,
                "characterId": character,
                "cardRarityType": "rarity_4",
                "attr": "cool",
                "supportUnit": support,
                "skillId": skill,
                "cardParameters": { "param1": [1000], "param2": [1000], "param3": [1000] },
            });
            if let Some(after) = after {
                card["specialTrainingSkillId"] = json!(after);
            }
            card
        })
        .collect::<Vec<_>>();
    let mut tables = vec![
        ("cards.json", json!(cards)),
        ("gameCharacterUnits.json", json!(units)),
        ("skills.json", skills()),
        (
            "cardRarities.json",
            json!([{ "cardRarityType": "rarity_4", "maxLevel": 50,
                     "trainingMaxLevel": 60, "maxSkillLevel": 4 }]),
        ),
    ];
    for empty in [
        "events.json",
        "areaItemLevels.json",
        "cardEpisodes.json",
        "cardMysekaiCanvasBonuses.json",
        "characterRanks.json",
        "eventCards.json",
        "eventDeckBonuses.json",
        "eventRarityBonusRates.json",
        "masterLessons.json",
        "worldBloomDifferentAttributeBonuses.json",
    ] {
        tables.push((empty, json!([])));
    }
    let sources = MasterdataSources::from_strings(
        tables
            .into_iter()
            .map(|(name, value)| (name.to_string(), value.to_string())),
        "[]".to_string(),
    );
    OwnedGameData::from_sources(&sources).expect("fixture masterdata")
}

fn user(special_image: &[i32]) -> UserProfile {
    UserProfile {
        user_cards: CARDS
            .iter()
            .map(|&(card_id, _, _, _, after)| {
                let trained = after.is_some();
                UserCard {
                    card_id,
                    level: 1,
                    skill_level: 1,
                    master_rank: 0,
                    special_training_status: if trained { "done" } else { "none" }.to_string(),
                    default_image: if special_image.contains(&card_id) {
                        "special_training"
                    } else {
                        "original"
                    }
                    .to_string(),
                    episodes_read: Vec::new(),
                    is_virtual: false,
                    has_canvas_bonus_override: None,
                }
            })
            .collect(),
        ..UserProfile::default()
    }
}

struct Built {
    pool: CardPool,
    ctx: SearchContext,
}

fn build(keep_art_state: bool, special_image: &[i32], strategy: SkillReferenceStrategy) -> Built {
    let owned = game();
    let params = BuildParams {
        target: ScoreTarget::Skill,
        keep_after_training_state: keep_art_state,
        skill_reference_strategy: strategy,
        ..BuildParams::default()
    };
    let (pool, ctx) =
        crate::handler::build_card_pool(&user(special_image), &owned.as_ref(), &params)
            .expect("fixture pool");
    Built { pool, ctx }
}

impl Built {
    /// Dense entries of one public card, in dense order.
    fn entries(&self, card_id: i32) -> Vec<CardIdx> {
        self.pool
            .indices()
            .filter(|&idx| i32::from(self.pool.game_id(idx)) == card_id)
            .collect()
    }

    /// The single dense entry of a card that has one skill state.
    fn card(&self, card_id: i32) -> CardIdx {
        let entries = self.entries(card_id);
        assert_eq!(entries.len(), 1, "card {card_id} entries");
        entries[0]
    }

    /// The dense entry of a card carrying the given skill kind.
    fn card_with_skill(&self, card_id: i32, skill_type: u8) -> CardIdx {
        let entries = self
            .entries(card_id)
            .into_iter()
            .filter(|&idx| self.pool.skill(idx).skill_type == skill_type)
            .collect::<Vec<_>>();
        assert_eq!(entries.len(), 1, "card {card_id} skill type {skill_type}");
        entries[0]
    }

    /// Resolved skill value of `subject` inside the deck.
    fn skill_in(&self, deck: [CardIdx; 5], subject: CardIdx) -> f64 {
        let summary = summarize_deck(&self.pool, &self.ctx, &deck).expect("deck summary");
        summary
            .ordered_cards
            .iter()
            .zip(summary.card_skill_score_up)
            .find(|(card, _)| **card == subject)
            .map(|(_, value)| value)
            .expect("subject in deck")
    }

    fn deck(&self, ids: [i32; 5]) -> [CardIdx; 5] {
        ids.map(|id| self.card(id))
    }
}

fn assert_close(actual: f64, expected: f64, label: &str) {
    assert!(
        (actual - expected).abs() < 1e-9,
        "{label}: expected {expected}, resolved {actual}"
    );
}

fn assert_skill(built: &Built, ids: [i32; 5], expected: f64) {
    let deck = built.deck(ids);
    assert_close(
        built.skill_in(deck, deck[0]),
        expected,
        &format!("deck {ids:?}"),
    );
}

#[test]
fn different_unit_skill_counts_distinct_other_units() {
    let built = build(false, &[], SkillReferenceStrategy::Average);
    // Base 70; +30 for one counted unit, +60 for two or more.
    assert_skill(&built, [100, 101, 102, 103, 104], 70.0);
    assert_skill(&built, [100, 110, 111, 112, 113], 100.0);
    assert_skill(&built, [100, 110, 111, 120, 121], 130.0);
    assert_skill(&built, [100, 110, 120, 130, 101], 130.0);
}

#[test]
fn different_unit_skill_counts_support_units_and_repeats_once() {
    let built = build(false, &[], SkillReferenceStrategy::Average);
    // A supported Virtual Singer counts as its support unit.
    assert_skill(&built, [100, 140, 141, 101, 102], 130.0);
    // Repeated units, including a supported Virtual Singer, count once.
    assert_skill(&built, [100, 110, 111, 112, 140], 100.0);
}

#[test]
fn different_unit_skill_excludes_the_cards_own_unit() {
    let built = build(false, &[], SkillReferenceStrategy::Average);
    // Supported Virtual Singer: its own unit is the support unit.
    assert_skill(&built, [150, 110, 111, 112, 113], 70.0);
    assert_skill(&built, [150, 120, 110, 111, 112], 100.0);
    assert_skill(&built, [150, 101, 102, 103, 104], 100.0);
    // Unit member: supported Virtual Singers of the same unit are excluded,
    // Virtual Singers without a support unit count as another unit.
    assert_skill(&built, [151, 140, 110, 111, 112], 70.0);
    assert_skill(&built, [151, 101, 120, 121, 110], 130.0);
}

#[test]
fn reference_targets_use_the_static_skill_maximum() {
    let built = build(false, &[], SkillReferenceStrategy::Average);
    let reference = |id| built.pool.skill_reference(built.card(id));
    assert_eq!(reference(110), 100);
    assert_eq!(reference(160), 105);
    // The highest character-rank row applies, not the owner's rank.
    assert_eq!(reference(170), 140);
    // Every counted different unit applies.
    assert_eq!(reference(100), 130);
    // The full same-unit enhancement applies.
    assert_eq!(reference(180), 130);
    // A reference skill contributes its largest addition.
    assert_eq!(reference(310), 160);
}

#[test]
fn reference_skill_adds_an_unrounded_share_of_static_maxima() {
    // Shares of 50%: 100 -> 50, 105 -> 52.5, 140 -> 70, 130 -> 65 (all below 100).
    for (strategy, share) in [
        (
            SkillReferenceStrategy::Average,
            (50.0 + 52.5 + 70.0 + 65.0) / 4.0,
        ),
        (SkillReferenceStrategy::Max, 70.0),
        (SkillReferenceStrategy::Min, 50.0),
    ] {
        let built = build(false, &[], strategy);
        assert_skill(&built, [310, 110, 160, 170, 100], 60.0 + share);
    }
}

#[test]
fn reference_share_is_capped_by_the_skill_maximum_addition() {
    let built = build(false, &[], SkillReferenceStrategy::Max);
    // 140 * 50% = 70 exceeds the 60 cap of the reference skill.
    let deck = [
        built.card_with_skill(300, 3),
        built.card(110),
        built.card(160),
        built.card(170),
        built.card(100),
    ];
    let value = built.skill_in(deck, deck[0]);
    assert_close(value, 120.0, "capped reference");
}

#[test]
fn kept_original_art_resolves_composition_dependent_skills() {
    let built = build(true, &[], SkillReferenceStrategy::Average);
    let different_unit = built.card(200);
    assert_eq!(built.pool.skill(different_unit).skill_type, 2);
    let deck = [
        different_unit,
        built.card(110),
        built.card(111),
        built.card(120),
        built.card(121),
    ];
    assert_close(
        built.skill_in(deck, different_unit),
        130.0,
        "different-unit",
    );

    let reference = built.card(300);
    assert_eq!(built.pool.skill(reference).skill_type, 3);
    let deck = [
        reference,
        built.card(110),
        built.card(160),
        built.card(170),
        built.card(100),
    ];
    // Average of min(share, 60) over 50, 52.5, 60, 60.
    let expected = 60.0 + (50.0 + 52.5 + 60.0 + 60.0) / 4.0;
    assert_close(built.skill_in(deck, reference), expected, "reference");
}

#[test]
fn kept_special_art_uses_the_after_training_skill() {
    let built = build(true, &[200, 300], SkillReferenceStrategy::Average);
    for id in [200, 300] {
        let card = built.card(id);
        assert_eq!(built.pool.skill(card).skill_type, 0, "card {id}");
        let deck = [
            card,
            built.card(110),
            built.card(111),
            built.card(120),
            built.card(121),
        ];
        // Character rank 0 reaches no rank row.
        assert_close(built.skill_in(deck, card), 90.0, &format!("card {id}"));
    }
}

#[test]
fn unlocked_art_state_offers_both_skills() {
    let built = build(false, &[], SkillReferenceStrategy::Average);
    let different_unit = built.card_with_skill(200, 2);
    let rank = built.card_with_skill(200, 0);
    let members = [
        built.card(110),
        built.card(111),
        built.card(120),
        built.card(121),
    ];
    let deck = [
        different_unit,
        members[0],
        members[1],
        members[2],
        members[3],
    ];
    assert_close(
        built.skill_in(deck, different_unit),
        130.0,
        "different-unit",
    );
    let deck = [rank, members[0], members[1], members[2], members[3]];
    assert_close(built.skill_in(deck, rank), 90.0, "character rank");
}

#[test]
fn compacted_and_restricted_pools_keep_reference_values() {
    let built = build(false, &[], SkillReferenceStrategy::Average);
    let keep = built
        .pool
        .indices()
        .map(|idx| idx.raw() % 2 == 1)
        .collect::<Vec<_>>();
    let kept = built
        .pool
        .indices()
        .filter(|idx| keep[idx.raw()])
        .map(|idx| built.pool.skill_reference(idx))
        .collect::<Vec<_>>();
    assert!(kept.iter().any(|&value| value != 100));
    let power_bound = built
        .pool
        .indices()
        .map(|idx| built.pool.power_max(idx))
        .collect::<Vec<_>>();
    for pool in [
        built.pool.compact(&keep),
        built.pool.restrict(&keep, &power_bound),
    ] {
        let values = pool
            .indices()
            .map(|idx| pool.skill_reference(idx))
            .collect::<Vec<_>>();
        assert_eq!(values, kept);
    }
}
