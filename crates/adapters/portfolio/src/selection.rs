use mfm_artifact_capabilities::fact_response_artifact_requirement;
use mfm_facts::FactQueryResult;
use mfm_state_portfolio::{PortfolioHoldingFactResponse, SelectHoldingsReadPlan};
use mfm_store::v1 as store;

/// Hydrates every retained fact response in query-row order without applying selection policy.
pub(crate) async fn hydrate_holding_responses(
    plan: &SelectHoldingsReadPlan,
    responses: &[FactQueryResult],
    artifacts: &dyn store::RetainedArtifactReadProvider,
) -> mfm_runtime::Result<Vec<Vec<PortfolioHoldingFactResponse>>> {
    if responses.len() != plan.receipt().holdings().len() {
        return Err(mfm_runtime::RuntimeError::InvalidRunnerOutput(
            "fact-index batch response count does not match receipt demand".to_owned(),
        ));
    }
    let mut hydrated = Vec::with_capacity(responses.len());
    for response in responses {
        let mut query_material = Vec::with_capacity(response.rows().len());
        for row in response.rows() {
            let fact_ref = row.fact_ref();
            let requirement = fact_response_artifact_requirement(fact_ref);
            let artifact = artifacts
                .read_retained_artifact(&requirement)
                .await
                .map_err(runtime_artifact_read_error)?;
            let material = PortfolioHoldingFactResponse::from_canonical_bytes(artifact.bytes())
                .map_err(hydration_error)?;
            query_material.push(material);
        }
        hydrated.push(query_material);
    }
    Ok(hydrated)
}

fn hydration_error(error: impl std::fmt::Display) -> mfm_runtime::RuntimeError {
    mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string())
}

fn runtime_artifact_read_error(error: store::StoreError) -> mfm_runtime::RuntimeError {
    mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string())
}

#[cfg(test)]
pub(crate) async fn select_holdings(
    config: mfm_program::ValidatedConfig<mfm_state_portfolio::SelectHoldingsConfig>,
    input: mfm_state_portfolio::SelectHoldingsInput,
    artifacts: &dyn store::RetainedArtifactReadProvider,
    fact_index: &dyn mfm_fact_capabilities::FactIndexReadProvider,
) -> mfm_runtime::Result<(
    mfm_state_portfolio::SelectedHoldings,
    Vec<mfm_facts::FactQueryEvidence>,
)> {
    use mfm_program::{ReadState, StateSpec};

    let state = mfm_state_portfolio::SelectHoldingsState::new(config)
        .map_err(|error| mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string()))?;
    let context = mfm_program::CertifiedContext::no_context();
    let plan = state
        .plan(&input, &context)
        .map_err(|error| mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string()))?;
    let requests = plan
        .requests()
        .map_err(|error| mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string()))?;
    let responses = if requests.is_empty() {
        Vec::new()
    } else {
        fact_index
            .read_fact_index_batch(&requests)
            .await
            .map_err(|error| mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string()))?
    };
    plan.validate_query_results(&responses)
        .map_err(|error| mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string()))?;
    let hydrated = hydrate_holding_responses(&plan, &responses, artifacts).await?;
    let queries = plan
        .query_evidence(&responses, &hydrated)
        .map_err(|error| mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string()))?;
    let primary = mfm_state_portfolio::SelectHoldingsReadEvidence::new(&queries, hydrated)
        .map_err(|error| mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string()))?;
    let evidence = mfm_program::ExternalReadEvidenceSet::new(primary, queries.clone());
    let selected = state
        .reduce(&input, &evidence, &context)
        .map_err(|error| mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string()))?;
    Ok((selected, queries))
}
