//! Runtime-owned admission, resume, and owner-fate coordination.
//!
//! The lower-level typed primitives in [`crate::single_trust`] are useful to domain integration
//! tests and adapters. This module is the process-facing coordinator: it owns one immutable
//! assembly/store association, keeps the latest qualified context in an affine session, and
//! returns every live owner through an explicit outcome.

use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::Arc;

use mfm_capabilities::AccessCapabilityContract;
#[cfg(test)]
use mfm_ids::AppendRequestId;
use mfm_ids::{ContentRef, RunId, StableId};
use mfm_program::{nominal_contract_ref, ProgramCatalog, QualifiedTypedValue, QualifiedValue};
use mfm_store::single_trust::{AppendDisposition, QualifiedRun, RunAction, SelectedRun};
use mfm_store::{
    QualifiedHistoryPort, ResolvedConfigurationHead, SelectedConclusion, SelectedConclusionOutcome,
    SelectedConclusionPreparationOutcome,
};
use mfm_values::MfmValue;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::single_trust::{
    AccessHandlerResolution, AccessImplementation, CommittedCall, PreparedExecution,
    PureImplementation, RuntimeAssembly, RuntimeError, State, UnresolvedClassification,
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
        let outcome = self.implementation.evaluate_contained(input.as_ref())?;
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
        run_id: RunId,
        occurrence: mfm_journal::single_trust::SequentialControlAddress,
        input: ErasedValue,
    ) -> std::result::Result<Box<dyn DynamicPrepared>, DynamicPreparationFailure> {
        let state = match assembly.program().document().declaration(&occurrence) {
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
        let prepared = match PreparedExecution::new(assembly, run_id, occurrence, input) {
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
            let resolution = implementation.execute(call).await?;
            DynamicResolution::from_handler(&assembly, &expected_call_id, resolution)
        })
    }
}

/// Result of an access implementation before Store conclusion qualification.
pub(crate) struct DynamicResolution {
    input: ErasedValue,
    evidence: Option<ErasedValue>,
    outcome: Option<DynamicOutcome>,
    classification: Option<UnresolvedClassification>,
    fact_continuation: Option<mfm_store::FactContinuation>,
}

impl DynamicResolution {
    fn from_handler<S: State, C: AccessCapabilityContract>(
        assembly: &RuntimeAssembly,
        expected_call_id: &StableId,
        resolution: AccessHandlerResolution<S, S::Output, S::Failure, C>,
    ) -> LifecycleResult<Self> {
        let (
            brand,
            input,
            call_id,
            intent,
            evidence,
            outcome,
            classification,
            _preparation,
            fact_continuation,
        ) = resolution.into_parts();
        if !assembly.has_brand(&brand) || &call_id != expected_call_id {
            return Err(RuntimeError::Identity);
        }
        if classification.is_some() != (evidence.is_none() && outcome.is_none())
            || classification.is_none() != (evidence.is_some() && outcome.is_some())
        {
            return Err(RuntimeError::Value);
        }
        let input = input.erase();
        let _intent = qualify_erased(assembly, nominal_contract_ref::<C::Intent>()?, intent)?;
        let evidence = evidence
            .map(|evidence| {
                qualify_erased(assembly, nominal_contract_ref::<C::Evidence>()?, evidence)
            })
            .transpose()?;
        let outcome = outcome
            .map(|outcome| match outcome {
                mfm_capabilities::ProposedStateOutcome::Success { output, facts } => {
                    facts.validate().map_err(|_| RuntimeError::Value)?;
                    Ok::<DynamicOutcome, RuntimeError>(DynamicOutcome::Success {
                        value: qualify_erased(
                            assembly,
                            nominal_contract_ref::<S::Output>()?,
                            output,
                        )?,
                        facts,
                    })
                }
                mfm_capabilities::ProposedStateOutcome::Failure { failure } => {
                    Ok::<DynamicOutcome, RuntimeError>(DynamicOutcome::Failure(qualify_erased(
                        assembly,
                        nominal_contract_ref::<S::Failure>()?,
                        failure,
                    )?))
                }
            })
            .transpose()?;
        Ok(Self {
            input,
            evidence,
            outcome,
            classification,
            fact_continuation,
        })
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
        if !assembly.program().belongs_to_catalog(store.catalog()) {
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
        value: QualifiedTypedValue<T>,
        configuration: ResolvedConfigurationHead,
        source_refs: Vec<ContentRef>,
    ) -> LifecycleResult<AdmissionInput<T>> {
        AdmissionInput::new(self, run_id, value, configuration, source_refs)
    }

    /// Resumes one exact run after cold callback-free prefix qualification.
    pub async fn resume<T: MfmValue>(&self, input: ResumeInput<T>) -> ResumeStep {
        if !Arc::ptr_eq(&self.inner, &input.runtime.inner) {
            return ResumeStep::Failed(ResumeFailure::Identity);
        }
        let selected = match self.inner.store.select(&input.run_id).await {
            Ok(selected) => selected,
            Err(_) => return ResumeStep::Failed(ResumeFailure::History),
        };
        match self
            .session_from_selected(selected, Some(input.value))
            .await
        {
            Ok(session) => self.classify_resume_session(session).await,
            Err(SessionBuildFailure {
                error: RuntimeError::Capacity,
                ..
            }) => ResumeStep::Failed(ResumeFailure::Capacity),
            Err(_) => ResumeStep::Failed(ResumeFailure::Identity),
        }
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

    /// Returns the immutable entry-point identity of this Runtime's Program.
    pub fn entry_point_id(&self) -> StableId {
        self.inner
            .assembly
            .program()
            .document()
            .entry_point_id()
            .clone()
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
        occurrence: mfm_journal::single_trust::SequentialControlAddress,
        call: Box<dyn DynamicCall>,
        selected: SelectedRun,
    ) -> RuntimeStep {
        let _state = match self
            .inner
            .assembly
            .program()
            .document()
            .declaration(&occurrence)
        {
            Some(mfm_program::Declaration::State(state)) => state,
            _ => {
                return RuntimeStep::Failed {
                    history: selected.into_qualified_run(),
                    error: RuntimeError::Identity,
                }
            }
        };
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
            return match self
                .session_from_selected(selected, Some(resolution.input))
                .await
            {
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
            .admit(
                run_id,
                self.inner.assembly.program(),
                &value,
                configuration,
                source_refs,
            )
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
        if selected.program_ref() != self.inner.assembly.program().program_ref().content_ref() {
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
        let state = match runtime
            .inner
            .assembly
            .program()
            .document()
            .declaration(&occurrence)
        {
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
            registration.prepare_access(&assembly, run_id, prepare_occurrence, latest)
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
        let state = match runtime
            .inner
            .assembly
            .program()
            .document()
            .declaration(&occurrence)
        {
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
    value: QualifiedTypedValue<T>,
    configuration: ResolvedConfigurationHead,
    source_refs: Vec<ContentRef>,
}

impl<T: MfmValue> AdmissionInput<T> {
    /// Mints an admission owner from one catalog-qualified planning value.
    pub fn new(
        runtime: &Runtime,
        run_id: RunId,
        value: QualifiedTypedValue<T>,
        configuration: ResolvedConfigurationHead,
        source_refs: Vec<ContentRef>,
    ) -> LifecycleResult<Self> {
        if !value.belongs_to_catalog(runtime.inner.assembly.catalog())
            || value.contract_ref()
                != runtime
                    .inner
                    .assembly
                    .program()
                    .document()
                    .admitted_context_contract_ref()
        {
            return Err(RuntimeError::Identity);
        }
        Ok(Self {
            runtime: runtime.clone(),
            run_id,
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
                self.value,
                self.configuration,
                self.source_refs,
            )
            .await
    }
}

/// Typed cold-resume owner carrying the admitted value needed for exact catalog reification.
pub struct ResumeInput<T: MfmValue> {
    runtime: Runtime,
    run_id: RunId,
    value: ErasedValue,
    _marker: PhantomData<fn() -> T>,
}

impl<T: MfmValue> ResumeInput<T> {
    /// Creates a resume owner from a catalog-qualified admitted value.
    pub fn new(
        runtime: &Runtime,
        run_id: RunId,
        value: QualifiedTypedValue<T>,
    ) -> LifecycleResult<Self> {
        if !value.belongs_to_catalog(runtime.inner.assembly.catalog()) {
            return Err(RuntimeError::Identity);
        }
        Ok(Self {
            value: value.erase(),
            runtime: runtime.clone(),
            run_id,
            _marker: PhantomData,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mfm_canonical::raw_content_digest;
    use mfm_capabilities::{AccessCapabilityContract, EffectMode, NoPriorFacts, ReadMode};
    use mfm_program::single_trust::{
        BindingDescriptor, ExecutionMode, ProgramDocument, StateDeclaration,
    };
    use mfm_program_derive::{MfmConfig as DeriveMfmConfig, MfmValue as DeriveMfmValue};
    use mfm_store::{
        BackendAppendCommand, BackendAppendOutcome, BackendConfigurationOutcome, BackendFuture,
        ConfigurationAppendCommand, ConfigurationCommitOutcome, MemoryStructuredBackend,
        RawConfigurationRevision, RawFactSnapshot, RawHistoryLoadLimit, RawRunPrefix,
        StoreWorkLimits, StructuredStore, StructuredStoreBackend, StructuredStoreIdentity,
    };
    use mfm_values::ValidatedConfig;
    use serde::{Deserialize, Serialize};
    use std::num::NonZeroU16;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn runtime_limits_require_one_permit_per_work_lane() {
        assert!(RuntimeLimits::default().validate().is_ok());
        assert_eq!(RuntimeLimits::default().max_cpu_jobs(), 8);
        assert!(RuntimeLimits::new(0, 1, 1, 1).validate().is_err());
        assert!(RuntimeLimits::new(1, 0, 1, 1).validate().is_err());
        assert!(RuntimeLimits::new(1, 1, 0, 1).validate().is_err());
        assert!(RuntimeLimits::new(1, 1, 1, 0).validate().is_err());
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, DeriveMfmValue)]
    #[serde(deny_unknown_fields)]
    struct TestContext {
        value: u64,
    }

    #[derive(Debug, Serialize, Deserialize, DeriveMfmConfig)]
    #[serde(deny_unknown_fields)]
    struct TestConfig {
        value: u64,
    }

    impl crate::single_trust::FailureValue for TestContext {
        fn integrity_blocked() -> Self {
            Self { value: 0 }
        }
    }

    struct TestPure;

    impl State for TestPure {
        type Input = TestContext;
        type Output = TestContext;
        type Failure = TestContext;

        fn state_id() -> LifecycleResult<StableId> {
            StableId::new("mfm.test.lifecycle-pure").map_err(|_| RuntimeError::Identity)
        }
    }

    struct TestAccess;

    impl State for TestAccess {
        type Input = TestContext;
        type Output = TestContext;
        type Failure = TestContext;

        fn state_id() -> LifecycleResult<StableId> {
            StableId::new("mfm.test.lifecycle-access").map_err(|_| RuntimeError::Identity)
        }
    }

    struct TestRead;

    impl AccessCapabilityContract for TestRead {
        type Mode = ReadMode;
        type Intent = TestContext;
        type Evidence = TestContext;
        type Facts = NoPriorFacts;

        fn contract_id() -> mfm_capabilities::Result<StableId> {
            StableId::new("mfm.test.lifecycle-read")
                .map_err(|_| mfm_capabilities::CapabilityError::InvalidContract)
        }

        fn total_attempt_bound() -> NonZeroU16 {
            NonZeroU16::new(1).expect("nonzero")
        }

        fn bind_evidence(
            intent: &Self::Intent,
            evidence: &Self::Evidence,
        ) -> mfm_capabilities::Result<()> {
            (evidence.value == intent.value + 1)
                .then_some(())
                .ok_or(mfm_capabilities::CapabilityError::EvidenceBinding)
        }
    }

    struct TestEffect;

    impl AccessCapabilityContract for TestEffect {
        type Mode = EffectMode;
        type Intent = TestContext;
        type Evidence = TestContext;
        type Facts = NoPriorFacts;

        fn contract_id() -> mfm_capabilities::Result<StableId> {
            StableId::new("mfm.test.lifecycle-effect")
                .map_err(|_| mfm_capabilities::CapabilityError::InvalidContract)
        }

        fn total_attempt_bound() -> NonZeroU16 {
            NonZeroU16::new(1).expect("nonzero")
        }

        fn bind_evidence(
            intent: &Self::Intent,
            evidence: &Self::Evidence,
        ) -> mfm_capabilities::Result<()> {
            (evidence.value == intent.value + 1)
                .then_some(())
                .ok_or(mfm_capabilities::CapabilityError::EvidenceBinding)
        }
    }

    fn test_ref(label: &[u8]) -> ContentRef {
        let schema = TestContext::schema_id().expect("schema");
        ContentRef::new(schema, raw_content_digest(label)).expect("content ref")
    }

    fn test_catalog_builder() -> mfm_program::ProgramCatalogBuilder {
        let mut builder = ProgramCatalog::builder();
        builder
            .register_value::<TestContext>()
            .expect("test context");
        builder
            .register_capability::<TestRead>()
            .expect("test read capability");
        builder
            .register_capability::<TestEffect>()
            .expect("test effect capability");
        builder
    }

    fn test_identity() -> StructuredStoreIdentity {
        StructuredStoreIdentity::new(
            mfm_ids::StoreScopeId::new("mfm.store_scope.v1:0123456789abcdef0123456789abcdef")
                .expect("scope"),
            mfm_ids::StoreEpoch::new(1),
            mfm_ids::TenantScopeId::new("mfm.tenant_scope.v1:0123456789abcdef0123456789abcdef")
                .expect("tenant"),
        )
    }

    fn pure_runtime_assembly(
        pure_entries: Option<Arc<AtomicUsize>>,
    ) -> (
        crate::single_trust::RuntimeAssembly,
        ProgramCatalog,
        ContentRef,
    ) {
        let contract = nominal_contract_ref::<TestContext>().expect("contract");
        let first_implementation_ref = test_ref(b"mfm.test.concurrent-pure-first");
        let second_implementation_ref = test_ref(b"mfm.test.concurrent-pure-second");
        let first_occurrence =
            mfm_journal::single_trust::SequentialControlAddress::new(0, Vec::new())
                .expect("occurrence");
        let second_occurrence =
            mfm_journal::single_trust::SequentialControlAddress::new(1, Vec::new())
                .expect("occurrence");
        let document = ProgramDocument::new(
            StableId::new("mfm.test.concurrent-entry").expect("entry"),
            contract.clone(),
            contract.clone(),
            vec![
                mfm_program::Declaration::State(Box::new(
                    StateDeclaration::with_next(
                        first_occurrence,
                        first_implementation_ref.clone(),
                        contract.clone(),
                        contract.clone(),
                        None,
                        ExecutionMode::Pure,
                        second_occurrence.clone(),
                    )
                    .expect("state"),
                )),
                mfm_program::Declaration::State(Box::new(
                    StateDeclaration::new(
                        second_occurrence,
                        second_implementation_ref.clone(),
                        contract.clone(),
                        contract,
                        None,
                        ExecutionMode::Pure,
                        true,
                    )
                    .expect("state"),
                )),
            ],
        )
        .expect("document");
        let (catalog, program) = test_catalog_builder().finish(document).expect("program");
        let implementation = PureImplementation::<TestPure>::new(move |input| {
            if let Some(pure_entries) = &pure_entries {
                pure_entries.fetch_add(1, Ordering::SeqCst);
            }
            mfm_capabilities::ProposedStateOutcome::Success {
                output: TestContext {
                    value: input.value + 1,
                },
                facts: mfm_facts::FactProposalSet::empty(),
            }
        });
        let mut builder =
            crate::single_trust::RuntimeAssemblyBuilder::new(catalog.clone(), program)
                .expect("assembly builder");
        builder
            .register_pure(first_implementation_ref, implementation.clone())
            .expect("registration");
        builder
            .register_pure(second_implementation_ref, implementation)
            .expect("registration");
        let assembly = builder.finish().expect("assembly");
        (
            assembly,
            catalog,
            nominal_contract_ref::<TestContext>().expect("contract"),
        )
    }

    fn runtime_with_ports(
        assembly: RuntimeAssembly,
        store: mfm_store::OpenedStructuredStore,
    ) -> (
        Runtime,
        mfm_store::ConfigurationStore,
        mfm_store::HistoryReader,
    ) {
        let (history, reader, configuration, _audit) = store.split().into_parts();
        (
            Runtime::new(assembly, history).expect("runtime"),
            configuration,
            reader,
        )
    }

    fn pure_runtime_fixture() -> (
        Runtime,
        mfm_store::ConfigurationStore,
        mfm_store::HistoryReader,
        ProgramCatalog,
        ContentRef,
    ) {
        let (assembly, catalog, contract) = pure_runtime_assembly(None);
        let store = StructuredStore::open_memory(
            test_identity(),
            catalog.clone(),
            StoreWorkLimits::default(),
        )
        .expect("store");
        let (runtime, configuration, reader) = runtime_with_ports(assembly, store);
        (runtime, configuration, reader, catalog, contract)
    }

    async fn test_configuration(
        configuration: &mfm_store::ConfigurationStore,
    ) -> ResolvedConfigurationHead {
        let owner = configuration
            .initial_write_session::<TestConfig>()
            .prepare_local(
                AppendRequestId::new("runtime-test-configuration-000001").expect("request"),
                ValidatedConfig::new(TestConfig { value: 1 }).expect("config"),
            )
            .expect("configuration owner");
        match configuration
            .commit(owner)
            .await
            .expect("configuration commit")
        {
            ConfigurationCommitOutcome::NewlyCommitted(resolved)
            | ConfigurationCommitOutcome::Found(resolved) => resolved.into_head(),
            other => panic!("unexpected configuration outcome: {other:?}"),
        }
    }

    #[tokio::test]
    async fn resume_rejects_a_run_admitted_under_a_different_current_program() {
        let contract = nominal_contract_ref::<TestContext>().expect("contract");
        let admitted_document = ProgramDocument::new(
            StableId::new("mfm.test.retained-program-a").expect("entry"),
            contract.clone(),
            contract.clone(),
            Vec::new(),
        )
        .expect("admitted document");
        let current_document = ProgramDocument::new(
            StableId::new("mfm.test.retained-program-b").expect("entry"),
            contract.clone(),
            contract.clone(),
            Vec::new(),
        )
        .expect("current document");
        let (catalog, admitted_program) = test_catalog_builder()
            .finish(admitted_document)
            .expect("catalog");
        let current_program = catalog.program(current_document).expect("current program");
        let admitted_assembly =
            crate::single_trust::RuntimeAssemblyBuilder::new(catalog.clone(), admitted_program)
                .expect("admitted assembly")
                .finish()
                .expect("admitted assembly finish");
        let current_assembly =
            crate::single_trust::RuntimeAssemblyBuilder::new(catalog.clone(), current_program)
                .expect("current assembly")
                .finish()
                .expect("current assembly finish");

        let identity = test_identity();
        let backend = Arc::new(MemoryStructuredBackend::new(identity.clone()));
        let admitted_store = StructuredStore::open(
            backend.clone(),
            identity.clone(),
            catalog.clone(),
            StoreWorkLimits::default(),
        )
        .await
        .expect("admitted store");
        let current_store = StructuredStore::open(
            backend,
            identity,
            catalog.clone(),
            StoreWorkLimits::default(),
        )
        .await
        .expect("current store");
        let (admitted_history, _, configuration, _) = admitted_store.split().into_parts();
        let (current_history, _, _, _) = current_store.split().into_parts();
        let admitted_runtime = Runtime::new(admitted_assembly, admitted_history).expect("runtime");
        let current_runtime = Runtime::new(current_assembly, current_history).expect("runtime");
        let configuration = test_configuration(&configuration).await;
        let run_id = RunId::parse(
            "run:sha256-jcs-v1:9123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        )
        .expect("run id");
        let value = catalog
            .qualify(contract, TestContext { value: 1 })
            .expect("qualified input");
        assert!(matches!(
            admitted_runtime
                .admission(run_id.clone(), value, configuration, Vec::new())
                .expect("admission")
                .spawn()
                .await,
            SpawnStep::Terminal(_)
        ));
        assert!(matches!(
            current_runtime.resume_run(run_id).await,
            ResumeStep::Failed(ResumeFailure::Identity)
        ));
    }

    struct UnknownConclusionBackend {
        inner: Arc<MemoryStructuredBackend>,
        injected: AtomicUsize,
    }

    impl UnknownConclusionBackend {
        fn new(inner: Arc<MemoryStructuredBackend>) -> Self {
            Self {
                inner,
                injected: AtomicUsize::new(0),
            }
        }
    }

    impl StructuredStoreBackend for UnknownConclusionBackend {
        fn identity(&self) -> StructuredStoreIdentity {
            self.inner.identity()
        }

        fn load_complete_prefix<'a>(
            &'a self,
            run_id: &'a RunId,
            limit: RawHistoryLoadLimit,
        ) -> BackendFuture<'a, Option<RawRunPrefix>> {
            self.inner.load_complete_prefix(run_id, limit)
        }

        fn compare_and_append<'a>(
            &'a self,
            command: &'a BackendAppendCommand<'a>,
        ) -> BackendFuture<'a, BackendAppendOutcome> {
            let inner = Arc::clone(&self.inner);
            let injected = &self.injected;
            Box::pin(async move {
                let outcome = inner.compare_and_append(command).await?;
                if !command.is_admission()
                    && matches!(outcome, BackendAppendOutcome::NewlyCommitted)
                    && injected.fetch_add(1, Ordering::SeqCst) == 0
                {
                    return Ok(BackendAppendOutcome::AcknowledgementUnknown);
                }
                Ok(outcome)
            })
        }

        fn load_configuration<'a>(&'a self) -> BackendFuture<'a, Vec<RawConfigurationRevision>> {
            self.inner.load_configuration()
        }

        fn compare_and_append_configuration<'a>(
            &'a self,
            command: &'a ConfigurationAppendCommand<'a>,
        ) -> BackendFuture<'a, BackendConfigurationOutcome> {
            self.inner.compare_and_append_configuration(command)
        }

        fn load_facts<'a>(&'a self) -> BackendFuture<'a, RawFactSnapshot> {
            self.inner.load_facts()
        }

        fn audit_run_ids<'a>(&'a self) -> BackendFuture<'a, Vec<RunId>> {
            self.inner.audit_run_ids()
        }
    }

    #[derive(Default)]
    struct AccessCounters {
        provider_entries: AtomicUsize,
        ingress: AtomicUsize,
        preparations: AtomicUsize,
        interpretations: AtomicUsize,
    }

    #[cfg(target_os = "linux")]
    fn process_vm_hwm_bytes() -> Option<usize> {
        std::fs::read_to_string("/proc/self/status")
            .ok()?
            .lines()
            .find_map(|line| line.strip_prefix("VmHWM:")?.split_whitespace().next())
            .and_then(|kilobytes| kilobytes.parse::<usize>().ok())
            .and_then(|kilobytes| kilobytes.checked_mul(1024))
    }

    #[cfg(not(target_os = "linux"))]
    const fn process_vm_hwm_bytes() -> Option<usize> {
        None
    }

    #[tokio::test]
    async fn pure_session_advances_through_runtime_and_store() {
        let contract = nominal_contract_ref::<TestContext>().expect("contract");
        let implementation_ref = test_ref(b"mfm.test.lifecycle-pure-implementation");
        let second_implementation_ref = test_ref(b"mfm.test.lifecycle-pure-second-implementation");
        let first_occurrence =
            mfm_journal::single_trust::SequentialControlAddress::new(0, Vec::new())
                .expect("occurrence");
        let second_occurrence =
            mfm_journal::single_trust::SequentialControlAddress::new(1, Vec::new())
                .expect("occurrence");
        let document = ProgramDocument::new(
            StableId::new("mfm.test.lifecycle-entry").expect("entry"),
            contract.clone(),
            contract.clone(),
            vec![
                mfm_program::Declaration::State(Box::new(
                    StateDeclaration::with_next(
                        first_occurrence,
                        implementation_ref.clone(),
                        contract.clone(),
                        contract.clone(),
                        None,
                        ExecutionMode::Pure,
                        second_occurrence.clone(),
                    )
                    .expect("state"),
                )),
                mfm_program::Declaration::State(Box::new(
                    StateDeclaration::new(
                        second_occurrence,
                        second_implementation_ref.clone(),
                        contract.clone(),
                        contract.clone(),
                        None,
                        ExecutionMode::Pure,
                        true,
                    )
                    .expect("state"),
                )),
            ],
        )
        .expect("document");
        let (catalog, program) = test_catalog_builder().finish(document).expect("program");
        let mut builder =
            crate::single_trust::RuntimeAssemblyBuilder::new(catalog.clone(), program)
                .expect("assembly builder");
        let pure_entries = Arc::new(AtomicUsize::new(0));
        let pure_entries_for_state = Arc::clone(&pure_entries);
        let pure_implementation = PureImplementation::<TestPure>::new(move |input| {
            pure_entries_for_state.fetch_add(1, Ordering::SeqCst);
            mfm_capabilities::ProposedStateOutcome::Success {
                output: TestContext {
                    value: input.value + 1,
                },
                facts: mfm_facts::FactProposalSet::empty(),
            }
        });
        builder
            .register_pure(implementation_ref, pure_implementation.clone())
            .expect("registration");
        builder
            .register_pure(second_implementation_ref, pure_implementation)
            .expect("registration");
        let assembly = builder.finish().expect("assembly");
        let store = StructuredStore::open_memory(
            test_identity(),
            catalog.clone(),
            StoreWorkLimits::default(),
        )
        .expect("store");
        let (runtime, configuration_store, reader) = runtime_with_ports(assembly, store);
        let value = catalog
            .qualify(contract, TestContext { value: 1 })
            .expect("qualified input");
        let run_id = RunId::parse(
            "run:sha256-jcs-v1:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        )
        .expect("run id");
        let configuration = test_configuration(&configuration_store).await;
        let admission = runtime
            .admission(run_id, value, configuration, Vec::new())
            .expect("admission");
        let session = match admission.spawn().await {
            SpawnStep::Active(session) => session,
            other => panic!("unexpected spawn outcome: {}", spawn_name(&other)),
        };
        let advanced = match session.drive().await {
            RuntimeStep::Advanced(session) => session,
            RuntimeStep::Failed { error, .. } => panic!("unexpected drive failure: {error:?}"),
            other => panic!("unexpected drive outcome: {}", runtime_name(&other)),
        };
        assert_eq!(advanced.head_sequence(), 2);
        let run_id = advanced.run_id().clone();
        let hot_prefix = reader.load(&run_id).await.expect("hot prefix");
        let hot_frame_bytes: usize = hot_prefix
            .frames()
            .iter()
            .map(|frame| {
                frame
                    .canonical_bytes()
                    .expect("frame bytes")
                    .as_bytes()
                    .len()
            })
            .sum();
        let hot_context_bytes = advanced.latest.canonical_bytes().len();
        let hot_process_vm_hwm_bytes = process_vm_hwm_bytes();
        eprintln!(
            "capacity-envelope runtime pure hot_head={} hot_frame_bytes={} hot_context_bytes={} process_vm_hwm_bytes={:?} executor=retained-session",
            hot_prefix.head_sequence(),
            hot_frame_bytes,
            hot_context_bytes,
            hot_process_vm_hwm_bytes,
        );
        drop(advanced);
        let cold = match runtime.resume_run(run_id).await {
            ResumeStep::Active(session) => session,
            other => panic!("unexpected cold resume outcome: {}", resume_name(&other)),
        };
        let cold_context_bytes = cold.latest.canonical_bytes().len();
        let terminal = match cold.drive().await {
            RuntimeStep::Terminal(terminal) => terminal,
            RuntimeStep::Failed { error, .. } => panic!("unexpected cold drive failure: {error:?}"),
            other => panic!("unexpected cold drive outcome: {}", runtime_name(&other)),
        };
        assert_eq!(terminal.head_sequence(), 3);
        eprintln!(
            "capacity-envelope runtime pure cold_resume_head={} cold_frame_bytes={} cold_context_bytes={} process_vm_hwm_bytes={:?} executor=one-shot-resume-drive",
            terminal.head_sequence(),
            terminal
                .qualified_run()
                .frames()
                .iter()
                .map(|frame| frame
                    .canonical_bytes()
                    .expect("frame bytes")
                    .as_bytes()
                    .len())
            .sum::<usize>(),
            cold_context_bytes,
            process_vm_hwm_bytes(),
        );
        assert_eq!(pure_entries.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn unknown_pure_conclusion_recovery_does_not_reexecute_state() {
        let pure_entries = Arc::new(AtomicUsize::new(0));
        let (assembly, catalog, contract) = pure_runtime_assembly(Some(Arc::clone(&pure_entries)));
        let identity = test_identity();
        let backend = Arc::new(UnknownConclusionBackend::new(Arc::new(
            MemoryStructuredBackend::new(identity.clone()),
        )));
        let store = StructuredStore::open(
            backend,
            identity,
            catalog.clone(),
            StoreWorkLimits::default(),
        )
        .await
        .expect("store");
        let (runtime, configuration_store, _reader) = runtime_with_ports(assembly, store);
        let run_id = RunId::parse(
            "run:sha256-jcs-v1:4123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        )
        .expect("run id");
        let value = catalog
            .qualify(contract, TestContext { value: 1 })
            .expect("qualified input");
        let configuration = test_configuration(&configuration_store).await;
        let admission = runtime
            .admission(run_id, value, configuration, Vec::new())
            .expect("admission");
        let session = match admission.spawn().await {
            SpawnStep::Active(session) => session,
            other => panic!("unexpected spawn outcome: {}", spawn_name(&other)),
        };
        let suspended = match session.drive().await {
            RuntimeStep::Suspended(suspended) => suspended,
            other => panic!("unexpected first drive outcome: {}", runtime_name(&other)),
        };
        assert_eq!(pure_entries.load(Ordering::SeqCst), 1);
        let advanced = match suspended.resolve().await {
            RuntimeStep::Advanced(session) => session,
            other => panic!("unexpected conclusion recovery: {}", runtime_name(&other)),
        };
        assert_eq!(advanced.head_sequence(), 2);
        assert_eq!(pure_entries.load(Ordering::SeqCst), 1);
        let terminal = match advanced.drive().await {
            RuntimeStep::Terminal(terminal) => terminal,
            other => panic!(
                "unexpected terminal drive outcome: {}",
                runtime_name(&other)
            ),
        };
        assert_eq!(terminal.head_sequence(), 3);
        assert_eq!(pure_entries.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn concurrent_spawn_and_resume_share_cas_outcomes_without_duplicate_pure_entries() {
        let (runtime, configuration_store, _reader, catalog, contract) = pure_runtime_fixture();
        let run_id = RunId::parse(
            "run:sha256-jcs-v1:2123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        )
        .expect("run id");
        let configuration = test_configuration(&configuration_store).await;
        let value = |amount| {
            catalog
                .qualify(contract.clone(), TestContext { value: amount })
                .expect("qualified value")
        };
        let left = runtime
            .admission(run_id.clone(), value(1), configuration.clone(), Vec::new())
            .expect("admission");
        let right = runtime
            .admission(run_id.clone(), value(1), configuration.clone(), Vec::new())
            .expect("admission");
        let (left, right) = tokio::join!(left.spawn(), right.spawn());
        assert!(matches!(
            left,
            SpawnStep::Active(_) | SpawnStep::Terminal(_)
        ));
        assert!(matches!(
            right,
            SpawnStep::Active(_) | SpawnStep::Terminal(_)
        ));

        let value = value(1);
        let seed = runtime
            .admission(run_id.clone(), value, configuration, Vec::new())
            .expect("same admission");
        drop(seed.spawn().await);

        let (left, right) = tokio::join!(
            runtime.resume_run(run_id.clone()),
            runtime.resume_run(run_id)
        );
        let left = match left {
            ResumeStep::Active(session) => session,
            other => panic!("unexpected left resume: {}", resume_name(&other)),
        };
        let right = match right {
            ResumeStep::Active(session) => session,
            other => panic!("unexpected right resume: {}", resume_name(&other)),
        };
        let (left, right) = tokio::join!(left.drive(), right.drive());
        assert!(matches!(
            left,
            RuntimeStep::Advanced(_) | RuntimeStep::Terminal(_)
        ));
        assert!(matches!(
            right,
            RuntimeStep::Advanced(_) | RuntimeStep::Terminal(_)
        ));
    }

    #[tokio::test]
    async fn access_session_enters_only_the_bound_adapter_and_concludes() {
        let contract = nominal_contract_ref::<TestContext>().expect("contract");
        let capability_contract =
            mfm_program::capability_contract_ref::<TestRead>().expect("capability");
        let implementation_ref = test_ref(b"mfm.test.lifecycle-access-implementation");
        let adapter_ref = test_ref(b"mfm.test.lifecycle-access-adapter");
        let physical_target_ref = test_ref(b"mfm.test.lifecycle-access-target");
        let occurrence = mfm_journal::single_trust::SequentialControlAddress::new(0, Vec::new())
            .expect("occurrence");
        let binding = BindingDescriptor::new(
            implementation_ref.clone(),
            Some(capability_contract.clone()),
            Some(adapter_ref.clone()),
            physical_target_ref,
            None,
            None,
        )
        .expect("binding");
        let document = ProgramDocument::new(
            StableId::new("mfm.test.lifecycle-access-entry").expect("entry"),
            contract.clone(),
            contract.clone(),
            vec![mfm_program::Declaration::State(Box::new(
                StateDeclaration::new(
                    occurrence,
                    implementation_ref.clone(),
                    contract.clone(),
                    contract.clone(),
                    None,
                    ExecutionMode::Read {
                        capability_contract_ref: capability_contract.clone(),
                        total_attempt_bound: 1,
                        fact_selection_required: false,
                    },
                    true,
                )
                .expect("state")
                .with_execution_binding(binding)
                .expect("execution binding"),
            ))],
        )
        .expect("document");
        let (catalog, program) = test_catalog_builder().finish(document).expect("program");
        let counters = Arc::new(AccessCounters::default());
        let mut builder =
            crate::single_trust::RuntimeAssemblyBuilder::new(catalog.clone(), program)
                .expect("assembly builder");
        let counters_for_registration = Arc::clone(&counters);
        let counters_for_state = Arc::clone(&counters);
        let counters_for_preparation = Arc::clone(&counters);
        builder
            .register_access::<TestAccess, TestRead, _>(
                implementation_ref,
                AccessImplementation::new(
                    move |input: &TestContext| {
                        counters_for_preparation
                            .preparations
                            .fetch_add(1, Ordering::SeqCst);
                        Ok(*input)
                    },
                    move |call: CommittedCall<TestAccess, TestRead>| {
                        let counters = Arc::clone(&counters_for_state);
                        Box::pin(async move {
                            match call.invoke_bound_adapter().await? {
                                crate::single_trust::AccessResolution::Outcome(accepted) => {
                                    counters.interpretations.fetch_add(1, Ordering::SeqCst);
                                    let output = TestContext {
                                        value: accepted.evidence().value,
                                    };
                                    Ok(accepted.conclude(
                                        mfm_capabilities::ProposedStateOutcome::Success {
                                            output,
                                            facts: mfm_facts::FactProposalSet::empty(),
                                        },
                                    ))
                                }
                                crate::single_trust::AccessResolution::BlockedIntegrity(
                                    accepted,
                                ) => Ok(accepted.conclude_blocked()),
                                crate::single_trust::AccessResolution::Unresolved(unresolved) => {
                                    Ok(unresolved.finish())
                                }
                            }
                        })
                    },
                ),
                move |call| {
                    counters_for_registration
                        .provider_entries
                        .fetch_add(1, Ordering::SeqCst);
                    let counters = Arc::clone(&counters_for_registration);
                    Box::pin(async move {
                        counters.ingress.fetch_add(1, Ordering::SeqCst);
                        let evidence = TestContext {
                            value: call.intent().value + 1,
                        };
                        call.accept_evidence(evidence)
                            .map(crate::single_trust::AccessResolution::Outcome)
                    })
                },
            )
            .expect("registration");
        let assembly = builder.finish().expect("assembly");
        let store = StructuredStore::open_memory(
            test_identity(),
            catalog.clone(),
            StoreWorkLimits::default(),
        )
        .expect("store");
        let (runtime, configuration_store, _reader) = runtime_with_ports(assembly, store);
        let value = catalog
            .qualify(contract, TestContext { value: 4 })
            .expect("qualified input");
        let run_id = RunId::parse(
            "run:sha256-jcs-v1:1123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        )
        .expect("run id");
        let configuration = test_configuration(&configuration_store).await;
        let admission = runtime
            .admission(run_id.clone(), value, configuration, Vec::new())
            .expect("admission");
        let session = match admission.spawn().await {
            SpawnStep::Active(session) => session,
            other => panic!("unexpected spawn outcome: {}", spawn_name(&other)),
        };
        drop(session);
        let (left, right) = tokio::join!(
            runtime.resume_run(run_id.clone()),
            runtime.resume_run(run_id)
        );
        let left = match left {
            ResumeStep::Active(session) => session,
            other => panic!("unexpected left resume: {}", resume_name(&other)),
        };
        let right = match right {
            ResumeStep::Active(session) => session,
            other => panic!("unexpected right resume: {}", resume_name(&other)),
        };
        let (left, right) = tokio::join!(left.drive(), right.drive());
        let suspended = match (left, right) {
            (RuntimeStep::Terminal(terminal), RuntimeStep::Suspended(suspended))
            | (RuntimeStep::Suspended(suspended), RuntimeStep::Terminal(terminal)) => {
                assert_eq!(terminal.head_sequence(), 3);
                suspended
            }
            (left, right) => panic!(
                "expected one terminal and one suspended preparation, got {} and {}",
                runtime_name(&left),
                runtime_name(&right)
            ),
        };
        match suspended.resolve().await {
            RuntimeStep::Terminal(terminal) => assert_eq!(terminal.head_sequence(), 3),
            other => panic!(
                "unexpected preparation resolution: {}",
                runtime_name(&other)
            ),
        }
        assert_eq!(counters.provider_entries.load(Ordering::SeqCst), 1);
        assert_eq!(counters.ingress.load(Ordering::SeqCst), 1);
        assert_eq!(counters.preparations.load(Ordering::SeqCst), 2);
        assert_eq!(counters.interpretations.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn unresolved_effect_parks_without_another_preparation_or_provider_entry() {
        let contract = nominal_contract_ref::<TestContext>().expect("contract");
        let capability_contract =
            mfm_program::capability_contract_ref::<TestEffect>().expect("capability");
        let implementation_ref = test_ref(b"mfm.test.lifecycle-effect-implementation");
        let adapter_ref = test_ref(b"mfm.test.lifecycle-effect-adapter");
        let physical_target_ref = test_ref(b"mfm.test.lifecycle-effect-target");
        let effect_domain = StableId::new("mfm.test.lifecycle-effect-domain").expect("domain");
        let occurrence = mfm_journal::single_trust::SequentialControlAddress::new(0, Vec::new())
            .expect("occurrence");
        let binding = BindingDescriptor::new(
            implementation_ref.clone(),
            Some(capability_contract.clone()),
            Some(adapter_ref.clone()),
            physical_target_ref,
            Some(effect_domain.clone()),
            None,
        )
        .expect("binding");
        let document = ProgramDocument::new(
            StableId::new("mfm.test.lifecycle-effect-entry").expect("entry"),
            contract.clone(),
            contract.clone(),
            vec![mfm_program::Declaration::State(Box::new(
                StateDeclaration::new(
                    occurrence,
                    implementation_ref.clone(),
                    contract.clone(),
                    contract.clone(),
                    None,
                    ExecutionMode::Effect {
                        capability_contract_ref: capability_contract.clone(),
                        effect_domain,
                        fact_selection_required: false,
                    },
                    true,
                )
                .expect("state")
                .with_execution_binding(binding)
                .expect("execution binding"),
            ))],
        )
        .expect("document");
        let (catalog, program) = test_catalog_builder().finish(document).expect("program");
        let counters = Arc::new(AccessCounters::default());
        let mut builder =
            crate::single_trust::RuntimeAssemblyBuilder::new(catalog.clone(), program)
                .expect("assembly builder");
        let preparation_counters = Arc::clone(&counters);
        let provider_counters = Arc::clone(&counters);
        builder
            .register_access::<TestAccess, TestEffect, _>(
                implementation_ref,
                AccessImplementation::new(
                    move |input: &TestContext| {
                        preparation_counters
                            .preparations
                            .fetch_add(1, Ordering::SeqCst);
                        Ok(*input)
                    },
                    move |call: CommittedCall<TestAccess, TestEffect>| {
                        Box::pin(async move {
                            match call.invoke_bound_adapter().await? {
                                crate::single_trust::AccessResolution::Unresolved(unresolved) => {
                                    Ok(unresolved.finish())
                                }
                                crate::single_trust::AccessResolution::Outcome(_)
                                | crate::single_trust::AccessResolution::BlockedIntegrity(_) => {
                                    unreachable!("test adapter is always unresolved")
                                }
                            }
                        })
                    },
                ),
                move |call| {
                    provider_counters
                        .provider_entries
                        .fetch_add(1, Ordering::SeqCst);
                    Box::pin(async move {
                        Ok(crate::single_trust::AccessResolution::Unresolved(
                            call.unresolved(
                                crate::single_trust::UnresolvedClassification::AcknowledgementUnknown,
                            ),
                        ))
                    })
                },
            )
            .expect("registration");
        let assembly = builder.finish().expect("assembly");
        let store = StructuredStore::open_memory(
            test_identity(),
            catalog.clone(),
            StoreWorkLimits::default(),
        )
        .expect("store");
        let (runtime, configuration_store, _reader) = runtime_with_ports(assembly, store);
        let value = catalog
            .qualify(contract, TestContext { value: 4 })
            .expect("qualified input");
        let run_id = RunId::parse(
            "run:sha256-jcs-v1:5123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        )
        .expect("run id");
        let configuration = test_configuration(&configuration_store).await;
        let admission = runtime
            .admission(run_id.clone(), value, configuration, Vec::new())
            .expect("admission");
        let session = match admission.spawn().await {
            SpawnStep::Active(session) => session,
            other => panic!("unexpected spawn outcome: {}", spawn_name(&other)),
        };
        let session = match session.drive().await {
            RuntimeStep::Unresolved {
                session,
                classification:
                    crate::single_trust::UnresolvedClassification::AcknowledgementUnknown,
            } => session,
            other => panic!("unexpected drive outcome: {}", runtime_name(&other)),
        };
        assert_eq!(counters.preparations.load(Ordering::SeqCst), 1);
        assert_eq!(counters.provider_entries.load(Ordering::SeqCst), 1);
        assert!(matches!(
            session.drive().await,
            RuntimeStep::Parked {
                reason: ParkReason::WaitingPreparation,
                ..
            }
        ));
        assert!(matches!(
            runtime.resume_run(run_id).await,
            ResumeStep::Parked(_)
        ));
        assert_eq!(counters.preparations.load(Ordering::SeqCst), 1);
        assert_eq!(counters.provider_entries.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn finite_owner_fate_model_never_mints_a_call_from_a_non_new_append() {
        #[derive(Clone, Copy)]
        enum Operation {
            Admission,
            Pure,
            Preparation,
            Replacement,
            Conclusion,
            Fact,
            Match,
            Failure,
            Late,
        }

        #[derive(Clone, Copy, PartialEq, Eq)]
        enum AppendResult {
            NewlyCommitted,
            Found,
            Stale,
            AcknowledgementUnknown,
            Rejected,
        }

        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        enum Owner {
            Prepared,
            Call,
            Suspended,
            History,
        }

        let operations = [
            Operation::Admission,
            Operation::Pure,
            Operation::Preparation,
            Operation::Replacement,
            Operation::Conclusion,
            Operation::Fact,
            Operation::Match,
            Operation::Failure,
            Operation::Late,
        ];
        let outcomes = [
            AppendResult::NewlyCommitted,
            AppendResult::Found,
            AppendResult::Stale,
            AppendResult::AcknowledgementUnknown,
            AppendResult::Rejected,
        ];
        for operation in operations {
            for outcome in outcomes {
                let allows_call = matches!(operation, Operation::Preparation)
                    && matches!(outcome, AppendResult::NewlyCommitted);
                let (owner, calls) = if allows_call {
                    (Owner::Call, 1)
                } else {
                    match outcome {
                        AppendResult::NewlyCommitted
                        | AppendResult::Found
                        | AppendResult::Stale => (Owner::History, 0),
                        AppendResult::AcknowledgementUnknown | AppendResult::Rejected => {
                            (Owner::Suspended, 0)
                        }
                    }
                };
                assert_eq!(calls, usize::from(owner == Owner::Call));
                assert_ne!(owner, Owner::Prepared);
                assert!(calls <= 1);
            }
        }
    }

    fn spawn_name(step: &SpawnStep) -> &'static str {
        match step {
            SpawnStep::Active(_) => "active",
            SpawnStep::Terminal(_) => "terminal",
            SpawnStep::Suspended(_) => "suspended",
            SpawnStep::Conflict(_) => "conflict",
            SpawnStep::Failed(_) => "failed",
        }
    }

    fn runtime_name(step: &RuntimeStep) -> &'static str {
        match step {
            RuntimeStep::Advanced(_) => "advanced",
            RuntimeStep::Terminal(_) => "terminal",
            RuntimeStep::PreparationRejected { .. } => "preparation-rejected",
            RuntimeStep::Unresolved { .. } => "unresolved",
            RuntimeStep::Parked { .. } => "parked",
            RuntimeStep::Suspended(_) => "suspended",
            RuntimeStep::ConclusionRejected { .. } => "conclusion-rejected",
            RuntimeStep::AdmissionRejected(_) => "admission-rejected",
            RuntimeStep::Conflict { .. } => "conflict",
            RuntimeStep::Failed { .. } => "failed",
        }
    }

    fn resume_name(step: &ResumeStep) -> &'static str {
        match step {
            ResumeStep::Active(_) => "active",
            ResumeStep::Terminal(_) => "terminal",
            ResumeStep::Parked(_) => "parked",
            ResumeStep::Failed(_) => "failed",
        }
    }
}
