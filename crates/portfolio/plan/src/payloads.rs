use mfm_portfolio_model::symbol::{
    PriceSourceRef, SymbolKind, SymbolRole, ValuationSourceReaderConfig,
};
use mfm_portfolio_model::wallet::{WalletCapabilities, WalletImplementationConfig};
use serde::{Deserialize, Serialize};

/// Planner/runtime payload for one EVM subject locator.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvmSubjectLocator {
    /// Stable network identifier for the subject.
    pub network_id: String,
    /// Canonical EVM address for the subject.
    pub address: String,
    /// Configured wallet implementation used to derive the subject.
    pub implementation: WalletImplementationConfig,
}

/// Runtime-resolved EVM subject value.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvmResolvedSubjectValue {
    /// Stable network identifier for the resolved subject.
    pub network_id: String,
    /// Canonical EVM address for the resolved subject.
    pub address: String,
    /// Stable resolved implementation kind.
    pub implementation_kind: String,
    /// Runtime capabilities derived during resolution.
    pub capabilities: WalletCapabilities,
}

/// Planner/runtime payload for one Bitcoin address subject locator.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BitcoinSubjectLocator {
    /// Stable network identifier for the subject.
    pub network_id: String,
    /// Canonical Bitcoin address for the subject.
    pub address: String,
    /// Configured wallet implementation used to derive the subject.
    pub implementation: WalletImplementationConfig,
}

/// Runtime-resolved Bitcoin address subject value.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BitcoinResolvedSubjectValue {
    /// Stable network identifier for the resolved subject.
    pub network_id: String,
    /// Canonical Bitcoin address for the resolved subject.
    pub address: String,
    /// Stable resolved implementation kind.
    pub implementation_kind: String,
    /// Runtime capabilities derived during resolution.
    pub capabilities: WalletCapabilities,
}

/// Planner-owned EVM execution routing policy.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvmRoutePolicy {
    /// Stable network identifier addressed by this route.
    pub network_id: String,
    /// Expected EVM chain id for this route.
    pub chain_id: u64,
    /// Stable control-plane scope used for managed RPC reads.
    pub control_scope: String,
}

/// Planner-owned Bitcoin execution routing policy.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BitcoinRoutePolicy {
    /// Stable network identifier addressed by this route.
    pub network_id: String,
    /// Stable control-plane scope used for managed source reads.
    pub control_scope: String,
}

/// Planner-owned quantity rendering schema for one instrument.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuantitySchema {
    /// Optional planner-configured decimals. Runtime adapters resolve missing decimals when needed.
    pub decimals: Option<u8>,
}

/// Stable observation projection used to preserve current read-model fields.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObservationProjection {
    /// Stable canonical symbol identifier.
    pub symbol_id: String,
    /// Optional display symbol rendered in observation outputs.
    pub display_symbol: Option<String>,
    /// Canonical symbol kind preserved in read models.
    pub kind: SymbolKind,
    /// Canonical symbol role preserved in read models.
    pub role: SymbolRole,
    /// Stable network identifier rendered in observation outputs.
    pub network_id: String,
    /// Optional protocol identifier rendered in observation outputs.
    pub protocol: Option<String>,
    /// Optional planner-configured decimals used to format raw quantities.
    pub decimals: Option<u8>,
}

/// Planner/runtime payload for one native-balance observation binding.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NativeBalanceObservationPayload {
    /// Read-model projection preserved for the emitted observation.
    pub projection: ObservationProjection,
    /// Route policy used for the pinned EVM read.
    pub route_policy: EvmRoutePolicy,
}

/// Planner/runtime payload for one ERC-20 balance observation binding.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Erc20BalanceObservationPayload {
    /// Read-model projection preserved for the emitted observation.
    pub projection: ObservationProjection,
    /// Route policy used for the pinned EVM read.
    pub route_policy: EvmRoutePolicy,
    /// Canonical token contract address.
    pub token_address: String,
}

/// Planner/runtime payload for one Bitcoin UTXO-set observation binding.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BitcoinUtxoSetObservationPayload {
    /// Read-model projection preserved for the emitted observation.
    pub projection: ObservationProjection,
    /// Route policy used for the pinned Bitcoin read.
    pub route_policy: BitcoinRoutePolicy,
}

/// Planner/runtime payload for one fixed-unit-price valuation task.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FixedUnitPriceValuationPayload {
    /// Symbol identity whose unit price applies to the observation.
    pub priced_symbol_id: String,
    /// Canonical decimal-string unit price.
    pub unit_price_dec: String,
}

/// Planner/runtime payload for one direct price source read.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DirectPriceSourcePayload {
    /// Stable valuation source reference.
    pub source: PriceSourceRef,
    /// Route policy used for the pinned source read.
    pub route_policy: EvmRoutePolicy,
    /// Concrete source reader configuration.
    pub source_reader: ValuationSourceReaderConfig,
}

/// Planner/runtime payload for one direct-unit-price valuation task.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DirectPriceValuationPayload {
    /// Symbol identity whose unit price applies to the observation.
    pub priced_symbol_id: String,
    /// Direct price source used to resolve the unit price.
    pub source: DirectPriceSourcePayload,
}

/// Planner/runtime payload for one derived-unit-price valuation task.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DerivedUnitPriceValuationPayload {
    /// Symbol identity whose unit price applies to the observation.
    pub priced_symbol_id: String,
    /// Numerator source for the derived price.
    pub numerator: DirectPriceSourcePayload,
    /// Denominator source for the derived price.
    pub denominator: DirectPriceSourcePayload,
}
