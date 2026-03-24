#![cfg_attr(test, allow(clippy::disallowed_methods, clippy::disallowed_types))]
#![cfg_attr(not(test), deny(clippy::disallowed_methods, clippy::disallowed_types))]
#![warn(missing_docs)]
//! Canonical portfolio snapshot planner op.
//!
//! Source of truth:
//! - `docs/redesign.md`
//! - `docs/ops-and-states.md`
//!
//! `portfolio_tracker` remains a thin planner that validates canonical config inputs and wires the
//! reusable shared-state runtime for multi-network portfolio execution.
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

use std::collections::HashSet;
use std::sync::Arc;

use mfm_evm_runtime::states::rpc_control::{PrepareSourcesState, RpcControlNetworkRoute};
use mfm_machine::config::RunConfig;
use mfm_machine::errors::ErrorCategory;
use mfm_machine::ids::{ContextKey, FactKey, OpId, OpPath};
use mfm_machine::plan::DependencyEdge;
use mfm_sdk::errors::SdkError;
use mfm_sdk::ids::PortKey;
use mfm_sdk::op::{
    leaf_state_id, leaf_state_node, LeafOpSpec, OpInterface, Operation, PlannedOp, PlannedOpKind,
};
use mfm_state_aave_v3::portfolio::model::{
    is_aave_protocol_position, validate_aave_portfolio_config,
};
use mfm_state_aave_v3::portfolio::states::CollectAaveObservationsState;
use mfm_state_common::errors as op_errors;
use mfm_state_portfolio::model::{
    decode_portfolio_config, validate_portfolio_bundle, PortfolioConfig,
};
use mfm_state_portfolio::states::{
    PinPortfolioNetworksState, WritePortfolioReportState, WritePortfolioSnapshotState,
};
use mfm_state_symbol::model::{decode_valuation_source_registry, ValuationSourceRegistry};
use mfm_state_symbol::states::{
    CollectObservationsState, MergeObservationsState, NetworkRouteConfig, ReadDirectPricesState,
};
use mfm_state_wallet::model::WalletConfig;
use mfm_state_wallet::states::ResolveWalletsState;
use serde_json::Value;

const OP_ID: &str = "portfolio_tracker";
const OP_VERSION: &str = "v1";
const MAIN_OP_PATH: &str = "portfolio_tracker.main";

const KEY_RESOLVED_WALLETS: &str = "resolved_wallets";
const KEY_NETWORK_PINS: &str = "network_pins";
const KEY_DIRECT_PRICES: &str = "direct_prices";
const KEY_BASE_OBSERVATIONS: &str = "base_observations";
const KEY_AAVE_OBSERVATIONS: &str = "aave_observations";
const KEY_OBSERVATIONS: &str = "observations";
const KEY_SNAPSHOT: &str = "snapshot";
const KEY_SNAPSHOT_ARTIFACT_ID: &str = "snapshot_artifact_id";
const KEY_REPORT: &str = "report";

fn ctx_key(suffix: &'static str) -> ContextKey {
    ContextKey(suffix.to_string())
}

/// Returns the context key that stores the canonical portfolio snapshot JSON.
pub fn portfolio_snapshot_context_key() -> ContextKey {
    ContextKey(format!("{MAIN_OP_PATH}.{KEY_SNAPSHOT}"))
}

/// Returns the context key that stores the canonical portfolio snapshot artifact id.
pub fn portfolio_snapshot_artifact_id_context_key() -> ContextKey {
    ContextKey(format!("{MAIN_OP_PATH}.{KEY_SNAPSHOT_ARTIFACT_ID}"))
}

/// Returns the context key that stores the canonical portfolio report JSON.
pub fn portfolio_snapshot_report_context_key() -> ContextKey {
    ContextKey(format!("{MAIN_OP_PATH}.{KEY_REPORT}"))
}

#[derive(Clone, Debug)]
struct PortfolioTrackerConfig {
    portfolio: PortfolioConfig,
    valuation_source_registry: ValuationSourceRegistry,
}

fn sdk_input_error(code: &'static str, message: impl Into<String>) -> SdkError {
    op_errors::sdk_error(code, ErrorCategory::ParsingInput, false, message)
}

fn parse_config(op_config: &Value) -> Result<PortfolioTrackerConfig, SdkError> {
    let obj = op_config.as_object().ok_or_else(|| {
        op_errors::sdk_parse_error(
            "invalid_op_config",
            "portfolio_tracker op_config must be a JSON object",
        )
    })?;

    let portfolio_value = obj.get("portfolio").ok_or_else(|| {
        sdk_input_error(
            "missing_portfolio_config",
            "portfolio_tracker op_config must contain `portfolio`",
        )
    })?;
    let valuation_source_registry_value =
        obj.get("valuation_source_registry").ok_or_else(|| {
            sdk_input_error(
                "missing_valuation_source_registry",
                "portfolio_tracker op_config must contain `valuation_source_registry`",
            )
        })?;

    let portfolio = decode_portfolio_config(portfolio_value)
        .map_err(|err| sdk_input_error("invalid_portfolio_config", err.to_string()))?;
    let valuation_source_registry =
        decode_valuation_source_registry(valuation_source_registry_value)
            .map_err(|err| sdk_input_error("invalid_valuation_source_registry", err.to_string()))?;
    validate_portfolio_bundle(&portfolio, &valuation_source_registry)
        .map_err(|err| sdk_input_error("invalid_portfolio_bundle", err.to_string()))?;
    validate_aave_portfolio_config(&portfolio)
        .map_err(|err| sdk_input_error("invalid_aave_portfolio_config", err.to_string()))?;

    Ok(PortfolioTrackerConfig {
        portfolio,
        valuation_source_registry,
    })
}

fn output_fact_key(op_path: &OpPath) -> FactKey {
    FactKey(format!("portfolio:output|op:{}", op_path.0))
}

fn network_routes(portfolio: &PortfolioConfig) -> Vec<NetworkRouteConfig> {
    portfolio
        .networks
        .iter()
        .map(|network| NetworkRouteConfig {
            network_id: network.network_id.clone(),
            control_scope: network.control_scope.clone(),
        })
        .collect()
}

fn split_symbols(
    portfolio: &PortfolioConfig,
) -> (
    Vec<mfm_state_symbol::model::SymbolConfig>,
    Vec<mfm_state_symbol::model::SymbolConfig>,
) {
    let mut base_symbols = Vec::new();
    let mut aave_symbols = Vec::new();
    for symbol in &portfolio.symbol_configs {
        if is_aave_protocol_position(symbol) {
            aave_symbols.push(symbol.clone());
        } else {
            base_symbols.push(symbol.clone());
        }
    }
    (base_symbols, aave_symbols)
}

fn filter_wallets_for_symbol_ids(
    wallets: &[WalletConfig],
    allowed_symbol_ids: &HashSet<String>,
) -> Vec<WalletConfig> {
    wallets
        .iter()
        .filter_map(|wallet| {
            let mut wallet = wallet.clone();
            wallet
                .symbol_ids
                .retain(|symbol_id| allowed_symbol_ids.contains(symbol_id));
            (!wallet.symbol_ids.is_empty()).then_some(wallet)
        })
        .collect()
}

/// Thin planner op that validates canonical portfolio inputs and wires the shared runtime states.
#[derive(Clone, Default)]
pub struct PortfolioTrackerOp;

impl Operation for PortfolioTrackerOp {
    fn op_id(&self) -> OpId {
        OpId::must_new(OP_ID.to_string())
    }

    fn op_version(&self) -> String {
        OP_VERSION.to_string()
    }

    fn expand(
        &self,
        op_path: OpPath,
        op_config: &Value,
        _run_config: &RunConfig,
    ) -> Result<PlannedOp, SdkError> {
        let cfg = parse_config(op_config)?;
        let routes = network_routes(&cfg.portfolio);
        let (base_symbols, aave_symbols) = split_symbols(&cfg.portfolio);
        let base_symbol_ids: HashSet<_> = base_symbols
            .iter()
            .map(|symbol| symbol.symbol_id.clone())
            .collect();
        let aave_symbol_ids: HashSet<_> = aave_symbols
            .iter()
            .map(|symbol| symbol.symbol_id.clone())
            .collect();
        let base_wallets = filter_wallets_for_symbol_ids(&cfg.portfolio.wallets, &base_symbol_ids);
        let aave_wallets = filter_wallets_for_symbol_ids(&cfg.portfolio.wallets, &aave_symbol_ids);

        let mut states = Vec::new();
        let mut edges = Vec::new();

        let rpc_sources = cfg
            .portfolio
            .networks
            .iter()
            .map(|network| RpcControlNetworkRoute {
                network_id: network.network_id.clone(),
                control_scope: network.control_scope.clone(),
            })
            .collect();

        let prepare_sources_sid = leaf_state_id(&op_path, "prepare_sources")?;
        states.push(leaf_state_node(
            &op_path,
            "prepare_sources",
            Arc::new(PrepareSourcesState {
                state_id: prepare_sources_sid.clone(),
                networks: rpc_sources,
            }),
        )?);

        let pin_networks_sid = leaf_state_id(&op_path, "pin_networks")?;
        edges.push(DependencyEdge {
            from: prepare_sources_sid,
            to: pin_networks_sid.clone(),
        });
        states.push(leaf_state_node(
            &op_path,
            "pin_networks",
            Arc::new(PinPortfolioNetworksState {
                state_id: pin_networks_sid.clone(),
                portfolio: cfg.portfolio.clone(),
                valuation_sources: cfg.valuation_source_registry.clone(),
                output_key: ctx_key(KEY_NETWORK_PINS),
            }),
        )?);

        let resolve_wallets_sid = leaf_state_id(&op_path, "resolve_wallets")?;
        edges.push(DependencyEdge {
            from: pin_networks_sid.clone(),
            to: resolve_wallets_sid.clone(),
        });
        states.push(leaf_state_node(
            &op_path,
            "resolve_wallets",
            Arc::new(ResolveWalletsState {
                state_id: resolve_wallets_sid.clone(),
                wallets: cfg.portfolio.wallets.clone(),
                output_key: ctx_key(KEY_RESOLVED_WALLETS),
            }),
        )?);

        let read_direct_prices_sid = leaf_state_id(&op_path, "read_direct_prices")?;
        edges.push(DependencyEdge {
            from: pin_networks_sid,
            to: read_direct_prices_sid.clone(),
        });
        states.push(leaf_state_node(
            &op_path,
            "read_direct_prices",
            Arc::new(ReadDirectPricesState {
                state_id: read_direct_prices_sid.clone(),
                symbols: cfg.portfolio.symbol_configs.clone(),
                valuation_sources: cfg.valuation_source_registry.clone(),
                networks: routes.clone(),
                network_pins_key: ctx_key(KEY_NETWORK_PINS),
                output_key: ctx_key(KEY_DIRECT_PRICES),
            }),
        )?);

        let collect_observations_sid = leaf_state_id(&op_path, "collect_observations")?;
        edges.push(DependencyEdge {
            from: resolve_wallets_sid.clone(),
            to: collect_observations_sid.clone(),
        });
        edges.push(DependencyEdge {
            from: read_direct_prices_sid.clone(),
            to: collect_observations_sid.clone(),
        });
        states.push(leaf_state_node(
            &op_path,
            "collect_observations",
            Arc::new(CollectObservationsState {
                state_id: collect_observations_sid.clone(),
                wallets: base_wallets,
                symbols: base_symbols,
                networks: routes.clone(),
                resolved_wallets_key: ctx_key(KEY_RESOLVED_WALLETS),
                network_pins_key: ctx_key(KEY_NETWORK_PINS),
                direct_prices_key: ctx_key(KEY_DIRECT_PRICES),
                output_key: ctx_key(KEY_BASE_OBSERVATIONS),
            }),
        )?);

        let collect_aave_observations_sid = leaf_state_id(&op_path, "collect_aave_observations")?;
        edges.push(DependencyEdge {
            from: resolve_wallets_sid.clone(),
            to: collect_aave_observations_sid.clone(),
        });
        edges.push(DependencyEdge {
            from: read_direct_prices_sid.clone(),
            to: collect_aave_observations_sid.clone(),
        });
        states.push(leaf_state_node(
            &op_path,
            "collect_aave_observations",
            Arc::new(CollectAaveObservationsState {
                state_id: collect_aave_observations_sid.clone(),
                wallets: aave_wallets,
                symbols: aave_symbols,
                networks: routes.clone(),
                resolved_wallets_key: ctx_key(KEY_RESOLVED_WALLETS),
                network_pins_key: ctx_key(KEY_NETWORK_PINS),
                direct_prices_key: ctx_key(KEY_DIRECT_PRICES),
                output_key: ctx_key(KEY_AAVE_OBSERVATIONS),
            }),
        )?);

        let merge_observations_sid = leaf_state_id(&op_path, "merge_observations")?;
        edges.push(DependencyEdge {
            from: collect_observations_sid,
            to: merge_observations_sid.clone(),
        });
        edges.push(DependencyEdge {
            from: collect_aave_observations_sid,
            to: merge_observations_sid.clone(),
        });
        states.push(leaf_state_node(
            &op_path,
            "merge_observations",
            Arc::new(MergeObservationsState {
                state_id: merge_observations_sid.clone(),
                input_keys: vec![
                    ctx_key(KEY_BASE_OBSERVATIONS),
                    ctx_key(KEY_AAVE_OBSERVATIONS),
                ],
                output_key: ctx_key(KEY_OBSERVATIONS),
            }),
        )?);

        let write_snapshot_sid = leaf_state_id(&op_path, "write_snapshot")?;
        edges.push(DependencyEdge {
            from: merge_observations_sid,
            to: write_snapshot_sid.clone(),
        });
        states.push(leaf_state_node(
            &op_path,
            "write_snapshot",
            Arc::new(WritePortfolioSnapshotState {
                state_id: write_snapshot_sid.clone(),
                portfolio: cfg.portfolio,
                resolved_wallets_key: ctx_key(KEY_RESOLVED_WALLETS),
                network_pins_key: ctx_key(KEY_NETWORK_PINS),
                observations_key: ctx_key(KEY_OBSERVATIONS),
                fact_key: output_fact_key(&op_path),
                artifact_id_output_key: ctx_key(KEY_SNAPSHOT_ARTIFACT_ID),
                snapshot_output_key: ctx_key(KEY_SNAPSHOT),
            }),
        )?);

        let write_report_sid = leaf_state_id(&op_path, "write_report")?;
        edges.push(DependencyEdge {
            from: write_snapshot_sid,
            to: write_report_sid.clone(),
        });
        states.push(leaf_state_node(
            &op_path,
            "write_report",
            Arc::new(WritePortfolioReportState {
                state_id: write_report_sid,
                snapshot_key: ctx_key(KEY_SNAPSHOT),
                output_key: ctx_key(KEY_REPORT),
                event_name: "portfolio_tracker.completed",
            }),
        )?);

        Ok(PlannedOp {
            interface: OpInterface {
                imports: Vec::new(),
                exports: vec![
                    PortKey(KEY_SNAPSHOT_ARTIFACT_ID.to_string()),
                    PortKey(KEY_REPORT.to_string()),
                ],
            },
            kind: PlannedOpKind::Leaf(LeafOpSpec { states, edges }),
        })
    }
}

#[cfg(test)]
#[path = "tests/portfolio_tracker_op_tests.rs"]
mod portfolio_tracker_op_tests;
