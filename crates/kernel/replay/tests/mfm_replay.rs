use std::str::FromStr;

use mfm_ids::{JournalRecordHash, RunId};
use mfm_journal::v1::{RecordRef, TransitionRef};
use mfm_replay::v1::{ExactReproduction, ReplayError};

#[path = "../../../../tests/support/recoverability_v2.rs"]
mod recoverability_v2_support;

#[test]
fn replay_executes_the_complete_recoverability_v2_corpus() {
    recoverability_v2_support::run_consumer("mfm-replay", |vector| {
        recoverability_v2_support::assert_lower_layer_owner_vector(vector);

        let run_id = RunId::from_str(
            "run:sha256-jcs-v1:\
             0000000000000000000000000000000000000000000000000000000000000000",
        )
        .expect("frozen sample run id");
        let unavailable = ExactReproduction::Unavailable
            .canonical_result(&run_id)
            .expect("canonical unavailable result");
        let decoded = mfm_replay::v1::CanonicalReplayResult::strict_decode(unavailable.as_bytes())
            .expect("strict replay result");
        let value: serde_json::Value =
            serde_json::from_slice(decoded.as_bytes()).expect("canonical replay JSON");
        let object = value.as_object().expect("replay result object");
        assert_eq!(
            object.get("kind").and_then(|value| value.as_str()),
            Some("reproduced")
        );
        assert_eq!(
            object.get("result").and_then(|value| value.as_str()),
            Some("unavailable")
        );
        assert!(!object.contains_key("reason"));
        assert!(!object.contains_key("stage"));
        assert!(!object.contains_key("safe_diagnostic"));

        let invalid = ReplayError::InvalidRecordedHistory;
        assert_eq!(invalid.code(), "MFM_REPLAY_RECORDED_HISTORY_INVALID");
        assert!(!invalid.to_string().contains(vector.id()));

        let not_found = ReplayError::RunNotFound;
        assert_eq!(not_found.code(), "MFM_REPLAY_RUN_NOT_FOUND");
    });
}

#[test]
fn reproduction_result_has_only_the_frozen_optional_transition_field() {
    let run_id = sample_run_id();
    let transition_ref = sample_transition_ref(&run_id, 2, 0);
    let mismatch = ExactReproduction::Mismatch {
        transition_ref: Some(transition_ref),
    }
    .canonical_result(&run_id)
    .expect("canonical mismatch");
    let mismatch: serde_json::Value =
        serde_json::from_slice(mismatch.as_bytes()).expect("mismatch JSON");
    let mismatch = mismatch.as_object().expect("mismatch object");
    assert_eq!(
        mismatch.get("result").and_then(serde_json::Value::as_str),
        Some("mismatch")
    );
    assert!(mismatch.contains_key("transition_ref"));
    assert_eq!(mismatch.len(), 4);

    for outcome in [ExactReproduction::Matched, ExactReproduction::Unavailable] {
        let value = outcome
            .canonical_result(&run_id)
            .expect("canonical reproduction");
        let value: serde_json::Value =
            serde_json::from_slice(value.as_bytes()).expect("reproduction JSON");
        let value = value.as_object().expect("reproduction object");
        assert!(!value.contains_key("transition_ref"));
        assert!(!value.contains_key("reason"));
        assert_eq!(value.len(), 3);
    }
}

fn sample_run_id() -> RunId {
    RunId::from_str(
        "run:sha256-jcs-v1:\
         0000000000000000000000000000000000000000000000000000000000000000",
    )
    .expect("sample run id")
}

fn sample_transition_ref(run_id: &RunId, run_sequence: u64, ordinal: u32) -> TransitionRef {
    let hash = JournalRecordHash::from_str(
        "sha256-jcs-v1:\
         1111111111111111111111111111111111111111111111111111111111111111",
    )
    .expect("sample record hash");
    let record = RecordRef::new(run_id, run_sequence, ordinal, &hash).expect("record ref");
    TransitionRef::new(&record).expect("transition ref")
}
