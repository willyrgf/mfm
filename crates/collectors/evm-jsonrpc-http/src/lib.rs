//! EVM JSON-RPC over HTTP live transport.
//!
//! This crate implements a `LiveIoTransportFactory` for the `namespace = "evm"` IO surface.
//! It supports source-id based routing, retry-oriented failover, and optional light-call hedging.
//!
//! Security notes:
//! - RPC URLs and authorization headers are runtime configuration and MUST NOT be persisted.
//! - Errors MUST NOT include request payloads, response bodies, or authorization values.

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet, VecDeque};
use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;
use tracing::{debug, warn};

use mfm_collectors_evm::JsonRpcCall;
use mfm_machine::errors::{ErrorCategory, ErrorInfo, IoError};
use mfm_machine::ids::ErrorCode;
use mfm_machine::io::IoCall;
use mfm_machine::live_io::{LiveIoEnv, LiveIoTransport, LiveIoTransportFactory};

const CODE_EVM_REQUEST_INVALID: &str = "evm_request_invalid";
const CODE_EVM_HTTP_REQUEST_FAILED: &str = "evm_http_request_failed";
const CODE_EVM_HTTP_STATUS: &str = "evm_http_status";
const CODE_EVM_HTTP_BODY_READ_FAILED: &str = "evm_http_body_read_failed";
const CODE_EVM_RESPONSE_INVALID_JSON: &str = "evm_response_invalid_json";
const CODE_EVM_JSONRPC_ERROR: &str = "evm_jsonrpc_error";
const CODE_EVM_JSONRPC_MISSING_RESULT: &str = "evm_jsonrpc_missing_result";
const CODE_EVM_JSONRPC_INVALID_RESPONSE: &str = "evm_jsonrpc_invalid_response";
const CODE_EVM_RATE_LIMITED: &str = "evm_rate_limited";
const CODE_EVM_SOURCE_UNHEALTHY: &str = "evm_source_unhealthy";
const CODE_EVM_NO_HEALTHY_SOURCE: &str = "evm_no_healthy_source";
const CODE_EVM_HEDGE_EXHAUSTED: &str = "evm_hedge_exhausted";
const CODE_EVM_ROUTE_SOURCE_UNKNOWN: &str = "evm_route_source_unknown";
const CODE_EVM_CONFIG_INVALID: &str = "evm_config_invalid";
const CODE_EVM_LOGS_CHUNKING_INVALID_RANGE: &str = "evm_logs_chunking_invalid_range";
const CODE_EVM_LOGS_CHUNKING_EXHAUSTED: &str = "evm_logs_chunking_exhausted";
const METHOD_EVM_ROUTING_ANALYSIS: &str = "mfm_debugRoutingAnalysis";
const ENV_EVM_RPC_URL: &str = "MFM_EVM_RPC_URL";
const ENV_EVM_RPC_AUTHORIZATION: &str = "MFM_EVM_RPC_AUTHORIZATION";
const ENV_EVM_RPC_SOURCES_JSON: &str = "MFM_EVM_RPC_SOURCES_JSON";
const ENV_EVM_RPC_PREFERRED_ORDER: &str = "MFM_EVM_RPC_PREFERRED_ORDER";
const ENV_EVM_RPC_STRATEGY: &str = "MFM_EVM_RPC_STRATEGY";
const ENV_EVM_RPC_HEDGE_DELAY_MS: &str = "MFM_EVM_RPC_HEDGE_DELAY_MS";
const ENV_EVM_RPC_UNHEALTHY_COOLDOWN_CALLS: &str = "MFM_EVM_RPC_UNHEALTHY_COOLDOWN_CALLS";
const ENV_EVM_RPC_REQUIRE_GET_PROOF_IDS: &str = "MFM_EVM_RPC_REQUIRE_GET_PROOF_IDS";
const ENV_EVM_RPC_LOGS_MAX_BLOCK_SPAN: &str = "MFM_EVM_RPC_LOGS_MAX_BLOCK_SPAN";
const ENV_EVM_RPC_LOGS_MIN_BLOCK_SPAN: &str = "MFM_EVM_RPC_LOGS_MIN_BLOCK_SPAN";
const ENV_EVM_RPC_LOGS_MAX_CHUNKS_PER_CALL: &str = "MFM_EVM_RPC_LOGS_MAX_CHUNKS_PER_CALL";

const MAX_DIAGNOSTIC_MESSAGE_LEN: usize = 240;

fn info(
    code: &'static str,
    category: ErrorCategory,
    retryable: bool,
    message: &'static str,
) -> ErrorInfo {
    ErrorInfo {
        code: ErrorCode(code.to_string()),
        category,
        retryable,
        message: message.to_string(),
        details: None,
    }
}

fn info_with_details(
    code: &'static str,
    category: ErrorCategory,
    retryable: bool,
    message: &'static str,
    details: Option<serde_json::Value>,
) -> ErrorInfo {
    ErrorInfo {
        code: ErrorCode(code.to_string()),
        category,
        retryable,
        message: message.to_string(),
        details,
    }
}

fn truncate_message(raw: &str) -> String {
    if raw.len() <= MAX_DIAGNOSTIC_MESSAGE_LEN {
        raw.to_string()
    } else {
        raw.chars().take(MAX_DIAGNOSTIC_MESSAGE_LEN).collect()
    }
}

fn source_error_details(
    source_id: &str,
    extra: Option<serde_json::Value>,
) -> Option<serde_json::Value> {
    let mut obj = serde_json::Map::new();
    obj.insert("source_id".to_string(), json!(source_id));

    if let Some(extra) = extra.and_then(|v| v.as_object().cloned()) {
        for (k, v) in extra {
            obj.insert(k, v);
        }
    }

    Some(serde_json::Value::Object(obj))
}

fn jsonrpc_error_details(
    source_id: &str,
    error_value: &serde_json::Value,
) -> Option<serde_json::Value> {
    let error = error_value.as_object()?;
    let code = error.get("code").and_then(|v| v.as_i64());
    let message = error
        .get("message")
        .and_then(|v| v.as_str())
        .map(truncate_message);

    if code.is_none() && message.is_none() {
        return source_error_details(source_id, None);
    }

    source_error_details(
        source_id,
        Some(json!({
            "jsonrpc_error_code": code,
            "jsonrpc_error_message": message,
        })),
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvmSourceKind {
    Local,
    RemotePublic,
    RemoteUser,
}

impl EvmSourceKind {
    fn priority(self) -> u8 {
        match self {
            EvmSourceKind::Local => 0,
            EvmSourceKind::RemoteUser => 1,
            EvmSourceKind::RemotePublic => 2,
        }
    }
}

#[derive(Debug, Clone)]
pub struct EvmJsonRpcSource {
    pub id: String,
    pub rpc_url: String,
    pub authorization: Option<String>,
    pub kind: EvmSourceKind,
    pub require_get_proof_probe: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvmRoutingStrategy {
    Failover,
    HedgedLight,
}

#[derive(Clone)]
pub struct EvmJsonRpcHttpConfig {
    pub sources: Vec<EvmJsonRpcSource>,
    pub preferred_order: Vec<String>,
    pub strategy: EvmRoutingStrategy,
    pub hedge_delay: Duration,
    pub timeout: Duration,
    pub unhealthy_cooldown_calls: u64,
    pub hedge_max_eth_call_params_bytes: usize,
    pub logs_max_block_span: u64,
    pub logs_min_block_span: u64,
    pub logs_max_chunks_per_call: u64,
}

impl Default for EvmJsonRpcHttpConfig {
    fn default() -> Self {
        Self {
            sources: Vec::new(),
            preferred_order: Vec::new(),
            strategy: EvmRoutingStrategy::HedgedLight,
            hedge_delay: Duration::from_millis(100),
            timeout: Duration::from_secs(30),
            unhealthy_cooldown_calls: 2,
            hedge_max_eth_call_params_bytes: 4096,
            logs_max_block_span: 2_000,
            logs_min_block_span: 64,
            logs_max_chunks_per_call: 256,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvmJsonRpcHttpConfigError {
    NoSources,
    EmptySourceId,
    DuplicateSourceId(String),
    UnknownPreferredSourceId(String),
    InvalidLogsChunkingRange,
    InvalidLogsChunkingChunkLimit,
}

impl std::fmt::Display for EvmJsonRpcHttpConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EvmJsonRpcHttpConfigError::NoSources => {
                write!(f, "evm source registry is empty")
            }
            EvmJsonRpcHttpConfigError::EmptySourceId => {
                write!(f, "evm source id must not be empty")
            }
            EvmJsonRpcHttpConfigError::DuplicateSourceId(id) => {
                write!(f, "duplicate evm source id: {id}")
            }
            EvmJsonRpcHttpConfigError::UnknownPreferredSourceId(id) => {
                write!(f, "preferred source id not found in registry: {id}")
            }
            EvmJsonRpcHttpConfigError::InvalidLogsChunkingRange => {
                write!(
                    f,
                    "logs chunking config is invalid: min/max block span must be > 0 and min <= max"
                )
            }
            EvmJsonRpcHttpConfigError::InvalidLogsChunkingChunkLimit => {
                write!(
                    f,
                    "logs chunking config is invalid: max chunks per call must be > 0"
                )
            }
        }
    }
}

impl std::error::Error for EvmJsonRpcHttpConfigError {}

fn validate_config(cfg: &EvmJsonRpcHttpConfig) -> Result<(), EvmJsonRpcHttpConfigError> {
    if cfg.sources.is_empty() {
        return Err(EvmJsonRpcHttpConfigError::NoSources);
    }

    let mut seen: HashSet<&str> = HashSet::new();
    for source in &cfg.sources {
        if source.id.trim().is_empty() {
            return Err(EvmJsonRpcHttpConfigError::EmptySourceId);
        }
        if !seen.insert(source.id.as_str()) {
            return Err(EvmJsonRpcHttpConfigError::DuplicateSourceId(
                source.id.clone(),
            ));
        }
    }

    for preferred in &cfg.preferred_order {
        if !seen.contains(preferred.as_str()) {
            return Err(EvmJsonRpcHttpConfigError::UnknownPreferredSourceId(
                preferred.clone(),
            ));
        }
    }

    if cfg.logs_min_block_span == 0
        || cfg.logs_max_block_span == 0
        || cfg.logs_min_block_span > cfg.logs_max_block_span
    {
        return Err(EvmJsonRpcHttpConfigError::InvalidLogsChunkingRange);
    }
    if cfg.logs_max_chunks_per_call == 0 {
        return Err(EvmJsonRpcHttpConfigError::InvalidLogsChunkingChunkLimit);
    }

    Ok(())
}

#[derive(Clone)]
pub struct EvmJsonRpcHttpTransportFactory {
    cfg: EvmJsonRpcHttpConfig,
    client: reqwest::Client,
    config_error: Option<EvmJsonRpcHttpConfigError>,
}

#[derive(Debug, Clone, Deserialize)]
struct EnvEvmRpcSource {
    id: String,
    rpc_url: String,
    #[serde(default)]
    authorization: Option<String>,
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    require_get_proof_probe: bool,
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

fn parse_u64_env(var_name: &str) -> Option<u64> {
    let Ok(raw) = std::env::var(var_name) else {
        return None;
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    match trimmed.parse::<u64>() {
        Ok(v) => Some(v),
        Err(_) => {
            warn!(env_var = var_name, "ignoring invalid numeric env var");
            None
        }
    }
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

pub fn resolve_evm_rpc_sources_from_env() -> Vec<EvmJsonRpcSource> {
    if let Ok(raw_json) = std::env::var(ENV_EVM_RPC_SOURCES_JSON) {
        let trimmed = raw_json.trim();
        if !trimmed.is_empty() {
            match serde_json::from_str::<Vec<EnvEvmRpcSource>>(trimmed) {
                Ok(parsed) => {
                    let mut out = Vec::new();
                    for source in parsed {
                        if source.id.trim().is_empty() || source.rpc_url.trim().is_empty() {
                            warn!("skipping evm rpc source with empty id or rpc_url");
                            continue;
                        }
                        out.push(EvmJsonRpcSource {
                            id: source.id.trim().to_string(),
                            rpc_url: source.rpc_url.trim().to_string(),
                            authorization: source.authorization,
                            kind: parse_source_kind(source.kind.as_deref()),
                            require_get_proof_probe: source.require_get_proof_probe,
                        });
                    }
                    return out;
                }
                Err(_) => {
                    warn!("failed to parse MFM_EVM_RPC_SOURCES_JSON; falling back to legacy env");
                }
            }
        }
    }

    let rpc_url = std::env::var(ENV_EVM_RPC_URL)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty());
    let Some(rpc_url) = rpc_url else {
        return Vec::new();
    };

    vec![EvmJsonRpcSource {
        id: "user_primary".to_string(),
        rpc_url,
        authorization: std::env::var(ENV_EVM_RPC_AUTHORIZATION).ok(),
        kind: EvmSourceKind::RemoteUser,
        require_get_proof_probe: false,
    }]
}

fn resolve_evm_routing_strategy_from_env() -> EvmRoutingStrategy {
    let Some(raw) = std::env::var(ENV_EVM_RPC_STRATEGY).ok() else {
        return EvmRoutingStrategy::HedgedLight;
    };

    match raw.trim().to_ascii_lowercase().as_str() {
        "failover" => EvmRoutingStrategy::Failover,
        "hedged_light" | "hedged" => EvmRoutingStrategy::HedgedLight,
        _ => {
            warn!(
                env_var = ENV_EVM_RPC_STRATEGY,
                "unknown evm routing strategy; using hedged_light"
            );
            EvmRoutingStrategy::HedgedLight
        }
    }
}

fn resolve_evm_rpc_config_from_env() -> EvmJsonRpcHttpConfig {
    let mut sources = resolve_evm_rpc_sources_from_env();
    let require_get_proof_ids = parse_csv_env(ENV_EVM_RPC_REQUIRE_GET_PROOF_IDS)
        .into_iter()
        .collect::<HashSet<_>>();
    for source in &mut sources {
        if require_get_proof_ids.contains(&source.id) {
            source.require_get_proof_probe = true;
        }
    }

    let mut preferred_order = parse_csv_env(ENV_EVM_RPC_PREFERRED_ORDER);
    if preferred_order.is_empty() {
        preferred_order = sources.iter().map(|s| s.id.clone()).collect();
    }

    let mut cfg = EvmJsonRpcHttpConfig {
        sources,
        preferred_order,
        ..EvmJsonRpcHttpConfig::default()
    };
    cfg.strategy = resolve_evm_routing_strategy_from_env();
    if let Some(hedge_delay_ms) = parse_u64_env(ENV_EVM_RPC_HEDGE_DELAY_MS) {
        cfg.hedge_delay = Duration::from_millis(hedge_delay_ms);
    }
    if let Some(cooldown_calls) = parse_u64_env(ENV_EVM_RPC_UNHEALTHY_COOLDOWN_CALLS) {
        cfg.unhealthy_cooldown_calls = cooldown_calls;
    }
    if let Some(max_span) = parse_u64_env(ENV_EVM_RPC_LOGS_MAX_BLOCK_SPAN) {
        cfg.logs_max_block_span = max_span;
    }
    if let Some(min_span) = parse_u64_env(ENV_EVM_RPC_LOGS_MIN_BLOCK_SPAN) {
        cfg.logs_min_block_span = min_span;
    }
    if let Some(max_chunks) = parse_u64_env(ENV_EVM_RPC_LOGS_MAX_CHUNKS_PER_CALL) {
        cfg.logs_max_chunks_per_call = max_chunks;
    }
    cfg
}

impl EvmJsonRpcHttpTransportFactory {
    pub fn new(cfg: EvmJsonRpcHttpConfig) -> Self {
        let client = reqwest::Client::builder()
            .timeout(cfg.timeout)
            .build()
            .expect("reqwest client must build");
        let config_error = validate_config(&cfg).err();
        Self {
            cfg,
            client,
            config_error,
        }
    }

    pub fn try_new(cfg: EvmJsonRpcHttpConfig) -> Result<Self, EvmJsonRpcHttpConfigError> {
        validate_config(&cfg)?;
        let client = reqwest::Client::builder()
            .timeout(cfg.timeout)
            .build()
            .expect("reqwest client must build");
        Ok(Self {
            cfg,
            client,
            config_error: None,
        })
    }

    pub fn from_env() -> Result<Self, EvmJsonRpcHttpConfigError> {
        Ok(Self::new(resolve_evm_rpc_config_from_env()))
    }
}

impl LiveIoTransportFactory for EvmJsonRpcHttpTransportFactory {
    fn namespace_group(&self) -> &str {
        "evm"
    }

    fn make(&self, _env: LiveIoEnv) -> Box<dyn LiveIoTransport> {
        let mut source_states = HashMap::new();
        for source in &self.cfg.sources {
            source_states.insert(
                source.id.clone(),
                SourceRuntimeState {
                    source: source.clone(),
                    score: 0,
                    disabled_until_call: 0,
                    probed: false,
                },
            );
        }

        let base_order = source_base_order(&self.cfg);

        Box::new(EvmJsonRpcHttpTransport {
            cfg: self.cfg.clone(),
            client: self.client.clone(),
            next_id: 1,
            call_ordinal: 0,
            source_states,
            base_order,
            config_error: self.config_error.clone(),
        })
    }
}

#[derive(Clone)]
struct SourceRuntimeState {
    source: EvmJsonRpcSource,
    score: i32,
    disabled_until_call: u64,
    probed: bool,
}

struct EvmJsonRpcHttpTransport {
    cfg: EvmJsonRpcHttpConfig,
    client: reqwest::Client,
    next_id: u64,
    call_ordinal: u64,
    source_states: HashMap<String, SourceRuntimeState>,
    base_order: HashMap<String, usize>,
    config_error: Option<EvmJsonRpcHttpConfigError>,
}

#[derive(Debug, Clone, Deserialize)]
struct EvmRouteHint {
    source_id: String,
}

#[derive(Debug, Clone, Deserialize)]
struct EvmTransportRequest {
    method: String,
    params: serde_json::Value,
    #[serde(default)]
    route: Option<EvmRouteHint>,
    #[serde(default)]
    rpc_url: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MethodClass {
    ReadLight,
    ReadHeavy,
    WriteOrSideEffect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DispatchMode {
    PrimaryOnly,
    Failover,
    HedgedLight,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MethodPolicy {
    class: MethodClass,
    dispatch: DispatchMode,
    chunk_logs: bool,
}

#[derive(Debug, Clone, Deserialize)]
struct EvmRoutingAnalysisRequest {
    method: String,
    #[serde(default = "default_jsonrpc_params")]
    params: serde_json::Value,
    #[serde(default)]
    route: Option<EvmRouteHint>,
}

fn default_jsonrpc_params() -> serde_json::Value {
    json!([])
}

fn method_class_name(class: MethodClass) -> &'static str {
    match class {
        MethodClass::ReadLight => "read_light",
        MethodClass::ReadHeavy => "read_heavy",
        MethodClass::WriteOrSideEffect => "write_or_side_effect",
    }
}

fn dispatch_mode_name(dispatch: DispatchMode) -> &'static str {
    match dispatch {
        DispatchMode::PrimaryOnly => "primary_only",
        DispatchMode::Failover => "failover",
        DispatchMode::HedgedLight => "hedged_light",
    }
}

fn source_kind_name(kind: EvmSourceKind) -> &'static str {
    match kind {
        EvmSourceKind::Local => "local",
        EvmSourceKind::RemotePublic => "remote_public",
        EvmSourceKind::RemoteUser => "remote_user",
    }
}

fn routing_strategy_name(strategy: EvmRoutingStrategy) -> &'static str {
    match strategy {
        EvmRoutingStrategy::Failover => "failover",
        EvmRoutingStrategy::HedgedLight => "hedged_light",
    }
}

fn rpc_endpoint_name(rpc_url: &str) -> String {
    let Ok(parsed) = reqwest::Url::parse(rpc_url) else {
        return "<unavailable>".to_string();
    };
    let Some(host_raw) = parsed.host_str() else {
        return "<unavailable>".to_string();
    };
    let Some(port) = parsed.port_or_known_default() else {
        return "<unavailable>".to_string();
    };
    let host = if host_raw.contains(':') && !host_raw.starts_with('[') {
        format!("[{host_raw}]")
    } else {
        host_raw.to_string()
    };
    format!("{}://{}:{}", parsed.scheme(), host, port)
}

fn parse_hex_u64(raw: &str) -> Option<u64> {
    let trimmed = raw.strip_prefix("0x")?;
    if trimmed.is_empty() {
        return None;
    }
    u64::from_str_radix(trimmed, 16).ok()
}

fn is_write_method(method_lc: &str) -> bool {
    method_lc == "eth_sendrawtransaction"
        || method_lc == "eth_sendtransaction"
        || method_lc.starts_with("personal_")
        || method_lc.starts_with("admin_")
        || method_lc.starts_with("miner_")
        || method_lc.starts_with("txpool_")
        || method_lc.starts_with("engine_")
}

fn is_explicit_heavy_method(method_lc: &str) -> bool {
    method_lc == "eth_getlogs" || method_lc.starts_with("trace_") || method_lc.starts_with("debug_")
}

fn is_light_allowlist_method(method_lc: &str) -> bool {
    matches!(
        method_lc,
        "eth_chainid"
            | "eth_blocknumber"
            | "eth_getbalance"
            | "eth_call"
            | "eth_gettransactionreceipt"
            | "eth_getblockbynumber"
    )
}

fn classify_method(
    method: &str,
    params: &serde_json::Value,
    cfg: &EvmJsonRpcHttpConfig,
) -> MethodClass {
    let method_lc = method.to_ascii_lowercase();

    if is_write_method(&method_lc) {
        return MethodClass::WriteOrSideEffect;
    }

    if is_explicit_heavy_method(&method_lc) {
        return MethodClass::ReadHeavy;
    }

    if method_lc == "eth_call" {
        let size = serde_json::to_vec(params)
            .map(|b| b.len())
            .unwrap_or(usize::MAX);
        if size > cfg.hedge_max_eth_call_params_bytes {
            return MethodClass::ReadHeavy;
        }
    }

    if is_light_allowlist_method(&method_lc) {
        return MethodClass::ReadLight;
    }

    MethodClass::ReadHeavy
}

fn resolve_method_policy(
    method: &str,
    params: &serde_json::Value,
    cfg: &EvmJsonRpcHttpConfig,
) -> MethodPolicy {
    let class = classify_method(method, params, cfg);
    let dispatch = match class {
        MethodClass::WriteOrSideEffect => DispatchMode::PrimaryOnly,
        MethodClass::ReadHeavy => DispatchMode::Failover,
        MethodClass::ReadLight => match cfg.strategy {
            EvmRoutingStrategy::Failover => DispatchMode::Failover,
            EvmRoutingStrategy::HedgedLight => DispatchMode::HedgedLight,
        },
    };
    let chunk_logs = method.eq_ignore_ascii_case("eth_getLogs");
    MethodPolicy {
        class,
        dispatch,
        chunk_logs,
    }
}

fn source_base_order(cfg: &EvmJsonRpcHttpConfig) -> HashMap<String, usize> {
    let mut ordered_ids: Vec<String> = Vec::new();
    let mut seen = HashSet::new();

    for id in &cfg.preferred_order {
        if seen.insert(id.clone()) {
            ordered_ids.push(id.clone());
        }
    }
    for source in &cfg.sources {
        if seen.insert(source.id.clone()) {
            ordered_ids.push(source.id.clone());
        }
    }

    ordered_ids
        .into_iter()
        .enumerate()
        .map(|(idx, id)| (id, idx))
        .collect()
}

fn err_info(err: &IoError) -> &ErrorInfo {
    match err {
        IoError::MissingFactKey(info)
        | IoError::MissingFact { info, .. }
        | IoError::Transport(info)
        | IoError::RateLimited(info)
        | IoError::Other(info) => info,
    }
}

fn is_retryable(err: &IoError) -> bool {
    err_info(err).retryable
}

fn failure_detail(source_id: &str, err: &IoError) -> serde_json::Value {
    let info = err_info(err);
    json!({
        "source_id": source_id,
        "code": info.code.0,
        "retryable": info.retryable,
    })
}

fn io_error_summary(err: &IoError) -> serde_json::Value {
    let info = err_info(err);
    json!({
        "code": info.code.0,
        "category": info.category,
        "retryable": info.retryable,
        "message": info.message,
        "details": info.details,
    })
}

#[derive(Clone)]
struct PreparedCall {
    source_id: String,
    source: EvmJsonRpcSource,
    body: serde_json::Value,
}

impl EvmJsonRpcHttpTransport {
    fn current_call_ordinal(&mut self) -> u64 {
        let current = self.call_ordinal;
        self.call_ordinal = self.call_ordinal.saturating_add(1);
        current
    }

    fn next_request_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        id
    }

    fn request_body(&mut self, req: &JsonRpcCall) -> serde_json::Value {
        let id = self.next_request_id();
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": req.method,
            "params": req.params,
        })
    }

    fn config_error_info(&self) -> Option<ErrorInfo> {
        self.config_error.as_ref().map(|err| {
            info_with_details(
                CODE_EVM_CONFIG_INVALID,
                ErrorCategory::ParsingInput,
                false,
                "evm transport configuration is invalid",
                Some(json!({ "reason": err.to_string() })),
            )
        })
    }

    fn routing_analysis_request_invalid_error(&self) -> IoError {
        IoError::Other(info(
            CODE_EVM_REQUEST_INVALID,
            ErrorCategory::ParsingInput,
            false,
            "invalid evm routing analysis request",
        ))
    }

    fn source_analysis_entry(
        &self,
        source_id: &str,
        call_ordinal: u64,
    ) -> Option<serde_json::Value> {
        let state = self.source_states.get(source_id)?;
        let healthy_for_call = state.disabled_until_call <= call_ordinal;
        let cooldown_remaining_calls = if healthy_for_call {
            0
        } else {
            state.disabled_until_call.saturating_sub(call_ordinal)
        };
        Some(json!({
            "source_id": source_id,
            "kind": source_kind_name(state.source.kind),
            "score": state.score,
            "base_rank": self.base_order.get(source_id).copied(),
            "healthy_for_call": healthy_for_call,
            "disabled_until_call": state.disabled_until_call,
            "cooldown_remaining_calls": cooldown_remaining_calls,
            "probed": state.probed,
        }))
    }

    fn ranked_source_analysis(&self, call_ordinal: u64) -> Vec<serde_json::Value> {
        let mut ids = self.source_states.keys().cloned().collect::<Vec<_>>();
        ids.sort_by(|a, b| self.compare_sources(a, b));
        ids.into_iter()
            .filter_map(|id| self.source_analysis_entry(&id, call_ordinal))
            .collect()
    }

    fn build_routing_analysis(
        &self,
        target_method: &str,
        target_params: &serde_json::Value,
        route_source_id: Option<&str>,
        call_ordinal: u64,
    ) -> serde_json::Value {
        let method_policy = resolve_method_policy(target_method, target_params, &self.cfg);
        let params_size_bytes = serde_json::to_vec(target_params)
            .map(|bytes| bytes.len())
            .unwrap_or(usize::MAX);
        let ordered = self.ordered_source_ids(route_source_id, call_ordinal);
        let (selected_order, selection_error) = match ordered {
            Ok(ids) => (ids, None),
            Err(err) => (Vec::new(), Some(io_error_summary(&err))),
        };
        let pending_probe_sources = selected_order
            .iter()
            .filter(|source_id| {
                self.source_states
                    .get(*source_id)
                    .map(|state| !state.probed)
                    .unwrap_or(false)
            })
            .cloned()
            .collect::<Vec<_>>();

        json!({
            "operation": METHOD_EVM_ROUTING_ANALYSIS,
            "analysis_version": 1,
            "call_ordinal": call_ordinal,
            "target": {
                "method": target_method,
                "route_source_id": route_source_id,
                "params_size_bytes": params_size_bytes,
            },
            "routing": {
                "strategy": routing_strategy_name(self.cfg.strategy),
                "method_class": method_class_name(method_policy.class),
                "dispatch_mode": dispatch_mode_name(method_policy.dispatch),
                "chunk_logs": method_policy.chunk_logs,
                "selected_order": selected_order,
                "pending_probe_sources": pending_probe_sources,
                "selection_error": selection_error,
            },
            "sources_ranked": self.ranked_source_analysis(call_ordinal),
        })
    }

    fn call_routing_analysis_operation(
        &self,
        request: &EvmTransportRequest,
    ) -> Result<serde_json::Value, IoError> {
        if request.route.is_some() || request.rpc_url.is_some() {
            return Err(self.routing_analysis_request_invalid_error());
        }

        let target: EvmRoutingAnalysisRequest = serde_json::from_value(request.params.clone())
            .map_err(|_| self.routing_analysis_request_invalid_error())?;
        if target.method.trim().is_empty() {
            return Err(self.routing_analysis_request_invalid_error());
        }

        let route_source_id = target.route.as_ref().map(|route| route.source_id.as_str());
        Ok(self.build_routing_analysis(
            &target.method,
            &target.params,
            route_source_id,
            self.call_ordinal,
        ))
    }

    fn score_recovery_bonus(method_class: MethodClass) -> i32 {
        match method_class {
            MethodClass::ReadLight => 1,
            MethodClass::ReadHeavy => 2,
            MethodClass::WriteOrSideEffect => 3,
        }
    }

    fn score_failure_penalty(err: &IoError, method_class: MethodClass) -> i32 {
        let info = err_info(err);
        let mut penalty = match info.code.0.as_str() {
            CODE_EVM_RATE_LIMITED => 8,
            CODE_EVM_HTTP_REQUEST_FAILED => {
                let class = info
                    .details
                    .as_ref()
                    .and_then(|v| v.get("transport_error_class"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("transport");
                if class == "timeout" {
                    6
                } else {
                    5
                }
            }
            CODE_EVM_HTTP_STATUS => {
                let status_class = info
                    .details
                    .as_ref()
                    .and_then(|v| v.get("http_status_class"))
                    .and_then(|v| v.as_u64())
                    .unwrap_or(5);
                if status_class >= 5 {
                    5
                } else {
                    4
                }
            }
            CODE_EVM_JSONRPC_ERROR => 4,
            CODE_EVM_SOURCE_UNHEALTHY | CODE_EVM_NO_HEALTHY_SOURCE | CODE_EVM_HEDGE_EXHAUSTED => 3,
            _ => 3,
        };

        penalty += match method_class {
            MethodClass::ReadLight => 0,
            MethodClass::ReadHeavy => 1,
            MethodClass::WriteOrSideEffect => 2,
        };

        penalty.clamp(1, 20)
    }

    fn mark_source_success(&mut self, source_id: &str, method_class: MethodClass) {
        if let Some(state) = self.source_states.get_mut(source_id) {
            state.score = (state.score + Self::score_recovery_bonus(method_class)).clamp(-100, 100);
            state.disabled_until_call = 0;
        }
    }

    fn mark_source_failure(&mut self, source_id: &str, call_ordinal: u64, penalty: i32) {
        if let Some(state) = self.source_states.get_mut(source_id) {
            state.score = (state.score - penalty).clamp(-100, 100);
            if self.cfg.unhealthy_cooldown_calls > 0 {
                state.disabled_until_call =
                    call_ordinal.saturating_add(self.cfg.unhealthy_cooldown_calls);
            }
        }
    }

    fn mark_source_failure_with_error(
        &mut self,
        source_id: &str,
        call_ordinal: u64,
        err: &IoError,
        method_class: MethodClass,
    ) {
        let penalty = Self::score_failure_penalty(err, method_class);
        self.mark_source_failure(source_id, call_ordinal, penalty);
    }

    fn source_unhealthy_error(&self, source_id: &str) -> IoError {
        IoError::Transport(info_with_details(
            CODE_EVM_SOURCE_UNHEALTHY,
            ErrorCategory::Rpc,
            true,
            "evm source is marked unhealthy",
            source_error_details(source_id, None),
        ))
    }

    fn no_healthy_source_error(&self, failures: &[serde_json::Value]) -> IoError {
        IoError::Transport(info_with_details(
            CODE_EVM_NO_HEALTHY_SOURCE,
            ErrorCategory::Rpc,
            true,
            "no healthy evm source is available",
            Some(json!({ "failures": failures })),
        ))
    }

    fn hedge_exhausted_error(&self, failures: &[serde_json::Value]) -> IoError {
        IoError::Transport(info_with_details(
            CODE_EVM_HEDGE_EXHAUSTED,
            ErrorCategory::Rpc,
            true,
            "evm hedged call failed for all candidates",
            Some(json!({ "failures": failures })),
        ))
    }

    fn logs_chunking_invalid_range_error(
        &self,
        message: &'static str,
        details: Option<serde_json::Value>,
    ) -> IoError {
        IoError::Other(info_with_details(
            CODE_EVM_LOGS_CHUNKING_INVALID_RANGE,
            ErrorCategory::ParsingInput,
            false,
            message,
            details,
        ))
    }

    fn logs_chunking_exhausted_error(
        &self,
        attempted_chunks: u64,
        failures: &[serde_json::Value],
    ) -> IoError {
        IoError::Transport(info_with_details(
            CODE_EVM_LOGS_CHUNKING_EXHAUSTED,
            ErrorCategory::Rpc,
            true,
            "eth_getLogs chunking exhausted retry budget",
            Some(json!({
                "attempted_chunks": attempted_chunks,
                "failures": failures,
            })),
        ))
    }

    fn route_source_unknown_error(&self, source_id: &str) -> IoError {
        IoError::Other(info_with_details(
            CODE_EVM_ROUTE_SOURCE_UNKNOWN,
            ErrorCategory::ParsingInput,
            false,
            "route.source_id is not registered in evm source pool",
            source_error_details(source_id, None),
        ))
    }

    fn rpc_url_override_error(&self) -> IoError {
        IoError::Other(info(
            CODE_EVM_REQUEST_INVALID,
            ErrorCategory::ParsingInput,
            false,
            "per-request rpc_url override is not allowed for evm calls",
        ))
    }

    fn invalid_request_error(&self) -> IoError {
        IoError::Other(info(
            CODE_EVM_REQUEST_INVALID,
            ErrorCategory::ParsingInput,
            false,
            "invalid evm jsonrpc request",
        ))
    }

    fn source_is_healthy(&self, source_id: &str, call_ordinal: u64) -> bool {
        self.source_states
            .get(source_id)
            .map(|state| state.disabled_until_call <= call_ordinal)
            .unwrap_or(false)
    }

    fn ordered_source_ids(
        &self,
        route_source_id: Option<&str>,
        call_ordinal: u64,
    ) -> Result<Vec<String>, IoError> {
        if let Some(id) = route_source_id {
            if !self.source_states.contains_key(id) {
                return Err(self.route_source_unknown_error(id));
            }
            if !self.source_is_healthy(id, call_ordinal) {
                return Err(self.source_unhealthy_error(id));
            }
            return Ok(vec![id.to_string()]);
        }

        let mut ids: Vec<String> = self
            .source_states
            .iter()
            .filter_map(|(id, state)| {
                if state.disabled_until_call <= call_ordinal {
                    Some(id.clone())
                } else {
                    None
                }
            })
            .collect();

        ids.sort_by(|a, b| self.compare_sources(a, b));
        if ids.is_empty() {
            return Err(self.no_healthy_source_error(&[]));
        }
        Ok(ids)
    }

    fn compare_sources(&self, a: &str, b: &str) -> Ordering {
        let Some(sa) = self.source_states.get(a) else {
            return Ordering::Equal;
        };
        let Some(sb) = self.source_states.get(b) else {
            return Ordering::Equal;
        };

        sb.score
            .cmp(&sa.score)
            .then_with(|| sa.source.kind.priority().cmp(&sb.source.kind.priority()))
            .then_with(|| {
                let a_rank = self.base_order.get(a).copied().unwrap_or(usize::MAX);
                let b_rank = self.base_order.get(b).copied().unwrap_or(usize::MAX);
                a_rank.cmp(&b_rank)
            })
    }

    async fn resolve_latest_block_number(
        &mut self,
        source_ids: &[String],
        call_ordinal: u64,
    ) -> Result<u64, IoError> {
        let latest_call = JsonRpcCall::new("eth_blockNumber", json!([]));
        let latest = self
            .call_failover(
                source_ids,
                &latest_call,
                call_ordinal,
                MethodClass::ReadLight,
            )
            .await?;

        let latest_str = latest.as_str().ok_or_else(|| {
            self.logs_chunking_invalid_range_error(
                "eth_blockNumber returned non-string during logs chunk planning",
                None,
            )
        })?;
        parse_hex_u64(latest_str).ok_or_else(|| {
            self.logs_chunking_invalid_range_error(
                "eth_blockNumber returned invalid hex during logs chunk planning",
                None,
            )
        })
    }

    async fn parse_logs_block_bound(
        &mut self,
        source_ids: &[String],
        call_ordinal: u64,
        value: &serde_json::Value,
    ) -> Result<Option<u64>, IoError> {
        if let Some(raw) = value.as_str() {
            let lowered = raw.to_ascii_lowercase();
            if lowered == "latest" {
                return self
                    .resolve_latest_block_number(source_ids, call_ordinal)
                    .await
                    .map(Some);
            }
            if lowered == "earliest" {
                return Ok(Some(0));
            }
            if lowered == "safe" || lowered == "finalized" || lowered == "pending" {
                // Preserve exact tag semantics by falling back to non-chunked execution.
                return Ok(None);
            }
            if let Some(parsed) = parse_hex_u64(raw) {
                return Ok(Some(parsed));
            }
            if let Ok(parsed) = raw.parse::<u64>() {
                return Ok(Some(parsed));
            }
            return Err(self.logs_chunking_invalid_range_error(
                "eth_getLogs block tag must be latest/earliest or numeric block number",
                Some(json!({ "value": raw })),
            ));
        }

        if let Some(parsed) = value.as_u64() {
            return Ok(Some(parsed));
        }

        Err(self.logs_chunking_invalid_range_error(
            "eth_getLogs block tag must be a string or integer",
            None,
        ))
    }

    fn initial_logs_ranges(&self, from_block: u64, to_block: u64) -> Vec<(u64, u64)> {
        let mut ranges = Vec::new();
        let mut cursor = from_block;
        while cursor <= to_block {
            let end = cursor
                .saturating_add(self.cfg.logs_max_block_span.saturating_sub(1))
                .min(to_block);
            ranges.push((cursor, end));
            if end == u64::MAX {
                break;
            }
            cursor = end.saturating_add(1);
        }
        ranges
    }

    async fn plan_logs_ranges(
        &mut self,
        source_ids: &[String],
        req: &JsonRpcCall,
        call_ordinal: u64,
    ) -> Result<Option<Vec<(u64, u64)>>, IoError> {
        if !req.method.eq_ignore_ascii_case("eth_getLogs") {
            return Ok(None);
        }

        let Some(params) = req.params.as_array() else {
            return Ok(None);
        };
        let Some(first) = params.first() else {
            return Ok(None);
        };
        let Some(filter) = first.as_object() else {
            return Ok(None);
        };
        if filter.get("blockHash").is_some() {
            return Ok(None);
        }

        let Some(from_raw) = filter.get("fromBlock") else {
            return Ok(None);
        };
        let Some(to_raw) = filter.get("toBlock") else {
            return Ok(None);
        };

        let Some(from_block) = self
            .parse_logs_block_bound(source_ids, call_ordinal, from_raw)
            .await?
        else {
            return Ok(None);
        };
        let Some(to_block) = self
            .parse_logs_block_bound(source_ids, call_ordinal, to_raw)
            .await?
        else {
            return Ok(None);
        };

        if from_block > to_block {
            return Err(self.logs_chunking_invalid_range_error(
                "eth_getLogs fromBlock must be <= toBlock",
                Some(json!({
                    "from_block": format!("0x{from_block:x}"),
                    "to_block": format!("0x{to_block:x}"),
                })),
            ));
        }

        Ok(Some(self.initial_logs_ranges(from_block, to_block)))
    }

    fn build_logs_chunk_call(
        &self,
        req: &JsonRpcCall,
        from_block: u64,
        to_block: u64,
    ) -> Result<JsonRpcCall, IoError> {
        let Some(mut params) = req.params.as_array().cloned() else {
            return Err(
                self.logs_chunking_invalid_range_error("eth_getLogs params must be an array", None)
            );
        };
        let Some(first) = params.first_mut() else {
            return Err(self.logs_chunking_invalid_range_error(
                "eth_getLogs params must include a filter object",
                None,
            ));
        };
        let Some(filter) = first.as_object_mut() else {
            return Err(self.logs_chunking_invalid_range_error(
                "eth_getLogs first param must be a filter object",
                None,
            ));
        };

        filter.insert("fromBlock".to_string(), json!(format!("0x{from_block:x}")));
        filter.insert("toBlock".to_string(), json!(format!("0x{to_block:x}")));

        Ok(JsonRpcCall::new(
            req.method.clone(),
            serde_json::Value::Array(params),
        ))
    }

    fn extend_logs_results(
        &self,
        out: &mut Vec<serde_json::Value>,
        value: serde_json::Value,
    ) -> Result<(), IoError> {
        let Some(items) = value.as_array() else {
            return Err(IoError::Other(info(
                CODE_EVM_JSONRPC_INVALID_RESPONSE,
                ErrorCategory::ParsingInput,
                false,
                "eth_getLogs response was not an array",
            )));
        };
        out.extend(items.iter().cloned());
        Ok(())
    }

    async fn call_logs_chunked_failover(
        &mut self,
        source_ids: &[String],
        req: &JsonRpcCall,
        call_ordinal: u64,
    ) -> Result<serde_json::Value, IoError> {
        let Some(initial_ranges) = self.plan_logs_ranges(source_ids, req, call_ordinal).await?
        else {
            return self
                .call_failover(source_ids, req, call_ordinal, MethodClass::ReadHeavy)
                .await;
        };

        let mut pending: VecDeque<(u64, u64)> = initial_ranges.into_iter().collect();
        let mut merged_logs = Vec::new();
        let mut attempted_chunks = 0_u64;
        let mut failures = Vec::new();

        while let Some((from_block, to_block)) = pending.pop_front() {
            if attempted_chunks >= self.cfg.logs_max_chunks_per_call {
                return Err(self.logs_chunking_exhausted_error(attempted_chunks, &failures));
            }
            attempted_chunks = attempted_chunks.saturating_add(1);

            let chunk_call = self.build_logs_chunk_call(req, from_block, to_block)?;
            match self
                .call_failover(
                    source_ids,
                    &chunk_call,
                    call_ordinal,
                    MethodClass::ReadHeavy,
                )
                .await
            {
                Ok(response) => {
                    self.extend_logs_results(&mut merged_logs, response)?;
                }
                Err(err) => {
                    failures.push(json!({
                        "from_block": format!("0x{from_block:x}"),
                        "to_block": format!("0x{to_block:x}"),
                        "code": err_info(&err).code.0,
                        "retryable": err_info(&err).retryable,
                    }));

                    let span = to_block.saturating_sub(from_block).saturating_add(1);
                    if is_retryable(&err) && span > self.cfg.logs_min_block_span {
                        let mid = from_block + (to_block - from_block) / 2;
                        if mid < to_block {
                            // Preserve deterministic log ordering by processing lower range first.
                            pending.push_front((mid.saturating_add(1), to_block));
                            pending.push_front((from_block, mid));
                            continue;
                        }
                    }

                    if is_retryable(&err) {
                        return Err(self.logs_chunking_exhausted_error(attempted_chunks, &failures));
                    }
                    return Err(err);
                }
            }
        }

        Ok(serde_json::Value::Array(merged_logs))
    }

    async fn ensure_source_probed(
        &mut self,
        source_id: &str,
        call_ordinal: u64,
    ) -> Result<(), IoError> {
        let should_probe = self
            .source_states
            .get(source_id)
            .map(|state| !state.probed)
            .unwrap_or(false);
        if !should_probe {
            return Ok(());
        }

        let source = self
            .source_states
            .get(source_id)
            .map(|state| state.source.clone())
            .ok_or_else(|| self.route_source_unknown_error(source_id))?;

        let probe_result = self.probe_source(&source).await;
        if let Some(state) = self.source_states.get_mut(source_id) {
            state.probed = true;
        }

        match probe_result {
            Ok(()) => {
                self.mark_source_success(source_id, MethodClass::ReadLight);
                Ok(())
            }
            Err(err) => {
                self.mark_source_failure_with_error(
                    source_id,
                    call_ordinal,
                    &err,
                    MethodClass::ReadHeavy,
                );
                Err(self.source_unhealthy_error(source_id))
            }
        }
    }

    async fn probe_source(&mut self, source: &EvmJsonRpcSource) -> Result<(), IoError> {
        let chain_id = JsonRpcCall::new("eth_chainId", json!([]));
        let block_number = JsonRpcCall::new("eth_blockNumber", json!([]));
        let eth_get_proof = JsonRpcCall::new(
            "eth_getProof",
            json!(["0x0000000000000000000000000000000000000000", [], "latest"]),
        );

        let chain_id_body = self.request_body(&chain_id);
        let block_number_body = self.request_body(&block_number);
        Self::execute_http_request(
            self.client.clone(),
            source.id.clone(),
            source.clone(),
            chain_id_body,
        )
        .await?;
        Self::execute_http_request(
            self.client.clone(),
            source.id.clone(),
            source.clone(),
            block_number_body,
        )
        .await?;
        if source.require_get_proof_probe {
            let proof_body = self.request_body(&eth_get_proof);
            Self::execute_http_request(
                self.client.clone(),
                source.id.clone(),
                source.clone(),
                proof_body,
            )
            .await?;
        }
        Ok(())
    }

    async fn call_source(
        &mut self,
        source_id: &str,
        req: &JsonRpcCall,
    ) -> Result<serde_json::Value, IoError> {
        let source = self
            .source_states
            .get(source_id)
            .map(|state| state.source.clone())
            .ok_or_else(|| self.route_source_unknown_error(source_id))?;
        let body = self.request_body(req);
        Self::execute_http_request(self.client.clone(), source_id.to_string(), source, body).await
    }

    fn prepare_call(
        &mut self,
        source_id: &str,
        req: &JsonRpcCall,
    ) -> Result<PreparedCall, IoError> {
        let source = self
            .source_states
            .get(source_id)
            .map(|state| state.source.clone())
            .ok_or_else(|| self.route_source_unknown_error(source_id))?;
        Ok(PreparedCall {
            source_id: source_id.to_string(),
            source,
            body: self.request_body(req),
        })
    }

    async fn call_failover(
        &mut self,
        source_ids: &[String],
        req: &JsonRpcCall,
        call_ordinal: u64,
        method_class: MethodClass,
    ) -> Result<serde_json::Value, IoError> {
        let mut failures = Vec::new();

        for source_id in source_ids {
            match self.ensure_source_probed(source_id, call_ordinal).await {
                Ok(()) => {}
                Err(err) => {
                    failures.push(failure_detail(source_id, &err));
                    if !is_retryable(&err) {
                        return Err(err);
                    }
                    continue;
                }
            }

            match self.call_source(source_id, req).await {
                Ok(result) => {
                    self.mark_source_success(source_id, method_class);
                    return Ok(result);
                }
                Err(err) => {
                    self.mark_source_failure_with_error(
                        source_id,
                        call_ordinal,
                        &err,
                        method_class,
                    );
                    failures.push(failure_detail(source_id, &err));
                    if !is_retryable(&err) {
                        return Err(err);
                    }
                }
            }
        }

        Err(self.no_healthy_source_error(&failures))
    }

    async fn call_write_primary(
        &mut self,
        source_ids: &[String],
        req: &JsonRpcCall,
        call_ordinal: u64,
    ) -> Result<serde_json::Value, IoError> {
        let Some(primary_id) = source_ids.first() else {
            return Err(self.no_healthy_source_error(&[]));
        };

        match self.call_source(primary_id, req).await {
            Ok(result) => {
                self.mark_source_success(primary_id, MethodClass::WriteOrSideEffect);
                Ok(result)
            }
            Err(err) => {
                self.mark_source_failure_with_error(
                    primary_id,
                    call_ordinal,
                    &err,
                    MethodClass::WriteOrSideEffect,
                );
                Err(err)
            }
        }
    }

    async fn call_hedged_light(
        &mut self,
        source_ids: &[String],
        req: &JsonRpcCall,
        call_ordinal: u64,
    ) -> Result<serde_json::Value, IoError> {
        if source_ids.len() < 2 {
            return self
                .call_failover(source_ids, req, call_ordinal, MethodClass::ReadLight)
                .await;
        }

        let primary_id = source_ids[0].clone();
        if let Err(err) = self.ensure_source_probed(&primary_id, call_ordinal).await {
            if !is_retryable(&err) {
                return Err(err);
            }
            return self
                .call_failover(&source_ids[1..], req, call_ordinal, MethodClass::ReadLight)
                .await;
        }

        let prepared_primary = self.prepare_call(&primary_id, req)?;
        let mut primary = Box::pin(Self::execute_http_request(
            self.client.clone(),
            prepared_primary.source_id.clone(),
            prepared_primary.source.clone(),
            prepared_primary.body,
        ));
        let early_primary = tokio::select! {
            res = &mut primary => Some(res),
            _ = tokio::time::sleep(self.cfg.hedge_delay) => None,
        };

        if let Some(primary_res) = early_primary {
            match primary_res {
                Ok(value) => {
                    self.mark_source_success(&primary_id, MethodClass::ReadLight);
                    return Ok(value);
                }
                Err(err) => {
                    self.mark_source_failure_with_error(
                        &primary_id,
                        call_ordinal,
                        &err,
                        MethodClass::ReadLight,
                    );
                    if !is_retryable(&err) {
                        return Err(err);
                    }
                    return self
                        .call_failover(&source_ids[1..], req, call_ordinal, MethodClass::ReadLight)
                        .await;
                }
            }
        }

        let secondary_id = source_ids[1].clone();
        if let Err(secondary_probe_err) =
            self.ensure_source_probed(&secondary_id, call_ordinal).await
        {
            if !is_retryable(&secondary_probe_err) {
                return Err(secondary_probe_err);
            }

            let mut failures = vec![failure_detail(&secondary_id, &secondary_probe_err)];
            let primary_res = primary.await;
            return match primary_res {
                Ok(value) => {
                    self.mark_source_success(&primary_id, MethodClass::ReadLight);
                    Ok(value)
                }
                Err(primary_err) => {
                    self.mark_source_failure_with_error(
                        &primary_id,
                        call_ordinal,
                        &primary_err,
                        MethodClass::ReadLight,
                    );
                    if !is_retryable(&primary_err) {
                        return Err(primary_err);
                    }
                    failures.push(failure_detail(&primary_id, &primary_err));
                    if source_ids.len() > 2 {
                        self.call_failover(
                            &source_ids[2..],
                            req,
                            call_ordinal,
                            MethodClass::ReadLight,
                        )
                        .await
                    } else {
                        Err(self.hedge_exhausted_error(&failures))
                    }
                }
            };
        }

        let prepared_secondary = self.prepare_call(&secondary_id, req)?;

        let mut secondary = Box::pin(Self::execute_http_request(
            self.client.clone(),
            prepared_secondary.source_id.clone(),
            prepared_secondary.source.clone(),
            prepared_secondary.body,
        ));

        enum FirstOutcome {
            Primary(Result<serde_json::Value, IoError>),
            Secondary(Result<serde_json::Value, IoError>),
        }

        let first = tokio::select! {
            res = &mut primary => FirstOutcome::Primary(res),
            res = &mut secondary => FirstOutcome::Secondary(res),
        };

        let mut failures = Vec::new();

        match first {
            FirstOutcome::Primary(Ok(value)) => {
                self.mark_source_success(&primary_id, MethodClass::ReadLight);
                return Ok(value);
            }
            FirstOutcome::Secondary(Ok(value)) => {
                self.mark_source_success(&secondary_id, MethodClass::ReadLight);
                return Ok(value);
            }
            FirstOutcome::Primary(Err(primary_err)) => {
                self.mark_source_failure_with_error(
                    &primary_id,
                    call_ordinal,
                    &primary_err,
                    MethodClass::ReadLight,
                );
                failures.push(failure_detail(&primary_id, &primary_err));

                let secondary_res = secondary.await;
                match secondary_res {
                    Ok(value) => {
                        self.mark_source_success(&secondary_id, MethodClass::ReadLight);
                        return Ok(value);
                    }
                    Err(secondary_err) => {
                        self.mark_source_failure_with_error(
                            &secondary_id,
                            call_ordinal,
                            &secondary_err,
                            MethodClass::ReadLight,
                        );
                        failures.push(failure_detail(&secondary_id, &secondary_err));
                    }
                }
            }
            FirstOutcome::Secondary(Err(secondary_err)) => {
                self.mark_source_failure_with_error(
                    &secondary_id,
                    call_ordinal,
                    &secondary_err,
                    MethodClass::ReadLight,
                );
                failures.push(failure_detail(&secondary_id, &secondary_err));

                let primary_res = primary.await;
                match primary_res {
                    Ok(value) => {
                        self.mark_source_success(&primary_id, MethodClass::ReadLight);
                        return Ok(value);
                    }
                    Err(primary_err) => {
                        self.mark_source_failure_with_error(
                            &primary_id,
                            call_ordinal,
                            &primary_err,
                            MethodClass::ReadLight,
                        );
                        failures.push(failure_detail(&primary_id, &primary_err));
                    }
                }
            }
        }

        if source_ids.len() > 2 {
            return self
                .call_failover(&source_ids[2..], req, call_ordinal, MethodClass::ReadLight)
                .await;
        }

        Err(self.hedge_exhausted_error(&failures))
    }

    async fn execute_http_request(
        client: reqwest::Client,
        source_id: String,
        source: EvmJsonRpcSource,
        body: serde_json::Value,
    ) -> Result<serde_json::Value, IoError> {
        let rpc_method = body
            .get("method")
            .and_then(|v| v.as_str())
            .unwrap_or("<unknown>")
            .to_string();
        let rpc_request_id = body.get("id").and_then(|v| v.as_u64());
        let source_kind = source_kind_name(source.kind);
        let rpc_endpoint = rpc_endpoint_name(&source.rpc_url);
        debug!(
            source_id = %source_id,
            source_kind = source_kind,
            rpc_endpoint = %rpc_endpoint,
            rpc_method = %rpc_method,
            rpc_request_id = ?rpc_request_id,
            "dispatching evm jsonrpc http request"
        );

        let mut rb = client.post(source.rpc_url).json(&body);
        if let Some(auth) = source.authorization {
            rb = rb.header(reqwest::header::AUTHORIZATION, auth);
        }

        let resp = rb.send().await.map_err(|err| {
            let err_class = if err.is_timeout() {
                "timeout"
            } else {
                "transport"
            };
            debug!(
                source_id = %source_id,
                source_kind = source_kind,
                rpc_endpoint = %rpc_endpoint,
                rpc_method = %rpc_method,
                rpc_request_id = ?rpc_request_id,
                transport_error_class = err_class,
                "evm jsonrpc http request failed before response"
            );
            IoError::Transport(info_with_details(
                CODE_EVM_HTTP_REQUEST_FAILED,
                ErrorCategory::Rpc,
                true,
                "evm http request failed",
                source_error_details(
                    &source_id,
                    Some(json!({ "transport_error_class": err_class })),
                ),
            ))
        })?;

        let status = resp.status();
        if status.as_u16() == 429 {
            debug!(
                source_id = %source_id,
                source_kind = source_kind,
                rpc_endpoint = %rpc_endpoint,
                rpc_method = %rpc_method,
                rpc_request_id = ?rpc_request_id,
                http_status = status.as_u16(),
                "evm jsonrpc request was rate-limited"
            );
            return Err(IoError::RateLimited(info_with_details(
                CODE_EVM_RATE_LIMITED,
                ErrorCategory::Rpc,
                true,
                "evm rpc rate limited",
                source_error_details(
                    &source_id,
                    Some(json!({
                        "http_status": status.as_u16(),
                        "http_status_class": status.as_u16() / 100,
                    })),
                ),
            )));
        }
        if !status.is_success() {
            debug!(
                source_id = %source_id,
                source_kind = source_kind,
                rpc_endpoint = %rpc_endpoint,
                rpc_method = %rpc_method,
                rpc_request_id = ?rpc_request_id,
                http_status = status.as_u16(),
                "evm jsonrpc request returned non-success status"
            );
            return Err(IoError::Transport(info_with_details(
                CODE_EVM_HTTP_STATUS,
                ErrorCategory::Rpc,
                true,
                "evm rpc returned non-success http status",
                source_error_details(
                    &source_id,
                    Some(json!({
                        "http_status": status.as_u16(),
                        "http_status_class": status.as_u16() / 100,
                    })),
                ),
            )));
        }

        let bytes = resp.bytes().await.map_err(|err| {
            debug!(
                source_id = %source_id,
                source_kind = source_kind,
                rpc_endpoint = %rpc_endpoint,
                rpc_method = %rpc_method,
                rpc_request_id = ?rpc_request_id,
                error = %err,
                "evm jsonrpc response body read failed"
            );
            IoError::Transport(info_with_details(
                CODE_EVM_HTTP_BODY_READ_FAILED,
                ErrorCategory::Rpc,
                true,
                "failed to read evm rpc response body",
                source_error_details(&source_id, None),
            ))
        })?;

        let v = serde_json::from_slice::<serde_json::Value>(&bytes).map_err(|_| {
            debug!(
                source_id = %source_id,
                source_kind = source_kind,
                rpc_endpoint = %rpc_endpoint,
                rpc_method = %rpc_method,
                rpc_request_id = ?rpc_request_id,
                "evm jsonrpc response was not valid json"
            );
            IoError::Other(info_with_details(
                CODE_EVM_RESPONSE_INVALID_JSON,
                ErrorCategory::ParsingInput,
                false,
                "evm rpc response was not valid json",
                source_error_details(&source_id, None),
            ))
        })?;

        let obj = v.as_object().ok_or_else(|| {
            debug!(
                source_id = %source_id,
                source_kind = source_kind,
                rpc_endpoint = %rpc_endpoint,
                rpc_method = %rpc_method,
                rpc_request_id = ?rpc_request_id,
                "evm jsonrpc response object was invalid"
            );
            IoError::Other(info_with_details(
                CODE_EVM_JSONRPC_INVALID_RESPONSE,
                ErrorCategory::ParsingInput,
                false,
                "evm jsonrpc response was not an object",
                source_error_details(&source_id, None),
            ))
        })?;

        if let Some(error_value) = obj.get("error") {
            let jsonrpc_error_code = error_value.get("code").and_then(|v| v.as_i64());
            debug!(
                source_id = %source_id,
                source_kind = source_kind,
                rpc_endpoint = %rpc_endpoint,
                rpc_method = %rpc_method,
                rpc_request_id = ?rpc_request_id,
                jsonrpc_error_code = ?jsonrpc_error_code,
                "evm jsonrpc returned an error object"
            );
            return Err(IoError::Transport(info_with_details(
                CODE_EVM_JSONRPC_ERROR,
                ErrorCategory::Rpc,
                true,
                "evm jsonrpc returned an error",
                jsonrpc_error_details(&source_id, error_value),
            )));
        }

        let Some(result) = obj.get("result") else {
            debug!(
                source_id = %source_id,
                source_kind = source_kind,
                rpc_endpoint = %rpc_endpoint,
                rpc_method = %rpc_method,
                rpc_request_id = ?rpc_request_id,
                "evm jsonrpc response missing result field"
            );
            return Err(IoError::Other(info_with_details(
                CODE_EVM_JSONRPC_MISSING_RESULT,
                ErrorCategory::ParsingInput,
                false,
                "evm jsonrpc response missing result",
                source_error_details(&source_id, None),
            )));
        };

        debug!(
            source_id = %source_id,
            source_kind = source_kind,
            rpc_endpoint = %rpc_endpoint,
            rpc_method = %rpc_method,
            rpc_request_id = ?rpc_request_id,
            http_status = status.as_u16(),
            "evm jsonrpc http request succeeded"
        );
        Ok(result.clone())
    }
}

#[async_trait]
impl LiveIoTransport for EvmJsonRpcHttpTransport {
    async fn call(&mut self, call: IoCall) -> Result<serde_json::Value, IoError> {
        if let Some(info) = self.config_error_info() {
            return Err(IoError::Other(info));
        }

        let req: EvmTransportRequest =
            serde_json::from_value(call.request).map_err(|_| self.invalid_request_error())?;

        if req.method.eq_ignore_ascii_case(METHOD_EVM_ROUTING_ANALYSIS) {
            return self.call_routing_analysis_operation(&req);
        }

        if req.rpc_url.is_some() {
            return Err(self.rpc_url_override_error());
        }

        let method_policy = resolve_method_policy(&req.method, &req.params, &self.cfg);
        let call_ordinal = self.current_call_ordinal();
        let json_call = JsonRpcCall::new(req.method.clone(), req.params.clone());

        let route_source_id = req.route.as_ref().map(|route| route.source_id.as_str());
        let source_ids = self.ordered_source_ids(route_source_id, call_ordinal)?;
        debug!(
            call_ordinal = call_ordinal,
            rpc_method = %req.method,
            method_class = method_class_name(method_policy.class),
            dispatch_mode = dispatch_mode_name(method_policy.dispatch),
            chunk_logs = method_policy.chunk_logs,
            route_source_id = ?route_source_id,
            source_order = ?source_ids,
            "resolved evm jsonrpc routing decision"
        );

        let result = if method_policy.chunk_logs {
            self.call_logs_chunked_failover(&source_ids, &json_call, call_ordinal)
                .await
        } else {
            match method_policy.dispatch {
                DispatchMode::PrimaryOnly => {
                    self.call_write_primary(&source_ids, &json_call, call_ordinal)
                        .await
                }
                DispatchMode::Failover => {
                    self.call_failover(&source_ids, &json_call, call_ordinal, method_policy.class)
                        .await
                }
                DispatchMode::HedgedLight => {
                    self.call_hedged_light(&source_ids, &json_call, call_ordinal)
                        .await
                }
            }
        };

        match &result {
            Ok(_) => {
                debug!(
                    call_ordinal = call_ordinal,
                    rpc_method = %req.method,
                    "evm jsonrpc call completed"
                );
            }
            Err(err) => {
                let info = err_info(err);
                debug!(
                    call_ordinal = call_ordinal,
                    rpc_method = %req.method,
                    error_code = %info.code.0,
                    retryable = info.retryable,
                    "evm jsonrpc call failed"
                );
            }
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use async_trait::async_trait;
    use mfm_machine::engine::Stores;
    use mfm_machine::errors::StorageError;
    use mfm_machine::events::EventEnvelope;
    use mfm_machine::ids::{ArtifactId, RunId, StateId};
    use mfm_machine::live_io::LiveIoEnv;
    use mfm_machine::stores::{ArtifactKind, ArtifactStore, EventStore};
    use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
    use std::sync::Arc;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};
    use tokio::sync::oneshot;

    #[derive(Clone)]
    struct NoopEventStore;

    #[async_trait]
    impl EventStore for NoopEventStore {
        async fn head_seq(&self, _run_id: RunId) -> Result<u64, StorageError> {
            Ok(0)
        }

        async fn append(
            &self,
            _run_id: RunId,
            _expected_seq: u64,
            _events: Vec<EventEnvelope>,
        ) -> Result<u64, StorageError> {
            Ok(0)
        }

        async fn read_range(
            &self,
            _run_id: RunId,
            _from_seq: u64,
            _to_seq: Option<u64>,
        ) -> Result<Vec<EventEnvelope>, StorageError> {
            Ok(Vec::new())
        }
    }

    #[derive(Clone)]
    struct NoopArtifactStore;

    #[async_trait]
    impl ArtifactStore for NoopArtifactStore {
        async fn put(
            &self,
            _kind: ArtifactKind,
            _bytes: Vec<u8>,
        ) -> Result<ArtifactId, StorageError> {
            Ok(ArtifactId("0".repeat(64)))
        }

        async fn get(&self, _id: &ArtifactId) -> Result<Vec<u8>, StorageError> {
            Ok(Vec::new())
        }

        async fn exists(&self, _id: &ArtifactId) -> Result<bool, StorageError> {
            Ok(false)
        }
    }

    fn env() -> LiveIoEnv {
        LiveIoEnv {
            stores: Stores {
                events: Arc::new(NoopEventStore),
                artifacts: Arc::new(NoopArtifactStore),
            },
            run_id: serde_json::from_str::<RunId>("\"00000000-0000-0000-0000-000000000000\"")
                .expect("valid RunId"),
            state_id: StateId::must_new("machine.main.s1".to_string()),
            attempt: 0,
        }
    }

    fn source(id: &str, url: &str) -> EvmJsonRpcSource {
        EvmJsonRpcSource {
            id: id.to_string(),
            rpc_url: url.to_string(),
            authorization: None,
            kind: EvmSourceKind::RemotePublic,
            require_get_proof_probe: false,
        }
    }

    fn config_with_sources(
        sources: Vec<EvmJsonRpcSource>,
        strategy: EvmRoutingStrategy,
    ) -> EvmJsonRpcHttpConfig {
        let preferred_order = sources.iter().map(|s| s.id.clone()).collect::<Vec<_>>();
        EvmJsonRpcHttpConfig {
            sources,
            preferred_order,
            strategy,
            hedge_delay: Duration::from_millis(30),
            timeout: Duration::from_millis(800),
            unhealthy_cooldown_calls: 2,
            hedge_max_eth_call_params_bytes: 1024,
            logs_max_block_span: 128,
            logs_min_block_span: 8,
            logs_max_chunks_per_call: 128,
        }
    }

    async fn call_transport(
        transport: &mut dyn LiveIoTransport,
        request: serde_json::Value,
    ) -> Result<serde_json::Value, IoError> {
        transport
            .call(IoCall {
                namespace: "evm".to_string(),
                request,
                fact_key: None,
            })
            .await
    }

    #[derive(Clone)]
    enum StubBehavior {
        JsonResult(serde_json::Value),
        DelayJsonResult {
            delay: Duration,
            result: serde_json::Value,
        },
        HttpStatus(u16),
        LogsRangeGate {
            max_ok_span: u64,
            fail_status: u16,
        },
    }

    struct StubServer {
        url: String,
        hits: Arc<AtomicUsize>,
        shutdown: Option<oneshot::Sender<()>>,
    }

    impl StubServer {
        fn hit_count(&self) -> usize {
            self.hits.load(AtomicOrdering::SeqCst)
        }
    }

    impl Drop for StubServer {
        fn drop(&mut self) {
            if let Some(tx) = self.shutdown.take() {
                let _ = tx.send(());
            }
        }
    }

    async fn start_stub_server(behavior: StubBehavior) -> StubServer {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test server");
        let addr = listener.local_addr().expect("local addr");
        let hits = Arc::new(AtomicUsize::new(0));
        let (shutdown_tx, mut shutdown_rx) = oneshot::channel::<()>();
        let hits_for_loop = Arc::clone(&hits);

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
                        let behavior = behavior.clone();
                        let hits = Arc::clone(&hits_for_loop);
                        tokio::spawn(async move {
                            let _ = handle_stub_connection(stream, behavior, hits).await;
                        });
                    }
                }
            }
        });

        StubServer {
            url: format!("http://127.0.0.1:{}/", addr.port()),
            hits,
            shutdown: Some(shutdown_tx),
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

    fn reason_phrase(status: u16) -> &'static str {
        match status {
            200 => "OK",
            429 => "Too Many Requests",
            500 => "Internal Server Error",
            _ => "Status",
        }
    }

    fn parse_request_json(raw_http_request: &[u8]) -> Option<serde_json::Value> {
        let header_end = header_end(raw_http_request)?;
        let body = raw_http_request.get((header_end + 4)..)?;
        serde_json::from_slice(body).ok()
    }

    fn parse_logs_range_span(request: &serde_json::Value) -> Option<u64> {
        let method = request.get("method")?.as_str()?;
        if !method.eq_ignore_ascii_case("eth_getLogs") {
            return None;
        }

        let filter = request.get("params")?.as_array()?.first()?.as_object()?;

        let parse_bound = |value: &serde_json::Value| -> Option<u64> {
            if let Some(raw) = value.as_str() {
                if raw.eq_ignore_ascii_case("latest") {
                    return None;
                }
                return parse_hex_u64(raw).or_else(|| raw.parse::<u64>().ok());
            }
            value.as_u64()
        };

        let from = parse_bound(filter.get("fromBlock")?)?;
        let to = parse_bound(filter.get("toBlock")?)?;
        if from > to {
            return None;
        }
        Some(to.saturating_sub(from).saturating_add(1))
    }

    fn parse_logs_from_block(request: &serde_json::Value) -> String {
        request
            .get("params")
            .and_then(|v| v.as_array())
            .and_then(|v| v.first())
            .and_then(|v| v.get("fromBlock"))
            .and_then(|v| v.as_str())
            .map(ToString::to_string)
            .unwrap_or_else(|| "0x0".to_string())
    }

    async fn handle_stub_connection(
        mut stream: TcpStream,
        behavior: StubBehavior,
        hits: Arc<AtomicUsize>,
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

        hits.fetch_add(1, AtomicOrdering::SeqCst);

        if let StubBehavior::DelayJsonResult { delay, .. } = &behavior {
            tokio::time::sleep(*delay).await;
        }

        let request_json = parse_request_json(&buf);
        let request_method = request_json
            .as_ref()
            .and_then(|v| v.get("method"))
            .and_then(|v| v.as_str())
            .map(str::to_ascii_lowercase);

        let (status, body) = match behavior {
            StubBehavior::JsonResult(result) => {
                let body = json!({
                    "jsonrpc": "2.0",
                    "id": 1,
                    "result": result,
                })
                .to_string();
                (200, body)
            }
            StubBehavior::DelayJsonResult { result, .. } => {
                let body = json!({
                    "jsonrpc": "2.0",
                    "id": 1,
                    "result": result,
                })
                .to_string();
                (200, body)
            }
            StubBehavior::HttpStatus(code) => {
                let body = json!({
                    "error": "stubbed status",
                })
                .to_string();
                (code, body)
            }
            StubBehavior::LogsRangeGate {
                max_ok_span,
                fail_status,
            } => {
                let span = request_json.as_ref().and_then(parse_logs_range_span);
                if span.is_some_and(|value| value > max_ok_span) {
                    let body = json!({
                        "error": "range too large",
                    })
                    .to_string();
                    (fail_status, body)
                } else {
                    let result = match request_method.as_deref() {
                        Some("eth_getlogs") => json!([{
                            "blockNumber": request_json
                                .as_ref()
                                .map(parse_logs_from_block)
                                .unwrap_or_else(|| "0x0".to_string()),
                            "logIndex": "0x0",
                            "transactionHash": "0x1111111111111111111111111111111111111111111111111111111111111111",
                        }]),
                        Some("eth_blocknumber") => json!("0x100"),
                        Some("eth_chainid") => json!("0x1"),
                        _ => json!("0x1"),
                    };
                    let body = json!({
                        "jsonrpc": "2.0",
                        "id": 1,
                        "result": result,
                    })
                    .to_string();
                    (200, body)
                }
            }
        };

        let status_line = format!("HTTP/1.1 {} {}\r\n", status, reason_phrase(status));
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

    #[test]
    fn transport_creation_fails_fast_when_source_registry_is_empty() {
        let cfg = EvmJsonRpcHttpConfig {
            sources: Vec::new(),
            ..EvmJsonRpcHttpConfig::default()
        };
        let result = EvmJsonRpcHttpTransportFactory::try_new(cfg);
        assert!(matches!(result, Err(EvmJsonRpcHttpConfigError::NoSources)));
    }

    #[test]
    fn transport_creation_accepts_multiple_sources() {
        let cfg = config_with_sources(
            vec![
                source("primary", "http://127.0.0.1:8545"),
                source("secondary", "http://127.0.0.1:9545"),
            ],
            EvmRoutingStrategy::Failover,
        );
        EvmJsonRpcHttpTransportFactory::try_new(cfg).expect("valid config");
    }

    #[test]
    fn rpc_endpoint_name_sanitizes_to_scheme_host_port() {
        assert_eq!(
            rpc_endpoint_name("https://user:pass@example.com:8545/path?token=secret#frag"),
            "https://example.com:8545"
        );
        assert_eq!(rpc_endpoint_name("http://127.0.0.1"), "http://127.0.0.1:80");
        assert_eq!(
            rpc_endpoint_name("https://[2001:db8::1]:8545/rpc"),
            "https://[2001:db8::1]:8545"
        );
        assert_eq!(rpc_endpoint_name("not-a-url"), "<unavailable>");
    }

    #[test]
    fn classifier_write_methods_are_write_or_side_effect() {
        let cfg = EvmJsonRpcHttpConfig::default();
        assert_eq!(
            classify_method("eth_sendRawTransaction", &json!(["0x01"]), &cfg),
            MethodClass::WriteOrSideEffect
        );
        assert_eq!(
            classify_method("admin_nodeInfo", &json!([]), &cfg),
            MethodClass::WriteOrSideEffect
        );
    }

    #[test]
    fn classifier_covers_representative_read_classes() {
        let cfg = EvmJsonRpcHttpConfig::default();
        assert_eq!(
            classify_method("eth_chainId", &json!([]), &cfg),
            MethodClass::ReadLight
        );
        assert_eq!(
            classify_method("eth_call", &json!([{"to":"0x1","data":"0x"}]), &cfg),
            MethodClass::ReadLight
        );
        assert_eq!(
            classify_method("eth_getLogs", &json!([]), &cfg),
            MethodClass::ReadHeavy
        );
        assert_eq!(
            classify_method("trace_block", &json!(["0x1"]), &cfg),
            MethodClass::ReadHeavy
        );
    }

    #[test]
    fn method_policy_enforces_no_hedge_for_logs_and_writes() {
        let cfg = EvmJsonRpcHttpConfig {
            strategy: EvmRoutingStrategy::HedgedLight,
            ..EvmJsonRpcHttpConfig::default()
        };

        let logs = resolve_method_policy("eth_getLogs", &json!([{}]), &cfg);
        assert_eq!(logs.class, MethodClass::ReadHeavy);
        assert_eq!(logs.dispatch, DispatchMode::Failover);
        assert!(logs.chunk_logs);

        let write = resolve_method_policy("eth_sendRawTransaction", &json!(["0x01"]), &cfg);
        assert_eq!(write.class, MethodClass::WriteOrSideEffect);
        assert_eq!(write.dispatch, DispatchMode::PrimaryOnly);
        assert!(!write.chunk_logs);
    }

    #[test]
    fn weighted_failure_penalty_prioritizes_rate_limits_and_writes() {
        let timeout = IoError::Transport(info_with_details(
            CODE_EVM_HTTP_REQUEST_FAILED,
            ErrorCategory::Rpc,
            true,
            "timeout",
            Some(json!({"transport_error_class": "timeout"})),
        ));
        let rate_limit = IoError::RateLimited(info_with_details(
            CODE_EVM_RATE_LIMITED,
            ErrorCategory::Rpc,
            true,
            "rate limited",
            Some(json!({"http_status": 429})),
        ));

        let timeout_read =
            EvmJsonRpcHttpTransport::score_failure_penalty(&timeout, MethodClass::ReadLight);
        let rate_read =
            EvmJsonRpcHttpTransport::score_failure_penalty(&rate_limit, MethodClass::ReadLight);
        let rate_write = EvmJsonRpcHttpTransport::score_failure_penalty(
            &rate_limit,
            MethodClass::WriteOrSideEffect,
        );

        assert!(rate_read > timeout_read);
        assert!(rate_write > rate_read);
    }

    #[test]
    fn transport_creation_rejects_invalid_logs_chunking_config() {
        let mut cfg = config_with_sources(
            vec![source("primary", "http://127.0.0.1:8545")],
            EvmRoutingStrategy::Failover,
        );
        cfg.logs_min_block_span = 64;
        cfg.logs_max_block_span = 32;
        let result = EvmJsonRpcHttpTransportFactory::try_new(cfg);
        assert!(matches!(
            result,
            Err(EvmJsonRpcHttpConfigError::InvalidLogsChunkingRange)
        ));
    }

    #[tokio::test]
    async fn routing_analysis_operation_returns_order_without_network_calls() {
        let primary = start_stub_server(StubBehavior::JsonResult(json!("0x1"))).await;
        let secondary = start_stub_server(StubBehavior::JsonResult(json!("0x2"))).await;
        let cfg = config_with_sources(
            vec![
                source("primary", &primary.url),
                source("secondary", &secondary.url),
            ],
            EvmRoutingStrategy::HedgedLight,
        );
        let factory = EvmJsonRpcHttpTransportFactory::new(cfg);
        let mut t = factory.make(env());

        let response = call_transport(
            t.as_mut(),
            json!({
                "method": METHOD_EVM_ROUTING_ANALYSIS,
                "params": {
                    "method": "eth_chainId",
                    "params": [],
                },
            }),
        )
        .await
        .expect("analysis operation should succeed");

        assert_eq!(
            response.get("operation"),
            Some(&json!(METHOD_EVM_ROUTING_ANALYSIS))
        );
        assert_eq!(
            response.pointer("/target/method"),
            Some(&json!("eth_chainId"))
        );
        assert_eq!(
            response.pointer("/routing/method_class"),
            Some(&json!("read_light"))
        );
        assert_eq!(
            response.pointer("/routing/dispatch_mode"),
            Some(&json!("hedged_light"))
        );
        assert_eq!(
            response.pointer("/routing/selected_order"),
            Some(&json!(["primary", "secondary"]))
        );

        let ranked = response
            .get("sources_ranked")
            .and_then(|v| v.as_array())
            .expect("sources_ranked array");
        assert_eq!(ranked.len(), 2);
        assert_eq!(ranked[0].get("source_id"), Some(&json!("primary")));
        assert_eq!(ranked[1].get("source_id"), Some(&json!("secondary")));

        // Analysis is local-only and does not issue network requests.
        assert_eq!(primary.hit_count(), 0);
        assert_eq!(secondary.hit_count(), 0);
    }

    #[tokio::test]
    async fn routing_analysis_operation_reports_unknown_route_without_failing() {
        let primary = start_stub_server(StubBehavior::JsonResult(json!("0x1"))).await;
        let cfg = config_with_sources(
            vec![source("primary", &primary.url)],
            EvmRoutingStrategy::Failover,
        );
        let factory = EvmJsonRpcHttpTransportFactory::new(cfg);
        let mut t = factory.make(env());

        let response = call_transport(
            t.as_mut(),
            json!({
                "method": METHOD_EVM_ROUTING_ANALYSIS,
                "params": {
                    "method": "eth_getLogs",
                    "params": [],
                    "route": {"source_id": "missing"},
                },
            }),
        )
        .await
        .expect("analysis operation should return error details in response");

        assert_eq!(
            response.pointer("/routing/selected_order"),
            Some(&json!([]))
        );
        assert_eq!(
            response.pointer("/routing/selection_error/code"),
            Some(&json!(CODE_EVM_ROUTE_SOURCE_UNKNOWN))
        );
        assert_eq!(primary.hit_count(), 0);
    }

    #[tokio::test]
    async fn failover_uses_secondary_on_primary_http_failure() {
        let primary = start_stub_server(StubBehavior::HttpStatus(500)).await;
        let secondary = start_stub_server(StubBehavior::JsonResult(json!("0x2"))).await;

        let cfg = config_with_sources(
            vec![
                source("primary", &primary.url),
                source("secondary", &secondary.url),
            ],
            EvmRoutingStrategy::Failover,
        );
        let factory = EvmJsonRpcHttpTransportFactory::new(cfg);
        let mut t = factory.make(env());

        let response = call_transport(
            t.as_mut(),
            json!({
                "method": "eth_getLogs",
                "params": [],
            }),
        )
        .await
        .expect("secondary should succeed");

        assert_eq!(response, json!("0x2"));
        assert_eq!(primary.hit_count(), 1); // probe failure marks source unhealthy
        assert_eq!(secondary.hit_count(), 3); // probes + call
    }

    #[tokio::test]
    async fn logs_chunking_splits_large_ranges_and_merges_results() {
        let primary = start_stub_server(StubBehavior::LogsRangeGate {
            max_ok_span: 32,
            fail_status: 429,
        })
        .await;
        let secondary = start_stub_server(StubBehavior::LogsRangeGate {
            max_ok_span: 32,
            fail_status: 429,
        })
        .await;

        let mut cfg = config_with_sources(
            vec![
                source("primary", &primary.url),
                source("secondary", &secondary.url),
            ],
            EvmRoutingStrategy::HedgedLight,
        );
        cfg.logs_max_block_span = 64;
        cfg.logs_min_block_span = 8;
        cfg.logs_max_chunks_per_call = 64;
        let factory = EvmJsonRpcHttpTransportFactory::new(cfg);
        let mut t = factory.make(env());

        let response = call_transport(
            t.as_mut(),
            json!({
                "method": "eth_getLogs",
                "params": [{
                    "address": "0x0000000000000000000000000000000000000000",
                    "fromBlock": "0x1",
                    "toBlock": "0x80",
                    "topics": [],
                }],
            }),
        )
        .await
        .expect("logs chunking should succeed");

        let logs = response.as_array().expect("array response");
        let from_blocks = logs
            .iter()
            .filter_map(|v| v.get("blockNumber"))
            .filter_map(|v| v.as_str())
            .map(str::to_string)
            .collect::<Vec<_>>();
        assert_eq!(
            from_blocks,
            vec![
                "0x1".to_string(),
                "0x21".to_string(),
                "0x41".to_string(),
                "0x61".to_string()
            ]
        );
    }

    #[tokio::test]
    async fn logs_chunking_exhausts_when_retryable_failures_persist() {
        let primary = start_stub_server(StubBehavior::LogsRangeGate {
            max_ok_span: 0,
            fail_status: 429,
        })
        .await;
        let secondary = start_stub_server(StubBehavior::LogsRangeGate {
            max_ok_span: 0,
            fail_status: 429,
        })
        .await;

        let mut cfg = config_with_sources(
            vec![
                source("primary", &primary.url),
                source("secondary", &secondary.url),
            ],
            EvmRoutingStrategy::Failover,
        );
        cfg.logs_max_block_span = 16;
        cfg.logs_min_block_span = 8;
        cfg.logs_max_chunks_per_call = 10;
        let factory = EvmJsonRpcHttpTransportFactory::new(cfg);
        let mut t = factory.make(env());

        let err = call_transport(
            t.as_mut(),
            json!({
                "method": "eth_getLogs",
                "params": [{
                    "address": "0x0000000000000000000000000000000000000000",
                    "fromBlock": "0x1",
                    "toBlock": "0x40",
                    "topics": [],
                }],
            }),
        )
        .await
        .expect_err("chunking should eventually exhaust");

        match err {
            IoError::Transport(info) => assert_eq!(info.code.0, CODE_EVM_LOGS_CHUNKING_EXHAUSTED),
            other => panic!("expected Transport, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn failover_uses_secondary_on_primary_rate_limit() {
        let primary = start_stub_server(StubBehavior::HttpStatus(429)).await;
        let secondary = start_stub_server(StubBehavior::JsonResult(json!("0x2"))).await;

        let cfg = config_with_sources(
            vec![
                source("primary", &primary.url),
                source("secondary", &secondary.url),
            ],
            EvmRoutingStrategy::Failover,
        );
        let factory = EvmJsonRpcHttpTransportFactory::new(cfg);
        let mut t = factory.make(env());

        let response = call_transport(
            t.as_mut(),
            json!({
                "method": "eth_getLogs",
                "params": [],
            }),
        )
        .await
        .expect("secondary should succeed");

        assert_eq!(response, json!("0x2"));
    }

    #[tokio::test]
    async fn failover_returns_stable_pool_error_when_all_sources_fail() {
        let primary = start_stub_server(StubBehavior::HttpStatus(500)).await;
        let secondary = start_stub_server(StubBehavior::HttpStatus(429)).await;

        let cfg = config_with_sources(
            vec![
                source("primary", &primary.url),
                source("secondary", &secondary.url),
            ],
            EvmRoutingStrategy::Failover,
        );
        let factory = EvmJsonRpcHttpTransportFactory::new(cfg);
        let mut t = factory.make(env());

        let err = call_transport(
            t.as_mut(),
            json!({
                "method": "eth_getLogs",
                "params": [],
            }),
        )
        .await
        .expect_err("all candidates should fail");

        match err {
            IoError::Transport(info) => {
                assert_eq!(info.code.0, CODE_EVM_NO_HEALTHY_SOURCE);
                assert!(info.retryable);
            }
            other => panic!("expected Transport, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn write_methods_use_primary_only_single_dispatch() {
        let primary = start_stub_server(StubBehavior::HttpStatus(500)).await;
        let secondary = start_stub_server(StubBehavior::JsonResult(json!("0xdead"))).await;

        let cfg = config_with_sources(
            vec![
                source("primary", &primary.url),
                source("secondary", &secondary.url),
            ],
            EvmRoutingStrategy::HedgedLight,
        );
        let factory = EvmJsonRpcHttpTransportFactory::new(cfg);
        let mut t = factory.make(env());

        let err = call_transport(
            t.as_mut(),
            json!({
                "method": "eth_sendRawTransaction",
                "params": ["0x01"],
            }),
        )
        .await
        .expect_err("primary should fail");

        match err {
            IoError::Transport(info) => assert_eq!(info.code.0, CODE_EVM_HTTP_STATUS),
            other => panic!("expected Transport, got {other:?}"),
        }
        assert_eq!(secondary.hit_count(), 0);
    }

    #[tokio::test]
    async fn hedging_returns_secondary_winner_when_primary_is_slow() {
        let primary = start_stub_server(StubBehavior::DelayJsonResult {
            delay: Duration::from_millis(180),
            result: json!("0x1"),
        })
        .await;
        let secondary = start_stub_server(StubBehavior::JsonResult(json!("0x2"))).await;

        let mut cfg = config_with_sources(
            vec![
                source("primary", &primary.url),
                source("secondary", &secondary.url),
            ],
            EvmRoutingStrategy::HedgedLight,
        );
        cfg.hedge_delay = Duration::from_millis(20);
        let factory = EvmJsonRpcHttpTransportFactory::new(cfg);
        let mut t = factory.make(env());

        let response = call_transport(
            t.as_mut(),
            json!({
                "method": "eth_chainId",
                "params": [],
            }),
        )
        .await
        .expect("hedged call should succeed");

        assert_eq!(response, json!("0x2"));
        assert_eq!(secondary.hit_count(), 3);
    }

    #[tokio::test]
    async fn hedging_falls_back_when_primary_probe_fails() {
        let primary = start_stub_server(StubBehavior::HttpStatus(500)).await;
        let secondary = start_stub_server(StubBehavior::JsonResult(json!("0x2"))).await;

        let cfg = config_with_sources(
            vec![
                source("primary", &primary.url),
                source("secondary", &secondary.url),
            ],
            EvmRoutingStrategy::HedgedLight,
        );
        let factory = EvmJsonRpcHttpTransportFactory::new(cfg);
        let mut t = factory.make(env());

        let response = call_transport(
            t.as_mut(),
            json!({
                "method": "eth_chainId",
                "params": [],
            }),
        )
        .await
        .expect("secondary should be used when primary probe fails");

        assert_eq!(response, json!("0x2"));
        assert_eq!(primary.hit_count(), 1);
        assert_eq!(secondary.hit_count(), 3);
    }

    #[tokio::test]
    async fn hedging_does_not_start_secondary_when_primary_finishes_before_delay() {
        let primary = start_stub_server(StubBehavior::JsonResult(json!("0x1"))).await;
        let secondary = start_stub_server(StubBehavior::JsonResult(json!("0x2"))).await;

        let mut cfg = config_with_sources(
            vec![
                source("primary", &primary.url),
                source("secondary", &secondary.url),
            ],
            EvmRoutingStrategy::HedgedLight,
        );
        cfg.hedge_delay = Duration::from_millis(250);
        let factory = EvmJsonRpcHttpTransportFactory::new(cfg);
        let mut t = factory.make(env());

        let response = call_transport(
            t.as_mut(),
            json!({
                "method": "eth_chainId",
                "params": [],
            }),
        )
        .await
        .expect("primary should win before hedge delay");

        assert_eq!(response, json!("0x1"));
        assert_eq!(secondary.hit_count(), 0);
    }

    #[tokio::test]
    async fn hedging_returns_stable_error_when_primary_and_secondary_fail() {
        let primary = start_stub_server(StubBehavior::HttpStatus(500)).await;
        let secondary = start_stub_server(StubBehavior::HttpStatus(429)).await;

        let mut cfg = config_with_sources(
            vec![
                source("primary", &primary.url),
                source("secondary", &secondary.url),
            ],
            EvmRoutingStrategy::HedgedLight,
        );
        cfg.hedge_delay = Duration::from_millis(20);

        let factory = EvmJsonRpcHttpTransportFactory::new(cfg);
        let mut t = factory.make(env());

        let err = call_transport(
            t.as_mut(),
            json!({
                "method": "eth_chainId",
                "params": [],
            }),
        )
        .await
        .expect_err("both candidates should fail");

        match err {
            IoError::Transport(info) => {
                assert!(
                    info.code.0 == CODE_EVM_HEDGE_EXHAUSTED
                        || info.code.0 == CODE_EVM_NO_HEALTHY_SOURCE
                        || info.code.0 == CODE_EVM_SOURCE_UNHEALTHY
                );
                assert!(info.retryable);
            }
            other => panic!("expected Transport, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn read_requests_reject_per_request_rpc_url_override() {
        let primary = start_stub_server(StubBehavior::JsonResult(json!("0x1"))).await;
        let cfg = config_with_sources(
            vec![source("primary", &primary.url)],
            EvmRoutingStrategy::Failover,
        );
        let factory = EvmJsonRpcHttpTransportFactory::new(cfg);
        let mut t = factory.make(env());

        let err = call_transport(
            t.as_mut(),
            json!({
                "method": "eth_chainId",
                "params": [],
                "rpc_url": "http://127.0.0.1:9/?token=secret",
            }),
        )
        .await
        .expect_err("read path should reject rpc_url override");

        match err {
            IoError::Other(info) => assert_eq!(info.code.0, CODE_EVM_REQUEST_INVALID),
            other => panic!("expected Other, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn write_requests_reject_per_request_rpc_url_override() {
        let primary = start_stub_server(StubBehavior::JsonResult(json!("0x1"))).await;
        let cfg = config_with_sources(
            vec![source("primary", &primary.url)],
            EvmRoutingStrategy::Failover,
        );
        let factory = EvmJsonRpcHttpTransportFactory::new(cfg);
        let mut t = factory.make(env());

        let err = call_transport(
            t.as_mut(),
            json!({
                "method": "eth_sendRawTransaction",
                "params": ["0x01"],
                "rpc_url": "http://127.0.0.1:9/?token=secret",
            }),
        )
        .await
        .expect_err("write path should reject rpc_url override");

        match err {
            IoError::Other(info) => assert_eq!(info.code.0, CODE_EVM_REQUEST_INVALID),
            other => panic!("expected Other, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn source_failures_do_not_leak_url_query_or_auth_secrets() {
        let cfg = config_with_sources(
            vec![EvmJsonRpcSource {
                id: "primary".to_string(),
                rpc_url: "http://127.0.0.1:9/?api_key=supersecret".to_string(),
                authorization: Some("Bearer topsecret".to_string()),
                kind: EvmSourceKind::RemoteUser,
                require_get_proof_probe: false,
            }],
            EvmRoutingStrategy::Failover,
        );
        let factory = EvmJsonRpcHttpTransportFactory::new(cfg);
        let mut t = factory.make(env());

        let err = call_transport(
            t.as_mut(),
            json!({
                "method": "eth_getLogs",
                "params": [],
            }),
        )
        .await
        .expect_err("source should fail");

        let info = err_info(&err);
        let detail_text = info
            .details
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_default();
        assert!(!detail_text.contains("supersecret"));
        assert!(!detail_text.contains("topsecret"));
        assert!(!detail_text.contains("api_key"));
        assert!(detail_text.contains("primary"));
    }

    #[tokio::test]
    async fn unhealthy_source_is_skipped_until_recovery_window() {
        let primary = start_stub_server(StubBehavior::HttpStatus(500)).await;
        let secondary = start_stub_server(StubBehavior::JsonResult(json!("0x2"))).await;

        let mut cfg = config_with_sources(
            vec![
                source("primary", &primary.url),
                source("secondary", &secondary.url),
            ],
            EvmRoutingStrategy::Failover,
        );
        cfg.unhealthy_cooldown_calls = 2;

        let factory = EvmJsonRpcHttpTransportFactory::new(cfg);
        let mut t = factory.make(env());

        let r1 = call_transport(t.as_mut(), json!({ "method": "eth_getLogs", "params": [] }))
            .await
            .expect("first call should fallback");
        assert_eq!(r1, json!("0x2"));
        let first_primary_hits = primary.hit_count();

        let r2 = call_transport(t.as_mut(), json!({ "method": "eth_getLogs", "params": [] }))
            .await
            .expect("second call should skip unhealthy source");
        assert_eq!(r2, json!("0x2"));
        assert_eq!(primary.hit_count(), first_primary_hits);

        let _ = call_transport(
            t.as_mut(),
            json!({
                "method": "eth_getLogs",
                "params": [],
                "route": {"source_id": "primary"},
            }),
        )
        .await;
        assert!(primary.hit_count() > first_primary_hits);
    }

    #[tokio::test]
    async fn healthy_local_source_is_preferred_over_equal_score_remote_source() {
        let remote = start_stub_server(StubBehavior::JsonResult(json!("0x1"))).await;
        let local = start_stub_server(StubBehavior::JsonResult(json!("0x2"))).await;

        let cfg = EvmJsonRpcHttpConfig {
            sources: vec![
                EvmJsonRpcSource {
                    id: "remote".to_string(),
                    rpc_url: remote.url.clone(),
                    authorization: None,
                    kind: EvmSourceKind::RemotePublic,
                    require_get_proof_probe: false,
                },
                EvmJsonRpcSource {
                    id: "local".to_string(),
                    rpc_url: local.url.clone(),
                    authorization: None,
                    kind: EvmSourceKind::Local,
                    require_get_proof_probe: false,
                },
            ],
            // Put remote first in base order to prove kind-priority tie-breaking.
            preferred_order: vec!["remote".to_string(), "local".to_string()],
            strategy: EvmRoutingStrategy::Failover,
            ..EvmJsonRpcHttpConfig::default()
        };

        let factory = EvmJsonRpcHttpTransportFactory::new(cfg);
        let mut t = factory.make(env());

        let response = call_transport(
            t.as_mut(),
            json!({
                "method": "eth_getLogs",
                "params": [],
            }),
        )
        .await
        .expect("local source should be preferred");

        assert_eq!(response, json!("0x2"));
        assert!(local.hit_count() >= 1);
        assert_eq!(remote.hit_count(), 0);
    }

    #[tokio::test]
    async fn route_source_id_must_exist() {
        let primary = start_stub_server(StubBehavior::JsonResult(json!("0x1"))).await;
        let cfg = config_with_sources(
            vec![source("primary", &primary.url)],
            EvmRoutingStrategy::Failover,
        );
        let factory = EvmJsonRpcHttpTransportFactory::new(cfg);
        let mut t = factory.make(env());

        let err = call_transport(
            t.as_mut(),
            json!({
                "method": "eth_chainId",
                "params": [],
                "route": {"source_id": "missing"},
            }),
        )
        .await
        .expect_err("unknown source id should fail");

        match err {
            IoError::Other(info) => {
                assert_eq!(info.code.0, CODE_EVM_ROUTE_SOURCE_UNKNOWN);
                let details = info.details.unwrap_or_default().to_string();
                assert!(details.contains("missing"));
                assert!(!details.contains("127.0.0.1"));
                assert!(!details.contains("token="));
            }
            other => panic!("expected Other, got {other:?}"),
        }
    }
}
