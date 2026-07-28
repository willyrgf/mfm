use super::*;

pub(crate) fn required_artifacts_for_payloads(
    lifecycle: &store::current_lifecycle::CurrentLifecycleReader<'_>,
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
            let Some(evidence) = committed_artifact_for_requirement(lifecycle, &requirement) else {
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

fn committed_artifact_for_requirement(
    lifecycle: &store::current_lifecycle::CurrentLifecycleReader<'_>,
    requirement: &store::EventArtifactRequirement,
) -> Option<store::ArtifactEvidenceRef> {
    let mut matched = None;
    let _ = lifecycle.visit_records(|record| {
        let _ = record.visit_artifact_requirements(|committed_requirement| {
            if committed_requirement.artifact_id != requirement.artifact_id
                || committed_requirement.evidence_hash != requirement.evidence_hash
            {
                return std::ops::ControlFlow::Continue(());
            }
            let Some(object) = lifecycle.object_for_requirement(committed_requirement) else {
                return std::ops::ControlFlow::Continue(());
            };
            if store::validate_artifact_requirement_against_evidence(requirement, object.evidence())
                .is_ok()
            {
                matched = Some(object.evidence().clone());
                return std::ops::ControlFlow::Break(());
            }
            std::ops::ControlFlow::Continue(())
        });
        if matched.is_some() {
            std::ops::ControlFlow::Break(())
        } else {
            std::ops::ControlFlow::Continue(())
        }
    });
    matched
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
