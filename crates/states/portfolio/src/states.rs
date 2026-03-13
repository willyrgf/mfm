use std::collections::{BTreeMap, HashMap};

use async_trait::async_trait;
use mfm_collectors_evm::{parse_u64_hex_value, EvmIoClient, JsonRpcCall};
use mfm_machine::context::DynContext;
use mfm_machine::errors::{ErrorCategory, StateError};
use mfm_machine::ids::{ContextKey, FactKey, StateId};
use mfm_machine::io::IoProvider;
use mfm_machine::meta::StateMeta;
use mfm_machine::recorder::EventRecorder;
use mfm_machine::state::{SnapshotPolicy, State, StateOutcome};
use mfm_state_common::ctx::{read_typed, write_json};
use mfm_state_common::errors::{
    state_error_with_state, state_from_io, state_unknown, state_unknown_msg,
};
use mfm_state_common::local_io_helpers::emit_report_event;
use mfm_state_common::output::write_output_artifact;
use mfm_state_common::states::meta;
use mfm_state_wallet::model::ResolvedWallet;

use crate::model::{
    validate_portfolio_bundle, NetworkPin, PortfolioConfig, PortfolioReport, PortfolioSnapshot,
    PortfolioSnapshotError, WalletReport, WalletSnapshot,
};

/// Pins each required network to a concrete block for one portfolio execution.
#[derive(Clone, Debug)]
pub struct PinPortfolioNetworksState {
    /// Stable state identifier assigned by the execution plan.
    pub state_id: StateId,
    /// Canonical portfolio config.
    pub portfolio: PortfolioConfig,
    /// Valuation source registry validated alongside the portfolio config.
    pub valuation_sources: mfm_state_symbol::model::ValuationSourceRegistry,
    /// Context key that receives the pinned networks.
    pub output_key: ContextKey,
}

#[async_trait]
impl State for PinPortfolioNetworksState {
    fn meta(&self) -> StateMeta {
        meta::fetch_data()
    }

    async fn handle(
        &self,
        ctx: &mut dyn DynContext,
        io: &mut dyn IoProvider,
        _rec: &mut dyn EventRecorder,
    ) -> Result<StateOutcome, StateError> {
        validate_portfolio_bundle(&self.portfolio, &self.valuation_sources).map_err(|err| {
            state_unknown_msg(
                "invalid_portfolio_runtime_inputs",
                format!("portfolio runtime inputs were invalid: {err}"),
            )
        })?;

        let mut required_networks = BTreeMap::new();
        for wallet in &self.portfolio.wallets {
            required_networks.insert(wallet.network_id.clone(), ());
        }
        for symbol in &self.portfolio.symbol_configs {
            for quote in &symbol.valuation.quotes {
                match &quote.reader {
                    mfm_state_symbol::model::ValuationReaderConfig::FixedUnitPrice { .. } => {}
                    mfm_state_symbol::model::ValuationReaderConfig::DirectPrice { source } => {
                        required_networks.insert(source.network_id.clone(), ());
                    }
                    mfm_state_symbol::model::ValuationReaderConfig::DerivedUnitPrice {
                        numerator,
                        denominator,
                    } => {
                        required_networks.insert(numerator.network_id.clone(), ());
                        required_networks.insert(denominator.network_id.clone(), ());
                    }
                }
            }
        }

        let networks_by_id: HashMap<_, _> = self
            .portfolio
            .networks
            .iter()
            .map(|network| (network.network_id.as_str(), network))
            .collect();
        let mut pins = Vec::with_capacity(required_networks.len());
        for network_id in required_networks.keys() {
            let Some(network) = networks_by_id.get(network_id.as_str()) else {
                return Err(state_unknown_msg(
                    "missing_network_config",
                    format!("network `{network_id}` was not found"),
                ));
            };

            let chain_id = read_u64_rpc(
                &self.state_id,
                io,
                network.rpc_source_id.as_deref(),
                "eth_chainId",
            )
            .await?;
            if chain_id != network.chain_id {
                return Err(state_error_with_state(
                    self.state_id.clone(),
                    "network_chain_id_mismatch",
                    ErrorCategory::ParsingInput,
                    false,
                    format!(
                        "rpc chain_id for network `{}` did not match configured chain_id",
                        network.network_id
                    ),
                ));
            }

            let block_number = read_u64_rpc(
                &self.state_id,
                io,
                network.rpc_source_id.as_deref(),
                "eth_blockNumber",
            )
            .await?;
            pins.push(NetworkPin {
                network_id: network.network_id.clone(),
                chain_id,
                block_number,
            });
        }

        pins.sort_by(|left, right| left.network_id.cmp(&right.network_id));
        let value = serde_json::to_value(&pins).map_err(|_| {
            state_unknown(
                "network_pins_serialize_failed",
                "failed to serialize network pins",
            )
        })?;
        write_json(ctx, self.output_key.clone(), value)?;

        Ok(StateOutcome {
            snapshot: SnapshotPolicy::OnSuccess,
        })
    }
}

/// Writes the canonical portfolio snapshot artifact.
#[derive(Clone, Debug)]
pub struct WritePortfolioSnapshotState {
    /// Stable state identifier assigned by the execution plan.
    pub state_id: StateId,
    /// Canonical portfolio config.
    pub portfolio: PortfolioConfig,
    /// Context key that contains the resolved wallets.
    pub resolved_wallets_key: ContextKey,
    /// Context key that contains the pinned networks.
    pub network_pins_key: ContextKey,
    /// Context key that contains the canonical observations.
    pub observations_key: ContextKey,
    /// Fact key used for the output artifact.
    pub fact_key: FactKey,
    /// Context key that receives the snapshot artifact id.
    pub artifact_id_output_key: ContextKey,
    /// Context key that receives the full snapshot JSON.
    pub snapshot_output_key: ContextKey,
}

#[async_trait]
impl State for WritePortfolioSnapshotState {
    fn meta(&self) -> StateMeta {
        meta::pure()
    }

    async fn handle(
        &self,
        ctx: &mut dyn DynContext,
        io: &mut dyn IoProvider,
        rec: &mut dyn EventRecorder,
    ) -> Result<StateOutcome, StateError> {
        let resolved_wallets: Vec<ResolvedWallet> = read_typed(
            ctx,
            &self.resolved_wallets_key,
            "missing_resolved_wallets",
            "missing resolved wallets in context",
            "resolved_wallets_decode_failed",
            "failed to decode resolved wallets",
        )?;
        let network_pins: Vec<NetworkPin> = read_typed(
            ctx,
            &self.network_pins_key,
            "missing_network_pins",
            "missing network pins in context",
            "network_pins_decode_failed",
            "failed to decode network pins",
        )?;
        let observations: Vec<mfm_state_symbol::model::Observation> = read_typed(
            ctx,
            &self.observations_key,
            "missing_observations",
            "missing observations in context",
            "observations_decode_failed",
            "failed to decode observations",
        )?;
        let observations_by_wallet = group_observations_by_wallet(observations);

        let generated_at_ms = io.now_millis().await.map_err(state_from_io)?;
        let mut wallets = resolved_wallets
            .into_iter()
            .map(|wallet| WalletSnapshot {
                wallet_id: wallet.wallet_id.clone(),
                address: wallet.address,
                network_id: wallet.network_id.clone(),
                observations: observations_by_wallet
                    .get(wallet.wallet_id.as_str())
                    .cloned()
                    .unwrap_or_default(),
            })
            .collect::<Vec<_>>();
        wallets.sort_by(|left, right| left.wallet_id.cmp(&right.wallet_id));

        let mut symbol_configs = self.portfolio.symbol_configs.clone();
        for symbol in &mut symbol_configs {
            symbol.normalize();
        }
        symbol_configs.sort_by(|left, right| left.symbol_id.cmp(&right.symbol_id));

        let mut snapshot = PortfolioSnapshot {
            portfolio_id: self.portfolio.portfolio_id.clone(),
            generated_at_ms,
            network_pins,
            wallets,
            symbol_configs,
            errors: Vec::<PortfolioSnapshotError>::new(),
        };
        snapshot.normalize();

        let snapshot_value = serde_json::to_value(&snapshot).map_err(|_| {
            state_unknown(
                "portfolio_snapshot_serialize_failed",
                "failed to serialize portfolio snapshot",
            )
        })?;
        write_json(
            ctx,
            self.snapshot_output_key.clone(),
            snapshot_value.clone(),
        )?;
        write_output_artifact(
            ctx,
            io,
            rec,
            self.fact_key.clone(),
            snapshot_value,
            self.artifact_id_output_key.clone(),
        )
        .await?;

        Ok(StateOutcome {
            snapshot: SnapshotPolicy::OnSuccess,
        })
    }
}

/// Writes the canonical portfolio report derived from the snapshot artifact.
#[derive(Clone, Debug)]
pub struct WritePortfolioReportState {
    /// Stable state identifier assigned by the execution plan.
    pub state_id: StateId,
    /// Context key that contains the canonical snapshot JSON.
    pub snapshot_key: ContextKey,
    /// Context key that receives the report JSON.
    pub output_key: ContextKey,
    /// Domain event name emitted after the report is written.
    pub event_name: &'static str,
}

#[async_trait]
impl State for WritePortfolioReportState {
    fn meta(&self) -> StateMeta {
        meta::pure()
    }

    async fn handle(
        &self,
        ctx: &mut dyn DynContext,
        _io: &mut dyn IoProvider,
        rec: &mut dyn EventRecorder,
    ) -> Result<StateOutcome, StateError> {
        let snapshot: PortfolioSnapshot = read_typed(
            ctx,
            &self.snapshot_key,
            "missing_snapshot",
            "missing snapshot in context",
            "snapshot_decode_failed",
            "failed to decode portfolio snapshot",
        )?;

        let mut report = PortfolioReport {
            portfolio_id: snapshot.portfolio_id.clone(),
            generated_at_ms: snapshot.generated_at_ms,
            network_pins: snapshot.network_pins.clone(),
            wallet_summaries: snapshot
                .wallets
                .iter()
                .map(|wallet| WalletReport {
                    wallet_id: wallet.wallet_id.clone(),
                    network_id: wallet.network_id.clone(),
                    totals_by_quote: Vec::new(),
                })
                .collect(),
            totals_by_quote: Vec::new(),
            error_count: snapshot.errors.len() as u64,
        };
        report.normalize();

        let report_value = serde_json::to_value(&report).map_err(|_| {
            state_unknown(
                "portfolio_report_serialize_failed",
                "failed to serialize portfolio report",
            )
        })?;
        write_json(ctx, self.output_key.clone(), report_value.clone())?;
        emit_report_event(rec, self.event_name, report_value).await?;

        Ok(StateOutcome {
            snapshot: SnapshotPolicy::OnSuccess,
        })
    }
}

fn group_observations_by_wallet(
    mut observations: Vec<mfm_state_symbol::model::Observation>,
) -> HashMap<String, Vec<mfm_state_symbol::model::Observation>> {
    let mut grouped: HashMap<String, Vec<mfm_state_symbol::model::Observation>> = HashMap::new();
    observations.sort_by(|left, right| {
        (left.wallet_id.as_str(), left.symbol_id.as_str())
            .cmp(&(right.wallet_id.as_str(), right.symbol_id.as_str()))
    });
    for observation in observations {
        grouped
            .entry(observation.wallet_id.clone())
            .or_default()
            .push(observation);
    }
    grouped
}

async fn read_u64_rpc(
    state_id: &StateId,
    io: &mut dyn IoProvider,
    route_source_id: Option<&str>,
    method: &'static str,
) -> Result<u64, StateError> {
    let mut client = EvmIoClient::new(state_id.clone(), io);
    let mut call = JsonRpcCall::new(method, serde_json::json!([]));
    if let Some(route_source_id) = route_source_id {
        call = call.with_route_source_id(route_source_id.to_string());
    }
    let res = client.call(call).await.map_err(state_from_io)?;
    parse_u64_hex_value(&res.response).map_err(|_| {
        state_unknown_msg(
            "evm_response_invalid",
            format!("rpc response for `{method}` was not a hex u64"),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use mfm_machine::context::DynContext;
    use mfm_machine::errors::{ContextError, ErrorInfo, IoError, RunError};
    use mfm_machine::events::DomainEvent;
    use mfm_machine::hashing::artifact_id_for_json;
    use mfm_machine::ids::{ArtifactId, ErrorCode};
    use mfm_machine::io::{IoCall, IoResult};
    use mfm_state_symbol::model::{
        BalanceReaderConfig, QuoteCode, SymbolConfig, SymbolKind, SymbolRole,
        SymbolValuationConfig, ValuationReaderConfig, ValuationSourceConfig,
        ValuationSourceReaderConfig, ValuationSourceRegistry,
    };
    use mfm_state_symbol::states::{
        CollectObservationsState, DirectPriceValue, NetworkRouteConfig, ReadDirectPricesState,
    };
    use mfm_state_wallet::model::{WalletConfig, WalletImplementationConfig};
    use mfm_state_wallet::states::ResolveWalletsState;
    use serde_json::json;
    use serde_json::Value;

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
            for (key, value) in &self.inner {
                out.insert(key.clone(), value.clone());
            }
            Ok(serde_json::Value::Object(out))
        }
    }

    fn info(code: &'static str, message: &'static str) -> ErrorInfo {
        ErrorInfo {
            code: ErrorCode(code.to_string()),
            category: ErrorCategory::Unknown,
            retryable: false,
            message: message.to_string(),
            details: None,
        }
    }

    #[derive(Default)]
    struct MockIo {
        calls: Vec<serde_json::Value>,
        recorded: HashMap<String, serde_json::Value>,
    }

    #[async_trait]
    impl IoProvider for MockIo {
        async fn call(&mut self, call: IoCall) -> Result<IoResult, IoError> {
            self.calls.push(call.request.clone());

            let method = call
                .request
                .get("method")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            let route_source_id = call
                .request
                .get("route")
                .and_then(serde_json::Value::as_object)
                .and_then(|route| route.get("source_id"))
                .and_then(serde_json::Value::as_str);
            let params = call
                .request
                .get("params")
                .and_then(serde_json::Value::as_array)
                .cloned()
                .unwrap_or_default();

            let response = match method {
                "eth_chainId" => match route_source_id {
                    Some("arbitrum_primary") => json!("0xa4b1"),
                    _ => json!("0x1"),
                },
                "eth_blockNumber" => match route_source_id {
                    Some("arbitrum_primary") => json!("0xc8"),
                    _ => json!("0x64"),
                },
                "eth_getBalance" => match params.first().and_then(serde_json::Value::as_str) {
                    Some("0x000000000000000000000000000000000000dead") => {
                        json!("0x0de0b6b3a7640000")
                    }
                    Some("0x000000000000000000000000000000000000beef") => {
                        json!("0x1bc16d674ec80000")
                    }
                    _ => {
                        return Err(IoError::Other(info(
                            "unexpected_balance_call",
                            "unexpected balance call",
                        )))
                    }
                },
                "eth_call" => {
                    let target = params
                        .first()
                        .and_then(serde_json::Value::as_object)
                        .cloned()
                        .unwrap_or_default();
                    let to = target
                        .get("to")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default();
                    let data = target
                        .get("data")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default();

                    match (to, data) {
                        ("0x0000000000000000000000000000000000000001", "0x313ce567") => {
                            json!("0x0000000000000000000000000000000000000000000000000000000000000006")
                        }
                        ("0x0000000000000000000000000000000000000001", d)
                            if d.starts_with("0x70a08231") =>
                        {
                            json!("0x0000000000000000000000000000000000000000000000000000000000989680")
                        }
                        ("0x0000000000000000000000000000000000001001", "0x313ce567")
                        | ("0x0000000000000000000000000000000000001002", "0x313ce567")
                        | ("0x0000000000000000000000000000000000001003", "0x313ce567") => {
                            json!("0x0000000000000000000000000000000000000000000000000000000000000008")
                        }
                        ("0x0000000000000000000000000000000000001001", "0xfeaf968c") => {
                            chainlink_round_data_hex(200_000_000)
                        }
                        ("0x0000000000000000000000000000000000001002", "0xfeaf968c") => {
                            chainlink_round_data_hex(20_000_000_000)
                        }
                        ("0x0000000000000000000000000000000000001003", "0xfeaf968c") => {
                            chainlink_round_data_hex(100_000_000)
                        }
                        _ => {
                            return Err(IoError::Other(info(
                                "unexpected_eth_call",
                                "unexpected eth_call payload",
                            )))
                        }
                    }
                }
                _ => {
                    return Err(IoError::Other(info(
                        "unexpected_method",
                        "unexpected method",
                    )))
                }
            };

            Ok(IoResult {
                response,
                recorded_payload_id: None,
            })
        }

        async fn record_value(
            &mut self,
            key: FactKey,
            value: serde_json::Value,
        ) -> Result<ArtifactId, IoError> {
            self.recorded.insert(key.0.clone(), value.clone());
            artifact_id_for_json(&value)
                .map_err(|_| IoError::Other(info("artifact_id_failed", "artifact id failed")))
        }

        async fn get_recorded_fact(
            &mut self,
            key: &FactKey,
        ) -> Result<Option<ArtifactId>, IoError> {
            Ok(self
                .recorded
                .get(&key.0)
                .and_then(|value| artifact_id_for_json(value).ok()))
        }

        async fn now_millis(&mut self) -> Result<u64, IoError> {
            Ok(1234)
        }

        async fn random_bytes(&mut self, n: usize) -> Result<Vec<u8>, IoError> {
            Ok(vec![0; n])
        }
    }

    #[derive(Default)]
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

    #[tokio::test]
    async fn base_runtime_slice_builds_canonical_snapshot_and_report() {
        let portfolio = portfolio_config();
        let registry = valuation_source_registry();
        let mut ctx = MapContext::default();
        let mut io = MockIo::default();
        let mut rec = NoopRecorder;

        ResolveWalletsState {
            state_id: StateId::must_new("wallet.main.resolve".to_string()),
            wallets: portfolio.wallets.clone(),
            output_key: ContextKey("resolved_wallets".to_string()),
        }
        .handle(&mut ctx, &mut io, &mut rec)
        .await
        .expect("resolve wallets");

        PinPortfolioNetworksState {
            state_id: StateId::must_new("portfolio.main.pin_networks".to_string()),
            portfolio: portfolio.clone(),
            valuation_sources: registry.clone(),
            output_key: ContextKey("network_pins".to_string()),
        }
        .handle(&mut ctx, &mut io, &mut rec)
        .await
        .expect("pin networks");

        ReadDirectPricesState {
            state_id: StateId::must_new("symbol.main.direct_prices".to_string()),
            symbols: portfolio.symbol_configs.clone(),
            valuation_sources: registry.clone(),
            networks: network_routes(&portfolio),
            network_pins_key: ContextKey("network_pins".to_string()),
            output_key: ContextKey("direct_prices".to_string()),
        }
        .handle(&mut ctx, &mut io, &mut rec)
        .await
        .expect("read direct prices");

        CollectObservationsState {
            state_id: StateId::must_new("symbol.main.observations".to_string()),
            wallets: portfolio.wallets.clone(),
            symbols: portfolio.symbol_configs.clone(),
            networks: network_routes(&portfolio),
            resolved_wallets_key: ContextKey("resolved_wallets".to_string()),
            network_pins_key: ContextKey("network_pins".to_string()),
            direct_prices_key: ContextKey("direct_prices".to_string()),
            output_key: ContextKey("observations".to_string()),
        }
        .handle(&mut ctx, &mut io, &mut rec)
        .await
        .expect("collect observations");

        WritePortfolioSnapshotState {
            state_id: StateId::must_new("portfolio.main.snapshot".to_string()),
            portfolio: portfolio.clone(),
            resolved_wallets_key: ContextKey("resolved_wallets".to_string()),
            network_pins_key: ContextKey("network_pins".to_string()),
            observations_key: ContextKey("observations".to_string()),
            fact_key: FactKey("portfolio:test:snapshot".to_string()),
            artifact_id_output_key: ContextKey("snapshot_artifact_id".to_string()),
            snapshot_output_key: ContextKey("snapshot".to_string()),
        }
        .handle(&mut ctx, &mut io, &mut rec)
        .await
        .expect("write snapshot");

        WritePortfolioReportState {
            state_id: StateId::must_new("portfolio.main.report".to_string()),
            snapshot_key: ContextKey("snapshot".to_string()),
            output_key: ContextKey("report".to_string()),
            event_name: "portfolio_snapshot.completed",
        }
        .handle(&mut ctx, &mut io, &mut rec)
        .await
        .expect("write report");

        let snapshot: PortfolioSnapshot = serde_json::from_value(
            ctx.read(&ContextKey("snapshot".to_string()))
                .expect("read")
                .expect("snapshot"),
        )
        .expect("typed snapshot");
        assert_eq!(snapshot.network_pins.len(), 2);
        assert_eq!(snapshot.wallets.len(), 2);
        assert_eq!(snapshot.wallets[0].wallet_id, "wallet_ops_arb");
        assert_eq!(snapshot.wallets[1].observations.len(), 2);
        let usdc = snapshot.wallets[1]
            .observations
            .iter()
            .find(|observation| observation.symbol_id == "usdc.wallet.ethereum-mainnet")
            .expect("usdc observation");
        assert_eq!(usdc.values[0].quote, QuoteCode::Btc);
        assert_eq!(usdc.values[0].valuation_reader_kind, "derived_unit_price");
        assert_eq!(usdc.values[1].quote, QuoteCode::Usd);
        assert_eq!(usdc.values[1].valuation_reader_kind, "direct_price");
        assert_eq!(usdc.values[0].source_refs.len(), 2);
        assert!(snapshot
            .wallets
            .iter()
            .flat_map(|wallet| wallet.observations.iter())
            .all(|observation| !observation.values.is_empty()));

        let report: PortfolioReport = serde_json::from_value(
            ctx.read(&ContextKey("report".to_string()))
                .expect("read")
                .expect("report"),
        )
        .expect("typed report");
        assert_eq!(report.portfolio_id, "portfolio_main");
        assert_eq!(report.error_count, 0);
        assert_eq!(report.wallet_summaries.len(), 2);

        let direct_prices: Vec<DirectPriceValue> = serde_json::from_value(
            ctx.read(&ContextKey("direct_prices".to_string()))
                .expect("read")
                .expect("direct prices"),
        )
        .expect("typed direct prices");
        assert_eq!(direct_prices.len(), 3);
    }

    fn chainlink_round_data_hex(answer: u64) -> serde_json::Value {
        let answer_hex = format!("{answer:064x}");
        json!(format!(
            "0x\
0000000000000000000000000000000000000000000000000000000000000001\
{answer_hex}\
0000000000000000000000000000000000000000000000000000000000000001\
0000000000000000000000000000000000000000000000000000000000000001\
0000000000000000000000000000000000000000000000000000000000000001"
        ))
    }

    fn network_routes(portfolio: &PortfolioConfig) -> Vec<NetworkRouteConfig> {
        portfolio
            .networks
            .iter()
            .map(|network| NetworkRouteConfig {
                network_id: network.network_id.clone(),
                rpc_source_id: network.rpc_source_id.clone(),
            })
            .collect()
    }

    fn portfolio_config() -> PortfolioConfig {
        PortfolioConfig {
            portfolio_id: "portfolio_main".to_string(),
            quote_codes: vec![QuoteCode::Usd, QuoteCode::Btc],
            networks: vec![
                crate::model::NetworkConfig {
                    network_id: "ethereum-mainnet".to_string(),
                    chain_id: 1,
                    rpc_source_id: Some("mainnet_primary".to_string()),
                    metadata: BTreeMap::new(),
                },
                crate::model::NetworkConfig {
                    network_id: "arbitrum-mainnet".to_string(),
                    chain_id: 42161,
                    rpc_source_id: Some("arbitrum_primary".to_string()),
                    metadata: BTreeMap::new(),
                },
            ],
            wallets: vec![
                WalletConfig {
                    wallet_id: "wallet_treasury_eth".to_string(),
                    address: "0x000000000000000000000000000000000000dead".to_string(),
                    network_id: "ethereum-mainnet".to_string(),
                    implementation: WalletImplementationConfig::AddressOnly {},
                    symbol_ids: vec![
                        "eth.native.ethereum-mainnet".to_string(),
                        "usdc.wallet.ethereum-mainnet".to_string(),
                    ],
                    metadata: BTreeMap::new(),
                },
                WalletConfig {
                    wallet_id: "wallet_ops_arb".to_string(),
                    address: "0x000000000000000000000000000000000000beef".to_string(),
                    network_id: "arbitrum-mainnet".to_string(),
                    implementation: WalletImplementationConfig::AddressOnly {},
                    symbol_ids: vec!["eth.native.arbitrum-mainnet".to_string()],
                    metadata: BTreeMap::new(),
                },
            ],
            symbol_configs: vec![
                SymbolConfig {
                    symbol_id: "eth.native.ethereum-mainnet".to_string(),
                    display_symbol: Some("ETH".to_string()),
                    kind: SymbolKind::NativeBalance,
                    role: SymbolRole::Native,
                    network_id: "ethereum-mainnet".to_string(),
                    protocol: None,
                    balance_reader: BalanceReaderConfig::NativeBalance {},
                    valuation: SymbolValuationConfig {
                        quotes: vec![
                            mfm_state_symbol::model::QuoteValuationConfig {
                                quote: QuoteCode::Usd,
                                priced_symbol_id: "eth.native.ethereum-mainnet".to_string(),
                                reader: ValuationReaderConfig::DirectPrice {
                                    source: mfm_state_symbol::model::PriceSourceRef {
                                        source_id: "chainlink_eth_usd_mainnet".to_string(),
                                        network_id: "ethereum-mainnet".to_string(),
                                        base_symbol_id: "eth.native.ethereum-mainnet".to_string(),
                                        quote: QuoteCode::Usd,
                                    },
                                },
                            },
                            mfm_state_symbol::model::QuoteValuationConfig {
                                quote: QuoteCode::Btc,
                                priced_symbol_id: "eth.native.ethereum-mainnet".to_string(),
                                reader: ValuationReaderConfig::FixedUnitPrice {
                                    unit_price_dec: "0.01000000".to_string(),
                                },
                            },
                        ],
                    },
                    decimals: Some(18),
                    underlying_symbol_id: None,
                    metadata: BTreeMap::new(),
                },
                SymbolConfig {
                    symbol_id: "usdc.wallet.ethereum-mainnet".to_string(),
                    display_symbol: Some("USDC".to_string()),
                    kind: SymbolKind::Erc20Balance,
                    role: SymbolRole::Asset,
                    network_id: "ethereum-mainnet".to_string(),
                    protocol: None,
                    balance_reader: BalanceReaderConfig::Erc20Balance {
                        token_address: "0x0000000000000000000000000000000000000001".to_string(),
                    },
                    valuation: SymbolValuationConfig {
                        quotes: vec![
                            mfm_state_symbol::model::QuoteValuationConfig {
                                quote: QuoteCode::Usd,
                                priced_symbol_id: "usdc.wallet.ethereum-mainnet".to_string(),
                                reader: ValuationReaderConfig::DirectPrice {
                                    source: mfm_state_symbol::model::PriceSourceRef {
                                        source_id: "chainlink_usdc_usd_mainnet".to_string(),
                                        network_id: "ethereum-mainnet".to_string(),
                                        base_symbol_id: "usdc.wallet.ethereum-mainnet".to_string(),
                                        quote: QuoteCode::Usd,
                                    },
                                },
                            },
                            mfm_state_symbol::model::QuoteValuationConfig {
                                quote: QuoteCode::Btc,
                                priced_symbol_id: "usdc.wallet.ethereum-mainnet".to_string(),
                                reader: ValuationReaderConfig::DerivedUnitPrice {
                                    numerator: mfm_state_symbol::model::PriceSourceRef {
                                        source_id: "chainlink_usdc_usd_mainnet".to_string(),
                                        network_id: "ethereum-mainnet".to_string(),
                                        base_symbol_id: "usdc.wallet.ethereum-mainnet".to_string(),
                                        quote: QuoteCode::Usd,
                                    },
                                    denominator: mfm_state_symbol::model::PriceSourceRef {
                                        source_id: "chainlink_btc_usd_mainnet".to_string(),
                                        network_id: "ethereum-mainnet".to_string(),
                                        base_symbol_id: "btc.wallet.ethereum-mainnet".to_string(),
                                        quote: QuoteCode::Usd,
                                    },
                                },
                            },
                        ],
                    },
                    decimals: Some(6),
                    underlying_symbol_id: None,
                    metadata: BTreeMap::new(),
                },
                SymbolConfig {
                    symbol_id: "eth.native.arbitrum-mainnet".to_string(),
                    display_symbol: Some("ETH".to_string()),
                    kind: SymbolKind::NativeBalance,
                    role: SymbolRole::Native,
                    network_id: "arbitrum-mainnet".to_string(),
                    protocol: None,
                    balance_reader: BalanceReaderConfig::NativeBalance {},
                    valuation: SymbolValuationConfig {
                        quotes: vec![
                            mfm_state_symbol::model::QuoteValuationConfig {
                                quote: QuoteCode::Usd,
                                priced_symbol_id: "eth.native.arbitrum-mainnet".to_string(),
                                reader: ValuationReaderConfig::FixedUnitPrice {
                                    unit_price_dec: "200.00000000".to_string(),
                                },
                            },
                            mfm_state_symbol::model::QuoteValuationConfig {
                                quote: QuoteCode::Btc,
                                priced_symbol_id: "eth.native.arbitrum-mainnet".to_string(),
                                reader: ValuationReaderConfig::FixedUnitPrice {
                                    unit_price_dec: "0.01000000".to_string(),
                                },
                            },
                        ],
                    },
                    decimals: Some(18),
                    underlying_symbol_id: None,
                    metadata: BTreeMap::new(),
                },
            ],
            metadata: BTreeMap::new(),
        }
    }

    fn valuation_source_registry() -> ValuationSourceRegistry {
        ValuationSourceRegistry {
            sources: vec![
                ValuationSourceConfig {
                    source_id: "chainlink_eth_usd_mainnet".to_string(),
                    network_id: "ethereum-mainnet".to_string(),
                    base_symbol_id: "eth.native.ethereum-mainnet".to_string(),
                    quote: QuoteCode::Usd,
                    reader: ValuationSourceReaderConfig::EvmOracle {
                        oracle_kind: "chainlink_aggregator_v3".to_string(),
                        config: BTreeMap::from([(
                            "contract_address".to_string(),
                            Value::String("0x0000000000000000000000000000000000001001".to_string()),
                        )]),
                    },
                    metadata: BTreeMap::new(),
                },
                ValuationSourceConfig {
                    source_id: "chainlink_usdc_usd_mainnet".to_string(),
                    network_id: "ethereum-mainnet".to_string(),
                    base_symbol_id: "usdc.wallet.ethereum-mainnet".to_string(),
                    quote: QuoteCode::Usd,
                    reader: ValuationSourceReaderConfig::EvmOracle {
                        oracle_kind: "chainlink_aggregator_v3".to_string(),
                        config: BTreeMap::from([(
                            "contract_address".to_string(),
                            Value::String("0x0000000000000000000000000000000000001003".to_string()),
                        )]),
                    },
                    metadata: BTreeMap::new(),
                },
                ValuationSourceConfig {
                    source_id: "chainlink_btc_usd_mainnet".to_string(),
                    network_id: "ethereum-mainnet".to_string(),
                    base_symbol_id: "btc.wallet.ethereum-mainnet".to_string(),
                    quote: QuoteCode::Usd,
                    reader: ValuationSourceReaderConfig::EvmOracle {
                        oracle_kind: "chainlink_aggregator_v3".to_string(),
                        config: BTreeMap::from([(
                            "contract_address".to_string(),
                            Value::String("0x0000000000000000000000000000000000001002".to_string()),
                        )]),
                    },
                    metadata: BTreeMap::new(),
                },
            ],
        }
    }
}
