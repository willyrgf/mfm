#![cfg(feature = "parity-tests")]
#![allow(clippy::disallowed_methods)]

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use mfm_aave_v3_origin_config::{
    AaveV3OriginSourceConfig, AaveV3OriginStackCanonicalConfig, AAVE_V3_ORIGIN_BACKEND_COMMIT_SHA,
    AAVE_V3_ORIGIN_BACKEND_REPO_URL,
};
use mfm_artifact_store_s3::S3ArtifactStore;
use mfm_integration_tests::parity_run_ids::write_parity_aave_run_ids;
use mfm_integration_tests::rpc_control;
use mfm_machine::config::{
    BackoffPolicy, BuildProvenance, ContextCheckpointing, EventProfile, ExecutionMode, IoMode,
    RetryPolicy, RunConfig,
};
use mfm_machine::context::DynContext;
use mfm_machine::engine::{RunPhase, Stores};
use mfm_machine::errors::ContextError;
use mfm_machine::events::{event_envelopes_from_stream_records, Event, KernelEvent};
use mfm_machine::ids::{ContextKey, OpId, RunId};
use mfm_machine::stores::{ArtifactKind, ArtifactStore, StreamId, StreamStore};
use mfm_machine_test_support::init_test_observability;
use mfm_sdk::ids::{MachineId, StepId};
use mfm_sdk::launcher::{LaunchPipeline, RunLauncher};
use mfm_sdk::pipeline::{Pipeline, PipelineStep};
use mfm_sdk::unstable::DefaultRunLauncher;
use mfm_stream_store_postgres::PostgresStreamStore;
use mfm_transports_rpc_control::RpcControlBootstrapSource;
use serde::{Deserialize, Serialize};
use tower::ServiceExt;

const RETH_DEV_ACCOUNT0_PRIVATE_KEY: &str =
    "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
const RETH_DEV_ACCOUNT1_PRIVATE_KEY: &str =
    "0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d";
const RETH_DEV_ACCOUNT2_PRIVATE_KEY: &str =
    "0x5de4111afa1a4b94908f83103eb1f1706367c2e68ca870fc3fb9a804cdab365a";

const SCENARIO_REPORT_KIND: &str = "aave_v3_reth_scenario_report_v1";
const DEPLOY_MANIFEST_KIND: &str = "aave_v3_deploy_manifest_v1";

const CONTRACT_USDC: &str = "usdc";
const CONTRACT_WBTC: &str = "wbtc";
const CONTRACT_POOL: &str = "pool";
const CONTRACT_USDC_A_TOKEN: &str = "usdc_a_token";
const CONTRACT_WBTC_A_TOKEN: &str = "wbtc_a_token";
const CONTRACT_USDC_VARIABLE_DEBT_TOKEN: &str = "usdc_variable_debt_token";

const USDC_SUPPLY_AMOUNT: u64 = 1_000_000_000_000;
const WBTC_COLLATERAL_AMOUNT: u64 = 1_000_000_000;
const USDC_BORROW_AMOUNT: u64 = 1_000_000;
const BORROW_RATE_MODE: u64 = 2;
const NETWORK_ID: &str = "reth-local";
const CONTROL_SCOPE: &str = "parity.aave_v3_reth_scenario";
const PARITY_RUN_MAX_ATTEMPTS: u32 = 3;
const PARITY_RUN_RETRY_DELAY_MS: u64 = 250;

#[derive(Default)]
struct MapContext {
    inner: HashMap<String, serde_json::Value>,
}

impl DynContext for MapContext {
    fn read(&self, key: &ContextKey) -> Result<Option<serde_json::Value>, ContextError> {
        Ok(self.inner.get(&key.0).cloned())
    }

    fn write(&mut self, key: ContextKey, value: serde_json::Value) -> Result<(), ContextError> {
        self.inner.insert(key.0, value);
        Ok(())
    }

    fn delete(&mut self, key: &ContextKey) -> Result<(), ContextError> {
        self.inner.remove(&key.0);
        Ok(())
    }

    fn dump(&self) -> Result<serde_json::Value, ContextError> {
        let mut out = serde_json::Map::new();
        for (k, v) in &self.inner {
            out.insert(k.clone(), v.clone());
        }
        Ok(serde_json::Value::Object(out))
    }
}

#[derive(Clone, Debug, Deserialize)]
struct AaveDeployManifest {
    kind: String,
    contracts: Vec<AaveDeployManifestContract>,
}

#[derive(Clone, Debug, Deserialize)]
struct AaveDeployManifestContract {
    id: String,
    address: String,
    artifact: serde_json::Value,
}

#[derive(Clone, Debug)]
struct ScenarioActors {
    funder: String,
    supplier: String,
    borrower: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct AaveScenarioAccounts {
    funder: String,
    supplier: String,
    borrower: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct AaveScenarioAmounts {
    usdc_supply: u64,
    wbtc_collateral: u64,
    usdc_borrow: u64,
    borrow_rate_mode: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct AaveScenarioPositionSnapshot {
    supplier_supplied_usdc: u64,
    borrower_collateral_wbtc: u64,
    borrower_borrowed_usdc: u64,
    borrower_usdc_balance: u64,
    pool_usdc_balance: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct AaveScenarioAssertions {
    strict: bool,
    passed: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct AaveScenarioReport {
    kind: String,
    chain_id: u64,
    accounts: AaveScenarioAccounts,
    amounts: AaveScenarioAmounts,
    positions: AaveScenarioPositionSnapshot,
    assertions: AaveScenarioAssertions,
    deploy_manifest_kind: String,
}

fn run_config_with_allowlist(allowlist: Vec<String>) -> RunConfig {
    RunConfig {
        io_mode: IoMode::Live,
        retry_policy: RetryPolicy {
            max_attempts: PARITY_RUN_MAX_ATTEMPTS,
            backoff: BackoffPolicy::Fixed {
                delay: Duration::from_millis(PARITY_RUN_RETRY_DELAY_MS),
            },
        },
        event_profile: EventProfile::Normal,
        execution_mode: ExecutionMode::Sequential,
        context_checkpointing: ContextCheckpointing::AfterEveryState,
        replay_missing_fact_retryable: false,
        skip_tags: Vec::new(),
        nix_flake_allowlist: allowlist,
    }
}

fn parse_u64_hex(s: &str) -> u64 {
    let trimmed = s
        .strip_prefix("0x")
        .expect("hex strings must start with 0x");
    u64::from_str_radix(trimmed, 16).expect("hex string must parse as u64")
}

fn snapshot_key_candidates(key: &str) -> Vec<String> {
    let mut candidates = vec![key.to_string()];
    if !key.contains(".in.") && !key.contains(".out.") && !key.contains(".work.") {
        if let Some((prefix, leaf)) = key.rsplit_once('.') {
            candidates.push(format!("{prefix}.out.{leaf}"));
            candidates.push(format!("{prefix}.work.{leaf}"));
        }
    }
    candidates
}

fn nested_snapshot_value<'a>(
    snapshot: &'a serde_json::Value,
    key: &str,
) -> Option<&'a serde_json::Value> {
    let mut current = snapshot;
    for segment in key.split('.') {
        current = current.get(segment)?;
    }
    Some(current)
}

fn snapshot_kind(snapshot: &serde_json::Value, key: &str) -> Option<String> {
    snapshot_key_candidates(key)
        .into_iter()
        .find_map(|candidate| {
            snapshot
                .get(format!("{candidate}.kind"))
                .and_then(|v| v.as_str())
                .or_else(|| {
                    nested_snapshot_value(snapshot, &format!("{candidate}.kind"))
                        .and_then(|v| v.as_str())
                })
                .or_else(|| {
                    snapshot
                        .get(&candidate)
                        .and_then(|v| v.get("kind"))
                        .and_then(|v| v.as_str())
                })
                .or_else(|| {
                    nested_snapshot_value(snapshot, &candidate)
                        .and_then(|v| v.get("kind"))
                        .and_then(|v| v.as_str())
                })
                .map(ToString::to_string)
        })
}

fn snapshot_value<'a>(snapshot: &'a serde_json::Value, key: &str) -> Option<&'a serde_json::Value> {
    snapshot_key_candidates(key)
        .into_iter()
        .find_map(|candidate| {
            snapshot
                .get(&candidate)
                .or_else(|| nested_snapshot_value(snapshot, &candidate))
        })
}

fn json_post(uri: &str, body: serde_json::Value) -> Request<Body> {
    let s = serde_json::to_string(&body).expect("json request must serialize");
    Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(s))
        .expect("request")
}

async fn response_json(resp: axum::response::Response) -> serde_json::Value {
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("body bytes");
    serde_json::from_slice(&bytes).expect("json response")
}

fn aave_portfolio_snapshot_payload(
    chain_id: u64,
    deploy_manifest: &AaveDeployManifest,
    actors: &ScenarioActors,
    control_scope: &str,
) -> serde_json::Value {
    let pool = contract_from_manifest(deploy_manifest, CONTRACT_POOL);
    let usdc = contract_from_manifest(deploy_manifest, CONTRACT_USDC);
    let wbtc = contract_from_manifest(deploy_manifest, CONTRACT_WBTC);
    let usdc_a_token = contract_from_manifest(deploy_manifest, CONTRACT_USDC_A_TOKEN);
    let wbtc_a_token = contract_from_manifest(deploy_manifest, CONTRACT_WBTC_A_TOKEN);
    let usdc_variable_debt =
        contract_from_manifest(deploy_manifest, CONTRACT_USDC_VARIABLE_DEBT_TOKEN);
    let supplier_address = normalize_address_lower(&actors.supplier);
    let borrower_address = normalize_address_lower(&actors.borrower);
    let pool_address = normalize_address_lower(&pool.address);
    let usdc_address = normalize_address_lower(&usdc.address);
    let wbtc_address = normalize_address_lower(&wbtc.address);
    let usdc_a_token_address = normalize_address_lower(&usdc_a_token.address);
    let wbtc_a_token_address = normalize_address_lower(&wbtc_a_token.address);
    let usdc_variable_debt_address = normalize_address_lower(&usdc_variable_debt.address);

    serde_json::json!({
        "portfolio": {
            "portfolio_id": "aave-v3-reth-portfolio",
            "quote_codes": ["USD"],
            "networks": [
                {
                    "network_id": "reth-local",
                    "chain_id": chain_id,
                    "control_scope": control_scope,
                    "metadata": {}
                }
            ],
            "wallets": [
                {
                    "wallet_id": "wallet_supplier",
                    "address": supplier_address,
                    "network_id": "reth-local",
                    "implementation": { "kind": "address_only" },
                    "symbol_ids": ["aave_v3.usdc.asset.reth-local"],
                    "metadata": {}
                },
                {
                    "wallet_id": "wallet_borrower",
                    "address": borrower_address,
                    "network_id": "reth-local",
                    "implementation": { "kind": "address_only" },
                    "symbol_ids": [
                        "aave_v3.wbtc.collateral.reth-local",
                        "aave_v3.usdc.debt.reth-local"
                    ],
                    "metadata": {}
                }
            ],
            "symbol_configs": [
                {
                    "symbol_id": "usdc.wallet.reth-local",
                    "display_symbol": "USDC",
                    "kind": "erc20_balance",
                    "role": "asset",
                    "network_id": "reth-local",
                    "protocol": null,
                    "balance_reader": {
                        "kind": "erc20_balance",
                        "token_address": usdc_address
                    },
                    "valuation": {
                        "quotes": [{
                            "quote": "USD",
                            "priced_symbol_id": "usdc.wallet.reth-local",
                            "reader": {
                                "kind": "fixed_unit_price",
                                "unit_price_dec": "1.00"
                            }
                        }]
                    },
                    "decimals": 6,
                    "underlying_symbol_id": null,
                    "metadata": {}
                },
                {
                    "symbol_id": "wbtc.wallet.reth-local",
                    "display_symbol": "WBTC",
                    "kind": "erc20_balance",
                    "role": "asset",
                    "network_id": "reth-local",
                    "protocol": null,
                    "balance_reader": {
                        "kind": "erc20_balance",
                        "token_address": wbtc_address
                    },
                    "valuation": {
                        "quotes": [{
                            "quote": "USD",
                            "priced_symbol_id": "wbtc.wallet.reth-local",
                            "reader": {
                                "kind": "fixed_unit_price",
                                "unit_price_dec": "70000.00"
                            }
                        }]
                    },
                    "decimals": 8,
                    "underlying_symbol_id": null,
                    "metadata": {}
                },
                {
                    "symbol_id": "aave_v3.usdc.asset.reth-local",
                    "display_symbol": "USDC",
                    "kind": "protocol_position",
                    "role": "asset",
                    "network_id": "reth-local",
                    "protocol": "aave_v3",
                    "balance_reader": {
                        "kind": "protocol_position",
                        "protocol": "aave_v3",
                        "reader": "reserve_position",
                        "config": {
                            "market": {
                                "market_id": "aave-v3-reth",
                                "network_id": "reth-local",
                                "chain_id": chain_id,
                                "pool_address": pool_address,
                                "reserves": [
                                    {
                                        "reserve_id": "usdc",
                                        "reserve_index": 0,
                                        "underlying_token_address": usdc_address,
                                        "a_token_address": usdc_a_token_address,
                                        "variable_debt_token_address": usdc_variable_debt_address,
                                        "stable_debt_token_address": null,
                                        "metadata": {}
                                    },
                                    {
                                        "reserve_id": "wbtc",
                                        "reserve_index": 1,
                                        "underlying_token_address": wbtc_address,
                                        "a_token_address": wbtc_a_token_address,
                                        "variable_debt_token_address": null,
                                        "stable_debt_token_address": null,
                                        "metadata": {}
                                    }
                                ],
                                "metadata": {}
                            },
                            "reserve_id": "usdc"
                        }
                    },
                    "valuation": {
                        "quotes": [{
                            "quote": "USD",
                            "priced_symbol_id": "usdc.wallet.reth-local",
                            "reader": {
                                "kind": "fixed_unit_price",
                                "unit_price_dec": "1.00"
                            }
                        }]
                    },
                    "decimals": null,
                    "underlying_symbol_id": "usdc.wallet.reth-local",
                    "metadata": {}
                },
                {
                    "symbol_id": "aave_v3.wbtc.collateral.reth-local",
                    "display_symbol": "WBTC",
                    "kind": "protocol_position",
                    "role": "collateral",
                    "network_id": "reth-local",
                    "protocol": "aave_v3",
                    "balance_reader": {
                        "kind": "protocol_position",
                        "protocol": "aave_v3",
                        "reader": "reserve_position",
                        "config": {
                            "market": {
                                "market_id": "aave-v3-reth",
                                "network_id": "reth-local",
                                "chain_id": chain_id,
                                "pool_address": pool_address,
                                "reserves": [
                                    {
                                        "reserve_id": "usdc",
                                        "reserve_index": 0,
                                        "underlying_token_address": usdc_address,
                                        "a_token_address": usdc_a_token_address,
                                        "variable_debt_token_address": usdc_variable_debt_address,
                                        "stable_debt_token_address": null,
                                        "metadata": {}
                                    },
                                    {
                                        "reserve_id": "wbtc",
                                        "reserve_index": 1,
                                        "underlying_token_address": wbtc_address,
                                        "a_token_address": wbtc_a_token_address,
                                        "variable_debt_token_address": null,
                                        "stable_debt_token_address": null,
                                        "metadata": {}
                                    }
                                ],
                                "metadata": {}
                            },
                            "reserve_id": "wbtc"
                        }
                    },
                    "valuation": {
                        "quotes": [{
                            "quote": "USD",
                            "priced_symbol_id": "wbtc.wallet.reth-local",
                            "reader": {
                                "kind": "fixed_unit_price",
                                "unit_price_dec": "70000.00"
                            }
                        }]
                    },
                    "decimals": null,
                    "underlying_symbol_id": "wbtc.wallet.reth-local",
                    "metadata": {}
                },
                {
                    "symbol_id": "aave_v3.usdc.debt.reth-local",
                    "display_symbol": "USDC",
                    "kind": "protocol_position",
                    "role": "debt",
                    "network_id": "reth-local",
                    "protocol": "aave_v3",
                    "balance_reader": {
                        "kind": "protocol_position",
                        "protocol": "aave_v3",
                        "reader": "debt_position",
                        "config": {
                            "market": {
                                "market_id": "aave-v3-reth",
                                "network_id": "reth-local",
                                "chain_id": chain_id,
                                "pool_address": pool_address,
                                "reserves": [
                                    {
                                        "reserve_id": "usdc",
                                        "reserve_index": 0,
                                        "underlying_token_address": usdc_address,
                                        "a_token_address": usdc_a_token_address,
                                        "variable_debt_token_address": usdc_variable_debt_address,
                                        "stable_debt_token_address": null,
                                        "metadata": {}
                                    },
                                    {
                                        "reserve_id": "wbtc",
                                        "reserve_index": 1,
                                        "underlying_token_address": wbtc_address,
                                        "a_token_address": wbtc_a_token_address,
                                        "variable_debt_token_address": null,
                                        "stable_debt_token_address": null,
                                        "metadata": {}
                                    }
                                ],
                                "metadata": {}
                            },
                            "reserve_id": "usdc",
                            "debt_kind": "variable"
                        }
                    },
                    "valuation": {
                        "quotes": [{
                            "quote": "USD",
                            "priced_symbol_id": "usdc.wallet.reth-local",
                            "reader": {
                                "kind": "fixed_unit_price",
                                "unit_price_dec": "1.00"
                            }
                        }]
                    },
                    "decimals": null,
                    "underlying_symbol_id": "usdc.wallet.reth-local",
                    "metadata": {}
                }
            ],
            "metadata": {}
        },
        "valuation_source_registry": {
            "sources": []
        }
    })
}

fn normalize_address_lower(s: &str) -> String {
    let s = s.trim();
    let rest = s
        .strip_prefix("0x")
        .or_else(|| s.strip_prefix("0X"))
        .expect("0x");
    assert_eq!(rest.len(), 40, "address must be 20 bytes hex");
    assert!(
        rest.chars().all(|c| c.is_ascii_hexdigit()),
        "address must be hex"
    );
    format!("0x{}", rest.to_ascii_lowercase())
}

fn find_wallet<'a>(snapshot: &'a serde_json::Value, wallet_id: &str) -> &'a serde_json::Value {
    snapshot["wallets"]
        .as_array()
        .expect("wallets array")
        .iter()
        .find(|wallet| wallet["wallet_id"] == wallet_id)
        .unwrap_or_else(|| panic!("wallet not found: {wallet_id}"))
}

fn find_wallet_summary<'a>(
    report: &'a serde_json::Value,
    wallet_id: &str,
) -> &'a serde_json::Value {
    report["wallet_summaries"]
        .as_array()
        .expect("wallet_summaries array")
        .iter()
        .find(|wallet| wallet["wallet_id"] == wallet_id)
        .unwrap_or_else(|| panic!("wallet summary not found: {wallet_id}"))
}

fn find_observation<'a>(wallet: &'a serde_json::Value, symbol_id: &str) -> &'a serde_json::Value {
    wallet["observations"]
        .as_array()
        .expect("observations array")
        .iter()
        .find(|observation| observation["symbol_id"] == symbol_id)
        .unwrap_or_else(|| panic!("observation not found: {symbol_id}"))
}

fn find_quote_total<'a>(totals: &'a serde_json::Value, quote: &str) -> &'a serde_json::Value {
    totals
        .as_array()
        .expect("quote totals array")
        .iter()
        .find(|total| total["quote"] == quote)
        .unwrap_or_else(|| panic!("quote total not found: {quote}"))
}

fn required_program_path(program: &str) -> String {
    let out = std::process::Command::new("sh")
        .arg("-c")
        .arg(format!("command -v {program}"))
        .output()
        .unwrap_or_else(|_| panic!("resolve {program} path"));
    assert!(out.status.success(), "{program} must be in PATH");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn contract_from_manifest<'a>(
    manifest: &'a AaveDeployManifest,
    id: &str,
) -> &'a AaveDeployManifestContract {
    manifest
        .contracts
        .iter()
        .find(|c| c.id == id)
        .unwrap_or_else(|| panic!("missing contract in deploy manifest: {id}"))
}

fn balance_of_calldata(owner: &str) -> String {
    let normalized = owner
        .strip_prefix("0x")
        .unwrap_or_else(|| panic!("address must start with 0x: {owner}"));
    assert_eq!(normalized.len(), 40, "address must be 20 bytes: {owner}");
    format!("0x70a08231{:0>64}", normalized.to_ascii_lowercase())
}

async fn rpc_call(
    rpc_sources: &[RpcControlBootstrapSource],
    control_scope: &str,
    streams: Arc<dyn StreamStore>,
    artifacts: Arc<dyn ArtifactStore>,
    method: &str,
    params: serde_json::Value,
) -> serde_json::Value {
    rpc_control::call_in_scope(
        rpc_sources,
        NETWORK_ID,
        control_scope,
        streams,
        artifacts,
        method,
        params,
    )
    .await
}

async fn erc20_balance_u64(
    rpc_sources: &[RpcControlBootstrapSource],
    control_scope: &str,
    streams: Arc<dyn StreamStore>,
    artifacts: Arc<dyn ArtifactStore>,
    token: &str,
    owner: &str,
) -> u64 {
    let value = rpc_call(
        rpc_sources,
        control_scope,
        streams,
        artifacts,
        "eth_call",
        serde_json::json!([
            {
                "to": token,
                "data": balance_of_calldata(owner),
            },
            "latest"
        ]),
    )
    .await;

    let raw = value.as_str().expect("eth_call must return hex string");
    parse_u64_hex(raw)
}

async fn connect_postgres_with_retry(max_attempts: u32, delay_ms: u64) -> PostgresStreamStore {
    let mut last_err: Option<mfm_machine::errors::StorageError> = None;
    for _ in 0..max_attempts {
        match PostgresStreamStore::connect_env().await {
            Ok(pg) => return pg,
            Err(err) => {
                last_err = Some(err);
                tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            }
        }
    }

    let db_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| "<missing>".to_string());
    panic!(
        "postgres config after retries (DATABASE_URL={}): {:?}",
        db_url, last_err
    );
}

fn summarize_exec_error_details(details: Option<&serde_json::Value>) -> Option<String> {
    let obj = details?.as_object()?;
    let mut parts = Vec::new();

    if let Some(program_path) = obj.get("program_path").and_then(|v| v.as_str()) {
        parts.push(format!("program_path={program_path}"));
    }
    if let Some(timeout_ms) = obj.get("timeout_ms").and_then(|v| v.as_u64()) {
        parts.push(format!("timeout_ms={timeout_ms}"));
    }
    if let Some(exit_code) = obj.get("exit_code").and_then(|v| v.as_i64()) {
        parts.push(format!("exit_code={exit_code}"));
    }
    if let Some(signal) = obj.get("signal").and_then(|v| v.as_i64()) {
        parts.push(format!("signal={signal}"));
    }

    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" "))
    }
}

fn read_reth_probe_diagnostics() -> Option<String> {
    let path = std::env::var("MFM_PARITY_AAVE_V3_RETH_PROBE_PATH").ok()?;
    let contents = std::fs::read_to_string(path).ok()?;
    let last_line = contents
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())?;
    let probe: serde_json::Value = serde_json::from_str(last_line).ok()?;
    let mut parts = Vec::new();

    if let Some(ts) = probe.get("timestamp").and_then(|v| v.as_str()) {
        parts.push(format!("timestamp={ts}"));
    }
    if let Some(phase) = probe.get("phase").and_then(|v| v.as_str()) {
        parts.push(format!("phase={phase}"));
    }
    if let Some(running) = probe.get("running").and_then(|v| v.as_str()) {
        parts.push(format!("running={running}"));
    }
    if let Some(health_ok) = probe.get("health_ok").and_then(|v| v.as_bool()) {
        parts.push(format!("health_ok={health_ok}"));
    }
    if let Some(pid) = probe.get("pid").and_then(|v| v.as_str()) {
        parts.push(format!("pid={pid}"));
    }
    if let Some(wait_reason) = probe.get("wait_reason").and_then(|v| v.as_str()) {
        parts.push(format!("wait_reason={wait_reason}"));
    }

    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" "))
    }
}

async fn run_failure_diagnostics(streams: Arc<dyn StreamStore>, run_id: RunId) -> String {
    let stream = match streams
        .read_range(&StreamId::run(run_id), 1, None)
        .await
        .and_then(|records| event_envelopes_from_stream_records(run_id, records))
    {
        Ok(stream) => stream,
        Err(err) => {
            return format!("run_id={} read_range_failed={err:?}", run_id.0);
        }
    };

    let mut last_state_entered: Option<(u64, String, u32)> = None;
    let mut last_state_failed: Option<(u64, String, String, bool, String, Option<String>)> = None;
    let mut last_fact_key: Option<(u64, String)> = None;

    for envelope in stream {
        match envelope.event {
            Event::Kernel(KernelEvent::StateEntered {
                state_id, attempt, ..
            }) => {
                last_state_entered = Some((envelope.seq, state_id.to_string(), attempt));
            }
            Event::Kernel(KernelEvent::StateFailed {
                state_id, error, ..
            }) => {
                let detail_summary = summarize_exec_error_details(error.info.details.as_ref());
                last_state_failed = Some((
                    envelope.seq,
                    state_id.to_string(),
                    error.info.code.0,
                    error.info.retryable,
                    error.info.message,
                    detail_summary,
                ));
            }
            Event::Domain(domain) if domain.name == "fact_recorded" => {
                if let Some(key) = domain.payload.get("key").and_then(|v| v.as_str()) {
                    last_fact_key = Some((envelope.seq, key.to_string()));
                }
            }
            _ => {}
        }
    }

    let mut parts = vec![format!("run_id={}", run_id.0)];
    if let Some((seq, state_id, attempt)) = last_state_entered {
        parts.push(format!(
            "last_state_entered={state_id} attempt={attempt} seq={seq}"
        ));
    }
    if let Some((seq, state_id, code, retryable, message, detail_summary)) = last_state_failed {
        parts.push(format!(
            "state_failed={state_id} seq={seq} code={code} retryable={retryable} message={message}"
        ));
        if let Some(details) = detail_summary {
            parts.push(format!("state_failed_details={details}"));
        }
    }
    if let Some((seq, fact_key)) = last_fact_key {
        parts.push(format!("last_fact_key={fact_key} seq={seq}"));
    }
    if let Some(reth_probe) = read_reth_probe_diagnostics() {
        parts.push(format!("reth_probe={reth_probe}"));
    }
    if parts.len() == 1 {
        parts.push("no_state_failed_event_found".to_string());
    }

    parts.join("; ")
}

fn raw_phase_a_pipeline() -> Pipeline {
    let fetch_origin_program = required_program_path("mfm-aave-v3-origin-fetch");
    let compile_origin_program = required_program_path("mfm-aave-v3-origin-compile");
    let deploy_origin_program = required_program_path("mfm-aave-v3-origin-deploy");

    Pipeline {
        machine_id: MachineId("aave_v3_reth_pipeline".to_string()),
        pipeline_version: "v1".to_string(),
        steps: vec![
            PipelineStep {
                step_id: StepId("fetch_origin".to_string()),
                op_id: OpId::must_new("nix_app".to_string()),
                op_version: "v1".to_string(),
                op_config: serde_json::json!({
                    "program_path": fetch_origin_program,
                    "stdin_json": {},
                    "timeout_ms": 300000,
                    "write_result_to": "fetch_origin_result",
                }),
            },
            PipelineStep {
                step_id: StepId("compile_origin".to_string()),
                op_id: OpId::must_new("nix_app".to_string()),
                op_version: "v1".to_string(),
                op_config: serde_json::json!({
                    "program_path": compile_origin_program,
                    "stdin_json": {},
                    "timeout_ms": 600000,
                    "write_result_to": "compile_origin_result",
                }),
            },
            PipelineStep {
                step_id: StepId("deploy_origin_stack".to_string()),
                op_id: OpId::must_new("nix_app".to_string()),
                op_version: "v1".to_string(),
                op_config: serde_json::json!({
                    "program_path": deploy_origin_program,
                    "stdin_json": {},
                    "timeout_ms": 600000,
                    "write_result_to": "deploy_origin_result",
                }),
            },
            PipelineStep {
                step_id: StepId("adapt_origin_deploy".to_string()),
                op_id: OpId::must_new("aave_v3_origin_adapt_deploy".to_string()),
                op_version: "v1".to_string(),
                op_config: serde_json::json!({
                    "origin_deploy_port": "deploy_origin_result",
                    "deploy_manifest_export_key": "deploy_manifest",
                }),
            },
        ],
    }
}

fn phase_a_canonical_config(
    actors: &ScenarioActors,
    control_scope: &str,
) -> AaveV3OriginStackCanonicalConfig {
    AaveV3OriginStackCanonicalConfig {
        source: AaveV3OriginSourceConfig {
            repo_url: AAVE_V3_ORIGIN_BACKEND_REPO_URL.to_string(),
            commit_sha: AAVE_V3_ORIGIN_BACKEND_COMMIT_SHA.to_string(),
        },
        network_id: NETWORK_ID.to_string(),
        control_scope: control_scope.to_string(),
        deploy_signing_key_env: "MFM_AAVE_V3_PARITY_DEPLOY_SIGNING_KEY".to_string(),
        rpc_url_env: "MFM_EVM_RPC_URL".to_string(),
        supplier: actors.supplier.clone(),
        borrower: actors.borrower.clone(),
        usdc_supply_amount: USDC_SUPPLY_AMOUNT,
        wbtc_collateral_amount: WBTC_COLLATERAL_AMOUNT,
        fetch_timeout_ms: 300_000,
        compile_timeout_ms: 600_000,
        deploy_timeout_ms: 600_000,
    }
}

fn phase_a_pipeline(actors: &ScenarioActors, control_scope: &str) -> Pipeline {
    Pipeline {
        machine_id: MachineId("aave_v3_reth_pipeline".to_string()),
        pipeline_version: "v1".to_string(),
        steps: vec![PipelineStep {
            step_id: StepId("deploy_origin_stack".to_string()),
            op_id: OpId::must_new("aave_v3_origin_stack".to_string()),
            op_version: "v1".to_string(),
            op_config: serde_json::to_value(phase_a_canonical_config(actors, control_scope))
                .expect("serialize canonical phase A config"),
        }],
    }
}

fn phase_b_pipeline(
    deploy_manifest: &AaveDeployManifest,
    actors: &ScenarioActors,
    expected_chain_id: u64,
    control_scope: &str,
) -> Pipeline {
    let pool = contract_from_manifest(deploy_manifest, CONTRACT_POOL);
    let usdc = contract_from_manifest(deploy_manifest, CONTRACT_USDC);
    let wbtc = contract_from_manifest(deploy_manifest, CONTRACT_WBTC);
    let supplier = actors.supplier.clone();
    let borrower = actors.borrower.clone();
    let pool_artifact = pool.artifact.clone();
    let pool_address = pool.address.clone();
    let usdc_artifact = usdc.artifact.clone();
    let usdc_address = usdc.address.clone();
    let wbtc_artifact = wbtc.artifact.clone();
    let wbtc_address = wbtc.address.clone();

    Pipeline {
        machine_id: MachineId("aave_v3_reth_scenario_generic_pipeline".to_string()),
        pipeline_version: "v1".to_string(),
        steps: vec![
            PipelineStep {
                step_id: StepId("approve_usdc".to_string()),
                op_id: OpId::must_new("evm_configure".to_string()),
                op_version: "v1".to_string(),
                op_config: serde_json::json!({
                    "artifact": usdc_artifact,
                    "network_id": NETWORK_ID,
                    "control_scope": control_scope,
                    "from": supplier.clone(),
                    "signing_key_env": "MFM_AAVE_V3_PARITY_SUPPLIER_SIGNING_KEY",
                    "contract_address": usdc_address.clone(),
                    "calls": [{
                        "function": "approve",
                        "args": [pool_address.clone(), USDC_SUPPLY_AMOUNT],
                    }],
                    "tx_hashes_export_key": "approve_usdc_tx_hashes",
                    "receipts_export_key": "approve_usdc_receipts",
                    "poll_interval_ms": 200,
                    "max_receipt_polls": 120,
                }),
            },
            PipelineStep {
                step_id: StepId("approve_wbtc".to_string()),
                op_id: OpId::must_new("evm_configure".to_string()),
                op_version: "v1".to_string(),
                op_config: serde_json::json!({
                    "artifact": wbtc_artifact,
                    "network_id": NETWORK_ID,
                    "control_scope": control_scope,
                    "from": borrower.clone(),
                    "signing_key_env": "MFM_AAVE_V3_PARITY_BORROWER_SIGNING_KEY",
                    "contract_address": wbtc_address.clone(),
                    "calls": [{
                        "function": "approve",
                        "args": [pool_address.clone(), WBTC_COLLATERAL_AMOUNT],
                    }],
                    "tx_hashes_export_key": "approve_wbtc_tx_hashes",
                    "receipts_export_key": "approve_wbtc_receipts",
                    "poll_interval_ms": 200,
                    "max_receipt_polls": 120,
                }),
            },
            PipelineStep {
                step_id: StepId("supply_usdc".to_string()),
                op_id: OpId::must_new("evm_configure".to_string()),
                op_version: "v1".to_string(),
                op_config: serde_json::json!({
                    "artifact": pool_artifact.clone(),
                    "network_id": NETWORK_ID,
                    "control_scope": control_scope,
                    "from": supplier.clone(),
                    "signing_key_env": "MFM_AAVE_V3_PARITY_SUPPLIER_SIGNING_KEY",
                    "contract_address": pool_address.clone(),
                    "calls": [{
                        "function": "supply",
                        "args": [
                            usdc_address.clone(),
                            USDC_SUPPLY_AMOUNT,
                            supplier.clone(),
                            0,
                        ],
                    }],
                    "tx_hashes_export_key": "supply_usdc_tx_hashes",
                    "receipts_export_key": "supply_usdc_receipts",
                    "poll_interval_ms": 200,
                    "max_receipt_polls": 120,
                }),
            },
            PipelineStep {
                step_id: StepId("supply_wbtc".to_string()),
                op_id: OpId::must_new("evm_configure".to_string()),
                op_version: "v1".to_string(),
                op_config: serde_json::json!({
                    "artifact": pool_artifact.clone(),
                    "network_id": NETWORK_ID,
                    "control_scope": control_scope,
                    "from": borrower.clone(),
                    "signing_key_env": "MFM_AAVE_V3_PARITY_BORROWER_SIGNING_KEY",
                    "contract_address": pool_address.clone(),
                    "calls": [{
                        "function": "supply",
                        "args": [
                            wbtc_address.clone(),
                            WBTC_COLLATERAL_AMOUNT,
                            borrower.clone(),
                            0,
                        ],
                    }],
                    "tx_hashes_export_key": "supply_wbtc_tx_hashes",
                    "receipts_export_key": "supply_wbtc_receipts",
                    "poll_interval_ms": 200,
                    "max_receipt_polls": 120,
                }),
            },
            PipelineStep {
                step_id: StepId("borrow_usdc".to_string()),
                op_id: OpId::must_new("evm_configure".to_string()),
                op_version: "v1".to_string(),
                op_config: serde_json::json!({
                    "artifact": pool_artifact.clone(),
                    "network_id": NETWORK_ID,
                    "control_scope": control_scope,
                    "from": borrower.clone(),
                    "signing_key_env": "MFM_AAVE_V3_PARITY_BORROWER_SIGNING_KEY",
                    "contract_address": pool_address.clone(),
                    "calls": [{
                        "function": "borrow",
                        "args": [
                            usdc_address.clone(),
                            USDC_BORROW_AMOUNT,
                            BORROW_RATE_MODE,
                            0,
                            borrower.clone(),
                        ],
                    }],
                    "tx_hashes_export_key": "borrow_usdc_tx_hashes",
                    "receipts_export_key": "borrow_usdc_receipts",
                    "poll_interval_ms": 200,
                    "max_receipt_polls": 120,
                }),
            },
            PipelineStep {
                step_id: StepId("validate_scenario".to_string()),
                op_id: OpId::must_new("evm_validate".to_string()),
                op_version: "v1".to_string(),
                op_config: serde_json::json!({
                    "artifact": pool_artifact,
                    "contract_address": pool_address,
                    "network_id": NETWORK_ID,
                    "control_scope": control_scope,
                    "expected_chain_id": expected_chain_id,
                    "require_client_substring": "reth",
                    "read_assertions": [],
                    "event_assertions": [],
                }),
            },
        ],
    }
}

#[tokio::test]
async fn parity_aave_v3_reth_scenario_pipeline() {
    init_test_observability();

    let pg = connect_postgres_with_retry(20, 250).await;
    let streams: Arc<dyn StreamStore> = Arc::new(pg);

    let s3 = S3ArtifactStore::from_env().expect("s3 config");
    s3.ensure_bucket_exists().await.expect("bucket exists");
    let artifacts: Arc<dyn ArtifactStore> = Arc::new(s3);

    let rpc_sources = rpc_control::required_bootstrap_sources_from_env_for_network(NETWORK_ID);
    let control_scope = format!("{CONTROL_SCOPE}.{}", uuid::Uuid::new_v4().simple());
    let bootstrap_control_scope = format!("{control_scope}.bootstrap");
    let chain_id_hex = rpc_call(
        &rpc_sources,
        &bootstrap_control_scope,
        Arc::clone(&streams),
        Arc::clone(&artifacts),
        "eth_chainId",
        serde_json::json!([]),
    )
    .await;
    let expected_chain_id = chain_id_hex
        .as_str()
        .map(parse_u64_hex)
        .expect("eth_chainId hex");
    let accounts_json = rpc_call(
        &rpc_sources,
        &bootstrap_control_scope,
        Arc::clone(&streams),
        Arc::clone(&artifacts),
        "eth_accounts",
        serde_json::json!([]),
    )
    .await;
    let accounts = accounts_json
        .as_array()
        .expect("eth_accounts returned array");
    let actors = ScenarioActors {
        funder: accounts
            .first()
            .and_then(|v| v.as_str())
            .expect("funder account")
            .to_string(),
        supplier: accounts
            .get(1)
            .and_then(|v| v.as_str())
            .expect("supplier account")
            .to_string(),
        borrower: accounts
            .get(2)
            .and_then(|v| v.as_str())
            .expect("borrower account")
            .to_string(),
    };

    std::env::set_var(
        "MFM_AAVE_V3_PARITY_DEPLOY_SIGNING_KEY",
        RETH_DEV_ACCOUNT0_PRIVATE_KEY,
    );
    std::env::set_var(
        "MFM_AAVE_V3_PARITY_SUPPLIER_SIGNING_KEY",
        RETH_DEV_ACCOUNT1_PRIVATE_KEY,
    );
    std::env::set_var(
        "MFM_AAVE_V3_PARITY_BORROWER_SIGNING_KEY",
        RETH_DEV_ACCOUNT2_PRIVATE_KEY,
    );
    std::env::set_var("MFM_AAVE_V3_ORIGIN_SUPPLIER", &actors.supplier);
    std::env::set_var("MFM_AAVE_V3_ORIGIN_BORROWER", &actors.borrower);
    std::env::set_var(
        "MFM_AAVE_V3_ORIGIN_USDC_SUPPLY_AMOUNT",
        USDC_SUPPLY_AMOUNT.to_string(),
    );
    std::env::set_var(
        "MFM_AAVE_V3_ORIGIN_WBTC_COLLATERAL_AMOUNT",
        WBTC_COLLATERAL_AMOUNT.to_string(),
    );

    let run_config_phase_a =
        run_config_with_allowlist(mfm_machine::config::default_nix_flake_allowlist());

    let bundle = mfm_rest_api::make_engine_bundle();
    let launcher = DefaultRunLauncher;

    let phase_a_run = launcher
        .start_pipeline(
            Arc::clone(&bundle.engine),
            Stores {
                streams: Arc::clone(&streams),
                artifacts: Arc::clone(&artifacts),
            },
            Arc::clone(&bundle.registry),
            Arc::clone(&bundle.planner),
            LaunchPipeline {
                pipeline: phase_a_pipeline(&actors, &control_scope),
                input: serde_json::json!({}),
                run_config: run_config_phase_a,
                build: BuildProvenance {
                    git_commit: None,
                    cargo_lock_hash: None,
                    flake_lock_hash: None,
                    rustc_version: None,
                    target_triple: None,
                    env_allowlist: Vec::new(),
                },
                initial_context: Box::new(MapContext::default()),
            },
        )
        .await
        .expect("start phase A pipeline");

    if phase_a_run.phase != RunPhase::Completed {
        let diagnostics = run_failure_diagnostics(Arc::clone(&streams), phase_a_run.run_id).await;
        panic!(
            "phase A expected Completed, got {:?}; {}",
            phase_a_run.phase, diagnostics
        );
    }
    let phase_a_snapshot_id = phase_a_run
        .final_snapshot_id
        .expect("phase A final snapshot");
    let phase_a_snapshot_bytes = artifacts
        .get(&phase_a_snapshot_id)
        .await
        .expect("read phase A final snapshot");
    let phase_a_snapshot: serde_json::Value =
        serde_json::from_slice(&phase_a_snapshot_bytes).expect("decode phase A snapshot json");

    assert_eq!(
        snapshot_kind(
            &phase_a_snapshot,
            "aave_v3_reth_pipeline.deploy_origin_stack.execute.adapt_origin_deploy.deploy_manifest",
        )
        .as_deref(),
        Some(DEPLOY_MANIFEST_KIND)
    );
    assert_eq!(
        snapshot_kind(
            &phase_a_snapshot,
            "aave_v3_reth_pipeline.deploy_origin_stack.execute.fetch_origin.fetch_origin_result"
        )
        .as_deref(),
        Some("aave_v3_origin_source_v1")
    );
    assert_eq!(
        snapshot_kind(
            &phase_a_snapshot,
            "aave_v3_reth_pipeline.deploy_origin_stack.execute.compile_origin.compile_origin_result"
        )
        .as_deref(),
        Some("aave_v3_origin_compile_manifest_v1")
    );
    assert_eq!(
        snapshot_kind(
            &phase_a_snapshot,
            "aave_v3_reth_pipeline.deploy_origin_stack.execute.deploy_origin_stack.deploy_origin_result"
        )
        .as_deref(),
        Some("aave_v3_origin_deploy_output_v1")
    );

    let deploy_manifest = snapshot_value(
        &phase_a_snapshot,
        "aave_v3_reth_pipeline.deploy_origin_stack.execute.adapt_origin_deploy.deploy_manifest",
    )
    .cloned()
    .expect("deploy manifest in phase A snapshot");
    let deploy_manifest: AaveDeployManifest =
        serde_json::from_value(deploy_manifest).expect("decode deploy manifest");
    assert_eq!(deploy_manifest.kind, DEPLOY_MANIFEST_KIND);

    let phase_b_run = launcher
        .start_pipeline(
            Arc::clone(&bundle.engine),
            Stores {
                streams: Arc::clone(&streams),
                artifacts: Arc::clone(&artifacts),
            },
            Arc::clone(&bundle.registry),
            Arc::clone(&bundle.planner),
            LaunchPipeline {
                pipeline: phase_b_pipeline(
                    &deploy_manifest,
                    &actors,
                    expected_chain_id,
                    &control_scope,
                ),
                input: serde_json::json!({}),
                run_config: run_config_with_allowlist(
                    mfm_machine::config::default_nix_flake_allowlist(),
                ),
                build: BuildProvenance {
                    git_commit: None,
                    cargo_lock_hash: None,
                    flake_lock_hash: None,
                    rustc_version: None,
                    target_triple: None,
                    env_allowlist: Vec::new(),
                },
                initial_context: Box::new(MapContext::default()),
            },
        )
        .await
        .expect("start phase B pipeline");

    if phase_b_run.phase != RunPhase::Completed {
        let diagnostics = run_failure_diagnostics(Arc::clone(&streams), phase_b_run.run_id).await;
        panic!(
            "phase B expected Completed, got {:?}; {}",
            phase_b_run.phase, diagnostics
        );
    }

    let pool = contract_from_manifest(&deploy_manifest, CONTRACT_POOL);
    let usdc = contract_from_manifest(&deploy_manifest, CONTRACT_USDC);
    let usdc_a_token = contract_from_manifest(&deploy_manifest, CONTRACT_USDC_A_TOKEN);
    let wbtc_a_token = contract_from_manifest(&deploy_manifest, CONTRACT_WBTC_A_TOKEN);
    let usdc_variable_debt =
        contract_from_manifest(&deploy_manifest, CONTRACT_USDC_VARIABLE_DEBT_TOKEN);

    let positions = AaveScenarioPositionSnapshot {
        supplier_supplied_usdc: erc20_balance_u64(
            &rpc_sources,
            &control_scope,
            Arc::clone(&streams),
            Arc::clone(&artifacts),
            &usdc_a_token.address,
            &actors.supplier,
        )
        .await,
        borrower_collateral_wbtc: erc20_balance_u64(
            &rpc_sources,
            &control_scope,
            Arc::clone(&streams),
            Arc::clone(&artifacts),
            &wbtc_a_token.address,
            &actors.borrower,
        )
        .await,
        borrower_borrowed_usdc: erc20_balance_u64(
            &rpc_sources,
            &control_scope,
            Arc::clone(&streams),
            Arc::clone(&artifacts),
            &usdc_variable_debt.address,
            &actors.borrower,
        )
        .await,
        borrower_usdc_balance: erc20_balance_u64(
            &rpc_sources,
            &control_scope,
            Arc::clone(&streams),
            Arc::clone(&artifacts),
            &usdc.address,
            &actors.borrower,
        )
        .await,
        pool_usdc_balance: erc20_balance_u64(
            &rpc_sources,
            &control_scope,
            Arc::clone(&streams),
            Arc::clone(&artifacts),
            &usdc.address,
            &pool.address,
        )
        .await,
    };

    assert!(positions.supplier_supplied_usdc >= USDC_SUPPLY_AMOUNT);
    assert!(positions.borrower_collateral_wbtc >= WBTC_COLLATERAL_AMOUNT);
    assert!(positions.borrower_borrowed_usdc >= USDC_BORROW_AMOUNT);

    let report = AaveScenarioReport {
        kind: SCENARIO_REPORT_KIND.to_string(),
        chain_id: expected_chain_id,
        accounts: AaveScenarioAccounts {
            funder: actors.funder.clone(),
            supplier: actors.supplier.clone(),
            borrower: actors.borrower.clone(),
        },
        amounts: AaveScenarioAmounts {
            usdc_supply: USDC_SUPPLY_AMOUNT,
            wbtc_collateral: WBTC_COLLATERAL_AMOUNT,
            usdc_borrow: USDC_BORROW_AMOUNT,
            borrow_rate_mode: BORROW_RATE_MODE,
        },
        positions,
        assertions: AaveScenarioAssertions {
            strict: true,
            passed: true,
        },
        deploy_manifest_kind: deploy_manifest.kind.clone(),
    };

    assert_eq!(report.kind, SCENARIO_REPORT_KIND);
    assert_eq!(report.chain_id, expected_chain_id);
    assert_eq!(report.amounts.usdc_supply, USDC_SUPPLY_AMOUNT);
    assert_eq!(report.amounts.wbtc_collateral, WBTC_COLLATERAL_AMOUNT);
    assert_eq!(report.amounts.usdc_borrow, USDC_BORROW_AMOUNT);
    assert!(report.assertions.strict);
    assert!(report.assertions.passed);

    let report_artifact_id = artifacts
        .put(
            ArtifactKind::Output,
            serde_json::to_vec(&report).expect("serialize scenario report"),
        )
        .await
        .expect("write scenario report artifact");
    let report_artifact = artifacts
        .get(&report_artifact_id)
        .await
        .expect("get report artifact");
    let report_artifact_json: serde_json::Value =
        serde_json::from_slice(&report_artifact).expect("decode report artifact");
    assert_eq!(
        report_artifact_json.get("kind").and_then(|v| v.as_str()),
        Some(SCENARIO_REPORT_KIND)
    );

    let app = mfm_rest_api::make_app(mfm_rest_api::AppState {
        bundle: mfm_rest_api::make_engine_bundle(),
        streams: Arc::clone(&streams),
        artifacts: Arc::clone(&artifacts),
    });
    let resp = app
        .clone()
        .oneshot(json_post(
            "/v1/features/portfolio.snapshot/execute",
            serde_json::json!({
                "payload": aave_portfolio_snapshot_payload(
                    expected_chain_id,
                    &deploy_manifest,
                    &actors,
                    &control_scope
                )
            }),
        ))
        .await
        .expect("portfolio snapshot feature response");

    assert_eq!(resp.status(), StatusCode::OK);
    let feature = response_json(resp).await;
    assert_eq!(feature["status"], "success");
    assert_eq!(feature["data"]["feature_id"], "portfolio.snapshot");
    assert_eq!(feature["data"]["result"]["phase"], "completed");
    assert_eq!(
        feature["data"]["result"]["report"]["portfolio_id"],
        "aave-v3-reth-portfolio"
    );
    assert_eq!(feature["data"]["result"]["report"]["error_count"], 0);
    let report = &feature["data"]["result"]["report"];
    let supplier_report_usd = find_quote_total(
        &find_wallet_summary(report, "wallet_supplier")["totals_by_quote"],
        "USD",
    );
    assert_eq!(supplier_report_usd["collateral_value_dec"], "0");
    assert_eq!(supplier_report_usd["debt_value_dec"], "0");
    assert_eq!(supplier_report_usd["staked_value_dec"], "0");

    let snapshot_artifact_id = feature["data"]["result"]["snapshot_artifact_id"]
        .as_str()
        .expect("snapshot_artifact_id")
        .to_string();
    let artifact_resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/v1/artifacts/{snapshot_artifact_id}"))
                .body(Body::empty())
                .expect("artifact request"),
        )
        .await
        .expect("artifact response");
    assert_eq!(artifact_resp.status(), StatusCode::OK);

    let snapshot_response = response_json(artifact_resp).await;
    let snapshot = snapshot_response["data"]["value"].clone();
    assert_eq!(snapshot["portfolio_id"], "aave-v3-reth-portfolio");
    assert_eq!(
        snapshot["network_pins"][0]["anchor"]["chain_id"],
        expected_chain_id
    );

    let supplier_wallet = find_wallet(&snapshot, "wallet_supplier");
    let supplier_usdc = find_observation(supplier_wallet, "aave_v3.usdc.asset.reth-local");
    let supplier_usdc_raw: u64 = supplier_usdc["quantity"]["raw_dec"]
        .as_str()
        .expect("supplier raw_dec")
        .parse()
        .expect("supplier raw quantity");
    assert!(supplier_usdc_raw >= USDC_SUPPLY_AMOUNT);
    assert_eq!(
        supplier_usdc["source"]["balance_reader_kind"],
        "protocol_position:aave_v3:reserve_position"
    );
    assert_eq!(
        supplier_usdc["values"][0]["priced_symbol_id"],
        "usdc.wallet.reth-local"
    );
    assert_eq!(
        supplier_report_usd["assets_value_dec"],
        supplier_usdc["values"][0]["value_dec"]
    );
    assert_eq!(
        supplier_report_usd["net_value_dec"],
        supplier_usdc["values"][0]["value_dec"]
    );

    let borrower_wallet = find_wallet(&snapshot, "wallet_borrower");
    let borrower_collateral =
        find_observation(borrower_wallet, "aave_v3.wbtc.collateral.reth-local");
    let borrower_collateral_raw: u64 = borrower_collateral["quantity"]["raw_dec"]
        .as_str()
        .expect("collateral raw_dec")
        .parse()
        .expect("collateral raw quantity");
    assert!(borrower_collateral_raw >= WBTC_COLLATERAL_AMOUNT);
    assert_eq!(borrower_collateral["role"], "collateral");
    assert_eq!(
        borrower_collateral["values"][0]["priced_symbol_id"],
        "wbtc.wallet.reth-local"
    );

    let borrower_debt = find_observation(borrower_wallet, "aave_v3.usdc.debt.reth-local");
    let borrower_debt_raw: u64 = borrower_debt["quantity"]["raw_dec"]
        .as_str()
        .expect("debt raw_dec")
        .parse()
        .expect("debt raw quantity");
    assert!(borrower_debt_raw >= USDC_BORROW_AMOUNT);
    assert_eq!(borrower_debt["role"], "debt");
    assert_eq!(
        borrower_debt["source"]["balance_reader_kind"],
        "protocol_position:aave_v3:debt_position"
    );
    assert_eq!(borrower_debt["metadata"]["debt_kind"], "variable");
    assert_eq!(
        borrower_debt["values"][0]["priced_symbol_id"],
        "usdc.wallet.reth-local"
    );

    let borrower_report_usd = find_quote_total(
        &find_wallet_summary(report, "wallet_borrower")["totals_by_quote"],
        "USD",
    );
    assert_eq!(
        borrower_report_usd["collateral_value_dec"],
        borrower_collateral["values"][0]["value_dec"]
    );
    assert_eq!(
        borrower_report_usd["debt_value_dec"],
        borrower_debt["values"][0]["value_dec"]
    );
    assert_eq!(borrower_report_usd["assets_value_dec"], "0");
    assert_eq!(borrower_report_usd["staked_value_dec"], "0");
    assert!(borrower_report_usd["net_value_dec"]
        .as_str()
        .is_some_and(|value| !value.is_empty()));
    let portfolio_report_usd = find_quote_total(
        &feature["data"]["result"]["report"]["totals_by_quote"],
        "USD",
    );
    assert!(portfolio_report_usd["net_value_dec"]
        .as_str()
        .is_some_and(|value| !value.is_empty()));

    write_parity_aave_run_ids(&phase_a_run.run_id, &phase_b_run.run_id);
}

#[tokio::test]
async fn parity_aave_v3_reth_phase_a_raw_wrapper_pipeline() {
    init_test_observability();

    let pg = connect_postgres_with_retry(20, 250).await;
    let streams: Arc<dyn StreamStore> = Arc::new(pg);

    let s3 = S3ArtifactStore::from_env().expect("s3 config");
    s3.ensure_bucket_exists().await.expect("bucket exists");
    let artifacts: Arc<dyn ArtifactStore> = Arc::new(s3);

    std::env::set_var(
        "MFM_AAVE_V3_PARITY_DEPLOY_SIGNING_KEY",
        RETH_DEV_ACCOUNT0_PRIVATE_KEY,
    );

    let bundle = mfm_rest_api::make_engine_bundle();
    let launcher = DefaultRunLauncher;
    let run = launcher
        .start_pipeline(
            Arc::clone(&bundle.engine),
            Stores {
                streams: Arc::clone(&streams),
                artifacts: Arc::clone(&artifacts),
            },
            Arc::clone(&bundle.registry),
            Arc::clone(&bundle.planner),
            LaunchPipeline {
                pipeline: raw_phase_a_pipeline(),
                input: serde_json::json!({}),
                run_config: run_config_with_allowlist(
                    mfm_machine::config::default_nix_flake_allowlist(),
                ),
                build: BuildProvenance {
                    git_commit: None,
                    cargo_lock_hash: None,
                    flake_lock_hash: None,
                    rustc_version: None,
                    target_triple: None,
                    env_allowlist: Vec::new(),
                },
                initial_context: Box::new(MapContext::default()),
            },
        )
        .await
        .expect("start raw phase A pipeline");

    if run.phase != RunPhase::Completed {
        let diagnostics = run_failure_diagnostics(Arc::clone(&streams), run.run_id).await;
        panic!(
            "raw phase A expected Completed, got {:?}; {}",
            run.phase, diagnostics
        );
    }

    let snapshot_id = run.final_snapshot_id.expect("raw phase A final snapshot");
    let snapshot_bytes = artifacts
        .get(&snapshot_id)
        .await
        .expect("read raw phase A final snapshot");
    let snapshot: serde_json::Value =
        serde_json::from_slice(&snapshot_bytes).expect("decode raw phase A snapshot json");

    assert_eq!(
        snapshot_kind(
            &snapshot,
            "aave_v3_reth_pipeline.fetch_origin.fetch_origin_result"
        )
        .as_deref(),
        Some("aave_v3_origin_source_v1")
    );
    assert_eq!(
        snapshot_kind(
            &snapshot,
            "aave_v3_reth_pipeline.compile_origin.compile_origin_result"
        )
        .as_deref(),
        Some("aave_v3_origin_compile_manifest_v1")
    );
    assert_eq!(
        snapshot_kind(
            &snapshot,
            "aave_v3_reth_pipeline.deploy_origin_stack.deploy_origin_result"
        )
        .as_deref(),
        Some("aave_v3_origin_deploy_output_v1")
    );
    assert_eq!(
        snapshot_kind(
            &snapshot,
            "aave_v3_reth_pipeline.adapt_origin_deploy.deploy_manifest",
        )
        .as_deref(),
        Some(DEPLOY_MANIFEST_KIND)
    );
}
