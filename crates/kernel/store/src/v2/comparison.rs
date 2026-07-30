use mfm_canonical::{PlainCanonicalJsonBytes, RecoverabilityContractV3};
use mfm_ids::{ContentRef, FieldPath, RunSemanticStateDigest};
use mfm_journal::v2::{
    AuthorizationRef, AuthorizationScopeFields, BlockingSource, CapabilityBindingRef,
    ExecutorEnsureResult, ExecutorEnsureResultFields, FactClaimEnvelope, InputManifest,
    InputManifestRef, ObservationOutcomeFields, ObservationRef, ProducerBindingFields, Settlement,
    SettlementFields, TerminalEffectEvidence, TransitionBodyFields, TransitionRef, ValueRef,
};
use mfm_spec::v1::{
    CertifiedFactSlot, CertifiedNodeContract, CertifiedStateExecution, RetainedValueContract,
};

use super::frame_preparation::validate_historical_bindings;
use super::objects::validate_value_contract;
use super::{Result, SafeFailureMetadata, StoreError, VerifiedObservedAccess, VerifiedRunView};

const READ_RESULT_PATH: &str = "outcome.result_ref";
const READ_FAILURE_PATH: &str = "outcome.safe_failure.diagnostic_ref";
const ENSURE_RESULT_PATH: &str = "executor.ensure_result";
const TERMINAL_EVIDENCE_PATH: &str = "executor.terminal_evidence";

/// Closed semantic kind of one recorded comparison transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComparisonTransitionKind {
    /// A pure callback settled.
    PureSettled,
    /// A read callback settled from one recorded observation.
    ReadSettled,
    /// A durable effect request was frozen.
    EffectRequested,
    /// A durable effect settled from terminal evidence.
    EffectSettled,
    /// An unsatisfied dependency deterministically skipped the node.
    DependencySkipped,
}

/// Closed evidence family retained for comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComparisonEvidenceKind {
    /// One audited read observation.
    Read,
    /// One audited executor ensure observation.
    TerminalEffect,
}

/// Meaning assigned to evidence by the recorded transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordedEvidenceVerdict {
    /// The recorded decision continued past this observation.
    InsufficientEvidence,
    /// The recorded decision settled from this observation.
    Settlement,
}

/// Closed result kind of one recorded callback settlement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComparisonSettlementKind {
    /// The state callback succeeded.
    Succeeded,
    /// The state callback returned its certified typed failure.
    Failed,
}

/// One exact retained canonical value admitted to semantic comparison.
pub struct VerifiedComparisonValue {
    value_ref: ValueRef,
    canonical: PlainCanonicalJsonBytes,
}

impl VerifiedComparisonValue {
    /// Returns the full producer-bound retained authority.
    pub const fn value_ref(&self) -> &ValueRef {
        &self.value_ref
    }

    /// Returns exact canonical bytes after schema and object verification.
    pub const fn canonical(&self) -> &PlainCanonicalJsonBytes {
        &self.canonical
    }
}

/// Exact historical state frame frozen by one recorded transition.
pub struct VerifiedComparisonStateFrame {
    input_manifest_ref: InputManifestRef,
    config: VerifiedComparisonValue,
    context: Option<VerifiedComparisonValue>,
    input: VerifiedComparisonValue,
}

impl VerifiedComparisonStateFrame {
    /// Returns the exact frozen input-manifest authority.
    pub const fn input_manifest_ref(&self) -> &InputManifestRef {
        &self.input_manifest_ref
    }

    /// Returns the exact verified configuration value.
    pub const fn config(&self) -> &VerifiedComparisonValue {
        &self.config
    }

    /// Returns the optional exact verified context value.
    pub const fn context(&self) -> Option<&VerifiedComparisonValue> {
        self.context.as_ref()
    }

    /// Returns the complete store-assembled input root.
    pub const fn input(&self) -> &VerifiedComparisonValue {
        &self.input
    }
}

/// One exact read outcome retained by a recorded comparison frame.
pub enum VerifiedComparisonReadOutcome {
    /// The operation returned a verified value.
    Returned(VerifiedComparisonValue),
    /// The operation proved it did not enter.
    DidNotEnter(VerifiedComparisonSafeFailure),
    /// The operation could not determine entry or outcome.
    Indeterminate(VerifiedComparisonSafeFailure),
}

/// Exact generic safe-failure metadata and optional typed diagnostic for replay comparison.
pub struct VerifiedComparisonSafeFailure {
    metadata: SafeFailureMetadata,
    diagnostic: Option<VerifiedComparisonValue>,
}

impl VerifiedComparisonSafeFailure {
    /// Returns the classifier-approved generic metadata.
    pub const fn metadata(&self) -> &SafeFailureMetadata {
        &self.metadata
    }

    /// Returns the optional exact typed diagnostic.
    pub const fn diagnostic(&self) -> Option<&VerifiedComparisonValue> {
        self.diagnostic.as_ref()
    }
}

/// Exact structurally verified terminal effect material.
pub struct VerifiedComparisonTerminalEffect {
    evidence: TerminalEffectEvidence,
    evidence_ref: ValueRef,
    retained_closure: Vec<VerifiedComparisonValue>,
}

impl VerifiedComparisonTerminalEffect {
    /// Returns the complete terminal evidence descriptor.
    pub const fn evidence(&self) -> &TerminalEffectEvidence {
        &self.evidence
    }

    /// Returns the full retained authority for that descriptor.
    pub const fn evidence_ref(&self) -> &ValueRef {
        &self.evidence_ref
    }

    /// Resolves only a member of this observation's verified executor closure.
    pub fn resolve(&self, value_ref: &ValueRef) -> Result<&VerifiedComparisonValue> {
        self.retained_closure
            .iter()
            .find(|value| value.value_ref() == value_ref)
            .ok_or(StoreError::ObjectNotReachable)
    }
}

/// One exact observation and the verdict fixed by recorded history.
pub struct VerifiedComparisonEvidence {
    observation_ref: ObservationRef,
    kind: ComparisonEvidenceKind,
    read_outcome: Option<VerifiedComparisonReadOutcome>,
    terminal_effect: Option<VerifiedComparisonTerminalEffect>,
    recorded_verdict: RecordedEvidenceVerdict,
}

impl VerifiedComparisonEvidence {
    /// Returns the exact observation record reference.
    pub const fn observation_ref(&self) -> &ObservationRef {
        &self.observation_ref
    }

    /// Returns the closed evidence family.
    pub const fn kind(&self) -> ComparisonEvidenceKind {
        self.kind
    }

    /// Returns a read outcome only for read evidence.
    pub const fn read_outcome(&self) -> Option<&VerifiedComparisonReadOutcome> {
        self.read_outcome.as_ref()
    }

    /// Returns terminal evidence only for a terminal executor result.
    pub const fn terminal_effect(&self) -> Option<&VerifiedComparisonTerminalEffect> {
        self.terminal_effect.as_ref()
    }

    /// Returns the verdict selected by the recorded transition.
    pub const fn recorded_verdict(&self) -> RecordedEvidenceVerdict {
        self.recorded_verdict
    }
}

/// One normalized successful output.
pub struct VerifiedComparisonOutput {
    output_ordinal: u32,
    field_path: FieldPath,
    retained_contract: RetainedValueContract,
    value: VerifiedComparisonValue,
}

impl VerifiedComparisonOutput {
    /// Returns the certified output ordinal.
    pub const fn output_ordinal(&self) -> u32 {
        self.output_ordinal
    }

    /// Returns the certified output field path.
    pub const fn field_path(&self) -> &FieldPath {
        &self.field_path
    }

    /// Returns the complete certified retained-value contract.
    pub const fn retained_contract(&self) -> &RetainedValueContract {
        &self.retained_contract
    }

    /// Returns the exact recorded output value.
    pub const fn value(&self) -> &VerifiedComparisonValue {
        &self.value
    }
}

/// One normalized successful fact emission.
pub struct VerifiedComparisonFact {
    emission_ordinal: u32,
    fact_slot_ordinal: u32,
    descriptor_ref: ContentRef,
    subject: VerifiedComparisonValue,
    response: VerifiedComparisonValue,
}

impl VerifiedComparisonFact {
    /// Returns the dense emission ordinal.
    pub const fn emission_ordinal(&self) -> u32 {
        self.emission_ordinal
    }

    /// Returns the certified homogeneous fact-slot ordinal.
    pub const fn fact_slot_ordinal(&self) -> u32 {
        self.fact_slot_ordinal
    }

    /// Returns the exact certified fact descriptor.
    pub const fn descriptor_ref(&self) -> &ContentRef {
        &self.descriptor_ref
    }

    /// Returns the exact fact subject.
    pub const fn subject(&self) -> &VerifiedComparisonValue {
        &self.subject
    }

    /// Returns the exact fact response.
    pub const fn response(&self) -> &VerifiedComparisonValue {
        &self.response
    }
}

/// One normalized callback settlement.
pub struct VerifiedComparisonSettlement {
    kind: ComparisonSettlementKind,
    outputs: Vec<VerifiedComparisonOutput>,
    facts: Vec<VerifiedComparisonFact>,
    failure: Option<VerifiedComparisonValue>,
}

impl VerifiedComparisonSettlement {
    /// Returns the closed settlement kind.
    pub const fn kind(&self) -> ComparisonSettlementKind {
        self.kind
    }

    /// Returns successful outputs in certified ordinal order.
    pub fn outputs(&self) -> &[VerifiedComparisonOutput] {
        &self.outputs
    }

    /// Returns successful facts in dense emission order.
    pub fn facts(&self) -> &[VerifiedComparisonFact] {
        &self.facts
    }

    /// Returns the exact typed failure only for a failed settlement.
    pub const fn failure(&self) -> Option<&VerifiedComparisonValue> {
        self.failure.as_ref()
    }
}

/// One complete, callback-free historical transition frame.
pub struct VerifiedComparisonFrame<'view> {
    transition_ref: TransitionRef,
    containing_journal_head: mfm_journal::v2::JournalHead,
    certified_node: &'view CertifiedNodeContract,
    kind: ComparisonTransitionKind,
    frame: Option<VerifiedComparisonStateFrame>,
    request_transition_ref: Option<TransitionRef>,
    request: Option<VerifiedComparisonValue>,
    evidence: Vec<VerifiedComparisonEvidence>,
    recorded_settlement: Option<VerifiedComparisonSettlement>,
    blocking_sources: Vec<BlockingSource>,
    resulting_run_state_digest: RunSemanticStateDigest,
}

impl<'view> VerifiedComparisonFrame<'view> {
    /// Returns the exact assigned transition reference.
    pub const fn transition_ref(&self) -> &TransitionRef {
        &self.transition_ref
    }

    /// Returns the physical head containing this transition.
    pub const fn containing_journal_head(&self) -> &mfm_journal::v2::JournalHead {
        &self.containing_journal_head
    }

    /// Returns the exact certified node occurrence.
    pub const fn certified_node(&self) -> &'view CertifiedNodeContract {
        self.certified_node
    }

    /// Returns the closed transition kind.
    pub const fn kind(&self) -> ComparisonTransitionKind {
        self.kind
    }

    /// Returns the exact historical input-manifest reference when a frame exists.
    pub fn input_manifest_ref(&self) -> Option<&InputManifestRef> {
        self.frame.as_ref().map(|frame| &frame.input_manifest_ref)
    }

    /// Returns the original effect-request transition only for effect settlement.
    pub const fn request_transition_ref(&self) -> Option<&TransitionRef> {
        self.request_transition_ref.as_ref()
    }

    /// Returns the exact historical state frame; dependency skips have none.
    pub const fn frame(&self) -> Option<&VerifiedComparisonStateFrame> {
        self.frame.as_ref()
    }

    /// Returns the read or effect request; pure settlements and skips have none.
    pub const fn request(&self) -> Option<&VerifiedComparisonValue> {
        self.request.as_ref()
    }

    /// Returns the exact recorded evidence prefix ending in one settlement verdict.
    pub fn evidence(&self) -> &[VerifiedComparisonEvidence] {
        &self.evidence
    }

    /// Returns the normalized recorded settlement when this transition settled.
    pub const fn recorded_settlement(&self) -> Option<&VerifiedComparisonSettlement> {
        self.recorded_settlement.as_ref()
    }

    /// Returns the deterministic blocking set only for dependency skip.
    pub fn blocking_sources(&self) -> &[BlockingSource] {
        &self.blocking_sources
    }

    /// Returns the exact semantic state digest after this transition.
    pub const fn resulting_run_state_digest(&self) -> &RunSemanticStateDigest {
        &self.resulting_run_state_digest
    }
}

/// Eager sealed traversal over all verified historical comparison frames.
pub struct VerifiedComparisonFrameReader<'view> {
    frames: Vec<VerifiedComparisonFrame<'view>>,
}

impl<'view> VerifiedComparisonFrameReader<'view> {
    /// Returns complete frames in committed semantic order.
    pub fn frames(
        &self,
    ) -> impl ExactSizeIterator<Item = &VerifiedComparisonFrame<'view>> + DoubleEndedIterator {
        self.frames.iter()
    }
}

impl VerifiedRunView {
    /// Reconstructs every exact historical transition frame without live callbacks or ambient IO.
    pub fn comparison_frames(&self) -> Result<VerifiedComparisonFrameReader<'_>> {
        let frames = self
            .transition_entries()
            .map(|entry| comparison_frame(self, entry))
            .collect::<Result<Vec<_>>>()?;
        Ok(VerifiedComparisonFrameReader { frames })
    }
}

pub(super) fn comparison_frame<'view>(
    view: &'view VerifiedRunView,
    entry: &super::FoldedTransitionEntry,
) -> Result<VerifiedComparisonFrame<'view>> {
    let transition = entry.transition().fields()?;
    let node = view
        .certified_spec()
        .nodes()
        .iter()
        .find(|node| node.node_id() == &transition.node_id)
        .ok_or(StoreError::TransitionFoldMismatch {
            field: "comparison_certified_node",
        })?;
    let resulting_run_state_digest = transition.after.fields()?.run_state_digest;
    let transition_ref = entry.transition_ref().clone();
    let containing_journal_head = entry.containing_journal_head().clone();

    let (
        kind,
        frame,
        request_transition_ref,
        request,
        evidence,
        recorded_settlement,
        blocking_sources,
    ) = match transition.body.fields()? {
        TransitionBodyFields::PureSettled {
            input_manifest_ref,
            settlement,
        } => (
            ComparisonTransitionKind::PureSettled,
            Some(comparison_state_frame(view, node, input_manifest_ref)?),
            None,
            None,
            Vec::new(),
            Some(comparison_settlement(view, node, settlement)?),
            Vec::new(),
        ),
        TransitionBodyFields::ReadSettled {
            input_manifest_ref,
            request_ref,
            consumed_observation_ref,
            settlement,
        } => {
            let CertifiedStateExecution::Read {
                request_contract,
                returned_contract,
                safe_failure_contract,
                ..
            } = node.execution()
            else {
                return Err(comparison_mismatch("comparison_read_contract"));
            };
            let frame = comparison_state_frame(view, node, input_manifest_ref.clone())?;
            let request = comparison_value(view, request_ref.clone(), request_contract)?;
            let evidence = read_evidence(
                view,
                node,
                &input_manifest_ref,
                &request_ref,
                &consumed_observation_ref,
                returned_contract,
                safe_failure_contract,
            )?;
            (
                ComparisonTransitionKind::ReadSettled,
                Some(frame),
                None,
                Some(request),
                evidence,
                Some(comparison_settlement(view, node, settlement)?),
                Vec::new(),
            )
        }
        TransitionBodyFields::EffectRequested {
            input_manifest_ref,
            semantic_request_ref,
            executor_binding_ref,
            ..
        } => {
            let CertifiedStateExecution::Effect {
                request_contract,
                executor_binding_ref: certified_binding,
                ..
            } = node.execution()
            else {
                return Err(comparison_mismatch("comparison_effect_contract"));
            };
            if executor_binding_ref.fields()? != *certified_binding {
                return Err(comparison_mismatch("comparison_effect_binding"));
            }
            (
                ComparisonTransitionKind::EffectRequested,
                Some(comparison_state_frame(view, node, input_manifest_ref)?),
                None,
                Some(comparison_value(
                    view,
                    semantic_request_ref,
                    request_contract,
                )?),
                Vec::new(),
                None,
                Vec::new(),
            )
        }
        TransitionBodyFields::EffectSettled {
            request_transition_ref,
            request_input_manifest_ref,
            consumed_terminal_observation_ref,
            settlement,
        } => {
            let request_entry = view
                .transition_entries()
                .find(|candidate| candidate.transition_ref() == &request_transition_ref)
                .ok_or_else(|| comparison_mismatch("comparison_effect_request"))?;
            let request_transition = request_entry.transition().fields()?;
            if request_transition.node_id != transition.node_id {
                return Err(comparison_mismatch("comparison_effect_request"));
            }
            let TransitionBodyFields::EffectRequested {
                input_manifest_ref,
                effect_key,
                semantic_request_ref,
                request_digest,
                executor_binding_ref,
            } = request_transition.body.fields()?
            else {
                return Err(comparison_mismatch("comparison_effect_request"));
            };
            let CertifiedStateExecution::Effect {
                request_contract,
                executor_binding_ref: certified_binding,
                ensure_result_contract,
                terminal_evidence_contract,
                ..
            } = node.execution()
            else {
                return Err(comparison_mismatch("comparison_effect_contract"));
            };
            if input_manifest_ref != request_input_manifest_ref
                || executor_binding_ref.fields()? != *certified_binding
            {
                return Err(comparison_mismatch("comparison_effect_request"));
            }
            let frame = comparison_state_frame(view, node, request_input_manifest_ref)?;
            let request = comparison_value(view, semantic_request_ref.clone(), request_contract)?;
            let evidence = effect_evidence(
                view,
                node,
                &request_transition_ref,
                &semantic_request_ref,
                &consumed_terminal_observation_ref,
                &executor_binding_ref,
                &effect_key,
                &request_digest,
                ensure_result_contract,
                terminal_evidence_contract,
            )?;
            (
                ComparisonTransitionKind::EffectSettled,
                Some(frame),
                Some(request_transition_ref),
                Some(request),
                evidence,
                Some(comparison_settlement(view, node, settlement)?),
                Vec::new(),
            )
        }
        TransitionBodyFields::DependencySkipped { blocking_sources } => (
            ComparisonTransitionKind::DependencySkipped,
            None,
            None,
            None,
            Vec::new(),
            None,
            blocking_sources,
        ),
    };

    Ok(VerifiedComparisonFrame {
        transition_ref,
        containing_journal_head,
        certified_node: node,
        kind,
        frame,
        request_transition_ref,
        request,
        evidence,
        recorded_settlement,
        blocking_sources,
        resulting_run_state_digest,
    })
}

fn comparison_state_frame(
    view: &VerifiedRunView,
    node: &CertifiedNodeContract,
    input_manifest_ref: InputManifestRef,
) -> Result<VerifiedComparisonStateFrame> {
    validate_value_contract(
        view.certified_spec()
            .journal_protocol_contracts()
            .input_manifest_contract(),
        &input_manifest_ref.value_ref()?,
    )?;
    let object = view.input_manifest_object(&input_manifest_ref)?;
    let manifest = InputManifest::strict_decode(object.bytes())?;
    let fields = manifest.fields()?;
    if fields.input_schema_id != *node.input_contract().schema_id()
        || fields.bindings.len() != node.input_bindings().len()
    {
        return Err(comparison_mismatch("comparison_input_manifest"));
    }
    let config_ref = fields
        .config_ref
        .ok_or_else(|| comparison_mismatch("comparison_config"))?;
    let config = comparison_value(view, config_ref, node.config_binding().value_contract())?;
    let context = match (fields.context_ref, node.context_binding()) {
        (Some(value_ref), Some(binding)) => {
            Some(comparison_value(view, value_ref, binding.value_contract())?)
        }
        (None, None) => None,
        _ => return Err(comparison_mismatch("comparison_context")),
    };
    let input_object = view.retained_value(&fields.root_input_ref)?;
    validate_historical_bindings(view, node, &fields.bindings, input_object.bytes())?;
    let input = comparison_value(view, fields.root_input_ref, node.input_contract())?;
    Ok(VerifiedComparisonStateFrame {
        input_manifest_ref,
        config,
        context,
        input,
    })
}

fn comparison_value(
    view: &VerifiedRunView,
    value_ref: ValueRef,
    contract: &RetainedValueContract,
) -> Result<VerifiedComparisonValue> {
    validate_value_contract(contract, &value_ref)?;
    comparison_uncontracted_value(view, value_ref)
}

fn comparison_uncontracted_value(
    view: &VerifiedRunView,
    value_ref: ValueRef,
) -> Result<VerifiedComparisonValue> {
    let object = view.retained_value(&value_ref)?;
    RecoverabilityContractV3::embedded()?
        .strict_decode("mfm.primitive-canonical_value.v1", object.bytes())?;
    let canonical = PlainCanonicalJsonBytes::from_canonical_json_slice(object.bytes())
        .map_err(|_| StoreError::JournalContract)?;
    Ok(VerifiedComparisonValue {
        value_ref,
        canonical,
    })
}

fn read_evidence(
    view: &VerifiedRunView,
    node: &CertifiedNodeContract,
    input_manifest_ref: &InputManifestRef,
    request_ref: &ValueRef,
    consumed: &ObservationRef,
    returned_contract: &RetainedValueContract,
    safe_failure_contract: &RetainedValueContract,
) -> Result<Vec<VerifiedComparisonEvidence>> {
    let history = view.access_history(node.node_id())?;
    for attempt in history.entries() {
        let fields = attempt.authorization().fields()?;
        if fields.request_ref != *request_ref
            || !matches!(
                fields.scope.fields()?,
                AuthorizationScopeFields::Read {
                    input_manifest_ref: ref actual,
                } if actual == input_manifest_ref
            )
        {
            return Err(comparison_mismatch("comparison_read_history"));
        }
    }
    let observed = history.observed_suffix().collect::<Vec<_>>();
    require_consumed_tail(&observed, consumed)?;
    observed
        .into_iter()
        .map(|observed| {
            let recorded_verdict = verdict(observed.observation_ref(), consumed);
            let read_outcome =
                comparison_read_outcome(view, &observed, returned_contract, safe_failure_contract)?;
            Ok(VerifiedComparisonEvidence {
                observation_ref: observed.observation_ref().clone(),
                kind: ComparisonEvidenceKind::Read,
                read_outcome: Some(read_outcome),
                terminal_effect: None,
                recorded_verdict,
            })
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn effect_evidence(
    view: &VerifiedRunView,
    node: &CertifiedNodeContract,
    request_transition_ref: &TransitionRef,
    request_ref: &ValueRef,
    consumed: &ObservationRef,
    executor_binding_ref: &CapabilityBindingRef,
    effect_key: &mfm_ids::EffectKey,
    request_digest: &mfm_ids::RequestDigest,
    ensure_result_contract: &RetainedValueContract,
    terminal_evidence_contract: &RetainedValueContract,
) -> Result<Vec<VerifiedComparisonEvidence>> {
    let history = view.access_history(node.node_id())?;
    for attempt in history.entries() {
        let fields = attempt.authorization().fields()?;
        if fields.request_ref != *request_ref
            || fields.frozen_read_intent_ref.is_some()
            || !matches!(
                fields.scope.fields()?,
                AuthorizationScopeFields::EnsureEffect {
                    effect_request_transition_ref: ref actual,
                } if actual == request_transition_ref
            )
        {
            return Err(comparison_mismatch("comparison_effect_history"));
        }
    }
    let observed = history.observed_suffix().collect::<Vec<_>>();
    require_consumed_tail(&observed, consumed)?;
    observed
        .into_iter()
        .map(|observed| {
            let recorded_verdict = verdict(observed.observation_ref(), consumed);
            let terminal_effect = comparison_terminal_effect(
                view,
                &observed,
                executor_binding_ref,
                effect_key,
                request_digest,
                ensure_result_contract,
                terminal_evidence_contract,
            )?;
            match (recorded_verdict, terminal_effect.is_some()) {
                (RecordedEvidenceVerdict::InsufficientEvidence, false)
                | (RecordedEvidenceVerdict::Settlement, true) => {}
                _ => return Err(comparison_mismatch("comparison_effect_verdict")),
            }
            Ok(VerifiedComparisonEvidence {
                observation_ref: observed.observation_ref().clone(),
                kind: ComparisonEvidenceKind::TerminalEffect,
                read_outcome: None,
                terminal_effect,
                recorded_verdict,
            })
        })
        .collect()
}

fn require_consumed_tail(
    observed: &[VerifiedObservedAccess<'_>],
    consumed: &ObservationRef,
) -> Result<()> {
    if observed
        .last()
        .is_none_or(|observed| observed.observation_ref() != consumed)
        || observed[..observed.len().saturating_sub(1)]
            .iter()
            .any(|observed| observed.observation_ref() == consumed)
    {
        return Err(comparison_mismatch("comparison_consumed_observation"));
    }
    Ok(())
}

fn verdict(observation_ref: &ObservationRef, consumed: &ObservationRef) -> RecordedEvidenceVerdict {
    if observation_ref == consumed {
        RecordedEvidenceVerdict::Settlement
    } else {
        RecordedEvidenceVerdict::InsufficientEvidence
    }
}

fn comparison_read_outcome(
    view: &VerifiedRunView,
    observed: &VerifiedObservedAccess<'_>,
    returned_contract: &RetainedValueContract,
    safe_failure_contract: &RetainedValueContract,
) -> Result<VerifiedComparisonReadOutcome> {
    let observation = observed.observation().fields()?;
    if observation.authorization_ref != *observed.audit().authorization_ref() {
        return Err(comparison_mismatch("comparison_read_observation"));
    }
    match observation.outcome.fields()? {
        ObservationOutcomeFields::Returned { result_ref } => observed_value(
            view,
            observed.audit().authorization_ref(),
            READ_RESULT_PATH,
            result_ref,
            returned_contract,
        )
        .map(VerifiedComparisonReadOutcome::Returned),
        ObservationOutcomeFields::DidNotEnter { safe_failure } => safe_failure_value(
            view,
            observed.audit().authorization_ref(),
            safe_failure,
            safe_failure_contract,
        )
        .map(VerifiedComparisonReadOutcome::DidNotEnter),
        ObservationOutcomeFields::Indeterminate { safe_failure } => safe_failure_value(
            view,
            observed.audit().authorization_ref(),
            safe_failure,
            safe_failure_contract,
        )
        .map(VerifiedComparisonReadOutcome::Indeterminate),
        ObservationOutcomeFields::NonDomainFailure { .. } => {
            Err(comparison_mismatch("comparison_non_domain_failure"))
        }
    }
}

fn safe_failure_value(
    view: &VerifiedRunView,
    authorization_ref: &AuthorizationRef,
    safe_failure: mfm_journal::v2::SafeFailure,
    contract: &RetainedValueContract,
) -> Result<VerifiedComparisonSafeFailure> {
    let fields = safe_failure.fields()?;
    let metadata = SafeFailureMetadata::new(
        fields.safe_failure_contract_ref,
        fields.stable_code,
        fields.failure_class,
        fields.boundary_stage,
        fields.coarse_size_class,
    );
    let diagnostic = fields
        .diagnostic_ref
        .map(|diagnostic_ref| {
            observed_value(
                view,
                authorization_ref,
                READ_FAILURE_PATH,
                diagnostic_ref,
                contract,
            )
        })
        .transpose()?;
    Ok(VerifiedComparisonSafeFailure {
        metadata,
        diagnostic,
    })
}

fn observed_value(
    view: &VerifiedRunView,
    authorization_ref: &AuthorizationRef,
    path: &str,
    value_ref: ValueRef,
    contract: &RetainedValueContract,
) -> Result<VerifiedComparisonValue> {
    validate_value_contract(contract, &value_ref)?;
    let expected_path = FieldPath::new(path)?;
    match value_ref.fields()?.producer_binding.fields()? {
        ProducerBindingFields::ExternalObservation {
            authorization_ref: actual_authorization,
            field_path,
        } if actual_authorization == *authorization_ref && field_path == expected_path => {}
        _ => return Err(comparison_mismatch("comparison_observation_value")),
    }
    comparison_uncontracted_value(view, value_ref)
}

#[allow(clippy::too_many_arguments)]
fn comparison_terminal_effect(
    view: &VerifiedRunView,
    observed: &VerifiedObservedAccess<'_>,
    executor_binding_ref: &CapabilityBindingRef,
    effect_key: &mfm_ids::EffectKey,
    request_digest: &mfm_ids::RequestDigest,
    ensure_result_contract: &RetainedValueContract,
    terminal_evidence_contract: &RetainedValueContract,
) -> Result<Option<VerifiedComparisonTerminalEffect>> {
    let authorization_ref = observed.audit().authorization_ref();
    let observation = observed.observation().fields()?;
    if observation.authorization_ref != *authorization_ref
        || observation.fact_selection_scan_attestation_ref.is_some()
    {
        return Err(comparison_mismatch("comparison_effect_observation"));
    }
    let ObservationOutcomeFields::Returned { result_ref } = observation.outcome.fields()? else {
        return Err(comparison_mismatch("comparison_effect_observation"));
    };
    let ensure_value = observed_value(
        view,
        authorization_ref,
        ENSURE_RESULT_PATH,
        result_ref,
        ensure_result_contract,
    )?;
    let ensure = ExecutorEnsureResult::strict_decode(ensure_value.canonical().as_bytes())?;
    let ExecutorEnsureResultFields::Terminal { evidence_ref } = ensure.fields()? else {
        return Ok(None);
    };
    let evidence_value = observed_value(
        view,
        authorization_ref,
        TERMINAL_EVIDENCE_PATH,
        evidence_ref.clone(),
        terminal_evidence_contract,
    )?;
    let evidence = TerminalEffectEvidence::strict_decode(evidence_value.canonical().as_bytes())?;
    let fields = evidence.fields()?;
    if fields.executor_binding_ref != *executor_binding_ref
        || fields.effect_key != *effect_key
        || fields.request_digest != *request_digest
    {
        return Err(comparison_mismatch("comparison_effect_identity"));
    }
    let mut retained_closure = view
        .journal()
        .objects()
        .filter_map(|object| {
            let value_ref = object.value_ref();
            let fields = value_ref.fields().ok()?;
            let ProducerBindingFields::ExternalObservation {
                authorization_ref: producer_authorization,
                field_path,
            } = fields.producer_binding.fields().ok()?
            else {
                return None;
            };
            let digest = fields.content_digest.digest().to_string();
            (producer_authorization == *authorization_ref && executor_path(&field_path, &digest))
                .then(|| comparison_uncontracted_value(view, value_ref.clone()))
        })
        .collect::<Result<Vec<_>>>()?;
    retained_closure
        .sort_by(|left, right| left.value_ref.as_bytes().cmp(right.value_ref.as_bytes()));
    if retained_closure
        .windows(2)
        .any(|pair| pair[0].value_ref == pair[1].value_ref)
        || [
            &fields.delivery_audit_ref,
            &fields.terminal_tombstone_ref,
            &fields.domain_evidence_ref,
        ]
        .iter()
        .any(|required| {
            !retained_closure
                .iter()
                .any(|value| value.value_ref() == *required)
        })
    {
        return Err(comparison_mismatch("comparison_effect_closure"));
    }
    Ok(Some(VerifiedComparisonTerminalEffect {
        evidence,
        evidence_ref,
        retained_closure,
    }))
}

pub(super) fn executor_path(path: &FieldPath, content_digest: &str) -> bool {
    let path = path.as_str();
    path == ENSURE_RESULT_PATH
        || path == TERMINAL_EVIDENCE_PATH
        || [
            "executor.delivery_audit.",
            "executor.frontier.",
            "executor.terminal_tombstone.",
            "executor.terminal_proof.",
            "executor.domain_evidence.",
        ]
        .iter()
        .any(|prefix| path.strip_prefix(prefix) == Some(content_digest))
}

fn comparison_settlement(
    view: &VerifiedRunView,
    node: &CertifiedNodeContract,
    settlement: Settlement,
) -> Result<VerifiedComparisonSettlement> {
    match settlement.fields()? {
        SettlementFields::Succeeded {
            output_bindings,
            fact_emissions,
        } => {
            let slots = node.settlement_contract().output_slots();
            if output_bindings.len() != slots.len() {
                return Err(comparison_mismatch("comparison_outputs"));
            }
            let outputs = output_bindings
                .into_iter()
                .zip(slots)
                .map(|(binding, slot)| {
                    let fields = binding.fields()?;
                    if fields.output_ordinal != slot.output_ordinal()
                        || fields.field_path != *slot.field_path()
                    {
                        return Err(comparison_mismatch("comparison_output_slot"));
                    }
                    verify_output_producer(view, node, &fields.value_ref, fields.output_ordinal)?;
                    Ok(VerifiedComparisonOutput {
                        output_ordinal: fields.output_ordinal,
                        field_path: fields.field_path,
                        retained_contract: slot.value_contract().clone(),
                        value: comparison_value(view, fields.value_ref, slot.value_contract())?,
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            let facts = comparison_facts(view, node, fact_emissions)?;
            Ok(VerifiedComparisonSettlement {
                kind: ComparisonSettlementKind::Succeeded,
                outputs,
                facts,
                failure: None,
            })
        }
        SettlementFields::Failed { typed_failure_ref } => {
            let contract = node
                .settlement_contract()
                .typed_failure_contract()
                .ok_or_else(|| comparison_mismatch("comparison_failure_contract"))?;
            Ok(VerifiedComparisonSettlement {
                kind: ComparisonSettlementKind::Failed,
                outputs: Vec::new(),
                facts: Vec::new(),
                failure: Some(comparison_value(view, typed_failure_ref, contract)?),
            })
        }
    }
}

fn verify_output_producer(
    view: &VerifiedRunView,
    node: &CertifiedNodeContract,
    value_ref: &ValueRef,
    output_ordinal: u32,
) -> Result<()> {
    match value_ref.fields()?.producer_binding.fields()? {
        ProducerBindingFields::TransitionOutput {
            run_id,
            node_id,
            output_ordinal: actual,
        } if run_id == *view.run_id() && node_id == *node.node_id() && actual == output_ordinal => {
            Ok(())
        }
        _ => Err(comparison_mismatch("comparison_output_producer")),
    }
}

fn comparison_facts(
    view: &VerifiedRunView,
    node: &CertifiedNodeContract,
    emissions: Vec<mfm_journal::v2::FactEmission>,
) -> Result<Vec<VerifiedComparisonFact>> {
    let slots = node.settlement_contract().fact_slots();
    let mut counts = vec![0_u32; slots.len()];
    let facts = emissions
        .into_iter()
        .map(|emission| {
            let fields = emission.fields()?;
            let slot_index = usize::try_from(fields.fact_slot_ordinal)
                .map_err(|_| StoreError::SequenceOverflow)?;
            let slot = slots
                .get(slot_index)
                .ok_or_else(|| comparison_mismatch("comparison_fact_slot"))?;
            if slot.fact_slot_ordinal() != fields.fact_slot_ordinal
                || slot.fact_descriptor_ref() != &fields.fact_descriptor_ref
            {
                return Err(comparison_mismatch("comparison_fact_slot"));
            }
            counts[slot_index] = counts[slot_index]
                .checked_add(1)
                .ok_or(StoreError::SequenceOverflow)?;
            let claim_value = view.retained_value(&fields.claim_ref)?;
            let claim = FactClaimEnvelope::strict_decode(claim_value.bytes())?.fields()?;
            if claim.fact_descriptor_ref != fields.fact_descriptor_ref {
                return Err(comparison_mismatch("comparison_fact_claim"));
            }
            verify_fact_producer(
                view,
                node,
                &claim.subject_ref,
                fields.emission_ordinal,
                mfm_journal::v2::FactValueComponent::Subject,
            )?;
            verify_fact_producer(
                view,
                node,
                &claim.response_ref,
                fields.emission_ordinal,
                mfm_journal::v2::FactValueComponent::Response,
            )?;
            Ok(VerifiedComparisonFact {
                emission_ordinal: fields.emission_ordinal,
                fact_slot_ordinal: fields.fact_slot_ordinal,
                descriptor_ref: fields.fact_descriptor_ref,
                subject: comparison_value(view, claim.subject_ref, slot.subject_contract())?,
                response: comparison_value(view, claim.response_ref, slot.response_contract())?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    validate_fact_counts(slots, &counts)?;
    Ok(facts)
}

fn validate_fact_counts(slots: &[CertifiedFactSlot], counts: &[u32]) -> Result<()> {
    if slots
        .iter()
        .zip(counts)
        .any(|(slot, count)| *count < slot.minimum_emissions() || *count > slot.maximum_emissions())
    {
        Err(comparison_mismatch("comparison_fact_count"))
    } else {
        Ok(())
    }
}

fn verify_fact_producer(
    view: &VerifiedRunView,
    node: &CertifiedNodeContract,
    value_ref: &ValueRef,
    emission_ordinal: u32,
    component: mfm_journal::v2::FactValueComponent,
) -> Result<()> {
    match value_ref.fields()?.producer_binding.fields()? {
        ProducerBindingFields::TransitionFact {
            run_id,
            node_id,
            emission_ordinal: actual,
            component: actual_component,
        } if run_id == *view.run_id()
            && node_id == *node.node_id()
            && actual == emission_ordinal
            && actual_component == component =>
        {
            Ok(())
        }
        _ => Err(comparison_mismatch("comparison_fact_producer")),
    }
}

const fn comparison_mismatch(field: &'static str) -> StoreError {
    StoreError::PersistedMismatch { field }
}
