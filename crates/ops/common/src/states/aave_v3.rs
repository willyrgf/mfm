use async_trait::async_trait;
use mfm_collectors_evm::EvmIoClient;
use mfm_machine::context::DynContext;
use mfm_machine::errors::{ErrorCategory, StateError};
use mfm_machine::ids::{ContextKey, StateId};
use mfm_machine::io::IoProvider;
use mfm_machine::meta::StateMeta;
use mfm_machine::recorder::EventRecorder;
use mfm_machine::state::{SnapshotPolicy, State, StateOutcome};
use serde::{Deserialize, Serialize};

use crate::abi as common_abi;
use crate::ctx as op_ctx;
use crate::errors as op_errors;
use crate::evm_dcv as shared_dcv;
use crate::evm_rpc;
use crate::idempotency as op_idempotency;
use crate::states::meta;

pub use crate::aave_v3_manifest::{
    contract_from_manifest, decode_compile_manifest, decode_deploy_manifest,
    decode_origin_deploy_output, origin_contract_from_output, validate_compile_manifest,
    validate_configure_runtime_config, validate_deploy_manifest, validate_deploy_runtime_config,
    validate_origin_deploy_output, AaveCompileManifest, AaveCompileManifestContract,
    AaveConfigCallRecord, AaveConfigReport, AaveConfigureRuntimeConfig, AaveDeployManifest,
    AaveDeployManifestContract, AaveDeployRuntimeConfig, AaveOriginDeployOutput,
    AaveOriginDeployOutputContract, ContractArtifactJson, COMPILE_MANIFEST_KIND,
    CONFIG_REPORT_KIND, CONTRACT_POOL, CONTRACT_USDC, CONTRACT_USDC_A_TOKEN,
    CONTRACT_USDC_VARIABLE_DEBT_TOKEN, CONTRACT_WBTC, CONTRACT_WBTC_A_TOKEN, DEPLOY_MANIFEST_KIND,
    ORIGIN_DEPLOY_OUTPUT_KIND,
};

const KEY_COMPILE_MANIFEST: &str = "compile_manifest";
const KEY_PENDING_DEPLOY_TXS: &str = "pending_deploy_txs";
const KEY_DEPLOY_RECEIPTS: &str = "deploy_receipts";
const KEY_DEPLOY_OUTPUTS: &str = "deploy_outputs";

const KEY_DEPLOY_MANIFEST_LOADED: &str = "deploy_manifest_loaded";
const KEY_PENDING_CONFIG_CALLS: &str = "pending_config_calls";
const KEY_CONFIG_RECEIPTS: &str = "config_receipts";
const KEY_CONFIG_OUTPUTS: &str = "config_outputs";

#[derive(Clone, Debug, Serialize, Deserialize)]
struct PendingDeployment {
    id: String,
    tx_hash: String,
    artifact: ContractArtifactJson,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct DeploymentReceipt {
    id: String,
    tx_hash: String,
    contract_address: String,
    deploy_receipt: serde_json::Value,
    artifact: ContractArtifactJson,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct PendingRuntimeCall {
    function: String,
    tx_hash: String,
}

#[derive(Clone, Debug)]
pub struct LoadCompileManifestState {
    pub state_id: StateId,
    pub compile_manifest_port: String,
}

#[derive(Clone, Debug)]
pub struct DeployContractState {
    pub state_id: StateId,
    pub cfg: AaveDeployRuntimeConfig,
}

#[derive(Clone, Debug)]
pub struct WaitForReceiptState {
    pub state_id: StateId,
    pub poll_interval_ms: u64,
    pub max_receipt_polls: u64,
}

#[derive(Clone, Debug)]
pub struct CollectDeployOutputsState {
    pub state_id: StateId,
}

#[derive(Clone, Debug)]
pub struct WriteDeployManifestState {
    pub state_id: StateId,
    pub deploy_manifest_export_key: String,
}

#[derive(Clone, Debug)]
pub struct AdaptOriginDeployOutputState {
    pub state_id: StateId,
    pub origin_deploy_port: String,
    pub deploy_manifest_export_key: String,
}

#[derive(Clone, Debug)]
pub struct LoadDeployManifestState {
    pub state_id: StateId,
    pub deploy_manifest_port: String,
}

#[derive(Clone, Debug)]
pub struct ConfigureRuntimeCallState {
    pub state_id: StateId,
    pub cfg: AaveConfigureRuntimeConfig,
}

#[derive(Clone, Debug)]
pub struct WaitForConfigReceiptState {
    pub state_id: StateId,
    pub poll_interval_ms: u64,
    pub max_receipt_polls: u64,
}

#[derive(Clone, Debug)]
pub struct CollectConfigOutputsState {
    pub state_id: StateId,
}

#[derive(Clone, Debug)]
pub struct WriteConfigReportState {
    pub state_id: StateId,
    pub config_report_export_key: String,
}

#[async_trait]
impl State for LoadCompileManifestState {
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
            &ContextKey(self.compile_manifest_port.clone()),
            "missing_compile_manifest",
            "missing compile manifest in context",
        )?;
        let manifest = decode_compile_manifest(&manifest_value)?;
        op_ctx::write_json(
            ctx,
            ContextKey(KEY_COMPILE_MANIFEST.to_string()),
            serde_json::to_value(manifest).map_err(|_| {
                op_errors::state_unknown(
                    "serialize_compile_manifest_failed",
                    "failed to serialize compile manifest",
                )
            })?,
        )?;

        Ok(StateOutcome {
            snapshot: SnapshotPolicy::OnSuccess,
        })
    }
}

#[async_trait]
impl State for DeployContractState {
    fn meta(&self) -> StateMeta {
        meta::apply_side_effect(op_idempotency::state_purpose(
            "aave_v3_deploy_runtime",
            &self.state_id,
            "deploy_contract",
        ))
    }

    async fn handle(
        &self,
        ctx: &mut dyn DynContext,
        io: &mut dyn IoProvider,
        _rec: &mut dyn EventRecorder,
    ) -> Result<StateOutcome, StateError> {
        let manifest = read_compile_manifest(ctx)?;
        let signing_key_env = self.cfg.signing_key_env.as_deref();
        let deployer = evm_rpc::resolve_deployer_address(
            io,
            &self.state_id,
            self.cfg.deployer_account_index,
            signing_key_env,
        )
        .await?;
        let mut next_nonce = if signing_key_env.is_some() {
            Some(evm_rpc::pending_nonce_u128(io, &self.state_id, &deployer).await?)
        } else {
            None
        };

        let mut pending: Vec<PendingDeployment> = Vec::with_capacity(manifest.contracts.len());
        for contract in manifest.contracts {
            let (abi, bytecode) = contract.artifact.parse()?;
            let constructor_payload =
                common_abi::constructor_data(&abi, &bytecode, &contract.constructor_args).map_err(
                    |_| {
                        op_errors::state_unknown(
                            "invalid_constructor_args",
                            "constructor args did not match ABI",
                        )
                    },
                )?;
            let tx_hash = if let Some(env_name) = signing_key_env {
                let nonce = next_nonce.as_mut().expect("nonce initialized");
                let nonce_hex = format!("0x{:x}", *nonce);
                let mut client = EvmIoClient::new(self.state_id.clone(), io);
                let tx_hash = evm_rpc::send_signed_create_transaction_with_nonce(
                    &mut client,
                    env_name,
                    &deployer,
                    &nonce_hex,
                    &constructor_payload,
                    None,
                )
                .await?;
                *nonce = nonce.checked_add(1).ok_or_else(|| {
                    op_errors::state_unknown(
                        "evm_response_invalid",
                        "nonce overflow while preparing signed deployment transactions",
                    )
                })?;
                tx_hash
            } else {
                let mut client = EvmIoClient::new(self.state_id.clone(), io);
                evm_rpc::send_transaction(
                    &mut client,
                    serde_json::json!({
                        "from": deployer,
                        "data": shared_dcv::bytes_to_hex_prefixed(&constructor_payload),
                    }),
                )
                .await?
            };

            pending.push(PendingDeployment {
                id: contract.id,
                tx_hash,
                artifact: contract.artifact,
            });
        }

        op_ctx::write_json(
            ctx,
            ContextKey(KEY_PENDING_DEPLOY_TXS.to_string()),
            serde_json::to_value(&pending).map_err(|_| {
                op_errors::state_unknown(
                    "serialize_pending_deployments_failed",
                    "failed to serialize pending deployments",
                )
            })?,
        )?;

        Ok(StateOutcome {
            snapshot: SnapshotPolicy::OnSuccess,
        })
    }
}

#[async_trait]
impl State for WaitForReceiptState {
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
            &ContextKey(KEY_PENDING_DEPLOY_TXS.to_string()),
            "missing_pending_deployments",
            "missing pending deployments in context",
            "pending_deployments_invalid",
            "pending deployments were invalid",
        )?;

        let mut completed: Vec<DeploymentReceipt> = Vec::with_capacity(pending.len());
        for p in pending {
            let receipt = evm_rpc::wait_for_receipt(
                &self.state_id,
                io,
                &p.tx_hash,
                self.poll_interval_ms,
                self.max_receipt_polls,
            )
            .await?;
            evm_rpc::ensure_receipt_success(&receipt)?;
            let contract_address = evm_rpc::receipt_contract_address(&receipt)?;
            completed.push(DeploymentReceipt {
                id: p.id,
                tx_hash: p.tx_hash,
                contract_address,
                deploy_receipt: receipt,
                artifact: p.artifact,
            });
        }

        op_ctx::write_json(
            ctx,
            ContextKey(KEY_DEPLOY_RECEIPTS.to_string()),
            serde_json::to_value(&completed).map_err(|_| {
                op_errors::state_unknown(
                    "serialize_deploy_receipts_failed",
                    "failed to serialize deployment receipts",
                )
            })?,
        )?;

        Ok(StateOutcome {
            snapshot: SnapshotPolicy::OnSuccess,
        })
    }
}

#[async_trait]
impl State for CollectDeployOutputsState {
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
            "missing_deploy_receipts",
            "missing deployment receipts in context",
            "deploy_receipts_invalid",
            "deployment receipts were invalid",
        )?;

        let contracts = receipts
            .into_iter()
            .map(|r| AaveDeployManifestContract {
                id: r.id,
                address: r.contract_address,
                deploy_tx_hash: r.tx_hash,
                deploy_receipt: r.deploy_receipt,
                artifact: r.artifact,
            })
            .collect::<Vec<_>>();
        let manifest = AaveDeployManifest {
            kind: DEPLOY_MANIFEST_KIND.to_string(),
            contracts,
        };

        validate_deploy_manifest(&manifest)?;
        op_ctx::write_json(
            ctx,
            ContextKey(KEY_DEPLOY_OUTPUTS.to_string()),
            serde_json::to_value(&manifest).map_err(|_| {
                op_errors::state_unknown(
                    "serialize_deploy_manifest_failed",
                    "failed to serialize deploy manifest",
                )
            })?,
        )?;

        Ok(StateOutcome {
            snapshot: SnapshotPolicy::OnSuccess,
        })
    }
}

#[async_trait]
impl State for WriteDeployManifestState {
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
            &ContextKey(KEY_DEPLOY_OUTPUTS.to_string()),
            "missing_deploy_outputs",
            "missing deploy outputs in context",
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

#[async_trait]
impl State for AdaptOriginDeployOutputState {
    fn meta(&self) -> StateMeta {
        meta::config()
    }

    async fn handle(
        &self,
        ctx: &mut dyn DynContext,
        _io: &mut dyn IoProvider,
        _rec: &mut dyn EventRecorder,
    ) -> Result<StateOutcome, StateError> {
        let value = op_ctx::read_json_required(
            ctx,
            &ContextKey(self.origin_deploy_port.clone()),
            "missing_origin_deploy_output",
            "missing origin deploy output in context",
        )?;
        let output = decode_origin_deploy_output(&value)?;

        let contracts = output
            .contracts
            .into_iter()
            .map(|c| AaveDeployManifestContract {
                id: c.id,
                address: c.address,
                deploy_tx_hash: c.deploy_tx_hash.unwrap_or_else(|| "0x0".to_string()),
                deploy_receipt: c.deploy_receipt.unwrap_or_else(|| serde_json::json!({})),
                artifact: c.artifact,
            })
            .collect::<Vec<_>>();
        let manifest = AaveDeployManifest {
            kind: DEPLOY_MANIFEST_KIND.to_string(),
            contracts,
        };
        validate_deploy_manifest(&manifest)?;

        op_ctx::write_json(
            ctx,
            ContextKey(self.deploy_manifest_export_key.clone()),
            serde_json::to_value(manifest).map_err(|_| {
                op_errors::state_unknown(
                    "serialize_deploy_manifest_failed",
                    "failed to serialize deploy manifest",
                )
            })?,
        )?;

        Ok(StateOutcome {
            snapshot: SnapshotPolicy::OnSuccess,
        })
    }
}

#[async_trait]
impl State for LoadDeployManifestState {
    fn meta(&self) -> StateMeta {
        meta::config()
    }

    async fn handle(
        &self,
        ctx: &mut dyn DynContext,
        _io: &mut dyn IoProvider,
        _rec: &mut dyn EventRecorder,
    ) -> Result<StateOutcome, StateError> {
        let value = op_ctx::read_json_required(
            ctx,
            &ContextKey(self.deploy_manifest_port.clone()),
            "missing_deploy_manifest",
            "missing deploy manifest in context",
        )?;
        let manifest = decode_deploy_manifest(&value)?;
        op_ctx::write_json(
            ctx,
            ContextKey(KEY_DEPLOY_MANIFEST_LOADED.to_string()),
            serde_json::to_value(&manifest).map_err(|_| {
                op_errors::state_unknown(
                    "serialize_deploy_manifest_failed",
                    "failed to serialize deploy manifest",
                )
            })?,
        )?;
        Ok(StateOutcome {
            snapshot: SnapshotPolicy::OnSuccess,
        })
    }
}

#[async_trait]
impl State for ConfigureRuntimeCallState {
    fn meta(&self) -> StateMeta {
        meta::apply_side_effect(op_idempotency::state_purpose(
            "aave_v3_configure_runtime",
            &self.state_id,
            "configure_runtime",
        ))
    }

    async fn handle(
        &self,
        ctx: &mut dyn DynContext,
        io: &mut dyn IoProvider,
        _rec: &mut dyn EventRecorder,
    ) -> Result<StateOutcome, StateError> {
        let manifest = read_deploy_manifest_loaded(ctx)?;
        let from =
            evm_rpc::resolve_account_by_index(io, &self.state_id, self.cfg.from_account_index)
                .await?;
        let pool = contract_from_manifest(&manifest, CONTRACT_POOL)?;
        let usdc = contract_from_manifest(&manifest, CONTRACT_USDC)?;
        let wbtc = contract_from_manifest(&manifest, CONTRACT_WBTC)?;

        let tx_hash = send_contract_transaction(
            io,
            &self.state_id,
            &from,
            pool,
            ContractCallSpec {
                function: "configureRuntime",
                args: vec![
                    serde_json::json!(usdc.address),
                    serde_json::json!(wbtc.address),
                ],
                value: None,
                err_code: "configure_runtime_call_failed",
            },
        )
        .await?;

        let pending = vec![PendingRuntimeCall {
            function: "configureRuntime".to_string(),
            tx_hash,
        }];
        op_ctx::write_json(
            ctx,
            ContextKey(KEY_PENDING_CONFIG_CALLS.to_string()),
            serde_json::to_value(&pending).map_err(|_| {
                op_errors::state_unknown(
                    "serialize_pending_config_calls_failed",
                    "failed to serialize pending config calls",
                )
            })?,
        )?;

        Ok(StateOutcome {
            snapshot: SnapshotPolicy::OnSuccess,
        })
    }
}

#[async_trait]
impl State for WaitForConfigReceiptState {
    fn meta(&self) -> StateMeta {
        meta::fetch_data()
    }

    async fn handle(
        &self,
        ctx: &mut dyn DynContext,
        io: &mut dyn IoProvider,
        _rec: &mut dyn EventRecorder,
    ) -> Result<StateOutcome, StateError> {
        let pending = op_ctx::read_typed::<Vec<PendingRuntimeCall>>(
            ctx,
            &ContextKey(KEY_PENDING_CONFIG_CALLS.to_string()),
            "missing_pending_config_calls",
            "missing pending config calls in context",
            "pending_config_calls_invalid",
            "pending config calls were invalid",
        )?;

        let mut calls = Vec::with_capacity(pending.len());
        for p in pending {
            let receipt = evm_rpc::wait_for_receipt(
                &self.state_id,
                io,
                &p.tx_hash,
                self.poll_interval_ms,
                self.max_receipt_polls,
            )
            .await?;
            evm_rpc::ensure_receipt_success(&receipt)?;
            calls.push(AaveConfigCallRecord {
                function: p.function,
                tx_hash: p.tx_hash,
                receipt,
            });
        }

        op_ctx::write_json(
            ctx,
            ContextKey(KEY_CONFIG_RECEIPTS.to_string()),
            serde_json::to_value(&calls).map_err(|_| {
                op_errors::state_unknown(
                    "serialize_config_receipts_failed",
                    "failed to serialize config receipts",
                )
            })?,
        )?;

        Ok(StateOutcome {
            snapshot: SnapshotPolicy::OnSuccess,
        })
    }
}

#[async_trait]
impl State for CollectConfigOutputsState {
    fn meta(&self) -> StateMeta {
        meta::pure()
    }

    async fn handle(
        &self,
        ctx: &mut dyn DynContext,
        _io: &mut dyn IoProvider,
        _rec: &mut dyn EventRecorder,
    ) -> Result<StateOutcome, StateError> {
        let manifest = read_deploy_manifest_loaded(ctx)?;
        let pool = contract_from_manifest(&manifest, CONTRACT_POOL)?;
        let usdc = contract_from_manifest(&manifest, CONTRACT_USDC)?;
        let wbtc = contract_from_manifest(&manifest, CONTRACT_WBTC)?;
        let calls = op_ctx::read_typed::<Vec<AaveConfigCallRecord>>(
            ctx,
            &ContextKey(KEY_CONFIG_RECEIPTS.to_string()),
            "missing_config_receipts",
            "missing config receipts in context",
            "config_receipts_invalid",
            "config receipts were invalid",
        )?;

        let report = AaveConfigReport {
            kind: CONFIG_REPORT_KIND.to_string(),
            pool: pool.address.clone(),
            usdc: usdc.address.clone(),
            wbtc: wbtc.address.clone(),
            calls,
        };
        op_ctx::write_json(
            ctx,
            ContextKey(KEY_CONFIG_OUTPUTS.to_string()),
            serde_json::to_value(report).map_err(|_| {
                op_errors::state_unknown(
                    "serialize_config_report_failed",
                    "failed to serialize config report",
                )
            })?,
        )?;
        Ok(StateOutcome {
            snapshot: SnapshotPolicy::OnSuccess,
        })
    }
}

#[async_trait]
impl State for WriteConfigReportState {
    fn meta(&self) -> StateMeta {
        meta::pure()
    }

    async fn handle(
        &self,
        ctx: &mut dyn DynContext,
        _io: &mut dyn IoProvider,
        _rec: &mut dyn EventRecorder,
    ) -> Result<StateOutcome, StateError> {
        let report = op_ctx::read_json_required(
            ctx,
            &ContextKey(KEY_CONFIG_OUTPUTS.to_string()),
            "missing_config_outputs",
            "missing config outputs in context",
        )?;
        op_ctx::write_json(
            ctx,
            ContextKey(self.config_report_export_key.clone()),
            report,
        )?;
        Ok(StateOutcome {
            snapshot: SnapshotPolicy::OnSuccess,
        })
    }
}

fn read_compile_manifest(ctx: &dyn DynContext) -> Result<AaveCompileManifest, StateError> {
    let value = op_ctx::read_json_required(
        ctx,
        &ContextKey(KEY_COMPILE_MANIFEST.to_string()),
        "missing_compile_manifest",
        "missing compile manifest in context",
    )?;
    decode_compile_manifest(&value)
}

fn read_deploy_manifest_loaded(ctx: &dyn DynContext) -> Result<AaveDeployManifest, StateError> {
    let value = op_ctx::read_json_required(
        ctx,
        &ContextKey(KEY_DEPLOY_MANIFEST_LOADED.to_string()),
        "missing_deploy_manifest",
        "missing deploy manifest in context",
    )?;
    decode_deploy_manifest(&value)
}

fn encode_call_data(
    contract: &AaveDeployManifestContract,
    function: &str,
    args: Vec<serde_json::Value>,
) -> Result<String, StateError> {
    let (abi, _bytecode) = contract.artifact.parse()?;
    let (calldata, _outputs) =
        common_abi::resolve_function_call(&abi, function, &args).map_err(|_| {
            op_errors::state_error(
                "invalid_function_call",
                ErrorCategory::ParsingInput,
                false,
                "function call did not match ABI",
            )
        })?;
    Ok(shared_dcv::bytes_to_hex_prefixed(&calldata))
}

struct ContractCallSpec<'a> {
    function: &'a str,
    args: Vec<serde_json::Value>,
    value: Option<String>,
    err_code: &'static str,
}

async fn send_contract_transaction(
    io: &mut dyn IoProvider,
    state_id: &StateId,
    from: &str,
    contract: &AaveDeployManifestContract,
    spec: ContractCallSpec<'_>,
) -> Result<String, StateError> {
    let from_addr = shared_dcv::normalize_address(from).map_err(|_| {
        op_errors::state_error(
            "invalid_from_address",
            ErrorCategory::ParsingInput,
            false,
            "from address was invalid",
        )
    })?;
    let data = encode_call_data(contract, spec.function, spec.args)?;
    let mut tx = serde_json::json!({
        "from": from_addr,
        "to": contract.address,
        "data": data,
    });
    if let Some(v) = spec.value {
        tx["value"] = serde_json::json!(v);
    }

    let mut client = EvmIoClient::new(state_id.clone(), io);
    evm_rpc::send_transaction(&mut client, tx)
        .await
        .map_err(|_| {
            op_errors::state_error(
                spec.err_code,
                ErrorCategory::OnChain,
                false,
                "contract transaction failed to submit",
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    use async_trait::async_trait;
    use mfm_machine::errors::{ErrorCategory, IoError};
    use mfm_machine::ids::{ArtifactId, FactKey};
    use mfm_machine::io::{IoCall, IoResult};

    fn test_artifact() -> ContractArtifactJson {
        ContractArtifactJson {
            abi: serde_json::json!([
                {
                    "type": "function",
                    "name": "balanceOf",
                    "inputs": [{ "type": "address" }],
                    "outputs": [{ "type": "uint256" }]
                }
            ]),
            bytecode: serde_json::json!({ "object": "0x00" }),
        }
    }

    fn make_origin_output_contract(id: &str, address: &str) -> serde_json::Value {
        serde_json::json!({
            "id": id,
            "address": address,
            "artifact": test_artifact(),
            "deploy_tx_hash": "0x0",
            "deploy_receipt": {},
        })
    }

    #[derive(Default)]
    struct ResolveDeployerTestIo {
        calls: Vec<IoCall>,
    }

    impl ResolveDeployerTestIo {
        fn saw_eth_accounts(&self) -> bool {
            self.calls.iter().any(|call| {
                call.namespace == "evm"
                    && call.request.get("method").and_then(|v| v.as_str()) == Some("eth_accounts")
            })
        }

        fn saw_local_signer_address(&self) -> bool {
            self.calls
                .iter()
                .any(|call| call.namespace == "local.evm.signer_address")
        }
    }

    #[async_trait]
    impl IoProvider for ResolveDeployerTestIo {
        async fn call(&mut self, call: IoCall) -> Result<IoResult, IoError> {
            self.calls.push(call.clone());

            if call.namespace == "local.evm.signer_address" {
                return Ok(IoResult {
                    response: serde_json::json!({
                        "address": "0x7e5f4552091a69125d5dfcb7b8c2659029395bdf",
                    }),
                    recorded_payload_id: None,
                });
            }

            if call.namespace == "evm"
                && call.request.get("method").and_then(|v| v.as_str()) == Some("eth_accounts")
            {
                return Ok(IoResult {
                    response: serde_json::json!(["0x1111111111111111111111111111111111111111"]),
                    recorded_payload_id: None,
                });
            }

            Err(IoError::Other(crate::errors::info(
                "unexpected_io_call",
                ErrorCategory::Unknown,
                false,
                "unexpected io call",
            )))
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

    #[tokio::test]
    async fn resolve_deployer_address_signed_mode_uses_local_signer_address() {
        let state_id = StateId("m.test.aave_v3.resolve_deployer".to_string());
        let mut io = ResolveDeployerTestIo::default();

        let deployer =
            evm_rpc::resolve_deployer_address(&mut io, &state_id, 9, Some("MFM_TEST_SIGNING_KEY"))
                .await
                .expect("resolve deployer");
        assert_eq!(deployer, "0x7e5f4552091a69125d5dfcb7b8c2659029395bdf");
        assert!(io.saw_local_signer_address());
        assert!(!io.saw_eth_accounts());
    }

    #[tokio::test]
    async fn resolve_deployer_address_unsigned_mode_uses_eth_accounts() {
        let state_id = StateId("m.test.aave_v3.resolve_deployer".to_string());
        let mut io = ResolveDeployerTestIo::default();

        let deployer = evm_rpc::resolve_deployer_address(&mut io, &state_id, 0, None)
            .await
            .expect("resolve deployer");
        assert_eq!(deployer, "0x1111111111111111111111111111111111111111");
        assert!(!io.saw_local_signer_address());
        assert!(io.saw_eth_accounts());
    }

    #[test]
    fn decode_origin_deploy_output_accepts_required_contracts() {
        let value = serde_json::json!({
            "kind": ORIGIN_DEPLOY_OUTPUT_KIND,
            "contracts": [
                make_origin_output_contract(CONTRACT_POOL, "0x0000000000000000000000000000000000000001"),
                make_origin_output_contract(CONTRACT_USDC, "0x0000000000000000000000000000000000000002"),
                make_origin_output_contract(CONTRACT_WBTC, "0x0000000000000000000000000000000000000003"),
                make_origin_output_contract(CONTRACT_USDC_A_TOKEN, "0x0000000000000000000000000000000000000004"),
                make_origin_output_contract(CONTRACT_WBTC_A_TOKEN, "0x0000000000000000000000000000000000000005"),
                make_origin_output_contract(CONTRACT_USDC_VARIABLE_DEBT_TOKEN, "0x0000000000000000000000000000000000000006")
            ]
        });

        let output = decode_origin_deploy_output(&value).expect("expected valid origin output");
        assert_eq!(output.contracts.len(), 6);
    }

    #[test]
    fn decode_origin_deploy_output_rejects_missing_required_contract() {
        let value = serde_json::json!({
            "kind": ORIGIN_DEPLOY_OUTPUT_KIND,
            "contracts": [
                make_origin_output_contract(CONTRACT_POOL, "0x0000000000000000000000000000000000000001"),
                make_origin_output_contract(CONTRACT_USDC, "0x0000000000000000000000000000000000000002"),
                make_origin_output_contract(CONTRACT_WBTC, "0x0000000000000000000000000000000000000003"),
                make_origin_output_contract(CONTRACT_USDC_A_TOKEN, "0x0000000000000000000000000000000000000004"),
                make_origin_output_contract(CONTRACT_WBTC_A_TOKEN, "0x0000000000000000000000000000000000000005")
            ]
        });

        let err = decode_origin_deploy_output(&value).expect_err("expected validation failure");
        assert_eq!(err.info.code.0, "origin_deploy_output_missing_contract");
    }
}
