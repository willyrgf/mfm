use crate::blockchain::{
    abi::cow_swap::*,
    dex::{DexError, DexProvider, SwapQuote},
    evm::BlockchainError,
};
use ethers::{
    abi::AbiEncode,
    prelude::*,
    providers::{Http, Provider},
    types::{
        transaction::eip712::{EIP712Domain as Eip712Domain, Eip712DomainType, TypedData},
        Address, Bytes, H256, U256,
    },
};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::{
    collections::BTreeMap,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

pub mod api;
use api::{CowSwapApiClient, OrderRequest, OrderResponse, Quote, QuoteResponse};

// ERC20 ABI for token approval
abigen!(
    IERC20,
    r#"[
        function approve(address spender, uint256 amount) external returns (bool)
        function allowance(address owner, address spender) external view returns (uint256)
    ]"#
);

const SETTLEMENT_ADDRESS: &str = "0x9008D19f58AAbD9eD0D60971565AA8510560ab41";
const APP_DATA: &str = "0x0000000000000000000000000000000000000000000000000000000000000000";

#[derive(Debug)]
pub struct CowSwapProvider {
    provider: Arc<Provider<Http>>,
    chain_id: u64,
    signer: Option<LocalWallet>,
    api_client: api::CowSwapApiClient,
    settlement: Address,
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
    pub kind: String,
    pub partially_fillable: bool,
    pub receiver: Option<Address>,
}

impl CowSwapProvider {
    pub fn new(provider: Arc<Provider<Http>>, chain_id: u64, signer: Option<LocalWallet>) -> Self {
        Self {
            provider,
            chain_id,
            signer,
            api_client: api::CowSwapApiClient::new(),
            settlement: SETTLEMENT_ADDRESS.parse().unwrap(),
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
            kind: "sell".to_string(),
            partially_fillable: false,
            receiver,
        })
    }

    pub async fn sign_order(&self, order: &Order) -> Result<Bytes, BlockchainError> {
        if let Some(signer) = &self.signer {
            let domain = Eip712Domain {
                name: Some("Gnosis Protocol".to_string()),
                version: Some("v2".to_string()),
                chain_id: Some(self.chain_id.into()),
                verifying_contract: Some(self.settlement),
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
                    format!(
                        "0x{:x}",
                        order
                            .receiver
                            .unwrap_or_else(|| self.signer.as_ref().unwrap().address())
                    )
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
            message.insert("kind".to_string(), JsonValue::String(order.kind.clone()));
            message.insert(
                "partiallyFillable".to_string(),
                JsonValue::Bool(order.partially_fillable),
            );
            message.insert(
                "sellTokenBalance".to_string(),
                JsonValue::String("erc20".to_string()),
            );
            message.insert(
                "buyTokenBalance".to_string(),
                JsonValue::String("erc20".to_string()),
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

    async fn get_quote(
        &self,
        from_token: Address,
        to_token: Address,
        amount: U256,
        wallet_address: Address,
        is_sell_order: bool,
    ) -> Result<api::QuoteResponse, DexError> {
        self.api_client
            .get_quote(from_token, to_token, amount, wallet_address, is_sell_order)
            .await
            .map_err(|e| DexError::QuoteError(e))
    }

    async fn submit_order(
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
    ) -> Result<H256, DexError> {
        let response = self
            .api_client
            .submit_order(
                sell_token,
                buy_token,
                sell_amount,
                buy_amount,
                valid_to,
                app_data,
                fee_amount,
                signature,
                from,
            )
            .await
            .map_err(|e| DexError::SwapError(e))?;

        // Convert order_uid to H256
        let order_uid = response
            .order_uid
            .strip_prefix("0x")
            .unwrap_or(&response.order_uid);
        let bytes = hex::decode(order_uid).map_err(|e| DexError::SwapError(e.to_string()))?;
        let mut hash = [0u8; 32];
        hash.copy_from_slice(&bytes);
        Ok(H256::from(hash))
    }

    async fn check_and_approve_token(&self, token: Address, amount: U256) -> Result<(), DexError> {
        let signer = self.signer.as_ref().ok_or(DexError::NoSignerConfigured)?;
        self.check_and_approve_token_internal(token, self.settlement, amount)
            .await
    }

    async fn check_and_approve_token_internal(
        &self,
        token: Address,
        spender: Address,
        amount: U256,
    ) -> Result<(), DexError> {
        let token_contract = IERC20::new(token, Arc::new(self.provider.clone()));
        let contract_call = token_contract.approve(spender, amount);

        let pending_tx = contract_call
            .send()
            .await
            .map_err(|e| DexError::TokenApprovalError(e.to_string()))?;

        pending_tx
            .await
            .map_err(|e| DexError::TokenApprovalError(e.to_string()))?;

        Ok(())
    }
}

#[async_trait::async_trait]
impl DexProvider for CowSwapProvider {
    async fn get_quote(
        &self,
        from_token: Address,
        to_token: Address,
        amount: U256,
    ) -> Result<SwapQuote, DexError> {
        let quote = self
            .get_quote(from_token, to_token, amount, Address::zero(), false)
            .await?;

        // Calculate price impact
        let price_impact = if amount.is_zero() {
            0.0
        } else {
            let expected_amount = U256::from_dec_str(&quote.quote.buy_amount).unwrap();
            let actual_amount = U256::from_dec_str(&quote.quote.buy_amount).unwrap();
            ((expected_amount.as_u128() as f64 - actual_amount.as_u128() as f64)
                / expected_amount.as_u128() as f64)
                * 100.0
        };

        Ok(SwapQuote {
            from_token,
            to_token,
            from_amount: amount,
            to_amount: U256::from_dec_str(&quote.quote.buy_amount).unwrap(),
            price_impact,
            route: vec![from_token, to_token],
        })
    }

    async fn execute_swap(
        &self,
        from_token: Address,
        to_token: Address,
        amount: U256,
        exact_approval: bool,
    ) -> Result<H256, DexError> {
        let signer = self.signer.as_ref().ok_or(DexError::NoSignerConfigured)?;

        // Check and approve token if needed
        self.check_and_approve_token_internal(
            from_token,
            self.settlement,
            if exact_approval { amount } else { U256::MAX },
        )
        .await?;

        // Get quote for minimum amount out
        let quote = self
            .get_quote(from_token, to_token, amount, signer.address(), false)
            .await?;

        // Create order
        let order = self
            .create_order(
                from_token,
                to_token,
                amount,
                U256::from_dec_str(&quote.quote.buy_amount).unwrap(),
                None,
                None,
            )
            .await
            .map_err(|e| DexError::SwapError(e.to_string()))?;

        // Sign order
        let signature = self
            .sign_order(&order)
            .await
            .map_err(|e| DexError::SwapError(e.to_string()))?;

        // Submit order
        self.submit_order(
            order.sell_token,
            order.buy_token,
            order.sell_amount,
            order.buy_amount,
            order.valid_to as u64,
            format!("0x{}", hex::encode(&order.app_data)),
            order.fee_amount,
            hex::encode(signature),
            signer.address(),
        )
        .await
    }

    async fn check_and_approve_token(&self, token: Address, amount: U256) -> Result<(), DexError> {
        let signer = self.signer.as_ref().ok_or(DexError::NoSignerConfigured)?;
        self.check_and_approve_token_internal(token, self.settlement, amount)
            .await
    }
}
