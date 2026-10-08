use super::*;
use crate::power::MultiUnitBonusMode;

#[test]
fn optional_shuffle_table_and_finale_chapters_share_the_master_loader() {
    let required = [
        "cards.json",
        "gameCharacterUnits.json",
        "events.json",
        "skills.json",
        "cardRarities.json",
        "cardEpisodes.json",
        "masterLessons.json",
        "areaItemLevels.json",
        "characterRanks.json",
        "cardMysekaiCanvasBonuses.json",
        "eventCards.json",
        "eventDeckBonuses.json",
        "worldBloomDifferentAttributeBonuses.json",
        "eventRarityBonusRates.json",
    ];
    let mut sources = MasterdataSources::from_strings(
        required.into_iter().map(|name| (name.into(), "[]".into())),
        "[]".into(),
    );
    let plain = OwnedGameData::from_sources(&sources).unwrap();
    assert!(plain.event_shuffle_unit_bonuses.is_empty());
    sources.tables.insert(
        "worldBlooms.json".into(),
        serde_json::json!([
            {"eventId":931,"worldBloomChapterType":"finale","chapterNo":1}
        ])
        .to_string(),
    );
    sources.tables.insert(
        "eventShuffleUnitBonuses.json".into(),
        serde_json::json!([
            {"eventId":931,"unitCount":3,"bonusRate":71},
            {"eventId":931,"unitCount":5,"bonusRate":19}
        ])
        .to_string(),
    );
    let loaded = OwnedGameData::from_sources(&sources).unwrap();
    let game = loaded.as_ref();
    assert!(game.is_world_bloom_finale(931));
    assert!(!game.is_world_bloom_finale(932));
    assert_eq!(game.event_shuffle_unit_bonuses[0].bonus_rate, 71);
    assert_eq!(game.event_shuffle_unit_bonuses[1].unit_count, 5);
    let encoded = serde_json::to_value(&loaded).unwrap();
    let roundtrip: OwnedGameData = serde_json::from_value(encoded).unwrap();
    assert_eq!(
        roundtrip.event_shuffle_unit_bonuses,
        loaded.event_shuffle_unit_bonuses
    );
    sources
        .tables
        .insert("eventShuffleUnitBonuses.json".into(), "null".into());
    assert!(
        OwnedGameData::from_sources(&sources)
            .unwrap_err()
            .contains("eventShuffleUnitBonuses.json")
    );
    sources
        .tables
        .insert("eventShuffleUnitBonuses.json".into(), "[]".into());
    sources.tables.insert(
        "worldBlooms.json".into(),
        serde_json::json!([
            {"eventId":931,"worldBloomChapterType":null,"chapterNo":1}
        ])
        .to_string(),
    );
    assert!(
        !OwnedGameData::from_sources(&sources)
            .unwrap()
            .as_ref()
            .is_world_bloom_finale(931)
    );
}

#[test]
fn area_rows_keep_three_dimensions_nullability_and_master_order() {
    let input = serde_json::json!([
        {"areaItemId":56,"level":1,"targetUnit":"multi_unit","targetGameCharacterId":0,
        "power1BonusRate":1.0,"power2BonusRate":2.0,"power3BonusRate":3.0,
        "power1AllMatchBonusRate":5.0,"power2AllMatchBonusRate":null,"power3AllMatchBonusRate":7.0},
        {"areaItemId":56,"level":1,"targetUnit":"any","targetGameCharacterId":0,
        "power1BonusRate":0.1,"power2BonusRate":0.2,"power3BonusRate":0.3,
        "power1AllMatchBonusRate":1.0,"power2AllMatchBonusRate":2.0,"power3AllMatchBonusRate":3.0}
    ]);
    let rows: Vec<RawAreaItemLevel> = serde_json::from_value(input).unwrap();
    let rows = flatten_area_item_levels(rows);
    assert_eq!(rows[0].unit.as_deref(), Some("multi_unit"));
    assert_eq!(rows[0].power_rate, [1.0, 2.0, 3.0]);
    assert_eq!(rows[0].character_id, None);
    assert_eq!(rows[0].power_all_match_rate, None);
    assert_eq!(rows[1].power_all_match_rate, Some([1.0, 2.0, 3.0]));
}

#[test]
fn evaluation_public_spellings_and_invalid_values() {
    for key in ["multi_unit_bonus_evaluation", "multiUnitBonusEvaluation"] {
        for (value, mode) in [
            ("by_deck", MultiUnitBonusMode::ByDeck),
            ("force_on", MultiUnitBonusMode::ForceOn),
            ("force_off", MultiUnitBonusMode::ForceOff),
        ] {
            let input = serde_json::json!({key:value}).to_string();
            assert_eq!(
                parse_build_params_json(&input)
                    .unwrap()
                    .multi_unit_bonus_mode,
                mode
            );
        }
        for value in [
            serde_json::json!("sometimes"),
            serde_json::json!(true),
            serde_json::json!(7),
        ] {
            assert!(parse_build_params_json(&serde_json::json!({key:value}).to_string()).is_err());
        }
    }
    assert_eq!(
        parse_build_params_json("{}").unwrap().multi_unit_bonus_mode,
        MultiUnitBonusMode::ByDeck
    );
}
