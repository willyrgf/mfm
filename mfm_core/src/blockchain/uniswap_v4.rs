use crate::blockchain::{
    abi::uniswap_v4::*,
    dex::{self, DexError, DexProvider},
    evm::BlockchainError,
};
use ethers::{
    prelude::*,
    signers::{LocalWallet, Signer as EthersSigner},
    types::{Address, H256, I256, U256},
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

const FACTORY_ADDRESS: &str = "0x1F98431c8aD98523631AE4a59f267346ea31F984";
const ROUTER_ADDRESS: &str = "0xE592427A0AEce92De3Edee1F18E0157C05861564";
const QUOTER_ADDRESS: &str = "0xb27308f9F90D607463bb33eA1BeBb41C27CE5AB6";

const POOL_MANAGER_ADDRESS: &str = "0x8f21D1778C8E27E60707dF4997C1C7E9c8e4E1d5"; // Uniswap V4 Pool Manager on Sepolia

// ERC20 ABI for token approval
abigen!(
    IERC20,
    r#"[
        function approve(address spender, uint256 amount) external returns (bool)
        function allowance(address owner, address spender) external view returns (uint256)
    ]"#
);

#[derive(Debug, Clone)]
pub struct UniswapV4Provider {
    pub provider: Arc<Provider<Http>>,
    pub chain_id: u64,
    pub signer: Option<LocalWallet>,
    pool_manager: Option<IPoolManager<SignerMiddleware<Provider<Http>, LocalWallet>>>,
}

impl UniswapV4Provider {
    pub fn new(provider: Arc<Provider<Http>>, chain_id: u64, signer: Option<LocalWallet>) -> Self {
        let pool_manager = signer.as_ref().map(|s| {
            let client = SignerMiddleware::new(provider.as_ref().clone(), s.clone());
            IPoolManager::new(
                POOL_MANAGER_ADDRESS.parse::<Address>().unwrap(),
                Arc::new(client),
            )
        });

        Self {
            provider,
            chain_id,
            signer,
            pool_manager,
        }
    }

    pub fn with_signer(mut self, signer: LocalWallet) -> Self {
        self.signer = Some(signer.clone());
        // Create a new pool manager instance with the signer
        let client = SignerMiddleware::new(self.provider.as_ref().clone(), signer);
        self.pool_manager = Some(IPoolManager::new(
            POOL_MANAGER_ADDRESS.parse::<Address>().unwrap(),
            Arc::new(client),
        ));
        self
    }

    async fn create_pool_key(
        &self,
        token0: Address,
        token1: Address,
        fee: u32,
    ) -> Result<i_pool_manager::PoolKey, BlockchainError> {
        // Sort tokens to match Uniswap's ordering
        let (currency0, currency1) = if token0 < token1 {
            (token0, token1)
        } else {
            (token1, token0)
        };

        Ok(i_pool_manager::PoolKey {
            currency_0: currency0,
            currency_1: currency1,
            fee: fee as u32,
            tick_spacing: 60,       // Default tick spacing
            hooks: Address::zero(), // No hooks by default
        })
    }

    async fn execute_swap_with_pool_manager(
        &self,
        pool_key: i_pool_manager::PoolKey,
        params: i_pool_manager::SwapParams,
        _wallet_address: Address,
    ) -> Result<String, DexError> {
        if let Some(pool_manager) = &self.pool_manager {
            // Create the transaction
            let tx_call = pool_manager.swap(pool_key, params);

            // Execute the swap through the pool manager
            let pending_tx = tx_call
                .send()
                .await
                .map_err(|e| DexError::SwapError(e.to_string()))?;

            Ok(format!("{:?}", pending_tx.tx_hash()))
        } else {
            Err(DexError::SwapError(
                "No pool manager configured".to_string(),
            ))
        }
    }
}

#[async_trait::async_trait]
impl DexProvider for UniswapV4Provider {
    async fn get_quote(
        &self,
        from_token: Address,
        to_token: Address,
        amount: U256,
    ) -> Result<dex::SwapQuote, DexError> {
        // TODO: Implement quote functionality
        Ok(dex::SwapQuote {
            from_token,
            to_token,
            from_amount: amount,
            to_amount: amount, // TODO: Calculate actual output amount
            price_impact: 0.0,
            route: vec![from_token, to_token],
        })
    }

    async fn execute_swap(
        &self,
        from_token: Address,
        to_token: Address,
        amount: U256,
        exact_approval: bool,
    ) -> Result<H256, DexError> {
        let signer = self.signer.as_ref().ok_or(DexError::NoSignerConfigured)?;
        let wallet_address = signer.address();

        // Check and approve token if needed
        if exact_approval {
            self.check_and_approve_token(from_token, amount).await?;
        } else {
            self.check_and_approve_token(from_token, U256::MAX).await?;
        }

        // Get quote for minimum amount out
        let quote = self.get_quote(from_token, to_token, amount).await?;
        let min_amount_out = quote.to_amount; // TODO: Add slippage tolerance

        // Execute swap using pool manager
        let pool_manager = self.pool_manager.as_ref().ok_or(DexError::SwapError(
            "Pool manager not configured".to_string(),
        ))?;

        let pool_key = self
            .create_pool_key(from_token, to_token, 3000)
            .await
            .map_err(|e| DexError::SwapError(e.to_string()))?;

        let params = i_pool_manager::SwapParams {
            zero_for_one: from_token < to_token,
            amount_specified: I256::from_raw(amount),
            sqrt_price_limit_x96: U256::zero(),
        };

        let swap_call = pool_manager.swap(pool_key, params);
        let pending_tx = swap_call
            .send()
            .await
            .map_err(|e| DexError::SwapError(e.to_string()))?;

        let receipt = pending_tx
            .await
            .map_err(|e| DexError::SwapError(e.to_string()))?;
        Ok(receipt.unwrap().transaction_hash)
    }

    async fn check_and_approve_token(&self, token: Address, amount: U256) -> Result<(), DexError> {
        let signer = self.signer.as_ref().ok_or(DexError::NoSignerConfigured)?;
        let pool_manager = self.pool_manager.as_ref().ok_or(DexError::SwapError(
            "Pool manager not configured".to_string(),
        ))?;

        // Create ERC20 contract instance with SignerMiddleware
        let token_contract = IERC20::new(
            token,
            Arc::new(SignerMiddleware::new(
                self.provider.as_ref().clone(),
                signer.clone(),
            )),
        );

        // Check current allowance
        let current_allowance = token_contract
            .allowance(signer.address(), pool_manager.address())
            .call()
            .await
            .map_err(|e| DexError::TokenApprovalError(e.to_string()))?;

        // If current allowance is less than required amount, approve
        if current_allowance < amount {
            let approve_call = token_contract.approve(pool_manager.address(), amount);
            let pending_tx = approve_call
                .send()
                .await
                .map_err(|e| DexError::TokenApprovalError(e.to_string()))?;

            let _receipt = pending_tx
                .await
                .map_err(|e| DexError::TokenApprovalError(e.to_string()))?;
        }

        Ok(())
    }
}
