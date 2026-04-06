#![cfg_attr(test, allow(clippy::disallowed_methods, clippy::disallowed_types))]
#![cfg_attr(not(test), deny(clippy::disallowed_methods, clippy::disallowed_types))]
#![warn(missing_docs)]
//! Nix-app execution op.
//!
//! Source of truth: `docs/design.md` (v4), especially the Replay/IO contract.
//!
//! This op expands into a single state that requests external execution via `namespace="exec"`.
//!
//! # Examples
//!
//! ```rust
//! use mfm_op_nix_app::NixAppOp;
//! use mfm_sdk::op::Operation;
//!
//! let op = NixAppOp;
//! assert_eq!(op.op_id().as_str(), "nix_app");
//! ```

use std::sync::Arc;

#[cfg(test)]
use async_trait::async_trait;

use mfm_machine::config::RunConfig;
use mfm_machine::errors::ErrorCategory;
use mfm_machine::ids::{OpId, OpPath};
use mfm_state_common::errors as op_errors;
use mfm_state_common::states::nix::{validate_nix_exec_config, NixExecState, NixExecStateConfig};

use mfm_sdk::errors::SdkError;
use mfm_sdk::ids::PortKey;
use mfm_sdk::op::{
    leaf_state_id, leaf_state_node, LeafOpSpec, OpInterface, Operation, PlannedOp, PlannedOpKind,
};

const OP_ID: &str = "nix_app";
const OP_VERSION: &str = "v1";

/// Thin planner op that validates nix execution config and expands to a single shared exec state.
#[derive(Clone, Default)]
pub struct NixAppOp;

impl Operation for NixAppOp {
    fn op_id(&self) -> OpId {
        OpId::must_new(OP_ID.to_string())
    }

    fn op_version(&self) -> String {
        OP_VERSION.to_string()
    }

    fn expand(
        &self,
        op_path: OpPath,
        op_config: &serde_json::Value,
        _run_config: &RunConfig,
    ) -> Result<PlannedOp, SdkError> {
        let cfg: NixExecStateConfig = serde_json::from_value(op_config.clone()).map_err(|_| {
            op_errors::sdk_unknown_error("invalid_op_config", "invalid nix_app op_config")
        })?;
        validate_nix_exec_config(&cfg).map_err(|msg| {
            op_errors::sdk_error("invalid_op_config", ErrorCategory::Unknown, false, msg)
        })?;

        let sid = leaf_state_id(&op_path, "run")?;
        let export_key = cfg.write_result_to.clone();
        let state = Arc::new(NixExecState {
            state_id: sid.clone(),
            cfg,
        });
        let imports = state
            .cfg
            .stdin_json_port
            .as_ref()
            .map(|port| vec![PortKey(port.clone())])
            .unwrap_or_default();

        Ok(PlannedOp {
            interface: OpInterface {
                imports,
                exports: vec![PortKey(export_key)],
            },
            kind: PlannedOpKind::Leaf(LeafOpSpec {
                states: vec![leaf_state_node(&op_path, "run", state)?],
                edges: Vec::new(),
            }),
        })
    }
}

#[cfg(test)]
#[path = "tests/nix_app_op_tests.rs"]
mod nix_app_op_tests;
