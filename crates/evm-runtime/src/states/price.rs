//! Reusable EVM oracle and valuation-source helpers.
//!
//! This module keeps concrete oracle decoding out of higher-level portfolio states so symbol and
//! portfolio crates can request routed, pinned unit-price reads without reimplementing JSON-RPC
//! plumbing or ABI decoding details.

use std::collections::BTreeMap;

use alloy_primitives::U256;
use mfm_collectors_rpc_control::{EvmIoClient, JsonRpcCall};
use mfm_evm_core::abi::function_selector;
use mfm_evm_core::encoding::{
    parse_hex_string_response, parse_u256_hex_value, parse_u8_u256, u64_hex_quantity,
};
use mfm_evm_core::hex::{bytes_to_hex_prefixed, hex_to_bytes};
use mfm_machine::errors::{ErrorCategory, StateError};
use mfm_machine::ids::StateId;
use mfm_machine::io::IoProvider;
use mfm_state_common::errors::{state_error_with_state, state_from_io, state_unknown_msg};
use serde_json::Value;

use mfm_evm_dcv_model::normalize_address;

/// Runtime output for one successfully resolved direct price source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OracleUnitPrice {
    /// Unit price formatted as a decimal string.
    pub unit_price_dec: String,
}

/// Reads a unit price through a routed and block-pinned EVM oracle source.
pub async fn read_evm_oracle_unit_price(
    state_id: &StateId,
    io: &mut dyn IoProvider,
    network_id: &str,
    control_scope: &str,
    oracle_kind: &str,
    config: &BTreeMap<String, Value>,
    block_number: u64,
) -> Result<OracleUnitPrice, StateError> {
    match oracle_kind {
        "chainlink_aggregator_v3" => {
            read_chainlink_aggregator_v3_unit_price(
                state_id,
                io,
                network_id,
                control_scope,
                config,
                block_number,
            )
            .await
        }
        _ => Err(state_error_with_state(
            state_id.clone(),
            "unsupported_valuation_source_kind",
            ErrorCategory::Unknown,
            false,
            format!("unsupported valuation source kind `{oracle_kind}`"),
        )),
    }
}

async fn read_chainlink_aggregator_v3_unit_price(
    state_id: &StateId,
    io: &mut dyn IoProvider,
    network_id: &str,
    control_scope: &str,
    config: &BTreeMap<String, Value>,
    block_number: u64,
) -> Result<OracleUnitPrice, StateError> {
    let contract_address = chainlink_contract_address(config)?;
    let block = serde_json::json!(u64_hex_quantity(block_number));

    let decimals_raw = eth_call_with_route(
        state_id,
        io,
        network_id,
        control_scope,
        &contract_address,
        &crate::states::read::encode_erc20_decimals(),
        block.clone(),
    )
    .await?;
    let decimals_u256 = parse_u256_hex_value(&decimals_raw).map_err(|err| {
        state_unknown_msg(
            "evm_response_invalid",
            format!("oracle decimals response was invalid: {}", err.message),
        )
    })?;
    let decimals = parse_u8_u256(decimals_u256).map_err(|err| {
        state_unknown_msg(
            "evm_response_invalid",
            format!("oracle decimals response was invalid: {}", err.message),
        )
    })?;

    let round_data_raw = eth_call_with_route(
        state_id,
        io,
        network_id,
        control_scope,
        &contract_address,
        &encode_latest_round_data(),
        block,
    )
    .await?;
    let answer = decode_chainlink_latest_round_data_answer(&round_data_raw)?;

    Ok(OracleUnitPrice {
        unit_price_dec: mfm_evm_core::encoding::format_u256_units(&answer, decimals),
    })
}

fn chainlink_contract_address(config: &BTreeMap<String, Value>) -> Result<String, StateError> {
    let Some(raw) = config.get("contract_address").and_then(Value::as_str) else {
        return Err(state_unknown_msg(
            "valuation_source_config_invalid",
            "chainlink_aggregator_v3 config must contain contract_address",
        ));
    };

    normalize_address(raw).map_err(|err| {
        state_unknown_msg(
            "valuation_source_config_invalid",
            format!("chainlink_aggregator_v3 contract_address was invalid: {err}"),
        )
    })
}

async fn eth_call_with_route(
    state_id: &StateId,
    io: &mut dyn IoProvider,
    network_id: &str,
    control_scope: &str,
    to: &str,
    data: &str,
    block: Value,
) -> Result<Value, StateError> {
    let mut client = EvmIoClient::new(state_id.clone(), io);
    let call = JsonRpcCall::for_scope_and_network(
        control_scope,
        network_id,
        "eth_call",
        serde_json::json!([
            {"to": to, "data": data},
            block
        ]),
    );
    let res = client.call(call).await.map_err(state_from_io)?;
    Ok(res.response)
}

fn encode_latest_round_data() -> String {
    bytes_to_hex_prefixed(&function_selector("latestRoundData", &[]))
}

fn decode_chainlink_latest_round_data_answer(raw: &Value) -> Result<U256, StateError> {
    let hex = parse_hex_string_response(raw).map_err(|err| {
        state_unknown_msg(
            "evm_response_invalid",
            format!(
                "oracle latestRoundData response was invalid: {}",
                err.message
            ),
        )
    })?;
    let bytes = hex_to_bytes(&hex).map_err(|err| {
        state_unknown_msg(
            "evm_response_invalid",
            format!(
                "oracle latestRoundData response was invalid: {}",
                err.message
            ),
        )
    })?;
    if bytes.len() < 64 || bytes.len() % 32 != 0 {
        return Err(state_unknown_msg(
            "evm_response_invalid",
            "oracle latestRoundData response was invalid",
        ));
    }

    let answer = &bytes[32..64];
    if answer[0] & 0x80 != 0 {
        return Err(state_unknown_msg(
            "evm_response_invalid",
            "oracle latestRoundData answer must be non-negative",
        ));
    }

    Ok(U256::from_be_slice(answer))
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use mfm_machine::errors::{ErrorInfo, IoError};
    use mfm_machine::ids::{ArtifactId, ErrorCode, FactKey};
    use mfm_machine::io::{IoCall, IoResult};

    fn info(code: &'static str, message: &'static str) -> ErrorInfo {
        ErrorInfo {
            code: ErrorCode::must_new(code),
            category: ErrorCategory::Unknown,
            retryable: false,
            message: message.to_string(),
            details: None,
        }
    }

    #[derive(Default)]
    struct FixedIo {
        calls: Vec<serde_json::Value>,
    }

    #[async_trait]
    impl IoProvider for FixedIo {
        async fn call(&mut self, call: IoCall) -> Result<IoResult, IoError> {
            self.calls.push(call.request.clone());

            let params = call
                .request
                .get("params")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let target = params
                .first()
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            let to = target.get("to").and_then(Value::as_str).unwrap_or_default();
            let data = target
                .get("data")
                .and_then(Value::as_str)
                .unwrap_or_default();

            let response = match (to, data) {
                ("0x0000000000000000000000000000000000000001", d)
                    if d == crate::states::read::encode_erc20_decimals() =>
                {
                    serde_json::json!(
                        "0x0000000000000000000000000000000000000000000000000000000000000008"
                    )
                }
                ("0x0000000000000000000000000000000000000001", "0xfeaf968c") => serde_json::json!(
                    "0x\
0000000000000000000000000000000000000000000000000000000000000001\
000000000000000000000000000000000000000000000000000000000bebc200\
0000000000000000000000000000000000000000000000000000000000000001\
0000000000000000000000000000000000000000000000000000000000000001\
0000000000000000000000000000000000000000000000000000000000000001"
                ),
                _ => {
                    return Err(IoError::Other(info(
                        "unexpected_call",
                        "unexpected test call",
                    )))
                }
            };

            Ok(IoResult {
                response,
                recorded_payload_id: None,
            })
        }

        async fn record_value(
            &mut self,
            _key: FactKey,
            _value: Value,
        ) -> Result<ArtifactId, IoError> {
            Ok(ArtifactId::must_new("0".repeat(64)))
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

    #[tokio::test]
    async fn reads_chainlink_unit_price_with_network_and_pin() {
        let mut io = FixedIo::default();
        let state_id = StateId::must_new("price.main.read".to_string());
        let config = BTreeMap::from([(
            "contract_address".to_string(),
            Value::String("0x0000000000000000000000000000000000000001".to_string()),
        )]);

        let price = read_evm_oracle_unit_price(
            &state_id,
            &mut io,
            "ethereum-mainnet",
            "shared",
            "chainlink_aggregator_v3",
            &config,
            123,
        )
        .await
        .expect("price");

        assert_eq!(price.unit_price_dec, "2.00000000");
        assert_eq!(io.calls.len(), 2);
        assert_eq!(
            io.calls[0].get("network_id").and_then(Value::as_str),
            Some("ethereum-mainnet")
        );
        assert_eq!(
            io.calls[0]
                .get("params")
                .and_then(Value::as_array)
                .and_then(|params| params.get(1))
                .and_then(Value::as_str),
            Some("0x7b")
        );
    }

    #[tokio::test]
    async fn unsupported_valuation_source_kind_is_structured() {
        let mut io = FixedIo::default();
        let state_id = StateId::must_new("price.main.unsupported".to_string());
        let err = read_evm_oracle_unit_price(
            &state_id,
            &mut io,
            "ethereum-mainnet",
            "shared",
            "unknown_oracle",
            &BTreeMap::new(),
            1,
        )
        .await
        .expect_err("expected unsupported kind");

        assert_eq!(err.info.code.as_str(), "unsupported_valuation_source_kind");
    }
}
