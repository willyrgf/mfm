use super::*;

pub(super) fn apply_fact_recorded(
    projections: &mut ProjectionSnapshot,
    envelope: &KernelEventEnvelope,
    payload: &events::FactRecorded,
    artifact_bytes: &ArtifactByteAuthorityMap,
) -> Result<()> {
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
    let descriptor = load_projected_fact_descriptor(&descriptor_projection, artifact_bytes)?;
    validate_fact_claim_against_descriptor(claim, &descriptor_projection)?;
    let subject_material = mfm_facts::parse_canonical_fact_subject_material_bytes(
        claim.subject().subject_material().as_bytes(),
    )
    .map_err(|error| StoreError::Identity(error.to_string()))?;

    let response = claim.response();
    let (response_bytes, response_evidence) = require_artifact_bytes_by_key(
        artifact_bytes,
        response.artifact_id(),
        response.artifact_evidence_hash(),
    )?;
    validate_fact_response_evidence(response, response_evidence)?;
    let recorded_at = fact_recorded_at(envelope);
    let store_commit_order = envelope.store_commit_order().as_u64();
    let projection = FactQueryProjection::from_recorded_event(
        envelope,
        payload,
        Some(response_evidence.clone()),
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

    for term in terms {
        let key = (claim_id.clone(), term.field_id().clone());
        if projections
            .fact_term_entries
            .insert(
                key.clone(),
                FactIndexTermProjection::from_extracted_term(
                    &claim_id,
                    claim.fact_descriptor_hash(),
                    &term,
                ),
            )
            .is_some()
        {
            return Err(StoreError::ProjectionConflict {
                key: format!(
                    "{}:{}",
                    fact_claim_projection_key("fact_term", &claim_id),
                    key.1
                ),
                message: "duplicate fact term projection".to_owned(),
            });
        }
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

pub(super) fn apply_fact_descriptor_artifact(
    projections: &mut ProjectionSnapshot,
    artifact: &events::RunArtifactEvidenceRef,
    artifact_bytes: &ArtifactByteAuthorityMap,
) -> Result<()> {
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
    let bytes = require_artifact_bytes_exact(artifact_bytes, &evidence)?;
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

fn load_projected_fact_descriptor(
    projection: &FactDescriptorProjection,
    artifact_bytes: &ArtifactByteAuthorityMap,
) -> Result<mfm_facts::FactDescriptor> {
    let expected_schema = mfm_facts::fact_descriptor_schema_id()
        .map_err(|error| StoreError::Identity(error.to_string()))?;
    for ((artifact_id, _), (bytes, evidence)) in artifact_bytes {
        if artifact_id != &projection.descriptor_artifact_id {
            continue;
        }
        if evidence.digest != projection.descriptor_hash {
            continue;
        }
        if evidence.artifact_role != ArtifactRole::FactDescriptor
            || evidence.schema_id.as_ref() != Some(&expected_schema)
        {
            return Err(StoreError::ArtifactEvidenceMismatch {
                artifact_id: evidence.artifact_id.clone(),
                field: "fact_descriptor",
            });
        }
        let descriptor = mfm_facts::parse_canonical_fact_descriptor_bytes(bytes)
            .map_err(|error| StoreError::Identity(error.to_string()))?;
        let descriptor_hash = mfm_facts::fact_descriptor_hash(&descriptor)
            .map_err(|error| StoreError::Identity(error.to_string()))?;
        if descriptor_hash != projection.descriptor_hash {
            return Err(StoreError::ArtifactEvidenceMismatch {
                artifact_id: evidence.artifact_id.clone(),
                field: "digest",
            });
        }
        return Ok(descriptor);
    }
    Err(StoreError::MissingArtifact {
        artifact_id: projection.descriptor_artifact_id.clone(),
    })
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

fn require_artifact_bytes_exact<'a>(
    artifact_bytes: &'a ArtifactByteAuthorityMap,
    evidence: &ArtifactEvidenceRef,
) -> Result<&'a [u8]> {
    let evidence_hash = evidence.evidence_hash()?;
    let Some((bytes, stored_evidence)) =
        artifact_bytes.get(&(evidence.artifact_id.clone(), evidence_hash))
    else {
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
    super::super::verify_retained_artifact_bytes(bytes, evidence)?;
    Ok(bytes.as_slice())
}

fn require_artifact_bytes_by_key<'a>(
    artifact_bytes: &'a ArtifactByteAuthorityMap,
    artifact_id: &ArtifactId,
    evidence_hash: &ContentDigest,
) -> Result<(&'a [u8], &'a ArtifactEvidenceRef)> {
    let Some((bytes, evidence)) = artifact_bytes.get(&(artifact_id.clone(), evidence_hash.clone()))
    else {
        return Err(StoreError::MissingArtifact {
            artifact_id: artifact_id.clone(),
        });
    };
    super::super::verify_retained_artifact_bytes(bytes, evidence)?;
    Ok((bytes.as_slice(), evidence))
}

fn fact_recorded_at(_envelope: &KernelEventEnvelope) -> String {
    "1970-01-01T00:00:00Z".to_owned()
}
