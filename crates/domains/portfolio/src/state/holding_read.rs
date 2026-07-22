//! State-owned fact-query plan, evidence, and reducer for portfolio holding selection.

use crate::{ExecutionAnchor, HoldingSourceConfig, NetworkConfig, NetworkPin, PortfolioConfig};
use mfm_bitcoin::{
    decode_bitcoin_balance_snapshot_response, BitcoinBalanceCollectionReceipt,
    BitcoinBalanceSnapshotResponse,
};
use mfm_evm::{
    decode_evm_balance_snapshot_response, EvmBalanceAsset, EvmBalanceCollectionReceipt,
    EvmBalanceSnapshotResponse, EvmBalanceSource,
};
use mfm_facts::{
    compile_fact_query_plan, fact_query_evidence_hash, fact_query_result_rows_from_receipt,
    CanonicalFactQueryPlan, FactCanonicalScalar, FactContentIdentityEvidence, FactFieldId,
    FactKind, FactOrderingName, FactQueryEvidence, FactQueryInput, FactQueryOperator,
    FactQueryPredicate, FactQueryResult, FactSelectionEvidence, QueryResultCardinality,
    StoreReadFrontier,
};
use mfm_program_derive::MfmValue;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::num::NonZeroU64;

use super::selection::{holding_candidate_from_bitcoin, BitcoinHoldingCandidateFields};
use super::{
    observations_from_selected_holdings, portfolio_holding_selection_policy_digest,
    project_network_pins_from_observations, symbols_by_id_map, HoldingCandidate,
    HoldingRequirementKey, PortfolioHoldingErrorCode, PortfolioHoldingSelectionError,
    SelectHoldingsConfig, SelectHoldingsFactDescriptors, SelectHoldingsInput, SelectedHolding,
    SelectedHoldingMaterial, SelectedHoldings,
};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct BitcoinSourceKey {
    network_id: String,
    bitcoin_network: String,
    semantic_source_identity: String,
    address: String,
}

#[derive(Debug, Clone)]
enum ReceiptSource {
    Bitcoin {
        key: BitcoinSourceKey,
        anchor_height: u64,
        anchor_hash: String,
        fact_content_identity: FactContentIdentityEvidence,
    },
    Evm {
        network_id: String,
        chain_id: u64,
        source: EvmBalanceSource,
        block_anchor: mfm_evm::EvmBlockAnchor,
        fact_content_identity: FactContentIdentityEvidence,
    },
}

impl ReceiptSource {
    fn network_id(&self) -> &str {
        match self {
            Self::Bitcoin { key, .. } => &key.network_id,
            Self::Evm { network_id, .. } => network_id,
        }
    }

    fn anchor(&self) -> Result<ExecutionAnchor, PortfolioHoldingSelectionError> {
        match self {
            Self::Bitcoin {
                anchor_height,
                anchor_hash,
                ..
            } => Ok(ExecutionAnchor::Bitcoin {
                height: *anchor_height,
                block_hash: anchor_hash.clone(),
            }),
            Self::Evm {
                chain_id,
                block_anchor,
                ..
            } => Ok(ExecutionAnchor::Evm {
                chain_id: NonZeroU64::new(*chain_id).ok_or_else(|| {
                    selection_error(
                        PortfolioHoldingErrorCode::ReceiptMismatch,
                        "EVM receipt chain id was zero",
                        None,
                    )
                })?,
                block: block_anchor.clone(),
            }),
        }
    }

    fn fact_content_identity(&self) -> &FactContentIdentityEvidence {
        match self {
            Self::Bitcoin {
                fact_content_identity,
                ..
            }
            | Self::Evm {
                fact_content_identity,
                ..
            } => fact_content_identity,
        }
    }
}

#[derive(Debug, Clone)]
struct ReceiptHolding {
    requirement: HoldingRequirementKey,
    holding: HoldingSourceConfig,
    source: ReceiptSource,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum DemandedSourceKey {
    Bitcoin(BitcoinSourceKey),
    Evm {
        network_id: String,
        source: EvmBalanceSource,
    },
}

impl ReceiptHolding {
    fn anchor(&self) -> Result<ExecutionAnchor, PortfolioHoldingSelectionError> {
        self.source.anchor().map_err(|mut error| {
            error.holding_key = Some(self.requirement.as_key_str());
            error.network_id = Some(self.requirement.network_id.clone());
            error
        })
    }
}

/// Complete deterministic fact-query plan for receipt-pinned portfolio selection.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, MfmValue)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "select_holdings_read_plan",
    schema = "mfm.portfolio.external_read.select_holdings.plan"
)]
pub struct SelectHoldingsReadPlan {
    portfolio: PortfolioConfig,
    selection_policy_id: String,
    fact_descriptors: SelectHoldingsFactDescriptors,
    bitcoin_receipts: Vec<BitcoinBalanceCollectionReceipt>,
    evm_receipts: Vec<EvmBalanceCollectionReceipt>,
}

impl SelectHoldingsReadPlan {
    /// Creates a plan after validating both family receipt vectors against portfolio demand.
    pub fn new(
        config: &SelectHoldingsConfig,
        input: &SelectHoldingsInput,
    ) -> Result<Self, PortfolioHoldingSelectionError> {
        let plan = Self {
            portfolio: config.portfolio().clone(),
            selection_policy_id: config.selection_policy_id().to_owned(),
            fact_descriptors: config.fact_descriptors().clone(),
            bitcoin_receipts: input.bitcoin_receipts.clone(),
            evm_receipts: input.evm_receipts.clone(),
        };
        plan.receipt_holdings()?;
        plan.requests()?;
        Ok(plan)
    }

    /// Returns the exact number of receipt-authorized holdings to hydrate.
    pub fn holding_count(&self) -> Result<usize, PortfolioHoldingSelectionError> {
        Ok(self.receipt_holdings()?.len())
    }

    /// Builds ordered, exact fact-index requests for every receipt-authorized holding.
    pub fn requests(&self) -> Result<Vec<CanonicalFactQueryPlan>, PortfolioHoldingSelectionError> {
        self.receipt_holdings()?
            .iter()
            .map(|entry| holding_fact_index_request(&self.config(), entry))
            .collect()
    }

    /// Builds auxiliary fact-query evidence using the state-owned selection policy.
    pub fn query_evidence(
        &self,
        responses: &[FactQueryResult],
        hydrated: &[Vec<PortfolioHoldingFactEvidence>],
    ) -> Result<Vec<FactQueryEvidence>, PortfolioHoldingSelectionError> {
        let entries = self.receipt_holdings()?;
        let requests = self.requests()?;
        self.validate_result_shapes(&entries, responses, hydrated)?;
        requests
            .iter()
            .zip(responses)
            .zip(hydrated)
            .zip(&entries)
            .map(|(((request, response), material), entry)| {
                let (_, selected_index) = select_entry(entry, &self.config(), response, material)?;
                Ok(FactQueryEvidence::new(
                    request.clone(),
                    response.receipt().clone(),
                    selection_evidence_for_index(selected_index)?,
                ))
            })
            .collect()
    }

    /// Validates the all-query snapshot/cardinality barrier before artifact hydration.
    pub fn validate_query_results(
        &self,
        responses: &[FactQueryResult],
    ) -> Result<(), PortfolioHoldingSelectionError> {
        let entries = self.receipt_holdings()?;
        if responses.len() != entries.len() {
            return Err(selection_error(
                PortfolioHoldingErrorCode::ReceiptMismatch,
                "fact-index batch response count did not match receipt demand",
                None,
            ));
        }
        require_shared_snapshot_read_frontier(
            responses
                .iter()
                .map(|response| response.receipt().read_frontier()),
        )?;
        for (entry, response) in entries.iter().zip(responses) {
            require_identity_pinned_cardinality(response, entry)?;
        }
        Ok(())
    }

    fn reduce(
        &self,
        evidence: &SelectHoldingsReadEvidence,
        queries: &[FactQueryEvidence],
    ) -> Result<SelectedHoldings, PortfolioHoldingSelectionError> {
        let entries = self.receipt_holdings()?;
        let requests = self.requests()?;
        let queries = order_query_evidence(&requests, queries, &entries)?;
        evidence.validate_query_hashes(&queries)?;
        if evidence.hydrated_responses.len() != requests.len() {
            return Err(selection_error(
                PortfolioHoldingErrorCode::ReceiptMismatch,
                "portfolio selection evidence count did not match receipt demand",
                None,
            ));
        }

        let results = queries
            .iter()
            .map(|query| {
                mfm_facts::validate_fact_query_evidence(query).map_err(|error| {
                    selection_error(
                        PortfolioHoldingErrorCode::ReceiptMismatch,
                        error.to_string(),
                        None,
                    )
                })?;
                FactQueryResult::new(
                    fact_query_result_rows_from_receipt(query.receipt()),
                    query.receipt().clone(),
                )
                .map_err(|error| {
                    selection_error(
                        PortfolioHoldingErrorCode::ReceiptMismatch,
                        error.to_string(),
                        None,
                    )
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        self.validate_result_shapes(&entries, &results, &evidence.hydrated_responses)?;

        let mut selected = Vec::with_capacity(requests.len());
        for ((((request, query), result), material), entry) in requests
            .iter()
            .zip(&queries)
            .zip(&results)
            .zip(&evidence.hydrated_responses)
            .zip(&entries)
        {
            if query.plan() != request {
                return Err(receipt_selection_error(
                    entry,
                    "fact-query evidence did not match its ordered state-authored request",
                ));
            }
            let (winner, selected_index) = select_entry(entry, &self.config(), result, material)?;
            if query.selection() != &selection_evidence_for_index(selected_index)? {
                return Err(receipt_selection_error(
                    entry,
                    "fact-query selection did not match receipt identity and LWW policy",
                ));
            }
            selected.push(winner);
        }

        selected.sort_by(|left, right| left.key.cmp(&right.key));
        let symbols = symbols_by_id_map(&self.portfolio.symbol_configs)?;
        let observations = observations_from_selected_holdings(&selected, &symbols)?;
        if project_network_pins_from_observations(&observations)? != receipt_network_pins(&entries)?
        {
            return Err(selection_error(
                PortfolioHoldingErrorCode::ReceiptMismatch,
                "selected holding anchors did not equal the family receipt anchors",
                None,
            ));
        }
        Ok(SelectedHoldings { observations })
    }

    fn validate_result_shapes(
        &self,
        entries: &[ReceiptHolding],
        responses: &[FactQueryResult],
        hydrated: &[Vec<PortfolioHoldingFactEvidence>],
    ) -> Result<(), PortfolioHoldingSelectionError> {
        self.validate_query_results(responses)?;
        if hydrated.len() != responses.len() {
            return Err(selection_error(
                PortfolioHoldingErrorCode::ReceiptMismatch,
                "hydrated response count did not match receipt demand",
                None,
            ));
        }
        for ((entry, response), material) in entries.iter().zip(responses).zip(hydrated) {
            if response.rows().len() != material.len() {
                return Err(receipt_selection_error(
                    entry,
                    "hydrated response material did not align with retained query rows",
                ));
            }
        }
        Ok(())
    }

    fn receipt_holdings(&self) -> Result<Vec<ReceiptHolding>, PortfolioHoldingSelectionError> {
        receipt_holdings(&self.portfolio, &self.bitcoin_receipts, &self.evm_receipts)
    }

    fn config(&self) -> SelectHoldingsConfig {
        SelectHoldingsConfig {
            portfolio: self.portfolio.clone(),
            selection_policy_id: self.selection_policy_id.clone(),
            fact_descriptors: self.fact_descriptors.clone(),
        }
    }
}

fn receipt_holdings(
    portfolio: &PortfolioConfig,
    bitcoin_receipts: &[BitcoinBalanceCollectionReceipt],
    evm_receipts: &[EvmBalanceCollectionReceipt],
) -> Result<Vec<ReceiptHolding>, PortfolioHoldingSelectionError> {
    let mut bitcoin = BTreeMap::new();
    for receipt in bitcoin_receipts {
        if receipt.addresses().len() != receipt.fact_content_identities().len() {
            return Err(selection_error(
                PortfolioHoldingErrorCode::ReceiptMismatch,
                "Bitcoin receipt address and fact identity counts differed",
                None,
            ));
        }
        for (address, identity) in receipt
            .addresses()
            .iter()
            .zip(receipt.fact_content_identities())
        {
            let key = BitcoinSourceKey {
                network_id: receipt.network_id().to_owned(),
                bitcoin_network: receipt.bitcoin_network().to_owned(),
                semantic_source_identity: receipt.semantic_source_identity().to_owned(),
                address: address.clone(),
            };
            let source = ReceiptSource::Bitcoin {
                key: key.clone(),
                anchor_height: receipt.anchor_height(),
                anchor_hash: receipt.anchor_hash().to_owned(),
                fact_content_identity: identity.clone(),
            };
            if bitcoin.insert(key, source).is_some() {
                return Err(selection_error(
                    PortfolioHoldingErrorCode::ReceiptMismatch,
                    "Bitcoin receipts contained a duplicate source",
                    None,
                ));
            }
        }
    }

    let mut evm = BTreeMap::new();
    for receipt in evm_receipts {
        for (source, identity) in receipt
            .sources()
            .iter()
            .zip(receipt.fact_content_identities())
        {
            let key = (receipt.network_id().to_owned(), source.clone());
            let authority = ReceiptSource::Evm {
                network_id: receipt.network_id().to_owned(),
                chain_id: receipt.chain_id(),
                source: source.clone(),
                block_anchor: receipt.block_anchor().clone(),
                fact_content_identity: identity.clone(),
            };
            if evm.insert(key, authority).is_some() {
                return Err(selection_error(
                    PortfolioHoldingErrorCode::ReceiptMismatch,
                    "EVM receipts contained a duplicate source",
                    None,
                ));
            }
        }
    }

    let networks = portfolio
        .networks
        .iter()
        .map(|network| (network.network_id().as_str(), network))
        .collect::<BTreeMap<_, _>>();
    let symbols = portfolio
        .symbol_configs
        .iter()
        .map(|symbol| (symbol.symbol_id.as_str(), symbol))
        .collect::<BTreeMap<_, _>>();
    let mut seen_sources = BTreeSet::new();
    let mut holdings = Vec::new();
    for wallet in &portfolio.wallets {
        let network = networks
            .get(wallet.network_id.as_str())
            .copied()
            .ok_or_else(|| {
                selection_error(
                    PortfolioHoldingErrorCode::UnsupportedRequirement,
                    "portfolio wallet referenced an unknown network",
                    None,
                )
            })?;
        for symbol_id in &wallet.symbol_ids {
            let symbol = symbols.get(symbol_id.as_str()).copied().ok_or_else(|| {
                selection_error(
                    PortfolioHoldingErrorCode::UnsupportedRequirement,
                    "portfolio wallet referenced an unknown symbol",
                    None,
                )
            })?;
            let requirement = HoldingRequirementKey {
                wallet_id: wallet.wallet_id.to_string(),
                symbol_id: symbol.symbol_id.to_string(),
                network_id: wallet.network_id.to_string(),
            };
            let source = match (network, &symbol.source) {
                (
                    NetworkConfig::Bitcoin {
                        network_id,
                        bitcoin_network,
                        source_identity,
                        ..
                    },
                    HoldingSourceConfig::Native,
                ) => {
                    let key = BitcoinSourceKey {
                        network_id: network_id.to_string(),
                        bitcoin_network: bitcoin_network.clone(),
                        semantic_source_identity: source_identity.to_string(),
                        address: wallet.subject.address_str().to_owned(),
                    };
                    if !seen_sources.insert(DemandedSourceKey::Bitcoin(key.clone())) {
                        return Err(receipt_selection_error_message(
                            &requirement,
                            "portfolio demand aliased one Bitcoin receipt source",
                        ));
                    }
                    bitcoin.remove(&key).ok_or_else(|| {
                        missing_receipt_source(&requirement, "Bitcoin receipt source was missing")
                    })?
                }
                (NetworkConfig::Evm { chain_id, .. }, holding) => {
                    let account = wallet.subject.evm_address().ok_or_else(|| {
                        receipt_selection_error_message(
                            &requirement,
                            "EVM wallet did not contain an EVM address",
                        )
                    })?;
                    let account = account.to_address().map_err(|error| {
                        receipt_selection_error_message(&requirement, error.to_string())
                    })?;
                    let asset = match holding {
                        HoldingSourceConfig::Native => EvmBalanceAsset::Native,
                        HoldingSourceConfig::Erc20 { contract_address } => {
                            let contract_address =
                                contract_address.to_address().map_err(|error| {
                                    receipt_selection_error_message(&requirement, error.to_string())
                                })?;
                            EvmBalanceAsset::erc20(contract_address).map_err(|error| {
                                receipt_selection_error_message(&requirement, error.to_string())
                            })?
                        }
                    };
                    let balance_source =
                        EvmBalanceSource::new(account, asset).map_err(|error| {
                            receipt_selection_error_message(&requirement, error.to_string())
                        })?;
                    let source_key = DemandedSourceKey::Evm {
                        network_id: wallet.network_id.to_string(),
                        source: balance_source.clone(),
                    };
                    if !seen_sources.insert(source_key) {
                        return Err(receipt_selection_error_message(
                            &requirement,
                            "portfolio demand aliased one EVM receipt source",
                        ));
                    }
                    let source = evm
                        .remove(&(wallet.network_id.to_string(), balance_source))
                        .ok_or_else(|| {
                            missing_receipt_source(&requirement, "EVM receipt source was missing")
                        })?;
                    if !matches!(&source, ReceiptSource::Evm { chain_id: actual, .. } if *actual == chain_id.get())
                    {
                        return Err(receipt_selection_error_message(
                            &requirement,
                            "EVM receipt chain id did not match portfolio network authority",
                        ));
                    }
                    source
                }
                _ => {
                    return Err(receipt_selection_error_message(
                        &requirement,
                        "portfolio holding source did not match network family",
                    ));
                }
            };
            holdings.push(ReceiptHolding {
                requirement,
                holding: symbol.source.clone(),
                source,
            });
        }
    }
    if !bitcoin.is_empty() || !evm.is_empty() {
        return Err(selection_error(
            PortfolioHoldingErrorCode::ReceiptMismatch,
            "family receipts contained sources outside exact portfolio demand",
            None,
        ));
    }
    holdings.sort_by(|left, right| left.requirement.cmp(&right.requirement));
    Ok(holdings)
}

fn receipt_network_pins(
    entries: &[ReceiptHolding],
) -> Result<Vec<NetworkPin>, PortfolioHoldingSelectionError> {
    let mut anchors = BTreeMap::new();
    for entry in entries {
        let anchor = entry.anchor()?;
        match anchors.get(entry.source.network_id()) {
            None => {
                anchors.insert(entry.source.network_id().to_owned(), anchor);
            }
            Some(existing) if existing == &anchor => {}
            Some(_) => {
                return Err(receipt_selection_error(
                    entry,
                    "family receipts disagreed on one portfolio network anchor",
                ));
            }
        }
    }
    Ok(anchors
        .into_iter()
        .map(|(network_id, anchor)| NetworkPin { network_id, anchor })
        .collect())
}

fn order_query_evidence(
    requests: &[CanonicalFactQueryPlan],
    queries: &[FactQueryEvidence],
    entries: &[ReceiptHolding],
) -> Result<Vec<FactQueryEvidence>, PortfolioHoldingSelectionError> {
    if requests.len() != queries.len() || entries.len() != requests.len() {
        return Err(selection_error(
            PortfolioHoldingErrorCode::ReceiptMismatch,
            "portfolio selection query evidence count did not match receipt demand",
            None,
        ));
    }
    let mut remaining = queries.iter().collect::<Vec<_>>();
    let mut ordered = Vec::with_capacity(requests.len());
    for (request, entry) in requests.iter().zip(entries) {
        let matches = remaining
            .iter()
            .enumerate()
            .filter_map(|(index, query)| (query.plan() == request).then_some(index))
            .collect::<Vec<_>>();
        let [index] = matches.as_slice() else {
            return Err(receipt_selection_error(
                entry,
                "fact-query evidence was missing or duplicated for a state-authored request",
            ));
        };
        ordered.push((*remaining.remove(*index)).clone());
    }
    if !remaining.is_empty() {
        return Err(selection_error(
            PortfolioHoldingErrorCode::ReceiptMismatch,
            "portfolio selection carried unexpected fact-query evidence",
            None,
        ));
    }
    Ok(ordered)
}

/// Closed typed evidence hydrated from one retained source-domain fact response.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[serde(tag = "family", content = "response", rename_all = "snake_case")]
#[mfm(
    namespace = "mfm.portfolio",
    name = "holding_fact_evidence",
    schema = "mfm.portfolio.external_read.holding_fact_evidence"
)]
pub enum PortfolioHoldingFactEvidence {
    /// Checked aggregate Bitcoin balance response.
    Bitcoin(BitcoinBalanceSnapshotResponse),
    /// Checked EVM balance response.
    Evm(EvmBalanceSnapshotResponse),
}

impl PortfolioHoldingFactEvidence {
    /// Decodes canonical bytes through the source-domain decoder selected by the recorded kind.
    pub fn from_canonical_bytes(
        fact_kind: &FactKind,
        bytes: &[u8],
    ) -> Result<Self, PortfolioHoldingSelectionError> {
        match fact_kind.as_str() {
            "bitcoin.balance_snapshot" => decode_bitcoin_balance_snapshot_response(bytes)
                .map(Self::Bitcoin)
                .map_err(|_| {
                    selection_error(
                        PortfolioHoldingErrorCode::ReceiptMismatch,
                        "retained Bitcoin balance response was invalid",
                        None,
                    )
                }),
            "evm.balance_snapshot" => decode_evm_balance_snapshot_response(bytes)
                .map(Self::Evm)
                .map_err(|_| {
                    selection_error(
                        PortfolioHoldingErrorCode::ReceiptMismatch,
                        "retained EVM balance response was invalid",
                        None,
                    )
                }),
            _ => Err(selection_error(
                PortfolioHoldingErrorCode::ReceiptMismatch,
                "retained holding fact kind was unsupported",
                None,
            )),
        }
    }
}

/// Primary retained evidence binding query receipts to hydrated holding response material.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "select_holdings_read_evidence",
    schema = "mfm.portfolio.external_read.select_holdings.evidence"
)]
pub struct SelectHoldingsReadEvidence {
    fact_query_evidence_hashes: Vec<String>,
    hydrated_responses: Vec<Vec<PortfolioHoldingFactEvidence>>,
}

impl SelectHoldingsReadEvidence {
    /// Creates primary evidence bound to ordered auxiliary query evidence.
    pub fn new(
        queries: &[FactQueryEvidence],
        hydrated_responses: Vec<Vec<PortfolioHoldingFactEvidence>>,
    ) -> Result<Self, PortfolioHoldingSelectionError> {
        if queries.len() != hydrated_responses.len() {
            return Err(selection_error(
                PortfolioHoldingErrorCode::ReceiptMismatch,
                "query evidence and hydrated response counts differed",
                None,
            ));
        }
        Ok(Self {
            fact_query_evidence_hashes: query_hashes(queries)?,
            hydrated_responses,
        })
    }

    fn validate_query_hashes(
        &self,
        queries: &[FactQueryEvidence],
    ) -> Result<(), PortfolioHoldingSelectionError> {
        if self.fact_query_evidence_hashes != query_hashes(queries)? {
            return Err(selection_error(
                PortfolioHoldingErrorCode::ReceiptMismatch,
                "primary evidence did not bind the ordered fact-query evidence",
                None,
            ));
        }
        Ok(())
    }
}

fn query_hashes(
    queries: &[FactQueryEvidence],
) -> Result<Vec<String>, PortfolioHoldingSelectionError> {
    queries
        .iter()
        .map(|query| {
            fact_query_evidence_hash(query)
                .map(|hash| hash.as_str().to_owned())
                .map_err(|error| {
                    selection_error(
                        PortfolioHoldingErrorCode::ReceiptMismatch,
                        error.to_string(),
                        None,
                    )
                })
        })
        .collect()
}

fn select_entry(
    entry: &ReceiptHolding,
    config: &SelectHoldingsConfig,
    response: &FactQueryResult,
    hydrated: &[PortfolioHoldingFactEvidence],
) -> Result<(SelectedHolding, usize), PortfolioHoldingSelectionError> {
    let mut matching = Vec::new();
    for (row_index, (row, material)) in response.rows().iter().zip(hydrated).enumerate() {
        let store_commit_order = store_commit_order_from_row(row).ok_or_else(|| {
            receipt_selection_error(
                entry,
                "receipt-pinned holding query omitted metadata.store_commit_order",
            )
        })?;
        if let Some(candidate) =
            candidate_from_response(entry, config, row.fact_ref(), material, store_commit_order)?
        {
            matching.push((row_index, candidate));
        }
    }
    matching.sort_by(|left, right| {
        right
            .1
            .store_commit_order
            .cmp(&left.1.store_commit_order)
            .then_with(|| right.1.fact_claim_id.cmp(&left.1.fact_claim_id))
    });
    let Some((selected_index, candidate)) = matching.into_iter().next() else {
        return Err(selection_error(
            PortfolioHoldingErrorCode::MissingFact,
            "no retained fact had the family receipt's exact content identity",
            Some(entry),
        ));
    };
    Ok((
        SelectedHolding {
            key: entry.requirement.clone(),
            anchor: candidate.anchor,
            store_commit_order: candidate.store_commit_order,
            fact_claim_id: candidate.fact_claim_id,
            material: candidate.response_material,
        },
        selected_index,
    ))
}

fn candidate_from_response(
    entry: &ReceiptHolding,
    config: &SelectHoldingsConfig,
    fact_ref: &mfm_facts::InternalFactRef,
    material: &PortfolioHoldingFactEvidence,
    store_commit_order: u64,
) -> Result<Option<HoldingCandidate>, PortfolioHoldingSelectionError> {
    let descriptor = match &entry.source {
        ReceiptSource::Bitcoin { .. } => config.fact_descriptors().bitcoin_native()?,
        ReceiptSource::Evm { .. } => config.fact_descriptors().evm_balance()?,
    };
    let subject = holding_subject_value(entry)?;
    let identity_matches = match material {
        PortfolioHoldingFactEvidence::Bitcoin(response) => {
            matches!(&entry.source, ReceiptSource::Bitcoin { .. })
                && identity_matches(entry, fact_ref, &descriptor, &subject, response)?
        }
        PortfolioHoldingFactEvidence::Evm(response) => {
            matches!(&entry.source, ReceiptSource::Evm { .. })
                && identity_matches(entry, fact_ref, &descriptor, &subject, response)?
        }
    };
    if !identity_matches {
        return Ok(None);
    }
    let candidate = match material {
        PortfolioHoldingFactEvidence::Bitcoin(response) => {
            btc_candidate(entry, fact_ref, response, store_commit_order)?
        }
        PortfolioHoldingFactEvidence::Evm(response) => {
            evm_candidate(entry, fact_ref, response, store_commit_order)?
        }
    };
    Ok(Some(candidate))
}

fn btc_candidate(
    entry: &ReceiptHolding,
    fact_ref: &mfm_facts::InternalFactRef,
    response: &BitcoinBalanceSnapshotResponse,
    store_commit_order: u64,
) -> Result<HoldingCandidate, PortfolioHoldingSelectionError> {
    let anchor_height = response.anchor_height();
    let anchor_hash = response.anchor_hash();
    let balance_sats = response.balance_sats();
    let ReceiptSource::Bitcoin {
        anchor_height: expected_height,
        anchor_hash: expected_hash,
        ..
    } = &entry.source
    else {
        unreachable!("Bitcoin candidate requires Bitcoin receipt authority")
    };
    if anchor_height != *expected_height || anchor_hash != expected_hash {
        return Err(receipt_selection_error(
            entry,
            "hydrated Bitcoin response did not match the receipt anchor",
        ));
    }
    holding_candidate_from_bitcoin(
        &entry.requirement,
        store_commit_order,
        fact_ref.fact_claim_id().clone(),
        BitcoinHoldingCandidateFields {
            holding: entry.holding.clone(),
            raw_dec: balance_sats.to_string(),
            decimals: 8,
            height: anchor_height,
            block_hash: anchor_hash.to_owned(),
        },
    )
}

fn evm_candidate(
    entry: &ReceiptHolding,
    fact_ref: &mfm_facts::InternalFactRef,
    response: &EvmBalanceSnapshotResponse,
    store_commit_order: u64,
) -> Result<HoldingCandidate, PortfolioHoldingSelectionError> {
    let ReceiptSource::Evm {
        chain_id,
        block_anchor,
        ..
    } = &entry.source
    else {
        unreachable!("EVM candidate requires EVM receipt authority")
    };
    if response.block_anchor() != block_anchor {
        return Err(receipt_selection_error(
            entry,
            "hydrated EVM response did not match receipt block anchor",
        ));
    }
    let raw_units = response.raw_units();
    let decimals = response.decimals();
    let anchor = ExecutionAnchor::Evm {
        chain_id: NonZeroU64::new(*chain_id)
            .ok_or_else(|| receipt_selection_error(entry, "EVM receipt chain id was zero"))?,
        block: block_anchor.clone(),
    };
    Ok(HoldingCandidate {
        network_id: entry.requirement.network_id.clone(),
        anchor: anchor.clone(),
        store_commit_order,
        fact_claim_id: fact_ref.fact_claim_id().clone(),
        response_material: SelectedHoldingMaterial {
            wallet_id: entry.requirement.wallet_id.clone(),
            symbol_id: entry.requirement.symbol_id.clone(),
            network_id: entry.requirement.network_id.clone(),
            holding: entry.holding.clone(),
            raw_dec: raw_units.to_owned(),
            decimals,
            observation_anchor: anchor,
        },
    })
}

fn holding_subject_value(
    entry: &ReceiptHolding,
) -> Result<serde_json::Value, PortfolioHoldingSelectionError> {
    match &entry.source {
        ReceiptSource::Bitcoin { key, .. } => Ok(serde_json::json!({
            "address": key.address,
            "bitcoin_network": key.bitcoin_network,
            "network_id": key.network_id,
            "semantic_source_identity": key.semantic_source_identity,
        })),
        ReceiptSource::Evm {
            network_id,
            chain_id,
            source,
            ..
        } => Ok(serde_json::json!({
            "network_id": network_id,
            "chain_id": chain_id,
            "account": source.account(),
            "asset": source.asset(),
        })),
    }
}

fn identity_matches<S, R>(
    entry: &ReceiptHolding,
    fact_ref: &mfm_facts::InternalFactRef,
    descriptor: &mfm_facts::FactDescriptor,
    subject: &S,
    response: &R,
) -> Result<bool, PortfolioHoldingSelectionError>
where
    S: Serialize,
    R: Serialize,
{
    let identity =
        mfm_facts::derive_fact_content_identity_from_typed_values(descriptor, subject, response)
            .map_err(|error| receipt_selection_error(entry, error.to_string()))?;
    if fact_ref.fact_descriptor_hash() != identity.fact_descriptor_hash()
        || fact_ref.subject_material_hash() != identity.subject_material_hash()
        || fact_ref.response_schema_id() != identity.response_schema_id()
        || fact_ref.response_hash() != identity.response_hash()
    {
        return Err(receipt_selection_error(
            entry,
            "fact reference did not match descriptor-checked hydrated content",
        ));
    }
    Ok(entry
        .source
        .fact_content_identity()
        .verify_against_typed_values(descriptor, subject, response)
        .map_err(|error| receipt_selection_error(entry, error.to_string()))?
        .is_some())
}

fn holding_fact_index_request(
    config: &SelectHoldingsConfig,
    entry: &ReceiptHolding,
) -> Result<CanonicalFactQueryPlan, PortfolioHoldingSelectionError> {
    let (descriptor, predicates, return_fields) = match &entry.source {
        ReceiptSource::Bitcoin {
            key,
            anchor_height,
            anchor_hash,
            ..
        } => (
            config.fact_descriptors().bitcoin_native()?,
            vec![
                query_eq_string(entry, "subject.network_id", &key.network_id)?,
                query_eq_string(entry, "subject.bitcoin_network", &key.bitcoin_network)?,
                query_eq_string(
                    entry,
                    "subject.semantic_source_identity",
                    &key.semantic_source_identity,
                )?,
                query_eq_string(entry, "subject.address", &key.address)?,
                query_eq_u64(entry, "result.anchor_height", *anchor_height)?,
                query_eq_string(entry, "result.anchor_hash", anchor_hash)?,
            ],
            query_fields(
                entry,
                &[
                    "result.anchor_height",
                    "result.anchor_hash",
                    "result.balance_sats",
                    "metadata.store_commit_order",
                ],
            )?,
        ),
        ReceiptSource::Evm {
            network_id,
            chain_id,
            source,
            block_anchor,
            ..
        } => {
            let mut predicates = vec![
                query_eq_string(entry, "subject.network_id", network_id)?,
                query_eq_u64(entry, "subject.chain_id", *chain_id)?,
                query_eq_string(entry, "subject.account", source.account())?,
                query_eq_string(
                    entry,
                    "subject.asset.kind",
                    match source.asset() {
                        EvmBalanceAsset::Native => "native",
                        EvmBalanceAsset::Erc20 { .. } => "erc20",
                    },
                )?,
                query_eq_string(entry, "result.block_anchor.number", block_anchor.number())?,
                query_eq_string(entry, "result.block_anchor.hash", block_anchor.hash())?,
            ];
            if let EvmBalanceAsset::Erc20 { contract_address } = source.asset() {
                predicates.push(query_eq_string(
                    entry,
                    "subject.asset.contract_address",
                    contract_address.as_str(),
                )?);
            }
            (
                config.fact_descriptors().evm_balance()?,
                predicates,
                query_fields(
                    entry,
                    &[
                        "result.block_anchor.number",
                        "result.block_anchor.hash",
                        "result.raw_units",
                        "result.decimals",
                        "metadata.store_commit_order",
                    ],
                )?,
            )
        }
    };
    let input = FactQueryInput::new(
        predicates,
        return_fields,
        FactOrderingName::new("metadata.store_commit_order.desc")
            .map_err(|error| receipt_selection_error(entry, error.to_string()))?,
        Some(1),
    )
    .map_err(|error| receipt_selection_error(entry, error.to_string()))?
    .with_content_identity(entry.source.fact_content_identity().clone());
    let plan = compile_fact_query_plan(&descriptor, input)
        .map_err(|error| receipt_selection_error(entry, error.to_string()))?;
    Ok(plan)
}

fn query_field(
    entry: &ReceiptHolding,
    id: &str,
) -> Result<FactFieldId, PortfolioHoldingSelectionError> {
    FactFieldId::new(id).map_err(|error| receipt_selection_error(entry, error.to_string()))
}

fn query_fields(
    entry: &ReceiptHolding,
    ids: &[&str],
) -> Result<Vec<FactFieldId>, PortfolioHoldingSelectionError> {
    ids.iter().map(|id| query_field(entry, id)).collect()
}

fn query_eq_string(
    entry: &ReceiptHolding,
    id: &str,
    value: &str,
) -> Result<FactQueryPredicate, PortfolioHoldingSelectionError> {
    Ok(FactQueryPredicate::new(
        query_field(entry, id)?,
        FactQueryOperator::Equal,
        FactCanonicalScalar::string(value),
    ))
}

fn query_eq_u64(
    entry: &ReceiptHolding,
    id: &str,
    value: u64,
) -> Result<FactQueryPredicate, PortfolioHoldingSelectionError> {
    Ok(FactQueryPredicate::new(
        query_field(entry, id)?,
        FactQueryOperator::Equal,
        FactCanonicalScalar::UnsignedInteger(value),
    ))
}

fn require_identity_pinned_cardinality(
    response: &FactQueryResult,
    entry: &ReceiptHolding,
) -> Result<(), PortfolioHoldingSelectionError> {
    let rows = response.rows();
    match (response.receipt().result_cardinality(), rows.len()) {
        (QueryResultCardinality::Exact(0), 0) => Err(selection_error(
            PortfolioHoldingErrorCode::MissingFact,
            "the receipt-pinned fact content was absent from the store snapshot",
            Some(entry),
        )),
        (QueryResultCardinality::Exact(1) | QueryResultCardinality::AtLeast(1), 1)
            if entry
                .source
                .fact_content_identity()
                .matches_internal_ref(rows[0].fact_ref()) =>
        {
            Ok(())
        }
        _ => Err(receipt_selection_error(
            entry,
            "receipt-pinned fact query did not provide one bounded content-identity match",
        )),
    }
}

fn require_shared_snapshot_read_frontier<'a>(
    frontiers: impl IntoIterator<Item = &'a StoreReadFrontier>,
) -> Result<(), PortfolioHoldingSelectionError> {
    let mut expected: Option<&StoreReadFrontier> = None;
    for frontier in frontiers {
        match expected {
            None => expected = Some(frontier),
            Some(shared) if shared != frontier => {
                return Err(selection_error(
                    PortfolioHoldingErrorCode::ReceiptMismatch,
                    "portfolio selection queries use mixed read frontiers",
                    None,
                ));
            }
            Some(_) => {}
        }
    }
    Ok(())
}

fn selection_evidence_for_index(
    selected_index: usize,
) -> Result<FactSelectionEvidence, PortfolioHoldingSelectionError> {
    let selected_index = u64::try_from(selected_index).map_err(|_| {
        selection_error(
            PortfolioHoldingErrorCode::AmbiguousFacts,
            "selected fact query row index overflowed u64",
            None,
        )
    })?;
    FactSelectionEvidence::new(
        portfolio_holding_selection_policy_digest(),
        vec![selected_index],
        None,
    )
    .map_err(|error| {
        selection_error(
            PortfolioHoldingErrorCode::AmbiguousFacts,
            error.to_string(),
            None,
        )
    })
}

fn store_commit_order_from_row(row: &mfm_facts::FactQueryResultRow) -> Option<u64> {
    row.returned_fields().iter().find_map(|field| {
        (field.field_id().as_str() == "metadata.store_commit_order").then(|| {
            match field.value() {
                FactCanonicalScalar::UnsignedInteger(value) => Some(*value),
                _ => None,
            }
        })?
    })
}

fn missing_receipt_source(
    requirement: &HoldingRequirementKey,
    message: impl Into<String>,
) -> PortfolioHoldingSelectionError {
    PortfolioHoldingSelectionError::new(
        PortfolioHoldingErrorCode::MissingFact,
        message,
        Some(requirement.as_key_str()),
        Some(requirement.network_id.clone()),
    )
}

fn receipt_selection_error_message(
    requirement: &HoldingRequirementKey,
    message: impl Into<String>,
) -> PortfolioHoldingSelectionError {
    PortfolioHoldingSelectionError::new(
        PortfolioHoldingErrorCode::ReceiptMismatch,
        message,
        Some(requirement.as_key_str()),
        Some(requirement.network_id.clone()),
    )
}

fn receipt_selection_error(
    entry: &ReceiptHolding,
    message: impl Into<String>,
) -> PortfolioHoldingSelectionError {
    selection_error(
        PortfolioHoldingErrorCode::ReceiptMismatch,
        message,
        Some(entry),
    )
}

fn selection_error(
    code: PortfolioHoldingErrorCode,
    message: impl Into<String>,
    entry: Option<&ReceiptHolding>,
) -> PortfolioHoldingSelectionError {
    PortfolioHoldingSelectionError::new(
        code,
        message,
        entry.map(|entry| entry.requirement.as_key_str()),
        entry.map(|entry| entry.requirement.network_id.clone()),
    )
}

pub(crate) fn reduce_select_holdings(
    plan: &SelectHoldingsReadPlan,
    evidence: &SelectHoldingsReadEvidence,
    queries: &[FactQueryEvidence],
) -> Result<SelectedHoldings, PortfolioHoldingSelectionError> {
    plan.reduce(evidence, queries)
}
