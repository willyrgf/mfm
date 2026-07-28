#[path = "recoverability_store_contract_prototype/model.rs"]
mod model;

use model::{
    AdmissionKey, AdmissionOutcome, AdmissionRoot, AppendRejection, AppendRequestId,
    AuthorizationAppend, AuthorizationAppendOutcome, AuthorizationId, CandidateDigest,
    CommitAcknowledgement, EntryPointOperationId, InvocationIdentity, JournalAppendOutcome,
    JournalHead, ObservationAppend, PrototypeStore, Reconciliation, RunAccessAuthorityIssuer,
    RunAccessGrant, RunAccessRejection, RunId, StoreScopeId, TenantScopeId, TransitionAppend,
};

const STORE: StoreScopeId = StoreScopeId::new("store-a");
const TENANT: TenantScopeId = TenantScopeId::new("tenant-a");
const ENTRY_POINT: EntryPointOperationId = EntryPointOperationId::new("operation-a");

fn admission_key(invocation: &'static str) -> AdmissionKey {
    AdmissionKey::new(TENANT, ENTRY_POINT, InvocationIdentity::new(invocation))
}

fn admission_root(key: AdmissionKey) -> AdmissionRoot {
    AdmissionRoot::new(
        key,
        "operation-contract-a",
        "config-manifest-a",
        "genesis-a",
    )
}

fn newly_admitted(store: &mut PrototypeStore, invocation: &'static str) -> (RunId, JournalHead) {
    let AdmissionOutcome::NewlyAdmitted { run_id, head } =
        store.admit_run(admission_root(admission_key(invocation)))
    else {
        panic!("fresh admission must create the run");
    };
    (run_id, head)
}

#[test]
fn first_admission_attaches_only_to_the_exact_root_for_its_logical_key() {
    let mut store = PrototypeStore::new(STORE);
    let key = admission_key("invocation-admission");
    let root = admission_root(key);

    let AdmissionOutcome::NewlyAdmitted {
        run_id,
        head: admission_head,
    } = store.admit_run(root)
    else {
        panic!("first admission must be new");
    };
    assert_eq!(admission_head.run_sequence(), 1);

    assert_eq!(
        store.admit_run(root),
        AdmissionOutcome::AttachedToExisting {
            run_id,
            head: admission_head,
        }
    );

    let changed_root = AdmissionRoot::new(
        key,
        "operation-contract-a",
        "config-manifest-b",
        "genesis-a",
    );
    assert_eq!(
        store.admit_run(changed_root),
        AdmissionOutcome::Conflict { run_id }
    );

    let other_key = admission_key("invocation-other");
    assert!(matches!(
        store.admit_run(admission_root(other_key)),
        AdmissionOutcome::NewlyAdmitted {
            run_id: other_run_id,
            ..
        } if other_run_id == RunId::derive(STORE, other_key)
    ));
}

#[test]
fn journal_head_is_a_per_run_compare_and_swap_with_request_id_idempotency() {
    let mut store = PrototypeStore::new(STORE);
    let (run_id, admission_head) = newly_admitted(&mut store, "invocation-cas");
    let (other_run_id, other_admission_head) =
        newly_admitted(&mut store, "invocation-cas-other-run");
    let first = TransitionAppend::new(
        run_id,
        admission_head,
        AppendRequestId::new("transition-first"),
        CandidateDigest::new("transition-first-candidate"),
        false,
    );

    let JournalAppendOutcome::NewlyAppended { head: first_head } = store
        .commit_transition(first, CommitAcknowledgement::DirectlyObserved)
        .expect("first transition must append")
    else {
        panic!("first transition must be directly observed");
    };
    assert_eq!(first_head.run_sequence(), 2);
    assert_eq!(store.current_head(other_run_id), Some(other_admission_head));

    assert_eq!(
        store
            .commit_transition(first, CommitAcknowledgement::DirectlyObserved)
            .expect("exact retry must resolve idempotently"),
        JournalAppendOutcome::AlreadyCommitted { head: first_head }
    );

    let changed_candidate = TransitionAppend::new(
        run_id,
        admission_head,
        AppendRequestId::new("transition-first"),
        CandidateDigest::new("changed-candidate"),
        false,
    );
    assert_eq!(
        store.commit_transition(changed_candidate, CommitAcknowledgement::DirectlyObserved),
        Err(AppendRejection::AppendRequestConflict {
            committed: first_head,
        })
    );

    let changed_predecessor = TransitionAppend::new(
        run_id,
        first_head,
        AppendRequestId::new("transition-first"),
        CandidateDigest::new("transition-first-candidate"),
        false,
    );
    assert_eq!(
        store.commit_transition(changed_predecessor, CommitAcknowledgement::DirectlyObserved),
        Err(AppendRejection::AppendRequestConflict {
            committed: first_head,
        })
    );

    let stale_competitor = TransitionAppend::new(
        run_id,
        admission_head,
        AppendRequestId::new("transition-stale"),
        CandidateDigest::new("transition-stale-candidate"),
        false,
    );
    assert_eq!(
        store.commit_transition(stale_competitor, CommitAcknowledgement::DirectlyObserved),
        Err(AppendRejection::HeadMismatch {
            current: first_head,
        })
    );
}

#[test]
fn outcome_unknown_reconciles_to_the_original_commit_without_new_authority() {
    let mut store = PrototypeStore::new(STORE);
    let (run_id, admission_head) = newly_admitted(&mut store, "invocation-unknown");
    let append = AuthorizationAppend::new(
        run_id,
        admission_head,
        AppendRequestId::new("authorize-unknown"),
        CandidateDigest::new("authorize-unknown-candidate"),
        AuthorizationId::new("authorization-unknown"),
    );

    assert!(matches!(
        store
            .authorize_external_access(append, CommitAcknowledgement::LostAfterCommit)
            .expect("commit succeeds before acknowledgement is lost"),
        AuthorizationAppendOutcome::OutcomeUnknown
    ));
    let committed_head = store.current_head(run_id).expect("run remains present");
    assert_eq!(committed_head.run_sequence(), 2);

    assert_eq!(
        store.reconcile_authorization_append(append),
        Reconciliation::Committed {
            head: committed_head,
        }
    );
    assert!(matches!(
        store
            .authorize_external_access(append, CommitAcknowledgement::DirectlyObserved)
            .expect("retry finds the original commit"),
        AuthorizationAppendOutcome::AlreadyCommitted {
            head
        } if head == committed_head
    ));
    assert_eq!(
        store.reconcile_authorization_append(AuthorizationAppend::new(
            run_id,
            admission_head,
            AppendRequestId::new("authorize-unknown"),
            CandidateDigest::new("different-candidate"),
            AuthorizationId::new("authorization-unknown"),
        )),
        Reconciliation::Conflict {
            committed: committed_head,
        }
    );
}

#[test]
fn only_a_directly_observed_new_authorization_append_mints_sealed_authority() {
    let mut store = PrototypeStore::new(STORE);
    let (run_id, admission_head) = newly_admitted(&mut store, "invocation-authority");
    let authorization_id = AuthorizationId::new("authorization-direct");
    let append = AuthorizationAppend::new(
        run_id,
        admission_head,
        AppendRequestId::new("authorize-direct"),
        CandidateDigest::new("authorize-direct-candidate"),
        authorization_id,
    );

    let AuthorizationAppendOutcome::NewlyAppended {
        head,
        authorization,
    } = store
        .authorize_external_access(append, CommitAcknowledgement::DirectlyObserved)
        .expect("authorization append")
    else {
        panic!("direct observation of the winning append must mint authority");
    };
    assert_eq!(authorization.run_id(), run_id);
    assert_eq!(authorization.authorization_id(), authorization_id);
    assert_eq!(authorization.committed_head(), head);

    let cross_purpose_reuse = TransitionAppend::new(
        run_id,
        admission_head,
        AppendRequestId::new("authorize-direct"),
        CandidateDigest::new("authorize-direct-candidate"),
        false,
    );
    assert_eq!(
        store.commit_transition(cross_purpose_reuse, CommitAcknowledgement::DirectlyObserved,),
        Err(AppendRejection::AppendRequestConflict { committed: head })
    );

    assert!(matches!(
        store
            .authorize_external_access(append, CommitAcknowledgement::DirectlyObserved)
            .expect("exact retry"),
        AuthorizationAppendOutcome::AlreadyCommitted {
            head: retry_head
        } if retry_head == head
    ));
}

#[test]
fn run_access_authority_is_bound_to_grant_store_tenant_and_run() {
    let mut store = PrototypeStore::new(STORE);
    let (run_id, current_head) = newly_admitted(&mut store, "invocation-access");
    let other_key = admission_key("invocation-access-other-run");
    let other_run_id = RunId::derive(STORE, other_key);
    let issuer = RunAccessAuthorityIssuer::new(STORE, TENANT);

    for grant in [
        RunAccessGrant::Drive,
        RunAccessGrant::Replay,
        RunAccessGrant::ReadPublic,
        RunAccessGrant::InspectTrace,
        RunAccessGrant::InspectAudit,
        RunAccessGrant::Export,
    ] {
        let authority = issuer.issue(grant, run_id);
        assert_eq!(
            store.authorized_head(&authority, grant, run_id),
            Ok(current_head)
        );
    }

    let replay = issuer.issue(RunAccessGrant::Replay, run_id);
    assert_eq!(
        store.authorized_head(&replay, RunAccessGrant::Drive, run_id),
        Err(RunAccessRejection::WrongGrant)
    );

    let foreign_store = RunAccessAuthorityIssuer::new(StoreScopeId::new("store-b"), TENANT)
        .issue(RunAccessGrant::Drive, run_id);
    assert_eq!(
        store.authorized_head(&foreign_store, RunAccessGrant::Drive, run_id),
        Err(RunAccessRejection::WrongStore)
    );

    let foreign_tenant = RunAccessAuthorityIssuer::new(STORE, TenantScopeId::new("tenant-b"))
        .issue(RunAccessGrant::Drive, run_id);
    assert_eq!(
        store.authorized_head(&foreign_tenant, RunAccessGrant::Drive, run_id),
        Err(RunAccessRejection::WrongTenant)
    );

    let foreign_run = issuer.issue(RunAccessGrant::Drive, other_run_id);
    assert_eq!(
        store.authorized_head(&foreign_run, RunAccessGrant::Drive, run_id),
        Err(RunAccessRejection::WrongRun)
    );
}

#[test]
fn closure_keeps_a_fixed_semantic_head_and_accepts_one_late_matched_observation() {
    let mut store = PrototypeStore::new(STORE);
    let (run_id, admission_head) = newly_admitted(&mut store, "invocation-late-observation");
    let authorization_id = AuthorizationId::new("authorization-before-close");
    let authorization = AuthorizationAppend::new(
        run_id,
        admission_head,
        AppendRequestId::new("authorize-before-close"),
        CandidateDigest::new("authorize-before-close-candidate"),
        authorization_id,
    );
    let AuthorizationAppendOutcome::NewlyAppended {
        head: authorization_head,
        ..
    } = store
        .authorize_external_access(authorization, CommitAcknowledgement::DirectlyObserved)
        .expect("authorization before closure")
    else {
        panic!("authorization must append");
    };

    let close = TransitionAppend::new(
        run_id,
        authorization_head,
        AppendRequestId::new("terminal-transition"),
        CandidateDigest::new("terminal-transition-candidate"),
        true,
    );
    let JournalAppendOutcome::NewlyAppended { head: closure_head } = store
        .commit_transition(close, CommitAcknowledgement::DirectlyObserved)
        .expect("terminal transition")
    else {
        panic!("terminal transition must append");
    };
    assert_eq!(store.closure_head(run_id), Some(closure_head));

    let late_observation = ObservationAppend::new(
        run_id,
        closure_head,
        AppendRequestId::new("late-observation"),
        CandidateDigest::new("late-observation-candidate"),
        authorization_id,
    );
    let JournalAppendOutcome::NewlyAppended {
        head: audit_tail_head,
    } = store
        .observe_external_access(late_observation, CommitAcknowledgement::DirectlyObserved)
        .expect("one matched observation remains legal after closure")
    else {
        panic!("late observation must append");
    };
    assert_eq!(
        audit_tail_head.run_sequence(),
        closure_head.run_sequence() + 1
    );
    assert_eq!(store.closure_head(run_id), Some(closure_head));
    assert_eq!(store.current_head(run_id), Some(audit_tail_head));

    let authorization_after_close = AuthorizationAppend::new(
        run_id,
        audit_tail_head,
        AppendRequestId::new("authorization-after-close"),
        CandidateDigest::new("authorization-after-close-candidate"),
        AuthorizationId::new("authorization-after-close"),
    );
    assert!(matches!(
        store.authorize_external_access(
            authorization_after_close,
            CommitAcknowledgement::DirectlyObserved,
        ),
        Err(AppendRejection::RunClosed)
    ));

    let unmatched_observation = ObservationAppend::new(
        run_id,
        audit_tail_head,
        AppendRequestId::new("unmatched-late-observation"),
        CandidateDigest::new("unmatched-late-observation-candidate"),
        AuthorizationId::new("unknown-authorization"),
    );
    assert_eq!(
        store.observe_external_access(
            unmatched_observation,
            CommitAcknowledgement::DirectlyObserved,
        ),
        Err(AppendRejection::UnknownAuthorization)
    );

    assert_eq!(
        store
            .observe_external_access(late_observation, CommitAcknowledgement::DirectlyObserved,)
            .expect("exact retry remains idempotent"),
        JournalAppendOutcome::AlreadyCommitted {
            head: audit_tail_head,
        }
    );

    let second_observation = ObservationAppend::new(
        run_id,
        audit_tail_head,
        AppendRequestId::new("second-late-observation"),
        CandidateDigest::new("second-late-observation-candidate"),
        authorization_id,
    );
    assert_eq!(
        store.observe_external_access(second_observation, CommitAcknowledgement::DirectlyObserved,),
        Err(AppendRejection::ObservationAlreadyRecorded)
    );

    let post_closure_transition = TransitionAppend::new(
        run_id,
        audit_tail_head,
        AppendRequestId::new("post-closure-transition"),
        CandidateDigest::new("post-closure-transition-candidate"),
        false,
    );
    assert_eq!(
        store.commit_transition(
            post_closure_transition,
            CommitAcknowledgement::DirectlyObserved,
        ),
        Err(AppendRejection::RunClosed)
    );
}
