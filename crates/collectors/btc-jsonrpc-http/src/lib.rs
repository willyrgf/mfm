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

use serde::{de, Deserialize, Deserializer, Serialize};
use serde_json::value::RawValue;
use tracing::debug;

const SATOSHIS_PER_BTC: u64 = 100_000_000;

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

/// Error returned when a Bitcoin BTC-denominated JSON amount cannot be represented exactly.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BtcAmountParseError {
    /// The amount token or string was empty.
    Empty,
    /// The amount was negative.
    Negative,
    /// The amount was not a plain base-10 integer or decimal.
    Invalid,
    /// The amount had more than eight decimal places.
    TooPrecise,
    /// The amount exceeded `u64` satoshi range.
    Overflow,
}

impl std::fmt::Display for BtcAmountParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BtcAmountParseError::Empty => write!(f, "bitcoin amount was empty"),
            BtcAmountParseError::Negative => write!(f, "bitcoin amount must not be negative"),
            BtcAmountParseError::Invalid => {
                write!(f, "bitcoin amount must be a base-10 integer or decimal")
            }
            BtcAmountParseError::TooPrecise => {
                write!(f, "bitcoin amount must not have more than 8 decimal places")
            }
            BtcAmountParseError::Overflow => write!(f, "bitcoin amount overflowed satoshi range"),
        }
    }
}

impl std::error::Error for BtcAmountParseError {}

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
#[derive(Clone, Debug)]
pub struct ScannedUtxo {
    /// Transaction ID.
    pub txid: String,
    /// Output index.
    pub vout: u32,
    /// Value in satoshis.
    pub amount_sats: u64,
    /// Block height where this output was confirmed.
    pub height: u64,
}

/// Response from `scantxoutset`.
#[derive(Clone, Debug)]
pub struct ScanTxOutSetResult {
    /// Whether the scan completed successfully.
    pub success: bool,
    /// Block height of the scanned UTXO set.
    pub height: u64,
    /// Best block hash of the scanned UTXO set.
    pub bestblock: String,
    /// Total amount in satoshis across all matching UTXOs.
    pub total_amount_sats: u64,
    /// Individual unspent outputs.
    pub unspents: Vec<ScannedUtxo>,
}

#[derive(Deserialize)]
struct ScannedUtxoWire<'a> {
    txid: String,
    vout: u32,
    #[serde(borrow)]
    amount: &'a RawValue,
    height: u64,
}

#[derive(Deserialize)]
struct ScanTxOutSetResultWire<'a> {
    success: bool,
    #[serde(default)]
    height: u64,
    #[serde(default)]
    bestblock: String,
    #[serde(borrow)]
    total_amount: &'a RawValue,
    #[serde(default, borrow)]
    unspents: Vec<ScannedUtxoWire<'a>>,
}

impl<'de> Deserialize<'de> for ScanTxOutSetResult {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = ScanTxOutSetResultWire::deserialize(deserializer)?;
        let total_amount_sats =
            btc_amount_json_to_sats(wire.total_amount.get()).map_err(de::Error::custom)?;
        let unspents = wire
            .unspents
            .into_iter()
            .map(|utxo| {
                let amount_sats =
                    btc_amount_json_to_sats(utxo.amount.get()).map_err(de::Error::custom)?;
                Ok(ScannedUtxo {
                    txid: utxo.txid,
                    vout: utxo.vout,
                    amount_sats,
                    height: utxo.height,
                })
            })
            .collect::<Result<Vec<_>, D::Error>>()?;

        Ok(Self {
            success: wire.success,
            height: wire.height,
            bestblock: wire.bestblock,
            total_amount_sats,
            unspents,
        })
    }
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

#[derive(Deserialize)]
struct JsonRpcRawResponse<'a> {
    #[serde(borrow)]
    result: Option<&'a RawValue>,
    error: Option<JsonRpcErrorObj>,
}

/// JSON-RPC 2.0 error object.
#[derive(Deserialize)]
struct JsonRpcErrorObj {
    code: i64,
    message: String,
}

/// Parses a Bitcoin BTC-denominated JSON amount into exact satoshis.
///
/// The input must be the raw JSON token for an integer, decimal number, or string containing
/// a plain decimal amount. Exponents, negative values, and fractional precision above eight
/// places are rejected.
pub fn btc_amount_json_to_sats(raw_json: &str) -> Result<u64, BtcAmountParseError> {
    let raw = raw_json.trim();
    if raw.is_empty() {
        return Err(BtcAmountParseError::Empty);
    }

    let amount = if raw.starts_with('"') {
        serde_json::from_str::<String>(raw).map_err(|_| BtcAmountParseError::Invalid)?
    } else {
        raw.to_string()
    };
    parse_btc_decimal_to_sats(amount.trim())
}

fn parse_btc_decimal_to_sats(amount: &str) -> Result<u64, BtcAmountParseError> {
    if amount.is_empty() {
        return Err(BtcAmountParseError::Empty);
    }
    if amount.starts_with('-') {
        return Err(BtcAmountParseError::Negative);
    }
    if amount.starts_with('+') {
        return Err(BtcAmountParseError::Invalid);
    }

    let mut parts = amount.split('.');
    let whole = parts.next().ok_or(BtcAmountParseError::Invalid)?;
    let fractional = parts.next();
    if parts.next().is_some() || whole.is_empty() {
        return Err(BtcAmountParseError::Invalid);
    }
    if !whole.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(BtcAmountParseError::Invalid);
    }

    let whole_btc = whole
        .parse::<u64>()
        .map_err(|_| BtcAmountParseError::Overflow)?;
    let whole_sats = whole_btc
        .checked_mul(SATOSHIS_PER_BTC)
        .ok_or(BtcAmountParseError::Overflow)?;

    let fractional_sats = match fractional {
        None => 0,
        Some("") => return Err(BtcAmountParseError::Invalid),
        Some(frac) => parse_fractional_sats(frac)?,
    };

    whole_sats
        .checked_add(fractional_sats)
        .ok_or(BtcAmountParseError::Overflow)
}

fn parse_fractional_sats(fractional: &str) -> Result<u64, BtcAmountParseError> {
    if fractional.len() > 8 {
        return Err(BtcAmountParseError::TooPrecise);
    }
    if !fractional.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(BtcAmountParseError::Invalid);
    }

    let mut value = 0_u64;
    for byte in fractional.bytes() {
        value = value
            .checked_mul(10)
            .and_then(|current| current.checked_add(u64::from(byte - b'0')))
            .ok_or(BtcAmountParseError::Overflow)?;
    }
    for _ in fractional.len()..8 {
        value = value.checked_mul(10).ok_or(BtcAmountParseError::Overflow)?;
    }
    Ok(value)
}

impl BtcJsonRpcClient {
    /// Creates a new client with the given configuration.
    pub fn new(config: BtcJsonRpcConfig) -> Result<Self, BtcRpcError> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|err| {
                BtcRpcError::Http(format!("failed to build bitcoin rpc client: {err}"))
            })?;
        Ok(Self {
            config,
            http,
            next_id: AtomicU64::new(1),
        })
    }

    /// Low-level JSON-RPC call returning the raw `result` value.
    async fn rpc_call(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, BtcRpcError> {
        let text = self.rpc_call_text(method, params).await?;
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

    async fn rpc_call_raw_scan_result(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<ScanTxOutSetResult, BtcRpcError> {
        let text = self.rpc_call_text(method, params).await?;
        let rpc_resp: JsonRpcRawResponse =
            serde_json::from_str(&text).map_err(|e| BtcRpcError::InvalidJson(e.to_string()))?;

        if let Some(err) = rpc_resp.error {
            return Err(BtcRpcError::JsonRpcError {
                code: err.code,
                message: err.message,
            });
        }

        let result = rpc_resp.result.ok_or(BtcRpcError::MissingResult)?;
        serde_json::from_str(result.get()).map_err(|e| BtcRpcError::InvalidJson(e.to_string()))
    }

    async fn rpc_call_text(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<String, BtcRpcError> {
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
        if !(200..300).contains(&status) {
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
        Ok(text)
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
        self.rpc_call_raw_scan_result("scantxoutset", params).await
    }

    /// Returns the total balance in satoshis for a single address.
    ///
    /// Convenience method that calls [`scan_tx_out_set`](Self::scan_tx_out_set) and converts
    /// the BTC amount to satoshis.
    pub async fn address_balance_sats(&self, address: &str) -> Result<u64, BtcRpcError> {
        let result = self.scan_tx_out_set(address).await?;
        Ok(result.total_amount_sats)
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
        let _ = BtcJsonRpcClient::new(config).expect("constructor should not fail");
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
        let json = r#"{
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
                    "amount": 0.00000001,
                    "height": 839999
                }
            ],
            "total_amount": 0.05000000
        }"#;
        let result: ScanTxOutSetResult = serde_json::from_str(json).expect("deserialize");
        assert!(result.success);
        assert_eq!(result.unspents.len(), 1);
        assert_eq!(result.unspents[0].txid, "abc123");
        assert_eq!(result.unspents[0].amount_sats, 1);
        assert_eq!(result.total_amount_sats, 5_000_000);
    }

    #[test]
    fn empty_scan_result_deserializes() {
        let json = r#"{
            "success": true,
            "txouts": 120000000,
            "height": 840000,
            "bestblock": "0000000000000000000320283a032748cef8227873ff4872689bf23f1cda83a5",
            "unspents": [],
            "total_amount": 0
        }"#;
        let result: ScanTxOutSetResult = serde_json::from_str(json).expect("deserialize");
        assert!(result.success);
        assert!(result.unspents.is_empty());
        assert_eq!(result.total_amount_sats, 0);
    }

    #[test]
    fn btc_amount_json_to_sats_parses_exact_satoshis() {
        assert_eq!(btc_amount_json_to_sats("0.00000001").unwrap(), 1);
        assert_eq!(btc_amount_json_to_sats("0.05000000").unwrap(), 5_000_000);
        assert_eq!(
            btc_amount_json_to_sats("\"1.23000000\"").unwrap(),
            123_000_000
        );
    }

    #[test]
    fn btc_amount_json_to_sats_rejects_invalid_amounts() {
        assert_eq!(
            btc_amount_json_to_sats("0.000000001").unwrap_err(),
            BtcAmountParseError::TooPrecise
        );
        assert_eq!(
            btc_amount_json_to_sats("-0.00000001").unwrap_err(),
            BtcAmountParseError::Negative
        );
        assert_eq!(
            btc_amount_json_to_sats("18446744073709551615").unwrap_err(),
            BtcAmountParseError::Overflow
        );
    }
}
