use crate::blockchain::adapter::types::{Address, U256};
use reqwest::Client;
use serde::{Deserialize, Serialize};

/// Quote request for CowSwap API
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuoteRequest {
    /// The sell token
    pub sell_token: String,
    /// The buy token
    pub buy_token: String,
    /// The sell amount
    pub sell_amount: String,
    /// Kind of order (sell or buy)
    pub kind: String,
    /// Partially fillable flag
    pub partially_fillable: bool,
    /// From address
    pub from: String,
}

/// Quote response from CowSwap API
#[derive(Debug, Clone, Deserialize)]
pub struct QuoteResponse {
    /// The quote details
    pub quote: Quote,
}

/// Quote details from CowSwap API
#[derive(Debug, Clone, Deserialize)]
pub struct Quote {
    /// The sell token
    pub sell_token: String,
    /// The buy token
    pub buy_token: String,
    /// The amount to sell
    pub sell_amount: String,
    /// The amount to buy
    pub buy_amount: String,
    /// The fee amount
    pub fee_amount: String,
    /// The app data
    #[serde(rename = "appData")]
    pub app_data: String,
    /// The validity time
    #[serde(rename = "validTo")]
    pub valid_to: u64,
}

/// Order creation response
#[derive(Debug, Deserialize)]
pub struct OrderResponse {
    /// The order UID
    #[serde(rename = "orderUid")]
    pub order_uid: String,
}

/// Order request for submission
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct OrderRequest {
    /// The sell token
    sell_token: String,
    /// The buy token
    buy_token: String,
    /// The receiver address
    receiver: String,
    /// The sell amount
    sell_amount: String,
    /// The buy amount
    buy_amount: String,
    /// The validity time
    valid_to: u64,
    /// The app data
    app_data: String,
    /// The fee amount
    fee_amount: String,
    /// Kind of order (sell or buy)
    kind: String,
    /// Partially fillable flag
    partially_fillable: bool,
    /// Signature
    signature: String,
    /// From address
    from: String,
    /// Quote ID
    quote_id: Option<String>,
}

/// Parameters for submitting an order
pub struct OrderParams {
    pub from_token: Address,
    pub to_token: Address,
    pub sell_amount: U256,
    pub buy_amount: U256,
    pub valid_to: u64,
    pub app_data: String,
    pub fee_amount: U256,
    pub signature: String,
    pub from_address: Address,
}

/// CowSwap API Client
#[derive(Clone, Debug)]
pub struct CowSwapApiClient {
    /// HTTP client for API requests
    client: Client,
}

impl CowSwapApiClient {
    /// Create a new CowSwap API client
    pub fn new() -> Self {
        Self {
            client: Client::new(),
        }
    }

    /// Get a quote from the CowSwap API
    pub async fn get_quote(
        &self,
        from_token: Address,
        to_token: Address,
        amount: U256,
        from_address: Address,
        _sell_token_balance: bool,
    ) -> Result<QuoteResponse, Box<dyn std::error::Error + Send + Sync>> {
        // Default to mainnet API
        self.get_quote_with_base_url(
            "https://api.cow.fi/mainnet/api/v1",
            from_token,
            to_token,
            amount,
            from_address,
            _sell_token_balance,
        )
        .await
    }

    /// Get a quote from the CowSwap API with a specific base URL
    pub async fn get_quote_with_base_url(
        &self,
        base_url: &str,
        from_token: Address,
        to_token: Address,
        amount: U256,
        from_address: Address,
        _sell_token_balance: bool,
    ) -> Result<QuoteResponse, Box<dyn std::error::Error + Send + Sync>> {
        // Format the URL query parameters
        let from_token_str = format!("{}", from_token);
        let to_token_str = format!("{}", to_token);
        let amount_str = format!("{}", amount);
        let from_address_str = format!("{}", from_address);

        // Build the URL with query parameters
        let url = format!(
            "{}/quote?sellToken={}&buyToken={}&sellAmountBeforeFee={}&from={}&priceQuality=fast",
            base_url, from_token_str, to_token_str, amount_str, from_address_str
        );

        // Make the API request
        let response = self.client.get(&url).send().await?;

        // Check the response status
        let status = response.status();
        if !status.is_success() {
            let error_text = response.text().await?;
            return Err(Box::new(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("API error: {} - {}", status, error_text),
            )));
        }

        // Parse the response
        let quote_response = response.json::<QuoteResponse>().await?;
        Ok(quote_response)
    }

    /// Submit an order to the CowSwap API
    pub async fn submit_order(
        &self,
        params: OrderParams,
    ) -> Result<OrderResponse, Box<dyn std::error::Error + Send + Sync>> {
        // Default to mainnet API
        self.submit_order_with_base_url("https://api.cow.fi/mainnet/api/v1", params)
            .await
    }

    /// Submit an order to the CowSwap API with a specific base URL
    pub async fn submit_order_with_base_url(
        &self,
        base_url: &str,
        params: OrderParams,
    ) -> Result<OrderResponse, Box<dyn std::error::Error + Send + Sync>> {
        // Create the order request
        let order_request = OrderRequest {
            sell_token: format!("{}", params.from_token),
            buy_token: format!("{}", params.to_token),
            receiver: format!("{}", params.from_address), // Receiver is the same as the sender
            sell_amount: format!("{}", params.sell_amount),
            buy_amount: format!("{}", params.buy_amount),
            valid_to: params.valid_to,
            app_data: params.app_data,
            fee_amount: format!("{}", params.fee_amount),
            kind: "sell".to_string(),  // We're selling tokens
            partially_fillable: false, // We want the full order to be filled
            signature: params.signature,
            from: format!("{}", params.from_address),
            quote_id: None, // Optional quote ID
        };

        // Submit the order
        let url = format!("{}/orders", base_url);
        let response = self.client.post(&url).json(&order_request).send().await?;

        // Check the response status
        let status = response.status();
        if !status.is_success() {
            let error_text = response.text().await?;
            return Err(Box::new(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("API error: {} - {}", status, error_text),
            )));
        }

        // Parse the response
        let order_response = response.json::<OrderResponse>().await?;
        Ok(order_response)
    }
}

impl Default for CowSwapApiClient {
    fn default() -> Self {
        Self::new()
    }
}
