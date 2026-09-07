//! Managed PostgreSQL/Reth coverage of lost reservation acknowledgement, reconstructed Runtime
//! recovery, external nonce advancement, and unchanged terminal history. Keystore custody stays
//! alive across Runtime reconstruction. Preparation recovery without signing and Journal append
//! faults are exercised in `src/transaction_tests.rs`; this test does not count provider calls.

use std::io::Read;
use std::marker::PhantomData;
use std::num::NonZeroU64;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use alloy_primitives::U256;
use mfm_evm::custody::{
    AuthorityError, AuthorityFuture, EvmTransactionAuthority, LoadedTransaction, PreparedRecord,
    Reservation,
};
use mfm_evm::{
    CheckedCallPlan, CheckedCreatePlan, CheckedObservationPlan, CheckedTargetCallPlan, EvmAddress,
    EvmAnchoredContractCallRead, EvmAuthorityEpoch, EvmEndpoint, EvmHash, EvmTransactionBinding,
    EvmTransactionRoute, EvmU256, ReadAnchoredContractCall,
};
use mfm_evm_live::{
    ethereum_address, register_evm_anchored_contract_calls, register_evm_transaction_adapters,
    register_evm_transaction_states, EvmAdapterLocator, EvmReadProvider, EvmTransactionProvider,
    JsonRpcEvmProvider, EVM_EIP1559_SIGNING_PURPOSE_ID,
};
use mfm_ids::{ContentRef, DigestBytes, EffectId, EntryPointId, RunId, StableId};
use mfm_keystore::{KeystoreOwner, SecretSecp256k1Scalar};
use mfm_program::{
    expand_program, Operation, OperationExpansion, ProgramError, ProposedStateOutcome, PureState,
    State,
};
use mfm_program_derive::MfmValue;
use mfm_runtime::{RunView, RunViewState, Runtime, RuntimeAssemblyBuilder, RuntimeError};
use mfm_signing::Secp256k1Signer;
use mfm_storage_postgres::{
    provision_evm_transaction_authority, provision_postgres, AdminPostgresLocator, PostgresBackend,
    PostgresEvmTransactionAuthority, RuntimePostgresLocator,
};
use mfm_store::Store;
use mfm_values::MfmValue;
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

const MAX_INITCODE_BYTES: usize = 49_152;
const MAX_FUNDING_RESPONSE_BYTES: usize = 16 * 1024;
const PROGRESS_TIMEOUT: Duration = Duration::from_secs(60);
const POLL_INTERVAL: Duration = Duration::from_millis(100);
const RPC_TIMEOUT: Duration = Duration::from_secs(10);
const CONFIGURED_VALUE: u64 = 42;
const DEPLOYMENT_GAS: u64 = 2_000_000;
const CONFIGURATION_GAS: u64 = 200_000;
const PRIORITY_FEE: u64 = 1_000_000_000;
const MAX_FEE: u64 = 10_000_000_000;
const FUNDING_WEI_HEX: &str = "0xde0b6b3a7640000";
const CONFIGURE_SELECTOR: [u8; 4] = [0x1e, 0xb2, 0x5e, 0x0a];
const VALUE_SELECTOR: [u8; 4] = [0x3f, 0xa4, 0xf2, 0x45];

#[path = "support/contract_workflow.rs"]
mod workflow;
use workflow::*;

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
                return Err(AuthorityError::Unavailable);
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
) -> Runtime {
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
        binding.authority_epoch()
    );
    let provider = Arc::new(JsonRpcEvmProvider::connect(rpc_locator).expect("provider"));
    let authority: Arc<dyn EvmTransactionAuthority> = Arc::new(ReservationAcknowledgementFault {
        inner: transaction_authority,
        consumed,
    });
    let transaction_provider: Arc<dyn EvmTransactionProvider> = provider.clone();
    let read_provider: Arc<dyn EvmReadProvider> = provider;

    let mut builder = RuntimeAssemblyBuilder::new().expect("builder");
    register_fixture_states(&mut builder).expect("fixture State ABIs");
    register_evm_transaction_states::<WalletInitial, WalletRecipe>(&mut builder)
        .expect("wallet State ABIs");
    register_evm_transaction_adapters(
        &mut builder,
        binding.clone(),
        signer,
        authority,
        transaction_provider,
    )
    .expect("transaction adapter");
    register_evm_anchored_contract_calls(&mut builder, binding.route().clone(), read_provider)
        .expect("anchored adapter");
    let store: Arc<dyn Store> = backend;
    Runtime::new(builder.finish(), store)
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

fn fixture_configure_calldata() -> Vec<u8> {
    let mut calldata = Vec::with_capacity(36);
    calldata.extend_from_slice(&CONFIGURE_SELECTOR);
    calldata.extend_from_slice(&abi_word(CONFIGURED_VALUE));
    calldata
}

fn abi_word(value: u64) -> [u8; 32] {
    let mut word = [0_u8; 32];
    word[24..].copy_from_slice(&value.to_be_bytes());
    word
}

fn decode_fixture_value(return_bytes: &[u8]) -> Option<EvmU256> {
    let word = <[u8; 32]>::try_from(return_bytes).ok()?;
    EvmU256::new(U256::from_be_bytes(word).to_string()).ok()
}

#[derive(Debug, Clone, Copy, thiserror::Error)]
#[error("Reth development funding is unavailable")]
struct FundingError;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AccountsResponse {
    jsonrpc: String,
    id: u64,
    result: Vec<EvmAddress>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FundingResponse {
    jsonrpc: String,
    id: u64,
    result: EvmHash,
}

async fn fund_sender(locator: &str, sender: &EvmAddress) -> Result<(), FundingError> {
    let url = reqwest::Url::parse(locator).map_err(|_| FundingError)?;
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .referer(false)
        .retry(reqwest::retry::never())
        .timeout(RPC_TIMEOUT)
        .build()
        .map_err(|_| FundingError)?;
    let accounts_response = client
        .post(url.clone())
        .json(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "eth_accounts",
            "params": [],
        }))
        .send()
        .await
        .map_err(|_| FundingError)?;
    let accounts: AccountsResponse =
        serde_json::from_slice(&funding_response_body(accounts_response).await?)
            .map_err(|_| FundingError)?;
    if accounts.jsonrpc != "2.0" || accounts.id != 1 {
        return Err(FundingError);
    }
    let source = accounts.result.first().ok_or(FundingError)?;
    let funding_response = client
        .post(url)
        .json(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "eth_sendTransaction",
            "params": [{
                "from": source,
                "gas": format!("{:#x}", 21_000_u64),
                "maxFeePerGas": format!("{MAX_FEE:#x}"),
                "maxPriorityFeePerGas": format!("{PRIORITY_FEE:#x}"),
                "to": sender,
                "value": FUNDING_WEI_HEX,
            }],
        }))
        .send()
        .await
        .map_err(|_| FundingError)?;
    let funding: FundingResponse =
        serde_json::from_slice(&funding_response_body(funding_response).await?)
            .map_err(|_| FundingError)?;
    if funding.jsonrpc != "2.0" || funding.id != 1 {
        return Err(FundingError);
    }
    let _transaction_hash = funding.result;
    Ok(())
}

async fn funding_response_body(mut response: reqwest::Response) -> Result<Vec<u8>, FundingError> {
    if !response.status().is_success()
        || response
            .content_length()
            .is_some_and(|length| length > MAX_FUNDING_RESPONSE_BYTES as u64)
    {
        return Err(FundingError);
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| FundingError)? {
        if body.len() + chunk.len() > MAX_FUNDING_RESPONSE_BYTES {
            return Err(FundingError);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
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
        chain_id: binding.route().chain_instance().chain_id().get(),
        nonce,
        gas_limit: 21_000,
        max_fee_per_gas: u128::from(MAX_FEE),
        max_priority_fee_per_gas: u128::from(PRIORITY_FEE),
        to: TxKind::Call(Address::from(*binding.sender().as_bytes())),
        ..Default::default()
    };
    let digest = mfm_signing::SigningDigest::from_bytes(transaction.signature_hash().into());
    let signature = signer
        .sign(digest)
        .await
        .expect("external wallet signature");
    let bytes = signature.as_bytes();
    let signature = Signature::from_scalars_and_parity(
        alloy_primitives::B256::from_slice(&bytes[..32]),
        alloy_primitives::B256::from_slice(&bytes[32..]),
        signature.recovery_id() == 1,
    );
    let signed = transaction.into_signed(signature);
    let raw = mfm_evm::custody::ExactRawTransaction::new(signed.encoded_2718())
        .expect("bounded external transaction");
    let submitted = provider
        .submit_raw(&raw)
        .await
        .expect("external wallet transfer");
    assert_eq!(submitted, EvmHash::from_bytes((*signed.hash()).into()));
    assert_eq!(
        provider.pending_nonce(binding.sender()).await.unwrap(),
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

async fn drive_to_success<F: serde::de::DeserializeOwned + std::fmt::Debug>(
    mut step: impl AsyncFnMut() -> mfm_runtime::Result<RunView>,
) -> RunView {
    let mut last_progress = String::from("no completed invocation");
    tokio::time::timeout(PROGRESS_TIMEOUT, async {
        loop {
            match step().await {
                Ok(view) => match view.state() {
                    RunViewState::Succeeded(_) => return view,
                    RunViewState::Failed(value) => {
                        let failure: F = serde_json::from_slice(value.canonical_bytes())
                            .expect("typed fixture failure");
                        panic!(
                            "fixture failed at frame {}: {failure:?}",
                            view.head_sequence()
                        );
                    }
                    RunViewState::Runnable => {
                        last_progress = format!("runnable at frame {}", view.head_sequence())
                    }
                },
                Err(RuntimeError::Unavailable) => {
                    last_progress = String::from("dependency unavailable")
                }
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

fn terminal_value(view: &RunView) -> FixtureReport {
    let RunViewState::Succeeded(value) = view.state() else {
        panic!("effect fixture must succeed")
    };
    serde_json::from_slice(value.canonical_bytes()).expect("typed final value")
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
    let route = EvmTransactionRoute::new(
        chain,
        EvmEndpoint::new("reth-effect-e2e")
            .expect("endpoint")
            .endpoint_ref()
            .expect("endpoint ref"),
    );
    let owner = KeystoreOwner::start().expect("keystore owner");
    let signer = generated_signer(&owner).await;
    let sender = ethereum_address(signer.public_key());
    let binding = EvmTransactionBinding::new(
        route,
        setup_authority.authority_epoch().clone(),
        sender.clone(),
    );
    drop(setup_authority);

    fund_sender(&rpc_raw, &sender).await.expect("fund wallet");
    assert_eq!(
        setup_provider
            .pending_nonce(&sender)
            .await
            .expect("initial pending nonce"),
        0
    );

    let deployment = CheckedCreatePlan::new(
        binding.clone(),
        initcode,
        EvmU256::from_u64(0),
        nonzero(DEPLOYMENT_GAS),
        EvmU256::from_u64(PRIORITY_FEE),
        EvmU256::from_u64(MAX_FEE),
    )
    .expect("checked deployment");
    let configuration = CheckedCallPlan::new(
        binding.clone(),
        fixture_configure_calldata(),
        EvmU256::from_u64(0),
        nonzero(CONFIGURATION_GAS),
        EvmU256::from_u64(PRIORITY_FEE),
        EvmU256::from_u64(MAX_FEE),
    )
    .expect("checked configuration");
    let observation = CheckedObservationPlan::new(binding.route().clone(), VALUE_SELECTOR.to_vec())
        .expect("checked observation");
    let input = ContractWorkflow {
        request: FixtureRequest { label: 17 },
        deployment,
        configuration,
        observation,
    };
    let expected_input = input.clone();
    let program = expand_program(
        EntryPointId::new("mfm.test.evm-effect/run@1").expect("entry point"),
        &EffectFixtureOperation {
            binding: binding.clone(),
        },
    )
    .expect("fixture Program");
    let run_id = RunId::from_digest(DigestBytes::from_array([0x5a; 32]));
    let consumed = Arc::new(AtomicBool::new(false));
    let initial_runtime = runtime(
        &runtime_locator,
        &rpc_locator,
        &binding,
        signer.clone(),
        consumed.clone(),
    )
    .await;
    let initial = tokio::time::timeout(
        PROGRESS_TIMEOUT,
        initial_runtime.start(run_id.clone(), program, input),
    )
    .await
    .expect("initial reservation deadline");
    assert!(matches!(initial, Err(RuntimeError::Unavailable)));
    assert!(consumed.load(Ordering::SeqCst));
    assert_eq!(
        setup_provider
            .pending_nonce(&sender)
            .await
            .expect("nonce before broadcast"),
        0
    );
    drop(initial_runtime);

    let terminal = drive_to_success::<FixtureFailure>(async || {
        let runtime = runtime(
            &runtime_locator,
            &rpc_locator,
            &binding,
            signer.clone(),
            consumed.clone(),
        )
        .await;
        runtime.resume(&run_id).await
    })
    .await;
    assert_eq!(
        terminal_value(&terminal).decoded(),
        &EvmU256::from_u64(CONFIGURED_VALUE)
    );
    let report = terminal_value(&terminal);
    assert_eq!(report.context().request, expected_input.request);
    assert_eq!(
        report.context().deployment.command(),
        &expected_input.deployment.command()
    );
    assert_eq!(
        report.context().configuration.command(),
        &expected_input.configuration.command_for(
            report
                .context()
                .deployment
                .outcome()
                .created_address()
                .clone()
        )
    );
    assert_eq!(report.context().deployment.reservation().nonce(), 0);
    assert_eq!(report.context().configuration.reservation().nonce(), 1);
    assert_eq!(
        report.context().deployment.preparation().transaction_hash(),
        report.context().deployment.settlement().transaction_hash()
    );
    assert_eq!(
        report
            .context()
            .configuration
            .preparation()
            .transaction_hash(),
        report
            .context()
            .configuration
            .settlement()
            .transaction_hash()
    );
    assert_eq!(
        report.context().observation.intent(),
        &expected_input.observation.intent_for(
            report.context().configuration.outcome().target().clone(),
            report
                .context()
                .configuration
                .settlement()
                .block_anchor()
                .clone()
        )
    );
    assert_eq!(
        report
            .context()
            .observation
            .result()
            .unwrap()
            .return_bytes(),
        &abi_word(CONFIGURED_VALUE)
    );
    let final_nonce = setup_provider
        .pending_nonce(&sender)
        .await
        .expect("final pending nonce");
    assert_eq!(final_nonce, 2);

    external_wallet_transfer(&setup_provider, signer.as_ref(), &binding, final_nonce).await;
    let fresh_plan = CheckedTargetCallPlan::new(
        CheckedCallPlan::new(
            binding.clone(),
            Vec::new(),
            EvmU256::from_u64(0),
            nonzero(21_000),
            EvmU256::from_u64(PRIORITY_FEE),
            EvmU256::from_u64(MAX_FEE),
        )
        .unwrap(),
        sender.clone(),
    );
    let cold_runtime = runtime(&runtime_locator, &rpc_locator, &binding, signer, consumed).await;
    let fresh_run = RunId::from_digest(DigestBytes::from_array([0x5b; 32]));
    let fresh_program = expand_program(
        EntryPointId::new("mfm.test.evm-effect/wallet-call@1").unwrap(),
        &WalletTransaction::new(binding.clone()),
    )
    .unwrap();
    // Resume first so Unavailable from admission is retried without assuming genesis committed.
    let wallet_terminal =
        drive_to_success::<WalletFailure>(async || match cold_runtime.resume(&fresh_run).await {
            Err(RuntimeError::Absent) => {
                cold_runtime
                    .start(
                        fresh_run.clone(),
                        fresh_program.clone(),
                        WalletContext {
                            transaction: fresh_plan.clone(),
                            label: 18,
                        },
                    )
                    .await
            }
            progress => progress,
        })
        .await;
    let RunViewState::Succeeded(wallet_value) = wallet_terminal.state() else {
        panic!("wallet follow-up success")
    };
    let wallet_report: WalletReport =
        serde_json::from_slice(wallet_value.canonical_bytes()).unwrap();
    assert_eq!(wallet_report.label, 18);
    assert_eq!(wallet_report.transaction.command(), &fresh_plan.command());
    assert_eq!(wallet_report.transaction.reservation().nonce(), 3);
    let final_nonce = final_nonce + 2;
    assert_eq!(
        setup_provider.pending_nonce(&sender).await.unwrap(),
        final_nonce
    );
    let cold_read = cold_runtime.read(&run_id).await.expect("cold read");
    assert_eq!(cold_read.head_sequence(), terminal.head_sequence());
    assert_eq!(cold_read.head_digest(), terminal.head_digest());
    assert_eq!(terminal_value(&cold_read), report);
    let cold_resume = cold_runtime.resume(&run_id).await.expect("terminal resume");
    assert_eq!(cold_resume.head_sequence(), terminal.head_sequence());
    assert_eq!(cold_resume.head_digest(), terminal.head_digest());
    assert_eq!(terminal_value(&cold_resume), report);
    assert_eq!(
        setup_provider
            .pending_nonce(&sender)
            .await
            .expect("cold replay nonce"),
        final_nonce
    );

    owner.shutdown().await.expect("keystore shutdown");
}

#[path = "support/accumulating_contract.rs"]
mod accumulating_contract;
#[path = "support/context_capacity.rs"]
mod context_capacity;
