#![cfg_attr(test, allow(clippy::disallowed_methods, clippy::disallowed_types))]
#![cfg_attr(not(test), deny(clippy::disallowed_methods, clippy::disallowed_types))]
#![warn(missing_docs)]
//! EVM write/validation operations.
//!
//! This crate provides reusable EVM operations that can be composed as a pipeline:
//! - `evm_deploy`   : deploy a contract from an artifact
//! - `evm_configure`: execute post-deploy contract calls
//! - `evm_validate` : enforce chain/client/read/event assertions
//! - `evm_contract_from_nix`: adapt `nix_app` output into an EVM artifact export
//! - `evm_deploy_contract_set`: deploy a compiled contract-set manifest through generic runtime states
//!
//! Notes:
//! - All network interaction flows through `namespace = "evm"` IO.
//! - No secrets are persisted in op_config or outputs.
//!
//! # Examples
//!
//! ```rust
//! use mfm_op_evm_write::EvmDeployOp;
//! use mfm_sdk::op::Operation;
//!
//! let op = EvmDeployOp;
//! assert_eq!(op.op_id().as_str(), "evm_deploy");
//! ```

use std::sync::Arc;

use serde::Deserialize;
#[cfg(test)]
use std::time::Duration;

use mfm_evm_dcv_model as shared_dcv;
use mfm_evm_runtime::states::contract_set::{
    validate_deploy_contract_set_config, CollectDeployedContractSetState,
    DeployContractSetState as SharedDeployContractSetState,
    EvmDeployContractSetStateConfig as SharedDeployContractSetStateConfig,
    LoadCompiledContractSetState, WaitForContractSetReceiptsState, WriteDeployedContractSetState,
};
use mfm_evm_runtime::states::write::{
    EvmConfigureRuntimeCall as SharedConfigureRuntimeCall,
    EvmConfigureState as SharedConfigureState,
    EvmConfigureStateConfig as SharedConfigureStateConfig, EvmDeployState as SharedDeployState,
    EvmDeployStateConfig as SharedDeployStateConfig, EvmValidateState as SharedValidateState,
    EvmValidateStateConfig as SharedValidateStateConfig, NixArtifactToEvmContractState,
};
use mfm_machine::config::RunConfig;
use mfm_machine::errors::ErrorCategory;
use mfm_machine::ids::{OpId, OpPath};
use mfm_state_common::errors as op_errors;
use mfm_state_common::rpc as op_rpc;

use mfm_sdk::errors::SdkError;
use mfm_sdk::ids::PortKey;
use mfm_sdk::op::{
    leaf_state_id, leaf_state_node, LeafOpSpec, OpInterface, Operation, PlannedOp, PlannedOpKind,
};

const OP_ID_CONTRACT_FROM_NIX: &str = "evm_contract_from_nix";
const OP_ID_DEPLOY_CONTRACT_SET: &str = "evm_deploy_contract_set";
const OP_ID_DEPLOY: &str = "evm_deploy";
const OP_ID_CONFIGURE: &str = "evm_configure";
const OP_ID_VALIDATE: &str = "evm_validate";
const OP_VERSION: &str = "v1";

const KEY_NIX_RESULT: &str = "result";
const KEY_CONTRACT_ARTIFACT: &str = "contract_artifact";
const KEY_CONTRACT_SET: &str = "contract_set";
const KEY_CONTRACT_ADDRESS: &str = "contract_address";
const KEY_DEPLOY_TX_HASH: &str = "deploy_tx_hash";
const KEY_DEPLOY_RECEIPT: &str = "deploy_receipt";

const KEY_CONFIGURE_TX_HASHES: &str = "configure_tx_hashes";
const KEY_CONFIGURE_RECEIPTS: &str = "configure_receipts";

const KEY_VALIDATED: &str = "validated";
const KEY_CHAIN_ID: &str = "chain_id";
const KEY_CLIENT_VERSION: &str = "client_version";

fn default_poll_interval_ms() -> u64 {
    500
}

fn default_max_receipt_polls() -> u64 {
    120
}

fn default_require_client_substring() -> String {
    "reth".to_string()
}

fn default_min_count() -> u64 {
    1
}

fn default_result_pointer() -> String {
    "/artifact".to_string()
}

fn default_artifact_port() -> String {
    KEY_CONTRACT_ARTIFACT.to_string()
}

fn default_contract_set_port() -> String {
    KEY_CONTRACT_SET.to_string()
}

fn default_control_scope() -> String {
    "shared".to_string()
}

fn default_configure_tx_hashes_export_key() -> String {
    KEY_CONFIGURE_TX_HASHES.to_string()
}

fn default_configure_receipts_export_key() -> String {
    KEY_CONFIGURE_RECEIPTS.to_string()
}

fn default_deploy_manifest_export_key() -> String {
    "deploy_manifest".to_string()
}

#[derive(Clone, Debug, Deserialize)]
struct ContractArtifactConfig {
    abi: serde_json::Value,
    bytecode: serde_json::Value,
}

#[derive(Clone, Debug, Deserialize)]
struct EvmContractFromNixConfig {
    #[serde(default = "default_result_pointer")]
    result_pointer: String,
}

#[derive(Clone, Debug, Deserialize)]
struct EvmDeployConfig {
    #[serde(default)]
    artifact: Option<ContractArtifactConfig>,

    #[serde(default = "default_artifact_port")]
    artifact_port: String,

    network_id: String,

    #[serde(default = "default_control_scope")]
    control_scope: String,

    from: String,

    #[serde(default)]
    constructor_args: Vec<serde_json::Value>,

    #[serde(default)]
    value_wei: Option<String>,

    #[serde(default)]
    signing_key_env: Option<String>,

    #[serde(default = "default_poll_interval_ms")]
    poll_interval_ms: u64,

    #[serde(default = "default_max_receipt_polls")]
    max_receipt_polls: u64,
}

#[derive(Clone, Debug, Deserialize)]
struct EvmDeployContractSetConfig {
    #[serde(default = "default_contract_set_port")]
    contract_set_port: String,

    network_id: String,

    #[serde(default = "default_control_scope")]
    control_scope: String,

    #[serde(default)]
    deployer_account_index: usize,

    #[serde(default)]
    signing_key_env: Option<String>,

    #[serde(default = "default_poll_interval_ms")]
    poll_interval_ms: u64,

    #[serde(default = "default_max_receipt_polls")]
    max_receipt_polls: u64,

    #[serde(default = "default_deploy_manifest_export_key")]
    deploy_manifest_export_key: String,
}

#[derive(Clone, Debug, Deserialize)]
struct ConfigureCallConfig {
    function: String,

    #[serde(default)]
    args: Vec<serde_json::Value>,

    #[serde(default)]
    value_wei: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
struct EvmConfigureConfig {
    #[serde(default)]
    artifact: Option<ContractArtifactConfig>,

    #[serde(default = "default_artifact_port")]
    artifact_port: String,

    network_id: String,

    #[serde(default = "default_control_scope")]
    control_scope: String,

    from: String,

    #[serde(default)]
    signing_key_env: Option<String>,

    #[serde(default)]
    contract_address: Option<String>,

    calls: Vec<ConfigureCallConfig>,

    #[serde(default = "default_configure_tx_hashes_export_key")]
    tx_hashes_export_key: String,

    #[serde(default = "default_configure_receipts_export_key")]
    receipts_export_key: String,

    #[serde(default = "default_poll_interval_ms")]
    poll_interval_ms: u64,

    #[serde(default = "default_max_receipt_polls")]
    max_receipt_polls: u64,
}

#[derive(Clone, Debug, Deserialize)]
struct ReadAssertionConfig {
    function: String,

    #[serde(default)]
    args: Vec<serde_json::Value>,

    expected: serde_json::Value,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
enum BlockTag {
    Number(u64),
    Tag(String),
}

#[derive(Clone, Debug, Deserialize)]
struct EventAssertionConfig {
    event: String,

    #[serde(default = "default_min_count")]
    min_count: u64,

    #[serde(default)]
    from_block: Option<BlockTag>,

    #[serde(default)]
    to_block: Option<BlockTag>,
}

#[derive(Clone, Debug, Deserialize)]
struct EvmValidateConfig {
    #[serde(default)]
    artifact: Option<ContractArtifactConfig>,

    #[serde(default = "default_artifact_port")]
    artifact_port: String,

    network_id: String,

    #[serde(default = "default_control_scope")]
    control_scope: String,

    #[serde(default)]
    contract_address: Option<String>,

    expected_chain_id: u64,

    #[serde(default = "default_require_client_substring")]
    require_client_substring: String,

    #[serde(default)]
    read_assertions: Vec<ReadAssertionConfig>,

    #[serde(default)]
    event_assertions: Vec<EventAssertionConfig>,
}

fn shared_artifact_config(artifact: &ContractArtifactConfig) -> shared_dcv::ContractArtifactConfig {
    shared_dcv::ContractArtifactConfig {
        abi: artifact.abi.clone().into(),
        bytecode: artifact.bytecode.clone().into(),
    }
}

fn into_shared_artifact_config(
    artifact: ContractArtifactConfig,
) -> shared_dcv::ContractArtifactConfig {
    shared_dcv::ContractArtifactConfig {
        abi: artifact.abi.into(),
        bytecode: artifact.bytecode.into(),
    }
}

fn shared_block_tag(block: BlockTag) -> shared_dcv::BlockTag {
    match block {
        BlockTag::Number(n) => shared_dcv::BlockTag::Number(n),
        BlockTag::Tag(s) => shared_dcv::BlockTag::Tag(s),
    }
}

fn shared_read_assertion_config(
    assertion: &ReadAssertionConfig,
) -> shared_dcv::ReadAssertionConfig {
    shared_dcv::ReadAssertionConfig {
        function: assertion.function.clone(),
        args: assertion.args.iter().cloned().map(Into::into).collect(),
        expected: assertion.expected.clone().into(),
    }
}

fn shared_event_assertion_config(
    assertion: &EventAssertionConfig,
) -> shared_dcv::EventAssertionConfig {
    shared_dcv::EventAssertionConfig {
        event: assertion.event.clone(),
        min_count: assertion.min_count,
        from_block: assertion.from_block.clone().map(shared_block_tag),
        to_block: assertion.to_block.clone().map(shared_block_tag),
    }
}

fn ensure_nonempty_env_name(env_name: &str) -> Result<(), String> {
    if env_name.trim().is_empty() {
        return Err("signing_key_env must be non-empty".to_string());
    }
    Ok(())
}

/// Planner that adapts a `nix_app` result into a contract artifact export.
#[derive(Clone, Default)]
pub struct EvmContractFromNixOp;

impl Operation for EvmContractFromNixOp {
    fn op_id(&self) -> OpId {
        OpId::must_new(OP_ID_CONTRACT_FROM_NIX.to_string())
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
        let cfg: EvmContractFromNixConfig =
            serde_json::from_value(op_config.clone()).map_err(|_| {
                op_errors::sdk_parse_error(
                    "invalid_op_config",
                    "invalid evm_contract_from_nix op_config",
                )
            })?;
        if !cfg.result_pointer.is_empty() && !cfg.result_pointer.starts_with('/') {
            return Err(op_errors::sdk_parse_error(
                "invalid_op_config",
                "result_pointer must be empty or start with '/'",
            ));
        }

        let state = Arc::new(NixArtifactToEvmContractState {
            result_pointer: cfg.result_pointer,
        });

        Ok(PlannedOp {
            interface: OpInterface {
                imports: vec![PortKey(KEY_NIX_RESULT.to_string())],
                exports: vec![PortKey(KEY_CONTRACT_ARTIFACT.to_string())],
            },
            kind: PlannedOpKind::Leaf(LeafOpSpec {
                states: vec![leaf_state_node(&op_path, "adapt", state)?],
                edges: Vec::new(),
            }),
        })
    }
}

/// Planner that deploys a compiled EVM contract-set manifest.
#[derive(Clone, Default)]
pub struct EvmDeployContractSetOp;

impl Operation for EvmDeployContractSetOp {
    fn op_id(&self) -> OpId {
        OpId::must_new(OP_ID_DEPLOY_CONTRACT_SET.to_string())
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
        let cfg: EvmDeployContractSetConfig =
            serde_json::from_value(op_config.clone()).map_err(|_| {
                op_errors::sdk_parse_error(
                    "invalid_op_config",
                    "invalid evm_deploy_contract_set op_config",
                )
            })?;

        validate_deploy_contract_set_config(&SharedDeployContractSetStateConfig {
            network_id: cfg.network_id.clone(),
            control_scope: cfg.control_scope.clone(),
            contract_set_port: cfg.contract_set_port.clone(),
            deployer_account_index: cfg.deployer_account_index,
            signing_key_env: cfg.signing_key_env.clone(),
            poll_interval_ms: cfg.poll_interval_ms,
            max_receipt_polls: cfg.max_receipt_polls,
            deploy_manifest_export_key: cfg.deploy_manifest_export_key.clone(),
        })
        .map_err(|msg| {
            op_errors::sdk_error(
                "invalid_op_config",
                mfm_machine::errors::ErrorCategory::ParsingInput,
                false,
                msg,
            )
        })?;
        let Some(env_name) = cfg.signing_key_env.as_deref() else {
            return Err(op_errors::sdk_parse_error(
                "invalid_op_config",
                "signing_key_env is required for evm_deploy_contract_set",
            ));
        };
        ensure_nonempty_env_name(env_name).map_err(|_| {
            op_errors::sdk_parse_error("invalid_op_config", "signing_key_env must be non-empty")
        })?;
        let contract_set_port = cfg.contract_set_port.clone();
        let network_id = cfg.network_id.clone();
        let control_scope = cfg.control_scope.clone();
        let poll_interval_ms = cfg.poll_interval_ms;
        let max_receipt_polls = cfg.max_receipt_polls;
        let deploy_manifest_export_key = cfg.deploy_manifest_export_key.clone();

        let load_state_id = leaf_state_id(&op_path, "load_contract_set")?;
        let deploy_state_id = leaf_state_id(&op_path, "deploy_contract_set")?;
        let wait_state_id = leaf_state_id(&op_path, "wait_for_receipts")?;
        let collect_state_id = leaf_state_id(&op_path, "collect_deploy_manifest")?;
        let write_state_id = leaf_state_id(&op_path, "write_deploy_manifest")?;

        Ok(PlannedOp {
            interface: OpInterface {
                imports: vec![PortKey(contract_set_port.clone())],
                exports: vec![PortKey(deploy_manifest_export_key.clone())],
            },
            kind: PlannedOpKind::Leaf(LeafOpSpec {
                states: vec![
                    leaf_state_node(
                        &op_path,
                        "load_contract_set",
                        Arc::new(LoadCompiledContractSetState {
                            state_id: load_state_id.clone(),
                            contract_set_port,
                        }),
                    )?,
                    leaf_state_node(
                        &op_path,
                        "deploy_contract_set",
                        Arc::new(SharedDeployContractSetState {
                            state_id: deploy_state_id.clone(),
                            cfg: SharedDeployContractSetStateConfig {
                                network_id,
                                control_scope: control_scope.clone(),
                                contract_set_port: cfg.contract_set_port,
                                deployer_account_index: cfg.deployer_account_index,
                                signing_key_env: cfg.signing_key_env,
                                poll_interval_ms,
                                max_receipt_polls,
                                deploy_manifest_export_key: deploy_manifest_export_key.clone(),
                            },
                        }),
                    )?,
                    leaf_state_node(
                        &op_path,
                        "wait_for_receipts",
                        Arc::new(WaitForContractSetReceiptsState {
                            state_id: wait_state_id.clone(),
                            network_id: cfg.network_id,
                            control_scope,
                            poll_interval_ms,
                            max_receipt_polls,
                        }),
                    )?,
                    leaf_state_node(
                        &op_path,
                        "collect_deploy_manifest",
                        Arc::new(CollectDeployedContractSetState {
                            state_id: collect_state_id.clone(),
                        }),
                    )?,
                    leaf_state_node(
                        &op_path,
                        "write_deploy_manifest",
                        Arc::new(WriteDeployedContractSetState {
                            state_id: write_state_id.clone(),
                            deploy_manifest_export_key,
                        }),
                    )?,
                ],
                edges: vec![
                    mfm_machine::plan::DependencyEdge {
                        from: load_state_id,
                        to: deploy_state_id.clone(),
                    },
                    mfm_machine::plan::DependencyEdge {
                        from: deploy_state_id,
                        to: wait_state_id.clone(),
                    },
                    mfm_machine::plan::DependencyEdge {
                        from: wait_state_id,
                        to: collect_state_id.clone(),
                    },
                    mfm_machine::plan::DependencyEdge {
                        from: collect_state_id,
                        to: write_state_id,
                    },
                ],
            }),
        })
    }
}

/// Planner that deploys an EVM contract artifact.
#[derive(Clone, Default)]
pub struct EvmDeployOp;

impl Operation for EvmDeployOp {
    fn op_id(&self) -> OpId {
        OpId::must_new(OP_ID_DEPLOY.to_string())
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
        let cfg: EvmDeployConfig = serde_json::from_value(op_config.clone()).map_err(|_| {
            op_errors::sdk_parse_error("invalid_op_config", "invalid evm_deploy op_config")
        })?;

        shared_dcv::ensure_nonempty_artifact_port(&cfg.artifact_port).map_err(|_| {
            op_errors::sdk_parse_error("invalid_op_config", "artifact_port must be non-empty")
        })?;
        let Some(env_name) = cfg.signing_key_env.as_deref() else {
            return Err(op_errors::sdk_parse_error(
                "invalid_op_config",
                "signing_key_env is required for evm_deploy",
            ));
        };
        ensure_nonempty_env_name(env_name).map_err(|_| {
            op_errors::sdk_parse_error("invalid_op_config", "signing_key_env must be non-empty")
        })?;
        if let Some(artifact) = &cfg.artifact {
            let shared_artifact = shared_artifact_config(artifact);
            let (abi, bytecode) = shared_dcv::parse_artifact(&shared_artifact).map_err(|err| {
                op_errors::sdk_error(
                    "invalid_op_config",
                    ErrorCategory::ParsingInput,
                    false,
                    format!("invalid contract artifact: {err}"),
                )
            })?;
            shared_dcv::constructor_data(&abi, &bytecode, &cfg.constructor_args).map_err(
                |err| {
                    op_errors::sdk_error(
                        "invalid_op_config",
                        ErrorCategory::ParsingInput,
                        false,
                        format!("constructor args did not match ABI: {err}"),
                    )
                },
            )?;
        }

        let from = shared_dcv::normalize_address(&cfg.from)
            .map_err(|_| op_errors::sdk_parse_error("invalid_op_config", "invalid from address"))?;

        shared_dcv::ensure_nonzero_polls(cfg.max_receipt_polls).map_err(|_| {
            op_errors::sdk_parse_error("invalid_op_config", "max_receipt_polls must be > 0")
        })?;

        let value_hex = shared_dcv::parse_value_wei_to_hex(&cfg.value_wei)
            .map_err(|_| op_errors::sdk_parse_error("invalid_op_config", "invalid value_wei"))?;
        let artifact_from_port = cfg.artifact.is_none();
        let artifact_port = cfg.artifact_port.clone();

        let state_id = leaf_state_id(&op_path, "deploy")?;
        let state = Arc::new(SharedDeployState {
            state_id: state_id.clone(),
            cfg: SharedDeployStateConfig {
                artifact: cfg.artifact.map(into_shared_artifact_config),
                artifact_port: cfg.artifact_port,
                network_id: cfg.network_id,
                control_scope: cfg.control_scope,
                from,
                constructor_args: cfg.constructor_args.into_iter().map(Into::into).collect(),
                value_hex,
                signing_key_env: cfg.signing_key_env,
                poll_interval_ms: cfg.poll_interval_ms,
                max_receipt_polls: cfg.max_receipt_polls,
            },
        });

        let imports = if artifact_from_port {
            vec![PortKey(artifact_port)]
        } else {
            Vec::new()
        };

        Ok(PlannedOp {
            interface: OpInterface {
                imports,
                exports: vec![
                    PortKey(KEY_CONTRACT_ADDRESS.to_string()),
                    PortKey(KEY_DEPLOY_TX_HASH.to_string()),
                    PortKey(KEY_DEPLOY_RECEIPT.to_string()),
                ],
            },
            kind: PlannedOpKind::Leaf(LeafOpSpec {
                states: vec![leaf_state_node(&op_path, "deploy", state)?],
                edges: Vec::new(),
            }),
        })
    }
}

/// Planner that executes post-deploy contract calls.
#[derive(Clone, Default)]
pub struct EvmConfigureOp;

impl Operation for EvmConfigureOp {
    fn op_id(&self) -> OpId {
        OpId::must_new(OP_ID_CONFIGURE.to_string())
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
        let cfg: EvmConfigureConfig = serde_json::from_value(op_config.clone()).map_err(|_| {
            op_errors::sdk_parse_error("invalid_op_config", "invalid evm_configure op_config")
        })?;

        shared_dcv::ensure_nonempty_artifact_port(&cfg.artifact_port).map_err(|_| {
            op_errors::sdk_parse_error("invalid_op_config", "artifact_port must be non-empty")
        })?;

        let Some(env_name) = cfg.signing_key_env.as_deref() else {
            return Err(op_errors::sdk_parse_error(
                "invalid_op_config",
                "signing_key_env is required for evm_configure",
            ));
        };
        ensure_nonempty_env_name(env_name).map_err(|_| {
            op_errors::sdk_parse_error("invalid_op_config", "signing_key_env must be non-empty")
        })?;

        if cfg.calls.is_empty() {
            return Err(op_errors::sdk_parse_error(
                "invalid_op_config",
                "evm_configure requires at least one call",
            ));
        }
        if cfg.tx_hashes_export_key.trim().is_empty() {
            return Err(op_errors::sdk_parse_error(
                "invalid_op_config",
                "tx_hashes_export_key must be non-empty",
            ));
        }
        if cfg.receipts_export_key.trim().is_empty() {
            return Err(op_errors::sdk_parse_error(
                "invalid_op_config",
                "receipts_export_key must be non-empty",
            ));
        }

        let parsed_abi = if let Some(artifact) = &cfg.artifact {
            let shared_artifact = shared_artifact_config(artifact);
            let (abi, _bytecode) = shared_dcv::parse_artifact(&shared_artifact).map_err(|err| {
                op_errors::sdk_error(
                    "invalid_op_config",
                    ErrorCategory::ParsingInput,
                    false,
                    format!("invalid contract artifact: {err}"),
                )
            })?;
            Some(abi)
        } else {
            None
        };

        let from = shared_dcv::normalize_address(&cfg.from)
            .map_err(|_| op_errors::sdk_parse_error("invalid_op_config", "invalid from address"))?;

        let contract_address = cfg
            .contract_address
            .as_ref()
            .map(|a| shared_dcv::normalize_address(a))
            .transpose()
            .map_err(|_| {
                op_errors::sdk_parse_error("invalid_op_config", "invalid contract_address")
            })?;

        shared_dcv::ensure_nonzero_polls(cfg.max_receipt_polls).map_err(|_| {
            op_errors::sdk_parse_error("invalid_op_config", "max_receipt_polls must be > 0")
        })?;
        let artifact_from_port = cfg.artifact.is_none();
        let contract_address_from_port = cfg.contract_address.is_none();
        let artifact_port = cfg.artifact_port.clone();

        let mut calls = Vec::with_capacity(cfg.calls.len());
        for c in &cfg.calls {
            if let Some(abi) = &parsed_abi {
                let _ = shared_dcv::resolve_function_call(abi, &c.function, &c.args).map_err(
                    |err| {
                        op_errors::sdk_error(
                            "invalid_op_config",
                            ErrorCategory::ParsingInput,
                            false,
                            format!("configure call did not match ABI: {err}"),
                        )
                    },
                )?;
            }
            let value_hex = shared_dcv::parse_value_wei_to_hex(&c.value_wei).map_err(|_| {
                op_errors::sdk_parse_error(
                    "invalid_op_config",
                    "invalid value_wei in configure call",
                )
            })?;

            calls.push(SharedConfigureRuntimeCall {
                function: c.function.clone(),
                args: c.args.iter().cloned().map(Into::into).collect(),
                value_hex,
            });
        }

        let state_id = leaf_state_id(&op_path, "configure")?;
        let state = Arc::new(SharedConfigureState {
            state_id: state_id.clone(),
            cfg: SharedConfigureStateConfig {
                artifact: cfg.artifact.map(into_shared_artifact_config),
                artifact_port: cfg.artifact_port,
                network_id: cfg.network_id,
                control_scope: cfg.control_scope,
                from,
                signing_key_env: cfg.signing_key_env,
                contract_address,
                calls,
                tx_hashes_export_key: cfg.tx_hashes_export_key.clone(),
                receipts_export_key: cfg.receipts_export_key.clone(),
                poll_interval_ms: cfg.poll_interval_ms,
                max_receipt_polls: cfg.max_receipt_polls,
            },
        });

        let mut imports = Vec::new();
        if artifact_from_port {
            imports.push(PortKey(artifact_port));
        }
        if contract_address_from_port {
            imports.push(PortKey(KEY_CONTRACT_ADDRESS.to_string()));
        }

        Ok(PlannedOp {
            interface: OpInterface {
                imports,
                exports: vec![
                    PortKey(cfg.tx_hashes_export_key),
                    PortKey(cfg.receipts_export_key),
                ],
            },
            kind: PlannedOpKind::Leaf(LeafOpSpec {
                states: vec![leaf_state_node(&op_path, "configure", state)?],
                edges: Vec::new(),
            }),
        })
    }
}

/// Planner that validates deployed contracts and chain assumptions.
#[derive(Clone, Default)]
pub struct EvmValidateOp;

impl Operation for EvmValidateOp {
    fn op_id(&self) -> OpId {
        OpId::must_new(OP_ID_VALIDATE.to_string())
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
        let cfg: EvmValidateConfig = serde_json::from_value(op_config.clone()).map_err(|_| {
            op_errors::sdk_parse_error("invalid_op_config", "invalid evm_validate op_config")
        })?;

        shared_dcv::ensure_nonempty_artifact_port(&cfg.artifact_port).map_err(|_| {
            op_errors::sdk_parse_error("invalid_op_config", "artifact_port must be non-empty")
        })?;

        let parsed_abi = if let Some(artifact) = &cfg.artifact {
            let shared_artifact = shared_artifact_config(artifact);
            let (abi, _bytecode) = shared_dcv::parse_artifact(&shared_artifact).map_err(|err| {
                op_errors::sdk_error(
                    "invalid_op_config",
                    ErrorCategory::ParsingInput,
                    false,
                    format!("invalid contract artifact: {err}"),
                )
            })?;
            Some(abi)
        } else {
            None
        };

        let contract_address = cfg
            .contract_address
            .as_ref()
            .map(|a| shared_dcv::normalize_address(a))
            .transpose()
            .map_err(|_| {
                op_errors::sdk_parse_error("invalid_op_config", "invalid contract_address")
            })?;

        if cfg.require_client_substring.trim().is_empty() {
            return Err(op_errors::sdk_parse_error(
                "invalid_op_config",
                "require_client_substring must be non-empty",
            ));
        }

        if let Some(abi) = &parsed_abi {
            let read_assertions: Vec<_> = cfg
                .read_assertions
                .iter()
                .map(shared_read_assertion_config)
                .collect();
            let event_assertions: Vec<_> = cfg
                .event_assertions
                .iter()
                .map(shared_event_assertion_config)
                .collect();
            shared_dcv::prepare_validate_assertions(abi, &read_assertions, &event_assertions)
                .map_err(|err| {
                    op_errors::sdk_parse_error(
                        "invalid_op_config",
                        op_rpc::validation_assertion_error_message(&err),
                    )
                })?;
        }
        let artifact_from_port = cfg.artifact.is_none();
        let contract_address_from_port = cfg.contract_address.is_none();
        let artifact_port = cfg.artifact_port.clone();

        let state_id = leaf_state_id(&op_path, "validate")?;
        let state = Arc::new(SharedValidateState {
            state_id: state_id.clone(),
            cfg: SharedValidateStateConfig {
                artifact: cfg.artifact.map(into_shared_artifact_config),
                artifact_port: cfg.artifact_port,
                network_id: cfg.network_id,
                control_scope: cfg.control_scope,
                contract_address,
                expected_chain_id: cfg.expected_chain_id,
                require_client_substring: cfg.require_client_substring,
                read_assertions: cfg
                    .read_assertions
                    .into_iter()
                    .map(|a| shared_dcv::ReadAssertionConfig {
                        function: a.function,
                        args: a.args.into_iter().map(Into::into).collect(),
                        expected: a.expected.into(),
                    })
                    .collect(),
                event_assertions: cfg
                    .event_assertions
                    .into_iter()
                    .map(|a| shared_dcv::EventAssertionConfig {
                        event: a.event,
                        min_count: a.min_count,
                        from_block: a.from_block.map(shared_block_tag),
                        to_block: a.to_block.map(shared_block_tag),
                    })
                    .collect(),
            },
        });

        let mut imports = Vec::new();
        if artifact_from_port {
            imports.push(PortKey(artifact_port));
        }
        if contract_address_from_port {
            imports.push(PortKey(KEY_CONTRACT_ADDRESS.to_string()));
        }

        Ok(PlannedOp {
            interface: OpInterface {
                imports,
                exports: vec![
                    PortKey(KEY_VALIDATED.to_string()),
                    PortKey(KEY_CHAIN_ID.to_string()),
                    PortKey(KEY_CLIENT_VERSION.to_string()),
                ],
            },
            kind: PlannedOpKind::Leaf(LeafOpSpec {
                states: vec![leaf_state_node(&op_path, "validate", state)?],
                edges: Vec::new(),
            }),
        })
    }
}

#[cfg(test)]
#[path = "tests/evm_write_op_tests.rs"]
mod evm_write_op_tests;
