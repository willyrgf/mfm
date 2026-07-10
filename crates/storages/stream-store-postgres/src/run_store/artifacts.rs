use super::*;

pub(super) async fn load_artifacts(
    tx: &mut Transaction<'_, Postgres>,
    run_id: &RunId,
) -> Result<ArtifactAuthorityMap> {
    let rows = sqlx::query(
        "SELECT a.artifact_id, a.evidence_hash, a.digest, a.byte_len, a.media_type, a.schema_id, \
         a.semantic_type_id, a.producer_node_id, a.producer_seed_id, a.artifact_role \
         FROM artifact_admissions a \
         INNER JOIN run_artifact_admissions ra \
           ON ra.artifact_id = a.artifact_id AND ra.evidence_hash = a.evidence_hash \
         WHERE ra.run_id = $1",
    )
    .bind(run_id.as_str())
    .fetch_all(&mut **tx)
    .await
    .map_err(|error| database_error("failed to load artifact evidence", error))?;
    let mut artifacts = BTreeMap::new();
    for row in rows {
        let artifact_id: String = row
            .try_get("artifact_id")
            .map_err(|error| database_error("failed to decode artifact evidence row", error))?;
        let artifact_id = parse_identity::<ArtifactId>(&artifact_id)?;
        let evidence_hash: String = row
            .try_get("evidence_hash")
            .map_err(|error| database_error("failed to decode artifact evidence row", error))?;
        let evidence_hash = parse_identity::<ContentDigest>(&evidence_hash)?;
        let evidence = ArtifactEvidenceParts {
            artifact_id: artifact_id.clone(),
            digest: row
                .try_get("digest")
                .map_err(|error| database_error("failed to decode artifact evidence row", error))?,
            byte_len: row
                .try_get("byte_len")
                .map_err(|error| database_error("failed to decode artifact evidence row", error))?,
            media_type: row
                .try_get("media_type")
                .map_err(|error| database_error("failed to decode artifact evidence row", error))?,
            schema_id: row
                .try_get("schema_id")
                .map_err(|error| database_error("failed to decode artifact evidence row", error))?,
            semantic_type_id: row
                .try_get("semantic_type_id")
                .map_err(|error| database_error("failed to decode artifact evidence row", error))?,
            producer_node_id: row
                .try_get("producer_node_id")
                .map_err(|error| database_error("failed to decode artifact evidence row", error))?,
            producer_seed_id: row
                .try_get("producer_seed_id")
                .map_err(|error| database_error("failed to decode artifact evidence row", error))?,
            artifact_role: row
                .try_get("artifact_role")
                .map_err(|error| database_error("failed to decode artifact evidence row", error))?,
        }
        .into_evidence_ref()?;
        if evidence.evidence_hash()? != evidence_hash {
            return Err(StoreError::ArtifactEvidenceMismatch {
                artifact_id,
                field: "evidence_hash",
            }
            .into());
        }
        let key = (artifact_id.clone(), evidence_hash);
        if let Some(existing) = artifacts.get(&key) {
            if existing != &evidence {
                return Err(StoreError::ArtifactEvidenceMismatch {
                    artifact_id,
                    field: "artifact",
                }
                .into());
            }
        } else {
            artifacts.insert(key, evidence);
        }
    }
    Ok(artifacts)
}

pub(super) async fn load_artifact_record_tx(
    tx: &mut Transaction<'_, Postgres>,
    artifact_id: &ArtifactId,
    evidence_hash: &ContentDigest,
) -> Result<ArtifactRecord> {
    let row = sqlx::query(
        "SELECT a.artifact_id, a.evidence_hash, a.digest, a.byte_len, a.media_type, a.schema_id, \
         a.semantic_type_id, a.producer_node_id, a.producer_seed_id, a.artifact_role, b.bytes \
         FROM artifact_admissions a \
         INNER JOIN artifact_blobs b \
           ON b.artifact_id = a.artifact_id AND b.digest = a.digest AND b.byte_len = a.byte_len \
         WHERE a.artifact_id = $1 AND a.evidence_hash = $2",
    )
    .bind(artifact_id.as_str())
    .bind(evidence_hash.as_str())
    .fetch_optional(&mut **tx)
    .await
    .map_err(|error| database_error("failed to query artifact bytes", error))?;
    let Some(row) = row else {
        return Err(StoreError::MissingArtifact {
            artifact_id: artifact_id.clone(),
        }
        .into());
    };
    artifact_record_from_row(row)
}

pub(super) fn verify_artifact_record(
    record: &ArtifactRecord,
    evidence: &ArtifactEvidenceRef,
    bytes: Option<&[u8]>,
) -> Result<()> {
    PreparedArtifactBytes::new(record.artifact_bytes.clone(), record.evidence.clone())?;
    if record.evidence_hash != evidence.evidence_hash()? || record.evidence != *evidence {
        return Err(StoreError::ArtifactEvidenceMismatch {
            artifact_id: evidence.artifact_id.clone(),
            field: "artifact",
        }
        .into());
    }
    if let Some(bytes) = bytes {
        if record.artifact_bytes.as_slice() != bytes {
            return Err(StoreError::ArtifactEvidenceMismatch {
                artifact_id: evidence.artifact_id.clone(),
                field: "artifact_bytes",
            }
            .into());
        }
    }
    Ok(())
}

pub(super) struct ArtifactRecord {
    pub(super) evidence_hash: ContentDigest,
    pub(super) evidence: ArtifactEvidenceRef,
    pub(super) artifact_bytes: Vec<u8>,
}

pub(super) fn artifact_record_from_row(row: PgRow) -> Result<ArtifactRecord> {
    let artifact_id_text: String = row
        .try_get("artifact_id")
        .map_err(|error| database_error("failed to decode artifact row", error))?;
    let evidence_hash_text: String = row
        .try_get("evidence_hash")
        .map_err(|error| database_error("failed to decode artifact row", error))?;
    let evidence = ArtifactEvidenceParts {
        artifact_id: parse_identity::<ArtifactId>(&artifact_id_text)?,
        digest: row
            .try_get("digest")
            .map_err(|error| database_error("failed to decode artifact row", error))?,
        byte_len: row
            .try_get("byte_len")
            .map_err(|error| database_error("failed to decode artifact row", error))?,
        media_type: row
            .try_get("media_type")
            .map_err(|error| database_error("failed to decode artifact row", error))?,
        schema_id: row
            .try_get("schema_id")
            .map_err(|error| database_error("failed to decode artifact row", error))?,
        semantic_type_id: row
            .try_get("semantic_type_id")
            .map_err(|error| database_error("failed to decode artifact row", error))?,
        producer_node_id: row
            .try_get("producer_node_id")
            .map_err(|error| database_error("failed to decode artifact row", error))?,
        producer_seed_id: row
            .try_get("producer_seed_id")
            .map_err(|error| database_error("failed to decode artifact row", error))?,
        artifact_role: row
            .try_get("artifact_role")
            .map_err(|error| database_error("failed to decode artifact row", error))?,
    }
    .into_evidence_ref()?;
    Ok(ArtifactRecord {
        evidence_hash: parse_identity::<ContentDigest>(&evidence_hash_text)?,
        evidence: {
            validate_artifact_blob_size(&evidence)?;
            evidence
        },
        artifact_bytes: row
            .try_get("bytes")
            .map_err(|error| database_error("failed to decode artifact row", error))?,
    })
}

pub(super) async fn read_retained_artifact_from_pool(
    pool: &PgPool,
    requirement: &EventArtifactRequirement,
) -> mfm_store::v1::Result<VerifiedRunArtifactBytes> {
    let row = sqlx::query(
        "SELECT a.artifact_id, a.evidence_hash, a.digest, a.byte_len, a.media_type, a.schema_id, \
         a.semantic_type_id, a.producer_node_id, a.producer_seed_id, a.artifact_role, b.bytes \
         FROM artifact_admissions a \
         INNER JOIN artifact_blobs b \
           ON b.artifact_id = a.artifact_id AND b.digest = a.digest AND b.byte_len = a.byte_len \
         WHERE a.artifact_id = $1 AND a.evidence_hash = $2",
    )
    .bind(requirement.artifact_id.as_str())
    .bind(requirement.evidence_hash.as_str())
    .fetch_optional(pool)
    .await
    .map_err(|_| StoreError::ArtifactReadFailed {
        artifact_id: requirement.artifact_id.clone(),
    })?;
    let Some(row) = row else {
        return Err(StoreError::MissingArtifact {
            artifact_id: requirement.artifact_id.clone(),
        });
    };
    let record = artifact_record_from_row(row).map_err(|_| StoreError::ArtifactReadFailed {
        artifact_id: requirement.artifact_id.clone(),
    })?;
    VerifiedRunArtifactBytes::new(record.artifact_bytes, record.evidence, requirement)
}

pub(super) struct ArtifactEvidenceParts {
    pub(super) artifact_id: ArtifactId,
    pub(super) digest: String,
    pub(super) byte_len: i64,
    pub(super) media_type: String,
    pub(super) schema_id: Option<String>,
    pub(super) semantic_type_id: Option<String>,
    pub(super) producer_node_id: Option<String>,
    pub(super) producer_seed_id: Option<String>,
    pub(super) artifact_role: String,
}

impl ArtifactEvidenceParts {
    pub(super) fn into_evidence_ref(self) -> Result<ArtifactEvidenceRef> {
        Ok(ArtifactEvidenceRef {
            artifact_id: self.artifact_id,
            digest: parse_identity::<ContentDigest>(&self.digest)?,
            byte_len: i64_to_nonnegative_u64(self.byte_len, "artifact_admissions.byte_len")?,
            media_type: MediaType::new(self.media_type)?,
            schema_id: parse_optional_identity(self.schema_id)?,
            semantic_type_id: parse_optional_identity(self.semantic_type_id)?,
            producer_node_id: parse_optional_identity(self.producer_node_id)?,
            producer_seed_id: parse_optional_identity(self.producer_seed_id)?,
            artifact_role: decode_artifact_role_tag(&self.artifact_role)?,
        })
    }
}

pub(super) fn decode_artifact_role_tag(value: &str) -> Result<events::ArtifactRole> {
    events::ArtifactRole::parse(value)
        .ok_or_else(|| StoreError::Identity(format!("unknown artifact role {value}")).into())
}
