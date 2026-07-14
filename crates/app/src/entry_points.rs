use crate::{EntryPointOpError, EntryPointOpRegistry, EntryPointPlannerAdapter};

/// Builds the production entry-point operation registry for this process.
pub(crate) fn production_entry_point_op_registry() -> Result<EntryPointOpRegistry, crate::AppError>
{
    let mut registry = EntryPointOpRegistry::new();
    registry.register(EntryPointPlannerAdapter::new(
        mfm_op_portfolio_tracker::PORTFOLIO_SNAPSHOT_ENTRY_POINT,
        mfm_op_portfolio_tracker::plan_portfolio_snapshot_entry_point,
        portfolio_snapshot_plan_error,
    )?)?;
    registry.register(EntryPointPlannerAdapter::new(
        mfm_op_evm_contract_lifecycle::CONTRACT_DEPLOY_ENTRY_POINT,
        mfm_op_evm_contract_lifecycle::plan_contract_deploy_entry_point,
        evm_contract_plan_error,
    )?)?;
    registry.register(EntryPointPlannerAdapter::new(
        mfm_op_evm_contract_lifecycle::CONTRACT_CONFIGURE_ENTRY_POINT,
        mfm_op_evm_contract_lifecycle::plan_contract_configure_entry_point,
        evm_contract_plan_error,
    )?)?;
    registry.register(EntryPointPlannerAdapter::new(
        mfm_op_evm_contract_lifecycle::CONTRACT_VALIDATE_ENTRY_POINT,
        mfm_op_evm_contract_lifecycle::plan_contract_validate_entry_point,
        evm_contract_plan_error,
    )?)?;
    registry.register(EntryPointPlannerAdapter::new(
        mfm_op_evm_contract_lifecycle::CONTRACT_LIFECYCLE_ENTRY_POINT,
        mfm_op_evm_contract_lifecycle::plan_contract_lifecycle_entry_point,
        evm_contract_plan_error,
    )?)?;
    registry.register(EntryPointPlannerAdapter::new(
        mfm_op_btc_collectors::BTC_ADDRESS_BALANCE_ENTRY_POINT,
        mfm_op_btc_collectors::plan_btc_address_balance_entry_point,
        btc_collector_plan_error,
    )?)?;
    registry.register(EntryPointPlannerAdapter::new(
        mfm_op_evm_collectors::EVM_NATIVE_BALANCE_ENTRY_POINT,
        mfm_op_evm_collectors::plan_evm_native_balance_entry_point,
        evm_collector_plan_error,
    )?)?;
    // btc_chain_head_collector_cycle remains off the public entry-point registry: it is a
    // control checkpoint / chain-head surface, not report pin authority or a balance collector.
    Ok(registry)
}

fn portfolio_snapshot_plan_error(
    error: mfm_op_portfolio_tracker::PortfolioSnapshotPlanError,
) -> EntryPointOpError {
    match error {
        mfm_op_portfolio_tracker::PortfolioSnapshotPlanError::Config(_) => EntryPointOpError::new(
            "PortfolioSnapshotConfigInvalid",
            "portfolio snapshot config validation failed",
        ),
        mfm_op_portfolio_tracker::PortfolioSnapshotPlanError::Plan(_) => EntryPointOpError::new(
            "PortfolioSnapshotPlanFailed",
            "portfolio snapshot entry-point planning failed",
        ),
    }
}

fn evm_contract_plan_error(_error: mfm_program::PlanError) -> EntryPointOpError {
    EntryPointOpError::new(
        "EvmContractPlanFailed",
        "EVM contract entry-point planning failed",
    )
}

fn btc_collector_plan_error(_error: mfm_program::PlanError) -> EntryPointOpError {
    EntryPointOpError::new(
        "BtcAddressBalancePlanFailed",
        "Bitcoin address-balance collector entry-point planning failed",
    )
}

fn evm_collector_plan_error(_error: mfm_program::PlanError) -> EntryPointOpError {
    EntryPointOpError::new(
        "EvmNativeBalancePlanFailed",
        "EVM native-balance collector entry-point planning failed",
    )
}

#[cfg(test)]
#[path = "entry_points_tests.rs"]
mod tests;
