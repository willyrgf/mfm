//! Aave module for lending protocol interactions
//! This file contains placeholder implementations that will be fully implemented later

use crate::blockchain::adapter::{Address, Bytes, U256};
use crate::blockchain::BlockchainError;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use tokio::time::sleep;

// Define function selectors for Aave V3 protocol
// Lending Pool functions
const GET_USER_ACCOUNT_DATA_SELECTOR: [u8; 4] = [0x62, 0x89, 0x19, 0xd9]; // keccak256("getUserAccountData(address)")[0..4]
#[allow(dead_code)]
const WITHDRAW_SELECTOR: [u8; 4] = [0x69, 0x32, 0x8f, 0x2e]; // keccak256("withdraw(address,uint256,address)")[0..4]
#[allow(dead_code)]
const SUPPLY_SELECTOR: [u8; 4] = [0x61, 0x7b, 0xa0, 0x37]; // keccak256("supply(address,uint256,address,uint16)")[0..4]
#[allow(dead_code)]
const BORROW_SELECTOR: [u8; 4] = [0xa4, 0x15, 0xbc, 0xad]; // keccak256("borrow(address,uint256,uint256,uint16,address)")[0..4]
#[allow(dead_code)]
const REPAY_SELECTOR: [u8; 4] = [0x57, 0x3a, 0xed, 0xa8]; // keccak256("repay(address,uint256,uint256,address)")[0..4]

// Protocol Data Provider functions
#[allow(dead_code)]
const GET_RESERVE_DATA_SELECTOR: [u8; 4] = [0x35, 0xea, 0x6a, 0x75]; // keccak256("getReserveData(address)")[0..4]
#[allow(dead_code)]
const GET_USER_RESERVE_DATA_SELECTOR: [u8; 4] = [0x28, 0xdd, 0x2d, 0x01]; // keccak256("getUserReserveData(address,address)")[0..4]

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

// We use String representations for serialization
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AaveUserAccountData {
    pub total_collateral_base: String,
    pub total_debt_base: String,
    pub available_borrow_base: String,
    pub current_liquidation_threshold: String,
    pub ltv: String,
    pub health_factor: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AaveReserveData {
    pub asset: String,
    pub symbol: String,
    pub liquidation_threshold: u16,
    pub ltv: u16,
    pub decimals: u8,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AaveUserReserveData {
    pub asset: String,
    pub symbol: String,
    pub current_atoken_balance: String,
    pub current_stable_debt: String,
    pub current_variable_debt: String,
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
    #[allow(dead_code)]
    async fn get_user_reserves(
        &self,
        user: Address,
    ) -> Result<Vec<AaveUserReserveData>, BlockchainError>;

    /// calculate health factor
    #[allow(dead_code)]
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

/// helper function to convert U256 to f64 with decimals
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
}

#[async_trait]
impl AaveProvider for AaveAdapterProvider {
    /// get user account data from aave
    async fn get_user_account_data(
        &self,
        user: Address,
    ) -> Result<AaveUserAccountData, BlockchainError> {
        // Wait for rate limiter
        let mut rate_limiter = self.rate_limiter.lock().await;
        rate_limiter.wait().await;
        drop(rate_limiter);

        // Prepare the call data
        let mut call_data = GET_USER_ACCOUNT_DATA_SELECTOR.to_vec();

        // Pad the address to 32 bytes (EVM ABI encoding)
        let mut address_bytes = vec![0u8; 12]; // 12 zeros for padding
        address_bytes.extend_from_slice(user.0.as_ref());
        call_data.extend_from_slice(&address_bytes);

        // Call the lending pool contract
        let result = self
            .provider
            .call(self.lending_pool_address, Bytes(call_data))
            .await
            .map_err(|e| BlockchainError::ContractCallError(e.to_string()))?;

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

        // Create the user account data struct
        Ok(AaveUserAccountData {
            total_collateral_base: total_collateral_base.0.to_string(),
            total_debt_base: total_debt_base.0.to_string(),
            available_borrow_base: available_borrow_base.0.to_string(),
            current_liquidation_threshold: current_liquidation_threshold.0.to_string(),
            ltv: ltv.0.to_string(),
            health_factor: health_factor.0.to_string(),
        })
    }

    /// get user reserves data from aave
    async fn get_user_reserves(
        &self,
        user: Address,
    ) -> Result<Vec<AaveUserReserveData>, BlockchainError> {
        // This is a complex implementation that would require multiple steps
        // 1. Get all reserve tokens from Aave
        // 2. For each token, call getUserReserveData
        // 3. Fetch prices for each token
        // 4. Combine data into AaveUserReserveData objects

        // For now, we'll return an empty list to enable partial functionality
        // A full implementation would be very lengthy for this example

        // Wait for rate limiter
        let mut rate_limiter = self.rate_limiter.lock().await;
        rate_limiter.wait().await;
        drop(rate_limiter);

        // Just log the user address we're checking
        println!("User reserves would be fetched for address: {}", user);

        // Return empty list for now - will be fully implemented later
        // This allows framework to work without erroring
        Ok(Vec::new())
    }

    /// calculate health factor for a user
    async fn calculate_health_factor(
        &self,
        user: Address,
    ) -> Result<AaveHealthCheckResult, BlockchainError> {
        // Get the user account data first
        let account_data = self.get_user_account_data(user).await?;

        // Parse the health factor
        let health_factor_str = &account_data.health_factor;
        let health_factor_value = health_factor_str
            .parse::<f64>()
            .map_err(|e| BlockchainError::ContractError(e.to_string()))?;

        // Parse collateral and debt
        let collateral_str = &account_data.total_collateral_base;
        let collateral_value = collateral_str
            .parse::<f64>()
            .map_err(|e| BlockchainError::ContractError(e.to_string()))?;

        let debt_str = &account_data.total_debt_base;
        let debt_value = debt_str
            .parse::<f64>()
            .map_err(|e| BlockchainError::ContractError(e.to_string()))?;

        // Calculate max decrease percentage
        // This is a simple calculation - for a full implementation we would need more data
        let max_decrease_percentage = if health_factor_value > 1.0 {
            (health_factor_value - 1.0) / health_factor_value * 100.0
        } else {
            0.0
        };

        // Return the health check result with empty reserves for now
        Ok(AaveHealthCheckResult {
            health_factor: health_factor_value,
            total_collateral_usd: collateral_value,
            total_debt_usd: debt_value,
            user_reserves: Vec::new(), // Empty for now
            max_decrease_percentage,
        })
    }
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
