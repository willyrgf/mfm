use mfm_canonical::sha256_digest_bytes;
use mfm_ids::{EffectKey, RequestDigest, SemanticDigest, TenantScopeId};

use super::*;
use crate::{
    ExecutorBinding, ExecutorDeployment, NonDomainDisposition, NonDomainEntryStatus,
    NonDomainFailureCode, ReferenceFailureCode,
};

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
    let bounds = EvidenceBounds::new(4, 16, 500_000, 10_000, 2, 10_000).expect("bounds");
    (
        identity,
        bounds,
        proof_ref,
        DeliveryAudit::from_ledger(vec![first, second, third]),
    )
}

fn exact_completion_bounds(max_completion_record_bytes: usize) -> EvidenceBounds {
    let max_completion_record_bytes =
        u64::try_from(max_completion_record_bytes).expect("test completion length");
    EvidenceBounds::new(
        1,
        8,
        1_000_000,
        max_completion_record_bytes,
        2,
        max_completion_record_bytes,
    )
    .expect("exact completion bounds")
}

fn audit_with_observation(
    label: &str,
    outcome: DeliveryAttemptOutcome,
) -> (
    EffectIdentity,
    ContentRef,
    DeliveryAudit,
    DeliveryAuditFrontier,
) {
    let identity = effect_identity(label);
    let proof_ref = reviewed_ref(&format!("{label}.proof"));
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
    .expect("effect binding");
    let operation = reviewed_ref(&format!("{label}.operation"));
    let attempt_id = derive_attempt_id(&identity, 0, &operation).expect("attempt");
    let second = DeliveryAuditFrontier::append(
        &identity,
        Some(first.reference().expect("binding frontier")),
        vec![ExecutorEvidenceRecord::DeliveryAttemptAuthorized {
            attempt_ordinal: 0,
            attempt_id: attempt_id.clone(),
            target_operation_ref: operation,
        }],
        proof_ref.clone(),
    )
    .expect("authorization");
    let observation = DeliveryAuditFrontier::append(
        &identity,
        Some(second.reference().expect("authorization frontier")),
        vec![ExecutorEvidenceRecord::DeliveryAttemptObserved {
            attempt_id,
            outcome,
        }],
        proof_ref.clone(),
    )
    .expect("observation");
    (
        identity,
        proof_ref,
        DeliveryAudit::from_ledger(vec![first, second, observation.clone()]),
        observation,
    )
}

#[test]
fn evidence_bounds_reject_legacy_zero_inconsistent_and_overflowing_forms() {
    EvidenceBounds::new(0, 1, 4096, 0, 0, 0).expect("zero-attempt bounds");
    for invalid in [
        EvidenceBounds::new(0, 2, 4096, 1, 0, 0),
        EvidenceBounds::new(1, 2, 4096, 0, 2, 1024),
        EvidenceBounds::new(1, 2, 4096, 1025, 2, 1024),
        EvidenceBounds::new(1, 2, 4096, 1024, 1, 1024),
        EvidenceBounds::new(1, 3, 4096, 1024, 3, 1024),
        EvidenceBounds::new(1, 2, u64::MAX, u64::MAX, 2, u64::MAX),
    ] {
        assert_eq!(
            invalid.expect_err("invalid completion bounds"),
            ExecutorError::EvidenceBoundsExhausted
        );
    }

    let legacy = br#"{"completion_reserve_bytes":"1024","completion_reserve_records":2,"max_attempts":1,"max_records":4,"max_retained_bytes":"4096"}"#;
    assert!(EvidenceBounds::strict_decode(legacy).is_err());
    let overflowing = br#"{"completion_reserve_bytes":"18446744073709551616","completion_reserve_records":2,"max_attempts":1,"max_completion_record_bytes":"18446744073709551616","max_records":4,"max_retained_bytes":"18446744073709551616"}"#;
    assert!(EvidenceBounds::strict_decode(overflowing).is_err());

    let exact = EvidenceBounds::new(1, 4, 4096, 1024, 2, 1024).expect("exact bounds");
    assert_eq!(
        EvidenceBounds::strict_decode(exact.validated().expect("canonical").as_bytes())
            .expect("strict round trip"),
        exact
    );
}

#[test]
fn every_observation_outcome_enforces_the_exact_completion_closure_boundary() {
    let safe_result = encode(
        "mfm.primitive-stable_id.v1",
        &CanonicalValue::String("returned".to_owned()),
    )
    .expect("safe result");
    let outcomes = [
        (
            "returned",
            DeliveryAttemptOutcome::returned(
                SchemaQualifiedCanonicalValue::from_validated(&safe_result)
                    .expect("schema-qualified result"),
            )
            .expect("returned outcome"),
        ),
        (
            "did-not-enter",
            DeliveryAttemptOutcome::did_not_enter(
                reference_safe_failure(
                    reviewed_ref("did-not-enter.contract"),
                    ReferenceFailureCode::GenerationFenced,
                    FailureClass::Authorization,
                    BoundaryStage::BeforeBoundaryEntry,
                )
                .expect("did-not-enter failure"),
            )
            .expect("did-not-enter outcome"),
        ),
        (
            "result-unrepresentable",
            DeliveryAttemptOutcome::indeterminate(
                reference_safe_failure(
                    reviewed_ref("result-unrepresentable.contract"),
                    ReferenceFailureCode::ResultUnrepresentable,
                    FailureClass::UnrepresentableResponse,
                    BoundaryStage::BoundaryObservation,
                )
                .expect("result-unrepresentable failure"),
            )
            .expect("indeterminate outcome"),
        ),
        (
            "non-domain",
            DeliveryAttemptOutcome::non_domain_failure(
                NonDomainFailure::new(
                    NonDomainEntryStatus::MayHaveEntered,
                    NonDomainDisposition::IntegrityBlocked,
                    NonDomainFailureCode::AdapterContractViolation,
                )
                .expect("non-domain failure"),
            )
            .expect("non-domain outcome"),
        ),
    ];

    for (label, outcome) in outcomes {
        let (identity, proof_ref, audit, observation) = audit_with_observation(label, outcome);
        let completion_bytes = observation.retained_bytes().expect("completion closure");
        audit
            .verify_structure(
                &identity,
                &exact_completion_bounds(completion_bytes),
                &proof_ref,
            )
            .expect("exact completion boundary");
        assert_eq!(
            audit
                .verify_structure(
                    &identity,
                    &EvidenceBounds::new(
                        1,
                        8,
                        1_000_000,
                        u64::try_from(completion_bytes - 1).expect("short completion"),
                        2,
                        u64::try_from(completion_bytes).expect("completion reserve"),
                    )
                    .expect("one-short bounds"),
                    &proof_ref,
                )
                .expect_err("one byte over completion maximum"),
            ExecutorError::EvidenceBoundsExhausted,
            "{label}"
        );
    }
}

#[test]
fn terminal_tombstone_enforces_the_exact_completion_closure_boundary() {
    let safe_result = encode(
        "mfm.primitive-stable_id.v1",
        &CanonicalValue::String("terminal-result".to_owned()),
    )
    .expect("safe result");
    let returned = ReturnedOutcome::new(
        SchemaQualifiedCanonicalValue::from_validated(&safe_result)
            .expect("schema-qualified result"),
    )
    .expect("returned outcome");
    let (identity, proof_ref, observed_audit, observation) = audit_with_observation(
        "tombstone-bound",
        DeliveryAttemptOutcome::returned(returned.safe_result().clone())
            .expect("returned attempt outcome"),
    );
    let attempt = observed_audit
        .attempts()
        .expect("attempts")
        .into_iter()
        .next()
        .expect("attempt");
    let tombstone = TerminalTombstone::new(
        "terminal.effect",
        "applied",
        ReferenceTerminalProof::new(
            attempt.attempt_id().clone(),
            returned,
            attempt
                .returned_observation_ref()
                .expect("returned observation")
                .clone(),
        )
        .expect("terminal proof"),
    )
    .expect("tombstone");
    let terminal = DeliveryAuditFrontier::append(
        &identity,
        Some(observed_audit.head_ref().expect("observation head")),
        vec![ExecutorEvidenceRecord::TerminalTombstone(tombstone)],
        proof_ref.clone(),
    )
    .expect("tombstone frontier");
    let terminal_bytes = terminal.retained_bytes().expect("tombstone closure");
    assert!(
        terminal_bytes >= observation.retained_bytes().expect("observation closure"),
        "tombstone fixture must select the individual maximum"
    );
    let mut frontiers = observed_audit.frontiers().to_vec();
    frontiers.push(terminal);
    let terminal_audit = DeliveryAudit::from_ledger(frontiers);
    terminal_audit
        .verify_structure(
            &identity,
            &exact_completion_bounds(terminal_bytes),
            &proof_ref,
        )
        .expect("exact tombstone boundary");
    assert_eq!(
        terminal_audit
            .verify_structure(
                &identity,
                &EvidenceBounds::new(
                    1,
                    8,
                    1_000_000,
                    u64::try_from(terminal_bytes - 1).expect("short terminal"),
                    2,
                    u64::try_from(terminal_bytes).expect("terminal reserve"),
                )
                .expect("one-short bounds"),
                &proof_ref,
            )
            .expect_err("one byte over tombstone maximum"),
        ExecutorError::EvidenceBoundsExhausted
    );
}

#[test]
fn forged_proof_is_rejected_without_retaining_candidate() {
    let (identity, bounds, proof_ref, mut audit) = valid_audit();
    audit
        .verify_structure(&identity, &bounds, &proof_ref)
        .expect("valid");
    audit.frontiers[1].proof =
        FrontierProof(SemanticDigest::from_digest(sha256_digest_bytes(b"forged")));

    let mut accumulator = DeliveryAuditAccumulator::default();
    assert_eq!(
        accumulator
            .admit_structure(audit, &identity, &bounds, &proof_ref)
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
            .verify_structure(&identity, &bounds, &proof_ref)
            .expect_err("omitted predecessor"),
        ExecutorError::InvalidFrontier
    );

    let mut wrong_binding = audit;
    wrong_binding.frontiers[2].executor_binding_ref =
        effect_identity("wrong").executor_binding_ref().clone();
    assert_eq!(
        wrong_binding
            .verify_structure(&identity, &bounds, &proof_ref)
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
            .admit_structure(audit.clone(), &identity, &bounds, &proof_ref)
            .expect("initial"),
        AdmitFrontier::Advanced
    );
    assert_eq!(
        accumulator
            .admit_structure(
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
            .admit_structure(forked, &identity, &bounds, &proof_ref)
            .expect_err("fork"),
        ExecutorError::FrontierFork
    );
}

#[test]
fn completion_reserve_is_cumulative_for_unmatched_attempts() {
    let identity = effect_identity("reserve");
    let proof_ref = reviewed_ref("reserve.proof");
    let bounds = EvidenceBounds::new(3, 4, 1_000_000, 100, 2, 100).expect("bounds");
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
        .verify_structure(&identity, &bounds, &proof_ref)
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
            .verify_structure(&identity, &bounds, &proof_ref)
            .expect_err("two observations plus tombstone do not fit"),
        ExecutorError::EvidenceBoundsExhausted
    );
}

#[test]
fn unmatched_authorization_and_shared_tombstone_are_charged_before_target_entry() {
    let identity = effect_identity("reserve-records");
    let proof_ref = reviewed_ref("reserve-records.proof");
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
    let operation = reviewed_ref("reserve-records.operation");
    let second = DeliveryAuditFrontier::append(
        &identity,
        Some(first.reference().expect("first ref")),
        vec![ExecutorEvidenceRecord::DeliveryAttemptAuthorized {
            attempt_ordinal: 0,
            attempt_id: derive_attempt_id(&identity, 0, &operation).expect("attempt"),
            target_operation_ref: operation,
        }],
        proof_ref.clone(),
    )
    .expect("authorization");
    let audit = DeliveryAudit::from_ledger(vec![first, second]);
    assert_eq!(
        audit
            .verify_structure(
                &identity,
                &EvidenceBounds::new(1, 3, 1_000_000, 100, 2, 100).expect("tight bounds"),
                &proof_ref,
            )
            .expect_err("one observation and one shared tombstone do not fit"),
        ExecutorError::EvidenceBoundsExhausted
    );
    audit
        .verify_structure(
            &identity,
            &EvidenceBounds::new(1, 4, 1_000_000, 100, 2, 100).expect("fitting bounds"),
            &proof_ref,
        )
        .expect("full structural completion envelope fits");
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
        outcome: DeliveryAttemptOutcome::returned(returned.safe_result().clone())
            .expect("returned attempt outcome"),
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
        vec![ExecutorEvidenceRecord::EffectBound {
            executor_binding_ref: identity.executor_binding_ref().clone(),
            effect_key: identity.effect_key().clone(),
            request_digest: identity.request_digest().clone(),
        }],
        proof_ref.clone(),
    )
    .expect("frontier");
    let second = DeliveryAuditFrontier::append(
        &identity,
        Some(first.reference().expect("first frontier")),
        vec![ExecutorEvidenceRecord::DeliveryAttemptAuthorized {
            attempt_ordinal: 0,
            attempt_id,
            target_operation_ref: operation,
        }],
        proof_ref.clone(),
    )
    .expect("authorization frontier");
    let third = DeliveryAuditFrontier::append(
        &identity,
        Some(second.reference().expect("second frontier")),
        vec![observed],
        proof_ref.clone(),
    )
    .expect("observation frontier");
    let fourth = DeliveryAuditFrontier::append(
        &identity,
        Some(third.reference().expect("third frontier")),
        vec![ExecutorEvidenceRecord::TerminalTombstone(tombstone)],
        proof_ref.clone(),
    )
    .expect("tombstone frontier");
    let bounds = EvidenceBounds::new(4, 16, 500_000, 10_000, 2, 10_000).expect("bounds");
    assert_eq!(
        DeliveryAudit::from_ledger(vec![first, second, third, fourth])
            .verify_structure(&identity, &bounds, &proof_ref)
            .expect_err("wrong proof attempt"),
        ExecutorError::TerminalProofMismatch
    );
}
