//! Token operations module for token-specific operations
// use super::*;
use crate::blockchain::adapter::types::{Address, U256};
use crate::blockchain::BlockchainError;
use crate::contexts::portfolio::PortfolioContext;
use crate::operations::{Operation, OperationError, OperationResult, OperationTag};
// use futures::executor::block_on;
use std::future::Future;
use std::pin::Pin;
use std::str::FromStr;

/// Token approval operation
#[derive(Clone)]
pub struct TokenApprovalOperation {
    pub context: PortfolioContext,
    pub token_address: Address,
    pub spender_address: Address,
    pub amount: U256,
    pub exact_approval: bool,
}

impl Operation<()> for TokenApprovalOperation {
    fn execute(&self) -> Pin<Box<dyn Future<Output = OperationResult<()>> + Send + '_>> {
        let context = self.context.clone();
        let token_address = self.token_address;
        let spender_address = self.spender_address;
        let amount = self.amount;
        let exact_approval = self.exact_approval;

        Box::pin(async move {
            // Get the wallet address
            let wallet_address_str = context.get_wallet_address();
            let wallet_address = Address::from_str(&wallet_address_str)
                .map_err(|e| OperationError::Failed(format!("Invalid wallet address: {}", e)))?;

            // Use the with_blockchain_provider method to safely access the provider
            let result = context.with_blockchain_provider(|provider| {
                // Create a future that we'll run
                let future = async {
                    // We need to check the current allowance to avoid unnecessary approvals
                    let current_allowance = provider
                        .check_token_allowance(token_address, wallet_address, spender_address)
                        .await?;

                    // If the current allowance is sufficient, we don't need to approve again
                    if current_allowance >= amount {
                        println!("Current allowance of {} is sufficient", current_allowance);
                        return Ok(());
                    }

                    // If exact_approval is true, we approve the exact amount
                    // If false, we approve the maximum amount to avoid multiple approvals
                    let approval_amount = if exact_approval {
                        amount
                    } else {
                        U256::from(alloy_primitives::U256::MAX)
                    };

                    // Log the approval we're about to do
                    println!(
                        "Approving {} tokens to be spent by {}",
                        approval_amount, spender_address
                    );

                    // Approve the token
                    let tx_hash = provider
                        .approve_token(token_address, spender_address, approval_amount)
                        .await?;

                    println!("Approval successful with transaction: {}", tx_hash);
                    Ok(())
                };

                // Use tokio::runtime::Handle to run the future
                match tokio::runtime::Handle::try_current() {
                    Ok(handle) => handle.block_on(future),
                    Err(_) => Err(BlockchainError::Other("No tokio runtime available".into())),
                }
            });

            // Map the result to the appropriate operation error
            result.map_err(OperationError::Portfolio)
        })
    }

    fn tags(&self) -> Vec<OperationTag> {
        vec![OperationTag::Blockchain, OperationTag::Network]
    }
}

/// Create a token approval operation
pub fn create_token_approval_operation(
    context: PortfolioContext,
    token_address: Address,
    spender_address: Address,
    amount: U256,
    exact_approval: bool,
) -> impl Operation<()> {
    TokenApprovalOperation {
        context,
        token_address,
        spender_address,
        amount,
        exact_approval,
    }
}

/*
// Token balance check operation is disabled until check_token_balance is implemented
pub struct TokenBalanceOperation {
    pub context: PortfolioContext,
    pub token: Address,
    pub owner: Address,
}

impl Operation<U256> for TokenBalanceOperation {
    fn execute(&self) -> Pin<Box<dyn Future<Output = OperationResult<U256>> + Send + '_>> {
        let context = self.context.clone();
        let token = self.token;
        let owner = self.owner;

        Box::pin(async move {
            let result = context.check_token_balance(token, owner).await;
            result.map_err(OperationError::Portfolio)
        })
    }

    fn tags(&self) -> Vec<OperationTag> {
        vec![OperationTag::Blockchain, OperationTag::Network]
    }
}
*/
