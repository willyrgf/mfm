#![cfg(feature = "parity-tests")]
//! Retained real-PostgreSQL executor/reference-destination contract prototype.
//!
//! This fixture owns temporary schemas only. Ledger payloads are opaque `BYTEA`; the local hashes
//! detect prototype corruption but deliberately do not freeze canonical annex encodings.
//!
//! ## Predicate-to-test matrix
//!
//! | Required predicate | Retained test |
//! | --- | --- |
//! | Effect plus zero-or-one resource changes under one atomic CAS | `effect_and_optional_resource_cas_are_atomic` |
//! | Append identity yields `ExistingSame`, conflict, and reconcilable `OutcomeUnknown` | `unknown_append_reconciles_and_only_positive_new_can_enter_destination` |
//! | Separate PostgreSQL pools serialize same and conflicting concurrent append identities | `concurrent_append_identity_is_atomic_across_independent_pools` |
//! | Only directly observed new authorization carries target-entry authority | `unknown_append_reconciles_and_only_positive_new_can_enter_destination` |
//! | Recovery converges after every executor stage boundary | `executor_crash_matrix_converges_without_duplicate_mutation` |
//! | Every successful target entry has exact durable attempt provenance | `destination_receipts_survive_reordered_attempts_and_reload_exactly` |
//! | A pre-terminal receipt can strengthen the tombstoned audit exactly once after reload | `tombstoned_executor_recovers_exact_late_receipt_without_reentry` |
//! | Bounded UTF-8 effect identifiers fit the fixed completion byte reserve | `effect_id_byte_bound_is_enforced_before_persistence` |
//! | Promotion requires exact authoritative logical heads | `promotion_rejects_missing_ancestor_extra_corrupt_and_forked_candidates` |
//! | Stale instance/writer cannot append after promotion | `promotion_rejects_missing_ancestor_extra_corrupt_and_forked_candidates` |
//! | Promotion serializes with an in-flight append | `promotion_racing_in_flight_append_observes_its_committed_head` |
//! | Destination generation fences delayed work from an old writer | `destination_generation_fences_delayed_old_enqueue` |
//! | Promotion waits for target entry that already holds the shared destination fence | `promotion_waits_for_entry_holding_destination_fence` |
//! | Finite inventory is unique, monotonic, single-use, and restore-stable | `account_inventory_survives_concurrency_crash_and_restore` |
//! | Account-keyed sequence allocation is unique, monotonic, crash-reconcilable, and restore-stable | `account_sequence_policy_is_monotonic_across_concurrency_crash_and_restore` |
//! | Simulated destination IO holds no PostgreSQL ledger lock | `destination_io_wait_holds_no_ledger_lock` |
//! | Promotion preserves exact executor binding and non-rollback ledger generation | `promotion_preserves_binding_generation_and_canonical_round_trip` |
//! | Canonical candidate rows round-trip without freezing annex bytes | `promotion_preserves_binding_generation_and_canonical_round_trip` |
//! | Hostile copied binding, proof, and object-closure rows fail closed | `hostile_candidate_rows_fail_closed` |
//! | Missing, extra, mutated, ancestor, descendant, and forked restore surfaces fail closed | `hostile_promotion_surface_matrix_fails_closed` |
//! | Credentials, bearer bytes, and their fingerprints never reach retained/error surfaces | `secret_sentinels_never_reach_retained_or_error_surfaces` |
//! | Non-UTF-8 payloads remain exact opaque bytes | `unknown_append_reconciles_and_only_positive_new_can_enter_destination` |

#[path = "recoverability_postgres_executor_prototype/support/mod.rs"]
mod support;

use std::{sync::Arc, time::Duration};

use support::{
    authority_effect, contains_bytes, forbidden_encodings, pause_pair, pending_effect,
    second_resource_effect, AuthorizationFailurePoint, AuthorizationOutcome, CandidateMutation,
    CommitAcknowledgement, CrashPoint, DestinationMutation, DriveOutcome, EffectState, Executor,
    JournalHead, PrototypeError, Reconciliation, SequenceAcknowledgement,
    SequenceAllocationOutcome, TargetEntryWitness, TestTopology, MAX_EFFECT_ATTEMPTS,
    MAX_EFFECT_ID_UTF8_BYTES,
};
use tokio::{
    sync::{oneshot, Barrier},
    time::timeout,
};

fn require_new_authorization(outcome: AuthorizationOutcome) -> (JournalHead, TargetEntryWitness) {
    let AuthorizationOutcome::NewlyAppended {
        commit,
        target_entry,
    } = outcome
    else {
        panic!("expected a directly observed new authorization");
    };
    (commit.head().clone(), target_entry)
}

#[tokio::test]
async fn effect_and_optional_resource_cas_are_atomic() {
    let (topology, writer) = TestTopology::new(8).await.expect("create topology");
    let initial_a = writer
        .seed_effect("effect-a", Some("resource-shared"), b"semantic-a")
        .await
        .expect("seed effect a");
    writer
        .seed_effect("effect-b", Some("resource-shared"), b"semantic-b")
        .await
        .expect("seed effect b");
    writer
        .seed_effect("effect-without-resource", None, b"semantic-no-resource")
        .await
        .expect("seed optional-resource effect");

    let rollback_request = writer
        .authorization_request("effect-a", "rollback-a", b"opaque-a")
        .await
        .expect("prepare rollback request");
    assert!(matches!(
        writer
            .authorize_effect(
                &rollback_request,
                CommitAcknowledgement::DirectlyObserved,
                AuthorizationFailurePoint::RollbackAfterEffectAndResourceCas,
            )
            .await,
        Err(PrototypeError::SimulatedRollback)
    ));
    assert_eq!(
        writer
            .effect_snapshot("effect-a")
            .await
            .expect("effect after rollback")
            .state,
        EffectState::Pending
    );
    assert_eq!(
        writer
            .resource_holder("resource-shared")
            .await
            .expect("resource after rollback"),
        None
    );
    assert_eq!(
        writer.current_head("effect-a").await.expect("head"),
        initial_a
    );
    assert_eq!(
        writer
            .append_request_count("effect-a")
            .await
            .expect("append count"),
        0
    );

    let request_a = writer
        .authorization_request("effect-a", "cas-a", b"opaque-a")
        .await
        .expect("prepare a");
    let request_b = writer
        .authorization_request("effect-b", "cas-b", b"opaque-b")
        .await
        .expect("prepare b");
    let (outcome_a, outcome_b) = tokio::join!(
        writer.authorize_effect(
            &request_a,
            CommitAcknowledgement::DirectlyObserved,
            AuthorizationFailurePoint::None,
        ),
        writer.authorize_effect(
            &request_b,
            CommitAcknowledgement::DirectlyObserved,
            AuthorizationFailurePoint::None,
        )
    );
    let winner = match (outcome_a, outcome_b) {
        (Ok(AuthorizationOutcome::NewlyAppended { .. }), Err(PrototypeError::ResourceBusy)) => {
            "effect-a"
        }
        (Err(PrototypeError::ResourceBusy), Ok(AuthorizationOutcome::NewlyAppended { .. })) => {
            "effect-b"
        }
        (left, right) => panic!("exactly one resource claimant must win: {left:?}, {right:?}"),
    };
    assert_eq!(
        writer
            .resource_holder("resource-shared")
            .await
            .expect("resource holder")
            .as_deref(),
        Some(winner)
    );
    for effect_id in ["effect-a", "effect-b"] {
        let snapshot = writer
            .effect_snapshot(effect_id)
            .await
            .expect("effect snapshot");
        assert_eq!(
            snapshot.state,
            if effect_id == winner {
                EffectState::Authorized
            } else {
                EffectState::Pending
            }
        );
    }

    let no_resource = writer
        .authorization_request(
            "effect-without-resource",
            "cas-no-resource",
            b"opaque-no-resource",
        )
        .await
        .expect("prepare optional-resource authorization");
    assert!(matches!(
        writer
            .authorize_effect(
                &no_resource,
                CommitAcknowledgement::DirectlyObserved,
                AuthorizationFailurePoint::None,
            )
            .await
            .expect("authorize without resource"),
        AuthorizationOutcome::NewlyAppended { .. }
    ));

    writer
        .seed_destination_effect(
            "effect-corrupt-resource-intent",
            b"semantic corrupt resource intent",
        )
        .await
        .expect("seed authenticated resource intent");
    writer
        .corrupt_effect_intent_resource_key("effect-corrupt-resource-intent")
        .await
        .expect("inject hostile intent projection");
    assert!(matches!(
        writer
            .effect_snapshot("effect-corrupt-resource-intent")
            .await,
        Err(PrototypeError::Model(_))
    ));

    topology.cleanup().await.expect("clean topology");
}

#[tokio::test]
async fn unknown_append_reconciles_and_only_positive_new_can_enter_destination() {
    let (topology, writer) = TestTopology::new(4).await.expect("create topology");
    writer
        .seed_destination_effect("effect-unknown", b"semantic-request")
        .await
        .expect("seed effect");
    let opaque_authorization = [0x00, 0xff, 0x80, b'{', b'}', 0x00];
    let request = writer
        .authorization_request("effect-unknown", "append-unknown", &opaque_authorization)
        .await
        .expect("prepare authorization");

    assert!(matches!(
        writer
            .authorize_effect(
                &request,
                CommitAcknowledgement::LostAfterCommit,
                AuthorizationFailurePoint::None,
            )
            .await
            .expect("commit then lose acknowledgement"),
        AuthorizationOutcome::OutcomeUnknown
    ));
    let reconciliation = writer
        .reconcile_authorization(&request)
        .await
        .expect("reconcile append");
    let Reconciliation::Committed { commit: reconciled } = reconciliation else {
        panic!("unknown append must reconcile to its commit");
    };
    assert_eq!(
        writer
            .reload_authorization("effect-unknown", "append-unknown")
            .await
            .expect("reload")
            .expect("committed authorization"),
        reconciled
    );
    assert_eq!(
        writer
            .journal_payload("effect-unknown", reconciled.head().sequence())
            .await
            .expect("reload opaque payload bytes"),
        opaque_authorization
    );
    assert!(matches!(
        writer
            .authorize_effect(
                &request,
                CommitAcknowledgement::DirectlyObserved,
                AuthorizationFailurePoint::None,
            )
            .await
            .expect("idempotent retry"),
        AuthorizationOutcome::ExistingSame { commit } if commit == reconciled
    ));
    let conflicting_request = request.with_payload(b"different opaque auth");
    assert!(matches!(
        writer
            .authorize_effect(
                &conflicting_request,
                CommitAcknowledgement::DirectlyObserved,
                AuthorizationFailurePoint::None,
            )
            .await
            .expect("conflicting retry"),
        AuthorizationOutcome::Conflict { commit } if commit == reconciled
    ));
    assert_eq!(
        writer
            .reconcile_authorization(&conflicting_request)
            .await
            .expect("reconcile conflicting append identity"),
        Reconciliation::Conflict {
            commit: reconciled.clone(),
        }
    );
    assert_eq!(
        topology
            .destination_mutation_count("effect-unknown")
            .await
            .expect("destination count"),
        0,
        "unknown, reload, existing, and conflict outcomes carry no target-entry witness"
    );

    let fresh = writer
        .authorization_request("effect-unknown", "append-fresh", &opaque_authorization)
        .await
        .expect("prepare fresh authorization");
    let (_, witness) = require_new_authorization(
        writer
            .authorize_effect(
                &fresh,
                CommitAcknowledgement::DirectlyObserved,
                AuthorizationFailurePoint::None,
            )
            .await
            .expect("directly observed fresh append"),
    );
    assert!(matches!(
        writer
            .enter_destination(witness, None)
            .await
            .expect("enter destination"),
        DestinationMutation::NewlyApplied { .. }
    ));
    assert_eq!(
        topology
            .destination_mutation_count("effect-unknown")
            .await
            .expect("destination count"),
        1
    );

    topology.cleanup().await.expect("clean topology");
}

#[tokio::test]
async fn concurrent_append_identity_is_atomic_across_independent_pools() {
    let (topology, writer) = TestTopology::new(4).await.expect("create topology");
    writer
        .seed_effect("effect-concurrent-same", None, b"semantic-same")
        .await
        .expect("seed same-request effect");
    let same_request = writer
        .authorization_request(
            "effect-concurrent-same",
            "concurrent-identity",
            b"opaque concurrent identity",
        )
        .await
        .expect("prepare shared append identity");
    let left_writer = writer
        .independent_pool_writer()
        .await
        .expect("open independent left pool");
    let right_writer = writer
        .independent_pool_writer()
        .await
        .expect("open independent right pool");
    let barrier = Arc::new(Barrier::new(3));
    let left_barrier = Arc::clone(&barrier);
    let left_request = same_request.clone();
    let left = tokio::spawn(async move {
        left_barrier.wait().await;
        left_writer
            .authorize_effect(
                &left_request,
                CommitAcknowledgement::DirectlyObserved,
                AuthorizationFailurePoint::None,
            )
            .await
    });
    let right_barrier = Arc::clone(&barrier);
    let right_request = same_request.clone();
    let right = tokio::spawn(async move {
        right_barrier.wait().await;
        right_writer
            .authorize_effect(
                &right_request,
                CommitAcknowledgement::DirectlyObserved,
                AuthorizationFailurePoint::None,
            )
            .await
    });
    barrier.wait().await;
    let left = left.await.expect("join same left").expect("same left");
    let right = right.await.expect("join same right").expect("same right");
    let (new_commit, existing_commit) = match (left, right) {
        (
            AuthorizationOutcome::NewlyAppended { commit, .. },
            AuthorizationOutcome::ExistingSame {
                commit: existing_commit,
            },
        )
        | (
            AuthorizationOutcome::ExistingSame {
                commit: existing_commit,
            },
            AuthorizationOutcome::NewlyAppended { commit, .. },
        ) => (commit, existing_commit),
        (left, right) => panic!("expected one new and one existing append: {left:?}, {right:?}"),
    };
    assert_eq!(new_commit, existing_commit);
    assert_eq!(
        writer
            .append_request_count("effect-concurrent-same")
            .await
            .expect("same append count"),
        1
    );

    writer
        .seed_effect("effect-concurrent-conflict", None, b"semantic-conflict")
        .await
        .expect("seed conflicting-request effect");
    let first_request = writer
        .authorization_request(
            "effect-concurrent-conflict",
            "conflicting-identity",
            b"opaque candidate a",
        )
        .await
        .expect("prepare candidate a");
    let second_request = first_request.with_payload(b"opaque candidate b");
    let left_writer = writer
        .independent_pool_writer()
        .await
        .expect("open conflicting left pool");
    let right_writer = writer
        .independent_pool_writer()
        .await
        .expect("open conflicting right pool");
    let barrier = Arc::new(Barrier::new(3));
    let left_barrier = Arc::clone(&barrier);
    let left = tokio::spawn(async move {
        left_barrier.wait().await;
        left_writer
            .authorize_effect(
                &first_request,
                CommitAcknowledgement::DirectlyObserved,
                AuthorizationFailurePoint::None,
            )
            .await
    });
    let right_barrier = Arc::clone(&barrier);
    let right = tokio::spawn(async move {
        right_barrier.wait().await;
        right_writer
            .authorize_effect(
                &second_request,
                CommitAcknowledgement::DirectlyObserved,
                AuthorizationFailurePoint::None,
            )
            .await
    });
    barrier.wait().await;
    let left = left
        .await
        .expect("join conflict left")
        .expect("conflict left");
    let right = right
        .await
        .expect("join conflict right")
        .expect("conflict right");
    let (new_commit, conflict_commit) = match (left, right) {
        (
            AuthorizationOutcome::NewlyAppended { commit, .. },
            AuthorizationOutcome::Conflict {
                commit: conflict_commit,
            },
        )
        | (
            AuthorizationOutcome::Conflict {
                commit: conflict_commit,
            },
            AuthorizationOutcome::NewlyAppended { commit, .. },
        ) => (commit, conflict_commit),
        (left, right) => panic!("expected one new and one conflicting append: {left:?}, {right:?}"),
    };
    assert_eq!(new_commit, conflict_commit);
    assert_eq!(new_commit.head().sequence(), 2);
    assert_eq!(
        writer
            .append_request_count("effect-concurrent-conflict")
            .await
            .expect("conflict append count"),
        1
    );

    topology.cleanup().await.expect("clean topology");
}

#[tokio::test]
async fn executor_crash_matrix_converges_without_duplicate_mutation() {
    let crash_points = [
        CrashPoint::BeforeAuthorization,
        CrashPoint::AfterAuthorization,
        CrashPoint::BeforeDestinationMutation,
        CrashPoint::AfterDestinationMutation,
        CrashPoint::BeforeObservation,
        CrashPoint::AfterObservation,
        CrashPoint::BeforeTombstone,
        CrashPoint::AfterTombstone,
    ];
    let (topology, writer) = TestTopology::new(crash_points.len() + 2)
        .await
        .expect("create topology");
    let executor = Executor::new(writer.clone());

    for (index, crash_point) in crash_points.into_iter().enumerate() {
        let effect_id = format!("crash-effect-{index}");
        writer
            .seed_destination_effect(&effect_id, format!("semantic-{index}").as_bytes())
            .await
            .expect("seed crash effect");
        assert_eq!(
            executor
                .drive_effect(&effect_id, crash_point)
                .await
                .expect("drive to crash"),
            DriveOutcome::Crashed(crash_point)
        );
        assert!(matches!(
            executor
                .drive_effect(&effect_id, CrashPoint::Never)
                .await
                .expect("recover effect"),
            DriveOutcome::Completed | DriveOutcome::AlreadyComplete
        ));

        let snapshot = writer
            .effect_snapshot(&effect_id)
            .await
            .expect("recovered effect");
        assert_eq!(snapshot.state, EffectState::Tombstoned);
        assert_eq!(snapshot.observation_count, 1);
        assert!(snapshot.destination_account.is_some());
        assert_eq!(
            snapshot.resource_id.as_deref(),
            Some("resource-key:reference-destination-account-inventory:v1")
        );
        assert_eq!(
            topology
                .destination_mutation_count(&effect_id)
                .await
                .expect("destination mutation count"),
            1
        );
    }

    topology.cleanup().await.expect("clean topology");
}

#[tokio::test]
async fn destination_receipts_survive_reordered_attempts_and_reload_exactly() {
    let effect_id = "effect-reordered-receipts";
    let (topology, writer) = TestTopology::new(1).await.expect("create topology");
    writer
        .seed_destination_effect(effect_id, b"semantic-reordered-receipts")
        .await
        .expect("seed destination effect");

    let request_a = writer
        .authorization_request(
            effect_id,
            "authorization-reordered-a",
            b"opaque authorization a",
        )
        .await
        .expect("prepare authorization a");
    let (_, witness_a) = require_new_authorization(
        writer
            .authorize_effect(
                &request_a,
                CommitAcknowledgement::DirectlyObserved,
                AuthorizationFailurePoint::None,
            )
            .await
            .expect("authorize attempt a"),
    );
    let attempt_a = witness_a.attempt_id().to_vec();
    let delayed_writer = writer
        .independent_pool_writer()
        .await
        .expect("open delayed destination pool");
    let (pause, entered, release) = pause_pair();
    let delayed_entry = tokio::spawn(async move {
        delayed_writer
            .enter_destination(witness_a, Some(pause))
            .await
    });
    entered.await.expect("attempt a reaches delayed entry");

    let request_b = writer
        .authorization_request(
            effect_id,
            "authorization-reordered-b",
            b"opaque authorization b",
        )
        .await
        .expect("prepare authorization b");
    let (_, witness_b) = require_new_authorization(
        writer
            .authorize_effect(
                &request_b,
                CommitAcknowledgement::DirectlyObserved,
                AuthorizationFailurePoint::None,
            )
            .await
            .expect("authorize attempt b"),
    );
    let attempt_b = witness_b.attempt_id().to_vec();

    release.send(()).expect("release attempt a");
    {
        let mutation_a = delayed_entry
            .await
            .expect("join attempt a entry")
            .expect("enter attempt a");
        assert!(matches!(
            &mutation_a,
            DestinationMutation::NewlyApplied { .. }
        ));
        let receipt_a = mutation_a.into_receipt();
        assert_eq!(receipt_a.attempt_id(), attempt_a);
    }

    let reloaded_a = writer
        .destination_receipt(effect_id, &attempt_a)
        .await
        .expect("reload attempt a receipt")
        .expect("attempt a receipt is durable");
    assert_eq!(reloaded_a.attempt_id(), attempt_a);
    writer
        .append_observation(reloaded_a)
        .await
        .expect("observe exact attempt a receipt");
    assert_eq!(
        writer
            .observed_attempt_ids(effect_id)
            .await
            .expect("load observations after attempt a"),
        vec![attempt_a.clone()]
    );

    let mutation_b = writer
        .enter_destination(witness_b, None)
        .await
        .expect("enter convergent attempt b");
    assert!(matches!(
        &mutation_b,
        DestinationMutation::ExistingSame { .. }
    ));
    let receipt_b = mutation_b.into_receipt();
    assert_eq!(receipt_b.attempt_id(), attempt_b);
    writer
        .append_observation(receipt_b)
        .await
        .expect("observe exact attempt b receipt");
    assert_eq!(
        writer
            .observed_attempt_ids(effect_id)
            .await
            .expect("load observations after attempt b"),
        vec![attempt_a, attempt_b]
    );
    assert_eq!(
        topology
            .destination_mutation_count(effect_id)
            .await
            .expect("one semantic destination mutation"),
        1
    );
    assert_eq!(
        topology
            .destination_receipt_count(effect_id)
            .await
            .expect("one receipt per successful attempt"),
        2
    );

    topology.cleanup().await.expect("clean topology");
}

#[tokio::test]
async fn tombstoned_executor_recovers_exact_late_receipt_without_reentry() {
    let effect_id = "effect-terminal-late-receipt";
    let (topology, writer) = TestTopology::new(1).await.expect("create topology");
    writer
        .seed_destination_effect(effect_id, b"semantic-terminal-late-receipt")
        .await
        .expect("seed destination effect");

    let request_a = writer
        .authorization_request(
            effect_id,
            "authorization-terminal-a",
            b"opaque terminal authorization a",
        )
        .await
        .expect("prepare terminal attempt a");
    let (_, witness_a) = require_new_authorization(
        writer
            .authorize_effect(
                &request_a,
                CommitAcknowledgement::DirectlyObserved,
                AuthorizationFailurePoint::None,
            )
            .await
            .expect("authorize terminal attempt a"),
    );
    let attempt_a = witness_a.attempt_id().to_vec();

    let request_b = writer
        .authorization_request(
            effect_id,
            "authorization-terminal-b",
            b"opaque terminal authorization b",
        )
        .await
        .expect("prepare terminal attempt b");
    let (_, witness_b) = require_new_authorization(
        writer
            .authorize_effect(
                &request_b,
                CommitAcknowledgement::DirectlyObserved,
                AuthorizationFailurePoint::None,
            )
            .await
            .expect("authorize terminal attempt b"),
    );
    let attempt_b = witness_b.attempt_id().to_vec();

    let mutation_a = writer
        .enter_destination(witness_a, None)
        .await
        .expect("enter terminal attempt a");
    assert!(matches!(
        &mutation_a,
        DestinationMutation::NewlyApplied { .. }
    ));
    writer
        .append_observation(mutation_a.into_receipt())
        .await
        .expect("observe terminal attempt a");
    {
        let mutation_b = writer
            .enter_destination(witness_b, None)
            .await
            .expect("enter convergent terminal attempt b");
        assert!(matches!(
            &mutation_b,
            DestinationMutation::ExistingSame { .. }
        ));
        let lost_receipt_b = mutation_b.into_receipt();
        assert_eq!(lost_receipt_b.attempt_id(), attempt_b);
    }

    writer
        .append_tombstone(effect_id)
        .await
        .expect("commit tombstone for exact attempt a");
    assert!(writer
        .terminal_proof_targets_attempt(effect_id, &attempt_a)
        .await
        .expect("validate initial terminal proof"));
    let terminal = writer
        .effect_snapshot(effect_id)
        .await
        .expect("load terminal snapshot");
    assert_eq!(terminal.state, EffectState::Tombstoned);
    assert_eq!(terminal.authorization_count, 2);
    assert_eq!(terminal.observation_count, 1);
    assert_eq!(terminal.late_observation_count, 0);

    assert_eq!(
        Executor::new(writer.clone())
            .drive_effect(effect_id, CrashPoint::Never)
            .await
            .expect("reload tombstoned effect and drain receipt b"),
        DriveOutcome::AlreadyComplete
    );
    let strengthened = writer
        .effect_snapshot(effect_id)
        .await
        .expect("load strengthened terminal snapshot");
    assert_eq!(strengthened.state, EffectState::Tombstoned);
    assert_eq!(strengthened.authorization_count, 2);
    assert_eq!(strengthened.observation_count, 2);
    assert_eq!(strengthened.late_observation_count, 1);
    assert_eq!(
        writer
            .observed_attempt_ids(effect_id)
            .await
            .expect("load exact terminal observations"),
        vec![attempt_a.clone(), attempt_b]
    );
    assert!(writer
        .terminal_proof_targets_attempt(effect_id, &attempt_a)
        .await
        .expect("terminal proof remains tied to attempt a"));
    assert_eq!(
        topology
            .destination_mutation_count(effect_id)
            .await
            .expect("one semantic target mutation"),
        1
    );
    assert_eq!(
        topology
            .destination_receipt_count(effect_id)
            .await
            .expect("two exact target receipts"),
        2
    );

    assert_eq!(
        Executor::new(writer.clone())
            .drive_effect(effect_id, CrashPoint::Never)
            .await
            .expect("reloading again is idempotent"),
        DriveOutcome::AlreadyComplete
    );
    let idempotent = writer
        .effect_snapshot(effect_id)
        .await
        .expect("load idempotent terminal snapshot");
    assert_eq!(idempotent.authorization_count, 2);
    assert_eq!(idempotent.observation_count, 2);
    assert_eq!(idempotent.late_observation_count, 1);
    assert_eq!(
        topology
            .destination_receipt_count(effect_id)
            .await
            .expect("no post-terminal target entry"),
        2
    );

    topology.cleanup().await.expect("clean topology");
}

#[tokio::test]
async fn effect_id_byte_bound_is_enforced_before_persistence() {
    let (topology, writer) = TestTopology::new(1).await.expect("create topology");
    let maximum_effect_id = "é".repeat(MAX_EFFECT_ID_UTF8_BYTES / "é".len());
    assert_eq!(maximum_effect_id.len(), MAX_EFFECT_ID_UTF8_BYTES);
    writer
        .seed_destination_effect(&maximum_effect_id, b"semantic-maximum-effect-id")
        .await
        .expect("persist exact maximum effect id");
    assert_eq!(
        writer
            .effect_intent_count(&maximum_effect_id)
            .await
            .expect("maximum effect intent is durable"),
        1
    );
    let mut final_witness = None;
    for attempt_ordinal in 0..MAX_EFFECT_ATTEMPTS {
        let request = writer
            .authorization_request(
                &maximum_effect_id,
                &format!("maximum-bound-authorization:{attempt_ordinal}"),
                b"opaque maximum-bound authorization",
            )
            .await
            .expect("prepare maximum-bound authorization");
        let (_, witness) = require_new_authorization(
            writer
                .authorize_effect(
                    &request,
                    CommitAcknowledgement::DirectlyObserved,
                    AuthorizationFailurePoint::None,
                )
                .await
                .expect("admit maximum-bound authorization"),
        );
        if attempt_ordinal == MAX_EFFECT_ATTEMPTS - 1 {
            final_witness = Some(witness);
        }
    }
    let final_destination = writer
        .enter_destination(
            final_witness.expect("final maximum-bound target authority"),
            None,
        )
        .await
        .expect("enter final maximum-bound destination");
    writer
        .append_observation(final_destination.into_receipt())
        .await
        .expect("append maximum-bound observation");
    writer
        .append_tombstone(&maximum_effect_id)
        .await
        .expect("append maximum-bound tombstone");
    let maximum_snapshot = writer
        .effect_snapshot(&maximum_effect_id)
        .await
        .expect("load maximum effect id");
    assert_eq!(maximum_snapshot.state, EffectState::Tombstoned);
    assert_eq!(
        i64::from(maximum_snapshot.authorization_count),
        MAX_EFFECT_ATTEMPTS
    );
    assert_eq!(
        writer
            .completion_record_bytes(&maximum_effect_id)
            .await
            .expect("load maximum-bound completion sizes"),
        (1_041, 1_331)
    );

    let oversized_effect_id = format!("{maximum_effect_id}x");
    assert_eq!(oversized_effect_id.len(), MAX_EFFECT_ID_UTF8_BYTES + 1);
    assert!(matches!(
        writer
            .seed_destination_effect(&oversized_effect_id, b"semantic-oversized-effect-id")
            .await,
        Err(PrototypeError::Model(_))
    ));
    assert_eq!(
        writer
            .effect_intent_count(&oversized_effect_id)
            .await
            .expect("oversized effect intent remains absent"),
        0
    );

    topology.cleanup().await.expect("clean topology");
}

#[tokio::test]
async fn promotion_rejects_missing_ancestor_extra_corrupt_and_forked_candidates() {
    let (topology, writer) = TestTopology::new(4).await.expect("create topology");
    writer
        .seed_effect("effect-restore", None, b"semantic-restore")
        .await
        .expect("seed effect");
    writer
        .append_marker("effect-restore", "marker-before-restore", b"opaque marker")
        .await
        .expect("advance required head");

    let missing = topology
        .snapshot_clone(&writer)
        .await
        .expect("missing candidate");
    topology
        .candidate_remove_stream(&missing, "effect-restore")
        .await
        .expect("remove stream");
    assert!(matches!(
        topology.promote(missing, 1, "writer-missing").await,
        Err(PrototypeError::MissingCandidate { .. })
    ));

    let ancestor = topology
        .snapshot_clone(&writer)
        .await
        .expect("ancestor candidate");
    topology
        .candidate_make_ancestor(&ancestor, "effect-restore")
        .await
        .expect("truncate candidate");
    assert!(matches!(
        topology.promote(ancestor, 1, "writer-ancestor").await,
        Err(PrototypeError::AncestorCandidate { .. })
    ));

    let extra = topology
        .snapshot_clone(&writer)
        .await
        .expect("extra candidate");
    topology
        .candidate_append_extra(&extra, "effect-restore")
        .await
        .expect("append candidate-only suffix");
    assert!(matches!(
        topology.promote(extra, 1, "writer-extra").await,
        Err(PrototypeError::ExtraCandidate { .. })
    ));

    let corrupt = topology
        .snapshot_clone(&writer)
        .await
        .expect("corrupt candidate");
    topology
        .candidate_corrupt_payload(&corrupt, "effect-restore")
        .await
        .expect("corrupt candidate payload");
    assert!(matches!(
        topology.promote(corrupt, 1, "writer-corrupt").await,
        Err(PrototypeError::CorruptCandidate { .. })
    ));

    let forked = topology
        .snapshot_clone(&writer)
        .await
        .expect("forked candidate");
    topology
        .candidate_fork_head(&forked, "effect-restore")
        .await
        .expect("fork candidate");
    assert!(matches!(
        topology.promote(forked, 1, "writer-forked").await,
        Err(PrototypeError::ForkedCandidate { .. })
    ));
    assert_eq!(
        topology
            .writer_generation()
            .await
            .expect("generation after rejection"),
        1
    );

    let exact = topology
        .snapshot_clone(&writer)
        .await
        .expect("exact candidate");
    let promoted = topology
        .promote(exact, 1, "writer-promoted")
        .await
        .expect("promote exact required heads");
    assert_eq!(promoted.generation(), 2);
    assert!(matches!(
        writer
            .append_marker("effect-restore", "stale-writer", b"stale")
            .await,
        Err(PrototypeError::WriterFenced)
    ));
    promoted
        .append_marker("effect-restore", "promoted-writer", b"current")
        .await
        .expect("promoted writer appends");

    topology.cleanup().await.expect("clean topology");
}

#[tokio::test]
async fn promotion_preserves_binding_generation_and_canonical_round_trip() {
    let (topology, writer) = TestTopology::new(2).await.expect("create topology");
    writer
        .seed_effect("effect-binding-restore", None, b"semantic binding restore")
        .await
        .expect("seed binding restore");
    writer
        .append_marker(
            "effect-binding-restore",
            "binding-restore-marker",
            b"\x00opaque\xffrow",
        )
        .await
        .expect("append binding restore marker");
    let binding = writer.executor_binding_ref().to_owned();
    let ledger_generation = writer.ledger_generation();

    let mut wrong_binding = topology
        .snapshot_clone(&writer)
        .await
        .expect("clone wrong binding candidate");
    topology
        .candidate_change_binding(&mut wrong_binding)
        .await
        .expect("change copied binding");
    assert!(matches!(
        topology
            .promote(wrong_binding, 1, "writer-wrong-binding")
            .await,
        Err(PrototypeError::CandidateBindingMismatch)
    ));

    let mut wrong_generation = topology
        .snapshot_clone(&writer)
        .await
        .expect("clone wrong generation candidate");
    topology
        .candidate_change_ledger_generation(&mut wrong_generation)
        .await
        .expect("change copied ledger generation");
    assert!(matches!(
        topology
            .promote(wrong_generation, 1, "writer-wrong-generation")
            .await,
        Err(PrototypeError::CandidateLedgerGenerationMismatch)
    ));

    let exact = topology
        .snapshot_clone(&writer)
        .await
        .expect("clone canonical exact candidate");
    let first_encoding = topology
        .candidate_canonical_round_trip(&exact)
        .await
        .expect("canonical candidate round-trip");
    let second_encoding = topology
        .candidate_canonical_round_trip(&exact)
        .await
        .expect("repeat canonical candidate round-trip");
    assert!(!first_encoding.is_empty());
    assert_eq!(first_encoding, second_encoding);
    let promoted = topology
        .promote(exact, 1, "writer-exact-binding")
        .await
        .expect("promote exact binding and ledger generation");
    assert_eq!(promoted.executor_binding_ref(), binding);
    assert_eq!(promoted.ledger_generation(), ledger_generation);
    assert_eq!(promoted.generation(), 2);

    topology.cleanup().await.expect("clean topology");
}

#[tokio::test]
async fn hostile_candidate_rows_fail_closed() {
    let (topology, writer) = TestTopology::new(2).await.expect("create topology");
    writer
        .seed_effect(
            "effect-hostile-candidate",
            None,
            b"semantic hostile candidate",
        )
        .await
        .expect("seed hostile candidate");
    writer
        .append_marker(
            "effect-hostile-candidate",
            "hostile-candidate-marker",
            b"opaque hostile candidate",
        )
        .await
        .expect("append hostile candidate marker");

    let copied_binding = topology
        .snapshot_clone(&writer)
        .await
        .expect("clone copied binding candidate");
    topology
        .candidate_corrupt_record_binding(&copied_binding, "effect-hostile-candidate")
        .await
        .expect("corrupt copied record binding");
    assert!(matches!(
        topology
            .promote(copied_binding, 1, "writer-hostile-record-binding")
            .await,
        Err(PrototypeError::CorruptCandidate { .. })
    ));

    let copied_generation = topology
        .snapshot_clone(&writer)
        .await
        .expect("clone copied generation candidate");
    topology
        .candidate_corrupt_record_generation(&copied_generation, "effect-hostile-candidate")
        .await
        .expect("corrupt copied record generation");
    assert!(matches!(
        topology
            .promote(copied_generation, 1, "writer-hostile-record-generation")
            .await,
        Err(PrototypeError::CorruptCandidate { .. })
    ));

    let proof = topology
        .snapshot_clone(&writer)
        .await
        .expect("clone proof candidate");
    topology
        .candidate_corrupt_proof(&proof, "effect-hostile-candidate")
        .await
        .expect("corrupt binding proof");
    assert!(matches!(
        topology.promote(proof, 1, "writer-hostile-proof").await,
        Err(PrototypeError::CorruptCandidate { .. })
    ));

    let closure = topology
        .snapshot_clone(&writer)
        .await
        .expect("clone object closure candidate");
    topology
        .candidate_break_object_closure(&closure, "effect-hostile-candidate")
        .await
        .expect("break evidence object closure");
    assert!(matches!(
        topology
            .promote(closure, 1, "writer-hostile-object-closure")
            .await,
        Err(PrototypeError::CorruptCandidate { .. })
    ));
    assert_eq!(topology.writer_generation().await.expect("generation"), 1);

    topology.cleanup().await.expect("clean topology");
}

#[tokio::test]
async fn hostile_promotion_surface_matrix_fails_closed() {
    let (topology, writer) = TestTopology::new(4).await.expect("create topology");
    let executor = Executor::new(writer.clone());
    for effect_id in [authority_effect(), second_resource_effect()] {
        writer
            .seed_destination_effect(effect_id, format!("semantic:{effect_id}").as_bytes())
            .await
            .expect("seed shared-resource effect");
        assert_eq!(
            executor
                .drive_effect(effect_id, CrashPoint::Never)
                .await
                .expect("complete shared-resource effect"),
            DriveOutcome::Completed
        );
    }
    writer
        .seed_effect(
            pending_effect(),
            None,
            b"semantic:pending-hostile-authority",
        )
        .await
        .expect("seed pending effect");

    for mutation in CandidateMutation::ALL {
        let candidate = topology
            .snapshot_clone(&writer)
            .await
            .unwrap_or_else(|error| panic!("clone {} candidate: {error}", mutation.label()));
        topology
            .mutate_restore_candidate(&candidate, *mutation)
            .await
            .unwrap_or_else(|error| panic!("apply {} mutation: {error}", mutation.label()));
        let writer_id = format!("writer-hostile-{}", mutation.label().replace(' ', "-"));
        assert!(
            topology.promote(candidate, 1, &writer_id).await.is_err(),
            "{} candidate unexpectedly promoted",
            mutation.label()
        );
        assert_eq!(
            topology.writer_generation().await.expect("generation"),
            1,
            "{} candidate changed the generation",
            mutation.label()
        );
    }

    topology.cleanup().await.expect("clean topology");
}

#[tokio::test]
async fn promotion_racing_in_flight_append_observes_its_committed_head() {
    let (topology, writer) = TestTopology::new(2).await.expect("create topology");
    let predecessor = writer
        .seed_effect("effect-race", None, b"semantic-race")
        .await
        .expect("seed effect");
    let stale_candidate = topology
        .snapshot_clone(&writer)
        .await
        .expect("clone before append");
    let (append_pause, append_entered, append_release) = pause_pair();
    let append_writer = writer.clone();
    let append = tokio::spawn(async move {
        append_writer
            .append_marker_from_head(
                "effect-race",
                predecessor,
                "in-flight",
                b"opaque in-flight append",
                Some(append_pause),
            )
            .await
    });
    append_entered
        .await
        .expect("append holds writer/head locks");

    let (promotion_started_tx, promotion_started_rx) = oneshot::channel();
    let promotion_topology = topology.clone();
    let promotion = tokio::spawn(async move {
        promotion_topology
            .promote_with_probe(
                stale_candidate,
                1,
                "writer-racing-promotion",
                Some(promotion_started_tx),
            )
            .await
    });
    promotion_started_rx.await.expect("promotion started");
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(
        !promotion.is_finished(),
        "promotion must wait behind the in-flight writer fence lock"
    );

    append_release.send(()).expect("release append");
    assert_eq!(
        append
            .await
            .expect("join append")
            .expect("append commits")
            .head()
            .sequence(),
        2
    );
    assert!(matches!(
        promotion.await.expect("join promotion"),
        Err(PrototypeError::AncestorCandidate { .. })
    ));
    assert_eq!(topology.writer_generation().await.expect("generation"), 1);
    writer
        .append_marker("effect-race", "after-race", b"after race")
        .await
        .expect("original writer remains current after rejected promotion");

    topology.cleanup().await.expect("clean topology");
}

#[tokio::test]
async fn destination_generation_fences_delayed_old_enqueue() {
    let (topology, writer) = TestTopology::new(3).await.expect("create topology");
    writer
        .seed_destination_effect("effect-delayed", b"semantic-delayed")
        .await
        .expect("seed effect");
    let old_request = writer
        .authorization_request("effect-delayed", "old-authorization", b"opaque auth")
        .await
        .expect("prepare old authorization");
    let (_, old_witness) = require_new_authorization(
        writer
            .authorize_effect(
                &old_request,
                CommitAcknowledgement::DirectlyObserved,
                AuthorizationFailurePoint::None,
            )
            .await
            .expect("authorize old generation"),
    );
    let exact = topology
        .snapshot_clone(&writer)
        .await
        .expect("clone authorized effect");
    let promoted = topology
        .promote(exact, 1, "writer-new-generation")
        .await
        .expect("promote");

    assert!(matches!(
        writer.enter_destination(old_witness, None).await,
        Err(PrototypeError::DestinationGenerationFenced)
    ));
    assert_eq!(
        topology
            .destination_mutation_count("effect-delayed")
            .await
            .expect("destination count"),
        0
    );
    assert!(matches!(
        writer
            .append_marker("effect-delayed", "stale-instance", b"stale")
            .await,
        Err(PrototypeError::WriterFenced)
    ));

    let new_request = promoted
        .authorization_request("effect-delayed", "new-authorization", b"opaque auth")
        .await
        .expect("prepare new-generation authorization");
    let (_, new_witness) = require_new_authorization(
        promoted
            .authorize_effect(
                &new_request,
                CommitAcknowledgement::DirectlyObserved,
                AuthorizationFailurePoint::None,
            )
            .await
            .expect("authorize new generation"),
    );
    assert!(matches!(
        promoted
            .enter_destination(new_witness, None)
            .await
            .expect("enter new generation"),
        DestinationMutation::NewlyApplied { .. }
    ));

    topology.cleanup().await.expect("clean topology");
}

#[tokio::test]
async fn promotion_waits_for_entry_holding_destination_fence() {
    let (topology, writer) = TestTopology::new(2).await.expect("create topology");
    writer
        .seed_destination_effect("effect-held-fence", b"semantic-held-fence")
        .await
        .expect("seed effect");
    let request = writer
        .authorization_request(
            "effect-held-fence",
            "held-fence-authorization",
            b"opaque held fence authorization",
        )
        .await
        .expect("prepare authorization");
    let (_, witness) = require_new_authorization(
        writer
            .authorize_effect(
                &request,
                CommitAcknowledgement::DirectlyObserved,
                AuthorizationFailurePoint::None,
            )
            .await
            .expect("authorize"),
    );
    let candidate = topology
        .snapshot_clone(&writer)
        .await
        .expect("clone candidate");
    let (fence_pause, fence_entered, fence_release) = pause_pair();
    let entry_writer = writer.clone();
    let entry = tokio::spawn(async move {
        entry_writer
            .enter_destination_holding_fence(witness, fence_pause)
            .await
    });
    fence_entered
        .await
        .expect("entry acquired shared destination fence");

    let (promotion_started_tx, promotion_started_rx) = oneshot::channel();
    let promotion_topology = topology.clone();
    let promotion = tokio::spawn(async move {
        promotion_topology
            .promote_with_probe(
                candidate,
                1,
                "writer-after-held-destination-fence",
                Some(promotion_started_tx),
            )
            .await
    });
    promotion_started_rx.await.expect("promotion started");
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(
        !promotion.is_finished(),
        "promotion must wait for target entry holding the shared fence"
    );

    fence_release.send(()).expect("release target entry");
    assert!(matches!(
        entry
            .await
            .expect("join target entry")
            .expect("target entry commits"),
        DestinationMutation::NewlyApplied { .. }
    ));
    let promoted = timeout(Duration::from_secs(2), promotion)
        .await
        .expect("promotion completes after target entry")
        .expect("join promotion")
        .expect("promotion succeeds");
    assert_eq!(promoted.generation(), 2);
    assert_eq!(
        topology
            .destination_mutation_count("effect-held-fence")
            .await
            .expect("destination mutation count"),
        1
    );

    topology.cleanup().await.expect("clean topology");
}

#[tokio::test]
async fn account_inventory_survives_concurrency_crash_and_restore() {
    let (topology, writer) = TestTopology::new(3).await.expect("create topology");
    let mut requests = Vec::new();
    for index in 0..4 {
        let effect_id = format!("inventory-effect-{index}");
        writer
            .seed_destination_effect(&effect_id, format!("semantic-inventory-{index}").as_bytes())
            .await
            .expect("seed inventory effect");
        let request = writer
            .authorization_request(
                &effect_id,
                &format!("inventory-authorization-{index}"),
                b"opaque inventory authorization",
            )
            .await
            .expect("prepare inventory authorization");
        requests.push((index, request));
    }

    let mut authorization_tasks = Vec::new();
    for (index, request) in requests.drain(..3) {
        let independent = writer
            .independent_pool_writer()
            .await
            .expect("open independent inventory pool");
        authorization_tasks.push(tokio::spawn(async move {
            let outcome = independent
                .authorize_effect(
                    &request,
                    CommitAcknowledgement::DirectlyObserved,
                    AuthorizationFailurePoint::None,
                )
                .await;
            (index, outcome)
        }));
    }
    let mut witnesses = Vec::new();
    for task in authorization_tasks {
        let (index, outcome) = task.await.expect("join inventory authorization");
        let (_, witness) = require_new_authorization(outcome.expect("authorize inventory effect"));
        witnesses.push((index, witness));
    }
    witnesses.sort_by_key(|(index, _)| *index);
    let (_, exhausted_request) = requests.pop().expect("fourth inventory request");
    assert!(matches!(
        writer
            .authorize_effect(
                &exhausted_request,
                CommitAcknowledgement::DirectlyObserved,
                AuthorizationFailurePoint::None,
            )
            .await,
        Err(PrototypeError::InventoryExhausted)
    ));
    assert_eq!(
        topology
            .allocated_accounts(&writer)
            .await
            .expect("allocated accounts"),
        vec![10_001, 10_002, 10_003]
    );

    let (_, first_witness) = witnesses.remove(0);
    let first_attempt_id = first_witness.attempt_id().to_vec();
    let same_request = writer
        .authorization_request(
            "inventory-effect-0",
            "inventory-authorization-same",
            b"opaque inventory authorization retry",
        )
        .await
        .expect("prepare same destination authorization");
    let (_, same_witness) = require_new_authorization(
        writer
            .authorize_effect(
                &same_request,
                CommitAcknowledgement::DirectlyObserved,
                AuthorizationFailurePoint::None,
            )
            .await
            .expect("authorize same destination retry"),
    );
    let same_attempt_id = same_witness.attempt_id().to_vec();
    let left_writer = writer
        .independent_pool_writer()
        .await
        .expect("open destination left pool");
    let right_writer = writer
        .independent_pool_writer()
        .await
        .expect("open destination right pool");
    let barrier = Arc::new(Barrier::new(3));
    let left_barrier = Arc::clone(&barrier);
    let left = tokio::spawn(async move {
        left_barrier.wait().await;
        left_writer.enter_destination(first_witness, None).await
    });
    let right_barrier = Arc::clone(&barrier);
    let right = tokio::spawn(async move {
        right_barrier.wait().await;
        right_writer.enter_destination(same_witness, None).await
    });
    barrier.wait().await;
    let left = left.await.expect("join destination left");
    let right = right.await.expect("join destination right");
    let (effect_zero_account, first_receipt, second_receipt) = match (left, right) {
        (
            Ok(DestinationMutation::NewlyApplied { receipt }),
            Ok(DestinationMutation::ExistingSame {
                receipt: existing_receipt,
            }),
        )
        | (
            Ok(DestinationMutation::ExistingSame {
                receipt: existing_receipt,
            }),
            Ok(DestinationMutation::NewlyApplied { receipt }),
        ) if receipt.account_id() == existing_receipt.account_id() => {
            (receipt.account_id(), receipt, existing_receipt)
        }
        (left, right) => {
            panic!("same enqueue must converge to new plus existing: {left:?}, {right:?}")
        }
    };
    let mut returned_attempt_ids = vec![
        first_receipt.attempt_id().to_vec(),
        second_receipt.attempt_id().to_vec(),
    ];
    returned_attempt_ids.sort();
    let mut expected_attempt_ids = vec![first_attempt_id, same_attempt_id];
    expected_attempt_ids.sort();
    assert_eq!(returned_attempt_ids, expected_attempt_ids);

    let mut entered_accounts = vec![effect_zero_account];
    for (_, witness) in witnesses {
        entered_accounts.push(
            writer
                .enter_destination(witness, None)
                .await
                .expect("enter allocated destination")
                .account_id(),
        );
    }
    entered_accounts.sort_unstable();
    assert_eq!(entered_accounts, vec![10_001, 10_002, 10_003]);

    let conflict_request = writer
        .authorization_request(
            "inventory-effect-0",
            "inventory-authorization-conflict",
            b"opaque conflicting destination authorization",
        )
        .await
        .expect("prepare conflicting destination authorization");
    let (_, conflict_witness) = require_new_authorization(
        writer
            .authorize_effect(
                &conflict_request,
                CommitAcknowledgement::DirectlyObserved,
                AuthorizationFailurePoint::None,
            )
            .await
            .expect("authorize conflicting destination attempt"),
    );
    assert!(matches!(
        writer
            .enter_destination(conflict_witness.into_conflicting_request(), None)
            .await,
        Err(PrototypeError::DestinationConflict)
    ));

    let delayed_request = writer
        .authorization_request(
            "inventory-effect-0",
            "inventory-authorization-delayed",
            b"opaque delayed destination authorization",
        )
        .await
        .expect("prepare delayed destination authorization");
    let (_, delayed_witness) = require_new_authorization(
        writer
            .authorize_effect(
                &delayed_request,
                CommitAcknowledgement::DirectlyObserved,
                AuthorizationFailurePoint::None,
            )
            .await
            .expect("authorize delayed destination attempt"),
    );
    let observed_request = writer
        .authorization_request(
            "inventory-effect-0",
            "inventory-authorization-observed",
            b"opaque observed destination authorization",
        )
        .await
        .expect("prepare observed destination authorization");
    let (_, observed_witness) = require_new_authorization(
        writer
            .authorize_effect(
                &observed_request,
                CommitAcknowledgement::DirectlyObserved,
                AuthorizationFailurePoint::None,
            )
            .await
            .expect("authorize observed destination attempt"),
    );
    writer
        .append_observation(first_receipt)
        .await
        .expect("record first exact destination receipt");
    writer
        .append_observation(second_receipt)
        .await
        .expect("record second exact destination receipt");
    let observed_destination = writer
        .enter_destination(observed_witness, None)
        .await
        .expect("enter later exact destination attempt");
    assert!(matches!(
        &observed_destination,
        DestinationMutation::ExistingSame { .. }
    ));
    writer
        .append_observation(observed_destination.into_receipt())
        .await
        .expect("record later exact destination receipt");
    assert_eq!(
        Executor::new(writer.clone())
            .drive_effect("inventory-effect-0", CrashPoint::Never)
            .await
            .expect("observe and tombstone destination effect"),
        DriveOutcome::Completed
    );
    assert!(matches!(
        writer.enter_destination(delayed_witness, None).await,
        Err(PrototypeError::DestinationConflict)
    ));

    assert_eq!(
        topology
            .destination_mutation_count("inventory-effect-0")
            .await
            .expect("single mutation"),
        1
    );
    assert_eq!(
        topology
            .allocated_accounts(&writer)
            .await
            .expect("stable inventory"),
        vec![10_001, 10_002, 10_003]
    );

    let exact = topology
        .snapshot_clone(&writer)
        .await
        .expect("clone after permanent allocation");
    let promoted = topology
        .promote(exact, 1, "writer-inventory-restore")
        .await
        .expect("promote exact restore");
    assert_eq!(
        topology
            .allocated_accounts(&promoted)
            .await
            .expect("restore retains permanent inventory"),
        vec![10_001, 10_002, 10_003]
    );

    topology.cleanup().await.expect("clean topology");
}

#[tokio::test]
async fn account_sequence_policy_is_monotonic_across_concurrency_crash_and_restore() {
    let (topology, writer) = TestTopology::new(2).await.expect("create topology");
    let account_key = "account-sequence:sender-a";
    writer
        .seed_account_sequence(account_key, 41)
        .await
        .expect("seed account sequence");
    for index in 0..5 {
        writer
            .seed_account_sequence_effect(
                &format!("sequence-effect-{index}"),
                account_key,
                format!("semantic-sequence-{index}").as_bytes(),
            )
            .await
            .expect("seed sequence effect");
    }

    assert_eq!(
        writer
            .allocate_account_sequence(
                "sequence-effect-0",
                account_key,
                SequenceAcknowledgement::LostAfterCommit,
            )
            .await
            .expect("commit sequence before lost acknowledgement"),
        SequenceAllocationOutcome::OutcomeUnknown
    );
    assert_eq!(
        writer
            .allocate_account_sequence(
                "sequence-effect-0",
                account_key,
                SequenceAcknowledgement::DirectlyObserved,
            )
            .await
            .expect("reconcile sequence allocation"),
        SequenceAllocationOutcome::ExistingSame { sequence: 41 }
    );
    assert!(matches!(
        writer
            .allocate_account_sequence(
                "sequence-effect-0",
                "account-sequence:different-sender",
                SequenceAcknowledgement::DirectlyObserved,
            )
            .await,
        Err(PrototypeError::SequenceAllocationConflict)
    ));

    let left_writer = writer
        .independent_pool_writer()
        .await
        .expect("open sequence left pool");
    let right_writer = writer
        .independent_pool_writer()
        .await
        .expect("open sequence right pool");
    let barrier = Arc::new(Barrier::new(3));
    let left_barrier = Arc::clone(&barrier);
    let left = tokio::spawn(async move {
        left_barrier.wait().await;
        left_writer
            .allocate_account_sequence(
                "sequence-effect-1",
                account_key,
                SequenceAcknowledgement::DirectlyObserved,
            )
            .await
    });
    let right_barrier = Arc::clone(&barrier);
    let right = tokio::spawn(async move {
        right_barrier.wait().await;
        right_writer
            .allocate_account_sequence(
                "sequence-effect-2",
                account_key,
                SequenceAcknowledgement::DirectlyObserved,
            )
            .await
    });
    barrier.wait().await;
    let mut concurrent = vec![
        left.await
            .expect("join sequence left")
            .expect("allocate sequence left"),
        right
            .await
            .expect("join sequence right")
            .expect("allocate sequence right"),
    ];
    concurrent.sort_by_key(|outcome| match outcome {
        SequenceAllocationOutcome::NewlyAllocated { sequence }
        | SequenceAllocationOutcome::ExistingSame { sequence } => *sequence,
        SequenceAllocationOutcome::OutcomeUnknown => i64::MAX,
    });
    assert_eq!(
        concurrent,
        vec![
            SequenceAllocationOutcome::NewlyAllocated { sequence: 42 },
            SequenceAllocationOutcome::NewlyAllocated { sequence: 43 },
        ]
    );

    let left_writer = writer
        .independent_pool_writer()
        .await
        .expect("open same-sequence left pool");
    let right_writer = writer
        .independent_pool_writer()
        .await
        .expect("open same-sequence right pool");
    let barrier = Arc::new(Barrier::new(3));
    let left_barrier = Arc::clone(&barrier);
    let left = tokio::spawn(async move {
        left_barrier.wait().await;
        left_writer
            .allocate_account_sequence(
                "sequence-effect-3",
                account_key,
                SequenceAcknowledgement::DirectlyObserved,
            )
            .await
    });
    let right_barrier = Arc::clone(&barrier);
    let right = tokio::spawn(async move {
        right_barrier.wait().await;
        right_writer
            .allocate_account_sequence(
                "sequence-effect-3",
                account_key,
                SequenceAcknowledgement::DirectlyObserved,
            )
            .await
    });
    barrier.wait().await;
    let left = left.await.expect("join same-sequence left");
    let right = right.await.expect("join same-sequence right");
    assert!(matches!(
        (left, right),
        (
            Ok(SequenceAllocationOutcome::NewlyAllocated { sequence: 44 }),
            Ok(SequenceAllocationOutcome::ExistingSame { sequence: 44 }),
        ) | (
            Ok(SequenceAllocationOutcome::ExistingSame { sequence: 44 }),
            Ok(SequenceAllocationOutcome::NewlyAllocated { sequence: 44 }),
        )
    ));

    for index in 0..4 {
        let effect_id = format!("sequence-effect-{index}");
        let request = writer
            .authorization_request(
                &effect_id,
                &format!("sequence-authorization-{index}"),
                b"opaque sequence authorization",
            )
            .await
            .expect("prepare sequence authorization");
        assert!(matches!(
            writer
                .authorize_effect(
                    &request,
                    CommitAcknowledgement::DirectlyObserved,
                    AuthorizationFailurePoint::None,
                )
                .await
                .expect("authorize preallocated sequence effect"),
            AuthorizationOutcome::NewlyAppended { commit, .. }
                if commit.head().sequence() == 3
        ));
    }

    let exact = topology
        .snapshot_clone(&writer)
        .await
        .expect("clone after sequence allocations");
    let promoted = topology
        .promote(exact, 1, "writer-sequence-restore")
        .await
        .expect("promote sequence restore");
    assert!(matches!(
        writer
            .allocate_account_sequence(
                "sequence-effect-4",
                account_key,
                SequenceAcknowledgement::DirectlyObserved,
            )
            .await,
        Err(PrototypeError::WriterFenced)
    ));
    assert_eq!(
        promoted
            .allocate_account_sequence(
                "sequence-effect-0",
                account_key,
                SequenceAcknowledgement::DirectlyObserved,
            )
            .await
            .expect("restore retains first allocation"),
        SequenceAllocationOutcome::ExistingSame { sequence: 41 }
    );
    assert_eq!(
        promoted
            .allocate_account_sequence(
                "sequence-effect-4",
                account_key,
                SequenceAcknowledgement::DirectlyObserved,
            )
            .await
            .expect("allocate after restore"),
        SequenceAllocationOutcome::NewlyAllocated { sequence: 45 }
    );
    let allocations = topology
        .account_sequence_allocations(&promoted, account_key)
        .await
        .expect("load sequence allocations");
    assert_eq!(
        allocations
            .iter()
            .map(|(_, sequence)| *sequence)
            .collect::<Vec<_>>(),
        vec![41, 42, 43, 44, 45]
    );
    let mut effects = allocations
        .iter()
        .map(|(effect_id, _)| effect_id.as_str())
        .collect::<Vec<_>>();
    effects.sort_unstable();
    assert_eq!(
        effects,
        vec![
            "sequence-effect-0",
            "sequence-effect-1",
            "sequence-effect-2",
            "sequence-effect-3",
            "sequence-effect-4",
        ]
    );

    topology.cleanup().await.expect("clean topology");
}

#[tokio::test]
async fn secret_sentinels_never_reach_retained_or_error_surfaces() {
    let (topology, writer) = TestTopology::new(2).await.expect("create topology");
    let sentinel = [
        b"Bearer ".as_slice(),
        b"credential-prototype-".as_slice(),
        &[0x00, 0xff, 0x81, 0x10],
    ]
    .concat();
    writer
        .seed_destination_effect("effect-secret-sentinel", b"reviewed semantic request")
        .await
        .expect("seed secret sentinel effect");
    let request = writer
        .authorization_request(
            "effect-secret-sentinel",
            "secret-sentinel-authorization",
            b"reviewed public authorization",
        )
        .await
        .expect("prepare reviewed authorization");
    let (_, witness) = require_new_authorization(
        writer
            .authorize_effect(
                &request,
                CommitAcknowledgement::DirectlyObserved,
                AuthorizationFailurePoint::None,
            )
            .await
            .expect("authorize reviewed request"),
    );

    let rejected = writer
        .rejected_credential_probe(&sentinel)
        .expect_err("credential rejection is redacted");
    let error_surfaces = [format!("{rejected}"), format!("{rejected:?}")];
    let forbidden = forbidden_encodings(&sentinel);
    assert!(error_surfaces.iter().all(|surface| {
        forbidden
            .iter()
            .all(|needle| !contains_bytes(surface.as_bytes(), needle))
    }));

    let _destination = writer
        .enter_destination_with_ephemeral_credential(witness, &sentinel)
        .await
        .expect("credential remains below retained destination boundary");
    topology
        .assert_retained_surfaces_exclude(&sentinel)
        .await
        .expect("no secret or fingerprint in PostgreSQL rows");
    for fixture in [
        include_bytes!("recoverability_postgres_executor_prototype.rs").as_slice(),
        include_bytes!("recoverability_postgres_executor_prototype/support/mod.rs").as_slice(),
    ] {
        assert!(
            forbidden
                .iter()
                .all(|needle| !contains_bytes(fixture, needle)),
            "the adversarial sentinel or its fingerprint must not become a fixture constant"
        );
    }

    topology.cleanup().await.expect("clean topology");
}

#[tokio::test]
async fn destination_io_wait_holds_no_ledger_lock() {
    let (topology, writer) = TestTopology::new(2).await.expect("create topology");
    writer
        .seed_destination_effect("effect-io", b"semantic-io")
        .await
        .expect("seed effect");
    let probe_head = writer
        .seed_effect("effect-io-ledger-probe", None, b"semantic io ledger probe")
        .await
        .expect("seed independent ledger probe");
    let request = writer
        .authorization_request("effect-io", "authorization-io", b"opaque auth")
        .await
        .expect("prepare authorization");
    let (authorization_head, witness) = require_new_authorization(
        writer
            .authorize_effect(
                &request,
                CommitAcknowledgement::DirectlyObserved,
                AuthorizationFailurePoint::None,
            )
            .await
            .expect("authorize"),
    );
    let (io_pause, io_entered, io_release) = pause_pair();
    let destination = writer.clone();
    let destination_task =
        tokio::spawn(async move { destination.enter_destination(witness, Some(io_pause)).await });
    io_entered.await.expect("destination IO wait entered");

    let append = timeout(
        Duration::from_millis(750),
        writer.append_marker_from_head(
            "effect-io-ledger-probe",
            probe_head,
            "marker-during-io",
            b"opaque marker",
            None,
        ),
    )
    .await
    .expect("ledger append must not wait for destination IO")
    .expect("ledger append succeeds");
    assert_eq!(append.head().sequence(), 2);
    assert_eq!(authorization_head.sequence(), 3);

    io_release.send(()).expect("release destination IO");
    assert!(matches!(
        destination_task
            .await
            .expect("join destination")
            .expect("destination mutation"),
        DestinationMutation::NewlyApplied { .. }
    ));

    topology.cleanup().await.expect("clean topology");
}
