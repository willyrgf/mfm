//! DeFi operations for interacting with decentralized finance protocols

// Expose the aave_health module
pub mod aave_health;

use super::*;
use crate::blockchain::aave::{AaveHealthCheckResult, AaveProvider, MockAaveProvider};
use crate::blockchain::adapter::types::Address;
use std::str::FromStr;
use std::sync::Arc;

// Re-export main functions from aave_health module
pub use aave_health::{
    create_aave_health_check_stateful_operation, AaveHealthCheckStatefulOperation,
};

/// aave health factor operation
pub struct AaveHealthFactorOperation {
    pub context: PortfolioContext,
    pub address: Option<String>,
    pub aave_provider: Arc<dyn AaveProvider + Send + Sync>,
}

impl Operation<AaveHealthCheckResult> for AaveHealthFactorOperation {
    fn execute(
        &self,
    ) -> Pin<Box<dyn Future<Output = OperationResult<AaveHealthCheckResult>> + Send + '_>> {
        let context = self.context.clone();
        let address_opt = self.address.clone();
        let aave_provider = self.aave_provider.clone();

        Box::pin(async move {
            // use provided wallet address or default to config wallet
            let addr_str = match address_opt {
                Some(addr) => addr,
                None => context.get_wallet_address(),
            };

            // parse address
            let address = Address::from_str(&addr_str)
                .map_err(|e| OperationError::Failed(format!("Invalid address: {}", e)))?;

            // calculate health factor
            let health_result = aave_provider
                .calculate_health_factor(address)
                .await
                .map_err(|e| {
                    OperationError::Failed(format!("Failed to get health factor: {}", e))
                })?;

            Ok(health_result)
        })
    }

    fn tags(&self) -> Vec<OperationTag> {
        vec![OperationTag::Blockchain, OperationTag::Network]
    }
}

/// factory function to create a chain of aave operations
pub fn create_aave_health_check_operation(
    context: PortfolioContext,
    address: Option<String>,
    aave_provider: Arc<dyn AaveProvider + Send + Sync>,
) -> impl Operation<String> {
    AaveHealthFactorOperation {
        context,
        address,
        aave_provider,
    }
    .map(|health_result| {
        // format health factor with color indicators
        let status = if health_result.health_factor > 2.0 {
            "HEALTHY"
        } else if health_result.health_factor > 1.1 {
            "CAUTION"
        } else {
            "DANGER"
        };

        format!(
            "Aave Health Factor: {:.2} ({}) - Collateral: ${:.2}, Debt: ${:.2}",
            health_result.health_factor,
            status,
            health_result.total_collateral_usd,
            health_result.total_debt_usd
        )
    })
}

/// Simplified factory function to create a health check operation with just address
pub fn create_aave_health_check_operation_with_address(
    context: PortfolioContext,
    address: Address,
) -> impl Operation<AaveHealthCheckResult> {
    // Create a mock Aave provider for simple use cases
    let aave_provider = Arc::new(MockAaveProvider::new(address));

    AaveHealthFactorOperation {
        context,
        address: Some(address.to_string()),
        aave_provider,
    }
}
