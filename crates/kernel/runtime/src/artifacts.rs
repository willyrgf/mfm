use mfm_canonical::sha256_digest_bytes;
use mfm_events::v1 as events;
use mfm_ids::{ArtifactId, AttemptId, ContentDigest, DigestAlgorithm, NodeId, RunId};
use mfm_store::v1 as store;

use crate::{ErasedRunCtx, PreInvocationRunCtx, Result, RuntimeError};

/// Runtime artifact capability used by the scheduler.
pub trait RuntimeArtifactStore: store::RetainedArtifactReadProvider {}

impl<T> RuntimeArtifactStore for T where T: store::RetainedArtifactReadProvider {}

/// Runtime-owned artifact binding kind for one staged attempt artifact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StagedArtifactBindingKind {
    /// Artifact is the terminal output for a state cell.
    StateOutput,
    /// Artifact is an external read fact response.
    FactResponse,
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

impl StagedSideEffectArtifactPhase {
    fn staging_class(self) -> events::ArtifactStagingClass {
        match self {
            Self::Intent => events::ArtifactStagingClass::SideEffectIntent,
            Self::PreparedInvocation => events::ArtifactStagingClass::SideEffectPreparedInvocation,
            Self::NotSubmittedProof => events::ArtifactStagingClass::SideEffectNotSubmittedProof,
            Self::Submission => events::ArtifactStagingClass::SideEffectSubmission,
            Self::SubmissionUnknownEvidence => {
                events::ArtifactStagingClass::SideEffectSubmissionUnknown
            }
            Self::Receipt => events::ArtifactStagingClass::SideEffectReceipt,
            Self::Confirmation => events::ArtifactStagingClass::SideEffectConfirmation,
            Self::AmbiguityEvidence => events::ArtifactStagingClass::SideEffectAmbiguity,
        }
    }
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
        validate_staged_artifact_producer(ctx, &evidence)?;
        Ok(Self {
            run_id: ctx.run_id().clone(),
            node_id: ctx.node().node_id.clone(),
            attempt_id: ctx.attempt_id().clone(),
            binding,
            evidence,
        })
    }

    fn for_pre_invocation(
        ctx: &PreInvocationRunCtx<'_>,
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
        validate_staged_artifact_producer_for_node(&ctx.node().node_id, &evidence)?;
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

fn validate_staged_artifact_producer(
    ctx: &ErasedRunCtx<'_>,
    evidence: &store::ArtifactEvidenceRef,
) -> Result<()> {
    validate_staged_artifact_producer_for_node(&ctx.node().node_id, evidence)
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

    /// Creates an inline staged side-effect artifact from pre-invocation lane preflight.
    pub fn inline_pre_invocation_side_effect_artifact(
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
    match role.contract().staging {
        events::ArtifactStagingClass::AttemptStateOutput => {
            Some(StagedArtifactBindingKind::StateOutput)
        }
        events::ArtifactStagingClass::AttemptFactResponse => {
            Some(StagedArtifactBindingKind::FactResponse)
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
        | events::ArtifactStagingClass::AttemptFactQueryEvidence
        | events::ArtifactStagingClass::ManualResolution
        | events::ArtifactStagingClass::AttemptPublicOutput
        | events::ArtifactStagingClass::AttemptRedactedDiagnostic
        | events::ArtifactStagingClass::MiddlewareRetentionManifest => None,
    }
}

fn artifact_role_for_staging_class(staging: events::ArtifactStagingClass) -> events::ArtifactRole {
    events::ArtifactRole::ALL
        .iter()
        .copied()
        .find(|role| role.contract().staging == staging)
        .expect("artifact role contract for runtime staging class")
}

fn staged_artifact_binding_staging_class(
    binding: &StagedArtifactBindingKind,
) -> events::ArtifactStagingClass {
    match binding {
        StagedArtifactBindingKind::StateOutput => events::ArtifactStagingClass::AttemptStateOutput,
        StagedArtifactBindingKind::FactResponse => {
            events::ArtifactStagingClass::AttemptFactResponse
        }
        StagedArtifactBindingKind::FactQueryEvidence => {
            events::ArtifactStagingClass::AttemptFactQueryEvidence
        }
        StagedArtifactBindingKind::SideEffectEvidence { phase, .. } => phase.staging_class(),
        StagedArtifactBindingKind::PublicOutput => {
            events::ArtifactStagingClass::AttemptPublicOutput
        }
        StagedArtifactBindingKind::RedactedDiagnostic => {
            events::ArtifactStagingClass::AttemptRedactedDiagnostic
        }
        StagedArtifactBindingKind::RetentionManifest => {
            events::ArtifactStagingClass::MiddlewareRetentionManifest
        }
    }
}

pub(crate) fn staged_artifact_binding_role(
    binding: &StagedArtifactBindingKind,
) -> events::ArtifactRole {
    artifact_role_for_staging_class(staged_artifact_binding_staging_class(binding))
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

pub(crate) fn validate_fact_query_returned_ref_authority(
    projections: &store::ProjectionSnapshot,
    fact_ref: &mfm_facts::InternalFactRef,
) -> Result<()> {
    let descriptor = projections
        .fact_descriptor(fact_ref.fact_descriptor_hash())
        .ok_or_else(|| fact_query_ref_authority_error("missing descriptor authority"))?;
    if !fact_descriptor_projection_matches_returned_ref(descriptor, fact_ref) {
        return Err(fact_query_ref_authority_error(
            "descriptor authority does not match returned ref",
        ));
    }

    let record = projections
        .fact_record(fact_ref.fact_claim_id())
        .ok_or_else(|| fact_query_ref_authority_error("missing source fact authority"))?;
    if !fact_record_projection_matches_returned_ref(record, fact_ref) {
        return Err(fact_query_ref_authority_error(
            "source fact authority does not match returned ref",
        ));
    }

    let index = projections
        .fact_index_entry(fact_ref.fact_claim_id())
        .ok_or_else(|| fact_query_ref_authority_error("missing indexed fact authority"))?;
    if !fact_index_projection_matches_returned_ref(index, fact_ref) {
        return Err(fact_query_ref_authority_error(
            "indexed fact authority does not match returned ref",
        ));
    }

    Ok(())
}

fn fact_descriptor_projection_matches_returned_ref(
    descriptor: &store::FactDescriptorProjection,
    fact_ref: &mfm_facts::InternalFactRef,
) -> bool {
    &descriptor.descriptor_hash == fact_ref.fact_descriptor_hash()
        && &descriptor.fact_kind == fact_ref.fact_kind()
        && &descriptor.response_schema_id == fact_ref.response_schema_id()
        && &descriptor.fact_subject_namespace_hash == fact_ref.fact_subject_namespace_hash()
}

fn fact_record_projection_matches_returned_ref(
    record: &store::FactRecordProjection,
    fact_ref: &mfm_facts::InternalFactRef,
) -> bool {
    let claim = &record.claim;
    let request_schema_id = claim.request().map(|request| request.request_schema_id());
    let request_hash = claim.request().map(|request| request.request_hash());
    let subject_material_hash = claim.subject().subject_material().content_digest();
    &record.fact_claim_id == fact_ref.fact_claim_id()
        && &record.source_event_id == fact_ref.source_event_id()
        && &record.source_run_id == fact_ref.fact_claim_id().source_run_id()
        && record.source_seq == fact_ref.fact_claim_id().source_seq()
        && record.source_ordinal == fact_ref.fact_claim_id().source_ordinal()
        && claim.visibility() == fact_ref.visibility()
        && claim.fact_kind() == fact_ref.fact_kind()
        && claim.fact_descriptor_hash() == fact_ref.fact_descriptor_hash()
        && claim.observed_at() == fact_ref.observed_at()
        && claim.subject().fact_subject_namespace_hash() == fact_ref.fact_subject_namespace_hash()
        && claim.subject().subject_material_hash() == fact_ref.subject_material_hash()
        && &subject_material_hash == fact_ref.subject_material_hash()
        && claim.subject().fact_key() == fact_ref.fact_key()
        && request_schema_id == fact_ref.request_schema_id()
        && request_hash == fact_ref.request_hash()
        && claim.response().response_schema_id() == fact_ref.response_schema_id()
        && claim.response().response_hash() == fact_ref.response_hash()
        && claim.response().artifact_id() == fact_ref.artifact_id()
        && claim.response().artifact_evidence_hash() == fact_ref.artifact_evidence_hash()
        && claim.producer().capability_kind() == fact_ref.capability_kind()
        && claim.producer().capability_version() == fact_ref.capability_version()
        && claim.producer().adapter_kind() == fact_ref.adapter_kind()
        && claim.producer().adapter_version() == fact_ref.adapter_version()
}

fn fact_index_projection_matches_returned_ref(
    index: &store::FactIndexProjection,
    fact_ref: &mfm_facts::InternalFactRef,
) -> bool {
    let index_visibility = mfm_facts::FactVisibility::indexed_default(index.audience);
    &index.fact_claim_id == fact_ref.fact_claim_id()
        && &index.source_run_id == fact_ref.fact_claim_id().source_run_id()
        && index.source_seq == fact_ref.fact_claim_id().source_seq()
        && index.source_ordinal == fact_ref.fact_claim_id().source_ordinal()
        && &index.source_event_id == fact_ref.source_event_id()
        && index.recorded_at == fact_ref.recorded_at()
        && index.observed_at.as_deref() == fact_ref.observed_at()
        && &index_visibility == fact_ref.visibility()
        && index.visibility_scope == mfm_facts::FactVisibilityScope::Default
        && &index.fact_kind == fact_ref.fact_kind()
        && &index.fact_descriptor_hash == fact_ref.fact_descriptor_hash()
        && &index.fact_subject_namespace_hash == fact_ref.fact_subject_namespace_hash()
        && &index.fact_key == fact_ref.fact_key()
        && &index.subject_material_hash == fact_ref.subject_material_hash()
        && index.request_schema_id.as_ref() == fact_ref.request_schema_id()
        && index.request_hash.as_ref() == fact_ref.request_hash()
        && &index.response_schema_id == fact_ref.response_schema_id()
        && &index.response_hash == fact_ref.response_hash()
        && &index.artifact_id == fact_ref.artifact_id()
        && &index.artifact_evidence_hash == fact_ref.artifact_evidence_hash()
        && &index.capability_kind == fact_ref.capability_kind()
        && &index.capability_version == fact_ref.capability_version()
        && &index.adapter_kind == fact_ref.adapter_kind()
        && &index.adapter_version == fact_ref.adapter_version()
}

fn fact_query_ref_authority_error(message: &'static str) -> RuntimeError {
    RuntimeError::InvalidRunnerOutput(format!("fact query evidence returned ref {message}"))
}
