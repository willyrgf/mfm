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

fn runtime_artifact_read_error(_error: store::StoreError) -> mfm_runtime::RuntimeError {
    mfm_runtime::RuntimeError::InvalidRunnerOutput(
        "retained holding artifact was unavailable or invalid".to_owned(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_source_fact_bytes_fail_closed_without_rendering_bytes() {
        let secret = b"not-json-private-material";
        let kind = mfm_facts::FactKind::new("evm.balance_snapshot").expect("fact kind");
        let error = PortfolioHoldingFactEvidence::from_canonical_bytes(&kind, secret)
            .expect_err("invalid canonical response bytes");

        assert!(!error.to_string().contains("private-material"));
    }

    #[test]
    fn canonical_bytes_from_another_fact_family_fail_closed() {
        let bitcoin_response = br#"{"anchor_hash":"0000000000000000000000000000000000000000000000000000000000000000","anchor_height":1,"balance_sats":1}"#;
        let bitcoin_kind = mfm_facts::FactKind::new("bitcoin.balance_snapshot").expect("fact kind");
        let evm_kind = mfm_facts::FactKind::new("evm.balance_snapshot").expect("fact kind");

        PortfolioHoldingFactEvidence::from_canonical_bytes(&bitcoin_kind, bitcoin_response)
            .expect("valid Bitcoin response bytes");
        PortfolioHoldingFactEvidence::from_canonical_bytes(&evm_kind, bitcoin_response)
            .expect_err("Bitcoin response bytes must not hydrate as EVM evidence");
    }

    #[test]
    fn artifact_failures_are_redacted() {
        let error = runtime_artifact_read_error(store::StoreError::Identity(
            "private backend artifact path".to_owned(),
        ));
        assert!(!error.to_string().contains("private backend artifact path"));
    }
}
