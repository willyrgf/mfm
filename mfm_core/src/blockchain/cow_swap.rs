use crate::blockchain::{
    abi::cow_swap::*,
    dex::{self, DexError, DexProvider},
    evm::BlockchainError,
};
use ethers::{
    abi::AbiEncode,
    prelude::*,
    types::{
        transaction::eip712::{EIP712Domain as Eip712Domain, Eip712DomainType, TypedData},
        Bytes, H160, U256,
    },
    utils::keccak256,
};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::{
    collections::BTreeMap,
    time::{SystemTime, UNIX_EPOCH},
};

pub mod api;
use api::CowSwapApiClient;

const SETTLEMENT_CONTRACT_ADDRESS: &str = "0x9008D19f58AAbD9eD0D60971565AA8510560ab41";
const APP_DATA: &str = "0x0000000000000000000000000000000000000000000000000000000000000000";

#[derive(Debug, Clone)]
pub struct CowSwapProvider {
    pub provider: Provider<Http>,
    pub chain_id: u64,
    pub api_client: CowSwapApiClient,
    pub signer: Option<LocalWallet>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Order {
    pub sell_token: H160,
    pub buy_token: H160,
    pub receiver: H160,
    pub sell_amount: U256,
    pub buy_amount: U256,
    pub valid_to: u32,
    pub app_data: [u8; 32],
    pub fee_amount: U256,
    pub kind: [u8; 32],
    pub partially_fillable: bool,
    pub sell_token_balance: [u8; 32],
    pub buy_token_balance: [u8; 32],
}

impl CowSwapProvider {
    pub fn new(provider: Provider<Http>, chain_id: u64) -> Self {
        Self {
            provider,
            chain_id,
            api_client: CowSwapApiClient::new(),
            signer: None,
        }
    }

    pub fn with_signer(mut self, signer: LocalWallet) -> Self {
        self.signer = Some(signer);
        self
    }

    pub async fn create_order(
        &self,
        sell_token: H160,
        buy_token: H160,
        sell_amount: U256,
        buy_amount: U256,
        receiver: Option<H160>,
        valid_to: Option<u32>,
    ) -> Result<Order, BlockchainError> {
        // Default valid_to to 30 minutes from now if not specified
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as u32;
        let valid_to = valid_to.unwrap_or(now + 1800);

        // Get quote from API to determine fee
        let quote = self
            .api_client
            .get_quote(sell_token, buy_token, sell_amount, false)
            .await
            .map_err(|e| BlockchainError::Other(e.to_string()))?;

        let fee_amount = U256::from_str_radix(&quote.quote.fee_amount[2..], 16)
            .map_err(|e| BlockchainError::Other(e.to_string()))?;

        Ok(Order {
            sell_token,
            buy_token,
            receiver: receiver.unwrap_or_default(),
            sell_amount,
            buy_amount,
            valid_to,
            app_data: keccak256(APP_DATA),
            fee_amount,
            kind: hex::decode(ORDER_KIND_SELL).unwrap().try_into().unwrap(),
            partially_fillable: false,
            sell_token_balance: hex::decode(BALANCE_ERC20).unwrap().try_into().unwrap(),
            buy_token_balance: hex::decode(BALANCE_ERC20).unwrap().try_into().unwrap(),
        })
    }

    pub async fn sign_order(&self, order: &Order) -> Result<Bytes, BlockchainError> {
        let signer = self.signer.as_ref().ok_or_else(|| {
            BlockchainError::Other("No signer configured for CowSwapProvider".to_string())
        })?;

        let domain = Eip712Domain {
            name: Some("Gnosis Protocol".to_string()),
            version: Some("v2".to_string()),
            chain_id: Some(self.chain_id.into()),
            verifying_contract: Some(SETTLEMENT_CONTRACT_ADDRESS.parse().unwrap()),
            salt: None,
        };

        let mut types = BTreeMap::new();
        types.insert(
            "Order".to_string(),
            vec![
                Eip712DomainType {
                    name: "sellToken".into(),
                    r#type: "address".into(),
                },
                Eip712DomainType {
                    name: "buyToken".into(),
                    r#type: "address".into(),
                },
                Eip712DomainType {
                    name: "receiver".into(),
                    r#type: "address".into(),
                },
                Eip712DomainType {
                    name: "sellAmount".into(),
                    r#type: "uint256".into(),
                },
                Eip712DomainType {
                    name: "buyAmount".into(),
                    r#type: "uint256".into(),
                },
                Eip712DomainType {
                    name: "validTo".into(),
                    r#type: "uint32".into(),
                },
                Eip712DomainType {
                    name: "appData".into(),
                    r#type: "bytes32".into(),
                },
                Eip712DomainType {
                    name: "feeAmount".into(),
                    r#type: "uint256".into(),
                },
                Eip712DomainType {
                    name: "kind".into(),
                    r#type: "bytes32".into(),
                },
                Eip712DomainType {
                    name: "partiallyFillable".into(),
                    r#type: "bool".into(),
                },
                Eip712DomainType {
                    name: "sellTokenBalance".into(),
                    r#type: "bytes32".into(),
                },
                Eip712DomainType {
                    name: "buyTokenBalance".into(),
                    r#type: "bytes32".into(),
                },
            ],
        );

        let mut message = BTreeMap::new();
        message.insert(
            "sellToken".to_string(),
            JsonValue::String(format!("0x{}", hex::encode(order.sell_token.encode()))),
        );
        message.insert(
            "buyToken".to_string(),
            JsonValue::String(format!("0x{}", hex::encode(order.buy_token.encode()))),
        );
        message.insert(
            "receiver".to_string(),
            JsonValue::String(format!("0x{}", hex::encode(order.receiver.encode()))),
        );
        message.insert(
            "sellAmount".to_string(),
            JsonValue::String(format!("0x{}", hex::encode(order.sell_amount.encode()))),
        );
        message.insert(
            "buyAmount".to_string(),
            JsonValue::String(format!("0x{}", hex::encode(order.buy_amount.encode()))),
        );
        message.insert(
            "validTo".to_string(),
            JsonValue::Number(order.valid_to.into()),
        );
        message.insert(
            "appData".to_string(),
            JsonValue::String(format!("0x{}", hex::encode(order.app_data))),
        );
        message.insert(
            "feeAmount".to_string(),
            JsonValue::String(format!("0x{}", hex::encode(order.fee_amount.encode()))),
        );
        message.insert(
            "kind".to_string(),
            JsonValue::String(format!("0x{}", hex::encode(order.kind))),
        );
        message.insert(
            "partiallyFillable".to_string(),
            JsonValue::Bool(order.partially_fillable),
        );
        message.insert(
            "sellTokenBalance".to_string(),
            JsonValue::String(format!("0x{}", hex::encode(order.sell_token_balance))),
        );
        message.insert(
            "buyTokenBalance".to_string(),
            JsonValue::String(format!("0x{}", hex::encode(order.buy_token_balance))),
        );

        let typed_data = TypedData {
            domain,
            primary_type: "Order".to_string(),
            types,
            message,
        };

        let signature = signer
            .sign_typed_data(&typed_data)
            .await
            .map_err(|e| BlockchainError::Other(e.to_string()))?;
        Ok(signature.to_vec().into())
    }
}

#[async_trait::async_trait]
impl DexProvider for CowSwapProvider {
    async fn get_quote(
        &self,
        from_token: Address,
        to_token: Address,
        amount: U256,
    ) -> Result<dex::SwapQuote, DexError> {
        let quote = self
            .api_client
            .get_quote(from_token, to_token, amount, false)
            .await
            .map_err(|e| DexError::ApiError(e.to_string()))?;

        let to_amount = U256::from_str_radix(&quote.quote.buy_amount[2..], 16)
            .map_err(|e| DexError::QuoteError(e.to_string()))?;

        // Calculate price impact
        let price_impact = if amount.is_zero() {
            0.0
        } else {
            let expected_amount = to_amount;
            let actual_amount = to_amount;
            ((expected_amount.as_u128() as f64 - actual_amount.as_u128() as f64)
                / expected_amount.as_u128() as f64)
                * 100.0
        };

        Ok(dex::SwapQuote {
            from_token,
            to_token,
            from_amount: amount,
            to_amount,
            price_impact,
            route: vec![from_token, to_token],
        })
    }

    async fn execute_swap(
        &self,
        quote: dex::SwapQuote,
        wallet_address: Address,
        min_amount_out: U256,
    ) -> Result<String, DexError> {
        let order = self
            .create_order(
                quote.from_token,
                quote.to_token,
                quote.from_amount,
                min_amount_out,
                Some(wallet_address),
                None,
            )
            .await
            .map_err(|e| DexError::SwapError(e.to_string()))?;

        let signature = self
            .sign_order(&order)
            .await
            .map_err(|e| DexError::SwapError(e.to_string()))?;

        let response = self
            .api_client
            .submit_order(
                order.sell_token,
                order.buy_token,
                order.sell_amount,
                order.buy_amount,
                order.valid_to,
                format!("0x{}", hex::encode(order.app_data)),
                order.fee_amount,
                signature,
                wallet_address,
            )
            .await
            .map_err(|e| DexError::SwapError(e.to_string()))?;

        Ok(response.order_uid)
    }
}
