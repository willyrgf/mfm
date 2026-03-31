#![allow(clippy::disallowed_methods)]

use super::*;

use std::sync::Arc;

use mfm_aave_v3_origin_config::{
    build_aave_v3_origin_stack_outcome, AaveV3OriginSourceConfig, AaveV3OriginStackBuildReport,
    AaveV3OriginStackCanonicalConfig,
};
use mfm_machine::engine::{ExecutionEngine, RunPhase, Stores};
use mfm_machine::ids::ArtifactId;
use mfm_sdk::op::{CompositeOpSpec, PlannedOp, PlannedOpKind};
use mfm_sdk::unstable::{context_value_with_slot_fallback, single_op_pipeline, SdkPlanResolver};
use mfm_state_common::test_support as op_test_support;

fn into_composite(planned: PlannedOp) -> CompositeOpSpec {
    match planned.kind {
        PlannedOpKind::Composite(spec) => spec,
        PlannedOpKind::Leaf(_) => panic!("expected composite planned op"),
    }
}

fn canonical_config() -> AaveV3OriginStackCanonicalConfig {
    AaveV3OriginStackCanonicalConfig {
        source: AaveV3OriginSourceConfig {
            repo_url: mfm_aave_v3_origin_config::AAVE_V3_ORIGIN_BACKEND_REPO_URL.to_string(),
            commit_sha: mfm_aave_v3_origin_config::AAVE_V3_ORIGIN_BACKEND_COMMIT_SHA.to_string(),
        },
        network_id: "reth-local".to_string(),
        control_scope: "shared".to_string(),
        deploy_signing_key_env: "MFM_AAVE_V3_PARITY_DEPLOY_SIGNING_KEY".to_string(),
        rpc_url_env: "MFM_EVM_RPC_URL".to_string(),
        supplier: "0x70997970c51812dc3a010c7d01b50e0d17dc79c8".to_string(),
        borrower: "0x3c44cdddb6a900fa2b585dd299e03d12fa4293bc".to_string(),
        usdc_supply_amount: 1_000_000_000_000,
        wbtc_collateral_amount: 1_000_000_000,
        fetch_timeout_ms: 300_000,
        compile_timeout_ms: 600_000,
        deploy_timeout_ms: 600_000,
    }
}

fn canonical_op_config() -> serde_json::Value {
    serde_json::to_value(canonical_config()).expect("canonical op config")
}

fn built_op_config() -> serde_json::Value {
    serde_json::to_value(
        build_aave_v3_origin_stack_outcome(canonical_config())
            .expect("build outcome")
            .built,
    )
    .expect("built op config")
}

async fn load_context_snapshot(stores: &Stores, snapshot_id: &ArtifactId) -> serde_json::Value {
    let bytes = stores
        .artifacts
        .get(snapshot_id)
        .await
        .expect("snapshot bytes");
    serde_json::from_slice(&bytes).expect("snapshot json")
}

fn read_required_context_value(
    snapshot: &serde_json::Value,
    key: &ContextKey,
) -> serde_json::Value {
    context_value_with_slot_fallback(snapshot, key).unwrap_or_else(|| {
        let keys = snapshot
            .as_object()
            .map(|obj| obj.keys().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        panic!("missing context key `{}`; keys={keys:?}", key.0)
    })
}

#[test]
fn aave_origin_tracker_canonical_input_composes_build_then_execute() {
    let op = AaveV3OriginStackOp;
    let planned = op
        .expand(
            OpPath("aave_v3_origin_stack.main".to_string()),
            &canonical_op_config(),
            &op_test_support::run_config_live(),
        )
        .expect("expand");
    let composite = into_composite(planned);

    assert!(composite
        .children
        .iter()
        .any(|child| child.child_op_local_id.0 == STACK_BUILD_CHILD_ID));
    assert!(composite
        .children
        .iter()
        .any(|child| child.child_op_local_id.0 == STACK_EXECUTE_CHILD_ID));
    assert!(composite
        .re_exports
        .iter()
        .any(|binding| binding.export.0 == PORT_DEPLOY_MANIFEST));
    assert!(composite
        .re_exports
        .iter()
        .any(|binding| binding.export.0 == PORT_REPORT));
}

#[test]
fn aave_origin_execute_rejects_canonical_config() {
    let op = AaveV3OriginStackExecuteOp;
    let err = op
        .expand(
            OpPath("aave_v3_origin_stack_execute.main".to_string()),
            &canonical_op_config(),
            &op_test_support::run_config_live(),
        )
        .err()
        .expect("canonical input must fail");

    assert_eq!(err.info.code.0, "invalid_aave_v3_origin_stack_config");
}

#[test]
fn aave_origin_execute_built_input_reexports_phase_a_outputs() {
    let op = AaveV3OriginStackExecuteOp;
    let planned = op
        .expand(
            OpPath("aave_v3_origin_stack_execute.main".to_string()),
            &built_op_config(),
            &op_test_support::run_config_live(),
        )
        .expect("expand");
    let composite = into_composite(planned);

    assert!(composite
        .children
        .iter()
        .any(|child| child.child_op_local_id.0 == FETCH_ORIGIN_CHILD_ID));
    assert!(composite
        .children
        .iter()
        .any(|child| child.child_op_local_id.0 == PROJECT_REPORT_CHILD_ID));
    assert!(composite
        .re_exports
        .iter()
        .any(|binding| binding.export.0 == PORT_FETCH_ORIGIN_ARTIFACT_ID));
    assert!(composite
        .re_exports
        .iter()
        .any(|binding| binding.export.0 == PORT_COMPILE_ORIGIN_ARTIFACT_ID));
    assert!(composite
        .re_exports
        .iter()
        .any(|binding| binding.export.0 == PORT_DEPLOY_ORIGIN_ARTIFACT_ID));
}

#[tokio::test]
async fn aave_origin_config_build_emits_built_config_artifacts_and_report() {
    let registry = op_test_support::registry_with_ops(aave_v3_origin_ops());
    let planner = op_test_support::default_pipeline_planner();
    let pipeline = single_op_pipeline(
        OpId::must_new(AAVE_V3_ORIGIN_STACK_CONFIG_BUILD_OP_ID.to_string()),
        AAVE_V3_ORIGIN_STACK_PUBLIC_OP_VERSION.to_string(),
        canonical_op_config(),
    )
    .expect("pipeline");
    let stores = op_test_support::in_memory_stores();
    let resolver = Arc::new(SdkPlanResolver::new(
        Arc::clone(&registry),
        Arc::clone(&planner),
    ));
    let engine: Arc<dyn ExecutionEngine> =
        Arc::new(mfm_machine::runtime::DefaultExecutionEngine::new(resolver));

    let run = op_test_support::start_pipeline_with_defaults(
        engine,
        &stores,
        registry,
        planner,
        pipeline,
        op_test_support::run_config_live(),
    )
    .await
    .expect("start");

    assert_eq!(run.phase, RunPhase::Completed);
    let final_snapshot_id = run.final_snapshot_id.expect("final snapshot");
    let context_snapshot = load_context_snapshot(&stores, &final_snapshot_id).await;

    let built_config: AaveV3OriginStackBuiltConfig =
        serde_json::from_value(read_required_context_value(
            &context_snapshot,
            &aave_v3_origin_stack_config_build_built_config_context_key(),
        ))
        .expect("built config");
    let report: AaveV3OriginStackBuildReport = serde_json::from_value(read_required_context_value(
        &context_snapshot,
        &aave_v3_origin_stack_config_build_report_context_key(),
    ))
    .expect("build report");
    let canonical_artifact_id: String = serde_json::from_value(read_required_context_value(
        &context_snapshot,
        &aave_v3_origin_stack_config_build_canonical_artifact_id_context_key(),
    ))
    .expect("canonical artifact id");
    let built_artifact_id: String = serde_json::from_value(read_required_context_value(
        &context_snapshot,
        &aave_v3_origin_stack_config_build_built_artifact_id_context_key(),
    ))
    .expect("built artifact id");

    assert_eq!(report.canonical_config_artifact_id, canonical_artifact_id);
    assert_eq!(report.built_config_artifact_id, built_artifact_id);
    assert_eq!(report.network_id, built_config.canonical.network_id);
    assert_eq!(report.step_count, 4);
}
