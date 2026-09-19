//! Checked shared collection entry and product-specific resumption.
use super::*;
use mfm_chain::balance::{BalanceCollectionMetadata, ConfirmedBalance};
use mfm_values::InvocationDiagnostic;

#[derive(Debug, Serialize, thiserror::Error)]
enum HandoffError {
    #[error("Portfolio collection ordinal differs from the retained continuation")]
    Ordinal { expected: Option<u32>, actual: u32 },
    #[error("Portfolio collection request differs from the retained demand")]
    Request,
    #[error("Portfolio collection metadata differs from the retained demand")]
    Metadata,
}
fn rejected(source: HandoffError) -> InvocationDiagnostic {
    InvocationDiagnostic::from_fields(
        "state_internal",
        "resume_portfolio_collection",
        &source,
        None,
    )
}

impl PureState for EnterPortfolioCollection {
    fn evaluate(
        input: PortfolioContinuation,
    ) -> Result<ProposedStateOutcome<Self::Output, Self::Failure>, InvocationDiagnostic> {
        let (request, metadata) = enter(&input)?;
        Ok(portfolio_success(BalanceContext::new(
            request, input, metadata,
        )))
    }
}

pub(super) fn enter(
    input: &PortfolioContinuation,
) -> Result<(BalanceRequest, BalanceCollectionMetadata), InvocationDiagnostic> {
    let ordinal = input.next_collection_ordinal().ok_or_else(|| {
        InvocationDiagnostic::from_fields(
            "state_internal",
            "enter_portfolio_collection",
            &PortfolioError::InvalidContinuation,
            None,
        )
    })?;
    let demand = input.input.collection(ordinal as usize).ok_or_else(|| {
        InvocationDiagnostic::from_fields(
            "state_internal",
            "enter_portfolio_collection",
            &PortfolioError::InvalidContinuation,
            None,
        )
    })?;
    let metadata = BalanceCollectionMetadata::new(
        ordinal,
        demand.correlation.clone(),
        demand
            .route_ref()
            .map_err(|source| source.into_diagnostic("enter_portfolio_collection"))?
            .clone(),
    )
    .map_err(|source| {
        InvocationDiagnostic::from_fields(
            "state_internal",
            "enter_portfolio_collection",
            &source,
            None,
        )
    })?;
    let request = demand.request.clone();
    Ok((request, metadata))
}

impl PureState for ResumePortfolioCollection {
    fn evaluate(
        input: BalanceCollectionCompletion<PortfolioContinuation>,
    ) -> Result<ProposedStateOutcome<Self::Output, Self::Failure>, InvocationDiagnostic> {
        let (context, total_scaled) = input.into_parts();
        let (request, continuation, metadata, confirmed) = context.into_parts();
        resume(continuation, request, metadata, confirmed, total_scaled).map(portfolio_success)
    }
}

pub(super) fn resume(
    mut continuation: PortfolioContinuation,
    request: BalanceRequest,
    metadata: BalanceCollectionMetadata,
    confirmed: Vec<ConfirmedBalance>,
    total_scaled: String,
) -> Result<PortfolioContinuation, InvocationDiagnostic> {
    let ordinal = metadata.collection_ordinal();
    let expected = continuation.next_collection_ordinal();
    if expected != Some(ordinal) {
        return Err(rejected(HandoffError::Ordinal {
            expected,
            actual: ordinal,
        }));
    }
    let demand = continuation
        .input
        .collection(ordinal as usize)
        .ok_or_else(|| {
            rejected(HandoffError::Ordinal {
                expected,
                actual: ordinal,
            })
        })?;
    if request != demand.request {
        return Err(rejected(HandoffError::Request));
    }
    if metadata.correlation() != demand.correlation
        || metadata.route_ref()
            != demand
                .route_ref()
                .map_err(|source| source.into_diagnostic("resume_portfolio_collection"))?
    {
        return Err(rejected(HandoffError::Metadata));
    }
    let collection = PortfolioSnapshotCollection {
        metadata,
        decimals: request.decimals(),
        balances: confirmed,
        executions: demand.executions.clone(),
        total_scaled,
    };
    continuation.completed_collections.push(collection);
    continuation
        .validate()
        .map_err(|source| source.into_diagnostic("resume_portfolio_collection"))?;
    Ok(continuation)
}

pub(super) fn project_failure<K: mfm_values::MfmValue>(
    context: &BalanceContext<K>,
    continuation: &PortfolioContinuation,
    code: BalanceFailureCode,
) -> Result<PortfolioSnapshotFailure, InvocationDiagnostic> {
    continuation
        .validate()
        .map_err(|cause| cause.into_diagnostic("project_collection_failure"))?;
    let (request, metadata) = enter(continuation)?;
    if context.request() != &request {
        return Err(rejected(HandoffError::Request));
    }
    if context.metadata() != &metadata {
        return Err(rejected(HandoffError::Metadata));
    }
    let failure = PortfolioSnapshotFailure::CollectionFailed {
        ordinal: metadata.collection_ordinal() as u16,
        code,
    };
    failure
        .validate()
        .map_err(|cause| cause.into_diagnostic("project_collection_failure"))?;
    Ok(failure)
}

impl PortfolioContinuation {
    /// Projects an exact collection failure only after checking its retained product context.
    pub fn project_collection_failure(
        context: &BalanceContext<Self>,
        code: BalanceFailureCode,
    ) -> Result<PortfolioSnapshotFailure, InvocationDiagnostic> {
        project_failure(context, context.caller(), code)
    }
}

#[cfg(test)]
mod tests;
