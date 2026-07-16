use super::*;
use mfm_portfolio_model::portfolio::ExecutionAnchor;
use mfm_portfolio_model::symbol::HoldingSourceConfig;

/// Resolves configured wallets into typed subjects.
pub fn resolve_subjects_from_config(config: &ResolveSubjectsConfig) -> ResolvedSubjects {
    let mut subjects = config
        .portfolio
        .wallets
        .iter()
        .map(|wallet| ResolvedSubject {
            wallet_id: wallet.wallet_id.to_string(),
            address: wallet.subject.address_str().to_owned(),
            subject_kind: wallet.subject.kind(),
            network_id: wallet.network_id.to_string(),
            implementation_kind: wallet_implementation_kind(&wallet.implementation).to_owned(),
        })
        .collect::<Vec<_>>();
    subjects.sort_by(|left, right| left.wallet_id.cmp(&right.wallet_id));
    ResolvedSubjects { subjects }
}

/// Builds quantity-only observations from selected holdings (valuation join deferred).
pub fn observations_from_selected_holdings(
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
            coverage: item.material.coverage.clone(),
            metadata: symbol.metadata.clone(),
        };
        observation.normalize();
        observations.push(observation);
    }
    Ok(observations)
}

/// Joins fixed unit-price valuations onto observations (hard-fail on missing route).
pub fn apply_valuations_to_observations(
    mut observations: Vec<Observation>,
    valuations: &ResolvedValuations,
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
        let mut values = Vec::new();
        for quote in &symbol.valuation.quotes {
            let resolved = valuations
                .valuations
                .iter()
                .find(|valuation| {
                    valuation.symbol_id == observation.symbol_id.as_str()
                        && valuation.quote == quote.quote
                })
                .ok_or_else(|| {
                    StateError::Message(format!(
                        "missing_fixed_unit_price: missing valuation for symbol `{}` quote `{}`",
                        observation.symbol_id, quote.quote
                    ))
                })?;
            let value_dec = multiply_decimal_strings(
                &observation.quantity.amount_dec,
                &resolved.unit_price_dec,
            )
            .map_err(|error| StateError::Message(error.to_string()))?;
            values.push(ObservationValue {
                quote: resolved.quote,
                priced_symbol_id: resolved.priced_symbol_id.clone(),
                value_dec,
                unit_price_dec: resolved.unit_price_dec.clone(),
            });
        }
        values.sort_by_key(|value| value.quote);
        observation.values = values;
        observation.normalize();
    }
    Ok(observations)
}

/// Resolves configured fixed unit-price valuation routes (hard-fail).
pub fn resolve_valuations_from_config(
    config: &ResolveValuationsConfig,
) -> StateResult<ResolvedValuations> {
    let mut valuations = Vec::new();
    for symbol in &config.portfolio.symbol_configs {
        for quote in &symbol.valuation.quotes {
            valuations.push(resolved_valuation_for_quote(symbol, quote)?);
        }
    }
    valuations.sort_by(|left, right| {
        (left.symbol_id.as_str(), left.quote).cmp(&(right.symbol_id.as_str(), right.quote))
    });
    Ok(ResolvedValuations { valuations })
}

fn resolved_valuation_for_quote(
    symbol: &SymbolConfig,
    quote: &QuoteValuationConfig,
) -> StateResult<ResolvedValuation> {
    Ok(ResolvedValuation {
        symbol_id: symbol.symbol_id.to_string(),
        quote: quote.quote,
        priced_symbol_id: quote.priced_symbol_id.to_string(),
        unit_price_dec: quote.unit_price_dec.to_string(),
    })
}

/// Proves that selected observations exactly realize the collection receipt.
///
/// Selection is the primary receipt consumer, but snapshot assembly independently rejects
/// missing, duplicated, unexpected, source-mismatched, or anchor-mismatched observations so a
/// substituted runner output cannot become a public snapshot.
fn require_receipt_holding_observations(
    receipt: &PortfolioCollectionReceipt,
    observations: &[Observation],
) -> Result<(), PortfolioHoldingSelectionError> {
    let expected = receipt
        .holdings()
        .iter()
        .map(|entry| (entry.requirement().clone(), entry))
        .collect::<BTreeMap<_, _>>();
    let mut actual = BTreeMap::new();
    for observation in observations {
        let key = HoldingRequirementKey {
            wallet_id: observation.wallet_id.clone(),
            symbol_id: observation.symbol_id.clone(),
            network_id: observation.network_id.clone(),
        };
        let Some(entry) = expected.get(&key).copied() else {
            return Err(receipt_observation_error(
                &key,
                "selected observations contained a holding absent from the collection receipt",
            ));
        };
        if actual.insert(key.clone(), observation).is_some() {
            return Err(receipt_observation_error(
                &key,
                "selected observations contained a duplicate collection receipt holding",
            ));
        }
        if !observation_source_matches_receipt(entry.source(), &observation.source.holding) {
            return Err(receipt_observation_error(
                &key,
                "selected observation source did not match the collection receipt",
            ));
        }
        if observation_execution_anchor(&observation.source.anchor) != *entry.anchor() {
            return Err(receipt_observation_error(
                &key,
                "selected observation anchor did not match the collection receipt",
            ));
        }
        if observation.coverage != entry.coverage() {
            return Err(receipt_observation_error(
                &key,
                "selected observation coverage did not match the collection receipt",
            ));
        }
    }
    for key in expected.keys() {
        if !actual.contains_key(key) {
            return Err(receipt_observation_error(
                key,
                "collection receipt holding was missing from selected observations",
            ));
        }
    }
    Ok(())
}

fn receipt_observation_error(
    key: &HoldingRequirementKey,
    message: impl Into<String>,
) -> PortfolioHoldingSelectionError {
    PortfolioHoldingSelectionError::new(
        PortfolioHoldingErrorCode::ReceiptMismatch,
        message,
        Some(key.as_key_str()),
        Some(key.network_id.clone()),
    )
}

fn observation_source_matches_receipt(
    source: &HoldingSourceKey,
    observation_source: &HoldingSourceConfig,
) -> bool {
    matches!(
        (source, observation_source),
        (
            HoldingSourceKey::BitcoinNative { .. },
            HoldingSourceConfig::Native
        ) | (
            HoldingSourceKey::EvmNative { .. },
            HoldingSourceConfig::Native
        )
    ) || matches!(
        (source, observation_source),
        (
            HoldingSourceKey::EvmErc20 {
                contract_address: expected,
                ..
            },
            HoldingSourceConfig::Erc20 { contract_address: actual }
        ) if expected == actual.as_str()
    )
}

fn observation_execution_anchor(anchor: &ObservationAnchor) -> ExecutionAnchor {
    match anchor {
        ObservationAnchor::Bitcoin { height, block_hash } => ExecutionAnchor::Bitcoin {
            height: *height,
            block_hash: block_hash.clone(),
        },
        ObservationAnchor::Evm {
            chain_id,
            block_number,
            block_hash,
        } => ExecutionAnchor::Evm {
            chain_id: *chain_id,
            block_number: *block_number,
            block_hash: block_hash.clone(),
        },
    }
}

/// Assembles the canonical portfolio snapshot (hard-fail; pins from selected observations).
pub fn assemble_snapshot(
    config: &AssembleSnapshotConfig,
    input: AssembleSnapshotInput,
) -> StateResult<PortfolioSnapshot> {
    validate_receipt_against_portfolio(&input.receipt, &config.portfolio).map_err(|error| {
        StateError::Message(format!(
            "{code}: {message}",
            code = error.code,
            message = error.message
        ))
    })?;
    require_receipt_holding_observations(&input.receipt, &input.holdings.observations).map_err(
        |error| {
            StateError::Message(format!(
                "{code}: {message}",
                code = error.code,
                message = error.message
            ))
        },
    )?;
    let symbols = symbols_by_id(&config.portfolio.symbol_configs).map_err(|error| {
        StateError::Message(format!(
            "{code}: {message}",
            code = error.code,
            message = error.message
        ))
    })?;
    let observations =
        apply_valuations_to_observations(input.holdings.observations, &input.valuations, &symbols)?;
    let network_pins = project_network_pins_from_observations(&observations).map_err(|error| {
        StateError::Message(format!(
            "{code}: {message}",
            code = error.code,
            message = error.message
        ))
    })?;
    if network_pins.as_slice() != input.receipt.network_anchors() {
        return Err(StateError::Message(
            "receipt_mismatch: selected observation pins did not equal collection receipt anchors"
                .to_owned(),
        ));
    }

    let mut observations_by_wallet: BTreeMap<String, Vec<Observation>> = BTreeMap::new();
    for observation in observations {
        observations_by_wallet
            .entry(observation.wallet_id.clone())
            .or_default()
            .push(observation);
    }
    let mut subjects_by_wallet = BTreeMap::new();
    for subject in input.subjects.subjects {
        subjects_by_wallet.insert(subject.wallet_id.clone(), subject);
    }

    let mut wallets = Vec::with_capacity(config.portfolio.wallets.len());
    for wallet_cfg in &config.portfolio.wallets {
        let subject = subjects_by_wallet
            .get(wallet_cfg.wallet_id.as_str())
            .cloned()
            .unwrap_or_else(|| ResolvedSubject {
                wallet_id: wallet_cfg.wallet_id.to_string(),
                address: wallet_cfg.subject.address_str().to_owned(),
                subject_kind: wallet_cfg.subject.kind(),
                network_id: wallet_cfg.network_id.to_string(),
                implementation_kind: wallet_implementation_kind(&wallet_cfg.implementation)
                    .to_owned(),
            });
        // Required holdings already verified; absent wallet key means zero symbols configured.
        let observations = observations_by_wallet
            .remove(wallet_cfg.wallet_id.as_str())
            .unwrap_or_default();
        let mut wallet = WalletSnapshot {
            wallet_id: wallet_cfg.wallet_id.to_string(),
            address: subject.address,
            subject_kind: subject.subject_kind,
            network_id: subject.network_id,
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

/// Projects the version-1 canonical portfolio report from a version-1 snapshot.
pub fn project_report_from_snapshot(snapshot: PortfolioSnapshot) -> StateResult<PortfolioReport> {
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
pub fn symbols_by_id_map(
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

fn wallet_implementation_kind(implementation: &WalletImplementationConfig) -> &'static str {
    match implementation {
        WalletImplementationConfig::AddressOnly {} => "address_only",
        WalletImplementationConfig::KeystoreEntry { .. } => "keystore_entry",
        WalletImplementationConfig::NodeManagedAccount { .. } => "node_managed_account",
        WalletImplementationConfig::ExternalSigner { .. } => "external_signer",
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
