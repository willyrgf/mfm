use std::collections::{BTreeMap, BTreeSet};

use alloy_primitives::{Address, U256};
use async_trait::async_trait;
use mfm_collectors_rpc_control::{EvmIoClient, JsonRpcCall};
use mfm_evm_core::abi::function_selector;
use mfm_evm_core::encoding::{
    encode_erc20_balance_of, format_u256_units, parse_u256_hex_value, parse_u8_u256,
    u64_hex_quantity,
};
use mfm_machine::errors::StateError;
use mfm_machine::ids::StateId;
use mfm_machine::io::IoProvider;
use mfm_state_common::errors::{state_from_io, state_unknown, state_unknown_msg};
use mfm_state_portfolio::semantic::{
    AdapterId, CompiledObservationBinding, EvmResolvedSubjectValue, ExecutionAnchor, Observation,
    ObservationRuntimeAdapter, ObservationRuntimeInput, PinnedNetworkView, ResolvedSubject,
    RuntimeAdapter,
};
use mfm_state_symbol::model::{ObservationQuantity, ObservationSource, ObservationValue};
use num_bigint::BigInt;
use num_traits::Signed;
use serde::Deserialize;
use serde_json::Value;

use crate::portfolio::model::{AaveDebtKind, AAVE_V3_PROTOCOL_ID};
use crate::portfolio::semantic::{AaveDebtObservationPayload, AaveReserveObservationPayload};

const ADAPTER_OBSERVE_AAVE_RESERVE: &str = "observe_position/aave_v3/reserve_position";
const ADAPTER_OBSERVE_AAVE_DEBT: &str = "observe_position/aave_v3/debt_position";

/// Runtime adapter for compiled Aave reserve-position observations.
#[derive(Clone, Debug, Default)]
pub struct AaveReserveObservationRuntimeAdapter;

impl RuntimeAdapter for AaveReserveObservationRuntimeAdapter {
    fn id(&self) -> &AdapterId {
        static ID: std::sync::OnceLock<AdapterId> = std::sync::OnceLock::new();
        ID.get_or_init(|| AdapterId(ADAPTER_OBSERVE_AAVE_RESERVE.to_string()))
    }
}

#[async_trait]
impl ObservationRuntimeAdapter for AaveReserveObservationRuntimeAdapter {
    async fn observe(
        &self,
        state_id: &StateId,
        io: &mut dyn IoProvider,
        binding: &CompiledObservationBinding,
        input: ObservationRuntimeInput<'_>,
    ) -> Result<Observation, StateError> {
        let payload: AaveReserveObservationPayload = decode_object_map(
            "compiled_observation_binding",
            &binding.binding_id,
            &binding.payload,
        )?;
        let subject = resolved_evm_subject_for_binding(binding, input.resolved_subjects)?;
        let pinned =
            pinned_evm_view_for_binding(binding, input.pinned_views, &payload.route_policy)?;
        let block_number = match pinned.anchor {
            ExecutionAnchor::Evm { block_number, .. } => block_number,
            ExecutionAnchor::Bitcoin { .. } => {
                unreachable!("validated by pinned_evm_view_for_binding")
            }
        };
        let reserve = payload
            .config
            .market
            .reserve(payload.config.reserve_id.as_str())
            .ok_or_else(|| {
                state_unknown_msg(
                    "aave_reserve_not_found",
                    format!(
                        "compiled Aave reserve binding `{}` referenced unknown reserve `{}`",
                        binding.binding_id, payload.config.reserve_id
                    ),
                )
            })?;
        let collateral_enabled = match payload.config.use_as_collateral_required {
            Some(required) => {
                let enabled = read_user_collateral_enabled(
                    state_id,
                    io,
                    payload.route_policy.network_id.as_str(),
                    payload.route_policy.control_scope.as_str(),
                    payload.config.market.pool_address.as_str(),
                    subject.address.as_str(),
                    reserve.reserve_index,
                    block_number,
                )
                .await?;
                Some((required, enabled))
            }
            None => None,
        };
        let raw = match collateral_enabled {
            Some((required, enabled)) if required != enabled => U256::ZERO,
            _ => {
                read_erc20_balance(
                    state_id,
                    io,
                    payload.route_policy.network_id.as_str(),
                    payload.route_policy.control_scope.as_str(),
                    reserve.a_token_address.as_str(),
                    subject.address.as_str(),
                    block_number,
                )
                .await?
            }
        };
        let decimals = match payload.projection.decimals {
            Some(decimals) => decimals,
            None => {
                read_token_decimals(
                    state_id,
                    io,
                    payload.route_policy.network_id.as_str(),
                    payload.route_policy.control_scope.as_str(),
                    reserve.a_token_address.as_str(),
                    block_number,
                )
                .await?
            }
        };
        let amount_dec = format_u256_units(&raw, decimals);
        let mut metadata = BTreeMap::new();
        metadata.insert(
            "market_id".to_string(),
            Value::String(payload.config.market.market_id.clone()),
        );
        metadata.insert(
            "reserve_id".to_string(),
            Value::String(payload.config.reserve_id.clone()),
        );
        if let Some((_, enabled)) = collateral_enabled {
            metadata.insert("collateral_enabled".to_string(), Value::Bool(enabled));
        }
        Ok(Observation {
            wallet_id: binding.observation_key.subject_id.clone(),
            symbol_id: payload.projection.symbol_id,
            display_symbol: payload.projection.display_symbol,
            kind: payload.projection.kind,
            role: payload.projection.role,
            network_id: payload.projection.network_id,
            protocol: payload
                .projection
                .protocol
                .or_else(|| Some(AAVE_V3_PROTOCOL_ID.to_string())),
            quantity: ObservationQuantity {
                raw_dec: raw.to_string(),
                decimals,
                amount_dec: amount_dec.clone(),
            },
            values: build_observation_values_from_resolved(
                binding,
                payload.underlying_symbol_id.as_str(),
                &amount_dec,
                input,
            )?,
            source: ObservationSource {
                balance_reader_kind: "protocol_position:aave_v3:reserve_position".to_string(),
                network_id: pinned.network_id.clone(),
                block_number,
            },
            metadata,
        })
    }
}

/// Runtime adapter for compiled Aave debt-position observations.
#[derive(Clone, Debug, Default)]
pub struct AaveDebtObservationRuntimeAdapter;

impl RuntimeAdapter for AaveDebtObservationRuntimeAdapter {
    fn id(&self) -> &AdapterId {
        static ID: std::sync::OnceLock<AdapterId> = std::sync::OnceLock::new();
        ID.get_or_init(|| AdapterId(ADAPTER_OBSERVE_AAVE_DEBT.to_string()))
    }
}

#[async_trait]
impl ObservationRuntimeAdapter for AaveDebtObservationRuntimeAdapter {
    async fn observe(
        &self,
        state_id: &StateId,
        io: &mut dyn IoProvider,
        binding: &CompiledObservationBinding,
        input: ObservationRuntimeInput<'_>,
    ) -> Result<Observation, StateError> {
        let payload: AaveDebtObservationPayload = decode_object_map(
            "compiled_observation_binding",
            &binding.binding_id,
            &binding.payload,
        )?;
        let subject = resolved_evm_subject_for_binding(binding, input.resolved_subjects)?;
        let pinned =
            pinned_evm_view_for_binding(binding, input.pinned_views, &payload.route_policy)?;
        let block_number = match pinned.anchor {
            ExecutionAnchor::Evm { block_number, .. } => block_number,
            ExecutionAnchor::Bitcoin { .. } => {
                unreachable!("validated by pinned_evm_view_for_binding")
            }
        };
        let reserve = payload
            .config
            .market
            .reserve(payload.config.reserve_id.as_str())
            .ok_or_else(|| {
                state_unknown_msg(
                    "aave_reserve_not_found",
                    format!(
                        "compiled Aave debt binding `{}` referenced unknown reserve `{}`",
                        binding.binding_id, payload.config.reserve_id
                    ),
                )
            })?;
        let (token_address, debt_kind) = match payload.config.debt_kind {
            AaveDebtKind::Variable => (
                reserve
                    .variable_debt_token_address
                    .as_deref()
                    .ok_or_else(|| {
                        state_unknown_msg(
                            "aave_debt_token_not_configured",
                            format!(
                                "compiled Aave debt binding `{}` was missing variable debt token config",
                                binding.binding_id
                            ),
                        )
                    })?,
                AaveDebtKind::Variable,
            ),
            AaveDebtKind::Stable => (
                reserve
                    .stable_debt_token_address
                    .as_deref()
                    .ok_or_else(|| {
                        state_unknown_msg(
                            "aave_debt_token_not_configured",
                            format!(
                                "compiled Aave debt binding `{}` was missing stable debt token config",
                                binding.binding_id
                            ),
                        )
                    })?,
                AaveDebtKind::Stable,
            ),
        };
        let raw = read_erc20_balance(
            state_id,
            io,
            payload.route_policy.network_id.as_str(),
            payload.route_policy.control_scope.as_str(),
            token_address,
            subject.address.as_str(),
            block_number,
        )
        .await?;
        let decimals = match payload.projection.decimals {
            Some(decimals) => decimals,
            None => {
                read_token_decimals(
                    state_id,
                    io,
                    payload.route_policy.network_id.as_str(),
                    payload.route_policy.control_scope.as_str(),
                    token_address,
                    block_number,
                )
                .await?
            }
        };
        let amount_dec = format_u256_units(&raw, decimals);
        let mut metadata = BTreeMap::new();
        metadata.insert(
            "market_id".to_string(),
            Value::String(payload.config.market.market_id.clone()),
        );
        metadata.insert(
            "reserve_id".to_string(),
            Value::String(payload.config.reserve_id.clone()),
        );
        metadata.insert(
            "debt_kind".to_string(),
            Value::String(debt_kind.as_str().to_string()),
        );
        Ok(Observation {
            wallet_id: binding.observation_key.subject_id.clone(),
            symbol_id: payload.projection.symbol_id,
            display_symbol: payload.projection.display_symbol,
            kind: payload.projection.kind,
            role: payload.projection.role,
            network_id: payload.projection.network_id,
            protocol: payload
                .projection
                .protocol
                .or_else(|| Some(AAVE_V3_PROTOCOL_ID.to_string())),
            quantity: ObservationQuantity {
                raw_dec: raw.to_string(),
                decimals,
                amount_dec: amount_dec.clone(),
            },
            values: build_observation_values_from_resolved(
                binding,
                payload.underlying_symbol_id.as_str(),
                &amount_dec,
                input,
            )?,
            source: ObservationSource {
                balance_reader_kind: "protocol_position:aave_v3:debt_position".to_string(),
                network_id: pinned.network_id.clone(),
                block_number,
            },
            metadata,
        })
    }
}

fn resolved_evm_subject_for_binding(
    binding: &CompiledObservationBinding,
    resolved_subjects: &BTreeMap<String, ResolvedSubject>,
) -> Result<EvmResolvedSubjectValue, StateError> {
    let subject = resolved_subjects
        .get(binding.observation_key.subject_id.as_str())
        .ok_or_else(|| {
            state_unknown_msg(
                "missing_resolved_subject",
                format!(
                    "missing resolved subject `{}` for observation binding `{}`",
                    binding.observation_key.subject_id, binding.binding_id
                ),
            )
        })?;
    serde_json::from_value(subject.value.clone()).map_err(|err| {
        state_unknown_msg(
            "semantic_runtime_subject_decode_failed",
            format!(
                "resolved subject `{}` decode failed for observation binding `{}`: {err}",
                subject.subject_id, binding.binding_id
            ),
        )
    })
}

fn pinned_evm_view_for_binding<'a>(
    binding: &CompiledObservationBinding,
    pinned_views: &'a BTreeMap<String, PinnedNetworkView>,
    route_policy: &mfm_state_portfolio::semantic::EvmRoutePolicy,
) -> Result<&'a PinnedNetworkView, StateError> {
    let pinned = pinned_views
        .get(binding.observation_key.network_view_id.as_str())
        .ok_or_else(|| {
            state_unknown_msg(
                "missing_pinned_view",
                format!(
                    "missing pinned execution view `{}` for observation binding `{}`",
                    binding.observation_key.network_view_id, binding.binding_id
                ),
            )
        })?;
    if pinned.network_id != route_policy.network_id {
        return Err(state_unknown_msg(
            "pinned_view_network_mismatch",
            format!(
                "pinned execution view `{}` targeted network `{}` but binding `{}` required `{}`",
                pinned.network_view_id,
                pinned.network_id,
                binding.binding_id,
                route_policy.network_id
            ),
        ));
    }
    match pinned.anchor {
        ExecutionAnchor::Evm { chain_id, .. } => {
            if chain_id != route_policy.chain_id {
                Err(state_unknown_msg(
                    "pinned_view_chain_id_mismatch",
                    format!(
                        "pinned execution view `{}` chain_id `{chain_id}` did not match expected chain_id `{}`",
                        pinned.network_view_id, route_policy.chain_id
                    ),
                ))
            } else {
                Ok(pinned)
            }
        }
        ExecutionAnchor::Bitcoin { .. } => Err(state_unknown_msg(
            "pinned_view_family_mismatch",
            format!(
                "observation binding `{}` expected an EVM execution view",
                binding.binding_id
            ),
        )),
    }
}

fn build_observation_values_from_resolved(
    binding: &CompiledObservationBinding,
    expected_instrument_id: &str,
    amount_dec: &str,
    input: ObservationRuntimeInput<'_>,
) -> Result<Vec<ObservationValue>, StateError> {
    let mut seen_quotes = BTreeSet::new();
    let mut values = Vec::with_capacity(binding.valuation_ids.len());
    for valuation_id in &binding.valuation_ids {
        let value = input
            .resolved_valuations
            .get(valuation_id.as_str())
            .ok_or_else(|| {
                state_unknown_msg(
                    "missing_resolved_valuation",
                    format!(
                        "missing resolved valuation `{valuation_id}` for observation binding `{}`",
                        binding.binding_id
                    ),
                )
            })?;
        if value.instrument_id != expected_instrument_id {
            return Err(state_unknown_msg(
                "observation_valuation_instrument_mismatch",
                format!(
                    "resolved valuation `{}` targeted instrument `{}` but observation binding `{}` expected `{}`",
                    value.valuation_id, value.instrument_id, binding.binding_id, expected_instrument_id
                ),
            ));
        }
        if !seen_quotes.insert(value.quote) {
            return Err(state_unknown_msg(
                "duplicate_observation_quote",
                format!(
                    "observation binding `{}` resolved multiple valuations for quote `{}`",
                    binding.binding_id, value.quote
                ),
            ));
        }
        values.push(ObservationValue {
            quote: value.quote,
            priced_symbol_id: value.priced_symbol_id.clone(),
            value_dec: multiply_decimal_strings(amount_dec, value.unit_price_dec.as_str())?,
            unit_price_dec: value.unit_price_dec.clone(),
            valuation_reader_kind: value.valuation_reader_kind.clone(),
            source_refs: value.source_refs.clone(),
        });
    }
    values.sort_by(|left, right| left.quote.cmp(&right.quote));
    Ok(values)
}

fn decode_object_map<T: for<'de> Deserialize<'de>>(
    entity: &'static str,
    id: &str,
    map: &BTreeMap<String, Value>,
) -> Result<T, StateError> {
    serde_json::from_value(Value::Object(
        map.iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect(),
    ))
    .map_err(|err| {
        state_unknown_msg(
            "semantic_runtime_payload_decode_failed",
            format!("{entity} `{id}` payload decode failed: {err}"),
        )
    })
}

async fn read_token_decimals(
    state_id: &StateId,
    io: &mut dyn IoProvider,
    network_id: &str,
    control_scope: &str,
    token_address: &str,
    block_number: u64,
) -> Result<u8, StateError> {
    let mut client = EvmIoClient::new(state_id.clone(), io);
    let call = JsonRpcCall::for_scope_and_network(
        control_scope,
        network_id,
        "eth_call",
        serde_json::json!([
            {
                "to": token_address,
                "data": mfm_evm_runtime::states::read::encode_erc20_decimals()
            },
            u64_hex_quantity(block_number)
        ]),
    );
    let res = client.call(call).await.map_err(state_from_io)?;
    let raw = parse_u256_hex_value(&res.response).map_err(|err| {
        state_unknown_msg(
            "evm_response_invalid",
            format!("token decimals response was invalid: {}", err.message),
        )
    })?;
    parse_u8_u256(raw).map_err(|err| {
        state_unknown_msg(
            "evm_response_invalid",
            format!("token decimals response was invalid: {}", err.message),
        )
    })
}

async fn read_erc20_balance(
    state_id: &StateId,
    io: &mut dyn IoProvider,
    network_id: &str,
    control_scope: &str,
    token_address: &str,
    wallet_address: &str,
    block_number: u64,
) -> Result<U256, StateError> {
    let wallet: Address = wallet_address.parse().map_err(|_| {
        state_unknown(
            "invalid_wallet_address",
            "wallet address was invalid for ERC-20 balance read",
        )
    })?;
    let mut client = EvmIoClient::new(state_id.clone(), io);
    let call = JsonRpcCall::for_scope_and_network(
        control_scope,
        network_id,
        "eth_call",
        serde_json::json!([
            {"to": token_address, "data": encode_erc20_balance_of(&wallet)},
            u64_hex_quantity(block_number)
        ]),
    );
    let res = client.call(call).await.map_err(state_from_io)?;
    parse_u256_hex_value(&res.response).map_err(|err| {
        state_unknown_msg(
            "evm_response_invalid",
            format!("token balance response was invalid: {}", err.message),
        )
    })
}

#[allow(clippy::too_many_arguments)]
async fn read_user_collateral_enabled(
    state_id: &StateId,
    io: &mut dyn IoProvider,
    network_id: &str,
    control_scope: &str,
    pool_address: &str,
    wallet_address: &str,
    reserve_index: u16,
    block_number: u64,
) -> Result<bool, StateError> {
    let wallet: Address = wallet_address.parse().map_err(|_| {
        state_unknown(
            "invalid_wallet_address",
            "wallet address was invalid for Aave collateral read",
        )
    })?;
    let mut client = EvmIoClient::new(state_id.clone(), io);
    let call = JsonRpcCall::for_scope_and_network(
        control_scope,
        network_id,
        "eth_call",
        serde_json::json!([
            {"to": pool_address, "data": encode_get_user_configuration(&wallet)},
            u64_hex_quantity(block_number)
        ]),
    );
    let res = client.call(call).await.map_err(state_from_io)?;
    let raw = parse_u256_hex_value(&res.response).map_err(|err| {
        state_unknown_msg(
            "evm_response_invalid",
            format!(
                "Aave user configuration response was invalid: {}",
                err.message
            ),
        )
    })?;
    let collateral_bit_index = usize::from(reserve_index) * 2;
    Ok(((raw >> collateral_bit_index) & U256::from(1u8)) == U256::from(1u8))
}

fn encode_get_user_configuration(wallet: &Address) -> String {
    let selector = function_selector("getUserConfiguration", &["address".to_string()]);
    let mut data = Vec::with_capacity(4 + 32);
    data.extend_from_slice(&selector);
    data.extend_from_slice(&[0u8; 12]);
    data.extend_from_slice(wallet.as_slice());
    format!("0x{}", hex::encode(data))
}

fn multiply_decimal_strings(left: &str, right: &str) -> Result<String, StateError> {
    let left = DecimalValue::parse(left)?;
    let right = DecimalValue::parse(right)?;
    let product = DecimalValue {
        digits: left.digits * right.digits,
        scale: left.scale + right.scale,
    };
    Ok(product.to_string_with_min_scale(left.scale.max(right.scale)))
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct DecimalValue {
    digits: BigInt,
    scale: u32,
}

impl DecimalValue {
    fn parse(input: &str) -> Result<Self, StateError> {
        let trimmed = input.trim();
        if trimmed.is_empty() || trimmed.starts_with('-') {
            return Err(state_unknown_msg(
                "invalid_decimal_string",
                format!("invalid decimal string `{input}`"),
            ));
        }
        let parts: Vec<_> = trimmed.split('.').collect();
        if parts.len() > 2
            || parts
                .iter()
                .any(|part| !part.is_empty() && !part.chars().all(|ch| ch.is_ascii_digit()))
        {
            return Err(state_unknown_msg(
                "invalid_decimal_string",
                format!("invalid decimal string `{input}`"),
            ));
        }
        let whole = parts[0];
        let frac = parts.get(1).copied().unwrap_or("");
        let digits = format!("{whole}{frac}");
        let digits = if digits.is_empty() {
            "0"
        } else {
            digits.as_str()
        };
        Ok(Self {
            digits: digits.parse().map_err(|_| {
                state_unknown_msg(
                    "invalid_decimal_string",
                    format!("invalid decimal string `{input}`"),
                )
            })?,
            scale: frac.len() as u32,
        })
    }

    fn to_string_with_min_scale(&self, min_scale: u32) -> String {
        let negative = self.digits.is_negative();
        let digits = self.digits.abs().to_string();
        let scale = self.scale as usize;
        let mut out = if scale == 0 {
            digits
        } else if digits.len() <= scale {
            format!("0.{}{}", "0".repeat(scale - digits.len()), digits)
        } else {
            let split = digits.len() - scale;
            format!("{}.{}", &digits[..split], &digits[split..])
        };
        if let Some((whole, frac)) = out.split_once('.') {
            let mut frac = frac.to_string();
            while frac.len() > min_scale as usize && frac.ends_with('0') {
                frac.pop();
            }
            if frac.len() < min_scale as usize {
                frac.push_str(&"0".repeat(min_scale as usize - frac.len()));
            }
            out = format!("{whole}.{frac}");
        } else if min_scale > 0 {
            out.push('.');
            out.push_str(&"0".repeat(min_scale as usize));
        }
        if negative && out != "0" {
            format!("-{out}")
        } else {
            out
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use mfm_machine::errors::{ErrorCategory, ErrorInfo, IoError};
    use mfm_machine::ids::{ArtifactId, ErrorCode, FactKey};
    use mfm_machine::io::{IoCall, IoResult};
    use mfm_state_portfolio::semantic::{ObservationKey, PositionSemantics};
    use mfm_state_symbol::model::QuoteCode;
    use serde_json::json;

    fn info(code: &'static str, message: &'static str) -> ErrorInfo {
        ErrorInfo {
            code: ErrorCode(code.to_string()),
            category: ErrorCategory::Unknown,
            retryable: false,
            message: message.to_string(),
            details: None,
        }
    }

    struct TestIo;

    #[async_trait]
    impl IoProvider for TestIo {
        async fn call(&mut self, call: IoCall) -> Result<IoResult, IoError> {
            let method = call
                .request
                .get("method")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let params = call
                .request
                .get("params")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let response = match method {
                "eth_call" => {
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
                    match (to, data) {
                        ("0x00000000000000000000000000000000000000a2", d)
                            if d.starts_with("0x70a08231") =>
                        {
                            ok_u256_as_32byte_hex(1_500_000)
                        }
                        ("0x00000000000000000000000000000000000000a3", d)
                            if d.starts_with("0x70a08231") =>
                        {
                            ok_u256_as_32byte_hex(750_000)
                        }
                        ("0x00000000000000000000000000000000000000a2", "0x313ce567")
                        | ("0x00000000000000000000000000000000000000a3", "0x313ce567") => {
                            ok_u256_as_32byte_hex(6)
                        }
                        ("0x0000000000000000000000000000000000000abc", d)
                            if d == encode_get_user_configuration(
                                &"0x000000000000000000000000000000000000beef"
                                    .parse()
                                    .expect("wallet"),
                            ) =>
                        {
                            ok_u256_as_32byte_hex(1 << 2)
                        }
                        _ => {
                            return Err(IoError::Other(info(
                                "unexpected_eth_call",
                                "unexpected eth_call payload",
                            )))
                        }
                    }
                }
                _ => {
                    return Err(IoError::Other(info(
                        "unexpected_method",
                        "unexpected method",
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
            Ok(ArtifactId("artifact".to_string()))
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
    }

    fn ok_u256_as_32byte_hex(n: u64) -> Value {
        let hex_val = format!("{:x}", n);
        let padded = format!("{}{}", "0".repeat(64 - hex_val.len()), hex_val);
        json!(format!("0x{padded}"))
    }

    fn market() -> crate::portfolio::model::AaveMarketConfig {
        crate::portfolio::model::AaveMarketConfig {
            market_id: "aave-v3-mainnet".to_string(),
            network_id: "ethereum-mainnet".to_string(),
            chain_id: 1,
            pool_address: "0x0000000000000000000000000000000000000abc".to_string(),
            reserves: vec![crate::portfolio::model::AaveReserveConfig {
                reserve_id: "usdc".to_string(),
                reserve_index: 1,
                underlying_token_address: "0x00000000000000000000000000000000000000a1".to_string(),
                a_token_address: "0x00000000000000000000000000000000000000a2".to_string(),
                variable_debt_token_address: Some(
                    "0x00000000000000000000000000000000000000a3".to_string(),
                ),
                stable_debt_token_address: None,
                metadata: BTreeMap::new(),
            }],
            metadata: BTreeMap::new(),
        }
    }

    fn resolved_subjects() -> BTreeMap<String, ResolvedSubject> {
        BTreeMap::from([(
            "wallet_main".to_string(),
            ResolvedSubject {
                subject_id: "wallet_main".to_string(),
                kind: mfm_state_portfolio::semantic::SubjectKind::EvmAddress,
                value: serde_json::to_value(EvmResolvedSubjectValue {
                    network_id: "ethereum-mainnet".to_string(),
                    address: "0x000000000000000000000000000000000000beef".to_string(),
                    implementation_kind: "address_only".to_string(),
                    capabilities: mfm_state_wallet::model::WalletCapabilities {
                        can_resolve_address: true,
                        can_sign: false,
                        can_submit: false,
                    },
                })
                .expect("subject"),
            },
        )])
    }

    fn pinned_views() -> BTreeMap<String, PinnedNetworkView> {
        BTreeMap::from([(
            "ethereum-mainnet".to_string(),
            PinnedNetworkView {
                network_view_id: "ethereum-mainnet".to_string(),
                network_id: "ethereum-mainnet".to_string(),
                family: mfm_state_portfolio::semantic::NetworkFamily::Evm,
                anchor: ExecutionAnchor::Evm {
                    chain_id: 1,
                    block_number: 100,
                },
            },
        )])
    }

    fn resolved_valuations() -> BTreeMap<String, mfm_state_portfolio::semantic::ResolvedUnitPrice> {
        BTreeMap::from([(
            "usdc.quote.usd".to_string(),
            mfm_state_portfolio::semantic::ResolvedUnitPrice {
                valuation_id: "usdc.quote.usd".to_string(),
                instrument_id: "usdc.wallet.ethereum-mainnet".to_string(),
                priced_symbol_id: "usdc.wallet.ethereum-mainnet".to_string(),
                quote: QuoteCode::Usd,
                unit_price_dec: "1.00".to_string(),
                valuation_reader_kind: "fixed_unit_price".to_string(),
                source_refs: Vec::new(),
            },
        )])
    }

    #[tokio::test]
    async fn reserve_runtime_emits_collateral_metadata_and_values() {
        let adapter = AaveReserveObservationRuntimeAdapter;
        let mut io = TestIo;
        let binding = CompiledObservationBinding {
            binding_id: "binding.aave.reserve".to_string(),
            observation_key: ObservationKey {
                subject_id: "wallet_main".to_string(),
                network_view_id: "ethereum-mainnet".to_string(),
                instrument_id: "usdc.wallet.ethereum-mainnet".to_string(),
                position_kind: PositionSemantics::LendingDeposit,
                venue_id: None,
                discriminator: Some("aave_v3.usdc.collateral.ethereum-mainnet".to_string()),
            },
            adapter: AdapterId(ADAPTER_OBSERVE_AAVE_RESERVE.to_string()),
            valuation_ids: vec!["usdc.quote.usd".to_string()],
            payload: serde_json::from_value(json!({
                "projection": {
                    "symbol_id": "aave_v3.usdc.collateral.ethereum-mainnet",
                    "display_symbol": "aUSDC",
                    "kind": "protocol_position",
                    "role": "collateral",
                    "network_id": "ethereum-mainnet",
                    "protocol": "aave_v3",
                    "decimals": null
                },
                "route_policy": {
                    "network_id": "ethereum-mainnet",
                    "chain_id": 1u64,
                    "control_scope": "rpc.mainnet"
                },
                "underlying_symbol_id": "usdc.wallet.ethereum-mainnet",
                "config": {
                    "market": market(),
                    "reserve_id": "usdc",
                    "use_as_collateral_required": true
                }
            }))
            .expect("payload"),
        };

        let observation = adapter
            .observe(
                &StateId::must_new("portfolio.main.observe_aave_reserve".to_string()),
                &mut io,
                &binding,
                ObservationRuntimeInput {
                    resolved_subjects: &resolved_subjects(),
                    pinned_views: &pinned_views(),
                    resolved_valuations: &resolved_valuations(),
                },
            )
            .await
            .expect("observation");

        assert_eq!(observation.quantity.decimals, 6);
        assert_eq!(observation.quantity.amount_dec, "1.500000");
        assert_eq!(observation.metadata["collateral_enabled"], json!(true));
        assert_eq!(
            observation.values[0].priced_symbol_id,
            "usdc.wallet.ethereum-mainnet"
        );
    }

    #[tokio::test]
    async fn debt_runtime_emits_debt_kind_metadata_and_values() {
        let adapter = AaveDebtObservationRuntimeAdapter;
        let mut io = TestIo;
        let binding = CompiledObservationBinding {
            binding_id: "binding.aave.debt".to_string(),
            observation_key: ObservationKey {
                subject_id: "wallet_main".to_string(),
                network_view_id: "ethereum-mainnet".to_string(),
                instrument_id: "usdc.wallet.ethereum-mainnet".to_string(),
                position_kind: PositionSemantics::LendingDebt,
                venue_id: None,
                discriminator: Some("aave_v3.usdc.debt.ethereum-mainnet".to_string()),
            },
            adapter: AdapterId(ADAPTER_OBSERVE_AAVE_DEBT.to_string()),
            valuation_ids: vec!["usdc.quote.usd".to_string()],
            payload: serde_json::from_value(json!({
                "projection": {
                    "symbol_id": "aave_v3.usdc.debt.ethereum-mainnet",
                    "display_symbol": "vUSDC",
                    "kind": "protocol_position",
                    "role": "debt",
                    "network_id": "ethereum-mainnet",
                    "protocol": "aave_v3",
                    "decimals": null
                },
                "route_policy": {
                    "network_id": "ethereum-mainnet",
                    "chain_id": 1u64,
                    "control_scope": "rpc.mainnet"
                },
                "underlying_symbol_id": "usdc.wallet.ethereum-mainnet",
                "config": {
                    "market": market(),
                    "reserve_id": "usdc",
                    "debt_kind": "variable"
                }
            }))
            .expect("payload"),
        };

        let observation = adapter
            .observe(
                &StateId::must_new("portfolio.main.observe_aave_debt".to_string()),
                &mut io,
                &binding,
                ObservationRuntimeInput {
                    resolved_subjects: &resolved_subjects(),
                    pinned_views: &pinned_views(),
                    resolved_valuations: &resolved_valuations(),
                },
            )
            .await
            .expect("observation");

        assert_eq!(observation.quantity.decimals, 6);
        assert_eq!(observation.quantity.amount_dec, "0.750000");
        assert_eq!(observation.metadata["debt_kind"], json!("variable"));
        assert_eq!(observation.values[0].value_dec, "0.750000");
    }
}
