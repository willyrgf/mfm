use std::collections::{BTreeMap, HashMap};

use alloy_primitives::{Address, U256};
use async_trait::async_trait;
use mfm_collectors_rpc_control::{EvmIoClient, JsonRpcCall, DEFAULT_CONTROL_SCOPE};
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
use mfm_state_wallet::model::{ResolvedWallet, WalletConfig};
use num_bigint::BigInt;
use num_traits::{Signed, Zero};
use serde::{Deserialize, Serialize};

use crate::model::{
    BalanceReaderConfig, Observation, ObservationQuantity, ObservationSource, ObservationValue,
    ObservationValueSourceRef, QuoteCode, SymbolConfig, ValuationReaderConfig,
    ValuationSourceReaderConfig, ValuationSourceRegistry,
};

fn default_control_scope() -> String {
    DEFAULT_CONTROL_SCOPE.to_string()
}

/// Minimal network routing surface required by symbol runtime states.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkRouteConfig {
    /// Stable network identifier.
    pub network_id: String,
    /// Stable control-plane scope used for managed rpc.control reads on this network.
    #[serde(default = "default_control_scope")]
    pub control_scope: String,
}

/// Minimal pinned-network view consumed by symbol runtime states.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PinnedNetwork {
    /// Stable network identifier.
    pub network_id: String,
    /// EVM chain id.
    pub chain_id: u64,
    /// Concrete pinned block number.
    pub block_number: u64,
}

/// Resolved direct price value for one valuation source.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirectPriceValue {
    /// Stable valuation source id.
    pub source_id: String,
    /// Stable network identifier.
    pub network_id: String,
    /// Base symbol identity advertised by the source.
    pub base_symbol_id: String,
    /// Quote unit returned by the source.
    pub quote: QuoteCode,
    /// Decimal-string unit price.
    pub unit_price_dec: String,
    /// Concrete source ref pinned to a block.
    pub source_ref: ObservationValueSourceRef,
}

/// Reads each unique direct valuation source once for the current pinned portfolio view.
#[derive(Clone, Debug)]
pub struct ReadDirectPricesState {
    /// Stable state identifier assigned by the execution plan.
    pub state_id: StateId,
    /// Symbol configs whose valuation routes should be resolved.
    pub symbols: Vec<SymbolConfig>,
    /// Valuation source registry used to resolve `source_id`.
    pub valuation_sources: ValuationSourceRegistry,
    /// Routed network configs used for EVM reads.
    pub networks: Vec<NetworkRouteConfig>,
    /// Context key that contains the pinned networks.
    pub network_pins_key: ContextKey,
    /// Context key that receives the resolved direct prices.
    pub output_key: ContextKey,
}

#[async_trait]
impl State for ReadDirectPricesState {
    fn meta(&self) -> StateMeta {
        meta::fetch_data()
    }

    async fn handle(
        &self,
        ctx: &mut dyn DynContext,
        io: &mut dyn IoProvider,
        _rec: &mut dyn EventRecorder,
    ) -> Result<StateOutcome, StateError> {
        let pins: Vec<PinnedNetwork> = read_typed(
            ctx,
            &self.network_pins_key,
            "missing_network_pins",
            "missing network pins in context",
            "network_pins_decode_failed",
            "failed to decode network pins",
        )?;
        let pins_by_id: HashMap<_, _> = pins
            .iter()
            .map(|pin| (pin.network_id.as_str(), pin))
            .collect();
        let routes_by_id: HashMap<_, _> = self
            .networks
            .iter()
            .map(|network| (network.network_id.as_str(), network))
            .collect();
        let sources_by_id: HashMap<_, _> = self
            .valuation_sources
            .sources
            .iter()
            .map(|source| (source.source_id.as_str(), source))
            .collect();

        let mut refs_by_id = BTreeMap::new();
        for symbol in &self.symbols {
            for quote in &symbol.valuation.quotes {
                match &quote.reader {
                    ValuationReaderConfig::FixedUnitPrice { .. } => {}
                    ValuationReaderConfig::DirectPrice { source } => {
                        refs_by_id
                            .entry(source.source_id.clone())
                            .or_insert(source.clone());
                    }
                    ValuationReaderConfig::DerivedUnitPrice {
                        numerator,
                        denominator,
                    } => {
                        refs_by_id
                            .entry(numerator.source_id.clone())
                            .or_insert(numerator.clone());
                        refs_by_id
                            .entry(denominator.source_id.clone())
                            .or_insert(denominator.clone());
                    }
                }
            }
        }

        let mut direct_prices = Vec::with_capacity(refs_by_id.len());
        for source_ref in refs_by_id.values() {
            let Some(source_cfg) = sources_by_id.get(source_ref.source_id.as_str()) else {
                return Err(state_unknown_msg(
                    "valuation_source_not_found",
                    format!("valuation source `{}` was not found", source_ref.source_id),
                ));
            };
            let Some(network) = routes_by_id.get(source_cfg.network_id.as_str()) else {
                return Err(state_unknown_msg(
                    "missing_network_route_config",
                    format!(
                        "network route config `{}` was not available",
                        source_cfg.network_id
                    ),
                ));
            };
            let Some(pin) = pins_by_id.get(source_cfg.network_id.as_str()) else {
                return Err(state_unknown_msg(
                    "missing_network_pin",
                    format!("network pin `{}` was not available", source_cfg.network_id),
                ));
            };

            let unit_price_dec = match &source_cfg.reader {
                ValuationSourceReaderConfig::EvmOracle {
                    oracle_kind,
                    config,
                } => {
                    mfm_evm_runtime::states::price::read_evm_oracle_unit_price(
                        &self.state_id,
                        io,
                        &network.network_id,
                        &network.control_scope,
                        oracle_kind,
                        config,
                        pin.block_number,
                    )
                    .await?
                    .unit_price_dec
                }
            };

            direct_prices.push(DirectPriceValue {
                source_id: source_cfg.source_id.clone(),
                network_id: source_cfg.network_id.clone(),
                base_symbol_id: source_cfg.base_symbol_id.clone(),
                quote: source_cfg.quote,
                unit_price_dec,
                source_ref: ObservationValueSourceRef {
                    source_id: source_cfg.source_id.clone(),
                    network_id: source_cfg.network_id.clone(),
                    block_number: pin.block_number,
                },
            });
        }

        direct_prices.sort_by(|left, right| left.source_id.cmp(&right.source_id));
        let value = serde_json::to_value(&direct_prices).map_err(|_| {
            state_unknown(
                "direct_prices_serialize_failed",
                "failed to serialize direct prices",
            )
        })?;
        write_json(ctx, self.output_key.clone(), value)?;

        Ok(StateOutcome {
            snapshot: SnapshotPolicy::OnSuccess,
        })
    }
}

/// Reads balances and normalizes canonical observations for the configured wallets and symbols.
#[derive(Clone, Debug)]
pub struct CollectObservationsState {
    /// Stable state identifier assigned by the execution plan.
    pub state_id: StateId,
    /// Wallet configs used to determine the symbol assignment per wallet.
    pub wallets: Vec<WalletConfig>,
    /// Symbol configs available to the portfolio.
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
impl State for CollectObservationsState {
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
        let direct_prices_by_id: HashMap<_, _> = direct_prices
            .iter()
            .map(|price| (price.source_id.as_str(), price))
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
                    build_observation(
                        &self.state_id,
                        io,
                        wallet,
                        symbol,
                        pin,
                        &route.network_id,
                        &route.control_scope,
                        &direct_prices_by_id,
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

/// Merges multiple observation vectors into one canonical, deterministically ordered output.
#[derive(Clone, Debug)]
pub struct MergeObservationsState {
    /// Stable state identifier assigned by the execution plan.
    pub state_id: StateId,
    /// Context keys that contain `Vec<Observation>` payloads.
    pub input_keys: Vec<ContextKey>,
    /// Context key that receives the merged observations.
    pub output_key: ContextKey,
}

#[async_trait]
impl State for MergeObservationsState {
    fn meta(&self) -> StateMeta {
        meta::pure()
    }

    async fn handle(
        &self,
        ctx: &mut dyn DynContext,
        _io: &mut dyn IoProvider,
        _rec: &mut dyn EventRecorder,
    ) -> Result<StateOutcome, StateError> {
        let mut observations = Vec::new();
        for input_key in &self.input_keys {
            let mut next: Vec<Observation> = read_typed(
                ctx,
                input_key,
                "missing_observations",
                "missing observations in context",
                "observations_decode_failed",
                "failed to decode observations",
            )?;
            observations.append(&mut next);
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

#[allow(clippy::too_many_arguments)]
async fn build_observation(
    state_id: &StateId,
    io: &mut dyn IoProvider,
    wallet: &ResolvedWallet,
    symbol: &SymbolConfig,
    pin: &PinnedNetwork,
    network_id: &str,
    control_scope: &str,
    direct_prices_by_id: &HashMap<&str, &DirectPriceValue>,
) -> Result<Observation, StateError> {
    let (raw_dec, decimals, amount_dec, balance_reader_kind) = match &symbol.balance_reader {
        BalanceReaderConfig::NativeBalance {} => {
            let raw = read_native_balance(
                state_id,
                io,
                network_id,
                control_scope,
                &wallet.address,
                pin.block_number,
            )
            .await?;
            let decimals = symbol.decimals.unwrap_or(18);
            (
                raw.to_string(),
                decimals,
                format_u256_units(&raw, decimals),
                "native_balance".to_string(),
            )
        }
        BalanceReaderConfig::Erc20Balance { token_address } => {
            let decimals = match symbol.decimals {
                Some(decimals) => decimals,
                None => {
                    read_token_decimals(
                        state_id,
                        io,
                        network_id,
                        control_scope,
                        token_address,
                        pin.block_number,
                    )
                    .await?
                }
            };
            let raw = read_erc20_balance(
                state_id,
                io,
                network_id,
                control_scope,
                token_address,
                &wallet.address,
                pin.block_number,
            )
            .await?;
            (
                raw.to_string(),
                decimals,
                format_u256_units(&raw, decimals),
                "erc20_balance".to_string(),
            )
        }
        BalanceReaderConfig::ProtocolPosition {
            protocol, reader, ..
        } => {
            return Err(state_unknown_msg(
                "unsupported_balance_reader_kind",
                format!("unsupported balance reader `{protocol}:{reader}` in base runtime slice"),
            ))
        }
    };

    let values = build_observation_values_from_lookup(symbol, &amount_dec, direct_prices_by_id)?;

    Ok(Observation {
        wallet_id: wallet.wallet_id.clone(),
        symbol_id: symbol.symbol_id.clone(),
        display_symbol: symbol.display_symbol.clone(),
        kind: symbol.kind,
        role: symbol.role,
        network_id: symbol.network_id.clone(),
        protocol: symbol.protocol.clone(),
        quantity: ObservationQuantity {
            raw_dec,
            decimals,
            amount_dec: amount_dec.to_string(),
        },
        values,
        source: ObservationSource {
            balance_reader_kind,
            network_id: pin.network_id.clone(),
            block_number: pin.block_number,
        },
        metadata: BTreeMap::new(),
    })
}

/// Builds canonical valuation outputs for one symbol amount using the already-resolved direct
/// prices available to the current pinned portfolio view.
pub fn build_observation_values(
    symbol: &SymbolConfig,
    amount_dec: &str,
    direct_prices: &[DirectPriceValue],
) -> Result<Vec<ObservationValue>, StateError> {
    let direct_prices_by_id: HashMap<_, _> = direct_prices
        .iter()
        .map(|price| (price.source_id.as_str(), price))
        .collect();
    build_observation_values_from_lookup(symbol, amount_dec, &direct_prices_by_id)
}

fn build_observation_values_from_lookup(
    symbol: &SymbolConfig,
    amount_dec: &str,
    direct_prices_by_id: &HashMap<&str, &DirectPriceValue>,
) -> Result<Vec<ObservationValue>, StateError> {
    let mut values = Vec::with_capacity(symbol.valuation.quotes.len());
    for quote in &symbol.valuation.quotes {
        let (unit_price_dec, valuation_reader_kind, source_refs) = match &quote.reader {
            ValuationReaderConfig::FixedUnitPrice { unit_price_dec } => (
                unit_price_dec.clone(),
                "fixed_unit_price".to_string(),
                Vec::new(),
            ),
            ValuationReaderConfig::DirectPrice { source } => {
                let price = direct_prices_by_id
                    .get(source.source_id.as_str())
                    .ok_or_else(|| {
                        state_unknown_msg(
                            "missing_direct_price",
                            format!("missing direct price for source `{}`", source.source_id),
                        )
                    })?;
                (
                    price.unit_price_dec.clone(),
                    "direct_price".to_string(),
                    vec![price.source_ref.clone()],
                )
            }
            ValuationReaderConfig::DerivedUnitPrice {
                numerator,
                denominator,
            } => {
                let numerator = direct_prices_by_id
                    .get(numerator.source_id.as_str())
                    .ok_or_else(|| {
                        state_unknown_msg(
                            "missing_direct_price",
                            format!("missing direct price for source `{}`", numerator.source_id),
                        )
                    })?;
                let denominator = direct_prices_by_id
                    .get(denominator.source_id.as_str())
                    .ok_or_else(|| {
                        state_unknown_msg(
                            "missing_direct_price",
                            format!(
                                "missing direct price for source `{}`",
                                denominator.source_id
                            ),
                        )
                    })?;
                (
                    divide_decimal_strings(&numerator.unit_price_dec, &denominator.unit_price_dec)?,
                    "derived_unit_price".to_string(),
                    vec![numerator.source_ref.clone(), denominator.source_ref.clone()],
                )
            }
        };

        values.push(ObservationValue {
            quote: quote.quote,
            priced_symbol_id: quote.priced_symbol_id.clone(),
            value_dec: multiply_decimal_strings(amount_dec, &unit_price_dec)?,
            unit_price_dec,
            valuation_reader_kind,
            source_refs,
        });
    }

    values.sort_by(|left, right| left.quote.cmp(&right.quote));
    Ok(values)
}

async fn read_native_balance(
    state_id: &StateId,
    io: &mut dyn IoProvider,
    network_id: &str,
    control_scope: &str,
    wallet_address: &str,
    block_number: u64,
) -> Result<U256, StateError> {
    let mut client = EvmIoClient::new(state_id.clone(), io);
    let call = JsonRpcCall::for_scope_and_network(
        control_scope,
        network_id,
        "eth_getBalance",
        serde_json::json!([wallet_address, u64_hex_quantity(block_number)]),
    );
    let res = client.call(call).await.map_err(state_from_io)?;
    parse_u256_hex_value(&res.response).map_err(|err| {
        state_unknown_msg(
            "evm_response_invalid",
            format!("native balance response was invalid: {}", err.message),
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
            {"to": token_address, "data": mfm_evm_runtime::states::read::encode_erc20_decimals()},
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

fn multiply_decimal_strings(left: &str, right: &str) -> Result<String, StateError> {
    let left = DecimalValue::parse(left)?;
    let right = DecimalValue::parse(right)?;
    let product = DecimalValue {
        digits: left.digits * right.digits,
        scale: left.scale + right.scale,
    };
    Ok(product.to_string_with_min_scale(left.scale.max(right.scale)))
}

fn divide_decimal_strings(numerator: &str, denominator: &str) -> Result<String, StateError> {
    let numerator = DecimalValue::parse(numerator)?;
    let denominator = DecimalValue::parse(denominator)?;
    if denominator.digits.is_zero() {
        return Err(state_unknown(
            "division_by_zero",
            "derived unit price denominator must be non-zero",
        ));
    }

    let min_scale = numerator.scale.max(denominator.scale).max(8);
    let working_scale = min_scale + 18;
    let scaled_numerator = numerator.digits * ten_pow(working_scale + denominator.scale);
    let scaled_denominator = denominator.digits * ten_pow(numerator.scale);
    let quotient = scaled_numerator / scaled_denominator;
    Ok(DecimalValue {
        digits: quotient,
        scale: working_scale,
    }
    .to_string_with_min_scale(min_scale))
}

fn ten_pow(n: u32) -> BigInt {
    BigInt::from(10u8).pow(n)
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
