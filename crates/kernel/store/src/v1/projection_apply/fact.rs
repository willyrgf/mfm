use super::*;

pub(super) fn apply_fact_recorded<R>(
    projections: &mut ProjectionSnapshot,
    envelope: &KernelEventEnvelope,
    payload: &events::FactRecorded,
    objects: &R,
) -> Result<()>
where
    R: ExactRetainedObjectResolver + ?Sized,
{
    let claim = &payload.claim;
    let claim_id = mfm_facts::derive_fact_claim_id(
        envelope.run_id().clone(),
        envelope.seq().as_u64(),
        envelope.ordinal().as_u32(),
    )
    .map_err(|error| StoreError::Identity(error.to_string()))?;
    require_started_fact_attempt(projections, payload, &claim_id)?;

    let descriptor_projection = projections
        .fact_descriptor(claim.fact_descriptor_hash())
        .cloned()
        .ok_or_else(|| StoreError::ProjectionConflict {
            key: format!("fact_descriptor:{}", claim.fact_descriptor_hash()),
            message: "fact descriptor must be admitted before recording a fact".to_owned(),
        })?;
    let descriptor = load_projected_fact_descriptor(&descriptor_projection, objects)?;
    validate_fact_claim_against_descriptor(claim, &descriptor_projection)?;
    let subject_material = mfm_facts::parse_canonical_fact_subject_material_bytes(
        claim.subject().subject_material().as_bytes(),
    )
    .map_err(|error| StoreError::Identity(error.to_string()))?;

    let response = claim.response();
    let (response_bytes, response_evidence) = require_artifact_bytes_by_key(
        objects,
        response.artifact_id(),
        response.artifact_evidence_hash(),
    )?;
    validate_fact_response_evidence(response, response_evidence)?;
    let recorded_at = fact_recorded_at(envelope);
    let store_commit_order = envelope.store_commit_order().as_u64();
    let metadata = mfm_facts::FactExtractionMetadata::new(recorded_at.clone(), store_commit_order)
        .map_err(|error| StoreError::Identity(error.to_string()))?;
    let response_value =
        mfm_facts::parse_canonical_fact_response_bytes(&descriptor, response_bytes)
            .map_err(|error| StoreError::Identity(error.to_string()))?;
    let terms = mfm_facts::extract_terms_from_material(
        &descriptor,
        &subject_material,
        &response_value,
        &metadata,
    )
    .map_err(|error| StoreError::Identity(error.to_string()))?;
    let projection = FactQueryProjection::from_recorded_event(
        envelope,
        payload,
        Some(response_evidence.clone()),
        terms,
    )?;
    if projections
        .fact_query_entries
        .insert(claim_id.clone(), projection)
        .is_some()
    {
        return Err(StoreError::ProjectionConflict {
            key: fact_claim_projection_key("fact_query", &claim_id),
            message: "fact claim id is already projected".to_owned(),
        });
    }
    Ok(())
}

fn require_started_fact_attempt(
    projections: &ProjectionSnapshot,
    payload: &events::FactRecorded,
    claim_id: &mfm_facts::FactClaimId,
) -> Result<()> {
    match projections.attempt(&payload.node_id, &payload.attempt_id) {
        Some(AttemptProjection {
            status: AttemptStatus::Started { .. },
            ..
        }) => Ok(()),
        Some(_) => Err(StoreError::ProjectionConflict {
            key: fact_claim_projection_key("fact", claim_id),
            message: "fact requires an active started attempt".to_owned(),
        }),
        None => Err(StoreError::ProjectionConflict {
            key: fact_claim_projection_key("fact", claim_id),
            message: "fact requires a started attempt".to_owned(),
        }),
    }
}

pub(super) fn apply_fact_descriptor_artifact<R>(
    projections: &mut ProjectionSnapshot,
    artifact: &events::RunArtifactEvidenceRef,
    objects: &R,
) -> Result<()>
where
    R: ExactRetainedObjectResolver + ?Sized,
{
    let expected_schema = mfm_facts::fact_descriptor_schema_id()
        .map_err(|error| StoreError::Identity(error.to_string()))?;
    let evidence = ArtifactEvidenceRef::from_run_artifact(artifact);
    if evidence.artifact_role != ArtifactRole::FactDescriptor
        || evidence.schema_id.as_ref() != Some(&expected_schema)
    {
        return Err(StoreError::ArtifactEvidenceMismatch {
            artifact_id: evidence.artifact_id.clone(),
            field: "fact_descriptor",
        });
    }
    let bytes = require_artifact_bytes_exact(objects, &evidence)?;
    let descriptor = mfm_facts::parse_canonical_fact_descriptor_bytes(bytes)
        .map_err(|error| StoreError::Identity(error.to_string()))?;
    let descriptor_hash = mfm_facts::fact_descriptor_hash(&descriptor)
        .map_err(|error| StoreError::Identity(error.to_string()))?;
    if descriptor_hash != evidence.digest {
        return Err(StoreError::ArtifactEvidenceMismatch {
            artifact_id: evidence.artifact_id.clone(),
            field: "digest",
        });
    }
    let namespace_hash = mfm_facts::fact_subject_namespace_hash(&descriptor)
        .map_err(|error| StoreError::Identity(error.to_string()))?;
    let projection = FactDescriptorProjection {
        descriptor_hash: descriptor_hash.clone(),
        descriptor_artifact_id: artifact.artifact_id.clone(),
        descriptor_artifact_evidence: evidence.clone(),
        fact_kind: descriptor.fact_kind().clone(),
        descriptor_schema_id: descriptor.descriptor_schema_id().clone(),
        subject_schema_id: descriptor.subject_schema_id().clone(),
        response_schema_id: descriptor.response_schema_id().clone(),
        fact_subject_namespace_hash: namespace_hash,
    };
    match projections
        .fact_descriptors
        .insert(descriptor_hash.clone(), projection.clone())
    {
        Some(existing) if equivalent_fact_descriptor_projection(&existing, &projection) => {
            projections
                .fact_descriptors
                .insert(descriptor_hash, existing);
        }
        Some(_) => {
            return Err(StoreError::ProjectionConflict {
                key: format!("fact_descriptor:{descriptor_hash}"),
                message: "conflicting fact descriptor projection".to_owned(),
            });
        }
        None => {}
    }
    Ok(())
}

fn equivalent_fact_descriptor_projection(
    left: &FactDescriptorProjection,
    right: &FactDescriptorProjection,
) -> bool {
    left.descriptor_hash == right.descriptor_hash
        && left.descriptor_artifact_id == right.descriptor_artifact_id
        && left.descriptor_artifact_evidence == right.descriptor_artifact_evidence
        && left.fact_kind == right.fact_kind
        && left.descriptor_schema_id == right.descriptor_schema_id
        && left.subject_schema_id == right.subject_schema_id
        && left.response_schema_id == right.response_schema_id
        && left.fact_subject_namespace_hash == right.fact_subject_namespace_hash
}

fn load_projected_fact_descriptor<R>(
    projection: &FactDescriptorProjection,
    objects: &R,
) -> Result<mfm_facts::FactDescriptor>
where
    R: ExactRetainedObjectResolver + ?Sized,
{
    let expected_schema = mfm_facts::fact_descriptor_schema_id()
        .map_err(|error| StoreError::Identity(error.to_string()))?;
    let projected_evidence = &projection.descriptor_artifact_evidence;
    if projected_evidence.artifact_id != projection.descriptor_artifact_id
        || projected_evidence.digest != projection.descriptor_hash
        || projected_evidence.artifact_role != ArtifactRole::FactDescriptor
        || projected_evidence.schema_id.as_ref() != Some(&expected_schema)
        || projected_evidence.semantic_type_id.is_some()
        || projected_evidence.producer_node_id.is_some()
        || projected_evidence.producer_seed_id.is_some()
    {
        return Err(StoreError::ArtifactEvidenceMismatch {
            artifact_id: projection.descriptor_artifact_id.clone(),
            field: "fact_descriptor",
        });
    }
    let bytes = require_artifact_bytes_exact(objects, projected_evidence)?;
    let descriptor = mfm_facts::parse_canonical_fact_descriptor_bytes(bytes)
        .map_err(|error| StoreError::Identity(error.to_string()))?;
    let descriptor_hash = mfm_facts::fact_descriptor_hash(&descriptor)
        .map_err(|error| StoreError::Identity(error.to_string()))?;
    let subject_namespace_hash = mfm_facts::fact_subject_namespace_hash(&descriptor)
        .map_err(|error| StoreError::Identity(error.to_string()))?;
    if descriptor_hash != projection.descriptor_hash
        || descriptor.fact_kind() != &projection.fact_kind
        || descriptor.descriptor_schema_id() != &projection.descriptor_schema_id
        || descriptor.subject_schema_id() != &projection.subject_schema_id
        || descriptor.response_schema_id() != &projection.response_schema_id
        || subject_namespace_hash != projection.fact_subject_namespace_hash
    {
        return Err(StoreError::ArtifactEvidenceMismatch {
            artifact_id: projection.descriptor_artifact_id.clone(),
            field: "fact_descriptor",
        });
    }
    Ok(descriptor)
}

fn validate_fact_claim_against_descriptor(
    claim: &mfm_facts::FactClaim,
    descriptor: &FactDescriptorProjection,
) -> Result<()> {
    if claim.fact_descriptor_hash() != &descriptor.descriptor_hash
        || claim.fact_kind() != &descriptor.fact_kind
        || claim.subject().fact_subject_namespace_hash() != &descriptor.fact_subject_namespace_hash
        || claim.response().response_schema_id() != &descriptor.response_schema_id
    {
        return Err(StoreError::ProjectionConflict {
            key: format!("fact_descriptor:{}", claim.fact_descriptor_hash()),
            message: "fact claim does not match admitted descriptor".to_owned(),
        });
    }
    Ok(())
}

fn validate_fact_response_evidence(
    response: &mfm_facts::FactResponseEvidence,
    evidence: &ArtifactEvidenceRef,
) -> Result<()> {
    if evidence.artifact_role != ArtifactRole::FactResponse
        || evidence.schema_id.as_ref() != Some(response.response_schema_id())
        || &evidence.digest != response.response_hash()
        || &evidence.artifact_id != response.artifact_id()
        || evidence.evidence_hash()? != *response.artifact_evidence_hash()
    {
        return Err(StoreError::ArtifactEvidenceMismatch {
            artifact_id: response.artifact_id().clone(),
            field: "fact_response",
        });
    }
    Ok(())
}

fn require_artifact_bytes_exact<'a, R>(
    objects: &'a R,
    evidence: &ArtifactEvidenceRef,
) -> Result<&'a [u8]>
where
    R: ExactRetainedObjectResolver + ?Sized,
{
    let evidence_hash = evidence.evidence_hash()?;
    let key = (evidence.artifact_id.clone(), evidence_hash);
    let Some((bytes, stored_evidence)) = objects.resolve_exact(&key) else {
        return Err(StoreError::MissingArtifact {
            artifact_id: evidence.artifact_id.clone(),
        });
    };
    if stored_evidence != evidence {
        return Err(StoreError::ArtifactEvidenceMismatch {
            artifact_id: evidence.artifact_id.clone(),
            field: "artifact",
        });
    }
    super::super::verify_retained_artifact_bytes(bytes, stored_evidence)?;
    Ok(bytes)
}

fn require_artifact_bytes_by_key<'a, R>(
    objects: &'a R,
    artifact_id: &ArtifactId,
    evidence_hash: &ContentDigest,
) -> Result<(&'a [u8], &'a ArtifactEvidenceRef)>
where
    R: ExactRetainedObjectResolver + ?Sized,
{
    let key = (artifact_id.clone(), evidence_hash.clone());
    let Some((bytes, evidence)) = objects.resolve_exact(&key) else {
        return Err(StoreError::MissingArtifact {
            artifact_id: artifact_id.clone(),
        });
    };
    super::super::verify_retained_artifact_bytes(bytes, evidence)?;
    Ok((bytes, evidence))
}

fn fact_recorded_at(_envelope: &KernelEventEnvelope) -> String {
    "1970-01-01T00:00:00Z".to_owned()
}
