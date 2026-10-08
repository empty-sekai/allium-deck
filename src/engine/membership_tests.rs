use super::parse_build_params_json;
use crate::handler::FixedConstraintMode;

#[test]
fn fixed_constraint_mode_is_explicit_and_defaults_to_slots() {
    assert_eq!(
        parse_build_params_json("{}").unwrap().fixed_constraint_mode,
        FixedConstraintMode::Slots
    );
    for key in ["fixedConstraintMode", "fixed_constraint_mode"] {
        let request = format!(r#"{{"{key}":"members"}}"#);
        assert_eq!(
            parse_build_params_json(&request)
                .unwrap()
                .fixed_constraint_mode,
            FixedConstraintMode::Members
        );
        for invalid in [r#""other""#, "true", "12"] {
            assert!(parse_build_params_json(&format!(r#"{{"{key}":{invalid}}}"#)).is_err());
        }
    }
}
