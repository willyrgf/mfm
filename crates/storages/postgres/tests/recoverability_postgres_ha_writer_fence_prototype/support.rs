use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{SystemTime, UNIX_EPOCH},
};

use mfm_canonical::sha256_digest_bytes;
use sqlx::{postgres::PgPoolOptions, AssertSqlSafe, PgPool, Postgres, Row, Transaction};
use thiserror::Error;

const AUTHORITY_ID: &str = "recoverability-store";
static SCHEMA_COUNTER: AtomicU64 = AtomicU64::new(0);

type Result<T> = std::result::Result<T, FenceError>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct StoreIdentity {
    scope_id: String,
    epoch: String,
}

impl StoreIdentity {
    pub(super) fn new(scope_id: &str, epoch: &str) -> Self {
        Self {
            scope_id: scope_id.to_owned(),
            epoch: epoch.to_owned(),
        }
    }
}

#[derive(Debug, Error)]
pub(super) enum FenceError {
    #[error("PostgreSQL rejected the prototype operation")]
    Database(#[from] sqlx::Error),
    #[error("the external fence generation changed from {expected} to {actual}")]
    GenerationChanged { expected: i64, actual: i64 },
    #[error("the writer no longer owns the external fence")]
    WriterFenced,
    #[error("a restored store changed its scope or epoch")]
    StoreIdentityMismatch,
    #[error("the candidate lost a retained journal suffix for {run_id}")]
    JournalSuffixRolledBack { run_id: String },
    #[error("the candidate journal head is inconsistent for {run_id}")]
    JournalHeadMismatch { run_id: String },
    #[error("the candidate lost a retained tenant fact head for {tenant_id}")]
    TenantFactHeadRolledBack { tenant_id: String },
    #[error("the candidate tenant fact head is inconsistent for {tenant_id}")]
    TenantFactHeadMismatch { tenant_id: String },
    #[error("the reset store scope has already been used")]
    StoreScopeAlreadyUsed,
    #[error("the reset store epoch has already been used")]
    StoreEpochAlreadyUsed,
    #[error("a destructive reset candidate must be empty")]
    ResetCandidateNotEmpty,
}

#[derive(Clone, Debug)]
pub(super) struct StoreReplica {
    pool: PgPool,
    schema: SchemaName,
}

impl StoreReplica {
    pub(super) async fn identity(&self) -> Result<StoreIdentity> {
        load_store_identity_pool(&self.pool, &self.schema).await
    }

    pub(super) async fn journal_head_sequence(&self, run_id: &str) -> Result<Option<i64>> {
        let sql = format!(
            "SELECT sequence FROM {}.prototype_journal_heads WHERE run_id = $1",
            self.schema.as_str()
        );
        sqlx::query_scalar(audited_sql(sql))
            .bind(run_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(FenceError::from)
    }
}

#[derive(Debug)]
pub(super) struct PromotedWriter {
    fence: FenceAuthority,
    replica: StoreReplica,
    token: WriterToken,
}

impl PromotedWriter {
    pub(super) fn generation(&self) -> i64 {
        self.token.generation
    }

    pub(super) fn replica(&self) -> &StoreReplica {
        &self.replica
    }

    pub(super) async fn append_journal(&self, run_id: &str, payload: &[u8]) -> Result<i64> {
        let mut transaction = self.replica.pool.begin().await?;
        self.fence
            .validate_writer(&mut transaction, &self.replica, &self.token)
            .await?;

        let head_sql = format!(
            "SELECT sequence, record_digest \
             FROM {}.prototype_journal_heads \
             WHERE run_id = $1 \
             FOR UPDATE",
            self.replica.schema.as_str()
        );
        let current = sqlx::query(audited_sql(head_sql))
            .bind(run_id)
            .fetch_optional(&mut *transaction)
            .await?;
        let (sequence, predecessor) = match current {
            Some(row) => {
                let sequence: i64 = row.try_get("sequence")?;
                let digest: Vec<u8> = row.try_get("record_digest")?;
                (sequence + 1, Some(digest))
            }
            None => (1, None),
        };
        let record_digest = linked_digest(b"journal", predecessor.as_deref(), payload);

        let record_sql = format!(
            "INSERT INTO {}.prototype_journal_records \
             (run_id, sequence, predecessor_digest, payload, record_digest) \
             VALUES ($1, $2, $3, $4, $5)",
            self.replica.schema.as_str()
        );
        sqlx::query(audited_sql(record_sql))
            .bind(run_id)
            .bind(sequence)
            .bind(predecessor)
            .bind(payload)
            .bind(&record_digest)
            .execute(&mut *transaction)
            .await?;

        let head_sql = format!(
            "INSERT INTO {}.prototype_journal_heads (run_id, sequence, record_digest) \
             VALUES ($1, $2, $3) \
             ON CONFLICT (run_id) DO UPDATE \
             SET sequence = EXCLUDED.sequence, record_digest = EXCLUDED.record_digest",
            self.replica.schema.as_str()
        );
        sqlx::query(audited_sql(head_sql))
            .bind(run_id)
            .bind(sequence)
            .bind(&record_digest)
            .execute(&mut *transaction)
            .await?;

        self.fence
            .retain_journal_head(
                &mut transaction,
                &self.token.identity,
                run_id,
                sequence,
                &record_digest,
            )
            .await?;
        transaction.commit().await?;
        Ok(sequence)
    }

    pub(super) async fn publish_fact(&self, tenant_id: &str, payload: &[u8]) -> Result<i64> {
        let mut transaction = self.replica.pool.begin().await?;
        self.fence
            .validate_writer(&mut transaction, &self.replica, &self.token)
            .await?;

        let head_sql = format!(
            "SELECT fact_order, record_digest \
             FROM {}.prototype_tenant_fact_heads \
             WHERE tenant_id = $1 \
             FOR UPDATE",
            self.replica.schema.as_str()
        );
        let current = sqlx::query(audited_sql(head_sql))
            .bind(tenant_id)
            .fetch_optional(&mut *transaction)
            .await?;
        let (fact_order, predecessor) = match current {
            Some(row) => {
                let fact_order: i64 = row.try_get("fact_order")?;
                let digest: Vec<u8> = row.try_get("record_digest")?;
                (fact_order + 1, Some(digest))
            }
            None => (1, None),
        };
        let record_digest = linked_digest(b"tenant-fact", predecessor.as_deref(), payload);

        let record_sql = format!(
            "INSERT INTO {}.prototype_fact_publications \
             (tenant_id, fact_order, predecessor_digest, payload, record_digest) \
             VALUES ($1, $2, $3, $4, $5)",
            self.replica.schema.as_str()
        );
        sqlx::query(audited_sql(record_sql))
            .bind(tenant_id)
            .bind(fact_order)
            .bind(predecessor)
            .bind(payload)
            .bind(&record_digest)
            .execute(&mut *transaction)
            .await?;

        let head_sql = format!(
            "INSERT INTO {}.prototype_tenant_fact_heads \
             (tenant_id, fact_order, record_digest) \
             VALUES ($1, $2, $3) \
             ON CONFLICT (tenant_id) DO UPDATE \
             SET fact_order = EXCLUDED.fact_order, record_digest = EXCLUDED.record_digest",
            self.replica.schema.as_str()
        );
        sqlx::query(audited_sql(head_sql))
            .bind(tenant_id)
            .bind(fact_order)
            .bind(&record_digest)
            .execute(&mut *transaction)
            .await?;

        self.fence
            .retain_tenant_fact_head(
                &mut transaction,
                &self.token.identity,
                tenant_id,
                fact_order,
                &record_digest,
            )
            .await?;
        transaction.commit().await?;
        Ok(fact_order)
    }
}

#[derive(Clone, Debug)]
struct WriterToken {
    generation: i64,
    writer_id: String,
    identity: StoreIdentity,
}

#[derive(Clone, Debug)]
struct FenceAuthority {
    schema: SchemaName,
}

impl FenceAuthority {
    async fn validate_writer(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
        replica: &StoreReplica,
        token: &WriterToken,
    ) -> Result<()> {
        let fence = self.load_fence(transaction, FenceLock::Shared).await?;
        let store_identity = load_store_identity_transaction(transaction, &replica.schema).await?;
        if fence.generation != token.generation
            || fence.writer_id != token.writer_id
            || fence.identity != token.identity
            || store_identity != token.identity
        {
            return Err(FenceError::WriterFenced);
        }
        Ok(())
    }

    async fn load_fence(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
        lock: FenceLock,
    ) -> Result<FenceRow> {
        let sql = format!(
            "SELECT generation, writer_id, store_scope_id, store_epoch \
             FROM {}.prototype_fence_authority \
             WHERE authority_id = $1 {}",
            self.schema.as_str(),
            lock.sql()
        );
        let row = sqlx::query(audited_sql(sql))
            .bind(AUTHORITY_ID)
            .fetch_one(&mut **transaction)
            .await?;
        Ok(FenceRow {
            generation: row.try_get("generation")?,
            writer_id: row.try_get("writer_id")?,
            identity: StoreIdentity {
                scope_id: row.try_get("store_scope_id")?,
                epoch: row.try_get("store_epoch")?,
            },
        })
    }

    async fn retain_journal_head(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
        identity: &StoreIdentity,
        run_id: &str,
        sequence: i64,
        digest: &[u8],
    ) -> Result<()> {
        let sql = format!(
            "INSERT INTO {}.prototype_required_journal_heads \
             (authority_id, store_scope_id, store_epoch, run_id, sequence, record_digest) \
             VALUES ($1, $2, $3, $4, $5, $6) \
             ON CONFLICT (authority_id, store_scope_id, store_epoch, run_id) DO UPDATE \
             SET sequence = EXCLUDED.sequence, record_digest = EXCLUDED.record_digest",
            self.schema.as_str()
        );
        sqlx::query(audited_sql(sql))
            .bind(AUTHORITY_ID)
            .bind(&identity.scope_id)
            .bind(&identity.epoch)
            .bind(run_id)
            .bind(sequence)
            .bind(digest)
            .execute(&mut **transaction)
            .await?;
        Ok(())
    }

    async fn retain_tenant_fact_head(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
        identity: &StoreIdentity,
        tenant_id: &str,
        fact_order: i64,
        digest: &[u8],
    ) -> Result<()> {
        let sql = format!(
            "INSERT INTO {}.prototype_required_tenant_fact_heads \
             (authority_id, store_scope_id, store_epoch, tenant_id, fact_order, record_digest) \
             VALUES ($1, $2, $3, $4, $5, $6) \
             ON CONFLICT (authority_id, store_scope_id, store_epoch, tenant_id) DO UPDATE \
             SET fact_order = EXCLUDED.fact_order, record_digest = EXCLUDED.record_digest",
            self.schema.as_str()
        );
        sqlx::query(audited_sql(sql))
            .bind(AUTHORITY_ID)
            .bind(&identity.scope_id)
            .bind(&identity.epoch)
            .bind(tenant_id)
            .bind(fact_order)
            .bind(digest)
            .execute(&mut **transaction)
            .await?;
        Ok(())
    }
}

#[derive(Debug)]
struct FenceRow {
    generation: i64,
    writer_id: String,
    identity: StoreIdentity,
}

#[derive(Clone, Copy)]
enum FenceLock {
    Shared,
    Exclusive,
}

impl FenceLock {
    const fn sql(self) -> &'static str {
        match self {
            Self::Shared => "FOR SHARE",
            Self::Exclusive => "FOR UPDATE",
        }
    }
}

#[derive(Clone, Debug)]
struct SchemaName(String);

impl SchemaName {
    fn unique(label: &str) -> Self {
        let micros = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time must follow unix epoch")
            .as_micros()
            % 1_000_000_000_000;
        let counter = SCHEMA_COUNTER.fetch_add(1, Ordering::Relaxed);
        let value = format!(
            "mfmha_{}_{}_{}_{}",
            std::process::id(),
            micros,
            counter,
            label
        );
        assert!(
            value.len() < 64
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_'),
            "prototype schema names must be safe unquoted PostgreSQL identifiers"
        );
        Self(value)
    }

    fn as_str(&self) -> &str {
        &self.0
    }
}

pub(super) struct TestTopology {
    pool: PgPool,
    fence: FenceAuthority,
    schemas: Arc<Mutex<Vec<SchemaName>>>,
}

impl TestTopology {
    pub(super) async fn new(identity: StoreIdentity) -> Result<(Self, PromotedWriter)> {
        let database_url =
            std::env::var("DATABASE_URL").expect("DATABASE_URL must be set for parity tests");
        let pool = PgPoolOptions::new()
            .max_connections(12)
            .connect(&database_url)
            .await?;
        let schemas = Arc::new(Mutex::new(Vec::new()));
        let authority_schema = SchemaName::unique("authority");
        register_schema(&schemas, authority_schema.clone());
        create_authority_schema(&pool, &authority_schema).await?;

        let topology = Self {
            pool: pool.clone(),
            fence: FenceAuthority {
                schema: authority_schema,
            },
            schemas,
        };
        let primary = topology.empty_store(identity.clone(), "primary").await?;
        topology.bootstrap_authority(&identity, "primary").await?;
        let writer = PromotedWriter {
            fence: topology.fence.clone(),
            replica: primary,
            token: WriterToken {
                generation: 1,
                writer_id: "primary".to_owned(),
                identity,
            },
        };
        Ok((topology, writer))
    }

    pub(super) async fn empty_store(
        &self,
        identity: StoreIdentity,
        label: &str,
    ) -> Result<StoreReplica> {
        let schema = SchemaName::unique(label);
        register_schema(&self.schemas, schema.clone());
        create_store_schema(&self.pool, &schema, &identity).await?;
        Ok(StoreReplica {
            pool: self.pool.clone(),
            schema,
        })
    }

    pub(super) async fn snapshot_clone(
        &self,
        source: &StoreReplica,
        label: &str,
    ) -> Result<StoreReplica> {
        let identity = source.identity().await?;
        let target = self.empty_store(identity, label).await?;
        let mut transaction = self.pool.begin().await?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ")
            .execute(&mut *transaction)
            .await?;
        for table in [
            "prototype_journal_records",
            "prototype_journal_heads",
            "prototype_fact_publications",
            "prototype_tenant_fact_heads",
        ] {
            let sql = format!(
                "INSERT INTO {}.{table} SELECT * FROM {}.{table}",
                target.schema.as_str(),
                source.schema.as_str()
            );
            sqlx::query(audited_sql(sql))
                .execute(&mut *transaction)
                .await?;
        }
        transaction.commit().await?;
        Ok(target)
    }

    pub(super) async fn generation(&self) -> Result<i64> {
        let sql = format!(
            "SELECT generation FROM {}.prototype_fence_authority WHERE authority_id = $1",
            self.fence.schema.as_str()
        );
        sqlx::query_scalar(audited_sql(sql))
            .bind(AUTHORITY_ID)
            .fetch_one(&self.pool)
            .await
            .map_err(FenceError::from)
    }

    pub(super) async fn promote(
        &self,
        candidate: StoreReplica,
        expected_generation: i64,
        writer_id: &str,
    ) -> Result<PromotedWriter> {
        let mut transaction = self.pool.begin().await?;
        let current = self
            .fence
            .load_fence(&mut transaction, FenceLock::Exclusive)
            .await?;
        if current.generation != expected_generation {
            return Err(FenceError::GenerationChanged {
                expected: expected_generation,
                actual: current.generation,
            });
        }

        let candidate_identity =
            load_store_identity_transaction(&mut transaction, &candidate.schema).await?;
        if candidate_identity != current.identity {
            return Err(FenceError::StoreIdentityMismatch);
        }
        self.verify_candidate(&mut transaction, &candidate, &candidate_identity)
            .await?;
        self.capture_candidate_heads(&mut transaction, &candidate, &candidate_identity)
            .await?;

        let next_generation = expected_generation + 1;
        self.update_fence_writer(
            &mut transaction,
            next_generation,
            writer_id,
            &candidate_identity,
        )
        .await?;
        transaction.commit().await?;

        Ok(PromotedWriter {
            fence: self.fence.clone(),
            replica: candidate,
            token: WriterToken {
                generation: next_generation,
                writer_id: writer_id.to_owned(),
                identity: candidate_identity,
            },
        })
    }

    pub(super) async fn destructive_reset(
        &self,
        candidate: StoreReplica,
        expected_generation: i64,
        writer_id: &str,
    ) -> Result<PromotedWriter> {
        let mut transaction = self.pool.begin().await?;
        let current = self
            .fence
            .load_fence(&mut transaction, FenceLock::Exclusive)
            .await?;
        if current.generation != expected_generation {
            return Err(FenceError::GenerationChanged {
                expected: expected_generation,
                actual: current.generation,
            });
        }

        let candidate_identity =
            load_store_identity_transaction(&mut transaction, &candidate.schema).await?;
        self.require_empty_store(&mut transaction, &candidate)
            .await?;
        self.require_unused_identity(&mut transaction, &candidate_identity)
            .await?;

        let scope_sql = format!(
            "INSERT INTO {}.prototype_used_store_scopes (store_scope_id) VALUES ($1)",
            self.fence.schema.as_str()
        );
        sqlx::query(audited_sql(scope_sql))
            .bind(&candidate_identity.scope_id)
            .execute(&mut *transaction)
            .await?;
        let epoch_sql = format!(
            "INSERT INTO {}.prototype_used_store_epochs (store_epoch) VALUES ($1)",
            self.fence.schema.as_str()
        );
        sqlx::query(audited_sql(epoch_sql))
            .bind(&candidate_identity.epoch)
            .execute(&mut *transaction)
            .await?;

        let next_generation = expected_generation + 1;
        self.update_fence_writer(
            &mut transaction,
            next_generation,
            writer_id,
            &candidate_identity,
        )
        .await?;
        transaction.commit().await?;

        Ok(PromotedWriter {
            fence: self.fence.clone(),
            replica: candidate,
            token: WriterToken {
                generation: next_generation,
                writer_id: writer_id.to_owned(),
                identity: candidate_identity,
            },
        })
    }

    async fn bootstrap_authority(&self, identity: &StoreIdentity, writer_id: &str) -> Result<()> {
        let mut transaction = self.pool.begin().await?;
        let fence_sql = format!(
            "INSERT INTO {}.prototype_fence_authority \
             (authority_id, generation, writer_id, store_scope_id, store_epoch) \
             VALUES ($1, 1, $2, $3, $4)",
            self.fence.schema.as_str()
        );
        sqlx::query(audited_sql(fence_sql))
            .bind(AUTHORITY_ID)
            .bind(writer_id)
            .bind(&identity.scope_id)
            .bind(&identity.epoch)
            .execute(&mut *transaction)
            .await?;
        let scope_sql = format!(
            "INSERT INTO {}.prototype_used_store_scopes (store_scope_id) VALUES ($1)",
            self.fence.schema.as_str()
        );
        sqlx::query(audited_sql(scope_sql))
            .bind(&identity.scope_id)
            .execute(&mut *transaction)
            .await?;
        let epoch_sql = format!(
            "INSERT INTO {}.prototype_used_store_epochs (store_epoch) VALUES ($1)",
            self.fence.schema.as_str()
        );
        sqlx::query(audited_sql(epoch_sql))
            .bind(&identity.epoch)
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await?;
        Ok(())
    }

    async fn verify_candidate(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
        candidate: &StoreReplica,
        identity: &StoreIdentity,
    ) -> Result<()> {
        self.verify_candidate_journals(transaction, candidate, identity)
            .await?;
        self.verify_candidate_facts(transaction, candidate, identity)
            .await
    }

    async fn verify_candidate_journals(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
        candidate: &StoreReplica,
        identity: &StoreIdentity,
    ) -> Result<()> {
        let candidate_heads_sql = format!(
            "SELECT run_id, sequence, record_digest \
             FROM {}.prototype_journal_heads ORDER BY run_id",
            candidate.schema.as_str()
        );
        let candidate_heads = sqlx::query(audited_sql(candidate_heads_sql))
            .fetch_all(&mut **transaction)
            .await?;
        for head in candidate_heads {
            let run_id: String = head.try_get("run_id")?;
            let sequence: i64 = head.try_get("sequence")?;
            let digest: Vec<u8> = head.try_get("record_digest")?;
            validate_journal_chain(transaction, candidate, &run_id, sequence, &digest).await?;
        }

        let required_sql = format!(
            "SELECT run_id, sequence, record_digest \
             FROM {}.prototype_required_journal_heads \
             WHERE authority_id = $1 AND store_scope_id = $2 AND store_epoch = $3 \
             ORDER BY run_id",
            self.fence.schema.as_str()
        );
        let required = sqlx::query(audited_sql(required_sql))
            .bind(AUTHORITY_ID)
            .bind(&identity.scope_id)
            .bind(&identity.epoch)
            .fetch_all(&mut **transaction)
            .await?;
        for row in required {
            let run_id: String = row.try_get("run_id")?;
            let required_sequence: i64 = row.try_get("sequence")?;
            let required_digest: Vec<u8> = row.try_get("record_digest")?;
            let head_sql = format!(
                "SELECT sequence FROM {}.prototype_journal_heads WHERE run_id = $1",
                candidate.schema.as_str()
            );
            let candidate_sequence: Option<i64> = sqlx::query_scalar(audited_sql(head_sql))
                .bind(&run_id)
                .fetch_optional(&mut **transaction)
                .await?;
            if candidate_sequence.is_none_or(|sequence| sequence < required_sequence) {
                return Err(FenceError::JournalSuffixRolledBack { run_id });
            }

            let record_sql = format!(
                "SELECT record_digest FROM {}.prototype_journal_records \
                 WHERE run_id = $1 AND sequence = $2",
                candidate.schema.as_str()
            );
            let candidate_digest: Option<Vec<u8>> = sqlx::query_scalar(audited_sql(record_sql))
                .bind(&run_id)
                .bind(required_sequence)
                .fetch_optional(&mut **transaction)
                .await?;
            if candidate_digest.as_deref() != Some(required_digest.as_slice()) {
                return Err(FenceError::JournalSuffixRolledBack { run_id });
            }
        }
        Ok(())
    }

    async fn verify_candidate_facts(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
        candidate: &StoreReplica,
        identity: &StoreIdentity,
    ) -> Result<()> {
        let candidate_heads_sql = format!(
            "SELECT tenant_id, fact_order, record_digest \
             FROM {}.prototype_tenant_fact_heads ORDER BY tenant_id",
            candidate.schema.as_str()
        );
        let candidate_heads = sqlx::query(audited_sql(candidate_heads_sql))
            .fetch_all(&mut **transaction)
            .await?;
        for head in candidate_heads {
            let tenant_id: String = head.try_get("tenant_id")?;
            let fact_order: i64 = head.try_get("fact_order")?;
            let digest: Vec<u8> = head.try_get("record_digest")?;
            validate_fact_chain(transaction, candidate, &tenant_id, fact_order, &digest).await?;
        }

        let required_sql = format!(
            "SELECT tenant_id, fact_order, record_digest \
             FROM {}.prototype_required_tenant_fact_heads \
             WHERE authority_id = $1 AND store_scope_id = $2 AND store_epoch = $3 \
             ORDER BY tenant_id",
            self.fence.schema.as_str()
        );
        let required = sqlx::query(audited_sql(required_sql))
            .bind(AUTHORITY_ID)
            .bind(&identity.scope_id)
            .bind(&identity.epoch)
            .fetch_all(&mut **transaction)
            .await?;
        for row in required {
            let tenant_id: String = row.try_get("tenant_id")?;
            let required_order: i64 = row.try_get("fact_order")?;
            let required_digest: Vec<u8> = row.try_get("record_digest")?;
            let head_sql = format!(
                "SELECT fact_order FROM {}.prototype_tenant_fact_heads WHERE tenant_id = $1",
                candidate.schema.as_str()
            );
            let candidate_order: Option<i64> = sqlx::query_scalar(audited_sql(head_sql))
                .bind(&tenant_id)
                .fetch_optional(&mut **transaction)
                .await?;
            if candidate_order.is_none_or(|order| order < required_order) {
                return Err(FenceError::TenantFactHeadRolledBack { tenant_id });
            }

            let record_sql = format!(
                "SELECT record_digest FROM {}.prototype_fact_publications \
                 WHERE tenant_id = $1 AND fact_order = $2",
                candidate.schema.as_str()
            );
            let candidate_digest: Option<Vec<u8>> = sqlx::query_scalar(audited_sql(record_sql))
                .bind(&tenant_id)
                .bind(required_order)
                .fetch_optional(&mut **transaction)
                .await?;
            if candidate_digest.as_deref() != Some(required_digest.as_slice()) {
                return Err(FenceError::TenantFactHeadRolledBack { tenant_id });
            }
        }
        Ok(())
    }

    async fn capture_candidate_heads(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
        candidate: &StoreReplica,
        identity: &StoreIdentity,
    ) -> Result<()> {
        let journal_sql = format!(
            "SELECT run_id, sequence, record_digest FROM {}.prototype_journal_heads",
            candidate.schema.as_str()
        );
        for row in sqlx::query(audited_sql(journal_sql))
            .fetch_all(&mut **transaction)
            .await?
        {
            let run_id: String = row.try_get("run_id")?;
            let sequence: i64 = row.try_get("sequence")?;
            let digest: Vec<u8> = row.try_get("record_digest")?;
            self.fence
                .retain_journal_head(transaction, identity, &run_id, sequence, &digest)
                .await?;
        }

        let fact_sql = format!(
            "SELECT tenant_id, fact_order, record_digest \
             FROM {}.prototype_tenant_fact_heads",
            candidate.schema.as_str()
        );
        for row in sqlx::query(audited_sql(fact_sql))
            .fetch_all(&mut **transaction)
            .await?
        {
            let tenant_id: String = row.try_get("tenant_id")?;
            let fact_order: i64 = row.try_get("fact_order")?;
            let digest: Vec<u8> = row.try_get("record_digest")?;
            self.fence
                .retain_tenant_fact_head(transaction, identity, &tenant_id, fact_order, &digest)
                .await?;
        }
        Ok(())
    }

    async fn update_fence_writer(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
        generation: i64,
        writer_id: &str,
        identity: &StoreIdentity,
    ) -> Result<()> {
        let sql = format!(
            "UPDATE {}.prototype_fence_authority \
             SET generation = $1, writer_id = $2, store_scope_id = $3, store_epoch = $4 \
             WHERE authority_id = $5",
            self.fence.schema.as_str()
        );
        sqlx::query(audited_sql(sql))
            .bind(generation)
            .bind(writer_id)
            .bind(&identity.scope_id)
            .bind(&identity.epoch)
            .bind(AUTHORITY_ID)
            .execute(&mut **transaction)
            .await?;
        Ok(())
    }

    async fn require_unused_identity(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
        identity: &StoreIdentity,
    ) -> Result<()> {
        let scope_sql = format!(
            "SELECT EXISTS(SELECT 1 FROM {}.prototype_used_store_scopes \
             WHERE store_scope_id = $1)",
            self.fence.schema.as_str()
        );
        let scope_used: bool = sqlx::query_scalar(audited_sql(scope_sql))
            .bind(&identity.scope_id)
            .fetch_one(&mut **transaction)
            .await?;
        if scope_used {
            return Err(FenceError::StoreScopeAlreadyUsed);
        }

        let epoch_sql = format!(
            "SELECT EXISTS(SELECT 1 FROM {}.prototype_used_store_epochs \
             WHERE store_epoch = $1)",
            self.fence.schema.as_str()
        );
        let epoch_used: bool = sqlx::query_scalar(audited_sql(epoch_sql))
            .bind(&identity.epoch)
            .fetch_one(&mut **transaction)
            .await?;
        if epoch_used {
            return Err(FenceError::StoreEpochAlreadyUsed);
        }
        Ok(())
    }

    async fn require_empty_store(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
        candidate: &StoreReplica,
    ) -> Result<()> {
        for table in [
            "prototype_journal_records",
            "prototype_journal_heads",
            "prototype_fact_publications",
            "prototype_tenant_fact_heads",
        ] {
            let sql = format!("SELECT COUNT(*) FROM {}.{table}", candidate.schema.as_str());
            let count: i64 = sqlx::query_scalar(audited_sql(sql))
                .fetch_one(&mut **transaction)
                .await?;
            if count != 0 {
                return Err(FenceError::ResetCandidateNotEmpty);
            }
        }
        Ok(())
    }

    pub(super) async fn cleanup(self) -> Result<()> {
        let schemas = self
            .schemas
            .lock()
            .expect("schema registry mutex must not be poisoned")
            .clone();
        for schema in schemas.into_iter().rev() {
            let sql = format!("DROP SCHEMA IF EXISTS {} CASCADE", schema.as_str());
            sqlx::raw_sql(AssertSqlSafe(sql))
                .execute(&self.pool)
                .await?;
        }
        self.pool.close().await;
        Ok(())
    }
}

async fn create_authority_schema(pool: &PgPool, schema: &SchemaName) -> Result<()> {
    let sql = format!(
        "CREATE SCHEMA {schema};
         CREATE TABLE {schema}.prototype_fence_authority (
           authority_id TEXT PRIMARY KEY,
           generation BIGINT NOT NULL CHECK (generation > 0),
           writer_id TEXT NOT NULL,
           store_scope_id TEXT NOT NULL,
           store_epoch TEXT NOT NULL
         );
         CREATE TABLE {schema}.prototype_used_store_scopes (
           store_scope_id TEXT PRIMARY KEY
         );
         CREATE TABLE {schema}.prototype_used_store_epochs (
           store_epoch TEXT PRIMARY KEY
         );
         CREATE TABLE {schema}.prototype_required_journal_heads (
           authority_id TEXT NOT NULL,
           store_scope_id TEXT NOT NULL,
           store_epoch TEXT NOT NULL,
           run_id TEXT NOT NULL,
           sequence BIGINT NOT NULL CHECK (sequence > 0),
           record_digest BYTEA NOT NULL CHECK (octet_length(record_digest) = 32),
           PRIMARY KEY (authority_id, store_scope_id, store_epoch, run_id)
         );
         CREATE TABLE {schema}.prototype_required_tenant_fact_heads (
           authority_id TEXT NOT NULL,
           store_scope_id TEXT NOT NULL,
           store_epoch TEXT NOT NULL,
           tenant_id TEXT NOT NULL,
           fact_order BIGINT NOT NULL CHECK (fact_order > 0),
           record_digest BYTEA NOT NULL CHECK (octet_length(record_digest) = 32),
           PRIMARY KEY (authority_id, store_scope_id, store_epoch, tenant_id)
         )",
        schema = schema.as_str()
    );
    sqlx::raw_sql(AssertSqlSafe(sql)).execute(pool).await?;
    Ok(())
}

async fn create_store_schema(
    pool: &PgPool,
    schema: &SchemaName,
    identity: &StoreIdentity,
) -> Result<()> {
    let sql = format!(
        "CREATE SCHEMA {schema};
         CREATE TABLE {schema}.prototype_store_identity (
           singleton BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (singleton),
           store_scope_id TEXT NOT NULL,
           store_epoch TEXT NOT NULL
         );
         CREATE TABLE {schema}.prototype_journal_records (
           run_id TEXT NOT NULL,
           sequence BIGINT NOT NULL CHECK (sequence > 0),
           predecessor_digest BYTEA CHECK (predecessor_digest IS NULL OR octet_length(predecessor_digest) = 32),
           payload BYTEA NOT NULL,
           record_digest BYTEA NOT NULL CHECK (octet_length(record_digest) = 32),
           PRIMARY KEY (run_id, sequence),
           CHECK ((sequence = 1) = (predecessor_digest IS NULL))
         );
         CREATE TABLE {schema}.prototype_journal_heads (
           run_id TEXT PRIMARY KEY,
           sequence BIGINT NOT NULL CHECK (sequence > 0),
           record_digest BYTEA NOT NULL CHECK (octet_length(record_digest) = 32)
         );
         CREATE TABLE {schema}.prototype_fact_publications (
           tenant_id TEXT NOT NULL,
           fact_order BIGINT NOT NULL CHECK (fact_order > 0),
           predecessor_digest BYTEA CHECK (predecessor_digest IS NULL OR octet_length(predecessor_digest) = 32),
           payload BYTEA NOT NULL,
           record_digest BYTEA NOT NULL CHECK (octet_length(record_digest) = 32),
           PRIMARY KEY (tenant_id, fact_order),
           CHECK ((fact_order = 1) = (predecessor_digest IS NULL))
         );
         CREATE TABLE {schema}.prototype_tenant_fact_heads (
           tenant_id TEXT PRIMARY KEY,
           fact_order BIGINT NOT NULL CHECK (fact_order > 0),
           record_digest BYTEA NOT NULL CHECK (octet_length(record_digest) = 32)
         )",
        schema = schema.as_str()
    );
    sqlx::raw_sql(AssertSqlSafe(sql)).execute(pool).await?;
    let identity_sql = format!(
        "INSERT INTO {}.prototype_store_identity \
         (singleton, store_scope_id, store_epoch) VALUES (TRUE, $1, $2)",
        schema.as_str()
    );
    sqlx::query(audited_sql(identity_sql))
        .bind(&identity.scope_id)
        .bind(&identity.epoch)
        .execute(pool)
        .await?;
    Ok(())
}

async fn load_store_identity_pool(pool: &PgPool, schema: &SchemaName) -> Result<StoreIdentity> {
    let sql = format!(
        "SELECT store_scope_id, store_epoch FROM {}.prototype_store_identity \
         WHERE singleton = TRUE",
        schema.as_str()
    );
    let row = sqlx::query(audited_sql(sql)).fetch_one(pool).await?;
    Ok(StoreIdentity {
        scope_id: row.try_get("store_scope_id")?,
        epoch: row.try_get("store_epoch")?,
    })
}

async fn load_store_identity_transaction(
    transaction: &mut Transaction<'_, Postgres>,
    schema: &SchemaName,
) -> Result<StoreIdentity> {
    let sql = format!(
        "SELECT store_scope_id, store_epoch FROM {}.prototype_store_identity \
         WHERE singleton = TRUE",
        schema.as_str()
    );
    let row = sqlx::query(audited_sql(sql))
        .fetch_one(&mut **transaction)
        .await?;
    Ok(StoreIdentity {
        scope_id: row.try_get("store_scope_id")?,
        epoch: row.try_get("store_epoch")?,
    })
}

async fn validate_journal_chain(
    transaction: &mut Transaction<'_, Postgres>,
    replica: &StoreReplica,
    run_id: &str,
    head_sequence: i64,
    head_digest: &[u8],
) -> Result<()> {
    let sql = format!(
        "SELECT sequence, predecessor_digest, payload, record_digest \
         FROM {}.prototype_journal_records \
         WHERE run_id = $1 ORDER BY sequence",
        replica.schema.as_str()
    );
    let rows = sqlx::query(audited_sql(sql))
        .bind(run_id)
        .fetch_all(&mut **transaction)
        .await?;
    let mut predecessor: Option<Vec<u8>> = None;
    let mut expected_sequence = 1_i64;
    for row in rows {
        let sequence: i64 = row.try_get("sequence")?;
        let persisted_predecessor: Option<Vec<u8>> = row.try_get("predecessor_digest")?;
        let payload: Vec<u8> = row.try_get("payload")?;
        let digest: Vec<u8> = row.try_get("record_digest")?;
        if sequence != expected_sequence
            || persisted_predecessor.as_deref() != predecessor.as_deref()
            || digest != linked_digest(b"journal", predecessor.as_deref(), &payload)
        {
            return Err(FenceError::JournalHeadMismatch {
                run_id: run_id.to_owned(),
            });
        }
        predecessor = Some(digest);
        expected_sequence += 1;
    }
    if expected_sequence - 1 != head_sequence || predecessor.as_deref() != Some(head_digest) {
        return Err(FenceError::JournalHeadMismatch {
            run_id: run_id.to_owned(),
        });
    }
    Ok(())
}

async fn validate_fact_chain(
    transaction: &mut Transaction<'_, Postgres>,
    replica: &StoreReplica,
    tenant_id: &str,
    head_order: i64,
    head_digest: &[u8],
) -> Result<()> {
    let sql = format!(
        "SELECT fact_order, predecessor_digest, payload, record_digest \
         FROM {}.prototype_fact_publications \
         WHERE tenant_id = $1 ORDER BY fact_order",
        replica.schema.as_str()
    );
    let rows = sqlx::query(audited_sql(sql))
        .bind(tenant_id)
        .fetch_all(&mut **transaction)
        .await?;
    let mut predecessor: Option<Vec<u8>> = None;
    let mut expected_order = 1_i64;
    for row in rows {
        let fact_order: i64 = row.try_get("fact_order")?;
        let persisted_predecessor: Option<Vec<u8>> = row.try_get("predecessor_digest")?;
        let payload: Vec<u8> = row.try_get("payload")?;
        let digest: Vec<u8> = row.try_get("record_digest")?;
        if fact_order != expected_order
            || persisted_predecessor.as_deref() != predecessor.as_deref()
            || digest != linked_digest(b"tenant-fact", predecessor.as_deref(), &payload)
        {
            return Err(FenceError::TenantFactHeadMismatch {
                tenant_id: tenant_id.to_owned(),
            });
        }
        predecessor = Some(digest);
        expected_order += 1;
    }
    if expected_order - 1 != head_order || predecessor.as_deref() != Some(head_digest) {
        return Err(FenceError::TenantFactHeadMismatch {
            tenant_id: tenant_id.to_owned(),
        });
    }
    Ok(())
}

fn linked_digest(domain: &[u8], predecessor: Option<&[u8]>, payload: &[u8]) -> Vec<u8> {
    let mut preimage = Vec::with_capacity(
        domain.len() + predecessor.map_or(1, |value| value.len()) + payload.len() + 18,
    );
    preimage.extend_from_slice(&(domain.len() as u64).to_be_bytes());
    preimage.extend_from_slice(domain);
    match predecessor {
        Some(predecessor) => {
            preimage.push(1);
            preimage.extend_from_slice(predecessor);
        }
        None => preimage.push(0),
    }
    preimage.extend_from_slice(&(payload.len() as u64).to_be_bytes());
    preimage.extend_from_slice(payload);
    sha256_digest_bytes(&preimage).as_bytes().to_vec()
}

// Dynamic fragments contain only generated, validated schema identifiers. All test data remains
// in bind parameters.
fn audited_sql(sql: String) -> AssertSqlSafe<String> {
    AssertSqlSafe(sql)
}

fn register_schema(schemas: &Arc<Mutex<Vec<SchemaName>>>, schema: SchemaName) {
    schemas
        .lock()
        .expect("schema registry mutex must not be poisoned")
        .push(schema);
}
