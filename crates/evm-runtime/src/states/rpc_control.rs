//! Reusable `rpc.control` source preparation helpers.
//!
//! This module contains a state that runs the managed source setup/probe/rank preflight
//! before downstream runtime states consume `rpc.control` sources. It is reusable across ops
//! that need to fail fast when a network scope has no responsive managed sources.

use std::collections::HashSet;

use async_trait::async_trait;
use mfm_collectors_rpc_control::{EvmIoClient, DEFAULT_CONTROL_SCOPE};
use mfm_machine::context::DynContext;
use mfm_machine::errors::{ErrorCategory, StateError};
use mfm_machine::ids::StateId;
use mfm_machine::io::IoProvider;
use mfm_machine::meta::StateMeta;
use mfm_machine::recorder::EventRecorder;
use mfm_machine::state::{SnapshotPolicy, State, StateOutcome};
use mfm_state_common::errors as op_errors;
use mfm_state_common::states::meta;

/// Minimal routed network config consumed by [`PrepareSourcesState`].
#[derive(Clone, Debug)]
pub struct RpcControlNetworkRoute {
    /// Stable network identifier to target when preparing sources.
    pub network_id: String,
    /// Stable control-plane scope used for managed source ranking.
    pub control_scope: String,
}

impl RpcControlNetworkRoute {
    /// Builds a normalized route entry.
    pub fn new(network_id: impl Into<String>, control_scope: impl Into<String>) -> Self {
        Self {
            network_id: network_id.into(),
            control_scope: control_scope.into(),
        }
    }
}

/// Runtime state that preflights managed sources for each configured routed network.
#[derive(Clone, Debug)]
pub struct PrepareSourcesState {
    /// Stable state identifier assigned by the execution plan.
    pub state_id: StateId,
    /// Target networks whose managed source pools should be prepared.
    pub networks: Vec<RpcControlNetworkRoute>,
}

#[async_trait]
impl State for PrepareSourcesState {
    fn meta(&self) -> StateMeta {
        meta::fetch_data()
    }

    async fn handle(
        &self,
        _ctx: &mut dyn DynContext,
        io: &mut dyn IoProvider,
        _rec: &mut dyn EventRecorder,
    ) -> Result<StateOutcome, StateError> {
        if self.networks.is_empty() {
            return Ok(StateOutcome {
                snapshot: SnapshotPolicy::OnSuccess,
            });
        }

        let mut dedup = HashSet::new();
        let mut client = EvmIoClient::new(self.state_id.clone(), io);

        for network in &self.networks {
            let network_id = network.network_id.trim();
            if network_id.is_empty() {
                return Err(op_errors::state_error_with_state(
                    self.state_id.clone(),
                    "rpc_control_network_required",
                    ErrorCategory::ParsingInput,
                    false,
                    "rpc.control managed requests require a non-empty network_id",
                ));
            }

            let control_scope = network.control_scope.trim();
            let control_scope = if control_scope.is_empty() {
                DEFAULT_CONTROL_SCOPE
            } else {
                control_scope
            };

            if !dedup.insert((network_id.to_string(), control_scope.to_string())) {
                continue;
            }

            let response = client
                .prepare_sources_in_scope(control_scope, network_id.to_string())
                .await
                .map_err(op_errors::state_from_io)?;

            if !response.sources.iter().any(|entry| entry.healthy) {
                return Err(op_errors::state_error_with_state(
                    self.state_id.clone(),
                    "rpc_control_no_healthy_sources",
                    ErrorCategory::Rpc,
                    false,
                    format!(
                        "no responsive rpc.control sources found for network `{}` and scope `{}`",
                        network_id, response.control_scope
                    ),
                ));
            }
        }

        Ok(StateOutcome {
            snapshot: SnapshotPolicy::OnSuccess,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    use async_trait::async_trait;
    use mfm_machine::context::DynContext;
    use mfm_machine::errors::{ContextError, ErrorCategory, ErrorInfo, IoError, RunError};
    use mfm_machine::events::DomainEvent;
    use mfm_machine::ids::{ArtifactId, ErrorCode, FactKey};
    use mfm_machine::io::{IoCall, IoResult};
    use mfm_machine::recorder::EventRecorder;
    use serde_json::Value;

    fn info(code: &'static str, category: ErrorCategory, message: &'static str) -> ErrorInfo {
        ErrorInfo {
            code: ErrorCode(code.to_string()),
            category,
            retryable: false,
            message: message.to_string(),
            details: None,
        }
    }

    fn prepared_response(
        network_id: &str,
        control_scope: &str,
        healthy: bool,
    ) -> serde_json::Value {
        serde_json::json!({
            "control_scope": control_scope,
            "network_id": network_id,
            "pool_kind": "test",
            "available_source_ids": ["source-1"],
            "ranked_source_ids": ["source-1"],
            "sources": [
                {
                    "source_id": "source-1",
                    "healthy": healthy,
                }
            ]
        })
    }

    #[derive(Default)]
    struct MapContext {
        values: HashMap<String, serde_json::Value>,
    }

    impl DynContext for MapContext {
        fn read(
            &self,
            key: &mfm_machine::ids::ContextKey,
        ) -> Result<Option<serde_json::Value>, ContextError> {
            Ok(self.values.get(&key.0).cloned())
        }

        fn write(
            &mut self,
            key: mfm_machine::ids::ContextKey,
            value: serde_json::Value,
        ) -> Result<(), ContextError> {
            self.values.insert(key.0, value);
            Ok(())
        }

        fn delete(&mut self, key: &mfm_machine::ids::ContextKey) -> Result<(), ContextError> {
            self.values.remove(&key.0);
            Ok(())
        }

        fn dump(&self) -> Result<serde_json::Value, ContextError> {
            let mut out = serde_json::Map::new();
            for (key, value) in &self.values {
                out.insert(key.clone(), value.clone());
            }
            Ok(serde_json::Value::Object(out))
        }
    }

    #[derive(Default)]
    struct TrackingIo {
        calls: Vec<IoCall>,
        responses: HashMap<(String, String), serde_json::Value>,
    }

    #[async_trait]
    impl IoProvider for TrackingIo {
        async fn call(&mut self, call: IoCall) -> Result<IoResult, IoError> {
            self.calls.push(call.clone());

            let kind = call
                .request
                .get("kind")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if kind != "prepare_sources" {
                return Err(IoError::Other(info(
                    "unexpected_call",
                    ErrorCategory::Unknown,
                    "unexpected rpc.control request",
                )));
            }

            let network_id = call
                .request
                .get("network_id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let control_scope = call
                .request
                .get("control_scope")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();

            self.responses
                .get(&(network_id, control_scope))
                .cloned()
                .map(|response| IoResult {
                    response,
                    recorded_payload_id: None,
                })
                .ok_or_else(|| {
                    IoError::Other(info(
                        "missing_mock_response",
                        ErrorCategory::Unknown,
                        "missing prepare_sources mock response",
                    ))
                })
        }

        async fn record_value(
            &mut self,
            _key: FactKey,
            _value: serde_json::Value,
        ) -> Result<ArtifactId, IoError> {
            Ok(ArtifactId::must_new("0".repeat(64)))
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
            Ok(vec![0; n])
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
    async fn prepare_sources_deduplicates_network_scope_pairs() {
        let mut io = TrackingIo::default();
        io.responses.insert(
            ("ethereum-mainnet".to_string(), "shared".to_string()),
            prepared_response("ethereum-mainnet", "shared", true),
        );
        io.responses.insert(
            ("arbitrum-mainnet".to_string(), "shared".to_string()),
            prepared_response("arbitrum-mainnet", "shared", true),
        );

        let state = PrepareSourcesState {
            state_id: StateId::must_new("prepare.main.sources".to_string()),
            networks: vec![
                RpcControlNetworkRoute::new("ethereum-mainnet", "shared"),
                RpcControlNetworkRoute::new("  ethereum-mainnet ", "  shared  "),
                RpcControlNetworkRoute::new("arbitrum-mainnet", "shared"),
            ],
        };

        let mut ctx = MapContext::default();
        let mut rec = NoopRecorder;
        state
            .handle(&mut ctx, &mut io, &mut rec)
            .await
            .expect("prepare sources");

        assert_eq!(io.calls.len(), 2);
        assert_eq!(
            io.calls[0]
                .request
                .get("network_id")
                .and_then(Value::as_str),
            Some("ethereum-mainnet")
        );
        assert_eq!(
            io.calls[1]
                .request
                .get("network_id")
                .and_then(Value::as_str),
            Some("arbitrum-mainnet")
        );
    }

    #[tokio::test]
    async fn prepare_sources_rejects_missing_network_id() {
        let mut io = TrackingIo::default();
        let state = PrepareSourcesState {
            state_id: StateId::must_new("prepare.main.sources".to_string()),
            networks: vec![RpcControlNetworkRoute::new("", "shared")],
        };
        let mut ctx = MapContext::default();
        let mut rec = NoopRecorder;

        let err = state
            .handle(&mut ctx, &mut io, &mut rec)
            .await
            .expect_err("missing network_id");

        assert_eq!(err.info.code.0, "rpc_control_network_required");
        assert_eq!(err.info.category, ErrorCategory::ParsingInput);
    }

    #[tokio::test]
    async fn prepare_sources_requires_at_least_one_healthy() {
        let mut io = TrackingIo::default();
        io.responses.insert(
            ("ethereum-mainnet".to_string(), "shared".to_string()),
            prepared_response("ethereum-mainnet", "shared", false),
        );
        let state = PrepareSourcesState {
            state_id: StateId::must_new("prepare.main.sources".to_string()),
            networks: vec![RpcControlNetworkRoute::new("ethereum-mainnet", "shared")],
        };
        let mut ctx = MapContext::default();
        let mut rec = NoopRecorder;

        let err = state
            .handle(&mut ctx, &mut io, &mut rec)
            .await
            .expect_err("no healthy source");

        assert_eq!(err.info.code.0, "rpc_control_no_healthy_sources");
        assert_eq!(err.info.category, ErrorCategory::Rpc);
    }
}
