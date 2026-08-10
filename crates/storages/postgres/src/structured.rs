use std::collections::BTreeMap;

use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_ids::{
    AppendRequestId, ContentDigest, ContentRef, JournalCommitDigest, JournalRecordHash, RunId,
    SchemaId, StableId, StoreEpoch, StoreScopeId, TenantScopeId,
};
use mfm_journal::structured::{
    canonical_json, AssignedRecord, CommittedBatch, HistoryObject, JournalHead, RecordRef,
    TenantFactCoordinate, TenantFactFrontier,
};
use mfm_store::structured::{
    AppendAttemptLookup, BackendAppendOutcome, RawRunHistory, RunCurrentProjection,
    StructuredBackendFuture, StructuredHistoryBackend, StructuredRunSnapshot, StructuredStoreError,
    StructuredStoreIdentity, StructuredStoreRunSnapshot, StructuredStoreSnapshot,
    TenantFactProjectionPlan, TenantFactProjectionSnapshot, TenantFactPublication,
    ValidatedRunAppend, MAX_BATCH_OBJECTS, MAX_STORED_FRAME_BYTES,
};
use serde::{Deserialize, Serialize};
use sqlx::postgres::PgRow;
use sqlx::{Postgres, QueryBuilder, Row, Transaction};

use crate::session::{PostgresApplicationSessions, RoleSession, TargetBinding};
use crate::sql_catalog::StructuredBatchQuery;
use crate::transaction::{
    begin_read, begin_run_write, lock_run_and_tenant, store_identity_from_binding, LockedWriteTx,
    ReadTx,
};

const OBJECT_INSERT_CHUNK_SIZE: usize = 8_192;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredBatchEnvelope {
    store_scope_id: StoreScopeId,
    store_epoch: StoreEpoch,
    predecessor: Option<JournalHead>,
    append_request_id: AppendRequestId,
    tenant_fact_coordinate: TenantFactCoordinate,
    candidate_digest: ContentDigest,
    records: Vec<AssignedRecord>,
    object_count: u32,
    head: JournalHead,
}

impl StoredBatchEnvelope {
    fn from_batch(batch: &CommittedBatch) -> Result<Self, StructuredStoreError> {
        if batch.objects.len() > MAX_BATCH_OBJECTS {
            return Err(StructuredStoreError::InvalidHistory);
        }
        let object_count =
            u32::try_from(batch.objects.len()).map_err(|_| StructuredStoreError::InvalidHistory)?;
        Ok(Self {
            store_scope_id: batch.store_scope_id.clone(),
            store_epoch: batch.store_epoch,
            predecessor: batch.predecessor.clone(),
            append_request_id: batch.append_request_id.clone(),
            tenant_fact_coordinate: batch.tenant_fact_coordinate.clone(),
            candidate_digest: batch.candidate_digest.clone(),
            records: batch.records.clone(),
            object_count,
            head: batch.head.clone(),
        })
    }

    fn into_batch(
        self,
        objects: Vec<HistoryObject>,
    ) -> Result<CommittedBatch, StructuredStoreError> {
        if usize::try_from(self.object_count).ok() != Some(objects.len())
            || objects.len() > MAX_BATCH_OBJECTS
        {
            return Err(StructuredStoreError::InvalidHistory);
        }
        Ok(CommittedBatch {
            store_scope_id: self.store_scope_id,
            store_epoch: self.store_epoch,
            predecessor: self.predecessor,
            append_request_id: self.append_request_id,
            tenant_fact_coordinate: self.tenant_fact_coordinate,
            candidate_digest: self.candidate_digest,
            records: self.records,
            objects,
            head: self.head,
        })
    }
}

struct StoredBatchRow {
    run_id: String,
    run_sequence: u64,
    append_request_id: String,
    candidate_digest: String,
    predecessor_sequence: Option<u64>,
    predecessor_commit_digest: Option<String>,
    head_commit_digest: String,
    batch_envelope_json: String,
}

impl StoredBatchRow {
    fn decode(row: &PgRow) -> Result<Self, StructuredStoreError> {
        let run_sequence = required_sequence(row, "run_sequence")?;
        let predecessor_sequence = optional_sequence(row, "predecessor_sequence")?;
        Ok(Self {
            run_id: required_text(row, "run_id")?,
            run_sequence,
            append_request_id: required_text(row, "append_request_id")?,
            candidate_digest: required_text(row, "candidate_digest")?,
            predecessor_sequence,
            predecessor_commit_digest: row
                .try_get::<Option<String>, _>("predecessor_commit_digest")
                .map_err(|_| invalid("structured PostgreSQL predecessor digest is absent"))?,
            head_commit_digest: required_text(row, "head_commit_digest")?,
            batch_envelope_json: required_text(row, "batch_envelope_json")?,
        })
    }

    fn reconstruct(
        self,
        object_rows: Vec<StoredObjectRow>,
    ) -> Result<CommittedBatch, StructuredStoreError> {
        let envelope = decode_canonical_envelope(&self.batch_envelope_json)?;
        let objects = decode_objects(object_rows)?;
        let batch = envelope.into_batch(objects)?;
        let batch_run_id = batch
            .records
            .first()
            .ok_or_else(|| invalid("structured PostgreSQL batch has no record"))?
            .record_ref
            .run_id
            .as_str();
        if batch_run_id != self.run_id
            || batch
                .records
                .iter()
                .any(|record| record.record_ref.run_id.as_str() != self.run_id)
            || batch.head.run_sequence != self.run_sequence
            || batch.append_request_id.as_str() != self.append_request_id
            || batch.candidate_digest.as_str() != self.candidate_digest
            || batch.predecessor.as_ref().map(|head| head.run_sequence) != self.predecessor_sequence
            || batch
                .predecessor
                .as_ref()
                .map(|head| head.commit_digest.as_str())
                != self.predecessor_commit_digest.as_deref()
            || batch.head.commit_digest.as_str() != self.head_commit_digest
        {
            return Err(invalid(
                "structured PostgreSQL batch columns differ from its envelope",
            ));
        }
        Ok(batch)
    }
}

struct StoredObjectRow {
    ordinal: i32,
    object_type: String,
    content_schema_id: String,
    content_digest: String,
    canonical_json: String,
}

fn decode_tenant_publication(
    row: &PgRow,
    store_scope_id: &StoreScopeId,
    store_epoch: StoreEpoch,
    tenant_scope_id: &TenantScopeId,
) -> Result<TenantFactPublication, StructuredStoreError> {
    if required_text(row, "store_scope_id")? != store_scope_id.as_str()
        || required_text(row, "store_epoch")? != store_epoch.get().to_string()
        || required_text(row, "tenant_scope_id")? != tenant_scope_id.as_str()
    {
        return Err(invalid(
            "structured PostgreSQL tenant publication changed identity",
        ));
    }
    let fact_order = parse_fact_order(&required_text(row, "fact_order")?, false)?;
    let run_id = RunId::parse(required_text(row, "run_id")?)
        .map_err(|_| invalid("structured PostgreSQL publication run id is invalid"))?;
    let run_sequence = required_sequence(row, "run_sequence")?;
    let ordinal = row
        .try_get::<i32, _>("transition_ordinal")
        .ok()
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(|| invalid("structured PostgreSQL publication ordinal is invalid"))?;
    let record_hash = JournalRecordHash::parse(required_text(row, "transition_record_hash")?)
        .map_err(|_| invalid("structured PostgreSQL publication record hash is invalid"))?;
    let producer_head = JournalHead {
        run_sequence,
        commit_digest: JournalCommitDigest::parse(&required_text(row, "head_commit_digest")?)
            .map_err(|_| invalid("structured PostgreSQL publication head is invalid"))?,
    };
    Ok(TenantFactPublication {
        frontier: TenantFactFrontier::new(
            store_scope_id.clone(),
            store_epoch,
            tenant_scope_id.clone(),
            fact_order,
        ),
        transition_ref: RecordRef {
            run_id,
            run_sequence,
            ordinal,
            record_hash,
        },
        producer_head,
    })
}

/// Real PostgreSQL implementation of the shared structured-history backend seam.
pub struct PostgresStructuredHistoryBackend {
    run_reader: RoleSession,
    run_writer: RoleSession,
    target: TargetBinding,
    identity: StructuredStoreIdentity,
}

impl mfm_authority_seal::ValidatedAppendConsumerSeal for PostgresStructuredHistoryBackend {}

impl std::fmt::Debug for PostgresStructuredHistoryBackend {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PostgresStructuredHistoryBackend")
            .field("identity", &self.identity)
            .finish_non_exhaustive()
    }
}

impl PostgresStructuredHistoryBackend {
    pub(crate) fn from_application_sessions(sessions: PostgresApplicationSessions) -> Self {
        let (run_reader, run_writer, target) = sessions.into_run_parts();
        let identity = store_identity_from_binding(&target);
        Self {
            run_reader,
            run_writer,
            target,
            identity,
        }
    }

    pub(crate) fn from_run_parts(
        run_reader: RoleSession,
        run_writer: RoleSession,
        target: TargetBinding,
    ) -> Self {
        let identity = store_identity_from_binding(&target);
        Self {
            run_reader,
            run_writer,
            target,
            identity,
        }
    }

    async fn begin_read(&self) -> Result<ReadTx<'_>, StructuredStoreError> {
        begin_read(&self.run_reader, &self.target).await
    }

    async fn begin_locked_write(
        &self,
        run_id: &str,
        tenant_key: Option<&str>,
    ) -> Result<LockedWriteTx<'_>, StructuredStoreError> {
        let write = begin_run_write(&self.run_writer, &self.target).await?;
        lock_run_and_tenant(write, run_id, tenant_key).await
    }
}

impl StructuredHistoryBackend for PostgresStructuredHistoryBackend {
    fn identity(&self) -> &StructuredStoreIdentity {
        &self.identity
    }

    fn load_snapshot<'a>(
        &'a self,
        run_id: &'a RunId,
    ) -> StructuredBackendFuture<'a, StructuredRunSnapshot> {
        Box::pin(async move {
            let mut transaction = self.begin_read().await?;
            let snapshot = load_run_snapshot(
                transaction.conn(),
                run_id,
                &self.identity,
                self.target.schema_name(),
            )
            .await?;
            transaction.commit_checked(&self.target).await?;
            Ok(snapshot)
        })
    }

    fn current_run_projection<'a>(
        &'a self,
        run_id: &'a RunId,
    ) -> StructuredBackendFuture<'a, Option<RunCurrentProjection>> {
        Box::pin(async move {
            let mut transaction = self.begin_read().await?;
            let row = sqlx::query(
                "SELECT head_sequence::text AS head_sequence, head_commit_digest, \
                        tenant_scope_id, has_effect_entry_attention, store_scope_id, \
                        store_epoch::text AS store_epoch \
                   FROM run_history_heads WHERE run_id = $1",
            )
            .bind(run_id.as_str())
            .fetch_optional(&mut **transaction.conn())
            .await
            .map_err(|_| StructuredStoreError::BackendUnavailable)?;
            let projection = row
                .map(|row| decode_run_projection(&row, run_id, &self.identity))
                .transpose()?;
            transaction.validate_target(&self.target).await?;
            transaction.commit_checked(&self.target).await?;
            Ok(projection)
        })
    }

    fn load_prefix<'a>(
        &'a self,
        run_id: &'a RunId,
        through: &'a JournalHead,
    ) -> StructuredBackendFuture<'a, Option<RawRunHistory>> {
        Box::pin(async move {
            if through.run_sequence == 0 {
                return Err(invalid(
                    "structured PostgreSQL prefix sequence must be positive",
                ));
            }
            let mut transaction = self.begin_read().await?;
            let history = load_exact_prefix(transaction.conn(), run_id, through).await?;
            transaction.commit_checked(&self.target).await?;
            Ok(history)
        })
    }

    fn lookup_append_attempt<'a>(
        &'a self,
        run_id: &'a RunId,
        append_request_id: &'a AppendRequestId,
    ) -> StructuredBackendFuture<'a, Option<AppendAttemptLookup>> {
        Box::pin(async move {
            let mut transaction = self.begin_read().await?;
            let rows = select_batch_rows(
                transaction.conn(),
                StructuredBatchQuery::ByAppendRequest,
                run_id,
                Some(append_request_id.as_str()),
            )
            .await?;
            let history = match exactly_one_or_none(rows)? {
                Some(row) => {
                    let through = JournalHead {
                        run_sequence: row.run_sequence,
                        commit_digest: JournalCommitDigest::parse(&row.head_commit_digest)
                            .map_err(|_| invalid("PostgreSQL append head is invalid"))?,
                    };
                    load_exact_prefix(transaction.conn(), run_id, &through)
                        .await?
                        .map(|history| AppendAttemptLookup { history })
                }
                None => None,
            };
            transaction.commit_checked(&self.target).await?;
            Ok(history)
        })
    }

    fn scan_run_ids<'a>(
        &'a self,
        after_run_id: Option<&'a RunId>,
        maximum_items: u32,
    ) -> StructuredBackendFuture<'a, Vec<RunId>> {
        Box::pin(async move {
            if maximum_items == 0 {
                return Err(invalid("PostgreSQL run page is empty"));
            }
            let mut transaction = self.begin_read().await?;
            let rows = select_run_ids(transaction.conn(), after_run_id, maximum_items).await?;
            transaction.commit_checked(&self.target).await?;
            Ok(rows)
        })
    }

    fn load_store_snapshot(&self) -> StructuredBackendFuture<'_, StructuredStoreSnapshot> {
        Box::pin(async move {
            let mut transaction = self.begin_read().await?;
            let mut runs = Vec::new();
            let mut after = None;
            loop {
                let page = select_run_ids(transaction.conn(), after.as_ref(), 256).await?;
                for run_id in &page {
                    let snapshot = load_run_snapshot(
                        transaction.conn(),
                        run_id,
                        &self.identity,
                        self.target.schema_name(),
                    )
                    .await?;
                    runs.push(StructuredStoreRunSnapshot {
                        run_id: run_id.clone(),
                        history: snapshot.history,
                        current_projection: snapshot.current_projection,
                    });
                }
                after = page.last().cloned();
                if page.len() < 256 {
                    break;
                }
            }
            let mut tenant_facts = Vec::new();
            let mut after_tenant = None;
            loop {
                let page =
                    select_fact_tenants(transaction.conn(), after_tenant.as_ref(), 256).await?;
                for tenant_scope_id in &page {
                    tenant_facts.push(
                        load_fact_projection(
                            transaction.conn(),
                            &self.identity,
                            tenant_scope_id.clone(),
                        )
                        .await?,
                    );
                }
                after_tenant = page.last().cloned();
                if page.len() < 256 {
                    break;
                }
            }
            transaction.commit_checked(&self.target).await?;
            Ok(StructuredStoreSnapshot { runs, tenant_facts })
        })
    }

    fn validate_authority(&self) -> StructuredBackendFuture<'_, ()> {
        Box::pin(async move {
            let mut transaction = self.begin_read().await?;
            transaction.validate_target(&self.target).await?;
            transaction.commit_checked(&self.target).await
        })
    }

    fn tenant_fact_frontier<'a>(
        &'a self,
        tenant_scope_id: &'a TenantScopeId,
    ) -> StructuredBackendFuture<'a, TenantFactFrontier> {
        Box::pin(async move {
            let mut transaction = self.begin_read().await?;
            let head = sqlx::query_scalar::<_, String>(
                "SELECT fact_order::text FROM tenant_fact_heads \
                  WHERE store_scope_id = $1 AND store_epoch = $2::numeric \
                    AND tenant_scope_id = $3",
            )
            .bind(self.identity.store_scope_id.as_str())
            .bind(self.identity.store_epoch.get().to_string())
            .bind(tenant_scope_id.as_str())
            .fetch_optional(&mut **transaction.conn())
            .await
            .map_err(|_| StructuredStoreError::BackendUnavailable)?;
            let latest_route = sqlx::query_scalar::<_, String>(
                "SELECT fact_order::text FROM tenant_fact_publications \
                  WHERE store_scope_id = $1 AND store_epoch = $2::numeric \
                    AND tenant_scope_id = $3 \
                  ORDER BY fact_order DESC LIMIT 1",
            )
            .bind(self.identity.store_scope_id.as_str())
            .bind(self.identity.store_epoch.get().to_string())
            .bind(tenant_scope_id.as_str())
            .fetch_optional(&mut **transaction.conn())
            .await
            .map_err(|_| StructuredStoreError::BackendUnavailable)?;
            let fact_order = match (head, latest_route) {
                (None, None) => 0,
                (Some(head), Some(latest)) if head == latest => parse_fact_order(&head, false)?,
                _ => {
                    return Err(invalid(
                        "structured PostgreSQL tenant fact head differs from its latest route",
                    ));
                }
            };
            transaction.validate_target(&self.target).await?;
            transaction.commit_checked(&self.target).await?;
            Ok(TenantFactFrontier::new(
                self.identity.store_scope_id.clone(),
                self.identity.store_epoch,
                tenant_scope_id.clone(),
                fact_order,
            ))
        })
    }

    fn scan_fact_publications<'a>(
        &'a self,
        tenant_scope_id: &'a TenantScopeId,
        first_order: u64,
        through_order: u64,
        maximum_items: u32,
    ) -> StructuredBackendFuture<'a, Vec<TenantFactPublication>> {
        Box::pin(async move {
            if first_order == 0 || maximum_items == 0 || maximum_items > 1_024 {
                return Err(invalid(
                    "structured PostgreSQL tenant fact page is outside the fixed bounds",
                ));
            }
            if first_order > through_order {
                return Ok(Vec::new());
            }
            let mut transaction = self.begin_read().await?;
            let rows = select_fact_publication_rows(
                transaction.conn(),
                &self.identity,
                tenant_scope_id,
                first_order,
                through_order,
                maximum_items,
            )
            .await?;
            let mut publications = Vec::with_capacity(rows.len());
            for row in rows {
                publications.push(decode_tenant_publication(
                    &row,
                    &self.identity.store_scope_id,
                    self.identity.store_epoch,
                    tenant_scope_id,
                )?);
            }
            transaction.commit_checked(&self.target).await?;
            Ok(publications)
        })
    }

    fn append<'a>(
        &'a self,
        command: ValidatedRunAppend,
    ) -> StructuredBackendFuture<'a, BackendAppendOutcome> {
        Box::pin(async move {
            let (committed, run_plan, tenant_plan) = command.into_parts(self);
            if committed.store_scope_id != self.identity.store_scope_id
                || committed.store_epoch != self.identity.store_epoch
            {
                return Ok(BackendAppendOutcome::StaleHead);
            }
            let run_id = run_plan.successor().run_id.clone();
            if committed.records.is_empty()
                || committed
                    .records
                    .iter()
                    .any(|record| record.record_ref.run_id != run_id)
                || run_plan.successor().journal_head != committed.head
                || run_plan
                    .expected()
                    .map(|projection| &projection.journal_head)
                    != committed.predecessor.as_ref()
            {
                return Err(invalid("validated PostgreSQL run plan is incoherent"));
            }
            let envelope = StoredBatchEnvelope::from_batch(&committed)?;
            let canonical_envelope = canonical_json(&envelope)
                .map_err(|_| invalid("validated PostgreSQL batch envelope is not canonical"))?;
            validate_stored_frame(canonical_envelope.as_str())?;

            let tenant_lock_key = match &tenant_plan {
                TenantFactProjectionPlan::None => None,
                TenantFactProjectionPlan::Barrier { expected_frontier } => {
                    Some(expected_frontier.tenant_scope_id.as_str())
                }
                TenantFactProjectionPlan::Publish {
                    expected_predecessor,
                    publication,
                } => {
                    if publication.frontier.store_scope_id != self.identity.store_scope_id
                        || publication.frontier.store_epoch != self.identity.store_epoch
                        || publication.frontier.tenant_scope_id
                            != expected_predecessor.tenant_scope_id
                        || publication.frontier.fact_order
                            != expected_predecessor
                                .fact_order
                                .checked_add(1)
                                .ok_or_else(|| invalid("tenant fact order overflowed"))?
                    {
                        return Err(invalid("validated PostgreSQL fact plan is incoherent"));
                    }
                    Some(expected_predecessor.tenant_scope_id.as_str())
                }
            };
            let mut transaction = self
                .begin_locked_write(run_id.as_str(), tenant_lock_key)
                .await?;

            let existing_rows = select_batch_rows(
                transaction.conn(),
                StructuredBatchQuery::ByAppendRequest,
                &run_id,
                Some(committed.append_request_id.as_str()),
            )
            .await?;
            if let Some(existing_row) = exactly_one_or_none(existing_rows)? {
                let existing_sequence = existing_row.run_sequence;
                let mut object_rows =
                    load_object_rows(transaction.conn(), &run_id, Some(existing_sequence)).await?;
                let existing = existing_row
                    .reconstruct(object_rows.remove(&existing_sequence).unwrap_or_default())?;
                if !object_rows.is_empty() {
                    return Err(invalid(
                        "structured PostgreSQL idempotent objects differ from their batch",
                    ));
                }
                transaction.rollback().await?;
                return if existing == committed {
                    Ok(BackendAppendOutcome::ExistingSame(existing))
                } else {
                    Err(StructuredStoreError::AppendConflict)
                };
            }

            let current_projection =
                load_locked_run_projection(transaction.conn(), &run_id, &self.identity).await?;
            if current_projection.as_ref() != run_plan.expected() {
                transaction.rollback().await?;
                return Ok(BackendAppendOutcome::StaleHead);
            }
            match &tenant_plan {
                TenantFactProjectionPlan::None => {}
                TenantFactProjectionPlan::Barrier { expected_frontier }
                | TenantFactProjectionPlan::Publish {
                    expected_predecessor: expected_frontier,
                    ..
                } => {
                    if expected_frontier.store_scope_id != self.identity.store_scope_id
                        || expected_frontier.store_epoch != self.identity.store_epoch
                        || load_locked_fact_frontier(
                            transaction.conn(),
                            &self.identity,
                            &expected_frontier.tenant_scope_id,
                        )
                        .await?
                            != *expected_frontier
                    {
                        transaction.rollback().await?;
                        return Ok(BackendAppendOutcome::StaleHead);
                    }
                }
            }

            let predecessor_sequence = committed
                .predecessor
                .as_ref()
                .map(|head| head.run_sequence.to_string());
            let predecessor_digest = committed
                .predecessor
                .as_ref()
                .map(|head| head.commit_digest.as_str());
            transaction.validate_target(&self.target).await?;
            let batch_insert = sqlx::query(
                "INSERT INTO run_history_batches ( \
                    run_id, run_sequence, append_request_id, candidate_digest, \
                    predecessor_sequence, predecessor_commit_digest, head_commit_digest, \
                    batch_envelope_json \
                 ) VALUES ($1, $2::numeric, $3, $4, $5::numeric, $6, $7, $8)",
            )
            .bind(run_id.as_str())
            .bind(committed.head.run_sequence.to_string())
            .bind(committed.append_request_id.as_str())
            .bind(committed.candidate_digest.as_str())
            .bind(predecessor_sequence)
            .bind(predecessor_digest)
            .bind(committed.head.commit_digest.as_str())
            .bind(canonical_envelope.as_str())
            .execute(&mut **transaction.conn())
            .await;
            if let Err(error) = batch_insert {
                if is_contention_sqlstate(&error) {
                    transaction.rollback().await?;
                    return self
                        .classify_existing_after_contention(&run_id, &committed)
                        .await;
                }
                return Err(StructuredStoreError::BackendUnavailable);
            }
            insert_object_rows(
                transaction.conn(),
                &run_id,
                committed.head.run_sequence,
                &committed.objects,
            )
            .await?;

            let successor = run_plan.successor();
            let affected = if let Some(expected) = run_plan.expected() {
                sqlx::query(
                    "UPDATE run_history_heads \
                        SET head_sequence = $2::numeric, head_commit_digest = $3, \
                            tenant_scope_id = $8, has_effect_entry_attention = $9 \
                      WHERE run_id = $1 \
                        AND head_sequence = $4::numeric AND head_commit_digest = $5 \
                        AND store_scope_id = $6 AND store_epoch = $7::numeric \
                        AND tenant_scope_id = $10 AND has_effect_entry_attention = $11",
                )
                .bind(run_id.as_str())
                .bind(successor.journal_head.run_sequence.to_string())
                .bind(successor.journal_head.commit_digest.as_str())
                .bind(expected.journal_head.run_sequence.to_string())
                .bind(expected.journal_head.commit_digest.as_str())
                .bind(self.identity.store_scope_id.as_str())
                .bind(self.identity.store_epoch.get().to_string())
                .bind(successor.tenant_scope_id.as_str())
                .bind(successor.has_effect_entry_attention)
                .bind(expected.tenant_scope_id.as_str())
                .bind(expected.has_effect_entry_attention)
                .execute(&mut **transaction.conn())
                .await
                .map_err(|_| StructuredStoreError::BackendUnavailable)?
                .rows_affected()
            } else {
                match sqlx::query(
                    "INSERT INTO run_history_heads ( \
                        run_id, store_scope_id, store_epoch, tenant_scope_id, \
                        head_sequence, head_commit_digest, has_effect_entry_attention \
                     ) VALUES ($1, $2, $3::numeric, $4, $5::numeric, $6, $7)",
                )
                .bind(run_id.as_str())
                .bind(self.identity.store_scope_id.as_str())
                .bind(self.identity.store_epoch.get().to_string())
                .bind(successor.tenant_scope_id.as_str())
                .bind(successor.journal_head.run_sequence.to_string())
                .bind(successor.journal_head.commit_digest.as_str())
                .bind(successor.has_effect_entry_attention)
                .execute(&mut **transaction.conn())
                .await
                {
                    Ok(result) => result.rows_affected(),
                    Err(error) if is_contention_sqlstate(&error) => {
                        transaction.rollback().await?;
                        return Ok(BackendAppendOutcome::StaleHead);
                    }
                    Err(_) => return Err(StructuredStoreError::BackendUnavailable),
                }
            };
            if affected != 1 {
                transaction.rollback().await?;
                return Ok(BackendAppendOutcome::StaleHead);
            }

            if let TenantFactProjectionPlan::Publish {
                expected_predecessor,
                publication,
            } = tenant_plan
            {
                let inserted = match sqlx::query(
                    "INSERT INTO tenant_fact_publications ( \
                        store_scope_id, store_epoch, tenant_scope_id, fact_order, \
                        run_id, run_sequence, transition_ordinal, transition_record_hash \
                     ) VALUES ($1, $2::numeric, $3, $4::numeric, $5, $6::numeric, $7, $8)",
                )
                .bind(self.identity.store_scope_id.as_str())
                .bind(self.identity.store_epoch.get().to_string())
                .bind(publication.frontier.tenant_scope_id.as_str())
                .bind(publication.frontier.fact_order.to_string())
                .bind(publication.transition_ref.run_id.as_str())
                .bind(publication.transition_ref.run_sequence.to_string())
                .bind(
                    i32::try_from(publication.transition_ref.ordinal)
                        .map_err(|_| invalid("PostgreSQL publication ordinal exceeds i32"))?,
                )
                .bind(publication.transition_ref.record_hash.as_str())
                .execute(&mut **transaction.conn())
                .await
                {
                    Ok(result) => result.rows_affected(),
                    Err(error) if is_contention_sqlstate(&error) => {
                        transaction.rollback().await?;
                        return Ok(BackendAppendOutcome::StaleHead);
                    }
                    Err(_) => return Err(StructuredStoreError::BackendUnavailable),
                };
                let advanced = if expected_predecessor.fact_order == 0 {
                    match sqlx::query(
                        "INSERT INTO tenant_fact_heads ( \
                            store_scope_id, store_epoch, tenant_scope_id, fact_order \
                         ) VALUES ($1, $2::numeric, $3, $4::numeric)",
                    )
                    .bind(self.identity.store_scope_id.as_str())
                    .bind(self.identity.store_epoch.get().to_string())
                    .bind(publication.frontier.tenant_scope_id.as_str())
                    .bind(publication.frontier.fact_order.to_string())
                    .execute(&mut **transaction.conn())
                    .await
                    {
                        Ok(result) => result.rows_affected(),
                        Err(error) if is_contention_sqlstate(&error) => {
                            transaction.rollback().await?;
                            return Ok(BackendAppendOutcome::StaleHead);
                        }
                        Err(_) => return Err(StructuredStoreError::BackendUnavailable),
                    }
                } else {
                    sqlx::query(
                        "UPDATE tenant_fact_heads SET fact_order = $4::numeric \
                          WHERE store_scope_id = $1 AND store_epoch = $2::numeric \
                            AND tenant_scope_id = $3 AND fact_order = $5::numeric",
                    )
                    .bind(self.identity.store_scope_id.as_str())
                    .bind(self.identity.store_epoch.get().to_string())
                    .bind(publication.frontier.tenant_scope_id.as_str())
                    .bind(publication.frontier.fact_order.to_string())
                    .bind(expected_predecessor.fact_order.to_string())
                    .execute(&mut **transaction.conn())
                    .await
                    .map_err(|_| StructuredStoreError::BackendUnavailable)?
                    .rows_affected()
                };
                if inserted != 1 || advanced != 1 {
                    transaction.rollback().await?;
                    return Ok(BackendAppendOutcome::StaleHead);
                }
            }

            match transaction.commit_outcome(&self.target).await? {
                crate::transaction::CommitOutcome::Committed => {
                    Ok(BackendAppendOutcome::NewlyCommitted(committed))
                }
                crate::transaction::CommitOutcome::AcknowledgementUnknown => {
                    Ok(BackendAppendOutcome::AcknowledgementUnknown)
                }
            }
        })
    }
    fn scan_effect_entry_attention_routes<'a>(
        &'a self,
        tenant_scope_id: &'a mfm_ids::TenantScopeId,
        after_run_id: Option<&'a RunId>,
        maximum_items: u32,
    ) -> StructuredBackendFuture<'a, Vec<mfm_store::structured::EffectEntryAttentionRoute>> {
        Box::pin(async move {
            // Exactly the partial index: tenant equality, keyset continuation by
            // run identity, and the attention predicate. No record family,
            // frontier, or outcome is consulted.
            let mut transaction = self.begin_read().await?;
            let rows = sqlx::query(
                "SELECT run_id, head_sequence::text AS head_sequence, head_commit_digest \
                   FROM run_history_heads \
                  WHERE has_effect_entry_attention \
                    AND tenant_scope_id = $1 \
                    AND ($2::text IS NULL OR run_id > $2::text) \
                  ORDER BY run_id \
                  LIMIT $3::bigint",
            )
            .bind(tenant_scope_id.as_str())
            .bind(after_run_id.map(mfm_ids::RunId::as_str))
            .bind(i64::from(maximum_items))
            .fetch_all(&mut **transaction.conn())
            .await
            .map_err(|_| StructuredStoreError::BackendUnavailable)?;
            transaction.commit_checked(&self.target).await?;
            rows.into_iter()
                .map(|row| {
                    Ok(mfm_store::structured::EffectEntryAttentionRoute {
                        run_id: RunId::parse(&required_text(&row, "run_id")?).map_err(|_| {
                            invalid("structured PostgreSQL attention route run id is invalid")
                        })?,
                        journal_head: JournalHead {
                            run_sequence: required_sequence(&row, "head_sequence")?,
                            commit_digest: JournalCommitDigest::parse(&required_text(
                                &row,
                                "head_commit_digest",
                            )?)
                            .map_err(|_| {
                                invalid("structured PostgreSQL attention route head is invalid")
                            })?,
                        },
                    })
                })
                .collect()
        })
    }
}

fn decode_run_projection(
    row: &PgRow,
    run_id: &RunId,
    identity: &StructuredStoreIdentity,
) -> Result<RunCurrentProjection, StructuredStoreError> {
    if required_text(row, "store_scope_id")? != identity.store_scope_id.as_str()
        || required_text(row, "store_epoch")? != identity.store_epoch.get().to_string()
    {
        return Err(invalid("PostgreSQL run projection changed store identity"));
    }
    Ok(RunCurrentProjection {
        run_id: run_id.clone(),
        tenant_scope_id: TenantScopeId::new(required_text(row, "tenant_scope_id")?)
            .map_err(|_| invalid("PostgreSQL run projection tenant is invalid"))?,
        journal_head: JournalHead {
            run_sequence: required_sequence(row, "head_sequence")?,
            commit_digest: JournalCommitDigest::parse(&required_text(row, "head_commit_digest")?)
                .map_err(|_| invalid("PostgreSQL run projection digest is invalid"))?,
        },
        has_effect_entry_attention: row
            .try_get("has_effect_entry_attention")
            .map_err(|_| invalid("PostgreSQL run projection attention is invalid"))?,
    })
}

async fn load_run_snapshot(
    transaction: &mut Transaction<'_, Postgres>,
    run_id: &RunId,
    identity: &StructuredStoreIdentity,
    schema_name: &str,
) -> Result<StructuredRunSnapshot, StructuredStoreError> {
    let projection_row = sqlx::query(
        "SELECT store_scope_id, store_epoch::text AS store_epoch, tenant_scope_id, \
                head_sequence::text AS head_sequence, head_commit_digest, \
                has_effect_entry_attention \
           FROM run_history_heads WHERE run_id = $1",
    )
    .bind(run_id.as_str())
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|_| StructuredStoreError::BackendUnavailable)?;
    let current_projection = projection_row
        .map(|row| decode_run_projection(&row, run_id, identity))
        .transpose()?;
    #[cfg(feature = "test-support")]
    crate::transaction::await_read_phase_barrier(schema_name, "after_head").await;
    #[cfg(not(feature = "test-support"))]
    let _ = schema_name;
    let rows = select_batch_rows(transaction, StructuredBatchQuery::Prefix, run_id, None).await?;
    #[cfg(feature = "test-support")]
    crate::transaction::await_read_phase_barrier(schema_name, "after_batches").await;
    let history = if rows.is_empty() {
        None
    } else {
        let mut objects = load_object_rows(transaction, run_id, None).await?;
        let mut batches = Vec::with_capacity(rows.len());
        for row in rows {
            let sequence = row.run_sequence;
            batches.push(row.reconstruct(objects.remove(&sequence).unwrap_or_default())?);
        }
        if !objects.is_empty() {
            return Err(invalid("PostgreSQL objects have no retained batch"));
        }
        Some(RawRunHistory {
            run_id: run_id.clone(),
            batches,
        })
    };
    Ok(StructuredRunSnapshot {
        history,
        current_projection,
    })
}

async fn load_exact_prefix(
    transaction: &mut Transaction<'_, Postgres>,
    run_id: &RunId,
    through: &JournalHead,
) -> Result<Option<RawRunHistory>, StructuredStoreError> {
    let rows = sqlx::query(
        "SELECT run_id, run_sequence::text AS run_sequence, append_request_id, \
                candidate_digest, predecessor_sequence::text AS predecessor_sequence, \
                predecessor_commit_digest, head_commit_digest, batch_envelope_json \
           FROM run_history_batches \
          WHERE run_id = $1 AND run_sequence <= $2::numeric \
          ORDER BY run_history_batches.run_sequence",
    )
    .bind(run_id.as_str())
    .bind(through.run_sequence.to_string())
    .fetch_all(&mut **transaction)
    .await
    .map_err(|_| StructuredStoreError::BackendUnavailable)?
    .iter()
    .map(StoredBatchRow::decode)
    .collect::<Result<Vec<_>, _>>()?;
    if rows.is_empty() {
        return Ok(None);
    }
    let mut objects = load_object_rows_through(transaction, run_id, through.run_sequence).await?;
    let mut batches = Vec::with_capacity(rows.len());
    for row in rows {
        let sequence = row.run_sequence;
        batches.push(row.reconstruct(objects.remove(&sequence).unwrap_or_default())?);
    }
    if !objects.is_empty() || batches.last().map(|batch| &batch.head) != Some(through) {
        return Err(invalid(
            "PostgreSQL prefix differs from its exact requested head",
        ));
    }
    Ok(Some(RawRunHistory {
        run_id: run_id.clone(),
        batches,
    }))
}

async fn select_run_ids(
    transaction: &mut Transaction<'_, Postgres>,
    after: Option<&RunId>,
    maximum_items: u32,
) -> Result<Vec<RunId>, StructuredStoreError> {
    let rows = sqlx::query(
        "SELECT run_id FROM ( \
             SELECT run_id FROM run_history_batches \
             UNION SELECT run_id FROM run_history_heads \
         ) AS run_keys \
         WHERE ($1::text IS NULL OR run_id > $1::text) \
         ORDER BY run_id LIMIT $2::bigint",
    )
    .bind(after.map(RunId::as_str))
    .bind(i64::from(maximum_items))
    .fetch_all(&mut **transaction)
    .await
    .map_err(|_| StructuredStoreError::BackendUnavailable)?;
    rows.into_iter()
        .map(|row| {
            RunId::parse(required_text(&row, "run_id")?)
                .map_err(|_| invalid("PostgreSQL run key is invalid"))
        })
        .collect()
}

async fn select_fact_tenants(
    transaction: &mut Transaction<'_, Postgres>,
    after: Option<&TenantScopeId>,
    maximum_items: u32,
) -> Result<Vec<TenantScopeId>, StructuredStoreError> {
    let rows = sqlx::query(
        "SELECT tenant_scope_id FROM ( \
             SELECT tenant_scope_id FROM tenant_fact_publications \
             UNION SELECT tenant_scope_id FROM tenant_fact_heads \
         ) AS tenant_keys \
         WHERE ($1::text IS NULL OR tenant_scope_id > $1::text) \
         ORDER BY tenant_scope_id LIMIT $2::bigint",
    )
    .bind(after.map(TenantScopeId::as_str))
    .bind(i64::from(maximum_items))
    .fetch_all(&mut **transaction)
    .await
    .map_err(|_| StructuredStoreError::BackendUnavailable)?;
    rows.into_iter()
        .map(|row| {
            TenantScopeId::new(required_text(&row, "tenant_scope_id")?)
                .map_err(|_| invalid("PostgreSQL fact tenant key is invalid"))
        })
        .collect()
}

async fn load_fact_projection(
    transaction: &mut Transaction<'_, Postgres>,
    identity: &StructuredStoreIdentity,
    tenant_scope_id: TenantScopeId,
) -> Result<TenantFactProjectionSnapshot, StructuredStoreError> {
    let head_rows = sqlx::query(
        "SELECT store_scope_id, store_epoch::text AS store_epoch, tenant_scope_id, \
                fact_order::text AS fact_order \
           FROM tenant_fact_heads WHERE tenant_scope_id = $1",
    )
    .bind(tenant_scope_id.as_str())
    .fetch_all(&mut **transaction)
    .await
    .map_err(|_| StructuredStoreError::BackendUnavailable)?;
    let current_frontier = match head_rows.as_slice() {
        [] => None,
        [row]
            if required_text(row, "store_scope_id")? == identity.store_scope_id.as_str()
                && required_text(row, "store_epoch")? == identity.store_epoch.get().to_string()
                && required_text(row, "tenant_scope_id")? == tenant_scope_id.as_str() =>
        {
            Some(TenantFactFrontier::new(
                identity.store_scope_id.clone(),
                identity.store_epoch,
                tenant_scope_id.clone(),
                parse_fact_order(&required_text(row, "fact_order")?, false)?,
            ))
        }
        _ => {
            return Err(invalid(
                "PostgreSQL fact head is not unique or changed identity",
            ))
        }
    };
    let rows = select_fact_publication_rows(
        transaction,
        identity,
        &tenant_scope_id,
        1,
        u64::MAX,
        u32::MAX,
    )
    .await?;
    let publications = rows
        .into_iter()
        .map(|row| {
            decode_tenant_publication(
                &row,
                &identity.store_scope_id,
                identity.store_epoch,
                &tenant_scope_id,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(TenantFactProjectionSnapshot {
        tenant_scope_id,
        current_frontier,
        publications,
    })
}

async fn select_fact_publication_rows(
    transaction: &mut Transaction<'_, Postgres>,
    identity: &StructuredStoreIdentity,
    tenant_scope_id: &TenantScopeId,
    first_order: u64,
    through_order: u64,
    maximum_items: u32,
) -> Result<Vec<PgRow>, StructuredStoreError> {
    sqlx::query(
        "SELECT publication.store_scope_id, publication.store_epoch::text AS store_epoch, \
                publication.tenant_scope_id, publication.fact_order::text AS fact_order, \
                publication.run_id, publication.run_sequence::text AS run_sequence, \
                publication.transition_ordinal, publication.transition_record_hash, \
                batch.head_commit_digest \
           FROM tenant_fact_publications AS publication \
           JOIN run_history_batches AS batch \
             ON batch.run_id = publication.run_id \
            AND batch.run_sequence = publication.run_sequence \
          WHERE publication.store_scope_id = $1 \
            AND publication.store_epoch = $2::numeric \
            AND publication.tenant_scope_id = $3 \
            AND publication.fact_order >= $4::numeric \
            AND publication.fact_order <= $5::numeric \
          ORDER BY publication.fact_order LIMIT $6::bigint",
    )
    .bind(identity.store_scope_id.as_str())
    .bind(identity.store_epoch.get().to_string())
    .bind(tenant_scope_id.as_str())
    .bind(first_order.to_string())
    .bind(through_order.to_string())
    .bind(i64::from(maximum_items))
    .fetch_all(&mut **transaction)
    .await
    .map_err(|_| StructuredStoreError::BackendUnavailable)
}

async fn load_locked_run_projection(
    transaction: &mut Transaction<'_, Postgres>,
    run_id: &RunId,
    identity: &StructuredStoreIdentity,
) -> Result<Option<RunCurrentProjection>, StructuredStoreError> {
    sqlx::query(
        "SELECT store_scope_id, store_epoch::text AS store_epoch, tenant_scope_id, \
                head_sequence::text AS head_sequence, head_commit_digest, \
                has_effect_entry_attention \
           FROM run_history_heads WHERE run_id = $1 FOR UPDATE",
    )
    .bind(run_id.as_str())
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|_| StructuredStoreError::BackendUnavailable)?
    .map(|row| decode_run_projection(&row, run_id, identity))
    .transpose()
}

async fn load_locked_fact_frontier(
    transaction: &mut Transaction<'_, Postgres>,
    identity: &StructuredStoreIdentity,
    tenant_scope_id: &TenantScopeId,
) -> Result<TenantFactFrontier, StructuredStoreError> {
    let rows = sqlx::query(
        "SELECT store_scope_id, store_epoch::text AS store_epoch, tenant_scope_id, \
                fact_order::text AS fact_order \
           FROM tenant_fact_heads WHERE tenant_scope_id = $1 FOR UPDATE",
    )
    .bind(tenant_scope_id.as_str())
    .fetch_all(&mut **transaction)
    .await
    .map_err(|_| StructuredStoreError::BackendUnavailable)?;
    let order = match rows.as_slice() {
        [] => 0,
        [row]
            if required_text(row, "store_scope_id")? == identity.store_scope_id.as_str()
                && required_text(row, "store_epoch")? == identity.store_epoch.get().to_string()
                && required_text(row, "tenant_scope_id")? == tenant_scope_id.as_str() =>
        {
            parse_fact_order(&required_text(row, "fact_order")?, false)?
        }
        _ => {
            return Err(invalid(
                "PostgreSQL fact head is not unique or changed identity",
            ))
        }
    };
    Ok(TenantFactFrontier::new(
        identity.store_scope_id.clone(),
        identity.store_epoch,
        tenant_scope_id.clone(),
        order,
    ))
}

async fn select_batch_rows(
    transaction: &mut Transaction<'_, Postgres>,
    query_kind: StructuredBatchQuery,
    run_id: &RunId,
    append_request_id: Option<&str>,
) -> Result<Vec<StoredBatchRow>, StructuredStoreError> {
    let mut query = sqlx::query(crate::sql_catalog::structured_select_batches(query_kind))
        .bind(run_id.as_str());
    if let Some(append_request_id) = append_request_id {
        query = query.bind(append_request_id);
    }
    query
        .fetch_all(&mut **transaction)
        .await
        .map_err(|_| StructuredStoreError::BackendUnavailable)?
        .iter()
        .map(StoredBatchRow::decode)
        .collect()
}

fn exactly_one_or_none(
    mut rows: Vec<StoredBatchRow>,
) -> Result<Option<StoredBatchRow>, StructuredStoreError> {
    if rows.len() > 1 {
        return Err(invalid(
            "structured PostgreSQL append identity is not unique",
        ));
    }
    Ok(rows.pop())
}

async fn load_object_rows(
    transaction: &mut Transaction<'_, Postgres>,
    run_id: &RunId,
    run_sequence: Option<u64>,
) -> Result<BTreeMap<u64, Vec<StoredObjectRow>>, StructuredStoreError> {
    let rows = if let Some(run_sequence) = run_sequence {
        sqlx::query(
            "SELECT run_sequence::text AS run_sequence, object_ordinal, object_type, \
                    content_schema_id, content_digest, canonical_json \
               FROM run_history_batch_objects \
              WHERE run_id = $1 AND run_sequence = $2::numeric \
              ORDER BY run_history_batch_objects.run_sequence, object_ordinal",
        )
        .bind(run_id.as_str())
        .bind(run_sequence.to_string())
        .fetch_all(&mut **transaction)
        .await
    } else {
        sqlx::query(
            "SELECT run_sequence::text AS run_sequence, object_ordinal, object_type, \
                    content_schema_id, content_digest, canonical_json \
               FROM run_history_batch_objects \
              WHERE run_id = $1 \
              ORDER BY run_history_batch_objects.run_sequence, object_ordinal",
        )
        .bind(run_id.as_str())
        .fetch_all(&mut **transaction)
        .await
    }
    .map_err(|_| StructuredStoreError::BackendUnavailable)?;

    group_object_rows(rows)
}

async fn load_object_rows_through(
    transaction: &mut Transaction<'_, Postgres>,
    run_id: &RunId,
    through: u64,
) -> Result<BTreeMap<u64, Vec<StoredObjectRow>>, StructuredStoreError> {
    let rows = sqlx::query(
        "SELECT run_sequence::text AS run_sequence, object_ordinal, object_type, \
                content_schema_id, content_digest, canonical_json \
           FROM run_history_batch_objects \
          WHERE run_id = $1 AND run_sequence <= $2::numeric \
          ORDER BY run_history_batch_objects.run_sequence, object_ordinal",
    )
    .bind(run_id.as_str())
    .bind(through.to_string())
    .fetch_all(&mut **transaction)
    .await
    .map_err(|_| StructuredStoreError::BackendUnavailable)?;

    group_object_rows(rows)
}

fn group_object_rows(
    rows: Vec<PgRow>,
) -> Result<BTreeMap<u64, Vec<StoredObjectRow>>, StructuredStoreError> {
    let mut grouped = BTreeMap::<u64, Vec<StoredObjectRow>>::new();
    for row in rows {
        let sequence = required_sequence(&row, "run_sequence")?;
        grouped.entry(sequence).or_default().push(StoredObjectRow {
            ordinal: row
                .try_get("object_ordinal")
                .map_err(|_| invalid("structured PostgreSQL object ordinal is absent"))?,
            object_type: required_text(&row, "object_type")?,
            content_schema_id: required_text(&row, "content_schema_id")?,
            content_digest: required_text(&row, "content_digest")?,
            canonical_json: required_text(&row, "canonical_json")?,
        });
    }
    Ok(grouped)
}

async fn insert_object_rows(
    transaction: &mut Transaction<'_, Postgres>,
    run_id: &RunId,
    run_sequence: u64,
    objects: &[HistoryObject],
) -> Result<(), StructuredStoreError> {
    let sequence = run_sequence.to_string();
    for (chunk_index, chunk) in objects.chunks(OBJECT_INSERT_CHUNK_SIZE).enumerate() {
        let first_ordinal = chunk_index * OBJECT_INSERT_CHUNK_SIZE;
        let mut query =
            QueryBuilder::<Postgres>::new(crate::sql_catalog::structured_insert_object_rows());
        query.push_values(chunk.iter().enumerate(), |mut row, (offset, object)| {
            let ordinal = i32::try_from(first_ordinal + offset)
                .expect("bounded batch object ordinal must fit i32");
            row.push_bind(run_id.as_str())
                .push_bind(sequence.as_str())
                .push_unseparated("::numeric")
                .push_bind(ordinal)
                .push_bind(object.object_type.as_str())
                .push_bind(object.content_ref.schema_id().as_str())
                .push_bind(object.content_ref.content_digest().as_str())
                .push_bind(object.canonical_json.as_str());
        });
        query
            .build()
            .execute(&mut **transaction)
            .await
            .map_err(|_| StructuredStoreError::BackendUnavailable)?;
    }
    Ok(())
}

fn decode_canonical_envelope(json: &str) -> Result<StoredBatchEnvelope, StructuredStoreError> {
    validate_stored_frame(json)?;
    let value: serde_json::Value = serde_json::from_str(json)
        .map_err(|_| invalid("structured PostgreSQL batch envelope cannot be strictly decoded"))?;
    // Decode the bounded record wrappers field-by-field. The derived serde visitor for the
    // inline `AssignedRecord` vector can recurse through the closed record algebra; explicit
    // field closure keeps reload stack-bounded while retaining strict unknown-field rejection.
    let object = value
        .as_object()
        .ok_or_else(|| invalid("structured PostgreSQL batch envelope is not an object"))?;
    const FIELD_NAMES: [&str; 9] = [
        "store_scope_id",
        "store_epoch",
        "predecessor",
        "append_request_id",
        "tenant_fact_coordinate",
        "candidate_digest",
        "records",
        "object_count",
        "head",
    ];
    if object.len() != FIELD_NAMES.len()
        || object
            .keys()
            .any(|key| !FIELD_NAMES.contains(&key.as_str()))
    {
        return Err(invalid(
            "structured PostgreSQL batch envelope contains an unknown field",
        ));
    }
    let field = |name: &str| {
        object
            .get(name)
            .cloned()
            .ok_or_else(|| invalid("structured PostgreSQL batch envelope field is absent"))
    };
    let records_value = field("records")?;
    let records_array = records_value
        .as_array()
        .ok_or_else(|| invalid("structured PostgreSQL batch envelope records are not an array"))?;
    let mut records = Vec::with_capacity(records_array.len());
    for record_value in records_array {
        let record_object = record_value.as_object().ok_or_else(|| {
            invalid("structured PostgreSQL batch envelope record is not an object")
        })?;
        if record_object.len() != 2
            || record_object
                .keys()
                .any(|key| key != "record_ref" && key != "record")
        {
            return Err(invalid(
                "structured PostgreSQL batch envelope record contains an unknown field",
            ));
        }
        let record_ref = record_object
            .get("record_ref")
            .cloned()
            .ok_or_else(|| invalid("structured PostgreSQL record reference is absent"))?;
        let record = record_object
            .get("record")
            .cloned()
            .ok_or_else(|| invalid("structured PostgreSQL record payload is absent"))?;
        records.push(AssignedRecord {
            record_ref: serde_json::from_value(record_ref)
                .map_err(|_| invalid("structured PostgreSQL record reference is invalid"))?,
            record: serde_json::from_value(record)
                .map_err(|_| invalid("structured PostgreSQL record payload is invalid"))?,
        });
    }
    let envelope = StoredBatchEnvelope {
        store_scope_id: serde_json::from_value(field("store_scope_id")?)
            .map_err(|_| invalid("structured PostgreSQL store scope is invalid"))?,
        store_epoch: serde_json::from_value(field("store_epoch")?)
            .map_err(|_| invalid("structured PostgreSQL store epoch is invalid"))?,
        predecessor: serde_json::from_value(field("predecessor")?)
            .map_err(|_| invalid("structured PostgreSQL predecessor is invalid"))?,
        append_request_id: serde_json::from_value(field("append_request_id")?)
            .map_err(|_| invalid("structured PostgreSQL append request is invalid"))?,
        tenant_fact_coordinate: serde_json::from_value(field("tenant_fact_coordinate")?)
            .map_err(|_| invalid("structured PostgreSQL tenant fact coordinate is invalid"))?,
        candidate_digest: serde_json::from_value(field("candidate_digest")?)
            .map_err(|_| invalid("structured PostgreSQL candidate digest is invalid"))?,
        records,
        object_count: serde_json::from_value(field("object_count")?)
            .map_err(|_| invalid("structured PostgreSQL object count is invalid"))?,
        head: serde_json::from_value(field("head")?)
            .map_err(|_| invalid("structured PostgreSQL head is invalid"))?,
    };
    if usize::try_from(envelope.object_count)
        .ok()
        .is_none_or(|count| count > MAX_BATCH_OBJECTS)
    {
        return Err(invalid(
            "structured PostgreSQL batch envelope object count exceeds its bound",
        ));
    }
    let canonical = canonical_json(&envelope).map_err(|_| {
        invalid("structured PostgreSQL batch envelope cannot be canonically encoded")
    })?;
    if canonical.as_str() != json {
        return Err(invalid(
            "structured PostgreSQL batch envelope text is not canonical",
        ));
    }
    Ok(envelope)
}

fn validate_stored_frame(json: &str) -> Result<(), StructuredStoreError> {
    if json.len() < 2 || json.len() > MAX_STORED_FRAME_BYTES {
        return Err(invalid("PostgreSQL canonical frame exceeds its bound"));
    }
    PlainCanonicalJsonBytes::from_canonical_json_slice(json.as_bytes())
        .map_err(|_| invalid("PostgreSQL canonical frame is invalid"))?;
    Ok(())
}

fn decode_objects(rows: Vec<StoredObjectRow>) -> Result<Vec<HistoryObject>, StructuredStoreError> {
    if rows.len() > MAX_BATCH_OBJECTS {
        return Err(invalid(
            "structured PostgreSQL batch object rows exceed their bound",
        ));
    }
    let mut objects = Vec::with_capacity(rows.len());
    for (expected_ordinal, row) in rows.into_iter().enumerate() {
        if usize::try_from(row.ordinal).ok() != Some(expected_ordinal)
            || row.canonical_json.is_empty()
            || row.canonical_json.len() > MAX_STORED_FRAME_BYTES
        {
            return Err(invalid(
                "structured PostgreSQL object rows are not dense bounded frames",
            ));
        }
        let object_type = StableId::new(row.object_type)
            .map_err(|_| invalid("structured PostgreSQL object type is invalid"))?;
        let schema_id = SchemaId::parse(row.content_schema_id)
            .map_err(|_| invalid("structured PostgreSQL object schema is invalid"))?;
        let content_digest = ContentDigest::parse(row.content_digest)
            .map_err(|_| invalid("structured PostgreSQL object digest is invalid"))?;
        let content_ref = ContentRef::new(schema_id, content_digest)
            .map_err(|_| invalid("structured PostgreSQL object content reference is invalid"))?;
        let object = HistoryObject {
            object_type,
            content_ref,
            canonical_json: row.canonical_json,
        };
        object
            .validate()
            .map_err(|_| invalid("structured PostgreSQL object content reference differs"))?;
        if objects
            .last()
            .is_some_and(|prior: &HistoryObject| prior.content_ref >= object.content_ref)
        {
            return Err(invalid(
                "structured PostgreSQL object rows are not in canonical reference order",
            ));
        }
        objects.push(object);
    }
    Ok(objects)
}

fn required_text(row: &PgRow, column: &str) -> Result<String, StructuredStoreError> {
    row.try_get(column)
        .map_err(|_| invalid("structured PostgreSQL retained text is absent"))
}

fn required_sequence(row: &PgRow, column: &str) -> Result<u64, StructuredStoreError> {
    let value = required_text(row, column)?;
    parse_sequence(&value)
}

fn optional_sequence(row: &PgRow, column: &str) -> Result<Option<u64>, StructuredStoreError> {
    row.try_get::<Option<String>, _>(column)
        .map_err(|_| invalid("structured PostgreSQL optional sequence is invalid"))?
        .map(|value| parse_sequence(&value))
        .transpose()
}

fn parse_sequence(value: &str) -> Result<u64, StructuredStoreError> {
    let parsed = value
        .parse::<u64>()
        .map_err(|_| invalid("structured PostgreSQL sequence is invalid"))?;
    if parsed == 0 || parsed.to_string() != value {
        return Err(invalid("structured PostgreSQL sequence is not canonical"));
    }
    Ok(parsed)
}

fn parse_fact_order(value: &str, allow_zero: bool) -> Result<u64, StructuredStoreError> {
    let parsed = value
        .parse::<u64>()
        .map_err(|_| invalid("structured PostgreSQL fact order is invalid"))?;
    if (!allow_zero && parsed == 0) || parsed.to_string() != value {
        return Err(invalid("structured PostgreSQL fact order is not canonical"));
    }
    Ok(parsed)
}

const fn invalid(_message: &'static str) -> StructuredStoreError {
    StructuredStoreError::InvalidHistory
}

fn is_contention_sqlstate(error: &sqlx::Error) -> bool {
    matches!(
        error,
        sqlx::Error::Database(database)
            if matches!(
                database.code().as_deref(),
                Some("23505" | "40001" | "40P01")
            )
    )
}

impl PostgresStructuredHistoryBackend {
    async fn classify_existing_after_contention(
        &self,
        run_id: &RunId,
        committed: &CommittedBatch,
    ) -> Result<BackendAppendOutcome, StructuredStoreError> {
        let mut transaction = self.begin_locked_write(run_id.as_str(), None).await?;
        let existing_rows = select_batch_rows(
            transaction.conn(),
            StructuredBatchQuery::ByAppendRequest,
            run_id,
            Some(committed.append_request_id.as_str()),
        )
        .await?;
        if let Some(existing_row) = exactly_one_or_none(existing_rows)? {
            let existing_sequence = existing_row.run_sequence;
            let mut object_rows =
                load_object_rows(transaction.conn(), run_id, Some(existing_sequence)).await?;
            let existing = existing_row
                .reconstruct(object_rows.remove(&existing_sequence).unwrap_or_default())?;
            if !object_rows.is_empty() {
                return Err(invalid(
                    "structured PostgreSQL idempotent objects differ from their batch",
                ));
            }
            return match transaction.commit_outcome(&self.target).await? {
                crate::transaction::CommitOutcome::AcknowledgementUnknown => {
                    Ok(BackendAppendOutcome::AcknowledgementUnknown)
                }
                crate::transaction::CommitOutcome::Committed if existing == *committed => {
                    Ok(BackendAppendOutcome::ExistingSame(existing))
                }
                crate::transaction::CommitOutcome::Committed => {
                    Err(StructuredStoreError::AppendConflict)
                }
            };
        }
        transaction.rollback().await?;
        Ok(BackendAppendOutcome::StaleHead)
    }
}
