use super::*;

pub(super) fn insert_retention_ref(
    refs: &mut BTreeMap<(ArtifactId, ContentDigest), events::RetentionRef>,
    retention_ref: events::RetentionRef,
) {
    refs.entry((
        retention_ref.artifact_id.clone(),
        retention_ref.evidence_hash.clone(),
    ))
    .or_insert(retention_ref);
}

pub(super) fn ensure_artifact_role(
    artifact: &RunnerJsonArtifact,
    role: events::ArtifactRole,
) -> Result<()> {
    if artifact.evidence.artifact_role == role {
        Ok(())
    } else {
        Err(RuntimeError::InvalidRunnerOutput(format!(
            "runner artifact role {} did not match expected role {}",
            artifact.evidence.artifact_role.as_str(),
            role.as_str()
        )))
    }
}

pub(super) fn artifact_schema_id(artifact: &RunnerJsonArtifact) -> Result<SchemaId> {
    artifact.evidence.schema_id.clone().ok_or_else(|| {
        RuntimeError::InvalidRunnerOutput(format!(
            "runner artifact role {} did not carry schema metadata",
            artifact.evidence.artifact_role.as_str()
        ))
    })
}

pub(super) fn ensure_fact_descriptor_matches_type<T>(
    descriptor: &mfm_facts::FactDescriptor,
) -> Result<()>
where
    T: MfmFactType,
{
    let subject_schema_id = <T::Subject as MfmValue>::schema_id().map_err(runtime_value_error)?;
    let response_schema_id = <T::Response as MfmValue>::schema_id().map_err(runtime_value_error)?;

    if descriptor.subject_schema_id() != &subject_schema_id {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "fact subject schema {} did not match subject type schema {}",
            descriptor.subject_schema_id(),
            subject_schema_id
        )));
    }
    if descriptor.response_schema_id() != &response_schema_id {
        return Err(RuntimeError::InvalidRunnerOutput(format!(
            "fact response schema {} did not match response type schema {}",
            descriptor.response_schema_id(),
            response_schema_id
        )));
    }

    Ok(())
}

pub(super) fn ensure_node_allows_fact_descriptor(
    node: &spec::NodeSpec,
    descriptor_hash: &ContentDigest,
) -> Result<()> {
    if node
        .fact_descriptor_allowlist
        .iter()
        .any(|reference| &reference.descriptor_hash == descriptor_hash)
    {
        return Ok(());
    }
    Err(RuntimeError::InvalidRunnerOutput(format!(
        "node {} is not certified to emit fact descriptor {}",
        node.node_id, descriptor_hash
    )))
}

pub(super) fn canonical_json<T>(value: &T) -> Result<PlainCanonicalJsonBytes>
where
    T: Serialize,
{
    let json =
        serde_json::to_string(value).map_err(|error| RuntimeError::Canonical(error.to_string()))?;
    PlainCanonicalJsonBytes::from_json_str(&json)
        .map_err(|error| RuntimeError::Canonical(error.to_string()))
}

pub(super) fn runtime_value_error(error: mfm_values::ValueError) -> RuntimeError {
    RuntimeError::InvalidRunnerOutput(error.to_string())
}

pub(super) fn runtime_fact_error(error: mfm_facts::FactError) -> RuntimeError {
    RuntimeError::InvalidRunnerOutput(error.to_string())
}
