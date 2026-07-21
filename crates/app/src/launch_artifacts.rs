use super::*;

pub(super) async fn retained_source_fact_events_from_query_evidence<S, A>(
    store: &S,
    artifacts: &A,
    stream: &[store::KernelEventEnvelope],
) -> Result<Vec<RetainedSourceFactReplayEvent>, PublicError>
where
    S: store::RunEventStore + Send + Sync,
    A: store::RetainedArtifactReadProvider + ?Sized,
{
    let mut source_events = BTreeMap::new();
    for event in stream {
        let events::KernelEventPayload::ArtifactReferenced(payload) = event.payload() else {
            continue;
        };
        if payload.artifact_ref.role != events::ArtifactRole::FactQueryEvidence {
            continue;
        }
        let artifact = artifacts
            .read_retained_artifact(&store::artifact_referenced_artifact_requirement(payload))
            .await
            .map_err(async_app_store_error)?;
        let evidence = mfm_facts::parse_canonical_fact_query_evidence_bytes(artifact.bytes())
            .map_err(|_| {
                PublicError::backend(
                    ErrorClass::Internal,
                    "FactQueryEvidenceInvalid",
                    "Fact query evidence artifact is invalid",
                )
            })?;
        for fact_ref in evidence.receipt().returned_refs() {
            let fact_claim_id = fact_ref.fact_claim_id().clone();
            if source_events.contains_key(&fact_claim_id) {
                continue;
            }
            let source_stream = store
                .load_run_stream(fact_claim_id.source_run_id())
                .await
                .map_err(async_app_store_error)?;
            let envelope = source_stream
                .into_iter()
                .find(|candidate| {
                    candidate.seq().as_u64() == fact_claim_id.source_seq()
                        && candidate.ordinal().as_u32() == fact_claim_id.source_ordinal()
                })
                .ok_or_else(|| {
                    PublicError::backend(
                        ErrorClass::Internal,
                        "FactQuerySourceFactMissing",
                        "Fact query evidence source fact event is missing",
                    )
                })?;
            let source_event = RetainedSourceFactReplayEvent::new(fact_claim_id.clone(), envelope)?;
            source_events.insert(fact_claim_id, source_event);
        }
    }
    Ok(source_events.into_values().collect())
}

pub(super) fn certified_spec_launch_artifact(
    runtime_spec: &CertifiedRuntimeSpec,
) -> Result<RunLaunchArtifact, PublicError> {
    let canonical = runtime_spec.spec().canonical_json().map_err(|error| {
        let _ = error;
        PublicError::backend(
            ErrorClass::Internal,
            "CertifiedSpecCanonicalError",
            "Certified typed spec canonicalization failed",
        )
    })?;
    Ok(launch_artifact(
        canonical.to_vec(),
        runtime_spec.spec().media_type.clone(),
        Some(spec::typed_execution_spec_schema_id().map_err(|_| {
            PublicError::backend(
                ErrorClass::Internal,
                "TypedSpecSchemaInvalid",
                "Typed execution spec schema identity is invalid",
            )
        })?),
        None,
        None,
        events::ArtifactRole::TypedExecutionSpec,
    ))
}

pub(super) fn certified_spec_certificate_launch_artifact(
    runtime_spec: &CertifiedRuntimeSpec,
) -> Result<RunLaunchArtifact, PublicError> {
    let canonical = runtime_spec
        .certificate()
        .canonical_json()
        .map_err(|error| {
            let _ = error;
            PublicError::backend(
                ErrorClass::Internal,
                "CertifiedCertificateCanonicalError",
                "Certified typed spec certificate canonicalization failed",
            )
        })?;
    let media_type =
        spec::MediaType::new(mfm_certify::CERTIFICATE_MEDIA_TYPE).map_err(|error| {
            let _ = error;
            PublicError::backend(
                ErrorClass::Internal,
                "CertifiedCertificateMediaTypeInvalid",
                "Certified typed spec certificate media type is invalid",
            )
        })?;
    Ok(launch_artifact(
        canonical.to_vec(),
        media_type,
        Some(
            mfm_certify::typed_spec_certificate_schema_id().map_err(|_| {
                PublicError::backend(
                    ErrorClass::Internal,
                    "TypedSpecCertificateSchemaInvalid",
                    "Typed spec certificate schema identity is invalid",
                )
            })?,
        ),
        None,
        None,
        events::ArtifactRole::TypedSpecCertificate,
    ))
}

pub(super) fn config_launch_artifacts_for_spec(
    runtime_spec: &CertifiedRuntimeSpec,
    registry: &CertificationRegistry,
    configs: Vec<RunLaunchConfigArtifact>,
) -> Result<Vec<RunLaunchArtifact>, PublicError> {
    let mut supplied = BTreeMap::new();
    for config in configs {
        let artifact = launch_artifact(
            config.bytes,
            config.media_type,
            Some(config.schema_id.clone()),
            None,
            None,
            events::ArtifactRole::TypedConfig,
        );
        let key = config_input_key(&config.schema_id, &artifact.evidence.digest);
        if let Some(existing) = supplied.get(&key) {
            if existing != &artifact {
                return Err(PublicError::new(
                    ErrorClass::BadRequest,
                    "DuplicateLaunchConfigArtifact",
                    "config input was supplied more than once with conflicting bytes",
                ));
            }
            continue;
        }
        if supplied.insert(key, artifact).is_some() {
            return Err(PublicError::new(
                ErrorClass::BadRequest,
                "DuplicateLaunchConfigArtifact",
                "config input was supplied more than once",
            ));
        }
    }

    let mut validated = Vec::with_capacity(runtime_spec.spec().config_refs.len());
    for config_ref in &runtime_spec.spec().config_refs {
        let key = config_input_key(&config_ref.schema_id, &config_ref.digest);
        let artifact = supplied.remove(&key).ok_or_else(|| {
            PublicError::new(
                ErrorClass::BadRequest,
                "MissingLaunchConfigArtifact",
                format!("missing config input for {}", config_ref.schema_id),
            )
        })?;
        validate_artifact_requirement_for_app(
            store::config_ref_artifact_requirement(config_ref)?,
            &artifact.evidence,
            ErrorClass::BadRequest,
            "LaunchConfigArtifactMismatch",
            "typed config input does not match the certified spec",
        )?;
        if registry
            .validate_config_ref_bytes(config_ref, &artifact.bytes)?
            .is_none()
            && !framework_config_matches_ref(runtime_spec.spec(), config_ref, &artifact.bytes)?
        {
            return Err(PublicError::new(
                ErrorClass::BadRequest,
                "LaunchConfigValidatorMissing",
                format!(
                    "no trusted typed config validator was registered for {}",
                    config_ref.schema_id
                ),
            ));
        }
        validated.push(artifact);
    }
    if !supplied.is_empty() {
        return Err(PublicError::new(
            ErrorClass::BadRequest,
            "UnknownLaunchConfigArtifact",
            "config input was supplied for a config not present in the certified spec",
        ));
    }
    Ok(validated)
}

pub(super) fn fact_descriptor_launch_artifacts_for_spec(
    runtime_spec: &CertifiedRuntimeSpec,
    registry: &CertificationRegistry,
) -> Result<Vec<RunLaunchArtifact>, PublicError> {
    let schema_id = mfm_program::facts::fact_descriptor_schema_id().map_err(|_| {
        PublicError::backend(
            ErrorClass::Internal,
            "FactDescriptorSchemaInvalid",
            "Fact descriptor schema identity is invalid",
        )
    })?;
    let media_type = json_media_type()?;
    let required = runtime_spec
        .spec()
        .nodes
        .iter()
        .chain(runtime_spec.spec().remediations.values())
        .flat_map(|node| {
            node.fact_descriptor_allowlist
                .iter()
                .map(|reference| reference.descriptor_hash.clone())
        })
        .collect::<BTreeSet<_>>();
    let mut artifacts = Vec::with_capacity(required.len());
    for descriptor_hash in required {
        let descriptor = registry
            .fact_descriptor_artifact(&descriptor_hash)
            .ok_or_else(|| {
                PublicError::backend(
                    ErrorClass::Internal,
                    "FactDescriptorArtifactMissing",
                    "certified fact descriptor bytes are not available for launch",
                )
            })?;
        let artifact = launch_artifact(
            descriptor.bytes().to_vec(),
            media_type.clone(),
            Some(schema_id.clone()),
            None,
            None,
            events::ArtifactRole::FactDescriptor,
        );
        if artifact.evidence.digest != *descriptor.descriptor_hash()
            || artifact.evidence.digest != descriptor_hash
        {
            return Err(PublicError::backend(
                ErrorClass::Internal,
                "FactDescriptorArtifactTampered",
                "certified fact descriptor bytes do not match their descriptor hash",
            ));
        }
        artifacts.push(artifact);
    }
    Ok(artifacts)
}

pub(super) fn framework_config_launch_artifacts_for_spec(
    execution_spec: &spec::TypedExecutionSpec,
) -> Result<Vec<RunLaunchConfigArtifact>, PublicError> {
    let mut artifacts = Vec::new();
    for node in &execution_spec.nodes {
        let Some(framework) = &node.framework else {
            continue;
        };
        let bytes =
            match spec::framework_config_canonical_json(framework.config_kind(), &node.node_id) {
                Ok(bytes) => bytes,
                Err(error) => {
                    let _ = error;
                    return Err(PublicError::backend(
                        ErrorClass::Internal,
                        "LaunchFrameworkConfigInvalid",
                        "Framework config canonicalization failed",
                    ));
                }
            };
        artifacts.push(RunLaunchConfigArtifact {
            schema_id: node.config_ref.schema_id.clone(),
            bytes: bytes.to_vec(),
            media_type: json_media_type()?,
        });
    }
    Ok(artifacts)
}

pub(super) fn framework_config_matches_ref(
    execution_spec: &spec::TypedExecutionSpec,
    config_ref: &spec::ConfigRef,
    bytes: &[u8],
) -> Result<bool, PublicError> {
    for node in &execution_spec.nodes {
        if &node.config_ref != config_ref {
            continue;
        }
        let Some(framework) = &node.framework else {
            continue;
        };
        let expected =
            spec::framework_config_canonical_json(framework.config_kind(), &node.node_id).map_err(
                |error| {
                    let _ = error;
                    PublicError::backend(
                        ErrorClass::Internal,
                        "LaunchFrameworkConfigInvalid",
                        "Framework config canonicalization failed",
                    )
                },
            )?;
        return Ok(expected.as_bytes() == bytes);
    }
    Ok(false)
}

pub(super) fn seed_launch_cells_for_spec(
    runtime_spec: &CertifiedRuntimeSpec,
    seeds: Vec<RunLaunchSeedArtifact>,
) -> Result<Vec<RunLaunchSeedCell>, PublicError> {
    let mut supplied = std::collections::BTreeMap::new();
    for seed in seeds {
        if supplied.insert(seed.seed_id.clone(), seed).is_some() {
            return Err(PublicError::new(
                ErrorClass::BadRequest,
                "DuplicateLaunchSeedArtifact",
                "seed input was supplied more than once",
            ));
        }
    }

    let mut seed_refs = Vec::with_capacity(runtime_spec.spec().seeds.len());
    for seed_spec in &runtime_spec.spec().seeds {
        let input = supplied.remove(&seed_spec.seed_id).ok_or_else(|| {
            PublicError::new(
                ErrorClass::BadRequest,
                "MissingLaunchSeedArtifact",
                format!("missing seed input for {}", seed_spec.seed_id),
            )
        })?;
        let artifact = launch_artifact(
            input.bytes,
            input.media_type,
            Some(seed_spec.schema_id.clone()),
            Some(seed_spec.semantic_type_id.clone()),
            Some(seed_spec.seed_id.clone()),
            events::ArtifactRole::SeedInput,
        );
        validate_artifact_requirement_for_app(
            store::seed_artifact_requirement(seed_spec, &artifact.evidence)?,
            &artifact.evidence,
            ErrorClass::BadRequest,
            "LaunchSeedArtifactMismatch",
            "seed input artifact metadata does not match the certified spec",
        )?;
        if let Some(required_digest) = &seed_spec.required_digest {
            if &artifact.evidence.digest != required_digest {
                return Err(PublicError::new(
                    ErrorClass::BadRequest,
                    "LaunchSeedDigestMismatch",
                    "seed input digest does not match the certified spec",
                ));
            }
        }
        seed_refs.push(RunLaunchSeedCell {
            bytes: artifact.bytes,
            cell: events::SeedCellRef {
                seed_id: seed_spec.seed_id.clone(),
                cell_id: seed_spec.cell_id.clone(),
                scope_id: seed_spec.scope_id.clone(),
                semantic_type_id: seed_spec.semantic_type_id.clone(),
                schema_id: seed_spec.schema_id.clone(),
                digest: artifact.evidence.digest.clone(),
                seed_artifact: events::ArtifactEvidenceRef {
                    artifact_id: artifact.evidence.artifact_id.clone(),
                    role: artifact.evidence.artifact_role,
                    schema_id: seed_spec.schema_id.clone(),
                    semantic_type_id: artifact.evidence.semantic_type_id.clone(),
                    content_digest: artifact.evidence.digest.clone(),
                    evidence_hash: artifact.evidence.evidence_hash()?,
                    byte_len: artifact.evidence.byte_len,
                    media_type: artifact.evidence.media_type.clone(),
                },
            },
        });
    }
    if !supplied.is_empty() {
        return Err(PublicError::new(
            ErrorClass::BadRequest,
            "UnknownLaunchSeedArtifact",
            "seed input was supplied for a seed not present in the certified spec",
        ));
    }
    Ok(seed_refs)
}

pub(super) fn launch_artifact(
    bytes: Vec<u8>,
    media_type: spec::MediaType,
    schema_id: Option<SchemaId>,
    semantic_type_id: Option<SemanticTypeId>,
    producer_seed_id: Option<SeedId>,
    artifact_role: events::ArtifactRole,
) -> RunLaunchArtifact {
    let byte_len = bytes.len() as u64;
    let digest = content_digest_for_bytes(&bytes);
    RunLaunchArtifact {
        bytes,
        evidence: store::ArtifactEvidenceRef {
            artifact_id: artifact_id_for_digest(&digest),
            digest,
            byte_len,
            media_type,
            schema_id,
            semantic_type_id,
            producer_node_id: None,
            producer_seed_id,
            artifact_role,
        },
    }
}

pub(super) fn content_digest_for_bytes(bytes: &[u8]) -> ContentDigest {
    ContentDigest::from_digest(DigestAlgorithm::Sha256JcsV1, sha256_digest_bytes(bytes))
}

pub(super) fn artifact_id_for_digest(digest: &ContentDigest) -> ArtifactId {
    ArtifactId::from_digest(digest.algorithm(), *digest.digest())
}

pub(super) fn config_input_key(schema_id: &SchemaId, digest: &ContentDigest) -> String {
    format!("{schema_id}:{digest}")
}

pub(super) fn validate_artifact_requirement_for_app(
    requirement: store::EventArtifactRequirement,
    evidence: &store::ArtifactEvidenceRef,
    class: ErrorClass,
    code: &'static str,
    message: &'static str,
) -> Result<(), PublicError> {
    store::validate_artifact_requirement_against_evidence(&requirement, evidence).map_err(|error| {
        match error {
            store::StoreError::ArtifactEvidenceMismatch { .. } => {
                PublicError::new(class, code, message)
            }
            error => error.into(),
        }
    })
}
