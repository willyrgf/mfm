use super::*;

pub(super) fn validate_unique_artifact_evidence(
    field: &'static str,
    artifacts: &[ArtifactEvidenceRef],
) -> Result<()> {
    let mut by_artifact = BTreeMap::<ArtifactAuthorityKey, &ArtifactEvidenceRef>::new();
    for artifact in artifacts {
        let key = artifact_authority_key(artifact)?;
        if let Some(existing) = by_artifact.insert(key, artifact) {
            if existing != artifact {
                return Err(StoreError::ArtifactEvidenceMismatch {
                    artifact_id: artifact.artifact_id.clone(),
                    field,
                });
            }
        }
    }
    Ok(())
}

pub(super) fn validate_required_artifacts_cover_payload_references(
    purpose: &'static str,
    request: &CommitRequest,
) -> Result<()> {
    for payload in &request.payloads {
        for requirement in event_artifact_requirements(payload) {
            let Some(evidence) = request.required_artifacts.iter().find(|evidence| {
                evidence.artifact_id == requirement.artifact_id
                    && evidence.evidence_hash().ok().as_ref() == Some(&requirement.evidence_hash)
            }) else {
                return Err(invalid_prepared_commit_purpose(
                    purpose,
                    format!(
                        "missing required artifact evidence for {}",
                        requirement.artifact_id
                    ),
                ));
            };
            validate_required_artifact_requirement(purpose, &requirement, evidence)?;
        }
    }
    Ok(())
}

pub(super) fn reject_store_materialized_resource_lane_payloads(
    purpose: &'static str,
    request: &CommitRequest,
) -> Result<()> {
    if request.payloads.iter().any(|payload| {
        matches!(
            payload,
            KernelEventPayload::ResourceLaneClaimed(_)
                | KernelEventPayload::ResourceLaneReleased(_)
        )
    }) {
        return Err(invalid_prepared_commit_purpose(
            purpose,
            "prepared commits must use resource-lane intents, not store-filled lane events",
        ));
    }
    Ok(())
}

fn validate_required_artifact_requirement(
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

pub(super) fn verify_retained_artifact_bytes(
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

pub(super) fn validate_payload_public_diagnostics(payloads: &[KernelEventPayload]) -> Result<()> {
    for payload in payloads {
        match payload {
            KernelEventPayload::PublicOutputRenderFailed(payload) => payload.error.validate()?,
            KernelEventPayload::StateAttemptFailed(payload) => payload.error.validate()?,
            KernelEventPayload::SideEffectFailed(payload) => payload.error.validate()?,
            _ => {}
        }
    }
    Ok(())
}

pub(super) fn validate_run_start_commit(request: &CommitRequest) -> Result<()> {
    if request.payloads.len() != 1
        || !matches!(
            request.payloads.first(),
            Some(KernelEventPayload::RunAdmitted(_))
        )
    {
        return Err(invalid_prepared_commit_purpose(
            RunAdmission::NAME,
            "run-admission commits must contain exactly one RunAdmitted payload",
        ));
    }
    let Some(KernelEventPayload::RunAdmitted(payload)) = request.payloads.first() else {
        unreachable!("run-admission payload shape was checked above");
    };
    let authority = request
        .preconditions
        .certified_run_authority
        .as_ref()
        .ok_or_else(|| {
            invalid_prepared_commit_purpose(
                RunAdmission::NAME,
                "run admission requires certified run store authority",
            )
        })?;
    if authority.run_id() != request.run_id() || authority.spec_hash() != &payload.spec_hash {
        return Err(invalid_prepared_commit_purpose(
            RunAdmission::NAME,
            "certified run store authority does not match RunAdmitted",
        ));
    }
    validate_run_admitted_identity_for_request(request.run_id(), payload)?;
    if request.preconditions.required_run_state != RequiredRunState::Absent {
        return Err(invalid_prepared_commit_purpose(
            RunAdmission::NAME,
            "run admission requires absent-run precondition",
        ));
    }
    Ok(())
}

fn validate_run_admitted_identity_for_request(
    run_id: &RunId,
    payload: &events::RunAdmitted,
) -> Result<()> {
    if payload.run_id != *run_id {
        return Err(invalid_prepared_commit_purpose(
            RunAdmission::NAME,
            "RunAdmitted run id does not match commit run id",
        ));
    }
    if payload.identity_material.certified_spec_hash != payload.spec_hash {
        return Err(invalid_prepared_commit_purpose(
            RunAdmission::NAME,
            "RunAdmitted identity material spec hash does not match event spec hash",
        ));
    }
    let derived = payload.identity_material.derive_run_id().map_err(|_| {
        invalid_prepared_commit_purpose(
            RunAdmission::NAME,
            "RunAdmitted identity material is invalid",
        )
    })?;
    if derived != payload.run_id {
        return Err(invalid_prepared_commit_purpose(
            RunAdmission::NAME,
            "RunAdmitted run id does not match identity material",
        ));
    }
    Ok(())
}

pub(super) fn validate_state_attempt_started_commit(request: &CommitRequest) -> Result<()> {
    if request.payloads.len() != 1
        || !matches!(
            request.payloads.first(),
            Some(KernelEventPayload::StateAttemptStarted(_))
        )
    {
        return Err(invalid_prepared_commit_purpose(
            StateAttemptStarted::NAME,
            "state-attempt-start commits must contain exactly one StateAttemptStarted payload",
        ));
    }
    if request.preconditions.required_run_state != RequiredRunState::NotCompleted {
        return Err(invalid_prepared_commit_purpose(
            StateAttemptStarted::NAME,
            "state-attempt-start requires not-completed run precondition",
        ));
    }
    Ok(())
}

pub(super) fn validate_attempt_terminal_commit(request: &CommitRequest) -> Result<()> {
    reject_wrong_purpose_payloads(
        AttemptTerminal::NAME,
        request,
        is_attempt_terminal_commit_payload,
        "attempt-terminal commits cannot contain non-attempt-terminal payloads",
    )?;
    require_purpose_payload(
        AttemptTerminal::NAME,
        request,
        is_attempt_terminal_payload,
        "missing attempt-terminal payload",
    )?;
    validate_attempt_terminal_resource_lane_release_batch(request)?;
    validate_terminal_attempt_cell_pairs(&request.payloads)
}

pub(super) fn validate_side_effect_terminal_commit(request: &CommitRequest) -> Result<()> {
    reject_wrong_purpose_payloads(
        SideEffectTerminal::NAME,
        request,
        is_side_effect_terminal_commit_payload,
        "side-effect terminal commits cannot contain non-side-effect-terminal payloads",
    )?;
    require_purpose_payload(
        SideEffectTerminal::NAME,
        request,
        is_side_effect_terminal_disposition_payload,
        "missing side-effect terminal payload",
    )?;
    if request.preconditions.certified_run_authority.is_none() {
        return Err(invalid_prepared_commit_purpose(
            SideEffectTerminal::NAME,
            "side-effect terminal commits require certified run authority",
        ));
    }
    validate_side_effect_terminal_resource_lane_release_batch(request)?;
    validate_terminal_attempt_cell_pairs(&request.payloads)
}

pub(super) fn validate_side_effect_progress_commit(request: &CommitRequest) -> Result<()> {
    reject_wrong_purpose_payloads(
        SideEffectProgress::NAME,
        request,
        is_side_effect_progress_commit_payload,
        "side-effect progress commits cannot contain non-side-effect-progress payloads",
    )?;
    require_purpose_payload(
        SideEffectProgress::NAME,
        request,
        is_side_effect_payload,
        "missing side-effect payload",
    )?;
    if request.preconditions.certified_run_authority.is_none() {
        return Err(invalid_prepared_commit_purpose(
            SideEffectProgress::NAME,
            "side-effect progress commits require certified run authority",
        ));
    }
    Ok(())
}

pub(super) fn validate_retention_commit(request: &CommitRequest) -> Result<()> {
    reject_wrong_purpose_payloads(
        Retention::NAME,
        request,
        is_retention_commit_payload,
        "retention commits cannot contain non-retention payloads",
    )?;
    require_purpose_payload(
        Retention::NAME,
        request,
        is_retention_payload,
        "missing retention payload",
    )?;
    validate_terminal_attempt_cell_pairs(&request.payloads)
}

fn validate_manual_resolution_commit(request: &CommitRequest) -> Result<()> {
    let manual_resolution_count = request
        .payloads
        .iter()
        .filter(|payload| matches!(payload, KernelEventPayload::ManualResolutionRecorded(_)))
        .count();
    if manual_resolution_count != 1 {
        return Err(invalid_prepared_commit_purpose(
            ManualResolution::NAME,
            "manual resolution commits must contain exactly one ManualResolutionRecorded payload",
        ));
    }
    if !matches!(
        request.payloads.last(),
        Some(KernelEventPayload::ManualResolutionRecorded(_))
    ) {
        return Err(invalid_prepared_commit_purpose(
            ManualResolution::NAME,
            "manual resolution release intents must precede ManualResolutionRecorded",
        ));
    }
    if request.payloads.iter().any(|payload| {
        !matches!(
            payload,
            KernelEventPayload::ManualResolutionRecorded(_)
                | KernelEventPayload::ResourceLaneReleaseIntent(_)
        )
    }) {
        return Err(invalid_prepared_commit_purpose(
            ManualResolution::NAME,
            "manual resolution commits may only contain resource lane release intents and ManualResolutionRecorded",
        ));
    }
    if request.payloads.iter().any(|payload| {
        matches!(
            payload,
            KernelEventPayload::ResourceLaneReleaseIntent(events::ResourceLaneReleaseIntent {
                release_authority,
                ..
            }) if *release_authority != events::ResourceLaneReleaseAuthority::ManualResolution
        )
    }) {
        return Err(invalid_prepared_commit_purpose(
            ManualResolution::NAME,
            "manual resolution release intents require manual resolution release authority",
        ));
    }
    if request.preconditions.required_run_state != RequiredRunState::NotCompleted {
        return Err(invalid_prepared_commit_purpose(
            ManualResolution::NAME,
            "manual resolution requires not-completed run precondition",
        ));
    }
    if request.preconditions.certified_run_authority.is_none() {
        return Err(invalid_prepared_commit_purpose(
            ManualResolution::NAME,
            "manual resolution requires certified run authority",
        ));
    }
    Ok(())
}

pub(super) fn validate_manual_resolution_commit_with_proof(
    request: &CommitRequest,
    proof: &VerifiedManualResolutionForPrefix,
) -> Result<()> {
    validate_manual_resolution_commit(request)?;
    let manual_resolutions = request
        .payloads
        .iter()
        .filter_map(|payload| match payload {
            KernelEventPayload::ManualResolutionRecorded(payload) => Some(payload),
            _ => None,
        })
        .collect::<Vec<_>>();
    if manual_resolutions.len() != 1 {
        return Err(invalid_prepared_commit_purpose(
            ManualResolution::NAME,
            "manual resolution requires exactly one ManualResolutionRecorded payload",
        ));
    }
    let payload = manual_resolutions[0];
    let token = request
        .preconditions
        .certified_run_authority
        .as_ref()
        .ok_or_else(|| {
            invalid_prepared_commit_purpose(
                ManualResolution::NAME,
                "manual resolution requires certified run authority",
            )
        })?;
    let prefix = proof.prefix();
    if prefix.run_id() != request.run_id()
        || prefix.run_id() != &payload.run_id
        || prefix.run_id() != token.run_id()
    {
        return Err(invalid_prepared_commit_purpose(
            ManualResolution::NAME,
            "manual proof run id does not match manual resolution request",
        ));
    }
    if prefix.spec_hash() != &payload.spec_hash || prefix.spec_hash() != token.spec_hash() {
        return Err(invalid_prepared_commit_purpose(
            ManualResolution::NAME,
            "manual proof spec hash does not match manual resolution request",
        ));
    }
    if prefix.expected_next_seq() != request.expected_next_seq().as_u64() {
        return Err(invalid_prepared_commit_purpose(
            ManualResolution::NAME,
            "manual proof prefix expected_next_seq does not match manual resolution request",
        ));
    }
    if proof.outcome() != payload.outcome {
        return Err(invalid_prepared_commit_purpose(
            ManualResolution::NAME,
            "ManualResolutionRecorded outcome does not match manual proof",
        ));
    }
    if proof.evidence().schema_id != payload.evidence_schema_id
        || proof.evidence().content_hash != payload.evidence_hash
        || proof.evidence().artifact_id != payload.evidence_artifact_id
        || proof.authorization().schema_id != payload.authorization_schema_id
        || proof.authorization().content_hash != payload.authorization_hash
        || proof.authorization().artifact_id != payload.authorization_artifact_id
    {
        return Err(invalid_prepared_commit_purpose(
            ManualResolution::NAME,
            "ManualResolutionRecorded artifact refs do not match manual proof",
        ));
    }
    let block_reason = manual_block_reason_from_auth(prefix.manual_block_reason());
    let certified_manual_policy = manual_policy_for_block_reason(token.saga_policy(), block_reason)
        .ok_or_else(|| {
            invalid_prepared_commit_purpose(
                ManualResolution::NAME,
                "certified run authority policy does not permit manual proof block reason",
            )
        })?;
    if certified_manual_policy != prefix.manual_policy() {
        return Err(invalid_prepared_commit_purpose(
            ManualResolution::NAME,
            "manual proof policy does not match certified run authority",
        ));
    }
    Ok(())
}

fn validate_saga_terminal_commit(request: &CommitRequest) -> Result<()> {
    reject_wrong_purpose_payloads(
        SagaTerminal::NAME,
        request,
        is_saga_terminal_commit_payload,
        "saga terminal commits cannot contain non-terminal payloads",
    )?;
    if request
        .payloads
        .iter()
        .filter(|payload| is_run_completed_payload(payload))
        .count()
        != 1
    {
        return Err(invalid_prepared_commit_purpose(
            SagaTerminal::NAME,
            "saga terminal resolution commits must contain exactly one RunCompleted payload",
        ));
    }
    if request.preconditions.certified_run_authority.is_none() {
        return Err(invalid_prepared_commit_purpose(
            SagaTerminal::NAME,
            "saga terminal resolution requires certified run authority",
        ));
    }
    validate_terminal_attempt_cell_pairs(&request.payloads)
}

pub(super) fn validate_saga_terminal_commit_with_proof(
    request: &CommitRequest,
    proof: &SagaTerminalProof,
) -> Result<()> {
    validate_saga_terminal_commit(request)?;
    let completed = request
        .payloads
        .iter()
        .filter_map(|payload| match payload {
            KernelEventPayload::RunCompleted(payload) => Some(payload),
            _ => None,
        })
        .collect::<Vec<_>>();
    if completed.len() != 1 {
        return Err(invalid_prepared_commit_purpose(
            SagaTerminal::NAME,
            "saga terminal resolution requires exactly one RunCompleted payload",
        ));
    }
    let payload = completed[0];
    let token = request
        .preconditions
        .certified_run_authority
        .as_ref()
        .ok_or_else(|| {
            invalid_prepared_commit_purpose(
                SagaTerminal::NAME,
                "saga terminal resolution requires certified run authority",
            )
        })?;
    if proof.run_id() != request.run_id()
        || proof.run_id() != &payload.run_id
        || proof.run_id() != token.run_id()
    {
        return Err(invalid_prepared_commit_purpose(
            SagaTerminal::NAME,
            "SagaTerminalProof run id does not match terminal request",
        ));
    }
    if proof.prefix_next_seq() != request.expected_next_seq() {
        return Err(invalid_prepared_commit_purpose(
            SagaTerminal::NAME,
            "SagaTerminalProof prefix does not match terminal request expected next sequence",
        ));
    }
    if proof.saga_policy_digest() != token.saga_policy_digest() {
        return Err(invalid_prepared_commit_purpose(
            SagaTerminal::NAME,
            "SagaTerminalProof saga policy digest does not match certified run authority",
        ));
    }
    let proof_outcome = proof.outcome();
    if payload.outcome != proof_outcome {
        return Err(invalid_prepared_commit_purpose(
            SagaTerminal::NAME,
            "RunCompleted outcome does not match SagaTerminalProof",
        ));
    }
    if matches!(payload.outcome, events::RunCompletionOutcome::Completed(_)) {
        return Err(invalid_prepared_commit_purpose(
            SagaTerminal::NAME,
            "completed public-output terminal must use CompleteRun authority",
        ));
    }
    if let Some(spec_hash) = proof.manual_spec_hash() {
        if spec_hash != &payload.spec_hash {
            return Err(invalid_prepared_commit_purpose(
                SagaTerminal::NAME,
                "manual proof spec hash does not match RunCompleted payload",
            ));
        }
    }
    Ok(())
}

fn require_purpose_payload(
    purpose: &'static str,
    request: &CommitRequest,
    predicate: impl Fn(&KernelEventPayload) -> bool,
    message: &'static str,
) -> Result<()> {
    if request.payloads.iter().any(predicate) {
        Ok(())
    } else {
        Err(invalid_prepared_commit_purpose(purpose, message))
    }
}

fn reject_wrong_purpose_payloads(
    purpose: &'static str,
    request: &CommitRequest,
    predicate: impl Fn(&KernelEventPayload) -> bool,
    message: &'static str,
) -> Result<()> {
    if let Some(payload) = request.payloads.iter().find(|payload| !predicate(payload)) {
        Err(invalid_prepared_commit_purpose(
            purpose,
            format!("{message}: {:?}", payload.event_schema_id()),
        ))
    } else {
        Ok(())
    }
}

fn validate_attempt_terminal_resource_lane_release_batch(request: &CommitRequest) -> Result<()> {
    reject_terminal_resource_lane_claims(AttemptTerminal::NAME, request)?;
    for (release_index, release) in request
        .payloads
        .iter()
        .enumerate()
        .filter(|(_, payload)| is_resource_lane_release_payload(payload))
    {
        let Some(release_ref) = release.resource_lane_authority_ref() else {
            continue;
        };
        let matched_terminal =
            request
                .payloads
                .iter()
                .enumerate()
                .any(|(terminal_index, terminal)| {
                    terminal_index > release_index
                        && terminal_payload_matches_resource_lane_release(
                            terminal,
                            &release_ref,
                            TerminalReleaseMatchKind::AttemptTerminal,
                            None,
                        )
                });
        if !matched_terminal {
            return Err(invalid_prepared_commit_purpose(
                AttemptTerminal::NAME,
                "resource lane release must precede a matching terminal attempt payload",
            ));
        }
    }
    Ok(())
}

fn validate_side_effect_terminal_resource_lane_release_batch(
    request: &CommitRequest,
) -> Result<()> {
    reject_terminal_resource_lane_claims(SideEffectTerminal::NAME, request)?;
    let authority = request
        .preconditions
        .certified_run_authority
        .as_ref()
        .ok_or_else(|| {
            invalid_prepared_commit_purpose(
                SideEffectTerminal::NAME,
                "side-effect terminal resource-lane release requires certified run authority",
            )
        })?;
    for (release_index, release) in request
        .payloads
        .iter()
        .enumerate()
        .filter(|(_, payload)| is_resource_lane_release_payload(payload))
    {
        let Some(release_ref) = release.resource_lane_authority_ref() else {
            continue;
        };
        let matched_terminal =
            request
                .payloads
                .iter()
                .enumerate()
                .any(|(terminal_index, terminal)| {
                    terminal_index > release_index
                        && terminal_payload_matches_resource_lane_release(
                            terminal,
                            &release_ref,
                            TerminalReleaseMatchKind::SideEffectTerminal,
                            Some(authority),
                        )
                });
        if !matched_terminal {
            return Err(invalid_prepared_commit_purpose(
                SideEffectTerminal::NAME,
                "resource lane release must precede a matching side-effect terminal payload",
            ));
        }
    }
    Ok(())
}

fn reject_terminal_resource_lane_claims(
    purpose: &'static str,
    request: &CommitRequest,
) -> Result<()> {
    if request.payloads.iter().any(is_resource_lane_claim_payload) {
        return Err(invalid_prepared_commit_purpose(
            purpose,
            "terminal commits cannot acquire resource lanes",
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TerminalReleaseMatchKind {
    AttemptTerminal,
    SideEffectTerminal,
}

fn terminal_payload_matches_resource_lane_release(
    terminal: &KernelEventPayload,
    release: &events::ResourceLaneAuthorityRef<'_>,
    kind: TerminalReleaseMatchKind,
    authority: Option<&CertifiedRunStoreAuthority>,
) -> bool {
    match kind {
        TerminalReleaseMatchKind::AttemptTerminal => {
            attempt_terminal_payload_matches_release(terminal, release)
        }
        TerminalReleaseMatchKind::SideEffectTerminal => {
            let Some(authority) = authority else {
                return false;
            };
            side_effect_terminal_payload_matches_release(terminal, release, authority)
        }
    }
}

fn attempt_terminal_payload_matches_release(
    terminal: &KernelEventPayload,
    release: &events::ResourceLaneAuthorityRef<'_>,
) -> bool {
    let Some(emitter) = release.emitter else {
        return false;
    };
    match terminal {
        KernelEventPayload::StateAttemptCompleted(payload) => {
            payload.node_id == *emitter.node_id && payload.attempt_id == *emitter.attempt_id
        }
        KernelEventPayload::StateAttemptInterrupted(payload) => {
            payload.node_id == *emitter.node_id && payload.attempt_id == *emitter.attempt_id
        }
        KernelEventPayload::StateAttemptFailed(payload) => {
            payload.node_id == *emitter.node_id && payload.attempt_id == *emitter.attempt_id
        }
        _ => false,
    }
}

fn side_effect_terminal_payload_matches_release(
    terminal: &KernelEventPayload,
    release: &events::ResourceLaneAuthorityRef<'_>,
    authority: &CertifiedRunStoreAuthority,
) -> bool {
    if release.release_authority != Some(events::ResourceLaneReleaseAuthority::VerifyTerminal) {
        return false;
    }
    if !is_side_effect_terminal_disposition_payload(terminal) {
        return false;
    }
    let Some(terminal) = terminal.side_effect_ledger_ref() else {
        return false;
    };
    terminal.ledger_key == release.ledger.ledger_key
        && terminal.ledger_purpose == release.ledger.ledger_purpose
        && terminal.pair_id == release.ledger.pair_id
        && terminal.invocation_epoch == release.ledger.invocation_epoch
        && side_effect_terminal_release_role_allowed(
            terminal.kind,
            terminal.pair_role,
            release.ledger.pair_role,
        )
        && side_effect_terminal_release_policy_allowed(terminal.kind, terminal.pair_id, authority)
}

fn side_effect_terminal_release_policy_allowed(
    terminal_kind: events::SideEffectEventKind,
    pair_id: &SideEffectPairId,
    authority: &CertifiedRunStoreAuthority,
) -> bool {
    let Ok(pair) = authority.side_effect_pair(pair_id) else {
        return false;
    };
    match terminal_kind {
        events::SideEffectEventKind::ReceiptObserved => {
            pair.terminal_policy == SideEffectTerminalPolicy::Receipt
        }
        events::SideEffectEventKind::ConfirmationObserved => true,
        events::SideEffectEventKind::NotSubmittedProven | events::SideEffectEventKind::Failed => {
            true
        }
        _ => false,
    }
}

fn side_effect_terminal_release_role_allowed(
    terminal_kind: events::SideEffectEventKind,
    terminal_role: events::SideEffectPairRole,
    release_role: events::SideEffectPairRole,
) -> bool {
    if release_role != events::SideEffectPairRole::Verify {
        return false;
    }
    match terminal_kind {
        events::SideEffectEventKind::NotSubmittedProven => {
            matches!(
                terminal_role,
                events::SideEffectPairRole::Submit | events::SideEffectPairRole::Verify
            )
        }
        events::SideEffectEventKind::ReceiptObserved
        | events::SideEffectEventKind::ConfirmationObserved => {
            terminal_role == events::SideEffectPairRole::Verify
        }
        events::SideEffectEventKind::Failed => matches!(
            terminal_role,
            events::SideEffectPairRole::Submit | events::SideEffectPairRole::Verify
        ),
        _ => false,
    }
}

fn is_attempt_terminal_payload(payload: &KernelEventPayload) -> bool {
    matches!(
        payload,
        KernelEventPayload::StateAttemptCompleted(_)
            | KernelEventPayload::StateAttemptInterrupted(_)
            | KernelEventPayload::StateAttemptFailed(_)
            | KernelEventPayload::CellProduced(_)
            | KernelEventPayload::CellSkipped(_)
            | KernelEventPayload::FactRecorded(_)
            | KernelEventPayload::ArtifactReferenced(_)
            | KernelEventPayload::PublicOutputProduced(_)
            | KernelEventPayload::PublicOutputRenderFailed(_)
            | KernelEventPayload::RunCompleted(_)
    )
}

fn is_attempt_terminal_commit_payload(payload: &KernelEventPayload) -> bool {
    is_attempt_terminal_payload(payload)
        || matches!(
            payload,
            KernelEventPayload::ResourceLaneReleased(_)
                | KernelEventPayload::ResourceLaneReleaseIntent(_)
        )
        || is_retention_ref_payload(payload)
}

fn is_side_effect_terminal_payload(payload: &KernelEventPayload) -> bool {
    matches!(
        payload,
        KernelEventPayload::SideEffectNotSubmittedProven(_)
            | KernelEventPayload::SideEffectSubmissionObserved(_)
            | KernelEventPayload::SideEffectSubmissionUnknown(_)
            | KernelEventPayload::SideEffectReceiptObserved(_)
            | KernelEventPayload::SideEffectConfirmationObserved(_)
            | KernelEventPayload::SideEffectAmbiguous(_)
            | KernelEventPayload::SideEffectFailed(_)
            | KernelEventPayload::ResourceLaneReleased(_)
            | KernelEventPayload::ResourceLaneReleaseIntent(_)
    )
}

fn is_side_effect_terminal_disposition_payload(payload: &KernelEventPayload) -> bool {
    is_side_effect_terminal_payload(payload) && !is_resource_lane_release_payload(payload)
}

fn is_side_effect_terminal_commit_payload(payload: &KernelEventPayload) -> bool {
    is_side_effect_payload(payload)
        || is_attempt_terminal_payload(payload)
        || is_retention_ref_payload(payload)
}

fn is_side_effect_progress_commit_payload(payload: &KernelEventPayload) -> bool {
    (is_side_effect_payload(payload) && !is_side_effect_terminal_payload(payload))
        || is_retention_ref_payload(payload)
}

fn is_side_effect_payload(payload: &KernelEventPayload) -> bool {
    payload.side_effect_ledger_ref().is_some()
}

fn is_resource_lane_claim_payload(payload: &KernelEventPayload) -> bool {
    matches!(
        payload,
        KernelEventPayload::ResourceLaneClaimed(_) | KernelEventPayload::ResourceLaneClaimIntent(_)
    )
}

fn is_resource_lane_release_payload(payload: &KernelEventPayload) -> bool {
    matches!(
        payload,
        KernelEventPayload::ResourceLaneReleased(_)
            | KernelEventPayload::ResourceLaneReleaseIntent(_)
    )
}

fn is_retention_payload(payload: &KernelEventPayload) -> bool {
    matches!(
        payload,
        KernelEventPayload::RetentionRefsAppended(_)
            | KernelEventPayload::RetentionManifestProjected(_)
    )
}

fn is_retention_ref_payload(payload: &KernelEventPayload) -> bool {
    matches!(payload, KernelEventPayload::RetentionRefsAppended(_))
}

fn is_run_completed_payload(payload: &KernelEventPayload) -> bool {
    matches!(payload, KernelEventPayload::RunCompleted(_))
}

fn is_completed_run_payload(payload: &KernelEventPayload) -> bool {
    matches!(
        payload,
        KernelEventPayload::RunCompleted(events::RunCompleted {
            outcome: events::RunCompletionOutcome::Completed(_),
            ..
        })
    )
}

fn is_non_run_completed_attempt_terminal_payload(payload: &KernelEventPayload) -> bool {
    is_attempt_terminal_payload(payload) && !is_run_completed_payload(payload)
}

fn is_retention_commit_payload(payload: &KernelEventPayload) -> bool {
    is_retention_payload(payload)
        || is_non_run_completed_attempt_terminal_payload(payload)
        || is_completed_run_payload(payload)
}

fn is_saga_terminal_commit_payload(payload: &KernelEventPayload) -> bool {
    is_run_completed_payload(payload)
        || is_non_run_completed_attempt_terminal_payload(payload)
        || is_retention_ref_payload(payload)
}

pub(super) fn request_contains_saga_terminal_outcome(request: &CommitRequest) -> bool {
    request.payloads.iter().any(|payload| {
        matches!(
            payload,
            KernelEventPayload::RunCompleted(events::RunCompleted {
                outcome: events::RunCompletionOutcome::Compensated
                    | events::RunCompletionOutcome::ManuallyResolved
                    | events::RunCompletionOutcome::FailedWithoutAcdcClaim,
                ..
            })
        )
    })
}

pub(super) fn request_contains_manual_resolution(request: &CommitRequest) -> bool {
    request
        .payloads
        .iter()
        .any(|payload| matches!(payload, KernelEventPayload::ManualResolutionRecorded(_)))
}

pub(super) fn invalid_prepared_commit_purpose(
    purpose: &'static str,
    message: impl Into<String>,
) -> StoreError {
    StoreError::InvalidPreparedCommitPurpose {
        purpose,
        message: message.into(),
    }
}

pub(super) fn validate_payload_run_and_spec(
    run_id: &RunId,
    payloads: &[KernelEventPayload],
) -> Result<()> {
    let Some(first) = payloads.first() else {
        return Err(StoreError::EmptyCommit);
    };
    let expected_spec_hash = payload_spec_hash(first);
    for payload in payloads {
        if let Some(payload_run_id) = payload_run_id(payload) {
            if payload_run_id != *run_id {
                return Err(StoreError::PayloadRunMismatch {
                    expected: Box::new(run_id.clone()),
                    actual: Box::new(payload_run_id),
                });
            }
        }
        let actual_spec_hash = payload_spec_hash(payload);
        if actual_spec_hash != expected_spec_hash {
            return Err(StoreError::PayloadSpecHashMismatch {
                expected: Box::new(expected_spec_hash),
                actual: Box::new(actual_spec_hash),
            });
        }
    }
    Ok(())
}

pub(super) fn validate_terminal_attempt_cell_pairs(payloads: &[KernelEventPayload]) -> Result<()> {
    let mut completions = BTreeSet::new();
    let mut terminal_cells = BTreeSet::new();
    let mut public_outputs = BTreeSet::new();

    for payload in payloads {
        match payload {
            KernelEventPayload::StateAttemptCompleted(payload) => {
                completions.insert((
                    payload.node_id.clone(),
                    payload.attempt_id.clone(),
                    payload.output_cell_id.clone(),
                ));
            }
            KernelEventPayload::CellProduced(payload) => {
                terminal_cells.insert((
                    payload.node_id.clone(),
                    payload.attempt_id.clone(),
                    payload.cell_id.clone(),
                ));
            }
            KernelEventPayload::CellSkipped(payload) => {
                terminal_cells.insert((
                    payload.node_id.clone(),
                    payload.attempt_id.clone(),
                    payload.cell_id.clone(),
                ));
            }
            KernelEventPayload::PublicOutputProduced(payload) => {
                public_outputs.insert((
                    payload.node_id.clone(),
                    payload.attempt_id.clone(),
                    payload.receipt_cell_id.clone(),
                ));
            }
            _ => {}
        }
    }

    for (node_id, attempt_id, output_cell_id) in &completions {
        if !terminal_cells.contains(&(node_id.clone(), attempt_id.clone(), output_cell_id.clone()))
        {
            return Err(StoreError::ProjectionConflict {
                key: format!("attempt:{node_id}:{attempt_id}"),
                message: "attempt completion requires matching terminal cell in same commit"
                    .to_owned(),
            });
        }
    }
    for (node_id, attempt_id, cell_id) in &terminal_cells {
        if !completions.contains(&(node_id.clone(), attempt_id.clone(), cell_id.clone())) {
            return Err(StoreError::ProjectionConflict {
                key: format!("cell:{cell_id}:terminal"),
                message: "terminal cell requires matching attempt completion in same commit"
                    .to_owned(),
            });
        }
    }
    for (node_id, attempt_id, receipt_cell_id) in &public_outputs {
        let terminal = (node_id.clone(), attempt_id.clone(), receipt_cell_id.clone());
        if !terminal_cells.contains(&terminal) || !completions.contains(&terminal) {
            return Err(StoreError::ProjectionConflict {
                key: format!("public_output:{node_id}:{attempt_id}"),
                message: "public output requires matching render receipt terminal in same commit"
                    .to_owned(),
            });
        }
    }
    Ok(())
}

pub(super) fn validate_supported_stream_model(events: &[KernelEventEnvelope]) -> Result<()> {
    let mut started_attempts = BTreeMap::<(NodeId, AttemptId), StreamSeq>::new();
    for event in events {
        if let KernelEventPayload::StateAttemptStarted(payload) = event.payload() {
            started_attempts.insert(
                (payload.node_id.clone(), payload.attempt_id.clone()),
                event.seq(),
            );
        }
    }
    for event in events {
        for (node_id, attempt_id) in payload_required_started_attempts(event.payload()) {
            let key = (node_id.clone(), attempt_id.clone());
            let Some(start_seq) = started_attempts.get(&key) else {
                return Err(StoreError::ProjectionConflict {
                    key: format!("stream_model:{node_id}:{attempt_id}"),
                    message: "invalid run stream model: attempt-bound payload is not preceded by a StateAttemptStarted commit".to_owned(),
                });
            };
            if *start_seq >= event.seq() {
                return Err(StoreError::ProjectionConflict {
                    key: format!("stream_model:{node_id}:{attempt_id}"),
                    message: "invalid run stream model: StateAttemptStarted must be committed before attempt-bound terminal payloads".to_owned(),
                });
            }
        }
    }
    Ok(())
}

fn payload_required_started_attempts(payload: &KernelEventPayload) -> Vec<(NodeId, AttemptId)> {
    match payload {
        KernelEventPayload::StateAttemptCompleted(payload) => {
            vec![(payload.node_id.clone(), payload.attempt_id.clone())]
        }
        KernelEventPayload::StateAttemptInterrupted(payload) => {
            vec![(payload.node_id.clone(), payload.attempt_id.clone())]
        }
        KernelEventPayload::StateAttemptFailed(payload) => {
            vec![(payload.node_id.clone(), payload.attempt_id.clone())]
        }
        KernelEventPayload::CellProduced(payload) => {
            vec![(payload.node_id.clone(), payload.attempt_id.clone())]
        }
        KernelEventPayload::CellSkipped(payload) => {
            vec![(payload.node_id.clone(), payload.attempt_id.clone())]
        }
        KernelEventPayload::FactRecorded(payload) => {
            vec![(payload.node_id.clone(), payload.attempt_id.clone())]
        }
        KernelEventPayload::PublicOutputProduced(payload) => {
            vec![(payload.node_id.clone(), payload.attempt_id.clone())]
        }
        KernelEventPayload::PublicOutputRenderFailed(payload) => {
            vec![(payload.node_id.clone(), payload.attempt_id.clone())]
        }
        payload => payload
            .side_effect_ref()
            .map(|side_effect| vec![(side_effect.node_id.clone(), side_effect.attempt_id.clone())])
            .unwrap_or_default(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TerminalSideEffectEvidencePair {
    node_id: NodeId,
    attempt_id: AttemptId,
    retryable: bool,
    kind: TerminalSideEffectEvidenceKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TerminalSideEffectEvidenceKind {
    Ambiguous,
    Failed,
}

impl TerminalSideEffectEvidencePair {
    fn ambiguous(payload: &side_effect::Ambiguous) -> Self {
        Self {
            node_id: payload.node_id.clone(),
            attempt_id: payload.attempt_id.clone(),
            retryable: false,
            kind: TerminalSideEffectEvidenceKind::Ambiguous,
        }
    }

    fn failed(payload: &side_effect::Failed) -> Self {
        Self {
            node_id: payload.node_id.clone(),
            attempt_id: payload.attempt_id.clone(),
            retryable: payload.retryable,
            kind: TerminalSideEffectEvidenceKind::Failed,
        }
    }

    fn node_attempt_key(&self) -> (NodeId, AttemptId) {
        (self.node_id.clone(), self.attempt_id.clone())
    }

    fn label(&self) -> &'static str {
        match self.kind {
            TerminalSideEffectEvidenceKind::Ambiguous => "ambiguity",
            TerminalSideEffectEvidenceKind::Failed => "failure",
        }
    }
}

pub(super) fn validate_terminal_side_effect_evidence_pairs(
    payloads: &[KernelEventPayload],
) -> Result<()> {
    let mut side_effect_terminals = BTreeMap::new();
    let mut attempt_failures = BTreeMap::new();
    for payload in payloads {
        match payload {
            KernelEventPayload::SideEffectAmbiguous(payload) => {
                insert_terminal_side_effect_pair(
                    &mut side_effect_terminals,
                    TerminalSideEffectEvidencePair::ambiguous(payload),
                )?;
            }
            KernelEventPayload::SideEffectFailed(payload) => {
                insert_terminal_side_effect_pair(
                    &mut side_effect_terminals,
                    TerminalSideEffectEvidencePair::failed(payload),
                )?;
            }
            KernelEventPayload::StateAttemptFailed(payload) => {
                attempt_failures.insert(
                    (payload.node_id.clone(), payload.attempt_id.clone()),
                    payload.retryable,
                );
            }
            _ => {}
        }
    }
    for pair in side_effect_terminals.values() {
        let key = pair.node_attempt_key();
        match attempt_failures.get(&key) {
            Some(attempt_retryable) if *attempt_retryable == pair.retryable => {}
            Some(_) => {
                return Err(StoreError::ProjectionConflict {
                    key: format!(
                        "sidefx:{}:{}:{}",
                        pair.node_id,
                        pair.attempt_id,
                        pair.label()
                    ),
                    message: format!(
                        "side-effect {} retryability must match attempt failure",
                        pair.label()
                    ),
                });
            }
            None => {
                return Err(StoreError::ProjectionConflict {
                    key: format!(
                        "sidefx:{}:{}:{}",
                        pair.node_id,
                        pair.attempt_id,
                        pair.label()
                    ),
                    message: format!(
                        "side-effect {} requires matching StateAttemptFailed in same commit",
                        pair.label()
                    ),
                });
            }
        }
    }
    Ok(())
}

pub(super) fn validate_side_effect_attempt_failures_have_terminal_evidence(
    projections: &ProjectionSnapshot,
    payloads: &[KernelEventPayload],
) -> Result<()> {
    let mut current_side_effect_authority = BTreeSet::new();
    let mut terminal_side_effect_evidence = BTreeSet::new();
    let mut attempt_failures = Vec::new();
    for payload in payloads {
        if let Some(side_effect) = payload.side_effect_ref() {
            let key = (side_effect.node_id.clone(), side_effect.attempt_id.clone());
            current_side_effect_authority.insert(key.clone());
            if matches!(
                side_effect.kind,
                events::SideEffectEventKind::Ambiguous | events::SideEffectEventKind::Failed
            ) {
                terminal_side_effect_evidence.insert(key);
            }
        }
        if let KernelEventPayload::StateAttemptFailed(payload) = payload {
            attempt_failures.push((payload.node_id.clone(), payload.attempt_id.clone()));
        }
    }

    for (node_id, attempt_id) in attempt_failures {
        let key = (node_id.clone(), attempt_id.clone());
        if terminal_side_effect_evidence.contains(&key) {
            continue;
        }
        let prior_side_effect = projections.side_effects().find_map(|(_, projection)| {
            if projection.intent.node_id == node_id && projection.intent.attempt_id == attempt_id {
                Some(projection)
            } else {
                None
            }
        });
        if prior_side_effect
            .map(|projection| projection.ledger_state().map(|state| state.is_confirmed()))
            .transpose()?
            .unwrap_or(false)
        {
            continue;
        }
        if prior_side_effect.is_some() || current_side_effect_authority.contains(&key) {
            return Err(StoreError::ProjectionConflict {
                key: format!("attempt:{node_id}:{attempt_id}"),
                message:
                    "side-effect attempt failure requires terminal side-effect evidence in the same commit"
                        .to_owned(),
            });
        }
    }
    Ok(())
}

fn insert_terminal_side_effect_pair(
    pairs: &mut BTreeMap<(NodeId, AttemptId), TerminalSideEffectEvidencePair>,
    pair: TerminalSideEffectEvidencePair,
) -> Result<()> {
    let key = pair.node_attempt_key();
    if pairs.insert(key.clone(), pair).is_some() {
        return Err(StoreError::ProjectionConflict {
            key: format!("sidefx:{}:{}:terminal", key.0, key.1),
            message: "terminal side-effect evidence must be unique per node attempt in one commit"
                .to_owned(),
        });
    }
    Ok(())
}

pub(super) fn validate_retention_manifest_pairs(payloads: &[KernelEventPayload]) -> Result<()> {
    let retained_manifest_refs = payloads
        .iter()
        .filter_map(|payload| match payload {
            KernelEventPayload::RetentionRefsAppended(payload) => Some(&payload.refs),
            _ => None,
        })
        .flatten()
        .filter(|retention_ref| retention_ref.role == ArtifactRole::RetentionManifest)
        .map(|retention_ref| {
            (
                retention_ref.artifact_id.clone(),
                retention_ref.content_digest.clone(),
            )
        })
        .collect::<BTreeSet<_>>();

    for payload in payloads {
        let KernelEventPayload::RetentionManifestProjected(payload) = payload else {
            continue;
        };
        if !retained_manifest_refs.contains(&(
            payload.manifest_artifact_id.clone(),
            payload.manifest_digest.clone(),
        )) {
            return Err(StoreError::ProjectionConflict {
                key: format!(
                    "retention:{}:manifest:{}",
                    payload.run_id, payload.manifest_seq
                ),
                message:
                    "retention manifest projection requires matching retention ref in same commit"
                        .to_owned(),
            });
        }
    }
    Ok(())
}

fn payload_run_id(payload: &KernelEventPayload) -> Option<RunId> {
    payload.run_id().cloned()
}

pub(super) fn payload_spec_hash(payload: &KernelEventPayload) -> SpecHash {
    payload.spec_hash().clone()
}

pub(super) fn derive_event_id(
    run_id: &RunId,
    seq: StreamSeq,
    ordinal: CommitOrdinal,
    event_schema_id: &SchemaId,
    payload_hash: &ContentDigest,
) -> Result<EventId> {
    let canonical = canonical_json(serde_json::json!({
        "event_schema_id": event_schema_id.as_str(),
        "ordinal": ordinal.as_u32(),
        "payload_hash": payload_hash.as_str(),
        "run_id": run_id.as_str(),
        "seq": seq.as_u64(),
    }))?;
    Ok(EventId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        canonical.digest_bytes(),
    ))
}

pub(super) fn derive_logical_key(
    run_id: &RunId,
    seq: StreamSeq,
    ordinal: CommitOrdinal,
    payload: &KernelEventPayload,
    payload_hash: &ContentDigest,
) -> Result<LogicalEventKey> {
    let key = match payload {
        KernelEventPayload::RunAdmitted(_) => "run:admission".to_owned(),
        KernelEventPayload::ManualResolutionRecorded(_) => "run:manual_resolution".to_owned(),
        KernelEventPayload::RunCompleted(_) => "run:complete".to_owned(),
        KernelEventPayload::StateAttemptStarted(payload) => {
            format!("attempt:{}:{}", payload.node_id, payload.attempt_id)
        }
        KernelEventPayload::StateAttemptCompleted(payload) => {
            format!("attempt:{}:{}", payload.node_id, payload.attempt_id)
        }
        KernelEventPayload::StateAttemptInterrupted(payload) => {
            format!("attempt:{}:{}", payload.node_id, payload.attempt_id)
        }
        KernelEventPayload::StateAttemptFailed(payload) => {
            format!("attempt:{}:{}", payload.node_id, payload.attempt_id)
        }
        KernelEventPayload::FactRecorded(_) => {
            let claim_id =
                mfm_facts::derive_fact_claim_id(run_id.clone(), seq.as_u64(), ordinal.as_u32())
                    .map_err(|error| StoreError::Event(error.to_string()))?;
            fact_claim_projection_key("fact", &claim_id)
        }
        KernelEventPayload::ArtifactReferenced(payload) => {
            format!("artifact:{}:ref", payload.artifact_ref.artifact_id)
        }
        KernelEventPayload::CellProduced(payload) => {
            format!("cell:{}:terminal", payload.cell_id)
        }
        KernelEventPayload::CellSkipped(payload) => {
            format!("cell:{}:terminal", payload.cell_id)
        }
        KernelEventPayload::SideEffectIntentPersisted(payload) => {
            format!(
                "sidefx:{}:{}:intent",
                side_effect_ledger_purpose_key(&payload.ledger_purpose),
                payload.pair_id
            )
        }
        KernelEventPayload::SideEffectClaimed(payload) => format!(
            "sidefx:{}:{}:claim:{}:{}",
            side_effect_ledger_purpose_key(&payload.ledger_purpose),
            payload.pair_id,
            payload.invocation_epoch,
            payload.claim_generation
        ),
        KernelEventPayload::SideEffectClaimTakenOver(payload) => format!(
            "sidefx:{}:{}:claim:{}:{}:taken_over",
            side_effect_ledger_purpose_key(&payload.ledger_purpose),
            payload.pair_id,
            payload.invocation_epoch,
            payload.claim_generation
        ),
        KernelEventPayload::ResourceLaneClaimed(payload) => format!(
            "resource_lane:{}:{}:claim:{}",
            side_effect_ledger_purpose_key(&payload.ledger_purpose),
            payload.pair_id,
            payload.claim_id
        ),
        KernelEventPayload::ResourceLaneClaimIntent(_)
        | KernelEventPayload::ResourceLaneReleaseIntent(_) => {
            return Err(StoreError::Event(
                "resource-lane intent payload reached persisted logical-key derivation".to_owned(),
            ));
        }
        KernelEventPayload::SideEffectInvocationPrepared(payload) => format!(
            "sidefx:{}:{}:invocation:{}:prepared:{}",
            side_effect_ledger_purpose_key(&payload.ledger_purpose),
            payload.pair_id,
            payload.invocation_epoch,
            payload.claim_generation
        ),
        KernelEventPayload::SideEffectInvocationStarted(payload) => format!(
            "sidefx:{}:{}:invocation:{}:started",
            side_effect_ledger_purpose_key(&payload.ledger_purpose),
            payload.pair_id,
            payload.invocation_epoch
        ),
        KernelEventPayload::SideEffectNotSubmittedProven(payload) => format!(
            "sidefx:{}:{}:invocation:{}:submission_result",
            side_effect_ledger_purpose_key(&payload.ledger_purpose),
            payload.pair_id,
            payload.invocation_epoch
        ),
        KernelEventPayload::SideEffectSubmissionObserved(payload) => format!(
            "sidefx:{}:{}:invocation:{}:submission_result",
            side_effect_ledger_purpose_key(&payload.ledger_purpose),
            payload.pair_id,
            payload.invocation_epoch
        ),
        KernelEventPayload::SideEffectSubmissionUnknown(payload) => format!(
            "sidefx:{}:{}:invocation:{}:submission_result",
            side_effect_ledger_purpose_key(&payload.ledger_purpose),
            payload.pair_id,
            payload.invocation_epoch
        ),
        KernelEventPayload::SideEffectReceiptObserved(payload) => format!(
            "sidefx:{}:{}:invocation:{}:receipt",
            side_effect_ledger_purpose_key(&payload.ledger_purpose),
            payload.pair_id,
            payload.invocation_epoch
        ),
        KernelEventPayload::SideEffectConfirmationObserved(payload) => format!(
            "sidefx:{}:{}:invocation:{}:confirmation",
            side_effect_ledger_purpose_key(&payload.ledger_purpose),
            payload.pair_id,
            payload.invocation_epoch
        ),
        KernelEventPayload::SideEffectAmbiguous(payload) => {
            format!(
                "sidefx:{}:{}:ambiguous",
                side_effect_ledger_purpose_key(&payload.ledger_purpose),
                payload.pair_id
            )
        }
        KernelEventPayload::SideEffectFailed(payload) => format!(
            "sidefx:{}:{}:invocation:{}:failure",
            side_effect_ledger_purpose_key(&payload.ledger_purpose),
            payload.pair_id,
            payload.invocation_epoch
        ),
        KernelEventPayload::ResourceLaneReleased(payload) => format!(
            "resource_lane:{}:{}:release:{}",
            side_effect_ledger_purpose_key(&payload.ledger_purpose),
            payload.pair_id,
            payload.release_id
        ),
        KernelEventPayload::PublicOutputProduced(payload) => {
            format!("public_output:{}", payload.public_schema_id)
        }
        KernelEventPayload::PublicOutputRenderFailed(payload) => {
            format!(
                "public_output_failed:{}:{}:{}",
                payload.public_schema_id, payload.node_id, payload.attempt_id
            )
        }
        KernelEventPayload::RetentionRefsAppended(payload) => {
            format!("retention:{}:refs:{}", payload.run_id, payload_hash)
        }
        KernelEventPayload::RetentionManifestProjected(payload) => {
            format!(
                "retention:{}:manifest:{}",
                payload.run_id, payload.manifest_seq
            )
        }
    };
    LogicalEventKey::new(key)
}

pub(super) fn is_unique_logical_key(key: &LogicalEventKey) -> bool {
    let key = key.as_str();
    !key.starts_with("attempt:")
}

pub(super) fn unique_logical_key_rewrite_allowed(
    base: &CommitBase,
    run_id: &RunId,
    logical_key: &LogicalEventKey,
    payload: &KernelEventPayload,
    projections: &ProjectionSnapshot,
) -> Result<bool> {
    if !base
        .logical_keys
        .contains(&(run_id.clone(), logical_key.clone()))
    {
        return Ok(false);
    }
    let Some((pair_id, invocation_epoch)) = recoverable_submission_result_payload(payload) else {
        return Ok(false);
    };
    Ok(matches!(
        projections.side_effect_state_for_pair(run_id, pair_id)?,
        Some(state)
            if matches!(
                state.phase(),
                SideEffectLedgerPhase::SubmissionKnown {
                    claim,
                    status: SideEffectSubmissionState::Unknown,
                } if claim.invocation_epoch == invocation_epoch
            )
    ))
}

fn recoverable_submission_result_payload(
    payload: &KernelEventPayload,
) -> Option<(&SideEffectPairId, u32)> {
    match payload {
        KernelEventPayload::SideEffectNotSubmittedProven(payload) => {
            Some((&payload.pair_id, payload.invocation_epoch))
        }
        KernelEventPayload::SideEffectSubmissionObserved(payload) => {
            Some((&payload.pair_id, payload.invocation_epoch))
        }
        KernelEventPayload::SideEffectSubmissionUnknown(payload) => {
            Some((&payload.pair_id, payload.invocation_epoch))
        }
        _ => None,
    }
}

fn side_effect_ledger_purpose_key(purpose: &events::SideEffectLedgerPurpose) -> String {
    match purpose {
        events::SideEffectLedgerPurpose::Forward => "forward".to_owned(),
        events::SideEffectLedgerPurpose::Remediation {
            forward_pair_id, ..
        } => {
            format!("remediation:{forward_pair_id}")
        }
    }
}
