use super::*;

fn row(unit: u8, normal: [f64; 3], all: Option<[f64; 3]>) -> PowerAreaItem {
    PowerAreaItem {
        unit,
        attr: PowerAreaItem::ANY,
        character_id: PowerAreaItem::ANY_CHARACTER,
        power_rate: normal,
        power_all_match_rate: all,
    }
}

#[test]
fn multi_tied_extra_keeps_unit_rates_and_floors_each_dimension() {
    let rows = [
        row(0, [1.0; 3], Some([6.0, 2.0, 1.0])),
        row(PowerAreaItem::MULTI, [0.0, 0.0, 6.0], None),
    ];
    // Extra rates sum to six for either choice; unit wins the tie.
    // 101*6% -> 6, 203*2% -> 4, 307*1% -> 3.
    assert_eq!(multi_area_bonus(&rows, [101, 203, 307], 1, 1, 0, 4), 13);
    let rows = [rows[0], row(PowerAreaItem::MULTI, [0.0, 0.0, 7.0], None)];
    // Multi wins, so the unit row reverts to [1,1,1].
    assert_eq!(
        multi_area_bonus(&rows, [101, 203, 307], 1, 1, 0, 4),
        1 + 2 + 24
    );
}

#[test]
fn multi_nullable_all_match_uses_the_whole_normal_row() {
    let rows = [
        row(0, [1.0, 2.0, 3.0], None),
        row(PowerAreaItem::MULTI, [1.0; 3], None),
    ];
    assert_eq!(
        multi_area_bonus(&rows, [101, 203, 307], 1, 1, 0, 4),
        2 + 6 + 12
    );
}

#[test]
fn multi_unit_support_decision_is_f32_with_original_winning_ties() {
    let rows = [
        row(5, [2.0; 3], None),
        row(0, [2.00000005; 3], None),
        row(PowerAreaItem::MULTI, [0.0; 3], None),
    ];
    assert_eq!(
        multi_area_bonus(&rows, [2_000_000_000, 0, 0], 21, 33, 0, 0),
        40_000_000
    );
    let rows = [
        rows[0],
        row(0, [3.0; 3], None),
        row(PowerAreaItem::MULTI, [1.0; 3], None),
    ];
    assert_eq!(multi_area_bonus(&rows, [100, 200, 300], 21, 33, 0, 0), 24);
}

#[test]
fn multi_bucket_priority_is_character_then_unit_then_attribute() {
    let mut character = row(PowerAreaItem::MULTI, [1.0; 3], Some([99.0; 3]));
    character.character_id = 21;
    character.attr = 4;
    let mut unit = row(5, [2.0; 3], Some([4.0; 3]));
    unit.attr = 4;
    let rows = [character, unit];
    // Character and unit rows do not additionally require their attribute.
    assert_eq!(multi_area_bonus(&rows, [100; 3], 21, 32, 0, 4), 15);
}

#[test]
fn multi_accumulation_preserves_original_row_order() {
    let tiny = 5.684341886080801e-13;
    let rows = [
        row(PowerAreaItem::MULTI, [9999.999999999998, 0.0, 0.0], None),
        row(PowerAreaItem::ANY, [tiny, 0.0, 0.0], None),
        row(PowerAreaItem::ANY, [tiny, 0.0, 0.0], None),
    ];
    // Adding each tiny term after the large term loses it. Adding the two
    // tiny terms first crosses the integer boundary after the large term.
    assert_eq!(multi_area_bonus(&rows, [1, 0, 0], 1, 1, 0, 0), 99);
    let reordered = [rows[1], rows[2], rows[0]];
    assert_eq!(multi_area_bonus(&reordered, [1, 0, 0], 1, 1, 0, 0), 100);
}
