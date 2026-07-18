//! State-owned fact-query plan, evidence, and reducer for portfolio holding selection.

use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_fact_capabilities::FactIndexReadRequest;
use mfm_facts::{
    compile_fact_query_plan, fact_query_evidence_hash, fact_query_result_rows_from_receipt,
    FactAudience, FactCanonicalScalar, FactFieldId, FactOrderingName, FactQueryEvidence,
    FactQueryInput, FactQueryOperator, FactQueryPredicate, FactQueryResult, FactQueryScope,
    FactSelectionEvidence, FactVisibilityScope, QueryResultCardinality, ScopeDecisionEvidence,
    StoreReadFrontier, StoreReadFrontierType, StoreScopeRef,
};
use mfm_portfolio_model::portfolio::ExecutionAnchor;
use mfm_portfolio_model::symbol::HoldingSourceConfig;
use mfm_program_derive::MfmValue;
use serde::{Deserialize, Serialize};

use crate::selection::{holding_candidate_from_bitcoin, BitcoinHoldingCandidateFields};
use crate::{
    observations_from_selected_holdings, portfolio_holding_select_scope_decision_hash,
    portfolio_holding_selection_policy_digest, project_network_pins_from_observations,
    symbols_by_id_map, validate_receipt_against_portfolio, CollectedHoldingReceipt,
    HoldingCandidate, HoldingSourceKey, PortfolioCollectionReceipt, PortfolioHoldingErrorCode,
    PortfolioHoldingSelectionError, SelectHoldingsConfig, SelectHoldingsFactDescriptors,
    SelectHoldingsInput, SelectedHolding, SelectedHoldings,
};

/// Complete deterministic fact-query plan for receipt-pinned portfolio selection.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, MfmValue)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "select_holdings_read_plan",
    schema = "mfm.portfolio.external_read.select_holdings.plan"
)]
pub struct SelectHoldingsReadPlan {
    portfolio: mfm_portfolio_model::portfolio::PortfolioConfig,
    store_scope: String,
    selection_policy_id: String,
    candidate_scan_limit: u64,
    fact_descriptors: SelectHoldingsFactDescriptors,
    receipt: PortfolioCollectionReceipt,
}

impl SelectHoldingsReadPlan {
    /// Creates a plan after validating the receipt against certified portfolio requirements.
    pub fn new(
        config: &SelectHoldingsConfig,
        input: &SelectHoldingsInput,
    ) -> Result<Self, PortfolioHoldingSelectionError> {
        validate_receipt_against_portfolio(&input.receipt, config.portfolio())?;
        let plan = Self {
            portfolio: config.portfolio().clone(),
            store_scope: config.store_scope().to_owned(),
            selection_policy_id: config.selection_policy_id().to_owned(),
            candidate_scan_limit: config.candidate_scan_limit(),
            fact_descriptors: config.fact_descriptors().clone(),
            receipt: input.receipt.clone(),
        };
        plan.requests()?;
        Ok(plan)
    }

    /// Returns the exact receipt whose holding facts must be loaded.
    pub const fn receipt(&self) -> &PortfolioCollectionReceipt {
        &self.receipt
    }

    /// Builds ordered, exact fact-index requests for every receipt holding.
    pub fn requests(&self) -> Result<Vec<FactIndexReadRequest>, PortfolioHoldingSelectionError> {
        self.receipt
            .holdings()
            .iter()
            .map(|entry| holding_fact_index_request(&self.config(), entry))
            .collect()
    }

    /// Builds auxiliary fact-query evidence using the state-owned selection policy.
    pub fn query_evidence(
        &self,
        responses: &[FactQueryResult],
        hydrated: &[Vec<PortfolioHoldingFactResponse>],
    ) -> Result<Vec<FactQueryEvidence>, PortfolioHoldingSelectionError> {
        let requests = self.requests()?;
        self.validate_result_shapes(responses, hydrated)?;
        requests
            .iter()
            .zip(responses)
            .zip(hydrated)
            .zip(self.receipt.holdings())
            .map(|(((request, response), material), entry)| {
                let (_, selected_index) = select_entry(entry, &self.config(), response, material)?;
                Ok(FactQueryEvidence::new(
                    request.plan().clone(),
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
        if responses.len() != self.receipt.holdings().len() {
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
            responses
                .iter()
                .map(|response| response.receipt().frontier_type()),
        )?;
        for (entry, response) in self.receipt.holdings().iter().zip(responses) {
            require_exact_bounded_cardinality(response, &self.config(), entry)?;
        }
        Ok(())
    }

    fn reduce(
        &self,
        evidence: &SelectHoldingsReadEvidence,
        queries: &[FactQueryEvidence],
    ) -> Result<SelectedHoldings, PortfolioHoldingSelectionError> {
        let requests = self.requests()?;
        let queries = order_query_evidence(&requests, queries, self.receipt.holdings())?;
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
        self.validate_result_shapes(&results, &evidence.hydrated_responses)?;

        let mut selected = Vec::with_capacity(requests.len());
        for ((((request, query), result), material), entry) in requests
            .iter()
            .zip(&queries)
            .zip(&results)
            .zip(&evidence.hydrated_responses)
            .zip(self.receipt.holdings())
        {
            if query.plan() != request.plan() {
                return Err(receipt_selection_error(
                    entry,
                    "fact-query evidence did not match its ordered state-authored request",
                ));
            }
            let (winner, selected_index) = select_entry(entry, &self.config(), result, material)?;
            let expected_selection = selection_evidence_for_index(selected_index)?;
            if query.selection() != &expected_selection {
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
        let pins = project_network_pins_from_observations(&observations)?;
        if pins.as_slice() != self.receipt.network_anchors() {
            return Err(selection_error(
                PortfolioHoldingErrorCode::ReceiptMismatch,
                "selected holding anchors did not equal the collection receipt anchors",
                None,
            ));
        }
        Ok(SelectedHoldings { observations })
    }

    fn validate_result_shapes(
        &self,
        responses: &[FactQueryResult],
        hydrated: &[Vec<PortfolioHoldingFactResponse>],
    ) -> Result<(), PortfolioHoldingSelectionError> {
        self.validate_query_results(responses)?;
        if hydrated.len() != responses.len() {
            return Err(selection_error(
                PortfolioHoldingErrorCode::ReceiptMismatch,
                "hydrated response count did not match receipt demand",
                None,
            ));
        }
        for ((entry, response), material) in
            self.receipt.holdings().iter().zip(responses).zip(hydrated)
        {
            if response.rows().len() != material.len() {
                return Err(receipt_selection_error(
                    entry,
                    "hydrated response material did not align with retained query rows",
                ));
            }
        }
        Ok(())
    }

    fn config(&self) -> SelectHoldingsConfig {
        SelectHoldingsConfig {
            portfolio: self.portfolio.clone(),
            store_scope: self.store_scope.clone(),
            selection_policy_id: self.selection_policy_id.clone(),
            candidate_scan_limit: std::num::NonZeroU64::new(self.candidate_scan_limit)
                .expect("validated portfolio read plans have a non-zero scan limit"),
            fact_descriptors: self.fact_descriptors.clone(),
        }
    }
}

fn order_query_evidence(
    requests: &[FactIndexReadRequest],
    queries: &[FactQueryEvidence],
    entries: &[CollectedHoldingReceipt],
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
            .filter_map(|(index, query)| (query.plan() == request.plan()).then_some(index))
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

/// Canonical hydrated response material for one portfolio holding fact row.
///
/// The canonical JSON string is checked against the configured fact descriptor and the retained
/// fact reference by the state reducer. Keeping the original response material avoids a second
/// family-specific wire schema in the portfolio crate.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "holding_fact_response",
    schema = "mfm.portfolio.external_read.holding_fact_response"
)]
pub struct PortfolioHoldingFactResponse {
    canonical_json: String,
}

impl PortfolioHoldingFactResponse {
    /// Admits one canonical retained fact response without interpreting its family schema.
    pub fn from_canonical_bytes(bytes: &[u8]) -> Result<Self, PortfolioHoldingSelectionError> {
        let canonical =
            PlainCanonicalJsonBytes::from_canonical_json_slice(bytes).map_err(|error| {
                selection_error(
                    PortfolioHoldingErrorCode::ReceiptMismatch,
                    error.to_string(),
                    None,
                )
            })?;
        let canonical_json = String::from_utf8(canonical.as_bytes().to_vec()).map_err(|error| {
            selection_error(
                PortfolioHoldingErrorCode::ReceiptMismatch,
                error.to_string(),
                None,
            )
        })?;
        Ok(Self { canonical_json })
    }

    fn value(
        &self,
        entry: &CollectedHoldingReceipt,
    ) -> Result<serde_json::Value, PortfolioHoldingSelectionError> {
        serde_json::from_str(&self.canonical_json)
            .map_err(|error| receipt_selection_error(entry, error.to_string()))
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
    hydrated_responses: Vec<Vec<PortfolioHoldingFactResponse>>,
}

impl SelectHoldingsReadEvidence {
    /// Creates primary evidence bound to ordered auxiliary query evidence.
    pub fn new(
        queries: &[FactQueryEvidence],
        hydrated_responses: Vec<Vec<PortfolioHoldingFactResponse>>,
    ) -> Result<Self, PortfolioHoldingSelectionError> {
        if queries.len() != hydrated_responses.len() {
            return Err(selection_error(
                PortfolioHoldingErrorCode::ReceiptMismatch,
                "query evidence and hydrated response counts differed",
                None,
            ));
        }
        let fact_query_evidence_hashes = query_hashes(queries)?;
        Ok(Self {
            fact_query_evidence_hashes,
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
    entry: &CollectedHoldingReceipt,
    config: &SelectHoldingsConfig,
    response: &FactQueryResult,
    hydrated: &[PortfolioHoldingFactResponse],
) -> Result<(SelectedHolding, usize), PortfolioHoldingSelectionError> {
    let symbol = config
        .portfolio()
        .symbol_configs
        .iter()
        .find(|symbol| symbol.symbol_id.as_str() == entry.requirement().symbol_id)
        .ok_or_else(|| {
            receipt_selection_error(
                entry,
                "receipt holding referenced a missing portfolio symbol",
            )
        })?;
    let mut matching = Vec::new();
    for (row_index, (row, material)) in response.rows().iter().zip(hydrated).enumerate() {
        let store_commit_order = store_commit_order_from_row(row).ok_or_else(|| {
            receipt_selection_error(
                entry,
                "receipt-pinned holding query omitted metadata.store_commit_order",
            )
        })?;
        let candidate = candidate_from_response(
            entry,
            config,
            symbol.source.clone(),
            row.fact_ref(),
            material,
            store_commit_order,
        )?;
        if let Some(candidate) = candidate {
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
            "no retained fact had the collection receipt's exact content identity",
            Some(entry),
        ));
    };
    Ok((
        SelectedHolding {
            key: entry.requirement().clone(),
            anchor: candidate.anchor,
            store_commit_order: candidate.store_commit_order,
            fact_claim_id: candidate.fact_claim_id,
            material: candidate.response_material,
        },
        selected_index,
    ))
}

fn candidate_from_response(
    entry: &CollectedHoldingReceipt,
    config: &SelectHoldingsConfig,
    holding: HoldingSourceConfig,
    fact_ref: &mfm_facts::InternalFactRef,
    material: &PortfolioHoldingFactResponse,
    store_commit_order: u64,
) -> Result<Option<HoldingCandidate>, PortfolioHoldingSelectionError> {
    let descriptor = config
        .fact_descriptors()
        .descriptor_for_source(entry.source())?;
    let subject = holding_subject_value(entry)?;
    let response = material.value(entry)?;
    let HoldingSourceKey::BitcoinNative { .. } = entry.source();
    let candidate = btc_candidate(entry, holding, fact_ref, &response, store_commit_order)?;
    if !identity_matches(entry, fact_ref, &descriptor, &subject, &response)? {
        return Ok(None);
    }
    Ok(candidate)
}

fn btc_candidate(
    entry: &CollectedHoldingReceipt,
    holding: HoldingSourceConfig,
    fact_ref: &mfm_facts::InternalFactRef,
    response: &serde_json::Value,
    store_commit_order: u64,
) -> Result<Option<HoldingCandidate>, PortfolioHoldingSelectionError> {
    let anchor_height = response_u64(entry, response, "anchor_height")?;
    let anchor_hash = response_string(entry, response, "anchor_hash")?;
    let balance_sats = response_u64(entry, response, "balance_sats")?;
    let coverage = response_string(entry, response, "coverage")?;
    let source_status = response_string(entry, response, "source_status")?;
    if !matches_btc_receipt(entry, anchor_height, anchor_hash, coverage, source_status) {
        return Err(receipt_selection_error(
            entry,
            "hydrated Bitcoin response did not match receipt anchor/status",
        ));
    }
    holding_candidate_from_bitcoin(
        entry.requirement(),
        store_commit_order,
        fact_ref.fact_claim_id().clone(),
        BitcoinHoldingCandidateFields {
            holding,
            raw_dec: balance_sats.to_string(),
            decimals: 8,
            height: anchor_height,
            block_hash: anchor_hash.to_owned(),
            coverage: coverage.to_owned(),
            source_status: source_status.to_owned(),
        },
    )
    .map(Some)
}

fn holding_subject_value(
    entry: &CollectedHoldingReceipt,
) -> Result<serde_json::Value, PortfolioHoldingSelectionError> {
    match entry.source() {
        HoldingSourceKey::BitcoinNative {
            network_id,
            bitcoin_network,
            semantic_source_identity,
            address,
        } => Ok(serde_json::json!({
            "address": address,
            "bitcoin_network": bitcoin_network,
            "network": network_id,
            "semantic_source_identity": semantic_source_identity,
        })),
    }
}

fn response_u64(
    entry: &CollectedHoldingReceipt,
    response: &serde_json::Value,
    field: &str,
) -> Result<u64, PortfolioHoldingSelectionError> {
    response
        .get(field)
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| {
            receipt_selection_error(entry, format!("response field {field} was not u64"))
        })
}

fn response_string<'a>(
    entry: &CollectedHoldingReceipt,
    response: &'a serde_json::Value,
    field: &str,
) -> Result<&'a str, PortfolioHoldingSelectionError> {
    response
        .get(field)
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            receipt_selection_error(entry, format!("response field {field} was not a string"))
        })
}

fn identity_matches<S, R>(
    entry: &CollectedHoldingReceipt,
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
        .fact_content_identity_evidence()
        .verify_against_typed_values(descriptor, subject, response)
        .map_err(|error| receipt_selection_error(entry, error.to_string()))?
        .is_some())
}

fn holding_fact_index_request(
    config: &SelectHoldingsConfig,
    entry: &CollectedHoldingReceipt,
) -> Result<FactIndexReadRequest, PortfolioHoldingSelectionError> {
    let store_scope = StoreScopeRef::new(config.store_scope())
        .map_err(|error| receipt_selection_error(entry, error.to_string()))?;
    let descriptor = config
        .fact_descriptors()
        .descriptor_for_source(entry.source())?;
    let (network, bitcoin_network, semantic_source_identity, address) =
        entry.source().bitcoin_native_parts();
    let ExecutionAnchor::Bitcoin { height, block_hash } = entry.anchor() else {
        return Err(receipt_selection_error(
            entry,
            "Bitcoin source did not carry a Bitcoin anchor",
        ));
    };
    let predicates = vec![
        query_eq_string(entry, "subject.network", network)?,
        query_eq_string(entry, "subject.bitcoin_network", bitcoin_network)?,
        query_eq_string(
            entry,
            "subject.semantic_source_identity",
            semantic_source_identity,
        )?,
        query_eq_string(entry, "subject.address", address)?,
        query_eq_u64(entry, "result.anchor_height", *height)?,
        query_eq_string(entry, "result.anchor_hash", block_hash)?,
        query_eq_string(entry, "result.coverage", entry.coverage())?,
        query_eq_string(entry, "result.source_status", entry.source_status())?,
    ];
    let return_fields = query_fields(
        entry,
        &[
            "result.anchor_height",
            "result.anchor_hash",
            "result.balance_sats",
            "result.coverage",
            "result.source_status",
            "metadata.store_commit_order",
        ],
    )?;
    let input = FactQueryInput::new(
        store_scope,
        FactQueryScope::new(FactAudience::Platform, FactVisibilityScope::Default),
        ScopeDecisionEvidence::new(portfolio_holding_select_scope_decision_hash()),
        predicates,
        return_fields,
        FactOrderingName::new("metadata.store_commit_order.desc")
            .map_err(|error| receipt_selection_error(entry, error.to_string()))?,
        Some(config.candidate_scan_limit()),
    )
    .map_err(|error| receipt_selection_error(entry, error.to_string()))?;
    let plan = compile_fact_query_plan(&descriptor, input)
        .map_err(|error| receipt_selection_error(entry, error.to_string()))?;
    FactIndexReadRequest::new(plan)
        .map_err(|error| receipt_selection_error(entry, error.to_string()))
}

fn query_field(
    entry: &CollectedHoldingReceipt,
    id: &str,
) -> Result<FactFieldId, PortfolioHoldingSelectionError> {
    FactFieldId::new(id).map_err(|error| receipt_selection_error(entry, error.to_string()))
}

fn query_fields(
    entry: &CollectedHoldingReceipt,
    ids: &[&str],
) -> Result<Vec<FactFieldId>, PortfolioHoldingSelectionError> {
    ids.iter().map(|id| query_field(entry, id)).collect()
}

fn query_eq_string(
    entry: &CollectedHoldingReceipt,
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
    entry: &CollectedHoldingReceipt,
    id: &str,
    value: u64,
) -> Result<FactQueryPredicate, PortfolioHoldingSelectionError> {
    Ok(FactQueryPredicate::new(
        query_field(entry, id)?,
        FactQueryOperator::Equal,
        FactCanonicalScalar::UnsignedInteger(value),
    ))
}

fn require_exact_bounded_cardinality(
    response: &FactQueryResult,
    config: &SelectHoldingsConfig,
    entry: &CollectedHoldingReceipt,
) -> Result<(), PortfolioHoldingSelectionError> {
    let rows = response.rows();
    match response.receipt().result_cardinality() {
        QueryResultCardinality::Exact(count)
            if count == rows.len() as u64 && count <= config.candidate_bound() =>
        {
            Ok(())
        }
        QueryResultCardinality::AtLeast(count) if count >= config.candidate_scan_limit() => {
            Err(selection_error(
                PortfolioHoldingErrorCode::CandidateBoundExhausted,
                "receipt-pinned fact query saturated before candidate exhaustion was proven",
                Some(entry),
            ))
        }
        _ => Err(receipt_selection_error(
            entry,
            "receipt-pinned fact query did not provide an exact bounded result set",
        )),
    }
}

fn require_shared_snapshot_read_frontier<'a>(
    frontiers: impl IntoIterator<Item = &'a StoreReadFrontier>,
    frontier_types: impl IntoIterator<Item = StoreReadFrontierType>,
) -> Result<(), PortfolioHoldingSelectionError> {
    let mut expected: Option<&StoreReadFrontier> = None;
    for (frontier, frontier_type) in frontiers.into_iter().zip(frontier_types) {
        if frontier_type != StoreReadFrontierType::Snapshot {
            return Err(selection_error(
                PortfolioHoldingErrorCode::ReceiptMismatch,
                "portfolio selection queries must use snapshot read frontiers",
                None,
            ));
        }
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

fn matches_btc_receipt(
    entry: &CollectedHoldingReceipt,
    anchor_height: u64,
    anchor_hash: &str,
    coverage: &str,
    source_status: &str,
) -> bool {
    matches!(
        entry.anchor(),
        ExecutionAnchor::Bitcoin { height, block_hash }
            if *height == anchor_height
                && block_hash == anchor_hash
                && entry.coverage() == coverage
                && entry.source_status() == source_status
    )
}

fn receipt_selection_error(
    entry: &CollectedHoldingReceipt,
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
    entry: Option<&CollectedHoldingReceipt>,
) -> PortfolioHoldingSelectionError {
    PortfolioHoldingSelectionError::new(
        code,
        message,
        entry.map(|entry| entry.requirement().as_key_str()),
        entry.map(|entry| entry.requirement().network_id.clone()),
    )
}

pub(crate) fn reduce_select_holdings(
    plan: &SelectHoldingsReadPlan,
    evidence: &SelectHoldingsReadEvidence,
    queries: &[FactQueryEvidence],
) -> Result<SelectedHoldings, PortfolioHoldingSelectionError> {
    plan.reduce(evidence, queries)
}
