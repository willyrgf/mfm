use serde_json::{json, Map, Value};

use super::*;

fn valid_envelope() -> Value {
    let contract = default_evidence_contract();
    json!({
        "domain": RESTORE_DOMAIN,
        "version": RESTORE_VERSION,
        "instance_identity": [{
            "singleton": true,
            "lineage_id": "lineage:codec-test",
            "instance_id": "instance:codec-test",
            "executor_binding_ref": "executor-binding:codec-test",
            "ledger_generation": 1
        }],
        "evidence_contracts": [{
            "contract_ref": contract.contract_ref,
            "max_attempts": contract.max_attempts,
            "max_records": contract.max_records,
            "max_retained_bytes": contract.max_retained_bytes,
            "completion_reserve_records": contract.completion_reserve_records,
            "completion_reserve_bytes": contract.completion_reserve_bytes
        }],
        "evidence_objects": [],
        "effect_records": [],
        "resource_configurations": [],
        "resource_records": []
    })
}

fn canonical(value: &Value) -> Vec<u8> {
    canonical_encode(value).expect("encode canonical restore fixture")
}

fn envelope_object(value: &mut Value) -> &mut Map<String, Value> {
    value.as_object_mut().expect("restore fixture object")
}

fn identity_object(value: &mut Value) -> &mut Map<String, Value> {
    envelope_object(value)
        .get_mut("instance_identity")
        .and_then(Value::as_array_mut)
        .and_then(|rows| rows.first_mut())
        .and_then(Value::as_object_mut)
        .expect("restore identity fixture")
}

fn bound_effect_row() -> Value {
    let digest = format!("\\x{}", "00".repeat(32));
    json!({
        "stream_id": "effect:codec-bound",
        "sequence": 1,
        "predecessor_digest": null,
        "opaque_payload": "\\x00",
        "executor_binding_ref": "executor-binding:codec-test",
        "ledger_generation": 1,
        "effect_key": digest,
        "request_digest": digest,
        "destination_identity": "destination:codec-test",
        "append_request_id": null,
        "append_predecessor_sequence": null,
        "append_predecessor_digest": null,
        "append_purpose": null,
        "append_candidate_digest": null,
        "record_kind": "effect_bound",
        "resource_ownership_ref": null,
        "policy_ref": null,
        "policy_configuration_ref": null,
        "resource_key_ref": null,
        "allocation_state_ref": null,
        "allocation_value": null,
        "fencing_ref": null,
        "resource_append_request_id": null,
        "resource_record_ref": null,
        "attempt_ordinal": null,
        "attempt_id": null,
        "target_operation_ref": null,
        "observation_outcome": null,
        "safe_outcome_ref": null,
        "destination_account": null,
        "terminal_attempt_id": null,
        "external_operation_ref": null,
        "terminal_outcome_ref": null,
        "terminal_proof_ref": null,
        "evidence_object_digest": digest,
        "retained_bytes": 1,
        "commit_digest": digest,
        "proof_digest": digest
    })
}

fn assert_rejected(value: &Value, label: &str) {
    assert!(
        decode_authority(&canonical(value)).is_err(),
        "{label} restore envelope unexpectedly decoded"
    );
}

#[test]
fn strict_restore_codec_rejects_unknown_shape_domain_and_version() {
    decode_authority(&canonical(&valid_envelope())).expect("valid envelope");

    let mut unknown_envelope = valid_envelope();
    envelope_object(&mut unknown_envelope).insert("future_authority".to_owned(), json!([]));
    assert_rejected(&unknown_envelope, "unknown envelope field");

    let mut unknown_row = valid_envelope();
    identity_object(&mut unknown_row).insert("future_identity".to_owned(), json!(true));
    assert_rejected(&unknown_row, "unknown row field");

    let mut wrong_domain = valid_envelope();
    envelope_object(&mut wrong_domain).insert("domain".to_owned(), json!("future.domain"));
    assert_rejected(&wrong_domain, "wrong domain");

    let mut wrong_version = valid_envelope();
    envelope_object(&mut wrong_version).insert("version".to_owned(), json!(2));
    assert_rejected(&wrong_version, "wrong version");
}

#[test]
fn strict_restore_codec_closes_bound_append_columns_and_completion_reserve() {
    let mut round_trip = valid_envelope();
    envelope_object(&mut round_trip)
        .insert("effect_records".to_owned(), json!([bound_effect_row()]));
    decode_authority(&canonical(&round_trip)).expect("closed bound append columns round-trip");

    let mut spurious_append_sibling = round_trip;
    envelope_object(&mut spurious_append_sibling)
        .get_mut("effect_records")
        .and_then(Value::as_array_mut)
        .and_then(|records| records.first_mut())
        .and_then(Value::as_object_mut)
        .expect("bound effect row")
        .insert("append_purpose".to_owned(), json!("spurious-purpose"));
    assert_rejected(
        &spurious_append_sibling,
        "spurious append sibling under null request id",
    );

    let mut oversized_completion_reserve = valid_envelope();
    envelope_object(&mut oversized_completion_reserve)
        .get_mut("evidence_contracts")
        .and_then(Value::as_array_mut)
        .and_then(|contracts| contracts.first_mut())
        .and_then(Value::as_object_mut)
        .expect("evidence contract")
        .insert("completion_reserve_records".to_owned(), json!(3));
    assert_rejected(
        &oversized_completion_reserve,
        "non-exact completion reserve",
    );
}

#[test]
fn strict_restore_codec_rejects_open_tags_and_uppercase_bytea() {
    let mut tag_sets = Map::new();
    tag_sets.insert(
        "effect_records".to_owned(),
        json!([{"record_kind": "future_effect_record", "observation_outcome": null}]),
    );
    tag_sets.insert("resource_configurations".to_owned(), json!([]));
    tag_sets.insert("resource_records".to_owned(), json!([]));
    assert!(
        validate_closed_tags(&tag_sets).is_err(),
        "future effect-record tag unexpectedly accepted"
    );

    tag_sets.insert(
        "effect_records".to_owned(),
        json!([{"record_kind": "effect_bound", "observation_outcome": "future_outcome"}]),
    );
    assert!(
        validate_closed_tags(&tag_sets).is_err(),
        "future observation tag unexpectedly accepted"
    );

    let mut open_resource_tag = valid_envelope();
    envelope_object(&mut open_resource_tag).insert(
        "resource_configurations".to_owned(),
        json!([{
            "resource_ownership_ref": "resource-owner:codec-test",
            "resource_key_ref": "resource-key:codec-test",
            "policy_ref": "resource-policy:codec-test",
            "policy_configuration_kind": "future_resource_policy",
            "finite_allocation_values": null,
            "account_sequence_initial_value": null,
            "policy_configuration_ref": format!("\\x{}", "00".repeat(32))
        }]),
    );
    assert_rejected(&open_resource_tag, "future resource-policy tag");

    let mut uppercase_bytea = valid_envelope();
    envelope_object(&mut uppercase_bytea).insert(
        "evidence_objects".to_owned(),
        json!([{
            "object_digest": format!("\\x{}", "AA".repeat(32)),
            "opaque_bytes": "\\x00"
        }]),
    );
    assert_rejected(&uppercase_bytea, "uppercase bytea");
}

#[test]
fn strict_restore_codec_rejects_noncanonical_malformed_and_bounded_input() {
    let canonical = canonical(&valid_envelope());
    let mut noncanonical = vec![b' '];
    noncanonical.extend_from_slice(&canonical);
    assert!(
        decode_authority(&noncanonical).is_err(),
        "noncanonical whitespace unexpectedly accepted"
    );
    assert!(
        decode_authority(b"{").is_err(),
        "malformed JSON unexpectedly accepted"
    );

    let mut overlong_reference = valid_envelope();
    identity_object(&mut overlong_reference).insert(
        "lineage_id".to_owned(),
        Value::String("x".repeat(MAX_REFERENCE_BYTES + 1)),
    );
    assert_rejected(&overlong_reference, "overlong reference");

    let too_many = Value::Array(vec![Value::Null; MAX_ROWS_PER_SET + 1]);
    assert!(
        validate_json_bounds(&too_many, 0).is_err(),
        "oversized row set unexpectedly accepted"
    );
}
