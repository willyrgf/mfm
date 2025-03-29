//! Aave module for lending protocol interactions
//! This file contains placeholder implementations that will be fully implemented later

use crate::blockchain::adapter::{Address, Bytes, U256};
use crate::blockchain::BlockchainError;
use async_trait::async_trait;
use lazy_static;
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tiny_keccak::{Hasher, Keccak};
use tokio::sync::Mutex;
use tokio::time::sleep;

/// Compute a function selector from its signature
///
/// Takes a function signature string and computes the first 4 bytes
/// of its keccak256 hash, which is the standard function selector in Ethereum
fn compute_selector(signature: &str) -> [u8; 4] {
    let mut selector = [0u8; 4];
    let mut hasher = Keccak::v256();
    hasher.update(signature.as_bytes());
    let mut hash = [0u8; 32];
    hasher.finalize(&mut hash);
    selector.copy_from_slice(&hash[0..4]);
    selector
}

// Define function selectors for Aave V3 protocol
// Lending Pool functions - these are now readable by showing the actual function signatures
lazy_static::lazy_static! {
    static ref GET_USER_ACCOUNT_DATA_SELECTOR: [u8; 4] = compute_selector("getUserAccountData(address)");
    #[allow(dead_code)]
    static ref WITHDRAW_SELECTOR: [u8; 4] = compute_selector("withdraw(address,uint256,address)");
    #[allow(dead_code)]
    static ref SUPPLY_SELECTOR: [u8; 4] = compute_selector("supply(address,uint256,address,uint16)");
    #[allow(dead_code)]
    static ref BORROW_SELECTOR: [u8; 4] = compute_selector("borrow(address,uint256,uint256,uint16,address)");
    #[allow(dead_code)]
    static ref REPAY_SELECTOR: [u8; 4] = compute_selector("repay(address,uint256,uint256,address)");

    // Protocol Data Provider functions
    #[allow(dead_code)]
    static ref GET_RESERVE_DATA_SELECTOR: [u8; 4] = compute_selector("getReserveData(address)");
    #[allow(dead_code)]
    static ref GET_USER_RESERVE_DATA_SELECTOR: [u8; 4] = compute_selector("getUserReserveData(address,address)");
    #[allow(dead_code)]
    static ref SYMBOL_SELECTOR: [u8; 4] = compute_selector("symbol()");
    #[allow(dead_code)]
    static ref DECIMALS_SELECTOR: [u8; 4] = compute_selector("decimals()");
}

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

// We directly use our adapter U256 for cleaner code
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AaveUserAccountData {
    pub total_collateral_base: U256,
    pub total_debt_base: U256,
    pub available_borrow_base: U256,
    pub current_liquidation_threshold: U256,
    pub ltv: U256,
    pub health_factor: U256,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AaveReserveData {
    pub asset: Address,
    pub symbol: String,
    pub liquidation_threshold: u16,
    pub ltv: u16,
    pub decimals: u8,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AaveUserReserveData {
    pub asset: Address,
    pub symbol: String,
    pub current_atoken_balance: U256,
    pub current_stable_debt: U256,
    pub current_variable_debt: U256,
    pub liquidation_threshold: u16,
    pub price_usd: f64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AaveHealthCheckResult {
    pub health_factor: f64,
    pub total_collateral_usd: f64,
    pub total_debt_usd: f64,
    pub user_reserves: Vec<AaveUserReserveData>,
    pub max_decrease_percentage: f64,
}

/// trait for aave protocol interactions
#[async_trait]
pub trait AaveProvider: Send + Sync {
    /// get user account data
    async fn get_user_account_data(
        &self,
        user: Address,
    ) -> Result<AaveUserAccountData, BlockchainError>;

    /// get user reserve data
    async fn get_user_reserves(
        &self,
        user: Address,
    ) -> Result<Vec<AaveUserReserveData>, BlockchainError>;

    /// calculate health factor
    async fn calculate_health_factor(
        &self,
        user: Address,
    ) -> Result<AaveHealthCheckResult, BlockchainError>;
}

/// function to fetch price from coingecko
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

/// Helper function to convert U256 to f64 with decimals
pub fn u256_to_f64(value: U256, decimals: u8) -> f64 {
    let ten_pow_decimals =
        alloy_primitives::U256::from(10).pow(alloy_primitives::U256::from(decimals as u64));

    let integer_part = value.0.checked_div(ten_pow_decimals).unwrap_or_default();
    let fractional_part = value.0.checked_rem(ten_pow_decimals).unwrap_or_default();

    let integer_f64 = integer_part.to_string().parse::<f64>().unwrap_or_default();
    let fractional_f64 = fractional_part
        .to_string()
        .parse::<f64>()
        .unwrap_or_default()
        / 10f64.powi(decimals as i32);

    integer_f64 + fractional_f64
}

/// Helper function to get price with retry
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

/// aave provider implementation using our adapter
#[derive(Clone)]
pub struct AaveAdapterProvider {
    provider: Arc<crate::blockchain::adapter::Provider>,
    lending_pool_address: Address,
    data_provider_address: Address,
    rate_limiter: Arc<Mutex<RateLimiter>>,
}

impl std::fmt::Debug for AaveAdapterProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AaveAdapterProvider")
            .field("lending_pool_address", &self.lending_pool_address)
            .field("data_provider_address", &self.data_provider_address)
            .finish_non_exhaustive()
    }
}

/// rate limiter to prevent api rate limits
#[derive(Debug, Clone)]
struct RateLimiter {
    last_call: Option<Instant>,
    min_interval: Duration,
}

impl RateLimiter {
    /// create a new rate limiter with min interval between calls
    fn new(min_interval_ms: u64) -> Self {
        Self {
            last_call: None,
            min_interval: Duration::from_millis(min_interval_ms),
        }
    }

    /// wait until it's safe to make another call
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

impl AaveAdapterProvider {
    /// create a new aave provider
    pub fn new(
        provider: Arc<crate::blockchain::adapter::Provider>,
        lending_pool_address: Address,
        data_provider_address: Address,
    ) -> Self {
        Self {
            provider,
            lending_pool_address,
            data_provider_address,
            rate_limiter: Arc::new(Mutex::new(RateLimiter::new(200))), // 200ms between calls
        }
    }

    /// Call a contract function safely with rate limiting
    async fn call_contract(
        &self,
        address: Address,
        data: Vec<u8>,
    ) -> Result<Bytes, BlockchainError> {
        // Apply rate limiting
        {
            let mut rate_limiter = self.rate_limiter.lock().await;
            rate_limiter.wait().await;
        }

        // Call the contract
        self.provider
            .call(address, Bytes(data))
            .await
            .map_err(|e| BlockchainError::ContractCallError(e.to_string()))
    }

    /// Get token symbol
    #[allow(dead_code)]
    async fn get_token_symbol(&self, token_address: Address) -> Result<String, BlockchainError> {
        let call_data = (*SYMBOL_SELECTOR).to_vec();

        match self.call_contract(token_address, call_data).await {
            Ok(result) => {
                // The result is an ABI-encoded string, usually starting at offset 32
                if result.0.len() < 64 {
                    return Err(BlockchainError::ContractCallError(
                        "Invalid response length for symbol".to_string(),
                    ));
                }

                // Extract string data from the ABI encoding
                let length_bytes = &result.0[32..64];
                let length = u32::from_be_bytes([
                    length_bytes[28],
                    length_bytes[29],
                    length_bytes[30],
                    length_bytes[31],
                ]) as usize;

                if result.0.len() < 64 + length {
                    return Err(BlockchainError::ContractCallError(
                        "Invalid string length for symbol".to_string(),
                    ));
                }

                let symbol_bytes = &result.0[64..64 + length];
                let symbol = String::from_utf8(symbol_bytes.to_vec()).map_err(|e| {
                    BlockchainError::ContractCallError(format!("Invalid UTF-8 in symbol: {}", e))
                })?;

                Ok(symbol)
            }
            Err(e) => Err(e),
        }
    }

    /// Get token decimals
    #[allow(dead_code)]
    async fn get_token_decimals(&self, token_address: Address) -> Result<u8, BlockchainError> {
        let call_data = (*DECIMALS_SELECTOR).to_vec();

        match self.call_contract(token_address, call_data).await {
            Ok(result) => {
                if result.0.len() < 32 {
                    return Err(BlockchainError::ContractCallError(
                        "Invalid response length for decimals".to_string(),
                    ));
                }

                // Decimals is a uint8, extract just the relevant byte
                Ok(result.0[31])
            }
            Err(e) => Err(e),
        }
    }
}

#[async_trait]
impl AaveProvider for AaveAdapterProvider {
    /// get user account data from aave
    async fn get_user_account_data(
        &self,
        user: Address,
    ) -> Result<AaveUserAccountData, BlockchainError> {
        // Prepare the call data
        let mut call_data = (*GET_USER_ACCOUNT_DATA_SELECTOR).to_vec();

        // Pad the address to 32 bytes (EVM ABI encoding)
        let mut address_bytes = vec![0u8; 12]; // 12 zeros for padding
        address_bytes.extend_from_slice(user.0.as_ref());
        call_data.extend_from_slice(&address_bytes);

        // Log the user address we're querying
        println!("Querying AAVE V3 for user: {}", user);

        // Call the lending pool contract
        let result = self
            .call_contract(self.lending_pool_address, call_data)
            .await?;

        // Decode result - this is a complex tuple with 6 uint256 values
        // (totalCollateralBase, totalDebtBase, availableBorrowsBase, currentLiquidationThreshold, ltv, healthFactor)
        if result.0.len() < 192 {
            return Err(BlockchainError::ContractCallError(
                "Invalid response length from getUserAccountData".to_string(),
            ));
        }

        // Extract each value - they're 32 bytes each, packed in order
        let total_collateral_base = parse_uint256(&result.0[0..32])?;
        let total_debt_base = parse_uint256(&result.0[32..64])?;
        let available_borrow_base = parse_uint256(&result.0[64..96])?;
        let current_liquidation_threshold = parse_uint256(&result.0[96..128])?;
        let ltv = parse_uint256(&result.0[128..160])?;
        let health_factor = parse_uint256(&result.0[160..192])?;

        // Create the user account data struct with the directly parsed U256 values
        Ok(AaveUserAccountData {
            total_collateral_base,
            total_debt_base,
            available_borrow_base,
            current_liquidation_threshold,
            ltv,
            health_factor,
        })
    }

    /// get user reserves data from aave
    async fn get_user_reserves(
        &self,
        user: Address,
    ) -> Result<Vec<AaveUserReserveData>, BlockchainError> {
        // These are common Aave assets on mainnet
        let reserve_tokens = [("USDC", "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48")];

        let mut user_reserves = Vec::new();

        for (symbol, address_str) in reserve_tokens.iter() {
            // Parse address
            let reserve_address = match Address::from_str(address_str) {
                Ok(addr) => addr,
                Err(_) => continue, // Skip invalid addresses
            };

            // Prepare the call data for getUserReserveData
            let mut call_data = (*GET_USER_RESERVE_DATA_SELECTOR).to_vec();

            // Pad the asset address to 32 bytes
            let mut asset_addr_bytes = vec![0u8; 12]; // 12 zeros for padding
            asset_addr_bytes.extend_from_slice(reserve_address.0.as_ref());
            call_data.extend_from_slice(&asset_addr_bytes);

            // Pad the user address to 32 bytes
            let mut user_addr_bytes = vec![0u8; 12]; // 12 zeros for padding
            user_addr_bytes.extend_from_slice(user.0.as_ref());
            call_data.extend_from_slice(&user_addr_bytes);

            // Call the data provider contract
            let result = match self
                .call_contract(self.data_provider_address, call_data)
                .await
            {
                Ok(result) => result,
                Err(_) => continue, // Skip failed calls
            };

            // Decode result - this is a complex tuple with multiple values
            // We're most interested in:
            // - currentATokenBalance (first value, index 0-31)
            // - currentStableDebt (second value, index 32-63)
            // - currentVariableDebt (third value, index 64-95)
            if result.0.len() < 96 {
                continue; // Skip if response is too short
            }

            // Extract the values
            let current_atoken_balance = match parse_uint256(&result.0[0..32]) {
                Ok(val) => val,
                Err(_) => continue,
            };

            let current_stable_debt = match parse_uint256(&result.0[32..64]) {
                Ok(val) => val,
                Err(_) => continue,
            };

            let current_variable_debt = match parse_uint256(&result.0[64..96]) {
                Ok(val) => val,
                Err(_) => continue,
            };

            // Get price for the token with retry
            let price = get_price_with_retry(symbol, 3).await.unwrap_or(0.0);

            // Only add reserves where the user has a balance or debt
            if current_atoken_balance.0 > alloy_primitives::U256::ZERO
                || current_stable_debt.0 > alloy_primitives::U256::ZERO
                || current_variable_debt.0 > alloy_primitives::U256::ZERO
            {
                user_reserves.push(AaveUserReserveData {
                    asset: reserve_address,
                    symbol: symbol.to_string(),
                    current_atoken_balance,
                    current_stable_debt,
                    current_variable_debt,
                    liquidation_threshold: 8000, // Default to 80%
                    price_usd: price,
                });
            }
        }

        Ok(user_reserves)
    }

    /// calculate health factor for a user
    async fn calculate_health_factor(
        &self,
        user: Address,
    ) -> Result<AaveHealthCheckResult, BlockchainError> {
        // Get the user account data first
        let account_data = self.get_user_account_data(user).await?;

        // If user has no collateral and no debt, return a default health factor result
        if account_data.total_collateral_base.0 == alloy_primitives::U256::ZERO
            && account_data.total_debt_base.0 == alloy_primitives::U256::ZERO
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

        // Convert to f64 for calculations
        let health_factor = u256_to_f64(account_data.health_factor, 18);
        let total_collateral_usd = u256_to_f64(account_data.total_collateral_base, 8);
        let total_debt_usd = u256_to_f64(account_data.total_debt_base, 8);

        // Calculate maximum percentage decrease before health factor drops to 1
        let max_decrease_percentage = if health_factor.is_infinite() || health_factor.is_nan() {
            100.0
        } else if health_factor > 1.0 {
            100.0 * (1.0 - 1.0 / health_factor)
        } else {
            0.0
        };

        // Get detailed reserve data
        let user_reserves = self.get_user_reserves(user).await?;

        // Return the health check result
        Ok(AaveHealthCheckResult {
            health_factor,
            total_collateral_usd,
            total_debt_usd,
            user_reserves,
            max_decrease_percentage,
        })
    }
}

/// helper function to parse a uint256 from EVM ABI encoding
fn parse_uint256(data: &[u8]) -> Result<U256, BlockchainError> {
    if data.len() != 32 {
        return Err(BlockchainError::ContractCallError(
            "Invalid data length for uint256".to_string(),
        ));
    }

    // Use alloy_primitives to parse the bytes into a U256
    let mut bytes = [0u8; 32];
    bytes.copy_from_slice(data);
    let value = alloy_primitives::U256::from_be_bytes(bytes);

    // Convert to our adapter type
    Ok(U256(value))
}

/// create an aave provider
pub async fn create_aave_provider(
    provider: Arc<crate::blockchain::adapter::Provider>,
    lending_pool_address: Address,
    data_provider_address: Address,
) -> Result<impl AaveProvider, BlockchainError> {
    Ok(AaveAdapterProvider::new(
        provider,
        lending_pool_address,
        data_provider_address,
    ))
}

/// A simple mock implementation of AaveProvider for testing
#[derive(Clone, Debug)]
pub struct MockAaveProvider {
    user_address: Address,
}

impl MockAaveProvider {
    pub fn new(user_address: Address) -> Self {
        Self { user_address }
    }
}

#[async_trait]
impl AaveProvider for MockAaveProvider {
    /// Mock implementation of get_user_account_data
    async fn get_user_account_data(
        &self,
        user: Address,
    ) -> Result<AaveUserAccountData, BlockchainError> {
        // Simple check to make it look like it's doing something
        if user != self.user_address {
            println!(
                "Mock provider: Querying for {}, expected {}",
                user, self.user_address
            );
        }

        // Return mock data
        Ok(AaveUserAccountData {
            total_collateral_base: U256(alloy_primitives::U256::from(1000000000000000000u128)), // 1 ETH
            total_debt_base: U256(alloy_primitives::U256::from(500000000000000000u128)), // 0.5 ETH
            available_borrow_base: U256(alloy_primitives::U256::from(300000000000000000u128)), // 0.3 ETH
            current_liquidation_threshold: U256(alloy_primitives::U256::from(8500u64)), // 85%
            ltv: U256(alloy_primitives::U256::from(7500u64)),                           // 75%
            health_factor: U256(alloy_primitives::U256::from(2000000000000000000u128)), // 2.0
        })
    }

    /// Mock implementation of get_user_reserves
    async fn get_user_reserves(
        &self,
        _user: Address,
    ) -> Result<Vec<AaveUserReserveData>, BlockchainError> {
        // Return mock data
        let usdc_addr = Address::from_str("0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48").unwrap();

        Ok(vec![AaveUserReserveData {
            asset: usdc_addr,
            symbol: "USDC".to_string(),
            current_atoken_balance: U256(alloy_primitives::U256::from(1000000000u128)), // 1000 USDC (with 6 decimals)
            current_stable_debt: U256(alloy_primitives::U256::from(0u64)),
            current_variable_debt: U256(alloy_primitives::U256::from(500000000u128)), // 500 USDC
            liquidation_threshold: 8000,                                              // 80%
            price_usd: 1.0,
        }])
    }

    /// Mock implementation of calculate_health_factor
    async fn calculate_health_factor(
        &self,
        user: Address,
    ) -> Result<AaveHealthCheckResult, BlockchainError> {
        // Get user account data from our mock
        let account_data = self.get_user_account_data(user).await?;

        // Convert health factor to float (it's stored as 1e18 precision)
        let health_factor = account_data.health_factor.0.as_limbs()[0] as f64 / 1e18;

        // Get mock reserves
        let reserves = self.get_user_reserves(user).await?;

        // Calculate total values in USD for display
        let total_collateral_usd = 2000.0; // $2000 worth of collateral
        let total_debt_usd = 1000.0; // $1000 worth of debt

        Ok(AaveHealthCheckResult {
            health_factor,
            total_collateral_usd,
            total_debt_usd,
            user_reserves: reserves,
            max_decrease_percentage: 50.0, // 50% decrease allowed
        })
    }
}
