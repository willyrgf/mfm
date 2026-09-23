//! Exact retained-contract decoding; Portfolio owns collection/context agreement.
use mfm_chain::balance::{
    BalanceCollectionFailure, BalanceContext, BalanceFailureCode, BalanceRead,
    ConsolidateBalanceCollection, ObserveBalance, ObserveBalanceFailure, PreparedBalance,
};
use mfm_evm::{EvmBalanceFailure, EvmNativeBalance, EvmTokenBalance};
use mfm_portfolio::{EnrichmentContinuation, PortfolioContinuation, PortfolioSnapshotFailure};
use mfm_program::{
    nominal_contract_ref, state_implementation_ref, Execution, NativeAbi, State, StateDeclaration,
};
use mfm_values::{InvocationDiagnostic, MfmValue, Object};

fn invalid_projection(reason: &'static str) -> InvocationDiagnostic {
    InvocationDiagnostic::from_fields(
        "native_projection",
        "project_portfolio_failure",
        &reason,
        None,
    )
}
fn program_cause(cause: mfm_program::ProgramError) -> InvocationDiagnostic {
    match cause {
        mfm_program::ProgramError::Diagnostic(cause) => cause,
        cause => InvocationDiagnostic::from_fields(
            "program_contract",
            "project_portfolio_failure",
            &cause,
            None,
        ),
    }
}
fn state_matches<S: State>(declaration: &StateDeclaration) -> Result<bool, InvocationDiagnostic> {
    if declaration.state_implementation_ref()
        != &state_implementation_ref::<S>().map_err(program_cause)?
    {
        return Ok(false);
    }
    if declaration.input_contract_ref()
        != &nominal_contract_ref::<S::Input>().map_err(program_cause)?
        || declaration.output_contract_ref()
            != &nominal_contract_ref::<S::Output>().map_err(program_cause)?
        || declaration.failure_contract_ref()
            != &nominal_contract_ref::<S::Failure>().map_err(program_cause)?
    {
        return Err(invalid_projection("state_abi_mismatch"));
    }
    Ok(true)
}
fn native_code(original: &Object, chain: bool) -> Result<BalanceFailureCode, InvocationDiagnostic> {
    Ok(match original.decode::<EvmBalanceFailure>()? {
        EvmBalanceFailure::AnchorChanged { .. } => BalanceFailureCode::AnchorChanged,
        EvmBalanceFailure::ObservationRejected { .. } if chain => {
            BalanceFailureCode::ChainIdentityUnavailable
        }
        EvmBalanceFailure::ObservationRejected { .. } => BalanceFailureCode::ObservationUnavailable,
        EvmBalanceFailure::Collection { source } => source.code(),
        EvmBalanceFailure::IntegrityBlocked => BalanceFailureCode::IntegrityBlocked,
    })
}

/// Projects a snapshot domain original through its exact retained contracts, without Runtime or IO.
pub fn snapshot_failure(
    state: &StateDeclaration,
    input: &Object,
    original: &Object,
) -> Result<PortfolioSnapshotFailure, InvocationDiagnostic> {
    if let Some(projected) = collection::<PortfolioContinuation>(
        state,
        input,
        original,
        PortfolioContinuation::project_collection_failure,
    )? {
        return Ok(projected);
    }
    if state_matches::<mfm_portfolio::ConsolidatePortfolio>(state)? {
        if !matches!(state.execution(), Execution::Pure {}) {
            return Err(invalid_projection("product_state_mode"));
        }
        input.decode::<<mfm_portfolio::ConsolidatePortfolio as State>::Input>()?;
        let original =
            original.decode::<<mfm_portfolio::ConsolidatePortfolio as State>::Failure>()?;
        return Ok(match original {
            mfm_portfolio::PortfolioConsolidationFailure::AggregateCapacityExceeded => {
                PortfolioSnapshotFailure::ConsolidationFailed
            }
        });
    }
    Err(invalid_projection("unknown_snapshot_state"))
}
/// Projects an enrichment domain original through its exact retained contracts, without Runtime or IO.
pub fn enrichment_failure(
    state: &StateDeclaration,
    input: &Object,
    original: &Object,
) -> Result<PortfolioSnapshotFailure, InvocationDiagnostic> {
    if let Some(projected) = collection::<EnrichmentContinuation>(
        state,
        input,
        original,
        EnrichmentContinuation::project_collection_failure,
    )? {
        return Ok(projected);
    }
    Err(invalid_projection("unknown_enrichment_state"))
}
fn collection<K: MfmValue>(
    declaration: &StateDeclaration,
    input: &Object,
    original: &Object,
    project: impl Fn(
        &BalanceContext<K>,
        BalanceFailureCode,
    ) -> Result<PortfolioSnapshotFailure, InvocationDiagnostic>,
) -> Result<Option<PortfolioSnapshotFailure>, InvocationDiagnostic> {
    if let Some(projected) =
        mfm_evm::project_balance_context::<K, _>(declaration, input, |stage, context| {
            context.active_source().map_err(|cause| {
                InvocationDiagnostic::from_fields(
                    "native_projection",
                    "active_source",
                    &cause,
                    None,
                )
            })?;
            project(
                context,
                native_code(
                    original,
                    stage == mfm_evm::NativeBalanceStage::ChainIdentity,
                )?,
            )
        })?
    {
        return Ok(Some(projected));
    }
    if state_matches::<ObserveBalance<K>>(declaration)? {
        let Execution::Read { abi, .. } = declaration.execution() else {
            return Err(invalid_projection("observation_mode"));
        };
        if abi != &NativeAbi::read::<BalanceRead, EvmNativeBalance>().map_err(program_cause)?
            && abi != &NativeAbi::read::<BalanceRead, EvmTokenBalance>().map_err(program_cause)?
        {
            return Err(invalid_projection("observation_native_abi"));
        }
        let input = input.decode::<PreparedBalance<K>>()?;
        let code = original.decode::<ObserveBalanceFailure>()?.code();
        return project(input.context(), code).map(Some);
    }
    if state_matches::<ConsolidateBalanceCollection<K>>(declaration)? {
        if !matches!(declaration.execution(), Execution::Pure {}) {
            return Err(invalid_projection("consolidation_mode"));
        }
        let input = input.decode::<BalanceContext<K>>()?;
        input
            .request()
            .validate_confirmed(input.completed())
            .map_err(|cause| {
                InvocationDiagnostic::from_fields(
                    "native_projection",
                    "complete_collection",
                    &cause,
                    None,
                )
            })?;
        let code = original.decode::<BalanceCollectionFailure>()?.code();
        return project(&input, code).map(Some);
    }
    Ok(None)
}
