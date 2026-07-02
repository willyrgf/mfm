use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PhysicalFactProjections {
    pub(super) fact_descriptors: BTreeMap<ContentDigest, mfm_store::v1::FactDescriptorProjection>,
    pub(super) fact_index_entries:
        BTreeMap<mfm_facts::FactClaimId, mfm_store::v1::FactIndexProjection>,
    pub(super) fact_term_entries: BTreeMap<
        (mfm_facts::FactClaimId, mfm_facts::FactFieldId),
        mfm_store::v1::FactIndexTermProjection,
    >,
}

pub(super) async fn insert_fact_projection_rows_tx(
    tx: &mut Transaction<'_, Postgres>,
    before: &ProjectionSnapshot,
    after: &ProjectionSnapshot,
    events: &[KernelEventEnvelope],
    commit_id: &str,
) -> Result<()> {
    for (descriptor_hash, projection) in after.fact_descriptors() {
        if before.fact_descriptor(descriptor_hash).is_none() {
            insert_fact_descriptor_projection_tx(tx, projection, events, commit_id).await?;
        }
    }
    for (claim_id, projection) in after.fact_index_entries() {
        if before.fact_index_entry(claim_id).is_none() {
            insert_fact_index_projection_tx(tx, projection, commit_id).await?;
        }
    }
    for (key, projection) in after.fact_term_entries() {
        if !before
            .fact_term_entries()
            .any(|(existing_key, _)| existing_key == key)
        {
            insert_fact_term_projection_tx(tx, projection).await?;
        }
    }
    Ok(())
}

pub(super) async fn load_fact_projection_tables_tx(
    tx: &mut Transaction<'_, Postgres>,
    run_id: &RunId,
) -> Result<PhysicalFactProjections> {
    Ok(PhysicalFactProjections {
        fact_descriptors: load_fact_descriptor_index_tx(tx, run_id).await?,
        fact_index_entries: load_fact_index_tx(tx, run_id).await?,
        fact_term_entries: load_fact_index_terms_tx(tx, run_id).await?,
    })
}

#[cfg(all(test, feature = "parity-tests"))]
pub(super) async fn rebuild_fact_projection_tables_tx(
    tx: &mut Transaction<'_, Postgres>,
    run_id: &RunId,
    stream: &[KernelEventEnvelope],
) -> Result<ProjectionSnapshot> {
    let artifact_bytes = load_fact_rebuild_artifact_bytes_tx(tx, stream).await?;
    let rebuilt =
        ProjectionSnapshot::rebuild_from_run_stream_with_artifact_bytes(stream, &artifact_bytes)?;
    let commit_ids = load_commit_ids_by_event_tx(tx, run_id).await?;
    delete_fact_projection_rows_tx(tx, run_id).await?;
    insert_rebuilt_fact_projection_rows_tx(tx, &rebuilt, stream, &commit_ids).await?;
    increment_fact_projection_generation_tx(tx).await?;
    let actual = load_fact_projection_tables_tx(tx, run_id).await?;
    let expected = physical_fact_projections_from_snapshot(&rebuilt);
    if actual != expected {
        return Err(PostgresStoreError::Corruption(
            "rebuilt fact projection tables did not match authoritative stream projection"
                .to_owned(),
        ));
    }
    Ok(rebuilt)
}

#[cfg(all(test, feature = "parity-tests"))]
async fn increment_fact_projection_generation_tx(tx: &mut Transaction<'_, Postgres>) -> Result<()> {
    sqlx::query(
        "UPDATE fact_projection_metadata \
         SET projection_generation = projection_generation + 1 \
         WHERE singleton",
    )
    .execute(&mut **tx)
    .await
    .map_err(|error| database_error("failed to increment fact projection generation", error))?;
    Ok(())
}

pub(super) async fn load_fact_rebuild_artifact_bytes_tx(
    tx: &mut Transaction<'_, Postgres>,
    stream: &[KernelEventEnvelope],
) -> Result<ArtifactByteAuthorityMap> {
    let mut artifact_bytes = ArtifactByteAuthorityMap::new();
    for event in stream {
        match event.payload() {
            events::KernelEventPayload::RunAdmitted(payload) => {
                for artifact in &payload.fact_descriptor_artifacts {
                    let evidence = store_artifact_from_run_artifact(artifact);
                    let evidence_hash = evidence.evidence_hash()?;
                    let record =
                        load_artifact_record_tx(tx, &evidence.artifact_id, &evidence_hash).await?;
                    verify_artifact_record(&record, &evidence, None)?;
                    insert_artifact_record_into_byte_authority(&mut artifact_bytes, record)?;
                }
            }
            events::KernelEventPayload::FactRecorded(payload) => {
                let response = payload.claim.response();
                let record = load_artifact_record_tx(
                    tx,
                    response.artifact_id(),
                    response.artifact_evidence_hash(),
                )
                .await?;
                insert_artifact_record_into_byte_authority(&mut artifact_bytes, record)?;
            }
            _ => {}
        }
    }
    Ok(artifact_bytes)
}

pub(super) async fn load_fact_descriptor_artifact_bytes_tx(
    tx: &mut Transaction<'_, Postgres>,
    run_id: &RunId,
    projections: &ProjectionSnapshot,
    artifact_bytes: &mut ArtifactByteAuthorityMap,
) -> Result<()> {
    for (_, projection) in projections.fact_descriptors() {
        load_fact_descriptor_artifact_bytes_for_projection_tx(
            tx,
            run_id,
            projection,
            artifact_bytes,
        )
        .await?;
    }
    Ok(())
}

async fn load_fact_descriptor_artifact_bytes_for_projection_tx(
    tx: &mut Transaction<'_, Postgres>,
    run_id: &RunId,
    projection: &mfm_store::v1::FactDescriptorProjection,
    artifact_bytes: &mut ArtifactByteAuthorityMap,
) -> Result<()> {
    let row = sqlx::query(
        "SELECT a.artifact_id, a.evidence_hash, a.digest, a.byte_len, a.media_type, \
         a.schema_id, a.semantic_type_id, a.producer_node_id, a.producer_seed_id, \
         a.artifact_role, b.bytes \
         FROM fact_descriptor_index f \
         INNER JOIN artifact_admissions a \
           ON a.artifact_id = f.descriptor_artifact_id \
          AND a.evidence_hash = f.descriptor_artifact_evidence_hash \
         INNER JOIN run_artifact_admissions ra \
           ON ra.run_id = f.source_run_id \
          AND ra.artifact_id = a.artifact_id \
          AND ra.evidence_hash = a.evidence_hash \
         INNER JOIN artifact_blobs b \
           ON b.artifact_id = a.artifact_id \
          AND b.digest = a.digest \
          AND b.byte_len = a.byte_len \
         WHERE f.source_run_id = $1 AND f.descriptor_hash = $2",
    )
    .bind(run_id.as_str())
    .bind(projection.descriptor_hash.as_str())
    .fetch_optional(&mut **tx)
    .await
    .map_err(|error| database_error("failed to load fact descriptor artifact bytes", error))?;
    let Some(row) = row else {
        return Err(StoreError::MissingArtifact {
            artifact_id: projection.descriptor_artifact_id.clone(),
        }
        .into());
    };
    let record = artifact_record_from_row(row)?;
    let evidence_hash = record.evidence.evidence_hash()?;
    if evidence_hash != record.evidence_hash {
        return Err(StoreError::ArtifactEvidenceMismatch {
            artifact_id: record.evidence.artifact_id.clone(),
            field: "evidence_hash",
        }
        .into());
    }
    if record.evidence.artifact_id != projection.descriptor_artifact_id
        || record.evidence.digest != projection.descriptor_hash
        || record.evidence.artifact_role != events::ArtifactRole::FactDescriptor
    {
        return Err(StoreError::ArtifactEvidenceMismatch {
            artifact_id: projection.descriptor_artifact_id.clone(),
            field: "fact_descriptor",
        }
        .into());
    }
    let expected_schema = mfm_facts::fact_descriptor_schema_id()
        .map_err(|error| StoreError::Identity(error.to_string()))?;
    if record.evidence.schema_id.as_ref() != Some(&expected_schema) {
        return Err(StoreError::ArtifactEvidenceMismatch {
            artifact_id: projection.descriptor_artifact_id.clone(),
            field: "fact_descriptor",
        }
        .into());
    }

    insert_artifact_record_into_byte_authority(artifact_bytes, record)
}

#[cfg(all(test, feature = "parity-tests"))]
async fn delete_fact_projection_rows_tx(
    tx: &mut Transaction<'_, Postgres>,
    run_id: &RunId,
) -> Result<()> {
    sqlx::query("DELETE FROM fact_index_terms WHERE source_run_id = $1")
        .bind(run_id.as_str())
        .execute(&mut **tx)
        .await
        .map_err(|error| database_error("failed to delete fact term projections", error))?;
    sqlx::query("DELETE FROM fact_index WHERE source_run_id = $1")
        .bind(run_id.as_str())
        .execute(&mut **tx)
        .await
        .map_err(|error| database_error("failed to delete fact index projections", error))?;
    sqlx::query("DELETE FROM fact_descriptor_index WHERE source_run_id = $1")
        .bind(run_id.as_str())
        .execute(&mut **tx)
        .await
        .map_err(|error| database_error("failed to delete fact descriptor projections", error))?;
    Ok(())
}

#[cfg(all(test, feature = "parity-tests"))]
async fn insert_rebuilt_fact_projection_rows_tx(
    tx: &mut Transaction<'_, Postgres>,
    snapshot: &ProjectionSnapshot,
    events: &[KernelEventEnvelope],
    commit_ids_by_event: &BTreeMap<mfm_ids::EventId, String>,
) -> Result<()> {
    for (_, projection) in snapshot.fact_descriptors() {
        let commit_id = required_commit_id(commit_ids_by_event, &projection.source_event_id)?;
        insert_fact_descriptor_projection_tx(tx, projection, events, commit_id).await?;
    }
    for (_, projection) in snapshot.fact_index_entries() {
        let commit_id = required_commit_id(commit_ids_by_event, &projection.source_event_id)?;
        insert_fact_index_projection_tx(tx, projection, commit_id).await?;
    }
    for (_, projection) in snapshot.fact_term_entries() {
        insert_fact_term_projection_tx(tx, projection).await?;
    }
    Ok(())
}

#[cfg(all(test, feature = "parity-tests"))]
async fn load_commit_ids_by_event_tx(
    tx: &mut Transaction<'_, Postgres>,
    run_id: &RunId,
) -> Result<BTreeMap<mfm_ids::EventId, String>> {
    let rows = sqlx::query(
        "SELECT event_id, commit_id FROM run_events WHERE run_id = $1 ORDER BY seq, ordinal",
    )
    .bind(run_id.as_str())
    .fetch_all(&mut **tx)
    .await
    .map_err(|error| database_error("failed to load fact projection commit ids", error))?;
    let mut commit_ids = BTreeMap::new();
    for row in rows {
        let event_id = parse_identity::<mfm_ids::EventId>(&required_string(
            &row,
            "event_id",
            "run_events.event_id",
        )?)?;
        let commit_id = required_string(&row, "commit_id", "run_events.commit_id")?;
        commit_ids.insert(event_id, commit_id);
    }
    Ok(commit_ids)
}

#[cfg(all(test, feature = "parity-tests"))]
fn required_commit_id<'a>(
    commit_ids_by_event: &'a BTreeMap<mfm_ids::EventId, String>,
    event_id: &mfm_ids::EventId,
) -> Result<&'a str> {
    commit_ids_by_event
        .get(event_id)
        .map(String::as_str)
        .ok_or_else(|| {
            PostgresStoreError::Corruption(format!(
                "event {event_id} missing commit id for fact projection rebuild"
            ))
        })
}

#[cfg(all(test, feature = "parity-tests"))]
fn physical_fact_projections_from_snapshot(
    snapshot: &ProjectionSnapshot,
) -> PhysicalFactProjections {
    PhysicalFactProjections {
        fact_descriptors: snapshot
            .fact_descriptors()
            .map(|(descriptor_hash, projection)| (descriptor_hash.clone(), projection.clone()))
            .collect(),
        fact_index_entries: snapshot
            .fact_index_entries()
            .map(|(claim_id, projection)| (claim_id.clone(), projection.clone()))
            .collect(),
        fact_term_entries: snapshot
            .fact_term_entries()
            .map(|(key, projection)| (key.clone(), projection.clone()))
            .collect(),
    }
}

fn insert_artifact_record_into_byte_authority(
    artifact_bytes: &mut ArtifactByteAuthorityMap,
    record: ArtifactRecord,
) -> Result<()> {
    let expected_evidence_hash = record.evidence_hash.clone();
    let verified = PreparedArtifactBytes::new(record.artifact_bytes, record.evidence)?;
    let (bytes, evidence, evidence_hash) = verified.into_parts();
    if evidence_hash != expected_evidence_hash {
        return Err(StoreError::ArtifactEvidenceMismatch {
            artifact_id: evidence.artifact_id.clone(),
            field: "evidence_hash",
        }
        .into());
    }
    let key = (evidence.artifact_id.clone(), evidence_hash);
    match artifact_bytes.get(&key) {
        Some((stored_bytes, stored_evidence))
            if stored_bytes == &bytes && stored_evidence == &evidence => {}
        Some((_, stored_evidence)) => {
            return Err(StoreError::ArtifactEvidenceMismatch {
                artifact_id: stored_evidence.artifact_id.clone(),
                field: "artifact",
            }
            .into());
        }
        None => {
            artifact_bytes.insert(key, (bytes, evidence));
        }
    }
    Ok(())
}

async fn insert_fact_descriptor_projection_tx(
    tx: &mut Transaction<'_, Postgres>,
    projection: &mfm_store::v1::FactDescriptorProjection,
    events: &[KernelEventEnvelope],
    commit_id: &str,
) -> Result<()> {
    let event = event_for_id(events, &projection.source_event_id)?;
    let evidence_hash = descriptor_evidence_hash(event, projection)?;
    sqlx::query(
        "INSERT INTO fact_descriptor_index \
         (descriptor_hash, descriptor_artifact_id, descriptor_artifact_evidence_hash, \
          source_run_id, source_seq, source_ordinal, source_event_id, commit_id, fact_kind, \
          descriptor_schema_id, subject_schema_id, response_schema_id, \
          fact_subject_namespace_hash, compatibility_group) \
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14)",
    )
    .bind(projection.descriptor_hash.as_str())
    .bind(projection.descriptor_artifact_id.as_str())
    .bind(evidence_hash.as_str())
    .bind(event.run_id().as_str())
    .bind(u64_to_i64(
        event.seq().as_u64(),
        "fact_descriptor_index.source_seq",
    )?)
    .bind(i32::try_from(event.ordinal().as_u32()).map_err(|_| {
        PostgresStoreError::Corruption("fact_descriptor_index.source_ordinal overflow".into())
    })?)
    .bind(event.event_id().as_str())
    .bind(commit_id)
    .bind(projection.fact_kind.as_str())
    .bind(projection.descriptor_schema_id.as_str())
    .bind(projection.subject_schema_id.as_str())
    .bind(projection.response_schema_id.as_str())
    .bind(projection.fact_subject_namespace_hash.as_str())
    .bind(
        projection
            .compatibility_group
            .as_ref()
            .map(|group| group.as_str()),
    )
    .execute(&mut **tx)
    .await
    .map_err(|error| database_error("failed to insert fact descriptor projection", error))?;
    Ok(())
}

async fn insert_fact_index_projection_tx(
    tx: &mut Transaction<'_, Postgres>,
    projection: &mfm_store::v1::FactIndexProjection,
    commit_id: &str,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO fact_index \
         (source_run_id, source_seq, source_ordinal, source_event_id, commit_id, commit_key, \
          store_commit_order, recorded_at, observed_at, audience, visibility_scope, fact_kind, \
          fact_descriptor_hash, fact_subject_namespace_hash, fact_key, subject_material_hash, \
          request_schema_id, request_hash, response_schema_id, response_hash, \
          response_artifact_id, response_artifact_evidence_hash, capability_kind, \
          capability_version, adapter_kind, adapter_version) \
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20,$21,$22,$23,$24,$25,$26)",
    )
    .bind(projection.source_run_id.as_str())
    .bind(u64_to_i64(projection.source_seq, "fact_index.source_seq")?)
    .bind(i32::try_from(projection.source_ordinal).map_err(|_| {
        PostgresStoreError::Corruption("fact_index.source_ordinal overflow".into())
    })?)
    .bind(projection.source_event_id.as_str())
    .bind(commit_id)
    .bind(projection.commit_id.as_str())
    .bind(u64_to_i64(
        projection.store_commit_order,
        "fact_index.store_commit_order",
    )?)
    .bind(&projection.recorded_at)
    .bind(projection.observed_at.as_deref())
    .bind(fact_audience_tag(projection.audience))
    .bind(fact_visibility_scope_tag(projection.visibility_scope))
    .bind(projection.fact_kind.as_str())
    .bind(projection.fact_descriptor_hash.as_str())
    .bind(projection.fact_subject_namespace_hash.as_str())
    .bind(projection.fact_key.as_str())
    .bind(projection.subject_material_hash.as_str())
    .bind(
        projection
            .request_schema_id
            .as_ref()
            .map(|schema_id| schema_id.as_str()),
    )
    .bind(
        projection
            .request_hash
            .as_ref()
            .map(|digest| digest.as_str()),
    )
    .bind(projection.response_schema_id.as_str())
    .bind(projection.response_hash.as_str())
    .bind(projection.artifact_id.as_str())
    .bind(projection.artifact_evidence_hash.as_str())
    .bind(projection.capability_kind.as_str())
    .bind(projection.capability_version.as_str())
    .bind(projection.adapter_kind.as_str())
    .bind(projection.adapter_version.as_str())
    .execute(&mut **tx)
    .await
    .map_err(|error| database_error("failed to insert fact index projection", error))?;
    Ok(())
}

async fn insert_fact_term_projection_tx(
    tx: &mut Transaction<'_, Postgres>,
    projection: &mfm_store::v1::FactIndexTermProjection,
) -> Result<()> {
    let value = TermValueColumns::from_scalar(&projection.value);
    sqlx::query(
        "INSERT INTO fact_index_terms \
         (source_run_id, source_seq, source_ordinal, fact_descriptor_hash, field_id, source, \
          value_type, value_text, value_bool, value_i64, value_u64, value_decimal, \
          value_timestamp, value_digest, unit, scale) \
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16)",
    )
    .bind(projection.fact_claim_id.source_run_id().as_str())
    .bind(u64_to_i64(
        projection.fact_claim_id.source_seq(),
        "fact_index_terms.source_seq",
    )?)
    .bind(
        i32::try_from(projection.fact_claim_id.source_ordinal()).map_err(|_| {
            PostgresStoreError::Corruption("fact_index_terms.source_ordinal overflow".into())
        })?,
    )
    .bind(projection.fact_descriptor_hash.as_str())
    .bind(projection.field_id.as_str())
    .bind(fact_field_source_tag(projection.source))
    .bind(fact_field_value_type_tag(projection.value_type))
    .bind(value.value_text.as_deref())
    .bind(value.value_bool)
    .bind(value.value_i64)
    .bind(value.value_u64.as_deref())
    .bind(value.value_decimal.as_deref())
    .bind(value.value_timestamp.as_deref())
    .bind(value.value_digest.as_deref())
    .bind(projection.unit.as_ref().map(|unit| unit.as_str()))
    .bind(projection.scale.map(|scale| i32::from(scale.exponent())))
    .execute(&mut **tx)
    .await
    .map_err(|error| database_error("failed to insert fact term projection", error))?;
    Ok(())
}

async fn load_fact_descriptor_index_tx(
    tx: &mut Transaction<'_, Postgres>,
    run_id: &RunId,
) -> Result<BTreeMap<ContentDigest, mfm_store::v1::FactDescriptorProjection>> {
    let rows = sqlx::query(
        "SELECT descriptor_hash, descriptor_artifact_id, source_event_id, fact_kind, \
          descriptor_schema_id, subject_schema_id, response_schema_id, \
          fact_subject_namespace_hash, compatibility_group \
         FROM fact_descriptor_index WHERE source_run_id = $1 ORDER BY descriptor_hash",
    )
    .bind(run_id.as_str())
    .fetch_all(&mut **tx)
    .await
    .map_err(|error| database_error("failed to load fact descriptor projections", error))?;
    let mut projections = BTreeMap::new();
    for row in rows {
        let descriptor_hash = parse_identity::<ContentDigest>(&required_string(
            &row,
            "descriptor_hash",
            "fact_descriptor_index.descriptor_hash",
        )?)?;
        let projection = mfm_store::v1::FactDescriptorProjection {
            descriptor_hash: descriptor_hash.clone(),
            descriptor_artifact_id: parse_identity(&required_string(
                &row,
                "descriptor_artifact_id",
                "fact_descriptor_index.descriptor_artifact_id",
            )?)?,
            fact_kind: mfm_facts::FactKind::new(required_string(
                &row,
                "fact_kind",
                "fact_descriptor_index.fact_kind",
            )?)
            .map_err(fact_error)?,
            descriptor_schema_id: parse_identity(&required_string(
                &row,
                "descriptor_schema_id",
                "fact_descriptor_index.descriptor_schema_id",
            )?)?,
            subject_schema_id: parse_identity(&required_string(
                &row,
                "subject_schema_id",
                "fact_descriptor_index.subject_schema_id",
            )?)?,
            response_schema_id: parse_identity(&required_string(
                &row,
                "response_schema_id",
                "fact_descriptor_index.response_schema_id",
            )?)?,
            fact_subject_namespace_hash: parse_identity(&required_string(
                &row,
                "fact_subject_namespace_hash",
                "fact_descriptor_index.fact_subject_namespace_hash",
            )?)?,
            compatibility_group: optional_string(&row, "compatibility_group")?
                .map(mfm_facts::FactCompatibilityGroup::new)
                .transpose()
                .map_err(fact_error)?,
            source_event_id: parse_identity(&required_string(
                &row,
                "source_event_id",
                "fact_descriptor_index.source_event_id",
            )?)?,
        };
        projections.insert(descriptor_hash, projection);
    }
    Ok(projections)
}

async fn load_fact_index_tx(
    tx: &mut Transaction<'_, Postgres>,
    run_id: &RunId,
) -> Result<BTreeMap<mfm_facts::FactClaimId, mfm_store::v1::FactIndexProjection>> {
    let rows = sqlx::query(
        "SELECT source_run_id, source_seq, source_ordinal, source_event_id, commit_id, \
          commit_key, store_commit_order, recorded_at, observed_at, audience, visibility_scope, \
          fact_kind, fact_descriptor_hash, fact_subject_namespace_hash, fact_key, \
          subject_material_hash, request_schema_id, request_hash, response_schema_id, \
          response_hash, response_artifact_id, response_artifact_evidence_hash, \
          capability_kind, capability_version, adapter_kind, adapter_version \
         FROM fact_index WHERE source_run_id = $1 ORDER BY source_seq, source_ordinal",
    )
    .bind(run_id.as_str())
    .fetch_all(&mut **tx)
    .await
    .map_err(|error| database_error("failed to load fact index projections", error))?;
    let mut projections = BTreeMap::new();
    for row in rows {
        let source_run_id = parse_identity::<RunId>(&required_string(
            &row,
            "source_run_id",
            "fact_index.source_run_id",
        )?)?;
        let source_seq = i64_to_positive_u64(
            required_i64(&row, "source_seq", "fact_index.source_seq")?,
            "fact_index.source_seq",
        )?;
        let source_ordinal = i64_to_u32(
            required_i32(&row, "source_ordinal", "fact_index.source_ordinal")?.into(),
            "fact_index.source_ordinal",
        )?;
        let claim_id =
            mfm_facts::FactClaimId::new(source_run_id.clone(), source_seq, source_ordinal)
                .map_err(fact_error)?;
        let projection = mfm_store::v1::FactIndexProjection {
            fact_claim_id: claim_id.clone(),
            source_run_id,
            source_seq,
            source_ordinal,
            source_event_id: parse_identity(&required_string(
                &row,
                "source_event_id",
                "fact_index.source_event_id",
            )?)?,
            commit_id: CommitKey::new(required_string(
                &row,
                "commit_key",
                "fact_index.commit_key",
            )?)?,
            store_commit_order: i64_to_positive_u64(
                required_i64(&row, "store_commit_order", "fact_index.store_commit_order")?,
                "fact_index.store_commit_order",
            )?,
            recorded_at: required_string(&row, "recorded_at", "fact_index.recorded_at")?,
            observed_at: optional_string(&row, "observed_at")?,
            audience: parse_fact_audience(&required_string(
                &row,
                "audience",
                "fact_index.audience",
            )?)?,
            visibility_scope: parse_fact_visibility_scope(&required_string(
                &row,
                "visibility_scope",
                "fact_index.visibility_scope",
            )?)?,
            fact_kind: mfm_facts::FactKind::new(required_string(
                &row,
                "fact_kind",
                "fact_index.fact_kind",
            )?)
            .map_err(fact_error)?,
            fact_descriptor_hash: parse_identity(&required_string(
                &row,
                "fact_descriptor_hash",
                "fact_index.fact_descriptor_hash",
            )?)?,
            fact_subject_namespace_hash: parse_identity(&required_string(
                &row,
                "fact_subject_namespace_hash",
                "fact_index.fact_subject_namespace_hash",
            )?)?,
            fact_key: mfm_facts::FactKey::from_digest(parse_identity(&required_string(
                &row,
                "fact_key",
                "fact_index.fact_key",
            )?)?),
            subject_material_hash: parse_identity(&required_string(
                &row,
                "subject_material_hash",
                "fact_index.subject_material_hash",
            )?)?,
            request_schema_id: parse_optional_identity(optional_string(
                &row,
                "request_schema_id",
            )?)?,
            request_hash: parse_optional_identity(optional_string(&row, "request_hash")?)?,
            response_schema_id: parse_identity(&required_string(
                &row,
                "response_schema_id",
                "fact_index.response_schema_id",
            )?)?,
            response_hash: parse_identity(&required_string(
                &row,
                "response_hash",
                "fact_index.response_hash",
            )?)?,
            artifact_id: parse_identity(&required_string(
                &row,
                "response_artifact_id",
                "fact_index.response_artifact_id",
            )?)?,
            artifact_evidence_hash: parse_identity(&required_string(
                &row,
                "response_artifact_evidence_hash",
                "fact_index.response_artifact_evidence_hash",
            )?)?,
            capability_kind: parse_identity(&required_string(
                &row,
                "capability_kind",
                "fact_index.capability_kind",
            )?)?,
            capability_version: required_string(
                &row,
                "capability_version",
                "fact_index.capability_version",
            )?
            .parse()
            .map_err(|error| PostgresStoreError::Corruption(format!("{error}")))?,
            adapter_kind: parse_identity(&required_string(
                &row,
                "adapter_kind",
                "fact_index.adapter_kind",
            )?)?,
            adapter_version: required_string(
                &row,
                "adapter_version",
                "fact_index.adapter_version",
            )?
            .parse()
            .map_err(|error| PostgresStoreError::Corruption(format!("{error}")))?,
        };
        projections.insert(claim_id, projection);
    }
    Ok(projections)
}

async fn load_fact_index_terms_tx(
    tx: &mut Transaction<'_, Postgres>,
    run_id: &RunId,
) -> Result<
    BTreeMap<
        (mfm_facts::FactClaimId, mfm_facts::FactFieldId),
        mfm_store::v1::FactIndexTermProjection,
    >,
> {
    let rows = sqlx::query(
        "SELECT source_run_id, source_seq, source_ordinal, fact_descriptor_hash, field_id, \
          source, value_type, value_text, value_bool, value_i64, value_u64, value_decimal, \
          value_timestamp, value_digest, unit, scale \
         FROM fact_index_terms WHERE source_run_id = $1 \
         ORDER BY source_seq, source_ordinal, field_id",
    )
    .bind(run_id.as_str())
    .fetch_all(&mut **tx)
    .await
    .map_err(|error| database_error("failed to load fact term projections", error))?;
    let mut projections = BTreeMap::new();
    for row in rows {
        let source_run_id = parse_identity::<RunId>(&required_string(
            &row,
            "source_run_id",
            "fact_index_terms.source_run_id",
        )?)?;
        let source_seq = i64_to_positive_u64(
            required_i64(&row, "source_seq", "fact_index_terms.source_seq")?,
            "fact_index_terms.source_seq",
        )?;
        let source_ordinal = i64_to_u32(
            required_i32(&row, "source_ordinal", "fact_index_terms.source_ordinal")?.into(),
            "fact_index_terms.source_ordinal",
        )?;
        let claim_id = mfm_facts::FactClaimId::new(source_run_id, source_seq, source_ordinal)
            .map_err(fact_error)?;
        let field_id = mfm_facts::FactFieldId::new(required_string(
            &row,
            "field_id",
            "fact_index_terms.field_id",
        )?)
        .map_err(fact_error)?;
        let value_type = parse_fact_field_value_type(&required_string(
            &row,
            "value_type",
            "fact_index_terms.value_type",
        )?)?;
        let value = parse_term_value(&row, value_type)?;
        let projection = mfm_store::v1::FactIndexTermProjection {
            fact_claim_id: claim_id.clone(),
            fact_descriptor_hash: parse_identity(&required_string(
                &row,
                "fact_descriptor_hash",
                "fact_index_terms.fact_descriptor_hash",
            )?)?,
            field_id: field_id.clone(),
            source: parse_fact_field_source(&required_string(
                &row,
                "source",
                "fact_index_terms.source",
            )?)?,
            value_type,
            value,
            unit: optional_string(&row, "unit")?
                .map(mfm_facts::FactUnit::new)
                .transpose()
                .map_err(fact_error)?,
            scale: optional_i32(&row, "scale")?
                .map(|value| {
                    i16::try_from(value)
                        .map_err(|_| {
                            PostgresStoreError::Corruption(
                                "fact_index_terms.scale overflow".to_owned(),
                            )
                        })
                        .and_then(|value| mfm_facts::FactScale::new(value).map_err(fact_error))
                })
                .transpose()?,
        };
        projections.insert((claim_id, field_id), projection);
    }
    Ok(projections)
}

fn descriptor_evidence_hash(
    event: &KernelEventEnvelope,
    projection: &mfm_store::v1::FactDescriptorProjection,
) -> Result<ContentDigest> {
    let events::KernelEventPayload::RunAdmitted(payload) = event.payload() else {
        return Err(PostgresStoreError::Corruption(
            "fact descriptor projection did not point at RunAdmitted".to_owned(),
        ));
    };
    let artifact = payload
        .fact_descriptor_artifacts
        .iter()
        .find(|artifact| {
            artifact.artifact_id == projection.descriptor_artifact_id
                && artifact.content_digest == projection.descriptor_hash
        })
        .ok_or_else(|| {
            PostgresStoreError::Corruption(
                "fact descriptor projection missing RunAdmitted artifact evidence".to_owned(),
            )
        })?;
    store_artifact_from_run_artifact(artifact)
        .evidence_hash()
        .map_err(Into::into)
}

fn event_for_id<'a>(
    events: &'a [KernelEventEnvelope],
    event_id: &mfm_ids::EventId,
) -> Result<&'a KernelEventEnvelope> {
    events
        .iter()
        .find(|event| event.event_id() == event_id)
        .ok_or_else(|| {
            PostgresStoreError::Corruption(format!("event {event_id} missing from staged batch"))
        })
}

fn store_artifact_from_run_artifact(
    artifact: &events::RunArtifactEvidenceRef,
) -> ArtifactEvidenceRef {
    ArtifactEvidenceRef {
        artifact_id: artifact.artifact_id.clone(),
        digest: artifact.content_digest.clone(),
        byte_len: artifact.byte_len,
        media_type: artifact.media_type.clone(),
        schema_id: artifact.schema_id.clone(),
        semantic_type_id: artifact.semantic_type_id.clone(),
        producer_node_id: None,
        producer_seed_id: None,
        artifact_role: artifact.role,
    }
}

struct TermValueColumns {
    value_text: Option<String>,
    value_bool: Option<bool>,
    value_i64: Option<i64>,
    value_u64: Option<String>,
    value_decimal: Option<String>,
    value_timestamp: Option<String>,
    value_digest: Option<String>,
}

impl TermValueColumns {
    fn from_scalar(value: &mfm_facts::FactCanonicalScalar) -> Self {
        let mut columns = Self {
            value_text: None,
            value_bool: None,
            value_i64: None,
            value_u64: None,
            value_decimal: None,
            value_timestamp: None,
            value_digest: None,
        };
        match value {
            mfm_facts::FactCanonicalScalar::String(value) => {
                columns.value_text = Some(value.clone())
            }
            mfm_facts::FactCanonicalScalar::Boolean(value) => columns.value_bool = Some(*value),
            mfm_facts::FactCanonicalScalar::SignedInteger(value) => {
                columns.value_i64 = Some(*value);
            }
            mfm_facts::FactCanonicalScalar::UnsignedInteger(value) => {
                columns.value_u64 = Some(value.to_string());
            }
            mfm_facts::FactCanonicalScalar::Timestamp(value) => {
                columns.value_timestamp = Some(value.clone());
            }
            mfm_facts::FactCanonicalScalar::DecimalString(value) => {
                columns.value_decimal = Some(value.as_str().to_owned());
            }
            mfm_facts::FactCanonicalScalar::Digest(value) => {
                columns.value_digest = Some(value.as_str().to_owned());
            }
        }
        columns
    }
}

pub(super) fn parse_term_value(
    row: &PgRow,
    value_type: mfm_facts::FactFieldValueType,
) -> Result<mfm_facts::FactCanonicalScalar> {
    match value_type {
        mfm_facts::FactFieldValueType::String => {
            required_string(row, "value_text", "fact_index_terms.value_text")
                .map(mfm_facts::FactCanonicalScalar::String)
        }
        mfm_facts::FactFieldValueType::Boolean => {
            required_bool(row, "value_bool", "fact_index_terms.value_bool")
                .map(mfm_facts::FactCanonicalScalar::Boolean)
        }
        mfm_facts::FactFieldValueType::SignedInteger => {
            required_i64(row, "value_i64", "fact_index_terms.value_i64")
                .map(mfm_facts::FactCanonicalScalar::SignedInteger)
        }
        mfm_facts::FactFieldValueType::UnsignedInteger => {
            let value = required_string(row, "value_u64", "fact_index_terms.value_u64")?
                .parse::<u64>()
                .map_err(|_| {
                    PostgresStoreError::Corruption(
                        "fact_index_terms.value_u64 was not a u64".to_owned(),
                    )
                })?;
            Ok(mfm_facts::FactCanonicalScalar::UnsignedInteger(value))
        }
        mfm_facts::FactFieldValueType::Timestamp => mfm_facts::FactCanonicalScalar::timestamp(
            required_string(row, "value_timestamp", "fact_index_terms.value_timestamp")?,
        )
        .map_err(fact_error),
        mfm_facts::FactFieldValueType::DecimalString => {
            mfm_facts::FactCanonicalScalar::decimal_variable(required_string(
                row,
                "value_decimal",
                "fact_index_terms.value_decimal",
            )?)
            .map_err(fact_error)
        }
        mfm_facts::FactFieldValueType::Digest => {
            let digest = parse_identity(&required_string(
                row,
                "value_digest",
                "fact_index_terms.value_digest",
            )?)?;
            Ok(mfm_facts::FactCanonicalScalar::Digest(digest))
        }
    }
}

pub(super) fn fact_audience_tag(value: mfm_facts::FactAudience) -> &'static str {
    match value {
        mfm_facts::FactAudience::Control => "control",
        mfm_facts::FactAudience::Platform => "platform",
    }
}

pub(super) fn parse_fact_audience(value: &str) -> Result<mfm_facts::FactAudience> {
    match value {
        "control" => Ok(mfm_facts::FactAudience::Control),
        "platform" => Ok(mfm_facts::FactAudience::Platform),
        _ => Err(PostgresStoreError::Corruption(format!(
            "unknown fact audience {value}"
        ))),
    }
}

pub(super) fn fact_visibility_scope_tag(value: mfm_facts::FactVisibilityScope) -> &'static str {
    match value {
        mfm_facts::FactVisibilityScope::Default => "default",
    }
}

pub(super) fn parse_fact_visibility_scope(value: &str) -> Result<mfm_facts::FactVisibilityScope> {
    match value {
        "default" => Ok(mfm_facts::FactVisibilityScope::Default),
        _ => Err(PostgresStoreError::Corruption(format!(
            "unknown fact visibility scope {value}"
        ))),
    }
}

fn fact_field_source_tag(value: mfm_facts::FactFieldSource) -> &'static str {
    match value {
        mfm_facts::FactFieldSource::Subject => "subject",
        mfm_facts::FactFieldSource::Result => "result",
        mfm_facts::FactFieldSource::Metadata => "metadata",
    }
}

fn parse_fact_field_source(value: &str) -> Result<mfm_facts::FactFieldSource> {
    match value {
        "subject" => Ok(mfm_facts::FactFieldSource::Subject),
        "result" => Ok(mfm_facts::FactFieldSource::Result),
        "metadata" => Ok(mfm_facts::FactFieldSource::Metadata),
        _ => Err(PostgresStoreError::Corruption(format!(
            "unknown fact field source {value}"
        ))),
    }
}

fn fact_field_value_type_tag(value: mfm_facts::FactFieldValueType) -> &'static str {
    match value {
        mfm_facts::FactFieldValueType::String => "string",
        mfm_facts::FactFieldValueType::Boolean => "boolean",
        mfm_facts::FactFieldValueType::SignedInteger => "signed_integer",
        mfm_facts::FactFieldValueType::UnsignedInteger => "unsigned_integer",
        mfm_facts::FactFieldValueType::Timestamp => "timestamp",
        mfm_facts::FactFieldValueType::DecimalString => "decimal_string",
        mfm_facts::FactFieldValueType::Digest => "digest",
    }
}

pub(super) fn parse_fact_field_value_type(value: &str) -> Result<mfm_facts::FactFieldValueType> {
    match value {
        "string" => Ok(mfm_facts::FactFieldValueType::String),
        "boolean" => Ok(mfm_facts::FactFieldValueType::Boolean),
        "signed_integer" => Ok(mfm_facts::FactFieldValueType::SignedInteger),
        "unsigned_integer" => Ok(mfm_facts::FactFieldValueType::UnsignedInteger),
        "timestamp" => Ok(mfm_facts::FactFieldValueType::Timestamp),
        "decimal_string" => Ok(mfm_facts::FactFieldValueType::DecimalString),
        "digest" => Ok(mfm_facts::FactFieldValueType::Digest),
        _ => Err(PostgresStoreError::Corruption(format!(
            "unknown fact field value type {value}"
        ))),
    }
}

fn required_string(row: &PgRow, column: &str, field: &'static str) -> Result<String> {
    row.try_get::<String, _>(column)
        .map_err(|_| PostgresStoreError::Corruption(format!("{field} was missing or invalid")))
}

fn optional_string(row: &PgRow, column: &str) -> Result<Option<String>> {
    row.try_get::<Option<String>, _>(column)
        .map_err(|_| PostgresStoreError::Corruption(format!("{column} was invalid")))
}

fn required_bool(row: &PgRow, column: &str, field: &'static str) -> Result<bool> {
    row.try_get::<bool, _>(column)
        .map_err(|_| PostgresStoreError::Corruption(format!("{field} was missing or invalid")))
}

fn required_i64(row: &PgRow, column: &str, field: &'static str) -> Result<i64> {
    row.try_get::<i64, _>(column)
        .map_err(|_| PostgresStoreError::Corruption(format!("{field} was missing or invalid")))
}

fn required_i32(row: &PgRow, column: &str, field: &'static str) -> Result<i32> {
    row.try_get::<i32, _>(column)
        .map_err(|_| PostgresStoreError::Corruption(format!("{field} was missing or invalid")))
}

fn optional_i32(row: &PgRow, column: &str) -> Result<Option<i32>> {
    row.try_get::<Option<i32>, _>(column)
        .map_err(|_| PostgresStoreError::Corruption(format!("{column} was invalid")))
}

fn i64_to_u32(value: i64, field: &'static str) -> Result<u32> {
    u32::try_from(value)
        .map_err(|_| PostgresStoreError::Corruption(format!("{field} outside u32 range")))
}

pub(super) fn fact_error(error: mfm_facts::FactDescriptorError) -> PostgresStoreError {
    StoreError::Identity(error.to_string()).into()
}
