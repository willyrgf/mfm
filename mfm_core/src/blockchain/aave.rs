use crate::blockchain::BlockchainError;
use async_trait::async_trait;
use ethers::{
    abi::{self, Token},
    providers::Middleware,
    types::{transaction::eip2718::TypedTransaction, Address, Bytes, U256},
    utils::keccak256,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use tokio::time::sleep;

// Asset IDs for price fetching
const ASSET_IDS: &[(&str, &str)] = &[
    ("LINK", "chainlink"),
    ("AAVE", "aave"),
    ("rETH", "rocket-pool-eth"),
    ("ETH", "ethereum"),
    ("WETH", "weth"),
    ("USDC", "usd-coin"),
    ("USDT", "tether"),
    ("DAI", "dai"),
    ("WBTC", "wrapped-bitcoin"),
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AaveUserAccountData {
    pub total_collateral_base: U256,
    pub total_debt_base: U256,
    pub available_borrow_base: U256,
    pub current_liquidation_threshold: U256,
    pub ltv: U256,
    pub health_factor: U256,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AaveReserveData {
    pub asset: Address,
    pub symbol: String,
    pub liquidation_threshold: u16,
    pub ltv: u16,
    pub decimals: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AaveUserReserveData {
    pub asset: Address,
    pub symbol: String,
    pub current_atoken_balance: U256,
    pub current_stable_debt: U256,
    pub current_variable_debt: U256,
    pub liquidation_threshold: u16,
    pub price_usd: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AaveHealthCheckResult {
    pub health_factor: f64,
    pub total_collateral_usd: f64,
    pub total_debt_usd: f64,
    pub user_reserves: Vec<AaveUserReserveData>,
    pub max_decrease_percentage: f64,
}

#[async_trait]
pub trait AaveProvider: Send + Sync {
    async fn get_user_account_data(
        &self,
        user: Address,
    ) -> Result<AaveUserAccountData, BlockchainError>;
    async fn get_user_reserves(
        &self,
        user: Address,
    ) -> Result<Vec<AaveUserReserveData>, BlockchainError>;
    async fn calculate_health_factor(
        &self,
        user: Address,
    ) -> Result<AaveHealthCheckResult, BlockchainError>;
}

// Function to fetch real-time price from CoinGecko
pub async fn get_price(asset: &str) -> Result<f64, BlockchainError> {
    let asset_id = ASSET_IDS
        .iter()
        .find(|(symbol, _)| *symbol == asset)
        .map(|(_, id)| *id)
        .unwrap_or("ethereum"); // Default to ETH if not found

    let url = format!(
        "https://api.coingecko.com/api/v3/simple/price?ids={}&vs_currencies=usd",
        asset_id
    );

    let client = reqwest::Client::new();
    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|e| BlockchainError::ApiError(format!("Failed to fetch price: {}", e)))?;

    let json: HashMap<String, HashMap<String, f64>> = response
        .json()
        .await
        .map_err(|e| BlockchainError::ApiError(format!("Failed to parse price response: {}", e)))?;

    json.get(asset_id)
        .and_then(|inner| inner.get("usd").copied())
        .ok_or_else(|| BlockchainError::ApiError(format!("Price not found for {}", asset)))
}

// Helper function to convert U256 to f64 with decimals
pub fn u256_to_f64(value: U256, decimals: u8) -> f64 {
    let divisor = U256::from(10).pow(U256::from(decimals));
    let integer_part = value / divisor;
    let fractional_part = value % divisor;

    let integer_f64 = integer_part.as_u128() as f64;
    let fractional_f64 = fractional_part.as_u128() as f64 / (10u128.pow(decimals as u32) as f64);

    integer_f64 + fractional_f64
}

// Implementation for EVM provider
#[derive(Debug, Clone)]
pub struct AaveEVMProvider<M: Middleware + 'static> {
    provider: M,
    lending_pool_address: Address,
    data_provider_address: Address,
    rate_limiter: Arc<Mutex<RateLimiter>>,
}

// Rate limiter to prevent Cloudflare blocks
#[derive(Debug, Clone)]
struct RateLimiter {
    last_call: Option<Instant>,
    min_interval: Duration,
}

impl RateLimiter {
    fn new(min_interval_ms: u64) -> Self {
        Self {
            last_call: None,
            min_interval: Duration::from_millis(min_interval_ms),
        }
    }

    async fn wait(&mut self) {
        if let Some(last_call) = self.last_call {
            let elapsed = last_call.elapsed();
            if elapsed < self.min_interval {
                let wait_time = self.min_interval.checked_sub(elapsed).unwrap_or_default();
                sleep(wait_time).await;
            }
        }
        self.last_call = Some(Instant::now());
    }
}

impl<M: Middleware + 'static> AaveEVMProvider<M> {
    pub fn new(provider: M, lending_pool_address: Address, data_provider_address: Address) -> Self {
        Self {
            provider,
            lending_pool_address,
            data_provider_address,
            rate_limiter: Arc::new(Mutex::new(RateLimiter::new(200))), // 200ms between calls
        }
    }

    async fn call_contract(&self, address: Address, data: Bytes) -> Result<Bytes, BlockchainError> {
        // Apply rate limiting
        {
            let mut rate_limiter = self.rate_limiter.lock().await;
            rate_limiter.wait().await;
        }

        // Call the contract with the provider
        let tx = ethers::types::TransactionRequest::new()
            .to(address)
            .data(data);

        // Convert to TypedTransaction for compatibility with Middleware::call
        let typed_tx: TypedTransaction = tx.into();

        match self.provider.call(&typed_tx, None).await {
            Ok(result) => Ok(result),
            Err(e) => {
                // Log the error and return it
                println!("Contract call error: {}", e);
                Err(BlockchainError::ContractError(e.to_string()))
            }
        }
    }

    async fn get_token_symbol(&self, token_address: Address) -> Result<String, BlockchainError> {
        let function_signature = "symbol()";
        let selector = &keccak256(function_signature.as_bytes())[0..4];

        match self
            .call_contract(token_address, selector.to_vec().into())
            .await
        {
            Ok(result) => {
                // Parse the result
                let tokens = abi::decode(&[abi::ParamType::String], &result).map_err(|e| {
                    BlockchainError::ContractError(format!("Failed to decode token symbol: {}", e))
                })?;

                if let Some(Token::String(symbol)) = tokens.first() {
                    Ok(symbol.clone())
                } else {
                    Err(BlockchainError::ContractError(
                        "Invalid token symbol format".to_string(),
                    ))
                }
            }
            Err(e) => Err(e),
        }
    }

    async fn get_token_decimals(&self, token_address: Address) -> Result<u8, BlockchainError> {
        let function_signature = "decimals()";
        let selector = &keccak256(function_signature.as_bytes())[0..4];

        match self
            .call_contract(token_address, selector.to_vec().into())
            .await
        {
            Ok(result) => {
                // Parse the result
                let tokens = abi::decode(&[abi::ParamType::Uint(8)], &result).map_err(|e| {
                    BlockchainError::ContractError(format!(
                        "Failed to decode token decimals: {}",
                        e
                    ))
                })?;

                if let Some(Token::Uint(decimals)) = tokens.first() {
                    Ok(decimals.as_u32() as u8)
                } else {
                    Err(BlockchainError::ContractError(
                        "Invalid token decimals format".to_string(),
                    ))
                }
            }
            Err(e) => Err(e),
        }
    }
}

#[async_trait]
impl<M: Middleware + Send + Sync + Clone + 'static> AaveProvider for AaveEVMProvider<M> {
    async fn get_user_account_data(
        &self,
        user: Address,
    ) -> Result<AaveUserAccountData, BlockchainError> {
        // Call getUserAccountData function
        let function = "getUserAccountData(address)";
        let selector = &keccak256(function.as_bytes())[0..4];
        let address_padded = abi::encode(&[Token::Address(user)]);
        let data = [selector, &address_padded].concat();

        // Log the user address we're querying
        println!("Querying AAVE V3 for user: {}", user);

        let result = match self
            .call_contract(self.lending_pool_address, data.into())
            .await
        {
            Ok(res) => res,
            Err(e) => {
                println!("Error getting user account data: {}", e);
                return Err(e);
            }
        };

        if result.len() < 192 {
            // 6 * 32 bytes
            println!("Invalid response length: {}", result.len());
            return Err(BlockchainError::ContractError(
                "Invalid response length".to_string(),
            ));
        }

        // Parse the result
        let total_collateral_base = U256::from_big_endian(&result[0..32]);
        let total_debt_base = U256::from_big_endian(&result[32..64]);
        let available_borrow_base = U256::from_big_endian(&result[64..96]);
        let current_liquidation_threshold = U256::from_big_endian(&result[96..128]);
        let ltv = U256::from_big_endian(&result[128..160]);
        let health_factor = U256::from_big_endian(&result[160..192]);

        Ok(AaveUserAccountData {
            total_collateral_base,
            total_debt_base,
            available_borrow_base,
            current_liquidation_threshold,
            ltv,
            health_factor,
        })
    }

    async fn get_user_reserves(
        &self,
        _user: Address,
    ) -> Result<Vec<AaveUserReserveData>, BlockchainError> {
        // In the simplified version, we don't need to fetch individual reserves
        // Just return an empty vector
        Ok(Vec::new())
    }

    async fn calculate_health_factor(
        &self,
        user: Address,
    ) -> Result<AaveHealthCheckResult, BlockchainError> {
        // Get user account data
        let account_data = self.get_user_account_data(user).await?;

        // If user has no collateral and no debt, return a default health factor result
        if account_data.total_collateral_base == U256::zero()
            && account_data.total_debt_base == U256::zero()
        {
            println!("User has no AAVE positions, returning default health factor");
            return Ok(AaveHealthCheckResult {
                health_factor: f64::INFINITY,
                total_collateral_usd: 0.0,
                total_debt_usd: 0.0,
                user_reserves: Vec::new(),
                max_decrease_percentage: 100.0,
            });
        }

        // Convert to f64 for easier calculations
        let health_factor = u256_to_f64(account_data.health_factor, 18);
        let total_collateral_usd = u256_to_f64(account_data.total_collateral_base, 18);
        let total_debt_usd = u256_to_f64(account_data.total_debt_base, 18);

        // Calculate maximum percentage decrease before health factor drops to 1
        let max_decrease_percentage = if health_factor.is_infinite() {
            100.0
        } else if health_factor > 1.0 {
            100.0 * (1.0 - 1.0 / health_factor)
        } else {
            0.0
        };

        Ok(AaveHealthCheckResult {
            health_factor,
            total_collateral_usd,
            total_debt_usd,
            user_reserves: Vec::new(), // We don't need individual reserves in the simplified version
            max_decrease_percentage,
        })
    }
}

impl<M: Middleware + 'static> AaveEVMProvider<M> {
    // Helper function to create AaveProvider from EvmProvider
    pub fn create_aave_provider(
        provider: M,
        lending_pool_address: Address,
        data_provider_address: Address,
    ) -> impl std::future::Future<Output = Result<AaveEVMProvider<M>, BlockchainError>> + Send {
        async move {
            Ok(AaveEVMProvider::new(
                provider,
                lending_pool_address,
                data_provider_address,
            ))
        }
    }
}

// Helper function to get price with retry
async fn get_price_with_retry(asset: &str, max_retries: usize) -> Result<f64, BlockchainError> {
    get_price_with_retry_internal(asset, 0, max_retries).await
}

async fn get_price_with_retry_internal(
    asset: &str,
    current_retry: usize,
    max_retries: usize,
) -> Result<f64, BlockchainError> {
    if current_retry > max_retries {
        return Err(BlockchainError::Other(
            "Max retries exceeded for price fetch".to_string(),
        ));
    }

    match get_price(asset).await {
        Ok(price) => Ok(price),
        Err(e) => {
            if current_retry < max_retries {
                println!(
                    "Error getting price for {}, retrying ({}/{}): {}",
                    asset,
                    current_retry + 1,
                    max_retries,
                    e
                );
                // Add exponential backoff
                sleep(Duration::from_millis(200 * (current_retry as u64 + 1))).await;
                // Use Box::pin for recursive async call
                Box::pin(get_price_with_retry_internal(
                    asset,
                    current_retry + 1,
                    max_retries,
                ))
                .await
            } else {
                Err(e)
            }
        }
    }
}

pub fn create_aave_provider<M: Middleware + Send + Sync + Clone + 'static>(
    provider: M,
    lending_pool_address: Address,
    data_provider_address: Address,
) -> impl std::future::Future<Output = Result<AaveEVMProvider<M>, BlockchainError>> + Send {
    async move {
        Ok(AaveEVMProvider::new(
            provider,
            lending_pool_address,
            data_provider_address,
        ))
    }
}
