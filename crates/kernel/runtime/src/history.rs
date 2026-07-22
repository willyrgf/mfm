use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;

use mfm_events::v1 as events;
use mfm_ids::{
    ArtifactId, AttemptId, CellId, ContentDigest, NodeId, RunId, SideEffectPairId, SpecHash,
};
use mfm_manual_auth::manual_authorization_proof_schema_id;
use mfm_spec::v1 as spec;
use mfm_store::v1 as store;

use crate::artifacts::{artifact_role_name, staged_artifact_binding_kind};
use crate::binding::{BoundRuntimeContext, BoundRuntimeContextLoader};
use crate::commit::SealedTerminalCommitValidation;
use crate::error::async_store_error;
use crate::framework::{
    build_retention_manifest_artifact, certified_complete_run_node,
    certified_resolve_saga_terminal_node, certified_retention_manifest_node,
    complete_run_receipt_json, projected_retention_manifest, public_output_receipt_digest,
    public_output_rendered_digest, resolve_saga_terminal_receipt_json,
    retention_manifest_receipt_json, run_completion_evidence, saga_terminal_completion_outcome,
};
use crate::manual_resolution::certified_manual_resolution_spec;
use crate::recovery::AttemptRecoveryLifecycle;
use crate::side_effects::{
    node_uses_side_effect_terminal_validation, side_effect_payload_ref,
    validate_atomic_side_effect_failure_pairs, validate_historical_side_effect_failure,
    validate_historical_side_effect_payload, validate_historical_side_effect_terminal,
    HistoricalSideEffectLedger,
};
use crate::{
    config_ref_key, validate_public_output, validate_public_output_render_node,
    CertifiedRuntimeSpec, MaterializedCell, MaterializedCellTerminal, MaterializedInputNode,
    MaterializedInputs, NamedMaterializedInput, Result, RuntimeError,
};

#[path = "history_artifacts.rs"]
mod history_artifacts;
#[path = "history_inputs.rs"]
mod history_inputs;
#[path = "history_validation.rs"]
mod history_validation;

use self::history_artifacts::{
    artifact_refs_from_stream, committed_input_artifact, config_artifacts_from_run_admitted,
    store_artifact_from_event_ref, store_artifact_from_run_ref,
};
pub(crate) use self::history_artifacts::{
    committed_config_artifact, event_artifact_ref_from_store, payload_spec_hash,
    run_artifact_ref_from_store, store_seed_artifact,
};
#[cfg(test)]
use self::history_inputs::raw_stream_requires_artifact_byte_authority;
pub(crate) use self::history_inputs::{materialize_inputs, validate_seed_cells};
pub(crate) use self::history_validation::{
    validate_certificate_artifact, validate_config_artifacts, validate_spec_artifact,
};
use self::history_validation::{validate_historical_run_stream, validate_run_identity_material};

#[derive(Debug, Clone)]
pub(crate) struct RuntimeRunView {
    pub(crate) stream: Vec<store::KernelEventEnvelope>,
    pub(crate) projections: store::ProjectionSnapshot,
    pub(crate) run_admitted: events::RunAdmitted,
    pub(crate) seed_cells: BTreeMap<CellId, events::SeedCellRef>,
    pub(crate) config_artifacts: BTreeMap<String, store::ArtifactEvidenceRef>,
    pub(crate) artifact_refs: BTreeMap<store::ArtifactAuthorityKey, CommittedArtifactReference>,
    pub(crate) artifact_byte_authority: store::ArtifactByteAuthorityMap,
    pub(crate) next_seq: store::StreamSeq,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CommittedArtifactReference {
    pub(crate) evidence: store::ArtifactEvidenceRef,
    pub(crate) attempt_id: Option<AttemptId>,
    pub(crate) commit_seq: store::StreamSeq,
    pub(crate) commit_key: store::CommitKey,
}

#[derive(Debug, Clone)]
struct RuntimeCommittedHistory {
    commits: Vec<RuntimeCommittedBatch>,
}

impl RuntimeCommittedHistory {
    fn from_committed_stream(committed: &store::CommittedRunStream) -> Self {
        let mut start = 0;
        let commits = committed
            .commits()
            .iter()
            .map(|commit| {
                let end = start + commit.events().len();
                let batch = RuntimeCommittedBatch {
                    event_range: start..end,
                };
                start = end;
                batch
            })
            .collect();
        Self { commits }
    }

    fn commits(&self) -> &[RuntimeCommittedBatch] {
        &self.commits
    }
}

#[derive(Debug, Clone)]
struct RuntimeCommittedBatch {
    event_range: Range<usize>,
}

impl RuntimeCommittedBatch {
    fn events<'a>(
        &self,
        stream: &'a [store::KernelEventEnvelope],
    ) -> &'a [store::KernelEventEnvelope] {
        &stream[self.event_range.clone()]
    }

    fn prefix<'a>(
        &self,
        stream: &'a [store::KernelEventEnvelope],
    ) -> &'a [store::KernelEventEnvelope] {
        &stream[..self.event_range.start]
    }
}

/// Shared verified run-history view for runtime, replay, and app read paths.
///
/// This view is minted only from a store-owned committed run stream, certified runtime authority,
/// and verified retained artifact evidence. It keeps the runtime fold private while exposing stable
/// accessors for consumers that need read authority without rebuilding raw stream projections.
#[derive(Debug, Clone)]
pub struct VerifiedRunHistoryView {
    run_id: RunId,
    spec_hash: SpecHash,
    committed: store::CommittedRunStream,
    artifacts: store::VerifiedRunArtifactStore,
    runtime: RuntimeRunView,
}

/// Scheduler-owned verified run context for transition and attempt dispatch.
///
/// This authority combines the bound runtime context for the certified spec with the store-owned
/// committed stream and runtime view. Scheduler drive paths use this loader as their typed
/// ingress boundary instead of rebuilding raw stream views directly.
#[derive(Clone)]
pub struct VerifiedRunContext {
    run_id: RunId,
    spec_hash: SpecHash,
    bound_context: BoundRuntimeContext,
    committed: store::CommittedRunStream,
    view: RuntimeRunView,
}

impl VerifiedRunContext {
    /// Run id covered by this verified context.
    pub fn run_id(&self) -> &RunId {
        &self.run_id
    }

    /// Certified spec hash covered by this verified context.
    pub fn spec_hash(&self) -> &SpecHash {
        &self.spec_hash
    }

    /// Next store-owned sequence for the committed stream.
    pub fn head_seq(&self) -> store::StreamSeq {
        self.committed.next_seq()
    }

    /// Bound runner/capability/framework authority for this scheduler step.
    pub fn bound_context(&self) -> &BoundRuntimeContext {
        &self.bound_context
    }

    /// Store-owned committed stream authority for this scheduler step.
    pub fn committed_stream(&self) -> &store::CommittedRunStream {
        &self.committed
    }

    pub(crate) fn view(&self) -> &RuntimeRunView {
        &self.view
    }
}

/// Loader for scheduler-owned verified run context.
#[derive(Clone)]
pub struct VerifiedRunContextLoader {
    runtime_contexts: BoundRuntimeContextLoader,
}

impl VerifiedRunContextLoader {
    /// Creates a verified run-context loader over bound runtime context authority.
    pub fn new(runtime_contexts: BoundRuntimeContextLoader) -> Self {
        Self { runtime_contexts }
    }

    /// Loads bound runtime context for admission-time authority.
    pub fn load_bound_context(
        &self,
        runtime_spec: &CertifiedRuntimeSpec,
    ) -> Result<BoundRuntimeContext> {
        self.runtime_contexts.load(runtime_spec)
    }

    /// Loads and verifies scheduler context from an async typed store.
    pub async fn load_async<S>(
        &self,
        runtime_spec: &CertifiedRuntimeSpec,
        run_id: &RunId,
        store: &S,
    ) -> Result<VerifiedRunContext>
    where
        S: store::RunEventStore + ?Sized,
    {
        let committed = store
            .load_committed_run_stream(run_id)
            .await
            .map_err(async_store_error)?;
        self.load_committed_stream(runtime_spec, committed)
    }

    fn load_committed_stream(
        &self,
        runtime_spec: &CertifiedRuntimeSpec,
        committed: store::CommittedRunStream,
    ) -> Result<VerifiedRunContext> {
        let bound_context = self.runtime_contexts.load(runtime_spec)?;
        let view = RuntimeRunView::from_committed_stream(runtime_spec, &committed)?;
        bound_context.validate_run_admitted_binding(&view.run_admitted)?;
        Ok(VerifiedRunContext {
            run_id: committed.run_id().clone(),
            spec_hash: runtime_spec.spec_hash().clone(),
            bound_context,
            committed,
            view,
        })
    }
}

impl VerifiedRunHistoryView {
    /// Validates a committed stream against certified runtime authority and verified retained
    /// artifact evidence.
    pub fn from_committed_stream(
        runtime_spec: &CertifiedRuntimeSpec,
        committed: store::CommittedRunStream,
        artifacts: store::VerifiedRunArtifactStore,
    ) -> Result<Self> {
        let runtime = RuntimeRunView::from_committed_stream(runtime_spec, &committed)?;
        artifacts.validate_committed_stream(&committed)?;
        Ok(Self {
            run_id: committed.run_id().clone(),
            spec_hash: runtime_spec.spec_hash().clone(),
            committed,
            artifacts,
            runtime,
        })
    }

    /// Run id covered by this verified view.
    pub fn run_id(&self) -> &RunId {
        &self.run_id
    }

    /// Certified spec hash covered by this verified view.
    pub fn spec_hash(&self) -> &SpecHash {
        &self.spec_hash
    }

    /// Authoritative committed event envelopes covered by this verified view.
    pub fn events(&self) -> &[store::KernelEventEnvelope] {
        self.committed.events()
    }

    /// Store-owned committed stream authority covered by this verified view.
    pub fn committed_stream(&self) -> &store::CommittedRunStream {
        &self.committed
    }

    /// Verified retained artifacts required by this run history.
    pub fn artifact_store(&self) -> &store::VerifiedRunArtifactStore {
        &self.artifacts
    }

    /// Projection rebuilt from the verified committed stream.
    pub fn projection_snapshot(&self) -> &store::ProjectionSnapshot {
        self.committed.projection()
    }

    /// Run admission evidence certified for this run.
    pub fn run_admitted(&self) -> &events::RunAdmitted {
        &self.runtime.run_admitted
    }

    /// Next store-owned stream sequence after this committed stream.
    pub fn head_seq(&self) -> store::StreamSeq {
        self.committed.next_seq()
    }
}

impl RuntimeRunView {
    #[cfg(test)]
    pub(crate) fn from_stream(
        runtime_spec: &CertifiedRuntimeSpec,
        run_id: &RunId,
        stream: &[store::KernelEventEnvelope],
    ) -> Result<Self> {
        if raw_stream_requires_artifact_byte_authority(stream) {
            return Err(RuntimeError::InvalidRunStream(
                "raw run stream contains fact descriptor or fact record evidence; use committed stream artifact authority"
                    .to_owned(),
            ));
        }
        let committed = store::CommittedRunStream::from_events(run_id.clone(), stream.to_vec())?;
        Self::from_committed_stream(runtime_spec, &committed)
    }

    pub(crate) fn from_committed_stream(
        runtime_spec: &CertifiedRuntimeSpec,
        committed: &store::CommittedRunStream,
    ) -> Result<Self> {
        let stream = committed.events();
        let history = RuntimeCommittedHistory::from_committed_stream(committed);
        let projections = committed.projection().clone();
        let mut run_admitted = None;
        for event in stream {
            match event.payload() {
                events::KernelEventPayload::RunAdmitted(payload) => {
                    validate_run_identity_material(runtime_spec, committed.run_id(), payload)?;
                    if run_admitted.replace(payload.clone()).is_some() {
                        return Err(RuntimeError::InvalidRunStream(
                            "run stream contains multiple RunAdmitted events".to_owned(),
                        ));
                    }
                }
                payload if payload_spec_hash(payload) != *runtime_spec.spec_hash() => {
                    return Err(RuntimeError::InvalidRunStream(format!(
                        "event payload spec hash {} does not match certified {}",
                        payload_spec_hash(payload),
                        runtime_spec.spec_hash()
                    )));
                }
                _ => {}
            }
        }
        let Some(run_admitted) = run_admitted else {
            return Err(RuntimeError::InvalidRunStream(
                "run has not started with certified RunAdmitted evidence".to_owned(),
            ));
        };
        validate_historical_run_stream(
            runtime_spec,
            committed.run_id(),
            stream,
            &projections,
            &history,
            committed.artifact_byte_authority(),
        )?;
        let seed_cells = validate_seed_cells(runtime_spec, &run_admitted.seed_cells)?;
        let config_artifacts =
            config_artifacts_from_run_admitted(runtime_spec, &run_admitted.config_artifacts)?;
        let artifact_refs = artifact_refs_from_stream(stream, &history)?;
        Ok(Self {
            stream: stream.to_vec(),
            projections,
            run_admitted: (*run_admitted).clone(),
            seed_cells,
            config_artifacts,
            artifact_refs,
            artifact_byte_authority: committed.artifact_byte_authority().clone(),
            next_seq: committed.next_seq(),
        })
    }

    pub(crate) fn with_status_authority(
        mut self,
        authority: &store::ProjectionSnapshot,
    ) -> Result<Self> {
        self.projections = self
            .projections
            .with_store_authority_from(authority)
            .map_err(RuntimeError::from)?;
        Ok(self)
    }
}
