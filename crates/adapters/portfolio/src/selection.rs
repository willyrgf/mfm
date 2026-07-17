use super::*;

/// Executes receipt-pinned holding selection from retained facts only.
pub(crate) async fn select_holdings(
    config: ValidatedConfig<SelectHoldingsConfig>,
    input: mfm_state_portfolio::SelectHoldingsInput,
    artifacts: &dyn store::RetainedArtifactReadProvider,
    fact_index: &dyn FactIndexReadProvider,
) -> mfm_runtime::Result<(SelectedHoldings, Vec<FactQueryEvidence>)> {
    let state = SelectHoldingsState::new(config)
        .map_err(|error| mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string()))?;
    let config = state.config();
    validate_receipt_against_portfolio(&input.receipt, config.portfolio())
        .map_err(holding_runtime_error)?;

    let receipt_entries = input.receipt.holdings().to_vec();
    let requests = receipt_entries
        .iter()
        .map(|entry| holding_fact_index_request(config, entry).map_err(holding_runtime_error))
        .collect::<mfm_runtime::Result<Vec<_>>>()?;
    let responses = fact_index
        .read_fact_index_batch(&requests)
        .await
        .map_err(fact_index_runtime_error)?;
    if responses.len() != requests.len() {
        return Err(mfm_runtime::RuntimeError::InvalidRunnerOutput(
            "fact-index batch response count does not match receipt demand".to_owned(),
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

    // Prove every bounded query is exhaustive before reading even one candidate artifact. A later
    // saturated receipt must not cause earlier holdings to perform partial hydration work.
    for (entry, response) in receipt_entries.iter().zip(&responses) {
        require_exact_bounded_cardinality(response, config, entry)?;
    }

    let symbols =
        symbols_by_id_map(&config.portfolio().symbol_configs).map_err(holding_runtime_error)?;
    let mut selected = Vec::with_capacity(receipt_entries.len());
    let mut evidences = Vec::with_capacity(receipt_entries.len());
    for ((entry, request), response) in receipt_entries.iter().zip(requests).zip(responses) {
        let (winner, selected_index) =
            candidates_for_receipt_entry(entry, config, &response, artifacts).await?;
        selected.push(winner);
        let selection = state
            .selection_evidence_for_index(selected_index)
            .map_err(holding_runtime_error)?;
        evidences.push(FactQueryEvidence::new(
            request.plan().clone(),
            response.receipt().clone(),
            selection,
        ));
    }
    selected.sort_by(|left, right| left.key.cmp(&right.key));
    let observations =
        observations_from_selected_holdings(&selected, &symbols).map_err(holding_runtime_error)?;
    let pins =
        project_network_pins_from_observations(&observations).map_err(holding_runtime_error)?;
    if pins.as_slice() != input.receipt.network_anchors() {
        return Err(holding_runtime_error(PortfolioHoldingSelectionError::new(
            PortfolioHoldingErrorCode::ReceiptMismatch,
            "selected holding anchors did not equal the collection receipt anchors",
            None,
            None,
        )));
    }
    Ok((SelectedHoldings { observations }, evidences))
}

fn require_exact_bounded_cardinality(
    response: &mfm_facts::FactQueryResult,
    config: &SelectHoldingsConfig,
    entry: &CollectedHoldingReceipt,
) -> mfm_runtime::Result<()> {
    let rows = response.rows();
    match response.receipt().result_cardinality() {
        QueryResultCardinality::Exact(count)
            if count == rows.len() as u64 && count <= config.candidate_bound() =>
        {
            Ok(())
        }
        QueryResultCardinality::AtLeast(count) if count >= config.candidate_scan_limit() => {
            Err(holding_runtime_error(PortfolioHoldingSelectionError::new(
                PortfolioHoldingErrorCode::CandidateBoundExhausted,
                "receipt-pinned fact query saturated before candidate exhaustion was proven",
                Some(entry.requirement().as_key_str()),
                Some(entry.requirement().network_id.clone()),
            )))
        }
        _ => Err(holding_runtime_error(PortfolioHoldingSelectionError::new(
            PortfolioHoldingErrorCode::ReceiptMismatch,
            "receipt-pinned fact query did not provide an exact bounded result set",
            Some(entry.requirement().as_key_str()),
            Some(entry.requirement().network_id.clone()),
        ))),
    }
}

async fn candidates_for_receipt_entry(
    entry: &CollectedHoldingReceipt,
    config: &SelectHoldingsConfig,
    response: &mfm_facts::FactQueryResult,
    artifacts: &dyn store::RetainedArtifactReadProvider,
) -> mfm_runtime::Result<(SelectedHolding, usize)> {
    let symbol = config
        .portfolio()
        .symbol_configs
        .iter()
        .find(|symbol| symbol.symbol_id.as_str() == entry.requirement().symbol_id)
        .ok_or_else(|| {
            holding_runtime_error(PortfolioHoldingSelectionError::new(
                PortfolioHoldingErrorCode::ReceiptMismatch,
                "receipt holding referenced a missing portfolio symbol",
                Some(entry.requirement().as_key_str()),
                Some(entry.requirement().network_id.clone()),
            ))
        })?;
    let mut matching = Vec::new();
    for (row_index, row) in response.rows().iter().enumerate() {
        let fact_ref = row.fact_ref();
        let store_commit_order = store_commit_order_from_row(row).ok_or_else(|| {
            mfm_runtime::RuntimeError::InvalidRunnerOutput(
                "receipt-pinned holding query omitted metadata.store_commit_order".to_owned(),
            )
        })?;
        let artifact = artifacts
            .read_retained_artifact(&fact_response_artifact_requirement(fact_ref))
            .await
            .map_err(runtime_artifact_read_error)?;
        let candidate = match entry.source() {
            HoldingSourceKey::BitcoinNative { .. } => hydrate_btc_candidate(
                entry,
                symbol.source.clone(),
                fact_ref,
                artifact.bytes(),
                store_commit_order,
            )?,
            HoldingSourceKey::EvmNative { .. } => hydrate_evm_native_candidate(
                entry,
                symbol.source.clone(),
                fact_ref,
                artifact.bytes(),
                store_commit_order,
            )?,
            HoldingSourceKey::EvmErc20 { .. } => hydrate_evm_erc20_candidate(
                entry,
                symbol.source.clone(),
                fact_ref,
                artifact.bytes(),
                store_commit_order,
            )?,
        };
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
        return Err(holding_runtime_error(PortfolioHoldingSelectionError::new(
            PortfolioHoldingErrorCode::MissingFact,
            "no retained fact had the collection receipt's exact content identity",
            Some(entry.requirement().as_key_str()),
            Some(entry.requirement().network_id.clone()),
        )));
    };
    Ok((
        SelectedHolding {
            key: candidate_key(entry),
            anchor: candidate.anchor,
            store_commit_order: candidate.store_commit_order,
            fact_claim_id: candidate.fact_claim_id,
            material: candidate.response_material,
        },
        selected_index,
    ))
}

pub(crate) fn hydrate_btc_candidate(
    entry: &CollectedHoldingReceipt,
    holding: HoldingSourceConfig,
    fact_ref: &mfm_facts::InternalFactRef,
    artifact_bytes: &[u8],
    store_commit_order: u64,
) -> mfm_runtime::Result<Option<HoldingCandidate>> {
    let Some((network, bitcoin_network, semantic_source_identity, address)) =
        entry.source().bitcoin_native_parts()
    else {
        return Err(receipt_shape_error(
            entry,
            "receipt source was not Bitcoin native",
        ));
    };
    let subject =
        BtcAddressBalanceSubject::new(network, bitcoin_network, semantic_source_identity, address)
            .map_err(|error| receipt_invalid_runtime_error(entry, error))?;
    let response: BtcAddressBalanceResponse = hydrate_fact_response_json(fact_ref, artifact_bytes)
        .map_err(|error| mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string()))?;
    let normalized = normalize_btc_address_balance(&subject, &response)
        .map_err(|error| receipt_invalid_runtime_error(entry, error))?;
    if !matches_btc_receipt(entry, &response) {
        return Err(receipt_shape_error(
            entry,
            "hydrated Bitcoin response did not match receipt anchor/status",
        ));
    }
    let descriptor = BtcAddressBalanceSnapshotFact::descriptor()
        .map_err(|error| receipt_invalid_runtime_error(entry, error))?;
    let identity =
        mfm_facts::derive_fact_content_identity_from_typed_values(&descriptor, &subject, &response)
            .map_err(|error| receipt_invalid_runtime_error(entry, error))?;
    ensure_reference_identity(entry, fact_ref, &identity)?;
    if entry
        .fact_content_identity_evidence()
        .verify_against_typed_values(&descriptor, &subject, &response)
        .map_err(|error| receipt_invalid_runtime_error(entry, error))?
        .is_none()
    {
        return Ok(None);
    }
    holding_candidate_from_normalized(
        entry.requirement(),
        store_commit_order,
        fact_ref.fact_claim_id().clone(),
        NormalizedHoldingFields {
            holding,
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
    .map(Some)
    .map_err(holding_runtime_error)
}

pub(crate) fn hydrate_evm_native_candidate(
    entry: &CollectedHoldingReceipt,
    holding: HoldingSourceConfig,
    fact_ref: &mfm_facts::InternalFactRef,
    artifact_bytes: &[u8],
    store_commit_order: u64,
) -> mfm_runtime::Result<Option<HoldingCandidate>> {
    let Some((network, chain_id, account)) = entry.source().evm_native_parts() else {
        return Err(receipt_shape_error(
            entry,
            "receipt source was not EVM native",
        ));
    };
    let subject = EvmAddressNativeBalanceSubject::new(network, chain_id, account)
        .map_err(|error| receipt_invalid_runtime_error(entry, error))?;
    let response: EvmAddressNativeBalanceResponse =
        hydrate_fact_response_json(fact_ref, artifact_bytes)
            .map_err(|error| mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string()))?;
    let normalized = normalize_evm_address_native_balance(&subject, &response)
        .map_err(|error| receipt_invalid_runtime_error(entry, error))?;
    if !matches_evm_receipt(
        entry,
        response.block_number(),
        response.block_hash(),
        response.coverage(),
        response.source_status(),
    ) {
        return Err(receipt_shape_error(
            entry,
            "hydrated EVM native response did not match receipt anchor/status",
        ));
    }
    let descriptor = EvmAddressNativeBalanceSnapshotFact::descriptor()
        .map_err(|error| receipt_invalid_runtime_error(entry, error))?;
    let identity =
        mfm_facts::derive_fact_content_identity_from_typed_values(&descriptor, &subject, &response)
            .map_err(|error| receipt_invalid_runtime_error(entry, error))?;
    ensure_reference_identity(entry, fact_ref, &identity)?;
    if entry
        .fact_content_identity_evidence()
        .verify_against_typed_values(&descriptor, &subject, &response)
        .map_err(|error| receipt_invalid_runtime_error(entry, error))?
        .is_none()
    {
        return Ok(None);
    }
    holding_candidate_from_normalized(
        entry.requirement(),
        store_commit_order,
        fact_ref.fact_claim_id().clone(),
        NormalizedHoldingFields {
            holding,
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
    .map(Some)
    .map_err(holding_runtime_error)
}

pub(crate) fn hydrate_evm_erc20_candidate(
    entry: &CollectedHoldingReceipt,
    holding: HoldingSourceConfig,
    fact_ref: &mfm_facts::InternalFactRef,
    artifact_bytes: &[u8],
    store_commit_order: u64,
) -> mfm_runtime::Result<Option<HoldingCandidate>> {
    let Some((network, chain_id, contract_address, account)) = entry.source().evm_erc20_parts()
    else {
        return Err(receipt_shape_error(
            entry,
            "receipt source was not EVM ERC-20",
        ));
    };
    let subject = EvmAddressErc20BalanceSubject::new(network, chain_id, contract_address, account)
        .map_err(|error| receipt_invalid_runtime_error(entry, error))?;
    let response: EvmAddressErc20BalanceResponse =
        hydrate_fact_response_json(fact_ref, artifact_bytes)
            .map_err(|error| mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string()))?;
    let rebuilt = EvmAddressErc20BalanceSnapshotFact::try_new(
        subject.clone(),
        response.block_number(),
        response.block_hash(),
        response.raw_units(),
        response.decimals(),
    )
    .map_err(|error| receipt_invalid_runtime_error(entry, error))?;
    if rebuilt.response() != &response
        || !matches_evm_receipt(
            entry,
            response.block_number(),
            response.block_hash(),
            response.coverage(),
            response.source_status(),
        )
    {
        return Err(receipt_shape_error(
            entry,
            "hydrated EVM ERC-20 response did not match receipt anchor/status",
        ));
    }
    let descriptor = EvmAddressErc20BalanceSnapshotFact::descriptor()
        .map_err(|error| receipt_invalid_runtime_error(entry, error))?;
    let identity =
        mfm_facts::derive_fact_content_identity_from_typed_values(&descriptor, &subject, &response)
            .map_err(|error| receipt_invalid_runtime_error(entry, error))?;
    ensure_reference_identity(entry, fact_ref, &identity)?;
    if entry
        .fact_content_identity_evidence()
        .verify_against_typed_values(&descriptor, &subject, &response)
        .map_err(|error| receipt_invalid_runtime_error(entry, error))?
        .is_none()
    {
        return Ok(None);
    }
    holding_candidate_from_normalized(
        entry.requirement(),
        store_commit_order,
        fact_ref.fact_claim_id().clone(),
        NormalizedHoldingFields {
            holding,
            raw_dec: response.raw_units().to_owned(),
            decimals: response.decimals(),
            observation_anchor: ObservationAnchor::Evm {
                chain_id,
                block_number: response.block_number(),
                block_hash: response.block_hash().to_owned(),
            },
            coverage: response.coverage().to_owned(),
            source_status: response.source_status().to_owned(),
        },
    )
    .map(Some)
    .map_err(holding_runtime_error)
}

/// Builds an exact receipt-constrained query request for one holding.
pub(crate) fn holding_fact_index_request(
    config: &SelectHoldingsConfig,
    entry: &CollectedHoldingReceipt,
) -> Result<FactIndexReadRequest, PortfolioHoldingSelectionError> {
    let store_scope = StoreScopeRef::new(config.store_scope())
        .map_err(|error| receipt_selection_error(entry, error.to_string()))?;
    let scope_decision = ScopeDecisionEvidence::new(portfolio_holding_select_scope_decision_hash());
    let plan = match entry.source() {
        HoldingSourceKey::BitcoinNative { .. } => {
            let Some((network, bitcoin_network, semantic_source_identity, address)) =
                entry.source().bitcoin_native_parts()
            else {
                return Err(receipt_selection_error(entry, "invalid Bitcoin source key"));
            };
            let subject = BtcAddressBalanceSubject::new(
                network,
                bitcoin_network,
                semantic_source_identity,
                address,
            )
            .map_err(|error| receipt_selection_error(entry, error.to_string()))?;
            let mfm_portfolio_model::portfolio::ExecutionAnchor::Bitcoin { height, block_hash } =
                entry.anchor()
            else {
                return Err(receipt_selection_error(
                    entry,
                    "Bitcoin source did not carry a Bitcoin anchor",
                ));
            };
            platform_address_balance_at_anchor_plan(
                &store_scope,
                scope_decision,
                &subject,
                *height,
                block_hash,
                entry.coverage(),
                entry.source_status(),
                config.candidate_scan_limit(),
            )
            .map_err(|error| receipt_selection_error(entry, error.to_string()))?
        }
        HoldingSourceKey::EvmNative { .. } => {
            let Some((network, chain_id, account)) = entry.source().evm_native_parts() else {
                return Err(receipt_selection_error(
                    entry,
                    "invalid EVM native source key",
                ));
            };
            let subject = EvmAddressNativeBalanceSubject::new(network, chain_id, account)
                .map_err(|error| receipt_selection_error(entry, error.to_string()))?;
            let mfm_portfolio_model::portfolio::ExecutionAnchor::Evm {
                block_number,
                block_hash,
                ..
            } = entry.anchor()
            else {
                return Err(receipt_selection_error(
                    entry,
                    "EVM source did not carry an EVM anchor",
                ));
            };
            platform_native_balance_at_anchor_plan(
                &store_scope,
                scope_decision,
                &subject,
                *block_number,
                block_hash,
                entry.coverage(),
                entry.source_status(),
                config.candidate_scan_limit(),
            )
            .map_err(|error| receipt_selection_error(entry, error.to_string()))?
        }
        HoldingSourceKey::EvmErc20 { .. } => {
            let Some((network, chain_id, contract_address, account)) =
                entry.source().evm_erc20_parts()
            else {
                return Err(receipt_selection_error(
                    entry,
                    "invalid EVM ERC-20 source key",
                ));
            };
            let subject =
                EvmAddressErc20BalanceSubject::new(network, chain_id, contract_address, account)
                    .map_err(|error| receipt_selection_error(entry, error.to_string()))?;
            let mfm_portfolio_model::portfolio::ExecutionAnchor::Evm {
                block_number,
                block_hash,
                ..
            } = entry.anchor()
            else {
                return Err(receipt_selection_error(
                    entry,
                    "ERC-20 source did not carry an EVM anchor",
                ));
            };
            platform_erc20_balance_at_anchor_plan(
                &store_scope,
                scope_decision,
                &subject,
                *block_number,
                block_hash,
                entry.coverage(),
                entry.source_status(),
                config.candidate_scan_limit(),
            )
            .map_err(|error| receipt_selection_error(entry, error.to_string()))?
        }
    };
    FactIndexReadRequest::new(plan)
        .map_err(|error| receipt_selection_error(entry, error.to_string()))
}

fn ensure_reference_identity(
    entry: &CollectedHoldingReceipt,
    fact_ref: &mfm_facts::InternalFactRef,
    identity: &mfm_facts::FactContentIdentity,
) -> mfm_runtime::Result<()> {
    if fact_ref.fact_descriptor_hash() != identity.fact_descriptor_hash()
        || fact_ref.subject_material_hash() != identity.subject_material_hash()
        || fact_ref.response_schema_id() != identity.response_schema_id()
        || fact_ref.response_hash() != identity.response_hash()
    {
        return Err(receipt_shape_error(
            entry,
            "fact reference did not match descriptor-checked hydrated content",
        ));
    }
    Ok(())
}

fn matches_btc_receipt(
    entry: &CollectedHoldingReceipt,
    response: &BtcAddressBalanceResponse,
) -> bool {
    matches!(
        entry.anchor(),
        mfm_portfolio_model::portfolio::ExecutionAnchor::Bitcoin { height, block_hash }
            if *height == response.anchor_height()
                && block_hash == response.anchor_hash()
                && entry.coverage() == response.coverage()
                && entry.source_status() == response.source_status()
    )
}

fn matches_evm_receipt(
    entry: &CollectedHoldingReceipt,
    block_number: u64,
    block_hash: &str,
    coverage: &str,
    source_status: &str,
) -> bool {
    matches!(
        entry.anchor(),
        mfm_portfolio_model::portfolio::ExecutionAnchor::Evm { block_number: expected_number, block_hash: expected_hash, .. }
            if *expected_number == block_number
                && expected_hash == block_hash
                && entry.coverage() == coverage
                && entry.source_status() == source_status
    )
}

fn candidate_key(entry: &CollectedHoldingReceipt) -> HoldingRequirementKey {
    entry.requirement().clone()
}

pub(crate) fn store_commit_order_from_row(row: &mfm_facts::FactQueryResultRow) -> Option<u64> {
    row.returned_fields().iter().find_map(|field| {
        (field.field_id().as_str() == "metadata.store_commit_order").then(|| {
            match field.value() {
                FactCanonicalScalar::UnsignedInteger(value) => Some(*value),
                _ => None,
            }
        })?
    })
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

/// Requires one shared snapshot read frontier across receipt-pinned queries.
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
        Vec::new(),
    )
    .expect("portfolio failure metadata is a checked public contract");
    mfm_runtime::RuntimeError::Failure(failure)
}

pub(crate) fn fact_index_runtime_error(
    error: mfm_fact_capabilities::FactIndexReadError,
) -> mfm_runtime::RuntimeError {
    mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string())
}

pub(crate) fn runtime_artifact_read_error(error: store::StoreError) -> mfm_runtime::RuntimeError {
    mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string())
}

fn receipt_selection_error(
    entry: &CollectedHoldingReceipt,
    message: impl Into<String>,
) -> PortfolioHoldingSelectionError {
    PortfolioHoldingSelectionError::new(
        PortfolioHoldingErrorCode::ReceiptMismatch,
        message,
        Some(entry.requirement().as_key_str()),
        Some(entry.requirement().network_id.clone()),
    )
}

fn receipt_shape_error(
    entry: &CollectedHoldingReceipt,
    message: impl Into<String>,
) -> mfm_runtime::RuntimeError {
    holding_runtime_error(receipt_selection_error(entry, message))
}

fn receipt_invalid_runtime_error(
    entry: &CollectedHoldingReceipt,
    error: impl std::fmt::Display,
) -> mfm_runtime::RuntimeError {
    receipt_shape_error(entry, error.to_string())
}
