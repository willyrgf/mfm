//! Reusable runtime states for generic EVM contract-set deployment.
//!
//! These states bridge a compiled contract-set manifest into a deployed contract-set manifest
//! without embedding protocol-specific behavior.

use async_trait::async_trait;
use mfm_collectors_rpc_control::EvmIoClient;
use mfm_evm_core::abi as common_abi;
use mfm_machine::context::DynContext;
use mfm_machine::errors::StateError;
use mfm_machine::ids::{ContextKey, StateId};
use mfm_machine::io::IoProvider;
use mfm_machine::meta::StateMeta;
use mfm_machine::recorder::EventRecorder;
use mfm_machine::state::{SnapshotPolicy, State, StateOutcome};
use mfm_state_common::ctx as op_ctx;
use mfm_state_common::errors as op_errors;
use mfm_state_common::idempotency as op_idempotency;
use mfm_state_common::states::meta;
use serde::{Deserialize, Serialize};

use crate::contract_set::{
    decode_compiled_contract_set_manifest, validate_deployed_contract_set_manifest,
    CompiledContractSetManifest, DeployedContractSetEntry, DeployedContractSetManifest,
    DEPLOYED_CONTRACT_SET_KIND,
};
use crate::dcv as shared_dcv;
use crate::rpc as evm_rpc;

const KEY_COMPILED_CONTRACT_SET: &str = "compiled_contract_set";
const KEY_PENDING_DEPLOYMENTS: &str = "contract_set_pending_deployments";
const KEY_DEPLOY_RECEIPTS: &str = "contract_set_deploy_receipts";
const KEY_DEPLOY_MANIFEST: &str = "contract_set_deploy_manifest";

fn default_contract_set_port() -> String {
    "contract_set".to_string()
}

fn default_control_scope() -> String {
    "shared".to_string()
}

fn default_deployer_account_index() -> usize {
    0
}

fn default_poll_interval_ms() -> u64 {
    200
}

fn default_max_receipt_polls() -> u64 {
    120
}

fn default_deploy_manifest_export_key() -> String {
    "deploy_manifest".to_string()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct PendingDeployment {
    id: String,
    tx_hash: String,
    artifact: shared_dcv::ContractArtifactConfig,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct DeploymentReceipt {
    id: String,
    tx_hash: String,
    contract_address: String,
    deploy_receipt: serde_json::Value,
    artifact: shared_dcv::ContractArtifactConfig,
}

/// Runtime configuration for generic contract-set deployment.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EvmDeployContractSetStateConfig {
    /// Stable network identifier targeted by managed RPC calls.
    pub network_id: String,
    /// Stable control-plane scope used to isolate managed source state.
    #[serde(default = "default_control_scope")]
    pub control_scope: String,
    /// Context key that contains the compiled contract-set manifest input.
    #[serde(default = "default_contract_set_port")]
    pub contract_set_port: String,
    /// Account index used when signing through node-managed accounts.
    #[serde(default = "default_deployer_account_index")]
    pub deployer_account_index: usize,
    /// Optional environment variable name that holds a private key for signed deploys.
    #[serde(default)]
    pub signing_key_env: Option<String>,
    /// Delay between receipt polls in milliseconds.
    #[serde(default = "default_poll_interval_ms")]
    pub poll_interval_ms: u64,
    /// Maximum number of receipt polls before timing out.
    #[serde(default = "default_max_receipt_polls")]
    pub max_receipt_polls: u64,
    /// Context key used to export the deployed contract-set manifest.
    #[serde(default = "default_deploy_manifest_export_key")]
    pub deploy_manifest_export_key: String,
}

/// Validates contract-set deployment runtime configuration before planning or execution.
pub fn validate_deploy_contract_set_config(
    cfg: &EvmDeployContractSetStateConfig,
) -> Result<(), String> {
    if cfg.network_id.trim().is_empty() {
        return Err("network_id must be non-empty".to_string());
    }
    if cfg.control_scope.trim().is_empty() {
        return Err("control_scope must be non-empty".to_string());
    }
    if cfg.contract_set_port.trim().is_empty() {
        return Err("contract_set_port must be non-empty".to_string());
    }
    if cfg.deploy_manifest_export_key.trim().is_empty() {
        return Err("deploy_manifest_export_key must be non-empty".to_string());
    }
    if cfg.max_receipt_polls == 0 {
        return Err("max_receipt_polls must be > 0".to_string());
    }
    if cfg
        .signing_key_env
        .as_deref()
        .map(str::trim)
        .unwrap_or("")
        .is_empty()
    {
        return Err("signing_key_env is required for signed contract-set deploys".to_string());
    }
    Ok(())
}

/// State that loads and validates a compiled contract-set manifest from context.
#[derive(Clone, Debug)]
pub struct LoadCompiledContractSetState {
    /// Stable state identifier assigned by the execution plan.
    pub state_id: StateId,
    /// Context key that contains the compiled contract-set manifest JSON value.
    pub contract_set_port: String,
}

/// State that submits deployment transactions for every contract in the compiled manifest.
#[derive(Clone, Debug)]
pub struct DeployContractSetState {
    /// Stable state identifier assigned by the execution plan.
    pub state_id: StateId,
    /// Runtime deployment configuration.
    pub cfg: EvmDeployContractSetStateConfig,
}

/// State that waits for every submitted deployment receipt.
#[derive(Clone, Debug)]
pub struct WaitForContractSetReceiptsState {
    /// Stable state identifier assigned by the execution plan.
    pub state_id: StateId,
    /// Stable managed RPC network identifier used for receipt polling.
    pub network_id: String,
    /// Stable managed RPC control scope used to isolate receipt polling state.
    pub control_scope: String,
    /// Delay between receipt polls in milliseconds.
    pub poll_interval_ms: u64,
    /// Maximum number of receipt polls before timing out.
    pub max_receipt_polls: u64,
}

/// State that converts deployment receipts into a validated deployed contract-set manifest.
#[derive(Clone, Debug)]
pub struct CollectDeployedContractSetState {
    /// Stable state identifier assigned by the execution plan.
    pub state_id: StateId,
}

/// State that writes the deployed contract-set manifest to its exported context key.
#[derive(Clone, Debug)]
pub struct WriteDeployedContractSetState {
    /// Stable state identifier assigned by the execution plan.
    pub state_id: StateId,
    /// Context key used to export the deployed contract-set manifest.
    pub deploy_manifest_export_key: String,
}

#[async_trait]
impl State for LoadCompiledContractSetState {
    fn meta(&self) -> StateMeta {
        meta::config()
    }

    async fn handle(
        &self,
        ctx: &mut dyn DynContext,
        _io: &mut dyn IoProvider,
        _rec: &mut dyn EventRecorder,
    ) -> Result<StateOutcome, StateError> {
        let manifest_value = op_ctx::read_json_required(
            ctx,
            &ContextKey(self.contract_set_port.clone()),
            "missing_compiled_contract_set",
            "missing compiled contract-set manifest in context",
        )?;
        let manifest = decode_compiled_contract_set_manifest(&manifest_value)?;
        op_ctx::write_json(
            ctx,
            ContextKey(KEY_COMPILED_CONTRACT_SET.to_string()),
            serde_json::to_value(manifest).map_err(|_| {
                op_errors::state_unknown(
                    "serialize_compiled_contract_set_failed",
                    "failed to serialize compiled contract-set manifest",
                )
            })?,
        )?;

        Ok(StateOutcome {
            snapshot: SnapshotPolicy::OnSuccess,
        })
    }
}

#[async_trait]
impl State for DeployContractSetState {
    fn meta(&self) -> StateMeta {
        meta::apply_side_effect(op_idempotency::state_purpose(
            "evm_deploy_contract_set",
            &self.state_id,
            "deploy_contract_set",
        ))
    }

    async fn handle(
        &self,
        ctx: &mut dyn DynContext,
        io: &mut dyn IoProvider,
        _rec: &mut dyn EventRecorder,
    ) -> Result<StateOutcome, StateError> {
        let manifest = read_compiled_contract_set(ctx)?;
        let signing_key_env = self.cfg.signing_key_env.as_deref().ok_or_else(|| {
            op_errors::state_unknown(
                "evm_signed_transaction_required",
                "contract-set deploy requires signing_key_env for durable signed transaction intent recording",
            )
        })?;
        let deployer = evm_rpc::resolve_deployer_address_for_network(
            io,
            &self.state_id,
            &self.cfg.network_id,
            &self.cfg.control_scope,
            self.cfg.deployer_account_index,
            Some(signing_key_env),
        )
        .await?;
        let mut next_nonce = evm_rpc::pending_nonce_u128_for_network(
            io,
            &self.state_id,
            &self.cfg.network_id,
            &self.cfg.control_scope,
            &deployer,
        )
        .await?;

        let mut pending = Vec::with_capacity(manifest.contracts.len());
        for contract in manifest.contracts {
            let (abi, bytecode) = shared_dcv::parse_artifact(&contract.artifact).map_err(|_| {
                op_errors::state_unknown(
                    "invalid_contract_artifact",
                    "compiled contract-set artifact was invalid",
                )
            })?;
            let constructor_payload =
                common_abi::constructor_data(&abi, &bytecode, &contract.constructor_args).map_err(
                    |_| {
                        op_errors::state_unknown(
                            "invalid_constructor_args",
                            "constructor args did not match ABI",
                        )
                    },
                )?;
            let contract_id = contract.id;
            let nonce_hex = format!("0x{next_nonce:x}");
            let logical_tx_id = format!("contract_set:{contract_id}");
            let intent = {
                let mut client = EvmIoClient::new(self.state_id.clone(), io);
                evm_rpc::prepare_signed_create_intent_for_network(
                    &mut client,
                    &self.cfg.network_id,
                    &self.cfg.control_scope,
                    &logical_tx_id,
                    signing_key_env,
                    &deployer,
                    &nonce_hex,
                    &constructor_payload,
                    None,
                )
                .await?
            };
            evm_rpc::record_tx_intent(io, &self.state_id, &intent).await?;
            let tx_hash =
                evm_rpc::broadcast_recorded_tx_intent(io, &self.state_id, &intent).await?;
            next_nonce = next_nonce.checked_add(1).ok_or_else(|| {
                op_errors::state_unknown(
                    "evm_response_invalid",
                    "nonce overflow while preparing signed deployment transactions",
                )
            })?;

            pending.push(PendingDeployment {
                id: contract_id,
                tx_hash,
                artifact: contract.artifact,
            });
        }

        op_ctx::write_json(
            ctx,
            ContextKey(KEY_PENDING_DEPLOYMENTS.to_string()),
            serde_json::to_value(&pending).map_err(|_| {
                op_errors::state_unknown(
                    "serialize_pending_contract_set_deployments_failed",
                    "failed to serialize pending contract-set deployments",
                )
            })?,
        )?;

        Ok(StateOutcome {
            snapshot: SnapshotPolicy::OnSuccess,
        })
    }
}

#[async_trait]
impl State for WaitForContractSetReceiptsState {
    fn meta(&self) -> StateMeta {
        meta::fetch_data()
    }

    async fn handle(
        &self,
        ctx: &mut dyn DynContext,
        io: &mut dyn IoProvider,
        _rec: &mut dyn EventRecorder,
    ) -> Result<StateOutcome, StateError> {
        let pending = op_ctx::read_typed::<Vec<PendingDeployment>>(
            ctx,
            &ContextKey(KEY_PENDING_DEPLOYMENTS.to_string()),
            "missing_pending_contract_set_deployments",
            "missing pending contract-set deployments in context",
            "pending_contract_set_deployments_invalid",
            "pending contract-set deployments were invalid",
        )?;

        let mut completed = Vec::with_capacity(pending.len());
        for deployment in pending {
            let receipt = evm_rpc::wait_for_receipt_for_network(
                &self.state_id,
                io,
                &self.network_id,
                &self.control_scope,
                &deployment.tx_hash,
                self.poll_interval_ms,
                self.max_receipt_polls,
            )
            .await?;
            evm_rpc::ensure_receipt_success(&receipt)?;
            let contract_address = evm_rpc::receipt_contract_address(&receipt)?;
            completed.push(DeploymentReceipt {
                id: deployment.id,
                tx_hash: deployment.tx_hash,
                contract_address,
                deploy_receipt: receipt,
                artifact: deployment.artifact,
            });
        }

        op_ctx::write_json(
            ctx,
            ContextKey(KEY_DEPLOY_RECEIPTS.to_string()),
            serde_json::to_value(&completed).map_err(|_| {
                op_errors::state_unknown(
                    "serialize_contract_set_deploy_receipts_failed",
                    "failed to serialize contract-set deployment receipts",
                )
            })?,
        )?;

        Ok(StateOutcome {
            snapshot: SnapshotPolicy::OnSuccess,
        })
    }
}

#[async_trait]
impl State for CollectDeployedContractSetState {
    fn meta(&self) -> StateMeta {
        meta::pure()
    }

    async fn handle(
        &self,
        ctx: &mut dyn DynContext,
        _io: &mut dyn IoProvider,
        _rec: &mut dyn EventRecorder,
    ) -> Result<StateOutcome, StateError> {
        let receipts = op_ctx::read_typed::<Vec<DeploymentReceipt>>(
            ctx,
            &ContextKey(KEY_DEPLOY_RECEIPTS.to_string()),
            "missing_contract_set_deploy_receipts",
            "missing contract-set deployment receipts in context",
            "contract_set_deploy_receipts_invalid",
            "contract-set deployment receipts were invalid",
        )?;

        let contracts = receipts
            .into_iter()
            .map(|receipt| DeployedContractSetEntry {
                id: receipt.id,
                address: receipt.contract_address,
                deploy_tx_hash: receipt.tx_hash,
                deploy_receipt: receipt.deploy_receipt,
                artifact: receipt.artifact,
            })
            .collect::<Vec<_>>();
        let manifest = DeployedContractSetManifest {
            kind: DEPLOYED_CONTRACT_SET_KIND.to_string(),
            contracts,
        };

        validate_deployed_contract_set_manifest(&manifest)?;
        op_ctx::write_json(
            ctx,
            ContextKey(KEY_DEPLOY_MANIFEST.to_string()),
            serde_json::to_value(&manifest).map_err(|_| {
                op_errors::state_unknown(
                    "serialize_contract_set_deploy_manifest_failed",
                    "failed to serialize contract-set deploy manifest",
                )
            })?,
        )?;

        Ok(StateOutcome {
            snapshot: SnapshotPolicy::OnSuccess,
        })
    }
}

#[async_trait]
impl State for WriteDeployedContractSetState {
    fn meta(&self) -> StateMeta {
        meta::pure()
    }

    async fn handle(
        &self,
        ctx: &mut dyn DynContext,
        _io: &mut dyn IoProvider,
        _rec: &mut dyn EventRecorder,
    ) -> Result<StateOutcome, StateError> {
        let manifest = op_ctx::read_json_required(
            ctx,
            &ContextKey(KEY_DEPLOY_MANIFEST.to_string()),
            "missing_contract_set_deploy_manifest",
            "missing contract-set deploy manifest in context",
        )?;
        op_ctx::write_json(
            ctx,
            ContextKey(self.deploy_manifest_export_key.clone()),
            manifest,
        )?;

        Ok(StateOutcome {
            snapshot: SnapshotPolicy::OnSuccess,
        })
    }
}

fn read_compiled_contract_set(
    ctx: &dyn DynContext,
) -> Result<CompiledContractSetManifest, StateError> {
    let value = op_ctx::read_json_required(
        ctx,
        &ContextKey(KEY_COMPILED_CONTRACT_SET.to_string()),
        "missing_compiled_contract_set",
        "missing compiled contract-set manifest in context",
    )?;
    decode_compiled_contract_set_manifest(&value)
}

#[cfg(test)]
mod tests {
    use super::*;

    use mfm_machine::errors::{ErrorCategory, ErrorInfo, IoError};
    use mfm_machine::ids::{ArtifactId, ErrorCode, FactKey};
    use mfm_machine::io::{IoCall, IoResult};
    use mfm_state_common::test_support::MapContext;

    use crate::contract_set::CompiledContractSetEntry;

    fn info(code: &'static str, message: impl Into<String>) -> ErrorInfo {
        ErrorInfo {
            code: ErrorCode(code.to_string()),
            category: ErrorCategory::Unknown,
            retryable: false,
            message: message.into(),
            details: None,
        }
    }

    #[derive(Default)]
    struct TestIo {
        calls: Vec<IoCall>,
        recorded_values: Vec<(FactKey, serde_json::Value)>,
        broadcast_count: usize,
        fail_on_broadcast_number: Option<usize>,
    }

    #[async_trait]
    impl IoProvider for TestIo {
        async fn call(&mut self, call: IoCall) -> Result<IoResult, IoError> {
            self.calls.push(call.clone());

            if call.namespace == "local.evm.signer_address" {
                return Ok(IoResult {
                    response: serde_json::json!({
                        "address": "0x1111111111111111111111111111111111111111",
                    }),
                    recorded_payload_id: None,
                });
            }
            if call.namespace == "local.evm.sign_legacy_create" {
                let raw_tx_hex = match call
                    .request
                    .get("nonce_hex")
                    .and_then(serde_json::Value::as_str)
                {
                    Some("0x0") => "0x01",
                    Some("0x1") => "0x02",
                    _ => "0x03",
                };
                return Ok(IoResult {
                    response: serde_json::json!({ "raw_tx_hex": raw_tx_hex }),
                    recorded_payload_id: None,
                });
            }

            match call.request.get("kind").and_then(serde_json::Value::as_str) {
                Some("evm_call") => {
                    let method = call
                        .request
                        .get("method")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default();
                    let response = match method {
                        "eth_getTransactionCount" => serde_json::json!("0x0"),
                        "eth_estimateGas" => serde_json::json!("0x5208"),
                        "eth_gasPrice" => serde_json::json!("0x1"),
                        "eth_chainId" => serde_json::json!("0x1"),
                        other => {
                            return Err(IoError::Other(info(
                                "unexpected_method",
                                format!("unexpected method `{other}`"),
                            )))
                        }
                    };
                    Ok(IoResult {
                        response,
                        recorded_payload_id: None,
                    })
                }
                Some("evm_broadcast_raw_transaction") => {
                    self.broadcast_count += 1;
                    if self.fail_on_broadcast_number == Some(self.broadcast_count) {
                        return Err(IoError::Other(info(
                            "simulated_broadcast_crash",
                            "simulated crash after prior contract broadcast",
                        )));
                    }
                    Ok(IoResult {
                        response: serde_json::json!({
                            "tx_hash": call.request.get("expected_tx_hash").cloned().unwrap_or(serde_json::Value::Null),
                        }),
                        recorded_payload_id: None,
                    })
                }
                other => Err(IoError::Other(info(
                    "unexpected_call_kind",
                    format!("unexpected call kind `{other:?}`"),
                ))),
            }
        }

        async fn record_value(
            &mut self,
            key: FactKey,
            value: serde_json::Value,
        ) -> Result<ArtifactId, IoError> {
            self.recorded_values.push((key, value));
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

        async fn sleep_ms(&mut self, _duration_ms: u64) -> Result<(), IoError> {
            Ok(())
        }
    }

    struct NoopRecorder;

    #[async_trait]
    impl EventRecorder for NoopRecorder {
        async fn emit(
            &mut self,
            _event: mfm_machine::events::DomainEvent,
        ) -> Result<(), mfm_machine::errors::RunError> {
            Ok(())
        }

        async fn emit_many(
            &mut self,
            _events: Vec<mfm_machine::events::DomainEvent>,
        ) -> Result<(), mfm_machine::errors::RunError> {
            Ok(())
        }
    }

    fn artifact() -> shared_dcv::ContractArtifactConfig {
        serde_json::from_value(serde_json::json!({
            "abi": [],
            "bytecode": {
                "object": "0x6000"
            }
        }))
        .expect("artifact")
    }

    fn compiled_manifest() -> CompiledContractSetManifest {
        CompiledContractSetManifest {
            kind: crate::contract_set::COMPILED_CONTRACT_SET_KIND.to_string(),
            contracts: vec![
                CompiledContractSetEntry {
                    id: "first".to_string(),
                    artifact: artifact(),
                    constructor_args: Vec::new(),
                },
                CompiledContractSetEntry {
                    id: "second".to_string(),
                    artifact: artifact(),
                    constructor_args: Vec::new(),
                },
            ],
        }
    }

    #[test]
    fn contract_set_deploy_config_requires_signing_key_env() {
        let err = validate_deploy_contract_set_config(&EvmDeployContractSetStateConfig {
            network_id: "ethereum-mainnet".to_string(),
            control_scope: "shared".to_string(),
            contract_set_port: "contract_set".to_string(),
            deployer_account_index: 0,
            signing_key_env: None,
            poll_interval_ms: 0,
            max_receipt_polls: 1,
            deploy_manifest_export_key: "deploy_manifest".to_string(),
        })
        .expect_err("unsigned contract-set deploy must be rejected");

        assert!(err.contains("signing_key_env"));
    }

    #[tokio::test]
    async fn contract_set_retry_after_first_broadcast_keeps_first_tx_intent_stable() {
        let mut ctx = MapContext::default();
        op_ctx::write_json(
            &mut ctx,
            ContextKey(KEY_COMPILED_CONTRACT_SET.to_string()),
            serde_json::to_value(compiled_manifest()).expect("manifest json"),
        )
        .expect("write compiled manifest");
        let state = DeployContractSetState {
            state_id: StateId::must_new("evm.contract_set.deploy".to_string()),
            cfg: EvmDeployContractSetStateConfig {
                network_id: "ethereum-mainnet".to_string(),
                control_scope: "shared".to_string(),
                contract_set_port: "contract_set".to_string(),
                deployer_account_index: 0,
                signing_key_env: Some("MFM_DEPLOYER_KEY".to_string()),
                poll_interval_ms: 0,
                max_receipt_polls: 1,
                deploy_manifest_export_key: "deploy_manifest".to_string(),
            },
        };
        let mut io = TestIo {
            fail_on_broadcast_number: Some(2),
            ..TestIo::default()
        };
        let mut rec = NoopRecorder;

        let err = state
            .handle(&mut ctx, &mut io, &mut rec)
            .await
            .expect_err("second broadcast simulates a crash");
        assert_eq!(err.info.code.0, "simulated_broadcast_crash");
        let first_attempt_first_intent = io
            .recorded_values
            .first()
            .expect("first intent recorded")
            .1
            .clone();

        io.fail_on_broadcast_number = None;
        state
            .handle(&mut ctx, &mut io, &mut rec)
            .await
            .expect("retry should complete");
        let retry_first_intent = io
            .recorded_values
            .get(2)
            .expect("first retry intent recorded")
            .1
            .clone();

        assert_eq!(first_attempt_first_intent, retry_first_intent);
    }
}
