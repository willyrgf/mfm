use super::*;

#[path = "fact_projection_terms.rs"]
mod fact_projection_terms;
pub(super) use self::fact_projection_terms::{
    descriptor_admission_evidence_hash, descriptor_projection_evidence_hash, fact_error,
    fact_term_value_from_reader, parse_fact_audience, parse_fact_field_source,
    parse_fact_visibility_scope, push_fact_index_term_projection_select_list, TermValueColumns,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PhysicalFactProjections {
    pub(super) fact_descriptors: BTreeMap<ContentDigest, mfm_store::v1::FactDescriptorProjection>,
    pub(super) fact_descriptor_admissions:
        BTreeMap<(RunId, ContentDigest), FactDescriptorAdmissionProjection>,
    pub(super) fact_records: BTreeMap<mfm_facts::FactClaimId, mfm_store::v1::FactRecordProjection>,
    pub(super) fact_index_entries:
        BTreeMap<mfm_facts::FactClaimId, mfm_store::v1::FactIndexProjection>,
    pub(super) fact_term_entries: BTreeMap<
        (mfm_facts::FactClaimId, mfm_facts::FactFieldId),
        mfm_store::v1::FactIndexTermProjection,
    >,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct FactDescriptorAdmissionProjection {
    pub(super) run_id: RunId,
    pub(super) descriptor_hash: ContentDigest,
    pub(super) descriptor_artifact_id: ArtifactId,
    pub(super) descriptor_artifact_evidence: ArtifactEvidenceRef,
    pub(super) source_seq: u64,
    pub(super) source_ordinal: u32,
    pub(super) source_event_id: mfm_ids::EventId,
}

impl PhysicalFactProjections {
    pub(super) fn from_snapshot_and_stream(
        snapshot: &ProjectionSnapshot,
        stream: &[KernelEventEnvelope],
    ) -> Result<Self> {
        let mut fact_descriptor_admissions = BTreeMap::new();
        for event in stream {
            if let events::KernelEventPayload::RunAdmitted(payload) = event.payload() {
                for artifact in &payload.fact_descriptor_artifacts {
                    let descriptor_hash = artifact.content_digest.clone();
                    let projection =
                        snapshot.fact_descriptor(&descriptor_hash).ok_or_else(|| {
                            PostgresStoreError::Corruption(format!(
                                "rebuilt descriptor {descriptor_hash} missing catalog projection",
                            ))
                        })?;
                    let evidence = ArtifactEvidenceRef::from_run_artifact(artifact);
                    if projection.descriptor_artifact_id != evidence.artifact_id
                        || projection.descriptor_artifact_evidence != evidence
                    {
                        return Err(StoreError::ArtifactEvidenceMismatch {
                            artifact_id: evidence.artifact_id,
                            field: "fact_descriptor",
                        }
                        .into());
                    }
                    let admission = FactDescriptorAdmissionProjection {
                        run_id: event.run_id().clone(),
                        descriptor_hash: descriptor_hash.clone(),
                        descriptor_artifact_id: projection.descriptor_artifact_id.clone(),
                        descriptor_artifact_evidence: projection
                            .descriptor_artifact_evidence
                            .clone(),
                        source_seq: event.seq().as_u64(),
                        source_ordinal: event.ordinal().as_u32(),
                        source_event_id: event.event_id().clone(),
                    };
                    fact_descriptor_admissions
                        .insert((event.run_id().clone(), descriptor_hash), admission);
                }
            }
        }
        Ok(Self {
            fact_descriptors: snapshot
                .fact_descriptors()
                .map(|(descriptor_hash, projection)| (descriptor_hash.clone(), projection.clone()))
                .collect(),
            fact_descriptor_admissions,
            fact_records: snapshot
                .fact_records()
                .map(|(claim_id, projection)| (claim_id.clone(), projection.clone()))
                .collect(),
            fact_index_entries: snapshot
                .fact_index_entries()
                .map(|(claim_id, projection)| (claim_id.clone(), projection.clone()))
                .collect(),
            fact_term_entries: snapshot
                .fact_term_entries()
                .map(|(key, projection)| (key.clone(), projection.clone()))
                .collect(),
        })
    }
}

pub(super) async fn insert_fact_projection_rows_tx(
    tx: &mut Transaction<'_, Postgres>,
    before: &ProjectionSnapshot,
    after: &ProjectionSnapshot,
    events: &[KernelEventEnvelope],
    commit_id: &str,
) -> Result<()> {
    for event in events {
        if let events::KernelEventPayload::RunAdmitted(payload) = event.payload() {
            for artifact in &payload.fact_descriptor_artifacts {
                let projection =
                    after
                        .fact_descriptor(&artifact.content_digest)
                        .ok_or_else(|| {
                            PostgresStoreError::Corruption(format!(
                                "RunAdmitted descriptor {} missing staged projection",
                                artifact.content_digest
                            ))
                        })?;
                insert_fact_descriptor_projection_tx(tx, projection).await?;
                insert_run_fact_descriptor_admission_projection_tx(
                    tx, event, projection, commit_id,
                )
                .await?;
            }
        }
    }
    for (claim_id, projection) in after.fact_index_entries() {
        if before.fact_index_entry(claim_id).is_none() {
            insert_fact_index_projection_tx(tx, projection, commit_id).await?;
        }
    }
    for (key, projection) in after.fact_term_entries() {
        let (claim_id, field_id) = key;
        if before.fact_term(claim_id, field_id).is_none() {
            insert_fact_term_projection_tx(tx, projection).await?;
        }
    }
    Ok(())
}

pub(super) async fn load_fact_projection_tables_tx(
    tx: &mut Transaction<'_, Postgres>,
    run_id: &RunId,
) -> Result<PhysicalFactProjections> {
    load_fact_projection_tables_scoped_tx(tx, Some(run_id)).await
}

async fn load_fact_projection_tables_scoped_tx(
    tx: &mut Transaction<'_, Postgres>,
    run_id: Option<&RunId>,
) -> Result<PhysicalFactProjections> {
    let fact_index_entries = load_fact_index_tx(tx, run_id).await?;
    let fact_records = load_fact_record_projections_tx(tx, run_id).await?;
    let projections = PhysicalFactProjections {
        fact_descriptors: load_fact_descriptor_index_tx(tx, run_id).await?,
        fact_descriptor_admissions: load_run_fact_descriptor_admissions_tx(tx, run_id).await?,
        fact_records,
        fact_index_entries,
        fact_term_entries: load_fact_index_terms_tx(tx, run_id).await?,
    };
    validate_physical_fact_projections(&projections)?;
    Ok(projections)
}

fn validate_physical_fact_projections(projections: &PhysicalFactProjections) -> Result<()> {
    for ((run_id, descriptor_hash), admission) in &projections.fact_descriptor_admissions {
        if run_id != &admission.run_id || descriptor_hash != &admission.descriptor_hash {
            return Err(PostgresStoreError::Corruption(format!(
                "run fact descriptor admission key {run_id}/{descriptor_hash} does not match row identity",
            )));
        }
        let Some(descriptor) = projections.fact_descriptors.get(descriptor_hash) else {
            return Err(PostgresStoreError::Corruption(format!(
                "run fact descriptor admission {run_id}/{descriptor_hash} references missing descriptor row",
            )));
        };
        if descriptor.descriptor_artifact_id != admission.descriptor_artifact_id
            || descriptor.descriptor_artifact_evidence != admission.descriptor_artifact_evidence
        {
            return Err(PostgresStoreError::Corruption(format!(
                "run fact descriptor admission {run_id}/{descriptor_hash} does not match descriptor catalog artifact",
            )));
        }
    }
    for (claim_id, record) in &projections.fact_records {
        if !projections
            .fact_descriptors
            .contains_key(record.claim.fact_descriptor_hash())
        {
            return Err(PostgresStoreError::Corruption(format!(
                "fact record {:?} references missing descriptor row",
                claim_id
            )));
        }
        let admission_key = (
            record.source_run_id.clone(),
            record.claim.fact_descriptor_hash().clone(),
        );
        if !projections
            .fact_descriptor_admissions
            .contains_key(&admission_key)
        {
            return Err(PostgresStoreError::Corruption(format!(
                "fact record {:?} references descriptor not admitted by run",
                claim_id
            )));
        }
        let is_indexed = matches!(
            record.claim.visibility(),
            mfm_facts::FactVisibility::Indexed { .. }
        );
        match (is_indexed, projections.fact_index_entries.get(claim_id)) {
            (true, None) => {
                return Err(PostgresStoreError::Corruption(format!(
                    "indexed fact record {:?} has no fact_index row",
                    claim_id
                )));
            }
            (true, Some(index)) if !record.matches_index_projection(index) => {
                return Err(PostgresStoreError::Corruption(format!(
                    "fact_index row {:?} does not match its FactRecorded payload",
                    claim_id
                )));
            }
            (false, Some(_)) => {
                return Err(PostgresStoreError::Corruption(format!(
                    "private fact record {:?} unexpectedly has a fact_index row",
                    claim_id
                )));
            }
            _ => {}
        }
    }
    for (claim_id, index) in &projections.fact_index_entries {
        if !projections.fact_records.contains_key(claim_id) {
            return Err(PostgresStoreError::Corruption(format!(
                "fact_index row {:?} has no FactRecorded payload",
                claim_id
            )));
        }
        if !projections
            .fact_descriptors
            .contains_key(&index.fact_descriptor_hash)
        {
            return Err(PostgresStoreError::Corruption(format!(
                "fact_index row {:?} references missing descriptor row",
                claim_id
            )));
        }
    }
    for ((claim_id, field_id), term) in &projections.fact_term_entries {
        let Some(index) = projections.fact_index_entries.get(claim_id) else {
            return Err(PostgresStoreError::Corruption(format!(
                "fact_index_terms row {:?} has no fact_index row",
                claim_id
            )));
        };
        if term.fact_descriptor_hash != index.fact_descriptor_hash {
            return Err(PostgresStoreError::Corruption(format!(
                "fact_index_terms row {:?}/{} references descriptor {} but parent fact_index row references {}",
                claim_id, field_id, term.fact_descriptor_hash, index.fact_descriptor_hash
            )));
        }
    }
    Ok(())
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
    let expected = PhysicalFactProjections::from_snapshot_and_stream(&rebuilt, stream)?;
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
                    let evidence = ArtifactEvidenceRef::from_run_artifact(artifact);
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
    projections: &ProjectionSnapshot,
    artifact_bytes: &mut ArtifactByteAuthorityMap,
) -> Result<()> {
    for (_, projection) in projections.fact_descriptors() {
        load_fact_descriptor_artifact_bytes_for_projection_tx(tx, projection, artifact_bytes)
            .await?;
    }
    Ok(())
}

async fn load_fact_descriptor_artifact_bytes_for_projection_tx(
    tx: &mut Transaction<'_, Postgres>,
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
         INNER JOIN artifact_blobs b \
           ON b.artifact_id = a.artifact_id \
          AND b.digest = a.digest \
          AND b.byte_len = a.byte_len \
         WHERE f.descriptor_hash = $1",
    )
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
    sqlx::query("DELETE FROM run_fact_descriptor_admissions WHERE run_id = $1")
        .bind(run_id.as_str())
        .execute(&mut **tx)
        .await
        .map_err(|error| {
            database_error("failed to delete run fact descriptor admissions", error)
        })?;
    Ok(())
}

#[cfg(all(test, feature = "parity-tests"))]
async fn insert_rebuilt_fact_projection_rows_tx(
    tx: &mut Transaction<'_, Postgres>,
    snapshot: &ProjectionSnapshot,
    events: &[KernelEventEnvelope],
    commit_ids_by_event: &BTreeMap<mfm_ids::EventId, String>,
) -> Result<()> {
    for event in events {
        if let events::KernelEventPayload::RunAdmitted(payload) = event.payload() {
            let commit_id = required_commit_id(commit_ids_by_event, event.event_id())?;
            for artifact in &payload.fact_descriptor_artifacts {
                let projection = snapshot
                    .fact_descriptor(&artifact.content_digest)
                    .ok_or_else(|| {
                        PostgresStoreError::Corruption(format!(
                            "rebuilt descriptor {} missing catalog projection",
                            artifact.content_digest
                        ))
                    })?;
                insert_fact_descriptor_projection_tx(tx, projection).await?;
                insert_run_fact_descriptor_admission_projection_tx(
                    tx, event, projection, commit_id,
                )
                .await?;
            }
        }
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
        let row = PgRowReader::new(&row, "run_events");
        let event_id = row.required_identity::<mfm_ids::EventId>("event_id")?;
        let commit_id = row.required_string("commit_id")?;
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
) -> Result<()> {
    let evidence_hash = descriptor_projection_evidence_hash(projection)?;
    sqlx::query(
        "INSERT INTO fact_descriptor_index \
         (descriptor_hash, descriptor_artifact_id, descriptor_artifact_evidence_hash, \
          fact_kind, descriptor_schema_id, subject_schema_id, response_schema_id, \
          fact_subject_namespace_hash) \
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8) \
         ON CONFLICT (descriptor_hash) DO NOTHING",
    )
    .bind(projection.descriptor_hash.as_str())
    .bind(projection.descriptor_artifact_id.as_str())
    .bind(evidence_hash.as_str())
    .bind(projection.fact_kind.as_str())
    .bind(projection.descriptor_schema_id.as_str())
    .bind(projection.subject_schema_id.as_str())
    .bind(projection.response_schema_id.as_str())
    .bind(projection.fact_subject_namespace_hash.as_str())
    .execute(&mut **tx)
    .await
    .map_err(|error| database_error("failed to insert fact descriptor projection", error))?;
    verify_fact_descriptor_projection_tx(tx, projection).await?;
    Ok(())
}

async fn verify_fact_descriptor_projection_tx(
    tx: &mut Transaction<'_, Postgres>,
    projection: &mfm_store::v1::FactDescriptorProjection,
) -> Result<()> {
    let stored = load_fact_descriptor_projection_tx(tx, &projection.descriptor_hash).await?;
    if &stored == projection {
        return Ok(());
    }
    Err(PostgresStoreError::Corruption(format!(
        "fact descriptor catalog row {} conflicts with staged descriptor projection",
        projection.descriptor_hash
    )))
}

async fn insert_run_fact_descriptor_admission_projection_tx(
    tx: &mut Transaction<'_, Postgres>,
    event: &KernelEventEnvelope,
    projection: &mfm_store::v1::FactDescriptorProjection,
    commit_id: &str,
) -> Result<()> {
    let evidence_hash = descriptor_admission_evidence_hash(event, projection)?;
    sqlx::query(
        "INSERT INTO run_fact_descriptor_admissions \
         (run_id, descriptor_hash, descriptor_artifact_id, descriptor_artifact_evidence_hash, \
          source_seq, source_ordinal, source_event_id, commit_id) \
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8)",
    )
    .bind(event.run_id().as_str())
    .bind(projection.descriptor_hash.as_str())
    .bind(projection.descriptor_artifact_id.as_str())
    .bind(evidence_hash.as_str())
    .bind(u64_to_i64(
        event.seq().as_u64(),
        "run_fact_descriptor_admissions.source_seq",
    )?)
    .bind(i32::try_from(event.ordinal().as_u32()).map_err(|_| {
        PostgresStoreError::Corruption(
            "run_fact_descriptor_admissions.source_ordinal overflow".into(),
        )
    })?)
    .bind(event.event_id().as_str())
    .bind(commit_id)
    .execute(&mut **tx)
    .await
    .map_err(|error| {
        database_error(
            "failed to insert run fact descriptor admission projection",
            error,
        )
    })?;
    Ok(())
}

async fn insert_fact_index_projection_tx(
    tx: &mut Transaction<'_, Postgres>,
    projection: &mfm_store::v1::FactIndexProjection,
    commit_id: &str,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO fact_index \
         (source_run_id, source_seq, source_ordinal, source_event_id, producer_node_id, commit_id, commit_key, \
          store_commit_order, recorded_at, observed_at, audience, visibility_scope, fact_kind, \
          fact_descriptor_hash, fact_subject_namespace_hash, fact_key, subject_material_hash, \
          request_schema_id, request_hash, response_schema_id, response_hash, \
          response_artifact_id, response_artifact_evidence_hash, capability_kind, \
          capability_version, adapter_kind, adapter_version) \
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20,$21,$22,$23,$24,$25,$26,$27)",
    )
    .bind(projection.source_run_id.as_str())
    .bind(u64_to_i64(projection.source_seq, "fact_index.source_seq")?)
    .bind(i32::try_from(projection.source_ordinal).map_err(|_| {
        PostgresStoreError::Corruption("fact_index.source_ordinal overflow".into())
    })?)
    .bind(projection.source_event_id.as_str())
    .bind(projection.producer_node_id.as_str())
    .bind(commit_id)
    .bind(projection.commit_id.as_str())
    .bind(u64_to_i64(
        projection.store_commit_order,
        "fact_index.store_commit_order",
    )?)
    .bind(&projection.recorded_at)
    .bind(projection.observed_at.as_deref())
    .bind(projection.audience.as_str())
    .bind(projection.visibility_scope.as_str())
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
    .bind(projection.source.path_prefix())
    .bind(projection.value_type.as_str())
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
    run_id: Option<&RunId>,
) -> Result<BTreeMap<ContentDigest, mfm_store::v1::FactDescriptorProjection>> {
    let sql = if run_id.is_some() {
        "SELECT f.descriptor_hash, f.descriptor_artifact_id, f.descriptor_artifact_evidence_hash, \
         f.fact_kind, f.descriptor_schema_id, f.subject_schema_id, f.response_schema_id, \
         f.fact_subject_namespace_hash \
         FROM run_fact_descriptor_admissions r \
         INNER JOIN fact_descriptor_index f \
           ON f.descriptor_hash = r.descriptor_hash \
          AND f.descriptor_artifact_id = r.descriptor_artifact_id \
          AND f.descriptor_artifact_evidence_hash = r.descriptor_artifact_evidence_hash \
         WHERE r.run_id = $1 ORDER BY f.descriptor_hash"
    } else {
        "SELECT descriptor_hash, descriptor_artifact_id, descriptor_artifact_evidence_hash, \
         fact_kind, descriptor_schema_id, subject_schema_id, response_schema_id, fact_subject_namespace_hash \
         FROM fact_descriptor_index ORDER BY descriptor_hash"
    };
    let mut query = sqlx::query(sql);
    if let Some(run_id) = run_id {
        query = query.bind(run_id.as_str());
    }
    let rows = query
        .fetch_all(&mut **tx)
        .await
        .map_err(|error| database_error("failed to load fact descriptor projections", error))?;
    fact_descriptor_projections_from_rows(tx, rows).await
}

async fn load_fact_descriptor_projection_tx(
    tx: &mut Transaction<'_, Postgres>,
    descriptor_hash: &ContentDigest,
) -> Result<mfm_store::v1::FactDescriptorProjection> {
    let rows = sqlx::query(
        "SELECT descriptor_hash, descriptor_artifact_id, descriptor_artifact_evidence_hash, \
         fact_kind, descriptor_schema_id, subject_schema_id, response_schema_id, \
         fact_subject_namespace_hash \
         FROM fact_descriptor_index WHERE descriptor_hash = $1",
    )
    .bind(descriptor_hash.as_str())
    .fetch_all(&mut **tx)
    .await
    .map_err(|error| database_error("failed to load fact descriptor projection", error))?;
    let projections = fact_descriptor_projections_from_rows(tx, rows).await?;
    projections.into_values().next().ok_or_else(|| {
        PostgresStoreError::Corruption(format!(
            "fact descriptor catalog row {descriptor_hash} missing after insert"
        ))
    })
}

async fn fact_descriptor_projections_from_rows(
    tx: &mut Transaction<'_, Postgres>,
    rows: Vec<PgRow>,
) -> Result<BTreeMap<ContentDigest, mfm_store::v1::FactDescriptorProjection>> {
    let mut projections = BTreeMap::new();
    for row in rows {
        let row = PgRowReader::new(&row, "fact_descriptor_index");
        let descriptor_hash = row.required_identity::<ContentDigest>("descriptor_hash")?;
        let descriptor_artifact_id = row.required_identity("descriptor_artifact_id")?;
        let descriptor_artifact_evidence_hash =
            row.required_identity("descriptor_artifact_evidence_hash")?;
        let descriptor_artifact = load_artifact_record_tx(
            tx,
            &descriptor_artifact_id,
            &descriptor_artifact_evidence_hash,
        )
        .await?;
        let projection = mfm_store::v1::FactDescriptorProjection {
            descriptor_hash: descriptor_hash.clone(),
            descriptor_artifact_id,
            descriptor_artifact_evidence: descriptor_artifact.evidence,
            fact_kind: mfm_facts::FactKind::new(row.required_string("fact_kind")?)
                .map_err(fact_error)?,
            descriptor_schema_id: row.required_identity("descriptor_schema_id")?,
            subject_schema_id: row.required_identity("subject_schema_id")?,
            response_schema_id: row.required_identity("response_schema_id")?,
            fact_subject_namespace_hash: row.required_identity("fact_subject_namespace_hash")?,
        };
        projections.insert(descriptor_hash, projection);
    }
    Ok(projections)
}

async fn load_run_fact_descriptor_admissions_tx(
    tx: &mut Transaction<'_, Postgres>,
    run_id: Option<&RunId>,
) -> Result<BTreeMap<(RunId, ContentDigest), FactDescriptorAdmissionProjection>> {
    let sql = if run_id.is_some() {
        "SELECT run_id, descriptor_hash, descriptor_artifact_id, \
         descriptor_artifact_evidence_hash, source_seq, source_ordinal, source_event_id \
         FROM run_fact_descriptor_admissions WHERE run_id = $1 \
         ORDER BY run_id, descriptor_hash"
    } else {
        "SELECT run_id, descriptor_hash, descriptor_artifact_id, \
         descriptor_artifact_evidence_hash, source_seq, source_ordinal, source_event_id \
         FROM run_fact_descriptor_admissions ORDER BY run_id, descriptor_hash"
    };
    let mut query = sqlx::query(sql);
    if let Some(run_id) = run_id {
        query = query.bind(run_id.as_str());
    }
    let rows = query
        .fetch_all(&mut **tx)
        .await
        .map_err(|error| database_error("failed to load run fact descriptor admissions", error))?;
    let mut admissions = BTreeMap::new();
    for row in rows {
        let row = PgRowReader::new(&row, "run_fact_descriptor_admissions");
        let run_id = row.required_identity::<RunId>("run_id")?;
        let descriptor_hash = row.required_identity::<ContentDigest>("descriptor_hash")?;
        let descriptor_artifact_id = row.required_identity("descriptor_artifact_id")?;
        let descriptor_artifact_evidence_hash =
            row.required_identity("descriptor_artifact_evidence_hash")?;
        let descriptor_artifact = load_artifact_record_tx(
            tx,
            &descriptor_artifact_id,
            &descriptor_artifact_evidence_hash,
        )
        .await?;
        let source_seq = i64_to_positive_u64(
            row.required_i64("source_seq")?,
            "run_fact_descriptor_admissions.source_seq",
        )?;
        let source_ordinal = row.required_u32("source_ordinal")?;
        let admission = FactDescriptorAdmissionProjection {
            run_id: run_id.clone(),
            descriptor_hash: descriptor_hash.clone(),
            descriptor_artifact_id,
            descriptor_artifact_evidence: descriptor_artifact.evidence,
            source_seq,
            source_ordinal,
            source_event_id: row.required_identity("source_event_id")?,
        };
        if admissions
            .insert((run_id, descriptor_hash), admission)
            .is_some()
        {
            return Err(PostgresStoreError::Corruption(
                "duplicate run fact descriptor admission projection".to_owned(),
            ));
        }
    }
    Ok(admissions)
}

async fn load_fact_record_projections_tx(
    tx: &mut Transaction<'_, Postgres>,
    run_id: Option<&RunId>,
) -> Result<BTreeMap<mfm_facts::FactClaimId, mfm_store::v1::FactRecordProjection>> {
    let sql = if run_id.is_some() {
        "SELECT e.run_id, e.seq, e.ordinal, c.store_commit_order, e.event_id, e.event_schema_id, e.spec_hash, \
         e.commit_key, e.logical_key, e.payload_hash, e.payload_canonical_json \
         FROM run_events e JOIN commits c ON c.run_id = e.run_id AND c.seq = e.seq \
         WHERE e.run_id = $1 \
         ORDER BY e.seq, e.ordinal"
    } else {
        "SELECT e.run_id, e.seq, e.ordinal, c.store_commit_order, e.event_id, e.event_schema_id, e.spec_hash, \
         e.commit_key, e.logical_key, e.payload_hash, e.payload_canonical_json \
         FROM run_events e JOIN commits c ON c.run_id = e.run_id AND c.seq = e.seq \
         ORDER BY e.run_id, e.seq, e.ordinal"
    };
    let mut query = sqlx::query(sql);
    if let Some(run_id) = run_id {
        query = query.bind(run_id.as_str());
    }
    let rows = query
        .fetch_all(&mut **tx)
        .await
        .map_err(|error| database_error("failed to load fact record projections", error))?;
    let mut projections = BTreeMap::new();
    for row in rows {
        let event = event_envelope_from_row(row)?;
        let events::KernelEventPayload::FactRecorded(payload) = event.payload() else {
            continue;
        };
        let response = payload.claim.response();
        let response_artifact = load_artifact_record_tx(
            tx,
            response.artifact_id(),
            response.artifact_evidence_hash(),
        )
        .await?;
        let projection = mfm_store::v1::FactRecordProjection::from_recorded_event(
            &event,
            payload,
            Some(response_artifact.evidence),
        )?;
        let claim_id = projection.fact_claim_id.clone();
        if projections.insert(claim_id.clone(), projection).is_some() {
            return Err(PostgresStoreError::Corruption(format!(
                "duplicate fact record projection {:?}",
                claim_id
            )));
        }
    }
    Ok(projections)
}

async fn load_fact_index_tx(
    tx: &mut Transaction<'_, Postgres>,
    run_id: Option<&RunId>,
) -> Result<BTreeMap<mfm_facts::FactClaimId, mfm_store::v1::FactIndexProjection>> {
    let mut builder = QueryBuilder::new("SELECT ");
    push_fact_index_projection_select_list(&mut builder, None);
    builder.push(" FROM fact_index");
    if let Some(run_id) = run_id {
        builder
            .push(" WHERE source_run_id = ")
            .push_bind(run_id.as_str().to_owned())
            .push(" ORDER BY source_seq, source_ordinal");
    } else {
        builder.push(" ORDER BY source_run_id, source_seq, source_ordinal");
    }
    let rows = builder
        .build()
        .fetch_all(&mut **tx)
        .await
        .map_err(|error| database_error("failed to load fact index projections", error))?;
    let mut projections = BTreeMap::new();
    for row in rows {
        let projection = fact_index_projection_from_row(&row)?;
        projections.insert(projection.fact_claim_id.clone(), projection);
    }
    Ok(projections)
}

const FACT_INDEX_PROJECTION_COLUMNS: &[&str] = &[
    "source_run_id",
    "source_seq",
    "source_ordinal",
    "source_event_id",
    "producer_node_id",
    "commit_key",
    "store_commit_order",
    "recorded_at",
    "observed_at",
    "audience",
    "visibility_scope",
    "fact_kind",
    "fact_descriptor_hash",
    "fact_subject_namespace_hash",
    "fact_key",
    "subject_material_hash",
    "request_schema_id",
    "request_hash",
    "response_schema_id",
    "response_hash",
    "response_artifact_id",
    "response_artifact_evidence_hash",
    "capability_kind",
    "capability_version",
    "adapter_kind",
    "adapter_version",
];

pub(super) fn push_fact_index_projection_select_list(
    builder: &mut QueryBuilder<Postgres>,
    alias: Option<&str>,
) {
    for (index, column) in FACT_INDEX_PROJECTION_COLUMNS.iter().enumerate() {
        if index > 0 {
            builder.push(", ");
        }
        if let Some(alias) = alias {
            builder
                .push(alias)
                .push(".")
                .push(*column)
                .push(" AS ")
                .push(*column);
        } else {
            builder.push(*column);
        }
    }
}

struct FactClaimCoordinates {
    source_run_id: RunId,
    source_seq: u64,
    source_ordinal: u32,
    claim_id: mfm_facts::FactClaimId,
}

fn fact_claim_coordinates_from_row(row: &PgRowReader<'_>) -> Result<FactClaimCoordinates> {
    let source_run_id = row.required_identity::<RunId>("source_run_id")?;
    let source_seq = row.required_positive_u64("source_seq")?;
    let source_ordinal = row.required_u32("source_ordinal")?;
    let claim_id = mfm_facts::FactClaimId::new(source_run_id.clone(), source_seq, source_ordinal)
        .map_err(fact_error)?;
    Ok(FactClaimCoordinates {
        source_run_id,
        source_seq,
        source_ordinal,
        claim_id,
    })
}

pub(super) fn fact_index_projection_from_row(
    row: &PgRow,
) -> Result<mfm_store::v1::FactIndexProjection> {
    let row = PgRowReader::new(row, "fact_index");
    let coordinates = fact_claim_coordinates_from_row(&row)?;
    Ok(mfm_store::v1::FactIndexProjection {
        fact_claim_id: coordinates.claim_id,
        source_run_id: coordinates.source_run_id,
        source_seq: coordinates.source_seq,
        source_ordinal: coordinates.source_ordinal,
        source_event_id: row.required_identity("source_event_id")?,
        producer_node_id: row.required_identity("producer_node_id")?,
        commit_id: CommitKey::new(row.required_string("commit_key")?)?,
        store_commit_order: row.required_positive_u64("store_commit_order")?,
        recorded_at: row.required_string("recorded_at")?,
        observed_at: row.optional_string("observed_at")?,
        audience: parse_fact_audience(&row.required_string("audience")?)?,
        visibility_scope: parse_fact_visibility_scope(&row.required_string("visibility_scope")?)?,
        fact_kind: mfm_facts::FactKind::new(row.required_string("fact_kind")?)
            .map_err(fact_error)?,
        fact_descriptor_hash: row.required_identity("fact_descriptor_hash")?,
        fact_subject_namespace_hash: row.required_identity("fact_subject_namespace_hash")?,
        fact_key: mfm_facts::FactKey::from_digest(row.required_identity("fact_key")?),
        subject_material_hash: row.required_identity("subject_material_hash")?,
        request_schema_id: row.optional_identity("request_schema_id")?,
        request_hash: row.optional_identity("request_hash")?,
        response_schema_id: row.required_identity("response_schema_id")?,
        response_hash: row.required_identity("response_hash")?,
        artifact_id: row.required_identity("response_artifact_id")?,
        artifact_evidence_hash: row.required_identity("response_artifact_evidence_hash")?,
        capability_kind: row.required_identity("capability_kind")?,
        capability_version: row.required_parsed("capability_version")?,
        adapter_kind: row.required_identity("adapter_kind")?,
        adapter_version: row.required_parsed("adapter_version")?,
    })
}

async fn load_fact_index_terms_tx(
    tx: &mut Transaction<'_, Postgres>,
    run_id: Option<&RunId>,
) -> Result<
    BTreeMap<
        (mfm_facts::FactClaimId, mfm_facts::FactFieldId),
        mfm_store::v1::FactIndexTermProjection,
    >,
> {
    let mut builder = QueryBuilder::new("SELECT ");
    push_fact_index_term_projection_select_list(&mut builder);
    builder.push(" FROM fact_index_terms");
    if let Some(run_id) = run_id {
        builder
            .push(" WHERE source_run_id = ")
            .push_bind(run_id.as_str().to_owned())
            .push(" ORDER BY source_seq, source_ordinal, field_id");
    } else {
        builder.push(" ORDER BY source_run_id, source_seq, source_ordinal, field_id");
    }
    let rows = builder
        .build()
        .fetch_all(&mut **tx)
        .await
        .map_err(|error| database_error("failed to load fact term projections", error))?;
    let mut projections = BTreeMap::new();
    for row in rows {
        let row = PgRowReader::new(&row, "fact_index_terms");
        let coordinates = fact_claim_coordinates_from_row(&row)?;
        let (field_id, value_type, value) = fact_term_value_from_reader(&row)?;
        let projection = mfm_store::v1::FactIndexTermProjection {
            fact_claim_id: coordinates.claim_id.clone(),
            fact_descriptor_hash: row.required_identity("fact_descriptor_hash")?,
            field_id: field_id.clone(),
            source: parse_fact_field_source(&row.required_string("source")?)?,
            value_type,
            value,
            unit: row
                .optional_string("unit")?
                .map(mfm_facts::FactUnit::new)
                .transpose()
                .map_err(fact_error)?,
            scale: row
                .optional_i32("scale")?
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
        projections.insert((coordinates.claim_id, field_id), projection);
    }
    Ok(projections)
}

const FACT_INDEX_TERM_IDENTITY_COLUMNS: &[&str] = &[
    "source_run_id",
    "source_seq",
    "source_ordinal",
    "fact_descriptor_hash",
    "field_id",
    "source",
    "value_type",
];

const FACT_INDEX_TERM_METADATA_COLUMNS: &[&str] = &["unit", "scale"];

#[cfg(test)]
#[path = "fact_projections_tests.rs"]
mod tests;
