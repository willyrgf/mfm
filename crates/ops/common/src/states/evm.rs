use async_trait::async_trait;

use mfm_collectors_evm::{parse_u64_hex_value, EvmIoClient, JsonRpcCall};
use mfm_machine::context::DynContext;
use mfm_machine::errors::{ErrorCategory, StateError};
use mfm_machine::ids::{ContextKey, StateId};
use mfm_machine::io::IoProvider;
use mfm_machine::meta::StateMeta;
use mfm_machine::recorder::EventRecorder;
use mfm_machine::state::{SnapshotPolicy, State, StateOutcome};

use crate::ctx::write_json;
use crate::errors::{state_error_with_state, state_from_io, state_unknown};
use crate::states::meta;

#[derive(Clone, Debug)]
pub struct U64Expectation {
    pub expected: u64,
    pub mismatch_code: &'static str,
    pub mismatch_category: ErrorCategory,
    pub mismatch_retryable: bool,
    pub mismatch_message: &'static str,
}

impl U64Expectation {
    pub fn parsing_input(
        expected: u64,
        mismatch_code: &'static str,
        mismatch_message: &'static str,
    ) -> Self {
        Self {
            expected,
            mismatch_code,
            mismatch_category: ErrorCategory::ParsingInput,
            mismatch_retryable: false,
            mismatch_message,
        }
    }
}

#[derive(Clone, Debug)]
pub struct ReadU64HexState {
    pub state_id: StateId,
    pub method: String,
    pub params: serde_json::Value,
    pub output_key: ContextKey,
    pub expectation: Option<U64Expectation>,
}

impl ReadU64HexState {
    pub fn new(
        state_id: StateId,
        method: impl Into<String>,
        params: serde_json::Value,
        output_key: ContextKey,
    ) -> Self {
        Self {
            state_id,
            method: method.into(),
            params,
            output_key,
            expectation: None,
        }
    }

    pub fn with_expectation(mut self, expectation: U64Expectation) -> Self {
        self.expectation = Some(expectation);
        self
    }
}

#[async_trait]
impl State for ReadU64HexState {
    fn meta(&self) -> StateMeta {
        meta::fetch_data()
    }

    async fn handle(
        &self,
        ctx: &mut dyn DynContext,
        io: &mut dyn IoProvider,
        _rec: &mut dyn EventRecorder,
    ) -> Result<StateOutcome, StateError> {
        let mut client = EvmIoClient::new(self.state_id.clone(), io);
        let res = client
            .call(JsonRpcCall::new(self.method.clone(), self.params.clone()))
            .await
            .map_err(state_from_io)?;

        let value = parse_u64_hex_value(&res.response)
            .map_err(|_| state_unknown("evm_response_invalid", "evm response was not a hex u64"))?;

        if let Some(expectation) = &self.expectation {
            if value != expectation.expected {
                return Err(state_error_with_state(
                    self.state_id.clone(),
                    expectation.mismatch_code,
                    expectation.mismatch_category.clone(),
                    expectation.mismatch_retryable,
                    expectation.mismatch_message,
                ));
            }
        }

        write_json(ctx, self.output_key.clone(), serde_json::json!(value))?;

        Ok(StateOutcome {
            snapshot: SnapshotPolicy::OnSuccess,
        })
    }
}

#[cfg(test)]
mod tests {
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
                return Err(IoError::Other(crate::errors::info(
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

        async fn get_recorded_fact(
            &mut self,
            _key: &FactKey,
        ) -> Result<Option<ArtifactId>, IoError> {
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
            StateId("m.main.chain_id".to_string()),
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
            StateId("m.main.chain_id".to_string()),
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
            StateId("m.main.chain_id".to_string()),
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
        assert_eq!(err.state_id, Some(StateId("m.main.chain_id".to_string())));
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
}
