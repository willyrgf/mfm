use mfm_app::{PageRequest, DEFAULT_PAGE_LIMIT, MAX_PAGE_LIMIT};
use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_ids::RunId;
use mfm_replay::v2::{CanonicalReplayResult, ExactReproduction};

#[test]
fn exact_reproduction_results_have_the_frozen_public_shape() {
    let run_id = run_id();

    let unavailable = ExactReproduction::Unavailable
        .canonical_result(&run_id)
        .expect("encode unavailable result");
    assert_eq!(
        unavailable.as_bytes(),
        format!("{{\"kind\":\"reproduced\",\"result\":\"unavailable\",\"run_id\":\"{run_id}\"}}")
            .as_bytes()
    );
    assert_eq!(
        CanonicalReplayResult::strict_decode(unavailable.as_bytes()).expect("decode unavailable"),
        unavailable
    );

    let matched = ExactReproduction::Matched
        .canonical_result(&run_id)
        .expect("encode matched result");
    assert_eq!(
        matched.as_bytes(),
        format!("{{\"kind\":\"reproduced\",\"result\":\"matched\",\"run_id\":\"{run_id}\"}}")
            .as_bytes()
    );
    assert_eq!(
        CanonicalReplayResult::strict_decode(matched.as_bytes()).expect("decode matched"),
        matched
    );

    let mismatch = ExactReproduction::Mismatch {
        transition_ref: None,
    }
    .canonical_result(&run_id)
    .expect("encode mismatch result");
    assert_eq!(
        mismatch.as_bytes(),
        format!("{{\"kind\":\"reproduced\",\"result\":\"mismatch\",\"run_id\":\"{run_id}\"}}")
            .as_bytes()
    );
    assert_eq!(
        CanonicalReplayResult::strict_decode(mismatch.as_bytes()).expect("decode mismatch"),
        mismatch
    );
}

#[test]
fn unavailable_reproduction_rejects_diagnostics_and_stages() {
    let run_id = run_id();

    for extra in ["diagnostic", "reason", "stage", "transition_ref"] {
        let noncanonical = format!(
            "{{\"{extra}\":\"redacted\",\"kind\":\"reproduced\",\"result\":\"unavailable\",\
             \"run_id\":\"{run_id}\"}}"
        );
        let bytes =
            PlainCanonicalJsonBytes::from_json_str(&noncanonical).expect("canonical test input");
        assert!(
            CanonicalReplayResult::strict_decode(bytes.as_bytes()).is_err(),
            "unavailable result must reject extra field {extra}"
        );
    }
}

#[test]
fn trace_and_audit_share_one_bounded_page_request() {
    let default = PageRequest::default();
    assert_eq!(default.effective_limit(), DEFAULT_PAGE_LIMIT);
    assert_eq!(default.cursor(), None);

    let maximum = PageRequest::new(Some("opaque".to_owned()), Some(MAX_PAGE_LIMIT))
        .expect("maximum page request");
    assert_eq!(maximum.effective_limit(), MAX_PAGE_LIMIT);
    assert_eq!(maximum.cursor(), Some("opaque"));
    assert!(PageRequest::new(None, Some(0)).is_err());
    assert!(PageRequest::new(None, Some(MAX_PAGE_LIMIT + 1)).is_err());
}

fn run_id() -> RunId {
    RunId::parse(format!("run:sha256-jcs-v1:{}", "1".repeat(64))).expect("run id")
}
