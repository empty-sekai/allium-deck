pub(crate) fn calc_mysekai_points(total_power: i32, total_bonus: f64) -> (i32, i32) {
    // internal uses as-i32 truncation (not round/floor) per game client behavior.
    let power_bonus = ((1.0 + total_power as f64 / 450_000.0) * 10.0 + 1e-6).floor() / 10.0;
    let event_bonus = (total_bonus + 1e-6).floor() / 100.0;
    let segmented = ((power_bonus * (1.0 + event_bonus)) + 1e-6).floor() as i32 * 500;
    let internal = (power_bonus * (1.0 + event_bonus) * 500.0) as i32;
    (segmented, internal)
}
