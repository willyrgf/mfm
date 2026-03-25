#![warn(missing_docs)]
//! Bitcoin Core JSON-RPC over HTTP client.
//!
//! Minimal typed client for the two Bitcoin Core RPCs needed by the mfm semantic runtime:
//! - `getblockchaininfo` — current chain height and best block hash (anchor)
//! - `scantxoutset` — UTXO balance for a given address (observation)
//!
//! The client speaks plain JSON-RPC 2.0 over HTTP with optional Basic auth, using `reqwest`.
//!
//! # Examples
//!
//! ```rust
//! use mfm_collectors_btc_jsonrpc_http::BtcJsonRpcConfig;
//!
//! let config = BtcJsonRpcConfig {
//!     rpc_url: "http://127.0.0.1:8332".to_string(),
//!     rpc_user: Some("user".to_string()),
//!     rpc_password: Some("pass".to_string()),
//! };
//! ```

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tracing::debug;

/// Configuration for a Bitcoin Core JSON-RPC connection.
#[derive(Clone, Debug)]
pub struct BtcJsonRpcConfig {
    /// Bitcoin Core RPC URL (e.g. `http://127.0.0.1:8332`).
    pub rpc_url: String,
    /// Optional RPC username for Basic auth.
    pub rpc_user: Option<String>,
    /// Optional RPC password for Basic auth.
    pub rpc_password: Option<String>,
}

/// Bitcoin Core JSON-RPC client.
pub struct BtcJsonRpcClient {
    config: BtcJsonRpcConfig,
    http: reqwest::Client,
    next_id: AtomicU64,
}

/// Error returned by the Bitcoin JSON-RPC client.
#[derive(Debug)]
pub enum BtcRpcError {
    /// HTTP transport failure.
    Http(String),
    /// Non-2xx HTTP status.
    HttpStatus(u16, String),
    /// Response body could not be read.
    BodyRead(String),
    /// Response was not valid JSON.
    InvalidJson(String),
    /// JSON-RPC error object returned by Bitcoin Core.
    JsonRpcError {
        /// Error code from Bitcoin Core.
        code: i64,
        /// Error message from Bitcoin Core.
        message: String,
    },
    /// JSON-RPC response missing `result` field.
    MissingResult,
}

impl std::fmt::Display for BtcRpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BtcRpcError::Http(e) => write!(f, "btc rpc http error: {e}"),
            BtcRpcError::HttpStatus(code, body) => {
                write!(f, "btc rpc http status {code}: {body}")
            }
            BtcRpcError::BodyRead(e) => write!(f, "btc rpc body read error: {e}"),
            BtcRpcError::InvalidJson(e) => write!(f, "btc rpc invalid json: {e}"),
            BtcRpcError::JsonRpcError { code, message } => {
                write!(f, "btc rpc error {code}: {message}")
            }
            BtcRpcError::MissingResult => write!(f, "btc rpc response missing result"),
        }
    }
}

impl std::error::Error for BtcRpcError {}

/// Response from `getblockchaininfo`.
#[derive(Clone, Debug, Deserialize)]
pub struct BlockchainInfo {
    /// Current block height.
    pub blocks: u64,
    /// Best block hash.
    pub bestblockhash: String,
    /// Current chain name (e.g. "main", "test", "signet", "regtest").
    pub chain: String,
}

/// One unspent output returned by `scantxoutset`.
#[derive(Clone, Debug, Deserialize)]
pub struct ScannedUtxo {
    /// Transaction ID.
    pub txid: String,
    /// Output index.
    pub vout: u32,
    /// Value in BTC (floating point from Bitcoin Core).
    pub amount: f64,
    /// Block height where this output was confirmed.
    pub height: u64,
}

/// Response from `scantxoutset`.
#[derive(Clone, Debug, Deserialize)]
pub struct ScanTxOutSetResult {
    /// Whether the scan completed successfully.
    pub success: bool,
    /// Total amount in BTC across all matching UTXOs.
    pub total_amount: f64,
    /// Individual unspent outputs.
    #[serde(default)]
    pub unspents: Vec<ScannedUtxo>,
}

/// JSON-RPC 2.0 request envelope.
#[derive(Serialize)]
struct JsonRpcRequest<'a> {
    jsonrpc: &'static str,
    id: u64,
    method: &'a str,
    params: serde_json::Value,
}

/// JSON-RPC 2.0 response envelope.
#[derive(Deserialize)]
struct JsonRpcResponse {
    result: Option<serde_json::Value>,
    error: Option<JsonRpcErrorObj>,
}

/// JSON-RPC 2.0 error object.
#[derive(Deserialize)]
struct JsonRpcErrorObj {
    code: i64,
    message: String,
}

impl BtcJsonRpcClient {
    /// Creates a new client with the given configuration.
    pub fn new(config: BtcJsonRpcConfig) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("reqwest client");
        Self {
            config,
            http,
            next_id: AtomicU64::new(1),
        }
    }

    /// Low-level JSON-RPC call returning the raw `result` value.
    async fn rpc_call(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, BtcRpcError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let body = JsonRpcRequest {
            jsonrpc: "2.0",
            id,
            method,
            params,
        };

        debug!(method, id, "btc rpc request");

        let mut req = self.http.post(&self.config.rpc_url).json(&body);
        if let (Some(user), Some(pass)) = (&self.config.rpc_user, &self.config.rpc_password) {
            req = req.basic_auth(user, Some(pass));
        }

        let resp = req
            .send()
            .await
            .map_err(|e| BtcRpcError::Http(e.to_string()))?;
        let status = resp.status().as_u16();
        if status < 200 || status >= 300 {
            let body = resp
                .text()
                .await
                .unwrap_or_else(|_| "<unreadable>".to_string());
            return Err(BtcRpcError::HttpStatus(status, body));
        }

        let text = resp
            .text()
            .await
            .map_err(|e| BtcRpcError::BodyRead(e.to_string()))?;
        let rpc_resp: JsonRpcResponse =
            serde_json::from_str(&text).map_err(|e| BtcRpcError::InvalidJson(e.to_string()))?;

        if let Some(err) = rpc_resp.error {
            return Err(BtcRpcError::JsonRpcError {
                code: err.code,
                message: err.message,
            });
        }

        rpc_resp.result.ok_or(BtcRpcError::MissingResult)
    }

    /// Calls `getblockchaininfo` and returns the current chain state.
    pub async fn get_blockchain_info(&self) -> Result<BlockchainInfo, BtcRpcError> {
        let result = self
            .rpc_call("getblockchaininfo", serde_json::json!([]))
            .await?;
        serde_json::from_value(result).map_err(|e| BtcRpcError::InvalidJson(e.to_string()))
    }

    /// Calls `scantxoutset` for a single address descriptor.
    ///
    /// Uses `"start"` action to perform a fresh scan. The descriptor uses `addr(ADDRESS)` format.
    pub async fn scan_tx_out_set(&self, address: &str) -> Result<ScanTxOutSetResult, BtcRpcError> {
        let params = serde_json::json!([
            "start",
            [{ "desc": format!("addr({address})") }]
        ]);
        let result = self.rpc_call("scantxoutset", params).await?;
        serde_json::from_value(result).map_err(|e| BtcRpcError::InvalidJson(e.to_string()))
    }

    /// Returns the total balance in satoshis for a single address.
    ///
    /// Convenience method that calls [`scan_tx_out_set`](Self::scan_tx_out_set) and converts
    /// the BTC amount to satoshis.
    pub async fn address_balance_sats(&self, address: &str) -> Result<u64, BtcRpcError> {
        let result = self.scan_tx_out_set(address).await?;
        // Bitcoin Core returns BTC as f64; convert to satoshis with rounding.
        Ok((result.total_amount * 100_000_000.0).round() as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_creates_without_auth() {
        let config = BtcJsonRpcConfig {
            rpc_url: "http://127.0.0.1:8332".to_string(),
            rpc_user: None,
            rpc_password: None,
        };
        let _ = BtcJsonRpcClient::new(config);
    }

    #[test]
    fn blockchain_info_deserializes() {
        let json = serde_json::json!({
            "chain": "main",
            "blocks": 840000,
            "headers": 840000,
            "bestblockhash": "0000000000000000000320283a032748cef8227873ff4872689bf23f1cda83a5",
            "difficulty": 83148355189239.77,
            "time": 1713571767,
            "mediantime": 1713569060,
            "verificationprogress": 0.9999,
            "initialblockdownload": false,
            "chainwork": "00000000000000000000000000000000000000007b48a3b73a8f3af5bf6d5e5e",
            "size_on_disk": 650000000000_u64,
            "pruned": false,
            "warnings": ""
        });
        let info: BlockchainInfo = serde_json::from_value(json).expect("deserialize");
        assert_eq!(info.blocks, 840000);
        assert_eq!(
            info.bestblockhash,
            "0000000000000000000320283a032748cef8227873ff4872689bf23f1cda83a5"
        );
        assert_eq!(info.chain, "main");
    }

    #[test]
    fn scan_tx_out_set_result_deserializes() {
        let json = serde_json::json!({
            "success": true,
            "txouts": 120000000,
            "height": 840000,
            "bestblock": "0000000000000000000320283a032748cef8227873ff4872689bf23f1cda83a5",
            "unspents": [
                {
                    "txid": "abc123",
                    "vout": 0,
                    "scriptPubKey": "76a914...",
                    "desc": "addr(1BoatSLRHtKNngkdXEeobR76b53LETtpyT)#...",
                    "amount": 0.05,
                    "height": 839999
                }
            ],
            "total_amount": 0.05
        });
        let result: ScanTxOutSetResult = serde_json::from_value(json).expect("deserialize");
        assert!(result.success);
        assert_eq!(result.unspents.len(), 1);
        assert_eq!(result.unspents[0].txid, "abc123");
        assert_eq!(
            (result.total_amount * 100_000_000.0).round() as u64,
            5_000_000
        );
    }

    #[test]
    fn empty_scan_result_deserializes() {
        let json = serde_json::json!({
            "success": true,
            "txouts": 120000000,
            "height": 840000,
            "bestblock": "0000000000000000000320283a032748cef8227873ff4872689bf23f1cda83a5",
            "unspents": [],
            "total_amount": 0.0
        });
        let result: ScanTxOutSetResult = serde_json::from_value(json).expect("deserialize");
        assert!(result.success);
        assert!(result.unspents.is_empty());
        assert_eq!((result.total_amount * 100_000_000.0).round() as u64, 0);
    }
}
