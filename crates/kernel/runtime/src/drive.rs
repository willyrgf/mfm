//! Stateless one-action runtime driver.

use std::collections::BTreeMap;
use std::sync::Arc;

use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_facts::FactSelectionRequest;
use mfm_ids::{ContentRef, EffectKey, NodeId, RequestDigest, StableId};
use mfm_journal::v1::{
    AuthorizationRef, AuthorizationScopeFields, CapabilityBindingRef, ClosureRef,
    FactSelectionScanContract, FrozenReadIntent, InputManifestRef, NodePhase,
    ReadCapabilityBinding, RunPhase, TransitionBodyFields, TransitionRef, ValueRef,
};
use mfm_program::{
    CandidateCertificationError, CandidateCertificationErrorKind, ProposedValueMaterial,
    QualifiedAdmittedProgram, QualifiedAuthoredRequest, QualifiedCandidateStateCallbacks,
    QualifiedEffectEntry, QualifiedEvidenceVerdict, QualifiedProgramRegistry, QualifiedReadEntry,
    QualifiedSettlement, VerifiedReadOutcome, VerifiedStateFrameMaterial,
    VerifiedTerminalEffectView, VerifiedValueMaterial,
};
use mfm_spec::{CertifiedNodeContract, CertifiedStateExecution, RetainedValueContract};
use mfm_store::{
    AppendOutcome, AppendRejection, AuthorizationMaterial, Drive, ExistingRunAppendMaterial,
    FactScanBackend, FactSelectionAuthorizationOutcome, NewlyAppended, NodeTerminalOutcome,
    ObjectGraphProposal, ObservationMaterial, PreparedFrame, PreparedJournalAppend,
    ProducedObjectRoot, ReadObservationMaterial, RunAccessAuthority, RunHistoryWriter,
    TransitionMaterial, VerifiedNodeAccessHistory, VerifiedRunView, FACT_SELECTION_OPERATION_ID,
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
use crate::runtime_error::map_store_error;
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
        observation_ref: mfm_journal::v1::ObservationRef,
        settlement: QualifiedSettlement,
        settlement_contract: mfm_spec::CertifiedSettlementContract,
    },
    Effect {
        node_id: NodeId,
        request_transition_ref: TransitionRef,
        observation_ref: mfm_journal::v1::ObservationRef,
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

enum AccessActionMaterial {
    Read(Box<ReadAccessAction>),
    FactSelection(Box<FactSelectionAccessAction>),
    Ensure(Box<EnsureAccessAction>),
}

struct ReadAccessAction {
    node_id: NodeId,
    state_contract_ref: ContentRef,
    frame: PreparedFrame,
    request_contract: RetainedValueContract,
    returned_contract: RetainedValueContract,
    safe_failure_contract: RetainedValueContract,
    binding_ref: ContentRef,
    operation_id: StableId,
    request: RoutedReadRequest,
}

struct FactSelectionAccessAction {
    node_id: NodeId,
    frame: PreparedFrame,
    request_contract: RetainedValueContract,
    routing_generation_ref: ContentRef,
    request: FactSelectionRequest,
    proposed: ProposedValueMaterial,
}

struct EnsureAccessAction {
    node_id: NodeId,
    intent: EffectIntent,
    request: QualifiedAuthoredRequest,
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

struct EncodedReadObservation {
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
                    self.perform_access(&authority, &view, &admitted, candidate)
                        .await?
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
        DecisionInput<
            SettlementAction,
            LocalActionMaterial,
            AccessActionMaterial,
            IntegrityFinding,
        >,
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
                                    AccessActionMaterial::FactSelection(Box::new(
                                        FactSelectionAccessAction {
                                            node_id: node.node_id().clone(),
                                            frame: *frame,
                                            request_contract: request_contract.clone(),
                                            routing_generation_ref,
                                            request,
                                            proposed,
                                        },
                                    )),
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
                        let invoker = RuntimeReadInvoker::from_entry(entry)?;
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
                                    AccessActionMaterial::Read(Box::new(ReadAccessAction {
                                        node_id: node.node_id().clone(),
                                        state_contract_ref: node.state_contract_ref().clone(),
                                        frame: *frame,
                                        request_contract: request_contract.clone(),
                                        returned_contract: returned_contract.clone(),
                                        safe_failure_contract: safe_failure_contract.clone(),
                                        binding_ref: capability_binding_ref.clone(),
                                        operation_id: capability_operation_id.clone(),
                                        request,
                                    })),
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
                    occurrences[index].access = Some((
                        authorization_count,
                        AccessAction::Ensure,
                        AccessActionMaterial::Ensure(Box::new(EnsureAccessAction {
                            node_id: node.node_id().clone(),
                            intent: *intent,
                            request,
                        })),
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
        admitted: &QualifiedAdmittedProgram,
        action: AccessActionMaterial,
    ) -> Result<ActionResult> {
        match action {
            AccessActionMaterial::Read(action) => {
                self.perform_read(authority, view, admitted, *action).await
            }
            AccessActionMaterial::FactSelection(action) => {
                self.perform_fact_selection(authority, view, *action).await
            }
            AccessActionMaterial::Ensure(action) => {
                self.perform_ensure(authority, view, *action).await
            }
        }
    }

    async fn perform_read(
        &self,
        authority: &RunAccessAuthority<Drive>,
        view: &VerifiedRunView,
        admitted: &QualifiedAdmittedProgram,
        action: ReadAccessAction,
    ) -> Result<ActionResult> {
        let input_manifest_ref = action.frame.input_manifest_ref().clone();
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
        let (expected_request_ref, frozen_read_intent_ref) =
            prepared_read_authorization_refs(&append)?;
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
        let authorized_head = witness.committed().journal_head().clone();
        let Some(entry) = exact_read_entry(
            &self.program_registry,
            &action.binding_ref,
            &action.operation_id,
            &action.request_contract,
            &action.returned_contract,
            &action.safe_failure_contract,
        ) else {
            return Err(RuntimeError::CatalogSelection);
        };
        let safe_failure_contract_ref = entry.binding().fields()?.safe_failure_contract_ref;
        let invoker = RuntimeReadInvoker::from_entry(entry)?;
        let observation = invoker
            .call(
                *witness,
                expected_request_ref,
                ReadInvocationContext {
                    binding_ref: action.binding_ref,
                    operation_id: action.operation_id,
                    node_id: action.node_id.clone(),
                    input_manifest_ref,
                    frozen_read_intent_ref,
                    routing_generation_ref,
                    safe_failure_contract_ref,
                },
                action.request,
            )
            .await?;
        let material = self.encode_read_observation(
            admitted,
            &action.state_contract_ref,
            &action.returned_contract,
            &action.safe_failure_contract,
            observation,
        );
        let material = match material {
            Ok(material) => material,
            Err(RuntimeError::CandidateCertification(error)) => {
                return candidate_failure_at_head(authorized_head, error)
                    .map(ActionResult::Outcome);
            }
            Err(error) => return Err(error),
        };
        self.append_read_observation(authority, &action.node_id, material)
            .await
    }

    fn encode_read_observation(
        &self,
        admitted: &QualifiedAdmittedProgram,
        state_contract_ref: &ContentRef,
        returned_contract: &RetainedValueContract,
        safe_failure_contract: &RetainedValueContract,
        observation: ErasedReadObservation,
    ) -> Result<EncodedReadObservation> {
        let candidate_callbacks = self.program_registry.candidate_callbacks(
            admitted.candidate_identity(),
            admitted.recorded_operation_id(),
        )?;
        if candidate_callbacks.state(state_contract_ref).is_none() {
            return Err(RuntimeError::CatalogSelection);
        }
        let callbacks = self
            .program_registry
            .state(state_contract_ref)
            .ok_or(RuntimeError::CatalogSelection)?
            .callbacks();
        let outcome = match observation.outcome {
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
        };
        Ok(EncodedReadObservation {
            authorization_ref: observation.authorization_ref,
            outcome,
        })
    }

    async fn append_read_observation(
        &self,
        authority: &RunAccessAuthority<Drive>,
        node_id: &NodeId,
        observation: EncodedReadObservation,
    ) -> Result<ActionResult> {
        loop {
            let journal = self
                .writer
                .load_for_drive(authority)
                .await
                .map_err(|error| map_store_error(&error))?;
            let view = journal.verify_recorded_history()?;
            let append_id =
                append_request_id("read_observation", view.journal_head(), Some(node_id))?;
            let append = self.writer.prepare_append(
                authority,
                &view,
                append_id,
                ExistingRunAppendMaterial::Observation(Box::new(ObservationMaterial::Read {
                    authorization_ref: observation.authorization_ref.clone(),
                    outcome: Box::new(observation.material()),
                })),
            )?;
            let result = self
                .writer
                .append(authority, append)
                .await
                .map_err(|error| map_store_error(&error))?;
            match observation_append_outcome(result)? {
                ActionResult::Retry => {}
                outcome => return Ok(outcome),
            }
        }
    }

    async fn perform_ensure(
        &self,
        authority: &RunAccessAuthority<Drive>,
        view: &VerifiedRunView,
        action: EnsureAccessAction,
    ) -> Result<ActionResult> {
        let node = view
            .certified_spec()
            .nodes()
            .iter()
            .find(|node| node.node_id() == &action.node_id)
            .ok_or(RuntimeError::CatalogSelection)?;
        let CertifiedStateExecution::Effect {
            executor_operation_id,
            executor_binding_ref,
            request_contract,
            ensure_result_contract,
            terminal_evidence_contract,
            ..
        } = node.execution()
        else {
            return Err(RuntimeError::CatalogSelection);
        };
        let Some(entry) = exact_effect_entry(
            &self.program_registry,
            executor_binding_ref,
            executor_operation_id,
            request_contract,
            ensure_result_contract,
            terminal_evidence_contract,
        ) else {
            return Err(RuntimeError::CatalogSelection);
        };
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
        let invoker = RuntimeEffectInvoker::from_entry(entry)?;
        let observation = invoker
            .ensure(
                *witness,
                expected_request_ref,
                EffectInvocationContext {
                    binding: entry.binding().clone(),
                    operation_id: executor_operation_id.clone(),
                    request_type: entry.request_type(),
                    response_type: entry.response_type(),
                    failure_type: entry.failure_type(),
                    tenant_scope_id: view.tenant_scope_id().clone(),
                    store_scope_id: view.store_identity().store_scope_id().clone(),
                    run_id: view.run_id().clone(),
                    node_id: action.node_id.clone(),
                    request_transition_ref: action.intent.request_transition_ref,
                    effect_key: action.intent.effect_key,
                    request_digest: action.intent.request_digest,
                },
                action.request,
            )
            .await?;
        self.append_effect_observation(authority, &action.node_id, observation)
            .await
    }

    async fn append_effect_observation(
        &self,
        authority: &RunAccessAuthority<Drive>,
        node_id: &NodeId,
        observation: crate::capability_registry::ErasedEnsureObservation,
    ) -> Result<ActionResult> {
        loop {
            let journal = self
                .writer
                .load_for_drive(authority)
                .await
                .map_err(|error| map_store_error(&error))?;
            let view = journal.verify_recorded_history()?;
            let append_id =
                append_request_id("effect_observation", view.journal_head(), Some(node_id))?;
            let append = self.writer.prepare_append(
                authority,
                &view,
                append_id,
                ExistingRunAppendMaterial::Observation(Box::new(
                    ObservationMaterial::EnsureEffect {
                        authorization_ref: observation.authorization_ref.clone(),
                        outcome: Box::new(observation.outcome.clone()),
                    },
                )),
            )?;
            let result = self
                .writer
                .append(authority, append)
                .await
                .map_err(|error| map_store_error(&error))?;
            match observation_append_outcome(result)? {
                ActionResult::Retry => {}
                outcome => return Ok(outcome),
            }
        }
    }

    async fn perform_fact_selection(
        &self,
        authority: &RunAccessAuthority<Drive>,
        view: &VerifiedRunView,
        action: FactSelectionAccessAction,
    ) -> Result<ActionResult> {
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
        let completed = self
            .writer
            .scan_fact_selection(permit)
            .await
            .map_err(|error| map_store_error(&error))?;
        let journal = self
            .writer
            .load_for_drive(authority)
            .await
            .map_err(|error| map_store_error(&error))?;
        let view = journal.verify_recorded_history()?;
        let append_id = append_request_id(
            "fact_selection_observation",
            view.journal_head(),
            Some(&action.node_id),
        )?;
        let append = self.writer.prepare_append(
            authority,
            &view,
            append_id,
            completed.into_observation_material(),
        )?;
        let result = self
            .writer
            .append(authority, append)
            .await
            .map_err(|error| map_store_error(&error))?;
        observation_append_outcome(result)
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
) -> Result<Vec<mfm_journal::v1::ObservationRef>> {
    history
        .observed_suffix()
        .map(|observed| {
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
            Ok(observation_ref)
        })
        .collect()
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
        mfm_journal::v1::ObservationRef,
        QualifiedSettlement,
    )>,
> {
    let verdicts = history
        .observed_suffix()
        .map(|observed| {
            let audit = observed.audit();
            let request_ref = audit.request_ref().clone();
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
) -> Result<BTreeMap<NodeId, mfm_journal::v1::ObservationRef>> {
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
) -> Result<EvidenceScan<(mfm_journal::v1::ObservationRef, QualifiedSettlement)>> {
    let verdicts = history
        .observed_suffix()
        .map(|observed| {
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

fn prepared_read_authorization_refs(
    append: &PreparedJournalAppend,
) -> Result<(ValueRef, ValueRef)> {
    let PreparedJournalAppend::AuthorizeExternalAccess(append) = append else {
        return Err(RuntimeError::InvalidCallbackResult);
    };
    let fields = append.authorization().fields()?;
    let frozen = fields
        .frozen_read_intent_ref
        .ok_or(RuntimeError::InvalidCallbackResult)?;
    Ok((fields.request_ref, frozen))
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

fn observation_append_outcome(outcome: AppendOutcome) -> Result<ActionResult> {
    match outcome {
        AppendOutcome::NewlyAppended(NewlyAppended::Observation(committed))
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

fn advanced(journal_head: mfm_journal::v1::JournalHead) -> DriveOutcome {
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
    journal_head: mfm_journal::v1::JournalHead,
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
    journal_head: mfm_journal::v1::JournalHead,
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
