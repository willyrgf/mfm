use std::time::Duration;

use chrono::{DateTime, Utc};
use mfm_collectors_evm::{EvmIoClient, JsonRpcCall};
use mfm_machine::errors::{ErrorCategory, StateError};
use mfm_machine::hashing::{artifact_id_for_json, CanonicalJsonError};
use mfm_machine::ids::{FactKey, StateId};
use mfm_machine::io::{IoCall, IoProvider};
use mfm_op_common::errors as op_errors;
use mfm_op_common::rpc as op_rpc;
use mfm_transports_local_evm::{LocalEvmIoClient, LocalEvmSignLegacyCreateCall};

use crate::dcv as shared_dcv;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RpcRawTxSubmission {
    pub tx_hash: String,
    pub rpc_source_id: String,
    pub submitted_at: String,
}

pub fn normalize_quantity_hex(raw: &str, message: &'static str) -> Result<String, StateError> {
    let trimmed = raw.trim();
    let Some(rest) = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
    else {
        return Err(op_errors::state_unknown("evm_response_invalid", message));
    };

    if rest.is_empty() {
        return Ok("0x0".to_string());
    }
    if !rest.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(op_errors::state_unknown("evm_response_invalid", message));
    }

    let normalized = rest.trim_start_matches('0');
    if normalized.is_empty() {
        Ok("0x0".to_string())
    } else {
        Ok(format!("0x{}", normalized.to_ascii_lowercase()))
    }
}

pub fn parse_quantity_hex_u128(raw: &str, message: &'static str) -> Result<u128, StateError> {
    let normalized = normalize_quantity_hex(raw, message)?;
    let digits = normalized
        .strip_prefix("0x")
        .ok_or_else(|| op_errors::state_unknown("evm_response_invalid", message))?;
    u128::from_str_radix(digits, 16)
        .map_err(|_| op_errors::state_unknown("evm_response_invalid", message))
}

pub async fn send_transaction(
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
    let tx_hash = op_rpc::expect_string(
        &res.response,
        "evm_response_invalid",
        "eth_sendTransaction returned non-string tx hash",
    )?;
    shared_dcv::normalize_hex_str(&tx_hash).map_err(|_| {
        op_errors::state_unknown(
            "evm_response_invalid",
            "eth_sendTransaction returned invalid tx hash",
        )
    })
}

pub async fn send_raw_transaction(
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

    let tx_hash = op_rpc::expect_string(
        &res.response,
        "evm_response_invalid",
        "eth_sendRawTransaction returned non-string tx hash",
    )?;
    shared_dcv::normalize_hex_str(&tx_hash).map_err(|_| {
        op_errors::state_unknown(
            "evm_response_invalid",
            "eth_sendRawTransaction returned invalid tx hash",
        )
    })
}

pub async fn estimate_gas_hex(
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

    let gas = op_rpc::expect_string(
        &res.response,
        "evm_response_invalid",
        "eth_estimateGas returned non-string gas value",
    )?;
    normalize_quantity_hex(&gas, "eth_estimateGas returned invalid hex gas value")
}

pub async fn gas_price_hex(client: &mut EvmIoClient<'_>) -> Result<String, StateError> {
    let res = client
        .call(JsonRpcCall::new("eth_gasPrice", serde_json::json!([])))
        .await
        .map_err(op_errors::state_from_io)?;

    let gas_price = op_rpc::expect_string(
        &res.response,
        "evm_response_invalid",
        "eth_gasPrice returned non-string gas price",
    )?;
    normalize_quantity_hex(&gas_price, "eth_gasPrice returned invalid hex gas price")
}

pub async fn transaction_count_hex(
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

    let nonce = op_rpc::expect_string(
        &res.response,
        "evm_response_invalid",
        "eth_getTransactionCount returned non-string nonce",
    )?;
    normalize_quantity_hex(&nonce, "eth_getTransactionCount returned invalid hex nonce")
}

pub async fn pending_nonce_u128(
    io: &mut dyn IoProvider,
    state_id: &StateId,
    from: &str,
) -> Result<u128, StateError> {
    let mut client = EvmIoClient::new(state_id.clone(), io);
    let nonce_hex = transaction_count_hex(&mut client, from).await?;
    parse_quantity_hex_u128(
        &nonce_hex,
        "eth_getTransactionCount returned invalid hex nonce",
    )
}

#[derive(Clone, Debug)]
pub struct LegacyCreateTxSigningRequest<'a> {
    pub signing_key_env: &'a str,
    pub from: &'a str,
    pub chain_id: u64,
    pub nonce_hex: &'a str,
    pub gas_price_hex: &'a str,
    pub gas_limit_hex: &'a str,
    pub value_hex: &'a str,
    pub constructor_payload: &'a [u8],
}

pub async fn local_sign_legacy_create_raw_tx(
    client: &mut EvmIoClient<'_>,
    req: LegacyCreateTxSigningRequest<'_>,
) -> Result<String, StateError> {
    let state_id = client.state_id().clone();
    let mut local = LocalEvmIoClient::new(state_id, client.io_mut());
    local
        .sign_legacy_create(LocalEvmSignLegacyCreateCall {
            signing_key_env: req.signing_key_env.to_string(),
            from: req.from.to_string(),
            chain_id: req.chain_id,
            nonce_hex: req.nonce_hex.to_string(),
            gas_price_hex: req.gas_price_hex.to_string(),
            gas_limit_hex: req.gas_limit_hex.to_string(),
            value_hex: req.value_hex.to_string(),
            data_hex: shared_dcv::bytes_to_hex_prefixed(req.constructor_payload),
        })
        .await
        .map_err(op_errors::state_from_io)
}

pub async fn send_signed_create_transaction(
    client: &mut EvmIoClient<'_>,
    signing_key_env: &str,
    from: &str,
    constructor_payload: &[u8],
    value_hex: Option<&str>,
) -> Result<String, StateError> {
    let nonce_hex = transaction_count_hex(client, from).await?;
    send_signed_create_transaction_with_nonce(
        client,
        signing_key_env,
        from,
        &nonce_hex,
        constructor_payload,
        value_hex,
    )
    .await
}

pub async fn send_signed_create_transaction_with_nonce(
    client: &mut EvmIoClient<'_>,
    signing_key_env: &str,
    from: &str,
    nonce_hex: &str,
    constructor_payload: &[u8],
    value_hex: Option<&str>,
) -> Result<String, StateError> {
    let configured_from = shared_dcv::normalize_address(from).map_err(|_| {
        op_errors::state_unknown("invalid_from_address", "from address was invalid")
    })?;
    let nonce_hex = normalize_quantity_hex(
        nonce_hex,
        "signed deploy nonce must be a valid hex quantity",
    )?;

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

pub async fn wait_for_receipt(
    state_id: &StateId,
    io: &mut dyn IoProvider,
    tx_hash: &str,
    poll_interval_ms: u64,
    max_receipt_polls: u64,
) -> Result<serde_json::Value, StateError> {
    if max_receipt_polls == 0 {
        return Err(op_errors::state_unknown(
            "invalid_op_config",
            "max_receipt_polls must be > 0",
        ));
    }

    let normalized_tx = shared_dcv::normalize_hex_str(tx_hash)
        .map_err(|_| op_errors::state_unknown("invalid_tx_hash", "tx hash was invalid hex"))?;

    for poll_index in 0..max_receipt_polls {
        let request = serde_json::to_value(JsonRpcCall::new(
            "eth_getTransactionReceipt",
            serde_json::json!([normalized_tx]),
        ))
        .expect("JsonRpcCall must serialize");
        let res = io
            .call(IoCall {
                namespace: "evm".to_string(),
                request,
                fact_key: Some(FactKey(format!(
                    "mfm:evm|state:{}|receipt_poll:{}|tx:{}",
                    state_id.0, poll_index, normalized_tx
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

pub fn ensure_receipt_success(receipt: &serde_json::Value) -> Result<(), StateError> {
    let Some(obj) = receipt.as_object() else {
        return Err(op_errors::state_unknown(
            "evm_response_invalid",
            "transaction receipt was not a JSON object",
        ));
    };

    let Some(status) = obj.get("status").and_then(|v| v.as_str()) else {
        return Err(op_errors::state_unknown(
            "evm_response_invalid",
            "transaction receipt status was missing",
        ));
    };

    let normalized = normalize_quantity_hex(status, "receipt status was not valid hex")?;
    if normalized != "0x01" && normalized != "0x1" {
        return Err(op_errors::state_error(
            "evm_receipt_failed_status",
            ErrorCategory::OnChain,
            false,
            "transaction receipt reported failed status",
        ));
    }

    Ok(())
}

pub fn receipt_contract_address(receipt: &serde_json::Value) -> Result<String, StateError> {
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

pub async fn rpc_accounts(
    io: &mut dyn IoProvider,
    state_id: &StateId,
) -> Result<Vec<String>, StateError> {
    let mut client = EvmIoClient::new(state_id.clone(), io);
    let res = client
        .call(JsonRpcCall::new("eth_accounts", serde_json::json!([])))
        .await
        .map_err(op_errors::state_from_io)?;
    let arr = op_rpc::expect_array(
        &res.response,
        "evm_response_invalid",
        "eth_accounts returned non-array",
    )?;

    let mut out = Vec::with_capacity(arr.len());
    for v in arr {
        let raw = op_rpc::expect_string(
            v,
            "evm_response_invalid",
            "eth_accounts entry was not a string",
        )?;
        let normalized = shared_dcv::normalize_address(&raw).map_err(|_| {
            op_errors::state_unknown(
                "evm_response_invalid",
                "eth_accounts entry was not a valid address",
            )
        })?;
        out.push(normalized);
    }
    Ok(out)
}

pub fn account_at(accounts: &[String], idx: usize) -> Result<String, StateError> {
    accounts.get(idx).cloned().ok_or_else(|| {
        op_errors::state_error(
            "invalid_account_index",
            ErrorCategory::ParsingInput,
            false,
            "account index was out of range",
        )
    })
}

pub async fn resolve_account_by_index(
    io: &mut dyn IoProvider,
    state_id: &StateId,
    idx: usize,
) -> Result<String, StateError> {
    let accounts = rpc_accounts(io, state_id).await?;
    account_at(&accounts, idx)
}

pub async fn resolve_signing_key_address(
    io: &mut dyn IoProvider,
    state_id: &StateId,
    signing_key_env: &str,
) -> Result<String, StateError> {
    let mut local = LocalEvmIoClient::new(state_id.clone(), io);
    local
        .signer_address(signing_key_env)
        .await
        .map_err(op_errors::state_from_io)
}

pub async fn resolve_deployer_address(
    io: &mut dyn IoProvider,
    state_id: &StateId,
    deployer_account_index: usize,
    signing_key_env: Option<&str>,
) -> Result<String, StateError> {
    if let Some(env_name) = signing_key_env {
        return resolve_signing_key_address(io, state_id, env_name).await;
    }
    resolve_account_by_index(io, state_id, deployer_account_index).await
}

pub async fn send_raw_transaction_via_io(
    state_id: &StateId,
    io: &mut dyn IoProvider,
    route_source_id: &str,
    raw_tx_hex: &str,
) -> Result<RpcRawTxSubmission, StateError> {
    let source_id = route_source_id.trim();
    if source_id.is_empty() {
        return Err(op_errors::state_error_with_state(
            state_id.clone(),
            "InvalidRpcSourceId",
            ErrorCategory::ParsingInput,
            false,
            "route source id must not be empty",
        ));
    }
    validate_raw_transaction_hex(raw_tx_hex).map_err(|_| {
        op_errors::state_error_with_state(
            state_id.clone(),
            "InvalidRawTransaction",
            ErrorCategory::ParsingInput,
            false,
            "raw transaction must be 0x-prefixed valid hex",
        )
    })?;

    let request = serde_json::json!({
        "method": "eth_sendRawTransaction",
        "params": [raw_tx_hex],
        "route": {
            "source_id": source_id,
        },
    });
    let fact_key = send_raw_fact_key(state_id, raw_tx_hex)?;
    let response = io
        .call(IoCall {
            namespace: "evm".to_string(),
            request,
            fact_key: Some(fact_key),
        })
        .await
        .map_err(op_errors::state_from_io)?;

    let tx_hash = response
        .response
        .as_str()
        .ok_or_else(|| {
            op_errors::state_error_with_state(
                state_id.clone(),
                "RpcInvalidResponse",
                ErrorCategory::ParsingInput,
                false,
                "eth_sendRawTransaction returned a non-string result",
            )
        })?
        .to_string();
    validate_tx_hash(&tx_hash).map_err(|_| {
        op_errors::state_error_with_state(
            state_id.clone(),
            "RpcInvalidResponse",
            ErrorCategory::ParsingInput,
            false,
            "eth_sendRawTransaction returned an invalid tx hash",
        )
    })?;

    let submitted_at = now_rfc3339(io, state_id).await?;

    Ok(RpcRawTxSubmission {
        tx_hash,
        rpc_source_id: source_id.to_string(),
        submitted_at,
    })
}

fn validate_raw_transaction_hex(raw_tx_hex: &str) -> Result<(), ()> {
    let value = raw_tx_hex.trim();
    if !value.starts_with("0x") {
        return Err(());
    }
    if value.len() <= 2 || !value.len().is_multiple_of(2) {
        return Err(());
    }
    if !value[2..].chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(());
    }
    Ok(())
}

fn validate_tx_hash(tx_hash: &str) -> Result<(), ()> {
    if tx_hash.len() != 66 || !tx_hash.starts_with("0x") {
        return Err(());
    }
    if !tx_hash[2..].chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(());
    }
    Ok(())
}

async fn now_rfc3339(io: &mut dyn IoProvider, state_id: &StateId) -> Result<String, StateError> {
    let now_ms = io.now_millis().await.map_err(op_errors::state_from_io)?;
    let timestamp = DateTime::<Utc>::from_timestamp_millis(now_ms as i64).ok_or_else(|| {
        op_errors::state_error_with_state(
            state_id.clone(),
            "InvalidTimestamp",
            ErrorCategory::Unknown,
            false,
            "failed to convert timestamp to RFC3339",
        )
    })?;
    Ok(timestamp.to_rfc3339())
}

fn send_raw_fact_key(state_id: &StateId, raw_tx_hex: &str) -> Result<FactKey, StateError> {
    let key_request = serde_json::json!({
        "method": "eth_sendRawTransaction",
        "params": [raw_tx_hex],
    });
    let req_id = artifact_id_for_json(&key_request).map_err(|err| match err {
        CanonicalJsonError::FloatNotAllowed => op_errors::state_error_with_state(
            state_id.clone(),
            "evm_request_not_canonical",
            ErrorCategory::ParsingInput,
            false,
            "raw tx request was not canonical-json-hashable (floats are forbidden)",
        ),
        CanonicalJsonError::SecretsNotAllowed => op_errors::state_error_with_state(
            state_id.clone(),
            "secrets_detected",
            ErrorCategory::Unknown,
            false,
            "raw tx request contained secrets (policy forbids persisting secrets)",
        ),
    })?;

    Ok(FactKey(format!(
        "mfm:evm_send_raw|state:{}|req:{}",
        state_id.0, req_id.0
    )))
}
