use super::*;

/// Returns artifact evidence requirements referenced by one kernel event payload.
pub fn event_artifact_requirements(payload: &KernelEventPayload) -> Vec<EventArtifactRequirement> {
    let mut requirements = Vec::new();
    match payload {
        KernelEventPayload::RunStarted(payload) => {
            requirements.push(EventArtifactRequirement {
                source: EventArtifactReferenceSource::RunSpec,
                artifact_id: payload.spec_artifact_id.clone(),
                digest: Some(ContentDigest::from_digest(
                    payload.spec_hash.algorithm(),
                    *payload.spec_hash.digest(),
                )),
                byte_len: None,
                media_type: Some(payload.spec_media_type.clone()),
                schema_id: None,
                semantic_type_id: None,
                producer_node_id: None,
                producer_seed_id: None,
                artifact_role: Some(ArtifactRole::TypedExecutionSpec),
            });
            requirements.push(EventArtifactRequirement {
                source: EventArtifactReferenceSource::RunCertificate,
                artifact_id: payload.certificate_artifact_id.clone(),
                digest: Some(payload.certificate_artifact_digest.clone()),
                byte_len: None,
                media_type: Some(payload.certificate_media_type.clone()),
                schema_id: None,
                semantic_type_id: None,
                producer_node_id: None,
                producer_seed_id: None,
                artifact_role: Some(ArtifactRole::TypedSpecCertificate),
            });
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
        KernelEventPayload::FactRecorded(payload) => requirements.push(EventArtifactRequirement {
            source: EventArtifactReferenceSource::FactResponse,
            artifact_id: payload.artifact_id.clone(),
            digest: Some(payload.response_hash.clone()),
            byte_len: None,
            media_type: None,
            schema_id: Some(payload.response_schema_id.clone()),
            semantic_type_id: None,
            producer_node_id: Some(payload.node_id.clone()),
            producer_seed_id: None,
            artifact_role: Some(ArtifactRole::FactResponse),
        }),
        KernelEventPayload::ArtifactReferenced(payload) => {
            push_event_artifact(
                &mut requirements,
                EventArtifactReferenceSource::ArtifactReferenced,
                &payload.artifact_ref,
                payload.node_id.clone(),
                None,
            );
        }
        KernelEventPayload::CellProduced(payload) => requirements.push(EventArtifactRequirement {
            source: EventArtifactReferenceSource::StateOutput,
            artifact_id: payload.artifact_id.clone(),
            digest: Some(payload.content_digest.clone()),
            byte_len: None,
            media_type: None,
            schema_id: Some(payload.schema_id.clone()),
            semantic_type_id: Some(payload.semantic_type_id.clone()),
            producer_node_id: Some(payload.node_id.clone()),
            producer_seed_id: None,
            artifact_role: Some(ArtifactRole::StateOutput),
        }),
        KernelEventPayload::PublicOutputProduced(payload) => {
            for cell in &payload.cells {
                let (producer_node_id, producer_seed_id) =
                    cell_producer_artifact_owner(&cell.producer);
                requirements.push(EventArtifactRequirement {
                    source: EventArtifactReferenceSource::PublicOutputCell,
                    artifact_id: cell.artifact_id.clone(),
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
            if let Some(artifact_id) = &payload.rendered_artifact_id {
                requirements.push(EventArtifactRequirement {
                    source: EventArtifactReferenceSource::PublicOutputRendered,
                    artifact_id: artifact_id.clone(),
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
            events::RunCompletionOutcome::Completed(_) => {}
            events::RunCompletionOutcome::Compensated
            | events::RunCompletionOutcome::ManuallyResolved
            | events::RunCompletionOutcome::FailedWithoutAcdcClaim => {}
        },
        KernelEventPayload::SideEffectIntentPersisted(payload) => {
            requirements.push(EventArtifactRequirement {
                source: EventArtifactReferenceSource::SideEffectIntent,
                artifact_id: payload.intent_artifact_id.clone(),
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
            if let (Some(artifact_id), Some(hash)) =
                (&payload.prepared_artifact_id, &payload.prepared_hash)
            {
                requirements.push(EventArtifactRequirement {
                    source: EventArtifactReferenceSource::PreparedInvocation,
                    artifact_id: artifact_id.clone(),
                    digest: Some(hash.clone()),
                    byte_len: None,
                    media_type: None,
                    schema_id: None,
                    semantic_type_id: None,
                    producer_node_id: Some(payload.node_id.clone()),
                    producer_seed_id: None,
                    artifact_role: Some(ArtifactRole::PreparedInvocation),
                });
            }
        }
        KernelEventPayload::SideEffectNotSubmittedProven(payload) => {
            requirements.push(EventArtifactRequirement {
                source: EventArtifactReferenceSource::NotSubmittedProof,
                artifact_id: payload.proof_artifact_id.clone(),
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
        | KernelEventPayload::SideEffectInvocationStarted(_)
        | KernelEventPayload::StateAttemptCompleted(_) => {}
    }
    requirements
}

pub(super) fn referenced_artifact_ids(request: &TypedCommitRequest) -> BTreeSet<ArtifactId> {
    let mut artifact_ids = BTreeSet::new();
    for payload in &request.payloads {
        for requirement in event_artifact_requirements(payload) {
            artifact_ids.insert(requirement.artifact_id);
        }
    }
    artifact_ids
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
