//! Hostile persisted-history qualification over a real certified program.
//!
//! Every fixture here admits a genuine run through the production writer and
//! then forges its exact raw prefix. Program trust is the real certification
//! owner, so a forgery is rejected by the same rule production uses — there is
//! no permissive verifier and no synthetic certification document anywhere in
//! this suite.

use std::sync::Arc;

use mfm_ids::{AppendRequestId, ContentRef, InvocationIdentity};
use mfm_journal::structured::{CommitCandidate, RunRecord};
use mfm_store::structured::test_support::{
    forge_wrong_admission_genesis, qualify_recorded_structure_only,
    verify_incremental_reduction_equivalence, LiveFixtureStore,
};
use mfm_store::structured::{verify_offline_recorded_history, RawRunHistory, StructuredStoreError};

/// Admits the fixture run and returns its exact raw persisted prefix.
async fn admitted_prefix(
    store: &LiveFixtureStore,
    discriminator: u8,
    label: &str,
) -> RawRunHistory {
    store
        .admit(discriminator, label)
        .await
        .expect("admit fixture run");
    store.raw_prefix().await.expect("raw admitted prefix")
}

/// Requalifies one raw prefix through the sole offline entry point.
fn requalify(store: &LiveFixtureStore, raw: RawRunHistory) -> Result<(), StructuredStoreError> {
    verify_offline_recorded_history(
        raw,
        &store.program_qualifier,
        store.physical_binding_verifier().as_ref(),
    )
    .map(|_| ())
}

/// Rebuilds the last batch after mutating its records, keeping the envelope
/// self-consistent so the forgery is rejected on semantics, not on framing.
fn forge_last_batch(
    store: &LiveFixtureStore,
    mut raw: RawRunHistory,
    append_id: &str,
    mutate: impl FnOnce(&mut Vec<RunRecord>, &mut Vec<mfm_journal::structured::HistoryObject>),
) -> RawRunHistory {
    let original = raw.batches.pop().expect("forgeable batch");
    let mut records = original
        .records
        .iter()
        .map(|assigned| assigned.record.clone())
        .collect::<Vec<_>>();
    let mut objects = original.objects;
    mutate(&mut records, &mut objects);
    objects.sort_by(|left, right| left.content_ref.cmp(&right.content_ref));
    let expected_head = raw.batches.last().map(|batch| batch.head.clone());
    let forged = mfm_store::structured::test_support::assign_hostile_candidate(
        &store.identity,
        CommitCandidate {
            run_id: raw.run_id.clone(),
            expected_head,
            append_request_id: AppendRequestId::new(append_id).expect("forged append id"),
            tenant_fact_coordinate: original.tenant_fact_coordinate,
            records,
            objects,
        },
    )
    .expect("well-formed hostile envelope");
    raw.batches.push(forged);
    raw
}

/// An unforged admitted prefix qualifies, which anchors every rejection below.
#[tokio::test]
async fn a_real_admitted_prefix_qualifies() {
    let store = LiveFixtureStore::open(70);
    let raw = admitted_prefix(&store, 70, "anchor").await;
    requalify(&store, raw).expect("the unforged admitted prefix qualifies");
}

/// A forgery that keeps the envelope and the admitted record in agreement is
/// still rejected, because run identity is derived rather than trusted.
#[tokio::test]
async fn a_self_consistent_forged_run_identity_fails_qualification() {
    let store = LiveFixtureStore::open(71);
    let raw = admitted_prefix(&store, 71, "forged-identity").await;
    requalify(&store, raw.clone()).expect("the unforged admitted prefix qualifies");

    let forged = forge_last_batch(&store, raw, "forged-identity", |records, _| {
        let RunRecord::RunAdmitted(admission) = &mut records[0] else {
            panic!("first record is the admission")
        };
        admission.invocation_identity =
            InvocationIdentity::new("00000000-0000-4000-8000-000000000002")
                .expect("foreign invocation");
    });
    let RunRecord::RunAdmitted(forged_admission) = &forged.batches[0].records[0].record else {
        panic!("forged admission record")
    };
    assert_eq!(
        forged_admission.run_id, forged.run_id,
        "the forgery keeps the envelope and the admitted record in agreement",
    );
    assert_eq!(
        requalify(&store, forged),
        Err(StructuredStoreError::InvalidHistory),
        "a derived run identity cannot be satisfied by internal agreement",
    );
}

/// Omitting an object the closure requires is rejected even though every
/// remaining record and hash is internally consistent.
#[tokio::test]
async fn an_omitted_required_object_fails_qualification() {
    let store = LiveFixtureStore::open(72);
    let raw = admitted_prefix(&store, 72, "omitted-object").await;
    requalify(&store, raw.clone()).expect("the unforged admitted prefix qualifies");

    let omitted: ContentRef = raw.batches[0]
        .objects
        .first()
        .expect("admitted object")
        .content_ref
        .clone();
    let forged = forge_last_batch(&store, raw, "omitted-object", |_, objects| {
        objects.retain(|object| object.content_ref != omitted);
    });
    assert_eq!(
        requalify(&store, forged),
        Err(StructuredStoreError::InvalidHistory),
        "the introduced-object closure is exact, not a lower bound",
    );
}

/// An admission naming a different retained program object is rejected, even
/// though the substituted object is genuinely present in the same batch.
#[tokio::test]
async fn a_substituted_certified_program_reference_fails_qualification() {
    let store = LiveFixtureStore::open(73);
    let raw = admitted_prefix(&store, 73, "substituted-program").await;
    requalify(&store, raw.clone()).expect("the unforged admitted prefix qualifies");

    let RunRecord::RunAdmitted(admitted) = &raw.batches[0].records[0].record else {
        panic!("first record is the admission")
    };
    let original = admitted.certified_program_ref.clone();
    let foreign = raw.batches[0]
        .objects
        .iter()
        .map(|object| object.content_ref.clone())
        .find(|content_ref| content_ref != &original)
        .expect("a second retained object to substitute");
    let forged = forge_last_batch(&store, raw, "substituted-program", |records, _| {
        let RunRecord::RunAdmitted(admission) = &mut records[0] else {
            panic!("first record is the admission")
        };
        admission.certified_program_ref = foreign;
    });
    // The retained value is not the certified root owner, so concrete program
    // certification rejects it.
    assert_eq!(
        requalify(&store, forged),
        Err(StructuredStoreError::Certification),
        "program trust is established from the exact retained root, not a present object",
    );
}

/// A typed, hash-valid batch can cross qualification while reduction still
/// rejects an impossible semantic digest.
#[tokio::test]
async fn qualification_does_not_claim_semantic_legality() {
    let store = LiveFixtureStore::open(74);
    let raw = admitted_prefix(&store, 74, "semantic-boundary").await;
    let forged = forge_wrong_admission_genesis(&store.identity, raw)
        .expect("well-typed wrong semantic digest");
    qualify_recorded_structure_only(forged.clone(), &store.program_qualifier)
        .expect("structural qualification remains honest");
    assert_eq!(
        requalify(&store, forged),
        Err(StructuredStoreError::InvalidHistory),
        "the sole reducer rejects the semantically impossible digest",
    );
}

/// Incremental advancement is value-equal to complete replay at every prefix.
#[tokio::test]
async fn incremental_reduction_matches_complete_replay_at_every_prefix() {
    let store = LiveFixtureStore::open(81);
    admitted_prefix(&store, 81, "incremental").await;
    store.drive_once().await.expect("terminal pure transition");
    let raw = store.raw_prefix().await.expect("complete prefix");
    assert!(raw.batches.len() > 1, "the evidence crosses a successor");
    verify_incremental_reduction_equivalence(
        raw,
        &store.program_qualifier,
        store.physical_binding_verifier().as_ref(),
    )
    .expect("incremental and complete replay are exactly equal");
}

/// Program trust for a different certified program rejects this run.
#[tokio::test]
async fn an_unrelated_certified_program_cannot_qualify_this_run() {
    let store = LiveFixtureStore::open(75);
    let raw = admitted_prefix(&store, 75, "foreign-program").await;
    let unrelated = mfm_store::structured::test_support::unrelated_program_qualifier();
    assert_eq!(
        verify_offline_recorded_history(
            raw,
            &unrelated,
            store.physical_binding_verifier().as_ref(),
        )
        .map(|_| ()),
        Err(StructuredStoreError::Certification),
        "a nominal entry point is not a trust key",
    );
}

/// The same admission request is idempotent and never mints a second run.
#[tokio::test]
async fn an_exactly_repeated_admission_is_the_same_committed_run() {
    let store = LiveFixtureStore::open(76);
    let first = store
        .admit(76, "idempotent")
        .await
        .expect("first admission");
    let second = store
        .admit(76, "idempotent")
        .await
        .expect("repeated admission");
    assert_eq!(
        first, second,
        "an exactly repeated append request resolves to the same candidate",
    );
    let verified = store.load_verified().await.expect("verified run");
    assert_eq!(verified.run_id(), &store.run_id);
}

/// A run this store never admitted has no verified state to load.
#[tokio::test]
async fn an_unadmitted_run_has_no_verified_state() {
    let store = LiveFixtureStore::open(77);
    assert!(
        store.load_verified().await.is_err(),
        "an unadmitted run cannot produce verified state",
    );
    let _ = Arc::clone(&store.program_qualifier);
}

/// The attention inventory is empty until a run actually blocks on Effect entry,
/// and its pagination contract is exact.
#[tokio::test]
async fn current_attention_lists_only_runs_that_currently_block() {
    let store = LiveFixtureStore::open(78);
    store.admit(78, "attention").await.expect("admit run");

    let page = store
        .list_effect_entry_attention(&store.tenant_scope_id(), None, 16)
        .await
        .expect("attention page");

    // The fixture program is a pure copy state, so nothing blocks on entry.
    assert!(
        page.entries().is_empty(),
        "a run with no unresolved Effect entry is not attention",
    );
    assert_eq!(
        page.next_after_run_id(),
        None,
        "an exhausted sweep has no continuation key",
    );
}

/// A zero limit is refused rather than silently treated as unbounded.
#[tokio::test]
async fn a_zero_attention_page_limit_is_rejected() {
    let store = LiveFixtureStore::open(79);
    store.admit(79, "attention-limit").await.expect("admit run");
    assert!(
        store
            .list_effect_entry_attention(&store.tenant_scope_id(), None, 0)
            .await
            .is_err(),
        "a zero page limit is not a valid sweep",
    );
}

/// Another tenant's sweep never observes this tenant's runs.
#[tokio::test]
async fn attention_sweeps_are_tenant_isolated() {
    let store = LiveFixtureStore::open(80);
    store
        .admit(80, "attention-tenant")
        .await
        .expect("admit run");
    let foreign = mfm_ids::TenantScopeId::new(format!(
        "{}{}",
        mfm_ids::TenantScopeId::PREFIX,
        "c".repeat(32)
    ))
    .expect("foreign tenant");
    let page = store
        .list_effect_entry_attention(&foreign, None, 16)
        .await
        .expect("foreign tenant page");
    assert!(
        page.entries().is_empty(),
        "a sweep is bound to the tenant it names",
    );
}
