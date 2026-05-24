//! Reusable EVM deploy/configure/validate states.
//!
//! These states own the executable runtime behavior behind contract artifact adaptation, contract
//! deployment, post-deploy runtime calls, and validation assertions against live or replayed IO.
//!
//! Thin op crates should compose these states rather than reimplementing write-path behavior.

use async_trait::async_trait;
use mfm_collectors_rpc_control::{
    parse_u64_hex_value, EvmIoClient, JsonRpcCall, DEFAULT_CONTROL_SCOPE,
};
use mfm_machine::context::DynContext;
use mfm_machine::errors::{ErrorCategory, StateError};
use mfm_machine::ids::{ContextKey, StateId};
use mfm_machine::io::IoProvider;
use mfm_machine::meta::StateMeta;
use mfm_machine::recorder::EventRecorder;
use mfm_machine::state::{SnapshotPolicy, State, StateOutcome};
use mfm_state_common::ctx as op_ctx;
use mfm_state_common::errors as op_errors;
use mfm_state_common::idempotency as op_idempotency;
use mfm_state_common::rpc as op_rpc;
use mfm_state_common::states::meta;

use crate::rpc as evm_rpc;
use mfm_evm_dcv_model as shared_dcv;

const KEY_NIX_RESULT: &str = "result";
const KEY_CONTRACT_ARTIFACT: &str = "contract_artifact";
const KEY_CONTRACT_ADDRESS: &str = "contract_address";
const KEY_DEPLOY_TX_HASH: &str = "deploy_tx_hash";
const KEY_DEPLOY_RECEIPT: &str = "deploy_receipt";
const KEY_VALIDATED: &str = "validated";
const KEY_CHAIN_ID: &str = "chain_id";
const KEY_CLIENT_VERSION: &str = "client_version";

fn managed_call(
    network_id: &str,
    control_scope: &str,
    method: impl Into<String>,
    params: serde_json::Value,
) -> JsonRpcCall {
    JsonRpcCall::new(method, params)
        .with_control_scope(control_scope.to_string())
        .with_network_id(network_id.to_string())
}

fn normalized_control_scope(control_scope: &str) -> String {
    let trimmed = control_scope.trim();
    if trimmed.is_empty() {
        DEFAULT_CONTROL_SCOPE.to_string()
    } else {
        trimmed.to_string()
    }
}

async fn prepare_managed_sources(
    state_id: &StateId,
    io: &mut dyn IoProvider,
    network_id: &str,
    control_scope: &str,
) -> Result<String, StateError> {
    let control_scope = normalized_control_scope(control_scope);
    let mut client = EvmIoClient::new(state_id.clone(), io);
    let response = client
        .prepare_sources_in_scope(control_scope.clone(), network_id.to_string())
        .await
        .map_err(op_errors::state_from_io)?;
    if !response.sources.iter().any(|entry| entry.healthy) {
        return Err(op_errors::state_error_with_state(
            state_id.clone(),
            "rpc_control_no_healthy_sources",
            ErrorCategory::Rpc,
            true,
            format!(
                "no responsive rpc.control sources found for network `{}` and scope `{}`",
                response.network_id, response.control_scope
            ),
        ));
    }
    Ok(control_scope)
}

/// Runtime configuration for [`EvmDeployState`].
///
/// # Examples
///
/// ```rust
/// use mfm_evm_dcv_model::AbiArgumentValue;
/// use mfm_evm_runtime::states::write::EvmDeployStateConfig;
///
/// let cfg = EvmDeployStateConfig {
///     network_id: "ethereum-mainnet".to_string(),
///     control_scope: "shared".to_string(),
///     artifact: None,
///     artifact_port: "contract_artifact".to_string(),
///     from: "0x0000000000000000000000000000000000000001".to_string(),
///     constructor_args: vec![AbiArgumentValue::from_json_value(&serde_json::json!(42)).unwrap()],
///     value_hex: Some("0x0".to_string()),
///     signing_key_env: Some("MFM_DEPLOYER_KEY".to_string()),
///     poll_interval_ms: 1_000,
///     max_receipt_polls: 30,
/// };
///
/// assert_eq!(cfg.artifact_port, "contract_artifact");
/// assert_eq!(cfg.max_receipt_polls, 30);
/// ```
#[derive(Clone, Debug)]
pub struct EvmDeployStateConfig {
    /// Optional inline contract artifact; falls back to `artifact_port` when absent.
    pub artifact: Option<shared_dcv::ContractArtifactConfig>,
    /// Context key used to load the contract artifact when `artifact` is absent.
    pub artifact_port: String,
    /// Stable network identifier targeted by the managed RPC calls.
    pub network_id: String,
    /// Stable control-plane scope used to isolate managed source state.
    pub control_scope: String,
    /// Deployer address or sender address.
    pub from: String,
    /// Constructor arguments passed during deployment.
    pub constructor_args: Vec<shared_dcv::AbiArgumentValue>,
    /// Optional deployment value expressed as a hex quantity.
    pub value_hex: Option<String>,
    /// Optional environment variable name used for local signing.
    pub signing_key_env: Option<String>,
    /// Delay between receipt polls in milliseconds.
    pub poll_interval_ms: u64,
    /// Maximum number of receipt polls before timing out.
    pub max_receipt_polls: u64,
}

/// Single runtime configuration call submitted by [`EvmConfigureState`].
#[derive(Clone, Debug)]
pub struct EvmConfigureRuntimeCall {
    /// Function name to invoke.
    pub function: String,
    /// Positional arguments passed to the function call.
    pub args: Vec<shared_dcv::AbiArgumentValue>,
    /// Optional call value expressed as a hex quantity.
    pub value_hex: Option<String>,
}

/// Runtime configuration for [`EvmConfigureState`].
#[derive(Clone, Debug)]
pub struct EvmConfigureStateConfig {
    /// Optional inline contract artifact; falls back to `artifact_port` when absent.
    pub artifact: Option<shared_dcv::ContractArtifactConfig>,
    /// Context key used to load the contract artifact when `artifact` is absent.
    pub artifact_port: String,
    /// Stable network identifier targeted by the managed RPC calls.
    pub network_id: String,
    /// Stable control-plane scope used to isolate managed source state.
    pub control_scope: String,
    /// Sender address used for configuration transactions.
    pub from: String,
    /// Optional environment variable name used for local signing.
    pub signing_key_env: Option<String>,
    /// Optional inline contract address; falls back to context when absent.
    pub contract_address: Option<String>,
    /// Calls to execute against the deployed contract.
    pub calls: Vec<EvmConfigureRuntimeCall>,
    /// Context key used to write the configure transaction hashes export.
    pub tx_hashes_export_key: String,
    /// Context key used to write the configure receipts export.
    pub receipts_export_key: String,
    /// Delay between receipt polls in milliseconds.
    pub poll_interval_ms: u64,
    /// Maximum number of receipt polls before timing out.
    pub max_receipt_polls: u64,
}

/// Runtime configuration for [`EvmValidateState`].
///
/// `require_client_substring` is matched case-insensitively against `web3_clientVersion`.
#[derive(Clone, Debug)]
pub struct EvmValidateStateConfig {
    /// Optional inline contract artifact; falls back to `artifact_port` when absent.
    pub artifact: Option<shared_dcv::ContractArtifactConfig>,
    /// Context key used to load the contract artifact when `artifact` is absent.
    pub artifact_port: String,
    /// Stable network identifier targeted by the managed RPC calls.
    pub network_id: String,
    /// Stable control-plane scope used to isolate managed source state.
    pub control_scope: String,
    /// Optional inline contract address; falls back to context when absent.
    pub contract_address: Option<String>,
    /// Expected chain id for the connected RPC endpoint.
    pub expected_chain_id: u64,
    /// Substring that must appear in `web3_clientVersion`.
    pub require_client_substring: String,
    /// Read assertions evaluated with `eth_call`.
    pub read_assertions: Vec<shared_dcv::ReadAssertionConfig>,
    /// Event assertions evaluated with `eth_getLogs`.
    pub event_assertions: Vec<shared_dcv::EventAssertionConfig>,
}

/// State that extracts a contract artifact from a Nix build result.
///
/// The extracted artifact is normalized into the shared `contract_artifact` context slot expected
/// by the downstream deploy/configure/validate states.
#[derive(Clone, Debug)]
pub struct NixArtifactToEvmContractState {
    /// JSON pointer applied to the Nix result payload.
    pub result_pointer: String,
}

/// State that deploys a contract and records its address plus receipt in context.
#[derive(Clone, Debug)]
pub struct EvmDeployState {
    /// Stable state identifier assigned by the execution plan.
    pub state_id: StateId,
    /// Deployment runtime configuration.
    pub cfg: EvmDeployStateConfig,
}

/// State that submits runtime configuration transactions to a deployed contract.
#[derive(Clone, Debug)]
pub struct EvmConfigureState {
    /// Stable state identifier assigned by the execution plan.
    pub state_id: StateId,
    /// Configuration runtime configuration.
    pub cfg: EvmConfigureStateConfig,
}

/// State that validates deployed contract behavior against configured assertions.
#[derive(Clone, Debug)]
pub struct EvmValidateState {
    /// Stable state identifier assigned by the execution plan.
    pub state_id: StateId,
    /// Validation runtime configuration.
    pub cfg: EvmValidateStateConfig,
}

#[async_trait]
impl State for NixArtifactToEvmContractState {
    fn meta(&self) -> StateMeta {
        meta::config()
    }

    async fn handle(
        &self,
        ctx: &mut dyn DynContext,
        _io: &mut dyn IoProvider,
        _rec: &mut dyn EventRecorder,
    ) -> Result<StateOutcome, StateError> {
        let result = op_ctx::read_typed::<serde_json::Value>(
            ctx,
            &ContextKey(KEY_NIX_RESULT.to_string()),
            "ctx_missing_key",
            "required context key was missing",
            "ctx_type_mismatch",
            "context value was invalid",
        )?;

        let artifact_value = if self.result_pointer.is_empty() {
            result
        } else {
            result
                .pointer(&self.result_pointer)
                .cloned()
                .ok_or_else(|| {
                    op_errors::state_unknown(
                        "nix_result_pointer_missing",
                        "nix result did not contain the configured pointer",
                    )
                })?
        };

        let artifact =
            serde_json::from_value::<shared_dcv::ContractArtifactConfig>(artifact_value.clone())
                .map_err(|_| {
                    op_errors::state_unknown(
                        "invalid_contract_artifact",
                        "nix result artifact was invalid",
                    )
                })?;
        shared_dcv::parse_artifact(&artifact).map_err(|_| {
            op_errors::state_unknown(
                "invalid_contract_artifact",
                "nix result artifact was invalid",
            )
        })?;

        context_write_json(ctx, KEY_CONTRACT_ARTIFACT, artifact_value)?;
        Ok(StateOutcome {
            snapshot: SnapshotPolicy::OnSuccess,
        })
    }
}

#[async_trait]
impl State for EvmDeployState {
    fn meta(&self) -> StateMeta {
        meta::apply_side_effect(op_idempotency::state_purpose(
            "evm_deploy",
            &self.state_id,
            "apply_side_effect",
        ))
    }

    async fn handle(
        &self,
        ctx: &mut dyn DynContext,
        io: &mut dyn IoProvider,
        _rec: &mut dyn EventRecorder,
    ) -> Result<StateOutcome, StateError> {
        let artifact = resolve_artifact_config(ctx, &self.cfg.artifact, &self.cfg.artifact_port)?;
        let (abi, bytecode) = shared_dcv::parse_artifact(&artifact).map_err(|_| {
            op_errors::state_unknown("invalid_contract_artifact", "contract artifact was invalid")
        })?;
        let constructor_payload = shared_dcv::constructor_data(
            &abi,
            &bytecode,
            &self.cfg.constructor_args,
        )
        .map_err(|_| {
            op_errors::state_unknown("invalid_op_config", "constructor args did not match ABI")
        })?;
        let control_scope = prepare_managed_sources(
            &self.state_id,
            io,
            &self.cfg.network_id,
            &self.cfg.control_scope,
        )
        .await?;

        let Some(env_name) = self.cfg.signing_key_env.as_deref() else {
            return Err(op_errors::state_unknown(
                "evm_signed_transaction_required",
                "EVM deploy requires signing_key_env for durable signed transaction intent recording",
            ));
        };
        let nonce = evm_rpc::pending_nonce_u128_for_network(
            io,
            &self.state_id,
            &self.cfg.network_id,
            &control_scope,
            &self.cfg.from,
        )
        .await?;
        let nonce_hex = format!("0x{nonce:x}");
        let prepared = {
            let mut client = EvmIoClient::new(self.state_id.clone(), io);
            evm_rpc::prepare_signed_create_intent_for_network(
                &mut client,
                &self.cfg.network_id,
                &control_scope,
                "deploy",
                env_name,
                &self.cfg.from,
                &nonce_hex,
                &constructor_payload,
                self.cfg.value_hex.as_deref(),
            )
            .await?
        };
        evm_rpc::record_tx_intent(io, &self.state_id, &prepared).await?;
        let tx_hash =
            evm_rpc::broadcast_recorded_tx_intent(io, &self.state_id, &prepared.intent).await?;

        let receipt = evm_rpc::wait_for_expected_receipt(
            &self.state_id,
            io,
            &prepared.intent,
            self.cfg.poll_interval_ms,
            self.cfg.max_receipt_polls,
        )
        .await?;
        evm_rpc::ensure_receipt_success(&receipt)?;

        let contract_address = evm_rpc::receipt_contract_address(&receipt)?;
        context_write_json(
            ctx,
            KEY_CONTRACT_ADDRESS,
            serde_json::json!(contract_address),
        )?;
        context_write_json(ctx, KEY_DEPLOY_TX_HASH, serde_json::json!(tx_hash))?;
        context_write_json(ctx, KEY_DEPLOY_RECEIPT, receipt)?;

        Ok(StateOutcome {
            snapshot: SnapshotPolicy::OnSuccess,
        })
    }
}

#[async_trait]
impl State for EvmConfigureState {
    fn meta(&self) -> StateMeta {
        meta::apply_side_effect(op_idempotency::state_purpose(
            "evm_configure",
            &self.state_id,
            "apply_side_effect",
        ))
    }

    async fn handle(
        &self,
        ctx: &mut dyn DynContext,
        io: &mut dyn IoProvider,
        _rec: &mut dyn EventRecorder,
    ) -> Result<StateOutcome, StateError> {
        let artifact = resolve_artifact_config(ctx, &self.cfg.artifact, &self.cfg.artifact_port)?;
        let (abi, _bytecode) = shared_dcv::parse_artifact(&artifact).map_err(|_| {
            op_errors::state_unknown("invalid_contract_artifact", "contract artifact was invalid")
        })?;
        let to = resolve_contract_address(ctx, &self.cfg.contract_address)?;
        let control_scope = prepare_managed_sources(
            &self.state_id,
            io,
            &self.cfg.network_id,
            &self.cfg.control_scope,
        )
        .await?;

        let Some(env_name) = self.cfg.signing_key_env.as_deref() else {
            return Err(op_errors::state_unknown(
                "evm_signed_transaction_required",
                "EVM configure requires signing_key_env for durable signed transaction intent recording",
            ));
        };
        let mut next_nonce = evm_rpc::pending_nonce_u128_for_network(
            io,
            &self.state_id,
            &self.cfg.network_id,
            &control_scope,
            &self.cfg.from,
        )
        .await?;
        let mut tx_hashes: Vec<serde_json::Value> = Vec::new();
        let mut receipts: Vec<serde_json::Value> = Vec::new();

        for (idx, call) in self.cfg.calls.iter().enumerate() {
            let (calldata, _outputs) = shared_dcv::resolve_function_call(
                &abi,
                &call.function,
                &call.args,
            )
            .map_err(|_| {
                op_errors::state_unknown("invalid_op_config", "configure call did not match ABI")
            })?;

            let nonce_hex = format!("0x{next_nonce:x}");
            let logical_tx_id = format!("configure:{idx}");
            let prepared = {
                let mut client = EvmIoClient::new(self.state_id.clone(), io);
                evm_rpc::prepare_signed_call_intent_for_network(
                    &mut client,
                    &self.cfg.network_id,
                    &control_scope,
                    &logical_tx_id,
                    env_name,
                    &self.cfg.from,
                    &to,
                    &nonce_hex,
                    &calldata,
                    call.value_hex.as_deref(),
                )
                .await?
            };
            evm_rpc::record_tx_intent(io, &self.state_id, &prepared).await?;
            let tx_hash =
                evm_rpc::broadcast_recorded_tx_intent(io, &self.state_id, &prepared.intent).await?;
            next_nonce = next_nonce.checked_add(1).ok_or_else(|| {
                op_errors::state_unknown(
                    "evm_response_invalid",
                    "nonce overflow while preparing signed configure transactions",
                )
            })?;
            let receipt = evm_rpc::wait_for_expected_receipt(
                &self.state_id,
                io,
                &prepared.intent,
                self.cfg.poll_interval_ms,
                self.cfg.max_receipt_polls,
            )
            .await?;
            evm_rpc::ensure_receipt_success(&receipt)?;

            tx_hashes.push(serde_json::json!({
                "function": call.function,
                "tx_hash": tx_hash,
            }));
            receipts.push(receipt);
        }

        context_write_json(
            ctx,
            &self.cfg.tx_hashes_export_key,
            serde_json::Value::Array(tx_hashes),
        )?;
        context_write_json(
            ctx,
            &self.cfg.receipts_export_key,
            serde_json::Value::Array(receipts),
        )?;

        Ok(StateOutcome {
            snapshot: SnapshotPolicy::OnSuccess,
        })
    }
}

#[async_trait]
impl State for EvmValidateState {
    fn meta(&self) -> StateMeta {
        meta::validate()
    }

    async fn handle(
        &self,
        ctx: &mut dyn DynContext,
        io: &mut dyn IoProvider,
        _rec: &mut dyn EventRecorder,
    ) -> Result<StateOutcome, StateError> {
        let artifact = resolve_artifact_config(ctx, &self.cfg.artifact, &self.cfg.artifact_port)?;
        let (abi, _bytecode) = shared_dcv::parse_artifact(&artifact).map_err(|_| {
            op_errors::state_unknown("invalid_contract_artifact", "contract artifact was invalid")
        })?;
        let (read_assertions, event_assertions) = shared_dcv::prepare_validate_assertions(
            &abi,
            &self.cfg.read_assertions,
            &self.cfg.event_assertions,
        )
        .map_err(|err| {
            op_errors::state_unknown(
                "invalid_op_config",
                op_rpc::validation_assertion_error_message(&err),
            )
        })?;
        let control_scope = prepare_managed_sources(
            &self.state_id,
            io,
            &self.cfg.network_id,
            &self.cfg.control_scope,
        )
        .await?;

        let mut client = EvmIoClient::new(self.state_id.clone(), io);

        let client_version_res = client
            .call(managed_call(
                &self.cfg.network_id,
                &control_scope,
                "web3_clientVersion",
                serde_json::json!([]),
            ))
            .await
            .map_err(op_errors::state_from_io)?;
        let client_version = op_rpc::expect_string(
            &client_version_res.response,
            "evm_response_invalid",
            "web3_clientVersion was not a string",
        )?;
        op_rpc::assert_condition(
            client_version
                .to_ascii_lowercase()
                .contains(&self.cfg.require_client_substring.to_ascii_lowercase()),
            "reth_client_mismatch",
            "rpc clientVersion did not match required reth substring",
        )?;

        let chain_id = client
            .call(managed_call(
                &self.cfg.network_id,
                &control_scope,
                "eth_chainId",
                serde_json::json!([]),
            ))
            .await
            .map_err(op_errors::state_from_io)
            .and_then(|res| {
                parse_u64_hex_value(&res.response).map_err(|_| {
                    op_errors::state_unknown(
                        "evm_response_invalid",
                        "eth_chainId returned invalid hex chain id",
                    )
                })
            })?;
        op_rpc::assert_condition(
            chain_id == self.cfg.expected_chain_id,
            "chain_id_mismatch",
            "rpc chain id did not match expected_chain_id",
        )?;

        let to = resolve_contract_address(ctx, &self.cfg.contract_address)?;

        for ra in &read_assertions {
            let res = client
                .call(managed_call(
                    &self.cfg.network_id,
                    &control_scope,
                    "eth_call",
                    serde_json::json!([{ "to": to, "data": ra.data_hex }, "latest"]),
                ))
                .await
                .map_err(op_errors::state_from_io)?;

            let raw = op_rpc::expect_string(
                &res.response,
                "evm_response_invalid",
                "eth_call returned non-string",
            )?;

            let actual =
                shared_dcv::decode_single_output_to_json(&ra.outputs, &raw).map_err(|_| {
                    op_errors::state_unknown(
                        "evm_response_invalid",
                        "failed to decode eth_call output",
                    )
                })?;
            let actual = shared_dcv::ExpectedValue::from_json_value(&actual).map_err(|_| {
                op_errors::state_unknown(
                    "evm_response_invalid",
                    "decoded eth_call output was not canonical JSON",
                )
            })?;

            op_rpc::assert_condition(
                shared_dcv::expected_matches(&actual, &ra.expected),
                "validation_failed",
                "read assertion failed during evm_validate",
            )?;
        }

        for ea in &event_assertions {
            let logs_res = client
                .call(managed_call(
                    &self.cfg.network_id,
                    &control_scope,
                    "eth_getLogs",
                    serde_json::json!([{
                        "address": to,
                        "fromBlock": ea.from_block,
                        "toBlock": ea.to_block,
                        "topics": [ea.topic0_hex],
                    }]),
                ))
                .await
                .map_err(op_errors::state_from_io)?;

            let logs = op_rpc::expect_array(
                &logs_res.response,
                "evm_response_invalid",
                "eth_getLogs returned non-array",
            )?;
            op_rpc::assert_condition(
                u64::try_from(logs.len()).unwrap_or(0) >= ea.min_count,
                "validation_failed",
                "event assertion failed during evm_validate",
            )?;

            let _ = &ea.event;
        }

        context_write_json(ctx, KEY_CHAIN_ID, serde_json::json!(chain_id))?;
        context_write_json(ctx, KEY_CLIENT_VERSION, serde_json::json!(client_version))?;
        context_write_json(ctx, KEY_VALIDATED, serde_json::json!(true))?;

        Ok(StateOutcome {
            snapshot: SnapshotPolicy::OnSuccess,
        })
    }
}

pub(crate) fn resolve_artifact_config(
    ctx: &dyn DynContext,
    configured: &Option<shared_dcv::ContractArtifactConfig>,
    artifact_port: &str,
) -> Result<shared_dcv::ContractArtifactConfig, StateError> {
    if let Some(artifact) = configured {
        return Ok(artifact.clone());
    }

    op_ctx::read_typed(
        ctx,
        &ContextKey(artifact_port.to_string()),
        "ctx_missing_key",
        "required context key was missing",
        "ctx_type_mismatch",
        "context artifact value was invalid",
    )
}

fn context_write_json(
    ctx: &mut dyn DynContext,
    key: &str,
    value: serde_json::Value,
) -> Result<(), StateError> {
    op_ctx::write_json(ctx, ContextKey(key.to_string()), value)
}

pub(crate) fn resolve_contract_address(
    ctx: &dyn DynContext,
    configured: &Option<String>,
) -> Result<String, StateError> {
    match configured {
        Some(a) => shared_dcv::normalize_address(a).map_err(|_| {
            op_errors::state_unknown("invalid_op_config", "contract_address was invalid")
        }),
        None => {
            let a = op_ctx::read_typed::<String>(
                ctx,
                &ContextKey(KEY_CONTRACT_ADDRESS.to_string()),
                "ctx_missing_key",
                "required context key was missing",
                "ctx_type_mismatch",
                "context value was not a string",
            )?;
            shared_dcv::normalize_address(&a).map_err(|_| {
                op_errors::state_unknown("ctx_type_mismatch", "context contract address invalid")
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use async_trait::async_trait;
    use mfm_machine::errors::{ContextError, ErrorInfo, IoError, RunError};
    use mfm_machine::events::DomainEvent;
    use mfm_machine::ids::{ArtifactId, ErrorCode, FactKey};
    use mfm_machine::io::{IoCall, IoResult};
    use serde_json::Value;
    use zeroize::Zeroizing;

    #[derive(Default)]
    struct MapContext {
        values: std::collections::BTreeMap<String, serde_json::Value>,
    }

    impl DynContext for MapContext {
        fn read(&self, key: &ContextKey) -> Result<Option<serde_json::Value>, ContextError> {
            Ok(self.values.get(&key.0).cloned())
        }

        fn write(&mut self, key: ContextKey, value: serde_json::Value) -> Result<(), ContextError> {
            self.values.insert(key.0, value);
            Ok(())
        }

        fn delete(&mut self, key: &ContextKey) -> Result<(), ContextError> {
            self.values.remove(&key.0);
            Ok(())
        }

        fn dump(&self) -> Result<serde_json::Value, ContextError> {
            let mut out = serde_json::Map::new();
            for (key, value) in &self.values {
                out.insert(key.clone(), value.clone());
            }
            Ok(serde_json::Value::Object(out))
        }
    }

    #[derive(Default)]
    struct NoopRecorder;

    #[async_trait]
    impl EventRecorder for NoopRecorder {
        async fn emit(&mut self, _event: DomainEvent) -> Result<(), RunError> {
            Ok(())
        }

        async fn emit_many(&mut self, _events: Vec<DomainEvent>) -> Result<(), RunError> {
            Ok(())
        }
    }

    #[derive(Default)]
    struct TrackingIo {
        calls: Vec<IoCall>,
        prepare_sources_healthy: bool,
        recorded_values: Vec<(FactKey, serde_json::Value)>,
        protected_values: std::collections::HashMap<FactKey, Vec<u8>>,
        fail_broadcast_once: bool,
    }

    fn io_info(code: &'static str, message: impl Into<String>) -> ErrorInfo {
        ErrorInfo {
            code: ErrorCode::must_new(code),
            category: ErrorCategory::Unknown,
            retryable: false,
            message: message.into(),
            details: None,
        }
    }

    #[async_trait]
    impl IoProvider for TrackingIo {
        async fn call(&mut self, call: IoCall) -> Result<IoResult, IoError> {
            self.calls.push(call.clone());

            if call.namespace == "local.evm.sign_legacy" {
                return Ok(IoResult {
                    response: serde_json::json!({ "raw_tx_hex": "0x01" }),
                    recorded_payload_id: None,
                });
            }

            let kind = call
                .request
                .get("kind")
                .and_then(Value::as_str)
                .unwrap_or_default();
            match kind {
                "prepare_sources" => Ok(IoResult {
                    response: serde_json::json!({
                        "control_scope": call.request.get("control_scope").and_then(Value::as_str).unwrap_or("shared"),
                        "network_id": call.request.get("network_id").and_then(Value::as_str).unwrap_or("ethereum-mainnet"),
                        "pool_kind": "test",
                        "available_source_ids": ["source-1"],
                        "ranked_source_ids": ["source-1"],
                        "sources": [{
                            "source_id": "source-1",
                            "healthy": self.prepare_sources_healthy,
                        }],
                    }),
                    recorded_payload_id: None,
                }),
                "evm_call" => {
                    let method = call
                        .request
                        .get("method")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    let response = match method {
                        "eth_estimateGas" => serde_json::json!("0x5208"),
                        "eth_gasPrice" => serde_json::json!("0x1"),
                        "eth_getTransactionCount" => serde_json::json!("0x0"),
                        "eth_getTransactionReceipt" => serde_json::json!({ "status": "0x1" }),
                        "web3_clientVersion" => serde_json::json!("reth/v1.0.0"),
                        "eth_chainId" => serde_json::json!("0x1"),
                        other => {
                            return Err(IoError::Other(io_info(
                                "unexpected_method",
                                format!("unexpected method `{other}`"),
                            )));
                        }
                    };
                    Ok(IoResult {
                        response,
                        recorded_payload_id: None,
                    })
                }
                "evm_broadcast_raw_transaction" => {
                    if self.fail_broadcast_once {
                        self.fail_broadcast_once = false;
                        return Err(IoError::Other(io_info(
                            "simulated_broadcast_crash",
                            "simulated crash after intent recording",
                        )));
                    }
                    Ok(IoResult {
                        response: serde_json::json!({
                            "tx_hash": call.request.get("expected_tx_hash").cloned().unwrap_or(Value::Null),
                        }),
                        recorded_payload_id: None,
                    })
                }
                _ => Err(IoError::Other(io_info(
                    "unexpected_call_kind",
                    "unexpected call kind",
                ))),
            }
        }

        async fn record_value(
            &mut self,
            key: FactKey,
            value: serde_json::Value,
        ) -> Result<ArtifactId, IoError> {
            self.recorded_values.push((key, value));
            Ok(ArtifactId::must_new("0".repeat(64)))
        }

        async fn record_protected_bytes(
            &mut self,
            key: FactKey,
            bytes: Zeroizing<Vec<u8>>,
        ) -> Result<ArtifactId, IoError> {
            self.protected_values.insert(key, bytes.to_vec());
            Ok(ArtifactId::must_new("1".repeat(64)))
        }

        async fn read_protected_bytes(
            &mut self,
            key: &FactKey,
        ) -> Result<Zeroizing<Vec<u8>>, IoError> {
            self.protected_values
                .get(key)
                .cloned()
                .map(Zeroizing::new)
                .ok_or_else(|| IoError::Other(io_info("protected_missing", "protected missing")))
        }

        async fn get_recorded_fact(
            &mut self,
            _key: &FactKey,
        ) -> Result<Option<ArtifactId>, IoError> {
            Ok(None)
        }

        async fn now_millis(&mut self) -> Result<u64, IoError> {
            Ok(0)
        }

        async fn random_bytes(&mut self, n: usize) -> Result<Vec<u8>, IoError> {
            Ok(vec![0; n])
        }

        async fn sleep_ms(&mut self, _duration_ms: u64) -> Result<(), IoError> {
            Ok(())
        }
    }

    fn sample_artifact_config() -> shared_dcv::ContractArtifactConfig {
        serde_json::from_value(serde_json::json!({
            "abi": [
                {
                    "type": "function",
                    "name": "setValue",
                    "inputs": [{"name": "x", "type": "uint256"}],
                    "outputs": []
                }
            ],
            "bytecode": {
                "object": "0x60006000"
            }
        }))
        .expect("artifact config")
    }

    #[tokio::test]
    async fn configure_prepares_sources_before_write_calls() {
        let mut ctx = MapContext::default();
        ctx.write(
            ContextKey(KEY_CONTRACT_ADDRESS.to_string()),
            serde_json::json!("0x1111111111111111111111111111111111111111"),
        )
        .expect("write contract address");
        let mut io = TrackingIo {
            calls: Vec::new(),
            prepare_sources_healthy: true,
            recorded_values: Vec::new(),
            protected_values: std::collections::HashMap::new(),
            fail_broadcast_once: false,
        };
        let mut rec = NoopRecorder;

        EvmConfigureState {
            state_id: StateId::must_new("evm.write.configure".to_string()),
            cfg: EvmConfigureStateConfig {
                artifact: Some(sample_artifact_config()),
                artifact_port: KEY_CONTRACT_ARTIFACT.to_string(),
                network_id: "ethereum-mainnet".to_string(),
                control_scope: "".to_string(),
                from: "0x1111111111111111111111111111111111111111".to_string(),
                signing_key_env: Some("MFM_DEPLOYER_KEY".to_string()),
                contract_address: None,
                calls: vec![EvmConfigureRuntimeCall {
                    function: "setValue".to_string(),
                    args: vec![
                        shared_dcv::AbiArgumentValue::from_json_value(&serde_json::json!(1))
                            .expect("arg"),
                    ],
                    value_hex: None,
                }],
                tx_hashes_export_key: "configure_tx_hashes".to_string(),
                receipts_export_key: "configure_receipts".to_string(),
                poll_interval_ms: 0,
                max_receipt_polls: 1,
            },
        }
        .handle(&mut ctx, &mut io, &mut rec)
        .await
        .expect("configure");

        assert_eq!(
            io.calls[0].request.get("kind").and_then(Value::as_str),
            Some("prepare_sources")
        );
        assert_eq!(
            io.calls[0]
                .request
                .get("control_scope")
                .and_then(Value::as_str),
            Some("shared")
        );
        assert_eq!(
            io.calls[1].request.get("method").and_then(Value::as_str),
            Some("eth_getTransactionCount")
        );
        assert!(io
            .recorded_values
            .iter()
            .any(|(key, _)| key.0.contains("mfm:evm.tx_intent")));
    }

    #[tokio::test]
    async fn validate_prepares_sources_before_read_assertions() {
        let mut ctx = MapContext::default();
        ctx.write(
            ContextKey(KEY_CONTRACT_ADDRESS.to_string()),
            serde_json::json!("0x1111111111111111111111111111111111111111"),
        )
        .expect("write contract address");
        let mut io = TrackingIo {
            calls: Vec::new(),
            prepare_sources_healthy: true,
            recorded_values: Vec::new(),
            protected_values: std::collections::HashMap::new(),
            fail_broadcast_once: false,
        };
        let mut rec = NoopRecorder;

        EvmValidateState {
            state_id: StateId::must_new("evm.write.validate".to_string()),
            cfg: EvmValidateStateConfig {
                artifact: Some(sample_artifact_config()),
                artifact_port: KEY_CONTRACT_ARTIFACT.to_string(),
                network_id: "ethereum-mainnet".to_string(),
                control_scope: "shared".to_string(),
                contract_address: None,
                expected_chain_id: 1,
                require_client_substring: "reth".to_string(),
                read_assertions: Vec::new(),
                event_assertions: Vec::new(),
            },
        }
        .handle(&mut ctx, &mut io, &mut rec)
        .await
        .expect("validate");

        assert_eq!(
            io.calls[0].request.get("kind").and_then(Value::as_str),
            Some("prepare_sources")
        );
        assert_eq!(
            io.calls[1].request.get("method").and_then(Value::as_str),
            Some("web3_clientVersion")
        );
    }

    #[tokio::test]
    async fn configure_rejects_node_managed_unsigned_writes() {
        let mut ctx = MapContext::default();
        ctx.write(
            ContextKey(KEY_CONTRACT_ADDRESS.to_string()),
            serde_json::json!("0x1111111111111111111111111111111111111111"),
        )
        .expect("write contract address");
        let mut io = TrackingIo {
            calls: Vec::new(),
            prepare_sources_healthy: true,
            recorded_values: Vec::new(),
            protected_values: std::collections::HashMap::new(),
            fail_broadcast_once: false,
        };
        let mut rec = NoopRecorder;

        let err = EvmConfigureState {
            state_id: StateId::must_new("evm.write.configure".to_string()),
            cfg: EvmConfigureStateConfig {
                artifact: Some(sample_artifact_config()),
                artifact_port: KEY_CONTRACT_ARTIFACT.to_string(),
                network_id: "ethereum-mainnet".to_string(),
                control_scope: "shared".to_string(),
                from: "0x1111111111111111111111111111111111111111".to_string(),
                signing_key_env: None,
                contract_address: None,
                calls: vec![EvmConfigureRuntimeCall {
                    function: "setValue".to_string(),
                    args: vec![
                        shared_dcv::AbiArgumentValue::from_json_value(&serde_json::json!(1))
                            .expect("arg"),
                    ],
                    value_hex: None,
                }],
                tx_hashes_export_key: "configure_tx_hashes".to_string(),
                receipts_export_key: "configure_receipts".to_string(),
                poll_interval_ms: 0,
                max_receipt_polls: 1,
            },
        }
        .handle(&mut ctx, &mut io, &mut rec)
        .await
        .expect_err("unsigned configure must be rejected");

        assert_eq!(err.info.code.as_str(), "evm_signed_transaction_required");
        assert!(!io.calls.iter().any(|call| {
            call.request.get("method").and_then(Value::as_str) == Some("eth_sendTransaction")
        }));
    }

    #[tokio::test]
    async fn configure_resume_after_intent_recording_reuses_same_intent() {
        let mut ctx = MapContext::default();
        ctx.write(
            ContextKey(KEY_CONTRACT_ADDRESS.to_string()),
            serde_json::json!("0x1111111111111111111111111111111111111111"),
        )
        .expect("write contract address");
        let state = EvmConfigureState {
            state_id: StateId::must_new("evm.write.configure".to_string()),
            cfg: EvmConfigureStateConfig {
                artifact: Some(sample_artifact_config()),
                artifact_port: KEY_CONTRACT_ARTIFACT.to_string(),
                network_id: "ethereum-mainnet".to_string(),
                control_scope: "shared".to_string(),
                from: "0x1111111111111111111111111111111111111111".to_string(),
                signing_key_env: Some("MFM_DEPLOYER_KEY".to_string()),
                contract_address: None,
                calls: vec![EvmConfigureRuntimeCall {
                    function: "setValue".to_string(),
                    args: vec![
                        shared_dcv::AbiArgumentValue::from_json_value(&serde_json::json!(1))
                            .expect("arg"),
                    ],
                    value_hex: None,
                }],
                tx_hashes_export_key: "configure_tx_hashes".to_string(),
                receipts_export_key: "configure_receipts".to_string(),
                poll_interval_ms: 0,
                max_receipt_polls: 1,
            },
        };
        let mut io = TrackingIo {
            calls: Vec::new(),
            prepare_sources_healthy: true,
            recorded_values: Vec::new(),
            protected_values: std::collections::HashMap::new(),
            fail_broadcast_once: true,
        };
        let mut rec = NoopRecorder;

        let err = state
            .handle(&mut ctx, &mut io, &mut rec)
            .await
            .expect_err("first broadcast is interrupted after intent recording");
        assert_eq!(err.info.code.as_str(), "simulated_broadcast_crash");
        let first_intent = io
            .recorded_values
            .first()
            .expect("intent recorded before broadcast")
            .1
            .clone();

        state
            .handle(&mut ctx, &mut io, &mut rec)
            .await
            .expect("resume should broadcast the same logical intent");
        let second_intent = io
            .recorded_values
            .get(1)
            .expect("intent recorded again on retry")
            .1
            .clone();

        assert_eq!(first_intent, second_intent);
        assert!(io.calls.iter().any(|call| {
            call.request.get("kind").and_then(Value::as_str)
                == Some("evm_broadcast_raw_transaction")
                && call.request.get("expected_tx_hash") == first_intent.get("raw_tx_hash")
        }));
    }

    #[tokio::test]
    async fn deploy_fails_fast_when_no_managed_sources_are_healthy() {
        let mut ctx = MapContext::default();
        let mut io = TrackingIo {
            calls: Vec::new(),
            prepare_sources_healthy: false,
            recorded_values: Vec::new(),
            protected_values: std::collections::HashMap::new(),
            fail_broadcast_once: false,
        };
        let mut rec = NoopRecorder;

        let err = EvmDeployState {
            state_id: StateId::must_new("evm.write.deploy".to_string()),
            cfg: EvmDeployStateConfig {
                artifact: Some(sample_artifact_config()),
                artifact_port: KEY_CONTRACT_ARTIFACT.to_string(),
                network_id: "ethereum-mainnet".to_string(),
                control_scope: "shared".to_string(),
                from: "0x1111111111111111111111111111111111111111".to_string(),
                constructor_args: Vec::new(),
                value_hex: None,
                signing_key_env: None,
                poll_interval_ms: 0,
                max_receipt_polls: 1,
            },
        }
        .handle(&mut ctx, &mut io, &mut rec)
        .await
        .expect_err("deploy should fail before issuing write calls");

        assert_eq!(err.info.code.as_str(), "rpc_control_no_healthy_sources");
        assert!(err.info.retryable);
        assert_eq!(io.calls.len(), 1);
        assert_eq!(
            io.calls[0].request.get("kind").and_then(Value::as_str),
            Some("prepare_sources")
        );
    }
}
