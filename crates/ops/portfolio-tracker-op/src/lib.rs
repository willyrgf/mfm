//! Portfolio tracker operation (Milestone 2+).
//!
//! Source of truth: `REDESIGN.md` (v4).
//!
//! Current scope (v1):
//! - validate `eth_chainId` matches configured `chain_id` (default: 1)
//! - fetch pinned `eth_blockNumber`
//! - fetch native ETH balance via `eth_getBalance` at that pinned block
//! - fetch allowlisted ERC-20 balances via `eth_call(balanceOf)` at that pinned block
//! - write a content-addressed snapshot output artifact via `portfolio.output`

use std::sync::Arc;

use async_trait::async_trait;
use serde::Deserialize;

use alloy_primitives::{Address, U256};
use mfm_collectors_evm::{EvmIoClient, JsonRpcCall};
use mfm_machine::config::RunConfig;
use mfm_machine::context::DynContext;
use mfm_machine::errors::{ErrorCategory, IoError, StateError};
use mfm_machine::ids::{ContextKey, FactKey, OpId, OpPath, StateId};
use mfm_machine::io::IoProvider;
use mfm_machine::meta::StateMeta;
use mfm_machine::plan::{DependencyEdge, StateGraph, StateNode};
use mfm_machine::recorder::EventRecorder;
use mfm_machine::state::{SnapshotPolicy, State, StateOutcome};
use mfm_op_common::ctx as op_ctx;
use mfm_op_common::errors as op_errors;
use mfm_op_common::output as op_output;
use mfm_op_common::states::evm::{ReadU64HexState, U64Expectation};
use mfm_op_common::states::meta;
use mfm_sdk::errors::SdkError;
use mfm_sdk::ids::PortKey;
use mfm_sdk::op::{OpIo, Operation};

const OP_ID: &str = "portfolio_tracker";
const OP_VERSION: &str = "v1";

const KEY_CHAIN_ID: &str = "chain_id";
const KEY_BLOCK_NUMBER: &str = "block_number";
const KEY_NATIVE: &str = "native";
const KEY_SNAPSHOT_ARTIFACT_ID: &str = "snapshot_artifact_id";

fn sdk_err(code: &'static str, message: &'static str) -> SdkError {
    op_errors::sdk_error(code, ErrorCategory::ParsingInput, false, message)
}

fn state_err(code: &'static str, message: &'static str) -> StateError {
    op_errors::state_unknown(code, message)
}

fn state_err_from_io(err: IoError) -> StateError {
    op_errors::state_from_io(err)
}

fn ctx_key(suffix: &'static str) -> ContextKey {
    ContextKey(suffix.to_string())
}

fn default_chain_id() -> u64 {
    1
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

fn address_hex_lower(addr: &Address) -> String {
    // `Debug` formatting is lowercase (non-checksummed), which is stable and safe for IDs.
    format!("{addr:?}")
}

fn address_hex_lower_no0x(addr: &Address) -> String {
    address_hex_lower(addr)
        .strip_prefix("0x")
        .unwrap_or("")
        .to_string()
}

fn u64_hex_quantity(n: u64) -> String {
    // JSON-RPC quantities are 0x-prefixed, no leading zeros.
    if n == 0 {
        return "0x0".to_string();
    }
    format!("0x{:x}", n)
}

fn format_u256_units(raw: &U256, decimals: u8) -> String {
    let s = raw.to_string();
    let d = decimals as usize;
    if d == 0 {
        return s;
    }
    if s.len() <= d {
        let mut out = String::with_capacity(2 + d + 1);
        out.push_str("0.");
        out.push_str(&"0".repeat(d - s.len()));
        out.push_str(&s);
        out
    } else {
        let split = s.len() - d;
        let mut out = String::with_capacity(s.len() + 1);
        out.push_str(&s[..split]);
        out.push('.');
        out.push_str(&s[split..]);
        out
    }
}

fn parse_u256_hex(s: &str) -> Result<U256, String> {
    let Some(rest) = s.strip_prefix("0x") else {
        return Err("missing 0x prefix".to_string());
    };
    if rest.is_empty() {
        return Err("empty hex string".to_string());
    }
    if rest.len() > 64 {
        return Err("hex value overflowed u256".to_string());
    }

    let mut hex_str = rest.to_string();
    if hex_str.len() % 2 == 1 {
        hex_str = format!("0{hex_str}");
    }
    let bytes = hex::decode(hex_str).map_err(|_| "invalid hex".to_string())?;
    Ok(U256::from_be_slice(&bytes))
}

fn parse_u256_hex_value(v: &serde_json::Value) -> Result<U256, StateError> {
    let Some(s) = v.as_str() else {
        return Err(state_err(
            "evm_response_invalid",
            "evm response was not a hex string",
        ));
    };
    parse_u256_hex(s)
        .map_err(|_| state_err("evm_response_invalid", "evm response was not a hex u256"))
}

fn parse_u8_u256(v: U256) -> Result<u8, StateError> {
    if v > U256::from(u8::MAX) {
        return Err(state_err(
            "evm_response_invalid",
            "evm response was out of range for u8",
        ));
    }
    Ok(v.to::<u8>())
}

fn output_fact_key(op_path: &OpPath) -> FactKey {
    FactKey(format!("portfolio:output|op:{}", op_path.0))
}

fn ctx_key_erc20_token(token_addr_no0x: &str) -> ContextKey {
    ContextKey(format!("erc20.{}", token_addr_no0x))
}

fn erc20_selector_balance_of() -> [u8; 4] {
    // keccak256("balanceOf(address)")[..4]
    [0x70, 0xa0, 0x82, 0x31]
}

fn erc20_selector_decimals() -> [u8; 4] {
    // keccak256("decimals()")[..4]
    [0x31, 0x3c, 0xe5, 0x67]
}

fn encode_erc20_balance_of(owner: &Address) -> String {
    let mut data = Vec::with_capacity(4 + 32);
    data.extend_from_slice(&erc20_selector_balance_of());
    data.extend_from_slice(&[0u8; 12]);
    data.extend_from_slice(owner.as_slice());
    format!("0x{}", hex::encode(data))
}

fn encode_erc20_decimals() -> String {
    let mut data = Vec::with_capacity(4);
    data.extend_from_slice(&erc20_selector_decimals());
    format!("0x{}", hex::encode(data))
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
            ],
        })
    }

    fn expand(
        &self,
        op_path: OpPath,
        op_config: &serde_json::Value,
        _run_config: &RunConfig,
    ) -> Result<StateGraph, SdkError> {
        let mut cfg: PortfolioTrackerConfig = serde_json::from_value(op_config.clone())
            .map_err(|_| sdk_err("invalid_op_config", "invalid portfolio_tracker op_config"))?;

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
            state: Arc::new(ReadEthBalanceState {
                state_id: eth_sid.clone(),
                wallet: cfg.wallet_address,
            }),
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
                state: Arc::new(ReadErc20BalanceState {
                    state_id: sid.clone(),
                    token: t,
                    wallet: cfg.wallet_address,
                }),
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

struct ReadEthBalanceState {
    state_id: StateId,
    wallet: Address,
}

#[async_trait]
impl State for ReadEthBalanceState {
    fn meta(&self) -> StateMeta {
        meta::fetch_data()
    }

    async fn handle(
        &self,
        ctx: &mut dyn DynContext,
        io: &mut dyn IoProvider,
        _rec: &mut dyn EventRecorder,
    ) -> Result<StateOutcome, StateError> {
        let block = op_ctx::read_u64_required(
            ctx,
            &ctx_key(KEY_BLOCK_NUMBER),
            "missing_block_number",
            "missing block_number in context",
        )?;

        let mut client = EvmIoClient::new(self.state_id.clone(), io);
        let res = client
            .call(JsonRpcCall::new(
                "eth_getBalance",
                serde_json::json!([address_hex_lower(&self.wallet), u64_hex_quantity(block)]),
            ))
            .await
            .map_err(state_err_from_io)?;

        let wei = parse_u256_hex_value(&res.response)?;
        let native = serde_json::json!({
            "symbol": "ETH",
            "raw_u256_dec": wei.to_string(),
            "decimals": 18,
            "amount_dec": format_u256_units(&wei, 18),
        });

        op_ctx::write_json(ctx, ctx_key(KEY_NATIVE), native)?;

        Ok(StateOutcome {
            snapshot: SnapshotPolicy::OnSuccess,
        })
    }
}

struct ReadErc20BalanceState {
    state_id: StateId,
    token: TokenConfig,
    wallet: Address,
}

#[async_trait]
impl State for ReadErc20BalanceState {
    fn meta(&self) -> StateMeta {
        meta::fetch_data()
    }

    async fn handle(
        &self,
        ctx: &mut dyn DynContext,
        io: &mut dyn IoProvider,
        _rec: &mut dyn EventRecorder,
    ) -> Result<StateOutcome, StateError> {
        let block = op_ctx::read_u64_required(
            ctx,
            &ctx_key(KEY_BLOCK_NUMBER),
            "missing_block_number",
            "missing block_number in context",
        )?;

        let addr_no0x = address_hex_lower_no0x(&self.token.address);
        let token_ctx_key = ctx_key_erc20_token(&addr_no0x);

        let token_to = address_hex_lower(&self.token.address);
        let at_block = u64_hex_quantity(block);
        let mut client = EvmIoClient::new(self.state_id.clone(), io);

        let decimals: u8 = match self.token.decimals {
            Some(d) => d,
            None => {
                let res = client
                    .call(JsonRpcCall::new(
                        "eth_call",
                        serde_json::json!([
                            {"to": token_to, "data": encode_erc20_decimals()},
                            at_block.clone()
                        ]),
                    ))
                    .await
                    .map_err(state_err_from_io)?;
                let v = parse_u256_hex_value(&res.response)?;
                parse_u8_u256(v)?
            }
        };

        let res = client
            .call(JsonRpcCall::new(
                "eth_call",
                serde_json::json!([
                    {"to": address_hex_lower(&self.token.address), "data": encode_erc20_balance_of(&self.wallet)},
                    at_block
                ]),
            ))
            .await
            .map_err(state_err_from_io)?;

        let raw = parse_u256_hex_value(&res.response)?;
        let token_obj = serde_json::json!({
            "address": address_hex_lower(&self.token.address),
            "symbol": self.token.symbol.clone(),
            "decimals": decimals,
            "raw_u256_dec": raw.to_string(),
            "amount_dec": format_u256_units(&raw, decimals),
        });

        op_ctx::write_json(ctx, token_ctx_key, token_obj)?;

        Ok(StateOutcome {
            snapshot: SnapshotPolicy::OnSuccess,
        })
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

        let generated_at_ms = io.now_millis().await.map_err(state_err_from_io)?;
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
            "portfolio.output",
            output_fact_key(&self.op_path),
            snapshot,
            ctx_key(KEY_SNAPSHOT_ARTIFACT_ID),
        )
        .await?;

        Ok(StateOutcome {
            snapshot: SnapshotPolicy::OnSuccess,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::HashMap;

    use mfm_machine::config::BuildProvenance;
    use mfm_machine::engine::{ExecutionEngine, RunPhase, Stores};
    use mfm_machine::errors::{ErrorCategory, ErrorInfo};
    use mfm_machine::events::{Event, KernelEvent};
    use mfm_machine::ids::ArtifactId;
    use mfm_machine::io::IoCall;
    use mfm_machine::live_io::{LiveIoTransport, LiveIoTransportFactory};
    use mfm_machine::live_io_router::RouterLiveIoTransportFactory;
    use mfm_machine::runtime::DefaultExecutionEngine;
    use mfm_op_common::test_support as op_test_support;
    use mfm_sdk::launcher::{LaunchPipeline, RunLauncher};
    use mfm_sdk::pipeline::PipelinePlanner;
    use mfm_sdk::unstable::{
        single_op_pipeline, DefaultPipelinePlanner, DefaultRunLauncher, HashMapOperationRegistry,
        SdkPlanResolver,
    };

    fn info(
        code: &'static str,
        category: ErrorCategory,
        retryable: bool,
        message: &str,
    ) -> ErrorInfo {
        op_errors::info(code, category, retryable, message)
    }

    #[derive(Clone)]
    struct EchoTransportFactory;

    impl LiveIoTransportFactory for EchoTransportFactory {
        fn make(&self, _env: mfm_machine::live_io::LiveIoEnv) -> Box<dyn LiveIoTransport> {
            Box::new(EchoTransport)
        }
    }

    struct EchoTransport;

    #[async_trait]
    impl LiveIoTransport for EchoTransport {
        async fn call(&mut self, call: IoCall) -> Result<serde_json::Value, IoError> {
            Ok(call.request)
        }
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
        let mut reg = HashMapOperationRegistry::default();
        reg.register(Arc::clone(&op));
        let registry: Arc<dyn mfm_sdk::op::OperationRegistry> = Arc::new(reg);

        let planner: Arc<dyn PipelinePlanner> = Arc::new(DefaultPipelinePlanner);
        let pipeline =
            single_op_pipeline(op.op_id(), op.op_version(), op_config).expect("pipeline");

        let resolver = Arc::new(SdkPlanResolver::new(
            Arc::clone(&registry),
            Arc::clone(&planner),
        ));

        let mut routes: HashMap<String, Arc<dyn LiveIoTransportFactory>> = HashMap::new();
        routes.insert("evm".to_string(), evm_factory);
        routes.insert("portfolio".to_string(), Arc::new(EchoTransportFactory));
        let factory: Arc<dyn LiveIoTransportFactory> =
            Arc::new(RouterLiveIoTransportFactory::new(routes));

        let engine =
            DefaultExecutionEngine::new(resolver).with_live_transport_factory(Arc::clone(&factory));
        let engine: Arc<dyn ExecutionEngine> = Arc::new(engine);

        let stores = op_test_support::in_memory_stores();

        let launcher: Arc<dyn RunLauncher> = Arc::new(DefaultRunLauncher);
        let res = launcher
            .start_pipeline(
                Arc::clone(&engine),
                Stores {
                    events: Arc::clone(&stores.events),
                    artifacts: Arc::clone(&stores.artifacts),
                },
                Arc::clone(&registry),
                Arc::clone(&planner),
                LaunchPipeline {
                    pipeline,
                    input: serde_json::json!({}),
                    run_config: op_test_support::run_config_live(),
                    build: BuildProvenance {
                        git_commit: None,
                        cargo_lock_hash: None,
                        flake_lock_hash: None,
                        rustc_version: None,
                        target_triple: None,
                        env_allowlist: Vec::new(),
                    },
                    initial_context: Box::new(op_test_support::MapContext::default()),
                },
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
