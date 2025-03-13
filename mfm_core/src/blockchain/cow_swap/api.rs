use ethers::types::{Bytes, H160, U256};
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

#[derive(Debug, Clone)]
pub struct CowSwapApiClient {
    client: Client,
}

#[derive(Debug, Serialize)]
struct QuoteRequest {
    #[serde(rename = "sellToken")]
    sell_token: String,
    #[serde(rename = "buyToken")]
    buy_token: String,
    #[serde(rename = "sellAmountBeforeFee")]
    sell_amount: String,
    #[serde(rename = "kind")]
    kind: String,
    #[serde(rename = "partiallyFillable")]
    partially_fillable: bool,
    #[serde(rename = "from")]
    from: String,
}

#[derive(Debug, Deserialize)]
pub struct QuoteResponse {
    pub quote: Quote,
}

#[derive(Debug, Deserialize)]
pub struct Quote {
    #[serde(rename = "sellAmount")]
    pub sell_amount: String,
    #[serde(rename = "buyAmount")]
    pub buy_amount: String,
    #[serde(rename = "feeAmount")]
    pub fee_amount: String,
}

#[derive(Debug, Serialize)]
struct OrderRequest {
    #[serde(rename = "sellToken")]
    sell_token: String,
    #[serde(rename = "buyToken")]
    buy_token: String,
    #[serde(rename = "sellAmount")]
    sell_amount: String,
    #[serde(rename = "buyAmount")]
    buy_amount: String,
    #[serde(rename = "validTo")]
    valid_to: u32,
    #[serde(rename = "appData")]
    app_data: String,
    #[serde(rename = "feeAmount")]
    fee_amount: String,
    kind: String,
    #[serde(rename = "partiallyFillable")]
    partially_fillable: bool,
    signature: String,
    from: String,
    #[serde(rename = "sellTokenBalance")]
    sell_token_balance: String,
    #[serde(rename = "buyTokenBalance")]
    buy_token_balance: String,
    #[serde(rename = "signingScheme")]
    signing_scheme: String,
}

#[derive(Debug, Deserialize)]
pub struct OrderResponse {
    #[serde(rename = "orderUid")]
    pub order_uid: String,
}

impl CowSwapApiClient {
    pub fn new() -> Self {
        Self {
            client: Client::new(),
        }
    }

    pub async fn get_quote(
        &self,
        sell_token: H160,
        buy_token: H160,
        sell_amount: U256,
        from: H160,
        partially_fillable: bool,
    ) -> Result<QuoteResponse, CowSwapApiError> {
        let request = QuoteRequest {
            sell_token: format!("0x{:x}", sell_token).to_lowercase(),
            buy_token: format!("0x{:x}", buy_token).to_lowercase(),
            sell_amount: sell_amount.to_string(),
            kind: "sell".to_string(),
            partially_fillable,
            from: format!("0x{:x}", from).to_lowercase(),
        };

        eprintln!("Sending quote request to CowSwap API:");
        eprintln!("  Sell token: {}", request.sell_token);
        eprintln!("  Buy token: {}", request.buy_token);
        eprintln!("  Sell amount: {}", request.sell_amount);
        eprintln!("  From: {}", request.from);

        let response = self
            .client
            .post(&format!("{}/quote", API_BASE_URL))
            .json(&request)
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await?;
            eprintln!("CowSwap API error response: {}", error_text);
            return Err(CowSwapApiError::ApiError(error_text));
        }

        let quote_response = response.json().await?;
        Ok(quote_response)
    }

    pub async fn submit_order(
        &self,
        sell_token: H160,
        buy_token: H160,
        sell_amount: U256,
        buy_amount: U256,
        valid_to: u32,
        app_data: String,
        fee_amount: U256,
        signature: Bytes,
        from: H160,
    ) -> Result<OrderResponse, CowSwapApiError> {
        let request = OrderRequest {
            sell_token: format!("0x{:x}", sell_token).to_lowercase(),
            buy_token: format!("0x{:x}", buy_token).to_lowercase(),
            sell_amount: sell_amount.to_string(),
            buy_amount: buy_amount.to_string(),
            valid_to,
            app_data,
            fee_amount: fee_amount.to_string(),
            kind: "sell".to_string(),
            partially_fillable: false,
            signature: format!("0x{}", hex::encode(signature)),
            from: format!("0x{:x}", from).to_lowercase(),
            sell_token_balance: "erc20".to_string(),
            buy_token_balance: "erc20".to_string(),
            signing_scheme: "eip712".to_string(),
        };

        eprintln!("Submitting order to CowSwap API:");
        eprintln!("  Sell token: {}", request.sell_token);
        eprintln!("  Buy token: {}", request.buy_token);
        eprintln!("  Sell amount: {}", request.sell_amount);
        eprintln!("  Buy amount: {}", request.buy_amount);
        eprintln!("  From: {}", request.from);

        let response = self
            .client
            .post(&format!("{}/orders", API_BASE_URL))
            .json(&request)
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await?;
            eprintln!("CowSwap API error response: {}", error_text);
            return Err(CowSwapApiError::ApiError(error_text));
        }

        let order_response = response.json().await?;
        Ok(order_response)
    }
}
