use super::*;

use std::collections::HashMap;

use mfm_machine::errors::ContextError;
use mfm_machine::errors::{IoError, RunError};
use mfm_machine::events::DomainEvent;
use mfm_machine::ids::{ArtifactId, ErrorCode, FactKey};
use mfm_machine::io::{IoCall, IoResult};

#[derive(Default)]
struct MapContext {
    values: HashMap<String, serde_json::Value>,
}

impl DynContext for MapContext {
    fn read(&self, key: &ContextKey) -> Result<Option<serde_json::Value>, ContextError> {
        Ok(self.values.get(&key.0).cloned())
    }

    fn write(&mut self, key: ContextKey, value: serde_json::Value) -> Result<(), ContextError> {
        self.values.insert(key.0, value);
        Ok(())
    }

    fn delete(&mut self, key: &ContextKey) -> Result<(), ContextError> {
        self.values.remove(&key.0);
        Ok(())
    }

    fn dump(&self) -> Result<serde_json::Value, ContextError> {
        let mut out = serde_json::Map::new();
        for (k, v) in &self.values {
            out.insert(k.clone(), v.clone());
        }
        Ok(serde_json::Value::Object(out))
    }
}

#[derive(Default)]
struct FixedIo {
    responses: HashMap<String, serde_json::Value>,
}

#[async_trait]
impl IoProvider for FixedIo {
    async fn call(&mut self, call: IoCall) -> Result<IoResult, IoError> {
        let method = call
            .request
            .get("method")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();

        let Some(response) = self.responses.get(&method) else {
            return Err(IoError::Other(mfm_state_common::errors::info(
                "unknown_method",
                ErrorCategory::Rpc,
                false,
                "unknown rpc method",
            )));
        };

        Ok(IoResult {
            response: response.clone(),
            recorded_payload_id: None,
        })
    }

    async fn record_value(
        &mut self,
        _key: FactKey,
        _value: serde_json::Value,
    ) -> Result<ArtifactId, IoError> {
        Ok(ArtifactId("0".repeat(64)))
    }

    async fn get_recorded_fact(&mut self, _key: &FactKey) -> Result<Option<ArtifactId>, IoError> {
        Ok(None)
    }

    async fn now_millis(&mut self) -> Result<u64, IoError> {
        Ok(0)
    }

    async fn random_bytes(&mut self, n: usize) -> Result<Vec<u8>, IoError> {
        Ok(vec![0u8; n])
    }
}

struct NoopRecorder;

const NETWORK_ID: &str = "ethereum-mainnet";

#[async_trait]
impl EventRecorder for NoopRecorder {
    async fn emit(&mut self, _event: DomainEvent) -> Result<(), RunError> {
        Ok(())
    }

    async fn emit_many(&mut self, _events: Vec<DomainEvent>) -> Result<(), RunError> {
        Ok(())
    }
}

#[tokio::test]
async fn reads_u64_and_writes_context() {
    let state = ReadU64HexState::new(
        StateId::must_new("m.main.chain_id".to_string()),
        NETWORK_ID,
        "eth_chainId",
        serde_json::json!([]),
        ContextKey("chain_id".to_string()),
    );

    let mut ctx = MapContext::default();
    let mut io = FixedIo::default();
    io.responses
        .insert("eth_chainId".to_string(), serde_json::json!("0x1"));
    let mut rec = NoopRecorder;

    let out = state
        .handle(&mut ctx, &mut io, &mut rec)
        .await
        .expect("state handle");
    assert_eq!(out.snapshot, SnapshotPolicy::OnSuccess);
    assert_eq!(
        ctx.read(&ContextKey("chain_id".to_string())).expect("read"),
        Some(serde_json::json!(1))
    );
}

#[tokio::test]
async fn invalid_hex_response_fails_with_stable_code() {
    let state = ReadU64HexState::new(
        StateId::must_new("m.main.chain_id".to_string()),
        NETWORK_ID,
        "eth_chainId",
        serde_json::json!([]),
        ContextKey("chain_id".to_string()),
    );

    let mut ctx = MapContext::default();
    let mut io = FixedIo::default();
    io.responses
        .insert("eth_chainId".to_string(), serde_json::json!("not_hex"));
    let mut rec = NoopRecorder;

    let err = state
        .handle(&mut ctx, &mut io, &mut rec)
        .await
        .expect_err("expected parse error");
    assert_eq!(err.info.code.0, "evm_response_invalid");
}

#[tokio::test]
async fn expectation_mismatch_fails_with_state_scoped_error() {
    let state = ReadU64HexState::new(
        StateId::must_new("m.main.chain_id".to_string()),
        NETWORK_ID,
        "eth_chainId",
        serde_json::json!([]),
        ContextKey("chain_id".to_string()),
    )
    .with_expectation(U64Expectation::parsing_input(
        1,
        "chain_id_mismatch",
        "rpc chain_id did not match configured chain_id",
    ));

    let mut ctx = MapContext::default();
    let mut io = FixedIo::default();
    io.responses
        .insert("eth_chainId".to_string(), serde_json::json!("0x2"));
    let mut rec = NoopRecorder;

    let err = state
        .handle(&mut ctx, &mut io, &mut rec)
        .await
        .expect_err("expected mismatch");
    assert_eq!(err.info.code.0, "chain_id_mismatch");
    assert_eq!(
        err.state_id,
        Some(StateId::must_new("m.main.chain_id".to_string()))
    );
}

#[test]
fn io_error_mapping_preserves_error_info() {
    let io_err = IoError::Other(mfm_machine::errors::ErrorInfo {
        code: ErrorCode("io_code".to_string()),
        category: ErrorCategory::Rpc,
        retryable: false,
        message: "io message".to_string(),
        details: None,
    });
    let state_err = state_from_io(io_err);
    assert_eq!(state_err.info.code.0, "io_code");
    assert_eq!(state_err.info.message, "io message");
}

#[tokio::test]
async fn read_hex_string_writes_string_value() {
    let state = ReadHexStringState::new(
        StateId::must_new("m.main.client_version".to_string()),
        NETWORK_ID,
        "web3_clientVersion",
        serde_json::json!([]),
        ContextKey("client_version".to_string()),
    );

    let mut ctx = MapContext::default();
    let mut io = FixedIo::default();
    io.responses.insert(
        "web3_clientVersion".to_string(),
        serde_json::json!("0xdeadbeef"),
    );
    let mut rec = NoopRecorder;

    state
        .handle(&mut ctx, &mut io, &mut rec)
        .await
        .expect("state");
    assert_eq!(
        ctx.read(&ContextKey("client_version".to_string()))
            .expect("read"),
        Some(serde_json::json!("0xdeadbeef"))
    );
}

#[tokio::test]
async fn read_u256_hex_rejects_overflow() {
    let state = ReadU256HexState::new(
        StateId::must_new("m.main.balance".to_string()),
        NETWORK_ID,
        "eth_getBalance",
        serde_json::json!(["0xabc", "latest"]),
        ContextKey("balance".to_string()),
    );

    let mut ctx = MapContext::default();
    let mut io = FixedIo::default();
    io.responses.insert(
        "eth_getBalance".to_string(),
        serde_json::json!(format!("0x{}", "f".repeat(65))),
    );
    let mut rec = NoopRecorder;

    let err = state
        .handle(&mut ctx, &mut io, &mut rec)
        .await
        .expect_err("overflow");
    assert_eq!(err.info.code.0, "evm_response_invalid");
}

#[tokio::test]
async fn eth_call_u64_decode_writes_numeric_value() {
    let state = EthCallState::new(
        StateId::must_new("m.main.eth_call".to_string()),
        NETWORK_ID,
        "0x0000000000000000000000000000000000000000",
        "0x313ce567",
        ContextKey("decimals".to_string()),
    )
    .with_decode(EthCallDecode::U64);

    let mut ctx = MapContext::default();
    let mut io = FixedIo::default();
    io.responses
        .insert("eth_call".to_string(), serde_json::json!("0x12"));
    let mut rec = NoopRecorder;

    state
        .handle(&mut ctx, &mut io, &mut rec)
        .await
        .expect("state");
    assert_eq!(
        ctx.read(&ContextKey("decimals".to_string())).expect("read"),
        Some(serde_json::json!(18))
    );
}
