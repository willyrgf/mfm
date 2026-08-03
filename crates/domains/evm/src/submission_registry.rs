//! Process qualification for the structured EVM submission expansion.

use std::str::FromStr;
use std::sync::Arc;

use alloy_primitives::{Address, B256, U256};
use mfm_capabilities::{
    AccessFaultCode, CapabilityContractFault, EffectCapabilityImplementation,
    ReadCapabilityImplementation,
};
use mfm_certify::structured::ProgramRegistryBuilder;
use mfm_ids::{ContentDigest, ContentRef, StableId, TenantScopeId};
use mfm_program::structured::{
    state_contract, CommittedObservation, State, StateFrame, StateSettlement,
    StructuredStateCallbacks,
};
use mfm_spec::structured::{
    ProposedStateOutcome, ProposedStateValue, SecretFreeImplementationDescriptor,
    StructuredComponentKind,
};

use crate::submission::{
    ActivateWalletCandidateState, ActiveCandidateWork, AttestCandidateIdentityCapability,
    AttestCandidateIdentityState, BroadcastExactCandidateCapability, BroadcastExactCandidateState,
    BroadcastLineageHead, BuildUnsignedCandidateState, CandidateActivationDecision,
    CandidateObservationWork, CandidateResolution, CandidateSlotDecision,
    CandidateTransactionObservation, CandidateWork, CollapseWalletStatusState,
    CompleteWalletNonceState, CompletedProjection, CompletionWork,
    DeriveCandidateActivationPermitState, DeriveEvmCandidateOperationKeyState,
    DeriveEvmNonceReservationKeyState, DeriveSubmissionIntentIdState, DeriveTransactionIntentState,
    DerivedSubmissionDomain, EvmFinalizedHeadCapability, EvmFinalizedHeadObservation,
    EvmFinalizedHeadRequest, EvmInclusionBlockCapability, EvmInclusionBlockObservation,
    EvmInclusionBlockRequest, EvmPendingNonceCapability, EvmPendingNonceRequest,
    EvmReceiptLookupCapability, EvmReceiptLookupObservation, EvmReceiptLookupRequest,
    EvmSubmissionConfiguration, EvmSubmissionExpansion, EvmSubmissionOutput, EvmSubmissionRequest,
    EvmTransactionLookupCapability, EvmTransactionLookupObservation, EvmTransactionLookupRequest,
    FailureReconciliationRequest, IntentBoundSubmission, MarkActivationReconcileState,
    MarkCandidateCompletedState, MarkCandidateFamilyExhaustedState, MarkObservationReconcileState,
    MarkSubmissionCompletedState, MarkSubmissionResumedState, ObservationRoundDecision,
    ObserveActivatedTransactionState, ObserveCandidateReceiptState, ObserveCanonicalInclusionState,
    ObserveFinalizedHeadState, ObservePendingNonceState, ObservedPendingSubmission,
    PendingEvmSubmissionFailure, PermittedCandidateWork, PostReservePreparedSubmission,
    PreparedCandidateActivation, PreparedWalletSubmission, ProjectCompletedWalletDispositionState,
    QualifiedPendingSubmission, QualifyPendingNonceFloorState,
    ReadCandidateStatusAfterFailureState, ReadCandidateWalletNonceStatusState,
    ReadPostReserveWalletNonceStatusState, ReadReservationStatusAfterFailureState,
    ReadWalletNonceStatusState, ReserveWalletNonceState, SelectCandidateSlotState,
    SelectObservationRoundState, SelectSubmissionTerminalState, SelectTerminalEvidenceState,
    StructuredSubmitEvmTransactionState, SubmissionProgress, SubmissionTerminalDecision,
    SubmissionWork, TerminalEvidenceDecision, TerminalEvidenceWork, UnsignedWalletCandidate,
    VerifyCanonicalInclusionState, WalletStatusBaseline, WalletStatusDecision,
};
use crate::submission_expansion::{
    candidate_attempt_recipe, initial_reservation_recipe, HandlePendingEvmSubmissionFailureState,
    MapEvmSubmissionFailureState, MapPendingEvmSubmissionFailureState,
    PendingEvmSubmissionFailureRoute, SubmissionFailureRoute,
};
use crate::submission_process;
use crate::{
    canonical_wallet_reference, derive_evm_candidate_operation_key, derive_evm_chain_lineage_id,
    derive_evm_nonce_completion_key, derive_evm_nonce_reservation_key, derive_wallet_nonce_domain,
    evm_wallet_assurance_policy_ref, evm_wallet_nonce_policy_ref, ActivateCandidateResponse,
    ActivateEvmCandidateRequest, ActivateWalletCandidateCapability, ActiveWalletCandidate,
    AttestCandidateIdentityRequest, AttestedWalletCandidate, BroadcastExactCandidateRequest,
    CandidateActivationPermit, CanonicalTerminalOutcome, ChainInstanceDeclaration,
    ChainInstanceRegistryAttestation, CompleteEvmNonceRequest, CompleteWalletNonceCapability,
    CompleteWalletNonceResponse, CompletedWalletNonce, EvmCallerSubmissionToken,
    EvmCandidateFamily, EvmSubmissionFailure, EvmTransactionIntent, EvmTransactionTarget,
    EvmWalletFeeCandidate, EvmWalletTransactionAction, EvmWalletTransactionTemplate,
    ExclusiveCurrentControl, ObservedPendingNonceFloor, PriorEffectDisposition,
    PriorResourceDisposition, ReadEvmWalletNonceStatusRequest, ReadWalletNonceStatusCapability,
    ReplayExclusionDisposition, ReserveEvmNonceRequest, ReserveWalletNonceCapability,
    ReserveWalletNonceResponse, ReservedWalletNonce, SubmittedCandidateProof,
    WalletNonceDomainActivationAttestation, WalletNonceDomainActivationRecord, WalletNonceStatus,
    WalletNonceStoreLineageHead,
};

/// Registered executable and qualification objects shared by every EVM
/// submission process implementation descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvmSubmissionProcessQualification {
    executable_identity_ref: ContentRef,
    qualification_artifact_ref: ContentRef,
}

impl EvmSubmissionProcessQualification {
    /// Binds already-registered secret-free process qualification objects.
    pub const fn new(
        executable_identity_ref: ContentRef,
        qualification_artifact_ref: ContentRef,
    ) -> Self {
        Self {
            executable_identity_ref,
            qualification_artifact_ref,
        }
    }

    /// Returns the exact executable identity object.
    pub const fn executable_identity_ref(&self) -> &ContentRef {
        &self.executable_identity_ref
    }

    /// Returns the exact qualification artifact object.
    pub const fn qualification_artifact_ref(&self) -> &ContentRef {
        &self.qualification_artifact_ref
    }
}

/// Callback-free semantic validator shared by the structured EVM
/// capabilities.
#[derive(Debug, Clone)]
pub struct EvmSubmissionCapabilityImplementation {
    contract_fault_code: StableId,
}

impl EvmSubmissionCapabilityImplementation {
    /// Constructs the one reviewed redaction-safe contract-fault classifier.
    pub fn new() -> Result<Self, crate::WalletAuthorityContractError> {
        let contract_fault_code = StableId::new("mfm.evm/structured-capability-contract-fault")
            .map_err(|_| {
                crate::WalletAuthorityContractError::Invalid("capability_contract_fault")
            })?;
        Ok(Self {
            contract_fault_code,
        })
    }

    fn reject(&self) -> CapabilityContractFault {
        CapabilityContractFault::new(self.contract_fault_code.clone())
    }

    fn require(&self, valid: bool) -> Result<(), CapabilityContractFault> {
        if valid {
            Ok(())
        } else {
            Err(self.reject())
        }
    }
}

trait SubmissionStateProcess: State {
    fn callbacks(fixture: &QualificationFixture) -> StructuredStateCallbacks<Self>
    where
        Self: Sized;
}

macro_rules! pure_process {
    ($state:ty, $callback:path) => {
        impl SubmissionStateProcess for $state {
            fn callbacks(_fixture: &QualificationFixture) -> StructuredStateCallbacks<Self> {
                StructuredStateCallbacks::Pure {
                    apply: Arc::new(|frame: StateFrame<'_, <Self as State>::Input>| {
                        $callback(frame.input())
                    }),
                }
            }
        }
    };
}

macro_rules! read_process {
    ($state:ty, $fixture:ident, $request:path, $settle:path, [$($failure:expr),+ $(,)?]) => {
        impl SubmissionStateProcess for $state {
            fn callbacks(_fixture: &QualificationFixture) -> StructuredStateCallbacks<Self> {
                StructuredStateCallbacks::Read {
                    request: Arc::new(
                        |frame: StateFrame<'_, <Self as State>::Input>| $request(frame.input()),
                    ),
                    settle_returned: Arc::new(
                        |frame: StateFrame<'_, <Self as State>::Input>, returned| {
                            $settle(
                                frame.input(),
                                &CommittedObservation::Returned(returned.clone()),
                            )
                        },
                    ),
                    settle_safe_failure: Arc::new(
                        |frame: StateFrame<'_, <Self as State>::Input>, failure| {
                            match $settle(
                                frame.input(),
                                &CommittedObservation::SafeFailure(failure.clone()),
                            ) {
                                StateSettlement::Proposed(outcome) => outcome,
                                StateSettlement::InvalidEvidence => {
                                    unreachable!(
                                        "safe-failure settlement cannot produce InvalidEvidence"
                                    )
                                }
                            }
                        },
                    ),
                }
            }
        }
    };
}

macro_rules! reconciling_read_process {
    ($state:ty, $fixture:ident, $request:path, $settle:path, [$($failure:expr),+ $(,)?]) => {
        impl SubmissionStateProcess for $state {
            fn callbacks(_fixture: &QualificationFixture) -> StructuredStateCallbacks<Self> {
                StructuredStateCallbacks::Read {
                    request: Arc::new(
                        |frame: StateFrame<'_, <Self as State>::Input>| $request(frame.input()),
                    ),
                    settle_returned: Arc::new(
                        |frame: StateFrame<'_, <Self as State>::Input>, returned| {
                            $settle(
                                frame.input(),
                                &CommittedObservation::Returned(returned.clone()),
                            )
                        },
                    ),
                    settle_safe_failure: Arc::new(
                        |frame: StateFrame<'_, <Self as State>::Input>, failure| {
                            match $settle(
                                frame.input(),
                                &CommittedObservation::SafeFailure(failure.clone()),
                            ) {
                                StateSettlement::Proposed(outcome) => outcome,
                                StateSettlement::InvalidEvidence => {
                                    unreachable!(
                                        "safe-failure settlement cannot produce InvalidEvidence"
                                    )
                                }
                            }
                        },
                    ),
                }
            }
        }
    };
}

macro_rules! reconciling_effect_process {
    ($state:ty, $fixture:ident, $request:path, $settle:path, [$($failure:expr),+ $(,)?]) => {
        impl SubmissionStateProcess for $state {
            fn callbacks(_fixture: &QualificationFixture) -> StructuredStateCallbacks<Self> {
                StructuredStateCallbacks::Effect {
                    request: Arc::new(
                        |frame: StateFrame<'_, <Self as State>::Input>| $request(frame.input()),
                    ),
                    settle_returned: Arc::new(
                        |frame: StateFrame<'_, <Self as State>::Input>, returned| {
                            $settle(
                                frame.input(),
                                &CommittedObservation::Returned(returned.clone()),
                            )
                        },
                    ),
                    settle_safe_failure: Arc::new(
                        |frame: StateFrame<'_, <Self as State>::Input>, failure| {
                            match $settle(
                                frame.input(),
                                &CommittedObservation::SafeFailure(failure.clone()),
                            ) {
                                StateSettlement::Proposed(outcome) => outcome,
                                StateSettlement::InvalidEvidence => {
                                    unreachable!(
                                        "safe-failure settlement cannot produce InvalidEvidence"
                                    )
                                }
                            }
                        },
                    ),
                }
            }
        }
    };
}

pure_process!(
    MapEvmSubmissionFailureState,
    submission_process::map_submission_failure
);
pure_process!(
    MapPendingEvmSubmissionFailureState,
    submission_process::map_pending_submission_failure
);
pure_process!(
    HandlePendingEvmSubmissionFailureState,
    submission_process::handle_pending_submission_failure
);
pure_process!(
    DeriveTransactionIntentState,
    submission_process::derive_transaction_intent
);
pure_process!(
    DeriveSubmissionIntentIdState,
    submission_process::derive_submission_intent
);
pure_process!(
    DeriveEvmNonceReservationKeyState,
    submission_process::derive_reservation_key
);
pure_process!(
    QualifyPendingNonceFloorState,
    submission_process::qualify_pending_nonce
);
pure_process!(
    CollapseWalletStatusState,
    submission_process::collapse_wallet_status
);
pure_process!(
    SelectCandidateSlotState,
    submission_process::select_candidate_slot
);
pure_process!(
    MarkCandidateFamilyExhaustedState,
    submission_process::mark_candidate_family_exhausted
);
pure_process!(
    BuildUnsignedCandidateState,
    submission_process::build_unsigned_candidate
);
pure_process!(
    DeriveCandidateActivationPermitState,
    submission_process::derive_candidate_activation_permit
);
pure_process!(
    DeriveEvmCandidateOperationKeyState,
    submission_process::derive_candidate_operation_key
);
pure_process!(
    SelectObservationRoundState,
    submission_process::select_observation_round
);
pure_process!(
    MarkActivationReconcileState,
    submission_process::mark_activation_reconcile
);
pure_process!(
    MarkObservationReconcileState,
    submission_process::mark_observation_reconcile
);
pure_process!(
    MarkCandidateCompletedState,
    submission_process::mark_candidate_completed
);
pure_process!(
    MarkSubmissionCompletedState,
    submission_process::mark_submission_completed
);
pure_process!(
    MarkSubmissionResumedState,
    submission_process::mark_submission_resumed
);
pure_process!(
    SelectTerminalEvidenceState,
    submission_process::select_terminal_evidence
);
pure_process!(
    VerifyCanonicalInclusionState,
    submission_process::verify_canonical_inclusion
);
pure_process!(
    ProjectCompletedWalletDispositionState,
    submission_process::project_completed
);
pure_process!(
    SelectSubmissionTerminalState,
    submission_process::select_submission_terminal
);

reconciling_read_process!(
    ReadWalletNonceStatusState,
    prepared,
    submission_process::read_status_request,
    submission_process::settle_wallet_status,
    [EvmSubmissionFailure::NonceAuthorityUnavailable]
);
reconciling_read_process!(
    ReadPostReserveWalletNonceStatusState,
    post_reserve,
    submission_process::post_reserve_status_request,
    submission_process::settle_post_reserve_wallet_status,
    [EvmSubmissionFailure::NonceAuthorityUnavailable]
);
read_process!(
    ReadReservationStatusAfterFailureState,
    reservation_reconciliation,
    submission_process::failure_reconciliation_status_request,
    submission_process::settle_reservation_failure_status,
    [EvmSubmissionFailure::NonceAuthorityUnavailable]
);
read_process!(
    ReadCandidateStatusAfterFailureState,
    candidate_reconciliation,
    submission_process::failure_reconciliation_status_request,
    submission_process::settle_candidate_failure_status,
    [EvmSubmissionFailure::NonceAuthorityUnavailable]
);
reconciling_read_process!(
    ReadCandidateWalletNonceStatusState,
    prepared,
    submission_process::read_status_request,
    submission_process::settle_candidate_wallet_status,
    [EvmSubmissionFailure::NonceAuthorityUnavailable]
);
reconciling_read_process!(
    ObservePendingNonceState,
    prepared,
    submission_process::pending_nonce_request,
    submission_process::settle_pending_nonce,
    [
        EvmSubmissionFailure::ProviderUnavailable,
        EvmSubmissionFailure::TransportUnavailable,
    ]
);
reconciling_effect_process!(
    ReserveWalletNonceState,
    qualified_pending,
    submission_process::reserve_nonce_request,
    submission_process::settle_reservation,
    [EvmSubmissionFailure::NonceAuthorityUnavailable]
);
reconciling_read_process!(
    AttestCandidateIdentityState,
    candidate,
    submission_process::attest_candidate_request,
    submission_process::settle_candidate_attestation,
    [EvmSubmissionFailure::SignerUnavailable]
);
reconciling_effect_process!(
    ActivateWalletCandidateState,
    prepared_activation,
    submission_process::activate_candidate_request,
    submission_process::settle_candidate_activation,
    [EvmSubmissionFailure::NonceAuthorityUnavailable]
);
reconciling_effect_process!(
    BroadcastExactCandidateState,
    active,
    submission_process::broadcast_request,
    submission_process::settle_broadcast,
    [
        EvmSubmissionFailure::DestinationRejected,
        EvmSubmissionFailure::ProviderUnavailable,
        EvmSubmissionFailure::SignerUnavailable,
        EvmSubmissionFailure::TransportUnavailable,
    ]
);
reconciling_read_process!(
    ObserveActivatedTransactionState,
    observation,
    submission_process::transaction_lookup_request,
    submission_process::settle_transaction_lookup,
    [
        EvmSubmissionFailure::ProviderUnavailable,
        EvmSubmissionFailure::TransportUnavailable,
    ]
);
reconciling_read_process!(
    ObserveCandidateReceiptState,
    observation,
    submission_process::receipt_lookup_request,
    submission_process::settle_receipt_lookup,
    [
        EvmSubmissionFailure::ProviderUnavailable,
        EvmSubmissionFailure::TransportUnavailable,
    ]
);
reconciling_read_process!(
    ObserveFinalizedHeadState,
    observation,
    submission_process::finalized_head_request,
    submission_process::settle_finalized_head,
    [
        EvmSubmissionFailure::ProviderUnavailable,
        EvmSubmissionFailure::TransportUnavailable,
    ]
);
reconciling_read_process!(
    ObserveCanonicalInclusionState,
    terminal,
    submission_process::inclusion_block_request,
    submission_process::settle_inclusion_block,
    [
        EvmSubmissionFailure::ProviderUnavailable,
        EvmSubmissionFailure::TransportUnavailable,
    ]
);
reconciling_effect_process!(
    CompleteWalletNonceState,
    completion,
    submission_process::completion_request,
    submission_process::settle_completion,
    [EvmSubmissionFailure::NonceAuthorityUnavailable]
);

/// Registers the complete state/capability side of the one structured EVM
/// submission expansion. Live crates must separately register its concrete
/// adapter, signer, and resource bindings.
pub fn register_evm_submission_process(
    registry: &mut ProgramRegistryBuilder,
    qualification: &EvmSubmissionProcessQualification,
) -> mfm_certify::Result<()> {
    let fixture = QualificationFixture::new()?;
    register_values(registry)?;
    register_closed_sums(registry)?;
    registry.register_capability_state::<StructuredSubmitEvmTransactionState>()?;

    register_state::<MapEvmSubmissionFailureState>(registry, qualification, &fixture)?;
    register_state::<MapPendingEvmSubmissionFailureState>(registry, qualification, &fixture)?;
    register_state::<HandlePendingEvmSubmissionFailureState>(registry, qualification, &fixture)?;
    register_state::<DeriveTransactionIntentState>(registry, qualification, &fixture)?;
    register_state::<DeriveSubmissionIntentIdState>(registry, qualification, &fixture)?;
    register_state::<DeriveEvmNonceReservationKeyState>(registry, qualification, &fixture)?;
    register_state::<ReadWalletNonceStatusState>(registry, qualification, &fixture)?;
    register_state::<ReadPostReserveWalletNonceStatusState>(registry, qualification, &fixture)?;
    register_state::<ReadReservationStatusAfterFailureState>(registry, qualification, &fixture)?;
    register_state::<ReadCandidateStatusAfterFailureState>(registry, qualification, &fixture)?;
    register_state::<ReadCandidateWalletNonceStatusState>(registry, qualification, &fixture)?;
    register_state::<ObservePendingNonceState>(registry, qualification, &fixture)?;
    register_state::<QualifyPendingNonceFloorState>(registry, qualification, &fixture)?;
    register_state::<ReserveWalletNonceState>(registry, qualification, &fixture)?;
    register_state::<CollapseWalletStatusState>(registry, qualification, &fixture)?;
    register_state::<SelectCandidateSlotState>(registry, qualification, &fixture)?;
    register_state::<MarkCandidateFamilyExhaustedState>(registry, qualification, &fixture)?;
    register_state::<BuildUnsignedCandidateState>(registry, qualification, &fixture)?;
    register_state::<AttestCandidateIdentityState>(registry, qualification, &fixture)?;
    register_state::<DeriveCandidateActivationPermitState>(registry, qualification, &fixture)?;
    register_state::<DeriveEvmCandidateOperationKeyState>(registry, qualification, &fixture)?;
    register_state::<ActivateWalletCandidateState>(registry, qualification, &fixture)?;
    register_state::<BroadcastExactCandidateState>(registry, qualification, &fixture)?;
    register_state::<SelectObservationRoundState>(registry, qualification, &fixture)?;
    register_state::<ObserveActivatedTransactionState>(registry, qualification, &fixture)?;
    register_state::<ObserveCandidateReceiptState>(registry, qualification, &fixture)?;
    register_state::<SelectTerminalEvidenceState>(registry, qualification, &fixture)?;
    register_state::<MarkActivationReconcileState>(registry, qualification, &fixture)?;
    register_state::<MarkObservationReconcileState>(registry, qualification, &fixture)?;
    register_state::<ObserveFinalizedHeadState>(registry, qualification, &fixture)?;
    register_state::<ObserveCanonicalInclusionState>(registry, qualification, &fixture)?;
    register_state::<VerifyCanonicalInclusionState>(registry, qualification, &fixture)?;
    register_state::<CompleteWalletNonceState>(registry, qualification, &fixture)?;
    register_state::<MarkCandidateCompletedState>(registry, qualification, &fixture)?;
    register_state::<MarkSubmissionCompletedState>(registry, qualification, &fixture)?;
    register_state::<MarkSubmissionResumedState>(registry, qualification, &fixture)?;
    register_state::<ProjectCompletedWalletDispositionState>(registry, qualification, &fixture)?;
    register_state::<SelectSubmissionTerminalState>(registry, qualification, &fixture)?;

    registry.register_child(
        initial_reservation_recipe()
            .map_err(|error| mfm_certify::CertifyError::Certification(error.to_string()))?,
    )?;
    registry.register_child(
        candidate_attempt_recipe()
            .map_err(|error| mfm_certify::CertifyError::Certification(error.to_string()))?,
    )?;
    registry.register_capability_expansion::<EvmSubmissionExpansion>()?;

    let implementation = Arc::new(
        EvmSubmissionCapabilityImplementation::new()
            .map_err(|error| mfm_certify::CertifyError::Certification(error.to_string()))?,
    );
    register_read_capability::<ReadWalletNonceStatusCapability>(
        registry,
        qualification,
        Arc::clone(&implementation),
    )?;
    register_read_capability::<EvmPendingNonceCapability>(
        registry,
        qualification,
        Arc::clone(&implementation),
    )?;
    register_read_capability::<AttestCandidateIdentityCapability>(
        registry,
        qualification,
        Arc::clone(&implementation),
    )?;
    register_read_capability::<EvmTransactionLookupCapability>(
        registry,
        qualification,
        Arc::clone(&implementation),
    )?;
    register_read_capability::<EvmReceiptLookupCapability>(
        registry,
        qualification,
        Arc::clone(&implementation),
    )?;
    register_read_capability::<EvmFinalizedHeadCapability>(
        registry,
        qualification,
        Arc::clone(&implementation),
    )?;
    register_read_capability::<EvmInclusionBlockCapability>(
        registry,
        qualification,
        Arc::clone(&implementation),
    )?;
    register_effect_capability::<ReserveWalletNonceCapability>(
        registry,
        qualification,
        Arc::clone(&implementation),
    )?;
    register_effect_capability::<ActivateWalletCandidateCapability>(
        registry,
        qualification,
        Arc::clone(&implementation),
    )?;
    register_effect_capability::<BroadcastExactCandidateCapability>(
        registry,
        qualification,
        Arc::clone(&implementation),
    )?;
    register_effect_capability::<CompleteWalletNonceCapability>(
        registry,
        qualification,
        implementation,
    )?;
    Ok(())
}

fn register_state<S>(
    registry: &mut ProgramRegistryBuilder,
    qualification: &EvmSubmissionProcessQualification,
    fixture: &QualificationFixture,
) -> mfm_certify::Result<()>
where
    S: SubmissionStateProcess,
{
    let contract = state_contract::<S>()
        .map_err(|error| mfm_certify::CertifyError::Certification(error.to_string()))?;
    let implementation_id = implementation_id(
        S::semantic_state_id()
            .map_err(|error| mfm_certify::CertifyError::Certification(error.to_string()))?,
    )?;
    registry.register_state::<S>(
        descriptor(
            StructuredComponentKind::State,
            contract.state_contract_ref,
            implementation_id,
            qualification,
        ),
        S::callbacks(fixture),
    )?;
    Ok(())
}

fn register_read_capability<C>(
    registry: &mut ProgramRegistryBuilder,
    qualification: &EvmSubmissionProcessQualification,
    implementation: Arc<EvmSubmissionCapabilityImplementation>,
) -> mfm_certify::Result<()>
where
    C: mfm_program::structured::RuntimeReadCapability,
    EvmSubmissionCapabilityImplementation: ReadCapabilityImplementation<C>,
{
    let contract = C::contract()
        .and_then(|contract| contract.content_ref().map_err(Into::into))
        .map_err(|error| mfm_certify::CertifyError::Certification(error.to_string()))?;
    registry.register_read_capability::<C, _>(
        descriptor(
            StructuredComponentKind::Capability,
            contract.clone(),
            implementation_id(contract.schema_id().as_str())?,
            qualification,
        ),
        implementation,
    )?;
    Ok(())
}

fn register_effect_capability<C>(
    registry: &mut ProgramRegistryBuilder,
    qualification: &EvmSubmissionProcessQualification,
    implementation: Arc<EvmSubmissionCapabilityImplementation>,
) -> mfm_certify::Result<()>
where
    C: mfm_program::structured::RuntimeEffectCapability,
    EvmSubmissionCapabilityImplementation: EffectCapabilityImplementation<C>,
{
    let contract = C::contract()
        .and_then(|contract| contract.content_ref().map_err(Into::into))
        .map_err(|error| mfm_certify::CertifyError::Certification(error.to_string()))?;
    registry.register_effect_capability::<C, _>(
        descriptor(
            StructuredComponentKind::Capability,
            contract.clone(),
            implementation_id(contract.schema_id().as_str())?,
            qualification,
        ),
        implementation,
    )?;
    Ok(())
}

fn descriptor(
    component_kind: StructuredComponentKind,
    semantic_contract_ref: ContentRef,
    implementation_id: StableId,
    qualification: &EvmSubmissionProcessQualification,
) -> SecretFreeImplementationDescriptor {
    SecretFreeImplementationDescriptor {
        component_kind,
        semantic_contract_ref,
        implementation_id,
        executable_identity_ref: qualification.executable_identity_ref.clone(),
        qualification_artifact_ref: qualification.qualification_artifact_ref.clone(),
    }
}

fn implementation_id(source: impl AsRef<str>) -> mfm_certify::Result<StableId> {
    let digest = mfm_journal::structured::domain_content_digest(
        "mfm.evm.structured-implementation-id.v1",
        &source.as_ref(),
    )
    .map_err(|error| mfm_certify::CertifyError::Certification(error.to_string()))?;
    StableId::new(format!("mfm.evm.implementation/{}", digest.digest()))
        .map_err(|error| mfm_certify::CertifyError::Certification(error.to_string()))
}

fn register_values(registry: &mut ProgramRegistryBuilder) -> mfm_certify::Result<()> {
    macro_rules! values {
        ($($value:ty),+ $(,)?) => {{
            $(registry.register_value::<$value>()?;)+
        }};
    }
    values!(
        EvmSubmissionRequest,
        EvmSubmissionOutput,
        EvmSubmissionFailure,
        SubmissionFailureRoute,
        PendingEvmSubmissionFailure,
        PendingEvmSubmissionFailureRoute,
        DerivedSubmissionDomain,
        IntentBoundSubmission,
        PreparedWalletSubmission,
        WalletStatusBaseline,
        WalletStatusDecision,
        FailureReconciliationRequest,
        PostReservePreparedSubmission,
        ObservedPendingSubmission,
        QualifiedPendingSubmission,
        SubmissionWork,
        SubmissionProgress,
        CandidateSlotDecision,
        SubmissionTerminalDecision,
        CandidateWork,
        PermittedCandidateWork,
        PreparedCandidateActivation,
        CandidateActivationDecision,
        ActiveCandidateWork,
        CandidateObservationWork,
        ObservationRoundDecision,
        TerminalEvidenceDecision,
        TerminalEvidenceWork,
        CompletionWork,
        CandidateResolution,
        CompletedProjection,
        ReadEvmWalletNonceStatusRequest,
        WalletNonceStatus,
        EvmPendingNonceRequest,
        ObservedPendingNonceFloor,
        ReserveEvmNonceRequest,
        ReserveWalletNonceResponse,
        AttestCandidateIdentityRequest,
        AttestedWalletCandidate,
        ActivateEvmCandidateRequest,
        ActivateCandidateResponse,
        BroadcastExactCandidateRequest,
        SubmittedCandidateProof,
        EvmTransactionLookupRequest,
        EvmTransactionLookupObservation,
        EvmReceiptLookupRequest,
        EvmReceiptLookupObservation,
        EvmFinalizedHeadRequest,
        EvmFinalizedHeadObservation,
        EvmInclusionBlockRequest,
        EvmInclusionBlockObservation,
        CompleteEvmNonceRequest,
        CompleteWalletNonceResponse,
        CompletedWalletNonce,
        WalletNonceStoreLineageHead,
        BroadcastLineageHead,
        UnsignedWalletCandidate,
    );
    Ok(())
}

fn register_closed_sums(registry: &mut ProgramRegistryBuilder) -> mfm_certify::Result<()> {
    registry.register_closed_sum::<SubmissionFailureRoute>()?;
    registry.register_closed_sum::<PendingEvmSubmissionFailureRoute>()?;
    registry.register_closed_sum::<PendingEvmSubmissionFailure>()?;
    registry.register_closed_sum::<WalletStatusDecision>()?;
    registry.register_closed_sum::<CandidateSlotDecision>()?;
    registry.register_closed_sum::<SubmissionTerminalDecision>()?;
    registry.register_closed_sum::<CandidateActivationDecision>()?;
    registry.register_closed_sum::<CandidateResolution>()?;
    registry.register_closed_sum::<ObservationRoundDecision>()?;
    registry.register_closed_sum::<TerminalEvidenceDecision>()?;
    registry.register_closed_sum::<CompletedProjection>()?;
    Ok(())
}

macro_rules! read_capability_implementation {
    (
        $capability:ty,
        request = $request:expr,
        returned = $returned:expr,
        safe_failures = [$($safe_failure:pat_param),+ $(,)?]
    ) => {
        impl ReadCapabilityImplementation<$capability>
            for EvmSubmissionCapabilityImplementation
        {
            fn validate_request(
                &self,
                request: &<$capability as mfm_capabilities::ReadCapabilityContract>::Request,
            ) -> Result<(), CapabilityContractFault> {
                self.require(($request)(request))
            }

            fn validate_returned(
                &self,
                returned: &<$capability as mfm_capabilities::ReadCapabilityContract>::Returned,
            ) -> Result<(), CapabilityContractFault> {
                self.require(($returned)(returned))
            }

            fn validate_safe_failure(
                &self,
                failure: &<$capability as mfm_capabilities::ReadCapabilityContract>::SafeFailure,
            ) -> Result<(), CapabilityContractFault> {
                self.require(matches!(failure, $($safe_failure)|+))
            }
        }
    };
}

macro_rules! effect_capability_implementation {
    (
        $capability:ty,
        request = $request:expr,
        returned = $returned:expr,
        evidence = $evidence:expr,
        safe_failures = [$($safe_failure:pat_param),+ $(,)?]
    ) => {
        impl EffectCapabilityImplementation<$capability>
            for EvmSubmissionCapabilityImplementation
        {
            fn validate_request(
                &self,
                request: &<$capability as mfm_capabilities::EffectCapabilityContract>::Request,
            ) -> Result<(), CapabilityContractFault> {
                self.require(($request)(request))
            }

            fn validate_returned(
                &self,
                returned: &<$capability as mfm_capabilities::EffectCapabilityContract>::Returned,
            ) -> Result<(), CapabilityContractFault> {
                self.require(($returned)(returned))
            }

            fn validate_safe_failure(
                &self,
                failure: &<$capability as mfm_capabilities::EffectCapabilityContract>::SafeFailure,
            ) -> Result<(), CapabilityContractFault> {
                self.require(matches!(failure, $($safe_failure)|+))
            }

            fn validate_superseded_before_entry(
                &self,
                evidence: &<<$capability as mfm_capabilities::EffectCapabilityContract>::Refresh as mfm_capabilities::EffectRefreshMode>::Evidence,
            ) -> Result<(), CapabilityContractFault> {
                self.require(($evidence)(evidence))
            }

            fn validate_entry_unknown(
                &self,
                fault: &AccessFaultCode,
            ) -> Result<(), CapabilityContractFault> {
                validate_access_fault(fault)
            }

            fn validate_integrity_fault(
                &self,
                fault: &AccessFaultCode,
            ) -> Result<(), CapabilityContractFault> {
                validate_access_fault(fault)
            }
        }
    };
}

read_capability_implementation!(
    ReadWalletNonceStatusCapability,
    request = valid_status_request,
    returned = valid_wallet_status,
    safe_failures = [EvmSubmissionFailure::NonceAuthorityUnavailable]
);
read_capability_implementation!(
    EvmPendingNonceCapability,
    request = valid_pending_nonce_request,
    returned = valid_pending_nonce_observation,
    safe_failures = [
        EvmSubmissionFailure::ProviderUnavailable,
        EvmSubmissionFailure::TransportUnavailable
    ]
);
read_capability_implementation!(
    AttestCandidateIdentityCapability,
    request = valid_attestation_request,
    returned = valid_attested_candidate,
    safe_failures = [EvmSubmissionFailure::SignerUnavailable]
);
read_capability_implementation!(
    EvmTransactionLookupCapability,
    request = valid_transaction_lookup_request,
    returned = valid_transaction_lookup_observation,
    safe_failures = [
        EvmSubmissionFailure::ProviderUnavailable,
        EvmSubmissionFailure::TransportUnavailable
    ]
);
read_capability_implementation!(
    EvmReceiptLookupCapability,
    request = valid_receipt_lookup_request,
    returned = valid_receipt_lookup_observation,
    safe_failures = [
        EvmSubmissionFailure::ProviderUnavailable,
        EvmSubmissionFailure::TransportUnavailable
    ]
);
read_capability_implementation!(
    EvmFinalizedHeadCapability,
    request = valid_finalized_head_request,
    returned = valid_finalized_head_observation,
    safe_failures = [
        EvmSubmissionFailure::ProviderUnavailable,
        EvmSubmissionFailure::TransportUnavailable
    ]
);
read_capability_implementation!(
    EvmInclusionBlockCapability,
    request = valid_inclusion_block_request,
    returned = valid_inclusion_block_observation,
    safe_failures = [
        EvmSubmissionFailure::ProviderUnavailable,
        EvmSubmissionFailure::TransportUnavailable
    ]
);

effect_capability_implementation!(
    ReserveWalletNonceCapability,
    request = valid_reserve_request,
    returned = valid_reserve_response,
    evidence = valid_wallet_lineage_head,
    safe_failures = [EvmSubmissionFailure::NonceAuthorityUnavailable]
);
effect_capability_implementation!(
    ActivateWalletCandidateCapability,
    request = valid_activation_request,
    returned = valid_activation_response,
    evidence = valid_wallet_lineage_head,
    safe_failures = [EvmSubmissionFailure::NonceAuthorityUnavailable]
);
effect_capability_implementation!(
    BroadcastExactCandidateCapability,
    request = valid_broadcast_request,
    returned = valid_submitted_candidate,
    evidence = valid_broadcast_lineage_head,
    safe_failures = [
        EvmSubmissionFailure::DestinationRejected,
        EvmSubmissionFailure::ProviderUnavailable,
        EvmSubmissionFailure::SignerUnavailable,
        EvmSubmissionFailure::TransportUnavailable
    ]
);
effect_capability_implementation!(
    CompleteWalletNonceCapability,
    request = valid_completion_request,
    returned = valid_completion_response,
    evidence = valid_wallet_lineage_head,
    safe_failures = [EvmSubmissionFailure::NonceAuthorityUnavailable]
);

fn valid_reference(reference: &crate::EvmWalletReference) -> bool {
    reference.to_content_ref().is_ok()
}

fn valid_digest(value: &str) -> bool {
    ContentDigest::from_str(value).is_ok()
}

fn valid_hash(value: &str) -> bool {
    B256::from_str(value).is_ok_and(|hash| hash != B256::ZERO && value == format!("{hash:#x}"))
}

fn valid_quantity(value: &str) -> bool {
    U256::from_str(value).is_ok_and(|quantity| value == quantity.to_string())
}

fn valid_status_request(request: &ReadEvmWalletNonceStatusRequest) -> bool {
    request.nonce_domain.validate().is_ok()
        && request.domain_activation_attestation.validate().is_ok()
        && request.semantic_reservation_key.validate().is_ok()
        && request.submission_intent_id.validate().is_ok()
        && valid_digest(&request.transaction_intent_digest)
        && valid_digest(&request.candidate_family_ref)
        && request
            .domain_activation_attestation
            .current_schema_record
            .wallet_nonce_domain
            == request.nonce_domain
}

fn valid_pending_nonce_request(request: &EvmPendingNonceRequest) -> bool {
    request.nonce_domain.validate().is_ok() && valid_reference(&request.route_generation_ref)
}

fn valid_pending_nonce_observation(observation: &ObservedPendingNonceFloor) -> bool {
    observation.nonce_domain.validate().is_ok()
        && valid_reference(&observation.route_generation_ref)
}

fn valid_transaction_lookup_request(request: &EvmTransactionLookupRequest) -> bool {
    valid_reference(&request.route_generation_ref) && valid_hash(&request.transaction_hash)
}

fn valid_transaction_lookup_observation(observation: &EvmTransactionLookupObservation) -> bool {
    match observation {
        EvmTransactionLookupObservation::Missing => true,
        EvmTransactionLookupObservation::Found {
            transaction_hash,
            block_number,
            block_hash,
        } => {
            valid_hash(transaction_hash)
                && match (block_number, block_hash) {
                    (None, None) => true,
                    (Some(block_number), Some(block_hash)) => {
                        valid_quantity(block_number) && valid_hash(block_hash)
                    }
                    _ => false,
                }
        }
    }
}

fn valid_receipt_lookup_request(request: &EvmReceiptLookupRequest) -> bool {
    valid_reference(&request.route_generation_ref) && valid_hash(&request.transaction_hash)
}

fn valid_receipt_lookup_observation(observation: &EvmReceiptLookupObservation) -> bool {
    match observation {
        EvmReceiptLookupObservation::Missing => true,
        EvmReceiptLookupObservation::Found {
            transaction_hash,
            block_number,
            block_hash,
            status,
        } => {
            valid_hash(transaction_hash)
                && valid_quantity(block_number)
                && valid_hash(block_hash)
                && matches!(status, 0 | 1)
        }
    }
}

fn valid_finalized_head_request(request: &EvmFinalizedHeadRequest) -> bool {
    valid_reference(&request.route_generation_ref)
}

fn valid_finalized_head_observation(observation: &EvmFinalizedHeadObservation) -> bool {
    valid_quantity(&observation.block_number) && valid_hash(&observation.block_hash)
}

fn valid_inclusion_block_request(request: &EvmInclusionBlockRequest) -> bool {
    valid_reference(&request.route_generation_ref) && valid_quantity(&request.block_number)
}

fn valid_inclusion_block_observation(observation: &EvmInclusionBlockObservation) -> bool {
    valid_quantity(&observation.block_number) && valid_hash(&observation.block_hash)
}

fn valid_unsigned_candidate(candidate: &UnsignedWalletCandidate) -> bool {
    if candidate.transaction_intent.validate().is_err()
        || candidate.semantic_reservation_key.validate().is_err()
        || usize::from(candidate.candidate_ordinal) >= crate::EVM_WALLET_REPLACEMENT_LIMIT
    {
        return false;
    }
    candidate
        .transaction_intent
        .unsigned_candidate(candidate.nonce, &candidate.fee)
        .is_ok_and(|envelope| {
            candidate.unsigned_candidate_digest == format!("{:#x}", envelope.signing_digest())
        })
}

fn valid_attestation_request(request: &AttestCandidateIdentityRequest) -> bool {
    valid_unsigned_candidate(&request.candidate)
}

fn valid_attested_candidate(candidate: &AttestedWalletCandidate) -> bool {
    candidate.semantic_reservation_key.validate().is_ok()
        && usize::from(candidate.candidate_ordinal) < crate::EVM_WALLET_REPLACEMENT_LIMIT
        && valid_reference(&candidate.candidate_descriptor_ref)
        && valid_hash(&candidate.unsigned_candidate_digest)
        && valid_hash(&candidate.transaction_hash)
        && StableId::new(&candidate.semantic_signer_id).is_ok()
        && valid_reference(&candidate.signing_profile_contract_ref)
        && valid_reference(&candidate.signer_attestation_ref)
}

fn valid_active_candidate(candidate: &ActiveWalletCandidate) -> bool {
    valid_attested_candidate(&candidate.attested_candidate)
        && valid_reference(&candidate.activation_evidence_ref)
}

fn valid_reservation(reservation: &crate::ReservedWalletNonce) -> bool {
    reservation.nonce_domain.validate().is_ok()
        && valid_reference(&reservation.domain_activation_record_ref)
        && reservation.semantic_reservation_key.validate().is_ok()
        && reservation.submission_intent_id.validate().is_ok()
        && valid_digest(&reservation.transaction_intent_digest)
        && valid_digest(&reservation.candidate_family_ref)
        && valid_digest(&reservation.observed_floor_ref)
        && valid_reference(&reservation.resource_lineage_ref)
        && valid_reference(&reservation.reservation_evidence_ref)
}

fn valid_active_prefix(
    reservation: &crate::ReservedWalletNonce,
    transaction_intent: &crate::EvmTransactionIntent,
    candidate_family: &crate::EvmCandidateFamily,
    activated_candidates: &[ActiveWalletCandidate],
) -> bool {
    crate::validate_active_wallet_candidate_prefix(
        reservation,
        transaction_intent,
        candidate_family,
        activated_candidates,
    )
    .is_ok()
}

fn valid_terminal_outcome(outcome: &CanonicalTerminalOutcome) -> bool {
    outcome.validate().is_ok()
}

fn valid_completed(completion: &CompletedWalletNonce) -> bool {
    completion.nonce_domain.validate().is_ok()
        && completion.semantic_reservation_key.validate().is_ok()
        && completion.semantic_completion_key.validate().is_ok()
        && valid_terminal_outcome(&completion.canonical_terminal_outcome)
        && completion.nonce_domain == completion.canonical_terminal_outcome.nonce_domain
        && completion.nonce == completion.canonical_terminal_outcome.nonce
        && completion.semantic_reservation_key
            == completion
                .canonical_terminal_outcome
                .semantic_reservation_key
        && derive_evm_nonce_completion_key(&completion.semantic_reservation_key)
            .is_ok_and(|key| key == completion.semantic_completion_key)
        && valid_digest(&completion.original_terminal_witnesses_ref)
        && valid_reference(&completion.completion_evidence_ref)
}

fn valid_wallet_status(status: &WalletNonceStatus) -> bool {
    match status {
        WalletNonceStatus::Absent | WalletNonceStatus::Busy => true,
        WalletNonceStatus::Reserved {
            reservation,
            transaction_intent,
            candidate_family,
            activated_candidates,
            current_candidate,
            resource_head_ref,
        } => {
            valid_reservation(reservation)
                && transaction_intent.validate().is_ok()
                && candidate_family.validate(transaction_intent).is_ok()
                && transaction_intent.nonce_domain() == &reservation.nonce_domain
                && transaction_intent.digest() == reservation.transaction_intent_digest
                && candidate_family.digest() == reservation.candidate_family_ref
                && valid_active_prefix(
                    reservation,
                    transaction_intent,
                    candidate_family,
                    activated_candidates,
                )
                && current_candidate.as_ref() == activated_candidates.last()
                && valid_reference(resource_head_ref)
        }
        WalletNonceStatus::Completed {
            reservation,
            transaction_intent,
            candidate_family,
            activated_candidates,
            completion,
            resource_head_ref,
        } => {
            valid_reservation(reservation)
                && transaction_intent.validate().is_ok()
                && candidate_family.validate(transaction_intent).is_ok()
                && transaction_intent.nonce_domain() == &reservation.nonce_domain
                && transaction_intent.digest() == reservation.transaction_intent_digest
                && candidate_family.digest() == reservation.candidate_family_ref
                && valid_active_prefix(
                    reservation,
                    transaction_intent,
                    candidate_family,
                    activated_candidates,
                )
                && valid_completed(completion)
                && completion.semantic_reservation_key == reservation.semantic_reservation_key
                && activated_candidates.iter().any(|candidate| {
                    candidate.attested_candidate.candidate_ordinal
                        == completion
                            .canonical_terminal_outcome
                            .winning_candidate_ordinal
                        && candidate.attested_candidate.transaction_hash
                            == completion.canonical_terminal_outcome.transaction_hash
                        && candidate.activation_evidence_ref
                            == completion
                                .canonical_terminal_outcome
                                .winning_activation_evidence_ref
                })
                && valid_reference(resource_head_ref)
        }
    }
}

fn valid_reserve_request(request: &ReserveEvmNonceRequest) -> bool {
    request.nonce_domain.validate().is_ok()
        && request.domain_activation_attestation.validate().is_ok()
        && request.submission_intent_id.validate().is_ok()
        && request.transaction_intent.validate().is_ok()
        && request
            .candidate_family
            .validate(&request.transaction_intent)
            .is_ok()
        && request.reservation_key.validate().is_ok()
        && request.qualified_floor.observed.nonce_domain == request.nonce_domain
        && valid_reference(&request.qualified_floor.observed.route_generation_ref)
        && valid_reference(&request.qualified_floor.pending_floor_policy_ref)
        && request.transaction_intent.nonce_domain() == &request.nonce_domain
        && request
            .domain_activation_attestation
            .current_schema_record
            .wallet_nonce_domain
            == request.nonce_domain
        && derive_evm_nonce_reservation_key(&request.nonce_domain, &request.submission_intent_id)
            .is_ok_and(|key| key == request.reservation_key)
}

fn valid_reserve_response(response: &ReserveWalletNonceResponse) -> bool {
    match response {
        ReserveWalletNonceResponse::Reserved { reservation } => valid_reservation(reservation),
        ReserveWalletNonceResponse::NonceDomainBusy
        | ReserveWalletNonceResponse::NonceLineageDiverged
        | ReserveWalletNonceResponse::NonceCapacityExhausted => true,
    }
}

fn valid_activation_permit(permit: &CandidateActivationPermit, ordinal: u16) -> bool {
    match permit {
        CandidateActivationPermit::Initial { exact_next_ordinal } => {
            ordinal == 0 && *exact_next_ordinal == 0
        }
        CandidateActivationPermit::Replacement {
            predecessor_activation_ref,
            predecessor_ordinal,
            exact_next_ordinal,
            replacement_policy_ref,
            eligibility_ref,
        } => {
            *exact_next_ordinal == ordinal
                && predecessor_ordinal
                    .checked_add(1)
                    .is_some_and(|next| next == ordinal)
                && valid_reference(predecessor_activation_ref)
                && valid_reference(replacement_policy_ref)
                && valid_digest(eligibility_ref)
        }
    }
}

fn valid_activation_request(request: &ActivateEvmCandidateRequest) -> bool {
    request.nonce_domain.validate().is_ok()
        && request.candidate_operation_key.validate().is_ok()
        && valid_attested_candidate(&request.next_candidate)
        && valid_activation_permit(
            &request.activation_permit,
            request.next_candidate.candidate_ordinal,
        )
        && derive_evm_candidate_operation_key(
            &request.next_candidate.semantic_reservation_key,
            request.next_candidate.candidate_ordinal,
        )
        .is_ok_and(|key| key == request.candidate_operation_key)
}

fn valid_activation_response(response: &ActivateCandidateResponse) -> bool {
    match response {
        ActivateCandidateResponse::Activated { candidate } => valid_active_candidate(candidate),
        ActivateCandidateResponse::CandidateProgressionConflict => true,
    }
}

fn valid_broadcast_request(request: &BroadcastExactCandidateRequest) -> bool {
    valid_unsigned_candidate(&request.unsigned_candidate)
        && valid_active_candidate(&request.active_candidate)
        && valid_reference(&request.route_generation_ref)
        && request.unsigned_candidate.semantic_reservation_key
            == request
                .active_candidate
                .attested_candidate
                .semantic_reservation_key
        && request.unsigned_candidate.candidate_ordinal
            == request
                .active_candidate
                .attested_candidate
                .candidate_ordinal
        && request.unsigned_candidate.unsigned_candidate_digest
            == request
                .active_candidate
                .attested_candidate
                .unsigned_candidate_digest
        && request
            .unsigned_candidate
            .transaction_intent
            .semantic_signer_id()
            == request
                .active_candidate
                .attested_candidate
                .semantic_signer_id
}

fn valid_submitted_candidate(proof: &SubmittedCandidateProof) -> bool {
    usize::from(proof.candidate_ordinal) < crate::EVM_WALLET_REPLACEMENT_LIMIT
        && valid_hash(&proof.unsigned_candidate_digest)
        && valid_hash(&proof.transaction_hash)
        && StableId::new(&proof.semantic_signer_id).is_ok()
        && valid_reference(&proof.signer_generation_ref)
        && valid_reference(&proof.signing_contract_ref)
        && valid_reference(&proof.submission_contract_ref)
}

fn valid_completion_request(request: &CompleteEvmNonceRequest) -> bool {
    request.validate().is_ok()
}

fn valid_completion_response(response: &CompleteWalletNonceResponse) -> bool {
    match response {
        CompleteWalletNonceResponse::Completed { completion } => valid_completed(completion),
    }
}

fn valid_wallet_lineage_head(head: &WalletNonceStoreLineageHead) -> bool {
    StableId::new(&head.wallet_nonce_store_lineage_id).is_ok()
        && head.writer_epoch > 0
        && valid_reference(&head.public_lineage_head_ref)
}

fn valid_broadcast_lineage_head(head: &BroadcastLineageHead) -> bool {
    StableId::new(&head.lineage_id).is_ok()
        && head.generation > 0
        && valid_reference(&head.public_lineage_head_ref)
}

#[allow(dead_code)] // retained for process-registration fixture construction
pub(crate) struct QualificationFixture {
    pub(crate) prepared: PreparedWalletSubmission,
    pub(crate) post_reserve: PostReservePreparedSubmission,
    pub(crate) reservation_reconciliation: FailureReconciliationRequest,
    pub(crate) candidate_reconciliation: FailureReconciliationRequest,
    pub(crate) qualified_pending: QualifiedPendingSubmission,
    pub(crate) candidate: CandidateWork,
    pub(crate) prepared_activation: PreparedCandidateActivation,
    pub(crate) active: ActiveCandidateWork,
    pub(crate) observation: CandidateObservationWork,
    pub(crate) terminal: TerminalEvidenceWork,
    pub(crate) completion: CompletionWork,
}

impl QualificationFixture {
    pub(crate) fn new() -> mfm_certify::Result<Self> {
        qualification_fixture()
    }
}

fn qualification_fixture() -> mfm_certify::Result<QualificationFixture> {
    let common_ref = evm_wallet_nonce_policy_ref().map_err(fixture_contract_error)?;
    let declaration = ChainInstanceDeclaration::new(
        common_ref.clone(),
        fixture_stable("mfm.evm.fixture/chain-instance")?,
        1,
        B256::repeat_byte(0x11),
        U256::from(10_u64),
        B256::repeat_byte(0x12),
    )
    .map_err(fixture_contract_error)?;
    let chain_attestation =
        ChainInstanceRegistryAttestation::new(declaration, common_ref.clone(), common_ref.clone())
            .map_err(fixture_contract_error)?;
    let chain_binding = chain_attestation
        .binding()
        .map_err(fixture_contract_error)?;
    let chain_lineage = derive_evm_chain_lineage_id(chain_binding.qualified_chain_instance_id())
        .map_err(fixture_contract_error)?;
    let sender = Address::repeat_byte(0x21);
    let nonce_domain =
        derive_wallet_nonce_domain(chain_lineage, sender).map_err(fixture_contract_error)?;
    let assurance_ref = evm_wallet_assurance_policy_ref().map_err(fixture_contract_error)?;
    let route_generation_ref = crate::EvmRoutingGenerationRef::from_content_ref(
        common_ref
            .to_content_ref()
            .map_err(fixture_contract_error)?,
    )
    .map_err(fixture_contract_error)?;
    let sender_inventory = mfm_journal::structured::domain_content_digest(
        "mfm.evm.fixture-sender-path-inventory.v1",
        &(nonce_domain.clone(), sender),
    )
    .map_err(|error| mfm_certify::CertifyError::Certification(error.to_string()))?;
    let activation_record = WalletNonceDomainActivationRecord {
        activation_contract_ref: common_ref.clone(),
        qualified_activation_registry_lineage_ref: common_ref.clone(),
        wallet_nonce_store_lineage_id: "mfm.evm.fixture/wallet-store".to_owned(),
        initial_store_incarnation_ref: common_ref.clone(),
        wallet_nonce_domain: nonce_domain.clone(),
        chain_instance_attestation: chain_attestation,
        initial_route_generation_ref: route_generation_ref,
        initial_route_membership_issuance_ref: common_ref.clone(),
        sender_identity: format!("{sender:#x}"),
        issuer_namespace_contract_ref: common_ref.clone(),
        replay_exclusion_contract_ref: common_ref.clone(),
        replay_exclusion_disposition:
            ReplayExclusionDisposition::EveryPriorRequestReplayAndRetryIngressExcluded,
        finalized_sender_nonce_floor: 0,
        finalized_block_number: "10".to_owned(),
        finalized_block_hash: format!("{:#x}", B256::repeat_byte(0x12)),
        qualified_observation_proof_ref: common_ref.clone(),
        exhaustive_sender_path_inventory_digest: sender_inventory.as_str().to_owned(),
        exclusive_current_control: ExclusiveCurrentControl::EveryPriorWriterSignerRelayerOperatorStaleDeploymentAndDirectSubmitPathFenced,
        prior_effect_disposition: PriorEffectDisposition::NoUnresolvedPossibleEntry,
        prior_resource_disposition:
            PriorResourceDisposition::EveryPriorAllocationAndSubmittedCandidateTerminal,
        new_idempotency_epoch: "mfm.evm.fixture/idempotency-epoch".to_owned(),
    };
    activation_record
        .validate()
        .map_err(fixture_contract_error)?;
    let activation_record_ref =
        canonical_wallet_reference(&activation_record).map_err(fixture_contract_error)?;
    let activation = WalletNonceDomainActivationAttestation {
        activation_record_ref: activation_record_ref.clone(),
        registry_issuance_ref: common_ref.clone(),
        activation_registry_lineage_ref: common_ref.clone(),
        initial_store_incarnation_ref: activation_record.initial_store_incarnation_ref.clone(),
        current_schema_record: activation_record,
    };
    activation.validate().map_err(fixture_contract_error)?;

    let template = EvmWalletTransactionTemplate::new(
        EvmTransactionTarget::new("qualification-target").map_err(fixture_contract_error)?,
        EvmWalletTransactionAction::call(Address::repeat_byte(0x31)),
        U256::ZERO,
        [],
        Vec::new(),
        U256::from(21_000_u64),
    )
    .map_err(fixture_contract_error)?;
    let intent = EvmTransactionIntent::new(
        chain_binding,
        nonce_domain.clone(),
        template,
        fixture_stable("mfm.evm.fixture/semantic-signer")?,
        common_ref.clone(),
        common_ref.clone(),
        assurance_ref,
    )
    .map_err(fixture_contract_error)?;
    let candidate_family = EvmCandidateFamily::new(
        &intent,
        vec![
            EvmWalletFeeCandidate::new(U256::from(100_u64), U256::from(2_u64))
                .map_err(fixture_contract_error)?,
        ],
    )
    .map_err(fixture_contract_error)?;
    let configuration = EvmSubmissionConfiguration::new(
        activation,
        common_ref.clone(),
        common_ref.clone(),
        intent,
        candidate_family,
        2,
    )
    .map_err(fixture_contract_error)?;
    let request = EvmSubmissionRequest::from_authorized(
        configuration,
        TenantScopeId::new("mfm.tenant_scope.v1:00000000000000000000000000000051")
            .map_err(fixture_contract_error)?,
        fixture_stable("mfm.evm.fixture/principal")?,
        EvmCallerSubmissionToken::new("qualification-token").map_err(fixture_contract_error)?,
    )
    .map_err(fixture_contract_error)?;

    let derived = fixture_success(submission_process::derive_transaction_intent(&request))?;
    let intent = fixture_success(submission_process::derive_submission_intent(&derived))?;
    let prepared = fixture_success(submission_process::derive_reservation_key(&intent))?;
    let observed = ObservedPendingNonceFloor {
        nonce_domain: nonce_domain.clone(),
        route_generation_ref: common_ref.clone(),
        pending_nonce: 0,
    };
    let observed_submission = ObservedPendingSubmission {
        prepared: prepared.clone(),
        observed: observed.clone(),
    };
    let qualified_pending = fixture_success(submission_process::qualify_pending_nonce(
        &observed_submission,
    ))?;
    let observed_floor_ref = canonical_wallet_reference(&observed)
        .map_err(fixture_contract_error)?
        .content_digest()
        .to_owned();
    let reservation = ReservedWalletNonce {
        nonce_domain: nonce_domain.clone(),
        domain_activation_record_ref: activation_record_ref,
        nonce: 0,
        semantic_reservation_key: prepared.reservation_key.clone(),
        submission_intent_id: prepared.intent.submission_intent_id.clone(),
        transaction_intent_digest: request.transaction_intent().digest().to_owned(),
        candidate_family_ref: request.candidate_family().digest().to_owned(),
        observed_floor_ref,
        resource_lineage_ref: common_ref.clone(),
        reservation_evidence_ref: common_ref.clone(),
    };
    let post_reserve = PostReservePreparedSubmission {
        prepared: prepared.clone(),
        reservation: reservation.clone(),
    };
    let reserved_status = WalletNonceStatus::Reserved {
        reservation: reservation.clone(),
        transaction_intent: request.transaction_intent().clone(),
        candidate_family: request.candidate_family().clone(),
        activated_candidates: Vec::new(),
        current_candidate: None,
        resource_head_ref: common_ref.clone(),
    };
    let status_baseline = WalletStatusBaseline::Reserved {
        resource_head_ref: common_ref.clone(),
        canonical_status_digest: canonical_wallet_reference(&reserved_status)
            .map_err(fixture_contract_error)?
            .content_digest()
            .to_owned(),
    };
    let work = SubmissionWork {
        prepared: prepared.clone(),
        reservation,
        activated_candidates: Vec::new(),
        current_candidate: None,
        next_candidate_ordinal: 0,
        status_baseline: status_baseline.clone(),
    };
    let mut candidate = fixture_success(submission_process::build_unsigned_candidate(&work))?;
    let attested = AttestedWalletCandidate {
        semantic_reservation_key: prepared.reservation_key.clone(),
        candidate_ordinal: 0,
        candidate_descriptor_ref: canonical_wallet_reference(&candidate.unsigned_candidate)
            .map_err(fixture_contract_error)?,
        unsigned_candidate_digest: candidate
            .unsigned_candidate
            .unsigned_candidate_digest
            .clone(),
        transaction_hash: format!("{:#x}", B256::repeat_byte(0x41)),
        semantic_signer_id: request.transaction_intent().semantic_signer_id().to_owned(),
        signing_profile_contract_ref: request
            .transaction_intent()
            .signing_profile_contract_ref()
            .clone(),
        signer_attestation_ref: common_ref.clone(),
    };
    candidate.attested_candidate = Some(attested.clone());
    let permitted = fixture_success(submission_process::derive_candidate_activation_permit(
        &candidate,
    ))?;
    let prepared_activation = fixture_success(submission_process::derive_candidate_operation_key(
        &permitted,
    ))?;
    let active_candidate = ActiveWalletCandidate {
        attested_candidate: attested.clone(),
        activation_evidence_ref: common_ref.clone(),
    };
    let active = ActiveCandidateWork {
        candidate: candidate.clone(),
        active_candidate,
    };
    let submitted = SubmittedCandidateProof {
        candidate_ordinal: 0,
        unsigned_candidate_digest: attested.unsigned_candidate_digest.clone(),
        transaction_hash: attested.transaction_hash.clone(),
        semantic_signer_id: attested.semantic_signer_id.clone(),
        signer_generation_ref: common_ref.clone(),
        signing_contract_ref: common_ref.clone(),
        submission_contract_ref: request
            .transaction_intent()
            .submission_contract_ref()
            .clone(),
    };
    let inclusion_hash = format!("{:#x}", B256::repeat_byte(0x51));
    let observation = CandidateObservationWork {
        active: active.clone(),
        submitted,
        next_round: 2,
        observation: CandidateTransactionObservation {
            transaction: Some(EvmTransactionLookupObservation::Found {
                transaction_hash: attested.transaction_hash.clone(),
                block_number: Some("10".to_owned()),
                block_hash: Some(inclusion_hash.clone()),
            }),
            receipt: Some(EvmReceiptLookupObservation::Found {
                transaction_hash: attested.transaction_hash,
                block_number: "10".to_owned(),
                block_hash: inclusion_hash.clone(),
                status: 1,
            }),
            finalized_head: Some(EvmFinalizedHeadObservation {
                block_number: "11".to_owned(),
                block_hash: format!("{:#x}", B256::repeat_byte(0x52)),
            }),
            inclusion_block: Some(EvmInclusionBlockObservation {
                block_number: "10".to_owned(),
                block_hash: inclusion_hash,
            }),
        },
    };
    let terminal = TerminalEvidenceWork {
        candidate: observation.clone(),
    };
    let completion = fixture_success(submission_process::verify_canonical_inclusion(&terminal))?;
    let reservation_reconciliation = FailureReconciliationRequest {
        prepared: prepared.clone(),
        baseline: WalletStatusBaseline::Absent,
        original_failure: EvmSubmissionFailure::ProviderUnavailable,
    };
    let candidate_reconciliation = FailureReconciliationRequest {
        prepared: prepared.clone(),
        baseline: status_baseline,
        original_failure: EvmSubmissionFailure::ProviderUnavailable,
    };
    Ok(QualificationFixture {
        prepared,
        post_reserve,
        reservation_reconciliation,
        candidate_reconciliation,
        qualified_pending,
        candidate,
        prepared_activation,
        active,
        observation,
        terminal,
        completion,
    })
}

fn fixture_stable(value: &str) -> mfm_certify::Result<StableId> {
    StableId::new(value)
        .map_err(|error| mfm_certify::CertifyError::Certification(error.to_string()))
}

fn fixture_contract_error(error: impl std::fmt::Display) -> mfm_certify::CertifyError {
    mfm_certify::CertifyError::Certification(error.to_string())
}

fn fixture_success<T: Clone, E>(outcome: ProposedStateOutcome<T, E>) -> mfm_certify::Result<T> {
    match outcome.value() {
        ProposedStateValue::Success(value) => Ok(value.clone()),
        ProposedStateValue::Failure(_) => Err(mfm_certify::CertifyError::Certification(
            "structured EVM qualification fixture did not reach success".to_owned(),
        )),
    }
}

fn validate_access_fault(_fault: &AccessFaultCode) -> Result<(), CapabilityContractFault> {
    Ok(())
}
