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
    assert!(validate_op_path("machine.main.extra"));
    assert!(!validate_op_path("m0._"));
    assert!(!validate_op_path("machine"));
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

#[test]
fn artifact_id_constructor_enforces_sha256_lowercase_hex() {
    assert!(ArtifactId::new("0".repeat(64)).is_ok());
    assert!(ArtifactId::new("a".repeat(64)).is_ok());
    assert!(ArtifactId::new("A".repeat(64)).is_err());
    assert!(ArtifactId::new("g".repeat(64)).is_err());
    assert!(ArtifactId::new("0".repeat(63)).is_err());
    assert!(ArtifactId::new("0".repeat(65)).is_err());
}

#[test]
fn artifact_id_serde_rejects_invalid_values() {
    let parsed: ArtifactId = serde_json::from_str(&format!("\"{}\"", "f".repeat(64)))
        .expect("valid lowercase sha256 hex should deserialize");
    assert_eq!(parsed.as_str(), "f".repeat(64));

    let bad_uppercase = serde_json::from_str::<ArtifactId>(&format!("\"{}\"", "F".repeat(64)));
    assert!(bad_uppercase.is_err());

    let bad_shape = serde_json::from_str::<ArtifactId>("\"artifact_123\"");
    assert!(bad_shape.is_err());
}

#[test]
fn error_code_constructor_enforces_persisted_code_shape() {
    assert!(ErrorCode::new("missing_fact_key").is_ok());
    assert!(ErrorCode::new("MissingFactKey").is_err());
    assert!(ErrorCode::new("missing-fact-key").is_err());
    assert!(ErrorCode::new("0missing_fact_key").is_err());
    assert!(ErrorCode::new("a".repeat(64)).is_err());
}

#[test]
fn error_code_serde_rejects_malformed_values() {
    let parsed: ErrorCode = serde_json::from_str("\"missing_fact_key\"")
        .expect("valid error code should deserialize");
    assert_eq!(parsed.as_str(), "missing_fact_key");

    assert!(serde_json::from_str::<ErrorCode>("\"missing-fact-key\"").is_err());
    assert!(serde_json::from_str::<ErrorCode>("\"MissingFactKey\"").is_err());
}

#[test]
fn op_path_parts_are_directly_addressable() {
    let op_path = OpPath::must_new("portfolio_snapshot.fetch_balances.reporting");
    assert_eq!(op_path.machine_and_step(), ("portfolio_snapshot", "fetch_balances"));
    assert_eq!(op_path.child_path(), Some("reporting"));
    assert_eq!(op_path.flattened_child_path(), Some("reporting".to_string()));
}

#[test]
fn state_id_parts_are_directly_addressable() {
    let state_id = StateId::must_new("portfolio_snapshot.fetch_balances.read_eth");
    assert_eq!(state_id.machine(), "portfolio_snapshot");
    assert_eq!(state_id.step(), "fetch_balances");
    assert_eq!(state_id.state_local_id(), "read_eth");
}
