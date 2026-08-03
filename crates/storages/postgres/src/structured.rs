use std::collections::BTreeMap;

use mfm_ids::{
    AppendRequestId, ContentDigest, ContentRef, JournalRecordHash, RunId, SchemaId, StableId,
    StoreEpoch, StoreScopeId, TenantScopeId,
};
use mfm_journal::structured::{
    canonical_json, AssignedRecord, CommittedBatch, HistoryObject, JournalHead, RecordRef,
    RunRecord, TenantFactCoordinate, TenantFactFrontier,
};
use mfm_store::structured::{
    BackendAppendOutcome, RawRunHistory, StructuredBackendFuture, StructuredHistoryBackend,
    StructuredStoreError, StructuredStoreIdentity, TenantFactPublication, ValidatedBatch,
};
use serde::{Deserialize, Serialize};
use sqlx::postgres::PgRow;
use sqlx::{Postgres, QueryBuilder, Row, Transaction};

use crate::session::{
    ApplicationTargetSessions, CombinedTargetSessions, RoleSession, TargetBinding,
};
use crate::transaction::{
    begin_read, begin_run_write, lock_run, lock_tenant_fact, store_identity_from_binding,
    CommitOutcome, LockedWriteTx, ReadTx,
};

const MAX_STORED_FRAME_BYTES: usize = 16_777_216;
const MAX_BATCH_OBJECTS: usize = 65_536;
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

struct PendingTenantFactPublication {
    frontier: TenantFactFrontier,
    transition_ref: RecordRef,
    predecessor_order: u64,
}

struct TenantFactRouteSummary {
    publication_count: u64,
    minimum_order: Option<u64>,
    maximum_order: Option<u64>,
}

impl TenantFactRouteSummary {
    fn is_dense_through(&self, fact_order: u64) -> bool {
        self.publication_count == fact_order
            && match fact_order {
                0 => self.minimum_order.is_none() && self.maximum_order.is_none(),
                _ => self.minimum_order == Some(1) && self.maximum_order == Some(fact_order),
            }
    }
}

/// Real PostgreSQL implementation of the shared structured-history backend seam.
pub struct PostgresStructuredHistoryBackend {
    run_reader: RoleSession,
    run_writer: RoleSession,
    target: TargetBinding,
    identity: StructuredStoreIdentity,
}

impl std::fmt::Debug for PostgresStructuredHistoryBackend {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PostgresStructuredHistoryBackend")
            .field("identity", &self.identity)
            .finish_non_exhaustive()
    }
}

impl PostgresStructuredHistoryBackend {
    pub(crate) fn from_application_sessions(sessions: ApplicationTargetSessions) -> Self {
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
    ) -> Result<LockedWriteTx<'_>, StructuredStoreError> {
        let write = begin_run_write(&self.run_writer, &self.target).await?;
        lock_run(write, run_id).await
    }
}

impl StructuredHistoryBackend for PostgresStructuredHistoryBackend {
    fn identity(&self) -> &StructuredStoreIdentity {
        &self.identity
    }

    fn load<'a>(&'a self, run_id: &'a RunId) -> StructuredBackendFuture<'a, Option<RawRunHistory>> {
        Box::pin(async move {
            let mut transaction = self.begin_read().await?;
            let rows = select_batch_rows(
                transaction.conn(),
                "SELECT run_id, run_sequence::text AS run_sequence, append_request_id, \
                        candidate_digest, predecessor_sequence::text AS predecessor_sequence, \
                        predecessor_commit_digest, head_commit_digest, batch_envelope_json \
                   FROM run_history_batches WHERE run_id = $1 \
                  ORDER BY run_history_batches.run_sequence",
                run_id,
                None,
            )
            .await?;
            if rows.is_empty() {
                transaction.commit().await?;
                return Ok(None);
            }
            let mut objects = load_object_rows(transaction.conn(), run_id, None).await?;
            let mut batches = Vec::with_capacity(rows.len());
            for row in rows {
                let sequence = row.run_sequence;
                let object_rows = objects.remove(&sequence).unwrap_or_default();
                batches.push(row.reconstruct(object_rows)?);
            }
            if !objects.is_empty() {
                return Err(invalid(
                    "structured PostgreSQL object rows have no retained batch",
                ));
            }
            transaction.commit().await?;
            Ok(Some(RawRunHistory {
                run_id: run_id.clone(),
                batches,
            }))
        })
    }

    fn load_prefix<'a>(
        &'a self,
        run_id: &'a RunId,
        through_sequence: u64,
    ) -> StructuredBackendFuture<'a, Option<RawRunHistory>> {
        Box::pin(async move {
            if through_sequence == 0 {
                return Err(invalid(
                    "structured PostgreSQL prefix sequence must be positive",
                ));
            }
            let mut transaction = self.begin_read().await?;
            let rows = sqlx::query(
                "SELECT run_id, run_sequence::text AS run_sequence, append_request_id, \
                        candidate_digest, predecessor_sequence::text AS predecessor_sequence, \
                        predecessor_commit_digest, head_commit_digest, batch_envelope_json \
                   FROM run_history_batches \
                  WHERE run_id = $1 AND run_sequence <= $2::numeric \
                  ORDER BY run_history_batches.run_sequence",
            )
            .bind(run_id.as_str())
            .bind(through_sequence.to_string())
            .fetch_all(&mut **transaction.conn())
            .await
            .map_err(|_| StructuredStoreError::BackendUnavailable)?
            .iter()
            .map(StoredBatchRow::decode)
            .collect::<Result<Vec<_>, _>>()?;
            if rows.is_empty() {
                transaction.commit().await?;
                return Ok(None);
            }
            let mut objects =
                load_object_rows_through(transaction.conn(), run_id, through_sequence).await?;
            let mut batches = Vec::with_capacity(rows.len());
            for row in rows {
                let sequence = row.run_sequence;
                let object_rows = objects.remove(&sequence).unwrap_or_default();
                batches.push(row.reconstruct(object_rows)?);
            }
            if !objects.is_empty() {
                return Err(invalid(
                    "structured PostgreSQL prefix objects have no retained batch",
                ));
            }
            transaction.commit().await?;
            Ok(Some(RawRunHistory {
                run_id: run_id.clone(),
                batches,
            }))
        })
    }

    fn tenant_fact_frontier<'a>(
        &'a self,
        tenant_scope_id: &'a TenantScopeId,
    ) -> StructuredBackendFuture<'a, TenantFactFrontier> {
        Box::pin(async move {
            let mut transaction = self.begin_read().await?;
            let rows = sqlx::query(
                "SELECT store_scope_id, store_epoch::text AS store_epoch, \
                        tenant_scope_id, fact_order::text AS fact_order \
                   FROM tenant_fact_heads WHERE tenant_scope_id = $1",
            )
            .bind(tenant_scope_id.as_str())
            .fetch_all(&mut **transaction.conn())
            .await
            .map_err(|_| StructuredStoreError::BackendUnavailable)?;
            let route_summary =
                load_tenant_fact_route_summary(transaction.conn(), tenant_scope_id).await?;
            let fact_order = match rows.as_slice() {
                [] => 0,
                [row]
                    if required_text(row, "store_scope_id")?
                        == self.identity.store_scope_id.as_str()
                        && required_text(row, "store_epoch")?
                            == self.identity.store_epoch.get().to_string()
                        && required_text(row, "tenant_scope_id")? == tenant_scope_id.as_str() =>
                {
                    parse_fact_order(&required_text(row, "fact_order")?, true)?
                }
                _ => {
                    return Err(invalid(
                        "structured PostgreSQL tenant fact head changed store identity",
                    ));
                }
            };
            if !route_summary.is_dense_through(fact_order) {
                return Err(invalid(
                    "structured PostgreSQL tenant fact head differs from dense routes",
                ));
            }
            transaction.commit().await?;
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
            let rows = sqlx::query(
                "SELECT store_scope_id, store_epoch::text AS store_epoch, tenant_scope_id, \
                        fact_order::text AS fact_order, run_id, \
                        run_sequence::text AS run_sequence, transition_ordinal, \
                        transition_record_hash \
                   FROM tenant_fact_publications \
                  WHERE store_scope_id = $1 AND store_epoch = $2::numeric \
                    AND tenant_scope_id = $3 \
                    AND fact_order >= $4::numeric AND fact_order <= $5::numeric \
                  ORDER BY fact_order LIMIT $6",
            )
            .bind(self.identity.store_scope_id.as_str())
            .bind(self.identity.store_epoch.get().to_string())
            .bind(tenant_scope_id.as_str())
            .bind(first_order.to_string())
            .bind(through_order.to_string())
            .bind(i64::from(maximum_items))
            .fetch_all(&mut **transaction.conn())
            .await
            .map_err(|_| StructuredStoreError::BackendUnavailable)?;
            let mut publications = Vec::with_capacity(rows.len());
            for row in rows {
                if required_text(&row, "store_scope_id")? != self.identity.store_scope_id.as_str()
                    || required_text(&row, "store_epoch")?
                        != self.identity.store_epoch.get().to_string()
                    || required_text(&row, "tenant_scope_id")? != tenant_scope_id.as_str()
                {
                    return Err(invalid(
                        "structured PostgreSQL tenant publication changed identity",
                    ));
                }
                let fact_order = parse_fact_order(&required_text(&row, "fact_order")?, false)?;
                let run_id = RunId::parse(required_text(&row, "run_id")?)
                    .map_err(|_| invalid("structured PostgreSQL publication run id is invalid"))?;
                let run_sequence = required_sequence(&row, "run_sequence")?;
                let ordinal = row
                    .try_get::<i32, _>("transition_ordinal")
                    .ok()
                    .and_then(|value| u32::try_from(value).ok())
                    .ok_or_else(|| {
                        invalid("structured PostgreSQL publication ordinal is invalid")
                    })?;
                let record_hash =
                    JournalRecordHash::parse(required_text(&row, "transition_record_hash")?)
                        .map_err(|_| {
                            invalid("structured PostgreSQL publication record hash is invalid")
                        })?;
                publications.push(TenantFactPublication {
                    frontier: TenantFactFrontier::new(
                        self.identity.store_scope_id.clone(),
                        self.identity.store_epoch,
                        tenant_scope_id.clone(),
                        fact_order,
                    ),
                    transition_ref: RecordRef {
                        run_id,
                        run_sequence,
                        ordinal,
                        record_hash,
                    },
                });
            }
            transaction.commit().await?;
            Ok(publications)
        })
    }

    fn append<'a>(
        &'a self,
        batch: ValidatedBatch,
    ) -> StructuredBackendFuture<'a, BackendAppendOutcome> {
        Box::pin(async move {
            let committed = batch.into_committed();
            if committed.store_scope_id != self.identity.store_scope_id
                || committed.store_epoch != self.identity.store_epoch
            {
                return Err(StructuredStoreError::StaleHead);
            }
            let run_id = committed
                .records
                .first()
                .ok_or_else(|| invalid("validated PostgreSQL batch has no record"))?
                .record_ref
                .run_id
                .clone();
            if committed
                .records
                .iter()
                .any(|record| record.record_ref.run_id != run_id)
            {
                return Err(invalid("validated PostgreSQL batch spans runs"));
            }
            validate_objects_for_storage(&committed.objects)?;
            let envelope = StoredBatchEnvelope::from_batch(&committed)?;
            let canonical_envelope = canonical_json(&envelope)
                .map_err(|_| invalid("validated PostgreSQL batch envelope is not canonical"))?;
            if canonical_envelope.as_bytes().len() > MAX_STORED_FRAME_BYTES {
                return Err(invalid(
                    "validated PostgreSQL batch envelope exceeds its byte bound",
                ));
            }

            let mut transaction = self.begin_locked_write(run_id.as_str()).await?;

            let existing_rows = select_batch_rows(
                transaction.conn(),
                "SELECT run_id, run_sequence::text AS run_sequence, append_request_id, \
                        candidate_digest, predecessor_sequence::text AS predecessor_sequence, \
                        predecessor_commit_digest, head_commit_digest, batch_envelope_json \
                   FROM run_history_batches \
                  WHERE run_id = $1 AND append_request_id = $2",
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
                transaction.commit().await?;
                return if existing == committed {
                    Ok(BackendAppendOutcome::ExistingSame(existing))
                } else {
                    Err(StructuredStoreError::AppendConflict)
                };
            }

            let head = sqlx::query(
                "SELECT head_sequence::text AS head_sequence, head_commit_digest \
                   FROM run_history_heads WHERE run_id = $1 FOR UPDATE",
            )
            .bind(run_id.as_str())
            .fetch_optional(&mut **transaction.conn())
            .await
            .map_err(|_| StructuredStoreError::BackendUnavailable)?;
            let predecessor_matches = match (head.as_ref(), committed.predecessor.as_ref()) {
                (None, None) => true,
                (Some(row), Some(predecessor)) => {
                    row.try_get::<String, _>("head_sequence").ok().as_deref()
                        == Some(predecessor.run_sequence.to_string().as_str())
                        && row
                            .try_get::<String, _>("head_commit_digest")
                            .ok()
                            .as_deref()
                            == Some(predecessor.commit_digest.as_str())
                }
                _ => false,
            };
            if !predecessor_matches {
                transaction.rollback().await?;
                return Ok(BackendAppendOutcome::StaleHead);
            }

            let pending_tenant_publication = match &committed.tenant_fact_coordinate {
                TenantFactCoordinate::None => None,
                TenantFactCoordinate::FactPublication { frontier }
                | TenantFactCoordinate::FactSelectionBarrier { frontier } => {
                    if frontier.store_scope_id != self.identity.store_scope_id
                        || frontier.store_epoch != self.identity.store_epoch
                    {
                        transaction.rollback().await?;
                        return Ok(BackendAppendOutcome::StaleHead);
                    }
                    let tenant_lock_key = format!(
                        "{}:{}:{}",
                        frontier.store_scope_id.as_str(),
                        frontier.store_epoch.get(),
                        frontier.tenant_scope_id.as_str()
                    );
                    lock_tenant_fact(&mut transaction, &tenant_lock_key).await?;
                    sqlx::query(
                        "INSERT INTO tenant_fact_heads ( \
                            store_scope_id, store_epoch, tenant_scope_id, fact_order \
                         ) VALUES ($1, $2::numeric, $3, 0) ON CONFLICT DO NOTHING",
                    )
                    .bind(self.identity.store_scope_id.as_str())
                    .bind(self.identity.store_epoch.get().to_string())
                    .bind(frontier.tenant_scope_id.as_str())
                    .execute(&mut **transaction.conn())
                    .await
                    .map_err(|_| StructuredStoreError::BackendUnavailable)?;
                    let rows = sqlx::query(
                        "SELECT store_scope_id, store_epoch::text AS store_epoch, \
                                tenant_scope_id, fact_order::text AS fact_order \
                           FROM tenant_fact_heads WHERE tenant_scope_id = $1 FOR UPDATE",
                    )
                    .bind(frontier.tenant_scope_id.as_str())
                    .fetch_all(&mut **transaction.conn())
                    .await
                    .map_err(|_| StructuredStoreError::BackendUnavailable)?;
                    let [head] = rows.as_slice() else {
                        return Err(invalid(
                            "structured PostgreSQL tenant fact head is not unique",
                        ));
                    };
                    if required_text(head, "store_scope_id")?
                        != self.identity.store_scope_id.as_str()
                        || required_text(head, "store_epoch")?
                            != self.identity.store_epoch.get().to_string()
                        || required_text(head, "tenant_scope_id")?
                            != frontier.tenant_scope_id.as_str()
                    {
                        return Err(invalid(
                            "structured PostgreSQL tenant fact head changed identity",
                        ));
                    }
                    let current_order =
                        parse_fact_order(&required_text(head, "fact_order")?, true)?;
                    if !load_tenant_fact_route_summary(transaction.conn(), &frontier.tenant_scope_id)
                        .await?
                        .is_dense_through(current_order)
                    {
                        return Err(invalid(
                            "structured PostgreSQL tenant fact head differs from dense routes",
                        ));
                    }
                    match &committed.tenant_fact_coordinate {
                        TenantFactCoordinate::FactSelectionBarrier { .. } => {
                            if frontier.fact_order != current_order {
                                transaction.rollback().await?;
                                return Ok(BackendAppendOutcome::StaleHead);
                            }
                            None
                        }
                        TenantFactCoordinate::FactPublication { .. } => {
                            if current_order.checked_add(1) != Some(frontier.fact_order) {
                                transaction.rollback().await?;
                                return Ok(BackendAppendOutcome::StaleHead);
                            }
                            let transition = committed.records.first().ok_or_else(|| {
                                invalid("structured PostgreSQL publication has no transition")
                            })?;
                            let RunRecord::StateTransitionCommitted(record) = &transition.record
                            else {
                                return Err(invalid(
                                    "structured PostgreSQL publication route is not a transition",
                                ));
                            };
                            if record.facts.is_empty() {
                                return Err(invalid(
                                    "structured PostgreSQL publication transition has no facts",
                                ));
                            }
                            Some(PendingTenantFactPublication {
                                frontier: frontier.clone(),
                                transition_ref: transition.record_ref.clone(),
                                predecessor_order: current_order,
                            })
                        }
                        TenantFactCoordinate::None => {
                            return Err(invalid(
                                "structured PostgreSQL tenant coordinate changed while locked",
                            ));
                        }
                    }
                }
            };

            let predecessor_sequence = committed
                .predecessor
                .as_ref()
                .map(|head| head.run_sequence.to_string());
            let predecessor_digest = committed
                .predecessor
                .as_ref()
                .map(|head| head.commit_digest.as_str());
            sqlx::query(
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
            .await
            .map_err(|_| StructuredStoreError::BackendUnavailable)?;
            insert_object_rows(
                transaction.conn(),
                &run_id,
                committed.head.run_sequence,
                &committed.objects,
            )
            .await?;

            let affected = if let Some(predecessor) = &committed.predecessor {
                sqlx::query(
                    "UPDATE run_history_heads \
                        SET head_sequence = $2::numeric, head_commit_digest = $3 \
                      WHERE run_id = $1 \
                        AND head_sequence = $4::numeric AND head_commit_digest = $5 \
                        AND store_scope_id = $6 AND store_epoch = $7::numeric",
                )
                .bind(run_id.as_str())
                .bind(committed.head.run_sequence.to_string())
                .bind(committed.head.commit_digest.as_str())
                .bind(predecessor.run_sequence.to_string())
                .bind(predecessor.commit_digest.as_str())
                .bind(self.identity.store_scope_id.as_str())
                .bind(self.identity.store_epoch.get().to_string())
                .execute(&mut **transaction.conn())
                .await
                .map_err(|_| StructuredStoreError::BackendUnavailable)?
                .rows_affected()
            } else {
                sqlx::query(
                    "INSERT INTO run_history_heads ( \
                        run_id, store_scope_id, store_epoch, head_sequence, head_commit_digest \
                     ) VALUES ($1, $2, $3::numeric, $4::numeric, $5)",
                )
                .bind(run_id.as_str())
                .bind(self.identity.store_scope_id.as_str())
                .bind(self.identity.store_epoch.get().to_string())
                .bind(committed.head.run_sequence.to_string())
                .bind(committed.head.commit_digest.as_str())
                .execute(&mut **transaction.conn())
                .await
                .map_err(|_| StructuredStoreError::BackendUnavailable)?
                .rows_affected()
            };
            if affected != 1 {
                return Err(StructuredStoreError::StaleHead);
            }
            if let Some(publication) = pending_tenant_publication {
                let inserted = sqlx::query(
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
                    i32::try_from(publication.transition_ref.ordinal).map_err(|_| {
                        invalid("structured PostgreSQL publication ordinal exceeds i32")
                    })?,
                )
                .bind(publication.transition_ref.record_hash.as_str())
                .execute(&mut **transaction.conn())
                .await
                .map_err(|_| StructuredStoreError::BackendUnavailable)?
                .rows_affected();
                let advanced = sqlx::query(
                    "UPDATE tenant_fact_heads SET fact_order = $4::numeric \
                      WHERE store_scope_id = $1 AND store_epoch = $2::numeric \
                        AND tenant_scope_id = $3 AND fact_order = $5::numeric",
                )
                .bind(self.identity.store_scope_id.as_str())
                .bind(self.identity.store_epoch.get().to_string())
                .bind(publication.frontier.tenant_scope_id.as_str())
                .bind(publication.frontier.fact_order.to_string())
                .bind(publication.predecessor_order.to_string())
                .execute(&mut **transaction.conn())
                .await
                .map_err(|_| StructuredStoreError::BackendUnavailable)?
                .rows_affected();
                if inserted != 1 || advanced != 1 {
                    transaction.rollback().await?;
                    return Ok(BackendAppendOutcome::StaleHead);
                }
            }
            match transaction.commit_outcome().await? {
                crate::transaction::CommitOutcome::Committed => {
                    Ok(BackendAppendOutcome::NewlyCommitted(committed))
                }
                crate::transaction::CommitOutcome::AcknowledgementUnknown => {
                    Ok(BackendAppendOutcome::AcknowledgementUnknown)
                }
            }
        })
    }

    fn resolve_append<'a>(
        &'a self,
        run_id: &'a RunId,
        append_request_id: &'a AppendRequestId,
        candidate_digest: &'a ContentDigest,
    ) -> StructuredBackendFuture<'a, Option<CommittedBatch>> {
        Box::pin(async move {
            let mut transaction = self.begin_read().await?;
            let rows = select_batch_rows(
                transaction.conn(),
                "SELECT run_id, run_sequence::text AS run_sequence, append_request_id, \
                        candidate_digest, predecessor_sequence::text AS predecessor_sequence, \
                        predecessor_commit_digest, head_commit_digest, batch_envelope_json \
                   FROM run_history_batches \
                  WHERE run_id = $1 AND append_request_id = $2",
                run_id,
                Some(append_request_id.as_str()),
            )
            .await?;
            let Some(row) = exactly_one_or_none(rows)? else {
                transaction.commit().await?;
                return Ok(None);
            };
            let sequence = row.run_sequence;
            let mut object_rows =
                load_object_rows(transaction.conn(), run_id, Some(sequence)).await?;
            let batch = row.reconstruct(object_rows.remove(&sequence).unwrap_or_default())?;
            if !object_rows.is_empty() {
                return Err(invalid(
                    "structured PostgreSQL resolved objects differ from their batch",
                ));
            }
            if batch.append_request_id != *append_request_id
                || batch.candidate_digest != *candidate_digest
                || batch
                    .records
                    .first()
                    .is_none_or(|record| record.record_ref.run_id != *run_id)
            {
                return Err(StructuredStoreError::AppendConflict);
            }
            transaction.commit().await?;
            Ok(Some(batch))
        })
    }
}

async fn load_tenant_fact_route_summary(
    transaction: &mut Transaction<'_, Postgres>,
    tenant_scope_id: &TenantScopeId,
) -> Result<TenantFactRouteSummary, StructuredStoreError> {
    let row = sqlx::query(
        "SELECT count(*)::text AS publication_count, \
                min(fact_order)::text AS minimum_order, \
                max(fact_order)::text AS maximum_order \
           FROM tenant_fact_publications WHERE tenant_scope_id = $1",
    )
    .bind(tenant_scope_id.as_str())
    .fetch_one(&mut **transaction)
    .await
    .map_err(|_| StructuredStoreError::BackendUnavailable)?;
    let publication_count = parse_fact_order(&required_text(&row, "publication_count")?, true)?;
    let minimum_order = row
        .try_get::<Option<String>, _>("minimum_order")
        .map_err(|_| invalid("structured PostgreSQL minimum fact order is invalid"))?
        .map(|value| parse_fact_order(&value, false))
        .transpose()?;
    let maximum_order = row
        .try_get::<Option<String>, _>("maximum_order")
        .map_err(|_| invalid("structured PostgreSQL maximum fact order is invalid"))?
        .map(|value| parse_fact_order(&value, false))
        .transpose()?;
    Ok(TenantFactRouteSummary {
        publication_count,
        minimum_order,
        maximum_order,
    })
}

async fn select_batch_rows(
    transaction: &mut Transaction<'_, Postgres>,
    statement: &'static str,
    run_id: &RunId,
    append_request_id: Option<&str>,
) -> Result<Vec<StoredBatchRow>, StructuredStoreError> {
    let mut query = sqlx::query(statement).bind(run_id.as_str());
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
    through_sequence: u64,
) -> Result<BTreeMap<u64, Vec<StoredObjectRow>>, StructuredStoreError> {
    let rows = sqlx::query(
        "SELECT run_sequence::text AS run_sequence, object_ordinal, object_type, \
                content_schema_id, content_digest, canonical_json \
           FROM run_history_batch_objects \
          WHERE run_id = $1 AND run_sequence <= $2::numeric \
          ORDER BY run_history_batch_objects.run_sequence, object_ordinal",
    )
    .bind(run_id.as_str())
    .bind(through_sequence.to_string())
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
        let mut query = QueryBuilder::<Postgres>::new(
            "INSERT INTO run_history_batch_objects ( \
                run_id, run_sequence, object_ordinal, object_type, content_schema_id, \
                content_digest, canonical_json \
             ) ",
        );
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

fn validate_objects_for_storage(objects: &[HistoryObject]) -> Result<(), StructuredStoreError> {
    if objects.len() > MAX_BATCH_OBJECTS {
        return Err(invalid(
            "validated PostgreSQL batch object count exceeds its bound",
        ));
    }
    if objects
        .windows(2)
        .any(|pair| pair[0].content_ref >= pair[1].content_ref)
    {
        return Err(invalid(
            "validated PostgreSQL batch objects are not in canonical reference order",
        ));
    }
    for object in objects {
        if object.canonical_json.is_empty()
            || object.canonical_json.len() > MAX_STORED_FRAME_BYTES
            || object.validate().is_err()
        {
            return Err(invalid(
                "validated PostgreSQL object bytes are not bounded canonical content",
            ));
        }
    }
    Ok(())
}

fn decode_canonical_envelope(json: &str) -> Result<StoredBatchEnvelope, StructuredStoreError> {
    if json.len() < 2 || json.len() > MAX_STORED_FRAME_BYTES {
        return Err(invalid(
            "structured PostgreSQL batch envelope exceeds its byte bound",
        ));
    }
    let envelope: StoredBatchEnvelope = serde_json::from_str(json)
        .map_err(|_| invalid("structured PostgreSQL batch envelope cannot be strictly decoded"))?;
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
