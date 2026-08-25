#![warn(missing_docs)]
//! PostgreSQL implementations of mechanical persistence and optional EVM transaction authority.
//!
//! Each concrete handle independently verifies only the static schemas and durability prerequisites
//! required by the authority it exposes.

use std::future::Future;
use std::time::Duration;

use mfm_canonical::sha256_digest_bytes;
use mfm_config::{
    ConfigDigest, ConfigFuture, ConfigImportResult, ConfigName, ConfigRepository, ConfigRevision,
};
use mfm_evm::EvmAuthorityEpoch;
use mfm_ids::{ContentDigest, DigestAlgorithm, RunId};
use mfm_journal::{
    frame_head_digest, EncodedRunFrame, StoredRunBytes, MAX_FRAME_BYTES, MAX_RUN_BYTES,
    MAX_RUN_FRAMES,
};
use mfm_store::{AppendResult, RunIndex, RunIndexError, RunPage, RunPageLimit, Store, StoreError};
use sqlx::postgres::{PgArguments, PgPoolOptions, PgRow};
use sqlx::{Arguments, Connection, PgConnection, PgPool, Row};

const SCHEMA_CONTRACT: &str = "mfm.run-history-postgres.v2";

mod catalog;
mod config;
mod evm_tx;
mod index;
mod locator;
mod provision;
pub use locator::{
    AdminPostgresLocator, PostgresLocatorError, RuntimePostgresLocator, MAX_POSTGRES_LOCATOR_BYTES,
};
pub use provision::{provision_evm_transaction_authority, provision_postgres, ProvisionError};

/// Complete PostgreSQL persistence backend after its connection gate succeeds.
pub struct PostgresBackend {
    pool: PgPool,
}

/// Independently gated PostgreSQL EVM transaction-authority handle.
pub struct PostgresEvmTransactionAuthority {
    pool: PgPool,
    authority_epoch: EvmAuthorityEpoch,
    #[cfg(test)]
    authority_commit_fault: std::sync::atomic::AtomicU8,
}

/// Redaction-safe production-open failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PostgresOpenError {
    /// The static schema or durability posture is incompatible.
    #[error("postgres backend is incompatible")]
    Incompatible,
    /// The database could not be observed.
    #[error("postgres backend is unavailable")]
    Unavailable,
}

impl PostgresBackend {
    /// Connects and verifies run-history, configuration, and durability prerequisites.
    pub async fn connect(
        locator: &RuntimePostgresLocator,
    ) -> std::result::Result<Self, PostgresOpenError> {
        assert_send_static::<PgRow>();
        assert_send_static::<PgArguments>();
        let options = locator
            .connect_options("mfm-runtime-postgres")
            .map_err(|_| PostgresOpenError::Unavailable)?;
        let mut gate_connection = PgConnection::connect_with(&options)
            .await
            .map_err(|_| PostgresOpenError::Unavailable)?;
        verify_connection(&mut gate_connection)
            .await
            .map_err(classify_gate_error)?;
        drop(gate_connection);
        let pool = PgPoolOptions::new()
            .acquire_timeout(Duration::from_secs(2))
            .after_connect(|connection, _metadata| {
                Box::pin(async move {
                    verify_connection(connection)
                        .await
                        .map(|_| ())
                        .map_err(|error| sqlx::Error::Protocol(error.marker().to_owned()))
                })
            })
            .connect_with(options)
            .await
            .map_err(classify_open_error)?;
        let mut admitted_connection = pool.acquire().await.map_err(classify_open_error)?;
        verify_connection(&mut admitted_connection)
            .await
            .map_err(classify_gate_error)?;
        drop(admitted_connection);
        Ok(Self { pool })
    }

    #[cfg(test)]
    pub(crate) const fn test_pool(&self) -> &PgPool {
        &self.pool
    }
}

impl PostgresEvmTransactionAuthority {
    /// Connects and verifies only the EVM transaction-authority schema and shared posture.
    pub async fn connect(
        locator: &RuntimePostgresLocator,
    ) -> std::result::Result<Self, PostgresOpenError> {
        let options = locator
            .connect_options("mfm-evm-transaction-authority")
            .map_err(|_| PostgresOpenError::Unavailable)?;
        let mut gate_connection = PgConnection::connect_with(&options)
            .await
            .map_err(|_| PostgresOpenError::Unavailable)?;
        verify_evm_connection(&mut gate_connection)
            .await
            .map_err(classify_gate_error)?;
        drop(gate_connection);
        let pool = PgPoolOptions::new()
            .acquire_timeout(Duration::from_secs(2))
            .after_connect(|connection, _metadata| {
                Box::pin(async move {
                    verify_evm_connection(connection)
                        .await
                        .map(|_| ())
                        .map_err(|error| sqlx::Error::Protocol(error.marker().to_owned()))
                })
            })
            .connect_with(options)
            .await
            .map_err(classify_open_error)?;
        let mut admitted_connection = pool.acquire().await.map_err(classify_open_error)?;
        let authority_epoch = verify_evm_connection(&mut admitted_connection)
            .await
            .map_err(classify_gate_error)?;
        drop(admitted_connection);
        Ok(Self {
            pool,
            authority_epoch,
            #[cfg(test)]
            authority_commit_fault: std::sync::atomic::AtomicU8::new(0),
        })
    }

    #[cfg(test)]
    pub(crate) const fn test_pool(&self) -> &PgPool {
        &self.pool
    }
}

const fn classify_gate_error(error: GateError) -> PostgresOpenError {
    match error {
        GateError::Incompatible => PostgresOpenError::Incompatible,
        GateError::Unavailable => PostgresOpenError::Unavailable,
    }
}

/// Counts every `mfm_`-prefixed relation, indexes included, in the public schema.
impl Store for PostgresBackend {
    fn load_run<'a>(
        &'a self,
        run_id: &'a RunId,
    ) -> std::pin::Pin<
        Box<
            dyn Future<Output = std::result::Result<Option<StoredRunBytes>, StoreError>>
                + Send
                + 'a,
        >,
    > {
        Box::pin(async move { load_run(&self.pool, run_id, LoadProbe::None).await })
    }

    fn append_run<'a>(
        &'a self,
        frame: &'a EncodedRunFrame,
    ) -> std::pin::Pin<
        Box<dyn Future<Output = std::result::Result<AppendResult, StoreError>> + Send + 'a>,
    > {
        Box::pin(async move { append_run(&self.pool, frame, CommitFault::None).await })
    }
}

impl RunIndex for PostgresBackend {
    fn list_runs<'a>(
        &'a self,
        after: Option<&'a RunId>,
        limit: RunPageLimit,
    ) -> std::pin::Pin<Box<dyn Future<Output = Result<RunPage, RunIndexError>> + Send + 'a>> {
        Box::pin(async move { index::list_runs(&self.pool, after, limit).await })
    }
}

impl ConfigRepository for PostgresBackend {
    fn import_config<'a>(
        &'a self,
        revision: &'a ConfigRevision,
    ) -> ConfigFuture<'a, ConfigImportResult> {
        Box::pin(async move {
            config::import_config(&self.pool, revision, config::MutationCommitFault::None).await
        })
    }

    fn load_config<'a>(
        &'a self,
        name: &'a ConfigName,
        digest: &'a ConfigDigest,
    ) -> ConfigFuture<'a, Option<ConfigRevision>> {
        Box::pin(async move { config::load_config(&self.pool, name, digest).await })
    }

    fn list_configs(&self) -> ConfigFuture<'_, Vec<ConfigRevision>> {
        Box::pin(async move { config::list_configs(&self.pool).await })
    }

    fn delete_config<'a>(
        &'a self,
        name: &'a ConfigName,
        digest: &'a ConfigDigest,
    ) -> ConfigFuture<'a, ()> {
        Box::pin(async move {
            config::delete_config(&self.pool, name, digest, config::MutationCommitFault::None).await
        })
    }
}

#[derive(Clone, Copy)]
enum CommitFault {
    None,
    #[cfg(test)]
    BeforeSubmission,
    #[cfg(test)]
    Rejected,
    #[cfg(test)]
    UnknownRolledBack,
    #[cfg(test)]
    UnknownCommitted,
}

enum LoadProbe {
    None,
    #[cfg(test)]
    SnapshotPause {
        entered: std::sync::Arc<tokio::sync::Notify>,
        release: std::sync::Arc<tokio::sync::Notify>,
    },
    #[cfg(test)]
    BlockingPause {
        entered: std::sync::Arc<tokio::sync::Notify>,
        release: std::sync::Arc<std::sync::atomic::AtomicBool>,
    },
}

#[derive(Debug, Clone, Copy)]
enum GateError {
    Incompatible,
    Unavailable,
}

impl GateError {
    const fn marker(self) -> &'static str {
        match self {
            Self::Incompatible => "mfm-postgres-gate:incompatible",
            Self::Unavailable => "mfm-postgres-gate:unavailable",
        }
    }
}

fn classify_open_error(error: sqlx::Error) -> PostgresOpenError {
    match &error {
        sqlx::Error::Protocol(message) if message.contains("mfm-postgres-gate:incompatible") => {
            PostgresOpenError::Incompatible
        }
        _ => PostgresOpenError::Unavailable,
    }
}

async fn verify_durability(connection: &mut PgConnection) -> std::result::Result<(), GateError> {
    let durable: (bool, String, String) = sqlx::query_as(
        "SELECT NOT pg_is_in_recovery(), current_setting('fsync'), \
         current_setting('full_page_writes')",
    )
    .fetch_one(&mut *connection)
    .await
    .map_err(|_| GateError::Unavailable)?;
    if !durability_matches(durable.0, &durable.1, &durable.2) {
        return Err(GateError::Incompatible);
    }
    Ok(())
}

async fn verify_connection(connection: &mut PgConnection) -> std::result::Result<(), GateError> {
    verify_durability(connection).await?;
    verify_runtime_authority(connection).await?;
    catalog::verify_surface(connection, &catalog::RUN_SURFACE, None).await?;
    verify_run_marker(connection).await?;
    catalog::verify_surface(connection, &catalog::CONFIG_SURFACE, None).await?;
    verify_config_marker(connection).await
}

async fn verify_evm_connection(
    connection: &mut PgConnection,
) -> std::result::Result<EvmAuthorityEpoch, GateError> {
    verify_durability(connection).await?;
    verify_runtime_authority(connection).await?;
    catalog::verify_surface(connection, &catalog::EVM_TX_SURFACE, None).await?;
    evm_tx::load_evm_tx_epoch(connection).await
}

async fn verify_run_marker(connection: &mut PgConnection) -> std::result::Result<(), GateError> {
    let markers: Vec<String> =
        sqlx::query_scalar("SELECT schema_contract FROM public.mfm_store_schema ORDER BY 1")
            .fetch_all(&mut *connection)
            .await
            .map_err(classify_catalog_query)?;
    (markers == [SCHEMA_CONTRACT])
        .then_some(())
        .ok_or(GateError::Incompatible)
}

async fn verify_config_marker(connection: &mut PgConnection) -> std::result::Result<(), GateError> {
    let markers: Vec<String> =
        sqlx::query_scalar("SELECT schema_contract FROM mfm_config.mfm_config_schema ORDER BY 1")
            .fetch_all(&mut *connection)
            .await
            .map_err(classify_catalog_query)?;
    (markers == ["mfm.config-postgres.v2"])
        .then_some(())
        .ok_or(GateError::Incompatible)
}

fn classify_catalog_query(error: sqlx::Error) -> GateError {
    if is_undefined_schema_object(&error) {
        GateError::Incompatible
    } else {
        GateError::Unavailable
    }
}

type RuntimeRoleRow = (String, bool, bool, bool, bool, bool, bool, bool);

async fn verify_runtime_authority(
    connection: &mut PgConnection,
) -> std::result::Result<(), GateError> {
    let role: Option<RuntimeRoleRow> = sqlx::query_as(
        "SELECT rolname, rolsuper, rolinherit, rolcreaterole, rolcreatedb, \
                rolcanlogin, rolreplication, rolbypassrls \
         FROM pg_catalog.pg_roles WHERE rolname = current_user",
    )
    .fetch_optional(&mut *connection)
    .await
    .map_err(|_| GateError::Unavailable)?;
    let Some(role) = role else {
        return Err(GateError::Incompatible);
    };
    if role.0 != "mfm_runtime"
        || role.1
        || role.2
        || role.3
        || role.4
        || !role.5
        || role.6
        || role.7
    {
        return Err(GateError::Incompatible);
    }
    let memberships: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_catalog.pg_auth_members m \
         JOIN pg_catalog.pg_roles r ON r.oid = m.member OR r.oid = m.roleid \
         WHERE r.rolname = 'mfm_runtime'",
    )
    .fetch_one(&mut *connection)
    .await
    .map_err(|_| GateError::Unavailable)?;
    if memberships != 0 {
        return Err(GateError::Incompatible);
    }
    let database: (bool, bool, bool) = sqlx::query_as(
        "SELECT has_database_privilege(current_user, current_database(), 'CONNECT'), \
                has_database_privilege(current_user, current_database(), 'CREATE'), \
                has_database_privilege(current_user, current_database(), 'TEMPORARY')",
    )
    .fetch_one(&mut *connection)
    .await
    .map_err(|_| GateError::Unavailable)?;
    if database != (true, false, false) {
        return Err(GateError::Incompatible);
    }
    let owns_objects: bool = sqlx::query_scalar(
        "SELECT EXISTS ( \
           SELECT 1 FROM pg_catalog.pg_database d \
            WHERE d.datname = current_database() AND pg_get_userbyid(d.datdba) = current_user \
           UNION ALL \
           SELECT 1 FROM pg_catalog.pg_namespace n WHERE pg_get_userbyid(n.nspowner) = current_user \
           UNION ALL \
           SELECT 1 FROM pg_catalog.pg_class c \
            JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
            WHERE pg_get_userbyid(c.relowner) = current_user \
              AND n.nspname NOT IN ('pg_catalog', 'information_schema') \
         )",
    )
    .fetch_one(&mut *connection)
    .await
    .map_err(|_| GateError::Unavailable)?;
    if owns_objects {
        return Err(GateError::Incompatible);
    }
    Ok(())
}

fn durability_matches(primary: bool, fsync: &str, full_page_writes: &str) -> bool {
    primary && fsync == "on" && full_page_writes == "on"
}

fn is_undefined_schema_object(error: &sqlx::Error) -> bool {
    error
        .as_database_error()
        .and_then(|error| error.code())
        .is_some_and(|code| matches!(code.as_ref(), "42P01" | "42703"))
}

async fn load_run(
    pool: &PgPool,
    run_id: &RunId,
    probe: LoadProbe,
) -> std::result::Result<Option<StoredRunBytes>, StoreError> {
    #[cfg(not(test))]
    let _ = probe;
    let mut transaction = pool.begin().await.map_err(|_| StoreError::Unavailable)?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
        .execute(&mut *transaction)
        .await
        .map_err(|_| StoreError::Unavailable)?;

    let head = sqlx::query(
        "SELECT h.run_id, h.head_sequence, h.total_bytes, \
                f.frame_bytes, f.head_digest \
         FROM public.mfm_run_heads h \
         LEFT JOIN public.mfm_run_frames f \
           ON f.run_id = h.run_id AND f.run_sequence = h.head_sequence \
         WHERE h.run_id = $1",
    )
    .bind(run_id.as_str())
    .fetch_optional(&mut *transaction)
    .await
    .map_err(|_| StoreError::Unavailable)?;

    let Some(head) = head else {
        let orphan: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM public.mfm_run_frames WHERE run_id = $1)",
        )
        .bind(run_id.as_str())
        .fetch_one(&mut *transaction)
        .await
        .map_err(|_| StoreError::Unavailable)?;
        if orphan {
            let _ = transaction.rollback().await;
            return Err(StoreError::CorruptPhysicalState);
        }
        transaction
            .commit()
            .await
            .map_err(|_| StoreError::Unavailable)?;
        return Ok(None);
    };

    #[cfg(test)]
    if let LoadProbe::SnapshotPause { entered, release } = &probe {
        entered.notify_one();
        release.notified().await;
    }

    let head_sequence: i64 = head
        .try_get("head_sequence")
        .map_err(|_| StoreError::CorruptPhysicalState)?;
    let total_bytes: i64 = head
        .try_get("total_bytes")
        .map_err(|_| StoreError::CorruptPhysicalState)?;
    if total_bytes <= 0
        || u64::try_from(total_bytes)
            .ok()
            .is_none_or(|v| v > MAX_RUN_BYTES)
    {
        let _ = transaction.rollback().await;
        return Err(StoreError::CorruptPhysicalState);
    }
    let aggregate: (i64, Option<i64>, Option<i64>, Option<i64>) = sqlx::query_as(
        "SELECT count(*)::bigint, min(run_sequence), max(run_sequence), \
                sum(octet_length(frame_bytes))::bigint \
         FROM public.mfm_run_frames WHERE run_id = $1",
    )
    .bind(run_id.as_str())
    .fetch_one(&mut *transaction)
    .await
    .map_err(|_| StoreError::Unavailable)?;
    if aggregate.0 != head_sequence
        || aggregate.1 != Some(1)
        || aggregate.2 != Some(head_sequence)
        || aggregate.3 != Some(total_bytes)
    {
        let _ = transaction.rollback().await;
        return Err(StoreError::CorruptPhysicalState);
    }
    let rows = sqlx::query(
        "SELECT run_id, run_sequence, frame_bytes, head_digest \
         FROM public.mfm_run_frames WHERE run_id = $1 ORDER BY run_sequence",
    )
    .bind(run_id.as_str())
    .fetch_all(&mut *transaction)
    .await
    .map_err(|_| StoreError::Unavailable)?;
    let expected_run_id = run_id.as_str().to_owned();
    #[cfg(test)]
    let blocking_probe = match &probe {
        LoadProbe::BlockingPause { entered, release } => Some((
            std::sync::Arc::clone(entered),
            std::sync::Arc::clone(release),
        )),
        _ => None,
    };
    let transfer = run_pure_blocking(move || {
        #[cfg(test)]
        if let Some((entered, release)) = blocking_probe {
            use std::sync::atomic::Ordering;

            entered.notify_one();
            while !release.load(Ordering::SeqCst) {
                std::thread::yield_now();
            }
        }
        validate_load_rows(expected_run_id, head_sequence, total_bytes, rows)
    })
    .await?;
    transaction
        .commit()
        .await
        .map_err(|_| StoreError::Unavailable)?;
    Ok(Some(transfer))
}

fn validate_load_rows(
    expected_run_id: String,
    head_sequence: i64,
    total_bytes: i64,
    rows: Vec<PgRow>,
) -> std::result::Result<StoredRunBytes, StoreError> {
    if usize::try_from(head_sequence).ok() != Some(rows.len()) {
        return Err(StoreError::CorruptPhysicalState);
    }
    let mut frames = Vec::new();
    frames
        .try_reserve_exact(rows.len())
        .map_err(|_| StoreError::Unavailable)?;
    let mut total = 0_u64;
    for (offset, row) in rows.into_iter().enumerate() {
        let row_run_id: &str = row
            .try_get("run_id")
            .map_err(|_| StoreError::CorruptPhysicalState)?;
        let sequence: i64 = row
            .try_get("run_sequence")
            .map_err(|_| StoreError::CorruptPhysicalState)?;
        let bytes: &[u8] = row
            .try_get("frame_bytes")
            .map_err(|_| StoreError::CorruptPhysicalState)?;
        let stored_digest: &str = row
            .try_get("head_digest")
            .map_err(|_| StoreError::CorruptPhysicalState)?;
        let expected_sequence =
            i64::try_from(offset + 1).map_err(|_| StoreError::CorruptPhysicalState)?;
        if row_run_id != expected_run_id
            || sequence != expected_sequence
            || bytes.is_empty()
            || bytes.len() > MAX_FRAME_BYTES
            || frame_head_digest(bytes).as_str() != stored_digest
        {
            return Err(StoreError::CorruptPhysicalState);
        }
        total = total
            .checked_add(u64::try_from(bytes.len()).map_err(|_| StoreError::CorruptPhysicalState)?)
            .ok_or(StoreError::CorruptPhysicalState)?;
        let mut copied = Vec::new();
        copied
            .try_reserve_exact(bytes.len())
            .map_err(|_| StoreError::Unavailable)?;
        copied.extend_from_slice(bytes);
        frames.push(copied);
    }
    if total != u64::try_from(total_bytes).map_err(|_| StoreError::CorruptPhysicalState)? {
        return Err(StoreError::CorruptPhysicalState);
    }
    StoredRunBytes::new(frames).map_err(|_| StoreError::CorruptPhysicalState)
}

async fn append_run(
    pool: &PgPool,
    frame: &EncodedRunFrame,
    fault: CommitFault,
) -> std::result::Result<AppendResult, StoreError> {
    let run_id = frame.run_id().as_str().to_owned();
    let sequence = i64::try_from(frame.run_sequence()).map_err(|_| StoreError::Capacity)?;
    let predecessor = frame.previous_head_digest().cloned();
    let head_digest = frame.head_digest().clone();
    let bytes = own_candidate_bytes(frame.canonical_bytes()).await?;

    let mut transaction = pool.begin().await.map_err(|_| StoreError::Unavailable)?;
    configure_append_transaction(&mut transaction).await?;
    let lock_key = advisory_lock_key(&run_id);
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(lock_key)
        .execute(&mut *transaction)
        .await
        .map_err(|_| StoreError::Unavailable)?;

    let head = sqlx::query(
        "SELECT h.run_id, h.head_sequence, h.total_bytes, \
                f.frame_bytes, f.head_digest \
         FROM public.mfm_run_heads h \
         LEFT JOIN public.mfm_run_frames f \
           ON f.run_id = h.run_id AND f.run_sequence = h.head_sequence \
         WHERE h.run_id = $1",
    )
    .bind(&run_id)
    .fetch_optional(&mut *transaction)
    .await
    .map_err(|_| StoreError::Unavailable)?;
    let target = sqlx::query(
        "SELECT run_id, run_sequence, frame_bytes, head_digest \
         FROM public.mfm_run_frames WHERE run_id = $1 AND run_sequence = $2",
    )
    .bind(&run_id)
    .bind(sequence)
    .fetch_optional(&mut *transaction)
    .await
    .map_err(|_| StoreError::Unavailable)?;
    let any_frame = if head.is_none() {
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM public.mfm_run_frames WHERE run_id = $1)")
            .bind(&run_id)
            .fetch_one(&mut *transaction)
            .await
            .map_err(|_| StoreError::Unavailable)?
    } else {
        false
    };

    let candidate = PgCandidate {
        run_id,
        sequence,
        predecessor,
        head_digest,
        bytes,
    };
    let plan =
        run_pure_blocking(move || plan_pg_append(head, target, any_frame, candidate)).await?;
    let PgAppendPlan::Insert {
        arguments,
        run_id,
        sequence,
        total_bytes,
    } = plan
    else {
        let _ = transaction.rollback().await;
        return Ok(AppendResult::NotInserted);
    };

    if let Err(error) = sqlx::query_with(
        "INSERT INTO public.mfm_run_frames \
         (run_id, run_sequence, frame_bytes, head_digest) VALUES ($1,$2,$3,$4)",
        arguments,
    )
    .execute(&mut *transaction)
    .await
    {
        let _ = transaction.rollback().await;
        return Err(classify_precommit_sql(error));
    }
    if let Err(error) = sqlx::query(
        "INSERT INTO public.mfm_run_heads (run_id, head_sequence, total_bytes) \
         VALUES ($1,$2,$3) \
         ON CONFLICT (run_id) DO UPDATE SET \
           head_sequence = EXCLUDED.head_sequence, total_bytes = EXCLUDED.total_bytes",
    )
    .bind(&run_id)
    .bind(sequence)
    .bind(total_bytes)
    .execute(&mut *transaction)
    .await
    {
        let _ = transaction.rollback().await;
        return Err(classify_precommit_sql(error));
    }
    #[cfg(test)]
    match fault {
        CommitFault::BeforeSubmission => {
            transaction
                .rollback()
                .await
                .map_err(|_| StoreError::Unavailable)?;
            return Err(StoreError::Unavailable);
        }
        CommitFault::Rejected => {
            sqlx::query(
                "DELETE FROM public.mfm_run_frames \
                 WHERE run_id = $1 AND run_sequence = $2",
            )
            .bind(&run_id)
            .bind(sequence)
            .execute(&mut *transaction)
            .await
            .map_err(classify_precommit_sql)?;
        }
        CommitFault::UnknownRolledBack => {
            let _ = transaction.rollback().await;
            return Err(StoreError::Indeterminate);
        }
        CommitFault::UnknownCommitted => {
            transaction
                .commit()
                .await
                .map_err(|_| StoreError::Indeterminate)?;
            return Err(StoreError::Indeterminate);
        }
        CommitFault::None => {}
    }
    #[cfg(not(test))]
    let _ = fault;
    match transaction.commit().await {
        Ok(()) => Ok(AppendResult::Inserted),
        Err(error) if error.as_database_error().is_some() => Err(StoreError::Unavailable),
        Err(_) => Err(StoreError::Indeterminate),
    }
}

async fn configure_append_transaction(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> std::result::Result<(), StoreError> {
    sqlx::query("SET TRANSACTION ISOLATION LEVEL READ COMMITTED, READ WRITE")
        .execute(&mut **transaction)
        .await
        .map_err(|_| StoreError::Unavailable)?;
    sqlx::query("SET LOCAL synchronous_commit = on")
        .execute(&mut **transaction)
        .await
        .map_err(|_| StoreError::Unavailable)?;
    Ok(())
}

struct PgCandidate {
    run_id: String,
    sequence: i64,
    predecessor: Option<ContentDigest>,
    head_digest: ContentDigest,
    bytes: Vec<u8>,
}

enum PgAppendPlan {
    NotInserted,
    Insert {
        arguments: PgArguments,
        run_id: String,
        sequence: i64,
        total_bytes: i64,
    },
}

fn plan_pg_append(
    head: Option<PgRow>,
    target: Option<PgRow>,
    any_frame: bool,
    candidate: PgCandidate,
) -> std::result::Result<PgAppendPlan, StoreError> {
    let current = validate_observed_head(head.as_ref(), any_frame, &candidate.run_id)?;
    let target_bytes = validate_observed_target(
        target.as_ref(),
        &candidate.run_id,
        candidate.sequence,
        current.as_ref(),
    )?;
    if target_bytes.is_some_and(|bytes| bytes == candidate.bytes) {
        return Ok(PgAppendPlan::NotInserted);
    }

    let predecessor_matches = match &current {
        None => candidate.sequence == 1 && candidate.predecessor.is_none(),
        Some(current) => {
            candidate.sequence == current.sequence.saturating_add(1)
                && candidate.predecessor.as_ref() == Some(&current.digest)
        }
    };
    if !predecessor_matches {
        return Ok(PgAppendPlan::NotInserted);
    }

    let frame_len = i64::try_from(candidate.bytes.len()).map_err(|_| StoreError::Capacity)?;
    let current_total = current.as_ref().map_or(0, |head| head.total_bytes);
    if candidate.bytes.is_empty()
        || candidate.bytes.len() > MAX_FRAME_BYTES
        || u64::try_from(candidate.sequence)
            .ok()
            .is_none_or(|sequence| sequence == 0 || sequence > MAX_RUN_FRAMES)
        || current_total < 0
    {
        return Err(StoreError::Capacity);
    }
    let current_total = u64::try_from(current_total).map_err(|_| StoreError::Capacity)?;
    let remaining = MAX_RUN_BYTES
        .checked_sub(current_total)
        .ok_or(StoreError::CorruptPhysicalState)?;
    let frame_len_u64 = u64::try_from(frame_len).map_err(|_| StoreError::Capacity)?;
    if frame_len_u64 > remaining {
        return Err(StoreError::Capacity);
    }
    if frame_head_digest(&candidate.bytes) != candidate.head_digest {
        return Err(StoreError::CorruptPhysicalState);
    }
    let total_bytes = current_total
        .checked_add(frame_len_u64)
        .and_then(|total| i64::try_from(total).ok())
        .ok_or(StoreError::Capacity)?;
    let mut arguments = PgArguments::default();
    arguments
        .add(candidate.run_id.clone())
        .map_err(|_| StoreError::Unavailable)?;
    arguments
        .add(candidate.sequence)
        .map_err(|_| StoreError::Unavailable)?;
    arguments
        .add(candidate.bytes)
        .map_err(|_| StoreError::Unavailable)?;
    arguments
        .add(candidate.head_digest.as_str().to_owned())
        .map_err(|_| StoreError::Unavailable)?;
    Ok(PgAppendPlan::Insert {
        arguments,
        run_id: candidate.run_id,
        sequence: candidate.sequence,
        total_bytes,
    })
}

struct ObservedHead {
    sequence: i64,
    total_bytes: i64,
    digest: ContentDigest,
}

fn validate_observed_head(
    head: Option<&PgRow>,
    any_frame: bool,
    expected_run_id: &str,
) -> std::result::Result<Option<ObservedHead>, StoreError> {
    let Some(head) = head else {
        return (!any_frame)
            .then_some(None)
            .ok_or(StoreError::CorruptPhysicalState);
    };
    let run_id: &str = head
        .try_get("run_id")
        .map_err(|_| StoreError::CorruptPhysicalState)?;
    let sequence: i64 = head
        .try_get("head_sequence")
        .map_err(|_| StoreError::CorruptPhysicalState)?;
    let total_bytes: i64 = head
        .try_get("total_bytes")
        .map_err(|_| StoreError::CorruptPhysicalState)?;
    let bytes: Option<&[u8]> = head
        .try_get("frame_bytes")
        .map_err(|_| StoreError::CorruptPhysicalState)?;
    let digest_text: Option<&str> = head
        .try_get("head_digest")
        .map_err(|_| StoreError::CorruptPhysicalState)?;
    let (Some(bytes), Some(digest_text)) = (bytes, digest_text) else {
        return Err(StoreError::CorruptPhysicalState);
    };
    let digest = ContentDigest::parse(digest_text).map_err(|_| StoreError::CorruptPhysicalState)?;
    if run_id != expected_run_id
        || sequence <= 0
        || u64::try_from(sequence)
            .ok()
            .is_none_or(|value| value > MAX_RUN_FRAMES)
        || total_bytes <= 0
        || u64::try_from(total_bytes)
            .ok()
            .is_none_or(|value| value > MAX_RUN_BYTES)
        || bytes.is_empty()
        || bytes.len() > MAX_FRAME_BYTES
        || digest.algorithm() != DigestAlgorithm::Sha256V1
        || frame_head_digest(bytes) != digest
    {
        return Err(StoreError::CorruptPhysicalState);
    }
    Ok(Some(ObservedHead {
        sequence,
        total_bytes,
        digest,
    }))
}

fn validate_observed_target<'a>(
    target: Option<&'a PgRow>,
    expected_run_id: &str,
    candidate_sequence: i64,
    head: Option<&ObservedHead>,
) -> std::result::Result<Option<&'a [u8]>, StoreError> {
    let Some(target) = target else {
        if head.is_some_and(|head| candidate_sequence <= head.sequence) {
            return Err(StoreError::CorruptPhysicalState);
        }
        return Ok(None);
    };
    let run_id: &str = target
        .try_get("run_id")
        .map_err(|_| StoreError::CorruptPhysicalState)?;
    let sequence: i64 = target
        .try_get("run_sequence")
        .map_err(|_| StoreError::CorruptPhysicalState)?;
    let bytes: &[u8] = target
        .try_get("frame_bytes")
        .map_err(|_| StoreError::CorruptPhysicalState)?;
    let digest_text: &str = target
        .try_get("head_digest")
        .map_err(|_| StoreError::CorruptPhysicalState)?;
    let digest = ContentDigest::parse(digest_text).map_err(|_| StoreError::CorruptPhysicalState)?;
    if run_id != expected_run_id
        || sequence != candidate_sequence
        || head.is_none_or(|head| sequence > head.sequence)
        || bytes.is_empty()
        || bytes.len() > MAX_FRAME_BYTES
        || digest.algorithm() != DigestAlgorithm::Sha256V1
        || frame_head_digest(bytes) != digest
    {
        return Err(StoreError::CorruptPhysicalState);
    }
    Ok(Some(bytes))
}

fn advisory_lock_key(run_id: &str) -> i64 {
    let canonical = format!("{{\"domain\":\"mfm.store.run-lock.v1\",\"run_id\":\"{run_id}\"}}");
    let digest = sha256_digest_bytes(canonical.as_bytes());
    i64::from_be_bytes(
        digest.as_bytes()[..8]
            .try_into()
            .expect("fixed digest prefix"),
    )
}

async fn own_candidate_bytes(source: &[u8]) -> std::result::Result<Vec<u8>, StoreError> {
    let length = source.len();
    let mut owned = run_pure_blocking(move || {
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(length)
            .map_err(|_| StoreError::Unavailable)?;
        Ok(bytes)
    })
    .await?;
    for chunk in source.chunks(256 * 1024) {
        owned.extend_from_slice(chunk);
        tokio::task::yield_now().await;
    }
    Ok(owned)
}

async fn run_pure_blocking<T, F>(job: F) -> std::result::Result<T, StoreError>
where
    T: Send + 'static,
    F: FnOnce() -> std::result::Result<T, StoreError> + Send + 'static,
{
    tokio::runtime::Handle::try_current().map_err(|_| StoreError::Unavailable)?;
    tokio::task::spawn_blocking(job)
        .await
        .map_err(|_| StoreError::Unavailable)?
}

fn classify_precommit_sql(error: sqlx::Error) -> StoreError {
    if error
        .as_database_error()
        .and_then(|error| error.code())
        .is_some_and(|code| code.starts_with("23"))
    {
        StoreError::CorruptPhysicalState
    } else {
        StoreError::Unavailable
    }
}

fn assert_send_static<T: Send + 'static>() {}

const RUN_SCHEMA_SQL: &str = include_str!("../migrations/run_history_postgres_v2.sql");
const CONFIG_SCHEMA_SQL: &str = include_str!("../migrations/config_postgres_v2.sql");

#[cfg(test)]
#[path = "../../../kernel/store/tests/support/scenarios.rs"]
mod store_scenarios;

#[cfg(test)]
#[path = "../../../kernel/store/tests/support/hostile.rs"]
mod store_hostile;

#[cfg(test)]
mod tests;
