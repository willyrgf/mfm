use mfm_state_symbol::model::ValuationSourceRegistry;

use crate::model::PortfolioConfig;

use super::{PlanningError, PortfolioExecutionSpec, SemanticCatalog};

/// Canonical planner request for semantic portfolio compilation.
#[derive(Clone, Debug, PartialEq)]
pub struct PortfolioRequest {
    /// Canonical portfolio config supplied by the caller.
    pub portfolio: PortfolioConfig,
    /// Canonical valuation source registry supplied by the caller.
    pub valuation_source_registry: ValuationSourceRegistry,
}

/// Pure compiler that lowers canonical portfolio requests into semantic execution specs.
pub trait PortfolioSemanticCompiler: Send + Sync {
    /// Compiles the request into one planner-owned semantic execution spec.
    fn compile(
        &self,
        request: &PortfolioRequest,
        catalog: &SemanticCatalog,
    ) -> Result<PortfolioExecutionSpec, PlanningError>;
}
