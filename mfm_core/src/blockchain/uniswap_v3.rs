use crate::blockchain::{
    dex::{DexError, DexProvider, SwapQuote},
    evm::BlockchainError,
};
use ethers::{
    prelude::*,
    types::{Address, H256, U256},
};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

const FACTORY_ADDRESS: &str = "0x1F98431c8aD98523631AE4a59f267346ea31F984";
const ROUTER_ADDRESS: &str = "0xE592427A0AEce92De3Edee1F18E0157C05861564";
const QUOTER_ADDRESS: &str = "0xb27308f9F90D607463bb33eA1BeBb41C27CE5AB6";

// ERC20 ABI for token approval
abigen!(
    IERC20,
    r#"[
        function approve(address spender, uint256 amount) external returns (bool)
        function allowance(address owner, address spender) external view returns (uint256)
        function balanceOf(address account) external view returns (uint256)
    ]"#
);

// Uniswap V3 Factory ABI
abigen!(
    IUniswapV3Factory,
    r#"[
        function getPool(address tokenA, address tokenB, uint24 fee) external view returns (address pool)
    ]"#
);

// Uniswap V3 Router ABI for swap parameters
abigen!(
    ISwapRouter,
    r#"[{
        "inputs": [{
            "components": [{
                "internalType": "address",
                "name": "tokenIn",
                "type": "address"
            }, {
                "internalType": "address",
                "name": "tokenOut",
                "type": "address"
            }, {
                "internalType": "uint24",
                "name": "fee",
                "type": "uint24"
            }, {
                "internalType": "address",
                "name": "recipient",
                "type": "address"
            }, {
                "internalType": "uint256",
                "name": "deadline",
                "type": "uint256"
            }, {
                "internalType": "uint256",
                "name": "amountIn",
                "type": "uint256"
            }, {
                "internalType": "uint256",
                "name": "amountOutMinimum",
                "type": "uint256"
            }, {
                "internalType": "uint160",
                "name": "sqrtPriceLimitX96",
                "type": "uint160"
            }],
            "internalType": "struct ISwapRouter.ExactInputSingleParams",
            "name": "params",
            "type": "tuple"
        }],
        "name": "exactInputSingle",
        "outputs": [{
            "internalType": "uint256",
            "name": "amountOut",
            "type": "uint256"
        }],
        "stateMutability": "payable",
        "type": "function"
    }]"#
);

// Uniswap V3 Quoter ABI
abigen!(
    IQuoter,
    r#"[{
        "inputs": [
            {"name": "tokenIn", "type": "address"},
            {"name": "tokenOut", "type": "address"},
            {"name": "fee", "type": "uint24"},
            {"name": "amountIn", "type": "uint256"},
            {"name": "sqrtPriceLimitX96", "type": "uint160"}
        ],
        "name": "quoteExactInputSingle",
        "outputs": [{"name": "amountOut", "type": "uint256"}],
        "stateMutability": "nonpayable",
        "type": "function"
    }]"#
);

// WETH9 ABI for wrapping ETH
abigen!(
    IWETH9,
    r#"[
        function deposit() external payable
        function withdraw(uint wad) external
    ]"#
);

#[derive(Debug, Clone)]
pub struct UniswapV3Provider {
    pub provider: Arc<Provider<Http>>,
    #[allow(dead_code)]
    pub chain_id: u64,
    pub signer: Option<LocalWallet>,
    #[allow(dead_code)]
    factory: Address,
    router: Address,
    #[allow(dead_code)]
    quoter: Address,
    weth_address: Option<Address>,
}

impl From<BlockchainError> for DexError {
    fn from(error: BlockchainError) -> Self {
        DexError::SwapError(error.to_string())
    }
}

impl UniswapV3Provider {
    pub fn new(provider: Arc<Provider<Http>>, chain_id: u64, signer: Option<LocalWallet>) -> Self {
        Self {
            provider,
            chain_id,
            signer,
            factory: FACTORY_ADDRESS.parse().unwrap(),
            router: ROUTER_ADDRESS.parse().unwrap(),
            quoter: QUOTER_ADDRESS.parse().unwrap(),
            weth_address: None,
        }
    }

    pub fn with_signer(mut self, signer: LocalWallet) -> Self {
        self.signer = Some(signer.clone());
        self
    }

    async fn execute_swap_with_params(
        &self,
        params: ExactInputSingleParams,
        signer: &LocalWallet,
    ) -> Result<H256, DexError> {
        let router = ISwapRouter::new(
            self.router,
            Arc::new(SignerMiddleware::new(self.provider.clone(), signer.clone())),
        );

        let contract_call = router.exact_input_single(params);
        let pending_tx = contract_call
            .send()
            .await
            .map_err(|e| DexError::SwapError(e.to_string()))?;

        let receipt = pending_tx
            .await
            .map_err(|e| DexError::SwapError(e.to_string()))?;

        Ok(receipt.unwrap().transaction_hash)
    }

    #[allow(dead_code)]
    async fn check_and_approve_token_internal(
        &self,
        token: Address,
        spender: Address,
        amount: U256,
        signer: &LocalWallet,
    ) -> Result<(), DexError> {
        let token_contract = IERC20::new(
            token,
            Arc::new(SignerMiddleware::new(self.provider.clone(), signer.clone())),
        );

        let current_allowance = token_contract
            .allowance(signer.address(), spender)
            .call()
            .await
            .map_err(|e| DexError::TokenApprovalError(e.to_string()))?;

        if current_allowance < amount {
            let contract_call = token_contract.approve(spender, amount);
            let pending_tx = contract_call
                .send()
                .await
                .map_err(|e| DexError::TokenApprovalError(e.to_string()))?;

            pending_tx
                .await
                .map_err(|e| DexError::TokenApprovalError(e.to_string()))?;
        }

        Ok(())
    }

    async fn wrap_eth_to_weth(&self, amount: U256) -> Result<(), DexError> {
        let signer = self.signer.as_ref().ok_or(DexError::NoSignerConfigured)?;
        let weth_address = self.weth_address.ok_or(DexError::MissingWethAddress)?;

        let weth_contract = IWETH9::new(
            weth_address,
            Arc::new(SignerMiddleware::new(self.provider.clone(), signer.clone())),
        );

        // Create the call
        let call = weth_contract.deposit();

        // Store the call with value in a variable to extend its lifetime
        let call_with_value = call.value(amount);

        let pending_tx = call_with_value
            .send()
            .await
            .map_err(|e| DexError::SwapError(e.to_string()))?;

        let receipt_opt = pending_tx
            .await
            .map_err(|e| DexError::SwapError(e.to_string()))?;

        if let Some(receipt) = receipt_opt {
            if receipt.status.unwrap_or_default().as_u64() == 0 {
                return Err(DexError::SwapError("ETH to WETH wrap failed".to_string()));
            }
        }

        Ok(())
    }
}

#[async_trait::async_trait]
impl DexProvider for UniswapV3Provider {
    async fn get_quote(
        &self,
        from_token: Address,
        to_token: Address,
        amount: U256,
    ) -> Result<SwapQuote, DexError> {
        let quoter = IQuoter::new(self.quoter, Arc::new(self.provider.clone()));

        let amount_out = quoter
            .quote_exact_input_single(
                from_token,
                to_token,
                3000, // Default fee tier (0.3%)
                amount,
                U256::zero(), // No price limit
            )
            .call()
            .await
            .map_err(|e| DexError::SwapError(e.to_string()))?;

        Ok(SwapQuote {
            from_token,
            to_token,
            from_amount: amount,
            to_amount: amount_out,
            price_impact: 0.0, // TODO: Calculate price impact
            route: vec![from_token, to_token],
        })
    }

    async fn execute_swap(
        &self,
        from_token: Address,
        to_token: Address,
        amount: U256,
        _exact_approval: bool,
    ) -> Result<H256, DexError> {
        // Get signer
        let signer = self
            .signer
            .as_ref()
            .ok_or(DexError::NoSignerConfigured)?
            .clone();

        // Check if we're swapping from ETH to something else
        let weth_address = "0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2"
            .parse::<Address>()
            .unwrap();

        // If from_token is WETH, check if we need to wrap ETH first
        if from_token == weth_address {
            // Check WETH balance
            let token_contract = IERC20::new(
                from_token,
                Arc::new(SignerMiddleware::new(self.provider.clone(), signer.clone())),
            );

            let weth_balance = token_contract
                .balance_of(signer.address())
                .call()
                .await
                .map_err(|e| DexError::SwapError(e.to_string()))?;

            eprintln!("Current WETH balance: {}", weth_balance);

            if weth_balance < amount {
                // We need to wrap some ETH
                self.wrap_eth_to_weth(amount).await?;
            }
        }

        // Check and approve token if needed
        self.check_and_approve_token(from_token, amount).await?;

        // Get quote for minimum amount out
        let quote = self.get_quote(from_token, to_token, amount).await?;

        let params = ExactInputSingleParams {
            token_in: from_token,
            token_out: to_token,
            fee: 3000, // Default fee tier (0.3%)
            recipient: signer.address(),
            deadline: U256::from(
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_secs()
                    + 300,
            ), // 5 minutes from now
            amount_in: amount,
            amount_out_minimum: quote.to_amount, // TODO: Add slippage tolerance
            sqrt_price_limit_x96: U256::zero(),
        };

        // Execute the swap using the signer
        self.execute_swap_with_params(params, &signer).await
    }

    async fn check_and_approve_token(&self, token: Address, amount: U256) -> Result<(), DexError> {
        let signer = self
            .signer
            .as_ref()
            .ok_or(DexError::NoSignerConfigured)?
            .clone();

        // First check if we need to approve
        let token_contract = IERC20::new(
            token,
            Arc::new(SignerMiddleware::new(self.provider.clone(), signer.clone())),
        );

        let current_allowance = token_contract
            .allowance(signer.address(), self.router)
            .call()
            .await
            .map_err(|e| DexError::TokenApprovalError(e.to_string()))?;

        if current_allowance < amount {
            eprintln!("Approving token {} for spending", token);
            eprintln!("Current allowance: {}", current_allowance);
            eprintln!("Required amount: {}", amount);

            // Approve the token for spending
            let approve_call = token_contract.approve(self.router, amount);
            let pending_tx = approve_call
                .send()
                .await
                .map_err(|e| DexError::TokenApprovalError(e.to_string()))?;

            let receipt = pending_tx
                .await
                .map_err(|e| DexError::TokenApprovalError(e.to_string()))?;

            eprintln!(
                "Token approved successfully: {:?}",
                receipt.unwrap().transaction_hash
            );
        } else {
            eprintln!("Token already approved for spending");
            eprintln!("Current allowance: {}", current_allowance);
            eprintln!("Required amount: {}", amount);
        }

        Ok(())
    }
}
