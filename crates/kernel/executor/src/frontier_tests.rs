use mfm_canonical::sha256_digest_bytes;
use mfm_ids::{EffectKey, RequestDigest, SemanticDigest, TenantScopeId};

use super::*;
use crate::{ExecutorBinding, ExecutorDeployment, ReferenceFailureCode};

fn reviewed_ref(label: &str) -> ContentRef {
    let value = encode(
        "mfm.primitive-stable_id.v1",
        &CanonicalValue::String(label.to_owned()),
    )
    .expect("stable value");
    content_ref(&value).expect("content ref")
}

fn effect_identity(label: &str) -> EffectIdentity {
    let tenant = TenantScopeId::new(format!("mfm.tenant_scope.v1:{:032x}", label.len() + 1))
        .expect("tenant");
    let deployment = ExecutorDeployment::new(
        reviewed_ref(&format!("{label}.namespace")),
        reviewed_ref(&format!("{label}.generation")),
        tenant.clone(),
        reviewed_ref(&format!("{label}.authority")),
        None,
    )
    .expect("deployment");
    let binding = ExecutorBinding::new(
        reviewed_ref(&format!("{label}.contract")),
        reviewed_ref(&format!("{label}.implementation")),
        deployment.reference().expect("deployment ref"),
    )
    .expect("binding");
    EffectIdentity::from_parts(
        tenant,
        binding.reference().expect("binding ref"),
        EffectKey::from_digest(sha256_digest_bytes(format!("{label}.effect").as_bytes())),
        RequestDigest::from_digest(sha256_digest_bytes(format!("{label}.request").as_bytes())),
    )
}

fn valid_audit() -> (EffectIdentity, EvidenceBounds, ContentRef, DeliveryAudit) {
    let identity = effect_identity("valid");
    let proof_ref = reviewed_ref("valid.proof");
    let bound = ExecutorEvidenceRecord::EffectBound {
        executor_binding_ref: identity.executor_binding_ref().clone(),
        effect_key: identity.effect_key().clone(),
        request_digest: identity.request_digest().clone(),
    };
    let first = DeliveryAuditFrontier::append(&identity, None, vec![bound], proof_ref.clone())
        .expect("first");
    let target_operation_ref = reviewed_ref("target.operation");
    let attempt_id = derive_attempt_id(&identity, 0, &target_operation_ref).expect("attempt");
    let second = DeliveryAuditFrontier::append(
        &identity,
        Some(first.reference().expect("first ref")),
        vec![ExecutorEvidenceRecord::DeliveryAttemptAuthorized {
            attempt_ordinal: 0,
            attempt_id: attempt_id.clone(),
            target_operation_ref,
        }],
        proof_ref.clone(),
    )
    .expect("second");
    let failure = reference_safe_failure(
        reviewed_ref("safe-failure.contract"),
        ReferenceFailureCode::UnclassifiedFailure,
        FailureClass::Unclassified,
        BoundaryStage::BoundaryObservation,
    )
    .expect("failure");
    let third = DeliveryAuditFrontier::append(
        &identity,
        Some(second.reference().expect("second ref")),
        vec![ExecutorEvidenceRecord::DeliveryAttemptObserved {
            attempt_id,
            outcome: DeliveryAttemptOutcome::indeterminate(failure).expect("outcome"),
        }],
        proof_ref.clone(),
    )
    .expect("third");
    let bounds = EvidenceBounds::new(4, 16, 500_000, 2, 10_000).expect("bounds");
    (
        identity,
        bounds,
        proof_ref,
        DeliveryAudit::from_ledger(vec![first, second, third]),
    )
}

#[test]
fn forged_proof_is_rejected_without_retaining_candidate() {
    let (identity, bounds, proof_ref, mut audit) = valid_audit();
    audit.verify(&identity, &bounds, &proof_ref).expect("valid");
    audit.frontiers[1].proof =
        FrontierProof(SemanticDigest::from_digest(sha256_digest_bytes(b"forged")));

    let mut accumulator = DeliveryAuditAccumulator::default();
    assert_eq!(
        accumulator
            .admit(audit, &identity, &bounds, &proof_ref)
            .expect_err("forged proof"),
        ExecutorError::InvalidFrontierProof
    );
    assert!(accumulator.greatest().is_none());
}

#[test]
fn omitted_predecessor_and_wrong_binding_are_rejected() {
    let (identity, bounds, proof_ref, audit) = valid_audit();

    let mut omitted = audit.clone();
    omitted.frontiers[1].predecessor_frontier_ref = None;
    assert_eq!(
        omitted
            .verify(&identity, &bounds, &proof_ref)
            .expect_err("omitted predecessor"),
        ExecutorError::InvalidFrontier
    );

    let mut wrong_binding = audit;
    wrong_binding.frontiers[2].executor_binding_ref =
        effect_identity("wrong").executor_binding_ref().clone();
    assert_eq!(
        wrong_binding
            .verify(&identity, &bounds, &proof_ref)
            .expect_err("wrong binding"),
        ExecutorError::InvalidFrontier
    );
}

#[test]
fn equal_and_ancestor_frontiers_do_not_hide_a_fork() {
    let (identity, bounds, proof_ref, audit) = valid_audit();
    let first = audit.frontiers[0].clone();
    let second = audit.frontiers[1].clone();
    let ExecutorEvidenceRecord::DeliveryAttemptAuthorized { attempt_id, .. } =
        &second.appended_records[0]
    else {
        panic!("authorized attempt");
    };
    let failure = reference_safe_failure(
        reviewed_ref("safe-failure.contract"),
        ReferenceFailureCode::RequestConflict,
        FailureClass::Destination,
        BoundaryStage::BoundaryObservation,
    )
    .expect("failure");
    let fork = DeliveryAuditFrontier::append(
        &identity,
        Some(second.reference().expect("second ref")),
        vec![ExecutorEvidenceRecord::DeliveryAttemptObserved {
            attempt_id: attempt_id.clone(),
            outcome: DeliveryAttemptOutcome::indeterminate(failure).expect("outcome"),
        }],
        proof_ref.clone(),
    )
    .expect("fork frontier");
    let forked = DeliveryAudit::from_ledger(vec![first, second, fork]);

    let mut accumulator = DeliveryAuditAccumulator::default();
    assert_eq!(
        accumulator
            .admit(audit.clone(), &identity, &bounds, &proof_ref)
            .expect("initial"),
        AdmitFrontier::Advanced
    );
    assert_eq!(
        accumulator
            .admit(
                DeliveryAudit::from_ledger(audit.frontiers[..2].to_vec()),
                &identity,
                &bounds,
                &proof_ref,
            )
            .expect("ancestor"),
        AdmitFrontier::StaleAncestor
    );
    assert_eq!(
        accumulator
            .admit(forked, &identity, &bounds, &proof_ref)
            .expect_err("fork"),
        ExecutorError::FrontierFork
    );
}

#[test]
fn completion_reserve_is_cumulative_for_unmatched_attempts() {
    let identity = effect_identity("reserve");
    let proof_ref = reviewed_ref("reserve.proof");
    let bounds = EvidenceBounds::new(3, 4, 1_000_000, 2, 100).expect("bounds");
    let first = DeliveryAuditFrontier::append(
        &identity,
        None,
        vec![ExecutorEvidenceRecord::EffectBound {
            executor_binding_ref: identity.executor_binding_ref().clone(),
            effect_key: identity.effect_key().clone(),
            request_digest: identity.request_digest().clone(),
        }],
        proof_ref.clone(),
    )
    .expect("bound");
    let operation = reviewed_ref("reserve.operation");
    let second = DeliveryAuditFrontier::append(
        &identity,
        Some(first.reference().expect("first ref")),
        vec![ExecutorEvidenceRecord::DeliveryAttemptAuthorized {
            attempt_ordinal: 0,
            attempt_id: derive_attempt_id(&identity, 0, &operation).expect("attempt"),
            target_operation_ref: operation.clone(),
        }],
        proof_ref.clone(),
    )
    .expect("first authorization");
    DeliveryAudit::from_ledger(vec![first.clone(), second.clone()])
        .verify(&identity, &bounds, &proof_ref)
        .expect("one observation plus tombstone fits");

    let third = DeliveryAuditFrontier::append(
        &identity,
        Some(second.reference().expect("second ref")),
        vec![ExecutorEvidenceRecord::DeliveryAttemptAuthorized {
            attempt_ordinal: 1,
            attempt_id: derive_attempt_id(&identity, 1, &operation).expect("attempt"),
            target_operation_ref: operation,
        }],
        proof_ref.clone(),
    )
    .expect("candidate frontier");
    assert_eq!(
        DeliveryAudit::from_ledger(vec![first, second, third])
            .verify(&identity, &bounds, &proof_ref)
            .expect_err("two observations plus tombstone do not fit"),
        ExecutorError::EvidenceBoundsExhausted
    );
}

#[test]
fn terminal_proof_must_name_the_exact_returned_observation() {
    let identity = effect_identity("terminal");
    let proof_ref = reviewed_ref("terminal.proof");
    let operation = reviewed_ref("terminal.operation");
    let attempt_id = derive_attempt_id(&identity, 0, &operation).expect("attempt");
    let safe_result = encode(
        "mfm.primitive-stable_id.v1",
        &CanonicalValue::String("applied".to_owned()),
    )
    .expect("safe result");
    let returned = ReturnedOutcome::new(
        SchemaQualifiedCanonicalValue::from_validated(&safe_result).expect("schema qualified"),
    )
    .expect("returned");
    let observed = ExecutorEvidenceRecord::DeliveryAttemptObserved {
        attempt_id: attempt_id.clone(),
        outcome: DeliveryAttemptOutcome::Returned(returned.clone()),
    };
    let observation_ref = observed
        .observed_content_ref()
        .expect("observation")
        .expect("observation ref");
    let wrong_attempt = derive_attempt_id(&identity, 1, &operation).expect("wrong attempt");
    let tombstone = TerminalTombstone::new(
        "terminal.effect",
        "applied",
        ReferenceTerminalProof::new(wrong_attempt, returned, observation_ref)
            .expect("proof object"),
    )
    .expect("tombstone object");

    let first = DeliveryAuditFrontier::append(
        &identity,
        None,
        vec![
            ExecutorEvidenceRecord::EffectBound {
                executor_binding_ref: identity.executor_binding_ref().clone(),
                effect_key: identity.effect_key().clone(),
                request_digest: identity.request_digest().clone(),
            },
            ExecutorEvidenceRecord::DeliveryAttemptAuthorized {
                attempt_ordinal: 0,
                attempt_id,
                target_operation_ref: operation,
            },
            observed,
            ExecutorEvidenceRecord::TerminalTombstone(tombstone),
        ],
        proof_ref.clone(),
    )
    .expect("frontier");
    let bounds = EvidenceBounds::new(4, 16, 500_000, 2, 10_000).expect("bounds");
    assert_eq!(
        DeliveryAudit::from_ledger(vec![first])
            .verify(&identity, &bounds, &proof_ref)
            .expect_err("wrong proof attempt"),
        ExecutorError::TerminalProofMismatch
    );
}
