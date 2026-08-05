use std::collections::BTreeMap;

use mfm_canonical::sha256_digest_bytes;
use mfm_ids::{
    AppendRequestId, ContentDigest, ContentRef, JournalCommitDigest, JournalRecordHash, RunId,
    SchemaId, StableId, StoreEpoch, StoreScopeId, TenantScopeId,
};
use mfm_journal::structured::{
    canonical_json, AssignedRecord, CommittedBatch, HistoryObject, JournalHead, RecordRef,
    RunRecord, TenantFactCoordinate, TenantFactFrontier,
};
use mfm_store::structured::{
    validate_envelope_frame, BackendAppendOutcome, CanonicalRunAppend, RawRunHistory,
    StructuredBackendFuture, StructuredHistoryBackend, StructuredStoreError,
    StructuredStoreIdentity, TenantFactPublication, MAX_BATCH_OBJECTS, MAX_STORED_FRAME_BYTES,
};
use serde::{Deserialize, Serialize};
use sqlx::postgres::PgRow;
use sqlx::{Postgres, QueryBuilder, Row, Transaction};

use crate::checkpoint::{CheckpointKey, CheckpointMutation, CheckpointStream, ReadFixationGuard};
use crate::session::{PostgresApplicationSessions, RoleSession, TargetBinding};
use crate::sql_catalog::StructuredBatchQuery;
use crate::transaction::{
    begin_read, begin_run_write, lock_run_and_tenant, store_identity_from_binding, LockedWriteTx,
    ReadTx,
};

const OBJECT_INSERT_CHUNK_SIZE: usize = 8_192;

fn checkpoint_mutation(
    target: &TargetBinding,
    key: CheckpointKey,
    successor_bytes: Vec<u8>,
) -> CheckpointMutation {
    CheckpointMutation {
        key,
        target: target.checkpoint_target(),
        successor_bytes,
    }
}

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
struct TenantFactCheckpointSuccessor {
    frontier: TenantFactFrontier,
    transition_ref: RecordRef,
    predecessor_order: u64,
}

fn checkpoint_digest(bytes: &[u8]) -> ContentDigest {
    ContentDigest::from_digest(
        mfm_ids::DigestAlgorithm::Sha256V1,
        sha256_digest_bytes(bytes),
    )
}

fn canonical_checkpoint_digest<T: Serialize>(
    value: &T,
) -> Result<ContentDigest, StructuredStoreError> {
    let bytes = canonical_json(value)
        .map_err(|_| invalid("structured PostgreSQL checkpoint value is not canonical"))?;
    Ok(checkpoint_digest(bytes.as_bytes()))
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
    })
}

fn tenant_publication_checkpoint_digest(
    publication: &TenantFactPublication,
) -> Result<ContentDigest, StructuredStoreError> {
    let predecessor_order = publication
        .frontier
        .fact_order
        .checked_sub(1)
        .ok_or_else(|| invalid("structured PostgreSQL publication order is zero"))?;
    canonical_checkpoint_digest(&TenantFactCheckpointSuccessor {
        frontier: publication.frontier.clone(),
        transition_ref: publication.transition_ref.clone(),
        predecessor_order,
    })
}

async fn load_tenant_publication_checkpoint_digest(
    transaction: &mut Transaction<'_, Postgres>,
    store_scope_id: &StoreScopeId,
    store_epoch: StoreEpoch,
    tenant_scope_id: &TenantScopeId,
    indexed_order: u64,
) -> Result<Option<ContentDigest>, StructuredStoreError> {
    let row = sqlx::query(
        "SELECT store_scope_id, store_epoch::text AS store_epoch, tenant_scope_id, \
                fact_order::text AS fact_order, run_id, run_sequence::text AS run_sequence, \
                transition_ordinal, transition_record_hash \
           FROM tenant_fact_publications \
          WHERE store_scope_id = $1 AND store_epoch = $2::numeric \
            AND tenant_scope_id = $3 \
          ORDER BY fact_order DESC LIMIT 1",
    )
    .bind(store_scope_id.as_str())
    .bind(store_epoch.get().to_string())
    .bind(tenant_scope_id.as_str())
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|_| StructuredStoreError::BackendUnavailable)?;
    let Some(row) = row else {
        if indexed_order == 0 {
            return Ok(None);
        }
        return Err(invalid(
            "structured PostgreSQL tenant fact head has no latest publication",
        ));
    };
    let publication =
        decode_tenant_publication(&row, store_scope_id, store_epoch, tenant_scope_id)?;
    if publication.frontier.fact_order != indexed_order {
        return Err(invalid(
            "structured PostgreSQL tenant fact head differs from latest publication",
        ));
    }
    tenant_publication_checkpoint_digest(&publication).map(Some)
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

    async fn indexed_run_checkpoint_digest(
        &self,
        run_id: &RunId,
    ) -> Result<Option<ContentDigest>, StructuredStoreError> {
        let fixation = self
            .target
            .checkpoint()
            .fixate_read(&CheckpointKey {
                store_scope_id: self.identity.store_scope_id.clone(),
                store_epoch: self.identity.store_epoch,
                target_key: self.target.target_key().as_str().to_owned(),
                stream: CheckpointStream::Run,
                stream_id: run_id.as_str().to_owned(),
                predecessor: None,
            })
            .map_err(|_| StructuredStoreError::StaleHead)?;
        let fixation_guard = ReadFixationGuard::new(self.target.checkpoint().as_ref(), &fixation);
        let mut transaction = self.begin_read().await?;
        let row = sqlx::query(
            "SELECT head_sequence::text AS head_sequence, head_commit_digest \
               FROM run_history_heads WHERE run_id = $1",
        )
        .bind(run_id.as_str())
        .fetch_optional(&mut **transaction.conn())
        .await
        .map_err(|_| StructuredStoreError::BackendUnavailable)?;
        let head = row
            .map(|row| {
                Ok(JournalHead {
                    run_sequence: required_sequence(&row, "head_sequence")?,
                    commit_digest: JournalCommitDigest::parse(&required_text(
                        &row,
                        "head_commit_digest",
                    )?)
                    .map_err(|_| invalid("structured PostgreSQL checkpoint head is invalid"))?,
                })
            })
            .transpose()?;
        transaction.validate_target(&self.target).await?;
        let indexed_digest = head.as_ref().map(canonical_checkpoint_digest).transpose()?;
        if indexed_digest != fixation.successor().cloned() {
            return Err(StructuredStoreError::StaleHead);
        }
        fixation_guard
            .release()
            .map_err(|_| StructuredStoreError::StaleHead)?;
        transaction.commit_checked(&self.target).await?;
        Ok(indexed_digest)
    }
}

impl StructuredHistoryBackend for PostgresStructuredHistoryBackend {
    fn identity(&self) -> &StructuredStoreIdentity {
        &self.identity
    }

    fn load<'a>(&'a self, run_id: &'a RunId) -> StructuredBackendFuture<'a, Option<RawRunHistory>> {
        Box::pin(async move {
            let fixation = self
                .target
                .checkpoint()
                .fixate_read(&CheckpointKey {
                    store_scope_id: self.identity.store_scope_id.clone(),
                    store_epoch: self.identity.store_epoch,
                    target_key: self.target.target_key().as_str().to_owned(),
                    stream: CheckpointStream::Run,
                    stream_id: run_id.as_str().to_owned(),
                    predecessor: None,
                })
                .map_err(|_| StructuredStoreError::StaleHead)?;
            let fixation_guard =
                ReadFixationGuard::new(self.target.checkpoint().as_ref(), &fixation);
            let mut transaction = self.begin_read().await?;
            let head_row = sqlx::query(
                "SELECT head_sequence::text AS head_sequence, head_commit_digest \
                   FROM run_history_heads WHERE run_id = $1",
            )
            .bind(run_id.as_str())
            .fetch_optional(&mut **transaction.conn())
            .await
            .map_err(|_| StructuredStoreError::BackendUnavailable)?;
            let indexed_head = head_row
                .map(|row| {
                    Ok(JournalHead {
                        run_sequence: required_sequence(&row, "head_sequence")?,
                        commit_digest: JournalCommitDigest::parse(&required_text(
                            &row,
                            "head_commit_digest",
                        )?)
                        .map_err(|_| {
                            invalid("structured PostgreSQL load head digest is invalid")
                        })?,
                    })
                })
                .transpose()?;
            let indexed_digest = indexed_head
                .as_ref()
                .map(canonical_checkpoint_digest)
                .transpose()?;
            if indexed_digest != fixation.successor().cloned() {
                return Err(invalid(
                    "structured PostgreSQL load head differs from external checkpoint",
                ));
            }
            fixation_guard
                .release()
                .map_err(|_| StructuredStoreError::StaleHead)?;
            transaction.validate_target(&self.target).await?;
            let rows = select_batch_rows(
                transaction.conn(),
                StructuredBatchQuery::Prefix,
                run_id,
                None,
            )
            .await?;
            if rows.is_empty() {
                if indexed_head.is_some() {
                    return Err(invalid(
                        "structured PostgreSQL indexed head has no retained prefix",
                    ));
                }
                transaction.commit_checked(&self.target).await?;
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
            if batches.last().map(|batch| batch.head.clone()) != indexed_head {
                return Err(invalid(
                    "structured PostgreSQL retained prefix differs from indexed head",
                ));
            }
            transaction.commit_checked(&self.target).await?;
            Ok(Some(RawRunHistory {
                run_id: run_id.clone(),
                batches,
            }))
        })
    }

    fn load_snapshot<'a>(
        &'a self,
        run_id: &'a RunId,
    ) -> StructuredBackendFuture<'a, mfm_store::structured::StructuredRunSnapshot> {
        Box::pin(async move {
            let fixation = self
                .target
                .checkpoint()
                .fixate_read(&CheckpointKey {
                    store_scope_id: self.identity.store_scope_id.clone(),
                    store_epoch: self.identity.store_epoch,
                    target_key: self.target.target_key().as_str().to_owned(),
                    stream: CheckpointStream::Run,
                    stream_id: run_id.as_str().to_owned(),
                    predecessor: None,
                })
                .map_err(|_| StructuredStoreError::StaleHead)?;
            let fixation_guard =
                ReadFixationGuard::new(self.target.checkpoint().as_ref(), &fixation);
            let mut transaction = self.begin_read().await?;
            // The indexed head is deliberately the first decision-bearing query. All subsequent
            // batch/object reads belong to this same repeatable snapshot.
            let head_row = sqlx::query(
                "SELECT head_sequence::text AS head_sequence, head_commit_digest \
                   FROM run_history_heads WHERE run_id = $1",
            )
            .bind(run_id.as_str())
            .fetch_optional(&mut **transaction.conn())
            .await
            .map_err(|_| StructuredStoreError::BackendUnavailable)?;
            let head = head_row
                .map(|row| {
                    Ok(JournalHead {
                        run_sequence: required_sequence(&row, "head_sequence")?,
                        commit_digest: JournalCommitDigest::parse(&required_text(
                            &row,
                            "head_commit_digest",
                        )?)
                        .map_err(|_| {
                            invalid("structured PostgreSQL snapshot head digest is invalid")
                        })?,
                    })
                })
                .transpose()?;
            let head_digest = head.as_ref().map(canonical_checkpoint_digest).transpose()?;
            if head_digest != fixation.successor().cloned() {
                return Err(invalid(
                    "structured PostgreSQL snapshot head differs from external checkpoint",
                ));
            }
            fixation_guard
                .release()
                .map_err(|_| StructuredStoreError::StaleHead)?;
            transaction.validate_target(&self.target).await?;
            let rows = select_batch_rows(
                transaction.conn(),
                StructuredBatchQuery::Prefix,
                run_id,
                None,
            )
            .await?;
            let history = if rows.is_empty() {
                None
            } else {
                let mut objects = load_object_rows(transaction.conn(), run_id, None).await?;
                let mut batches = Vec::with_capacity(rows.len());
                for row in rows {
                    let sequence = row.run_sequence;
                    let object_rows = objects.remove(&sequence).unwrap_or_default();
                    batches.push(row.reconstruct(object_rows)?);
                }
                if !objects.is_empty() {
                    return Err(invalid(
                        "structured PostgreSQL snapshot objects have no retained batch",
                    ));
                }
                let loaded_head = batches.last().map(|batch| batch.head.clone());
                if loaded_head != head {
                    return Err(invalid(
                        "structured PostgreSQL snapshot head differs from retained prefix",
                    ));
                }
                Some(RawRunHistory {
                    run_id: run_id.clone(),
                    batches,
                })
            };
            if history.is_none() && head.is_some() {
                return Err(invalid(
                    "structured PostgreSQL snapshot head has no retained prefix",
                ));
            }
            transaction.commit_checked(&self.target).await?;
            Ok(mfm_store::structured::StructuredRunSnapshot { history, head })
        })
    }

    fn current_head<'a>(
        &'a self,
        run_id: &'a RunId,
    ) -> StructuredBackendFuture<'a, Option<JournalHead>> {
        Box::pin(async move {
            let fixation = self
                .target
                .checkpoint()
                .fixate_read(&CheckpointKey {
                    store_scope_id: self.identity.store_scope_id.clone(),
                    store_epoch: self.identity.store_epoch,
                    target_key: self.target.target_key().as_str().to_owned(),
                    stream: CheckpointStream::Run,
                    stream_id: run_id.as_str().to_owned(),
                    predecessor: None,
                })
                .map_err(|_| StructuredStoreError::StaleHead)?;
            let fixation_guard =
                ReadFixationGuard::new(self.target.checkpoint().as_ref(), &fixation);
            let mut transaction = self.begin_read().await?;
            let row = sqlx::query(
                "SELECT head_sequence::text AS head_sequence, head_commit_digest \
                   FROM run_history_heads WHERE run_id = $1",
            )
            .bind(run_id.as_str())
            .fetch_optional(&mut **transaction.conn())
            .await
            .map_err(|_| StructuredStoreError::BackendUnavailable)?;
            let head = row
                .map(|row| {
                    Ok(JournalHead {
                        run_sequence: required_sequence(&row, "head_sequence")?,
                        commit_digest: JournalCommitDigest::parse(&required_text(
                            &row,
                            "head_commit_digest",
                        )?)
                        .map_err(|_| invalid("structured PostgreSQL head digest is invalid"))?,
                    })
                })
                .transpose()?;
            let digest = head.as_ref().map(canonical_checkpoint_digest).transpose()?;
            if digest != fixation.successor().cloned() {
                return Err(invalid(
                    "structured PostgreSQL current head differs from external checkpoint",
                ));
            }
            fixation_guard
                .release()
                .map_err(|_| StructuredStoreError::StaleHead)?;
            transaction.validate_target(&self.target).await?;
            transaction.commit_checked(&self.target).await?;
            Ok(head)
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
            let fixation = self
                .target
                .checkpoint()
                .fixate_read(&CheckpointKey {
                    store_scope_id: self.identity.store_scope_id.clone(),
                    store_epoch: self.identity.store_epoch,
                    target_key: self.target.target_key().as_str().to_owned(),
                    stream: CheckpointStream::Run,
                    stream_id: run_id.as_str().to_owned(),
                    predecessor: None,
                })
                .map_err(|_| StructuredStoreError::StaleHead)?;
            let fixation_guard =
                ReadFixationGuard::new(self.target.checkpoint().as_ref(), &fixation);
            let mut transaction = self.begin_read().await?;
            // Establish the repeatable-read snapshot with the indexed head before
            // reading the requested prefix or its objects.
            let head_row = sqlx::query(
                "SELECT head_sequence::text AS head_sequence, head_commit_digest \
                   FROM run_history_heads WHERE run_id = $1",
            )
            .bind(run_id.as_str())
            .fetch_optional(&mut **transaction.conn())
            .await
            .map_err(|_| StructuredStoreError::BackendUnavailable)?;
            let indexed_head = head_row
                .map(|row| {
                    Ok(JournalHead {
                        run_sequence: required_sequence(&row, "head_sequence")?,
                        commit_digest: JournalCommitDigest::parse(&required_text(
                            &row,
                            "head_commit_digest",
                        )?)
                        .map_err(|_| {
                            invalid("structured PostgreSQL prefix head digest is invalid")
                        })?,
                    })
                })
                .transpose()?;
            let indexed_digest = indexed_head
                .as_ref()
                .map(canonical_checkpoint_digest)
                .transpose()?;
            if indexed_digest != fixation.successor().cloned() {
                return Err(invalid(
                    "structured PostgreSQL prefix head differs from external checkpoint",
                ));
            }
            fixation_guard
                .release()
                .map_err(|_| StructuredStoreError::StaleHead)?;
            transaction.validate_target(&self.target).await?;
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
                if indexed_head.is_none() && fixation.successor().is_some() {
                    return Err(invalid(
                        "structured PostgreSQL prefix has no rows for an admitted head",
                    ));
                }
                transaction.commit_checked(&self.target).await?;
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
            let loaded_head = batches
                .last()
                .map(|batch| batch.head.clone())
                .ok_or_else(|| invalid("structured PostgreSQL prefix has no retained batch"))?;
            if batches
                .first()
                .is_none_or(|batch| batch.head.run_sequence != 1)
                || batches.windows(2).any(|pair| {
                    pair[1].head.run_sequence != pair[0].head.run_sequence.saturating_add(1)
                })
            {
                return Err(invalid(
                    "structured PostgreSQL prefix is not a dense retained sequence",
                ));
            }
            let expected_sequence = through_sequence.min(
                indexed_head
                    .as_ref()
                    .map_or(through_sequence, |head| head.run_sequence),
            );
            if loaded_head.run_sequence != expected_sequence
                || (through_sequence >= indexed_head.as_ref().map_or(0, |head| head.run_sequence)
                    && Some(loaded_head.clone()) != indexed_head)
            {
                return Err(invalid(
                    "structured PostgreSQL prefix differs from indexed head",
                ));
            }
            transaction.commit_checked(&self.target).await?;
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
            let fixation = self
                .target
                .checkpoint()
                .fixate_read(&CheckpointKey {
                    store_scope_id: self.identity.store_scope_id.clone(),
                    store_epoch: self.identity.store_epoch,
                    target_key: self.target.target_key().as_str().to_owned(),
                    stream: CheckpointStream::TenantFacts,
                    stream_id: tenant_scope_id.as_str().to_owned(),
                    predecessor: None,
                })
                .map_err(|_| StructuredStoreError::StaleHead)?;
            let fixation_guard =
                ReadFixationGuard::new(self.target.checkpoint().as_ref(), &fixation);
            let mut transaction = self.begin_read().await?;
            let rows = sqlx::query(
                "SELECT store_scope_id, store_epoch::text AS store_epoch, \
                        tenant_scope_id, fact_order::text AS fact_order, \
                        publication_count::text AS publication_count, \
                        minimum_order::text AS minimum_order, \
                        maximum_order::text AS maximum_order \
                   FROM tenant_fact_heads WHERE tenant_scope_id = $1",
            )
            .bind(tenant_scope_id.as_str())
            .fetch_all(&mut **transaction.conn())
            .await
            .map_err(|_| StructuredStoreError::BackendUnavailable)?;
            let fact_order = match rows.as_slice() {
                [] => 0,
                [row]
                    if required_text(row, "store_scope_id")?
                        == self.identity.store_scope_id.as_str()
                        && required_text(row, "store_epoch")?
                            == self.identity.store_epoch.get().to_string()
                        && required_text(row, "tenant_scope_id")? == tenant_scope_id.as_str() =>
                {
                    let order = parse_fact_order(&required_text(row, "fact_order")?, true)?;
                    let summary = TenantFactRouteSummary {
                        publication_count: parse_fact_order(
                            &required_text(row, "publication_count")?,
                            true,
                        )?,
                        minimum_order: optional_text(row, "minimum_order")?
                            .map(|value| parse_fact_order(&value, false))
                            .transpose()?,
                        maximum_order: optional_text(row, "maximum_order")?
                            .map(|value| parse_fact_order(&value, false))
                            .transpose()?,
                    };
                    if !summary.is_dense_through(order) {
                        return Err(invalid(
                            "structured PostgreSQL tenant fact head differs from dense routes",
                        ));
                    }
                    order
                }
                _ => {
                    return Err(invalid(
                        "structured PostgreSQL tenant fact head changed store identity",
                    ));
                }
            };
            transaction.validate_target(&self.target).await?;
            fixation_guard
                .release()
                .map_err(|_| StructuredStoreError::StaleHead)?;
            let indexed_successor = load_tenant_publication_checkpoint_digest(
                transaction.conn(),
                &self.identity.store_scope_id,
                self.identity.store_epoch,
                tenant_scope_id,
                fact_order,
            )
            .await?;
            if indexed_successor != fixation.successor().cloned() {
                return Err(invalid(
                    "structured PostgreSQL tenant fact head differs from external checkpoint",
                ));
            }
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
            let fixation = self
                .target
                .checkpoint()
                .fixate_read(&CheckpointKey {
                    store_scope_id: self.identity.store_scope_id.clone(),
                    store_epoch: self.target.store_epoch(),
                    target_key: self.target.target_key().as_str().to_owned(),
                    stream: CheckpointStream::TenantFacts,
                    stream_id: tenant_scope_id.as_str().to_owned(),
                    predecessor: None,
                })
                .map_err(|_| StructuredStoreError::StaleHead)?;
            let fixation_guard =
                ReadFixationGuard::new(self.target.checkpoint().as_ref(), &fixation);
            let mut transaction = self.begin_read().await?;
            let indexed_head = sqlx::query(
                "SELECT store_scope_id, store_epoch::text AS store_epoch, tenant_scope_id, \
                        fact_order::text AS fact_order, publication_count::text AS publication_count, \
                        minimum_order::text AS minimum_order, maximum_order::text AS maximum_order \
                   FROM tenant_fact_heads WHERE tenant_scope_id = $1",
            )
            .bind(tenant_scope_id.as_str())
            .fetch_optional(&mut **transaction.conn())
            .await
            .map_err(|_| StructuredStoreError::BackendUnavailable)?;
            transaction.validate_target(&self.target).await?;
            let indexed_order = indexed_head
                .as_ref()
                .map(|row| {
                    if required_text(row, "store_scope_id")?
                        != self.identity.store_scope_id.as_str()
                        || required_text(row, "store_epoch")?
                            != self.identity.store_epoch.get().to_string()
                        || required_text(row, "tenant_scope_id")? != tenant_scope_id.as_str()
                    {
                        return Err(invalid(
                            "structured PostgreSQL tenant fact head changed store identity",
                        ));
                    }
                    let order = parse_fact_order(&required_text(row, "fact_order")?, true)?;
                    let summary = TenantFactRouteSummary {
                        publication_count: parse_fact_order(
                            &required_text(row, "publication_count")?,
                            true,
                        )?,
                        minimum_order: optional_text(row, "minimum_order")?
                            .map(|value| parse_fact_order(&value, false))
                            .transpose()?,
                        maximum_order: optional_text(row, "maximum_order")?
                            .map(|value| parse_fact_order(&value, false))
                            .transpose()?,
                    };
                    if !summary.is_dense_through(order) {
                        return Err(invalid(
                            "structured PostgreSQL tenant fact head differs from dense routes",
                        ));
                    }
                    Ok(order)
                })
                .transpose()?;
            let Some(indexed_order) = indexed_order else {
                if fixation.successor().is_some() {
                    return Err(invalid(
                        "structured PostgreSQL tenant publication head differs from external checkpoint",
                    ));
                }
                fixation_guard
                    .release()
                    .map_err(|_| StructuredStoreError::StaleHead)?;
                let rows = Vec::new();
                transaction.commit_checked(&self.target).await?;
                return Ok(rows);
            };
            fixation_guard
                .release()
                .map_err(|_| StructuredStoreError::StaleHead)?;
            let indexed_successor = load_tenant_publication_checkpoint_digest(
                transaction.conn(),
                &self.identity.store_scope_id,
                self.identity.store_epoch,
                tenant_scope_id,
                indexed_order,
            )
            .await?;
            if indexed_successor != fixation.successor().cloned() {
                return Err(invalid(
                    "structured PostgreSQL tenant publication head differs from external checkpoint",
                ));
            }
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
        batch: CanonicalRunAppend,
    ) -> StructuredBackendFuture<'a, BackendAppendOutcome> {
        Box::pin(async move {
            if !batch.is_store_verified() {
                return Err(StructuredStoreError::InvalidHistory);
            }
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
            let envelope = StoredBatchEnvelope::from_batch(&committed)?;
            let canonical_envelope = canonical_json(&envelope)
                .map_err(|_| invalid("validated PostgreSQL batch envelope is not canonical"))?;
            validate_envelope_frame(canonical_envelope.as_str())?;

            let tenant_lock_key = match &committed.tenant_fact_coordinate {
                TenantFactCoordinate::None => None,
                TenantFactCoordinate::FactPublication { frontier }
                | TenantFactCoordinate::FactSelectionBarrier { frontier } => {
                    Some(frontier.tenant_scope_id.as_str())
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
                    // Reuse the same exact-key reconstruction used after a
                    // contention rollback. It acknowledges the run and, for
                    // a fact publication, its tenant successor as one set.
                    self.classify_existing_after_contention(&run_id, &committed)
                        .await
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
                    sqlx::query(
                        "INSERT INTO tenant_fact_heads ( \
                            store_scope_id, store_epoch, tenant_scope_id, fact_order, \
                            publication_count, minimum_order, maximum_order \
                         ) VALUES ($1, $2::numeric, $3, 0, 0, NULL, NULL) \
                         ON CONFLICT DO NOTHING",
                    )
                    .bind(self.identity.store_scope_id.as_str())
                    .bind(self.identity.store_epoch.get().to_string())
                    .bind(frontier.tenant_scope_id.as_str())
                    .execute(&mut **transaction.conn())
                    .await
                    .map_err(|_| StructuredStoreError::BackendUnavailable)?;
                    let rows = sqlx::query(
                        "SELECT store_scope_id, store_epoch::text AS store_epoch, \
                                tenant_scope_id, fact_order::text AS fact_order, \
                                publication_count::text AS publication_count, \
                                minimum_order::text AS minimum_order, \
                                maximum_order::text AS maximum_order \
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
                    let route_summary = TenantFactRouteSummary {
                        publication_count: parse_fact_order(
                            &required_text(head, "publication_count")?,
                            true,
                        )?,
                        minimum_order: optional_text(head, "minimum_order")?
                            .map(|value| parse_fact_order(&value, false))
                            .transpose()?,
                        maximum_order: optional_text(head, "maximum_order")?
                            .map(|value| parse_fact_order(&value, false))
                            .transpose()?,
                    };
                    if !route_summary.is_dense_through(current_order) {
                        return Err(invalid(
                            "structured PostgreSQL tenant fact head differs from dense routes",
                        ));
                    }
                    // Indexed probe: reject retained publications beyond the locked head without
                    // scanning lifetime aggregates.
                    let ahead = sqlx::query_scalar::<_, bool>(
                        "SELECT EXISTS ( \
                             SELECT 1 FROM tenant_fact_publications \
                              WHERE store_scope_id = $1 AND store_epoch = $2::numeric \
                                AND tenant_scope_id = $3 AND fact_order > $4::numeric \
                         )",
                    )
                    .bind(self.identity.store_scope_id.as_str())
                    .bind(self.identity.store_epoch.get().to_string())
                    .bind(frontier.tenant_scope_id.as_str())
                    .bind(current_order.to_string())
                    .fetch_one(&mut **transaction.conn())
                    .await
                    .map_err(|_| StructuredStoreError::BackendUnavailable)?;
                    if ahead {
                        return Err(invalid(
                            "structured PostgreSQL tenant fact head lags retained publications",
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
            let successor_bytes = canonical_json(&committed.head)
                .map_err(|_| {
                    invalid("structured PostgreSQL checkpoint successor is not canonical")
                })?
                .as_bytes()
                .to_vec();
            let run_base = CheckpointKey {
                store_scope_id: self.identity.store_scope_id.clone(),
                store_epoch: self.identity.store_epoch,
                target_key: self.target.target_key().as_str().to_owned(),
                stream: CheckpointStream::Run,
                stream_id: run_id.as_str().to_owned(),
                predecessor: None,
            };
            let expected_run_predecessor = committed
                .predecessor
                .as_ref()
                .map(canonical_checkpoint_digest)
                .transpose()?;
            if self.target.checkpoint().current_head(&run_base) != expected_run_predecessor {
                transaction.rollback().await?;
                return Ok(BackendAppendOutcome::StaleHead);
            }
            let run_checkpoint = CheckpointKey {
                predecessor: expected_run_predecessor,
                ..run_base
            };
            let mut checkpoint_mutations = vec![checkpoint_mutation(
                &self.target,
                run_checkpoint,
                successor_bytes.clone(),
            )];
            if let Some(publication) = pending_tenant_publication.as_ref() {
                let tenant_successor = canonical_json(&TenantFactCheckpointSuccessor {
                    frontier: publication.frontier.clone(),
                    transition_ref: publication.transition_ref.clone(),
                    predecessor_order: publication.predecessor_order,
                })
                .map_err(|_| invalid("structured PostgreSQL tenant checkpoint is not canonical"))?
                .as_bytes()
                .to_vec();
                let tenant_key_base = CheckpointKey {
                    store_scope_id: self.identity.store_scope_id.clone(),
                    store_epoch: self.identity.store_epoch,
                    target_key: self.target.target_key().as_str().to_owned(),
                    stream: CheckpointStream::TenantFacts,
                    stream_id: publication.frontier.tenant_scope_id.as_str().to_owned(),
                    predecessor: None,
                };
                let indexed_tenant_predecessor = load_tenant_publication_checkpoint_digest(
                    transaction.conn(),
                    &self.identity.store_scope_id,
                    self.identity.store_epoch,
                    &publication.frontier.tenant_scope_id,
                    publication.predecessor_order,
                )
                .await?;
                let external_tenant_predecessor =
                    self.target.checkpoint().current_head(&tenant_key_base);
                if external_tenant_predecessor != indexed_tenant_predecessor {
                    transaction.rollback().await?;
                    return Ok(BackendAppendOutcome::StaleHead);
                }
                checkpoint_mutations.push(checkpoint_mutation(
                    &self.target,
                    CheckpointKey {
                        predecessor: indexed_tenant_predecessor,
                        ..tenant_key_base
                    },
                    tenant_successor,
                ));
            }
            checkpoint_mutations.sort_by(|left, right| left.key.cmp(&right.key));
            transaction.validate_target(&self.target).await?;
            self.target
                .checkpoint()
                .prepare_many(&checkpoint_mutations)
                .map_err(|_| StructuredStoreError::StaleHead)?;
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
                    // Every failed SQL transaction is rolled back before classification. A
                    // connection in the aborted state cannot safely issue the deciding query.
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
                match sqlx::query(
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
                // Under the run advisory lock this is a lost CAS race, not unavailability.
                transaction.rollback().await?;
                return Ok(BackendAppendOutcome::StaleHead);
            }
            if let Some(publication) = pending_tenant_publication {
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
                    i32::try_from(publication.transition_ref.ordinal).map_err(|_| {
                        invalid("structured PostgreSQL publication ordinal exceeds i32")
                    })?,
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
                let advanced = sqlx::query(
                    "UPDATE tenant_fact_heads \
                        SET fact_order = $4::numeric, \
                            publication_count = $4::numeric, \
                            minimum_order = 1, \
                            maximum_order = $4::numeric \
                      WHERE store_scope_id = $1 AND store_epoch = $2::numeric \
                        AND tenant_scope_id = $3 AND fact_order = $5::numeric \
                        AND publication_count = $5::numeric",
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
            match transaction.commit_outcome(&self.target).await? {
                crate::transaction::CommitOutcome::Committed => {
                    if self
                        .target
                        .checkpoint()
                        .acknowledge_many(&checkpoint_mutations)
                        .is_err()
                    {
                        Ok(BackendAppendOutcome::AcknowledgementUnknown)
                    } else {
                        Ok(BackendAppendOutcome::NewlyCommitted(committed))
                    }
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
            let fixation = self
                .target
                .checkpoint()
                .fixate_read(&CheckpointKey {
                    store_scope_id: self.identity.store_scope_id.clone(),
                    store_epoch: self.identity.store_epoch,
                    target_key: self.target.target_key().as_str().to_owned(),
                    stream: CheckpointStream::Run,
                    stream_id: run_id.as_str().to_owned(),
                    predecessor: None,
                })
                .map_err(|_| StructuredStoreError::StaleHead)?;
            let fixation_guard =
                ReadFixationGuard::new(self.target.checkpoint().as_ref(), &fixation);
            let mut transaction = self.begin_read().await?;
            let head_row = sqlx::query(
                "SELECT head_sequence::text AS head_sequence, head_commit_digest \
                   FROM run_history_heads WHERE run_id = $1",
            )
            .bind(run_id.as_str())
            .fetch_optional(&mut **transaction.conn())
            .await
            .map_err(|_| StructuredStoreError::BackendUnavailable)?;
            let head = head_row
                .map(|row| {
                    Ok(JournalHead {
                        run_sequence: required_sequence(&row, "head_sequence")?,
                        commit_digest: JournalCommitDigest::parse(&required_text(
                            &row,
                            "head_commit_digest",
                        )?)
                        .map_err(|_| {
                            invalid("structured PostgreSQL resolution head digest is invalid")
                        })?,
                    })
                })
                .transpose()?;
            let head_digest = head.as_ref().map(canonical_checkpoint_digest).transpose()?;
            if head_digest != fixation.successor().cloned() {
                return Err(StructuredStoreError::StaleHead);
            }
            fixation_guard
                .release()
                .map_err(|_| StructuredStoreError::StaleHead)?;
            transaction.validate_target(&self.target).await?;
            let rows = select_batch_rows(
                transaction.conn(),
                StructuredBatchQuery::ByAppendRequest,
                run_id,
                Some(append_request_id.as_str()),
            )
            .await?;
            let Some(row) = exactly_one_or_none(rows)? else {
                transaction.commit_checked(&self.target).await?;
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
            transaction.commit_checked(&self.target).await?;
            self.acknowledge_batch_checkpoints(&batch).await?;
            Ok(Some(batch))
        })
    }
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
    validate_envelope_frame(json)?;
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

fn optional_text(row: &PgRow, column: &str) -> Result<Option<String>, StructuredStoreError> {
    row.try_get(column)
        .map_err(|_| invalid("structured PostgreSQL optional text is invalid"))
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
    async fn checkpoint_mutations_for_batch(
        &self,
        batch: &CommittedBatch,
    ) -> Result<Vec<CheckpointMutation>, StructuredStoreError> {
        let run_base = CheckpointKey {
            store_scope_id: self.identity.store_scope_id.clone(),
            store_epoch: self.identity.store_epoch,
            target_key: self.target.target_key().as_str().to_owned(),
            stream: CheckpointStream::Run,
            stream_id: batch
                .records
                .first()
                .ok_or_else(|| invalid("structured PostgreSQL batch has no record"))?
                .record_ref
                .run_id
                .as_str()
                .to_owned(),
            predecessor: None,
        };
        let run_successor = canonical_json(&batch.head)
            .map_err(|_| invalid("structured PostgreSQL checkpoint successor is not canonical"))?
            .as_bytes()
            .to_vec();
        let checkpoint = self.target.checkpoint();
        let mut mutations = Vec::new();
        let run_digest = checkpoint_digest(&run_successor);
        let expected_run_predecessor = batch
            .predecessor
            .as_ref()
            .map(canonical_checkpoint_digest)
            .transpose()?;
        let current_run_head = checkpoint.current_head(&run_base);
        let mut run_checkpoint_already_advanced = false;
        if current_run_head.as_ref() != Some(&run_digest)
            && current_run_head != expected_run_predecessor
        {
            let run_id = RunId::parse(&run_base.stream_id)
                .map_err(|_| invalid("structured PostgreSQL checkpoint run id is invalid"))?;
            if current_run_head != self.indexed_run_checkpoint_digest(&run_id).await? {
                return Err(StructuredStoreError::StaleHead);
            }
            run_checkpoint_already_advanced = true;
        }
        if current_run_head.as_ref() != Some(&run_digest) && !run_checkpoint_already_advanced {
            mutations.push(checkpoint_mutation(
                &self.target,
                CheckpointKey {
                    predecessor: expected_run_predecessor,
                    ..run_base
                },
                run_successor,
            ));
        }
        if let TenantFactCoordinate::FactPublication { frontier } = &batch.tenant_fact_coordinate {
            let transition_ref = batch
                .records
                .first()
                .ok_or_else(|| invalid("structured PostgreSQL publication has no transition"))?
                .record_ref
                .clone();
            let predecessor_order = frontier
                .fact_order
                .checked_sub(1)
                .ok_or_else(|| invalid("structured PostgreSQL publication order is zero"))?;
            let tenant_base = CheckpointKey {
                store_scope_id: self.identity.store_scope_id.clone(),
                store_epoch: self.identity.store_epoch,
                target_key: self.target.target_key().as_str().to_owned(),
                stream: CheckpointStream::TenantFacts,
                stream_id: frontier.tenant_scope_id.as_str().to_owned(),
                predecessor: None,
            };
            let tenant_successor = canonical_json(&TenantFactCheckpointSuccessor {
                frontier: frontier.clone(),
                transition_ref,
                predecessor_order,
            })
            .map_err(|_| invalid("structured PostgreSQL tenant checkpoint is not canonical"))?
            .as_bytes()
            .to_vec();
            let tenant_digest = checkpoint_digest(&tenant_successor);
            let external_predecessor = checkpoint.current_head(&tenant_base);
            let mut predecessor_transaction = self.begin_read().await?;
            let indexed_predecessor = load_tenant_publication_checkpoint_digest(
                predecessor_transaction.conn(),
                &self.identity.store_scope_id,
                self.identity.store_epoch,
                &frontier.tenant_scope_id,
                predecessor_order,
            )
            .await?;
            let current_order = sqlx::query(
                "SELECT store_scope_id, store_epoch::text AS store_epoch, tenant_scope_id, \
                        fact_order::text AS fact_order \
                   FROM tenant_fact_heads WHERE tenant_scope_id = $1",
            )
            .bind(frontier.tenant_scope_id.as_str())
            .fetch_optional(&mut **predecessor_transaction.conn())
            .await
            .map_err(|_| StructuredStoreError::BackendUnavailable)?
            .map(|row| {
                if required_text(&row, "store_scope_id")? != self.identity.store_scope_id.as_str()
                    || required_text(&row, "store_epoch")?
                        != self.identity.store_epoch.get().to_string()
                    || required_text(&row, "tenant_scope_id")? != frontier.tenant_scope_id.as_str()
                {
                    return Err(invalid(
                        "structured PostgreSQL tenant fact head changed identity",
                    ));
                }
                parse_fact_order(&required_text(&row, "fact_order")?, true)
            })
            .transpose()?
            .unwrap_or(0);
            let indexed_current = load_tenant_publication_checkpoint_digest(
                predecessor_transaction.conn(),
                &self.identity.store_scope_id,
                self.identity.store_epoch,
                &frontier.tenant_scope_id,
                current_order,
            )
            .await?;
            predecessor_transaction
                .validate_target(&self.target)
                .await?;
            predecessor_transaction.commit_checked(&self.target).await?;
            let tenant_checkpoint_already_advanced = external_predecessor != indexed_predecessor
                && external_predecessor == indexed_current;
            if external_predecessor != indexed_predecessor && !tenant_checkpoint_already_advanced {
                return Err(StructuredStoreError::StaleHead);
            }
            if !tenant_checkpoint_already_advanced
                && external_predecessor.as_ref() != Some(&tenant_digest)
            {
                mutations.push(checkpoint_mutation(
                    &self.target,
                    CheckpointKey {
                        predecessor: external_predecessor,
                        ..tenant_base
                    },
                    tenant_successor,
                ));
            }
        }
        mutations.sort_by(|left, right| left.key.cmp(&right.key));
        Ok(mutations)
    }

    async fn acknowledge_batch_checkpoints(
        &self,
        batch: &CommittedBatch,
    ) -> Result<(), StructuredStoreError> {
        let mutations = self.checkpoint_mutations_for_batch(batch).await?;
        if mutations.is_empty() {
            return Ok(());
        }
        self.target
            .checkpoint()
            .prepare_many(&mutations)
            .map_err(|_| StructuredStoreError::AcknowledgementUnknown)?;
        self.target
            .checkpoint()
            .acknowledge_many(&mutations)
            .map_err(|_| StructuredStoreError::AcknowledgementUnknown)
    }

    async fn classify_existing_after_contention(
        &self,
        run_id: &RunId,
        committed: &CommittedBatch,
    ) -> Result<BackendAppendOutcome, StructuredStoreError> {
        let tenant_lock_key = match &committed.tenant_fact_coordinate {
            TenantFactCoordinate::None => None,
            TenantFactCoordinate::FactPublication { frontier }
            | TenantFactCoordinate::FactSelectionBarrier { frontier } => {
                Some(frontier.tenant_scope_id.as_str())
            }
        };
        let mut transaction = self
            .begin_locked_write(run_id.as_str(), tenant_lock_key)
            .await?;
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
                    self.acknowledge_batch_checkpoints(&existing).await?;
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
