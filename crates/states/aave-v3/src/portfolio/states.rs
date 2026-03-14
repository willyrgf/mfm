use std::collections::{BTreeMap, HashMap};

use alloy_primitives::{Address, U256};
use async_trait::async_trait;
use mfm_collectors_rpc_control::{EvmIoClient, JsonRpcCall};
use mfm_evm_core::abi::function_selector;
use mfm_evm_core::encoding::{
    encode_erc20_balance_of, format_u256_units, parse_u256_hex_value, parse_u8_u256,
    u64_hex_quantity,
};
use mfm_machine::context::DynContext;
use mfm_machine::errors::StateError;
use mfm_machine::ids::{ContextKey, StateId};
use mfm_machine::io::IoProvider;
use mfm_machine::meta::StateMeta;
use mfm_machine::recorder::EventRecorder;
use mfm_machine::state::{SnapshotPolicy, State, StateOutcome};
use mfm_state_common::ctx::{read_typed, write_json};
use mfm_state_common::errors::{state_from_io, state_unknown, state_unknown_msg};
use mfm_state_common::states::meta;
use mfm_state_symbol::model::{Observation, ObservationQuantity, ObservationSource, SymbolConfig};
use mfm_state_symbol::states::{
    build_observation_values, DirectPriceValue, NetworkRouteConfig, PinnedNetwork,
};
use mfm_state_wallet::model::{ResolvedWallet, WalletConfig};
use serde_json::Value;

use crate::portfolio::model::{
    decode_aave_protocol_position_config, AaveDebtKind, AaveProtocolPositionConfig,
    AAVE_V3_PROTOCOL_ID,
};

/// Collects canonical observations for Aave V3 `protocol_position` symbols.
#[derive(Clone, Debug)]
pub struct CollectAaveObservationsState {
    /// Stable state identifier assigned by the execution plan.
    pub state_id: StateId,
    /// Wallet configs whose symbol assignments should be processed.
    pub wallets: Vec<WalletConfig>,
    /// Aave symbol configs available to the portfolio.
    pub symbols: Vec<SymbolConfig>,
    /// Routed network configs used for EVM reads.
    pub networks: Vec<NetworkRouteConfig>,
    /// Context key that contains the resolved wallets.
    pub resolved_wallets_key: ContextKey,
    /// Context key that contains the pinned networks.
    pub network_pins_key: ContextKey,
    /// Context key that contains the resolved direct prices.
    pub direct_prices_key: ContextKey,
    /// Context key that receives the canonical observations.
    pub output_key: ContextKey,
}

#[async_trait]
impl State for CollectAaveObservationsState {
    fn meta(&self) -> StateMeta {
        meta::fetch_data()
    }

    async fn handle(
        &self,
        ctx: &mut dyn DynContext,
        io: &mut dyn IoProvider,
        _rec: &mut dyn EventRecorder,
    ) -> Result<StateOutcome, StateError> {
        let resolved_wallets: Vec<ResolvedWallet> = read_typed(
            ctx,
            &self.resolved_wallets_key,
            "missing_resolved_wallets",
            "missing resolved wallets in context",
            "resolved_wallets_decode_failed",
            "failed to decode resolved wallets",
        )?;
        let pins: Vec<PinnedNetwork> = read_typed(
            ctx,
            &self.network_pins_key,
            "missing_network_pins",
            "missing network pins in context",
            "network_pins_decode_failed",
            "failed to decode network pins",
        )?;
        let direct_prices: Vec<DirectPriceValue> = read_typed(
            ctx,
            &self.direct_prices_key,
            "missing_direct_prices",
            "missing direct prices in context",
            "direct_prices_decode_failed",
            "failed to decode direct prices",
        )?;

        let resolved_wallets_by_id: HashMap<_, _> = resolved_wallets
            .iter()
            .map(|wallet| (wallet.wallet_id.as_str(), wallet))
            .collect();
        let pins_by_id: HashMap<_, _> = pins
            .iter()
            .map(|pin| (pin.network_id.as_str(), pin))
            .collect();
        let routes_by_id: HashMap<_, _> = self
            .networks
            .iter()
            .map(|network| (network.network_id.as_str(), network))
            .collect();
        let symbols_by_id: HashMap<_, _> = self
            .symbols
            .iter()
            .map(|symbol| (symbol.symbol_id.as_str(), symbol))
            .collect();

        let mut observations = Vec::new();
        let mut wallets = self.wallets.clone();
        wallets.sort_by(|left, right| left.wallet_id.cmp(&right.wallet_id));
        for wallet_cfg in wallets {
            let Some(wallet) = resolved_wallets_by_id.get(wallet_cfg.wallet_id.as_str()) else {
                return Err(state_unknown_msg(
                    "resolved_wallet_not_found",
                    format!("resolved wallet `{}` was not found", wallet_cfg.wallet_id),
                ));
            };
            let Some(pin) = pins_by_id.get(wallet.network_id.as_str()) else {
                return Err(state_unknown_msg(
                    "missing_network_pin",
                    format!("network pin `{}` was not available", wallet.network_id),
                ));
            };
            let Some(route) = routes_by_id.get(wallet.network_id.as_str()) else {
                return Err(state_unknown_msg(
                    "missing_network_route_config",
                    format!(
                        "network route config `{}` was not available",
                        wallet.network_id
                    ),
                ));
            };

            let mut symbol_ids = wallet_cfg.symbol_ids.clone();
            symbol_ids.sort();
            for symbol_id in symbol_ids {
                let Some(symbol) = symbols_by_id.get(symbol_id.as_str()) else {
                    return Err(state_unknown_msg(
                        "symbol_not_found",
                        format!("symbol `{symbol_id}` was not found"),
                    ));
                };
                observations.push(
                    build_aave_observation(
                        &self.state_id,
                        io,
                        wallet,
                        symbol,
                        pin,
                        &route.network_id,
                        &direct_prices,
                    )
                    .await?,
                );
            }
        }

        observations.sort_by(|left, right| {
            (left.wallet_id.as_str(), left.symbol_id.as_str())
                .cmp(&(right.wallet_id.as_str(), right.symbol_id.as_str()))
        });
        let value = serde_json::to_value(&observations).map_err(|_| {
            state_unknown(
                "observations_serialize_failed",
                "failed to serialize observations",
            )
        })?;
        write_json(ctx, self.output_key.clone(), value)?;

        Ok(StateOutcome {
            snapshot: SnapshotPolicy::OnSuccess,
        })
    }
}

async fn build_aave_observation(
    state_id: &StateId,
    io: &mut dyn IoProvider,
    wallet: &ResolvedWallet,
    symbol: &SymbolConfig,
    pin: &PinnedNetwork,
    network_id: &str,
    direct_prices: &[DirectPriceValue],
) -> Result<Observation, StateError> {
    let cfg = decode_aave_protocol_position_config(symbol).map_err(|err| {
        state_unknown_msg(
            "invalid_aave_reader_config",
            format!(
                "symbol `{}` had invalid Aave config: {err}",
                symbol.symbol_id
            ),
        )
    })?;
    let reserve = cfg.market().reserve(cfg.reserve_id()).ok_or_else(|| {
        state_unknown_msg(
            "aave_reserve_not_found",
            format!(
                "symbol `{}` referenced unknown reserve `{}`",
                symbol.symbol_id,
                cfg.reserve_id()
            ),
        )
    })?;

    let (token_address, raw, balance_reader_kind, metadata) = match &cfg {
        AaveProtocolPositionConfig::ReservePosition(reserve_cfg) => {
            let collateral_enabled = match reserve_cfg.use_as_collateral_required {
                Some(required) => {
                    let enabled = read_user_collateral_enabled(
                        state_id,
                        io,
                        network_id,
                        reserve_cfg.market.pool_address.as_str(),
                        &wallet.address,
                        reserve.reserve_index,
                        pin.block_number,
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
                        network_id,
                        reserve.a_token_address.as_str(),
                        &wallet.address,
                        pin.block_number,
                    )
                    .await?
                }
            };

            let mut metadata = BTreeMap::new();
            metadata.insert(
                "market_id".to_string(),
                Value::String(reserve_cfg.market.market_id.clone()),
            );
            metadata.insert(
                "reserve_id".to_string(),
                Value::String(reserve_cfg.reserve_id.clone()),
            );
            if let Some((_, enabled)) = collateral_enabled {
                metadata.insert("collateral_enabled".to_string(), Value::Bool(enabled));
            }
            (
                reserve.a_token_address.as_str(),
                raw,
                "protocol_position:aave_v3:reserve_position".to_string(),
                metadata,
            )
        }
        AaveProtocolPositionConfig::DebtPosition(debt_cfg) => {
            let (token_address, debt_kind) = match debt_cfg.debt_kind {
                AaveDebtKind::Variable => (
                    reserve
                        .variable_debt_token_address
                        .as_deref()
                        .ok_or_else(|| {
                            state_unknown_msg(
                                "aave_debt_token_not_configured",
                                format!(
                                    "symbol `{}` was missing variable debt token config",
                                    symbol.symbol_id
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
                                    "symbol `{}` was missing stable debt token config",
                                    symbol.symbol_id
                                ),
                            )
                        })?,
                    AaveDebtKind::Stable,
                ),
            };
            let raw = read_erc20_balance(
                state_id,
                io,
                network_id,
                token_address,
                &wallet.address,
                pin.block_number,
            )
            .await?;

            let mut metadata = BTreeMap::new();
            metadata.insert(
                "market_id".to_string(),
                Value::String(debt_cfg.market.market_id.clone()),
            );
            metadata.insert(
                "reserve_id".to_string(),
                Value::String(debt_cfg.reserve_id.clone()),
            );
            metadata.insert(
                "debt_kind".to_string(),
                Value::String(debt_kind.as_str().to_string()),
            );
            (
                token_address,
                raw,
                "protocol_position:aave_v3:debt_position".to_string(),
                metadata,
            )
        }
    };

    let decimals = match symbol.decimals {
        Some(decimals) => decimals,
        None => {
            read_token_decimals(state_id, io, network_id, token_address, pin.block_number).await?
        }
    };
    let amount_dec = format_u256_units(&raw, decimals);
    let values = build_observation_values(symbol, &amount_dec, direct_prices)?;

    Ok(Observation {
        wallet_id: wallet.wallet_id.clone(),
        symbol_id: symbol.symbol_id.clone(),
        display_symbol: symbol.display_symbol.clone(),
        kind: symbol.kind,
        role: symbol.role,
        network_id: symbol.network_id.clone(),
        protocol: Some(AAVE_V3_PROTOCOL_ID.to_string()),
        quantity: ObservationQuantity {
            raw_dec: raw.to_string(),
            decimals,
            amount_dec,
        },
        values,
        source: ObservationSource {
            balance_reader_kind,
            network_id: pin.network_id.clone(),
            block_number: pin.block_number,
        },
        metadata,
    })
}

async fn read_token_decimals(
    state_id: &StateId,
    io: &mut dyn IoProvider,
    network_id: &str,
    token_address: &str,
    block_number: u64,
) -> Result<u8, StateError> {
    let mut client = EvmIoClient::new(state_id.clone(), io);
    let call = JsonRpcCall::new(
        "eth_call",
        serde_json::json!([
            {
                "to": token_address,
                "data": mfm_evm_runtime::states::read::encode_erc20_decimals()
            },
            u64_hex_quantity(block_number)
        ]),
    )
    .with_network_id(network_id);
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
    let call = JsonRpcCall::new(
        "eth_call",
        serde_json::json!([
            {"to": token_address, "data": encode_erc20_balance_of(&wallet)},
            u64_hex_quantity(block_number)
        ]),
    )
    .with_network_id(network_id);
    let res = client.call(call).await.map_err(state_from_io)?;
    parse_u256_hex_value(&res.response).map_err(|err| {
        state_unknown_msg(
            "evm_response_invalid",
            format!("token balance response was invalid: {}", err.message),
        )
    })
}

async fn read_user_collateral_enabled(
    state_id: &StateId,
    io: &mut dyn IoProvider,
    network_id: &str,
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
    let call = JsonRpcCall::new(
        "eth_call",
        serde_json::json!([
            {"to": pool_address, "data": encode_get_user_configuration(&wallet)},
            u64_hex_quantity(block_number)
        ]),
    )
    .with_network_id(network_id);
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    use async_trait::async_trait;
    use mfm_machine::context::DynContext;
    use mfm_machine::errors::{ContextError, ErrorCategory, ErrorInfo, IoError, RunError};
    use mfm_machine::events::DomainEvent;
    use mfm_machine::ids::{ErrorCode, FactKey};
    use mfm_machine::io::{IoCall, IoResult};
    use mfm_state_symbol::model::{
        BalanceReaderConfig, QuoteCode, QuoteValuationConfig, SymbolKind, SymbolRole,
        SymbolValuationConfig, ValuationReaderConfig,
    };
    use serde_json::json;

    use crate::portfolio::model::{
        AaveMarketConfig, AaveReserveConfig, AAVE_V3_READER_DEBT_POSITION,
        AAVE_V3_READER_RESERVE_POSITION,
    };

    #[derive(Default)]
    struct MapContext {
        inner: HashMap<String, Value>,
    }

    impl DynContext for MapContext {
        fn read(&self, key: &ContextKey) -> Result<Option<Value>, ContextError> {
            Ok(self.inner.get(&key.0).cloned())
        }

        fn write(&mut self, key: ContextKey, value: Value) -> Result<(), ContextError> {
            self.inner.insert(key.0, value);
            Ok(())
        }

        fn delete(&mut self, key: &ContextKey) -> Result<(), ContextError> {
            self.inner.remove(&key.0);
            Ok(())
        }

        fn dump(&self) -> Result<Value, ContextError> {
            let mut out = serde_json::Map::new();
            for (key, value) in &self.inner {
                out.insert(key.clone(), value.clone());
            }
            Ok(Value::Object(out))
        }
    }

    fn info(code: &'static str, message: &'static str) -> ErrorInfo {
        ErrorInfo {
            code: ErrorCode(code.to_string()),
            category: ErrorCategory::Unknown,
            retryable: false,
            message: message.to_string(),
            details: None,
        }
    }

    #[derive(Default)]
    struct MockIo;

    #[async_trait]
    impl IoProvider for MockIo {
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
                        ("0x00000000000000000000000000000000000000b2", d)
                            if d.starts_with("0x70a08231") =>
                        {
                            ok_u256_as_32byte_hex(200_000_000)
                        }
                        ("0x00000000000000000000000000000000000000a3", d)
                            if d.starts_with("0x70a08231") =>
                        {
                            ok_u256_as_32byte_hex(750_000)
                        }
                        ("0x00000000000000000000000000000000000000b2", "0x313ce567")
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
        ) -> Result<mfm_machine::ids::ArtifactId, IoError> {
            Err(IoError::Other(info("unexpected_record_value", "not used")))
        }

        async fn get_recorded_fact(
            &mut self,
            _key: &FactKey,
        ) -> Result<Option<mfm_machine::ids::ArtifactId>, IoError> {
            Ok(None)
        }

        async fn now_millis(&mut self) -> Result<u64, IoError> {
            Ok(0)
        }

        async fn random_bytes(&mut self, n: usize) -> Result<Vec<u8>, IoError> {
            Ok(vec![0; n])
        }
    }

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

    fn ok_u256_as_32byte_hex(n: u64) -> Value {
        let hex_val = format!("{:x}", n);
        let padded = format!("{}{}", "0".repeat(64 - hex_val.len()), hex_val);
        json!(format!("0x{padded}"))
    }

    fn market() -> AaveMarketConfig {
        AaveMarketConfig {
            market_id: "aave-v3-mainnet".to_string(),
            network_id: "ethereum-mainnet".to_string(),
            chain_id: 1,
            pool_address: "0x0000000000000000000000000000000000000abc".to_string(),
            reserves: vec![
                AaveReserveConfig {
                    reserve_id: "usdc".to_string(),
                    reserve_index: 0,
                    underlying_token_address: "0x00000000000000000000000000000000000000a1"
                        .to_string(),
                    a_token_address: "0x00000000000000000000000000000000000000a2".to_string(),
                    variable_debt_token_address: Some(
                        "0x00000000000000000000000000000000000000a3".to_string(),
                    ),
                    stable_debt_token_address: None,
                    metadata: BTreeMap::new(),
                },
                AaveReserveConfig {
                    reserve_id: "wbtc".to_string(),
                    reserve_index: 1,
                    underlying_token_address: "0x00000000000000000000000000000000000000b1"
                        .to_string(),
                    a_token_address: "0x00000000000000000000000000000000000000b2".to_string(),
                    variable_debt_token_address: Some(
                        "0x00000000000000000000000000000000000000b3".to_string(),
                    ),
                    stable_debt_token_address: None,
                    metadata: BTreeMap::new(),
                },
            ],
            metadata: BTreeMap::new(),
        }
    }

    fn fixed_quote(priced_symbol_id: &str, unit_price_dec: &str) -> QuoteValuationConfig {
        QuoteValuationConfig {
            quote: QuoteCode::Usd,
            priced_symbol_id: priced_symbol_id.to_string(),
            reader: ValuationReaderConfig::FixedUnitPrice {
                unit_price_dec: unit_price_dec.to_string(),
            },
        }
    }

    struct ProtocolSymbolSpec<'a> {
        symbol_id: &'a str,
        display_symbol: &'a str,
        role: SymbolRole,
        reader: &'a str,
        config: Value,
        decimals: Option<u8>,
        underlying_symbol_id: &'a str,
        unit_price_dec: &'a str,
    }

    fn protocol_symbol(spec: ProtocolSymbolSpec<'_>) -> SymbolConfig {
        SymbolConfig {
            symbol_id: spec.symbol_id.to_string(),
            display_symbol: Some(spec.display_symbol.to_string()),
            kind: SymbolKind::ProtocolPosition,
            role: spec.role,
            network_id: "ethereum-mainnet".to_string(),
            protocol: Some(AAVE_V3_PROTOCOL_ID.to_string()),
            balance_reader: BalanceReaderConfig::ProtocolPosition {
                protocol: AAVE_V3_PROTOCOL_ID.to_string(),
                reader: spec.reader.to_string(),
                config: serde_json::from_value(spec.config).expect("config map"),
            },
            valuation: SymbolValuationConfig {
                quotes: vec![fixed_quote(spec.underlying_symbol_id, spec.unit_price_dec)],
            },
            decimals: spec.decimals,
            underlying_symbol_id: Some(spec.underlying_symbol_id.to_string()),
            metadata: BTreeMap::new(),
        }
    }

    #[tokio::test]
    async fn collects_aave_reserve_and_debt_observations() {
        let mut ctx = MapContext::default();
        ctx.write(
            ContextKey("resolved_wallets".to_string()),
            json!([{
                "wallet_id": "wallet_main",
                "address": "0x000000000000000000000000000000000000beef",
                "network_id": "ethereum-mainnet",
                "implementation_kind": "address_only",
                "capabilities": {
                    "can_resolve_address": true,
                    "can_sign": false,
                    "can_submit": false
                },
                "signer": null
            }]),
        )
        .expect("resolved wallets");
        ctx.write(
            ContextKey("network_pins".to_string()),
            json!([{
                "network_id": "ethereum-mainnet",
                "chain_id": 1,
                "block_number": 100
            }]),
        )
        .expect("network pins");
        ctx.write(ContextKey("direct_prices".to_string()), json!([]))
            .expect("direct prices");

        let wallets = vec![WalletConfig {
            wallet_id: "wallet_main".to_string(),
            address: "0x000000000000000000000000000000000000beef".to_string(),
            network_id: "ethereum-mainnet".to_string(),
            implementation: mfm_state_wallet::model::WalletImplementationConfig::AddressOnly {},
            symbol_ids: vec![
                "aave_v3.wbtc.collateral.ethereum-mainnet".to_string(),
                "aave_v3.usdc.debt.ethereum-mainnet".to_string(),
            ],
            metadata: BTreeMap::new(),
        }];
        let symbols = vec![
            protocol_symbol(ProtocolSymbolSpec {
                symbol_id: "aave_v3.wbtc.collateral.ethereum-mainnet",
                display_symbol: "WBTC",
                role: SymbolRole::Collateral,
                reader: AAVE_V3_READER_RESERVE_POSITION,
                config: json!({
                    "market": market(),
                    "reserve_id": "wbtc",
                    "use_as_collateral_required": true
                }),
                decimals: None,
                underlying_symbol_id: "wbtc.wallet.ethereum-mainnet",
                unit_price_dec: "70000.00",
            }),
            protocol_symbol(ProtocolSymbolSpec {
                symbol_id: "aave_v3.usdc.debt.ethereum-mainnet",
                display_symbol: "USDC",
                role: SymbolRole::Debt,
                reader: AAVE_V3_READER_DEBT_POSITION,
                config: json!({
                    "market": market(),
                    "reserve_id": "usdc",
                    "debt_kind": "variable"
                }),
                decimals: None,
                underlying_symbol_id: "usdc.wallet.ethereum-mainnet",
                unit_price_dec: "1.00",
            }),
        ];

        let mut io = MockIo;
        let mut rec = NoopRecorder;
        CollectAaveObservationsState {
            state_id: StateId::must_new("aave.main.collect".to_string()),
            wallets,
            symbols,
            networks: vec![NetworkRouteConfig {
                network_id: "ethereum-mainnet".to_string(),
            }],
            resolved_wallets_key: ContextKey("resolved_wallets".to_string()),
            network_pins_key: ContextKey("network_pins".to_string()),
            direct_prices_key: ContextKey("direct_prices".to_string()),
            output_key: ContextKey("observations".to_string()),
        }
        .handle(&mut ctx, &mut io, &mut rec)
        .await
        .expect("collect Aave observations");

        let observations: Vec<Observation> = serde_json::from_value(
            ctx.read(&ContextKey("observations".to_string()))
                .expect("read")
                .expect("value"),
        )
        .expect("decode observations");

        assert_eq!(observations.len(), 2);
        assert_eq!(
            observations[0].symbol_id,
            "aave_v3.usdc.debt.ethereum-mainnet"
        );
        assert_eq!(observations[0].quantity.raw_dec, "750000");
        assert_eq!(
            observations[0].values[0].priced_symbol_id,
            "usdc.wallet.ethereum-mainnet"
        );
        assert_eq!(
            observations[0].metadata["debt_kind"],
            Value::String("variable".to_string())
        );

        assert_eq!(
            observations[1].symbol_id,
            "aave_v3.wbtc.collateral.ethereum-mainnet"
        );
        assert_eq!(observations[1].quantity.raw_dec, "200000000");
        assert_eq!(
            observations[1].metadata["collateral_enabled"],
            Value::Bool(true)
        );
    }

    #[tokio::test]
    async fn zeroes_collateral_observation_when_required_flag_is_not_enabled() {
        let symbol = protocol_symbol(ProtocolSymbolSpec {
            symbol_id: "aave_v3.usdc.collateral.ethereum-mainnet",
            display_symbol: "USDC",
            role: SymbolRole::Collateral,
            reader: AAVE_V3_READER_RESERVE_POSITION,
            config: json!({
                "market": market(),
                "reserve_id": "usdc",
                "use_as_collateral_required": true
            }),
            decimals: Some(6),
            underlying_symbol_id: "usdc.wallet.ethereum-mainnet",
            unit_price_dec: "1.00",
        });
        let resolved_wallet = ResolvedWallet {
            wallet_id: "wallet_main".to_string(),
            address: "0x000000000000000000000000000000000000beef".to_string(),
            network_id: "ethereum-mainnet".to_string(),
            implementation_kind: "address_only".to_string(),
            capabilities: mfm_state_wallet::model::WalletCapabilities {
                can_resolve_address: true,
                can_sign: false,
                can_submit: false,
            },
            signer: None,
        };
        let pin = PinnedNetwork {
            network_id: "ethereum-mainnet".to_string(),
            chain_id: 1,
            block_number: 100,
        };

        let mut io = MockIo;
        let observation = build_aave_observation(
            &StateId::must_new("aave.main.observe".to_string()),
            &mut io,
            &resolved_wallet,
            &symbol,
            &pin,
            "ethereum-mainnet",
            &[],
        )
        .await
        .expect("build observation");

        assert_eq!(observation.quantity.raw_dec, "0");
        assert_eq!(observation.quantity.amount_dec, "0.000000");
        assert_eq!(
            observation.metadata["collateral_enabled"],
            Value::Bool(false)
        );
    }
}
