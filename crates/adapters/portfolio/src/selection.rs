use super::*;

pub(crate) async fn select_holdings(
    config: ValidatedConfig<SelectHoldingsConfig>,
    subjects: mfm_state_portfolio::ResolvedSubjects,
    artifacts: &dyn store::RetainedArtifactReadProvider,
    fact_index: &dyn FactIndexReadProvider,
) -> mfm_runtime::Result<(SelectedHoldings, Vec<FactQueryEvidence>)> {
    let state = SelectHoldingsState::new(config)
        .map_err(|error| mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string()))?;
    let config = state.config();
    let requirements =
        expand_required_holdings(config, &subjects).map_err(holding_runtime_error)?;

    let mut candidates_by_holding: BTreeMap<RequiredHoldingKey, Vec<HoldingCandidate>> =
        BTreeMap::new();
    let mut per_holding_claim_ids: BTreeMap<RequiredHoldingKey, Vec<String>> = BTreeMap::new();
    let mut request_response: Vec<(
        RequiredHoldingKey,
        mfm_fact_capabilities::FactIndexReadRequest,
        mfm_facts::FactQueryResult,
    )> = Vec::new();

    // One shared fact-index snapshot for all required holdings (single selection frontier).
    let mut requests = Vec::with_capacity(requirements.len());
    for requirement in &requirements {
        requests
            .push(holding_fact_index_request(config, requirement).map_err(holding_runtime_error)?);
    }
    let responses = fact_index
        .read_fact_index_batch(&requests)
        .await
        .map_err(fact_index_runtime_error)?;
    if responses.len() != requests.len() {
        return Err(mfm_runtime::RuntimeError::InvalidRunnerOutput(
            "fact-index batch response count does not match request count".to_owned(),
        ));
    }
    require_shared_snapshot_read_frontier(
        responses
            .iter()
            .map(|response| response.receipt().read_frontier()),
        responses
            .iter()
            .map(|response| response.receipt().frontier_type()),
    )
    .map_err(|message| {
        mfm_runtime::RuntimeError::InvalidRunnerOutput(format!("runner_output_invalid: {message}"))
    })?;
    for ((requirement, request), response) in requirements.iter().zip(requests).zip(responses) {
        let (candidates, claim_ids) =
            candidates_for_requirement(requirement, &response, artifacts).await?;
        candidates_by_holding.insert(requirement.key.clone(), candidates);
        per_holding_claim_ids.insert(requirement.key.clone(), claim_ids);
        request_response.push((requirement.key.clone(), request, response));
    }

    // Ensure every required holding key is present (even if empty) for pure select.
    for requirement in &requirements {
        candidates_by_holding
            .entry(requirement.key.clone())
            .or_default();
    }

    let selected =
        select_network_coherent(&candidates_by_holding).map_err(holding_runtime_error)?;
    let selected_claim_ids: BTreeSet<String> = selected
        .iter()
        .map(|item| fact_claim_id_string(&item.fact_claim_id))
        .collect();

    let symbols =
        symbols_by_id_map(&config.portfolio().symbol_configs).map_err(holding_runtime_error)?;
    let observations =
        observations_from_selected_holdings(&selected, &symbols).map_err(holding_runtime_error)?;
    // Guard: residual pin projection consistency (also enforced at assemble).
    let _ = project_network_pins_from_observations(&observations).map_err(holding_runtime_error)?;

    let mut evidences = Vec::with_capacity(request_response.len());
    for (key, request, response) in request_response {
        let row_claim_ids = per_holding_claim_ids.get(&key).cloned().unwrap_or_default();
        let selection = state
            .selection_evidence_for_claims(&row_claim_ids, &selected_claim_ids)
            .map_err(holding_runtime_error)?;
        let evidence = FactQueryEvidence::new(
            request.plan().clone(),
            response.receipt().clone(),
            selection,
        );
        evidences.push(evidence);
    }

    Ok((SelectedHoldings { observations }, evidences))
}

pub(crate) async fn candidates_for_requirement(
    requirement: &RequiredHoldingRequirement,
    response: &mfm_facts::FactQueryResult,
    artifacts: &dyn store::RetainedArtifactReadProvider,
) -> mfm_runtime::Result<(Vec<HoldingCandidate>, Vec<String>)> {
    let mut candidates = Vec::new();
    let mut claim_ids = Vec::new();
    for row in response.rows() {
        let fact_ref = row.fact_ref();
        let claim_id = fact_ref.fact_claim_id().clone();
        claim_ids.push(fact_claim_id_string(&claim_id));
        let store_commit_order = store_commit_order_from_row(row).ok_or_else(|| {
            mfm_runtime::RuntimeError::InvalidRunnerOutput(
                "missing_fact: holding fact row missing store_commit_order".to_owned(),
            )
        })?;
        let requirement_artifact = fact_response_artifact_requirement(fact_ref);
        let artifact = artifacts
            .read_retained_artifact(&requirement_artifact)
            .await
            .map_err(runtime_artifact_read_error)?;
        let built = match requirement.projection {
            HoldingFactProjection::BitcoinAddressBalance => {
                let hydrated: BtcAddressBalanceResponse =
                    hydrate_fact_response_json(fact_ref, artifact.bytes()).map_err(|error| {
                        mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string())
                    })?;
                btc_holding_candidate(requirement, &hydrated, store_commit_order, claim_id)
            }
            HoldingFactProjection::EvmNativeBalance => {
                let hydrated: EvmAddressNativeBalanceResponse =
                    hydrate_fact_response_json(fact_ref, artifact.bytes()).map_err(|error| {
                        mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string())
                    })?;
                evm_holding_candidate(requirement, &hydrated, store_commit_order, claim_id)
            }
        };
        match built {
            Ok(candidate) => candidates.push(candidate),
            // Coverage/status filter-empty collapses to missing_fact at select time.
            Err(error) if is_filter_empty_holding_error(error.code) => {}
            Err(error) => return Err(holding_runtime_error(error)),
        }
    }
    Ok((candidates, claim_ids))
}

pub(crate) fn holding_fact_index_request(
    config: &SelectHoldingsConfig,
    requirement: &RequiredHoldingRequirement,
) -> Result<FactIndexReadRequest, PortfolioHoldingSelectionError> {
    let store_scope = StoreScopeRef::new(config.store_scope()).map_err(|error| {
        PortfolioHoldingSelectionError::new(
            PortfolioHoldingErrorCode::UnsupportedRequirement,
            error.to_string(),
            Some(requirement.key.as_key_str()),
            Some(requirement.key.network_id.clone()),
        )
    })?;
    let scope_decision = ScopeDecisionEvidence::new(portfolio_holding_select_scope_decision_hash());
    let plan = match requirement.projection {
        HoldingFactProjection::BitcoinAddressBalance => {
            let subject = requirement_to_btc_subject(requirement)?;
            platform_address_balance_candidate_plan(&store_scope, scope_decision, &subject)
                .map_err(|error| {
                    PortfolioHoldingSelectionError::new(
                        PortfolioHoldingErrorCode::UnsupportedRequirement,
                        error.to_string(),
                        Some(requirement.key.as_key_str()),
                        Some(requirement.key.network_id.clone()),
                    )
                })?
        }
        HoldingFactProjection::EvmNativeBalance => {
            let subject = requirement_to_evm_subject(requirement)?;
            platform_native_balance_candidate_plan(&store_scope, scope_decision, &subject).map_err(
                |error| {
                    PortfolioHoldingSelectionError::new(
                        PortfolioHoldingErrorCode::UnsupportedRequirement,
                        error.to_string(),
                        Some(requirement.key.as_key_str()),
                        Some(requirement.key.network_id.clone()),
                    )
                },
            )?
        }
    };
    FactIndexReadRequest::new(plan).map_err(|error| {
        PortfolioHoldingSelectionError::new(
            PortfolioHoldingErrorCode::UnsupportedRequirement,
            error.to_string(),
            Some(requirement.key.as_key_str()),
            Some(requirement.key.network_id.clone()),
        )
    })
}

pub(crate) fn requirement_to_btc_subject(
    requirement: &RequiredHoldingRequirement,
) -> Result<BtcAddressBalanceSubject, PortfolioHoldingSelectionError> {
    let bitcoin_network = requirement.network.bitcoin_network().ok_or_else(|| {
        PortfolioHoldingSelectionError::new(
            PortfolioHoldingErrorCode::UnsupportedRequirement,
            "bitcoin network tag missing",
            Some(requirement.key.as_key_str()),
            Some(requirement.key.network_id.clone()),
        )
    })?;
    let source_identity = requirement
        .network
        .source_identity()
        .map(|id| id.to_string())
        .ok_or_else(|| {
            PortfolioHoldingSelectionError::new(
                PortfolioHoldingErrorCode::UnsupportedRequirement,
                "bitcoin source identity missing",
                Some(requirement.key.as_key_str()),
                Some(requirement.key.network_id.clone()),
            )
        })?;
    BtcAddressBalanceSubject::new(
        requirement.key.network_id.clone(),
        bitcoin_network.to_owned(),
        source_identity,
        requirement.address.clone(),
    )
    .map_err(|error| {
        PortfolioHoldingSelectionError::new(
            PortfolioHoldingErrorCode::UnsupportedRequirement,
            error.to_string(),
            Some(requirement.key.as_key_str()),
            Some(requirement.key.network_id.clone()),
        )
    })
}

pub(crate) fn requirement_to_evm_subject(
    requirement: &RequiredHoldingRequirement,
) -> Result<EvmAddressNativeBalanceSubject, PortfolioHoldingSelectionError> {
    let chain_id = requirement.network.chain_id_u64().ok_or_else(|| {
        PortfolioHoldingSelectionError::new(
            PortfolioHoldingErrorCode::UnsupportedRequirement,
            "evm chain_id missing",
            Some(requirement.key.as_key_str()),
            Some(requirement.key.network_id.clone()),
        )
    })?;
    EvmAddressNativeBalanceSubject::new(
        requirement.key.network_id.clone(),
        chain_id,
        requirement.address.clone(),
    )
    .map_err(|error| {
        PortfolioHoldingSelectionError::new(
            PortfolioHoldingErrorCode::UnsupportedRequirement,
            error.to_string(),
            Some(requirement.key.as_key_str()),
            Some(requirement.key.network_id.clone()),
        )
    })
}

pub(crate) fn btc_holding_candidate(
    requirement: &RequiredHoldingRequirement,
    response: &BtcAddressBalanceResponse,
    store_commit_order: u64,
    fact_claim_id: FactClaimId,
) -> Result<HoldingCandidate, PortfolioHoldingSelectionError> {
    let subject = requirement_to_btc_subject(requirement).map_err(|error| {
        PortfolioHoldingSelectionError::new(
            PortfolioHoldingErrorCode::MissingFact,
            error.message,
            error.holding_key,
            error.network_id,
        )
    })?;
    let normalized = normalize_btc_address_balance(&subject, response).map_err(|error| {
        PortfolioHoldingSelectionError::new(
            PortfolioHoldingErrorCode::MissingFact,
            error.to_string(),
            Some(requirement.key.as_key_str()),
            Some(requirement.key.network_id.clone()),
        )
    })?;
    holding_candidate_from_normalized(
        &requirement.key,
        store_commit_order,
        fact_claim_id,
        NormalizedHoldingFields {
            balance_reader_kind: balance_reader_kind(&requirement.symbol.balance_reader).to_owned(),
            raw_dec: normalized.balance_sats.to_string(),
            decimals: 8,
            observation_anchor: ObservationAnchor::Bitcoin {
                height: normalized.anchor_height,
                block_hash: normalized.anchor_hash,
            },
            coverage: normalized.coverage.as_str().to_owned(),
            source_status: normalized.source_status.as_str().to_owned(),
        },
    )
}

pub(crate) fn evm_holding_candidate(
    requirement: &RequiredHoldingRequirement,
    response: &EvmAddressNativeBalanceResponse,
    store_commit_order: u64,
    fact_claim_id: FactClaimId,
) -> Result<HoldingCandidate, PortfolioHoldingSelectionError> {
    let subject = requirement_to_evm_subject(requirement).map_err(|error| {
        PortfolioHoldingSelectionError::new(
            PortfolioHoldingErrorCode::MissingFact,
            error.message,
            error.holding_key,
            error.network_id,
        )
    })?;
    let normalized = normalize_evm_address_native_balance(&subject, response).map_err(|error| {
        PortfolioHoldingSelectionError::new(
            PortfolioHoldingErrorCode::MissingFact,
            error.to_string(),
            Some(requirement.key.as_key_str()),
            Some(requirement.key.network_id.clone()),
        )
    })?;
    holding_candidate_from_normalized(
        &requirement.key,
        store_commit_order,
        fact_claim_id,
        NormalizedHoldingFields {
            balance_reader_kind: balance_reader_kind(&requirement.symbol.balance_reader).to_owned(),
            raw_dec: normalized.raw_wei,
            decimals: normalized.decimals,
            observation_anchor: ObservationAnchor::Evm {
                chain_id: normalized.chain_id,
                block_number: normalized.block_number,
                block_hash: normalized.block_hash,
            },
            coverage: normalized.coverage.as_str().to_owned(),
            source_status: normalized.source_status.as_str().to_owned(),
        },
    )
}

pub(crate) fn store_commit_order_from_row(row: &mfm_facts::FactQueryResultRow) -> Option<u64> {
    for field in row.returned_fields() {
        if field.field_id().as_str() == "metadata.store_commit_order" {
            if let FactCanonicalScalar::UnsignedInteger(value) = field.value() {
                return Some(*value);
            }
        }
    }
    None
}

pub(crate) fn fact_claim_id_string(claim_id: &mfm_facts::FactClaimId) -> String {
    format!(
        "{}:{}:{}",
        claim_id.source_run_id(),
        claim_id.source_seq(),
        claim_id.source_ordinal()
    )
}

pub(crate) fn select_holdings_output(
    ctx: ErasedRunCtx<'_>,
    value: &SelectedHoldings,
    evidences: &[FactQueryEvidence],
) -> mfm_runtime::Result<ErasedRunnerOutput> {
    let mut output = RunnerOutputBuilder::new(&ctx);
    output.state_output(value)?;
    for evidence in evidences {
        output.record_fact_query_evidence(evidence.clone())?;
    }
    Ok(output.finish())
}

/// Requires one shared snapshot read frontier across a selection batch.
///
/// Live select and replay share this check so mixed frontiers fail closed on both paths.
pub(crate) fn require_shared_snapshot_read_frontier<'a>(
    frontiers: impl IntoIterator<Item = &'a StoreReadFrontier>,
    frontier_types: impl IntoIterator<Item = StoreReadFrontierType>,
) -> Result<(), &'static str> {
    let mut expected: Option<&StoreReadFrontier> = None;
    for (frontier, frontier_type) in frontiers.into_iter().zip(frontier_types) {
        if frontier_type != StoreReadFrontierType::Snapshot {
            return Err("portfolio selection queries must use snapshot read frontiers");
        }
        match expected {
            None => expected = Some(frontier),
            Some(shared) if shared != frontier => {
                return Err("portfolio selection queries use mixed read frontiers");
            }
            Some(_) => {}
        }
    }
    Ok(())
}

pub(crate) fn holding_runtime_error(
    error: PortfolioHoldingSelectionError,
) -> mfm_runtime::RuntimeError {
    let code = error.code.as_str();
    let failure = mfm_runtime::RuntimeFailure::new(
        events::ErrorCode::new(code).expect("portfolio error code is a checked public code"),
        events::ErrorCategory::Validation,
        format!("{code}: portfolio holding selection failed"),
    )
    .expect("portfolio failure metadata is a checked public contract");
    mfm_runtime::RuntimeError::InvalidRunnerOutputFailure {
        failure,
        diagnostic: None,
    }
}

pub(crate) fn fact_index_runtime_error(
    error: mfm_fact_capabilities::FactIndexReadError,
) -> mfm_runtime::RuntimeError {
    mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string())
}

pub(crate) fn runtime_artifact_read_error(error: store::StoreError) -> mfm_runtime::RuntimeError {
    mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string())
}
