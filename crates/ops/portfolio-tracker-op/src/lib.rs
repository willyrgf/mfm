//! Portfolio tracker operation.
//!
//! Source of truth: `docs/redesign.md` (v4).
//!
//! Current scope (v1):
//! - validate `eth_chainId` matches configured `chain_id` (default: 1)
//! - fetch pinned `eth_blockNumber`
//! - fetch native ETH balance via `eth_getBalance` at that pinned block
//! - fetch allowlisted ERC-20 balances via `eth_call(balanceOf)` at that pinned block
//! - write a content-addressed snapshot output artifact via deterministic fact recording

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use alloy_primitives::Address;
#[cfg(test)]
use mfm_evm_runtime::states::read::encode_erc20_decimals;
use mfm_evm_runtime::states::read::{
    address_hex_lower, address_hex_lower_no0x, NativeBalanceState, ReadU64HexState,
    TokenBalanceState, U64Expectation,
};
use mfm_machine::config::RunConfig;
use mfm_machine::context::DynContext;
use mfm_machine::errors::ErrorCategory;
use mfm_machine::errors::StateError;
use mfm_machine::events::DomainEvent;
use mfm_machine::ids::{ContextKey, FactKey, OpId, OpPath, StateId};
use mfm_machine::io::IoProvider;
use mfm_machine::meta::StateMeta;
use mfm_machine::plan::{DependencyEdge, StateGraph, StateNode};
use mfm_machine::recorder::EventRecorder;
use mfm_machine::state::{SnapshotPolicy, State, StateOutcome};
use mfm_op_common::ctx as op_ctx;
use mfm_op_common::errors as op_errors;
use mfm_op_common::output as op_output;
use mfm_op_common::states::meta;
use mfm_op_keystore_common::tx::output_context_key;
use mfm_sdk::errors::SdkError;
use mfm_sdk::ids::PortKey;
use mfm_sdk::op::{OpIo, Operation};

const OP_ID: &str = "portfolio_tracker";
const OP_VERSION: &str = "v1";

const KEY_CHAIN_ID: &str = "chain_id";
const KEY_BLOCK_NUMBER: &str = "block_number";
const KEY_NATIVE: &str = "native";
const KEY_SNAPSHOT_ARTIFACT_ID: &str = "snapshot_artifact_id";
const KEY_REPORT: &str = "report";

const ENV_PORTFOLIO_TOKENS_JSON: &str = "MFM_PORTFOLIO_TOKENS_JSON";

fn ctx_key(suffix: &'static str) -> ContextKey {
    ContextKey(suffix.to_string())
}

fn default_chain_id() -> u64 {
    1
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PortfolioTrackerReport {
    pub snapshot_artifact_id: String,
    pub chain_id: u64,
    pub block_number: u64,
}

pub fn portfolio_tracker_report_context_key() -> ContextKey {
    output_context_key("portfolio_tracker.main")
}

#[derive(Clone, Debug, Deserialize)]
struct TokenConfig {
    address: Address,
    #[serde(default)]
    symbol: Option<String>,
    #[serde(default)]
    decimals: Option<u8>,
}

#[derive(Clone, Debug, Deserialize)]
struct PortfolioTrackerConfig {
    wallet_address: Address,

    #[serde(default = "default_chain_id")]
    chain_id: u64,

    #[serde(default)]
    tokens: Vec<TokenConfig>,
}

#[derive(Clone, Debug, Deserialize)]
struct PortfolioSnapshotTokenConfig {
    address: String,
    #[serde(default)]
    symbol: Option<String>,
    #[serde(default)]
    decimals: Option<u8>,
}

#[derive(Clone, Debug, Deserialize)]
struct PortfolioSnapshotInputConfig {
    address: String,
    chain_id: Option<u64>,
    #[serde(default)]
    tokens: Vec<PortfolioSnapshotTokenConfig>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
enum PortfolioTrackerInputConfig {
    Tracker(PortfolioTrackerConfig),
    Snapshot(PortfolioSnapshotInputConfig),
}

fn sdk_input_error(code: &'static str, message: impl Into<String>) -> SdkError {
    op_errors::sdk_error(code, ErrorCategory::ParsingInput, false, message)
}

fn normalize_eth_address(s: &str) -> Option<String> {
    let s = s.trim();
    let rest = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X"))?;
    if rest.len() != 40 {
        return None;
    }
    if !rest.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    Some(format!("0x{}", rest.to_ascii_lowercase()))
}

fn load_portfolio_tokens_from_env() -> Result<Vec<PortfolioSnapshotTokenConfig>, SdkError> {
    let Ok(raw) = std::env::var(ENV_PORTFOLIO_TOKENS_JSON) else {
        return Ok(Vec::new());
    };

    serde_json::from_str::<Vec<PortfolioSnapshotTokenConfig>>(&raw).map_err(|_| {
        sdk_input_error(
            "InvalidPortfolioTokensJson",
            format!("invalid {ENV_PORTFOLIO_TOKENS_JSON}"),
        )
    })
}

fn parse_token_address(
    raw: &str,
    code: &'static str,
    message: &'static str,
) -> Result<Address, SdkError> {
    let normalized = normalize_eth_address(raw).ok_or_else(|| sdk_input_error(code, message))?;
    normalized
        .parse::<Address>()
        .map_err(|_| sdk_input_error(code, message))
}

fn normalize_snapshot_input(
    cfg: PortfolioSnapshotInputConfig,
) -> Result<PortfolioTrackerConfig, SdkError> {
    let wallet_address =
        parse_token_address(&cfg.address, "InvalidAddress", "invalid ethereum address")?;

    let mut merged: HashMap<String, PortfolioSnapshotTokenConfig> = HashMap::new();

    for token in load_portfolio_tokens_from_env()? {
        let normalized = normalize_eth_address(&token.address).ok_or_else(|| {
            sdk_input_error(
                "InvalidPortfolioTokensJson",
                "invalid token address in MFM_PORTFOLIO_TOKENS_JSON",
            )
        })?;
        merged.insert(
            normalized.clone(),
            PortfolioSnapshotTokenConfig {
                address: normalized,
                symbol: token.symbol,
                decimals: token.decimals,
            },
        );
    }

    for token in cfg.tokens {
        let normalized = normalize_eth_address(&token.address)
            .ok_or_else(|| sdk_input_error("InvalidTokenAddress", "invalid token address"))?;
        merged.insert(
            normalized.clone(),
            PortfolioSnapshotTokenConfig {
                address: normalized,
                symbol: token.symbol,
                decimals: token.decimals,
            },
        );
    }

    let mut addresses: Vec<String> = merged.keys().cloned().collect();
    addresses.sort();

    let mut tokens = Vec::with_capacity(addresses.len());
    for address in addresses {
        let Some(token) = merged.remove(&address) else {
            continue;
        };
        tokens.push(TokenConfig {
            address: parse_token_address(
                &token.address,
                "InvalidTokenAddress",
                "invalid token address",
            )?,
            symbol: token.symbol,
            decimals: token.decimals,
        });
    }

    Ok(PortfolioTrackerConfig {
        wallet_address,
        chain_id: cfg.chain_id.unwrap_or_else(default_chain_id),
        tokens,
    })
}

fn parse_config(op_config: &serde_json::Value) -> Result<PortfolioTrackerConfig, SdkError> {
    let cfg: PortfolioTrackerInputConfig =
        serde_json::from_value(op_config.clone()).map_err(|_| {
            op_errors::sdk_parse_error("invalid_op_config", "invalid portfolio_tracker op_config")
        })?;

    match cfg {
        PortfolioTrackerInputConfig::Tracker(cfg) => Ok(cfg),
        PortfolioTrackerInputConfig::Snapshot(cfg) => normalize_snapshot_input(cfg),
    }
}

fn output_fact_key(op_path: &OpPath) -> FactKey {
    FactKey(format!("portfolio:output|op:{}", op_path.0))
}

fn ctx_key_erc20_token(token_addr_no0x: &str) -> ContextKey {
    ContextKey(format!("erc20.{}", token_addr_no0x))
}

#[derive(Clone, Default)]
pub struct PortfolioTrackerOp;

impl Operation for PortfolioTrackerOp {
    fn op_id(&self) -> OpId {
        OpId(OP_ID.to_string())
    }

    fn op_version(&self) -> String {
        OP_VERSION.to_string()
    }

    fn io(&self, _op_config: &serde_json::Value) -> Result<OpIo, SdkError> {
        Ok(OpIo {
            imports: Vec::new(),
            exports: vec![
                PortKey(KEY_CHAIN_ID.to_string()),
                PortKey(KEY_BLOCK_NUMBER.to_string()),
                PortKey(KEY_SNAPSHOT_ARTIFACT_ID.to_string()),
                PortKey(KEY_REPORT.to_string()),
            ],
        })
    }

    fn expand(
        &self,
        op_path: OpPath,
        op_config: &serde_json::Value,
        _run_config: &RunConfig,
    ) -> Result<StateGraph, SdkError> {
        let mut cfg = parse_config(op_config)?;

        cfg.tokens
            .sort_by_key(|t| address_hex_lower_no0x(&t.address));

        let mut states: Vec<StateNode> = Vec::new();
        let mut edges: Vec<DependencyEdge> = Vec::new();

        // chain id (validates network)
        let chain_id_sid = StateId(format!("{}.chain_id", op_path.0));
        states.push(StateNode {
            id: chain_id_sid.clone(),
            state: Arc::new(
                ReadU64HexState::new(
                    chain_id_sid.clone(),
                    "eth_chainId",
                    serde_json::json!([]),
                    ctx_key(KEY_CHAIN_ID),
                )
                .with_expectation(U64Expectation::parsing_input(
                    cfg.chain_id,
                    "chain_id_mismatch",
                    "rpc chain_id did not match configured chain_id",
                )),
            ),
        });
        let mut last = chain_id_sid;

        // block number
        let block_sid = StateId(format!("{}.block_number", op_path.0));
        edges.push(DependencyEdge {
            from: last.clone(),
            to: block_sid.clone(),
        });
        states.push(StateNode {
            id: block_sid.clone(),
            state: Arc::new(ReadU64HexState::new(
                block_sid.clone(),
                "eth_blockNumber",
                serde_json::json!([]),
                ctx_key(KEY_BLOCK_NUMBER),
            )),
        });
        last = block_sid;

        // ETH balance
        let eth_sid = StateId(format!("{}.eth_balance", op_path.0));
        edges.push(DependencyEdge {
            from: last.clone(),
            to: eth_sid.clone(),
        });
        states.push(StateNode {
            id: eth_sid.clone(),
            state: Arc::new(NativeBalanceState::new(
                eth_sid.clone(),
                cfg.wallet_address,
                ctx_key(KEY_BLOCK_NUMBER),
                ctx_key(KEY_NATIVE),
            )),
        });
        last = eth_sid;

        // ERC-20 balances (allowlist)
        for t in cfg.tokens.clone() {
            let addr_no0x = address_hex_lower_no0x(&t.address);
            let sid = StateId(format!("{}.token_balance_{}", op_path.0, addr_no0x));
            edges.push(DependencyEdge {
                from: last.clone(),
                to: sid.clone(),
            });
            states.push(StateNode {
                id: sid.clone(),
                state: Arc::new(TokenBalanceState::new(
                    sid.clone(),
                    t.address,
                    cfg.wallet_address,
                    t.symbol,
                    t.decimals,
                    ctx_key(KEY_BLOCK_NUMBER),
                    ctx_key_erc20_token(&addr_no0x),
                )),
            });
            last = sid;
        }

        // write snapshot output
        let out_sid = StateId(format!("{}.write_snapshot", op_path.0));
        edges.push(DependencyEdge {
            from: last,
            to: out_sid.clone(),
        });
        states.push(StateNode {
            id: out_sid.clone(),
            state: Arc::new(WriteSnapshotState { op_path, cfg }),
        });

        Ok(StateGraph { states, edges })
    }
}

struct WriteSnapshotState {
    op_path: OpPath,
    cfg: PortfolioTrackerConfig,
}

#[async_trait]
impl State for WriteSnapshotState {
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
            &ctx_key(KEY_CHAIN_ID),
            "missing_chain_id",
            "missing chain_id in context",
        )?;
        let block_number = op_ctx::read_u64_required(
            ctx,
            &ctx_key(KEY_BLOCK_NUMBER),
            "missing_block_number",
            "missing block_number in context",
        )?;

        let native = op_ctx::read_json_required(
            ctx,
            &ctx_key(KEY_NATIVE),
            "missing_native",
            "missing native balance in context",
        )?;

        let mut tokens = Vec::with_capacity(self.cfg.tokens.len());
        for t in &self.cfg.tokens {
            let addr_no0x = address_hex_lower_no0x(&t.address);
            let k = ctx_key_erc20_token(&addr_no0x);
            let tok = op_ctx::read_json_required(
                ctx,
                &k,
                "missing_token",
                "missing token balance in context",
            )?;
            tokens.push(tok);
        }

        let generated_at_ms = io.now_millis().await.map_err(op_errors::state_from_io)?;
        let snapshot = serde_json::json!({
            "wallet_address": address_hex_lower(&self.cfg.wallet_address),
            "chain_id": chain_id,
            "block_number": block_number,
            "generated_at_ms": generated_at_ms,
            "native": native,
            "tokens": tokens,
            "errors": [],
        });

        op_output::write_output_artifact(
            ctx,
            io,
            rec,
            output_fact_key(&self.op_path),
            snapshot,
            ctx_key(KEY_SNAPSHOT_ARTIFACT_ID),
        )
        .await?;

        let snapshot_artifact_id = op_ctx::read_string_required(
            ctx,
            &ctx_key(KEY_SNAPSHOT_ARTIFACT_ID),
            "missing_snapshot_artifact_id",
            "missing snapshot artifact id in context",
            "snapshot_artifact_id_not_string",
            "snapshot artifact id in context must be a string",
        )?;

        let report = PortfolioTrackerReport {
            snapshot_artifact_id,
            chain_id,
            block_number,
        };
        let report_json = serde_json::to_value(&report).map_err(|_| {
            op_errors::state_unknown(
                "serialize_report_failed",
                "failed to serialize snapshot report",
            )
        })?;
        op_ctx::write_json(ctx, ctx_key(KEY_REPORT), report_json.clone())?;
        rec.emit(DomainEvent {
            name: "portfolio_tracker.completed".to_string(),
            payload: report_json,
            payload_ref: None,
        })
        .await
        .map_err(|_| op_errors::state_unknown("emit_failed", "failed to emit domain event"))?;

        Ok(StateOutcome {
            snapshot: SnapshotPolicy::OnSuccess,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use mfm_machine::engine::{ExecutionEngine, RunPhase, Stores};
    use mfm_machine::errors::{ErrorCategory, ErrorInfo, IoError};
    use mfm_machine::events::{Event, KernelEvent};
    use mfm_machine::ids::ArtifactId;
    use mfm_machine::io::IoCall;
    use mfm_machine::live_io::{LiveIoTransport, LiveIoTransportFactory};
    use mfm_machine::live_io_router::RouterLiveIoTransportFactory;
    use mfm_machine::runtime::DefaultExecutionEngine;
    use mfm_op_common::test_support as op_test_support;
    use mfm_sdk::unstable::SdkPlanResolver;

    fn info(
        code: &'static str,
        category: ErrorCategory,
        retryable: bool,
        message: &str,
    ) -> ErrorInfo {
        op_errors::info(code, category, retryable, message)
    }

    #[derive(Clone)]
    struct MockEvmTransportFactory {
        chain_id_hex: String,
    }

    impl Default for MockEvmTransportFactory {
        fn default() -> Self {
            Self {
                chain_id_hex: "0x1".to_string(),
            }
        }
    }

    impl LiveIoTransportFactory for MockEvmTransportFactory {
        fn namespace_group(&self) -> &str {
            "evm"
        }

        fn make(&self, _env: mfm_machine::live_io::LiveIoEnv) -> Box<dyn LiveIoTransport> {
            Box::new(MockEvmTransport {
                chain_id_hex: self.chain_id_hex.clone(),
            })
        }
    }

    struct MockEvmTransport {
        chain_id_hex: String,
    }

    fn ok_u256_as_32byte_hex(n: u64) -> serde_json::Value {
        let hex_val = format!("{:x}", n);
        let padded = format!("{}{}", "0".repeat(64 - hex_val.len()), hex_val);
        serde_json::json!(format!("0x{padded}"))
    }

    #[async_trait]
    impl LiveIoTransport for MockEvmTransport {
        async fn call(&mut self, call: IoCall) -> Result<serde_json::Value, IoError> {
            let method = call
                .request
                .get("method")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");

            match method {
                "eth_chainId" => Ok(serde_json::Value::String(self.chain_id_hex.clone())),
                "eth_blockNumber" => Ok(serde_json::json!("0x64")),
                "eth_getBalance" => Ok(serde_json::json!("0xde0b6b3a7640000")), // 1 ETH
                "eth_call" => {
                    let data = call
                        .request
                        .get("params")
                        .and_then(|v| v.as_array())
                        .and_then(|a| a.first())
                        .and_then(|v| v.as_object())
                        .and_then(|o| o.get("data"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("");

                    if data == encode_erc20_decimals() {
                        Ok(ok_u256_as_32byte_hex(6))
                    } else if data.starts_with("0x70a08231") {
                        Ok(ok_u256_as_32byte_hex(1_000_000)) // 1 token @ 6 decimals
                    } else {
                        Err(IoError::Other(info(
                            "unknown_eth_call",
                            ErrorCategory::Unknown,
                            false,
                            "unknown eth_call payload",
                        )))
                    }
                }
                _ => Err(IoError::Other(info(
                    "unknown_method",
                    ErrorCategory::Unknown,
                    false,
                    "unknown jsonrpc method",
                ))),
            }
        }
    }

    async fn run_portfolio_snapshot_with_transports(
        op_config: serde_json::Value,
        evm_factory: Arc<dyn LiveIoTransportFactory>,
    ) -> (mfm_machine::engine::RunResult, Stores) {
        let op: mfm_sdk::op::DynOperation = Arc::new(PortfolioTrackerOp);
        let (registry, planner, pipeline) =
            op_test_support::single_op_plan(op, op_config).expect("pipeline");

        let resolver = Arc::new(SdkPlanResolver::new(
            Arc::clone(&registry),
            Arc::clone(&planner),
        ));

        let factory: Arc<dyn LiveIoTransportFactory> = Arc::new(
            RouterLiveIoTransportFactory::from_factories(vec![evm_factory])
                .expect("router factories"),
        );

        let engine =
            DefaultExecutionEngine::new(resolver).with_live_transport_factory(Arc::clone(&factory));
        let engine: Arc<dyn ExecutionEngine> = Arc::new(engine);

        let stores = op_test_support::in_memory_stores();

        let res = op_test_support::start_pipeline_with_defaults(
            Arc::clone(&engine),
            &stores,
            Arc::clone(&registry),
            Arc::clone(&planner),
            pipeline,
            op_test_support::run_config_live(),
        )
        .await
        .expect("run");

        (res, stores)
    }

    #[test]
    fn expand_sorts_tokens_and_uses_full_address_in_state_id() {
        let op = PortfolioTrackerOp;
        let g = op
            .expand(
                OpPath("portfolio_tracker.main".to_string()),
                &serde_json::json!({
                    "wallet_address": "0x000000000000000000000000000000000000dead",
                    "chain_id": 1,
                    "tokens": [
                        {
                            "address": "0x000000000000000000000000000000000000beef",
                            "symbol": "BEEF",
                            "decimals": 6
                        },
                        {
                            "address": "0x0000000000000000000000000000000000000001",
                            "symbol": "ONE",
                            "decimals": 6
                        }
                    ]
                }),
                &op_test_support::run_config_live(),
            )
            .expect("expand");

        let ids: Vec<String> = g.states.iter().map(|n| n.id.0.clone()).collect();

        let a1 = "0000000000000000000000000000000000000001";
        let abeef = "000000000000000000000000000000000000beef";
        assert_eq!(a1.len(), 40);
        assert_eq!(abeef.len(), 40);

        let i1 = ids
            .iter()
            .position(|id| id == &format!("portfolio_tracker.main.token_balance_{a1}"))
            .expect("token_balance state for 0x...0001");
        let ib = ids
            .iter()
            .position(|id| id == &format!("portfolio_tracker.main.token_balance_{abeef}"))
            .expect("token_balance state for 0x...beef");

        assert!(i1 < ib, "token balance states must be sorted by address");
    }

    #[tokio::test]
    async fn chain_id_mismatch_fails_with_stable_code() {
        let (res, stores) = run_portfolio_snapshot_with_transports(
            serde_json::json!({
                "wallet_address": "0x000000000000000000000000000000000000dead",
                "chain_id": 1,
                "tokens": []
            }),
            Arc::new(MockEvmTransportFactory {
                chain_id_hex: "0x2".to_string(),
            }),
        )
        .await;

        assert_eq!(res.phase, RunPhase::Failed);

        let stream = stores
            .events
            .read_range(res.run_id, 1, None)
            .await
            .expect("events");

        let mut found = false;
        for e in &stream {
            let Event::Kernel(KernelEvent::StateFailed {
                state_id, error, ..
            }) = &e.event
            else {
                continue;
            };
            if state_id.0 != "portfolio_tracker.main.chain_id" {
                continue;
            }
            assert_eq!(error.info.code.0, "chain_id_mismatch");
            found = true;
            break;
        }
        assert!(found, "expected StateFailed for chain_id state");
    }

    #[tokio::test]
    async fn snapshot_decimals_fallback_calls_decimals_and_formats_amount() {
        let (res, stores) = run_portfolio_snapshot_with_transports(
            serde_json::json!({
                "wallet_address": "0x000000000000000000000000000000000000dead",
                "chain_id": 1,
                "tokens": [
                    {
                        "address": "0x000000000000000000000000000000000000beef",
                        "symbol": "TKN",
                        "decimals": null
                    }
                ]
            }),
            Arc::new(MockEvmTransportFactory::default()),
        )
        .await;

        assert_eq!(res.phase, RunPhase::Completed);

        let final_snapshot_id = res.final_snapshot_id.expect("final snapshot id");
        let bytes = stores
            .artifacts
            .get(&final_snapshot_id)
            .await
            .expect("snapshot bytes");
        let snapshot: serde_json::Value = serde_json::from_slice(&bytes).expect("snapshot json");

        let out_id = snapshot
            .get("portfolio_tracker.main.snapshot_artifact_id")
            .and_then(|v| v.as_str())
            .expect("snapshot_artifact_id (context key) missing")
            .to_string();

        let out_bytes = stores
            .artifacts
            .get(&ArtifactId(out_id))
            .await
            .expect("output artifact bytes");
        let out: serde_json::Value = serde_json::from_slice(&out_bytes).expect("output json");

        let tokens = out
            .get("tokens")
            .and_then(|v| v.as_array())
            .expect("tokens array");
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0].get("decimals").and_then(|v| v.as_u64()), Some(6));
        assert_eq!(
            tokens[0].get("amount_dec").and_then(|v| v.as_str()),
            Some("1.000000")
        );
    }

    #[tokio::test]
    async fn snapshot_token_ordering_is_deterministic() {
        let (res, stores) = run_portfolio_snapshot_with_transports(
            serde_json::json!({
                "wallet_address": "0x000000000000000000000000000000000000dead",
                "chain_id": 1,
                "tokens": [
                    {
                        "address": "0x000000000000000000000000000000000000beef",
                        "symbol": "BEEF",
                        "decimals": 6
                    },
                    {
                        "address": "0x0000000000000000000000000000000000000001",
                        "symbol": "ONE",
                        "decimals": 6
                    }
                ]
            }),
            Arc::new(MockEvmTransportFactory::default()),
        )
        .await;

        assert_eq!(res.phase, RunPhase::Completed);

        let final_snapshot_id = res.final_snapshot_id.expect("final snapshot id");
        let bytes = stores
            .artifacts
            .get(&final_snapshot_id)
            .await
            .expect("snapshot bytes");
        let snapshot: serde_json::Value = serde_json::from_slice(&bytes).expect("snapshot json");

        let out_id = snapshot
            .get("portfolio_tracker.main.snapshot_artifact_id")
            .and_then(|v| v.as_str())
            .expect("snapshot_artifact_id (context key) missing")
            .to_string();

        let out_bytes = stores
            .artifacts
            .get(&ArtifactId(out_id))
            .await
            .expect("output artifact bytes");
        let out: serde_json::Value = serde_json::from_slice(&out_bytes).expect("output json");

        let tokens = out
            .get("tokens")
            .and_then(|v| v.as_array())
            .expect("tokens array");
        assert_eq!(
            tokens
                .iter()
                .filter_map(|t| t.get("address").and_then(|v| v.as_str()))
                .collect::<Vec<_>>(),
            vec![
                "0x0000000000000000000000000000000000000001",
                "0x000000000000000000000000000000000000beef",
            ]
        );
    }

    #[tokio::test]
    async fn snapshot_writes_output_artifact() {
        let (res, stores) = run_portfolio_snapshot_with_transports(
            serde_json::json!({
                "wallet_address": "0x000000000000000000000000000000000000dead",
                "chain_id": 1,
                "tokens": [
                    {
                        "address": "0x000000000000000000000000000000000000beef",
                        "symbol": "TKN",
                        "decimals": 6
                    }
                ]
            }),
            Arc::new(MockEvmTransportFactory::default()),
        )
        .await;

        assert_eq!(res.phase, RunPhase::Completed);

        let stream = stores
            .events
            .read_range(res.run_id, 1, None)
            .await
            .expect("events");
        let final_snapshot_id =
            op_test_support::run_completed_snapshot_id(&stream).expect("final snapshot id");

        let bytes = stores
            .artifacts
            .get(&final_snapshot_id)
            .await
            .expect("snapshot bytes");
        let snapshot: serde_json::Value = serde_json::from_slice(&bytes).expect("snapshot json");

        let out_id = snapshot
            .get("portfolio_tracker.main.snapshot_artifact_id")
            .and_then(|v| v.as_str())
            .unwrap_or_else(|| panic!("snapshot_artifact_id (context key) missing: {snapshot}"));

        let out_bytes = stores
            .artifacts
            .get(&ArtifactId(out_id.to_string()))
            .await
            .expect("output artifact bytes");
        let out: serde_json::Value = serde_json::from_slice(&out_bytes).expect("output json");

        assert_eq!(
            out.get("wallet_address").and_then(|v| v.as_str()),
            Some("0x000000000000000000000000000000000000dead")
        );
        assert_eq!(out.get("chain_id").and_then(|v| v.as_u64()), Some(1));
        assert_eq!(out.get("block_number").and_then(|v| v.as_u64()), Some(100));
        assert_eq!(
            out.get("tokens")
                .and_then(|v| v.as_array())
                .map(|a| a.len()),
            Some(1)
        );
    }
}
