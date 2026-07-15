use super::*;

use mfm_replay::v1 as replay;

use super::selection::{
    btc_holding_candidate, evm_holding_candidate, fact_claim_id_string, holding_fact_index_request,
    require_shared_snapshot_read_frontier, store_commit_order_from_row,
};

/// Verifies the portfolio selection decision from certified config and retained query evidence.
///
/// The verifier shares pure candidate reconstruction with the live selection path. It accepts no
/// capability or transport, and it requires every complete candidate query, one shared snapshot
/// frontier, exact response evidence, and the exact state-output bytes produced by the live path.
pub fn verify_portfolio_replay(broker: &replay::ReplayBroker) -> replay::Result<()> {
    let subjects_frame =
        replay_single_state_frame::<ResolveSubjectsState>(broker, "resolved-subjects")?;
    let subjects_config: ResolveSubjectsConfig = replay_node_config(broker, &subjects_frame.node)?;
    let subjects = resolve_subjects_from_config(&subjects_config);
    verify_replay_output_bytes(&subjects_frame, &subjects, "resolved subjects")?;

    let select_kind = SelectHoldingsState::kind().map_err(replay_adapter_error)?;
    let select_version = SelectHoldingsState::version().map_err(replay_adapter_error)?;
    let select_frames = broker.produced_cell_frames_matching(|node, _cell, _produced| {
        Ok(node.state_kind == select_kind && node.state_version == select_version)
    })?;
    if select_frames.is_empty() {
        return Err(replay_portfolio_mismatch(
            "portfolio replay is not dispatched without a SelectHoldings output",
        ));
    }
    if select_frames.len() != 1 {
        return Err(replay_portfolio_mismatch(
            "portfolio replay requires exactly one SelectHoldings output",
        ));
    }
    let select_frame = &select_frames[0];
    let config: SelectHoldingsConfig = replay_node_config(broker, &select_frame.node)?;
    if config.selection_policy_id()
        != mfm_state_portfolio::PORTFOLIO_HOLDING_LATEST_NETWORK_COHERENT_POLICY_ID
    {
        return Err(replay_portfolio_mismatch(
            "portfolio selection policy does not match the certified policy",
        ));
    }
    if subjects_config.wallets() != config.portfolio().wallets {
        return Err(replay_portfolio_mismatch(
            "portfolio select config does not match resolved-subjects config",
        ));
    }
    let requirements =
        expand_required_holdings(&config, &subjects).map_err(replay_adapter_error)?;
    let expected_requests = requirements
        .iter()
        .map(|requirement| {
            holding_fact_index_request(&config, requirement).map_err(replay_adapter_error)
        })
        .collect::<replay::Result<Vec<_>>>()?;
    let query_evidence = replay_fact_query_evidence_for_attempt(
        broker,
        &select_frame.node.node_id,
        &select_frame.produced.attempt_id,
    )?;
    if query_evidence.len() != expected_requests.len() {
        return Err(replay_portfolio_mismatch(
            "portfolio selection query evidence is incomplete or has extra reads",
        ));
    }

    let mut evidence_by_requirement = vec![None; requirements.len()];
    require_shared_snapshot_read_frontier(
        query_evidence
            .iter()
            .map(|evidence| evidence.receipt().read_frontier()),
        query_evidence
            .iter()
            .map(|evidence| evidence.receipt().frontier_type()),
    )
    .map_err(replay_portfolio_mismatch)?;
    let mut matched_requirements = BTreeSet::new();
    for evidence in query_evidence {
        if evidence.selection().selection_policy_hash()
            != &mfm_state_portfolio::portfolio_holding_selection_policy_digest()
        {
            return Err(replay_portfolio_mismatch(
                "portfolio query evidence uses an unknown selection policy",
            ));
        }
        if evidence.selection().selected_summaries_digest().is_some() {
            return Err(replay_portfolio_mismatch(
                "portfolio query evidence does not describe a complete snapshot selection",
            ));
        }
        let Some(index) =
            first_unmatched_plan_index(&expected_requests, &matched_requirements, evidence.plan())
        else {
            return Err(replay_portfolio_mismatch(
                "portfolio query evidence does not match a certified holding request",
            ));
        };
        if evidence_by_requirement[index].replace(evidence).is_some() {
            return Err(replay_portfolio_mismatch(
                "portfolio holding query evidence is duplicated",
            ));
        }
        matched_requirements.insert(index);
    }

    let mut candidates_by_holding = BTreeMap::new();
    let mut row_claim_ids_by_holding = BTreeMap::new();
    for (index, (requirement, evidence)) in requirements
        .iter()
        .zip(evidence_by_requirement.iter())
        .enumerate()
    {
        let evidence = evidence.as_ref().ok_or_else(|| {
            replay_portfolio_mismatch(format!(
                "missing fact query evidence for holding requirement {index}"
            ))
        })?;
        let (candidates, row_claim_ids) =
            replay_candidates_for_requirement(broker, requirement, evidence)?;
        candidates_by_holding.insert(requirement.key.clone(), candidates);
        row_claim_ids_by_holding.insert(requirement.key.clone(), row_claim_ids);
    }

    let selected = select_network_coherent(&candidates_by_holding).map_err(replay_adapter_error)?;
    let selected_claim_ids: BTreeSet<String> = selected
        .iter()
        .map(|item| fact_claim_id_string(&item.fact_claim_id))
        .collect();
    for (index, requirement) in requirements.iter().enumerate() {
        let evidence = evidence_by_requirement[index]
            .as_ref()
            .ok_or_else(|| replay_portfolio_mismatch("missing matched holding evidence"))?;
        let row_claim_ids = row_claim_ids_by_holding
            .get(&requirement.key)
            .ok_or_else(|| replay_portfolio_mismatch("missing replayed holding rows"))?;
        let expected_indices = row_claim_ids
            .iter()
            .enumerate()
            .filter_map(|(index, claim_id)| {
                selected_claim_ids
                    .contains(claim_id)
                    .then(|| u64::try_from(index).ok())
                    .flatten()
            })
            .collect::<Vec<_>>();
        if evidence.selection().selected_indices() != expected_indices {
            return Err(replay_portfolio_mismatch(
                "portfolio selection indices do not match the recomputed winners",
            ));
        }
    }

    let symbols =
        symbols_by_id_map(&config.portfolio().symbol_configs).map_err(replay_adapter_error)?;
    let observations =
        observations_from_selected_holdings(&selected, &symbols).map_err(replay_adapter_error)?;
    project_network_pins_from_observations(&observations).map_err(replay_adapter_error)?;
    let expected_output = SelectedHoldings { observations };
    let expected_bytes = canonical_value_bytes(&expected_output)?;
    if expected_bytes != select_frame.artifact_bytes {
        return Err(replay_portfolio_mismatch(
            "portfolio SelectHoldings output does not match recomputed selection",
        ));
    }

    let valuations_frame =
        replay_single_state_frame::<ResolveValuationsState>(broker, "resolved-valuations")?;
    let valuations_config: ResolveValuationsConfig =
        replay_node_config(broker, &valuations_frame.node)?;
    if valuations_config.symbol_configs() != config.portfolio().symbol_configs {
        return Err(replay_portfolio_mismatch(
            "portfolio valuation config does not match select config",
        ));
    }
    let valuations =
        resolve_valuations_from_config(&valuations_config).map_err(replay_adapter_error)?;
    verify_replay_output_bytes(&valuations_frame, &valuations, "resolved valuations")?;

    let snapshot_frame =
        replay_single_state_frame::<AssembleSnapshotState>(broker, "assembled snapshot")?;
    let snapshot_config: AssembleSnapshotConfig = replay_node_config(broker, &snapshot_frame.node)?;
    if snapshot_config.portfolio() != config.portfolio() {
        return Err(replay_portfolio_mismatch(
            "portfolio snapshot config does not match select config",
        ));
    }
    let snapshot = assemble_snapshot(
        &snapshot_config,
        AssembleSnapshotInput {
            subjects,
            holdings: expected_output.clone(),
            valuations,
        },
    )
    .map_err(replay_adapter_error)?;
    verify_replay_output_bytes(&snapshot_frame, &snapshot, "assembled snapshot")?;

    let report_frame = replay_single_state_frame::<ProjectReportState>(broker, "project report")?;
    let report_config: ProjectReportConfig = replay_node_config(broker, &report_frame.node)?;
    let report =
        mfm_state_portfolio::project_report_from_snapshot(snapshot, report_config.report_version())
            .map_err(replay_adapter_error)?;
    verify_replay_output_bytes(&report_frame, &report, "project report")?;
    Ok(())
}

fn replay_single_state_frame<S>(
    broker: &replay::ReplayBroker,
    label: &'static str,
) -> replay::Result<replay::ProducedCellReplayFrame>
where
    S: StateSpec,
{
    let kind = S::kind().map_err(replay_adapter_error)?;
    let version = S::version().map_err(replay_adapter_error)?;
    let frames = broker.produced_cell_frames_matching(|node, _cell, _produced| {
        Ok(node.state_kind == kind && node.state_version == version)
    })?;
    if frames.len() != 1 {
        return Err(replay_portfolio_mismatch(format!(
            "portfolio replay requires exactly one {label} output"
        )));
    }
    Ok(frames
        .into_iter()
        .next()
        .expect("one replay frame was checked"))
}

fn verify_replay_output_bytes<T: serde::Serialize>(
    frame: &replay::ProducedCellReplayFrame,
    expected: &T,
    label: &'static str,
) -> replay::Result<()> {
    if canonical_value_bytes(expected)? != frame.artifact_bytes {
        return Err(replay_portfolio_mismatch(format!(
            "portfolio {label} output does not match recomputed value"
        )));
    }
    Ok(())
}

fn replay_fact_query_evidence_for_attempt(
    broker: &replay::ReplayBroker,
    node_id: &mfm_ids::NodeId,
    attempt_id: &mfm_ids::AttemptId,
) -> replay::Result<Vec<FactQueryEvidence>> {
    let mut evidence = Vec::new();
    for event in broker.events() {
        let events::KernelEventPayload::ArtifactReferenced(payload) = event.payload() else {
            continue;
        };
        if payload.artifact_ref.role != events::ArtifactRole::FactQueryEvidence
            || payload.node_id.as_ref() != Some(node_id)
            || payload.attempt_id.as_ref() != Some(attempt_id)
        {
            continue;
        }
        let requirement = store::EventArtifactRequirement {
            source: store::EventArtifactReferenceSource::ArtifactReferenced,
            artifact_id: payload.artifact_ref.artifact_id.clone(),
            evidence_hash: payload.artifact_ref.evidence_hash.clone(),
            digest: Some(payload.artifact_ref.content_digest.clone()),
            byte_len: Some(payload.artifact_ref.byte_len),
            media_type: Some(payload.artifact_ref.media_type.clone()),
            schema_id: Some(payload.artifact_ref.schema_id.clone()),
            semantic_type_id: payload.artifact_ref.semantic_type_id.clone(),
            producer_node_id: payload.node_id.clone(),
            producer_seed_id: None,
            artifact_role: Some(payload.artifact_ref.role),
        };
        let artifact = broker.retained_artifact(&requirement)?;
        let parsed = mfm_facts::parse_canonical_fact_query_evidence_bytes(&artifact.artifact_bytes)
            .map_err(replay_adapter_error)?;
        evidence.push(parsed);
    }
    Ok(evidence)
}

fn replay_candidates_for_requirement(
    broker: &replay::ReplayBroker,
    requirement: &RequiredHoldingRequirement,
    evidence: &FactQueryEvidence,
) -> replay::Result<(Vec<HoldingCandidate>, Vec<String>)> {
    let rows = fact_query_result_rows_from_receipt(evidence.receipt());
    if !matches!(
        evidence.receipt().result_cardinality(),
        QueryResultCardinality::Exact(count) if count == rows.len() as u64
    ) {
        return Err(replay_portfolio_mismatch(
            "portfolio holding query did not retain a complete candidate set",
        ));
    }
    if evidence.receipt().returned_field_summaries().is_none() {
        return Err(replay_portfolio_mismatch(
            "portfolio holding query omitted pinned returned fields",
        ));
    }
    let mut candidates = Vec::new();
    let mut claim_ids = Vec::with_capacity(rows.len());
    for row in &rows {
        let fact_ref = row.fact_ref();
        let claim_id = fact_ref.fact_claim_id().clone();
        claim_ids.push(fact_claim_id_string(&claim_id));
        let store_commit_order = store_commit_order_from_row(row).ok_or_else(|| {
            replay_portfolio_mismatch(
                "portfolio holding query omitted the store commit ordering field",
            )
        })?;
        let artifact = broker.retained_artifact(&fact_response_artifact_requirement(fact_ref))?;
        let built = match requirement.projection {
            HoldingFactProjection::BitcoinAddressBalance => {
                let response: BtcAddressBalanceResponse =
                    hydrate_fact_response_json(fact_ref, &artifact.artifact_bytes)
                        .map_err(replay_adapter_error)?;
                btc_holding_candidate(requirement, &response, store_commit_order, claim_id)
            }
            HoldingFactProjection::EvmNativeBalance => {
                let response: EvmAddressNativeBalanceResponse =
                    hydrate_fact_response_json(fact_ref, &artifact.artifact_bytes)
                        .map_err(replay_adapter_error)?;
                evm_holding_candidate(requirement, &response, store_commit_order, claim_id)
            }
        };
        match built {
            Ok(candidate) => candidates.push(candidate),
            Err(error) if is_filter_empty_holding_error(error.code) => {}
            Err(error) => return Err(replay_adapter_error(error)),
        }
    }
    Ok((candidates, claim_ids))
}

fn replay_node_config<T>(
    broker: &replay::ReplayBroker,
    node: &mfm_spec::v1::NodeSpec,
) -> replay::Result<T>
where
    T: mfm_values::MfmConfig + DeserializeOwned,
{
    let config_evidence = store::ArtifactEvidenceRef {
        artifact_id: node.config_ref.artifact_id.clone(),
        digest: node.config_ref.digest.clone(),
        byte_len: node.config_ref.byte_len,
        media_type: node.config_ref.media_type.clone(),
        schema_id: Some(node.config_ref.schema_id.clone()),
        semantic_type_id: None,
        producer_node_id: None,
        producer_seed_id: None,
        artifact_role: events::ArtifactRole::TypedConfig,
    };
    let requirement = store::EventArtifactRequirement {
        source: store::EventArtifactReferenceSource::RunConfig,
        artifact_id: node.config_ref.artifact_id.clone(),
        evidence_hash: config_evidence
            .evidence_hash()
            .map_err(replay_adapter_error)?,
        digest: Some(node.config_ref.digest.clone()),
        byte_len: Some(node.config_ref.byte_len),
        media_type: Some(node.config_ref.media_type.clone()),
        schema_id: Some(node.config_ref.schema_id.clone()),
        semantic_type_id: None,
        producer_node_id: None,
        producer_seed_id: None,
        artifact_role: Some(events::ArtifactRole::TypedConfig),
    };
    let artifact = broker.retained_artifact(&requirement)?;
    let config: T = serde_json::from_slice(&artifact.artifact_bytes).map_err(replay_json_error)?;
    ValidatedConfig::new(config)
        .map(ValidatedConfig::into_inner)
        .map_err(replay_adapter_error)
}

fn canonical_value_bytes<T: serde::Serialize>(value: &T) -> replay::Result<Vec<u8>> {
    let json = serde_json::to_string(value).map_err(replay_json_error)?;
    PlainCanonicalJsonBytes::from_json_str(&json)
        .map(|bytes| bytes.as_bytes().to_vec())
        .map_err(replay_adapter_error)
}

fn replay_json_error(error: serde_json::Error) -> replay::ReplayError {
    replay::ReplayError::new(
        replay::ReplayErrorKind::CertifiedEvidenceMismatch,
        error.to_string(),
    )
}

fn replay_adapter_error(error: impl std::fmt::Display) -> replay::ReplayError {
    replay::ReplayError::new(
        replay::ReplayErrorKind::CertifiedEvidenceMismatch,
        error.to_string(),
    )
}

fn replay_portfolio_mismatch(message: impl Into<String>) -> replay::ReplayError {
    replay::ReplayError::new(replay::ReplayErrorKind::CertifiedEvidenceMismatch, message)
}

/// Multiset plan match: first unmatched expected request with an equal query plan.
pub(crate) fn first_unmatched_plan_index(
    expected_requests: &[FactIndexReadRequest],
    matched_requirements: &BTreeSet<usize>,
    plan: &CanonicalFactQueryPlan,
) -> Option<usize> {
    expected_requests
        .iter()
        .enumerate()
        .find_map(|(index, request)| {
            (!matched_requirements.contains(&index) && request.plan() == plan).then_some(index)
        })
}
