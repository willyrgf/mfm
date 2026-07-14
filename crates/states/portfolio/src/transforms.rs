use super::*;

/// One required holding expanded from portfolio config + resolved subjects.
#[derive(Debug, Clone, PartialEq)]
pub struct RequiredHoldingRequirement {
    /// Selection key.
    pub key: RequiredHoldingKey,
    /// Cutover fact projection kind.
    pub projection: HoldingFactProjection,
    /// Wallet address for subject predicates.
    pub address: String,
    /// Symbol config for observation join.
    pub symbol: SymbolConfig,
    /// Network config for subject predicates.
    pub network: NetworkConfig,
}

/// Resolves configured wallets into typed subjects.
pub fn resolve_subjects_from_config(config: &ResolveSubjectsConfig) -> ResolvedSubjects {
    let mut subjects = config
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

/// Expands wallet×symbol requirements into cutover-supported holding requirements.
pub fn expand_required_holdings(
    config: &SelectHoldingsConfig,
    subjects: &ResolvedSubjects,
) -> Result<Vec<RequiredHoldingRequirement>, PortfolioHoldingSelectionError> {
    let networks = networks_by_id(&config.portfolio.networks)?;
    let symbols = symbols_by_id(&config.portfolio.symbol_configs)?;
    let subjects_by_wallet = subjects
        .subjects
        .iter()
        .map(|subject| (subject.wallet_id.as_str(), subject))
        .collect::<BTreeMap<_, _>>();

    let mut requirements = Vec::new();
    for wallet in &config.portfolio.wallets {
        let subject = subjects_by_wallet
            .get(wallet.wallet_id.as_str())
            .copied()
            .ok_or_else(|| {
                PortfolioHoldingSelectionError::new(
                    PortfolioHoldingErrorCode::MissingFact,
                    format!("missing resolved subject for wallet {}", wallet.wallet_id),
                    Some(wallet.wallet_id.to_string()),
                    Some(wallet.network_id.to_string()),
                )
            })?;
        for symbol_id in &wallet.symbol_ids {
            let symbol = symbols.get(symbol_id.as_str()).copied().ok_or_else(|| {
                PortfolioHoldingSelectionError::new(
                    PortfolioHoldingErrorCode::UnsupportedRequirement,
                    format!(
                        "wallet `{}` referenced unknown symbol `{symbol_id}`",
                        wallet.wallet_id
                    ),
                    Some(format!("{}/{}", wallet.wallet_id, symbol_id)),
                    Some(wallet.network_id.to_string()),
                )
            })?;
            let network = networks
                .get(symbol.network_id.as_str())
                .copied()
                .ok_or_else(|| {
                    PortfolioHoldingSelectionError::new(
                        PortfolioHoldingErrorCode::UnsupportedRequirement,
                        format!(
                            "missing network `{}` for symbol `{}`",
                            symbol.network_id, symbol.symbol_id
                        ),
                        Some(format!("{}/{}", wallet.wallet_id, symbol.symbol_id)),
                        Some(symbol.network_id.to_string()),
                    )
                })?;
            let family = match network.family() {
                NetworkFamilyConfig::Bitcoin => "bitcoin",
                NetworkFamilyConfig::Evm => "evm",
            };
            let is_native = matches!(
                (&symbol.kind, &symbol.balance_reader),
                (
                    SymbolKind::NativeBalance,
                    BalanceReaderConfig::NativeBalance {}
                )
            );
            if matches!(
                (&symbol.kind, &symbol.balance_reader),
                (
                    SymbolKind::Erc20Balance,
                    BalanceReaderConfig::Erc20Balance { .. }
                )
            ) {
                return Err(PortfolioHoldingSelectionError::new(
                    PortfolioHoldingErrorCode::UnsupportedRequirement,
                    format!(
                        "ERC-20 symbol `{}` is not supported at cutover",
                        symbol.symbol_id
                    ),
                    Some(format!("{}/{}", wallet.wallet_id, symbol.symbol_id)),
                    Some(symbol.network_id.to_string()),
                ));
            }
            let projection =
                project_holding_fact_for_network(family, is_native).map_err(|mut err| {
                    err.holding_key = Some(format!("{}/{}", wallet.wallet_id, symbol.symbol_id));
                    err.network_id = Some(symbol.network_id.to_string());
                    err
                })?;
            requirements.push(RequiredHoldingRequirement {
                key: RequiredHoldingKey {
                    wallet_id: wallet.wallet_id.to_string(),
                    symbol_id: symbol.symbol_id.to_string(),
                    network_id: symbol.network_id.to_string(),
                },
                projection,
                address: subject.address.clone(),
                symbol: symbol.clone(),
                network: network.clone(),
            });
        }
    }
    requirements.sort_by(|left, right| {
        (
            left.key.network_id.as_str(),
            left.key.wallet_id.as_str(),
            left.key.symbol_id.as_str(),
        )
            .cmp(&(
                right.key.network_id.as_str(),
                right.key.wallet_id.as_str(),
                right.key.symbol_id.as_str(),
            ))
    });
    Ok(requirements)
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
            kind: symbol.kind,
            role: symbol.role,
            network_id: item.material.network_id.clone(),
            protocol: symbol.protocol.as_ref().map(ToString::to_string),
            quantity: ObservationQuantity {
                raw_dec: item.material.raw_dec.clone(),
                decimals: item.material.decimals,
                amount_dec,
            },
            values: Vec::new(),
            source: ObservationSource {
                balance_reader_kind: item.material.balance_reader_kind.clone(),
                network_id: item.material.network_id.clone(),
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
    for symbol in &config.symbol_configs {
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

/// Hard-fail when any configured wallet×symbol required holding lacks an observation.
///
/// Defense-in-depth for the pure assemble path: SelectHoldings is the graph authority, but
/// assemble must not emit a successful snapshot with empty/partial required holdings.
fn require_required_holdings_present(
    portfolio: &PortfolioConfig,
    observations: &[Observation],
) -> Result<(), PortfolioHoldingSelectionError> {
    let present: BTreeSet<(String, String)> = observations
        .iter()
        .map(|observation| (observation.wallet_id.clone(), observation.symbol_id.clone()))
        .collect();
    for wallet in &portfolio.wallets {
        for symbol_id in &wallet.symbol_ids {
            let key = (wallet.wallet_id.to_string(), symbol_id.to_string());
            if !present.contains(&key) {
                return Err(PortfolioHoldingSelectionError::new(
                    PortfolioHoldingErrorCode::MissingFact,
                    format!(
                        "required holding missing observation for wallet `{}` symbol `{symbol_id}`",
                        wallet.wallet_id
                    ),
                    Some(format!("{}/{}", wallet.wallet_id, symbol_id)),
                    Some(wallet.network_id.to_string()),
                ));
            }
        }
    }
    Ok(())
}

/// Assembles the canonical portfolio snapshot (hard-fail; pins from selected observations).
pub fn assemble_snapshot(
    config: &AssembleSnapshotConfig,
    input: AssembleSnapshotInput,
    generated_at_ms: u64,
) -> StateResult<PortfolioSnapshot> {
    let symbols = symbols_by_id(&config.portfolio.symbol_configs).map_err(|error| {
        StateError::Message(format!(
            "{code}: {message}",
            code = error.code,
            message = error.message
        ))
    })?;
    let observations =
        apply_valuations_to_observations(input.holdings.observations, &input.valuations, &symbols)?;
    require_required_holdings_present(&config.portfolio, &observations).map_err(|error| {
        StateError::Message(format!(
            "{code}: {message}",
            code = error.code,
            message = error.message
        ))
    })?;
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
        schema_version: config.snapshot_version(),
        portfolio_id: config.portfolio.portfolio_id.to_string(),
        generated_at_ms,
        network_pins,
        wallets,
        symbol_configs,
    };
    snapshot.normalize();
    Ok(snapshot)
}

/// Projects a canonical portfolio report from a snapshot.
pub fn project_report_from_snapshot(
    snapshot: PortfolioSnapshot,
    report_version: u64,
) -> StateResult<PortfolioReport> {
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
        schema_version: report_version,
        portfolio_id: snapshot.portfolio_id,
        generated_at_ms: snapshot.generated_at_ms,
        network_pins: snapshot.network_pins,
        wallet_summaries,
        totals_by_quote: quote_totals_to_vec(portfolio_totals),
    };
    report.normalize();
    Ok(report)
}

/// Returns the canonical balance reader kind string.
pub fn balance_reader_kind(reader: &BalanceReaderConfig) -> &'static str {
    match reader {
        BalanceReaderConfig::NativeBalance {} => "native_balance",
        BalanceReaderConfig::Erc20Balance { .. } => "erc20_balance",
        BalanceReaderConfig::ProtocolPosition {
            protocol, reader, ..
        } if protocol.as_str() == AAVE_V3_PROTOCOL_ID => match reader.as_str() {
            "reserve_position" => "aave_v3/reserve_position",
            "debt_position" => "aave_v3/debt_position",
            _ => "protocol_position",
        },
        BalanceReaderConfig::ProtocolPosition { .. } => "protocol_position",
    }
}

/// Builds a symbols-by-id index for assemble / observation join.
pub fn symbols_by_id_map(
    symbols: &[SymbolConfig],
) -> Result<BTreeMap<&str, &SymbolConfig>, PortfolioHoldingSelectionError> {
    symbols_by_id(symbols)
}

fn networks_by_id(
    networks: &[NetworkConfig],
) -> Result<BTreeMap<&str, &NetworkConfig>, PortfolioHoldingSelectionError> {
    let mut by_id = BTreeMap::new();
    for network in networks {
        if by_id
            .insert(network.network_id().as_str(), network)
            .is_some()
        {
            return Err(PortfolioHoldingSelectionError::new(
                PortfolioHoldingErrorCode::UnsupportedRequirement,
                format!("duplicate network id `{}`", network.network_id()),
                None,
                Some(network.network_id().to_string()),
            ));
        }
    }
    Ok(by_id)
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
            let value_dec = DecimalValue::parse_signed(&value.value_dec)
                .map_err(|error| StateError::Message(error.to_string()))?;
            match observation.role {
                SymbolRole::Native | SymbolRole::Asset => {
                    entry.assets_value = entry.assets_value.add(&value_dec);
                }
                SymbolRole::Collateral => {
                    entry.collateral_value = entry.collateral_value.add(&value_dec);
                }
                SymbolRole::Debt => {
                    entry.debt_value = entry.debt_value.add(&value_dec);
                }
                SymbolRole::Staked => {
                    entry.staked_value = entry.staked_value.add(&value_dec);
                }
            }
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
    assets_value: DecimalValue,
    collateral_value: DecimalValue,
    debt_value: DecimalValue,
    staked_value: DecimalValue,
}

impl QuoteTotalsAccumulator {
    fn merge(&mut self, other: &Self) {
        self.assets_value = self.assets_value.add(&other.assets_value);
        self.collateral_value = self.collateral_value.add(&other.collateral_value);
        self.debt_value = self.debt_value.add(&other.debt_value);
        self.staked_value = self.staked_value.add(&other.staked_value);
    }

    fn into_report_total(self, quote: QuoteCode) -> PortfolioQuoteTotal {
        let positive_value = self
            .assets_value
            .add(&self.collateral_value)
            .add(&self.staked_value);
        let net_value = positive_value.sub(&self.debt_value);
        PortfolioQuoteTotal {
            quote,
            assets_value_dec: self.assets_value.to_canonical_string(),
            collateral_value_dec: self.collateral_value.to_canonical_string(),
            debt_value_dec: self.debt_value.to_canonical_string(),
            staked_value_dec: self.staked_value.to_canonical_string(),
            net_value_dec: net_value.to_canonical_string(),
        }
    }
}
