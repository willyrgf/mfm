use std::str::FromStr;
use std::time::{Duration, Instant};

use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool, Postgres, Transaction};
use tokio::sync::{mpsc, oneshot};

const STORE_SCOPE_ID: &str = "mfm.store_scope.v1:postgres-frontier-prototype";
const STORE_EPOCH: &str = "mfm.store.epoch.v1:postgres-frontier-prototype";
const LOCK_OBSERVATION_TIMEOUT: Duration = Duration::from_secs(10);
const INDEPENDENT_OPERATION_TIMEOUT: Duration = Duration::from_secs(30);

struct PrototypeDatabase {
    pool: PgPool,
    schema: String,
    database_url: String,
}

impl PrototypeDatabase {
    async fn create() -> Self {
        let database_url =
            std::env::var("DATABASE_URL").expect("DATABASE_URL must be set for parity tests");
        let admin_pool = PgPoolOptions::new()
            .max_connections(1)
            .connect(&database_url)
            .await
            .expect("connect prototype schema administrator");
        let schema = super::unique_schema();
        // The generated schema name contains only this process's numeric identity, time, and
        // counter. PostgreSQL cannot bind identifiers, so the audited generated name is the only
        // dynamic SQL in this prototype.
        sqlx::raw_sql(AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin_pool)
            .await
            .expect("create isolated prototype schema");
        admin_pool.close().await;

        let options = PgConnectOptions::from_str(&database_url)
            .expect("postgres URL")
            .options([("search_path", schema.as_str())]);
        let pool = PgPoolOptions::new()
            .max_connections(32)
            .connect_with(options)
            .await
            .expect("connect isolated prototype schema");
        sqlx::raw_sql(
            "CREATE TABLE prototype_tenant_fact_order_heads (
                tenant_scope_id TEXT PRIMARY KEY,
                current_fact_order BIGINT NOT NULL CHECK (current_fact_order >= 0)
            );
            CREATE TABLE prototype_fact_publications (
                tenant_scope_id TEXT NOT NULL
                    REFERENCES prototype_tenant_fact_order_heads (tenant_scope_id),
                fact_order BIGINT NOT NULL CHECK (fact_order > 0),
                producer_run_id TEXT NOT NULL,
                fact_count INTEGER NOT NULL CHECK (fact_count > 0),
                PRIMARY KEY (tenant_scope_id, fact_order)
            );
            CREATE TABLE prototype_fact_selection_barriers (
                authorization_ref TEXT PRIMARY KEY,
                tenant_scope_id TEXT NOT NULL
                    REFERENCES prototype_tenant_fact_order_heads (tenant_scope_id),
                frontier_fact_order BIGINT NOT NULL CHECK (frontier_fact_order >= 0)
            );
            CREATE TABLE prototype_replica_application (
                replica_id TEXT NOT NULL,
                tenant_scope_id TEXT NOT NULL,
                store_scope_id TEXT NOT NULL,
                store_epoch TEXT NOT NULL,
                applied_fact_order BIGINT NOT NULL CHECK (applied_fact_order >= 0),
                applied_authorization_ref TEXT,
                proof_is_trusted BOOLEAN NOT NULL,
                PRIMARY KEY (replica_id, tenant_scope_id)
            );",
        )
        .execute(&pool)
        .await
        .expect("create prototype-only frontier tables");

        Self {
            pool,
            schema,
            database_url,
        }
    }

    async fn cleanup(self) {
        let Self {
            pool,
            schema,
            database_url,
        } = self;
        pool.close().await;
        let admin_pool = PgPoolOptions::new()
            .max_connections(1)
            .connect(&database_url)
            .await
            .expect("reconnect prototype schema administrator");
        sqlx::raw_sql(AssertSqlSafe(format!(
            "DROP SCHEMA IF EXISTS {schema} CASCADE"
        )))
        .execute(&admin_pool)
        .await
        .expect("drop isolated prototype schema");
        admin_pool.close().await;
    }
}

#[derive(Default)]
struct LockHooks {
    attempting: Option<mpsc::UnboundedSender<i32>>,
    acquired: Option<oneshot::Sender<i32>>,
    release: Option<oneshot::Receiver<()>>,
}

#[derive(Debug)]
struct PublicationMeasurement {
    fact_order: i64,
    attempted_rows: u64,
    committed_rows: u64,
    elapsed: Duration,
}

#[derive(Debug)]
struct BarrierMeasurement {
    authorization_ref: String,
    frontier_fact_order: i64,
    committed_rows: u64,
    elapsed: Duration,
}

#[derive(Debug)]
struct LockWaitMeasurement {
    backend_pid: i32,
    wait_event: String,
    observed_after: Duration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HistorySourceVerdict {
    AuthoritativeWriter,
    QualifiedReplica,
    LaggingReplica,
    UnprovenReplica,
}

#[derive(Debug, Clone, Copy)]
enum HistoryReadSource<'a> {
    AuthoritativeWriter,
    Replica(&'a str),
}

#[derive(Debug, Clone, Copy)]
enum ReadPolicy {
    WriterOnly,
    QualifiedReplicaAllowed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReadFailure {
    WriterRequired,
    LaggingReplica,
    UnprovenReplica,
}

async fn ensure_tenant(pool: &PgPool, tenant_scope_id: &str) {
    sqlx::query(
        "INSERT INTO prototype_tenant_fact_order_heads (
             tenant_scope_id,
             current_fact_order
         )
         VALUES ($1, 0)
         ON CONFLICT (tenant_scope_id) DO NOTHING",
    )
    .bind(tenant_scope_id)
    .execute(pool)
    .await
    .expect("ensure prototype tenant head");
}

async fn begin_locked_tenant(
    pool: &PgPool,
    tenant_scope_id: &str,
    mut hooks: LockHooks,
) -> sqlx::Result<(Transaction<'static, Postgres>, i64)> {
    let mut tx = pool.begin().await?;
    sqlx::query(
        "INSERT INTO prototype_tenant_fact_order_heads (
             tenant_scope_id,
             current_fact_order
         )
         VALUES ($1, 0)
         ON CONFLICT (tenant_scope_id) DO NOTHING",
    )
    .bind(tenant_scope_id)
    .execute(&mut *tx)
    .await?;
    let backend_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *tx)
        .await?;
    if let Some(attempting) = hooks.attempting.take() {
        attempting
            .send(backend_pid)
            .expect("report prototype row-lock attempt");
    }
    let current_fact_order = sqlx::query_scalar(
        "SELECT current_fact_order
         FROM prototype_tenant_fact_order_heads
         WHERE tenant_scope_id = $1
         FOR UPDATE",
    )
    .bind(tenant_scope_id)
    .fetch_one(&mut *tx)
    .await?;
    if let Some(acquired) = hooks.acquired.take() {
        acquired
            .send(backend_pid)
            .expect("report acquired prototype row lock");
    }
    if let Some(release) = hooks.release.take() {
        release.await.expect("release held prototype row lock");
    }
    Ok((tx, current_fact_order))
}

async fn publish(
    pool: &PgPool,
    tenant_scope_id: &str,
    producer_run_id: &str,
    fact_count: i32,
    commit: bool,
    hooks: LockHooks,
) -> sqlx::Result<PublicationMeasurement> {
    let started = Instant::now();
    let (mut tx, current_fact_order) = begin_locked_tenant(pool, tenant_scope_id, hooks).await?;
    let fact_order = current_fact_order
        .checked_add(1)
        .expect("prototype fact order overflow");
    let inserted = sqlx::query(
        "INSERT INTO prototype_fact_publications (
             tenant_scope_id,
             fact_order,
             producer_run_id,
             fact_count
         )
         VALUES ($1, $2, $3, $4)",
    )
    .bind(tenant_scope_id)
    .bind(fact_order)
    .bind(producer_run_id)
    .bind(fact_count)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    let updated = sqlx::query(
        "UPDATE prototype_tenant_fact_order_heads
         SET current_fact_order = $2
         WHERE tenant_scope_id = $1
           AND current_fact_order = $3",
    )
    .bind(tenant_scope_id)
    .bind(fact_order)
    .bind(current_fact_order)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    assert_eq!(inserted, 1);
    assert_eq!(updated, 1);
    let attempted_rows = inserted + updated;
    let committed_rows = if commit {
        tx.commit().await?;
        attempted_rows
    } else {
        tx.rollback().await?;
        0
    };
    Ok(PublicationMeasurement {
        fact_order,
        attempted_rows,
        committed_rows,
        elapsed: started.elapsed(),
    })
}

async fn authorize_barrier(
    pool: &PgPool,
    tenant_scope_id: &str,
    authorization_ref: &str,
    hooks: LockHooks,
) -> sqlx::Result<BarrierMeasurement> {
    let started = Instant::now();
    let (mut tx, frontier_fact_order) = begin_locked_tenant(pool, tenant_scope_id, hooks).await?;
    let committed_rows = sqlx::query(
        "INSERT INTO prototype_fact_selection_barriers (
             authorization_ref,
             tenant_scope_id,
             frontier_fact_order
         )
         VALUES ($1, $2, $3)",
    )
    .bind(authorization_ref)
    .bind(tenant_scope_id)
    .bind(frontier_fact_order)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    assert_eq!(committed_rows, 1);
    tx.commit().await?;
    Ok(BarrierMeasurement {
        authorization_ref: authorization_ref.to_owned(),
        frontier_fact_order,
        committed_rows,
        elapsed: started.elapsed(),
    })
}

async fn observe_row_lock_wait(pool: &PgPool, backend_pid: i32) -> LockWaitMeasurement {
    let started = Instant::now();
    let wait_event = tokio::time::timeout(LOCK_OBSERVATION_TIMEOUT, async {
        loop {
            let activity = sqlx::query_as::<_, (Option<String>, Option<String>)>(
                "SELECT wait_event_type::text, wait_event::text
                 FROM pg_stat_activity
                 WHERE pid = $1",
            )
            .bind(backend_pid)
            .fetch_optional(pool)
            .await
            .expect("inspect prototype backend activity");
            if let Some((Some(wait_event_type), wait_event)) = activity {
                if wait_event_type == "Lock" {
                    return wait_event.unwrap_or_else(|| "unspecified".to_owned());
                }
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("prototype operation did not reach a PostgreSQL lock wait");
    LockWaitMeasurement {
        backend_pid,
        wait_event,
        observed_after: started.elapsed(),
    }
}

async fn current_fact_order(pool: &PgPool, tenant_scope_id: &str) -> i64 {
    sqlx::query_scalar(
        "SELECT current_fact_order
         FROM prototype_tenant_fact_order_heads
         WHERE tenant_scope_id = $1",
    )
    .bind(tenant_scope_id)
    .fetch_one(pool)
    .await
    .expect("load prototype tenant head")
}

async fn publication_orders(pool: &PgPool, tenant_scope_id: &str) -> Vec<i64> {
    sqlx::query_scalar(
        "SELECT fact_order
         FROM prototype_fact_publications
         WHERE tenant_scope_id = $1
         ORDER BY fact_order",
    )
    .bind(tenant_scope_id)
    .fetch_all(pool)
    .await
    .expect("load prototype publication orders")
}

async fn table_count(pool: &PgPool, query: &'static str) -> i64 {
    sqlx::query_scalar(query)
        .fetch_one(pool)
        .await
        .expect("load prototype table count")
}

async fn record_replica_application(
    pool: &PgPool,
    replica_id: &str,
    tenant_scope_id: &str,
    applied_fact_order: i64,
    applied_authorization_ref: Option<&str>,
    proof_is_trusted: bool,
    store_epoch: &str,
) {
    sqlx::query(
        "INSERT INTO prototype_replica_application (
             replica_id,
             tenant_scope_id,
             store_scope_id,
             store_epoch,
             applied_fact_order,
             applied_authorization_ref,
             proof_is_trusted
         )
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(replica_id)
    .bind(tenant_scope_id)
    .bind(STORE_SCOPE_ID)
    .bind(store_epoch)
    .bind(applied_fact_order)
    .bind(applied_authorization_ref)
    .bind(proof_is_trusted)
    .execute(pool)
    .await
    .expect("record prototype replica application");
}

async fn history_source_verdict(
    pool: &PgPool,
    tenant_scope_id: &str,
    barrier: &BarrierMeasurement,
    source: HistoryReadSource<'_>,
) -> HistorySourceVerdict {
    let HistoryReadSource::Replica(replica_id) = source else {
        return HistorySourceVerdict::AuthoritativeWriter;
    };
    let application = sqlx::query_as::<_, (String, String, i64, Option<String>, bool)>(
        "SELECT
             store_scope_id,
             store_epoch,
             applied_fact_order,
             applied_authorization_ref,
             proof_is_trusted
         FROM prototype_replica_application
         WHERE replica_id = $1
           AND tenant_scope_id = $2",
    )
    .bind(replica_id)
    .bind(tenant_scope_id)
    .fetch_optional(pool)
    .await
    .expect("load prototype replica application");
    let Some((
        store_scope_id,
        store_epoch,
        applied_fact_order,
        applied_authorization_ref,
        proof_is_trusted,
    )) = application
    else {
        return HistorySourceVerdict::UnprovenReplica;
    };
    if !proof_is_trusted || store_scope_id != STORE_SCOPE_ID || store_epoch != STORE_EPOCH {
        return HistorySourceVerdict::UnprovenReplica;
    }
    if applied_fact_order < barrier.frontier_fact_order
        || applied_authorization_ref.as_deref() != Some(barrier.authorization_ref.as_str())
    {
        return HistorySourceVerdict::LaggingReplica;
    }
    HistorySourceVerdict::QualifiedReplica
}

fn enforce_read_policy(
    policy: ReadPolicy,
    verdict: HistorySourceVerdict,
) -> Result<HistorySourceVerdict, ReadFailure> {
    match (policy, verdict) {
        (_, HistorySourceVerdict::AuthoritativeWriter)
        | (ReadPolicy::QualifiedReplicaAllowed, HistorySourceVerdict::QualifiedReplica) => {
            Ok(verdict)
        }
        (ReadPolicy::WriterOnly, HistorySourceVerdict::QualifiedReplica) => {
            Err(ReadFailure::WriterRequired)
        }
        (_, HistorySourceVerdict::LaggingReplica) => Err(ReadFailure::LaggingReplica),
        (_, HistorySourceVerdict::UnprovenReplica) => Err(ReadFailure::UnprovenReplica),
    }
}

#[tokio::test]
async fn postgres_row_locks_totally_order_same_tenant_publications_and_barriers() {
    let database = PrototypeDatabase::create().await;
    ensure_tenant(&database.pool, "publication-first").await;
    ensure_tenant(&database.pool, "barrier-first").await;

    let (publication_acquired_tx, publication_acquired_rx) = oneshot::channel();
    let (release_publication_tx, release_publication_rx) = oneshot::channel();
    let publication_pool = database.pool.clone();
    let publication = tokio::spawn(async move {
        publish(
            &publication_pool,
            "publication-first",
            "producer:publication-first",
            1,
            true,
            LockHooks {
                acquired: Some(publication_acquired_tx),
                release: Some(release_publication_rx),
                ..LockHooks::default()
            },
        )
        .await
    });
    publication_acquired_rx
        .await
        .expect("publication acquired tenant row lock");

    let (barrier_attempt_tx, mut barrier_attempt_rx) = mpsc::unbounded_channel();
    let barrier_pool = database.pool.clone();
    let barrier = tokio::spawn(async move {
        authorize_barrier(
            &barrier_pool,
            "publication-first",
            "authorization:publication-first",
            LockHooks {
                attempting: Some(barrier_attempt_tx),
                ..LockHooks::default()
            },
        )
        .await
    });
    let barrier_pid = barrier_attempt_rx
        .recv()
        .await
        .expect("barrier reported its row-lock attempt");
    let publication_first_wait = observe_row_lock_wait(&database.pool, barrier_pid).await;
    release_publication_tx
        .send(())
        .expect("release publication row lock");
    let publication_first_publication = publication
        .await
        .expect("publication task")
        .expect("publication transaction");
    let publication_first_barrier = barrier
        .await
        .expect("barrier task")
        .expect("barrier transaction");
    assert_eq!(publication_first_publication.fact_order, 1);
    assert_eq!(publication_first_publication.attempted_rows, 2);
    assert_eq!(publication_first_publication.committed_rows, 2);
    assert_eq!(publication_first_barrier.frontier_fact_order, 1);
    assert_eq!(publication_first_barrier.committed_rows, 1);

    let (barrier_acquired_tx, barrier_acquired_rx) = oneshot::channel();
    let (release_barrier_tx, release_barrier_rx) = oneshot::channel();
    let barrier_pool = database.pool.clone();
    let barrier = tokio::spawn(async move {
        authorize_barrier(
            &barrier_pool,
            "barrier-first",
            "authorization:barrier-first",
            LockHooks {
                acquired: Some(barrier_acquired_tx),
                release: Some(release_barrier_rx),
                ..LockHooks::default()
            },
        )
        .await
    });
    barrier_acquired_rx
        .await
        .expect("barrier acquired tenant row lock");

    let (publication_attempt_tx, mut publication_attempt_rx) = mpsc::unbounded_channel();
    let publication_pool = database.pool.clone();
    let publication = tokio::spawn(async move {
        publish(
            &publication_pool,
            "barrier-first",
            "producer:barrier-first",
            1,
            true,
            LockHooks {
                attempting: Some(publication_attempt_tx),
                ..LockHooks::default()
            },
        )
        .await
    });
    let publication_pid = publication_attempt_rx
        .recv()
        .await
        .expect("publication reported its row-lock attempt");
    let barrier_first_wait = observe_row_lock_wait(&database.pool, publication_pid).await;
    release_barrier_tx
        .send(())
        .expect("release barrier row lock");
    let barrier_first_barrier = barrier
        .await
        .expect("barrier task")
        .expect("barrier transaction");
    let barrier_first_publication = publication
        .await
        .expect("publication task")
        .expect("publication transaction");
    assert_eq!(barrier_first_barrier.frontier_fact_order, 0);
    assert_eq!(barrier_first_publication.fact_order, 1);

    assert_eq!(
        table_count(
            &database.pool,
            "SELECT COUNT(*) FROM prototype_tenant_fact_order_heads"
        )
        .await,
        2
    );
    assert_eq!(
        table_count(
            &database.pool,
            "SELECT COUNT(*) FROM prototype_fact_publications"
        )
        .await,
        2
    );
    assert_eq!(
        table_count(
            &database.pool,
            "SELECT COUNT(*) FROM prototype_fact_selection_barriers"
        )
        .await,
        2
    );
    assert_eq!(publication_first_wait.backend_pid, barrier_pid);
    assert_eq!(barrier_first_wait.backend_pid, publication_pid);
    assert!(!publication_first_wait.wait_event.is_empty());
    assert!(!barrier_first_wait.wait_event.is_empty());
    assert!(publication_first_wait.observed_after > Duration::ZERO);
    assert!(barrier_first_wait.observed_after > Duration::ZERO);
    assert!(publication_first_publication.elapsed > Duration::ZERO);
    assert!(publication_first_barrier.elapsed > Duration::ZERO);
    assert!(barrier_first_barrier.elapsed > Duration::ZERO);
    assert!(barrier_first_publication.elapsed > Duration::ZERO);
    eprintln!(
        "tenant_fact_frontier_postgres_ordering \
         operations=4 head_rows=2 publication_rows=2 barrier_rows=2 \
         publication_first_wait_event={} publication_first_wait_us={} \
         barrier_first_wait_event={} barrier_first_wait_us={} \
         publication_first_publication_us={} publication_first_barrier_us={} \
         barrier_first_barrier_us={} barrier_first_publication_us={}",
        publication_first_wait.wait_event,
        publication_first_wait.observed_after.as_micros(),
        barrier_first_wait.wait_event,
        barrier_first_wait.observed_after.as_micros(),
        publication_first_publication.elapsed.as_micros(),
        publication_first_barrier.elapsed.as_micros(),
        barrier_first_barrier.elapsed.as_micros(),
        barrier_first_publication.elapsed.as_micros(),
    );

    database.cleanup().await;
}

#[tokio::test]
async fn postgres_hot_tenant_contention_is_measured_without_blocking_other_tenants() {
    const HOT_PUBLICATIONS: usize = 8;

    let database = PrototypeDatabase::create().await;
    ensure_tenant(&database.pool, "hot-tenant").await;
    ensure_tenant(&database.pool, "cold-tenant").await;

    let (holder_acquired_tx, holder_acquired_rx) = oneshot::channel();
    let (release_holder_tx, release_holder_rx) = oneshot::channel();
    let holder_pool = database.pool.clone();
    let holder = tokio::spawn(async move {
        authorize_barrier(
            &holder_pool,
            "hot-tenant",
            "authorization:hot-holder",
            LockHooks {
                acquired: Some(holder_acquired_tx),
                release: Some(release_holder_rx),
                ..LockHooks::default()
            },
        )
        .await
    });
    holder_acquired_rx
        .await
        .expect("hot barrier acquired tenant row lock");

    let batch_started = Instant::now();
    let (attempt_tx, mut attempt_rx) = mpsc::unbounded_channel();
    let mut publications = Vec::with_capacity(HOT_PUBLICATIONS);
    for index in 0..HOT_PUBLICATIONS {
        let publication_pool = database.pool.clone();
        let attempt = attempt_tx.clone();
        publications.push(tokio::spawn(async move {
            publish(
                &publication_pool,
                "hot-tenant",
                &format!("hot-producer:{index}"),
                1,
                true,
                LockHooks {
                    attempting: Some(attempt),
                    ..LockHooks::default()
                },
            )
            .await
        }));
    }
    drop(attempt_tx);

    let mut waits = Vec::with_capacity(HOT_PUBLICATIONS);
    for _ in 0..HOT_PUBLICATIONS {
        let backend_pid = attempt_rx
            .recv()
            .await
            .expect("hot publication reported its row-lock attempt");
        waits.push(observe_row_lock_wait(&database.pool, backend_pid).await);
    }

    let cold_pool = database.pool.clone();
    let cold = tokio::spawn(async move {
        publish(
            &cold_pool,
            "cold-tenant",
            "cold-producer",
            1,
            true,
            LockHooks::default(),
        )
        .await
    });
    let cold = tokio::time::timeout(INDEPENDENT_OPERATION_TIMEOUT, cold)
        .await
        .expect("different-tenant publication must not wait on the hot tenant")
        .expect("cold publication task")
        .expect("cold publication transaction");
    assert!(
        !holder.is_finished(),
        "cold publication completed while the hot tenant lock remained held"
    );
    assert_eq!(cold.fact_order, 1);
    assert_eq!(cold.committed_rows, 2);

    release_holder_tx
        .send(())
        .expect("release hot tenant barrier");
    let holder = holder
        .await
        .expect("hot holder task")
        .expect("hot holder barrier");
    assert_eq!(holder.frontier_fact_order, 0);

    let mut measurements = Vec::with_capacity(HOT_PUBLICATIONS);
    for publication in publications {
        measurements.push(
            publication
                .await
                .expect("hot publication task")
                .expect("hot publication transaction"),
        );
    }
    let batch_elapsed = batch_started.elapsed();
    let mut returned_orders = measurements
        .iter()
        .map(|measurement| measurement.fact_order)
        .collect::<Vec<_>>();
    returned_orders.sort_unstable();
    assert_eq!(
        returned_orders,
        (1..=i64::try_from(HOT_PUBLICATIONS).expect("hot publication count fits"))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        publication_orders(&database.pool, "hot-tenant").await,
        returned_orders
    );
    assert_eq!(
        current_fact_order(&database.pool, "hot-tenant").await,
        i64::try_from(HOT_PUBLICATIONS).expect("hot publication count fits")
    );
    assert_eq!(current_fact_order(&database.pool, "cold-tenant").await, 1);
    assert_eq!(
        table_count(
            &database.pool,
            "SELECT COUNT(*) FROM prototype_fact_publications
             WHERE tenant_scope_id = 'hot-tenant'"
        )
        .await,
        i64::try_from(HOT_PUBLICATIONS).expect("hot publication count fits")
    );
    assert_eq!(waits.len(), HOT_PUBLICATIONS);
    assert!(waits.iter().all(|wait| !wait.wait_event.is_empty()));
    assert!(measurements
        .iter()
        .all(|measurement| measurement.attempted_rows == 2
            && measurement.committed_rows == 2
            && measurement.elapsed > Duration::ZERO));
    assert!(batch_elapsed > Duration::ZERO);
    let deterministic_rows_written = measurements
        .iter()
        .map(|measurement| measurement.committed_rows)
        .sum::<u64>();
    assert_eq!(
        deterministic_rows_written,
        u64::try_from(HOT_PUBLICATIONS).expect("hot publication count fits") * 2
    );
    let batch_elapsed_nanos = batch_elapsed.as_nanos();
    let throughput_milli_operations_per_second =
        u128::try_from(HOT_PUBLICATIONS).expect("hot publication count fits") * 1_000_000_000_000
            / batch_elapsed_nanos;
    let aggregate_operation_nanos = measurements
        .iter()
        .map(|measurement| measurement.elapsed.as_nanos())
        .sum::<u128>();
    let aggregate_lock_observation_nanos = waits
        .iter()
        .map(|wait| wait.observed_after.as_nanos())
        .sum::<u128>();
    eprintln!(
        "tenant_fact_frontier_postgres_hot_tenant \
         operations={} publication_rows={} deterministic_rows_written={} \
         observed_row_lock_waits={} batch_elapsed_us={} \
         aggregate_operation_us={} aggregate_lock_observation_us={} \
         throughput_milli_operations_per_second={} cold_elapsed_us={}",
        HOT_PUBLICATIONS,
        HOT_PUBLICATIONS,
        deterministic_rows_written,
        waits.len(),
        batch_elapsed.as_micros(),
        aggregate_operation_nanos / 1_000,
        aggregate_lock_observation_nanos / 1_000,
        throughput_milli_operations_per_second,
        cold.elapsed.as_micros(),
    );

    database.cleanup().await;
}

#[tokio::test]
async fn postgres_rollback_density_and_writer_only_replica_failures_are_explicit() {
    let database = PrototypeDatabase::create().await;
    ensure_tenant(&database.pool, "tenant").await;

    let rolled_back = publish(
        &database.pool,
        "tenant",
        "producer:rolled-back",
        2,
        false,
        LockHooks::default(),
    )
    .await
    .expect("rolled-back prototype publication");
    assert_eq!(rolled_back.fact_order, 1);
    assert_eq!(rolled_back.attempted_rows, 2);
    assert_eq!(rolled_back.committed_rows, 0);
    assert_eq!(current_fact_order(&database.pool, "tenant").await, 0);
    assert!(publication_orders(&database.pool, "tenant")
        .await
        .is_empty());

    let first = publish(
        &database.pool,
        "tenant",
        "producer:first",
        2,
        true,
        LockHooks::default(),
    )
    .await
    .expect("first committed prototype publication");
    let second = publish(
        &database.pool,
        "tenant",
        "producer:second",
        1,
        true,
        LockHooks::default(),
    )
    .await
    .expect("second committed prototype publication");
    assert_eq!(first.fact_order, 1);
    assert_eq!(second.fact_order, 2);
    assert_eq!(
        publication_orders(&database.pool, "tenant").await,
        vec![1, 2]
    );
    assert_eq!(current_fact_order(&database.pool, "tenant").await, 2);
    assert_eq!(
        table_count(
            &database.pool,
            "SELECT COUNT(*) FROM prototype_fact_publications"
        )
        .await,
        2
    );

    let barrier = authorize_barrier(
        &database.pool,
        "tenant",
        "authorization:replica-model",
        LockHooks::default(),
    )
    .await
    .expect("prototype replica-model barrier");
    assert_eq!(barrier.frontier_fact_order, 2);
    record_replica_application(
        &database.pool,
        "qualified",
        "tenant",
        2,
        Some(&barrier.authorization_ref),
        true,
        STORE_EPOCH,
    )
    .await;
    record_replica_application(
        &database.pool,
        "lagging",
        "tenant",
        1,
        Some(&barrier.authorization_ref),
        true,
        STORE_EPOCH,
    )
    .await;
    record_replica_application(
        &database.pool,
        "foreign-lineage",
        "tenant",
        2,
        Some(&barrier.authorization_ref),
        true,
        "mfm.store.epoch.v1:foreign",
    )
    .await;

    let writer = history_source_verdict(
        &database.pool,
        "tenant",
        &barrier,
        HistoryReadSource::AuthoritativeWriter,
    )
    .await;
    let qualified = history_source_verdict(
        &database.pool,
        "tenant",
        &barrier,
        HistoryReadSource::Replica("qualified"),
    )
    .await;
    let lagging = history_source_verdict(
        &database.pool,
        "tenant",
        &barrier,
        HistoryReadSource::Replica("lagging"),
    )
    .await;
    let unproven = history_source_verdict(
        &database.pool,
        "tenant",
        &barrier,
        HistoryReadSource::Replica("foreign-lineage"),
    )
    .await;
    assert_eq!(writer, HistorySourceVerdict::AuthoritativeWriter);
    assert_eq!(qualified, HistorySourceVerdict::QualifiedReplica);
    assert_eq!(lagging, HistorySourceVerdict::LaggingReplica);
    assert_eq!(unproven, HistorySourceVerdict::UnprovenReplica);
    assert_eq!(
        enforce_read_policy(ReadPolicy::WriterOnly, writer),
        Ok(HistorySourceVerdict::AuthoritativeWriter)
    );
    assert_eq!(
        enforce_read_policy(ReadPolicy::WriterOnly, qualified),
        Err(ReadFailure::WriterRequired)
    );
    assert_eq!(
        enforce_read_policy(ReadPolicy::QualifiedReplicaAllowed, lagging),
        Err(ReadFailure::LaggingReplica)
    );
    assert_eq!(
        enforce_read_policy(ReadPolicy::QualifiedReplicaAllowed, unproven),
        Err(ReadFailure::UnprovenReplica)
    );
    assert_eq!(
        enforce_read_policy(ReadPolicy::QualifiedReplicaAllowed, qualified),
        Ok(HistorySourceVerdict::QualifiedReplica)
    );
    assert!(rolled_back.elapsed > Duration::ZERO);
    assert!(first.elapsed > Duration::ZERO);
    assert!(second.elapsed > Duration::ZERO);
    assert!(barrier.elapsed > Duration::ZERO);
    eprintln!(
        "tenant_fact_frontier_postgres_density_and_source \
         attempted_operations=4 committed_operations=3 publication_rows=2 barrier_rows=1 \
         rolled_back_us={} first_publication_us={} second_publication_us={} barrier_us={} \
         writer_verdict={writer:?} qualified_verdict={qualified:?} \
         lagging_verdict={lagging:?} unproven_verdict={unproven:?}",
        rolled_back.elapsed.as_micros(),
        first.elapsed.as_micros(),
        second.elapsed.as_micros(),
        barrier.elapsed.as_micros(),
    );

    database.cleanup().await;
}
