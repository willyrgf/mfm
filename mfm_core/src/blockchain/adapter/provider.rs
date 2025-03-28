use crate::blockchain::adapter::types::{Address, Bytes, U256};
use alloy_primitives;
use alloy_rpc_client::RpcClient;
use alloy_transport_http::Http;
use anyhow::{anyhow, Result};
use hex;
use serde_json;
use std::sync::Arc;
use tokio;
use url::Url;

/// Provider implementation for blockchain RPC interactions
///
/// Wraps an alloy RPC client to provide methods for interacting with
/// the Ethereum blockchain
#[derive(Debug, Clone)]
pub struct Provider {
    /// Inner RPC client
    inner: Arc<RpcClient>,
}

/// Transaction structure to send to the network
#[derive(Debug, Clone)]
pub struct Transaction {
    /// Transaction nonce
    pub nonce: Option<U256>,
    /// Gas price (legacy)
    pub gas_price: Option<U256>,
    /// Maximum amount of gas for the transaction
    pub gas: Option<U256>,
    /// Target address (None for contract creation)
    pub to: Option<Address>,
    /// Value to send
    pub value: Option<U256>,
    /// Transaction data
    pub data: Bytes,
    /// Chain ID
    pub chain_id: Option<u64>,
}

/// Transaction hash type
pub type TxHash = String;

impl Provider {
    /// Create a new provider from an RpcClient
    pub fn new(client: Arc<RpcClient>) -> Self {
        Self { inner: client }
    }

    /// Create a new provider from a URL string
    pub async fn connect(url: &str) -> Result<Self> {
        let url = Url::parse(url)?;

        // For now, create a basic client without any complex configuration
        // We'll implement full functionality later
        let transport = Http::new(url);
        let client = RpcClient::new(transport, false);

        Ok(Self {
            inner: Arc::new(client),
        })
    }

    /// Get the balance of an address at the latest block
    pub async fn get_balance(&self, address: Address) -> Result<U256> {
        // Use eth_getBalance RPC method
        let address_str = format!("0x{}", hex::encode(address.0));
        let balance = self
            .inner
            .request::<_, alloy_primitives::U256>("eth_getBalance", (address_str, "latest"))
            .await
            .map_err(|e| anyhow!("Failed to get balance: {}", e))?;

        // Convert to our adapter type
        Ok(U256(balance))
    }

    /// Get the chain ID
    pub async fn get_chainid(&self) -> Result<u64> {
        // Use eth_chainId RPC method
        let chain_id = self
            .inner
            .request::<_, alloy_primitives::U256>("eth_chainId", ())
            .await
            .map_err(|e| anyhow!("Failed to get chain ID: {}", e))?;

        // Convert to u64
        Ok(chain_id.to::<u64>())
    }

    /// Call a contract method at the latest block
    pub async fn call(&self, to: Address, data: Bytes) -> Result<Bytes> {
        // Create the call request object
        let to_str = format!("0x{}", hex::encode(to.0));
        let data_str = format!("0x{}", hex::encode(&data.0));

        let call_object = serde_json::json!({
            "to": to_str,
            "data": data_str
        });

        // Make eth_call RPC request
        let bytes = self
            .inner
            .request::<_, alloy_primitives::Bytes>("eth_call", (call_object, "latest"))
            .await
            .map_err(|e| anyhow!("Contract call failed: {}", e))?;

        // Convert to our adapter type - copy the bytes into our type
        Ok(Bytes(bytes.to_vec()))
    }

    /// Send a transaction to the network
    pub async fn send_transaction(&self, transaction: &[u8]) -> Result<TxHash> {
        // The transaction is already RLP-encoded and signed
        let tx_hex = format!("0x{}", hex::encode(transaction));

        // Send the raw transaction using eth_sendRawTransaction
        let tx_hash = self
            .inner
            .request::<_, String>("eth_sendRawTransaction", [tx_hex])
            .await
            .map_err(|e| anyhow!("Failed to send transaction: {}", e))?;

        Ok(tx_hash)
    }

    /// Get the transaction count (nonce) for an address
    pub async fn get_transaction_count(&self, address: Address) -> Result<U256> {
        // Use eth_getTransactionCount RPC method
        let address_str = format!("0x{}", hex::encode(address.0));
        let nonce = self
            .inner
            .request::<_, alloy_primitives::U256>(
                "eth_getTransactionCount",
                (address_str, "latest"),
            )
            .await
            .map_err(|e| anyhow!("Failed to get transaction count: {}", e))?;

        // Convert to our adapter type
        Ok(U256(nonce))
    }

    /// Get the gas price
    pub async fn get_gas_price(&self) -> Result<U256> {
        // Use eth_gasPrice RPC method
        let gas_price = self
            .inner
            .request::<_, alloy_primitives::U256>("eth_gasPrice", ())
            .await
            .map_err(|e| anyhow!("Failed to get gas price: {}", e))?;

        // Convert to our adapter type
        Ok(U256(gas_price))
    }

    /// Estimate gas for a transaction
    pub async fn estimate_gas(&self, tx: &Transaction) -> Result<U256> {
        // Create call object
        let call_object = serde_json::json!({
            "from": tx.to.map(|addr| format!("0x{}", hex::encode(addr.0))),
            "to": tx.to.map(|addr| format!("0x{}", hex::encode(addr.0))),
            "gas": tx.gas.map(|g| format!("0x{:x}", g.0)),
            "gasPrice": tx.gas_price.map(|gp| format!("0x{:x}", gp.0)),
            "value": tx.value.map(|v| format!("0x{:x}", v.0)),
            "data": format!("0x{}", hex::encode(&tx.data.0))
        });

        // Call eth_estimateGas
        let gas = self
            .inner
            .request::<_, alloy_primitives::U256>("eth_estimateGas", [call_object])
            .await
            .map_err(|e| anyhow!("Failed to estimate gas: {}", e))?;

        // Convert to our adapter type
        Ok(U256(gas))
    }

    /// Get the inner provider
    pub fn inner(&self) -> &RpcClient {
        &self.inner
    }

    /// Get an Arc to the inner provider
    pub fn inner_arc(&self) -> Arc<RpcClient> {
        self.inner.clone()
    }

    /// Get the transaction receipt for a transaction hash
    pub async fn get_transaction_receipt(
        &self,
        tx_hash: &TxHash,
    ) -> Result<Option<serde_json::Value>> {
        // Call the eth_getTransactionReceipt RPC method
        let receipt = self
            .inner
            .request::<_, serde_json::Value>("eth_getTransactionReceipt", [tx_hash])
            .await
            .map_err(|e| anyhow!("Failed to get transaction receipt: {}", e))?;

        // Check if the receipt is null (transaction not yet mined)
        if receipt.is_null() {
            return Ok(None);
        }

        Ok(Some(receipt))
    }

    /// Get the latest block number
    pub async fn get_block_number(&self) -> Result<U256> {
        // Use eth_blockNumber RPC method
        let block_number = self
            .inner
            .request::<_, alloy_primitives::U256>("eth_blockNumber", ())
            .await
            .map_err(|e| anyhow!("Failed to get block number: {}", e))?;

        // Convert to our adapter type
        Ok(U256(block_number))
    }

    /// Wait for a transaction to be confirmed
    pub async fn wait_for_transaction(
        &self,
        tx_hash: &TxHash,
        confirmations: u64,
    ) -> Result<serde_json::Value> {
        // Loop until we have enough confirmations
        let mut attempts = 0;
        let max_attempts = 50; // Prevent infinite loops

        loop {
            if attempts >= max_attempts {
                return Err(anyhow!("Transaction not confirmed after max attempts"));
            }

            // Wait a bit between checks
            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

            // Get the transaction receipt
            if let Some(receipt) = self.get_transaction_receipt(tx_hash).await? {
                // Check if the receipt has a block number
                if let Some(block_number) = receipt.get("blockNumber") {
                    // Get the latest block number
                    let latest_block = self.get_block_number().await?;

                    // Parse the receipt block number
                    let receipt_block_str = block_number.as_str().unwrap_or("0x0");
                    let receipt_block = alloy_primitives::U256::from_str_radix(
                        receipt_block_str.trim_start_matches("0x"),
                        16,
                    )
                    .map_err(|e| anyhow!("Failed to parse block number: {}", e))?;

                    // Calculate confirmations
                    let confirmation_blocks = latest_block
                        .0
                        .checked_sub(receipt_block)
                        .unwrap_or_default();

                    if confirmation_blocks >= alloy_primitives::U256::from(confirmations) {
                        return Ok(receipt);
                    }
                }
            }

            attempts += 1;
        }
    }
}
