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
use mfm_collectors_evm::{parse_u64_hex_value, EvmIoClient, JsonRpcCall};
use mfm_machine::config::RunConfig;
use mfm_machine::context::DynContext;
use mfm_machine::errors::{ErrorCategory, ErrorInfo, IoError, StateError};
use mfm_machine::events::{ArtifactWritten, DomainEvent, DOMAIN_EVENT_ARTIFACT_WRITTEN};
use mfm_machine::ids::{ContextKey, ErrorCode, FactKey, OpId, OpPath, StateId};
use mfm_machine::io::{IoCall, IoProvider};
use mfm_machine::meta::{DependencyStrategy, Idempotency, SideEffectKind, StateMeta, Tag};
use mfm_machine::plan::{DependencyEdge, StateGraph, StateNode};
use mfm_machine::recorder::EventRecorder;
use mfm_machine::state::{SnapshotPolicy, State, StateOutcome};
use mfm_machine::stores::ArtifactKind;
use mfm_sdk::errors::SdkError;
use mfm_sdk::ids::PortKey;
use mfm_sdk::op::{OpIo, Operation};

const OP_ID: &str = "portfolio_tracker";
const OP_VERSION: &str = "v1";

const KEY_CHAIN_ID: &str = "chain_id";
const KEY_BLOCK_NUMBER: &str = "block_number";
const KEY_NATIVE: &str = "native";
const KEY_SNAPSHOT_ARTIFACT_ID: &str = "snapshot_artifact_id";

fn info(code: &'static str, category: ErrorCategory, retryable: bool, message: &str) -> ErrorInfo {
    ErrorInfo {
        code: ErrorCode(code.to_string()),
        category,
        retryable,
        message: message.to_string(),
        details: None,
    }
}

fn sdk_err(code: &'static str, message: &'static str) -> SdkError {
    SdkError {
        info: info(code, ErrorCategory::ParsingInput, false, message),
    }
}

fn state_err(code: &'static str, message: &'static str) -> StateError {
    StateError {
        state_id: None,
        info: info(code, ErrorCategory::Unknown, false, message),
    }
}

fn state_err_from_io(err: IoError) -> StateError {
    let info = match err {
        IoError::MissingFactKey(info)
        | IoError::Transport(info)
        | IoError::RateLimited(info)
        | IoError::Other(info)
        | IoError::MissingFact { info, .. } => info,
    };

    StateError {
        state_id: None,
        info,
    }
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
            state: Arc::new(ReadChainIdState {
                state_id: chain_id_sid.clone(),
                expected_chain_id: cfg.chain_id,
            }),
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
            state: Arc::new(ReadBlockNumberState {
                state_id: block_sid.clone(),
            }),
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

struct ReadChainIdState {
    state_id: StateId,
    expected_chain_id: u64,
}

#[async_trait]
impl State for ReadChainIdState {
    fn meta(&self) -> StateMeta {
        StateMeta {
            tags: vec![Tag("fetch_data".to_string())],
            depends_on: Vec::new(),
            depends_on_strategy: DependencyStrategy::Latest,
            side_effects: SideEffectKind::ReadOnlyIo,
            idempotency: Idempotency::None,
        }
    }

    async fn handle(
        &self,
        ctx: &mut dyn DynContext,
        io: &mut dyn IoProvider,
        _rec: &mut dyn EventRecorder,
    ) -> Result<StateOutcome, StateError> {
        let mut client = EvmIoClient::new(self.state_id.clone(), io);
        let res = client
            .call(JsonRpcCall::new("eth_chainId", serde_json::json!([])))
            .await
            .map_err(state_err_from_io)?;

        let chain_id = parse_u64_hex_value(&res.response)
            .map_err(|_| state_err("evm_response_invalid", "evm response was not a hex u64"))?;
        if chain_id != self.expected_chain_id {
            return Err(StateError {
                state_id: Some(self.state_id.clone()),
                info: info(
                    "chain_id_mismatch",
                    ErrorCategory::ParsingInput,
                    false,
                    "rpc chain_id did not match configured chain_id",
                ),
            });
        }

        ctx.write(ctx_key(KEY_CHAIN_ID), serde_json::json!(chain_id))
            .map_err(|_| state_err("ctx_write_failed", "context write failed"))?;

        Ok(StateOutcome {
            snapshot: SnapshotPolicy::OnSuccess,
        })
    }
}

struct ReadBlockNumberState {
    state_id: StateId,
}

#[async_trait]
impl State for ReadBlockNumberState {
    fn meta(&self) -> StateMeta {
        StateMeta {
            tags: vec![Tag("fetch_data".to_string())],
            depends_on: Vec::new(),
            depends_on_strategy: DependencyStrategy::Latest,
            side_effects: SideEffectKind::ReadOnlyIo,
            idempotency: Idempotency::None,
        }
    }

    async fn handle(
        &self,
        ctx: &mut dyn DynContext,
        io: &mut dyn IoProvider,
        _rec: &mut dyn EventRecorder,
    ) -> Result<StateOutcome, StateError> {
        let mut client = EvmIoClient::new(self.state_id.clone(), io);
        let res = client
            .call(JsonRpcCall::new("eth_blockNumber", serde_json::json!([])))
            .await
            .map_err(state_err_from_io)?;

        let n = parse_u64_hex_value(&res.response)
            .map_err(|_| state_err("evm_response_invalid", "evm response was not a hex u64"))?;

        ctx.write(ctx_key(KEY_BLOCK_NUMBER), serde_json::json!(n))
            .map_err(|_| state_err("ctx_write_failed", "context write failed"))?;

        Ok(StateOutcome {
            snapshot: SnapshotPolicy::OnSuccess,
        })
    }
}

struct ReadEthBalanceState {
    state_id: StateId,
    wallet: Address,
}

#[async_trait]
impl State for ReadEthBalanceState {
    fn meta(&self) -> StateMeta {
        StateMeta {
            tags: vec![Tag("fetch_data".to_string())],
            depends_on: Vec::new(),
            depends_on_strategy: DependencyStrategy::Latest,
            side_effects: SideEffectKind::ReadOnlyIo,
            idempotency: Idempotency::None,
        }
    }

    async fn handle(
        &self,
        ctx: &mut dyn DynContext,
        io: &mut dyn IoProvider,
        _rec: &mut dyn EventRecorder,
    ) -> Result<StateOutcome, StateError> {
        let block = ctx
            .read(&ctx_key(KEY_BLOCK_NUMBER))
            .map_err(|_| state_err("ctx_read_failed", "context read failed"))?
            .and_then(|v| v.as_u64())
            .ok_or_else(|| state_err("missing_block_number", "missing block_number in context"))?;

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

        ctx.write(ctx_key(KEY_NATIVE), native)
            .map_err(|_| state_err("ctx_write_failed", "context write failed"))?;

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
        StateMeta {
            tags: vec![Tag("fetch_data".to_string())],
            depends_on: Vec::new(),
            depends_on_strategy: DependencyStrategy::Latest,
            side_effects: SideEffectKind::ReadOnlyIo,
            idempotency: Idempotency::None,
        }
    }

    async fn handle(
        &self,
        ctx: &mut dyn DynContext,
        io: &mut dyn IoProvider,
        _rec: &mut dyn EventRecorder,
    ) -> Result<StateOutcome, StateError> {
        let block = ctx
            .read(&ctx_key(KEY_BLOCK_NUMBER))
            .map_err(|_| state_err("ctx_read_failed", "context read failed"))?
            .and_then(|v| v.as_u64())
            .ok_or_else(|| state_err("missing_block_number", "missing block_number in context"))?;

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

        ctx.write(token_ctx_key, token_obj)
            .map_err(|_| state_err("ctx_write_failed", "context write failed"))?;

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
        StateMeta {
            tags: Vec::new(),
            depends_on: Vec::new(),
            depends_on_strategy: DependencyStrategy::Latest,
            side_effects: SideEffectKind::Pure,
            idempotency: Idempotency::None,
        }
    }

    async fn handle(
        &self,
        ctx: &mut dyn DynContext,
        io: &mut dyn IoProvider,
        rec: &mut dyn EventRecorder,
    ) -> Result<StateOutcome, StateError> {
        let chain_id = ctx
            .read(&ctx_key(KEY_CHAIN_ID))
            .map_err(|_| state_err("ctx_read_failed", "context read failed"))?
            .and_then(|v| v.as_u64())
            .ok_or_else(|| state_err("missing_chain_id", "missing chain_id in context"))?;

        let block_number = ctx
            .read(&ctx_key(KEY_BLOCK_NUMBER))
            .map_err(|_| state_err("ctx_read_failed", "context read failed"))?
            .and_then(|v| v.as_u64())
            .ok_or_else(|| state_err("missing_block_number", "missing block_number in context"))?;

        let native = ctx
            .read(&ctx_key(KEY_NATIVE))
            .map_err(|_| state_err("ctx_read_failed", "context read failed"))?
            .ok_or_else(|| state_err("missing_native", "missing native balance in context"))?;

        let mut tokens = Vec::with_capacity(self.cfg.tokens.len());
        for t in &self.cfg.tokens {
            let addr_no0x = address_hex_lower_no0x(&t.address);
            let k = ctx_key_erc20_token(&addr_no0x);
            let tok = ctx
                .read(&k)
                .map_err(|_| state_err("ctx_read_failed", "context read failed"))?
                .ok_or_else(|| state_err("missing_token", "missing token balance in context"))?;
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

        let key = output_fact_key(&self.op_path);
        let existed = io
            .get_recorded_fact(&key)
            .await
            .map_err(|_| state_err("io_fact_lookup_failed", "failed to lookup recorded fact"))?
            .is_some();

        let res = io
            .call(IoCall {
                namespace: "portfolio.output".to_string(),
                request: snapshot,
                fact_key: Some(key),
            })
            .await
            .map_err(|_| state_err("output_io_failed", "output call failed"))?;

        let Some(payload_id) = res.recorded_payload_id else {
            return Err(state_err(
                "missing_output_payload_id",
                "expected recorded payload id for output",
            ));
        };

        ctx.write(
            ctx_key(KEY_SNAPSHOT_ARTIFACT_ID),
            serde_json::json!(payload_id.0.clone()),
        )
        .map_err(|_| state_err("ctx_write_failed", "context write failed"))?;

        if !existed {
            let payload = serde_json::to_value(ArtifactWritten {
                artifact_id: payload_id,
                kind: ArtifactKind::Output,
                meta: serde_json::json!({}),
            })
            .map_err(|_| {
                state_err(
                    "artifact_written_serialize_failed",
                    "failed to serialize ArtifactWritten",
                )
            })?;

            rec.emit(DomainEvent {
                name: DOMAIN_EVENT_ARTIFACT_WRITTEN.to_string(),
                payload,
                payload_ref: None,
            })
            .await
            .map_err(|_| state_err("emit_failed", "failed to emit artifact_written"))?;
        }

        Ok(StateOutcome {
            snapshot: SnapshotPolicy::OnSuccess,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::HashMap;

    use mfm_machine::config::{
        BackoffPolicy, BuildProvenance, ContextCheckpointing, EventProfile, ExecutionMode, IoMode,
        RetryPolicy,
    };
    use mfm_machine::context::DynContext;
    use mfm_machine::engine::{ExecutionEngine, RunPhase, Stores};
    use mfm_machine::errors::ContextError;
    use mfm_machine::errors::{ErrorCategory, StorageError};
    use mfm_machine::events::{Event, EventEnvelope, KernelEvent};
    use mfm_machine::hashing::artifact_id_for_bytes;
    use mfm_machine::ids::{ArtifactId, RunId};
    use mfm_machine::live_io::{LiveIoTransport, LiveIoTransportFactory};
    use mfm_machine::live_io_router::RouterLiveIoTransportFactory;
    use mfm_machine::runtime::DefaultExecutionEngine;
    use mfm_machine::stores::{ArtifactKind, ArtifactStore, EventStore};
    use mfm_sdk::launcher::{LaunchPipeline, RunLauncher};
    use mfm_sdk::pipeline::PipelinePlanner;
    use mfm_sdk::unstable::{
        single_op_pipeline, DefaultPipelinePlanner, DefaultRunLauncher, HashMapOperationRegistry,
        SdkPlanResolver,
    };
    use tokio::sync::Mutex;

    fn run_config_live() -> RunConfig {
        RunConfig {
            io_mode: IoMode::Live,
            retry_policy: RetryPolicy {
                max_attempts: 1,
                backoff: BackoffPolicy::Fixed {
                    delay: std::time::Duration::from_millis(0),
                },
            },
            event_profile: EventProfile::Normal,
            execution_mode: ExecutionMode::Sequential,
            context_checkpointing: ContextCheckpointing::AfterEveryState,
            replay_missing_fact_retryable: false,
            skip_tags: Vec::new(),
            nix_flake_allowlist: mfm_machine::config::default_nix_flake_allowlist(),
        }
    }

    fn storage_info(code: &'static str, message: &'static str) -> ErrorInfo {
        ErrorInfo {
            code: ErrorCode(code.to_string()),
            category: ErrorCategory::Storage,
            retryable: false,
            message: message.to_string(),
            details: None,
        }
    }

    #[derive(Clone, Default)]
    struct MemEventStoreImpl {
        inner: Arc<Mutex<HashMap<RunId, Vec<EventEnvelope>>>>,
    }

    #[async_trait]
    impl EventStore for MemEventStoreImpl {
        async fn head_seq(&self, run_id: RunId) -> Result<u64, StorageError> {
            let inner = self.inner.lock().await;
            Ok(inner
                .get(&run_id)
                .and_then(|v| v.last())
                .map(|e| e.seq)
                .unwrap_or(0))
        }

        async fn append(
            &self,
            run_id: RunId,
            expected_seq: u64,
            events: Vec<EventEnvelope>,
        ) -> Result<u64, StorageError> {
            let mut inner = self.inner.lock().await;
            let stream = inner.entry(run_id).or_default();
            let head = stream.last().map(|e| e.seq).unwrap_or(0);
            if head != expected_seq {
                return Err(StorageError::Concurrency(storage_info(
                    "event_store_concurrency",
                    "head seq did not match expected seq",
                )));
            }

            stream.extend(events);
            Ok(stream.last().map(|e| e.seq).unwrap_or(head))
        }

        async fn read_range(
            &self,
            run_id: RunId,
            from_seq: u64,
            to_seq: Option<u64>,
        ) -> Result<Vec<EventEnvelope>, StorageError> {
            let inner = self.inner.lock().await;
            let Some(stream) = inner.get(&run_id) else {
                return Ok(Vec::new());
            };

            let from = from_seq.max(1);
            let to = to_seq.unwrap_or(u64::MAX);
            Ok(stream
                .iter()
                .filter(|e| e.seq >= from && e.seq <= to)
                .cloned()
                .collect())
        }
    }

    #[derive(Clone, Default)]
    struct MemArtifactStoreImpl {
        inner: Arc<Mutex<HashMap<ArtifactId, Vec<u8>>>>,
    }

    #[async_trait]
    impl ArtifactStore for MemArtifactStoreImpl {
        async fn put(
            &self,
            _kind: ArtifactKind,
            bytes: Vec<u8>,
        ) -> Result<ArtifactId, StorageError> {
            let id = artifact_id_for_bytes(&bytes);
            self.inner.lock().await.insert(id.clone(), bytes);
            Ok(id)
        }

        async fn get(&self, id: &ArtifactId) -> Result<Vec<u8>, StorageError> {
            let inner = self.inner.lock().await;
            inner.get(id).cloned().ok_or_else(|| {
                StorageError::NotFound(storage_info("not_found", "artifact not found"))
            })
        }

        async fn exists(&self, id: &ArtifactId) -> Result<bool, StorageError> {
            Ok(self.inner.lock().await.contains_key(id))
        }
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
            let mut m = serde_json::Map::new();
            for (k, v) in &self.inner {
                m.insert(k.clone(), v.clone());
            }
            Ok(serde_json::Value::Object(m))
        }
    }

    #[derive(Clone)]
    struct MockEvmTransportFactory;

    impl LiveIoTransportFactory for MockEvmTransportFactory {
        fn make(&self, _env: mfm_machine::live_io::LiveIoEnv) -> Box<dyn LiveIoTransport> {
            Box::new(MockEvmTransport)
        }
    }

    struct MockEvmTransport;

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
                "eth_chainId" => Ok(serde_json::json!("0x1")),
                "eth_blockNumber" => Ok(serde_json::json!("0x64")),
                "eth_getBalance" => Ok(serde_json::json!("0xde0b6b3a7640000")), // 1 ETH
                "eth_call" => Ok(ok_u256_as_32byte_hex(1_000_000)), // 1 token @ 6 decimals
                _ => Err(IoError::Other(info(
                    "unknown_method",
                    ErrorCategory::Unknown,
                    false,
                    "unknown jsonrpc method",
                ))),
            }
        }
    }

    fn run_completed_snapshot_id(stream: &[EventEnvelope]) -> Option<ArtifactId> {
        for e in stream {
            if let Event::Kernel(KernelEvent::RunCompleted {
                final_snapshot_id, ..
            }) = &e.event
            {
                return final_snapshot_id.clone();
            }
        }
        None
    }

    #[tokio::test]
    async fn snapshot_writes_output_artifact() {
        let op: mfm_sdk::op::DynOperation = Arc::new(PortfolioTrackerOp);
        let mut reg = HashMapOperationRegistry::default();
        reg.register(Arc::clone(&op));
        let registry: Arc<dyn mfm_sdk::op::OperationRegistry> = Arc::new(reg);

        let planner: Arc<dyn PipelinePlanner> = Arc::new(DefaultPipelinePlanner);
        let pipeline = single_op_pipeline(
            op.op_id(),
            op.op_version(),
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
        )
        .expect("pipeline");

        let resolver = Arc::new(SdkPlanResolver::new(
            Arc::clone(&registry),
            Arc::clone(&planner),
        ));

        let mut routes: HashMap<String, Arc<dyn LiveIoTransportFactory>> = HashMap::new();
        routes.insert("evm".to_string(), Arc::new(MockEvmTransportFactory));
        routes.insert("portfolio".to_string(), Arc::new(EchoTransportFactory));
        let factory: Arc<dyn LiveIoTransportFactory> =
            Arc::new(RouterLiveIoTransportFactory::new(routes));

        let engine =
            DefaultExecutionEngine::new(resolver).with_live_transport_factory(Arc::clone(&factory));
        let engine: Arc<dyn ExecutionEngine> = Arc::new(engine);
        let stores = Stores {
            events: Arc::new(MemEventStoreImpl::default()),
            artifacts: Arc::new(MemArtifactStoreImpl::default()),
        };

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
                    pipeline: pipeline.clone(),
                    input: serde_json::json!({}),
                    run_config: run_config_live(),
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
            .expect("run");

        assert_eq!(res.phase, RunPhase::Completed);

        let stream = stores
            .events
            .read_range(res.run_id, 1, None)
            .await
            .expect("events");
        let final_snapshot_id = run_completed_snapshot_id(&stream).expect("final snapshot id");

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
