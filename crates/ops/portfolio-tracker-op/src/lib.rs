#![cfg_attr(test, allow(clippy::disallowed_methods, clippy::disallowed_types))]
#![cfg_attr(not(test), deny(clippy::disallowed_methods, clippy::disallowed_types))]
#![warn(missing_docs)]
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
//!
//! # Examples
//!
//! ```rust
//! use mfm_op_portfolio_tracker::PortfolioTrackerOp;
//! use mfm_sdk::op::Operation;
//!
//! let op = PortfolioTrackerOp;
//! assert_eq!(op.op_id().as_str(), "portfolio_tracker");
//! ```

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
use mfm_machine::ids::{ContextKey, FactKey, OpId, OpPath, StateId};
use mfm_machine::io::IoProvider;
use mfm_machine::meta::StateMeta;
use mfm_machine::plan::{DependencyEdge, StateGraph, StateNode};
use mfm_machine::recorder::EventRecorder;
use mfm_machine::state::{SnapshotPolicy, State, StateOutcome};
use mfm_sdk::errors::SdkError;
use mfm_sdk::ids::PortKey;
use mfm_sdk::op::{OpIo, Operation};
use mfm_state_common::ctx as op_ctx;
use mfm_state_common::errors as op_errors;
use mfm_state_common::local_io_helpers::emit_report_event;
use mfm_state_common::output as op_output;
use mfm_state_common::states::meta;
use mfm_state_keystore::tx::output_context_key;

const OP_ID: &str = "portfolio_tracker";
const OP_VERSION: &str = "v1";

const KEY_CHAIN_ID: &str = "chain_id";
const KEY_BLOCK_NUMBER: &str = "block_number";
const KEY_NATIVE: &str = "native";
const KEY_SNAPSHOT_ARTIFACT_ID: &str = "snapshot_artifact_id";
const KEY_REPORT: &str = "report";

fn ctx_key(suffix: &'static str) -> ContextKey {
    ContextKey(suffix.to_string())
}

fn default_chain_id() -> u64 {
    1
}

/// Rendered balance for one asset in the portfolio snapshot report.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PortfolioBalanceReport {
    /// Human-readable asset symbol.
    pub symbol: String,
    /// Raw integer balance rendered as a decimal string.
    pub raw_u256_dec: String,
    /// Token decimals used to interpret the raw balance.
    pub decimals: u8,
    /// Decimal-formatted amount using `decimals`.
    pub amount_dec: String,
}

/// Summary report written to context after the snapshot artifact is produced.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PortfolioTrackerReport {
    /// Content-addressed identifier of the snapshot output artifact.
    pub snapshot_artifact_id: String,
    /// Chain id used for the snapshot.
    pub chain_id: u64,
    /// Block number pinned for all balance reads.
    pub block_number: u64,
    #[serde(default)]
    /// Native balance summary, when present.
    pub native_balance: Option<PortfolioBalanceReport>,
}

/// Returns the context key that stores the final portfolio tracker report.
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

/// Thin planner op that expands the portfolio snapshot workflow into reusable read/write states.
#[derive(Clone, Default)]
pub struct PortfolioTrackerOp;

impl Operation for PortfolioTrackerOp {
    fn op_id(&self) -> OpId {
        OpId::must_new(OP_ID.to_string())
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
        let chain_id_sid = StateId::must_new(format!("{}.chain_id", op_path.0));
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
        let block_sid = StateId::must_new(format!("{}.block_number", op_path.0));
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
        let eth_sid = StateId::must_new(format!("{}.eth_balance", op_path.0));
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
            let sid = StateId::must_new(format!("{}.token_balance_{}", op_path.0, addr_no0x));
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
        let out_sid = StateId::must_new(format!("{}.write_snapshot", op_path.0));
        edges.push(DependencyEdge {
            from: last,
            to: out_sid.clone(),
        });
        states.push(StateNode {
            id: out_sid.clone(),
            state: Arc::new(WriteSnapshotState {
                op_path: op_path.clone(),
                cfg,
            }),
        });

        // write summary report
        let report_sid = StateId::must_new(format!("{}.report", op_path.0));
        edges.push(DependencyEdge {
            from: out_sid,
            to: report_sid.clone(),
        });
        states.push(StateNode {
            id: report_sid,
            state: Arc::new(WriteReportState),
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

        Ok(StateOutcome {
            snapshot: SnapshotPolicy::OnSuccess,
        })
    }
}

struct WriteReportState;

#[async_trait]
impl State for WriteReportState {
    fn meta(&self) -> StateMeta {
        meta::pure()
    }

    async fn handle(
        &self,
        ctx: &mut dyn DynContext,
        _io: &mut dyn IoProvider,
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
        let snapshot_artifact_id = op_ctx::read_string_required(
            ctx,
            &ctx_key(KEY_SNAPSHOT_ARTIFACT_ID),
            "missing_snapshot_artifact_id",
            "missing snapshot artifact id in context",
            "snapshot_artifact_id_not_string",
            "snapshot artifact id in context must be a string",
        )?;

        let native_balance: Option<PortfolioBalanceReport> = op_ctx::read_json_required(
            ctx,
            &ctx_key(KEY_NATIVE),
            "missing_native",
            "missing native balance in context",
        )
        .and_then(|native| {
            serde_json::from_value(native).map_err(|_| {
                op_errors::state_unknown(
                    "native_balance_decode_failed",
                    "failed to decode native balance report",
                )
            })
        })
        .map(Some)?;

        let report = PortfolioTrackerReport {
            snapshot_artifact_id,
            chain_id,
            block_number,
            native_balance,
        };

        let report_json = serde_json::to_value(&report).map_err(|_| {
            op_errors::state_unknown(
                "serialize_report_failed",
                "failed to serialize snapshot report",
            )
        })?;
        op_ctx::write_json(ctx, ctx_key(KEY_REPORT), report_json.clone())?;
        emit_report_event(rec, "portfolio_tracker.completed", report_json).await?;

        Ok(StateOutcome {
            snapshot: SnapshotPolicy::OnSuccess,
        })
    }
}

#[cfg(test)]
#[path = "tests/portfolio_tracker_op_tests.rs"]
mod portfolio_tracker_op_tests;
