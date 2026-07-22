use super::*;
use crate::{ExecutionAnchor, HoldingSourceConfig, NetworkConfig};

/// Builds quantity-only observations from selected holdings (valuation join deferred).
pub(super) fn observations_from_selected_holdings(
    selected: &[SelectedHolding],
    symbols_by_id: &BTreeMap<&str, &SymbolConfig>,
) -> Result<Vec<Observation>, PortfolioHoldingSelectionError> {
    let mut observations = Vec::with_capacity(selected.len());
    for item in selected {
        let symbol = symbols_by_id
            .get(item.key.symbol_id.as_str())
            .copied()
            .ok_or_else(|| {
                PortfolioHoldingSelectionError::new(
                    PortfolioHoldingErrorCode::UnsupportedRequirement,
                    format!("missing symbol config for {}", item.key.symbol_id),
                    Some(item.key.as_key_str()),
                    Some(item.key.network_id.clone()),
                )
            })?;
        let amount_dec = amount_dec_from_raw(&item.material.raw_dec, item.material.decimals)?;
        let mut observation = Observation {
            wallet_id: item.material.wallet_id.clone(),
            symbol_id: item.material.symbol_id.clone(),
            display_symbol: symbol.display_symbol.clone(),
            network_id: item.material.network_id.clone(),
            quantity: ObservationQuantity {
                raw_dec: item.material.raw_dec.clone(),
                decimals: item.material.decimals,
                amount_dec,
            },
            values: Vec::new(),
            source: AnchoredHoldingSource {
                holding: item.material.holding.clone(),
                anchor: item.material.observation_anchor.clone(),
            },
            metadata: symbol.metadata.clone(),
        };
        observation.normalize();
        observations.push(observation);
    }
    Ok(observations)
}

/// Derives configured display fields and fixed unit-price valuations onto observations.
///
/// The selected holdings are only authority for quantity and receipt-pinned collection evidence.
/// Every public presentation and valuation field is rebuilt from the certified portfolio config.
fn apply_configured_valuations_to_observations(
    mut observations: Vec<Observation>,
    symbols_by_id: &BTreeMap<&str, &SymbolConfig>,
) -> StateResult<Vec<Observation>> {
    for observation in &mut observations {
        let symbol = symbols_by_id
            .get(observation.symbol_id.as_str())
            .copied()
            .ok_or_else(|| {
                StateError::Message(format!(
                    "missing symbol config for observation {}",
                    observation.symbol_id
                ))
            })?;
        observation.display_symbol = symbol.display_symbol.clone();
        observation.metadata = symbol.metadata.clone();
        let mut values = Vec::new();
        for quote in &symbol.valuation.quotes {
            let value_dec = multiply_decimal_strings(
                &observation.quantity.amount_dec,
                quote.unit_price_dec.as_str(),
            )
            .map_err(|error| StateError::Message(error.to_string()))?;
            values.push(ObservationValue {
                quote: quote.quote,
                priced_symbol_id: quote.priced_symbol_id.to_string(),
                value_dec,
                unit_price_dec: quote.unit_price_dec.to_string(),
            });
        }
        values.sort_by_key(|value| value.quote);
        observation.values = values;
        observation.normalize();
    }
    Ok(observations)
}

/// Proves that selected observations exactly realize normalized portfolio demand.
///
/// Receipt identity and anchor checks belong to selection. Assembly independently protects its
/// public output against a substituted selection runner by rejecting missing, duplicate,
/// unexpected, family-mismatched, or config-mismatched observations.
fn require_exact_portfolio_observations(
    portfolio: &PortfolioConfig,
    observations: &[Observation],
) -> Result<(), PortfolioHoldingSelectionError> {
    let networks = portfolio
        .networks
        .iter()
        .map(|network| (network.network_id().as_str(), network))
        .collect::<BTreeMap<_, _>>();
    let symbols = symbols_by_id(&portfolio.symbol_configs)?;
    let mut expected = BTreeMap::new();
    for wallet in &portfolio.wallets {
        let network = networks
            .get(wallet.network_id.as_str())
            .copied()
            .ok_or_else(|| observation_error(None, "wallet network was missing"))?;
        for symbol_id in &wallet.symbol_ids {
            let symbol = symbols.get(symbol_id.as_str()).copied().ok_or_else(|| {
                observation_error(None, "wallet symbol was missing from portfolio config")
            })?;
            let key = HoldingRequirementKey {
                wallet_id: wallet.wallet_id.to_string(),
                symbol_id: symbol.symbol_id.to_string(),
                network_id: wallet.network_id.to_string(),
            };
            if expected.insert(key.clone(), (network, symbol)).is_some() {
                return Err(observation_error(
                    Some(&key),
                    "portfolio demand contained a duplicate holding requirement",
                ));
            }
        }
    }
    let mut actual = BTreeMap::new();
    for observation in observations {
        let key = HoldingRequirementKey {
            wallet_id: observation.wallet_id.clone(),
            symbol_id: observation.symbol_id.clone(),
            network_id: observation.network_id.clone(),
        };
        let Some((network, symbol)) = expected.get(&key).copied() else {
            return Err(observation_error(
                Some(&key),
                "selected observations contained an unexpected holding",
            ));
        };
        if actual.insert(key.clone(), observation).is_some() {
            return Err(observation_error(
                Some(&key),
                "selected observations contained a duplicate holding",
            ));
        }
        if observation.source.holding != symbol.source {
            return Err(observation_error(
                Some(&key),
                "selected observation source did not match portfolio config",
            ));
        }
        match (network, &observation.source.anchor) {
            (NetworkConfig::Bitcoin { .. }, ExecutionAnchor::Bitcoin { .. }) => {
                if observation.quantity.decimals != 8 {
                    return Err(observation_error(
                        Some(&key),
                        "Bitcoin selected observation did not use the fixed decimal scale",
                    ));
                }
            }
            (
                NetworkConfig::Evm {
                    chain_id,
                    native_decimals,
                    ..
                },
                ExecutionAnchor::Evm {
                    chain_id: observed_chain,
                    ..
                },
            ) if chain_id == observed_chain => {
                if matches!(symbol.source, HoldingSourceConfig::Native)
                    && observation.quantity.decimals != *native_decimals
                {
                    return Err(observation_error(
                        Some(&key),
                        "native EVM selected observation did not match the configured scale",
                    ));
                }
            }
            _ => {
                return Err(observation_error(
                    Some(&key),
                    "selected observation anchor did not match its network family",
                ));
            }
        }
    }
    if actual.len() != expected.len() {
        let missing = expected.keys().find(|key| !actual.contains_key(*key));
        return Err(observation_error(
            missing,
            "selected observations did not exactly cover portfolio demand",
        ));
    }
    Ok(())
}

fn observation_error(
    key: Option<&HoldingRequirementKey>,
    message: impl Into<String>,
) -> PortfolioHoldingSelectionError {
    PortfolioHoldingSelectionError::new(
        PortfolioHoldingErrorCode::ReceiptMismatch,
        message,
        key.map(HoldingRequirementKey::as_key_str),
        key.map(|key| key.network_id.clone()),
    )
}

/// Assembles the canonical portfolio snapshot (hard-fail; pins from selected observations).
pub(super) fn assemble_snapshot(
    config: &AssembleSnapshotConfig,
    input: AssembleSnapshotInput,
) -> StateResult<PortfolioSnapshot> {
    require_exact_portfolio_observations(&config.portfolio, &input.holdings.observations)
        .map_err(|error| StateError::Message(error.to_string()))?;
    let symbols = symbols_by_id(&config.portfolio.symbol_configs).map_err(|error| {
        StateError::Message(format!(
            "{code}: {message}",
            code = error.code,
            message = error.message
        ))
    })?;
    let mut observations = input.holdings.observations;
    observations.sort_by(|left, right| {
        (&left.wallet_id, &left.symbol_id, &left.network_id).cmp(&(
            &right.wallet_id,
            &right.symbol_id,
            &right.network_id,
        ))
    });
    let observations = apply_configured_valuations_to_observations(observations, &symbols)?;
    let network_pins = project_network_pins_from_observations(&observations).map_err(|error| {
        StateError::Message(format!(
            "{code}: {message}",
            code = error.code,
            message = error.message
        ))
    })?;
    let mut observations_by_wallet: BTreeMap<String, Vec<Observation>> = BTreeMap::new();
    for observation in observations {
        observations_by_wallet
            .entry(observation.wallet_id.clone())
            .or_default()
            .push(observation);
    }
    let mut wallets = Vec::with_capacity(config.portfolio.wallets.len());
    for wallet_cfg in &config.portfolio.wallets {
        // Required holdings already verified; absent wallet key means zero symbols configured.
        let observations = observations_by_wallet
            .remove(wallet_cfg.wallet_id.as_str())
            .unwrap_or_default();
        let mut wallet = WalletSnapshot {
            wallet_id: wallet_cfg.wallet_id.to_string(),
            subject: wallet_cfg.subject.clone(),
            network_id: wallet_cfg.network_id.to_string(),
            observations,
        };
        wallet.normalize();
        wallets.push(wallet);
    }
    wallets.sort_by(|left, right| left.wallet_id.cmp(&right.wallet_id));

    let mut symbol_configs = config.portfolio.symbol_configs.clone();
    for symbol in &mut symbol_configs {
        symbol.normalize();
    }
    symbol_configs.sort_by(|left, right| left.symbol_id.cmp(&right.symbol_id));

    let mut snapshot = PortfolioSnapshot {
        schema_version: PortfolioSnapshot::SCHEMA_VERSION,
        portfolio_id: config.portfolio.portfolio_id.to_string(),
        network_pins,
        wallets,
        symbol_configs,
    };
    snapshot.normalize();
    Ok(snapshot)
}

/// Projects the version-1 canonical portfolio report from a version-2 snapshot.
pub(super) fn project_report_from_snapshot(
    snapshot: PortfolioSnapshot,
) -> StateResult<PortfolioReport> {
    if snapshot.schema_version != PortfolioSnapshot::SCHEMA_VERSION {
        return Err(StateError::Message(
            "portfolio snapshot schema version is not supported".to_owned(),
        ));
    }
    let report_quotes = collect_report_quotes(&snapshot);
    let mut portfolio_totals = initialized_quote_totals(&report_quotes);
    let wallet_summaries = snapshot
        .wallets
        .iter()
        .map(|wallet| {
            let wallet_totals = derive_quote_totals(&report_quotes, &wallet.observations)?;
            merge_quote_totals(&mut portfolio_totals, &wallet_totals);
            Ok(WalletReport {
                wallet_id: wallet.wallet_id.to_string(),
                network_id: wallet.network_id.to_string(),
                totals_by_quote: quote_totals_to_vec(wallet_totals),
            })
        })
        .collect::<StateResult<Vec<_>>>()?;

    let mut report = PortfolioReport {
        schema_version: PortfolioReport::SCHEMA_VERSION,
        portfolio_id: snapshot.portfolio_id,
        network_pins: snapshot.network_pins,
        wallet_summaries,
        totals_by_quote: quote_totals_to_vec(portfolio_totals),
    };
    report.normalize();
    Ok(report)
}

/// Builds a symbols-by-id index for assemble / observation join.
pub(super) fn symbols_by_id_map(
    symbols: &[SymbolConfig],
) -> Result<BTreeMap<&str, &SymbolConfig>, PortfolioHoldingSelectionError> {
    symbols_by_id(symbols)
}

fn symbols_by_id(
    symbols: &[SymbolConfig],
) -> Result<BTreeMap<&str, &SymbolConfig>, PortfolioHoldingSelectionError> {
    let mut by_id = BTreeMap::new();
    for symbol in symbols {
        if by_id.insert(symbol.symbol_id.as_str(), symbol).is_some() {
            return Err(PortfolioHoldingSelectionError::new(
                PortfolioHoldingErrorCode::UnsupportedRequirement,
                format!("duplicate symbol id `{}`", symbol.symbol_id),
                None,
                Some(symbol.network_id.to_string()),
            ));
        }
    }
    Ok(by_id)
}

fn amount_dec_from_raw(
    raw_dec: &str,
    decimals: u8,
) -> Result<String, PortfolioHoldingSelectionError> {
    if raw_dec.is_empty() || !raw_dec.chars().all(|ch| ch.is_ascii_digit()) {
        return Err(PortfolioHoldingSelectionError::new(
            PortfolioHoldingErrorCode::MissingFact,
            format!("invalid raw balance decimal `{raw_dec}`"),
            None,
            None,
        ));
    }
    Ok(format_decimal_amount(raw_dec, decimals))
}

fn format_decimal_amount(raw_dec: &str, decimals: u8) -> String {
    let d = decimals as usize;
    if d == 0 {
        return raw_dec.to_owned();
    }
    if raw_dec.len() <= d {
        format!("0.{}{}", "0".repeat(d - raw_dec.len()), raw_dec)
    } else {
        let split = raw_dec.len() - d;
        format!("{}.{}", &raw_dec[..split], &raw_dec[split..])
    }
}

fn collect_report_quotes(snapshot: &PortfolioSnapshot) -> Vec<QuoteCode> {
    let mut quotes = BTreeSet::new();
    for symbol in &snapshot.symbol_configs {
        for quote in &symbol.valuation.quotes {
            quotes.insert(quote.quote);
        }
    }
    for wallet in &snapshot.wallets {
        for observation in &wallet.observations {
            for value in &observation.values {
                quotes.insert(value.quote);
            }
        }
    }
    quotes.into_iter().collect()
}

fn derive_quote_totals(
    report_quotes: &[QuoteCode],
    observations: &[Observation],
) -> StateResult<BTreeMap<QuoteCode, QuoteTotalsAccumulator>> {
    let mut totals = initialized_quote_totals(report_quotes);
    for observation in observations {
        for value in &observation.values {
            let entry = totals.entry(value.quote).or_default();
            let value_dec = DecimalValue::parse_non_negative(&value.value_dec)
                .map_err(|error| StateError::Message(error.to_string()))?;
            entry.total_value = entry.total_value.add(&value_dec);
        }
    }
    Ok(totals)
}

fn initialized_quote_totals(
    report_quotes: &[QuoteCode],
) -> BTreeMap<QuoteCode, QuoteTotalsAccumulator> {
    report_quotes
        .iter()
        .copied()
        .map(|quote| (quote, QuoteTotalsAccumulator::default()))
        .collect()
}

fn merge_quote_totals(
    target: &mut BTreeMap<QuoteCode, QuoteTotalsAccumulator>,
    source: &BTreeMap<QuoteCode, QuoteTotalsAccumulator>,
) {
    for (quote, totals) in source {
        target
            .entry(*quote)
            .and_modify(|acc| acc.merge(totals))
            .or_insert_with(|| totals.clone());
    }
}

fn quote_totals_to_vec(
    totals: BTreeMap<QuoteCode, QuoteTotalsAccumulator>,
) -> Vec<PortfolioQuoteTotal> {
    totals
        .into_iter()
        .map(|(quote, totals)| totals.into_report_total(quote))
        .collect()
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct QuoteTotalsAccumulator {
    total_value: DecimalValue,
}

impl QuoteTotalsAccumulator {
    fn merge(&mut self, other: &Self) {
        self.total_value = self.total_value.add(&other.total_value);
    }

    fn into_report_total(self, quote: QuoteCode) -> PortfolioQuoteTotal {
        PortfolioQuoteTotal {
            quote,
            total_value_dec: self.total_value.to_canonical_string(),
        }
    }
}
