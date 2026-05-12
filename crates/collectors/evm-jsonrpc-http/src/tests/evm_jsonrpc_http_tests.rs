use super::*;

use async_trait::async_trait;
use mfm_machine::engine::Stores;
use mfm_machine::errors::StorageError;
use mfm_machine::ids::{ArtifactId, RunId, StateId};
use mfm_machine::live_io::LiveIoEnv;
use mfm_machine::stores::{
    AppendBatchResult, ArtifactKind, ArtifactStore, StreamAppend, StreamId, StreamRecord,
    StreamStore,
};
use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;

#[derive(Clone)]
struct NoopStreamStore;

#[async_trait]
impl StreamStore for NoopStreamStore {
    async fn head_seq(&self, _stream_id: &StreamId) -> Result<u64, StorageError> {
        Ok(0)
    }

    async fn append(&self, _append: StreamAppend) -> Result<u64, StorageError> {
        Ok(0)
    }

    async fn append_batch(
        &self,
        _appends: Vec<StreamAppend>,
    ) -> Result<AppendBatchResult, StorageError> {
        Ok(AppendBatchResult {
            stream_heads: Vec::new(),
        })
    }

    async fn read_range(
        &self,
        _stream_id: &StreamId,
        _from_seq: u64,
        _to_seq: Option<u64>,
    ) -> Result<Vec<StreamRecord>, StorageError> {
        Ok(Vec::new())
    }
}

#[derive(Clone)]
struct NoopArtifactStore;

#[async_trait]
impl ArtifactStore for NoopArtifactStore {
    async fn put(&self, _kind: ArtifactKind, _bytes: Vec<u8>) -> Result<ArtifactId, StorageError> {
        Ok(ArtifactId::must_new("0".repeat(64)))
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
            streams: Arc::new(NoopStreamStore),
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

fn transport_factory(cfg: EvmJsonRpcHttpConfig) -> EvmJsonRpcHttpTransportFactory {
    EvmJsonRpcHttpTransportFactory::try_new(cfg).expect("valid config")
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
fn transport_creation_errors_do_not_include_source_url_credentials() {
    let mut cfg = config_with_sources(
        vec![EvmJsonRpcSource {
            id: "primary".to_string(),
            rpc_url: "https://url_user:url_password@example.com:8545/path?api_key=query_secret&token=query_token#frag".to_string(),
            authorization: Some("Bearer authorization_secret".to_string()),
            kind: EvmSourceKind::RemoteUser,
            require_get_proof_probe: false,
        }],
        EvmRoutingStrategy::Failover,
    );
    cfg.preferred_order = vec!["missing".to_string()];

    let err = match EvmJsonRpcHttpTransportFactory::try_new(cfg) {
        Ok(_) => panic!("unknown preferred source should fail construction"),
        Err(err) => err.to_string(),
    };

    assert!(!err.contains("url_user"));
    assert!(!err.contains("url_password"));
    assert!(!err.contains("api_key"));
    assert!(!err.contains("query_secret"));
    assert!(!err.contains("query_token"));
    assert!(!err.contains("authorization_secret"));
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
fn source_debug_redacts_url_query_userinfo_and_authorization() {
    let source = EvmJsonRpcSource {
        id: "primary".to_string(),
        rpc_url: "https://url_user:url_password@example.com:8545/path?api_key=query_secret&token=query_token#frag".to_string(),
        authorization: Some("Bearer authorization_secret".to_string()),
        kind: EvmSourceKind::RemoteUser,
        require_get_proof_probe: true,
    };

    let rendered = format!("{source:?}");

    assert!(rendered.contains("EvmJsonRpcSource"));
    assert!(rendered.contains("https://example.com:8545"));
    assert!(!rendered.contains("url_user"));
    assert!(!rendered.contains("url_password"));
    assert!(!rendered.contains("api_key"));
    assert!(!rendered.contains("query_secret"));
    assert!(!rendered.contains("query_token"));
    assert!(!rendered.contains("authorization_secret"));
}

#[test]
fn config_debug_redacts_source_secrets() {
    let cfg = config_with_sources(
        vec![EvmJsonRpcSource {
            id: "primary".to_string(),
            rpc_url: "https://url_user:url_password@example.com:8545/path?api_key=query_secret&token=query_token#frag".to_string(),
            authorization: Some("Bearer authorization_secret".to_string()),
            kind: EvmSourceKind::RemoteUser,
            require_get_proof_probe: false,
        }],
        EvmRoutingStrategy::Failover,
    );

    let rendered = format!("{cfg:?}");

    assert!(rendered.contains("EvmJsonRpcHttpConfig"));
    assert!(rendered.contains("https://example.com:8545"));
    assert!(!rendered.contains("url_user"));
    assert!(!rendered.contains("url_password"));
    assert!(!rendered.contains("api_key"));
    assert!(!rendered.contains("query_secret"));
    assert!(!rendered.contains("query_token"));
    assert!(!rendered.contains("authorization_secret"));
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
    let rate_write =
        EvmJsonRpcHttpTransport::score_failure_penalty(&rate_limit, MethodClass::WriteOrSideEffect);

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
    let factory = transport_factory(cfg);
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
    let factory = transport_factory(cfg);
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
    let factory = transport_factory(cfg);
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
    let factory = transport_factory(cfg);
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
    let factory = transport_factory(cfg);
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
    let factory = transport_factory(cfg);
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
    let factory = transport_factory(cfg);
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
    let factory = transport_factory(cfg);
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
    let factory = transport_factory(cfg);
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
    let factory = transport_factory(cfg);
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
    let factory = transport_factory(cfg);
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

    let factory = transport_factory(cfg);
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
    let factory = transport_factory(cfg);
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
    let factory = transport_factory(cfg);
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
    let factory = transport_factory(cfg);
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

#[test]
fn jsonrpc_error_message_redacts_secret_markers() {
    let details = jsonrpc_error_details(
        "primary",
        &json!({
            "code": -32000,
            "message": "Authorization: Bearer auth_token api_key=query_secret password=rpc_password",
        }),
    )
    .expect("error details should be produced");
    let detail_text = details.to_string();

    assert!(detail_text.contains(REDACTED_DIAGNOSTIC));
    assert!(!detail_text.contains("auth_token"));
    assert!(!detail_text.contains("query_secret"));
    assert!(!detail_text.contains("rpc_password"));
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

    let factory = transport_factory(cfg);
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

    let factory = transport_factory(cfg);
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
    let factory = transport_factory(cfg);
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
