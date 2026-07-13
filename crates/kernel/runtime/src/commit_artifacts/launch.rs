use super::*;

pub(crate) fn required_artifacts_for_payloads(
    view: &RuntimeRunView,
    required_artifacts: Vec<store::ArtifactEvidenceRef>,
    payloads: &[events::KernelEventPayload],
) -> Result<Vec<store::ArtifactEvidenceRef>> {
    let mut required_artifacts = required_artifacts;
    for payload in payloads {
        for requirement in store::event_artifact_requirements(payload) {
            if required_artifacts.iter().any(|evidence| {
                evidence.artifact_id == requirement.artifact_id
                    && store::validate_artifact_requirement_against_evidence(&requirement, evidence)
                        .is_ok()
            }) {
                continue;
            }
            let Some(evidence) = committed_artifact_for_requirement(view, &requirement) else {
                if requirement.source.is_retention() {
                    if let Some(evidence) =
                        retained_fact_artifact_evidence_for_requirement(view, &requirement)?
                    {
                        required_artifacts.push(evidence);
                    }
                    continue;
                }
                return Err(RuntimeError::InvalidRunnerOutput(format!(
                    "payload references artifact {} without required evidence",
                    requirement.artifact_id
                )));
            };
            required_artifacts.push(evidence);
        }
    }
    Ok(required_artifacts)
}

fn retained_fact_artifact_evidence_for_requirement(
    view: &RuntimeRunView,
    requirement: &store::EventArtifactRequirement,
) -> Result<Option<store::ArtifactEvidenceRef>> {
    match requirement.artifact_role {
        Some(events::ArtifactRole::FactDescriptor) => {
            let descriptor_hash = requirement.digest.as_ref().ok_or_else(|| {
                RuntimeError::InvalidRunnerOutput(format!(
                    "retention ref for artifact {} lacks descriptor digest",
                    requirement.artifact_id
                ))
            })?;
            let Some(descriptor) = view
                .projections
                .fact_descriptor(descriptor_hash)
                .filter(|descriptor| descriptor.descriptor_artifact_id == requirement.artifact_id)
                .filter(|descriptor| {
                    descriptor
                        .descriptor_artifact_evidence
                        .evidence_hash()
                        .is_ok_and(|actual| actual == requirement.evidence_hash)
                })
            else {
                return Ok(None);
            };
            Ok(Some(descriptor.descriptor_artifact_evidence.clone()))
        }
        Some(events::ArtifactRole::FactResponse) => Ok(view
            .projections
            .fact_index_entries()
            .find(|(_, index)| {
                index.artifact_id == requirement.artifact_id
                    && requirement
                        .digest
                        .as_ref()
                        .is_some_and(|digest| index.response_hash == *digest)
                    && index.artifact_evidence_hash == requirement.evidence_hash
            })
            .and_then(|(claim_id, _index)| {
                let record = view.projections.fact_record(claim_id)?;
                let evidence = record.response_artifact_evidence.as_ref()?;
                let evidence_hash = evidence.evidence_hash().ok()?;
                (evidence_hash == requirement.evidence_hash).then(|| evidence.clone())
            })),
        _ => Ok(None),
    }
}

fn committed_artifact_for_requirement(
    view: &RuntimeRunView,
    requirement: &store::EventArtifactRequirement,
) -> Option<store::ArtifactEvidenceRef> {
    view.artifact_refs
        .get(&(
            requirement.artifact_id.clone(),
            requirement.evidence_hash.clone(),
        ))
        .filter(|reference| {
            store::validate_artifact_requirement_against_evidence(requirement, &reference.evidence)
                .is_ok()
        })
        .map(|reference| reference.evidence.clone())
}

pub(crate) fn launch_artifacts_by_evidence(
    artifacts: Vec<RunLaunchArtifact>,
    kind: &'static str,
) -> Result<BTreeMap<(ArtifactId, ContentDigest), RunLaunchArtifact>> {
    let mut by_evidence = BTreeMap::new();
    for artifact in artifacts {
        verify_artifact_bytes(&artifact.bytes, &artifact.evidence)?;
        let evidence_hash = artifact.evidence.evidence_hash()?;
        let key = (artifact.evidence.artifact_id.clone(), evidence_hash);
        if by_evidence.insert(key, artifact).is_some() {
            return Err(RuntimeError::InvalidRunStream(format!(
                "duplicate staged {kind} launch artifact"
            )));
        }
    }
    Ok(by_evidence)
}

pub(crate) fn validate_fact_descriptor_launch_artifacts(
    runtime_spec: &CertifiedRuntimeSpec,
    artifacts: Vec<RunLaunchArtifact>,
) -> Result<Vec<RunLaunchArtifact>> {
    let required = runtime_spec.fact_descriptor_hashes();
    let schema_id = mfm_facts::fact_descriptor_schema_id()
        .map_err(|error| RuntimeError::Identity(error.to_string()))?;
    let media_type = spec::MediaType::new("application/json")?;
    let mut by_hash = BTreeMap::new();

    for artifact in artifacts {
        verify_artifact_bytes(&artifact.bytes, &artifact.evidence)?;
        let descriptor = mfm_facts::parse_canonical_fact_descriptor_bytes(&artifact.bytes)
            .map_err(runtime_fact_error)?;
        let canonical =
            mfm_facts::canonical_fact_descriptor_bytes(&descriptor).map_err(runtime_fact_error)?;
        let descriptor_hash =
            mfm_facts::fact_descriptor_hash(&descriptor).map_err(runtime_fact_error)?;
        let expected_artifact_id =
            ArtifactId::from_digest(descriptor_hash.algorithm(), *descriptor_hash.digest());
        if artifact.evidence.artifact_id != expected_artifact_id
            || artifact.evidence.digest != descriptor_hash
            || artifact.evidence.byte_len != canonical.as_bytes().len() as u64
            || artifact.evidence.media_type != media_type
            || artifact.evidence.schema_id.as_ref() != Some(&schema_id)
            || artifact.evidence.semantic_type_id.is_some()
            || artifact.evidence.producer_node_id.is_some()
            || artifact.evidence.producer_seed_id.is_some()
            || artifact.evidence.artifact_role != events::ArtifactRole::FactDescriptor
        {
            return Err(RuntimeError::InvalidRunnerOutput(format!(
                "fact descriptor artifact evidence does not match descriptor {}",
                descriptor_hash
            )));
        }
        if by_hash.insert(descriptor_hash.clone(), artifact).is_some() {
            return Err(RuntimeError::InvalidRunnerOutput(format!(
                "duplicate fact descriptor artifact for {}",
                descriptor_hash
            )));
        }
    }

    for required_hash in &required {
        if !by_hash.contains_key(required_hash) {
            return Err(RuntimeError::InvalidRunnerOutput(format!(
                "missing fact descriptor artifact for certified descriptor {}",
                required_hash
            )));
        }
    }
    for admitted_hash in by_hash.keys() {
        if !required.contains(admitted_hash) {
            return Err(RuntimeError::InvalidRunnerOutput(format!(
                "fact descriptor artifact {} is not certified by the runtime spec",
                admitted_hash
            )));
        }
    }

    Ok(by_hash.into_values().collect())
}

pub(crate) fn validate_launch_seed_artifacts<'a>(
    seeds: Vec<RunLaunchSeedCell>,
    validated_cells: impl IntoIterator<Item = &'a events::SeedCellRef>,
) -> Result<Vec<PreparedStagedArtifact>> {
    let mut by_cell = BTreeMap::new();
    for seed in seeds {
        if by_cell.insert(seed.cell.cell_id.clone(), seed).is_some() {
            return Err(RuntimeError::InvalidRunStream(
                "duplicate staged seed launch artifact".to_owned(),
            ));
        }
    }
    let mut staged = Vec::with_capacity(by_cell.len());
    for cell in validated_cells {
        let seed = by_cell.remove(&cell.cell_id).ok_or_else(|| {
            RuntimeError::InvalidRunStream(format!(
                "missing staged seed bytes for cell {}",
                cell.cell_id
            ))
        })?;
        let evidence = store_seed_artifact(cell);
        verify_artifact_bytes(&seed.bytes, &evidence)?;
        staged.push(PreparedStagedArtifact {
            bytes: seed.bytes,
            evidence,
        });
    }
    if !by_cell.is_empty() {
        return Err(RuntimeError::InvalidRunStream(
            "staged seed launch artifacts contain entries not certified by the spec".to_owned(),
        ));
    }
    Ok(staged)
}
