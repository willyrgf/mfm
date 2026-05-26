#![allow(clippy::disallowed_methods, dead_code)]

use mfm_app::{DriveMode, TypedPublicOutputResponse, TypedRunResponse, TypedRunResumeRequest};
use mfm_artifact_store_fs::{FsTypedArtifactStore, TypedArtifactDescriptor};
use mfm_events::v1 as events;
use mfm_op_portfolio_tracker::{
    certified_portfolio_spec, portfolio_config_artifacts_for_spec, portfolio_program_draft,
    PortfolioConfigArtifact, PortfolioWorkflowConfig,
};
use mfm_portfolio_config::decode_portfolio_snapshot_canonical_config;

pub struct TypedPortfolioSnapshotResult {
    pub run: TypedRunResponse,
    pub public_output: TypedPublicOutputResponse,
}

pub struct TypedPortfolioResumeResult {
    pub started: TypedRunResponse,
    pub resumed: TypedRunResponse,
    pub public_output: TypedPublicOutputResponse,
}

pub async fn run_typed_portfolio_snapshot(
    payload: serde_json::Value,
) -> TypedPortfolioSnapshotResult {
    let canonical =
        decode_portfolio_snapshot_canonical_config(&payload).expect("canonical portfolio config");
    let workflow_config = PortfolioWorkflowConfig::from(canonical);
    let draft = portfolio_program_draft(workflow_config.clone()).expect("portfolio draft");
    let certified = certified_portfolio_spec(workflow_config).expect("portfolio certified spec");
    let public_schema_id = certified
        .envelope()
        .spec
        .public_outputs
        .public_schema_id
        .clone();

    let tmp = tempfile::tempdir().expect("typed artifact tempdir");
    let artifacts = FsTypedArtifactStore::new(tmp.path());
    let runners =
        mfm_app::production_typed_runner_registry(artifacts.clone()).expect("typed runners");
    let services = mfm_app::make_in_memory_typed_services(runners, tmp.path());

    persist_config_artifacts(
        services.artifacts(),
        portfolio_config_artifacts_for_spec(&draft, &certified.envelope().spec)
            .expect("config artifacts"),
    )
    .await;

    let spec_bytes = certified
        .envelope()
        .spec
        .canonical_json()
        .expect("canonical spec")
        .to_vec();
    let run_id = mfm_app::new_run_id();
    let request = mfm_app::build_typed_run_start_request(
        services.artifacts(),
        &spec_bytes,
        run_id.clone(),
        "mfm.integration.portfolio.typed.v1",
        "integration-test",
        Vec::new(),
        DriveMode::UntilBlocked,
    )
    .await
    .expect("typed portfolio start request");
    let run = services
        .start_certified_run(request)
        .await
        .expect("typed portfolio run");
    let public_output = services
        .typed_public_output(&run_id, &public_schema_id)
        .await
        .expect("typed portfolio public output");
    TypedPortfolioSnapshotResult { run, public_output }
}

pub async fn resume_typed_portfolio_snapshot(
    payload: serde_json::Value,
) -> TypedPortfolioResumeResult {
    let canonical =
        decode_portfolio_snapshot_canonical_config(&payload).expect("canonical portfolio config");
    let workflow_config = PortfolioWorkflowConfig::from(canonical);
    let draft = portfolio_program_draft(workflow_config.clone()).expect("portfolio draft");
    let certified = certified_portfolio_spec(workflow_config).expect("portfolio certified spec");
    let public_schema_id = certified
        .envelope()
        .spec
        .public_outputs
        .public_schema_id
        .clone();

    let tmp = tempfile::tempdir().expect("typed artifact tempdir");
    let artifacts = FsTypedArtifactStore::new(tmp.path());
    let runners =
        mfm_app::production_typed_runner_registry(artifacts.clone()).expect("typed runners");
    let services = mfm_app::make_in_memory_typed_services(runners, tmp.path());

    persist_config_artifacts(
        services.artifacts(),
        portfolio_config_artifacts_for_spec(&draft, &certified.envelope().spec)
            .expect("config artifacts"),
    )
    .await;

    let spec_bytes = certified
        .envelope()
        .spec
        .canonical_json()
        .expect("canonical spec")
        .to_vec();
    let run_id = mfm_app::new_run_id();
    let request = mfm_app::build_typed_run_start_request(
        services.artifacts(),
        &spec_bytes,
        run_id.clone(),
        "mfm.integration.portfolio.typed.v1",
        "integration-test",
        Vec::new(),
        DriveMode::AppendOnly,
    )
    .await
    .expect("typed portfolio append-only start request");
    let started = services
        .start_certified_run(request)
        .await
        .expect("typed portfolio append-only run");
    let resumed = services
        .resume_certified_run(TypedRunResumeRequest {
            certified_spec: certified,
            run_id: run_id.clone(),
            drive: DriveMode::UntilBlocked,
        })
        .await
        .expect("typed portfolio resume");
    let public_output = services
        .typed_public_output(&run_id, &public_schema_id)
        .await
        .expect("typed portfolio public output");
    TypedPortfolioResumeResult {
        started,
        resumed,
        public_output,
    }
}

async fn persist_config_artifacts(
    artifacts: &FsTypedArtifactStore,
    configs: Vec<PortfolioConfigArtifact>,
) {
    for config in configs {
        artifacts
            .put_artifact(
                config.bytes,
                TypedArtifactDescriptor {
                    media_type: config.media_type,
                    schema_id: Some(config.schema_id),
                    semantic_type_id: None,
                    producer_node_id: None,
                    producer_seed_id: None,
                    artifact_role: events::ArtifactRole::TypedConfig,
                },
            )
            .await
            .expect("persist typed config artifact");
    }
}
