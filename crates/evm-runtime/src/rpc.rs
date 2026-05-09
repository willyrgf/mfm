use mfm_collectors_rpc_control::{EvmIoClient, JsonRpcCall, DEFAULT_CONTROL_SCOPE};
use mfm_machine::errors::{ErrorCategory, StateError};
use mfm_machine::ids::{FactKey, StateId};
use mfm_machine::io::IoProvider;
use mfm_state_common::errors as op_errors;
use mfm_state_common::rpc as op_rpc;
use mfm_transports_local_evm::{
    LocalEvmIoClient, LocalEvmSignLegacyCallCall, LocalEvmSignLegacyCreateCall,
};

use crate::dcv as shared_dcv;

/// Normalizes an RPC hex quantity into canonical lowercase `0x` form.
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

/// Parses an RPC hex quantity into `u128`.
pub fn parse_quantity_hex_u128(raw: &str, message: &'static str) -> Result<u128, StateError> {
    let normalized = normalize_quantity_hex(raw, message)?;
    let digits = normalized
        .strip_prefix("0x")
        .ok_or_else(|| op_errors::state_unknown("evm_response_invalid", message))?;
    u128::from_str_radix(digits, 16)
        .map_err(|_| op_errors::state_unknown("evm_response_invalid", message))
}

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

fn receipt_poll_fact_key(
    state_id: &StateId,
    control_scope: &str,
    network_id: &str,
    poll_index: u64,
    normalized_tx: &str,
) -> FactKey {
    FactKey(format!(
        "mfm:rpc.control|state:{}|scope:{}|network:{}|receipt_poll:{}|tx:{}",
        state_id.as_str(),
        control_scope,
        network_id,
        poll_index,
        normalized_tx
    ))
}

fn legacy_network_required_error(helper_name: &'static str) -> StateError {
    op_errors::state_unknown_msg(
        "rpc_control_network_required",
        format!(
            "{helper_name} requires an explicit network_id; use the *_for_network helper instead"
        ),
    )
}

/// Submits a transaction through `eth_sendTransaction`, filling gas and gas price when absent.
pub async fn send_transaction(
    client: &mut EvmIoClient<'_>,
    _tx_obj: serde_json::Value,
) -> Result<String, StateError> {
    let _ = client;
    Err(legacy_network_required_error("send_transaction"))
}

/// Submits a transaction through `eth_sendTransaction` for the supplied managed network and scope.
pub async fn send_transaction_for_network(
    client: &mut EvmIoClient<'_>,
    network_id: &str,
    control_scope: &str,
    mut tx_obj: serde_json::Value,
) -> Result<String, StateError> {
    if tx_obj.get("gas").is_none() {
        let gas = estimate_gas_hex_for_network(client, network_id, control_scope, &tx_obj).await?;
        tx_obj["gas"] = serde_json::json!(gas);
    }
    if tx_obj.get("gasPrice").is_none() && tx_obj.get("maxFeePerGas").is_none() {
        let gas_price = gas_price_hex_for_network(client, network_id, control_scope).await?;
        tx_obj["gasPrice"] = serde_json::json!(gas_price);
    }

    let res = client
        .call(managed_call(
            network_id,
            control_scope,
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

/// Submits a raw signed transaction through `eth_sendRawTransaction`.
pub async fn send_raw_transaction(
    client: &mut EvmIoClient<'_>,
    _raw_tx_hex: &str,
) -> Result<String, StateError> {
    let _ = client;
    Err(legacy_network_required_error("send_raw_transaction"))
}

/// Submits a raw signed transaction through `eth_sendRawTransaction` for the supplied managed network and scope.
pub async fn send_raw_transaction_for_network(
    client: &mut EvmIoClient<'_>,
    network_id: &str,
    control_scope: &str,
    raw_tx_hex: &str,
) -> Result<String, StateError> {
    let res = client
        .call(managed_call(
            network_id,
            control_scope,
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

/// Estimates gas for the supplied transaction object and returns a canonical hex quantity.
pub async fn estimate_gas_hex(
    client: &mut EvmIoClient<'_>,
    _tx_obj: &serde_json::Value,
) -> Result<String, StateError> {
    let _ = client;
    Err(legacy_network_required_error("estimate_gas_hex"))
}

/// Estimates gas for the supplied transaction object within the supplied managed network and scope.
pub async fn estimate_gas_hex_for_network(
    client: &mut EvmIoClient<'_>,
    network_id: &str,
    control_scope: &str,
    tx_obj: &serde_json::Value,
) -> Result<String, StateError> {
    let res = client
        .call(managed_call(
            network_id,
            control_scope,
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

/// Fetches the current gas price as a canonical hex quantity.
pub async fn gas_price_hex(client: &mut EvmIoClient<'_>) -> Result<String, StateError> {
    let _ = client;
    Err(legacy_network_required_error("gas_price_hex"))
}

/// Fetches the current gas price for the supplied managed network and scope.
pub async fn gas_price_hex_for_network(
    client: &mut EvmIoClient<'_>,
    network_id: &str,
    control_scope: &str,
) -> Result<String, StateError> {
    let res = client
        .call(managed_call(
            network_id,
            control_scope,
            "eth_gasPrice",
            serde_json::json!([]),
        ))
        .await
        .map_err(op_errors::state_from_io)?;

    let gas_price = op_rpc::expect_string(
        &res.response,
        "evm_response_invalid",
        "eth_gasPrice returned non-string gas price",
    )?;
    normalize_quantity_hex(&gas_price, "eth_gasPrice returned invalid hex gas price")
}

/// Fetches the pending transaction count for `from` as a canonical hex quantity.
pub async fn transaction_count_hex(
    client: &mut EvmIoClient<'_>,
    _from: &str,
) -> Result<String, StateError> {
    let _ = client;
    Err(legacy_network_required_error("transaction_count_hex"))
}

/// Fetches the pending transaction count for `from` within the supplied managed network and scope.
pub async fn transaction_count_hex_for_network(
    client: &mut EvmIoClient<'_>,
    network_id: &str,
    control_scope: &str,
    from: &str,
) -> Result<String, StateError> {
    let res = client
        .call(managed_call(
            network_id,
            control_scope,
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

/// Resolves the pending nonce for `from` as a `u128`.
pub async fn pending_nonce_u128(
    io: &mut dyn IoProvider,
    state_id: &StateId,
    _from: &str,
) -> Result<u128, StateError> {
    let _ = (io, state_id);
    Err(legacy_network_required_error("pending_nonce_u128"))
}

/// Resolves the pending nonce for `from` within the supplied managed network and scope.
pub async fn pending_nonce_u128_for_network(
    io: &mut dyn IoProvider,
    state_id: &StateId,
    network_id: &str,
    control_scope: &str,
    from: &str,
) -> Result<u128, StateError> {
    let mut client = EvmIoClient::new(state_id.clone(), io);
    let nonce_hex =
        transaction_count_hex_for_network(&mut client, network_id, control_scope, from).await?;
    parse_quantity_hex_u128(
        &nonce_hex,
        "eth_getTransactionCount returned invalid hex nonce",
    )
}

/// Inputs required to locally sign a legacy contract-creation transaction.
#[derive(Clone, Debug)]
pub struct LegacyCreateTxSigningRequest<'a> {
    /// Environment variable name that contains the signing key.
    pub signing_key_env: &'a str,
    /// Sender address.
    pub from: &'a str,
    /// Chain id used for signing.
    pub chain_id: u64,
    /// Nonce expressed as a canonical hex quantity.
    pub nonce_hex: &'a str,
    /// Gas price expressed as a canonical hex quantity.
    pub gas_price_hex: &'a str,
    /// Gas limit expressed as a canonical hex quantity.
    pub gas_limit_hex: &'a str,
    /// Value expressed as a canonical hex quantity.
    pub value_hex: &'a str,
    /// Constructor payload bytes.
    pub constructor_payload: &'a [u8],
}

/// Inputs required to locally sign a legacy contract call transaction.
#[derive(Clone, Debug)]
pub struct LegacyCallTxSigningRequest<'a> {
    /// Environment variable name that contains the signing key.
    pub signing_key_env: &'a str,
    /// Sender address.
    pub from: &'a str,
    /// Target address.
    pub to: &'a str,
    /// Chain id used for signing.
    pub chain_id: u64,
    /// Nonce expressed as a canonical hex quantity.
    pub nonce_hex: &'a str,
    /// Gas price expressed as a canonical hex quantity.
    pub gas_price_hex: &'a str,
    /// Gas limit expressed as a canonical hex quantity.
    pub gas_limit_hex: &'a str,
    /// Value expressed as a canonical hex quantity.
    pub value_hex: &'a str,
    /// Call payload bytes.
    pub call_payload: &'a [u8],
}

/// Asks the local EVM transport to sign a legacy contract-creation transaction.
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

/// Asks the local EVM transport to sign a legacy contract call transaction.
pub async fn local_sign_legacy_call_raw_tx(
    client: &mut EvmIoClient<'_>,
    req: LegacyCallTxSigningRequest<'_>,
) -> Result<String, StateError> {
    let state_id = client.state_id().clone();
    let mut local = LocalEvmIoClient::new(state_id, client.io_mut());
    local
        .sign_legacy_call(LocalEvmSignLegacyCallCall {
            signing_key_env: req.signing_key_env.to_string(),
            from: req.from.to_string(),
            to: req.to.to_string(),
            chain_id: req.chain_id,
            nonce_hex: req.nonce_hex.to_string(),
            gas_price_hex: req.gas_price_hex.to_string(),
            gas_limit_hex: req.gas_limit_hex.to_string(),
            value_hex: req.value_hex.to_string(),
            data_hex: shared_dcv::bytes_to_hex_prefixed(req.call_payload),
        })
        .await
        .map_err(op_errors::state_from_io)
}

/// Signs and submits a contract-creation transaction using the next pending nonce.
pub async fn send_signed_create_transaction(
    client: &mut EvmIoClient<'_>,
    _signing_key_env: &str,
    _from: &str,
    _constructor_payload: &[u8],
    _value_hex: Option<&str>,
) -> Result<String, StateError> {
    let _ = client;
    Err(legacy_network_required_error(
        "send_signed_create_transaction",
    ))
}

/// Signs and submits a contract-creation transaction for the supplied managed network and scope using the next pending nonce.
pub async fn send_signed_create_transaction_for_network(
    client: &mut EvmIoClient<'_>,
    network_id: &str,
    control_scope: &str,
    signing_key_env: &str,
    from: &str,
    constructor_payload: &[u8],
    value_hex: Option<&str>,
) -> Result<String, StateError> {
    let nonce_hex =
        transaction_count_hex_for_network(client, network_id, control_scope, from).await?;
    send_signed_create_transaction_with_nonce_for_network(
        client,
        network_id,
        control_scope,
        signing_key_env,
        from,
        &nonce_hex,
        constructor_payload,
        value_hex,
    )
    .await
}

/// Signs and submits a contract-creation transaction using the supplied nonce.
pub async fn send_signed_create_transaction_with_nonce(
    client: &mut EvmIoClient<'_>,
    _signing_key_env: &str,
    _from: &str,
    _nonce_hex: &str,
    _constructor_payload: &[u8],
    _value_hex: Option<&str>,
) -> Result<String, StateError> {
    let _ = client;
    Err(legacy_network_required_error(
        "send_signed_create_transaction_with_nonce",
    ))
}

/// Signs and submits a contract-creation transaction for the supplied managed network, scope, and nonce.
#[allow(clippy::too_many_arguments)]
pub async fn send_signed_create_transaction_with_nonce_for_network(
    client: &mut EvmIoClient<'_>,
    network_id: &str,
    control_scope: &str,
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

    let gas_hex = estimate_gas_hex_for_network(client, network_id, control_scope, &tx_obj).await?;
    let gas_price_hex = gas_price_hex_for_network(client, network_id, control_scope).await?;
    let chain_id = client
        .call(managed_call(
            network_id,
            control_scope,
            "eth_chainId",
            serde_json::json!([]),
        ))
        .await
        .map_err(op_errors::state_from_io)?;
    let chain_id = op_rpc::expect_string(
        &chain_id.response,
        "evm_response_invalid",
        "eth_chainId returned non-string chain id",
    )?;
    let chain_id = normalize_quantity_hex(&chain_id, "eth_chainId returned invalid chain id")?;
    let chain_id = u64::from_str_radix(
        chain_id
            .strip_prefix("0x")
            .expect("normalized quantity must have prefix"),
        16,
    )
    .map_err(|_| op_errors::state_unknown("evm_response_invalid", "eth_chainId overflowed u64"))?;

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

    send_raw_transaction_for_network(client, network_id, control_scope, &raw_tx_hex).await
}

/// Signs and submits a contract call transaction for the supplied managed network and scope using the next pending nonce.
#[allow(clippy::too_many_arguments)]
pub async fn send_signed_call_transaction_for_network(
    client: &mut EvmIoClient<'_>,
    network_id: &str,
    control_scope: &str,
    signing_key_env: &str,
    from: &str,
    to: &str,
    call_payload: &[u8],
    value_hex: Option<&str>,
) -> Result<String, StateError> {
    let nonce_hex =
        transaction_count_hex_for_network(client, network_id, control_scope, from).await?;
    send_signed_call_transaction_with_nonce_for_network(
        client,
        network_id,
        control_scope,
        signing_key_env,
        from,
        to,
        &nonce_hex,
        call_payload,
        value_hex,
    )
    .await
}

/// Signs and submits a contract call transaction for the supplied managed network, scope, and nonce.
#[allow(clippy::too_many_arguments)]
pub async fn send_signed_call_transaction_with_nonce_for_network(
    client: &mut EvmIoClient<'_>,
    network_id: &str,
    control_scope: &str,
    signing_key_env: &str,
    from: &str,
    to: &str,
    nonce_hex: &str,
    call_payload: &[u8],
    value_hex: Option<&str>,
) -> Result<String, StateError> {
    let configured_from = shared_dcv::normalize_address(from).map_err(|_| {
        op_errors::state_unknown("invalid_from_address", "from address was invalid")
    })?;
    let configured_to = shared_dcv::normalize_address(to)
        .map_err(|_| op_errors::state_unknown("invalid_to_address", "to address was invalid"))?;
    let nonce_hex =
        normalize_quantity_hex(nonce_hex, "signed call nonce must be a valid hex quantity")?;

    let tx_obj = {
        let mut tx = serde_json::json!({
            "from": configured_from,
            "to": configured_to,
            "data": shared_dcv::bytes_to_hex_prefixed(call_payload),
        });
        if let Some(v) = value_hex {
            tx["value"] = serde_json::json!(v);
        }
        tx
    };

    let gas_hex = estimate_gas_hex_for_network(client, network_id, control_scope, &tx_obj).await?;
    let gas_price_hex = gas_price_hex_for_network(client, network_id, control_scope).await?;
    let chain_id = client
        .call(managed_call(
            network_id,
            control_scope,
            "eth_chainId",
            serde_json::json!([]),
        ))
        .await
        .map_err(op_errors::state_from_io)?;
    let chain_id = op_rpc::expect_string(
        &chain_id.response,
        "evm_response_invalid",
        "eth_chainId returned non-string chain id",
    )?;
    let chain_id = normalize_quantity_hex(&chain_id, "eth_chainId returned invalid chain id")?;
    let chain_id = u64::from_str_radix(
        chain_id
            .strip_prefix("0x")
            .expect("normalized quantity must have prefix"),
        16,
    )
    .map_err(|_| op_errors::state_unknown("evm_response_invalid", "eth_chainId overflowed u64"))?;

    let raw_tx_hex = local_sign_legacy_call_raw_tx(
        client,
        LegacyCallTxSigningRequest {
            signing_key_env,
            from: &configured_from,
            to: &configured_to,
            chain_id,
            nonce_hex: &nonce_hex,
            gas_price_hex: &gas_price_hex,
            gas_limit_hex: &gas_hex,
            value_hex: value_hex.unwrap_or("0x0"),
            call_payload,
        },
    )
    .await?;

    send_raw_transaction_for_network(client, network_id, control_scope, &raw_tx_hex).await
}

/// Polls until a transaction receipt is available or the poll budget is exhausted.
pub async fn wait_for_receipt(
    state_id: &StateId,
    io: &mut dyn IoProvider,
    _tx_hash: &str,
    _poll_interval_ms: u64,
    _max_receipt_polls: u64,
) -> Result<serde_json::Value, StateError> {
    let _ = (state_id, io);
    Err(legacy_network_required_error("wait_for_receipt"))
}

/// Polls until a transaction receipt is available within the supplied managed network and scope.
pub async fn wait_for_receipt_for_network(
    state_id: &StateId,
    io: &mut dyn IoProvider,
    network_id: &str,
    control_scope: &str,
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
    let effective_scope = if control_scope.trim().is_empty() {
        DEFAULT_CONTROL_SCOPE
    } else {
        control_scope
    };

    for poll_index in 0..max_receipt_polls {
        let res = {
            let mut client = EvmIoClient::new(state_id.clone(), io)
                .with_default_control_scope(effective_scope.to_string());
            client
                .call_with_fact_key(
                    managed_call(
                        network_id,
                        effective_scope,
                        "eth_getTransactionReceipt",
                        serde_json::json!([normalized_tx.clone()]),
                    ),
                    receipt_poll_fact_key(
                        state_id,
                        effective_scope,
                        network_id,
                        poll_index,
                        &normalized_tx,
                    ),
                )
                .await
                .map_err(op_errors::state_from_io)?
        };
        if !res.response.is_null() {
            return Ok(res.response);
        }
        io.sleep_ms(poll_interval_ms)
            .await
            .map_err(op_errors::state_from_io)?;
    }

    Err(op_errors::state_unknown(
        "evm_receipt_timeout",
        "timed out waiting for transaction receipt",
    ))
}

/// Ensures a transaction receipt reports success.
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

/// Extracts and normalizes the deployed contract address from a transaction receipt.
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

/// Fetches and normalizes the account list returned by `eth_accounts`.
pub async fn rpc_accounts(
    io: &mut dyn IoProvider,
    state_id: &StateId,
) -> Result<Vec<String>, StateError> {
    let _ = (io, state_id);
    Err(legacy_network_required_error("rpc_accounts"))
}

/// Fetches and normalizes the account list returned by `eth_accounts` for the supplied managed
/// network and scope.
pub async fn rpc_accounts_for_network(
    io: &mut dyn IoProvider,
    state_id: &StateId,
    network_id: &str,
    control_scope: &str,
) -> Result<Vec<String>, StateError> {
    let mut client = EvmIoClient::new(state_id.clone(), io);
    let res = client
        .call(managed_call(
            network_id,
            control_scope,
            "eth_accounts",
            serde_json::json!([]),
        ))
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

/// Returns the account at `idx` or a structured range error.
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

/// Resolves an account by index through `eth_accounts`.
pub async fn resolve_account_by_index(
    io: &mut dyn IoProvider,
    state_id: &StateId,
    _idx: usize,
) -> Result<String, StateError> {
    let _ = (io, state_id);
    Err(legacy_network_required_error("resolve_account_by_index"))
}

/// Resolves an account by index through `eth_accounts` for the supplied managed network and scope.
pub async fn resolve_account_by_index_for_network(
    io: &mut dyn IoProvider,
    state_id: &StateId,
    network_id: &str,
    control_scope: &str,
    idx: usize,
) -> Result<String, StateError> {
    let accounts = rpc_accounts_for_network(io, state_id, network_id, control_scope).await?;
    account_at(&accounts, idx)
}

/// Resolves the address for a locally configured signing key.
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

/// Resolves the deployer address from either a signing key or an account index.
pub async fn resolve_deployer_address(
    io: &mut dyn IoProvider,
    state_id: &StateId,
    _deployer_account_index: usize,
    signing_key_env: Option<&str>,
) -> Result<String, StateError> {
    if let Some(env_name) = signing_key_env {
        return resolve_signing_key_address(io, state_id, env_name).await;
    }
    Err(legacy_network_required_error("resolve_deployer_address"))
}

/// Resolves the deployer address from either a signing key or an account index for the supplied
/// managed network and scope.
pub async fn resolve_deployer_address_for_network(
    io: &mut dyn IoProvider,
    state_id: &StateId,
    network_id: &str,
    control_scope: &str,
    deployer_account_index: usize,
    signing_key_env: Option<&str>,
) -> Result<String, StateError> {
    if let Some(env_name) = signing_key_env {
        return resolve_signing_key_address(io, state_id, env_name).await;
    }
    resolve_account_by_index_for_network(
        io,
        state_id,
        network_id,
        control_scope,
        deployer_account_index,
    )
    .await
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;
    use serde_json::Value;

    use mfm_machine::errors::IoError;
    use mfm_machine::ids::ArtifactId;
    use mfm_machine::io::{IoCall, IoProvider, IoResult};

    use super::*;

    #[derive(Default)]
    struct FixedIo {
        calls: Vec<IoCall>,
        responses: Vec<Value>,
        slept_ms: Vec<u64>,
    }

    #[async_trait]
    impl IoProvider for FixedIo {
        async fn call(&mut self, call: IoCall) -> Result<IoResult, IoError> {
            self.calls.push(call);
            let response = if self.responses.is_empty() {
                Value::Null
            } else {
                self.responses.remove(0)
            };
            Ok(IoResult {
                response,
                recorded_payload_id: Some(ArtifactId::must_new("1".repeat(64))),
            })
        }

        async fn record_value(
            &mut self,
            _key: FactKey,
            _value: Value,
        ) -> Result<ArtifactId, IoError> {
            Ok(ArtifactId::must_new("2".repeat(64)))
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
            Ok(vec![0_u8; n])
        }

        async fn sleep_ms(&mut self, duration_ms: u64) -> Result<(), IoError> {
            self.slept_ms.push(duration_ms);
            Ok(())
        }
    }

    #[test]
    fn receipt_poll_fact_key_differs_across_scope_and_network() {
        let state_id = StateId::must_new("rpc.receipt.scope".to_string());
        let tx_hash = "0x1234";

        let shared = receipt_poll_fact_key(&state_id, "shared", "ethereum-mainnet", 0, tx_hash);
        let isolated = receipt_poll_fact_key(&state_id, "isolated", "ethereum-mainnet", 0, tx_hash);
        let arbitrum = receipt_poll_fact_key(&state_id, "shared", "arbitrum-mainnet", 0, tx_hash);

        assert_ne!(shared, isolated);
        assert_ne!(shared, arbitrum);
        assert_ne!(isolated, arbitrum);
    }

    #[tokio::test]
    async fn wait_for_receipt_for_network_stamps_scope_into_request_and_fact_key() {
        let state_id = StateId::must_new("rpc.receipt.wait".to_string());
        let mut io = FixedIo {
            calls: Vec::new(),
            responses: vec![Value::Null, serde_json::json!({ "status": "0x1" })],
            slept_ms: Vec::new(),
        };

        let receipt = wait_for_receipt_for_network(
            &state_id,
            &mut io,
            "ethereum-mainnet",
            "",
            "0x1234",
            25,
            2,
        )
        .await
        .expect("receipt should resolve");

        assert_eq!(receipt, serde_json::json!({ "status": "0x1" }));
        assert_eq!(io.calls.len(), 2);
        assert_eq!(
            io.calls[0].request,
            serde_json::json!({
                "kind": "evm_call",
                "control_scope": "shared",
                "network_id": "ethereum-mainnet",
                "method": "eth_getTransactionReceipt",
                "params": ["0x1234"],
            })
        );
        assert_eq!(
            io.calls[0].fact_key,
            Some(receipt_poll_fact_key(
                &state_id,
                "shared",
                "ethereum-mainnet",
                0,
                "0x1234",
            ))
        );
        assert_eq!(
            io.calls[1].fact_key,
            Some(receipt_poll_fact_key(
                &state_id,
                "shared",
                "ethereum-mainnet",
                1,
                "0x1234",
            ))
        );
        assert_eq!(io.slept_ms, vec![25]);
    }
}
