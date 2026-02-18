use std::collections::HashSet;

use async_trait::async_trait;
use mfm_collectors_evm::{EvmIoClient, JsonRpcCall};
use mfm_machine::context::DynContext;
use mfm_machine::errors::{ErrorCategory, StateError};
use mfm_machine::ids::{ContextKey, FactKey, StateId};
use mfm_machine::io::IoProvider;
use mfm_machine::meta::StateMeta;
use mfm_machine::recorder::EventRecorder;
use mfm_machine::state::{SnapshotPolicy, State, StateOutcome};
use serde::{Deserialize, Serialize};

use crate::abi as common_abi;
use crate::ctx as op_ctx;
use crate::errors as op_errors;
use crate::evm_rpc;
use crate::idempotency as op_idempotency;
use crate::output as op_output;
use crate::rpc as op_rpc;
use crate::states::evm_dcv as shared_dcv;
use crate::states::meta;

pub const COMPILE_MANIFEST_KIND: &str = "aave_v3_origin_compile_manifest_v1";
pub const DEPLOY_MANIFEST_KIND: &str = "aave_v3_deploy_manifest_v1";
pub const ORIGIN_DEPLOY_OUTPUT_KIND: &str = "aave_v3_origin_deploy_output_v1";
pub const CONFIG_REPORT_KIND: &str = "aave_v3_config_report_v1";
pub const SCENARIO_REPORT_KIND: &str = "aave_v3_reth_scenario_report_v1";

pub const CONTRACT_USDC: &str = "usdc";
pub const CONTRACT_WBTC: &str = "wbtc";
pub const CONTRACT_POOL: &str = "pool";
pub const CONTRACT_USDC_A_TOKEN: &str = "usdc_a_token";
pub const CONTRACT_WBTC_A_TOKEN: &str = "wbtc_a_token";
pub const CONTRACT_USDC_VARIABLE_DEBT_TOKEN: &str = "usdc_variable_debt_token";

const KEY_COMPILE_MANIFEST: &str = "compile_manifest";
const KEY_PENDING_DEPLOY_TXS: &str = "pending_deploy_txs";
const KEY_DEPLOY_RECEIPTS: &str = "deploy_receipts";
const KEY_DEPLOY_OUTPUTS: &str = "deploy_outputs";

const KEY_DEPLOY_MANIFEST_LOADED: &str = "deploy_manifest_loaded";
const KEY_PENDING_CONFIG_CALLS: &str = "pending_config_calls";
const KEY_CONFIG_RECEIPTS: &str = "config_receipts";
const KEY_CONFIG_OUTPUTS: &str = "config_outputs";

const KEY_CHAIN_ID: &str = "chain_id";
const KEY_SCENARIO_ACCOUNTS: &str = "scenario_accounts";
const KEY_FUNDING_TX_HASHES: &str = "funding_tx_hashes";
const KEY_APPROVAL_TX_HASHES: &str = "approval_tx_hashes";
const KEY_SUPPLY_TX_HASHES: &str = "supply_tx_hashes";
const KEY_BORROW_TX_HASHES: &str = "borrow_tx_hashes";
const KEY_SCENARIO_POSITION_SNAPSHOT: &str = "scenario_position_snapshot";
const KEY_SCENARIO_ASSERTIONS: &str = "scenario_assertions";
const KEY_SCENARIO_REPORT_JSON: &str = "scenario_report_json";

fn default_poll_interval_ms() -> u64 {
    200
}

fn default_max_receipt_polls() -> u64 {
    120
}

fn default_compile_manifest_port() -> String {
    "result".to_string()
}

fn default_deploy_manifest_port() -> String {
    "deploy_manifest".to_string()
}

fn default_deploy_manifest_export_key() -> String {
    "deploy_manifest".to_string()
}

fn default_config_report_export_key() -> String {
    "config_report".to_string()
}

fn default_scenario_report_export_key() -> String {
    "scenario_report".to_string()
}

fn default_scenario_report_artifact_key() -> String {
    "scenario_report_artifact_id".to_string()
}

fn default_deployer_account_index() -> usize {
    0
}

fn default_configure_from_account_index() -> usize {
    0
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ContractArtifactJson {
    pub abi: serde_json::Value,
    pub bytecode: serde_json::Value,
}

impl ContractArtifactJson {
    pub fn parse(&self) -> Result<(common_abi::ParsedAbi, Vec<u8>), StateError> {
        let cfg = serde_json::from_value::<shared_dcv::ContractArtifactConfig>(serde_json::json!({
            "abi": self.abi,
            "bytecode": self.bytecode,
        }))
        .map_err(|_| {
            op_errors::state_unknown("invalid_contract_artifact", "contract artifact was invalid")
        })?;

        shared_dcv::parse_artifact(&cfg).map_err(|_| {
            op_errors::state_unknown("invalid_contract_artifact", "contract artifact was invalid")
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AaveCompileManifestContract {
    pub id: String,
    pub artifact: ContractArtifactJson,
    #[serde(default)]
    pub constructor_args: Vec<serde_json::Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AaveCompileManifest {
    pub kind: String,
    pub contracts: Vec<AaveCompileManifestContract>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AaveDeployManifestContract {
    pub id: String,
    pub address: String,
    pub deploy_tx_hash: String,
    pub deploy_receipt: serde_json::Value,
    pub artifact: ContractArtifactJson,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AaveDeployManifest {
    pub kind: String,
    pub contracts: Vec<AaveDeployManifestContract>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AaveOriginDeployOutputContract {
    pub id: String,
    pub address: String,
    pub artifact: ContractArtifactJson,
    #[serde(default)]
    pub deploy_tx_hash: Option<String>,
    #[serde(default)]
    pub deploy_receipt: Option<serde_json::Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AaveOriginDeployOutput {
    pub kind: String,
    pub contracts: Vec<AaveOriginDeployOutputContract>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AaveConfigCallRecord {
    pub function: String,
    pub tx_hash: String,
    pub receipt: serde_json::Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AaveConfigReport {
    pub kind: String,
    pub pool: String,
    pub usdc: String,
    pub wbtc: String,
    pub calls: Vec<AaveConfigCallRecord>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AaveScenarioAccounts {
    pub funder: String,
    pub supplier: String,
    pub borrower: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AaveScenarioAmounts {
    pub usdc_supply: u64,
    pub wbtc_collateral: u64,
    pub usdc_borrow: u64,
    pub borrow_rate_mode: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AaveScenarioPositionSnapshot {
    pub supplier_supplied_usdc: u64,
    pub borrower_collateral_wbtc: u64,
    pub borrower_borrowed_usdc: u64,
    pub borrower_usdc_balance: u64,
    pub pool_usdc_balance: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AaveScenarioAssertions {
    pub strict: bool,
    pub passed: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AaveScenarioReport {
    pub kind: String,
    pub chain_id: u64,
    pub accounts: AaveScenarioAccounts,
    pub amounts: AaveScenarioAmounts,
    pub positions: AaveScenarioPositionSnapshot,
    pub assertions: AaveScenarioAssertions,
    pub deploy_manifest_kind: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AaveDeployRuntimeConfig {
    #[serde(default = "default_compile_manifest_port")]
    pub compile_manifest_port: String,

    #[serde(default = "default_deployer_account_index")]
    pub deployer_account_index: usize,

    #[serde(default)]
    pub signing_key_env: Option<String>,

    #[serde(default = "default_poll_interval_ms")]
    pub poll_interval_ms: u64,

    #[serde(default = "default_max_receipt_polls")]
    pub max_receipt_polls: u64,

    #[serde(default = "default_deploy_manifest_export_key")]
    pub deploy_manifest_export_key: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AaveConfigureRuntimeConfig {
    #[serde(default = "default_deploy_manifest_port")]
    pub deploy_manifest_port: String,

    #[serde(default = "default_configure_from_account_index")]
    pub from_account_index: usize,

    #[serde(default = "default_poll_interval_ms")]
    pub poll_interval_ms: u64,

    #[serde(default = "default_max_receipt_polls")]
    pub max_receipt_polls: u64,

    #[serde(default = "default_config_report_export_key")]
    pub config_report_export_key: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AaveScenarioConfig {
    #[serde(default = "default_deploy_manifest_port")]
    pub deploy_manifest_port: String,

    pub funder_account_index: usize,

    pub supplier_account_index: usize,

    pub borrower_account_index: usize,

    pub fund_wei: String,

    pub usdc_supply_amount: u64,

    pub wbtc_collateral_amount: u64,

    pub usdc_borrow_amount: u64,

    pub borrow_rate_mode: u64,

    #[serde(default = "default_poll_interval_ms")]
    pub poll_interval_ms: u64,

    #[serde(default = "default_max_receipt_polls")]
    pub max_receipt_polls: u64,

    #[serde(default = "default_scenario_report_export_key")]
    pub scenario_report_export_key: String,

    #[serde(default = "default_scenario_report_artifact_key")]
    pub scenario_report_artifact_key: String,
}

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

#[derive(Clone, Debug)]
pub struct ResolveChainState {
    pub state_id: StateId,
}

#[derive(Clone, Debug)]
pub struct ResolveAccountsState {
    pub state_id: StateId,
    pub cfg: AaveScenarioConfig,
}

#[derive(Clone, Debug)]
pub struct FundWalletState {
    pub state_id: StateId,
    pub cfg: AaveScenarioConfig,
}

#[derive(Clone, Debug)]
pub struct ApproveErc20State {
    pub state_id: StateId,
    pub cfg: AaveScenarioConfig,
}

#[derive(Clone, Debug)]
pub struct SupplyAssetState {
    pub state_id: StateId,
    pub cfg: AaveScenarioConfig,
}

#[derive(Clone, Debug)]
pub struct BorrowAssetState {
    pub state_id: StateId,
    pub cfg: AaveScenarioConfig,
}

#[derive(Clone, Debug)]
pub struct ReadPositionsState {
    pub state_id: StateId,
}

#[derive(Clone, Debug)]
pub struct AssertInvariantsState {
    pub state_id: StateId,
    pub cfg: AaveScenarioConfig,
}

#[derive(Clone, Debug)]
pub struct WriteSummaryState {
    pub state_id: StateId,
    pub cfg: AaveScenarioConfig,
    pub op_path: String,
}

pub fn validate_deploy_runtime_config(cfg: &AaveDeployRuntimeConfig) -> Result<(), String> {
    if cfg.compile_manifest_port.trim().is_empty() {
        return Err("compile_manifest_port must be non-empty".to_string());
    }
    if cfg.deploy_manifest_export_key.trim().is_empty() {
        return Err("deploy_manifest_export_key must be non-empty".to_string());
    }
    if cfg.max_receipt_polls == 0 {
        return Err("max_receipt_polls must be > 0".to_string());
    }
    Ok(())
}

pub fn validate_configure_runtime_config(cfg: &AaveConfigureRuntimeConfig) -> Result<(), String> {
    if cfg.deploy_manifest_port.trim().is_empty() {
        return Err("deploy_manifest_port must be non-empty".to_string());
    }
    if cfg.config_report_export_key.trim().is_empty() {
        return Err("config_report_export_key must be non-empty".to_string());
    }
    if cfg.max_receipt_polls == 0 {
        return Err("max_receipt_polls must be > 0".to_string());
    }
    Ok(())
}

pub fn validate_scenario_config(cfg: &AaveScenarioConfig) -> Result<(), String> {
    if cfg.deploy_manifest_port.trim().is_empty() {
        return Err("deploy_manifest_port must be non-empty".to_string());
    }
    if cfg.scenario_report_export_key.trim().is_empty() {
        return Err("scenario_report_export_key must be non-empty".to_string());
    }
    if cfg.scenario_report_artifact_key.trim().is_empty() {
        return Err("scenario_report_artifact_key must be non-empty".to_string());
    }
    if cfg.fund_wei.trim().is_empty() {
        return Err("fund_wei must be non-empty".to_string());
    }
    let fund_wei = parse_wei_u128(&cfg.fund_wei)
        .map_err(|_| "fund_wei must be a valid decimal or 0x hex string".to_string())?;
    if fund_wei == 0 {
        return Err("fund_wei must be > 0".to_string());
    }
    if cfg.funder_account_index == cfg.supplier_account_index
        || cfg.funder_account_index == cfg.borrower_account_index
        || cfg.supplier_account_index == cfg.borrower_account_index
    {
        return Err(
            "funder_account_index, supplier_account_index, and borrower_account_index must differ"
                .to_string(),
        );
    }
    if cfg.usdc_supply_amount == 0 {
        return Err("usdc_supply_amount must be > 0".to_string());
    }
    if cfg.wbtc_collateral_amount == 0 {
        return Err("wbtc_collateral_amount must be > 0".to_string());
    }
    if cfg.usdc_borrow_amount == 0 {
        return Err("usdc_borrow_amount must be > 0".to_string());
    }
    if cfg.borrow_rate_mode == 0 {
        return Err("borrow_rate_mode must be > 0".to_string());
    }
    if cfg.max_receipt_polls == 0 {
        return Err("max_receipt_polls must be > 0".to_string());
    }
    Ok(())
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
            Some(pending_nonce_u128(io, &self.state_id, &deployer).await?)
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
        let pending = read_typed::<Vec<PendingDeployment>>(
            ctx,
            KEY_PENDING_DEPLOY_TXS,
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
        let receipts = read_typed::<Vec<DeploymentReceipt>>(
            ctx,
            KEY_DEPLOY_RECEIPTS,
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
            "configureRuntime",
            vec![
                serde_json::json!(usdc.address),
                serde_json::json!(wbtc.address),
            ],
            None,
            "configure_runtime_call_failed",
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
        let pending = read_typed::<Vec<PendingRuntimeCall>>(
            ctx,
            KEY_PENDING_CONFIG_CALLS,
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
        let calls = read_typed::<Vec<AaveConfigCallRecord>>(
            ctx,
            KEY_CONFIG_RECEIPTS,
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

#[async_trait]
impl State for ResolveChainState {
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
        let chain_id = client
            .chain_id_u64()
            .await
            .map_err(op_errors::state_from_io)?;
        op_ctx::write_json(
            ctx,
            ContextKey(KEY_CHAIN_ID.to_string()),
            serde_json::json!(chain_id),
        )?;

        Ok(StateOutcome {
            snapshot: SnapshotPolicy::OnSuccess,
        })
    }
}

#[async_trait]
impl State for ResolveAccountsState {
    fn meta(&self) -> StateMeta {
        meta::fetch_data()
    }

    async fn handle(
        &self,
        ctx: &mut dyn DynContext,
        io: &mut dyn IoProvider,
        _rec: &mut dyn EventRecorder,
    ) -> Result<StateOutcome, StateError> {
        let accounts = evm_rpc::rpc_accounts(io, &self.state_id).await?;
        let funder = accounts
            .get(self.cfg.funder_account_index)
            .cloned()
            .ok_or_else(|| {
                op_errors::state_error(
                    "invalid_funder_account_index",
                    ErrorCategory::ParsingInput,
                    false,
                    "account index was out of range",
                )
            })?;
        let supplier = accounts
            .get(self.cfg.supplier_account_index)
            .cloned()
            .ok_or_else(|| {
                op_errors::state_error(
                    "invalid_supplier_account_index",
                    ErrorCategory::ParsingInput,
                    false,
                    "account index was out of range",
                )
            })?;
        let borrower = accounts
            .get(self.cfg.borrower_account_index)
            .cloned()
            .ok_or_else(|| {
                op_errors::state_error(
                    "invalid_borrower_account_index",
                    ErrorCategory::ParsingInput,
                    false,
                    "account index was out of range",
                )
            })?;

        let resolved = AaveScenarioAccounts {
            funder,
            supplier,
            borrower,
        };
        op_ctx::write_json(
            ctx,
            ContextKey(KEY_SCENARIO_ACCOUNTS.to_string()),
            serde_json::to_value(resolved).map_err(|_| {
                op_errors::state_unknown(
                    "serialize_scenario_accounts_failed",
                    "failed to serialize scenario accounts",
                )
            })?,
        )?;
        Ok(StateOutcome {
            snapshot: SnapshotPolicy::OnSuccess,
        })
    }
}

#[async_trait]
impl State for FundWalletState {
    fn meta(&self) -> StateMeta {
        meta::apply_side_effect(op_idempotency::state_purpose(
            "aave_v3_scenario",
            &self.state_id,
            "fund_wallet",
        ))
    }

    async fn handle(
        &self,
        ctx: &mut dyn DynContext,
        io: &mut dyn IoProvider,
        _rec: &mut dyn EventRecorder,
    ) -> Result<StateOutcome, StateError> {
        let accounts = read_typed::<AaveScenarioAccounts>(
            ctx,
            KEY_SCENARIO_ACCOUNTS,
            "missing_scenario_accounts",
            "missing scenario accounts in context",
            "scenario_accounts_invalid",
            "scenario accounts were invalid",
        )?;
        let fund_wei = parse_wei_u128(&self.cfg.fund_wei)?;

        let mut tx_hashes = Vec::new();
        for to in [&accounts.supplier, &accounts.borrower] {
            let current_balance = evm_rpc::account_balance(io, &self.state_id, to).await?;
            if current_balance >= fund_wei {
                continue;
            }
            let top_up_wei = fund_wei - current_balance;
            let value_hex = format!("0x{top_up_wei:x}");

            let mut client = EvmIoClient::new(self.state_id.clone(), io);
            let tx_hash = evm_rpc::send_transaction(
                &mut client,
                serde_json::json!({
                    "from": accounts.funder,
                    "to": to,
                    "value": value_hex,
                }),
            )
            .await?;

            let receipt = evm_rpc::wait_for_receipt(
                &self.state_id,
                io,
                &tx_hash,
                self.cfg.poll_interval_ms,
                self.cfg.max_receipt_polls,
            )
            .await?;
            evm_rpc::ensure_receipt_success(&receipt)?;
            tx_hashes.push(tx_hash);
        }

        op_ctx::write_json(
            ctx,
            ContextKey(KEY_FUNDING_TX_HASHES.to_string()),
            serde_json::json!(tx_hashes),
        )?;
        Ok(StateOutcome {
            snapshot: SnapshotPolicy::OnSuccess,
        })
    }
}

#[async_trait]
impl State for ApproveErc20State {
    fn meta(&self) -> StateMeta {
        meta::apply_side_effect(op_idempotency::state_purpose(
            "aave_v3_scenario",
            &self.state_id,
            "approve_erc20",
        ))
    }

    async fn handle(
        &self,
        ctx: &mut dyn DynContext,
        io: &mut dyn IoProvider,
        _rec: &mut dyn EventRecorder,
    ) -> Result<StateOutcome, StateError> {
        let manifest = read_deploy_manifest_loaded(ctx)?;
        let accounts = read_typed::<AaveScenarioAccounts>(
            ctx,
            KEY_SCENARIO_ACCOUNTS,
            "missing_scenario_accounts",
            "missing scenario accounts in context",
            "scenario_accounts_invalid",
            "scenario accounts were invalid",
        )?;

        let pool = contract_from_manifest(&manifest, CONTRACT_POOL)?;
        let usdc = contract_from_manifest(&manifest, CONTRACT_USDC)?;
        let wbtc = contract_from_manifest(&manifest, CONTRACT_WBTC)?;

        let mut txs = Vec::new();
        let supplier_approve_tx = send_contract_transaction(
            io,
            &self.state_id,
            &accounts.supplier,
            usdc,
            "approve",
            vec![
                serde_json::json!(pool.address),
                serde_json::json!(self.cfg.usdc_supply_amount),
            ],
            None,
            "approve_usdc_failed",
        )
        .await?;
        let supplier_approve_receipt = evm_rpc::wait_for_receipt(
            &self.state_id,
            io,
            &supplier_approve_tx,
            self.cfg.poll_interval_ms,
            self.cfg.max_receipt_polls,
        )
        .await?;
        evm_rpc::ensure_receipt_success(&supplier_approve_receipt)?;
        txs.push(supplier_approve_tx);

        let borrower_approve_tx = send_contract_transaction(
            io,
            &self.state_id,
            &accounts.borrower,
            wbtc,
            "approve",
            vec![
                serde_json::json!(pool.address),
                serde_json::json!(self.cfg.wbtc_collateral_amount),
            ],
            None,
            "approve_wbtc_failed",
        )
        .await?;
        let borrower_approve_receipt = evm_rpc::wait_for_receipt(
            &self.state_id,
            io,
            &borrower_approve_tx,
            self.cfg.poll_interval_ms,
            self.cfg.max_receipt_polls,
        )
        .await?;
        evm_rpc::ensure_receipt_success(&borrower_approve_receipt)?;
        txs.push(borrower_approve_tx);

        op_ctx::write_json(
            ctx,
            ContextKey(KEY_APPROVAL_TX_HASHES.to_string()),
            serde_json::json!(txs),
        )?;
        Ok(StateOutcome {
            snapshot: SnapshotPolicy::OnSuccess,
        })
    }
}

#[async_trait]
impl State for SupplyAssetState {
    fn meta(&self) -> StateMeta {
        meta::apply_side_effect(op_idempotency::state_purpose(
            "aave_v3_scenario",
            &self.state_id,
            "supply_asset",
        ))
    }

    async fn handle(
        &self,
        ctx: &mut dyn DynContext,
        io: &mut dyn IoProvider,
        _rec: &mut dyn EventRecorder,
    ) -> Result<StateOutcome, StateError> {
        let manifest = read_deploy_manifest_loaded(ctx)?;
        let accounts = read_typed::<AaveScenarioAccounts>(
            ctx,
            KEY_SCENARIO_ACCOUNTS,
            "missing_scenario_accounts",
            "missing scenario accounts in context",
            "scenario_accounts_invalid",
            "scenario accounts were invalid",
        )?;

        let pool = contract_from_manifest(&manifest, CONTRACT_POOL)?;
        let usdc = contract_from_manifest(&manifest, CONTRACT_USDC)?;
        let wbtc = contract_from_manifest(&manifest, CONTRACT_WBTC)?;

        let mut txs = Vec::new();
        let supply_usdc_tx = send_contract_transaction(
            io,
            &self.state_id,
            &accounts.supplier,
            pool,
            "supply",
            vec![
                serde_json::json!(usdc.address),
                serde_json::json!(self.cfg.usdc_supply_amount),
                serde_json::json!(accounts.supplier),
                serde_json::json!(0),
            ],
            None,
            "supply_usdc_failed",
        )
        .await?;
        let supply_usdc_receipt = evm_rpc::wait_for_receipt(
            &self.state_id,
            io,
            &supply_usdc_tx,
            self.cfg.poll_interval_ms,
            self.cfg.max_receipt_polls,
        )
        .await?;
        evm_rpc::ensure_receipt_success(&supply_usdc_receipt)?;
        txs.push(supply_usdc_tx);

        let supply_wbtc_tx = send_contract_transaction(
            io,
            &self.state_id,
            &accounts.borrower,
            pool,
            "supply",
            vec![
                serde_json::json!(wbtc.address),
                serde_json::json!(self.cfg.wbtc_collateral_amount),
                serde_json::json!(accounts.borrower),
                serde_json::json!(0),
            ],
            None,
            "supply_wbtc_failed",
        )
        .await?;
        let supply_wbtc_receipt = evm_rpc::wait_for_receipt(
            &self.state_id,
            io,
            &supply_wbtc_tx,
            self.cfg.poll_interval_ms,
            self.cfg.max_receipt_polls,
        )
        .await?;
        evm_rpc::ensure_receipt_success(&supply_wbtc_receipt)?;
        txs.push(supply_wbtc_tx);

        op_ctx::write_json(
            ctx,
            ContextKey(KEY_SUPPLY_TX_HASHES.to_string()),
            serde_json::json!(txs),
        )?;
        Ok(StateOutcome {
            snapshot: SnapshotPolicy::OnSuccess,
        })
    }
}

#[async_trait]
impl State for BorrowAssetState {
    fn meta(&self) -> StateMeta {
        meta::apply_side_effect(op_idempotency::state_purpose(
            "aave_v3_scenario",
            &self.state_id,
            "borrow_asset",
        ))
    }

    async fn handle(
        &self,
        ctx: &mut dyn DynContext,
        io: &mut dyn IoProvider,
        _rec: &mut dyn EventRecorder,
    ) -> Result<StateOutcome, StateError> {
        let manifest = read_deploy_manifest_loaded(ctx)?;
        let accounts = read_typed::<AaveScenarioAccounts>(
            ctx,
            KEY_SCENARIO_ACCOUNTS,
            "missing_scenario_accounts",
            "missing scenario accounts in context",
            "scenario_accounts_invalid",
            "scenario accounts were invalid",
        )?;
        let pool = contract_from_manifest(&manifest, CONTRACT_POOL)?;
        let usdc = contract_from_manifest(&manifest, CONTRACT_USDC)?;

        let tx_hash = send_contract_transaction(
            io,
            &self.state_id,
            &accounts.borrower,
            pool,
            "borrow",
            vec![
                serde_json::json!(usdc.address),
                serde_json::json!(self.cfg.usdc_borrow_amount),
                serde_json::json!(self.cfg.borrow_rate_mode),
                serde_json::json!(0),
                serde_json::json!(accounts.borrower),
            ],
            None,
            "borrow_transaction_failed",
        )
        .await?;

        let receipt = evm_rpc::wait_for_receipt(
            &self.state_id,
            io,
            &tx_hash,
            self.cfg.poll_interval_ms,
            self.cfg.max_receipt_polls,
        )
        .await?;
        evm_rpc::ensure_receipt_success(&receipt)?;

        op_ctx::write_json(
            ctx,
            ContextKey(KEY_BORROW_TX_HASHES.to_string()),
            serde_json::json!([tx_hash]),
        )?;
        Ok(StateOutcome {
            snapshot: SnapshotPolicy::OnSuccess,
        })
    }
}

#[async_trait]
impl State for ReadPositionsState {
    fn meta(&self) -> StateMeta {
        meta::fetch_data()
    }

    async fn handle(
        &self,
        ctx: &mut dyn DynContext,
        io: &mut dyn IoProvider,
        _rec: &mut dyn EventRecorder,
    ) -> Result<StateOutcome, StateError> {
        let manifest = read_deploy_manifest_loaded(ctx)?;
        let accounts = read_typed::<AaveScenarioAccounts>(
            ctx,
            KEY_SCENARIO_ACCOUNTS,
            "missing_scenario_accounts",
            "missing scenario accounts in context",
            "scenario_accounts_invalid",
            "scenario accounts were invalid",
        )?;
        let pool = contract_from_manifest(&manifest, CONTRACT_POOL)?;
        let usdc = contract_from_manifest(&manifest, CONTRACT_USDC)?;
        let usdc_a_token = contract_from_manifest(&manifest, CONTRACT_USDC_A_TOKEN)?;
        let wbtc_a_token = contract_from_manifest(&manifest, CONTRACT_WBTC_A_TOKEN)?;
        let usdc_variable_debt =
            contract_from_manifest(&manifest, CONTRACT_USDC_VARIABLE_DEBT_TOKEN)?;

        let supplier_supplied = call_contract_u64(
            io,
            &self.state_id,
            usdc_a_token,
            "balanceOf",
            vec![serde_json::json!(accounts.supplier)],
        )
        .await?;
        let borrower_collateral = call_contract_u64(
            io,
            &self.state_id,
            wbtc_a_token,
            "balanceOf",
            vec![serde_json::json!(accounts.borrower)],
        )
        .await?;
        let borrower_borrowed = call_contract_u64(
            io,
            &self.state_id,
            usdc_variable_debt,
            "balanceOf",
            vec![serde_json::json!(accounts.borrower)],
        )
        .await?;
        let borrower_usdc_balance = call_contract_u64(
            io,
            &self.state_id,
            usdc,
            "balanceOf",
            vec![serde_json::json!(accounts.borrower)],
        )
        .await?;
        let pool_usdc_balance = call_contract_u64(
            io,
            &self.state_id,
            usdc,
            "balanceOf",
            vec![serde_json::json!(pool.address)],
        )
        .await?;

        let snapshot = AaveScenarioPositionSnapshot {
            supplier_supplied_usdc: supplier_supplied,
            borrower_collateral_wbtc: borrower_collateral,
            borrower_borrowed_usdc: borrower_borrowed,
            borrower_usdc_balance,
            pool_usdc_balance,
        };
        op_ctx::write_json(
            ctx,
            ContextKey(KEY_SCENARIO_POSITION_SNAPSHOT.to_string()),
            serde_json::to_value(snapshot).map_err(|_| {
                op_errors::state_unknown(
                    "serialize_position_snapshot_failed",
                    "failed to serialize scenario position snapshot",
                )
            })?,
        )?;
        Ok(StateOutcome {
            snapshot: SnapshotPolicy::OnSuccess,
        })
    }
}

#[async_trait]
impl State for AssertInvariantsState {
    fn meta(&self) -> StateMeta {
        meta::validate()
    }

    async fn handle(
        &self,
        ctx: &mut dyn DynContext,
        _io: &mut dyn IoProvider,
        _rec: &mut dyn EventRecorder,
    ) -> Result<StateOutcome, StateError> {
        let positions = read_typed::<AaveScenarioPositionSnapshot>(
            ctx,
            KEY_SCENARIO_POSITION_SNAPSHOT,
            "missing_position_snapshot",
            "missing scenario position snapshot in context",
            "position_snapshot_invalid",
            "scenario position snapshot was invalid",
        )?;

        assert_at_least_u64(
            positions.supplier_supplied_usdc,
            self.cfg.usdc_supply_amount,
            "scenario_assertion_failed",
            "supplier supplied usdc was lower than expected amount",
        )?;
        assert_at_least_u64(
            positions.borrower_collateral_wbtc,
            self.cfg.wbtc_collateral_amount,
            "scenario_assertion_failed",
            "borrower collateral wbtc was lower than expected amount",
        )?;
        assert_at_least_u64(
            positions.borrower_borrowed_usdc,
            self.cfg.usdc_borrow_amount,
            "scenario_assertion_failed",
            "borrower borrowed usdc was lower than expected amount",
        )?;
        assert_at_least_u64(
            positions.borrower_usdc_balance,
            self.cfg.usdc_borrow_amount,
            "scenario_assertion_failed",
            "borrower usdc balance was lower than expected borrowed amount",
        )?;
        let assertions = AaveScenarioAssertions {
            strict: true,
            passed: true,
        };
        op_ctx::write_json(
            ctx,
            ContextKey(KEY_SCENARIO_ASSERTIONS.to_string()),
            serde_json::to_value(assertions).map_err(|_| {
                op_errors::state_unknown(
                    "serialize_scenario_assertions_failed",
                    "failed to serialize scenario assertions",
                )
            })?,
        )?;
        Ok(StateOutcome {
            snapshot: SnapshotPolicy::OnSuccess,
        })
    }
}

#[async_trait]
impl State for WriteSummaryState {
    fn meta(&self) -> StateMeta {
        meta::pure()
    }

    async fn handle(
        &self,
        ctx: &mut dyn DynContext,
        io: &mut dyn IoProvider,
        rec: &mut dyn EventRecorder,
    ) -> Result<StateOutcome, StateError> {
        let chain_id = op_ctx::read_u64_required(
            ctx,
            &ContextKey(KEY_CHAIN_ID.to_string()),
            "missing_chain_id",
            "missing chain id in context",
        )?;
        let accounts = read_typed::<AaveScenarioAccounts>(
            ctx,
            KEY_SCENARIO_ACCOUNTS,
            "missing_scenario_accounts",
            "missing scenario accounts in context",
            "scenario_accounts_invalid",
            "scenario accounts were invalid",
        )?;
        let positions = read_typed::<AaveScenarioPositionSnapshot>(
            ctx,
            KEY_SCENARIO_POSITION_SNAPSHOT,
            "missing_position_snapshot",
            "missing scenario position snapshot in context",
            "position_snapshot_invalid",
            "scenario position snapshot was invalid",
        )?;
        let assertions = read_typed::<AaveScenarioAssertions>(
            ctx,
            KEY_SCENARIO_ASSERTIONS,
            "missing_scenario_assertions",
            "missing scenario assertions in context",
            "scenario_assertions_invalid",
            "scenario assertions were invalid",
        )?;
        let deploy_manifest = read_deploy_manifest_loaded(ctx)?;

        let report = AaveScenarioReport {
            kind: SCENARIO_REPORT_KIND.to_string(),
            chain_id,
            accounts,
            amounts: AaveScenarioAmounts {
                usdc_supply: self.cfg.usdc_supply_amount,
                wbtc_collateral: self.cfg.wbtc_collateral_amount,
                usdc_borrow: self.cfg.usdc_borrow_amount,
                borrow_rate_mode: self.cfg.borrow_rate_mode,
            },
            positions,
            assertions,
            deploy_manifest_kind: deploy_manifest.kind,
        };

        let report_json = serde_json::to_value(&report).map_err(|_| {
            op_errors::state_unknown(
                "serialize_scenario_report_failed",
                "failed to serialize scenario report",
            )
        })?;
        op_ctx::write_json(
            ctx,
            ContextKey(KEY_SCENARIO_REPORT_JSON.to_string()),
            report_json.clone(),
        )?;
        op_ctx::write_json(
            ctx,
            ContextKey(self.cfg.scenario_report_export_key.clone()),
            report_json.clone(),
        )?;

        op_output::write_output_artifact(
            ctx,
            io,
            rec,
            "aave.output",
            FactKey(format!("aave:output|op:{}", self.op_path)),
            report_json,
            ContextKey(self.cfg.scenario_report_artifact_key.clone()),
        )
        .await?;

        Ok(StateOutcome {
            snapshot: SnapshotPolicy::OnSuccess,
        })
    }
}

fn decode_compile_manifest(value: &serde_json::Value) -> Result<AaveCompileManifest, StateError> {
    let manifest: AaveCompileManifest = serde_json::from_value(value.clone()).map_err(|_| {
        op_errors::state_error(
            "compile_manifest_invalid",
            ErrorCategory::ParsingInput,
            false,
            "compile manifest was invalid",
        )
    })?;
    validate_compile_manifest(&manifest)?;
    Ok(manifest)
}

fn decode_deploy_manifest(value: &serde_json::Value) -> Result<AaveDeployManifest, StateError> {
    let manifest: AaveDeployManifest = serde_json::from_value(value.clone()).map_err(|_| {
        op_errors::state_error(
            "deploy_manifest_invalid",
            ErrorCategory::ParsingInput,
            false,
            "deploy manifest was invalid",
        )
    })?;
    validate_deploy_manifest(&manifest)?;
    Ok(manifest)
}

fn decode_origin_deploy_output(
    value: &serde_json::Value,
) -> Result<AaveOriginDeployOutput, StateError> {
    let output: AaveOriginDeployOutput = serde_json::from_value(value.clone()).map_err(|_| {
        op_errors::state_error(
            "origin_deploy_output_invalid",
            ErrorCategory::ParsingInput,
            false,
            "origin deploy output was invalid",
        )
    })?;
    validate_origin_deploy_output(&output)?;
    Ok(output)
}

fn validate_compile_manifest(manifest: &AaveCompileManifest) -> Result<(), StateError> {
    if manifest.kind != COMPILE_MANIFEST_KIND {
        return Err(op_errors::state_error(
            "compile_manifest_kind_mismatch",
            ErrorCategory::ParsingInput,
            false,
            "compile manifest kind mismatch",
        ));
    }
    if manifest.contracts.is_empty() {
        return Err(op_errors::state_error(
            "compile_manifest_empty",
            ErrorCategory::ParsingInput,
            false,
            "compile manifest must contain at least one contract",
        ));
    }
    let mut seen: HashSet<String> = HashSet::new();
    for c in &manifest.contracts {
        if c.id.trim().is_empty() {
            return Err(op_errors::state_error(
                "compile_manifest_contract_id_invalid",
                ErrorCategory::ParsingInput,
                false,
                "compile manifest contract id must be non-empty",
            ));
        }
        if !seen.insert(c.id.clone()) {
            return Err(op_errors::state_error(
                "compile_manifest_duplicate_contract_id",
                ErrorCategory::ParsingInput,
                false,
                "compile manifest contract ids must be unique",
            ));
        }
        let _ = c.artifact.parse()?;
    }
    Ok(())
}

fn validate_origin_deploy_output(output: &AaveOriginDeployOutput) -> Result<(), StateError> {
    if output.kind != ORIGIN_DEPLOY_OUTPUT_KIND {
        return Err(op_errors::state_error(
            "origin_deploy_output_kind_mismatch",
            ErrorCategory::ParsingInput,
            false,
            "origin deploy output kind mismatch",
        ));
    }
    if output.contracts.is_empty() {
        return Err(op_errors::state_error(
            "origin_deploy_output_empty",
            ErrorCategory::ParsingInput,
            false,
            "origin deploy output must contain at least one contract",
        ));
    }

    let mut seen: HashSet<String> = HashSet::new();
    for c in &output.contracts {
        if c.id.trim().is_empty() {
            return Err(op_errors::state_error(
                "origin_deploy_output_contract_id_invalid",
                ErrorCategory::ParsingInput,
                false,
                "origin deploy output contract id must be non-empty",
            ));
        }
        if !seen.insert(c.id.clone()) {
            return Err(op_errors::state_error(
                "origin_deploy_output_duplicate_contract_id",
                ErrorCategory::ParsingInput,
                false,
                "origin deploy output contract ids must be unique",
            ));
        }
        let _ = shared_dcv::normalize_address(&c.address).map_err(|_| {
            op_errors::state_error(
                "origin_deploy_output_contract_address_invalid",
                ErrorCategory::ParsingInput,
                false,
                "origin deploy output contract address was invalid",
            )
        })?;
        let _ = c.artifact.parse()?;
    }

    origin_contract_from_output(output, CONTRACT_USDC)?;
    origin_contract_from_output(output, CONTRACT_WBTC)?;
    origin_contract_from_output(output, CONTRACT_POOL)?;
    origin_contract_from_output(output, CONTRACT_USDC_A_TOKEN)?;
    origin_contract_from_output(output, CONTRACT_WBTC_A_TOKEN)?;
    origin_contract_from_output(output, CONTRACT_USDC_VARIABLE_DEBT_TOKEN)?;
    Ok(())
}

fn validate_deploy_manifest(manifest: &AaveDeployManifest) -> Result<(), StateError> {
    if manifest.kind != DEPLOY_MANIFEST_KIND {
        return Err(op_errors::state_error(
            "deploy_manifest_kind_mismatch",
            ErrorCategory::ParsingInput,
            false,
            "deploy manifest kind mismatch",
        ));
    }
    if manifest.contracts.is_empty() {
        return Err(op_errors::state_error(
            "deploy_manifest_empty",
            ErrorCategory::ParsingInput,
            false,
            "deploy manifest must contain at least one contract",
        ));
    }
    let mut seen: HashSet<String> = HashSet::new();
    for c in &manifest.contracts {
        if c.id.trim().is_empty() {
            return Err(op_errors::state_error(
                "deploy_manifest_contract_id_invalid",
                ErrorCategory::ParsingInput,
                false,
                "deploy manifest contract id must be non-empty",
            ));
        }
        if !seen.insert(c.id.clone()) {
            return Err(op_errors::state_error(
                "deploy_manifest_duplicate_contract_id",
                ErrorCategory::ParsingInput,
                false,
                "deploy manifest contract ids must be unique",
            ));
        }
        let _ = shared_dcv::normalize_address(&c.address).map_err(|_| {
            op_errors::state_error(
                "deploy_manifest_contract_address_invalid",
                ErrorCategory::ParsingInput,
                false,
                "deploy manifest contract address was invalid",
            )
        })?;
        let _ = c.artifact.parse()?;
    }
    contract_from_manifest(manifest, CONTRACT_USDC)?;
    contract_from_manifest(manifest, CONTRACT_WBTC)?;
    contract_from_manifest(manifest, CONTRACT_POOL)?;
    Ok(())
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

fn read_typed<T: serde::de::DeserializeOwned>(
    ctx: &dyn DynContext,
    key: &str,
    missing_code: &'static str,
    missing_message: &'static str,
    type_code: &'static str,
    type_message: &'static str,
) -> Result<T, StateError> {
    let value = op_ctx::read_json_required(
        ctx,
        &ContextKey(key.to_string()),
        missing_code,
        missing_message,
    )?;
    serde_json::from_value(value).map_err(|_| op_errors::state_unknown(type_code, type_message))
}

fn contract_from_manifest<'a>(
    manifest: &'a AaveDeployManifest,
    id: &str,
) -> Result<&'a AaveDeployManifestContract, StateError> {
    manifest
        .contracts
        .iter()
        .find(|c| c.id == id)
        .ok_or_else(|| {
            op_errors::state_error(
                "deploy_manifest_missing_contract",
                ErrorCategory::ParsingInput,
                false,
                format!("deploy manifest missing contract: {id}"),
            )
        })
}

fn origin_contract_from_output<'a>(
    output: &'a AaveOriginDeployOutput,
    id: &str,
) -> Result<&'a AaveOriginDeployOutputContract, StateError> {
    output.contracts.iter().find(|c| c.id == id).ok_or_else(|| {
        op_errors::state_error(
            "origin_deploy_output_missing_contract",
            ErrorCategory::ParsingInput,
            false,
            format!("origin deploy output missing contract: {id}"),
        )
    })
}

fn parse_wei_u128(raw: &str) -> Result<u128, StateError> {
    if raw.starts_with("0x") || raw.starts_with("0X") {
        let normalized = shared_dcv::normalize_hex_str(raw).map_err(|_| {
            op_errors::state_error(
                "fund_wei_invalid",
                ErrorCategory::ParsingInput,
                false,
                "fund_wei must be a valid decimal or 0x hex string",
            )
        })?;
        return evm_rpc::quantity_hex_to_u128(
            &normalized,
            "fund_wei must be a valid decimal or 0x hex string",
        )
        .map_err(|_| {
            op_errors::state_error(
                "fund_wei_invalid",
                ErrorCategory::ParsingInput,
                false,
                "fund_wei must be a valid decimal or 0x hex string",
            )
        });
    }
    raw.parse::<u128>().map_err(|_| {
        op_errors::state_error(
            "fund_wei_invalid",
            ErrorCategory::ParsingInput,
            false,
            "fund_wei must be a valid decimal or 0x hex string",
        )
    })
}

async fn pending_nonce_u128(
    io: &mut dyn IoProvider,
    state_id: &StateId,
    from: &str,
) -> Result<u128, StateError> {
    let mut client = EvmIoClient::new(state_id.clone(), io);
    let nonce_hex = evm_rpc::transaction_count_hex(&mut client, from).await?;
    evm_rpc::parse_quantity_hex_u128(
        &nonce_hex,
        "eth_getTransactionCount returned invalid hex nonce",
    )
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

async fn send_contract_transaction(
    io: &mut dyn IoProvider,
    state_id: &StateId,
    from: &str,
    contract: &AaveDeployManifestContract,
    function: &str,
    args: Vec<serde_json::Value>,
    value: Option<String>,
    err_code: &'static str,
) -> Result<String, StateError> {
    let from_addr = shared_dcv::normalize_address(from).map_err(|_| {
        op_errors::state_error(
            "invalid_from_address",
            ErrorCategory::ParsingInput,
            false,
            "from address was invalid",
        )
    })?;
    let data = encode_call_data(contract, function, args)?;
    let mut tx = serde_json::json!({
        "from": from_addr,
        "to": contract.address,
        "data": data,
    });
    if let Some(v) = value {
        tx["value"] = serde_json::json!(v);
    }

    let mut client = EvmIoClient::new(state_id.clone(), io);
    evm_rpc::send_transaction(&mut client, tx)
        .await
        .map_err(|_| {
            op_errors::state_error(
                err_code,
                ErrorCategory::OnChain,
                false,
                "contract transaction failed to submit",
            )
        })
}

async fn call_contract_u64(
    io: &mut dyn IoProvider,
    state_id: &StateId,
    contract: &AaveDeployManifestContract,
    function: &str,
    args: Vec<serde_json::Value>,
) -> Result<u64, StateError> {
    let (abi, _bytecode) = contract.artifact.parse()?;
    let (calldata, outputs) =
        common_abi::resolve_function_call(&abi, function, &args).map_err(|_| {
            op_errors::state_error(
                "invalid_function_call",
                ErrorCategory::ParsingInput,
                false,
                "function call did not match ABI",
            )
        })?;
    let data_hex = shared_dcv::bytes_to_hex_prefixed(&calldata);

    let mut client = EvmIoClient::new(state_id.clone(), io);
    let res = client
        .call(JsonRpcCall::new(
            "eth_call",
            serde_json::json!([
                {"to": contract.address, "data": data_hex},
                "latest"
            ]),
        ))
        .await
        .map_err(op_errors::state_from_io)?;
    let raw = op_rpc::expect_string(
        &res.response,
        "evm_response_invalid",
        "eth_call returned non-string response",
    )?;

    let decoded = shared_dcv::decode_single_output_to_json(&outputs, &raw).map_err(|_| {
        op_errors::state_unknown("evm_response_invalid", "failed to decode eth_call output")
    })?;
    decoded.as_u64().ok_or_else(|| {
        op_errors::state_unknown(
            "evm_response_invalid",
            "decoded eth_call output was not u64",
        )
    })
}

fn assert_at_least_u64(
    actual: u64,
    minimum: u64,
    code: &'static str,
    message: &'static str,
) -> Result<(), StateError> {
    if actual >= minimum {
        return Ok(());
    }
    Err(op_errors::state_error(
        code,
        ErrorCategory::ParsingInput,
        false,
        message,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::HashMap;

    use async_trait::async_trait;
    use mfm_machine::errors::{ContextError, ErrorCategory, IoError, RunError};
    use mfm_machine::events::DomainEvent;
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

    fn valid_scenario_config() -> AaveScenarioConfig {
        AaveScenarioConfig {
            deploy_manifest_port: "deploy_manifest".to_string(),
            funder_account_index: 0,
            supplier_account_index: 1,
            borrower_account_index: 2,
            fund_wei: "1000000000000000000".to_string(),
            usdc_supply_amount: 1_000_000_000_000,
            wbtc_collateral_amount: 1_000_000_000,
            usdc_borrow_amount: 1_000_000,
            borrow_rate_mode: 2,
            poll_interval_ms: 200,
            max_receipt_polls: 120,
            scenario_report_export_key: "scenario_report".to_string(),
            scenario_report_artifact_key: "scenario_report_artifact_id".to_string(),
        }
    }

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

    #[derive(Default)]
    struct FundWalletTestIo {
        balances: HashMap<String, u128>,
        sent_values: Vec<String>,
        tx_counter: u64,
    }

    impl FundWalletTestIo {
        fn with_balances(entries: &[(&str, u128)]) -> Self {
            let balances = entries
                .iter()
                .map(|(account, balance)| ((*account).to_string(), *balance))
                .collect::<HashMap<_, _>>();
            Self {
                balances,
                ..Default::default()
            }
        }
    }

    #[async_trait]
    impl IoProvider for FundWalletTestIo {
        async fn call(&mut self, call: IoCall) -> Result<IoResult, IoError> {
            let method = call
                .request
                .get("method")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            if call.namespace != "evm" {
                return Err(IoError::Other(crate::errors::info(
                    "unexpected_io_namespace",
                    ErrorCategory::Unknown,
                    false,
                    "unexpected io namespace",
                )));
            }

            match method {
                "eth_getBalance" => {
                    let account = call
                        .request
                        .get("params")
                        .and_then(|v| v.as_array())
                        .and_then(|params| params.first())
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| {
                            IoError::Other(crate::errors::info(
                                "unexpected_io_call",
                                ErrorCategory::Unknown,
                                false,
                                "missing account parameter",
                            ))
                        })?;
                    let balance = self.balances.get(account).copied().unwrap_or(0);
                    Ok(IoResult {
                        response: serde_json::json!(format!("0x{balance:x}")),
                        recorded_payload_id: None,
                    })
                }
                "eth_estimateGas" => Ok(IoResult {
                    response: serde_json::json!("0x5208"),
                    recorded_payload_id: None,
                }),
                "eth_gasPrice" => Ok(IoResult {
                    response: serde_json::json!("0x1"),
                    recorded_payload_id: None,
                }),
                "eth_sendTransaction" => {
                    let value = call
                        .request
                        .get("params")
                        .and_then(|v| v.as_array())
                        .and_then(|params| params.first())
                        .and_then(|tx| tx.get("value"))
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| {
                            IoError::Other(crate::errors::info(
                                "unexpected_io_call",
                                ErrorCategory::Unknown,
                                false,
                                "missing transaction value",
                            ))
                        })?;
                    self.sent_values.push(value.to_string());
                    self.tx_counter += 1;
                    Ok(IoResult {
                        response: serde_json::json!(format!("0x{:064x}", self.tx_counter)),
                        recorded_payload_id: None,
                    })
                }
                "eth_getTransactionReceipt" => Ok(IoResult {
                    response: serde_json::json!({ "status": "0x1" }),
                    recorded_payload_id: None,
                }),
                _ => Err(IoError::Other(crate::errors::info(
                    "unexpected_io_call",
                    ErrorCategory::Unknown,
                    false,
                    "unexpected io call",
                ))),
            }
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

    #[tokio::test]
    async fn fund_wallet_transfers_only_deficit_for_underfunded_accounts() {
        let state = FundWalletState {
            state_id: StateId("m.test.aave_v3.fund_wallet".to_string()),
            cfg: valid_scenario_config(),
        };
        let fund_wei = parse_wei_u128(&state.cfg.fund_wei).expect("fund_wei");
        let supplier = "0x00000000000000000000000000000000000000bb";
        let borrower = "0x00000000000000000000000000000000000000cc";
        let supplier_deficit = 100_000_000_000_000_000u128;
        let borrower_deficit = 750_000_000_000_000_000u128;

        let mut io = FundWalletTestIo::with_balances(&[
            (supplier, fund_wei - supplier_deficit),
            (borrower, fund_wei - borrower_deficit),
        ]);
        let mut ctx = MapContext::default();
        op_ctx::write_json(
            &mut ctx,
            ContextKey(KEY_SCENARIO_ACCOUNTS.to_string()),
            serde_json::to_value(AaveScenarioAccounts {
                funder: "0x00000000000000000000000000000000000000aa".to_string(),
                supplier: supplier.to_string(),
                borrower: borrower.to_string(),
            })
            .expect("serialize scenario accounts"),
        )
        .expect("write scenario accounts");
        let mut rec = NoopRecorder;

        state
            .handle(&mut ctx, &mut io, &mut rec)
            .await
            .expect("fund wallet");

        assert_eq!(
            io.sent_values,
            vec![
                format!("0x{supplier_deficit:x}"),
                format!("0x{borrower_deficit:x}")
            ]
        );
        let tx_hashes = ctx
            .read(&ContextKey(KEY_FUNDING_TX_HASHES.to_string()))
            .expect("read funding tx hashes")
            .expect("funding tx hashes should be written");
        assert_eq!(tx_hashes.as_array().map(|values| values.len()), Some(2));
    }

    #[test]
    fn scenario_config_requires_explicit_fields_on_deserialize() {
        let cfg = serde_json::from_value::<AaveScenarioConfig>(serde_json::json!({
            "deploy_manifest_port": "deploy_manifest",
            "poll_interval_ms": 200,
            "max_receipt_polls": 120,
            "scenario_report_export_key": "scenario_report",
            "scenario_report_artifact_key": "scenario_report_artifact_id"
        }));
        assert!(cfg.is_err());
    }

    #[test]
    fn validate_scenario_config_rejects_duplicate_account_indexes() {
        let mut cfg = valid_scenario_config();
        cfg.supplier_account_index = cfg.funder_account_index;

        let err = validate_scenario_config(&cfg).expect_err("expected invalid config");
        assert_eq!(
            err,
            "funder_account_index, supplier_account_index, and borrower_account_index must differ"
        );
    }

    #[test]
    fn validate_scenario_config_rejects_zero_fund_wei() {
        let mut cfg = valid_scenario_config();
        cfg.fund_wei = "0".to_string();

        let err = validate_scenario_config(&cfg).expect_err("expected invalid config");
        assert_eq!(err, "fund_wei must be > 0");
    }

    #[test]
    fn validate_scenario_config_rejects_invalid_fund_wei() {
        let mut cfg = valid_scenario_config();
        cfg.fund_wei = "not-a-number".to_string();

        let err = validate_scenario_config(&cfg).expect_err("expected invalid config");
        assert_eq!(err, "fund_wei must be a valid decimal or 0x hex string");
    }

    #[test]
    fn validate_scenario_config_rejects_zero_usdc_supply_amount() {
        let mut cfg = valid_scenario_config();
        cfg.usdc_supply_amount = 0;

        let err = validate_scenario_config(&cfg).expect_err("expected invalid config");
        assert_eq!(err, "usdc_supply_amount must be > 0");
    }

    #[test]
    fn validate_scenario_config_rejects_zero_wbtc_collateral_amount() {
        let mut cfg = valid_scenario_config();
        cfg.wbtc_collateral_amount = 0;

        let err = validate_scenario_config(&cfg).expect_err("expected invalid config");
        assert_eq!(err, "wbtc_collateral_amount must be > 0");
    }

    #[test]
    fn validate_scenario_config_rejects_zero_usdc_borrow_amount() {
        let mut cfg = valid_scenario_config();
        cfg.usdc_borrow_amount = 0;

        let err = validate_scenario_config(&cfg).expect_err("expected invalid config");
        assert_eq!(err, "usdc_borrow_amount must be > 0");
    }

    #[test]
    fn validate_scenario_config_rejects_zero_borrow_rate_mode() {
        let mut cfg = valid_scenario_config();
        cfg.borrow_rate_mode = 0;

        let err = validate_scenario_config(&cfg).expect_err("expected invalid config");
        assert_eq!(err, "borrow_rate_mode must be > 0");
    }

    #[test]
    fn validate_scenario_config_accepts_explicit_non_zero_config() {
        let cfg = valid_scenario_config();
        validate_scenario_config(&cfg).expect("expected valid config");
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
