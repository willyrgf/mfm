use std::collections::BTreeMap;

use alloy_primitives::{Address, U256};
use async_trait::async_trait;
use mfm_collectors_rpc_control::{EvmIoClient, PrepareSourcesResponse, DEFAULT_CONTROL_SCOPE};
use mfm_evm_core::encoding::{
    encode_erc20_balance_of, format_u256_units, parse_u256_hex_value, parse_u8_u256,
    u64_hex_quantity,
};
use mfm_evm_runtime::states::price::read_evm_oracle_unit_price;
use mfm_machine::errors::{ErrorCategory, StateError};
use mfm_machine::hashing::artifact_id_for_json;
use mfm_machine::ids::{FactKey, StateId};
use mfm_machine::io::{IoCall, IoProvider};
use mfm_portfolio_model::symbol::{
    EvmOracleConfig, Observation, ObservationAnchor, ObservationQuantity, ObservationSource,
    ObservationValue, ObservationValueSourceRef, ValuationSourceReaderConfig,
};
use mfm_portfolio_model::wallet::{WalletCapabilities, WalletImplementationConfig};
use mfm_state_common::decimal::{
    divide_decimal_strings as common_divide_decimal_strings,
    multiply_decimal_strings as common_multiply_decimal_strings, DecimalArithmeticError,
};
use mfm_state_common::errors::{
    state_error_with_state, state_from_io, state_unknown, state_unknown_msg,
};
use serde::Deserialize;
use serde_json::Value;

use mfm_portfolio_plan::{
    AdapterId, BitcoinResolvedSubjectValue, BitcoinRoutePolicy, BitcoinSubjectLocator,
    BitcoinUtxoSetObservationPayload, DerivedUnitPriceValuationPayload, DirectPriceSourcePayload,
    DirectPriceValuationPayload, DispatchObservationRuntimeAdapter, DispatchSubjectRuntimeAdapter,
    DispatchValuationRuntimeAdapter, DispatchViewRuntimeAdapter, EvmResolvedSubjectValue,
    EvmRoutePolicy, EvmSubjectLocator, ExecutionAnchor, FixedUnitPriceValuationPayload,
    NativeBalanceObservationPayload, ObservationRuntimeInput, PinnedNetworkView, ResolvedSubject,
    ResolvedUnitPrice, RuntimeAdapter, SubjectKind, SubjectResolutionTask, SubjectRuntimeInput,
    ValuationRuntimeInput, ValuationTask, ViewPinTask, ViewRuntimeInput,
};

const ADAPTER_RESOLVE_SUBJECT_EVM_ADDRESS: &str = "resolve_subject/evm_address";
const ADAPTER_RESOLVE_SUBJECT_BITCOIN_ADDRESS: &str = "resolve_subject/bitcoin_address";
const ADAPTER_PIN_VIEW_EVM: &str = "pin_view/evm";
const ADAPTER_PIN_VIEW_BITCOIN: &str = "pin_view/bitcoin";
const ADAPTER_RESOLVE_VALUATION_FIXED: &str = "resolve_valuation/fixed_unit_price";
const ADAPTER_RESOLVE_VALUATION_EVM_ORACLE: &str = "resolve_valuation/evm_oracle_direct_price";
const ADAPTER_RESOLVE_VALUATION_DERIVED: &str = "resolve_valuation/derived_unit_price";
const ADAPTER_OBSERVE_EVM_NATIVE_BALANCE: &str = "observe_position/evm/native_balance";
const ADAPTER_OBSERVE_EVM_ERC20_BALANCE: &str = "observe_position/evm/erc20_balance";
const ADAPTER_OBSERVE_BITCOIN_UTXO_SET: &str = "observe_position/bitcoin/utxo_set";

/// Runtime adapter for compiled EVM address subjects.
#[derive(Clone, Debug, Default)]
pub struct EvmAddressSubjectRuntimeAdapter;

impl RuntimeAdapter for EvmAddressSubjectRuntimeAdapter {
    fn id(&self) -> &AdapterId {
        static ID: std::sync::OnceLock<AdapterId> = std::sync::OnceLock::new();
        ID.get_or_init(|| AdapterId(ADAPTER_RESOLVE_SUBJECT_EVM_ADDRESS.to_string()))
    }
}

#[async_trait]
impl DispatchSubjectRuntimeAdapter for EvmAddressSubjectRuntimeAdapter {
    async fn resolve_subject(
        &self,
        state_id: &StateId,
        _io: &mut dyn IoProvider,
        task: &SubjectResolutionTask,
        _input: SubjectRuntimeInput<'_>,
    ) -> Result<ResolvedSubject, StateError> {
        let locator: EvmSubjectLocator =
            decode_object_map("subject_resolution_task", &task.task_id, &task.payload)?;
        let (implementation_kind, capabilities) = match locator.implementation {
            WalletImplementationConfig::AddressOnly {} => (
                "address_only".to_string(),
                WalletCapabilities {
                    can_resolve_address: true,
                    can_sign: false,
                    can_submit: false,
                },
            ),
            WalletImplementationConfig::KeystoreEntry { .. } => {
                return Err(unsupported_wallet_impl(state_id, "keystore_entry"));
            }
            WalletImplementationConfig::NodeManagedAccount { .. } => {
                return Err(unsupported_wallet_impl(state_id, "node_managed_account"));
            }
            WalletImplementationConfig::ExternalSigner { .. } => {
                return Err(unsupported_wallet_impl(state_id, "external_signer"));
            }
        };

        Ok(ResolvedSubject {
            subject_id: task.subject_id.clone(),
            kind: SubjectKind::EvmAddress,
            value: serde_json::to_value(EvmResolvedSubjectValue {
                network_id: locator.network_id,
                address: locator.address,
                implementation_kind,
                capabilities,
            })
            .map_err(|_| {
                state_unknown(
                    "resolved_subject_serialize_failed",
                    "failed to serialize resolved subject",
                )
            })?,
        })
    }
}

/// Runtime adapter for compiled Bitcoin address subjects.
#[derive(Clone, Debug, Default)]
pub struct BitcoinAddressSubjectRuntimeAdapter;

impl RuntimeAdapter for BitcoinAddressSubjectRuntimeAdapter {
    fn id(&self) -> &AdapterId {
        static ID: std::sync::OnceLock<AdapterId> = std::sync::OnceLock::new();
        ID.get_or_init(|| AdapterId(ADAPTER_RESOLVE_SUBJECT_BITCOIN_ADDRESS.to_string()))
    }
}

#[async_trait]
impl DispatchSubjectRuntimeAdapter for BitcoinAddressSubjectRuntimeAdapter {
    async fn resolve_subject(
        &self,
        state_id: &StateId,
        _io: &mut dyn IoProvider,
        task: &SubjectResolutionTask,
        _input: SubjectRuntimeInput<'_>,
    ) -> Result<ResolvedSubject, StateError> {
        let locator: BitcoinSubjectLocator =
            decode_object_map("subject_resolution_task", &task.task_id, &task.payload)?;
        let (implementation_kind, capabilities) = match locator.implementation {
            WalletImplementationConfig::AddressOnly {} => (
                "address_only".to_string(),
                WalletCapabilities {
                    can_resolve_address: true,
                    can_sign: false,
                    can_submit: false,
                },
            ),
            WalletImplementationConfig::KeystoreEntry { .. } => {
                return Err(unsupported_wallet_impl(state_id, "keystore_entry"));
            }
            WalletImplementationConfig::NodeManagedAccount { .. } => {
                return Err(unsupported_wallet_impl(state_id, "node_managed_account"));
            }
            WalletImplementationConfig::ExternalSigner { .. } => {
                return Err(unsupported_wallet_impl(state_id, "external_signer"));
            }
        };

        Ok(ResolvedSubject {
            subject_id: task.subject_id.clone(),
            kind: SubjectKind::BitcoinAddress,
            value: serde_json::to_value(BitcoinResolvedSubjectValue {
                network_id: locator.network_id,
                address: locator.address,
                implementation_kind,
                capabilities,
            })
            .map_err(|_| {
                state_unknown(
                    "resolved_subject_serialize_failed",
                    "failed to serialize resolved subject",
                )
            })?,
        })
    }
}

/// Runtime adapter for compiled EVM execution views.
#[derive(Clone, Debug, Default)]
pub struct EvmViewRuntimeAdapter;

impl RuntimeAdapter for EvmViewRuntimeAdapter {
    fn id(&self) -> &AdapterId {
        static ID: std::sync::OnceLock<AdapterId> = std::sync::OnceLock::new();
        ID.get_or_init(|| AdapterId(ADAPTER_PIN_VIEW_EVM.to_string()))
    }
}

#[async_trait]
impl DispatchViewRuntimeAdapter for EvmViewRuntimeAdapter {
    async fn pin_view(
        &self,
        state_id: &StateId,
        io: &mut dyn IoProvider,
        task: &ViewPinTask,
        input: ViewRuntimeInput<'_>,
    ) -> Result<PinnedNetworkView, StateError> {
        let route_policy: EvmRoutePolicy =
            decode_object_map("view_pin_task", &task.task_id, &task.payload)?;
        let prepared = input
            .prepared_sources
            .get(task.network_view_id.as_str())
            .ok_or_else(|| {
                state_unknown_msg(
                    "missing_prepared_source",
                    format!(
                        "prepared source payload `{}` was not available for view pinning",
                        task.network_view_id
                    ),
                )
            })?;
        let prepared: PrepareSourcesResponse =
            serde_json::from_value(prepared.clone()).map_err(|err| {
                state_unknown_msg(
                    "prepared_source_decode_failed",
                    format!(
                        "prepared source payload `{}` decode failed: {err}",
                        task.network_view_id
                    ),
                )
            })?;

        let control_scope = normalized_control_scope(route_policy.control_scope.as_str());
        if prepared.network_id != route_policy.network_id {
            return Err(state_unknown_msg(
                "prepared_source_network_mismatch",
                format!(
                    "prepared source network `{}` did not match route policy network `{}`",
                    prepared.network_id, route_policy.network_id
                ),
            ));
        }
        if prepared.control_scope != control_scope {
            return Err(state_unknown_msg(
                "prepared_source_scope_mismatch",
                format!(
                    "prepared source scope `{}` did not match route policy scope `{}`",
                    prepared.control_scope, control_scope
                ),
            ));
        }

        let mut client = EvmIoClient::new(state_id.clone(), io);
        let chain_id = client
            .chain_id_u64(route_policy.network_id.as_str(), control_scope)
            .await
            .map_err(state_from_io)?;
        if chain_id != route_policy.chain_id {
            return Err(state_error_with_state(
                state_id.clone(),
                "network_chain_id_mismatch",
                ErrorCategory::ParsingInput,
                false,
                format!(
                    "rpc chain_id for network `{}` did not match configured chain_id",
                    route_policy.network_id
                ),
            ));
        }

        let block_number = client
            .block_number_u64(route_policy.network_id.as_str(), control_scope)
            .await
            .map_err(state_from_io)?;

        Ok(PinnedNetworkView {
            network_view_id: task.network_view_id.clone(),
            network_id: route_policy.network_id,
            family: task.family.clone(),
            anchor: ExecutionAnchor::Evm {
                chain_id,
                block_number,
            },
        })
    }
}

/// Runtime adapter for compiled Bitcoin execution views.
#[derive(Clone, Debug, Default)]
pub struct BitcoinViewRuntimeAdapter;

impl RuntimeAdapter for BitcoinViewRuntimeAdapter {
    fn id(&self) -> &AdapterId {
        static ID: std::sync::OnceLock<AdapterId> = std::sync::OnceLock::new();
        ID.get_or_init(|| AdapterId(ADAPTER_PIN_VIEW_BITCOIN.to_string()))
    }
}

#[derive(Clone, Debug, Deserialize)]
struct BitcoinAnchorResponse {
    height: u64,
    block_hash: String,
}

#[async_trait]
impl DispatchViewRuntimeAdapter for BitcoinViewRuntimeAdapter {
    async fn pin_view(
        &self,
        state_id: &StateId,
        io: &mut dyn IoProvider,
        task: &ViewPinTask,
        input: ViewRuntimeInput<'_>,
    ) -> Result<PinnedNetworkView, StateError> {
        let route_policy: BitcoinRoutePolicy =
            decode_object_map("view_pin_task", &task.task_id, &task.payload)?;
        let prepared = input
            .prepared_sources
            .get(task.network_view_id.as_str())
            .ok_or_else(|| {
                state_unknown_msg(
                    "missing_prepared_source",
                    format!(
                        "prepared source payload `{}` was not available for view pinning",
                        task.network_view_id
                    ),
                )
            })?;
        let prepared: PrepareSourcesResponse =
            serde_json::from_value(prepared.clone()).map_err(|err| {
                state_unknown_msg(
                    "prepared_source_decode_failed",
                    format!(
                        "prepared source payload `{}` decode failed: {err}",
                        task.network_view_id
                    ),
                )
            })?;
        let control_scope = normalized_control_scope(route_policy.control_scope.as_str());
        if prepared.network_id != route_policy.network_id {
            return Err(state_unknown_msg(
                "prepared_source_network_mismatch",
                format!(
                    "prepared source network `{}` did not match route policy network `{}`",
                    prepared.network_id, route_policy.network_id
                ),
            ));
        }
        if prepared.control_scope != control_scope {
            return Err(state_unknown_msg(
                "prepared_source_scope_mismatch",
                format!(
                    "prepared source scope `{}` did not match route policy scope `{}`",
                    prepared.control_scope, control_scope
                ),
            ));
        }

        let anchor: BitcoinAnchorResponse = serde_json::from_value(
            call_rpc_control_raw(
                state_id,
                io,
                serde_json::json!({
                    "kind": "bitcoin_anchor",
                    "control_scope": control_scope,
                    "network_id": route_policy.network_id,
                }),
            )
            .await?,
        )
        .map_err(|err| {
            state_unknown_msg(
                "bitcoin_anchor_decode_failed",
                format!("bitcoin anchor payload decode failed: {err}"),
            )
        })?;

        Ok(PinnedNetworkView {
            network_view_id: task.network_view_id.clone(),
            network_id: route_policy.network_id,
            family: task.family.clone(),
            anchor: ExecutionAnchor::Bitcoin {
                height: anchor.height,
                block_hash: anchor.block_hash,
            },
        })
    }
}

/// Runtime adapter for fixed unit prices.
#[derive(Clone, Debug, Default)]
pub struct FixedUnitPriceRuntimeAdapter;

impl RuntimeAdapter for FixedUnitPriceRuntimeAdapter {
    fn id(&self) -> &AdapterId {
        static ID: std::sync::OnceLock<AdapterId> = std::sync::OnceLock::new();
        ID.get_or_init(|| AdapterId(ADAPTER_RESOLVE_VALUATION_FIXED.to_string()))
    }
}

#[async_trait]
impl DispatchValuationRuntimeAdapter for FixedUnitPriceRuntimeAdapter {
    async fn resolve(
        &self,
        _state_id: &StateId,
        _io: &mut dyn IoProvider,
        task: &ValuationTask,
        _input: ValuationRuntimeInput<'_>,
    ) -> Result<ResolvedUnitPrice, StateError> {
        let payload: FixedUnitPriceValuationPayload =
            decode_object_map("valuation_task", &task.valuation_id, &task.payload)?;
        Ok(ResolvedUnitPrice {
            valuation_id: task.valuation_id.clone(),
            instrument_id: task.instrument_id.clone(),
            priced_symbol_id: payload.priced_symbol_id,
            quote: task.quote,
            unit_price_dec: payload.unit_price_dec,
            valuation_reader_kind: "fixed_unit_price".to_string(),
            source_refs: Vec::new(),
        })
    }
}

/// Runtime adapter for direct EVM oracle prices.
#[derive(Clone, Debug, Default)]
pub struct EvmOracleDirectPriceRuntimeAdapter;

impl RuntimeAdapter for EvmOracleDirectPriceRuntimeAdapter {
    fn id(&self) -> &AdapterId {
        static ID: std::sync::OnceLock<AdapterId> = std::sync::OnceLock::new();
        ID.get_or_init(|| AdapterId(ADAPTER_RESOLVE_VALUATION_EVM_ORACLE.to_string()))
    }
}

#[async_trait]
impl DispatchValuationRuntimeAdapter for EvmOracleDirectPriceRuntimeAdapter {
    async fn resolve(
        &self,
        state_id: &StateId,
        io: &mut dyn IoProvider,
        task: &ValuationTask,
        input: ValuationRuntimeInput<'_>,
    ) -> Result<ResolvedUnitPrice, StateError> {
        let payload: DirectPriceValuationPayload =
            decode_object_map("valuation_task", &task.valuation_id, &task.payload)?;
        let (unit_price_dec, source_ref) =
            resolve_direct_source(state_id, io, &payload.source, input.pinned_views).await?;
        Ok(ResolvedUnitPrice {
            valuation_id: task.valuation_id.clone(),
            instrument_id: task.instrument_id.clone(),
            priced_symbol_id: payload.priced_symbol_id,
            quote: task.quote,
            unit_price_dec,
            valuation_reader_kind: "direct_price".to_string(),
            source_refs: vec![source_ref],
        })
    }
}

/// Runtime adapter for derived prices.
#[derive(Clone, Debug, Default)]
pub struct DerivedUnitPriceRuntimeAdapter;

impl RuntimeAdapter for DerivedUnitPriceRuntimeAdapter {
    fn id(&self) -> &AdapterId {
        static ID: std::sync::OnceLock<AdapterId> = std::sync::OnceLock::new();
        ID.get_or_init(|| AdapterId(ADAPTER_RESOLVE_VALUATION_DERIVED.to_string()))
    }
}

#[async_trait]
impl DispatchValuationRuntimeAdapter for DerivedUnitPriceRuntimeAdapter {
    async fn resolve(
        &self,
        state_id: &StateId,
        io: &mut dyn IoProvider,
        task: &ValuationTask,
        input: ValuationRuntimeInput<'_>,
    ) -> Result<ResolvedUnitPrice, StateError> {
        let payload: DerivedUnitPriceValuationPayload =
            decode_object_map("valuation_task", &task.valuation_id, &task.payload)?;
        let (numerator, numerator_ref) =
            resolve_direct_source(state_id, io, &payload.numerator, input.pinned_views).await?;
        let (denominator, denominator_ref) =
            resolve_direct_source(state_id, io, &payload.denominator, input.pinned_views).await?;
        Ok(ResolvedUnitPrice {
            valuation_id: task.valuation_id.clone(),
            instrument_id: task.instrument_id.clone(),
            priced_symbol_id: payload.priced_symbol_id,
            quote: task.quote,
            unit_price_dec: divide_decimal_strings(numerator.as_str(), denominator.as_str())?,
            valuation_reader_kind: "derived_unit_price".to_string(),
            source_refs: vec![numerator_ref, denominator_ref],
        })
    }
}

/// Runtime adapter for compiled EVM native-balance observations.
#[derive(Clone, Debug, Default)]
pub struct EvmNativeBalanceObservationRuntimeAdapter;

impl RuntimeAdapter for EvmNativeBalanceObservationRuntimeAdapter {
    fn id(&self) -> &AdapterId {
        static ID: std::sync::OnceLock<AdapterId> = std::sync::OnceLock::new();
        ID.get_or_init(|| AdapterId(ADAPTER_OBSERVE_EVM_NATIVE_BALANCE.to_string()))
    }
}

#[async_trait]
impl DispatchObservationRuntimeAdapter for EvmNativeBalanceObservationRuntimeAdapter {
    async fn observe(
        &self,
        state_id: &StateId,
        io: &mut dyn IoProvider,
        binding: &mfm_portfolio_plan::CompiledObservationBinding,
        input: ObservationRuntimeInput<'_>,
    ) -> Result<Observation, StateError> {
        let payload: NativeBalanceObservationPayload = decode_object_map(
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
        let raw = read_native_balance(
            state_id,
            io,
            payload.route_policy.network_id.as_str(),
            normalized_control_scope(payload.route_policy.control_scope.as_str()),
            subject.address.as_str(),
            block_number,
        )
        .await?;
        let decimals = payload.projection.decimals.unwrap_or(18);
        let amount_dec = format_u256_units(&raw, decimals);
        Ok(Observation {
            wallet_id: binding.observation_key.subject_id.clone(),
            symbol_id: payload.projection.symbol_id,
            display_symbol: payload.projection.display_symbol,
            kind: payload.projection.kind,
            role: payload.projection.role,
            network_id: payload.projection.network_id,
            protocol: payload.projection.protocol,
            quantity: ObservationQuantity {
                raw_dec: raw.to_string(),
                decimals,
                amount_dec: amount_dec.clone(),
            },
            values: build_observation_values_from_resolved(binding, &amount_dec, input)?,
            source: ObservationSource {
                balance_reader_kind: "native_balance".to_string(),
                network_id: pinned.network_id.clone(),
                anchor: ObservationAnchor::Evm {
                    chain_id: payload.route_policy.chain_id,
                    block_number,
                },
            },
            metadata: BTreeMap::new(),
        })
    }
}

/// Runtime adapter for compiled EVM ERC-20 balance observations.
#[derive(Clone, Debug, Default)]
pub struct EvmErc20BalanceObservationRuntimeAdapter;

impl RuntimeAdapter for EvmErc20BalanceObservationRuntimeAdapter {
    fn id(&self) -> &AdapterId {
        static ID: std::sync::OnceLock<AdapterId> = std::sync::OnceLock::new();
        ID.get_or_init(|| AdapterId(ADAPTER_OBSERVE_EVM_ERC20_BALANCE.to_string()))
    }
}

#[async_trait]
impl DispatchObservationRuntimeAdapter for EvmErc20BalanceObservationRuntimeAdapter {
    async fn observe(
        &self,
        state_id: &StateId,
        io: &mut dyn IoProvider,
        binding: &mfm_portfolio_plan::CompiledObservationBinding,
        input: ObservationRuntimeInput<'_>,
    ) -> Result<Observation, StateError> {
        let payload: mfm_portfolio_plan::Erc20BalanceObservationPayload = decode_object_map(
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
        let decimals = match payload.projection.decimals {
            Some(decimals) => decimals,
            None => {
                read_token_decimals(
                    state_id,
                    io,
                    payload.route_policy.network_id.as_str(),
                    normalized_control_scope(payload.route_policy.control_scope.as_str()),
                    payload.token_address.as_str(),
                    block_number,
                )
                .await?
            }
        };
        let raw = read_erc20_balance(
            state_id,
            io,
            payload.route_policy.network_id.as_str(),
            normalized_control_scope(payload.route_policy.control_scope.as_str()),
            payload.token_address.as_str(),
            subject.address.as_str(),
            block_number,
        )
        .await?;
        let amount_dec = format_u256_units(&raw, decimals);
        Ok(Observation {
            wallet_id: binding.observation_key.subject_id.clone(),
            symbol_id: payload.projection.symbol_id,
            display_symbol: payload.projection.display_symbol,
            kind: payload.projection.kind,
            role: payload.projection.role,
            network_id: payload.projection.network_id,
            protocol: payload.projection.protocol,
            quantity: ObservationQuantity {
                raw_dec: raw.to_string(),
                decimals,
                amount_dec: amount_dec.clone(),
            },
            values: build_observation_values_from_resolved(binding, &amount_dec, input)?,
            source: ObservationSource {
                balance_reader_kind: "erc20_balance".to_string(),
                network_id: pinned.network_id.clone(),
                anchor: ObservationAnchor::Evm {
                    chain_id: payload.route_policy.chain_id,
                    block_number,
                },
            },
            metadata: BTreeMap::new(),
        })
    }
}

/// Runtime adapter for compiled Bitcoin UTXO-set observations.
#[derive(Clone, Debug, Default)]
pub struct BitcoinUtxoSetObservationRuntimeAdapter;

impl RuntimeAdapter for BitcoinUtxoSetObservationRuntimeAdapter {
    fn id(&self) -> &AdapterId {
        static ID: std::sync::OnceLock<AdapterId> = std::sync::OnceLock::new();
        ID.get_or_init(|| AdapterId(ADAPTER_OBSERVE_BITCOIN_UTXO_SET.to_string()))
    }
}

#[derive(Clone, Debug, Deserialize)]
struct BitcoinUtxoBalanceResponse {
    amount_sats: String,
}

#[async_trait]
impl DispatchObservationRuntimeAdapter for BitcoinUtxoSetObservationRuntimeAdapter {
    async fn observe(
        &self,
        state_id: &StateId,
        io: &mut dyn IoProvider,
        binding: &mfm_portfolio_plan::CompiledObservationBinding,
        input: ObservationRuntimeInput<'_>,
    ) -> Result<Observation, StateError> {
        let payload: BitcoinUtxoSetObservationPayload = decode_object_map(
            "compiled_observation_binding",
            &binding.binding_id,
            &binding.payload,
        )?;
        let subject = resolved_bitcoin_subject_for_binding(binding, input.resolved_subjects)?;
        let pinned =
            pinned_bitcoin_view_for_binding(binding, input.pinned_views, &payload.route_policy)?;
        let (height, block_hash) = match &pinned.anchor {
            ExecutionAnchor::Bitcoin { height, block_hash } => (*height, block_hash.clone()),
            ExecutionAnchor::Evm { .. } => {
                unreachable!("validated by pinned_bitcoin_view_for_binding")
            }
        };
        let response: BitcoinUtxoBalanceResponse = serde_json::from_value(
            call_rpc_control_raw(
                state_id,
                io,
                serde_json::json!({
                    "kind": "bitcoin_scan_utxos",
                    "control_scope": normalized_control_scope(payload.route_policy.control_scope.as_str()),
                    "network_id": payload.route_policy.network_id,
                    "address": subject.address,
                    "height": height,
                    "block_hash": block_hash,
                }),
            )
            .await?,
        )
        .map_err(|err| {
            state_unknown_msg(
                "bitcoin_utxo_balance_decode_failed",
                format!("bitcoin utxo balance payload decode failed: {err}"),
            )
        })?;
        let raw = response.amount_sats.parse::<u64>().map_err(|err| {
            state_unknown_msg(
                "bitcoin_utxo_balance_invalid_amount",
                format!("bitcoin utxo balance amount_sats was not a u64: {err}"),
            )
        })?;
        let decimals = payload.projection.decimals.unwrap_or(8);
        let amount_dec = format_u256_units(&U256::from(raw), decimals);
        Ok(Observation {
            wallet_id: binding.observation_key.subject_id.clone(),
            symbol_id: payload.projection.symbol_id,
            display_symbol: payload.projection.display_symbol,
            kind: payload.projection.kind,
            role: payload.projection.role,
            network_id: payload.projection.network_id,
            protocol: payload.projection.protocol,
            quantity: ObservationQuantity {
                raw_dec: raw.to_string(),
                decimals,
                amount_dec: amount_dec.clone(),
            },
            values: build_observation_values_from_resolved(binding, &amount_dec, input)?,
            source: ObservationSource {
                balance_reader_kind: "bitcoin_utxo_set".to_string(),
                network_id: pinned.network_id.clone(),
                anchor: ObservationAnchor::Bitcoin { height, block_hash },
            },
            metadata: BTreeMap::new(),
        })
    }
}

async fn resolve_direct_source(
    state_id: &StateId,
    io: &mut dyn IoProvider,
    payload: &DirectPriceSourcePayload,
    pinned_views: &BTreeMap<String, PinnedNetworkView>,
) -> Result<(String, ObservationValueSourceRef), StateError> {
    if payload.source.network_id != payload.route_policy.network_id {
        return Err(state_unknown_msg(
            "valuation_route_network_mismatch",
            format!(
                "valuation source network `{}` did not match route policy network `{}`",
                payload.source.network_id, payload.route_policy.network_id
            ),
        ));
    }
    let pinned = pinned_evm_view_for_network(
        pinned_views,
        payload.route_policy.network_id.as_str(),
        payload.route_policy.chain_id,
    )?;
    let block_number = match pinned.anchor {
        ExecutionAnchor::Evm { block_number, .. } => block_number,
        ExecutionAnchor::Bitcoin { .. } => unreachable!("validated by pinned_evm_view_for_network"),
    };

    match &payload.source_reader {
        ValuationSourceReaderConfig::EvmOracle {
            oracle_kind,
            config,
        } => {
            let price = read_evm_oracle_unit_price(
                state_id,
                io,
                payload.route_policy.network_id.as_str(),
                normalized_control_scope(payload.route_policy.control_scope.as_str()),
                oracle_kind,
                &evm_oracle_config_map(config),
                block_number,
            )
            .await?;
            Ok((
                price.unit_price_dec,
                ObservationValueSourceRef {
                    source_id: payload.source.source_id.clone(),
                    network_id: payload.source.network_id.clone(),
                    anchor: ObservationAnchor::Evm {
                        chain_id: payload.route_policy.chain_id,
                        block_number,
                    },
                },
            ))
        }
    }
}

fn evm_oracle_config_map(config: &EvmOracleConfig) -> BTreeMap<String, Value> {
    BTreeMap::from([(
        "contract_address".to_string(),
        Value::String(config.contract_address.clone()),
    )])
}

fn resolved_evm_subject_for_binding(
    binding: &mfm_portfolio_plan::CompiledObservationBinding,
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
    if subject.kind != SubjectKind::EvmAddress {
        return Err(state_unknown_msg(
            "observation_subject_kind_mismatch",
            format!(
                "observation binding `{}` required an EVM subject but got `{:?}`",
                binding.binding_id, subject.kind
            ),
        ));
    }
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

fn resolved_bitcoin_subject_for_binding(
    binding: &mfm_portfolio_plan::CompiledObservationBinding,
    resolved_subjects: &BTreeMap<String, ResolvedSubject>,
) -> Result<BitcoinResolvedSubjectValue, StateError> {
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
    if subject.kind != SubjectKind::BitcoinAddress {
        return Err(state_unknown_msg(
            "observation_subject_kind_mismatch",
            format!(
                "observation binding `{}` required a bitcoin address subject but got `{:?}`",
                binding.binding_id, subject.kind
            ),
        ));
    }
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
    binding: &mfm_portfolio_plan::CompiledObservationBinding,
    pinned_views: &'a BTreeMap<String, PinnedNetworkView>,
    route_policy: &EvmRoutePolicy,
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

fn pinned_bitcoin_view_for_binding<'a>(
    binding: &mfm_portfolio_plan::CompiledObservationBinding,
    pinned_views: &'a BTreeMap<String, PinnedNetworkView>,
    route_policy: &BitcoinRoutePolicy,
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
        ExecutionAnchor::Bitcoin { .. } => Ok(pinned),
        ExecutionAnchor::Evm { .. } => Err(state_unknown_msg(
            "pinned_view_family_mismatch",
            format!(
                "observation binding `{}` expected a bitcoin execution view",
                binding.binding_id
            ),
        )),
    }
}

fn build_observation_values_from_resolved(
    binding: &mfm_portfolio_plan::CompiledObservationBinding,
    amount_dec: &str,
    input: ObservationRuntimeInput<'_>,
) -> Result<Vec<ObservationValue>, StateError> {
    let mut seen_quotes = BTreeMap::new();
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
        if value.instrument_id != binding.observation_key.instrument_id {
            return Err(state_unknown_msg(
                "observation_valuation_instrument_mismatch",
                format!(
                    "resolved valuation `{}` targeted instrument `{}` but observation binding `{}` targeted `{}`",
                    value.valuation_id,
                    value.instrument_id,
                    binding.binding_id,
                    binding.observation_key.instrument_id,
                ),
            ));
        }
        if seen_quotes
            .insert(value.quote, value.valuation_id.clone())
            .is_some()
        {
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

fn pinned_evm_view_for_network<'a>(
    pinned_views: &'a BTreeMap<String, PinnedNetworkView>,
    network_id: &str,
    expected_chain_id: u64,
) -> Result<&'a PinnedNetworkView, StateError> {
    let matches = pinned_views
        .values()
        .filter(|view| view.network_id == network_id)
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [] => Err(state_unknown_msg(
            "missing_pinned_view_for_network",
            format!("missing pinned execution view for network `{network_id}`"),
        )),
        [view] => match &view.anchor {
            ExecutionAnchor::Evm { chain_id, .. } => {
                if *chain_id != expected_chain_id {
                    Err(state_unknown_msg(
                        "pinned_view_chain_id_mismatch",
                        format!(
                            "pinned execution view chain_id `{chain_id}` did not match expected chain_id `{expected_chain_id}` for network `{network_id}`",
                        ),
                    ))
                } else {
                    Ok(*view)
                }
            }
            ExecutionAnchor::Bitcoin { .. } => Err(state_unknown_msg(
                "pinned_view_family_mismatch",
                format!("pinned execution view for network `{network_id}` was not EVM"),
            )),
        },
        _ => Err(state_unknown_msg(
            "ambiguous_pinned_view_for_network",
            format!("multiple pinned execution views matched network `{network_id}`"),
        )),
    }
}

fn normalized_control_scope(control_scope: &str) -> &str {
    if control_scope.trim().is_empty() {
        DEFAULT_CONTROL_SCOPE
    } else {
        control_scope
    }
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

fn unsupported_wallet_impl(state_id: &StateId, kind: &'static str) -> StateError {
    state_error_with_state(
        state_id.clone(),
        "unsupported_wallet_implementation_kind",
        ErrorCategory::Unknown,
        false,
        format!("wallet implementation kind `{kind}` is not supported in the semantic runtime"),
    )
}

async fn call_rpc_control_raw(
    state_id: &StateId,
    io: &mut dyn IoProvider,
    request: Value,
) -> Result<Value, StateError> {
    let request_id = artifact_id_for_json(&request).map_err(|err| {
        state_unknown_msg(
            "semantic_runtime_request_not_canonical",
            format!("semantic runtime request was not canonical json: {err}"),
        )
    })?;
    io.call(IoCall {
        namespace: "rpc.control".to_string(),
        request,
        fact_key: Some(FactKey(format!(
            "mfm:rpc.control|state:{}|req:{}",
            state_id.as_str(),
            request_id.as_str()
        ))),
    })
    .await
    .map_err(state_from_io)
    .map(|result| result.response)
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
    let call = mfm_collectors_rpc_control::JsonRpcCall::for_scope_and_network(
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
    let call = mfm_collectors_rpc_control::JsonRpcCall::for_scope_and_network(
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
    let call = mfm_collectors_rpc_control::JsonRpcCall::for_scope_and_network(
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
    common_multiply_decimal_strings(left, right).map_err(decimal_string_error)
}

fn divide_decimal_strings(numerator: &str, denominator: &str) -> Result<String, StateError> {
    common_divide_decimal_strings(numerator, denominator).map_err(|err| match err {
        DecimalArithmeticError::InvalidDecimalString { value } => state_unknown_msg(
            "invalid_decimal_string",
            format!("invalid decimal string `{value}`"),
        ),
        DecimalArithmeticError::DivisionByZero => state_unknown(
            "division_by_zero",
            "derived unit price denominator must be non-zero",
        ),
    })
}

fn decimal_string_error(err: DecimalArithmeticError) -> StateError {
    match err {
        DecimalArithmeticError::InvalidDecimalString { value } => state_unknown_msg(
            "invalid_decimal_string",
            format!("invalid decimal string `{value}`"),
        ),
        DecimalArithmeticError::DivisionByZero => state_unknown(
            "division_by_zero",
            "decimal operation failed because the denominator was zero",
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use async_trait::async_trait;
    use mfm_machine::errors::{ErrorInfo, IoError};
    use mfm_machine::ids::{ArtifactId, ErrorCode, FactKey};
    use mfm_machine::io::{IoCall, IoResult};
    use serde_json::json;

    struct TestIo {
        responses: BTreeMap<String, Value>,
    }

    #[async_trait]
    impl IoProvider for TestIo {
        async fn call(&mut self, call: IoCall) -> Result<IoResult, IoError> {
            let method = call
                .request
                .get("method")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let kind = call
                .request
                .get("kind")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let key = if kind == "prepare_sources" {
                format!(
                    "prepare:{}:{}",
                    call.request
                        .get("control_scope")
                        .and_then(Value::as_str)
                        .unwrap_or_default(),
                    call.request
                        .get("network_id")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                )
            } else if method == "eth_call" {
                call.request
                    .get("params")
                    .and_then(Value::as_array)
                    .and_then(|params| params.first())
                    .and_then(Value::as_object)
                    .and_then(|target| target.get("data"))
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string()
            } else {
                method.to_string()
            };
            let response = self.responses.get(&key).cloned().ok_or_else(|| {
                IoError::Other(ErrorInfo {
                    code: ErrorCode::must_new("missing_mock_response"),
                    category: ErrorCategory::Unknown,
                    retryable: false,
                    message: format!("missing mock response for `{key}`"),
                    details: None,
                })
            })?;
            Ok(IoResult {
                response,
                recorded_payload_id: None,
            })
        }

        async fn record_value(
            &mut self,
            _key: FactKey,
            _value: serde_json::Value,
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
    async fn subject_runtime_resolves_address_only_locator() {
        let adapter = EvmAddressSubjectRuntimeAdapter;
        let mut io = TestIo {
            responses: BTreeMap::new(),
        };
        let subject = adapter
            .resolve_subject(
                &StateId::must_new("portfolio.main.resolve".to_string()),
                &mut io,
                &SubjectResolutionTask {
                    task_id: "resolve.wallet_main".to_string(),
                    subject_id: "wallet_main".to_string(),
                    kind: SubjectKind::EvmAddress,
                    adapter: AdapterId(ADAPTER_RESOLVE_SUBJECT_EVM_ADDRESS.to_string()),
                    payload: BTreeMap::from([
                        ("network_id".to_string(), json!("ethereum-mainnet")),
                        (
                            "address".to_string(),
                            json!("0x000000000000000000000000000000000000dead"),
                        ),
                        (
                            "implementation".to_string(),
                            json!({ "kind": "address_only" }),
                        ),
                    ]),
                },
                SubjectRuntimeInput {
                    prepared_sources: &BTreeMap::new(),
                },
            )
            .await
            .expect("resolved subject");

        let value: EvmResolvedSubjectValue = serde_json::from_value(subject.value).expect("typed");
        assert_eq!(value.network_id, "ethereum-mainnet");
        assert_eq!(value.address, "0x000000000000000000000000000000000000dead");
    }

    #[tokio::test]
    async fn view_runtime_pins_evm_chain_and_block() {
        let adapter = EvmViewRuntimeAdapter;
        let mut io = TestIo {
            responses: BTreeMap::from([
                ("eth_chainId".to_string(), json!("0x1")),
                ("eth_blockNumber".to_string(), json!("0x64")),
            ]),
        };
        let prepared_sources = BTreeMap::from([(
            "ethereum-mainnet".to_string(),
            serde_json::to_value(PrepareSourcesResponse {
                control_scope: "rpc.mainnet".to_string(),
                network_id: "ethereum-mainnet".to_string(),
                pool_kind: "test".to_string(),
                available_source_ids: vec!["source-1".to_string()],
                ranked_source_ids: vec!["source-1".to_string()],
                sources: vec![],
            })
            .expect("prepared"),
        )]);

        let view = adapter
            .pin_view(
                &StateId::must_new("portfolio.main.pin".to_string()),
                &mut io,
                &ViewPinTask {
                    task_id: "pin.ethereum-mainnet".to_string(),
                    network_view_id: "ethereum-mainnet".to_string(),
                    family: mfm_portfolio_plan::NetworkFamily::Evm,
                    adapter: AdapterId(ADAPTER_PIN_VIEW_EVM.to_string()),
                    payload: BTreeMap::from([
                        ("network_id".to_string(), json!("ethereum-mainnet")),
                        ("chain_id".to_string(), json!(1u64)),
                        ("control_scope".to_string(), json!("rpc.mainnet")),
                    ]),
                },
                ViewRuntimeInput {
                    prepared_sources: &prepared_sources,
                },
            )
            .await
            .expect("pinned view");

        match view.anchor {
            ExecutionAnchor::Evm {
                chain_id,
                block_number,
            } => {
                assert_eq!(chain_id, 1);
                assert_eq!(block_number, 100);
            }
            ExecutionAnchor::Bitcoin { .. } => panic!("expected evm anchor"),
        }
    }

    #[tokio::test]
    async fn valuation_runtimes_emit_unit_price_provenance() {
        let direct = EvmOracleDirectPriceRuntimeAdapter;
        let derived = DerivedUnitPriceRuntimeAdapter;
        let mut io = TestIo {
            responses: BTreeMap::from([
                (
                    mfm_evm_runtime::states::read::encode_erc20_decimals(),
                    json!("0x0000000000000000000000000000000000000000000000000000000000000008"),
                ),
                (
                    "0xfeaf968c".to_string(),
                    json!(
                        "0x\
0000000000000000000000000000000000000000000000000000000000000001\
000000000000000000000000000000000000000000000000000000000bebc200\
0000000000000000000000000000000000000000000000000000000000000001\
0000000000000000000000000000000000000000000000000000000000000001\
0000000000000000000000000000000000000000000000000000000000000001"
                    ),
                ),
            ]),
        };
        let pinned_views = BTreeMap::from([(
            "ethereum-mainnet".to_string(),
            PinnedNetworkView {
                network_view_id: "ethereum-mainnet".to_string(),
                network_id: "ethereum-mainnet".to_string(),
                family: mfm_portfolio_plan::NetworkFamily::Evm,
                anchor: ExecutionAnchor::Evm {
                    chain_id: 1,
                    block_number: 100,
                },
            },
        )]);

        let direct_value = direct
            .resolve(
                &StateId::must_new("portfolio.main.values".to_string()),
                &mut io,
                &ValuationTask {
                    valuation_id: "eth.quote.usd".to_string(),
                    instrument_id: "eth.native.ethereum-mainnet".to_string(),
                    quote: mfm_portfolio_model::symbol::QuoteCode::Usd,
                    adapter: AdapterId(ADAPTER_RESOLVE_VALUATION_EVM_ORACLE.to_string()),
                    payload: BTreeMap::from([
                        ("priced_symbol_id".to_string(), json!("eth.native.ethereum-mainnet")),
                        (
                            "source".to_string(),
                            json!({
                                "source": {
                                    "source_id": "chainlink_eth_usd",
                                    "network_id": "ethereum-mainnet",
                                    "base_symbol_id": "eth.native.ethereum-mainnet",
                                    "quote": "USD"
                                },
                                "route_policy": {
                                    "network_id": "ethereum-mainnet",
                                    "chain_id": 1u64,
                                    "control_scope": "rpc.mainnet"
                                },
                                "source_reader": {
                                    "kind": "evm_oracle",
                                    "oracle_kind": "chainlink_aggregator_v3",
                                    "config": {
                                        "contract_address": "0x0000000000000000000000000000000000000010"
                                    }
                                }
                            }),
                        ),
                    ]),
                },
                ValuationRuntimeInput {
                    pinned_views: &pinned_views,
                },
            )
            .await
            .expect("direct");
        assert_eq!(direct_value.valuation_reader_kind, "direct_price");
        assert_eq!(direct_value.source_refs.len(), 1);

        let derived_value = derived
            .resolve(
                &StateId::must_new("portfolio.main.values".to_string()),
                &mut io,
                &ValuationTask {
                    valuation_id: "eth.quote.btc".to_string(),
                    instrument_id: "eth.native.ethereum-mainnet".to_string(),
                    quote: mfm_portfolio_model::symbol::QuoteCode::Btc,
                    adapter: AdapterId(ADAPTER_RESOLVE_VALUATION_DERIVED.to_string()),
                    payload: BTreeMap::from([
                        ("priced_symbol_id".to_string(), json!("eth.native.ethereum-mainnet")),
                        (
                            "numerator".to_string(),
                            json!({
                                "source": {
                                    "source_id": "chainlink_eth_usd",
                                    "network_id": "ethereum-mainnet",
                                    "base_symbol_id": "eth.native.ethereum-mainnet",
                                    "quote": "USD"
                                },
                                "route_policy": {
                                    "network_id": "ethereum-mainnet",
                                    "chain_id": 1u64,
                                    "control_scope": "rpc.mainnet"
                                },
                                "source_reader": {
                                    "kind": "evm_oracle",
                                    "oracle_kind": "chainlink_aggregator_v3",
                                    "config": {
                                        "contract_address": "0x0000000000000000000000000000000000000010"
                                    }
                                }
                            }),
                        ),
                        (
                            "denominator".to_string(),
                            json!({
                                "source": {
                                    "source_id": "chainlink_eth_usd",
                                    "network_id": "ethereum-mainnet",
                                    "base_symbol_id": "eth.native.ethereum-mainnet",
                                    "quote": "USD"
                                },
                                "route_policy": {
                                    "network_id": "ethereum-mainnet",
                                    "chain_id": 1u64,
                                    "control_scope": "rpc.mainnet"
                                },
                                "source_reader": {
                                    "kind": "evm_oracle",
                                    "oracle_kind": "chainlink_aggregator_v3",
                                    "config": {
                                        "contract_address": "0x0000000000000000000000000000000000000010"
                                    }
                                }
                            }),
                        ),
                    ]),
                },
                ValuationRuntimeInput {
                    pinned_views: &pinned_views,
                },
            )
            .await
            .expect("derived");
        assert_eq!(derived_value.valuation_reader_kind, "derived_unit_price");
        assert_eq!(derived_value.source_refs.len(), 2);
    }

    #[tokio::test]
    async fn native_observation_runtime_reads_balance_and_projects_values() {
        let adapter = EvmNativeBalanceObservationRuntimeAdapter;
        let mut io = TestIo {
            responses: BTreeMap::from([(
                "eth_getBalance".to_string(),
                json!("0x0de0b6b3a7640000"),
            )]),
        };
        let binding = mfm_portfolio_plan::CompiledObservationBinding {
            binding_id: "binding.wallet_main.eth".to_string(),
            observation_key: mfm_portfolio_plan::ObservationKey {
                subject_id: "wallet_main".to_string(),
                network_view_id: "ethereum-mainnet".to_string(),
                instrument_id: "eth.native.ethereum-mainnet".to_string(),
                position_kind: mfm_portfolio_plan::PositionKind::SpotBalance,
                venue_id: None,
                discriminator: Some("eth.native.ethereum-mainnet".to_string()),
            },
            adapter: AdapterId(ADAPTER_OBSERVE_EVM_NATIVE_BALANCE.to_string()),
            valuation_ids: vec!["eth.quote.usd".to_string()],
            payload: BTreeMap::from([
                (
                    "projection".to_string(),
                    json!({
                        "symbol_id": "eth.native.ethereum-mainnet",
                        "display_symbol": "ETH",
                        "kind": "native_balance",
                        "role": "native",
                        "network_id": "ethereum-mainnet",
                        "protocol": null,
                        "decimals": 18
                    }),
                ),
                (
                    "route_policy".to_string(),
                    json!({
                        "network_id": "ethereum-mainnet",
                        "chain_id": 1u64,
                        "control_scope": "rpc.mainnet"
                    }),
                ),
            ]),
        };
        let observation = adapter
            .observe(
                &StateId::must_new("portfolio.main.observe_native".to_string()),
                &mut io,
                &binding,
                ObservationRuntimeInput {
                    resolved_subjects: &BTreeMap::from([(
                        "wallet_main".to_string(),
                        ResolvedSubject {
                            subject_id: "wallet_main".to_string(),
                            kind: SubjectKind::EvmAddress,
                            value: serde_json::to_value(EvmResolvedSubjectValue {
                                network_id: "ethereum-mainnet".to_string(),
                                address: "0x000000000000000000000000000000000000dead".to_string(),
                                implementation_kind: "address_only".to_string(),
                                capabilities: WalletCapabilities {
                                    can_resolve_address: true,
                                    can_sign: false,
                                    can_submit: false,
                                },
                            })
                            .expect("subject"),
                        },
                    )]),
                    pinned_views: &BTreeMap::from([(
                        "ethereum-mainnet".to_string(),
                        PinnedNetworkView {
                            network_view_id: "ethereum-mainnet".to_string(),
                            network_id: "ethereum-mainnet".to_string(),
                            family: mfm_portfolio_plan::NetworkFamily::Evm,
                            anchor: ExecutionAnchor::Evm {
                                chain_id: 1,
                                block_number: 100,
                            },
                        },
                    )]),
                    resolved_valuations: &BTreeMap::from([(
                        "eth.quote.usd".to_string(),
                        ResolvedUnitPrice {
                            valuation_id: "eth.quote.usd".to_string(),
                            instrument_id: "eth.native.ethereum-mainnet".to_string(),
                            priced_symbol_id: "eth.native.ethereum-mainnet".to_string(),
                            quote: mfm_portfolio_model::symbol::QuoteCode::Usd,
                            unit_price_dec: "2.00".to_string(),
                            valuation_reader_kind: "fixed_unit_price".to_string(),
                            source_refs: Vec::new(),
                        },
                    )]),
                },
            )
            .await
            .expect("observation");

        assert_eq!(observation.wallet_id, "wallet_main");
        assert_eq!(observation.quantity.decimals, 18);
        assert_eq!(observation.quantity.amount_dec, "1.000000000000000000");
        assert_eq!(
            observation.values[0].priced_symbol_id,
            "eth.native.ethereum-mainnet"
        );
        assert_eq!(observation.values[0].value_dec, "2.000000000000000000");
        assert_eq!(observation.source.balance_reader_kind, "native_balance");
    }

    #[tokio::test]
    async fn erc20_observation_runtime_fetches_missing_decimals() {
        let adapter = EvmErc20BalanceObservationRuntimeAdapter;
        let wallet: Address = "0x000000000000000000000000000000000000dead"
            .parse()
            .expect("wallet");
        let mut io = TestIo {
            responses: BTreeMap::from([
                (
                    mfm_evm_runtime::states::read::encode_erc20_decimals(),
                    json!("0x0000000000000000000000000000000000000000000000000000000000000006"),
                ),
                (
                    encode_erc20_balance_of(&wallet),
                    json!("0x0000000000000000000000000000000000000000000000000000000000989680"),
                ),
            ]),
        };
        let binding = mfm_portfolio_plan::CompiledObservationBinding {
            binding_id: "binding.wallet_main.usdc".to_string(),
            observation_key: mfm_portfolio_plan::ObservationKey {
                subject_id: "wallet_main".to_string(),
                network_view_id: "ethereum-mainnet".to_string(),
                instrument_id: "usdc.wallet.ethereum-mainnet".to_string(),
                position_kind: mfm_portfolio_plan::PositionKind::SpotBalance,
                venue_id: None,
                discriminator: Some("usdc.wallet.ethereum-mainnet".to_string()),
            },
            adapter: AdapterId(ADAPTER_OBSERVE_EVM_ERC20_BALANCE.to_string()),
            valuation_ids: vec!["usdc.quote.usd".to_string()],
            payload: BTreeMap::from([
                (
                    "projection".to_string(),
                    json!({
                        "symbol_id": "usdc.wallet.ethereum-mainnet",
                        "display_symbol": "USDC",
                        "kind": "erc20_balance",
                        "role": "asset",
                        "network_id": "ethereum-mainnet",
                        "protocol": null,
                        "decimals": null
                    }),
                ),
                (
                    "route_policy".to_string(),
                    json!({
                        "network_id": "ethereum-mainnet",
                        "chain_id": 1u64,
                        "control_scope": "rpc.mainnet"
                    }),
                ),
                (
                    "token_address".to_string(),
                    json!("0x0000000000000000000000000000000000000001"),
                ),
            ]),
        };
        let observation = adapter
            .observe(
                &StateId::must_new("portfolio.main.observe_erc20".to_string()),
                &mut io,
                &binding,
                ObservationRuntimeInput {
                    resolved_subjects: &BTreeMap::from([(
                        "wallet_main".to_string(),
                        ResolvedSubject {
                            subject_id: "wallet_main".to_string(),
                            kind: SubjectKind::EvmAddress,
                            value: serde_json::to_value(EvmResolvedSubjectValue {
                                network_id: "ethereum-mainnet".to_string(),
                                address: "0x000000000000000000000000000000000000dead".to_string(),
                                implementation_kind: "address_only".to_string(),
                                capabilities: WalletCapabilities {
                                    can_resolve_address: true,
                                    can_sign: false,
                                    can_submit: false,
                                },
                            })
                            .expect("subject"),
                        },
                    )]),
                    pinned_views: &BTreeMap::from([(
                        "ethereum-mainnet".to_string(),
                        PinnedNetworkView {
                            network_view_id: "ethereum-mainnet".to_string(),
                            network_id: "ethereum-mainnet".to_string(),
                            family: mfm_portfolio_plan::NetworkFamily::Evm,
                            anchor: ExecutionAnchor::Evm {
                                chain_id: 1,
                                block_number: 100,
                            },
                        },
                    )]),
                    resolved_valuations: &BTreeMap::from([(
                        "usdc.quote.usd".to_string(),
                        ResolvedUnitPrice {
                            valuation_id: "usdc.quote.usd".to_string(),
                            instrument_id: "usdc.wallet.ethereum-mainnet".to_string(),
                            priced_symbol_id: "usdc.wallet.ethereum-mainnet".to_string(),
                            quote: mfm_portfolio_model::symbol::QuoteCode::Usd,
                            unit_price_dec: "1.00".to_string(),
                            valuation_reader_kind: "fixed_unit_price".to_string(),
                            source_refs: Vec::new(),
                        },
                    )]),
                },
            )
            .await
            .expect("observation");

        assert_eq!(observation.quantity.decimals, 6);
        assert_eq!(observation.quantity.amount_dec, "10.000000");
        assert_eq!(observation.values[0].value_dec, "10.000000");
        assert_eq!(observation.source.balance_reader_kind, "erc20_balance");
    }
}
