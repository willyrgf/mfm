use mfm_events::v1::{self as events, ArtifactRole, KernelEventPayload, RunCompletionOutcome};
use mfm_ids::{ArtifactId, ContentDigest, NodeId, SchemaId, SeedId, SemanticTypeId};
use mfm_spec::v1::{CellProducer, MediaType};

/// Source of an artifact reference carried by a kernel event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EventArtifactReferenceSource {
    /// Typed execution spec artifact from `RunAdmitted`.
    RunSpec,
    /// Typed spec certificate artifact from `RunAdmitted`.
    RunCertificate,
    /// Certified config artifact from `RunAdmitted`.
    RunConfig,
    /// Certified fact descriptor artifact from `RunAdmitted`.
    FactDescriptor,
    /// Seed-cell artifact reference from `RunAdmitted`.
    SeedCell,
    /// Read-fact response artifact.
    FactResponse,
    /// Explicit event artifact reference.
    ArtifactReferenced,
    /// State-output cell artifact.
    StateOutput,
    /// Public-output source cell artifact.
    PublicOutputCell,
    /// Rendered public-output artifact.
    PublicOutputRendered,
    /// Public-output render-failure diagnostic artifact reference.
    PublicOutputRenderFailureDiagnostic,
    /// State-attempt failure diagnostic artifact reference.
    StateAttemptFailureDiagnostic,
    /// Side-effect failure diagnostic artifact reference.
    SideEffectFailureDiagnostic,
    /// Manual-resolution evidence artifact.
    ManualResolutionEvidence,
    /// Manual-resolution authorization proof artifact.
    ManualResolutionAuthorization,
    /// Side-effect intent artifact.
    SideEffectIntent,
    /// Prepared side-effect invocation artifact.
    PreparedInvocation,
    /// Not-submitted proof artifact.
    NotSubmittedProof,
    /// Side-effect submission artifact.
    Submission,
    /// Side-effect submission-unknown evidence artifact.
    SubmissionUnknownEvidence,
    /// Side-effect receipt artifact.
    Receipt,
    /// Side-effect confirmation artifact.
    Confirmation,
    /// Side-effect resource touched-set evidence artifact.
    ResourceTouchedSet,
    /// Side-effect ambiguity evidence artifact.
    AmbiguityEvidence,
    /// Retention reference artifact.
    RetentionRef,
    /// Retention manifest artifact.
    RetentionManifest,
}

impl EventArtifactReferenceSource {
    /// Returns true when this source may be the framework terminal-lifecycle receipt.
    pub fn is_terminal_lifecycle_receipt_candidate(self) -> bool {
        matches!(self, Self::ArtifactReferenced | Self::StateOutput)
    }

    /// Returns true when this source belongs to retention-only metadata events.
    pub fn is_retention(self) -> bool {
        matches!(self, Self::RetentionRef | Self::RetentionManifest)
    }

    /// Returns true when this source is payload evidence that may authorize same-commit
    /// runtime retention refs.
    pub fn is_same_commit_payload_evidence(self) -> bool {
        matches!(
            self,
            Self::FactResponse
                | Self::StateOutput
                | Self::PublicOutputRendered
                | Self::PublicOutputRenderFailureDiagnostic
                | Self::StateAttemptFailureDiagnostic
                | Self::SideEffectFailureDiagnostic
                | Self::ManualResolutionEvidence
                | Self::ManualResolutionAuthorization
                | Self::SideEffectIntent
                | Self::PreparedInvocation
                | Self::NotSubmittedProof
                | Self::Submission
                | Self::SubmissionUnknownEvidence
                | Self::Receipt
                | Self::Confirmation
                | Self::AmbiguityEvidence
        )
    }
}

/// Artifact evidence requirement derived from a single kernel event payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventArtifactRequirement {
    /// Event field that referenced the artifact.
    pub source: EventArtifactReferenceSource,
    /// Referenced artifact id.
    pub artifact_id: ArtifactId,
    /// Expected exact retained-artifact evidence identity.
    pub evidence_hash: ContentDigest,
    /// Expected content digest, when the event carries one.
    pub digest: Option<ContentDigest>,
    /// Expected byte length, when the event carries one.
    pub byte_len: Option<u64>,
    /// Expected media type, when the event carries one.
    pub media_type: Option<MediaType>,
    /// Expected schema id, when the event carries one.
    pub schema_id: Option<SchemaId>,
    /// Expected semantic type id, when the event carries one.
    pub semantic_type_id: Option<SemanticTypeId>,
    /// Expected producer node id, when applicable.
    pub producer_node_id: Option<NodeId>,
    /// Expected producer seed id, when applicable.
    pub producer_seed_id: Option<SeedId>,
    /// Expected artifact role, when the event carries or implies one.
    pub artifact_role: Option<ArtifactRole>,
}

/// Derives every retained-artifact requirement carried by one kernel event payload.
pub fn event_artifact_requirements(payload: &KernelEventPayload) -> Vec<EventArtifactRequirement> {
    let mut requirements = Vec::new();
    match payload {
        KernelEventPayload::RunAdmitted(payload) => {
            push_run_artifact(
                &mut requirements,
                EventArtifactReferenceSource::RunSpec,
                &payload.spec_artifact,
            );
            push_run_artifact(
                &mut requirements,
                EventArtifactReferenceSource::RunCertificate,
                &payload.certificate_artifact,
            );
            for artifact in &payload.config_artifacts {
                push_run_artifact(
                    &mut requirements,
                    EventArtifactReferenceSource::RunConfig,
                    artifact,
                );
            }
            for artifact in &payload.fact_descriptor_artifacts {
                push_run_artifact(
                    &mut requirements,
                    EventArtifactReferenceSource::FactDescriptor,
                    artifact,
                );
            }
            for seed in &payload.seed_cells {
                push_event_artifact(
                    &mut requirements,
                    EventArtifactReferenceSource::SeedCell,
                    &seed.seed_artifact,
                    None,
                    Some(seed.seed_id.clone()),
                );
            }
        }
        KernelEventPayload::FactRecorded(payload) => {
            let response = payload.claim.response();
            requirements.push(EventArtifactRequirement {
                source: EventArtifactReferenceSource::FactResponse,
                artifact_id: response.artifact_id().clone(),
                evidence_hash: response.artifact_evidence_hash().clone(),
                digest: Some(response.response_hash().clone()),
                byte_len: None,
                media_type: None,
                schema_id: Some(response.response_schema_id().clone()),
                semantic_type_id: None,
                producer_node_id: Some(payload.node_id.clone()),
                producer_seed_id: None,
                artifact_role: Some(ArtifactRole::FactResponse),
            });
        }
        KernelEventPayload::ArtifactReferenced(payload) => {
            push_event_artifact(
                &mut requirements,
                EventArtifactReferenceSource::ArtifactReferenced,
                &payload.artifact_ref,
                payload.node_id.clone(),
                None,
            );
        }
        KernelEventPayload::CellProduced(payload) => {
            requirements.push(EventArtifactRequirement {
                source: EventArtifactReferenceSource::StateOutput,
                artifact_id: payload.artifact_id.clone(),
                evidence_hash: payload.evidence_hash.clone(),
                digest: Some(payload.content_digest.clone()),
                byte_len: None,
                media_type: None,
                schema_id: Some(payload.schema_id.clone()),
                semantic_type_id: Some(payload.semantic_type_id.clone()),
                producer_node_id: Some(payload.node_id.clone()),
                producer_seed_id: None,
                artifact_role: Some(ArtifactRole::StateOutput),
            });
        }
        KernelEventPayload::PublicOutputProduced(payload) => {
            for cell in &payload.cells {
                let (producer_node_id, producer_seed_id) =
                    cell_producer_artifact_owner(&cell.producer);
                requirements.push(EventArtifactRequirement {
                    source: EventArtifactReferenceSource::PublicOutputCell,
                    artifact_id: cell.artifact_id.clone(),
                    evidence_hash: cell.evidence_hash.clone(),
                    digest: Some(cell.content_digest.clone()),
                    byte_len: None,
                    media_type: None,
                    schema_id: Some(cell.schema_id.clone()),
                    semantic_type_id: Some(cell.semantic_type_id.clone()),
                    producer_node_id,
                    producer_seed_id,
                    artifact_role: None,
                });
            }
            if let (Some(artifact_id), Some(evidence_hash)) = (
                &payload.rendered_artifact_id,
                &payload.rendered_artifact_evidence_hash,
            ) {
                requirements.push(EventArtifactRequirement {
                    source: EventArtifactReferenceSource::PublicOutputRendered,
                    artifact_id: artifact_id.clone(),
                    evidence_hash: evidence_hash.clone(),
                    digest: Some(payload.rendered_digest.clone()),
                    byte_len: None,
                    media_type: None,
                    schema_id: Some(payload.public_schema_id.clone()),
                    semantic_type_id: None,
                    producer_node_id: Some(payload.node_id.clone()),
                    producer_seed_id: None,
                    artifact_role: Some(ArtifactRole::PublicOutput),
                });
            }
        }
        KernelEventPayload::PublicOutputRenderFailed(payload) => {
            if let Some(ref evidence) = payload.error.diagnostic_ref {
                push_event_artifact(
                    &mut requirements,
                    EventArtifactReferenceSource::PublicOutputRenderFailureDiagnostic,
                    evidence,
                    Some(payload.node_id.clone()),
                    None,
                );
            }
        }
        KernelEventPayload::StateAttemptFailed(payload) => {
            if let Some(ref evidence) = payload.error.diagnostic_ref {
                push_event_artifact(
                    &mut requirements,
                    EventArtifactReferenceSource::StateAttemptFailureDiagnostic,
                    evidence,
                    Some(payload.node_id.clone()),
                    None,
                );
            }
        }
        KernelEventPayload::ManualResolutionRecorded(payload) => {
            requirements.push(EventArtifactRequirement {
                source: EventArtifactReferenceSource::ManualResolutionEvidence,
                artifact_id: payload.evidence_artifact_id.clone(),
                evidence_hash: payload.evidence_artifact_evidence_hash.clone(),
                digest: Some(payload.evidence_hash.clone()),
                byte_len: None,
                media_type: None,
                schema_id: Some(payload.evidence_schema_id.clone()),
                semantic_type_id: None,
                producer_node_id: None,
                producer_seed_id: None,
                artifact_role: Some(ArtifactRole::ManualResolutionEvidence),
            });
            requirements.push(EventArtifactRequirement {
                source: EventArtifactReferenceSource::ManualResolutionAuthorization,
                artifact_id: payload.authorization_artifact_id.clone(),
                evidence_hash: payload.authorization_artifact_evidence_hash.clone(),
                digest: Some(payload.authorization_hash.clone()),
                byte_len: None,
                media_type: None,
                schema_id: Some(payload.authorization_schema_id.clone()),
                semantic_type_id: None,
                producer_node_id: None,
                producer_seed_id: None,
                artifact_role: Some(ArtifactRole::ManualResolutionAuthorization),
            });
        }
        KernelEventPayload::RunCompleted(payload) => match &payload.outcome {
            RunCompletionOutcome::Completed(_) => {}
            RunCompletionOutcome::Compensated
            | RunCompletionOutcome::ManuallyResolved
            | RunCompletionOutcome::FailedWithoutAcdcClaim => {}
        },
        KernelEventPayload::SideEffectIntentPersisted(payload) => {
            requirements.push(EventArtifactRequirement {
                source: EventArtifactReferenceSource::SideEffectIntent,
                artifact_id: payload.intent_artifact_id.clone(),
                evidence_hash: payload.intent_artifact_evidence_hash.clone(),
                digest: Some(payload.intent_hash.clone()),
                byte_len: None,
                media_type: None,
                schema_id: Some(payload.intent_schema_id.clone()),
                semantic_type_id: None,
                producer_node_id: Some(payload.node_id.clone()),
                producer_seed_id: None,
                artifact_role: Some(ArtifactRole::SideEffectIntent),
            });
        }
        KernelEventPayload::SideEffectInvocationPrepared(payload) => {
            requirements.push(EventArtifactRequirement {
                source: EventArtifactReferenceSource::PreparedInvocation,
                artifact_id: payload.prepared_artifact_id.clone(),
                evidence_hash: payload.prepared_artifact_evidence_hash.clone(),
                digest: Some(payload.prepared_hash.clone()),
                byte_len: None,
                media_type: None,
                schema_id: Some(payload.prepared_schema_id.clone()),
                semantic_type_id: None,
                producer_node_id: Some(payload.node_id.clone()),
                producer_seed_id: None,
                artifact_role: Some(ArtifactRole::PreparedInvocation),
            });
        }
        KernelEventPayload::SideEffectNotSubmittedProven(payload) => {
            requirements.push(EventArtifactRequirement {
                source: EventArtifactReferenceSource::NotSubmittedProof,
                artifact_id: payload.proof_artifact_id.clone(),
                evidence_hash: payload.proof_artifact_evidence_hash.clone(),
                digest: Some(payload.proof_hash.clone()),
                byte_len: None,
                media_type: None,
                schema_id: Some(payload.proof_schema_id.clone()),
                semantic_type_id: None,
                producer_node_id: Some(payload.node_id.clone()),
                producer_seed_id: None,
                artifact_role: Some(ArtifactRole::NotSubmittedProof),
            });
        }
        KernelEventPayload::SideEffectSubmissionObserved(payload) => {
            requirements.push(EventArtifactRequirement {
                source: EventArtifactReferenceSource::Submission,
                artifact_id: payload.submission_artifact_id.clone(),
                evidence_hash: payload.submission_artifact_evidence_hash.clone(),
                digest: Some(payload.submission_hash.clone()),
                byte_len: None,
                media_type: None,
                schema_id: Some(payload.submission_schema_id.clone()),
                semantic_type_id: None,
                producer_node_id: Some(payload.node_id.clone()),
                producer_seed_id: None,
                artifact_role: Some(ArtifactRole::Submission),
            });
        }
        KernelEventPayload::SideEffectSubmissionUnknown(payload) => {
            requirements.push(EventArtifactRequirement {
                source: EventArtifactReferenceSource::SubmissionUnknownEvidence,
                artifact_id: payload.evidence_artifact_id.clone(),
                evidence_hash: payload.evidence_artifact_evidence_hash.clone(),
                digest: Some(payload.evidence_hash.clone()),
                byte_len: None,
                media_type: None,
                schema_id: Some(payload.evidence_schema_id.clone()),
                semantic_type_id: None,
                producer_node_id: Some(payload.node_id.clone()),
                producer_seed_id: None,
                artifact_role: Some(ArtifactRole::SubmissionUnknownEvidence),
            });
        }
        KernelEventPayload::SideEffectReceiptObserved(payload) => {
            requirements.push(EventArtifactRequirement {
                source: EventArtifactReferenceSource::Receipt,
                artifact_id: payload.receipt_artifact_id.clone(),
                evidence_hash: payload.receipt_artifact_evidence_hash.clone(),
                digest: Some(payload.receipt_hash.clone()),
                byte_len: None,
                media_type: None,
                schema_id: Some(payload.receipt_schema_id.clone()),
                semantic_type_id: None,
                producer_node_id: Some(payload.node_id.clone()),
                producer_seed_id: None,
                artifact_role: Some(ArtifactRole::Receipt),
            });
            if let Some(touched_set) = &payload.resource_touched_set {
                push_resource_touched_set_requirement(&mut requirements, touched_set);
            }
        }
        KernelEventPayload::SideEffectConfirmationObserved(payload) => {
            requirements.push(EventArtifactRequirement {
                source: EventArtifactReferenceSource::Confirmation,
                artifact_id: payload.confirmation_artifact_id.clone(),
                evidence_hash: payload.confirmation_artifact_evidence_hash.clone(),
                digest: Some(payload.confirmation_hash.clone()),
                byte_len: None,
                media_type: None,
                schema_id: Some(payload.confirmation_schema_id.clone()),
                semantic_type_id: None,
                producer_node_id: Some(payload.node_id.clone()),
                producer_seed_id: None,
                artifact_role: Some(ArtifactRole::Confirmation),
            });
            if let Some(touched_set) = &payload.resource_touched_set {
                push_resource_touched_set_requirement(&mut requirements, touched_set);
            }
        }
        KernelEventPayload::SideEffectAmbiguous(payload) => {
            requirements.push(EventArtifactRequirement {
                source: EventArtifactReferenceSource::AmbiguityEvidence,
                artifact_id: payload.evidence_artifact_id.clone(),
                evidence_hash: payload.evidence_artifact_evidence_hash.clone(),
                digest: Some(payload.evidence_hash.clone()),
                byte_len: None,
                media_type: None,
                schema_id: Some(payload.evidence_schema_id.clone()),
                semantic_type_id: None,
                producer_node_id: Some(payload.node_id.clone()),
                producer_seed_id: None,
                artifact_role: Some(ArtifactRole::AmbiguityEvidence),
            });
        }
        KernelEventPayload::SideEffectFailed(payload) => {
            if let Some(ref evidence) = payload.error.diagnostic_ref {
                push_event_artifact(
                    &mut requirements,
                    EventArtifactReferenceSource::SideEffectFailureDiagnostic,
                    evidence,
                    Some(payload.node_id.clone()),
                    None,
                );
            }
        }
        KernelEventPayload::RetentionRefsAppended(payload) => {
            for retention_ref in &payload.refs {
                requirements.push(EventArtifactRequirement {
                    source: EventArtifactReferenceSource::RetentionRef,
                    artifact_id: retention_ref.artifact_id.clone(),
                    evidence_hash: retention_ref.evidence_hash.clone(),
                    digest: Some(retention_ref.content_digest.clone()),
                    byte_len: None,
                    media_type: None,
                    schema_id: None,
                    semantic_type_id: None,
                    producer_node_id: None,
                    producer_seed_id: None,
                    artifact_role: Some(retention_ref.role),
                });
            }
        }
        KernelEventPayload::RetentionManifestProjected(payload) => {
            requirements.push(EventArtifactRequirement {
                source: EventArtifactReferenceSource::RetentionManifest,
                artifact_id: payload.manifest_artifact_id.clone(),
                evidence_hash: payload.manifest_artifact_evidence_hash.clone(),
                digest: Some(payload.manifest_digest.clone()),
                byte_len: None,
                media_type: None,
                schema_id: None,
                semantic_type_id: None,
                producer_node_id: None,
                producer_seed_id: None,
                artifact_role: Some(ArtifactRole::RetentionManifest),
            });
        }
        KernelEventPayload::StateAttemptStarted(_)
        | KernelEventPayload::CellSkipped(_)
        | KernelEventPayload::SideEffectClaimed(_)
        | KernelEventPayload::SideEffectClaimTakenOver(_)
        | KernelEventPayload::ResourceLaneClaimed(_)
        | KernelEventPayload::ResourceLaneClaimIntent(_)
        | KernelEventPayload::SideEffectInvocationStarted(_)
        | KernelEventPayload::ResourceLaneReleased(_)
        | KernelEventPayload::ResourceLaneReleaseIntent(_)
        | KernelEventPayload::StateAttemptCompleted(_)
        | KernelEventPayload::StateAttemptInterrupted(_) => {}
    }
    requirements
}

fn push_run_artifact(
    requirements: &mut Vec<EventArtifactRequirement>,
    source: EventArtifactReferenceSource,
    evidence: &events::RunArtifactEvidenceRef,
) {
    requirements.push(EventArtifactRequirement {
        source,
        artifact_id: evidence.artifact_id.clone(),
        evidence_hash: evidence.evidence_hash.clone(),
        digest: Some(evidence.content_digest.clone()),
        byte_len: Some(evidence.byte_len),
        media_type: Some(evidence.media_type.clone()),
        schema_id: evidence.schema_id.clone(),
        semantic_type_id: evidence.semantic_type_id.clone(),
        producer_node_id: None,
        producer_seed_id: None,
        artifact_role: Some(evidence.role),
    });
}

fn push_event_artifact(
    requirements: &mut Vec<EventArtifactRequirement>,
    source: EventArtifactReferenceSource,
    evidence: &events::ArtifactEvidenceRef,
    producer_node_id: Option<NodeId>,
    producer_seed_id: Option<SeedId>,
) {
    requirements.push(EventArtifactRequirement {
        source,
        artifact_id: evidence.artifact_id.clone(),
        evidence_hash: evidence.evidence_hash.clone(),
        digest: Some(evidence.content_digest.clone()),
        byte_len: Some(evidence.byte_len),
        media_type: Some(evidence.media_type.clone()),
        schema_id: Some(evidence.schema_id.clone()),
        semantic_type_id: evidence.semantic_type_id.clone(),
        producer_node_id,
        producer_seed_id,
        artifact_role: Some(evidence.role),
    });
}

fn push_resource_touched_set_requirement(
    requirements: &mut Vec<EventArtifactRequirement>,
    evidence: &events::ResourceTouchedSetEvidence,
) {
    requirements.push(EventArtifactRequirement {
        source: EventArtifactReferenceSource::ResourceTouchedSet,
        artifact_id: evidence.evidence_artifact_id.clone(),
        evidence_hash: evidence.evidence_artifact_evidence_hash.clone(),
        digest: Some(evidence.evidence_hash.clone()),
        byte_len: None,
        media_type: None,
        schema_id: Some(evidence.evidence_schema_id.clone()),
        semantic_type_id: None,
        producer_node_id: None,
        producer_seed_id: None,
        artifact_role: None,
    });
}

fn cell_producer_artifact_owner(producer: &CellProducer) -> (Option<NodeId>, Option<SeedId>) {
    match producer {
        CellProducer::Node(node_id) => (Some(node_id.clone()), None),
        CellProducer::Seed(seed_id) => (None, Some(seed_id.clone())),
    }
}

/// Builds an exact requirement for a run-admission artifact reference.
pub fn run_artifact_requirement(
    source: EventArtifactReferenceSource,
    expected: &events::RunArtifactEvidenceRef,
    role: ArtifactRole,
) -> EventArtifactRequirement {
    EventArtifactRequirement {
        source,
        artifact_id: expected.artifact_id.clone(),
        evidence_hash: expected.evidence_hash.clone(),
        digest: Some(expected.content_digest.clone()),
        byte_len: Some(expected.byte_len),
        media_type: Some(expected.media_type.clone()),
        schema_id: expected.schema_id.clone(),
        semantic_type_id: expected.semantic_type_id.clone(),
        producer_node_id: None,
        producer_seed_id: None,
        artifact_role: Some(role),
    }
}

/// Builds an exact requirement for an explicit artifact-reference event.
pub fn artifact_referenced_artifact_requirement(
    payload: &events::ArtifactReferenced,
) -> EventArtifactRequirement {
    EventArtifactRequirement {
        source: EventArtifactReferenceSource::ArtifactReferenced,
        artifact_id: payload.artifact_ref.artifact_id.clone(),
        evidence_hash: payload.artifact_ref.evidence_hash.clone(),
        digest: Some(payload.artifact_ref.content_digest.clone()),
        byte_len: Some(payload.artifact_ref.byte_len),
        media_type: Some(payload.artifact_ref.media_type.clone()),
        schema_id: Some(payload.artifact_ref.schema_id.clone()),
        semantic_type_id: payload.artifact_ref.semantic_type_id.clone(),
        producer_node_id: payload.node_id.clone(),
        producer_seed_id: None,
        artifact_role: Some(payload.artifact_ref.role),
    }
}

/// Builds an exact requirement for a certified config reference.
pub fn config_ref_artifact_requirement(
    config_ref: &mfm_spec::v1::ConfigRef,
) -> super::Result<EventArtifactRequirement> {
    let evidence = super::ArtifactEvidenceRef {
        artifact_id: config_ref.artifact_id.clone(),
        digest: config_ref.digest.clone(),
        byte_len: config_ref.byte_len,
        media_type: config_ref.media_type.clone(),
        schema_id: Some(config_ref.schema_id.clone()),
        semantic_type_id: None,
        producer_node_id: None,
        producer_seed_id: None,
        artifact_role: ArtifactRole::TypedConfig,
    };
    Ok(EventArtifactRequirement {
        source: EventArtifactReferenceSource::RunConfig,
        artifact_id: config_ref.artifact_id.clone(),
        evidence_hash: evidence.evidence_hash()?,
        digest: Some(config_ref.digest.clone()),
        byte_len: Some(config_ref.byte_len),
        media_type: Some(config_ref.media_type.clone()),
        schema_id: Some(config_ref.schema_id.clone()),
        semantic_type_id: None,
        producer_node_id: None,
        producer_seed_id: None,
        artifact_role: Some(ArtifactRole::TypedConfig),
    })
}

/// Builds an exact requirement for authored seed bytes and their staged evidence.
pub fn seed_artifact_requirement(
    seed_spec: &mfm_spec::v1::SeedSpec,
    evidence: &super::ArtifactEvidenceRef,
) -> super::Result<EventArtifactRequirement> {
    Ok(EventArtifactRequirement {
        source: EventArtifactReferenceSource::SeedCell,
        artifact_id: evidence.artifact_id.clone(),
        evidence_hash: evidence.evidence_hash()?,
        digest: Some(evidence.digest.clone()),
        byte_len: Some(evidence.byte_len),
        media_type: Some(evidence.media_type.clone()),
        schema_id: Some(seed_spec.schema_id.clone()),
        semantic_type_id: Some(seed_spec.semantic_type_id.clone()),
        producer_node_id: None,
        producer_seed_id: Some(seed_spec.seed_id.clone()),
        artifact_role: Some(ArtifactRole::SeedInput),
    })
}

/// Builds an exact requirement for a seed cell recorded at run admission.
pub fn seed_cell_artifact_requirement(cell: &events::SeedCellRef) -> EventArtifactRequirement {
    EventArtifactRequirement {
        source: EventArtifactReferenceSource::SeedCell,
        artifact_id: cell.seed_artifact.artifact_id.clone(),
        evidence_hash: cell.seed_artifact.evidence_hash.clone(),
        digest: Some(cell.seed_artifact.content_digest.clone()),
        byte_len: Some(cell.seed_artifact.byte_len),
        media_type: Some(cell.seed_artifact.media_type.clone()),
        schema_id: Some(cell.seed_artifact.schema_id.clone()),
        semantic_type_id: cell.seed_artifact.semantic_type_id.clone(),
        producer_node_id: None,
        producer_seed_id: Some(cell.seed_id.clone()),
        artifact_role: Some(cell.seed_artifact.role),
    }
}

/// Builds an exact requirement for one typed public-output source cell.
pub fn public_output_cell_artifact_requirement(
    cell: &events::NamedTypedCellRef,
) -> EventArtifactRequirement {
    let (artifact_role, producer_node_id, producer_seed_id) = match &cell.producer {
        CellProducer::Node(node_id) => (ArtifactRole::StateOutput, Some(node_id.clone()), None),
        CellProducer::Seed(seed_id) => (ArtifactRole::SeedInput, None, Some(seed_id.clone())),
    };
    EventArtifactRequirement {
        source: EventArtifactReferenceSource::PublicOutputCell,
        artifact_id: cell.artifact_id.clone(),
        evidence_hash: cell.evidence_hash.clone(),
        digest: Some(cell.content_digest.clone()),
        byte_len: None,
        media_type: None,
        schema_id: Some(cell.schema_id.clone()),
        semantic_type_id: Some(cell.semantic_type_id.clone()),
        producer_node_id,
        producer_seed_id,
        artifact_role: Some(artifact_role),
    }
}

/// Builds an exact requirement for a rendered public-output artifact.
pub fn public_output_rendered_artifact_requirement(
    payload: &events::PublicOutputProduced,
    artifact_id: &ArtifactId,
    rendered_digest: &ContentDigest,
    media_type: MediaType,
) -> super::Result<EventArtifactRequirement> {
    let evidence_hash = payload
        .rendered_artifact_evidence_hash
        .clone()
        .ok_or_else(|| super::StoreError::ArtifactEvidenceMismatch {
            artifact_id: artifact_id.clone(),
            field: "evidence_hash",
        })?;
    Ok(EventArtifactRequirement {
        source: EventArtifactReferenceSource::PublicOutputRendered,
        artifact_id: artifact_id.clone(),
        evidence_hash,
        digest: Some(rendered_digest.clone()),
        byte_len: None,
        media_type: Some(media_type),
        schema_id: Some(payload.public_schema_id.clone()),
        semantic_type_id: None,
        producer_node_id: Some(payload.node_id.clone()),
        producer_seed_id: None,
        artifact_role: Some(ArtifactRole::PublicOutput),
    })
}

/// Builds an exact requirement for a redacted diagnostic artifact reference.
pub fn diagnostic_artifact_requirement(
    reference: &events::ArtifactEvidenceRef,
) -> EventArtifactRequirement {
    EventArtifactRequirement {
        source: EventArtifactReferenceSource::ArtifactReferenced,
        artifact_id: reference.artifact_id.clone(),
        evidence_hash: reference.evidence_hash.clone(),
        digest: Some(reference.content_digest.clone()),
        byte_len: Some(reference.byte_len),
        media_type: Some(reference.media_type.clone()),
        schema_id: Some(reference.schema_id.clone()),
        semantic_type_id: reference.semantic_type_id.clone(),
        producer_node_id: None,
        producer_seed_id: None,
        artifact_role: Some(reference.role),
    }
}

/// Builds an exact requirement for a retained fact-descriptor projection.
pub fn fact_descriptor_artifact_requirement(
    projection: &super::FactDescriptorProjection,
) -> super::Result<EventArtifactRequirement> {
    let evidence = &projection.descriptor_artifact_evidence;
    Ok(EventArtifactRequirement {
        source: EventArtifactReferenceSource::FactDescriptor,
        artifact_id: projection.descriptor_artifact_id.clone(),
        evidence_hash: evidence.evidence_hash()?,
        digest: Some(projection.descriptor_hash.clone()),
        byte_len: Some(evidence.byte_len),
        media_type: Some(evidence.media_type.clone()),
        schema_id: evidence.schema_id.clone(),
        semantic_type_id: evidence.semantic_type_id.clone(),
        producer_node_id: evidence.producer_node_id.clone(),
        producer_seed_id: evidence.producer_seed_id.clone(),
        artifact_role: Some(ArtifactRole::FactDescriptor),
    })
}

/// Builds an exact requirement for a fact response pinned by an internal fact reference.
pub fn fact_response_artifact_requirement(
    fact_ref: &mfm_facts::InternalFactRef,
) -> EventArtifactRequirement {
    EventArtifactRequirement {
        source: EventArtifactReferenceSource::FactResponse,
        artifact_id: fact_ref.artifact_id().clone(),
        evidence_hash: fact_ref.artifact_evidence_hash().clone(),
        digest: Some(fact_ref.response_hash().clone()),
        byte_len: None,
        media_type: None,
        schema_id: Some(fact_ref.response_schema_id().clone()),
        semantic_type_id: None,
        producer_node_id: Some(fact_ref.producer_node_id().clone()),
        producer_seed_id: None,
        artifact_role: Some(ArtifactRole::FactResponse),
    }
}
