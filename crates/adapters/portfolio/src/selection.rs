use mfm_facts::FactQueryResult;
use mfm_portfolio::{PortfolioHoldingFactEvidence, SelectHoldingsReadPlan};
use mfm_store::v1 as store;

/// Hydrates every retained fact response in query-row order without applying selection policy.
pub(crate) async fn hydrate_holding_responses(
    plan: &SelectHoldingsReadPlan,
    responses: &[FactQueryResult],
    artifacts: &dyn store::RetainedArtifactReadProvider,
) -> mfm_runtime::Result<Vec<Vec<PortfolioHoldingFactEvidence>>> {
    let holding_count = plan.holding_count().map_err(hydration_error)?;
    if responses.len() != holding_count {
        return Err(mfm_runtime::RuntimeError::InvalidRunnerOutput(
            "fact-index batch response count does not match receipt demand".to_owned(),
        ));
    }
    let mut hydrated = Vec::with_capacity(responses.len());
    for response in responses {
        let mut query_material = Vec::with_capacity(response.rows().len());
        for row in response.rows() {
            let fact_ref = row.fact_ref();
            let requirement = store::fact_response_artifact_requirement(fact_ref);
            let artifact = artifacts
                .read_retained_artifact(&requirement)
                .await
                .map_err(runtime_artifact_read_error)?;
            let material = PortfolioHoldingFactEvidence::from_canonical_bytes(
                fact_ref.fact_kind(),
                artifact.bytes(),
            )
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
