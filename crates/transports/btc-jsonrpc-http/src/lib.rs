#![warn(missing_docs)]
//! Bitcoin Core JSON-RPC over HTTP client.
//!
//! Minimal typed client for the Bitcoin Core RPCs needed by the mfm semantic runtime:
//! - `getblockchaininfo` — current chain height and best block hash (anchor)
//! - `getblockhash` and `getblockheader` — selected chain-head metadata
//! - `scantxoutset` — UTXO balance for a given address (observation)
//!
//! The client speaks plain JSON-RPC 2.0 over HTTP with optional Basic auth, using `reqwest`.
//! Debug output and errors redact RPC URL credentials, query strings, passwords, and raw response
//! bodies.
//!
//! # Examples
//!
//! ```rust
//! use mfm_transports_btc_jsonrpc_http::BtcJsonRpcConfig;
//!
//! let config = BtcJsonRpcConfig {
//!     rpc_url: "http://127.0.0.1:8332".to_string(),
//!     rpc_user: Some("user".to_string()),
//!     rpc_password: Some("pass".to_string()),
//! };
//! ```

use std::collections::BTreeMap;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use mfm_btc_capabilities::{
    btc_diagnostic, BtcBalanceReadProvider, BtcBalanceReadRequest, BtcBalanceReadResponse,
    BtcBlockHash, BtcCapabilityError, BtcCapabilityFuture, BtcChainHeadReadProvider,
    BtcChainHeadRequest, BtcChainHeadResponse, BtcFinality, BtcSourceBinding, BtcSourceIdentity,
    BtcSourceStatus, RedactedBtcSourceEvidence,
};
use mfm_capabilities::{
    ProviderDiagnosticCode, ProviderDiagnosticValue, RedactedProviderDiagnostic,
};
use mfm_ids::LocalPublicId;
use reqwest::header::CONTENT_TYPE;
use serde::{de, Deserialize, Deserializer, Serialize};
use tracing::debug;

const SATOSHIS_PER_BTC: u64 = 100_000_000;
const MAX_DIAGNOSTIC_MESSAGE_LEN: usize = 240;
const REDACTED_SECRET: &str = "<redacted>";
const REDACTED_DIAGNOSTIC: &str = "<redacted diagnostic message>";

/// Configuration for a Bitcoin Core JSON-RPC connection.
#[derive(Clone)]
pub struct BtcJsonRpcConfig {
    /// Bitcoin Core RPC URL (e.g. `http://127.0.0.1:8332`).
    pub rpc_url: String,
    /// Optional RPC username for Basic auth.
    pub rpc_user: Option<String>,
    /// Optional RPC password for Basic auth.
    pub rpc_password: Option<String>,
}

impl fmt::Debug for BtcJsonRpcConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BtcJsonRpcConfig")
            .field("rpc_url", &redacted_rpc_url(&self.rpc_url))
            .field("rpc_user", &redacted_optional_secret(&self.rpc_user))
            .field(
                "rpc_password",
                &redacted_optional_secret(&self.rpc_password),
            )
            .finish()
    }
}

/// Bitcoin Core JSON-RPC client.
pub struct BtcJsonRpcClient {
    config: BtcJsonRpcConfig,
    http: reqwest::Client,
    next_id: AtomicU64,
}

type BtcTransportFuture<'a, T> =
    Pin<Box<dyn Future<Output = std::result::Result<T, BtcRpcError>> + Send + 'a>>;

trait BtcJsonRpcChainHeadTransport: Send + Sync {
    fn get_blockchain_info<'a>(&'a self) -> BtcTransportFuture<'a, BlockchainInfo>;

    fn get_block_hash<'a>(
        &'a self,
        verified: &'a VerifiedBtcCall,
        height: u64,
    ) -> BtcTransportFuture<'a, String>;

    fn get_block_header<'a>(
        &'a self,
        verified: &'a VerifiedBtcCall,
        block_hash: &'a str,
    ) -> BtcTransportFuture<'a, BlockHeaderInfo>;

    fn scan_tx_out_set<'a>(
        &'a self,
        verified: &'a VerifiedBtcCall,
        address: &'a str,
    ) -> BtcTransportFuture<'a, ScanTxOutSetResult>;
}

impl BtcJsonRpcChainHeadTransport for BtcJsonRpcClient {
    fn get_blockchain_info<'a>(&'a self) -> BtcTransportFuture<'a, BlockchainInfo> {
        Box::pin(async move { BtcJsonRpcClient::get_blockchain_info(self).await })
    }

    fn get_block_hash<'a>(
        &'a self,
        verified: &'a VerifiedBtcCall,
        height: u64,
    ) -> BtcTransportFuture<'a, String> {
        Box::pin(async move { BtcJsonRpcClient::get_block_hash(self, verified, height).await })
    }

    fn get_block_header<'a>(
        &'a self,
        verified: &'a VerifiedBtcCall,
        block_hash: &'a str,
    ) -> BtcTransportFuture<'a, BlockHeaderInfo> {
        Box::pin(
            async move { BtcJsonRpcClient::get_block_header(self, verified, block_hash).await },
        )
    }

    fn scan_tx_out_set<'a>(
        &'a self,
        verified: &'a VerifiedBtcCall,
        address: &'a str,
    ) -> BtcTransportFuture<'a, ScanTxOutSetResult> {
        Box::pin(async move { BtcJsonRpcClient::scan_tx_out_set(self, verified, address).await })
    }
}

/// Bitcoin JSON-RPC router over semantic source identities.
///
/// Owns runtime route/source descriptors and low-level protocol mechanics. It exposes no-IO binding
/// validation and bind constructors only; it does not implement live capability provider traits.
#[derive(Clone)]
pub struct BtcJsonRpcRouter {
    routes: BTreeMap<BtcSourceIdentity, Arc<dyn BtcJsonRpcChainHeadTransport>>,
}

impl BtcJsonRpcRouter {
    /// Creates a router over JSON-RPC transports keyed by semantic source identity.
    pub fn new(routes: BTreeMap<BtcSourceIdentity, Arc<BtcJsonRpcClient>>) -> Self {
        Self {
            routes: routes
                .into_iter()
                .map(|(source, client)| (source, client as Arc<dyn BtcJsonRpcChainHeadTransport>))
                .collect(),
        }
    }

    #[cfg(test)]
    fn new_for_transport(
        routes: BTreeMap<BtcSourceIdentity, Arc<dyn BtcJsonRpcChainHeadTransport>>,
    ) -> Self {
        Self { routes }
    }

    /// Validates that a source binding can resolve to a configured route without network IO.
    pub fn validate_source_binding(
        &self,
        binding: &BtcSourceBinding,
    ) -> mfm_btc_capabilities::Result<()> {
        self.transport_for_source_identity(binding.source_identity())
            .map(|_| ())
    }

    /// Binds a checked semantic source binding to a live capability provider.
    pub fn bind_source(
        &self,
        binding: BtcSourceBinding,
    ) -> mfm_btc_capabilities::Result<BtcJsonRpcSourceProvider> {
        self.validate_source_binding(&binding)?;
        Ok(BtcJsonRpcSourceProvider {
            router: self.clone(),
            binding,
        })
    }

    fn transport_for_source_identity(
        &self,
        source_identity: &BtcSourceIdentity,
    ) -> mfm_btc_capabilities::Result<Arc<dyn BtcJsonRpcChainHeadTransport>> {
        self.routes
            .get(source_identity)
            .cloned()
            .ok_or_else(|| btc_provider_failure(missing_route_diagnostic(source_identity)))
    }
}

/// Bitcoin capability provider bound to a checked semantic source binding.
///
/// Owns a private sealed pipeline that mints [`VerifiedBtcCall`] before raw operation IO.
#[derive(Clone)]
pub struct BtcJsonRpcSourceProvider {
    router: BtcJsonRpcRouter,
    binding: BtcSourceBinding,
}

/// Private verified-call token minted by the sealed call-prepare stage.
///
/// Raw operation helpers require this token so capability IO cannot skip provider binding checks.
struct VerifiedBtcCall {
    transport: Arc<dyn BtcJsonRpcChainHeadTransport>,
    info: BlockchainInfo,
    evidence: RedactedBtcSourceEvidence,
}

impl BtcJsonRpcSourceProvider {
    async fn prepare_call(&self) -> mfm_btc_capabilities::Result<VerifiedBtcCall> {
        let transport = self
            .router
            .transport_for_source_identity(self.binding.source_identity())?;
        let info = transport
            .get_blockchain_info()
            .await
            .map_err(|error| btc_rpc_provider_error("getblockchaininfo", error))?;
        let status = source_status(&info);
        let evidence =
            RedactedBtcSourceEvidence::from_binding(&self.binding, info.chain.as_str(), status)?;
        Ok(VerifiedBtcCall {
            transport,
            info,
            evidence,
        })
    }

    async fn read_chain_head_checked(
        &self,
        verified: &VerifiedBtcCall,
        request: &BtcChainHeadRequest,
    ) -> mfm_btc_capabilities::Result<BtcChainHeadResponse> {
        let (height, hash) = selected_head(verified, request).await?;
        let header = verified
            .transport
            .get_block_header(verified, hash.as_str())
            .await
            .map_err(|error| btc_rpc_provider_error("getblockheader", error))?;
        verify_header(&header, height, hash.as_str())?;
        let provider_time_unix_ms = header.time.checked_mul(1000);
        Ok(BtcChainHeadResponse {
            evidence: verified.evidence.clone(),
            head_kind: request.selection().head_kind(),
            finality: request.selection().finality(),
            block_height: height,
            block_hash: hash,
            provider_time_unix_ms,
        })
    }

    async fn read_balance_checked(
        &self,
        verified: &VerifiedBtcCall,
        request: &BtcBalanceReadRequest,
    ) -> mfm_btc_capabilities::Result<BtcBalanceReadResponse> {
        verify_current_tip_matches_balance_request(verified, request)?;
        let scan = verified
            .transport
            .scan_tx_out_set(verified, request.address().as_str())
            .await
            .map_err(|error| btc_rpc_provider_error("scantxoutset", error))?;
        verify_scan_matches_balance_request(&scan, request)?;
        Ok(BtcBalanceReadResponse {
            evidence: verified.evidence.clone(),
            address: request.address().clone(),
            balance_sats: scan.total_amount_sats,
            block_height: request.block_height(),
            block_hash: request.block_hash().clone(),
        })
    }
}

impl BtcChainHeadReadProvider for BtcJsonRpcSourceProvider {
    fn read_chain_head<'a>(
        &'a self,
        request: &'a BtcChainHeadRequest,
    ) -> BtcCapabilityFuture<'a, BtcChainHeadResponse> {
        Box::pin(async move {
            let verified = self.prepare_call().await?;
            self.read_chain_head_checked(&verified, request).await
        })
    }
}

impl BtcBalanceReadProvider for BtcJsonRpcSourceProvider {
    fn read_balance<'a>(
        &'a self,
        request: &'a BtcBalanceReadRequest,
    ) -> BtcCapabilityFuture<'a, BtcBalanceReadResponse> {
        Box::pin(async move {
            let verified = self.prepare_call().await?;
            self.read_balance_checked(&verified, request).await
        })
    }
}

/// Error returned by the Bitcoin JSON-RPC client.
#[derive(Debug, thiserror::Error)]
pub enum BtcRpcError {
    /// HTTP transport failure.
    #[error("btc rpc http error: {0}")]
    Http(String),
    /// Non-2xx HTTP status.
    #[error("btc rpc http status {status}{status_details}", status_details = btc_status_details(*body_len, content_type.as_deref()))]
    HttpStatus {
        /// HTTP status code returned by the RPC endpoint.
        status: u16,
        /// Length in bytes of the omitted response body, when it could be read.
        body_len: Option<usize>,
        /// Sanitized `content-type` header value, when present.
        content_type: Option<String>,
    },
    /// Response body could not be read.
    #[error("btc rpc body read error: {0}")]
    BodyRead(String),
    /// Response was not valid JSON.
    #[error("btc rpc invalid json: {0}")]
    InvalidJson(String),
    /// JSON-RPC error object returned by Bitcoin Core.
    #[error("btc rpc error {code}: {message}")]
    JsonRpcError {
        /// Error code from Bitcoin Core.
        code: i64,
        /// Error message from Bitcoin Core.
        message: String,
    },
    /// JSON-RPC response missing `result` field.
    #[error("btc rpc response missing result")]
    MissingResult,
}

fn btc_status_details(body_len: Option<usize>, content_type: Option<&str>) -> String {
    match (body_len, content_type) {
        (Some(body_len), Some(content_type)) => {
            format!(" (body_len={body_len}, content_type={content_type})")
        }
        (Some(body_len), None) => format!(" (body_len={body_len})"),
        (None, Some(content_type)) => format!(" (content_type={content_type})"),
        (None, None) => String::new(),
    }
}

fn redacted_optional_secret(value: &Option<String>) -> &'static str {
    if value.is_some() {
        REDACTED_SECRET
    } else {
        "<unset>"
    }
}

fn redacted_rpc_url(raw: &str) -> String {
    let Ok(parsed) = reqwest::Url::parse(raw) else {
        return "<invalid-url>".to_string();
    };
    let Some(host_raw) = parsed.host_str() else {
        return "<invalid-url>".to_string();
    };
    let host = if host_raw.contains(':') && !host_raw.starts_with('[') {
        format!("[{host_raw}]")
    } else {
        host_raw.to_string()
    };

    match parsed.port() {
        Some(port) => format!("{}://{}:{}", parsed.scheme(), host, port),
        None => format!("{}://{}", parsed.scheme(), host),
    }
}

fn diagnostic_message(raw: &str) -> String {
    if contains_secret_marker(raw) || contains_secret_bearing_url(raw) {
        return REDACTED_DIAGNOSTIC.to_string();
    }
    truncate_diagnostic(raw)
}

fn contains_secret_bearing_url(raw: &str) -> bool {
    raw.split_ascii_whitespace().any(|part| {
        match reqwest::Url::parse(part.trim_matches(|c: char| {
            matches!(
                c,
                '"' | '\'' | '(' | ')' | '[' | ']' | '{' | '}' | ',' | ';'
            )
        })) {
            Ok(url) => {
                !url.username().is_empty() || url.password().is_some() || url.query().is_some()
            }
            Err(_) => false,
        }
    })
}

fn contains_secret_marker(raw: &str) -> bool {
    let lowered = raw.to_ascii_lowercase();
    [
        "authorization",
        "api_key",
        "apikey",
        "access_token",
        "token",
        "password",
        "passwd",
        "secret",
        "bearer ",
        "basic ",
    ]
    .iter()
    .any(|marker| lowered.contains(marker))
}

fn truncate_diagnostic(raw: &str) -> String {
    if raw.len() <= MAX_DIAGNOSTIC_MESSAGE_LEN {
        raw.to_string()
    } else {
        raw.chars().take(MAX_DIAGNOSTIC_MESSAGE_LEN).collect()
    }
}

fn sanitized_content_type(headers: &reqwest::header::HeaderMap) -> Option<String> {
    let raw = headers.get(CONTENT_TYPE)?.to_str().ok()?.trim();
    if raw.is_empty() {
        return None;
    }
    if contains_secret_marker(raw) || contains_secret_bearing_url(raw) {
        return Some(REDACTED_SECRET.to_string());
    }
    Some(
        raw.chars()
            .filter(|ch| {
                ch.is_ascii_alphanumeric()
                    || matches!(
                        ch,
                        '!' | '#'
                            | '$'
                            | '%'
                            | '&'
                            | '\''
                            | '*'
                            | '+'
                            | '-'
                            | '.'
                            | '^'
                            | '_'
                            | '`'
                            | '|'
                            | '~'
                            | '/'
                            | ';'
                            | '='
                            | ' '
                    )
            })
            .take(120)
            .collect(),
    )
}

/// Error returned when a Bitcoin BTC-denominated JSON amount cannot be represented exactly.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
enum BtcAmountParseError {
    #[error("bitcoin amount was empty")]
    Empty,
    #[error("bitcoin amount must not be negative")]
    Negative,
    #[error("bitcoin amount must be a base-10 integer or decimal")]
    Invalid,
    #[error("bitcoin amount must not have more than 8 decimal places")]
    TooPrecise,
    #[error("bitcoin amount overflowed satoshi range")]
    Overflow,
}

/// Response from `getblockchaininfo`.
#[derive(Clone, Debug, Deserialize)]
struct BlockchainInfo {
    blocks: u64,
    bestblockhash: String,
    chain: String,
    #[serde(default)]
    initialblockdownload: Option<bool>,
}

/// Response from `getblockheader` with verbose output.
#[derive(Clone, Debug, Deserialize)]
struct BlockHeaderInfo {
    hash: String,
    height: u64,
    time: u64,
}

/// Response from `scantxoutset` with only the fields used by the bound provider.
#[derive(Clone, Debug)]
struct ScanTxOutSetResult {
    success: bool,
    height: u64,
    bestblock: String,
    total_amount_sats: u64,
}

#[derive(Deserialize)]
struct ScanTxOutSetResultWire {
    success: bool,
    #[serde(default)]
    height: u64,
    #[serde(default)]
    bestblock: String,
    total_amount: serde_json::Value,
}

impl<'de> Deserialize<'de> for ScanTxOutSetResult {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = ScanTxOutSetResultWire::deserialize(deserializer)?;
        let total_amount_sats =
            btc_amount_json_to_sats(&wire.total_amount.to_string()).map_err(de::Error::custom)?;
        Ok(Self {
            success: wire.success,
            height: wire.height,
            bestblock: wire.bestblock,
            total_amount_sats,
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
fn btc_amount_json_to_sats(raw_json: &str) -> Result<u64, BtcAmountParseError> {
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
                BtcRpcError::Http(format!(
                    "failed to build bitcoin rpc client: {}",
                    diagnostic_message(&err.to_string())
                ))
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
                message: diagnostic_message(&err.message),
            });
        }

        rpc_resp.result.ok_or(BtcRpcError::MissingResult)
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
            .map_err(|e| BtcRpcError::Http(diagnostic_message(&e.to_string())))?;
        let status = resp.status().as_u16();
        if !(200..300).contains(&status) {
            let content_type = sanitized_content_type(resp.headers());
            let body_len = resp.bytes().await.ok().map(|bytes| bytes.len());
            return Err(BtcRpcError::HttpStatus {
                status,
                body_len,
                content_type,
            });
        }

        let text = resp
            .text()
            .await
            .map_err(|e| BtcRpcError::BodyRead(diagnostic_message(&e.to_string())))?;
        Ok(text)
    }

    async fn get_blockchain_info(&self) -> Result<BlockchainInfo, BtcRpcError> {
        let result = self
            .rpc_call("getblockchaininfo", serde_json::json!([]))
            .await?;
        serde_json::from_value(result).map_err(|e| BtcRpcError::InvalidJson(e.to_string()))
    }

    async fn get_block_hash(
        &self,
        _verified: &VerifiedBtcCall,
        height: u64,
    ) -> Result<String, BtcRpcError> {
        let result = self
            .rpc_call("getblockhash", serde_json::json!([height]))
            .await?;
        serde_json::from_value(result).map_err(|e| BtcRpcError::InvalidJson(e.to_string()))
    }

    async fn get_block_header(
        &self,
        _verified: &VerifiedBtcCall,
        block_hash: &str,
    ) -> Result<BlockHeaderInfo, BtcRpcError> {
        let result = self
            .rpc_call("getblockheader", serde_json::json!([block_hash, true]))
            .await?;
        serde_json::from_value(result).map_err(|e| BtcRpcError::InvalidJson(e.to_string()))
    }

    async fn scan_tx_out_set(
        &self,
        _verified: &VerifiedBtcCall,
        address: &str,
    ) -> Result<ScanTxOutSetResult, BtcRpcError> {
        let result = self
            .rpc_call(
                "scantxoutset",
                serde_json::json!(["start", [{"desc": format!("addr({address})")}]]),
            )
            .await?;
        serde_json::from_value(result).map_err(|e| BtcRpcError::InvalidJson(e.to_string()))
    }
}

async fn selected_head(
    verified: &VerifiedBtcCall,
    request: &BtcChainHeadRequest,
) -> mfm_btc_capabilities::Result<(u64, BtcBlockHash)> {
    match request.selection().finality() {
        BtcFinality::BestAvailable => {
            let hash = provider_block_hash("getblockchaininfo", &verified.info.bestblockhash)?;
            Ok((verified.info.blocks, hash))
        }
        BtcFinality::Confirmations(confirmations) => {
            let confirmations = confirmations.get();
            let available_confirmations = verified.info.blocks.saturating_add(1);
            if available_confirmations < confirmations {
                return Err(btc_provider_failure(
                    btc_operation_diagnostic(
                        ProviderDiagnosticCode::OperationIncomplete,
                        "getblockhash",
                    )
                    .with_field(
                        diagnostic_id("source_height"),
                        ProviderDiagnosticValue::U64(verified.info.blocks),
                    )
                    .with_field(
                        diagnostic_id("confirmations"),
                        ProviderDiagnosticValue::U64(confirmations),
                    ),
                ));
            }
            let height = verified.info.blocks + 1 - confirmations;
            let hash = verified
                .transport
                .get_block_hash(verified, height)
                .await
                .map_err(|error| btc_rpc_provider_error("getblockhash", error))?;
            Ok((height, provider_block_hash("getblockhash", &hash)?))
        }
    }
}

fn source_status(info: &BlockchainInfo) -> BtcSourceStatus {
    match info.initialblockdownload {
        Some(true) => BtcSourceStatus::InitialBlockDownload,
        Some(false) => BtcSourceStatus::Synced,
        None => BtcSourceStatus::Unknown,
    }
}

fn verify_header(
    header: &BlockHeaderInfo,
    height: u64,
    hash: &str,
) -> mfm_btc_capabilities::Result<()> {
    if header.height == height && header.hash.eq_ignore_ascii_case(hash) {
        Ok(())
    } else {
        Err(btc_provider_failure(btc_operation_diagnostic(
            ProviderDiagnosticCode::ResponseInvalid,
            "getblockheader",
        )))
    }
}

fn verify_current_tip_matches_balance_request(
    verified: &VerifiedBtcCall,
    request: &BtcBalanceReadRequest,
) -> mfm_btc_capabilities::Result<()> {
    let best_hash = provider_block_hash("getblockchaininfo", &verified.info.bestblockhash)?;
    if verified.info.blocks == request.block_height()
        && best_hash
            .as_str()
            .eq_ignore_ascii_case(request.block_hash().as_str())
    {
        Ok(())
    } else {
        Err(balance_anchor_unavailable_diagnostic(
            "getblockchaininfo",
            request,
            verified.info.blocks,
            best_hash.as_str(),
        ))
    }
}

fn verify_scan_matches_balance_request(
    scan: &ScanTxOutSetResult,
    request: &BtcBalanceReadRequest,
) -> mfm_btc_capabilities::Result<()> {
    if !scan.success {
        return Err(btc_provider_failure(
            btc_operation_diagnostic(ProviderDiagnosticCode::OperationIncomplete, "scantxoutset")
                .with_field(
                    diagnostic_id("scan_success"),
                    ProviderDiagnosticValue::Bool(false),
                ),
        ));
    }
    let scan_hash = provider_block_hash("scantxoutset", &scan.bestblock)?;
    if scan.height == request.block_height()
        && scan_hash
            .as_str()
            .eq_ignore_ascii_case(request.block_hash().as_str())
    {
        Ok(())
    } else {
        Err(balance_anchor_unavailable_diagnostic(
            "scantxoutset",
            request,
            scan.height,
            scan_hash.as_str(),
        ))
    }
}

fn balance_anchor_unavailable_diagnostic(
    operation: &'static str,
    request: &BtcBalanceReadRequest,
    observed_height: u64,
    observed_hash: &str,
) -> BtcCapabilityError {
    btc_provider_failure(
        btc_operation_diagnostic(ProviderDiagnosticCode::OperationIncomplete, operation)
            .with_field(
                diagnostic_id("requested_height"),
                ProviderDiagnosticValue::U64(request.block_height()),
            )
            .with_field(
                diagnostic_id("observed_height"),
                ProviderDiagnosticValue::U64(observed_height),
            )
            .with_field(
                diagnostic_id("requested_block_hash"),
                ProviderDiagnosticValue::Id(diagnostic_id(request.block_hash().as_str())),
            )
            .with_field(
                diagnostic_id("observed_block_hash"),
                ProviderDiagnosticValue::Id(diagnostic_id(observed_hash)),
            ),
    )
}

fn provider_block_hash(
    operation: &'static str,
    hash: &str,
) -> mfm_btc_capabilities::Result<BtcBlockHash> {
    BtcBlockHash::new(hash).map_err(|_| {
        btc_provider_failure(btc_operation_diagnostic(
            ProviderDiagnosticCode::ResponseInvalid,
            operation,
        ))
    })
}

fn missing_route_diagnostic(source_identity: &BtcSourceIdentity) -> RedactedProviderDiagnostic {
    btc_operation_diagnostic(ProviderDiagnosticCode::RouteUnavailable, "route_lookup").with_field(
        diagnostic_id("source_identity"),
        ProviderDiagnosticValue::Id(diagnostic_id(source_identity.as_str())),
    )
}

fn btc_rpc_provider_error(operation: &'static str, error: BtcRpcError) -> BtcCapabilityError {
    let diagnostic = match error {
        BtcRpcError::Http(_) | BtcRpcError::BodyRead(_) => {
            btc_operation_diagnostic(ProviderDiagnosticCode::TransportFailed, operation)
        }
        BtcRpcError::HttpStatus { status, .. } => {
            btc_operation_diagnostic(ProviderDiagnosticCode::RpcHttpStatus, operation).with_field(
                diagnostic_id("http_status"),
                ProviderDiagnosticValue::U64(u64::from(status)),
            )
        }
        BtcRpcError::InvalidJson(_) => {
            btc_operation_diagnostic(ProviderDiagnosticCode::ResponseInvalid, operation)
        }
        BtcRpcError::JsonRpcError { code, .. } => {
            btc_operation_diagnostic(ProviderDiagnosticCode::RpcJsonError, operation).with_field(
                diagnostic_id("rpc_code"),
                ProviderDiagnosticValue::I64(code),
            )
        }
        BtcRpcError::MissingResult => {
            btc_operation_diagnostic(ProviderDiagnosticCode::ResponseMissingResult, operation)
        }
    };
    btc_provider_failure(diagnostic)
}

fn btc_operation_diagnostic(
    code: ProviderDiagnosticCode,
    operation: &'static str,
) -> RedactedProviderDiagnostic {
    btc_diagnostic(code).with_operation(diagnostic_id(operation))
}

fn btc_provider_failure(diagnostic: RedactedProviderDiagnostic) -> BtcCapabilityError {
    debug!(
        provider_family = %diagnostic.provider_family(),
        diagnostic_code = diagnostic.code().as_str(),
        diagnostic = %diagnostic,
        "btc provider failure"
    );
    BtcCapabilityError::provider_failure(diagnostic)
}

fn diagnostic_id(value: &str) -> LocalPublicId {
    LocalPublicId::new(value).expect("BTC diagnostic label must be checked public text")
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
