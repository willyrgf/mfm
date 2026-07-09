#![warn(missing_docs)]
//! Bitcoin Core JSON-RPC over HTTP transport.
//!
//! Production public surface:
//! - [`BtcJsonRpcConfig`] + [`BtcJsonRpcClient::new`]
//! - [`BtcJsonRpcRouter::{new, validate_source_binding, bind_source}`]
//! - bound [`BtcJsonRpcSourceProvider`] capability implementations
//!
//! Probe (`getblockchaininfo`) and capability operation RPCs are crate-private. Live capability
//! methods enter a sealed `prepare_call` path that mints a private verified-call token; operation
//! IO requires that token. The transport trait used for in-crate mocking is not public.
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
use serde_json::value::RawValue;
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

/// Boxed future returned by Bitcoin JSON-RPC transport abstractions.
///
/// Crate-private: production callers use [`BtcJsonRpcClient`] via the router and bound provider,
/// not dyn transport trait objects.
type BtcTransportFuture<'a, T> =
    Pin<Box<dyn Future<Output = std::result::Result<T, BtcRpcError>> + Send + 'a>>;

/// Minimal Bitcoin JSON-RPC transport surface needed by the sealed provider pipeline.
///
/// Crate-private so adapters and app cannot call probe/operation methods via a public dyn trait.
/// Production code constructs [`BtcJsonRpcRouter`] from concrete [`BtcJsonRpcClient`] values only.
trait BtcJsonRpcChainHeadTransport: Send + Sync {
    /// Identity probe (`getblockchaininfo`). Only for sealed `prepare_call`.
    fn get_blockchain_info<'a>(&'a self) -> BtcTransportFuture<'a, BlockchainInfo>;

    /// Reads the block hash for a selected height (capability operation IO).
    fn get_block_hash<'a>(&'a self, height: u64) -> BtcTransportFuture<'a, String>;

    /// Reads verbose block-header metadata for a block hash (capability operation IO).
    fn get_block_header<'a>(
        &'a self,
        block_hash: &'a str,
    ) -> BtcTransportFuture<'a, BlockHeaderInfo>;

    /// Scans the UTXO set for a single address (capability operation IO).
    fn scan_tx_out_set<'a>(
        &'a self,
        address: &'a str,
    ) -> BtcTransportFuture<'a, ScanTxOutSetResult>;
}

impl BtcJsonRpcChainHeadTransport for BtcJsonRpcClient {
    fn get_blockchain_info<'a>(&'a self) -> BtcTransportFuture<'a, BlockchainInfo> {
        Box::pin(async move { BtcJsonRpcClient::get_blockchain_info(self).await })
    }

    fn get_block_hash<'a>(&'a self, height: u64) -> BtcTransportFuture<'a, String> {
        Box::pin(async move { BtcJsonRpcClient::get_block_hash(self, height).await })
    }

    fn get_block_header<'a>(
        &'a self,
        block_hash: &'a str,
    ) -> BtcTransportFuture<'a, BlockHeaderInfo> {
        Box::pin(async move { BtcJsonRpcClient::get_block_header(self, block_hash).await })
    }

    fn scan_tx_out_set<'a>(
        &'a self,
        address: &'a str,
    ) -> BtcTransportFuture<'a, ScanTxOutSetResult> {
        Box::pin(async move { BtcJsonRpcClient::scan_tx_out_set(self, address).await })
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
    /// Creates a router over concrete JSON-RPC clients keyed by semantic source identity.
    ///
    /// Converts clients to crate-private transport trait objects inside this crate. There is no
    /// public dyn-transport constructor; production code must pass [`BtcJsonRpcClient`] values.
    pub fn new(routes: BTreeMap<BtcSourceIdentity, Arc<BtcJsonRpcClient>>) -> Self {
        Self {
            routes: routes
                .into_iter()
                .map(|(identity, client)| {
                    (identity, client as Arc<dyn BtcJsonRpcChainHeadTransport>)
                })
                .collect(),
        }
    }

    /// Test-only router constructor that injects mock transports.
    ///
    /// Not a production dual path: unavailable outside `#[cfg(test)]` of this crate.
    #[cfg(test)]
    fn from_transports(
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

impl VerifiedBtcCall {
    /// Token-gated `getblockhash` operation IO.
    async fn get_block_hash(&self, height: u64) -> mfm_btc_capabilities::Result<String> {
        self.transport
            .get_block_hash(height)
            .await
            .map_err(|error| btc_rpc_provider_error("getblockhash", error))
    }

    /// Token-gated `getblockheader` operation IO.
    async fn get_block_header(
        &self,
        block_hash: &str,
    ) -> mfm_btc_capabilities::Result<BlockHeaderInfo> {
        self.transport
            .get_block_header(block_hash)
            .await
            .map_err(|error| btc_rpc_provider_error("getblockheader", error))
    }

    /// Token-gated `scantxoutset` operation IO (reserved for future exact-balance support).
    #[allow(dead_code)]
    async fn scan_tx_out_set(
        &self,
        address: &str,
    ) -> mfm_btc_capabilities::Result<ScanTxOutSetResult> {
        self.transport
            .scan_tx_out_set(address)
            .await
            .map_err(|error| btc_rpc_provider_error("scantxoutset", error))
    }
}

impl BtcJsonRpcSourceProvider {
    /// Returns the private semantic binding owned by this provider.
    pub const fn binding(&self) -> &BtcSourceBinding {
        &self.binding
    }

    /// Call-prepare: resolve bound source route, probe `getblockchaininfo`, mint verified token.
    ///
    /// The identity probe is a private pre-token path; capability operation IO requires the token.
    async fn prepare_call(&self) -> mfm_btc_capabilities::Result<VerifiedBtcCall> {
        let transport = self
            .router
            .transport_for_source_identity(self.binding.source_identity())?;
        // Private identity probe — not a general operation helper and not token-gated.
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
        let header = verified.get_block_header(hash.as_str()).await?;
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
        _verified: &VerifiedBtcCall,
        request: &BtcBalanceReadRequest,
    ) -> mfm_btc_capabilities::Result<BtcBalanceReadResponse> {
        // Token-gated path: prepare_call already ran. Live Core cannot prove exact anchors.
        Err(btc_provider_failure(
            btc_operation_diagnostic(ProviderDiagnosticCode::UnsupportedOperation, "read_balance")
                .with_field(
                    diagnostic_id("block_height"),
                    ProviderDiagnosticValue::U64(request.block_height()),
                ),
        ))
    }

    async fn read_chain_head_inner(
        &self,
        request: &BtcChainHeadRequest,
    ) -> mfm_btc_capabilities::Result<BtcChainHeadResponse> {
        let verified = self.prepare_call().await?;
        self.read_chain_head_checked(&verified, request).await
    }

    async fn read_balance_inner(
        &self,
        request: &BtcBalanceReadRequest,
    ) -> mfm_btc_capabilities::Result<BtcBalanceReadResponse> {
        let verified = self.prepare_call().await?;
        self.read_balance_checked(&verified, request).await
    }
}

impl BtcChainHeadReadProvider for BtcJsonRpcSourceProvider {
    fn read_chain_head<'a>(
        &'a self,
        request: &'a BtcChainHeadRequest,
    ) -> BtcCapabilityFuture<'a, BtcChainHeadResponse> {
        Box::pin(async move { self.read_chain_head_inner(request).await })
    }
}

impl BtcBalanceReadProvider for BtcJsonRpcSourceProvider {
    fn read_balance<'a>(
        &'a self,
        request: &'a BtcBalanceReadRequest,
    ) -> BtcCapabilityFuture<'a, BtcBalanceReadResponse> {
        Box::pin(async move { self.read_balance_inner(request).await })
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
pub enum BtcAmountParseError {
    /// The amount token or string was empty.
    #[error("bitcoin amount was empty")]
    Empty,
    /// The amount was negative.
    #[error("bitcoin amount must not be negative")]
    Negative,
    /// The amount was not a plain base-10 integer or decimal.
    #[error("bitcoin amount must be a base-10 integer or decimal")]
    Invalid,
    /// The amount had more than eight decimal places.
    #[error("bitcoin amount must not have more than 8 decimal places")]
    TooPrecise,
    /// The amount exceeded `u64` satoshi range.
    #[error("bitcoin amount overflowed satoshi range")]
    Overflow,
}

/// Response from `getblockchaininfo`.
#[derive(Clone, Debug, Deserialize)]
pub struct BlockchainInfo {
    /// Current block height.
    pub blocks: u64,
    /// Best block hash.
    pub bestblockhash: String,
    /// Current chain name (e.g. "main", "test", "signet", "regtest").
    pub chain: String,
    /// Whether the node reports initial block download, when present.
    #[serde(default)]
    pub initialblockdownload: Option<bool>,
}

/// Response from `getblockheader` with verbose output.
#[derive(Clone, Debug, Deserialize)]
pub struct BlockHeaderInfo {
    /// Block hash.
    pub hash: String,
    /// Block height.
    pub height: u64,
    /// Block timestamp in Unix seconds.
    pub time: u64,
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
                message: diagnostic_message(&err.message),
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

    /// Identity probe for sealed `prepare_call` only (not a public production API).
    async fn get_blockchain_info(&self) -> Result<BlockchainInfo, BtcRpcError> {
        let result = self
            .rpc_call("getblockchaininfo", serde_json::json!([]))
            .await?;
        serde_json::from_value(result).map_err(|e| BtcRpcError::InvalidJson(e.to_string()))
    }

    /// Operation IO for the sealed bound-provider path only.
    async fn get_block_hash(&self, height: u64) -> Result<String, BtcRpcError> {
        let result = self
            .rpc_call("getblockhash", serde_json::json!([height]))
            .await?;
        serde_json::from_value(result).map_err(|e| BtcRpcError::InvalidJson(e.to_string()))
    }

    /// Operation IO for the sealed bound-provider path only.
    async fn get_block_header(&self, block_hash: &str) -> Result<BlockHeaderInfo, BtcRpcError> {
        let result = self
            .rpc_call("getblockheader", serde_json::json!([block_hash, true]))
            .await?;
        serde_json::from_value(result).map_err(|e| BtcRpcError::InvalidJson(e.to_string()))
    }

    /// Operation IO for the sealed bound-provider path only.
    async fn scan_tx_out_set(&self, address: &str) -> Result<ScanTxOutSetResult, BtcRpcError> {
        let params = serde_json::json!([
            "start",
            [{ "desc": format!("addr({address})") }]
        ]);
        self.rpc_call_raw_scan_result("scantxoutset", params).await
    }

    /// Convenience helper over sealed `scan_tx_out_set` (crate-private).
    #[allow(dead_code)]
    async fn address_balance_sats(&self, address: &str) -> Result<u64, BtcRpcError> {
        let result = self.scan_tx_out_set(address).await?;
        Ok(result.total_amount_sats)
    }
}

/// Selects the requested head using token-gated operation helpers only.
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
            if verified.info.blocks < confirmations {
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
            let height = verified.info.blocks - confirmations;
            let hash = verified.get_block_hash(height).await?;
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
    btc_operation_diagnostic(ProviderDiagnosticCode::SourceMismatch, "route_lookup").with_field(
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
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU64;

    use mfm_btc_capabilities::{BitcoinNetworkTag, BtcAddress, BtcHeadSelection, BtcNetworkId};

    const BEST_BLOCK_HASH: &str =
        "0000000000000000000320283a032748cef8227873ff4872689bf23f1cda83a5";
    const CONFIRMED_BLOCK_HASH: &str =
        "00000000000000000001b2a7f3e0d5c4b6a897887766554433221100ffeeddcc";
    const TIP_HEIGHT: u64 = 840_000;

    #[derive(Clone, Copy)]
    enum MockInfoFailure {
        Http,
        HttpStatus,
        JsonRpcSecret,
    }

    struct MockBtcTransport {
        chain: &'static str,
        info_failure: Option<MockInfoFailure>,
        blockchain_info_calls: AtomicU64,
        block_hash_calls: AtomicU64,
        block_header_calls: AtomicU64,
        scan_calls: AtomicU64,
    }

    impl MockBtcTransport {
        fn new(chain: &'static str) -> Self {
            Self {
                chain,
                info_failure: None,
                blockchain_info_calls: AtomicU64::new(0),
                block_hash_calls: AtomicU64::new(0),
                block_header_calls: AtomicU64::new(0),
                scan_calls: AtomicU64::new(0),
            }
        }

        fn with_info_failure(failure: MockInfoFailure) -> Self {
            Self {
                info_failure: Some(failure),
                ..Self::new("main")
            }
        }
    }

    impl BtcJsonRpcChainHeadTransport for MockBtcTransport {
        fn get_blockchain_info<'a>(&'a self) -> BtcTransportFuture<'a, BlockchainInfo> {
            Box::pin(async move {
                self.blockchain_info_calls.fetch_add(1, Ordering::Relaxed);
                if let Some(failure) = self.info_failure {
                    return Err(match failure {
                        MockInfoFailure::Http => BtcRpcError::Http(
                            "http://user:password@localhost:8332 Authorization: Bearer secret"
                                .to_string(),
                        ),
                        MockInfoFailure::HttpStatus => BtcRpcError::HttpStatus {
                            status: 403,
                            body_len: Some(64),
                            content_type: Some("text/plain".to_string()),
                        },
                        MockInfoFailure::JsonRpcSecret => BtcRpcError::JsonRpcError {
                            code: -32601,
                            message:
                                "Authorization: Bearer secret at http://user:pass@node.invalid"
                                    .to_string(),
                        },
                    });
                }
                Ok(BlockchainInfo {
                    blocks: TIP_HEIGHT,
                    bestblockhash: BEST_BLOCK_HASH.to_string(),
                    chain: self.chain.to_string(),
                    initialblockdownload: Some(false),
                })
            })
        }

        fn get_block_hash<'a>(&'a self, height: u64) -> BtcTransportFuture<'a, String> {
            Box::pin(async move {
                self.block_hash_calls.fetch_add(1, Ordering::Relaxed);
                if height == TIP_HEIGHT {
                    Ok(BEST_BLOCK_HASH.to_string())
                } else {
                    Ok(CONFIRMED_BLOCK_HASH.to_string())
                }
            })
        }

        fn get_block_header<'a>(
            &'a self,
            block_hash: &'a str,
        ) -> BtcTransportFuture<'a, BlockHeaderInfo> {
            Box::pin(async move {
                self.block_header_calls.fetch_add(1, Ordering::Relaxed);
                let height = if block_hash == CONFIRMED_BLOCK_HASH {
                    TIP_HEIGHT - 6
                } else {
                    TIP_HEIGHT
                };
                Ok(BlockHeaderInfo {
                    hash: block_hash.to_string(),
                    height,
                    time: 1_713_571_767,
                })
            })
        }

        fn scan_tx_out_set<'a>(
            &'a self,
            _address: &'a str,
        ) -> BtcTransportFuture<'a, ScanTxOutSetResult> {
            Box::pin(async move {
                self.scan_calls.fetch_add(1, Ordering::Relaxed);
                Ok(ScanTxOutSetResult {
                    success: true,
                    height: TIP_HEIGHT,
                    bestblock: BEST_BLOCK_HASH.to_string(),
                    total_amount_sats: 42,
                    unspents: Vec::new(),
                })
            })
        }
    }

    fn source_identity() -> BtcSourceIdentity {
        BtcSourceIdentity::new("public-bitcoin-core").expect("source")
    }

    fn source_binding() -> BtcSourceBinding {
        BtcSourceBinding::new(
            BtcNetworkId::new("bitcoin-mainnet").expect("network"),
            source_identity(),
            BitcoinNetworkTag::Main,
        )
        .expect("binding")
    }

    fn router_with(
        source_identity: BtcSourceIdentity,
        transport: Arc<MockBtcTransport>,
    ) -> BtcJsonRpcRouter {
        let mut routes = BTreeMap::new();
        routes.insert(
            source_identity,
            transport as Arc<dyn BtcJsonRpcChainHeadTransport>,
        );
        BtcJsonRpcRouter::from_transports(routes)
    }

    fn provider_with(
        source_identity: BtcSourceIdentity,
        transport: Arc<MockBtcTransport>,
    ) -> BtcJsonRpcSourceProvider {
        router_with(source_identity, transport)
            .bind_source(source_binding())
            .expect("bind source")
    }

    fn chain_head_request(selection: BtcHeadSelection) -> BtcChainHeadRequest {
        BtcChainHeadRequest::new(selection)
    }

    fn balance_request() -> BtcBalanceReadRequest {
        BtcBalanceReadRequest::new(
            BtcAddress::new("bc1qns9f7yfx3ry9lj6yz7c9er0vwa0ye2eklpzqfw").expect("address"),
            840_000,
            BtcBlockHash::new(BEST_BLOCK_HASH).expect("block hash"),
        )
    }

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
    fn config_debug_redacts_secret_bearing_fields() {
        let config = BtcJsonRpcConfig {
            rpc_url: "http://url_user:url_password@example.com:8332/rpc?api_key=query_secret&token=query_token#frag".to_string(),
            rpc_user: Some("rpc_user_secret".to_string()),
            rpc_password: Some("rpc_password_secret".to_string()),
        };

        let rendered = format!("{config:?}");

        assert!(rendered.contains("BtcJsonRpcConfig"));
        assert!(rendered.contains("http://example.com:8332"));
        assert!(!rendered.contains("url_user"));
        assert!(!rendered.contains("url_password"));
        assert!(!rendered.contains("api_key"));
        assert!(!rendered.contains("query_secret"));
        assert!(!rendered.contains("query_token"));
        assert!(!rendered.contains("rpc_user_secret"));
        assert!(!rendered.contains("rpc_password_secret"));
    }

    #[test]
    fn http_status_error_omits_raw_response_body() {
        let err = BtcRpcError::HttpStatus {
            status: 500,
            body_len: Some(
                "Authorization: Bearer body_token password=body_password token=body_secret".len(),
            ),
            content_type: Some("text/plain".to_string()),
        };

        let rendered = format!("{err}");
        let debug = format!("{err:?}");

        assert!(rendered.contains("btc rpc http status 500"));
        assert!(rendered.contains("body_len="));
        assert!(!rendered.contains("body_token"));
        assert!(!rendered.contains("body_password"));
        assert!(!rendered.contains("body_secret"));
        assert!(!debug.contains("body_token"));
        assert!(!debug.contains("body_password"));
        assert!(!debug.contains("body_secret"));
    }

    #[test]
    fn diagnostic_message_redacts_secret_patterns() {
        let rendered = BtcRpcError::JsonRpcError {
            code: -32603,
            message: diagnostic_message(
                "Authorization: Bearer auth_token api_key=query_secret password=rpc_password",
            ),
        }
        .to_string();

        assert!(rendered.contains(REDACTED_DIAGNOSTIC));
        assert!(!rendered.contains("auth_token"));
        assert!(!rendered.contains("query_secret"));
        assert!(!rendered.contains("rpc_password"));
    }

    #[test]
    fn content_type_metadata_redacts_secret_markers() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            CONTENT_TYPE,
            reqwest::header::HeaderValue::from_static("text/plain; token=header_secret"),
        );

        let rendered = sanitized_content_type(&headers).expect("content type should parse");

        assert_eq!(rendered, REDACTED_SECRET);
        assert!(!rendered.contains("header_secret"));
    }

    #[test]
    fn missing_source_binding_is_redacted() {
        let router = BtcJsonRpcRouter::from_transports(BTreeMap::new());
        let error = router
            .validate_source_binding(&source_binding())
            .expect_err("route should be missing");
        let rendered = format!("{error:?} {error}");

        assert!(matches!(error, BtcCapabilityError::Provider { .. }));
        assert!(rendered.contains("public-bitcoin-core"));
        assert!(!rendered.contains("http://"));
        assert!(!rendered.contains(concat!("Author", "ization")));
        assert!(!rendered.contains("secret"));
    }

    #[tokio::test]
    async fn observed_network_mismatch_fails_before_operation_rpc() {
        let transport = Arc::new(MockBtcTransport::new("test"));
        let provider = provider_with(source_identity(), transport.clone());
        let request = chain_head_request(BtcHeadSelection::confirmed(6).expect("selection"));

        let error = provider
            .read_chain_head(&request)
            .await
            .expect_err("source mismatch");
        let rendered = format!("{error:?} {error}");

        assert!(matches!(error, BtcCapabilityError::SourceMismatch { .. }));
        assert_eq!(transport.blockchain_info_calls.load(Ordering::Relaxed), 1);
        assert_eq!(transport.block_hash_calls.load(Ordering::Relaxed), 0);
        assert_eq!(transport.block_header_calls.load(Ordering::Relaxed), 0);
        assert_eq!(transport.scan_calls.load(Ordering::Relaxed), 0);
        assert!(!rendered.contains("http://"));
        assert!(!rendered.contains(concat!("Author", "ization")));
        assert!(!rendered.contains("secret"));
    }

    #[tokio::test]
    async fn provider_maps_best_and_confirmed_head_requests() {
        for (name, selection, expected_height, expected_hash, expected_hash_calls) in [
            (
                "best head",
                BtcHeadSelection::best(),
                TIP_HEIGHT,
                BEST_BLOCK_HASH,
                0,
            ),
            (
                "confirmed head",
                BtcHeadSelection::confirmed(6).expect("confirmed"),
                TIP_HEIGHT - 6,
                CONFIRMED_BLOCK_HASH,
                1,
            ),
        ] {
            let transport = Arc::new(MockBtcTransport::new("main"));
            let provider = provider_with(source_identity(), transport.clone());
            let request = chain_head_request(selection);
            let response = provider.read_chain_head(&request).await.expect(name);

            assert_eq!(response.block_height, expected_height, "{name}");
            assert_eq!(response.block_hash.as_str(), expected_hash, "{name}");
            assert_eq!(
                response.provider_time_unix_ms,
                Some(1_713_571_767_000),
                "{name}"
            );
            assert_eq!(
                transport.blockchain_info_calls.load(Ordering::Relaxed),
                1,
                "{name}"
            );
            assert_eq!(
                transport.block_hash_calls.load(Ordering::Relaxed),
                expected_hash_calls,
                "{name}"
            );
            assert_eq!(
                transport.block_header_calls.load(Ordering::Relaxed),
                1,
                "{name}"
            );
            assert_eq!(transport.scan_calls.load(Ordering::Relaxed), 0, "{name}");
        }
    }

    #[tokio::test]
    async fn successful_response_evidence_matches_binding_by_construction() {
        let transport = Arc::new(MockBtcTransport::new("main"));
        let provider = provider_with(source_identity(), transport.clone());
        let binding = source_binding();
        let request = chain_head_request(BtcHeadSelection::best());

        let response = provider
            .read_chain_head(&request)
            .await
            .expect("successful response");

        assert_eq!(response.evidence.network_id, binding.network_id().clone());
        assert_eq!(
            response.evidence.source_identity,
            binding.source_identity().clone()
        );
        assert_eq!(
            response.evidence.bitcoin_network,
            binding.bitcoin_network().as_str()
        );
        assert_eq!(
            response.evidence.observed_bitcoin_network,
            binding.bitcoin_network().as_str()
        );
        assert_eq!(response.head_kind, request.selection().head_kind());
        assert_eq!(response.finality, request.selection().finality());
        assert_eq!(response.block_height, TIP_HEIGHT);
        assert_eq!(response.block_hash.as_str(), BEST_BLOCK_HASH);
        assert_eq!(response.provider_time_unix_ms, Some(1_713_571_767_000));
        assert_eq!(transport.blockchain_info_calls.load(Ordering::Relaxed), 1);
        assert_eq!(transport.block_header_calls.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn balance_source_mismatch_fails_before_unsupported_operation() {
        let transport = Arc::new(MockBtcTransport::new("test"));
        let provider = provider_with(source_identity(), transport.clone());
        let request = balance_request();

        let error = provider
            .read_balance(&request)
            .await
            .expect_err("source mismatch");

        assert!(matches!(error, BtcCapabilityError::SourceMismatch { .. }));
        assert_eq!(transport.blockchain_info_calls.load(Ordering::Relaxed), 1);
        assert_eq!(transport.scan_calls.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn balance_rejects_exact_anchor_without_utxo_scan() {
        let transport = Arc::new(MockBtcTransport::new("main"));
        let provider = provider_with(source_identity(), transport.clone());
        let request = balance_request();

        let error = provider
            .read_balance(&request)
            .await
            .expect_err("exact anchored balance unsupported");
        let BtcCapabilityError::Provider { diagnostic } = error else {
            panic!("expected provider diagnostic");
        };

        assert_eq!(
            diagnostic.stable_error_code(),
            "bitcoin_unsupported_operation"
        );
        assert_eq!(transport.blockchain_info_calls.load(Ordering::Relaxed), 1);
        assert_eq!(transport.scan_calls.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn provider_classifies_http_failure_without_secrets() {
        let transport = Arc::new(MockBtcTransport::with_info_failure(MockInfoFailure::Http));
        let provider = provider_with(source_identity(), transport);
        let error = provider
            .read_chain_head(&chain_head_request(BtcHeadSelection::best()))
            .await
            .expect_err("provider failure");
        let rendered = format!("{error:?} {error}");
        let BtcCapabilityError::Provider { diagnostic } = error else {
            panic!("expected provider diagnostic");
        };

        assert_eq!(diagnostic.stable_error_code(), "bitcoin_transport_failed");
        assert_eq!(
            diagnostic.summary(),
            "transport_failed operation=getblockchaininfo"
        );
        assert!(!rendered.contains("localhost"));
        assert!(!rendered.contains("password"));
        assert!(!rendered.contains("secret"));
        assert!(!rendered.contains(concat!("Author", "ization")));
    }

    #[tokio::test]
    async fn provider_classifies_http_status_failure_without_body() {
        let transport = Arc::new(MockBtcTransport::with_info_failure(
            MockInfoFailure::HttpStatus,
        ));
        let provider = provider_with(source_identity(), transport);
        let error = provider
            .read_chain_head(&chain_head_request(BtcHeadSelection::best()))
            .await
            .expect_err("provider failure");
        let BtcCapabilityError::Provider { diagnostic } = error else {
            panic!("expected provider diagnostic");
        };

        assert_eq!(diagnostic.stable_error_code(), "bitcoin_rpc_http_status");
        assert_eq!(
            diagnostic.summary(),
            "rpc_http_status operation=getblockchaininfo http_status=403"
        );
    }

    #[tokio::test]
    async fn provider_classifies_json_rpc_failure_without_message() {
        let transport = Arc::new(MockBtcTransport::with_info_failure(
            MockInfoFailure::JsonRpcSecret,
        ));
        let provider = provider_with(source_identity(), transport);
        let error = provider
            .read_chain_head(&chain_head_request(BtcHeadSelection::best()))
            .await
            .expect_err("provider failure");
        let rendered = format!("{error:?} {error}");
        let BtcCapabilityError::Provider { diagnostic } = error else {
            panic!("expected provider diagnostic");
        };

        assert_eq!(diagnostic.stable_error_code(), "bitcoin_rpc_json_error");
        assert_eq!(
            diagnostic.summary(),
            "rpc_json_error operation=getblockchaininfo rpc_code=-32601"
        );
        assert!(!rendered.contains("http://"));
        assert!(!rendered.contains("user:pass"));
        assert!(!rendered.contains(concat!("Author", "ization")));
        assert!(!rendered.contains("secret"));
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
        assert_eq!(info.initialblockdownload, Some(false));
    }

    #[test]
    fn block_header_info_deserializes() {
        let json = serde_json::json!({
            "hash": "0000000000000000000320283a032748cef8227873ff4872689bf23f1cda83a5",
            "confirmations": 12,
            "height": 840000,
            "version": 536870912,
            "versionHex": "20000000",
            "merkleroot": "4d5e...",
            "time": 1713571767,
            "mediantime": 1713569060,
            "nonce": 0,
            "bits": "17034219",
            "difficulty": 83148355189239.77,
            "chainwork": "00000000000000000000000000000000000000007b48a3b73a8f3af5bf6d5e5e",
            "nTx": 3200,
            "previousblockhash": "0000000000000000000011111111111111111111111111111111111111111111",
        });
        let header: BlockHeaderInfo = serde_json::from_value(json).expect("deserialize");

        assert_eq!(header.height, 840000);
        assert_eq!(
            header.hash,
            "0000000000000000000320283a032748cef8227873ff4872689bf23f1cda83a5"
        );
        assert_eq!(header.time, 1713571767);
    }

    #[test]
    fn scan_tx_out_set_results_deserialize_populated_and_empty_responses() {
        for (case, json, expected_unspents, expected_total_sats) in [
            (
                "populated",
                r#"{
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
                }"#,
                1,
                5_000_000,
            ),
            (
                "empty",
                r#"{
                    "success": true,
                    "txouts": 120000000,
                    "height": 840000,
                    "bestblock": "0000000000000000000320283a032748cef8227873ff4872689bf23f1cda83a5",
                    "unspents": [],
                    "total_amount": 0
                }"#,
                0,
                0,
            ),
        ] {
            let result: ScanTxOutSetResult = serde_json::from_str(json).expect(case);

            assert!(result.success, "{case}");
            assert_eq!(result.unspents.len(), expected_unspents, "{case}");
            assert_eq!(result.total_amount_sats, expected_total_sats, "{case}");
            if case == "populated" {
                assert_eq!(result.unspents[0].txid, "abc123");
                assert_eq!(result.unspents[0].amount_sats, 1);
            }
        }
    }

    #[test]
    fn btc_amount_json_to_sats_parses_exact_satoshis() {
        for (amount, expected_sats) in [
            ("0.00000001", 1),
            ("0.05000000", 5_000_000),
            ("\"1.23000000\"", 123_000_000),
        ] {
            assert_eq!(
                btc_amount_json_to_sats(amount).unwrap(),
                expected_sats,
                "{amount}"
            );
        }
    }

    #[test]
    fn btc_amount_json_to_sats_rejects_invalid_amounts() {
        for (amount, expected_error) in [
            ("0.000000001", BtcAmountParseError::TooPrecise),
            ("-0.00000001", BtcAmountParseError::Negative),
            ("18446744073709551615", BtcAmountParseError::Overflow),
        ] {
            assert_eq!(btc_amount_json_to_sats(amount).unwrap_err(), expected_error);
        }
    }
}
