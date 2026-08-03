use mfm_app::{PageRequest, DEFAULT_PAGE_LIMIT, MAX_PAGE_LIMIT};
use mfm_ids::{DigestAlgorithm, DigestBytes, RunId};
use mfm_replay::structured::{
    project_unavailable_comparison, project_unavailable_reproduction, StructuredReplayResult,
};

#[test]
fn unavailable_replay_results_have_the_current_structured_shape() {
    let run_id = run_id();
    let cases = [
        (
            project_unavailable_reproduction(&run_id).expect("reproduction projection"),
            "reproduction_unavailable",
        ),
        (
            project_unavailable_comparison(&run_id).expect("comparison projection"),
            "comparison_unavailable",
        ),
    ];

    for (projection, kind) in cases {
        assert_eq!(
            projection.as_bytes(),
            format!("{{\"kind\":\"{kind}\",\"result\":\"unavailable\",\"run_id\":\"{run_id}\"}}")
                .as_bytes()
        );
        assert_eq!(
            StructuredReplayResult::strict_decode(projection.as_bytes())
                .expect("strict structured replay decode")
                .as_bytes(),
            projection.as_bytes(),
        );
    }
}

#[test]
fn structured_replay_decode_rejects_noncanonical_or_float_bytes() {
    for bytes in [
        br#"{ "kind":"reproduction_unavailable","result":"unavailable","run_id":"run:sha256-jcs-v1:0000000000000000000000000000000000000000000000000000000000000000"}"#.as_slice(),
        br#"{"kind":"reproduction_unavailable","ratio":1.5,"result":"unavailable","run_id":"run:sha256-jcs-v1:0000000000000000000000000000000000000000000000000000000000000000"}"#.as_slice(),
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
        format!(
            r#"{{"extra":true,"kind":"reproduction_unavailable","result":"unavailable","run_id":"{run_id}"}}"#
        ),
        format!(r#"{{"kind":"reproduction_unavailable","result":"unknown","run_id":"{run_id}"}}"#),
        format!(r#"{{"kind":"legacy_replay_result","result":"unavailable","run_id":"{run_id}"}}"#),
    ];

    for bytes in cases {
        assert!(
            StructuredReplayResult::strict_decode(bytes.as_bytes()).is_err(),
            "canonical hostile shape passed: {bytes}"
        );
    }
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
