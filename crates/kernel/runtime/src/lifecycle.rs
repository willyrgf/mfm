//! Runtime-owned admission, resume, and owner-fate coordination.
//!
//! The lower-level typed primitives in [`crate::single_trust`] are useful to domain integration
//! tests and adapters. This module is the process-facing coordinator: it owns one immutable
//! assembly/store association, keeps the latest qualified context in an affine session, and
//! returns every live owner through an explicit outcome.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use mfm_capabilities::AccessCapabilityContract;
use mfm_ids::{ContentRef, RunId, StableId};
use mfm_program::{
    nominal_contract_ref, Program, ProgramCatalog, ProgramDocument, QualifiedTypedValue,
    QualifiedValue, State,
};
use mfm_store::single_trust::{AppendDisposition, QualifiedRun, RunAction, SelectedRun};
use mfm_store::{
    QualifiedHistoryPort, ResolvedConfigurationHead, SelectedConclusion, SelectedConclusionOutcome,
    SelectedConclusionPreparationOutcome,
};
use mfm_values::MfmValue;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::single_trust::{
    AccessImplementation, AccessResolution, CommittedCall, PreparedExecution, PureImplementation,
    RuntimeAssembly, RuntimeError, UnresolvedClassification,
};

type LifecycleResult<T> = std::result::Result<T, RuntimeError>;
type LifecycleFuture<T> = Pin<Box<dyn Future<Output = LifecycleResult<T>> + Send + 'static>>;

pub(crate) type ErasedValue = QualifiedValue;

pub(crate) enum DynamicOutcome {
    Success {
        value: ErasedValue,
        facts: mfm_facts::FactProposalSet,
    },
    Failure(ErasedValue),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_limits_require_each_live_work_lane() {
        assert!(RuntimeLimits::default().validate().is_ok());
        assert!(RuntimeLimits::new(0, 1, 1, 1).validate().is_err());
        assert!(RuntimeLimits::new(1, 0, 1, 1).validate().is_err());
        assert!(RuntimeLimits::new(1, 1, 0, 1).validate().is_err());
        assert!(RuntimeLimits::new(1, 1, 1, 0).validate().is_err());
    }
}

pub(crate) struct DynamicPreparationFailure {
    input: Option<ErasedValue>,
    error: RuntimeError,
}

/// Private erased registration bridge. It is constructed only by the typed assembly builder.
pub(crate) trait DynamicStateRegistration: Send + Sync {
    fn pure_evaluate(
        &self,
        assembly: &RuntimeAssembly,
        input: ErasedValue,
        input_contract: &ContentRef,
        output_contract: &ContentRef,
        failure_contract: Option<&ContentRef>,
    ) -> LifecycleResult<DynamicOutcome>;

    #[allow(clippy::result_large_err, clippy::too_many_arguments)]
    fn prepare_access(
        &self,
        assembly: &RuntimeAssembly,
        document: ProgramDocument,
        program_ref: ContentRef,
        run_id: RunId,
        occurrence: mfm_journal::single_trust::SequentialControlAddress,
        input: ErasedValue,
    ) -> std::result::Result<Box<dyn DynamicPrepared>, DynamicPreparationFailure>;
}

pub(crate) trait DynamicPrepared: Send {
    fn commit(
        self: Box<Self>,
        assembly: Arc<RuntimeAssembly>,
        store: Arc<mfm_store::QualifiedHistoryPort>,
        selected: SelectedRun,
    ) -> Pin<Box<dyn Future<Output = DynamicCommit> + Send + 'static>>;
}

#[allow(clippy::large_enum_variant)]
pub(crate) enum DynamicCommit {
    Direct {
        call: Box<dyn DynamicCall>,
        selected: SelectedRun,
    },
    Retained {
        prepared: Box<dyn DynamicPrepared>,
        disposition: Option<AppendDisposition>,
        selected: SelectedRun,
    },
}

pub(crate) trait DynamicCall: Send {
    fn execute(
        self: Box<Self>,
        assembly: Arc<RuntimeAssembly>,
    ) -> LifecycleFuture<DynamicResolution>;
}

struct DynamicPure<S: State> {
    implementation: PureImplementation<S>,
}

struct DynamicAccess<S: State, C: AccessCapabilityContract> {
    implementation: AccessImplementation<S, C>,
}

struct TypedPrepared<S: State, C: AccessCapabilityContract> {
    prepared: PreparedExecution<S, C>,
    implementation: AccessImplementation<S, C>,
}

struct TypedCall<S: State, C: AccessCapabilityContract> {
    call: CommittedCall<S, C>,
    implementation: AccessImplementation<S, C>,
}

pub(crate) fn pure_registration<S: State>(
    implementation: PureImplementation<S>,
) -> Arc<dyn DynamicStateRegistration> {
    Arc::new(DynamicPure { implementation })
}

pub(crate) fn access_registration<S: State, C: AccessCapabilityContract>(
    implementation: AccessImplementation<S, C>,
) -> Arc<dyn DynamicStateRegistration> {
    Arc::new(DynamicAccess { implementation })
}

fn qualify_erased<T: MfmValue>(
    assembly: &RuntimeAssembly,
    contract: ContentRef,
    value: T,
) -> LifecycleResult<ErasedValue> {
    let qualified = assembly
        .catalog()
        .qualify(contract, value)
        .map_err(|_| RuntimeError::Value)?;
    Ok(qualified.erase())
}

impl<S: State> DynamicStateRegistration for DynamicPure<S> {
    fn pure_evaluate(
        &self,
        assembly: &RuntimeAssembly,
        input: ErasedValue,
        input_contract: &ContentRef,
        output_contract: &ContentRef,
        failure_contract: Option<&ContentRef>,
    ) -> LifecycleResult<DynamicOutcome> {
        let input = input
            .try_downcast::<S::Input>(assembly.catalog(), input_contract)
            .map_err(|_| RuntimeError::Value)?;
        let outcome = self.implementation.evaluate_contained(input.into_value())?;
        match outcome {
            mfm_capabilities::ProposedStateOutcome::Success { output, facts } => {
                facts.validate().map_err(|_| RuntimeError::Value)?;
                Ok(DynamicOutcome::Success {
                    value: qualify_erased(assembly, output_contract.clone(), output)?,
                    facts,
                })
            }
            mfm_capabilities::ProposedStateOutcome::Failure { failure } => {
                let failure_contract = failure_contract.ok_or(RuntimeError::Value)?;
                Ok(DynamicOutcome::Failure(qualify_erased(
                    assembly,
                    failure_contract.clone(),
                    failure,
                )?))
            }
        }
    }

    fn prepare_access(
        &self,
        _assembly: &RuntimeAssembly,
        _document: ProgramDocument,
        _program_ref: ContentRef,
        _run_id: RunId,
        _occurrence: mfm_journal::single_trust::SequentialControlAddress,
        input: ErasedValue,
    ) -> std::result::Result<Box<dyn DynamicPrepared>, DynamicPreparationFailure> {
        Err(DynamicPreparationFailure {
            input: Some(input),
            error: RuntimeError::Mode,
        })
    }
}

impl<S: State, C: AccessCapabilityContract> DynamicStateRegistration for DynamicAccess<S, C> {
    fn pure_evaluate(
        &self,
        _assembly: &RuntimeAssembly,
        _input: ErasedValue,
        _input_contract: &ContentRef,
        _output_contract: &ContentRef,
        _failure_contract: Option<&ContentRef>,
    ) -> LifecycleResult<DynamicOutcome> {
        Err(RuntimeError::Mode)
    }

    fn prepare_access(
        &self,
        assembly: &RuntimeAssembly,
        document: ProgramDocument,
        program_ref: ContentRef,
        run_id: RunId,
        occurrence: mfm_journal::single_trust::SequentialControlAddress,
        input: ErasedValue,
    ) -> std::result::Result<Box<dyn DynamicPrepared>, DynamicPreparationFailure> {
        let state = match document.declaration(&occurrence) {
            Some(mfm_program::Declaration::State(state)) => state,
            _ => {
                return Err(DynamicPreparationFailure {
                    input: Some(input),
                    error: RuntimeError::Identity,
                })
            }
        };
        let input =
            match input.try_downcast::<S::Input>(assembly.catalog(), state.input_contract_ref()) {
                Ok(input) => input,
                Err(_) => {
                    return Err(DynamicPreparationFailure {
                        input: None,
                        error: RuntimeError::Value,
                    });
                }
            };
        let prepared = match PreparedExecution::new(
            assembly,
            &document,
            program_ref,
            run_id,
            occurrence,
            input,
        ) {
            Ok(prepared) => prepared,
            Err(error) => return Err(DynamicPreparationFailure { input: None, error }),
        };
        Ok(Box::new(TypedPrepared {
            prepared,
            implementation: self.implementation.clone(),
        }))
    }
}

impl<S: State, C: AccessCapabilityContract> DynamicPrepared for TypedPrepared<S, C> {
    fn commit(
        self: Box<Self>,
        assembly: Arc<RuntimeAssembly>,
        store: Arc<mfm_store::QualifiedHistoryPort>,
        selected: SelectedRun,
    ) -> Pin<Box<dyn Future<Output = DynamicCommit> + Send + 'static>> {
        Box::pin(async move {
            let TypedPrepared {
                prepared,
                implementation,
            } = *self;
            let committed = prepared.commit_opened(&assembly, &store, selected).await;

            match committed {
                crate::single_trust::OpenedPreparationCommit::Direct { call, selected } => {
                    DynamicCommit::Direct {
                        call: Box::new(TypedCall {
                            call,
                            implementation,
                        }) as Box<dyn DynamicCall>,
                        selected,
                    }
                }
                crate::single_trust::OpenedPreparationCommit::Retained {
                    owner,
                    selected,
                    disposition,
                } => DynamicCommit::Retained {
                    prepared: Box::new(TypedPrepared {
                        prepared: owner,
                        implementation,
                    }),
                    disposition: Some(disposition),
                    selected,
                },
                crate::single_trust::OpenedPreparationCommit::Rejected {
                    owner,
                    selected,
                    error: _,
                } => DynamicCommit::Retained {
                    prepared: Box::new(TypedPrepared {
                        prepared: owner,
                        implementation,
                    }),
                    disposition: None,
                    selected,
                },
            }
        })
    }
}

impl<S: State, C: AccessCapabilityContract> DynamicCall for TypedCall<S, C> {
    fn execute(
        self: Box<Self>,
        assembly: Arc<RuntimeAssembly>,
    ) -> LifecycleFuture<DynamicResolution> {
        let TypedCall {
            call,
            implementation,
        } = *self;
        let expected_call_id = call.call_id().clone();
        Box::pin(async move {
            let resolution = call.invoke_bound_adapter().await?;
            DynamicResolution::from_access(
                &assembly,
                &expected_call_id,
                &implementation,
                resolution,
            )
        })
    }
}

/// Result of an access implementation before Store conclusion qualification.
pub(crate) struct DynamicResolution {
    input: Option<ErasedValue>,
    evidence: Option<ErasedValue>,
    outcome: Option<DynamicOutcome>,
    classification: Option<UnresolvedClassification>,
    fact_continuation: Option<mfm_store::FactContinuation>,
}

impl DynamicResolution {
    fn from_access<S: State, C: AccessCapabilityContract>(
        assembly: &RuntimeAssembly,
        expected_call_id: &StableId,
        implementation: &AccessImplementation<S, C>,
        resolution: AccessResolution<S, C>,
    ) -> LifecycleResult<Self> {
        match resolution {
            AccessResolution::Outcome(access) => {
                let (brand, input, call_id, intent, evidence, _preparation, fact_continuation) =
                    access.into_parts();
                if !assembly.has_brand(&brand) || &call_id != expected_call_id {
                    return Err(RuntimeError::Identity);
                }
                let _intent =
                    qualify_erased(assembly, nominal_contract_ref::<C::Intent>()?, intent)?;
                let outcome = implementation.interpret_contained(input.into_value(), &evidence)?;
                let accepted_evidence =
                    qualify_erased(assembly, nominal_contract_ref::<C::Evidence>()?, evidence)?;
                let outcome = dynamic_outcome::<S>(assembly, outcome)?;
                Ok(Self {
                    input: None,
                    evidence: Some(accepted_evidence),
                    outcome: Some(outcome),
                    classification: None,
                    fact_continuation,
                })
            }
            AccessResolution::BlockedIntegrity(access) => {
                let (brand, input, call_id, intent, evidence, _preparation, fact_continuation) =
                    access.into_parts();
                if !assembly.has_brand(&brand) || &call_id != expected_call_id {
                    return Err(RuntimeError::Identity);
                }
                let _intent =
                    qualify_erased(assembly, nominal_contract_ref::<C::Intent>()?, intent)?;
                let evidence =
                    qualify_erased(assembly, nominal_contract_ref::<C::Evidence>()?, evidence)?;
                let failure = qualify_erased(
                    assembly,
                    nominal_contract_ref::<S::Failure>()?,
                    S::integrity_failure(input.as_ref()),
                )?;
                Ok(Self {
                    input: None,
                    evidence: Some(evidence),
                    outcome: Some(DynamicOutcome::Failure(failure)),
                    classification: None,
                    fact_continuation,
                })
            }
            AccessResolution::Unresolved(access) => {
                let (brand, input, call_id, intent, _preparation, _facts, classification) =
                    access.into_parts();
                if !assembly.has_brand(&brand) || &call_id != expected_call_id {
                    return Err(RuntimeError::Identity);
                }
                let _intent =
                    qualify_erased(assembly, nominal_contract_ref::<C::Intent>()?, intent)?;
                Ok(Self {
                    input: Some(input.erase()),
                    evidence: None,
                    outcome: None,
                    classification: Some(classification),
                    fact_continuation: None,
                })
            }
        }
    }
}

fn dynamic_outcome<S: State>(
    assembly: &RuntimeAssembly,
    outcome: mfm_capabilities::ProposedStateOutcome<S::Output, S::Failure>,
) -> LifecycleResult<DynamicOutcome> {
    match outcome {
        mfm_capabilities::ProposedStateOutcome::Success { output, facts } => {
            facts.validate().map_err(|_| RuntimeError::Value)?;
            Ok(DynamicOutcome::Success {
                value: qualify_erased(assembly, nominal_contract_ref::<S::Output>()?, output)?,
                facts,
            })
        }
        mfm_capabilities::ProposedStateOutcome::Failure { failure } => Ok(DynamicOutcome::Failure(
            qualify_erased(assembly, nominal_contract_ref::<S::Failure>()?, failure)?,
        )),
    }
}

/// Stable reason for retaining a live session without attempting another State callback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParkReason {
    /// The selected Access preparation is durable and awaits its provider-bound call.
    WaitingPreparation,
    /// The provider result is unresolved and the selected preparation remains durable.
    Unresolved(UnresolvedClassification),
}

/// Failure before an admission owner can become a live session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AdmissionFailure {
    /// The admission input does not belong to the Runtime's exact catalog/program.
    #[error("admission input is not qualified by this Runtime")]
    Identity,
    /// The Store rejected the admission without creating a live owner.
    #[error("admission could not be committed")]
    Store,
    /// The Runtime could not admit another bounded planning job.
    #[error("runtime admission capacity is unavailable")]
    Capacity,
}

/// Conflict between two semantic genesis candidates for one run identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("admission candidates conflict")]
pub struct AdmissionConflict;

/// Failure while loading a retained run for a Runtime resume.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ResumeFailure {
    /// The run is absent or retained history is invalid.
    #[error("run cannot be resumed")]
    History,
    /// The run belongs to another Runtime/store/catalog brand.
    #[error("run identity is invalid")]
    Identity,
    /// The Runtime could not admit another bounded active session.
    #[error("runtime resume capacity is unavailable")]
    Capacity,
}

/// Callback-free terminal evidence returned only after a durable conclusion is qualified.
pub struct TerminalRun {
    run: QualifiedRun,
}

impl TerminalRun {
    fn new(run: QualifiedRun) -> Self {
        Self { run }
    }

    /// Returns the durable run identity.
    pub fn run_id(&self) -> &RunId {
        self.run.run_id()
    }

    /// Returns the qualified terminal head sequence.
    pub fn head_sequence(&self) -> u64 {
        self.run.head_sequence()
    }

    /// Returns the callback-free retained evidence.
    pub fn qualified_run(&self) -> &QualifiedRun {
        &self.run
    }
}

/// A parked session that may be explicitly driven again by its owner.
pub struct ParkedRun {
    session: RunSession,
    reason: ParkReason,
}

impl ParkedRun {
    /// Returns why this session is parked.
    pub const fn reason(&self) -> ParkReason {
        self.reason
    }

    /// Consumes the parked owner back into its affine session.
    pub fn into_session(self) -> RunSession {
        self.session
    }

    /// Returns the durable run identity retained by this parked owner.
    pub fn run_id(&self) -> &RunId {
        self.session.run_id()
    }

    /// Returns the qualified durable head retained by this parked owner.
    pub fn head_sequence(&self) -> u64 {
        self.session.head_sequence()
    }
}

#[allow(clippy::large_enum_variant)]
enum SuspendedOwner {
    Admission {
        runtime: Runtime,
        owner: mfm_store::PreparedAdmission,
        value: ErasedValue,
    },
    Conclusion(PendingConclusion),
    Preparation {
        runtime: Runtime,
        prepared: Box<dyn DynamicPrepared>,
        selected: SelectedRun,
        occurrence: mfm_journal::single_trust::SequentialControlAddress,
        disposition: Option<AppendDisposition>,
    },
}

/// Runtime-owned affine conclusion handoff.
///
/// This owner contains the Store append authority and the retained session continuation, but no
/// provider, State implementation, raw response, or callback.  It can only be resolved through
/// the owning Runtime and cannot be cloned, serialized, or field-constructed by callers.
pub struct PendingConclusion {
    runtime: Runtime,
    owner: SelectedConclusion,
    successor: Option<ErasedValue>,
}

impl PendingConclusion {
    fn new(runtime: Runtime, owner: SelectedConclusion, successor: Option<ErasedValue>) -> Self {
        Self {
            runtime,
            owner,
            successor,
        }
    }

    /// Returns the run identity retained by this conclusion owner.
    pub fn run_id(&self) -> &RunId {
        self.owner.run_id()
    }

    async fn resolve(self) -> RuntimeStep {
        let Self {
            runtime,
            owner,
            successor,
        } = self;
        match runtime.inner.store.commit_selected_conclusion(owner).await {
            SelectedConclusionOutcome::AcknowledgementUnknown(owner) => {
                RuntimeStep::Suspended(SuspendedRun::conclusion(runtime, owner, successor))
            }
            SelectedConclusionOutcome::Rejected { owner, error } => {
                let suspended = SuspendedRun::conclusion(runtime, owner, successor);
                match error {
                    mfm_store::StoreError::FactFrontierChanged
                    | mfm_store::StoreError::Conflict
                    | mfm_store::StoreError::NotActionable => RuntimeStep::Suspended(suspended),
                    _ => RuntimeStep::ConclusionRejected {
                        owner: suspended,
                        error: error.into(),
                    },
                }
            }
            SelectedConclusionOutcome::AlreadyConcludedSame(selected)
            | SelectedConclusionOutcome::NoLongerSelected(selected)
            | SelectedConclusionOutcome::Committed(selected) => {
                runtime.step_from_selected(selected, successor).await
            }
            SelectedConclusionOutcome::Conflict(history) => RuntimeStep::Conflict {
                history,
                error: RuntimeError::Conclusion,
            },
            SelectedConclusionOutcome::InvalidHistory(history) => RuntimeStep::Failed {
                history,
                error: RuntimeError::Conclusion,
            },
        }
    }
}

/// Explicit owner retained across an acknowledgement-unknown or unresolved boundary.
pub struct SuspendedRun {
    owner: SuspendedOwner,
}

impl SuspendedRun {
    fn admission(
        runtime: Runtime,
        owner: mfm_store::PreparedAdmission,
        value: ErasedValue,
    ) -> Self {
        Self {
            owner: SuspendedOwner::Admission {
                runtime,
                owner,
                value,
            },
        }
    }

    fn conclusion(
        runtime: Runtime,
        owner: SelectedConclusion,
        successor: Option<ErasedValue>,
    ) -> Self {
        Self {
            owner: SuspendedOwner::Conclusion(PendingConclusion::new(runtime, owner, successor)),
        }
    }

    fn preparation(
        runtime: Runtime,
        prepared: Box<dyn DynamicPrepared>,
        selected: SelectedRun,
        occurrence: mfm_journal::single_trust::SequentialControlAddress,
        disposition: Option<AppendDisposition>,
    ) -> Self {
        Self {
            owner: SuspendedOwner::Preparation {
                runtime,
                prepared,
                selected,
                occurrence,
                disposition,
            },
        }
    }

    /// Returns the run identity retained by this owner-fate boundary.
    pub fn run_id(&self) -> &RunId {
        match &self.owner {
            SuspendedOwner::Admission { owner, .. } => owner.run_id(),
            SuspendedOwner::Conclusion(owner) => owner.run_id(),
            SuspendedOwner::Preparation { selected, .. } => selected.run_id(),
        }
    }

    /// Resolves the retained owner exactly once.
    pub async fn resolve(self) -> RuntimeStep {
        match self.owner {
            SuspendedOwner::Admission {
                runtime,
                owner,
                value,
            } => match runtime.inner.store.resolve_admission(owner).await {
                mfm_store::AdmissionOutcome::Selected(selected, _) => {
                    runtime.step_from_selected(selected, Some(value)).await
                }
                mfm_store::AdmissionOutcome::AcknowledgementUnknown(owner) => {
                    RuntimeStep::Suspended(SuspendedRun::admission(runtime, owner, value))
                }
                mfm_store::AdmissionOutcome::Conflict(history) => RuntimeStep::Conflict {
                    history,
                    error: RuntimeError::Conclusion,
                },
                mfm_store::AdmissionOutcome::Rejected(_) => {
                    RuntimeStep::AdmissionRejected(RuntimeError::Conclusion)
                }
                mfm_store::AdmissionOutcome::RetainedRejected { owner, .. } => {
                    RuntimeStep::Suspended(SuspendedRun::admission(runtime, owner, value))
                }
            },
            SuspendedOwner::Conclusion(owner) => owner.resolve().await,
            SuspendedOwner::Preparation {
                runtime,
                prepared,
                selected,
                occurrence,
                disposition,
            } => {
                if matches!(
                    disposition,
                    Some(disposition) if !matches!(disposition, AppendDisposition::AcknowledgementUnknown)
                ) {
                    return match runtime.inner.store.select(selected.run_id()).await {
                        Ok(latest) => runtime.step_from_selected(latest, None).await,
                        Err(_) => RuntimeStep::Suspended(SuspendedRun::preparation(
                            runtime,
                            prepared,
                            selected,
                            occurrence,
                            disposition,
                        )),
                    };
                }
                match prepared
                    .commit(
                        Arc::clone(&runtime.inner.assembly),
                        Arc::clone(&runtime.inner.store),
                        selected,
                    )
                    .await
                {
                    DynamicCommit::Direct { call, selected } => {
                        runtime
                            .execute_committed_access(occurrence, call, selected)
                            .await
                    }
                    DynamicCommit::Retained {
                        prepared,
                        disposition,
                        selected,
                    } => RuntimeStep::Suspended(SuspendedRun::preparation(
                        runtime,
                        prepared,
                        selected,
                        occurrence,
                        disposition,
                    )),
                }
            }
        }
    }
}

/// A non-Clone affine Runtime session retaining one latest cumulative context.
pub struct RunSession {
    runtime: Runtime,
    selected: SelectedRun,
    latest: ErasedValue,
    _active_permit: OwnedSemaphorePermit,
}

struct SessionBuildFailure {
    selected: SelectedRun,
    error: RuntimeError,
}

/// Result of one admission owner transition.
#[allow(clippy::large_enum_variant)]
pub enum SpawnStep {
    /// The admitted run has a live sequential session.
    Active(RunSession),
    /// The admitted run is already terminal, including valid zero-state admission.
    Terminal(TerminalRun),
    /// The physical admission acknowledgement must be resolved by the same owner.
    Suspended(SuspendedRun),
    /// A different semantic genesis is already durable.
    Conflict(AdmissionConflict),
    /// The admission failed before live execution authority existed.
    Failed(AdmissionFailure),
}

/// Result of one cold Runtime resume.
pub enum ResumeStep {
    /// The retained prefix has one executable selected action.
    Active(RunSession),
    /// The retained prefix is terminal.
    Terminal(TerminalRun),
    /// The retained prefix has a durable preparation or unresolved provider result.
    Parked(ParkedRun),
    /// The retained prefix cannot be resumed under this Runtime.
    Failed(ResumeFailure),
}

/// Result of one consumed session drive.
#[allow(clippy::large_enum_variant)]
pub enum RuntimeStep {
    /// A durable conclusion advanced the session to its next action.
    Advanced(RunSession),
    /// A durable qualified terminal conclusion is available.
    Terminal(TerminalRun),
    /// Preparation was rejected before a provider call existed.
    PreparationRejected {
        /// The exact session owner retained for a later explicit drive.
        session: RunSession,
        /// Redacted preparation failure.
        error: RuntimeError,
    },
    /// Provider entry did not yield a conclusive result.
    Unresolved {
        /// The exact session whose selected preparation remains durable.
        session: RunSession,
        /// Stable unresolved classification.
        classification: UnresolvedClassification,
    },
    /// The session is waiting for an explicit owner action.
    Parked {
        /// The exact retained session.
        session: RunSession,
        /// Stable parking reason.
        reason: ParkReason,
    },
    /// A live owner retains a physical acknowledgement or conclusion boundary.
    Suspended(SuspendedRun),
    /// A conclusion owner is retained with a permanent redaction-safe Store rejection.
    ConclusionRejected {
        /// The owner may be explicitly discarded or inspected by a supervisor.
        owner: SuspendedRun,
        /// The permanent rejection classification.
        error: RuntimeError,
    },
    /// Admission resolution failed before callback-free history was available.
    AdmissionRejected(RuntimeError),
    /// The consumed session raced with a different semantic head.
    Conflict {
        /// Qualified callback-free history at the competing head.
        history: QualifiedRun,
        /// Redacted conflict classification.
        error: RuntimeError,
    },
    /// The consumed owner reached a terminal operational failure with history retained.
    Failed {
        /// Qualified callback-free history available for diagnosis/replay.
        history: QualifiedRun,
        /// Redacted terminal failure.
        error: RuntimeError,
    },
}

/// Immutable process Runtime over one exact assembly and opened Store.
#[derive(Clone)]
pub struct Runtime {
    inner: Arc<RuntimeInner>,
}

/// Fixed process-local Runtime work limits.
///
/// These limits bound independent owners and deterministic work without introducing a scheduler,
/// per-run lock, or process-wide writer lease.  Provider futures use the ingress bound; pure,
/// preparation, qualification, and interpretation work use the corresponding deterministic
/// bounds.  A session holds one active-session permit for its entire affine lifetime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeLimits {
    max_active_sessions: usize,
    max_cpu_jobs: usize,
    max_planning_jobs: usize,
    max_ingress_jobs: usize,
}

impl RuntimeLimits {
    /// Creates one non-zero bounded Runtime envelope.
    pub const fn new(
        max_active_sessions: usize,
        max_cpu_jobs: usize,
        max_planning_jobs: usize,
        max_ingress_jobs: usize,
    ) -> Self {
        Self {
            max_active_sessions,
            max_cpu_jobs,
            max_planning_jobs,
            max_ingress_jobs,
        }
    }

    /// Returns the selected default envelope for the current MVP entry points.
    pub const fn default_envelope() -> Self {
        Self::new(64, 8, 8, 64)
    }

    /// Validates that every Runtime work lane has at least one permit.
    pub const fn validate(self) -> LifecycleResult<()> {
        if self.max_active_sessions == 0
            || self.max_cpu_jobs == 0
            || self.max_planning_jobs == 0
            || self.max_ingress_jobs == 0
        {
            Err(RuntimeError::Capacity)
        } else {
            Ok(())
        }
    }

    /// Returns the active-session bound.
    pub const fn max_active_sessions(self) -> usize {
        self.max_active_sessions
    }

    /// Returns the deterministic CPU-job bound.
    pub const fn max_cpu_jobs(self) -> usize {
        self.max_cpu_jobs
    }

    /// Returns the planning-job bound.
    pub const fn max_planning_jobs(self) -> usize {
        self.max_planning_jobs
    }

    /// Returns the provider-ingress bound.
    pub const fn max_ingress_jobs(self) -> usize {
        self.max_ingress_jobs
    }
}

impl Default for RuntimeLimits {
    fn default() -> Self {
        Self::default_envelope()
    }
}

struct RuntimeInner {
    assembly: Arc<RuntimeAssembly>,
    store: Arc<QualifiedHistoryPort>,
    limits: RuntimeLimits,
    active_sessions: Arc<Semaphore>,
    cpu_jobs: Arc<Semaphore>,
    planning_jobs: Arc<Semaphore>,
    ingress_jobs: Arc<Semaphore>,
}

impl Runtime {
    /// Opens one Runtime over an exact immutable assembly and branded Store.
    pub fn new(assembly: RuntimeAssembly, store: QualifiedHistoryPort) -> LifecycleResult<Self> {
        Self::new_with_limits(assembly, store, RuntimeLimits::default())
    }

    /// Opens one Runtime with one exact immutable assembly, Store, and bounded work envelope.
    pub fn new_with_limits(
        assembly: RuntimeAssembly,
        store: QualifiedHistoryPort,
        limits: RuntimeLimits,
    ) -> LifecycleResult<Self> {
        limits.validate()?;
        if !assembly.catalog().same_catalog(store.catalog()) {
            return Err(RuntimeError::Identity);
        }
        Ok(Self {
            inner: Arc::new(RuntimeInner {
                assembly: Arc::new(assembly),
                store: Arc::new(store),
                limits,
                active_sessions: Arc::new(Semaphore::new(limits.max_active_sessions)),
                cpu_jobs: Arc::new(Semaphore::new(limits.max_cpu_jobs)),
                planning_jobs: Arc::new(Semaphore::new(limits.max_planning_jobs)),
                ingress_jobs: Arc::new(Semaphore::new(limits.max_ingress_jobs)),
            }),
        })
    }

    /// Returns this Runtime's immutable bounded work envelope.
    pub fn limits(&self) -> RuntimeLimits {
        self.inner.limits
    }

    async fn acquire(
        semaphore: &Arc<Semaphore>,
    ) -> std::result::Result<OwnedSemaphorePermit, RuntimeError> {
        semaphore
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| RuntimeError::Capacity)
    }

    async fn acquire_active_session(&self) -> LifecycleResult<OwnedSemaphorePermit> {
        Self::acquire(&self.inner.active_sessions).await
    }

    async fn acquire_cpu_job(&self) -> LifecycleResult<OwnedSemaphorePermit> {
        Self::acquire(&self.inner.cpu_jobs).await
    }

    async fn acquire_planning_job(&self) -> LifecycleResult<OwnedSemaphorePermit> {
        Self::acquire(&self.inner.planning_jobs).await
    }

    async fn acquire_ingress_job(&self) -> LifecycleResult<OwnedSemaphorePermit> {
        Self::acquire(&self.inner.ingress_jobs).await
    }

    /// Creates one typed admission owner from a planning-owned singular `C0`.
    pub fn admission<T: MfmValue>(
        &self,
        run_id: RunId,
        program: Program,
        value: QualifiedTypedValue<T>,
        configuration: ResolvedConfigurationHead,
        source_refs: Vec<ContentRef>,
    ) -> LifecycleResult<AdmissionInput<T>> {
        AdmissionInput::new(self, run_id, program, value, configuration, source_refs)
    }

    /// Resumes one run by cold-qualified retained context owned by this Runtime.
    ///
    /// This is the process-facing entry point used by transports that retain only a run id.  It
    /// performs the same bounded prefix qualification as typed [`Runtime::resume`] and never
    /// exposes the retained history to State code.
    pub async fn resume_run(&self, run_id: RunId) -> ResumeStep {
        let selected = match self.inner.store.select(&run_id).await {
            Ok(selected) => selected,
            Err(_) => return ResumeStep::Failed(ResumeFailure::History),
        };
        match self.session_from_selected(selected, None).await {
            Ok(session) => self.classify_resume_session(session).await,
            Err(SessionBuildFailure {
                error: RuntimeError::Capacity,
                ..
            }) => ResumeStep::Failed(ResumeFailure::Capacity),
            Err(_) => ResumeStep::Failed(ResumeFailure::Identity),
        }
    }

    /// Returns immutable Store identity metadata for trusted composition.
    pub fn store_identity(&self) -> &mfm_store::StructuredStoreIdentity {
        self.inner.store.identity()
    }

    /// Returns the exact callback-free Program catalog associated with this Runtime.
    pub fn catalog(&self) -> ProgramCatalog {
        self.inner.assembly.catalog().clone()
    }

    /// Validates that one catalog-qualified final Program resolves entirely in this live assembly.
    ///
    /// Trusted composition uses this before exposing an Application; admission repeats the same
    /// check while creating its affine owner.
    pub fn validate_program(&self, program: &Program) -> LifecycleResult<()> {
        self.inner.assembly.validate_program(program)
    }

    /// Returns whether a configuration head was issued by this Runtime's exact Store opening.
    pub fn accepts_configuration_head(&self, head: &ResolvedConfigurationHead) -> bool {
        self.inner.store.accepts_configuration_head(head)
    }

    async fn classify_resume_session(&self, session: RunSession) -> ResumeStep {
        match session.selected.action().clone() {
            RunAction::ZeroStateTerminal { .. }
            | RunAction::Terminal { .. }
            | RunAction::Failed { .. } => {
                ResumeStep::Terminal(TerminalRun::new(session.selected.into_qualified_run()))
            }
            RunAction::WaitingPreparation { .. } => ResumeStep::Parked(ParkedRun {
                session,
                reason: ParkReason::WaitingPreparation,
            }),
            _ => ResumeStep::Active(session),
        }
    }

    async fn execute_committed_access(
        &self,
        _occurrence: mfm_journal::single_trust::SequentialControlAddress,
        call: Box<dyn DynamicCall>,
        selected: SelectedRun,
    ) -> RuntimeStep {
        let resolution = {
            let _ingress_permit = match self.acquire_ingress_job().await {
                Ok(permit) => permit,
                Err(error) => {
                    return RuntimeStep::Failed {
                        history: selected.into_qualified_run(),
                        error,
                    }
                }
            };
            let _cpu_permit = match self.acquire_cpu_job().await {
                Ok(permit) => permit,
                Err(error) => {
                    return RuntimeStep::Failed {
                        history: selected.into_qualified_run(),
                        error,
                    }
                }
            };
            call.execute(Arc::clone(&self.inner.assembly)).await.ok()
        };
        let Some(resolution) = resolution else {
            return self
                .neutral_access(selected, UnresolvedClassification::AcknowledgementUnknown)
                .await;
        };
        if let Some(classification) = resolution.classification {
            let Some(input) = resolution.input else {
                return self
                    .neutral_access(selected, UnresolvedClassification::InvalidResponse)
                    .await;
            };
            return match self.session_from_selected(selected, Some(input)).await {
                Ok(session) => RuntimeStep::Unresolved {
                    session,
                    classification,
                },
                Err(failure) => RuntimeStep::Failed {
                    history: failure.selected.into_qualified_run(),
                    error: failure.error,
                },
            };
        }
        let evidence = match resolution.evidence {
            Some(evidence) => evidence,
            None => {
                return self
                    .neutral_access(selected, UnresolvedClassification::InvalidResponse)
                    .await;
            }
        };
        let outcome = match resolution.outcome {
            Some(outcome) => outcome,
            None => {
                return self
                    .neutral_access(selected, UnresolvedClassification::InvalidResponse)
                    .await;
            }
        };
        let evidence = evidence;
        let proposal = match outcome {
            DynamicOutcome::Success { value, facts } => {
                let output = value;
                mfm_capabilities::ProposedStateOutcome::Success { output, facts }
            }
            DynamicOutcome::Failure(value) => {
                let failure = value;
                mfm_capabilities::ProposedStateOutcome::Failure { failure }
            }
        };
        let owner = match self.inner.store.prepare_selected_access_conclusion(
            selected,
            evidence,
            proposal,
            resolution.fact_continuation,
        ) {
            SelectedConclusionPreparationOutcome::Prepared(owner) => owner,
            SelectedConclusionPreparationOutcome::Rejected { selected, error } => {
                return RuntimeStep::Failed {
                    history: selected.into_qualified_run(),
                    error: error.into(),
                }
            }
        };
        match self.inner.store.commit_selected_conclusion(owner).await {
            SelectedConclusionOutcome::AcknowledgementUnknown(owner) => {
                RuntimeStep::Suspended(SuspendedRun::conclusion(self.clone(), owner, None))
            }
            SelectedConclusionOutcome::Rejected { owner, error } => {
                let suspended = SuspendedRun::conclusion(self.clone(), owner, None);
                match error {
                    mfm_store::StoreError::FactFrontierChanged
                    | mfm_store::StoreError::Conflict
                    | mfm_store::StoreError::NotActionable => RuntimeStep::Suspended(suspended),
                    _ => RuntimeStep::ConclusionRejected {
                        owner: suspended,
                        error: error.into(),
                    },
                }
            }
            SelectedConclusionOutcome::AlreadyConcludedSame(next)
            | SelectedConclusionOutcome::NoLongerSelected(next)
            | SelectedConclusionOutcome::Committed(next) => {
                self.step_from_selected(next, None).await
            }
            SelectedConclusionOutcome::Conflict(history) => RuntimeStep::Conflict {
                history,
                error: RuntimeError::Conclusion,
            },
            SelectedConclusionOutcome::InvalidHistory(history) => RuntimeStep::Failed {
                history,
                error: RuntimeError::Conclusion,
            },
        }
    }

    async fn step_from_selected(
        &self,
        selected: SelectedRun,
        successor: Option<ErasedValue>,
    ) -> RuntimeStep {
        match self.session_from_selected(selected, successor).await {
            Ok(session) => match session.selected.action() {
                RunAction::ZeroStateTerminal { .. }
                | RunAction::Terminal { .. }
                | RunAction::Failed { .. } => {
                    RuntimeStep::Terminal(TerminalRun::new(session.selected.into_qualified_run()))
                }
                _ => RuntimeStep::Advanced(session),
            },
            Err(failure) => RuntimeStep::Failed {
                history: failure.selected.into_qualified_run(),
                error: failure.error,
            },
        }
    }

    async fn neutral_access(
        &self,
        selected: SelectedRun,
        classification: UnresolvedClassification,
    ) -> RuntimeStep {
        match self.session_from_selected(selected, None).await {
            Ok(session) => RuntimeStep::Unresolved {
                session,
                classification,
            },
            Err(failure) => RuntimeStep::Failed {
                history: failure.selected.into_qualified_run(),
                error: failure.error,
            },
        }
    }

    async fn spawn_typed<T: MfmValue>(
        &self,
        run_id: RunId,
        program: Program,
        value: QualifiedTypedValue<T>,
        configuration: ResolvedConfigurationHead,
        source_refs: Vec<ContentRef>,
    ) -> SpawnStep {
        let _planning_permit = match self.acquire_planning_job().await {
            Ok(permit) => permit,
            Err(_) => return SpawnStep::Failed(AdmissionFailure::Capacity),
        };
        let outcome = self
            .inner
            .store
            .admit(run_id, &program, &value, configuration, source_refs)
            .await;
        let value = value.erase();
        match outcome {
            mfm_store::AdmissionOutcome::Selected(selected, _) => {
                match self.session_from_selected(selected, Some(value)).await {
                    Ok(session) => self.spawn_classify(session).await,
                    Err(_) => SpawnStep::Failed(AdmissionFailure::Store),
                }
            }
            mfm_store::AdmissionOutcome::AcknowledgementUnknown(owner)
            | mfm_store::AdmissionOutcome::RetainedRejected { owner, .. } => {
                SpawnStep::Suspended(SuspendedRun::admission(self.clone(), owner, value))
            }
            mfm_store::AdmissionOutcome::Conflict(_) => SpawnStep::Conflict(AdmissionConflict),
            mfm_store::AdmissionOutcome::Rejected(_) => SpawnStep::Failed(AdmissionFailure::Store),
        }
    }

    async fn spawn_classify(&self, session: RunSession) -> SpawnStep {
        match session.selected.action().clone() {
            RunAction::ZeroStateTerminal { .. }
            | RunAction::Terminal { .. }
            | RunAction::Failed { .. } => {
                SpawnStep::Terminal(TerminalRun::new(session.selected.into_qualified_run()))
            }
            _ => SpawnStep::Active(session),
        }
    }

    async fn session_from_selected(
        &self,
        selected: SelectedRun,
        supplied: Option<ErasedValue>,
    ) -> std::result::Result<RunSession, SessionBuildFailure> {
        if self
            .inner
            .assembly
            .validate_program(selected.program())
            .is_err()
        {
            return Err(SessionBuildFailure {
                selected,
                error: RuntimeError::Identity,
            });
        }
        let latest_result = if let Some(value) = supplied {
            Ok(value)
        } else {
            let object = selected.latest_context_object();
            let assembly = Arc::clone(&self.inner.assembly);
            let contract = selected.latest_context().contract_ref().clone();
            let canonical_bytes = object.canonical_json().as_bytes().to_vec();
            match self.acquire_cpu_job().await {
                Ok(cpu_permit) => tokio::task::spawn_blocking(move || {
                    let _cpu_permit = cpu_permit;
                    assembly.reify_value(&contract, &canonical_bytes)
                })
                .await
                .map_err(|_| RuntimeError::Value)
                .and_then(std::convert::identity),
                Err(error) => Err(error),
            }
        };
        let latest = match latest_result {
            Ok(value) => value,
            Err(error) => return Err(SessionBuildFailure { selected, error }),
        };
        if latest.value_ref() != selected.latest_context().value_ref()
            || latest.contract_ref() != selected.latest_context().contract_ref()
        {
            #[cfg(test)]
            eprintln!(
                "latest mismatch supplied={:?} selected={:?} supplied_contract={:?} reduced_contract={:?}",
                latest.value_ref(),
                selected.latest_context().value_ref(),
                latest.contract_ref(),
                selected.latest_context().contract_ref(),
            );
            return Err(SessionBuildFailure {
                selected,
                error: RuntimeError::Identity,
            });
        }
        let active_permit = match self.acquire_active_session().await {
            Ok(permit) => permit,
            Err(error) => return Err(SessionBuildFailure { selected, error }),
        };
        Ok(RunSession {
            runtime: self.clone(),
            selected,
            latest,
            _active_permit: active_permit,
        })
    }
}

impl RunSession {
    /// Returns the durable run identity retained by this affine session.
    pub fn run_id(&self) -> &RunId {
        self.selected.run_id()
    }

    /// Returns the qualified durable head retained by this affine session.
    pub fn head_sequence(&self) -> u64 {
        self.selected.head_sequence()
    }

    fn drive_access(
        self,
        occurrence: mfm_journal::single_trust::SequentialControlAddress,
    ) -> Pin<Box<dyn Future<Output = RuntimeStep> + Send + 'static>> {
        Box::pin(self.drive_access_inner(occurrence))
    }

    #[allow(clippy::result_large_err)]
    async fn drive_access_inner(
        self,
        occurrence: mfm_journal::single_trust::SequentialControlAddress,
    ) -> RuntimeStep {
        let RunSession {
            runtime,
            selected,
            latest,
            _active_permit,
        } = self;
        let document = selected.program().document().clone();
        let program_ref = selected.program_ref().clone();
        let state = match document.declaration(&occurrence) {
            Some(mfm_program::Declaration::State(state)) => state,
            _ => {
                return RuntimeStep::Failed {
                    history: selected.into_qualified_run(),
                    error: RuntimeError::Identity,
                }
            }
        };
        let registration = match runtime
            .inner
            .assembly
            .dynamic_registration(state.state_implementation_ref())
        {
            Ok(registration) => registration,
            Err(error) => {
                return RuntimeStep::Failed {
                    history: selected.into_qualified_run(),
                    error,
                }
            }
        };
        let _planning_permit = match runtime.acquire_planning_job().await {
            Ok(permit) => permit,
            Err(error) => {
                return RuntimeStep::Failed {
                    history: selected.into_qualified_run(),
                    error,
                }
            }
        };
        let assembly = Arc::clone(&runtime.inner.assembly);
        let run_id = selected.run_id().clone();
        let prepare_occurrence = occurrence.clone();
        let prepared_result = match tokio::task::spawn_blocking(move || {
            let _planning_permit = _planning_permit;
            registration.prepare_access(
                &assembly,
                document,
                program_ref,
                run_id,
                prepare_occurrence,
                latest,
            )
        })
        .await
        {
            Ok(result) => result,
            Err(_) => Err(DynamicPreparationFailure {
                input: None,
                error: RuntimeError::Preparation,
            }),
        };
        let prepared = match prepared_result {
            Ok(prepared) => prepared,
            Err(failure) => {
                if let Some(input) = failure.input {
                    return RuntimeStep::PreparationRejected {
                        session: RunSession {
                            runtime,
                            selected,
                            latest: input,
                            _active_permit,
                        },
                        error: failure.error,
                    };
                }
                return RuntimeStep::Failed {
                    history: selected.into_qualified_run(),
                    error: failure.error,
                };
            }
        };
        drop(_active_permit);
        let committed = prepared
            .commit(
                Arc::clone(&runtime.inner.assembly),
                Arc::clone(&runtime.inner.store),
                selected,
            )
            .await;
        match committed {
            DynamicCommit::Direct { call, selected } => {
                return runtime
                    .execute_committed_access(occurrence, call, selected)
                    .await;
            }
            DynamicCommit::Retained {
                prepared,
                disposition,
                selected,
            } => RuntimeStep::Suspended(SuspendedRun::preparation(
                runtime,
                prepared,
                selected,
                occurrence,
                disposition,
            )),
        }
    }

    async fn drive_pure(
        self,
        occurrence: mfm_journal::single_trust::SequentialControlAddress,
    ) -> RuntimeStep {
        let RunSession {
            runtime,
            selected,
            latest,
            _active_permit,
        } = self;
        let document = selected.program().document().clone();
        let state = match document.declaration(&occurrence) {
            Some(mfm_program::Declaration::State(state)) => state,
            _ => {
                return RuntimeStep::Failed {
                    history: selected.into_qualified_run(),
                    error: RuntimeError::Identity,
                }
            }
        };
        let registration = match runtime
            .inner
            .assembly
            .dynamic_registration(state.state_implementation_ref())
        {
            Ok(registration) => registration,
            Err(error) => {
                return RuntimeStep::Failed {
                    history: selected.into_qualified_run(),
                    error,
                };
            }
        };
        let input_contract = state.input_contract_ref().clone();
        let output_contract = state.output_contract_ref().clone();
        let failure_contract = state.failure_contract_ref().cloned();
        let assembly = Arc::clone(&runtime.inner.assembly);
        let cpu_permit = match runtime.acquire_cpu_job().await {
            Ok(permit) => permit,
            Err(error) => {
                return RuntimeStep::Failed {
                    history: selected.into_qualified_run(),
                    error,
                }
            }
        };
        let outcome = match tokio::task::spawn_blocking(move || {
            let _cpu_permit = cpu_permit;
            registration.pure_evaluate(
                &assembly,
                latest,
                &input_contract,
                &output_contract,
                failure_contract.as_ref(),
            )
        })
        .await
        {
            Ok(Ok(outcome)) => outcome,
            Ok(Err(error)) => {
                return RuntimeStep::Failed {
                    history: selected.into_qualified_run(),
                    error,
                };
            }
            Err(_) => {
                return RuntimeStep::Failed {
                    history: selected.into_qualified_run(),
                    error: RuntimeError::Unresolved,
                };
            }
        };
        let proposal = match outcome {
            DynamicOutcome::Success { value, facts } => {
                let output = value;
                mfm_capabilities::ProposedStateOutcome::Success { output, facts }
            }
            DynamicOutcome::Failure(value) => {
                let failure = value;
                mfm_capabilities::ProposedStateOutcome::Failure { failure }
            }
        };
        drop(_active_permit);
        let owner = match runtime
            .inner
            .store
            .prepare_selected_pure_conclusion(selected, proposal)
        {
            SelectedConclusionPreparationOutcome::Prepared(owner) => owner,
            SelectedConclusionPreparationOutcome::Rejected { selected, error } => {
                return RuntimeStep::Failed {
                    history: selected.into_qualified_run(),
                    error: error.into(),
                };
            }
        };
        match runtime.inner.store.commit_selected_conclusion(owner).await {
            SelectedConclusionOutcome::AcknowledgementUnknown(owner) => {
                RuntimeStep::Suspended(SuspendedRun::conclusion(runtime.clone(), owner, None))
            }
            SelectedConclusionOutcome::Rejected { owner, error } => {
                let suspended = SuspendedRun::conclusion(runtime.clone(), owner, None);
                match error {
                    mfm_store::StoreError::FactFrontierChanged
                    | mfm_store::StoreError::Conflict
                    | mfm_store::StoreError::NotActionable => RuntimeStep::Suspended(suspended),
                    _ => RuntimeStep::ConclusionRejected {
                        owner: suspended,
                        error: error.into(),
                    },
                }
            }
            SelectedConclusionOutcome::AlreadyConcludedSame(next)
            | SelectedConclusionOutcome::NoLongerSelected(next)
            | SelectedConclusionOutcome::Committed(next) => {
                runtime.step_from_selected(next, None).await
            }
            SelectedConclusionOutcome::Conflict(history) => RuntimeStep::Conflict {
                history,
                error: RuntimeError::Conclusion,
            },
            SelectedConclusionOutcome::InvalidHistory(history) => RuntimeStep::Failed {
                history,
                error: RuntimeError::Conclusion,
            },
        }
    }

    /// Consumes this session and performs one deterministic Runtime step.
    pub async fn drive(self) -> RuntimeStep {
        let action = self.selected.action().clone();
        match action {
            RunAction::ReadyPure { occurrence, .. } => self.drive_pure(occurrence).await,
            RunAction::WaitingPreparation { .. } => RuntimeStep::Parked {
                session: self,
                reason: ParkReason::WaitingPreparation,
            },
            RunAction::ReadyAccess { occurrence, .. } => self.drive_access(occurrence).await,
            RunAction::ZeroStateTerminal { .. }
            | RunAction::Terminal { .. }
            | RunAction::Failed { .. } => {
                RuntimeStep::Terminal(TerminalRun::new(self.selected.into_qualified_run()))
            }
        }
    }
}

/// Typed singular admission owner.
pub struct AdmissionInput<T: MfmValue> {
    runtime: Runtime,
    run_id: RunId,
    program: Program,
    value: QualifiedTypedValue<T>,
    configuration: ResolvedConfigurationHead,
    source_refs: Vec<ContentRef>,
}

impl<T: MfmValue> AdmissionInput<T> {
    /// Mints an admission owner from one catalog-qualified planning value.
    pub fn new(
        runtime: &Runtime,
        run_id: RunId,
        program: Program,
        value: QualifiedTypedValue<T>,
        configuration: ResolvedConfigurationHead,
        source_refs: Vec<ContentRef>,
    ) -> LifecycleResult<Self> {
        if !program.belongs_to_catalog(runtime.inner.assembly.catalog())
            || runtime.inner.assembly.validate_program(&program).is_err()
            || !value.belongs_to_catalog(runtime.inner.assembly.catalog())
            || value.contract_ref() != program.document().admitted_context_contract_ref()
        {
            return Err(RuntimeError::Identity);
        }
        Ok(Self {
            runtime: runtime.clone(),
            run_id,
            program,
            value,
            configuration,
            source_refs,
        })
    }

    /// Consumes this owner into an exhaustive admission outcome.
    pub async fn spawn(self) -> SpawnStep {
        self.runtime
            .spawn_typed(
                self.run_id,
                self.program,
                self.value,
                self.configuration,
                self.source_refs,
            )
            .await
    }
}
