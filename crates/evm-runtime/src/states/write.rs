//! Reusable EVM deploy/configure/validate states.
//!
//! These states own the executable runtime behavior behind contract artifact adaptation, contract
//! deployment, post-deploy runtime calls, and validation assertions against live or replayed IO.
//!
//! Thin op crates should compose these states rather than reimplementing write-path behavior.

use async_trait::async_trait;
use mfm_collectors_rpc_control::{EvmIoClient, JsonRpcCall};
use mfm_machine::context::DynContext;
use mfm_machine::errors::StateError;
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

use crate::dcv as shared_dcv;
use crate::rpc as evm_rpc;

const KEY_NIX_RESULT: &str = "result";
const KEY_CONTRACT_ARTIFACT: &str = "contract_artifact";
const KEY_CONTRACT_ADDRESS: &str = "contract_address";
const KEY_DEPLOY_TX_HASH: &str = "deploy_tx_hash";
const KEY_DEPLOY_RECEIPT: &str = "deploy_receipt";
const KEY_CONFIGURE_TX_HASHES: &str = "configure_tx_hashes";
const KEY_CONFIGURE_RECEIPTS: &str = "configure_receipts";
const KEY_VALIDATED: &str = "validated";
const KEY_CHAIN_ID: &str = "chain_id";
const KEY_CLIENT_VERSION: &str = "client_version";

/// Runtime configuration for [`EvmDeployState`].
///
/// # Examples
///
/// ```rust
/// use mfm_evm_runtime::states::write::EvmDeployStateConfig;
///
/// let cfg = EvmDeployStateConfig {
///     artifact: None,
///     artifact_port: "contract_artifact".to_string(),
///     from: "0x0000000000000000000000000000000000000001".to_string(),
///     constructor_args: vec![serde_json::json!(42)],
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
    /// Deployer address or sender address.
    pub from: String,
    /// Constructor arguments passed during deployment.
    pub constructor_args: Vec<serde_json::Value>,
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
    pub args: Vec<serde_json::Value>,
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
    /// Sender address used for configuration transactions.
    pub from: String,
    /// Optional inline contract address; falls back to context when absent.
    pub contract_address: Option<String>,
    /// Calls to execute against the deployed contract.
    pub calls: Vec<EvmConfigureRuntimeCall>,
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

        let mut client = EvmIoClient::new(self.state_id.clone(), io);

        let mut tx = serde_json::json!({
            "from": self.cfg.from,
            "data": shared_dcv::bytes_to_hex_prefixed(&constructor_payload),
        });
        if let Some(v) = &self.cfg.value_hex {
            tx["value"] = serde_json::json!(v);
        }

        let tx_hash = if let Some(env_name) = self.cfg.signing_key_env.as_deref() {
            evm_rpc::send_signed_create_transaction(
                &mut client,
                env_name,
                &self.cfg.from,
                &constructor_payload,
                self.cfg.value_hex.as_deref(),
            )
            .await?
        } else {
            evm_rpc::send_transaction(&mut client, tx).await?
        };
        drop(client);

        let receipt = evm_rpc::wait_for_receipt(
            &self.state_id,
            io,
            &tx_hash,
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

        let mut tx_hashes: Vec<serde_json::Value> = Vec::new();
        let mut receipts: Vec<serde_json::Value> = Vec::new();

        for call in &self.cfg.calls {
            let (calldata, _outputs) = shared_dcv::resolve_function_call(
                &abi,
                &call.function,
                &call.args,
            )
            .map_err(|_| {
                op_errors::state_unknown("invalid_op_config", "configure call did not match ABI")
            })?;

            let mut tx = serde_json::json!({
                "from": self.cfg.from,
                "to": to,
                "data": shared_dcv::bytes_to_hex_prefixed(&calldata),
            });
            if let Some(v) = &call.value_hex {
                tx["value"] = serde_json::json!(v);
            }

            let mut client = EvmIoClient::new(self.state_id.clone(), io);
            let tx_hash = evm_rpc::send_transaction(&mut client, tx).await?;
            drop(client);
            let receipt = evm_rpc::wait_for_receipt(
                &self.state_id,
                io,
                &tx_hash,
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
            KEY_CONFIGURE_TX_HASHES,
            serde_json::Value::Array(tx_hashes),
        )?;
        context_write_json(
            ctx,
            KEY_CONFIGURE_RECEIPTS,
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

        let mut client = EvmIoClient::new(self.state_id.clone(), io);

        let client_version_res = client
            .call(JsonRpcCall::new(
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
            .chain_id_u64()
            .await
            .map_err(op_errors::state_from_io)?;
        op_rpc::assert_condition(
            chain_id == self.cfg.expected_chain_id,
            "chain_id_mismatch",
            "rpc chain id did not match expected_chain_id",
        )?;

        let to = resolve_contract_address(ctx, &self.cfg.contract_address)?;

        for ra in &read_assertions {
            let res = client
                .call(JsonRpcCall::new(
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

            op_rpc::assert_condition(
                shared_dcv::expected_matches(&actual, &ra.expected),
                "validation_failed",
                "read assertion failed during evm_validate",
            )?;
        }

        for ea in &event_assertions {
            let logs_res = client
                .call(JsonRpcCall::new(
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
