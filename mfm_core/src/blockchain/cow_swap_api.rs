use crate::blockchain::adapter::types::{Address, Bytes, U256};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

const API_BASE_URL: &str = "https://api.cow.fi/mainnet/api/v1";

#[derive(Debug, Clone)]
pub struct CowSwapApiClient {
    client: Client,
}

#[derive(Debug, Serialize)]
struct QuoteRequest {
    sell_token: String,
    buy_token: String,
    #[serde(rename = "sellAmountBeforeFee")]
    sell_amount: String,
    kind: String,
    #[serde(rename = "partiallyFillable")]
    partially_fillable: bool,
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
        sell_token: Address,
        buy_token: Address,
        sell_amount: U256,
        partially_fillable: bool,
    ) -> Result<QuoteResponse, reqwest::Error> {
        let request = QuoteRequest {
            sell_token: format!("{}", sell_token),
            buy_token: format!("{}", buy_token),
            sell_amount: format!("{}", sell_amount),
            kind: "sell".to_string(),
            partially_fillable,
        };

        let response = self
            .client
            .post(&format!("{}/quote", API_BASE_URL))
            .json(&request)
            .send()
            .await?
            .json()
            .await?;

        Ok(response)
    }

    pub async fn submit_order(
        &self,
        sell_token: Address,
        buy_token: Address,
        sell_amount: U256,
        buy_amount: U256,
        valid_to: u32,
        app_data: String,
        fee_amount: U256,
        signature: Bytes,
        from: Address,
    ) -> Result<OrderResponse, reqwest::Error> {
        let request = OrderRequest {
            sell_token: format!("{}", sell_token),
            buy_token: format!("{}", buy_token),
            sell_amount: format!("{}", sell_amount),
            buy_amount: format!("{}", buy_amount),
            valid_to,
            app_data,
            fee_amount: format!("{}", fee_amount),
            kind: "sell".to_string(),
            partially_fillable: false,
            signature: format!("0x{}", hex::encode(signature.as_ref())),
            from: format!("{}", from),
            sell_token_balance: "erc20".to_string(),
            buy_token_balance: "erc20".to_string(),
        };

        let response = self
            .client
            .post(&format!("{}/orders", API_BASE_URL))
            .json(&request)
            .send()
            .await?
            .json()
            .await?;

        Ok(response)
    }

    // additional method for use with base_url parameter
    pub async fn get_quote_with_base_url(
        &self,
        base_url: &str,
        sell_token: Address,
        buy_token: Address,
        sell_amount: U256,
        from_address: Address,
        partially_fillable: bool,
    ) -> Result<QuoteResponse, reqwest::Error> {
        let request = QuoteRequest {
            sell_token: format!("{}", sell_token),
            buy_token: format!("{}", buy_token),
            sell_amount: format!("{}", sell_amount),
            kind: "sell".to_string(),
            partially_fillable,
        };

        let response = self
            .client
            .post(&format!("{}/quote", base_url))
            .json(&request)
            .send()
            .await?
            .json()
            .await?;

        Ok(response)
    }
}
