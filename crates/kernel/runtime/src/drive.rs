//! Stateless one-action runtime driver.

use std::collections::BTreeMap;
use std::sync::Arc;

use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_facts::FactSelectionRequest;
use mfm_ids::{ContentRef, EffectKey, NodeId, RequestDigest, StableId};
use mfm_journal::v2::{
    AuthorizationRef, AuthorizationScopeFields, CapabilityBindingRef, ClosureRef,
    FactSelectionScanContract, FrozenReadIntent, InputManifestRef, JournalHead, JournalPredecessor,
    NodePhase, NonDomainDisposition, NonDomainEntryStatus, NonDomainFailure, NonDomainFailureCode,
    ReadCapabilityBinding, RunPhase, TransitionBodyFields, TransitionRef, ValueRef,
};
use mfm_program::{
    CandidateCertificationError, CandidateCertificationErrorKind, ProposedValueMaterial,
    QualifiedAdmittedProgram, QualifiedAuthoredRequest, QualifiedCandidateStateCallbacks,
    QualifiedEffectEntry, QualifiedEvidenceVerdict, QualifiedProgramRegistry, QualifiedReadEntry,
    QualifiedSettlement, QualifiedStateEntry, VerifiedReadOutcome, VerifiedStateFrameMaterial,
    VerifiedTerminalEffectView, VerifiedValueMaterial,
};
use mfm_spec::{CertifiedNodeContract, CertifiedStateExecution, RetainedValueContract};
use mfm_store::{
    AppendOutcome, AppendRejection, AuthorizationMaterial, Drive, ExactResolution,
    ExistingRunAppendMaterial, FactScanBackend, FactScanFailureProvenance,
    FactSelectionAuthorizationOutcome, NewlyAppended, NodeTerminalOutcome, ObjectGraphProposal,
    ObservationMaterial, PreparedFrame, PreparedJournalAppend, ProducedObjectRoot,
    ReadObservationMaterial, RunAccessAuthority, RunHistoryWriter, SealedFactSelectionObservation,
    StoreError, StoreErrorInspection, TransitionMaterial, VerifiedNodeAccessHistory,
    VerifiedRunView, FACT_SELECTION_OPERATION_ID,
};

use crate::access_protocol::{
    AccessKind, Authorized, CommittedObservation, Ensure, FactSelection, PendingObservation,
    Prepared, PreparedAccess, Read,
};
use crate::append_id::append_request_id;
use crate::callback_material::{settlement_material, verified_callback_frame};
use crate::capability_registry::{
    EffectInvocationContext, EffectRequestContext, ErasedReadObservation, ErasedReadOutcome,
    ReadInvocationContext, RoutedReadRequest, RuntimeEffectInvoker, RuntimeReadInvoker,
};
use crate::decision::{
    scan_observations, select_action, AccessAction, DecisionInput, EvidenceScan, IntegrityBlock,
    LocalAction, ObservationVerdict, OccurrenceCandidate, SelectedAction,
};
use crate::materialization::{frame_readiness, FrameReadiness};
use crate::observation_material::{
    committed_read_observation, committed_terminal_observation, CommittedEffectResolver,
};
use crate::runtime_error::{map_store_error, non_domain_failure};
use crate::{DriveOutcome, DriveWaitReason, Result, RuntimeError};

/// Stateless interpreter over one authoritative run-journal store.
pub struct Runtime<B> {
    pub(crate) writer: RunHistoryWriter<B>,
    pub(crate) program_registry: Arc<QualifiedProgramRegistry>,
}

impl<B> Runtime<B> {
    /// Binds the sole history writer to the exact qualified program registry.
    pub fn new(
        writer: RunHistoryWriter<B>,
        program_registry: Arc<QualifiedProgramRegistry>,
    ) -> Self {
        Self {
            writer,
            program_registry,
        }
    }
}

enum SettlementAction {
    Read {
        frame: Box<PreparedFrame>,
        request_ref: ValueRef,
        observation_ref: mfm_journal::v2::ObservationRef,
        settlement: QualifiedSettlement,
        settlement_contract: mfm_spec::CertifiedSettlementContract,
    },
    Effect {
        node_id: NodeId,
        request_transition_ref: TransitionRef,
        observation_ref: mfm_journal::v2::ObservationRef,
        settlement: QualifiedSettlement,
        settlement_contract: mfm_spec::CertifiedSettlementContract,
    },
}

enum LocalActionMaterial {
    Pure {
        node_id: NodeId,
        frame: Box<PreparedFrame>,
        settlement: QualifiedSettlement,
        settlement_contract: mfm_spec::CertifiedSettlementContract,
    },
    EffectRequest {
        node_id: NodeId,
        frame: Box<PreparedFrame>,
        request_contract: Box<RetainedValueContract>,
        request: ProposedValueMaterial,
        effect_key: EffectKey,
        request_digest: RequestDigest,
        executor_binding_ref: CapabilityBindingRef,
    },
    DependencySkip {
        node_id: NodeId,
    },
}

pub(crate) struct ReadAccessAction {
    node_id: NodeId,
    frame: PreparedFrame,
    request_contract: RetainedValueContract,
    returned_contract: RetainedValueContract,
    safe_failure_contract: RetainedValueContract,
    result_encoder: QualifiedStateEntry,
    invoker: RuntimeReadInvoker,
    safe_failure_contract_ref: ContentRef,
    request: RoutedReadRequest,
    result_encoding_failure: NonDomainFailure,
}

pub(crate) struct FactSelectionAccessAction {
    node_id: NodeId,
    frame: PreparedFrame,
    request_contract: RetainedValueContract,
    routing_generation_ref: ContentRef,
    request: FactSelectionRequest,
    proposed: ProposedValueMaterial,
    store_unavailable_failure: NonDomainFailure,
    history_invalid_failure: NonDomainFailure,
    adapter_contract_failure: NonDomainFailure,
}

pub(crate) struct EnsureAccessAction {
    node_id: NodeId,
    intent: EffectIntent,
    invoker: RuntimeEffectInvoker,
    invocation_context: EffectInvocationContext,
    request: QualifiedAuthoredRequest,
}

impl AccessKind for Read {
    type PreparedInvocation = Box<ReadAccessAction>;
    type BoundaryReturn = mfm_store::NewlyAppendedAuthorization;
    type PendingMaterial = EncodedReadObservation;
}

impl AccessKind for Ensure {
    type PreparedInvocation = Box<EnsureAccessAction>;
    type BoundaryReturn = mfm_store::NewlyAppendedAuthorization;
    type PendingMaterial = crate::capability_registry::ErasedEnsureObservation;
}

impl AccessKind for FactSelection {
    type PreparedInvocation = Box<FactSelectionAccessAction>;
    type BoundaryReturn = mfm_store::FactScanPermit;
    type PendingMaterial = FactSelectionObservation;
}

#[derive(Clone)]
enum FactSelectionObservationOutcome {
    Returned(Box<SealedFactSelectionObservation>),
    NonDomainFailure(NonDomainFailure),
}

#[derive(Clone)]
pub(crate) struct FactSelectionObservation {
    authorization_ref: AuthorizationRef,
    outcome: FactSelectionObservationOutcome,
}

impl FactSelectionObservation {
    fn material(&self) -> ObservationMaterial {
        match &self.outcome {
            FactSelectionObservationOutcome::Returned(sealed) => {
                ObservationMaterial::FactSelection {
                    sealed: sealed.clone(),
                }
            }
            FactSelectionObservationOutcome::NonDomainFailure(failure) => {
                ObservationMaterial::FactSelectionFailure {
                    authorization_ref: self.authorization_ref.clone(),
                    failure: *failure,
                }
            }
        }
    }
}

enum DeferredAccessDerivation {
    Read {
        frame: Box<PreparedFrame>,
        baseline: Option<Box<FrozenReadBaseline>>,
        authorization_count: usize,
        is_fact_selection: bool,
    },
    Ensure {
        intent: Box<EffectIntent>,
        authorization_count: usize,
    },
}

impl DeferredAccessDerivation {
    const fn authorization_count(&self) -> usize {
        match self {
            Self::Read {
                authorization_count,
                ..
            }
            | Self::Ensure {
                authorization_count,
                ..
            } => *authorization_count,
        }
    }
}

pub(crate) struct EncodedReadObservation {
    authorization_ref: AuthorizationRef,
    outcome: EncodedReadOutcome,
}

enum EncodedReadOutcome {
    Returned(ProducedObjectRoot),
    DidNotEnter {
        diagnostic: Option<ProducedObjectRoot>,
        metadata: mfm_store::SafeFailureMetadata,
    },
    Indeterminate {
        diagnostic: Option<ProducedObjectRoot>,
        metadata: mfm_store::SafeFailureMetadata,
    },
    NonDomainFailure(NonDomainFailure),
}

impl EncodedReadObservation {
    fn material(&self) -> ReadObservationMaterial {
        match &self.outcome {
            EncodedReadOutcome::Returned(returned_root) => ReadObservationMaterial::Returned {
                returned_root: returned_root.clone(),
            },
            EncodedReadOutcome::DidNotEnter {
                diagnostic,
                metadata,
            } => ReadObservationMaterial::DidNotEnter {
                diagnostic_root: diagnostic.clone(),
                metadata: metadata.clone(),
            },
            EncodedReadOutcome::Indeterminate {
                diagnostic,
                metadata,
            } => ReadObservationMaterial::Indeterminate {
                diagnostic_root: diagnostic.clone(),
                metadata: metadata.clone(),
            },
            EncodedReadOutcome::NonDomainFailure(failure) => {
                ReadObservationMaterial::NonDomainFailure { failure: *failure }
            }
        }
    }
}

#[derive(Clone)]
struct EffectIntent {
    request_transition_ref: TransitionRef,
    input_manifest_ref: InputManifestRef,
    effect_key: EffectKey,
    semantic_request_ref: ValueRef,
    request_digest: RequestDigest,
    executor_binding_ref: CapabilityBindingRef,
}

struct FrozenReadBaseline {
    request_ref: ValueRef,
    request_bytes: PlainCanonicalJsonBytes,
    routing_generation_ref: ContentRef,
}

#[derive(Clone, Copy)]
enum IntegrityFinding {
    InvalidEvidence,
}

enum ActionResult {
    Outcome(DriveOutcome),
    Retry,
}

struct ObservationRetryBackoff {
    next_delay_ms: u64,
}

impl ObservationRetryBackoff {
    const fn new() -> Self {
        Self { next_delay_ms: 10 }
    }

    const fn reset(&mut self) {
        self.next_delay_ms = 10;
    }

    fn take_delay(&mut self) -> std::time::Duration {
        let delay = std::time::Duration::from_millis(self.next_delay_ms);
        self.next_delay_ms = self.next_delay_ms.saturating_mul(2).min(1_000);
        delay
    }

    async fn wait(&mut self) {
        tokio::time::sleep(self.take_delay()).await;
    }
}

trait ObservationRetry {
    fn reset(&mut self);

    async fn wait(&mut self);
}

impl ObservationRetry for ObservationRetryBackoff {
    fn reset(&mut self) {
        ObservationRetryBackoff::reset(self);
    }

    async fn wait(&mut self) {
        ObservationRetryBackoff::wait(self).await;
    }
}

enum ObservationLogicalResolution<ObservationRef, Head> {
    Absent,
    Identical {
        observation_ref: ObservationRef,
        journal_head: Head,
    },
    Conflict,
    Retry,
}

enum ObservationPhysicalResolution {
    AbsentAtCurrentHead,
    StalePredecessor,
    Occupied,
    Retry,
}

enum ObservationPreparation<PreparedAppend, LogicalIdentity, PhysicalIdentity> {
    Prepared {
        append: PreparedAppend,
        logical: LogicalIdentity,
        physical: PhysicalIdentity,
    },
    AlreadyCommitted,
    Retry,
}

enum ObservationAppendAttempt {
    Progress,
    StalePredecessor,
    Conflict,
    Retry,
}

trait ObservationCommitDriver {
    type View;
    type Head: Clone + PartialEq;
    type LogicalIdentity: PartialEq;
    type PhysicalIdentity: PartialEq;
    type PreparedAppend;
    type ObservationRef;
    type Material;

    async fn load_verified(&mut self) -> std::result::Result<Self::View, ()>;

    fn head<'a>(&self, view: &'a Self::View) -> &'a Self::Head;

    fn resolve_logical(
        &mut self,
        view: &Self::View,
        logical: &Self::LogicalIdentity,
    ) -> ObservationLogicalResolution<Self::ObservationRef, Self::Head>;

    fn resolve_physical(
        &mut self,
        view: &Self::View,
        physical: &Self::PhysicalIdentity,
    ) -> ObservationPhysicalResolution;

    fn prepare(
        &mut self,
        view: &Self::View,
        physical: Option<&Self::PhysicalIdentity>,
        material: Self::Material,
    ) -> ObservationPreparation<Self::PreparedAppend, Self::LogicalIdentity, Self::PhysicalIdentity>;

    fn logical_is_expected(&self, logical: &Self::LogicalIdentity) -> bool;

    async fn append(&mut self, append: Self::PreparedAppend) -> ObservationAppendAttempt;
}

struct StoreObservationCommitDriver<'a, B> {
    writer: &'a RunHistoryWriter<B>,
    authority: &'a RunAccessAuthority<Drive>,
    node_id: &'a NodeId,
    append_purpose: &'static str,
    authorization_ref: &'a AuthorizationRef,
}

impl<B> ObservationCommitDriver for StoreObservationCommitDriver<'_, B>
where
    B: FactScanBackend,
{
    type View = VerifiedRunView;
    type Head = JournalHead;
    type LogicalIdentity = mfm_store::LogicalObservationIdentity;
    type PhysicalIdentity = mfm_store::PhysicalAppendIdentity;
    type PreparedAppend = PreparedJournalAppend;
    type ObservationRef = mfm_journal::v2::ObservationRef;
    type Material = ObservationMaterial;

    async fn load_verified(&mut self) -> std::result::Result<Self::View, ()> {
        self.writer
            .load_for_drive(self.authority)
            .await
            .map_err(|_| ())?
            .verify_recorded_history()
            .map_err(|_| ())
    }

    fn head<'a>(&self, view: &'a Self::View) -> &'a Self::Head {
        view.journal_head()
    }

    fn resolve_logical(
        &mut self,
        view: &Self::View,
        logical: &Self::LogicalIdentity,
    ) -> ObservationLogicalResolution<Self::ObservationRef, Self::Head> {
        match view.resolve_logical_observation(logical) {
            Ok(ExactResolution::Absent) => ObservationLogicalResolution::Absent,
            Ok(ExactResolution::Identical(committed)) => ObservationLogicalResolution::Identical {
                observation_ref: committed.observation_ref().clone(),
                journal_head: committed.journal_head().clone(),
            },
            Ok(ExactResolution::Conflict) => ObservationLogicalResolution::Conflict,
            Err(_) => ObservationLogicalResolution::Retry,
        }
    }

    fn resolve_physical(
        &mut self,
        view: &Self::View,
        physical: &Self::PhysicalIdentity,
    ) -> ObservationPhysicalResolution {
        let resolution = match view.resolve_physical_append(physical) {
            Ok(resolution) => resolution,
            Err(_) => return ObservationPhysicalResolution::Retry,
        };
        match resolution.exact() {
            ExactResolution::Identical(_) | ExactResolution::Conflict => {
                ObservationPhysicalResolution::Occupied
            }
            ExactResolution::Absent => {
                let current_predecessor =
                    match JournalPredecessor::journal_head(resolution.current_head()) {
                        Ok(predecessor) => predecessor,
                        Err(_) => return ObservationPhysicalResolution::Retry,
                    };
                if physical.expected_predecessor() == &current_predecessor {
                    ObservationPhysicalResolution::AbsentAtCurrentHead
                } else {
                    ObservationPhysicalResolution::StalePredecessor
                }
            }
        }
    }

    fn prepare(
        &mut self,
        view: &Self::View,
        physical: Option<&Self::PhysicalIdentity>,
        material: ObservationMaterial,
    ) -> ObservationPreparation<Self::PreparedAppend, Self::LogicalIdentity, Self::PhysicalIdentity>
    {
        let append_id = match physical {
            Some(physical) => physical.append_request_id().clone(),
            None => match append_request_id(
                self.append_purpose,
                view.journal_head(),
                Some(self.node_id),
            ) {
                Ok(append_id) => append_id,
                Err(_) => return ObservationPreparation::Retry,
            },
        };
        let append = match physical {
            Some(_) => self.writer.prepare_append(
                self.authority,
                view,
                append_id,
                ExistingRunAppendMaterial::Observation(Box::new(material)),
            ),
            None => self.writer.prepare_observation_resolution(
                self.authority,
                view,
                append_id,
                material,
            ),
        };
        let append = match append {
            Ok(append) => append,
            Err(StoreError::ObservationAlreadyCommitted) => {
                return ObservationPreparation::AlreadyCommitted;
            }
            Err(_) => return ObservationPreparation::Retry,
        };
        let logical = match append.logical_observation_identity() {
            Ok(Some(logical)) => logical,
            Ok(None) | Err(_) => return ObservationPreparation::Retry,
        };
        let physical = append.physical_identity();
        ObservationPreparation::Prepared {
            append,
            logical,
            physical,
        }
    }

    fn logical_is_expected(&self, logical: &Self::LogicalIdentity) -> bool {
        logical.authorization_ref() == self.authorization_ref
    }

    async fn append(&mut self, append: Self::PreparedAppend) -> ObservationAppendAttempt {
        match self.writer.append(self.authority, append).await {
            Ok(
                AppendOutcome::NewlyAppended(NewlyAppended::Observation(_))
                | AppendOutcome::AlreadyCommitted(_),
            ) => ObservationAppendAttempt::Progress,
            Ok(AppendOutcome::Rejected(AppendRejection::StaleHead { .. })) => {
                ObservationAppendAttempt::StalePredecessor
            }
            Ok(AppendOutcome::Rejected(AppendRejection::AppendRequestConflict)) => {
                ObservationAppendAttempt::Conflict
            }
            Ok(
                AppendOutcome::OutcomeUnknown
                | AppendOutcome::Rejected(
                    AppendRejection::AdmissionConflict | AppendRejection::RunClosed,
                )
                | AppendOutcome::NewlyAppended(
                    NewlyAppended::RunAdmitted(_)
                    | NewlyAppended::Transition(_)
                    | NewlyAppended::Authorization(_),
                ),
            )
            | Err(_) => ObservationAppendAttempt::Retry,
        }
    }
}

async fn commit_observation_loop<K, F, Driver, Retry>(
    driver: &mut Driver,
    pending: &mut PendingObservation<K, Driver::LogicalIdentity, Driver::PhysicalIdentity>,
    mut material: F,
    retry: &mut Retry,
) -> Result<CommittedObservation<K, Driver::Head, Driver::ObservationRef>>
where
    K: AccessKind,
    F: FnMut(&K::PendingMaterial) -> Driver::Material,
    Driver: ObservationCommitDriver,
    Retry: ObservationRetry,
{
    let mut last_verified_head: Option<Driver::Head> = None;
    loop {
        let view = match driver.load_verified().await {
            Ok(view) => view,
            Err(()) => {
                retry.wait().await;
                continue;
            }
        };
        let loaded_head = driver.head(&view).clone();
        if last_verified_head
            .as_ref()
            .is_some_and(|head| head != &loaded_head)
        {
            retry.reset();
        }
        last_verified_head = Some(loaded_head);

        if let Some(logical) = pending.logical_identity() {
            match driver.resolve_logical(&view, logical) {
                ObservationLogicalResolution::Absent => {}
                ObservationLogicalResolution::Identical {
                    observation_ref,
                    journal_head,
                } => {
                    return Ok(CommittedObservation::new(observation_ref, journal_head));
                }
                ObservationLogicalResolution::Conflict => {
                    return Err(RuntimeError::ObservationConflict);
                }
                ObservationLogicalResolution::Retry => {
                    retry.wait().await;
                    continue;
                }
            }
        }

        if let Some(physical) = pending.physical_identity() {
            match driver.resolve_physical(&view, physical) {
                ObservationPhysicalResolution::AbsentAtCurrentHead => {}
                ObservationPhysicalResolution::StalePredecessor => {
                    pending.discard_stale_physical();
                    retry.reset();
                    continue;
                }
                ObservationPhysicalResolution::Occupied => {
                    return Err(RuntimeError::ObservationConflict);
                }
                ObservationPhysicalResolution::Retry => {
                    retry.wait().await;
                    continue;
                }
            }
        }

        let first_physical_attempt = pending.physical_identity().is_none();
        let candidate_material = material(pending.material());
        let prepared = driver.prepare(&view, pending.physical_identity(), candidate_material);
        let (append, logical, physical) = match prepared {
            ObservationPreparation::Prepared {
                append,
                logical,
                physical,
            } => (append, logical, physical),
            ObservationPreparation::AlreadyCommitted => {
                retry.reset();
                continue;
            }
            ObservationPreparation::Retry => {
                retry.wait().await;
                continue;
            }
        };
        if !driver.logical_is_expected(&logical) {
            return Err(RuntimeError::ObservationConflict);
        }
        pending.remember_prepared(logical, physical)?;

        // Freeze and resolve the logical identity before the first physical append.
        if first_physical_attempt {
            continue;
        }

        match driver.append(append).await {
            ObservationAppendAttempt::Progress => retry.reset(),
            ObservationAppendAttempt::StalePredecessor => {
                pending.discard_stale_physical();
                retry.reset();
            }
            ObservationAppendAttempt::Conflict => {
                return Err(RuntimeError::ObservationConflict);
            }
            ObservationAppendAttempt::Retry => retry.wait().await,
        }
    }
}

impl<B> Runtime<B>
where
    B: FactScanBackend,
{
    /// Loads a fresh verified view and performs at most one legal action.
    pub async fn drive_once(&self, authority: RunAccessAuthority<Drive>) -> Result<DriveOutcome> {
        loop {
            let journal = self
                .writer
                .load_for_drive(&authority)
                .await
                .map_err(|error| map_store_error(&error))?;
            let view = journal.verify_recorded_history()?;
            let admitted = match self.program_registry.qualify_admitted_run(&view) {
                Ok(admitted) => admitted,
                Err(error) => return candidate_failure_outcome(&view, error),
            };
            let decision = match self.derive_decision(&authority, &view, &admitted).await {
                Ok(decision) => decision,
                Err(RuntimeError::CandidateCertification(error)) => {
                    return candidate_failure_outcome(&view, error);
                }
                Err(error) => return Err(error),
            };
            let selected = select_action(decision);
            let result = match selected {
                SelectedAction::Settle { candidate, .. } => {
                    self.commit_settlement(&authority, &view, candidate).await?
                }
                SelectedAction::Local { candidate, .. } => {
                    self.commit_local(&authority, &view, candidate).await?
                }
                SelectedAction::Access { candidate, .. } => {
                    self.perform_access(&authority, &view, candidate).await?
                }
                SelectedAction::Closed => ActionResult::Outcome(DriveOutcome::Closed {
                    closure_ref: closure_ref(&view)?,
                }),
                SelectedAction::IntegrityBlocked { .. } => {
                    ActionResult::Outcome(waiting(&view, DriveWaitReason::IntegrityBlock))
                }
                SelectedAction::OperationallyBlocked => {
                    ActionResult::Outcome(waiting(&view, DriveWaitReason::OperationalBlock))
                }
                SelectedAction::Waiting => {
                    ActionResult::Outcome(waiting(&view, DriveWaitReason::RetryableEvidenceGap))
                }
            };
            match result {
                ActionResult::Outcome(outcome) => return Ok(outcome),
                ActionResult::Retry => {}
            }
        }
    }

    async fn derive_decision(
        &self,
        authority: &RunAccessAuthority<Drive>,
        view: &VerifiedRunView,
        admitted: &QualifiedAdmittedProgram,
    ) -> Result<
        DecisionInput<SettlementAction, LocalActionMaterial, PreparedAccess, IntegrityFinding>,
    > {
        let candidate_callbacks = self.program_registry.candidate_callbacks(
            admitted.candidate_identity(),
            admitted.recorded_operation_id(),
        )?;
        let terminal_consumed_observations = terminal_consumed_observations(view)?;
        let mut occurrences = Vec::with_capacity(view.certified_spec().nodes().len());
        let mut deferred_access = Vec::new();
        let mut integrity_blocks = Vec::new();
        let mut operationally_blocked = false;

        for (certified_order, node) in view.certified_spec().nodes().iter().enumerate() {
            let phase = view
                .node_phase(node.node_id())
                .ok_or(RuntimeError::InvalidCallbackResult)?;
            let terminal = phase == NodePhase::Terminal;
            let mut occurrence = OccurrenceCandidate {
                certified_order,
                settlement: None,
                local: None,
                access: None,
                terminal,
            };
            let Some(callbacks) = candidate_callbacks.state(node.state_contract_ref()) else {
                operationally_blocked = true;
                occurrences.push(occurrence);
                continue;
            };

            if terminal {
                let Some(outcome) = view.node_terminal_outcome(node.node_id()) else {
                    integrity_blocks.push(integrity(certified_order, 0));
                    occurrences.push(occurrence);
                    continue;
                };
                if outcome == NodeTerminalOutcome::Skipped {
                    occurrences.push(occurrence);
                    continue;
                }
            }

            if phase == NodePhase::Unstarted {
                match frame_readiness(view, node)? {
                    FrameReadiness::Blocked => {
                        occurrence.local = Some((
                            LocalAction::DependencySkip,
                            LocalActionMaterial::DependencySkip {
                                node_id: node.node_id().clone(),
                            },
                        ));
                        occurrences.push(occurrence);
                        continue;
                    }
                    FrameReadiness::Waiting => {
                        occurrences.push(occurrence);
                        continue;
                    }
                    FrameReadiness::Ready => {}
                }
            }

            match node.execution() {
                CertifiedStateExecution::Pure => {
                    if phase != NodePhase::Unstarted && phase != NodePhase::Terminal {
                        integrity_blocks.push(integrity(certified_order, 0));
                    }
                }
                CertifiedStateExecution::Read {
                    capability_operation_id,
                    returned_contract,
                    safe_failure_contract,
                    ..
                } => {
                    if phase != NodePhase::Unstarted && phase != NodePhase::Terminal {
                        integrity_blocks.push(integrity(certified_order, 0));
                        occurrences.push(occurrence);
                        continue;
                    }
                    let history = match view.access_history(node.node_id()) {
                        Ok(history) => history,
                        Err(_) => {
                            integrity_blocks.push(integrity(certified_order, 0));
                            occurrences.push(occurrence);
                            continue;
                        }
                    };
                    let frame = self.prepare_frame(authority, view, node.node_id()).await?;
                    let callback_frame = verified_callback_frame(&frame)?;
                    let baseline = match validate_read_history(view, node, &frame, &history) {
                        Ok(baseline) => baseline,
                        Err(_) => {
                            integrity_blocks.push(integrity(certified_order, 0));
                            occurrences.push(occurrence);
                            continue;
                        }
                    };
                    let is_fact_selection =
                        capability_operation_id.as_str() == FACT_SELECTION_OPERATION_ID;
                    if is_fact_selection {
                        let observation_refs = match fact_selection_observation_refs(
                            view,
                            &history,
                            returned_contract,
                            safe_failure_contract,
                        ) {
                            Ok(observation_refs) => observation_refs,
                            Err(_) => {
                                integrity_blocks.push(integrity(certified_order, 0));
                                occurrences.push(occurrence);
                                continue;
                            }
                        };
                        if !observation_refs.is_empty() {
                            if let Err(error) = self
                                .writer
                                .verify_drive_fact_selection_observations(
                                    authority,
                                    view,
                                    &observation_refs,
                                )
                                .await
                            {
                                match map_store_error(&error) {
                                    RuntimeError::StoreBackendUnavailable => {
                                        return Err(RuntimeError::StoreBackendUnavailable);
                                    }
                                    _ => {
                                        integrity_blocks.push(integrity(certified_order, 0));
                                        occurrences.push(occurrence);
                                        continue;
                                    }
                                }
                            }
                        }
                    }
                    match scan_read_suffix(
                        view,
                        &history,
                        &callbacks,
                        &callback_frame,
                        returned_contract,
                        safe_failure_contract,
                        is_fact_selection,
                    )? {
                        EvidenceScan::InvalidEvidence { observation_order } => {
                            integrity_blocks.push(integrity(certified_order, observation_order));
                        }
                        EvidenceScan::Settlement {
                            observation_order,
                            candidate: (request_ref, observation_ref, settlement),
                        } if phase == NodePhase::Unstarted => {
                            occurrence.settlement = Some((
                                observation_order,
                                SettlementAction::Read {
                                    frame: Box::new(frame),
                                    request_ref,
                                    observation_ref,
                                    settlement,
                                    settlement_contract: node.settlement_contract().clone(),
                                },
                            ));
                        }
                        EvidenceScan::Settlement {
                            candidate: (_, observation_ref, _),
                            ..
                        } if phase == NodePhase::Terminal => {
                            if terminal_consumed_observations.get(node.node_id())
                                != Some(&observation_ref)
                            {
                                integrity_blocks.push(integrity(certified_order, 0));
                            }
                        }
                        EvidenceScan::AllInsufficient if phase == NodePhase::Terminal => {
                            integrity_blocks.push(integrity(certified_order, 0));
                        }
                        EvidenceScan::AllInsufficient => {
                            deferred_access.push((
                                certified_order,
                                DeferredAccessDerivation::Read {
                                    frame: Box::new(frame),
                                    baseline: baseline.map(Box::new),
                                    authorization_count: usize::try_from(
                                        history.authorization_count(),
                                    )
                                    .map_err(|_| RuntimeError::InvalidCallbackResult)?,
                                    is_fact_selection,
                                },
                            ));
                        }
                        EvidenceScan::Settlement { .. } => {
                            integrity_blocks.push(integrity(certified_order, 0));
                        }
                    }
                }
                CertifiedStateExecution::Effect {
                    ensure_result_contract,
                    terminal_evidence_contract,
                    ..
                } => {
                    if phase != NodePhase::Unstarted {
                        let intent = match effect_intent(view, node) {
                            Ok(intent) => intent,
                            Err(_) => {
                                integrity_blocks.push(integrity(certified_order, 0));
                                occurrences.push(occurrence);
                                continue;
                            }
                        };
                        let history = match view.access_history(node.node_id()) {
                            Ok(history) => history,
                            Err(_) => {
                                integrity_blocks.push(integrity(certified_order, 0));
                                occurrences.push(occurrence);
                                continue;
                            }
                        };
                        if validate_effect_history(&history, &intent).is_err() {
                            integrity_blocks.push(integrity(certified_order, 0));
                            occurrences.push(occurrence);
                            continue;
                        }
                        let frame = self.prepare_frame(authority, view, node.node_id()).await?;
                        if frame.input_manifest_ref() != &intent.input_manifest_ref {
                            integrity_blocks.push(integrity(certified_order, 0));
                            occurrences.push(occurrence);
                            continue;
                        }
                        let callback_frame = verified_callback_frame(&frame)?;
                        match scan_effect_suffix(
                            view,
                            &history,
                            &callbacks,
                            &callback_frame,
                            &intent,
                            ensure_result_contract,
                            terminal_evidence_contract,
                        )? {
                            EvidenceScan::InvalidEvidence { observation_order } => {
                                integrity_blocks
                                    .push(integrity(certified_order, observation_order));
                            }
                            EvidenceScan::Settlement {
                                observation_order,
                                candidate: (observation_ref, settlement),
                            } if phase == NodePhase::AwaitingEffect => {
                                occurrence.settlement = Some((
                                    observation_order,
                                    SettlementAction::Effect {
                                        node_id: node.node_id().clone(),
                                        request_transition_ref: intent
                                            .request_transition_ref
                                            .clone(),
                                        observation_ref,
                                        settlement,
                                        settlement_contract: node.settlement_contract().clone(),
                                    },
                                ));
                            }
                            EvidenceScan::Settlement {
                                candidate: (observation_ref, _),
                                ..
                            } if phase == NodePhase::Terminal => {
                                if terminal_consumed_observations.get(node.node_id())
                                    != Some(&observation_ref)
                                {
                                    integrity_blocks.push(integrity(certified_order, 0));
                                }
                            }
                            EvidenceScan::AllInsufficient if phase == NodePhase::Terminal => {
                                integrity_blocks.push(integrity(certified_order, 0));
                            }
                            EvidenceScan::AllInsufficient => {
                                deferred_access.push((
                                    certified_order,
                                    DeferredAccessDerivation::Ensure {
                                        intent: Box::new(intent),
                                        authorization_count: usize::try_from(
                                            history.authorization_count(),
                                        )
                                        .map_err(|_| RuntimeError::InvalidCallbackResult)?,
                                    },
                                ));
                            }
                            EvidenceScan::Settlement { .. } => {
                                integrity_blocks.push(integrity(certified_order, 0));
                            }
                        }
                    }
                }
            }
            occurrences.push(occurrence);
        }

        let closed = view.run_phase() == RunPhase::Closed;
        let open_all_terminal = (view.run_phase() == RunPhase::Open
            && occurrences.iter().all(|occurrence| occurrence.terminal))
        .then_some(IntegrityFinding::InvalidEvidence);
        if !integrity_blocks.is_empty()
            || occurrences
                .iter()
                .any(|occurrence| occurrence.settlement.is_some())
            || closed
            || open_all_terminal.is_some()
        {
            return Ok(DecisionInput {
                occurrences,
                integrity_blocks,
                operationally_blocked,
                closed,
                open_all_terminal,
            });
        }

        for (index, node) in view.certified_spec().nodes().iter().enumerate() {
            if occurrences[index].local.is_some() {
                return Ok(DecisionInput {
                    occurrences,
                    integrity_blocks,
                    operationally_blocked,
                    closed: false,
                    open_all_terminal: None,
                });
            }
            if view.node_phase(node.node_id()) != Some(NodePhase::Unstarted) {
                continue;
            }
            let Some(callbacks) = candidate_callbacks.state(node.state_contract_ref()) else {
                continue;
            };
            match frame_readiness(view, node)? {
                FrameReadiness::Blocked => {
                    occurrences[index].local = Some((
                        LocalAction::DependencySkip,
                        LocalActionMaterial::DependencySkip {
                            node_id: node.node_id().clone(),
                        },
                    ));
                }
                FrameReadiness::Waiting => continue,
                FrameReadiness::Ready => match node.execution() {
                    CertifiedStateExecution::Pure => {
                        let frame = self.prepare_frame(authority, view, node.node_id()).await?;
                        let callback_frame = verified_callback_frame(&frame)?;
                        let settlement = callbacks.settle_pure(&callback_frame)?;
                        occurrences[index].local = Some((
                            LocalAction::PureSettlement,
                            LocalActionMaterial::Pure {
                                node_id: node.node_id().clone(),
                                frame: Box::new(frame),
                                settlement,
                                settlement_contract: node.settlement_contract().clone(),
                            },
                        ));
                    }
                    CertifiedStateExecution::Effect {
                        executor_operation_id,
                        executor_binding_ref,
                        request_contract,
                        ensure_result_contract,
                        terminal_evidence_contract,
                        ..
                    } => {
                        let Some(entry) = exact_effect_entry(
                            &self.program_registry,
                            executor_binding_ref,
                            executor_operation_id,
                            request_contract,
                            ensure_result_contract,
                            terminal_evidence_contract,
                        ) else {
                            operationally_blocked = true;
                            continue;
                        };
                        let frame = self.prepare_frame(authority, view, node.node_id()).await?;
                        let callback_frame = verified_callback_frame(&frame)?;
                        let authored = callbacks.author_request(&callback_frame)?;
                        let invoker = RuntimeEffectInvoker::from_entry(entry)?;
                        let prepared = invoker.identify_request(
                            effect_request_context(view, node.node_id(), entry),
                            authored,
                        )?;
                        occurrences[index].local = Some((
                            LocalAction::EffectRequest,
                            LocalActionMaterial::EffectRequest {
                                node_id: node.node_id().clone(),
                                frame: Box::new(frame),
                                request_contract: Box::new(request_contract.clone()),
                                request: prepared.proposed,
                                effect_key: prepared.effect_key,
                                request_digest: prepared.request_digest,
                                executor_binding_ref: CapabilityBindingRef::new(
                                    executor_binding_ref,
                                )?,
                            },
                        ));
                    }
                    CertifiedStateExecution::Read { .. } => continue,
                },
            }
            return Ok(DecisionInput {
                occurrences,
                integrity_blocks,
                operationally_blocked,
                closed: false,
                open_all_terminal: None,
            });
        }

        deferred_access.sort_by_key(|(index, deferred)| (deferred.authorization_count(), *index));
        for (index, deferred) in deferred_access {
            let node = &view.certified_spec().nodes()[index];
            let Some(callbacks) = candidate_callbacks.state(node.state_contract_ref()) else {
                continue;
            };
            match deferred {
                DeferredAccessDerivation::Read {
                    frame,
                    baseline,
                    authorization_count,
                    is_fact_selection,
                } => {
                    let CertifiedStateExecution::Read {
                        capability_operation_id,
                        capability_binding_ref,
                        request_contract,
                        returned_contract,
                        safe_failure_contract,
                    } = node.execution()
                    else {
                        return Err(RuntimeError::InvalidCallbackResult);
                    };
                    let callback_frame = verified_callback_frame(&frame)?;
                    let authored = match baseline.as_ref() {
                        Some(baseline) => {
                            let retained = VerifiedValueMaterial::new(
                                baseline.request_bytes.clone(),
                                baseline.request_ref.clone(),
                            );
                            callbacks.decode_request(&retained)?
                        }
                        None => callbacks.author_request(&callback_frame)?,
                    };
                    if is_fact_selection {
                        match qualify_fact_selection_request(
                            view,
                            node,
                            capability_binding_ref,
                            request_contract,
                            returned_contract,
                            authored,
                            baseline.as_deref(),
                        ) {
                            Ok((request, proposed, routing_generation_ref)) => {
                                occurrences[index].access = Some((
                                    authorization_count,
                                    AccessAction::Read,
                                    PreparedAccess::FactSelection(Prepared::new(Box::new(
                                        FactSelectionAccessAction {
                                            node_id: node.node_id().clone(),
                                            frame: *frame,
                                            request_contract: request_contract.clone(),
                                            routing_generation_ref,
                                            request,
                                            proposed,
                                            store_unavailable_failure: non_domain_failure(
                                                NonDomainEntryStatus::MayHaveEntered,
                                                NonDomainDisposition::RetryableOperational,
                                                NonDomainFailureCode::FactStoreUnavailable,
                                            )?,
                                            history_invalid_failure: non_domain_failure(
                                                NonDomainEntryStatus::MayHaveEntered,
                                                NonDomainDisposition::IntegrityBlocked,
                                                NonDomainFailureCode::FactHistoryInvalid,
                                            )?,
                                            adapter_contract_failure: non_domain_failure(
                                                NonDomainEntryStatus::MayHaveEntered,
                                                NonDomainDisposition::IntegrityBlocked,
                                                NonDomainFailureCode::AdapterContractViolation,
                                            )?,
                                        },
                                    ))),
                                ));
                                break;
                            }
                            Err(RuntimeError::CatalogSelection) => {
                                operationally_blocked = true;
                            }
                            Err(error) => return Err(error),
                        }
                    } else {
                        let Some(entry) = exact_read_entry(
                            &self.program_registry,
                            capability_binding_ref,
                            capability_operation_id,
                            request_contract,
                            returned_contract,
                            safe_failure_contract,
                        ) else {
                            operationally_blocked = true;
                            continue;
                        };
                        let invoker = RuntimeReadInvoker::from_entry(entry)?.clone();
                        let Some(result_encoder) = self
                            .program_registry
                            .state(node.state_contract_ref())
                            .cloned()
                        else {
                            operationally_blocked = true;
                            continue;
                        };
                        let safe_failure_contract_ref =
                            entry.binding().fields()?.safe_failure_contract_ref;
                        match invoker.route_request(entry, authored) {
                            Ok(request) => {
                                if let Some(baseline) = baseline.as_ref() {
                                    if !routed_request_matches(&request, baseline)? {
                                        integrity_blocks
                                            .push(integrity(occurrences[index].certified_order, 0));
                                        break;
                                    }
                                }
                                occurrences[index].access = Some((
                                    authorization_count,
                                    AccessAction::Read,
                                    PreparedAccess::Read(Prepared::new(Box::new(
                                        ReadAccessAction {
                                            node_id: node.node_id().clone(),
                                            frame: *frame,
                                            request_contract: request_contract.clone(),
                                            returned_contract: returned_contract.clone(),
                                            safe_failure_contract: safe_failure_contract.clone(),
                                            result_encoder,
                                            invoker,
                                            safe_failure_contract_ref,
                                            request,
                                            result_encoding_failure: non_domain_failure(
                                                NonDomainEntryStatus::MayHaveEntered,
                                                NonDomainDisposition::IntegrityBlocked,
                                                NonDomainFailureCode::ResultEncodingFailure,
                                            )?,
                                        },
                                    ))),
                                ));
                                break;
                            }
                            Err(RuntimeError::CatalogSelection) => {
                                operationally_blocked = true;
                            }
                            Err(error) => return Err(error),
                        }
                    }
                }
                DeferredAccessDerivation::Ensure {
                    intent,
                    authorization_count,
                } => {
                    let CertifiedStateExecution::Effect {
                        executor_operation_id,
                        executor_binding_ref,
                        request_contract,
                        ensure_result_contract,
                        terminal_evidence_contract,
                        ..
                    } = node.execution()
                    else {
                        return Err(RuntimeError::InvalidCallbackResult);
                    };
                    let Some(entry) = exact_effect_entry(
                        &self.program_registry,
                        executor_binding_ref,
                        executor_operation_id,
                        request_contract,
                        ensure_result_contract,
                        terminal_evidence_contract,
                    ) else {
                        operationally_blocked = true;
                        continue;
                    };
                    let invoker = RuntimeEffectInvoker::from_entry(entry)?.clone();
                    let retained = verified_retained_value(
                        view,
                        &intent.semantic_request_ref,
                        request_contract,
                    )?;
                    let request = callbacks.decode_request(&retained)?;
                    if request.proposed().canonical().as_bytes() != retained.canonical().as_bytes()
                        || !proposed_matches_value_ref(
                            request.proposed(),
                            &intent.semantic_request_ref,
                        )?
                        || request.request_type() != entry.request_type()
                    {
                        integrity_blocks.push(integrity(occurrences[index].certified_order, 0));
                        break;
                    }
                    let invocation_context = EffectInvocationContext {
                        binding: entry.binding().clone(),
                        operation_id: executor_operation_id.clone(),
                        request_type: entry.request_type(),
                        response_type: entry.response_type(),
                        failure_type: entry.failure_type(),
                        tenant_scope_id: view.tenant_scope_id().clone(),
                        store_scope_id: view.store_identity().store_scope_id().clone(),
                        run_id: view.run_id().clone(),
                        node_id: node.node_id().clone(),
                        effect_key: intent.effect_key.clone(),
                        request_digest: intent.request_digest.clone(),
                        adapter_may_have_entered: invoker.adapter_may_have_entered(),
                    };
                    occurrences[index].access = Some((
                        authorization_count,
                        AccessAction::Ensure,
                        PreparedAccess::Ensure(Prepared::new(Box::new(EnsureAccessAction {
                            node_id: node.node_id().clone(),
                            intent: *intent,
                            invoker,
                            invocation_context,
                            request,
                        }))),
                    ));
                    break;
                }
            }
        }

        Ok(DecisionInput {
            occurrences,
            integrity_blocks,
            operationally_blocked,
            closed: false,
            open_all_terminal: None,
        })
    }

    async fn prepare_frame(
        &self,
        authority: &RunAccessAuthority<Drive>,
        view: &VerifiedRunView,
        node_id: &NodeId,
    ) -> Result<PreparedFrame> {
        self.writer
            .prepare_frame(authority, view, node_id)
            .await
            .map_err(|error| map_store_error(&error))
    }

    async fn commit_settlement(
        &self,
        authority: &RunAccessAuthority<Drive>,
        view: &VerifiedRunView,
        action: SettlementAction,
    ) -> Result<ActionResult> {
        let (node_id, material, action_id) = match action {
            SettlementAction::Read {
                frame,
                request_ref,
                observation_ref,
                settlement,
                settlement_contract,
            } => {
                let node_id = frame.node_id().clone();
                (
                    node_id,
                    TransitionMaterial::ReadSettled {
                        prepared_frame: frame,
                        immutable_request_ref: request_ref,
                        consumed_observation_ref: observation_ref,
                        settlement: settlement_material(settlement, &settlement_contract)?,
                        object_graph: ObjectGraphProposal::empty(),
                    },
                    "read_settlement",
                )
            }
            SettlementAction::Effect {
                node_id,
                request_transition_ref,
                observation_ref,
                settlement,
                settlement_contract,
            } => (
                node_id,
                TransitionMaterial::EffectSettled {
                    request_transition_ref,
                    consumed_terminal_observation_ref: observation_ref,
                    settlement: settlement_material(settlement, &settlement_contract)?,
                    object_graph: ObjectGraphProposal::empty(),
                },
                "effect_settlement",
            ),
        };
        self.append_transition(authority, view, node_id, action_id, material)
            .await
    }

    async fn commit_local(
        &self,
        authority: &RunAccessAuthority<Drive>,
        view: &VerifiedRunView,
        action: LocalActionMaterial,
    ) -> Result<ActionResult> {
        let (node_id, action_id, material) = match action {
            LocalActionMaterial::Pure {
                node_id,
                frame,
                settlement,
                settlement_contract,
            } => (
                node_id,
                "pure_settlement",
                TransitionMaterial::PureSettled {
                    prepared_frame: frame,
                    settlement: settlement_material(settlement, &settlement_contract)?,
                    object_graph: ObjectGraphProposal::empty(),
                },
            ),
            LocalActionMaterial::EffectRequest {
                node_id,
                frame,
                request_contract,
                request,
                effect_key,
                request_digest,
                executor_binding_ref,
            } => (
                node_id,
                "effect_request",
                TransitionMaterial::EffectRequested {
                    prepared_frame: frame,
                    semantic_request_root: Box::new(ProducedObjectRoot::new(
                        *request_contract,
                        request.canonical().clone(),
                    )),
                    effect_key,
                    request_digest,
                    executor_binding_ref,
                    object_graph: ObjectGraphProposal::empty(),
                },
            ),
            LocalActionMaterial::DependencySkip { node_id } => (
                node_id.clone(),
                "dependency_skip",
                TransitionMaterial::DependencySkipped { node_id },
            ),
        };
        self.append_transition(authority, view, node_id, action_id, material)
            .await
    }

    async fn append_transition(
        &self,
        authority: &RunAccessAuthority<Drive>,
        view: &VerifiedRunView,
        node_id: NodeId,
        action: &'static str,
        material: TransitionMaterial,
    ) -> Result<ActionResult> {
        let append_id = append_request_id(action, view.journal_head(), Some(&node_id))?;
        let append = self.writer.prepare_append(
            authority,
            view,
            append_id,
            ExistingRunAppendMaterial::Transition(Box::new(material)),
        )?;
        let outcome = self
            .writer
            .append(authority, append)
            .await
            .map_err(|error| map_store_error(&error))?;
        transition_append_outcome(outcome)
    }

    async fn perform_access(
        &self,
        authority: &RunAccessAuthority<Drive>,
        view: &VerifiedRunView,
        action: PreparedAccess,
    ) -> Result<ActionResult> {
        match action {
            PreparedAccess::Read(action) => {
                self.perform_read(authority, view, *action.into_invocation())
                    .await
            }
            PreparedAccess::FactSelection(action) => {
                self.perform_fact_selection(authority, view, *action.into_invocation())
                    .await
            }
            PreparedAccess::Ensure(action) => {
                self.perform_ensure(authority, view, *action.into_invocation())
                    .await
            }
        }
    }

    async fn perform_read(
        &self,
        authority: &RunAccessAuthority<Drive>,
        view: &VerifiedRunView,
        action: ReadAccessAction,
    ) -> Result<ActionResult> {
        let routing_generation_ref = action.request.routing_generation_ref().clone();
        let immutable_request_root = ProducedObjectRoot::new(
            action.request_contract.clone(),
            action.request.proposed().canonical().clone(),
        );
        let append_id = append_request_id(
            "read_authorization",
            view.journal_head(),
            Some(&action.node_id),
        )?;
        let append = self.writer.prepare_append(
            authority,
            view,
            append_id,
            ExistingRunAppendMaterial::Authorization(Box::new(AuthorizationMaterial::Read {
                prepared_frame: Box::new(action.frame),
                immutable_request_root: Box::new(immutable_request_root),
                routing_generation_ref: routing_generation_ref.clone(),
            })),
        )?;
        let expected_request_ref = prepared_authorization_request_ref(&append)?;
        let prepared_invocation = action.invoker.prepare_call(
            expected_request_ref,
            ReadInvocationContext {
                routing_generation_ref,
                safe_failure_contract_ref: action.safe_failure_contract_ref,
                adapter_may_have_entered: action.invoker.adapter_may_have_entered(),
            },
            action.request,
        )?;
        let outcome = self
            .writer
            .append(authority, append)
            .await
            .map_err(|error| map_store_error(&error))?;
        let witness = match outcome {
            AppendOutcome::NewlyAppended(NewlyAppended::Authorization(witness)) => witness,
            AppendOutcome::AlreadyCommitted(committed) => {
                return Ok(ActionResult::Outcome(advanced(
                    committed.journal_head().clone(),
                )));
            }
            AppendOutcome::Rejected(
                AppendRejection::StaleHead { .. } | AppendRejection::RunClosed,
            ) => {
                return Ok(ActionResult::Retry);
            }
            AppendOutcome::Rejected(rejection) => return Err(rejection_error(rejection)),
            AppendOutcome::OutcomeUnknown => return Err(RuntimeError::OutcomeUnknown),
            AppendOutcome::NewlyAppended(_) => return Err(RuntimeError::InvalidCallbackResult),
        };
        let witness = Authorized::<Read>::new(*witness).into_invocation();
        let node_id = action.node_id.clone();
        let observation = prepared_invocation.invoke_and_totalize(witness).await;
        let material = self.encode_read_observation(
            &action.result_encoder,
            &action.returned_contract,
            &action.safe_failure_contract,
            observation,
            action.result_encoding_failure,
        );
        let mut pending = PendingObservation::<Read>::new(material);
        let pending_authorization_ref = pending.material().authorization_ref.clone();
        let committed = self
            .commit_observation(
                authority,
                &node_id,
                "read_observation",
                &pending_authorization_ref,
                &mut pending,
                |observation| ObservationMaterial::Read {
                    authorization_ref: observation.authorization_ref.clone(),
                    outcome: Box::new(observation.material()),
                },
            )
            .await?;
        Ok(ActionResult::Outcome(advanced(committed.outcome().clone())))
    }

    fn encode_read_observation(
        &self,
        result_encoder: &QualifiedStateEntry,
        returned_contract: &RetainedValueContract,
        safe_failure_contract: &RetainedValueContract,
        observation: ErasedReadObservation,
        result_encoding_failure: NonDomainFailure,
    ) -> EncodedReadObservation {
        let authorization_ref = observation.authorization_ref;
        let encoded = (|| -> Result<EncodedReadOutcome> {
            let callbacks = result_encoder.callbacks();
            Ok(match observation.outcome {
                ErasedReadOutcome::Returned(value) => {
                    let proposed = callbacks.encode_read_returned(value.as_ref())?;
                    EncodedReadOutcome::Returned(ProducedObjectRoot::new(
                        returned_contract.clone(),
                        proposed.canonical().clone(),
                    ))
                }
                ErasedReadOutcome::DidNotEnter {
                    diagnostic,
                    metadata,
                } => {
                    let diagnostic = diagnostic
                        .as_ref()
                        .map(|diagnostic| {
                            let proposed = callbacks.encode_read_diagnostic(diagnostic.as_ref())?;
                            Ok::<_, RuntimeError>(ProducedObjectRoot::new(
                                safe_failure_contract.clone(),
                                proposed.canonical().clone(),
                            ))
                        })
                        .transpose()?;
                    EncodedReadOutcome::DidNotEnter {
                        diagnostic,
                        metadata,
                    }
                }
                ErasedReadOutcome::Indeterminate {
                    diagnostic,
                    metadata,
                } => {
                    let diagnostic = diagnostic
                        .as_ref()
                        .map(|diagnostic| {
                            let proposed = callbacks.encode_read_diagnostic(diagnostic.as_ref())?;
                            Ok::<_, RuntimeError>(ProducedObjectRoot::new(
                                safe_failure_contract.clone(),
                                proposed.canonical().clone(),
                            ))
                        })
                        .transpose()?;
                    EncodedReadOutcome::Indeterminate {
                        diagnostic,
                        metadata,
                    }
                }
                ErasedReadOutcome::NonDomainFailure(failure) => {
                    EncodedReadOutcome::NonDomainFailure(failure)
                }
            })
        })();
        EncodedReadObservation {
            authorization_ref,
            outcome: encoded.unwrap_or(EncodedReadOutcome::NonDomainFailure(
                result_encoding_failure,
            )),
        }
    }

    async fn perform_ensure(
        &self,
        authority: &RunAccessAuthority<Drive>,
        view: &VerifiedRunView,
        action: EnsureAccessAction,
    ) -> Result<ActionResult> {
        let append_id = append_request_id(
            "ensure_authorization",
            view.journal_head(),
            Some(&action.node_id),
        )?;
        let append = self.writer.prepare_append(
            authority,
            view,
            append_id,
            ExistingRunAppendMaterial::Authorization(Box::new(
                AuthorizationMaterial::EnsureEffect {
                    request_transition_ref: action.intent.request_transition_ref.clone(),
                },
            )),
        )?;
        let expected_request_ref = prepared_authorization_request_ref(&append)?;
        let prepared_invocation = action.invoker.prepare_ensure(
            expected_request_ref,
            action.invocation_context,
            action.request,
        )?;
        let outcome = self
            .writer
            .append(authority, append)
            .await
            .map_err(|error| map_store_error(&error))?;
        let witness = match outcome {
            AppendOutcome::NewlyAppended(NewlyAppended::Authorization(witness)) => witness,
            AppendOutcome::AlreadyCommitted(committed) => {
                return Ok(ActionResult::Outcome(advanced(
                    committed.journal_head().clone(),
                )));
            }
            AppendOutcome::Rejected(
                AppendRejection::StaleHead { .. } | AppendRejection::RunClosed,
            ) => {
                return Ok(ActionResult::Retry);
            }
            AppendOutcome::Rejected(rejection) => return Err(rejection_error(rejection)),
            AppendOutcome::OutcomeUnknown => return Err(RuntimeError::OutcomeUnknown),
            AppendOutcome::NewlyAppended(_) => return Err(RuntimeError::InvalidCallbackResult),
        };
        let witness = Authorized::<Ensure>::new(*witness).into_invocation();
        let node_id = action.node_id.clone();
        let observation = prepared_invocation.invoke_and_totalize(witness).await;
        let mut pending = PendingObservation::<Ensure>::new(observation);
        let pending_authorization_ref = pending.material().authorization_ref.clone();
        let committed = self
            .commit_observation(
                authority,
                &node_id,
                "effect_observation",
                &pending_authorization_ref,
                &mut pending,
                |observation| ObservationMaterial::EnsureEffect {
                    authorization_ref: observation.authorization_ref.clone(),
                    outcome: Box::new(observation.outcome.clone()),
                },
            )
            .await?;
        Ok(ActionResult::Outcome(advanced(committed.outcome().clone())))
    }

    async fn commit_observation<K, F>(
        &self,
        authority: &RunAccessAuthority<Drive>,
        node_id: &NodeId,
        append_purpose: &'static str,
        authorization_ref: &AuthorizationRef,
        pending: &mut PendingObservation<K>,
        material: F,
    ) -> Result<CommittedObservation<K, JournalHead>>
    where
        K: AccessKind,
        F: FnMut(&K::PendingMaterial) -> ObservationMaterial,
    {
        let mut driver = StoreObservationCommitDriver {
            writer: &self.writer,
            authority,
            node_id,
            append_purpose,
            authorization_ref,
        };
        let mut backoff = ObservationRetryBackoff::new();
        commit_observation_loop(&mut driver, pending, material, &mut backoff).await
    }

    async fn perform_fact_selection(
        &self,
        authority: &RunAccessAuthority<Drive>,
        view: &VerifiedRunView,
        action: FactSelectionAccessAction,
    ) -> Result<ActionResult> {
        let node_id = action.node_id.clone();
        let append_id = append_request_id(
            "fact-selection-authorize",
            view.journal_head(),
            Some(&action.node_id),
        )?;
        let append = self.writer.prepare_append(
            authority,
            view,
            append_id,
            ExistingRunAppendMaterial::Authorization(Box::new(AuthorizationMaterial::Read {
                prepared_frame: Box::new(action.frame),
                immutable_request_root: Box::new(ProducedObjectRoot::new(
                    action.request_contract,
                    action.proposed.canonical().clone(),
                )),
                routing_generation_ref: action.routing_generation_ref,
            })),
        )?;
        let PreparedJournalAppend::AuthorizeExternalAccess(append) = append else {
            return Err(RuntimeError::InvalidCallbackResult);
        };
        let outcome = self
            .writer
            .append_fact_selection_authorization(authority, append, action.request)
            .await
            .map_err(|error| map_store_error(&error))?;
        let permit = match outcome {
            FactSelectionAuthorizationOutcome::NewlyAuthorized(permit) => *permit,
            FactSelectionAuthorizationOutcome::AlreadyCommitted(committed) => {
                return Ok(ActionResult::Outcome(advanced(
                    committed.journal_head().clone(),
                )));
            }
            FactSelectionAuthorizationOutcome::Rejected(
                AppendRejection::StaleHead { .. } | AppendRejection::RunClosed,
            ) => return Ok(ActionResult::Retry),
            FactSelectionAuthorizationOutcome::Rejected(rejection) => {
                return Err(rejection_error(rejection));
            }
            FactSelectionAuthorizationOutcome::OutcomeUnknown => {
                return Err(RuntimeError::OutcomeUnknown);
            }
        };
        let permit = Authorized::<FactSelection>::new(permit).into_invocation();
        let authorization_ref = permit.authorization_ref().clone();
        let outcome = match self.writer.scan_fact_selection(permit).await {
            Ok(completed) => FactSelectionObservationOutcome::Returned(Box::new(completed.seal())),
            Err(error) => FactSelectionObservationOutcome::NonDomainFailure(
                match error.fact_scan_failure_provenance() {
                    FactScanFailureProvenance::StoreUnavailable => action.store_unavailable_failure,
                    FactScanFailureProvenance::HistoryInvalid => action.history_invalid_failure,
                    FactScanFailureProvenance::AdapterContractViolation => {
                        action.adapter_contract_failure
                    }
                },
            ),
        };
        let mut pending = PendingObservation::<FactSelection>::new(FactSelectionObservation {
            authorization_ref,
            outcome,
        });
        let pending_authorization_ref = pending.material().authorization_ref.clone();
        let committed = self
            .commit_observation(
                authority,
                &node_id,
                "fact_selection_observation",
                &pending_authorization_ref,
                &mut pending,
                FactSelectionObservation::material,
            )
            .await?;
        Ok(ActionResult::Outcome(advanced(committed.outcome().clone())))
    }
}

fn exact_read_entry<'a>(
    registry: &'a QualifiedProgramRegistry,
    binding_ref: &ContentRef,
    operation_id: &StableId,
    request_contract: &RetainedValueContract,
    returned_contract: &RetainedValueContract,
    safe_failure_contract: &RetainedValueContract,
) -> Option<&'a QualifiedReadEntry> {
    let entry = registry.read(binding_ref, operation_id)?;
    let operation = entry.operation();
    (entry.binding_ref() == binding_ref
        && operation.operation_id() == operation_id
        && operation.request_contract() == request_contract
        && operation.returned_contract() == returned_contract
        && operation.safe_failure_contract() == safe_failure_contract)
        .then_some(entry)
}

fn exact_effect_entry<'a>(
    registry: &'a QualifiedProgramRegistry,
    binding_ref: &ContentRef,
    operation_id: &StableId,
    request_contract: &RetainedValueContract,
    ensure_result_contract: &RetainedValueContract,
    terminal_evidence_contract: &RetainedValueContract,
) -> Option<&'a QualifiedEffectEntry> {
    let entry = registry.effect(binding_ref, operation_id)?;
    let retained = entry.binding().contract().retained_closure_contract();
    (entry.binding_ref() == binding_ref
        && entry.operation_id() == operation_id
        && entry.binding().contract().semantic_request_contract() == request_contract
        && retained.ensure_result_contract() == ensure_result_contract
        && retained.terminal_evidence_contract() == terminal_evidence_contract)
        .then_some(entry)
}

fn effect_request_context(
    view: &VerifiedRunView,
    node_id: &NodeId,
    entry: &QualifiedEffectEntry,
) -> EffectRequestContext {
    EffectRequestContext {
        binding: entry.binding().clone(),
        operation_id: entry.operation_id().clone(),
        request_type: entry.request_type(),
        response_type: entry.response_type(),
        failure_type: entry.failure_type(),
        tenant_scope_id: view.tenant_scope_id().clone(),
        store_scope_id: view.store_identity().store_scope_id().clone(),
        run_id: view.run_id().clone(),
        node_id: node_id.clone(),
    }
}

fn validate_read_history(
    view: &VerifiedRunView,
    node: &CertifiedNodeContract,
    frame: &PreparedFrame,
    history: &VerifiedNodeAccessHistory<'_>,
) -> Result<Option<FrozenReadBaseline>> {
    let CertifiedStateExecution::Read {
        capability_operation_id,
        capability_binding_ref,
        request_contract,
        returned_contract,
        safe_failure_contract,
    } = node.execution()
    else {
        return Err(RuntimeError::CatalogSelection);
    };
    let expected_binding = CapabilityBindingRef::new(capability_binding_ref)?;
    let mut baseline: Option<FrozenReadBaseline> = None;
    for attempt in history.entries() {
        let authorization = attempt.authorization().fields()?;
        let AuthorizationScopeFields::Read { input_manifest_ref } = authorization.scope.fields()?
        else {
            return Err(RuntimeError::AuthorityMismatch);
        };
        let frozen_ref = authorization
            .frozen_read_intent_ref
            .as_ref()
            .ok_or(RuntimeError::AuthorityMismatch)?;
        let frozen =
            FrozenReadIntent::strict_decode(view.retained_value(frozen_ref)?.bytes())?.fields()?;
        let request = verified_retained_value(view, &authorization.request_ref, request_contract)?;
        if input_manifest_ref != *frame.input_manifest_ref()
            || authorization.capability_binding_ref != expected_binding
            || authorization.capability_operation_id != *capability_operation_id
            || frozen.node_id != *node.node_id()
            || frozen.input_manifest_ref != input_manifest_ref
            || frozen.state_contract_ref != *node.state_contract_ref()
            || frozen.capability_binding_ref != expected_binding
            || frozen.capability_operation_id != *capability_operation_id
            || frozen.request_ref != authorization.request_ref
            || frozen.request_contract != *request_contract
            || frozen.returned_contract != *returned_contract
            || frozen.safe_failure_contract != *safe_failure_contract
        {
            return Err(RuntimeError::AuthorityMismatch);
        }
        match baseline.as_ref() {
            None => {
                baseline = Some(FrozenReadBaseline {
                    request_ref: authorization.request_ref,
                    request_bytes: request.canonical().clone(),
                    routing_generation_ref: frozen.routing_generation_ref,
                });
            }
            Some(expected)
                if expected.request_bytes.as_bytes() == request.canonical().as_bytes()
                    && expected.routing_generation_ref == frozen.routing_generation_ref => {}
            Some(_) => return Err(RuntimeError::AuthorityMismatch),
        }
    }
    Ok(baseline)
}

fn fact_selection_observation_refs(
    view: &VerifiedRunView,
    history: &VerifiedNodeAccessHistory<'_>,
    returned_contract: &RetainedValueContract,
    safe_failure_contract: &RetainedValueContract,
) -> Result<Vec<mfm_journal::v2::ObservationRef>> {
    let mut references = Vec::new();
    for observed in history.observed_suffix() {
        if let Some(disposition) = observed_non_domain_disposition(&observed)? {
            match disposition {
                NonDomainDisposition::RetryableOperational => continue,
                NonDomainDisposition::IntegrityBlocked => {
                    return Err(RuntimeError::InvalidCallbackResult);
                }
            }
        }
        let observation_ref = observed.observation_ref().clone();
        let committed = committed_read_observation(
            view,
            &observed,
            returned_contract,
            safe_failure_contract,
            true,
        )?;
        if !matches!(committed.outcome(), VerifiedReadOutcome::Returned(_)) {
            return Err(RuntimeError::InvalidCallbackResult);
        }
        references.push(observation_ref);
    }
    Ok(references)
}

fn scan_read_suffix(
    view: &VerifiedRunView,
    history: &VerifiedNodeAccessHistory<'_>,
    callbacks: &QualifiedCandidateStateCallbacks<'_>,
    frame: &VerifiedStateFrameMaterial,
    returned_contract: &RetainedValueContract,
    safe_failure_contract: &RetainedValueContract,
    requires_fact_selection_attestation: bool,
) -> Result<
    EvidenceScan<(
        ValueRef,
        mfm_journal::v2::ObservationRef,
        QualifiedSettlement,
    )>,
> {
    let verdicts = history
        .observed_suffix()
        .map(|observed| {
            let audit = observed.audit();
            let request_ref = audit.request_ref().clone();
            if let Some(disposition) = observed_non_domain_disposition(&observed)? {
                return Ok(match disposition {
                    NonDomainDisposition::RetryableOperational => {
                        ObservationVerdict::InsufficientEvidence
                    }
                    NonDomainDisposition::IntegrityBlocked => ObservationVerdict::InvalidEvidence,
                });
            }
            let verdict = match committed_read_observation(
                view,
                &observed,
                returned_contract,
                safe_failure_contract,
                requires_fact_selection_attestation,
            ) {
                Ok(committed) => match callbacks.settle_read(frame, committed.outcome()) {
                    Ok(QualifiedEvidenceVerdict::Settlement(settlement)) => {
                        ObservationVerdict::Settlement((
                            request_ref,
                            committed.observation_ref().clone(),
                            settlement,
                        ))
                    }
                    Ok(QualifiedEvidenceVerdict::InsufficientEvidence) => {
                        ObservationVerdict::InsufficientEvidence
                    }
                    Ok(QualifiedEvidenceVerdict::InvalidEvidence) => {
                        ObservationVerdict::InvalidEvidence
                    }
                    Err(error) => return Err(RuntimeError::CandidateCertification(error)),
                },
                Err(_) => ObservationVerdict::InvalidEvidence,
            };
            Ok(verdict)
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(scan_observations(verdicts))
}

fn terminal_consumed_observations(
    view: &VerifiedRunView,
) -> Result<BTreeMap<NodeId, mfm_journal::v2::ObservationRef>> {
    let mut consumed = BTreeMap::new();
    for entry in view.transition_entries() {
        let fields = entry.transition().fields()?;
        let Some(observation_ref) = fields.body.consumed_observation_ref()? else {
            continue;
        };
        if consumed.insert(fields.node_id, observation_ref).is_some() {
            return Err(RuntimeError::InvalidCallbackResult);
        }
    }
    Ok(consumed)
}

fn effect_intent(view: &VerifiedRunView, node: &CertifiedNodeContract) -> Result<EffectIntent> {
    let mut found = None;
    for entry in view.transition_entries() {
        let transition = entry.transition().fields()?;
        if transition.node_id != *node.node_id() {
            continue;
        }
        if let TransitionBodyFields::EffectRequested {
            input_manifest_ref,
            effect_key,
            semantic_request_ref,
            request_digest,
            executor_binding_ref,
        } = transition.body.fields()?
        {
            if found.is_some() {
                return Err(RuntimeError::EffectIdentityMismatch);
            }
            found = Some(EffectIntent {
                request_transition_ref: entry.transition_ref().clone(),
                input_manifest_ref,
                effect_key,
                semantic_request_ref,
                request_digest,
                executor_binding_ref,
            });
        }
    }
    found.ok_or(RuntimeError::EffectIdentityMismatch)
}

fn validate_effect_history(
    history: &VerifiedNodeAccessHistory<'_>,
    intent: &EffectIntent,
) -> Result<()> {
    for attempt in history.entries() {
        let authorization = attempt.authorization().fields()?;
        let AuthorizationScopeFields::EnsureEffect {
            effect_request_transition_ref,
        } = authorization.scope.fields()?
        else {
            return Err(RuntimeError::EffectIdentityMismatch);
        };
        if effect_request_transition_ref != intent.request_transition_ref
            || authorization.capability_binding_ref != intent.executor_binding_ref
            || authorization.request_ref != intent.semantic_request_ref
            || authorization.frozen_read_intent_ref.is_some()
        {
            return Err(RuntimeError::EffectIdentityMismatch);
        }
    }
    Ok(())
}

fn scan_effect_suffix(
    view: &VerifiedRunView,
    history: &VerifiedNodeAccessHistory<'_>,
    callbacks: &QualifiedCandidateStateCallbacks<'_>,
    frame: &VerifiedStateFrameMaterial,
    intent: &EffectIntent,
    ensure_result_contract: &RetainedValueContract,
    terminal_evidence_contract: &RetainedValueContract,
) -> Result<EvidenceScan<(mfm_journal::v2::ObservationRef, QualifiedSettlement)>> {
    let verdicts = history
        .observed_suffix()
        .map(|observed| {
            if let Some(disposition) = observed_non_domain_disposition(&observed)? {
                return Ok(match disposition {
                    NonDomainDisposition::RetryableOperational => {
                        ObservationVerdict::InsufficientEvidence
                    }
                    NonDomainDisposition::IntegrityBlocked => ObservationVerdict::InvalidEvidence,
                });
            }
            let verdict = match committed_terminal_observation(
                view,
                &observed,
                &intent.executor_binding_ref,
                &intent.effect_key,
                &intent.request_digest,
                ensure_result_contract,
                terminal_evidence_contract,
            ) {
                Ok(Some(committed)) => {
                    let audit = observed.audit();
                    let resolver = CommittedEffectResolver::new(view, audit.authorization_ref());
                    let terminal = VerifiedTerminalEffectView::new(
                        committed.outcome().evidence(),
                        committed.outcome().evidence_ref(),
                        &resolver,
                    );
                    match callbacks.settle_effect(frame, terminal) {
                        Ok(QualifiedEvidenceVerdict::Settlement(settlement)) => {
                            ObservationVerdict::Settlement((
                                committed.observation_ref().clone(),
                                settlement,
                            ))
                        }
                        Ok(QualifiedEvidenceVerdict::InsufficientEvidence) => {
                            ObservationVerdict::InsufficientEvidence
                        }
                        Ok(QualifiedEvidenceVerdict::InvalidEvidence) => {
                            ObservationVerdict::InvalidEvidence
                        }
                        Err(error) => return Err(RuntimeError::CandidateCertification(error)),
                    }
                }
                Ok(None) => ObservationVerdict::InsufficientEvidence,
                Err(_) => ObservationVerdict::InvalidEvidence,
            };
            Ok(verdict)
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(scan_observations(verdicts))
}

fn observed_non_domain_disposition(
    observed: &mfm_store::VerifiedObservedAccess<'_>,
) -> Result<Option<NonDomainDisposition>> {
    observed
        .audit()
        .non_domain_failure()
        .map(|failure| failure.fields().disposition)
        .map_or(Ok(None), |disposition| Ok(Some(disposition)))
}

fn qualify_fact_selection_request(
    view: &VerifiedRunView,
    node: &CertifiedNodeContract,
    binding_ref: &ContentRef,
    request_contract: &RetainedValueContract,
    returned_contract: &RetainedValueContract,
    authored: QualifiedAuthoredRequest,
    baseline: Option<&FrozenReadBaseline>,
) -> Result<(FactSelectionRequest, ProposedValueMaterial, ContentRef)> {
    if authored.request_type() != std::any::TypeId::of::<FactSelectionRequest>() {
        return Err(RuntimeError::InvalidCallbackResult);
    }
    let mut admitted = view
        .capability_binding_manifest()
        .entries()
        .iter()
        .filter(|entry| entry.operation_id.as_str() == FACT_SELECTION_OPERATION_ID);
    let manifest_entry = admitted.next().ok_or(RuntimeError::CatalogSelection)?;
    if admitted.next().is_some() || manifest_entry.binding_ref != *binding_ref {
        return Err(RuntimeError::CatalogSelection);
    }
    let binding = ReadCapabilityBinding::strict_decode(view.retained_content(binding_ref)?)?;
    let binding_fields = binding.fields()?;
    let scan_contract = FactSelectionScanContract::strict_decode(
        view.retained_content(&binding_fields.capability_contract_ref)?,
    )?;
    let scan = scan_contract.fields()?;
    if scan.capability_operation_id.as_str() != FACT_SELECTION_OPERATION_ID
        || scan.request_contract != *request_contract
        || scan.response_contract != *returned_contract
        || !matches!(
            node.execution(),
            CertifiedStateExecution::Read {
                capability_operation_id,
                capability_binding_ref,
                request_contract: node_request,
                returned_contract: node_returned,
                ..
            } if capability_operation_id.as_str() == FACT_SELECTION_OPERATION_ID
                && capability_binding_ref == binding_ref
                && node_request == request_contract
                && node_returned == returned_contract
        )
    {
        return Err(RuntimeError::CatalogSelection);
    }
    view.retained_content(&binding_fields.routing_catalog_ref)?;
    let proposed = authored.proposed().clone();
    let request = authored
        .into_value()
        .downcast::<FactSelectionRequest>()
        .map_err(|_| RuntimeError::InvalidCallbackResult)?;
    if request.canonical_json() != proposed.canonical().as_bytes()
        || request.content_ref()? != *proposed.content_ref()
    {
        return Err(RuntimeError::InvalidCallbackResult);
    }
    if baseline.is_some_and(|baseline| {
        baseline.request_bytes.as_bytes() != proposed.canonical().as_bytes()
            || baseline.routing_generation_ref != binding_fields.routing_catalog_ref
    }) {
        return Err(RuntimeError::AuthorityMismatch);
    }
    Ok((*request, proposed, binding_fields.routing_catalog_ref))
}

fn routed_request_matches(
    request: &RoutedReadRequest,
    baseline: &FrozenReadBaseline,
) -> Result<bool> {
    Ok(
        request.proposed().canonical().as_bytes() == baseline.request_bytes.as_bytes()
            && request.routing_generation_ref() == &baseline.routing_generation_ref
            && proposed_matches_value_ref(request.proposed(), &baseline.request_ref)?,
    )
}

fn proposed_matches_value_ref(
    proposed: &ProposedValueMaterial,
    value_ref: &ValueRef,
) -> Result<bool> {
    let fields = value_ref.fields()?;
    let byte_length = u64::try_from(proposed.canonical().as_bytes().len())
        .map_err(|_| RuntimeError::InvalidCallbackResult)?;
    Ok(&fields.schema_id == proposed.content_ref().schema_id()
        && &fields.content_digest == proposed.content_ref().content_digest()
        && fields.byte_length == byte_length)
}

fn verified_retained_value(
    view: &VerifiedRunView,
    value_ref: &ValueRef,
    contract: &RetainedValueContract,
) -> Result<VerifiedValueMaterial> {
    value_ref.validate_contract(contract)?;
    let retained = view.retained_value(value_ref)?;
    Ok(VerifiedValueMaterial::new(
        PlainCanonicalJsonBytes::from_canonical_json_slice(retained.bytes())?,
        value_ref.clone(),
    ))
}

fn prepared_authorization_request_ref(append: &PreparedJournalAppend) -> Result<ValueRef> {
    let PreparedJournalAppend::AuthorizeExternalAccess(append) = append else {
        return Err(RuntimeError::InvalidCallbackResult);
    };
    Ok(append.authorization().fields()?.request_ref)
}

fn transition_append_outcome(outcome: AppendOutcome) -> Result<ActionResult> {
    match outcome {
        AppendOutcome::NewlyAppended(NewlyAppended::Transition(committed))
        | AppendOutcome::AlreadyCommitted(committed) => Ok(ActionResult::Outcome(advanced(
            committed.journal_head().clone(),
        ))),
        AppendOutcome::Rejected(AppendRejection::StaleHead { .. } | AppendRejection::RunClosed) => {
            Ok(ActionResult::Retry)
        }
        AppendOutcome::Rejected(rejection) => Err(rejection_error(rejection)),
        AppendOutcome::OutcomeUnknown => Err(RuntimeError::OutcomeUnknown),
        AppendOutcome::NewlyAppended(_) => Err(RuntimeError::InvalidCallbackResult),
    }
}

fn rejection_error(rejection: AppendRejection) -> RuntimeError {
    RuntimeError::Store(match rejection {
        AppendRejection::StaleHead { expected, actual } => mfm_store::StoreError::HeadMismatch {
            expected: Box::new(expected),
            actual,
        },
        AppendRejection::AppendRequestConflict => mfm_store::StoreError::AppendRequestConflict,
        AppendRejection::AdmissionConflict => mfm_store::StoreError::AdmissionConflict,
        AppendRejection::RunClosed => mfm_store::StoreError::RunClosed,
    })
}

fn integrity(certified_order: usize, observation_order: usize) -> IntegrityBlock<IntegrityFinding> {
    IntegrityBlock {
        certified_order,
        observation_order,
        detail: IntegrityFinding::InvalidEvidence,
    }
}

fn advanced(journal_head: mfm_journal::v2::JournalHead) -> DriveOutcome {
    DriveOutcome::Advanced { journal_head }
}

fn waiting(view: &VerifiedRunView, reason: DriveWaitReason) -> DriveOutcome {
    waiting_at_head(view.journal_head().clone(), reason)
}

fn candidate_failure_outcome(
    view: &VerifiedRunView,
    error: CandidateCertificationError,
) -> Result<DriveOutcome> {
    candidate_failure_at_head(view.journal_head().clone(), error)
}

fn candidate_failure_at_head(
    journal_head: mfm_journal::v2::JournalHead,
    error: CandidateCertificationError,
) -> Result<DriveOutcome> {
    match error.kind() {
        CandidateCertificationErrorKind::Unavailable => Ok(waiting_at_head(
            journal_head,
            DriveWaitReason::OperationalBlock,
        )),
        CandidateCertificationErrorKind::IntegrityFailed => Ok(waiting_at_head(
            journal_head,
            DriveWaitReason::IntegrityBlock,
        )),
        CandidateCertificationErrorKind::ExecutionFailed => {
            Err(RuntimeError::CandidateCertification(error))
        }
    }
}

fn waiting_at_head(
    journal_head: mfm_journal::v2::JournalHead,
    reason: DriveWaitReason,
) -> DriveOutcome {
    DriveOutcome::Waiting {
        journal_head,
        reason,
    }
}

fn closure_ref(view: &VerifiedRunView) -> Result<ClosureRef> {
    view.transition_entries()
        .find_map(|entry| entry.closure_ref())
        .cloned()
        .ok_or(RuntimeError::InvalidCallbackResult)
}

#[cfg(test)]
mod observation_retry_tests {
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    use mfm_store::{
        open_in_memory,
        test_support::{FactScanConformanceFixture, LegalAdmissionFixture},
        ObservationMaterial,
    };

    use crate::access_protocol::{FactSelection, ObservationCommitProbe, PendingObservation};
    use crate::RuntimeError;

    use super::{
        commit_observation_loop, FactSelectionObservation, FactSelectionObservationOutcome,
        ObservationAppendAttempt, ObservationCommitDriver, ObservationLogicalResolution,
        ObservationPhysicalResolution, ObservationPreparation, ObservationRetry,
        ObservationRetryBackoff,
    };

    #[derive(Clone, PartialEq, Eq)]
    struct TestLogicalIdentity {
        id: u64,
        expected_authorization: bool,
    }

    struct TestView {
        head: u64,
    }

    struct TestPreparedAppend {
        physical_id: u64,
    }

    enum LogicalOutcome {
        Absent,
        Identical {
            observation_ref: u64,
            journal_head: u64,
        },
        Conflict,
        Retry,
    }

    enum PhysicalOutcome {
        AbsentAtCurrentHead,
        StalePredecessor,
        Occupied,
        Retry,
    }

    enum PrepareOutcome {
        Prepared {
            logical_id: u64,
            expected_authorization: bool,
            physical_id: u64,
        },
        AlreadyCommitted,
        Retry,
    }

    enum AppendOutcome {
        Progress,
        StalePredecessor,
        Conflict,
        Retry,
    }

    enum WriterEvent {
        Load(std::result::Result<u64, ()>),
        ResolveLogical {
            expected_id: u64,
            outcome: LogicalOutcome,
        },
        ResolvePhysical {
            expected_id: u64,
            outcome: PhysicalOutcome,
        },
        Prepare {
            expected_physical_id: Option<u64>,
            expected_material: u64,
            outcome: PrepareOutcome,
        },
        Append {
            expected_physical_id: u64,
            outcome: AppendOutcome,
        },
    }

    struct InjectedWriter<Material = u64> {
        events: VecDeque<WriterEvent>,
        load_count: usize,
        prepared_physical_ids: Vec<Option<u64>>,
        appended_physical_ids: Vec<u64>,
        assert_material: fn(&Material, u64),
    }

    impl InjectedWriter<u64> {
        fn new(events: impl IntoIterator<Item = WriterEvent>) -> Self {
            Self {
                events: events.into_iter().collect(),
                load_count: 0,
                prepared_physical_ids: Vec::new(),
                appended_physical_ids: Vec::new(),
                assert_material: |material, expected| assert_eq!(*material, expected),
            }
        }
    }

    impl<Material> InjectedWriter<Material> {
        fn with_material_assertion(
            events: impl IntoIterator<Item = WriterEvent>,
            assert_material: fn(&Material, u64),
        ) -> Self {
            Self {
                events: events.into_iter().collect(),
                load_count: 0,
                prepared_physical_ids: Vec::new(),
                appended_physical_ids: Vec::new(),
                assert_material,
            }
        }

        fn next_event(&mut self) -> WriterEvent {
            self.events
                .pop_front()
                .expect("the actual observation loop made an unexpected writer call")
        }

        fn assert_exhausted(&self) {
            assert!(
                self.events.is_empty(),
                "the actual observation loop skipped an expected writer call"
            );
        }
    }

    impl<Material> ObservationCommitDriver for InjectedWriter<Material> {
        type View = TestView;
        type Head = u64;
        type LogicalIdentity = TestLogicalIdentity;
        type PhysicalIdentity = u64;
        type PreparedAppend = TestPreparedAppend;
        type ObservationRef = u64;
        type Material = Material;

        async fn load_verified(&mut self) -> std::result::Result<Self::View, ()> {
            self.load_count += 1;
            let WriterEvent::Load(result) = self.next_event() else {
                panic!("the observation loop loaded where another writer operation was expected")
            };
            result.map(|head| TestView { head })
        }

        fn head<'a>(&self, view: &'a Self::View) -> &'a Self::Head {
            &view.head
        }

        fn resolve_logical(
            &mut self,
            _view: &Self::View,
            logical: &Self::LogicalIdentity,
        ) -> ObservationLogicalResolution<Self::ObservationRef, Self::Head> {
            let WriterEvent::ResolveLogical {
                expected_id,
                outcome,
            } = self.next_event()
            else {
                panic!("the observation loop skipped an expected logical resolution")
            };
            assert_eq!(logical.id, expected_id);
            match outcome {
                LogicalOutcome::Absent => ObservationLogicalResolution::Absent,
                LogicalOutcome::Identical {
                    observation_ref,
                    journal_head,
                } => ObservationLogicalResolution::Identical {
                    observation_ref,
                    journal_head,
                },
                LogicalOutcome::Conflict => ObservationLogicalResolution::Conflict,
                LogicalOutcome::Retry => ObservationLogicalResolution::Retry,
            }
        }

        fn resolve_physical(
            &mut self,
            _view: &Self::View,
            physical: &Self::PhysicalIdentity,
        ) -> ObservationPhysicalResolution {
            let WriterEvent::ResolvePhysical {
                expected_id,
                outcome,
            } = self.next_event()
            else {
                panic!("the observation loop skipped an expected physical resolution")
            };
            assert_eq!(physical, &expected_id);
            match outcome {
                PhysicalOutcome::AbsentAtCurrentHead => {
                    ObservationPhysicalResolution::AbsentAtCurrentHead
                }
                PhysicalOutcome::StalePredecessor => {
                    ObservationPhysicalResolution::StalePredecessor
                }
                PhysicalOutcome::Occupied => ObservationPhysicalResolution::Occupied,
                PhysicalOutcome::Retry => ObservationPhysicalResolution::Retry,
            }
        }

        fn prepare(
            &mut self,
            _view: &Self::View,
            physical: Option<&Self::PhysicalIdentity>,
            material: Self::Material,
        ) -> ObservationPreparation<
            Self::PreparedAppend,
            Self::LogicalIdentity,
            Self::PhysicalIdentity,
        > {
            let WriterEvent::Prepare {
                expected_physical_id,
                expected_material,
                outcome,
            } = self.next_event()
            else {
                panic!("the observation loop skipped an expected preparation")
            };
            assert_eq!(physical.copied(), expected_physical_id);
            (self.assert_material)(&material, expected_material);
            self.prepared_physical_ids.push(physical.copied());
            match outcome {
                PrepareOutcome::Prepared {
                    logical_id,
                    expected_authorization,
                    physical_id,
                } => ObservationPreparation::Prepared {
                    append: TestPreparedAppend { physical_id },
                    logical: TestLogicalIdentity {
                        id: logical_id,
                        expected_authorization,
                    },
                    physical: physical_id,
                },
                PrepareOutcome::AlreadyCommitted => ObservationPreparation::AlreadyCommitted,
                PrepareOutcome::Retry => ObservationPreparation::Retry,
            }
        }

        fn logical_is_expected(&self, logical: &Self::LogicalIdentity) -> bool {
            logical.expected_authorization
        }

        async fn append(&mut self, append: Self::PreparedAppend) -> ObservationAppendAttempt {
            let WriterEvent::Append {
                expected_physical_id,
                outcome,
            } = self.next_event()
            else {
                panic!("the observation loop skipped an expected append")
            };
            assert_eq!(append.physical_id, expected_physical_id);
            self.appended_physical_ids.push(append.physical_id);
            match outcome {
                AppendOutcome::Progress => ObservationAppendAttempt::Progress,
                AppendOutcome::StalePredecessor => ObservationAppendAttempt::StalePredecessor,
                AppendOutcome::Conflict => ObservationAppendAttempt::Conflict,
                AppendOutcome::Retry => ObservationAppendAttempt::Retry,
            }
        }
    }

    struct RecordingRetry {
        backoff: ObservationRetryBackoff,
        delays: Vec<Duration>,
        resets: usize,
    }

    impl RecordingRetry {
        const fn new() -> Self {
            Self {
                backoff: ObservationRetryBackoff::new(),
                delays: Vec::new(),
                resets: 0,
            }
        }
    }

    impl ObservationRetry for RecordingRetry {
        fn reset(&mut self) {
            self.resets += 1;
            self.backoff.reset();
        }

        async fn wait(&mut self) {
            self.delays.push(self.backoff.take_delay());
            tokio::task::yield_now().await;
        }
    }

    struct NeverCompletingRetry {
        entered: Arc<AtomicBool>,
        completed: Arc<AtomicBool>,
    }

    impl ObservationRetry for NeverCompletingRetry {
        fn reset(&mut self) {}

        async fn wait(&mut self) {
            self.entered.store(true, Ordering::SeqCst);
            std::future::pending::<()>().await;
            self.completed.store(true, Ordering::SeqCst);
        }
    }

    fn pending_after_boundary(
        boundary_invocations: &AtomicUsize,
    ) -> PendingObservation<ObservationCommitProbe, TestLogicalIdentity, u64> {
        boundary_invocations.fetch_add(1, Ordering::SeqCst);
        PendingObservation::new(71)
    }

    fn logical(id: u64) -> TestLogicalIdentity {
        TestLogicalIdentity {
            id,
            expected_authorization: true,
        }
    }

    fn prepared(logical_id: u64, physical_id: u64) -> PrepareOutcome {
        PrepareOutcome::Prepared {
            logical_id,
            expected_authorization: true,
            physical_id,
        }
    }

    #[test]
    fn retry_backoff_is_capped_and_resets_after_progress() {
        let mut backoff = ObservationRetryBackoff::new();
        let delays = (0..10).map(|_| backoff.take_delay()).collect::<Vec<_>>();
        assert_eq!(
            delays,
            [10, 20, 40, 80, 160, 320, 640, 1_000, 1_000, 1_000].map(Duration::from_millis)
        );

        backoff.reset();
        assert_eq!(backoff.take_delay(), Duration::from_millis(10));
    }

    #[tokio::test]
    async fn actual_loop_rebases_a_stale_predecessor_without_reinvoking_the_boundary() {
        let boundary_invocations = AtomicUsize::new(0);
        let mut pending = pending_after_boundary(&boundary_invocations);
        let materializations = AtomicUsize::new(0);
        let mut writer = InjectedWriter::new([
            WriterEvent::Load(Ok(1)),
            WriterEvent::Prepare {
                expected_physical_id: None,
                expected_material: 71,
                outcome: prepared(1, 10),
            },
            WriterEvent::Load(Ok(2)),
            WriterEvent::ResolveLogical {
                expected_id: 1,
                outcome: LogicalOutcome::Absent,
            },
            WriterEvent::ResolvePhysical {
                expected_id: 10,
                outcome: PhysicalOutcome::StalePredecessor,
            },
            WriterEvent::Load(Ok(2)),
            WriterEvent::ResolveLogical {
                expected_id: 1,
                outcome: LogicalOutcome::Absent,
            },
            WriterEvent::Prepare {
                expected_physical_id: None,
                expected_material: 71,
                outcome: prepared(1, 11),
            },
            WriterEvent::Load(Ok(2)),
            WriterEvent::ResolveLogical {
                expected_id: 1,
                outcome: LogicalOutcome::Absent,
            },
            WriterEvent::ResolvePhysical {
                expected_id: 11,
                outcome: PhysicalOutcome::AbsentAtCurrentHead,
            },
            WriterEvent::Prepare {
                expected_physical_id: Some(11),
                expected_material: 71,
                outcome: prepared(1, 11),
            },
            WriterEvent::Append {
                expected_physical_id: 11,
                outcome: AppendOutcome::Progress,
            },
            WriterEvent::Load(Ok(3)),
            WriterEvent::ResolveLogical {
                expected_id: 1,
                outcome: LogicalOutcome::Identical {
                    observation_ref: 91,
                    journal_head: 3,
                },
            },
        ]);
        let mut retry = RecordingRetry::new();

        let committed = commit_observation_loop(
            &mut writer,
            &mut pending,
            |material| {
                materializations.fetch_add(1, Ordering::SeqCst);
                *material
            },
            &mut retry,
        )
        .await
        .expect("rebased observation");

        assert_eq!(committed.observation_ref(), &91);
        assert_eq!(committed.outcome(), &3);
        assert_eq!(boundary_invocations.load(Ordering::SeqCst), 1);
        assert_eq!(materializations.load(Ordering::SeqCst), 3);
        assert_eq!(writer.prepared_physical_ids, [None, None, Some(11)]);
        assert_eq!(writer.appended_physical_ids, [11]);
        assert_eq!(retry.resets, 4);
        writer.assert_exhausted();
    }

    #[tokio::test]
    async fn actual_loop_retries_the_same_physical_candidate_across_ack_ambiguity() {
        let read_callback_invocations = AtomicUsize::new(0);
        let mut read_pending = pending_after_boundary(&read_callback_invocations);
        let mut before_commit_writer = InjectedWriter::new([
            WriterEvent::Load(Ok(1)),
            WriterEvent::Prepare {
                expected_physical_id: None,
                expected_material: 71,
                outcome: prepared(1, 20),
            },
            WriterEvent::Load(Ok(1)),
            WriterEvent::ResolveLogical {
                expected_id: 1,
                outcome: LogicalOutcome::Absent,
            },
            WriterEvent::ResolvePhysical {
                expected_id: 20,
                outcome: PhysicalOutcome::AbsentAtCurrentHead,
            },
            WriterEvent::Prepare {
                expected_physical_id: Some(20),
                expected_material: 71,
                outcome: prepared(1, 20),
            },
            WriterEvent::Append {
                expected_physical_id: 20,
                outcome: AppendOutcome::Retry,
            },
            WriterEvent::Load(Ok(1)),
            WriterEvent::ResolveLogical {
                expected_id: 1,
                outcome: LogicalOutcome::Absent,
            },
            WriterEvent::ResolvePhysical {
                expected_id: 20,
                outcome: PhysicalOutcome::AbsentAtCurrentHead,
            },
            WriterEvent::Prepare {
                expected_physical_id: Some(20),
                expected_material: 71,
                outcome: prepared(1, 20),
            },
            WriterEvent::Append {
                expected_physical_id: 20,
                outcome: AppendOutcome::Progress,
            },
            WriterEvent::Load(Ok(2)),
            WriterEvent::ResolveLogical {
                expected_id: 1,
                outcome: LogicalOutcome::Identical {
                    observation_ref: 92,
                    journal_head: 2,
                },
            },
        ]);
        let mut before_commit_retry = RecordingRetry::new();
        commit_observation_loop(
            &mut before_commit_writer,
            &mut read_pending,
            |material| *material,
            &mut before_commit_retry,
        )
        .await
        .expect("retry absent pre-commit candidate");
        assert_eq!(before_commit_writer.appended_physical_ids, [20, 20]);
        assert_eq!(
            before_commit_writer.prepared_physical_ids,
            [None, Some(20), Some(20)]
        );
        assert_eq!(before_commit_retry.delays, [Duration::from_millis(10)]);
        assert_eq!(read_callback_invocations.load(Ordering::SeqCst), 1);
        before_commit_writer.assert_exhausted();

        let effect_callback_invocations = AtomicUsize::new(0);
        let mut effect_pending = pending_after_boundary(&effect_callback_invocations);
        let mut after_commit_writer = InjectedWriter::new([
            WriterEvent::Load(Ok(1)),
            WriterEvent::Prepare {
                expected_physical_id: None,
                expected_material: 71,
                outcome: prepared(2, 30),
            },
            WriterEvent::Load(Ok(1)),
            WriterEvent::ResolveLogical {
                expected_id: 2,
                outcome: LogicalOutcome::Absent,
            },
            WriterEvent::ResolvePhysical {
                expected_id: 30,
                outcome: PhysicalOutcome::AbsentAtCurrentHead,
            },
            WriterEvent::Prepare {
                expected_physical_id: Some(30),
                expected_material: 71,
                outcome: prepared(2, 30),
            },
            WriterEvent::Append {
                expected_physical_id: 30,
                outcome: AppendOutcome::Retry,
            },
            WriterEvent::Load(Ok(2)),
            WriterEvent::ResolveLogical {
                expected_id: 2,
                outcome: LogicalOutcome::Identical {
                    observation_ref: 93,
                    journal_head: 2,
                },
            },
        ]);
        let mut after_commit_retry = RecordingRetry::new();
        commit_observation_loop(
            &mut after_commit_writer,
            &mut effect_pending,
            |material| *material,
            &mut after_commit_retry,
        )
        .await
        .expect("resolve committed acknowledgement loss");
        assert_eq!(after_commit_writer.appended_physical_ids, [30]);
        assert_eq!(after_commit_writer.prepared_physical_ids, [None, Some(30)]);
        assert_eq!(after_commit_retry.delays, [Duration::from_millis(10)]);
        assert_eq!(effect_callback_invocations.load(Ordering::SeqCst), 1);
        after_commit_writer.assert_exhausted();
    }

    #[tokio::test]
    async fn actual_loop_distinguishes_logical_missing_identical_and_conflict() {
        let mut identical_pending =
            PendingObservation::<ObservationCommitProbe, TestLogicalIdentity, u64>::new(71);
        identical_pending
            .remember_prepared(logical(5), 50)
            .expect("seed identical pending identity");
        let mut identical_writer = InjectedWriter::new([
            WriterEvent::Load(Ok(2)),
            WriterEvent::ResolveLogical {
                expected_id: 5,
                outcome: LogicalOutcome::Identical {
                    observation_ref: 95,
                    journal_head: 2,
                },
            },
        ]);
        let mut identical_retry = RecordingRetry::new();
        let identical = commit_observation_loop(
            &mut identical_writer,
            &mut identical_pending,
            |material| *material,
            &mut identical_retry,
        )
        .await
        .expect("identical logical observation");
        assert_eq!(identical.observation_ref(), &95);
        assert_eq!(identical.outcome(), &2);
        identical_writer.assert_exhausted();

        let mut missing_pending =
            PendingObservation::<ObservationCommitProbe, TestLogicalIdentity, u64>::new(71);
        missing_pending
            .remember_prepared(logical(6), 60)
            .expect("seed missing pending identity");
        let mut missing_writer = InjectedWriter::new([
            WriterEvent::Load(Ok(2)),
            WriterEvent::ResolveLogical {
                expected_id: 6,
                outcome: LogicalOutcome::Absent,
            },
            WriterEvent::ResolvePhysical {
                expected_id: 60,
                outcome: PhysicalOutcome::AbsentAtCurrentHead,
            },
            WriterEvent::Prepare {
                expected_physical_id: Some(60),
                expected_material: 71,
                outcome: prepared(6, 60),
            },
            WriterEvent::Append {
                expected_physical_id: 60,
                outcome: AppendOutcome::Progress,
            },
            WriterEvent::Load(Ok(3)),
            WriterEvent::ResolveLogical {
                expected_id: 6,
                outcome: LogicalOutcome::Identical {
                    observation_ref: 96,
                    journal_head: 3,
                },
            },
        ]);
        let mut missing_retry = RecordingRetry::new();
        commit_observation_loop(
            &mut missing_writer,
            &mut missing_pending,
            |material| *material,
            &mut missing_retry,
        )
        .await
        .expect("missing logical observation is appended");
        assert_eq!(missing_writer.appended_physical_ids, [60]);
        missing_writer.assert_exhausted();

        let mut conflict_pending =
            PendingObservation::<ObservationCommitProbe, TestLogicalIdentity, u64>::new(71);
        conflict_pending
            .remember_prepared(logical(7), 70)
            .expect("seed conflicting pending identity");
        let mut conflict_writer = InjectedWriter::new([
            WriterEvent::Load(Ok(2)),
            WriterEvent::ResolveLogical {
                expected_id: 7,
                outcome: LogicalOutcome::Conflict,
            },
        ]);
        let mut conflict_retry = RecordingRetry::new();
        assert!(matches!(
            commit_observation_loop(
                &mut conflict_writer,
                &mut conflict_pending,
                |material| *material,
                &mut conflict_retry,
            )
            .await,
            Err(RuntimeError::ObservationConflict)
        ));
        assert!(conflict_writer.appended_physical_ids.is_empty());
        conflict_writer.assert_exhausted();
    }

    #[tokio::test]
    async fn actual_loop_caps_backoff_and_resets_it_after_commit_progress() {
        let mut events = (0..10)
            .map(|_| WriterEvent::Load(Err(())))
            .collect::<Vec<_>>();
        events.extend([
            WriterEvent::Load(Ok(1)),
            WriterEvent::Prepare {
                expected_physical_id: None,
                expected_material: 71,
                outcome: prepared(8, 80),
            },
            WriterEvent::Load(Ok(1)),
            WriterEvent::ResolveLogical {
                expected_id: 8,
                outcome: LogicalOutcome::Absent,
            },
            WriterEvent::ResolvePhysical {
                expected_id: 80,
                outcome: PhysicalOutcome::AbsentAtCurrentHead,
            },
            WriterEvent::Prepare {
                expected_physical_id: Some(80),
                expected_material: 71,
                outcome: prepared(8, 80),
            },
            WriterEvent::Append {
                expected_physical_id: 80,
                outcome: AppendOutcome::Progress,
            },
            WriterEvent::Load(Err(())),
            WriterEvent::Load(Ok(2)),
            WriterEvent::ResolveLogical {
                expected_id: 8,
                outcome: LogicalOutcome::Identical {
                    observation_ref: 98,
                    journal_head: 2,
                },
            },
        ]);
        let mut writer = InjectedWriter::new(events);
        let mut pending =
            PendingObservation::<ObservationCommitProbe, TestLogicalIdentity, u64>::new(71);
        let mut retry = RecordingRetry::new();
        commit_observation_loop(&mut writer, &mut pending, |material| *material, &mut retry)
            .await
            .expect("eventual observation");

        assert_eq!(
            retry.delays,
            [10, 20, 40, 80, 160, 320, 640, 1_000, 1_000, 1_000, 10,].map(Duration::from_millis)
        );
        assert_eq!(retry.resets, 2);
        writer.assert_exhausted();
    }

    #[tokio::test]
    async fn actual_loop_classifies_transient_and_conflicting_writer_results() {
        let mut pending =
            PendingObservation::<ObservationCommitProbe, TestLogicalIdentity, u64>::new(71);
        pending
            .remember_prepared(logical(10), 100)
            .expect("seed pending identities");
        let mut writer = InjectedWriter::new([
            WriterEvent::Load(Ok(1)),
            WriterEvent::ResolveLogical {
                expected_id: 10,
                outcome: LogicalOutcome::Retry,
            },
            WriterEvent::Load(Ok(1)),
            WriterEvent::ResolveLogical {
                expected_id: 10,
                outcome: LogicalOutcome::Absent,
            },
            WriterEvent::ResolvePhysical {
                expected_id: 100,
                outcome: PhysicalOutcome::Retry,
            },
            WriterEvent::Load(Ok(1)),
            WriterEvent::ResolveLogical {
                expected_id: 10,
                outcome: LogicalOutcome::Absent,
            },
            WriterEvent::ResolvePhysical {
                expected_id: 100,
                outcome: PhysicalOutcome::AbsentAtCurrentHead,
            },
            WriterEvent::Prepare {
                expected_physical_id: Some(100),
                expected_material: 71,
                outcome: PrepareOutcome::Retry,
            },
            WriterEvent::Load(Ok(1)),
            WriterEvent::ResolveLogical {
                expected_id: 10,
                outcome: LogicalOutcome::Absent,
            },
            WriterEvent::ResolvePhysical {
                expected_id: 100,
                outcome: PhysicalOutcome::AbsentAtCurrentHead,
            },
            WriterEvent::Prepare {
                expected_physical_id: Some(100),
                expected_material: 71,
                outcome: PrepareOutcome::AlreadyCommitted,
            },
            WriterEvent::Load(Ok(1)),
            WriterEvent::ResolveLogical {
                expected_id: 10,
                outcome: LogicalOutcome::Absent,
            },
            WriterEvent::ResolvePhysical {
                expected_id: 100,
                outcome: PhysicalOutcome::AbsentAtCurrentHead,
            },
            WriterEvent::Prepare {
                expected_physical_id: Some(100),
                expected_material: 71,
                outcome: prepared(10, 100),
            },
            WriterEvent::Append {
                expected_physical_id: 100,
                outcome: AppendOutcome::StalePredecessor,
            },
            WriterEvent::Load(Ok(2)),
            WriterEvent::ResolveLogical {
                expected_id: 10,
                outcome: LogicalOutcome::Absent,
            },
            WriterEvent::Prepare {
                expected_physical_id: None,
                expected_material: 71,
                outcome: prepared(10, 101),
            },
            WriterEvent::Load(Ok(2)),
            WriterEvent::ResolveLogical {
                expected_id: 10,
                outcome: LogicalOutcome::Absent,
            },
            WriterEvent::ResolvePhysical {
                expected_id: 101,
                outcome: PhysicalOutcome::AbsentAtCurrentHead,
            },
            WriterEvent::Prepare {
                expected_physical_id: Some(101),
                expected_material: 71,
                outcome: prepared(10, 101),
            },
            WriterEvent::Append {
                expected_physical_id: 101,
                outcome: AppendOutcome::Conflict,
            },
        ]);
        let mut retry = RecordingRetry::new();
        assert!(matches!(
            commit_observation_loop(&mut writer, &mut pending, |material| *material, &mut retry,)
                .await,
            Err(RuntimeError::ObservationConflict)
        ));
        assert_eq!(retry.delays, [10, 20, 40].map(Duration::from_millis));
        assert_eq!(writer.appended_physical_ids, [100, 101]);
        writer.assert_exhausted();

        let mut occupied_pending =
            PendingObservation::<ObservationCommitProbe, TestLogicalIdentity, u64>::new(71);
        occupied_pending
            .remember_prepared(logical(11), 110)
            .expect("seed occupied physical identity");
        let mut occupied_writer = InjectedWriter::new([
            WriterEvent::Load(Ok(1)),
            WriterEvent::ResolveLogical {
                expected_id: 11,
                outcome: LogicalOutcome::Absent,
            },
            WriterEvent::ResolvePhysical {
                expected_id: 110,
                outcome: PhysicalOutcome::Occupied,
            },
        ]);
        let mut occupied_retry = RecordingRetry::new();
        assert!(matches!(
            commit_observation_loop(
                &mut occupied_writer,
                &mut occupied_pending,
                |material| *material,
                &mut occupied_retry,
            )
            .await,
            Err(RuntimeError::ObservationConflict)
        ));
        occupied_writer.assert_exhausted();
    }

    #[tokio::test]
    async fn cancelling_the_actual_loop_drops_only_the_pending_retry() {
        let boundary_invocations = AtomicUsize::new(0);
        let mut pending = pending_after_boundary(&boundary_invocations);
        let mut writer = InjectedWriter::new([WriterEvent::Load(Err(()))]);
        let entered = Arc::new(AtomicBool::new(false));
        let completed = Arc::new(AtomicBool::new(false));
        let mut retry = NeverCompletingRetry {
            entered: Arc::clone(&entered),
            completed: Arc::clone(&completed),
        };
        let mut commit = Box::pin(commit_observation_loop(
            &mut writer,
            &mut pending,
            |material| *material,
            &mut retry,
        ));
        tokio::select! {
            biased;
            _ = &mut commit => panic!("retry unexpectedly completed"),
            () = async {
                while !entered.load(Ordering::SeqCst) {
                    tokio::task::yield_now().await;
                }
            } => {}
        }
        drop(commit);

        assert_eq!(boundary_invocations.load(Ordering::SeqCst), 1);
        assert_eq!(writer.load_count, 1);
        assert!(!completed.load(Ordering::SeqCst));
        writer.assert_exhausted();
    }

    #[tokio::test]
    async fn sealed_fact_scan_pending_material_is_never_rescanned_by_retries() {
        let completed_fact_scan_count = AtomicUsize::new(0);
        let namespace = LegalAdmissionFixture::new(210).expect("fixture namespace");
        let fixture = FactScanConformanceFixture::new(
            namespace.store_identity().clone(),
            namespace.tenant_scope_id().clone(),
            211,
            212,
            213,
        )
        .expect("fact scan fixture");
        let (store, issuer) = open_in_memory(namespace.store_identity().clone());
        fixture
            .producer()
            .provision_in_memory(&store)
            .expect("producer configured value");
        fixture
            .consumer()
            .provision_in_memory(&store)
            .expect("consumer configured value");
        completed_fact_scan_count.fetch_add(1, Ordering::SeqCst);
        let completed = fixture
            .completed_scan_on(store, &issuer)
            .await
            .expect("genuine completed fact scan");
        let authorization_ref = completed.authorization_ref().clone();
        let mut pending = PendingObservation::<FactSelection, TestLogicalIdentity, u64>::new(
            FactSelectionObservation {
                authorization_ref,
                outcome: FactSelectionObservationOutcome::Returned(Box::new(completed.seal())),
            },
        );
        let mut writer = InjectedWriter::with_material_assertion(
            [
                WriterEvent::Load(Err(())),
                WriterEvent::Load(Err(())),
                WriterEvent::Load(Ok(1)),
                WriterEvent::Prepare {
                    expected_physical_id: None,
                    expected_material: 71,
                    outcome: prepared(9, 90),
                },
                WriterEvent::Load(Ok(1)),
                WriterEvent::ResolveLogical {
                    expected_id: 9,
                    outcome: LogicalOutcome::Absent,
                },
                WriterEvent::ResolvePhysical {
                    expected_id: 90,
                    outcome: PhysicalOutcome::AbsentAtCurrentHead,
                },
                WriterEvent::Prepare {
                    expected_physical_id: Some(90),
                    expected_material: 71,
                    outcome: prepared(9, 90),
                },
                WriterEvent::Append {
                    expected_physical_id: 90,
                    outcome: AppendOutcome::Retry,
                },
                WriterEvent::Load(Ok(2)),
                WriterEvent::ResolveLogical {
                    expected_id: 9,
                    outcome: LogicalOutcome::Identical {
                        observation_ref: 99,
                        journal_head: 2,
                    },
                },
            ],
            |material: &ObservationMaterial, expected| {
                assert_eq!(expected, 71);
                assert!(matches!(
                    material,
                    ObservationMaterial::FactSelection { .. }
                ));
            },
        );
        let mut retry = RecordingRetry::new();
        commit_observation_loop(
            &mut writer,
            &mut pending,
            FactSelectionObservation::material,
            &mut retry,
        )
        .await
        .expect("resolve sealed fact-selection observation");

        assert_eq!(
            completed_fact_scan_count.load(Ordering::SeqCst),
            1,
            "CompletedFactScan is consumed into pending material once, before this loop"
        );
        writer.assert_exhausted();
    }
}
