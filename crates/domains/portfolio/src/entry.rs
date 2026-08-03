//! Public portfolio entry selector.

use mfm_program_derive::MfmValue;
use serde::{Deserialize, Serialize};

use crate::PortfolioId;

/// Sole published portfolio snapshot entry-point id.
pub const PORTFOLIO_SNAPSHOT_ENTRY_POINT_ID: &str = "mfm.portfolio/snapshot@1";

/// Public value-only selector for one configured portfolio.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "snapshot-selector",
    version = "1",
    schema = "mfm.portfolio.snapshot_selector"
)]
pub struct PortfolioSnapshotSelector {
    target: PortfolioId,
}

impl PortfolioSnapshotSelector {
    /// Creates a selector from one checked configured-value target.
    pub const fn new(target: PortfolioId) -> Self {
        Self { target }
    }

    /// Returns the exact configured-value target.
    pub const fn target(&self) -> &PortfolioId {
        &self.target
    }
}
