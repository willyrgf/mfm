use crate::{EntryPointOpRegistry, OpLaunchError, TypedEntryPointOp};

/// Builds the production entry-point operation registry for this process.
pub(crate) fn production_entry_point_op_registry() -> Result<EntryPointOpRegistry, crate::AppError>
{
    let mut registry = EntryPointOpRegistry::new();
    registry.register(TypedEntryPointOp::new(
        mfm_op_portfolio_tracker::PORTFOLIO_SNAPSHOT_ENTRY_POINT,
        mfm_op_portfolio_tracker::plan_portfolio_snapshot_entry_point,
        portfolio_snapshot_plan_error,
    )?)?;
    registry.register(TypedEntryPointOp::new(
        mfm_op_evm_contract_lifecycle::CONTRACT_DEPLOY_ENTRY_POINT,
        mfm_op_evm_contract_lifecycle::plan_contract_deploy_entry_point,
        evm_contract_plan_error,
    )?)?;
    registry.register(TypedEntryPointOp::new(
        mfm_op_evm_contract_lifecycle::CONTRACT_CONFIGURE_ENTRY_POINT,
        mfm_op_evm_contract_lifecycle::plan_contract_configure_entry_point,
        evm_contract_plan_error,
    )?)?;
    registry.register(TypedEntryPointOp::new(
        mfm_op_evm_contract_lifecycle::CONTRACT_VALIDATE_ENTRY_POINT,
        mfm_op_evm_contract_lifecycle::plan_contract_validate_entry_point,
        evm_contract_plan_error,
    )?)?;
    registry.register(TypedEntryPointOp::new(
        mfm_op_evm_contract_lifecycle::CONTRACT_LIFECYCLE_ENTRY_POINT,
        mfm_op_evm_contract_lifecycle::plan_contract_lifecycle_entry_point,
        evm_contract_plan_error,
    )?)?;
    Ok(registry)
}

fn portfolio_snapshot_plan_error(
    _error: mfm_op_portfolio_tracker::PortfolioSnapshotPlanError,
) -> OpLaunchError {
    OpLaunchError::new(
        "PortfolioSnapshotPlanFailed",
        "portfolio snapshot entry-point planning failed",
    )
}

fn evm_contract_plan_error(
    _error: mfm_op_evm_contract_lifecycle::ContractLifecyclePlanError,
) -> OpLaunchError {
    OpLaunchError::new(
        "EvmContractPlanFailed",
        "EVM contract entry-point planning failed",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{OpVersion, PublicOpName};
    use mfm_authored_config::{AuthoredConfig, AuthoredConfigFormat};

    #[test]
    fn production_registry_resolves_portfolio_snapshot_latest() {
        let registry = production_entry_point_op_registry().expect("registry");
        let name =
            PublicOpName::new(mfm_op_portfolio_tracker::PORTFOLIO_SNAPSHOT_ENTRY_POINT.public_name)
                .expect("name");
        let op = registry.resolve_latest(&name).expect("portfolio op");

        assert_eq!(op.version(), OpVersion::new(1).unwrap());
        assert_eq!(
            op.accepted_config_formats(),
            &[AuthoredConfigFormat::Toml, AuthoredConfigFormat::Json]
        );
    }

    #[test]
    fn portfolio_snapshot_entry_point_plans_from_json_config() {
        let registry = production_entry_point_op_registry().expect("registry");
        let name =
            PublicOpName::new(mfm_op_portfolio_tracker::PORTFOLIO_SNAPSHOT_ENTRY_POINT.public_name)
                .expect("name");
        let op = registry.resolve_latest(&name).expect("portfolio op");
        let authored =
            AuthoredConfig::new(AuthoredConfigFormat::Json, sample_portfolio_config_json())
                .expect("authored config");

        let plan = op.plan(authored).expect("portfolio plan");

        assert!(!plan.draft.state_nodes().is_empty());
        assert!(!plan.config_material.is_empty());
        assert!(plan.seed_material.is_empty());
    }

    #[test]
    fn app_prepare_entry_point_run_launch_records_evidence_and_certifies() {
        let entry_point_registry = production_entry_point_op_registry().expect("registry");
        let certification_registry = crate::production_certification_registry().expect("cert");
        let public_op_name =
            PublicOpName::new(mfm_op_portfolio_tracker::PORTFOLIO_SNAPSHOT_ENTRY_POINT.public_name)
                .expect("name");
        let authored =
            AuthoredConfig::new(AuthoredConfigFormat::Json, sample_portfolio_config_json())
                .expect("authored config");
        let registry_digest = entry_point_registry.registry_digest().expect("digest");
        let run_id = crate::new_run_id();

        let prepared = crate::prepare_entry_point_run_launch(crate::EntryPointRunLaunchInput {
            entry_point_registry: &entry_point_registry,
            public_op_name: public_op_name.clone(),
            op_version: None,
            authored_config: authored,
            certification_registry: &certification_registry,
            run_id: run_id.clone(),
            drive: crate::DriveMode::AppendOnly,
        })
        .expect("prepared entry-point launch");

        assert_eq!(
            prepared.evidence.resolved_op_id.name,
            mfm_op_portfolio_tracker::PORTFOLIO_SNAPSHOT_ENTRY_POINT.name
        );
        assert_eq!(
            prepared.evidence.entry_point_registry_digest,
            registry_digest
        );
        assert_eq!(prepared.request.run_id, run_id);
        assert!(!prepared.request.evidence.config_artifacts.is_empty());
    }

    #[test]
    fn evm_contract_entry_point_registry_resolves_all_versions() {
        let registry = production_entry_point_op_registry().expect("registry");

        for name in [
            "evm_contract_deploy",
            "evm_contract_configure",
            "evm_contract_validate",
            "evm_contract_lifecycle",
        ] {
            let op = registry
                .resolve_latest(&PublicOpName::new(name).expect("name"))
                .expect("EVM contract op");
            assert_eq!(op.version(), OpVersion::new(1).unwrap());
        }
    }

    #[test]
    fn evm_contract_entry_point_plans_deploy_and_lifecycle_without_seeds() {
        let registry = production_entry_point_op_registry().expect("registry");

        for (name, config) in [
            ("evm_contract_deploy", deploy_config_json().to_string()),
            (
                "evm_contract_lifecycle",
                lifecycle_config_json().to_string(),
            ),
        ] {
            let op = registry
                .resolve_latest(&PublicOpName::new(name).expect("name"))
                .expect("EVM contract op");
            let authored =
                AuthoredConfig::new(AuthoredConfigFormat::Json, config).expect("authored");
            let plan = op.plan(authored).expect("EVM contract plan");

            assert!(!plan.draft.state_nodes().is_empty());
            assert!(!plan.config_material.is_empty());
            assert!(plan.seed_material.is_empty());
        }
    }

    #[test]
    fn evm_contract_entry_point_plans_configure_and_validate_with_seed_material() {
        let registry = production_entry_point_op_registry().expect("registry");

        for (name, config) in [
            (
                "evm_contract_configure",
                configure_entry_config_json().to_string(),
            ),
            (
                "evm_contract_validate",
                validate_entry_config_json().to_string(),
            ),
        ] {
            let op = registry
                .resolve_latest(&PublicOpName::new(name).expect("name"))
                .expect("EVM contract op");
            let authored =
                AuthoredConfig::new(AuthoredConfigFormat::Json, config).expect("authored");
            let plan = op.plan(authored).expect("EVM contract plan");

            assert_eq!(plan.seed_material.len(), 1);
            assert_eq!(
                plan.seed_material[0].media_type.as_str(),
                "application/json"
            );
            assert_eq!(plan.seed_material[0].seed_id, plan.draft.seeds()[0].seed_id);
            assert!(!plan.config_material.is_empty());
        }
    }

    fn sample_portfolio_config_json() -> String {
        serde_json::json!({
            "portfolio": {
                "portfolio_id": "portfolio_main",
                "quote_codes": ["USD"],
                "networks": [
                    {
                        "network_id": "ethereum-mainnet",
                        "family": "evm",
                        "chain_id": 1,
                        "control_scope": "shared",
                        "metadata": {}
                    }
                ],
                "wallets": [
                    {
                        "wallet_id": "wallet_main",
                        "subject": {
                            "kind": "evm_address",
                            "address": "0x000000000000000000000000000000000000dead"
                        },
                        "implementation": { "kind": "address_only" },
                        "network_id": "ethereum-mainnet",
                        "symbol_ids": ["eth.native.ethereum-mainnet"],
                        "metadata": {}
                    }
                ],
                "symbol_configs": [
                    {
                        "symbol_id": "eth.native.ethereum-mainnet",
                        "display_symbol": "ETH",
                        "kind": "native_balance",
                        "role": "native",
                        "network_id": "ethereum-mainnet",
                        "protocol": null,
                        "balance_reader": { "kind": "native_balance" },
                        "valuation": {
                            "quotes": [
                                {
                                    "quote": "USD",
                                    "priced_symbol_id": "eth.native.ethereum-mainnet",
                                    "reader": {
                                        "kind": "fixed_unit_price",
                                        "unit_price_dec": "1800.00"
                                    }
                                }
                            ]
                        },
                        "decimals": 18,
                        "underlying_symbol_id": null,
                        "metadata": {}
                    }
                ],
                "metadata": {}
            },
            "valuation_source_registry": { "sources": [] }
        })
        .to_string()
    }

    fn network_json() -> serde_json::Value {
        serde_json::json!({
            "network_id": "ethereum-mainnet",
            "expected_chain_id": 1,
        })
    }

    fn signer_json() -> serde_json::Value {
        serde_json::json!({
            "signer_ref": "deployer",
            "expected_signer_address": "0x000000000000000000000000000000000000dead",
        })
    }

    fn deploy_config_json() -> serde_json::Value {
        serde_json::json!({
            "network": network_json(),
            "signer": signer_json(),
        })
    }

    fn configure_config_json() -> serde_json::Value {
        serde_json::json!({
            "network": network_json(),
            "signer": signer_json(),
            "calls": [],
        })
    }

    fn validate_config_json() -> serde_json::Value {
        serde_json::json!({
            "network": network_json(),
        })
    }

    fn lifecycle_config_json() -> serde_json::Value {
        serde_json::json!({
            "deploy": deploy_config_json(),
            "configure": configure_config_json(),
            "validate": validate_config_json(),
        })
    }

    fn deployed_contract_json() -> serde_json::Value {
        serde_json::json!({
            "lifecycle_version": 1,
            "network_id": "ethereum-mainnet",
            "expected_chain_id": 1,
            "contract_address": "0x000000000000000000000000000000000000dead",
            "deploy_tx_hash": "0x01",
            "deploy_receipt_evidence": null,
            "deployed_block_number": 1,
        })
    }

    fn configured_contract_json() -> serde_json::Value {
        serde_json::json!({
            "lifecycle_version": 1,
            "deployed": deployed_contract_json(),
            "configure_calls": [],
            "confirmation_read_assertions": [],
            "confirmation_event_assertions": [],
            "configure_tx_hashes": [],
            "configure_receipt_evidence": [],
            "configured_block_number": 2,
        })
    }

    fn configure_entry_config_json() -> serde_json::Value {
        serde_json::json!({
            "config": configure_config_json(),
            "deployed": deployed_contract_json(),
        })
    }

    fn validate_entry_config_json() -> serde_json::Value {
        serde_json::json!({
            "config": validate_config_json(),
            "configured": configured_contract_json(),
        })
    }
}
