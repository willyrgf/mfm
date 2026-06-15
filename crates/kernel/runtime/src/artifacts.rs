use std::future::Future;
use std::pin::Pin;

use mfm_canonical::sha256_digest_bytes;
use mfm_events::v1 as events;
use mfm_ids::{ArtifactId, AttemptId, ContentDigest, DigestAlgorithm, NodeId, RunId};
use mfm_store::v1 as store;

use crate::{ErasedRunCtx, Result, RuntimeError};

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

    pub(crate) fn inline_retention_manifest_artifact(
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
    pub(crate) fn finalized_attempt_artifact_for_tests(
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

pub(crate) fn staged_artifact_binding_kind(
    role: events::ArtifactRole,
) -> Option<StagedArtifactBindingKind> {
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
        | events::ArtifactRole::ManualResolutionEvidence
        | events::ArtifactRole::ManualResolutionAuthorization
        | events::ArtifactRole::RetentionManifest => None,
    }
}

pub(crate) fn staged_side_effect_artifact_phase(
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
        | events::ArtifactRole::ManualResolutionEvidence
        | events::ArtifactRole::ManualResolutionAuthorization
        | events::ArtifactRole::RetentionManifest => None,
    }
}

pub(crate) fn staged_artifact_binding_role(
    binding: &StagedArtifactBindingKind,
) -> events::ArtifactRole {
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

pub(crate) fn verify_artifact_bytes(
    bytes: &[u8],
    evidence: &store::ArtifactEvidenceRef,
) -> Result<()> {
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

pub(crate) fn artifact_role_name(role: events::ArtifactRole) -> &'static str {
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
        events::ArtifactRole::ManualResolutionEvidence => "manual_resolution_evidence",
        events::ArtifactRole::ManualResolutionAuthorization => "manual_resolution_authorization",
        events::ArtifactRole::PublicOutput => "public_output",
        events::ArtifactRole::RedactedDiagnostic => "redacted_diagnostic",
        events::ArtifactRole::RetentionManifest => "retention_manifest",
    }
}

/// Retention refs staged by a runner before the scheduler binds them to typed events.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StagedRetentionRefs {
    pub(crate) refs: Vec<events::RetentionRef>,
    pub(crate) reason: events::RetentionReason,
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

    pub(crate) fn framework_public_output(refs: Vec<events::RetentionRef>) -> Self {
        Self {
            refs,
            reason: events::RetentionReason::PublicOutput,
        }
    }
}
