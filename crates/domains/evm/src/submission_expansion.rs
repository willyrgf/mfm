//! Deterministic static lowering of one abstract EVM submission call.

use std::sync::OnceLock;

use mfm_ids::{ContentRef, StableId};
use mfm_program::structured::{
    AuthoringPolicy, BlockBuilder, ChildOperation, ClosedSum, CustomFailureHandler,
    DefaultFailureMapper, Never, OperationBuilder, Pure, RecoveryRouteBuilder,
    SafeFailureNotApplicable, State, Value,
};
use mfm_program_derive::MfmValue;
use serde::{Deserialize, Serialize};

use crate::submission::{
    ActivateWalletCandidateState, ActiveCandidateWork, AttestCandidateIdentityState,
    BroadcastExactCandidateState, BuildUnsignedCandidateState, CandidateObservationWork,
    CandidateResolution, CollapseCandidateResolutionState, CollapseWalletStatusState,
    CompleteWalletNonceState, DeriveCandidateActivationPermitState,
    DeriveEvmCandidateOperationKeyState, DeriveEvmNonceReservationKeyState,
    DeriveSubmissionIntentIdState, DeriveTransactionIntentState, EvmSubmissionRequest,
    ExtractSubmissionWorkState, FailureReconciliationRequest, MarkActivationReconcileState,
    MarkCandidateCompletedState, ObserveActivatedTransactionState, ObserveCandidateReceiptState,
    ObserveCanonicalInclusionState, ObserveFinalizedHeadState, ObservePendingNonceState,
    PendingEvmSubmissionFailure, PrepareExhaustionReconciliationState,
    PrepareRetainedCandidateObservationState, PreparedWalletSubmission,
    ProjectCompletedWalletDispositionState, QualifyPendingNonceFloorState,
    ReadCandidateStatusAfterFailureState, ReadCandidateWalletNonceStatusState,
    ReadExhaustionStatusState, ReadObservedCandidateStatusState,
    ReadPostReserveWalletNonceStatusState, ReadReservationStatusAfterFailureState,
    ReadWalletNonceStatusState, ReserveWalletNonceState, SelectCandidateAttemptRouteState,
    SelectCandidateSlotState, SelectObservationRoundState, SelectSubmissionTerminalState,
    SelectTerminalEvidenceState, SubmissionProgress, SubmissionWork, VerifyCanonicalInclusionState,
    WalletStatusDecision,
};
use crate::{EvmSubmissionFailure, EvmSubmissionOutput, EVM_WALLET_REPLACEMENT_LIMIT};

/// Closed identity mapping route for the submission failure boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "submission-failure-route",
    version = "1",
    schema = "mfm.evm.submission_failure_route"
)]
pub(crate) enum SubmissionFailureRoute {
    Propagate { failure: EvmSubmissionFailure },
}

impl ClosedSum for SubmissionFailureRoute {}

pub(crate) struct MapEvmSubmissionFailureState;

impl State for MapEvmSubmissionFailureState {
    type Input = EvmSubmissionFailure;
    type Output = SubmissionFailureRoute;
    type Failure = Never;
    type Request = ();
    type Returned = ();
    type SafeFailure = ();
    type Execution = Pure;
    type SafeFailureDisposition = SafeFailureNotApplicable;
    type Capability = mfm_program::structured::Direct;

    fn semantic_state_id() -> mfm_program::Result<StableId> {
        stable("mfm.evm.state/map-submission-failure")
    }
}

pub(crate) struct EvmSubmissionFailureMapper;

impl DefaultFailureMapper<EvmSubmissionFailure, EvmSubmissionFailure>
    for EvmSubmissionFailureMapper
{
    type Route = SubmissionFailureRoute;
    type Mapper = MapEvmSubmissionFailureState;
}

/// One-tag identity route used inside both reconciliation-aware children.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "pending-submission-failure-route",
    version = "1",
    schema = "mfm.evm.pending_submission_failure_route"
)]
pub(crate) enum PendingEvmSubmissionFailureRoute {
    Propagate {
        failure: PendingEvmSubmissionFailure,
    },
}

impl ClosedSum for PendingEvmSubmissionFailureRoute {}

pub(crate) struct MapPendingEvmSubmissionFailureState;

impl State for MapPendingEvmSubmissionFailureState {
    type Input = PendingEvmSubmissionFailure;
    type Output = PendingEvmSubmissionFailureRoute;
    type Failure = Never;
    type Request = ();
    type Returned = ();
    type SafeFailure = ();
    type Execution = Pure;
    type SafeFailureDisposition = SafeFailureNotApplicable;
    type Capability = mfm_program::structured::Direct;

    fn semantic_state_id() -> mfm_program::Result<StableId> {
        stable("mfm.evm.state/map-pending-submission-failure")
    }
}

pub(crate) struct PendingEvmSubmissionFailureMapper;

impl DefaultFailureMapper<PendingEvmSubmissionFailure, PendingEvmSubmissionFailure>
    for PendingEvmSubmissionFailureMapper
{
    type Route = PendingEvmSubmissionFailureRoute;
    type Mapper = MapPendingEvmSubmissionFailureState;
}

pub(crate) struct HandlePendingEvmSubmissionFailureState;

impl State for HandlePendingEvmSubmissionFailureState {
    type Input = PendingEvmSubmissionFailure;
    type Output = PendingEvmSubmissionFailure;
    type Failure = Never;
    type Request = ();
    type Returned = ();
    type SafeFailure = ();
    type Execution = Pure;
    type SafeFailureDisposition = SafeFailureNotApplicable;
    type Capability = mfm_program::structured::Direct;

    fn semantic_state_id() -> mfm_program::Result<StableId> {
        stable("mfm.evm.state/handle-pending-submission-failure")
    }
}

pub(crate) struct InitialReservationChild;

impl ChildOperation for InitialReservationChild {
    type Output = WalletStatusDecision;
    type Failure = PendingEvmSubmissionFailure;

    fn authored_program_ref() -> mfm_program::Result<mfm_ids::ContentRef> {
        static PROGRAM_REF: OnceLock<ContentRef> = OnceLock::new();
        if let Some(reference) = PROGRAM_REF.get() {
            return Ok(reference.clone());
        }
        let reference = initial_reservation_recipe()?
            .content_ref()
            .map_err(mfm_program::ProgramError::from)?;
        let _ = PROGRAM_REF.set(reference.clone());
        Ok(reference)
    }
}

pub(crate) struct CandidateAttemptChild;

impl ChildOperation for CandidateAttemptChild {
    type Output = SubmissionProgress;
    type Failure = PendingEvmSubmissionFailure;

    fn authored_program_ref() -> mfm_program::Result<mfm_ids::ContentRef> {
        static PROGRAM_REF: OnceLock<ContentRef> = OnceLock::new();
        if let Some(reference) = PROGRAM_REF.get() {
            return Ok(reference.clone());
        }
        let reference = candidate_attempt_recipe()?
            .content_ref()
            .map_err(mfm_program::ProgramError::from)?;
        let _ = PROGRAM_REF.set(reference.clone());
        Ok(reference)
    }
}

pub(crate) struct ReservationFailureHandler;

impl CustomFailureHandler<PendingEvmSubmissionFailure, WalletStatusDecision, EvmSubmissionFailure>
    for ReservationFailureHandler
{
    type Route = PendingEvmSubmissionFailure;
    type Handler = HandlePendingEvmSubmissionFailureState;

    fn author_routes<Policy>(
        routes: &mut RecoveryRouteBuilder<WalletStatusDecision, EvmSubmissionFailure, Policy>,
    ) -> mfm_program::Result<()>
    where
        Policy: AuthoringPolicy,
    {
        routes.arm(
            "direct",
            stable("reservation-failure-direct")?,
            |block, payloads| {
                let failure = payloads.value::<EvmSubmissionFailure>(&[stable("failure")?])?;
                block.scope_failure::<WalletStatusDecision>(&failure)
            },
        )?;
        routes.arm(
            "reconcile",
            stable("reservation-failure-reconcile")?,
            |block, payloads| {
                let request =
                    payloads.value::<FailureReconciliationRequest>(&[stable("request")?])?;
                let status = block
                    .state::<ReadReservationStatusAfterFailureState>(
                        stable("read-reservation-status-after-failure")?,
                        &request,
                    )?
                    .or_default()
                    .map_err(|error| {
                        mfm_program::ProgramError::Authoring(format!("activate child: {error}"))
                    })?;
                block.normal(&status)
            },
        )
    }
}

pub(crate) struct CandidateFailureHandler;

impl CustomFailureHandler<PendingEvmSubmissionFailure, SubmissionProgress, EvmSubmissionFailure>
    for CandidateFailureHandler
{
    type Route = PendingEvmSubmissionFailure;
    type Handler = HandlePendingEvmSubmissionFailureState;

    fn author_routes<Policy>(
        routes: &mut RecoveryRouteBuilder<SubmissionProgress, EvmSubmissionFailure, Policy>,
    ) -> mfm_program::Result<()>
    where
        Policy: AuthoringPolicy,
    {
        routes.arm(
            "direct",
            stable("candidate-failure-direct")?,
            |block, payloads| {
                let failure = payloads.value::<EvmSubmissionFailure>(&[stable("failure")?])?;
                block.scope_failure::<SubmissionProgress>(&failure)
            },
        )?;
        routes.arm(
            "reconcile",
            stable("candidate-failure-reconcile")?,
            |block, payloads| {
                let request =
                    payloads.value::<FailureReconciliationRequest>(&[stable("request")?])?;
                let resolution = block
                    .state::<ReadCandidateStatusAfterFailureState>(
                        stable("read-candidate-status-after-failure")?,
                        &request,
                    )?
                    .or_default()?;
                block.normal(&resolution)
            },
        )
    }
}

pub(crate) fn evm_submission_recipe(
) -> mfm_program::Result<mfm_spec::structured::AuthoredStructuredProgram> {
    static PROGRAM: OnceLock<mfm_spec::structured::AuthoredStructuredProgram> = OnceLock::new();
    if let Some(program) = PROGRAM.get() {
        return Ok(program.clone());
    }
    let program = author_evm_submission_recipe(EVM_WALLET_REPLACEMENT_LIMIT as u16)?;
    let _ = PROGRAM.set(program.clone());
    Ok(program)
}

#[cfg(test)]
pub(crate) fn evm_submission_recipe_with_candidate_slots(
    candidate_slots: u16,
) -> mfm_program::Result<mfm_spec::structured::AuthoredStructuredProgram> {
    author_evm_submission_recipe(candidate_slots)
}

fn author_evm_submission_recipe(
    candidate_slots: u16,
) -> mfm_program::Result<mfm_spec::structured::AuthoredStructuredProgram> {
    let mut builder = OperationBuilder::<EvmSubmissionOutput, EvmSubmissionFailure>::new(
        stable("mfm.evm.expansion/submit-transaction")?,
        stable("mfm.evm.scope/submit-transaction")?,
    )?;
    builder
        .root()
        .failure_map::<EvmSubmissionFailure, EvmSubmissionFailureMapper>()?;
    let input = builder.input::<EvmSubmissionRequest>(stable("submission-request")?)?;
    let derived = builder
        .root()
        .state::<DeriveTransactionIntentState>(stable("derive-transaction-intent")?, &input)?
        .infallible()?;
    let intent = builder
        .root()
        .state::<DeriveSubmissionIntentIdState>(stable("derive-submission-intent")?, &derived)?
        .or_default()?;
    let prepared = builder
        .root()
        .state::<DeriveEvmNonceReservationKeyState>(stable("derive-reservation-key")?, &intent)?
        .or_default()?;
    let status = builder
        .root()
        .child::<InitialReservationChild>(
            stable("initial-reservation")?,
            vec![prepared.bind_child(initial_reservation_input_id()?)],
        )?
        .on_failure::<ReservationFailureHandler>()?;
    let mut progress = builder
        .root()
        .state::<CollapseWalletStatusState>(stable("collapse-initial-status")?, &status)?
        .infallible()?;
    for slot in 0..candidate_slots {
        progress = author_candidate_slot(builder.root(), &progress, slot)?;
    }
    let output = author_submission_terminal(builder.root(), &progress)?;
    let completion = builder.succeed(&output)?;
    builder.finish(completion)
}

pub(crate) fn evm_submission_entry_program(
    operation_id: StableId,
    scope_id: StableId,
) -> mfm_program::Result<mfm_spec::structured::AuthoredStructuredProgram> {
    let mut builder =
        OperationBuilder::<EvmSubmissionOutput, EvmSubmissionFailure>::new(operation_id, scope_id)?;
    builder
        .root()
        .failure_map::<EvmSubmissionFailure, EvmSubmissionFailureMapper>()?;
    let input = builder.input::<EvmSubmissionRequest>(stable("submission-request")?)?;
    let output = builder
        .root()
        .state::<crate::StructuredSubmitEvmTransactionState>(
            stable("submit-evm-transaction")?,
            &input,
        )?
        .or_default()?;
    let completion = builder.succeed(&output)?;
    builder.finish(completion)
}

pub(crate) fn initial_reservation_recipe(
) -> mfm_program::Result<mfm_spec::structured::AuthoredStructuredProgram> {
    static PROGRAM: OnceLock<mfm_spec::structured::AuthoredStructuredProgram> = OnceLock::new();
    if let Some(program) = PROGRAM.get() {
        return Ok(program.clone());
    }
    let program = author_initial_reservation_recipe()?;
    let _ = PROGRAM.set(program.clone());
    Ok(program)
}

fn author_initial_reservation_recipe(
) -> mfm_program::Result<mfm_spec::structured::AuthoredStructuredProgram> {
    let mut builder = OperationBuilder::<WalletStatusDecision, PendingEvmSubmissionFailure>::new(
        stable("mfm.evm.child/initial-reservation")?,
        stable("mfm.evm.scope/initial-reservation")?,
    )?;
    builder
        .root()
        .failure_map::<PendingEvmSubmissionFailure, PendingEvmSubmissionFailureMapper>()?;
    let prepared = builder.input::<PreparedWalletSubmission>(initial_reservation_input_id()?)?;
    let status = builder
        .root()
        .state::<ReadWalletNonceStatusState>(stable("read-initial-status")?, &prepared)?
        .or_default()?;
    let status = builder
        .root()
        .match_value(stable("match-initial-status")?, &status, |arms| {
            arms.arm("absent", stable("initial-status-absent")?, |arm, _| {
                let observed = arm
                    .state::<ObservePendingNonceState>(stable("observe-pending-nonce")?, &prepared)?
                    .or_default()?;
                let qualified = arm
                    .state::<QualifyPendingNonceFloorState>(
                        stable("qualify-pending-floor")?,
                        &observed,
                    )?
                    .or_default()?;
                let reserved = arm
                    .state::<ReserveWalletNonceState>(stable("reserve-wallet-nonce")?, &qualified)?
                    .or_default()?;
                let status = arm
                    .state::<ReadPostReserveWalletNonceStatusState>(
                        stable("read-post-reserve-status")?,
                        &reserved,
                    )?
                    .or_default()?;
                arm.normal(&status)
            })?;
            arms.arm("busy", stable("initial-status-busy")?, |arm, _| {
                arm.normal(&status)
            })?;
            arms.arm(
                "completed",
                stable("initial-status-completed")?,
                |arm, _| arm.normal(&status),
            )?;
            arms.arm("reserved", stable("initial-status-reserved")?, |arm, _| {
                arm.normal(&status)
            })
        })?;
    let completion = builder.succeed(&status)?;
    let program = builder.finish(completion)?;
    Ok(program)
}

pub(crate) fn candidate_attempt_recipe(
) -> mfm_program::Result<mfm_spec::structured::AuthoredStructuredProgram> {
    static PROGRAM: OnceLock<mfm_spec::structured::AuthoredStructuredProgram> = OnceLock::new();
    if let Some(program) = PROGRAM.get() {
        return Ok(program.clone());
    }
    let program = author_candidate_attempt_recipe()?;
    let _ = PROGRAM.set(program.clone());
    Ok(program)
}

fn author_candidate_attempt_recipe(
) -> mfm_program::Result<mfm_spec::structured::AuthoredStructuredProgram> {
    let mut builder = OperationBuilder::<SubmissionProgress, PendingEvmSubmissionFailure>::new(
        stable("mfm.evm.child/candidate-attempt")?,
        stable("mfm.evm.scope/candidate-attempt")?,
    )?;
    builder
        .root()
        .failure_map::<PendingEvmSubmissionFailure, PendingEvmSubmissionFailureMapper>()?;
    let work = builder.input::<SubmissionWork>(candidate_attempt_input_id()?)?;
    let route = builder
        .root()
        .state::<SelectCandidateAttemptRouteState>(stable("select-candidate-route")?, &work)?
        .infallible()?;
    let progress =
        builder
            .root()
            .match_value(stable("match-candidate-route")?, &route, |arms| {
                arms.arm("activate", stable("candidate-activate")?, |arm, _| {
                    let candidate = arm
                        .state::<BuildUnsignedCandidateState>(stable("build-candidate")?, &work)?
                        .or_default()?;
                    let candidate = arm
                        .state::<AttestCandidateIdentityState>(
                            stable("attest-candidate")?,
                            &candidate,
                        )?
                        .or_default()?;
                    let permitted = arm
                        .state::<DeriveCandidateActivationPermitState>(
                            stable("derive-activation-permit")?,
                            &candidate,
                        )?
                        .or_default()?;
                    let prepared = arm
                        .state::<DeriveEvmCandidateOperationKeyState>(
                            stable("derive-candidate-operation-key")?,
                            &permitted,
                        )?
                        .or_default()?;
                    let activation = arm
                        .state::<ActivateWalletCandidateState>(
                            stable("activate-candidate")?,
                            &prepared,
                        )?
                        .or_default()?;
                    let resolution = arm.match_value(
                        stable("match-candidate-activation")?,
                        &activation,
                        |arms| {
                            arms.arm(
                                "activated",
                                stable("candidate-activated")?,
                                |arm, payloads| {
                                    let active = payloads
                                        .value::<ActiveCandidateWork>(&[stable("active")?])?;
                                    let observed = arm
                                        .state::<BroadcastExactCandidateState>(
                                            stable("broadcast-candidate")?,
                                            &active,
                                        )?
                                        .or_default()?;
                                    let observed = author_observation_round(arm, &observed, 0)?;
                                    let observed = author_observation_round(arm, &observed, 1)?;
                                    let resolution = author_terminal_decision(arm, &observed)?;
                                    arm.normal(&resolution)
                                },
                            )?;
                            arms.arm(
                                "reconcile",
                                stable("candidate-activation-reconcile")?,
                                |arm, _| {
                                    let prepared = arm
                                        .state::<MarkActivationReconcileState>(
                                            stable("mark-activation-reconcile")?,
                                            &prepared,
                                        )?
                                        .infallible()?;
                                    let resolution =
                                        read_candidate_status(arm, "activation", &prepared)?;
                                    arm.normal(&resolution)
                                },
                            )
                        },
                    )?;
                    let progress = arm
                        .state::<CollapseCandidateResolutionState>(
                            stable("collapse-candidate-resolution")?,
                            &resolution,
                        )?
                        .infallible()?;
                    arm.normal(&progress)
                })?;
                arms.arm("exhausted", stable("candidate-exhausted")?, |arm, _| {
                    let request = arm
                        .state::<PrepareExhaustionReconciliationState>(
                            stable("prepare-exhaustion-reconciliation")?,
                            &work,
                        )?
                        .infallible()?;
                    let resolution = arm
                        .state::<ReadExhaustionStatusState>(
                            stable("read-exhaustion-status")?,
                            &request,
                        )?
                        .or_default()?;
                    let progress = arm
                        .state::<CollapseCandidateResolutionState>(
                            stable("collapse-exhaustion-resolution")?,
                            &resolution,
                        )?
                        .infallible()?;
                    arm.normal(&progress)
                })?;
                arms.arm(
                    "observe_retained",
                    stable("candidate-observe-retained")?,
                    |arm, _| {
                        let observed = arm
                            .state::<PrepareRetainedCandidateObservationState>(
                                stable("prepare-retained-observation")?,
                                &work,
                            )?
                            .or_default()?;
                        // Retained candidates always traverse the same fresh
                        // transaction/receipt observation rounds before any
                        // reconciliation or replacement decision. Construction
                        // of the observation work alone is never evidence.
                        let observed = author_observation_round(arm, &observed, 0)?;
                        let observed = author_observation_round(arm, &observed, 1)?;
                        let resolution = author_terminal_decision(arm, &observed)?;
                        let progress = arm
                            .state::<CollapseCandidateResolutionState>(
                                stable("collapse-observation-resolution")?,
                                &resolution,
                            )?
                            .infallible()?;
                        arm.normal(&progress)
                    },
                )
            })?;
    let completion = builder.succeed(&progress)?;
    builder.finish(completion)
}

fn author_candidate_slot(
    block: &mut BlockBuilder<EvmSubmissionFailure>,
    progress: &Value<SubmissionProgress>,
    slot: u16,
) -> mfm_program::Result<Value<SubmissionProgress>> {
    let decision = block
        .state::<SelectCandidateSlotState>(label("select-candidate", slot)?, progress)?
        .infallible()?;
    block.match_value(label("match-candidate", slot)?, &decision, |arms| {
        arms.arm("execute", label("candidate-execute", slot)?, |arm, _| {
            let work = arm
                .state::<ExtractSubmissionWorkState>(
                    label("extract-candidate-work", slot)?,
                    progress,
                )?
                .or_default()?;
            let resolution = arm
                .child::<CandidateAttemptChild>(
                    label("candidate-attempt", slot)?,
                    vec![work.bind_child(candidate_attempt_input_id()?)],
                )?
                .on_failure::<CandidateFailureHandler>()?;
            arm.normal(&resolution)
        })?;
        arms.arm("skip", label("candidate-skip", slot)?, |arm, _| {
            arm.normal(progress)
        })
    })
}

fn author_observation_round(
    block: &mut BlockBuilder<PendingEvmSubmissionFailure>,
    work: &Value<CandidateObservationWork>,
    round: u8,
) -> mfm_program::Result<Value<CandidateObservationWork>> {
    let decision = block
        .state::<SelectObservationRoundState>(label("select-observation", round)?, work)?
        .infallible()?;
    block.match_value(label("match-observation", round)?, &decision, |arms| {
        arms.arm("observe", label("observe-round", round)?, |arm, _| {
            let work = arm
                .state::<ObserveActivatedTransactionState>(
                    label("observe-transaction", round)?,
                    work,
                )?
                .or_default()?;
            let work = arm
                .state::<ObserveCandidateReceiptState>(label("observe-receipt", round)?, &work)?
                .or_default()?;
            arm.normal(&work)
        })?;
        arms.arm("skip", label("skip-round", round)?, |arm, _| {
            arm.normal(work)
        })
    })
}

fn author_terminal_decision(
    block: &mut BlockBuilder<PendingEvmSubmissionFailure>,
    observed: &Value<CandidateObservationWork>,
) -> mfm_program::Result<Value<CandidateResolution>> {
    let decision = block
        .state::<SelectTerminalEvidenceState>(stable("select-terminal-evidence")?, observed)?
        .infallible()?;
    block.match_value(stable("match-terminal-evidence")?, &decision, |arms| {
        arms.arm("observe_finality", stable("observe-finality")?, |arm, _| {
            let terminal = arm
                .state::<ObserveFinalizedHeadState>(stable("observe-finalized-head")?, observed)?
                .or_default()?;
            let terminal = arm
                .state::<ObserveCanonicalInclusionState>(
                    stable("observe-canonical-inclusion")?,
                    &terminal,
                )?
                .or_default()?;
            let completion = arm
                .state::<VerifyCanonicalInclusionState>(
                    stable("verify-canonical-inclusion")?,
                    &terminal,
                )?
                .or_default()?;
            let completion = arm
                .state::<CompleteWalletNonceState>(stable("complete-wallet-nonce")?, &completion)?
                .or_default()?;
            let resolution = arm
                .state::<MarkCandidateCompletedState>(
                    stable("mark-candidate-completed")?,
                    &completion,
                )?
                .infallible()?;
            arm.normal(&resolution)
        })?;
        arms.arm("reconcile", stable("terminal-reconcile")?, |arm, _| {
            let resolution = arm
                .state::<ReadObservedCandidateStatusState>(
                    stable("read-observed-candidate-status")?,
                    observed,
                )?
                .or_default()?;
            arm.normal(&resolution)
        })
    })
}

fn read_candidate_status(
    block: &mut BlockBuilder<PendingEvmSubmissionFailure>,
    suffix: &str,
    prepared: &Value<PreparedWalletSubmission>,
) -> mfm_program::Result<Value<CandidateResolution>> {
    block
        .state::<ReadCandidateWalletNonceStatusState>(
            label("read-candidate-wallet-status", suffix)?,
            prepared,
        )?
        .or_default()
}

fn author_submission_terminal(
    block: &mut BlockBuilder<EvmSubmissionFailure>,
    progress: &Value<SubmissionProgress>,
) -> mfm_program::Result<Value<EvmSubmissionOutput>> {
    let decision = block
        .state::<SelectSubmissionTerminalState>(stable("select-submission-terminal")?, progress)?
        .infallible()?;
    block.match_value(stable("match-submission-terminal")?, &decision, |arms| {
        arms.arm(
            "completed",
            stable("submission-completed")?,
            |arm, payloads| {
                let completion = payloads.value(&[stable("completion")?])?;
                let output = project_completion(arm, "terminal", &completion)?;
                arm.normal(&output)
            },
        )?;
        arms.arm(
            "exhausted",
            stable("submission-exhausted")?,
            |arm, payloads| {
                let failure = payloads.value(&[stable("failure")?])?;
                arm.scope_failure::<EvmSubmissionOutput>(&failure)
            },
        )?;
        arms.arm("failed", stable("submission-failed")?, |arm, payloads| {
            let failure = payloads.value(&[stable("failure")?])?;
            arm.scope_failure::<EvmSubmissionOutput>(&failure)
        })
    })
}

fn project_completion(
    block: &mut BlockBuilder<EvmSubmissionFailure>,
    suffix: &str,
    completion: &Value<crate::CompletedWalletNonce>,
) -> mfm_program::Result<Value<EvmSubmissionOutput>> {
    let projection = block
        .state::<ProjectCompletedWalletDispositionState>(
            label("project-completed-disposition", suffix)?,
            completion,
        )?
        .infallible()?;
    block.match_value(
        label("match-completed-disposition", suffix)?,
        &projection,
        |arms| {
            arms.arm(
                "failure",
                label("completed-failure", suffix)?,
                |arm, payloads| {
                    let failure = payloads.value(&[stable("failure")?])?;
                    arm.scope_failure::<EvmSubmissionOutput>(&failure)
                },
            )?;
            arms.arm(
                "success",
                label("completed-success", suffix)?,
                |arm, payloads| {
                    let output = payloads.value(&[stable("output")?])?;
                    arm.normal(&output)
                },
            )
        },
    )
}

fn initial_reservation_input_id() -> mfm_program::Result<StableId> {
    stable("prepared-wallet-submission")
}

fn candidate_attempt_input_id() -> mfm_program::Result<StableId> {
    stable("submission-work")
}

fn label(prefix: &str, suffix: impl ToString) -> mfm_program::Result<StableId> {
    stable(&format!("{prefix}-{}", suffix.to_string()))
}

fn stable(value: &str) -> mfm_program::Result<StableId> {
    StableId::new(value).map_err(|error| mfm_program::ProgramError::Authoring(error.to_string()))
}
