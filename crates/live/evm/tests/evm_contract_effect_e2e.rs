//! Managed PostgreSQL/Reth coverage of lost reservation acknowledgement, reconstructed Runtime
//! recovery, external nonce advancement, and unchanged terminal history. Keystore custody stays
//! alive across Runtime reconstruction. Preparation recovery without signing and Journal append
//! faults are exercised in `src/transaction_tests.rs`; this fixture counts submission and Pending calls.

use std::io::Read;
use std::num::NonZeroU64;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use mfm_chain::transaction::*;
use mfm_evm::custody::{
    AuthorityError, AuthorityFuture, EvmTransactionAuthority, LoadedTransaction, PreparedRecord,
    Reservation,
};
use mfm_evm::{
    Eip1559Options, EvmAddress, EvmAuthorityEpoch, EvmBalanceRoute, EvmContractExecutionConfig,
    EvmEndpoint, EvmHash, EvmScalarContractArtifact, EvmTransactionBinding, EvmTransactionRoute,
    EvmU256,
};
use mfm_evm_live::client::contract::{ContractResources, EvmContractConfig};
use mfm_evm_live::{
    ethereum_address, EvmAdapterLocator, EvmReadProvider, EvmTransactionProvider,
    EvmTransactionResource, JsonRpcEvmProvider, EVM_EIP1559_SIGNING_PURPOSE_ID,
};
use mfm_ids::{ContentRef, DigestBytes, EffectId, EntryPointId, RunId, StableId};
use mfm_keystore::{KeystoreOwner, SecretSecp256k1Scalar};
use mfm_program::{compile, load, Effect, Operation, Program, ProgramLimits, Pure};
use mfm_runtime::{RunView, RunViewState, Runtime, RuntimeError};
use mfm_signing::Secp256k1Signer;
use mfm_storage_postgres::{
    provision_evm_transaction_authority, provision_postgres, AdminPostgresLocator, PostgresBackend,
    PostgresEvmTransactionAuthority, RuntimePostgresLocator,
};
use mfm_store::Store;
use zeroize::Zeroizing;

const MAX_INITCODE_BYTES: usize = 49_152;
// Cold progress requalifies the complete accumulating history on every invocation.
const PROGRESS_TIMEOUT: Duration = Duration::from_secs(300);
const POLL_INTERVAL: Duration = Duration::from_millis(100);
const CONFIGURED_VALUE: u64 = 42;
const DEPLOYMENT_GAS: u64 = 2_000_000;
const CONFIGURATION_GAS: u64 = 200_000;
const PRIORITY_FEE: u64 = 1_000_000_000;
const MAX_FEE: u64 = 10_000_000_000;
const FUNDING_WEI: u64 = 1_000_000_000_000_000_000;

#[path = "support/managed_provider.rs"]
mod managed_provider;
use managed_provider::*;

fn nonzero(value: u64) -> NonZeroU64 {
    NonZeroU64::new(value).expect("nonzero fixture")
}

struct ReservationAcknowledgementFault {
    inner: Arc<PostgresEvmTransactionAuthority>,
    consumed: Arc<AtomicBool>,
}

impl EvmTransactionAuthority for ReservationAcknowledgementFault {
    fn authority_epoch(&self) -> &EvmAuthorityEpoch {
        self.inner.authority_epoch()
    }

    fn load<'a>(
        &'a self,
        effect_id: &'a EffectId,
    ) -> AuthorityFuture<'a, Option<LoadedTransaction>> {
        self.inner.load(effect_id)
    }

    fn reserve_or_compare<'a>(
        &'a self,
        effect_id: &'a EffectId,
        command_value_ref: &'a ContentRef,
        domain: &'a mfm_evm::custody::NonceDomain,
        observed_pending_nonce: u64,
    ) -> AuthorityFuture<'a, Reservation> {
        Box::pin(async move {
            let retained = self
                .inner
                .reserve_or_compare(effect_id, command_value_ref, domain, observed_pending_nonce)
                .await?;
            if !self.consumed.swap(true, Ordering::SeqCst) {
                return Err(AuthorityError::Unavailable(
                    mfm_values::DiagnosticEvidence::from_value(
                        serde_json::json!({"operation": "test.authority", "injected": "unavailable"}),
                    ),
                ));
            }
            Ok(retained)
        })
    }

    fn retain_prepared<'a>(
        &'a self,
        reservation: &'a Reservation,
        candidate: &'a PreparedRecord,
    ) -> AuthorityFuture<'a, PreparedRecord> {
        self.inner.retain_prepared(reservation, candidate)
    }
}

async fn runtime(
    runtime_locator: &RuntimePostgresLocator,
    rpc_locator: &EvmAdapterLocator,
    binding: &EvmTransactionBinding,
    signer: Arc<dyn Secp256k1Signer>,
    consumed: Arc<AtomicBool>,
    calls: Arc<ProviderCalls>,
) -> (Runtime, ContractResources) {
    let backend = Arc::new(
        PostgresBackend::connect(runtime_locator)
            .await
            .expect("admitted backend"),
    );
    let transaction_authority = Arc::new(
        PostgresEvmTransactionAuthority::connect(runtime_locator)
            .await
            .expect("admitted transaction authority"),
    );
    assert_eq!(
        transaction_authority.authority_epoch(),
        (&binding.authority_epoch)
    );
    let provider = Arc::new(JsonRpcEvmProvider::connect(rpc_locator).expect("provider"));
    let authority: Arc<dyn EvmTransactionAuthority> = Arc::new(ReservationAcknowledgementFault {
        inner: transaction_authority,
        consumed,
    });
    let transaction_provider = Arc::new(ObservedProvider {
        inner: provider.clone(),
        calls,
    });
    let read_provider: Arc<dyn EvmReadProvider> = provider;
    let resource =
        EvmTransactionResource::new(binding.clone(), signer, authority, transaction_provider)
            .unwrap();
    let route = EvmBalanceRoute::new(
        binding.route.chain_instance.chain_id,
        EvmEndpoint::new("reth-effect-e2e").unwrap(),
    );
    let resources = ContractResources::new(vec![(route, read_provider)], vec![resource]).unwrap();
    let store: Arc<dyn Store> = backend;
    (Runtime::new(store), resources)
}

fn fixture_initcode() -> Vec<u8> {
    let path =
        std::env::var_os("MFM_EFFECT_E2E_INITCODE_PATH").expect("managed fixture initcode path");
    let maximum_text_bytes = MAX_INITCODE_BYTES
        .checked_mul(2)
        .and_then(|value| value.checked_add(1))
        .expect("fixture bound");
    let file = std::fs::File::open(path).expect("fixture initcode");
    let mut encoded = Vec::new();
    file.take(u64::try_from(maximum_text_bytes + 1).expect("fixture bound"))
        .read_to_end(&mut encoded)
        .expect("fixture initcode");
    assert!(encoded.len() <= maximum_text_bytes);
    let encoded = std::str::from_utf8(&encoded).expect("ASCII fixture initcode");
    let digits = encoded.strip_suffix('\n').unwrap_or(encoded);
    assert!(
        !digits.is_empty()
            && digits.len().is_multiple_of(2)
            && digits.len() / 2 <= MAX_INITCODE_BYTES
            && digits
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    );
    alloy_primitives::hex::decode(digits).expect("hexadecimal fixture initcode")
}

async fn cold_program(runtime: &Runtime, resources: &ContractResources, run: &RunId) -> Program {
    let document = runtime.program_document(run).await.unwrap();
    load(document.canonical_bytes(), resources).unwrap()
}

#[derive(Debug, thiserror::Error)]
enum FundingError {
    #[error("Reth development funding is unavailable")]
    Build(#[from] mfm_evm_live::EvmProviderBuildError),
    #[error("Reth development funding is unavailable")]
    Provider(#[from] mfm_capabilities::AdapterError<mfm_evm::EvmOperationalError>),
}

async fn fund_sender(locator: &str, sender: &EvmAddress) -> Result<(), FundingError> {
    let locator = EvmAdapterLocator::parse(locator)?;
    let provider = JsonRpcEvmProvider::connect(&locator)?;
    let transaction = provider
        .fund_development_sender(
            sender,
            &EvmU256::from_u64(FUNDING_WEI),
            u128::from(MAX_FEE),
            u128::from(PRIORITY_FEE),
        )
        .await?;
    tokio::time::timeout(PROGRESS_TIMEOUT, async {
        while provider.receipt(&transaction).await?.is_none() {
            tokio::time::sleep(POLL_INTERVAL).await;
        }
        Ok::<_, FundingError>(())
    })
    .await
    .expect("funding settlement deadline")?;
    Ok(())
}

// This transfer deliberately bypasses Program and custody, like another wallet application.
async fn external_wallet_transfer(
    provider: &JsonRpcEvmProvider,
    signer: &dyn Secp256k1Signer,
    binding: &EvmTransactionBinding,
    nonce: u64,
) {
    use alloy_consensus::{SignableTransaction, TxEip1559};
    use alloy_eips::Encodable2718;
    use alloy_primitives::{Address, Signature, TxKind};
    let transaction = TxEip1559 {
        chain_id: binding.route.chain_instance.chain_id.get(),
        nonce,
        gas_limit: 21_000,
        max_fee_per_gas: u128::from(MAX_FEE),
        max_priority_fee_per_gas: u128::from(PRIORITY_FEE),
        to: TxKind::Call(Address::from(*binding.sender.as_bytes())),
        ..Default::default()
    };
    let digest = mfm_signing::SigningDigest::from_bytes(transaction.signature_hash().into());
    let signature = signer
        .sign(digest)
        .await
        .expect("external wallet signature");
    assert!(signature.recovery_id() <= 1);
    let signature =
        Signature::from_bytes_and_parity(signature.as_bytes(), signature.recovery_id() == 1);
    let signed = transaction.into_signed(signature);
    let raw = mfm_evm::custody::ExactRawTransaction::new(signed.encoded_2718())
        .expect("bounded external transaction");
    let submitted = provider
        .submit_raw(&raw)
        .await
        .expect("external wallet transfer");
    assert_eq!(submitted, EvmHash::from_bytes((*signed.hash()).into()));
    assert_eq!(
        provider.pending_nonce(&binding.sender).await.unwrap(),
        nonce + 1
    );
}

async fn generated_signer(owner: &KeystoreOwner) -> Arc<dyn Secp256k1Signer> {
    for _ in 0..4 {
        let mut candidate = Zeroizing::new([0_u8; 32]);
        getrandom::fill(candidate.as_mut()).expect("OS entropy");
        if let Ok(secret) = SecretSecp256k1Scalar::new(*candidate) {
            let signer = owner
                .import_secp256k1(
                    secret,
                    StableId::new(EVM_EIP1559_SIGNING_PURPOSE_ID).expect("signing purpose"),
                )
                .await
                .expect("key import");
            return Arc::new(signer);
        }
    }
    panic!("four entropy candidates did not contain a valid secp256k1 scalar")
}

async fn drive_to_success(
    mut step: impl AsyncFnMut() -> Result<RunView, mfm_runtime::InvocationFailure>,
) -> RunView {
    let mut last_progress = String::from("no completed invocation");
    tokio::time::timeout(PROGRESS_TIMEOUT, async {
        loop {
            match step().await {
                Ok(view) => match view.state() {
                    RunViewState::Succeeded(_) => return view,
                    RunViewState::Failed(value) => {
                        let failure = value.failure().original();
                        panic!(
                            "fixture failed at frame {}: {failure:?}",
                            view.head_sequence()
                        );
                    }
                    RunViewState::Runnable { .. }
                    | RunViewState::EffectPending { .. }
                    | RunViewState::AwaitingRecovery { .. }
                    | RunViewState::AwaitingInterpretation { .. } => {
                        last_progress = format!("runnable at frame {}", view.head_sequence());
                        eprintln!("{last_progress}");
                    }
                },
                Err(mfm_runtime::InvocationFailure::RecoveryStopped { .. })
                | Err(mfm_runtime::InvocationFailure::Execution {
                    error:
                        RuntimeError::Store(
                            mfm_store::StoreError::Unavailable(_)
                            | mfm_store::StoreError::Indeterminate(_),
                        ),
                    ..
                }) => last_progress = String::from("dependency unavailable"),
                Err(error) => panic!("unexpected fixture progress error: {error:?}"),
            }
            tokio::time::sleep(POLL_INTERVAL).await;
        }
    })
    .await
    .unwrap_or_else(|_| {
        panic!("fixture exceeded {PROGRESS_TIMEOUT:?}; last progress: {last_progress}")
    })
}

fn terminal_value(view: &RunView) -> ContractDeploymentReport {
    view.success()
        .expect("lifecycle success")
        .decode()
        .expect("checked semantic report")
}

#[tokio::test]
#[ignore = "requires the managed PostgreSQL, Reth, and pinned solc fixture"]
async fn evm_contract_effect_recovers_cold_and_accepts_external_nonce_advance() {
    let initcode = fixture_initcode();
    let admin_raw =
        std::env::var("MFM_TEST_ADMIN_POSTGRES_LOCATOR").expect("managed admin locator");
    let runtime_raw =
        std::env::var("MFM_TEST_RUNTIME_POSTGRES_LOCATOR").expect("managed runtime locator");
    let rpc_raw = std::env::var("MFM_TEST_EVM_ADAPTER_LOCATOR").expect("managed Reth locator");
    let admin_locator = AdminPostgresLocator::parse(&admin_raw).expect("admin locator");
    let runtime_locator = RuntimePostgresLocator::parse(&runtime_raw).expect("runtime locator");
    provision_postgres(&admin_locator, &runtime_locator)
        .await
        .expect("fresh base schemas");
    provision_evm_transaction_authority(&admin_locator, &runtime_locator)
        .await
        .expect("fresh transaction authority");
    let rpc_locator = EvmAdapterLocator::parse(&rpc_raw).expect("RPC locator");

    let setup_authority = PostgresEvmTransactionAuthority::connect(&runtime_locator)
        .await
        .expect("setup authority");
    let setup_provider = JsonRpcEvmProvider::connect(&rpc_locator).expect("setup provider");
    let chain = setup_provider
        .chain_instance()
        .await
        .expect("chain instance");
    let route = EvmTransactionRoute {
        chain_instance: chain,
        endpoint_ref: EvmEndpoint::new("reth-effect-e2e")
            .expect("endpoint")
            .endpoint_ref()
            .expect("endpoint ref"),
    };
    let owner = KeystoreOwner::start().expect("keystore owner");
    let signing_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let signer: Arc<dyn Secp256k1Signer> = Arc::new(ObservedSigner {
        inner: generated_signer(&owner).await,
        calls: signing_calls.clone(),
    });
    let provider_calls = Arc::new(ProviderCalls::default());
    let sender = ethereum_address(signer.public_key());
    let binding = EvmTransactionBinding {
        route,
        authority_epoch: setup_authority.authority_epoch().clone(),
        sender: sender.clone(),
    };
    drop(setup_authority);

    fund_sender(&rpc_raw, &sender).await.expect("fund wallet");
    assert_eq!(
        setup_provider
            .pending_nonce(&sender)
            .await
            .expect("initial pending nonce"),
        0
    );

    // Reuse the managed wallet fixture to observe actual authority failures before any reservation.
    use mfm_program::ClassifyError;
    use sqlx::{ConnectOptions, Connection};
    let options = admin_raw
        .parse::<sqlx::postgres::PgConnectOptions>()
        .unwrap_or_else(|_| panic!("managed admin options"))
        .disable_statement_logging();
    let mut admin = sqlx::PgConnection::connect_with(&options)
        .await
        .unwrap_or_else(|_| panic!("managed admin connection"));
    let diagnostic_run = RunId::from_digest(DigestBytes::from_array([0x59; 32]));
    let config = EvmContractConfig {
        artifact: EvmScalarContractArtifact::new(initcode).unwrap(),
        execution: EvmContractExecutionConfig::new(
            binding.clone(),
            Eip1559Options::new(nonzero(DEPLOYMENT_GAS), PRIORITY_FEE.into(), MAX_FEE.into())
                .unwrap(),
            Eip1559Options::new(
                nonzero(CONFIGURATION_GAS),
                PRIORITY_FEE.into(),
                MAX_FEE.into(),
            )
            .unwrap(),
        ),
        requested: ConfigurationValue::new("42").unwrap(),
        increment: ConfigurationValue::new("42").unwrap(),
        retry_allowance: Some(0),
        restart_allowance: Some(0),
    };
    let diagnostic_input = config.clone().into_request().unwrap();
    let (hot, resources) = runtime(
        &runtime_locator,
        &rpc_locator,
        &binding,
        signer.clone(),
        Arc::new(AtomicBool::new(true)),
        provider_calls.clone(),
    )
    .await;
    let diagnostic_program = compile(
        EntryPointId::new("mfm.test.evm-effect/authority-diagnostic@1").unwrap(),
        &Effect::<Deploy, TransactionEffect<DeploymentRequest>>::default(),
        &diagnostic_input,
        &resources,
        ProgramLimits::new(0),
    )
    .unwrap();
    sqlx::query("REVOKE SELECT ON mfm_evm_tx.nonce_reservations FROM mfm_runtime")
        .execute(&mut admin)
        .await
        .unwrap();
    let failure = hot
        .start(
            diagnostic_run.clone(),
            &diagnostic_program,
            &diagnostic_input,
        )
        .await;
    sqlx::query("GRANT SELECT ON mfm_evm_tx.nonce_reservations TO mfm_runtime")
        .execute(&mut admin)
        .await
        .unwrap();
    let Err(failure) = failure else {
        panic!("durable authority failure")
    };
    let mfm_runtime::InvocationFailure::RecoveryStopped { observed } = &failure else {
        panic!("durable authority failure")
    };
    let RunViewState::EffectPending {
        latest_failure: Some((original, _)),
        ..
    } = observed.state()
    else {
        panic!("retained authority original")
    };
    let original = original
        .decode::<mfm_evm::EvmTransactionOperationalError>()
        .unwrap();
    assert_eq!(
        original.classify(),
        mfm_program::Classification::OutcomeUnknown
    );
    let mfm_evm::EvmTransactionOperationalError::AuthorityUnavailable { cause } = &original else {
        panic!("authority owner")
    };
    assert_eq!(cause.as_value()["operation"], "authority.load");
    assert_eq!(cause.as_value()["stage"], "load_state");
    assert_eq!(cause.as_value()["sources"][0]["kind"], "database");
    assert_eq!(cause.as_value()["sources"][1]["sqlstate"], "42501");
    let hot_wire =
        serde_json::to_value(mfm_app::SerializableRunView::new(observed).unwrap()).unwrap();
    assert_eq!(
        hot_wire["state"]["latest_failure"]["error"]["value"],
        serde_json::to_value(&original).unwrap()
    );
    let envelope = serde_json::to_value(
        mfm_app::SerializableClientError::for_run(
            &mfm_app::RunRequestError::Invocation(failure),
            "recovery stopped",
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(envelope["code"], "recovery_stopped");
    assert_eq!(envelope["invocation"]["observed"], hot_wire);
    assert_eq!(
        envelope["invocation"]["error"],
        hot_wire["state"]["latest_failure"]["error"]
    );
    drop(hot);
    let (cold, resources) = runtime(
        &runtime_locator,
        &rpc_locator,
        &binding,
        signer.clone(),
        Arc::new(AtomicBool::new(true)),
        provider_calls.clone(),
    )
    .await;
    drop(diagnostic_program);
    drop(diagnostic_input);
    let diagnostic_program = cold_program(&cold, &resources, &diagnostic_run).await;
    let cold_view = cold
        .read(&diagnostic_run, &diagnostic_program)
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(mfm_app::SerializableRunView::new(&cold_view).unwrap()).unwrap(),
        hot_wire
    );
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM mfm_evm_tx.nonce_reservations")
        .fetch_one(&mut admin)
        .await
        .unwrap();
    assert_eq!(count, 0, "SQL failure must precede nonce reservation");

    let saved_epoch = binding.authority_epoch.as_bytes();
    let mut other_epoch = saved_epoch.to_vec();
    other_epoch[0] ^= 1;
    sqlx::query("UPDATE mfm_evm_tx.mfm_evm_tx_schema SET authority_epoch = $1")
        .bind(other_epoch.as_slice())
        .execute(&mut admin)
        .await
        .unwrap();
    let internal = cold.resume(&diagnostic_run, &diagnostic_program).await;
    sqlx::query("UPDATE mfm_evm_tx.mfm_evm_tx_schema SET authority_epoch = $1")
        .bind(saved_epoch)
        .execute(&mut admin)
        .await
        .unwrap();
    let Err(failure) = internal else {
        panic!("retained epoch mismatch")
    };
    let mfm_runtime::InvocationFailure::Execution {
        error: RuntimeError::Native { cause, .. },
        last_observed: Some(previous),
        ..
    } = &failure
    else {
        panic!("internal invocation with previous observation")
    };
    assert_eq!(cause.code(), "authority_internal");
    assert_eq!(cause.operation(), "authority.load");
    assert_eq!(
        cause.details().as_value()["check"],
        "schema or epoch binding"
    );
    assert_eq!(previous.head_digest(), cold_view.head_digest());
    assert_eq!(previous.head_sequence(), cold_view.head_sequence());
    let wire = serde_json::to_value(
        mfm_app::SerializableClientError::for_run(
            &mfm_app::RunRequestError::Invocation(failure),
            "authority invocation",
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        wire["invocation"]["cause"]["native"]["cause"]["code"],
        "authority_internal"
    );
    let after = cold
        .read(&diagnostic_run, &diagnostic_program)
        .await
        .unwrap();
    assert_eq!(after.head_sequence(), cold_view.head_sequence());
    assert_eq!(after.head_digest(), cold_view.head_digest());
    assert_eq!(
        serde_json::to_value(mfm_app::SerializableRunView::new(&after).unwrap()).unwrap(),
        hot_wire
    );
    drop(cold);
    drop(admin);

    let encoded_config = serde_json::to_value(&config).unwrap();
    let mut malformed_config = encoded_config.clone();
    malformed_config["requested"] = serde_json::json!("042");
    assert!(serde_json::from_value::<EvmContractConfig>(malformed_config).is_err());
    let config: EvmContractConfig = serde_json::from_value(encoded_config).unwrap();
    let expected_execution = config.execution.clone();
    let input = config.into_request().unwrap();
    assert_eq!(input.retry_allowance(), Some(0));
    assert_eq!(input.restart_allowance(), Some(0));
    assert_eq!(
        input
            .execution()
            .native()
            .decode::<EvmContractExecutionConfig>()
            .unwrap(),
        expected_execution
    );
    assert_eq!(
        input.execution().transaction_implementation().as_str(),
        "mfm.evm.transaction@1"
    );
    assert_eq!(
        input.execution().read_implementation().as_str(),
        "mfm.evm.contract-read@1"
    );
    let run_id = RunId::from_digest(DigestBytes::from_array([0x5a; 32]));
    let consumed = Arc::new(AtomicBool::new(false));
    let (initial_runtime, resources) = runtime(
        &runtime_locator,
        &rpc_locator,
        &binding,
        signer.clone(),
        consumed.clone(),
        provider_calls.clone(),
    )
    .await;
    let program = compile(
        EntryPointId::new("mfm.test.evm-effect/run@1").unwrap(),
        &ContractDeploymentLifecycle::default(),
        &input,
        &resources,
        ProgramLimits::new(1),
    )
    .unwrap();
    let initial = tokio::time::timeout(
        PROGRESS_TIMEOUT,
        initial_runtime.start(run_id.clone(), &program, &input),
    )
    .await
    .expect("reservation deadline");
    assert!(matches!(
        initial,
        Err(mfm_runtime::InvocationFailure::RecoveryStopped { .. })
    ));
    assert!(consumed.load(Ordering::SeqCst));
    assert_eq!(setup_provider.pending_nonce(&sender).await.unwrap(), 0);
    assert_eq!(signing_calls.load(Ordering::SeqCst), 0);
    assert!(provider_calls.submitted.lock().unwrap().is_empty());
    drop(input);
    drop(program);
    drop(resources);
    drop(initial_runtime);

    // Cancel after the real node accepts a signed transaction while interval mining is delayed.
    // The exact prepared command already has durable authority; dropping this future records none
    // of its unacknowledged work and leaves the keystore owner alive for cold reconstruction.
    let (interrupted, resources) = runtime(
        &runtime_locator,
        &rpc_locator,
        &binding,
        signer.clone(),
        consumed.clone(),
        provider_calls.clone(),
    )
    .await;
    let program = cold_program(&interrupted, &resources, &run_id).await;
    tokio::time::timeout(PROGRESS_TIMEOUT, async {
        let progress = interrupted.resume(&run_id, &program);
        tokio::pin!(progress);
        tokio::select! {
            _ = provider_calls.broadcast.notified() => {},
            result = &mut progress => panic!("expected cancellation after broadcast; error: {:?}", result.err()),
        }
    })
    .await
    .expect("broadcast deadline");
    let pending = interrupted.read(&run_id, &program).await.unwrap();
    let RunViewState::EffectPending { effect, .. } = pending.state() else {
        panic!("broadcast cancellation must preserve command authority")
    };
    let prepared = effect
        .command()
        .decode::<PreparedTransaction<DeploymentRequest>>()
        .unwrap();
    let native = prepared
        .native()
        .decode::<mfm_evm::PreparedEvmTransaction>()
        .unwrap();
    assert_eq!(native.reserved().reservation().nonce(), 0);
    assert_eq!(
        native.reserved().command(),
        &mfm_evm::EvmTransactionRecipe::command(prepared.request()).unwrap()
    );
    assert_eq!(signing_calls.load(Ordering::SeqCst), 1);
    assert_eq!(provider_calls.submitted.lock().unwrap().len(), 1);
    drop(prepared);
    drop(native);
    drop(pending);
    drop(program);
    drop(resources);
    drop(interrupted);

    let terminal = drive_to_success(async || {
        let (runtime, resources) = runtime(
            &runtime_locator,
            &rpc_locator,
            &binding,
            signer.clone(),
            consumed.clone(),
            provider_calls.clone(),
        )
        .await;
        let program = cold_program(&runtime, &resources, &run_id).await;
        runtime.resume(&run_id, &program).await
    })
    .await;
    let report = terminal_value(&terminal);
    assert_eq!(report.requested_value().to_string(), "42");
    assert_eq!(report.effective_value().to_string(), "42");
    assert_eq!(report.observed_value().to_string(), "42");
    let deployment = report
        .deployment()
        .original()
        .decode::<mfm_evm::EvmTransactionSettlement>()
        .unwrap();
    let configuration = report
        .configuration()
        .original()
        .decode::<mfm_evm::EvmTransactionSettlement>()
        .unwrap();
    assert_eq!(deployment.nonce(), 0);
    assert_eq!(configuration.nonce(), 1);
    assert_eq!(
        &report
            .deployment()
            .transaction()
            .native()
            .decode::<EvmHash>()
            .unwrap(),
        deployment.transaction_hash()
    );
    assert_eq!(
        &report
            .configuration()
            .transaction()
            .native()
            .decode::<EvmHash>()
            .unwrap(),
        configuration.transaction_hash()
    );
    let observation = report
        .observation()
        .original()
        .decode::<mfm_evm::AnchoredContractCallEvidence>()
        .unwrap();
    let mfm_evm::AnchoredContractCallEvidence::Returned {
        result: observation,
        ..
    } = observation
    else {
        panic!("authenticated anchored scalar result")
    };
    assert_eq!(observation.anchor(), configuration.block_anchor());
    assert_eq!(observation.return_bytes().len(), 32);
    assert_eq!(observation.return_bytes()[31], CONFIGURED_VALUE as u8);
    assert_eq!(setup_provider.pending_nonce(&sender).await.unwrap(), 2);
    assert_eq!(
        signing_calls.load(Ordering::SeqCst),
        2,
        "cold recovery never re-signs the prepared deployment"
    );

    // The maintained child also runs from a real deployed predecessor and calls its existing address.
    external_wallet_transfer(&setup_provider, signer.as_ref(), &binding, 2).await;
    let fresh_input = DeployedContract::new(
        report.request().clone(),
        report.effective_value().clone(),
        report.deployment().clone(),
    )
    .unwrap();
    let (cold_runtime, resources) = runtime(
        &runtime_locator,
        &rpc_locator,
        &binding,
        signer.clone(),
        consumed.clone(),
        provider_calls.clone(),
    )
    .await;
    let fresh_run = RunId::from_digest(DigestBytes::from_array([0x5b; 32]));
    let fresh_program = compile(
        EntryPointId::new("mfm.test.evm-effect/existing-contract@1").unwrap(),
        &ConfigureAndObserve::default(),
        &fresh_input,
        &resources,
        ProgramLimits::new(1),
    )
    .unwrap();
    let existing =
        drive_to_success(
            async || match cold_runtime.resume(&fresh_run, &fresh_program).await {
                Err(mfm_runtime::InvocationFailure::Execution {
                    error: RuntimeError::Absent,
                    ..
                }) => {
                    cold_runtime
                        .start(fresh_run.clone(), &fresh_program, &fresh_input)
                        .await
                }
                progress => progress,
            },
        )
        .await;
    let observed = existing
        .success()
        .unwrap()
        .decode::<ObservedConfiguration>()
        .unwrap();
    let existing_native = observed
        .configured()
        .configuration()
        .original()
        .decode::<mfm_evm::EvmTransactionSettlement>()
        .unwrap();
    assert_eq!(existing_native.nonce(), 3);
    assert_eq!(
        observed.configured().configured().contract().unwrap(),
        fresh_input.contract().unwrap()
    );
    assert!(
        matches!(observed.observation().outcome(), ContractValueOutcome::Observed { value, .. } if value.to_string() == "42")
    );
    assert_eq!(setup_provider.pending_nonce(&sender).await.unwrap(), 4);

    let composed_run = RunId::from_digest(DigestBytes::from_array([0x5d; 32]));
    let composed_input = report.request().clone();
    let composed = compile(
        EntryPointId::new("mfm.test.evm-effect/composed@1").unwrap(),
        &Operation::<_, LifecycleDefaults>::from((
            Effect::<Deploy, TransactionEffect<DeploymentRequest>>::default(),
            Pure::<CheckedAddConfigurationValue>::default(),
            ConfigureAndObserve::default(),
            Pure::<Validate>::default(),
            Pure::<Report>::default(),
        )),
        &composed_input,
        &resources,
        ProgramLimits::new(1),
    )
    .unwrap();
    let started = cold_runtime
        .start(composed_run.clone(), &composed, &composed_input)
        .await;
    assert!(started.is_ok(), "composed admission must succeed");
    drop(composed_input);
    drop(composed);
    let eighty_four = drive_to_success(async || {
        let (runtime, resources) = runtime(
            &runtime_locator,
            &rpc_locator,
            &binding,
            signer.clone(),
            consumed.clone(),
            provider_calls.clone(),
        )
        .await;
        let program = cold_program(&runtime, &resources, &composed_run).await;
        runtime.resume(&composed_run, &program).await
    })
    .await;
    let composed_report = terminal_value(&eighty_four);
    assert_eq!(composed_report.requested_value().to_string(), "42");
    assert_eq!(composed_report.effective_value().to_string(), "84");
    assert_eq!(composed_report.observed_value().to_string(), "84");
    assert_eq!(
        composed_report
            .deployment()
            .original()
            .decode::<mfm_evm::EvmTransactionSettlement>()
            .unwrap()
            .nonce(),
        4
    );
    assert_eq!(
        composed_report
            .configuration()
            .original()
            .decode::<mfm_evm::EvmTransactionSettlement>()
            .unwrap()
            .nonce(),
        5
    );
    assert_eq!(setup_provider.pending_nonce(&sender).await.unwrap(), 6);
    assert!(provider_calls.absent_receipts.load(Ordering::SeqCst) > 0);
    assert!(
        provider_calls.known_transactions.load(Ordering::SeqCst) > 0,
        "delayed mining must exercise known Pending transactions"
    );
    {
        let submitted = provider_calls.submitted.lock().unwrap();
        assert_eq!(
            submitted.len(),
            5,
            "one accepted submission per native transaction"
        );
        let unique: std::collections::BTreeSet<_> = submitted.iter().collect();
        assert_eq!(
            unique.len(),
            5,
            "known transactions must not be rebroadcast"
        );
    }
    assert_eq!(
        signing_calls.load(Ordering::SeqCst),
        6,
        "five native signatures plus the external wallet transfer"
    );
    for (run, expected) in [(&run_id, &terminal), (&composed_run, &eighty_four)] {
        let program = cold_program(&cold_runtime, &resources, run).await;
        let read = cold_runtime.read(run, &program).await.unwrap();
        let resumed = cold_runtime.resume(run, &program).await.unwrap();
        for view in [read, resumed] {
            assert_eq!(view.head_sequence(), expected.head_sequence());
            assert_eq!(view.head_digest(), expected.head_digest());
            assert_eq!(view.success(), expected.success());
        }
    }
    assert_eq!(setup_provider.pending_nonce(&sender).await.unwrap(), 6);
    drop(diagnostic_program);
    // Keep this additional recovery future off the already large managed fixture's stack.
    Box::pin(async {
        // Finish the existing standalone deployment after resource restoration, without readmission.
        // The saved failure prefix and command authority must survive the successful continuation.
        let backend = PostgresBackend::connect(&runtime_locator).await.unwrap();
        let RunViewState::EffectPending { effect, .. } = cold_view.state() else {
            panic!("standalone deployment retains the failed command");
        };
        let retained_effect = serde_json::to_value(effect).unwrap();
        let mut failure_prefix = Vec::new();
        for sequence in 1..=cold_view.head_sequence() {
            let snapshot = backend
                .load_run(&diagnostic_run, Some(sequence))
                .await
                .unwrap()
                .unwrap();
            failure_prefix.push(snapshot.probe().unwrap().clone());
        }
        let (restored, resources) = runtime(
            &runtime_locator,
            &rpc_locator,
            &binding,
            signer.clone(),
            Arc::new(AtomicBool::new(true)),
            provider_calls.clone(),
        )
        .await;
        let program = cold_program(&restored, &resources, &diagnostic_run).await;
        let standalone =
            drive_to_success(async || restored.resume(&diagnostic_run, &program).await).await;
        assert_eq!(standalone.run_id(), &diagnostic_run);
        let deployed = standalone
            .success()
            .unwrap()
            .decode::<DeployedContract>()
            .unwrap();
        assert_eq!(deployed.effective().to_string(), "42");
        let settlement = deployed
            .deployment()
            .original()
            .decode::<mfm_evm::EvmTransactionSettlement>()
            .unwrap();
        assert_eq!(settlement.nonce(), 6);
        let receipt = setup_provider
            .receipt(settlement.transaction_hash())
            .await
            .unwrap()
            .unwrap();
        let mfm_evm_live::ProviderReceiptResult::SuccessCreate { contract_address } =
            receipt.result()
        else {
            panic!("independent receipt confirms standalone deployment");
        };
        assert_eq!(
            settlement.outcome(),
            &mfm_evm::EvmTransactionOutcome::Created {
                created_address: contract_address.clone(),
            }
        );
        assert_eq!(receipt.block_anchor(), settlement.block_anchor());
        let mut settled_retained_command = false;
        for sequence in 1..=standalone.head_sequence() {
            let snapshot = backend
                .load_run(&diagnostic_run, Some(sequence))
                .await
                .unwrap()
                .unwrap();
            let bytes = snapshot.probe().unwrap();
            if sequence <= cold_view.head_sequence() {
                assert_eq!(bytes, &failure_prefix[(sequence - 1) as usize]);
            } else {
                let frame = mfm_journal::decode_frame(bytes).unwrap();
                let payload: serde_json::Value =
                    serde_json::from_slice(frame.payload().as_bytes()).unwrap();
                if payload["operation"]["effect_settled"]["effect"] == retained_effect {
                    settled_retained_command = true;
                }
            }
        }
        assert!(
            settled_retained_command,
            "settlement reuses the exact failed command and EffectId"
        );
        assert_eq!(provider_calls.submitted.lock().unwrap().len(), 6);
        assert_eq!(signing_calls.load(Ordering::SeqCst), 7);
        let replayed = restored.resume(&diagnostic_run, &program).await.unwrap();
        assert_eq!(replayed.head_digest(), standalone.head_digest());
        assert_eq!(replayed.success(), standalone.success());
        assert_eq!(provider_calls.submitted.lock().unwrap().len(), 6);
        assert_eq!(signing_calls.load(Ordering::SeqCst), 7);
        assert_eq!(setup_provider.pending_nonce(&sender).await.unwrap(), 7);
    })
    .await;
    owner.shutdown().await.expect("keystore shutdown");
    let signing_run = RunId::from_digest(DigestBytes::from_array([0x5c; 32]));
    let (hot, resources) = runtime(
        &runtime_locator,
        &rpc_locator,
        &binding,
        signer.clone(),
        Arc::new(AtomicBool::new(true)),
        provider_calls.clone(),
    )
    .await;
    let fresh_program = load(fresh_program.canonical_bytes(), &resources).unwrap();
    let failed = hot
        .start(signing_run.clone(), &fresh_program, &fresh_input)
        .await;
    let Err(mfm_runtime::InvocationFailure::RecoveryStopped { observed }) = failed else {
        panic!("durable closed signer failure")
    };
    let RunViewState::EffectPending {
        latest_failure: Some((original, _)),
        ..
    } = observed.state()
    else {
        panic!("retained signer original")
    };
    let original = original
        .decode::<mfm_evm::EvmTransactionOperationalError>()
        .unwrap();
    assert_eq!(original.classify(), mfm_program::Classification::Retryable);
    let mfm_evm::EvmTransactionOperationalError::SignerUnavailable { cause } = &original else {
        panic!("signer owner")
    };
    assert_eq!(
        cause.as_value(),
        &serde_json::json!({"operation": "sign", "stage": "request_send", "kind": "channel_closed", "message": "channel closed"})
    );
    let hot_wire =
        serde_json::to_value(mfm_app::SerializableRunView::new(&observed).unwrap()).unwrap();
    assert_eq!(
        hot_wire["state"]["latest_failure"]["error"]["value"],
        serde_json::to_value(&original).unwrap()
    );
    drop(hot);
    let (cold, resources) = runtime(
        &runtime_locator,
        &rpc_locator,
        &binding,
        signer,
        Arc::new(AtomicBool::new(true)),
        provider_calls.clone(),
    )
    .await;
    let program = cold_program(&cold, &resources, &signing_run).await;
    let cold_view = cold.read(&signing_run, &program).await.unwrap();
    assert_eq!(
        serde_json::to_value(mfm_app::SerializableRunView::new(&cold_view).unwrap()).unwrap(),
        hot_wire
    );
    assert_eq!(setup_provider.pending_nonce(&sender).await.unwrap(), 7);
}
