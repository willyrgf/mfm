use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use mfm_state_aave_v3::portfolio::model::{
    decode_aave_protocol_position_config, is_aave_protocol_position,
    validate_aave_portfolio_config, AaveProtocolPositionConfig,
};
use mfm_state_aave_v3::portfolio::semantic::{
    AaveDebtObservationPayload, AaveReserveObservationPayload,
};
use mfm_state_portfolio::semantic_adapters::{
    DerivedUnitPriceRuntimeAdapter, EvmAddressSubjectRuntimeAdapter,
    EvmOracleDirectPriceRuntimeAdapter, EvmViewRuntimeAdapter,
    FixedUnitPriceRuntimeAdapter,
};
use mfm_state_portfolio::model::{validate_portfolio_bundle, NetworkConfig};
use mfm_state_portfolio::semantic::{
    AdapterId, CompiledObservationBatch, CompiledObservationBinding, Instrument,
    DerivedUnitPriceValuationPayload, DirectPriceSourcePayload, DirectPriceValuationPayload,
    Erc20BalanceObservationPayload, EvmRoutePolicy, EvmSubjectLocator,
    FixedUnitPriceValuationPayload, InstrumentSemantics, NativeBalanceObservationPayload,
    NetworkFamily, NetworkView, ObservationPlanRequest, ObservationProjection, ObservationTarget,
    PlannerAdapter, PlanningError, PortfolioExecutionSpec, PortfolioRequest,
    PortfolioSemanticCompiler, PortfolioSemanticConfig, Position, PositionSemantics,
    QuantitySchema, SemanticCatalog, SemanticCatalogError, SemanticCatalogParts,
    SourcePreparationTask, Subject, SubjectKind, SubjectPlanRequest, SubjectPlannerAdapter,
    SubjectResolutionTask, Valuation, ValuationPlanRequest, ValuationPlannerAdapter,
    ValuationSemantics, ValuationTask, Venue, VenueId, ViewPinTask, ViewPlanRequest,
    ViewPlannerAdapter,
};
use mfm_state_symbol::model::{
    BalanceReaderConfig, PriceSourceRef, QuoteValuationConfig, SymbolConfig, SymbolKind,
    ValuationReaderConfig, ValuationSourceConfig, ValuationSourceReaderConfig,
};
use mfm_state_wallet::model::WalletConfig;
use serde::{Deserialize, Serialize};
use serde_json::Value;

const ADAPTER_RESOLVE_SUBJECT_EVM_ADDRESS: &str = "resolve_subject/evm_address";
const ADAPTER_PIN_VIEW_EVM: &str = "pin_view/evm";
const ADAPTER_RESOLVE_VALUATION_FIXED: &str = "resolve_valuation/fixed_unit_price";
const ADAPTER_RESOLVE_VALUATION_EVM_ORACLE: &str = "resolve_valuation/evm_oracle_direct_price";
const ADAPTER_RESOLVE_VALUATION_DERIVED: &str = "resolve_valuation/derived_unit_price";
const ADAPTER_OBSERVE_EVM_NATIVE_BALANCE: &str = "observe_position/evm/native_balance";
const ADAPTER_OBSERVE_EVM_ERC20_BALANCE: &str = "observe_position/evm/erc20_balance";
const ADAPTER_OBSERVE_AAVE_RESERVE: &str = "observe_position/aave_v3/reserve_position";
const ADAPTER_OBSERVE_AAVE_DEBT: &str = "observe_position/aave_v3/debt_position";

const READER_HINT_EVM_NATIVE_BALANCE: &str = "evm/native_balance";
const READER_HINT_EVM_ERC20_BALANCE: &str = "evm/erc20_balance";
const READER_HINT_AAVE_RESERVE: &str = "aave_v3/reserve_position";
const READER_HINT_AAVE_DEBT: &str = "aave_v3/debt_position";

/// Deterministic built-in semantic catalog for the current portfolio compiler.
pub fn builtin_semantic_catalog() -> Result<SemanticCatalog, SemanticCatalogError> {
    SemanticCatalog::new(SemanticCatalogParts {
        observation_planners: vec![
            Arc::new(EvmNativeBalanceObservationPlannerAdapter),
            Arc::new(EvmErc20BalanceObservationPlannerAdapter),
            Arc::new(AaveReserveObservationPlannerAdapter),
            Arc::new(AaveDebtObservationPlannerAdapter),
        ],
        valuation_planners: vec![
            Arc::new(FixedUnitPricePlannerAdapter),
            Arc::new(EvmOracleDirectPricePlannerAdapter),
            Arc::new(DerivedUnitPricePlannerAdapter),
        ],
        subject_planners: vec![Arc::new(EvmAddressSubjectPlannerAdapter)],
        subject_runtimes: vec![Arc::new(EvmAddressSubjectRuntimeAdapter)],
        view_planners: vec![Arc::new(EvmViewPlannerAdapter)],
        view_runtimes: vec![Arc::new(EvmViewRuntimeAdapter)],
        valuation_runtimes: vec![
            Arc::new(FixedUnitPriceRuntimeAdapter),
            Arc::new(EvmOracleDirectPriceRuntimeAdapter),
            Arc::new(DerivedUnitPriceRuntimeAdapter),
        ],
        ..SemanticCatalogParts::default()
    })
}

/// Default compiler that lowers the current canonical portfolio bundle into semantic execution.
#[derive(Clone, Default)]
pub struct DefaultPortfolioSemanticCompiler;

impl PortfolioSemanticCompiler for DefaultPortfolioSemanticCompiler {
    fn compile(
        &self,
        request: &PortfolioRequest,
        catalog: &SemanticCatalog,
    ) -> Result<PortfolioExecutionSpec, PlanningError> {
        validate_portfolio_bundle(&request.portfolio, &request.valuation_source_registry)
            .map_err(|err| compile_error("invalid_portfolio_bundle", err.to_string()))?;
        validate_aave_portfolio_config(&request.portfolio)
            .map_err(|err| compile_error("invalid_aave_portfolio_config", err.to_string()))?;

        let semantic_config = lower_semantic_config(request)?.normalized();
        semantic_config
            .validate()
            .map_err(|err| compile_error("invalid_semantic_config", err.to_string()))?;

        let subjects_by_id: HashMap<&str, &Subject> = semantic_config
            .subjects
            .iter()
            .map(|subject| (subject.subject_id.as_str(), subject))
            .collect();
        let views_by_id: HashMap<&str, &NetworkView> = semantic_config
            .network_views
            .iter()
            .map(|view| (view.network_view_id.as_str(), view))
            .collect();
        let instruments_by_id: HashMap<&str, &Instrument> = semantic_config
            .instruments
            .iter()
            .map(|instrument| (instrument.instrument_id.as_str(), instrument))
            .collect();
        let venues_by_id: HashMap<&str, &Venue> = semantic_config
            .venues
            .iter()
            .map(|venue| (venue.venue_id.0.as_str(), venue))
            .collect();
        let positions_by_id: HashMap<&str, &Position> = semantic_config
            .positions
            .iter()
            .map(|position| (position.position_id.as_str(), position))
            .collect();
        let valuations_by_id: HashMap<&str, &Valuation> = semantic_config
            .valuations
            .iter()
            .map(|valuation| (valuation.valuation_id.as_str(), valuation))
            .collect();

        let mut source_tasks = Vec::new();
        for view in &semantic_config.network_views {
            source_tasks.push(SourcePreparationTask {
                task_id: format!("prepare.{}", view.network_view_id),
                network_view_id: view.network_view_id.clone(),
                family: view.family.clone(),
                payload: value_to_object_map(
                    &view.route_policy,
                    "network_view.route_policy",
                    &view.network_view_id,
                )?,
            });
        }

        let mut subject_tasks = Vec::new();
        for subject in &semantic_config.subjects {
            subject_tasks.push(catalog.plan_subject(SubjectPlanRequest { subject })?);
        }

        let mut view_tasks = Vec::new();
        for view in &semantic_config.network_views {
            view_tasks.push(catalog.plan_view(ViewPlanRequest { network_view: view })?);
        }

        let mut valuation_tasks = Vec::new();
        for valuation in &semantic_config.valuations {
            let instrument = instruments_by_id
                .get(valuation.instrument_id.as_str())
                .copied()
                .ok_or_else(|| {
                    compile_error(
                        "missing_instrument_for_valuation",
                        format!(
                            "instrument `{}` was not found for valuation `{}`",
                            valuation.instrument_id, valuation.valuation_id
                        ),
                    )
                })?;
            valuation_tasks.push(catalog.plan_valuation(ValuationPlanRequest {
                valuation,
                instrument,
            })?);
        }

        let mut batches_by_key: BTreeMap<(AdapterId, String), Vec<CompiledObservationBinding>> =
            BTreeMap::new();
        for target in &semantic_config.observation_targets {
            let subject = subjects_by_id
                .get(target.subject_id.as_str())
                .copied()
                .ok_or_else(|| {
                    compile_error(
                        "missing_subject_for_target",
                        format!(
                            "subject `{}` was not found for target `{}`",
                            target.subject_id, target.target_id
                        ),
                    )
                })?;
            let network_view = views_by_id
                .get(target.network_view_id.as_str())
                .copied()
                .ok_or_else(|| {
                    compile_error(
                        "missing_view_for_target",
                        format!(
                            "network view `{}` was not found for target `{}`",
                            target.network_view_id, target.target_id
                        ),
                    )
                })?;
            let position = positions_by_id
                .get(target.position_id.as_str())
                .copied()
                .ok_or_else(|| {
                    compile_error(
                        "missing_position_for_target",
                        format!(
                            "position `{}` was not found for target `{}`",
                            target.position_id, target.target_id
                        ),
                    )
                })?;
            let instrument = instruments_by_id
                .get(position.instrument_id.as_str())
                .copied()
                .ok_or_else(|| {
                    compile_error(
                        "missing_instrument_for_position",
                        format!(
                            "instrument `{}` was not found for position `{}`",
                            position.instrument_id, position.position_id
                        ),
                    )
                })?;
            let venue = position
                .venue_id
                .as_ref()
                .and_then(|venue_id| venues_by_id.get(venue_id.0.as_str()).copied());
            let valuations = target
                .valuation_ids
                .iter()
                .map(|valuation_id| {
                    valuations_by_id
                        .get(valuation_id.as_str())
                        .map(|valuation| (*valuation).clone())
                        .ok_or_else(|| {
                            compile_error(
                                "missing_valuation_for_target",
                                format!(
                                    "valuation `{}` was not found for target `{}`",
                                    valuation_id, target.target_id
                                ),
                            )
                        })
                })
                .collect::<Result<Vec<_>, _>>()?;
            let binding = catalog.plan_observation(ObservationPlanRequest {
                target,
                subject,
                network_view,
                position,
                instrument,
                venue,
                valuations: valuations.as_slice(),
            })?;
            batches_by_key
                .entry((binding.adapter.clone(), target.network_view_id.clone()))
                .or_default()
                .push(binding);
        }

        let mut observation_batches = Vec::new();
        for ((adapter, network_view_id), bindings) in batches_by_key {
            observation_batches.push(CompiledObservationBatch {
                batch_id: format!("observe.{network_view_id}.{}", sanitize_id(&adapter.0)),
                adapter,
                network_view_id,
                bindings,
            });
        }

        let mut spec = PortfolioExecutionSpec {
            portfolio_id: semantic_config.portfolio_id.clone(),
            quote_codes: semantic_config.quote_codes.clone(),
            source_tasks,
            subject_tasks,
            view_tasks,
            valuation_tasks,
            observation_batches,
        };
        spec.normalize();
        spec.validate()
            .map_err(|err| compile_error("invalid_execution_spec", err.to_string()))?;
        Ok(spec)
    }
}

fn lower_semantic_config(
    request: &PortfolioRequest,
) -> Result<PortfolioSemanticConfig, PlanningError> {
    let networks_by_id: HashMap<&str, &NetworkConfig> = request
        .portfolio
        .networks
        .iter()
        .map(|network| (network.network_id.as_str(), network))
        .collect();
    let symbols_by_id: HashMap<&str, &SymbolConfig> = request
        .portfolio
        .symbol_configs
        .iter()
        .map(|symbol| (symbol.symbol_id.as_str(), symbol))
        .collect();
    let valuation_sources_by_id: HashMap<&str, &ValuationSourceConfig> = request
        .valuation_source_registry
        .sources
        .iter()
        .map(|source| (source.source_id.as_str(), source))
        .collect();

    let network_views = request
        .portfolio
        .networks
        .iter()
        .map(lower_network_view)
        .collect::<Result<Vec<_>, _>>()?;
    let subjects = request
        .portfolio
        .wallets
        .iter()
        .map(lower_subject)
        .collect::<Result<Vec<_>, _>>()?;

    let mut instruments_by_id: BTreeMap<String, Instrument> = BTreeMap::new();
    let mut venues_by_id: BTreeMap<String, Venue> = BTreeMap::new();
    let mut positions_by_symbol_id: BTreeMap<String, Position> = BTreeMap::new();
    let mut valuation_ids_by_symbol_id: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut valuations_by_id: BTreeMap<String, Valuation> = BTreeMap::new();

    for symbol in &request.portfolio.symbol_configs {
        let lowering = lower_symbol(
            symbol,
            &networks_by_id,
            &symbols_by_id,
            &valuation_sources_by_id,
            &mut instruments_by_id,
            &mut venues_by_id,
        )?;
        if positions_by_symbol_id
            .insert(symbol.symbol_id.clone(), lowering.position)
            .is_some()
        {
            return Err(compile_error(
                "duplicate_symbol_position",
                format!("symbol `{}` lowered more than once", symbol.symbol_id),
            ));
        }
        valuation_ids_by_symbol_id.insert(symbol.symbol_id.clone(), Vec::new());
        for valuation in lowering.valuations {
            let valuation_id = valuation.valuation_id.clone();
            if valuations_by_id
                .insert(valuation_id.clone(), valuation)
                .is_some()
            {
                return Err(compile_error(
                    "duplicate_valuation_id",
                    format!("valuation `{valuation_id}` was lowered more than once"),
                ));
            }
            valuation_ids_by_symbol_id
                .get_mut(&symbol.symbol_id)
                .expect("symbol was inserted")
                .push(valuation_id);
        }
    }

    let mut observation_targets = Vec::new();
    for wallet in &request.portfolio.wallets {
        for symbol_id in &wallet.symbol_ids {
            let position = positions_by_symbol_id.get(symbol_id).ok_or_else(|| {
                compile_error(
                    "missing_symbol_for_wallet",
                    format!(
                        "wallet `{}` referenced unknown symbol `{symbol_id}`",
                        wallet.wallet_id
                    ),
                )
            })?;
            let symbol = symbols_by_id
                .get(symbol_id.as_str())
                .copied()
                .ok_or_else(|| {
                    compile_error(
                        "missing_symbol_config",
                        format!("symbol `{symbol_id}` was not found during target lowering"),
                    )
                })?;
            if symbol.network_id != wallet.network_id {
                return Err(compile_error(
                    "wallet_symbol_network_mismatch",
                    format!(
                        "wallet `{}` network `{}` did not match symbol `{}` network `{}`",
                        wallet.wallet_id, wallet.network_id, symbol.symbol_id, symbol.network_id
                    ),
                ));
            }
            observation_targets.push(ObservationTarget {
                target_id: format!("{}::{}", wallet.wallet_id, symbol.symbol_id),
                subject_id: wallet.wallet_id.clone(),
                network_view_id: wallet.network_id.clone(),
                position_id: position.position_id.clone(),
                valuation_ids: valuation_ids_by_symbol_id
                    .get(symbol.symbol_id.as_str())
                    .cloned()
                    .unwrap_or_default(),
                metadata: BTreeMap::new(),
            });
        }
    }

    Ok(PortfolioSemanticConfig {
        portfolio_id: request.portfolio.portfolio_id.clone(),
        quote_codes: request.portfolio.quote_codes.clone(),
        network_views,
        subjects,
        instruments: instruments_by_id.into_values().collect(),
        venues: venues_by_id.into_values().collect(),
        positions: positions_by_symbol_id.into_values().collect(),
        valuations: valuations_by_id.into_values().collect(),
        observation_targets,
        metadata: request.portfolio.metadata.clone(),
    })
}

struct LoweredSymbol {
    position: Position,
    valuations: Vec<Valuation>,
}

fn lower_symbol(
    symbol: &SymbolConfig,
    networks_by_id: &HashMap<&str, &NetworkConfig>,
    symbols_by_id: &HashMap<&str, &SymbolConfig>,
    valuation_sources_by_id: &HashMap<&str, &ValuationSourceConfig>,
    instruments_by_id: &mut BTreeMap<String, Instrument>,
    venues_by_id: &mut BTreeMap<String, Venue>,
) -> Result<LoweredSymbol, PlanningError> {
    let instrument_source = if is_aave_protocol_position(symbol) {
        let underlying_symbol_id = symbol.underlying_symbol_id.as_ref().ok_or_else(|| {
            compile_error(
                "missing_underlying_symbol",
                format!(
                    "symbol `{}` did not declare underlying_symbol_id",
                    symbol.symbol_id
                ),
            )
        })?;
        symbols_by_id
            .get(underlying_symbol_id.as_str())
            .copied()
            .ok_or_else(|| {
                compile_error(
                    "unknown_underlying_symbol",
                    format!(
                        "symbol `{}` referenced unknown underlying symbol `{}`",
                        symbol.symbol_id, underlying_symbol_id
                    ),
                )
            })?
    } else {
        symbol
    };

    let instrument = build_instrument(instrument_source)?;
    insert_instrument(instruments_by_id, instrument)?;

    let (position, additional_venues) = build_position(symbol, networks_by_id)?;
    for venue in additional_venues {
        insert_venue(venues_by_id, venue)?;
    }

    let valuations = symbol
        .valuation
        .quotes
        .iter()
        .map(|quote| {
            build_valuation(
                symbol,
                instrument_source.symbol_id.clone(),
                quote,
                networks_by_id,
                valuation_sources_by_id,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;

    Ok(LoweredSymbol {
        position,
        valuations,
    })
}

fn lower_network_view(network: &NetworkConfig) -> Result<NetworkView, PlanningError> {
    Ok(NetworkView {
        network_view_id: network.network_id.clone(),
        network_id: network.network_id.clone(),
        family: NetworkFamily::Evm,
        route_policy: to_value(&EvmRoutePolicy {
            network_id: network.network_id.clone(),
            chain_id: network.chain_id,
            control_scope: network.control_scope.clone(),
        })?,
        metadata: network.metadata.clone(),
    })
}

fn lower_subject(wallet: &WalletConfig) -> Result<Subject, PlanningError> {
    Ok(Subject {
        subject_id: wallet.wallet_id.clone(),
        kind: SubjectKind::EvmAddress,
        locator: to_value(&EvmSubjectLocator {
            network_id: wallet.network_id.clone(),
            address: wallet.address.clone(),
            implementation: wallet.implementation.clone(),
        })?,
        metadata: wallet.metadata.clone(),
    })
}

fn route_policy_for_network(
    networks_by_id: &HashMap<&str, &NetworkConfig>,
    network_id: &str,
) -> Result<EvmRoutePolicy, PlanningError> {
    let network = networks_by_id.get(network_id).copied().ok_or_else(|| {
        compile_error(
            "missing_network_for_route_policy",
            format!("network `{network_id}` was not found for semantic route policy lowering"),
        )
    })?;
    Ok(EvmRoutePolicy {
        network_id: network.network_id.clone(),
        chain_id: network.chain_id,
        control_scope: network.control_scope.clone(),
    })
}

fn build_instrument(symbol: &SymbolConfig) -> Result<Instrument, PlanningError> {
    Ok(Instrument {
        instrument_id: symbol.symbol_id.clone(),
        display_symbol: symbol.display_symbol.clone(),
        semantics: match symbol.kind {
            SymbolKind::NativeBalance => InstrumentSemantics::NativeAsset,
            SymbolKind::Erc20Balance
            | SymbolKind::ProtocolPosition
            | SymbolKind::StakedPosition => InstrumentSemantics::FungibleToken,
        },
        quantity_schema: to_value(&QuantitySchema {
            decimals: symbol.decimals.unwrap_or(18),
        })?,
        metadata: symbol.metadata.clone(),
    })
}

fn insert_instrument(
    instruments_by_id: &mut BTreeMap<String, Instrument>,
    instrument: Instrument,
) -> Result<(), PlanningError> {
    match instruments_by_id.get(instrument.instrument_id.as_str()) {
        Some(existing) if existing != &instrument => Err(compile_error(
            "inconsistent_instrument_definition",
            format!(
                "instrument `{}` was lowered with conflicting definitions",
                instrument.instrument_id
            ),
        )),
        Some(_) => Ok(()),
        None => {
            instruments_by_id.insert(instrument.instrument_id.clone(), instrument);
            Ok(())
        }
    }
}

fn insert_venue(
    venues_by_id: &mut BTreeMap<String, Venue>,
    venue: Venue,
) -> Result<(), PlanningError> {
    match venues_by_id.get(venue.venue_id.0.as_str()) {
        Some(existing) if existing != &venue => Err(compile_error(
            "inconsistent_venue_definition",
            format!(
                "venue `{}` was lowered with conflicting definitions",
                venue.venue_id
            ),
        )),
        Some(_) => Ok(()),
        None => {
            venues_by_id.insert(venue.venue_id.0.clone(), venue);
            Ok(())
        }
    }
}

fn build_position(
    symbol: &SymbolConfig,
    networks_by_id: &HashMap<&str, &NetworkConfig>,
) -> Result<(Position, Vec<Venue>), PlanningError> {
    if is_aave_protocol_position(symbol) {
        return build_aave_position(symbol, networks_by_id);
    }

    let projection = observation_projection(symbol);
    let route_policy = route_policy_for_network(networks_by_id, &symbol.network_id)?;
    match &symbol.balance_reader {
        BalanceReaderConfig::NativeBalance {} => Ok((
            Position {
                position_id: symbol.symbol_id.clone(),
                instrument_id: symbol.symbol_id.clone(),
                semantics: PositionSemantics::SpotBalance,
                venue_id: None,
                reader_hint: Some(READER_HINT_EVM_NATIVE_BALANCE.to_string()),
                metadata: to_object_map(&NativeBalanceObservationPayload {
                    projection,
                    route_policy,
                })?,
            },
            Vec::new(),
        )),
        BalanceReaderConfig::Erc20Balance { token_address } => Ok((
            Position {
                position_id: symbol.symbol_id.clone(),
                instrument_id: symbol.symbol_id.clone(),
                semantics: PositionSemantics::SpotBalance,
                venue_id: None,
                reader_hint: Some(READER_HINT_EVM_ERC20_BALANCE.to_string()),
                metadata: to_object_map(&Erc20BalanceObservationPayload {
                    projection,
                    route_policy,
                    token_address: token_address.clone(),
                })?,
            },
            Vec::new(),
        )),
        BalanceReaderConfig::ProtocolPosition {
            protocol, reader, ..
        } => Err(compile_error(
            "unsupported_protocol_position_reader",
            format!(
                "symbol `{}` used unsupported protocol position reader `{protocol}/{reader}`",
                symbol.symbol_id
            ),
        )),
    }
}

fn build_aave_position(
    symbol: &SymbolConfig,
    networks_by_id: &HashMap<&str, &NetworkConfig>,
) -> Result<(Position, Vec<Venue>), PlanningError> {
    let projection = observation_projection(symbol);
    let underlying_symbol_id = symbol.underlying_symbol_id.clone().ok_or_else(|| {
        compile_error(
            "missing_underlying_symbol",
            format!(
                "aave symbol `{}` did not declare underlying_symbol_id",
                symbol.symbol_id
            ),
        )
    })?;
    let cfg = decode_aave_protocol_position_config(symbol)
        .map_err(|err| compile_error("invalid_aave_symbol", err.to_string()))?;
    let market = cfg.market().clone();
    let route_policy = route_policy_for_network(networks_by_id, &market.network_id)?;
    let reserve_id = cfg.reserve_id().to_string();
    let market_venue_id = VenueId(format!("aave_v3/{}", market.market_id));
    let reserve_venue_id = VenueId(format!("{}/{}", market_venue_id.0, reserve_id));
    let market_venue = Venue {
        venue_id: market_venue_id.clone(),
        display_name: Some(market.market_id.clone()),
        kind: "lending_market".to_string(),
        network_id: Some(market.network_id.clone()),
        parent_venue_id: None,
        metadata: market.metadata.clone(),
    };
    let reserve_metadata = market
        .reserve(reserve_id.as_str())
        .map(|reserve| reserve.metadata.clone())
        .unwrap_or_default();
    let reserve_venue = Venue {
        venue_id: reserve_venue_id.clone(),
        display_name: Some(reserve_id.clone()),
        kind: "reserve".to_string(),
        network_id: Some(market.network_id.clone()),
        parent_venue_id: Some(market_venue_id.clone()),
        metadata: reserve_metadata,
    };

    match cfg {
        AaveProtocolPositionConfig::ReservePosition(cfg) => Ok((
            Position {
                position_id: symbol.symbol_id.clone(),
                instrument_id: underlying_symbol_id.clone(),
                semantics: PositionSemantics::LendingDeposit,
                venue_id: Some(reserve_venue_id),
                reader_hint: Some(READER_HINT_AAVE_RESERVE.to_string()),
                metadata: to_object_map(&AaveReserveObservationPayload {
                    projection,
                    route_policy,
                    underlying_symbol_id,
                    config: cfg,
                })?,
            },
            vec![market_venue, reserve_venue],
        )),
        AaveProtocolPositionConfig::DebtPosition(cfg) => Ok((
            Position {
                position_id: symbol.symbol_id.clone(),
                instrument_id: underlying_symbol_id.clone(),
                semantics: PositionSemantics::LendingDebt,
                venue_id: Some(reserve_venue_id),
                reader_hint: Some(READER_HINT_AAVE_DEBT.to_string()),
                metadata: to_object_map(&AaveDebtObservationPayload {
                    projection,
                    route_policy,
                    underlying_symbol_id,
                    config: cfg,
                })?,
            },
            vec![market_venue, reserve_venue],
        )),
    }
}

fn build_valuation(
    symbol: &SymbolConfig,
    instrument_id: String,
    quote: &QuoteValuationConfig,
    networks_by_id: &HashMap<&str, &NetworkConfig>,
    valuation_sources_by_id: &HashMap<&str, &ValuationSourceConfig>,
) -> Result<Valuation, PlanningError> {
    let valuation_id = format!(
        "{}.quote.{}",
        symbol.symbol_id,
        quote.quote.as_str().to_ascii_lowercase()
    );
    let (semantics, strategy) = match &quote.reader {
        ValuationReaderConfig::FixedUnitPrice { unit_price_dec } => (
            ValuationSemantics::FixedUnitPrice,
            to_value(&FixedUnitPriceValuationPayload {
                priced_symbol_id: quote.priced_symbol_id.clone(),
                unit_price_dec: unit_price_dec.clone(),
            })?,
        ),
        ValuationReaderConfig::DirectPrice { source } => {
            let source_cfg =
                lookup_valuation_source(valuation_sources_by_id, &valuation_id, source)?;
            (
                ValuationSemantics::DirectUnitPrice,
                to_value(&DirectPriceValuationPayload {
                    priced_symbol_id: quote.priced_symbol_id.clone(),
                    source: DirectPriceSourcePayload {
                        source: source.clone(),
                        route_policy: route_policy_for_network(networks_by_id, &source.network_id)?,
                        source_reader: source_cfg.reader.clone(),
                    },
                })?,
            )
        }
        ValuationReaderConfig::DerivedUnitPrice {
            numerator,
            denominator,
        } => {
            let numerator_cfg =
                lookup_valuation_source(valuation_sources_by_id, &valuation_id, numerator)?;
            let denominator_cfg =
                lookup_valuation_source(valuation_sources_by_id, &valuation_id, denominator)?;
            (
                ValuationSemantics::DerivedUnitPrice,
                to_value(&DerivedUnitPriceValuationPayload {
                    priced_symbol_id: quote.priced_symbol_id.clone(),
                    numerator: DirectPriceSourcePayload {
                        source: numerator.clone(),
                        route_policy: route_policy_for_network(
                            networks_by_id,
                            &numerator.network_id,
                        )?,
                        source_reader: numerator_cfg.reader.clone(),
                    },
                    denominator: DirectPriceSourcePayload {
                        source: denominator.clone(),
                        route_policy: route_policy_for_network(
                            networks_by_id,
                            &denominator.network_id,
                        )?,
                        source_reader: denominator_cfg.reader.clone(),
                    },
                })?,
            )
        }
    };

    Ok(Valuation {
        valuation_id,
        instrument_id,
        quote: quote.quote,
        semantics,
        strategy,
        metadata: symbol.metadata.clone(),
    })
}

fn lookup_valuation_source<'a>(
    valuation_sources_by_id: &'a HashMap<&str, &'a ValuationSourceConfig>,
    valuation_id: &str,
    source: &PriceSourceRef,
) -> Result<&'a ValuationSourceConfig, PlanningError> {
    valuation_sources_by_id
        .get(source.source_id.as_str())
        .copied()
        .ok_or_else(|| {
            compile_error(
                "unknown_valuation_source",
                format!(
                    "valuation `{valuation_id}` referenced unknown valuation source `{}`",
                    source.source_id
                ),
            )
        })
}

fn observation_projection(symbol: &SymbolConfig) -> ObservationProjection {
    ObservationProjection {
        symbol_id: symbol.symbol_id.clone(),
        display_symbol: symbol.display_symbol.clone(),
        kind: symbol.kind,
        role: symbol.role,
        network_id: symbol.network_id.clone(),
        protocol: symbol.protocol.clone(),
        decimals: symbol.decimals.unwrap_or(18),
    }
}

fn sanitize_id(value: &str) -> String {
    value
        .chars()
        .map(|ch| match ch {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '_' => ch,
            _ => '_',
        })
        .collect()
}

fn compile_error(code: &'static str, message: impl Into<String>) -> PlanningError {
    PlanningError::compile(code, message)
}

fn to_value<T: Serialize>(value: &T) -> Result<Value, PlanningError> {
    serde_json::to_value(value)
        .map_err(|err| compile_error("semantic_serialize_failed", err.to_string()))
}

fn to_object_map<T: Serialize>(value: &T) -> Result<BTreeMap<String, Value>, PlanningError> {
    value_to_object_map(
        &to_value(value)?,
        "semantic_payload",
        "semantic_payload_object",
    )
}

fn value_to_object_map(
    value: &Value,
    field: &'static str,
    owner: &str,
) -> Result<BTreeMap<String, Value>, PlanningError> {
    let object = value.as_object().ok_or_else(|| {
        compile_error(
            "semantic_payload_not_object",
            format!("{field} for `{owner}` was not a JSON object"),
        )
    })?;
    Ok(object
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect())
}

fn decode_object_map<T: for<'de> Deserialize<'de>>(
    adapter: &AdapterId,
    field: &'static str,
    map: &BTreeMap<String, Value>,
) -> Result<T, PlanningError> {
    serde_json::from_value(Value::Object(
        map.iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect(),
    ))
    .map_err(|err| PlanningError::adapter(adapter.clone(), field, err.to_string()))
}

fn decode_value<T: for<'de> Deserialize<'de>>(
    adapter: &AdapterId,
    field: &'static str,
    value: &Value,
) -> Result<T, PlanningError> {
    serde_json::from_value(value.clone())
        .map_err(|err| PlanningError::adapter(adapter.clone(), field, err.to_string()))
}

fn build_observation_binding(
    adapter: &str,
    target: &ObservationTarget,
    position: &Position,
    payload: BTreeMap<String, Value>,
) -> CompiledObservationBinding {
    CompiledObservationBinding {
        binding_id: format!("binding.{}", target.target_id),
        observation_key: mfm_state_portfolio::semantic::ObservationKey {
            subject_id: target.subject_id.clone(),
            network_view_id: target.network_view_id.clone(),
            instrument_id: position.instrument_id.clone(),
            position_kind: position.semantics.clone(),
            venue_id: position.venue_id.clone(),
            discriminator: Some(position.position_id.clone()),
        },
        adapter: AdapterId(adapter.to_string()),
        valuation_ids: target.valuation_ids.clone(),
        payload,
    }
}

struct EvmAddressSubjectPlannerAdapter;

impl PlannerAdapter for EvmAddressSubjectPlannerAdapter {
    fn id(&self) -> AdapterId {
        AdapterId(ADAPTER_RESOLVE_SUBJECT_EVM_ADDRESS.to_string())
    }
}

impl SubjectPlannerAdapter for EvmAddressSubjectPlannerAdapter {
    fn supports(&self, req: &SubjectPlanRequest<'_>) -> bool {
        req.subject.kind == SubjectKind::EvmAddress
    }

    fn plan(&self, req: SubjectPlanRequest<'_>) -> Result<SubjectResolutionTask, PlanningError> {
        let adapter = self.id();
        let _locator: EvmSubjectLocator =
            decode_value(&adapter, "subject_locator", &req.subject.locator)?;
        Ok(SubjectResolutionTask {
            task_id: format!("resolve.{}", req.subject.subject_id),
            subject_id: req.subject.subject_id.clone(),
            kind: req.subject.kind.clone(),
            adapter: adapter.clone(),
            payload: value_to_object_map(
                &req.subject.locator,
                "subject.locator",
                &req.subject.subject_id,
            )?,
        })
    }
}

struct EvmViewPlannerAdapter;

impl PlannerAdapter for EvmViewPlannerAdapter {
    fn id(&self) -> AdapterId {
        AdapterId(ADAPTER_PIN_VIEW_EVM.to_string())
    }
}

impl ViewPlannerAdapter for EvmViewPlannerAdapter {
    fn supports(&self, req: &ViewPlanRequest<'_>) -> bool {
        req.network_view.family == NetworkFamily::Evm
    }

    fn plan(&self, req: ViewPlanRequest<'_>) -> Result<ViewPinTask, PlanningError> {
        let adapter = self.id();
        let _policy: EvmRoutePolicy = decode_value(
            &adapter,
            "view_route_policy",
            &req.network_view.route_policy,
        )?;
        Ok(ViewPinTask {
            task_id: format!("pin.{}", req.network_view.network_view_id),
            network_view_id: req.network_view.network_view_id.clone(),
            family: req.network_view.family.clone(),
            adapter: adapter.clone(),
            payload: value_to_object_map(
                &req.network_view.route_policy,
                "network_view.route_policy",
                &req.network_view.network_view_id,
            )?,
        })
    }
}

struct FixedUnitPricePlannerAdapter;

impl PlannerAdapter for FixedUnitPricePlannerAdapter {
    fn id(&self) -> AdapterId {
        AdapterId(ADAPTER_RESOLVE_VALUATION_FIXED.to_string())
    }
}

impl ValuationPlannerAdapter for FixedUnitPricePlannerAdapter {
    fn supports(&self, req: &ValuationPlanRequest<'_>) -> bool {
        req.valuation.semantics == ValuationSemantics::FixedUnitPrice
    }

    fn plan(&self, req: ValuationPlanRequest<'_>) -> Result<ValuationTask, PlanningError> {
        let adapter = self.id();
        let payload: FixedUnitPriceValuationPayload =
            decode_value(&adapter, "valuation_strategy", &req.valuation.strategy)?;
        Ok(ValuationTask {
            valuation_id: req.valuation.valuation_id.clone(),
            instrument_id: req.valuation.instrument_id.clone(),
            quote: req.valuation.quote,
            adapter: adapter.clone(),
            payload: to_object_map(&payload)?,
        })
    }
}

struct EvmOracleDirectPricePlannerAdapter;

impl PlannerAdapter for EvmOracleDirectPricePlannerAdapter {
    fn id(&self) -> AdapterId {
        AdapterId(ADAPTER_RESOLVE_VALUATION_EVM_ORACLE.to_string())
    }
}

impl ValuationPlannerAdapter for EvmOracleDirectPricePlannerAdapter {
    fn supports(&self, req: &ValuationPlanRequest<'_>) -> bool {
        if req.valuation.semantics != ValuationSemantics::DirectUnitPrice {
            return false;
        }
        serde_json::from_value::<DirectPriceValuationPayload>(req.valuation.strategy.clone())
            .map(|payload| {
                matches!(
                    payload.source.source_reader,
                    ValuationSourceReaderConfig::EvmOracle { .. }
                )
            })
            .unwrap_or(false)
    }

    fn plan(&self, req: ValuationPlanRequest<'_>) -> Result<ValuationTask, PlanningError> {
        let adapter = self.id();
        let payload: DirectPriceValuationPayload =
            decode_value(&adapter, "valuation_strategy", &req.valuation.strategy)?;
        Ok(ValuationTask {
            valuation_id: req.valuation.valuation_id.clone(),
            instrument_id: req.valuation.instrument_id.clone(),
            quote: req.valuation.quote,
            adapter: adapter.clone(),
            payload: to_object_map(&payload)?,
        })
    }
}

struct DerivedUnitPricePlannerAdapter;

impl PlannerAdapter for DerivedUnitPricePlannerAdapter {
    fn id(&self) -> AdapterId {
        AdapterId(ADAPTER_RESOLVE_VALUATION_DERIVED.to_string())
    }
}

impl ValuationPlannerAdapter for DerivedUnitPricePlannerAdapter {
    fn supports(&self, req: &ValuationPlanRequest<'_>) -> bool {
        req.valuation.semantics == ValuationSemantics::DerivedUnitPrice
    }

    fn plan(&self, req: ValuationPlanRequest<'_>) -> Result<ValuationTask, PlanningError> {
        let adapter = self.id();
        let payload: DerivedUnitPriceValuationPayload =
            decode_value(&adapter, "valuation_strategy", &req.valuation.strategy)?;
        Ok(ValuationTask {
            valuation_id: req.valuation.valuation_id.clone(),
            instrument_id: req.valuation.instrument_id.clone(),
            quote: req.valuation.quote,
            adapter: adapter.clone(),
            payload: to_object_map(&payload)?,
        })
    }
}

struct EvmNativeBalanceObservationPlannerAdapter;

impl PlannerAdapter for EvmNativeBalanceObservationPlannerAdapter {
    fn id(&self) -> AdapterId {
        AdapterId(ADAPTER_OBSERVE_EVM_NATIVE_BALANCE.to_string())
    }
}

impl mfm_state_portfolio::semantic::ObservationPlannerAdapter
    for EvmNativeBalanceObservationPlannerAdapter
{
    fn supports(&self, req: &ObservationPlanRequest<'_>) -> bool {
        req.subject.kind == SubjectKind::EvmAddress
            && req.network_view.family == NetworkFamily::Evm
            && req.position.reader_hint.as_deref() == Some(READER_HINT_EVM_NATIVE_BALANCE)
    }

    fn plan(
        &self,
        req: ObservationPlanRequest<'_>,
    ) -> Result<CompiledObservationBinding, PlanningError> {
        let adapter = self.id();
        let payload: NativeBalanceObservationPayload =
            decode_object_map(&adapter, "position_payload", &req.position.metadata)?;
        Ok(build_observation_binding(
            ADAPTER_OBSERVE_EVM_NATIVE_BALANCE,
            req.target,
            req.position,
            to_object_map(&payload)?,
        ))
    }
}

struct EvmErc20BalanceObservationPlannerAdapter;

impl PlannerAdapter for EvmErc20BalanceObservationPlannerAdapter {
    fn id(&self) -> AdapterId {
        AdapterId(ADAPTER_OBSERVE_EVM_ERC20_BALANCE.to_string())
    }
}

impl mfm_state_portfolio::semantic::ObservationPlannerAdapter
    for EvmErc20BalanceObservationPlannerAdapter
{
    fn supports(&self, req: &ObservationPlanRequest<'_>) -> bool {
        req.subject.kind == SubjectKind::EvmAddress
            && req.network_view.family == NetworkFamily::Evm
            && req.position.reader_hint.as_deref() == Some(READER_HINT_EVM_ERC20_BALANCE)
    }

    fn plan(
        &self,
        req: ObservationPlanRequest<'_>,
    ) -> Result<CompiledObservationBinding, PlanningError> {
        let adapter = self.id();
        let payload: Erc20BalanceObservationPayload =
            decode_object_map(&adapter, "position_payload", &req.position.metadata)?;
        Ok(build_observation_binding(
            ADAPTER_OBSERVE_EVM_ERC20_BALANCE,
            req.target,
            req.position,
            to_object_map(&payload)?,
        ))
    }
}

struct AaveReserveObservationPlannerAdapter;

impl PlannerAdapter for AaveReserveObservationPlannerAdapter {
    fn id(&self) -> AdapterId {
        AdapterId(ADAPTER_OBSERVE_AAVE_RESERVE.to_string())
    }
}

impl mfm_state_portfolio::semantic::ObservationPlannerAdapter
    for AaveReserveObservationPlannerAdapter
{
    fn supports(&self, req: &ObservationPlanRequest<'_>) -> bool {
        req.subject.kind == SubjectKind::EvmAddress
            && req.network_view.family == NetworkFamily::Evm
            && req.position.reader_hint.as_deref() == Some(READER_HINT_AAVE_RESERVE)
    }

    fn plan(
        &self,
        req: ObservationPlanRequest<'_>,
    ) -> Result<CompiledObservationBinding, PlanningError> {
        let adapter = self.id();
        let payload: AaveReserveObservationPayload =
            decode_object_map(&adapter, "position_payload", &req.position.metadata)?;
        Ok(build_observation_binding(
            ADAPTER_OBSERVE_AAVE_RESERVE,
            req.target,
            req.position,
            to_object_map(&payload)?,
        ))
    }
}

struct AaveDebtObservationPlannerAdapter;

impl PlannerAdapter for AaveDebtObservationPlannerAdapter {
    fn id(&self) -> AdapterId {
        AdapterId(ADAPTER_OBSERVE_AAVE_DEBT.to_string())
    }
}

impl mfm_state_portfolio::semantic::ObservationPlannerAdapter
    for AaveDebtObservationPlannerAdapter
{
    fn supports(&self, req: &ObservationPlanRequest<'_>) -> bool {
        req.subject.kind == SubjectKind::EvmAddress
            && req.network_view.family == NetworkFamily::Evm
            && req.position.reader_hint.as_deref() == Some(READER_HINT_AAVE_DEBT)
    }

    fn plan(
        &self,
        req: ObservationPlanRequest<'_>,
    ) -> Result<CompiledObservationBinding, PlanningError> {
        let adapter = self.id();
        let payload: AaveDebtObservationPayload =
            decode_object_map(&adapter, "position_payload", &req.position.metadata)?;
        Ok(build_observation_binding(
            ADAPTER_OBSERVE_AAVE_DEBT,
            req.target,
            req.position,
            to_object_map(&payload)?,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_request() -> PortfolioRequest {
        let portfolio = serde_json::from_value(serde_json::json!({
            "portfolio_id": "portfolio_main",
            "quote_codes": ["USD", "BTC"],
            "networks": [
                {
                    "network_id": "ethereum-mainnet",
                    "chain_id": 1u64,
                    "control_scope": "rpc.mainnet",
                    "metadata": {}
                }
            ],
            "wallets": [
                {
                    "wallet_id": "wallet_main",
                    "address": "0x000000000000000000000000000000000000dead",
                    "implementation": { "kind": "address_only" },
                    "network_id": "ethereum-mainnet",
                    "symbol_ids": [
                        "eth.native.ethereum-mainnet",
                        "usdc.wallet.ethereum-mainnet",
                        "aave_v3.usdc.collateral.ethereum-mainnet"
                    ],
                    "metadata": {}
                }
            ],
            "symbol_configs": [
                {
                    "symbol_id": "eth.native.ethereum-mainnet",
                    "display_symbol": "ETH",
                    "kind": "native_balance",
                    "role": "native",
                    "network_id": "ethereum-mainnet",
                    "protocol": null,
                    "balance_reader": { "kind": "native_balance" },
                    "valuation": {
                        "quotes": [
                            {
                                "quote": "USD",
                                "priced_symbol_id": "eth.native.ethereum-mainnet",
                                "reader": {
                                    "kind": "direct_price",
                                    "source": {
                                        "source_id": "chainlink_eth_usd",
                                        "network_id": "ethereum-mainnet",
                                        "base_symbol_id": "eth.native.ethereum-mainnet",
                                        "quote": "USD"
                                    }
                                }
                            },
                            {
                                "quote": "BTC",
                                "priced_symbol_id": "eth.native.ethereum-mainnet",
                                "reader": {
                                    "kind": "direct_price",
                                    "source": {
                                        "source_id": "chainlink_eth_btc",
                                        "network_id": "ethereum-mainnet",
                                        "base_symbol_id": "eth.native.ethereum-mainnet",
                                        "quote": "BTC"
                                    }
                                }
                            }
                        ]
                    },
                    "decimals": 18,
                    "underlying_symbol_id": null,
                    "metadata": {}
                },
                {
                    "symbol_id": "usdc.wallet.ethereum-mainnet",
                    "display_symbol": "USDC",
                    "kind": "erc20_balance",
                    "role": "asset",
                    "network_id": "ethereum-mainnet",
                    "protocol": null,
                    "balance_reader": {
                        "kind": "erc20_balance",
                        "token_address": "0x0000000000000000000000000000000000000001"
                    },
                    "valuation": {
                        "quotes": [
                            {
                                "quote": "USD",
                                "priced_symbol_id": "usdc.wallet.ethereum-mainnet",
                                "reader": {
                                    "kind": "fixed_unit_price",
                                    "unit_price_dec": "1"
                                }
                            },
                            {
                                "quote": "BTC",
                                "priced_symbol_id": "usdc.wallet.ethereum-mainnet",
                                "reader": {
                                    "kind": "fixed_unit_price",
                                    "unit_price_dec": "0.00001"
                                }
                            }
                        ]
                    },
                    "decimals": 6,
                    "underlying_symbol_id": null,
                    "metadata": {}
                },
                {
                    "symbol_id": "aave_v3.usdc.collateral.ethereum-mainnet",
                    "display_symbol": "aUSDC",
                    "kind": "protocol_position",
                    "role": "collateral",
                    "network_id": "ethereum-mainnet",
                    "protocol": "aave_v3",
                    "balance_reader": {
                        "kind": "protocol_position",
                        "protocol": "aave_v3",
                        "reader": "reserve_position",
                        "config": {
                            "market": {
                                "market_id": "aave-v3-mainnet",
                                "network_id": "ethereum-mainnet",
                                "chain_id": 1u64,
                                "pool_address": "0x0000000000000000000000000000000000000002",
                                "reserves": [
                                    {
                                        "reserve_id": "usdc",
                                        "reserve_index": 1,
                                        "underlying_token_address": "0x0000000000000000000000000000000000000001",
                                        "a_token_address": "0x0000000000000000000000000000000000000003"
                                    }
                                ]
                            },
                            "reserve_id": "usdc",
                            "use_as_collateral_required": true
                        }
                    },
                    "valuation": {
                        "quotes": [
                            {
                                "quote": "USD",
                                "priced_symbol_id": "usdc.wallet.ethereum-mainnet",
                                "reader": {
                                    "kind": "fixed_unit_price",
                                    "unit_price_dec": "1"
                                }
                            },
                            {
                                "quote": "BTC",
                                "priced_symbol_id": "usdc.wallet.ethereum-mainnet",
                                "reader": {
                                    "kind": "fixed_unit_price",
                                    "unit_price_dec": "0.00001"
                                }
                            }
                        ]
                    },
                    "decimals": 6,
                    "underlying_symbol_id": "usdc.wallet.ethereum-mainnet",
                    "metadata": {}
                }
            ],
            "metadata": {}
        }))
        .expect("portfolio config");

        let valuation_source_registry = serde_json::from_value(serde_json::json!({
            "sources": [
                {
                    "source_id": "chainlink_eth_usd",
                    "network_id": "ethereum-mainnet",
                    "base_symbol_id": "eth.native.ethereum-mainnet",
                    "quote": "USD",
                    "reader": {
                        "kind": "evm_oracle",
                        "oracle_kind": "chainlink_aggregator_v3",
                        "config": {
                            "contract_address": "0x0000000000000000000000000000000000000010"
                        }
                    }
                },
                {
                    "source_id": "chainlink_eth_btc",
                    "network_id": "ethereum-mainnet",
                    "base_symbol_id": "eth.native.ethereum-mainnet",
                    "quote": "BTC",
                    "reader": {
                        "kind": "evm_oracle",
                        "oracle_kind": "chainlink_aggregator_v3",
                        "config": {
                            "contract_address": "0x0000000000000000000000000000000000000011"
                        }
                    }
                }
            ]
        }))
        .expect("valuation source registry");

        PortfolioRequest {
            portfolio,
            valuation_source_registry,
        }
    }

    #[test]
    fn compiler_lowers_current_config_into_semantic_execution_spec() {
        let catalog = builtin_semantic_catalog().expect("catalog");
        let compiler = DefaultPortfolioSemanticCompiler;
        let spec = compiler
            .compile(&sample_request(), &catalog)
            .expect("compiled execution spec");

        assert_eq!(spec.subject_tasks.len(), 1);
        assert_eq!(spec.view_tasks.len(), 1);
        assert_eq!(spec.source_tasks.len(), 1);
        assert_eq!(spec.valuation_tasks.len(), 6);
        assert_eq!(spec.observation_batches.len(), 3);
        assert_eq!(
            spec.observation_batches
                .iter()
                .map(|batch| batch.adapter.0.as_str())
                .collect::<Vec<_>>(),
            vec![
                ADAPTER_OBSERVE_AAVE_RESERVE,
                ADAPTER_OBSERVE_EVM_ERC20_BALANCE,
                ADAPTER_OBSERVE_EVM_NATIVE_BALANCE,
            ]
        );
    }

    #[test]
    fn compiler_is_deterministic_across_config_ordering() {
        let catalog = builtin_semantic_catalog().expect("catalog");
        let compiler = DefaultPortfolioSemanticCompiler;
        let mut request = sample_request();
        request.portfolio.wallets.reverse();
        request.portfolio.symbol_configs.reverse();
        request.portfolio.quote_codes.reverse();
        request.valuation_source_registry.sources.reverse();

        let left = compiler
            .compile(&sample_request(), &catalog)
            .expect("compiled execution spec");
        let right = compiler
            .compile(&request, &catalog)
            .expect("compiled execution spec");

        assert_eq!(left, right);
    }

    #[test]
    fn compiler_rejects_duplicate_semantic_observation_targets() {
        let catalog = builtin_semantic_catalog().expect("catalog");
        let compiler = DefaultPortfolioSemanticCompiler;
        let mut request = sample_request();
        request.portfolio.wallets[0]
            .symbol_ids
            .push("eth.native.ethereum-mainnet".to_string());

        let err = compiler.compile(&request, &catalog).expect_err("must fail");
        assert!(
            matches!(err, PlanningError::Compile { code, .. } if code == "invalid_semantic_config")
        );
    }
}
