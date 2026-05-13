#![allow(clippy::disallowed_methods, clippy::disallowed_types)]
#![warn(missing_docs)]
//! Live `rpc.control` transport for managed EVM and Bitcoin RPC routing.
//!
//! This transport keeps `rpc.control` as the canonical state-facing ingress while reusing the
//! existing HTTP JSON-RPC executors internally. It owns:
//! - bootstrap source catalog parsing from env (EVM)
//! - durable `rpc_source:*` and `source_pool:*` updates (EVM)
//! - source probing and ranking (EVM)
//! - managed source selection for unpinned EVM calls
//! - Bitcoin `getblockchaininfo` and `scantxoutset` dispatch via `MFM_BTC_RPC_URL`
//!
//! The inner `evm` transport is treated as an executor only. Every managed EVM call is pinned to
//! one concrete source before dispatch, so route choice and cooldown authority stay here.
//! Bitcoin calls are dispatched directly to the configured Bitcoin Core endpoint.
//! Debug output redacts RPC URL credentials, query strings, and authorization headers.

use std::collections::{BTreeSet, HashMap};
use std::fmt;
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use serde::Deserialize;
use tracing::warn;

use mfm_collectors_btc_jsonrpc_http::{
    BlockchainInfo, BtcJsonRpcClient, BtcJsonRpcConfig, BtcRpcError,
};
use mfm_collectors_evm_jsonrpc_http::{
    EvmJsonRpcHttpConfig, EvmJsonRpcHttpTransportFactory, EvmJsonRpcSource, EvmRoutingStrategy,
    EvmSourceKind,
};
use mfm_collectors_rpc_control::{
    parse_u64_hex_value, EvmBroadcastRawTransactionResponse, PrepareSourcesResponse,
    PreparedSourceSummary, RpcControlRequest, NAMESPACE_RPC_CONTROL,
};
use mfm_control_plane_postgres::{
    rebuild_rpc_source_state, rebuild_source_pool_state, ControlPlanePostgresStore,
    RpcSourceObservedRecord, RpcSourceOutcome, RpcSourceProbeKind, RpcSourceProbedRecord,
    RpcSourceRecord, RpcSourceRef, RpcSourceState, SourcePoolCatalogDeclaredRecord,
    SourcePoolCatalogSnapshot, SourcePoolCatalogSource, SourcePoolMembershipDeclaredRecord,
    SourcePoolRankedRecord, SourcePoolRecord, SourcePoolRef, SourcePoolState,
    SOURCE_POOL_CATALOG_SCHEMA_VERSION,
};
use mfm_machine::errors::{ErrorCategory, ErrorInfo, IoError, StorageError};
use mfm_machine::ids::ErrorCode;
use mfm_machine::io::IoCall;
use mfm_machine::live_io::{LiveIoEnv, LiveIoTransport, LiveIoTransportFactory};
use mfm_machine::stores::{StreamAppend, StreamStore};

const ENV_EVM_RPC_SOURCES_JSON: &str = "MFM_EVM_RPC_SOURCES_JSON";
const ENV_EVM_RPC_PREFERRED_ORDER: &str = "MFM_EVM_RPC_PREFERRED_ORDER";
const ENV_EVM_RPC_REQUIRE_GET_PROOF_IDS: &str = "MFM_EVM_RPC_REQUIRE_GET_PROOF_IDS";
const ENV_BTC_RPC_URL: &str = "MFM_BTC_RPC_URL";
const ENV_BTC_RPC_USER: &str = "MFM_BTC_RPC_USER";
const ENV_BTC_RPC_PASSWORD: &str = "MFM_BTC_RPC_PASSWORD";
const REDACTED_SECRET: &str = "<redacted>";

const DEFAULT_POOL_KIND: &str = "default";
const PROBE_REFRESH_INTERVAL_MS: u64 = 15_000;
const FAILURE_COOLDOWN_MS: u64 = 15_000;

fn bitcoin_chain_for_network_id(network_id: &str) -> Option<&'static str> {
    match network_id {
        "bitcoin-mainnet" => Some("main"),
        "bitcoin-testnet" => Some("test"),
        "bitcoin-signet" => Some("signet"),
        "bitcoin-regtest" => Some("regtest"),
        _ => None,
    }
}

/// Control-plane persistence backend used by `rpc.control`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RpcControlPlaneStorageMode {
    /// Use the dedicated Postgres control-plane store discovered from `DATABASE_URL`.
    #[default]
    PostgresEnv,
    /// Persist control-plane stream families directly through the runtime `StreamStore`.
    StreamStore,
}

/// Typed tuning knobs forwarded to the inner EVM executor used by `rpc.control`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RpcControlExecutorTuning {
    /// Maximum block span per chunked `eth_getLogs` request.
    pub logs_max_block_span: u64,
    /// Minimum block span while shrinking retryable `eth_getLogs` chunks.
    pub logs_min_block_span: u64,
    /// Maximum number of `eth_getLogs` chunks attempted for one logical call.
    pub logs_max_chunks_per_call: u64,
}

impl RpcControlExecutorTuning {
    /// Creates explicit executor tuning for the inner `evm` transport.
    pub fn new(
        logs_max_block_span: u64,
        logs_min_block_span: u64,
        logs_max_chunks_per_call: u64,
    ) -> Self {
        Self {
            logs_max_block_span,
            logs_min_block_span,
            logs_max_chunks_per_call,
        }
    }
}

impl Default for RpcControlExecutorTuning {
    fn default() -> Self {
        Self {
            logs_max_block_span: 2_000,
            logs_min_block_span: 64,
            logs_max_chunks_per_call: 256,
        }
    }
}

fn info(
    code: &'static str,
    category: ErrorCategory,
    retryable: bool,
    message: impl Into<String>,
) -> ErrorInfo {
    ErrorInfo {
        code: ErrorCode::must_new(code),
        category,
        retryable,
        message: message.into(),
        details: None,
    }
}

fn io_other(
    code: &'static str,
    category: ErrorCategory,
    retryable: bool,
    message: impl Into<String>,
) -> IoError {
    IoError::Other(info(code, category, retryable, message))
}

fn io_transport(
    code: &'static str,
    category: ErrorCategory,
    retryable: bool,
    message: impl Into<String>,
) -> IoError {
    IoError::Transport(info(code, category, retryable, message))
}

fn io_from_storage(err: StorageError) -> IoError {
    match err {
        StorageError::Concurrency(info) => IoError::Other(info),
        StorageError::NotFound(info) => IoError::Other(info),
        StorageError::Corruption(info) => IoError::Other(info),
        StorageError::Other(info) => IoError::Other(info),
    }
}

fn io_projection_invalid(
    code: &'static str,
    stream_id: &str,
    err: impl std::fmt::Display,
) -> IoError {
    io_other(
        code,
        ErrorCategory::Storage,
        false,
        format!("invalid control-plane projection for `{stream_id}`: {err}"),
    )
}

fn io_control_plane_concurrency(info: ErrorInfo) -> IoError {
    IoError::Other(ErrorInfo {
        code: ErrorCode::must_new("control_plane_concurrency"),
        category: info.category,
        retryable: info.retryable,
        message: info.message,
        details: info.details,
    })
}

fn io_error_code(err: &IoError) -> String {
    match err {
        IoError::MissingFactKey(info)
        | IoError::Transport(info)
        | IoError::RateLimited(info)
        | IoError::Other(info) => info.code.as_str().to_string(),
        IoError::MissingFact { info, .. } => info.code.as_str().to_string(),
    }
}

fn io_error_details(err: &IoError) -> Option<&serde_json::Value> {
    match err {
        IoError::MissingFactKey(info)
        | IoError::Transport(info)
        | IoError::RateLimited(info)
        | IoError::Other(info) => info.details.as_ref(),
        IoError::MissingFact { info, .. } => info.details.as_ref(),
    }
}

fn jsonrpc_error_message(err: &IoError) -> Option<String> {
    io_error_details(err)?
        .get("jsonrpc_error_message")
        .and_then(|value| value.as_str())
        .map(|value| value.to_ascii_lowercase())
}

fn normalize_tx_hash(raw: &str) -> Result<String, IoError> {
    let trimmed = raw.trim();
    let Some(rest) = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
    else {
        return Err(io_transport(
            "evm_tx_hash_invalid",
            ErrorCategory::ParsingInput,
            false,
            "transaction hash must be 0x-prefixed hex",
        ));
    };
    if rest.len() != 64 || !rest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(io_transport(
            "evm_tx_hash_invalid",
            ErrorCategory::ParsingInput,
            false,
            "transaction hash must be exactly 32 bytes",
        ));
    }
    Ok(format!("0x{}", rest.to_ascii_lowercase()))
}

fn is_already_known_error(err: &IoError) -> bool {
    let Some(message) = jsonrpc_error_message(err) else {
        return false;
    };
    message.contains("already known")
        || message.contains("already imported")
        || message.contains("known transaction")
        || message.contains("transaction already")
}

fn is_nonce_too_low_error(err: &IoError) -> bool {
    let Some(message) = jsonrpc_error_message(err) else {
        return false;
    };
    message.contains("nonce too low")
        || message.contains("nonce has already been used")
        || message.contains("already used")
}

fn now_ms() -> Result<u64, IoError> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| {
            io_other(
                "time_unavailable",
                ErrorCategory::Unknown,
                false,
                "system time is not available",
            )
        })?
        .as_millis() as u64)
}

fn parse_csv_env(var_name: &str) -> Vec<String> {
    std::env::var(var_name)
        .ok()
        .into_iter()
        .flat_map(|raw| CsvValues::from_raw(&raw).into_iter())
        .collect()
}

#[derive(Clone, Debug)]
struct CsvValues;

impl CsvValues {
    fn from_raw(raw: &str) -> Vec<String> {
        raw.split(',')
            .map(str::trim)
            .map(str::to_string)
            .filter(|value| !value.is_empty())
            .collect()
    }
}

fn parse_source_kind(raw: Option<&str>) -> EvmSourceKind {
    let normalized = raw.unwrap_or("remote_public").trim().to_ascii_lowercase();
    match normalized.as_str() {
        "local" | "local_reth" => EvmSourceKind::Local,
        "remote_user" | "user" => EvmSourceKind::RemoteUser,
        "remote_public" | "public" => EvmSourceKind::RemotePublic,
        _ => EvmSourceKind::RemotePublic,
    }
}

fn kind_rank(kind: EvmSourceKind) -> u8 {
    match kind {
        EvmSourceKind::Local => 0,
        EvmSourceKind::RemoteUser => 1,
        EvmSourceKind::RemotePublic => 2,
    }
}

fn source_kind_label(kind: EvmSourceKind) -> &'static str {
    match kind {
        EvmSourceKind::Local => "local",
        EvmSourceKind::RemoteUser => "remote_user",
        EvmSourceKind::RemotePublic => "remote_public",
    }
}

fn normalize_optional_field(raw: Option<String>) -> Option<String> {
    raw.map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn redacted_optional_secret(value: &Option<String>) -> &'static str {
    if value.is_some() {
        REDACTED_SECRET
    } else {
        "<unset>"
    }
}

fn redacted_rpc_url(raw: &str) -> String {
    let Ok(parsed) = url::Url::parse(raw) else {
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

    match parsed.port_or_known_default() {
        Some(port) => format!("{}://{}:{}", parsed.scheme(), host, port),
        None => format!("{}://{}", parsed.scheme(), host),
    }
}

/// Bootstrap source definition used by the `rpc.control` transport.
#[derive(Clone, PartialEq, Eq)]
pub struct RpcControlBootstrapSource {
    /// Stable source identifier.
    pub id: String,
    /// Stable network identifier.
    pub network_id: Option<String>,
    /// Full RPC URL.
    pub rpc_url: String,
    /// Optional authorization header value.
    pub authorization: Option<String>,
    /// Source kind used during ranking.
    pub kind: EvmSourceKind,
    /// Whether the source must pass an `eth_getProof` capability probe before normal selection.
    pub require_get_proof_probe: bool,
}

impl fmt::Debug for RpcControlBootstrapSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RpcControlBootstrapSource")
            .field("id", &self.id)
            .field("network_id", &self.network_id)
            .field("rpc_url", &redacted_rpc_url(&self.rpc_url))
            .field(
                "authorization",
                &redacted_optional_secret(&self.authorization),
            )
            .field("kind", &self.kind)
            .field("require_get_proof_probe", &self.require_get_proof_probe)
            .finish()
    }
}

#[derive(Clone, Debug, Default)]
struct BootstrapCatalog {
    sources: Vec<RpcControlBootstrapSource>,
    preferred_order: Vec<String>,
}

#[derive(Clone)]
struct StreamBackedControlPlaneStore {
    streams: Arc<dyn StreamStore>,
}

impl StreamBackedControlPlaneStore {
    fn new(streams: Arc<dyn StreamStore>) -> Self {
        Self { streams }
    }

    async fn read_all_records(
        &self,
        stream_id: &mfm_machine::stores::StreamId,
    ) -> Result<Option<Vec<mfm_machine::stores::StreamRecord>>, IoError> {
        let head_seq = match self.streams.head_seq(stream_id).await {
            Ok(head_seq) => head_seq,
            Err(StorageError::NotFound(_)) => return Ok(None),
            Err(err) => return Err(io_from_storage(err)),
        };
        if head_seq == 0 {
            return Ok(None);
        }
        self.streams
            .read_range(stream_id, 1, Some(head_seq))
            .await
            .map(Some)
            .map_err(io_from_storage)
    }

    async fn rpc_source_state(
        &self,
        source_ref: &RpcSourceRef,
    ) -> Result<Option<RpcSourceState>, IoError> {
        let stream_id = source_ref.stream_id();
        let Some(records) = self.read_all_records(&stream_id).await? else {
            return Ok(None);
        };
        rebuild_rpc_source_state(source_ref.clone(), &records)
            .map(Some)
            .map_err(|err| {
                io_projection_invalid("control_plane_projection_invalid", stream_id.as_str(), err)
            })
    }

    async fn append_rpc_source_records(
        &self,
        source_ref: &RpcSourceRef,
        expected_seq: u64,
        records: Vec<RpcSourceRecord>,
    ) -> Result<RpcSourceState, IoError> {
        if records.is_empty() {
            return Err(io_other(
                "control_plane_append_invalid",
                ErrorCategory::Storage,
                false,
                "rpc_source append must include at least one record",
            ));
        }

        let stream_id = source_ref.stream_id();
        let encoded_records = records
            .iter()
            .map(RpcSourceRecord::to_new_stream_record)
            .collect::<Result<Vec<_>, _>>()
            .map_err(io_from_storage)?;
        let head_seq = self
            .streams
            .append(StreamAppend::new(
                stream_id.clone(),
                expected_seq,
                encoded_records,
            ))
            .await
            .map_err(|err| match err {
                StorageError::Concurrency(info) => io_control_plane_concurrency(info),
                other => io_from_storage(other),
            })?;
        let records = self
            .streams
            .read_range(&stream_id, 1, Some(head_seq))
            .await
            .map_err(io_from_storage)?;
        rebuild_rpc_source_state(source_ref.clone(), &records).map_err(|err| {
            io_projection_invalid("control_plane_projection_invalid", stream_id.as_str(), err)
        })
    }

    async fn source_pool_state(
        &self,
        pool_ref: &SourcePoolRef,
    ) -> Result<Option<SourcePoolState>, IoError> {
        let stream_id = pool_ref.stream_id();
        let Some(records) = self.read_all_records(&stream_id).await? else {
            return Ok(None);
        };
        rebuild_source_pool_state(pool_ref.clone(), &records)
            .map(Some)
            .map_err(|err| {
                io_projection_invalid("control_plane_projection_invalid", stream_id.as_str(), err)
            })
    }

    async fn append_source_pool_records(
        &self,
        pool_ref: &SourcePoolRef,
        expected_seq: u64,
        records: Vec<SourcePoolRecord>,
    ) -> Result<SourcePoolState, IoError> {
        if records.is_empty() {
            return Err(io_other(
                "control_plane_append_invalid",
                ErrorCategory::Storage,
                false,
                "source_pool append must include at least one record",
            ));
        }

        let stream_id = pool_ref.stream_id();
        let encoded_records = records
            .iter()
            .map(SourcePoolRecord::to_new_stream_record)
            .collect::<Result<Vec<_>, _>>()
            .map_err(io_from_storage)?;
        let head_seq = self
            .streams
            .append(StreamAppend::new(
                stream_id.clone(),
                expected_seq,
                encoded_records,
            ))
            .await
            .map_err(|err| match err {
                StorageError::Concurrency(info) => io_control_plane_concurrency(info),
                other => io_from_storage(other),
            })?;
        let records = self
            .streams
            .read_range(&stream_id, 1, Some(head_seq))
            .await
            .map_err(io_from_storage)?;
        rebuild_source_pool_state(pool_ref.clone(), &records).map_err(|err| {
            io_projection_invalid("control_plane_projection_invalid", stream_id.as_str(), err)
        })
    }
}

enum ControlPlaneStore {
    PostgresEnv {
        store: Option<ControlPlanePostgresStore>,
    },
    StreamBacked(StreamBackedControlPlaneStore),
}

impl ControlPlaneStore {
    fn from_mode(mode: RpcControlPlaneStorageMode, streams: Arc<dyn StreamStore>) -> Self {
        match mode {
            RpcControlPlaneStorageMode::PostgresEnv => Self::PostgresEnv { store: None },
            RpcControlPlaneStorageMode::StreamStore => {
                Self::StreamBacked(StreamBackedControlPlaneStore::new(streams))
            }
        }
    }

    async fn rpc_source_state(
        &mut self,
        source_ref: &RpcSourceRef,
    ) -> Result<Option<RpcSourceState>, IoError> {
        match self {
            ControlPlaneStore::PostgresEnv { store } => {
                let store = ensure_postgres_control_plane_store(store).await?;
                store
                    .rpc_source_state(source_ref)
                    .await
                    .map_err(io_from_storage)
            }
            ControlPlaneStore::StreamBacked(store) => store.rpc_source_state(source_ref).await,
        }
    }

    async fn append_rpc_source_records(
        &mut self,
        source_ref: &RpcSourceRef,
        expected_seq: u64,
        records: Vec<RpcSourceRecord>,
    ) -> Result<RpcSourceState, IoError> {
        match self {
            ControlPlaneStore::PostgresEnv { store } => {
                let store = ensure_postgres_control_plane_store(store).await?;
                store
                    .append_rpc_source_records(source_ref, expected_seq, records)
                    .await
                    .map_err(io_from_storage)
            }
            ControlPlaneStore::StreamBacked(store) => {
                store
                    .append_rpc_source_records(source_ref, expected_seq, records)
                    .await
            }
        }
    }

    async fn source_pool_state(
        &mut self,
        pool_ref: &SourcePoolRef,
    ) -> Result<Option<SourcePoolState>, IoError> {
        match self {
            ControlPlaneStore::PostgresEnv { store } => {
                let store = ensure_postgres_control_plane_store(store).await?;
                store
                    .source_pool_state(pool_ref)
                    .await
                    .map_err(io_from_storage)
            }
            ControlPlaneStore::StreamBacked(store) => store.source_pool_state(pool_ref).await,
        }
    }

    async fn append_source_pool_records(
        &mut self,
        pool_ref: &SourcePoolRef,
        expected_seq: u64,
        records: Vec<SourcePoolRecord>,
    ) -> Result<SourcePoolState, IoError> {
        match self {
            ControlPlaneStore::PostgresEnv { store } => {
                let store = ensure_postgres_control_plane_store(store).await?;
                store
                    .append_source_pool_records(pool_ref, expected_seq, records)
                    .await
                    .map_err(io_from_storage)
            }
            ControlPlaneStore::StreamBacked(store) => {
                store
                    .append_source_pool_records(pool_ref, expected_seq, records)
                    .await
            }
        }
    }
}

async fn ensure_postgres_control_plane_store(
    store: &mut Option<ControlPlanePostgresStore>,
) -> Result<ControlPlanePostgresStore, IoError> {
    if let Some(store) = store {
        return Ok(store.clone());
    }

    let connected = ControlPlanePostgresStore::connect_env()
        .await
        .map_err(io_from_storage)?;
    *store = Some(connected.clone());
    Ok(connected)
}

/// Validation errors for the bootstrap `rpc.control` source catalog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RpcControlConfigError {
    /// No bootstrap sources were configured.
    NoSources,
    /// A configured source id was empty.
    EmptySourceId,
    /// Two configured sources shared the same id.
    DuplicateSourceId(String),
    /// Preferred ordering referenced an unknown source id.
    UnknownPreferredSourceId(String),
    /// A configured source omitted `network_id`.
    MissingNetworkId(String),
    /// The inner EVM executor factory could not be constructed.
    ExecutorConfig(String),
}

impl std::fmt::Display for RpcControlConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RpcControlConfigError::NoSources => {
                write!(f, "rpc.control bootstrap source registry is empty")
            }
            RpcControlConfigError::EmptySourceId => {
                write!(f, "rpc.control source id must not be empty")
            }
            RpcControlConfigError::DuplicateSourceId(source_id) => {
                write!(f, "duplicate rpc.control source id: {source_id}")
            }
            RpcControlConfigError::UnknownPreferredSourceId(source_id) => {
                write!(f, "preferred source id not found in registry: {source_id}")
            }
            RpcControlConfigError::MissingNetworkId(source_id) => {
                write!(
                    f,
                    "rpc.control source `{source_id}` must declare network_id"
                )
            }
            RpcControlConfigError::ExecutorConfig(reason) => {
                write!(
                    f,
                    "rpc.control evm executor configuration invalid: {reason}"
                )
            }
        }
    }
}

impl std::error::Error for RpcControlConfigError {}

#[derive(Clone, Deserialize)]
struct EnvBootstrapSource {
    id: String,
    #[serde(default)]
    network_id: Option<String>,
    rpc_url: String,
    #[serde(default)]
    authorization: Option<String>,
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    require_get_proof_probe: bool,
}

fn validate_catalog(catalog: &BootstrapCatalog) -> Result<(), RpcControlConfigError> {
    if catalog.sources.is_empty() {
        return Err(RpcControlConfigError::NoSources);
    }

    let mut seen = BTreeSet::new();
    for source in &catalog.sources {
        if source.id.trim().is_empty() {
            return Err(RpcControlConfigError::EmptySourceId);
        }
        if source
            .network_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .is_none()
        {
            return Err(RpcControlConfigError::MissingNetworkId(source.id.clone()));
        }
        if !seen.insert(source.id.as_str()) {
            return Err(RpcControlConfigError::DuplicateSourceId(source.id.clone()));
        }
    }

    for source_id in &catalog.preferred_order {
        if !seen.contains(source_id.as_str()) {
            return Err(RpcControlConfigError::UnknownPreferredSourceId(
                source_id.clone(),
            ));
        }
    }

    Ok(())
}

fn parse_bootstrap_sources_from_json(raw_json: &str) -> Vec<RpcControlBootstrapSource> {
    let Ok(parsed) = serde_json::from_str::<Vec<EnvBootstrapSource>>(raw_json) else {
        warn!("failed to parse MFM_EVM_RPC_SOURCES_JSON for rpc.control bootstrap");
        return Vec::new();
    };

    parsed
        .into_iter()
        .filter_map(|source| {
            let id = source.id.trim().to_string();
            let rpc_url = source.rpc_url.trim().to_string();
            if id.is_empty() || rpc_url.is_empty() {
                warn!("skipping rpc.control bootstrap source with empty id or rpc_url");
                return None;
            }

            Some(RpcControlBootstrapSource {
                id,
                network_id: normalize_optional_field(source.network_id),
                rpc_url,
                authorization: normalize_optional_field(source.authorization),
                kind: parse_source_kind(source.kind.as_deref()),
                require_get_proof_probe: source.require_get_proof_probe,
            })
        })
        .collect()
}

/// Resolves the bootstrap `rpc.control` source registry from supported environment variables.
pub fn resolve_rpc_control_bootstrap_sources_from_env() -> Vec<RpcControlBootstrapSource> {
    let mut sources = if let Ok(raw_json) = std::env::var(ENV_EVM_RPC_SOURCES_JSON) {
        let trimmed = raw_json.trim();
        if trimmed.is_empty() {
            Vec::new()
        } else {
            parse_bootstrap_sources_from_json(trimmed)
        }
    } else {
        Vec::new()
    };

    let require_get_proof_ids = parse_csv_env(ENV_EVM_RPC_REQUIRE_GET_PROOF_IDS)
        .into_iter()
        .collect::<BTreeSet<_>>();
    for source in &mut sources {
        if require_get_proof_ids.contains(source.id.as_str()) {
            source.require_get_proof_probe = true;
        }
    }

    sources
}

fn resolve_bootstrap_catalog_from_env() -> BootstrapCatalog {
    let sources = resolve_rpc_control_bootstrap_sources_from_env();
    let mut preferred_order = parse_csv_env(ENV_EVM_RPC_PREFERRED_ORDER);
    if preferred_order.is_empty() {
        preferred_order = sources.iter().map(|source| source.id.clone()).collect();
    }

    BootstrapCatalog {
        sources,
        preferred_order,
    }
}

fn inner_executor_config(
    catalog: &BootstrapCatalog,
    tuning: RpcControlExecutorTuning,
) -> EvmJsonRpcHttpConfig {
    let sources = catalog
        .sources
        .iter()
        .map(|source| EvmJsonRpcSource {
            id: source.id.clone(),
            rpc_url: source.rpc_url.clone(),
            authorization: source.authorization.clone(),
            kind: source.kind,
            require_get_proof_probe: false,
        })
        .collect();

    EvmJsonRpcHttpConfig {
        sources,
        preferred_order: catalog.preferred_order.clone(),
        strategy: EvmRoutingStrategy::Failover,
        unhealthy_cooldown_calls: 0,
        logs_max_block_span: tuning.logs_max_block_span,
        logs_min_block_span: tuning.logs_min_block_span,
        logs_max_chunks_per_call: tuning.logs_max_chunks_per_call,
        ..EvmJsonRpcHttpConfig::default()
    }
}

/// Live transport factory for the `rpc.control` namespace.
#[derive(Clone)]
pub struct RpcControlTransportFactory {
    catalog: BootstrapCatalog,
    config_error: Option<RpcControlConfigError>,
    control_plane_storage_mode: RpcControlPlaneStorageMode,
    executor_tuning: RpcControlExecutorTuning,
}

impl RpcControlTransportFactory {
    /// Builds a transport factory from the supplied bootstrap catalog.
    pub fn new(catalog: Vec<RpcControlBootstrapSource>) -> Self {
        let mut preferred_order = catalog.iter().map(|source| source.id.clone()).collect();
        let catalog = BootstrapCatalog {
            sources: catalog,
            preferred_order: std::mem::take(&mut preferred_order),
        };
        let config_error = validate_catalog(&catalog).err();
        Self {
            catalog,
            config_error,
            control_plane_storage_mode: RpcControlPlaneStorageMode::default(),
            executor_tuning: RpcControlExecutorTuning::default(),
        }
    }

    /// Builds a transport factory from environment-derived bootstrap config.
    pub fn from_env() -> Result<Self, RpcControlConfigError> {
        let catalog = resolve_bootstrap_catalog_from_env();
        let config_error = validate_catalog(&catalog).err();
        let factory = Self {
            catalog,
            config_error,
            control_plane_storage_mode: RpcControlPlaneStorageMode::default(),
            executor_tuning: RpcControlExecutorTuning::default(),
        };
        factory.validate_inner_executor_factory()?;
        Ok(factory)
    }

    /// Returns a copy of the factory configured to use `mode` for control-plane persistence.
    pub fn with_control_plane_storage_mode(mut self, mode: RpcControlPlaneStorageMode) -> Self {
        self.control_plane_storage_mode = mode;
        self
    }

    /// Returns a copy of the factory configured with explicit inner-executor tuning.
    pub fn with_executor_tuning(mut self, tuning: RpcControlExecutorTuning) -> Self {
        self.executor_tuning = tuning;
        self
    }

    /// Returns the bootstrap configuration error, if any.
    pub fn config_error(&self) -> Option<&RpcControlConfigError> {
        self.config_error.as_ref()
    }

    fn validate_inner_executor_factory(&self) -> Result<(), RpcControlConfigError> {
        if self.config_error.is_some() {
            return Ok(());
        }
        EvmJsonRpcHttpTransportFactory::try_new(inner_executor_config(
            &self.catalog,
            self.executor_tuning,
        ))
        .map(|_| ())
        .map_err(|err| RpcControlConfigError::ExecutorConfig(err.to_string()))
    }
}

impl LiveIoTransportFactory for RpcControlTransportFactory {
    fn namespace_group(&self) -> &str {
        "rpc.control"
    }

    fn make(&self, env: LiveIoEnv) -> Box<dyn LiveIoTransport> {
        let control_plane_store = ControlPlaneStore::from_mode(
            self.control_plane_storage_mode,
            Arc::clone(&env.stores.streams),
        );
        let (executor, executor_error) = if self.config_error.is_none() {
            match EvmJsonRpcHttpTransportFactory::try_new(inner_executor_config(
                &self.catalog,
                self.executor_tuning,
            )) {
                Ok(factory) => (Some(factory.make(env)), None),
                Err(err) => (None, Some(err.to_string())),
            }
        } else {
            (None, None)
        };

        let btc_client = std::env::var(ENV_BTC_RPC_URL).ok().map(|rpc_url| {
            BtcJsonRpcClient::new(BtcJsonRpcConfig {
                rpc_url,
                rpc_user: std::env::var(ENV_BTC_RPC_USER).ok(),
                rpc_password: std::env::var(ENV_BTC_RPC_PASSWORD).ok(),
            })
        });

        Box::new(RpcControlTransport {
            executor,
            executor_error,
            btc_client,
            control_plane_store,
            catalog: self.catalog.clone(),
            config_error: self.config_error.clone(),
        })
    }
}

struct RpcControlTransport {
    executor: Option<Box<dyn LiveIoTransport>>,
    executor_error: Option<String>,
    btc_client: Option<Result<BtcJsonRpcClient, BtcRpcError>>,
    control_plane_store: ControlPlaneStore,
    catalog: BootstrapCatalog,
    config_error: Option<RpcControlConfigError>,
}

#[derive(Clone, Debug)]
struct RankedSource {
    source: RpcControlBootstrapSource,
    state: Option<RpcSourceState>,
    healthy: bool,
}

impl RpcControlTransport {
    fn ensure_executor(&mut self) -> Result<&mut (dyn LiveIoTransport + '_), IoError> {
        match &self.config_error {
            Some(err) => Err(io_transport(
                "rpc_control_config_invalid",
                ErrorCategory::ParsingInput,
                false,
                err.to_string(),
            )),
            None if self.executor_error.is_some() => Err(io_transport(
                "rpc_control_executor_config_invalid",
                ErrorCategory::ParsingInput,
                false,
                format!(
                    "rpc.control evm executor configuration invalid: {}",
                    self.executor_error.as_deref().unwrap_or("unknown error")
                ),
            )),
            None => match self.executor.as_mut() {
                Some(executor) => Ok(executor.as_mut()),
                None => Err(io_transport(
                    "rpc_control_executor_missing",
                    ErrorCategory::Unknown,
                    false,
                    "rpc.control executor is not configured",
                )),
            },
        }
    }

    fn ordered_candidate_sources(
        &self,
        network_id: &str,
    ) -> Result<Vec<RpcControlBootstrapSource>, IoError> {
        let sources = self
            .catalog
            .sources
            .iter()
            .filter(|source| source.network_id.as_deref() == Some(network_id))
            .cloned()
            .collect::<Vec<_>>();
        if sources.is_empty() {
            return Err(io_transport(
                "rpc_control_no_sources",
                ErrorCategory::Unknown,
                false,
                format!("no bootstrap sources configured for network `{network_id}`"),
            ));
        }

        let mut by_id = sources
            .into_iter()
            .map(|source| (source.id.clone(), source))
            .collect::<HashMap<_, _>>();
        let mut ordered = Vec::new();
        for source_id in &self.catalog.preferred_order {
            if let Some(source) = by_id.remove(source_id) {
                ordered.push(source);
            }
        }

        let mut remaining = by_id.into_values().collect::<Vec<_>>();
        remaining.sort_by(|left, right| left.id.cmp(&right.id));
        ordered.extend(remaining);
        Ok(ordered)
    }

    fn catalog_snapshot_for_pool(
        &self,
        control_scope: &str,
        network_id: &str,
        pool_kind: &str,
        candidates: &[RpcControlBootstrapSource],
    ) -> Result<SourcePoolCatalogSnapshot, IoError> {
        let candidate_ids = candidates
            .iter()
            .map(|source| source.id.as_str())
            .collect::<BTreeSet<_>>();
        let mut sources = candidates
            .iter()
            .map(|source| SourcePoolCatalogSource {
                id: source.id.clone(),
                kind: source_kind_label(source.kind).to_string(),
                require_get_proof_probe: source.require_get_proof_probe,
            })
            .collect::<Vec<_>>();
        sources.sort_by(|left, right| left.id.cmp(&right.id));

        let preferred_source_ids = self
            .catalog
            .preferred_order
            .iter()
            .filter(|source_id| candidate_ids.contains(source_id.as_str()))
            .cloned()
            .collect::<Vec<_>>();

        let snapshot = SourcePoolCatalogSnapshot {
            schema_version: SOURCE_POOL_CATALOG_SCHEMA_VERSION,
            control_scope: control_scope.to_string(),
            network_id: network_id.to_string(),
            pool_kind: pool_kind.to_string(),
            sources,
            preferred_source_ids,
        };
        snapshot.validate().map_err(|message| {
            io_transport(
                "rpc_control_catalog_invalid",
                ErrorCategory::ParsingInput,
                false,
                format!("invalid rpc.control catalog snapshot: {message}"),
            )
        })?;
        Ok(snapshot)
    }

    async fn ensure_declared_catalog(
        &mut self,
        pool_ref: &SourcePoolRef,
        catalog_snapshot: &SourcePoolCatalogSnapshot,
    ) -> Result<SourcePoolState, IoError> {
        let catalog_fingerprint = catalog_snapshot.fingerprint().map_err(|err| {
            io_transport(
                "rpc_control_catalog_invalid",
                ErrorCategory::ParsingInput,
                false,
                format!("rpc.control catalog snapshot was not canonical-json-hashable: {err}"),
            )
        })?;
        let current_pool = self.control_plane_store.source_pool_state(pool_ref).await?;
        if let Some(current_pool) = current_pool {
            match current_pool.catalog_fingerprint.as_deref() {
                Some(current) if current == catalog_fingerprint => return Ok(current_pool),
                Some(current) => {
                    return Err(io_other(
                        "rpc_control_catalog_mismatch",
                        ErrorCategory::Storage,
                        false,
                        format!(
                            "rpc.control catalog mismatch for scope `{}` network `{}` pool `{}`: declared fingerprint `{current}` did not match current fingerprint `{catalog_fingerprint}`",
                            pool_ref.control_scope(),
                            pool_ref.network_id(),
                            pool_ref.pool_kind(),
                        ),
                    ));
                }
                None => {}
            }
        }

        self.append_source_pool_records_retry(
            pool_ref,
            vec![SourcePoolRecord::CatalogDeclared(
                SourcePoolCatalogDeclaredRecord {
                    declared_at_ms: now_ms()?,
                    catalog_fingerprint,
                    catalog_snapshot: catalog_snapshot.clone(),
                },
            )],
        )
        .await
    }

    fn resolve_network_scope(&self, requested_network_id: Option<&str>) -> Result<String, IoError> {
        if let Some(network_id) = requested_network_id {
            let trimmed = network_id.trim();
            if trimmed.is_empty() {
                return Err(io_transport(
                    "rpc_control_network_invalid",
                    ErrorCategory::ParsingInput,
                    false,
                    "network_id must not be empty",
                ));
            }
            return Ok(trimmed.to_string());
        }

        Err(io_transport(
            "rpc_control_network_required",
            ErrorCategory::ParsingInput,
            false,
            "network_id is required for canonical rpc.control managed calls",
        ))
    }

    async fn execute_inner(
        &mut self,
        source_id: &str,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, IoError> {
        let call = IoCall {
            namespace: "evm".to_string(),
            request: serde_json::json!({
                "method": method,
                "params": params,
                "route": {
                    "source_id": source_id,
                },
            }),
            fact_key: None,
        };

        self.ensure_executor()?.call(call).await
    }

    async fn append_source_records_retry(
        &mut self,
        source_ref: &RpcSourceRef,
        records: Vec<RpcSourceRecord>,
    ) -> Result<RpcSourceState, IoError> {
        let mut last_err = None;
        for _ in 0..3 {
            let expected_seq = self
                .control_plane_store
                .rpc_source_state(source_ref)
                .await?
                .map(|state| state.head_seq)
                .unwrap_or(0);
            match self
                .control_plane_store
                .append_rpc_source_records(source_ref, expected_seq, records.clone())
                .await
            {
                Ok(state) => return Ok(state),
                Err(IoError::Other(info)) if info.code.as_str() == "control_plane_concurrency" => {
                    last_err = Some(IoError::Other(info))
                }
                Err(err) => return Err(err),
            }
        }

        Err(last_err.unwrap_or_else(|| {
            io_other(
                "rpc_control_append_retry_exhausted",
                ErrorCategory::Storage,
                true,
                "rpc.control source append retry exhausted",
            )
        }))
    }

    async fn append_source_pool_records_retry(
        &mut self,
        pool_ref: &SourcePoolRef,
        records: Vec<SourcePoolRecord>,
    ) -> Result<SourcePoolState, IoError> {
        let mut last_err = None;
        for _ in 0..3 {
            let expected_seq = self
                .control_plane_store
                .source_pool_state(pool_ref)
                .await?
                .map(|state| state.head_seq)
                .unwrap_or(0);
            match self
                .control_plane_store
                .append_source_pool_records(pool_ref, expected_seq, records.clone())
                .await
            {
                Ok(state) => return Ok(state),
                Err(IoError::Other(info)) if info.code.as_str() == "control_plane_concurrency" => {
                    last_err = Some(IoError::Other(info))
                }
                Err(err) => return Err(err),
            }
        }

        Err(last_err.unwrap_or_else(|| {
            io_other(
                "rpc_control_append_retry_exhausted",
                ErrorCategory::Storage,
                true,
                "rpc.control source pool append retry exhausted",
            )
        }))
    }

    async fn append_runtime_observation(
        &mut self,
        control_scope: &str,
        network_scope: &str,
        source_id: &str,
        method: &str,
        response: Result<(&serde_json::Value, u64), (&IoError, u64)>,
    ) {
        let Ok(source_ref) = RpcSourceRef::new(control_scope, network_scope, source_id) else {
            return;
        };

        let observed_at_ms = match now_ms() {
            Ok(value) => value,
            Err(_) => return,
        };

        let record = match response {
            Ok((value, latency_ms)) => RpcSourceRecord::Observed(RpcSourceObservedRecord {
                observed_at_ms,
                outcome: RpcSourceOutcome::Success,
                head_block_number: if method == "eth_blockNumber" {
                    parse_u64_hex_value(value).ok()
                } else {
                    None
                },
                latency_ms: Some(latency_ms),
                cooldown_until_ms: None,
                diagnostic_code: None,
            }),
            Err((err, latency_ms)) => RpcSourceRecord::Observed(RpcSourceObservedRecord {
                observed_at_ms,
                outcome: RpcSourceOutcome::Failure,
                head_block_number: None,
                latency_ms: Some(latency_ms),
                cooldown_until_ms: Some(observed_at_ms.saturating_add(FAILURE_COOLDOWN_MS)),
                diagnostic_code: Some(io_error_code(err)),
            }),
        };

        if let Err(err) = self
            .append_source_records_retry(&source_ref, vec![record])
            .await
        {
            warn!(error = %io_error_code(&err), source_id, "failed to persist rpc.control observation");
        }
    }

    fn needs_probe(
        &self,
        source: &RpcControlBootstrapSource,
        state: Option<&RpcSourceState>,
        now_ms: u64,
    ) -> bool {
        let Some(state) = state else {
            return true;
        };

        let Some(last_recorded_at_ms) = state.last_recorded_at_ms else {
            return true;
        };
        if now_ms.saturating_sub(last_recorded_at_ms) > PROBE_REFRESH_INTERVAL_MS {
            return true;
        }
        if source.require_get_proof_probe && state.supports_get_proof != Some(true) {
            return true;
        }
        false
    }

    async fn probe_source(
        &mut self,
        control_scope: &str,
        network_scope: &str,
        source: &RpcControlBootstrapSource,
    ) -> Result<RpcSourceState, IoError> {
        let source_ref = RpcSourceRef::new(control_scope, network_scope, source.id.clone())
            .map_err(|err| {
                io_transport(
                    "rpc_control_source_invalid",
                    ErrorCategory::ParsingInput,
                    false,
                    err.to_string(),
                )
            })?;

        let probed_at_ms = now_ms()?;
        let mut records = Vec::new();

        let start = Instant::now();
        let basic_probe = self
            .execute_inner(&source.id, "eth_blockNumber", serde_json::json!([]))
            .await;
        let latency_ms = start.elapsed().as_millis() as u64;

        match basic_probe {
            Ok(response) => {
                records.push(RpcSourceRecord::Observed(RpcSourceObservedRecord {
                    observed_at_ms: probed_at_ms,
                    outcome: RpcSourceOutcome::Success,
                    head_block_number: parse_u64_hex_value(&response).ok(),
                    latency_ms: Some(latency_ms),
                    cooldown_until_ms: None,
                    diagnostic_code: None,
                }));
                records.push(RpcSourceRecord::Probed(RpcSourceProbedRecord {
                    probed_at_ms,
                    probe_kind: RpcSourceProbeKind::Basic,
                    outcome: RpcSourceOutcome::Success,
                    latency_ms: Some(latency_ms),
                    supports_get_proof: None,
                    cooldown_until_ms: None,
                    diagnostic_code: None,
                }));
            }
            Err(err) => {
                let code = io_error_code(&err);
                let cooldown_until_ms = Some(probed_at_ms.saturating_add(FAILURE_COOLDOWN_MS));
                records.push(RpcSourceRecord::Observed(RpcSourceObservedRecord {
                    observed_at_ms: probed_at_ms,
                    outcome: RpcSourceOutcome::Failure,
                    head_block_number: None,
                    latency_ms: Some(latency_ms),
                    cooldown_until_ms,
                    diagnostic_code: Some(code.clone()),
                }));
                records.push(RpcSourceRecord::Probed(RpcSourceProbedRecord {
                    probed_at_ms,
                    probe_kind: RpcSourceProbeKind::Basic,
                    outcome: RpcSourceOutcome::Failure,
                    latency_ms: Some(latency_ms),
                    supports_get_proof: None,
                    cooldown_until_ms,
                    diagnostic_code: Some(code),
                }));
                return self.append_source_records_retry(&source_ref, records).await;
            }
        }

        if source.require_get_proof_probe {
            let start = Instant::now();
            let get_proof = self
                .execute_inner(
                    &source.id,
                    "eth_getProof",
                    serde_json::json!(["0x0000000000000000000000000000000000000000", [], "latest"]),
                )
                .await;
            let latency_ms = start.elapsed().as_millis() as u64;
            match get_proof {
                Ok(_) => records.push(RpcSourceRecord::Probed(RpcSourceProbedRecord {
                    probed_at_ms,
                    probe_kind: RpcSourceProbeKind::GetProof,
                    outcome: RpcSourceOutcome::Success,
                    latency_ms: Some(latency_ms),
                    supports_get_proof: Some(true),
                    cooldown_until_ms: None,
                    diagnostic_code: None,
                })),
                Err(err) => records.push(RpcSourceRecord::Probed(RpcSourceProbedRecord {
                    probed_at_ms,
                    probe_kind: RpcSourceProbeKind::GetProof,
                    outcome: RpcSourceOutcome::Failure,
                    latency_ms: Some(latency_ms),
                    supports_get_proof: Some(false),
                    cooldown_until_ms: Some(probed_at_ms.saturating_add(FAILURE_COOLDOWN_MS)),
                    diagnostic_code: Some(io_error_code(&err)),
                })),
            }
        }

        self.append_source_records_retry(&source_ref, records).await
    }

    fn source_healthy(
        &self,
        source: &RpcControlBootstrapSource,
        state: Option<&RpcSourceState>,
        now_ms: u64,
    ) -> bool {
        let Some(state) = state else {
            return false;
        };
        let cooldown_active = state
            .cooldown_until_ms
            .map(|deadline| deadline > now_ms)
            .unwrap_or(false);
        let capability_ok =
            !source.require_get_proof_probe || state.supports_get_proof == Some(true);
        capability_ok && !cooldown_active && state.consecutive_failures == 0
    }

    fn rank_sources(
        &self,
        candidates: Vec<RpcControlBootstrapSource>,
        states: &HashMap<String, RpcSourceState>,
        now_ms: u64,
    ) -> Vec<RankedSource> {
        let preferred_index = self
            .catalog
            .preferred_order
            .iter()
            .enumerate()
            .map(|(index, source_id)| (source_id.as_str(), index))
            .collect::<HashMap<_, _>>();

        let mut ranked = candidates
            .into_iter()
            .map(|source| {
                let state = states.get(source.id.as_str()).cloned();
                let healthy = self.source_healthy(&source, state.as_ref(), now_ms);
                RankedSource {
                    source,
                    state,
                    healthy,
                }
            })
            .collect::<Vec<_>>();

        ranked.sort_by(|left, right| {
            let left_state = left.state.as_ref();
            let right_state = right.state.as_ref();

            let left_cooldown = left_state
                .and_then(|state| state.cooldown_until_ms)
                .map(|deadline| deadline > now_ms)
                .unwrap_or(true);
            let right_cooldown = right_state
                .and_then(|state| state.cooldown_until_ms)
                .map(|deadline| deadline > now_ms)
                .unwrap_or(true);
            let left_capability = !left.source.require_get_proof_probe
                || left_state.and_then(|state| state.supports_get_proof) == Some(true);
            let right_capability = !right.source.require_get_proof_probe
                || right_state.and_then(|state| state.supports_get_proof) == Some(true);
            let left_failures = left_state
                .map(|state| state.consecutive_failures)
                .unwrap_or(u64::MAX);
            let right_failures = right_state
                .map(|state| state.consecutive_failures)
                .unwrap_or(u64::MAX);
            let left_latency = left_state
                .and_then(|state| state.last_probe_latency_ms.or(state.last_latency_ms))
                .unwrap_or(u64::MAX);
            let right_latency = right_state
                .and_then(|state| state.last_probe_latency_ms.or(state.last_latency_ms))
                .unwrap_or(u64::MAX);
            let left_pref = preferred_index
                .get(left.source.id.as_str())
                .copied()
                .unwrap_or(usize::MAX);
            let right_pref = preferred_index
                .get(right.source.id.as_str())
                .copied()
                .unwrap_or(usize::MAX);

            (!left.healthy)
                .cmp(&!right.healthy)
                .then_with(|| (!left_capability).cmp(&!right_capability))
                .then_with(|| left_cooldown.cmp(&right_cooldown))
                .then_with(|| left_failures.cmp(&right_failures))
                .then_with(|| left_latency.cmp(&right_latency))
                .then_with(|| kind_rank(left.source.kind).cmp(&kind_rank(right.source.kind)))
                .then_with(|| left_pref.cmp(&right_pref))
                .then_with(|| left.source.id.cmp(&right.source.id))
        });

        ranked
    }

    async fn prepare_sources_impl(
        &mut self,
        control_scope: &str,
        network_scope: &str,
    ) -> Result<PrepareSourcesResponse, IoError> {
        let candidates = self.ordered_candidate_sources(network_scope)?;
        let available_source_ids = candidates
            .iter()
            .map(|source| source.id.clone())
            .collect::<Vec<_>>();
        let pool_ref = SourcePoolRef::new(control_scope, network_scope, DEFAULT_POOL_KIND)
            .map_err(|err| {
                io_transport(
                    "rpc_control_pool_invalid",
                    ErrorCategory::ParsingInput,
                    false,
                    err.to_string(),
                )
            })?;

        let catalog_snapshot = self.catalog_snapshot_for_pool(
            control_scope,
            network_scope,
            DEFAULT_POOL_KIND,
            &candidates,
        )?;
        let current_pool = Some(
            self.ensure_declared_catalog(&pool_ref, &catalog_snapshot)
                .await?,
        );
        if current_pool
            .as_ref()
            .map(|state| state.member_source_ids.as_slice())
            != Some(available_source_ids.as_slice())
        {
            self.append_source_pool_records_retry(
                &pool_ref,
                vec![SourcePoolRecord::MembershipDeclared(
                    SourcePoolMembershipDeclaredRecord {
                        declared_at_ms: now_ms()?,
                        member_source_ids: available_source_ids.clone(),
                    },
                )],
            )
            .await?;
        }

        let mut states = HashMap::new();
        let current_ms = now_ms()?;
        for source in &candidates {
            let source_ref = RpcSourceRef::new(control_scope, network_scope, source.id.clone())
                .map_err(|err| {
                    io_transport(
                        "rpc_control_source_invalid",
                        ErrorCategory::ParsingInput,
                        false,
                        err.to_string(),
                    )
                })?;
            let existing = self
                .control_plane_store
                .rpc_source_state(&source_ref)
                .await?;
            let state = if self.needs_probe(source, existing.as_ref(), current_ms) {
                self.probe_source(control_scope, network_scope, source)
                    .await?
            } else {
                existing.expect("existing state checked above")
            };
            states.insert(source.id.clone(), state);
        }

        let ranked = self.rank_sources(candidates.clone(), &states, current_ms);
        let ranked_source_ids = ranked
            .iter()
            .map(|entry| entry.source.id.clone())
            .collect::<Vec<_>>();
        let current_pool = self
            .control_plane_store
            .source_pool_state(&pool_ref)
            .await?;
        if current_pool
            .as_ref()
            .map(|state| state.ranked_source_ids.as_slice())
            != Some(ranked_source_ids.as_slice())
        {
            self.append_source_pool_records_retry(
                &pool_ref,
                vec![SourcePoolRecord::Ranked(SourcePoolRankedRecord {
                    ranked_at_ms: now_ms()?,
                    ranked_source_ids: ranked_source_ids.clone(),
                })],
            )
            .await?;
        }

        let summaries = ranked
            .iter()
            .map(|entry| PreparedSourceSummary {
                source_id: entry.source.id.clone(),
                healthy: entry.healthy,
                supports_get_proof: entry
                    .state
                    .as_ref()
                    .and_then(|state| state.supports_get_proof),
                cooldown_until_ms: entry
                    .state
                    .as_ref()
                    .and_then(|state| state.cooldown_until_ms),
                last_error_code: entry
                    .state
                    .as_ref()
                    .and_then(|state| state.last_error_code.clone()),
            })
            .collect::<Vec<_>>();

        Ok(PrepareSourcesResponse {
            control_scope: control_scope.to_string(),
            network_id: network_scope.to_string(),
            pool_kind: DEFAULT_POOL_KIND.to_string(),
            available_source_ids,
            ranked_source_ids,
            sources: summaries,
        })
    }

    async fn handle_prepare_sources(
        &mut self,
        control_scope: &str,
        network_id: &str,
    ) -> Result<serde_json::Value, IoError> {
        let response = self.prepare_sources_impl(control_scope, network_id).await?;
        serde_json::to_value(response).map_err(|_| {
            io_transport(
                "rpc_control_response_encode_failed",
                ErrorCategory::ParsingInput,
                false,
                "failed to encode rpc.control prepare_sources response",
            )
        })
    }

    async fn select_managed_source(
        &mut self,
        control_scope: &str,
        network_scope: &str,
    ) -> Result<String, IoError> {
        let prepared = self
            .prepare_sources_impl(control_scope, network_scope)
            .await?;
        prepared
            .sources
            .iter()
            .find(|source| source.healthy)
            .map(|source| source.source_id.clone())
            .or_else(|| prepared.ranked_source_ids.first().cloned())
            .ok_or_else(|| {
                io_transport(
                    "rpc_control_no_sources",
                    ErrorCategory::Unknown,
                    false,
                    format!("no managed rpc sources available for network `{network_scope}`"),
                )
            })
    }

    async fn handle_evm_call(
        &mut self,
        managed_call: mfm_collectors_rpc_control::JsonRpcCall,
    ) -> Result<serde_json::Value, IoError> {
        if matches!(
            managed_call.method.as_str(),
            "eth_sendTransaction" | "eth_sendRawTransaction"
        ) {
            return Err(io_transport(
                "evm_tx_intent_required",
                ErrorCategory::Rpc,
                false,
                "EVM write methods require a typed signed transaction intent broadcast",
            ));
        }
        let requested_network_id = managed_call.network_id.as_deref();
        let network_scope = self.resolve_network_scope(requested_network_id)?;
        let source_id = self
            .select_managed_source(&managed_call.control_scope, &network_scope)
            .await?;

        let start = Instant::now();
        let result = self
            .execute_inner(
                &source_id,
                &managed_call.method,
                managed_call.params.clone(),
            )
            .await;
        let latency_ms = start.elapsed().as_millis() as u64;

        match result {
            Ok(response) => {
                self.append_runtime_observation(
                    &managed_call.control_scope,
                    &network_scope,
                    &source_id,
                    &managed_call.method,
                    Ok((&response, latency_ms)),
                )
                .await;
                Ok(response)
            }
            Err(err) => {
                self.append_runtime_observation(
                    &managed_call.control_scope,
                    &network_scope,
                    &source_id,
                    &managed_call.method,
                    Err((&err, latency_ms)),
                )
                .await;
                Err(err)
            }
        }
    }

    async fn execute_inner_observed(
        &mut self,
        control_scope: &str,
        network_scope: &str,
        source_id: &str,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, IoError> {
        let start = Instant::now();
        let result = self.execute_inner(source_id, method, params).await;
        let latency_ms = start.elapsed().as_millis() as u64;
        match &result {
            Ok(response) => {
                self.append_runtime_observation(
                    control_scope,
                    network_scope,
                    source_id,
                    method,
                    Ok((response, latency_ms)),
                )
                .await;
            }
            Err(err) => {
                self.append_runtime_observation(
                    control_scope,
                    network_scope,
                    source_id,
                    method,
                    Err((err, latency_ms)),
                )
                .await;
            }
        }
        result
    }

    async fn expected_tx_has_receipt(
        &mut self,
        control_scope: &str,
        network_scope: &str,
        source_id: &str,
        expected_tx_hash: &str,
    ) -> Result<bool, IoError> {
        let receipt = self
            .execute_inner_observed(
                control_scope,
                network_scope,
                source_id,
                "eth_getTransactionReceipt",
                serde_json::json!([expected_tx_hash]),
            )
            .await?;
        Ok(!receipt.is_null())
    }

    async fn expected_tx_is_pending_or_mined(
        &mut self,
        control_scope: &str,
        network_scope: &str,
        source_id: &str,
        expected_tx_hash: &str,
    ) -> Result<bool, IoError> {
        if self
            .expected_tx_has_receipt(control_scope, network_scope, source_id, expected_tx_hash)
            .await?
        {
            return Ok(true);
        }

        let tx = self
            .execute_inner_observed(
                control_scope,
                network_scope,
                source_id,
                "eth_getTransactionByHash",
                serde_json::json!([expected_tx_hash]),
            )
            .await?;
        Ok(!tx.is_null())
    }

    async fn handle_evm_broadcast_raw_transaction(
        &mut self,
        control_scope: &str,
        network_id: &str,
        raw_tx_hex: &str,
        expected_tx_hash: &str,
    ) -> Result<serde_json::Value, IoError> {
        let network_scope = self.resolve_network_scope(Some(network_id))?;
        let expected_tx_hash = normalize_tx_hash(expected_tx_hash)?;
        let source_id = self
            .select_managed_source(control_scope, &network_scope)
            .await?;

        match self
            .execute_inner_observed(
                control_scope,
                &network_scope,
                &source_id,
                "eth_sendRawTransaction",
                serde_json::json!([raw_tx_hex]),
            )
            .await
        {
            Ok(response) => {
                let observed = response.as_str().ok_or_else(|| {
                    io_transport(
                        "evm_broadcast_response_invalid",
                        ErrorCategory::ParsingInput,
                        false,
                        "eth_sendRawTransaction returned non-string tx hash",
                    )
                })?;
                let observed = normalize_tx_hash(observed)?;
                if observed != expected_tx_hash {
                    return Err(io_transport(
                        "evm_broadcast_hash_mismatch",
                        ErrorCategory::Rpc,
                        false,
                        "eth_sendRawTransaction returned a different transaction hash",
                    ));
                }
            }
            Err(err) if is_already_known_error(&err) => {
                if !self
                    .expected_tx_is_pending_or_mined(
                        control_scope,
                        &network_scope,
                        &source_id,
                        &expected_tx_hash,
                    )
                    .await?
                {
                    return Err(io_transport(
                        "evm_broadcast_duplicate_unverified",
                        ErrorCategory::Rpc,
                        true,
                        "duplicate transaction response could not be verified against expected hash",
                    ));
                }
            }
            Err(err) if is_nonce_too_low_error(&err) => {
                if !self
                    .expected_tx_has_receipt(
                        control_scope,
                        &network_scope,
                        &source_id,
                        &expected_tx_hash,
                    )
                    .await?
                {
                    return Err(io_transport(
                        "evm_broadcast_nonce_too_low_unverified",
                        ErrorCategory::Rpc,
                        false,
                        "nonce-too-low response did not have a receipt for the expected hash",
                    ));
                }
            }
            Err(err) => return Err(err),
        }

        serde_json::to_value(EvmBroadcastRawTransactionResponse {
            tx_hash: expected_tx_hash,
        })
        .map_err(|_| {
            io_transport(
                "rpc_control_response_invalid",
                ErrorCategory::ParsingInput,
                false,
                "failed to encode raw transaction broadcast response",
            )
        })
    }

    fn ensure_btc_client(&mut self) -> Result<&mut BtcJsonRpcClient, IoError> {
        match self.btc_client.as_mut() {
            None => Err(io_transport(
                "btc_rpc_not_configured",
                ErrorCategory::Unknown,
                false,
                format!(
                    "Bitcoin RPC not configured: set {} to enable Bitcoin IO",
                    ENV_BTC_RPC_URL
                ),
            )),
            Some(Ok(client)) => Ok(client),
            Some(Err(err)) => Err(io_transport(
                "btc_rpc_client_init_failed",
                ErrorCategory::Unknown,
                false,
                format!("Bitcoin RPC client initialization failed: {err}"),
            )),
        }
    }

    async fn handle_bitcoin_anchor(
        &mut self,
        network_id: &str,
    ) -> Result<serde_json::Value, IoError> {
        let info = self.ensure_matching_btc_network(network_id).await?;
        Ok(serde_json::json!({
            "height": info.blocks,
            "block_hash": info.bestblockhash,
        }))
    }

    async fn ensure_matching_btc_network(
        &mut self,
        network_id: &str,
    ) -> Result<BlockchainInfo, IoError> {
        let network_id = network_id.trim();
        if network_id.is_empty() {
            return Err(io_transport(
                "btc_rpc_network_invalid",
                ErrorCategory::ParsingInput,
                false,
                "bitcoin network_id must not be empty",
            ));
        }
        let expected_chain = bitcoin_chain_for_network_id(network_id).ok_or_else(|| {
            io_transport(
                "btc_rpc_network_unsupported",
                ErrorCategory::ParsingInput,
                false,
                format!("unsupported bitcoin network id `{network_id}`"),
            )
        })?;
        let client = self.ensure_btc_client()?;
        let info = client.get_blockchain_info().await.map_err(|err| {
            io_transport(
                "btc_rpc_network_query_failed",
                ErrorCategory::Unknown,
                true,
                format!("bitcoin network query failed: {err}"),
            )
        })?;
        if info.chain != expected_chain {
            return Err(io_transport(
                "btc_rpc_network_mismatch",
                ErrorCategory::ParsingInput,
                false,
                format!(
                    "bitcoin request for `{network_id}` expects chain `{expected_chain}`, but rpc reports `{}`",
                    info.chain,
                ),
            ));
        }
        Ok(info)
    }

    async fn handle_bitcoin_scan_utxos(
        &mut self,
        network_id: &str,
        address: &str,
        expected_height: u64,
        expected_block_hash: &str,
    ) -> Result<serde_json::Value, IoError> {
        let info = self.ensure_matching_btc_network(network_id).await?;
        if info.blocks != expected_height || info.bestblockhash != expected_block_hash {
            return Err(io_transport(
                "btc_rpc_scan_utxos_anchor_mismatch",
                ErrorCategory::ParsingInput,
                false,
                format!(
                    "bitcoin scan utxos request anchored at {expected_height}@{expected_block_hash}, \
                    but node reports {actual_height}@{actual_hash}",
                    actual_height = info.blocks,
                    actual_hash = info.bestblockhash
                ),
            ));
        }
        let client = self.ensure_btc_client()?;
        let result = client.scan_tx_out_set(address).await.map_err(|err| {
            io_transport(
                "btc_rpc_scan_utxos_failed",
                ErrorCategory::Unknown,
                true,
                format!("bitcoin scan utxos failed: {err}"),
            )
        })?;

        if result.height != expected_height || result.bestblock != expected_block_hash {
            return Err(io_transport(
                "btc_rpc_scan_utxos_anchor_mismatch",
                ErrorCategory::ParsingInput,
                false,
                format!(
                    "bitcoin scan utxos request anchored at {expected_height}@{expected_block_hash}, \
                    but actual scan executed at {actual_height}@{actual_hash}",
                    actual_height = result.height,
                    actual_hash = result.bestblock
                ),
            ));
        }

        Ok(serde_json::json!({
            "amount_sats": result.total_amount_sats.to_string(),
        }))
    }
}

#[async_trait]
impl LiveIoTransport for RpcControlTransport {
    async fn call(&mut self, call: IoCall) -> Result<serde_json::Value, IoError> {
        if call.namespace != NAMESPACE_RPC_CONTROL {
            return Err(io_transport(
                "unknown_namespace",
                ErrorCategory::Unknown,
                false,
                format!("unknown rpc.control namespace `{}`", call.namespace),
            ));
        }

        let request: RpcControlRequest = serde_json::from_value(call.request).map_err(|_| {
            io_transport(
                "rpc_control_request_invalid",
                ErrorCategory::ParsingInput,
                false,
                "rpc.control request payload was invalid",
            )
        })?;

        match request {
            RpcControlRequest::EvmCall { call } => self.handle_evm_call(call).await,
            RpcControlRequest::EvmBroadcastRawTransaction {
                control_scope,
                network_id,
                raw_tx_hex,
                expected_tx_hash,
            } => {
                self.handle_evm_broadcast_raw_transaction(
                    &control_scope,
                    &network_id,
                    &raw_tx_hex,
                    &expected_tx_hash,
                )
                .await
            }
            RpcControlRequest::PrepareSources {
                control_scope,
                network_id,
            } => {
                self.handle_prepare_sources(&control_scope, &network_id)
                    .await
            }
            RpcControlRequest::BitcoinAnchor { network_id, .. } => {
                self.handle_bitcoin_anchor(&network_id).await
            }
            RpcControlRequest::BitcoinScanUtxos {
                address,
                network_id,
                height,
                block_hash,
                ..
            } => {
                self.handle_bitcoin_scan_utxos(&network_id, &address, height, &block_hash)
                    .await
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::sync::Mutex as StdMutex;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};
    use tokio::sync::oneshot;

    use mfm_stream_store_mem::MemStreamStore;

    struct NoopArtifactStore;

    #[async_trait]
    impl mfm_machine::stores::ArtifactStore for NoopArtifactStore {
        async fn put(
            &self,
            _kind: mfm_machine::stores::ArtifactKind,
            _bytes: Vec<u8>,
        ) -> Result<mfm_machine::ids::ArtifactId, StorageError> {
            Ok(mfm_machine::ids::ArtifactId::must_new("0".repeat(64)))
        }

        async fn get(&self, _id: &mfm_machine::ids::ArtifactId) -> Result<Vec<u8>, StorageError> {
            Ok(Vec::new())
        }

        async fn exists(&self, _id: &mfm_machine::ids::ArtifactId) -> Result<bool, StorageError> {
            Ok(false)
        }
    }

    fn live_env_for_tests() -> LiveIoEnv {
        LiveIoEnv {
            stores: mfm_machine::engine::Stores {
                streams: Arc::new(MemStreamStore::new()),
                artifacts: Arc::new(NoopArtifactStore),
            },
            run_id: serde_json::from_str::<mfm_machine::ids::RunId>(
                "\"00000000-0000-0000-0000-000000000000\"",
            )
            .expect("valid RunId"),
            state_id: mfm_machine::ids::StateId::must_new("rpc_control.test.s1".to_string()),
            attempt: 0,
        }
    }

    struct StubExecutor;

    #[async_trait]
    impl LiveIoTransport for StubExecutor {
        async fn call(&mut self, call: IoCall) -> Result<serde_json::Value, IoError> {
            let method = call
                .request
                .get("method")
                .and_then(|value| value.as_str())
                .unwrap_or_default();
            match method {
                "eth_blockNumber" => Ok(serde_json::json!("0x1")),
                "eth_getProof" => Ok(serde_json::json!({"accountProof": []})),
                other => Err(io_transport(
                    "stub_executor_unsupported",
                    ErrorCategory::Unknown,
                    false,
                    format!("stub executor does not support `{other}`"),
                )),
            }
        }
    }

    enum BroadcastMode {
        AlreadyKnownPending,
        NonceTooLowWithReceipt,
        NonceTooLowWithoutReceipt,
    }

    struct BroadcastExecutor {
        mode: BroadcastMode,
        expected_hash: String,
        calls: Arc<StdMutex<Vec<String>>>,
    }

    fn jsonrpc_transport_error(message: &'static str) -> IoError {
        IoError::Transport(ErrorInfo {
            code: ErrorCode::must_new("evm_jsonrpc_error"),
            category: ErrorCategory::Rpc,
            retryable: true,
            message: "evm jsonrpc returned an error".to_string(),
            details: Some(serde_json::json!({
                "source_id": "source-1",
                "jsonrpc_error_code": -32000,
                "jsonrpc_error_message": message,
            })),
        })
    }

    #[async_trait]
    impl LiveIoTransport for BroadcastExecutor {
        async fn call(&mut self, call: IoCall) -> Result<serde_json::Value, IoError> {
            let method = call
                .request
                .get("method")
                .and_then(|value| value.as_str())
                .unwrap_or_default()
                .to_string();
            self.calls.lock().expect("calls lock").push(method.clone());

            match method.as_str() {
                "eth_blockNumber" => Ok(serde_json::json!("0x1")),
                "eth_getProof" => Ok(serde_json::json!({"accountProof": []})),
                "eth_sendRawTransaction" => match self.mode {
                    BroadcastMode::AlreadyKnownPending => {
                        Err(jsonrpc_transport_error("already known"))
                    }
                    BroadcastMode::NonceTooLowWithReceipt
                    | BroadcastMode::NonceTooLowWithoutReceipt => {
                        Err(jsonrpc_transport_error("nonce too low"))
                    }
                },
                "eth_getTransactionReceipt" => match self.mode {
                    BroadcastMode::NonceTooLowWithReceipt => {
                        Ok(serde_json::json!({"status": "0x1"}))
                    }
                    _ => Ok(serde_json::Value::Null),
                },
                "eth_getTransactionByHash" => match self.mode {
                    BroadcastMode::AlreadyKnownPending => {
                        Ok(serde_json::json!({"hash": self.expected_hash}))
                    }
                    _ => Ok(serde_json::Value::Null),
                },
                other => Err(io_transport(
                    "stub_executor_unsupported",
                    ErrorCategory::Unknown,
                    false,
                    format!("stub executor does not support `{other}`"),
                )),
            }
        }
    }

    fn source(
        id: &str,
        network_id: Option<&str>,
        kind: EvmSourceKind,
        require_get_proof_probe: bool,
    ) -> RpcControlBootstrapSource {
        RpcControlBootstrapSource {
            id: id.to_string(),
            network_id: network_id.map(str::to_string),
            rpc_url: format!("http://127.0.0.1/{}", id),
            authorization: None,
            kind,
            require_get_proof_probe,
        }
    }

    #[test]
    fn bootstrap_source_debug_redacts_rpc_url_and_authorization() {
        let source = RpcControlBootstrapSource {
            id: "primary".to_string(),
            network_id: Some("ethereum-mainnet".to_string()),
            rpc_url:
                "https://url_user:url_password@example.com:8545/rpc?api_key=query_secret&token=query_token#frag"
                    .to_string(),
            authorization: Some("Bearer authorization_secret".to_string()),
            kind: EvmSourceKind::RemoteUser,
            require_get_proof_probe: true,
        };

        let rendered = format!("{source:?}");

        assert!(rendered.contains("RpcControlBootstrapSource"));
        assert!(rendered.contains("https://example.com:8545"));
        assert!(!rendered.contains("url_user"));
        assert!(!rendered.contains("url_password"));
        assert!(!rendered.contains("api_key"));
        assert!(!rendered.contains("query_secret"));
        assert!(!rendered.contains("query_token"));
        assert!(!rendered.contains("authorization_secret"));
    }

    #[test]
    fn btc_error_surface_through_transport_omits_http_body_credentials() {
        let omitted_body =
            "Authorization: Bearer body_token password=body_password token=body_secret";
        let mut transport = RpcControlTransport {
            executor: None,
            executor_error: None,
            btc_client: Some(Err(BtcRpcError::HttpStatus {
                status: 500,
                body_len: Some(omitted_body.len()),
                content_type: Some("text/plain".to_string()),
            })),
            control_plane_store: ControlPlaneStore::PostgresEnv { store: None },
            catalog: BootstrapCatalog {
                sources: Vec::new(),
                preferred_order: Vec::new(),
            },
            config_error: None,
        };

        let err = match transport.ensure_btc_client() {
            Ok(_) => panic!("client init failure should surface through transport"),
            Err(err) => err,
        };
        let message = match err {
            IoError::Transport(info) => info.message,
            other => panic!("expected transport error, got {other:?}"),
        };

        assert!(message.contains("btc rpc http status 500"));
        assert!(message.contains("body_len="));
        assert!(!message.contains("body_token"));
        assert!(!message.contains("body_password"));
        assert!(!message.contains("body_secret"));
    }

    #[tokio::test]
    async fn inner_executor_construction_error_surfaces_without_rpc_credentials() {
        let factory = RpcControlTransportFactory::new(vec![RpcControlBootstrapSource {
            id: "primary".to_string(),
            network_id: Some("ethereum-mainnet".to_string()),
            rpc_url:
                "https://url_user:url_password@example.com:8545/rpc?api_key=query_secret&token=query_token#frag"
                    .to_string(),
            authorization: Some("Bearer authorization_secret".to_string()),
            kind: EvmSourceKind::RemoteUser,
            require_get_proof_probe: false,
        }])
        .with_control_plane_storage_mode(RpcControlPlaneStorageMode::StreamStore)
        .with_executor_tuning(RpcControlExecutorTuning::new(32, 64, 1));
        let mut transport = factory.make(live_env_for_tests());
        let request = serde_json::to_value(RpcControlRequest::EvmCall {
            call: mfm_collectors_rpc_control::JsonRpcCall::for_network(
                "ethereum-mainnet",
                "eth_blockNumber",
                serde_json::json!([]),
            ),
        })
        .expect("request should serialize");

        let err = transport
            .call(IoCall {
                namespace: NAMESPACE_RPC_CONTROL.to_string(),
                request,
                fact_key: None,
            })
            .await
            .expect_err("invalid inner executor config should fail before dispatch");
        let message = match err {
            IoError::Transport(info) => {
                assert_eq!(info.code.as_str(), "rpc_control_executor_config_invalid");
                info.message
            }
            other => panic!("expected transport error, got {other:?}"),
        };

        assert!(message.contains("logs chunking config is invalid"));
        assert!(!message.contains("url_user"));
        assert!(!message.contains("url_password"));
        assert!(!message.contains("api_key"));
        assert!(!message.contains("query_secret"));
        assert!(!message.contains("query_token"));
        assert!(!message.contains("authorization_secret"));
    }

    fn transport_for_tests(sources: Vec<RpcControlBootstrapSource>) -> RpcControlTransport {
        let preferred_order = sources.iter().map(|source| source.id.clone()).collect();
        RpcControlTransport {
            executor: None,
            executor_error: None,
            btc_client: None,
            control_plane_store: ControlPlaneStore::PostgresEnv { store: None },
            catalog: BootstrapCatalog {
                sources,
                preferred_order,
            },
            config_error: None,
        }
    }

    fn stream_backed_transport_for_tests(
        streams: Arc<MemStreamStore>,
        sources: Vec<RpcControlBootstrapSource>,
    ) -> RpcControlTransport {
        let preferred_order = sources.iter().map(|source| source.id.clone()).collect();
        RpcControlTransport {
            executor: Some(Box::new(StubExecutor)),
            executor_error: None,
            btc_client: None,
            control_plane_store: ControlPlaneStore::StreamBacked(
                StreamBackedControlPlaneStore::new(streams),
            ),
            catalog: BootstrapCatalog {
                sources,
                preferred_order,
            },
            config_error: None,
        }
    }

    fn stream_backed_transport_with_executor(
        streams: Arc<MemStreamStore>,
        sources: Vec<RpcControlBootstrapSource>,
        executor: Box<dyn LiveIoTransport>,
    ) -> RpcControlTransport {
        let preferred_order = sources.iter().map(|source| source.id.clone()).collect();
        RpcControlTransport {
            executor: Some(executor),
            executor_error: None,
            btc_client: None,
            control_plane_store: ControlPlaneStore::StreamBacked(
                StreamBackedControlPlaneStore::new(streams),
            ),
            catalog: BootstrapCatalog {
                sources,
                preferred_order,
            },
            config_error: None,
        }
    }

    struct StubBitcoinServer {
        url: String,
        shutdown: Option<oneshot::Sender<()>>,
    }

    impl Drop for StubBitcoinServer {
        fn drop(&mut self) {
            if let Some(tx) = self.shutdown.take() {
                let _ = tx.send(());
            }
        }
    }

    fn header_end(buf: &[u8]) -> Option<usize> {
        buf.windows(4).position(|w| w == b"\r\n\r\n")
    }

    fn parse_content_length(header: &str) -> usize {
        for line in header.lines() {
            let lc = line.to_ascii_lowercase();
            if let Some(rest) = lc.strip_prefix("content-length:") {
                return rest.trim().parse::<usize>().unwrap_or(0);
            }
        }
        0
    }

    async fn handle_stub_connection(
        mut stream: TcpStream,
        responses: Arc<Vec<serde_json::Value>>,
        calls: Arc<AtomicUsize>,
    ) -> Result<(), ()> {
        let mut buf = Vec::new();
        let mut temp = [0u8; 1024];
        let mut total_needed: Option<usize> = None;

        loop {
            let read_n = stream.read(&mut temp).await.map_err(|_| ())?;
            if read_n == 0 {
                return Err(());
            }
            buf.extend_from_slice(&temp[..read_n]);

            if let Some(end) = header_end(&buf) {
                if total_needed.is_none() {
                    let header = std::str::from_utf8(&buf[..end]).map_err(|_| ())?;
                    let content_len = parse_content_length(header);
                    total_needed = Some(end + 4 + content_len);
                }
                if let Some(needed) = total_needed {
                    if buf.len() >= needed {
                        break;
                    }
                }
            }
        }

        let call_index = calls.fetch_add(1, Ordering::SeqCst);
        let response = responses
            .get(call_index)
            .cloned()
            .unwrap_or_else(|| serde_json::json!({ "jsonrpc": "2.0", "id": 1, "result": {} }));
        let body = response.to_string();
        let status_line = "HTTP/1.1 200 OK\r\n".to_string();
        let headers = format!(
            "content-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
            body.len()
        );
        stream
            .write_all(status_line.as_bytes())
            .await
            .map_err(|_| ())?;
        stream.write_all(headers.as_bytes()).await.map_err(|_| ())?;
        stream.write_all(body.as_bytes()).await.map_err(|_| ())?;
        Ok(())
    }

    async fn start_stub_btc_server(responses: Vec<serde_json::Value>) -> StubBitcoinServer {
        let responses = Arc::new(responses);
        let calls = Arc::new(AtomicUsize::new(0));
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test server");
        let addr = listener.local_addr().expect("local addr");

        let (shutdown_tx, mut shutdown_rx) = oneshot::channel::<()>();
        let responses_for_loop = Arc::clone(&responses);
        let calls_for_loop = Arc::clone(&calls);

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = &mut shutdown_rx => {
                        break;
                    }
                    accepted = listener.accept() => {
                        let Ok((stream, _)) = accepted else {
                            break;
                        };
                        let responses = Arc::clone(&responses_for_loop);
                        let calls = Arc::clone(&calls_for_loop);
                        tokio::spawn(async move {
                            let _ = handle_stub_connection(stream, responses, calls).await;
                        });
                    }
                }
            }
        });

        StubBitcoinServer {
            url: format!("http://127.0.0.1:{}/", addr.port()),
            shutdown: Some(shutdown_tx),
        }
    }

    #[test]
    fn bootstrap_catalog_rejects_missing_network_id() {
        std::env::remove_var(ENV_EVM_RPC_SOURCES_JSON);
        let catalog = BootstrapCatalog {
            sources: vec![source("primary", None, EvmSourceKind::RemoteUser, false)],
            preferred_order: vec!["primary".to_string()],
        };
        let err = validate_catalog(&catalog).expect_err("missing network_id must be rejected");
        assert_eq!(
            err,
            RpcControlConfigError::MissingNetworkId("primary".to_string())
        );
    }

    #[test]
    fn ranking_prefers_healthy_low_latency_sources() {
        let transport = transport_for_tests(vec![
            source(
                "local_fast",
                Some("ethereum-mainnet"),
                EvmSourceKind::Local,
                false,
            ),
            source(
                "remote_slow",
                Some("ethereum-mainnet"),
                EvmSourceKind::RemotePublic,
                false,
            ),
            source(
                "proof_missing",
                Some("ethereum-mainnet"),
                EvmSourceKind::Local,
                true,
            ),
        ]);
        let mut states = HashMap::new();
        states.insert(
            "local_fast".to_string(),
            RpcSourceState {
                source_ref: RpcSourceRef::new("shared", "ethereum-mainnet", "local_fast").unwrap(),
                stream_id: RpcSourceRef::new("shared", "ethereum-mainnet", "local_fast")
                    .unwrap()
                    .stream_id(),
                head_seq: 1,
                last_recorded_at_ms: Some(100),
                last_observed_at_ms: Some(100),
                last_probed_at_ms: Some(100),
                last_observed_head: Some(1),
                last_latency_ms: Some(10),
                last_probe_latency_ms: Some(10),
                supports_get_proof: Some(true),
                success_count: 1,
                failure_count: 0,
                consecutive_failures: 0,
                cooldown_until_ms: None,
                last_error_code: None,
            },
        );
        states.insert(
            "remote_slow".to_string(),
            RpcSourceState {
                source_ref: RpcSourceRef::new("shared", "ethereum-mainnet", "remote_slow").unwrap(),
                stream_id: RpcSourceRef::new("shared", "ethereum-mainnet", "remote_slow")
                    .unwrap()
                    .stream_id(),
                head_seq: 1,
                last_recorded_at_ms: Some(100),
                last_observed_at_ms: Some(100),
                last_probed_at_ms: Some(100),
                last_observed_head: Some(1),
                last_latency_ms: Some(80),
                last_probe_latency_ms: Some(80),
                supports_get_proof: Some(true),
                success_count: 1,
                failure_count: 0,
                consecutive_failures: 0,
                cooldown_until_ms: None,
                last_error_code: None,
            },
        );
        states.insert(
            "proof_missing".to_string(),
            RpcSourceState {
                source_ref: RpcSourceRef::new("shared", "ethereum-mainnet", "proof_missing")
                    .unwrap(),
                stream_id: RpcSourceRef::new("shared", "ethereum-mainnet", "proof_missing")
                    .unwrap()
                    .stream_id(),
                head_seq: 1,
                last_recorded_at_ms: Some(100),
                last_observed_at_ms: Some(100),
                last_probed_at_ms: Some(100),
                last_observed_head: Some(1),
                last_latency_ms: Some(5),
                last_probe_latency_ms: Some(5),
                supports_get_proof: Some(false),
                success_count: 1,
                failure_count: 0,
                consecutive_failures: 0,
                cooldown_until_ms: None,
                last_error_code: None,
            },
        );

        let ranked = transport.rank_sources(
            transport
                .ordered_candidate_sources("ethereum-mainnet")
                .expect("network sources should resolve"),
            &states,
            1_000,
        );
        assert_eq!(
            ranked
                .into_iter()
                .map(|entry| entry.source.id)
                .collect::<Vec<_>>(),
            vec![
                "local_fast".to_string(),
                "remote_slow".to_string(),
                "proof_missing".to_string()
            ]
        );
    }

    #[test]
    fn managed_scope_requires_network_when_catalog_is_multi_network_only() {
        let transport = transport_for_tests(vec![
            source(
                "mainnet",
                Some("ethereum-mainnet"),
                EvmSourceKind::Local,
                false,
            ),
            source("arb", Some("arbitrum-mainnet"), EvmSourceKind::Local, false),
        ]);
        let err = transport
            .resolve_network_scope(None)
            .expect_err("multi-network catalog should require network");
        assert_eq!(io_error_code(&err), "rpc_control_network_required");
    }

    #[test]
    fn bitcoin_network_chain_mapping_is_known() {
        assert_eq!(
            bitcoin_chain_for_network_id("bitcoin-mainnet"),
            Some("main")
        );
        assert_eq!(
            bitcoin_chain_for_network_id("bitcoin-testnet"),
            Some("test")
        );
        assert_eq!(
            bitcoin_chain_for_network_id("bitcoin-signet"),
            Some("signet")
        );
        assert_eq!(
            bitcoin_chain_for_network_id("bitcoin-regtest"),
            Some("regtest")
        );
        assert_eq!(bitcoin_chain_for_network_id("bitcoin-dev"), None);
    }

    #[tokio::test]
    async fn managed_call_without_network_id_is_rejected() {
        let mut transport = transport_for_tests(vec![source(
            "mainnet",
            Some("ethereum-mainnet"),
            EvmSourceKind::Local,
            false,
        )]);
        let err = transport
            .handle_evm_call(mfm_collectors_rpc_control::JsonRpcCall::new(
                "eth_chainId",
                serde_json::json!([]),
            ))
            .await
            .expect_err("networkless managed calls must fail");
        assert_eq!(io_error_code(&err), "rpc_control_network_required");
    }

    #[tokio::test]
    async fn same_scope_different_catalog_is_rejected() {
        let streams = Arc::new(MemStreamStore::new());
        let mut first = stream_backed_transport_for_tests(
            Arc::clone(&streams),
            vec![source(
                "primary",
                Some("ethereum-mainnet"),
                EvmSourceKind::Local,
                false,
            )],
        );
        first
            .prepare_sources_impl("shared", "ethereum-mainnet")
            .await
            .expect("first catalog declaration should succeed");

        let mut second = stream_backed_transport_for_tests(
            Arc::clone(&streams),
            vec![
                source(
                    "primary",
                    Some("ethereum-mainnet"),
                    EvmSourceKind::Local,
                    false,
                ),
                source(
                    "secondary",
                    Some("ethereum-mainnet"),
                    EvmSourceKind::RemotePublic,
                    false,
                ),
            ],
        );
        let err = second
            .prepare_sources_impl("shared", "ethereum-mainnet")
            .await
            .expect_err("mismatched catalog must fail");
        assert_eq!(io_error_code(&err), "rpc_control_catalog_mismatch");
    }

    #[tokio::test]
    async fn same_catalog_declaration_is_idempotent() {
        let streams = Arc::new(MemStreamStore::new());
        let sources = vec![source(
            "primary",
            Some("ethereum-mainnet"),
            EvmSourceKind::Local,
            false,
        )];
        let mut first = stream_backed_transport_for_tests(Arc::clone(&streams), sources.clone());
        let first_prepared = first
            .prepare_sources_impl("shared", "ethereum-mainnet")
            .await
            .expect("first declaration should succeed");

        let mut second = stream_backed_transport_for_tests(Arc::clone(&streams), sources);
        let second_prepared = second
            .prepare_sources_impl("shared", "ethereum-mainnet")
            .await
            .expect("matching declaration should succeed");

        assert_eq!(
            first_prepared.available_source_ids,
            second_prepared.available_source_ids
        );
        assert_eq!(
            first_prepared.ranked_source_ids,
            second_prepared.ranked_source_ids
        );
    }

    #[tokio::test]
    async fn same_scope_across_different_networks_does_not_collide() {
        let streams = Arc::new(MemStreamStore::new());
        let mut mainnet = stream_backed_transport_for_tests(
            Arc::clone(&streams),
            vec![source(
                "mainnet_primary",
                Some("ethereum-mainnet"),
                EvmSourceKind::Local,
                false,
            )],
        );
        mainnet
            .prepare_sources_impl("shared", "ethereum-mainnet")
            .await
            .expect("mainnet declaration should succeed");

        let mut arbitrum = stream_backed_transport_for_tests(
            Arc::clone(&streams),
            vec![source(
                "arb_primary",
                Some("arbitrum-mainnet"),
                EvmSourceKind::RemotePublic,
                false,
            )],
        );
        let prepared = arbitrum
            .prepare_sources_impl("shared", "arbitrum-mainnet")
            .await
            .expect("different network should not collide");
        assert_eq!(
            prepared.available_source_ids,
            vec!["arb_primary".to_string()]
        );
    }

    async fn broadcast_with_mode(
        mode: BroadcastMode,
    ) -> (Result<serde_json::Value, IoError>, Vec<String>) {
        let expected_hash = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let calls = Arc::new(StdMutex::new(Vec::new()));
        let executor = BroadcastExecutor {
            mode,
            expected_hash: expected_hash.to_string(),
            calls: Arc::clone(&calls),
        };
        let mut transport = stream_backed_transport_with_executor(
            Arc::new(MemStreamStore::new()),
            vec![source(
                "source-1",
                Some("ethereum-mainnet"),
                EvmSourceKind::Local,
                false,
            )],
            Box::new(executor),
        );

        let result = transport
            .handle_evm_broadcast_raw_transaction(
                "shared",
                "ethereum-mainnet",
                "0x01",
                expected_hash,
            )
            .await;
        let calls = calls.lock().expect("calls lock").clone();
        (result, calls)
    }

    #[tokio::test]
    async fn broadcast_treats_already_known_as_success_only_when_expected_hash_is_pending() {
        let (result, calls) = broadcast_with_mode(BroadcastMode::AlreadyKnownPending).await;
        let response = result.expect("already-known pending tx should be accepted");

        assert_eq!(
            response,
            serde_json::json!({
                "tx_hash": "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
            })
        );
        assert!(calls
            .iter()
            .any(|method| method == "eth_sendRawTransaction"));
        assert!(calls
            .iter()
            .any(|method| method == "eth_getTransactionByHash"));
    }

    #[tokio::test]
    async fn broadcast_treats_nonce_too_low_as_success_only_with_expected_receipt() {
        let (result, calls) = broadcast_with_mode(BroadcastMode::NonceTooLowWithReceipt).await;
        result.expect("nonce-too-low with expected receipt should be accepted");

        assert!(calls
            .iter()
            .any(|method| method == "eth_sendRawTransaction"));
        assert!(calls
            .iter()
            .any(|method| method == "eth_getTransactionReceipt"));
        assert!(!calls
            .iter()
            .any(|method| method == "eth_getTransactionByHash"));
    }

    #[tokio::test]
    async fn broadcast_rejects_nonce_too_low_without_expected_receipt() {
        let (result, _calls) = broadcast_with_mode(BroadcastMode::NonceTooLowWithoutReceipt).await;
        let err = result.expect_err("nonce-too-low without expected receipt is ambiguous");

        assert_eq!(
            io_error_code(&err),
            "evm_broadcast_nonce_too_low_unverified"
        );
    }

    #[tokio::test]
    async fn generic_evm_call_rejects_write_methods() {
        let mut transport = stream_backed_transport_for_tests(
            Arc::new(MemStreamStore::new()),
            vec![source(
                "source-1",
                Some("ethereum-mainnet"),
                EvmSourceKind::Local,
                false,
            )],
        );

        let err = transport
            .handle_evm_call(
                mfm_collectors_rpc_control::JsonRpcCall::for_scope_and_network(
                    "shared",
                    "ethereum-mainnet",
                    "eth_sendRawTransaction",
                    serde_json::json!(["0x01"]),
                ),
            )
            .await
            .expect_err("generic write method must be rejected");

        assert_eq!(io_error_code(&err), "evm_tx_intent_required");
    }

    #[tokio::test]
    async fn bitcoin_scan_utxos_returns_exact_sats_from_rpc_amount() {
        let expected_height = 840_000_u64;
        let expected_hash = "0000000000000000000320283a032748cef8227873ff4872689bf23f1cda83a5";
        let server = start_stub_btc_server(vec![
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": 1,
                "result": {
                    "chain": "main",
                    "blocks": expected_height,
                    "bestblockhash": expected_hash,
                }
            }),
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": 1,
                "result": {
                    "success": true,
                    "height": expected_height,
                    "bestblock": expected_hash,
                    "total_amount": "0.05000000",
                    "unspents": [],
                }
            }),
        ])
        .await;

        let client = BtcJsonRpcClient::new(BtcJsonRpcConfig {
            rpc_url: server.url.clone(),
            rpc_user: None,
            rpc_password: None,
        })
        .expect("test config should create client");

        let mut transport = RpcControlTransport {
            executor: None,
            executor_error: None,
            btc_client: Some(Ok(client)),
            control_plane_store: ControlPlaneStore::PostgresEnv { store: None },
            catalog: BootstrapCatalog {
                sources: Vec::new(),
                preferred_order: Vec::new(),
            },
            config_error: None,
        };

        let result = transport
            .handle_bitcoin_scan_utxos(
                "bitcoin-mainnet",
                "bc1qqqexample",
                expected_height,
                expected_hash,
            )
            .await
            .expect("scan should succeed");

        assert_eq!(result["amount_sats"], "5000000");
    }

    #[tokio::test]
    async fn bitcoin_scan_utxos_rejects_anchor_mismatch_after_scan() {
        let expected_height = 840_000_u64;
        let expected_hash = "0000000000000000000320283a032748cef8227873ff4872689bf23f1cda83a5";
        let server = start_stub_btc_server(vec![
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": 1,
                "result": {
                    "chain": "main",
                    "blocks": expected_height,
                    "bestblockhash": expected_hash,
                }
            }),
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": 1,
                "result": {
                    "success": true,
                    "height": expected_height + 1,
                    "bestblock": "000000000000000000000000000000000000000000000000000000000000000000",
                    "total_amount": "0",
                }
            }),
        ])
        .await;

        let client = BtcJsonRpcClient::new(BtcJsonRpcConfig {
            rpc_url: server.url.clone(),
            rpc_user: None,
            rpc_password: None,
        })
        .expect("test config should create client");

        let mut transport = RpcControlTransport {
            executor: None,
            executor_error: None,
            btc_client: Some(Ok(client)),
            control_plane_store: ControlPlaneStore::PostgresEnv { store: None },
            catalog: BootstrapCatalog {
                sources: Vec::new(),
                preferred_order: Vec::new(),
            },
            config_error: None,
        };

        let err = transport
            .handle_bitcoin_scan_utxos(
                "bitcoin-mainnet",
                "bc1qqqexample",
                expected_height,
                expected_hash,
            )
            .await
            .expect_err("scan with changed anchor should be rejected");

        assert_eq!(io_error_code(&err), "btc_rpc_scan_utxos_anchor_mismatch");
    }
}
