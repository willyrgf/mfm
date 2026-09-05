//! Managed PostgreSQL/Reth coverage of lost reservation acknowledgement, reconstructed Runtime
//! recovery, external nonce advancement, and unchanged terminal history. Keystore custody stays
//! alive across Runtime reconstruction. Preparation recovery without signing and Journal append
//! faults are exercised in `src/transaction_tests.rs`; this test does not count provider calls.

use mfm_evm::{
    EvmNonceReservationEffect, EvmTransactionPreparationEffect, PrepareEvmTransaction,
    ProjectEvmTransactionOutcome, ReserveEvmNonce,
};
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
    AnchoredContractCallCompletion, AnchoredContractCallContext, AnchoredContractCallFailure,
    Eip1559TransactionCommand, EvmAddress, EvmAnchoredContractCallRead, EvmAuthorityEpoch,
    EvmEndpoint, EvmHash, EvmTransactionBinding, EvmTransactionCompletion, EvmTransactionContext,
    EvmTransactionEffect, EvmTransactionReversion, EvmTransactionRoute, EvmU256,
    ExecuteEvmTransaction, ReadAnchoredContractCall,
};
use mfm_evm_live::{
    ethereum_address, register_evm_anchored_contract_calls, register_evm_transaction_adapters,
    EvmAdapterLocator, EvmReadProvider, EvmTransactionProvider, JsonRpcEvmProvider,
    EVM_EIP1559_SIGNING_PURPOSE_ID,
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

type Deployment = EvmTransactionCompletion<EvmU256>;
type Configuration = EvmTransactionCompletion<Deployment>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum FixtureFailure {
    MissingCreatedAddress,
    InvalidConfigurationCommand,
    MissingCallTarget,
    InvalidObservationContext,
    InvalidReturnData,
    DeploymentReverted,
    ConfigurationReverted,
    ObservationFailed,
}

impl From<EvmTransactionReversion<EvmU256>> for FixtureFailure {
    fn from(_: EvmTransactionReversion<EvmU256>) -> Self {
        Self::DeploymentReverted
    }
}
impl From<EvmTransactionReversion<Deployment>> for FixtureFailure {
    fn from(_: EvmTransactionReversion<Deployment>) -> Self {
        Self::ConfigurationReverted
    }
}
impl From<AnchoredContractCallFailure<Configuration>> for FixtureFailure {
    fn from(_: AnchoredContractCallFailure<Configuration>) -> Self {
        Self::ObservationFailed
    }
}

fn nonzero(value: u64) -> NonZeroU64 {
    NonZeroU64::new(value).expect("nonzero fixture")
}

struct PrepareConfiguration;

impl State for PrepareConfiguration {
    type Input = Deployment;
    type Output = EvmTransactionContext<Deployment>;
    type Failure = FixtureFailure;

    fn state_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.test.evm-effect/prepare-configuration@1")
            .map_err(|_| ProgramError::InvalidContract)
    }
}

impl PureState for PrepareConfiguration {
    fn evaluate(input: Self::Input) -> ProposedStateOutcome<Self::Output, Self::Failure> {
        let Some(target) = input.outcome().created_address().cloned() else {
            return fixture_failure(FixtureFailure::MissingCreatedAddress);
        };
        let Ok(command) = Eip1559TransactionCommand::call(
            input.binding().clone(),
            target,
            fixture_configure_calldata(),
            EvmU256::from_u64(0),
            nonzero(CONFIGURATION_GAS),
            EvmU256::from_u64(PRIORITY_FEE),
            EvmU256::from_u64(MAX_FEE),
        ) else {
            return fixture_failure(FixtureFailure::InvalidConfigurationCommand);
        };
        ProposedStateOutcome::Success {
            output: EvmTransactionContext::new(input, command),
        }
    }
}

struct PrepareObservation;

impl State for PrepareObservation {
    type Input = Configuration;
    type Output = AnchoredContractCallContext<Configuration>;
    type Failure = FixtureFailure;

    fn state_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.test.evm-effect/prepare-observation@1")
            .map_err(|_| ProgramError::InvalidContract)
    }
}

impl PureState for PrepareObservation {
    fn evaluate(input: Self::Input) -> ProposedStateOutcome<Self::Output, Self::Failure> {
        let Some(target) = input.outcome().target().cloned() else {
            return fixture_failure(FixtureFailure::MissingCallTarget);
        };
        let route = input.binding().route().clone();
        let anchor = input.receipt().block_anchor().clone();
        match AnchoredContractCallContext::for_route(
            input,
            &route,
            target,
            VALUE_SELECTOR.to_vec(),
            anchor,
        ) {
            Ok(output) => ProposedStateOutcome::Success { output },
            Err(_) => fixture_failure(FixtureFailure::InvalidObservationContext),
        }
    }
}

struct DecodeValue;

impl State for DecodeValue {
    type Input = AnchoredContractCallCompletion<Configuration>;
    type Output = EvmU256;
    type Failure = FixtureFailure;

    fn state_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.test.evm-effect/decode-value@1")
            .map_err(|_| ProgramError::InvalidContract)
    }
}

impl PureState for DecodeValue {
    fn evaluate(input: Self::Input) -> ProposedStateOutcome<Self::Output, Self::Failure> {
        match decode_fixture_value(input.result().return_bytes()) {
            Some(output) => ProposedStateOutcome::Success { output },
            None => fixture_failure(FixtureFailure::InvalidReturnData),
        }
    }
}

struct Abort<I, O>(PhantomData<fn(I) -> O>);

impl<I: MfmValue, O: MfmValue> State for Abort<I, O>
where
    FixtureFailure: From<I>,
{
    type Input = I;
    type Output = O;
    type Failure = FixtureFailure;

    fn state_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.test.evm-effect/abort@1").map_err(|_| ProgramError::InvalidContract)
    }
}

impl<I: MfmValue, O: MfmValue> PureState for Abort<I, O>
where
    FixtureFailure: From<I>,
{
    fn evaluate(input: Self::Input) -> ProposedStateOutcome<Self::Output, Self::Failure> {
        fixture_failure(input.into())
    }
}

fn fixture_failure<O>(failure: FixtureFailure) -> ProposedStateOutcome<O, FixtureFailure> {
    ProposedStateOutcome::Failure { failure }
}

struct EffectFixtureOperation {
    binding: EvmTransactionBinding,
}

impl Operation for EffectFixtureOperation {
    type Input = EvmTransactionContext<EvmU256>;
    type Output = EvmU256;
    type Failure = FixtureFailure;

    fn expand(
        &self,
        body: &mut OperationExpansion<Self::Input, Self::Output, Self::Failure>,
    ) -> mfm_program::Result<()> {
        body.with_failure_handler::<
            EvmTransactionReversion<EvmU256>,
            EvmTransactionContext<Deployment>,
        >(
            |protected| {
                protected
                    .effect::<ExecuteEvmTransaction<EvmU256>, EvmTransactionEffect>(&self.binding)?;
                protected.pure::<PrepareConfiguration>()
            },
            |handler| {
                handler.pure::<
                    Abort<
                        EvmTransactionReversion<EvmU256>,
                        EvmTransactionContext<Deployment>,
                    >,
                >()
            },
        )?;
        body.with_failure_handler::<
            EvmTransactionReversion<Deployment>,
            AnchoredContractCallContext<Configuration>,
        >(
            |protected| {
                protected.effect::<ExecuteEvmTransaction<Deployment>, EvmTransactionEffect>(
                    &self.binding,
                )?;
                protected.pure::<PrepareObservation>()
            },
            |handler| {
                handler.pure::<
                    Abort<
                        EvmTransactionReversion<Deployment>,
                        AnchoredContractCallContext<Configuration>,
                    >,
                >()
            },
        )?;
        body.with_failure_handler::<AnchoredContractCallFailure<Configuration>, EvmU256>(
            |protected| {
                protected
                    .read::<ReadAnchoredContractCall<Configuration>, EvmAnchoredContractCallRead>(
                        self.binding.route(),
                    )?;
                protected.pure::<DecodeValue>()
            },
            |handler| handler.pure::<Abort<AnchoredContractCallFailure<Configuration>, EvmU256>>(),
        )
    }
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
    builder
        .register_pure::<PrepareConfiguration>()
        .expect("prepare configuration");
    builder
        .register_pure::<PrepareObservation>()
        .expect("prepare observation");
    builder
        .register_pure::<DecodeValue>()
        .expect("decode value");
    builder
        .register_pure::<
            Abort<EvmTransactionReversion<EvmU256>, EvmTransactionContext<Deployment>>,
        >()
        .expect("deployment failure");
    builder
        .register_pure::<
            Abort<
                EvmTransactionReversion<Deployment>,
                AnchoredContractCallContext<Configuration>,
            >,
        >()
        .expect("configuration failure");
    builder
        .register_pure::<Abort<AnchoredContractCallFailure<Configuration>, EvmU256>>()
        .expect("anchored failure");
    builder
        .register_effect::<ExecuteEvmTransaction<EvmU256>, EvmTransactionEffect>()
        .expect("creation state");
    builder
        .register_effect::<ReserveEvmNonce<EvmU256>, EvmNonceReservationEffect>()
        .unwrap();
    builder
        .register_effect::<PrepareEvmTransaction<EvmU256>, EvmTransactionPreparationEffect>()
        .unwrap();
    builder
        .register_pure::<ProjectEvmTransactionOutcome<EvmU256>>()
        .unwrap();
    builder
        .register_effect::<ExecuteEvmTransaction<Deployment>, EvmTransactionEffect>()
        .expect("call state");
    builder
        .register_effect::<ReserveEvmNonce<Deployment>, EvmNonceReservationEffect>()
        .unwrap();
    builder
        .register_effect::<PrepareEvmTransaction<Deployment>, EvmTransactionPreparationEffect>()
        .unwrap();
    builder
        .register_pure::<ProjectEvmTransactionOutcome<Deployment>>()
        .unwrap();
    builder
        .register_read::<ReadAnchoredContractCall<Configuration>, EvmAnchoredContractCallRead>()
        .expect("anchored state");
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

fn terminal_value(view: &RunView) -> EvmU256 {
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

    let deployment_command = Eip1559TransactionCommand::create(
        binding.clone(),
        initcode,
        EvmU256::from_u64(0),
        nonzero(DEPLOYMENT_GAS),
        EvmU256::from_u64(PRIORITY_FEE),
        EvmU256::from_u64(MAX_FEE),
    )
    .expect("deployment command");
    let input = EvmTransactionContext::new(EvmU256::from_u64(0), deployment_command);
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
        terminal_value(&terminal),
        EvmU256::from_u64(CONFIGURED_VALUE)
    );
    let final_nonce = setup_provider
        .pending_nonce(&sender)
        .await
        .expect("final pending nonce");
    assert_eq!(final_nonce, 2);

    external_wallet_transfer(&setup_provider, signer.as_ref(), &binding, final_nonce).await;
    let fresh_command = Eip1559TransactionCommand::call(
        binding.clone(),
        sender.clone(),
        Vec::new(),
        EvmU256::from_u64(0),
        nonzero(21_000),
        EvmU256::from_u64(PRIORITY_FEE),
        EvmU256::from_u64(MAX_FEE),
    )
    .unwrap();
    let cold_runtime = runtime(&runtime_locator, &rpc_locator, &binding, signer, consumed).await;
    let fresh_run = RunId::from_digest(DigestBytes::from_array([0x5b; 32]));
    struct WalletCall(EvmTransactionBinding);
    impl Operation for WalletCall {
        type Input = EvmTransactionContext<EvmU256>;
        type Output = EvmTransactionCompletion<EvmU256>;
        type Failure = EvmTransactionReversion<EvmU256>;
        fn expand(
            &self,
            body: &mut OperationExpansion<Self::Input, Self::Output, Self::Failure>,
        ) -> mfm_program::Result<()> {
            body.effect::<ExecuteEvmTransaction<EvmU256>, EvmTransactionEffect>(&self.0)
        }
    }
    let fresh_program = expand_program(
        EntryPointId::new("mfm.test.evm-effect/wallet-call@1").unwrap(),
        &WalletCall(binding.clone()),
    )
    .unwrap();
    // Resume first so Unavailable from admission is retried without assuming genesis committed.
    drive_to_success::<EvmTransactionReversion<EvmU256>>(async || {
        match cold_runtime.resume(&fresh_run).await {
            Err(RuntimeError::Absent) => {
                cold_runtime
                    .start(
                        fresh_run.clone(),
                        fresh_program.clone(),
                        EvmTransactionContext::new(EvmU256::from_u64(0), fresh_command.clone()),
                    )
                    .await
            }
            progress => progress,
        }
    })
    .await;
    let final_nonce = final_nonce + 2;
    assert_eq!(
        setup_provider.pending_nonce(&sender).await.unwrap(),
        final_nonce
    );
    let cold_read = cold_runtime.read(&run_id).await.expect("cold read");
    assert_eq!(cold_read.head_sequence(), terminal.head_sequence());
    assert_eq!(cold_read.head_digest(), terminal.head_digest());
    assert_eq!(
        terminal_value(&cold_read),
        EvmU256::from_u64(CONFIGURED_VALUE)
    );
    let cold_resume = cold_runtime.resume(&run_id).await.expect("terminal resume");
    assert_eq!(cold_resume.head_sequence(), terminal.head_sequence());
    assert_eq!(cold_resume.head_digest(), terminal.head_digest());
    assert_eq!(
        terminal_value(&cold_resume),
        EvmU256::from_u64(CONFIGURED_VALUE)
    );
    assert_eq!(
        setup_provider
            .pending_nonce(&sender)
            .await
            .expect("cold replay nonce"),
        final_nonce
    );

    owner.shutdown().await.expect("keystore shutdown");
}

#[tokio::test]
#[should_panic(expected = "ConfigurationReverted")]
async fn progress_retries_unavailable_then_reports_typed_failure_immediately() {
    struct FailingOperation;
    impl Operation for FailingOperation {
        type Input = FixtureFailure;
        type Output = EvmU256;
        type Failure = FixtureFailure;
        fn expand(
            &self,
            body: &mut OperationExpansion<Self::Input, Self::Output, Self::Failure>,
        ) -> mfm_program::Result<()> {
            body.pure::<Abort<FixtureFailure, EvmU256>>()
        }
    }
    let mut builder = RuntimeAssemblyBuilder::new().unwrap();
    builder
        .register_pure::<Abort<FixtureFailure, EvmU256>>()
        .unwrap();
    let runtime = Runtime::new(builder.finish(), Arc::new(mfm_store::MemoryStore::new()));
    let mut unavailable = true;
    tokio::time::timeout(
        Duration::from_secs(2),
        drive_to_success::<FixtureFailure>(async || {
            if std::mem::take(&mut unavailable) {
                return Err(RuntimeError::Unavailable);
            }
            runtime
                .start(
                    RunId::from_digest(DigestBytes::from_array([0x5c; 32])),
                    expand_program(
                        EntryPointId::new("mfm.test.evm-effect/failure@1").unwrap(),
                        &FailingOperation,
                    )
                    .unwrap(),
                    FixtureFailure::ConfigurationReverted,
                )
                .await
        }),
    )
    .await
    .expect("a terminal failure must not be polled until the progress deadline");
}
