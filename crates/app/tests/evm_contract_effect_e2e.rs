use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use mfm_evm::{
    AnchoredContractCallCompletion, AnchoredContractCallContext, AnchoredContractCallFailure,
    Eip1559TransactionCommand, EvmAddress, EvmAnchoredContractCallRead, EvmAuthorityEpoch,
    EvmBlockAnchor, EvmChainInstance, EvmEndpoint, EvmHash, EvmTransactionAction,
    EvmTransactionBinding, EvmTransactionCompletion, EvmTransactionConfirmation,
    EvmTransactionContext, EvmTransactionEffect, EvmTransactionReversion, EvmTransactionRoute,
    EvmU256, ExecuteEvmTransaction, ReadAnchoredContractCall,
};
use mfm_evm_live::{
    ethereum_address, evm_keccak256, register_evm_anchored_contract_calls,
    register_evm_transaction_effect, EvmAdapterLocator, EvmProvider, EvmTransactionProvider,
    JsonRpcEvmProvider, EVM_EIP1559_SIGNING_PURPOSE_ID,
};
use mfm_evm_transaction_authority::{
    AuthorityError, AuthorityFuture, AuthorityState, EvmTransactionAuthority, PreparedRecord,
    Reservation, SettledRecord,
};
use mfm_ids::{ContentRef, DigestBytes, EffectId, EntryPointId, RunId, StableId};
use mfm_journal::{JournalHistory, JournalRecord};
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
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

const SOURCE: &str = include_str!("fixtures/MfmEffectFixture.sol");
const MAX_SOLC_STDOUT_BYTES: usize = 8_388_608;
const MAX_RPC_RESPONSE_BYTES: usize = 512 * 1024;
const RPC_TIMEOUT: Duration = Duration::from_secs(10);
const INITIAL_VALUE: u64 = 7;
const CONFIGURED_VALUE: u64 = 42;
const DEPLOYMENT_GAS: u64 = 2_000_000;
const CONFIGURATION_GAS: u64 = 200_000;
const PRIORITY_FEE: u64 = 1_000_000_000;
const MAX_FEE: u64 = 10_000_000_000;
const FUNDING_WEI_HEX: &str = "0xde0b6b3a7640000";
const CONFIGURE_SELECTOR: [u8; 4] = [0x1e, 0xb2, 0x5e, 0x0a];
const VALUE_SELECTOR: [u8; 4] = [0x3f, 0xa4, 0xf2, 0x45];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
#[mfm(
    namespace = "mfm.test.evm-effect",
    name = "lifecycle-context",
    version = "1",
    schema = "mfm.test.evm-effect-lifecycle-context"
)]
enum LifecycleContext {
    AwaitingDeployment {
        binding: EvmTransactionBinding,
    },
    AwaitingConfiguration {
        binding: EvmTransactionBinding,
        deployment: EvmTransactionConfirmation,
    },
    BothTransactionsComplete {
        deployment: EvmTransactionConfirmation,
        configuration: EvmTransactionConfirmation,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.test.evm-effect",
    name = "report",
    version = "1",
    schema = "mfm.test.evm-effect-report"
)]
struct EffectFixtureReport {
    anchor: EvmBlockAnchor,
    configuration_hash: EvmHash,
    contract_address: EvmAddress,
    deployment_hash: EvmHash,
    value: EvmU256,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[mfm(
    namespace = "mfm.test.evm-effect",
    name = "failure",
    version = "1",
    schema = "mfm.test.evm-effect-failure"
)]
enum EffectFixtureFailure {
    TransactionReverted,
    AnchoredObservationFailed,
    InvalidLifecycle,
    InvalidReturnData,
}

struct PrepareConfiguration;

impl State for PrepareConfiguration {
    type Input = EvmTransactionCompletion<LifecycleContext>;
    type Output = EvmTransactionContext<LifecycleContext>;
    type Failure = EffectFixtureFailure;

    fn state_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.test.evm-effect/prepare-configuration@1")
            .map_err(|_| ProgramError::InvalidContract)
    }
}

impl PureState for PrepareConfiguration {
    fn evaluate(input: Self::Input) -> ProposedStateOutcome<Self::Output, Self::Failure> {
        let LifecycleContext::AwaitingDeployment { binding } = input.caller_context().clone()
        else {
            return invalid_lifecycle();
        };
        let deployment = input.confirmed().clone();
        let EvmTransactionConfirmation::Created {
            created_address, ..
        } = &deployment
        else {
            return invalid_lifecycle();
        };
        let command = Eip1559TransactionCommand::new(
            binding.clone(),
            EvmTransactionAction::call(created_address.clone(), fixture_configure_calldata())
                .expect("fixed fixture calldata is bounded"),
            EvmU256::from_u64(0),
            CONFIGURATION_GAS,
            EvmU256::from_u64(PRIORITY_FEE),
            EvmU256::from_u64(MAX_FEE),
        )
        .expect("fixed configuration command is valid");
        ProposedStateOutcome::Success {
            output: EvmTransactionContext::new(
                LifecycleContext::AwaitingConfiguration {
                    binding,
                    deployment,
                },
                command,
            ),
        }
    }
}

struct PrepareObservation;

impl State for PrepareObservation {
    type Input = EvmTransactionCompletion<LifecycleContext>;
    type Output = AnchoredContractCallContext<LifecycleContext>;
    type Failure = EffectFixtureFailure;

    fn state_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.test.evm-effect/prepare-observation@1")
            .map_err(|_| ProgramError::InvalidContract)
    }
}

impl PureState for PrepareObservation {
    fn evaluate(input: Self::Input) -> ProposedStateOutcome<Self::Output, Self::Failure> {
        let LifecycleContext::AwaitingConfiguration {
            binding,
            deployment,
        } = input.caller_context().clone()
        else {
            return invalid_lifecycle();
        };
        let EvmTransactionConfirmation::Created {
            created_address, ..
        } = &deployment
        else {
            return invalid_lifecycle();
        };
        let created_address = created_address.clone();
        let configuration = input.confirmed().clone();
        let EvmTransactionConfirmation::Called { block_anchor, .. } = &configuration else {
            return invalid_lifecycle();
        };
        let block_anchor = block_anchor.clone();
        let context = LifecycleContext::BothTransactionsComplete {
            deployment,
            configuration,
        };
        let output = AnchoredContractCallContext::for_route(
            context,
            binding.route(),
            created_address,
            VALUE_SELECTOR.to_vec(),
            block_anchor,
        )
        .expect("fixed anchored call is valid");
        ProposedStateOutcome::Success { output }
    }
}

struct FinalizeReport;

impl State for FinalizeReport {
    type Input = AnchoredContractCallCompletion<LifecycleContext>;
    type Output = EffectFixtureReport;
    type Failure = EffectFixtureFailure;

    fn state_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.test.evm-effect/finalize-report@1")
            .map_err(|_| ProgramError::InvalidContract)
    }
}

impl PureState for FinalizeReport {
    fn evaluate(input: Self::Input) -> ProposedStateOutcome<Self::Output, Self::Failure> {
        let LifecycleContext::BothTransactionsComplete {
            deployment,
            configuration,
        } = input.caller_context().clone()
        else {
            return invalid_lifecycle();
        };
        let EvmTransactionConfirmation::Created {
            created_address,
            transaction_hash: deployment_hash,
            ..
        } = deployment
        else {
            return invalid_lifecycle();
        };
        let EvmTransactionConfirmation::Called {
            block_anchor,
            transaction_hash: configuration_hash,
        } = configuration
        else {
            return invalid_lifecycle();
        };
        if input.result().anchor() != &block_anchor {
            return invalid_lifecycle();
        }
        let Ok(return_bytes) = input.result().return_bytes() else {
            return invalid_return_data();
        };
        let Ok(value) = decode_fixture_value(&return_bytes) else {
            return invalid_return_data();
        };
        ProposedStateOutcome::Success {
            output: EffectFixtureReport {
                anchor: input.result().anchor().clone(),
                configuration_hash,
                contract_address: created_address,
                deployment_hash,
                value,
            },
        }
    }
}

fn fixture_configure_calldata() -> Vec<u8> {
    let mut calldata = Vec::with_capacity(36);
    calldata.extend_from_slice(&CONFIGURE_SELECTOR);
    calldata.extend_from_slice(&abi_word(CONFIGURED_VALUE));
    calldata
}

fn decode_fixture_value(return_bytes: &[u8]) -> Result<EvmU256, EffectFixtureFailure> {
    let word =
        <[u8; 32]>::try_from(return_bytes).map_err(|_| EffectFixtureFailure::InvalidReturnData)?;
    if word[..24].iter().any(|byte| *byte != 0) {
        return Err(EffectFixtureFailure::InvalidReturnData);
    }
    let suffix =
        <[u8; 8]>::try_from(&word[24..]).map_err(|_| EffectFixtureFailure::InvalidReturnData)?;
    Ok(EvmU256::from_u64(u64::from_be_bytes(suffix)))
}

#[test]
fn fixture_return_projection_rejects_malformed_or_wider_values() {
    assert_eq!(
        decode_fixture_value(&abi_word(CONFIGURED_VALUE)),
        Ok(EvmU256::from_u64(CONFIGURED_VALUE))
    );
    assert_eq!(
        decode_fixture_value(&[0; 31]),
        Err(EffectFixtureFailure::InvalidReturnData)
    );
    assert_eq!(
        decode_fixture_value(&[0; 33]),
        Err(EffectFixtureFailure::InvalidReturnData)
    );
    let mut wider = [0; 32];
    wider[23] = 1;
    assert_eq!(
        decode_fixture_value(&wider),
        Err(EffectFixtureFailure::InvalidReturnData)
    );
}

fn invalid_lifecycle<O>() -> ProposedStateOutcome<O, EffectFixtureFailure> {
    ProposedStateOutcome::Failure {
        failure: EffectFixtureFailure::InvalidLifecycle,
    }
}

fn invalid_return_data<O>() -> ProposedStateOutcome<O, EffectFixtureFailure> {
    ProposedStateOutcome::Failure {
        failure: EffectFixtureFailure::InvalidReturnData,
    }
}

macro_rules! failure_mapper {
    ($name:ident, $input:ty, $output:ty, $state_id:literal, $failure:expr) => {
        struct $name;

        impl State for $name {
            type Input = $input;
            type Output = $output;
            type Failure = EffectFixtureFailure;

            fn state_id() -> mfm_program::Result<StableId> {
                StableId::new($state_id).map_err(|_| ProgramError::InvalidContract)
            }
        }

        impl PureState for $name {
            fn evaluate(_input: Self::Input) -> ProposedStateOutcome<Self::Output, Self::Failure> {
                ProposedStateOutcome::Failure { failure: $failure }
            }
        }
    };
}

failure_mapper!(
    MapDeploymentFailure,
    EvmTransactionReversion<LifecycleContext>,
    EvmTransactionContext<LifecycleContext>,
    "mfm.test.evm-effect/map-deployment-failure@1",
    EffectFixtureFailure::TransactionReverted
);
failure_mapper!(
    MapConfigurationFailure,
    EvmTransactionReversion<LifecycleContext>,
    AnchoredContractCallContext<LifecycleContext>,
    "mfm.test.evm-effect/map-configuration-failure@1",
    EffectFixtureFailure::TransactionReverted
);
failure_mapper!(
    MapAnchoredFailure,
    AnchoredContractCallFailure<LifecycleContext>,
    EffectFixtureReport,
    "mfm.test.evm-effect/map-anchored-failure@1",
    EffectFixtureFailure::AnchoredObservationFailed
);

struct EffectFixtureOperation {
    binding: EvmTransactionBinding,
    route: EvmTransactionRoute,
}

impl Operation for EffectFixtureOperation {
    type Input = EvmTransactionContext<LifecycleContext>;
    type Output = EffectFixtureReport;
    type Failure = EffectFixtureFailure;

    fn expand(
        &self,
        body: &mut OperationExpansion<Self::Input, Self::Output, Self::Failure>,
    ) -> mfm_program::Result<()> {
        body.with_failure_handler::<
            EvmTransactionReversion<LifecycleContext>,
            EvmTransactionContext<LifecycleContext>,
        >(
            |protected| {
                protected.effect::<ExecuteEvmTransaction<LifecycleContext>, EvmTransactionEffect>(
                    &self.binding,
                )?;
                protected.pure::<PrepareConfiguration>()
            },
            |handler| handler.pure::<MapDeploymentFailure>(),
        )?;
        body.with_failure_handler::<
            EvmTransactionReversion<LifecycleContext>,
            AnchoredContractCallContext<LifecycleContext>,
        >(
            |protected| {
                protected.effect::<ExecuteEvmTransaction<LifecycleContext>, EvmTransactionEffect>(
                    &self.binding,
                )?;
                protected.pure::<PrepareObservation>()
            },
            |handler| handler.pure::<MapConfigurationFailure>(),
        )?;
        body.with_failure_handler::<
            AnchoredContractCallFailure<LifecycleContext>,
            EffectFixtureReport,
        >(
            |protected| {
                protected.read::<
                    ReadAnchoredContractCall<LifecycleContext>,
                    EvmAnchoredContractCallRead,
                >(&self.route)?;
                protected.pure::<FinalizeReport>()
            },
            |handler| handler.pure::<MapAnchoredFailure>(),
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
        expected_command_ref: &'a ContentRef,
    ) -> AuthorityFuture<'a, Option<AuthorityState>> {
        self.inner.load(effect_id, expected_command_ref)
    }

    fn reserve_or_compare<'a>(
        &'a self,
        effect_id: &'a EffectId,
        command_ref: &'a ContentRef,
        domain: &'a mfm_evm_transaction_authority::NonceDomain,
        observed_pending_nonce: u64,
    ) -> AuthorityFuture<'a, Reservation> {
        Box::pin(async move {
            let retained = self
                .inner
                .reserve_or_compare(effect_id, command_ref, domain, observed_pending_nonce)
                .await?;
            if !self.consumed.swap(true, Ordering::SeqCst) {
                return Err(AuthorityError::Unavailable);
            }
            Ok(retained)
        })
    }

    fn retain_prepared<'a>(
        &'a self,
        effect_id: &'a EffectId,
        command_ref: &'a ContentRef,
        transaction_hash: &'a EvmHash,
        raw_transaction: &'a mfm_evm_transaction_authority::ExactRawTransaction,
    ) -> AuthorityFuture<'a, PreparedRecord> {
        self.inner
            .retain_prepared(effect_id, command_ref, transaction_hash, raw_transaction)
    }

    fn retain_settlement<'a>(
        &'a self,
        effect_id: &'a EffectId,
        command_ref: &'a ContentRef,
        evidence: &'a mfm_evm::EvmTransactionSettlement,
    ) -> AuthorityFuture<'a, SettledRecord> {
        self.inner
            .retain_settlement(effect_id, command_ref, evidence)
    }
}

async fn runtime(
    runtime_locator: &RuntimePostgresLocator,
    rpc_locator: &EvmAdapterLocator,
    binding: &EvmTransactionBinding,
    signer: Arc<dyn Secp256k1Signer>,
    consumed: Arc<AtomicBool>,
) -> (
    Runtime,
    Arc<PostgresBackend>,
    Arc<PostgresEvmTransactionAuthority>,
) {
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
        inner: Arc::clone(&transaction_authority),
        consumed,
    });
    let transaction_provider: Arc<dyn EvmTransactionProvider> = provider.clone();
    let read_provider: Arc<dyn EvmProvider> = provider;

    let mut builder = RuntimeAssemblyBuilder::new();
    builder
        .register_pure::<PrepareConfiguration>()
        .expect("prepare configuration");
    builder
        .register_pure::<PrepareObservation>()
        .expect("prepare observation");
    builder
        .register_pure::<FinalizeReport>()
        .expect("finalize report");
    builder
        .register_pure::<MapDeploymentFailure>()
        .expect("deployment failure");
    builder
        .register_pure::<MapConfigurationFailure>()
        .expect("configuration failure");
    builder
        .register_pure::<MapAnchoredFailure>()
        .expect("anchored failure");
    builder
        .register_effect::<ExecuteEvmTransaction<LifecycleContext>, EvmTransactionEffect>()
        .expect("transaction state");
    builder
        .register_read::<ReadAnchoredContractCall<LifecycleContext>, EvmAnchoredContractCallRead>()
        .expect("anchored state");
    register_evm_transaction_effect(
        &mut builder,
        binding.clone(),
        signer,
        authority,
        transaction_provider,
    )
    .expect("transaction adapter");
    register_evm_anchored_contract_calls(&mut builder, binding.route().clone(), read_provider)
        .expect("anchored adapter");
    let store: Arc<dyn Store> = backend.clone();
    (
        Runtime::new(builder.finish().expect("assembly"), store),
        backend,
        transaction_authority,
    )
}

#[derive(Debug, Clone)]
struct CompiledFixture {
    creation: Vec<u8>,
    deployed: Vec<u8>,
    configure_calldata: Vec<u8>,
    value_calldata: Vec<u8>,
    configured_topic: EvmHash,
}

#[derive(Debug, Clone, Copy, thiserror::Error)]
#[error("fixture compilation failed")]
struct CompileError;

fn compile_fixture() -> Result<CompiledFixture, CompileError> {
    let input = serde_json::json!({
        "language": "Solidity",
        "sources": { "MfmEffectFixture.sol": { "content": SOURCE } },
        "settings": {
            "optimizer": { "enabled": true, "runs": 200 },
            "evmVersion": "cancun",
            "viaIR": false,
            "metadata": { "bytecodeHash": "none", "appendCBOR": false },
            "outputSelection": {
                "*": { "*": ["abi", "evm.bytecode.object", "evm.deployedBytecode.object"] }
            }
        }
    });
    let mut child = Command::new("solc")
        .arg("--standard-json")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|_| CompileError)?;
    child
        .stdin
        .take()
        .ok_or(CompileError)?
        .write_all(&serde_json::to_vec(&input).map_err(|_| CompileError)?)
        .map_err(|_| CompileError)?;
    let stdout = child.stdout.take().ok_or(CompileError)?;
    let stderr = child.stderr.take().ok_or(CompileError)?;
    let stdout_reader = std::thread::spawn(move || read_bounded(stdout, MAX_SOLC_STDOUT_BYTES));
    let stderr_reader = std::thread::spawn(move || read_bounded(stderr, MAX_RPC_RESPONSE_BYTES));
    let status = child.wait().map_err(|_| CompileError)?;
    let stdout = stdout_reader.join().map_err(|_| CompileError)??;
    let _stderr = stderr_reader.join().map_err(|_| CompileError)??;
    if !status.success() {
        return Err(CompileError);
    }
    let output: serde_json::Value = serde_json::from_slice(&stdout).map_err(|_| CompileError)?;
    if output
        .get("errors")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|errors| {
            errors.iter().any(|error| {
                error.get("severity").and_then(serde_json::Value::as_str) == Some("error")
            })
        })
    {
        return Err(CompileError);
    }
    let contract = output
        .pointer("/contracts/MfmEffectFixture.sol/MfmEffectFixture")
        .ok_or(CompileError)?;
    let abi = contract
        .get("abi")
        .and_then(serde_json::Value::as_array)
        .ok_or(CompileError)?;
    require_abi(abi, "constructor", None, &["uint256"], &[])?;
    let configure_selector = selector_bytes(&require_abi(
        abi,
        "function",
        Some("configure"),
        &["uint256"],
        &[],
    )?)?;
    let value_selector = selector_bytes(&require_abi(
        abi,
        "function",
        Some("value"),
        &[],
        &["uint256"],
    )?)?;
    if configure_selector != CONFIGURE_SELECTOR || value_selector != VALUE_SELECTOR {
        return Err(CompileError);
    }
    let configured_topic = require_abi(abi, "event", Some("Configured"), &["uint256"], &[])?;
    let mut creation = decode_hex_data(
        contract
            .pointer("/evm/bytecode/object")
            .and_then(serde_json::Value::as_str)
            .ok_or(CompileError)?,
        49_152,
    )?;
    creation.extend_from_slice(&abi_word(INITIAL_VALUE));
    let deployed = decode_hex_data(
        contract
            .pointer("/evm/deployedBytecode/object")
            .and_then(serde_json::Value::as_str)
            .ok_or(CompileError)?,
        24_576,
    )?;
    if creation.is_empty() || deployed.is_empty() {
        return Err(CompileError);
    }
    let mut configure_calldata = configure_selector.to_vec();
    configure_calldata.extend_from_slice(&abi_word(CONFIGURED_VALUE));
    let value_calldata = value_selector.to_vec();
    Ok(CompiledFixture {
        creation,
        deployed,
        configure_calldata,
        value_calldata,
        configured_topic,
    })
}

fn read_bounded(mut input: impl Read, maximum: usize) -> Result<Vec<u8>, CompileError> {
    let limit = u64::try_from(maximum).map_err(|_| CompileError)? + 1;
    let mut bytes = Vec::new();
    input
        .by_ref()
        .take(limit)
        .read_to_end(&mut bytes)
        .map_err(|_| CompileError)?;
    if bytes.len() > maximum {
        return Err(CompileError);
    }
    Ok(bytes)
}

fn require_abi(
    abi: &[serde_json::Value],
    kind: &str,
    name: Option<&str>,
    inputs: &[&str],
    outputs: &[&str],
) -> Result<EvmHash, CompileError> {
    let entry = abi
        .iter()
        .find(|entry| {
            entry.get("type").and_then(serde_json::Value::as_str) == Some(kind)
                && name.is_none_or(|name| {
                    entry.get("name").and_then(serde_json::Value::as_str) == Some(name)
                })
                && abi_types(entry, "inputs").as_deref() == Some(inputs)
                && (kind != "function" || abi_types(entry, "outputs").as_deref() == Some(outputs))
        })
        .ok_or(CompileError)?;
    if kind == "event" && entry.get("anonymous").and_then(serde_json::Value::as_bool) != Some(false)
    {
        return Err(CompileError);
    }
    let signature = match name {
        Some(name) => format!("{name}({})", inputs.join(",")),
        None => "constructor(uint256)".to_owned(),
    };
    evm_keccak256(signature.as_bytes()).map_err(|_| CompileError)
}

fn abi_types<'a>(entry: &'a serde_json::Value, field: &str) -> Option<Vec<&'a str>> {
    entry
        .get(field)?
        .as_array()?
        .iter()
        .map(|parameter| parameter.get("type")?.as_str())
        .collect()
}

fn selector_bytes(hash: &EvmHash) -> Result<[u8; 4], CompileError> {
    decode_hex_data(&hash.as_str()[2..10], 4)?
        .try_into()
        .map_err(|_| CompileError)
}

fn abi_word(value: u64) -> [u8; 32] {
    let mut word = [0_u8; 32];
    word[24..].copy_from_slice(&value.to_be_bytes());
    word
}

fn decode_hex_data(value: &str, maximum: usize) -> Result<Vec<u8>, CompileError> {
    let digits = value.strip_prefix("0x").unwrap_or(value);
    if !digits.len().is_multiple_of(2)
        || digits.len() / 2 > maximum
        || !digits
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(CompileError);
    }
    let mut bytes = Vec::with_capacity(digits.len() / 2);
    for pair in digits.as_bytes().chunks_exact(2) {
        let pair = std::str::from_utf8(pair).map_err(|_| CompileError)?;
        bytes.push(u8::from_str_radix(pair, 16).map_err(|_| CompileError)?);
    }
    Ok(bytes)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("Reth development observer is unavailable")]
struct ObserverError;

struct RethDevObserver {
    url: reqwest::Url,
    client: reqwest::Client,
}

impl RethDevObserver {
    fn new(locator: &str) -> Result<Self, ObserverError> {
        let url = reqwest::Url::parse(locator).map_err(|_| ObserverError)?;
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .referer(false)
            .retry(reqwest::retry::never())
            .timeout(RPC_TIMEOUT)
            .build()
            .map_err(|_| ObserverError)?;
        Ok(Self { url, client })
    }

    async fn rpc(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, ObserverError> {
        let response = self
            .client
            .post(self.url.clone())
            .json(&serde_json::json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": method,
                "params": params,
            }))
            .send()
            .await
            .map_err(|_| ObserverError)?;
        if !response.status().is_success()
            || response
                .content_length()
                .is_some_and(|length| length > MAX_RPC_RESPONSE_BYTES as u64)
        {
            return Err(ObserverError);
        }
        let mut response = response;
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| ObserverError)? {
            if body.len() + chunk.len() > MAX_RPC_RESPONSE_BYTES {
                return Err(ObserverError);
            }
            body.extend_from_slice(&chunk);
        }
        let envelope: serde_json::Value =
            serde_json::from_slice(&body).map_err(|_| ObserverError)?;
        if envelope.get("jsonrpc").and_then(serde_json::Value::as_str) != Some("2.0")
            || envelope.get("id").and_then(serde_json::Value::as_u64) != Some(1)
            || envelope.get("error").is_some()
        {
            return Err(ObserverError);
        }
        envelope.get("result").cloned().ok_or(ObserverError)
    }

    async fn unlocked_account(&self) -> Result<EvmAddress, ObserverError> {
        self.rpc("eth_accounts", serde_json::json!([]))
            .await?
            .as_array()
            .and_then(|accounts| accounts.first())
            .and_then(serde_json::Value::as_str)
            .and_then(|address| EvmAddress::new(address.to_owned()).ok())
            .ok_or(ObserverError)
    }

    async fn fund(&self, from: &EvmAddress, to: &EvmAddress) -> Result<EvmHash, ObserverError> {
        self.rpc(
            "eth_sendTransaction",
            serde_json::json!([{
                "from": from.as_str(),
                "gas": format!("{:#x}", 21_000_u64),
                "maxFeePerGas": format!("{MAX_FEE:#x}"),
                "maxPriorityFeePerGas": format!("{PRIORITY_FEE:#x}"),
                "to": to.as_str(),
                "value": FUNDING_WEI_HEX,
            }]),
        )
        .await?
        .as_str()
        .and_then(|hash| EvmHash::new(hash.to_owned()).ok())
        .ok_or(ObserverError)
    }

    async fn base_fee(&self) -> Result<u64, ObserverError> {
        self.rpc("eth_getBlockByNumber", serde_json::json!(["latest", false]))
            .await?
            .get("baseFeePerGas")
            .and_then(serde_json::Value::as_str)
            .and_then(quantity_u64)
            .ok_or(ObserverError)
    }

    async fn pending_nonce(&self, sender: &EvmAddress) -> Result<u64, ObserverError> {
        self.rpc(
            "eth_getTransactionCount",
            serde_json::json!([sender.as_str(), "pending"]),
        )
        .await?
        .as_str()
        .and_then(quantity_u64)
        .ok_or(ObserverError)
    }

    async fn await_receipt(&self, hash: &EvmHash) -> Result<serde_json::Value, ObserverError> {
        for _ in 0..100 {
            let receipt = self
                .rpc(
                    "eth_getTransactionReceipt",
                    serde_json::json!([hash.as_str()]),
                )
                .await?;
            if !receipt.is_null() {
                return Ok(receipt);
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        Err(ObserverError)
    }

    async fn code(&self, address: &EvmAddress) -> Result<Vec<u8>, ObserverError> {
        let result = self
            .rpc(
                "eth_getCode",
                serde_json::json!([address.as_str(), "latest"]),
            )
            .await?;
        decode_rpc_data(result.as_str().ok_or(ObserverError)?, 24_576)
    }

    async fn call_at(
        &self,
        address: &EvmAddress,
        calldata: &[u8],
        anchor: &EvmBlockAnchor,
    ) -> Result<Vec<u8>, ObserverError> {
        let result = self
            .rpc(
                "eth_call",
                serde_json::json!([{
                    "to": address.as_str(),
                    "data": format!("0x{}", encode_hex(calldata)),
                }, {
                    "blockHash": anchor.hash().as_str(),
                    "requireCanonical": true,
                }]),
            )
            .await?;
        decode_rpc_data(result.as_str().ok_or(ObserverError)?, 131_072)
    }

    async fn wallet_transactions(
        &self,
        sender: &EvmAddress,
    ) -> Result<Vec<ObservedTransaction>, ObserverError> {
        let latest = self
            .rpc("eth_blockNumber", serde_json::json!([]))
            .await?
            .as_str()
            .and_then(quantity_u64)
            .ok_or(ObserverError)?;
        let first = latest.saturating_sub(255);
        let mut total_transactions = 0_usize;
        let mut total_logs = 0_usize;
        let mut selected = Vec::new();
        for number in first..=latest {
            let block = self
                .rpc(
                    "eth_getBlockByNumber",
                    serde_json::json!([format!("0x{number:x}"), true]),
                )
                .await?;
            let transactions = block
                .get("transactions")
                .and_then(serde_json::Value::as_array)
                .ok_or(ObserverError)?;
            total_transactions = total_transactions
                .checked_add(transactions.len())
                .ok_or(ObserverError)?;
            if total_transactions > 1_024 {
                return Err(ObserverError);
            }
            for transaction in transactions {
                if transaction.get("from").and_then(serde_json::Value::as_str)
                    == Some(sender.as_str())
                {
                    let observed = ObservedTransaction::from_rpc(transaction)?;
                    let receipt = self.await_receipt(&observed.hash).await?;
                    total_logs = total_logs
                        .checked_add(
                            receipt
                                .get("logs")
                                .and_then(serde_json::Value::as_array)
                                .ok_or(ObserverError)?
                                .len(),
                        )
                        .ok_or(ObserverError)?;
                    if total_transactions + total_logs > 1_024 {
                        return Err(ObserverError);
                    }
                    selected.push(ObservedTransaction {
                        receipt,
                        ..observed
                    });
                }
            }
        }
        Ok(selected)
    }
}

#[derive(Clone)]
struct ObservedTransaction {
    hash: EvmHash,
    from: EvmAddress,
    to: Option<EvmAddress>,
    nonce: u64,
    transaction_type: u64,
    chain_id: u64,
    gas: u64,
    max_priority_fee: u64,
    max_fee: u64,
    value: u64,
    input: Vec<u8>,
    receipt: serde_json::Value,
}

impl ObservedTransaction {
    fn from_rpc(value: &serde_json::Value) -> Result<Self, ObserverError> {
        Ok(Self {
            hash: checked_hash(value, "hash")?,
            from: checked_address(value, "from")?,
            to: optional_address(value, "to")?,
            nonce: checked_quantity(value, "nonce")?,
            transaction_type: checked_quantity(value, "type")?,
            chain_id: checked_quantity(value, "chainId")?,
            gas: checked_quantity(value, "gas")?,
            max_priority_fee: checked_quantity(value, "maxPriorityFeePerGas")?,
            max_fee: checked_quantity(value, "maxFeePerGas")?,
            value: checked_quantity(value, "value")?,
            input: decode_rpc_data(
                value
                    .get("input")
                    .and_then(serde_json::Value::as_str)
                    .ok_or(ObserverError)?,
                132_096,
            )?,
            receipt: serde_json::Value::Null,
        })
    }
}

fn checked_hash(value: &serde_json::Value, field: &str) -> Result<EvmHash, ObserverError> {
    value
        .get(field)
        .and_then(serde_json::Value::as_str)
        .and_then(|hash| EvmHash::new(hash.to_owned()).ok())
        .ok_or(ObserverError)
}

fn checked_address(value: &serde_json::Value, field: &str) -> Result<EvmAddress, ObserverError> {
    value
        .get(field)
        .and_then(serde_json::Value::as_str)
        .and_then(|address| EvmAddress::new(address.to_owned()).ok())
        .ok_or(ObserverError)
}

fn optional_address(
    value: &serde_json::Value,
    field: &str,
) -> Result<Option<EvmAddress>, ObserverError> {
    let value = value.get(field).ok_or(ObserverError)?;
    if value.is_null() {
        Ok(None)
    } else {
        value
            .as_str()
            .and_then(|address| EvmAddress::new(address.to_owned()).ok())
            .map(Some)
            .ok_or(ObserverError)
    }
}

fn checked_quantity(value: &serde_json::Value, field: &str) -> Result<u64, ObserverError> {
    value
        .get(field)
        .and_then(serde_json::Value::as_str)
        .and_then(quantity_u64)
        .ok_or(ObserverError)
}

fn quantity_u64(value: &str) -> Option<u64> {
    let digits = value.strip_prefix("0x")?;
    if digits.is_empty()
        || (digits.len() > 1 && digits.starts_with('0'))
        || digits.len() > 16
        || !digits
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return None;
    }
    u64::from_str_radix(digits, 16).ok()
}

fn decode_rpc_data(value: &str, maximum: usize) -> Result<Vec<u8>, ObserverError> {
    decode_hex_data(value, maximum).map_err(|_| ObserverError)
}

fn encode_hex(bytes: &[u8]) -> String {
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        write!(&mut encoded, "{byte:02x}").expect("String writes do not fail");
    }
    encoded
}

fn expected_create_address(sender: &EvmAddress) -> EvmAddress {
    let sender = decode_hex_data(sender.as_str(), 20).expect("checked sender bytes");
    let mut preimage = Vec::with_capacity(23);
    preimage.extend_from_slice(&[0xd6, 0x94]);
    preimage.extend_from_slice(&sender);
    preimage.push(0x80);
    let hash = evm_keccak256(&preimage).expect("CREATE address hash");
    EvmAddress::new(format!("0x{}", &hash.as_str()[26..])).expect("derived CREATE address")
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

fn terminal_report(view: &RunView) -> EffectFixtureReport {
    let RunViewState::Succeeded(value) = view.state() else {
        panic!("effect fixture must succeed")
    };
    serde_json::from_slice(value.canonical_bytes()).expect("typed final report")
}

async fn journal_effects(
    backend: &PostgresBackend,
    run_id: &RunId,
) -> (
    Vec<(EffectId, ContentRef, Eip1559TransactionCommand)>,
    usize,
) {
    let stored = backend
        .load_run(run_id)
        .await
        .expect("load history")
        .expect("retained history");
    let history = JournalHistory::qualify(run_id, stored).expect("qualified history");
    let mut prepared = Vec::new();
    let mut concluded = 0;
    let mut previous_was_effect_prepare = false;
    for record in history.records() {
        match record {
            JournalRecord::StateEffectPrepared { effect_id, command } => {
                let typed: Eip1559TransactionCommand =
                    serde_json::from_slice(command.canonical_bytes()).expect("typed command");
                prepared.push((effect_id.clone(), command.content_ref().clone(), typed));
                previous_was_effect_prepare = true;
            }
            JournalRecord::StateEffectConcluded { .. } => {
                assert!(previous_was_effect_prepare);
                concluded += 1;
                previous_was_effect_prepare = false;
            }
            _ => previous_was_effect_prepare = false,
        }
    }
    (prepared, concluded)
}

fn assert_transaction(
    transaction: &ObservedTransaction,
    command: &Eip1559TransactionCommand,
    nonce: u64,
    sender: &EvmAddress,
    expected_input: &[u8],
) {
    assert_eq!(transaction.from, *sender);
    assert_eq!(transaction.nonce, nonce);
    assert_eq!(transaction.transaction_type, 2);
    assert_eq!(
        transaction.chain_id,
        command.binding().route().chain_instance().chain_id()
    );
    assert_eq!(transaction.gas, command.gas_limit());
    assert_eq!(transaction.max_priority_fee, PRIORITY_FEE);
    assert_eq!(transaction.max_fee, MAX_FEE);
    assert_eq!(transaction.value, 0);
    assert_eq!(transaction.input, expected_input);
    assert_eq!(
        transaction
            .receipt
            .get("status")
            .and_then(serde_json::Value::as_str),
        Some("0x1")
    );
}

fn receipt_anchor(receipt: &serde_json::Value) -> EvmBlockAnchor {
    EvmBlockAnchor::new(
        EvmU256::from_u64(checked_quantity(receipt, "blockNumber").expect("receipt block number")),
        checked_hash(receipt, "blockHash").expect("receipt block hash"),
    )
}

#[tokio::test]
#[ignore = "requires the managed PostgreSQL, Reth, and pinned solc fixture"]
async fn evm_contract_effect_recovers_cold_and_mutates_exactly_twice() {
    let compiled = tokio::task::spawn_blocking(compile_fixture)
        .await
        .expect("compiler task")
        .expect("compiled fixture");
    let admin_raw =
        std::env::var("MFM_EFFECT_E2E_ADMIN_POSTGRES_LOCATOR").expect("managed admin locator");
    let runtime_raw =
        std::env::var("MFM_EFFECT_E2E_RUNTIME_POSTGRES_LOCATOR").expect("managed runtime locator");
    let rpc_raw =
        std::env::var("MFM_EFFECT_E2E_EVM_ADAPTER_LOCATOR").expect("managed Reth locator");
    let admin_locator = AdminPostgresLocator::parse(&admin_raw).expect("admin locator");
    let runtime_locator = RuntimePostgresLocator::parse(&runtime_raw).expect("runtime locator");
    provision_postgres(&admin_locator, &runtime_locator)
        .await
        .expect("fresh base schemas");
    provision_evm_transaction_authority(&admin_locator, &runtime_locator)
        .await
        .expect("fresh transaction authority");
    let rpc_locator = EvmAdapterLocator::parse(&rpc_raw).expect("RPC locator");
    let observer = RethDevObserver::new(&rpc_raw).expect("observer");

    let setup_authority = PostgresEvmTransactionAuthority::connect(&runtime_locator)
        .await
        .expect("setup authority");
    let setup_provider = JsonRpcEvmProvider::connect(&rpc_locator).expect("setup provider");
    let chain = setup_provider
        .chain_instance()
        .await
        .expect("chain instance");
    let route = EvmTransactionRoute::new(
        EvmChainInstance::new(chain.chain_id(), chain.genesis_hash().clone()).expect("chain"),
        EvmEndpoint::new("reth-effect-e2e")
            .expect("endpoint")
            .endpoint_ref()
            .expect("endpoint ref"),
    );
    let owner = KeystoreOwner::start().expect("keystore owner");
    let signer = generated_signer(&owner).await;
    let sender = ethereum_address(signer.public_key());
    let binding = EvmTransactionBinding::new(
        route.clone(),
        setup_authority.authority_epoch().clone(),
        sender.clone(),
    );
    drop(setup_provider);
    drop(setup_authority);

    assert!(observer.base_fee().await.expect("base fee") <= MAX_FEE);
    let funding_source = observer.unlocked_account().await.expect("unlocked account");
    let funding_hash = observer
        .fund(&funding_source, &sender)
        .await
        .expect("fund wallet");
    observer
        .await_receipt(&funding_hash)
        .await
        .expect("funding receipt");
    assert_eq!(
        observer
            .pending_nonce(&sender)
            .await
            .expect("pending nonce"),
        0
    );

    let expected_contract = expected_create_address(&sender);
    let deployment_command = Eip1559TransactionCommand::new(
        binding.clone(),
        EvmTransactionAction::create(compiled.creation.clone()).expect("creation action"),
        EvmU256::from_u64(0),
        DEPLOYMENT_GAS,
        EvmU256::from_u64(PRIORITY_FEE),
        EvmU256::from_u64(MAX_FEE),
    )
    .expect("deployment command");
    let configuration_command = Eip1559TransactionCommand::new(
        binding.clone(),
        EvmTransactionAction::call(
            expected_contract.clone(),
            compiled.configure_calldata.clone(),
        )
        .expect("configuration action"),
        EvmU256::from_u64(0),
        CONFIGURATION_GAS,
        EvmU256::from_u64(PRIORITY_FEE),
        EvmU256::from_u64(MAX_FEE),
    )
    .expect("configuration command");
    let input = EvmTransactionContext::new(
        LifecycleContext::AwaitingDeployment {
            binding: binding.clone(),
        },
        deployment_command.clone(),
    );
    let program = expand_program(
        EntryPointId::new("mfm.test.evm-effect/run@1").expect("entry point"),
        &EffectFixtureOperation {
            binding: binding.clone(),
            route,
        },
    )
    .expect("fixture Program");
    let run_id = RunId::from_digest(DigestBytes::from_array([0x5a; 32]));
    let consumed = Arc::new(AtomicBool::new(false));
    let mut initial = Some((program, input));
    let mut independently_observed = Vec::<PreparedRecord>::new();
    let mut terminal = None;

    for attempt in 0..8 {
        let (runtime, backend, authority) = runtime(
            &runtime_locator,
            &rpc_locator,
            &binding,
            Arc::clone(&signer),
            Arc::clone(&consumed),
        )
        .await;
        let progress = if attempt == 0 {
            let (program, input) = initial.take().expect("one initial admission");
            runtime.start(run_id.clone(), program, input).await
        } else {
            runtime.resume(&run_id).await
        };

        if attempt == 0 {
            assert!(matches!(progress, Err(RuntimeError::Unavailable)));
            assert!(consumed.load(Ordering::SeqCst));
            assert_eq!(
                observer
                    .pending_nonce(&sender)
                    .await
                    .expect("pending nonce"),
                0
            );
            let (effects, conclusions) = journal_effects(&backend, &run_id).await;
            assert_eq!(effects.len(), 1);
            assert_eq!(conclusions, 0);
            assert!(matches!(
                authority
                    .load(&effects[0].0, &effects[0].1)
                    .await
                    .expect("reservation"),
                Some(AuthorityState::Reserved(_))
            ));
            drop(runtime);
            drop(backend);
            drop(authority);
            continue;
        }

        match progress {
            Ok(view) if matches!(view.state(), RunViewState::Runnable) => {
                let (effects, _) = journal_effects(&backend, &run_id).await;
                let mut newly_prepared = 0;
                for (effect_id, command_ref, _) in effects {
                    let state = authority
                        .load(&effect_id, &command_ref)
                        .await
                        .expect("public authority state")
                        .expect("retained authority state");
                    let AuthorityState::Prepared(prepared) = state else {
                        continue;
                    };
                    if independently_observed
                        .iter()
                        .any(|observed| observed.transaction_hash() == prepared.transaction_hash())
                    {
                        continue;
                    }
                    let receipt = observer
                        .await_receipt(prepared.transaction_hash())
                        .await
                        .expect("independently observed prepared transaction");
                    if prepared.reservation().nonce() == 0 {
                        assert_eq!(
                            receipt
                                .get("contractAddress")
                                .and_then(serde_json::Value::as_str),
                            Some(expected_contract.as_str())
                        );
                    }
                    independently_observed.push(prepared);
                    newly_prepared += 1;
                }
                assert_eq!(newly_prepared, 1);
                drop(runtime);
                drop(backend);
                drop(authority);
            }
            Ok(view) => {
                drop(runtime);
                terminal = Some((view, backend, authority));
                break;
            }
            Err(error) => panic!("unexpected recovery failure: {error:?}"),
        }
    }

    let (terminal, final_backend, final_authority) =
        terminal.expect("fixture must terminate within eight caller invocations");
    independently_observed.sort_by_key(|prepared| prepared.reservation().nonce());
    let [deployment_prepared, configuration_prepared] = independently_observed.as_slice() else {
        panic!("exactly two prepared transactions must reach Reth")
    };
    let report = terminal_report(&terminal);
    let terminal_bytes = match terminal.state() {
        RunViewState::Succeeded(value) => value.canonical_bytes().to_vec(),
        _ => unreachable!(),
    };
    assert_eq!(report.contract_address, expected_contract);
    assert_eq!(
        report.deployment_hash,
        *deployment_prepared.transaction_hash()
    );
    assert_eq!(
        report.configuration_hash,
        *configuration_prepared.transaction_hash()
    );
    assert_eq!(report.value, EvmU256::from_u64(CONFIGURED_VALUE));

    let (final_effects, final_conclusions) = journal_effects(&final_backend, &run_id).await;
    assert_eq!(final_effects.len(), 2);
    assert_eq!(final_conclusions, 2);
    assert_eq!(final_effects[0].2, deployment_command);
    assert_eq!(final_effects[1].2, configuration_command);
    let mut settled = Vec::new();
    for (effect_id, command_ref, _) in &final_effects {
        let Some(AuthorityState::Settled(state)) = final_authority
            .load(effect_id, command_ref)
            .await
            .expect("settled authority")
        else {
            panic!("both Effects must be settled")
        };
        assert_eq!(
            state.prepared().reservation().domain().authority_epoch(),
            binding.authority_epoch()
        );
        assert_eq!(
            state.prepared().reservation().domain().chain_instance(),
            binding.route().chain_instance()
        );
        assert_eq!(state.prepared().reservation().domain().sender(), &sender);
        assert_eq!(
            evm_keccak256(state.prepared().raw_transaction().as_bytes()).expect("raw hash"),
            *state.prepared().transaction_hash()
        );
        settled.push(state);
    }
    assert_eq!(settled[0].prepared().reservation().nonce(), 0);
    assert_eq!(settled[1].prepared().reservation().nonce(), 1);
    assert_eq!(
        settled[0].evidence().transaction_hash(),
        &report.deployment_hash
    );
    assert_eq!(
        settled[1].evidence().transaction_hash(),
        &report.configuration_hash
    );

    let mut transactions = observer
        .wallet_transactions(&sender)
        .await
        .expect("canonical wallet transactions");
    transactions.sort_by_key(|transaction| transaction.nonce);
    assert_eq!(transactions.len(), 2);
    assert_eq!(
        transactions[0].hash,
        *settled[0].prepared().transaction_hash()
    );
    assert_eq!(
        transactions[1].hash,
        *settled[1].prepared().transaction_hash()
    );
    assert_eq!(transactions[0].hash, report.deployment_hash);
    assert_eq!(transactions[1].hash, report.configuration_hash);
    assert_eq!(
        settled[0].evidence().block_anchor(),
        &receipt_anchor(&transactions[0].receipt)
    );
    assert_eq!(
        settled[1].evidence().block_anchor(),
        &receipt_anchor(&transactions[1].receipt)
    );
    assert_eq!(settled[1].evidence().block_anchor(), &report.anchor);
    assert_transaction(
        &transactions[0],
        &deployment_command,
        0,
        &sender,
        &compiled.creation,
    );
    assert_transaction(
        &transactions[1],
        &configuration_command,
        1,
        &sender,
        &compiled.configure_calldata,
    );
    assert_eq!(transactions[0].to, None);
    assert_eq!(transactions[1].to.as_ref(), Some(&expected_contract));
    assert_eq!(
        observer
            .code(&expected_contract)
            .await
            .expect("deployed code"),
        compiled.deployed
    );

    let logs = transactions[1]
        .receipt
        .get("logs")
        .and_then(serde_json::Value::as_array)
        .expect("configuration logs");
    let expected_word = format!("0x{}", encode_hex(&abi_word(CONFIGURED_VALUE)));
    let configured_logs = logs
        .iter()
        .filter(|log| {
            log.get("address").and_then(serde_json::Value::as_str)
                == Some(expected_contract.as_str())
                && log
                    .get("topics")
                    .and_then(serde_json::Value::as_array)
                    .is_some_and(|topics| {
                        topics.len() == 1
                            && topics[0].as_str() == Some(compiled.configured_topic.as_str())
                    })
                && log.get("data").and_then(serde_json::Value::as_str)
                    == Some(expected_word.as_str())
        })
        .count();
    assert_eq!(configured_logs, 1);
    assert_eq!(
        observer
            .call_at(&expected_contract, &compiled.value_calldata, &report.anchor)
            .await
            .expect("independent anchored value"),
        abi_word(CONFIGURED_VALUE)
    );

    drop(final_backend);
    drop(final_authority);
    let (cold_runtime, _cold_backend, _cold_authority) =
        runtime(&runtime_locator, &rpc_locator, &binding, signer, consumed).await;
    let cold_read = cold_runtime.read(&run_id).await.expect("cold read");
    let cold_bytes = match cold_read.state() {
        RunViewState::Succeeded(value) => value.canonical_bytes(),
        _ => panic!("cold read must remain terminal"),
    };
    assert_eq!(cold_read.head_digest(), terminal.head_digest());
    assert_eq!(cold_bytes, terminal_bytes);
    let final_resume = cold_runtime.resume(&run_id).await.expect("terminal resume");
    assert_eq!(final_resume.head_digest(), terminal.head_digest());
    assert_eq!(terminal_report(&final_resume), report);
    assert_eq!(
        observer.pending_nonce(&sender).await.expect("final nonce"),
        2
    );

    owner.shutdown().await.expect("keystore shutdown");
}
