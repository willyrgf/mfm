use std::collections::HashSet;
use std::time::Duration;

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
use crate::idempotency as op_idempotency;
use crate::output as op_output;
use crate::rpc as op_rpc;
use crate::states::evm_dcv as shared_dcv;
use crate::states::meta;

pub const COMPILE_MANIFEST_KIND: &str = "aave_v3_compile_manifest_v1";
pub const DEPLOY_MANIFEST_KIND: &str = "aave_v3_deploy_manifest_v1";
pub const CONFIG_REPORT_KIND: &str = "aave_v3_config_report_v1";
pub const SCENARIO_REPORT_KIND: &str = "aave_v3_reth_scenario_report_v1";

pub const CONTRACT_USDC: &str = "usdc";
pub const CONTRACT_WBTC: &str = "wbtc";
pub const CONTRACT_POOL: &str = "pool";

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

fn default_config_report_port() -> String {
    "config_report".to_string()
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

fn default_funder_account_index() -> usize {
    0
}

fn default_merican_account_index() -> usize {
    1
}

fn default_saylor_account_index() -> usize {
    2
}

fn default_usdc_supply_amount() -> u64 {
    1_000_000_000_000
}

fn default_wbtc_collateral_amount() -> u64 {
    1_000_000_000
}

fn default_usdc_borrow_amount() -> u64 {
    400_000_000_000
}

fn default_borrow_rate_mode() -> u64 {
    2
}

fn default_fund_wei() -> String {
    "1000000000000000000".to_string()
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
    pub merican: String,
    pub saylor: String,
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
    pub merican_supplied_usdc: u64,
    pub saylor_supplied_usdc: u64,
    pub saylor_collateral_wbtc: u64,
    pub saylor_borrowed_usdc: u64,
    pub saylor_usdc_balance: u64,
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
    pub config_report_kind: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AaveDeployRuntimeConfig {
    #[serde(default = "default_compile_manifest_port")]
    pub compile_manifest_port: String,

    #[serde(default = "default_deployer_account_index")]
    pub deployer_account_index: usize,

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

    #[serde(default = "default_config_report_port")]
    pub config_report_port: String,

    #[serde(default = "default_funder_account_index")]
    pub funder_account_index: usize,

    #[serde(default = "default_merican_account_index")]
    pub merican_account_index: usize,

    #[serde(default = "default_saylor_account_index")]
    pub saylor_account_index: usize,

    #[serde(default = "default_fund_wei")]
    pub fund_wei: String,

    #[serde(default = "default_usdc_supply_amount")]
    pub usdc_supply_amount: u64,

    #[serde(default = "default_wbtc_collateral_amount")]
    pub wbtc_collateral_amount: u64,

    #[serde(default = "default_usdc_borrow_amount")]
    pub usdc_borrow_amount: u64,

    #[serde(default = "default_borrow_rate_mode")]
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
    if cfg.config_report_port.trim().is_empty() {
        return Err("config_report_port must be non-empty".to_string());
    }
    if cfg.scenario_report_export_key.trim().is_empty() {
        return Err("scenario_report_export_key must be non-empty".to_string());
    }
    if cfg.scenario_report_artifact_key.trim().is_empty() {
        return Err("scenario_report_artifact_key must be non-empty".to_string());
    }
    if cfg.merican_account_index == cfg.saylor_account_index {
        return Err("merican_account_index and saylor_account_index must differ".to_string());
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
        let deployer =
            resolve_account_by_index(io, &self.state_id, self.cfg.deployer_account_index).await?;

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
            let tx_hash = send_transaction(
                io,
                &self.state_id,
                serde_json::json!({
                    "from": deployer,
                    "data": shared_dcv::bytes_to_hex_prefixed(&constructor_payload),
                }),
            )
            .await?;

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
            let receipt = wait_for_receipt(
                io,
                &self.state_id,
                &p.tx_hash,
                self.poll_interval_ms,
                self.max_receipt_polls,
            )
            .await?;
            ensure_receipt_success(&receipt, "deploy_receipt_failed")?;
            let contract_address = receipt_contract_address(&receipt)?;
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
            resolve_account_by_index(io, &self.state_id, self.cfg.from_account_index).await?;
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
            let receipt = wait_for_receipt(
                io,
                &self.state_id,
                &p.tx_hash,
                self.poll_interval_ms,
                self.max_receipt_polls,
            )
            .await?;
            ensure_receipt_success(&receipt, "configure_receipt_failed")?;
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
        let accounts = rpc_accounts(io, &self.state_id).await?;
        let funder = account_at(
            &accounts,
            self.cfg.funder_account_index,
            "invalid_funder_account_index",
        )?;
        let merican = account_at(
            &accounts,
            self.cfg.merican_account_index,
            "invalid_merican_account_index",
        )?;
        let saylor = account_at(
            &accounts,
            self.cfg.saylor_account_index,
            "invalid_saylor_account_index",
        )?;

        let resolved = AaveScenarioAccounts {
            funder,
            merican,
            saylor,
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
        let value_hex = wei_to_hex(&self.cfg.fund_wei)?;

        let mut tx_hashes = Vec::new();
        for to in [&accounts.merican, &accounts.saylor] {
            let tx_hash = send_transaction(
                io,
                &self.state_id,
                serde_json::json!({
                    "from": accounts.funder,
                    "to": to,
                    "value": value_hex,
                }),
            )
            .await?;

            let receipt = wait_for_receipt(
                io,
                &self.state_id,
                &tx_hash,
                self.cfg.poll_interval_ms,
                self.cfg.max_receipt_polls,
            )
            .await?;
            ensure_receipt_success(&receipt, "fund_wallet_receipt_failed")?;
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
        txs.push(
            send_contract_transaction(
                io,
                &self.state_id,
                &accounts.merican,
                usdc,
                "approve",
                vec![
                    serde_json::json!(pool.address),
                    serde_json::json!(self.cfg.usdc_supply_amount),
                ],
                None,
                "approve_usdc_failed",
            )
            .await?,
        );
        txs.push(
            send_contract_transaction(
                io,
                &self.state_id,
                &accounts.saylor,
                wbtc,
                "approve",
                vec![
                    serde_json::json!(pool.address),
                    serde_json::json!(self.cfg.wbtc_collateral_amount),
                ],
                None,
                "approve_wbtc_failed",
            )
            .await?,
        );

        for tx_hash in &txs {
            let receipt = wait_for_receipt(
                io,
                &self.state_id,
                tx_hash,
                self.cfg.poll_interval_ms,
                self.cfg.max_receipt_polls,
            )
            .await?;
            ensure_receipt_success(&receipt, "approve_receipt_failed")?;
        }

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
        txs.push(
            send_contract_transaction(
                io,
                &self.state_id,
                &accounts.merican,
                pool,
                "supply",
                vec![
                    serde_json::json!(usdc.address),
                    serde_json::json!(self.cfg.usdc_supply_amount),
                    serde_json::json!(accounts.merican),
                    serde_json::json!(0),
                ],
                None,
                "supply_usdc_failed",
            )
            .await?,
        );
        txs.push(
            send_contract_transaction(
                io,
                &self.state_id,
                &accounts.saylor,
                pool,
                "supply",
                vec![
                    serde_json::json!(wbtc.address),
                    serde_json::json!(self.cfg.wbtc_collateral_amount),
                    serde_json::json!(accounts.saylor),
                    serde_json::json!(0),
                ],
                None,
                "supply_wbtc_failed",
            )
            .await?,
        );

        for tx_hash in &txs {
            let receipt = wait_for_receipt(
                io,
                &self.state_id,
                tx_hash,
                self.cfg.poll_interval_ms,
                self.cfg.max_receipt_polls,
            )
            .await?;
            ensure_receipt_success(&receipt, "supply_receipt_failed")?;
        }

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
            &accounts.saylor,
            pool,
            "borrow",
            vec![
                serde_json::json!(usdc.address),
                serde_json::json!(self.cfg.usdc_borrow_amount),
                serde_json::json!(self.cfg.borrow_rate_mode),
                serde_json::json!(0),
                serde_json::json!(accounts.saylor),
            ],
            None,
            "borrow_transaction_failed",
        )
        .await?;

        let receipt = wait_for_receipt(
            io,
            &self.state_id,
            &tx_hash,
            self.cfg.poll_interval_ms,
            self.cfg.max_receipt_polls,
        )
        .await?;
        ensure_receipt_success(&receipt, "borrow_failed")?;

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

        let merican_supplied = call_contract_u64(
            io,
            &self.state_id,
            pool,
            "suppliedUsdcOf",
            vec![serde_json::json!(accounts.merican)],
        )
        .await?;
        let saylor_supplied = call_contract_u64(
            io,
            &self.state_id,
            pool,
            "suppliedUsdcOf",
            vec![serde_json::json!(accounts.saylor)],
        )
        .await?;
        let saylor_collateral = call_contract_u64(
            io,
            &self.state_id,
            pool,
            "collateralWbtcOf",
            vec![serde_json::json!(accounts.saylor)],
        )
        .await?;
        let saylor_borrowed = call_contract_u64(
            io,
            &self.state_id,
            pool,
            "borrowedUsdcOf",
            vec![serde_json::json!(accounts.saylor)],
        )
        .await?;
        let saylor_usdc_balance = call_contract_u64(
            io,
            &self.state_id,
            usdc,
            "balanceOf",
            vec![serde_json::json!(accounts.saylor)],
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
            merican_supplied_usdc: merican_supplied,
            saylor_supplied_usdc: saylor_supplied,
            saylor_collateral_wbtc: saylor_collateral,
            saylor_borrowed_usdc: saylor_borrowed,
            saylor_usdc_balance,
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

        assert_equal_u64(
            positions.merican_supplied_usdc,
            self.cfg.usdc_supply_amount,
            "scenario_assertion_failed",
            "merican supplied usdc did not match expected amount",
        )?;
        assert_equal_u64(
            positions.saylor_collateral_wbtc,
            self.cfg.wbtc_collateral_amount,
            "scenario_assertion_failed",
            "saylor collateral wbtc did not match expected amount",
        )?;
        assert_equal_u64(
            positions.saylor_borrowed_usdc,
            self.cfg.usdc_borrow_amount,
            "scenario_assertion_failed",
            "saylor borrowed usdc did not match expected amount",
        )?;
        assert_equal_u64(
            positions.saylor_usdc_balance,
            self.cfg.usdc_borrow_amount,
            "scenario_assertion_failed",
            "saylor usdc balance did not match expected borrowed amount",
        )?;
        let expected_pool_usdc = self
            .cfg
            .usdc_supply_amount
            .checked_sub(self.cfg.usdc_borrow_amount)
            .ok_or_else(|| {
                op_errors::state_unknown(
                    "scenario_assertion_failed",
                    "expected pool usdc underflow",
                )
            })?;
        assert_equal_u64(
            positions.pool_usdc_balance,
            expected_pool_usdc,
            "scenario_assertion_failed",
            "pool usdc balance did not match expected remaining liquidity",
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
        let config_report = read_config_report(ctx, &self.cfg.config_report_port)?;

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
            config_report_kind: config_report.kind,
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

fn read_config_report(
    ctx: &dyn DynContext,
    config_report_port: &str,
) -> Result<AaveConfigReport, StateError> {
    let value = op_ctx::read_json_required(
        ctx,
        &ContextKey(config_report_port.to_string()),
        "missing_config_report",
        "missing config report in context",
    )?;
    let report: AaveConfigReport = serde_json::from_value(value).map_err(|_| {
        op_errors::state_error(
            "config_report_invalid",
            ErrorCategory::ParsingInput,
            false,
            "config report was invalid",
        )
    })?;
    if report.kind != CONFIG_REPORT_KIND {
        return Err(op_errors::state_error(
            "config_report_kind_mismatch",
            ErrorCategory::ParsingInput,
            false,
            "config report kind mismatch",
        ));
    }
    Ok(report)
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

async fn rpc_accounts(
    io: &mut dyn IoProvider,
    state_id: &StateId,
) -> Result<Vec<String>, StateError> {
    let mut client = EvmIoClient::new(state_id.clone(), io);
    let res = client
        .call(JsonRpcCall::new("eth_accounts", serde_json::json!([])))
        .await
        .map_err(op_errors::state_from_io)?;
    let arr = op_rpc::expect_array(
        &res.response,
        "evm_response_invalid",
        "eth_accounts returned non-array",
    )?;

    let mut out = Vec::with_capacity(arr.len());
    for v in arr {
        let raw = op_rpc::expect_string(
            v,
            "evm_response_invalid",
            "eth_accounts entry was not a string",
        )?;
        let normalized = shared_dcv::normalize_address(&raw).map_err(|_| {
            op_errors::state_unknown(
                "evm_response_invalid",
                "eth_accounts entry was not a valid address",
            )
        })?;
        out.push(normalized);
    }
    Ok(out)
}

fn account_at(accounts: &[String], idx: usize, code: &'static str) -> Result<String, StateError> {
    accounts.get(idx).cloned().ok_or_else(|| {
        op_errors::state_error(
            code,
            ErrorCategory::ParsingInput,
            false,
            "account index was out of range",
        )
    })
}

async fn resolve_account_by_index(
    io: &mut dyn IoProvider,
    state_id: &StateId,
    idx: usize,
) -> Result<String, StateError> {
    let accounts = rpc_accounts(io, state_id).await?;
    account_at(&accounts, idx, "invalid_account_index")
}

fn wei_to_hex(raw: &str) -> Result<String, StateError> {
    if raw.starts_with("0x") || raw.starts_with("0X") {
        return shared_dcv::normalize_hex_str(raw).map_err(|_| {
            op_errors::state_error(
                "fund_wei_invalid",
                ErrorCategory::ParsingInput,
                false,
                "fund_wei must be a valid decimal or 0x hex string",
            )
        });
    }
    let v = raw.parse::<u128>().map_err(|_| {
        op_errors::state_error(
            "fund_wei_invalid",
            ErrorCategory::ParsingInput,
            false,
            "fund_wei must be a valid decimal or 0x hex string",
        )
    })?;
    Ok(format!("0x{v:x}"))
}

async fn send_transaction(
    io: &mut dyn IoProvider,
    state_id: &StateId,
    tx_obj: serde_json::Value,
) -> Result<String, StateError> {
    let mut client = EvmIoClient::new(state_id.clone(), io);
    let res = client
        .call(JsonRpcCall::new(
            "eth_sendTransaction",
            serde_json::json!([tx_obj]),
        ))
        .await
        .map_err(op_errors::state_from_io)?;
    let tx_hash = op_rpc::expect_string(
        &res.response,
        "evm_response_invalid",
        "eth_sendTransaction returned non-string tx hash",
    )?;
    shared_dcv::normalize_hex_str(&tx_hash).map_err(|_| {
        op_errors::state_unknown(
            "evm_response_invalid",
            "eth_sendTransaction returned invalid tx hash",
        )
    })
}

async fn wait_for_receipt(
    io: &mut dyn IoProvider,
    state_id: &StateId,
    tx_hash: &str,
    poll_interval_ms: u64,
    max_receipt_polls: u64,
) -> Result<serde_json::Value, StateError> {
    if max_receipt_polls == 0 {
        return Err(op_errors::state_unknown(
            "invalid_op_config",
            "max_receipt_polls must be > 0",
        ));
    }

    let normalized_tx = shared_dcv::normalize_hex_str(tx_hash)
        .map_err(|_| op_errors::state_unknown("invalid_tx_hash", "tx hash was invalid hex"))?;

    let mut client = EvmIoClient::new(state_id.clone(), io);
    for _ in 0..max_receipt_polls {
        let res = client
            .call(JsonRpcCall::new(
                "eth_getTransactionReceipt",
                serde_json::json!([normalized_tx]),
            ))
            .await
            .map_err(op_errors::state_from_io)?;
        if !res.response.is_null() {
            return Ok(res.response);
        }
        tokio::time::sleep(Duration::from_millis(poll_interval_ms)).await;
    }

    Err(op_errors::state_unknown(
        "receipt_timeout",
        "timed out while waiting for receipt",
    ))
}

fn ensure_receipt_success(
    receipt: &serde_json::Value,
    code: &'static str,
) -> Result<(), StateError> {
    let Some(status) = receipt.get("status").and_then(|v| v.as_str()) else {
        return Err(op_errors::state_unknown(
            "evm_response_invalid",
            "receipt status was missing",
        ));
    };
    let normalized = shared_dcv::normalize_hex_str(status).map_err(|_| {
        op_errors::state_unknown("evm_response_invalid", "receipt status was invalid hex")
    })?;
    if normalized != "0x01" && normalized != "0x1" {
        return Err(op_errors::state_error(
            code,
            ErrorCategory::OnChain,
            false,
            "transaction receipt status was unsuccessful",
        ));
    }
    Ok(())
}

fn receipt_contract_address(receipt: &serde_json::Value) -> Result<String, StateError> {
    let address = receipt
        .get("contractAddress")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            op_errors::state_unknown(
                "deploy_receipt_missing_contract_address",
                "deployment receipt missing contractAddress",
            )
        })?;
    shared_dcv::normalize_address(address).map_err(|_| {
        op_errors::state_unknown(
            "deploy_receipt_invalid_contract_address",
            "deployment receipt contractAddress was invalid",
        )
    })
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

    send_transaction(io, state_id, tx).await.map_err(|_| {
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

fn assert_equal_u64(
    actual: u64,
    expected: u64,
    code: &'static str,
    message: &'static str,
) -> Result<(), StateError> {
    if actual == expected {
        return Ok(());
    }
    Err(op_errors::state_error(
        code,
        ErrorCategory::ParsingInput,
        false,
        message,
    ))
}
