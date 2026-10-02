use super::*;
use crate::power::MultiUnitBonusMode;

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
