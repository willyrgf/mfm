#![cfg_attr(test, allow(clippy::disallowed_methods, clippy::disallowed_types))]
#![cfg_attr(not(test), deny(clippy::disallowed_methods, clippy::disallowed_types))]
#![warn(missing_docs)]
//! EVM read-only op.
//!
//! Source of truth: `docs/redesign.md` (v4).
//!
//! This op is intentionally small: it exists as the first “real” vertical slice that demonstrates:
//! - deterministic facts recording in live mode
//! - replay determinism
//! - crash/resume determinism (orphan attempt reuse)
//!
//! # Examples
//!
//! ```rust
//! use mfm_op_evm_read::EvmReadOp;
//! use mfm_sdk::op::Operation;
//!
//! let op = EvmReadOp;
//! assert_eq!(op.op_id().as_str(), "evm_read");
//! ```

use std::sync::Arc;

use serde::Deserialize;

use mfm_evm_runtime::states::read::ReadU64HexState;
use mfm_machine::config::RunConfig;
use mfm_machine::errors::ErrorCategory;
use mfm_machine::ids::{ContextKey, OpId, OpPath, StateId};
use mfm_machine::plan::{DependencyEdge, StateGraph, StateNode};

use mfm_sdk::errors::SdkError;
use mfm_sdk::ids::PortKey;
use mfm_sdk::op::{OpIo, Operation};
use mfm_state_common::errors;

const OP_ID: &str = "evm_read";
const OP_VERSION: &str = "v1";

fn sdk_err(code: &'static str, message: &'static str) -> SdkError {
    errors::sdk_error(code, ErrorCategory::Unknown, false, message)
}

fn ctx_key(op_path: &OpPath, suffix: &'static str) -> ContextKey {
    ContextKey(format!("{}.{}", op_path.0, suffix))
}

fn default_true() -> bool {
    true
}

fn default_control_scope() -> String {
    "shared".to_string()
}

#[derive(Clone, Debug, Deserialize)]
struct EvmReadConfig {
    network_id: String,

    #[serde(default = "default_control_scope")]
    control_scope: String,

    #[serde(default = "default_true")]
    include_chain_id: bool,

    #[serde(default = "default_true")]
    include_block_number: bool,
}

impl Default for EvmReadConfig {
    fn default() -> Self {
        Self {
            network_id: "ethereum-mainnet".to_string(),
            control_scope: default_control_scope(),
            include_chain_id: true,
            include_block_number: true,
        }
    }
}

/// Planner for read-only EVM RPC queries such as chain ID and block number.
#[derive(Clone, Default)]
pub struct EvmReadOp;

impl Operation for EvmReadOp {
    fn op_id(&self) -> OpId {
        OpId::must_new(OP_ID.to_string())
    }

    fn op_version(&self) -> String {
        OP_VERSION.to_string()
    }

    fn io(&self, _op_config: &serde_json::Value) -> Result<OpIo, SdkError> {
        Ok(OpIo {
            imports: Vec::new(),
            exports: vec![
                PortKey("chain_id".to_string()),
                PortKey("block_number".to_string()),
            ],
        })
    }

    fn expand(
        &self,
        op_path: OpPath,
        op_config: &serde_json::Value,
        _run_config: &RunConfig,
    ) -> Result<StateGraph, SdkError> {
        let cfg: EvmReadConfig = serde_json::from_value(op_config.clone())
            .map_err(|_| sdk_err("invalid_op_config", "invalid evm_read op_config"))?;
        if cfg.network_id.trim().is_empty() {
            return Err(sdk_err("invalid_op_config", "network_id must be non-empty"));
        }
        if !cfg.include_chain_id && !cfg.include_block_number {
            return Err(sdk_err(
                "invalid_op_config",
                "at least one query must be enabled",
            ));
        }

        let mut states: Vec<StateNode> = Vec::new();
        let mut edges: Vec<DependencyEdge> = Vec::new();

        let mut last: Option<StateId> = None;

        if cfg.include_chain_id {
            let id = StateId::must_new(format!("{}.chain_id", op_path.0));
            let st = Arc::new(
                ReadU64HexState::new(
                    id.clone(),
                    cfg.network_id.clone(),
                    "eth_chainId",
                    serde_json::json!([]),
                    ctx_key(&op_path, "chain_id"),
                )
                .with_control_scope(cfg.control_scope.clone()),
            );
            states.push(StateNode {
                id: id.clone(),
                state: st,
            });
            last = Some(id);
        }

        if cfg.include_block_number {
            let id = StateId::must_new(format!("{}.block_number", op_path.0));
            let st = Arc::new(
                ReadU64HexState::new(
                    id.clone(),
                    cfg.network_id.clone(),
                    "eth_blockNumber",
                    serde_json::json!([]),
                    ctx_key(&op_path, "block_number"),
                )
                .with_control_scope(cfg.control_scope.clone()),
            );
            if let Some(prev) = &last {
                edges.push(DependencyEdge {
                    from: prev.clone(),
                    to: id.clone(),
                });
            }
            states.push(StateNode {
                id: id.clone(),
                state: st,
            });
        }

        Ok(StateGraph { states, edges })
    }
}

#[cfg(test)]
#[path = "tests/evm_read_op_tests.rs"]
mod evm_read_op_tests;
