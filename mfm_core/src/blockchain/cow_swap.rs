use crate::blockchain::{
    abi::cow_swap::*,
    dex::{self, DexError, DexProvider},
    evm::BlockchainError,
};
use ethers::{
    abi::AbiEncode,
    prelude::*,
    signers::{LocalWallet, Signer as EthersSigner},
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Order {
    pub sell_token: Address,
    pub buy_token: Address,
    pub sell_amount: U256,
    pub buy_amount: U256,
    pub valid_to: u32,
    pub app_data: Vec<u8>,
    pub fee_amount: U256,
    pub receiver: Option<Address>,
    pub sell_token_balance: String,
    pub buy_token_balance: String,
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
        sell_token: Address,
        buy_token: Address,
        sell_amount: U256,
        buy_amount: U256,
        receiver: Option<Address>,
        valid_to: Option<u32>,
    ) -> Result<Order, BlockchainError> {
        let valid_to = valid_to.unwrap_or_else(|| {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs() as u32;
            now + 3600 // 1 hour from now
        });

        let app_data = vec![0u8; 32]; // Empty app data

        let fee_amount = U256::from(0); // TODO: Calculate fee

        Ok(Order {
            sell_token,
            buy_token,
            sell_amount,
            buy_amount,
            valid_to,
            app_data,
            fee_amount,
            receiver,
            sell_token_balance: "erc20".to_string(),
            buy_token_balance: "erc20".to_string(),
        })
    }

    pub async fn sign_order(&self, order: &Order) -> Result<Bytes, BlockchainError> {
        if let Some(signer) = &self.signer {
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
                        name: "sellToken".to_string(),
                        r#type: "address".to_string(),
                    },
                    Eip712DomainType {
                        name: "buyToken".to_string(),
                        r#type: "address".to_string(),
                    },
                    Eip712DomainType {
                        name: "receiver".to_string(),
                        r#type: "address".to_string(),
                    },
                    Eip712DomainType {
                        name: "sellAmount".to_string(),
                        r#type: "uint256".to_string(),
                    },
                    Eip712DomainType {
                        name: "buyAmount".to_string(),
                        r#type: "uint256".to_string(),
                    },
                    Eip712DomainType {
                        name: "validTo".to_string(),
                        r#type: "uint32".to_string(),
                    },
                    Eip712DomainType {
                        name: "appData".to_string(),
                        r#type: "bytes32".to_string(),
                    },
                    Eip712DomainType {
                        name: "feeAmount".to_string(),
                        r#type: "uint256".to_string(),
                    },
                    Eip712DomainType {
                        name: "kind".to_string(),
                        r#type: "string".to_string(),
                    },
                    Eip712DomainType {
                        name: "partiallyFillable".to_string(),
                        r#type: "bool".to_string(),
                    },
                    Eip712DomainType {
                        name: "sellTokenBalance".to_string(),
                        r#type: "string".to_string(),
                    },
                    Eip712DomainType {
                        name: "buyTokenBalance".to_string(),
                        r#type: "string".to_string(),
                    },
                ],
            );

            let mut message = BTreeMap::new();
            message.insert(
                "sellToken".to_string(),
                JsonValue::String(format!("0x{:x}", order.sell_token).to_lowercase()),
            );
            message.insert(
                "buyToken".to_string(),
                JsonValue::String(format!("0x{:x}", order.buy_token).to_lowercase()),
            );
            message.insert(
                "receiver".to_string(),
                JsonValue::String(
                    format!("0x{:x}", order.receiver.unwrap_or_else(|| signer.address()))
                        .to_lowercase(),
                ),
            );
            message.insert(
                "sellAmount".to_string(),
                JsonValue::String(order.sell_amount.to_string()),
            );
            message.insert(
                "buyAmount".to_string(),
                JsonValue::String(order.buy_amount.to_string()),
            );
            message.insert(
                "validTo".to_string(),
                JsonValue::Number(order.valid_to.into()),
            );
            message.insert(
                "appData".to_string(),
                JsonValue::String(format!("0x{}", hex::encode(&order.app_data))),
            );
            message.insert(
                "feeAmount".to_string(),
                JsonValue::String(order.fee_amount.to_string()),
            );
            message.insert("kind".to_string(), JsonValue::String("sell".to_string()));
            message.insert("partiallyFillable".to_string(), JsonValue::Bool(false));
            message.insert(
                "sellTokenBalance".to_string(),
                JsonValue::String(order.sell_token_balance.clone()),
            );
            message.insert(
                "buyTokenBalance".to_string(),
                JsonValue::String(order.buy_token_balance.clone()),
            );

            let typed_data = TypedData {
                domain,
                types,
                primary_type: "Order".to_string(),
                message,
            };

            let signature = signer
                .sign_typed_data(&typed_data)
                .await
                .map_err(|e| BlockchainError::SigningError(e.to_string()))?;

            Ok(signature.to_vec().into())
        } else {
            Err(BlockchainError::SigningError(
                "No signer configured".to_string(),
            ))
        }
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
        if let Some(signer) = &self.signer {
            let quote = self
                .api_client
                .get_quote(from_token, to_token, amount, signer.address(), false)
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
        } else {
            Err(DexError::SwapError("No signer configured".to_string()))
        }
    }

    async fn execute_swap(
        &self,
        quote: dex::SwapQuote,
        wallet_address: Address,
        min_amount_out: U256,
    ) -> Result<String, DexError> {
        if let Some(signer) = &self.signer {
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
                    signer.address(),
                )
                .await
                .map_err(|e| DexError::SwapError(e.to_string()))?;

            Ok(response.order_uid)
        } else {
            Err(DexError::SwapError("No signer configured".to_string()))
        }
    }
}
