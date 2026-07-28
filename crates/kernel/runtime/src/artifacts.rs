use mfm_canonical::sha256_digest_bytes;
use mfm_events::v1 as events;
use mfm_ids::{ArtifactId, AttemptId, ContentDigest, DigestAlgorithm, NodeId, RunId};
use mfm_store::v1 as store;

use crate::{ErasedRunCtx, PreInvocationRunCtx, Result, RuntimeError};

/// Runtime-owned artifact binding kind for one staged attempt artifact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum StagedArtifactBindingKind {
    /// Artifact is the terminal output for a state cell.
    StateOutput,
    /// Artifact is an external read fact response.
    FactResponse,
    /// Artifact is retained request/response evidence for an external capability read.
    ExternalReadEvidence,
    /// Artifact is private replay evidence for a fact query.
    FactQueryEvidence,
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
pub(crate) enum StagedSideEffectArtifactPhase {
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
pub(crate) struct StagedArtifactHandle {
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
        Self::for_context(
            ctx.run_id(),
            &ctx.node().node_id,
            ctx.attempt_id(),
            evidence,
            binding,
        )
    }

    fn for_pre_invocation(
        ctx: &PreInvocationRunCtx<'_>,
        evidence: store::ArtifactEvidenceRef,
        binding: StagedArtifactBindingKind,
    ) -> Result<Self> {
        Self::for_context(
            ctx.run_id(),
            &ctx.node().node_id,
            ctx.attempt_id(),
            evidence,
            binding,
        )
    }

    fn for_context(
        run_id: &RunId,
        node_id: &NodeId,
        attempt_id: &AttemptId,
        evidence: store::ArtifactEvidenceRef,
        binding: StagedArtifactBindingKind,
    ) -> Result<Self> {
        if staged_artifact_binding_role(&binding) != evidence.artifact_role {
            return Err(RuntimeError::InvalidRunnerOutput(format!(
                "node {} staged artifact role {} with mismatched binding",
                node_id,
                artifact_role_name(evidence.artifact_role)
            )));
        }
        validate_staged_artifact_producer_for_node(node_id, &evidence)?;
        Ok(Self {
            run_id: run_id.clone(),
            node_id: node_id.clone(),
            attempt_id: attempt_id.clone(),
            binding,
            evidence,
        })
    }

    /// Returns the run id this handle is sealed to.
    pub(crate) fn run_id(&self) -> &RunId {
        &self.run_id
    }

    /// Returns the node id this handle is sealed to.
    pub(crate) fn node_id(&self) -> &NodeId {
        &self.node_id
    }

    /// Returns the attempt id this handle is sealed to.
    pub(crate) fn attempt_id(&self) -> &AttemptId {
        &self.attempt_id
    }

    /// Returns the artifact binding metadata.
    pub(crate) fn binding(&self) -> &StagedArtifactBindingKind {
        &self.binding
    }

    /// Returns the finalized artifact evidence.
    pub(crate) fn evidence(&self) -> &store::ArtifactEvidenceRef {
        &self.evidence
    }
}

fn validate_staged_artifact_producer_for_node(
    node_id: &NodeId,
    evidence: &store::ArtifactEvidenceRef,
) -> Result<()> {
    match evidence.artifact_role.contract().producer {
        events::ArtifactProducerScope::NodeRequired => {
            if evidence.producer_node_id.as_ref() == Some(node_id)
                && evidence.producer_seed_id.is_none()
            {
                Ok(())
            } else {
                Err(RuntimeError::InvalidRunnerOutput(format!(
                    "node {node_id} staged artifact {} with producer evidence outside its attempt",
                    evidence.artifact_id
                )))
            }
        }
        events::ArtifactProducerScope::DiagnosticOptionalNodeNoSeed => {
            if evidence.producer_seed_id.is_some()
                || evidence
                    .producer_node_id
                    .as_ref()
                    .is_some_and(|producer_node_id| producer_node_id != node_id)
            {
                Err(RuntimeError::InvalidRunnerOutput(format!(
                    "node {node_id} staged artifact {} with producer evidence outside its attempt",
                    evidence.artifact_id
                )))
            } else {
                Ok(())
            }
        }
        events::ArtifactProducerScope::GlobalNoSeed
        | events::ArtifactProducerScope::MiddlewareNoSeed
        | events::ArtifactProducerScope::LaunchOrGlobalNoSeed => {
            if evidence.producer_node_id.is_none() && evidence.producer_seed_id.is_none() {
                Ok(())
            } else {
                Err(RuntimeError::InvalidRunnerOutput(format!(
                    "node {node_id} staged artifact {} with producer evidence outside its attempt",
                    evidence.artifact_id
                )))
            }
        }
        events::ArtifactProducerScope::SeedRequired => {
            Err(RuntimeError::InvalidRunnerOutput(format!(
                "node {node_id} staged artifact {} with producer evidence outside its attempt",
                evidence.artifact_id
            )))
        }
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
    pub(crate) fn inline_side_effect_artifact(
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

    /// Creates an inline staged side-effect artifact from pre-invocation lane preflight.
    pub(crate) fn inline_pre_invocation_side_effect_artifact(
        ctx: &PreInvocationRunCtx<'_>,
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
        let binding = StagedArtifactBindingKind::SideEffectEvidence {
            ledger_key,
            invocation_epoch,
            phase,
        };
        verify_artifact_bytes(&bytes, &evidence)?;
        Ok(Self {
            handle: StagedArtifactHandle::for_pre_invocation(ctx, evidence, binding)?,
            bytes: Some(bytes),
        })
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

    /// Returns the sealed handle.
    pub(crate) fn handle(&self) -> &StagedArtifactHandle {
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
    match role.contract().staging {
        events::ArtifactStagingClass::AttemptStateOutput => {
            Some(StagedArtifactBindingKind::StateOutput)
        }
        events::ArtifactStagingClass::AttemptFactResponse => {
            Some(StagedArtifactBindingKind::FactResponse)
        }
        events::ArtifactStagingClass::AttemptExternalReadEvidence => {
            Some(StagedArtifactBindingKind::ExternalReadEvidence)
        }
        events::ArtifactStagingClass::AttemptFactQueryEvidence => {
            Some(StagedArtifactBindingKind::FactQueryEvidence)
        }
        events::ArtifactStagingClass::AttemptPublicOutput => {
            Some(StagedArtifactBindingKind::PublicOutput)
        }
        events::ArtifactStagingClass::AttemptRedactedDiagnostic => {
            Some(StagedArtifactBindingKind::RedactedDiagnostic)
        }
        events::ArtifactStagingClass::RunAdmission
        | events::ArtifactStagingClass::SideEffectIntent
        | events::ArtifactStagingClass::SideEffectPreparedInvocation
        | events::ArtifactStagingClass::SideEffectNotSubmittedProof
        | events::ArtifactStagingClass::SideEffectSubmission
        | events::ArtifactStagingClass::SideEffectSubmissionUnknown
        | events::ArtifactStagingClass::SideEffectReceipt
        | events::ArtifactStagingClass::SideEffectConfirmation
        | events::ArtifactStagingClass::SideEffectAmbiguity
        | events::ArtifactStagingClass::ManualResolution
        | events::ArtifactStagingClass::MiddlewareRetentionManifest => None,
    }
}

pub(crate) fn staged_side_effect_artifact_phase(
    role: events::ArtifactRole,
) -> Option<StagedSideEffectArtifactPhase> {
    match role.contract().staging {
        events::ArtifactStagingClass::SideEffectIntent => {
            Some(StagedSideEffectArtifactPhase::Intent)
        }
        events::ArtifactStagingClass::SideEffectPreparedInvocation => {
            Some(StagedSideEffectArtifactPhase::PreparedInvocation)
        }
        events::ArtifactStagingClass::SideEffectNotSubmittedProof => {
            Some(StagedSideEffectArtifactPhase::NotSubmittedProof)
        }
        events::ArtifactStagingClass::SideEffectSubmission => {
            Some(StagedSideEffectArtifactPhase::Submission)
        }
        events::ArtifactStagingClass::SideEffectSubmissionUnknown => {
            Some(StagedSideEffectArtifactPhase::SubmissionUnknownEvidence)
        }
        events::ArtifactStagingClass::SideEffectReceipt => {
            Some(StagedSideEffectArtifactPhase::Receipt)
        }
        events::ArtifactStagingClass::SideEffectConfirmation => {
            Some(StagedSideEffectArtifactPhase::Confirmation)
        }
        events::ArtifactStagingClass::SideEffectAmbiguity => {
            Some(StagedSideEffectArtifactPhase::AmbiguityEvidence)
        }
        events::ArtifactStagingClass::RunAdmission
        | events::ArtifactStagingClass::AttemptStateOutput
        | events::ArtifactStagingClass::AttemptFactResponse
        | events::ArtifactStagingClass::AttemptExternalReadEvidence
        | events::ArtifactStagingClass::AttemptFactQueryEvidence
        | events::ArtifactStagingClass::ManualResolution
        | events::ArtifactStagingClass::AttemptPublicOutput
        | events::ArtifactStagingClass::AttemptRedactedDiagnostic
        | events::ArtifactStagingClass::MiddlewareRetentionManifest => None,
    }
}

pub(crate) fn staged_artifact_binding_role(
    binding: &StagedArtifactBindingKind,
) -> events::ArtifactRole {
    match binding {
        StagedArtifactBindingKind::StateOutput => events::ArtifactRole::StateOutput,
        StagedArtifactBindingKind::FactResponse => events::ArtifactRole::FactResponse,
        StagedArtifactBindingKind::ExternalReadEvidence => {
            events::ArtifactRole::ExternalReadEvidence
        }
        StagedArtifactBindingKind::FactQueryEvidence => events::ArtifactRole::FactQueryEvidence,
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
    role.as_str()
}

/// Retention refs staged by a runner before the scheduler binds them to typed events.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StagedRetentionRefs {
    pub(crate) refs: Vec<events::RetentionRef>,
    pub(crate) reason: events::RetentionReason,
    pub(crate) authority: StagedRetentionRefAuthority,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum StagedRetentionRefAuthority {
    CurrentCommitArtifacts,
    FactQueryEvidence {
        returned_refs: Vec<mfm_facts::InternalFactRef>,
    },
}

impl StagedRetentionRefs {
    /// Stages runtime-evidence retention refs for the current runner attempt.
    pub fn runtime_evidence(refs: Vec<events::RetentionRef>) -> Self {
        Self {
            refs,
            reason: events::RetentionReason::RuntimeEvidence,
            authority: StagedRetentionRefAuthority::CurrentCommitArtifacts,
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
            authority: StagedRetentionRefAuthority::CurrentCommitArtifacts,
        }
    }

    pub(crate) fn fact_query_evidence(
        refs: Vec<events::RetentionRef>,
        returned_refs: Vec<mfm_facts::InternalFactRef>,
    ) -> Self {
        Self {
            refs,
            reason: events::RetentionReason::RuntimeEvidence,
            authority: StagedRetentionRefAuthority::FactQueryEvidence { returned_refs },
        }
    }
}

fn validate_fact_query_returned_ref_authority(
    lifecycle: &store::current_lifecycle::CurrentLifecycleReader<'_>,
    fact_ref: &mfm_facts::InternalFactRef,
) -> Result<()> {
    let descriptor = lifecycle
        .fact_descriptor(fact_ref.fact_descriptor_hash())
        .ok_or_else(|| fact_query_ref_authority_error("missing descriptor authority"))?;
    if !fact_descriptor_matches_returned_ref(&descriptor, fact_ref) {
        return Err(fact_query_ref_authority_error(
            "descriptor authority does not match returned ref",
        ));
    }

    let query = lifecycle
        .fact_query_entry(fact_ref.fact_claim_id())
        .ok_or_else(|| fact_query_ref_authority_error("missing fact query authority"))?;
    if query.internal_ref().ok().as_ref() != Some(fact_ref) {
        return Err(fact_query_ref_authority_error(
            "fact query authority does not match returned ref",
        ));
    }

    Ok(())
}

pub(crate) fn fact_query_returned_ref_retention_refs(
    lifecycle: &store::current_lifecycle::CurrentLifecycleReader<'_>,
    fact_ref: &mfm_facts::InternalFactRef,
) -> Result<[events::RetentionRef; 2]> {
    validate_fact_query_returned_ref_authority(lifecycle, fact_ref)?;
    let descriptor = lifecycle
        .fact_descriptor(fact_ref.fact_descriptor_hash())
        .ok_or_else(|| fact_query_ref_authority_error("missing descriptor authority"))?;
    let query = lifecycle
        .fact_query_entry(fact_ref.fact_claim_id())
        .ok_or_else(|| fact_query_ref_authority_error("missing response artifact authority"))?;
    let response_evidence = query
        .response_artifact_evidence()
        .ok_or_else(|| fact_query_ref_authority_error("missing response artifact authority"))?;
    Ok([
        descriptor
            .artifact_evidence()
            .retention_ref()
            .map_err(|_| fact_query_ref_authority_error("invalid descriptor artifact authority"))?,
        response_evidence
            .retention_ref()
            .map_err(|_| fact_query_ref_authority_error("invalid response artifact authority"))?,
    ])
}

fn fact_descriptor_matches_returned_ref(
    descriptor: &store::current_lifecycle::CurrentFactDescriptorRef<'_>,
    fact_ref: &mfm_facts::InternalFactRef,
) -> bool {
    descriptor.descriptor_hash() == fact_ref.fact_descriptor_hash()
        && descriptor.fact_kind() == fact_ref.fact_kind()
        && descriptor.response_schema_id() == fact_ref.response_schema_id()
        && descriptor.subject_namespace_hash() == fact_ref.fact_subject_namespace_hash()
}

fn fact_query_ref_authority_error(message: &'static str) -> RuntimeError {
    RuntimeError::InvalidRunnerOutput(format!("fact query evidence returned ref {message}"))
}
