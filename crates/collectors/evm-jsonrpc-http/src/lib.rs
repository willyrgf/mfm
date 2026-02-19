//! EVM JSON-RPC over HTTP live transport.
//!
//! This crate implements a `LiveIoTransportFactory` for the `namespace = "evm"` IO surface.
//! It supports source-id based routing, retry-oriented failover, and optional light-call hedging.
//!
//! Security notes:
//! - RPC URLs and authorization headers are runtime configuration and MUST NOT be persisted.
//! - Errors MUST NOT include request payloads, response bodies, or authorization values.

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;

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
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvmJsonRpcHttpConfigError {
    NoSources,
    EmptySourceId,
    DuplicateSourceId(String),
    UnknownPreferredSourceId(String),
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

    Ok(())
}

#[derive(Clone)]
pub struct EvmJsonRpcHttpTransportFactory {
    cfg: EvmJsonRpcHttpConfig,
    client: reqwest::Client,
    config_error: Option<EvmJsonRpcHttpConfigError>,
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
}

impl LiveIoTransportFactory for EvmJsonRpcHttpTransportFactory {
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

    fn mark_source_success(&mut self, source_id: &str) {
        if let Some(state) = self.source_states.get_mut(source_id) {
            state.score = (state.score + 1).clamp(-100, 100);
            state.disabled_until_call = 0;
        }
    }

    fn mark_source_failure(&mut self, source_id: &str, call_ordinal: u64) {
        if let Some(state) = self.source_states.get_mut(source_id) {
            state.score = (state.score - 3).clamp(-100, 100);
            if self.cfg.unhealthy_cooldown_calls > 0 {
                state.disabled_until_call =
                    call_ordinal.saturating_add(self.cfg.unhealthy_cooldown_calls);
            }
        }
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
                self.mark_source_success(source_id);
                Ok(())
            }
            Err(err) => {
                self.mark_source_failure(source_id, call_ordinal);
                let _ = err;
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
                    self.mark_source_success(source_id);
                    return Ok(result);
                }
                Err(err) => {
                    self.mark_source_failure(source_id, call_ordinal);
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
                self.mark_source_success(primary_id);
                Ok(result)
            }
            Err(err) => {
                self.mark_source_failure(primary_id, call_ordinal);
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
            return self.call_failover(source_ids, req, call_ordinal).await;
        }

        let primary_id = source_ids[0].clone();
        self.ensure_source_probed(&primary_id, call_ordinal).await?;

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
                    self.mark_source_success(&primary_id);
                    return Ok(value);
                }
                Err(err) => {
                    self.mark_source_failure(&primary_id, call_ordinal);
                    if !is_retryable(&err) {
                        return Err(err);
                    }
                    return self
                        .call_failover(&source_ids[1..], req, call_ordinal)
                        .await;
                }
            }
        }

        let secondary_id = source_ids[1].clone();
        self.ensure_source_probed(&secondary_id, call_ordinal)
            .await?;

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
                self.mark_source_success(&primary_id);
                return Ok(value);
            }
            FirstOutcome::Secondary(Ok(value)) => {
                self.mark_source_success(&secondary_id);
                return Ok(value);
            }
            FirstOutcome::Primary(Err(primary_err)) => {
                self.mark_source_failure(&primary_id, call_ordinal);
                failures.push(failure_detail(&primary_id, &primary_err));

                let secondary_res = secondary.await;
                match secondary_res {
                    Ok(value) => {
                        self.mark_source_success(&secondary_id);
                        return Ok(value);
                    }
                    Err(secondary_err) => {
                        self.mark_source_failure(&secondary_id, call_ordinal);
                        failures.push(failure_detail(&secondary_id, &secondary_err));
                    }
                }
            }
            FirstOutcome::Secondary(Err(secondary_err)) => {
                self.mark_source_failure(&secondary_id, call_ordinal);
                failures.push(failure_detail(&secondary_id, &secondary_err));

                let primary_res = primary.await;
                match primary_res {
                    Ok(value) => {
                        self.mark_source_success(&primary_id);
                        return Ok(value);
                    }
                    Err(primary_err) => {
                        self.mark_source_failure(&primary_id, call_ordinal);
                        failures.push(failure_detail(&primary_id, &primary_err));
                    }
                }
            }
        }

        if source_ids.len() > 2 {
            return self
                .call_failover(&source_ids[2..], req, call_ordinal)
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

        let bytes = resp.bytes().await.map_err(|_| {
            IoError::Transport(info_with_details(
                CODE_EVM_HTTP_BODY_READ_FAILED,
                ErrorCategory::Rpc,
                true,
                "failed to read evm rpc response body",
                source_error_details(&source_id, None),
            ))
        })?;

        let v = serde_json::from_slice::<serde_json::Value>(&bytes).map_err(|_| {
            IoError::Other(info_with_details(
                CODE_EVM_RESPONSE_INVALID_JSON,
                ErrorCategory::ParsingInput,
                false,
                "evm rpc response was not valid json",
                source_error_details(&source_id, None),
            ))
        })?;

        let obj = v.as_object().ok_or_else(|| {
            IoError::Other(info_with_details(
                CODE_EVM_JSONRPC_INVALID_RESPONSE,
                ErrorCategory::ParsingInput,
                false,
                "evm jsonrpc response was not an object",
                source_error_details(&source_id, None),
            ))
        })?;

        if let Some(error_value) = obj.get("error") {
            return Err(IoError::Transport(info_with_details(
                CODE_EVM_JSONRPC_ERROR,
                ErrorCategory::Rpc,
                true,
                "evm jsonrpc returned an error",
                jsonrpc_error_details(&source_id, error_value),
            )));
        }

        let Some(result) = obj.get("result") else {
            return Err(IoError::Other(info_with_details(
                CODE_EVM_JSONRPC_MISSING_RESULT,
                ErrorCategory::ParsingInput,
                false,
                "evm jsonrpc response missing result",
                source_error_details(&source_id, None),
            )));
        };

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
        let method_class = classify_method(&req.method, &req.params, &self.cfg);

        if req.rpc_url.is_some() {
            return Err(self.rpc_url_override_error());
        }

        let call_ordinal = self.current_call_ordinal();
        let json_call = JsonRpcCall::new(req.method.clone(), req.params.clone());

        let route_source_id = req.route.as_ref().map(|route| route.source_id.as_str());
        let source_ids = self.ordered_source_ids(route_source_id, call_ordinal)?;

        match method_class {
            MethodClass::WriteOrSideEffect => {
                self.call_write_primary(&source_ids, &json_call, call_ordinal)
                    .await
            }
            MethodClass::ReadHeavy => {
                self.call_failover(&source_ids, &json_call, call_ordinal)
                    .await
            }
            MethodClass::ReadLight => match self.cfg.strategy {
                EvmRoutingStrategy::Failover => {
                    self.call_failover(&source_ids, &json_call, call_ordinal)
                        .await
                }
                EvmRoutingStrategy::HedgedLight => {
                    self.call_hedged_light(&source_ids, &json_call, call_ordinal)
                        .await
                }
            },
        }
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
            state_id: StateId("machine.main.s1".to_string()),
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
            IoError::Other(info) => assert_eq!(info.code.0, CODE_EVM_ROUTE_SOURCE_UNKNOWN),
            other => panic!("expected Other, got {other:?}"),
        }
    }
}
