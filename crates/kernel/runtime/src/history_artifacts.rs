use super::*;

/// One exact input artifact borrowed from the verified current lifecycle.
pub(crate) struct CurrentInputArtifactRef<'view> {
    object: store::current_lifecycle::CurrentObjectRef<'view>,
    attempt_id: Option<&'view AttemptId>,
}

impl<'view> CurrentInputArtifactRef<'view> {
    pub(crate) fn evidence(&self) -> &'view store::ArtifactEvidenceRef {
        self.object.evidence()
    }

    pub(crate) fn bytes(&self) -> &'view [u8] {
        self.object.bytes()
    }

    pub(crate) fn attempt_id(&self) -> Option<&'view AttemptId> {
        self.attempt_id
    }
}

pub(crate) fn committed_config_artifact(
    node: &spec::NodeSpec,
    lifecycle: &store::current_lifecycle::CurrentLifecycleReader<'_>,
) -> Result<store::ArtifactEvidenceRef> {
    let admitted = lifecycle
        .config(&node.config_ref.artifact_id)?
        .ok_or_else(|| {
            RuntimeError::InputMaterialization(format!(
                "node {} config artifact {} is not committed in the run journal",
                node.node_id, node.config_ref.artifact_id
            ))
        })?;
    let admitted = admitted.evidence();
    if admitted.artifact_id != node.config_ref.artifact_id
        || admitted.content_digest != node.config_ref.digest
        || admitted.byte_len != node.config_ref.byte_len
        || admitted.media_type != node.config_ref.media_type
        || admitted.schema_id.as_ref() != Some(&node.config_ref.schema_id)
        || admitted.semantic_type_id.is_some()
        || admitted.role != events::ArtifactRole::TypedConfig
    {
        return Err(RuntimeError::InputMaterialization(format!(
            "node {} committed config artifact evidence does not match certified config ref",
            node.node_id
        )));
    }
    let requirement = store::run_artifact_requirement(
        store::EventArtifactReferenceSource::RunConfig,
        admitted,
        events::ArtifactRole::TypedConfig,
    );
    let object = lifecycle
        .object_for_requirement(&requirement)
        .ok_or_else(|| {
            RuntimeError::InputMaterialization(format!(
                "node {} config artifact {} lacks exact retained-object authority",
                node.node_id, node.config_ref.artifact_id
            ))
        })?;
    Ok(object.evidence().clone())
}

pub(crate) fn committed_input_artifact<'view>(
    lifecycle: &store::current_lifecycle::CurrentLifecycleReader<'view>,
    cell_id: &CellId,
    artifact_id: &ArtifactId,
    evidence_hash: &ContentDigest,
    expected_role: events::ArtifactRole,
) -> Result<CurrentInputArtifactRef<'view>> {
    let mut matched = None;
    let _ = lifecycle.visit_records(|record| {
        let (requirements, attempt_id) = match record.kind() {
            store::current_lifecycle::CurrentRecordKindRef::RunAdmitted(payload) => (
                store::event_artifact_requirements(&events::KernelEventPayload::RunAdmitted(
                    Box::new(payload.clone()),
                )),
                None,
            ),
            store::current_lifecycle::CurrentRecordKindRef::ArtifactReferenced(payload) => (
                vec![store::artifact_referenced_artifact_requirement(payload)],
                payload.attempt_id.as_ref(),
            ),
            _ => return std::ops::ControlFlow::Continue(()),
        };
        for requirement in requirements {
            if &requirement.artifact_id == artifact_id
                && &requirement.evidence_hash == evidence_hash
                && requirement.artifact_role == Some(expected_role)
            {
                matched = Some((requirement, attempt_id));
                return std::ops::ControlFlow::Break(());
            }
        }
        std::ops::ControlFlow::Continue(())
    });
    let (requirement, attempt_id) = matched.ok_or_else(|| {
        RuntimeError::InputMaterialization(format!(
            "input cell {cell_id} artifact {artifact_id} is not committed in the run journal",
        ))
    })?;
    let object = lifecycle
        .object_for_requirement(&requirement)
        .ok_or_else(|| {
            RuntimeError::InputMaterialization(format!(
                "input cell {cell_id} artifact {artifact_id} lacks exact retained-object authority",
            ))
        })?;
    Ok(CurrentInputArtifactRef { object, attempt_id })
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

pub(crate) fn validate_spec_artifact(
    runtime_spec: &CertifiedRuntimeSpec,
    evidence: store::ArtifactEvidenceRef,
) -> Result<store::ArtifactEvidenceRef> {
    let canonical = runtime_spec
        .spec()
        .canonical_json()
        .map_err(|error| RuntimeError::Canonical(error.to_string()))?;
    validate_certified_artifact(
        evidence,
        &canonical,
        runtime_spec.spec().media_type.clone(),
        spec::typed_execution_spec_schema_id()
            .map_err(|error| RuntimeError::Identity(error.to_string()))?,
        events::ArtifactRole::TypedExecutionSpec,
        "typed execution spec artifact evidence does not match the certified spec",
    )
}

pub(crate) fn validate_certificate_artifact(
    runtime_spec: &CertifiedRuntimeSpec,
    evidence: store::ArtifactEvidenceRef,
) -> Result<store::ArtifactEvidenceRef> {
    let canonical = runtime_spec
        .certificate()
        .canonical_json()
        .map_err(|error| RuntimeError::Canonical(error.to_string()))?;
    let media_type = spec::MediaType::new(mfm_certify::CERTIFICATE_MEDIA_TYPE)
        .map_err(|error| RuntimeError::Identity(error.to_string()))?;
    validate_certified_artifact(
        evidence,
        &canonical,
        media_type,
        mfm_certify::typed_spec_certificate_schema_id()
            .map_err(|error| RuntimeError::Identity(error.to_string()))?,
        events::ArtifactRole::TypedSpecCertificate,
        "typed spec certificate artifact evidence does not match the certified spec",
    )
}

fn validate_certified_artifact(
    evidence: store::ArtifactEvidenceRef,
    canonical: &mfm_canonical::PlainCanonicalJsonBytes,
    media_type: spec::MediaType,
    schema_id: SchemaId,
    artifact_role: events::ArtifactRole,
    mismatch_message: &'static str,
) -> Result<store::ArtifactEvidenceRef> {
    let digest = canonical.content_digest();
    let expected_artifact_id = ArtifactId::from_digest(digest.algorithm(), *digest.digest());
    if evidence.artifact_id != expected_artifact_id
        || evidence.digest != digest
        || evidence.byte_len != canonical.as_bytes().len() as u64
        || evidence.media_type != media_type
        || evidence.schema_id.as_ref() != Some(&schema_id)
        || evidence.semantic_type_id.is_some()
        || evidence.producer_node_id.is_some()
        || evidence.producer_seed_id.is_some()
        || evidence.artifact_role != artifact_role
    {
        return Err(RuntimeError::InvalidRunStream(mismatch_message.to_owned()));
    }
    Ok(evidence)
}

pub(crate) fn validate_config_artifacts(
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
