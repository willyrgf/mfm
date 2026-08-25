use std::num::NonZeroU64;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

use mfm_canonical::{raw_content_digest, PlainCanonicalJsonBytes};
use mfm_config::{
    ConfigDigest, ConfigImportResult, ConfigName, ConfigRepository, ConfigRepositoryError,
    ConfigRevision,
};
use mfm_evm::{
    EvmAddress, EvmAuthorityEpoch, EvmBlockAnchor, EvmChainInstance, EvmHash,
    EvmTransactionConfirmation, EvmTransactionRevert, EvmTransactionSettlement, EvmU256,
};
use mfm_evm_transaction_authority::{
    AuthorityError, AuthorityState, EvmTransactionAuthority, ExactRawTransaction, NonceDomain,
};
use mfm_ids::{ContentRef, DigestAlgorithm, DigestBytes, SchemaId};
use mfm_journal::{JournalHistory, OutcomeKind};
use mfm_store::{AppendResult, RunIndex, RunPageLimit};
use mfm_values::canonicalize_mfm_value;
use sqlx::postgres::PgSslMode;
use sqlx::{Connection, Executor};

use super::*;

fn run_id(byte: u8) -> RunId {
    RunId::from_digest(DigestBytes::from_array([byte; 32]))
}

fn effect_id(byte: u8) -> mfm_ids::EffectId {
    mfm_ids::EffectId::from_digest(DigestBytes::from_array([byte; 32]))
}

fn evm_hash(byte: u8) -> EvmHash {
    EvmHash::new(format!("0x{}", format!("{byte:02x}").repeat(32))).expect("EVM hash")
}

fn evm_address(byte: u8) -> EvmAddress {
    EvmAddress::new(format!("0x{}", format!("{byte:02x}").repeat(20))).expect("EVM address")
}

fn nonce_domain(
    epoch: &EvmAuthorityEpoch,
    chain_id: u64,
    genesis_byte: u8,
    sender_byte: u8,
) -> NonceDomain {
    NonceDomain::new(
        epoch.clone(),
        EvmChainInstance::new(
            NonZeroU64::new(chain_id).expect("nonzero chain"),
            evm_hash(genesis_byte),
        ),
        evm_address(sender_byte),
    )
}

fn transaction_settlement(
    effect_id: mfm_ids::EffectId,
    nonce: u64,
    transaction_hash: EvmHash,
) -> EvmTransactionSettlement {
    EvmTransactionSettlement::confirmed(
        effect_id,
        nonce,
        EvmTransactionConfirmation::Called {
            block_anchor: EvmBlockAnchor::new(EvmU256::from_u64(12), evm_hash(12)),
            transaction_hash,
        },
    )
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

fn config_revision(name: &str, value: usize) -> ConfigRevision {
    let canonical = PlainCanonicalJsonBytes::from_json_str(&format!(r#"{{"value":{value}}}"#))
        .expect("canonical JSON");
    ConfigRevision::new(
        ConfigName::new(name).expect("config name"),
        ConfigDigest::new(canonical.content_digest()).expect("config digest"),
        canonical.to_vec(),
    )
    .expect("config revision")
}

fn managed_locators() -> (AdminPostgresLocator, RuntimePostgresLocator) {
    let admin = std::env::var("MFM_TEST_ADMIN_POSTGRES_LOCATOR")
        .expect("postgres-test must supply the admin locator");
    let runtime = std::env::var("MFM_TEST_RUNTIME_POSTGRES_LOCATOR")
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

async fn assert_base_gate_rejects(runtime: &RuntimePostgresLocator) {
    assert!(matches!(
        PostgresBackend::connect(runtime).await,
        Err(PostgresOpenError::Incompatible)
    ));
}

async fn assert_evm_gate_rejects(runtime: &RuntimePostgresLocator) {
    assert!(matches!(
        PostgresEvmTransactionAuthority::connect(runtime).await,
        Err(PostgresOpenError::Incompatible)
    ));
}

async fn reset_schemas(connection: &mut PgConnection) {
    connection
        .execute("DROP SCHEMA IF EXISTS mfm_evm_tx CASCADE")
        .await
        .expect("drop EVM transaction schema");
    connection
        .execute("DROP SCHEMA IF EXISTS mfm_config CASCADE")
        .await
        .expect("drop config schema");
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
    static_assertions::assert_not_impl_any!(PostgresBackend: EvmTransactionAuthority);
    static_assertions::assert_not_impl_any!(PostgresEvmTransactionAuthority: Store, RunIndex, ConfigRepository);
    assert_eq!(SCHEMA_CONTRACT, "mfm.run-history-postgres.v1");
    assert!(RUN_SCHEMA_SQL.contains("CREATE TABLE public.mfm_store_schema"));
    assert!(RUN_SCHEMA_SQL.contains("CREATE TABLE public.mfm_run_frames"));
    assert!(RUN_SCHEMA_SQL.contains("CREATE TABLE public.mfm_run_heads"));
    assert!(CONFIG_SCHEMA_SQL.contains("CREATE SCHEMA mfm_config"));
    assert!(CONFIG_SCHEMA_SQL.contains("CREATE TABLE mfm_config.config_revisions"));
    assert!(CONFIG_SCHEMA_SQL.contains("mfm.config-postgres.v2"));
    assert!(evm_tx::EVM_TX_SCHEMA_SQL.contains("CREATE SCHEMA mfm_evm_tx"));
    assert!(evm_tx::EVM_TX_SCHEMA_SQL.contains("CREATE TABLE mfm_evm_tx.nonce_domains"));
    assert!(evm_tx::EVM_TX_SCHEMA_SQL.contains("CREATE TABLE mfm_evm_tx.nonce_reservations"));
    assert!(evm_tx::EVM_TX_SCHEMA_SQL.contains("CREATE TABLE mfm_evm_tx.prepared_transactions"));
    assert!(evm_tx::EVM_TX_SCHEMA_SQL.contains("CREATE TABLE mfm_evm_tx.transaction_settlements"));
    assert!(!evm_tx::EVM_TX_SCHEMA_SQL.contains("INSERT INTO"));
    assert!(!CONFIG_SCHEMA_SQL.contains("current"));
    assert!(!RUN_SCHEMA_SQL.contains("UNLOGGED"));
    assert!(!CONFIG_SCHEMA_SQL.contains("UNLOGGED"));
    assert!(!evm_tx::EVM_TX_SCHEMA_SQL.contains("UNLOGGED"));
    assert_eq!(
        advisory_lock_key(run_id(7).as_str()),
        -9_027_535_993_765_170_775
    );
    assert_eq!(
        classify_open_error(sqlx::Error::Protocol(
            GateError::Incompatible.marker().to_owned()
        )),
        PostgresOpenError::Incompatible
    );
    assert_eq!(
        classify_open_error(sqlx::Error::Protocol("transport".to_owned())),
        PostgresOpenError::Unavailable
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

#[test]
fn evm_nonce_domain_lock_vector_is_frozen() {
    let key = NonceDomain::new(
        EvmAuthorityEpoch::new([1; 32]),
        EvmChainInstance::new(
            NonZeroU64::new(1).expect("nonzero chain"),
            EvmHash::new(format!("0x{}", "02".repeat(32))).expect("genesis"),
        ),
        EvmAddress::new("0x0303030303030303030303030303030303030303").expect("sender"),
    );
    let (preimage, digest, lock_key) = evm_tx::test_lock_vector(&key).expect("lock vector");
    assert_eq!(
        preimage,
        "{\"authority_epoch\":\"AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE\",\"chain_id\":1,\"domain\":\"mfm.evm.nonce-domain-lock.v1\",\"expected_genesis_hash\":\"0x0202020202020202020202020202020202020202020202020202020202020202\",\"sender\":\"0x0303030303030303030303030303030303030303\"}"
    );
    assert_eq!(
        digest,
        [
            0x41, 0xa8, 0xa1, 0xeb, 0x96, 0x53, 0x39, 0x95, 0x19, 0x6e, 0xb0, 0x5e, 0x6e, 0x74,
            0x62, 0xa5, 0xfb, 0x04, 0x4d, 0xaa, 0x8f, 0x69, 0x4a, 0xe8, 0x3d, 0x91, 0xa1, 0xab,
            0x6b, 0x1b, 0xe8, 0x01,
        ]
    );
    assert_eq!(lock_key, 4_731_209_441_764_129_173);
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
        provision_postgres(&admin, &runtime).await,
        Err(ProvisionError::Incompatible)
    );
}

async fn assert_snapshot_and_blocking_contract(store: &Arc<PostgresBackend>) {
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
    store: &Arc<PostgresBackend>,
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

async fn assert_config_mutation_contract(backend: &Arc<PostgresBackend>) {
    use config::MutationCommitFault as Fault;

    let before = config_revision("ambiguous-before", 1);
    assert_eq!(
        config::import_config_with_fault(backend.test_pool(), &before, Fault::BeforeSubmission)
            .await,
        Err(ConfigRepositoryError::Unavailable)
    );
    assert!(backend
        .load_config(before.name(), before.digest())
        .await
        .expect("resolve before-submission import")
        .is_none());

    let rolled_back = config_revision("ambiguous-rollback", 2);
    assert_eq!(
        config::import_config_with_fault(
            backend.test_pool(),
            &rolled_back,
            Fault::UnknownRolledBack,
        )
        .await,
        Err(ConfigRepositoryError::Indeterminate)
    );
    assert!(backend
        .load_config(rolled_back.name(), rolled_back.digest())
        .await
        .expect("resolve rolled-back import")
        .is_none());

    let committed = config_revision("ambiguous-import", 3);
    assert_eq!(
        config::import_config_with_fault(backend.test_pool(), &committed, Fault::UnknownCommitted,)
            .await,
        Err(ConfigRepositoryError::Indeterminate)
    );
    assert_eq!(
        backend
            .load_config(committed.name(), committed.digest())
            .await
            .expect("resolve committed import")
            .expect("committed revision")
            .digest(),
        committed.digest()
    );

    let second = config_revision("ambiguous-import", 4);
    assert_eq!(
        config::import_config_with_fault(backend.test_pool(), &second, Fault::UnknownCommitted,)
            .await,
        Err(ConfigRepositoryError::Indeterminate)
    );
    assert_eq!(
        backend
            .load_config(second.name(), second.digest())
            .await
            .expect("resolve second committed import")
            .expect("second revision")
            .digest(),
        second.digest()
    );
    assert_eq!(
        backend
            .load_config(committed.name(), committed.digest())
            .await
            .expect("load first revision")
            .expect("first revision")
            .canonical_bytes(),
        committed.canonical_bytes()
    );
    assert_eq!(
        backend
            .import_config(&committed)
            .await
            .expect("repeat first revision"),
        ConfigImportResult::Unchanged
    );
    let collision = ConfigRevision::new(
        committed.name().clone(),
        committed.digest().clone(),
        br#"{"different":true}"#.to_vec(),
    )
    .expect("bounded collision");
    assert_eq!(
        backend.import_config(&collision).await,
        Err(ConfigRepositoryError::Corrupt)
    );

    let race_left = config_revision("race", 5);
    let race_right = race_left.clone();
    let barrier = Arc::new(tokio::sync::Barrier::new(2));
    let left = {
        let backend = Arc::clone(backend);
        let barrier = Arc::clone(&barrier);
        tokio::spawn(async move {
            barrier.wait().await;
            backend.import_config(&race_left).await
        })
    };
    let right = {
        let backend = Arc::clone(backend);
        let barrier = Arc::clone(&barrier);
        tokio::spawn(async move {
            barrier.wait().await;
            backend.import_config(&race_right).await
        })
    };
    let outcomes = [
        left.await.expect("left config join").expect("left import"),
        right
            .await
            .expect("right config join")
            .expect("right import"),
    ];
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| **outcome == ConfigImportResult::Created)
            .count(),
        1
    );
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| **outcome == ConfigImportResult::Unchanged)
            .count(),
        1
    );

    for index in 0..300 {
        assert_eq!(
            backend
                .import_config(&config_revision(&format!("unbounded-{index:03}"), index,))
                .await
                .expect("unbounded import"),
            ConfigImportResult::Created
        );
    }

    let entries = backend.list_configs().await.expect("config revisions");
    assert_eq!(
        entries
            .iter()
            .filter(|entry| entry.name().as_str() == "ambiguous-import")
            .count(),
        2
    );
    assert_eq!(
        entries
            .iter()
            .filter(|entry| entry.name().as_str() == "race")
            .count(),
        1
    );
    assert_eq!(
        entries
            .iter()
            .filter(|entry| entry.name().as_str().starts_with("unbounded-"))
            .count(),
        300
    );

    assert_delete_fault(
        backend,
        config_revision("delete-before", 7),
        Fault::BeforeSubmission,
        ConfigRepositoryError::Unavailable,
        true,
    )
    .await;
    assert_delete_fault(
        backend,
        config_revision("delete-rollback", 8),
        Fault::UnknownRolledBack,
        ConfigRepositoryError::Indeterminate,
        true,
    )
    .await;
    let delete_committed = config_revision("delete-committed", 9);
    assert_delete_fault(
        backend,
        delete_committed.clone(),
        Fault::UnknownCommitted,
        ConfigRepositoryError::Indeterminate,
        false,
    )
    .await;
    backend
        .delete_config(delete_committed.name(), delete_committed.digest())
        .await
        .expect("idempotent delete retry");
}

async fn assert_delete_fault(
    backend: &PostgresBackend,
    revision: ConfigRevision,
    fault: config::MutationCommitFault,
    error: ConfigRepositoryError,
    retained: bool,
) {
    backend.import_config(&revision).await.expect("import");
    assert_eq!(
        config::delete_config_with_fault(
            backend.test_pool(),
            revision.name(),
            revision.digest(),
            fault,
        )
        .await,
        Err(error)
    );
    assert_eq!(
        backend
            .load_config(revision.name(), revision.digest())
            .await
            .expect("resolve delete")
            .is_some(),
        retained
    );
}

async fn assert_evm_transaction_authority_contract(backend: &Arc<PostgresEvmTransactionAuthority>) {
    let epoch = backend.authority_epoch().clone();
    let domain = nonce_domain(&epoch, 1, 2, 3);
    let command_ref = reference("mfm.test.evm-command", &[5]);
    let first_effect = effect_id(6);
    assert!(backend
        .load(&first_effect, &command_ref)
        .await
        .expect("absent load")
        .is_none());

    let reserved = backend
        .reserve_or_compare(&first_effect, &command_ref, &domain, 7)
        .await
        .expect("first reservation");
    assert_eq!(reserved.nonce(), 7);
    assert!(reserved.domain() == &domain);
    assert_eq!(
        backend
            .reserve_or_compare(&first_effect, &command_ref, &domain, 999)
            .await
            .expect("same Effect retry")
            .nonce(),
        7
    );
    assert_eq!(
        backend
            .load(&first_effect, &reference("mfm.test.other-command", &[8]))
            .await
            .err(),
        Some(AuthorityError::Internal)
    );
    assert_eq!(
        backend
            .reserve_or_compare(
                &first_effect,
                &reference("mfm.test.other-command", &[8]),
                &domain,
                7,
            )
            .await
            .err(),
        Some(AuthorityError::Internal)
    );
    assert_eq!(
        backend
            .reserve_or_compare(&effect_id(11), &command_ref, &domain, 8)
            .await
            .err(),
        Some(AuthorityError::Unavailable)
    );

    let transaction_hash = evm_hash(13);
    let raw = ExactRawTransaction::new(vec![2, 0xc0]).expect("raw transaction");
    assert_eq!(
        backend
            .retain_prepared(
                &first_effect,
                &reference("mfm.test.other-command", &[8]),
                &transaction_hash,
                &raw,
            )
            .await
            .err(),
        Some(AuthorityError::Internal)
    );
    let prepared = backend
        .retain_prepared(&first_effect, &command_ref, &transaction_hash, &raw)
        .await
        .expect("retain prepared");
    assert_eq!(prepared.raw_transaction().as_bytes(), raw.as_bytes());
    assert_eq!(
        backend
            .retain_prepared(&first_effect, &command_ref, &transaction_hash, &raw)
            .await
            .expect("prepared retry")
            .raw_transaction()
            .as_bytes(),
        raw.as_bytes()
    );
    assert_eq!(
        backend
            .retain_prepared(
                &first_effect,
                &command_ref,
                &transaction_hash,
                &ExactRawTransaction::new(vec![2, 0xc1]).expect("wrong raw"),
            )
            .await
            .err(),
        Some(AuthorityError::Internal)
    );
    let evidence = transaction_settlement(first_effect.clone(), 7, transaction_hash.clone());
    assert_eq!(
        backend
            .retain_settlement(
                &first_effect,
                &reference("mfm.test.other-command", &[8]),
                &evidence,
            )
            .await
            .err(),
        Some(AuthorityError::Internal)
    );
    assert_eq!(
        backend
            .retain_settlement(
                &first_effect,
                &command_ref,
                &transaction_settlement(effect_id(41), 7, transaction_hash.clone()),
            )
            .await
            .err(),
        Some(AuthorityError::Internal)
    );
    assert_eq!(
        backend
            .retain_settlement(
                &first_effect,
                &command_ref,
                &transaction_settlement(first_effect.clone(), 7, evm_hash(42)),
            )
            .await
            .err(),
        Some(AuthorityError::Internal)
    );
    assert_eq!(
        backend
            .retain_settlement(
                &first_effect,
                &command_ref,
                &transaction_settlement(first_effect.clone(), 8, transaction_hash.clone()),
            )
            .await
            .err(),
        Some(AuthorityError::Internal)
    );
    let settled = backend
        .retain_settlement(&first_effect, &command_ref, &evidence)
        .await
        .expect("retain settlement");
    assert_eq!(settled.evidence(), &evidence);
    assert_eq!(
        backend
            .retain_settlement(&first_effect, &command_ref, &evidence)
            .await
            .expect("settlement retry")
            .evidence(),
        &evidence
    );
    let AuthorityState::Settled(loaded) = backend
        .load(&first_effect, &command_ref)
        .await
        .expect("settled load")
        .expect("settled state")
    else {
        panic!("expected settled authority state");
    };
    assert_eq!(loaded.evidence(), &evidence);
    assert_eq!(
        loaded.prepared().raw_transaction().as_bytes(),
        raw.as_bytes()
    );

    let second_effect = effect_id(14);
    for observed in [7, 9] {
        assert_eq!(
            backend
                .reserve_or_compare(&second_effect, &command_ref, &domain, observed)
                .await
                .err(),
            Some(AuthorityError::Unavailable)
        );
    }
    assert_eq!(
        backend
            .reserve_or_compare(&second_effect, &command_ref, &domain, 8)
            .await
            .expect("next reservation")
            .nonce(),
        8
    );
    let second_hash = evm_hash(43);
    backend
        .retain_prepared(
            &second_effect,
            &command_ref,
            &second_hash,
            &ExactRawTransaction::new(vec![2, 0xc4]).expect("revert raw"),
        )
        .await
        .expect("revert prepared");
    let reverted = EvmTransactionSettlement::reverted(
        second_effect.clone(),
        8,
        EvmTransactionRevert::new(
            EvmBlockAnchor::new(EvmU256::from_u64(13), evm_hash(44)),
            second_hash,
        ),
    );
    backend
        .retain_settlement(&second_effect, &command_ref, &reverted)
        .await
        .expect("revert settlement");
    assert_eq!(
        backend
            .reserve_or_compare(&effect_id(45), &command_ref, &domain, 9)
            .await
            .expect("post-revert reservation")
            .nonce(),
        9
    );

    let zero_domain = nonce_domain(&epoch, 46, 47, 48);
    assert_eq!(
        backend
            .reserve_or_compare(&effect_id(50), &command_ref, &zero_domain, 0)
            .await
            .expect("zero reservation")
            .nonce(),
        0
    );

    let max_domain = nonce_domain(&epoch, u64::MAX, 15, 16);
    let max_effect = effect_id(18);
    assert_eq!(
        backend
            .reserve_or_compare(&max_effect, &command_ref, &max_domain, u64::MAX)
            .await
            .expect("u64 max reservation")
            .nonce(),
        u64::MAX
    );
    let max_hash = evm_hash(39);
    backend
        .retain_prepared(
            &max_effect,
            &command_ref,
            &max_hash,
            &ExactRawTransaction::new(vec![2, 0xc3]).expect("max raw"),
        )
        .await
        .expect("max prepared");
    backend
        .retain_settlement(
            &max_effect,
            &command_ref,
            &transaction_settlement(max_effect.clone(), u64::MAX, max_hash),
        )
        .await
        .expect("max settlement");
    assert_eq!(
        backend
            .reserve_or_compare(&effect_id(40), &command_ref, &max_domain, 0)
            .await
            .err(),
        Some(AuthorityError::Internal)
    );
    let distinct_genesis = nonce_domain(&epoch, 1, 19, 3);
    assert_eq!(
        backend
            .reserve_or_compare(&effect_id(20), &command_ref, &distinct_genesis, 42)
            .await
            .expect("distinct genesis domain")
            .nonce(),
        42
    );

    let racing_domain = nonce_domain(&epoch, 21, 22, 23);
    let race = [effect_id(25), effect_id(26)]
        .into_iter()
        .map(|effect| {
            let backend = Arc::clone(backend);
            let command_ref = command_ref.clone();
            let domain = racing_domain.clone();
            tokio::spawn(async move {
                backend
                    .reserve_or_compare(&effect, &command_ref, &domain, 3)
                    .await
            })
        })
        .collect::<Vec<_>>();
    let mut outcomes = Vec::new();
    for attempt in race {
        outcomes.push(attempt.await.expect("different Effect race join"));
    }
    assert_eq!(outcomes.iter().filter(|outcome| outcome.is_ok()).count(), 1);
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| outcome.as_ref().err() == Some(&AuthorityError::Unavailable))
            .count(),
        1
    );

    let same_effect_domain = nonce_domain(&epoch, 27, 28, 29);
    let same_effect = effect_id(31);
    let attempts = (0..2)
        .map(|_| {
            let backend = Arc::clone(backend);
            let command_ref = command_ref.clone();
            let domain = same_effect_domain.clone();
            let effect = same_effect.clone();
            tokio::spawn(async move {
                backend
                    .reserve_or_compare(&effect, &command_ref, &domain, 4)
                    .await
            })
        })
        .collect::<Vec<_>>();
    for attempt in attempts {
        assert_eq!(
            attempt
                .await
                .expect("same Effect join")
                .expect("same Effect convergence")
                .nonce(),
            4
        );
    }

    let retained_bytes: Vec<u8> = sqlx::query_scalar(
        "SELECT settlement_bytes FROM mfm_evm_tx.transaction_settlements WHERE effect_id = $1",
    )
    .bind(first_effect.as_str())
    .fetch_one(backend.test_pool())
    .await
    .expect("retained settlement bytes");
    assert_eq!(
        retained_bytes,
        canonicalize_mfm_value(&evidence)
            .expect("canonical evidence")
            .0
            .as_bytes()
    );

    let fault_domain = nonce_domain(&epoch, 32, 33, 34);
    let fault_effect = effect_id(36);
    backend.inject_authority_commit_fault(evm_tx::AuthorityCommitFault::UnknownRolledBack);
    assert_eq!(
        backend
            .reserve_or_compare(&fault_effect, &command_ref, &fault_domain, 5)
            .await
            .err(),
        Some(AuthorityError::Unavailable)
    );
    assert!(backend
        .load(&fault_effect, &command_ref)
        .await
        .expect("rolled-back reservation load")
        .is_none());
    backend.inject_authority_commit_fault(evm_tx::AuthorityCommitFault::UnknownCommitted);
    assert_eq!(
        backend
            .reserve_or_compare(&fault_effect, &command_ref, &fault_domain, 5)
            .await
            .err(),
        Some(AuthorityError::Unavailable)
    );
    assert!(matches!(
        backend
            .load(&fault_effect, &command_ref)
            .await
            .expect("committed reservation load"),
        Some(AuthorityState::Reserved(_))
    ));

    let fault_hash = evm_hash(37);
    let fault_raw = ExactRawTransaction::new(vec![2, 0xc2]).expect("fault raw");
    backend.inject_authority_commit_fault(evm_tx::AuthorityCommitFault::UnknownRolledBack);
    assert_eq!(
        backend
            .retain_prepared(&fault_effect, &command_ref, &fault_hash, &fault_raw)
            .await
            .err(),
        Some(AuthorityError::Unavailable)
    );
    assert!(matches!(
        backend
            .load(&fault_effect, &command_ref)
            .await
            .expect("rolled-back prepared load"),
        Some(AuthorityState::Reserved(_))
    ));
    backend.inject_authority_commit_fault(evm_tx::AuthorityCommitFault::UnknownCommitted);
    assert_eq!(
        backend
            .retain_prepared(&fault_effect, &command_ref, &fault_hash, &fault_raw)
            .await
            .err(),
        Some(AuthorityError::Unavailable)
    );
    assert!(matches!(
        backend
            .load(&fault_effect, &command_ref)
            .await
            .expect("committed prepared load"),
        Some(AuthorityState::Prepared(_))
    ));

    let fault_evidence = transaction_settlement(fault_effect.clone(), 5, fault_hash);
    backend.inject_authority_commit_fault(evm_tx::AuthorityCommitFault::UnknownRolledBack);
    assert_eq!(
        backend
            .retain_settlement(&fault_effect, &command_ref, &fault_evidence)
            .await
            .err(),
        Some(AuthorityError::Unavailable)
    );
    assert!(matches!(
        backend
            .load(&fault_effect, &command_ref)
            .await
            .expect("rolled-back settlement load"),
        Some(AuthorityState::Prepared(_))
    ));
    backend.inject_authority_commit_fault(evm_tx::AuthorityCommitFault::UnknownCommitted);
    assert_eq!(
        backend
            .retain_settlement(&fault_effect, &command_ref, &fault_evidence)
            .await
            .err(),
        Some(AuthorityError::Unavailable)
    );
    assert!(matches!(
        backend
            .load(&fault_effect, &command_ref)
            .await
            .expect("committed settlement load"),
        Some(AuthorityState::Settled(_))
    ));
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires the managed local PostgreSQL service provided by postgres-test"]
async fn managed_postgres_persistence_authority_contract() {
    let (admin, runtime) = managed_locators();
    let mut connection = admin_connection(&admin).await;
    reset_schemas(&mut connection).await;

    provision_postgres(&admin, &runtime)
        .await
        .expect("initial provisioning");
    provision_postgres(&admin, &runtime)
        .await
        .expect("idempotent provisioning verification");
    let evm_schema_exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM pg_catalog.pg_namespace WHERE nspname = 'mfm_evm_tx')",
    )
    .fetch_one(&mut connection)
    .await
    .expect("optional schema absence");
    assert!(!evm_schema_exists);
    assert!(matches!(
        PostgresEvmTransactionAuthority::connect(&runtime).await,
        Err(PostgresOpenError::Incompatible)
    ));

    let backend = Arc::new(
        PostgresBackend::connect(&runtime)
            .await
            .expect("postgres backend"),
    );
    provision_evm_transaction_authority(&admin, &runtime)
        .await
        .expect("authority provisioning");
    provision_evm_transaction_authority(&admin, &runtime)
        .await
        .expect("idempotent authority provisioning");
    let authority = Arc::new(
        PostgresEvmTransactionAuthority::connect(&runtime)
            .await
            .expect("transaction authority"),
    );
    let stable_authority = PostgresEvmTransactionAuthority::connect(&runtime)
        .await
        .expect("stable authority");
    assert_eq!(
        stable_authority.authority_epoch(),
        authority.authority_epoch()
    );
    drop(stable_authority);
    let options = runtime
        .connect_options("mfm-local-transport-contract-test")
        .expect("production connection options");
    assert!(matches!(options.get_ssl_mode(), PgSslMode::Disable));
    store_scenarios::exercise_store(backend.as_ref(), &store_scenarios::run(9)).await;

    let first_run_id = run_id(21);
    let second_run_id = run_id(22);
    assert_eq!(
        backend
            .append_run(&genesis(&second_run_id))
            .await
            .expect("append second run"),
        AppendResult::Inserted
    );
    assert_eq!(
        backend
            .append_run(&genesis(&first_run_id))
            .await
            .expect("append first run"),
        AppendResult::Inserted
    );
    let page = backend
        .list_runs(None, RunPageLimit::new(1).expect("page limit"))
        .await
        .expect("first run page");
    assert_eq!(page.items().len(), 1);
    let after = page.next_after().expect("next run id").clone();
    let next = backend
        .list_runs(Some(&after), RunPageLimit::new(1).expect("page limit"))
        .await
        .expect("second run page");
    assert_eq!(next.items().len(), 1);
    assert!(page.items()[0].run_id() < next.items()[0].run_id());

    let first = config_revision("alpha", 1);
    let replacement = config_revision("alpha", 2);
    assert_eq!(
        backend.import_config(&first).await.expect("import config"),
        ConfigImportResult::Created
    );
    assert_eq!(
        backend.import_config(&first).await.expect("retry config"),
        ConfigImportResult::Unchanged
    );
    assert_eq!(
        backend
            .import_config(&replacement)
            .await
            .expect("new revision"),
        ConfigImportResult::Created
    );
    let retained = backend
        .load_config(replacement.name(), replacement.digest())
        .await
        .expect("load config")
        .expect("retained config");
    assert_eq!(retained.digest(), replacement.digest());
    assert_eq!(retained.canonical_bytes(), replacement.canonical_bytes());
    let original = backend
        .load_config(first.name(), first.digest())
        .await
        .expect("load original config")
        .expect("original config");
    assert_eq!(original.canonical_bytes(), first.canonical_bytes());

    assert_snapshot_and_blocking_contract(&backend).await;
    assert_commit_and_hostile_contract(&backend, &mut connection).await;
    assert_config_mutation_contract(&backend).await;
    assert_evm_transaction_authority_contract(&authority).await;

    let epoch_bytes = authority.authority_epoch().as_bytes();
    for invalid_chain_id in ["0", "18446744073709551616"] {
        assert!(sqlx::query(
            "INSERT INTO mfm_evm_tx.nonce_domains \
             (authority_epoch, chain_id, genesis_hash, sender) \
             VALUES ($1, $2::numeric, $3, $4)",
        )
        .bind(epoch_bytes)
        .bind(invalid_chain_id)
        .bind([51_u8; 32].as_slice())
        .bind([52_u8; 20].as_slice())
        .execute(&mut connection)
        .await
        .is_err());
    }
    let command_ref = reference("mfm.test.evm-command", &[5]);
    assert!(sqlx::query(
        "INSERT INTO mfm_evm_tx.nonce_reservations \
         (effect_id, command_schema_id, command_content_digest, authority_epoch, chain_id, \
          genesis_hash, sender, reserved_nonce) \
         VALUES ($1, $2, $3, $4, 46, $5, $6, 18446744073709551616)",
    )
    .bind(effect_id(52).as_str())
    .bind(command_ref.schema_id().as_str())
    .bind(command_ref.content_digest().as_str())
    .bind(epoch_bytes)
    .bind([47_u8; 32].as_slice())
    .bind([48_u8; 20].as_slice())
    .execute(&mut connection)
    .await
    .is_err());

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
    assert!(
        sqlx::query("UPDATE mfm_config.config_revisions SET canonical = canonical")
            .execute(&mut runtime_connection)
            .await
            .is_err()
    );
    assert!(sqlx::query("DELETE FROM mfm_evm_tx.nonce_reservations")
        .execute(&mut runtime_connection)
        .await
        .is_err());
    assert!(
        sqlx::query("UPDATE mfm_evm_tx.nonce_domains SET sender = sender")
            .execute(&mut runtime_connection)
            .await
            .is_err()
    );

    connection
        .execute("GRANT TRUNCATE ON public.mfm_run_frames TO mfm_runtime")
        .await
        .expect("grant excess run privilege");
    assert!(matches!(
        PostgresBackend::connect(&runtime).await,
        Err(PostgresOpenError::Incompatible)
    ));
    connection
        .execute("REVOKE TRUNCATE ON public.mfm_run_frames FROM mfm_runtime")
        .await
        .expect("revoke excess run privilege");
    assert!(PostgresBackend::connect(&runtime).await.is_ok());

    connection
        .execute("GRANT CREATE ON SCHEMA mfm_config TO mfm_runtime")
        .await
        .expect("grant excess config privilege");
    assert!(matches!(
        PostgresBackend::connect(&runtime).await,
        Err(PostgresOpenError::Incompatible)
    ));
    connection
        .execute("REVOKE CREATE ON SCHEMA mfm_config FROM mfm_runtime")
        .await
        .expect("revoke excess config privilege");

    connection
        .execute("GRANT DELETE ON mfm_evm_tx.nonce_reservations TO mfm_runtime")
        .await
        .expect("grant excess transaction privilege");
    assert!(matches!(
        PostgresEvmTransactionAuthority::connect(&runtime).await,
        Err(PostgresOpenError::Incompatible)
    ));
    assert!(PostgresBackend::connect(&runtime).await.is_ok());
    connection
        .execute("REVOKE DELETE ON mfm_evm_tx.nonce_reservations FROM mfm_runtime")
        .await
        .expect("revoke excess transaction privilege");

    for (grant, revoke) in [
        (
            "GRANT UPDATE (frame_bytes) ON public.mfm_run_frames TO mfm_runtime",
            "REVOKE UPDATE (frame_bytes) ON public.mfm_run_frames FROM mfm_runtime",
        ),
        (
            "GRANT UPDATE (total_bytes) ON public.mfm_run_heads TO mfm_runtime",
            "REVOKE UPDATE (total_bytes) ON public.mfm_run_heads FROM mfm_runtime",
        ),
        (
            "GRANT UPDATE (canonical) ON mfm_config.config_revisions TO mfm_runtime",
            "REVOKE UPDATE (canonical) ON mfm_config.config_revisions FROM mfm_runtime",
        ),
    ] {
        connection
            .execute(grant)
            .await
            .expect("grant hostile column privilege");
        assert_base_gate_rejects(&runtime).await;
        connection
            .execute(revoke)
            .await
            .expect("revoke hostile column privilege");
    }

    connection
        .execute(
            "GRANT UPDATE (raw_transaction) \
             ON mfm_evm_tx.prepared_transactions TO mfm_runtime",
        )
        .await
        .expect("grant raw transaction mutation authority");
    assert_evm_gate_rejects(&runtime).await;
    connection
        .execute(
            "REVOKE UPDATE (raw_transaction) \
             ON mfm_evm_tx.prepared_transactions FROM mfm_runtime",
        )
        .await
        .expect("revoke raw transaction mutation authority");

    connection
        .execute("GRANT MAINTAIN ON public.mfm_run_frames TO mfm_runtime")
        .await
        .expect("grant maintain");
    assert_base_gate_rejects(&runtime).await;
    connection
        .execute("REVOKE MAINTAIN ON public.mfm_run_frames FROM mfm_runtime")
        .await
        .expect("revoke maintain");

    connection
        .execute("CREATE ROLE mfm_unexpected_runtime_grant NOLOGIN")
        .await
        .expect("create inherited privilege role");
    connection
        .execute("GRANT SELECT ON public.mfm_store_schema TO mfm_unexpected_runtime_grant")
        .await
        .expect("grant inherited privilege");
    connection
        .execute("GRANT mfm_unexpected_runtime_grant TO mfm_runtime")
        .await
        .expect("grant unexpected membership");
    assert_base_gate_rejects(&runtime).await;
    connection
        .execute("REVOKE mfm_unexpected_runtime_grant FROM mfm_runtime")
        .await
        .expect("revoke unexpected membership");
    connection
        .execute("REVOKE SELECT ON public.mfm_store_schema FROM mfm_unexpected_runtime_grant")
        .await
        .expect("revoke inherited privilege");
    connection
        .execute("DROP ROLE mfm_unexpected_runtime_grant")
        .await
        .expect("drop inherited privilege role");

    connection
        .execute("CREATE ROLE mfm_unexpected_surface_owner NOLOGIN")
        .await
        .expect("create hostile owner");
    connection
        .execute("ALTER TABLE mfm_config.config_revisions OWNER TO mfm_unexpected_surface_owner")
        .await
        .expect("change surface owner");
    assert_base_gate_rejects(&runtime).await;
    connection
        .execute("ALTER TABLE mfm_config.config_revisions OWNER TO CURRENT_USER")
        .await
        .expect("restore surface owner");
    connection
        .execute("DROP ROLE mfm_unexpected_surface_owner")
        .await
        .expect("drop hostile owner");

    connection
        .execute("ALTER TABLE mfm_config.config_revisions ENABLE ROW LEVEL SECURITY")
        .await
        .expect("enable row security");
    assert_base_gate_rejects(&runtime).await;
    connection
        .execute("ALTER TABLE mfm_config.config_revisions DISABLE ROW LEVEL SECURITY")
        .await
        .expect("disable row security");
    connection
        .execute("CREATE POLICY mfm_hostile_policy ON mfm_config.config_revisions USING (true)")
        .await
        .expect("create row security policy");
    connection
        .execute("ALTER TABLE mfm_config.config_revisions ENABLE ROW LEVEL SECURITY")
        .await
        .expect("enable policy");
    assert_base_gate_rejects(&runtime).await;
    connection
        .execute("DROP POLICY mfm_hostile_policy ON mfm_config.config_revisions")
        .await
        .expect("drop row security policy");
    connection
        .execute("ALTER TABLE mfm_config.config_revisions DISABLE ROW LEVEL SECURITY")
        .await
        .expect("disable policy");

    connection
        .execute(
            "CREATE RULE mfm_hostile_rule AS ON DELETE TO mfm_config.config_revisions \
             DO ALSO NOTHING",
        )
        .await
        .expect("create hostile rule");
    assert_base_gate_rejects(&runtime).await;
    connection
        .execute("DROP RULE mfm_hostile_rule ON mfm_config.config_revisions")
        .await
        .expect("drop hostile rule");

    connection
        .execute("ALTER TABLE mfm_config.config_revisions SET (fillfactor = 90)")
        .await
        .expect("set hostile relation option");
    assert_base_gate_rejects(&runtime).await;
    connection
        .execute("ALTER TABLE mfm_config.config_revisions RESET (fillfactor)")
        .await
        .expect("reset hostile relation option");

    let tablespace_path = std::env::temp_dir().join(format!(
        "mfm-postgres-hostile-tablespace-{}",
        std::process::id()
    ));
    std::fs::create_dir(&tablespace_path).expect("create hostile tablespace directory");
    let create_tablespace: String = sqlx::query_scalar(
        "SELECT format('CREATE TABLESPACE mfm_hostile_tablespace LOCATION %L', $1)",
    )
    .bind(tablespace_path.to_string_lossy().as_ref())
    .fetch_one(&mut connection)
    .await
    .expect("render tablespace statement");
    sqlx::query(sqlx::AssertSqlSafe(create_tablespace.as_str()))
        .execute(&mut connection)
        .await
        .expect("create hostile tablespace");
    connection
        .execute("ALTER TABLE mfm_config.config_revisions SET TABLESPACE mfm_hostile_tablespace")
        .await
        .expect("move relation to hostile tablespace");
    assert_base_gate_rejects(&runtime).await;
    connection
        .execute("ALTER TABLE mfm_config.config_revisions SET TABLESPACE pg_default")
        .await
        .expect("restore relation tablespace");
    connection
        .execute("DROP TABLESPACE mfm_hostile_tablespace")
        .await
        .expect("drop hostile tablespace");
    std::fs::remove_dir(&tablespace_path).expect("remove hostile tablespace directory");

    for (break_index, restore_index) in [
        (
            "UPDATE pg_catalog.pg_index SET indisvalid = false \
             WHERE indexrelid = 'mfm_config.mfm_config_revisions_pkey'::regclass",
            "UPDATE pg_catalog.pg_index SET indisvalid = true \
             WHERE indexrelid = 'mfm_config.mfm_config_revisions_pkey'::regclass",
        ),
        (
            "UPDATE pg_catalog.pg_index SET indisready = false \
             WHERE indexrelid = 'mfm_config.mfm_config_revisions_pkey'::regclass",
            "UPDATE pg_catalog.pg_index SET indisready = true \
             WHERE indexrelid = 'mfm_config.mfm_config_revisions_pkey'::regclass",
        ),
        (
            "UPDATE pg_catalog.pg_index SET indislive = false \
             WHERE indexrelid = 'mfm_config.mfm_config_revisions_pkey'::regclass",
            "UPDATE pg_catalog.pg_index SET indislive = true \
             WHERE indexrelid = 'mfm_config.mfm_config_revisions_pkey'::regclass",
        ),
    ] {
        connection
            .execute(break_index)
            .await
            .expect("break expected index state");
        assert_base_gate_rejects(&runtime).await;
        connection
            .execute(restore_index)
            .await
            .expect("restore expected index state");
    }

    connection
        .execute(
            "CREATE INDEX mfm_hostile_expression_index \
             ON mfm_config.config_revisions ((octet_length(canonical))) \
             WHERE octet_length(canonical) > 0",
        )
        .await
        .expect("create hostile expression and partial index");
    assert_base_gate_rejects(&runtime).await;
    connection
        .execute("DROP INDEX mfm_config.mfm_hostile_expression_index")
        .await
        .expect("drop hostile expression index");

    connection
        .execute(
            "UPDATE pg_catalog.pg_constraint SET convalidated = false \
             WHERE conname = 'mfm_config_revisions_bytes_check' \
               AND conrelid = 'mfm_config.config_revisions'::regclass",
        )
        .await
        .expect("invalidate expected constraint");
    assert_base_gate_rejects(&runtime).await;
    connection
        .execute(
            "UPDATE pg_catalog.pg_constraint SET convalidated = true \
             WHERE conname = 'mfm_config_revisions_bytes_check' \
               AND conrelid = 'mfm_config.config_revisions'::regclass",
        )
        .await
        .expect("restore expected constraint");

    connection
        .execute("CREATE TABLE mfm_config.unexpected_relation (value bigint)")
        .await
        .expect("create unexpected relation");
    assert_base_gate_rejects(&runtime).await;
    connection
        .execute("DROP TABLE mfm_config.unexpected_relation")
        .await
        .expect("drop unexpected relation");
    connection
        .execute("ALTER TABLE mfm_config.config_revisions ADD COLUMN unexpected bytea")
        .await
        .expect("add unexpected column");
    assert_base_gate_rejects(&runtime).await;
    connection
        .execute("ALTER TABLE mfm_config.config_revisions DROP COLUMN unexpected")
        .await
        .expect("drop unexpected column");
    assert_base_gate_rejects(&runtime).await;

    connection
        .execute(
            "UPDATE mfm_evm_tx.transaction_settlements SET settlement_bytes = '{ }'::bytea \
             WHERE effect_id = 'effect:sha256-jcs-v1:2424242424242424242424242424242424242424242424242424242424242424'",
        )
        .await
        .expect("inject noncanonical settlement bytes");
    assert_eq!(
        authority
            .load(&effect_id(36), &reference("mfm.test.evm-command", &[5]),)
            .await
            .err(),
        Some(AuthorityError::Internal)
    );
    connection
        .execute(
            "UPDATE mfm_evm_tx.transaction_settlements SET settlement_bytes = '{}'::bytea \
             WHERE effect_id = 'effect:sha256-jcs-v1:2424242424242424242424242424242424242424242424242424242424242424'",
        )
        .await
        .expect("inject malformed settlement bytes");
    assert_eq!(
        authority
            .reserve_or_compare(
                &effect_id(38),
                &reference("mfm.test.evm-command", &[5]),
                &nonce_domain(authority.authority_epoch(), 32, 33, 34),
                6,
            )
            .await
            .err(),
        Some(AuthorityError::Internal)
    );

    let original_epoch = authority.authority_epoch().clone();
    reset_schemas(&mut connection).await;
    provision_postgres(&admin, &runtime)
        .await
        .expect("fresh base recreation");
    provision_evm_transaction_authority(&admin, &runtime)
        .await
        .expect("fresh authority recreation");
    let replacement = PostgresEvmTransactionAuthority::connect(&runtime)
        .await
        .expect("replacement authority");
    assert_ne!(replacement.authority_epoch(), &original_epoch);
    assert_eq!(
        authority
            .load(&effect_id(6), &reference("mfm.test.evm-command", &[5]),)
            .await
            .err(),
        Some(AuthorityError::Internal)
    );

    drop(authority);
    drop(replacement);
    connection
        .execute("DELETE FROM mfm_config.mfm_config_schema")
        .await
        .expect("break config marker");
    assert!(matches!(
        PostgresBackend::connect(&runtime).await,
        Err(PostgresOpenError::Incompatible)
    ));

    reset_schemas(&mut connection).await;
    provision_postgres(&admin, &runtime)
        .await
        .expect("restore schemas");
    connection
        .execute("DELETE FROM public.mfm_store_schema")
        .await
        .expect("break run marker");
    assert!(matches!(
        PostgresBackend::connect(&runtime).await,
        Err(PostgresOpenError::Incompatible)
    ));

    reset_schemas(&mut connection).await;
    provision_postgres(&admin, &runtime)
        .await
        .expect("complete base before optional-state test");
    connection
        .execute("CREATE SCHEMA mfm_evm_tx")
        .await
        .expect("partial optional schema");
    provision_postgres(&admin, &runtime)
        .await
        .expect("base ignores optional authority state");
    assert!(PostgresBackend::connect(&runtime).await.is_ok());
    assert_eq!(
        provision_evm_transaction_authority(&admin, &runtime).await,
        Err(ProvisionError::Incompatible)
    );

    reset_schemas(&mut connection).await;
    provision_evm_transaction_authority(&admin, &runtime)
        .await
        .expect("standalone authority provisioning");
    assert!(PostgresEvmTransactionAuthority::connect(&runtime)
        .await
        .is_ok());
    assert!(matches!(
        PostgresBackend::connect(&runtime).await,
        Err(PostgresOpenError::Incompatible)
    ));
    provision_postgres(&admin, &runtime)
        .await
        .expect("base provisioning after authority");
    assert!(PostgresBackend::connect(&runtime).await.is_ok());

    connection
        .execute(
            "ALTER TABLE mfm_evm_tx.mfm_evm_tx_schema \
             DROP CONSTRAINT mfm_evm_tx_schema_contract_check",
        )
        .await
        .expect("remove current marker constraint");
    connection
        .execute(
            "UPDATE mfm_evm_tx.mfm_evm_tx_schema \
             SET schema_contract = 'mfm.evm-transaction-postgres.v1'",
        )
        .await
        .expect("install obsolete marker");
    assert!(matches!(
        PostgresEvmTransactionAuthority::connect(&runtime).await,
        Err(PostgresOpenError::Incompatible)
    ));
    assert!(PostgresBackend::connect(&runtime).await.is_ok());

    reset_schemas(&mut connection).await;
    connection
        .execute("CREATE TABLE public.mfm_run_frames (surprise text)")
        .await
        .expect("partial schema");
    assert_eq!(
        provision_postgres(&admin, &runtime).await,
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
