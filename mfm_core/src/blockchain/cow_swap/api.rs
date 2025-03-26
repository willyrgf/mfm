use ethers::types::{Address, U256};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use thiserror::Error;

const API_BASE_URL: &str = "https://api.cow.fi/mainnet/api/v1";

#[derive(Debug, Error)]
pub enum CowSwapApiError {
    #[error("Request error: {0}")]
    RequestError(#[from] reqwest::Error),
    #[error("API error: {0}")]
    ApiError(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuoteRequest {
    pub sell_token: String,
    pub buy_token: String,
    pub sell_amount: String,
    pub kind: String,
    pub partially_fillable: bool,
    pub from: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuoteResponse {
    pub quote: Quote,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Quote {
    pub sell_token: String,
    pub buy_token: String,
    pub sell_amount: String,
    pub buy_amount: String,
    pub valid_to: u64,
    pub app_data: String,
    pub fee_amount: String,
}

#[derive(Debug, Clone)]
pub struct CowSwapApiClient {
    client: Client,
}

impl CowSwapApiClient {
    pub fn new() -> Self {
        Self {
            client: Client::new(),
        }
    }

    pub async fn get_quote(
        &self,
        from_token: Address,
        to_token: Address,
        amount: U256,
        wallet_address: Address,
        is_sell_order: bool,
    ) -> Result<QuoteResponse, String> {
        let url = format!(
            "https://api.cow.fi/mainnet/api/v1/quote?sellToken=0x{:x}&buyToken=0x{:x}&amount=0x{:x}&from=0x{:x}&kind={}",
            from_token,
            to_token,
            amount,
            wallet_address,
            if is_sell_order { "sell" } else { "buy" }
        );

        let response = reqwest::get(&url)
            .await
            .map_err(|e| e.to_string())?
            .json::<QuoteResponse>()
            .await
            .map_err(|e| e.to_string())?;

        Ok(response)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn submit_order(
        &self,
        sell_token: Address,
        buy_token: Address,
        sell_amount: U256,
        buy_amount: U256,
        valid_to: u64,
        app_data: String,
        fee_amount: U256,
        signature: String,
        from: Address,
    ) -> Result<OrderResponse, String> {
        let request = OrderRequest {
            sell_token: format!("0x{:x}", sell_token),
            buy_token: format!("0x{:x}", buy_token),
            sell_amount: format!("0x{:x}", sell_amount),
            buy_amount: format!("0x{:x}", buy_amount),
            valid_to,
            app_data,
            fee_amount: format!("0x{:x}", fee_amount),
            kind: "sell".to_string(),
            partially_fillable: false,
            signature,
            from: format!("0x{:x}", from),
        };

        let response = self
            .client
            .post(format!("{}/orders", API_BASE_URL))
            .json(&request)
            .send()
            .await
            .map_err(|e| e.to_string())?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(format!("API error: {}", error_text));
        }

        response.json().await.map_err(|e| e.to_string())
    }
}

impl Default for CowSwapApiClient {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct OrderRequest {
    pub sell_token: String,
    pub buy_token: String,
    pub sell_amount: String,
    pub buy_amount: String,
    pub valid_to: u64,
    pub app_data: String,
    pub fee_amount: String,
    pub kind: String,
    pub partially_fillable: bool,
    pub signature: String,
    pub from: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OrderResponse {
    pub order_uid: String,
}
