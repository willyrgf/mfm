use super::*;

pub(super) fn config_artifacts_from_run_admitted(
    runtime_spec: &CertifiedRuntimeSpec,
    config_artifacts: &[events::RunArtifactEvidenceRef],
) -> Result<BTreeMap<String, store::ArtifactEvidenceRef>> {
    let validated = validate_config_artifacts(
        runtime_spec,
        config_artifacts
            .iter()
            .map(store_artifact_from_run_ref)
            .collect(),
    )?;
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

pub(super) fn artifact_refs_from_stream(
    stream: &[store::KernelEventEnvelope],
    history: &RuntimeCommittedHistory,
) -> Result<BTreeMap<store::ArtifactAuthorityKey, CommittedArtifactReference>> {
    let mut artifacts = BTreeMap::new();
    for batch in history.commits() {
        let commit = batch.events(stream);
        for event in commit {
            match event.payload() {
                events::KernelEventPayload::RunAdmitted(payload) => {
                    for artifact in std::iter::once(&payload.spec_artifact)
                        .chain(std::iter::once(&payload.certificate_artifact))
                        .chain(payload.config_artifacts.iter())
                        .chain(payload.fact_descriptor_artifacts.iter())
                    {
                        insert_committed_artifact(
                            &mut artifacts,
                            CommittedArtifactReference {
                                evidence: store_artifact_from_run_ref(artifact),
                                attempt_id: None,
                                commit_seq: event.seq(),
                                commit_key: event.commit_key().clone(),
                            },
                        )?;
                    }
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
                && payload.evidence_hash == reference.artifact_ref.evidence_hash
                && payload.schema_id == reference.artifact_ref.schema_id
                && reference.artifact_ref.semantic_type_id.as_ref()
                    == Some(&payload.semantic_type_id)
        }
        events::KernelEventPayload::FactRecorded(payload) => {
            let response = payload.claim.response();
            reference.artifact_ref.role == events::ArtifactRole::FactResponse
                && &payload.node_id == reference_node_id
                && &payload.attempt_id == reference_attempt_id
                && response.artifact_id() == &reference.artifact_ref.artifact_id
                && response.response_hash() == &reference.artifact_ref.content_digest
                && response.artifact_evidence_hash() == &reference.artifact_ref.evidence_hash
                && response.response_schema_id() == &reference.artifact_ref.schema_id
        }
        events::KernelEventPayload::PublicOutputProduced(payload) => {
            reference.artifact_ref.role == events::ArtifactRole::PublicOutput
                && &payload.node_id == reference_node_id
                && &payload.attempt_id == reference_attempt_id
                && payload.rendered_artifact_id.as_ref()
                    == Some(&reference.artifact_ref.artifact_id)
                && payload.rendered_digest == reference.artifact_ref.content_digest
                && payload.rendered_artifact_evidence_hash.as_ref()
                    == Some(&reference.artifact_ref.evidence_hash)
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
        events::KernelEventPayload::ArtifactReferenced(payload) => {
            matches!(
                reference.artifact_ref.role,
                events::ArtifactRole::FactQueryEvidence
                    | events::ArtifactRole::ExternalReadEvidence
            ) && payload.artifact_ref.role == reference.artifact_ref.role
                && payload.node_id.as_ref() == Some(reference_node_id)
                && payload.attempt_id.as_ref() == Some(reference_attempt_id)
                && event_artifact_refs_match(&payload.artifact_ref, reference)
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
        && diagnostic.evidence_hash == reference.artifact_ref.evidence_hash
        && diagnostic.byte_len == reference.artifact_ref.byte_len
        && diagnostic.media_type == reference.artifact_ref.media_type
}

fn insert_committed_artifact(
    artifacts: &mut BTreeMap<store::ArtifactAuthorityKey, CommittedArtifactReference>,
    artifact: CommittedArtifactReference,
) -> Result<()> {
    let key = (
        artifact.evidence.artifact_id.clone(),
        artifact
            .evidence
            .evidence_hash()
            .map_err(RuntimeError::from)?,
    );
    if let Some(existing) = artifacts.get(&key) {
        if existing != &artifact {
            return Err(RuntimeError::InvalidRunStream(format!(
                "conflicting committed artifact evidence for {}",
                artifact.evidence.artifact_id
            )));
        }
        return Ok(());
    }
    artifacts.insert(key, artifact);
    Ok(())
}

pub(crate) fn committed_config_artifact(
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

pub(super) fn committed_input_artifact(
    view: &RuntimeRunView,
    cell_id: &CellId,
    artifact_id: &ArtifactId,
    evidence_hash: &ContentDigest,
) -> Result<CommittedArtifactReference> {
    view.artifact_refs
        .get(&(artifact_id.clone(), evidence_hash.clone()))
        .cloned()
        .ok_or_else(|| {
            RuntimeError::InputMaterialization(format!(
                "input cell {cell_id} artifact {artifact_id} is not committed in the run stream",
            ))
        })
}

pub(super) fn store_artifact_from_event_ref(
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

pub(super) fn store_artifact_from_run_ref(
    evidence: &events::RunArtifactEvidenceRef,
) -> store::ArtifactEvidenceRef {
    store::ArtifactEvidenceRef {
        artifact_id: evidence.artifact_id.clone(),
        digest: evidence.content_digest.clone(),
        byte_len: evidence.byte_len,
        media_type: evidence.media_type.clone(),
        schema_id: evidence.schema_id.clone(),
        semantic_type_id: evidence.semantic_type_id.clone(),
        producer_node_id: None,
        producer_seed_id: None,
        artifact_role: evidence.role,
    }
}

pub(crate) fn run_artifact_ref_from_store(
    artifact: &store::ArtifactEvidenceRef,
) -> Result<events::RunArtifactEvidenceRef> {
    Ok(events::RunArtifactEvidenceRef {
        artifact_id: artifact.artifact_id.clone(),
        role: artifact.artifact_role,
        schema_id: artifact.schema_id.clone(),
        semantic_type_id: artifact.semantic_type_id.clone(),
        content_digest: artifact.digest.clone(),
        evidence_hash: artifact.evidence_hash()?,
        byte_len: artifact.byte_len,
        media_type: artifact.media_type.clone(),
    })
}

pub(crate) fn event_artifact_ref_from_store(
    artifact: &store::ArtifactEvidenceRef,
) -> Result<events::ArtifactEvidenceRef> {
    let schema_id = artifact.schema_id.clone().ok_or_else(|| {
        RuntimeError::InvalidRunStream(format!(
            "artifact {} cannot be referenced without schema id",
            artifact.artifact_id
        ))
    })?;
    Ok(events::ArtifactEvidenceRef {
        artifact_id: artifact.artifact_id.clone(),
        role: artifact.artifact_role,
        schema_id,
        semantic_type_id: artifact.semantic_type_id.clone(),
        content_digest: artifact.digest.clone(),
        evidence_hash: artifact.evidence_hash()?,
        byte_len: artifact.byte_len,
        media_type: artifact.media_type.clone(),
    })
}

pub(crate) fn payload_spec_hash(payload: &events::KernelEventPayload) -> SpecHash {
    payload.spec_hash().clone()
}

pub(crate) fn store_seed_artifact(seed: &events::SeedCellRef) -> store::ArtifactEvidenceRef {
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
