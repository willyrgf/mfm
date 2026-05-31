#![allow(clippy::disallowed_methods, dead_code)]

use mfm_app::{DriveMode, TypedConfigInput, TypedPublicOutputResponse, TypedRunResponse};
use mfm_artifact_store_fs::FsTypedArtifactStore;
use mfm_events::v1 as events;
use mfm_op_portfolio_tracker::{
    certified_portfolio_spec, portfolio_config_artifacts_for_spec, portfolio_program_draft,
    PortfolioConfigArtifact, PortfolioWorkflowConfig,
};
use mfm_portfolio_config::decode_portfolio_snapshot_canonical_config;
use mfm_store::v1 as store;
use mfm_store::v1::TypedRunEventStore;

pub struct TypedPortfolioAuthorityEvidence {
    pub spec_hash: String,
    pub certificate_hash: String,
    pub retained_artifacts: usize,
    pub public_output_event_id: String,
    pub public_output_rendered_digest: String,
}

pub struct TypedPortfolioSnapshotResult {
    pub run: TypedRunResponse,
    pub public_output: TypedPublicOutputResponse,
    pub authority: TypedPortfolioAuthorityEvidence,
}

pub struct TypedPortfolioResumeResult {
    pub started: TypedRunResponse,
    pub resumed: TypedRunResponse,
    pub public_output: TypedPublicOutputResponse,
    pub authority: TypedPortfolioAuthorityEvidence,
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
    let registry = mfm_app::production_certification_registry().expect("certification registry");
    let services = mfm_app::make_in_memory_typed_services_with_certification_registry(
        runners,
        tmp.path(),
        registry.clone(),
    );

    let config_inputs = typed_config_inputs(
        portfolio_config_artifacts_for_spec(&draft, &certified.envelope().spec)
            .expect("config artifacts"),
    );

    let bundle = certified.bundle().expect("certified bundle");
    let run_id = mfm_app::new_run_id();
    let request = mfm_app::verify_certified_bundle_run_start_request(
        mfm_app::UntrustedCertifiedSpecBundleStartInput {
            spec_bytes: bundle.spec_bytes(),
            certificate_bytes: bundle.certificate_bytes(),
            registry: &registry,
            run_id: run_id.clone(),
            framework_version: "mfm.integration.portfolio.typed.v1",
            source_revision: "integration-test",
            drive: DriveMode::UntilBlocked,
        },
        config_inputs,
        Vec::new(),
    )
    .expect("typed portfolio start request");
    let run = services
        .start_certified_run(request)
        .await
        .expect("typed portfolio run");
    let public_output = services
        .typed_public_output(&run_id, &public_schema_id)
        .await
        .expect("typed portfolio public output");
    let stream = {
        let store = services.store();
        let store = store.lock().await;
        store.load_run_stream(&run_id)
    };
    let replay_broker = services
        .replay_broker(&run_id)
        .await
        .expect("typed portfolio replay authority");
    let retained_artifacts = replay_broker
        .projection_snapshot()
        .retention(&run_id)
        .map(|retention| retention.refs.len())
        .unwrap_or_default();
    let authority =
        authority_evidence_from_stream(&stream, &run, &public_output, retained_artifacts);
    TypedPortfolioSnapshotResult {
        run,
        public_output,
        authority,
    }
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
    let registry = mfm_app::production_certification_registry().expect("certification registry");
    let services = mfm_app::make_in_memory_typed_services_with_certification_registry(
        runners,
        tmp.path(),
        registry.clone(),
    );

    let config_inputs = typed_config_inputs(
        portfolio_config_artifacts_for_spec(&draft, &certified.envelope().spec)
            .expect("config artifacts"),
    );

    let bundle = certified.bundle().expect("certified bundle");
    let run_id = mfm_app::new_run_id();
    let request = mfm_app::verify_certified_bundle_run_start_request(
        mfm_app::UntrustedCertifiedSpecBundleStartInput {
            spec_bytes: bundle.spec_bytes(),
            certificate_bytes: bundle.certificate_bytes(),
            registry: &registry,
            run_id: run_id.clone(),
            framework_version: "mfm.integration.portfolio.typed.v1",
            source_revision: "integration-test",
            drive: DriveMode::AppendOnly,
        },
        config_inputs,
        Vec::new(),
    )
    .expect("typed portfolio append-only start request");
    let started = services
        .start_certified_run(request)
        .await
        .expect("typed portfolio append-only run");
    let resumed = services
        .resume_stored_run(&run_id, DriveMode::UntilBlocked)
        .await
        .expect("typed portfolio resume");
    let public_output = services
        .typed_public_output(&run_id, &public_schema_id)
        .await
        .expect("typed portfolio public output");
    let stream = {
        let store = services.store();
        let store = store.lock().await;
        store.load_run_stream(&run_id)
    };
    let replay_broker = services
        .replay_broker(&run_id)
        .await
        .expect("typed portfolio replay authority");
    let retained_artifacts = replay_broker
        .projection_snapshot()
        .retention(&run_id)
        .map(|retention| retention.refs.len())
        .unwrap_or_default();
    let authority =
        authority_evidence_from_stream(&stream, &resumed, &public_output, retained_artifacts);
    TypedPortfolioResumeResult {
        started,
        resumed,
        public_output,
        authority,
    }
}

fn authority_evidence_from_stream(
    stream: &[store::KernelEventEnvelope],
    run: &TypedRunResponse,
    public_output: &TypedPublicOutputResponse,
    retained_artifacts: usize,
) -> TypedPortfolioAuthorityEvidence {
    let run_started = stream
        .iter()
        .find_map(|event| match event.payload() {
            events::KernelEventPayload::RunStarted(payload) => Some(payload),
            _ => None,
        })
        .expect("RunStarted evidence");
    assert_eq!(run_started.spec_hash.as_str(), run.spec_hash);

    let (public_output_event, public_output_payload) = stream
        .iter()
        .find_map(|event| match event.payload() {
            events::KernelEventPayload::PublicOutputProduced(payload) => Some((event, payload)),
            _ => None,
        })
        .expect("PublicOutputProduced evidence");
    assert_eq!(
        public_output_event.event_id().as_str(),
        public_output.event_id
    );
    assert_eq!(
        public_output_payload.rendered_digest.as_str(),
        public_output.rendered_digest
    );

    TypedPortfolioAuthorityEvidence {
        spec_hash: run_started.spec_hash.as_str().to_owned(),
        certificate_hash: run_started.certificate_artifact_digest.as_str().to_owned(),
        retained_artifacts,
        public_output_event_id: public_output_event.event_id().as_str().to_owned(),
        public_output_rendered_digest: public_output_payload.rendered_digest.as_str().to_owned(),
    }
}

fn typed_config_inputs(configs: Vec<PortfolioConfigArtifact>) -> Vec<TypedConfigInput> {
    configs
        .into_iter()
        .map(|config| TypedConfigInput {
            schema_id: config.schema_id,
            bytes: config.bytes,
            media_type: config.media_type,
        })
        .collect()
}
