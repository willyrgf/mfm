//! Typed family receipt input for receipt-pinned portfolio fact selection.

use mfm_bitcoin::BitcoinBalanceCollectionReceipt;
use mfm_program_derive::{MfmValue, StateInput};
use mfm_states_evm::EvmBalanceCollectionReceipt;
use serde::{Deserialize, Serialize};

/// Stable logical portfolio holding requirement.
///
/// This key carries presentation identifiers only at the portfolio boundary; source-near facts
/// never include these fields.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "holding-requirement-key",
    schema = "mfm.portfolio.holding_requirement_key"
)]
pub struct HoldingRequirementKey {
    /// Stable wallet identifier.
    pub wallet_id: String,
    /// Stable symbol identifier.
    pub symbol_id: String,
    /// Stable semantic network identifier.
    pub network_id: String,
}

impl HoldingRequirementKey {
    /// Returns the canonical diagnostic key.
    pub fn as_key_str(&self) -> String {
        format!("{}/{}/{}", self.wallet_id, self.symbol_id, self.network_id)
    }
}

/// Typed family receipts consumed directly by receipt-pinned holding selection.
///
/// These vectors are both selection authority and the collector-settlement completion barrier. No
/// portfolio-owned fan-in receipt is constructed between collection and selection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, StateInput)]
#[serde(deny_unknown_fields)]
#[mfm(schema = "mfm.portfolio.input.select_holdings")]
pub struct SelectHoldingsInput {
    /// Exact Bitcoin network collection receipts produced by the same graph.
    pub bitcoin_receipts: Vec<BitcoinBalanceCollectionReceipt>,
    /// Exact EVM balance collection receipts produced by the same graph.
    pub evm_receipts: Vec<EvmBalanceCollectionReceipt>,
}
