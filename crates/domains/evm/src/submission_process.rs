//! Deterministic callbacks and semantic validators for structured EVM submission.

use std::str::FromStr;

use alloy_primitives::U256;
use mfm_ids::{StableId, TenantScopeId};
use mfm_program::structured::{CommittedObservation, StateSettlement};
use mfm_spec::structured::ProposedStateOutcome;

use crate::submission::{
    ActiveCandidateWork, CandidateActivationDecision, CandidateObservationWork,
    CandidateResolution, CandidateSlotDecision, CandidateTransactionObservation, CandidateWork,
    CompletedProjection, CompletionWork, DerivedSubmissionDomain, EvmFinalizedHeadObservation,
    EvmInclusionBlockObservation, EvmReceiptLookupObservation, EvmSubmissionOutput,
    EvmTransactionLookupObservation, FailureReconciliationRequest, IntentBoundSubmission,
    ObservationRoundDecision, ObservedPendingSubmission, PendingEvmSubmissionFailure,
    PermittedCandidateWork, PostReservePreparedSubmission, PreparedCandidateActivation,
    PreparedWalletSubmission, QualifiedPendingSubmission, SubmissionProgress,
    SubmissionTerminalDecision, SubmissionWork, TerminalEvidenceDecision, TerminalEvidenceWork,
    UnsignedWalletCandidate, WalletStatusBaseline, WalletStatusDecision,
};
use crate::submission_expansion::{PendingEvmSubmissionFailureRoute, SubmissionFailureRoute};
use crate::{
    canonical_wallet_reference, derive_authenticated_intent_issuer_id,
    derive_evm_candidate_operation_key, derive_evm_nonce_completion_key,
    derive_evm_nonce_reservation_key, derive_exact_candidate_activation_permit,
    derive_submission_intent_id, ActivateCandidateResponse, ActivateEvmCandidateRequest,
    ActiveWalletCandidate, AttestedWalletCandidate, CanonicalTerminalOutcome,
    CompleteEvmNonceRequest, CompleteWalletNonceResponse, CompletedWalletNonce,
    EvmPendingNonceRequest, EvmReceiptLookupRequest, EvmSubmissionFailure, EvmSubmissionRequest,
    EvmTransactionLookupRequest, EvmWalletReference, ExecutionDisposition,
    ObservedPendingNonceFloor, QualifiedPendingNonceFloor, ReadEvmWalletNonceStatusRequest,
    ReserveEvmNonceRequest, ReserveWalletNonceResponse, ReservedWalletNonce,
    SubmittedCandidateProof, TerminalWitnesses, WalletNonceStatus,
};

pub(crate) fn map_submission_failure(
    failure: &EvmSubmissionFailure,
) -> ProposedStateOutcome<SubmissionFailureRoute, mfm_program::structured::Never> {
    ProposedStateOutcome::Success(SubmissionFailureRoute::Propagate { failure: *failure })
}

pub(crate) fn map_pending_submission_failure(
    failure: &PendingEvmSubmissionFailure,
) -> ProposedStateOutcome<PendingEvmSubmissionFailureRoute, mfm_program::structured::Never> {
    ProposedStateOutcome::Success(PendingEvmSubmissionFailureRoute::Propagate {
        failure: failure.clone(),
    })
}

pub(crate) fn handle_pending_submission_failure(
    failure: &PendingEvmSubmissionFailure,
) -> ProposedStateOutcome<PendingEvmSubmissionFailure, mfm_program::structured::Never> {
    ProposedStateOutcome::Success(failure.clone())
}

pub(crate) fn derive_transaction_intent(
    request: &EvmSubmissionRequest,
) -> ProposedStateOutcome<DerivedSubmissionDomain, mfm_program::structured::Never> {
    ProposedStateOutcome::Success(DerivedSubmissionDomain {
        request: request.clone(),
        qualified_chain_instance_id: request
            .transaction_intent()
            .qualified_chain_instance_id()
            .clone(),
        nonce_domain: request.transaction_intent().nonce_domain().clone(),
    })
}

pub(crate) fn derive_submission_intent(
    derived: &DerivedSubmissionDomain,
) -> ProposedStateOutcome<IntentBoundSubmission, EvmSubmissionFailure> {
    if derived.request.validate().is_err()
        || derived.qualified_chain_instance_id
            != *derived
                .request
                .transaction_intent()
                .qualified_chain_instance_id()
        || derived.nonce_domain != *derived.request.transaction_intent().nonce_domain()
    {
        return ProposedStateOutcome::Failure(EvmSubmissionFailure::NonceLineageDiverged);
    }
    let request = &derived.request;
    let tenant = match TenantScopeId::new(request.tenant_scope_id()) {
        Ok(value) => value,
        Err(_) => return ProposedStateOutcome::Failure(EvmSubmissionFailure::DestinationRejected),
    };
    let principal = match StableId::new(request.authenticated_principal_id()) {
        Ok(value) => value,
        Err(_) => return ProposedStateOutcome::Failure(EvmSubmissionFailure::DestinationRejected),
    };
    let issuer_id = match derive_authenticated_intent_issuer_id(
        &tenant,
        &principal,
        request.issuer_namespace_contract_ref(),
    ) {
        Ok(value) => value,
        Err(_) => return ProposedStateOutcome::Failure(EvmSubmissionFailure::DestinationRejected),
    };
    let expansion_contract_ref = match crate::evm_submission_expansion_policy_ref() {
        Ok(value) => value,
        Err(_) => return ProposedStateOutcome::Failure(EvmSubmissionFailure::DestinationRejected),
    };
    let submission_intent_id = match derive_submission_intent_id(
        &derived.nonce_domain,
        &issuer_id,
        request.caller_submission_token(),
        request.observation_rounds(),
        request.candidate_family().digest(),
        &expansion_contract_ref,
    ) {
        Ok(value) => value,
        Err(_) => return ProposedStateOutcome::Failure(EvmSubmissionFailure::DestinationRejected),
    };
    ProposedStateOutcome::Success(IntentBoundSubmission {
        derived: derived.clone(),
        issuer_id,
        submission_intent_id,
    })
}

pub(crate) fn derive_reservation_key(
    intent: &IntentBoundSubmission,
) -> ProposedStateOutcome<PreparedWalletSubmission, EvmSubmissionFailure> {
    match derive_evm_nonce_reservation_key(
        &intent.derived.nonce_domain,
        &intent.submission_intent_id,
    ) {
        Ok(reservation_key) => ProposedStateOutcome::Success(PreparedWalletSubmission {
            intent: intent.clone(),
            reservation_key,
        }),
        Err(_) => ProposedStateOutcome::Failure(EvmSubmissionFailure::NonceLineageDiverged),
    }
}

pub(crate) fn read_status_request(
    prepared: &PreparedWalletSubmission,
) -> ReadEvmWalletNonceStatusRequest {
    let request = &prepared.intent.derived.request;
    ReadEvmWalletNonceStatusRequest {
        nonce_domain: prepared.intent.derived.nonce_domain.clone(),
        domain_activation_attestation: request.domain_activation_attestation().clone(),
        semantic_reservation_key: prepared.reservation_key.clone(),
        submission_intent_id: prepared.intent.submission_intent_id.clone(),
        transaction_intent_digest: request.transaction_intent().digest().to_owned(),
        candidate_family_ref: request.candidate_family().digest().to_owned(),
    }
}

pub(crate) fn post_reserve_status_request(
    prepared: &PostReservePreparedSubmission,
) -> ReadEvmWalletNonceStatusRequest {
    read_status_request(&prepared.prepared)
}

pub(crate) fn failure_reconciliation_status_request(
    request: &FailureReconciliationRequest,
) -> ReadEvmWalletNonceStatusRequest {
    read_status_request(&request.prepared)
}

pub(crate) fn settle_wallet_status(
    prepared: &PreparedWalletSubmission,
    observation: &CommittedObservation<WalletNonceStatus, EvmSubmissionFailure>,
) -> StateSettlement<WalletStatusDecision, PendingEvmSubmissionFailure> {
    let returned = match observation {
        CommittedObservation::SafeFailure(failure) => {
            return StateSettlement::Proposed(ProposedStateOutcome::Failure(
                PendingEvmSubmissionFailure::Direct { failure: *failure },
            ));
        }
        CommittedObservation::Returned(returned) => returned,
    };
    match validated_wallet_status(prepared, returned) {
        Some(decision) => StateSettlement::Proposed(ProposedStateOutcome::Success(decision)),
        None => StateSettlement::InvalidEvidence,
    }
}

pub(crate) fn settle_post_reserve_wallet_status(
    prepared: &PostReservePreparedSubmission,
    observation: &CommittedObservation<WalletNonceStatus, EvmSubmissionFailure>,
) -> StateSettlement<WalletStatusDecision, PendingEvmSubmissionFailure> {
    let returned = match observation {
        CommittedObservation::SafeFailure(failure) => {
            return StateSettlement::Proposed(ProposedStateOutcome::Failure(
                PendingEvmSubmissionFailure::Direct { failure: *failure },
            ));
        }
        CommittedObservation::Returned(returned) => returned,
    };
    let reservation_matches = match returned {
        WalletNonceStatus::Reserved { reservation, .. }
        | WalletNonceStatus::Completed { reservation, .. } => reservation == &prepared.reservation,
        WalletNonceStatus::Absent | WalletNonceStatus::Busy => false,
    };
    if !reservation_matches {
        return StateSettlement::InvalidEvidence;
    }
    match validated_wallet_status(&prepared.prepared, returned) {
        Some(decision) => StateSettlement::Proposed(ProposedStateOutcome::Success(decision)),
        None => StateSettlement::InvalidEvidence,
    }
}

pub(crate) fn settle_reservation_failure_status(
    request: &FailureReconciliationRequest,
    observation: &CommittedObservation<WalletNonceStatus, EvmSubmissionFailure>,
) -> StateSettlement<WalletStatusDecision, EvmSubmissionFailure> {
    let returned = match observation {
        CommittedObservation::SafeFailure(failure) => {
            return StateSettlement::Proposed(ProposedStateOutcome::Failure(*failure));
        }
        CommittedObservation::Returned(returned) => returned,
    };
    if request.baseline != WalletStatusBaseline::Absent {
        return StateSettlement::InvalidEvidence;
    }
    let Some(status) = validated_wallet_status(&request.prepared, returned) else {
        return StateSettlement::InvalidEvidence;
    };
    match status {
        WalletStatusDecision::Absent => {
            StateSettlement::Proposed(ProposedStateOutcome::Failure(request.original_failure))
        }
        WalletStatusDecision::Busy { .. } => StateSettlement::Proposed(
            ProposedStateOutcome::Failure(EvmSubmissionFailure::NonceDomainBusy),
        ),
        WalletStatusDecision::Reserved { .. } | WalletStatusDecision::Completed { .. } => {
            StateSettlement::Proposed(ProposedStateOutcome::Success(status))
        }
    }
}

pub(crate) fn settle_candidate_failure_status(
    request: &FailureReconciliationRequest,
    observation: &CommittedObservation<WalletNonceStatus, EvmSubmissionFailure>,
) -> StateSettlement<CandidateResolution, EvmSubmissionFailure> {
    let returned = match observation {
        CommittedObservation::SafeFailure(failure) => {
            return StateSettlement::Proposed(ProposedStateOutcome::Failure(*failure));
        }
        CommittedObservation::Returned(returned) => returned,
    };
    let WalletStatusBaseline::Reserved { .. } = &request.baseline else {
        return StateSettlement::InvalidEvidence;
    };
    let Some(status) = validated_wallet_status(&request.prepared, returned) else {
        return StateSettlement::InvalidEvidence;
    };
    match status {
        WalletStatusDecision::Reserved { work } if work.status_baseline == request.baseline => {
            StateSettlement::Proposed(ProposedStateOutcome::Failure(request.original_failure))
        }
        WalletStatusDecision::Reserved { work } => {
            StateSettlement::Proposed(ProposedStateOutcome::Success(CandidateResolution::Resume {
                work,
            }))
        }
        WalletStatusDecision::Completed { completion } => StateSettlement::Proposed(
            ProposedStateOutcome::Success(CandidateResolution::Completed { completion }),
        ),
        WalletStatusDecision::Absent | WalletStatusDecision::Busy { .. } => {
            StateSettlement::InvalidEvidence
        }
    }
}

pub(crate) fn settle_candidate_wallet_status(
    prepared: &PreparedWalletSubmission,
    observation: &CommittedObservation<WalletNonceStatus, EvmSubmissionFailure>,
) -> StateSettlement<CandidateResolution, PendingEvmSubmissionFailure> {
    let returned = match observation {
        CommittedObservation::SafeFailure(failure) => {
            return StateSettlement::Proposed(ProposedStateOutcome::Failure(
                PendingEvmSubmissionFailure::Direct { failure: *failure },
            ));
        }
        CommittedObservation::Returned(returned) => returned,
    };
    let Some(status) = validated_wallet_status(prepared, returned) else {
        return StateSettlement::InvalidEvidence;
    };
    match status {
        WalletStatusDecision::Reserved { work } => {
            StateSettlement::Proposed(ProposedStateOutcome::Success(CandidateResolution::Resume {
                work,
            }))
        }
        WalletStatusDecision::Completed { completion } => StateSettlement::Proposed(
            ProposedStateOutcome::Success(CandidateResolution::Completed { completion }),
        ),
        WalletStatusDecision::Absent | WalletStatusDecision::Busy { .. } => {
            StateSettlement::InvalidEvidence
        }
    }
}

fn validated_wallet_status(
    prepared: &PreparedWalletSubmission,
    returned: &WalletNonceStatus,
) -> Option<WalletStatusDecision> {
    let decision = match returned {
        WalletNonceStatus::Absent => WalletStatusDecision::Absent,
        WalletNonceStatus::Busy => WalletStatusDecision::Busy {
            failure: EvmSubmissionFailure::NonceDomainBusy,
        },
        WalletNonceStatus::Reserved {
            reservation,
            transaction_intent,
            candidate_family,
            activated_candidates,
            current_candidate,
            resource_head_ref,
        } => {
            if !valid_reserved_status(
                prepared,
                reservation,
                transaction_intent,
                candidate_family,
                activated_candidates,
                current_candidate.as_ref(),
                resource_head_ref,
            ) {
                return None;
            }
            let canonical_status_digest = canonical_wallet_reference(returned)
                .ok()?
                .content_digest()
                .to_owned();
            WalletStatusDecision::Reserved {
                work: SubmissionWork {
                    prepared: prepared.clone(),
                    reservation: reservation.clone(),
                    activated_candidates: activated_candidates.clone(),
                    current_candidate: current_candidate.clone(),
                    next_candidate_ordinal: activated_candidates.len() as u16,
                    status_baseline: WalletStatusBaseline::Reserved {
                        resource_head_ref: resource_head_ref.clone(),
                        canonical_status_digest,
                    },
                },
            }
        }
        WalletNonceStatus::Completed {
            reservation,
            transaction_intent,
            candidate_family,
            activated_candidates,
            completion,
            resource_head_ref,
        } => {
            if !valid_reserved_status(
                prepared,
                reservation,
                transaction_intent,
                candidate_family,
                activated_candidates,
                activated_candidates.last(),
                resource_head_ref,
            ) || !valid_completion(prepared, reservation, completion)
            {
                return None;
            }
            WalletStatusDecision::Completed {
                completion: completion.clone(),
            }
        }
    };
    Some(decision)
}

fn valid_reserved_status(
    prepared: &PreparedWalletSubmission,
    reservation: &ReservedWalletNonce,
    transaction_intent: &crate::EvmTransactionIntent,
    candidate_family: &crate::EvmCandidateFamily,
    activated_candidates: &[ActiveWalletCandidate],
    current_candidate: Option<&ActiveWalletCandidate>,
    resource_head_ref: &EvmWalletReference,
) -> bool {
    let request = &prepared.intent.derived.request;
    reservation.nonce_domain == prepared.intent.derived.nonce_domain
        && reservation.semantic_reservation_key == prepared.reservation_key
        && reservation.submission_intent_id == prepared.intent.submission_intent_id
        && reservation.transaction_intent_digest == request.transaction_intent().digest()
        && reservation.candidate_family_ref == request.candidate_family().digest()
        && transaction_intent == request.transaction_intent()
        && candidate_family == request.candidate_family()
        && candidate_family.validate(transaction_intent).is_ok()
        && resource_head_ref.to_content_ref().is_ok()
        && valid_active_prefix(
            reservation,
            transaction_intent,
            candidate_family,
            activated_candidates,
            current_candidate,
        )
}

fn valid_active_prefix(
    reservation: &ReservedWalletNonce,
    transaction_intent: &crate::EvmTransactionIntent,
    candidate_family: &crate::EvmCandidateFamily,
    activated_candidates: &[ActiveWalletCandidate],
    current_candidate: Option<&ActiveWalletCandidate>,
) -> bool {
    crate::validate_active_wallet_candidate_prefix(
        reservation,
        transaction_intent,
        candidate_family,
        activated_candidates,
    )
    .is_ok()
        && current_candidate == activated_candidates.last()
}

fn valid_completion(
    prepared: &PreparedWalletSubmission,
    reservation: &ReservedWalletNonce,
    completion: &CompletedWalletNonce,
) -> bool {
    completion.nonce_domain == reservation.nonce_domain
        && completion.nonce == reservation.nonce
        && completion.semantic_reservation_key == prepared.reservation_key
        && completion.canonical_terminal_outcome.nonce_domain == reservation.nonce_domain
        && completion
            .canonical_terminal_outcome
            .semantic_reservation_key
            == prepared.reservation_key
        && completion.canonical_terminal_outcome.submission_intent_id
            == prepared.intent.submission_intent_id
        && completion.completion_evidence_ref.to_content_ref().is_ok()
}

pub(crate) fn pending_nonce_request(prepared: &PreparedWalletSubmission) -> EvmPendingNonceRequest {
    EvmPendingNonceRequest {
        nonce_domain: prepared.intent.derived.nonce_domain.clone(),
        route_generation_ref: prepared
            .intent
            .derived
            .request
            .route_generation_ref()
            .clone(),
    }
}

fn reconcile_absent(
    prepared: &PreparedWalletSubmission,
    failure: EvmSubmissionFailure,
) -> PendingEvmSubmissionFailure {
    PendingEvmSubmissionFailure::Reconcile {
        request: FailureReconciliationRequest {
            prepared: prepared.clone(),
            baseline: WalletStatusBaseline::Absent,
            original_failure: failure,
        },
    }
}

fn reconcile_reserved(
    work: &SubmissionWork,
    failure: EvmSubmissionFailure,
) -> PendingEvmSubmissionFailure {
    PendingEvmSubmissionFailure::Reconcile {
        request: FailureReconciliationRequest {
            prepared: work.prepared.clone(),
            baseline: work.status_baseline.clone(),
            original_failure: failure,
        },
    }
}

pub(crate) fn settle_pending_nonce(
    prepared: &PreparedWalletSubmission,
    observation: &CommittedObservation<ObservedPendingNonceFloor, EvmSubmissionFailure>,
) -> StateSettlement<ObservedPendingSubmission, PendingEvmSubmissionFailure> {
    match observation {
        CommittedObservation::SafeFailure(failure) => StateSettlement::Proposed(
            ProposedStateOutcome::Failure(reconcile_absent(prepared, *failure)),
        ),
        CommittedObservation::Returned(observed)
            if observed.nonce_domain == prepared.intent.derived.nonce_domain
                && observed.route_generation_ref
                    == *prepared.intent.derived.request.route_generation_ref() =>
        {
            StateSettlement::Proposed(ProposedStateOutcome::Success(ObservedPendingSubmission {
                prepared: prepared.clone(),
                observed: observed.clone(),
            }))
        }
        CommittedObservation::Returned(_) => StateSettlement::InvalidEvidence,
    }
}

pub(crate) fn qualify_pending_nonce(
    observed: &ObservedPendingSubmission,
) -> ProposedStateOutcome<QualifiedPendingSubmission, PendingEvmSubmissionFailure> {
    let policy = match crate::evm_wallet_nonce_policy_ref() {
        Ok(reference) => reference,
        Err(_) => {
            return ProposedStateOutcome::Failure(reconcile_absent(
                &observed.prepared,
                EvmSubmissionFailure::ObservationPolicyExhausted,
            ));
        }
    };
    ProposedStateOutcome::Success(QualifiedPendingSubmission {
        prepared: observed.prepared.clone(),
        floor: QualifiedPendingNonceFloor {
            observed: observed.observed.clone(),
            pending_floor_policy_ref: policy,
        },
    })
}

pub(crate) fn reserve_nonce_request(
    qualified: &QualifiedPendingSubmission,
) -> ReserveEvmNonceRequest {
    let prepared = &qualified.prepared;
    let request = &prepared.intent.derived.request;
    ReserveEvmNonceRequest {
        nonce_domain: prepared.intent.derived.nonce_domain.clone(),
        domain_activation_attestation: request.domain_activation_attestation().clone(),
        submission_intent_id: prepared.intent.submission_intent_id.clone(),
        transaction_intent: request.transaction_intent().clone(),
        candidate_family: request.candidate_family().clone(),
        reservation_key: prepared.reservation_key.clone(),
        qualified_floor: qualified.floor.clone(),
    }
}

pub(crate) fn settle_reservation(
    qualified: &QualifiedPendingSubmission,
    observation: &CommittedObservation<ReserveWalletNonceResponse, EvmSubmissionFailure>,
) -> StateSettlement<PostReservePreparedSubmission, PendingEvmSubmissionFailure> {
    match observation {
        CommittedObservation::SafeFailure(failure) => StateSettlement::Proposed(
            ProposedStateOutcome::Failure(reconcile_absent(&qualified.prepared, *failure)),
        ),
        CommittedObservation::Returned(ReserveWalletNonceResponse::Reserved { reservation })
            if reservation_matches_prepared(reservation, &qualified.prepared) =>
        {
            StateSettlement::Proposed(ProposedStateOutcome::Success(
                PostReservePreparedSubmission {
                    prepared: qualified.prepared.clone(),
                    reservation: reservation.clone(),
                },
            ))
        }
        CommittedObservation::Returned(ReserveWalletNonceResponse::NonceDomainBusy) => {
            StateSettlement::Proposed(ProposedStateOutcome::Failure(reconcile_absent(
                &qualified.prepared,
                EvmSubmissionFailure::NonceDomainBusy,
            )))
        }
        CommittedObservation::Returned(ReserveWalletNonceResponse::NonceLineageDiverged) => {
            StateSettlement::Proposed(ProposedStateOutcome::Failure(reconcile_absent(
                &qualified.prepared,
                EvmSubmissionFailure::NonceLineageDiverged,
            )))
        }
        CommittedObservation::Returned(ReserveWalletNonceResponse::NonceCapacityExhausted) => {
            StateSettlement::Proposed(ProposedStateOutcome::Failure(reconcile_absent(
                &qualified.prepared,
                EvmSubmissionFailure::NonceCapacityExhausted,
            )))
        }
        CommittedObservation::Returned(_) => StateSettlement::InvalidEvidence,
    }
}

fn reservation_matches_prepared(
    reservation: &ReservedWalletNonce,
    prepared: &PreparedWalletSubmission,
) -> bool {
    let request = &prepared.intent.derived.request;
    reservation.nonce_domain == prepared.intent.derived.nonce_domain
        && reservation.semantic_reservation_key == prepared.reservation_key
        && reservation.submission_intent_id == prepared.intent.submission_intent_id
        && reservation.transaction_intent_digest == request.transaction_intent().digest()
        && reservation.candidate_family_ref == request.candidate_family().digest()
}

pub(crate) fn collapse_wallet_status(
    status: &WalletStatusDecision,
) -> ProposedStateOutcome<SubmissionProgress, mfm_program::structured::Never> {
    let progress = match status {
        WalletStatusDecision::Completed { completion } => SubmissionProgress {
            work: None,
            completion: Some(completion.clone()),
            failure: None,
        },
        WalletStatusDecision::Busy { failure } => SubmissionProgress {
            work: None,
            completion: None,
            failure: Some(*failure),
        },
        WalletStatusDecision::Reserved { work } => SubmissionProgress {
            work: Some(work.clone()),
            completion: None,
            failure: None,
        },
        WalletStatusDecision::Absent => SubmissionProgress {
            work: None,
            completion: None,
            failure: Some(EvmSubmissionFailure::NonceAuthorityUnavailable),
        },
    };
    ProposedStateOutcome::Success(progress)
}

pub(crate) fn select_candidate_slot(
    progress: &SubmissionProgress,
) -> ProposedStateOutcome<CandidateSlotDecision, mfm_program::structured::Never> {
    let decision = match (&progress.work, &progress.completion, &progress.failure) {
        (Some(work), None, None) => {
            let family_len = work
                .prepared
                .intent
                .derived
                .request
                .candidate_family()
                .candidates()
                .len();
            let activated_len = work.activated_candidates.len();
            let next = usize::from(work.next_candidate_ordinal);
            if next < family_len {
                CandidateSlotDecision::Execute { work: work.clone() }
            } else if activated_len > 0 {
                // EVM-03: family slots are exhausted, but retained activated
                // candidates must still be observed on recovery so already-
                // finalized work can complete. Re-enter the last activated
                // ordinal; activation/broadcast resolve idempotently.
                let mut recover = work.clone();
                let last = u16::try_from(activated_len.saturating_sub(1)).unwrap_or(0);
                recover.next_candidate_ordinal = last;
                recover.current_candidate = work.activated_candidates.last().cloned();
                CandidateSlotDecision::Execute { work: recover }
            } else {
                CandidateSlotDecision::Exhausted
            }
        }
        _ => CandidateSlotDecision::Skip,
    };
    ProposedStateOutcome::Success(decision)
}

pub(crate) fn mark_candidate_family_exhausted(
    progress: &SubmissionProgress,
) -> ProposedStateOutcome<SubmissionProgress, mfm_program::structured::Never> {
    ProposedStateOutcome::Success(SubmissionProgress {
        work: None,
        completion: progress.completion.clone(),
        failure: Some(EvmSubmissionFailure::ReplacementPolicyExhausted),
    })
}

pub(crate) fn build_unsigned_candidate(
    work: &SubmissionWork,
) -> ProposedStateOutcome<CandidateWork, PendingEvmSubmissionFailure> {
    let request = &work.prepared.intent.derived.request;
    let Some(fee) = request
        .candidate_family()
        .candidates()
        .get(usize::from(work.next_candidate_ordinal))
    else {
        return ProposedStateOutcome::Failure(reconcile_reserved(
            work,
            EvmSubmissionFailure::ReplacementPolicyExhausted,
        ));
    };
    let envelope = match request
        .transaction_intent()
        .unsigned_candidate(work.reservation.nonce, fee)
    {
        Ok(value) => value,
        Err(_) => {
            return ProposedStateOutcome::Failure(reconcile_reserved(
                work,
                EvmSubmissionFailure::DestinationRejected,
            ));
        }
    };
    ProposedStateOutcome::Success(CandidateWork {
        work: work.clone(),
        unsigned_candidate: UnsignedWalletCandidate {
            transaction_intent: request.transaction_intent().clone(),
            semantic_reservation_key: work.prepared.reservation_key.clone(),
            nonce: work.reservation.nonce,
            candidate_ordinal: work.next_candidate_ordinal,
            fee: fee.clone(),
            unsigned_candidate_digest: format!("{:#x}", envelope.signing_digest()),
        },
        attested_candidate: None,
    })
}

pub(crate) fn attest_candidate_request(
    candidate: &CandidateWork,
) -> crate::AttestCandidateIdentityRequest {
    crate::AttestCandidateIdentityRequest {
        candidate: candidate.unsigned_candidate.clone(),
    }
}

pub(crate) fn settle_candidate_attestation(
    candidate: &CandidateWork,
    observation: &CommittedObservation<AttestedWalletCandidate, EvmSubmissionFailure>,
) -> StateSettlement<CandidateWork, PendingEvmSubmissionFailure> {
    match observation {
        CommittedObservation::SafeFailure(failure) => StateSettlement::Proposed(
            ProposedStateOutcome::Failure(reconcile_reserved(&candidate.work, *failure)),
        ),
        CommittedObservation::Returned(attested)
            if attestation_matches_candidate(attested, &candidate.unsigned_candidate) =>
        {
            let mut candidate = candidate.clone();
            candidate.attested_candidate = Some(attested.clone());
            StateSettlement::Proposed(ProposedStateOutcome::Success(candidate))
        }
        CommittedObservation::Returned(_) => StateSettlement::InvalidEvidence,
    }
}

fn attestation_matches_candidate(
    attested: &AttestedWalletCandidate,
    candidate: &UnsignedWalletCandidate,
) -> bool {
    attested.semantic_reservation_key == candidate.semantic_reservation_key
        && attested.candidate_ordinal == candidate.candidate_ordinal
        && attested.unsigned_candidate_digest == candidate.unsigned_candidate_digest
        && attested.semantic_signer_id == candidate.transaction_intent.semantic_signer_id()
        && attested.signing_profile_contract_ref
            == *candidate.transaction_intent.signing_profile_contract_ref()
        && attested.candidate_descriptor_ref.to_content_ref().is_ok()
        && attested.signer_attestation_ref.to_content_ref().is_ok()
        && crate::submission::validate_transaction_hash(&attested.transaction_hash)
}

pub(crate) fn derive_candidate_activation_permit(
    candidate: &CandidateWork,
) -> ProposedStateOutcome<PermittedCandidateWork, PendingEvmSubmissionFailure> {
    if candidate.attested_candidate.is_none() {
        return ProposedStateOutcome::Failure(reconcile_reserved(
            &candidate.work,
            EvmSubmissionFailure::SignerUnavailable,
        ));
    }
    if candidate.work.current_candidate.as_ref() != candidate.work.activated_candidates.last() {
        return ProposedStateOutcome::Failure(reconcile_reserved(
            &candidate.work,
            EvmSubmissionFailure::ReplacementPolicyExhausted,
        ));
    }
    let permit = match derive_exact_candidate_activation_permit(
        &candidate.work.reservation,
        &candidate.work.activated_candidates,
        candidate.work.next_candidate_ordinal,
    ) {
        Ok(permit) => permit,
        Err(_) => {
            return ProposedStateOutcome::Failure(reconcile_reserved(
                &candidate.work,
                EvmSubmissionFailure::ReplacementPolicyExhausted,
            ));
        }
    };
    ProposedStateOutcome::Success(PermittedCandidateWork {
        candidate: candidate.clone(),
        activation_permit: permit,
    })
}

pub(crate) fn derive_candidate_operation_key(
    permitted: &PermittedCandidateWork,
) -> ProposedStateOutcome<PreparedCandidateActivation, PendingEvmSubmissionFailure> {
    let candidate = &permitted.candidate;
    let Some(attested) = candidate.attested_candidate.as_ref() else {
        return ProposedStateOutcome::Failure(reconcile_reserved(
            &candidate.work,
            EvmSubmissionFailure::SignerUnavailable,
        ));
    };
    let key = match derive_evm_candidate_operation_key(
        &candidate.work.prepared.reservation_key,
        candidate.unsigned_candidate.candidate_ordinal,
    ) {
        Ok(value) => value,
        Err(_) => {
            return ProposedStateOutcome::Failure(reconcile_reserved(
                &candidate.work,
                EvmSubmissionFailure::ReplacementPolicyExhausted,
            ));
        }
    };
    ProposedStateOutcome::Success(PreparedCandidateActivation {
        candidate: candidate.clone(),
        activation_request: ActivateEvmCandidateRequest {
            nonce_domain: candidate.work.prepared.intent.derived.nonce_domain.clone(),
            candidate_operation_key: key,
            next_candidate: attested.clone(),
            activation_permit: permitted.activation_permit.clone(),
        },
    })
}

pub(crate) fn activate_candidate_request(
    prepared: &PreparedCandidateActivation,
) -> ActivateEvmCandidateRequest {
    prepared.activation_request.clone()
}

pub(crate) fn settle_candidate_activation(
    prepared: &PreparedCandidateActivation,
    observation: &CommittedObservation<ActivateCandidateResponse, EvmSubmissionFailure>,
) -> StateSettlement<CandidateActivationDecision, PendingEvmSubmissionFailure> {
    let candidate = &prepared.candidate;
    match observation {
        CommittedObservation::SafeFailure(failure) => StateSettlement::Proposed(
            ProposedStateOutcome::Failure(reconcile_reserved(&candidate.work, *failure)),
        ),
        CommittedObservation::Returned(ActivateCandidateResponse::CandidateProgressionConflict) => {
            StateSettlement::Proposed(ProposedStateOutcome::Success(
                CandidateActivationDecision::Reconcile,
            ))
        }
        CommittedObservation::Returned(ActivateCandidateResponse::Activated {
            candidate: active,
        }) if candidate
            .attested_candidate
            .as_ref()
            .is_some_and(|attested| active.attested_candidate == *attested)
            && active.activation_evidence_ref.to_content_ref().is_ok() =>
        {
            StateSettlement::Proposed(ProposedStateOutcome::Success(
                CandidateActivationDecision::Activated {
                    active: ActiveCandidateWork {
                        candidate: candidate.clone(),
                        active_candidate: active.clone(),
                    },
                },
            ))
        }
        CommittedObservation::Returned(_) => StateSettlement::InvalidEvidence,
    }
}

pub(crate) fn mark_activation_reconcile(
    prepared: &PreparedCandidateActivation,
) -> ProposedStateOutcome<PreparedWalletSubmission, mfm_program::structured::Never> {
    ProposedStateOutcome::Success(prepared.candidate.work.prepared.clone())
}

pub(crate) fn broadcast_request(
    active: &ActiveCandidateWork,
) -> crate::BroadcastExactCandidateRequest {
    crate::BroadcastExactCandidateRequest {
        unsigned_candidate: active.candidate.unsigned_candidate.clone(),
        active_candidate: active.active_candidate.clone(),
        route_generation_ref: active
            .candidate
            .work
            .prepared
            .intent
            .derived
            .request
            .route_generation_ref()
            .clone(),
    }
}

pub(crate) fn settle_broadcast(
    active: &ActiveCandidateWork,
    observation: &CommittedObservation<SubmittedCandidateProof, EvmSubmissionFailure>,
) -> StateSettlement<CandidateObservationWork, PendingEvmSubmissionFailure> {
    match observation {
        CommittedObservation::SafeFailure(failure) => StateSettlement::Proposed(
            ProposedStateOutcome::Failure(reconcile_reserved(&active.candidate.work, *failure)),
        ),
        CommittedObservation::Returned(submitted)
            if submitted.candidate_ordinal
                == active.active_candidate.attested_candidate.candidate_ordinal
                && submitted.unsigned_candidate_digest
                    == active
                        .active_candidate
                        .attested_candidate
                        .unsigned_candidate_digest
                && submitted.transaction_hash
                    == active.active_candidate.attested_candidate.transaction_hash
                && submitted.semantic_signer_id
                    == active
                        .active_candidate
                        .attested_candidate
                        .semantic_signer_id
                && submitted.signer_generation_ref.to_content_ref().is_ok()
                && submitted.signing_contract_ref.to_content_ref().is_ok()
                && submitted.submission_contract_ref
                    == *active
                        .candidate
                        .unsigned_candidate
                        .transaction_intent
                        .submission_contract_ref() =>
        {
            StateSettlement::Proposed(ProposedStateOutcome::Success(CandidateObservationWork {
                active: active.clone(),
                submitted: submitted.clone(),
                next_round: 0,
                observation: CandidateTransactionObservation {
                    transaction: None,
                    receipt: None,
                    finalized_head: None,
                    inclusion_block: None,
                },
            }))
        }
        CommittedObservation::Returned(_) => StateSettlement::InvalidEvidence,
    }
}

pub(crate) fn select_observation_round(
    work: &CandidateObservationWork,
) -> ProposedStateOutcome<ObservationRoundDecision, mfm_program::structured::Never> {
    let rounds = work
        .active
        .candidate
        .work
        .prepared
        .intent
        .derived
        .request
        .observation_rounds();
    ProposedStateOutcome::Success(if work.next_round < rounds {
        ObservationRoundDecision::Observe
    } else {
        ObservationRoundDecision::Skip
    })
}

fn candidate_hash(work: &CandidateObservationWork) -> &str {
    &work
        .active
        .active_candidate
        .attested_candidate
        .transaction_hash
}

fn route_generation(work: &CandidateObservationWork) -> EvmWalletReference {
    work.active
        .candidate
        .work
        .prepared
        .intent
        .derived
        .request
        .route_generation_ref()
        .clone()
}

pub(crate) fn transaction_lookup_request(
    work: &CandidateObservationWork,
) -> EvmTransactionLookupRequest {
    EvmTransactionLookupRequest {
        route_generation_ref: route_generation(work),
        transaction_hash: candidate_hash(work).to_owned(),
    }
}

pub(crate) fn settle_transaction_lookup(
    work: &CandidateObservationWork,
    observation: &CommittedObservation<EvmTransactionLookupObservation, EvmSubmissionFailure>,
) -> StateSettlement<CandidateObservationWork, PendingEvmSubmissionFailure> {
    match observation {
        CommittedObservation::SafeFailure(failure) => {
            StateSettlement::Proposed(ProposedStateOutcome::Failure(reconcile_reserved(
                &work.active.candidate.work,
                *failure,
            )))
        }
        CommittedObservation::Returned(returned)
            if valid_transaction_observation(candidate_hash(work), returned) =>
        {
            let mut work = work.clone();
            work.observation.transaction = Some(returned.clone());
            StateSettlement::Proposed(ProposedStateOutcome::Success(work))
        }
        CommittedObservation::Returned(_) => StateSettlement::InvalidEvidence,
    }
}

fn valid_transaction_observation(
    expected_hash: &str,
    observation: &EvmTransactionLookupObservation,
) -> bool {
    match observation {
        EvmTransactionLookupObservation::Missing => true,
        EvmTransactionLookupObservation::Found {
            transaction_hash,
            block_number,
            block_hash,
        } => {
            transaction_hash == expected_hash
                && crate::submission::validate_transaction_hash(transaction_hash)
                && match (block_number, block_hash) {
                    (None, None) => true,
                    (Some(number), Some(hash)) => {
                        crate::submission::validate_quantity(number)
                            && crate::submission::validate_transaction_hash(hash)
                    }
                    _ => false,
                }
        }
    }
}

pub(crate) fn receipt_lookup_request(work: &CandidateObservationWork) -> EvmReceiptLookupRequest {
    EvmReceiptLookupRequest {
        route_generation_ref: route_generation(work),
        transaction_hash: candidate_hash(work).to_owned(),
    }
}

pub(crate) fn settle_receipt_lookup(
    work: &CandidateObservationWork,
    observation: &CommittedObservation<EvmReceiptLookupObservation, EvmSubmissionFailure>,
) -> StateSettlement<CandidateObservationWork, PendingEvmSubmissionFailure> {
    match observation {
        CommittedObservation::SafeFailure(failure) => {
            StateSettlement::Proposed(ProposedStateOutcome::Failure(reconcile_reserved(
                &work.active.candidate.work,
                *failure,
            )))
        }
        CommittedObservation::Returned(returned)
            if valid_receipt_observation(candidate_hash(work), returned) =>
        {
            let mut work = work.clone();
            work.observation.receipt = Some(returned.clone());
            work.next_round = work.next_round.saturating_add(1);
            StateSettlement::Proposed(ProposedStateOutcome::Success(work))
        }
        CommittedObservation::Returned(_) => StateSettlement::InvalidEvidence,
    }
}

fn valid_receipt_observation(
    expected_hash: &str,
    observation: &EvmReceiptLookupObservation,
) -> bool {
    match observation {
        EvmReceiptLookupObservation::Missing => true,
        EvmReceiptLookupObservation::Found {
            transaction_hash,
            block_number,
            block_hash,
            status,
        } => {
            transaction_hash == expected_hash
                && crate::submission::validate_transaction_hash(transaction_hash)
                && crate::submission::validate_quantity(block_number)
                && crate::submission::validate_transaction_hash(block_hash)
                && matches!(status, 0 | 1)
        }
    }
}

pub(crate) fn select_terminal_evidence(
    work: &CandidateObservationWork,
) -> ProposedStateOutcome<TerminalEvidenceDecision, mfm_program::structured::Never> {
    let terminal = match (&work.observation.transaction, &work.observation.receipt) {
        (
            Some(EvmTransactionLookupObservation::Found {
                transaction_hash: transaction_hash_left,
                block_number: Some(transaction_number),
                block_hash: Some(transaction_block_hash),
            }),
            Some(EvmReceiptLookupObservation::Found {
                transaction_hash: transaction_hash_right,
                block_number: receipt_number,
                block_hash: receipt_block_hash,
                ..
            }),
        ) if transaction_hash_left == transaction_hash_right
            && transaction_number == receipt_number
            && transaction_block_hash == receipt_block_hash =>
        {
            TerminalEvidenceDecision::ObserveFinality
        }
        _ => TerminalEvidenceDecision::Reconcile,
    };
    ProposedStateOutcome::Success(terminal)
}

pub(crate) fn mark_observation_reconcile(
    work: &CandidateObservationWork,
) -> ProposedStateOutcome<PreparedWalletSubmission, mfm_program::structured::Never> {
    ProposedStateOutcome::Success(work.active.candidate.work.prepared.clone())
}

pub(crate) fn finalized_head_request(
    work: &CandidateObservationWork,
) -> crate::EvmFinalizedHeadRequest {
    crate::EvmFinalizedHeadRequest {
        route_generation_ref: route_generation(work),
    }
}

pub(crate) fn settle_finalized_head(
    work: &CandidateObservationWork,
    observation: &CommittedObservation<EvmFinalizedHeadObservation, EvmSubmissionFailure>,
) -> StateSettlement<TerminalEvidenceWork, PendingEvmSubmissionFailure> {
    match observation {
        CommittedObservation::SafeFailure(failure) => {
            StateSettlement::Proposed(ProposedStateOutcome::Failure(reconcile_reserved(
                &work.active.candidate.work,
                *failure,
            )))
        }
        CommittedObservation::Returned(head)
            if crate::submission::validate_quantity(&head.block_number)
                && crate::submission::validate_transaction_hash(&head.block_hash) =>
        {
            let mut candidate = work.clone();
            candidate.observation.finalized_head = Some(head.clone());
            StateSettlement::Proposed(ProposedStateOutcome::Success(TerminalEvidenceWork {
                candidate,
            }))
        }
        CommittedObservation::Returned(_) => StateSettlement::InvalidEvidence,
    }
}

pub(crate) fn inclusion_block_request(
    terminal: &TerminalEvidenceWork,
) -> crate::EvmInclusionBlockRequest {
    let block_number = match &terminal.candidate.observation.receipt {
        Some(EvmReceiptLookupObservation::Found { block_number, .. }) => block_number.clone(),
        _ => String::from("0"),
    };
    crate::EvmInclusionBlockRequest {
        route_generation_ref: route_generation(&terminal.candidate),
        block_number,
    }
}

pub(crate) fn settle_inclusion_block(
    terminal: &TerminalEvidenceWork,
    observation: &CommittedObservation<EvmInclusionBlockObservation, EvmSubmissionFailure>,
) -> StateSettlement<TerminalEvidenceWork, PendingEvmSubmissionFailure> {
    match observation {
        CommittedObservation::SafeFailure(failure) => {
            StateSettlement::Proposed(ProposedStateOutcome::Failure(reconcile_reserved(
                &terminal.candidate.active.candidate.work,
                *failure,
            )))
        }
        CommittedObservation::Returned(block)
            if crate::submission::validate_quantity(&block.block_number)
                && crate::submission::validate_transaction_hash(&block.block_hash) =>
        {
            let mut terminal = terminal.clone();
            terminal.candidate.observation.inclusion_block = Some(block.clone());
            StateSettlement::Proposed(ProposedStateOutcome::Success(terminal))
        }
        CommittedObservation::Returned(_) => StateSettlement::InvalidEvidence,
    }
}

pub(crate) fn verify_canonical_inclusion(
    terminal: &TerminalEvidenceWork,
) -> ProposedStateOutcome<CompletionWork, PendingEvmSubmissionFailure> {
    let work = &terminal.candidate;
    let Some(EvmTransactionLookupObservation::Found {
        transaction_hash,
        block_number: Some(transaction_number),
        block_hash: Some(transaction_block_hash),
    }) = work.observation.transaction.as_ref()
    else {
        return canonical_inclusion_failure(terminal);
    };
    let Some(EvmReceiptLookupObservation::Found {
        transaction_hash: receipt_hash,
        block_number: receipt_number,
        block_hash: receipt_block_hash,
        status,
    }) = work.observation.receipt.as_ref()
    else {
        return canonical_inclusion_failure(terminal);
    };
    let (Some(finalized_head), Some(inclusion_block)) = (
        work.observation.finalized_head.as_ref(),
        work.observation.inclusion_block.as_ref(),
    ) else {
        return canonical_inclusion_failure(terminal);
    };
    let finalized_number = match U256::from_str(&finalized_head.block_number) {
        Ok(value) => value,
        Err(_) => {
            return canonical_inclusion_failure(terminal);
        }
    };
    let inclusion_number = match U256::from_str(receipt_number) {
        Ok(value) => value,
        Err(_) => {
            return canonical_inclusion_failure(terminal);
        }
    };
    if transaction_hash != receipt_hash
        || transaction_number != receipt_number
        || transaction_block_hash != receipt_block_hash
        || inclusion_block.block_number != *receipt_number
        || inclusion_block.block_hash != *receipt_block_hash
        || finalized_number < inclusion_number
    {
        return canonical_inclusion_failure(terminal);
    }
    let prepared = &work.active.candidate.work.prepared;
    let reservation = &work.active.candidate.work.reservation;
    let completion_key = match derive_evm_nonce_completion_key(&prepared.reservation_key) {
        Ok(value) => value,
        Err(_) => {
            return canonical_inclusion_failure(terminal);
        }
    };
    let public_result = if *status == 1 {
        "{\"execution_disposition\":\"succeeded\"}"
    } else {
        "{\"execution_disposition\":\"reverted\"}"
    };
    let canonical_terminal_outcome = CanonicalTerminalOutcome {
        nonce_domain: reservation.nonce_domain.clone(),
        semantic_reservation_key: prepared.reservation_key.clone(),
        submission_intent_id: prepared.intent.submission_intent_id.clone(),
        transaction_intent_digest: prepared
            .intent
            .derived
            .request
            .transaction_intent()
            .digest()
            .to_owned(),
        nonce: reservation.nonce,
        winning_candidate_ordinal: work
            .active
            .active_candidate
            .attested_candidate
            .candidate_ordinal,
        winning_activation_evidence_ref: work
            .active
            .active_candidate
            .activation_evidence_ref
            .clone(),
        transaction_hash: transaction_hash.clone(),
        inclusion_block_number: receipt_number.clone(),
        inclusion_block_hash: receipt_block_hash.clone(),
        terminal_assurance_contract_ref: prepared
            .intent
            .derived
            .request
            .transaction_intent()
            .terminal_assurance_contract_ref()
            .clone(),
        execution_disposition: if *status == 1 {
            ExecutionDisposition::Succeeded
        } else {
            ExecutionDisposition::Reverted
        },
        canonical_public_result: public_result.to_owned(),
    };
    let (Some(transaction), Some(receipt)) = (
        work.observation.transaction.clone(),
        work.observation.receipt.clone(),
    ) else {
        return canonical_inclusion_failure(terminal);
    };
    let terminal_witnesses = TerminalWitnesses {
        transaction,
        receipt,
        finalized_head: finalized_head.clone(),
        inclusion_block: inclusion_block.clone(),
        terminal_assurance_contract_ref: canonical_terminal_outcome
            .terminal_assurance_contract_ref
            .clone(),
        canonical_public_result: public_result.to_owned(),
    };
    if terminal_witnesses
        .validate_against(&canonical_terminal_outcome)
        .is_err()
    {
        return canonical_inclusion_failure(terminal);
    }
    ProposedStateOutcome::Success(CompletionWork {
        request: CompleteEvmNonceRequest {
            nonce_domain: reservation.nonce_domain.clone(),
            completion_key,
            current_reservation: reservation.clone(),
            canonical_terminal_outcome,
            terminal_witnesses,
        },
        submission_work: work.active.candidate.work.clone(),
    })
}

fn canonical_inclusion_failure(
    terminal: &TerminalEvidenceWork,
) -> ProposedStateOutcome<CompletionWork, PendingEvmSubmissionFailure> {
    ProposedStateOutcome::Failure(reconcile_reserved(
        &terminal.candidate.active.candidate.work,
        EvmSubmissionFailure::ObservationPolicyExhausted,
    ))
}

pub(crate) fn completion_request(work: &CompletionWork) -> CompleteEvmNonceRequest {
    work.request.clone()
}

pub(crate) fn settle_completion(
    work: &CompletionWork,
    observation: &CommittedObservation<CompleteWalletNonceResponse, EvmSubmissionFailure>,
) -> StateSettlement<CompletedWalletNonce, PendingEvmSubmissionFailure> {
    match observation {
        CommittedObservation::SafeFailure(failure) => StateSettlement::Proposed(
            ProposedStateOutcome::Failure(reconcile_reserved(&work.submission_work, *failure)),
        ),
        CommittedObservation::Returned(CompleteWalletNonceResponse::Completed { completion })
            if completion.nonce_domain == work.request.nonce_domain
                && completion.semantic_completion_key == work.request.completion_key
                && completion.semantic_reservation_key
                    == work.request.current_reservation.semantic_reservation_key
                && completion.canonical_terminal_outcome
                    == work.request.canonical_terminal_outcome
                && completion.completion_evidence_ref.to_content_ref().is_ok() =>
        {
            StateSettlement::Proposed(ProposedStateOutcome::Success(completion.clone()))
        }
        CommittedObservation::Returned(_) => StateSettlement::InvalidEvidence,
    }
}

pub(crate) fn mark_candidate_completed(
    completion: &CompletedWalletNonce,
) -> ProposedStateOutcome<CandidateResolution, mfm_program::structured::Never> {
    ProposedStateOutcome::Success(CandidateResolution::Completed {
        completion: completion.clone(),
    })
}

pub(crate) fn mark_submission_completed(
    completion: &CompletedWalletNonce,
) -> ProposedStateOutcome<SubmissionProgress, mfm_program::structured::Never> {
    ProposedStateOutcome::Success(SubmissionProgress {
        work: None,
        completion: Some(completion.clone()),
        failure: None,
    })
}

pub(crate) fn mark_submission_resumed(
    work: &SubmissionWork,
) -> ProposedStateOutcome<SubmissionProgress, mfm_program::structured::Never> {
    ProposedStateOutcome::Success(SubmissionProgress {
        work: Some(work.clone()),
        completion: None,
        failure: None,
    })
}

pub(crate) fn select_submission_terminal(
    progress: &SubmissionProgress,
) -> ProposedStateOutcome<SubmissionTerminalDecision, mfm_program::structured::Never> {
    let decision = if let Some(completion) = &progress.completion {
        SubmissionTerminalDecision::Completed {
            completion: completion.clone(),
        }
    } else if let Some(failure) = progress.failure {
        if failure == EvmSubmissionFailure::ReplacementPolicyExhausted {
            SubmissionTerminalDecision::Exhausted { failure }
        } else {
            SubmissionTerminalDecision::Failed { failure }
        }
    } else {
        SubmissionTerminalDecision::Exhausted {
            failure: EvmSubmissionFailure::ReplacementPolicyExhausted,
        }
    };
    ProposedStateOutcome::Success(decision)
}

pub(crate) fn project_completed(
    completion: &CompletedWalletNonce,
) -> ProposedStateOutcome<CompletedProjection, mfm_program::structured::Never> {
    ProposedStateOutcome::Success(
        match completion.canonical_terminal_outcome.execution_disposition {
            ExecutionDisposition::Succeeded => CompletedProjection::Success {
                output: EvmSubmissionOutput {
                    completion: completion.clone(),
                },
            },
            ExecutionDisposition::Reverted => CompletedProjection::Failure {
                failure: EvmSubmissionFailure::ExecutionReverted,
            },
        },
    )
}
