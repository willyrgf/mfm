use mfm_live_adapter_authority_prototype::PrototypeLiveAdapter;
use mfm_program_authority_prototype::RequestView;
use mfm_runtime_authority_prototype::{authorize_access, commit_live_observation};
use mfm_store_authority_prototype::{
    AuthorizationAppend, AuthorizationAppendOutcome, CommitAcknowledgement, PrototypeStore,
};

#[test]
fn directly_observed_append_flows_once_through_runtime_and_live_adapter() {
    let mut store = PrototypeStore::new();
    let outcome = store.append_authorization(
        AuthorizationAppend::new("append-a", "candidate-a"),
        CommitAcknowledgement::DirectlyObserved,
    );
    let AuthorizationAppendOutcome::NewlyAppended {
        committed,
        authorization,
    } = outcome
    else {
        panic!("directly observed first append must mint authority");
    };

    let request = RequestView::new("prototype.request.v1", b"request-value");
    let access = authorize_access(authorization, request);
    let adapter = PrototypeLiveAdapter::new(b"observation-value".to_vec());
    let receipt = commit_live_observation(&adapter, access);

    assert_eq!(adapter.last_request_len(), Some(b"request-value".len()));
    assert_eq!(receipt.authorization_ref(), committed.authorization_ref());
    assert_eq!(receipt.schema_len(), "prototype.observation.v1".len());
    assert_eq!(receipt.value_len(), b"observation-value".len());
}
