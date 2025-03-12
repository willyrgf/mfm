use anyhow::Result;
use ethers::types::{Address, U256};
use mfm_machine::state::{
    DependencyStrategy, Label, StateHandler, StateMetadata, StateResult, Tag,
};
use mfm_machine_derive::StateMetadataReqs;
use serde::{Deserialize, Serialize};

use crate::blockchain::{BlockchainError, BlockchainProvider, DexProvider, SwapQuote};

#[derive(Debug, Clone, PartialEq, StateMetadataReqs)]
pub struct CheckBalance {
    label: Label,
    tags: Vec<Tag>,
    depends_on: Vec<Tag>,
    depends_on_strategy: DependencyStrategy,
}

pub static CHECK_BALANCE: Label = Label("check_balance");

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckBalanceInput {
    pub wallet_address: Address,
    pub token_address: Option<Address>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckBalanceOutput {
    pub balance: U256,
}

impl CheckBalance {
    pub fn new(
        tags: Vec<Tag>,
        depends_on: Vec<Tag>,
        depends_on_strategy: DependencyStrategy,
    ) -> Self {
        Self {
            label: CHECK_BALANCE,
            tags,
            depends_on,
            depends_on_strategy,
        }
    }
}

impl Default for CheckBalance {
    fn default() -> Self {
        Self {
            label: CHECK_BALANCE,
            tags: vec![Tag::new("blockchain").unwrap()],
            depends_on: vec![Tag::new("setup").unwrap()],
            depends_on_strategy: DependencyStrategy::Latest,
        }
    }
}

impl StateHandler for CheckBalance {
    fn handler(&self, context: ContextWrapper) -> StateResult {
        let input: CheckBalanceInput = context
            .lock()
            .unwrap()
            .read("check_balance_input".into())
            .unwrap();

        let provider: Box<dyn BlockchainProvider> = context
            .lock()
            .unwrap()
            .read("blockchain_provider".into())
            .unwrap();

        let balance = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(async {
                provider
                    .get_balance(input.wallet_address, input.token_address)
                    .await
            })
            .map_err(|e| anyhow::anyhow!("Failed to get balance: {}", e))?;

        let output = CheckBalanceOutput { balance };
        context
            .lock()
            .unwrap()
            .write("check_balance_output".into(), &output)
            .unwrap();

        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, StateMetadataReqs)]
pub struct Swap {
    label: Label,
    tags: Vec<Tag>,
    depends_on: Vec<Tag>,
    depends_on_strategy: DependencyStrategy,
}

pub static SWAP: Label = Label("swap");

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwapInput {
    pub from_token: Address,
    pub to_token: Address,
    pub amount: U256,
    pub slippage: f64,
    pub wallet_address: Address,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwapOutput {
    pub transaction_hash: String,
    pub quote: SwapQuote,
}

impl Swap {
    pub fn new(
        tags: Vec<Tag>,
        depends_on: Vec<Tag>,
        depends_on_strategy: DependencyStrategy,
    ) -> Self {
        Self {
            label: SWAP,
            tags,
            depends_on,
            depends_on_strategy,
        }
    }
}

impl Default for Swap {
    fn default() -> Self {
        Self {
            label: SWAP,
            tags: vec![Tag::new("blockchain").unwrap()],
            depends_on: vec![Tag::new("setup").unwrap()],
            depends_on_strategy: DependencyStrategy::Latest,
        }
    }
}

impl StateHandler for Swap {
    fn handler(&self, context: ContextWrapper) -> StateResult {
        let input: SwapInput = context.lock().unwrap().read("swap_input".into()).unwrap();

        let dex_provider: Box<dyn DexProvider> =
            context.lock().unwrap().read("dex_provider".into()).unwrap();

        let blockchain_provider: Box<dyn BlockchainProvider> = context
            .lock()
            .unwrap()
            .read("blockchain_provider".into())
            .unwrap();

        let runtime = tokio::runtime::Runtime::new().unwrap();

        // Get quote
        let quote = runtime
            .block_on(async {
                dex_provider
                    .get_quote(input.from_token, input.to_token, input.amount)
                    .await
            })
            .map_err(|e| anyhow::anyhow!("Failed to get quote: {}", e))?;

        // Calculate minimum amount out with slippage
        let min_amount_out =
            quote.to_amount * (U256::from((10000 - (input.slippage * 100.0) as u64) * 100) / 10000);

        // Check allowance and approve if needed
        let allowance = runtime
            .block_on(async {
                blockchain_provider
                    .get_allowance(
                        input.from_token,
                        input.wallet_address,
                        dex_provider.get_router_address(),
                    )
                    .await
            })
            .map_err(|e| anyhow::anyhow!("Failed to get allowance: {}", e))?;

        if allowance < input.amount {
            runtime
                .block_on(async {
                    blockchain_provider
                        .approve(
                            input.from_token,
                            dex_provider.get_router_address(),
                            input.amount,
                        )
                        .await
                })
                .map_err(|e| anyhow::anyhow!("Failed to approve token: {}", e))?;
        }

        // Execute swap
        let transaction_hash = runtime
            .block_on(async {
                dex_provider
                    .execute_swap(
                        input.from_token,
                        input.to_token,
                        input.amount,
                        min_amount_out,
                        input.wallet_address,
                    )
                    .await
            })
            .map_err(|e| anyhow::anyhow!("Failed to execute swap: {}", e))?;

        let output = SwapOutput {
            transaction_hash,
            quote,
        };

        context
            .lock()
            .unwrap()
            .write("swap_output".into(), &output)
            .unwrap();

        Ok(())
    }
}
