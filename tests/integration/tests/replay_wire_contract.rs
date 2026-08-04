use mfm_app::{PageRequest, DEFAULT_PAGE_LIMIT, MAX_PAGE_LIMIT};
use mfm_ids::{DigestAlgorithm, DigestBytes, RunId};
use mfm_replay::structured::StructuredReplayResult;

#[test]
fn recorded_replay_results_have_the_current_structured_shape() {
    let projection = StructuredReplayResult::strict_decode(&replay_bytes())
        .expect("strict structured replay decode");
    assert_eq!(
        StructuredReplayResult::strict_decode(projection.as_bytes())
            .expect("round-trip structured replay decode")
            .as_bytes(),
        projection.as_bytes(),
    );
}

#[test]
fn structured_replay_decode_rejects_noncanonical_or_float_bytes() {
    for bytes in [
        br#"{ "kind":"verified" }"#.as_slice(),
        br#"{"kind":"verified","ratio":1.5}"#.as_slice(),
    ] {
        assert!(StructuredReplayResult::strict_decode(bytes).is_err());
    }
}

#[test]
fn structured_replay_decode_rejects_canonical_wrong_and_retired_shapes() {
    let run_id = run_id();
    let cases = [
        "null".to_owned(),
        "{}".to_owned(),
        "[]".to_owned(),
        format!(r#"{{"kind":"verified","run_id":"{run_id}"}}"#),
        format!(r#"{{"extra":true,"kind":"verified","run_id":"{run_id}"}}"#),
        format!(r#"{{"kind":"verified","status":"unknown","run_id":"{run_id}"}}"#),
        format!(r#"{{"kind":"legacy_replay_result","run_id":"{run_id}"}}"#),
    ];

    for bytes in cases {
        assert!(
            StructuredReplayResult::strict_decode(bytes.as_bytes()).is_err(),
            "canonical hostile shape passed: {bytes}"
        );
    }
}

fn replay_bytes() -> Vec<u8> {
    let run_id = run_id();
    let value: serde_json::Value = serde_json::from_str(&format!(
        "{{\"kind\":\"verified\",\"journal_head\":{{\"commit_digest\":\"sha256-jcs-v1:{}\",\"run_sequence\":1}},\"record_count\":1,\"run_id\":\"{run_id}\",\"semantic_head\":{{\"admission_ref\":{{\"ordinal\":0,\"record_hash\":\"sha256-jcs-v1:{}\",\"run_id\":\"{run_id}\",\"run_sequence\":1}},\"kind\":\"genesis\",\"semantic_state_digest\":\"sha256-jcs-v1:{}\"}},\"status\":\"closed\",\"version\":\"mfm.structured-replay-result.v1\"}}",
        "0".repeat(64),
        "0".repeat(64),
        "0".repeat(64),
    ))
    .expect("replay JSON");
    mfm_journal::structured::canonical_json(&value)
        .expect("canonical replay JSON")
        .as_bytes()
        .to_vec()
}

#[test]
fn app_page_request_uses_the_closed_public_bounds() {
    assert_eq!(PageRequest::default().effective_limit(), DEFAULT_PAGE_LIMIT);
    assert!(PageRequest::new(None, Some(1)).is_ok());
    assert!(PageRequest::new(None, Some(MAX_PAGE_LIMIT)).is_ok());
    assert!(PageRequest::new(None, Some(0)).is_err());
    assert!(PageRequest::new(None, Some(MAX_PAGE_LIMIT + 1)).is_err());
}

fn run_id() -> RunId {
    RunId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([7; 32]),
    )
}
