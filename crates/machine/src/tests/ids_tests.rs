use super::*;

#[test]
fn id_segment_validation() {
    assert!(is_valid_id_segment("a"));
    assert!(is_valid_id_segment("a0"));
    assert!(is_valid_id_segment("a_0b"));
    assert!(!is_valid_id_segment(""));
    assert!(!is_valid_id_segment("_a"));
    assert!(!is_valid_id_segment("A"));
    assert!(!is_valid_id_segment("a-1"));
    assert!(!is_valid_id_segment("0a"));
    assert!(!is_valid_id_segment(&"a".repeat(64)));
}

#[test]
fn op_path_shape_and_segments() {
    assert!(validate_op_path("machine.main"));
    assert!(!validate_op_path("m0._"));
    assert!(!validate_op_path("machine"));
    assert!(!validate_op_path("machine.main.extra"));
    assert!(!validate_op_path("Machine.main"));
    assert!(!validate_op_path("machine.ma-in"));
}

#[test]
fn state_id_shape_and_segments() {
    assert!(validate_state_id("machine.main.setup"));
    assert!(!validate_state_id("machine.main"));
    assert!(!validate_state_id("machine.main.setup.extra"));
    assert!(!validate_state_id("machine.Main.setup"));
    assert!(!validate_state_id("machine.main.set-up"));
}

#[test]
fn op_id_constructor_enforces_segment_rules() {
    assert!(OpId::new("portfolio_tracker").is_ok());
    assert!(OpId::new("PortfolioTracker").is_err());
    assert!(OpId::new("portfolio-tracker").is_err());
}

#[test]
fn state_id_constructor_enforces_shape_and_segment_rules() {
    assert!(StateId::new("machine.main.setup").is_ok());
    assert!(StateId::new("machine.main").is_err());
    assert!(StateId::new("machine.main.setup.extra").is_err());
    assert!(StateId::new("machine.main.set-up").is_err());
}

#[test]
fn serde_rejects_invalid_op_and_state_ids() {
    let bad_op_id = serde_json::from_str::<OpId>("\"bad-op\"");
    assert!(bad_op_id.is_err());

    let bad_state_id = serde_json::from_str::<StateId>("\"machine.main.extra.parts\"");
    assert!(bad_state_id.is_err());
}
