use mfm_state_portfolio::plan::{EvmRoutePolicy, ObservationProjection};
use serde::{Deserialize, Serialize};

use crate::portfolio::model::{AaveDebtPositionConfig, AaveReservePositionConfig};

/// Planner/runtime payload for one Aave reserve-position observation binding.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AaveReserveObservationPayload {
    /// Read-model projection preserved for the emitted observation.
    pub projection: ObservationProjection,
    /// Route policy used for the pinned EVM read.
    pub route_policy: EvmRoutePolicy,
    /// Underlying symbol identity used for valuation outputs.
    pub underlying_symbol_id: String,
    /// Typed Aave reserve-position config.
    pub config: AaveReservePositionConfig,
}

/// Planner/runtime payload for one Aave debt-position observation binding.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AaveDebtObservationPayload {
    /// Read-model projection preserved for the emitted observation.
    pub projection: ObservationProjection,
    /// Route policy used for the pinned EVM read.
    pub route_policy: EvmRoutePolicy,
    /// Underlying symbol identity used for valuation outputs.
    pub underlying_symbol_id: String,
    /// Typed Aave debt-position config.
    pub config: AaveDebtPositionConfig,
}
