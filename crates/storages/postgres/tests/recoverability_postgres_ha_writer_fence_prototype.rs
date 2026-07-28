#![cfg(feature = "parity-tests")]
//! Retained contract prototype for PostgreSQL writer fencing and restore admission.
//!
//! The fixture owns isolated schemas only. Its fence schema is deliberately not copied with a
//! store snapshot, so every simulated primary or restored clone consults one shared authority.
//! This is not the production store schema and does not claim to exercise physical replication.

#[path = "recoverability_postgres_ha_writer_fence_prototype/support.rs"]
mod support;

use support::{FenceError, StoreIdentity, TestTopology};

#[tokio::test]
async fn concurrent_generation_cas_fences_the_stale_primary() {
    let (topology, primary) =
        TestTopology::new(StoreIdentity::new("scope-original", "epoch-original"))
            .await
            .expect("create topology");

    assert_eq!(
        primary
            .append_journal("run-a", b"first")
            .await
            .expect("append first record"),
        1
    );

    let restore_a = topology
        .snapshot_clone(primary.replica(), "restore_a")
        .await
        .expect("clone restore a");
    let restore_b = topology
        .snapshot_clone(primary.replica(), "restore_b")
        .await
        .expect("clone restore b");
    let expected_generation = primary.generation();

    let (promotion_a, promotion_b) = tokio::join!(
        topology.promote(restore_a, expected_generation, "writer-a"),
        topology.promote(restore_b, expected_generation, "writer-b")
    );

    let promoted = match (promotion_a, promotion_b) {
        (Ok(promoted), Err(FenceError::GenerationChanged { expected, actual }))
        | (Err(FenceError::GenerationChanged { expected, actual }), Ok(promoted)) => {
            assert_eq!(expected, expected_generation);
            assert_eq!(actual, expected_generation + 1);
            promoted
        }
        (left, right) => panic!("one promotion must win the generation CAS: {left:?}, {right:?}"),
    };

    assert!(matches!(
        primary.append_journal("run-a", b"stale-primary").await,
        Err(FenceError::WriterFenced)
    ));
    assert_eq!(
        primary
            .replica()
            .journal_head_sequence("run-a")
            .await
            .expect("read stale primary"),
        Some(1),
        "a fenced writer remains readable but cannot append"
    );

    assert_eq!(
        promoted
            .append_journal("run-a", b"second")
            .await
            .expect("append through promoted writer"),
        2
    );
    assert!(matches!(
        topology
            .promote(
                primary.replica().clone(),
                promoted.generation(),
                "stale-primary-repromotion"
            )
            .await,
        Err(FenceError::JournalSuffixRolledBack { run_id }) if run_id == "run-a"
    ));

    topology.cleanup().await.expect("clean topology");
}

#[tokio::test]
async fn promotion_rejects_journal_suffix_and_tenant_head_rollback() {
    let (topology, primary) =
        TestTopology::new(StoreIdentity::new("scope-rollback", "epoch-rollback"))
            .await
            .expect("create topology");

    primary
        .append_journal("run-a", b"first")
        .await
        .expect("append first record");
    primary
        .publish_fact("tenant-a", b"first")
        .await
        .expect("publish first fact");

    let journal_stale = topology
        .snapshot_clone(primary.replica(), "journal_stale")
        .await
        .expect("clone before journal suffix");

    primary
        .append_journal("run-a", b"second")
        .await
        .expect("append second record");

    let fact_stale = topology
        .snapshot_clone(primary.replica(), "fact_stale")
        .await
        .expect("clone before fact publication");

    primary
        .publish_fact("tenant-a", b"second")
        .await
        .expect("publish second fact");

    let current = topology
        .snapshot_clone(primary.replica(), "current")
        .await
        .expect("clone current store");
    let expected_generation = primary.generation();

    assert!(matches!(
        topology
            .promote(journal_stale, expected_generation, "journal-stale")
            .await,
        Err(FenceError::JournalSuffixRolledBack { run_id }) if run_id == "run-a"
    ));
    assert!(matches!(
        topology
            .promote(fact_stale, expected_generation, "fact-stale")
            .await,
        Err(FenceError::TenantFactHeadRolledBack { tenant_id }) if tenant_id == "tenant-a"
    ));
    assert_eq!(
        topology
            .generation()
            .await
            .expect("read generation after rejected promotions"),
        expected_generation,
        "a rejected candidate must not advance the fence"
    );

    topology
        .promote(current, expected_generation, "current-writer")
        .await
        .expect("promote non-rollback clone");

    topology.cleanup().await.expect("clean topology");
}

#[tokio::test]
async fn restore_preserves_identity_and_reset_requires_unused_scope_and_epoch() {
    let initial_identity = StoreIdentity::new("scope-preserved", "epoch-preserved");
    let (topology, primary) = TestTopology::new(initial_identity.clone())
        .await
        .expect("create topology");
    primary
        .append_journal("run-a", b"retained")
        .await
        .expect("append retained record");

    let restored = topology
        .snapshot_clone(primary.replica(), "restored")
        .await
        .expect("clone store");
    assert_eq!(
        restored.identity().await.expect("read restored identity"),
        initial_identity
    );
    let restored_writer = topology
        .promote(restored, primary.generation(), "restored-writer")
        .await
        .expect("promote identity-preserving restore");

    let reused_scope = topology
        .empty_store(
            StoreIdentity::new("scope-preserved", "epoch-fresh-a"),
            "reused_scope",
        )
        .await
        .expect("create reused-scope candidate");
    assert!(matches!(
        topology
            .destructive_reset(
                reused_scope,
                restored_writer.generation(),
                "reused-scope-writer"
            )
            .await,
        Err(FenceError::StoreScopeAlreadyUsed)
    ));

    let reused_epoch = topology
        .empty_store(
            StoreIdentity::new("scope-fresh-a", "epoch-preserved"),
            "reused_epoch",
        )
        .await
        .expect("create reused-epoch candidate");
    assert!(matches!(
        topology
            .destructive_reset(
                reused_epoch,
                restored_writer.generation(),
                "reused-epoch-writer"
            )
            .await,
        Err(FenceError::StoreEpochAlreadyUsed)
    ));

    let changed_restore_identity = topology
        .empty_store(
            StoreIdentity::new("scope-preserved", "epoch-fresh-b"),
            "changed_restore_identity",
        )
        .await
        .expect("create changed-identity candidate");
    assert!(matches!(
        topology
            .promote(
                changed_restore_identity,
                restored_writer.generation(),
                "changed-identity-writer"
            )
            .await,
        Err(FenceError::StoreIdentityMismatch)
    ));

    let reset_identity = StoreIdentity::new("scope-reset", "epoch-reset");
    let reset_candidate = topology
        .empty_store(reset_identity.clone(), "reset")
        .await
        .expect("create reset candidate");
    let reset_writer = topology
        .destructive_reset(
            reset_candidate,
            restored_writer.generation(),
            "reset-writer",
        )
        .await
        .expect("reset with fresh identity");
    assert_eq!(
        reset_writer
            .replica()
            .identity()
            .await
            .expect("read reset identity"),
        reset_identity
    );

    assert!(matches!(
        topology
            .promote(
                primary.replica().clone(),
                reset_writer.generation(),
                "old-lineage-writer"
            )
            .await,
        Err(FenceError::StoreIdentityMismatch)
    ));

    topology.cleanup().await.expect("clean topology");
}
