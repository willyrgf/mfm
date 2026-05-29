#![warn(missing_docs)]
//! Serial typed scheduler for certified MFM execution specs.
//!
//! This crate owns the first certified runtime boundary. It derives runnable nodes, materialized
//! input evidence, runner bindings, and runtime capabilities only from a verified
//! [`mfm_certify::CertifiedTypedSpec`] plus store-owned typed projections.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use mfm_canonical::{sha256_digest_bytes, PlainCanonicalJsonBytes};
use mfm_capabilities::{
    CapabilityDescriptor, CapabilitySetDescriptor, EffectSpec, ManagedPlatformWrite,
};
use mfm_certify::{CertifiedSpecCertificate, CertifiedTypedSpec};
use mfm_events::v1 as events;
use mfm_ids::{
    AdapterKind, AdapterVersion, ArtifactId, AttemptId, CapabilityKind, CapabilityVersion, CellId,
    ContentDigest, DescriptorId, DigestAlgorithm, NodeId, RunId, SchemaId, SemanticTypeId,
    SpecHash,
};
use mfm_spec::v1 as spec;
use mfm_store::v1 as store;

/// Result type for typed runtime operations.
pub type Result<T> = std::result::Result<T, RuntimeError>;

/// Boxed future returned by an erased typed runner.
pub type ErasedRunnerFuture<'a> =
    Pin<Box<dyn Future<Output = Result<ErasedRunnerOutput>> + Send + 'a>>;

/// Boxed future returned by runtime-owned artifact staging.
pub type RuntimeArtifactStageFuture<'a> = Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>>;

/// Runtime-owned artifact staging capability used before admitting run authority.
///
/// This capability is held by the scheduler/middleware boundary, not by domain runners. Failed
/// store commits may leave bytes staged here, but run-store artifact evidence is admitted only by
/// the prepared typed commit that references the artifact.
pub trait RuntimeArtifactStager: Send + Sync {
    /// Stages verified artifact bytes for middleware-owned promotion before the run commit.
    fn stage_verified_artifact<'a>(
        &'a self,
        bytes: Vec<u8>,
        evidence: store::ArtifactEvidenceRef,
    ) -> RuntimeArtifactStageFuture<'a>;
}

/// Object-safe erased runner boundary used after typed spec certification.
///
/// Runner selection is keyed by the certified node descriptor id. The runner receives only
/// store-verified input cell evidence and certified capability descriptors.
pub trait ErasedNodeRunner: Send + Sync {
    /// Executes one certified node attempt.
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a>;
}

/// Prepared, store-verified invocation supplied to an erased node runner.
///
/// The runtime constructs this value only after revalidating the latest run stream against the
/// certified spec. It carries committed config evidence, materialized input evidence, certified
/// capability descriptors, and recovery facts into the runner without exposing a public
/// constructor.
pub struct PreparedRunnerInvocation<'a> {
    runtime_spec: &'a CertifiedRuntimeSpec,
    run_id: &'a RunId,
    spec_hash: &'a SpecHash,
    node: &'a spec::NodeSpec,
    descriptor: &'a spec::StateDescriptorIdentity,
    output_cell: &'a spec::CellSpec,
    attempt_id: &'a AttemptId,
    attempt_no: u32,
    config_artifact: store::ArtifactEvidenceRef,
    inputs: MaterializedInputs,
    caps: CertifiedRuntimeCapabilities,
    recorded_facts: RecordedFacts,
    projections: &'a store::ProjectionSnapshot,
    run_stream: &'a [store::KernelEventEnvelope],
}

impl<'a> PreparedRunnerInvocation<'a> {
    /// Run id being executed.
    pub fn run_id(&self) -> &'a RunId {
        self.run_id
    }

    /// Certified typed spec hash.
    pub fn spec_hash(&self) -> &'a SpecHash {
        self.spec_hash
    }

    /// Certified node spec.
    pub fn node(&self) -> &'a spec::NodeSpec {
        self.node
    }

    /// Certified state descriptor identity for the node.
    pub fn descriptor(&self) -> &'a spec::StateDescriptorIdentity {
        self.descriptor
    }

    /// Certified output cell spec for the node.
    pub fn output_cell(&self) -> &'a spec::CellSpec {
        self.output_cell
    }

    /// Store-owned attempt id minted by the scheduler.
    pub fn attempt_id(&self) -> &'a AttemptId {
        self.attempt_id
    }

    /// Attempt number for this node.
    pub const fn attempt_no(&self) -> u32 {
        self.attempt_no
    }

    /// Store-committed typed config artifact evidence matching the certified config ref.
    pub fn config_artifact(&self) -> &store::ArtifactEvidenceRef {
        &self.config_artifact
    }

    /// Materialized input evidence derived only from certified cells and validated stream state.
    pub fn inputs(&self) -> &MaterializedInputs {
        &self.inputs
    }

    /// Runtime capabilities minted only from the certified node capability set.
    pub fn caps(&self) -> &CertifiedRuntimeCapabilities {
        &self.caps
    }

    /// Facts already committed for this attempt and therefore reusable after recovery.
    pub fn recorded_facts(&self) -> &RecordedFacts {
        &self.recorded_facts
    }

    /// Store-owned projection snapshot observed before the runner invocation.
    pub fn projections(&self) -> &store::ProjectionSnapshot {
        self.projections
    }

    fn runtime_spec(&self) -> &'a CertifiedRuntimeSpec {
        self.runtime_spec
    }

    fn run_stream(&self) -> &'a [store::KernelEventEnvelope] {
        self.run_stream
    }
}

/// Context supplied to an erased node runner.
pub struct ErasedRunCtx<'a> {
    invocation: &'a PreparedRunnerInvocation<'a>,
}

impl<'a> ErasedRunCtx<'a> {
    fn from_prepared(invocation: &'a PreparedRunnerInvocation<'a>) -> Self {
        Self { invocation }
    }

    /// Prepared runner invocation backing this context.
    pub fn invocation(&self) -> &'a PreparedRunnerInvocation<'a> {
        self.invocation
    }

    /// Run id being executed.
    pub fn run_id(&self) -> &'a RunId {
        self.invocation.run_id()
    }

    /// Certified typed spec hash.
    pub fn spec_hash(&self) -> &'a SpecHash {
        self.invocation.spec_hash()
    }

    /// Certified node spec.
    pub fn node(&self) -> &'a spec::NodeSpec {
        self.invocation.node()
    }

    /// Certified state descriptor identity for the node.
    pub fn descriptor(&self) -> &'a spec::StateDescriptorIdentity {
        self.invocation.descriptor()
    }

    /// Certified output cell spec for the node.
    pub fn output_cell(&self) -> &'a spec::CellSpec {
        self.invocation.output_cell()
    }

    /// Store-owned attempt id minted by the scheduler.
    pub fn attempt_id(&self) -> &'a AttemptId {
        self.invocation.attempt_id()
    }

    /// Attempt number for this node.
    pub const fn attempt_no(&self) -> u32 {
        self.invocation.attempt_no()
    }

    /// Store-committed typed config artifact evidence matching the certified config ref.
    pub fn config_artifact(&self) -> &store::ArtifactEvidenceRef {
        self.invocation.config_artifact()
    }

    /// Materialized input evidence derived only from certified cells and validated stream state.
    pub fn inputs(&self) -> &MaterializedInputs {
        self.invocation.inputs()
    }

    /// Runtime capabilities minted only from the certified node capability set.
    pub fn caps(&self) -> &CertifiedRuntimeCapabilities {
        self.invocation.caps()
    }

    /// Facts already committed for this attempt and therefore reusable after recovery.
    pub fn recorded_facts(&self) -> &RecordedFacts {
        self.invocation.recorded_facts()
    }

    /// Store-owned projection snapshot observed before the runner invocation.
    pub fn projections(&self) -> &store::ProjectionSnapshot {
        self.invocation.projections()
    }

    fn runtime_spec(&self) -> &'a CertifiedRuntimeSpec {
        self.invocation.runtime_spec()
    }

    fn run_stream(&self) -> &'a [store::KernelEventEnvelope] {
        self.invocation.run_stream()
    }
}

/// Facts committed for one node attempt before recovery resumed execution.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RecordedFacts {
    facts: BTreeMap<events::FactKey, RecordedFact>,
}

impl RecordedFacts {
    /// Returns true when no facts have been recorded for the attempt.
    pub fn is_empty(&self) -> bool {
        self.facts.is_empty()
    }

    /// Returns a recorded fact by stable fact key.
    pub fn get(&self, fact_key: &events::FactKey) -> Option<&RecordedFact> {
        self.facts.get(fact_key)
    }

    /// Iterates recorded facts in deterministic fact-key order.
    pub fn iter(&self) -> impl Iterator<Item = (&events::FactKey, &RecordedFact)> {
        self.facts.iter()
    }
}

/// Store-projected read fact available for same-attempt recovery.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedFact {
    /// Stable fact key.
    pub fact_key: events::FactKey,
    /// Request schema id.
    pub request_schema_id: SchemaId,
    /// Canonical request hash.
    pub request_hash: ContentDigest,
    /// Response schema id.
    pub response_schema_id: SchemaId,
    /// Canonical response hash.
    pub response_hash: ContentDigest,
    /// Response artifact id.
    pub artifact_id: ArtifactId,
    /// Capability kind used for the original read.
    pub capability_kind: CapabilityKind,
    /// Capability version used for the original read.
    pub capability_version: CapabilityVersion,
    /// Adapter kind used for the original read.
    pub adapter_kind: AdapterKind,
    /// Adapter version used for the original read.
    pub adapter_version: AdapterVersion,
}

/// Typed payload batch returned by an erased runner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErasedRunnerOutput {
    /// Staged artifacts or sealed finalized handles referenced by payloads.
    pub staged_artifacts: Vec<StagedArtifact>,
    /// Retention refs staged by the runner for scheduler-owned event binding.
    pub staged_retention_refs: Vec<StagedRetentionRefs>,
    /// Typed event payloads to commit atomically for this attempt.
    pub payloads: Vec<events::KernelEventPayload>,
}

impl ErasedRunnerOutput {
    /// Creates an output batch from payloads with no additional artifact evidence.
    pub fn new(payloads: Vec<events::KernelEventPayload>) -> Self {
        Self {
            staged_artifacts: Vec::new(),
            staged_retention_refs: Vec::new(),
            payloads,
        }
    }
}

/// Runtime-owned artifact binding kind for one staged attempt artifact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StagedArtifactBindingKind {
    /// Artifact is the terminal output for a state cell.
    StateOutput,
    /// Artifact is an external read fact response.
    FactResponse,
    /// Artifact is side-effect evidence bound to one ledger phase.
    SideEffectEvidence {
        /// Side-effect ledger the evidence belongs to.
        ledger_key: events::SideEffectLedgerKey,
        /// Invocation epoch the evidence belongs to.
        invocation_epoch: u32,
        /// Side-effect phase the evidence proves.
        phase: StagedSideEffectArtifactPhase,
    },
    /// Artifact is a public-output payload.
    PublicOutput,
    /// Artifact is a redacted diagnostic.
    RedactedDiagnostic,
    /// Artifact is a framework retention manifest projected by runtime middleware.
    RetentionManifest,
}

/// Runtime-owned side-effect phase for one staged side-effect artifact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum StagedSideEffectArtifactPhase {
    /// Initial side-effect intent persisted before claims and invocation.
    Intent,
    /// Prepared invocation payload.
    PreparedInvocation,
    /// Proof that the invocation was not submitted.
    NotSubmittedProof,
    /// Submission evidence.
    Submission,
    /// Evidence that submission status is unknown.
    SubmissionUnknownEvidence,
    /// Receipt evidence.
    Receipt,
    /// Confirmation evidence.
    Confirmation,
    /// Ambiguity evidence.
    AmbiguityEvidence,
}

/// Sealed finalized artifact handle bound to one run, node, attempt, and evidence role.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StagedArtifactHandle {
    run_id: RunId,
    node_id: NodeId,
    attempt_id: AttemptId,
    binding: StagedArtifactBindingKind,
    evidence: store::ArtifactEvidenceRef,
}

impl StagedArtifactHandle {
    fn for_attempt(
        ctx: &ErasedRunCtx<'_>,
        evidence: store::ArtifactEvidenceRef,
        binding: StagedArtifactBindingKind,
    ) -> Result<Self> {
        if staged_artifact_binding_role(&binding) != evidence.artifact_role {
            return Err(RuntimeError::InvalidRunnerOutput(format!(
                "node {} staged artifact role {} with mismatched binding",
                ctx.node().node_id,
                artifact_role_name(evidence.artifact_role)
            )));
        }
        if evidence.producer_node_id.as_ref() != Some(&ctx.node().node_id)
            || evidence.producer_seed_id.is_some()
        {
            return Err(RuntimeError::InvalidRunnerOutput(format!(
                "node {} staged artifact {} with producer evidence outside its attempt",
                ctx.node().node_id,
                evidence.artifact_id
            )));
        }
        Ok(Self {
            run_id: ctx.run_id().clone(),
            node_id: ctx.node().node_id.clone(),
            attempt_id: ctx.attempt_id().clone(),
            binding,
            evidence,
        })
    }

    /// Returns the run id this handle is sealed to.
    pub fn run_id(&self) -> &RunId {
        &self.run_id
    }

    /// Returns the node id this handle is sealed to.
    pub fn node_id(&self) -> &NodeId {
        &self.node_id
    }

    /// Returns the attempt id this handle is sealed to.
    pub fn attempt_id(&self) -> &AttemptId {
        &self.attempt_id
    }

    /// Returns the artifact binding metadata.
    pub fn binding(&self) -> &StagedArtifactBindingKind {
        &self.binding
    }

    /// Returns the finalized artifact evidence.
    pub fn evidence(&self) -> &store::ArtifactEvidenceRef {
        &self.evidence
    }
}

/// Attempt artifact staged for runtime middleware admission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StagedArtifact {
    handle: StagedArtifactHandle,
    bytes: Option<Vec<u8>>,
}

impl StagedArtifact {
    /// Creates an inline staged artifact bound to the current attempt.
    pub fn inline_attempt_artifact(
        ctx: &ErasedRunCtx<'_>,
        bytes: Vec<u8>,
        evidence: store::ArtifactEvidenceRef,
    ) -> Result<Self> {
        let binding = staged_artifact_binding_kind(evidence.artifact_role).ok_or_else(|| {
            RuntimeError::InvalidRunnerOutput(format!(
                "node {} staged artifact role {} outside non-side-effect attempt artifact authority",
                ctx.node().node_id,
                artifact_role_name(evidence.artifact_role)
            ))
        })?;
        Self::inline_attempt_artifact_with_binding(ctx, bytes, evidence, binding)
    }

    /// Creates an inline staged side-effect artifact bound to one ledger phase.
    pub fn inline_side_effect_artifact(
        ctx: &ErasedRunCtx<'_>,
        bytes: Vec<u8>,
        evidence: store::ArtifactEvidenceRef,
        ledger_key: events::SideEffectLedgerKey,
        invocation_epoch: u32,
    ) -> Result<Self> {
        let phase = staged_side_effect_artifact_phase(evidence.artifact_role).ok_or_else(|| {
            RuntimeError::InvalidRunnerOutput(format!(
                "node {} staged artifact role {} outside side-effect artifact authority",
                ctx.node().node_id,
                artifact_role_name(evidence.artifact_role)
            ))
        })?;
        Self::inline_attempt_artifact_with_binding(
            ctx,
            bytes,
            evidence,
            StagedArtifactBindingKind::SideEffectEvidence {
                ledger_key,
                invocation_epoch,
                phase,
            },
        )
    }

    fn inline_attempt_artifact_with_binding(
        ctx: &ErasedRunCtx<'_>,
        bytes: Vec<u8>,
        evidence: store::ArtifactEvidenceRef,
        binding: StagedArtifactBindingKind,
    ) -> Result<Self> {
        verify_artifact_bytes(&bytes, &evidence)?;
        Ok(Self {
            handle: StagedArtifactHandle::for_attempt(ctx, evidence, binding)?,
            bytes: Some(bytes),
        })
    }

    fn inline_retention_manifest_artifact(
        ctx: &ErasedRunCtx<'_>,
        bytes: Vec<u8>,
        evidence: store::ArtifactEvidenceRef,
    ) -> Result<Self> {
        Self::inline_attempt_artifact_with_binding(
            ctx,
            bytes,
            evidence,
            StagedArtifactBindingKind::RetentionManifest,
        )
    }

    #[cfg(test)]
    fn finalized_attempt_artifact_for_tests(
        ctx: &ErasedRunCtx<'_>,
        evidence: store::ArtifactEvidenceRef,
        binding: StagedArtifactBindingKind,
    ) -> Result<Self> {
        Ok(Self {
            handle: StagedArtifactHandle::for_attempt(ctx, evidence, binding)?,
            bytes: None,
        })
    }

    /// Returns the sealed handle.
    pub fn handle(&self) -> &StagedArtifactHandle {
        &self.handle
    }

    /// Returns the artifact evidence bound to this staged artifact.
    pub fn evidence(&self) -> &store::ArtifactEvidenceRef {
        self.handle().evidence()
    }

    /// Returns inline bytes when carried by the output.
    pub fn bytes(&self) -> Option<&[u8]> {
        self.bytes.as_deref()
    }
}

fn staged_artifact_binding_kind(role: events::ArtifactRole) -> Option<StagedArtifactBindingKind> {
    match role {
        events::ArtifactRole::StateOutput => Some(StagedArtifactBindingKind::StateOutput),
        events::ArtifactRole::FactResponse => Some(StagedArtifactBindingKind::FactResponse),
        events::ArtifactRole::PublicOutput => Some(StagedArtifactBindingKind::PublicOutput),
        events::ArtifactRole::RedactedDiagnostic => {
            Some(StagedArtifactBindingKind::RedactedDiagnostic)
        }
        events::ArtifactRole::TypedExecutionSpec
        | events::ArtifactRole::TypedSpecCertificate
        | events::ArtifactRole::TypedConfig
        | events::ArtifactRole::SeedInput
        | events::ArtifactRole::SideEffectIntent
        | events::ArtifactRole::PreparedInvocation
        | events::ArtifactRole::NotSubmittedProof
        | events::ArtifactRole::Submission
        | events::ArtifactRole::SubmissionUnknownEvidence
        | events::ArtifactRole::Receipt
        | events::ArtifactRole::Confirmation
        | events::ArtifactRole::AmbiguityEvidence
        | events::ArtifactRole::RetentionManifest => None,
    }
}

fn staged_side_effect_artifact_phase(
    role: events::ArtifactRole,
) -> Option<StagedSideEffectArtifactPhase> {
    match role {
        events::ArtifactRole::SideEffectIntent => Some(StagedSideEffectArtifactPhase::Intent),
        events::ArtifactRole::PreparedInvocation => {
            Some(StagedSideEffectArtifactPhase::PreparedInvocation)
        }
        events::ArtifactRole::NotSubmittedProof => {
            Some(StagedSideEffectArtifactPhase::NotSubmittedProof)
        }
        events::ArtifactRole::Submission => Some(StagedSideEffectArtifactPhase::Submission),
        events::ArtifactRole::SubmissionUnknownEvidence => {
            Some(StagedSideEffectArtifactPhase::SubmissionUnknownEvidence)
        }
        events::ArtifactRole::Receipt => Some(StagedSideEffectArtifactPhase::Receipt),
        events::ArtifactRole::Confirmation => Some(StagedSideEffectArtifactPhase::Confirmation),
        events::ArtifactRole::AmbiguityEvidence => {
            Some(StagedSideEffectArtifactPhase::AmbiguityEvidence)
        }
        events::ArtifactRole::TypedExecutionSpec
        | events::ArtifactRole::TypedSpecCertificate
        | events::ArtifactRole::TypedConfig
        | events::ArtifactRole::SeedInput
        | events::ArtifactRole::StateOutput
        | events::ArtifactRole::FactResponse
        | events::ArtifactRole::PublicOutput
        | events::ArtifactRole::RedactedDiagnostic
        | events::ArtifactRole::RetentionManifest => None,
    }
}

fn staged_artifact_binding_role(binding: &StagedArtifactBindingKind) -> events::ArtifactRole {
    match binding {
        StagedArtifactBindingKind::StateOutput => events::ArtifactRole::StateOutput,
        StagedArtifactBindingKind::FactResponse => events::ArtifactRole::FactResponse,
        StagedArtifactBindingKind::SideEffectEvidence { phase, .. } => match phase {
            StagedSideEffectArtifactPhase::Intent => events::ArtifactRole::SideEffectIntent,
            StagedSideEffectArtifactPhase::PreparedInvocation => {
                events::ArtifactRole::PreparedInvocation
            }
            StagedSideEffectArtifactPhase::NotSubmittedProof => {
                events::ArtifactRole::NotSubmittedProof
            }
            StagedSideEffectArtifactPhase::Submission => events::ArtifactRole::Submission,
            StagedSideEffectArtifactPhase::SubmissionUnknownEvidence => {
                events::ArtifactRole::SubmissionUnknownEvidence
            }
            StagedSideEffectArtifactPhase::Receipt => events::ArtifactRole::Receipt,
            StagedSideEffectArtifactPhase::Confirmation => events::ArtifactRole::Confirmation,
            StagedSideEffectArtifactPhase::AmbiguityEvidence => {
                events::ArtifactRole::AmbiguityEvidence
            }
        },
        StagedArtifactBindingKind::PublicOutput => events::ArtifactRole::PublicOutput,
        StagedArtifactBindingKind::RedactedDiagnostic => events::ArtifactRole::RedactedDiagnostic,
        StagedArtifactBindingKind::RetentionManifest => events::ArtifactRole::RetentionManifest,
    }
}

fn verify_artifact_bytes(bytes: &[u8], evidence: &store::ArtifactEvidenceRef) -> Result<()> {
    let digest =
        ContentDigest::from_digest(DigestAlgorithm::Sha256JcsV1, sha256_digest_bytes(bytes));
    let artifact_id = ArtifactId::from_digest(digest.algorithm(), *digest.digest());
    if evidence.digest != digest
        || evidence.artifact_id != artifact_id
        || evidence.byte_len != bytes.len() as u64
    {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "staged artifact {} bytes do not match typed evidence",
            evidence.artifact_id
        )));
    }
    Ok(())
}

fn artifact_role_name(role: events::ArtifactRole) -> &'static str {
    match role {
        events::ArtifactRole::TypedExecutionSpec => "typed_execution_spec",
        events::ArtifactRole::TypedSpecCertificate => "typed_spec_certificate",
        events::ArtifactRole::TypedConfig => "typed_config",
        events::ArtifactRole::SeedInput => "seed_input",
        events::ArtifactRole::StateOutput => "state_output",
        events::ArtifactRole::FactResponse => "fact_response",
        events::ArtifactRole::SideEffectIntent => "side_effect_intent",
        events::ArtifactRole::PreparedInvocation => "prepared_invocation",
        events::ArtifactRole::NotSubmittedProof => "not_submitted_proof",
        events::ArtifactRole::Submission => "submission",
        events::ArtifactRole::SubmissionUnknownEvidence => "submission_unknown_evidence",
        events::ArtifactRole::Receipt => "receipt",
        events::ArtifactRole::Confirmation => "confirmation",
        events::ArtifactRole::AmbiguityEvidence => "ambiguity_evidence",
        events::ArtifactRole::PublicOutput => "public_output",
        events::ArtifactRole::RedactedDiagnostic => "redacted_diagnostic",
        events::ArtifactRole::RetentionManifest => "retention_manifest",
    }
}

/// Retention refs staged by a runner before the scheduler binds them to typed events.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StagedRetentionRefs {
    refs: Vec<events::RetentionRef>,
    reason: events::RetentionReason,
}

impl StagedRetentionRefs {
    /// Stages runtime-evidence retention refs for the current runner attempt.
    pub fn runtime_evidence(refs: Vec<events::RetentionRef>) -> Self {
        Self {
            refs,
            reason: events::RetentionReason::RuntimeEvidence,
        }
    }

    /// Returns the staged retention refs.
    pub fn refs(&self) -> &[events::RetentionRef] {
        &self.refs
    }

    /// Returns the middleware retention reason that will be bound to the commit.
    pub fn reason(&self) -> events::RetentionReason {
        self.reason
    }

    fn framework_public_output(refs: Vec<events::RetentionRef>) -> Self {
        Self {
            refs,
            reason: events::RetentionReason::PublicOutput,
        }
    }
}

/// Runtime failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeError {
    /// Certified spec hash verification failed.
    SpecHash(String),
    /// Certified spec structure is not executable by the runtime.
    InvalidSpec(String),
    /// The run stream does not match the certified spec or requested run id.
    InvalidRunStream(String),
    /// A runner binding is missing or inconsistent with certified descriptor evidence.
    RunnerBinding(String),
    /// No node is runnable and the run is not complete.
    Blocked(String),
    /// Input materialization failed.
    InputMaterialization(String),
    /// Runner output violated certified node or capability authority.
    InvalidRunnerOutput(String),
    /// Store contract rejected a typed commit.
    Store(String),
    /// Identity construction failed.
    Identity(String),
    /// Canonical JSON construction failed.
    Canonical(String),
}

impl fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SpecHash(message) => write!(f, "certified spec hash error: {message}"),
            Self::InvalidSpec(message) => write!(f, "invalid typed runtime spec: {message}"),
            Self::InvalidRunStream(message) => write!(f, "invalid typed run stream: {message}"),
            Self::RunnerBinding(message) => write!(f, "typed runner binding error: {message}"),
            Self::Blocked(message) => write!(f, "typed scheduler blocked: {message}"),
            Self::InputMaterialization(message) => {
                write!(f, "typed input materialization failed: {message}")
            }
            Self::InvalidRunnerOutput(message) => write!(f, "invalid runner output: {message}"),
            Self::Store(message) => write!(f, "typed store error: {message}"),
            Self::Identity(message) => write!(f, "identity error: {message}"),
            Self::Canonical(message) => write!(f, "canonical JSON error: {message}"),
        }
    }
}

impl std::error::Error for RuntimeError {}

impl From<store::StoreError> for RuntimeError {
    fn from(error: store::StoreError) -> Self {
        Self::Store(error.to_string())
    }
}

impl From<mfm_ids::IdentityError> for RuntimeError {
    fn from(error: mfm_ids::IdentityError) -> Self {
        Self::Identity(error.to_string())
    }
}

impl From<mfm_spec::SpecError> for RuntimeError {
    fn from(error: mfm_spec::SpecError) -> Self {
        Self::SpecHash(error.to_string())
    }
}

impl From<mfm_events::EventError> for RuntimeError {
    fn from(error: mfm_events::EventError) -> Self {
        Self::Identity(error.to_string())
    }
}

fn async_store_error(error: impl fmt::Display) -> RuntimeError {
    RuntimeError::Store(error.to_string())
}

/// Certified executable runtime spec with indexes used by the serial scheduler.
#[derive(Debug, Clone)]
pub struct CertifiedRuntimeSpec {
    envelope: spec::HashedSpecEnvelope,
    certificate: CertifiedSpecCertificate,
    state_descriptors: BTreeMap<DescriptorId, spec::StateDescriptorIdentity>,
    nodes: BTreeMap<NodeId, spec::NodeSpec>,
    cells: BTreeMap<CellId, spec::CellSpec>,
    topological_order: Vec<NodeId>,
}

impl CertifiedRuntimeSpec {
    /// Builds deterministic runtime indexes from certifier-backed typed-spec authority.
    pub fn new(certified: CertifiedTypedSpec) -> Result<Self> {
        let (envelope, certificate) = certified.into_parts();
        Self::from_verified_parts(envelope, certificate)
    }

    #[cfg(test)]
    fn from_verified_envelope(envelope: spec::HashedSpecEnvelope) -> Result<Self> {
        let certificate = mfm_certify::CertifiedSpecCertificate::from_evidence(
            mfm_certify::CertifiedSpecCertificateEvidence {
                certificate_version: mfm_certify::CERTIFICATE_VERSION.to_owned(),
                media_type: mfm_certify::CERTIFICATE_MEDIA_TYPE.to_owned(),
                certifier_version: "mfm-runtime-test-placeholder".to_owned(),
                certifier_algorithm: "mfm-runtime-test-placeholder".to_owned(),
                certificate_canonicalization: DigestAlgorithm::Sha256JcsV1,
                spec_hash: envelope.spec_hash.clone(),
                spec_canonicalization: envelope.spec.canonicalization,
                registry_digest: ContentDigest::from_digest(
                    DigestAlgorithm::Sha256JcsV1,
                    mfm_ids::DigestBytes::from_array([0; 32]),
                ),
                descriptor_identities: Vec::new(),
                lowering_version: envelope.spec.lowering_version.clone(),
                public_output_schema_id: envelope.spec.public_outputs.public_schema_id.clone(),
                public_output_canonicalizer_identity: envelope
                    .spec
                    .public_outputs
                    .renderer_descriptor
                    .canonicalizer_identity
                    .clone(),
                audit: mfm_certify::CertifiedSpecAuditMetadata {
                    problem_classes_covered: Vec::new(),
                    scope_count: envelope.spec.scopes.len() as u64,
                    operation_lineage_count: envelope.spec.planning_lineage.len() as u64,
                    node_count: envelope.spec.nodes.len() as u64,
                    cell_count: envelope.spec.cells.len() as u64,
                    seed_count: envelope.spec.seeds.len() as u64,
                    descriptor_count: envelope.spec.descriptor_identities.len() as u64,
                },
            },
        )
        .map_err(|error| RuntimeError::InvalidSpec(error.to_string()))?;
        Self::from_verified_parts(envelope, certificate)
    }

    fn from_verified_parts(
        envelope: spec::HashedSpecEnvelope,
        certificate: CertifiedSpecCertificate,
    ) -> Result<Self> {
        envelope.verify_hash()?;
        let mut state_descriptors = BTreeMap::new();
        for descriptor in &envelope.spec.descriptor_identities {
            if let spec::DescriptorIdentity::State(identity) = descriptor {
                let previous = state_descriptors
                    .insert(identity.descriptor_id.clone(), identity.as_ref().clone());
                if previous.is_some() {
                    return Err(RuntimeError::InvalidSpec(format!(
                        "duplicate state descriptor {}",
                        identity.descriptor_id
                    )));
                }
            }
        }

        let mut cells = BTreeMap::new();
        for cell in &envelope.spec.cells {
            if cells.insert(cell.cell_id.clone(), cell.clone()).is_some() {
                return Err(RuntimeError::InvalidSpec(format!(
                    "duplicate cell {}",
                    cell.cell_id
                )));
            }
        }

        let mut nodes = BTreeMap::new();
        for node in &envelope.spec.nodes {
            if nodes.insert(node.node_id.clone(), node.clone()).is_some() {
                return Err(RuntimeError::InvalidSpec(format!(
                    "duplicate node {}",
                    node.node_id
                )));
            }
        }

        let runtime = Self {
            envelope,
            certificate,
            state_descriptors,
            nodes,
            cells,
            topological_order: Vec::new(),
        };
        runtime.validate_runtime_contract()?;
        let topological_order = runtime.compute_topological_order()?;
        Ok(Self {
            topological_order,
            ..runtime
        })
    }

    /// Returns the hash-only spec envelope.
    pub fn envelope(&self) -> &spec::HashedSpecEnvelope {
        &self.envelope
    }

    /// Returns the certified spec hash.
    pub fn spec_hash(&self) -> &SpecHash {
        &self.envelope.spec_hash
    }

    /// Returns the verified certificate evidence used to mint this runtime spec.
    pub fn certificate(&self) -> &CertifiedSpecCertificate {
        &self.certificate
    }

    /// Returns the hash-defining typed execution spec.
    pub fn spec(&self) -> &spec::TypedExecutionSpec {
        &self.envelope.spec
    }

    /// Returns deterministic topological node ids.
    pub fn topological_order(&self) -> &[NodeId] {
        &self.topological_order
    }

    /// Returns a certified node by id.
    pub fn node(&self, node_id: &NodeId) -> Option<&spec::NodeSpec> {
        self.nodes.get(node_id)
    }

    /// Returns a certified cell by id.
    pub fn cell(&self, cell_id: &CellId) -> Option<&spec::CellSpec> {
        self.cells.get(cell_id)
    }

    /// Returns the certified state descriptor for a node.
    pub fn state_descriptor_for_node(
        &self,
        node: &spec::NodeSpec,
    ) -> Result<&spec::StateDescriptorIdentity> {
        self.state_descriptors
            .get(&node.descriptor_id)
            .ok_or_else(|| {
                RuntimeError::InvalidSpec(format!(
                    "node {} references missing state descriptor {}",
                    node.node_id, node.descriptor_id
                ))
            })
    }

    fn validate_runtime_contract(&self) -> Result<()> {
        let config_refs = self
            .spec()
            .config_refs
            .iter()
            .map(config_ref_key)
            .collect::<BTreeSet<_>>();

        for seed in &self.spec().seeds {
            let cell = self.cells.get(&seed.cell_id).ok_or_else(|| {
                RuntimeError::InvalidSpec(format!(
                    "seed {} references missing cell {}",
                    seed.seed_id, seed.cell_id
                ))
            })?;
            if cell.producer != spec::CellProducer::Seed(seed.seed_id.clone())
                || cell.scope_id != seed.scope_id
                || cell.schema_id != seed.schema_id
                || cell.semantic_type_id != seed.semantic_type_id
            {
                return Err(RuntimeError::InvalidSpec(format!(
                    "seed {} cell metadata does not match certified cell {}",
                    seed.seed_id, seed.cell_id
                )));
            }
        }

        for node in self.nodes.values() {
            let descriptor = self.state_descriptor_for_node(node)?;
            self.validate_framework_descriptor_variant(node, descriptor)?;
            if descriptor.state_kind != node.state_kind
                || descriptor.state_version != node.state_version
                || descriptor.config_schema_id != node.config_ref.schema_id
                || descriptor.input_schema_id != node.input_bindings.input_schema_id
                || descriptor.output_schema_id
                    != self
                        .cells
                        .get(&node.output_cell)
                        .map(|cell| cell.schema_id.clone())
                        .ok_or_else(|| {
                            RuntimeError::InvalidSpec(format!(
                                "node {} output cell {} is missing",
                                node.node_id, node.output_cell
                            ))
                        })?
                || descriptor.output_semantic_type_id
                    != self
                        .cells
                        .get(&node.output_cell)
                        .map(|cell| cell.semantic_type_id.clone())
                        .expect("checked above")
                || descriptor.effect_kind != node.effect_kind
                || descriptor.capabilities != node.capability_bindings
            {
                return Err(RuntimeError::InvalidSpec(format!(
                    "node {} does not match certified state descriptor {}",
                    node.node_id, node.descriptor_id
                )));
            }
            match (&node.side_effect, &descriptor.side_effect_contract_digest) {
                (Some(contract), Some(descriptor_digest))
                    if descriptor_digest == &contract.contract_digest => {}
                (Some(_), _) => {
                    return Err(RuntimeError::InvalidSpec(format!(
                        "side-effect node {} lacks matching descriptor side-effect contract",
                        node.node_id
                    )));
                }
                (None, Some(_)) => {
                    return Err(RuntimeError::InvalidSpec(format!(
                        "non-side-effect node {} references a descriptor with side-effect contract",
                        node.node_id
                    )));
                }
                (None, None) => {}
            }

            if !config_refs.contains(&config_ref_key(&node.config_ref)) {
                return Err(RuntimeError::InvalidSpec(format!(
                    "node {} references missing config artifact {}",
                    node.node_id, node.config_ref.artifact_id
                )));
            }

            let output = self.cells.get(&node.output_cell).expect("checked above");
            if output.producer != spec::CellProducer::Node(node.node_id.clone()) {
                return Err(RuntimeError::InvalidSpec(format!(
                    "node {} output cell {} has mismatched producer",
                    node.node_id, node.output_cell
                )));
            }

            let input_cells = self.validate_input_binding(&node.input_bindings.root)?;
            let expected_predecessors = self.predecessors_for_input_cells(&input_cells)?;
            if node.deterministic_predecessors != expected_predecessors {
                return Err(RuntimeError::InvalidSpec(format!(
                    "node {} deterministic predecessors do not match input cells",
                    node.node_id
                )));
            }
        }

        for output in &self.spec().public_outputs.outputs {
            let cell = self.cells.get(&output.cell_id).ok_or_else(|| {
                RuntimeError::InvalidSpec(format!(
                    "public output {} references missing cell {}",
                    output.public_field_path.as_str(),
                    output.cell_id
                ))
            })?;
            if cell.producer != output.producer
                || cell.scope_id != output.scope_id
                || cell.semantic_type_id != output.semantic_type_id
                || cell.schema_id != output.schema_id
                || cell.value_lineage != output.value_lineage
            {
                return Err(RuntimeError::InvalidSpec(format!(
                    "public output {} does not match certified cell {}",
                    output.public_field_path.as_str(),
                    output.cell_id
                )));
            }
        }
        self.validate_public_output_render_contract()?;
        self.validate_lifecycle_framework_contract()?;

        Ok(())
    }

    fn validate_public_output_render_contract(&self) -> Result<()> {
        let public_outputs = &self.spec().public_outputs;
        let output_spec_digest = public_outputs.digest()?;
        let render_nodes = self
            .nodes
            .values()
            .filter_map(|node| match &node.framework {
                Some(spec::FrameworkNodeSpec::PublicOutputRender(render)) => Some((node, render)),
                _ => None,
            })
            .collect::<Vec<_>>();
        if render_nodes.len() != 1 {
            return Err(RuntimeError::InvalidSpec(format!(
                "expected exactly one public-output render node, found {}",
                render_nodes.len()
            )));
        }
        let (node, render) = render_nodes[0];
        if render.public_schema_id != public_outputs.public_schema_id
            || render.output_spec_digest != output_spec_digest
            || render.renderer_descriptor != public_outputs.renderer_descriptor
            || render.required_cells != public_outputs.outputs
        {
            return Err(RuntimeError::InvalidSpec(format!(
                "public-output render node {} does not match certified public outputs",
                node.node_id
            )));
        }
        let descriptor = self.state_descriptor_for_node(node)?;
        let managed_effect = ManagedPlatformWrite::descriptor()
            .map_err(|error| RuntimeError::InvalidSpec(error.to_string()))?;
        if descriptor.name != "mfm.framework.render_public_outputs"
            || descriptor.runner != "managed_platform_write"
            || descriptor.effect_kind != managed_effect.kind
            || descriptor.effect_class != managed_effect.class.as_str()
            || descriptor.effect_name != managed_effect.name
            || !descriptor.capabilities.capabilities.is_empty()
        {
            return Err(RuntimeError::InvalidSpec(format!(
                "public-output render node {} is not bound to the framework renderer",
                node.node_id
            )));
        }
        let output_cell = self.cells.get(&node.output_cell).ok_or_else(|| {
            RuntimeError::InvalidSpec(format!(
                "public-output render node {} output cell {} is missing",
                node.node_id, node.output_cell
            ))
        })?;
        let receipt_schema_id = spec::public_output_receipt_schema_id()?;
        if output_cell.schema_id != receipt_schema_id
            || output_cell.terminal_policy != spec::CellTerminalPolicy::ProducedOnly
            || output_cell.storage_policy != spec::StoragePolicy::PublicOutputArtifact
            || output_cell.redaction_policy != spec::RedactionPolicy::Public
        {
            return Err(RuntimeError::InvalidSpec(format!(
                "public-output render node {} output cell is not a public-output receipt",
                node.node_id
            )));
        }
        let input_cells = render
            .required_cells
            .iter()
            .map(|cell| cell.cell_id.clone())
            .collect::<Vec<_>>();
        let expected_predecessors = self.predecessors_for_input_cells(&input_cells)?;
        if node.deterministic_predecessors != expected_predecessors {
            return Err(RuntimeError::InvalidSpec(format!(
                "public-output render node {} predecessors do not match public cells",
                node.node_id
            )));
        }
        Ok(())
    }

    fn validate_lifecycle_framework_contract(&self) -> Result<()> {
        let mut bootstrap_count = 0_usize;
        let mut retention_count = 0_usize;
        let mut completion_count = 0_usize;
        let mut lifecycle_outputs = BTreeMap::<CellId, &spec::NodeSpec>::new();
        let mut input_consumers = BTreeMap::<CellId, Vec<NodeId>>::new();
        for node in self.nodes.values() {
            for cell_id in self.validate_input_binding(&node.input_bindings.root)? {
                input_consumers
                    .entry(cell_id)
                    .or_default()
                    .push(node.node_id.clone());
            }
            if matches!(
                &node.framework,
                Some(
                    spec::FrameworkNodeSpec::BootstrapRun(_)
                        | spec::FrameworkNodeSpec::ProjectRetentionManifest(_)
                        | spec::FrameworkNodeSpec::CompleteRun(_)
                )
            ) {
                lifecycle_outputs.insert(node.output_cell.clone(), node);
            }
        }

        for node in self.nodes.values() {
            if let Some(framework) = &node.framework {
                self.validate_framework_permissions(node)?;
                self.validate_framework_config_ref(node, framework.config_kind())?;
            }
            match &node.framework {
                Some(spec::FrameworkNodeSpec::BootstrapRun(_)) => {
                    bootstrap_count += 1;
                    self.validate_framework_descriptor(node, "mfm.framework.bootstrap_run")?;
                    self.validate_framework_output_cell(
                        node,
                        &spec::bootstrap_run_receipt_schema_id()?,
                        &spec::bootstrap_run_receipt_semantic_type_id()?,
                        spec::StoragePolicy::ContentAddressed,
                    )?;
                    self.validate_framework_input_binding(
                        node,
                        &spec::framework_lifecycle_unit_input_binding("bootstrap_run")?,
                    )?;
                    let input_cells = self.validate_input_binding(&node.input_bindings.root)?;
                    if !input_cells.is_empty() || !node.deterministic_predecessors.is_empty() {
                        return Err(RuntimeError::InvalidSpec(format!(
                            "bootstrap lifecycle node {} must not have inputs",
                            node.node_id
                        )));
                    }
                    self.validate_no_framework_receipt_consumers(node, &input_consumers)?;
                }
                Some(spec::FrameworkNodeSpec::PublicOutputRender(_)) => {
                    self.validate_framework_receipt_consumers(
                        node,
                        &input_consumers,
                        "project-retention-manifest lifecycle node",
                        |consumer| {
                            matches!(
                                &consumer.framework,
                                Some(spec::FrameworkNodeSpec::ProjectRetentionManifest(retention))
                                    if retention.public_output_receipt_cell == node.output_cell
                            )
                        },
                    )?;
                }
                Some(spec::FrameworkNodeSpec::ProjectRetentionManifest(retention)) => {
                    retention_count += 1;
                    self.validate_framework_descriptor(
                        node,
                        "mfm.framework.project_retention_manifest",
                    )?;
                    if retention.public_schema_id != self.spec().public_outputs.public_schema_id {
                        return Err(RuntimeError::InvalidSpec(format!(
                            "retention lifecycle node {} public schema mismatch",
                            node.node_id
                        )));
                    }
                    self.validate_framework_output_cell(
                        node,
                        &spec::retention_manifest_receipt_schema_id()?,
                        &spec::retention_manifest_receipt_semantic_type_id()?,
                        spec::StoragePolicy::ContentAddressed,
                    )?;
                    let public_output_receipt_cell = self
                        .cells
                        .get(&retention.public_output_receipt_cell)
                        .ok_or_else(|| {
                            RuntimeError::InvalidSpec(format!(
                                "retention lifecycle node {} missing public-output receipt cell",
                                node.node_id
                            ))
                        })?;
                    self.validate_framework_input_binding(
                        node,
                        &spec::framework_lifecycle_receipt_input_binding(
                            "project_retention_manifest",
                            "public_output_receipt",
                            public_output_receipt_cell,
                        )?,
                    )?;
                    let input_cells = self.validate_input_binding(&node.input_bindings.root)?;
                    if input_cells != vec![retention.public_output_receipt_cell.clone()] {
                        return Err(RuntimeError::InvalidSpec(format!(
                            "retention lifecycle node {} must depend on the public-output receipt cell",
                            node.node_id
                        )));
                    }
                    let render_node = self.nodes.values().find(|candidate| {
                        candidate.output_cell == retention.public_output_receipt_cell
                            && matches!(
                                &candidate.framework,
                                Some(spec::FrameworkNodeSpec::PublicOutputRender(render))
                                    if render.public_schema_id == retention.public_schema_id
                            )
                    });
                    if render_node.is_none() {
                        return Err(RuntimeError::InvalidSpec(format!(
                            "retention lifecycle node {} is not ordered after public-output render",
                            node.node_id
                        )));
                    }
                    self.validate_framework_receipt_consumers(
                        node,
                        &input_consumers,
                        "complete-run lifecycle node",
                        |consumer| {
                            matches!(
                                &consumer.framework,
                                Some(spec::FrameworkNodeSpec::CompleteRun(complete))
                                    if complete.retention_manifest_receipt_cell == node.output_cell
                            )
                        },
                    )?;
                }
                Some(spec::FrameworkNodeSpec::CompleteRun(complete)) => {
                    completion_count += 1;
                    self.validate_framework_descriptor(node, "mfm.framework.complete_run")?;
                    if complete.public_schema_id != self.spec().public_outputs.public_schema_id {
                        return Err(RuntimeError::InvalidSpec(format!(
                            "completion lifecycle node {} public schema mismatch",
                            node.node_id
                        )));
                    }
                    self.validate_framework_output_cell(
                        node,
                        &spec::complete_run_receipt_schema_id()?,
                        &spec::complete_run_receipt_semantic_type_id()?,
                        spec::StoragePolicy::ContentAddressed,
                    )?;
                    let retention_manifest_receipt_cell = self
                        .cells
                        .get(&complete.retention_manifest_receipt_cell)
                        .ok_or_else(|| {
                            RuntimeError::InvalidSpec(format!(
                                "completion lifecycle node {} missing retention receipt cell",
                                node.node_id
                            ))
                        })?;
                    self.validate_framework_input_binding(
                        node,
                        &spec::framework_lifecycle_receipt_input_binding(
                            "complete_run",
                            "retention_manifest_receipt",
                            retention_manifest_receipt_cell,
                        )?,
                    )?;
                    let input_cells = self.validate_input_binding(&node.input_bindings.root)?;
                    if input_cells != vec![complete.retention_manifest_receipt_cell.clone()] {
                        return Err(RuntimeError::InvalidSpec(format!(
                            "completion lifecycle node {} must depend on the retention receipt cell",
                            node.node_id
                        )));
                    }
                    let retention_node = lifecycle_outputs
                        .get(&complete.retention_manifest_receipt_cell)
                        .filter(|candidate| {
                            matches!(
                                &candidate.framework,
                                Some(spec::FrameworkNodeSpec::ProjectRetentionManifest(retention))
                                    if retention.public_schema_id == complete.public_schema_id
                            )
                        });
                    if retention_node.is_none() {
                        return Err(RuntimeError::InvalidSpec(format!(
                            "completion lifecycle node {} is not ordered after retention projection",
                            node.node_id
                        )));
                    }
                    self.validate_no_framework_receipt_consumers(node, &input_consumers)?;
                }
                Some(spec::FrameworkNodeSpec::Bridge(_)) | None => {}
            }
        }

        if retention_count != 1 {
            return Err(RuntimeError::InvalidSpec(format!(
                "expected exactly one retention lifecycle framework node, found {retention_count}"
            )));
        }
        if completion_count != 1 {
            return Err(RuntimeError::InvalidSpec(format!(
                "expected exactly one completion lifecycle framework node, found {completion_count}"
            )));
        }
        if bootstrap_count > 1 {
            return Err(RuntimeError::InvalidSpec(
                "duplicate lifecycle framework nodes are not allowed".to_owned(),
            ));
        }
        self.validate_lifecycle_tail_finality()?;
        Ok(())
    }

    fn validate_lifecycle_tail_finality(&self) -> Result<()> {
        let render_node = self
            .nodes
            .values()
            .find(|node| {
                matches!(
                    node.framework,
                    Some(spec::FrameworkNodeSpec::PublicOutputRender(_))
                )
            })
            .ok_or_else(|| {
                RuntimeError::InvalidSpec("missing public-output render node".to_owned())
            })?;
        let mut render_ancestors = BTreeSet::new();
        self.collect_deterministic_ancestors(render_node, &mut render_ancestors)?;
        for node in self.nodes.values() {
            if node.node_id == render_node.node_id || render_ancestors.contains(&node.node_id) {
                continue;
            }
            match &node.framework {
                Some(
                    spec::FrameworkNodeSpec::BootstrapRun(_)
                    | spec::FrameworkNodeSpec::ProjectRetentionManifest(_)
                    | spec::FrameworkNodeSpec::CompleteRun(_),
                ) => {}
                _ => {
                    return Err(RuntimeError::InvalidSpec(format!(
                        "node {} is outside the certified public-output lifecycle tail",
                        node.node_id
                    )));
                }
            }
        }
        Ok(())
    }

    fn collect_deterministic_ancestors(
        &self,
        node: &spec::NodeSpec,
        ancestors: &mut BTreeSet<NodeId>,
    ) -> Result<()> {
        for predecessor_id in &node.deterministic_predecessors {
            if ancestors.insert(predecessor_id.clone()) {
                let predecessor = self.nodes.get(predecessor_id).ok_or_else(|| {
                    RuntimeError::InvalidSpec(format!(
                        "node {} references missing predecessor {}",
                        node.node_id, predecessor_id
                    ))
                })?;
                self.collect_deterministic_ancestors(predecessor, ancestors)?;
            }
        }
        Ok(())
    }

    fn validate_framework_permissions(&self, node: &spec::NodeSpec) -> Result<()> {
        if node.side_effect.is_some()
            || !node.adapter_bindings.is_empty()
            || !node.capability_bindings.capabilities.is_empty()
        {
            return Err(RuntimeError::InvalidSpec(format!(
                "framework node {} carries user-controlled execution permissions",
                node.node_id
            )));
        }
        Ok(())
    }

    fn validate_framework_config_ref(&self, node: &spec::NodeSpec, kind: &str) -> Result<()> {
        let expected = spec::framework_config_ref(kind, &node.node_id)?;
        if node.config_ref != expected {
            return Err(RuntimeError::InvalidSpec(format!(
                "framework node {} config ref is not deterministic for {kind}",
                node.node_id
            )));
        }
        Ok(())
    }

    fn validate_framework_input_binding(
        &self,
        node: &spec::NodeSpec,
        expected: &spec::InputBindingSpec,
    ) -> Result<()> {
        if &node.input_bindings != expected {
            return Err(RuntimeError::InvalidSpec(format!(
                "framework node {} input binding is not deterministic",
                node.node_id
            )));
        }
        Ok(())
    }

    fn validate_framework_descriptor_variant(
        &self,
        node: &spec::NodeSpec,
        descriptor: &spec::StateDescriptorIdentity,
    ) -> Result<()> {
        let matches_variant = match descriptor.name.as_str() {
            "mfm.framework.bootstrap_run" => {
                matches!(
                    &node.framework,
                    Some(spec::FrameworkNodeSpec::BootstrapRun(_))
                )
            }
            "mfm.framework.bridge_same_value" => {
                matches!(&node.framework, Some(spec::FrameworkNodeSpec::Bridge(_)))
            }
            "mfm.framework.render_public_outputs" => {
                matches!(
                    &node.framework,
                    Some(spec::FrameworkNodeSpec::PublicOutputRender(_))
                )
            }
            "mfm.framework.project_retention_manifest" => {
                matches!(
                    &node.framework,
                    Some(spec::FrameworkNodeSpec::ProjectRetentionManifest(_))
                )
            }
            "mfm.framework.complete_run" => {
                matches!(
                    &node.framework,
                    Some(spec::FrameworkNodeSpec::CompleteRun(_))
                )
            }
            _ => return Ok(()),
        };
        if !matches_variant {
            return Err(RuntimeError::InvalidSpec(format!(
                "node {} uses built-in framework descriptor {} without matching framework metadata",
                node.node_id, descriptor.name
            )));
        }
        Ok(())
    }

    fn validate_no_framework_receipt_consumers(
        &self,
        node: &spec::NodeSpec,
        consumers_by_cell: &BTreeMap<CellId, Vec<NodeId>>,
    ) -> Result<()> {
        if consumers_by_cell
            .get(&node.output_cell)
            .map(|consumers| !consumers.is_empty())
            .unwrap_or(false)
        {
            return Err(RuntimeError::InvalidSpec(format!(
                "framework receipt cell {} must not be consumed",
                node.output_cell
            )));
        }
        Ok(())
    }

    fn validate_framework_receipt_consumers(
        &self,
        node: &spec::NodeSpec,
        consumers_by_cell: &BTreeMap<CellId, Vec<NodeId>>,
        expected_consumer: &str,
        mut allowed: impl FnMut(&spec::NodeSpec) -> bool,
    ) -> Result<()> {
        for consumer_id in consumers_by_cell
            .get(&node.output_cell)
            .into_iter()
            .flatten()
        {
            let consumer = self.nodes.get(consumer_id).ok_or_else(|| {
                RuntimeError::InvalidSpec(format!(
                    "framework receipt cell {} has missing consumer node {}",
                    node.output_cell, consumer_id
                ))
            })?;
            if !allowed(consumer) {
                return Err(RuntimeError::InvalidSpec(format!(
                    "framework receipt cell {} may only feed {expected_consumer}",
                    node.output_cell
                )));
            }
        }
        Ok(())
    }

    fn validate_framework_descriptor(
        &self,
        node: &spec::NodeSpec,
        expected_name: &str,
    ) -> Result<()> {
        let descriptor = self.state_descriptor_for_node(node)?;
        let managed_effect = ManagedPlatformWrite::descriptor()
            .map_err(|error| RuntimeError::InvalidSpec(error.to_string()))?;
        if descriptor.name != expected_name
            || descriptor.runner != "managed_platform_write"
            || descriptor.effect_kind != managed_effect.kind
            || descriptor.effect_class != managed_effect.class.as_str()
            || descriptor.effect_name != managed_effect.name
            || !descriptor.capabilities.capabilities.is_empty()
        {
            return Err(RuntimeError::InvalidSpec(format!(
                "framework node {} is not bound to {expected_name}",
                node.node_id
            )));
        }
        Ok(())
    }

    fn validate_framework_output_cell(
        &self,
        node: &spec::NodeSpec,
        expected_schema_id: &SchemaId,
        expected_semantic_type_id: &SemanticTypeId,
        expected_storage_policy: spec::StoragePolicy,
    ) -> Result<()> {
        let output = self.cells.get(&node.output_cell).ok_or_else(|| {
            RuntimeError::InvalidSpec(format!(
                "framework node {} output cell {} is missing",
                node.node_id, node.output_cell
            ))
        })?;
        if output.schema_id != *expected_schema_id
            || output.semantic_type_id != *expected_semantic_type_id
            || output.terminal_policy != spec::CellTerminalPolicy::ProducedOnly
            || output.storage_policy != expected_storage_policy
            || output.redaction_policy != spec::RedactionPolicy::Public
        {
            return Err(RuntimeError::InvalidSpec(format!(
                "framework node {} output cell contract mismatch",
                node.node_id
            )));
        }
        Ok(())
    }

    fn validate_input_binding(&self, input: &spec::InputBindingNodeSpec) -> Result<Vec<CellId>> {
        let mut cells = Vec::new();
        self.validate_input_node(input, &mut cells)?;
        Ok(cells)
    }

    fn validate_input_node(
        &self,
        input: &spec::InputBindingNodeSpec,
        cells: &mut Vec<CellId>,
    ) -> Result<()> {
        match input {
            spec::InputBindingNodeSpec::Unit => {}
            spec::InputBindingNodeSpec::Cell(cell) => {
                let certified = self.cells.get(&cell.cell_id).ok_or_else(|| {
                    RuntimeError::InvalidSpec(format!(
                        "input binding references missing cell {}",
                        cell.cell_id
                    ))
                })?;
                if certified.semantic_type_id != cell.semantic_type_id
                    || certified.schema_id != cell.schema_id
                    || certified.value_lineage != cell.value_lineage
                {
                    return Err(RuntimeError::InvalidSpec(format!(
                        "input binding for cell {} does not match certified cell metadata",
                        cell.cell_id
                    )));
                }
                cells.push(cell.cell_id.clone());
            }
            spec::InputBindingNodeSpec::Tuple(elements) => {
                for element in elements {
                    self.validate_input_node(element, cells)?;
                }
            }
            spec::InputBindingNodeSpec::Struct(fields) => {
                for field in fields {
                    self.validate_input_node(&field.node, cells)?;
                }
            }
            spec::InputBindingNodeSpec::Vec { elements, .. }
            | spec::InputBindingNodeSpec::NonEmptyVec { elements, .. } => {
                for element in elements {
                    self.validate_input_node(element, cells)?;
                }
            }
        }
        Ok(())
    }

    fn predecessors_for_input_cells(&self, input_cells: &[CellId]) -> Result<Vec<NodeId>> {
        let mut predecessors = BTreeSet::new();
        for cell_id in input_cells {
            let cell = self.cells.get(cell_id).ok_or_else(|| {
                RuntimeError::InvalidSpec(format!("input cell {cell_id} is missing"))
            })?;
            if let spec::CellProducer::Node(node_id) = &cell.producer {
                predecessors.insert(node_id.clone());
            }
        }
        Ok(predecessors.into_iter().collect())
    }

    fn compute_topological_order(&self) -> Result<Vec<NodeId>> {
        let mut indegree = BTreeMap::<NodeId, usize>::new();
        let mut successors = BTreeMap::<NodeId, BTreeSet<NodeId>>::new();
        for node_id in self.nodes.keys() {
            indegree.insert(node_id.clone(), 0);
            successors.insert(node_id.clone(), BTreeSet::new());
        }
        for node in self.nodes.values() {
            for predecessor in &node.deterministic_predecessors {
                if !self.nodes.contains_key(predecessor) {
                    return Err(RuntimeError::InvalidSpec(format!(
                        "node {} references missing predecessor {}",
                        node.node_id, predecessor
                    )));
                }
                successors
                    .get_mut(predecessor)
                    .expect("predecessor exists")
                    .insert(node.node_id.clone());
                *indegree.get_mut(&node.node_id).expect("node exists") += 1;
            }
        }

        let mut ready = indegree
            .iter()
            .filter_map(|(node_id, count)| (*count == 0).then_some(node_id.clone()))
            .collect::<VecDeque<_>>();
        let mut order = Vec::with_capacity(self.nodes.len());
        while let Some(node_id) = ready.pop_front() {
            order.push(node_id.clone());
            for successor in successors.get(&node_id).expect("successors exist") {
                let count = indegree.get_mut(successor).expect("successor exists");
                *count -= 1;
                if *count == 0 {
                    let index = ready
                        .iter()
                        .position(|queued| successor < queued)
                        .unwrap_or(ready.len());
                    ready.insert(index, successor.clone());
                }
            }
        }
        if order.len() != self.nodes.len() {
            return Err(RuntimeError::InvalidSpec(
                "certified node graph contains a cycle".to_owned(),
            ));
        }
        Ok(order)
    }
}

/// Registered erased runner binding for one certified state descriptor.
#[derive(Clone)]
pub struct ErasedRunnerBinding {
    descriptor_id: DescriptorId,
    factory_id: events::RunnerFactoryId,
    executable: events::ExecutableIdentity,
    runner: Arc<dyn ErasedNodeRunner>,
}

impl ErasedRunnerBinding {
    /// Creates a runner binding for a certified state descriptor id.
    pub fn new(
        descriptor_id: DescriptorId,
        factory_id: events::RunnerFactoryId,
        executable: events::ExecutableIdentity,
        runner: Arc<dyn ErasedNodeRunner>,
    ) -> Result<Self> {
        if executable.factory_id != factory_id {
            return Err(RuntimeError::RunnerBinding(format!(
                "executable factory {} does not match binding factory {}",
                executable.factory_id, factory_id
            )));
        }
        Ok(Self {
            descriptor_id,
            factory_id,
            executable,
            runner,
        })
    }

    /// Certified descriptor id this binding executes.
    pub fn descriptor_id(&self) -> &DescriptorId {
        &self.descriptor_id
    }

    /// Runner factory id.
    pub fn factory_id(&self) -> &events::RunnerFactoryId {
        &self.factory_id
    }

    /// Executable identity for run-start evidence.
    pub fn executable(&self) -> &events::ExecutableIdentity {
        &self.executable
    }
}

/// Registry of erased runners keyed by certified state descriptor id.
#[derive(Clone, Default)]
pub struct ErasedRunnerRegistry {
    bindings: BTreeMap<DescriptorId, ErasedRunnerBinding>,
}

impl ErasedRunnerRegistry {
    /// Creates an empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers one erased runner binding.
    pub fn register(&mut self, binding: ErasedRunnerBinding) -> Result<()> {
        if self
            .bindings
            .insert(binding.descriptor_id.clone(), binding)
            .is_some()
        {
            return Err(RuntimeError::RunnerBinding(
                "duplicate runner binding".to_owned(),
            ));
        }
        Ok(())
    }

    fn resolve(
        &self,
        node: &spec::NodeSpec,
        descriptor: &spec::StateDescriptorIdentity,
    ) -> Result<ErasedRunnerBinding> {
        if matches!(
            &node.framework,
            Some(spec::FrameworkNodeSpec::PublicOutputRender(_))
        ) {
            return framework_public_output_binding(node, descriptor);
        }
        if matches!(
            &node.framework,
            Some(spec::FrameworkNodeSpec::ProjectRetentionManifest(_))
        ) {
            return framework_retention_manifest_binding(node, descriptor);
        }
        if matches!(
            &node.framework,
            Some(spec::FrameworkNodeSpec::CompleteRun(_))
        ) {
            return framework_complete_run_binding(node, descriptor);
        }
        let binding = self.bindings.get(&node.descriptor_id).ok_or_else(|| {
            RuntimeError::RunnerBinding(format!(
                "missing runner binding for node {} descriptor {}",
                node.node_id, node.descriptor_id
            ))
        })?;
        if binding.descriptor_id != node.descriptor_id {
            return Err(RuntimeError::RunnerBinding(format!(
                "runner binding descriptor mismatch for node {}",
                node.node_id
            )));
        }
        if binding.factory_id.as_str() != descriptor.runner {
            return Err(RuntimeError::RunnerBinding(format!(
                "runner binding factory {} does not match descriptor runner {} for node {}",
                binding.factory_id, descriptor.runner, node.node_id
            )));
        }
        Ok(binding.clone())
    }

    fn executables_for_spec(
        &self,
        runtime_spec: &CertifiedRuntimeSpec,
    ) -> Result<Vec<events::ExecutableIdentity>> {
        let mut seen = BTreeSet::new();
        let mut executables = Vec::new();
        for node_id in runtime_spec.topological_order() {
            let node = runtime_spec.node(node_id).expect("topological node exists");
            let descriptor = runtime_spec.state_descriptor_for_node(node)?;
            let binding = self.resolve(node, descriptor)?;
            let key = binding.executable.factory_id.as_str().to_owned();
            if seen.insert(key) {
                executables.push(binding.executable.clone());
            }
        }
        Ok(executables)
    }
}

fn framework_public_output_binding(
    node: &spec::NodeSpec,
    descriptor: &spec::StateDescriptorIdentity,
) -> Result<ErasedRunnerBinding> {
    let Some(spec::FrameworkNodeSpec::PublicOutputRender(_)) = &node.framework else {
        return Err(RuntimeError::RunnerBinding(format!(
            "node {} is not a public-output render node",
            node.node_id
        )));
    };
    if descriptor.name != "mfm.framework.render_public_outputs" {
        return Err(RuntimeError::RunnerBinding(format!(
            "public-output render node {} has non-framework descriptor {}",
            node.node_id, descriptor.name
        )));
    }
    let factory_id = events::RunnerFactoryId::new(descriptor.runner.as_str())?;
    ErasedRunnerBinding::new(
        node.descriptor_id.clone(),
        factory_id.clone(),
        framework_public_output_executable(factory_id)?,
        Arc::new(FrameworkPublicOutputRunner),
    )
}

fn framework_public_output_executable(
    factory_id: events::RunnerFactoryId,
) -> Result<events::ExecutableIdentity> {
    let package_digest = content_digest_json(serde_json::json!({
        "crate": "mfm-runtime",
        "runner": "framework_public_output",
        "version": env!("CARGO_PKG_VERSION"),
    }))?;
    let binary_digest = content_digest_json(serde_json::json!({
        "crate": "mfm-runtime",
        "factory_id": factory_id.as_str(),
        "runner": "framework_public_output",
        "version": env!("CARGO_PKG_VERSION"),
    }))?;
    Ok(events::ExecutableIdentity {
        factory_id,
        source_revision: events::SourceRevision::new("mfm-runtime-built-in")?,
        cargo_package_name: events::PackageName::new("mfm-runtime")?,
        cargo_package_version: events::PackageVersion::new(env!("CARGO_PKG_VERSION"))?,
        cargo_package_digest: package_digest,
        binary_digest,
        nix_derivation_hash: None,
        nix_output_hash: None,
    })
}

struct FrameworkPublicOutputRunner;

impl ErasedNodeRunner for FrameworkPublicOutputRunner {
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move { render_public_output(ctx) })
    }
}

fn framework_retention_manifest_binding(
    node: &spec::NodeSpec,
    descriptor: &spec::StateDescriptorIdentity,
) -> Result<ErasedRunnerBinding> {
    let Some(spec::FrameworkNodeSpec::ProjectRetentionManifest(_)) = &node.framework else {
        return Err(RuntimeError::RunnerBinding(format!(
            "node {} is not a retention-manifest framework node",
            node.node_id
        )));
    };
    if descriptor.name != "mfm.framework.project_retention_manifest" {
        return Err(RuntimeError::RunnerBinding(format!(
            "retention-manifest node {} has non-framework descriptor {}",
            node.node_id, descriptor.name
        )));
    }
    let factory_id = events::RunnerFactoryId::new(descriptor.runner.as_str())?;
    ErasedRunnerBinding::new(
        node.descriptor_id.clone(),
        factory_id.clone(),
        framework_retention_manifest_executable(factory_id)?,
        Arc::new(FrameworkRetentionManifestRunner),
    )
}

fn framework_retention_manifest_executable(
    factory_id: events::RunnerFactoryId,
) -> Result<events::ExecutableIdentity> {
    let package_digest = content_digest_json(serde_json::json!({
        "crate": "mfm-runtime",
        "runner": "framework_retention_manifest",
        "version": env!("CARGO_PKG_VERSION"),
    }))?;
    let binary_digest = content_digest_json(serde_json::json!({
        "crate": "mfm-runtime",
        "factory_id": factory_id.as_str(),
        "runner": "framework_retention_manifest",
        "version": env!("CARGO_PKG_VERSION"),
    }))?;
    Ok(events::ExecutableIdentity {
        factory_id,
        source_revision: events::SourceRevision::new("mfm-runtime-built-in")?,
        cargo_package_name: events::PackageName::new("mfm-runtime")?,
        cargo_package_version: events::PackageVersion::new(env!("CARGO_PKG_VERSION"))?,
        cargo_package_digest: package_digest,
        binary_digest,
        nix_derivation_hash: None,
        nix_output_hash: None,
    })
}

struct FrameworkRetentionManifestRunner;

impl ErasedNodeRunner for FrameworkRetentionManifestRunner {
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move { project_retention_manifest(ctx) })
    }
}

fn framework_complete_run_binding(
    node: &spec::NodeSpec,
    descriptor: &spec::StateDescriptorIdentity,
) -> Result<ErasedRunnerBinding> {
    let Some(spec::FrameworkNodeSpec::CompleteRun(_)) = &node.framework else {
        return Err(RuntimeError::RunnerBinding(format!(
            "node {} is not a complete-run framework node",
            node.node_id
        )));
    };
    if descriptor.name != "mfm.framework.complete_run" {
        return Err(RuntimeError::RunnerBinding(format!(
            "complete-run node {} has non-framework descriptor {}",
            node.node_id, descriptor.name
        )));
    }
    let factory_id = events::RunnerFactoryId::new(descriptor.runner.as_str())?;
    ErasedRunnerBinding::new(
        node.descriptor_id.clone(),
        factory_id.clone(),
        framework_complete_run_executable(factory_id)?,
        Arc::new(FrameworkCompleteRunRunner),
    )
}

fn framework_complete_run_executable(
    factory_id: events::RunnerFactoryId,
) -> Result<events::ExecutableIdentity> {
    let package_digest = content_digest_json(serde_json::json!({
        "crate": "mfm-runtime",
        "runner": "framework_complete_run",
        "version": env!("CARGO_PKG_VERSION"),
    }))?;
    let binary_digest = content_digest_json(serde_json::json!({
        "crate": "mfm-runtime",
        "factory_id": factory_id.as_str(),
        "runner": "framework_complete_run",
        "version": env!("CARGO_PKG_VERSION"),
    }))?;
    Ok(events::ExecutableIdentity {
        factory_id,
        source_revision: events::SourceRevision::new("mfm-runtime-built-in")?,
        cargo_package_name: events::PackageName::new("mfm-runtime")?,
        cargo_package_version: events::PackageVersion::new(env!("CARGO_PKG_VERSION"))?,
        cargo_package_digest: package_digest,
        binary_digest,
        nix_derivation_hash: None,
        nix_output_hash: None,
    })
}

struct FrameworkCompleteRunRunner;

impl ErasedNodeRunner for FrameworkCompleteRunRunner {
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move { complete_run_framework(ctx) })
    }
}

fn render_public_output(ctx: ErasedRunCtx<'_>) -> Result<ErasedRunnerOutput> {
    let Some(spec::FrameworkNodeSpec::PublicOutputRender(render)) = &ctx.node().framework else {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "node {} is not a public-output render node",
            ctx.node().node_id
        )));
    };
    let mut cells = Vec::with_capacity(render.required_cells.len());
    for required in &render.required_cells {
        let Some(store::CellTerminalProjection::Produced {
            artifact_id,
            content_digest,
            ..
        }) = ctx.projections().cell_terminal(&required.cell_id)
        else {
            return Err(RuntimeError::InvalidRunnerOutput(format!(
                "public-output render node {} required incomplete cell {}",
                ctx.node().node_id,
                required.cell_id
            )));
        };
        cells.push(events::NamedTypedCellRef {
            public_field_path: required.public_field_path.clone(),
            cell_id: required.cell_id.clone(),
            producer: required.producer.clone(),
            scope_id: required.scope_id.clone(),
            semantic_type_id: required.semantic_type_id.clone(),
            schema_id: required.schema_id.clone(),
            value_lineage: required.value_lineage.clone(),
            content_digest: content_digest.clone(),
            artifact_id: artifact_id.clone(),
        });
    }
    let rendered_digest = public_output_rendered_digest(render, &cells)?;
    let rendered_artifact_id = None;
    let receipt_bytes = public_output_receipt_json(
        render,
        &cells,
        &rendered_digest,
        rendered_artifact_id.as_ref(),
    )?;
    let receipt_digest = receipt_bytes.content_digest();
    let receipt_artifact_id =
        ArtifactId::from_digest(receipt_digest.algorithm(), *receipt_digest.digest());
    let receipt_artifact = store::ArtifactEvidenceRef {
        artifact_id: receipt_artifact_id.clone(),
        digest: receipt_digest.clone(),
        byte_len: receipt_bytes.as_bytes().len() as u64,
        media_type: spec::MediaType::new("application/json")?,
        schema_id: Some(ctx.output_cell().schema_id.clone()),
        semantic_type_id: Some(ctx.output_cell().semantic_type_id.clone()),
        producer_node_id: Some(ctx.node().node_id.clone()),
        producer_seed_id: None,
        artifact_role: events::ArtifactRole::StateOutput,
    };
    let receipt_artifact =
        StagedArtifact::inline_attempt_artifact(&ctx, receipt_bytes.to_vec(), receipt_artifact)?;
    let receipt_retention_ref = retention_ref_for_artifact(receipt_artifact.evidence());
    Ok(ErasedRunnerOutput {
        staged_artifacts: vec![receipt_artifact],
        staged_retention_refs: vec![StagedRetentionRefs::framework_public_output(vec![
            receipt_retention_ref,
        ])],
        payloads: vec![
            events::KernelEventPayload::CellProduced(events::CellProduced {
                spec_hash: ctx.spec_hash().clone(),
                node_id: ctx.node().node_id.clone(),
                cell_id: ctx.node().output_cell.clone(),
                scope_id: ctx.output_cell().scope_id.clone(),
                attempt_id: ctx.attempt_id().clone(),
                semantic_type_id: ctx.output_cell().semantic_type_id.clone(),
                schema_id: ctx.output_cell().schema_id.clone(),
                value_lineage: ctx.output_cell().value_lineage.clone(),
                artifact_id: receipt_artifact_id,
                content_digest: receipt_digest,
                producer_state_kind: Some(ctx.node().state_kind.clone()),
                producer_state_version: Some(ctx.node().state_version.clone()),
            }),
            events::KernelEventPayload::PublicOutputProduced(events::PublicOutputProduced {
                spec_hash: ctx.spec_hash().clone(),
                node_id: ctx.node().node_id.clone(),
                attempt_id: ctx.attempt_id().clone(),
                receipt_cell_id: ctx.node().output_cell.clone(),
                public_schema_id: render.public_schema_id.clone(),
                output_spec_digest: render.output_spec_digest.clone(),
                cells,
                rendered_digest,
                rendered_artifact_id,
                renderer_descriptor_id: render.renderer_descriptor.descriptor_id.clone(),
            }),
            events::KernelEventPayload::StateAttemptCompleted(events::StateAttemptCompleted {
                spec_hash: ctx.spec_hash().clone(),
                node_id: ctx.node().node_id.clone(),
                attempt_id: ctx.attempt_id().clone(),
                output_cell_id: ctx.node().output_cell.clone(),
            }),
        ],
    })
}

fn project_retention_manifest(ctx: ErasedRunCtx<'_>) -> Result<ErasedRunnerOutput> {
    let Some(spec::FrameworkNodeSpec::ProjectRetentionManifest(_)) = &ctx.node().framework else {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "node {} is not a retention-manifest framework node",
            ctx.node().node_id
        )));
    };
    let manifest = build_retention_manifest_artifact_with_producer(
        ctx.runtime_spec(),
        ctx.run_id(),
        ctx.run_stream(),
        Some(ctx.node().node_id.clone()),
    )?;
    let manifest_artifact = StagedArtifact::inline_retention_manifest_artifact(
        &ctx,
        manifest.bytes.to_vec(),
        manifest.evidence.clone(),
    )?;
    let receipt_bytes = retention_manifest_receipt_json(&manifest, ctx.run_stream())?;
    let receipt_digest = receipt_bytes.content_digest();
    let receipt_artifact_id =
        ArtifactId::from_digest(receipt_digest.algorithm(), *receipt_digest.digest());
    let receipt_artifact = store::ArtifactEvidenceRef {
        artifact_id: receipt_artifact_id.clone(),
        digest: receipt_digest.clone(),
        byte_len: receipt_bytes.as_bytes().len() as u64,
        media_type: spec::MediaType::new("application/json")?,
        schema_id: Some(ctx.output_cell().schema_id.clone()),
        semantic_type_id: Some(ctx.output_cell().semantic_type_id.clone()),
        producer_node_id: Some(ctx.node().node_id.clone()),
        producer_seed_id: None,
        artifact_role: events::ArtifactRole::StateOutput,
    };
    let receipt_artifact =
        StagedArtifact::inline_attempt_artifact(&ctx, receipt_bytes.to_vec(), receipt_artifact)?;
    let receipt_retention_ref = retention_ref_for_artifact(receipt_artifact.evidence());
    Ok(ErasedRunnerOutput {
        staged_artifacts: vec![manifest_artifact, receipt_artifact],
        staged_retention_refs: vec![StagedRetentionRefs::runtime_evidence(vec![
            receipt_retention_ref,
        ])],
        payloads: vec![
            events::KernelEventPayload::CellProduced(events::CellProduced {
                spec_hash: ctx.spec_hash().clone(),
                node_id: ctx.node().node_id.clone(),
                cell_id: ctx.node().output_cell.clone(),
                scope_id: ctx.output_cell().scope_id.clone(),
                attempt_id: ctx.attempt_id().clone(),
                semantic_type_id: ctx.output_cell().semantic_type_id.clone(),
                schema_id: ctx.output_cell().schema_id.clone(),
                value_lineage: ctx.output_cell().value_lineage.clone(),
                artifact_id: receipt_artifact_id,
                content_digest: receipt_digest,
                producer_state_kind: Some(ctx.node().state_kind.clone()),
                producer_state_version: Some(ctx.node().state_version.clone()),
            }),
            events::KernelEventPayload::StateAttemptCompleted(events::StateAttemptCompleted {
                spec_hash: ctx.spec_hash().clone(),
                node_id: ctx.node().node_id.clone(),
                attempt_id: ctx.attempt_id().clone(),
                output_cell_id: ctx.node().output_cell.clone(),
            }),
        ],
    })
}

fn complete_run_framework(ctx: ErasedRunCtx<'_>) -> Result<ErasedRunnerOutput> {
    let Some(spec::FrameworkNodeSpec::CompleteRun(_)) = &ctx.node().framework else {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "node {} is not a complete-run framework node",
            ctx.node().node_id
        )));
    };
    let completion = run_completion_evidence(ctx.runtime_spec(), ctx.projections())?;
    let retention_manifest = projected_retention_manifest(ctx.run_id(), ctx.projections())?;
    let receipt_bytes =
        complete_run_receipt_json(&completion, retention_manifest, ctx.run_stream())?;
    let receipt_digest = receipt_bytes.content_digest();
    let receipt_artifact_id =
        ArtifactId::from_digest(receipt_digest.algorithm(), *receipt_digest.digest());
    let receipt_artifact = store::ArtifactEvidenceRef {
        artifact_id: receipt_artifact_id.clone(),
        digest: receipt_digest.clone(),
        byte_len: receipt_bytes.as_bytes().len() as u64,
        media_type: spec::MediaType::new("application/json")?,
        schema_id: Some(ctx.output_cell().schema_id.clone()),
        semantic_type_id: Some(ctx.output_cell().semantic_type_id.clone()),
        producer_node_id: Some(ctx.node().node_id.clone()),
        producer_seed_id: None,
        artifact_role: events::ArtifactRole::StateOutput,
    };
    let receipt_artifact =
        StagedArtifact::inline_attempt_artifact(&ctx, receipt_bytes.to_vec(), receipt_artifact)?;
    Ok(ErasedRunnerOutput {
        staged_artifacts: vec![receipt_artifact],
        staged_retention_refs: Vec::new(),
        payloads: vec![
            events::KernelEventPayload::CellProduced(events::CellProduced {
                spec_hash: ctx.spec_hash().clone(),
                node_id: ctx.node().node_id.clone(),
                cell_id: ctx.node().output_cell.clone(),
                scope_id: ctx.output_cell().scope_id.clone(),
                attempt_id: ctx.attempt_id().clone(),
                semantic_type_id: ctx.output_cell().semantic_type_id.clone(),
                schema_id: ctx.output_cell().schema_id.clone(),
                value_lineage: ctx.output_cell().value_lineage.clone(),
                artifact_id: receipt_artifact_id,
                content_digest: receipt_digest,
                producer_state_kind: Some(ctx.node().state_kind.clone()),
                producer_state_version: Some(ctx.node().state_version.clone()),
            }),
            events::KernelEventPayload::StateAttemptCompleted(events::StateAttemptCompleted {
                spec_hash: ctx.spec_hash().clone(),
                node_id: ctx.node().node_id.clone(),
                attempt_id: ctx.attempt_id().clone(),
                output_cell_id: ctx.node().output_cell.clone(),
            }),
        ],
    })
}

fn public_output_rendered_digest(
    render: &spec::PublicOutputRenderNodeSpec,
    cells: &[events::NamedTypedCellRef],
) -> Result<ContentDigest> {
    content_digest_json(serde_json::json!({
        "cells": cells.iter().map(public_output_cell_json).collect::<Vec<_>>(),
        "output_spec_digest": render.output_spec_digest.as_str(),
        "public_schema_id": render.public_schema_id.as_str(),
        "renderer_descriptor_id": render.renderer_descriptor.descriptor_id.as_str(),
    }))
}

fn retention_manifest_receipt_json(
    manifest: &RetentionManifestArtifact,
    pre_projection_stream: &[store::KernelEventEnvelope],
) -> Result<PlainCanonicalJsonBytes> {
    canonical_json(serde_json::json!({
        "manifest_artifact_id": manifest.evidence.artifact_id.as_str(),
        "manifest_digest": manifest.evidence.digest.as_str(),
        "manifest_seq": manifest.manifest_seq,
        "pre_projection_stream_seq": pre_projection_stream
            .last()
            .map(|event| event.seq().as_u64()),
        "previous_manifest_digest": manifest.previous_manifest_digest.as_ref().map(ContentDigest::as_str),
    }))
}

fn complete_run_receipt_json(
    completion: &events::PublicOutputCompletionEvidence,
    retention_manifest: &store::RetentionManifestProjection,
    pre_completion_stream: &[store::KernelEventEnvelope],
) -> Result<PlainCanonicalJsonBytes> {
    canonical_json(serde_json::json!({
        "public_output_event_id": completion.public_output_event_id.as_str(),
        "public_output_schema_id": completion.public_output_schema_id.as_str(),
        "retention_manifest_artifact_id": retention_manifest.manifest_artifact_id.as_str(),
        "retention_manifest_digest": retention_manifest.manifest_digest.as_str(),
        "retention_manifest_seq": retention_manifest.manifest_seq,
        "pre_completion_stream_seq": pre_completion_stream
            .last()
            .map(|event| event.seq().as_u64()),
    }))
}

fn run_completion_evidence(
    runtime_spec: &CertifiedRuntimeSpec,
    projections: &store::ProjectionSnapshot,
) -> Result<events::PublicOutputCompletionEvidence> {
    let public_schema_id = runtime_spec.spec().public_outputs.public_schema_id.clone();
    match projections.public_output(&public_schema_id) {
        Some(store::PublicOutputProjection::Produced { event_id, .. }) => {
            Ok(events::PublicOutputCompletionEvidence {
                public_output_schema_id: public_schema_id,
                public_output_event_id: event_id.clone(),
            })
        }
        Some(store::PublicOutputProjection::RenderFailed { .. }) | None => {
            Err(RuntimeError::InvalidRunStream(
                "run completion requires projected public output evidence".to_owned(),
            ))
        }
    }
}

fn projected_retention_manifest<'a>(
    run_id: &RunId,
    projections: &'a store::ProjectionSnapshot,
) -> Result<&'a store::RetentionManifestProjection> {
    projections
        .retention(run_id)
        .and_then(|projection| projection.manifest.as_ref())
        .ok_or_else(|| {
            RuntimeError::InvalidRunStream(
                "run completion requires projected retention manifest evidence".to_owned(),
            )
        })
}

fn framework_run_completed_payload(
    runtime_spec: &CertifiedRuntimeSpec,
    run_id: &RunId,
    node: &spec::NodeSpec,
    projections: &store::ProjectionSnapshot,
) -> Result<Option<events::KernelEventPayload>> {
    let Some(spec::FrameworkNodeSpec::CompleteRun(complete)) = &node.framework else {
        return Ok(None);
    };
    let certified_node = certified_complete_run_node(runtime_spec)?;
    if certified_node.node_id != node.node_id {
        return Err(RuntimeError::InvalidSpec(format!(
            "complete-run node {} is not the certified completion lifecycle node",
            node.node_id
        )));
    }
    if complete.public_schema_id != runtime_spec.spec().public_outputs.public_schema_id {
        return Err(RuntimeError::InvalidSpec(format!(
            "complete-run node {} references public schema {} outside certified public outputs",
            node.node_id, complete.public_schema_id
        )));
    }
    let completion = run_completion_evidence(runtime_spec, projections)?;
    projected_retention_manifest(run_id, projections)?;
    Ok(Some(events::KernelEventPayload::RunCompleted(
        events::RunCompleted {
            run_id: run_id.clone(),
            spec_hash: runtime_spec.spec_hash().clone(),
            outcome: events::RunCompletionOutcome::Completed(completion),
        },
    )))
}

fn public_output_receipt_digest(
    render: &spec::PublicOutputRenderNodeSpec,
    cells: &[events::NamedTypedCellRef],
    rendered_digest: &ContentDigest,
    rendered_artifact_id: Option<&ArtifactId>,
) -> Result<ContentDigest> {
    Ok(
        public_output_receipt_json(render, cells, rendered_digest, rendered_artifact_id)?
            .content_digest(),
    )
}

fn public_output_receipt_json(
    render: &spec::PublicOutputRenderNodeSpec,
    cells: &[events::NamedTypedCellRef],
    rendered_digest: &ContentDigest,
    rendered_artifact_id: Option<&ArtifactId>,
) -> Result<PlainCanonicalJsonBytes> {
    canonical_json(serde_json::json!({
        "cells": cells.iter().map(public_output_cell_json).collect::<Vec<_>>(),
        "output_spec_digest": render.output_spec_digest.as_str(),
        "public_schema_id": render.public_schema_id.as_str(),
        "rendered_artifact_id": rendered_artifact_id.map(ArtifactId::as_str),
        "rendered_digest": rendered_digest.as_str(),
        "renderer_descriptor_id": render.renderer_descriptor.descriptor_id.as_str(),
    }))
}

fn public_output_cell_json(cell: &events::NamedTypedCellRef) -> serde_json::Value {
    serde_json::json!({
        "artifact_id": cell.artifact_id.as_str(),
        "cell_id": cell.cell_id.as_str(),
        "content_digest": cell.content_digest.as_str(),
        "producer": cell_producer_json(&cell.producer),
        "public_field_path": cell.public_field_path.as_str(),
        "schema_id": cell.schema_id.as_str(),
        "scope_id": cell.scope_id.as_str(),
        "semantic_type_id": cell.semantic_type_id.as_str(),
        "value_lineage": cell.value_lineage.lineage_digest.as_str(),
    })
}

/// Rebuilds framework public-output receipt artifact bytes and evidence.
pub fn build_public_output_receipt_artifact(
    runtime_spec: &CertifiedRuntimeSpec,
    payload: &events::PublicOutputProduced,
) -> Result<(PlainCanonicalJsonBytes, store::ArtifactEvidenceRef)> {
    let node = runtime_spec.node(&payload.node_id).ok_or_else(|| {
        RuntimeError::InvalidSpec(format!(
            "public-output payload references missing node {}",
            payload.node_id
        ))
    })?;
    validate_public_output(runtime_spec, node, payload)?;
    let Some(spec::FrameworkNodeSpec::PublicOutputRender(render)) = &node.framework else {
        return Err(RuntimeError::InvalidSpec(format!(
            "public-output node {} is not a framework render node",
            node.node_id
        )));
    };
    let output_cell = runtime_spec.cell(&node.output_cell).ok_or_else(|| {
        RuntimeError::InvalidSpec(format!(
            "public-output node {} output cell {} is missing",
            node.node_id, node.output_cell
        ))
    })?;
    let bytes = public_output_receipt_json(
        render,
        &payload.cells,
        &payload.rendered_digest,
        payload.rendered_artifact_id.as_ref(),
    )?;
    let digest = bytes.content_digest();
    let evidence = store::ArtifactEvidenceRef {
        artifact_id: ArtifactId::from_digest(digest.algorithm(), *digest.digest()),
        digest,
        byte_len: bytes.as_bytes().len() as u64,
        media_type: spec::MediaType::new("application/json")?,
        schema_id: Some(output_cell.schema_id.clone()),
        semantic_type_id: Some(output_cell.semantic_type_id.clone()),
        producer_node_id: Some(node.node_id.clone()),
        producer_seed_id: None,
        artifact_role: events::ArtifactRole::StateOutput,
    };
    Ok((bytes, evidence))
}

fn retention_manifest_json(
    runtime_spec: &CertifiedRuntimeSpec,
    run_id: &RunId,
    run_started: &events::RunStarted,
    manifest_seq: u64,
    previous_manifest_digest: Option<&ContentDigest>,
    retained_refs: &[&events::RetentionRef],
    stream: &[store::KernelEventEnvelope],
) -> Result<PlainCanonicalJsonBytes> {
    let spec_canonical = runtime_spec
        .spec()
        .canonical_json()
        .map_err(|error| RuntimeError::Canonical(error.to_string()))?;
    let spec_digest = spec_canonical.content_digest();
    let certificate_canonical = runtime_spec
        .certificate()
        .canonical_json()
        .map_err(|error| RuntimeError::Canonical(error.to_string()))?;
    let event_schema_ids = stream
        .iter()
        .map(|event| event.event_schema_id().as_str())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let retained_by_role = retained_refs_by_role(retained_refs);
    canonical_json(serde_json::json!({
        "adapter_executables": run_started.adapter_executables.iter().map(executable_json).collect::<Vec<_>>(),
        "canonicalizer_identity": run_started.canonicalizer_identity.as_str(),
        "certificate_artifact": {
            "artifact_id": run_started.certificate_artifact_id.as_str(),
            "byte_len": certificate_canonical.as_bytes().len(),
            "content_digest": run_started.certificate_artifact_digest.as_str(),
            "media_type": run_started.certificate_media_type.as_str(),
        },
        "config_artifacts": runtime_spec.spec().config_refs.iter().map(config_artifact_json).collect::<Vec<_>>(),
        "descriptor_digests": runtime_spec.spec().descriptor_identities.iter().map(descriptor_digest_json).collect::<Vec<_>>(),
        "descriptor_identities": runtime_spec.spec().descriptor_identities.iter().map(descriptor_identity_json).collect::<Vec<_>>(),
        "event_schema_ids": event_schema_ids,
        "manifest_seq": manifest_seq,
        "previous_manifest_digest": previous_manifest_digest.map(ContentDigest::as_str),
        "public_output_artifacts": retained_by_role.public_output_artifacts,
        "receipt_artifacts": retained_by_role.receipt_artifacts,
        "confirmation_artifacts": retained_by_role.confirmation_artifacts,
        "retained_refs": retained_refs.iter().map(|retention_ref| retention_ref_json(retention_ref)).collect::<Vec<_>>(),
        "run_id": run_id.as_str(),
        "runner_executables": run_started.runner_executables.iter().map(executable_json).collect::<Vec<_>>(),
        "spec_artifact": {
            "artifact_id": run_started.spec_artifact_id.as_str(),
            "byte_len": spec_canonical.as_bytes().len(),
            "content_digest": spec_digest.as_str(),
            "media_type": run_started.spec_media_type.as_str(),
        },
        "spec_hash": runtime_spec.spec_hash().as_str(),
        "value_artifacts": retained_by_role.value_artifacts,
    }))
}

struct RetainedRefsByRole {
    value_artifacts: Vec<String>,
    receipt_artifacts: Vec<String>,
    confirmation_artifacts: Vec<String>,
    public_output_artifacts: Vec<String>,
}

fn retained_refs_by_role(retained_refs: &[&events::RetentionRef]) -> RetainedRefsByRole {
    let mut value_artifacts = Vec::new();
    let mut receipt_artifacts = Vec::new();
    let mut confirmation_artifacts = Vec::new();
    let mut public_output_artifacts = Vec::new();
    for retention_ref in retained_refs {
        match retention_ref.role {
            events::ArtifactRole::StateOutput
            | events::ArtifactRole::FactResponse
            | events::ArtifactRole::SideEffectIntent
            | events::ArtifactRole::PreparedInvocation
            | events::ArtifactRole::NotSubmittedProof
            | events::ArtifactRole::Submission
            | events::ArtifactRole::SubmissionUnknownEvidence
            | events::ArtifactRole::AmbiguityEvidence
            | events::ArtifactRole::RedactedDiagnostic => {
                value_artifacts.push(retention_ref.artifact_id.as_str().to_owned());
            }
            events::ArtifactRole::Receipt => {
                receipt_artifacts.push(retention_ref.artifact_id.as_str().to_owned());
            }
            events::ArtifactRole::Confirmation => {
                confirmation_artifacts.push(retention_ref.artifact_id.as_str().to_owned());
            }
            events::ArtifactRole::PublicOutput => {
                public_output_artifacts.push(retention_ref.artifact_id.as_str().to_owned());
            }
            events::ArtifactRole::TypedExecutionSpec
            | events::ArtifactRole::TypedSpecCertificate
            | events::ArtifactRole::TypedConfig
            | events::ArtifactRole::SeedInput
            | events::ArtifactRole::RetentionManifest => {}
        }
    }
    RetainedRefsByRole {
        value_artifacts,
        receipt_artifacts,
        confirmation_artifacts,
        public_output_artifacts,
    }
}

fn config_artifact_json(config: &spec::ConfigRef) -> serde_json::Value {
    serde_json::json!({
        "artifact_id": config.artifact_id.as_str(),
        "byte_len": config.byte_len,
        "content_digest": config.digest.as_str(),
        "media_type": config.media_type.as_str(),
        "schema_id": config.schema_id.as_str(),
    })
}

fn descriptor_identity_json(identity: &spec::DescriptorIdentity) -> serde_json::Value {
    match identity {
        spec::DescriptorIdentity::State(identity) => serde_json::json!({
            "descriptor_family": "state",
            "descriptor_id": identity.descriptor_id.as_str(),
            "name": identity.name.as_str(),
            "state_kind": identity.state_kind.as_str(),
            "state_version": identity.state_version.as_str(),
        }),
        spec::DescriptorIdentity::Operation(identity) => serde_json::json!({
            "descriptor_family": "operation",
            "descriptor_id": identity.descriptor_id.as_str(),
            "name": identity.name.as_str(),
            "operation_kind": identity.operation_kind.as_str(),
            "operation_version": identity.operation_version.as_str(),
        }),
        spec::DescriptorIdentity::Renderer(identity) => serde_json::json!({
            "descriptor_family": "renderer",
            "descriptor_id": identity.descriptor_id.as_str(),
            "renderer_kind": identity.renderer_kind.as_str(),
            "renderer_version": identity.renderer_version.as_str(),
        }),
    }
}

fn descriptor_digest_json(identity: &spec::DescriptorIdentity) -> serde_json::Value {
    let descriptor_id = match identity {
        spec::DescriptorIdentity::State(identity) => &identity.descriptor_id,
        spec::DescriptorIdentity::Operation(identity) => &identity.descriptor_id,
        spec::DescriptorIdentity::Renderer(identity) => &identity.descriptor_id,
    };
    serde_json::json!({
        "descriptor_id": descriptor_id.as_str(),
        "digest": ContentDigest::from_digest(descriptor_id.algorithm(), *descriptor_id.digest()).as_str(),
    })
}

fn executable_json(identity: &events::ExecutableIdentity) -> serde_json::Value {
    serde_json::json!({
        "binary_digest": identity.binary_digest.as_str(),
        "cargo_package_digest": identity.cargo_package_digest.as_str(),
        "cargo_package_name": identity.cargo_package_name.as_str(),
        "cargo_package_version": identity.cargo_package_version.as_str(),
        "factory_id": identity.factory_id.as_str(),
        "nix_derivation_hash": identity.nix_derivation_hash.as_ref().map(events::NixDerivationHash::as_str),
        "nix_output_hash": identity.nix_output_hash.as_ref().map(events::NixOutputHash::as_str),
        "source_revision": identity.source_revision.as_str(),
    })
}

fn retention_ref_json(retention_ref: &events::RetentionRef) -> serde_json::Value {
    serde_json::json!({
        "artifact_id": retention_ref.artifact_id.as_str(),
        "content_digest": retention_ref.content_digest.as_str(),
        "role": retention_role_str(retention_ref.role),
    })
}

fn retention_role_str(role: events::ArtifactRole) -> &'static str {
    match role {
        events::ArtifactRole::TypedExecutionSpec => "typed_execution_spec",
        events::ArtifactRole::TypedSpecCertificate => "typed_spec_certificate",
        events::ArtifactRole::TypedConfig => "typed_config",
        events::ArtifactRole::SeedInput => "seed_input",
        events::ArtifactRole::StateOutput => "state_output",
        events::ArtifactRole::FactResponse => "fact_response",
        events::ArtifactRole::SideEffectIntent => "side_effect_intent",
        events::ArtifactRole::PreparedInvocation => "prepared_invocation",
        events::ArtifactRole::NotSubmittedProof => "not_submitted_proof",
        events::ArtifactRole::Submission => "submission",
        events::ArtifactRole::SubmissionUnknownEvidence => "submission_unknown_evidence",
        events::ArtifactRole::Receipt => "receipt",
        events::ArtifactRole::Confirmation => "confirmation",
        events::ArtifactRole::AmbiguityEvidence => "ambiguity_evidence",
        events::ArtifactRole::PublicOutput => "public_output",
        events::ArtifactRole::RedactedDiagnostic => "redacted_diagnostic",
        events::ArtifactRole::RetentionManifest => "retention_manifest",
    }
}

fn retention_reason_str(reason: events::RetentionReason) -> &'static str {
    match reason {
        events::RetentionReason::RunStarted => "run_started",
        events::RetentionReason::RuntimeEvidence => "runtime_evidence",
        events::RetentionReason::PublicOutput => "public_output",
        events::RetentionReason::ManifestProjection => "manifest_projection",
    }
}

fn cell_producer_json(producer: &spec::CellProducer) -> serde_json::Value {
    match producer {
        spec::CellProducer::Seed(seed_id) => serde_json::json!({
            "kind": "seed",
            "seed_id": seed_id.as_str(),
        }),
        spec::CellProducer::Node(node_id) => serde_json::json!({
            "kind": "node",
            "node_id": node_id.as_str(),
        }),
    }
}

/// Runtime capabilities minted for a node attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CertifiedRuntimeCapabilities {
    node_id: NodeId,
    descriptor: CapabilitySetDescriptor,
}

impl CertifiedRuntimeCapabilities {
    fn new(node_id: NodeId, descriptor: CapabilitySetDescriptor) -> Self {
        Self {
            node_id,
            descriptor,
        }
    }

    /// Node id these capabilities were minted for.
    pub fn node_id(&self) -> &NodeId {
        &self.node_id
    }

    /// Certified capability descriptors.
    pub fn descriptors(&self) -> &[CapabilityDescriptor] {
        &self.descriptor.capabilities
    }

    /// Returns true when the certified capability set contains this kind/version.
    pub fn contains(&self, kind: &CapabilityKind, version: &CapabilityVersion) -> bool {
        self.descriptor
            .capabilities
            .iter()
            .any(|capability| capability.kind == *kind && capability.version == *version)
    }
}

/// Materialized typed input tree for one node attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaterializedInputs {
    /// Certified input schema id.
    pub input_schema_id: SchemaId,
    /// Root materialized input node.
    pub root: MaterializedInputNode,
}

/// Materialized input tree node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MaterializedInputNode {
    /// Unit input.
    Unit,
    /// Certified cell input.
    Cell(Box<MaterializedCell>),
    /// Tuple input.
    Tuple(Vec<MaterializedInputNode>),
    /// Struct input.
    Struct(Vec<NamedMaterializedInput>),
    /// Vector input.
    Vec(Vec<MaterializedInputNode>),
    /// Non-empty vector input.
    NonEmptyVec(Vec<MaterializedInputNode>),
}

/// Named materialized struct field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamedMaterializedInput {
    /// Certified field path.
    pub field_path: spec::PublicFieldPath,
    /// Materialized field node.
    pub node: MaterializedInputNode,
}

/// Materialized certified cell evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaterializedCell {
    /// Cell id.
    pub cell_id: CellId,
    /// Schema id.
    pub schema_id: SchemaId,
    /// Semantic type id.
    pub semantic_type_id: mfm_ids::SemanticTypeId,
    /// Value lineage ref.
    pub value_lineage: spec::ValueLineageRef,
    /// Terminal evidence.
    pub terminal: MaterializedCellTerminal,
}

/// Materialized terminal cell evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MaterializedCellTerminal {
    /// Seed material from `RunStarted`.
    Seed {
        /// Seed id.
        seed_id: mfm_ids::SeedId,
        /// Artifact id.
        artifact_id: ArtifactId,
        /// Content digest.
        content_digest: ContentDigest,
    },
    /// Produced node output.
    Produced {
        /// Artifact id.
        artifact_id: ArtifactId,
        /// Content digest.
        content_digest: ContentDigest,
    },
    /// Skipped node output.
    Skipped {
        /// Skip reason.
        skip_reason: events::SkipReason,
    },
}

#[derive(Debug, Clone)]
struct RuntimeRunView {
    stream: Vec<store::KernelEventEnvelope>,
    projections: store::ProjectionSnapshot,
    seed_cells: BTreeMap<CellId, events::SeedCellRef>,
    config_artifacts: BTreeMap<String, store::ArtifactEvidenceRef>,
    artifact_refs: BTreeMap<ArtifactId, CommittedArtifactReference>,
    next_seq: store::StreamSeq,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CommittedArtifactReference {
    evidence: store::ArtifactEvidenceRef,
    attempt_id: Option<AttemptId>,
    commit_seq: store::StreamSeq,
    commit_key: store::CommitKey,
}

impl RuntimeRunView {
    fn from_store<S: store::TypedRunEventStore + ?Sized>(
        runtime_spec: &CertifiedRuntimeSpec,
        run_id: &RunId,
        store: &S,
    ) -> Result<Self> {
        let stream = store.load_run_stream(run_id);
        Self::from_stream(runtime_spec, run_id, &stream)
    }

    fn from_stream(
        runtime_spec: &CertifiedRuntimeSpec,
        run_id: &RunId,
        stream: &[store::KernelEventEnvelope],
    ) -> Result<Self> {
        store::ProjectionSnapshot::validate_run_stream(stream)?;
        let next_seq = next_seq_after_stream(stream)?;
        let projections = store::ProjectionSnapshot::rebuild_from_run_stream(stream)?;
        let mut run_started = None;
        for event in stream {
            match event.payload() {
                events::KernelEventPayload::RunStarted(payload) => {
                    if &payload.run_id != run_id {
                        return Err(RuntimeError::InvalidRunStream(format!(
                            "run stream contains RunStarted for {} while executing {}",
                            payload.run_id, run_id
                        )));
                    }
                    if &payload.spec_hash != runtime_spec.spec_hash() {
                        return Err(RuntimeError::InvalidRunStream(format!(
                            "RunStarted spec hash {} does not match certified {}",
                            payload.spec_hash,
                            runtime_spec.spec_hash()
                        )));
                    }
                    if run_started.replace(payload.clone()).is_some() {
                        return Err(RuntimeError::InvalidRunStream(
                            "run stream contains multiple RunStarted events".to_owned(),
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
        let Some(run_started) = run_started else {
            return Err(RuntimeError::InvalidRunStream(
                "run has not started with certified RunStarted evidence".to_owned(),
            ));
        };
        validate_historical_run_stream(runtime_spec, run_id, stream, &projections)?;
        let seed_cells = validate_seed_cells(runtime_spec, &run_started.seed_cells)?;
        let config_artifacts = config_artifacts_from_stream(runtime_spec, stream)?;
        let artifact_refs = artifact_refs_from_stream(stream)?;
        Ok(Self {
            stream: stream.to_vec(),
            projections,
            seed_cells,
            config_artifacts,
            artifact_refs,
            next_seq,
        })
    }
}

fn next_seq_after_stream(stream: &[store::KernelEventEnvelope]) -> Result<store::StreamSeq> {
    let Some(event) = stream.last() else {
        return Ok(store::StreamSeq::FIRST);
    };
    let next =
        event.seq().as_u64().checked_add(1).ok_or_else(|| {
            RuntimeError::InvalidRunStream("run stream sequence overflow".to_owned())
        })?;
    store::StreamSeq::new(next).map_err(RuntimeError::from)
}

/// Validates a stored typed run stream against the certified runtime spec without executing work.
///
/// This is the read-only counterpart to scheduler resume: callers that inspect, render, or
/// append-only resume a run must still prove the historical stream is bound to the stored certified
/// spec before trusting projections.
pub fn validate_run_stream(
    runtime_spec: &CertifiedRuntimeSpec,
    run_id: &RunId,
    stream: &[store::KernelEventEnvelope],
) -> Result<()> {
    RuntimeRunView::from_stream(runtime_spec, run_id, stream).map(|_| ())
}

/// Evidence needed to append `RunStarted`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunStartEvidence {
    /// Artifact evidence containing the certified spec bytes.
    pub spec_artifact: store::ArtifactEvidenceRef,
    /// Artifact evidence containing the certified spec certificate bytes.
    pub certificate_artifact: store::ArtifactEvidenceRef,
    /// Artifact evidence for every certified config reference.
    pub config_artifacts: Vec<store::ArtifactEvidenceRef>,
    /// Framework build/version identity.
    pub framework_version: events::FrameworkVersion,
    /// Source revision identity.
    pub source_revision: events::SourceRevision,
    /// Adapter executable identities bound to the run.
    pub adapter_executables: Vec<events::ExecutableIdentity>,
    /// Seed cells materialized at run start.
    pub seed_cells: Vec<events::SeedCellRef>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RetentionManifestArtifact {
    bytes: PlainCanonicalJsonBytes,
    evidence: store::ArtifactEvidenceRef,
    manifest_seq: u64,
    previous_manifest_digest: Option<ContentDigest>,
}

/// Result of one serial scheduler drive call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchedulerStatus {
    /// At least one node attempt ran and committed.
    Advanced,
    /// No node is currently runnable.
    Blocked,
    /// Public output has already been projected.
    PublicOutputProjected,
}

struct RunnableNode<'a> {
    node: &'a spec::NodeSpec,
    attempt: AttemptPlan,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum AttemptPlan {
    StartNew,
    Continue {
        attempt_id: AttemptId,
        attempt_no: u32,
    },
}

struct RunnerOutputCommitInput<'a> {
    runtime_spec: &'a CertifiedRuntimeSpec,
    run_id: &'a RunId,
    node: &'a spec::NodeSpec,
    attempt_id: &'a AttemptId,
    caps: &'a CertifiedRuntimeCapabilities,
    recorded_facts: &'a RecordedFacts,
    view: &'a RuntimeRunView,
    output: ErasedRunnerOutput,
}

struct PreparedRunnerOutput {
    commit: store::PreparedTypedCommit,
    artifacts_to_stage: Vec<PreparedStagedArtifact>,
}

struct PreparedStagedArtifact {
    bytes: Vec<u8>,
    evidence: store::ArtifactEvidenceRef,
}

fn prepare_runner_invocation<'a>(
    runtime_spec: &'a CertifiedRuntimeSpec,
    run_id: &'a RunId,
    node: &'a spec::NodeSpec,
    descriptor: &'a spec::StateDescriptorIdentity,
    output_cell: &'a spec::CellSpec,
    attempt_id: &'a AttemptId,
    attempt_no: u32,
    view: &'a RuntimeRunView,
) -> Result<PreparedRunnerInvocation<'a>> {
    let config_artifact = committed_config_artifact(node, view)?;
    let inputs = materialize_inputs(runtime_spec, node, view)?;
    let caps =
        CertifiedRuntimeCapabilities::new(node.node_id.clone(), node.capability_bindings.clone());
    let recorded_facts = recorded_facts_for_attempt(&view.projections, &node.node_id, attempt_id)?;
    Ok(PreparedRunnerInvocation {
        runtime_spec,
        run_id,
        spec_hash: runtime_spec.spec_hash(),
        node,
        descriptor,
        output_cell,
        attempt_id,
        attempt_no,
        config_artifact,
        inputs,
        caps,
        recorded_facts,
        projections: &view.projections,
        run_stream: &view.stream,
    })
}

struct RuntimeMutationMiddleware;

impl RuntimeMutationMiddleware {
    fn prepare_run_start(
        runners: &ErasedRunnerRegistry,
        runtime_spec: &CertifiedRuntimeSpec,
        run_id: RunId,
        evidence: RunStartEvidence,
        expected_next_seq: store::StreamSeq,
    ) -> Result<store::PreparedTypedCommit> {
        let spec_artifact = validate_spec_artifact(runtime_spec, evidence.spec_artifact)?;
        let certificate_artifact =
            validate_certificate_artifact(runtime_spec, evidence.certificate_artifact)?;
        let config_artifacts = validate_config_artifacts(runtime_spec, evidence.config_artifacts)?;
        let config_reference_payloads =
            config_artifact_reference_payloads(runtime_spec.spec_hash(), &config_artifacts)?;
        let seed_cells = validate_seed_cells(runtime_spec, &evidence.seed_cells)?;
        let runner_executables = runners.executables_for_spec(runtime_spec)?;
        let mut required_artifacts =
            Vec::with_capacity(2 + config_artifacts.len() + seed_cells.len());
        required_artifacts.push(spec_artifact.clone());
        required_artifacts.push(certificate_artifact.clone());
        required_artifacts.extend(config_artifacts);
        required_artifacts.extend(seed_cells.values().map(store_seed_artifact));
        let admitted_artifacts = required_artifacts.clone();
        let run_started_retention_refs = required_artifacts
            .iter()
            .map(retention_ref_for_artifact)
            .collect::<Vec<_>>();
        let payload = events::KernelEventPayload::RunStarted(events::RunStarted {
            run_id: run_id.clone(),
            spec_hash: runtime_spec.spec_hash().clone(),
            spec_artifact_id: spec_artifact.artifact_id,
            certificate_artifact_id: certificate_artifact.artifact_id,
            certificate_artifact_digest: certificate_artifact.digest,
            certificate_media_type: certificate_artifact.media_type,
            spec_media_type: runtime_spec.spec().media_type.clone(),
            spec_version: runtime_spec.spec().spec_version.clone(),
            lowering_version: runtime_spec.spec().lowering_version.clone(),
            public_output_schema_id: runtime_spec.spec().public_outputs.public_schema_id.clone(),
            descriptor_identities: runtime_spec.spec().descriptor_identities.clone(),
            runner_executables,
            adapter_executables: evidence.adapter_executables,
            canonicalizer_identity: runtime_spec
                .spec()
                .public_outputs
                .renderer_descriptor
                .canonicalizer_identity
                .clone(),
            framework_version: evidence.framework_version,
            source_revision: evidence.source_revision,
            seed_cells: evidence.seed_cells,
        });
        let retention_payload =
            events::KernelEventPayload::RetentionRefsAppended(events::RetentionRefsAppended {
                run_id: run_id.clone(),
                spec_hash: runtime_spec.spec_hash().clone(),
                refs: run_started_retention_refs,
                reason: events::RetentionReason::RunStarted,
            });
        let mut payloads = Vec::with_capacity(2 + config_reference_payloads.len());
        payloads.push(payload);
        payloads.extend(config_reference_payloads);
        payloads.push(retention_payload);
        let request = store::TypedCommitRequest {
            run_id,
            expected_next_seq,
            commit_key: store::CommitKey::new(format!(
                "run-start:{}",
                runtime_spec.spec_hash().as_str()
            ))?,
            payloads,
            required_artifacts,
            preconditions: store::CommitPreconditions {
                required_run_state: store::RequiredRunState::Absent,
                ..store::CommitPreconditions::default()
            },
        };
        store::PreparedTypedCommit::new(request, admitted_artifacts).map_err(RuntimeError::from)
    }

    fn prepare_attempt_start(
        runtime_spec: &CertifiedRuntimeSpec,
        run_id: &RunId,
        node: &spec::NodeSpec,
        attempt_id: &AttemptId,
        attempt_no: u32,
        view: &RuntimeRunView,
    ) -> Result<store::PreparedTypedCommit> {
        let start_payload =
            events::KernelEventPayload::StateAttemptStarted(events::StateAttemptStarted {
                spec_hash: runtime_spec.spec_hash().clone(),
                node_id: node.node_id.clone(),
                attempt_id: attempt_id.clone(),
                attempt_no,
                state_kind: node.state_kind.clone(),
                state_version: node.state_version.clone(),
            });
        let mut preconditions = store::CommitPreconditions {
            required_run_state: store::RequiredRunState::NotCompleted,
            required_cell_states: vec![store::CellStatePrecondition {
                cell_id: node.output_cell.clone(),
                required: store::RequiredCellState::Absent,
            }],
            ..store::CommitPreconditions::default()
        };
        preconditions
            .required_cell_states
            .extend(node_cell_preconditions(runtime_spec, node)?);
        let request = store::TypedCommitRequest {
            run_id: run_id.clone(),
            expected_next_seq: view.next_seq,
            commit_key: store::CommitKey::new(format!(
                "attempt-start:{}:{}",
                node.node_id, attempt_id
            ))?,
            payloads: vec![start_payload],
            required_artifacts: Vec::new(),
            preconditions,
        };
        store::PreparedTypedCommit::new(request, Vec::new()).map_err(RuntimeError::from)
    }

    fn prepare_runner_output(input: RunnerOutputCommitInput<'_>) -> Result<PreparedRunnerOutput> {
        Self::prepare_runner_output_with_start(input, None)
    }

    fn prepare_started_runner_output(
        input: RunnerOutputCommitInput<'_>,
        attempt_no: u32,
    ) -> Result<PreparedRunnerOutput> {
        Self::prepare_runner_output_with_start(input, Some(attempt_no))
    }

    fn prepare_runner_output_with_start(
        input: RunnerOutputCommitInput<'_>,
        started_in_same_commit: Option<u32>,
    ) -> Result<PreparedRunnerOutput> {
        validate_runner_output(
            input.runtime_spec,
            input.node,
            input.attempt_id,
            input.caps,
            input.recorded_facts,
            &input.view.projections,
            &input.output,
        )?;
        let ErasedRunnerOutput {
            staged_artifacts,
            staged_retention_refs,
            payloads: runner_payloads,
        } = input.output;
        if matches!(
            &input.node.framework,
            Some(spec::FrameworkNodeSpec::CompleteRun(_))
        ) && !staged_retention_refs.is_empty()
        {
            return Err(RuntimeError::InvalidRunnerOutput(format!(
                "complete-run framework node {} cannot stage retention refs after manifest projection",
                input.node.node_id
            )));
        }
        let staged_artifacts = validate_staged_artifacts(
            input.run_id,
            input.node,
            input.attempt_id,
            &staged_artifacts,
        )?;
        let retention_manifest = framework_retention_manifest_artifact(
            input.runtime_spec,
            input.run_id,
            input.node,
            &input.view.stream,
            &staged_artifacts,
        )?;
        let payload_bound_artifacts = staged_artifacts
            .iter()
            .filter(|artifact| artifact.binding != StagedArtifactBindingKind::RetentionManifest)
            .cloned()
            .collect::<Vec<_>>();
        validate_staged_artifact_payload_bindings(
            input.node,
            input.attempt_id,
            &runner_payloads,
            &payload_bound_artifacts,
        )?;
        let mut payloads = if let Some(attempt_no) = started_in_same_commit {
            vec![events::KernelEventPayload::StateAttemptStarted(
                events::StateAttemptStarted {
                    spec_hash: input.runtime_spec.spec_hash().clone(),
                    node_id: input.node.node_id.clone(),
                    attempt_id: input.attempt_id.clone(),
                    attempt_no,
                    state_kind: input.node.state_kind.clone(),
                    state_version: input.node.state_version.clone(),
                },
            )]
        } else {
            Vec::new()
        };
        payloads.extend(runner_payloads);
        if let Some(manifest) = retention_manifest {
            payloads.extend(retention_manifest_payloads(
                input.runtime_spec,
                input.run_id,
                manifest,
            ));
        }
        payloads.extend(staged_artifact_reference_payloads(
            input.runtime_spec.spec_hash(),
            input.node,
            input.attempt_id,
            &payloads,
            &payload_bound_artifacts,
        ));
        let artifacts_to_stage = staged_artifacts
            .iter()
            .filter_map(|artifact| {
                artifact.bytes.as_ref().map(|bytes| PreparedStagedArtifact {
                    bytes: bytes.clone(),
                    evidence: artifact.evidence.clone(),
                })
            })
            .collect::<Vec<_>>();
        let required_artifacts = staged_artifacts
            .iter()
            .map(|artifact| artifact.evidence.clone())
            .collect::<Vec<_>>();
        payloads.extend(bind_staged_retention_refs(
            input.runtime_spec,
            input.run_id,
            input.node,
            &required_artifacts,
            staged_retention_refs,
        )?);
        if let Some(run_completed) = framework_run_completed_payload(
            input.runtime_spec,
            input.run_id,
            input.node,
            &input.view.projections,
        )? {
            payloads.push(run_completed);
        }
        let admitted_artifacts = required_artifacts.clone();
        let preconditions = runner_output_preconditions(
            input.runtime_spec,
            input.run_id,
            input.node,
            input.attempt_id,
            &input.view.projections,
            &payloads,
            started_in_same_commit.is_none(),
        )?;
        let request = store::TypedCommitRequest {
            run_id: input.run_id.clone(),
            expected_next_seq: input.view.next_seq,
            commit_key: runner_output_commit_key(input.node, input.attempt_id, &payloads)?,
            payloads,
            required_artifacts,
            preconditions,
        };
        let commit = store::PreparedTypedCommit::new(request, admitted_artifacts)?;
        Ok(PreparedRunnerOutput {
            commit,
            artifacts_to_stage,
        })
    }
}

/// Serial typed scheduler.
#[derive(Clone)]
pub struct SerialTypedScheduler {
    runners: ErasedRunnerRegistry,
    artifact_stager: Arc<dyn RuntimeArtifactStager>,
}

impl SerialTypedScheduler {
    /// Creates a scheduler using a certified runner registry and runtime artifact stager.
    pub fn new(
        runners: ErasedRunnerRegistry,
        artifact_stager: Arc<dyn RuntimeArtifactStager>,
    ) -> Self {
        Self {
            runners,
            artifact_stager,
        }
    }

    /// Appends the typed `RunStarted` event after validating seed and runner executable evidence.
    pub fn start_run<S: store::TypedRunEventStore + ?Sized>(
        &self,
        store: &mut S,
        runtime_spec: &CertifiedRuntimeSpec,
        run_id: RunId,
        evidence: RunStartEvidence,
    ) -> Result<store::CommitOutcome> {
        let expected_next_seq = store.expected_next_seq(&run_id);
        let commit = RuntimeMutationMiddleware::prepare_run_start(
            &self.runners,
            runtime_spec,
            run_id,
            evidence,
            expected_next_seq,
        )?;
        Ok(store.append_prepared_typed_commit(commit)?)
    }

    /// Appends the typed `RunStarted` event through an async typed store.
    pub async fn start_run_async<S: store::AsyncTypedRunEventStore + ?Sized>(
        &self,
        store: &S,
        runtime_spec: &CertifiedRuntimeSpec,
        run_id: RunId,
        evidence: RunStartEvidence,
    ) -> Result<store::CommitOutcome> {
        let expected_next_seq = store
            .expected_next_seq(&run_id)
            .await
            .map_err(async_store_error)?;
        let commit = RuntimeMutationMiddleware::prepare_run_start(
            &self.runners,
            runtime_spec,
            run_id,
            evidence,
            expected_next_seq,
        )?;
        store
            .append_prepared_typed_commit(commit)
            .await
            .map_err(async_store_error)
    }

    /// Runs one deterministic runnable node, if any.
    pub async fn drive_once<S: store::TypedRunEventStore + ?Sized>(
        &self,
        store: &mut S,
        runtime_spec: &CertifiedRuntimeSpec,
        run_id: &RunId,
    ) -> Result<SchedulerStatus> {
        let view = RuntimeRunView::from_store(runtime_spec, run_id, store)?;
        if view.projections.run_state(run_id) == store::RunState::Completed {
            return Ok(SchedulerStatus::PublicOutputProjected);
        }
        if public_output_is_produced(runtime_spec, &view.projections) {
            if let Some(runnable) = next_runnable_node(runtime_spec, &view)? {
                self.run_node_attempt(store, runtime_spec, run_id, &view, runnable)
                    .await?;
                return Ok(SchedulerStatus::Advanced);
            }
            return Ok(SchedulerStatus::PublicOutputProjected);
        }
        let Some(runnable) = next_runnable_node(runtime_spec, &view)? else {
            return Ok(SchedulerStatus::Blocked);
        };
        self.run_node_attempt(store, runtime_spec, run_id, &view, runnable)
            .await?;
        Ok(SchedulerStatus::Advanced)
    }

    /// Runs deterministic runnable nodes until no node is runnable or public output is projected.
    pub async fn drive_until_blocked<S: store::TypedRunEventStore + ?Sized>(
        &self,
        store: &mut S,
        runtime_spec: &CertifiedRuntimeSpec,
        run_id: &RunId,
    ) -> Result<SchedulerStatus> {
        let mut advanced = false;
        loop {
            match self.drive_once(store, runtime_spec, run_id).await? {
                SchedulerStatus::Advanced => advanced = true,
                SchedulerStatus::Blocked if advanced => return Ok(SchedulerStatus::Advanced),
                status => return Ok(status),
            }
        }
    }

    /// Runs one deterministic runnable node against an async durable typed store, if any.
    pub async fn drive_once_async<S: store::AsyncTypedRunEventStore + ?Sized>(
        &self,
        store: &S,
        runtime_spec: &CertifiedRuntimeSpec,
        run_id: &RunId,
    ) -> Result<SchedulerStatus> {
        let stream = store
            .load_run_stream(run_id)
            .await
            .map_err(async_store_error)?;
        let view = RuntimeRunView::from_stream(runtime_spec, run_id, &stream)?;
        if view.projections.run_state(run_id) == store::RunState::Completed {
            return Ok(SchedulerStatus::PublicOutputProjected);
        }
        if public_output_is_produced(runtime_spec, &view.projections) {
            if let Some(runnable) = next_runnable_node(runtime_spec, &view)? {
                self.run_node_attempt_async(store, runtime_spec, run_id, &view, runnable)
                    .await?;
                return Ok(SchedulerStatus::Advanced);
            }
            return Ok(SchedulerStatus::PublicOutputProjected);
        }
        let Some(runnable) = next_runnable_node(runtime_spec, &view)? else {
            return Ok(SchedulerStatus::Blocked);
        };
        self.run_node_attempt_async(store, runtime_spec, run_id, &view, runnable)
            .await?;
        Ok(SchedulerStatus::Advanced)
    }

    /// Runs deterministic runnable nodes against an async durable typed store until blocked.
    pub async fn drive_until_blocked_async<S: store::AsyncTypedRunEventStore + ?Sized>(
        &self,
        store: &S,
        runtime_spec: &CertifiedRuntimeSpec,
        run_id: &RunId,
    ) -> Result<SchedulerStatus> {
        let mut advanced = false;
        loop {
            match self.drive_once_async(store, runtime_spec, run_id).await? {
                SchedulerStatus::Advanced => advanced = true,
                SchedulerStatus::Blocked if advanced => return Ok(SchedulerStatus::Advanced),
                status => return Ok(status),
            }
        }
    }

    async fn run_node_attempt<S: store::TypedRunEventStore + ?Sized>(
        &self,
        store: &mut S,
        runtime_spec: &CertifiedRuntimeSpec,
        run_id: &RunId,
        view: &RuntimeRunView,
        runnable: RunnableNode<'_>,
    ) -> Result<()> {
        let node = runnable.node;
        let descriptor = runtime_spec.state_descriptor_for_node(node)?;
        let output_cell = runtime_spec.cell(&node.output_cell).ok_or_else(|| {
            RuntimeError::InvalidSpec(format!(
                "node {} output cell {} is missing",
                node.node_id, node.output_cell
            ))
        })?;
        let binding = self.runners.resolve(node, descriptor)?;
        if matches!(
            &node.framework,
            Some(
                spec::FrameworkNodeSpec::ProjectRetentionManifest(_)
                    | spec::FrameworkNodeSpec::CompleteRun(_)
            )
        ) {
            self.run_started_framework_node_attempt(
                store,
                runtime_spec,
                run_id,
                view,
                runnable,
                descriptor,
                output_cell,
                binding,
            )
            .await?;
            return Ok(());
        }
        let (attempt_id, attempt_no) = match runnable.attempt {
            AttemptPlan::StartNew => {
                let attempt_no = next_attempt_no(&view.projections, &node.node_id)?;
                let attempt_id =
                    attempt_id(run_id, runtime_spec.spec_hash(), &node.node_id, attempt_no)?;
                prepare_runner_invocation(
                    runtime_spec,
                    run_id,
                    node,
                    descriptor,
                    output_cell,
                    &attempt_id,
                    attempt_no,
                    view,
                )?;
                let start_commit = RuntimeMutationMiddleware::prepare_attempt_start(
                    runtime_spec,
                    run_id,
                    node,
                    &attempt_id,
                    attempt_no,
                    view,
                )?;
                store.append_prepared_typed_commit(start_commit)?;
                (attempt_id, attempt_no)
            }
            AttemptPlan::Continue {
                attempt_id,
                attempt_no,
            } => (attempt_id, attempt_no),
        };

        let latest_stream = store.load_run_stream(run_id);
        let latest_view = RuntimeRunView::from_stream(runtime_spec, run_id, &latest_stream)?;
        let invocation = prepare_runner_invocation(
            runtime_spec,
            run_id,
            node,
            descriptor,
            output_cell,
            &attempt_id,
            attempt_no,
            &latest_view,
        )?;
        let output = binding
            .runner
            .run_erased(ErasedRunCtx::from_prepared(&invocation))
            .await?;
        let terminal_output =
            RuntimeMutationMiddleware::prepare_runner_output(RunnerOutputCommitInput {
                runtime_spec,
                run_id,
                node,
                attempt_id: &attempt_id,
                caps: invocation.caps(),
                recorded_facts: invocation.recorded_facts(),
                view: &latest_view,
                output,
            })?;
        self.stage_prepared_artifacts(&terminal_output.artifacts_to_stage)
            .await?;
        store.append_prepared_typed_commit(terminal_output.commit)?;
        Ok(())
    }

    async fn run_started_framework_node_attempt<S: store::TypedRunEventStore + ?Sized>(
        &self,
        store: &mut S,
        runtime_spec: &CertifiedRuntimeSpec,
        run_id: &RunId,
        view: &RuntimeRunView,
        runnable: RunnableNode<'_>,
        descriptor: &spec::StateDescriptorIdentity,
        output_cell: &spec::CellSpec,
        binding: ErasedRunnerBinding,
    ) -> Result<()> {
        let node = runnable.node;
        let AttemptPlan::StartNew = runnable.attempt else {
            return Err(RuntimeError::InvalidRunStream(format!(
                "framework lifecycle node {} attempt was split across commits",
                node.node_id
            )));
        };
        let attempt_no = next_attempt_no(&view.projections, &node.node_id)?;
        let attempt_id = attempt_id(run_id, runtime_spec.spec_hash(), &node.node_id, attempt_no)?;
        let invocation = prepare_runner_invocation(
            runtime_spec,
            run_id,
            node,
            descriptor,
            output_cell,
            &attempt_id,
            attempt_no,
            view,
        )?;
        let output = binding
            .runner
            .run_erased(ErasedRunCtx::from_prepared(&invocation))
            .await?;
        let terminal_output = RuntimeMutationMiddleware::prepare_started_runner_output(
            RunnerOutputCommitInput {
                runtime_spec,
                run_id,
                node,
                attempt_id: &attempt_id,
                caps: invocation.caps(),
                recorded_facts: invocation.recorded_facts(),
                view,
                output,
            },
            attempt_no,
        )?;
        self.stage_prepared_artifacts(&terminal_output.artifacts_to_stage)
            .await?;
        store.append_prepared_typed_commit(terminal_output.commit)?;
        Ok(())
    }

    async fn run_node_attempt_async<S: store::AsyncTypedRunEventStore + ?Sized>(
        &self,
        store: &S,
        runtime_spec: &CertifiedRuntimeSpec,
        run_id: &RunId,
        view: &RuntimeRunView,
        runnable: RunnableNode<'_>,
    ) -> Result<()> {
        let node = runnable.node;
        let descriptor = runtime_spec.state_descriptor_for_node(node)?;
        let output_cell = runtime_spec.cell(&node.output_cell).ok_or_else(|| {
            RuntimeError::InvalidSpec(format!(
                "node {} output cell {} is missing",
                node.node_id, node.output_cell
            ))
        })?;
        let binding = self.runners.resolve(node, descriptor)?;
        if matches!(
            &node.framework,
            Some(
                spec::FrameworkNodeSpec::ProjectRetentionManifest(_)
                    | spec::FrameworkNodeSpec::CompleteRun(_)
            )
        ) {
            self.run_started_framework_node_attempt_async(
                store,
                runtime_spec,
                run_id,
                view,
                runnable,
                descriptor,
                output_cell,
                binding,
            )
            .await?;
            return Ok(());
        }
        let (attempt_id, attempt_no) = match runnable.attempt {
            AttemptPlan::StartNew => {
                let attempt_no = next_attempt_no(&view.projections, &node.node_id)?;
                let attempt_id =
                    attempt_id(run_id, runtime_spec.spec_hash(), &node.node_id, attempt_no)?;
                prepare_runner_invocation(
                    runtime_spec,
                    run_id,
                    node,
                    descriptor,
                    output_cell,
                    &attempt_id,
                    attempt_no,
                    view,
                )?;
                let start_commit = RuntimeMutationMiddleware::prepare_attempt_start(
                    runtime_spec,
                    run_id,
                    node,
                    &attempt_id,
                    attempt_no,
                    view,
                )?;
                store
                    .append_prepared_typed_commit(start_commit)
                    .await
                    .map_err(async_store_error)?;
                (attempt_id, attempt_no)
            }
            AttemptPlan::Continue {
                attempt_id,
                attempt_no,
            } => (attempt_id, attempt_no),
        };

        let latest_stream = store
            .load_run_stream(run_id)
            .await
            .map_err(async_store_error)?;
        let latest_view = RuntimeRunView::from_stream(runtime_spec, run_id, &latest_stream)?;
        let invocation = prepare_runner_invocation(
            runtime_spec,
            run_id,
            node,
            descriptor,
            output_cell,
            &attempt_id,
            attempt_no,
            &latest_view,
        )?;
        let output = binding
            .runner
            .run_erased(ErasedRunCtx::from_prepared(&invocation))
            .await?;
        let terminal_output =
            RuntimeMutationMiddleware::prepare_runner_output(RunnerOutputCommitInput {
                runtime_spec,
                run_id,
                node,
                attempt_id: &attempt_id,
                caps: invocation.caps(),
                recorded_facts: invocation.recorded_facts(),
                view: &latest_view,
                output,
            })?;
        self.stage_prepared_artifacts(&terminal_output.artifacts_to_stage)
            .await?;
        store
            .append_prepared_typed_commit(terminal_output.commit)
            .await
            .map_err(async_store_error)?;
        Ok(())
    }

    async fn run_started_framework_node_attempt_async<
        S: store::AsyncTypedRunEventStore + ?Sized,
    >(
        &self,
        store: &S,
        runtime_spec: &CertifiedRuntimeSpec,
        run_id: &RunId,
        view: &RuntimeRunView,
        runnable: RunnableNode<'_>,
        descriptor: &spec::StateDescriptorIdentity,
        output_cell: &spec::CellSpec,
        binding: ErasedRunnerBinding,
    ) -> Result<()> {
        let node = runnable.node;
        let AttemptPlan::StartNew = runnable.attempt else {
            return Err(RuntimeError::InvalidRunStream(format!(
                "framework lifecycle node {} attempt was split across commits",
                node.node_id
            )));
        };
        let attempt_no = next_attempt_no(&view.projections, &node.node_id)?;
        let attempt_id = attempt_id(run_id, runtime_spec.spec_hash(), &node.node_id, attempt_no)?;
        let invocation = prepare_runner_invocation(
            runtime_spec,
            run_id,
            node,
            descriptor,
            output_cell,
            &attempt_id,
            attempt_no,
            view,
        )?;
        let output = binding
            .runner
            .run_erased(ErasedRunCtx::from_prepared(&invocation))
            .await?;
        let terminal_output = RuntimeMutationMiddleware::prepare_started_runner_output(
            RunnerOutputCommitInput {
                runtime_spec,
                run_id,
                node,
                attempt_id: &attempt_id,
                caps: invocation.caps(),
                recorded_facts: invocation.recorded_facts(),
                view,
                output,
            },
            attempt_no,
        )?;
        self.stage_prepared_artifacts(&terminal_output.artifacts_to_stage)
            .await?;
        store
            .append_prepared_typed_commit(terminal_output.commit)
            .await
            .map_err(async_store_error)?;
        Ok(())
    }

    async fn stage_prepared_artifacts(&self, artifacts: &[PreparedStagedArtifact]) -> Result<()> {
        for artifact in artifacts {
            self.artifact_stager
                .stage_verified_artifact(artifact.bytes.clone(), artifact.evidence.clone())
                .await?;
        }
        Ok(())
    }
}

fn public_output_is_produced(
    runtime_spec: &CertifiedRuntimeSpec,
    projections: &store::ProjectionSnapshot,
) -> bool {
    let public_schema_id = &runtime_spec.spec().public_outputs.public_schema_id;
    matches!(
        projections.public_output(public_schema_id),
        Some(store::PublicOutputProjection::Produced { .. })
    )
}

fn build_retention_manifest_artifact_with_producer(
    runtime_spec: &CertifiedRuntimeSpec,
    run_id: &RunId,
    stream: &[store::KernelEventEnvelope],
    producer_node_id: Option<NodeId>,
) -> Result<RetentionManifestArtifact> {
    store::ProjectionSnapshot::validate_run_stream(stream)?;
    let projection = store::ProjectionSnapshot::rebuild_from_run_stream(stream)?;
    let run_started = stream
        .iter()
        .find_map(|event| match event.payload() {
            events::KernelEventPayload::RunStarted(payload) => Some(payload),
            _ => None,
        })
        .ok_or_else(|| {
            RuntimeError::InvalidRunStream(
                "retention manifest requires RunStarted evidence".to_owned(),
            )
        })?;
    if &run_started.run_id != run_id || &run_started.spec_hash != runtime_spec.spec_hash() {
        return Err(RuntimeError::InvalidRunStream(
            "retention manifest run-start evidence does not match certified run".to_owned(),
        ));
    }
    let retention = projection.retention(run_id).cloned().unwrap_or_default();
    let manifest_seq = retention
        .manifest
        .as_ref()
        .map(|manifest| {
            manifest.manifest_seq.checked_add(1).ok_or_else(|| {
                RuntimeError::InvalidRunStream("retention manifest sequence overflow".to_owned())
            })
        })
        .transpose()?
        .unwrap_or(1);
    let previous_manifest_digest = retention
        .manifest
        .as_ref()
        .map(|manifest| manifest.manifest_digest.clone());
    let retained_refs = retention.refs.values().collect::<Vec<_>>();
    let manifest_json = retention_manifest_json(
        runtime_spec,
        run_id,
        run_started,
        manifest_seq,
        previous_manifest_digest.as_ref(),
        &retained_refs,
        stream,
    )?;
    let digest = manifest_json.content_digest();
    let artifact_id = ArtifactId::from_digest(digest.algorithm(), *digest.digest());
    let byte_len = manifest_json.as_bytes().len() as u64;
    Ok(RetentionManifestArtifact {
        bytes: manifest_json,
        evidence: store::ArtifactEvidenceRef {
            artifact_id,
            digest,
            byte_len,
            media_type: spec::MediaType::new(
                "application/vnd.mfm.retention-manifest+json;version=1",
            )?,
            schema_id: None,
            semantic_type_id: None,
            producer_node_id,
            producer_seed_id: None,
            artifact_role: events::ArtifactRole::RetentionManifest,
        },
        manifest_seq,
        previous_manifest_digest,
    })
}

fn runner_output_commit_key(
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    payloads: &[events::KernelEventPayload],
) -> Result<store::CommitKey> {
    let mut fragments = BTreeSet::new();
    for payload in payloads {
        fragments.insert(runner_output_commit_fragment(payload));
    }
    if fragments.is_empty() {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "runner for node {} returned no typed payloads",
            node.node_id
        )));
    }
    let suffix = content_digest_json(serde_json::json!({
        "fragments": fragments.into_iter().collect::<Vec<_>>(),
    }))?;
    Ok(store::CommitKey::new(format!(
        "attempt-output:{}:{}:{}",
        node.node_id, attempt_id, suffix
    ))?)
}

fn runner_output_commit_fragment(payload: &events::KernelEventPayload) -> String {
    match payload {
        events::KernelEventPayload::StateAttemptCompleted(payload) => {
            format!("completed:{}", payload.output_cell_id)
        }
        events::KernelEventPayload::StateAttemptFailed(_) => "failed".to_owned(),
        events::KernelEventPayload::CellProduced(payload) => {
            format!("cell-produced:{}", payload.cell_id)
        }
        events::KernelEventPayload::CellSkipped(payload) => {
            format!("cell-skipped:{}", payload.cell_id)
        }
        events::KernelEventPayload::FactRecorded(payload) => format!("fact:{}", payload.fact_key),
        events::KernelEventPayload::ArtifactReferenced(payload) => {
            format!("artifact:{}", payload.artifact_ref.artifact_id)
        }
        events::KernelEventPayload::PublicOutputProduced(payload) => {
            format!("public-output:{}", payload.public_schema_id)
        }
        events::KernelEventPayload::PublicOutputRenderFailed(payload) => {
            format!("public-output-failed:{}", payload.public_schema_id)
        }
        events::KernelEventPayload::SideEffectIntentPersisted(payload) => {
            format!("sidefx-intent:{}", payload.ledger_key)
        }
        events::KernelEventPayload::SideEffectClaimed(payload) => format!(
            "sidefx-claim:{}:{}:{}",
            payload.ledger_key, payload.invocation_epoch, payload.claim_generation
        ),
        events::KernelEventPayload::SideEffectClaimTakenOver(payload) => format!(
            "sidefx-claim-takeover:{}:{}:{}",
            payload.ledger_key, payload.invocation_epoch, payload.claim_generation
        ),
        events::KernelEventPayload::SideEffectInvocationPrepared(payload) => format!(
            "sidefx-prepared:{}:{}",
            payload.ledger_key, payload.invocation_epoch
        ),
        events::KernelEventPayload::SideEffectInvocationStarted(payload) => format!(
            "sidefx-started:{}:{}",
            payload.ledger_key, payload.invocation_epoch
        ),
        events::KernelEventPayload::SideEffectNotSubmittedProven(payload) => format!(
            "sidefx-not-submitted:{}:{}",
            payload.ledger_key, payload.invocation_epoch
        ),
        events::KernelEventPayload::SideEffectSubmissionObserved(payload) => format!(
            "sidefx-submission:{}:{}",
            payload.ledger_key, payload.invocation_epoch
        ),
        events::KernelEventPayload::SideEffectSubmissionUnknown(payload) => format!(
            "sidefx-submission-unknown:{}:{}",
            payload.ledger_key, payload.invocation_epoch
        ),
        events::KernelEventPayload::SideEffectReceiptObserved(payload) => format!(
            "sidefx-receipt:{}:{}",
            payload.ledger_key, payload.invocation_epoch
        ),
        events::KernelEventPayload::SideEffectConfirmationObserved(payload) => format!(
            "sidefx-confirmation:{}:{}",
            payload.ledger_key, payload.invocation_epoch
        ),
        events::KernelEventPayload::SideEffectAmbiguous(payload) => {
            format!("sidefx-ambiguous:{}", payload.ledger_key)
        }
        events::KernelEventPayload::SideEffectFailed(payload) => format!(
            "sidefx-failed:{}:{}",
            payload.ledger_key, payload.invocation_epoch
        ),
        events::KernelEventPayload::RetentionManifestProjected(payload) => {
            format!(
                "retention-manifest:{}:{}",
                payload.manifest_seq, payload.manifest_digest
            )
        }
        events::KernelEventPayload::RetentionRefsAppended(payload) => format!(
            "retention-refs:{}:{}",
            retention_reason_str(payload.reason),
            payload.refs.len()
        ),
        events::KernelEventPayload::RunStarted(_)
        | events::KernelEventPayload::RunCompleted(_)
        | events::KernelEventPayload::StateAttemptStarted(_) => "scheduler-owned".to_owned(),
    }
}

fn validate_staged_artifacts(
    run_id: &RunId,
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    staged_artifacts: &[StagedArtifact],
) -> Result<Vec<ValidatedStagedArtifact>> {
    let mut by_artifact = BTreeMap::<ArtifactId, ValidatedStagedArtifact>::new();
    for staged in staged_artifacts {
        let handle = staged.handle();
        if handle.run_id() != run_id
            || handle.node_id() != &node.node_id
            || handle.attempt_id() != attempt_id
        {
            return Err(RuntimeError::InvalidRunnerOutput(format!(
                "node {} staged artifact {} outside its sealed attempt",
                node.node_id,
                handle.evidence().artifact_id
            )));
        }
        if let Some(bytes) = staged.bytes() {
            verify_artifact_bytes(bytes, handle.evidence())?;
        }
        if let Some(existing) = by_artifact.get(&handle.evidence().artifact_id) {
            if existing.evidence != *handle.evidence() || existing.binding != *handle.binding() {
                return Err(RuntimeError::InvalidRunnerOutput(format!(
                    "node {} staged conflicting evidence for artifact {}",
                    node.node_id,
                    handle.evidence().artifact_id
                )));
            }
            continue;
        }
        by_artifact.insert(
            handle.evidence().artifact_id.clone(),
            ValidatedStagedArtifact {
                evidence: handle.evidence().clone(),
                binding: handle.binding().clone(),
                bytes: staged.bytes().map(ToOwned::to_owned),
            },
        );
    }
    Ok(by_artifact.into_values().collect())
}

fn framework_retention_manifest_artifact(
    runtime_spec: &CertifiedRuntimeSpec,
    run_id: &RunId,
    node: &spec::NodeSpec,
    pre_projection_stream: &[store::KernelEventEnvelope],
    staged_artifacts: &[ValidatedStagedArtifact],
) -> Result<Option<RetentionManifestArtifact>> {
    let manifests = staged_artifacts
        .iter()
        .filter(|artifact| artifact.binding == StagedArtifactBindingKind::RetentionManifest)
        .collect::<Vec<_>>();
    if manifests.is_empty() {
        if matches!(
            &node.framework,
            Some(spec::FrameworkNodeSpec::ProjectRetentionManifest(_))
        ) {
            return Err(RuntimeError::InvalidRunnerOutput(format!(
                "retention framework node {} did not stage a retention manifest",
                node.node_id
            )));
        }
        return Ok(None);
    }
    if !matches!(
        &node.framework,
        Some(spec::FrameworkNodeSpec::ProjectRetentionManifest(_))
    ) {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "node {} staged retention manifest outside framework retention authority",
            node.node_id
        )));
    }
    if manifests.len() != 1 {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "retention framework node {} staged multiple retention manifests",
            node.node_id
        )));
    }
    let staged = manifests[0];
    let Some(bytes) = staged.bytes.as_deref() else {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "retention framework node {} staged manifest without bytes",
            node.node_id
        )));
    };
    let expected = build_retention_manifest_artifact_with_producer(
        runtime_spec,
        run_id,
        pre_projection_stream,
        Some(node.node_id.clone()),
    )?;
    if staged.evidence != expected.evidence || bytes != expected.bytes.as_bytes() {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "retention framework node {} staged manifest outside authoritative stream",
            node.node_id
        )));
    }
    Ok(Some(expected))
}

fn retention_manifest_payloads(
    runtime_spec: &CertifiedRuntimeSpec,
    run_id: &RunId,
    manifest: RetentionManifestArtifact,
) -> Vec<events::KernelEventPayload> {
    let manifest_ref = retention_ref_for_artifact(&manifest.evidence);
    vec![
        events::KernelEventPayload::RetentionManifestProjected(
            events::RetentionManifestProjected {
                run_id: run_id.clone(),
                spec_hash: runtime_spec.spec_hash().clone(),
                manifest_seq: manifest.manifest_seq,
                manifest_digest: manifest.evidence.digest.clone(),
                previous_manifest_digest: manifest.previous_manifest_digest,
                manifest_artifact_id: manifest.evidence.artifact_id.clone(),
            },
        ),
        events::KernelEventPayload::RetentionRefsAppended(events::RetentionRefsAppended {
            run_id: run_id.clone(),
            spec_hash: runtime_spec.spec_hash().clone(),
            refs: vec![manifest_ref],
            reason: events::RetentionReason::ManifestProjection,
        }),
    ]
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ValidatedStagedArtifact {
    evidence: store::ArtifactEvidenceRef,
    binding: StagedArtifactBindingKind,
    bytes: Option<Vec<u8>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StagedArtifactRequirement {
    artifact_id: ArtifactId,
    digest: ContentDigest,
    byte_len: Option<u64>,
    media_type: Option<spec::MediaType>,
    schema_id: Option<SchemaId>,
    semantic_type_id: Option<SemanticTypeId>,
    role: events::ArtifactRole,
    binding: StagedArtifactBindingKind,
}

fn validate_staged_artifact_payload_bindings(
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    payloads: &[events::KernelEventPayload],
    staged_artifacts: &[ValidatedStagedArtifact],
) -> Result<()> {
    let requirements = staged_payload_artifact_requirements(node, attempt_id, payloads)?;
    let mut requirements_by_artifact: BTreeMap<ArtifactId, &StagedArtifactRequirement> =
        BTreeMap::new();
    for requirement in &requirements {
        if let Some(existing) = requirements_by_artifact.get(&requirement.artifact_id) {
            if !staged_artifact_requirements_are_compatible(existing, requirement) {
                return Err(RuntimeError::InvalidRunnerOutput(format!(
                    "node {} returned conflicting typed payload requirements for artifact {}",
                    node.node_id, requirement.artifact_id
                )));
            }
        }
        requirements_by_artifact.insert(requirement.artifact_id.clone(), requirement);
    }
    let staged_by_artifact = staged_artifacts
        .iter()
        .map(|artifact| (artifact.evidence.artifact_id.clone(), artifact))
        .collect::<BTreeMap<_, _>>();

    for staged in staged_artifacts {
        let Some(requirement) = requirements_by_artifact.get(&staged.evidence.artifact_id) else {
            return Err(RuntimeError::InvalidRunnerOutput(format!(
                "node {} staged artifact {} without typed payload reference",
                node.node_id, staged.evidence.artifact_id
            )));
        };
        validate_staged_artifact_requirement(node, &staged.evidence, &staged.binding, requirement)?;
    }

    for requirement in requirements {
        let Some(staged) = staged_by_artifact.get(&requirement.artifact_id) else {
            return Err(RuntimeError::InvalidRunnerOutput(format!(
                "node {} referenced artifact {} without staged artifact",
                node.node_id, requirement.artifact_id
            )));
        };
        validate_staged_artifact_requirement(
            node,
            &staged.evidence,
            &staged.binding,
            &requirement,
        )?;
    }
    Ok(())
}

fn validate_staged_artifact_requirement(
    node: &spec::NodeSpec,
    evidence: &store::ArtifactEvidenceRef,
    binding: &StagedArtifactBindingKind,
    requirement: &StagedArtifactRequirement,
) -> Result<()> {
    if evidence.artifact_id != requirement.artifact_id
        || evidence.digest != requirement.digest
        || requirement
            .byte_len
            .is_some_and(|byte_len| evidence.byte_len != byte_len)
        || requirement
            .media_type
            .as_ref()
            .is_some_and(|media_type| &evidence.media_type != media_type)
        || requirement
            .schema_id
            .as_ref()
            .is_some_and(|schema_id| evidence.schema_id.as_ref() != Some(schema_id))
        || requirement
            .semantic_type_id
            .as_ref()
            .is_some_and(|semantic_type_id| {
                evidence.semantic_type_id.as_ref() != Some(semantic_type_id)
            })
        || evidence.producer_node_id.as_ref() != Some(&node.node_id)
        || evidence.producer_seed_id.is_some()
        || evidence.artifact_role != requirement.role
        || binding != &requirement.binding
    {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "node {} staged artifact {} does not match typed payload binding",
            node.node_id, evidence.artifact_id
        )));
    }
    Ok(())
}

fn staged_artifact_requirements_are_compatible(
    left: &StagedArtifactRequirement,
    right: &StagedArtifactRequirement,
) -> bool {
    left.artifact_id == right.artifact_id
        && left.digest == right.digest
        && optional_requirements_are_compatible(left.byte_len.as_ref(), right.byte_len.as_ref())
        && optional_requirements_are_compatible(left.media_type.as_ref(), right.media_type.as_ref())
        && optional_requirements_are_compatible(left.schema_id.as_ref(), right.schema_id.as_ref())
        && optional_requirements_are_compatible(
            left.semantic_type_id.as_ref(),
            right.semantic_type_id.as_ref(),
        )
        && left.role == right.role
        && left.binding == right.binding
}

fn optional_requirements_are_compatible<T: Eq>(left: Option<&T>, right: Option<&T>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => left == right,
        _ => true,
    }
}

fn staged_payload_artifact_requirements(
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    payloads: &[events::KernelEventPayload],
) -> Result<Vec<StagedArtifactRequirement>> {
    let mut requirements = Vec::new();
    for payload in payloads {
        match payload {
            events::KernelEventPayload::FactRecorded(payload) => {
                require_attempt(node, attempt_id, &payload.node_id, &payload.attempt_id)?;
                requirements.push(StagedArtifactRequirement {
                    artifact_id: payload.artifact_id.clone(),
                    digest: payload.response_hash.clone(),
                    byte_len: None,
                    media_type: None,
                    schema_id: Some(payload.response_schema_id.clone()),
                    semantic_type_id: None,
                    role: events::ArtifactRole::FactResponse,
                    binding: StagedArtifactBindingKind::FactResponse,
                });
            }
            events::KernelEventPayload::CellProduced(payload) => {
                require_attempt(node, attempt_id, &payload.node_id, &payload.attempt_id)?;
                requirements.push(StagedArtifactRequirement {
                    artifact_id: payload.artifact_id.clone(),
                    digest: payload.content_digest.clone(),
                    byte_len: None,
                    media_type: None,
                    schema_id: Some(payload.schema_id.clone()),
                    semantic_type_id: Some(payload.semantic_type_id.clone()),
                    role: events::ArtifactRole::StateOutput,
                    binding: StagedArtifactBindingKind::StateOutput,
                });
            }
            events::KernelEventPayload::PublicOutputProduced(payload) => {
                require_attempt(node, attempt_id, &payload.node_id, &payload.attempt_id)?;
                if let Some(artifact_id) = &payload.rendered_artifact_id {
                    requirements.push(StagedArtifactRequirement {
                        artifact_id: artifact_id.clone(),
                        digest: payload.rendered_digest.clone(),
                        byte_len: None,
                        media_type: None,
                        schema_id: Some(payload.public_schema_id.clone()),
                        semantic_type_id: None,
                        role: events::ArtifactRole::PublicOutput,
                        binding: StagedArtifactBindingKind::PublicOutput,
                    });
                }
            }
            events::KernelEventPayload::PublicOutputRenderFailed(payload) => {
                require_attempt(node, attempt_id, &payload.node_id, &payload.attempt_id)?;
                if let Some(ref diagnostic) = payload.error.diagnostic_ref {
                    push_staged_event_artifact_requirement(
                        node,
                        diagnostic,
                        StagedArtifactBindingKind::RedactedDiagnostic,
                        &mut requirements,
                    )?;
                }
            }
            events::KernelEventPayload::StateAttemptFailed(payload) => {
                require_attempt(node, attempt_id, &payload.node_id, &payload.attempt_id)?;
                if let Some(ref diagnostic) = payload.error.diagnostic_ref {
                    push_staged_event_artifact_requirement(
                        node,
                        diagnostic,
                        StagedArtifactBindingKind::RedactedDiagnostic,
                        &mut requirements,
                    )?;
                }
            }
            events::KernelEventPayload::SideEffectIntentPersisted(payload) => {
                require_attempt(node, attempt_id, &payload.node_id, &payload.attempt_id)?;
                requirements.push(side_effect_artifact_requirement(
                    payload.intent_artifact_id.clone(),
                    payload.intent_hash.clone(),
                    Some(payload.intent_schema_id.clone()),
                    events::ArtifactRole::SideEffectIntent,
                    payload.ledger_key.clone(),
                    payload.invocation_epoch,
                    StagedSideEffectArtifactPhase::Intent,
                ));
            }
            events::KernelEventPayload::SideEffectInvocationPrepared(payload) => {
                require_attempt(node, attempt_id, &payload.node_id, &payload.attempt_id)?;
                if let (Some(artifact_id), Some(hash)) =
                    (&payload.prepared_artifact_id, &payload.prepared_hash)
                {
                    requirements.push(side_effect_artifact_requirement(
                        artifact_id.clone(),
                        hash.clone(),
                        None,
                        events::ArtifactRole::PreparedInvocation,
                        payload.ledger_key.clone(),
                        payload.invocation_epoch,
                        StagedSideEffectArtifactPhase::PreparedInvocation,
                    ));
                }
            }
            events::KernelEventPayload::SideEffectNotSubmittedProven(payload) => {
                require_attempt(node, attempt_id, &payload.node_id, &payload.attempt_id)?;
                requirements.push(side_effect_artifact_requirement(
                    payload.proof_artifact_id.clone(),
                    payload.proof_hash.clone(),
                    Some(payload.proof_schema_id.clone()),
                    events::ArtifactRole::NotSubmittedProof,
                    payload.ledger_key.clone(),
                    payload.invocation_epoch,
                    StagedSideEffectArtifactPhase::NotSubmittedProof,
                ));
            }
            events::KernelEventPayload::SideEffectSubmissionObserved(payload) => {
                require_attempt(node, attempt_id, &payload.node_id, &payload.attempt_id)?;
                requirements.push(side_effect_artifact_requirement(
                    payload.submission_artifact_id.clone(),
                    payload.submission_hash.clone(),
                    Some(payload.submission_schema_id.clone()),
                    events::ArtifactRole::Submission,
                    payload.ledger_key.clone(),
                    payload.invocation_epoch,
                    StagedSideEffectArtifactPhase::Submission,
                ));
            }
            events::KernelEventPayload::SideEffectSubmissionUnknown(payload) => {
                require_attempt(node, attempt_id, &payload.node_id, &payload.attempt_id)?;
                requirements.push(side_effect_artifact_requirement(
                    payload.evidence_artifact_id.clone(),
                    payload.evidence_hash.clone(),
                    Some(payload.evidence_schema_id.clone()),
                    events::ArtifactRole::SubmissionUnknownEvidence,
                    payload.ledger_key.clone(),
                    payload.invocation_epoch,
                    StagedSideEffectArtifactPhase::SubmissionUnknownEvidence,
                ));
            }
            events::KernelEventPayload::SideEffectReceiptObserved(payload) => {
                require_attempt(node, attempt_id, &payload.node_id, &payload.attempt_id)?;
                requirements.push(side_effect_artifact_requirement(
                    payload.receipt_artifact_id.clone(),
                    payload.receipt_hash.clone(),
                    Some(payload.receipt_schema_id.clone()),
                    events::ArtifactRole::Receipt,
                    payload.ledger_key.clone(),
                    payload.invocation_epoch,
                    StagedSideEffectArtifactPhase::Receipt,
                ));
            }
            events::KernelEventPayload::SideEffectConfirmationObserved(payload) => {
                require_attempt(node, attempt_id, &payload.node_id, &payload.attempt_id)?;
                requirements.push(side_effect_artifact_requirement(
                    payload.confirmation_artifact_id.clone(),
                    payload.confirmation_hash.clone(),
                    Some(payload.confirmation_schema_id.clone()),
                    events::ArtifactRole::Confirmation,
                    payload.ledger_key.clone(),
                    payload.invocation_epoch,
                    StagedSideEffectArtifactPhase::Confirmation,
                ));
            }
            events::KernelEventPayload::SideEffectAmbiguous(payload) => {
                require_attempt(node, attempt_id, &payload.node_id, &payload.attempt_id)?;
                requirements.push(side_effect_artifact_requirement(
                    payload.evidence_artifact_id.clone(),
                    payload.evidence_hash.clone(),
                    Some(payload.evidence_schema_id.clone()),
                    events::ArtifactRole::AmbiguityEvidence,
                    payload.ledger_key.clone(),
                    payload.invocation_epoch,
                    StagedSideEffectArtifactPhase::AmbiguityEvidence,
                ));
            }
            events::KernelEventPayload::RunStarted(_)
            | events::KernelEventPayload::ArtifactReferenced(_)
            | events::KernelEventPayload::RunCompleted(_)
            | events::KernelEventPayload::RetentionRefsAppended(_)
            | events::KernelEventPayload::RetentionManifestProjected(_)
            | events::KernelEventPayload::StateAttemptStarted(_)
            | events::KernelEventPayload::StateAttemptCompleted(_)
            | events::KernelEventPayload::CellSkipped(_)
            | events::KernelEventPayload::SideEffectClaimed(_)
            | events::KernelEventPayload::SideEffectClaimTakenOver(_)
            | events::KernelEventPayload::SideEffectInvocationStarted(_)
            | events::KernelEventPayload::SideEffectFailed(_) => {}
        }
    }
    Ok(requirements)
}

fn staged_artifact_reference_payloads(
    spec_hash: &SpecHash,
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    payloads: &[events::KernelEventPayload],
    staged_artifacts: &[ValidatedStagedArtifact],
) -> Vec<events::KernelEventPayload> {
    let existing_refs = payloads
        .iter()
        .filter_map(|payload| match payload {
            events::KernelEventPayload::ArtifactReferenced(payload)
                if payload.node_id.as_ref() == Some(&node.node_id)
                    && payload.attempt_id.as_ref() == Some(attempt_id) =>
            {
                Some(payload.artifact_ref.artifact_id.clone())
            }
            _ => None,
        })
        .collect::<BTreeSet<_>>();
    let mut refs = Vec::new();
    for artifact in staged_artifacts {
        if staged_artifact_binding_kind(artifact.evidence.artifact_role).is_none() {
            continue;
        }
        if existing_refs.contains(&artifact.evidence.artifact_id) {
            continue;
        }
        let Some(schema_id) = artifact.evidence.schema_id.clone() else {
            continue;
        };
        refs.push(events::KernelEventPayload::ArtifactReferenced(
            events::ArtifactReferenced {
                spec_hash: spec_hash.clone(),
                node_id: Some(node.node_id.clone()),
                attempt_id: Some(attempt_id.clone()),
                artifact_ref: events::ArtifactEvidenceRef {
                    artifact_id: artifact.evidence.artifact_id.clone(),
                    role: artifact.evidence.artifact_role,
                    schema_id,
                    semantic_type_id: artifact.evidence.semantic_type_id.clone(),
                    content_digest: artifact.evidence.digest.clone(),
                    byte_len: artifact.evidence.byte_len,
                    media_type: artifact.evidence.media_type.clone(),
                },
            },
        ));
    }
    refs
}

fn push_staged_event_artifact_requirement(
    node: &spec::NodeSpec,
    artifact: &events::ArtifactEvidenceRef,
    binding: StagedArtifactBindingKind,
    requirements: &mut Vec<StagedArtifactRequirement>,
) -> Result<()> {
    let role = staged_artifact_binding_role(&binding);
    if artifact.role != role {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "node {} diagnostic artifact role {} does not match staged binding",
            node.node_id,
            artifact_role_name(artifact.role)
        )));
    }
    requirements.push(StagedArtifactRequirement {
        artifact_id: artifact.artifact_id.clone(),
        digest: artifact.content_digest.clone(),
        byte_len: Some(artifact.byte_len),
        media_type: Some(artifact.media_type.clone()),
        schema_id: Some(artifact.schema_id.clone()),
        semantic_type_id: artifact.semantic_type_id.clone(),
        role: artifact.role,
        binding,
    });
    Ok(())
}

fn side_effect_artifact_requirement(
    artifact_id: ArtifactId,
    digest: ContentDigest,
    schema_id: Option<SchemaId>,
    role: events::ArtifactRole,
    ledger_key: events::SideEffectLedgerKey,
    invocation_epoch: u32,
    phase: StagedSideEffectArtifactPhase,
) -> StagedArtifactRequirement {
    StagedArtifactRequirement {
        artifact_id,
        digest,
        byte_len: None,
        media_type: None,
        schema_id,
        semantic_type_id: None,
        role,
        binding: StagedArtifactBindingKind::SideEffectEvidence {
            ledger_key,
            invocation_epoch,
            phase,
        },
    }
}

fn bind_staged_retention_refs(
    runtime_spec: &CertifiedRuntimeSpec,
    run_id: &RunId,
    node: &spec::NodeSpec,
    required_artifacts: &[store::ArtifactEvidenceRef],
    staged: Vec<StagedRetentionRefs>,
) -> Result<Vec<events::KernelEventPayload>> {
    let artifact_evidence = required_artifacts
        .iter()
        .map(|artifact| (artifact.artifact_id.clone(), artifact))
        .collect::<BTreeMap<_, _>>();
    let mut payloads = Vec::with_capacity(staged.len());
    for staged_refs in staged {
        let reason = staged_refs.reason;
        validate_staged_retention_reason(runtime_spec, node, &artifact_evidence, &staged_refs)?;
        if staged_refs.refs.is_empty() {
            return Err(RuntimeError::InvalidRunnerOutput(format!(
                "node {} staged empty retention refs",
                node.node_id
            )));
        }
        for retention_ref in &staged_refs.refs {
            let Some(artifact) = artifact_evidence.get(&retention_ref.artifact_id) else {
                return Err(RuntimeError::InvalidRunnerOutput(format!(
                    "node {} staged retention for artifact {} without staged artifact evidence",
                    node.node_id, retention_ref.artifact_id
                )));
            };
            if artifact.digest != retention_ref.content_digest
                || artifact.artifact_role != retention_ref.role
            {
                return Err(RuntimeError::InvalidRunnerOutput(format!(
                    "node {} staged retention evidence for artifact {} does not match artifact evidence",
                    node.node_id, retention_ref.artifact_id
                )));
            }
        }
        payloads.push(events::KernelEventPayload::RetentionRefsAppended(
            events::RetentionRefsAppended {
                run_id: run_id.clone(),
                spec_hash: runtime_spec.spec_hash().clone(),
                refs: staged_refs.refs,
                reason,
            },
        ));
    }
    Ok(payloads)
}

fn validate_staged_retention_reason(
    runtime_spec: &CertifiedRuntimeSpec,
    node: &spec::NodeSpec,
    artifact_evidence: &BTreeMap<ArtifactId, &store::ArtifactEvidenceRef>,
    staged_refs: &StagedRetentionRefs,
) -> Result<()> {
    match staged_refs.reason {
        events::RetentionReason::RunStarted | events::RetentionReason::ManifestProjection => {
            return Err(RuntimeError::InvalidRunnerOutput(format!(
                "node {} staged middleware-owned retention reason {}",
                node.node_id,
                retention_reason_str(staged_refs.reason)
            )));
        }
        events::RetentionReason::PublicOutput => {
            if !matches!(
                &node.framework,
                Some(spec::FrameworkNodeSpec::PublicOutputRender(_))
            ) {
                return Err(RuntimeError::InvalidRunnerOutput(format!(
                    "node {} staged public-output retention outside sealed framework renderer",
                    node.node_id
                )));
            }
            let output_cell = runtime_spec.cell(&node.output_cell).ok_or_else(|| {
                RuntimeError::InvalidSpec(format!(
                    "public-output framework node {} references missing output cell {}",
                    node.node_id, node.output_cell
                ))
            })?;
            for retention_ref in &staged_refs.refs {
                let artifact = artifact_evidence
                    .get(&retention_ref.artifact_id)
                    .ok_or_else(|| {
                        RuntimeError::InvalidRunnerOutput(format!(
                            "node {} staged public-output retention for artifact {} without staged artifact evidence",
                            node.node_id, retention_ref.artifact_id
                        ))
                    })?;
                let framework_artifact = artifact.producer_node_id.as_ref() == Some(&node.node_id)
                    && matches!(
                        artifact.artifact_role,
                        events::ArtifactRole::StateOutput | events::ArtifactRole::PublicOutput
                    )
                    && (artifact.artifact_role != events::ArtifactRole::StateOutput
                        || artifact.schema_id.as_ref() == Some(&output_cell.schema_id));
                if !framework_artifact {
                    return Err(RuntimeError::InvalidRunnerOutput(format!(
                        "node {} staged public-output retention for non-framework artifact {}",
                        node.node_id, retention_ref.artifact_id
                    )));
                }
            }
        }
        events::RetentionReason::RuntimeEvidence => {}
    }
    Ok(())
}

fn runner_output_preconditions(
    runtime_spec: &CertifiedRuntimeSpec,
    run_id: &RunId,
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    projections: &store::ProjectionSnapshot,
    payloads: &[events::KernelEventPayload],
    require_existing_attempt: bool,
) -> Result<store::CommitPreconditions> {
    let mut preconditions = store::CommitPreconditions {
        required_run_state: store::RequiredRunState::NotCompleted,
        required_cell_states: vec![store::CellStatePrecondition {
            cell_id: node.output_cell.clone(),
            required: store::RequiredCellState::Absent,
        }],
        required_public_output_absent: matches!(
            &node.framework,
            Some(spec::FrameworkNodeSpec::PublicOutputRender(_))
        ),
        ..store::CommitPreconditions::default()
    };
    if require_existing_attempt {
        preconditions
            .required_present_logical_keys
            .push(store::LogicalEventKey::new(format!(
                "attempt:{}:{}",
                node.node_id, attempt_id
            ))?);
    } else {
        preconditions
            .required_cell_states
            .extend(node_cell_preconditions(runtime_spec, node)?);
    }
    if matches!(
        &node.framework,
        Some(spec::FrameworkNodeSpec::CompleteRun(_))
    ) {
        let completion = run_completion_evidence(runtime_spec, projections)?;
        let retention_manifest = projected_retention_manifest(run_id, projections)?;
        preconditions
            .required_present_logical_keys
            .push(store::LogicalEventKey::new(format!(
                "public_output:{}",
                completion.public_output_schema_id
            ))?);
        preconditions
            .required_present_logical_keys
            .push(store::LogicalEventKey::new(format!(
                "retention:{}:manifest:{}",
                run_id, retention_manifest.manifest_seq
            ))?);
    }

    if node.side_effect.is_none() {
        return Ok(preconditions);
    }

    let mut requires_terminal_confirmation = false;
    let prepared_in_batch = payloads
        .iter()
        .filter_map(|payload| match payload {
            events::KernelEventPayload::SideEffectInvocationPrepared(payload) => {
                Some(payload.ledger_key.clone())
            }
            _ => None,
        })
        .collect::<BTreeSet<_>>();
    for payload in payloads {
        match payload {
            events::KernelEventPayload::SideEffectIntentPersisted(payload) => {
                preconditions.required_side_effect_states.push(
                    store::SideEffectStatePrecondition {
                        ledger_key: payload.ledger_key.clone(),
                        required: store::RequiredSideEffectState::Absent,
                    },
                );
            }
            events::KernelEventPayload::SideEffectInvocationStarted(payload) => {
                if !prepared_in_batch.contains(&payload.ledger_key) {
                    preconditions.required_side_effect_states.push(
                        store::SideEffectStatePrecondition {
                            ledger_key: payload.ledger_key.clone(),
                            required: store::RequiredSideEffectState::InvocationPrepared,
                        },
                    );
                }
            }
            events::KernelEventPayload::SideEffectReceiptObserved(payload) => {
                preconditions.required_side_effect_states.push(
                    store::SideEffectStatePrecondition {
                        ledger_key: payload.ledger_key.clone(),
                        required: store::RequiredSideEffectState::SubmissionResult,
                    },
                );
            }
            events::KernelEventPayload::SideEffectConfirmationObserved(payload) => {
                preconditions.required_side_effect_states.push(
                    store::SideEffectStatePrecondition {
                        ledger_key: payload.ledger_key.clone(),
                        required: store::RequiredSideEffectState::ReceiptObserved,
                    },
                );
            }
            events::KernelEventPayload::CellProduced(_)
            | events::KernelEventPayload::CellSkipped(_)
            | events::KernelEventPayload::StateAttemptCompleted(_) => {
                requires_terminal_confirmation = true;
            }
            _ => {}
        }
    }

    if requires_terminal_confirmation {
        let projection = side_effect_projection_for_attempt(projections, node, attempt_id)?
            .ok_or_else(|| {
                RuntimeError::InvalidRunnerOutput(format!(
                    "side-effect node {} attempted output without ledger evidence",
                    node.node_id
                ))
            })?;
        preconditions
            .required_side_effect_states
            .push(store::SideEffectStatePrecondition {
                ledger_key: projection.ledger_key.clone(),
                required: store::RequiredSideEffectState::ConfirmationObserved,
            });
    }

    Ok(preconditions)
}

fn next_runnable_node<'a>(
    runtime_spec: &'a CertifiedRuntimeSpec,
    view: &RuntimeRunView,
) -> Result<Option<RunnableNode<'a>>> {
    if view
        .projections
        .side_effects()
        .any(|(_, projection)| matches!(projection.phase, store::SideEffectPhase::Ambiguous { .. }))
    {
        return Ok(None);
    }
    for node_id in runtime_spec.topological_order() {
        let node = runtime_spec.node(node_id).expect("topological node exists");
        let Some(attempt) = attempt_plan(runtime_spec, node, view)? else {
            continue;
        };
        if node_inputs_ready(runtime_spec, node, view)? {
            return Ok(Some(RunnableNode { node, attempt }));
        }
    }
    Ok(None)
}

fn attempt_plan(
    runtime_spec: &CertifiedRuntimeSpec,
    node: &spec::NodeSpec,
    view: &RuntimeRunView,
) -> Result<Option<AttemptPlan>> {
    if node.side_effect.is_some() {
        side_effect_attempt_plan(runtime_spec, node, view)
    } else {
        non_side_effect_attempt_plan(runtime_spec, node, view)
    }
}

fn non_side_effect_attempt_plan(
    runtime_spec: &CertifiedRuntimeSpec,
    node: &spec::NodeSpec,
    view: &RuntimeRunView,
) -> Result<Option<AttemptPlan>> {
    if let Some(cell_terminal) = view.projections.cell_terminal(&node.output_cell) {
        validate_terminal_cell_has_completed_attempt(
            runtime_spec,
            &view.projections,
            node,
            cell_terminal,
        )?;
        return Ok(None);
    }

    let mut started = None::<(AttemptId, u32)>;
    for ((attempt_node_id, attempt_id), projection) in view.projections.attempts() {
        if attempt_node_id != &node.node_id {
            continue;
        }
        match &projection.status {
            store::AttemptStatus::Started {
                attempt_no,
                state_kind,
                state_version,
            } => {
                if state_kind != &node.state_kind || state_version != &node.state_version {
                    return Err(RuntimeError::InvalidRunStream(format!(
                        "started attempt {} for node {} has state identity outside the certified spec",
                        attempt_id, node.node_id
                    )));
                }
                if started.replace((attempt_id.clone(), *attempt_no)).is_some() {
                    return Err(RuntimeError::InvalidRunStream(format!(
                        "node {} has multiple non-terminal attempts",
                        node.node_id
                    )));
                }
            }
            store::AttemptStatus::Completed { output_cell_id } => {
                return Err(RuntimeError::InvalidRunStream(format!(
                    "node {} attempt {} completed output cell {} without terminal cell authority",
                    node.node_id, attempt_id, output_cell_id
                )));
            }
            store::AttemptStatus::Failed { retryable, .. } => {
                if !*retryable {
                    return Ok(None);
                }
            }
        }
    }

    if let Some((attempt_id, attempt_no)) = started {
        Ok(Some(AttemptPlan::Continue {
            attempt_id,
            attempt_no,
        }))
    } else {
        Ok(Some(AttemptPlan::StartNew))
    }
}

fn side_effect_attempt_plan(
    runtime_spec: &CertifiedRuntimeSpec,
    node: &spec::NodeSpec,
    view: &RuntimeRunView,
) -> Result<Option<AttemptPlan>> {
    if let Some(cell_terminal) = view.projections.cell_terminal(&node.output_cell) {
        let attempt_id = validate_terminal_cell_has_completed_attempt(
            runtime_spec,
            &view.projections,
            node,
            cell_terminal,
        )?;
        validate_side_effect_terminal_evidence(&view.projections, node, &attempt_id)?;
        return Ok(None);
    }

    let mut started = None::<(AttemptId, u32)>;
    for ((attempt_node_id, attempt_id), projection) in view.projections.attempts() {
        if attempt_node_id != &node.node_id {
            continue;
        }
        match &projection.status {
            store::AttemptStatus::Started {
                attempt_no,
                state_kind,
                state_version,
            } => {
                if state_kind != &node.state_kind || state_version != &node.state_version {
                    return Err(RuntimeError::InvalidRunStream(format!(
                        "started side-effect attempt {} for node {} has state identity outside the certified spec",
                        attempt_id, node.node_id
                    )));
                }
                if let Some(projection) =
                    side_effect_projection_for_attempt(&view.projections, node, attempt_id)?
                {
                    match projection.phase {
                        store::SideEffectPhase::Ambiguous { .. } => return Ok(None),
                        store::SideEffectPhase::Failed { .. } => {
                            return Err(RuntimeError::InvalidRunStream(format!(
                                "side-effect ledger {} failed while attempt {} for node {} remained started",
                                projection.ledger_key, attempt_id, node.node_id
                            )));
                        }
                        _ => {}
                    }
                }
                if started.replace((attempt_id.clone(), *attempt_no)).is_some() {
                    return Err(RuntimeError::InvalidRunStream(format!(
                        "node {} has multiple non-terminal side-effect attempts",
                        node.node_id
                    )));
                }
            }
            store::AttemptStatus::Completed { output_cell_id } => {
                if view.projections.cell_terminal(output_cell_id).is_none() {
                    return Err(RuntimeError::InvalidRunStream(format!(
                        "side-effect node {} attempt {} completed without terminal output cell {}",
                        node.node_id, attempt_id, output_cell_id
                    )));
                }
            }
            store::AttemptStatus::Failed { retryable, .. } => {
                if !*retryable {
                    return Ok(None);
                }
            }
        }
    }

    if let Some((attempt_id, attempt_no)) = started {
        Ok(Some(AttemptPlan::Continue {
            attempt_id,
            attempt_no,
        }))
    } else {
        Ok(Some(AttemptPlan::StartNew))
    }
}

fn validate_terminal_cell_has_completed_attempt(
    runtime_spec: &CertifiedRuntimeSpec,
    projections: &store::ProjectionSnapshot,
    node: &spec::NodeSpec,
    terminal: &store::CellTerminalProjection,
) -> Result<AttemptId> {
    let certified = runtime_spec.cell(&node.output_cell).ok_or_else(|| {
        RuntimeError::InvalidSpec(format!(
            "node {} output cell {} is missing",
            node.node_id, node.output_cell
        ))
    })?;
    let (terminal_node_id, terminal_attempt_id, schema_id, semantic_type_id) = match terminal {
        store::CellTerminalProjection::Produced {
            node_id,
            attempt_id,
            schema_id,
            semantic_type_id,
            ..
        }
        | store::CellTerminalProjection::Skipped {
            node_id,
            attempt_id,
            schema_id,
            semantic_type_id,
            ..
        } => (node_id, attempt_id, schema_id, semantic_type_id),
    };
    if terminal_node_id != &node.node_id
        || certified.producer != spec::CellProducer::Node(node.node_id.clone())
        || schema_id != &certified.schema_id
        || semantic_type_id != &certified.semantic_type_id
    {
        return Err(RuntimeError::InvalidRunStream(format!(
            "terminal cell {} is not certified terminal evidence for node {}",
            node.output_cell, node.node_id
        )));
    }
    match projections.attempt(&node.node_id, terminal_attempt_id) {
        Some(store::AttemptProjection {
            status: store::AttemptStatus::Completed { output_cell_id },
            ..
        }) if output_cell_id == &node.output_cell => Ok(terminal_attempt_id.clone()),
        _ => Err(RuntimeError::InvalidRunStream(format!(
            "terminal cell {} for node {} lacks matching completed attempt {}",
            node.output_cell, node.node_id, terminal_attempt_id
        ))),
    }
}

fn side_effect_projection_for_attempt<'a>(
    projections: &'a store::ProjectionSnapshot,
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
) -> Result<Option<&'a store::SideEffectProjection>> {
    let mut found = None;
    for (_, projection) in projections.side_effects() {
        if projection.intent.node_id == node.node_id
            && projection.intent.attempt_id == *attempt_id
            && found.replace(projection).is_some()
        {
            return Err(RuntimeError::InvalidRunStream(format!(
                "side-effect node {} attempt {} has multiple ledger projections",
                node.node_id, attempt_id
            )));
        }
    }
    Ok(found)
}

fn validate_side_effect_terminal_evidence(
    projections: &store::ProjectionSnapshot,
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
) -> Result<()> {
    let Some(projection) = side_effect_projection_for_attempt(projections, node, attempt_id)?
    else {
        return Err(RuntimeError::InvalidRunStream(format!(
            "side-effect node {} attempt {} produced output without ledger evidence",
            node.node_id, attempt_id
        )));
    };
    if matches!(
        projection.phase,
        store::SideEffectPhase::ConfirmationObserved { .. }
    ) {
        Ok(())
    } else {
        Err(RuntimeError::InvalidRunStream(format!(
            "side-effect node {} attempt {} produced output before confirmation",
            node.node_id, attempt_id
        )))
    }
}

fn recorded_facts_for_attempt(
    projections: &store::ProjectionSnapshot,
    node_id: &NodeId,
    attempt_id: &AttemptId,
) -> Result<RecordedFacts> {
    let mut facts = BTreeMap::new();
    for ((fact_node_id, fact_attempt_id, fact_key), projection) in projections.facts() {
        if fact_node_id != node_id || fact_attempt_id != attempt_id {
            continue;
        }
        if projection.node_id != *node_id
            || projection.attempt_id != *attempt_id
            || projection.fact_key != *fact_key
        {
            return Err(RuntimeError::InvalidRunStream(format!(
                "fact projection {} for node {} attempt {} is internally inconsistent",
                fact_key, node_id, attempt_id
            )));
        }
        facts.insert(
            fact_key.clone(),
            RecordedFact {
                fact_key: fact_key.clone(),
                request_schema_id: projection.request_schema_id.clone(),
                request_hash: projection.request_hash.clone(),
                response_schema_id: projection.response_schema_id.clone(),
                response_hash: projection.response_hash.clone(),
                artifact_id: projection.artifact_id.clone(),
                capability_kind: projection.capability_kind.clone(),
                capability_version: projection.capability_version.clone(),
                adapter_kind: projection.adapter_kind.clone(),
                adapter_version: projection.adapter_version.clone(),
            },
        );
    }
    Ok(RecordedFacts { facts })
}

fn node_inputs_ready(
    runtime_spec: &CertifiedRuntimeSpec,
    node: &spec::NodeSpec,
    view: &RuntimeRunView,
) -> Result<bool> {
    let input_cells = runtime_spec.validate_input_binding(&node.input_bindings.root)?;
    for cell_id in input_cells {
        let cell = runtime_spec.cell(&cell_id).ok_or_else(|| {
            RuntimeError::InvalidSpec(format!(
                "node {} input cell {} is missing",
                node.node_id, cell_id
            ))
        })?;
        match &cell.producer {
            spec::CellProducer::Seed(_) => {
                if !view.seed_cells.contains_key(&cell_id) {
                    return Ok(false);
                }
            }
            spec::CellProducer::Node(_) => {
                if view.projections.cell_terminal(&cell_id).is_none() {
                    return Ok(false);
                }
            }
        }
    }
    Ok(true)
}

fn node_cell_preconditions(
    runtime_spec: &CertifiedRuntimeSpec,
    node: &spec::NodeSpec,
) -> Result<Vec<store::CellStatePrecondition>> {
    let mut preconditions = Vec::new();
    for cell_id in runtime_spec.validate_input_binding(&node.input_bindings.root)? {
        let cell = runtime_spec
            .cell(&cell_id)
            .expect("validated input binding cell exists");
        if matches!(cell.producer, spec::CellProducer::Node(_)) {
            preconditions.push(store::CellStatePrecondition {
                cell_id,
                required: store::RequiredCellState::Terminal,
            });
        }
    }
    Ok(preconditions)
}

fn materialize_inputs(
    runtime_spec: &CertifiedRuntimeSpec,
    node: &spec::NodeSpec,
    view: &RuntimeRunView,
) -> Result<MaterializedInputs> {
    Ok(MaterializedInputs {
        input_schema_id: node.input_bindings.input_schema_id.clone(),
        root: materialize_input_node(runtime_spec, &node.input_bindings.root, view)?,
    })
}

fn materialize_input_node(
    runtime_spec: &CertifiedRuntimeSpec,
    input: &spec::InputBindingNodeSpec,
    view: &RuntimeRunView,
) -> Result<MaterializedInputNode> {
    match input {
        spec::InputBindingNodeSpec::Unit => Ok(MaterializedInputNode::Unit),
        spec::InputBindingNodeSpec::Cell(cell) => Ok(MaterializedInputNode::Cell(Box::new(
            materialize_cell(runtime_spec, cell, view)?,
        ))),
        spec::InputBindingNodeSpec::Tuple(elements) => Ok(MaterializedInputNode::Tuple(
            elements
                .iter()
                .map(|element| materialize_input_node(runtime_spec, element, view))
                .collect::<Result<Vec<_>>>()?,
        )),
        spec::InputBindingNodeSpec::Struct(fields) => Ok(MaterializedInputNode::Struct(
            fields
                .iter()
                .map(|field| {
                    Ok(NamedMaterializedInput {
                        field_path: field.field_path.clone(),
                        node: materialize_input_node(runtime_spec, &field.node, view)?,
                    })
                })
                .collect::<Result<Vec<_>>>()?,
        )),
        spec::InputBindingNodeSpec::Vec { elements, .. } => Ok(MaterializedInputNode::Vec(
            elements
                .iter()
                .map(|element| materialize_input_node(runtime_spec, element, view))
                .collect::<Result<Vec<_>>>()?,
        )),
        spec::InputBindingNodeSpec::NonEmptyVec { elements, .. } => {
            Ok(MaterializedInputNode::NonEmptyVec(
                elements
                    .iter()
                    .map(|element| materialize_input_node(runtime_spec, element, view))
                    .collect::<Result<Vec<_>>>()?,
            ))
        }
    }
}

fn materialize_cell(
    runtime_spec: &CertifiedRuntimeSpec,
    cell: &spec::InputBindingCellSpec,
    view: &RuntimeRunView,
) -> Result<MaterializedCell> {
    let certified = runtime_spec.cell(&cell.cell_id).ok_or_else(|| {
        RuntimeError::InputMaterialization(format!(
            "input cell {} is not certified by the spec",
            cell.cell_id
        ))
    })?;
    if certified.schema_id != cell.schema_id
        || certified.semantic_type_id != cell.semantic_type_id
        || certified.value_lineage != cell.value_lineage
    {
        return Err(RuntimeError::InputMaterialization(format!(
            "input cell {} metadata does not match certified cell",
            cell.cell_id
        )));
    }
    let terminal = match &certified.producer {
        spec::CellProducer::Seed(seed_id) => {
            let seed = view.seed_cells.get(&cell.cell_id).ok_or_else(|| {
                RuntimeError::InputMaterialization(format!(
                    "seed cell {} has no RunStarted evidence",
                    cell.cell_id
                ))
            })?;
            let seed_artifact =
                committed_input_artifact(view, &cell.cell_id, &seed.seed_artifact.artifact_id)?;
            if seed_artifact.evidence.digest != seed.digest
                || seed_artifact.evidence.byte_len != seed.seed_artifact.byte_len
                || seed_artifact.evidence.media_type != seed.seed_artifact.media_type
                || seed_artifact.evidence.schema_id.as_ref() != Some(&seed.schema_id)
                || seed_artifact.evidence.semantic_type_id.as_ref() != Some(&seed.semantic_type_id)
                || seed_artifact.evidence.producer_node_id.is_some()
                || seed_artifact.evidence.producer_seed_id.as_ref() != Some(seed_id)
                || seed_artifact.evidence.artifact_role != events::ArtifactRole::SeedInput
            {
                return Err(RuntimeError::InputMaterialization(format!(
                    "seed cell {} committed artifact evidence does not match certified seed",
                    cell.cell_id
                )));
            }
            MaterializedCellTerminal::Seed {
                seed_id: seed_id.clone(),
                artifact_id: seed.seed_artifact.artifact_id.clone(),
                content_digest: seed.digest.clone(),
            }
        }
        spec::CellProducer::Node(_) => {
            let projection = view
                .projections
                .cell_terminal(&cell.cell_id)
                .ok_or_else(|| {
                    RuntimeError::InputMaterialization(format!(
                        "input cell {} is not terminal",
                        cell.cell_id
                    ))
                })?;
            match projection {
                store::CellTerminalProjection::Produced {
                    node_id,
                    attempt_id,
                    schema_id,
                    semantic_type_id,
                    artifact_id,
                    content_digest,
                    ..
                } => {
                    if certified.producer != spec::CellProducer::Node(node_id.clone())
                        || schema_id != &cell.schema_id
                        || semantic_type_id != &cell.semantic_type_id
                    {
                        return Err(RuntimeError::InputMaterialization(format!(
                            "produced cell {} projection metadata mismatch",
                            cell.cell_id
                        )));
                    }
                    match view.projections.attempt(node_id, attempt_id) {
                        Some(store::AttemptProjection {
                            status: store::AttemptStatus::Completed { output_cell_id },
                            ..
                        }) if output_cell_id == &cell.cell_id => {}
                        _ => {
                            return Err(RuntimeError::InputMaterialization(format!(
                                "produced cell {} is not backed by a completed producer attempt",
                                cell.cell_id
                            )));
                        }
                    }
                    let artifact = committed_input_artifact(view, &cell.cell_id, artifact_id)?;
                    if artifact.evidence.digest != *content_digest
                        || artifact.evidence.schema_id.as_ref() != Some(schema_id)
                        || artifact.evidence.semantic_type_id.as_ref() != Some(semantic_type_id)
                        || artifact.evidence.producer_node_id.as_ref() != Some(node_id)
                        || artifact.evidence.producer_seed_id.is_some()
                        || artifact.evidence.artifact_role != events::ArtifactRole::StateOutput
                        || artifact.attempt_id.as_ref() != Some(attempt_id)
                    {
                        return Err(RuntimeError::InputMaterialization(format!(
                            "produced cell {} committed artifact evidence does not match terminal projection",
                            cell.cell_id
                        )));
                    }
                    MaterializedCellTerminal::Produced {
                        artifact_id: artifact_id.clone(),
                        content_digest: content_digest.clone(),
                    }
                }
                store::CellTerminalProjection::Skipped {
                    node_id,
                    attempt_id,
                    schema_id,
                    semantic_type_id,
                    skip_reason,
                    ..
                } => {
                    if cell.required_terminal == spec::RequiredTerminal::ProducedOnly {
                        return Err(RuntimeError::InputMaterialization(format!(
                            "input cell {} requires produced terminal but was skipped",
                            cell.cell_id
                        )));
                    }
                    if certified.producer != spec::CellProducer::Node(node_id.clone())
                        || schema_id != &cell.schema_id
                        || semantic_type_id != &cell.semantic_type_id
                    {
                        return Err(RuntimeError::InputMaterialization(format!(
                            "skipped cell {} projection metadata mismatch",
                            cell.cell_id
                        )));
                    }
                    match view.projections.attempt(node_id, attempt_id) {
                        Some(store::AttemptProjection {
                            status: store::AttemptStatus::Completed { output_cell_id },
                            ..
                        }) if output_cell_id == &cell.cell_id => {}
                        _ => {
                            return Err(RuntimeError::InputMaterialization(format!(
                                "skipped cell {} is not backed by a completed producer attempt",
                                cell.cell_id
                            )));
                        }
                    }
                    MaterializedCellTerminal::Skipped {
                        skip_reason: skip_reason.clone(),
                    }
                }
            }
        }
    };
    Ok(MaterializedCell {
        cell_id: cell.cell_id.clone(),
        schema_id: cell.schema_id.clone(),
        semantic_type_id: cell.semantic_type_id.clone(),
        value_lineage: cell.value_lineage.clone(),
        terminal,
    })
}

fn validate_seed_cells(
    runtime_spec: &CertifiedRuntimeSpec,
    seed_cells: &[events::SeedCellRef],
) -> Result<BTreeMap<CellId, events::SeedCellRef>> {
    let mut by_seed = BTreeMap::<_, _>::new();
    let mut by_cell = BTreeMap::<_, _>::new();
    for seed in seed_cells {
        if by_seed.insert(seed.seed_id.clone(), seed.clone()).is_some() {
            return Err(RuntimeError::InvalidRunStream(format!(
                "duplicate seed evidence for {}",
                seed.seed_id
            )));
        }
        if by_cell.insert(seed.cell_id.clone(), seed.clone()).is_some() {
            return Err(RuntimeError::InvalidRunStream(format!(
                "duplicate seed cell evidence for {}",
                seed.cell_id
            )));
        }
    }
    for declared in &runtime_spec.spec().seeds {
        let seed = by_seed.get(&declared.seed_id).ok_or_else(|| {
            RuntimeError::InvalidRunStream(format!(
                "missing RunStarted seed evidence for {}",
                declared.seed_id
            ))
        })?;
        if seed.cell_id != declared.cell_id
            || seed.scope_id != declared.scope_id
            || seed.schema_id != declared.schema_id
            || seed.semantic_type_id != declared.semantic_type_id
        {
            return Err(RuntimeError::InvalidRunStream(format!(
                "RunStarted seed {} metadata does not match certified seed",
                declared.seed_id
            )));
        }
        if let Some(required_digest) = &declared.required_digest {
            if &seed.digest != required_digest {
                return Err(RuntimeError::InvalidRunStream(format!(
                    "RunStarted seed {} digest does not match certified required digest",
                    declared.seed_id
                )));
            }
        }
        if seed.seed_artifact.role != events::ArtifactRole::SeedInput
            || seed.seed_artifact.schema_id != declared.schema_id
            || seed.seed_artifact.semantic_type_id.as_ref() != Some(&declared.semantic_type_id)
            || seed.seed_artifact.content_digest != seed.digest
        {
            return Err(RuntimeError::InvalidRunStream(format!(
                "RunStarted seed {} artifact evidence does not match certified seed",
                declared.seed_id
            )));
        }
    }
    if by_seed.len() != runtime_spec.spec().seeds.len() {
        return Err(RuntimeError::InvalidRunStream(
            "RunStarted contains seed evidence not certified by the spec".to_owned(),
        ));
    }
    Ok(by_cell)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HistoricalSideEffectPhase {
    IntentPersisted,
    Claimed,
    InvocationPrepared,
    InvocationStarted,
    NotSubmittedProven,
    SubmissionObserved,
    SubmissionUnknown,
    ReceiptObserved,
    ConfirmationObserved,
    Ambiguous,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct HistoricalSideEffectLedger {
    node_id: NodeId,
    attempt_id: AttemptId,
    phase: HistoricalSideEffectPhase,
}

fn validate_historical_run_stream(
    runtime_spec: &CertifiedRuntimeSpec,
    run_id: &RunId,
    stream: &[store::KernelEventEnvelope],
    projections: &store::ProjectionSnapshot,
) -> Result<()> {
    let mut available_cells = BTreeSet::<CellId>::new();
    let mut active_attempts = BTreeSet::<(NodeId, AttemptId)>::new();
    let mut side_effect_ledgers =
        BTreeMap::<events::SideEffectLedgerKey, HistoricalSideEffectLedger>::new();
    let mut seen_run_started = false;
    let mut produced_public_output = None::<events::PublicOutputCompletionEvidence>;
    let mut retention_manifest_projected_seq = None::<store::StreamSeq>;
    let mut completed = false;
    for event in stream {
        if completed {
            return Err(RuntimeError::InvalidRunStream(
                "run stream contains events after RunCompleted".to_owned(),
            ));
        }
        if !seen_run_started
            && !matches!(event.payload(), events::KernelEventPayload::RunStarted(_))
        {
            return Err(RuntimeError::InvalidRunStream(
                "run stream events appeared before RunStarted".to_owned(),
            ));
        }
        match event.payload() {
            events::KernelEventPayload::RunStarted(payload) => {
                if seen_run_started {
                    return Err(RuntimeError::InvalidRunStream(
                        "run stream contains multiple RunStarted events".to_owned(),
                    ));
                }
                seen_run_started = true;
                for cell_id in validate_seed_cells(runtime_spec, &payload.seed_cells)?.into_keys() {
                    available_cells.insert(cell_id);
                }
            }
            events::KernelEventPayload::RunCompleted(payload) => {
                if !active_attempts.is_empty() {
                    return Err(RuntimeError::InvalidRunStream(
                        "RunCompleted cannot finalize while state attempts are active".to_owned(),
                    ));
                }
                validate_historical_run_completed(
                    runtime_spec,
                    run_id,
                    event,
                    payload,
                    produced_public_output.as_ref(),
                    retention_manifest_projected_seq,
                )?;
                completed = true;
            }
            events::KernelEventPayload::RetentionRefsAppended(_) => {}
            events::KernelEventPayload::RetentionManifestProjected(payload) => {
                if &payload.run_id == run_id && payload.spec_hash == *runtime_spec.spec_hash() {
                    retention_manifest_projected_seq = Some(event.seq());
                }
            }
            events::KernelEventPayload::StateAttemptStarted(payload) => {
                let node = runtime_spec.node(&payload.node_id).ok_or_else(|| {
                    RuntimeError::InvalidRunStream(format!(
                        "attempt started for uncertified node {}",
                        payload.node_id
                    ))
                })?;
                if payload.state_kind != node.state_kind
                    || payload.state_version != node.state_version
                {
                    return Err(RuntimeError::InvalidRunStream(format!(
                        "attempt {} for node {} carries state identity outside the certified spec",
                        payload.attempt_id, payload.node_id
                    )));
                }
                validate_attempt_start_boundary(runtime_spec, node, payload, &available_cells)?;
                if !active_attempts.insert((payload.node_id.clone(), payload.attempt_id.clone())) {
                    return Err(RuntimeError::InvalidRunStream(format!(
                        "attempt {} for node {} was started more than once",
                        payload.attempt_id, payload.node_id
                    )));
                }
            }
            events::KernelEventPayload::StateAttemptCompleted(payload) => {
                let node = runtime_spec.node(&payload.node_id).ok_or_else(|| {
                    RuntimeError::InvalidRunStream(format!(
                        "attempt completed for uncertified node {}",
                        payload.node_id
                    ))
                })?;
                if payload.output_cell_id != node.output_cell {
                    return Err(RuntimeError::InvalidRunStream(format!(
                        "attempt {} for node {} completed uncertified output cell {}",
                        payload.attempt_id, payload.node_id, payload.output_cell_id
                    )));
                }
                if node.side_effect.is_some() {
                    validate_historical_side_effect_confirmation(
                        &side_effect_ledgers,
                        &payload.node_id,
                        &payload.attempt_id,
                    )?;
                }
                if !active_attempts.remove(&(payload.node_id.clone(), payload.attempt_id.clone())) {
                    return Err(RuntimeError::InvalidRunStream(format!(
                        "attempt completion for node {} attempt {} was not preceded by an active attempt",
                        payload.node_id, payload.attempt_id
                    )));
                }
            }
            events::KernelEventPayload::StateAttemptFailed(payload) => {
                runtime_spec.node(&payload.node_id).ok_or_else(|| {
                    RuntimeError::InvalidRunStream(format!(
                        "attempt failed for uncertified node {}",
                        payload.node_id
                    ))
                })?;
                if runtime_spec
                    .node(&payload.node_id)
                    .expect("checked above")
                    .side_effect
                    .is_some()
                {
                    validate_historical_side_effect_failure(
                        &side_effect_ledgers,
                        &payload.node_id,
                        &payload.attempt_id,
                    )?;
                }
                if !active_attempts.remove(&(payload.node_id.clone(), payload.attempt_id.clone())) {
                    return Err(RuntimeError::InvalidRunStream(format!(
                        "attempt failure for node {} attempt {} was not preceded by an active attempt",
                        payload.node_id, payload.attempt_id
                    )));
                }
            }
            events::KernelEventPayload::CellProduced(payload) => {
                validate_historical_produced_cell(runtime_spec, projections, event, payload)?;
                let node = runtime_spec
                    .node(&payload.node_id)
                    .expect("validated cell node");
                if node.side_effect.is_some() {
                    validate_historical_side_effect_confirmation(
                        &side_effect_ledgers,
                        &payload.node_id,
                        &payload.attempt_id,
                    )?;
                }
                available_cells.insert(payload.cell_id.clone());
            }
            events::KernelEventPayload::CellSkipped(payload) => {
                validate_historical_skipped_cell(runtime_spec, projections, event, payload)?;
                let node = runtime_spec
                    .node(&payload.node_id)
                    .expect("validated cell node");
                if node.side_effect.is_some() {
                    validate_historical_side_effect_confirmation(
                        &side_effect_ledgers,
                        &payload.node_id,
                        &payload.attempt_id,
                    )?;
                }
                available_cells.insert(payload.cell_id.clone());
            }
            events::KernelEventPayload::FactRecorded(payload) => {
                let node = runtime_spec.node(&payload.node_id).ok_or_else(|| {
                    RuntimeError::InvalidRunStream(format!(
                        "fact recorded for uncertified node {}",
                        payload.node_id
                    ))
                })?;
                let caps = CertifiedRuntimeCapabilities::new(
                    node.node_id.clone(),
                    node.capability_bindings.clone(),
                );
                require_capability(
                    &caps,
                    &payload.capability_kind,
                    &payload.capability_version,
                    &node.node_id,
                )
                .map_err(|error| RuntimeError::InvalidRunStream(error.to_string()))?;
                require_adapter(node, &payload.adapter_kind, &payload.adapter_version)
                    .map_err(|error| RuntimeError::InvalidRunStream(error.to_string()))?;
                require_projected_attempt(
                    projections,
                    &payload.node_id,
                    &payload.attempt_id,
                    "fact",
                )?;
                if !active_attempts.contains(&(payload.node_id.clone(), payload.attempt_id.clone()))
                {
                    return Err(RuntimeError::InvalidRunStream(format!(
                        "fact {} for node {} attempt {} was recorded outside an active started attempt",
                        payload.fact_key, payload.node_id, payload.attempt_id
                    )));
                }
                let fact = projections
                    .fact(&payload.node_id, &payload.attempt_id, &payload.fact_key)
                    .ok_or_else(|| {
                        RuntimeError::InvalidRunStream(format!(
                            "fact {} for node {} attempt {} is not projected",
                            payload.fact_key, payload.node_id, payload.attempt_id
                        ))
                    })?;
                if fact.event_id != *event.event_id()
                    || fact.request_schema_id != payload.request_schema_id
                    || fact.request_hash != payload.request_hash
                    || fact.response_schema_id != payload.response_schema_id
                    || fact.response_hash != payload.response_hash
                    || fact.artifact_id != payload.artifact_id
                {
                    return Err(RuntimeError::InvalidRunStream(format!(
                        "fact {} projection does not match authoritative event",
                        payload.fact_key
                    )));
                }
            }
            events::KernelEventPayload::ArtifactReferenced(payload) => {
                if let Some(node_id) = &payload.node_id {
                    runtime_spec.node(node_id).ok_or_else(|| {
                        RuntimeError::InvalidRunStream(format!(
                            "artifact referenced for uncertified node {node_id}"
                        ))
                    })?;
                }
            }
            events::KernelEventPayload::PublicOutputProduced(payload) => {
                let node = runtime_spec.node(&payload.node_id).ok_or_else(|| {
                    RuntimeError::InvalidRunStream(format!(
                        "public output produced by uncertified node {}",
                        payload.node_id
                    ))
                })?;
                validate_public_output(runtime_spec, node, payload)
                    .map_err(|error| RuntimeError::InvalidRunStream(error.to_string()))?;
                validate_historical_public_output_produced(
                    runtime_spec,
                    projections,
                    event,
                    node,
                    payload,
                )?;
                produced_public_output = Some(events::PublicOutputCompletionEvidence {
                    public_output_schema_id: payload.public_schema_id.clone(),
                    public_output_event_id: event.event_id().clone(),
                });
            }
            events::KernelEventPayload::PublicOutputRenderFailed(payload) => {
                let node = runtime_spec.node(&payload.node_id).ok_or_else(|| {
                    RuntimeError::InvalidRunStream(format!(
                        "public output failure by uncertified node {}",
                        payload.node_id
                    ))
                })?;
                validate_public_output_render_node(
                    runtime_spec,
                    node,
                    payload.public_schema_id.clone(),
                    &payload.renderer_descriptor_id,
                )
                .map_err(|error| RuntimeError::InvalidRunStream(error.to_string()))?;
                validate_historical_public_output_failed(projections, payload)?;
            }
            events::KernelEventPayload::SideEffectIntentPersisted(_)
            | events::KernelEventPayload::SideEffectClaimed(_)
            | events::KernelEventPayload::SideEffectClaimTakenOver(_)
            | events::KernelEventPayload::SideEffectInvocationPrepared(_)
            | events::KernelEventPayload::SideEffectInvocationStarted(_)
            | events::KernelEventPayload::SideEffectNotSubmittedProven(_)
            | events::KernelEventPayload::SideEffectSubmissionObserved(_)
            | events::KernelEventPayload::SideEffectSubmissionUnknown(_)
            | events::KernelEventPayload::SideEffectReceiptObserved(_)
            | events::KernelEventPayload::SideEffectConfirmationObserved(_)
            | events::KernelEventPayload::SideEffectAmbiguous(_)
            | events::KernelEventPayload::SideEffectFailed(_) => {
                validate_historical_side_effect_payload(
                    runtime_spec,
                    &active_attempts,
                    &mut side_effect_ledgers,
                    event.payload(),
                )?;
            }
        }
    }
    validate_atomic_terminal_pairs(runtime_spec, stream)?;
    validate_historical_retention_ref_batches(runtime_spec, stream)?;
    validate_historical_retention_manifest_batches(runtime_spec, stream)?;
    validate_historical_complete_run_tail(runtime_spec, run_id, stream)?;
    validate_atomic_side_effect_failure_pairs(runtime_spec, stream)?;
    validate_recovery_frontier(runtime_spec, projections)?;
    Ok(())
}

fn validate_historical_side_effect_payload(
    runtime_spec: &CertifiedRuntimeSpec,
    active_attempts: &BTreeSet<(NodeId, AttemptId)>,
    ledgers: &mut BTreeMap<events::SideEffectLedgerKey, HistoricalSideEffectLedger>,
    payload: &events::KernelEventPayload,
) -> Result<()> {
    let (node_id, attempt_id, ledger_key, phase) = side_effect_payload_ref(payload)
        .ok_or_else(|| RuntimeError::InvalidRunStream("expected side-effect payload".to_owned()))?;
    let node = runtime_spec.node(node_id).ok_or_else(|| {
        RuntimeError::InvalidRunStream(format!("side-effect event for uncertified node {node_id}"))
    })?;
    if node.side_effect.is_none() {
        return Err(RuntimeError::InvalidRunStream(format!(
            "non-side-effect node {} emitted side-effect ledger event",
            node.node_id
        )));
    }
    if !active_attempts.contains(&(node_id.clone(), attempt_id.clone())) {
        return Err(RuntimeError::InvalidRunStream(format!(
            "side-effect ledger {} for node {} attempt {} was recorded outside an active started attempt",
            ledger_key, node_id, attempt_id
        )));
    }

    match payload {
        events::KernelEventPayload::SideEffectIntentPersisted(payload) => {
            if payload.scope_id != node.scope_id {
                return Err(RuntimeError::InvalidRunStream(format!(
                    "side-effect intent for node {} carries uncertified scope {}",
                    node.node_id, payload.scope_id
                )));
            }
            let caps = CertifiedRuntimeCapabilities::new(
                node.node_id.clone(),
                node.capability_bindings.clone(),
            );
            require_capability(
                &caps,
                &payload.capability_kind,
                &payload.capability_version,
                &node.node_id,
            )
            .map_err(|error| RuntimeError::InvalidRunStream(error.to_string()))?;
            require_adapter(node, &payload.adapter_kind, &payload.adapter_version)
                .map_err(|error| RuntimeError::InvalidRunStream(error.to_string()))?;
            if ledgers
                .insert(
                    ledger_key.clone(),
                    HistoricalSideEffectLedger {
                        node_id: node_id.clone(),
                        attempt_id: attempt_id.clone(),
                        phase,
                    },
                )
                .is_some()
            {
                return Err(RuntimeError::InvalidRunStream(format!(
                    "side-effect ledger {} persisted intent more than once",
                    ledger_key
                )));
            }
        }
        _ => {
            let ledger = ledgers.get_mut(ledger_key).ok_or_else(|| {
                RuntimeError::InvalidRunStream(format!(
                    "side-effect ledger {} advanced before intent was persisted",
                    ledger_key
                ))
            })?;
            if ledger.node_id != *node_id || ledger.attempt_id != *attempt_id {
                return Err(RuntimeError::InvalidRunStream(format!(
                    "side-effect ledger {} changed node or attempt authority",
                    ledger_key
                )));
            }
            ledger.phase = phase;
        }
    }

    Ok(())
}

fn side_effect_payload_ref(
    payload: &events::KernelEventPayload,
) -> Option<(
    &NodeId,
    &AttemptId,
    &events::SideEffectLedgerKey,
    HistoricalSideEffectPhase,
)> {
    match payload {
        events::KernelEventPayload::SideEffectIntentPersisted(payload) => Some((
            &payload.node_id,
            &payload.attempt_id,
            &payload.ledger_key,
            HistoricalSideEffectPhase::IntentPersisted,
        )),
        events::KernelEventPayload::SideEffectClaimed(payload) => Some((
            &payload.node_id,
            &payload.attempt_id,
            &payload.ledger_key,
            HistoricalSideEffectPhase::Claimed,
        )),
        events::KernelEventPayload::SideEffectClaimTakenOver(payload) => Some((
            &payload.node_id,
            &payload.attempt_id,
            &payload.ledger_key,
            HistoricalSideEffectPhase::Claimed,
        )),
        events::KernelEventPayload::SideEffectInvocationPrepared(payload) => Some((
            &payload.node_id,
            &payload.attempt_id,
            &payload.ledger_key,
            HistoricalSideEffectPhase::InvocationPrepared,
        )),
        events::KernelEventPayload::SideEffectInvocationStarted(payload) => Some((
            &payload.node_id,
            &payload.attempt_id,
            &payload.ledger_key,
            HistoricalSideEffectPhase::InvocationStarted,
        )),
        events::KernelEventPayload::SideEffectNotSubmittedProven(payload) => Some((
            &payload.node_id,
            &payload.attempt_id,
            &payload.ledger_key,
            HistoricalSideEffectPhase::NotSubmittedProven,
        )),
        events::KernelEventPayload::SideEffectSubmissionObserved(payload) => Some((
            &payload.node_id,
            &payload.attempt_id,
            &payload.ledger_key,
            HistoricalSideEffectPhase::SubmissionObserved,
        )),
        events::KernelEventPayload::SideEffectSubmissionUnknown(payload) => Some((
            &payload.node_id,
            &payload.attempt_id,
            &payload.ledger_key,
            HistoricalSideEffectPhase::SubmissionUnknown,
        )),
        events::KernelEventPayload::SideEffectReceiptObserved(payload) => Some((
            &payload.node_id,
            &payload.attempt_id,
            &payload.ledger_key,
            HistoricalSideEffectPhase::ReceiptObserved,
        )),
        events::KernelEventPayload::SideEffectConfirmationObserved(payload) => Some((
            &payload.node_id,
            &payload.attempt_id,
            &payload.ledger_key,
            HistoricalSideEffectPhase::ConfirmationObserved,
        )),
        events::KernelEventPayload::SideEffectAmbiguous(payload) => Some((
            &payload.node_id,
            &payload.attempt_id,
            &payload.ledger_key,
            HistoricalSideEffectPhase::Ambiguous,
        )),
        events::KernelEventPayload::SideEffectFailed(payload) => Some((
            &payload.node_id,
            &payload.attempt_id,
            &payload.ledger_key,
            HistoricalSideEffectPhase::Failed,
        )),
        _ => None,
    }
}

fn validate_historical_side_effect_confirmation(
    ledgers: &BTreeMap<events::SideEffectLedgerKey, HistoricalSideEffectLedger>,
    node_id: &NodeId,
    attempt_id: &AttemptId,
) -> Result<()> {
    let ledger = historical_side_effect_ledger_for_attempt(ledgers, node_id, attempt_id)?;
    if ledger.phase == HistoricalSideEffectPhase::ConfirmationObserved {
        Ok(())
    } else {
        Err(RuntimeError::InvalidRunStream(format!(
            "side-effect node {} attempt {} produced output before confirmation",
            node_id, attempt_id
        )))
    }
}

fn validate_historical_side_effect_failure(
    ledgers: &BTreeMap<events::SideEffectLedgerKey, HistoricalSideEffectLedger>,
    node_id: &NodeId,
    attempt_id: &AttemptId,
) -> Result<()> {
    let ledger = historical_side_effect_ledger_for_attempt(ledgers, node_id, attempt_id)?;
    if ledger.phase == HistoricalSideEffectPhase::Failed {
        Ok(())
    } else {
        Err(RuntimeError::InvalidRunStream(format!(
            "side-effect node {} attempt {} failed without side-effect failure evidence",
            node_id, attempt_id
        )))
    }
}

fn historical_side_effect_ledger_for_attempt<'a>(
    ledgers: &'a BTreeMap<events::SideEffectLedgerKey, HistoricalSideEffectLedger>,
    node_id: &NodeId,
    attempt_id: &AttemptId,
) -> Result<&'a HistoricalSideEffectLedger> {
    let mut found = None;
    for ledger in ledgers.values() {
        if ledger.node_id == *node_id
            && ledger.attempt_id == *attempt_id
            && found.replace(ledger).is_some()
        {
            return Err(RuntimeError::InvalidRunStream(format!(
                "side-effect node {} attempt {} has multiple ledgers in history",
                node_id, attempt_id
            )));
        }
    }
    found.ok_or_else(|| {
        RuntimeError::InvalidRunStream(format!(
            "side-effect node {} attempt {} lacks ledger evidence",
            node_id, attempt_id
        ))
    })
}

fn validate_attempt_start_boundary(
    runtime_spec: &CertifiedRuntimeSpec,
    node: &spec::NodeSpec,
    payload: &events::StateAttemptStarted,
    available_cells: &BTreeSet<CellId>,
) -> Result<()> {
    for cell_id in runtime_spec.validate_input_binding(&node.input_bindings.root)? {
        if !available_cells.contains(&cell_id) {
            return Err(RuntimeError::InvalidRunStream(format!(
                "attempt {} for node {} started before certified input cell {} was terminal",
                payload.attempt_id, node.node_id, cell_id
            )));
        }
    }
    Ok(())
}

fn validate_atomic_terminal_pairs(
    runtime_spec: &CertifiedRuntimeSpec,
    stream: &[store::KernelEventEnvelope],
) -> Result<()> {
    let mut completions = BTreeSet::new();
    let mut terminal_cells = BTreeSet::new();
    let mut public_outputs = BTreeSet::new();
    for event in stream {
        match event.payload() {
            events::KernelEventPayload::StateAttemptCompleted(payload) => {
                let node = runtime_spec.node(&payload.node_id).ok_or_else(|| {
                    RuntimeError::InvalidRunStream(format!(
                        "attempt completed for uncertified node {}",
                        payload.node_id
                    ))
                })?;
                let _ = node;
                completions.insert((
                    event.seq(),
                    payload.node_id.clone(),
                    payload.attempt_id.clone(),
                    payload.output_cell_id.clone(),
                ));
            }
            events::KernelEventPayload::CellProduced(payload) => {
                let node = runtime_spec.node(&payload.node_id).ok_or_else(|| {
                    RuntimeError::InvalidRunStream(format!(
                        "cell produced by uncertified node {}",
                        payload.node_id
                    ))
                })?;
                let _ = node;
                terminal_cells.insert((
                    event.seq(),
                    payload.node_id.clone(),
                    payload.attempt_id.clone(),
                    payload.cell_id.clone(),
                ));
            }
            events::KernelEventPayload::CellSkipped(payload) => {
                let node = runtime_spec.node(&payload.node_id).ok_or_else(|| {
                    RuntimeError::InvalidRunStream(format!(
                        "cell skipped by uncertified node {}",
                        payload.node_id
                    ))
                })?;
                let _ = node;
                terminal_cells.insert((
                    event.seq(),
                    payload.node_id.clone(),
                    payload.attempt_id.clone(),
                    payload.cell_id.clone(),
                ));
            }
            events::KernelEventPayload::PublicOutputProduced(payload) => {
                let node = runtime_spec.node(&payload.node_id).ok_or_else(|| {
                    RuntimeError::InvalidRunStream(format!(
                        "public output produced by uncertified node {}",
                        payload.node_id
                    ))
                })?;
                if !matches!(
                    &node.framework,
                    Some(spec::FrameworkNodeSpec::PublicOutputRender(_))
                ) {
                    return Err(RuntimeError::InvalidRunStream(format!(
                        "public output produced by non-render node {}",
                        payload.node_id
                    )));
                }
                public_outputs.insert((
                    event.seq(),
                    payload.node_id.clone(),
                    payload.attempt_id.clone(),
                    payload.receipt_cell_id.clone(),
                ));
            }
            _ => {}
        }
    }

    for (seq, node_id, attempt_id, output_cell_id) in &completions {
        if !terminal_cells.contains(&(
            *seq,
            node_id.clone(),
            attempt_id.clone(),
            output_cell_id.clone(),
        )) {
            return Err(RuntimeError::InvalidRunStream(format!(
                "attempt {} for node {} completed without terminal cell {} in the same commit",
                attempt_id, node_id, output_cell_id
            )));
        }
    }
    for (seq, node_id, attempt_id, cell_id) in &terminal_cells {
        if !completions.contains(&(*seq, node_id.clone(), attempt_id.clone(), cell_id.clone())) {
            return Err(RuntimeError::InvalidRunStream(format!(
                "terminal cell {} for node {} lacks StateAttemptCompleted in the same commit",
                cell_id, node_id
            )));
        }
    }
    for (seq, node_id, attempt_id, receipt_cell_id) in &public_outputs {
        let terminal = (
            *seq,
            node_id.clone(),
            attempt_id.clone(),
            receipt_cell_id.clone(),
        );
        if !terminal_cells.contains(&terminal) || !completions.contains(&terminal) {
            return Err(RuntimeError::InvalidRunStream(format!(
                "public output for node {} attempt {} was split from receipt terminal cell {}",
                node_id, attempt_id, receipt_cell_id
            )));
        }
    }
    Ok(())
}

fn validate_historical_retention_manifest_batches(
    runtime_spec: &CertifiedRuntimeSpec,
    stream: &[store::KernelEventEnvelope],
) -> Result<()> {
    let mut index = 0;
    while index < stream.len() {
        let first = &stream[index];
        let seq = first.seq();
        let commit_key = first.commit_key().clone();
        let mut end = index + 1;
        while end < stream.len()
            && stream[end].seq() == seq
            && stream[end].commit_key() == &commit_key
        {
            end += 1;
        }
        let commit = &stream[index..end];
        validate_historical_retention_manifest_batch(runtime_spec, &stream[..index], commit)?;
        index = end;
    }
    Ok(())
}

type RetentionRefKey = (ArtifactId, ContentDigest, events::ArtifactRole);

fn validate_historical_retention_ref_batches(
    runtime_spec: &CertifiedRuntimeSpec,
    stream: &[store::KernelEventEnvelope],
) -> Result<()> {
    let mut index = 0;
    while index < stream.len() {
        let first = &stream[index];
        let seq = first.seq();
        let commit_key = first.commit_key().clone();
        let mut end = index + 1;
        while end < stream.len()
            && stream[end].seq() == seq
            && stream[end].commit_key() == &commit_key
        {
            end += 1;
        }
        validate_historical_retention_ref_batch(runtime_spec, &stream[index..end])?;
        index = end;
    }
    Ok(())
}

fn validate_historical_retention_ref_batch(
    runtime_spec: &CertifiedRuntimeSpec,
    commit: &[store::KernelEventEnvelope],
) -> Result<()> {
    let retention_refs = commit
        .iter()
        .filter_map(|event| match event.payload() {
            events::KernelEventPayload::RetentionRefsAppended(payload) => Some((event, payload)),
            _ => None,
        })
        .collect::<Vec<_>>();
    let run_started = commit
        .iter()
        .filter_map(|event| match event.payload() {
            events::KernelEventPayload::RunStarted(payload) => Some(payload),
            _ => None,
        })
        .collect::<Vec<_>>();
    if run_started.len() > 1 {
        return Err(RuntimeError::InvalidRunStream(
            "commit contains multiple RunStarted payloads".to_owned(),
        ));
    }
    if run_started.len() == 1 {
        let run_started_refs = retention_refs
            .iter()
            .filter(|(_, payload)| payload.reason == events::RetentionReason::RunStarted)
            .count();
        if retention_refs.len() != 1 || run_started_refs != 1 {
            return Err(RuntimeError::InvalidRunStream(
                "RunStarted commit must include exactly one run-start retention refs append"
                    .to_owned(),
            ));
        }
    }
    if retention_refs.is_empty() {
        return Ok(());
    }

    let has_manifest_projection = commit.iter().any(|event| {
        matches!(
            event.payload(),
            events::KernelEventPayload::RetentionManifestProjected(_)
        )
    });
    let artifact_refs = same_commit_artifact_reference_keys(commit);
    let typed_payload_refs = same_commit_typed_artifact_keys(commit);

    for (event, payload) in retention_refs {
        if event.run_id() != &payload.run_id {
            return Err(RuntimeError::InvalidRunStream(format!(
                "retention refs for run {} were appended to stream {}",
                payload.run_id,
                event.run_id()
            )));
        }
        if payload.spec_hash != *runtime_spec.spec_hash() {
            return Err(RuntimeError::InvalidRunStream(
                "retention refs spec hash does not match certified runtime spec".to_owned(),
            ));
        }
        if payload.refs.is_empty() {
            return Err(RuntimeError::InvalidRunStream(
                "retention refs append cannot be empty".to_owned(),
            ));
        }
        match payload.reason {
            events::RetentionReason::RunStarted => {
                if run_started.len() != 1 {
                    return Err(RuntimeError::InvalidRunStream(
                        "run-start retention refs must be appended in the RunStarted commit"
                            .to_owned(),
                    ));
                }
                let expected = run_started_retention_ref_keys(run_started[0], commit);
                let actual = payload
                    .refs
                    .iter()
                    .map(retention_ref_key)
                    .collect::<BTreeSet<_>>();
                if actual != expected {
                    return Err(RuntimeError::InvalidRunStream(
                        "run-start retention refs do not match launch artifact evidence".to_owned(),
                    ));
                }
            }
            events::RetentionReason::ManifestProjection => {
                if !has_manifest_projection {
                    return Err(RuntimeError::InvalidRunStream(
                        "manifest-projection retention refs must be appended with the manifest projection"
                            .to_owned(),
                    ));
                }
            }
            events::RetentionReason::RuntimeEvidence => {
                validate_same_commit_retention_ref_evidence(
                    payload,
                    &artifact_refs,
                    &typed_payload_refs,
                )?;
            }
            events::RetentionReason::PublicOutput => {
                validate_same_commit_retention_ref_evidence(
                    payload,
                    &artifact_refs,
                    &typed_payload_refs,
                )?;
                validate_public_output_retention_refs(runtime_spec, commit, payload)?;
            }
        }
    }
    Ok(())
}

fn validate_same_commit_retention_ref_evidence(
    payload: &events::RetentionRefsAppended,
    artifact_refs: &BTreeSet<RetentionRefKey>,
    typed_payload_refs: &BTreeSet<RetentionRefKey>,
) -> Result<()> {
    for retention_ref in &payload.refs {
        let key = retention_ref_key(retention_ref);
        if retention_ref_requires_artifact_reference(retention_ref.role)
            && !artifact_refs.contains(&key)
        {
            return Err(RuntimeError::InvalidRunStream(format!(
                "retention ref for artifact {} lacks same-commit artifact reference evidence",
                retention_ref.artifact_id
            )));
        }
        if !typed_payload_refs.contains(&key) {
            return Err(RuntimeError::InvalidRunStream(format!(
                "retention ref for artifact {} lacks same-commit typed payload evidence",
                retention_ref.artifact_id
            )));
        }
    }
    Ok(())
}

fn validate_public_output_retention_refs(
    runtime_spec: &CertifiedRuntimeSpec,
    commit: &[store::KernelEventEnvelope],
    payload: &events::RetentionRefsAppended,
) -> Result<()> {
    let public_outputs = commit
        .iter()
        .filter_map(|event| match event.payload() {
            events::KernelEventPayload::PublicOutputProduced(payload) => Some(payload),
            _ => None,
        })
        .collect::<Vec<_>>();
    if public_outputs.len() != 1 {
        return Err(RuntimeError::InvalidRunStream(
            "public-output retention refs must be appended with exactly one framework public-output payload"
                .to_owned(),
        ));
    }
    let public_output = public_outputs[0];
    let node = runtime_spec.node(&public_output.node_id).ok_or_else(|| {
        RuntimeError::InvalidRunStream(format!(
            "public-output retention refs reference uncertified node {}",
            public_output.node_id
        ))
    })?;
    if !matches!(
        &node.framework,
        Some(spec::FrameworkNodeSpec::PublicOutputRender(_))
    ) {
        return Err(RuntimeError::InvalidRunStream(format!(
            "public-output retention refs were appended by non-render node {}",
            public_output.node_id
        )));
    }
    let allowed = public_output_retention_ref_keys(commit, public_output)?;
    for retention_ref in &payload.refs {
        if !allowed.contains(&retention_ref_key(retention_ref)) {
            return Err(RuntimeError::InvalidRunStream(format!(
                "public-output retention ref for artifact {} is not sealed to framework public-output evidence",
                retention_ref.artifact_id
            )));
        }
    }
    Ok(())
}

fn public_output_retention_ref_keys(
    commit: &[store::KernelEventEnvelope],
    public_output: &events::PublicOutputProduced,
) -> Result<BTreeSet<RetentionRefKey>> {
    let mut allowed = BTreeSet::new();
    for event in commit {
        if let events::KernelEventPayload::CellProduced(payload) = event.payload() {
            if payload.node_id == public_output.node_id
                && payload.attempt_id == public_output.attempt_id
                && payload.cell_id == public_output.receipt_cell_id
            {
                allowed.insert((
                    payload.artifact_id.clone(),
                    payload.content_digest.clone(),
                    events::ArtifactRole::StateOutput,
                ));
            }
        }
    }
    if let Some(artifact_id) = &public_output.rendered_artifact_id {
        allowed.insert((
            artifact_id.clone(),
            public_output.rendered_digest.clone(),
            events::ArtifactRole::PublicOutput,
        ));
    }
    if allowed.is_empty() {
        return Err(RuntimeError::InvalidRunStream(
            "public-output retention refs lack same-commit framework receipt/render evidence"
                .to_owned(),
        ));
    }
    Ok(allowed)
}

fn retention_ref_requires_artifact_reference(role: events::ArtifactRole) -> bool {
    staged_artifact_binding_kind(role).is_some()
}

fn retention_ref_key(retention_ref: &events::RetentionRef) -> RetentionRefKey {
    (
        retention_ref.artifact_id.clone(),
        retention_ref.content_digest.clone(),
        retention_ref.role,
    )
}

fn event_artifact_ref_key(artifact: &events::ArtifactEvidenceRef) -> RetentionRefKey {
    (
        artifact.artifact_id.clone(),
        artifact.content_digest.clone(),
        artifact.role,
    )
}

fn same_commit_artifact_reference_keys(
    commit: &[store::KernelEventEnvelope],
) -> BTreeSet<RetentionRefKey> {
    commit
        .iter()
        .filter_map(|event| match event.payload() {
            events::KernelEventPayload::ArtifactReferenced(payload) => {
                Some(event_artifact_ref_key(&payload.artifact_ref))
            }
            _ => None,
        })
        .collect()
}

fn same_commit_typed_artifact_keys(
    commit: &[store::KernelEventEnvelope],
) -> BTreeSet<RetentionRefKey> {
    let mut keys = BTreeSet::new();
    for event in commit {
        match event.payload() {
            events::KernelEventPayload::CellProduced(payload) => {
                keys.insert((
                    payload.artifact_id.clone(),
                    payload.content_digest.clone(),
                    events::ArtifactRole::StateOutput,
                ));
            }
            events::KernelEventPayload::FactRecorded(payload) => {
                keys.insert((
                    payload.artifact_id.clone(),
                    payload.response_hash.clone(),
                    events::ArtifactRole::FactResponse,
                ));
            }
            events::KernelEventPayload::PublicOutputProduced(payload) => {
                if let Some(artifact_id) = &payload.rendered_artifact_id {
                    keys.insert((
                        artifact_id.clone(),
                        payload.rendered_digest.clone(),
                        events::ArtifactRole::PublicOutput,
                    ));
                }
            }
            events::KernelEventPayload::PublicOutputRenderFailed(payload) => {
                insert_error_diagnostic_key(&payload.error, &mut keys);
            }
            events::KernelEventPayload::StateAttemptFailed(payload) => {
                insert_error_diagnostic_key(&payload.error, &mut keys);
            }
            events::KernelEventPayload::SideEffectIntentPersisted(payload) => {
                keys.insert((
                    payload.intent_artifact_id.clone(),
                    payload.intent_hash.clone(),
                    events::ArtifactRole::SideEffectIntent,
                ));
            }
            events::KernelEventPayload::SideEffectInvocationPrepared(payload) => {
                if let (Some(artifact_id), Some(content_digest)) =
                    (&payload.prepared_artifact_id, &payload.prepared_hash)
                {
                    keys.insert((
                        artifact_id.clone(),
                        content_digest.clone(),
                        events::ArtifactRole::PreparedInvocation,
                    ));
                }
            }
            events::KernelEventPayload::SideEffectNotSubmittedProven(payload) => {
                keys.insert((
                    payload.proof_artifact_id.clone(),
                    payload.proof_hash.clone(),
                    events::ArtifactRole::NotSubmittedProof,
                ));
            }
            events::KernelEventPayload::SideEffectSubmissionObserved(payload) => {
                keys.insert((
                    payload.submission_artifact_id.clone(),
                    payload.submission_hash.clone(),
                    events::ArtifactRole::Submission,
                ));
            }
            events::KernelEventPayload::SideEffectSubmissionUnknown(payload) => {
                keys.insert((
                    payload.evidence_artifact_id.clone(),
                    payload.evidence_hash.clone(),
                    events::ArtifactRole::SubmissionUnknownEvidence,
                ));
            }
            events::KernelEventPayload::SideEffectReceiptObserved(payload) => {
                keys.insert((
                    payload.receipt_artifact_id.clone(),
                    payload.receipt_hash.clone(),
                    events::ArtifactRole::Receipt,
                ));
            }
            events::KernelEventPayload::SideEffectConfirmationObserved(payload) => {
                keys.insert((
                    payload.confirmation_artifact_id.clone(),
                    payload.confirmation_hash.clone(),
                    events::ArtifactRole::Confirmation,
                ));
            }
            events::KernelEventPayload::SideEffectAmbiguous(payload) => {
                keys.insert((
                    payload.evidence_artifact_id.clone(),
                    payload.evidence_hash.clone(),
                    events::ArtifactRole::AmbiguityEvidence,
                ));
            }
            events::KernelEventPayload::SideEffectFailed(payload) => {
                insert_error_diagnostic_key(&payload.error, &mut keys);
            }
            _ => {}
        }
    }
    keys
}

fn insert_error_diagnostic_key(error: &events::MfmErrorInfo, keys: &mut BTreeSet<RetentionRefKey>) {
    if let Some(diagnostic) = &error.diagnostic_ref {
        keys.insert(event_artifact_ref_key(diagnostic));
    }
}

fn run_started_retention_ref_keys(
    run_started: &events::RunStarted,
    commit: &[store::KernelEventEnvelope],
) -> BTreeSet<RetentionRefKey> {
    let mut keys = BTreeSet::new();
    keys.insert((
        run_started.spec_artifact_id.clone(),
        ContentDigest::from_digest(
            run_started.spec_hash.algorithm(),
            *run_started.spec_hash.digest(),
        ),
        events::ArtifactRole::TypedExecutionSpec,
    ));
    keys.insert((
        run_started.certificate_artifact_id.clone(),
        run_started.certificate_artifact_digest.clone(),
        events::ArtifactRole::TypedSpecCertificate,
    ));
    for seed in &run_started.seed_cells {
        keys.insert(event_artifact_ref_key(&seed.seed_artifact));
    }
    for event in commit {
        if let events::KernelEventPayload::ArtifactReferenced(payload) = event.payload() {
            if payload.artifact_ref.role == events::ArtifactRole::TypedConfig {
                keys.insert(event_artifact_ref_key(&payload.artifact_ref));
            }
        }
    }
    keys
}

fn validate_historical_retention_manifest_batch(
    runtime_spec: &CertifiedRuntimeSpec,
    pre_projection_stream: &[store::KernelEventEnvelope],
    commit: &[store::KernelEventEnvelope],
) -> Result<()> {
    let projections = commit
        .iter()
        .filter_map(|event| match event.payload() {
            events::KernelEventPayload::RetentionManifestProjected(payload) => Some(payload),
            _ => None,
        })
        .collect::<Vec<_>>();
    if projections.is_empty() {
        return Ok(());
    }
    if projections.len() != 1 {
        return Err(RuntimeError::InvalidRunStream(
            "retention manifest projection commit contains multiple manifest projections"
                .to_owned(),
        ));
    }
    let projection = projections[0];
    let retention_node = certified_retention_manifest_node(runtime_spec)?;
    if projection.spec_hash != *runtime_spec.spec_hash() {
        return Err(RuntimeError::InvalidRunStream(
            "retention manifest projection spec hash does not match certified runtime spec"
                .to_owned(),
        ));
    }
    let expected = build_retention_manifest_artifact_with_producer(
        runtime_spec,
        &projection.run_id,
        pre_projection_stream,
        Some(retention_node.node_id.clone()),
    )?;
    if projection.manifest_seq != expected.manifest_seq
        || projection.manifest_digest != expected.evidence.digest
        || projection.previous_manifest_digest != expected.previous_manifest_digest
        || projection.manifest_artifact_id != expected.evidence.artifact_id
    {
        return Err(RuntimeError::InvalidRunStream(
            "retention manifest projection does not match the authoritative pre-projection stream"
                .to_owned(),
        ));
    }

    let receipt_bytes = retention_manifest_receipt_json(&expected, pre_projection_stream)?;
    let receipt_digest = receipt_bytes.content_digest();
    let receipt_artifact_id =
        ArtifactId::from_digest(receipt_digest.algorithm(), *receipt_digest.digest());
    let produced = commit
        .iter()
        .filter_map(|event| match event.payload() {
            events::KernelEventPayload::CellProduced(payload)
                if payload.node_id == retention_node.node_id =>
            {
                Some(payload)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    if produced.len() != 1 {
        return Err(RuntimeError::InvalidRunStream(
            "retention manifest projection was not produced by exactly one framework retention receipt cell"
                .to_owned(),
        ));
    }
    let produced = produced[0];
    if produced.spec_hash != *runtime_spec.spec_hash()
        || produced.cell_id != retention_node.output_cell
        || produced.artifact_id != receipt_artifact_id
        || produced.content_digest != receipt_digest
    {
        return Err(RuntimeError::InvalidRunStream(
            "retention manifest receipt cell does not match the projected manifest".to_owned(),
        ));
    }

    let started_count = commit
        .iter()
        .filter(|event| {
            matches!(
                event.payload(),
                events::KernelEventPayload::StateAttemptStarted(payload)
                    if payload.node_id == retention_node.node_id
                        && payload.attempt_id == produced.attempt_id
                        && payload.spec_hash == *runtime_spec.spec_hash()
                        && payload.state_kind == retention_node.state_kind
                        && payload.state_version == retention_node.state_version
            )
        })
        .count();
    if started_count != 1 {
        return Err(RuntimeError::InvalidRunStream(
            "retention manifest projection lacks matching framework StateAttemptStarted in the same commit"
                .to_owned(),
        ));
    }

    let completed_count = commit
        .iter()
        .filter(|event| {
            matches!(
                event.payload(),
                events::KernelEventPayload::StateAttemptCompleted(payload)
                    if payload.node_id == retention_node.node_id
                        && payload.attempt_id == produced.attempt_id
                        && payload.output_cell_id == retention_node.output_cell
            )
        })
        .count();
    if completed_count != 1 {
        return Err(RuntimeError::InvalidRunStream(
            "retention manifest projection lacks matching framework StateAttemptCompleted in the same commit"
                .to_owned(),
        ));
    }

    let refs = commit
        .iter()
        .filter_map(|event| match event.payload() {
            events::KernelEventPayload::RetentionRefsAppended(payload) => Some(payload),
            _ => None,
        })
        .collect::<Vec<_>>();
    let manifest_refs = refs
        .iter()
        .copied()
        .filter(|payload| payload.reason == events::RetentionReason::ManifestProjection)
        .collect::<Vec<_>>();
    if manifest_refs.len() != 1 {
        return Err(RuntimeError::InvalidRunStream(
            "retention manifest projection commit must contain exactly one manifest retention refs append"
                .to_owned(),
        ));
    }
    let manifest_refs = manifest_refs[0];
    if manifest_refs.run_id != projection.run_id
        || manifest_refs.spec_hash != *runtime_spec.spec_hash()
        || manifest_refs.refs.len() != 1
    {
        return Err(RuntimeError::InvalidRunStream(
            "retention manifest projection retention refs are not sealed to manifest projection"
                .to_owned(),
        ));
    }
    let manifest_ref = &manifest_refs.refs[0];
    if manifest_ref.artifact_id != expected.evidence.artifact_id
        || manifest_ref.content_digest != expected.evidence.digest
        || manifest_ref.role != events::ArtifactRole::RetentionManifest
    {
        return Err(RuntimeError::InvalidRunStream(
            "retention manifest projection retention ref does not match the manifest artifact"
                .to_owned(),
        ));
    }

    let receipt_refs = refs
        .iter()
        .copied()
        .filter(|payload| payload.reason == events::RetentionReason::RuntimeEvidence)
        .collect::<Vec<_>>();
    if receipt_refs.len() != 1 {
        return Err(RuntimeError::InvalidRunStream(
            "retention manifest projection commit must retain its framework receipt artifact"
                .to_owned(),
        ));
    }
    let receipt_refs = receipt_refs[0];
    if receipt_refs.run_id != projection.run_id
        || receipt_refs.spec_hash != *runtime_spec.spec_hash()
        || receipt_refs.refs.len() != 1
    {
        return Err(RuntimeError::InvalidRunStream(
            "retention manifest receipt retention refs are not sealed to runtime evidence"
                .to_owned(),
        ));
    }
    let receipt_ref = &receipt_refs.refs[0];
    if receipt_ref.artifact_id != receipt_artifact_id
        || receipt_ref.content_digest != receipt_digest
        || receipt_ref.role != events::ArtifactRole::StateOutput
    {
        return Err(RuntimeError::InvalidRunStream(
            "retention manifest receipt retention ref does not match the receipt artifact"
                .to_owned(),
        ));
    }
    if refs.len() != 2 {
        return Err(RuntimeError::InvalidRunStream(
            "retention manifest projection commit contains unsupported retention refs".to_owned(),
        ));
    }
    validate_retention_projection_commit_payload_set(
        commit,
        &retention_node.node_id,
        &produced.attempt_id,
        &retention_node.output_cell,
        &receipt_artifact_id,
        &receipt_digest,
        &expected.evidence.artifact_id,
    )?;
    Ok(())
}

fn validate_retention_projection_commit_payload_set(
    commit: &[store::KernelEventEnvelope],
    retention_node_id: &NodeId,
    attempt_id: &AttemptId,
    receipt_cell_id: &CellId,
    receipt_artifact_id: &ArtifactId,
    receipt_digest: &ContentDigest,
    manifest_artifact_id: &ArtifactId,
) -> Result<()> {
    let mut started = 0_usize;
    let mut produced = 0_usize;
    let mut completed = 0_usize;
    let mut artifact_referenced = 0_usize;
    let mut manifest_projected = 0_usize;
    let mut retention_refs = 0_usize;

    for event in commit {
        match event.payload() {
            events::KernelEventPayload::StateAttemptStarted(payload)
                if payload.node_id == *retention_node_id && payload.attempt_id == *attempt_id =>
            {
                started += 1;
            }
            events::KernelEventPayload::CellProduced(payload)
                if payload.node_id == *retention_node_id
                    && payload.attempt_id == *attempt_id
                    && payload.cell_id == *receipt_cell_id
                    && payload.artifact_id == *receipt_artifact_id
                    && payload.content_digest == *receipt_digest =>
            {
                produced += 1;
            }
            events::KernelEventPayload::StateAttemptCompleted(payload)
                if payload.node_id == *retention_node_id
                    && payload.attempt_id == *attempt_id
                    && payload.output_cell_id == *receipt_cell_id =>
            {
                completed += 1;
            }
            events::KernelEventPayload::ArtifactReferenced(payload)
                if payload.node_id.as_ref() == Some(retention_node_id)
                    && payload.attempt_id.as_ref() == Some(attempt_id)
                    && payload.artifact_ref.artifact_id == *receipt_artifact_id
                    && payload.artifact_ref.content_digest == *receipt_digest
                    && payload.artifact_ref.role == events::ArtifactRole::StateOutput =>
            {
                artifact_referenced += 1;
            }
            events::KernelEventPayload::RetentionManifestProjected(payload)
                if payload.manifest_artifact_id == *manifest_artifact_id =>
            {
                manifest_projected += 1;
            }
            events::KernelEventPayload::RetentionRefsAppended(payload) => match payload.reason {
                events::RetentionReason::ManifestProjection
                    if payload
                        .refs
                        .iter()
                        .all(|reference| reference.artifact_id == *manifest_artifact_id) =>
                {
                    retention_refs += 1;
                }
                events::RetentionReason::RuntimeEvidence
                    if payload
                        .refs
                        .iter()
                        .all(|reference| reference.artifact_id == *receipt_artifact_id) =>
                {
                    retention_refs += 1;
                }
                _ => {
                    return Err(RuntimeError::InvalidRunStream(
                        "retention manifest projection commit contains unsupported payload"
                            .to_owned(),
                    ));
                }
            },
            _ => {
                return Err(RuntimeError::InvalidRunStream(
                    "retention manifest projection commit contains unsupported payload".to_owned(),
                ));
            }
        }
    }

    if started == 1
        && produced == 1
        && completed == 1
        && artifact_referenced == 1
        && manifest_projected == 1
        && retention_refs == 2
        && commit.len() == 7
    {
        Ok(())
    } else {
        Err(RuntimeError::InvalidRunStream(
            "retention manifest projection commit does not match the sealed framework batch"
                .to_owned(),
        ))
    }
}

fn validate_historical_complete_run_tail(
    runtime_spec: &CertifiedRuntimeSpec,
    run_id: &RunId,
    stream: &[store::KernelEventEnvelope],
) -> Result<()> {
    let completion_node = certified_complete_run_node(runtime_spec)?;
    let mut last_retention_commit_end = None::<usize>;
    let mut completion_commit = None::<(usize, usize)>;
    let mut index = 0;
    while index < stream.len() {
        let first = &stream[index];
        let seq = first.seq();
        let commit_key = first.commit_key().clone();
        let mut end = index + 1;
        while end < stream.len()
            && stream[end].seq() == seq
            && stream[end].commit_key() == &commit_key
        {
            end += 1;
        }
        let commit = &stream[index..end];
        let has_retention_projection = commit.iter().any(|event| {
            matches!(
                event.payload(),
                events::KernelEventPayload::RetentionManifestProjected(_)
            )
        });
        if has_retention_projection {
            last_retention_commit_end = Some(end);
        }
        let has_completion_node_payload = commit
            .iter()
            .any(|event| payload_targets_node(event.payload(), &completion_node.node_id));
        let completion_count = commit
            .iter()
            .filter(|event| matches!(event.payload(), events::KernelEventPayload::RunCompleted(_)))
            .count();
        if has_completion_node_payload || completion_count > 0 {
            if completion_commit.replace((index, end)).is_some() {
                return Err(RuntimeError::InvalidRunStream(
                    "run stream contains multiple CompleteRun commits".to_owned(),
                ));
            }
            if completion_count == 0 {
                return Err(RuntimeError::InvalidRunStream(
                    "CompleteRun commit is missing RunCompleted payload".to_owned(),
                ));
            }
            if completion_count != 1 {
                return Err(RuntimeError::InvalidRunStream(
                    "CompleteRun commit contains multiple RunCompleted payloads".to_owned(),
                ));
            }
        }
        index = end;
    }

    match (last_retention_commit_end, completion_commit) {
        (Some(retention_end), Some((completion_start, completion_end)))
            if retention_end == completion_start =>
        {
            validate_historical_complete_run_batch(
                runtime_spec,
                run_id,
                &stream[..completion_start],
                &stream[completion_start..completion_end],
            )
        }
        (Some(_), Some(_)) => Err(RuntimeError::InvalidRunStream(
            "run stream contains events after retention manifest projection outside sealed CompleteRun commit"
                .to_owned(),
        )),
        (Some(retention_end), None) if retention_end == stream.len() => Ok(()),
        (Some(_), None) => Err(RuntimeError::InvalidRunStream(
            "run stream contains events after retention manifest projection outside sealed CompleteRun commit"
                .to_owned(),
        )),
        (None, Some(_)) => Err(RuntimeError::InvalidRunStream(
            "RunCompleted appeared before retention manifest projection".to_owned(),
        )),
        (None, None) => Ok(()),
    }
}

fn payload_targets_node(payload: &events::KernelEventPayload, node_id: &NodeId) -> bool {
    match payload {
        events::KernelEventPayload::StateAttemptStarted(payload) => payload.node_id == *node_id,
        events::KernelEventPayload::StateAttemptCompleted(payload) => payload.node_id == *node_id,
        events::KernelEventPayload::StateAttemptFailed(payload) => payload.node_id == *node_id,
        events::KernelEventPayload::CellProduced(payload) => payload.node_id == *node_id,
        events::KernelEventPayload::CellSkipped(payload) => payload.node_id == *node_id,
        events::KernelEventPayload::FactRecorded(payload) => payload.node_id == *node_id,
        events::KernelEventPayload::ArtifactReferenced(payload) => {
            payload.node_id.as_ref() == Some(node_id)
        }
        events::KernelEventPayload::PublicOutputProduced(payload) => payload.node_id == *node_id,
        events::KernelEventPayload::PublicOutputRenderFailed(payload) => {
            payload.node_id == *node_id
        }
        payload => side_effect_payload_ref(payload)
            .map(|(payload_node_id, _, _, _)| payload_node_id == node_id)
            .unwrap_or(false),
    }
}

fn validate_historical_complete_run_batch(
    runtime_spec: &CertifiedRuntimeSpec,
    run_id: &RunId,
    pre_completion_stream: &[store::KernelEventEnvelope],
    commit: &[store::KernelEventEnvelope],
) -> Result<()> {
    let completion_payload = commit
        .iter()
        .find_map(|event| match event.payload() {
            events::KernelEventPayload::RunCompleted(payload) => Some(payload),
            _ => None,
        })
        .expect("caller checked completion count");
    let completion_node = certified_complete_run_node(runtime_spec)?;
    let pre_completion_projection =
        store::ProjectionSnapshot::rebuild_from_run_stream(pre_completion_stream)?;
    let completion = run_completion_evidence(runtime_spec, &pre_completion_projection)?;
    let retention_manifest = projected_retention_manifest(run_id, &pre_completion_projection)?;
    if completion_payload.run_id != *run_id
        || completion_payload.spec_hash != *runtime_spec.spec_hash()
        || completion_payload.outcome != events::RunCompletionOutcome::Completed(completion.clone())
    {
        return Err(RuntimeError::InvalidRunStream(
            "RunCompleted payload does not match sealed CompleteRun evidence".to_owned(),
        ));
    }

    let receipt_bytes =
        complete_run_receipt_json(&completion, retention_manifest, pre_completion_stream)?;
    let receipt_digest = receipt_bytes.content_digest();
    let receipt_artifact_id =
        ArtifactId::from_digest(receipt_digest.algorithm(), *receipt_digest.digest());
    let produced = commit
        .iter()
        .filter_map(|event| match event.payload() {
            events::KernelEventPayload::CellProduced(payload)
                if payload.node_id == completion_node.node_id =>
            {
                Some(payload)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    if produced.len() != 1 {
        return Err(RuntimeError::InvalidRunStream(
            "CompleteRun commit was not produced by exactly one framework completion receipt cell"
                .to_owned(),
        ));
    }
    let produced = produced[0];
    if produced.spec_hash != *runtime_spec.spec_hash()
        || produced.cell_id != completion_node.output_cell
        || produced.artifact_id != receipt_artifact_id
        || produced.content_digest != receipt_digest
    {
        return Err(RuntimeError::InvalidRunStream(
            "CompleteRun receipt cell does not match the sealed completion evidence".to_owned(),
        ));
    }
    let completion_cell = runtime_spec
        .cell(&completion_node.output_cell)
        .ok_or_else(|| {
            RuntimeError::InvalidSpec(format!(
                "completion lifecycle node {} output cell {} is missing",
                completion_node.node_id, completion_node.output_cell
            ))
        })?;
    let expected_receipt_ref = events::ArtifactEvidenceRef {
        artifact_id: receipt_artifact_id.clone(),
        role: events::ArtifactRole::StateOutput,
        schema_id: completion_cell.schema_id.clone(),
        semantic_type_id: Some(completion_cell.semantic_type_id.clone()),
        content_digest: receipt_digest.clone(),
        byte_len: receipt_bytes.as_bytes().len() as u64,
        media_type: spec::MediaType::new("application/json")?,
    };

    validate_complete_run_commit_payload_set(
        commit,
        run_id,
        runtime_spec.spec_hash(),
        &completion,
        &completion_node.node_id,
        &produced.attempt_id,
        &completion_node.output_cell,
        &receipt_artifact_id,
        &receipt_digest,
        &expected_receipt_ref,
    )
}

fn validate_complete_run_commit_payload_set(
    commit: &[store::KernelEventEnvelope],
    run_id: &RunId,
    spec_hash: &SpecHash,
    completion: &events::PublicOutputCompletionEvidence,
    completion_node_id: &NodeId,
    attempt_id: &AttemptId,
    receipt_cell_id: &CellId,
    receipt_artifact_id: &ArtifactId,
    receipt_digest: &ContentDigest,
    expected_receipt_ref: &events::ArtifactEvidenceRef,
) -> Result<()> {
    if commit.len() != 5 {
        return Err(RuntimeError::InvalidRunStream(
            "CompleteRun commit does not match the sealed framework batch".to_owned(),
        ));
    }
    match commit[0].payload() {
        events::KernelEventPayload::StateAttemptStarted(payload)
            if payload.node_id == *completion_node_id && payload.attempt_id == *attempt_id => {}
        _ => {
            return Err(RuntimeError::InvalidRunStream(
                "CompleteRun commit does not match the sealed framework batch".to_owned(),
            ));
        }
    }
    match commit[1].payload() {
        events::KernelEventPayload::CellProduced(payload)
            if payload.node_id == *completion_node_id
                && payload.attempt_id == *attempt_id
                && payload.cell_id == *receipt_cell_id
                && payload.artifact_id == *receipt_artifact_id
                && payload.content_digest == *receipt_digest => {}
        _ => {
            return Err(RuntimeError::InvalidRunStream(
                "CompleteRun commit does not match the sealed framework batch".to_owned(),
            ));
        }
    }
    match commit[2].payload() {
        events::KernelEventPayload::StateAttemptCompleted(payload)
            if payload.node_id == *completion_node_id
                && payload.attempt_id == *attempt_id
                && payload.output_cell_id == *receipt_cell_id => {}
        _ => {
            return Err(RuntimeError::InvalidRunStream(
                "CompleteRun commit does not match the sealed framework batch".to_owned(),
            ));
        }
    }
    match commit[3].payload() {
        events::KernelEventPayload::ArtifactReferenced(payload)
            if payload.node_id.as_ref() == Some(completion_node_id)
                && payload.attempt_id.as_ref() == Some(attempt_id)
                && payload.artifact_ref == *expected_receipt_ref => {}
        _ => {
            return Err(RuntimeError::InvalidRunStream(
                "CompleteRun commit does not match the sealed framework batch".to_owned(),
            ));
        }
    }
    match commit[4].payload() {
        events::KernelEventPayload::RunCompleted(payload)
            if payload.run_id == *run_id
                && payload.spec_hash == *spec_hash
                && payload.outcome
                    == events::RunCompletionOutcome::Completed(completion.clone()) =>
        {
            Ok(())
        }
        _ => Err(RuntimeError::InvalidRunStream(
            "CompleteRun commit does not match the sealed framework batch".to_owned(),
        )),
    }
}

fn certified_retention_manifest_node(
    runtime_spec: &CertifiedRuntimeSpec,
) -> Result<&spec::NodeSpec> {
    let mut retention_node = None;
    for node_id in runtime_spec.topological_order() {
        let node = runtime_spec.node(node_id).expect("topological node exists");
        if matches!(
            &node.framework,
            Some(spec::FrameworkNodeSpec::ProjectRetentionManifest(_))
        ) {
            if retention_node.replace(node).is_some() {
                return Err(RuntimeError::InvalidRunStream(
                    "multiple certified retention manifest framework nodes".to_owned(),
                ));
            }
        }
    }
    retention_node.ok_or_else(|| {
        RuntimeError::InvalidRunStream(
            "retention manifest projection lacks a certified framework retention node".to_owned(),
        )
    })
}

fn certified_complete_run_node(runtime_spec: &CertifiedRuntimeSpec) -> Result<&spec::NodeSpec> {
    let mut completion_node = None;
    for node_id in runtime_spec.topological_order() {
        let node = runtime_spec.node(node_id).expect("topological node exists");
        if matches!(
            &node.framework,
            Some(spec::FrameworkNodeSpec::CompleteRun(_))
        ) {
            if completion_node.replace(node).is_some() {
                return Err(RuntimeError::InvalidRunStream(
                    "multiple certified completion framework nodes".to_owned(),
                ));
            }
        }
    }
    completion_node.ok_or_else(|| {
        RuntimeError::InvalidRunStream(
            "RunCompleted lacks a certified CompleteRun framework node".to_owned(),
        )
    })
}

fn validate_atomic_side_effect_failure_pairs(
    runtime_spec: &CertifiedRuntimeSpec,
    stream: &[store::KernelEventEnvelope],
) -> Result<()> {
    let mut side_effect_failures = BTreeMap::new();
    let mut attempt_failures = BTreeMap::new();
    for event in stream {
        match event.payload() {
            events::KernelEventPayload::SideEffectFailed(payload) => {
                side_effect_failures.insert(
                    (
                        event.seq(),
                        payload.node_id.clone(),
                        payload.attempt_id.clone(),
                    ),
                    payload.retryable,
                );
            }
            events::KernelEventPayload::StateAttemptFailed(payload) => {
                let node = runtime_spec.node(&payload.node_id).ok_or_else(|| {
                    RuntimeError::InvalidRunStream(format!(
                        "attempt failed for uncertified node {}",
                        payload.node_id
                    ))
                })?;
                if node.side_effect.is_some() {
                    attempt_failures.insert(
                        (
                            event.seq(),
                            payload.node_id.clone(),
                            payload.attempt_id.clone(),
                        ),
                        payload.retryable,
                    );
                }
            }
            _ => {}
        }
    }
    for (failure, side_effect_retryable) in &side_effect_failures {
        match attempt_failures.get(failure) {
            Some(attempt_retryable) if attempt_retryable == side_effect_retryable => {}
            Some(_) => {
                return Err(RuntimeError::InvalidRunStream(format!(
                    "side-effect failure for node {} attempt {} disagrees with StateAttemptFailed retryability",
                    failure.1, failure.2
                )));
            }
            None => {
                return Err(RuntimeError::InvalidRunStream(format!(
                    "side-effect failure for node {} attempt {} lacks StateAttemptFailed in the same commit",
                    failure.1, failure.2
                )));
            }
        }
    }
    for failure in attempt_failures.keys() {
        if !side_effect_failures.contains_key(failure) {
            return Err(RuntimeError::InvalidRunStream(format!(
                "side-effect attempt failure for node {} attempt {} lacks SideEffectFailed in the same commit",
                failure.1, failure.2
            )));
        }
    }
    Ok(())
}

fn validate_recovery_frontier(
    runtime_spec: &CertifiedRuntimeSpec,
    projections: &store::ProjectionSnapshot,
) -> Result<()> {
    for node_id in runtime_spec.topological_order() {
        let node = runtime_spec.node(node_id).expect("topological node exists");
        if let Some(terminal) = projections.cell_terminal(&node.output_cell) {
            let attempt_id = validate_terminal_cell_has_completed_attempt(
                runtime_spec,
                projections,
                node,
                terminal,
            )?;
            if node.side_effect.is_some() {
                validate_side_effect_terminal_evidence(projections, node, &attempt_id)?;
            }
        }
        let mut started = None;
        for ((attempt_node_id, attempt_id), projection) in projections.attempts() {
            if attempt_node_id != &node.node_id {
                continue;
            }
            match &projection.status {
                store::AttemptStatus::Started { .. } => {
                    if projections.cell_terminal(&node.output_cell).is_some() {
                        return Err(RuntimeError::InvalidRunStream(format!(
                            "node {} has a started attempt after its output cell became terminal",
                            node.node_id
                        )));
                    }
                    if started.replace(attempt_id.clone()).is_some() {
                        return Err(RuntimeError::InvalidRunStream(format!(
                            "node {} has multiple started attempts during recovery",
                            node.node_id
                        )));
                    }
                }
                store::AttemptStatus::Completed { output_cell_id }
                    if projections.cell_terminal(output_cell_id).is_none() =>
                {
                    return Err(RuntimeError::InvalidRunStream(format!(
                        "node {} attempt {} completed without terminal cell projection",
                        node.node_id, attempt_id
                    )));
                }
                store::AttemptStatus::Completed { .. } | store::AttemptStatus::Failed { .. } => {}
            }
        }
    }
    Ok(())
}

fn require_projected_attempt(
    projections: &store::ProjectionSnapshot,
    node_id: &NodeId,
    attempt_id: &AttemptId,
    event_kind: &'static str,
) -> Result<store::AttemptStatus> {
    let attempt = projections.attempt(node_id, attempt_id).ok_or_else(|| {
        RuntimeError::InvalidRunStream(format!(
            "{event_kind} event references missing attempt {attempt_id} for node {node_id}"
        ))
    })?;
    Ok(attempt.status.clone())
}

fn validate_historical_produced_cell(
    runtime_spec: &CertifiedRuntimeSpec,
    projections: &store::ProjectionSnapshot,
    event: &store::KernelEventEnvelope,
    payload: &events::CellProduced,
) -> Result<()> {
    let cell = runtime_spec.cell(&payload.cell_id).ok_or_else(|| {
        RuntimeError::InvalidRunStream(format!("produced uncertified cell {}", payload.cell_id))
    })?;
    let node = runtime_spec.node(&payload.node_id).ok_or_else(|| {
        RuntimeError::InvalidRunStream(format!(
            "cell {} was produced by uncertified node {}",
            payload.cell_id, payload.node_id
        ))
    })?;
    if node.output_cell != payload.cell_id
        || cell.producer != spec::CellProducer::Node(payload.node_id.clone())
        || cell.scope_id != payload.scope_id
        || cell.schema_id != payload.schema_id
        || cell.semantic_type_id != payload.semantic_type_id
        || cell.value_lineage != payload.value_lineage
    {
        return Err(RuntimeError::InvalidRunStream(format!(
            "produced cell {} does not match certified cell metadata",
            payload.cell_id
        )));
    }
    match projections.cell_terminal(&payload.cell_id) {
        Some(store::CellTerminalProjection::Produced {
            event_id,
            node_id,
            attempt_id,
            schema_id,
            semantic_type_id,
            artifact_id,
            content_digest,
        }) if event_id == event.event_id()
            && node_id == &payload.node_id
            && attempt_id == &payload.attempt_id
            && schema_id == &payload.schema_id
            && semantic_type_id == &payload.semantic_type_id
            && artifact_id == &payload.artifact_id
            && content_digest == &payload.content_digest =>
        {
            Ok(())
        }
        _ => Err(RuntimeError::InvalidRunStream(format!(
            "produced cell {} projection does not match authoritative event",
            payload.cell_id
        ))),
    }
}

fn validate_historical_skipped_cell(
    runtime_spec: &CertifiedRuntimeSpec,
    projections: &store::ProjectionSnapshot,
    event: &store::KernelEventEnvelope,
    payload: &events::CellSkipped,
) -> Result<()> {
    let cell = runtime_spec.cell(&payload.cell_id).ok_or_else(|| {
        RuntimeError::InvalidRunStream(format!("skipped uncertified cell {}", payload.cell_id))
    })?;
    let node = runtime_spec.node(&payload.node_id).ok_or_else(|| {
        RuntimeError::InvalidRunStream(format!(
            "cell {} was skipped by uncertified node {}",
            payload.cell_id, payload.node_id
        ))
    })?;
    if node.output_cell != payload.cell_id
        || cell.producer != spec::CellProducer::Node(payload.node_id.clone())
        || cell.scope_id != payload.scope_id
        || cell.schema_id != payload.schema_id
        || cell.semantic_type_id != payload.semantic_type_id
        || cell.value_lineage != payload.value_lineage
        || cell.terminal_policy == spec::CellTerminalPolicy::ProducedOnly
    {
        return Err(RuntimeError::InvalidRunStream(format!(
            "skipped cell {} does not match certified cell metadata",
            payload.cell_id
        )));
    }
    match projections.cell_terminal(&payload.cell_id) {
        Some(store::CellTerminalProjection::Skipped {
            event_id,
            node_id,
            attempt_id,
            schema_id,
            semantic_type_id,
            skip_reason,
        }) if event_id == event.event_id()
            && node_id == &payload.node_id
            && attempt_id == &payload.attempt_id
            && schema_id == &payload.schema_id
            && semantic_type_id == &payload.semantic_type_id
            && skip_reason == &payload.skip_reason =>
        {
            Ok(())
        }
        _ => Err(RuntimeError::InvalidRunStream(format!(
            "skipped cell {} projection does not match authoritative event",
            payload.cell_id
        ))),
    }
}

fn validate_historical_public_output_produced(
    runtime_spec: &CertifiedRuntimeSpec,
    projections: &store::ProjectionSnapshot,
    event: &store::KernelEventEnvelope,
    node: &spec::NodeSpec,
    payload: &events::PublicOutputProduced,
) -> Result<()> {
    match require_projected_attempt(
        projections,
        &payload.node_id,
        &payload.attempt_id,
        "public output",
    )? {
        store::AttemptStatus::Completed { output_cell_id }
            if output_cell_id == node.output_cell => {}
        _ => {
            return Err(RuntimeError::InvalidRunStream(format!(
                "public output for node {} attempt {} is not backed by a completed render attempt",
                payload.node_id, payload.attempt_id
            )));
        }
    }
    if projections.cell_terminal(&node.output_cell).is_none() {
        return Err(RuntimeError::InvalidRunStream(format!(
            "public output for node {} has no terminal render receipt cell",
            payload.node_id
        )));
    }
    for cell in &payload.cells {
        let certified = runtime_spec.cell(&cell.cell_id).ok_or_else(|| {
            RuntimeError::InvalidRunStream(format!(
                "public output references uncertified cell {}",
                cell.cell_id
            ))
        })?;
        match projections.cell_terminal(&cell.cell_id) {
            Some(store::CellTerminalProjection::Produced {
                schema_id,
                semantic_type_id,
                artifact_id,
                content_digest,
                ..
            }) if schema_id == &cell.schema_id
                && semantic_type_id == &cell.semantic_type_id
                && artifact_id == &cell.artifact_id
                && content_digest == &cell.content_digest
                && certified.producer == cell.producer
                && certified.scope_id == cell.scope_id
                && certified.value_lineage == cell.value_lineage => {}
            _ => {
                return Err(RuntimeError::InvalidRunStream(format!(
                    "public output source cell {} is not backed by matching terminal evidence",
                    cell.cell_id
                )));
            }
        }
    }
    let Some(spec::FrameworkNodeSpec::PublicOutputRender(render)) = &node.framework else {
        return Err(RuntimeError::InvalidRunStream(format!(
            "public output for node {} is not backed by a render node",
            node.node_id
        )));
    };
    let expected_rendered_digest = public_output_rendered_digest(render, &payload.cells)?;
    if payload.rendered_digest != expected_rendered_digest {
        return Err(RuntimeError::InvalidRunStream(format!(
            "public output for node {} carries a rendered digest that does not match certified cells",
            node.node_id
        )));
    }
    let expected_receipt_digest = public_output_receipt_digest(
        render,
        &payload.cells,
        &expected_rendered_digest,
        payload.rendered_artifact_id.as_ref(),
    )?;
    let expected_receipt_artifact_id = ArtifactId::from_digest(
        expected_receipt_digest.algorithm(),
        *expected_receipt_digest.digest(),
    );
    let output_cell = runtime_spec.cell(&node.output_cell).ok_or_else(|| {
        RuntimeError::InvalidRunStream(format!(
            "public output render node {} output cell {} is missing",
            node.node_id, node.output_cell
        ))
    })?;
    let receipt_schema_id = spec::public_output_receipt_schema_id()?;
    match projections.cell_terminal(&node.output_cell) {
        Some(store::CellTerminalProjection::Produced {
            schema_id,
            semantic_type_id,
            artifact_id,
            content_digest,
            ..
        }) if schema_id == &receipt_schema_id
            && semantic_type_id == &output_cell.semantic_type_id
            && artifact_id == &expected_receipt_artifact_id
            && content_digest == &expected_receipt_digest => {}
        _ => {
            return Err(RuntimeError::InvalidRunStream(format!(
                "public output for node {} has no matching terminal receipt cell",
                node.node_id
            )));
        }
    }
    match projections.public_output(&payload.public_schema_id) {
        Some(store::PublicOutputProjection::Produced {
            event_id,
            rendered_digest,
            rendered_artifact_id,
        }) if event_id == event.event_id()
            && rendered_digest == &payload.rendered_digest
            && rendered_artifact_id == &payload.rendered_artifact_id =>
        {
            Ok(())
        }
        _ => Err(RuntimeError::InvalidRunStream(format!(
            "public output projection for schema {} does not match authoritative event",
            payload.public_schema_id
        ))),
    }
}

fn validate_historical_run_completed(
    runtime_spec: &CertifiedRuntimeSpec,
    run_id: &RunId,
    event: &store::KernelEventEnvelope,
    payload: &events::RunCompleted,
    produced_public_output: Option<&events::PublicOutputCompletionEvidence>,
    retention_manifest_projected_seq: Option<store::StreamSeq>,
) -> Result<()> {
    if &payload.run_id != run_id {
        return Err(RuntimeError::InvalidRunStream(format!(
            "RunCompleted references run {} while validating {}",
            payload.run_id, run_id
        )));
    }
    let events::RunCompletionOutcome::Completed(completion) = &payload.outcome else {
        return Err(RuntimeError::InvalidRunStream(
            "RunCompleted failed/cancelled outcomes require sealed CompleteRun authority"
                .to_owned(),
        ));
    };
    if completion.public_output_schema_id != runtime_spec.spec().public_outputs.public_schema_id {
        return Err(RuntimeError::InvalidRunStream(format!(
            "RunCompleted references public schema {} outside certified public outputs",
            completion.public_output_schema_id
        )));
    }
    match produced_public_output {
        Some(produced) if produced == completion => Ok(()),
        Some(_) => Err(RuntimeError::InvalidRunStream(
            "RunCompleted public-output evidence does not match preceding PublicOutputProduced"
                .to_owned(),
        )),
        None => Err(RuntimeError::InvalidRunStream(
            "RunCompleted appeared before PublicOutputProduced".to_owned(),
        )),
    }?;
    match retention_manifest_projected_seq {
        Some(seq) if seq < event.seq() => Ok(()),
        Some(_) => Err(RuntimeError::InvalidRunStream(
            "RunCompleted must follow a prior retention manifest projection commit".to_owned(),
        )),
        None => Err(RuntimeError::InvalidRunStream(
            "RunCompleted appeared before retention manifest projection".to_owned(),
        )),
    }
}

fn validate_historical_public_output_failed(
    projections: &store::ProjectionSnapshot,
    payload: &events::PublicOutputRenderFailed,
) -> Result<()> {
    match require_projected_attempt(
        projections,
        &payload.node_id,
        &payload.attempt_id,
        "public output failure",
    )? {
        store::AttemptStatus::Failed { .. } => {}
        _ => {
            return Err(RuntimeError::InvalidRunStream(format!(
                "public output failure for node {} attempt {} is not backed by a failed render attempt",
                payload.node_id, payload.attempt_id
            )));
        }
    }
    match projections.public_output(&payload.public_schema_id) {
        Some(store::PublicOutputProjection::Produced { .. })
        | Some(store::PublicOutputProjection::RenderFailed { .. }) => Ok(()),
        _ => Err(RuntimeError::InvalidRunStream(format!(
            "public output failure for schema {} is not reflected in public-output projection",
            payload.public_schema_id
        ))),
    }
}

fn validate_spec_artifact(
    runtime_spec: &CertifiedRuntimeSpec,
    evidence: store::ArtifactEvidenceRef,
) -> Result<store::ArtifactEvidenceRef> {
    let canonical = runtime_spec
        .spec()
        .canonical_json()
        .map_err(|error| RuntimeError::Canonical(error.to_string()))?;
    let digest = canonical.content_digest();
    let expected_artifact_id = ArtifactId::from_digest(digest.algorithm(), *digest.digest());
    if evidence.artifact_id != expected_artifact_id
        || evidence.digest != digest
        || evidence.byte_len != canonical.as_bytes().len() as u64
        || evidence.media_type != runtime_spec.spec().media_type
        || evidence.schema_id.is_some()
        || evidence.semantic_type_id.is_some()
        || evidence.producer_node_id.is_some()
        || evidence.producer_seed_id.is_some()
        || evidence.artifact_role != events::ArtifactRole::TypedExecutionSpec
    {
        return Err(RuntimeError::InvalidRunStream(
            "typed execution spec artifact evidence does not match the certified spec".to_owned(),
        ));
    }
    Ok(evidence)
}

fn validate_certificate_artifact(
    runtime_spec: &CertifiedRuntimeSpec,
    evidence: store::ArtifactEvidenceRef,
) -> Result<store::ArtifactEvidenceRef> {
    let canonical = runtime_spec
        .certificate()
        .canonical_json()
        .map_err(|error| RuntimeError::Canonical(error.to_string()))?;
    let digest = canonical.content_digest();
    let expected_artifact_id = ArtifactId::from_digest(digest.algorithm(), *digest.digest());
    let media_type = spec::MediaType::new(mfm_certify::CERTIFICATE_MEDIA_TYPE)
        .map_err(|error| RuntimeError::Identity(error.to_string()))?;
    if evidence.artifact_id != expected_artifact_id
        || evidence.digest != digest
        || evidence.byte_len != canonical.as_bytes().len() as u64
        || evidence.media_type != media_type
        || evidence.schema_id.is_some()
        || evidence.semantic_type_id.is_some()
        || evidence.producer_node_id.is_some()
        || evidence.producer_seed_id.is_some()
        || evidence.artifact_role != events::ArtifactRole::TypedSpecCertificate
    {
        return Err(RuntimeError::InvalidRunStream(
            "typed spec certificate artifact evidence does not match the certified spec".to_owned(),
        ));
    }
    Ok(evidence)
}

fn validate_config_artifacts(
    runtime_spec: &CertifiedRuntimeSpec,
    evidence: Vec<store::ArtifactEvidenceRef>,
) -> Result<Vec<store::ArtifactEvidenceRef>> {
    let mut by_key = BTreeMap::new();
    for artifact in evidence {
        let Some(schema_id) = artifact.schema_id.clone() else {
            return Err(RuntimeError::InvalidRunStream(
                "typed config artifact evidence must carry schema_id".to_owned(),
            ));
        };
        if artifact.artifact_role != events::ArtifactRole::TypedConfig
            || artifact.semantic_type_id.is_some()
            || artifact.producer_node_id.is_some()
            || artifact.producer_seed_id.is_some()
        {
            return Err(RuntimeError::InvalidRunStream(
                "typed config artifact evidence has invalid role or producer metadata".to_owned(),
            ));
        }
        let key = format!("{}:{}", schema_id, artifact.digest);
        if by_key.insert(key, artifact).is_some() {
            return Err(RuntimeError::InvalidRunStream(
                "duplicate typed config artifact evidence".to_owned(),
            ));
        }
    }

    let mut validated = Vec::with_capacity(runtime_spec.spec().config_refs.len());
    for config in &runtime_spec.spec().config_refs {
        let key = config_ref_key(config);
        let artifact = by_key.remove(&key).ok_or_else(|| {
            RuntimeError::InvalidRunStream(format!(
                "missing typed config artifact evidence for schema {} digest {}",
                config.schema_id, config.digest
            ))
        })?;
        if artifact.artifact_id != config.artifact_id
            || artifact.digest != config.digest
            || artifact.byte_len != config.byte_len
            || artifact.media_type != config.media_type
            || artifact.schema_id.as_ref() != Some(&config.schema_id)
        {
            return Err(RuntimeError::InvalidRunStream(format!(
                "typed config artifact evidence for schema {} digest {} does not match certified config ref",
                config.schema_id, config.digest
            )));
        }
        validated.push(artifact);
    }
    if !by_key.is_empty() {
        return Err(RuntimeError::InvalidRunStream(
            "typed config artifact evidence contains entries not certified by the spec".to_owned(),
        ));
    }
    Ok(validated)
}

fn config_artifacts_from_stream(
    runtime_spec: &CertifiedRuntimeSpec,
    stream: &[store::KernelEventEnvelope],
) -> Result<BTreeMap<String, store::ArtifactEvidenceRef>> {
    let (start_seq, start_commit_key) = stream
        .iter()
        .find(|event| matches!(event.payload(), events::KernelEventPayload::RunStarted(_)))
        .map(|event| (event.seq(), event.commit_key().clone()))
        .ok_or_else(|| {
            RuntimeError::InvalidRunStream(
                "run has not started with certified RunStarted evidence".to_owned(),
            )
        })?;
    let mut referenced = Vec::new();
    for event in stream {
        let events::KernelEventPayload::ArtifactReferenced(payload) = event.payload() else {
            continue;
        };
        if payload.artifact_ref.role != events::ArtifactRole::TypedConfig {
            continue;
        }
        if payload.node_id.is_some() || payload.attempt_id.is_some() {
            return Err(RuntimeError::InvalidRunStream(
                "typed config artifact references must not be scoped to a runner attempt"
                    .to_owned(),
            ));
        }
        if event.seq() != start_seq || event.commit_key() != &start_commit_key {
            return Err(RuntimeError::InvalidRunStream(
                "typed config artifact references must be part of the RunStarted commit".to_owned(),
            ));
        }
        referenced.push(store_artifact_from_event_ref(
            &payload.artifact_ref,
            None,
            None,
        ));
    }
    let validated = validate_config_artifacts(runtime_spec, referenced)?;
    Ok(validated
        .into_iter()
        .map(|artifact| {
            let key = format!(
                "{}:{}",
                artifact
                    .schema_id
                    .as_ref()
                    .expect("validated config artifact has schema id"),
                artifact.digest
            );
            (key, artifact)
        })
        .collect())
}

fn artifact_refs_from_stream(
    stream: &[store::KernelEventEnvelope],
) -> Result<BTreeMap<ArtifactId, CommittedArtifactReference>> {
    let mut artifacts = BTreeMap::new();
    let mut index = 0;
    while index < stream.len() {
        let first = &stream[index];
        let seq = first.seq();
        let commit_key = first.commit_key().clone();
        let mut end = index + 1;
        while end < stream.len()
            && stream[end].seq() == seq
            && stream[end].commit_key() == &commit_key
        {
            end += 1;
        }
        let commit = &stream[index..end];
        for event in commit {
            match event.payload() {
                events::KernelEventPayload::RunStarted(payload) => {
                    for seed in &payload.seed_cells {
                        insert_committed_artifact(
                            &mut artifacts,
                            CommittedArtifactReference {
                                evidence: store_seed_artifact(seed),
                                attempt_id: None,
                                commit_seq: event.seq(),
                                commit_key: event.commit_key().clone(),
                            },
                        )?;
                    }
                }
                events::KernelEventPayload::ArtifactReferenced(payload)
                    if staged_artifact_binding_kind(payload.artifact_ref.role).is_some() =>
                {
                    let (Some(node_id), Some(attempt_id)) =
                        (payload.node_id.clone(), payload.attempt_id.clone())
                    else {
                        return Err(RuntimeError::InvalidRunStream(format!(
                            "staged artifact reference {} must be scoped to a producer attempt",
                            payload.artifact_ref.artifact_id
                        )));
                    };
                    if !artifact_reference_matches_same_commit_payload(commit, payload) {
                        return Err(RuntimeError::InvalidRunStream(format!(
                            "staged artifact reference {} is not bound to a same-commit typed payload",
                            payload.artifact_ref.artifact_id
                        )));
                    }
                    if payload.artifact_ref.role == events::ArtifactRole::StateOutput {
                        insert_committed_artifact(
                            &mut artifacts,
                            CommittedArtifactReference {
                                evidence: store_artifact_from_event_ref(
                                    &payload.artifact_ref,
                                    Some(node_id),
                                    None,
                                ),
                                attempt_id: Some(attempt_id),
                                commit_seq: event.seq(),
                                commit_key: event.commit_key().clone(),
                            },
                        )?;
                    }
                }
                events::KernelEventPayload::ArtifactReferenced(payload)
                    if payload.artifact_ref.role != events::ArtifactRole::TypedConfig =>
                {
                    return Err(RuntimeError::InvalidRunStream(format!(
                        "unsupported artifact reference role {} for {}",
                        artifact_role_name(payload.artifact_ref.role),
                        payload.artifact_ref.artifact_id
                    )));
                }
                _ => {}
            }
        }
        index = end;
    }
    Ok(artifacts)
}

fn artifact_reference_matches_same_commit_payload(
    commit: &[store::KernelEventEnvelope],
    reference: &events::ArtifactReferenced,
) -> bool {
    let (Some(reference_node_id), Some(reference_attempt_id)) =
        (&reference.node_id, &reference.attempt_id)
    else {
        return false;
    };
    commit.iter().any(|event| match event.payload() {
        events::KernelEventPayload::CellProduced(payload) => {
            reference.artifact_ref.role == events::ArtifactRole::StateOutput
                && &payload.node_id == reference_node_id
                && &payload.attempt_id == reference_attempt_id
                && payload.artifact_id == reference.artifact_ref.artifact_id
                && payload.content_digest == reference.artifact_ref.content_digest
                && payload.schema_id == reference.artifact_ref.schema_id
                && reference.artifact_ref.semantic_type_id.as_ref()
                    == Some(&payload.semantic_type_id)
        }
        events::KernelEventPayload::FactRecorded(payload) => {
            reference.artifact_ref.role == events::ArtifactRole::FactResponse
                && &payload.node_id == reference_node_id
                && &payload.attempt_id == reference_attempt_id
                && payload.artifact_id == reference.artifact_ref.artifact_id
                && payload.response_hash == reference.artifact_ref.content_digest
                && payload.response_schema_id == reference.artifact_ref.schema_id
        }
        events::KernelEventPayload::PublicOutputProduced(payload) => {
            reference.artifact_ref.role == events::ArtifactRole::PublicOutput
                && &payload.node_id == reference_node_id
                && &payload.attempt_id == reference_attempt_id
                && payload.rendered_artifact_id.as_ref()
                    == Some(&reference.artifact_ref.artifact_id)
                && payload.rendered_digest == reference.artifact_ref.content_digest
                && payload.public_schema_id == reference.artifact_ref.schema_id
        }
        events::KernelEventPayload::PublicOutputRenderFailed(payload) => {
            reference.artifact_ref.role == events::ArtifactRole::RedactedDiagnostic
                && &payload.node_id == reference_node_id
                && &payload.attempt_id == reference_attempt_id
                && payload
                    .error
                    .diagnostic_ref
                    .as_ref()
                    .is_some_and(|diagnostic| event_artifact_refs_match(diagnostic, reference))
        }
        events::KernelEventPayload::StateAttemptFailed(payload) => {
            reference.artifact_ref.role == events::ArtifactRole::RedactedDiagnostic
                && &payload.node_id == reference_node_id
                && &payload.attempt_id == reference_attempt_id
                && payload
                    .error
                    .diagnostic_ref
                    .as_ref()
                    .is_some_and(|diagnostic| event_artifact_refs_match(diagnostic, reference))
        }
        _ => false,
    })
}

fn event_artifact_refs_match(
    diagnostic: &events::ArtifactEvidenceRef,
    reference: &events::ArtifactReferenced,
) -> bool {
    diagnostic.artifact_id == reference.artifact_ref.artifact_id
        && diagnostic.role == reference.artifact_ref.role
        && diagnostic.schema_id == reference.artifact_ref.schema_id
        && diagnostic.semantic_type_id == reference.artifact_ref.semantic_type_id
        && diagnostic.content_digest == reference.artifact_ref.content_digest
        && diagnostic.byte_len == reference.artifact_ref.byte_len
        && diagnostic.media_type == reference.artifact_ref.media_type
}

fn insert_committed_artifact(
    artifacts: &mut BTreeMap<ArtifactId, CommittedArtifactReference>,
    artifact: CommittedArtifactReference,
) -> Result<()> {
    if let Some(existing) = artifacts.get(&artifact.evidence.artifact_id) {
        if existing != &artifact {
            return Err(RuntimeError::InvalidRunStream(format!(
                "conflicting committed artifact evidence for {}",
                artifact.evidence.artifact_id
            )));
        }
        return Ok(());
    }
    artifacts.insert(artifact.evidence.artifact_id.clone(), artifact);
    Ok(())
}

fn committed_config_artifact(
    node: &spec::NodeSpec,
    view: &RuntimeRunView,
) -> Result<store::ArtifactEvidenceRef> {
    let key = config_ref_key(&node.config_ref);
    let artifact = view.config_artifacts.get(&key).ok_or_else(|| {
        RuntimeError::InputMaterialization(format!(
            "node {} config artifact {} is not committed in the run stream",
            node.node_id, node.config_ref.artifact_id
        ))
    })?;
    if artifact.artifact_id != node.config_ref.artifact_id
        || artifact.digest != node.config_ref.digest
        || artifact.byte_len != node.config_ref.byte_len
        || artifact.media_type != node.config_ref.media_type
        || artifact.schema_id.as_ref() != Some(&node.config_ref.schema_id)
        || artifact.semantic_type_id.is_some()
        || artifact.producer_node_id.is_some()
        || artifact.producer_seed_id.is_some()
        || artifact.artifact_role != events::ArtifactRole::TypedConfig
    {
        return Err(RuntimeError::InputMaterialization(format!(
            "node {} committed config artifact evidence does not match certified config ref",
            node.node_id
        )));
    }
    Ok(artifact.clone())
}

fn committed_input_artifact(
    view: &RuntimeRunView,
    cell_id: &CellId,
    artifact_id: &ArtifactId,
) -> Result<CommittedArtifactReference> {
    view.artifact_refs.get(artifact_id).cloned().ok_or_else(|| {
        RuntimeError::InputMaterialization(format!(
            "input cell {cell_id} artifact {artifact_id} is not committed in the run stream",
        ))
    })
}

fn store_artifact_from_event_ref(
    evidence: &events::ArtifactEvidenceRef,
    producer_node_id: Option<NodeId>,
    producer_seed_id: Option<mfm_ids::SeedId>,
) -> store::ArtifactEvidenceRef {
    store::ArtifactEvidenceRef {
        artifact_id: evidence.artifact_id.clone(),
        digest: evidence.content_digest.clone(),
        byte_len: evidence.byte_len,
        media_type: evidence.media_type.clone(),
        schema_id: Some(evidence.schema_id.clone()),
        semantic_type_id: evidence.semantic_type_id.clone(),
        producer_node_id,
        producer_seed_id,
        artifact_role: evidence.role,
    }
}

fn validate_runner_output(
    runtime_spec: &CertifiedRuntimeSpec,
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    caps: &CertifiedRuntimeCapabilities,
    recorded_facts: &RecordedFacts,
    projections: &store::ProjectionSnapshot,
    output: &ErasedRunnerOutput,
) -> Result<()> {
    if output.payloads.is_empty() {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "runner for node {} returned no typed payloads",
            node.node_id
        )));
    }
    let mut completed = false;
    let mut failed = false;
    let mut terminal_cell = false;
    let mut public_output_produced = false;
    let mut public_output_failed = false;
    let mut side_effect_payload = false;
    let mut side_effect_failed = false;
    let mut attempt_failure_retryable = None;
    let mut side_effect_failure_retryable = None;
    for payload in &output.payloads {
        if payload_spec_hash(payload) != *runtime_spec.spec_hash() {
            return Err(RuntimeError::InvalidRunnerOutput(format!(
                "runner for node {} returned payload with mismatched spec hash",
                node.node_id
            )));
        }
        match payload {
            events::KernelEventPayload::StateAttemptCompleted(payload) => {
                require_attempt(node, attempt_id, &payload.node_id, &payload.attempt_id)?;
                if payload.output_cell_id != node.output_cell {
                    return Err(RuntimeError::InvalidRunnerOutput(format!(
                        "node {} completed output cell {} instead of certified {}",
                        node.node_id, payload.output_cell_id, node.output_cell
                    )));
                }
                completed = true;
            }
            events::KernelEventPayload::StateAttemptFailed(payload) => {
                require_attempt(node, attempt_id, &payload.node_id, &payload.attempt_id)?;
                if failed {
                    return Err(RuntimeError::InvalidRunnerOutput(format!(
                        "runner for node {} returned multiple failure payloads",
                        node.node_id
                    )));
                }
                attempt_failure_retryable = Some(payload.retryable);
                failed = true;
            }
            events::KernelEventPayload::CellProduced(payload) => {
                require_attempt(node, attempt_id, &payload.node_id, &payload.attempt_id)?;
                let cell = runtime_spec.cell(&payload.cell_id).ok_or_else(|| {
                    RuntimeError::InvalidRunnerOutput(format!(
                        "node {} produced uncertified cell {}",
                        node.node_id, payload.cell_id
                    ))
                })?;
                if payload.cell_id != node.output_cell
                    || cell.producer != spec::CellProducer::Node(node.node_id.clone())
                    || cell.scope_id != payload.scope_id
                    || cell.schema_id != payload.schema_id
                    || cell.semantic_type_id != payload.semantic_type_id
                    || cell.value_lineage != payload.value_lineage
                {
                    return Err(RuntimeError::InvalidRunnerOutput(format!(
                        "node {} produced cell metadata outside certified spec",
                        node.node_id
                    )));
                }
                terminal_cell = true;
            }
            events::KernelEventPayload::CellSkipped(payload) => {
                require_attempt(node, attempt_id, &payload.node_id, &payload.attempt_id)?;
                let cell = runtime_spec.cell(&payload.cell_id).ok_or_else(|| {
                    RuntimeError::InvalidRunnerOutput(format!(
                        "node {} skipped uncertified cell {}",
                        node.node_id, payload.cell_id
                    ))
                })?;
                if payload.cell_id != node.output_cell
                    || cell.producer != spec::CellProducer::Node(node.node_id.clone())
                    || cell.scope_id != payload.scope_id
                    || cell.schema_id != payload.schema_id
                    || cell.semantic_type_id != payload.semantic_type_id
                    || cell.value_lineage != payload.value_lineage
                    || cell.terminal_policy == spec::CellTerminalPolicy::ProducedOnly
                {
                    return Err(RuntimeError::InvalidRunnerOutput(format!(
                        "node {} skipped a cell outside certified skip policy",
                        node.node_id
                    )));
                }
                terminal_cell = true;
            }
            events::KernelEventPayload::FactRecorded(payload) => {
                require_attempt(node, attempt_id, &payload.node_id, &payload.attempt_id)?;
                if !recorded_facts.is_empty() {
                    return Err(RuntimeError::InvalidRunnerOutput(format!(
                        "node {} attempted to record fact {} after committed facts existed for the same attempt",
                        node.node_id, payload.fact_key
                    )));
                }
                require_capability(
                    caps,
                    &payload.capability_kind,
                    &payload.capability_version,
                    &node.node_id,
                )?;
                require_adapter(node, &payload.adapter_kind, &payload.adapter_version)?;
            }
            events::KernelEventPayload::ArtifactReferenced(payload) => {
                return Err(RuntimeError::InvalidRunnerOutput(format!(
                    "runner for node {} returned artifact reference payload for {}",
                    node.node_id, payload.artifact_ref.artifact_id
                )));
            }
            events::KernelEventPayload::PublicOutputProduced(payload) => {
                require_attempt(node, attempt_id, &payload.node_id, &payload.attempt_id)?;
                validate_public_output(runtime_spec, node, payload)?;
                public_output_produced = true;
            }
            events::KernelEventPayload::PublicOutputRenderFailed(payload) => {
                require_attempt(node, attempt_id, &payload.node_id, &payload.attempt_id)?;
                validate_public_output_render_node(
                    runtime_spec,
                    node,
                    payload.public_schema_id.clone(),
                    &payload.renderer_descriptor_id,
                )?;
                public_output_failed = true;
            }
            events::KernelEventPayload::RunStarted(_)
            | events::KernelEventPayload::RunCompleted(_)
            | events::KernelEventPayload::RetentionRefsAppended(_)
            | events::KernelEventPayload::RetentionManifestProjected(_)
            | events::KernelEventPayload::StateAttemptStarted(_) => {
                return Err(RuntimeError::InvalidRunnerOutput(format!(
                    "runner for node {} returned scheduler-owned payload",
                    node.node_id
                )));
            }
            events::KernelEventPayload::SideEffectIntentPersisted(_)
            | events::KernelEventPayload::SideEffectClaimed(_)
            | events::KernelEventPayload::SideEffectClaimTakenOver(_)
            | events::KernelEventPayload::SideEffectInvocationPrepared(_)
            | events::KernelEventPayload::SideEffectInvocationStarted(_)
            | events::KernelEventPayload::SideEffectNotSubmittedProven(_)
            | events::KernelEventPayload::SideEffectSubmissionObserved(_)
            | events::KernelEventPayload::SideEffectSubmissionUnknown(_)
            | events::KernelEventPayload::SideEffectReceiptObserved(_)
            | events::KernelEventPayload::SideEffectConfirmationObserved(_)
            | events::KernelEventPayload::SideEffectAmbiguous(_)
            | events::KernelEventPayload::SideEffectFailed(_) => {
                validate_runner_side_effect_payload(node, attempt_id, caps, projections, payload)?;
                side_effect_payload = true;
                if let events::KernelEventPayload::SideEffectFailed(payload) = payload {
                    side_effect_failed = true;
                    side_effect_failure_retryable = Some(payload.retryable);
                }
            }
        }
    }
    if node.side_effect.is_some() {
        validate_side_effect_resume_output(projections, node, attempt_id, &output.payloads)?;
        if failed {
            if !side_effect_failed {
                return Err(RuntimeError::InvalidRunnerOutput(format!(
                    "side-effect node {} returned StateAttemptFailed without SideEffectFailed",
                    node.node_id
                )));
            }
            if side_effect_failure_retryable != attempt_failure_retryable {
                return Err(RuntimeError::InvalidRunnerOutput(format!(
                    "side-effect node {} returned inconsistent failure retryability",
                    node.node_id
                )));
            }
            if completed || terminal_cell || public_output_produced {
                return Err(RuntimeError::InvalidRunnerOutput(format!(
                    "runner for node {} mixed side-effect failure with successful terminal evidence",
                    node.node_id
                )));
            }
            return Ok(());
        }
        if side_effect_failed {
            return Err(RuntimeError::InvalidRunnerOutput(format!(
                "side-effect node {} returned SideEffectFailed without StateAttemptFailed",
                node.node_id
            )));
        }
        if side_effect_payload {
            if completed || terminal_cell || public_output_produced || public_output_failed {
                return Err(RuntimeError::InvalidRunnerOutput(format!(
                    "side-effect node {} mixed ledger phase events with terminal output evidence",
                    node.node_id
                )));
            }
            return Ok(());
        }
        if !completed || !terminal_cell {
            return Err(RuntimeError::InvalidRunnerOutput(format!(
                "side-effect node {} output commit must pair StateAttemptCompleted with terminal cell evidence",
                node.node_id
            )));
        }
        validate_side_effect_terminal_evidence(projections, node, attempt_id)
            .map_err(|error| RuntimeError::InvalidRunnerOutput(error.to_string()))?;
        if public_output_produced && !completed {
            return Err(RuntimeError::InvalidRunnerOutput(format!(
                "node {} projected public output without completing its certified output cell",
                node.node_id
            )));
        }
        return Ok(());
    }
    if failed && (completed || terminal_cell || public_output_produced) {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "runner for node {} mixed failure with successful terminal evidence",
            node.node_id
        )));
    }
    if public_output_failed && !failed {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "runner for node {} returned public-output failure without StateAttemptFailed",
            node.node_id
        )));
    }
    if !failed && (!completed || !terminal_cell) {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "node {} successful terminal commit must pair StateAttemptCompleted with terminal cell evidence",
            node.node_id
        )));
    }
    if public_output_produced && !completed {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "node {} projected public output without completing its certified output cell",
            node.node_id
        )));
    }
    Ok(())
}

fn validate_runner_side_effect_payload(
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    caps: &CertifiedRuntimeCapabilities,
    projections: &store::ProjectionSnapshot,
    payload: &events::KernelEventPayload,
) -> Result<()> {
    if node.side_effect.is_none() {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "non-side-effect node {} returned side-effect payload",
            node.node_id
        )));
    }
    let (payload_node_id, payload_attempt_id, _, _) =
        side_effect_payload_ref(payload).ok_or_else(|| {
            RuntimeError::InvalidRunnerOutput("expected side-effect payload".to_owned())
        })?;
    require_attempt(node, attempt_id, payload_node_id, payload_attempt_id)?;
    match payload {
        events::KernelEventPayload::SideEffectIntentPersisted(payload) => {
            if payload.scope_id != node.scope_id {
                return Err(RuntimeError::InvalidRunnerOutput(format!(
                    "side-effect node {} persisted intent for uncertified scope {}",
                    node.node_id, payload.scope_id
                )));
            }
            require_capability(
                caps,
                &payload.capability_kind,
                &payload.capability_version,
                &node.node_id,
            )?;
            require_adapter(node, &payload.adapter_kind, &payload.adapter_version)?;
        }
        events::KernelEventPayload::SideEffectClaimTakenOver(payload) => {
            if payload.new_claim_owner == payload.previous_claim_owner {
                return Err(RuntimeError::InvalidRunnerOutput(format!(
                    "side-effect node {} takeover reused the previous claim owner",
                    node.node_id
                )));
            }
            if payload.claim_generation <= payload.previous_claim_generation {
                return Err(RuntimeError::InvalidRunnerOutput(format!(
                    "side-effect node {} takeover did not increase claim generation",
                    node.node_id
                )));
            }
            if let Some(projection) =
                side_effect_projection_for_attempt(projections, node, attempt_id)?
            {
                let claim = projection.claim.as_ref().ok_or_else(|| {
                    RuntimeError::InvalidRunnerOutput(format!(
                        "side-effect node {} takeover requires an active claim",
                        node.node_id
                    ))
                })?;
                if payload.claim_fencing_token == claim.claim_fencing_token {
                    return Err(RuntimeError::InvalidRunnerOutput(format!(
                        "side-effect node {} takeover reused the previous fencing token",
                        node.node_id
                    )));
                }
            }
        }
        _ => {}
    }
    Ok(())
}

fn validate_side_effect_resume_output(
    projections: &store::ProjectionSnapshot,
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    payloads: &[events::KernelEventPayload],
) -> Result<()> {
    let Some(projection) = side_effect_projection_for_attempt(projections, node, attempt_id)?
    else {
        return Ok(());
    };
    let has_takeover = payloads.iter().any(|payload| {
        matches!(
            payload,
            events::KernelEventPayload::SideEffectClaimTakenOver(_)
        )
    });
    let has_claim = payloads
        .iter()
        .any(|payload| matches!(payload, events::KernelEventPayload::SideEffectClaimed(_)));
    let has_invocation_prepared = payloads.iter().any(|payload| {
        matches!(
            payload,
            events::KernelEventPayload::SideEffectInvocationPrepared(_)
        )
    });
    let has_invocation_started = payloads.iter().any(|payload| {
        matches!(
            payload,
            events::KernelEventPayload::SideEffectInvocationStarted(_)
        )
    });
    let has_submission_recovery = payloads.iter().any(|payload| {
        matches!(
            payload,
            events::KernelEventPayload::SideEffectNotSubmittedProven(_)
                | events::KernelEventPayload::SideEffectSubmissionObserved(_)
                | events::KernelEventPayload::SideEffectSubmissionUnknown(_)
                | events::KernelEventPayload::SideEffectAmbiguous(_)
        )
    });

    match projection.phase {
        store::SideEffectPhase::Claimed { .. } => {
            if !has_takeover && !has_invocation_prepared {
                return Err(RuntimeError::InvalidRunnerOutput(format!(
                    "side-effect node {} resumed claimed ledger {} without takeover or prepared invocation",
                    node.node_id, projection.ledger_key
                )));
            }
        }
        store::SideEffectPhase::InvocationPrepared { .. } => {
            if !has_takeover && !has_invocation_started {
                return Err(RuntimeError::InvalidRunnerOutput(format!(
                    "side-effect node {} resumed prepared ledger {} without takeover or invocation start",
                    node.node_id, projection.ledger_key
                )));
            }
        }
        store::SideEffectPhase::NotSubmittedProven { .. } => {
            if !has_claim {
                return Err(RuntimeError::InvalidRunnerOutput(format!(
                    "side-effect node {} resumed not-submitted ledger {} without next-epoch claim",
                    node.node_id, projection.ledger_key
                )));
            }
        }
        store::SideEffectPhase::InvocationStarted { .. }
        | store::SideEffectPhase::SubmissionUnknown { .. } => {
            if !has_submission_recovery {
                return Err(RuntimeError::InvalidRunnerOutput(format!(
                    "side-effect node {} resumed uncertain submission ledger {} without submission recovery evidence",
                    node.node_id, projection.ledger_key
                )));
            }
        }
        store::SideEffectPhase::Ambiguous { .. } => {
            return Err(RuntimeError::InvalidRunnerOutput(format!(
                "side-effect node {} attempted to run ambiguous ledger {}",
                node.node_id, projection.ledger_key
            )));
        }
        store::SideEffectPhase::Failed { .. } => {
            return Err(RuntimeError::InvalidRunnerOutput(format!(
                "side-effect node {} attempted to run failed ledger {} on the same attempt",
                node.node_id, projection.ledger_key
            )));
        }
        store::SideEffectPhase::IntentPersisted { .. }
        | store::SideEffectPhase::SubmissionObserved { .. }
        | store::SideEffectPhase::ReceiptObserved { .. }
        | store::SideEffectPhase::ConfirmationObserved { .. } => {}
    }

    if has_invocation_started
        && matches!(projection.phase, store::SideEffectPhase::Claimed { .. })
        && !has_takeover
        && !has_invocation_prepared
    {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "side-effect node {} started invocation from stale claim without takeover",
            node.node_id
        )));
    }

    Ok(())
}

fn validate_public_output(
    runtime_spec: &CertifiedRuntimeSpec,
    node: &spec::NodeSpec,
    payload: &events::PublicOutputProduced,
) -> Result<()> {
    validate_public_output_render_node(
        runtime_spec,
        node,
        payload.public_schema_id.clone(),
        &payload.renderer_descriptor_id,
    )?;
    let Some(spec::FrameworkNodeSpec::PublicOutputRender(render)) = &node.framework else {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "node {} is not certified as a public-output render node",
            node.node_id
        )));
    };
    if payload.output_spec_digest != render.output_spec_digest
        || payload.receipt_cell_id != node.output_cell
        || payload.cells.len() != render.required_cells.len()
    {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "public-output payload for node {} does not match certified render contract",
            node.node_id
        )));
    }
    for (actual, expected) in payload.cells.iter().zip(render.required_cells.iter()) {
        if actual.public_field_path != expected.public_field_path
            || actual.cell_id != expected.cell_id
            || actual.producer != expected.producer
            || actual.scope_id != expected.scope_id
            || actual.semantic_type_id != expected.semantic_type_id
            || actual.schema_id != expected.schema_id
            || actual.value_lineage != expected.value_lineage
        {
            return Err(RuntimeError::InvalidRunnerOutput(format!(
                "public-output cell evidence for node {} does not match certified output",
                node.node_id
            )));
        }
    }
    Ok(())
}

fn validate_public_output_render_node(
    runtime_spec: &CertifiedRuntimeSpec,
    node: &spec::NodeSpec,
    public_schema_id: SchemaId,
    renderer_descriptor_id: &DescriptorId,
) -> Result<()> {
    let Some(spec::FrameworkNodeSpec::PublicOutputRender(render)) = &node.framework else {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "node {} is not certified as a public-output render node",
            node.node_id
        )));
    };
    if render.public_schema_id != public_schema_id
        || runtime_spec.spec().public_outputs.public_schema_id != public_schema_id
        || render.renderer_descriptor.descriptor_id != *renderer_descriptor_id
    {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "public-output render metadata for node {} does not match certified spec",
            node.node_id
        )));
    }
    Ok(())
}

fn require_attempt(
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    actual_node_id: &NodeId,
    actual_attempt_id: &AttemptId,
) -> Result<()> {
    if actual_node_id == &node.node_id && actual_attempt_id == attempt_id {
        Ok(())
    } else {
        Err(RuntimeError::InvalidRunnerOutput(format!(
            "payload for node {} attempt {} was bound to node {} attempt {}",
            node.node_id, attempt_id, actual_node_id, actual_attempt_id
        )))
    }
}

fn require_capability(
    caps: &CertifiedRuntimeCapabilities,
    kind: &CapabilityKind,
    version: &CapabilityVersion,
    node_id: &NodeId,
) -> Result<()> {
    if caps.contains(kind, version) {
        Ok(())
    } else {
        Err(RuntimeError::InvalidRunnerOutput(format!(
            "node {node_id} used uncertified capability {kind}:{version}"
        )))
    }
}

fn require_adapter(
    node: &spec::NodeSpec,
    kind: &AdapterKind,
    version: &AdapterVersion,
) -> Result<()> {
    if node
        .adapter_bindings
        .iter()
        .any(|adapter| adapter.adapter_kind == *kind && adapter.adapter_version == *version)
    {
        Ok(())
    } else {
        Err(RuntimeError::InvalidRunnerOutput(format!(
            "node {} used uncertified adapter {}:{}",
            node.node_id, kind, version
        )))
    }
}

fn next_attempt_no(projections: &store::ProjectionSnapshot, node_id: &NodeId) -> Result<u32> {
    let count = projections
        .attempts()
        .filter(|((attempt_node_id, _), _)| attempt_node_id == node_id)
        .count();
    u32::try_from(count + 1).map_err(|_| {
        RuntimeError::InvalidRunStream(format!("attempt count overflow for {node_id}"))
    })
}

fn attempt_id(
    run_id: &RunId,
    spec_hash: &SpecHash,
    node_id: &NodeId,
    attempt_no: u32,
) -> Result<AttemptId> {
    let canonical = canonical_json(serde_json::json!({
        "attempt_no": attempt_no,
        "node_id": node_id.as_str(),
        "run_id": run_id.as_str(),
        "spec_hash": spec_hash.as_str(),
    }))?;
    Ok(AttemptId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        *canonical.content_digest().digest(),
    ))
}

fn payload_spec_hash(payload: &events::KernelEventPayload) -> SpecHash {
    match payload {
        events::KernelEventPayload::RunStarted(payload) => payload.spec_hash.clone(),
        events::KernelEventPayload::StateAttemptStarted(payload) => payload.spec_hash.clone(),
        events::KernelEventPayload::FactRecorded(payload) => payload.spec_hash.clone(),
        events::KernelEventPayload::ArtifactReferenced(payload) => payload.spec_hash.clone(),
        events::KernelEventPayload::CellProduced(payload) => payload.spec_hash.clone(),
        events::KernelEventPayload::CellSkipped(payload) => payload.spec_hash.clone(),
        events::KernelEventPayload::SideEffectIntentPersisted(payload) => payload.spec_hash.clone(),
        events::KernelEventPayload::SideEffectClaimed(payload) => payload.spec_hash.clone(),
        events::KernelEventPayload::SideEffectClaimTakenOver(payload) => payload.spec_hash.clone(),
        events::KernelEventPayload::SideEffectInvocationPrepared(payload) => {
            payload.spec_hash.clone()
        }
        events::KernelEventPayload::SideEffectInvocationStarted(payload) => {
            payload.spec_hash.clone()
        }
        events::KernelEventPayload::SideEffectNotSubmittedProven(payload) => {
            payload.spec_hash.clone()
        }
        events::KernelEventPayload::SideEffectSubmissionObserved(payload) => {
            payload.spec_hash.clone()
        }
        events::KernelEventPayload::SideEffectSubmissionUnknown(payload) => {
            payload.spec_hash.clone()
        }
        events::KernelEventPayload::SideEffectReceiptObserved(payload) => payload.spec_hash.clone(),
        events::KernelEventPayload::SideEffectConfirmationObserved(payload) => {
            payload.spec_hash.clone()
        }
        events::KernelEventPayload::SideEffectAmbiguous(payload) => payload.spec_hash.clone(),
        events::KernelEventPayload::SideEffectFailed(payload) => payload.spec_hash.clone(),
        events::KernelEventPayload::PublicOutputProduced(payload) => payload.spec_hash.clone(),
        events::KernelEventPayload::PublicOutputRenderFailed(payload) => payload.spec_hash.clone(),
        events::KernelEventPayload::StateAttemptCompleted(payload) => payload.spec_hash.clone(),
        events::KernelEventPayload::StateAttemptFailed(payload) => payload.spec_hash.clone(),
        events::KernelEventPayload::RunCompleted(payload) => payload.spec_hash.clone(),
        events::KernelEventPayload::RetentionRefsAppended(payload) => payload.spec_hash.clone(),
        events::KernelEventPayload::RetentionManifestProjected(payload) => {
            payload.spec_hash.clone()
        }
    }
}

fn store_seed_artifact(seed: &events::SeedCellRef) -> store::ArtifactEvidenceRef {
    store::ArtifactEvidenceRef {
        artifact_id: seed.seed_artifact.artifact_id.clone(),
        digest: seed.seed_artifact.content_digest.clone(),
        byte_len: seed.seed_artifact.byte_len,
        media_type: seed.seed_artifact.media_type.clone(),
        schema_id: Some(seed.seed_artifact.schema_id.clone()),
        semantic_type_id: seed.seed_artifact.semantic_type_id.clone(),
        producer_node_id: None,
        producer_seed_id: Some(seed.seed_id.clone()),
        artifact_role: seed.seed_artifact.role,
    }
}

fn config_artifact_reference_payloads(
    spec_hash: &SpecHash,
    artifacts: &[store::ArtifactEvidenceRef],
) -> Result<Vec<events::KernelEventPayload>> {
    artifacts
        .iter()
        .map(|artifact| {
            let schema_id = artifact.schema_id.clone().ok_or_else(|| {
                RuntimeError::InvalidRunStream(format!(
                    "config artifact {} is missing schema id",
                    artifact.artifact_id
                ))
            })?;
            Ok(events::KernelEventPayload::ArtifactReferenced(
                events::ArtifactReferenced {
                    spec_hash: spec_hash.clone(),
                    node_id: None,
                    attempt_id: None,
                    artifact_ref: events::ArtifactEvidenceRef {
                        artifact_id: artifact.artifact_id.clone(),
                        role: artifact.artifact_role,
                        schema_id,
                        semantic_type_id: artifact.semantic_type_id.clone(),
                        content_digest: artifact.digest.clone(),
                        byte_len: artifact.byte_len,
                        media_type: artifact.media_type.clone(),
                    },
                },
            ))
        })
        .collect()
}

fn retention_ref_for_artifact(artifact: &store::ArtifactEvidenceRef) -> events::RetentionRef {
    events::RetentionRef {
        artifact_id: artifact.artifact_id.clone(),
        role: artifact.artifact_role,
        content_digest: artifact.digest.clone(),
    }
}

fn canonical_json(value: serde_json::Value) -> Result<PlainCanonicalJsonBytes> {
    let json = serde_json::to_string(&value)
        .map_err(|error| RuntimeError::Canonical(error.to_string()))?;
    PlainCanonicalJsonBytes::from_json_str(&json)
        .map_err(|error| RuntimeError::Canonical(error.to_string()))
}

fn content_digest_json(value: serde_json::Value) -> Result<ContentDigest> {
    Ok(canonical_json(value)?.content_digest())
}

fn config_ref_key(config_ref: &spec::ConfigRef) -> String {
    format!("{}:{}", config_ref.schema_id, config_ref.digest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mfm_capabilities::CapabilityRole;
    use mfm_ids::{
        DigestBytes, EffectKind, EffectVersion, EventId, ScopeId, SeedId, SemanticTypeId,
        StateKind, StateVersion,
    };
    use mfm_program::{
        build_root_with_registries, CanonicalSeed, PublicOutputKey, PureState, RootBuilder,
        ScopeKey, StateKey, StateRegistryBuilder, StateResult, StateSpec,
    };
    use mfm_program_derive::{MfmConfig, MfmValue, PublicOutputs};
    use mfm_store::v1::{TypedProjectionRead, TypedRunEventStore};
    use serde::{Deserialize, Serialize};

    const D0: DigestBytes = DigestBytes::from_array([0x10; 32]);
    const D1: DigestBytes = DigestBytes::from_array([0x11; 32]);
    const D2: DigestBytes = DigestBytes::from_array([0x12; 32]);
    const D3: DigestBytes = DigestBytes::from_array([0x13; 32]);
    const D4: DigestBytes = DigestBytes::from_array([0x14; 32]);
    const D5: DigestBytes = DigestBytes::from_array([0x15; 32]);
    const D6: DigestBytes = DigestBytes::from_array([0x16; 32]);
    const D7: DigestBytes = DigestBytes::from_array([0x17; 32]);
    const D8: DigestBytes = DigestBytes::from_array([0x18; 32]);
    const D9: DigestBytes = DigestBytes::from_array([0x19; 32]);

    trait TestPreparedCommitExt {
        fn append_prepared_commit(
            &mut self,
            request: store::TypedCommitRequest,
        ) -> store::Result<store::CommitOutcome>;
    }

    impl TestPreparedCommitExt for store::InMemoryTypedRunStore {
        fn append_prepared_commit(
            &mut self,
            request: store::TypedCommitRequest,
        ) -> store::Result<store::CommitOutcome> {
            let admitted_artifacts = request.required_artifacts.clone();
            let commit = store::PreparedTypedCommit::new(request, admitted_artifacts)?;
            self.append_prepared_typed_commit(commit)
        }
    }

    #[derive(Clone)]
    struct TestRuntimeArtifactStager;

    impl RuntimeArtifactStager for TestRuntimeArtifactStager {
        fn stage_verified_artifact<'a>(
            &'a self,
            bytes: Vec<u8>,
            evidence: store::ArtifactEvidenceRef,
        ) -> RuntimeArtifactStageFuture<'a> {
            Box::pin(async move {
                verify_artifact_bytes(&bytes, &evidence)?;
                Ok(())
            })
        }
    }

    #[derive(Clone)]
    struct FailingRuntimeArtifactStager;

    impl RuntimeArtifactStager for FailingRuntimeArtifactStager {
        fn stage_verified_artifact<'a>(
            &'a self,
            _bytes: Vec<u8>,
            _evidence: store::ArtifactEvidenceRef,
        ) -> RuntimeArtifactStageFuture<'a> {
            Box::pin(async move {
                Err(RuntimeError::Store(
                    "test artifact staging failure".to_owned(),
                ))
            })
        }
    }

    fn test_scheduler(registry: ErasedRunnerRegistry) -> SerialTypedScheduler {
        test_scheduler_with_stager(registry, Arc::new(TestRuntimeArtifactStager))
    }

    fn test_scheduler_with_stager(
        registry: ErasedRunnerRegistry,
        artifact_stager: Arc<dyn RuntimeArtifactStager>,
    ) -> SerialTypedScheduler {
        SerialTypedScheduler::new(registry, artifact_stager)
    }

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
    #[mfm(
        namespace = "mfm.runtime.test",
        name = "value",
        version = "1",
        schema = "mfm.runtime.test.value"
    )]
    struct CertifierValue {
        amount: u64,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmConfig)]
    struct CertifierConfig {
        multiplier: u64,
    }

    #[derive(PublicOutputs)]
    #[mfm(schema = "mfm.runtime.test.public_outputs")]
    struct CertifierPublicOutputs<'p, 's> {
        result: mfm_program::Handle<'p, 's, CertifierValue>,
    }

    struct CertifierState {
        config: CertifierConfig,
    }

    impl StateSpec for CertifierState {
        type Config = CertifierConfig;
        type Input = CertifierValue;
        type Output = CertifierValue;
        type Effect = mfm_effects::Pure;
        type Caps = mfm_capabilities::NoCaps;

        fn kind() -> mfm_program::Result<StateKind> {
            StateKind::new(
                "mfm.runtime.test",
                "multiply",
                DigestAlgorithm::Sha256JcsV1,
                D1,
            )
            .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
        }

        fn version() -> mfm_program::Result<StateVersion> {
            StateVersion::new("mfm.runtime.test.multiply.v1")
                .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
        }

        fn name() -> &'static str {
            "mfm.runtime.test.multiply"
        }

        fn new(config: Self::Config) -> mfm_program::Result<Self> {
            Ok(Self { config })
        }
    }

    impl PureState for CertifierState {
        fn run(&self, input: Self::Input) -> StateResult<Self::Output> {
            Ok(CertifierValue {
                amount: input.amount * self.config.multiplier,
            })
        }
    }

    fn certifier_backed_runtime_authority() -> (
        mfm_certify::CertifiedTypedSpec,
        mfm_certify::CertificationRegistry,
    ) {
        let mut states = StateRegistryBuilder::new();
        let registered = states
            .register::<CertifierState>()
            .expect("state registration");
        let mut registry = mfm_certify::CertificationRegistry::new();
        registry
            .register_state(&registered)
            .expect("certification registry");
        let draft = build_root_with_registries(
            ScopeKey::new("root").expect("root key"),
            states.snapshot(),
            mfm_program::OperationRegistryBuilder::new().snapshot(),
            |root: &mut RootBuilder<'_, '_>| {
                let seed = root.seed(
                    mfm_program::SeedKey::new("initial").expect("seed key"),
                    CanonicalSeed::from_value(&CertifierValue { amount: 2 }).expect("seed"),
                )?;
                let result = root.scope().state::<CertifierState, _>(
                    StateKey::new("multiply-state")?,
                    CertifierConfig { multiplier: 3 },
                    seed,
                )?;
                root.bind_public_outputs(
                    PublicOutputKey::new("terminal")?,
                    &CertifierPublicOutputs { result },
                )
            },
        )
        .expect("program draft");
        (
            mfm_certify::certify_program_draft(&draft).expect("certified program"),
            registry,
        )
    }

    const DA: DigestBytes = DigestBytes::from_array([0x1a; 32]);
    const DB: DigestBytes = DigestBytes::from_array([0x1b; 32]);
    const DC: DigestBytes = DigestBytes::from_array([0x1c; 32]);
    const DD: DigestBytes = DigestBytes::from_array([0x1d; 32]);
    const DE: DigestBytes = DigestBytes::from_array([0x1e; 32]);
    const DF: DigestBytes = DigestBytes::from_array([0x1f; 32]);

    #[derive(Clone)]
    struct Fixture {
        runtime_spec: CertifiedRuntimeSpec,
        run_id: RunId,
        seed_ref: events::SeedCellRef,
        descriptor_a: DescriptorId,
        descriptor_b: DescriptorId,
        render_node: NodeId,
        render_cell: CellId,
        cell_a: CellId,
        cell_b: CellId,
        cap_kind: CapabilityKind,
        cap_version: CapabilityVersion,
        adapter_kind: AdapterKind,
        adapter_version: AdapterVersion,
    }

    struct RecordingRunner {
        expected_caps: Vec<(CapabilityKind, CapabilityVersion)>,
        output_artifact: ArtifactId,
        output_digest: ContentDigest,
    }

    impl ErasedNodeRunner for RecordingRunner {
        fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
            Box::pin(async move {
                for (kind, version) in &self.expected_caps {
                    assert!(ctx.caps().contains(kind, version));
                }
                let output_cell = ctx.node().output_cell.clone();
                let cell = match ctx.inputs().root.clone() {
                    MaterializedInputNode::Cell(cell) => cell,
                    MaterializedInputNode::Unit
                    | MaterializedInputNode::Tuple(_)
                    | MaterializedInputNode::Struct(_)
                    | MaterializedInputNode::Vec(_)
                    | MaterializedInputNode::NonEmptyVec(_) => {
                        panic!("expected cell input")
                    }
                };
                assert!(matches!(
                    cell.terminal,
                    MaterializedCellTerminal::Seed { .. }
                        | MaterializedCellTerminal::Produced { .. }
                ));
                let certified_cell = ctx.projections().cell_terminal(&output_cell).is_none();
                assert!(certified_cell);
                let artifact = store::ArtifactEvidenceRef {
                    artifact_id: self.output_artifact.clone(),
                    digest: self.output_digest.clone(),
                    byte_len: 17,
                    media_type: spec::MediaType::new("application/json").expect("media"),
                    schema_id: Some(ctx.descriptor().output_schema_id.clone()),
                    semantic_type_id: Some(ctx.descriptor().output_semantic_type_id.clone()),
                    producer_node_id: Some(ctx.node().node_id.clone()),
                    producer_seed_id: None,
                    artifact_role: events::ArtifactRole::StateOutput,
                };
                let staged_artifact = staged_attempt_artifact(&ctx, artifact)?;
                Ok(ErasedRunnerOutput {
                    staged_artifacts: vec![staged_artifact],
                    staged_retention_refs: Vec::new(),
                    payloads: vec![
                        events::KernelEventPayload::CellProduced(events::CellProduced {
                            spec_hash: ctx.spec_hash().clone(),
                            node_id: ctx.node().node_id.clone(),
                            cell_id: ctx.node().output_cell.clone(),
                            scope_id: ctx.node().scope_id.clone(),
                            attempt_id: ctx.attempt_id().clone(),
                            semantic_type_id: ctx.descriptor().output_semantic_type_id.clone(),
                            schema_id: ctx.descriptor().output_schema_id.clone(),
                            value_lineage: ctx.output_cell().value_lineage.clone(),
                            artifact_id: self.output_artifact.clone(),
                            content_digest: self.output_digest.clone(),
                            producer_state_kind: Some(ctx.node().state_kind.clone()),
                            producer_state_version: Some(ctx.node().state_version.clone()),
                        }),
                        events::KernelEventPayload::StateAttemptCompleted(
                            events::StateAttemptCompleted {
                                spec_hash: ctx.spec_hash().clone(),
                                node_id: ctx.node().node_id.clone(),
                                attempt_id: ctx.attempt_id().clone(),
                                output_cell_id: ctx.node().output_cell.clone(),
                            },
                        ),
                    ],
                })
            })
        }
    }

    #[tokio::test]
    async fn serial_scheduler_runs_nodes_in_certified_topological_order() {
        let fixture = fixture();
        let mut registry = ErasedRunnerRegistry::new();
        registry
            .register(binding(
                fixture.descriptor_a.clone(),
                "pure",
                RecordingRunner {
                    expected_caps: Vec::new(),
                    output_artifact: artifact(0xa1),
                    output_digest: content(0xa2),
                },
            ))
            .expect("binding a");
        registry
            .register(binding(
                fixture.descriptor_b.clone(),
                "read",
                RecordingRunner {
                    expected_caps: vec![(fixture.cap_kind.clone(), fixture.cap_version.clone())],
                    output_artifact: artifact(0xb1),
                    output_digest: content(0xb2),
                },
            ))
            .expect("binding b");
        let scheduler = test_scheduler(registry);
        let mut store = store::InMemoryTypedRunStore::new();
        scheduler
            .start_run(
                &mut store,
                &fixture.runtime_spec,
                fixture.run_id.clone(),
                run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]),
            )
            .expect("start run");

        assert_eq!(
            scheduler
                .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
                .await
                .expect("drive a"),
            SchedulerStatus::Advanced
        );
        assert!(store
            .projection_snapshot()
            .cell_terminal(&fixture.cell_a)
            .is_some());
        assert!(store
            .projection_snapshot()
            .cell_terminal(&fixture.cell_b)
            .is_none());
        assert_eq!(
            scheduler
                .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
                .await
                .expect("drive b"),
            SchedulerStatus::Advanced
        );
        assert!(store
            .projection_snapshot()
            .cell_terminal(&fixture.cell_b)
            .is_some());
    }

    #[tokio::test]
    async fn scheduler_completes_run_after_public_output_evidence() {
        let fixture = fixture();
        let scheduler = test_scheduler(registered_fixture_runners(&fixture));
        let mut store = store::InMemoryTypedRunStore::new();
        scheduler
            .start_run(
                &mut store,
                &fixture.runtime_spec,
                fixture.run_id.clone(),
                run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]),
            )
            .expect("start run");

        assert_eq!(
            scheduler
                .drive_until_blocked(&mut store, &fixture.runtime_spec, &fixture.run_id)
                .await
                .expect("drive to public output"),
            SchedulerStatus::PublicOutputProjected
        );
        assert_eq!(
            store.projection_snapshot().run_state(&fixture.run_id),
            store::RunState::Completed
        );
        assert!(store
            .projection_snapshot()
            .cell_terminal(&fixture.render_cell)
            .is_some());

        let stream = store.load_run_stream(&fixture.run_id);
        let public_output_pos = stream
            .iter()
            .position(|event| {
                matches!(
                    event.payload(),
                    events::KernelEventPayload::PublicOutputProduced(_)
                )
            })
            .expect("public output produced");
        let completed_pos = stream
            .iter()
            .position(|event| {
                matches!(event.payload(), events::KernelEventPayload::RunCompleted(_))
            })
            .expect("run completed");
        assert!(public_output_pos < completed_pos);

        let public_event = &stream[public_output_pos];
        let public_payload = match public_event.payload() {
            events::KernelEventPayload::PublicOutputProduced(payload) => payload,
            _ => unreachable!("checked above"),
        };
        assert_eq!(public_payload.node_id, fixture.render_node);
        assert_eq!(public_payload.receipt_cell_id, fixture.render_cell);
        assert!(public_payload.rendered_artifact_id.is_none());

        let completed_payload = match stream[completed_pos].payload() {
            events::KernelEventPayload::RunCompleted(payload) => payload,
            _ => unreachable!("checked above"),
        };
        assert_eq!(
            completed_payload.outcome,
            events::RunCompletionOutcome::Completed(events::PublicOutputCompletionEvidence {
                public_output_schema_id: fixture
                    .runtime_spec
                    .spec()
                    .public_outputs
                    .public_schema_id
                    .clone(),
                public_output_event_id: public_event.event_id().clone(),
            })
        );
        assert_eq!(completed_pos, stream.len() - 1);

        let complete_node =
            certified_complete_run_node(&fixture.runtime_spec).expect("complete node");
        let completion_seq = stream[completed_pos].seq();
        let completion_key = stream[completed_pos].commit_key().clone();
        let completion_commit = stream
            .iter()
            .filter(|event| event.seq() == completion_seq && event.commit_key() == &completion_key)
            .collect::<Vec<_>>();
        assert_eq!(completion_commit.len(), 5);
        let complete_attempt = completion_commit
            .iter()
            .find_map(|event| match event.payload() {
                events::KernelEventPayload::StateAttemptStarted(payload)
                    if payload.node_id == complete_node.node_id =>
                {
                    Some(payload.attempt_id.clone())
                }
                _ => None,
            })
            .expect("complete attempt started");
        let complete_receipt = completion_commit
            .iter()
            .find_map(|event| match event.payload() {
                events::KernelEventPayload::CellProduced(payload)
                    if payload.node_id == complete_node.node_id =>
                {
                    Some(payload)
                }
                _ => None,
            })
            .expect("complete receipt cell");
        assert_eq!(complete_receipt.attempt_id, complete_attempt);
        assert_eq!(complete_receipt.cell_id, complete_node.output_cell);
        assert!(completion_commit.iter().any(|event| {
            matches!(
                event.payload(),
                events::KernelEventPayload::StateAttemptCompleted(payload)
                    if payload.node_id == complete_node.node_id
                        && payload.attempt_id == complete_attempt
                        && payload.output_cell_id == complete_node.output_cell
            )
        }));
        assert!(completion_commit.iter().any(|event| {
            matches!(
                event.payload(),
                events::KernelEventPayload::ArtifactReferenced(payload)
                    if payload.node_id.as_ref() == Some(&complete_node.node_id)
                        && payload.attempt_id.as_ref() == Some(&complete_attempt)
                        && payload.artifact_ref.artifact_id == complete_receipt.artifact_id
                        && payload.artifact_ref.content_digest == complete_receipt.content_digest
                        && payload.artifact_ref.role == events::ArtifactRole::StateOutput
            )
        }));
        let stream_len = stream.len();
        assert_eq!(
            scheduler
                .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
                .await
                .expect("drive completed run"),
            SchedulerStatus::PublicOutputProjected
        );
        assert_eq!(store.load_run_stream(&fixture.run_id).len(), stream_len);
    }

    #[tokio::test]
    async fn public_output_receipt_staging_failure_prevents_commit() {
        let fixture = fixture();
        let scheduler = test_scheduler_with_stager(
            registered_fixture_runners(&fixture),
            Arc::new(FailingRuntimeArtifactStager),
        );
        let mut store = store::InMemoryTypedRunStore::new();
        scheduler
            .start_run(
                &mut store,
                &fixture.runtime_spec,
                fixture.run_id.clone(),
                run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]),
            )
            .expect("start run");
        scheduler
            .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
            .await
            .expect("drive a");
        scheduler
            .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
            .await
            .expect("drive b");

        assert!(matches!(
            scheduler
                .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
                .await,
            Err(RuntimeError::Store(message))
                if message.contains("test artifact staging failure")
        ));
        assert!(store
            .projection_snapshot()
            .cell_terminal(&fixture.render_cell)
            .is_none());
        assert!(store.load_run_stream(&fixture.run_id).iter().all(|event| {
            !matches!(
                event.payload(),
                events::KernelEventPayload::PublicOutputProduced(_)
            )
        }));
    }

    #[tokio::test]
    async fn scheduler_binds_staged_retention_refs_and_projects_manifest() {
        let fixture = fixture_with_retention_lifecycle_node();
        let scheduler = test_scheduler(registered_fixture_runners(&fixture));
        let mut store = store::InMemoryTypedRunStore::new();
        scheduler
            .start_run(
                &mut store,
                &fixture.runtime_spec,
                fixture.run_id.clone(),
                run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]),
            )
            .expect("start run");

        let start_retention = store
            .projection_snapshot()
            .retention(&fixture.run_id)
            .expect("run-start retention");
        assert!(start_retention
            .refs
            .contains_key(&fixture.seed_ref.seed_artifact.artifact_id));

        let status = scheduler
            .drive_until_blocked(&mut store, &fixture.runtime_spec, &fixture.run_id)
            .await
            .expect("drive to completion");
        assert_eq!(status, SchedulerStatus::PublicOutputProjected);
        let render_receipt_artifact = match store
            .projection_snapshot()
            .cell_terminal(&fixture.render_cell)
            .expect("render receipt cell")
        {
            store::CellTerminalProjection::Produced { artifact_id, .. } => artifact_id.clone(),
            terminal => panic!("unexpected render terminal: {terminal:?}"),
        };
        assert!(store
            .projection_snapshot()
            .retention(&fixture.run_id)
            .expect("runtime retention")
            .refs
            .contains_key(&render_receipt_artifact));

        let projection = store
            .projection_snapshot()
            .retention(&fixture.run_id)
            .expect("retention projection");
        let manifest = projection.manifest.as_ref().expect("manifest");
        assert_eq!(manifest.manifest_seq, 1);
        assert!(projection.refs.contains_key(&manifest.manifest_artifact_id));

        let retention_node =
            certified_retention_manifest_node(&fixture.runtime_spec).expect("retention node");
        let stream = store.load_run_stream(&fixture.run_id);
        let manifest_event = stream
            .iter()
            .find_map(|event| match event.payload() {
                events::KernelEventPayload::RetentionManifestProjected(payload)
                    if payload.manifest_artifact_id == manifest.manifest_artifact_id =>
                {
                    Some((event.seq(), payload))
                }
                _ => None,
            })
            .expect("manifest event");
        let retention_seq = manifest_event.0;
        assert_eq!(manifest_event.1.manifest_digest, manifest.manifest_digest);
        assert!(stream.iter().any(|event| {
            matches!(
                event.payload(),
                events::KernelEventPayload::StateAttemptStarted(payload)
                    if event.seq() == retention_seq
                        && payload.node_id == retention_node.node_id
            )
        }));
        let receipt = stream
            .iter()
            .find_map(|event| match event.payload() {
                events::KernelEventPayload::CellProduced(payload)
                    if event.seq() == retention_seq
                        && payload.node_id == retention_node.node_id =>
                {
                    Some(payload)
                }
                _ => None,
            })
            .expect("retention receipt cell");
        assert_eq!(receipt.cell_id, retention_node.output_cell);
        assert!(stream.iter().any(|event| {
            matches!(
                event.payload(),
                events::KernelEventPayload::StateAttemptCompleted(payload)
                    if event.seq() == retention_seq
                        && payload.node_id == retention_node.node_id
                        && payload.attempt_id == receipt.attempt_id
                        && payload.output_cell_id == retention_node.output_cell
            )
        }));
        let manifest_ref = stream
            .iter()
            .find_map(|event| match event.payload() {
                events::KernelEventPayload::RetentionRefsAppended(payload)
                    if event.seq() == retention_seq
                        && payload.reason == events::RetentionReason::ManifestProjection =>
                {
                    payload.refs.first()
                }
                _ => None,
            })
            .expect("manifest retention ref");
        assert_eq!(manifest_ref.artifact_id, manifest.manifest_artifact_id);
        assert_eq!(manifest_ref.content_digest, manifest.manifest_digest);
        assert_eq!(manifest_ref.role, events::ArtifactRole::RetentionManifest);
    }

    #[tokio::test]
    async fn retention_manifest_projection_retry_is_idempotent_after_current_store_advanced() {
        let fixture = fixture_with_retention_lifecycle_node();
        let scheduler = test_scheduler(registered_fixture_runners(&fixture));
        let mut store = store::InMemoryTypedRunStore::new();
        scheduler
            .start_run(
                &mut store,
                &fixture.runtime_spec,
                fixture.run_id.clone(),
                run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]),
            )
            .expect("start run");

        drive_until_public_output_produced(&scheduler, &mut store, &fixture).await;
        let stale_stream = store.load_run_stream(&fixture.run_id);

        scheduler
            .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
            .await
            .expect("advance current store with retention projection");
        assert!(store
            .projection_snapshot()
            .retention(&fixture.run_id)
            .and_then(|retention| retention.manifest.as_ref())
            .is_some());

        let mut stale_store = StaleStreamStore {
            inner: &mut store,
            stream: stale_stream,
        };
        assert_eq!(
            scheduler
                .drive_once(&mut stale_store, &fixture.runtime_spec, &fixture.run_id)
                .await
                .expect("idempotent retention retry"),
            SchedulerStatus::Advanced
        );
    }

    #[tokio::test]
    async fn retention_manifest_projection_rejects_corrupt_stream_before_commit() {
        let fixture = fixture_with_retention_lifecycle_node();
        let scheduler = test_scheduler(registered_fixture_runners(&fixture));
        let mut store = store::InMemoryTypedRunStore::new();
        scheduler
            .start_run(
                &mut store,
                &fixture.runtime_spec,
                fixture.run_id.clone(),
                run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]),
            )
            .expect("start run");

        drive_until_public_output_produced(&scheduler, &mut store, &fixture).await;

        let mut corrupt_stream = store.load_run_stream(&fixture.run_id);
        let corrupt_pos = corrupt_stream
            .iter()
            .position(|event| {
                event.ordinal() == store::CommitOrdinal::new(0)
                    && matches!(
                        event.payload(),
                        events::KernelEventPayload::StateAttemptStarted(_)
                    )
            })
            .expect("attempt-start event");
        let mut corrupt_payload = match corrupt_stream[corrupt_pos].payload().clone() {
            events::KernelEventPayload::StateAttemptStarted(payload) => payload,
            _ => unreachable!("position checked"),
        };
        corrupt_payload.spec_hash = SpecHash::from_digest(DigestAlgorithm::Sha256JcsV1, D9);
        corrupt_stream[corrupt_pos] = rewrite_single_payload_envelope(
            &corrupt_stream[corrupt_pos],
            events::KernelEventPayload::StateAttemptStarted(corrupt_payload),
        );

        let projection = store::ProjectionSnapshot::rebuild_from_run_stream(&corrupt_stream)
            .expect("projection rebuild accepts ordered corrupt stream");
        let mut corrupt_store = ReadOnlyCorruptStore {
            stream: corrupt_stream,
            projection,
        };

        assert!(matches!(
            scheduler
                .drive_once(&mut corrupt_store, &fixture.runtime_spec, &fixture.run_id)
                .await,
            Err(RuntimeError::InvalidRunStream(message))
                if message.contains("event payload spec hash")
        ));
    }

    #[tokio::test]
    async fn runtime_rejects_standalone_retention_manifest_projection_history() {
        let fixture = fixture_with_retention_lifecycle_node();
        let scheduler = test_scheduler(registered_fixture_runners(&fixture));
        let mut store = store::InMemoryTypedRunStore::new();
        scheduler
            .start_run(
                &mut store,
                &fixture.runtime_spec,
                fixture.run_id.clone(),
                run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]),
            )
            .expect("start run");
        drive_until_public_output_produced(&scheduler, &mut store, &fixture).await;

        let retention_node =
            certified_retention_manifest_node(&fixture.runtime_spec).expect("retention node");
        let manifest = build_retention_manifest_artifact_with_producer(
            &fixture.runtime_spec,
            &fixture.run_id,
            &store.load_run_stream(&fixture.run_id),
            Some(retention_node.node_id.clone()),
        )
        .expect("manifest");
        let manifest_evidence = manifest.evidence.clone();
        let request = store::TypedCommitRequest {
            run_id: fixture.run_id.clone(),
            expected_next_seq: store.expected_next_seq(&fixture.run_id),
            commit_key: store::CommitKey::new("synthetic/standalone-retention-projection")
                .expect("commit key"),
            payloads: retention_manifest_payloads(&fixture.runtime_spec, &fixture.run_id, manifest),
            required_artifacts: vec![manifest_evidence],
            preconditions: store::CommitPreconditions::default(),
        };
        store
            .append_prepared_commit(request)
            .expect("synthetic standalone projection");

        assert!(matches!(
            RuntimeRunView::from_store(&fixture.runtime_spec, &fixture.run_id, &store),
            Err(RuntimeError::InvalidRunStream(message))
                if message.contains("retention manifest projection was not produced")
        ));
    }

    #[tokio::test]
    async fn runtime_rejects_completed_history_without_retention_projection() {
        let fixture = fixture();
        let scheduler = test_scheduler(registered_fixture_runners(&fixture));
        let mut store = store::InMemoryTypedRunStore::new();
        scheduler
            .start_run(
                &mut store,
                &fixture.runtime_spec,
                fixture.run_id.clone(),
                run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]),
            )
            .expect("start run");
        assert_eq!(
            scheduler
                .drive_until_blocked(&mut store, &fixture.runtime_spec, &fixture.run_id)
                .await
                .expect("drive to completion"),
            SchedulerStatus::PublicOutputProjected
        );
        assert_eq!(
            store.projection_snapshot().run_state(&fixture.run_id),
            store::RunState::Completed
        );

        let valid_stream = store.load_run_stream(&fixture.run_id);
        let corrupt_stream = rewrite_stream_without_commit_containing(&valid_stream, |payload| {
            matches!(
                payload,
                events::KernelEventPayload::RetentionManifestProjected(_)
            )
        });

        assert!(matches!(
            validate_run_stream(&fixture.runtime_spec, &fixture.run_id, &corrupt_stream),
            Err(RuntimeError::InvalidRunStream(message))
                if message.contains("started before certified input cell")
        ));
    }

    #[tokio::test]
    async fn runtime_rejects_standalone_run_completed_after_retention_projection() {
        let fixture = fixture();
        let scheduler = test_scheduler(registered_fixture_runners(&fixture));
        let mut store = store::InMemoryTypedRunStore::new();
        scheduler
            .start_run(
                &mut store,
                &fixture.runtime_spec,
                fixture.run_id.clone(),
                run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]),
            )
            .expect("start run");
        scheduler
            .drive_until_blocked(&mut store, &fixture.runtime_spec, &fixture.run_id)
            .await
            .expect("drive to completion");

        let valid_stream = store.load_run_stream(&fixture.run_id);
        let completion_payload = valid_stream
            .iter()
            .find_map(|event| match event.payload() {
                events::KernelEventPayload::RunCompleted(payload) => Some(payload.clone()),
                _ => None,
            })
            .expect("run completion");
        let mut corrupt_stream =
            rewrite_stream_without_commit_containing(&valid_stream, |payload| {
                matches!(payload, events::KernelEventPayload::RunCompleted(_))
            });
        append_payload_commit_for_tests(
            &mut corrupt_stream,
            &fixture.run_id,
            "standalone-run-completed-after-retention",
            events::KernelEventPayload::RunCompleted(completion_payload),
        );

        assert!(matches!(
            validate_run_stream(&fixture.runtime_spec, &fixture.run_id, &corrupt_stream),
            Err(RuntimeError::InvalidRunStream(message))
                if message.contains("CompleteRun commit was not produced")
        ));
    }

    #[tokio::test]
    async fn runtime_rejects_complete_run_receipt_commit_without_run_completed() {
        let fixture = fixture();
        let scheduler = test_scheduler(registered_fixture_runners(&fixture));
        let mut store = store::InMemoryTypedRunStore::new();
        scheduler
            .start_run(
                &mut store,
                &fixture.runtime_spec,
                fixture.run_id.clone(),
                run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]),
            )
            .expect("start run");
        scheduler
            .drive_until_blocked(&mut store, &fixture.runtime_spec, &fixture.run_id)
            .await
            .expect("drive to completion");

        let valid_stream = store.load_run_stream(&fixture.run_id);
        let corrupt_stream = rewrite_stream_without_payloads(&valid_stream, |payload| {
            matches!(payload, events::KernelEventPayload::RunCompleted(_))
        });

        assert!(matches!(
            validate_run_stream(&fixture.runtime_spec, &fixture.run_id, &corrupt_stream),
            Err(RuntimeError::InvalidRunStream(message))
                if message.contains("missing RunCompleted")
        ));
    }

    #[tokio::test]
    async fn runtime_rejects_complete_run_receipt_artifact_ref_metadata_tampering() {
        for mutation in ["byte_len", "media_type"] {
            let fixture = fixture();
            let scheduler = test_scheduler(registered_fixture_runners(&fixture));
            let mut store = store::InMemoryTypedRunStore::new();
            scheduler
                .start_run(
                    &mut store,
                    &fixture.runtime_spec,
                    fixture.run_id.clone(),
                    run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]),
                )
                .expect("start run");
            scheduler
                .drive_until_blocked(&mut store, &fixture.runtime_spec, &fixture.run_id)
                .await
                .expect("drive to completion");

            let complete_node =
                certified_complete_run_node(&fixture.runtime_spec).expect("complete node");
            let valid_stream = store.load_run_stream(&fixture.run_id);
            let artifact_ref_pos = valid_stream
                .iter()
                .position(|event| {
                    matches!(
                        event.payload(),
                        events::KernelEventPayload::ArtifactReferenced(payload)
                            if payload.node_id.as_ref() == Some(&complete_node.node_id)
                    )
                })
                .expect("completion artifact ref");
            let mut payload = match valid_stream[artifact_ref_pos].payload().clone() {
                events::KernelEventPayload::ArtifactReferenced(payload) => payload,
                _ => unreachable!("checked above"),
            };
            match mutation {
                "byte_len" => payload.artifact_ref.byte_len += 1,
                "media_type" => {
                    payload.artifact_ref.media_type =
                        spec::MediaType::new("application/octet-stream").expect("media type");
                }
                _ => unreachable!("known mutation"),
            }
            let corrupt_stream = rewrite_commit_payload(
                &valid_stream,
                artifact_ref_pos,
                events::KernelEventPayload::ArtifactReferenced(payload),
            );

            assert!(matches!(
                validate_run_stream(&fixture.runtime_spec, &fixture.run_id, &corrupt_stream),
                Err(RuntimeError::InvalidRunStream(message))
                    if message.contains("sealed framework batch")
            ));
        }
    }

    #[tokio::test]
    async fn runtime_rejects_bare_retention_refs_before_completion() {
        let fixture = fixture();
        let scheduler = test_scheduler(registered_fixture_runners(&fixture));
        let mut store = store::InMemoryTypedRunStore::new();
        scheduler
            .start_run(
                &mut store,
                &fixture.runtime_spec,
                fixture.run_id.clone(),
                run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]),
            )
            .expect("start run");
        let mut corrupt_stream = store.load_run_stream(&fixture.run_id);
        append_payload_commit_for_tests(
            &mut corrupt_stream,
            &fixture.run_id,
            "bare-runtime-retention-ref",
            events::KernelEventPayload::RetentionRefsAppended(events::RetentionRefsAppended {
                run_id: fixture.run_id.clone(),
                spec_hash: fixture.runtime_spec.spec_hash().clone(),
                refs: vec![events::RetentionRef {
                    artifact_id: fixture.seed_ref.seed_artifact.artifact_id.clone(),
                    role: fixture.seed_ref.seed_artifact.role,
                    content_digest: fixture.seed_ref.seed_artifact.content_digest.clone(),
                }],
                reason: events::RetentionReason::RuntimeEvidence,
            }),
        );

        assert!(matches!(
            validate_run_stream(&fixture.runtime_spec, &fixture.run_id, &corrupt_stream),
            Err(RuntimeError::InvalidRunStream(message))
                if message.contains("same-commit typed payload evidence")
        ));
    }

    #[tokio::test]
    async fn runtime_rejects_missing_run_start_retention_refs() {
        let fixture = fixture();
        let scheduler = test_scheduler(registered_fixture_runners(&fixture));
        let mut store = store::InMemoryTypedRunStore::new();
        scheduler
            .start_run(
                &mut store,
                &fixture.runtime_spec,
                fixture.run_id.clone(),
                run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]),
            )
            .expect("start run");

        let valid_stream = store.load_run_stream(&fixture.run_id);
        let corrupt_stream = rewrite_stream_without_payloads(&valid_stream, |payload| {
            matches!(
                payload,
                events::KernelEventPayload::RetentionRefsAppended(events::RetentionRefsAppended {
                    reason: events::RetentionReason::RunStarted,
                    ..
                })
            )
        });

        assert!(matches!(
            validate_run_stream(&fixture.runtime_spec, &fixture.run_id, &corrupt_stream),
            Err(RuntimeError::InvalidRunStream(message))
                if message.contains("RunStarted commit must include")
        ));
    }

    #[tokio::test]
    async fn runtime_rejects_failed_or_cancelled_run_completion_without_complete_authority() {
        for (name, outcome) in [
            (
                "failed",
                events::RunCompletionOutcome::Failed(public_output_error()),
            ),
            (
                "cancelled",
                events::RunCompletionOutcome::Cancelled(public_output_error()),
            ),
        ] {
            let fixture = fixture();
            let scheduler = test_scheduler(registered_fixture_runners(&fixture));
            let mut store = store::InMemoryTypedRunStore::new();
            scheduler
                .start_run(
                    &mut store,
                    &fixture.runtime_spec,
                    fixture.run_id.clone(),
                    run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]),
                )
                .expect("start run");
            store
                .append_prepared_commit(store::TypedCommitRequest {
                    run_id: fixture.run_id.clone(),
                    expected_next_seq: store.expected_next_seq(&fixture.run_id),
                    commit_key: store::CommitKey::new(format!("forged-{name}-completion"))
                        .expect("commit key"),
                    payloads: vec![events::KernelEventPayload::RunCompleted(
                        events::RunCompleted {
                            run_id: fixture.run_id.clone(),
                            spec_hash: fixture.runtime_spec.spec_hash().clone(),
                            outcome,
                        },
                    )],
                    required_artifacts: Vec::new(),
                    preconditions: store::CommitPreconditions {
                        required_run_state: store::RequiredRunState::NotCompleted,
                        ..store::CommitPreconditions::default()
                    },
                })
                .expect("append forged completion");

            assert!(matches!(
                validate_run_stream(
                    &fixture.runtime_spec,
                    &fixture.run_id,
                    &store.load_run_stream(&fixture.run_id),
                ),
                Err(RuntimeError::InvalidRunStream(message))
                    if message.contains("failed/cancelled")
            ));
        }
    }

    #[tokio::test]
    async fn runtime_rejects_post_completion_retention_refs() {
        let fixture = fixture();
        let scheduler = test_scheduler(registered_fixture_runners(&fixture));
        let mut store = store::InMemoryTypedRunStore::new();
        scheduler
            .start_run(
                &mut store,
                &fixture.runtime_spec,
                fixture.run_id.clone(),
                run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]),
            )
            .expect("start run");
        assert_eq!(
            scheduler
                .drive_until_blocked(&mut store, &fixture.runtime_spec, &fixture.run_id)
                .await
                .expect("drive to completion"),
            SchedulerStatus::PublicOutputProjected
        );

        let mut corrupt_stream = store.load_run_stream(&fixture.run_id);
        append_payload_commit_for_tests(
            &mut corrupt_stream,
            &fixture.run_id,
            "post-completion-retention-ref",
            events::KernelEventPayload::RetentionRefsAppended(events::RetentionRefsAppended {
                run_id: fixture.run_id.clone(),
                spec_hash: fixture.runtime_spec.spec_hash().clone(),
                refs: vec![events::RetentionRef {
                    artifact_id: fixture.seed_ref.seed_artifact.artifact_id.clone(),
                    role: fixture.seed_ref.seed_artifact.role,
                    content_digest: fixture.seed_ref.seed_artifact.content_digest.clone(),
                }],
                reason: events::RetentionReason::RuntimeEvidence,
            }),
        );

        assert!(matches!(
            validate_run_stream(&fixture.runtime_spec, &fixture.run_id, &corrupt_stream),
            Err(RuntimeError::InvalidRunStream(message))
                if message.contains("events after RunCompleted")
        ));
    }

    #[tokio::test]
    async fn runtime_rejects_run_completed_with_active_retention_attempt() {
        let fixture = fixture();
        let scheduler = test_scheduler(registered_fixture_runners(&fixture));
        let mut store = store::InMemoryTypedRunStore::new();
        scheduler
            .start_run(
                &mut store,
                &fixture.runtime_spec,
                fixture.run_id.clone(),
                run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]),
            )
            .expect("start run");
        drive_until_public_output_produced(&scheduler, &mut store, &fixture).await;

        let retention_node =
            certified_retention_manifest_node(&fixture.runtime_spec).expect("retention node");
        let mut corrupt_stream = store.load_run_stream(&fixture.run_id);
        let public_event_id = corrupt_stream
            .iter()
            .find(|event| {
                matches!(
                    event.payload(),
                    events::KernelEventPayload::PublicOutputProduced(_)
                )
            })
            .expect("public output produced")
            .event_id()
            .clone();
        append_payloads_commit_for_tests(
            &mut corrupt_stream,
            &fixture.run_id,
            "active-retention-at-completion",
            vec![
                events::KernelEventPayload::StateAttemptStarted(events::StateAttemptStarted {
                    spec_hash: fixture.runtime_spec.spec_hash().clone(),
                    node_id: retention_node.node_id.clone(),
                    attempt_id: AttemptId::from_digest(
                        DigestAlgorithm::Sha256JcsV1,
                        DigestBytes::from_array([0x75; 32]),
                    ),
                    attempt_no: 99,
                    state_kind: retention_node.state_kind.clone(),
                    state_version: retention_node.state_version.clone(),
                }),
                events::KernelEventPayload::RunCompleted(events::RunCompleted {
                    run_id: fixture.run_id.clone(),
                    spec_hash: fixture.runtime_spec.spec_hash().clone(),
                    outcome: events::RunCompletionOutcome::Completed(
                        events::PublicOutputCompletionEvidence {
                            public_output_schema_id: fixture
                                .runtime_spec
                                .spec()
                                .public_outputs
                                .public_schema_id
                                .clone(),
                            public_output_event_id: public_event_id,
                        },
                    ),
                }),
            ],
        );

        assert!(matches!(
            validate_run_stream(&fixture.runtime_spec, &fixture.run_id, &corrupt_stream),
            Err(RuntimeError::InvalidRunStream(message))
                if message.contains("attempts are active")
        ));
    }

    #[tokio::test]
    async fn runtime_rejects_retained_evidence_between_retention_projection_and_completion() {
        let fixture = fixture();
        let scheduler = test_scheduler(registered_fixture_runners(&fixture));
        let mut store = store::InMemoryTypedRunStore::new();
        scheduler
            .start_run(
                &mut store,
                &fixture.runtime_spec,
                fixture.run_id.clone(),
                run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]),
            )
            .expect("start run");
        scheduler
            .drive_until_blocked(&mut store, &fixture.runtime_spec, &fixture.run_id)
            .await
            .expect("drive to completion");

        let valid_stream = store.load_run_stream(&fixture.run_id);
        let completion_payload = valid_stream
            .iter()
            .find_map(|event| match event.payload() {
                events::KernelEventPayload::RunCompleted(payload) => Some(payload.clone()),
                _ => None,
            })
            .expect("run completion");
        let retention_node =
            certified_retention_manifest_node(&fixture.runtime_spec).expect("retention node");
        let attempt_id = AttemptId::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([0x77; 32]),
        );
        let diagnostic_digest = content(0x78);
        let diagnostic_artifact =
            ArtifactId::from_digest(diagnostic_digest.algorithm(), *diagnostic_digest.digest());
        let diagnostic_ref = events::ArtifactEvidenceRef {
            artifact_id: diagnostic_artifact.clone(),
            role: events::ArtifactRole::RedactedDiagnostic,
            schema_id: retention_node.config_ref.schema_id.clone(),
            semantic_type_id: None,
            content_digest: diagnostic_digest.clone(),
            byte_len: 10,
            media_type: spec::MediaType::new("application/json").expect("media type"),
        };
        let mut corrupt_stream =
            rewrite_stream_without_commit_containing(&valid_stream, |payload| {
                matches!(payload, events::KernelEventPayload::RunCompleted(_))
            });
        append_payloads_commit_for_tests(
            &mut corrupt_stream,
            &fixture.run_id,
            "retained-evidence-after-retention-projection",
            vec![
                events::KernelEventPayload::StateAttemptStarted(events::StateAttemptStarted {
                    spec_hash: fixture.runtime_spec.spec_hash().clone(),
                    node_id: retention_node.node_id.clone(),
                    attempt_id: attempt_id.clone(),
                    attempt_no: 100,
                    state_kind: retention_node.state_kind.clone(),
                    state_version: retention_node.state_version.clone(),
                }),
                events::KernelEventPayload::ArtifactReferenced(events::ArtifactReferenced {
                    spec_hash: fixture.runtime_spec.spec_hash().clone(),
                    node_id: Some(retention_node.node_id.clone()),
                    attempt_id: Some(attempt_id.clone()),
                    artifact_ref: diagnostic_ref.clone(),
                }),
                events::KernelEventPayload::StateAttemptFailed(events::StateAttemptFailed {
                    spec_hash: fixture.runtime_spec.spec_hash().clone(),
                    node_id: retention_node.node_id.clone(),
                    attempt_id,
                    retryable: false,
                    error: events::MfmErrorInfo {
                        diagnostic_ref: Some(diagnostic_ref.clone()),
                        ..public_output_error()
                    },
                }),
                events::KernelEventPayload::RetentionRefsAppended(events::RetentionRefsAppended {
                    run_id: fixture.run_id.clone(),
                    spec_hash: fixture.runtime_spec.spec_hash().clone(),
                    refs: vec![events::RetentionRef {
                        artifact_id: diagnostic_artifact,
                        role: events::ArtifactRole::RedactedDiagnostic,
                        content_digest: diagnostic_digest,
                    }],
                    reason: events::RetentionReason::RuntimeEvidence,
                }),
            ],
        );
        append_payload_commit_for_tests(
            &mut corrupt_stream,
            &fixture.run_id,
            "completion-after-post-retention-evidence",
            events::KernelEventPayload::RunCompleted(completion_payload),
        );

        assert!(matches!(
            validate_run_stream(&fixture.runtime_spec, &fixture.run_id, &corrupt_stream),
            Err(RuntimeError::InvalidRunStream(message))
                if message.contains("after retention manifest projection")
        ));
    }

    #[tokio::test]
    async fn runtime_rejects_extra_attempt_evidence_in_retention_projection_commit() {
        let fixture = fixture();
        let scheduler = test_scheduler(registered_fixture_runners(&fixture));
        let mut store = store::InMemoryTypedRunStore::new();
        scheduler
            .start_run(
                &mut store,
                &fixture.runtime_spec,
                fixture.run_id.clone(),
                run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]),
            )
            .expect("start run");
        scheduler
            .drive_until_blocked(&mut store, &fixture.runtime_spec, &fixture.run_id)
            .await
            .expect("drive to completion");

        let retention_node =
            certified_retention_manifest_node(&fixture.runtime_spec).expect("retention node");
        let attempt_id = AttemptId::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([0x79; 32]),
        );
        let diagnostic_digest = content(0x7a);
        let diagnostic_artifact =
            ArtifactId::from_digest(diagnostic_digest.algorithm(), *diagnostic_digest.digest());
        let diagnostic_ref = events::ArtifactEvidenceRef {
            artifact_id: diagnostic_artifact,
            role: events::ArtifactRole::RedactedDiagnostic,
            schema_id: retention_node.config_ref.schema_id.clone(),
            semantic_type_id: None,
            content_digest: diagnostic_digest,
            byte_len: 10,
            media_type: spec::MediaType::new("application/json").expect("media type"),
        };
        let valid_stream = store.load_run_stream(&fixture.run_id);
        let corrupt_stream = prepend_payloads_to_retention_projection_commit_for_tests(
            &valid_stream,
            vec![
                events::KernelEventPayload::StateAttemptStarted(events::StateAttemptStarted {
                    spec_hash: fixture.runtime_spec.spec_hash().clone(),
                    node_id: retention_node.node_id.clone(),
                    attempt_id: attempt_id.clone(),
                    attempt_no: 100,
                    state_kind: retention_node.state_kind.clone(),
                    state_version: retention_node.state_version.clone(),
                }),
                events::KernelEventPayload::ArtifactReferenced(events::ArtifactReferenced {
                    spec_hash: fixture.runtime_spec.spec_hash().clone(),
                    node_id: Some(retention_node.node_id.clone()),
                    attempt_id: Some(attempt_id.clone()),
                    artifact_ref: diagnostic_ref.clone(),
                }),
                events::KernelEventPayload::StateAttemptFailed(events::StateAttemptFailed {
                    spec_hash: fixture.runtime_spec.spec_hash().clone(),
                    node_id: retention_node.node_id.clone(),
                    attempt_id,
                    retryable: false,
                    error: events::MfmErrorInfo {
                        diagnostic_ref: Some(diagnostic_ref),
                        ..public_output_error()
                    },
                }),
            ],
        );

        assert!(matches!(
            validate_run_stream(&fixture.runtime_spec, &fixture.run_id, &corrupt_stream),
            Err(RuntimeError::InvalidRunStream(message))
                if message.contains(
                    "retention manifest projection commit contains unsupported payload"
                )
        ));
    }

    #[tokio::test]
    async fn runtime_rejects_same_sequence_sidecar_commit_at_retention_projection() {
        let fixture = fixture();
        let scheduler = test_scheduler(registered_fixture_runners(&fixture));
        let mut store = store::InMemoryTypedRunStore::new();
        scheduler
            .start_run(
                &mut store,
                &fixture.runtime_spec,
                fixture.run_id.clone(),
                run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]),
            )
            .expect("start run");
        scheduler
            .drive_until_blocked(&mut store, &fixture.runtime_spec, &fixture.run_id)
            .await
            .expect("drive to completion");

        let valid_stream = store.load_run_stream(&fixture.run_id);
        let corrupt_stream =
            append_same_sequence_sidecar_to_retention_projection_for_tests(&valid_stream);

        assert!(matches!(
            validate_run_stream(&fixture.runtime_spec, &fixture.run_id, &corrupt_stream),
            Err(RuntimeError::Store(message)) if message.contains("multiple commit keys")
        ));
    }

    #[tokio::test]
    async fn runtime_rejects_post_completion_retention_attempt() {
        let fixture = fixture();
        let scheduler = test_scheduler(registered_fixture_runners(&fixture));
        let mut store = store::InMemoryTypedRunStore::new();
        scheduler
            .start_run(
                &mut store,
                &fixture.runtime_spec,
                fixture.run_id.clone(),
                run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]),
            )
            .expect("start run");
        assert_eq!(
            scheduler
                .drive_until_blocked(&mut store, &fixture.runtime_spec, &fixture.run_id)
                .await
                .expect("drive to completion"),
            SchedulerStatus::PublicOutputProjected
        );

        let retention_node =
            certified_retention_manifest_node(&fixture.runtime_spec).expect("retention node");
        let mut corrupt_stream = store.load_run_stream(&fixture.run_id);
        append_payload_commit_for_tests(
            &mut corrupt_stream,
            &fixture.run_id,
            "post-completion-retention-attempt",
            events::KernelEventPayload::StateAttemptStarted(events::StateAttemptStarted {
                spec_hash: fixture.runtime_spec.spec_hash().clone(),
                node_id: retention_node.node_id.clone(),
                attempt_id: AttemptId::from_digest(DigestAlgorithm::Sha256JcsV1, DF),
                attempt_no: 99,
                state_kind: retention_node.state_kind.clone(),
                state_version: retention_node.state_version.clone(),
            }),
        );

        assert!(matches!(
            validate_run_stream(&fixture.runtime_spec, &fixture.run_id, &corrupt_stream),
            Err(RuntimeError::InvalidRunStream(message))
                if message.contains("events after RunCompleted")
        ));
    }

    #[tokio::test]
    async fn runtime_rejects_started_attempt_for_terminal_retention_node() {
        let fixture = fixture();
        let scheduler = test_scheduler(registered_fixture_runners(&fixture));
        let mut store = store::InMemoryTypedRunStore::new();
        scheduler
            .start_run(
                &mut store,
                &fixture.runtime_spec,
                fixture.run_id.clone(),
                run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]),
            )
            .expect("start run");
        scheduler
            .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
            .await
            .expect("produce first cell");

        let terminal_node = node_by_output(&fixture, &fixture.cell_a);
        let mut corrupt_stream = store.load_run_stream(&fixture.run_id);
        append_payload_commit_for_tests(
            &mut corrupt_stream,
            &fixture.run_id,
            "terminal-node-started-again",
            events::KernelEventPayload::StateAttemptStarted(events::StateAttemptStarted {
                spec_hash: fixture.runtime_spec.spec_hash().clone(),
                node_id: terminal_node.node_id.clone(),
                attempt_id: AttemptId::from_digest(
                    DigestAlgorithm::Sha256JcsV1,
                    DigestBytes::from_array([0x76; 32]),
                ),
                attempt_no: 99,
                state_kind: terminal_node.state_kind.clone(),
                state_version: terminal_node.state_version.clone(),
            }),
        );

        assert!(matches!(
            validate_run_stream(&fixture.runtime_spec, &fixture.run_id, &corrupt_stream),
            Err(RuntimeError::InvalidRunStream(message))
                if message.contains("after its output cell became terminal")
        ));
    }

    #[tokio::test]
    async fn public_output_render_failure_resumes_and_completes() {
        let fixture = fixture();
        let scheduler = test_scheduler(registered_fixture_runners(&fixture));
        let mut store = store::InMemoryTypedRunStore::new();
        scheduler
            .start_run(
                &mut store,
                &fixture.runtime_spec,
                fixture.run_id.clone(),
                run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]),
            )
            .expect("start run");
        scheduler
            .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
            .await
            .expect("drive a");
        scheduler
            .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
            .await
            .expect("drive b");
        let render_node = node_by_output(&fixture, &fixture.render_cell);
        let failed_attempt = append_attempt_start(&mut store, &fixture, render_node, 1);
        append_public_output_render_failure(&mut store, &fixture, render_node, &failed_attempt);
        assert!(matches!(
            store
                .projection_snapshot()
                .public_output(&fixture.runtime_spec.spec().public_outputs.public_schema_id),
            Some(store::PublicOutputProjection::RenderFailed { .. })
        ));

        assert_eq!(
            scheduler
                .drive_until_blocked(&mut store, &fixture.runtime_spec, &fixture.run_id)
                .await
                .expect("retry render"),
            SchedulerStatus::PublicOutputProjected
        );
        assert_eq!(
            attempt_started_count(&store, &fixture.run_id, &fixture.render_node),
            2
        );
        assert_eq!(
            store.projection_snapshot().run_state(&fixture.run_id),
            store::RunState::Completed
        );
        assert!(matches!(
            store
                .projection_snapshot()
                .public_output(&fixture.runtime_spec.spec().public_outputs.public_schema_id),
            Some(store::PublicOutputProjection::Produced { .. })
        ));
    }

    #[test]
    fn certified_runtime_spec_rejects_hash_mismatch() {
        let fixture = fixture();
        let mut envelope = fixture.runtime_spec.envelope().clone();
        envelope.spec_hash = SpecHash::from_digest(DigestAlgorithm::Sha256JcsV1, D9);
        assert!(matches!(
            CertifiedRuntimeSpec::from_verified_envelope(envelope),
            Err(RuntimeError::SpecHash(_))
        ));
    }

    #[test]
    fn certified_runtime_spec_accepts_certifier_authority() {
        let (certified, _registry) = certifier_backed_runtime_authority();
        let runtime = CertifiedRuntimeSpec::new(certified).expect("runtime authority");
        assert!(!runtime.topological_order().is_empty());
    }

    #[test]
    fn certified_runtime_spec_accepts_verified_bundle_authority() {
        let (certified, registry) = certifier_backed_runtime_authority();
        let bundle = certified.bundle().expect("certified bundle");
        let verified = mfm_certify::verify_certified_bundle(
            bundle.spec_bytes(),
            bundle.certificate_bytes(),
            &registry,
        )
        .expect("verified persisted bundle");
        let runtime = CertifiedRuntimeSpec::new(verified).expect("runtime authority");
        assert!(!runtime.topological_order().is_empty());
    }

    #[test]
    fn certified_runtime_spec_requires_framework_public_output_render_node() {
        let fixture = fixture();
        let mut envelope = fixture.runtime_spec.envelope().clone();
        envelope.spec.nodes.retain(|node| {
            !matches!(
                node.framework,
                Some(spec::FrameworkNodeSpec::PublicOutputRender(_))
            )
        });
        let envelope =
            spec::HashedSpecEnvelope::new(envelope.spec, envelope.audit).expect("rehash");
        assert!(matches!(
            CertifiedRuntimeSpec::from_verified_envelope(envelope),
            Err(RuntimeError::InvalidSpec(message))
                if message.contains("public-output render node")
        ));
    }

    #[test]
    fn certified_runtime_spec_rejects_forged_lifecycle_framework_variant() {
        let fixture = fixture();
        let mut envelope = fixture.runtime_spec.envelope().clone();
        let node = envelope
            .spec
            .nodes
            .iter_mut()
            .find(|node| node.framework.is_none())
            .expect("user node");
        node.framework = Some(spec::FrameworkNodeSpec::BootstrapRun(
            spec::BootstrapRunNodeSpec {},
        ));
        let envelope =
            spec::HashedSpecEnvelope::new(envelope.spec, envelope.audit).expect("rehash");

        assert!(matches!(
            CertifiedRuntimeSpec::from_verified_envelope(envelope),
            Err(RuntimeError::InvalidSpec(_))
        ));
    }

    #[test]
    fn certified_runtime_spec_rejects_lifecycle_ordering() {
        let fixture = fixture();
        let mut envelope = fixture.runtime_spec.envelope().clone();
        let public_cell = envelope
            .spec
            .public_outputs
            .outputs
            .first()
            .expect("public output")
            .cell_id
            .clone();
        envelope.spec.nodes.retain(|node| {
            !matches!(
                node.framework,
                Some(spec::FrameworkNodeSpec::ProjectRetentionManifest(_))
            )
        });
        append_runtime_retention_lifecycle_node(&mut envelope.spec, public_cell, false);
        let envelope =
            spec::HashedSpecEnvelope::new(envelope.spec, envelope.audit).expect("rehash");

        assert!(matches!(
            CertifiedRuntimeSpec::from_verified_envelope(envelope),
            Err(RuntimeError::InvalidSpec(_))
        ));
    }

    #[test]
    fn certified_runtime_spec_rejects_lifecycle_framework_permissions() {
        let fixture = fixture();
        let mut envelope = fixture.runtime_spec.envelope().clone();
        find_runtime_lifecycle_node_mut(
            &mut envelope.spec,
            RuntimeLifecycleVariant::ProjectRetentionManifest,
        )
        .adapter_bindings
        .push(spec::AdapterBinding {
            adapter_kind: AdapterKind::new(
                "mfm.runtime.test",
                "forged-adapter",
                DigestAlgorithm::Sha256JcsV1,
                DigestBytes::from_array([0xf7; 32]),
            )
            .expect("adapter kind"),
            adapter_version: AdapterVersion::new("mfm.runtime.test.adapter.v1")
                .expect("adapter version"),
            binding_digest: None,
        });
        let envelope =
            spec::HashedSpecEnvelope::new(envelope.spec, envelope.audit).expect("rehash");

        assert!(matches!(
            CertifiedRuntimeSpec::from_verified_envelope(envelope),
            Err(RuntimeError::InvalidSpec(message))
                if message.contains("user-controlled execution permissions")
        ));
    }

    #[test]
    fn certified_runtime_spec_rejects_user_consumers_of_lifecycle_receipts() {
        let fixture = fixture();

        let mut bootstrap_envelope = fixture.runtime_spec.envelope().clone();
        let bootstrap_receipt =
            append_runtime_bootstrap_lifecycle_node(&mut bootstrap_envelope.spec);
        append_runtime_user_receipt_consumer(
            &mut bootstrap_envelope.spec,
            bootstrap_receipt,
            "user/bootstrap-receipt",
        );
        let bootstrap_envelope =
            spec::HashedSpecEnvelope::new(bootstrap_envelope.spec, bootstrap_envelope.audit)
                .expect("rehash");
        assert!(matches!(
            CertifiedRuntimeSpec::from_verified_envelope(bootstrap_envelope),
            Err(RuntimeError::InvalidSpec(message))
                if message.contains("must not be consumed")
        ));

        let mut render_envelope = fixture.runtime_spec.envelope().clone();
        let render_receipt = runtime_render_receipt_cell(&render_envelope.spec);
        append_runtime_user_receipt_consumer(
            &mut render_envelope.spec,
            render_receipt,
            "user/render-receipt",
        );
        let render_envelope =
            spec::HashedSpecEnvelope::new(render_envelope.spec, render_envelope.audit)
                .expect("rehash");
        assert!(matches!(
            CertifiedRuntimeSpec::from_verified_envelope(render_envelope),
            Err(RuntimeError::InvalidSpec(message))
                if message.contains("project-retention-manifest")
        ));

        let mut retention_envelope = fixture.runtime_spec.envelope().clone();
        let retention_receipt = runtime_retention_receipt_cell(&retention_envelope.spec);
        append_runtime_user_receipt_consumer(
            &mut retention_envelope.spec,
            retention_receipt,
            "user/retention-receipt",
        );
        let retention_envelope =
            spec::HashedSpecEnvelope::new(retention_envelope.spec, retention_envelope.audit)
                .expect("rehash");
        assert!(matches!(
            CertifiedRuntimeSpec::from_verified_envelope(retention_envelope),
            Err(RuntimeError::InvalidSpec(message)) if message.contains("complete-run")
        ));
    }

    #[test]
    fn certified_runtime_spec_rejects_executable_nodes_outside_lifecycle_tail() {
        let fixture = fixture();
        let mut envelope = fixture.runtime_spec.envelope().clone();
        append_runtime_independent_user_node(&mut envelope.spec, "user/outside-lifecycle-tail");
        let envelope =
            spec::HashedSpecEnvelope::new(envelope.spec, envelope.audit).expect("rehash");

        assert!(matches!(
            CertifiedRuntimeSpec::from_verified_envelope(envelope),
            Err(RuntimeError::InvalidSpec(message))
                if message.contains("outside the certified public-output lifecycle tail")
        ));
    }

    #[test]
    fn certified_runtime_spec_rejects_forged_lifecycle_config_ref() {
        let fixture = fixture();
        let mut envelope = fixture.runtime_spec.envelope().clone();
        find_runtime_lifecycle_node_mut(
            &mut envelope.spec,
            RuntimeLifecycleVariant::ProjectRetentionManifest,
        )
        .config_ref
        .byte_len += 1;
        let envelope =
            spec::HashedSpecEnvelope::new(envelope.spec, envelope.audit).expect("rehash");

        assert!(matches!(
            CertifiedRuntimeSpec::from_verified_envelope(envelope),
            Err(RuntimeError::InvalidSpec(message))
                if message.contains("config ref is not deterministic")
        ));
    }

    #[test]
    fn certified_runtime_spec_accepts_lifecycle_framework_node_chain() {
        let fixture = fixture();
        let mut envelope = fixture.runtime_spec.envelope().clone();
        append_runtime_bootstrap_lifecycle_node(&mut envelope.spec);
        let envelope =
            spec::HashedSpecEnvelope::new(envelope.spec, envelope.audit).expect("rehash");

        CertifiedRuntimeSpec::from_verified_envelope(envelope)
            .expect("lifecycle framework chain is valid runtime authority");
    }

    #[test]
    fn certified_runtime_spec_rejects_missing_retention_lifecycle_node() {
        let fixture = fixture();
        let mut envelope = fixture.runtime_spec.envelope().clone();
        envelope.spec.nodes.retain(|node| {
            !matches!(
                node.framework,
                Some(spec::FrameworkNodeSpec::ProjectRetentionManifest(_))
            )
        });
        let envelope =
            spec::HashedSpecEnvelope::new(envelope.spec, envelope.audit).expect("rehash");

        assert!(matches!(
            CertifiedRuntimeSpec::from_verified_envelope(envelope),
            Err(RuntimeError::InvalidSpec(message))
                if message.contains("retention")
                    || message.contains("project-retention-manifest")
        ));
    }

    #[test]
    fn certified_runtime_spec_rejects_builtin_lifecycle_descriptors_without_framework_metadata() {
        let fixture = fixture();

        let mut bootstrap_envelope = fixture.runtime_spec.envelope().clone();
        append_runtime_bootstrap_lifecycle_node(&mut bootstrap_envelope.spec);
        clear_runtime_framework_metadata(
            &mut bootstrap_envelope.spec,
            RuntimeLifecycleVariant::BootstrapRun,
        );
        let bootstrap_envelope =
            spec::HashedSpecEnvelope::new(bootstrap_envelope.spec, bootstrap_envelope.audit)
                .expect("rehash");
        assert!(matches!(
            CertifiedRuntimeSpec::from_verified_envelope(bootstrap_envelope),
            Err(RuntimeError::InvalidSpec(message))
                if message.contains("without matching framework metadata")
        ));

        let mut retention_envelope = fixture.runtime_spec.envelope().clone();
        clear_runtime_framework_metadata(
            &mut retention_envelope.spec,
            RuntimeLifecycleVariant::ProjectRetentionManifest,
        );
        let retention_envelope =
            spec::HashedSpecEnvelope::new(retention_envelope.spec, retention_envelope.audit)
                .expect("rehash");
        assert!(matches!(
            CertifiedRuntimeSpec::from_verified_envelope(retention_envelope),
            Err(RuntimeError::InvalidSpec(message))
                if message.contains("without matching framework metadata")
        ));

        let mut complete_envelope = fixture.runtime_spec.envelope().clone();
        clear_runtime_framework_metadata(
            &mut complete_envelope.spec,
            RuntimeLifecycleVariant::CompleteRun,
        );
        let complete_envelope =
            spec::HashedSpecEnvelope::new(complete_envelope.spec, complete_envelope.audit)
                .expect("rehash");
        assert!(matches!(
            CertifiedRuntimeSpec::from_verified_envelope(complete_envelope),
            Err(RuntimeError::InvalidSpec(message))
                if message.contains("without matching framework metadata")
        ));
    }

    #[test]
    fn certified_runtime_spec_rejects_forged_lifecycle_input_binding() {
        let fixture = fixture();

        let mut bootstrap_envelope = fixture.runtime_spec.envelope().clone();
        append_runtime_bootstrap_lifecycle_node(&mut bootstrap_envelope.spec);
        let node = find_runtime_lifecycle_node_mut(
            &mut bootstrap_envelope.spec,
            RuntimeLifecycleVariant::BootstrapRun,
        );
        node.input_bindings.input_descriptor_id = DescriptorId::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([0xb1; 32]),
        );
        let bootstrap_envelope =
            spec::HashedSpecEnvelope::new(bootstrap_envelope.spec, bootstrap_envelope.audit)
                .expect("rehash");
        assert!(matches!(
            CertifiedRuntimeSpec::from_verified_envelope(bootstrap_envelope),
            Err(RuntimeError::InvalidSpec(message))
                if message.contains("input binding is not deterministic")
        ));

        let mut retention_envelope = fixture.runtime_spec.envelope().clone();
        let node = find_runtime_lifecycle_node_mut(
            &mut retention_envelope.spec,
            RuntimeLifecycleVariant::ProjectRetentionManifest,
        );
        let current = match &node.input_bindings.root {
            spec::InputBindingNodeSpec::Cell(cell) => cell.clone(),
            _ => panic!("expected cell input"),
        };
        node.input_bindings.root =
            spec::InputBindingNodeSpec::Struct(vec![spec::NamedInputBindingSpec {
                field_path: spec::PublicFieldPath::new("public_output_receipt")
                    .expect("field path"),
                node: spec::InputBindingNodeSpec::Cell(current),
            }]);
        node.input_bindings.digest =
            content_digest_json(input_node_json(&node.input_bindings.root)).expect("input digest");
        let retention_envelope =
            spec::HashedSpecEnvelope::new(retention_envelope.spec, retention_envelope.audit)
                .expect("rehash");
        assert!(matches!(
            CertifiedRuntimeSpec::from_verified_envelope(retention_envelope),
            Err(RuntimeError::InvalidSpec(message))
                if message.contains("input binding is not deterministic")
        ));

        let mut complete_envelope = fixture.runtime_spec.envelope().clone();
        let node = find_runtime_lifecycle_node_mut(
            &mut complete_envelope.spec,
            RuntimeLifecycleVariant::CompleteRun,
        );
        let current = match &node.input_bindings.root {
            spec::InputBindingNodeSpec::Cell(cell) => cell.clone(),
            _ => panic!("expected cell input"),
        };
        node.input_bindings.root =
            spec::InputBindingNodeSpec::Struct(vec![spec::NamedInputBindingSpec {
                field_path: spec::PublicFieldPath::new("retention_manifest_receipt")
                    .expect("field path"),
                node: spec::InputBindingNodeSpec::Cell(current),
            }]);
        node.input_bindings.digest =
            content_digest_json(input_node_json(&node.input_bindings.root)).expect("input digest");
        let complete_envelope =
            spec::HashedSpecEnvelope::new(complete_envelope.spec, complete_envelope.audit)
                .expect("rehash");
        assert!(matches!(
            CertifiedRuntimeSpec::from_verified_envelope(complete_envelope),
            Err(RuntimeError::InvalidSpec(message))
                if message.contains("input binding is not deterministic")
        ));
    }

    #[tokio::test]
    async fn replay_rejects_run_completed_without_public_output_evidence() {
        let fixture = fixture();
        let scheduler = test_scheduler(registered_fixture_runners(&fixture));
        let mut store = store::InMemoryTypedRunStore::new();
        scheduler
            .start_run(
                &mut store,
                &fixture.runtime_spec,
                fixture.run_id.clone(),
                run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]),
            )
            .expect("start run");
        store
            .append_prepared_commit(store::TypedCommitRequest {
                run_id: fixture.run_id.clone(),
                expected_next_seq: store.expected_next_seq(&fixture.run_id),
                commit_key: store::CommitKey::new("forged-complete-without-public-output")
                    .expect("commit key"),
                payloads: vec![events::KernelEventPayload::RunCompleted(
                    events::RunCompleted {
                        run_id: fixture.run_id.clone(),
                        spec_hash: fixture.runtime_spec.spec_hash().clone(),
                        outcome: events::RunCompletionOutcome::Completed(
                            events::PublicOutputCompletionEvidence {
                                public_output_schema_id: fixture
                                    .runtime_spec
                                    .spec()
                                    .public_outputs
                                    .public_schema_id
                                    .clone(),
                                public_output_event_id: EventId::from_digest(
                                    DigestAlgorithm::Sha256JcsV1,
                                    D9,
                                ),
                            },
                        ),
                    },
                )],
                required_artifacts: Vec::new(),
                preconditions: store::CommitPreconditions {
                    required_run_state: store::RequiredRunState::NotCompleted,
                    ..store::CommitPreconditions::default()
                },
            })
            .expect("append forged completion");

        assert!(matches!(
            scheduler
                .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
                .await,
            Err(RuntimeError::InvalidRunStream(message))
                if message.contains("RunCompleted appeared before PublicOutputProduced")
        ));
    }

    #[tokio::test]
    async fn scheduler_rejects_uncertified_capability_use() {
        struct BadFactRunner {
            cap_kind: CapabilityKind,
            cap_version: CapabilityVersion,
        }

        impl ErasedNodeRunner for BadFactRunner {
            fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
                Box::pin(async move {
                    Ok(ErasedRunnerOutput::new(vec![
                        events::KernelEventPayload::FactRecorded(events::FactRecorded {
                            spec_hash: ctx.spec_hash().clone(),
                            node_id: ctx.node().node_id.clone(),
                            attempt_id: ctx.attempt_id().clone(),
                            capability_kind: self.cap_kind.clone(),
                            capability_version: self.cap_version.clone(),
                            adapter_kind: AdapterKind::new(
                                "mfm.test",
                                "adapter",
                                DigestAlgorithm::Sha256JcsV1,
                                D1,
                            )
                            .expect("adapter"),
                            adapter_version: AdapterVersion::new("mfm.adapter.v1")
                                .expect("adapter version"),
                            request_schema_id: ctx.node().config_ref.schema_id.clone(),
                            request_hash: content(0xc1),
                            response_schema_id: ctx.node().config_ref.schema_id.clone(),
                            response_hash: content(0xc2),
                            fact_key: events::FactKey::new("bad-fact").expect("fact key"),
                            artifact_id: artifact(0xc3),
                        }),
                    ]))
                })
            }
        }

        let fixture = fixture();
        let mut registry = ErasedRunnerRegistry::new();
        registry
            .register(binding(
                fixture.descriptor_a.clone(),
                "pure",
                BadFactRunner {
                    cap_kind: fixture.cap_kind.clone(),
                    cap_version: fixture.cap_version.clone(),
                },
            ))
            .expect("binding");
        registry
            .register(binding(
                fixture.descriptor_b.clone(),
                "read",
                RecordingRunner {
                    expected_caps: vec![(fixture.cap_kind.clone(), fixture.cap_version.clone())],
                    output_artifact: artifact(0xb1),
                    output_digest: content(0xb2),
                },
            ))
            .expect("binding b");
        let scheduler = test_scheduler(registry);
        let mut store = store::InMemoryTypedRunStore::new();
        scheduler
            .start_run(
                &mut store,
                &fixture.runtime_spec,
                fixture.run_id.clone(),
                run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]),
            )
            .expect("start run");
        assert!(matches!(
            scheduler
                .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
                .await,
            Err(RuntimeError::InvalidRunnerOutput(_))
        ));
    }

    #[tokio::test]
    async fn runner_cannot_return_scheduler_owned_lifecycle_payload() {
        struct SchedulerOwnedPayloadRunner {
            public_schema_id: SchemaId,
        }

        impl ErasedNodeRunner for SchedulerOwnedPayloadRunner {
            fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
                Box::pin(async move {
                    Ok(ErasedRunnerOutput::new(vec![
                        events::KernelEventPayload::RunCompleted(events::RunCompleted {
                            run_id: ctx.run_id().clone(),
                            spec_hash: ctx.spec_hash().clone(),
                            outcome: events::RunCompletionOutcome::Completed(
                                events::PublicOutputCompletionEvidence {
                                    public_output_schema_id: self.public_schema_id.clone(),
                                    public_output_event_id: EventId::from_digest(
                                        DigestAlgorithm::Sha256JcsV1,
                                        D9,
                                    ),
                                },
                            ),
                        }),
                    ]))
                })
            }
        }

        let fixture = fixture();
        let mut registry = ErasedRunnerRegistry::new();
        registry
            .register(binding(
                fixture.descriptor_a.clone(),
                "pure",
                SchedulerOwnedPayloadRunner {
                    public_schema_id: fixture
                        .runtime_spec
                        .spec()
                        .public_outputs
                        .public_schema_id
                        .clone(),
                },
            ))
            .expect("binding a");
        registry
            .register(binding(
                fixture.descriptor_b.clone(),
                "read",
                RecordingRunner {
                    expected_caps: vec![(fixture.cap_kind.clone(), fixture.cap_version.clone())],
                    output_artifact: artifact(0xb1),
                    output_digest: content(0xb2),
                },
            ))
            .expect("binding b");
        let scheduler = test_scheduler(registry);
        let mut store = store::InMemoryTypedRunStore::new();
        scheduler
            .start_run(
                &mut store,
                &fixture.runtime_spec,
                fixture.run_id.clone(),
                run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]),
            )
            .expect("start run");

        assert!(matches!(
            scheduler
                .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
                .await,
            Err(RuntimeError::InvalidRunnerOutput(message))
                if message.contains("scheduler-owned payload")
        ));
    }

    #[tokio::test]
    async fn runner_cannot_stage_artifact_with_foreign_producer() {
        struct ForeignProducerArtifactRunner {
            foreign_node_id: NodeId,
            output_artifact: ArtifactId,
            output_digest: ContentDigest,
        }

        impl ErasedNodeRunner for ForeignProducerArtifactRunner {
            fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
                Box::pin(async move {
                    let mut artifact = state_output_artifact(
                        ctx.node(),
                        ctx.descriptor(),
                        self.output_artifact.clone(),
                        self.output_digest.clone(),
                    );
                    artifact.producer_node_id = Some(self.foreign_node_id.clone());
                    let staged_artifact = staged_attempt_artifact(&ctx, artifact)?;
                    Ok(ErasedRunnerOutput {
                        staged_artifacts: vec![staged_artifact],
                        staged_retention_refs: Vec::new(),
                        payloads: terminal_payloads(
                            &ctx,
                            self.output_artifact.clone(),
                            self.output_digest.clone(),
                        ),
                    })
                })
            }
        }

        let fixture = fixture();
        let foreign_node_id = node_by_output(&fixture, &fixture.cell_b).node_id.clone();
        let mut registry = ErasedRunnerRegistry::new();
        registry
            .register(binding(
                fixture.descriptor_a.clone(),
                "pure",
                ForeignProducerArtifactRunner {
                    foreign_node_id,
                    output_artifact: artifact(0xa1),
                    output_digest: content(0xa2),
                },
            ))
            .expect("binding a");
        registry
            .register(binding(
                fixture.descriptor_b.clone(),
                "read",
                RecordingRunner {
                    expected_caps: vec![(fixture.cap_kind.clone(), fixture.cap_version.clone())],
                    output_artifact: artifact(0xb1),
                    output_digest: content(0xb2),
                },
            ))
            .expect("binding b");
        let scheduler = test_scheduler(registry);
        let mut store = store::InMemoryTypedRunStore::new();
        scheduler
            .start_run(
                &mut store,
                &fixture.runtime_spec,
                fixture.run_id.clone(),
                run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]),
            )
            .expect("start run");

        assert!(matches!(
            scheduler
                .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
                .await,
            Err(RuntimeError::InvalidRunnerOutput(message))
                if message.contains("producer evidence outside its attempt")
        ));
    }

    #[tokio::test]
    async fn runner_cannot_stage_inline_artifact_with_mismatched_bytes() {
        struct BadInlineArtifactRunner {
            output_artifact: ArtifactId,
            output_digest: ContentDigest,
        }

        impl ErasedNodeRunner for BadInlineArtifactRunner {
            fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
                Box::pin(async move {
                    let artifact = state_output_artifact(
                        ctx.node(),
                        ctx.descriptor(),
                        self.output_artifact.clone(),
                        self.output_digest.clone(),
                    );
                    let staged_artifact = StagedArtifact::inline_attempt_artifact(
                        &ctx,
                        b"mismatched".to_vec(),
                        artifact,
                    )?;
                    Ok(ErasedRunnerOutput {
                        staged_artifacts: vec![staged_artifact],
                        staged_retention_refs: Vec::new(),
                        payloads: terminal_payloads(
                            &ctx,
                            self.output_artifact.clone(),
                            self.output_digest.clone(),
                        ),
                    })
                })
            }
        }

        let fixture = fixture();
        let mut registry = ErasedRunnerRegistry::new();
        registry
            .register(binding(
                fixture.descriptor_a.clone(),
                "pure",
                BadInlineArtifactRunner {
                    output_artifact: artifact(0xa1),
                    output_digest: content(0xa2),
                },
            ))
            .expect("binding a");
        registry
            .register(binding(
                fixture.descriptor_b.clone(),
                "read",
                RecordingRunner {
                    expected_caps: vec![(fixture.cap_kind.clone(), fixture.cap_version.clone())],
                    output_artifact: artifact(0xb1),
                    output_digest: content(0xb2),
                },
            ))
            .expect("binding b");
        let scheduler = test_scheduler(registry);
        let mut store = store::InMemoryTypedRunStore::new();
        scheduler
            .start_run(
                &mut store,
                &fixture.runtime_spec,
                fixture.run_id.clone(),
                run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]),
            )
            .expect("start run");

        assert!(matches!(
            scheduler
                .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
                .await,
            Err(RuntimeError::InvalidRunnerOutput(message))
                if message.contains("bytes do not match typed evidence")
        ));
    }

    #[tokio::test]
    async fn runner_can_commit_inline_state_output_artifact() {
        struct InlineArtifactRunner {
            output_bytes: Vec<u8>,
        }

        impl ErasedNodeRunner for InlineArtifactRunner {
            fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
                Box::pin(async move {
                    let artifact = state_output_artifact_for_bytes(
                        ctx.node(),
                        ctx.descriptor(),
                        &self.output_bytes,
                    );
                    let staged_artifact = StagedArtifact::inline_attempt_artifact(
                        &ctx,
                        self.output_bytes.clone(),
                        artifact.clone(),
                    )?;
                    Ok(ErasedRunnerOutput {
                        staged_artifacts: vec![staged_artifact],
                        staged_retention_refs: Vec::new(),
                        payloads: terminal_payloads(
                            &ctx,
                            artifact.artifact_id.clone(),
                            artifact.digest.clone(),
                        ),
                    })
                })
            }
        }

        let fixture = fixture();
        let output_bytes = br#"{"inline":true}"#.to_vec();
        let output_digest = digest_for_bytes(&output_bytes);
        let output_artifact =
            ArtifactId::from_digest(output_digest.algorithm(), *output_digest.digest());
        let mut registry = ErasedRunnerRegistry::new();
        registry
            .register(binding(
                fixture.descriptor_a.clone(),
                "pure",
                InlineArtifactRunner { output_bytes },
            ))
            .expect("binding a");
        registry
            .register(binding(
                fixture.descriptor_b.clone(),
                "read",
                RecordingRunner {
                    expected_caps: vec![(fixture.cap_kind.clone(), fixture.cap_version.clone())],
                    output_artifact: artifact(0xb1),
                    output_digest: content(0xb2),
                },
            ))
            .expect("binding b");
        let scheduler = test_scheduler(registry);
        let mut store = store::InMemoryTypedRunStore::new();
        scheduler
            .start_run(
                &mut store,
                &fixture.runtime_spec,
                fixture.run_id.clone(),
                run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]),
            )
            .expect("start run");

        assert_eq!(
            scheduler
                .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
                .await
                .expect("drive inline output"),
            SchedulerStatus::Advanced
        );
        assert!(matches!(
            store.projection_snapshot().cell_terminal(&fixture.cell_a),
            Some(store::CellTerminalProjection::Produced {
                artifact_id,
                content_digest,
                ..
            }) if artifact_id == &output_artifact && content_digest == &output_digest
        ));
    }

    #[tokio::test]
    async fn runner_cannot_stage_reserved_retention_reasons() {
        struct ReservedRetentionReasonRunner {
            output_bytes: Vec<u8>,
            reason: events::RetentionReason,
        }

        impl ErasedNodeRunner for ReservedRetentionReasonRunner {
            fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
                Box::pin(async move {
                    let artifact = state_output_artifact_for_bytes(
                        ctx.node(),
                        ctx.descriptor(),
                        &self.output_bytes,
                    );
                    let staged_artifact = StagedArtifact::inline_attempt_artifact(
                        &ctx,
                        self.output_bytes.clone(),
                        artifact.clone(),
                    )?;
                    Ok(ErasedRunnerOutput {
                        staged_artifacts: vec![staged_artifact],
                        staged_retention_refs: vec![StagedRetentionRefs {
                            refs: vec![retention_ref_for_artifact(&artifact)],
                            reason: self.reason,
                        }],
                        payloads: terminal_payloads(
                            &ctx,
                            artifact.artifact_id.clone(),
                            artifact.digest.clone(),
                        ),
                    })
                })
            }
        }

        for reason in [
            events::RetentionReason::RunStarted,
            events::RetentionReason::ManifestProjection,
            events::RetentionReason::PublicOutput,
        ] {
            let fixture = fixture();
            let node = node_by_output(&fixture, &fixture.cell_a).clone();
            let mut registry = ErasedRunnerRegistry::new();
            registry
                .register(binding(
                    fixture.descriptor_a.clone(),
                    "pure",
                    ReservedRetentionReasonRunner {
                        output_bytes: br#"{"reserved":true}"#.to_vec(),
                        reason,
                    },
                ))
                .expect("binding a");
            registry
                .register(binding(
                    fixture.descriptor_b.clone(),
                    "read",
                    RecordingRunner {
                        expected_caps: vec![(
                            fixture.cap_kind.clone(),
                            fixture.cap_version.clone(),
                        )],
                        output_artifact: artifact(0xb1),
                        output_digest: content(0xb2),
                    },
                ))
                .expect("binding b");
            let scheduler = test_scheduler(registry);
            let mut store = store::InMemoryTypedRunStore::new();
            scheduler
                .start_run(
                    &mut store,
                    &fixture.runtime_spec,
                    fixture.run_id.clone(),
                    run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]),
                )
                .expect("start run");
            append_attempt_start(&mut store, &fixture, &node, 1);
            let stream_before = store.load_run_stream(&fixture.run_id);

            assert!(matches!(
                scheduler
                    .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
                    .await,
                Err(RuntimeError::InvalidRunnerOutput(message))
                    if message.contains("middleware-owned retention reason")
                        || message.contains("public-output retention outside sealed framework")
            ));
            assert_eq!(store.load_run_stream(&fixture.run_id), stream_before);
            assert!(store
                .projection_snapshot()
                .cell_terminal(&fixture.cell_a)
                .is_none());
        }
    }

    #[tokio::test]
    async fn runtime_rejects_public_output_retention_reason_on_user_commit() {
        struct RetainedStateOutputRunner {
            output_bytes: Vec<u8>,
        }

        impl ErasedNodeRunner for RetainedStateOutputRunner {
            fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
                Box::pin(async move {
                    let artifact = state_output_artifact_for_bytes(
                        ctx.node(),
                        ctx.descriptor(),
                        &self.output_bytes,
                    );
                    let staged_artifact = StagedArtifact::inline_attempt_artifact(
                        &ctx,
                        self.output_bytes.clone(),
                        artifact.clone(),
                    )?;
                    Ok(ErasedRunnerOutput {
                        staged_artifacts: vec![staged_artifact],
                        staged_retention_refs: vec![StagedRetentionRefs::runtime_evidence(vec![
                            retention_ref_for_artifact(&artifact),
                        ])],
                        payloads: terminal_payloads(
                            &ctx,
                            artifact.artifact_id.clone(),
                            artifact.digest.clone(),
                        ),
                    })
                })
            }
        }

        let fixture = fixture();
        let mut registry = ErasedRunnerRegistry::new();
        registry
            .register(binding(
                fixture.descriptor_a.clone(),
                "pure",
                RetainedStateOutputRunner {
                    output_bytes: br#"{"retained":true}"#.to_vec(),
                },
            ))
            .expect("binding a");
        registry
            .register(binding(
                fixture.descriptor_b.clone(),
                "read",
                RecordingRunner {
                    expected_caps: vec![(fixture.cap_kind.clone(), fixture.cap_version.clone())],
                    output_artifact: artifact(0xb1),
                    output_digest: content(0xb2),
                },
            ))
            .expect("binding b");
        let scheduler = test_scheduler(registry);
        let mut store = store::InMemoryTypedRunStore::new();
        scheduler
            .start_run(
                &mut store,
                &fixture.runtime_spec,
                fixture.run_id.clone(),
                run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]),
            )
            .expect("start run");
        assert_eq!(
            scheduler
                .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
                .await
                .expect("drive retained state output"),
            SchedulerStatus::Advanced
        );

        let valid_stream = store.load_run_stream(&fixture.run_id);
        let corrupt_pos = valid_stream
            .iter()
            .position(|event| {
                matches!(
                    event.payload(),
                    events::KernelEventPayload::RetentionRefsAppended(
                        events::RetentionRefsAppended {
                            reason: events::RetentionReason::RuntimeEvidence,
                            ..
                        }
                    )
                )
            })
            .expect("runtime evidence retention refs");
        let mut corrupt_payload = match valid_stream[corrupt_pos].payload().clone() {
            events::KernelEventPayload::RetentionRefsAppended(payload) => payload,
            _ => unreachable!("position checked"),
        };
        corrupt_payload.reason = events::RetentionReason::PublicOutput;
        let corrupt_stream = rewrite_commit_payload(
            &valid_stream,
            corrupt_pos,
            events::KernelEventPayload::RetentionRefsAppended(corrupt_payload),
        );

        assert!(matches!(
            validate_run_stream(&fixture.runtime_spec, &fixture.run_id, &corrupt_stream),
            Err(RuntimeError::InvalidRunStream(message))
                if message.contains("public-output retention refs must be appended")
        ));
    }

    #[tokio::test]
    async fn runner_cannot_return_artifact_reference_payload() {
        struct ArtifactReferencePayloadRunner {
            output_bytes: Vec<u8>,
        }

        impl ErasedNodeRunner for ArtifactReferencePayloadRunner {
            fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
                Box::pin(async move {
                    let artifact = state_output_artifact_for_bytes(
                        ctx.node(),
                        ctx.descriptor(),
                        &self.output_bytes,
                    );
                    let staged_artifact = StagedArtifact::inline_attempt_artifact(
                        &ctx,
                        self.output_bytes.clone(),
                        artifact.clone(),
                    )?;
                    let mut artifact_ref = event_artifact_ref_from_store(&artifact);
                    artifact_ref.byte_len += 1;
                    let mut payloads = terminal_payloads(
                        &ctx,
                        artifact.artifact_id.clone(),
                        artifact.digest.clone(),
                    );
                    payloads.push(events::KernelEventPayload::ArtifactReferenced(
                        events::ArtifactReferenced {
                            spec_hash: ctx.spec_hash().clone(),
                            node_id: Some(ctx.node().node_id.clone()),
                            attempt_id: Some(ctx.attempt_id().clone()),
                            artifact_ref,
                        },
                    ));
                    Ok(ErasedRunnerOutput {
                        staged_artifacts: vec![staged_artifact],
                        staged_retention_refs: Vec::new(),
                        payloads,
                    })
                })
            }
        }

        let fixture = fixture();
        let mut registry = ErasedRunnerRegistry::new();
        registry
            .register(binding(
                fixture.descriptor_a.clone(),
                "pure",
                ArtifactReferencePayloadRunner {
                    output_bytes: br#"{"metadata":"payload"}"#.to_vec(),
                },
            ))
            .expect("binding a");
        registry
            .register(binding(
                fixture.descriptor_b.clone(),
                "read",
                RecordingRunner {
                    expected_caps: vec![(fixture.cap_kind.clone(), fixture.cap_version.clone())],
                    output_artifact: artifact(0xb1),
                    output_digest: content(0xb2),
                },
            ))
            .expect("binding b");
        let scheduler = test_scheduler(registry);
        let mut store = store::InMemoryTypedRunStore::new();
        scheduler
            .start_run(
                &mut store,
                &fixture.runtime_spec,
                fixture.run_id.clone(),
                run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]),
            )
            .expect("start run");

        assert!(matches!(
            scheduler
                .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
                .await,
            Err(RuntimeError::InvalidRunnerOutput(message))
                if message.contains("returned artifact reference payload")
        ));
    }

    #[tokio::test]
    async fn runner_cannot_bind_extra_staged_artifact_with_artifact_reference_only() {
        struct ExtraReferencedArtifactRunner {
            output_bytes: Vec<u8>,
            extra_bytes: Vec<u8>,
        }

        impl ErasedNodeRunner for ExtraReferencedArtifactRunner {
            fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
                Box::pin(async move {
                    let output_artifact = state_output_artifact_for_bytes(
                        ctx.node(),
                        ctx.descriptor(),
                        &self.output_bytes,
                    );
                    let output_staged_artifact = StagedArtifact::inline_attempt_artifact(
                        &ctx,
                        self.output_bytes.clone(),
                        output_artifact.clone(),
                    )?;
                    let extra_digest = digest_for_bytes(&self.extra_bytes);
                    let extra_artifact = store::ArtifactEvidenceRef {
                        artifact_id: ArtifactId::from_digest(
                            extra_digest.algorithm(),
                            *extra_digest.digest(),
                        ),
                        digest: extra_digest,
                        byte_len: self.extra_bytes.len() as u64,
                        media_type: spec::MediaType::new("application/json").expect("media"),
                        schema_id: Some(ctx.node().config_ref.schema_id.clone()),
                        semantic_type_id: None,
                        producer_node_id: Some(ctx.node().node_id.clone()),
                        producer_seed_id: None,
                        artifact_role: events::ArtifactRole::FactResponse,
                    };
                    let extra_staged_artifact = StagedArtifact::inline_attempt_artifact(
                        &ctx,
                        self.extra_bytes.clone(),
                        extra_artifact.clone(),
                    )?;
                    let mut payloads = terminal_payloads(
                        &ctx,
                        output_artifact.artifact_id.clone(),
                        output_artifact.digest.clone(),
                    );
                    payloads.push(events::KernelEventPayload::ArtifactReferenced(
                        events::ArtifactReferenced {
                            spec_hash: ctx.spec_hash().clone(),
                            node_id: Some(ctx.node().node_id.clone()),
                            attempt_id: Some(ctx.attempt_id().clone()),
                            artifact_ref: event_artifact_ref_from_store(&extra_artifact),
                        },
                    ));
                    Ok(ErasedRunnerOutput {
                        staged_artifacts: vec![output_staged_artifact, extra_staged_artifact],
                        staged_retention_refs: Vec::new(),
                        payloads,
                    })
                })
            }
        }

        let fixture = fixture();
        let mut registry = ErasedRunnerRegistry::new();
        registry
            .register(binding(
                fixture.descriptor_a.clone(),
                "pure",
                ExtraReferencedArtifactRunner {
                    output_bytes: br#"{"terminal":"output"}"#.to_vec(),
                    extra_bytes: br#"{"extra":"artifact"}"#.to_vec(),
                },
            ))
            .expect("binding a");
        registry
            .register(binding(
                fixture.descriptor_b.clone(),
                "read",
                RecordingRunner {
                    expected_caps: vec![(fixture.cap_kind.clone(), fixture.cap_version.clone())],
                    output_artifact: artifact(0xb1),
                    output_digest: content(0xb2),
                },
            ))
            .expect("binding b");
        let scheduler = test_scheduler(registry);
        let mut store = store::InMemoryTypedRunStore::new();
        scheduler
            .start_run(
                &mut store,
                &fixture.runtime_spec,
                fixture.run_id.clone(),
                run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]),
            )
            .expect("start run");

        assert!(matches!(
            scheduler
                .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
                .await,
            Err(RuntimeError::InvalidRunnerOutput(message))
                if message.contains("returned artifact reference payload")
        ));
        assert!(store
            .projection_snapshot()
            .cell_terminal(&fixture.cell_a)
            .is_none());
    }

    #[tokio::test]
    async fn runner_output_requires_payload_bound_staged_artifact() {
        struct MissingStagedArtifactRunner {
            output_artifact: ArtifactId,
            output_digest: ContentDigest,
        }

        impl ErasedNodeRunner for MissingStagedArtifactRunner {
            fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
                Box::pin(async move {
                    Ok(ErasedRunnerOutput {
                        staged_artifacts: Vec::new(),
                        staged_retention_refs: Vec::new(),
                        payloads: terminal_payloads(
                            &ctx,
                            self.output_artifact.clone(),
                            self.output_digest.clone(),
                        ),
                    })
                })
            }
        }

        let fixture = fixture();
        let mut registry = ErasedRunnerRegistry::new();
        registry
            .register(binding(
                fixture.descriptor_a.clone(),
                "pure",
                MissingStagedArtifactRunner {
                    output_artifact: artifact(0xa1),
                    output_digest: content(0xa2),
                },
            ))
            .expect("binding a");
        registry
            .register(binding(
                fixture.descriptor_b.clone(),
                "read",
                RecordingRunner {
                    expected_caps: vec![(fixture.cap_kind.clone(), fixture.cap_version.clone())],
                    output_artifact: artifact(0xb1),
                    output_digest: content(0xb2),
                },
            ))
            .expect("binding b");
        let scheduler = test_scheduler(registry);
        let mut store = store::InMemoryTypedRunStore::new();
        scheduler
            .start_run(
                &mut store,
                &fixture.runtime_spec,
                fixture.run_id.clone(),
                run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]),
            )
            .expect("start run");

        assert!(matches!(
            scheduler
                .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
                .await,
            Err(RuntimeError::InvalidRunnerOutput(message))
                if message.contains("without staged artifact")
        ));
    }

    #[tokio::test]
    async fn rejected_staged_payload_mismatch_does_not_admit_artifact_evidence() {
        struct MismatchedStagedArtifactRunner {
            staged_artifact: ArtifactId,
            staged_digest: ContentDigest,
            payload_artifact: ArtifactId,
            payload_digest: ContentDigest,
        }

        impl ErasedNodeRunner for MismatchedStagedArtifactRunner {
            fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
                Box::pin(async move {
                    let artifact = state_output_artifact(
                        ctx.node(),
                        ctx.descriptor(),
                        self.staged_artifact.clone(),
                        self.staged_digest.clone(),
                    );
                    let staged_artifact = staged_attempt_artifact(&ctx, artifact)?;
                    Ok(ErasedRunnerOutput {
                        staged_artifacts: vec![staged_artifact],
                        staged_retention_refs: Vec::new(),
                        payloads: terminal_payloads(
                            &ctx,
                            self.payload_artifact.clone(),
                            self.payload_digest.clone(),
                        ),
                    })
                })
            }
        }

        let fixture = fixture();
        let node = node_by_output(&fixture, &fixture.cell_a).clone();
        let staged_artifact = artifact(0xa1);
        let staged_digest = content(0xa2);
        let payload_artifact = artifact(0xa3);
        let payload_digest = content(0xa4);
        let mut registry = ErasedRunnerRegistry::new();
        registry
            .register(binding(
                fixture.descriptor_a.clone(),
                "pure",
                MismatchedStagedArtifactRunner {
                    staged_artifact: staged_artifact.clone(),
                    staged_digest: staged_digest.clone(),
                    payload_artifact,
                    payload_digest,
                },
            ))
            .expect("binding a");
        registry
            .register(binding(
                fixture.descriptor_b.clone(),
                "read",
                RecordingRunner {
                    expected_caps: vec![(fixture.cap_kind.clone(), fixture.cap_version.clone())],
                    output_artifact: artifact(0xb1),
                    output_digest: content(0xb2),
                },
            ))
            .expect("binding b");
        let scheduler = test_scheduler(registry);
        let mut store = store::InMemoryTypedRunStore::new();
        scheduler
            .start_run(
                &mut store,
                &fixture.runtime_spec,
                fixture.run_id.clone(),
                run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]),
            )
            .expect("start run");
        let attempt_id = append_attempt_start(&mut store, &fixture, &node, 1);

        assert!(matches!(
            scheduler
                .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
                .await,
            Err(RuntimeError::InvalidRunnerOutput(message))
                if message.contains("without typed payload reference")
        ));

        let descriptor = fixture
            .runtime_spec
            .state_descriptor_for_node(&node)
            .expect("descriptor");
        let output_cell = fixture
            .runtime_spec
            .cell(&node.output_cell)
            .expect("output cell");
        let attempt_logical_key =
            store::LogicalEventKey::new(format!("attempt:{}:{}", node.node_id, attempt_id))
                .expect("attempt key");
        let leaked_artifact_request = store::TypedCommitRequest {
            run_id: fixture.run_id.clone(),
            expected_next_seq: store.expected_next_seq(&fixture.run_id),
            commit_key: store::CommitKey::new("missing-leaked-staged-artifact")
                .expect("commit key"),
            payloads: vec![
                events::KernelEventPayload::CellProduced(events::CellProduced {
                    spec_hash: fixture.runtime_spec.spec_hash().clone(),
                    node_id: node.node_id.clone(),
                    cell_id: node.output_cell.clone(),
                    scope_id: node.scope_id.clone(),
                    attempt_id: attempt_id.clone(),
                    semantic_type_id: descriptor.output_semantic_type_id.clone(),
                    schema_id: descriptor.output_schema_id.clone(),
                    value_lineage: output_cell.value_lineage.clone(),
                    artifact_id: staged_artifact.clone(),
                    content_digest: staged_digest,
                    producer_state_kind: Some(node.state_kind.clone()),
                    producer_state_version: Some(node.state_version.clone()),
                }),
                events::KernelEventPayload::StateAttemptCompleted(events::StateAttemptCompleted {
                    spec_hash: fixture.runtime_spec.spec_hash().clone(),
                    node_id: node.node_id.clone(),
                    attempt_id: attempt_id.clone(),
                    output_cell_id: node.output_cell.clone(),
                }),
            ],
            required_artifacts: Vec::new(),
            preconditions: store::CommitPreconditions {
                required_run_state: store::RequiredRunState::NotCompleted,
                required_present_logical_keys: vec![attempt_logical_key],
                required_cell_states: vec![store::CellStatePrecondition {
                    cell_id: node.output_cell.clone(),
                    required: store::RequiredCellState::Absent,
                }],
                ..store::CommitPreconditions::default()
            },
        };
        let commit = store::PreparedTypedCommit::new(leaked_artifact_request, Vec::new())
            .expect("prepare leak probe");
        assert!(matches!(
            store.append_prepared_typed_commit(commit),
            Err(store::StoreError::MissingArtifact { artifact_id }) if artifact_id == staged_artifact
        ));
    }

    #[test]
    fn materialization_rejects_seed_digest_not_certified() {
        let fixture = fixture();
        let mut seed = fixture.seed_ref.clone();
        seed.digest = content(0xee);
        let scheduler = test_scheduler(registered_fixture_runners(&fixture));
        let mut store = store::InMemoryTypedRunStore::new();
        assert!(matches!(
            scheduler.start_run(
                &mut store,
                &fixture.runtime_spec,
                fixture.run_id.clone(),
                run_start_evidence(&fixture, vec![seed]),
            ),
            Err(RuntimeError::InvalidRunStream(_))
        ));
    }

    #[test]
    fn run_start_rejects_missing_config_artifact_evidence() {
        let fixture = fixture();
        let scheduler = test_scheduler(registered_fixture_runners(&fixture));
        let mut store = store::InMemoryTypedRunStore::new();
        let mut evidence = run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]);
        evidence.config_artifacts.clear();
        assert!(matches!(
            scheduler.start_run(
                &mut store,
                &fixture.runtime_spec,
                fixture.run_id.clone(),
                evidence,
            ),
            Err(RuntimeError::InvalidRunStream(_))
        ));
    }

    #[tokio::test]
    async fn runner_invocation_requires_committed_config_reference() {
        let fixture = fixture();
        let scheduler = test_scheduler(registered_fixture_runners(&fixture));
        let mut store = store::InMemoryTypedRunStore::new();
        scheduler
            .start_run(
                &mut store,
                &fixture.runtime_spec,
                fixture.run_id.clone(),
                run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]),
            )
            .expect("start run");

        let valid_stream = store.load_run_stream(&fixture.run_id);
        let config_ref_pos = valid_stream
            .iter()
            .position(|event| {
                matches!(
                    event.payload(),
                    events::KernelEventPayload::ArtifactReferenced(payload)
                        if payload.artifact_ref.role == events::ArtifactRole::TypedConfig
                )
            })
            .expect("config artifact reference");
        let mut corrupt_payload = match valid_stream[config_ref_pos].payload().clone() {
            events::KernelEventPayload::ArtifactReferenced(payload) => payload,
            _ => unreachable!("position checked"),
        };
        corrupt_payload.artifact_ref.role = events::ArtifactRole::StateOutput;
        let corrupt_stream = rewrite_commit_payload(
            &valid_stream,
            config_ref_pos,
            events::KernelEventPayload::ArtifactReferenced(corrupt_payload),
        );
        let mut corrupt_store = StaleStreamStore {
            inner: &mut store,
            stream: corrupt_stream,
        };

        assert!(matches!(
            scheduler
                .drive_once(&mut corrupt_store, &fixture.runtime_spec, &fixture.run_id)
                .await,
            Err(RuntimeError::InvalidRunStream(message))
                if message.contains("run-start retention refs")
        ));
    }

    #[tokio::test]
    async fn runner_invocation_rejects_config_reference_outside_run_start_commit() {
        let fixture = fixture();
        let scheduler = test_scheduler(registered_fixture_runners(&fixture));
        let mut store = store::InMemoryTypedRunStore::new();
        scheduler
            .start_run(
                &mut store,
                &fixture.runtime_spec,
                fixture.run_id.clone(),
                run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]),
            )
            .expect("start run");

        let valid_stream = store.load_run_stream(&fixture.run_id);
        let config_ref = valid_stream
            .iter()
            .find_map(|event| match event.payload() {
                events::KernelEventPayload::ArtifactReferenced(payload)
                    if payload.artifact_ref.role == events::ArtifactRole::TypedConfig =>
                {
                    Some(event.payload().clone())
                }
                _ => None,
            })
            .expect("config artifact reference");
        let mut corrupt_stream = rewrite_stream_without_payloads(&valid_stream, |payload| {
            matches!(
                payload,
                events::KernelEventPayload::ArtifactReferenced(payload)
                    if payload.artifact_ref.role == events::ArtifactRole::TypedConfig
            )
        });
        append_payload_commit_for_tests(
            &mut corrupt_stream,
            &fixture.run_id,
            "late-config-reference",
            config_ref,
        );
        let mut corrupt_store = StaleStreamStore {
            inner: &mut store,
            stream: corrupt_stream,
        };

        assert!(matches!(
            scheduler
                .drive_once(&mut corrupt_store, &fixture.runtime_spec, &fixture.run_id)
                .await,
            Err(RuntimeError::InvalidRunStream(message))
                if message.contains("run-start retention refs")
        ));
    }

    #[tokio::test]
    async fn runner_invocation_requires_committed_produced_input_artifact_reference() {
        let fixture = fixture();
        let scheduler = test_scheduler(registered_fixture_runners(&fixture));
        let mut store = store::InMemoryTypedRunStore::new();
        scheduler
            .start_run(
                &mut store,
                &fixture.runtime_spec,
                fixture.run_id.clone(),
                run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]),
            )
            .expect("start run");
        scheduler
            .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
            .await
            .expect("produce first cell");

        let producer_node_id = node_by_output(&fixture, &fixture.cell_a).node_id.clone();
        let consumer_node_id = node_by_output(&fixture, &fixture.cell_b).node_id.clone();
        let valid_stream = store.load_run_stream(&fixture.run_id);
        let corrupt_stream = rewrite_stream_without_payloads(&valid_stream, |payload| {
            matches!(
                payload,
                events::KernelEventPayload::ArtifactReferenced(payload)
                    if payload.artifact_ref.role == events::ArtifactRole::StateOutput
                        && payload.node_id.as_ref() == Some(&producer_node_id)
            )
        });
        let mut corrupt_store = StaleStreamStore {
            inner: &mut store,
            stream: corrupt_stream,
        };

        assert!(matches!(
            scheduler
                .drive_once(&mut corrupt_store, &fixture.runtime_spec, &fixture.run_id)
                .await,
            Err(RuntimeError::InputMaterialization(message))
                if message.contains("is not committed in the run stream")
        ));
        assert_eq!(
            attempt_started_count(&store, &fixture.run_id, &consumer_node_id),
            0
        );
    }

    #[tokio::test]
    async fn runner_invocation_rejects_late_produced_input_artifact_reference() {
        let fixture = fixture();
        let scheduler = test_scheduler(registered_fixture_runners(&fixture));
        let mut store = store::InMemoryTypedRunStore::new();
        scheduler
            .start_run(
                &mut store,
                &fixture.runtime_spec,
                fixture.run_id.clone(),
                run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]),
            )
            .expect("start run");
        scheduler
            .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
            .await
            .expect("produce first cell");

        let producer_node_id = node_by_output(&fixture, &fixture.cell_a).node_id.clone();
        let consumer_node_id = node_by_output(&fixture, &fixture.cell_b).node_id.clone();
        let valid_stream = store.load_run_stream(&fixture.run_id);
        let output_ref = valid_stream
            .iter()
            .find_map(|event| match event.payload() {
                events::KernelEventPayload::ArtifactReferenced(payload)
                    if payload.artifact_ref.role == events::ArtifactRole::StateOutput
                        && payload.node_id.as_ref() == Some(&producer_node_id) =>
                {
                    Some(event.payload().clone())
                }
                _ => None,
            })
            .expect("state output artifact reference");
        let mut corrupt_stream = rewrite_stream_without_payloads(&valid_stream, |payload| {
            matches!(
                payload,
                events::KernelEventPayload::ArtifactReferenced(payload)
                    if payload.artifact_ref.role == events::ArtifactRole::StateOutput
                        && payload.node_id.as_ref() == Some(&producer_node_id)
            )
        });
        append_payload_commit_for_tests(
            &mut corrupt_stream,
            &fixture.run_id,
            "late-state-output-reference",
            output_ref,
        );
        let mut corrupt_store = StaleStreamStore {
            inner: &mut store,
            stream: corrupt_stream,
        };

        assert!(matches!(
            scheduler
                .drive_once(&mut corrupt_store, &fixture.runtime_spec, &fixture.run_id)
                .await,
            Err(RuntimeError::InvalidRunStream(message))
                if message.contains("same-commit typed payload")
        ));
        assert_eq!(
            attempt_started_count(&store, &fixture.run_id, &consumer_node_id),
            0
        );
    }

    #[tokio::test]
    async fn runner_invocation_rejects_unsupported_artifact_reference_role() {
        let fixture = fixture();
        let scheduler = test_scheduler(registered_fixture_runners(&fixture));
        let mut store = store::InMemoryTypedRunStore::new();
        scheduler
            .start_run(
                &mut store,
                &fixture.runtime_spec,
                fixture.run_id.clone(),
                run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]),
            )
            .expect("start run");

        let node = node_by_output(&fixture, &fixture.cell_a);
        let artifact = store::ArtifactEvidenceRef {
            artifact_id: artifact(0xe1),
            digest: content(0xe2),
            byte_len: 17,
            media_type: spec::MediaType::new("application/json").expect("media"),
            schema_id: Some(node.config_ref.schema_id.clone()),
            semantic_type_id: None,
            producer_node_id: Some(node.node_id.clone()),
            producer_seed_id: None,
            artifact_role: events::ArtifactRole::SideEffectIntent,
        };
        let mut corrupt_stream = store.load_run_stream(&fixture.run_id);
        append_payload_commit_for_tests(
            &mut corrupt_stream,
            &fixture.run_id,
            "unsupported-artifact-reference",
            events::KernelEventPayload::ArtifactReferenced(events::ArtifactReferenced {
                spec_hash: fixture.runtime_spec.spec_hash().clone(),
                node_id: Some(node.node_id.clone()),
                attempt_id: None,
                artifact_ref: event_artifact_ref_from_store(&artifact),
            }),
        );
        let mut corrupt_store = StaleStreamStore {
            inner: &mut store,
            stream: corrupt_stream,
        };

        assert!(matches!(
            scheduler
                .drive_once(&mut corrupt_store, &fixture.runtime_spec, &fixture.run_id)
                .await,
            Err(RuntimeError::InvalidRunStream(message))
                if message.contains("unsupported artifact reference role")
        ));
    }

    #[tokio::test]
    async fn replay_rejects_terminal_cell_producer_outside_certified_spec() {
        let fixture = fixture();
        let scheduler = test_scheduler(registered_fixture_runners(&fixture));
        let mut store = store::InMemoryTypedRunStore::new();
        scheduler
            .start_run(
                &mut store,
                &fixture.runtime_spec,
                fixture.run_id.clone(),
                run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]),
            )
            .expect("start run");

        let forged_node = fixture
            .runtime_spec
            .topological_order()
            .iter()
            .filter_map(|node_id| fixture.runtime_spec.node(node_id))
            .find(|node| node.output_cell != fixture.cell_a)
            .expect("second node")
            .clone();
        let certified_cell = fixture
            .runtime_spec
            .cell(&fixture.cell_a)
            .expect("cell a")
            .clone();
        let forged_attempt = AttemptId::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([0xfa; 32]),
        );
        let artifact_id = artifact(0xfa);
        let artifact_digest = content(0xfb);
        let forged_artifact = store::ArtifactEvidenceRef {
            artifact_id: artifact_id.clone(),
            digest: artifact_digest.clone(),
            byte_len: 10,
            media_type: spec::MediaType::new("application/json").expect("media"),
            schema_id: Some(certified_cell.schema_id.clone()),
            semantic_type_id: Some(certified_cell.semantic_type_id.clone()),
            producer_node_id: Some(forged_node.node_id.clone()),
            producer_seed_id: None,
            artifact_role: events::ArtifactRole::StateOutput,
        };
        store
            .append_prepared_commit(store::TypedCommitRequest {
                run_id: fixture.run_id.clone(),
                expected_next_seq: store.expected_next_seq(&fixture.run_id),
                commit_key: store::CommitKey::new("forged-attempt-start").expect("commit key"),
                payloads: vec![events::KernelEventPayload::StateAttemptStarted(
                    events::StateAttemptStarted {
                        spec_hash: fixture.runtime_spec.spec_hash().clone(),
                        node_id: forged_node.node_id.clone(),
                        attempt_id: forged_attempt.clone(),
                        attempt_no: 1,
                        state_kind: forged_node.state_kind.clone(),
                        state_version: forged_node.state_version.clone(),
                    },
                )],
                required_artifacts: Vec::new(),
                preconditions: store::CommitPreconditions {
                    required_run_state: store::RequiredRunState::NotCompleted,
                    ..store::CommitPreconditions::default()
                },
            })
            .expect("append forged attempt start");
        store
            .append_prepared_commit(store::TypedCommitRequest {
                run_id: fixture.run_id.clone(),
                expected_next_seq: store.expected_next_seq(&fixture.run_id),
                commit_key: store::CommitKey::new("forged-terminal").expect("commit key"),
                payloads: vec![
                    events::KernelEventPayload::CellProduced(events::CellProduced {
                        spec_hash: fixture.runtime_spec.spec_hash().clone(),
                        node_id: forged_node.node_id.clone(),
                        cell_id: fixture.cell_a.clone(),
                        scope_id: certified_cell.scope_id.clone(),
                        attempt_id: forged_attempt.clone(),
                        semantic_type_id: certified_cell.semantic_type_id.clone(),
                        schema_id: certified_cell.schema_id.clone(),
                        value_lineage: certified_cell.value_lineage.clone(),
                        artifact_id,
                        content_digest: artifact_digest,
                        producer_state_kind: Some(forged_node.state_kind.clone()),
                        producer_state_version: Some(forged_node.state_version.clone()),
                    }),
                    events::KernelEventPayload::StateAttemptCompleted(
                        events::StateAttemptCompleted {
                            spec_hash: fixture.runtime_spec.spec_hash().clone(),
                            node_id: forged_node.node_id.clone(),
                            attempt_id: forged_attempt,
                            output_cell_id: fixture.cell_a.clone(),
                        },
                    ),
                ],
                required_artifacts: vec![forged_artifact],
                preconditions: store::CommitPreconditions {
                    required_run_state: store::RequiredRunState::NotCompleted,
                    ..store::CommitPreconditions::default()
                },
            })
            .expect("append forged terminal");

        assert!(matches!(
            scheduler
                .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
                .await,
            Err(RuntimeError::InvalidRunStream(_))
        ));
    }

    #[test]
    fn store_rejects_fact_without_started_attempt() {
        let fixture = fixture();
        let scheduler = test_scheduler(registered_fixture_runners(&fixture));
        let mut store = store::InMemoryTypedRunStore::new();
        scheduler
            .start_run(
                &mut store,
                &fixture.runtime_spec,
                fixture.run_id.clone(),
                run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]),
            )
            .expect("start run");
        let node = fixture
            .runtime_spec
            .topological_order()
            .iter()
            .filter_map(|node_id| fixture.runtime_spec.node(node_id))
            .find(|node| node.output_cell == fixture.cell_b)
            .expect("read node")
            .clone();
        let fact_artifact = artifact(0xd1);
        let fact_digest = content(0xd2);
        let fact_schema = node.config_ref.schema_id.clone();
        let fact_evidence = store::ArtifactEvidenceRef {
            artifact_id: fact_artifact.clone(),
            digest: fact_digest.clone(),
            byte_len: 10,
            media_type: spec::MediaType::new("application/json").expect("media"),
            schema_id: Some(fact_schema.clone()),
            semantic_type_id: None,
            producer_node_id: Some(node.node_id.clone()),
            producer_seed_id: None,
            artifact_role: events::ArtifactRole::FactResponse,
        };
        assert!(store
            .append_prepared_commit(store::TypedCommitRequest {
                run_id: fixture.run_id.clone(),
                expected_next_seq: store.expected_next_seq(&fixture.run_id),
                commit_key: store::CommitKey::new("forged-fact").expect("commit key"),
                payloads: vec![events::KernelEventPayload::FactRecorded(
                    events::FactRecorded {
                        spec_hash: fixture.runtime_spec.spec_hash().clone(),
                        node_id: node.node_id.clone(),
                        attempt_id: AttemptId::from_digest(
                            DigestAlgorithm::Sha256JcsV1,
                            DigestBytes::from_array([0xd3; 32]),
                        ),
                        capability_kind: fixture.cap_kind.clone(),
                        capability_version: fixture.cap_version.clone(),
                        adapter_kind: fixture.adapter_kind.clone(),
                        adapter_version: fixture.adapter_version.clone(),
                        request_schema_id: fact_schema.clone(),
                        request_hash: content(0xd4),
                        response_schema_id: fact_schema,
                        response_hash: fact_digest,
                        fact_key: events::FactKey::new("forged-fact").expect("fact key"),
                        artifact_id: fact_artifact,
                    },
                )],
                required_artifacts: vec![fact_evidence],
                preconditions: store::CommitPreconditions {
                    required_run_state: store::RequiredRunState::NotCompleted,
                    ..store::CommitPreconditions::default()
                },
            })
            .is_err());
    }

    #[tokio::test]
    async fn replay_rejects_public_output_without_render_attempt() {
        let fixture = fixture();
        let scheduler = test_scheduler(registered_fixture_runners(&fixture));
        let mut store = store::InMemoryTypedRunStore::new();
        scheduler
            .start_run(
                &mut store,
                &fixture.runtime_spec,
                fixture.run_id.clone(),
                run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]),
            )
            .expect("start run");
        let non_render_node = fixture
            .runtime_spec
            .topological_order()
            .iter()
            .filter_map(|node_id| fixture.runtime_spec.node(node_id))
            .find(|node| node.output_cell == fixture.cell_a)
            .expect("non-render node")
            .clone();
        let public_cell = fixture
            .runtime_spec
            .spec()
            .public_outputs
            .outputs
            .first()
            .expect("public cell")
            .clone();
        let source_artifact = artifact(0xe1);
        let source_digest = content(0xe2);
        let producer_node_id = match &public_cell.producer {
            spec::CellProducer::Node(node_id) => Some(node_id.clone()),
            spec::CellProducer::Seed(_) => None,
        };
        let source_evidence = store::ArtifactEvidenceRef {
            artifact_id: source_artifact.clone(),
            digest: source_digest.clone(),
            byte_len: 10,
            media_type: spec::MediaType::new("application/json").expect("media"),
            schema_id: Some(public_cell.schema_id.clone()),
            semantic_type_id: Some(public_cell.semantic_type_id.clone()),
            producer_node_id,
            producer_seed_id: None,
            artifact_role: events::ArtifactRole::StateOutput,
        };
        let forged_attempt = append_attempt_start(&mut store, &fixture, &non_render_node, 1);
        let output_cell = fixture
            .runtime_spec
            .cell(&non_render_node.output_cell)
            .expect("non-render output")
            .clone();
        let receipt_artifact = artifact(0xe3);
        let receipt_digest = content(0xe4);
        let receipt_evidence = store::ArtifactEvidenceRef {
            artifact_id: receipt_artifact.clone(),
            digest: receipt_digest.clone(),
            byte_len: 10,
            media_type: spec::MediaType::new("application/json").expect("media"),
            schema_id: Some(output_cell.schema_id.clone()),
            semantic_type_id: Some(output_cell.semantic_type_id.clone()),
            producer_node_id: Some(non_render_node.node_id.clone()),
            producer_seed_id: None,
            artifact_role: events::ArtifactRole::StateOutput,
        };
        store
            .append_prepared_commit(store::TypedCommitRequest {
                run_id: fixture.run_id.clone(),
                expected_next_seq: store.expected_next_seq(&fixture.run_id),
                commit_key: store::CommitKey::new("forged-public-output").expect("commit key"),
                payloads: vec![
                    events::KernelEventPayload::CellProduced(events::CellProduced {
                        spec_hash: fixture.runtime_spec.spec_hash().clone(),
                        node_id: non_render_node.node_id.clone(),
                        cell_id: non_render_node.output_cell.clone(),
                        scope_id: output_cell.scope_id.clone(),
                        attempt_id: forged_attempt.clone(),
                        semantic_type_id: output_cell.semantic_type_id.clone(),
                        schema_id: output_cell.schema_id.clone(),
                        value_lineage: output_cell.value_lineage.clone(),
                        artifact_id: receipt_artifact,
                        content_digest: receipt_digest,
                        producer_state_kind: Some(non_render_node.state_kind.clone()),
                        producer_state_version: Some(non_render_node.state_version.clone()),
                    }),
                    events::KernelEventPayload::PublicOutputProduced(
                        events::PublicOutputProduced {
                            spec_hash: fixture.runtime_spec.spec_hash().clone(),
                            node_id: non_render_node.node_id.clone(),
                            attempt_id: forged_attempt.clone(),
                            receipt_cell_id: non_render_node.output_cell.clone(),
                            public_schema_id: fixture
                                .runtime_spec
                                .spec()
                                .public_outputs
                                .public_schema_id
                                .clone(),
                            output_spec_digest: fixture
                                .runtime_spec
                                .spec()
                                .public_outputs
                                .digest()
                                .expect("public digest"),
                            cells: vec![events::NamedTypedCellRef {
                                public_field_path: public_cell.public_field_path.clone(),
                                cell_id: public_cell.cell_id.clone(),
                                producer: public_cell.producer.clone(),
                                scope_id: public_cell.scope_id.clone(),
                                semantic_type_id: public_cell.semantic_type_id.clone(),
                                schema_id: public_cell.schema_id.clone(),
                                value_lineage: public_cell.value_lineage.clone(),
                                content_digest: source_digest,
                                artifact_id: source_artifact,
                            }],
                            rendered_digest: content(0xe5),
                            rendered_artifact_id: None,
                            renderer_descriptor_id: fixture
                                .runtime_spec
                                .spec()
                                .public_outputs
                                .renderer_descriptor
                                .descriptor_id
                                .clone(),
                        },
                    ),
                    events::KernelEventPayload::StateAttemptCompleted(
                        events::StateAttemptCompleted {
                            spec_hash: fixture.runtime_spec.spec_hash().clone(),
                            node_id: non_render_node.node_id.clone(),
                            attempt_id: forged_attempt.clone(),
                            output_cell_id: non_render_node.output_cell.clone(),
                        },
                    ),
                ],
                required_artifacts: vec![source_evidence, receipt_evidence],
                preconditions: store::CommitPreconditions {
                    required_run_state: store::RequiredRunState::NotCompleted,
                    required_present_logical_keys: vec![store::LogicalEventKey::new(format!(
                        "attempt:{}:{}",
                        non_render_node.node_id, forged_attempt
                    ))
                    .expect("attempt key")],
                    required_cell_states: vec![store::CellStatePrecondition {
                        cell_id: non_render_node.output_cell.clone(),
                        required: store::RequiredCellState::Absent,
                    }],
                    required_public_output_absent: true,
                    ..store::CommitPreconditions::default()
                },
            })
            .expect("append forged public output");
        assert!(matches!(
            scheduler
                .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
                .await,
            Err(RuntimeError::InvalidRunStream(_))
        ));
    }

    #[tokio::test]
    async fn replay_rejects_public_output_with_forged_rendered_digest() {
        let fixture = fixture();
        let scheduler = test_scheduler(registered_fixture_runners(&fixture));
        let mut store = store::InMemoryTypedRunStore::new();
        scheduler
            .start_run(
                &mut store,
                &fixture.runtime_spec,
                fixture.run_id.clone(),
                run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]),
            )
            .expect("start run");
        scheduler
            .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
            .await
            .expect("drive a");
        scheduler
            .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
            .await
            .expect("drive b");

        let render_node = node_by_output(&fixture, &fixture.render_cell).clone();
        let attempt_id = append_attempt_start(&mut store, &fixture, &render_node, 1);
        let output_cell = fixture
            .runtime_spec
            .cell(&render_node.output_cell)
            .expect("render output")
            .clone();
        let bad_rendered_digest = content(0xf1);
        let bad_receipt_digest = content(0xf2);
        let bad_receipt_artifact =
            ArtifactId::from_digest(bad_receipt_digest.algorithm(), *bad_receipt_digest.digest());
        let bad_receipt_evidence = store::ArtifactEvidenceRef {
            artifact_id: bad_receipt_artifact.clone(),
            digest: bad_receipt_digest.clone(),
            byte_len: 17,
            media_type: spec::MediaType::new("application/json").expect("media"),
            schema_id: Some(output_cell.schema_id.clone()),
            semantic_type_id: Some(output_cell.semantic_type_id.clone()),
            producer_node_id: Some(render_node.node_id.clone()),
            producer_seed_id: None,
            artifact_role: events::ArtifactRole::StateOutput,
        };
        let public_cells = fixture
            .runtime_spec
            .spec()
            .public_outputs
            .outputs
            .iter()
            .map(|public_cell| {
                let Some(store::CellTerminalProjection::Produced {
                    artifact_id,
                    content_digest,
                    ..
                }) = store
                    .projection_snapshot()
                    .cell_terminal(&public_cell.cell_id)
                else {
                    panic!("public cell should be produced");
                };
                events::NamedTypedCellRef {
                    public_field_path: public_cell.public_field_path.clone(),
                    cell_id: public_cell.cell_id.clone(),
                    producer: public_cell.producer.clone(),
                    scope_id: public_cell.scope_id.clone(),
                    semantic_type_id: public_cell.semantic_type_id.clone(),
                    schema_id: public_cell.schema_id.clone(),
                    value_lineage: public_cell.value_lineage.clone(),
                    content_digest: content_digest.clone(),
                    artifact_id: artifact_id.clone(),
                }
            })
            .collect::<Vec<_>>();
        let Some(spec::FrameworkNodeSpec::PublicOutputRender(render)) = &render_node.framework
        else {
            panic!("expected render node");
        };
        store
            .append_prepared_commit(store::TypedCommitRequest {
                run_id: fixture.run_id.clone(),
                expected_next_seq: store.expected_next_seq(&fixture.run_id),
                commit_key: store::CommitKey::new("forged-public-output-rendered-digest")
                    .expect("commit key"),
                payloads: vec![
                    events::KernelEventPayload::CellProduced(events::CellProduced {
                        spec_hash: fixture.runtime_spec.spec_hash().clone(),
                        node_id: render_node.node_id.clone(),
                        cell_id: render_node.output_cell.clone(),
                        scope_id: output_cell.scope_id.clone(),
                        attempt_id: attempt_id.clone(),
                        semantic_type_id: output_cell.semantic_type_id.clone(),
                        schema_id: output_cell.schema_id.clone(),
                        value_lineage: output_cell.value_lineage.clone(),
                        artifact_id: bad_receipt_artifact,
                        content_digest: bad_receipt_digest,
                        producer_state_kind: Some(render_node.state_kind.clone()),
                        producer_state_version: Some(render_node.state_version.clone()),
                    }),
                    events::KernelEventPayload::PublicOutputProduced(
                        events::PublicOutputProduced {
                            spec_hash: fixture.runtime_spec.spec_hash().clone(),
                            node_id: render_node.node_id.clone(),
                            attempt_id: attempt_id.clone(),
                            receipt_cell_id: render_node.output_cell.clone(),
                            public_schema_id: render.public_schema_id.clone(),
                            output_spec_digest: render.output_spec_digest.clone(),
                            cells: public_cells,
                            rendered_digest: bad_rendered_digest,
                            rendered_artifact_id: None,
                            renderer_descriptor_id: render
                                .renderer_descriptor
                                .descriptor_id
                                .clone(),
                        },
                    ),
                    events::KernelEventPayload::StateAttemptCompleted(
                        events::StateAttemptCompleted {
                            spec_hash: fixture.runtime_spec.spec_hash().clone(),
                            node_id: render_node.node_id.clone(),
                            attempt_id: attempt_id.clone(),
                            output_cell_id: render_node.output_cell.clone(),
                        },
                    ),
                ],
                required_artifacts: vec![bad_receipt_evidence],
                preconditions: store::CommitPreconditions {
                    required_run_state: store::RequiredRunState::NotCompleted,
                    required_present_logical_keys: vec![store::LogicalEventKey::new(format!(
                        "attempt:{}:{}",
                        render_node.node_id, attempt_id
                    ))
                    .expect("attempt key")],
                    required_cell_states: vec![store::CellStatePrecondition {
                        cell_id: render_node.output_cell.clone(),
                        required: store::RequiredCellState::Absent,
                    }],
                    required_public_output_absent: true,
                    ..store::CommitPreconditions::default()
                },
            })
            .expect("append forged public output");

        assert!(matches!(
            scheduler
                .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
                .await,
            Err(RuntimeError::InvalidRunStream(message))
                if message.contains("rendered digest")
        ));
    }

    #[tokio::test]
    async fn replay_rejects_split_public_output_terminal_commit() {
        let fixture = fixture();
        let scheduler = test_scheduler(registered_fixture_runners(&fixture));
        let mut store = store::InMemoryTypedRunStore::new();
        scheduler
            .start_run(
                &mut store,
                &fixture.runtime_spec,
                fixture.run_id.clone(),
                run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]),
            )
            .expect("start run");
        scheduler
            .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
            .await
            .expect("drive a");
        scheduler
            .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
            .await
            .expect("drive b");
        scheduler
            .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
            .await
            .expect("render public output");

        let stream = store.load_run_stream(&fixture.run_id);
        let corrupt_stream = split_public_output_payload_to_own_commit_for_tests(&stream);
        let projection =
            store::ProjectionSnapshot::rebuild_from_run_stream(&corrupt_stream).expect("rebuild");
        let mut corrupt_store = ReadOnlyCorruptStore {
            stream: corrupt_stream,
            projection,
        };

        assert!(matches!(
            scheduler
                .drive_once(&mut corrupt_store, &fixture.runtime_spec, &fixture.run_id)
                .await,
            Err(RuntimeError::InvalidRunStream(message))
                if message.contains("split from receipt terminal")
        ));
    }

    #[tokio::test]
    async fn recovery_continues_started_pure_attempt_with_same_attempt_id() {
        let fixture = fixture();
        let scheduler = test_scheduler(registered_fixture_runners(&fixture));
        let mut store = store::InMemoryTypedRunStore::new();
        scheduler
            .start_run(
                &mut store,
                &fixture.runtime_spec,
                fixture.run_id.clone(),
                run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]),
            )
            .expect("start run");
        let node = node_by_output(&fixture, &fixture.cell_a);
        let attempt_id = append_attempt_start(&mut store, &fixture, node, 1);

        assert_eq!(
            scheduler
                .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
                .await
                .expect("resume pure"),
            SchedulerStatus::Advanced
        );
        assert_eq!(
            attempt_started_count(&store, &fixture.run_id, &node.node_id),
            1
        );
        match store
            .projection_snapshot()
            .cell_terminal(&fixture.cell_a)
            .expect("terminal cell")
        {
            store::CellTerminalProjection::Produced {
                attempt_id: produced_attempt,
                ..
            } => assert_eq!(produced_attempt, &attempt_id),
            terminal => panic!("unexpected terminal projection: {terminal:?}"),
        }
    }

    #[tokio::test]
    async fn recovery_rejects_split_terminal_cell_and_attempt_completion() {
        let fixture = fixture();
        let scheduler = test_scheduler(registered_fixture_runners(&fixture));
        let mut store = store::InMemoryTypedRunStore::new();
        scheduler
            .start_run(
                &mut store,
                &fixture.runtime_spec,
                fixture.run_id.clone(),
                run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]),
            )
            .expect("start run");
        scheduler
            .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
            .await
            .expect("produce first cell");
        let valid_stream = store.load_run_stream(&fixture.run_id);
        let corrupt_stream = rewrite_stream_without_payloads(&valid_stream, |payload| {
            matches!(
                payload,
                events::KernelEventPayload::StateAttemptCompleted(payload)
                    if payload.output_cell_id == fixture.cell_a
            )
        });
        let mut corrupt_store = StaleStreamStore {
            inner: &mut store,
            stream: corrupt_stream,
        };

        assert!(matches!(
            scheduler
                .drive_once(&mut corrupt_store, &fixture.runtime_spec, &fixture.run_id)
                .await,
            Err(RuntimeError::InvalidRunStream(_))
        ));
    }

    #[tokio::test]
    async fn recovery_rejects_attempt_started_before_inputs_were_terminal() {
        let fixture = fixture();
        let scheduler = test_scheduler(registered_fixture_runners(&fixture));
        let mut store = store::InMemoryTypedRunStore::new();
        scheduler
            .start_run(
                &mut store,
                &fixture.runtime_spec,
                fixture.run_id.clone(),
                run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]),
            )
            .expect("start run");
        let node_a = node_by_output(&fixture, &fixture.cell_a);
        let node_b = node_by_output(&fixture, &fixture.cell_b);
        append_attempt_start(&mut store, &fixture, node_b, 1);
        let attempt_a = append_attempt_start(&mut store, &fixture, node_a, 1);
        append_terminal(
            &mut store,
            &fixture,
            node_a,
            &attempt_a,
            artifact(0xa1),
            content(0xa2),
        );

        assert!(matches!(
            scheduler
                .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
                .await,
            Err(RuntimeError::InvalidRunStream(_))
        ));
    }

    #[tokio::test]
    async fn recovery_reuses_committed_read_facts_for_same_attempt() {
        struct FactReuseRunner {
            fact_key: events::FactKey,
            output_artifact: ArtifactId,
            output_digest: ContentDigest,
        }

        impl ErasedNodeRunner for FactReuseRunner {
            fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
                Box::pin(async move {
                    let fact = ctx
                        .recorded_facts()
                        .get(&self.fact_key)
                        .expect("recorded fact");
                    assert_eq!(fact.fact_key, self.fact_key);
                    assert_eq!(fact.request_schema_id, ctx.node().config_ref.schema_id);
                    assert_eq!(ctx.recorded_facts().iter().count(), 1);
                    let artifact = store::ArtifactEvidenceRef {
                        artifact_id: self.output_artifact.clone(),
                        digest: self.output_digest.clone(),
                        byte_len: 17,
                        media_type: spec::MediaType::new("application/json").expect("media"),
                        schema_id: Some(ctx.descriptor().output_schema_id.clone()),
                        semantic_type_id: Some(ctx.descriptor().output_semantic_type_id.clone()),
                        producer_node_id: Some(ctx.node().node_id.clone()),
                        producer_seed_id: None,
                        artifact_role: events::ArtifactRole::StateOutput,
                    };
                    let staged_artifact = staged_attempt_artifact(&ctx, artifact)?;
                    Ok(ErasedRunnerOutput {
                        staged_artifacts: vec![staged_artifact],
                        staged_retention_refs: Vec::new(),
                        payloads: terminal_payloads(
                            &ctx,
                            self.output_artifact.clone(),
                            self.output_digest.clone(),
                        ),
                    })
                })
            }
        }

        let fixture = fixture();
        let fact_key = events::FactKey::new("reused-fact").expect("fact key");
        let mut registry = ErasedRunnerRegistry::new();
        registry
            .register(binding(
                fixture.descriptor_a.clone(),
                "pure",
                RecordingRunner {
                    expected_caps: Vec::new(),
                    output_artifact: artifact(0xa1),
                    output_digest: content(0xa2),
                },
            ))
            .expect("binding a");
        registry
            .register(binding(
                fixture.descriptor_b.clone(),
                "read",
                FactReuseRunner {
                    fact_key: fact_key.clone(),
                    output_artifact: artifact(0xb1),
                    output_digest: content(0xb2),
                },
            ))
            .expect("binding b");
        let scheduler = test_scheduler(registry);
        let mut store = store::InMemoryTypedRunStore::new();
        scheduler
            .start_run(
                &mut store,
                &fixture.runtime_spec,
                fixture.run_id.clone(),
                run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]),
            )
            .expect("start run");
        scheduler
            .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
            .await
            .expect("produce input");
        let node = node_by_output(&fixture, &fixture.cell_b);
        let attempt_id = append_attempt_start(&mut store, &fixture, node, 1);
        append_fact(
            &mut store,
            &fixture,
            node,
            &attempt_id,
            fact_key.clone(),
            artifact(0xd1),
            content(0xd2),
        );

        scheduler
            .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
            .await
            .expect("resume read");
        assert_eq!(fact_recorded_count(&store), 1);
        match store
            .projection_snapshot()
            .cell_terminal(&fixture.cell_b)
            .expect("terminal cell")
        {
            store::CellTerminalProjection::Produced {
                attempt_id: produced_attempt,
                ..
            } => assert_eq!(produced_attempt, &attempt_id),
            terminal => panic!("unexpected terminal projection: {terminal:?}"),
        }
    }

    #[tokio::test]
    async fn recovery_rejects_new_fact_after_same_attempt_fact_exists() {
        struct NewFactRunner {
            cap_kind: CapabilityKind,
            cap_version: CapabilityVersion,
            adapter_kind: AdapterKind,
            adapter_version: AdapterVersion,
        }

        impl ErasedNodeRunner for NewFactRunner {
            fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
                Box::pin(async move {
                    Ok(ErasedRunnerOutput::new(vec![
                        events::KernelEventPayload::FactRecorded(events::FactRecorded {
                            spec_hash: ctx.spec_hash().clone(),
                            node_id: ctx.node().node_id.clone(),
                            attempt_id: ctx.attempt_id().clone(),
                            capability_kind: self.cap_kind.clone(),
                            capability_version: self.cap_version.clone(),
                            adapter_kind: self.adapter_kind.clone(),
                            adapter_version: self.adapter_version.clone(),
                            request_schema_id: ctx.node().config_ref.schema_id.clone(),
                            request_hash: content(0xe1),
                            response_schema_id: ctx.node().config_ref.schema_id.clone(),
                            response_hash: content(0xe2),
                            fact_key: events::FactKey::new("new-fact").expect("fact key"),
                            artifact_id: artifact(0xe3),
                        }),
                    ]))
                })
            }
        }

        let fixture = fixture();
        let mut registry = ErasedRunnerRegistry::new();
        registry
            .register(binding(
                fixture.descriptor_a.clone(),
                "pure",
                RecordingRunner {
                    expected_caps: Vec::new(),
                    output_artifact: artifact(0xa1),
                    output_digest: content(0xa2),
                },
            ))
            .expect("binding a");
        registry
            .register(binding(
                fixture.descriptor_b.clone(),
                "read",
                NewFactRunner {
                    cap_kind: fixture.cap_kind.clone(),
                    cap_version: fixture.cap_version.clone(),
                    adapter_kind: fixture.adapter_kind.clone(),
                    adapter_version: fixture.adapter_version.clone(),
                },
            ))
            .expect("binding b");
        let scheduler = test_scheduler(registry);
        let mut store = store::InMemoryTypedRunStore::new();
        scheduler
            .start_run(
                &mut store,
                &fixture.runtime_spec,
                fixture.run_id.clone(),
                run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]),
            )
            .expect("start run");
        scheduler
            .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
            .await
            .expect("produce input");
        let node = node_by_output(&fixture, &fixture.cell_b);
        let attempt_id = append_attempt_start(&mut store, &fixture, node, 1);
        append_fact(
            &mut store,
            &fixture,
            node,
            &attempt_id,
            events::FactKey::new("existing-fact").expect("fact key"),
            artifact(0xd1),
            content(0xd2),
        );

        assert!(matches!(
            scheduler
                .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
                .await,
            Err(RuntimeError::InvalidRunnerOutput(_))
        ));
        assert_eq!(store.projection_snapshot().facts().count(), 1);
    }

    #[tokio::test]
    async fn recovery_allows_managed_write_artifact_restage_before_terminal_commit() {
        let fixture = fixture_with_first_managed_write_state();
        let output_artifact = artifact(0xa1);
        let output_digest = content(0xa2);
        let node = node_by_output(&fixture, &fixture.cell_a);
        let expected_caps = node
            .capability_bindings
            .capabilities
            .iter()
            .map(|capability| (capability.kind.clone(), capability.version.clone()))
            .collect();
        let mut registry = ErasedRunnerRegistry::new();
        registry
            .register(binding(
                fixture.descriptor_a.clone(),
                "managed-write",
                RecordingRunner {
                    expected_caps,
                    output_artifact: output_artifact.clone(),
                    output_digest: output_digest.clone(),
                },
            ))
            .expect("binding a");
        registry
            .register(binding(
                fixture.descriptor_b.clone(),
                "read",
                RecordingRunner {
                    expected_caps: vec![(fixture.cap_kind.clone(), fixture.cap_version.clone())],
                    output_artifact: artifact(0xb1),
                    output_digest: content(0xb2),
                },
            ))
            .expect("binding b");
        let scheduler = test_scheduler(registry);
        let mut store = store::InMemoryTypedRunStore::new();
        scheduler
            .start_run(
                &mut store,
                &fixture.runtime_spec,
                fixture.run_id.clone(),
                run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]),
            )
            .expect("start run");
        let attempt_id = append_attempt_start(&mut store, &fixture, node, 1);
        assert!(store
            .projection_snapshot()
            .cell_terminal(&fixture.cell_a)
            .is_none());

        scheduler
            .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
            .await
            .expect("resume managed write");
        assert_eq!(
            attempt_started_count(&store, &fixture.run_id, &node.node_id),
            1
        );
        match store
            .projection_snapshot()
            .cell_terminal(&fixture.cell_a)
            .expect("terminal cell")
        {
            store::CellTerminalProjection::Produced {
                attempt_id: produced_attempt,
                ..
            } => assert_eq!(produced_attempt, &attempt_id),
            terminal => panic!("unexpected terminal projection: {terminal:?}"),
        }
    }

    #[tokio::test]
    async fn side_effect_scheduler_commits_durable_ledger_phases_before_output() {
        let fixture = fixture_with_first_side_effect_state();
        let scheduler = test_scheduler(registered_side_effect_fixture_runners(&fixture));
        let mut store = store::InMemoryTypedRunStore::new();
        scheduler
            .start_run(
                &mut store,
                &fixture.runtime_spec,
                fixture.run_id.clone(),
                run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]),
            )
            .expect("start run");

        for _ in 0..6 {
            assert_eq!(
                scheduler
                    .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
                    .await
                    .expect("drive side effect phase"),
                SchedulerStatus::Advanced
            );
        }

        assert!(store
            .projection_snapshot()
            .cell_terminal(&fixture.cell_a)
            .is_some());
        let node = node_by_output(&fixture, &fixture.cell_a);
        let attempt_id = attempt_id(
            &fixture.run_id,
            fixture.runtime_spec.spec_hash(),
            &node.node_id,
            1,
        )
        .expect("attempt id");
        let projection =
            side_effect_projection_for_attempt(store.projection_snapshot(), node, &attempt_id)
                .expect("projection lookup")
                .expect("side-effect projection");
        assert!(matches!(
            projection.phase,
            store::SideEffectPhase::ConfirmationObserved { .. }
        ));
        assert!(store
            .load_run_stream(&fixture.run_id)
            .iter()
            .any(|event| matches!(
                event.payload(),
                events::KernelEventPayload::SideEffectClaimTakenOver(_)
            )));
        assert_eq!(
            attempt_started_count(&store, &fixture.run_id, &node.node_id),
            1
        );
    }

    #[tokio::test]
    async fn side_effect_not_submitted_resume_claims_next_epoch() {
        let fixture = fixture_with_first_side_effect_state();
        let scheduler = test_scheduler(registered_side_effect_fixture_runners(&fixture));
        let mut store = store::InMemoryTypedRunStore::new();
        scheduler
            .start_run(
                &mut store,
                &fixture.runtime_spec,
                fixture.run_id.clone(),
                run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]),
            )
            .expect("start run");
        scheduler
            .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
            .await
            .expect("prepare side effect");
        scheduler
            .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
            .await
            .expect("take over and start invocation");

        let node = node_by_output(&fixture, &fixture.cell_a);
        let attempt_id = attempt_id(
            &fixture.run_id,
            fixture.runtime_spec.spec_hash(),
            &node.node_id,
            1,
        )
        .expect("attempt id");
        append_not_submitted_proven(&mut store, &fixture, node, &attempt_id, 1);

        scheduler
            .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
            .await
            .expect("resume not-submitted");
        let projection =
            side_effect_projection_for_attempt(store.projection_snapshot(), node, &attempt_id)
                .expect("projection lookup")
                .expect("side-effect projection");
        assert!(matches!(
            projection.phase,
            store::SideEffectPhase::InvocationStarted {
                invocation_epoch: 2,
                claim_generation: 3,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn side_effect_staged_artifact_must_match_payload_ledger_binding() {
        struct WrongLedgerStagedSideEffectRunner {
            cap_kind: CapabilityKind,
            cap_version: CapabilityVersion,
            adapter_kind: AdapterKind,
            adapter_version: AdapterVersion,
        }

        impl ErasedNodeRunner for WrongLedgerStagedSideEffectRunner {
            fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
                Box::pin(async move {
                    let ledger = side_effect_ledger_key(ctx.attempt_no());
                    let staged_ledger =
                        events::SideEffectLedgerKey::new("wrong-ledger").expect("ledger key");
                    let intent_bytes = br#"{"intent":"wrong-ledger"}"#.to_vec();
                    let intent_hash = digest_for_bytes(&intent_bytes);
                    let intent_artifact_id =
                        ArtifactId::from_digest(intent_hash.algorithm(), *intent_hash.digest());
                    let evidence = store::ArtifactEvidenceRef {
                        artifact_id: intent_artifact_id.clone(),
                        digest: intent_hash.clone(),
                        byte_len: intent_bytes.len() as u64,
                        media_type: spec::MediaType::new("application/json").expect("media"),
                        schema_id: Some(ctx.node().config_ref.schema_id.clone()),
                        semantic_type_id: None,
                        producer_node_id: Some(ctx.node().node_id.clone()),
                        producer_seed_id: None,
                        artifact_role: events::ArtifactRole::SideEffectIntent,
                    };
                    let staged_artifact = StagedArtifact::inline_side_effect_artifact(
                        &ctx,
                        intent_bytes,
                        evidence,
                        staged_ledger,
                        1,
                    )?;
                    Ok(ErasedRunnerOutput {
                        staged_artifacts: vec![staged_artifact],
                        staged_retention_refs: Vec::new(),
                        payloads: vec![
                            events::KernelEventPayload::SideEffectIntentPersisted(
                                events::side_effect::IntentPersisted {
                                    spec_hash: ctx.spec_hash().clone(),
                                    node_id: ctx.node().node_id.clone(),
                                    scope_id: ctx.node().scope_id.clone(),
                                    attempt_id: ctx.attempt_id().clone(),
                                    ledger_key: ledger.clone(),
                                    invocation_epoch: 1,
                                    intent_schema_id: ctx.node().config_ref.schema_id.clone(),
                                    intent_hash,
                                    intent_artifact_id,
                                    idempotency_input_schema_id: ctx
                                        .node()
                                        .config_ref
                                        .schema_id
                                        .clone(),
                                    idempotency_input_hash: content(0xc3),
                                    idempotency_key: events::IdempotencyKeyRef::new("idem-1")
                                        .expect("idempotency key"),
                                    capability_kind: self.cap_kind.clone(),
                                    capability_version: self.cap_version.clone(),
                                    adapter_kind: self.adapter_kind.clone(),
                                    adapter_version: self.adapter_version.clone(),
                                },
                            ),
                            side_effect_claimed(&ctx, ledger.clone(), 1, 1),
                            side_effect_prepared(&ctx, ledger, 1, 1),
                        ],
                    })
                })
            }
        }

        let fixture = fixture_with_first_side_effect_state();
        let mut registry = ErasedRunnerRegistry::new();
        registry
            .register(binding(
                fixture.descriptor_a.clone(),
                "sidefx",
                WrongLedgerStagedSideEffectRunner {
                    cap_kind: side_effect_capability_kind(),
                    cap_version: side_effect_capability_version(),
                    adapter_kind: fixture.adapter_kind.clone(),
                    adapter_version: fixture.adapter_version.clone(),
                },
            ))
            .expect("binding a");
        registry
            .register(binding(
                fixture.descriptor_b.clone(),
                "read",
                RecordingRunner {
                    expected_caps: vec![(fixture.cap_kind.clone(), fixture.cap_version.clone())],
                    output_artifact: artifact(0xb1),
                    output_digest: content(0xb2),
                },
            ))
            .expect("binding b");
        let scheduler = test_scheduler(registry);
        let mut store = store::InMemoryTypedRunStore::new();
        scheduler
            .start_run(
                &mut store,
                &fixture.runtime_spec,
                fixture.run_id.clone(),
                run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]),
            )
            .expect("start run");

        assert!(matches!(
            scheduler
                .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
                .await,
            Err(RuntimeError::InvalidRunnerOutput(message))
                if message.contains("does not match typed payload binding")
        ));
    }

    #[tokio::test]
    async fn side_effect_ambiguous_phase_blocks_resume() {
        let fixture = fixture_with_first_side_effect_state();
        let mut registry = ErasedRunnerRegistry::new();
        registry
            .register(binding(
                fixture.descriptor_a.clone(),
                "sidefx",
                AmbiguousSideEffectRunner::new(&fixture),
            ))
            .expect("binding a");
        registry
            .register(binding(
                fixture.descriptor_b.clone(),
                "read",
                RecordingRunner {
                    expected_caps: vec![(fixture.cap_kind.clone(), fixture.cap_version.clone())],
                    output_artifact: artifact(0xb1),
                    output_digest: content(0xb2),
                },
            ))
            .expect("binding b");
        let scheduler = test_scheduler(registry);
        let mut store = store::InMemoryTypedRunStore::new();
        scheduler
            .start_run(
                &mut store,
                &fixture.runtime_spec,
                fixture.run_id.clone(),
                run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]),
            )
            .expect("start run");

        for _ in 0..3 {
            assert_eq!(
                scheduler
                    .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
                    .await
                    .expect("advance to ambiguity"),
                SchedulerStatus::Advanced
            );
        }
        assert_eq!(
            scheduler
                .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
                .await
                .expect("ambiguous side effect blocks"),
            SchedulerStatus::Blocked
        );
        assert!(store
            .projection_snapshot()
            .cell_terminal(&fixture.cell_a)
            .is_none());
    }

    #[tokio::test]
    async fn side_effect_ambiguity_blocks_independent_ready_nodes() {
        let fixture = fixture_with_independent_second_node_and_first_side_effect_state();
        let mut registry = ErasedRunnerRegistry::new();
        registry
            .register(binding(
                fixture.descriptor_a.clone(),
                "sidefx",
                AmbiguousSideEffectRunner::new(&fixture),
            ))
            .expect("binding a");
        registry
            .register(binding(
                fixture.descriptor_b.clone(),
                "read",
                RecordingRunner {
                    expected_caps: vec![(fixture.cap_kind.clone(), fixture.cap_version.clone())],
                    output_artifact: artifact(0xb1),
                    output_digest: content(0xb2),
                },
            ))
            .expect("binding b");
        let scheduler = test_scheduler(registry);
        let mut store = store::InMemoryTypedRunStore::new();
        scheduler
            .start_run(
                &mut store,
                &fixture.runtime_spec,
                fixture.run_id.clone(),
                run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]),
            )
            .expect("start run");

        for _ in 0..3 {
            scheduler
                .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
                .await
                .expect("advance to ambiguity");
        }
        assert_eq!(
            scheduler
                .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
                .await
                .expect("ambiguity blocks independent node"),
            SchedulerStatus::Blocked
        );
        assert!(store
            .projection_snapshot()
            .cell_terminal(&fixture.cell_b)
            .is_none());
    }

    #[tokio::test]
    async fn side_effect_output_before_confirmation_is_rejected() {
        let fixture = fixture_with_first_side_effect_state();
        let mut registry = ErasedRunnerRegistry::new();
        registry
            .register(binding(
                fixture.descriptor_a.clone(),
                "sidefx",
                PrematureSideEffectOutputRunner {
                    output_artifact: artifact(0xa1),
                    output_digest: content(0xa2),
                },
            ))
            .expect("binding a");
        registry
            .register(binding(
                fixture.descriptor_b.clone(),
                "read",
                RecordingRunner {
                    expected_caps: vec![(fixture.cap_kind.clone(), fixture.cap_version.clone())],
                    output_artifact: artifact(0xb1),
                    output_digest: content(0xb2),
                },
            ))
            .expect("binding b");
        let scheduler = test_scheduler(registry);
        let mut store = store::InMemoryTypedRunStore::new();
        scheduler
            .start_run(
                &mut store,
                &fixture.runtime_spec,
                fixture.run_id.clone(),
                run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]),
            )
            .expect("start run");

        assert!(matches!(
            scheduler
                .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
                .await,
            Err(RuntimeError::InvalidRunnerOutput(_))
        ));
    }

    #[tokio::test]
    async fn side_effect_failure_retryability_must_match_attempt_failure() {
        let fixture = fixture_with_first_side_effect_state();
        let mut registry = ErasedRunnerRegistry::new();
        registry
            .register(binding(
                fixture.descriptor_a.clone(),
                "sidefx",
                MismatchedSideEffectFailureRunner,
            ))
            .expect("binding a");
        registry
            .register(binding(
                fixture.descriptor_b.clone(),
                "read",
                RecordingRunner {
                    expected_caps: vec![(fixture.cap_kind.clone(), fixture.cap_version.clone())],
                    output_artifact: artifact(0xb1),
                    output_digest: content(0xb2),
                },
            ))
            .expect("binding b");
        let scheduler = test_scheduler(registry);
        let mut store = store::InMemoryTypedRunStore::new();
        scheduler
            .start_run(
                &mut store,
                &fixture.runtime_spec,
                fixture.run_id.clone(),
                run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]),
            )
            .expect("start run");

        assert!(matches!(
            scheduler
                .drive_once(&mut store, &fixture.runtime_spec, &fixture.run_id)
                .await,
            Err(RuntimeError::InvalidRunnerOutput(_))
        ));
    }

    struct DeterministicSideEffectRunner {
        cap_kind: CapabilityKind,
        cap_version: CapabilityVersion,
        adapter_kind: AdapterKind,
        adapter_version: AdapterVersion,
        output_artifact: ArtifactId,
        output_digest: ContentDigest,
    }

    impl DeterministicSideEffectRunner {
        fn new(fixture: &Fixture) -> Self {
            Self {
                cap_kind: side_effect_capability_kind(),
                cap_version: side_effect_capability_version(),
                adapter_kind: fixture.adapter_kind.clone(),
                adapter_version: fixture.adapter_version.clone(),
                output_artifact: artifact(0xa1),
                output_digest: content(0xa2),
            }
        }
    }

    impl ErasedNodeRunner for DeterministicSideEffectRunner {
        fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
            Box::pin(async move {
                let ledger = side_effect_ledger_key(ctx.attempt_no());
                let phase = side_effect_projection_for_attempt(
                    ctx.projections(),
                    ctx.node(),
                    ctx.attempt_id(),
                )?;
                match phase {
                    None => self.prepare(ctx, ledger),
                    Some(projection)
                        if matches!(
                            projection.phase,
                            store::SideEffectPhase::Claimed { .. }
                                | store::SideEffectPhase::InvocationPrepared { .. }
                        ) =>
                    {
                        let claim = projection.claim.as_ref().expect("claim projection");
                        Ok(ErasedRunnerOutput::new(vec![
                            side_effect_claim_taken_over(&ctx, ledger.clone(), claim, 2),
                            side_effect_prepared(&ctx, ledger.clone(), 1, 2),
                            side_effect_invocation_started(&ctx, ledger, 1, 2),
                        ]))
                    }
                    Some(store::SideEffectProjection {
                        phase:
                            store::SideEffectPhase::InvocationStarted {
                                invocation_epoch, ..
                            }
                            | store::SideEffectPhase::SubmissionUnknown { invocation_epoch },
                        ..
                    }) => {
                        let artifact_id = artifact(0xc5);
                        let digest = content(0xc4);
                        let staged_artifact = staged_side_effect_artifact(
                            &ctx,
                            side_effect_artifact(
                                &ctx,
                                artifact_id.clone(),
                                digest.clone(),
                                events::ArtifactRole::Submission,
                            ),
                            ledger.clone(),
                            *invocation_epoch,
                        )?;
                        Ok(ErasedRunnerOutput {
                            staged_artifacts: vec![staged_artifact],
                            staged_retention_refs: Vec::new(),
                            payloads: vec![side_effect_submission_observed(
                                &ctx,
                                ledger,
                                *invocation_epoch,
                                artifact_id,
                                digest,
                            )],
                        })
                    }
                    Some(store::SideEffectProjection {
                        phase: store::SideEffectPhase::NotSubmittedProven { invocation_epoch },
                        claim,
                        ..
                    }) => {
                        let claim = claim.as_ref().expect("claim projection");
                        let next_epoch = invocation_epoch + 1;
                        let next_generation = claim.claim_generation + 1;
                        Ok(ErasedRunnerOutput::new(vec![
                            side_effect_claimed(&ctx, ledger.clone(), next_epoch, next_generation),
                            side_effect_prepared(&ctx, ledger.clone(), next_epoch, next_generation),
                            side_effect_invocation_started(
                                &ctx,
                                ledger,
                                next_epoch,
                                next_generation,
                            ),
                        ]))
                    }
                    Some(store::SideEffectProjection {
                        phase: store::SideEffectPhase::SubmissionObserved { invocation_epoch },
                        ..
                    }) => {
                        let artifact_id = artifact(0xc7);
                        let digest = content(0xc6);
                        let staged_artifact = staged_side_effect_artifact(
                            &ctx,
                            side_effect_artifact(
                                &ctx,
                                artifact_id.clone(),
                                digest.clone(),
                                events::ArtifactRole::Receipt,
                            ),
                            ledger.clone(),
                            *invocation_epoch,
                        )?;
                        Ok(ErasedRunnerOutput {
                            staged_artifacts: vec![staged_artifact],
                            staged_retention_refs: Vec::new(),
                            payloads: vec![side_effect_receipt_observed(
                                &ctx,
                                ledger,
                                *invocation_epoch,
                                artifact_id,
                                digest,
                            )],
                        })
                    }
                    Some(store::SideEffectProjection {
                        phase: store::SideEffectPhase::ReceiptObserved { invocation_epoch },
                        ..
                    }) => {
                        let artifact_id = artifact(0xc9);
                        let digest = content(0xc8);
                        let staged_artifact = staged_side_effect_artifact(
                            &ctx,
                            side_effect_artifact(
                                &ctx,
                                artifact_id.clone(),
                                digest.clone(),
                                events::ArtifactRole::Confirmation,
                            ),
                            ledger.clone(),
                            *invocation_epoch,
                        )?;
                        Ok(ErasedRunnerOutput {
                            staged_artifacts: vec![staged_artifact],
                            staged_retention_refs: Vec::new(),
                            payloads: vec![side_effect_confirmation_observed(
                                &ctx,
                                ledger,
                                *invocation_epoch,
                                artifact_id,
                                digest,
                            )],
                        })
                    }
                    Some(store::SideEffectProjection {
                        phase: store::SideEffectPhase::ConfirmationObserved { .. },
                        ..
                    }) => {
                        let artifact = state_output_artifact(
                            ctx.node(),
                            ctx.descriptor(),
                            self.output_artifact.clone(),
                            self.output_digest.clone(),
                        );
                        let staged_artifact = staged_attempt_artifact(&ctx, artifact)?;
                        Ok(ErasedRunnerOutput {
                            staged_artifacts: vec![staged_artifact],
                            staged_retention_refs: Vec::new(),
                            payloads: terminal_payloads(
                                &ctx,
                                self.output_artifact.clone(),
                                self.output_digest.clone(),
                            ),
                        })
                    }
                    Some(_) => Err(RuntimeError::Blocked(
                        "side-effect fixture blocked".to_owned(),
                    )),
                }
            })
        }
    }

    impl DeterministicSideEffectRunner {
        fn prepare(
            &self,
            ctx: ErasedRunCtx<'_>,
            ledger: events::SideEffectLedgerKey,
        ) -> Result<ErasedRunnerOutput> {
            assert!(ctx.caps().contains(&self.cap_kind, &self.cap_version));
            let intent_artifact_id = artifact(0xc2);
            let intent_hash = content(0xc1);
            let staged_artifact = staged_side_effect_artifact(
                &ctx,
                side_effect_artifact(
                    &ctx,
                    intent_artifact_id.clone(),
                    intent_hash.clone(),
                    events::ArtifactRole::SideEffectIntent,
                ),
                ledger.clone(),
                1,
            )?;
            Ok(ErasedRunnerOutput {
                staged_artifacts: vec![staged_artifact],
                staged_retention_refs: Vec::new(),
                payloads: vec![
                    events::KernelEventPayload::SideEffectIntentPersisted(
                        events::side_effect::IntentPersisted {
                            spec_hash: ctx.spec_hash().clone(),
                            node_id: ctx.node().node_id.clone(),
                            scope_id: ctx.node().scope_id.clone(),
                            attempt_id: ctx.attempt_id().clone(),
                            ledger_key: ledger.clone(),
                            invocation_epoch: 1,
                            intent_schema_id: ctx.node().config_ref.schema_id.clone(),
                            intent_hash,
                            intent_artifact_id,
                            idempotency_input_schema_id: ctx.node().config_ref.schema_id.clone(),
                            idempotency_input_hash: content(0xc3),
                            idempotency_key: events::IdempotencyKeyRef::new("idem-1")
                                .expect("idempotency key"),
                            capability_kind: self.cap_kind.clone(),
                            capability_version: self.cap_version.clone(),
                            adapter_kind: self.adapter_kind.clone(),
                            adapter_version: self.adapter_version.clone(),
                        },
                    ),
                    side_effect_claimed(&ctx, ledger.clone(), 1, 1),
                    side_effect_prepared(&ctx, ledger, 1, 1),
                ],
            })
        }
    }

    struct AmbiguousSideEffectRunner {
        inner: DeterministicSideEffectRunner,
    }

    impl AmbiguousSideEffectRunner {
        fn new(fixture: &Fixture) -> Self {
            Self {
                inner: DeterministicSideEffectRunner::new(fixture),
            }
        }
    }

    impl ErasedNodeRunner for AmbiguousSideEffectRunner {
        fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
            Box::pin(async move {
                let ledger = side_effect_ledger_key(ctx.attempt_no());
                let phase = side_effect_projection_for_attempt(
                    ctx.projections(),
                    ctx.node(),
                    ctx.attempt_id(),
                )?;
                if matches!(
                    phase.map(|projection| &projection.phase),
                    Some(store::SideEffectPhase::InvocationStarted { .. })
                ) {
                    let artifact_id = artifact(0xcb);
                    let digest = content(0xca);
                    let staged_artifact = staged_side_effect_artifact(
                        &ctx,
                        side_effect_artifact(
                            &ctx,
                            artifact_id.clone(),
                            digest.clone(),
                            events::ArtifactRole::AmbiguityEvidence,
                        ),
                        ledger.clone(),
                        1,
                    )?;
                    Ok(ErasedRunnerOutput {
                        staged_artifacts: vec![staged_artifact],
                        staged_retention_refs: Vec::new(),
                        payloads: vec![events::KernelEventPayload::SideEffectAmbiguous(
                            events::side_effect::Ambiguous {
                                spec_hash: ctx.spec_hash().clone(),
                                node_id: ctx.node().node_id.clone(),
                                attempt_id: ctx.attempt_id().clone(),
                                ledger_key: ledger,
                                invocation_epoch: 1,
                                ambiguity_code: events::AmbiguityCode::new("unknown_submission")
                                    .expect("ambiguity code"),
                                evidence_schema_id: ctx.node().config_ref.schema_id.clone(),
                                evidence_hash: digest,
                                evidence_artifact_id: artifact_id,
                            },
                        )],
                    })
                } else {
                    self.inner.run_erased(ctx).await
                }
            })
        }
    }

    struct PrematureSideEffectOutputRunner {
        output_artifact: ArtifactId,
        output_digest: ContentDigest,
    }

    impl ErasedNodeRunner for PrematureSideEffectOutputRunner {
        fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
            Box::pin(async move {
                let artifact = state_output_artifact(
                    ctx.node(),
                    ctx.descriptor(),
                    self.output_artifact.clone(),
                    self.output_digest.clone(),
                );
                let staged_artifact = staged_attempt_artifact(&ctx, artifact)?;
                Ok(ErasedRunnerOutput {
                    staged_artifacts: vec![staged_artifact],
                    staged_retention_refs: Vec::new(),
                    payloads: terminal_payloads(
                        &ctx,
                        self.output_artifact.clone(),
                        self.output_digest.clone(),
                    ),
                })
            })
        }
    }

    struct MismatchedSideEffectFailureRunner;

    impl ErasedNodeRunner for MismatchedSideEffectFailureRunner {
        fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
            Box::pin(async move {
                Ok(ErasedRunnerOutput::new(vec![
                    events::KernelEventPayload::SideEffectFailed(events::side_effect::Failed {
                        spec_hash: ctx.spec_hash().clone(),
                        node_id: ctx.node().node_id.clone(),
                        attempt_id: ctx.attempt_id().clone(),
                        ledger_key: side_effect_ledger_key(ctx.attempt_no()),
                        invocation_epoch: 1,
                        failure_phase: events::side_effect::FailurePhase::BeforeInvocationStarted,
                        retryable: false,
                        error: side_effect_error(false),
                    }),
                    events::KernelEventPayload::StateAttemptFailed(events::StateAttemptFailed {
                        spec_hash: ctx.spec_hash().clone(),
                        node_id: ctx.node().node_id.clone(),
                        attempt_id: ctx.attempt_id().clone(),
                        retryable: true,
                        error: side_effect_error(true),
                    }),
                ]))
            })
        }
    }

    struct StaleStreamStore<'a> {
        inner: &'a mut store::InMemoryTypedRunStore,
        stream: Vec<store::KernelEventEnvelope>,
    }

    impl store::TypedProjectionRead for StaleStreamStore<'_> {
        fn projection_snapshot(&self) -> &store::ProjectionSnapshot {
            self.inner.projection_snapshot()
        }
    }

    impl store::TypedRunEventStore for StaleStreamStore<'_> {
        fn append_prepared_typed_commit(
            &mut self,
            commit: store::PreparedTypedCommit,
        ) -> store::Result<store::CommitOutcome> {
            self.inner.append_prepared_typed_commit(commit)
        }

        fn load_run_stream(&self, _run_id: &RunId) -> Vec<store::KernelEventEnvelope> {
            self.stream.clone()
        }

        fn expected_next_seq(&self, run_id: &RunId) -> store::StreamSeq {
            self.inner.expected_next_seq(run_id)
        }
    }

    struct ReadOnlyCorruptStore {
        stream: Vec<store::KernelEventEnvelope>,
        projection: store::ProjectionSnapshot,
    }

    impl store::TypedProjectionRead for ReadOnlyCorruptStore {
        fn projection_snapshot(&self) -> &store::ProjectionSnapshot {
            &self.projection
        }
    }

    impl store::TypedRunEventStore for ReadOnlyCorruptStore {
        fn append_prepared_typed_commit(
            &mut self,
            _commit: store::PreparedTypedCommit,
        ) -> store::Result<store::CommitOutcome> {
            Err(store::StoreError::Identity(
                "corrupt test store is read-only".to_owned(),
            ))
        }

        fn load_run_stream(&self, _run_id: &RunId) -> Vec<store::KernelEventEnvelope> {
            self.stream.clone()
        }

        fn expected_next_seq(&self, _run_id: &RunId) -> store::StreamSeq {
            store::StreamSeq::FIRST
        }
    }

    fn rewrite_envelope(
        event: &store::KernelEventEnvelope,
        seq: store::StreamSeq,
        ordinal: store::CommitOrdinal,
        commit_key: store::CommitKey,
    ) -> store::KernelEventEnvelope {
        store::KernelEventEnvelope::from_persisted_record(store::PersistedKernelEventRecord {
            event_id: event_id_for(event, seq, ordinal),
            event_schema_id: event.event_schema_id().clone(),
            run_id: event.run_id().clone(),
            seq,
            ordinal,
            spec_hash: event.spec_hash().clone(),
            commit_key,
            logical_key: event.logical_key().clone(),
            payload_hash: event.payload_hash().clone(),
            payload: event.payload().clone(),
            payload_canonical_byte_len: event.audit().payload_canonical_byte_len(),
        })
        .expect("rewritten envelope")
    }

    fn rewrite_commit_payload(
        stream: &[store::KernelEventEnvelope],
        target_pos: usize,
        payload: events::KernelEventPayload,
    ) -> Vec<store::KernelEventEnvelope> {
        let target = &stream[target_pos];
        let seq = target.seq();
        let commit_key = target.commit_key().clone();
        let mut positions = stream
            .iter()
            .enumerate()
            .filter_map(|(index, event)| {
                (event.seq() == seq && event.commit_key() == &commit_key).then_some(index)
            })
            .collect::<Vec<_>>();
        positions.sort_by_key(|index| stream[*index].ordinal().as_u32());
        let target_ordinal = target.ordinal().as_u32() as usize;
        assert_eq!(positions[target_ordinal], target_pos);
        let mut payloads = positions
            .iter()
            .map(|index| stream[*index].payload().clone())
            .collect::<Vec<_>>();
        payloads[target_ordinal] = payload;
        let request = store::TypedCommitRequest {
            run_id: target.run_id().clone(),
            expected_next_seq: seq,
            commit_key,
            payloads,
            required_artifacts: Vec::new(),
            preconditions: store::CommitPreconditions::default(),
        };
        let batch =
            store::build_committed_batch(&request, seq).expect("rewritten commit payload batch");
        let mut rewritten = stream.to_vec();
        for (position, event) in positions.into_iter().zip(batch.events().iter().cloned()) {
            rewritten[position] = event;
        }
        rewritten
    }

    fn rewrite_stream_without_payloads<F>(
        stream: &[store::KernelEventEnvelope],
        mut should_remove: F,
    ) -> Vec<store::KernelEventEnvelope>
    where
        F: FnMut(&events::KernelEventPayload) -> bool,
    {
        let mut rewritten = Vec::with_capacity(stream.len());
        let mut index = 0;
        while index < stream.len() {
            let first = &stream[index];
            let seq = first.seq();
            let commit_key = first.commit_key().clone();
            let mut end = index + 1;
            while end < stream.len()
                && stream[end].seq() == seq
                && stream[end].commit_key() == &commit_key
            {
                end += 1;
            }
            let commit = &stream[index..end];
            let payloads = commit
                .iter()
                .filter(|event| !should_remove(event.payload()))
                .map(|event| event.payload().clone())
                .collect::<Vec<_>>();
            assert!(
                !payloads.is_empty(),
                "test corruption helper must not remove an entire commit"
            );
            if payloads.len() == commit.len() {
                rewritten.extend(commit.iter().cloned());
            } else {
                let mut ordinal = 0;
                for event in commit {
                    if should_remove(event.payload()) {
                        continue;
                    }
                    rewritten.push(rewrite_envelope(
                        event,
                        seq,
                        store::CommitOrdinal::new(ordinal),
                        commit_key.clone(),
                    ));
                    ordinal += 1;
                }
            }
            index = end;
        }
        rewritten
    }

    fn rewrite_stream_without_commit_containing<F>(
        stream: &[store::KernelEventEnvelope],
        mut should_remove_commit: F,
    ) -> Vec<store::KernelEventEnvelope>
    where
        F: FnMut(&events::KernelEventPayload) -> bool,
    {
        let mut rewritten = Vec::with_capacity(stream.len());
        let mut index = 0;
        while index < stream.len() {
            let first = &stream[index];
            let original_seq = first.seq();
            let commit_key = first.commit_key().clone();
            let mut end = index + 1;
            while end < stream.len()
                && stream[end].seq() == original_seq
                && stream[end].commit_key() == &commit_key
            {
                end += 1;
            }
            let commit = &stream[index..end];
            if commit
                .iter()
                .any(|event| should_remove_commit(event.payload()))
            {
                index = end;
                continue;
            }
            let seq = next_seq_after_stream(&rewritten).expect("next rewritten stream seq");
            let request = store::TypedCommitRequest {
                run_id: first.run_id().clone(),
                expected_next_seq: seq,
                commit_key,
                payloads: commit
                    .iter()
                    .map(|event| event.payload().clone())
                    .collect::<Vec<_>>(),
                required_artifacts: Vec::new(),
                preconditions: store::CommitPreconditions::default(),
            };
            let batch = store::build_committed_batch(&request, seq)
                .expect("rewritten commit payload batch");
            rewritten.extend(batch.events().iter().cloned());
            index = end;
        }
        rewritten
    }

    fn prepend_payloads_to_retention_projection_commit_for_tests(
        stream: &[store::KernelEventEnvelope],
        prefix_payloads: Vec<events::KernelEventPayload>,
    ) -> Vec<store::KernelEventEnvelope> {
        let mut rewritten = Vec::with_capacity(stream.len() + prefix_payloads.len());
        let mut inserted = false;
        let mut index = 0;
        while index < stream.len() {
            let first = &stream[index];
            let seq = first.seq();
            let commit_key = first.commit_key().clone();
            let mut end = index + 1;
            while end < stream.len()
                && stream[end].seq() == seq
                && stream[end].commit_key() == &commit_key
            {
                end += 1;
            }
            let commit = &stream[index..end];
            if commit.iter().any(|event| {
                matches!(
                    event.payload(),
                    events::KernelEventPayload::RetentionManifestProjected(_)
                )
            }) {
                let mut payloads = prefix_payloads.clone();
                payloads.extend(commit.iter().map(|event| event.payload().clone()));
                let request = store::TypedCommitRequest {
                    run_id: first.run_id().clone(),
                    expected_next_seq: seq,
                    commit_key,
                    payloads,
                    required_artifacts: Vec::new(),
                    preconditions: store::CommitPreconditions::default(),
                };
                let batch = store::build_committed_batch(&request, seq)
                    .expect("rewritten retention projection batch");
                rewritten.extend(batch.events().iter().cloned());
                inserted = true;
            } else {
                rewritten.extend(commit.iter().cloned());
            }
            index = end;
        }
        assert!(inserted, "retention projection commit exists");
        rewritten
    }

    fn append_same_sequence_sidecar_to_retention_projection_for_tests(
        stream: &[store::KernelEventEnvelope],
    ) -> Vec<store::KernelEventEnvelope> {
        let mut rewritten = Vec::with_capacity(stream.len() + 1);
        let mut inserted = false;
        let mut index = 0;
        while index < stream.len() {
            let first = &stream[index];
            let seq = first.seq();
            let commit_key = first.commit_key().clone();
            let mut end = index + 1;
            while end < stream.len()
                && stream[end].seq() == seq
                && stream[end].commit_key() == &commit_key
            {
                end += 1;
            }
            let commit = &stream[index..end];
            rewritten.extend(commit.iter().cloned());
            if commit.iter().any(|event| {
                matches!(
                    event.payload(),
                    events::KernelEventPayload::RetentionManifestProjected(_)
                )
            }) {
                let runtime_evidence = commit
                    .iter()
                    .find_map(|event| match event.payload() {
                        events::KernelEventPayload::RetentionRefsAppended(payload)
                            if payload.reason == events::RetentionReason::RuntimeEvidence =>
                        {
                            Some(payload)
                        }
                        _ => None,
                    })
                    .expect("retention projection runtime evidence refs");
                let sidecar_key =
                    store::CommitKey::new("forged-same-seq-retention-sidecar").expect("commit key");
                let request = store::TypedCommitRequest {
                    run_id: first.run_id().clone(),
                    expected_next_seq: seq,
                    commit_key: sidecar_key.clone(),
                    payloads: vec![events::KernelEventPayload::RetentionRefsAppended(
                        events::RetentionRefsAppended {
                            run_id: runtime_evidence.run_id.clone(),
                            spec_hash: runtime_evidence.spec_hash.clone(),
                            refs: runtime_evidence.refs.clone(),
                            reason: events::RetentionReason::RuntimeEvidence,
                        },
                    )],
                    required_artifacts: Vec::new(),
                    preconditions: store::CommitPreconditions::default(),
                };
                let batch =
                    store::build_committed_batch(&request, seq).expect("sidecar commit batch");
                let next_ordinal =
                    u32::try_from(commit.len()).expect("retention commit ordinal count");
                for (offset, event) in batch.events().iter().enumerate() {
                    rewritten.push(rewrite_envelope(
                        event,
                        seq,
                        store::CommitOrdinal::new(
                            next_ordinal
                                .checked_add(u32::try_from(offset).expect("sidecar ordinal offset"))
                                .expect("sidecar ordinal"),
                        ),
                        sidecar_key.clone(),
                    ));
                }
                inserted = true;
            }
            index = end;
        }
        assert!(inserted, "retention projection commit exists");
        rewritten
    }

    fn split_public_output_payload_to_own_commit_for_tests(
        stream: &[store::KernelEventEnvelope],
    ) -> Vec<store::KernelEventEnvelope> {
        let mut rewritten = Vec::with_capacity(stream.len());
        let mut next_seq = store::StreamSeq::FIRST;
        let mut split = false;
        let mut index = 0;
        while index < stream.len() {
            let first = &stream[index];
            let original_seq = first.seq();
            let commit_key = first.commit_key().clone();
            let mut end = index + 1;
            while end < stream.len()
                && stream[end].seq() == original_seq
                && stream[end].commit_key() == &commit_key
            {
                end += 1;
            }
            let commit = &stream[index..end];
            if commit.iter().any(|event| {
                matches!(
                    event.payload(),
                    events::KernelEventPayload::PublicOutputProduced(_)
                )
            }) {
                let mut terminal_payloads = Vec::new();
                let mut public_event = None;
                for event in commit {
                    if matches!(
                        event.payload(),
                        events::KernelEventPayload::PublicOutputProduced(_)
                    ) {
                        public_event = Some(event);
                    } else {
                        terminal_payloads.push(event.payload().clone());
                    }
                }
                assert!(
                    !terminal_payloads.is_empty(),
                    "public-output commit keeps terminal evidence"
                );
                let terminal_request = store::TypedCommitRequest {
                    run_id: first.run_id().clone(),
                    expected_next_seq: next_seq,
                    commit_key,
                    payloads: terminal_payloads,
                    required_artifacts: Vec::new(),
                    preconditions: store::CommitPreconditions::default(),
                };
                let terminal_batch = store::build_committed_batch(&terminal_request, next_seq)
                    .expect("terminal rewrite batch");
                rewritten.extend(terminal_batch.events().iter().cloned());
                next_seq = increment_stream_seq_for_tests(next_seq);

                rewritten.push(rewrite_envelope(
                    public_event.expect("public output payload"),
                    next_seq,
                    store::CommitOrdinal::new(0),
                    store::CommitKey::new("corrupt-public-output-split").expect("commit key"),
                ));
                next_seq = increment_stream_seq_for_tests(next_seq);
                split = true;
            } else {
                let request = store::TypedCommitRequest {
                    run_id: first.run_id().clone(),
                    expected_next_seq: next_seq,
                    commit_key,
                    payloads: commit.iter().map(|event| event.payload().clone()).collect(),
                    required_artifacts: Vec::new(),
                    preconditions: store::CommitPreconditions::default(),
                };
                let batch =
                    store::build_committed_batch(&request, next_seq).expect("shifted commit batch");
                rewritten.extend(batch.events().iter().cloned());
                next_seq = increment_stream_seq_for_tests(next_seq);
            }
            index = end;
        }
        assert!(split, "public output payload exists");
        rewritten
    }

    fn increment_stream_seq_for_tests(seq: store::StreamSeq) -> store::StreamSeq {
        store::StreamSeq::new(seq.as_u64() + 1).expect("next shifted seq")
    }

    fn append_payload_commit_for_tests(
        stream: &mut Vec<store::KernelEventEnvelope>,
        run_id: &RunId,
        commit_key: &str,
        payload: events::KernelEventPayload,
    ) {
        append_payloads_commit_for_tests(stream, run_id, commit_key, vec![payload]);
    }

    fn append_payloads_commit_for_tests(
        stream: &mut Vec<store::KernelEventEnvelope>,
        run_id: &RunId,
        commit_key: &str,
        payloads: Vec<events::KernelEventPayload>,
    ) {
        let seq = next_seq_after_stream(stream).expect("next stream seq");
        let request = store::TypedCommitRequest {
            run_id: run_id.clone(),
            expected_next_seq: seq,
            commit_key: store::CommitKey::new(commit_key).expect("commit key"),
            payloads,
            required_artifacts: Vec::new(),
            preconditions: store::CommitPreconditions::default(),
        };
        let batch = store::build_committed_batch(&request, seq)
            .expect("appended corrupt payload commit batch");
        stream.extend(batch.events().iter().cloned());
    }

    fn rewrite_single_payload_envelope(
        event: &store::KernelEventEnvelope,
        payload: events::KernelEventPayload,
    ) -> store::KernelEventEnvelope {
        assert_eq!(event.ordinal(), store::CommitOrdinal::new(0));
        let request = store::TypedCommitRequest {
            run_id: event.run_id().clone(),
            expected_next_seq: event.seq(),
            commit_key: event.commit_key().clone(),
            payloads: vec![payload],
            required_artifacts: Vec::new(),
            preconditions: store::CommitPreconditions::default(),
        };
        let batch =
            store::build_committed_batch(&request, event.seq()).expect("rewritten payload batch");
        batch
            .events()
            .first()
            .expect("rewritten payload event")
            .clone()
    }

    fn event_id_for(
        event: &store::KernelEventEnvelope,
        seq: store::StreamSeq,
        ordinal: store::CommitOrdinal,
    ) -> EventId {
        let canonical = canonical_json(serde_json::json!({
            "event_schema_id": event.event_schema_id().as_str(),
            "ordinal": ordinal.as_u32(),
            "payload_hash": event.payload_hash().as_str(),
            "run_id": event.run_id().as_str(),
            "seq": seq.as_u64(),
        }))
        .expect("event id canonical");
        EventId::from_digest(DigestAlgorithm::Sha256JcsV1, canonical.digest_bytes())
    }

    fn run_start_evidence(
        fixture: &Fixture,
        seed_cells: Vec<events::SeedCellRef>,
    ) -> RunStartEvidence {
        RunStartEvidence {
            spec_artifact: spec_artifact(&fixture.runtime_spec),
            certificate_artifact: certificate_artifact(&fixture.runtime_spec),
            config_artifacts: fixture
                .runtime_spec
                .spec()
                .config_refs
                .iter()
                .map(config_artifact)
                .collect(),
            framework_version: events::FrameworkVersion::new("mfm.test.1").expect("framework"),
            source_revision: events::SourceRevision::new("test-rev").expect("source"),
            adapter_executables: Vec::new(),
            seed_cells,
        }
    }

    fn spec_artifact(runtime_spec: &CertifiedRuntimeSpec) -> store::ArtifactEvidenceRef {
        let canonical = runtime_spec
            .spec()
            .canonical_json()
            .expect("canonical spec");
        let digest = canonical.content_digest();
        store::ArtifactEvidenceRef {
            artifact_id: ArtifactId::from_digest(digest.algorithm(), *digest.digest()),
            digest,
            byte_len: canonical.as_bytes().len() as u64,
            media_type: runtime_spec.spec().media_type.clone(),
            schema_id: None,
            semantic_type_id: None,
            producer_node_id: None,
            producer_seed_id: None,
            artifact_role: events::ArtifactRole::TypedExecutionSpec,
        }
    }

    fn certificate_artifact(runtime_spec: &CertifiedRuntimeSpec) -> store::ArtifactEvidenceRef {
        let canonical = runtime_spec
            .certificate()
            .canonical_json()
            .expect("canonical certificate");
        let digest = canonical.content_digest();
        store::ArtifactEvidenceRef {
            artifact_id: ArtifactId::from_digest(digest.algorithm(), *digest.digest()),
            digest,
            byte_len: canonical.as_bytes().len() as u64,
            media_type: spec::MediaType::new(mfm_certify::CERTIFICATE_MEDIA_TYPE)
                .expect("certificate media type"),
            schema_id: None,
            semantic_type_id: None,
            producer_node_id: None,
            producer_seed_id: None,
            artifact_role: events::ArtifactRole::TypedSpecCertificate,
        }
    }

    fn config_artifact(config: &spec::ConfigRef) -> store::ArtifactEvidenceRef {
        store::ArtifactEvidenceRef {
            artifact_id: config.artifact_id.clone(),
            digest: config.digest.clone(),
            byte_len: config.byte_len,
            media_type: config.media_type.clone(),
            schema_id: Some(config.schema_id.clone()),
            semantic_type_id: None,
            producer_node_id: None,
            producer_seed_id: None,
            artifact_role: events::ArtifactRole::TypedConfig,
        }
    }

    fn terminal_payloads(
        ctx: &ErasedRunCtx<'_>,
        output_artifact: ArtifactId,
        output_digest: ContentDigest,
    ) -> Vec<events::KernelEventPayload> {
        vec![
            events::KernelEventPayload::CellProduced(events::CellProduced {
                spec_hash: ctx.spec_hash().clone(),
                node_id: ctx.node().node_id.clone(),
                cell_id: ctx.node().output_cell.clone(),
                scope_id: ctx.node().scope_id.clone(),
                attempt_id: ctx.attempt_id().clone(),
                semantic_type_id: ctx.descriptor().output_semantic_type_id.clone(),
                schema_id: ctx.descriptor().output_schema_id.clone(),
                value_lineage: ctx.output_cell().value_lineage.clone(),
                artifact_id: output_artifact,
                content_digest: output_digest,
                producer_state_kind: Some(ctx.node().state_kind.clone()),
                producer_state_version: Some(ctx.node().state_version.clone()),
            }),
            events::KernelEventPayload::StateAttemptCompleted(events::StateAttemptCompleted {
                spec_hash: ctx.spec_hash().clone(),
                node_id: ctx.node().node_id.clone(),
                attempt_id: ctx.attempt_id().clone(),
                output_cell_id: ctx.node().output_cell.clone(),
            }),
        ]
    }

    fn state_output_artifact(
        node: &spec::NodeSpec,
        descriptor: &spec::StateDescriptorIdentity,
        artifact_id: ArtifactId,
        digest: ContentDigest,
    ) -> store::ArtifactEvidenceRef {
        store::ArtifactEvidenceRef {
            artifact_id,
            digest,
            byte_len: 17,
            media_type: spec::MediaType::new("application/json").expect("media"),
            schema_id: Some(descriptor.output_schema_id.clone()),
            semantic_type_id: Some(descriptor.output_semantic_type_id.clone()),
            producer_node_id: Some(node.node_id.clone()),
            producer_seed_id: None,
            artifact_role: events::ArtifactRole::StateOutput,
        }
    }

    fn state_output_artifact_for_bytes(
        node: &spec::NodeSpec,
        descriptor: &spec::StateDescriptorIdentity,
        bytes: &[u8],
    ) -> store::ArtifactEvidenceRef {
        let digest = digest_for_bytes(bytes);
        let artifact_id = ArtifactId::from_digest(digest.algorithm(), *digest.digest());
        store::ArtifactEvidenceRef {
            artifact_id,
            digest,
            byte_len: bytes.len() as u64,
            media_type: spec::MediaType::new("application/json").expect("media"),
            schema_id: Some(descriptor.output_schema_id.clone()),
            semantic_type_id: Some(descriptor.output_semantic_type_id.clone()),
            producer_node_id: Some(node.node_id.clone()),
            producer_seed_id: None,
            artifact_role: events::ArtifactRole::StateOutput,
        }
    }

    fn event_artifact_ref_from_store(
        artifact: &store::ArtifactEvidenceRef,
    ) -> events::ArtifactEvidenceRef {
        events::ArtifactEvidenceRef {
            artifact_id: artifact.artifact_id.clone(),
            role: artifact.artifact_role,
            schema_id: artifact.schema_id.clone().expect("schema-bearing artifact"),
            semantic_type_id: artifact.semantic_type_id.clone(),
            content_digest: artifact.digest.clone(),
            byte_len: artifact.byte_len,
            media_type: artifact.media_type.clone(),
        }
    }

    fn digest_for_bytes(bytes: &[u8]) -> ContentDigest {
        ContentDigest::from_digest(DigestAlgorithm::Sha256JcsV1, sha256_digest_bytes(bytes))
    }

    fn staged_attempt_artifact(
        ctx: &ErasedRunCtx<'_>,
        evidence: store::ArtifactEvidenceRef,
    ) -> Result<StagedArtifact> {
        let binding = staged_artifact_binding_kind(evidence.artifact_role).expect("staged role");
        StagedArtifact::finalized_attempt_artifact_for_tests(ctx, evidence, binding)
    }

    fn staged_side_effect_artifact(
        ctx: &ErasedRunCtx<'_>,
        evidence: store::ArtifactEvidenceRef,
        ledger_key: events::SideEffectLedgerKey,
        invocation_epoch: u32,
    ) -> Result<StagedArtifact> {
        let phase = staged_side_effect_artifact_phase(evidence.artifact_role)
            .expect("staged side-effect role");
        StagedArtifact::finalized_attempt_artifact_for_tests(
            ctx,
            evidence,
            StagedArtifactBindingKind::SideEffectEvidence {
                ledger_key,
                invocation_epoch,
                phase,
            },
        )
    }

    fn node_by_output<'a>(fixture: &'a Fixture, cell_id: &CellId) -> &'a spec::NodeSpec {
        fixture
            .runtime_spec
            .topological_order()
            .iter()
            .filter_map(|node_id| fixture.runtime_spec.node(node_id))
            .find(|node| &node.output_cell == cell_id)
            .expect("node by output")
    }

    fn append_attempt_start(
        store: &mut store::InMemoryTypedRunStore,
        fixture: &Fixture,
        node: &spec::NodeSpec,
        attempt_no: u32,
    ) -> AttemptId {
        let attempt_id = attempt_id(
            &fixture.run_id,
            fixture.runtime_spec.spec_hash(),
            &node.node_id,
            attempt_no,
        )
        .expect("attempt id");
        store
            .append_prepared_commit(store::TypedCommitRequest {
                run_id: fixture.run_id.clone(),
                expected_next_seq: store.expected_next_seq(&fixture.run_id),
                commit_key: store::CommitKey::new(format!(
                    "manual-attempt-start:{}:{}",
                    node.node_id, attempt_id
                ))
                .expect("commit key"),
                payloads: vec![events::KernelEventPayload::StateAttemptStarted(
                    events::StateAttemptStarted {
                        spec_hash: fixture.runtime_spec.spec_hash().clone(),
                        node_id: node.node_id.clone(),
                        attempt_id: attempt_id.clone(),
                        attempt_no,
                        state_kind: node.state_kind.clone(),
                        state_version: node.state_version.clone(),
                    },
                )],
                required_artifacts: Vec::new(),
                preconditions: store::CommitPreconditions {
                    required_run_state: store::RequiredRunState::NotCompleted,
                    required_cell_states: vec![store::CellStatePrecondition {
                        cell_id: node.output_cell.clone(),
                        required: store::RequiredCellState::Absent,
                    }],
                    ..store::CommitPreconditions::default()
                },
            })
            .expect("append attempt start");
        attempt_id
    }

    fn append_fact(
        store: &mut store::InMemoryTypedRunStore,
        fixture: &Fixture,
        node: &spec::NodeSpec,
        attempt_id: &AttemptId,
        fact_key: events::FactKey,
        artifact_id: ArtifactId,
        response_hash: ContentDigest,
    ) {
        let response_schema_id = node.config_ref.schema_id.clone();
        let evidence = store::ArtifactEvidenceRef {
            artifact_id: artifact_id.clone(),
            digest: response_hash.clone(),
            byte_len: 10,
            media_type: spec::MediaType::new("application/json").expect("media"),
            schema_id: Some(response_schema_id.clone()),
            semantic_type_id: None,
            producer_node_id: Some(node.node_id.clone()),
            producer_seed_id: None,
            artifact_role: events::ArtifactRole::FactResponse,
        };
        store
            .append_prepared_commit(store::TypedCommitRequest {
                run_id: fixture.run_id.clone(),
                expected_next_seq: store.expected_next_seq(&fixture.run_id),
                commit_key: store::CommitKey::new(format!(
                    "manual-fact:{}:{}:{}",
                    node.node_id, attempt_id, fact_key
                ))
                .expect("commit key"),
                payloads: vec![events::KernelEventPayload::FactRecorded(
                    events::FactRecorded {
                        spec_hash: fixture.runtime_spec.spec_hash().clone(),
                        node_id: node.node_id.clone(),
                        attempt_id: attempt_id.clone(),
                        capability_kind: fixture.cap_kind.clone(),
                        capability_version: fixture.cap_version.clone(),
                        adapter_kind: fixture.adapter_kind.clone(),
                        adapter_version: fixture.adapter_version.clone(),
                        request_schema_id: node.config_ref.schema_id.clone(),
                        request_hash: content(0xd4),
                        response_schema_id,
                        response_hash,
                        fact_key,
                        artifact_id,
                    },
                )],
                required_artifacts: vec![evidence],
                preconditions: store::CommitPreconditions {
                    required_run_state: store::RequiredRunState::NotCompleted,
                    required_present_logical_keys: vec![store::LogicalEventKey::new(format!(
                        "attempt:{}:{}",
                        node.node_id, attempt_id
                    ))
                    .expect("attempt logical key")],
                    ..store::CommitPreconditions::default()
                },
            })
            .expect("append fact");
    }

    fn append_terminal(
        store: &mut store::InMemoryTypedRunStore,
        fixture: &Fixture,
        node: &spec::NodeSpec,
        attempt_id: &AttemptId,
        artifact_id: ArtifactId,
        output_digest: ContentDigest,
    ) {
        let descriptor = fixture
            .runtime_spec
            .state_descriptor_for_node(node)
            .expect("descriptor");
        let output_cell = fixture
            .runtime_spec
            .cell(&node.output_cell)
            .expect("output cell");
        let evidence =
            state_output_artifact(node, descriptor, artifact_id.clone(), output_digest.clone());
        store
            .append_prepared_commit(store::TypedCommitRequest {
                run_id: fixture.run_id.clone(),
                expected_next_seq: store.expected_next_seq(&fixture.run_id),
                commit_key: store::CommitKey::new(format!(
                    "manual-terminal:{}:{}",
                    node.node_id, attempt_id
                ))
                .expect("commit key"),
                payloads: vec![
                    events::KernelEventPayload::CellProduced(events::CellProduced {
                        spec_hash: fixture.runtime_spec.spec_hash().clone(),
                        node_id: node.node_id.clone(),
                        cell_id: node.output_cell.clone(),
                        scope_id: node.scope_id.clone(),
                        attempt_id: attempt_id.clone(),
                        semantic_type_id: descriptor.output_semantic_type_id.clone(),
                        schema_id: descriptor.output_schema_id.clone(),
                        value_lineage: output_cell.value_lineage.clone(),
                        artifact_id,
                        content_digest: output_digest,
                        producer_state_kind: Some(node.state_kind.clone()),
                        producer_state_version: Some(node.state_version.clone()),
                    }),
                    events::KernelEventPayload::StateAttemptCompleted(
                        events::StateAttemptCompleted {
                            spec_hash: fixture.runtime_spec.spec_hash().clone(),
                            node_id: node.node_id.clone(),
                            attempt_id: attempt_id.clone(),
                            output_cell_id: node.output_cell.clone(),
                        },
                    ),
                ],
                required_artifacts: vec![evidence],
                preconditions: store::CommitPreconditions {
                    required_run_state: store::RequiredRunState::NotCompleted,
                    required_present_logical_keys: vec![store::LogicalEventKey::new(format!(
                        "attempt:{}:{}",
                        node.node_id, attempt_id
                    ))
                    .expect("attempt logical key")],
                    required_cell_states: vec![store::CellStatePrecondition {
                        cell_id: node.output_cell.clone(),
                        required: store::RequiredCellState::Absent,
                    }],
                    ..store::CommitPreconditions::default()
                },
            })
            .expect("append terminal");
    }

    fn append_public_output_render_failure(
        store: &mut store::InMemoryTypedRunStore,
        fixture: &Fixture,
        node: &spec::NodeSpec,
        attempt_id: &AttemptId,
    ) {
        let Some(spec::FrameworkNodeSpec::PublicOutputRender(render)) = &node.framework else {
            panic!("expected public-output render node");
        };
        let error = public_output_error();
        store
            .append_prepared_commit(store::TypedCommitRequest {
                run_id: fixture.run_id.clone(),
                expected_next_seq: store.expected_next_seq(&fixture.run_id),
                commit_key: store::CommitKey::new(format!(
                    "manual-public-output-failure:{}:{}",
                    node.node_id, attempt_id
                ))
                .expect("commit key"),
                payloads: vec![
                    events::KernelEventPayload::PublicOutputRenderFailed(
                        events::PublicOutputRenderFailed {
                            spec_hash: fixture.runtime_spec.spec_hash().clone(),
                            node_id: node.node_id.clone(),
                            attempt_id: attempt_id.clone(),
                            public_schema_id: render.public_schema_id.clone(),
                            renderer_descriptor_id: render
                                .renderer_descriptor
                                .descriptor_id
                                .clone(),
                            error: error.clone(),
                        },
                    ),
                    events::KernelEventPayload::StateAttemptFailed(events::StateAttemptFailed {
                        spec_hash: fixture.runtime_spec.spec_hash().clone(),
                        node_id: node.node_id.clone(),
                        attempt_id: attempt_id.clone(),
                        retryable: true,
                        error,
                    }),
                ],
                required_artifacts: Vec::new(),
                preconditions: store::CommitPreconditions {
                    required_run_state: store::RequiredRunState::NotCompleted,
                    required_present_logical_keys: vec![store::LogicalEventKey::new(format!(
                        "attempt:{}:{}",
                        node.node_id, attempt_id
                    ))
                    .expect("attempt logical key")],
                    required_cell_states: vec![store::CellStatePrecondition {
                        cell_id: node.output_cell.clone(),
                        required: store::RequiredCellState::Absent,
                    }],
                    required_public_output_absent: true,
                    ..store::CommitPreconditions::default()
                },
            })
            .expect("append public output failure");
    }

    fn append_not_submitted_proven(
        store: &mut store::InMemoryTypedRunStore,
        fixture: &Fixture,
        node: &spec::NodeSpec,
        attempt_id: &AttemptId,
        invocation_epoch: u32,
    ) {
        let proof_artifact = artifact(0xd5);
        let proof_hash = content(0xd6);
        let evidence = store::ArtifactEvidenceRef {
            artifact_id: proof_artifact.clone(),
            digest: proof_hash.clone(),
            byte_len: 19,
            media_type: spec::MediaType::new("application/json").expect("media"),
            schema_id: Some(node.config_ref.schema_id.clone()),
            semantic_type_id: None,
            producer_node_id: Some(node.node_id.clone()),
            producer_seed_id: None,
            artifact_role: events::ArtifactRole::NotSubmittedProof,
        };
        store
            .append_prepared_commit(store::TypedCommitRequest {
                run_id: fixture.run_id.clone(),
                expected_next_seq: store.expected_next_seq(&fixture.run_id),
                commit_key: store::CommitKey::new(format!(
                    "manual-not-submitted:{}:{}",
                    node.node_id, attempt_id
                ))
                .expect("commit key"),
                payloads: vec![events::KernelEventPayload::SideEffectNotSubmittedProven(
                    events::side_effect::NotSubmittedProven {
                        spec_hash: fixture.runtime_spec.spec_hash().clone(),
                        node_id: node.node_id.clone(),
                        attempt_id: attempt_id.clone(),
                        ledger_key: side_effect_ledger_key(1),
                        invocation_epoch,
                        proof_schema_id: node.config_ref.schema_id.clone(),
                        proof_hash,
                        proof_artifact_id: proof_artifact,
                    },
                )],
                required_artifacts: vec![evidence],
                preconditions: store::CommitPreconditions {
                    required_run_state: store::RequiredRunState::NotCompleted,
                    required_present_logical_keys: vec![store::LogicalEventKey::new(format!(
                        "attempt:{}:{}",
                        node.node_id, attempt_id
                    ))
                    .expect("attempt logical key")],
                    ..store::CommitPreconditions::default()
                },
            })
            .expect("append not-submitted proof");
    }

    fn attempt_started_count(
        store: &store::InMemoryTypedRunStore,
        run_id: &RunId,
        node_id: &NodeId,
    ) -> usize {
        store
            .load_run_stream(run_id)
            .iter()
            .filter(|event| {
                matches!(
                    event.payload(),
                    events::KernelEventPayload::StateAttemptStarted(payload)
                        if &payload.node_id == node_id
                )
            })
            .count()
    }

    fn fact_recorded_count(store: &store::InMemoryTypedRunStore) -> usize {
        store
            .projection_snapshot()
            .facts()
            .filter(|(_, fact)| fact.fact_key.as_str() == "reused-fact")
            .count()
    }

    #[test]
    fn runtime_order_is_deterministic_for_reordered_spec_nodes() {
        let fixture = fixture();
        let mut envelope = fixture.runtime_spec.envelope().clone();
        envelope.spec.nodes.reverse();
        let envelope =
            spec::HashedSpecEnvelope::new(envelope.spec, envelope.audit).expect("rehash");
        let runtime = CertifiedRuntimeSpec::from_verified_envelope(envelope).expect("runtime");
        assert_eq!(
            runtime.topological_order(),
            fixture.runtime_spec.topological_order()
        );
    }

    fn registered_fixture_runners(fixture: &Fixture) -> ErasedRunnerRegistry {
        let mut registry = ErasedRunnerRegistry::new();
        registry
            .register(binding(
                fixture.descriptor_a.clone(),
                "pure",
                RecordingRunner {
                    expected_caps: Vec::new(),
                    output_artifact: artifact(0xa1),
                    output_digest: content(0xa2),
                },
            ))
            .expect("binding a");
        registry
            .register(binding(
                fixture.descriptor_b.clone(),
                "read",
                RecordingRunner {
                    expected_caps: vec![(fixture.cap_kind.clone(), fixture.cap_version.clone())],
                    output_artifact: artifact(0xb1),
                    output_digest: content(0xb2),
                },
            ))
            .expect("binding b");
        registry
    }

    fn registered_side_effect_fixture_runners(fixture: &Fixture) -> ErasedRunnerRegistry {
        let mut registry = ErasedRunnerRegistry::new();
        registry
            .register(binding(
                fixture.descriptor_a.clone(),
                "sidefx",
                DeterministicSideEffectRunner::new(fixture),
            ))
            .expect("binding a");
        registry
            .register(binding(
                fixture.descriptor_b.clone(),
                "read",
                RecordingRunner {
                    expected_caps: vec![(fixture.cap_kind.clone(), fixture.cap_version.clone())],
                    output_artifact: artifact(0xb1),
                    output_digest: content(0xb2),
                },
            ))
            .expect("binding b");
        registry
    }

    fn binding<R: ErasedNodeRunner + 'static>(
        descriptor_id: DescriptorId,
        factory: &str,
        runner: R,
    ) -> ErasedRunnerBinding {
        let factory_id = events::RunnerFactoryId::new(factory).expect("factory");
        ErasedRunnerBinding::new(
            descriptor_id,
            factory_id.clone(),
            events::ExecutableIdentity {
                factory_id,
                source_revision: events::SourceRevision::new("test-rev").expect("source"),
                cargo_package_name: events::PackageName::new("mfm-test").expect("package"),
                cargo_package_version: events::PackageVersion::new("0.1.0").expect("version"),
                cargo_package_digest: content(0xe1),
                binary_digest: content(0xe2),
                nix_derivation_hash: None,
                nix_output_hash: None,
            },
            Arc::new(runner),
        )
        .expect("runner binding")
    }

    fn runtime_render_receipt_cell(typed: &spec::TypedExecutionSpec) -> CellId {
        typed
            .nodes
            .iter()
            .find(|node| {
                matches!(
                    node.framework,
                    Some(spec::FrameworkNodeSpec::PublicOutputRender(_))
                )
            })
            .expect("render node")
            .output_cell
            .clone()
    }

    fn runtime_retention_receipt_cell(typed: &spec::TypedExecutionSpec) -> CellId {
        typed
            .nodes
            .iter()
            .find(|node| {
                matches!(
                    node.framework,
                    Some(spec::FrameworkNodeSpec::ProjectRetentionManifest(_))
                )
            })
            .expect("retention node")
            .output_cell
            .clone()
    }

    #[derive(Clone, Copy)]
    enum RuntimeLifecycleVariant {
        BootstrapRun,
        ProjectRetentionManifest,
        CompleteRun,
    }

    impl RuntimeLifecycleVariant {
        fn matches(self, framework: &spec::FrameworkNodeSpec) -> bool {
            match self {
                Self::BootstrapRun => {
                    matches!(framework, spec::FrameworkNodeSpec::BootstrapRun(_))
                }
                Self::ProjectRetentionManifest => {
                    matches!(
                        framework,
                        spec::FrameworkNodeSpec::ProjectRetentionManifest(_)
                    )
                }
                Self::CompleteRun => {
                    matches!(framework, spec::FrameworkNodeSpec::CompleteRun(_))
                }
            }
        }
    }

    fn find_runtime_lifecycle_node_mut(
        typed: &mut spec::TypedExecutionSpec,
        variant: RuntimeLifecycleVariant,
    ) -> &mut spec::NodeSpec {
        typed
            .nodes
            .iter_mut()
            .find(|node| {
                node.framework
                    .as_ref()
                    .is_some_and(|framework| variant.matches(framework))
            })
            .expect("runtime lifecycle node")
    }

    fn clear_runtime_framework_metadata(
        typed: &mut spec::TypedExecutionSpec,
        variant: RuntimeLifecycleVariant,
    ) {
        find_runtime_lifecycle_node_mut(typed, variant).framework = None;
    }

    fn append_runtime_bootstrap_lifecycle_node(typed: &mut spec::TypedExecutionSpec) -> CellId {
        let node_id = NodeId::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([0xe8; 32]),
        );
        let output_cell = CellId::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([0xe9; 32]),
        );
        let descriptor_id = DescriptorId::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([0xea; 32]),
        );
        let config_ref =
            spec::framework_config_ref("bootstrap_run", &node_id).expect("bootstrap config ref");
        let input_binding =
            spec::framework_lifecycle_unit_input_binding("bootstrap_run").expect("input binding");
        let managed = ManagedPlatformWrite::descriptor().expect("managed effect");
        let receipt_schema =
            spec::bootstrap_run_receipt_schema_id().expect("bootstrap receipt schema");
        let receipt_semantic =
            spec::bootstrap_run_receipt_semantic_type_id().expect("bootstrap receipt semantic");
        let no_caps = CapabilitySetDescriptor::new(Vec::new()).expect("no caps");
        let state_kind = StateKind::new(
            "mfm.framework",
            "bootstrap_run",
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([0xed; 32]),
        )
        .expect("state kind");
        let state_version =
            StateVersion::new("mfm.framework.state.bootstrap_run.v1").expect("state version");
        typed
            .descriptor_identities
            .push(spec::DescriptorIdentity::State(Box::new(
                spec::StateDescriptorIdentity {
                    descriptor_id: descriptor_id.clone(),
                    name: "mfm.framework.bootstrap_run".to_owned(),
                    state_kind: state_kind.clone(),
                    state_version: state_version.clone(),
                    config_schema_id: config_ref.schema_id.clone(),
                    input_schema_id: input_binding.input_schema_id.clone(),
                    output_schema_id: receipt_schema.clone(),
                    output_semantic_type_id: receipt_semantic.clone(),
                    effect_kind: managed.kind.clone(),
                    effect_class: managed.class.as_str().to_owned(),
                    effect_name: managed.name.to_owned(),
                    effect_version: managed.version,
                    capabilities: no_caps.clone(),
                    runner: "managed_platform_write".to_owned(),
                    side_effect_contract_digest: None,
                },
            )));
        typed.config_refs.push(config_ref.clone());
        typed.cells.push(spec::CellSpec {
            cell_id: output_cell.clone(),
            producer: spec::CellProducer::Node(node_id.clone()),
            scope_id: typed.scopes[0].scope_id.clone(),
            semantic_type_id: receipt_semantic,
            schema_id: receipt_schema,
            value_lineage: spec::ValueLineageRef {
                lineage_digest: content(0xee),
            },
            terminal_policy: spec::CellTerminalPolicy::ProducedOnly,
            storage_policy: spec::StoragePolicy::ContentAddressed,
            redaction_policy: spec::RedactionPolicy::Public,
        });
        typed.nodes.push(spec::NodeSpec {
            node_id: node_id.clone(),
            stable_key: spec::StableAuthorKey::new("framework/bootstrap-run").expect("stable key"),
            scope_id: typed.scopes[0].scope_id.clone(),
            state_kind,
            state_version,
            descriptor_id,
            config_ref,
            input_bindings: input_binding,
            output_cell: output_cell.clone(),
            effect_kind: managed.kind,
            capability_bindings: no_caps,
            adapter_bindings: Vec::new(),
            side_effect: None,
            framework: Some(spec::FrameworkNodeSpec::BootstrapRun(
                spec::BootstrapRunNodeSpec {},
            )),
            planning_lineage: typed.scopes[0].planning_lineage.clone(),
            deterministic_predecessors: Vec::new(),
        });
        output_cell
    }

    fn append_runtime_retention_lifecycle_node(
        typed: &mut spec::TypedExecutionSpec,
        public_output_receipt_cell: CellId,
        valid_ordering: bool,
    ) -> NodeId {
        let node_id = NodeId::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([0xf1; 32]),
        );
        let output_cell = CellId::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([0xf2; 32]),
        );
        let descriptor_id = DescriptorId::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([0xf3; 32]),
        );
        let config_ref = spec::framework_config_ref("project_retention_manifest", &node_id)
            .expect("retention config ref");
        let input_cell = typed
            .cells
            .iter()
            .find(|cell| cell.cell_id == public_output_receipt_cell)
            .expect("input cell")
            .clone();
        let input_binding = spec::framework_lifecycle_receipt_input_binding(
            "project_retention_manifest",
            "public_output_receipt",
            &input_cell,
        )
        .expect("input binding");
        let managed = ManagedPlatformWrite::descriptor().expect("managed effect");
        let receipt_schema =
            spec::retention_manifest_receipt_schema_id().expect("retention receipt schema");
        let receipt_semantic = spec::retention_manifest_receipt_semantic_type_id()
            .expect("retention receipt semantic");
        let no_caps = CapabilitySetDescriptor::new(Vec::new()).expect("no caps");
        let state_kind = StateKind::new(
            "mfm.framework",
            "project_retention_manifest",
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([0xf9; 32]),
        )
        .expect("state kind");
        let state_version = StateVersion::new("mfm.framework.state.project_retention_manifest.v1")
            .expect("state version");
        typed
            .descriptor_identities
            .push(spec::DescriptorIdentity::State(Box::new(
                spec::StateDescriptorIdentity {
                    descriptor_id: descriptor_id.clone(),
                    name: "mfm.framework.project_retention_manifest".to_owned(),
                    state_kind: state_kind.clone(),
                    state_version: state_version.clone(),
                    config_schema_id: config_ref.schema_id.clone(),
                    input_schema_id: input_binding.input_schema_id.clone(),
                    output_schema_id: receipt_schema.clone(),
                    output_semantic_type_id: receipt_semantic.clone(),
                    effect_kind: managed.kind.clone(),
                    effect_class: managed.class.as_str().to_owned(),
                    effect_name: managed.name.to_owned(),
                    effect_version: managed.version,
                    capabilities: no_caps.clone(),
                    runner: "managed_platform_write".to_owned(),
                    side_effect_contract_digest: None,
                },
            )));
        typed.config_refs.push(config_ref.clone());
        typed.cells.push(spec::CellSpec {
            cell_id: output_cell.clone(),
            producer: spec::CellProducer::Node(node_id.clone()),
            scope_id: input_cell.scope_id.clone(),
            semantic_type_id: receipt_semantic,
            schema_id: receipt_schema,
            value_lineage: spec::ValueLineageRef {
                lineage_digest: content(0xfa),
            },
            terminal_policy: spec::CellTerminalPolicy::ProducedOnly,
            storage_policy: spec::StoragePolicy::ContentAddressed,
            redaction_policy: spec::RedactionPolicy::Public,
        });
        let predecessors = match &input_cell.producer {
            spec::CellProducer::Node(producer) => vec![producer.clone()],
            spec::CellProducer::Seed(_) => Vec::new(),
        };
        let framework_receipt = if valid_ordering {
            public_output_receipt_cell
        } else {
            input_cell.cell_id.clone()
        };
        typed.nodes.push(spec::NodeSpec {
            node_id: node_id.clone(),
            stable_key: spec::StableAuthorKey::new("framework/project-retention-manifest")
                .expect("stable key"),
            scope_id: input_cell.scope_id,
            state_kind,
            state_version,
            descriptor_id,
            config_ref,
            input_bindings: input_binding,
            output_cell,
            effect_kind: managed.kind,
            capability_bindings: no_caps,
            adapter_bindings: Vec::new(),
            side_effect: None,
            framework: Some(spec::FrameworkNodeSpec::ProjectRetentionManifest(
                spec::ProjectRetentionManifestNodeSpec {
                    public_schema_id: typed.public_outputs.public_schema_id.clone(),
                    public_output_receipt_cell: framework_receipt,
                },
            )),
            planning_lineage: typed.scopes[0].planning_lineage.clone(),
            deterministic_predecessors: predecessors,
        });
        node_id
    }

    fn append_runtime_complete_lifecycle_node(
        typed: &mut spec::TypedExecutionSpec,
        retention_manifest_receipt_cell: CellId,
        valid_ordering: bool,
    ) -> NodeId {
        let node_id = NodeId::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([0xb2; 32]),
        );
        let output_cell = CellId::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([0xb3; 32]),
        );
        let descriptor_id = DescriptorId::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([0xb4; 32]),
        );
        let config_ref =
            spec::framework_config_ref("complete_run", &node_id).expect("complete config ref");
        let input_cell = typed
            .cells
            .iter()
            .find(|cell| cell.cell_id == retention_manifest_receipt_cell)
            .expect("input cell")
            .clone();
        let input_binding = spec::framework_lifecycle_receipt_input_binding(
            "complete_run",
            "retention_manifest_receipt",
            &input_cell,
        )
        .expect("input binding");
        let managed = ManagedPlatformWrite::descriptor().expect("managed effect");
        let receipt_schema =
            spec::complete_run_receipt_schema_id().expect("complete receipt schema");
        let receipt_semantic =
            spec::complete_run_receipt_semantic_type_id().expect("complete receipt semantic");
        let no_caps = CapabilitySetDescriptor::new(Vec::new()).expect("no caps");
        let state_kind = StateKind::new(
            "mfm.framework",
            "complete_run",
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([0xb8; 32]),
        )
        .expect("state kind");
        let state_version =
            StateVersion::new("mfm.framework.state.complete_run.v1").expect("state version");
        typed
            .descriptor_identities
            .push(spec::DescriptorIdentity::State(Box::new(
                spec::StateDescriptorIdentity {
                    descriptor_id: descriptor_id.clone(),
                    name: "mfm.framework.complete_run".to_owned(),
                    state_kind: state_kind.clone(),
                    state_version: state_version.clone(),
                    config_schema_id: config_ref.schema_id.clone(),
                    input_schema_id: input_binding.input_schema_id.clone(),
                    output_schema_id: receipt_schema.clone(),
                    output_semantic_type_id: receipt_semantic.clone(),
                    effect_kind: managed.kind.clone(),
                    effect_class: managed.class.as_str().to_owned(),
                    effect_name: managed.name.to_owned(),
                    effect_version: managed.version,
                    capabilities: no_caps.clone(),
                    runner: "managed_platform_write".to_owned(),
                    side_effect_contract_digest: None,
                },
            )));
        typed.config_refs.push(config_ref.clone());
        typed.cells.push(spec::CellSpec {
            cell_id: output_cell.clone(),
            producer: spec::CellProducer::Node(node_id.clone()),
            scope_id: input_cell.scope_id.clone(),
            semantic_type_id: receipt_semantic,
            schema_id: receipt_schema,
            value_lineage: spec::ValueLineageRef {
                lineage_digest: content(0xbb),
            },
            terminal_policy: spec::CellTerminalPolicy::ProducedOnly,
            storage_policy: spec::StoragePolicy::ContentAddressed,
            redaction_policy: spec::RedactionPolicy::Public,
        });
        let predecessors = match &input_cell.producer {
            spec::CellProducer::Node(producer) => vec![producer.clone()],
            spec::CellProducer::Seed(_) => Vec::new(),
        };
        let framework_receipt = if valid_ordering {
            retention_manifest_receipt_cell
        } else {
            typed
                .public_outputs
                .outputs
                .first()
                .expect("public output")
                .cell_id
                .clone()
        };
        typed.nodes.push(spec::NodeSpec {
            node_id: node_id.clone(),
            stable_key: spec::StableAuthorKey::new("framework/complete-run").expect("stable key"),
            scope_id: input_cell.scope_id,
            state_kind,
            state_version,
            descriptor_id,
            config_ref,
            input_bindings: input_binding,
            output_cell,
            effect_kind: managed.kind,
            capability_bindings: no_caps,
            adapter_bindings: Vec::new(),
            side_effect: None,
            framework: Some(spec::FrameworkNodeSpec::CompleteRun(
                spec::CompleteRunNodeSpec {
                    public_schema_id: typed.public_outputs.public_schema_id.clone(),
                    retention_manifest_receipt_cell: framework_receipt,
                },
            )),
            planning_lineage: typed.scopes[0].planning_lineage.clone(),
            deterministic_predecessors: predecessors,
        });
        node_id
    }

    fn append_runtime_user_receipt_consumer(
        typed: &mut spec::TypedExecutionSpec,
        receipt_cell: CellId,
        stable_key: &str,
    ) -> NodeId {
        let template = typed
            .nodes
            .iter()
            .find(|node| node.framework.is_none())
            .expect("user node template")
            .clone();
        let input_cell = typed
            .cells
            .iter()
            .find(|cell| cell.cell_id == receipt_cell)
            .expect("receipt cell")
            .clone();
        let output_cell = CellId::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([0xfb; 32]),
        );
        let node_id = NodeId::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([0xfc; 32]),
        );
        let field_path = spec::PublicFieldPath::new("receipt").expect("field path");
        let input_root = spec::InputBindingNodeSpec::Cell(Box::new(spec::InputBindingCellSpec {
            field_path,
            cell_id: input_cell.cell_id.clone(),
            semantic_type_id: input_cell.semantic_type_id.clone(),
            schema_id: input_cell.schema_id.clone(),
            required_terminal: spec::RequiredTerminal::ProducedOnly,
            value_lineage: input_cell.value_lineage.clone(),
        }));
        let mut input_bindings = template.input_bindings.clone();
        input_bindings.root = input_root;
        input_bindings.digest =
            content_digest_json(input_node_json(&input_bindings.root)).expect("input digest");
        let predecessors = match &input_cell.producer {
            spec::CellProducer::Node(producer) => vec![producer.clone()],
            spec::CellProducer::Seed(_) => Vec::new(),
        };
        typed.cells.push(spec::CellSpec {
            cell_id: output_cell.clone(),
            producer: spec::CellProducer::Node(node_id.clone()),
            scope_id: template.scope_id.clone(),
            semantic_type_id: typed
                .cells
                .iter()
                .find(|cell| cell.cell_id == template.output_cell)
                .expect("template output")
                .semantic_type_id
                .clone(),
            schema_id: typed
                .cells
                .iter()
                .find(|cell| cell.cell_id == template.output_cell)
                .expect("template output")
                .schema_id
                .clone(),
            value_lineage: spec::ValueLineageRef {
                lineage_digest: content(0xfd),
            },
            terminal_policy: spec::CellTerminalPolicy::ProducedOnly,
            storage_policy: spec::StoragePolicy::ContentAddressed,
            redaction_policy: spec::RedactionPolicy::Public,
        });
        typed.nodes.push(spec::NodeSpec {
            node_id: node_id.clone(),
            stable_key: spec::StableAuthorKey::new(stable_key).expect("stable key"),
            scope_id: template.scope_id,
            state_kind: template.state_kind,
            state_version: template.state_version,
            descriptor_id: template.descriptor_id,
            config_ref: template.config_ref,
            input_bindings,
            output_cell,
            effect_kind: template.effect_kind,
            capability_bindings: template.capability_bindings,
            adapter_bindings: Vec::new(),
            side_effect: None,
            framework: None,
            planning_lineage: template.planning_lineage,
            deterministic_predecessors: predecessors,
        });
        node_id
    }

    fn append_runtime_independent_user_node(
        typed: &mut spec::TypedExecutionSpec,
        stable_key: &str,
    ) -> NodeId {
        let template = typed
            .nodes
            .iter()
            .find(|node| node.framework.is_none())
            .expect("user node template")
            .clone();
        let template_output = typed
            .cells
            .iter()
            .find(|cell| cell.cell_id == template.output_cell)
            .expect("template output")
            .clone();
        let input_cells = runtime_collect_input_cells(&template.input_bindings.root);
        let output_cell = CellId::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([0xee; 32]),
        );
        let node_id = NodeId::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([0xef; 32]),
        );
        typed.cells.push(spec::CellSpec {
            cell_id: output_cell.clone(),
            producer: spec::CellProducer::Node(node_id.clone()),
            scope_id: template.scope_id.clone(),
            semantic_type_id: template_output.semantic_type_id,
            schema_id: template_output.schema_id,
            value_lineage: spec::ValueLineageRef {
                lineage_digest: content(0xee),
            },
            terminal_policy: spec::CellTerminalPolicy::ProducedOnly,
            storage_policy: spec::StoragePolicy::ContentAddressed,
            redaction_policy: spec::RedactionPolicy::Public,
        });
        typed.nodes.push(spec::NodeSpec {
            node_id: node_id.clone(),
            stable_key: spec::StableAuthorKey::new(stable_key).expect("stable key"),
            scope_id: template.scope_id,
            state_kind: template.state_kind,
            state_version: template.state_version,
            descriptor_id: template.descriptor_id,
            config_ref: template.config_ref,
            input_bindings: template.input_bindings,
            output_cell,
            effect_kind: template.effect_kind,
            capability_bindings: template.capability_bindings,
            adapter_bindings: template.adapter_bindings,
            side_effect: template.side_effect,
            framework: None,
            planning_lineage: template.planning_lineage,
            deterministic_predecessors: runtime_predecessors_for_inputs(typed, &input_cells),
        });
        node_id
    }

    fn runtime_collect_input_cells(root: &spec::InputBindingNodeSpec) -> Vec<CellId> {
        let mut cells = Vec::new();
        runtime_collect_input_cells_into(root, &mut cells);
        cells
    }

    fn runtime_collect_input_cells_into(
        root: &spec::InputBindingNodeSpec,
        output: &mut Vec<CellId>,
    ) {
        match root {
            spec::InputBindingNodeSpec::Unit => {}
            spec::InputBindingNodeSpec::Cell(cell) => output.push(cell.cell_id.clone()),
            spec::InputBindingNodeSpec::Tuple(elements) => {
                for element in elements {
                    runtime_collect_input_cells_into(element, output);
                }
            }
            spec::InputBindingNodeSpec::Struct(fields) => {
                for field in fields {
                    runtime_collect_input_cells_into(&field.node, output);
                }
            }
            spec::InputBindingNodeSpec::Vec { elements, .. }
            | spec::InputBindingNodeSpec::NonEmptyVec { elements, .. } => {
                for element in elements {
                    runtime_collect_input_cells_into(element, output);
                }
            }
        }
    }

    fn runtime_predecessors_for_inputs(
        typed: &spec::TypedExecutionSpec,
        input_cells: &[CellId],
    ) -> Vec<NodeId> {
        let mut predecessors = BTreeSet::new();
        for input_cell in input_cells {
            let cell = typed
                .cells
                .iter()
                .find(|cell| cell.cell_id == *input_cell)
                .expect("input cell");
            if let spec::CellProducer::Node(node_id) = &cell.producer {
                predecessors.insert(node_id.clone());
            }
        }
        predecessors.into_iter().collect()
    }

    fn fixture() -> Fixture {
        let scope = ScopeId::from_digest(DigestAlgorithm::Sha256JcsV1, D0);
        let seed_id = SeedId::from_digest(DigestAlgorithm::Sha256JcsV1, D1);
        let seed_cell = CellId::from_digest(DigestAlgorithm::Sha256JcsV1, D2);
        let node_a = NodeId::from_digest(DigestAlgorithm::Sha256JcsV1, D3);
        let cell_a = CellId::from_digest(DigestAlgorithm::Sha256JcsV1, D4);
        let node_b = NodeId::from_digest(DigestAlgorithm::Sha256JcsV1, D5);
        let cell_b = CellId::from_digest(DigestAlgorithm::Sha256JcsV1, D6);
        let descriptor_a = DescriptorId::from_digest(DigestAlgorithm::Sha256JcsV1, D7);
        let descriptor_b = DescriptorId::from_digest(DigestAlgorithm::Sha256JcsV1, D8);
        let semantic =
            SemanticTypeId::new("mfm.test", "value", "1", DigestAlgorithm::Sha256JcsV1, D9)
                .expect("semantic");
        let value_schema =
            SchemaId::new("mfm.test.value", "1", DigestAlgorithm::Sha256JcsV1, DA).expect("schema");
        let input_schema = SchemaId::new("mfm.test.input", "1", DigestAlgorithm::Sha256JcsV1, DB)
            .expect("input schema");
        let config_schema = SchemaId::new("mfm.test.config", "1", DigestAlgorithm::Sha256JcsV1, DC)
            .expect("config schema");
        let public_schema = SchemaId::new("mfm.test.public", "1", DigestAlgorithm::Sha256JcsV1, DD)
            .expect("public schema");
        let effect_kind =
            EffectKind::new("mfm.test", "pure", DigestAlgorithm::Sha256JcsV1, DE).expect("effect");
        let read_effect = EffectKind::new("mfm.test", "read", DigestAlgorithm::Sha256JcsV1, DF)
            .expect("read effect");
        let cap_kind = CapabilityKind::new("mfm.test", "read-db", DigestAlgorithm::Sha256JcsV1, D0)
            .expect("cap kind");
        let cap_version = CapabilityVersion::new("mfm.cap.read_db.v1").expect("cap version");
        let adapter_kind =
            AdapterKind::new("mfm.test", "adapter", DigestAlgorithm::Sha256JcsV1, D1)
                .expect("adapter kind");
        let adapter_version = AdapterVersion::new("mfm.adapter.v1").expect("adapter version");
        let read_cap = CapabilityDescriptor::new(
            cap_kind.clone(),
            cap_version.clone(),
            CapabilityRole::ReadExternal,
            "read-db",
        )
        .expect("capability");
        let no_caps = CapabilitySetDescriptor::new(Vec::new()).expect("no caps");
        let read_caps = CapabilitySetDescriptor::new(vec![read_cap]).expect("read caps");
        let config_ref = spec::ConfigRef {
            schema_id: config_schema.clone(),
            artifact_id: artifact(0x31),
            digest: content(0x32),
            byte_len: 2,
            media_type: spec::MediaType::new("application/json").expect("media"),
        };
        let lineage_seed = spec::ValueLineageRef {
            lineage_digest: content(0x41),
        };
        let lineage_a = spec::ValueLineageRef {
            lineage_digest: content(0x42),
        };
        let lineage_b = spec::ValueLineageRef {
            lineage_digest: content(0x43),
        };
        let planning = spec::PlanningLineage {
            active_operation_instances: Vec::new(),
            completed_operation_frames: Vec::new(),
            lineage_digest: content(0x44),
        };
        let seed_ref = events::SeedCellRef {
            seed_id: seed_id.clone(),
            cell_id: seed_cell.clone(),
            scope_id: scope.clone(),
            semantic_type_id: semantic.clone(),
            schema_id: value_schema.clone(),
            digest: content(0x51),
            seed_artifact: events::ArtifactEvidenceRef {
                artifact_id: artifact(0x52),
                role: events::ArtifactRole::SeedInput,
                schema_id: value_schema.clone(),
                semantic_type_id: Some(semantic.clone()),
                content_digest: content(0x51),
                byte_len: 11,
                media_type: spec::MediaType::new("application/json").expect("media"),
            },
        };
        let renderer = spec::RendererDescriptorIdentity {
            descriptor_id: DescriptorId::from_digest(DigestAlgorithm::Sha256JcsV1, D1),
            renderer_kind: spec::RendererKind::new("public-output/json").expect("renderer"),
            renderer_version: spec::RendererVersion::new("mfm.renderer.test.v1")
                .expect("renderer version"),
            public_schema_id: public_schema.clone(),
            canonicalizer_identity: spec::CanonicalizerIdentity::new("sha256-jcs-v1")
                .expect("canonicalizer"),
        };
        let public_output_cell = spec::PublicOutputCell {
            public_field_path: spec::PublicFieldPath::new("result").expect("field"),
            cell_id: cell_b.clone(),
            producer: spec::CellProducer::Node(node_b.clone()),
            scope_id: scope.clone(),
            semantic_type_id: semantic.clone(),
            schema_id: value_schema.clone(),
            value_lineage: lineage_b.clone(),
            required_terminal: spec::RequiredTerminal::ProducedOnly,
        };
        let public_outputs = spec::PublicOutputSpec {
            public_schema_id: public_schema.clone(),
            outputs: vec![public_output_cell.clone()],
            renderer_descriptor: renderer.clone(),
        };
        let output_spec_digest = public_outputs.digest().expect("public output digest");
        let render_node = NodeId::from_digest(DigestAlgorithm::Sha256JcsV1, D8);
        let render_config_ref = spec::framework_config_ref("public_output_render", &render_node)
            .expect("render config ref");
        let render_cell = CellId::from_digest(DigestAlgorithm::Sha256JcsV1, D9);
        let render_descriptor = DescriptorId::from_digest(DigestAlgorithm::Sha256JcsV1, DA);
        let receipt_schema = spec::public_output_receipt_schema_id().expect("receipt schema");
        let receipt_semantic =
            spec::public_output_receipt_semantic_type_id().expect("receipt semantic");
        let render_lineage = spec::ValueLineageRef {
            lineage_digest: content(0x45),
        };
        let render_input_root =
            spec::InputBindingNodeSpec::Struct(vec![spec::NamedInputBindingSpec {
                field_path: public_output_cell.public_field_path.clone(),
                node: spec::InputBindingNodeSpec::Cell(Box::new(spec::InputBindingCellSpec {
                    field_path: public_output_cell.public_field_path.clone(),
                    cell_id: public_output_cell.cell_id.clone(),
                    semantic_type_id: public_output_cell.semantic_type_id.clone(),
                    schema_id: public_output_cell.schema_id.clone(),
                    required_terminal: public_output_cell.required_terminal,
                    value_lineage: public_output_cell.value_lineage.clone(),
                })),
            }]);
        let render_input_binding = spec::InputBindingSpec {
            input_schema_id: public_schema.clone(),
            input_descriptor_id: DescriptorId::from_digest(DigestAlgorithm::Sha256JcsV1, DC),
            digest: content_digest_json(input_node_json(&render_input_root))
                .expect("render input digest"),
            root: render_input_root,
        };
        let managed_effect = ManagedPlatformWrite::descriptor().expect("managed effect");
        let render_state_kind = StateKind::new(
            "mfm.framework.state",
            "render_public_outputs",
            DigestAlgorithm::Sha256JcsV1,
            DD,
        )
        .expect("render state kind");
        let render_state_version =
            StateVersion::new("mfm.framework.state.render_public_outputs.v1")
                .expect("render state version");
        let node_a_spec = node_spec(NodeSpecFixture {
            node_id: node_a.clone(),
            descriptor_id: descriptor_a.clone(),
            scope_id: scope.clone(),
            state_name: "mfm.test.state.a",
            state_kind: StateKind::new("mfm.test", "a", DigestAlgorithm::Sha256JcsV1, D2)
                .expect("state a"),
            state_version: StateVersion::new("mfm.test.state.a.v1").expect("state version"),
            effect_kind: effect_kind.clone(),
            config_ref: config_ref.clone(),
            input_schema: input_schema.clone(),
            input_cell: seed_cell.clone(),
            input_lineage: lineage_seed.clone(),
            output_cell: cell_a.clone(),
            output_schema: value_schema.clone(),
            semantic: semantic.clone(),
            caps: no_caps.clone(),
            predecessors: Vec::new(),
            adapter_bindings: Vec::new(),
            planning: planning.clone(),
        });
        let node_b_spec = node_spec(NodeSpecFixture {
            node_id: node_b.clone(),
            descriptor_id: descriptor_b.clone(),
            scope_id: scope.clone(),
            state_name: "mfm.test.state.b",
            state_kind: StateKind::new("mfm.test", "b", DigestAlgorithm::Sha256JcsV1, D3)
                .expect("state b"),
            state_version: StateVersion::new("mfm.test.state.b.v1").expect("state version"),
            effect_kind: read_effect.clone(),
            config_ref: config_ref.clone(),
            input_schema: input_schema.clone(),
            input_cell: cell_a.clone(),
            input_lineage: lineage_a.clone(),
            output_cell: cell_b.clone(),
            output_schema: value_schema.clone(),
            semantic: semantic.clone(),
            caps: read_caps.clone(),
            predecessors: vec![node_a.clone()],
            adapter_bindings: vec![spec::AdapterBinding {
                adapter_kind: adapter_kind.clone(),
                adapter_version: adapter_version.clone(),
                binding_digest: None,
            }],
            planning: planning.clone(),
        });
        let render_node_spec = spec::NodeSpec {
            node_id: render_node.clone(),
            stable_key: spec::StableAuthorKey::new("public-output").expect("render key"),
            scope_id: scope.clone(),
            state_kind: render_state_kind.clone(),
            state_version: render_state_version.clone(),
            descriptor_id: render_descriptor.clone(),
            config_ref: render_config_ref.clone(),
            input_bindings: render_input_binding,
            output_cell: render_cell.clone(),
            effect_kind: managed_effect.kind.clone(),
            capability_bindings: no_caps.clone(),
            adapter_bindings: Vec::new(),
            side_effect: None,
            framework: Some(spec::FrameworkNodeSpec::PublicOutputRender(
                spec::PublicOutputRenderNodeSpec {
                    public_schema_id: public_schema.clone(),
                    output_spec_digest: output_spec_digest.clone(),
                    renderer_descriptor: renderer.clone(),
                    required_cells: public_outputs.outputs.clone(),
                },
            )),
            planning_lineage: planning.clone(),
            deterministic_predecessors: vec![node_b.clone()],
        };
        let mut spec = spec::TypedExecutionSpec::new(spec::TypedExecutionSpecParts {
            authoring: spec::AuthoringProvenance::StateComposition {
                descriptor: spec::CompositionDescriptor {
                    descriptor_id: DescriptorId::from_digest(DigestAlgorithm::Sha256JcsV1, D4),
                    name: "mfm.test.composition".to_owned(),
                    version: "mfm.test.composition.v1".to_owned(),
                },
                config_hash: content(0x60),
            },
            scopes: vec![spec::ScopeSpec {
                scope_id: scope.clone(),
                parent_scope_id: None,
                stable_key: spec::StableAuthorKey::new("root").expect("stable key"),
                planning_lineage: planning.clone(),
            }],
            seeds: vec![spec::SeedSpec {
                seed_id: seed_id.clone(),
                seed_key: spec::StableAuthorKey::new("launch").expect("seed key"),
                cell_id: seed_cell.clone(),
                scope_id: scope.clone(),
                semantic_type_id: semantic.clone(),
                schema_id: value_schema.clone(),
                required_digest: Some(content(0x51)),
            }],
            descriptor_identities: vec![
                spec::DescriptorIdentity::State(Box::new(state_descriptor(
                    &node_a_spec,
                    descriptor_a.clone(),
                    "mfm.test.state.a",
                    effect_kind,
                    no_caps.clone(),
                    "pure",
                ))),
                spec::DescriptorIdentity::State(Box::new(state_descriptor(
                    &node_b_spec,
                    descriptor_b.clone(),
                    "mfm.test.state.b",
                    read_effect,
                    read_caps,
                    "read",
                ))),
                spec::DescriptorIdentity::State(Box::new(spec::StateDescriptorIdentity {
                    descriptor_id: render_descriptor.clone(),
                    name: "mfm.framework.render_public_outputs".to_owned(),
                    state_kind: render_state_kind,
                    state_version: render_state_version,
                    config_schema_id: render_config_ref.schema_id.clone(),
                    input_schema_id: public_schema.clone(),
                    output_schema_id: receipt_schema.clone(),
                    output_semantic_type_id: receipt_semantic.clone(),
                    effect_kind: managed_effect.kind,
                    effect_class: managed_effect.class.as_str().to_owned(),
                    effect_name: managed_effect.name.to_owned(),
                    effect_version: managed_effect.version,
                    capabilities: no_caps,
                    runner: "managed_platform_write".to_owned(),
                    side_effect_contract_digest: None,
                })),
                spec::DescriptorIdentity::Renderer(Box::new(renderer.clone())),
            ],
            config_refs: vec![config_ref.clone(), render_config_ref],
            nodes: vec![
                render_node_spec.clone(),
                node_b_spec.clone(),
                node_a_spec.clone(),
            ],
            cells: vec![
                spec::CellSpec {
                    cell_id: seed_cell,
                    producer: spec::CellProducer::Seed(seed_id),
                    scope_id: scope.clone(),
                    semantic_type_id: semantic.clone(),
                    schema_id: value_schema.clone(),
                    value_lineage: lineage_seed.clone(),
                    terminal_policy: spec::CellTerminalPolicy::ProducedOnly,
                    storage_policy: spec::StoragePolicy::ContentAddressed,
                    redaction_policy: spec::RedactionPolicy::Public,
                },
                spec::CellSpec {
                    cell_id: cell_a.clone(),
                    producer: spec::CellProducer::Node(node_a.clone()),
                    scope_id: scope.clone(),
                    semantic_type_id: semantic.clone(),
                    schema_id: value_schema.clone(),
                    value_lineage: lineage_a.clone(),
                    terminal_policy: spec::CellTerminalPolicy::ProducedOnly,
                    storage_policy: spec::StoragePolicy::ContentAddressed,
                    redaction_policy: spec::RedactionPolicy::Public,
                },
                spec::CellSpec {
                    cell_id: cell_b.clone(),
                    producer: spec::CellProducer::Node(node_b.clone()),
                    scope_id: scope.clone(),
                    semantic_type_id: semantic.clone(),
                    schema_id: value_schema.clone(),
                    value_lineage: lineage_b.clone(),
                    terminal_policy: spec::CellTerminalPolicy::ProducedOnly,
                    storage_policy: spec::StoragePolicy::ContentAddressed,
                    redaction_policy: spec::RedactionPolicy::Public,
                },
                spec::CellSpec {
                    cell_id: render_cell.clone(),
                    producer: spec::CellProducer::Node(render_node.clone()),
                    scope_id: scope.clone(),
                    semantic_type_id: receipt_semantic,
                    schema_id: receipt_schema,
                    value_lineage: render_lineage.clone(),
                    terminal_policy: spec::CellTerminalPolicy::ProducedOnly,
                    storage_policy: spec::StoragePolicy::PublicOutputArtifact,
                    redaction_policy: spec::RedactionPolicy::Public,
                },
            ],
            value_lineages: vec![
                spec::ValueLineage {
                    lineage_ref: lineage_seed.clone(),
                    scope_id: scope.clone(),
                    producer: spec::CellProducer::Seed(SeedId::from_digest(
                        DigestAlgorithm::Sha256JcsV1,
                        D1,
                    )),
                    input_cells: Vec::new(),
                    config_ref_digest: None,
                    planning_lineage: planning.clone(),
                    domain_keys: Vec::new(),
                    transform_policy: spec::LineageTransformPolicy::Source,
                },
                spec::ValueLineage {
                    lineage_ref: lineage_a.clone(),
                    scope_id: scope.clone(),
                    producer: spec::CellProducer::Node(node_a.clone()),
                    input_cells: vec![CellId::from_digest(DigestAlgorithm::Sha256JcsV1, D2)],
                    config_ref_digest: Some(config_ref.digest.clone()),
                    planning_lineage: planning.clone(),
                    domain_keys: Vec::new(),
                    transform_policy: spec::LineageTransformPolicy::StateOutput,
                },
                spec::ValueLineage {
                    lineage_ref: lineage_b.clone(),
                    scope_id: scope.clone(),
                    producer: spec::CellProducer::Node(node_b.clone()),
                    input_cells: vec![cell_a.clone()],
                    config_ref_digest: Some(config_ref.digest.clone()),
                    planning_lineage: planning.clone(),
                    domain_keys: Vec::new(),
                    transform_policy: spec::LineageTransformPolicy::StateOutput,
                },
                spec::ValueLineage {
                    lineage_ref: render_lineage,
                    scope_id: scope,
                    producer: spec::CellProducer::Node(render_node.clone()),
                    input_cells: vec![cell_b.clone()],
                    config_ref_digest: Some(config_ref.digest.clone()),
                    planning_lineage: planning,
                    domain_keys: Vec::new(),
                    transform_policy: spec::LineageTransformPolicy::StateOutput,
                },
            ],
            planning_lineage: Vec::new(),
            public_outputs,
        })
        .expect("typed spec");
        append_runtime_retention_lifecycle_node(&mut spec, render_cell.clone(), true);
        let retention_receipt = runtime_retention_receipt_cell(&spec);
        append_runtime_complete_lifecycle_node(&mut spec, retention_receipt, true);
        let envelope =
            spec::HashedSpecEnvelope::new(spec, spec::TypedExecutionSpecAudit::default())
                .expect("envelope");
        let runtime_spec =
            CertifiedRuntimeSpec::from_verified_envelope(envelope).expect("runtime spec");
        Fixture {
            runtime_spec,
            run_id: RunId::from_digest(DigestAlgorithm::Sha256JcsV1, D5),
            seed_ref,
            descriptor_a,
            descriptor_b,
            render_node,
            render_cell,
            cell_a,
            cell_b,
            cap_kind,
            cap_version,
            adapter_kind,
            adapter_version,
        }
    }

    fn fixture_with_retention_lifecycle_node() -> Fixture {
        fixture()
    }

    async fn drive_until_public_output_produced(
        scheduler: &SerialTypedScheduler,
        store: &mut store::InMemoryTypedRunStore,
        fixture: &Fixture,
    ) {
        for _ in 0..8 {
            scheduler
                .drive_once(store, &fixture.runtime_spec, &fixture.run_id)
                .await
                .expect("drive until public output");
            let projections = store.projection_snapshot();
            if projections.run_state(&fixture.run_id) == store::RunState::Started
                && matches!(
                    projections.public_output(
                        &fixture.runtime_spec.spec().public_outputs.public_schema_id
                    ),
                    Some(store::PublicOutputProjection::Produced { .. })
                )
            {
                return;
            }
        }
        panic!("public output was not produced");
    }

    fn fixture_with_first_managed_write_state() -> Fixture {
        let mut fixture = fixture();
        let mut envelope = fixture.runtime_spec.envelope().clone();
        let managed_effect = EffectKind::new(
            "mfm.test",
            "managed-write",
            DigestAlgorithm::Sha256JcsV1,
            D8,
        )
        .expect("managed effect");
        let managed_cap = CapabilityDescriptor::new(
            CapabilityKind::new(
                "mfm.test",
                "managed-store",
                DigestAlgorithm::Sha256JcsV1,
                D9,
            )
            .expect("managed cap kind"),
            CapabilityVersion::new("mfm.cap.managed_store.v1").expect("managed cap version"),
            CapabilityRole::ManagedPlatformWrite,
            "managed-store",
        )
        .expect("managed cap");
        let managed_caps = CapabilitySetDescriptor::new(vec![managed_cap]).expect("managed caps");
        for node in &mut envelope.spec.nodes {
            if node.descriptor_id == fixture.descriptor_a {
                node.effect_kind = managed_effect.clone();
                node.capability_bindings = managed_caps.clone();
            }
        }
        for descriptor in &mut envelope.spec.descriptor_identities {
            if let spec::DescriptorIdentity::State(identity) = descriptor {
                if identity.descriptor_id == fixture.descriptor_a {
                    identity.effect_kind = managed_effect.clone();
                    identity.effect_class = "managed-write".to_owned();
                    identity.effect_name = "managed-write".to_owned();
                    identity.capabilities = managed_caps.clone();
                    identity.runner = "managed-write".to_owned();
                }
            }
        }
        let envelope =
            spec::HashedSpecEnvelope::new(envelope.spec, envelope.audit).expect("rehash");
        fixture.runtime_spec =
            CertifiedRuntimeSpec::from_verified_envelope(envelope).expect("runtime spec");
        fixture
    }

    fn fixture_with_first_side_effect_state() -> Fixture {
        let mut fixture = fixture();
        let mut envelope = fixture.runtime_spec.envelope().clone();
        let side_effect =
            EffectKind::new("mfm.test", "side-effect", DigestAlgorithm::Sha256JcsV1, D8)
                .expect("side-effect");
        let side_effect_cap = CapabilityDescriptor::new(
            side_effect_capability_kind(),
            side_effect_capability_version(),
            CapabilityRole::ExternalMutationAuthority,
            "external-mutation",
        )
        .expect("side-effect cap");
        let side_effect_caps =
            CapabilitySetDescriptor::new(vec![side_effect_cap]).expect("side-effect caps");
        let contract_digest = content(0x88);
        for node in &mut envelope.spec.nodes {
            if node.descriptor_id == fixture.descriptor_a {
                node.effect_kind = side_effect.clone();
                node.capability_bindings = side_effect_caps.clone();
                node.adapter_bindings = vec![spec::AdapterBinding {
                    adapter_kind: fixture.adapter_kind.clone(),
                    adapter_version: fixture.adapter_version.clone(),
                    binding_digest: None,
                }];
                node.side_effect = Some(spec::SideEffectContractSpec {
                    contract_digest: contract_digest.clone(),
                });
            }
        }
        for descriptor in &mut envelope.spec.descriptor_identities {
            if let spec::DescriptorIdentity::State(identity) = descriptor {
                if identity.descriptor_id == fixture.descriptor_a {
                    identity.effect_kind = side_effect.clone();
                    identity.effect_class = "sidefx".to_owned();
                    identity.effect_name = "sidefx".to_owned();
                    identity.capabilities = side_effect_caps.clone();
                    identity.runner = "sidefx".to_owned();
                    identity.side_effect_contract_digest = Some(contract_digest.clone());
                }
            }
        }
        let envelope =
            spec::HashedSpecEnvelope::new(envelope.spec, envelope.audit).expect("rehash");
        fixture.runtime_spec =
            CertifiedRuntimeSpec::from_verified_envelope(envelope).expect("runtime spec");
        fixture
    }

    fn fixture_with_independent_second_node_and_first_side_effect_state() -> Fixture {
        let mut fixture = fixture_with_first_side_effect_state();
        let mut envelope = fixture.runtime_spec.envelope().clone();
        let seed_cell = envelope
            .spec
            .cells
            .iter()
            .find(|cell| cell.cell_id == fixture.seed_ref.cell_id)
            .expect("seed cell")
            .clone();
        let node_b_id = envelope
            .spec
            .nodes
            .iter()
            .find(|node| node.descriptor_id == fixture.descriptor_b)
            .expect("node b")
            .node_id
            .clone();
        let cell_a = envelope
            .spec
            .cells
            .iter()
            .find(|cell| cell.cell_id == fixture.cell_a)
            .expect("cell a")
            .clone();
        let cell_a_public_output = spec::PublicOutputCell {
            public_field_path: spec::PublicFieldPath::new("side_effect").expect("field"),
            cell_id: cell_a.cell_id.clone(),
            producer: cell_a.producer.clone(),
            scope_id: cell_a.scope_id.clone(),
            semantic_type_id: cell_a.semantic_type_id.clone(),
            schema_id: cell_a.schema_id.clone(),
            value_lineage: cell_a.value_lineage.clone(),
            required_terminal: spec::RequiredTerminal::ProducedOnly,
        };
        envelope
            .spec
            .public_outputs
            .outputs
            .push(cell_a_public_output);
        let public_output_cells = envelope.spec.public_outputs.outputs.clone();
        let output_spec_digest = envelope
            .spec
            .public_outputs
            .digest()
            .expect("public output digest");
        let render_input_root = spec::InputBindingNodeSpec::Struct(
            public_output_cells
                .iter()
                .map(|public_output| spec::NamedInputBindingSpec {
                    field_path: public_output.public_field_path.clone(),
                    node: spec::InputBindingNodeSpec::Cell(Box::new(spec::InputBindingCellSpec {
                        field_path: public_output.public_field_path.clone(),
                        cell_id: public_output.cell_id.clone(),
                        semantic_type_id: public_output.semantic_type_id.clone(),
                        schema_id: public_output.schema_id.clone(),
                        required_terminal: public_output.required_terminal,
                        value_lineage: public_output.value_lineage.clone(),
                    })),
                })
                .collect(),
        );
        let render_predecessors = runtime_predecessors_for_inputs(
            &envelope.spec,
            &public_output_cells
                .iter()
                .map(|public_output| public_output.cell_id.clone())
                .collect::<Vec<_>>(),
        );
        for node in &mut envelope.spec.nodes {
            if node.descriptor_id == fixture.descriptor_b {
                node.input_bindings.root =
                    spec::InputBindingNodeSpec::Cell(Box::new(spec::InputBindingCellSpec {
                        field_path: spec::PublicFieldPath::new("input").expect("field"),
                        cell_id: seed_cell.cell_id.clone(),
                        semantic_type_id: seed_cell.semantic_type_id.clone(),
                        schema_id: seed_cell.schema_id.clone(),
                        required_terminal: spec::RequiredTerminal::ProducedOnly,
                        value_lineage: seed_cell.value_lineage.clone(),
                    }));
                node.deterministic_predecessors.clear();
            }
            if node.node_id == fixture.render_node {
                if let Some(spec::FrameworkNodeSpec::PublicOutputRender(render)) =
                    &mut node.framework
                {
                    render.output_spec_digest = output_spec_digest.clone();
                    render.required_cells = public_output_cells.clone();
                }
                node.input_bindings.root = render_input_root.clone();
                node.input_bindings.digest =
                    content_digest_json(input_node_json(&node.input_bindings.root))
                        .expect("render input digest");
                node.deterministic_predecessors = render_predecessors.clone();
            }
        }
        for lineage in &mut envelope.spec.value_lineages {
            if lineage.producer == spec::CellProducer::Node(node_b_id.clone()) {
                lineage.input_cells = vec![seed_cell.cell_id.clone()];
            }
            if lineage.producer == spec::CellProducer::Node(fixture.render_node.clone()) {
                lineage.input_cells = public_output_cells
                    .iter()
                    .map(|public_output| public_output.cell_id.clone())
                    .collect();
            }
        }
        let envelope =
            spec::HashedSpecEnvelope::new(envelope.spec, envelope.audit).expect("rehash");
        fixture.runtime_spec =
            CertifiedRuntimeSpec::from_verified_envelope(envelope).expect("runtime spec");
        fixture
    }

    struct NodeSpecFixture {
        node_id: NodeId,
        descriptor_id: DescriptorId,
        scope_id: ScopeId,
        state_name: &'static str,
        state_kind: StateKind,
        state_version: StateVersion,
        effect_kind: EffectKind,
        config_ref: spec::ConfigRef,
        input_schema: SchemaId,
        input_cell: CellId,
        input_lineage: spec::ValueLineageRef,
        output_cell: CellId,
        output_schema: SchemaId,
        semantic: SemanticTypeId,
        caps: CapabilitySetDescriptor,
        predecessors: Vec<NodeId>,
        adapter_bindings: Vec<spec::AdapterBinding>,
        planning: spec::PlanningLineage,
    }

    fn node_spec(fixture: NodeSpecFixture) -> spec::NodeSpec {
        spec::NodeSpec {
            node_id: fixture.node_id,
            stable_key: spec::StableAuthorKey::new(
                fixture.state_name.rsplit('.').next().expect("state key"),
            )
            .expect("stable key"),
            scope_id: fixture.scope_id,
            state_kind: fixture.state_kind,
            state_version: fixture.state_version,
            descriptor_id: fixture.descriptor_id,
            config_ref: fixture.config_ref,
            input_bindings: spec::InputBindingSpec {
                input_schema_id: fixture.input_schema,
                input_descriptor_id: DescriptorId::from_digest(DigestAlgorithm::Sha256JcsV1, D6),
                root: spec::InputBindingNodeSpec::Cell(Box::new(spec::InputBindingCellSpec {
                    field_path: spec::PublicFieldPath::new("input").expect("field"),
                    cell_id: fixture.input_cell,
                    semantic_type_id: fixture.semantic.clone(),
                    schema_id: fixture.output_schema.clone(),
                    required_terminal: spec::RequiredTerminal::ProducedOnly,
                    value_lineage: fixture.input_lineage,
                })),
                digest: content(0x73),
            },
            output_cell: fixture.output_cell,
            effect_kind: fixture.effect_kind,
            capability_bindings: fixture.caps,
            adapter_bindings: fixture.adapter_bindings,
            side_effect: None,
            framework: None,
            planning_lineage: fixture.planning,
            deterministic_predecessors: fixture.predecessors,
        }
    }

    fn state_descriptor(
        node: &spec::NodeSpec,
        descriptor_id: DescriptorId,
        name: &str,
        effect_kind: EffectKind,
        capabilities: CapabilitySetDescriptor,
        runner: &str,
    ) -> spec::StateDescriptorIdentity {
        spec::StateDescriptorIdentity {
            descriptor_id,
            name: name.to_owned(),
            state_kind: node.state_kind.clone(),
            state_version: node.state_version.clone(),
            config_schema_id: node.config_ref.schema_id.clone(),
            input_schema_id: node.input_bindings.input_schema_id.clone(),
            output_schema_id: node
                .input_bindings
                .root
                .clone()
                .first_schema_or(node.config_ref.schema_id.clone()),
            output_semantic_type_id: match &node.input_bindings.root {
                spec::InputBindingNodeSpec::Cell(cell) => cell.semantic_type_id.clone(),
                _ => panic!("test input"),
            },
            effect_kind,
            effect_class: runner.to_owned(),
            effect_name: runner.to_owned(),
            effect_version: EffectVersion::new("mfm.effect.v1").expect("effect version"),
            capabilities,
            runner: runner.to_owned(),
            side_effect_contract_digest: None,
        }
    }

    trait FirstSchema {
        fn first_schema_or(&self, fallback: SchemaId) -> SchemaId;
    }

    impl FirstSchema for spec::InputBindingNodeSpec {
        fn first_schema_or(&self, fallback: SchemaId) -> SchemaId {
            match self {
                spec::InputBindingNodeSpec::Cell(cell) => cell.schema_id.clone(),
                _ => fallback,
            }
        }
    }

    fn content(byte: u8) -> ContentDigest {
        ContentDigest::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([byte; 32]),
        )
    }

    fn input_node_json(node: &spec::InputBindingNodeSpec) -> serde_json::Value {
        match node {
            spec::InputBindingNodeSpec::Unit => serde_json::json!({ "kind": "unit" }),
            spec::InputBindingNodeSpec::Cell(cell) => serde_json::json!({
                "cell_id": cell.cell_id.as_str(),
                "field_path": cell.field_path.as_str(),
                "kind": "cell",
                "required_terminal": match cell.required_terminal {
                    spec::RequiredTerminal::ProducedOnly => "produced_only",
                    spec::RequiredTerminal::MaybeSkipped => "maybe_skipped",
                },
                "schema_id": cell.schema_id.as_str(),
                "semantic_type_id": cell.semantic_type_id.as_str(),
                "value_lineage": cell.value_lineage.lineage_digest.as_str(),
            }),
            spec::InputBindingNodeSpec::Tuple(elements) => serde_json::json!({
                "elements": elements.iter().map(input_node_json).collect::<Vec<_>>(),
                "kind": "tuple",
            }),
            spec::InputBindingNodeSpec::Struct(fields) => serde_json::json!({
                "fields": fields.iter().map(|field| {
                    serde_json::json!({
                        "field_path": field.field_path.as_str(),
                        "node": input_node_json(&field.node),
                    })
                }).collect::<Vec<_>>(),
                "kind": "struct",
            }),
            spec::InputBindingNodeSpec::Vec {
                elements,
                ordering,
                domain_keys,
            } => serde_json::json!({
                "domain_keys": domain_keys.iter().map(stable_domain_key_ref_json).collect::<Vec<_>>(),
                "elements": elements.iter().map(input_node_json).collect::<Vec<_>>(),
                "kind": "vec",
                "ordering": ordering_json(*ordering),
            }),
            spec::InputBindingNodeSpec::NonEmptyVec {
                elements,
                ordering,
                domain_keys,
            } => serde_json::json!({
                "domain_keys": domain_keys.iter().map(stable_domain_key_ref_json).collect::<Vec<_>>(),
                "elements": elements.iter().map(input_node_json).collect::<Vec<_>>(),
                "kind": "non_empty_vec",
                "ordering": ordering_json(*ordering),
            }),
        }
    }

    fn ordering_json(ordering: spec::OrderingEvidence) -> &'static str {
        match ordering {
            spec::OrderingEvidence::ExplicitAuthorOrder => "explicit_author_order",
            spec::OrderingEvidence::StableDomainKey => "stable_domain_key",
        }
    }

    fn stable_domain_key_ref_json(key: &spec::StableDomainKeyRef) -> serde_json::Value {
        serde_json::json!({
            "content_digest": key.content_digest.as_str(),
            "schema_id": key.schema_id.as_str(),
        })
    }

    fn artifact(byte: u8) -> ArtifactId {
        ArtifactId::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([byte; 32]),
        )
    }

    fn side_effect_capability_kind() -> CapabilityKind {
        CapabilityKind::new(
            "mfm.test",
            "external-mutation",
            DigestAlgorithm::Sha256JcsV1,
            D9,
        )
        .expect("side-effect cap kind")
    }

    fn side_effect_capability_version() -> CapabilityVersion {
        CapabilityVersion::new("mfm.cap.external_mutation.v1").expect("side-effect cap version")
    }

    fn side_effect_ledger_key(attempt_no: u32) -> events::SideEffectLedgerKey {
        events::SideEffectLedgerKey::new(format!("ledger-{attempt_no}")).expect("ledger key")
    }

    fn side_effect_claim_owner(attempt_no: u32, generation: u32) -> events::RunnerInvocationId {
        events::RunnerInvocationId::new(format!("owner-{attempt_no}-{generation}"))
            .expect("claim owner")
    }

    fn side_effect_fencing_token(
        attempt_no: u32,
        generation: u32,
    ) -> events::side_effect::ClaimFencingToken {
        events::side_effect::ClaimFencingToken::new(format!("token-{attempt_no}-{generation}"))
            .expect("fencing token")
    }

    fn side_effect_artifact(
        ctx: &ErasedRunCtx<'_>,
        artifact_id: ArtifactId,
        digest: ContentDigest,
        role: events::ArtifactRole,
    ) -> store::ArtifactEvidenceRef {
        store::ArtifactEvidenceRef {
            artifact_id,
            digest,
            byte_len: 19,
            media_type: spec::MediaType::new("application/json").expect("media"),
            schema_id: Some(ctx.node().config_ref.schema_id.clone()),
            semantic_type_id: None,
            producer_node_id: Some(ctx.node().node_id.clone()),
            producer_seed_id: None,
            artifact_role: role,
        }
    }

    fn side_effect_claimed(
        ctx: &ErasedRunCtx<'_>,
        ledger: events::SideEffectLedgerKey,
        invocation_epoch: u32,
        claim_generation: u32,
    ) -> events::KernelEventPayload {
        events::KernelEventPayload::SideEffectClaimed(events::side_effect::Claimed {
            spec_hash: ctx.spec_hash().clone(),
            node_id: ctx.node().node_id.clone(),
            attempt_id: ctx.attempt_id().clone(),
            ledger_key: ledger,
            claim_owner: side_effect_claim_owner(ctx.attempt_no(), claim_generation),
            invocation_epoch,
            claim_generation,
            claim_fencing_token: side_effect_fencing_token(ctx.attempt_no(), claim_generation),
        })
    }

    fn side_effect_claim_taken_over(
        ctx: &ErasedRunCtx<'_>,
        ledger: events::SideEffectLedgerKey,
        previous: &store::SideEffectClaimProjection,
        claim_generation: u32,
    ) -> events::KernelEventPayload {
        events::KernelEventPayload::SideEffectClaimTakenOver(events::side_effect::ClaimTakenOver {
            spec_hash: ctx.spec_hash().clone(),
            node_id: ctx.node().node_id.clone(),
            attempt_id: ctx.attempt_id().clone(),
            ledger_key: ledger,
            previous_claim_owner: previous.claim_owner.clone(),
            new_claim_owner: side_effect_claim_owner(ctx.attempt_no(), claim_generation),
            invocation_epoch: previous.invocation_epoch,
            previous_claim_generation: previous.claim_generation,
            claim_generation,
            claim_fencing_token: side_effect_fencing_token(ctx.attempt_no(), claim_generation),
        })
    }

    fn side_effect_prepared(
        ctx: &ErasedRunCtx<'_>,
        ledger: events::SideEffectLedgerKey,
        invocation_epoch: u32,
        claim_generation: u32,
    ) -> events::KernelEventPayload {
        events::KernelEventPayload::SideEffectInvocationPrepared(
            events::side_effect::InvocationPrepared {
                spec_hash: ctx.spec_hash().clone(),
                node_id: ctx.node().node_id.clone(),
                attempt_id: ctx.attempt_id().clone(),
                ledger_key: ledger,
                invocation_epoch,
                claim_generation,
                claim_fencing_token: side_effect_fencing_token(ctx.attempt_no(), claim_generation),
                prepared_artifact_id: None,
                prepared_hash: None,
            },
        )
    }

    fn side_effect_invocation_started(
        ctx: &ErasedRunCtx<'_>,
        ledger: events::SideEffectLedgerKey,
        invocation_epoch: u32,
        claim_generation: u32,
    ) -> events::KernelEventPayload {
        events::KernelEventPayload::SideEffectInvocationStarted(
            events::side_effect::InvocationStarted {
                spec_hash: ctx.spec_hash().clone(),
                node_id: ctx.node().node_id.clone(),
                attempt_id: ctx.attempt_id().clone(),
                ledger_key: ledger,
                invocation_epoch,
                claim_owner: side_effect_claim_owner(ctx.attempt_no(), claim_generation),
                claim_generation,
                claim_fencing_token: side_effect_fencing_token(ctx.attempt_no(), claim_generation),
            },
        )
    }

    fn side_effect_submission_observed(
        ctx: &ErasedRunCtx<'_>,
        ledger: events::SideEffectLedgerKey,
        invocation_epoch: u32,
        artifact_id: ArtifactId,
        digest: ContentDigest,
    ) -> events::KernelEventPayload {
        events::KernelEventPayload::SideEffectSubmissionObserved(
            events::side_effect::SubmissionObserved {
                spec_hash: ctx.spec_hash().clone(),
                node_id: ctx.node().node_id.clone(),
                attempt_id: ctx.attempt_id().clone(),
                ledger_key: ledger,
                invocation_epoch,
                submission_schema_id: ctx.node().config_ref.schema_id.clone(),
                submission_hash: digest,
                submission_artifact_id: artifact_id,
            },
        )
    }

    fn side_effect_receipt_observed(
        ctx: &ErasedRunCtx<'_>,
        ledger: events::SideEffectLedgerKey,
        invocation_epoch: u32,
        artifact_id: ArtifactId,
        digest: ContentDigest,
    ) -> events::KernelEventPayload {
        events::KernelEventPayload::SideEffectReceiptObserved(
            events::side_effect::ReceiptObserved {
                spec_hash: ctx.spec_hash().clone(),
                node_id: ctx.node().node_id.clone(),
                attempt_id: ctx.attempt_id().clone(),
                ledger_key: ledger,
                invocation_epoch,
                receipt_schema_id: ctx.node().config_ref.schema_id.clone(),
                receipt_hash: digest,
                receipt_artifact_id: artifact_id,
                replay_verifier_id: events::ReplayVerifierId::new("verifier-1").expect("verifier"),
            },
        )
    }

    fn side_effect_confirmation_observed(
        ctx: &ErasedRunCtx<'_>,
        ledger: events::SideEffectLedgerKey,
        invocation_epoch: u32,
        artifact_id: ArtifactId,
        digest: ContentDigest,
    ) -> events::KernelEventPayload {
        events::KernelEventPayload::SideEffectConfirmationObserved(
            events::side_effect::ConfirmationObserved {
                spec_hash: ctx.spec_hash().clone(),
                node_id: ctx.node().node_id.clone(),
                attempt_id: ctx.attempt_id().clone(),
                ledger_key: ledger,
                invocation_epoch,
                confirmation_schema_id: ctx.node().config_ref.schema_id.clone(),
                confirmation_hash: digest,
                confirmation_artifact_id: artifact_id,
                replay_verifier_id: events::ReplayVerifierId::new("verifier-1").expect("verifier"),
            },
        )
    }

    fn side_effect_error(retryable: bool) -> events::MfmErrorInfo {
        events::MfmErrorInfo {
            code: events::ErrorCode::new("sidefx_failed").expect("error code"),
            category: events::ErrorCategory::SideEffect,
            retryable,
            safe_message: "side-effect failed".to_owned(),
            public_details: None,
            diagnostic_ref: None,
        }
    }

    fn public_output_error() -> events::MfmErrorInfo {
        events::MfmErrorInfo {
            code: events::ErrorCode::new("public_output_render_failed").expect("error code"),
            category: events::ErrorCategory::Runtime,
            retryable: true,
            safe_message: "public output render failed".to_owned(),
            public_details: None,
            diagnostic_ref: None,
        }
    }
}
