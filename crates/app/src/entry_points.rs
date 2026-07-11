use crate::{EntryPointOpRegistry, EntryPointPlannerAdapter, OpLaunchError};

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
) -> OpLaunchError {
    match error {
        mfm_op_portfolio_tracker::PortfolioSnapshotPlanError::Config(_) => OpLaunchError::new(
            "PortfolioSnapshotConfigInvalid",
            "portfolio snapshot config validation failed",
        ),
        mfm_op_portfolio_tracker::PortfolioSnapshotPlanError::Plan(_) => OpLaunchError::new(
            "PortfolioSnapshotPlanFailed",
            "portfolio snapshot entry-point planning failed",
        ),
    }
}

fn evm_contract_plan_error(_error: mfm_program::PlanError) -> OpLaunchError {
    OpLaunchError::new(
        "EvmContractPlanFailed",
        "EVM contract entry-point planning failed",
    )
}

fn btc_collector_plan_error(_error: mfm_program::PlanError) -> OpLaunchError {
    OpLaunchError::new(
        "BtcAddressBalancePlanFailed",
        "Bitcoin address-balance collector entry-point planning failed",
    )
}

fn evm_collector_plan_error(_error: mfm_program::PlanError) -> OpLaunchError {
    OpLaunchError::new(
        "EvmNativeBalancePlanFailed",
        "EVM native-balance collector entry-point planning failed",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{OpVersion, PublicOpName};
    use mfm_authored_config::{AuthoredConfig, AuthoredConfigFormat};
    use mfm_store::v1::{
        self as store, AdmissionToken, ExecutionClaimStatus, ExecutionClaimStore,
        NowaitSkipAdmissionResult, RunEventStore, StoreScopeStore,
    };
    use std::sync::Arc;

    fn portfolio_public_op_name() -> PublicOpName {
        PublicOpName::new(mfm_op_portfolio_tracker::PORTFOLIO_SNAPSHOT_ENTRY_POINT.public_name)
            .expect("name")
    }

    fn portfolio_op(registry: &EntryPointOpRegistry) -> Arc<dyn crate::LaunchableOp> {
        registry
            .resolve_latest(&portfolio_public_op_name())
            .expect("portfolio op")
    }

    #[test]
    fn production_registry_resolves_portfolio_snapshot_latest() {
        let registry = production_entry_point_op_registry().expect("registry");
        let op = portfolio_op(&registry);

        assert_eq!(op.descriptor().version, 1);
        assert_eq!(
            registry
                .resolve_version(&portfolio_public_op_name(), OpVersion::new(1).unwrap())
                .expect("portfolio snapshot v1")
                .descriptor()
                .version,
            1
        );
        assert_eq!(
            op.descriptor().accepted_config_formats,
            &[AuthoredConfigFormat::Toml, AuthoredConfigFormat::Json]
        );
    }

    #[test]
    fn production_registry_rejects_deleted_portfolio_snapshot_v2() {
        let registry = production_entry_point_op_registry().expect("registry");
        let error = match registry
            .resolve_version(&portfolio_public_op_name(), OpVersion::new(2).unwrap())
        {
            Ok(_) => panic!("portfolio snapshot v2 was deleted"),
            Err(error) => error,
        };

        assert_eq!(error.code(), "EntryPointOpVersionNotFound");
    }

    #[test]
    fn production_registry_resolves_balance_collector_entry_points() {
        let registry = production_entry_point_op_registry().expect("registry");
        let btc =
            PublicOpName::new(mfm_op_btc_collectors::BTC_ADDRESS_BALANCE_ENTRY_POINT.public_name)
                .expect("btc name");
        let evm =
            PublicOpName::new(mfm_op_evm_collectors::EVM_NATIVE_BALANCE_ENTRY_POINT.public_name)
                .expect("evm name");

        let btc_op = registry.resolve_latest(&btc).expect("btc address balance");
        let evm_op = registry.resolve_latest(&evm).expect("evm native balance");
        assert_eq!(btc_op.descriptor().version, 1);
        assert_eq!(evm_op.descriptor().version, 1);
        assert_eq!(
            btc_op.descriptor().accepted_config_formats,
            &[AuthoredConfigFormat::Toml, AuthoredConfigFormat::Json]
        );
        assert_eq!(
            evm_op.descriptor().accepted_config_formats,
            &[AuthoredConfigFormat::Toml, AuthoredConfigFormat::Json]
        );
    }

    #[test]
    fn production_registry_does_not_expose_btc_chain_head_as_public_entry_point() {
        let registry = production_entry_point_op_registry().expect("registry");
        let public_name = PublicOpName::new("btc_chain_head_collector").expect("name");

        let error = match registry.resolve_latest(&public_name) {
            Ok(_) => panic!("chain-head collector is not a public balance entry point"),
            Err(error) => error,
        };

        assert_eq!(error.code(), "EntryPointOpNotFound");
    }

    #[test]
    fn btc_address_balance_entry_point_plans_from_json_config() {
        let registry = production_entry_point_op_registry().expect("registry");
        let public_name =
            PublicOpName::new(mfm_op_btc_collectors::BTC_ADDRESS_BALANCE_ENTRY_POINT.public_name)
                .expect("name");
        let op = registry.resolve_latest(&public_name).expect("btc op");
        let authored = AuthoredConfig::new(
            AuthoredConfigFormat::Json,
            r#"{
                "network": "bitcoin-mainnet",
                "bitcoin_network": "main",
                "semantic_source_identity": "public-bitcoin-core",
                "addresses": ["bc1qxy2kgdygjrsqtzq2n0yrf2493p83kkfjhx0wlh"],
                "coverage": "configured_only",
                "max_source_reads": 1
            }"#,
        )
        .expect("authored");
        let plan = op.plan(authored).expect("btc plan");
        assert!(!plan.draft.state_nodes().is_empty());
        assert!(!plan.config_material.is_empty());
    }

    #[test]
    fn btc_address_balance_entry_point_rejects_removed_head_selection_fields() {
        let registry = production_entry_point_op_registry().expect("registry");
        let public_name =
            PublicOpName::new(mfm_op_btc_collectors::BTC_ADDRESS_BALANCE_ENTRY_POINT.public_name)
                .expect("name");
        let op = registry.resolve_latest(&public_name).expect("btc op");
        let authored = AuthoredConfig::new(
            AuthoredConfigFormat::Json,
            r#"{
                "network": "bitcoin-mainnet",
                "bitcoin_network": "main",
                "semantic_source_identity": "public-bitcoin-core",
                "addresses": ["bc1qxy2kgdygjrsqtzq2n0yrf2493p83kkfjhx0wlh"],
                "head_kind": "best",
                "coverage": "configured_only",
                "max_source_reads": 1
            }"#,
        )
        .expect("authored");

        let error = op.plan(authored).expect_err("removed field must reject");

        assert_eq!(error.code(), "AuthoredConfigUnknownField");
    }

    #[test]
    fn evm_native_balance_entry_point_plans_from_json_config() {
        let registry = production_entry_point_op_registry().expect("registry");
        let public_name =
            PublicOpName::new(mfm_op_evm_collectors::EVM_NATIVE_BALANCE_ENTRY_POINT.public_name)
                .expect("name");
        let op = registry.resolve_latest(&public_name).expect("evm op");
        let authored = AuthoredConfig::new(
            AuthoredConfigFormat::Json,
            r#"{
                "network": "ethereum-mainnet",
                "chain_id": 1,
                "accounts": ["0x0000000000000000000000000000000000000001"],
                "coverage": "configured_only",
                "decimals": 18,
                "max_source_reads": 1
            }"#,
        )
        .expect("authored");
        let plan = op.plan(authored).expect("evm plan");
        assert!(!plan.draft.state_nodes().is_empty());
        assert!(!plan.config_material.is_empty());
    }

    #[test]
    fn production_registry_plans_tracked_collect_then_report_configs() {
        let registry = production_entry_point_op_registry().expect("registry");
        for (name, raw) in [
            (
                "btc_address_balance",
                include_str!("../../../examples/configs/btc-address-balance.toml"),
            ),
            (
                "evm_native_balance",
                include_str!("../../../examples/configs/evm-native-balance.toml"),
            ),
            (
                "portfolio_snapshot",
                include_str!("../../../examples/configs/portfolio-dual-mainnet.toml"),
            ),
        ] {
            let public_name = PublicOpName::new(name).expect("public name");
            let op = registry.resolve_latest(&public_name).expect("tracked op");
            let authored = AuthoredConfig::new(AuthoredConfigFormat::Toml, raw)
                .expect("tracked authored config");
            let plan = op.plan(authored).expect("tracked config plan");

            assert!(!plan.draft.state_nodes().is_empty(), "{name}");
            assert!(!plan.config_material.is_empty(), "{name}");
        }
    }

    #[test]
    fn portfolio_snapshot_entry_point_plans_from_json_config() {
        let registry = production_entry_point_op_registry().expect("registry");
        let op = portfolio_op(&registry);
        let authored =
            AuthoredConfig::new(AuthoredConfigFormat::Json, sample_portfolio_config_json())
                .expect("authored config");

        let plan = op.plan(authored).expect("portfolio plan");

        assert!(!plan.draft.state_nodes().is_empty());
        assert!(!plan.config_material.is_empty());
        assert!(plan.seed_material.is_empty());
    }

    #[test]
    fn portfolio_snapshot_entry_point_preserves_config_validation_error_code() {
        let registry = production_entry_point_op_registry().expect("registry");
        let op = portfolio_op(&registry);
        let mut config: serde_json::Value =
            serde_json::from_str(&sample_portfolio_config_json()).expect("portfolio json");
        config["portfolio"]["wallets"][0]["network_id"] = serde_json::json!("missing-network");
        let authored = AuthoredConfig::new(AuthoredConfigFormat::Json, config.to_string())
            .expect("authored config");

        let error = op.plan(authored).expect_err("invalid portfolio config");

        assert_eq!(error.code(), "PortfolioSnapshotConfigInvalid");
    }

    #[test]
    fn app_prepare_entry_point_run_launch_records_evidence_and_certifies() {
        let fixture =
            EntryPointPrepFixture::with_store_scope_hex("50505050505050505050505050505050");
        let registry_digest = fixture
            .entry_point_registry
            .registry_digest()
            .expect("digest");
        let prepared = fixture.prepare_sample_portfolio(None);

        assert_eq!(
            prepared.evidence.resolved_op_id.name.as_str(),
            mfm_op_portfolio_tracker::PORTFOLIO_SNAPSHOT_ENTRY_POINT.name
        );
        assert_eq!(
            prepared.evidence.entry_point_registry_digest,
            registry_digest
        );
        assert_eq!(
            prepared.request.run_id,
            prepared
                .request
                .identity_material
                .derive_run_id()
                .expect("run id")
        );
        assert!(!prepared.request.evidence.config_artifacts.is_empty());
    }

    #[test]
    fn app_prepare_dual_mainnet_portfolio_run_launch_certifies() {
        let fixture =
            EntryPointPrepFixture::with_store_scope_hex("53535353535353535353535353535353");
        let authored_config = AuthoredConfig::new(
            AuthoredConfigFormat::Toml,
            include_str!("../../../examples/configs/portfolio-dual-mainnet.toml"),
        )
        .expect("authored config");

        crate::prepare_entry_point_run_launch(crate::EntryPointRunLaunchInput {
            entry_point_registry: &fixture.entry_point_registry,
            public_op_name: portfolio_public_op_name(),
            op_version: None,
            authored_config,
            certification_registry: &fixture.certification_registry,
            store_scope_id: fixture.store_scope_id,
            invocation_key: None,
        })
        .expect("dual-mainnet portfolio launch should certify");
    }

    #[test]
    fn app_prepare_entry_point_run_launch_derives_fresh_and_invocation_stable_run_ids() {
        let fixture =
            EntryPointPrepFixture::with_store_scope_hex("51515151515151515151515151515151");

        let first = fixture.prepare_sample_portfolio(None);
        let second = fixture.prepare_sample_portfolio(None);
        let explicit = crate::InvocationKey::new("alpha").expect("invocation key");
        let explicit_first = fixture.prepare_sample_portfolio(Some(explicit.clone()));
        let explicit_second = fixture.prepare_sample_portfolio(Some(explicit));
        let mut changed_config: serde_json::Value =
            serde_json::from_str(&sample_portfolio_config_json()).expect("portfolio json");
        changed_config["portfolio"]["portfolio_id"] = serde_json::json!("portfolio_other");
        let changed = fixture.prepare_portfolio_launch(changed_config.to_string(), None);

        assert_ne!(first.request.run_id, second.request.run_id);
        assert_eq!(
            first.request.certified_spec.spec_hash(),
            second.request.certified_spec.spec_hash()
        );
        assert_eq!(
            first.request.certified_spec.spec_hash(),
            explicit_first.request.certified_spec.spec_hash()
        );
        assert_eq!(
            explicit_first.request.run_id,
            explicit_second.request.run_id
        );
        assert_ne!(first.request.run_id, explicit_first.request.run_id);
        assert_eq!(
            explicit_first
                .request
                .identity_material
                .invocation_key_digest
                .as_str(),
            "content:sha256-jcs-v1:e8b2dba1a1730580547875fc9f27a3edf6a5c62660bdc3a6ac06518b487ecd43"
        );
        assert!(!format!("{:?}", explicit_first.request).contains("alpha"));
        assert_ne!(
            first.request.certified_spec.spec_hash(),
            changed.request.certified_spec.spec_hash()
        );
        assert_ne!(first.request.run_id, changed.request.run_id);
    }

    #[tokio::test]
    async fn app_launch_duplicate_same_spec_attaches_without_second_admission() {
        let fixture = EntryPointRunFixture::in_memory().await;
        let invocation_key =
            crate::InvocationKey::new("mfm.test.app.duplicate-attach").expect("key");
        let first = fixture.prepare_sample_portfolio(Some(invocation_key.clone()));
        let second = fixture.prepare_sample_portfolio(Some(invocation_key));
        let run_id = first.request.run_id.clone();
        let services = fixture.services();

        let first_outcome = services
            .launch_run(first.request)
            .await
            .expect("first launch");
        let second_outcome = services
            .launch_run(second.request)
            .await
            .expect("duplicate launch");

        assert_eq!(
            first_outcome.status(),
            crate::RunLaunchOutcomeStatus::Admitted
        );
        assert_eq!(
            second_outcome.status(),
            crate::RunLaunchOutcomeStatus::Attached
        );
        assert_eq!(
            second_outcome
                .run()
                .expect("attached launch returns run")
                .scheduler_status,
            "observed"
        );
        assert_eq!(fixture.run_admitted_count(&run_id).await, 1);
    }

    #[tokio::test]
    async fn report_only_portfolio_admits_without_live_evm_runtime_config() {
        // After collectors cutover, portfolio_snapshot never constructs live chain providers.
        // Missing runtime config must not block admission as LaunchRunnerUnavailable.
        let fixture = EntryPointRunFixture::in_memory().await;
        let prepared = fixture.prepare_sample_portfolio(None);
        let fact_index = crate::ProjectionFactIndexProvider::new(fixture.store.clone());
        let runners = crate::production_runner_registry(
            Arc::new(fixture.store.clone()),
            Arc::new(fact_index),
            None,
        )
        .expect("runners");
        let services = crate::make_run_services(
            runners,
            fixture.store.clone(),
            fixture.store.clone(),
            fixture.prep.certification_registry.clone(),
        );

        let outcome = services
            .launch_run(prepared.request)
            .await
            .expect("report-only portfolio admits without live EVM runtime config");

        assert_eq!(outcome.status(), crate::RunLaunchOutcomeStatus::Admitted);
    }

    #[tokio::test]
    async fn report_only_portfolio_admits_without_live_btc_runtime_config() {
        // Bitcoin portfolio report is facts-only; BTC JSON-RPC is required for collectors, not report.
        let fixture = EntryPointRunFixture::in_memory().await;
        let prepared = fixture.prepare_bitcoin_portfolio(None);
        let services = fixture.services();

        let outcome = services
            .launch_run(prepared.request)
            .await
            .expect("report-only portfolio admits without live BTC runtime config");

        assert_eq!(outcome.status(), crate::RunLaunchOutcomeStatus::Admitted);
    }

    #[tokio::test]
    async fn missing_platform_facts_follow_normal_report_failure_after_admission() {
        let fixture = EntryPointRunFixture::in_memory().await;
        let prepared = fixture.prepare_sample_portfolio(None);
        let runners = crate::production_runner_registry(
            Arc::new(fixture.store.clone()),
            crate::ProjectionFactIndexProvider::empty_arc(),
            None,
        )
        .expect("runners");
        let services = crate::make_run_services(
            runners,
            fixture.store.clone(),
            fixture.store.clone(),
            fixture.prep.certification_registry.clone(),
        );

        let run_id = prepared.request.run_id.clone();
        let outcome = services
            .launch_run(prepared.request)
            .await
            .expect("missing Platform facts must produce a failed admitted run");

        let run = outcome
            .run()
            .expect("report failure still returns the admitted run");
        assert_eq!(run.run_mode, crate::RunModeStatus::FailedWithoutAcdcClaim);
        assert!(run
            .attempt_dispositions
            .iter()
            .any(|attempt| attempt.error_code.as_deref() == Some("missing_fact")));
        assert!(
            fixture
                .store
                .load_run_stream(&run_id)
                .await
                .expect("run stream")
                .len()
                > 1
        );
    }

    #[tokio::test]
    async fn app_launch_concurrent_duplicates_admit_once_and_dedupe_rest() {
        const LAUNCHERS: usize = 8;

        let fixture = EntryPointRunFixture::in_memory().await;
        let prepared = fixture.prepare_sample_portfolio(None);
        let run_id = prepared.request.run_id.clone();
        let services = fixture.services();
        let barrier = Arc::new(tokio::sync::Barrier::new(LAUNCHERS));
        let mut handles = Vec::new();

        for _ in 0..LAUNCHERS {
            let services = services.clone();
            let request = prepared.request.clone();
            let barrier = barrier.clone();
            handles.push(tokio::spawn(async move {
                barrier.wait().await;
                services.launch_run(request).await.expect("launch").status()
            }));
        }

        let mut statuses = Vec::new();
        for handle in handles {
            statuses.push(handle.await.expect("launcher task"));
        }

        assert_eq!(
            statuses
                .iter()
                .filter(|status| **status == crate::RunLaunchOutcomeStatus::Admitted)
                .count(),
            1
        );
        assert_eq!(
            statuses
                .iter()
                .filter(|status| {
                    **status == crate::RunLaunchOutcomeStatus::Attached
                        || **status == crate::RunLaunchOutcomeStatus::AlreadyActive
                })
                .count(),
            LAUNCHERS - 1
        );
        assert_eq!(fixture.run_admitted_count(&run_id).await, 1);
    }

    #[tokio::test]
    async fn app_launch_duplicate_reports_already_active_for_live_claim() {
        let fixture = EntryPointRunFixture::in_memory().await;
        let invocation_key =
            crate::InvocationKey::new("mfm.test.app.duplicate-live-claim").expect("key");
        let first = fixture.prepare_sample_portfolio(Some(invocation_key.clone()));
        let second = fixture.prepare_sample_portfolio(Some(invocation_key));
        let run_id = first.request.run_id.clone();
        let execution_scope = store::ExecutionClaimScope::from_run_identity_material(
            &first.request.identity_material,
        );
        let services = fixture.services();
        services
            .launch_run(first.request)
            .await
            .expect("first launch");
        let token = AdmissionToken::new("mfm.test.app.execution_claim").expect("token");
        assert!(matches!(
            fixture
                .store
                .acquire_execution_claim(&execution_scope, &run_id, token)
                .await
                .expect("claim execution"),
            NowaitSkipAdmissionResult::Admitted(_)
        ));

        let outcome = services
            .launch_run(second.request)
            .await
            .expect("duplicate launch");

        assert_eq!(
            outcome.status(),
            crate::RunLaunchOutcomeStatus::AlreadyActive
        );
        assert_eq!(fixture.run_admitted_count(&run_id).await, 1);
    }

    #[tokio::test]
    async fn app_resume_reports_busy_for_live_execution_claim_without_driving() {
        let fixture = EntryPointRunFixture::in_memory().await;
        let prepared = fixture.prepare_sample_portfolio(None);
        let execution_scope = store::ExecutionClaimScope::from_run_identity_material(
            &prepared.request.identity_material,
        );
        let services = fixture.services();
        let run_id = admit_entry_point_run(&services, prepared).await;
        let head_seq = fixture
            .store
            .load_run_stream(&run_id)
            .await
            .expect("run stream")
            .last()
            .expect("admitted event")
            .seq()
            .as_u64();
        let token = AdmissionToken::new("mfm.test.app.execution_claim.resume_busy")
            .expect("execution claim token");
        assert!(matches!(
            fixture
                .store
                .acquire_execution_claim(&execution_scope, &run_id, token)
                .await
                .expect("claim execution"),
            NowaitSkipAdmissionResult::Admitted(_)
        ));

        let resumed = services
            .resume_stored_run(&run_id)
            .await
            .expect("resume response");

        assert_eq!(resumed.scheduler_status, "execution_claim_busy");
        assert_eq!(resumed.head_seq, head_seq);
        assert_eq!(fixture.run_admitted_count(&run_id).await, 1);
    }

    #[tokio::test]
    async fn app_resume_rejects_unavailable_runner_binding_without_claiming() {
        let fixture = EntryPointRunFixture::in_memory().await;
        let prepared = fixture.prepare_sample_portfolio(None);
        let execution_scope = store::ExecutionClaimScope::from_run_identity_material(
            &prepared.request.identity_material,
        );
        let services = fixture.services();
        let run_id = admit_entry_point_run(&services, prepared).await;
        let incompatible_services = crate::make_run_services(
            crate::ErasedRunnerRegistry::new(),
            fixture.store.clone(),
            fixture.store.clone(),
            fixture.prep.certification_registry.clone(),
        );

        let error = incompatible_services
            .resume_stored_run(&run_id)
            .await
            .expect_err("resume should reject unavailable runner bindings");

        assert_eq!(error.code, "LaunchRunnerUnavailable");
        assert!(matches!(
            fixture
                .store
                .execution_claim_status(&execution_scope)
                .await
                .expect("execution claim status"),
            ExecutionClaimStatus::Unclaimed
        ));
        assert_eq!(fixture.run_admitted_count(&run_id).await, 1);
    }

    #[tokio::test]
    async fn app_launch_fails_before_run_admitted_when_runner_binding_is_unavailable() {
        let fixture = EntryPointRunFixture::in_memory().await;
        let prepared = fixture.prepare_sample_portfolio(None);
        let run_id = prepared.request.run_id.clone();
        let services = crate::make_run_services(
            crate::ErasedRunnerRegistry::new(),
            fixture.store.clone(),
            fixture.store.clone(),
            fixture.prep.certification_registry.clone(),
        );

        let err = services
            .launch_run(prepared.request)
            .await
            .expect_err("missing runner binding rejects at ingress");

        assert_eq!(err.code, "LaunchRunnerUnavailable");
        assert_eq!(
            fixture
                .store
                .expected_next_seq(&run_id)
                .await
                .expect("expected next seq")
                .as_u64(),
            1
        );
        assert!(fixture
            .store
            .load_run_stream(&run_id)
            .await
            .expect("run stream")
            .is_empty());
    }

    #[test]
    fn evm_contract_entry_points_resolve_and_plan_without_seeds() {
        let registry = production_entry_point_op_registry().expect("registry");

        for (name, config, expects_import_node) in [
            ("evm_contract_deploy", deploy_config_json(), false),
            (
                "evm_contract_configure",
                configure_entry_config_json(),
                true,
            ),
            ("evm_contract_validate", validate_entry_config_json(), true),
            ("evm_contract_lifecycle", lifecycle_config_json(), false),
        ] {
            let op = registry
                .resolve_latest(&PublicOpName::new(name).expect("name"))
                .expect("EVM contract op");
            assert_eq!(op.descriptor().version, 1);
            let authored = AuthoredConfig::new(AuthoredConfigFormat::Json, config.to_string())
                .expect("authored");
            let plan = op.plan(authored).expect("EVM contract plan");

            assert!(!plan.config_material.is_empty());
            assert!(plan.seed_material.is_empty());
            assert!(plan.draft.seeds().is_empty());
            if expects_import_node {
                assert!(
                    plan.draft
                        .state_nodes()
                        .iter()
                        .any(|node| node.state_descriptor_name.contains("import_")),
                    "{name} should include an import state"
                );
            }
            assert!(
                !plan.draft.state_nodes().is_empty(),
                "{name} should plan state nodes"
            );
        }
    }

    struct EntryPointPrepFixture {
        entry_point_registry: EntryPointOpRegistry,
        certification_registry: mfm_certify::CertificationRegistry,
        store_scope_id: mfm_ids::StoreScopeId,
    }

    impl EntryPointPrepFixture {
        fn with_store_scope_hex(hex: &str) -> Self {
            Self::with_store_scope_id(
                mfm_ids::StoreScopeId::new(format!("mfm.store_scope.v1:{hex}"))
                    .expect("store scope"),
            )
        }

        fn with_store_scope_id(store_scope_id: mfm_ids::StoreScopeId) -> Self {
            Self {
                entry_point_registry: production_entry_point_op_registry().expect("registry"),
                certification_registry: crate::production_certification_registry().expect("cert"),
                store_scope_id,
            }
        }

        fn prepare_portfolio_launch(
            &self,
            config: String,
            invocation_key: Option<crate::InvocationKey>,
        ) -> crate::PreparedEntryPointRunLaunch {
            crate::prepare_entry_point_run_launch(crate::EntryPointRunLaunchInput {
                entry_point_registry: &self.entry_point_registry,
                public_op_name: portfolio_public_op_name(),
                op_version: None,
                authored_config: AuthoredConfig::new(AuthoredConfigFormat::Json, config)
                    .expect("authored config"),
                certification_registry: &self.certification_registry,
                store_scope_id: self.store_scope_id.clone(),
                invocation_key,
            })
            .expect("prepared entry-point launch")
        }

        fn prepare_sample_portfolio(
            &self,
            invocation_key: Option<crate::InvocationKey>,
        ) -> crate::PreparedEntryPointRunLaunch {
            self.prepare_portfolio_launch(sample_portfolio_config_json(), invocation_key)
        }

        fn prepare_bitcoin_portfolio(
            &self,
            invocation_key: Option<crate::InvocationKey>,
        ) -> crate::PreparedEntryPointRunLaunch {
            self.prepare_portfolio_launch(sample_bitcoin_portfolio_config_json(), invocation_key)
        }
    }

    struct EntryPointRunFixture {
        prep: EntryPointPrepFixture,
        store: mfm_store::v1::AsyncInMemoryRunStore,
        _runtime_config_dir: tempfile::TempDir,
        runtime_config_path: std::path::PathBuf,
    }

    impl EntryPointRunFixture {
        async fn in_memory() -> Self {
            let store = mfm_store::v1::AsyncInMemoryRunStore::default();
            let store_scope_id = store.load_store_scope_id().await.expect("store scope");
            let (runtime_config_dir, runtime_config_path) = test_runtime_config();
            Self {
                prep: EntryPointPrepFixture::with_store_scope_id(store_scope_id),
                store,
                _runtime_config_dir: runtime_config_dir,
                runtime_config_path,
            }
        }

        fn prepare_sample_portfolio(
            &self,
            invocation_key: Option<crate::InvocationKey>,
        ) -> crate::PreparedEntryPointRunLaunch {
            self.prep.prepare_sample_portfolio(invocation_key)
        }

        fn prepare_bitcoin_portfolio(
            &self,
            invocation_key: Option<crate::InvocationKey>,
        ) -> crate::PreparedEntryPointRunLaunch {
            self.prep.prepare_bitcoin_portfolio(invocation_key)
        }

        fn services(
            &self,
        ) -> crate::RunServices<
            mfm_store::v1::AsyncInMemoryRunStore,
            mfm_store::v1::AsyncInMemoryRunStore,
        > {
            let fact_index = crate::ProjectionFactIndexProvider::new(self.store.clone());
            let runners = crate::production_runner_registry(
                Arc::new(self.store.clone()),
                Arc::new(fact_index),
                Some(&self.runtime_config_path),
            )
            .expect("runners");
            crate::make_run_services(
                runners,
                self.store.clone(),
                self.store.clone(),
                self.prep.certification_registry.clone(),
            )
        }

        async fn run_admitted_count(&self, run_id: &mfm_ids::RunId) -> usize {
            self.store
                .load_run_stream(run_id)
                .await
                .expect("run stream")
                .into_iter()
                .filter(|event| {
                    matches!(
                        event.payload(),
                        mfm_events::v1::KernelEventPayload::RunAdmitted(_)
                    )
                })
                .count()
        }
    }

    async fn admit_entry_point_run(
        services: &crate::RunServices<
            mfm_store::v1::AsyncInMemoryRunStore,
            mfm_store::v1::AsyncInMemoryRunStore,
        >,
        prepared: crate::PreparedEntryPointRunLaunch,
    ) -> mfm_ids::RunId {
        let request = prepared.request;
        let run_id = request.run_id.clone();
        let runtime_spec = mfm_runtime::CertifiedRuntimeSpec::new(request.certified_spec.clone())
            .expect("runtime spec");
        let expected_next_seq = services
            .store()
            .expected_next_seq(&run_id)
            .await
            .expect("expected next seq");
        let launch = services
            .scheduler
            .prepare_run_launch(
                &runtime_spec,
                request.identity_material,
                request.evidence,
                expected_next_seq,
            )
            .expect("prepare admitted-only launch");
        services
            .scheduler
            .start_run(services.store(), launch)
            .await
            .expect("admit run");
        run_id
    }

    fn test_runtime_config() -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().expect("runtime config tempdir");
        let path = dir.path().join("runtime.toml");
        std::fs::write(
            &path,
            r#"
[evm.sources."ethereum-mainnet"]
rpc_url = "http://127.0.0.1:1"

[evm.routes."ethereum-mainnet"]
source_ref = "ethereum-mainnet"
"#,
        )
        .expect("write runtime config");
        (dir, path)
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
                                    "unit_price_dec": "1800.00"
                                }
                            ]
                        },
                        "underlying_symbol_id": null,
                        "metadata": {}
                    }
                ],
                "metadata": {}
            }
        })
        .to_string()
    }

    fn sample_bitcoin_portfolio_config_json() -> String {
        serde_json::json!({
            "portfolio": {
                "portfolio_id": "portfolio_btc",
                "quote_codes": ["USD"],
                "networks": [
                    {
                        "network_id": "bitcoin-mainnet",
                        "family": "bitcoin",
                        "bitcoin_network": "main",
                        "source_identity": "public-bitcoin-core",
                        "metadata": {}
                    }
                ],
                "wallets": [
                    {
                        "wallet_id": "wallet_btc_mainnet",
                        "subject": {
                            "kind": "bitcoin_address",
                            "address": "bc1qns9f7yfx3ry9lj6yz7c9er0vwa0ye2eklpzqfw"
                        },
                        "implementation": { "kind": "address_only" },
                        "network_id": "bitcoin-mainnet",
                        "symbol_ids": ["btc.native.bitcoin-mainnet"],
                        "metadata": {}
                    }
                ],
                "symbol_configs": [
                    {
                        "symbol_id": "btc.native.bitcoin-mainnet",
                        "display_symbol": "BTC",
                        "kind": "native_balance",
                        "role": "native",
                        "network_id": "bitcoin-mainnet",
                        "protocol": null,
                        "balance_reader": { "kind": "native_balance" },
                        "valuation": {
                            "quotes": [
                                {
                                    "quote": "USD",
                                    "priced_symbol_id": "btc.native.bitcoin-mainnet",
                                    "unit_price_dec": "0.00"
                                }
                            ]
                        },
                        "underlying_symbol_id": null,
                        "metadata": {}
                    }
                ],
                "metadata": {}
            }
        })
        .to_string()
    }

    fn content_digest_str(byte: u8) -> String {
        format!("content:sha256-jcs-v1:{}", format!("{byte:02x}").repeat(32))
    }

    fn context_json() -> serde_json::Value {
        serde_json::json!({
            "lifecycle_key": "app-entry-test-lifecycle",
            "network": {
                "network_id": "ethereum-mainnet",
                "expected_chain_id": 1,
                "chain_fingerprint": null,
                "finality_or_observation_policy": null,
            },
            "contract_profile": {
                "profile_id": "app-entry-test-contract",
                "artifact_digest": content_digest_str(0x20),
                "interface_digest": content_digest_str(0x21),
                "creation_bytecode_digest": null,
                "deployed_code_hash": null,
                "selector_event_policy_digest": null,
            },
        })
    }

    fn signer_json() -> serde_json::Value {
        serde_json::json!({
            "signer_ref": "deployer",
            "expected_signer_address": "0x000000000000000000000000000000000000dead",
        })
    }

    fn deploy_action_json() -> serde_json::Value {
        serde_json::json!({
            "signer": signer_json(),
        })
    }

    fn configure_action_json() -> serde_json::Value {
        serde_json::json!({
            "signer": signer_json(),
            "calls": [],
        })
    }

    fn validate_action_json() -> serde_json::Value {
        serde_json::json!({})
    }

    fn deploy_config_json() -> serde_json::Value {
        serde_json::json!({
            "context": context_json(),
            "deploy": deploy_action_json(),
        })
    }

    fn lifecycle_config_json() -> serde_json::Value {
        serde_json::json!({
            "context": context_json(),
            "deploy": deploy_action_json(),
            "configure": configure_action_json(),
            "validate": validate_action_json(),
        })
    }

    fn import_deployed_json() -> serde_json::Value {
        serde_json::json!({
            "kind": "adopt_external_address",
            "adoption": {
                "address": "0x000000000000000000000000000000000000dead",
                "provenance_label": "app-entry-test-external",
                "evidence_policy": {
                    "require_code": false
                }
            },
        })
    }

    fn import_configured_json() -> serde_json::Value {
        serde_json::json!({
            "kind": "adopt_external_address",
            "adoption": {
                "address": "0x000000000000000000000000000000000000dead",
                "provenance_label": "app-entry-test-external",
                "evidence_policy": {
                    "require_code": false,
                    "allow_external_claimed_configured": true
                }
            },
        })
    }

    fn configure_entry_config_json() -> serde_json::Value {
        serde_json::json!({
            "context": context_json(),
            "import_deployed": import_deployed_json(),
            "configure": configure_action_json(),
        })
    }

    fn validate_entry_config_json() -> serde_json::Value {
        serde_json::json!({
            "context": context_json(),
            "import_configured": import_configured_json(),
            "validate": validate_action_json(),
        })
    }
}
