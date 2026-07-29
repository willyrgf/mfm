use std::collections::BTreeSet;
use std::future::Future;
use std::pin::Pin;

use mfm_canonical::{CanonicalValue, RecoverabilityContractV1, ValidatedCanonicalValueV1};
use mfm_ids::{ContentRef, RunId, SchemaId};
use mfm_journal::v1::{
    FactSelectionCompleteness, JournalHead, PersistedJournalValue, TransitionRef,
};
use mfm_program::{
    CandidateCertificationError, CandidateCertificationErrorKind, CandidatePlanCompatibility,
    ProgramError, ProposedFactValue, ProposedValueMaterial, QualifiedCandidateCallbacks,
    QualifiedCandidateIdentity, QualifiedCandidateStateCallbacks, QualifiedEvidenceVerdict,
    QualifiedProgramRegistry, QualifiedSettlement, StateExecutionKind, TerminalEffectResolver,
    VerifiedReadOutcome, VerifiedStateFrameMaterial, VerifiedTerminalEffectView,
    VerifiedValueMaterial,
};
use mfm_spec::{
    CanonicalAuthoredProgram, CertifiedAdmissionArtifacts, CertifiedNodeContract,
    CertifiedStateExecution, EntryPointContract, ExpandedCertifiedSpec,
};
use mfm_store::v1::{
    ComparisonEvidenceKind, ComparisonSettlementKind, ComparisonTransitionKind,
    RecordedEvidenceVerdict, VerifiedComparisonEvidence, VerifiedComparisonFact,
    VerifiedComparisonFrame, VerifiedComparisonReadOutcome, VerifiedComparisonSettlement,
    VerifiedComparisonStateFrame, VerifiedComparisonTerminalEffect, VerifiedComparisonValue,
    VerifiedRunView,
};

use crate::trace_export::VerifiedPortableExport;

use super::{ReplayError, Result};

const EXACT_REPRODUCTION_PLAN_CONTRACT: &str = "mfm.exact-reproduction-plan.v1";
const CANDIDATE_COMPARISON_PLAN_CONTRACT: &str = "mfm.candidate-comparison-plan.v1";
const REPLAY_RESULT_CONTRACT: &str = "mfm.replay-result.v1";

/// Boxed asynchronous result returned by an injected reproduction resolver.
pub type ReproductionFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Resolver for one explicitly isolated exact historical executable.
///
/// The resolver receives canonical plan bytes only. It is never given a store,
/// verified view, append authority, live capability, executor, signer, or
/// domain-filesystem handle. Boundary setup failure must return
/// [`ExactReproduction::Unavailable`]; it must never launch the executable
/// without the configured isolation boundary.
pub trait ReproductionResolver: Send + Sync {
    /// Resolves and runs the exact admitted executable against one canonical plan.
    fn reproduce_exact<'a>(
        &'a self,
        canonical_plan: &'a [u8],
    ) -> ReproductionFuture<'a, ExactReproduction>;
}

/// Canonical callback input for one exact historical reproduction.
#[derive(Clone, PartialEq, Eq)]
pub struct ExactReproductionPlan {
    validated: ValidatedCanonicalValueV1,
}

impl ExactReproductionPlan {
    pub(crate) fn from_verified_export(portable_export: &VerifiedPortableExport) -> Result<Self> {
        let view = portable_export.verified_view();
        let admission = view.admission().fields()?;
        let semantic_head = view.semantic_head().clone();
        let semantic_sequence = semantic_head.fields()?.run_sequence;
        let mut ordered_transition_refs = Vec::new();
        let comparison_frames = view
            .comparison_frames()
            .map_err(|error| super::store_error(&error))?;
        for frame in comparison_frames.frames() {
            if frame.containing_journal_head().fields()?.run_sequence > semantic_sequence {
                break;
            }
            ordered_transition_refs.push(frame.transition_ref().canonical_value()?);
        }
        let value = object([
            ("version", string("mfm.exact-reproduction-plan.v1")),
            ("run_id", string(view.run_id().as_str())),
            ("semantic_head", semantic_head.canonical_value()?),
            (
                "portable_export_ref",
                content_ref(portable_export.content_ref())?,
            ),
            (
                "admitted_executable_identity_ref",
                content_ref(&admission.executable_identity_ref)?,
            ),
            (
                "state_implementation_manifest_ref",
                content_ref(&admission.state_implementation_manifest_ref)?,
            ),
            (
                "ordered_transition_refs",
                CanonicalValue::Array(ordered_transition_refs),
            ),
        ])?;
        let validated = RecoverabilityContractV1::embedded()?
            .encode(EXACT_REPRODUCTION_PLAN_CONTRACT, &value)?;
        Ok(Self { validated })
    }

    /// Strictly decodes exact canonical sandbox-plan bytes.
    pub fn strict_decode(bytes: &[u8]) -> Result<Self> {
        let validated = RecoverabilityContractV1::embedded()?
            .strict_decode(EXACT_REPRODUCTION_PLAN_CONTRACT, bytes)?;
        Ok(Self { validated })
    }

    /// Returns the exact canonical bytes passed to the isolated resolver.
    pub fn as_bytes(&self) -> &[u8] {
        self.validated.as_bytes()
    }

    /// Returns the annex-derived plan schema identity.
    pub const fn schema_id(&self) -> &SchemaId {
        self.validated.schema_id()
    }

    /// Reconstructs the canonical plan tree.
    pub fn canonical_value(&self) -> Result<CanonicalValue> {
        self.validated.canonical_value().map_err(Into::into)
    }
}

impl std::fmt::Debug for ExactReproductionPlan {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ExactReproductionPlan")
            .field("schema_id", self.schema_id())
            .finish_non_exhaustive()
    }
}

#[derive(Clone, PartialEq, Eq)]
struct CandidateComparisonPlan {
    validated: ValidatedCanonicalValueV1,
}

impl CandidateComparisonPlan {
    fn new(
        recorded: &ExactReproductionPlan,
        candidate: &QualifiedCandidateIdentity,
    ) -> Result<Self> {
        let value = object([
            ("version", string("mfm.candidate-comparison-plan.v1")),
            ("recorded", recorded.canonical_value()?),
            (
                "candidate_executable_identity_ref",
                content_ref(candidate.candidate_executable_identity_ref())?,
            ),
            (
                "candidate_planning_profile_ref",
                content_ref(candidate.planning_profile_ref())?,
            ),
            (
                "candidate_planner_contract_ref",
                content_ref(candidate.planner_contract_ref())?,
            ),
            (
                "candidate_planner_implementation_ref",
                content_ref(candidate.planner_implementation_ref())?,
            ),
            (
                "candidate_state_implementation_manifest_ref",
                content_ref(candidate.state_implementation_manifest_ref())?,
            ),
            (
                "candidate_capability_binding_manifest_ref",
                content_ref(candidate.capability_binding_manifest_ref())?,
            ),
        ])?;
        let validated = RecoverabilityContractV1::embedded()?
            .encode(CANDIDATE_COMPARISON_PLAN_CONTRACT, &value)?;
        Ok(Self { validated })
    }

    fn as_bytes(&self) -> &[u8] {
        self.validated.as_bytes()
    }
}

fn validate_candidate_plan<'history>(
    plan: &CandidateComparisonPlan,
    historical: &'history VerifiedPortableExport,
    candidate: &QualifiedCandidateIdentity,
) -> Result<&'history VerifiedRunView> {
    let view = historical.verified_view();
    let recorded = ExactReproductionPlan::from_verified_export(historical)
        .map_err(|_| ReplayError::InvalidRecordedHistory)?;
    let expected = CandidateComparisonPlan::new(&recorded, candidate)
        .map_err(|_| ReplayError::InvalidRecordedHistory)?;
    if plan.as_bytes() != expected.as_bytes() {
        return Err(ReplayError::ComparisonIntegrityFailed);
    }
    Ok(view)
}

fn recorded_candidate_contracts(
    historical: &VerifiedPortableExport,
    view: &VerifiedRunView,
) -> Result<(EntryPointContract, CanonicalAuthoredProgram)> {
    let admission = view
        .admission()
        .fields()
        .map_err(|_| ReplayError::InvalidRecordedHistory)?;
    let entry_point = historical
        .retained_content(&admission.operation_contract_ref)
        .map_err(|_| ReplayError::InvalidRecordedHistory)
        .and_then(|bytes| {
            EntryPointContract::from_canonical_json(bytes)
                .map_err(|_| ReplayError::InvalidRecordedHistory)
        })?;
    let authored_program = historical
        .retained_content(view.certified_spec().authored_program_ref())
        .map_err(|_| ReplayError::InvalidRecordedHistory)
        .and_then(|bytes| {
            CanonicalAuthoredProgram::from_canonical_json(bytes)
                .map_err(|_| ReplayError::InvalidRecordedHistory)
        })?;
    Ok((entry_point, authored_program))
}

fn candidate_semantic_graph_matches(
    recorded_authored_program: &CanonicalAuthoredProgram,
    recorded_spec: &ExpandedCertifiedSpec,
    candidate: &CertifiedAdmissionArtifacts,
) -> bool {
    exact_authored_program_matches(recorded_authored_program, candidate.authored_program())
        && expanded_spec_semantics_match(recorded_spec, candidate.expanded_spec())
}

fn exact_authored_program_matches(
    left: &CanonicalAuthoredProgram,
    right: &CanonicalAuthoredProgram,
) -> bool {
    left.canonical_json()
        .ok()
        .zip(right.canonical_json().ok())
        .is_some_and(|(left, right)| left.as_bytes() == right.as_bytes())
}

fn expanded_spec_semantics_match(
    left: &ExpandedCertifiedSpec,
    right: &ExpandedCertifiedSpec,
) -> bool {
    left.authored_program_ref() == right.authored_program_ref()
        && left.nodes().len() == right.nodes().len()
        && left
            .nodes()
            .iter()
            .zip(right.nodes())
            .all(|(left, right)| certified_node_semantics_match(left, right))
        && left.public_output_contract() == right.public_output_contract()
        && left.run_terminal_contract() == right.run_terminal_contract()
        && left.journal_protocol_contracts() == right.journal_protocol_contracts()
}

fn certified_node_semantics_match(
    left: &CertifiedNodeContract,
    right: &CertifiedNodeContract,
) -> bool {
    left.node_id() == right.node_id()
        && left.canonical_expansion_path() == right.canonical_expansion_path()
        && left.state_contract_ref() == right.state_contract_ref()
        && left.config_binding() == right.config_binding()
        && left.context_binding() == right.context_binding()
        && left.input_contract() == right.input_contract()
        && left.input_bindings() == right.input_bindings()
        && certified_execution_semantics_match(left.execution(), right.execution())
        && left.settlement_contract() == right.settlement_contract()
}

fn certified_execution_semantics_match(
    left: &CertifiedStateExecution,
    right: &CertifiedStateExecution,
) -> bool {
    match (left, right) {
        (CertifiedStateExecution::Pure, CertifiedStateExecution::Pure) => true,
        (
            CertifiedStateExecution::Read {
                capability_operation_id: left_operation,
                request_contract: left_request,
                returned_contract: left_returned,
                safe_failure_contract: left_failure,
                ..
            },
            CertifiedStateExecution::Read {
                capability_operation_id: right_operation,
                request_contract: right_request,
                returned_contract: right_returned,
                safe_failure_contract: right_failure,
                ..
            },
        ) => {
            left_operation == right_operation
                && left_request == right_request
                && left_returned == right_returned
                && left_failure == right_failure
        }
        (
            CertifiedStateExecution::Effect {
                executor_operation_id: left_operation,
                request_contract: left_request,
                ensure_result_contract: left_ensure,
                terminal_evidence_contract: left_terminal,
                domain_result_contract: left_domain,
                ..
            },
            CertifiedStateExecution::Effect {
                executor_operation_id: right_operation,
                request_contract: right_request,
                ensure_result_contract: right_ensure,
                terminal_evidence_contract: right_terminal,
                domain_result_contract: right_domain,
                ..
            },
        ) => {
            left_operation == right_operation
                && left_request == right_request
                && left_ensure == right_ensure
                && left_terminal == right_terminal
                && left_domain == right_domain
        }
        _ => false,
    }
}

/// Compares verified historical behavior with the unique sealed current candidate.
///
/// The candidate is selected solely from the stable operation recorded by admission. The
/// comparison invokes no live capability, executor, provider, filesystem, signer, or append
/// surface, and candidate outputs never become inputs to later transition comparisons.
pub fn compare_current(
    historical: &VerifiedPortableExport,
    registry: &QualifiedProgramRegistry,
) -> Result<CanonicalReplayResult> {
    let view = historical.verified_view();
    let admission = view
        .admission()
        .fields()
        .map_err(|_| ReplayError::InvalidRecordedHistory)?;
    let candidate = registry
        .select_current_candidate(&admission.entry_point_operation_id)
        .map_err(candidate_error)?;
    let callbacks = registry
        .candidate_callbacks(&candidate, &admission.entry_point_operation_id)
        .map_err(candidate_error)?;
    let exact_plan = ExactReproductionPlan::from_verified_export(historical)?;
    let plan = CandidateComparisonPlan::new(&exact_plan, &candidate)
        .map_err(|_| ReplayError::ComparisonIntegrityFailed)?;
    let view = validate_candidate_plan(&plan, historical, &candidate)?;
    let (recorded_entry, recorded_authored_program) =
        recorded_candidate_contracts(historical, view)?;
    let compatibility = callbacks.classify_plan_compatibility(
        &recorded_entry,
        view.recorded_configured_value().value_contract(),
    );
    let comparison_frames = view
        .comparison_frames()
        .map_err(|error| super::store_error(&error))?;

    let (plan_verdict, transitions) = match compatibility {
        CandidatePlanCompatibility::NotComparable => (
            ComparisonVerdict::NotComparable,
            comparison_frames
                .frames()
                .map(|frame| {
                    CandidateTransitionComparison::new(
                        frame.transition_ref().clone(),
                        ComparisonVerdict::NotComparable,
                    )
                })
                .collect(),
        ),
        CandidatePlanCompatibility::Comparable => {
            let candidate_authored_program = callbacks
                .author_recorded_configured(view.recorded_configured_value())
                .map_err(candidate_error)?;
            let candidate_artifacts = callbacks
                .certify(&candidate_authored_program)
                .map_err(candidate_error)?;
            let plan_verdict = if candidate_semantic_graph_matches(
                &recorded_authored_program,
                view.certified_spec(),
                &candidate_artifacts,
            ) {
                ComparisonVerdict::Agrees
            } else {
                ComparisonVerdict::Differs
            };
            let transitions = comparison_frames
                .frames()
                .map(|frame| {
                    compare_candidate_frame(frame, &candidate_artifacts, &callbacks).map(|result| {
                        CandidateTransitionComparison::new(frame.transition_ref().clone(), result)
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            (plan_verdict, transitions)
        }
    };

    CandidateComparison::new(
        view.run_id().clone(),
        CandidateComparisonReport {
            admitted_executable_identity_ref: admission.executable_identity_ref,
            candidate_executable_identity_ref: candidate
                .candidate_executable_identity_ref()
                .clone(),
            candidate_planning_profile_ref: candidate.planning_profile_ref().clone(),
            candidate_planner_contract_ref: candidate.planner_contract_ref().clone(),
            candidate_planner_implementation_ref: candidate.planner_implementation_ref().clone(),
            candidate_state_implementation_manifest_ref: candidate
                .state_implementation_manifest_ref()
                .clone(),
            candidate_capability_binding_manifest_ref: candidate
                .capability_binding_manifest_ref()
                .clone(),
            plan: plan_verdict,
            transitions,
        },
    )?
    .canonical_result()
}

fn compare_candidate_frame(
    recorded: &VerifiedComparisonFrame<'_>,
    candidate: &CertifiedAdmissionArtifacts,
    callbacks: &QualifiedCandidateCallbacks<'_>,
) -> Result<ComparisonVerdict> {
    let Some(candidate_node) = candidate
        .expanded_spec()
        .nodes()
        .iter()
        .find(|node| node.node_id() == recorded.certified_node().node_id())
    else {
        return Ok(ComparisonVerdict::NotComparable);
    };
    if !certified_node_semantics_match(recorded.certified_node(), candidate_node) {
        return Ok(ComparisonVerdict::NotComparable);
    }
    if recorded.kind() == ComparisonTransitionKind::DependencySkipped {
        return Ok(ComparisonVerdict::Agrees);
    }

    let state = callbacks
        .state(candidate_node.state_contract_ref())
        .ok_or(ReplayError::ComparisonIntegrityFailed)?;
    if !state_kind_matches_frame(state.kind(), recorded.kind()) {
        return Err(ReplayError::ComparisonIntegrityFailed);
    }
    let historical_frame = recorded
        .frame()
        .ok_or(ReplayError::InvalidRecordedHistory)?;
    let frame = callback_frame(historical_frame);

    match recorded.kind() {
        ComparisonTransitionKind::PureSettled => {
            let candidate_settlement = state.settle_pure(&frame).map_err(candidate_error)?;
            let recorded_settlement = recorded
                .recorded_settlement()
                .ok_or(ReplayError::InvalidRecordedHistory)?;
            settlement_agrees(&candidate_settlement, recorded_settlement)
                .map(bool_comparison_verdict)
        }
        ComparisonTransitionKind::ReadSettled => compare_candidate_read(recorded, &state, &frame),
        ComparisonTransitionKind::EffectRequested => {
            compare_candidate_request(recorded, &state, &frame).map(bool_comparison_verdict)
        }
        ComparisonTransitionKind::EffectSettled => {
            compare_candidate_effect(recorded, &state, &frame)
        }
        ComparisonTransitionKind::DependencySkipped => Err(ReplayError::ComparisonIntegrityFailed),
    }
}

const fn state_kind_matches_frame(
    state_kind: StateExecutionKind,
    frame_kind: ComparisonTransitionKind,
) -> bool {
    matches!(
        (state_kind, frame_kind),
        (
            StateExecutionKind::Pure,
            ComparisonTransitionKind::PureSettled
        ) | (
            StateExecutionKind::Read,
            ComparisonTransitionKind::ReadSettled
        ) | (
            StateExecutionKind::Effect,
            ComparisonTransitionKind::EffectRequested | ComparisonTransitionKind::EffectSettled
        )
    )
}

fn callback_frame(recorded: &VerifiedComparisonStateFrame) -> VerifiedStateFrameMaterial {
    VerifiedStateFrameMaterial::new(
        callback_value(recorded.config()),
        recorded.context().map(callback_value),
        callback_value(recorded.input()),
    )
}

fn callback_value(recorded: &VerifiedComparisonValue) -> VerifiedValueMaterial {
    VerifiedValueMaterial::new(recorded.canonical().clone(), recorded.value_ref().clone())
}

fn compare_candidate_request(
    recorded: &VerifiedComparisonFrame<'_>,
    state: &QualifiedCandidateStateCallbacks<'_>,
    frame: &VerifiedStateFrameMaterial,
) -> Result<bool> {
    let recorded_request = recorded
        .request()
        .ok_or(ReplayError::InvalidRecordedHistory)?;
    let authored = state.author_request(frame).map_err(candidate_error)?;
    if !proposed_value_agrees(authored.proposed(), recorded_request)? {
        return Ok(false);
    }
    let decoded = state
        .decode_request(&callback_value(recorded_request))
        .map_err(candidate_error)?;
    if authored.request_type() != decoded.request_type()
        || !proposed_value_agrees(decoded.proposed(), recorded_request)?
    {
        return Err(ReplayError::ComparisonIntegrityFailed);
    }
    Ok(true)
}

fn compare_candidate_read(
    recorded: &VerifiedComparisonFrame<'_>,
    state: &QualifiedCandidateStateCallbacks<'_>,
    frame: &VerifiedStateFrameMaterial,
) -> Result<ComparisonVerdict> {
    if !compare_candidate_request(recorded, state, frame)? {
        return Ok(ComparisonVerdict::Differs);
    }
    let recorded_settlement = recorded
        .recorded_settlement()
        .ok_or(ReplayError::InvalidRecordedHistory)?;
    for evidence in recorded.evidence() {
        if evidence.kind() != ComparisonEvidenceKind::Read {
            return Err(ReplayError::InvalidRecordedHistory);
        }
        let observation = callback_read_outcome(evidence)?;
        let candidate_verdict = state
            .settle_read(frame, &observation)
            .map_err(candidate_error)?;
        match compare_evidence_verdict(
            evidence.recorded_verdict(),
            &candidate_verdict,
            recorded_settlement,
        )? {
            EvidenceComparison::Continue => {}
            EvidenceComparison::Settled(result) => return Ok(bool_comparison_verdict(result)),
            EvidenceComparison::Differs => return Ok(ComparisonVerdict::Differs),
        }
    }
    Err(ReplayError::InvalidRecordedHistory)
}

fn callback_read_outcome(evidence: &VerifiedComparisonEvidence) -> Result<VerifiedReadOutcome> {
    match evidence
        .read_outcome()
        .ok_or(ReplayError::InvalidRecordedHistory)?
    {
        VerifiedComparisonReadOutcome::Returned(value) => {
            Ok(VerifiedReadOutcome::Returned(callback_value(value)))
        }
        VerifiedComparisonReadOutcome::DidNotEnter(failure) => {
            Ok(VerifiedReadOutcome::DidNotEnter {
                metadata: failure.metadata().clone(),
                diagnostic: failure.diagnostic().map(callback_value),
            })
        }
        VerifiedComparisonReadOutcome::Indeterminate(failure) => {
            Ok(VerifiedReadOutcome::Indeterminate {
                metadata: failure.metadata().clone(),
                diagnostic: failure.diagnostic().map(callback_value),
            })
        }
    }
}

fn compare_candidate_effect(
    recorded: &VerifiedComparisonFrame<'_>,
    state: &QualifiedCandidateStateCallbacks<'_>,
    frame: &VerifiedStateFrameMaterial,
) -> Result<ComparisonVerdict> {
    if !compare_candidate_request(recorded, state, frame)? {
        return Ok(ComparisonVerdict::Differs);
    }
    let recorded_settlement = recorded
        .recorded_settlement()
        .ok_or(ReplayError::InvalidRecordedHistory)?;
    for evidence in recorded.evidence() {
        if evidence.kind() != ComparisonEvidenceKind::TerminalEffect {
            return Err(ReplayError::InvalidRecordedHistory);
        }
        let Some(terminal) = evidence.terminal_effect() else {
            if evidence.recorded_verdict() != RecordedEvidenceVerdict::InsufficientEvidence {
                return Err(ReplayError::InvalidRecordedHistory);
            }
            continue;
        };
        let resolver = ComparisonTerminalResolver { terminal };
        let terminal_view = VerifiedTerminalEffectView::new(
            terminal.evidence(),
            terminal.evidence_ref(),
            &resolver,
        );
        let candidate_verdict = state
            .settle_effect(frame, terminal_view)
            .map_err(candidate_error)?;
        match compare_evidence_verdict(
            evidence.recorded_verdict(),
            &candidate_verdict,
            recorded_settlement,
        )? {
            EvidenceComparison::Continue => {}
            EvidenceComparison::Settled(result) => return Ok(bool_comparison_verdict(result)),
            EvidenceComparison::Differs => return Ok(ComparisonVerdict::Differs),
        }
    }
    Err(ReplayError::InvalidRecordedHistory)
}

enum EvidenceComparison {
    Continue,
    Settled(bool),
    Differs,
}

fn compare_evidence_verdict(
    recorded_verdict: RecordedEvidenceVerdict,
    candidate_verdict: &QualifiedEvidenceVerdict,
    recorded_settlement: &VerifiedComparisonSettlement,
) -> Result<EvidenceComparison> {
    match (recorded_verdict, candidate_verdict) {
        (
            RecordedEvidenceVerdict::InsufficientEvidence,
            QualifiedEvidenceVerdict::InsufficientEvidence,
        ) => Ok(EvidenceComparison::Continue),
        (
            RecordedEvidenceVerdict::Settlement,
            QualifiedEvidenceVerdict::Settlement(candidate_settlement),
        ) => settlement_agrees(candidate_settlement, recorded_settlement)
            .map(EvidenceComparison::Settled),
        _ => Ok(EvidenceComparison::Differs),
    }
}

struct ComparisonTerminalResolver<'a> {
    terminal: &'a VerifiedComparisonTerminalEffect,
}

impl TerminalEffectResolver for ComparisonTerminalResolver<'_> {
    fn resolve(
        &self,
        value_ref: &mfm_journal::v1::ValueRef,
    ) -> mfm_program::Result<mfm_canonical::PlainCanonicalJsonBytes> {
        self.terminal
            .resolve(value_ref)
            .map(|value| value.canonical().clone())
            .map_err(|_| {
                ProgramError::Codec(
                    "terminal evidence requested material outside its verified closure".to_owned(),
                )
            })
    }
}

fn settlement_agrees(
    candidate: &QualifiedSettlement,
    recorded: &VerifiedComparisonSettlement,
) -> Result<bool> {
    match (candidate, recorded.kind()) {
        (
            QualifiedSettlement::Succeeded {
                output_slots,
                facts,
            },
            ComparisonSettlementKind::Succeeded,
        ) => {
            if output_slots.len() != recorded.outputs().len()
                || facts.as_slice().len() != recorded.facts().len()
            {
                return Ok(false);
            }
            for (candidate, recorded) in output_slots.iter().zip(recorded.outputs()) {
                if candidate.output_ordinal() != recorded.output_ordinal()
                    || candidate.field_path() != recorded.field_path()
                    || candidate.value_contract() != recorded.retained_contract()
                    || !proposed_value_agrees(candidate.value(), recorded.value())?
                {
                    return Ok(false);
                }
            }
            for (emission_ordinal, (candidate, recorded)) in
                facts.as_slice().iter().zip(recorded.facts()).enumerate()
            {
                let emission_ordinal = u32::try_from(emission_ordinal)
                    .map_err(|_| ReplayError::ComparisonIntegrityFailed)?;
                if !fact_agrees(emission_ordinal, candidate, recorded)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        (QualifiedSettlement::Failed { failure }, ComparisonSettlementKind::Failed) => {
            proposed_value_agrees(
                failure,
                recorded
                    .failure()
                    .ok_or(ReplayError::InvalidRecordedHistory)?,
            )
        }
        _ => Ok(false),
    }
}

fn fact_agrees(
    emission_ordinal: u32,
    candidate: &mfm_program::FactProposal,
    recorded: &VerifiedComparisonFact,
) -> Result<bool> {
    Ok(emission_ordinal == recorded.emission_ordinal()
        && candidate.fact_slot_ordinal() == recorded.fact_slot_ordinal()
        && candidate.descriptor_ref() == recorded.descriptor_ref()
        && proposed_fact_value_agrees(candidate.subject(), recorded.subject())?
        && proposed_fact_value_agrees(candidate.response(), recorded.response())?)
}

fn proposed_fact_value_agrees(
    candidate: &ProposedFactValue,
    recorded: &VerifiedComparisonValue,
) -> Result<bool> {
    let fields = recorded.value_ref().fields()?;
    Ok(candidate.content_ref().schema_id() == &fields.schema_id
        && candidate.content_ref().content_digest() == &fields.content_digest
        && candidate.semantic_type_id() == &fields.semantic_type_id
        && candidate.role() == &fields.role
        && candidate.media_type() == fields.media_type
        && candidate.evidence_contract_ref() == &fields.evidence_contract_ref
        && candidate.canonical() == recorded.canonical())
}

fn proposed_value_agrees(
    candidate: &ProposedValueMaterial,
    recorded: &VerifiedComparisonValue,
) -> Result<bool> {
    let fields = recorded.value_ref().fields()?;
    Ok(candidate.content_ref().schema_id() == &fields.schema_id
        && candidate.content_ref().content_digest() == &fields.content_digest
        && candidate.canonical() == recorded.canonical())
}

const fn bool_comparison_verdict(agrees: bool) -> ComparisonVerdict {
    if agrees {
        ComparisonVerdict::Agrees
    } else {
        ComparisonVerdict::Differs
    }
}

fn candidate_error(error: CandidateCertificationError) -> ReplayError {
    match error.kind() {
        CandidateCertificationErrorKind::Unavailable => ReplayError::CandidateUnavailable,
        CandidateCertificationErrorKind::ExecutionFailed => ReplayError::CandidateExecutionFailed,
        CandidateCertificationErrorKind::IntegrityFailed => ReplayError::ComparisonIntegrityFailed,
    }
}

/// Runs exact historical reproduction only after binding a caller-verified portable export to
/// one affine callback-free verified-history session.
///
/// The resolver receives only the frozen canonical plan bytes. A mismatched portable-export
/// token or resolver result fails before a positive canonical result can be returned.
pub async fn reproduce_exact(
    historical: &VerifiedPortableExport,
    resolver: &dyn ReproductionResolver,
) -> Result<CanonicalReplayResult> {
    let view = historical.verified_view();
    let plan = ExactReproductionPlan::from_verified_export(historical)?;
    let result = resolver.reproduce_exact(plan.as_bytes()).await;
    if let ExactReproduction::Mismatch {
        transition_ref: Some(transition_ref),
    } = &result
    {
        let belongs_to_view = view
            .comparison_frames()
            .map_err(|error| super::store_error(&error))?
            .frames()
            .any(|frame| frame.transition_ref().as_bytes() == transition_ref.as_bytes());
        if !belongs_to_view {
            return Err(ReplayError::InvalidRecordedHistory);
        }
    }
    result.canonical_result(view.run_id())
}

/// Canonical replay response validated by the frozen recoverability annex.
///
/// The value intentionally has no Serde implementation. Its exact bytes come
/// only from the embedded annex codec.
#[derive(Clone, PartialEq, Eq)]
pub struct CanonicalReplayResult {
    validated: ValidatedCanonicalValueV1,
}

impl CanonicalReplayResult {
    fn encode(value: &CanonicalValue) -> Result<Self> {
        let validated =
            RecoverabilityContractV1::embedded()?.encode(REPLAY_RESULT_CONTRACT, value)?;
        Ok(Self { validated })
    }

    /// Strictly decodes exact canonical replay-result bytes.
    pub fn strict_decode(bytes: &[u8]) -> Result<Self> {
        let validated =
            RecoverabilityContractV1::embedded()?.strict_decode(REPLAY_RESULT_CONTRACT, bytes)?;
        Ok(Self { validated })
    }

    /// Returns exact annex-validated canonical bytes.
    pub fn as_bytes(&self) -> &[u8] {
        self.validated.as_bytes()
    }

    /// Returns the annex-derived replay-result schema identity.
    pub const fn schema_id(&self) -> &SchemaId {
        self.validated.schema_id()
    }

    /// Reconstructs the canonical value tree.
    pub fn canonical_value(&self) -> Result<CanonicalValue> {
        self.validated.canonical_value().map_err(Into::into)
    }
}

impl std::fmt::Debug for CanonicalReplayResult {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CanonicalReplayResult")
            .field("schema_id", self.schema_id())
            .finish_non_exhaustive()
    }
}

/// Affine callback-free verification session for one exact recorded run.
///
/// The authoritative store view remains private. Verification-mode callers may render the
/// canonical summary directly; non-verification modes consume this session to bind one explicit
/// caller-held semantic portable export.
pub struct VerifiedHistoryResult {
    view: VerifiedRunView,
    fact_selections: Vec<FactSelectionCompleteness>,
}

impl VerifiedHistoryResult {
    /// Constructs a verified response from one already callback-free verified view.
    pub(crate) fn new(
        view: VerifiedRunView,
        fact_selections: Vec<FactSelectionCompleteness>,
    ) -> Self {
        Self {
            view,
            fact_selections,
        }
    }

    /// Returns the verified run identity.
    pub const fn run_id(&self) -> &RunId {
        self.view.run_id()
    }

    /// Returns the exact verified physical journal head.
    pub const fn journal_head(&self) -> &JournalHead {
        self.view.journal_head()
    }

    /// Returns fact-selection completeness in transition order.
    pub fn fact_selections(&self) -> &[FactSelectionCompleteness] {
        &self.fact_selections
    }

    /// Consumes this affine verified session and binds one caller-held semantic export.
    pub fn verify_portable_export(
        self,
        bytes: &[u8],
        expected_ref: &ContentRef,
    ) -> Result<VerifiedPortableExport> {
        VerifiedPortableExport::verify(bytes, expected_ref, self.view)
    }

    /// Encodes the frozen `verified` replay-result variant.
    pub fn canonical_result(&self) -> Result<CanonicalReplayResult> {
        let fact_selections = self
            .fact_selections
            .iter()
            .map(PersistedJournalValue::canonical_value)
            .collect::<std::result::Result<Vec<_>, _>>()?;
        CanonicalReplayResult::encode(&object([
            ("kind", string("verified")),
            ("run_id", string(self.run_id().as_str())),
            ("journal_head", self.journal_head().canonical_value()?),
            ("fact_selections", CanonicalValue::Array(fact_selections)),
        ])?)
    }
}

impl std::fmt::Debug for VerifiedHistoryResult {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("VerifiedHistoryResult")
            .field("run_id", self.run_id())
            .field("journal_head", self.journal_head())
            .field("fact_selection_count", &self.fact_selections.len())
            .finish_non_exhaustive()
    }
}

/// Exact semantic reproduction outcome.
///
/// `Unavailable` intentionally carries no public reason. Operational resolver
/// and isolation diagnostics belong to the application/runtime boundary and
/// cannot be persisted or serialized through this type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExactReproduction {
    /// Every reproducible transition matched its recorded decision.
    Matched,
    /// A callback or plan diverged from recorded history.
    Mismatch {
        /// First mismatching transition when the mismatch is transition-local.
        transition_ref: Option<TransitionRef>,
    },
    /// The exact admitted executable could not be safely reproduced.
    Unavailable,
}

impl ExactReproduction {
    /// Encodes the frozen `reproduced` replay-result variant.
    pub fn canonical_result(&self, run_id: &RunId) -> Result<CanonicalReplayResult> {
        let (result, transition_ref) = match self {
            Self::Matched => ("matched", None),
            Self::Mismatch { transition_ref } => ("mismatch", transition_ref.as_ref()),
            Self::Unavailable => ("unavailable", None),
        };
        let mut entries = vec![
            ("kind", string("reproduced")),
            ("run_id", string(run_id.as_str())),
            ("result", string(result)),
        ];
        if let Some(transition_ref) = transition_ref {
            entries.push(("transition_ref", transition_ref.canonical_value()?));
        }
        CanonicalReplayResult::encode(&object(entries)?)
    }
}

/// Closed diagnostic comparison verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum ComparisonVerdict {
    /// Candidate behavior agrees with the recorded transition.
    Agrees,
    /// Candidate behavior is interpretable and differs.
    Differs,
    /// Candidate schemas or contracts cannot interpret the recorded transition.
    NotComparable,
}

impl ComparisonVerdict {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Agrees => "agrees",
            Self::Differs => "differs",
            Self::NotComparable => "not_comparable",
        }
    }
}

/// One transition-local candidate diagnostic.
#[derive(Debug, Clone, PartialEq, Eq)]
struct CandidateTransitionComparison {
    transition_ref: TransitionRef,
    result: ComparisonVerdict,
}

impl CandidateTransitionComparison {
    const fn new(transition_ref: TransitionRef, result: ComparisonVerdict) -> Self {
        Self {
            transition_ref,
            result,
        }
    }
}

/// Exact identities and outcomes produced by a candidate comparison.
#[derive(Debug, Clone)]
struct CandidateComparisonReport {
    admitted_executable_identity_ref: ContentRef,
    candidate_executable_identity_ref: ContentRef,
    candidate_planning_profile_ref: ContentRef,
    candidate_planner_contract_ref: ContentRef,
    candidate_planner_implementation_ref: ContentRef,
    candidate_state_implementation_manifest_ref: ContentRef,
    candidate_capability_binding_manifest_ref: ContentRef,
    plan: ComparisonVerdict,
    transitions: Vec<CandidateTransitionComparison>,
}

/// Non-authoritative cross-version candidate diagnostic.
///
/// Agreement never upgrades the verified view, fact completeness, public
/// output, or append authority.
#[derive(Debug, Clone)]
struct CandidateComparison {
    run_id: RunId,
    report: CandidateComparisonReport,
}

impl CandidateComparison {
    fn new(run_id: RunId, report: CandidateComparisonReport) -> Result<Self> {
        let mut prior = None;
        let mut seen = BTreeSet::new();
        for transition in &report.transitions {
            let fields = transition.transition_ref.fields()?;
            let coordinate = (fields.run_sequence, fields.ordinal);
            if fields.run_id != run_id
                || prior.is_some_and(|prior| prior >= coordinate)
                || !seen.insert(transition.transition_ref.as_bytes().to_vec())
            {
                return Err(ReplayError::ComparisonIntegrityFailed);
            }
            prior = Some(coordinate);
        }
        Ok(Self { run_id, report })
    }

    fn canonical_result(&self) -> Result<CanonicalReplayResult> {
        let transitions = self
            .report
            .transitions
            .iter()
            .map(|transition| {
                object([
                    (
                        "transition_ref",
                        transition.transition_ref.canonical_value()?,
                    ),
                    ("result", string(transition.result.as_str())),
                ])
            })
            .collect::<Result<Vec<_>>>()?;
        let report = object([
            (
                "admitted_executable_identity_ref",
                content_ref(&self.report.admitted_executable_identity_ref)?,
            ),
            (
                "candidate_executable_identity_ref",
                content_ref(&self.report.candidate_executable_identity_ref)?,
            ),
            (
                "candidate_planning_profile_ref",
                content_ref(&self.report.candidate_planning_profile_ref)?,
            ),
            (
                "candidate_planner_contract_ref",
                content_ref(&self.report.candidate_planner_contract_ref)?,
            ),
            (
                "candidate_planner_implementation_ref",
                content_ref(&self.report.candidate_planner_implementation_ref)?,
            ),
            (
                "candidate_state_implementation_manifest_ref",
                content_ref(&self.report.candidate_state_implementation_manifest_ref)?,
            ),
            (
                "candidate_capability_binding_manifest_ref",
                content_ref(&self.report.candidate_capability_binding_manifest_ref)?,
            ),
            ("plan", string(self.report.plan.as_str())),
            ("transitions", CanonicalValue::Array(transitions)),
        ])?;
        CanonicalReplayResult::encode(&object([
            ("kind", string("candidate_comparison")),
            ("run_id", string(self.run_id.as_str())),
            (
                "candidate_executable_identity_ref",
                content_ref(&self.report.candidate_executable_identity_ref)?,
            ),
            ("report", report),
        ])?)
    }
}

fn content_ref(value: &ContentRef) -> Result<CanonicalValue> {
    object([
        ("schema_id", string(value.schema_id().as_str())),
        ("content_digest", string(value.content_digest().as_str())),
    ])
}

fn string(value: impl Into<String>) -> CanonicalValue {
    CanonicalValue::String(value.into())
}

fn object(
    entries: impl IntoIterator<Item = (impl Into<String>, CanonicalValue)>,
) -> Result<CanonicalValue> {
    CanonicalValue::object(entries).map_err(|_| ReplayError::InvalidRecordedHistory)
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use mfm_ids::{ContentDigest, JournalRecordHash, RunId, SchemaId};
    use mfm_journal::v1::{RecordRef, TransitionRef};

    use super::{
        CandidateComparison, CandidateComparisonReport, CandidateTransitionComparison,
        ComparisonVerdict,
    };

    #[test]
    fn candidate_result_binds_every_exact_identity() {
        let run_id =
            RunId::from_str(&format!("run:sha256-jcs-v1:{}", "0".repeat(64))).expect("run id");
        let report = CandidateComparisonReport {
            admitted_executable_identity_ref: content_ref("admitted", '1'),
            candidate_executable_identity_ref: content_ref("executable", '2'),
            candidate_planning_profile_ref: content_ref("profile", '3'),
            candidate_planner_contract_ref: content_ref("planner-contract", '4'),
            candidate_planner_implementation_ref: content_ref("planner-implementation", '5'),
            candidate_state_implementation_manifest_ref: content_ref("state-manifest", '6'),
            candidate_capability_binding_manifest_ref: content_ref("capability-manifest", '7'),
            plan: ComparisonVerdict::Agrees,
            transitions: vec![CandidateTransitionComparison::new(
                transition_ref(&run_id),
                ComparisonVerdict::Differs,
            )],
        };
        let result = CandidateComparison::new(run_id, report)
            .expect("candidate comparison")
            .canonical_result()
            .expect("canonical result");
        let value: serde_json::Value =
            serde_json::from_slice(result.as_bytes()).expect("candidate JSON");
        let report = value
            .get("report")
            .and_then(serde_json::Value::as_object)
            .expect("candidate report");

        for (field, digest) in [
            ("admitted_executable_identity_ref", '1'),
            ("candidate_executable_identity_ref", '2'),
            ("candidate_planning_profile_ref", '3'),
            ("candidate_planner_contract_ref", '4'),
            ("candidate_planner_implementation_ref", '5'),
            ("candidate_state_implementation_manifest_ref", '6'),
            ("candidate_capability_binding_manifest_ref", '7'),
        ] {
            let expected_digest = format!("content:sha256-v1:{}", digest.to_string().repeat(64));
            assert_eq!(
                report
                    .get(field)
                    .and_then(|value| value.get("content_digest"))
                    .and_then(serde_json::Value::as_str),
                Some(expected_digest.as_str()),
            );
        }
        assert_eq!(
            value
                .get("candidate_executable_identity_ref")
                .and_then(|value| value.get("content_digest")),
            report
                .get("candidate_executable_identity_ref")
                .and_then(|value| value.get("content_digest")),
        );
        assert_eq!(
            report.get("plan").and_then(serde_json::Value::as_str),
            Some("agrees")
        );
        assert_eq!(
            report
                .get("transitions")
                .and_then(serde_json::Value::as_array)
                .and_then(|transitions| transitions.first())
                .and_then(|transition| transition.get("result"))
                .and_then(serde_json::Value::as_str),
            Some("differs"),
        );
    }

    fn content_ref(role: &str, digest: char) -> mfm_ids::ContentRef {
        let schema_id = SchemaId::from_str(&format!(
            "schema:mfm.test-{role}:1:sha256-jcs-v1:{}",
            digest.to_string().repeat(64)
        ))
        .expect("schema id");
        let digest = ContentDigest::from_str(&format!(
            "content:sha256-v1:{}",
            digest.to_string().repeat(64)
        ))
        .expect("content digest");
        mfm_ids::ContentRef::new(schema_id, digest).expect("content ref")
    }

    fn transition_ref(run_id: &RunId) -> TransitionRef {
        let hash = JournalRecordHash::from_str(&format!("sha256-jcs-v1:{}", "8".repeat(64)))
            .expect("journal hash");
        let record_ref = RecordRef::new(run_id, 2, 0, &hash).expect("record ref");
        TransitionRef::new(&record_ref).expect("transition ref")
    }
}
