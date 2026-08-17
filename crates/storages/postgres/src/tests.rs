use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

use mfm_canonical::raw_content_digest;
use mfm_ids::{ContentRef, DigestBytes, SchemaId};
use mfm_journal::{JournalHistory, OutcomeKind};
use mfm_store::AppendResult;

use super::*;

fn run_id() -> RunId {
    RunId::from_digest(DigestBytes::from_array([7; 32]))
}

fn run(byte: u8) -> RunId {
    RunId::from_digest(DigestBytes::from_array([byte; 32]))
}

fn reference(name: &str, bytes: &[u8]) -> ContentRef {
    ContentRef::new(
        SchemaId::new(
            name,
            "1",
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([0; 32]),
        )
        .expect("schema"),
        raw_content_digest(bytes),
    )
    .expect("reference")
}

fn genesis() -> EncodedRunFrame {
    genesis_for(&run_id(), br#"{"value":1}"#)
}

fn genesis_for(run_id: &RunId, context: &[u8]) -> EncodedRunFrame {
    let program = b"{}";
    EncodedRunFrame::admission(
        run_id,
        &reference("mfm.test.program", program),
        program,
        &reference("mfm.test.context", context),
        context,
    )
    .expect("genesis")
}

fn successor_for(run_id: &RunId, outcome: &[u8]) -> EncodedRunFrame {
    let genesis = genesis_for(run_id, br#"{"value":1}"#);
    let history = JournalHistory::from_genesis(genesis).expect("history");
    history
        .encode_pure_conclusion(
            OutcomeKind::Success,
            &reference("mfm.test.output", outcome),
            outcome,
        )
        .expect("successor")
}

async fn reset_schema(connection: &mut PgConnection) {
    connection
        .execute("DROP SCHEMA IF EXISTS public CASCADE")
        .await
        .expect("drop managed test schema");
    connection
        .execute("CREATE SCHEMA public")
        .await
        .expect("create managed test schema");
    install_fresh_schema(connection)
        .await
        .expect("install schema");
}

async fn checked_store(database_url: &str) -> Arc<PostgresStore> {
    Arc::new(
        PostgresStore::connect(database_url)
            .await
            .expect("checked store"),
    )
}

async fn assert_incompatible(database_url: &str) {
    assert!(matches!(
        PostgresStore::connect(database_url).await,
        Err(StoreOpenError::Incompatible)
    ));
}

fn observe_store<T>(result: std::result::Result<T, StoreError>) -> store_hostile::Observation {
    match result {
        Ok(_) => panic!("expected Store error"),
        Err(StoreError::Capacity) => store_hostile::Observation::Capacity,
        Err(StoreError::CorruptPhysicalState) => store_hostile::Observation::Corrupt,
        Err(StoreError::Unavailable) => store_hostile::Observation::Unavailable,
        Err(StoreError::Indeterminate) => store_hostile::Observation::Unavailable,
    }
}

#[test]
fn migration_and_classifier_contracts_are_exact() {
    assert_eq!(SCHEMA_CONTRACT, "mfm.run-history-postgres.v1");
    assert!(MIGRATION_SQL.contains("CREATE TABLE public.mfm_store_schema"));
    assert!(MIGRATION_SQL.contains("CREATE TABLE public.mfm_run_frames"));
    assert!(MIGRATION_SQL.contains("CREATE TABLE public.mfm_run_heads"));
    assert!(!MIGRATION_SQL.contains("UNLOGGED"));
    assert!(!MIGRATION_SQL.contains("predecessor"));
    assert_eq!(
        advisory_lock_key(run_id().as_str()),
        -9_027_535_993_765_170_775
    );
    assert_eq!(
        classify_open_error(sqlx::Error::Protocol(
            GateError::Incompatible.marker().to_owned()
        )),
        StoreOpenError::Incompatible
    );
    assert_eq!(
        classify_open_error(sqlx::Error::Protocol("transport".to_owned())),
        StoreOpenError::Unavailable
    );
    assert_eq!(
        classify_precommit_sql(sqlx::Error::Protocol("rejected".to_owned())),
        StoreError::Unavailable
    );
    assert!(durability_matches(true, "on", "on"));
    assert!(!durability_matches(false, "on", "on"));
    assert!(!durability_matches(true, "off", "on"));
    assert!(!durability_matches(true, "on", "off"));
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires the managed PostgreSQL service provided by the postgres-test task"]
async fn managed_postgres_store_contract() {
    use store_hostile::{HostileCase as Case, Observation};

    let mut hostile_observed = Vec::new();
    let database_url =
        std::env::var("DATABASE_URL").expect("postgres-test must supply DATABASE_URL");
    let mut connection = PgConnection::connect(&database_url)
        .await
        .expect("connect for schema install");
    reset_schema(&mut connection).await;

    let relations: Vec<String> = sqlx::query_scalar(
        "SELECT c.relname FROM pg_catalog.pg_class c \
             JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
             WHERE n.nspname = 'public' AND c.relname LIKE 'mfm_%' \
               AND c.relkind = 'r' ORDER BY c.relname",
    )
    .fetch_all(&mut connection)
    .await
    .expect("schema inventory");
    assert_eq!(
        relations,
        ["mfm_run_frames", "mfm_run_heads", "mfm_store_schema"]
    );

    let store = Arc::new(
        PostgresStore::connect(&database_url)
            .await
            .expect("checked store"),
    );
    store_scenarios::exercise_store(store.as_ref(), &store_scenarios::run(9)).await;
    hostile_observed.push((
        Case::Absence,
        if store
            .load_run(&run_id())
            .await
            .expect("absent load")
            .is_none()
        {
            Observation::None
        } else {
            panic!("absent PostgreSQL run returned a transfer")
        },
    ));

    let first = Arc::new(genesis());
    let left = {
        let store = Arc::clone(&store);
        let first = Arc::clone(&first);
        tokio::spawn(async move { store.append_run(&first).await })
    };
    let right = {
        let store = Arc::clone(&store);
        let first = Arc::clone(&first);
        tokio::spawn(async move { store.append_run(&first).await })
    };
    let results = [
        left.await.expect("left join").expect("left append"),
        right.await.expect("right join").expect("right append"),
    ];
    assert_eq!(
        results
            .iter()
            .filter(|result| **result == AppendResult::Inserted)
            .count(),
        1
    );
    assert_eq!(
        results
            .iter()
            .filter(|result| **result == AppendResult::NotInserted)
            .count(),
        1
    );

    let retained = store
        .load_run(&run_id())
        .await
        .expect("load")
        .expect("present");
    let mut history = JournalHistory::qualify(&run_id(), retained).expect("history");
    let second_bytes = br#"{"value":2}"#;
    let second = history
        .encode_pure_conclusion(
            OutcomeKind::Success,
            &reference("mfm.test.output", second_bytes),
            second_bytes,
        )
        .expect("second");
    assert_eq!(
        store.append_run(&second).await.expect("append second"),
        AppendResult::Inserted
    );
    history.extend_inserted(second).expect("extend second");
    let third_bytes = br#"{"value":3}"#;
    let third = history
        .encode_pure_conclusion(
            OutcomeKind::Failure,
            &reference("mfm.test.failure", third_bytes),
            third_bytes,
        )
        .expect("third");
    assert_eq!(
        store.append_run(&third).await.expect("append third"),
        AppendResult::Inserted
    );
    let historical_retry = store.append_run(&first).await.expect("historical retry");
    assert_eq!(historical_retry, AppendResult::NotInserted);
    hostile_observed.push((Case::NotInsertedBypass, Observation::NotInserted));

    let exact_bytes: Vec<u8> = sqlx::query_scalar(
        "SELECT frame_bytes FROM public.mfm_run_frames \
             WHERE run_id = $1 AND run_sequence = 1",
    )
    .bind(run_id().as_str())
    .fetch_one(&mut connection)
    .await
    .expect("exact BYTEA");
    assert_eq!(exact_bytes, first.canonical_bytes());

    let mut append_transaction = connection.begin().await.expect("append transaction");
    configure_append_transaction(&mut append_transaction)
        .await
        .expect("configure append transaction");
    let transaction_settings: (String, String, String) = sqlx::query_as(
        "SELECT current_setting('transaction_isolation'), \
                    current_setting('transaction_read_only'), \
                    current_setting('synchronous_commit')",
    )
    .fetch_one(&mut *append_transaction)
    .await
    .expect("transaction-local settings");
    assert_eq!(
        transaction_settings,
        (
            "read committed".to_owned(),
            "off".to_owned(),
            "on".to_owned()
        )
    );
    append_transaction.rollback().await.expect("rollback probe");

    let snapshot_id = run(10);
    let snapshot_genesis = genesis_for(&snapshot_id, br#"{"value":1}"#);
    let snapshot_successor = successor_for(&snapshot_id, br#"{"value":2}"#);
    assert_eq!(
        store
            .append_run(&snapshot_genesis)
            .await
            .expect("snapshot genesis"),
        AppendResult::Inserted
    );
    let snapshot_entered = Arc::new(tokio::sync::Notify::new());
    let snapshot_release = Arc::new(tokio::sync::Notify::new());
    let snapshot_load = {
        let store = Arc::clone(&store);
        let run_id = snapshot_id.clone();
        let entered = Arc::clone(&snapshot_entered);
        let release = Arc::clone(&snapshot_release);
        tokio::spawn(async move {
            load_run(
                &store.pool,
                &run_id,
                LoadProbe::SnapshotPause { entered, release },
            )
            .await
        })
    };
    snapshot_entered.notified().await;
    assert_eq!(
        store
            .append_run(&snapshot_successor)
            .await
            .expect("concurrent append"),
        AppendResult::Inserted
    );
    snapshot_release.notify_one();
    let raced = snapshot_load
        .await
        .expect("snapshot load join")
        .expect("snapshot load")
        .expect("snapshot present");
    assert_eq!(
        JournalHistory::qualify(&snapshot_id, raced)
            .expect("qualified raced snapshot")
            .head_sequence(),
        1
    );
    let complete = store
        .load_run(&snapshot_id)
        .await
        .expect("complete post-race load")
        .expect("present");
    assert_eq!(
        JournalHistory::qualify(&snapshot_id, complete)
            .expect("qualified")
            .head_sequence(),
        2
    );

    let mut lock_left = PgConnection::connect(&database_url)
        .await
        .expect("left lock connection");
    let mut lock_right = PgConnection::connect(&database_url)
        .await
        .expect("right lock connection");
    let mut left_transaction = lock_left.begin().await.expect("left lock transaction");
    let mut right_transaction = lock_right.begin().await.expect("right lock transaction");
    let lock_key = advisory_lock_key(snapshot_id.as_str());
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(lock_key)
        .execute(&mut *left_transaction)
        .await
        .expect("take independent lock");
    let competing_lock: bool = sqlx::query_scalar("SELECT pg_try_advisory_xact_lock($1)")
        .bind(lock_key)
        .fetch_one(&mut *right_transaction)
        .await
        .expect("try independent lock");
    assert!(!competing_lock);
    left_transaction.rollback().await.expect("left rollback");
    right_transaction.rollback().await.expect("right rollback");

    sqlx::query("DELETE FROM public.mfm_run_frames WHERE run_id = $1 AND run_sequence = 2")
        .bind(run_id().as_str())
        .execute(&mut connection)
        .await
        .expect("create interior gap");
    assert!(matches!(
        store.load_run(&run_id()).await,
        Err(StoreError::CorruptPhysicalState)
    ));

    drop(first);
    drop(store);

    for (byte, fault, expected, committed) in [
        (
            60,
            CommitFault::BeforeSubmission,
            StoreError::Unavailable,
            false,
        ),
        (61, CommitFault::Rejected, StoreError::Unavailable, false),
        (
            62,
            CommitFault::UnknownRolledBack,
            StoreError::Indeterminate,
            false,
        ),
        (
            63,
            CommitFault::UnknownCommitted,
            StoreError::Indeterminate,
            true,
        ),
    ] {
        reset_schema(&mut connection).await;
        let fault_store = checked_store(&database_url).await;
        if byte == 61 {
            connection
                .execute(
                    "ALTER TABLE public.mfm_run_heads \
                     DROP CONSTRAINT mfm_run_heads_frame_fkey, \
                     ADD CONSTRAINT mfm_run_heads_frame_fkey \
                     FOREIGN KEY (run_id, head_sequence) \
                     REFERENCES public.mfm_run_frames (run_id, run_sequence) \
                     DEFERRABLE INITIALLY DEFERRED",
                )
                .await
                .expect("defer FK for COMMIT rejection");
        }
        let fault_run = run(byte);
        let frame = genesis_for(&fault_run, br#"{"fault":true}"#);
        let fault_result = append_run(&fault_store.pool, &frame, fault).await;
        assert_eq!(fault_result, Err(expected));
        if byte == 60 {
            hostile_observed.push((Case::AtomicFault, observe_store(fault_result)));
        }
        if byte == 61 {
            connection
                .execute(
                    "ALTER TABLE public.mfm_run_heads \
                     DROP CONSTRAINT mfm_run_heads_frame_fkey, \
                     ADD CONSTRAINT mfm_run_heads_frame_fkey \
                     FOREIGN KEY (run_id, head_sequence) \
                     REFERENCES public.mfm_run_frames (run_id, run_sequence) \
                     ON UPDATE NO ACTION ON DELETE NO ACTION",
                )
                .await
                .expect("restore exact FK after COMMIT rejection");
        }
        assert_eq!(
            fault_store
                .load_run(&fault_run)
                .await
                .expect("resolve ambiguous append")
                .is_some(),
            committed
        );
        drop(fault_store);
    }

    reset_schema(&mut connection).await;
    connection
        .execute("DROP TABLE public.mfm_store_schema")
        .await
        .expect("drop marker table");
    connection
        .execute("CREATE TABLE public.mfm_store_schema (schema_contract BIGINT PRIMARY KEY)")
        .await
        .expect("install wrong marker type");
    connection
        .execute("INSERT INTO public.mfm_store_schema (schema_contract) VALUES (1)")
        .await
        .expect("insert wrong marker");
    assert_incompatible(&database_url).await;

    reset_schema(&mut connection).await;
    connection
        .execute("DELETE FROM public.mfm_store_schema")
        .await
        .expect("remove marker");
    assert_incompatible(&database_url).await;

    reset_schema(&mut connection).await;
    connection
        .execute("ALTER TABLE public.mfm_store_schema SET UNLOGGED")
        .await
        .expect("make authority unlogged");
    assert_incompatible(&database_url).await;

    reset_schema(&mut connection).await;
    connection
        .execute("CREATE TABLE public.mfm_old_receipts (id BIGINT PRIMARY KEY)")
        .await
        .expect("create old authority table");
    assert_incompatible(&database_url).await;

    reset_schema(&mut connection).await;
    connection
        .execute(
            "CREATE INDEX mfm_run_heads_legacy_idx \
                 ON public.mfm_run_heads (head_sequence)",
        )
        .await
        .expect("create old authority index");
    assert_incompatible(&database_url).await;

    reset_schema(&mut connection).await;
    connection
        .execute(
            "ALTER TABLE public.mfm_run_frames \
                 DROP CONSTRAINT mfm_run_frames_bytes_check, \
                 ADD CONSTRAINT mfm_run_frames_bytes_check \
                 CHECK (octet_length(frame_bytes) >= 1)",
        )
        .await
        .expect("weaken constraint");
    assert_incompatible(&database_url).await;

    reset_schema(&mut connection).await;
    connection
        .execute("CREATE TABLE public.operator_owned (id BIGINT PRIMARY KEY)")
        .await
        .expect("operator table");
    connection
        .execute("CREATE TABLE public.\"mfmXoperator\" (id BIGINT PRIMARY KEY)")
        .await
        .expect("non-reserved mfm operator table");
    drop(checked_store(&database_url).await);

    reset_schema(&mut connection).await;
    let reconnect_store = checked_store(&database_url).await;
    reconnect_store
        .pool
        .acquire()
        .await
        .expect("pooled connection")
        .close()
        .await
        .expect("retire checked physical connection");
    connection
        .execute("DELETE FROM public.mfm_store_schema")
        .await
        .expect("break reconnect marker");
    assert!(matches!(
        reconnect_store.load_run(&run(64)).await,
        Err(StoreError::Unavailable)
    ));
    drop(reconnect_store);

    reset_schema(&mut connection).await;
    let orphan_store = checked_store(&database_url).await;
    let orphan_id = run(65);
    let orphan = genesis_for(&orphan_id, br#"{"orphan":true}"#);
    sqlx::query(
        "INSERT INTO public.mfm_run_frames \
             (run_id, run_sequence, frame_bytes, head_digest) VALUES ($1,1,$2,$3)",
    )
    .bind(orphan_id.as_str())
    .bind(orphan.canonical_bytes())
    .bind(orphan.head_digest().as_str())
    .execute(&mut connection)
    .await
    .expect("orphan frame");
    let orphan_result = orphan_store.load_run(&orphan_id).await;
    hostile_observed.push((Case::AbsentHeadOrphan, observe_store(orphan_result)));
    drop(orphan_store);

    for (byte, mutation) in [
            (
                66,
                "UPDATE public.mfm_run_frames SET frame_bytes = '{}'::bytea WHERE run_id = $1",
            ),
            (
                67,
                "UPDATE public.mfm_run_frames SET head_digest = 'content:sha256-v1:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff' WHERE run_id = $1",
            ),
            (
                68,
                "UPDATE public.mfm_run_heads SET total_bytes = total_bytes + 1 WHERE run_id = $1",
            ),
        ] {
            reset_schema(&mut connection).await;
            let corrupt_store = checked_store(&database_url).await;
            let corrupt_id = run(byte);
            let frame = genesis_for(&corrupt_id, br#"{"value":1}"#);
            assert_eq!(
                corrupt_store
                    .append_run(&frame)
                    .await
                    .expect("seed corrupt case"),
                AppendResult::Inserted
            );
            sqlx::query(mutation)
                .bind(corrupt_id.as_str())
                .execute(&mut connection)
                .await
                .expect("corrupt physical row");
            let corrupt_result = corrupt_store.load_run(&corrupt_id).await;
            let case = match byte {
                66 => Case::CorruptBytes,
                67 => Case::CorruptDigest,
                68 => Case::CorruptTotal,
                _ => unreachable!(),
            };
            hostile_observed.push((case, observe_store(corrupt_result)));
            drop(corrupt_store);
        }

    reset_schema(&mut connection).await;
    let broken_join_store = checked_store(&database_url).await;
    let broken_join_id = run(69);
    let broken_join = genesis_for(&broken_join_id, br#"{"value":1}"#);
    assert_eq!(
        broken_join_store
            .append_run(&broken_join)
            .await
            .expect("seed broken join"),
        AppendResult::Inserted
    );
    connection
        .execute(
            "ALTER TABLE public.mfm_run_heads \
                 DROP CONSTRAINT mfm_run_heads_frame_fkey",
        )
        .await
        .expect("drop test FK");
    sqlx::query("DELETE FROM public.mfm_run_frames WHERE run_id = $1")
        .bind(broken_join_id.as_str())
        .execute(&mut connection)
        .await
        .expect("break head join");
    hostile_observed.push((
        Case::CorruptHead,
        observe_store(broken_join_store.load_run(&broken_join_id).await),
    ));
    drop(broken_join_store);

    reset_schema(&mut connection).await;
    let target_store = checked_store(&database_url).await;
    let target_id = run(70);
    let target_genesis = genesis_for(&target_id, br#"{"value":1}"#);
    let target_successor = successor_for(&target_id, br#"{"value":2}"#);
    assert_eq!(
        target_store
            .append_run(&target_genesis)
            .await
            .expect("target genesis"),
        AppendResult::Inserted
    );
    assert_eq!(
        target_store
            .append_run(&target_successor)
            .await
            .expect("target successor"),
        AppendResult::Inserted
    );
    sqlx::query("DELETE FROM public.mfm_run_frames WHERE run_id = $1 AND run_sequence = 1")
        .bind(target_id.as_str())
        .execute(&mut connection)
        .await
        .expect("remove historical target");
    hostile_observed.push((
        Case::CorruptTarget,
        observe_store(target_store.append_run(&target_genesis).await),
    ));
    drop(target_store);

    reset_schema(&mut connection).await;
    let large_store = checked_store(&database_url).await;
    let large_id = run(71);
    let mut large_context = Vec::with_capacity(4 * 1024 * 1024 + 2);
    large_context.push(b'"');
    large_context.resize(4 * 1024 * 1024 + 1, b'a');
    large_context.push(b'"');
    let large = genesis_for(&large_id, &large_context);
    assert_eq!(
        large_store.append_run(&large).await.expect("large append"),
        AppendResult::Inserted
    );
    let heartbeat_count = Arc::new(AtomicUsize::new(0));
    let blocking_entered = Arc::new(tokio::sync::Notify::new());
    let blocking_release = Arc::new(AtomicBool::new(false));
    let load_task = {
        let store = Arc::clone(&large_store);
        let run_id = large_id.clone();
        let entered = Arc::clone(&blocking_entered);
        let release = Arc::clone(&blocking_release);
        tokio::spawn(async move {
            load_run(
                &store.pool,
                &run_id,
                LoadProbe::BlockingPause { entered, release },
            )
            .await
        })
    };
    blocking_entered.notified().await;
    let heartbeat_done = Arc::new(AtomicBool::new(false));
    let heartbeat = {
        let done = Arc::clone(&heartbeat_done);
        let count = Arc::clone(&heartbeat_count);
        tokio::spawn(async move {
            while !done.load(Ordering::SeqCst) {
                count.fetch_add(1, Ordering::SeqCst);
                tokio::task::yield_now().await;
            }
        })
    };
    let before_blocking_progress = heartbeat_count.load(Ordering::SeqCst);
    while heartbeat_count.load(Ordering::SeqCst) == before_blocking_progress {
        tokio::task::yield_now().await;
    }
    blocking_release.store(true, Ordering::SeqCst);
    let large_retained = load_task
        .await
        .expect("large load join")
        .expect("large load")
        .expect("large present");
    heartbeat_done.store(true, Ordering::SeqCst);
    heartbeat.await.expect("heartbeat join");
    assert!(heartbeat_count.load(Ordering::SeqCst) > 0);
    assert_eq!(
        JournalHistory::qualify(&large_id, large_retained)
            .expect("qualify large transfer")
            .head_sequence(),
        1
    );
    drop(large_store);

    reset_schema(&mut connection).await;
    let valid_digest =
        "content:sha256-v1:0000000000000000000000000000000000000000000000000000000000000000";
    let boundary_id = run(72);
    sqlx::query(
        "INSERT INTO public.mfm_run_frames \
             (run_id, run_sequence, frame_bytes, head_digest) VALUES ($1,65536,$2,$3)",
    )
    .bind(boundary_id.as_str())
    .bind([0_u8])
    .bind(valid_digest)
    .execute(&mut connection)
    .await
    .expect("maximum sequence");
    let count_capacity = sqlx::query(
        "INSERT INTO public.mfm_run_frames \
             (run_id, run_sequence, frame_bytes, head_digest) VALUES ($1,65537,$2,$3)",
    )
    .bind(run(73).as_str())
    .bind([0_u8])
    .bind(valid_digest)
    .execute(&mut connection)
    .await;
    assert!(count_capacity.is_err());
    hostile_observed.push((Case::CountCapacity, Observation::Capacity));
    assert!(sqlx::query(
        "INSERT INTO public.mfm_run_frames \
             (run_id, run_sequence, frame_bytes, head_digest) VALUES ($1,1,$2,$3)",
    )
    .bind("run:sha256-v1:0000000000000000000000000000000000000000000000000000000000000000")
    .bind([0_u8])
    .bind(valid_digest)
    .execute(&mut connection)
    .await
    .is_err());
    assert!(sqlx::query(
        "INSERT INTO public.mfm_run_frames \
             (run_id, run_sequence, frame_bytes, head_digest) VALUES ($1,1,$2,$3)",
    )
    .bind(run(74).as_str())
    .bind([0_u8])
    .bind("content:sha256-jcs-v1:0000000000000000000000000000000000000000000000000000000000000000")
    .execute(&mut connection)
    .await
    .is_err());
    assert!(sqlx::query(
        "INSERT INTO public.mfm_run_frames \
             (run_id, run_sequence, frame_bytes, head_digest) VALUES ($1,1,$2,$3)",
    )
    .bind(run(75).as_str())
    .bind(Vec::<u8>::new())
    .bind(valid_digest)
    .execute(&mut connection)
    .await
    .is_err());

    let frame_limit_id = run(76);
    sqlx::query(
        "INSERT INTO public.mfm_run_frames \
             (run_id, run_sequence, frame_bytes, head_digest) \
             VALUES ($1,1,decode(repeat('00',$2),'hex'),$3)",
    )
    .bind(frame_limit_id.as_str())
    .bind(i32::try_from(MAX_FRAME_BYTES).expect("frame bound"))
    .bind(valid_digest)
    .execute(&mut connection)
    .await
    .expect("maximum frame bytes");
    let frame_capacity = sqlx::query(
        "INSERT INTO public.mfm_run_frames \
             (run_id, run_sequence, frame_bytes, head_digest) \
             VALUES ($1,1,decode(repeat('00',$2),'hex'),$3)",
    )
    .bind(run(77).as_str())
    .bind(i32::try_from(MAX_FRAME_BYTES + 1).expect("frame plus one"))
    .bind(valid_digest)
    .execute(&mut connection)
    .await;
    assert!(frame_capacity.is_err());
    hostile_observed.push((Case::FrameCapacity, Observation::Capacity));

    let total_id = run(78);
    sqlx::query(
        "INSERT INTO public.mfm_run_frames \
             (run_id, run_sequence, frame_bytes, head_digest) VALUES ($1,1,$2,$3)",
    )
    .bind(total_id.as_str())
    .bind([0_u8])
    .bind(valid_digest)
    .execute(&mut connection)
    .await
    .expect("total frame");
    sqlx::query(
        "INSERT INTO public.mfm_run_heads (run_id, head_sequence, total_bytes) \
             VALUES ($1,1,$2)",
    )
    .bind(total_id.as_str())
    .bind(i64::try_from(MAX_RUN_BYTES).expect("run bound"))
    .execute(&mut connection)
    .await
    .expect("maximum total bytes");
    sqlx::query("DELETE FROM public.mfm_run_heads WHERE run_id = $1")
        .bind(total_id.as_str())
        .execute(&mut connection)
        .await
        .expect("remove maximum head");
    let run_capacity = sqlx::query(
        "INSERT INTO public.mfm_run_heads (run_id, head_sequence, total_bytes) \
             VALUES ($1,1,$2)",
    )
    .bind(total_id.as_str())
    .bind(i64::try_from(MAX_RUN_BYTES + 1).expect("run plus one"))
    .execute(&mut connection)
    .await;
    assert!(run_capacity.is_err());
    hostile_observed.push((Case::RunCapacity, Observation::Capacity));

    store_hostile::assert_hostile_matrix(&hostile_observed);
}
