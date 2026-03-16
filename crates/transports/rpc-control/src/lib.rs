#![allow(clippy::disallowed_methods, clippy::disallowed_types)]
#![warn(missing_docs)]
//! Live `rpc.control` transport for managed EVM RPC routing.
//!
//! This transport keeps `rpc.control` as the canonical state-facing ingress while reusing the
//! existing HTTP JSON-RPC executor internally. It owns:
//! - bootstrap source catalog parsing from env
//! - durable `rpc_source:*` and `source_pool:*` updates
//! - source probing and ranking
//! - managed source selection for unpinned EVM calls
//!
//! The inner `evm` transport is treated as an executor only. Every managed call is pinned to one
//! concrete source before dispatch, so route choice and cooldown authority stay here.

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use serde::Deserialize;
use tracing::warn;

use mfm_collectors_evm_jsonrpc_http::{
    EvmJsonRpcHttpConfig, EvmJsonRpcHttpTransportFactory, EvmJsonRpcSource, EvmRoutingStrategy,
    EvmSourceKind,
};
use mfm_collectors_rpc_control::{
    parse_u64_hex_value, PrepareSourcesResponse, PreparedSourceSummary, RpcControlRequest,
    NAMESPACE_RPC_CONTROL,
};
use mfm_control_plane_postgres::{
    rebuild_rpc_source_state, rebuild_source_pool_state, ControlPlanePostgresStore,
    SourcePoolCatalogDeclaredRecord, SourcePoolCatalogSnapshot, SourcePoolCatalogSource,
    RpcSourceObservedRecord, RpcSourceOutcome, RpcSourceProbeKind, RpcSourceProbedRecord,
    RpcSourceRecord, RpcSourceRef, RpcSourceState, SourcePoolMembershipDeclaredRecord,
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

const DEFAULT_POOL_KIND: &str = "default";
const PROBE_REFRESH_INTERVAL_MS: u64 = 15_000;
const FAILURE_COOLDOWN_MS: u64 = 15_000;

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
        code: ErrorCode(code.to_string()),
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
        code: ErrorCode("control_plane_concurrency".to_string()),
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
        | IoError::Other(info) => info.code.0.clone(),
        IoError::MissingFact { info, .. } => info.code.0.clone(),
    }
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
        .flat_map(|raw| {
            raw.split(',')
                .map(str::trim)
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .filter(|s| !s.is_empty())
        .collect()
}

fn parse_source_kind(raw: Option<&str>) -> EvmSourceKind {
    let normalized = raw.unwrap_or("remote_public").trim().to_ascii_lowercase();
    match normalized.as_str() {
        "local" | "local_helios" | "local_reth" => EvmSourceKind::Local,
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

/// Bootstrap source definition used by the `rpc.control` transport.
#[derive(Clone, Debug, PartialEq, Eq)]
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
                write!(f, "rpc.control source `{source_id}` must declare network_id")
            }
        }
    }
}

impl std::error::Error for RpcControlConfigError {}

#[derive(Debug, Clone, Deserialize)]
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
    pub fn from_env() -> Self {
        let catalog = resolve_bootstrap_catalog_from_env();
        let config_error = validate_catalog(&catalog).err();
        Self {
            catalog,
            config_error,
            control_plane_storage_mode: RpcControlPlaneStorageMode::default(),
            executor_tuning: RpcControlExecutorTuning::default(),
        }
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
}

impl Default for RpcControlTransportFactory {
    fn default() -> Self {
        Self::from_env()
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
        let executor = if self.config_error.is_none() {
            Some(
                EvmJsonRpcHttpTransportFactory::new(inner_executor_config(
                    &self.catalog,
                    self.executor_tuning,
                ))
                .make(env),
            )
        } else {
            None
        };

        Box::new(RpcControlTransport {
            executor,
            control_plane_store,
            catalog: self.catalog.clone(),
            config_error: self.config_error.clone(),
        })
    }
}

struct RpcControlTransport {
    executor: Option<Box<dyn LiveIoTransport>>,
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
                Err(IoError::Other(info)) if info.code.0 == "control_plane_concurrency" => {
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
                Err(IoError::Other(info)) if info.code.0 == "control_plane_concurrency" => {
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
        let current_pool = Some(self.ensure_declared_catalog(&pool_ref, &catalog_snapshot).await?);
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
                self.probe_source(control_scope, network_scope, source).await?
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
            RpcControlRequest::PrepareSources {
                control_scope,
                network_id,
            } => {
                self.handle_prepare_sources(&control_scope, &network_id)
                    .await
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use mfm_stream_store_mem::MemStreamStore;

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

    fn transport_for_tests(sources: Vec<RpcControlBootstrapSource>) -> RpcControlTransport {
        let preferred_order = sources.iter().map(|source| source.id.clone()).collect();
        RpcControlTransport {
            executor: None,
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

    #[test]
    fn bootstrap_catalog_rejects_missing_network_id() {
        std::env::remove_var(ENV_EVM_RPC_SOURCES_JSON);
        let catalog = BootstrapCatalog {
            sources: vec![source("primary", None, EvmSourceKind::RemoteUser, false)],
            preferred_order: vec!["primary".to_string()],
        };
        let err = validate_catalog(&catalog).expect_err("missing network_id must be rejected");
        assert_eq!(err, RpcControlConfigError::MissingNetworkId("primary".to_string()));
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
                source_ref: RpcSourceRef::new("shared", "ethereum-mainnet", "local_fast")
                    .unwrap(),
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
                source_ref: RpcSourceRef::new("shared", "ethereum-mainnet", "remote_slow")
                    .unwrap(),
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
}
