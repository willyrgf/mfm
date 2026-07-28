use super::*;

#[derive(Clone, Copy, Debug)]
pub(super) enum CommittedJournalArtifactMode {
    Missing,
    TamperFactResponse,
}

pub(super) fn corrupt_committed_fact_response_for_test(
    store: &store::AsyncInMemoryRunStore,
    run_id: &RunId,
    mode: CommittedJournalArtifactMode,
) {
    let records = store
        .committed_records_for_test(run_id)
        .expect("committed records");
    let claim = records
        .iter()
        .find_map(|record| match record.payload() {
            events::KernelEventPayload::FactRecorded(payload) => Some(&payload.claim),
            _ => None,
        })
        .expect("fact response evidence");

    let changed = match mode {
        CommittedJournalArtifactMode::Missing => store
            .remove_retained_artifact_bytes_for_test(
                claim.response().artifact_id(),
                claim.response().artifact_evidence_hash(),
            )
            .expect("remove retained fact response"),
        CommittedJournalArtifactMode::TamperFactResponse => store
            .replace_retained_artifact_bytes_for_test(
                claim.response().artifact_id(),
                claim.response().artifact_evidence_hash(),
                b"tampered fact response".to_vec(),
            )
            .expect("replace retained fact response"),
    };
    assert!(
        changed,
        "fact response artifact must exist before corruption"
    );
}
