use super::*;

pub(super) fn validate_required_artifact_requirement(
    purpose: &'static str,
    requirement: &EventArtifactRequirement,
    evidence: &ArtifactEvidenceRef,
) -> Result<()> {
    validate_artifact_requirement_against_evidence(requirement, evidence).map_err(|error| {
        if let StoreError::ArtifactEvidenceMismatch { field, .. } = error {
            return invalid_prepared_commit_purpose(
                purpose,
                format!(
                    "required artifact {} field {field} does not satisfy payload reference",
                    requirement.artifact_id
                ),
            );
        }
        error
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ArtifactRequirementValidationMode {
    Strict,
    RetentionMetadata,
}

fn artifact_evidence_mismatch(artifact_id: &ArtifactId, field: &'static str) -> StoreError {
    StoreError::ArtifactEvidenceMismatch {
        artifact_id: artifact_id.clone(),
        field,
    }
}

fn require_artifact_option_present(
    artifact_id: &ArtifactId,
    field: &'static str,
    actual: Option<&str>,
) -> Result<()> {
    if actual.is_some() {
        Ok(())
    } else {
        Err(artifact_evidence_mismatch(artifact_id, field))
    }
}

fn require_artifact_option_absent(
    artifact_id: &ArtifactId,
    field: &'static str,
    actual: Option<&str>,
) -> Result<()> {
    if actual.is_none() {
        Ok(())
    } else {
        Err(artifact_evidence_mismatch(artifact_id, field))
    }
}

fn validate_artifact_requirement_exact_fields(
    requirement: &EventArtifactRequirement,
    evidence: &ArtifactEvidenceRef,
) -> Result<()> {
    if let Some(schema_id) = &requirement.schema_id {
        compare_artifact_option(
            &requirement.artifact_id,
            "schema_id",
            evidence.schema_id.as_ref().map(SchemaId::as_str),
            Some(schema_id.as_str()),
        )?;
    }
    if let Some(semantic_type_id) = &requirement.semantic_type_id {
        compare_artifact_option(
            &requirement.artifact_id,
            "semantic_type_id",
            evidence
                .semantic_type_id
                .as_ref()
                .map(SemanticTypeId::as_str),
            Some(semantic_type_id.as_str()),
        )?;
    }
    if let Some(producer_node_id) = &requirement.producer_node_id {
        compare_artifact_option(
            &requirement.artifact_id,
            "producer_node_id",
            evidence.producer_node_id.as_ref().map(NodeId::as_str),
            Some(producer_node_id.as_str()),
        )?;
    }
    if let Some(producer_seed_id) = &requirement.producer_seed_id {
        compare_artifact_option(
            &requirement.artifact_id,
            "producer_seed_id",
            evidence.producer_seed_id.as_ref().map(SeedId::as_str),
            Some(producer_seed_id.as_str()),
        )?;
    }
    Ok(())
}

fn validate_artifact_option_policy(
    requirement: &EventArtifactRequirement,
    field: &'static str,
    expected: Option<&str>,
    actual: Option<&str>,
    requires_value: bool,
    mode: ArtifactRequirementValidationMode,
) -> Result<()> {
    if requires_value {
        if let Some(expected) = expected {
            compare_artifact_option(&requirement.artifact_id, field, actual, Some(expected))
        } else if mode == ArtifactRequirementValidationMode::RetentionMetadata {
            require_artifact_option_present(&requirement.artifact_id, field, actual)
        } else {
            Err(artifact_evidence_mismatch(&requirement.artifact_id, field))
        }
    } else {
        if expected.is_some() {
            return Err(artifact_evidence_mismatch(&requirement.artifact_id, field));
        }
        require_artifact_option_absent(&requirement.artifact_id, field, actual)
    }
}

fn validate_artifact_schema_policy(
    requirement: &EventArtifactRequirement,
    evidence: &ArtifactEvidenceRef,
    policy: events::ArtifactSchemaPolicy,
    mode: ArtifactRequirementValidationMode,
) -> Result<()> {
    validate_artifact_option_policy(
        requirement,
        "schema_id",
        requirement.schema_id.as_ref().map(SchemaId::as_str),
        evidence.schema_id.as_ref().map(SchemaId::as_str),
        match policy {
            events::ArtifactSchemaPolicy::ExactSeedSchema
            | events::ArtifactSchemaPolicy::ExactValueSchema
            | events::ArtifactSchemaPolicy::ExactEvidenceSchema
            | events::ArtifactSchemaPolicy::ExactFactDescriptorSchema
            | events::ArtifactSchemaPolicy::ExactFactQueryEvidenceSchema
            | events::ArtifactSchemaPolicy::ExactPublicSchema
            | events::ArtifactSchemaPolicy::ExactDiagnosticSchema => true,
            events::ArtifactSchemaPolicy::Absent => false,
        },
        mode,
    )
}

fn validate_artifact_semantic_policy(
    requirement: &EventArtifactRequirement,
    evidence: &ArtifactEvidenceRef,
    policy: events::ArtifactSemanticPolicy,
    mode: ArtifactRequirementValidationMode,
) -> Result<()> {
    validate_artifact_option_policy(
        requirement,
        "semantic_type_id",
        requirement
            .semantic_type_id
            .as_ref()
            .map(SemanticTypeId::as_str),
        evidence
            .semantic_type_id
            .as_ref()
            .map(SemanticTypeId::as_str),
        match policy {
            events::ArtifactSemanticPolicy::ExactSeedSemantic
            | events::ArtifactSemanticPolicy::ExactValueSemantic => true,
            events::ArtifactSemanticPolicy::Absent => false,
        },
        mode,
    )
}

fn require_producer_node_absent(
    requirement: &EventArtifactRequirement,
    evidence: &ArtifactEvidenceRef,
) -> Result<()> {
    if requirement.producer_node_id.is_some() {
        return Err(artifact_evidence_mismatch(
            &requirement.artifact_id,
            "producer_node_id",
        ));
    }
    require_artifact_option_absent(
        &requirement.artifact_id,
        "producer_node_id",
        evidence.producer_node_id.as_ref().map(NodeId::as_str),
    )
}

fn require_producer_seed_absent(
    requirement: &EventArtifactRequirement,
    evidence: &ArtifactEvidenceRef,
) -> Result<()> {
    if requirement.producer_seed_id.is_some() {
        return Err(artifact_evidence_mismatch(
            &requirement.artifact_id,
            "producer_seed_id",
        ));
    }
    require_artifact_option_absent(
        &requirement.artifact_id,
        "producer_seed_id",
        evidence.producer_seed_id.as_ref().map(SeedId::as_str),
    )
}

fn require_producer_node_exact_or_present(
    requirement: &EventArtifactRequirement,
    evidence: &ArtifactEvidenceRef,
    mode: ArtifactRequirementValidationMode,
) -> Result<()> {
    let actual = evidence.producer_node_id.as_ref().map(NodeId::as_str);
    if let Some(producer_node_id) = &requirement.producer_node_id {
        compare_artifact_option(
            &requirement.artifact_id,
            "producer_node_id",
            actual,
            Some(producer_node_id.as_str()),
        )
    } else if mode == ArtifactRequirementValidationMode::RetentionMetadata {
        require_artifact_option_present(&requirement.artifact_id, "producer_node_id", actual)
    } else {
        Err(artifact_evidence_mismatch(
            &requirement.artifact_id,
            "producer_node_id",
        ))
    }
}

fn require_producer_seed_exact_or_present(
    requirement: &EventArtifactRequirement,
    evidence: &ArtifactEvidenceRef,
    mode: ArtifactRequirementValidationMode,
) -> Result<()> {
    let actual = evidence.producer_seed_id.as_ref().map(SeedId::as_str);
    if let Some(producer_seed_id) = &requirement.producer_seed_id {
        compare_artifact_option(
            &requirement.artifact_id,
            "producer_seed_id",
            actual,
            Some(producer_seed_id.as_str()),
        )
    } else if mode == ArtifactRequirementValidationMode::RetentionMetadata {
        require_artifact_option_present(&requirement.artifact_id, "producer_seed_id", actual)
    } else {
        Err(artifact_evidence_mismatch(
            &requirement.artifact_id,
            "producer_seed_id",
        ))
    }
}

fn validate_optional_producer_node_no_seed(
    requirement: &EventArtifactRequirement,
    evidence: &ArtifactEvidenceRef,
) -> Result<()> {
    require_producer_seed_absent(requirement, evidence)?;
    if let Some(producer_node_id) = &requirement.producer_node_id {
        compare_artifact_option(
            &requirement.artifact_id,
            "producer_node_id",
            evidence.producer_node_id.as_ref().map(NodeId::as_str),
            Some(producer_node_id.as_str()),
        )?;
    }
    Ok(())
}

fn validate_artifact_producer_policy(
    requirement: &EventArtifactRequirement,
    evidence: &ArtifactEvidenceRef,
    policy: events::ArtifactProducerScope,
    mode: ArtifactRequirementValidationMode,
) -> Result<()> {
    match policy {
        events::ArtifactProducerScope::LaunchOrGlobalNoSeed
        | events::ArtifactProducerScope::DiagnosticOptionalNodeNoSeed => {
            validate_optional_producer_node_no_seed(requirement, evidence)?;
        }
        events::ArtifactProducerScope::SeedRequired => {
            require_producer_node_absent(requirement, evidence)?;
            require_producer_seed_exact_or_present(requirement, evidence, mode)?;
        }
        events::ArtifactProducerScope::NodeRequired => {
            require_producer_seed_absent(requirement, evidence)?;
            require_producer_node_exact_or_present(requirement, evidence, mode)?;
        }
        events::ArtifactProducerScope::GlobalNoSeed
        | events::ArtifactProducerScope::MiddlewareNoSeed => {
            require_producer_node_absent(requirement, evidence)?;
            require_producer_seed_absent(requirement, evidence)?;
        }
    }
    Ok(())
}

fn validate_artifact_role_contract(
    requirement: &EventArtifactRequirement,
    evidence: &ArtifactEvidenceRef,
    role: ArtifactRole,
    mode: ArtifactRequirementValidationMode,
) -> Result<()> {
    let contract = role.contract();
    validate_artifact_schema_policy(requirement, evidence, contract.schema, mode)?;
    validate_artifact_semantic_policy(requirement, evidence, contract.semantic, mode)?;
    validate_artifact_producer_policy(requirement, evidence, contract.producer, mode)
}

/// Validates a typed event artifact requirement against retained artifact evidence.
///
/// This applies the closed [`events::ArtifactRole`] contract for role-bearing requirements and
/// exact carried-field matching for schema-only requirements.
pub fn validate_artifact_requirement_against_evidence(
    requirement: &EventArtifactRequirement,
    evidence: &ArtifactEvidenceRef,
) -> Result<()> {
    let mode = if requirement.source.is_retention() {
        ArtifactRequirementValidationMode::RetentionMetadata
    } else {
        ArtifactRequirementValidationMode::Strict
    };
    compare_artifact_field(
        &requirement.artifact_id,
        "artifact_id",
        evidence.artifact_id.as_str(),
        requirement.artifact_id.as_str(),
    )?;
    compare_artifact_field(
        &requirement.artifact_id,
        "evidence_hash",
        evidence.evidence_hash()?.as_str(),
        requirement.evidence_hash.as_str(),
    )?;
    if let Some(digest) = &requirement.digest {
        compare_artifact_field(
            &requirement.artifact_id,
            "digest",
            evidence.digest.as_str(),
            digest.as_str(),
        )?;
    }
    if let Some(byte_len) = requirement.byte_len {
        compare_artifact_field(
            &requirement.artifact_id,
            "byte_len",
            evidence.byte_len,
            byte_len,
        )?;
    }
    if let Some(media_type) = &requirement.media_type {
        compare_artifact_field(
            &requirement.artifact_id,
            "media_type",
            evidence.media_type.as_str(),
            media_type.as_str(),
        )?;
    }
    if let Some(role) = requirement.artifact_role {
        compare_artifact_field(
            &requirement.artifact_id,
            "artifact_role",
            evidence.artifact_role.as_str(),
            role.as_str(),
        )?;
        validate_artifact_role_contract(requirement, evidence, role, mode)?;
    } else {
        validate_artifact_requirement_exact_fields(requirement, evidence)?;
    }
    Ok(())
}

pub(crate) fn verify_retained_artifact_bytes(
    bytes: &[u8],
    evidence: &ArtifactEvidenceRef,
) -> Result<()> {
    let digest =
        ContentDigest::from_digest(DigestAlgorithm::Sha256JcsV1, sha256_digest_bytes(bytes));
    let artifact_id = ArtifactId::from_digest(digest.algorithm(), *digest.digest());
    if evidence.digest != digest
        || evidence.artifact_id != artifact_id
        || evidence.byte_len != bytes.len() as u64
    {
        return Err(StoreError::ArtifactEvidenceMismatch {
            artifact_id: evidence.artifact_id.clone(),
            field: "bytes",
        });
    }
    Ok(())
}
