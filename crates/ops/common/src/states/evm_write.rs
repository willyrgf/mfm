use std::time::Duration;

use async_trait::async_trait;
use mfm_collectors_evm::{EvmIoClient, JsonRpcCall};
use mfm_machine::context::DynContext;
use mfm_machine::errors::StateError;
use mfm_machine::hashing::{artifact_id_for_json, CanonicalJsonError};
use mfm_machine::ids::{ContextKey, FactKey, StateId};
use mfm_machine::io::{IoCall, IoProvider};
use mfm_machine::meta::StateMeta;
use mfm_machine::recorder::EventRecorder;
use mfm_machine::state::{SnapshotPolicy, State, StateOutcome};

use crate::ctx as op_ctx;
use crate::errors as op_errors;
use crate::idempotency as op_idempotency;
use crate::rpc as op_rpc;
use crate::states::evm_dcv as shared_dcv;
use crate::states::meta;

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
            send_signed_create_transaction(
                &mut client,
                env_name,
                &self.cfg.from,
                &constructor_payload,
                self.cfg.value_hex.as_deref(),
            )
            .await?
        } else {
            send_transaction(&mut client, tx).await?
        };
        drop(client);

        let receipt = wait_for_receipt(
            &self.state_id,
            io,
            &tx_hash,
            self.cfg.poll_interval_ms,
            self.cfg.max_receipt_polls,
        )
        .await?;
        ensure_receipt_success(&receipt)?;

        let contract_address = receipt_contract_address(&receipt)?;
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
            let tx_hash = send_transaction(&mut client, tx).await?;
            drop(client);
            let receipt = wait_for_receipt(
                &self.state_id,
                io,
                &tx_hash,
                self.cfg.poll_interval_ms,
                self.cfg.max_receipt_polls,
            )
            .await?;
            ensure_receipt_success(&receipt)?;

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

fn resolve_artifact_config(
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

async fn send_transaction(
    client: &mut EvmIoClient<'_>,
    mut tx_obj: serde_json::Value,
) -> Result<String, StateError> {
    if tx_obj.get("gas").is_none() {
        let gas = estimate_gas_hex(client, &tx_obj).await?;
        tx_obj["gas"] = serde_json::json!(gas);
    }
    if tx_obj.get("gasPrice").is_none() && tx_obj.get("maxFeePerGas").is_none() {
        let gas_price = gas_price_hex(client).await?;
        tx_obj["gasPrice"] = serde_json::json!(gas_price);
    }

    let res = client
        .call(JsonRpcCall::new(
            "eth_sendTransaction",
            serde_json::json!([tx_obj]),
        ))
        .await
        .map_err(op_errors::state_from_io)?;

    let Some(tx_hash) = res.response.as_str() else {
        return Err(op_errors::state_unknown(
            "evm_response_invalid",
            "eth_sendTransaction returned non-string tx hash",
        ));
    };

    shared_dcv::normalize_hex_str(tx_hash).map_err(|_| {
        op_errors::state_unknown(
            "evm_response_invalid",
            "eth_sendTransaction returned invalid hex tx hash",
        )
    })
}

async fn send_raw_transaction(
    client: &mut EvmIoClient<'_>,
    raw_tx_hex: &str,
) -> Result<String, StateError> {
    let res = client
        .call(JsonRpcCall::new(
            "eth_sendRawTransaction",
            serde_json::json!([raw_tx_hex]),
        ))
        .await
        .map_err(op_errors::state_from_io)?;

    let Some(tx_hash) = res.response.as_str() else {
        return Err(op_errors::state_unknown(
            "evm_response_invalid",
            "eth_sendRawTransaction returned non-string tx hash",
        ));
    };

    shared_dcv::normalize_hex_str(tx_hash).map_err(|_| {
        op_errors::state_unknown(
            "evm_response_invalid",
            "eth_sendRawTransaction returned invalid hex tx hash",
        )
    })
}

async fn send_signed_create_transaction(
    client: &mut EvmIoClient<'_>,
    signing_key_env: &str,
    from: &str,
    constructor_payload: &[u8],
    value_hex: Option<&str>,
) -> Result<String, StateError> {
    let configured_from = shared_dcv::normalize_address(from)
        .map_err(|_| op_errors::state_unknown("invalid_op_config", "invalid from address"))?;

    let tx_obj = {
        let mut tx = serde_json::json!({
            "from": configured_from,
            "data": shared_dcv::bytes_to_hex_prefixed(constructor_payload),
        });
        if let Some(v) = value_hex {
            tx["value"] = serde_json::json!(v);
        }
        tx
    };

    let nonce_hex = transaction_count_hex(client, &configured_from).await?;
    let gas_hex = estimate_gas_hex(client, &tx_obj).await?;
    let gas_price_hex = gas_price_hex(client).await?;
    let chain_id = client
        .chain_id_u64()
        .await
        .map_err(op_errors::state_from_io)?;

    let raw_tx_hex = local_sign_legacy_create_raw_tx(
        client,
        LegacyCreateTxSigningRequest {
            signing_key_env,
            from: &configured_from,
            chain_id,
            nonce_hex: &nonce_hex,
            gas_price_hex: &gas_price_hex,
            gas_limit_hex: &gas_hex,
            value_hex: value_hex.unwrap_or("0x0"),
            constructor_payload,
        },
    )
    .await?;

    send_raw_transaction(client, &raw_tx_hex).await
}

struct LegacyCreateTxSigningRequest<'a> {
    signing_key_env: &'a str,
    from: &'a str,
    chain_id: u64,
    nonce_hex: &'a str,
    gas_price_hex: &'a str,
    gas_limit_hex: &'a str,
    value_hex: &'a str,
    constructor_payload: &'a [u8],
}

async fn local_sign_legacy_create_raw_tx(
    client: &mut EvmIoClient<'_>,
    req: LegacyCreateTxSigningRequest<'_>,
) -> Result<String, StateError> {
    let state_id = client.state_id().clone();
    let request = serde_json::json!({
        "env_name_hex": hex::encode(req.signing_key_env.as_bytes()),
        "from": req.from,
        "chain_id": req.chain_id,
        "nonce_hex": req.nonce_hex,
        "gas_price_hex": req.gas_price_hex,
        "gas_limit_hex": req.gas_limit_hex,
        "value_hex": req.value_hex,
        "data_hex": shared_dcv::bytes_to_hex_prefixed(req.constructor_payload),
    });
    let fact_key = local_fact_key(&state_id, "deploy_sign_legacy_create", &request)?;
    let res = client
        .io_mut()
        .call(IoCall {
            namespace: "local.evm.sign_legacy_create".to_string(),
            request,
            fact_key: Some(fact_key),
        })
        .await
        .map_err(op_errors::state_from_io)?;

    let Some(raw_tx_hex) = res.response.get("raw_tx_hex").and_then(|v| v.as_str()) else {
        return Err(op_errors::state_unknown(
            "evm_response_invalid",
            "local signer returned non-string raw transaction",
        ));
    };

    shared_dcv::normalize_hex_str(raw_tx_hex).map_err(|_| {
        op_errors::state_unknown(
            "evm_response_invalid",
            "local signer returned invalid raw transaction hex",
        )
    })
}

fn local_fact_key(
    state_id: &StateId,
    purpose: &str,
    request: &serde_json::Value,
) -> Result<FactKey, StateError> {
    let req_id = artifact_id_for_json(request).map_err(|err| match err {
        CanonicalJsonError::FloatNotAllowed => op_errors::state_unknown(
            "local_request_not_canonical",
            "local io request was not canonical-json-hashable (floats are forbidden)",
        ),
        CanonicalJsonError::SecretsNotAllowed => {
            op_errors::state_unknown("secrets_detected", "local io request contained secrets")
        }
    })?;

    Ok(FactKey(format!(
        "mfm:local|state:{}|purpose:{purpose}|req:{}",
        state_id.0, req_id.0
    )))
}

async fn transaction_count_hex(
    client: &mut EvmIoClient<'_>,
    from: &str,
) -> Result<String, StateError> {
    let res = client
        .call(JsonRpcCall::new(
            "eth_getTransactionCount",
            serde_json::json!([from, "pending"]),
        ))
        .await
        .map_err(op_errors::state_from_io)?;

    let Some(nonce) = res.response.as_str() else {
        return Err(op_errors::state_unknown(
            "evm_response_invalid",
            "eth_getTransactionCount returned non-string nonce",
        ));
    };

    shared_dcv::normalize_hex_str(nonce).map_err(|_| {
        op_errors::state_unknown(
            "evm_response_invalid",
            "eth_getTransactionCount returned invalid hex nonce",
        )
    })
}

async fn estimate_gas_hex(
    client: &mut EvmIoClient<'_>,
    tx_obj: &serde_json::Value,
) -> Result<String, StateError> {
    let res = client
        .call(JsonRpcCall::new(
            "eth_estimateGas",
            serde_json::json!([tx_obj]),
        ))
        .await
        .map_err(op_errors::state_from_io)?;

    let Some(gas) = res.response.as_str() else {
        return Err(op_errors::state_unknown(
            "evm_response_invalid",
            "eth_estimateGas returned non-string gas value",
        ));
    };

    shared_dcv::normalize_hex_str(gas).map_err(|_| {
        op_errors::state_unknown(
            "evm_response_invalid",
            "eth_estimateGas returned invalid hex gas value",
        )
    })
}

async fn gas_price_hex(client: &mut EvmIoClient<'_>) -> Result<String, StateError> {
    let res = client
        .call(JsonRpcCall::new("eth_gasPrice", serde_json::json!([])))
        .await
        .map_err(op_errors::state_from_io)?;

    let Some(gas_price) = res.response.as_str() else {
        return Err(op_errors::state_unknown(
            "evm_response_invalid",
            "eth_gasPrice returned non-string gas price",
        ));
    };

    shared_dcv::normalize_hex_str(gas_price).map_err(|_| {
        op_errors::state_unknown(
            "evm_response_invalid",
            "eth_gasPrice returned invalid hex gas price",
        )
    })
}

async fn wait_for_receipt(
    state_id: &StateId,
    io: &mut dyn IoProvider,
    tx_hash: &str,
    poll_interval_ms: u64,
    max_receipt_polls: u64,
) -> Result<serde_json::Value, StateError> {
    for poll_index in 0..max_receipt_polls {
        let request = serde_json::to_value(JsonRpcCall::new(
            "eth_getTransactionReceipt",
            serde_json::json!([tx_hash]),
        ))
        .expect("JsonRpcCall must serialize");
        let res = io
            .call(IoCall {
                namespace: "evm".to_string(),
                request,
                fact_key: Some(FactKey(format!(
                    "mfm:evm|state:{}|receipt_poll:{}|tx:{}",
                    state_id.0, poll_index, tx_hash
                ))),
            })
            .await
            .map_err(op_errors::state_from_io)?;

        if !res.response.is_null() {
            return Ok(res.response);
        }
        tokio::time::sleep(Duration::from_millis(poll_interval_ms)).await;
    }

    Err(op_errors::state_unknown(
        "evm_receipt_timeout",
        "timed out waiting for transaction receipt",
    ))
}

fn ensure_receipt_success(receipt: &serde_json::Value) -> Result<(), StateError> {
    let Some(obj) = receipt.as_object() else {
        return Err(op_errors::state_unknown(
            "evm_response_invalid",
            "transaction receipt was not a JSON object",
        ));
    };

    if let Some(status) = obj.get("status").and_then(|v| v.as_str()) {
        let s = shared_dcv::normalize_hex_str(status).map_err(|_| {
            op_errors::state_unknown("evm_response_invalid", "receipt status was not valid hex")
        })?;
        if s != "0x01" && s != "0x1" {
            return Err(op_errors::state_unknown(
                "evm_receipt_failed_status",
                "transaction receipt reported failed status",
            ));
        }
    }

    Ok(())
}

fn receipt_contract_address(receipt: &serde_json::Value) -> Result<String, StateError> {
    let Some(obj) = receipt.as_object() else {
        return Err(op_errors::state_unknown(
            "evm_response_invalid",
            "transaction receipt was not a JSON object",
        ));
    };
    let Some(addr) = obj.get("contractAddress").and_then(|v| v.as_str()) else {
        return Err(op_errors::state_unknown(
            "evm_receipt_missing_contract_address",
            "receipt did not include contractAddress",
        ));
    };

    shared_dcv::normalize_address(addr).map_err(|_| {
        op_errors::state_unknown(
            "evm_response_invalid",
            "receipt contractAddress was invalid",
        )
    })
}

fn resolve_contract_address(
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
