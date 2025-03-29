// aave health check operations with state machine approach
use crate::blockchain::aave::{AaveHealthCheckResult, AaveUserAccountData};
use crate::blockchain::adapter::types::{Address, U256};
use crate::contexts::portfolio::PortfolioContext;
use crate::operations::{Operation, OperationError, OperationResult, OperationTag};
use mfm_machine::state::{
    safe_context::create_default_safe_context, safe_context::SafeContext, standard_tags,
    DependencyStrategy, Label, StateError, StateErrorRecoverability, StateHandler, StateMetadata,
    StateResult, Tag,
};
use mfm_machine::state_machine::StateMachine;
use mfm_machine_derive::StateMetadataReqs;
use serde_derive::{Deserialize, Serialize};
use std::future::Future;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;

/// aave health data collection state - fetches raw data from aave provider
#[derive(Debug, Clone, StateMetadataReqs)]
pub struct AaveHealthDataCollectionState {
    label: Label,
    tags: Vec<Tag>,
    depends_on: Vec<Tag>,
    depends_on_strategy: DependencyStrategy,
    context: PortfolioContext,
    address: Address,
}

impl AaveHealthDataCollectionState {
    pub fn new(context: PortfolioContext, address: Address) -> Self {
        Self {
            label: Label::new("aave_health_data_collection").unwrap(),
            tags: vec![
                Tag::new("aave").unwrap(),
                Tag::new("health_check").unwrap(),
                standard_tags::FETCH_DATA,
            ],
            depends_on: vec![],
            depends_on_strategy: DependencyStrategy::Latest,
            context,
            address,
        }
    }
}

/// storage structure for raw aave health data
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct AaveHealthData {
    pub health_result: AaveHealthCheckResult,
    pub account_data: AaveUserAccountData,
}

impl StateHandler for AaveHealthDataCollectionState {
    fn handler(&self, context: SafeContext) -> StateResult {
        // this uses cloning to avoid async issues with the context
        let portfolio_context = self.context.clone();
        let address = self.address;

        tokio::task::block_in_place(|| {
            let runtime = tokio::runtime::Handle::current();

            runtime.block_on(async move {
                // Get the Aave provider directly using our new method
                let aave_provider = match portfolio_context.get_aave_provider().await {
                    Ok(provider) => provider,
                    Err(e) => {
                        return Err(StateError::OnChainError(
                            StateErrorRecoverability::Recoverable,
                            anyhow::anyhow!("Failed to get Aave provider: {}", e),
                        ));
                    }
                };

                // get health result and account data
                let health_result = match aave_provider.calculate_health_factor(address).await {
                    Ok(result) => result,
                    Err(e) => {
                        return Err(StateError::OnChainError(
                            StateErrorRecoverability::Recoverable,
                            anyhow::anyhow!("Failed to calculate health factor: {}", e),
                        ));
                    }
                };

                let account_data = match aave_provider.get_user_account_data(address).await {
                    Ok(data) => data,
                    Err(e) => {
                        return Err(StateError::OnChainError(
                            StateErrorRecoverability::Recoverable,
                            anyhow::anyhow!("Failed to get user account data: {}", e),
                        ));
                    }
                };

                // store data in context
                let aave_health_data = AaveHealthData {
                    health_result,
                    account_data,
                };

                context
                    .write_typed("aave_health_data", &aave_health_data)
                    .map_err(|e| {
                        StateError::StorageAccess(StateErrorRecoverability::Recoverable, e)
                    })
            })
        })
    }
}

/// aave health data formatting state - processes raw data into formatted output
#[derive(Debug, Clone, StateMetadataReqs)]
pub struct AaveHealthDataFormattingState {
    label: Label,
    tags: Vec<Tag>,
    depends_on: Vec<Tag>,
    depends_on_strategy: DependencyStrategy,
}

impl AaveHealthDataFormattingState {
    pub fn new() -> Self {
        Self {
            label: Label::new("aave_health_data_formatting").unwrap(),
            tags: vec![
                Tag::new("aave").unwrap(),
                Tag::new("health_check").unwrap(),
                standard_tags::COMPUTE,
            ],
            depends_on: vec![Tag::new("aave").unwrap(), standard_tags::FETCH_DATA],
            depends_on_strategy: DependencyStrategy::Latest,
        }
    }

    /// helper function to convert u256 to f64 with decimals
    fn u256_to_f64(value: &U256, decimals: u8) -> f64 {
        let divisor = 10u128.pow(decimals as u32);
        let high_bits = ((value.0.as_limbs()[3] as u128) << 96)
            | ((value.0.as_limbs()[2] as u128) << 64)
            | ((value.0.as_limbs()[1] as u128) << 32)
            | (value.0.as_limbs()[0] as u128);

        high_bits as f64 / divisor as f64
    }
}

impl Default for AaveHealthDataFormattingState {
    fn default() -> Self {
        Self::new()
    }
}

/// formatted aave health data for presentation
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct FormattedAaveHealthData {
    pub health_factor: f64,
    pub total_collateral_usd: f64,
    pub total_debt_usd: f64,
    pub available_borrow_usd: f64,
    pub liquidation_threshold: f64,
    pub ltv: f64,
    pub max_decrease_percentage: f64,
    pub user_reserves: Vec<String>,
    pub health_status: String,
}

impl StateHandler for AaveHealthDataFormattingState {
    fn handler(&self, context: SafeContext) -> StateResult {
        // read the raw health data
        let aave_health_data: AaveHealthData =
            context.read_typed("aave_health_data").map_err(|e| {
                StateError::StorageAccess(
                    StateErrorRecoverability::Recoverable,
                    anyhow::anyhow!("Failed to read aave health data: {}", e),
                )
            })?;

        // extract data
        let health_result = aave_health_data.health_result;
        let account_data = aave_health_data.account_data;

        // format the data
        let available_borrow_usd = Self::u256_to_f64(&account_data.available_borrow_base, 8);
        let liquidation_threshold =
            Self::u256_to_f64(&account_data.current_liquidation_threshold, 2);
        let ltv = Self::u256_to_f64(&account_data.ltv, 2);

        // determine health status
        let health_status = if health_result.health_factor > 2.0 {
            "HEALTHY".to_string()
        } else if health_result.health_factor > 1.1 {
            "CAUTION".to_string()
        } else {
            "DANGER".to_string()
        };

        // format user reserves
        let user_reserves = health_result
            .user_reserves
            .iter()
            .map(|reserve| {
                format!(
                    "{}: Balance: {}, Variable Debt: {}, USD Value: ${:.2}",
                    reserve.symbol,
                    reserve.current_atoken_balance,
                    reserve.current_variable_debt,
                    reserve.price_usd
                )
            })
            .collect();

        // create formatted data
        let formatted_data = FormattedAaveHealthData {
            health_factor: health_result.health_factor,
            total_collateral_usd: health_result.total_collateral_usd,
            total_debt_usd: health_result.total_debt_usd,
            available_borrow_usd,
            liquidation_threshold,
            ltv,
            max_decrease_percentage: health_result.max_decrease_percentage,
            user_reserves,
            health_status,
        };

        // store formatted data
        context
            .write_typed("formatted_aave_health_data", &formatted_data)
            .map_err(|e| {
                StateError::StorageAccess(
                    StateErrorRecoverability::Recoverable,
                    anyhow::anyhow!("Failed to write formatted aave health data: {}", e),
                )
            })
    }
}

/// aave health data presentation state - displays the formatted data
#[derive(Debug, Clone, StateMetadataReqs)]
pub struct AaveHealthDataPresentationState {
    label: Label,
    tags: Vec<Tag>,
    depends_on: Vec<Tag>,
    depends_on_strategy: DependencyStrategy,
}

impl AaveHealthDataPresentationState {
    pub fn new() -> Self {
        Self {
            label: Label::new("aave_health_data_presentation").unwrap(),
            tags: vec![
                Tag::new("aave").unwrap(),
                Tag::new("health_check").unwrap(),
                standard_tags::REPORT,
            ],
            depends_on: vec![Tag::new("aave").unwrap(), standard_tags::COMPUTE],
            depends_on_strategy: DependencyStrategy::Latest,
        }
    }
}

impl Default for AaveHealthDataPresentationState {
    fn default() -> Self {
        Self::new()
    }
}

impl StateHandler for AaveHealthDataPresentationState {
    fn handler(&self, context: SafeContext) -> StateResult {
        // read the formatted health data
        let formatted_data: FormattedAaveHealthData = context
            .read_typed("formatted_aave_health_data")
            .map_err(|e| {
                StateError::StorageAccess(
                    StateErrorRecoverability::Recoverable,
                    anyhow::anyhow!("Failed to read formatted aave health data: {}", e),
                )
            })?;

        // print formatted data
        println!("\n=== AAVE Health Check Results ===");
        println!(
            "Total Collateral (USD): {:.2}",
            formatted_data.total_collateral_usd
        );
        println!("Total Debt (USD): {:.2}", formatted_data.total_debt_usd);
        println!(
            "Available Borrow (USD): {:.2}",
            formatted_data.available_borrow_usd
        );
        println!(
            "Liquidation Threshold: {:.2}%",
            formatted_data.liquidation_threshold
        );
        println!("Loan to Value (LTV): {:.2}%", formatted_data.ltv);
        println!(
            "Health Factor: {:.4} ({})",
            formatted_data.health_factor, formatted_data.health_status
        );
        println!(
            "Max Collateral Decrease: {:.2}%",
            formatted_data.max_decrease_percentage
        );

        if formatted_data.user_reserves.is_empty() {
            println!("\nNo reserves found or not fully implemented yet.");
        } else {
            println!("\nReserves:");
            for reserve in formatted_data.user_reserves {
                println!("{}", reserve);
            }
        }

        Ok(())
    }
}

/// aave health check operation that uses the state machine
pub struct AaveHealthCheckStatefulOperation {
    pub context: PortfolioContext,
    pub address: Address,
}

impl Operation<AaveHealthCheckResult> for AaveHealthCheckStatefulOperation {
    fn execute(
        &self,
    ) -> Pin<Box<dyn Future<Output = OperationResult<AaveHealthCheckResult>> + Send + '_>> {
        let context = self.context.clone();
        let address = self.address;

        Box::pin(async move {
            // create the states
            let collection_state = AaveHealthDataCollectionState::new(context.clone(), address);
            let formatting_state = AaveHealthDataFormattingState::new();
            let presentation_state = AaveHealthDataPresentationState::new();

            // collect states into a vector
            let states: Vec<Box<dyn StateHandler>> = vec![
                Box::new(collection_state),
                Box::new(formatting_state),
                Box::new(presentation_state),
            ];

            // create state machine and safe context
            let states = Arc::from(states);
            let mut state_machine = StateMachine::new(states);
            let safe_context = create_default_safe_context();

            // execute the state machine
            match state_machine.execute(safe_context) {
                Ok(context) => {
                    // read the raw health data to return the result
                    let aave_health_data: AaveHealthData = context
                        .read_typed("aave_health_data")
                        .map_err(|e| {
                        OperationError::Failed(format!("Failed to read aave health data: {}", e))
                    })?;

                    Ok(aave_health_data.health_result)
                }
                Err(e) => Err(OperationError::Failed(format!(
                    "State machine execution failed: {:#?}",
                    e
                ))),
            }
        })
    }

    fn tags(&self) -> Vec<OperationTag> {
        vec![OperationTag::Blockchain, OperationTag::Network]
    }
}

/// factory function to create aave health check operation with state machine
pub fn create_aave_health_check_stateful_operation(
    context: PortfolioContext,
    wallet_address: Option<String>,
) -> impl Operation<AaveHealthCheckResult> {
    // parse address from string or get default from context
    let address = match wallet_address {
        Some(addr_str) => match Address::from_str(&addr_str) {
            Ok(addr) => addr,
            Err(_) => {
                // if address is invalid, use the default from context
                let addr_str = context.get_wallet_address();
                Address::from_str(&addr_str).unwrap_or_else(|_| {
                    // fallback to zero address as last resort
                    Address::zero()
                })
            }
        },
        None => {
            // use address from context
            let addr_str = context.get_wallet_address();
            Address::from_str(&addr_str).unwrap_or_else(|_| {
                // fallback to zero address as last resort
                Address::zero()
            })
        }
    };

    AaveHealthCheckStatefulOperation { context, address }
}
