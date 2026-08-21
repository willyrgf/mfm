use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

use mfm_canonical::{raw_content_digest, PlainCanonicalJsonBytes};
use mfm_catalog::{
    CatalogEntry, CatalogError, CatalogPutResult, ConfigCatalog, ConfigDigest, ConfigName,
    RunIndex, RunPageLimit, MAX_CONFIG_ENTRIES,
};
use mfm_ids::{ContentRef, DigestAlgorithm, DigestBytes, SchemaId};
use mfm_journal::{JournalHistory, OutcomeKind};
use mfm_store::AppendResult;
use sqlx::postgres::PgSslMode;
use sqlx::{Connection, Executor};

use super::*;

fn run_id(byte: u8) -> RunId {
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

fn genesis(run_id: &RunId) -> EncodedRunFrame {
    let program = b"{}";
    let context = br#"{"value":1}"#;
    EncodedRunFrame::admission(
        run_id,
        &reference("mfm.test.program", program),
        program,
        &reference("mfm.test.context", context),
        context,
    )
    .expect("genesis")
}

fn successor(run_id: &RunId) -> EncodedRunFrame {
    let history = JournalHistory::from_genesis(genesis(run_id)).expect("history");
    let output = br#"{"value":2}"#;
    history
        .encode_pure_conclusion(
            OutcomeKind::Success,
            &reference("mfm.test.output", output),
            output,
        )
        .expect("successor")
}

fn observe_store<T>(result: Result<T, StoreError>) -> store_hostile::Observation {
    match result {
        Ok(_) => panic!("expected Store error"),
        Err(StoreError::Capacity) => store_hostile::Observation::Capacity,
        Err(StoreError::CorruptPhysicalState) => store_hostile::Observation::Corrupt,
        Err(StoreError::Unavailable | StoreError::Indeterminate) => {
            store_hostile::Observation::Unavailable
        }
    }
}

fn catalog_entry(name: &str, value: u8) -> CatalogEntry {
    let canonical = PlainCanonicalJsonBytes::from_json_str(&format!(r#"{{"value":{value}}}"#))
        .expect("canonical JSON");
    CatalogEntry::new(
        ConfigName::new(name).expect("config name"),
        ConfigDigest::new(canonical.content_digest()).expect("config digest"),
        canonical.to_vec(),
    )
    .expect("catalog entry")
}

fn managed_locators() -> (AdminPostgresLocator, RuntimePostgresLocator) {
    let admin = std::env::var("MFM_TEST_ADMIN_STORE_LOCATOR")
        .expect("postgres-test must supply the admin locator");
    let runtime = std::env::var("MFM_TEST_RUNTIME_STORE_LOCATOR")
        .expect("postgres-test must supply the runtime locator");
    (
        AdminPostgresLocator::parse(admin).expect("admin locator"),
        RuntimePostgresLocator::parse(runtime).expect("runtime locator"),
    )
}

async fn admin_connection(locator: &AdminPostgresLocator) -> PgConnection {
    let options = locator
        .connect_options("mfm-postgres-contract-test")
        .expect("admin connection options");
    PgConnection::connect_with(&options)
        .await
        .expect("admin connection")
}

async fn runtime_connection(locator: &RuntimePostgresLocator) -> PgConnection {
    let options = locator
        .connect_options("mfm-postgres-authority-test")
        .expect("runtime connection options");
    PgConnection::connect_with(&options)
        .await
        .expect("runtime connection")
}

async fn reset_schemas(connection: &mut PgConnection) {
    connection
        .execute("DROP SCHEMA IF EXISTS mfm_catalog CASCADE")
        .await
        .expect("drop catalog schema");
    connection
        .execute("DROP SCHEMA IF EXISTS public CASCADE")
        .await
        .expect("drop run schema");
    connection
        .execute("CREATE SCHEMA public AUTHORIZATION CURRENT_USER")
        .await
        .expect("create run schema");
}

#[test]
fn migration_and_classifier_contracts_are_exact() {
    assert_eq!(SCHEMA_CONTRACT, "mfm.run-history-postgres.v1");
    assert!(RUN_SCHEMA_SQL.contains("CREATE TABLE public.mfm_store_schema"));
    assert!(RUN_SCHEMA_SQL.contains("CREATE TABLE public.mfm_run_frames"));
    assert!(RUN_SCHEMA_SQL.contains("CREATE TABLE public.mfm_run_heads"));
    assert!(CATALOG_SCHEMA_SQL.contains("CREATE SCHEMA mfm_catalog"));
    assert!(CATALOG_SCHEMA_SQL.contains("CREATE TABLE mfm_catalog.config_entries"));
    assert!(!RUN_SCHEMA_SQL.contains("UNLOGGED"));
    assert!(!CATALOG_SCHEMA_SQL.contains("UNLOGGED"));
    assert_eq!(
        advisory_lock_key(run_id(7).as_str()),
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

#[tokio::test]
async fn provisioning_rejects_unequal_targets_before_connecting() {
    let authority = run_id(41).as_str().replace(':', "");
    let admin = AdminPostgresLocator::parse(format!(
        "postgresql://operator:{authority}@127.0.0.1:1/one?sslmode=disable"
    ))
    .expect("synthetic admin locator");
    let runtime = RuntimePostgresLocator::parse(format!(
        "postgresql://mfm_runtime:{authority}@127.0.0.1:1/two?sslmode=disable"
    ))
    .expect("synthetic runtime locator");
    assert_eq!(
        provision_schemas(&admin, &runtime).await,
        Err(ProvisionError::Incompatible)
    );
}

async fn assert_snapshot_and_blocking_contract(store: &Arc<PostgresStore>) {
    let snapshot_id = run_id(30);
    let snapshot_genesis = genesis(&snapshot_id);
    let snapshot_successor = successor(&snapshot_id);
    assert_eq!(
        store
            .append_run(&snapshot_genesis)
            .await
            .expect("snapshot genesis"),
        AppendResult::Inserted
    );
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let load = {
        let store = Arc::clone(store);
        let run_id = snapshot_id.clone();
        let entered = Arc::clone(&entered);
        let release = Arc::clone(&release);
        tokio::spawn(async move {
            load_run(
                &store.pool,
                &run_id,
                LoadProbe::SnapshotPause { entered, release },
            )
            .await
        })
    };
    entered.notified().await;
    assert_eq!(
        store
            .append_run(&snapshot_successor)
            .await
            .expect("concurrent append"),
        AppendResult::Inserted
    );
    release.notify_one();
    let raced = load
        .await
        .expect("snapshot join")
        .expect("snapshot load")
        .expect("snapshot present");
    assert_eq!(
        JournalHistory::qualify(&snapshot_id, raced)
            .expect("snapshot history")
            .head_sequence(),
        1
    );

    let large_id = run_id(31);
    let mut large_context = Vec::with_capacity(4 * 1024 * 1024 + 2);
    large_context.push(b'"');
    large_context.resize(4 * 1024 * 1024 + 1, b'a');
    large_context.push(b'"');
    let large = {
        let program = b"{}";
        EncodedRunFrame::admission(
            &large_id,
            &reference("mfm.test.program", program),
            program,
            &reference("mfm.test.context", &large_context),
            &large_context,
        )
        .expect("large genesis")
    };
    assert_eq!(
        store.append_run(&large).await.expect("large append"),
        AppendResult::Inserted
    );
    let heartbeat_count = Arc::new(AtomicUsize::new(0));
    let blocking_entered = Arc::new(tokio::sync::Notify::new());
    let blocking_release = Arc::new(AtomicBool::new(false));
    let load = {
        let store = Arc::clone(store);
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
    while heartbeat_count.load(Ordering::SeqCst) == 0 {
        tokio::task::yield_now().await;
    }
    blocking_release.store(true, Ordering::SeqCst);
    let retained = load
        .await
        .expect("large load join")
        .expect("large load")
        .expect("large present");
    heartbeat_done.store(true, Ordering::SeqCst);
    heartbeat.await.expect("heartbeat join");
    assert!(heartbeat_count.load(Ordering::SeqCst) > 0);
    assert_eq!(
        JournalHistory::qualify(&large_id, retained)
            .expect("large history")
            .head_sequence(),
        1
    );
}

async fn assert_commit_and_hostile_contract(
    store: &Arc<PostgresStore>,
    connection: &mut PgConnection,
) {
    use store_hostile::{HostileCase as Case, Observation};

    let mut observed = Vec::new();
    observed.push((
        Case::Absence,
        if store
            .load_run(&run_id(40))
            .await
            .expect("absent load")
            .is_none()
        {
            Observation::None
        } else {
            panic!("absent run returned retained bytes")
        },
    ));

    for (byte, fault, expected, committed) in [
        (
            41,
            CommitFault::BeforeSubmission,
            StoreError::Unavailable,
            false,
        ),
        (42, CommitFault::Rejected, StoreError::Unavailable, false),
        (
            43,
            CommitFault::UnknownRolledBack,
            StoreError::Indeterminate,
            false,
        ),
        (
            44,
            CommitFault::UnknownCommitted,
            StoreError::Indeterminate,
            true,
        ),
    ] {
        if matches!(fault, CommitFault::Rejected) {
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
                .expect("defer head constraint");
        }
        let fault_id = run_id(byte);
        let result = append_run(&store.pool, &genesis(&fault_id), fault).await;
        assert_eq!(result, Err(expected));
        if matches!(fault, CommitFault::BeforeSubmission) {
            observed.push((Case::AtomicFault, observe_store(result)));
        }
        if matches!(fault, CommitFault::Rejected) {
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
                .expect("restore head constraint");
        }
        assert_eq!(
            store
                .load_run(&fault_id)
                .await
                .expect("resolve fault")
                .is_some(),
            committed
        );
    }

    let retry_id = run_id(45);
    let retry = genesis(&retry_id);
    assert_eq!(
        store.append_run(&retry).await.expect("insert retry target"),
        AppendResult::Inserted
    );
    assert_eq!(
        store.append_run(&retry).await.expect("retry target"),
        AppendResult::NotInserted
    );
    observed.push((Case::NotInsertedBypass, Observation::NotInserted));

    let orphan_id = run_id(46);
    let orphan = genesis(&orphan_id);
    sqlx::query(
        "INSERT INTO public.mfm_run_frames \
         (run_id, run_sequence, frame_bytes, head_digest) VALUES ($1,1,$2,$3)",
    )
    .bind(orphan_id.as_str())
    .bind(orphan.canonical_bytes())
    .bind(orphan.head_digest().as_str())
    .execute(&mut *connection)
    .await
    .expect("insert orphan frame");
    observed.push((
        Case::AbsentHeadOrphan,
        observe_store(store.load_run(&orphan_id).await),
    ));

    for (byte, mutation, case) in [
        (
            47,
            "UPDATE public.mfm_run_frames SET frame_bytes = '{}'::bytea WHERE run_id = $1",
            Case::CorruptBytes,
        ),
        (
            48,
            "UPDATE public.mfm_run_frames SET head_digest = 'content:sha256-v1:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff' WHERE run_id = $1",
            Case::CorruptDigest,
        ),
        (
            49,
            "UPDATE public.mfm_run_heads SET total_bytes = total_bytes + 1 WHERE run_id = $1",
            Case::CorruptTotal,
        ),
    ] {
        let corrupt_id = run_id(byte);
        store
            .append_run(&genesis(&corrupt_id))
            .await
            .expect("seed corruption");
        sqlx::query(mutation)
            .bind(corrupt_id.as_str())
            .execute(&mut *connection)
            .await
            .expect("mutate retained row");
        observed.push((case, observe_store(store.load_run(&corrupt_id).await)));
    }

    let target_id = run_id(50);
    store
        .append_run(&genesis(&target_id))
        .await
        .expect("target genesis");
    store
        .append_run(&successor(&target_id))
        .await
        .expect("target successor");
    sqlx::query("DELETE FROM public.mfm_run_frames WHERE run_id = $1 AND run_sequence = 1")
        .bind(target_id.as_str())
        .execute(&mut *connection)
        .await
        .expect("remove historical target");
    observed.push((
        Case::CorruptTarget,
        observe_store(store.append_run(&genesis(&target_id)).await),
    ));

    let broken_head_id = run_id(51);
    store
        .append_run(&genesis(&broken_head_id))
        .await
        .expect("head genesis");
    connection
        .execute("ALTER TABLE public.mfm_run_heads DROP CONSTRAINT mfm_run_heads_frame_fkey")
        .await
        .expect("drop head constraint");
    sqlx::query("DELETE FROM public.mfm_run_frames WHERE run_id = $1")
        .bind(broken_head_id.as_str())
        .execute(&mut *connection)
        .await
        .expect("remove head frame");
    observed.push((
        Case::CorruptHead,
        observe_store(store.load_run(&broken_head_id).await),
    ));
    connection
        .execute(
            "ALTER TABLE public.mfm_run_heads \
             ADD CONSTRAINT mfm_run_heads_frame_fkey \
             FOREIGN KEY (run_id, head_sequence) \
             REFERENCES public.mfm_run_frames (run_id, run_sequence) \
             ON UPDATE NO ACTION ON DELETE NO ACTION NOT VALID",
        )
        .await
        .expect("restore unvalidated head constraint");
    sqlx::query("DELETE FROM public.mfm_run_heads WHERE run_id = $1")
        .bind(broken_head_id.as_str())
        .execute(&mut *connection)
        .await
        .expect("remove broken head");
    connection
        .execute("ALTER TABLE public.mfm_run_heads VALIDATE CONSTRAINT mfm_run_heads_frame_fkey")
        .await
        .expect("validate restored head constraint");

    let valid_digest =
        "content:sha256-v1:0000000000000000000000000000000000000000000000000000000000000000";
    let maximum_sequence_id = run_id(52);
    sqlx::query(
        "INSERT INTO public.mfm_run_frames \
         (run_id, run_sequence, frame_bytes, head_digest) VALUES ($1,65536,$2,$3)",
    )
    .bind(maximum_sequence_id.as_str())
    .bind([0_u8])
    .bind(valid_digest)
    .execute(&mut *connection)
    .await
    .expect("maximum sequence");
    assert!(sqlx::query(
        "INSERT INTO public.mfm_run_frames \
         (run_id, run_sequence, frame_bytes, head_digest) VALUES ($1,65537,$2,$3)",
    )
    .bind(run_id(53).as_str())
    .bind([0_u8])
    .bind(valid_digest)
    .execute(&mut *connection)
    .await
    .is_err());
    observed.push((Case::CountCapacity, Observation::Capacity));

    let maximum_frame_id = run_id(54);
    sqlx::query(
        "INSERT INTO public.mfm_run_frames \
         (run_id, run_sequence, frame_bytes, head_digest) \
         VALUES ($1,1,decode(repeat('00',$2),'hex'),$3)",
    )
    .bind(maximum_frame_id.as_str())
    .bind(i32::try_from(MAX_FRAME_BYTES).expect("frame bound"))
    .bind(valid_digest)
    .execute(&mut *connection)
    .await
    .expect("maximum frame bytes");
    assert!(sqlx::query(
        "INSERT INTO public.mfm_run_frames \
         (run_id, run_sequence, frame_bytes, head_digest) \
         VALUES ($1,1,decode(repeat('00',$2),'hex'),$3)",
    )
    .bind(run_id(55).as_str())
    .bind(i32::try_from(MAX_FRAME_BYTES + 1).expect("frame plus one"))
    .bind(valid_digest)
    .execute(&mut *connection)
    .await
    .is_err());
    observed.push((Case::FrameCapacity, Observation::Capacity));

    let total_id = run_id(56);
    sqlx::query(
        "INSERT INTO public.mfm_run_frames \
         (run_id, run_sequence, frame_bytes, head_digest) VALUES ($1,1,$2,$3)",
    )
    .bind(total_id.as_str())
    .bind([0_u8])
    .bind(valid_digest)
    .execute(&mut *connection)
    .await
    .expect("total frame");
    assert!(sqlx::query(
        "INSERT INTO public.mfm_run_heads (run_id, head_sequence, total_bytes) VALUES ($1,1,$2)",
    )
    .bind(total_id.as_str())
    .bind(i64::try_from(MAX_RUN_BYTES + 1).expect("run plus one"))
    .execute(&mut *connection)
    .await
    .is_err());
    observed.push((Case::RunCapacity, Observation::Capacity));

    store_hostile::assert_hostile_matrix(&observed);
}

async fn assert_catalog_mutation_contract(catalog: &Arc<PostgresCatalog>) {
    use catalog::MutationCommitFault as Fault;

    let before = catalog_entry("ambiguous-before", 1);
    assert_eq!(
        catalog::put_config_with_fault(catalog.test_pool(), &before, Fault::BeforeSubmission).await,
        Err(CatalogError::Unavailable)
    );
    assert!(catalog
        .load_config(before.name())
        .await
        .expect("resolve before-submission insert")
        .is_none());

    let rolled_back = catalog_entry("ambiguous-rollback", 2);
    assert_eq!(
        catalog::put_config_with_fault(
            catalog.test_pool(),
            &rolled_back,
            Fault::UnknownRolledBack,
        )
        .await,
        Err(CatalogError::Indeterminate)
    );
    assert!(catalog
        .load_config(rolled_back.name())
        .await
        .expect("resolve rolled-back insert")
        .is_none());

    let committed = catalog_entry("ambiguous-insert", 3);
    assert_eq!(
        catalog::put_config_with_fault(catalog.test_pool(), &committed, Fault::UnknownCommitted,)
            .await,
        Err(CatalogError::Indeterminate)
    );
    assert_eq!(
        catalog
            .load_config(committed.name())
            .await
            .expect("resolve committed insert")
            .expect("committed entry")
            .digest(),
        committed.digest()
    );

    let replacement = catalog_entry("ambiguous-insert", 4);
    assert_eq!(
        catalog::put_config_with_fault(catalog.test_pool(), &replacement, Fault::UnknownCommitted,)
            .await,
        Err(CatalogError::Indeterminate)
    );
    assert_eq!(
        catalog
            .load_config(replacement.name())
            .await
            .expect("resolve committed update")
            .expect("updated entry")
            .digest(),
        replacement.digest()
    );

    let race_left = catalog_entry("race", 5);
    let race_right = catalog_entry("race", 6);
    let barrier = Arc::new(tokio::sync::Barrier::new(2));
    let left = {
        let catalog = Arc::clone(catalog);
        let barrier = Arc::clone(&barrier);
        tokio::spawn(async move {
            barrier.wait().await;
            catalog.put_config(&race_left).await
        })
    };
    let right = {
        let catalog = Arc::clone(catalog);
        let barrier = Arc::clone(&barrier);
        tokio::spawn(async move {
            barrier.wait().await;
            catalog.put_config(&race_right).await
        })
    };
    let outcomes = [
        left.await.expect("left catalog join").expect("left insert"),
        right
            .await
            .expect("right catalog join")
            .expect("right insert"),
    ];
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| **outcome == CatalogPutResult::Inserted)
            .count(),
        1
    );
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| **outcome == CatalogPutResult::Updated)
            .count(),
        1
    );

    for (name, value) in [("beta", 7), ("gamma", 8)] {
        assert_eq!(
            catalog
                .put_config(&catalog_entry(name, value))
                .await
                .expect("page seed"),
            CatalogPutResult::Inserted
        );
    }
    for index in 0..(MAX_CONFIG_ENTRIES - 5) {
        assert_eq!(
            catalog
                .put_config(&catalog_entry(
                    &format!("quota-{index:03}"),
                    u8::try_from(index).expect("quota value"),
                ))
                .await
                .expect("quota insert"),
            CatalogPutResult::Inserted
        );
    }
    assert_eq!(
        catalog
            .put_config(&catalog_entry("quota-overflow", 9))
            .await,
        Err(CatalogError::Capacity)
    );
    assert_eq!(
        catalog
            .put_config(&catalog_entry("quota-000", u8::MAX))
            .await
            .expect("replace at capacity"),
        CatalogPutResult::Updated
    );

    let entries = catalog.list_configs().await.expect("catalog entries");
    let names = entries
        .items()
        .iter()
        .map(|entry| entry.name().clone())
        .collect::<Vec<_>>();
    assert_eq!(names.len(), MAX_CONFIG_ENTRIES);
    assert!(names.windows(2).all(|pair| pair[0] < pair[1]));
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires the managed local PostgreSQL service provided by postgres-test"]
async fn managed_postgres_persistence_authority_contract() {
    let (admin, runtime) = managed_locators();
    let mut connection = admin_connection(&admin).await;
    reset_schemas(&mut connection).await;

    provision_schemas(&admin, &runtime)
        .await
        .expect("initial provisioning");
    provision_schemas(&admin, &runtime)
        .await
        .expect("idempotent provisioning verification");

    let store = Arc::new(PostgresStore::connect(&runtime).await.expect("run store"));
    let catalog = Arc::new(PostgresCatalog::connect(&runtime).await.expect("catalog"));
    let options = runtime
        .connect_options("mfm-local-transport-contract-test")
        .expect("production connection options");
    assert!(matches!(options.get_ssl_mode(), PgSslMode::Disable));
    store_scenarios::exercise_store(store.as_ref(), &store_scenarios::run(9)).await;

    let first_run_id = run_id(21);
    let second_run_id = run_id(22);
    assert_eq!(
        store
            .append_run(&genesis(&second_run_id))
            .await
            .expect("append second run"),
        AppendResult::Inserted
    );
    assert_eq!(
        store
            .append_run(&genesis(&first_run_id))
            .await
            .expect("append first run"),
        AppendResult::Inserted
    );
    let page = store
        .list_runs(None, RunPageLimit::new(1).expect("page limit"))
        .await
        .expect("first run page");
    assert_eq!(page.items().len(), 1);
    let after = page.next_after().expect("next run id").clone();
    let next = store
        .list_runs(Some(&after), RunPageLimit::new(1).expect("page limit"))
        .await
        .expect("second run page");
    assert_eq!(next.items().len(), 1);
    assert!(page.items()[0].run_id() < next.items()[0].run_id());

    let first = catalog_entry("alpha", 1);
    let replacement = catalog_entry("alpha", 2);
    assert_eq!(
        catalog.put_config(&first).await.expect("insert config"),
        CatalogPutResult::Inserted
    );
    assert_eq!(
        catalog.put_config(&first).await.expect("retry config"),
        CatalogPutResult::Unchanged
    );
    assert_eq!(
        catalog
            .put_config(&replacement)
            .await
            .expect("replacement config"),
        CatalogPutResult::Updated
    );
    let retained = catalog
        .load_config(first.name())
        .await
        .expect("load config")
        .expect("retained config");
    assert_eq!(retained.digest(), replacement.digest());
    assert_eq!(retained.canonical_bytes(), replacement.canonical_bytes());

    assert_snapshot_and_blocking_contract(&store).await;
    assert_commit_and_hostile_contract(&store, &mut connection).await;
    assert_catalog_mutation_contract(&catalog).await;

    let mut runtime_connection = runtime_connection(&runtime).await;
    assert!(
        sqlx::query("CREATE TABLE public.runtime_must_not_own_ddl (id bigint)")
            .execute(&mut runtime_connection)
            .await
            .is_err()
    );
    assert!(sqlx::query("DELETE FROM public.mfm_run_frames")
        .execute(&mut runtime_connection)
        .await
        .is_err());
    assert!(sqlx::query("DELETE FROM mfm_catalog.config_entries")
        .execute(&mut runtime_connection)
        .await
        .is_err());

    connection
        .execute("GRANT TRUNCATE ON public.mfm_run_frames TO mfm_runtime")
        .await
        .expect("grant excess run privilege");
    assert!(matches!(
        PostgresStore::connect(&runtime).await,
        Err(StoreOpenError::Incompatible)
    ));
    connection
        .execute("REVOKE TRUNCATE ON public.mfm_run_frames FROM mfm_runtime")
        .await
        .expect("revoke excess run privilege");
    assert!(PostgresStore::connect(&runtime).await.is_ok());

    connection
        .execute("GRANT CREATE ON SCHEMA mfm_catalog TO mfm_runtime")
        .await
        .expect("grant excess catalog privilege");
    assert!(matches!(
        PostgresCatalog::connect(&runtime).await,
        Err(StoreOpenError::Incompatible)
    ));
    assert!(PostgresStore::connect(&runtime).await.is_ok());
    connection
        .execute("REVOKE CREATE ON SCHEMA mfm_catalog FROM mfm_runtime")
        .await
        .expect("revoke excess catalog privilege");

    drop((store, catalog));
    connection
        .execute("DELETE FROM mfm_catalog.mfm_catalog_schema")
        .await
        .expect("break catalog marker");
    assert!(PostgresStore::connect(&runtime).await.is_ok());
    assert!(matches!(
        PostgresCatalog::connect(&runtime).await,
        Err(StoreOpenError::Incompatible)
    ));

    reset_schemas(&mut connection).await;
    provision_schemas(&admin, &runtime)
        .await
        .expect("restore schemas");
    connection
        .execute("DELETE FROM public.mfm_store_schema")
        .await
        .expect("break run marker");
    assert!(PostgresCatalog::connect(&runtime).await.is_ok());
    assert!(matches!(
        PostgresStore::connect(&runtime).await,
        Err(StoreOpenError::Incompatible)
    ));

    reset_schemas(&mut connection).await;
    connection
        .execute("CREATE TABLE public.mfm_run_frames (surprise text)")
        .await
        .expect("partial schema");
    assert_eq!(
        provision_schemas(&admin, &runtime).await,
        Err(ProvisionError::Incompatible)
    );
    let columns: Vec<String> = sqlx::query_scalar(
        "SELECT a.attname FROM pg_catalog.pg_attribute a \
         JOIN pg_catalog.pg_class c ON c.oid = a.attrelid \
         JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
         WHERE n.nspname = 'public' AND c.relname = 'mfm_run_frames' \
           AND a.attnum > 0 AND NOT a.attisdropped ORDER BY a.attnum",
    )
    .fetch_all(&mut connection)
    .await
    .expect("partial schema columns");
    assert_eq!(columns, ["surprise"]);
    reset_schemas(&mut connection).await;
}

#[test]
#[ignore = "requires an isolated postgres-test subprocess with PGOPTIONS"]
fn managed_postgres_rejects_pgoptions() {
    let Ok(variable) = std::env::var("MFM_TEST_BLOCKED_POSTGRES_ENV") else {
        return;
    };
    assert_eq!(variable, "PGOPTIONS");
    assert!(std::env::var_os("PGOPTIONS").is_some());
    let (_, runtime) = managed_locators();
    assert!(matches!(
        runtime.connect_options("mfm-blocked-environment-test"),
        Err(PostgresLocatorError)
    ));
}
