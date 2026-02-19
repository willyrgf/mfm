use async_trait::async_trait;
use mfm_collectors_evm::{EvmIoClient, JsonRpcCall};
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

#[derive(Clone, Debug)]
pub struct EvmDeployStateConfig {
    pub artifact: Option<shared_dcv::ContractArtifactConfig>,
    pub artifact_port: String,
    pub from: String,
    pub constructor_args: Vec<serde_json::Value>,
    pub value_hex: Option<String>,
    pub signing_key_env: Option<String>,
    pub poll_interval_ms: u64,
    pub max_receipt_polls: u64,
}

#[derive(Clone, Debug)]
pub struct EvmConfigureRuntimeCall {
    pub function: String,
    pub args: Vec<serde_json::Value>,
    pub value_hex: Option<String>,
}

#[derive(Clone, Debug)]
pub struct EvmConfigureStateConfig {
    pub artifact: Option<shared_dcv::ContractArtifactConfig>,
    pub artifact_port: String,
    pub from: String,
    pub contract_address: Option<String>,
    pub calls: Vec<EvmConfigureRuntimeCall>,
    pub poll_interval_ms: u64,
    pub max_receipt_polls: u64,
}

#[derive(Clone, Debug)]
pub struct EvmValidateStateConfig {
    pub artifact: Option<shared_dcv::ContractArtifactConfig>,
    pub artifact_port: String,
    pub contract_address: Option<String>,
    pub expected_chain_id: u64,
    pub require_client_substring: String,
    pub read_assertions: Vec<shared_dcv::ReadAssertionConfig>,
    pub event_assertions: Vec<shared_dcv::EventAssertionConfig>,
}

#[derive(Clone, Debug)]
pub struct NixArtifactToEvmContractState {
    pub result_pointer: String,
}

#[derive(Clone, Debug)]
pub struct EvmDeployState {
    pub state_id: StateId,
    pub cfg: EvmDeployStateConfig,
}

#[derive(Clone, Debug)]
pub struct EvmConfigureState {
    pub state_id: StateId,
    pub cfg: EvmConfigureStateConfig,
}

#[derive(Clone, Debug)]
pub struct EvmValidateState {
    pub state_id: StateId,
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
        let result = context_read_json(ctx, KEY_NIX_RESULT)?;

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

fn context_read_string(ctx: &dyn DynContext, key: &str) -> Result<String, StateError> {
    op_ctx::read_string_required(
        ctx,
        &ContextKey(key.to_string()),
        "ctx_missing_key",
        "required context key was missing",
        "ctx_type_mismatch",
        "context value was not a string",
    )
}

fn context_read_json(ctx: &dyn DynContext, key: &str) -> Result<serde_json::Value, StateError> {
    op_ctx::read_json_required(
        ctx,
        &ContextKey(key.to_string()),
        "ctx_missing_key",
        "required context key was missing",
    )
}

pub(crate) fn resolve_artifact_config(
    ctx: &dyn DynContext,
    configured: &Option<shared_dcv::ContractArtifactConfig>,
    artifact_port: &str,
) -> Result<shared_dcv::ContractArtifactConfig, StateError> {
    if let Some(artifact) = configured {
        return Ok(artifact.clone());
    }

    let v = context_read_json(ctx, artifact_port)?;
    serde_json::from_value::<shared_dcv::ContractArtifactConfig>(v).map_err(|_| {
        op_errors::state_unknown("ctx_type_mismatch", "context artifact value was invalid")
    })
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
            let a = context_read_string(ctx, KEY_CONTRACT_ADDRESS)?;
            shared_dcv::normalize_address(&a).map_err(|_| {
                op_errors::state_unknown("ctx_type_mismatch", "context contract address invalid")
            })
        }
    }
}
