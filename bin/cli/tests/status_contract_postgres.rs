#![cfg(feature = "parity-tests")]
#![allow(clippy::disallowed_methods)]

use assert_cmd::Command;
use mfm_events::v1 as events;
use mfm_ids::{AttemptId, DigestAlgorithm, DigestBytes, RunId, SpecHash};
use mfm_spec::v1 as spec;
use mfm_store::v1 as store;
use mfm_stream_store_postgres::{PostgresSchema, PostgresTypedRunEventStore};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Output;
use tempfile::TempDir;

#[tokio::test]
async fn run_status_reports_interrupted_attempt_and_framework_attempts_from_history() {
    let database_url =
        std::env::var("DATABASE_URL").expect("DATABASE_URL must be set for parity tests");
    PostgresSchema::migrate(&database_url)
        .await
        .expect("migrate typed postgres schema");
    let store = PostgresTypedRunEventStore::connect(&database_url)
        .await
        .expect("connect typed postgres store");
    let temp = TempDir::new().expect("temp dir");
    let artifact_root = temp.path().join("typed-artifacts");
    std::fs::create_dir_all(&artifact_root).expect("artifact root");
    let proof_config = mfm_op_proof::ProofWorkflowConfig::default();
    let draft = mfm_op_proof::proof_program_draft(proof_config.clone()).expect("proof draft");
    let certified = mfm_op_proof::certified_proof_spec(proof_config).expect("proof spec");
    let (bundle_path, config_args) = proof_bundle_and_config_args(temp.path(), &draft, &certified);
    let run_id = mfm_app::new_run_id();

    let mut start_args = vec![
        "--output-format".to_owned(),
        "json".to_owned(),
        "run".to_owned(),
        "start".to_owned(),
        "--bundle".to_owned(),
        bundle_path.display().to_string(),
        "--run-id".to_owned(),
        run_id.as_str().to_owned(),
        "--drive".to_owned(),
        "append-only".to_owned(),
        "--typed-artifact-root".to_owned(),
        artifact_root.display().to_string(),
        "--database-url".to_owned(),
        database_url.clone(),
    ];
    start_args.extend(config_args);
    let start = run_cli(&start_args);
    assert_success(&start);
    let start_json = parse_success_json(&start.stdout);
    assert_eq!(start_json["run_mode"], "forward");

    let interrupted_node = certified
        .envelope()
        .spec
        .nodes
        .iter()
        .find(|node| node.framework.is_none())
        .expect("domain node");
    let interrupted_attempt_id = fixed_attempt_id(0x51);
    append_interrupted_attempt(
        &store,
        &run_id,
        certified.spec_hash(),
        interrupted_node,
        &interrupted_attempt_id,
    )
    .await;

    let resume = run_cli(&[
        "--output-format".to_owned(),
        "json".to_owned(),
        "run".to_owned(),
        "resume".to_owned(),
        run_id.as_str().to_owned(),
        "--drive".to_owned(),
        "until-blocked".to_owned(),
        "--typed-artifact-root".to_owned(),
        artifact_root.display().to_string(),
        "--database-url".to_owned(),
        database_url.clone(),
    ]);
    assert_success(&resume);
    let resume_json = parse_success_json(&resume.stdout);
    assert_eq!(resume_json["run_mode"], "completed");

    let status = run_cli(&[
        "--output-format".to_owned(),
        "json".to_owned(),
        "run".to_owned(),
        "status".to_owned(),
        run_id.as_str().to_owned(),
        "--typed-artifact-root".to_owned(),
        artifact_root.display().to_string(),
        "--database-url".to_owned(),
        database_url.clone(),
    ]);
    assert_success(&status);
    let status_json = parse_success_json(&status.stdout);
    assert_ne!(status_json["run_mode"], "interrupted");
    let attempts = status_json["attempt_dispositions"]
        .as_array()
        .expect("attempt dispositions");
    let interrupted = attempts
        .iter()
        .find(|attempt| {
            attempt["attempt_id"] == interrupted_attempt_id.as_str()
                && attempt["disposition"] == "interrupted"
        })
        .expect("interrupted attempt disposition");
    assert!(interrupted["retryable"].is_null());

    let completed_framework_kinds = certified
        .envelope()
        .spec
        .nodes
        .iter()
        .filter(|node| {
            attempts.iter().any(|attempt| {
                attempt["node_id"] == node.node_id.as_str() && attempt["disposition"] == "completed"
            })
        })
        .filter_map(|node| match &node.framework {
            Some(spec::FrameworkNodeSpec::PublicOutputRender(_)) => Some("public_output_render"),
            Some(spec::FrameworkNodeSpec::ProjectRetentionManifest(_)) => {
                Some("project_retention_manifest")
            }
            Some(spec::FrameworkNodeSpec::CompleteRun(_)) => Some("complete_run"),
            Some(spec::FrameworkNodeSpec::ResolveSagaTerminal(_)) => Some("resolve_saga_terminal"),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(
        completed_framework_kinds.contains(&"public_output_render"),
        "missing completed public-output render framework attempt"
    );
    assert!(
        completed_framework_kinds.contains(&"project_retention_manifest"),
        "missing completed retention framework attempt"
    );
    assert!(
        completed_framework_kinds.contains(&"complete_run"),
        "missing completed complete-run framework attempt"
    );

    let stream = run_cli(&[
        "--output-format".to_owned(),
        "json".to_owned(),
        "run".to_owned(),
        "stream".to_owned(),
        run_id.as_str().to_owned(),
        "--typed-artifact-root".to_owned(),
        artifact_root.display().to_string(),
        "--database-url".to_owned(),
        database_url,
    ]);
    assert_success(&stream);
    let stream_json = parse_success_json(&stream.stdout);
    let stream_events = stream_json["events"].as_array().expect("stream events");
    assert_framework_started_before_terminal_evidence(
        stream_events,
        attempts,
        &certified.envelope().spec.nodes,
        &run_id,
    );
}

fn proof_bundle_and_config_args(
    root: &Path,
    draft: &mfm_program::TypedProgramDraft,
    certified: &mfm_certify::CertifiedTypedSpec,
) -> (PathBuf, Vec<String>) {
    let bundle = certified.bundle().expect("proof bundle");
    let bundle_json = serde_json::json!({
        "kind": "certified_typed_spec_bundle_v1",
        "spec": serde_json::from_slice::<Value>(bundle.spec_bytes()).expect("spec JSON"),
        "certificate": serde_json::from_slice::<Value>(bundle.certificate_bytes())
            .expect("certificate JSON"),
    });
    let bundle_path = root.join("bundle.json");
    std::fs::write(
        &bundle_path,
        serde_json::to_vec(&bundle_json).expect("bundle JSON"),
    )
    .expect("write bundle");

    let mut args = Vec::new();
    let mut index = 0usize;
    for config in draft
        .state_nodes()
        .iter()
        .map(|node| &node.config)
        .chain(draft.operation_lineage().iter().map(|frame| &frame.config))
    {
        let path = root.join(format!("config-{index}.json"));
        index += 1;
        std::fs::write(&path, config.canonical_json.as_bytes()).expect("write config");
        args.push("--config".to_owned());
        args.push(format!("{}={}", config.schema_id, path.display()));
    }
    for node in &certified.envelope().spec.nodes {
        let Some(framework) = &node.framework else {
            continue;
        };
        let bytes = spec::framework_config_canonical_json(framework.config_kind(), &node.node_id)
            .expect("framework config");
        let path = root.join(format!("config-{index}.json"));
        index += 1;
        std::fs::write(&path, bytes.as_bytes()).expect("write framework config");
        args.push("--config".to_owned());
        args.push(format!("{}={}", node.config_ref.schema_id, path.display()));
    }

    (bundle_path, args)
}

async fn append_interrupted_attempt(
    store: &PostgresTypedRunEventStore,
    run_id: &RunId,
    spec_hash: &SpecHash,
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
) {
    let start = store::TypedCommitRequest::from_payloads(
        run_id.clone(),
        store
            .expected_next_seq(run_id)
            .await
            .expect("expected next seq"),
        store::CommitKey::new("cli-interrupted-attempt-start").expect("commit key"),
        vec![events::KernelEventPayload::StateAttemptStarted(
            events::StateAttemptStarted {
                spec_hash: spec_hash.clone(),
                node_id: node.node_id.clone(),
                attempt_id: attempt_id.clone(),
                attempt_no: 1,
                state_kind: node.state_kind.clone(),
                state_version: node.state_version.clone(),
            },
        )],
        Vec::new(),
        store::CommitPreconditions {
            required_run_state: store::RequiredRunState::NotCompleted,
            required_cell_states: vec![store::CellStatePrecondition {
                cell_id: node.output_cell.clone(),
                required: store::RequiredCellState::Absent,
            }],
            ..store::CommitPreconditions::default()
        },
    )
    .expect("attempt start request");
    append_typed_commit(store, start).await;

    let interrupted = store::TypedCommitRequest::from_payloads(
        run_id.clone(),
        store
            .expected_next_seq(run_id)
            .await
            .expect("expected next seq"),
        store::CommitKey::new("cli-interrupted-attempt-terminal").expect("commit key"),
        vec![events::KernelEventPayload::StateAttemptInterrupted(
            events::StateAttemptInterrupted {
                spec_hash: spec_hash.clone(),
                node_id: node.node_id.clone(),
                attempt_id: attempt_id.clone(),
            },
        )],
        Vec::new(),
        store::CommitPreconditions {
            required_run_state: store::RequiredRunState::NotCompleted,
            required_cell_states: vec![store::CellStatePrecondition {
                cell_id: node.output_cell.clone(),
                required: store::RequiredCellState::Absent,
            }],
            ..store::CommitPreconditions::default()
        },
    )
    .expect("attempt interrupted request");
    append_typed_commit(store, interrupted).await;
}

async fn append_typed_commit(
    store: &PostgresTypedRunEventStore,
    request: store::TypedCommitRequest,
) {
    let admitted_artifacts = request.required_artifacts().to_vec();
    let artifacts = store::CommitArtifactEvidenceSet::new(
        request.required_artifacts().to_vec(),
        admitted_artifacts,
    )
    .expect("artifact evidence set");
    let plan = if request
        .payloads()
        .iter()
        .all(|payload| matches!(payload, events::KernelEventPayload::StateAttemptStarted(_)))
    {
        store::PreparedCommit::<store::StateAttemptStarted>::new(request, artifacts)
            .expect("prepared attempt-start commit")
            .into()
    } else {
        store::PreparedCommit::<store::AttemptTerminal>::new(request, artifacts)
            .expect("prepared attempt-terminal commit")
            .into()
    };
    store
        .append_prepared_commit_plan(plan)
        .await
        .expect("append typed commit");
}

fn fixed_attempt_id(byte: u8) -> AttemptId {
    AttemptId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([byte; 32]),
    )
}

fn run_cli(args: &[String]) -> Output {
    let mut cmd = Command::cargo_bin("mfm_cli").expect("binary exists");
    sanitize_machine_readable_cli_env(&mut cmd)
        .args(args)
        .output()
        .expect("execute mfm_cli")
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn parse_success_json(stdout: &[u8]) -> Value {
    let parsed: Value = serde_json::from_slice(stdout).expect("stdout must be valid json");
    assert_eq!(parsed["status"], "success");
    parsed["data"].clone()
}

fn sanitize_machine_readable_cli_env(cmd: &mut Command) -> &mut Command {
    cmd.env_remove("LOG_LEVEL")
        .env_remove("RUST_LOG")
        .env_remove("LOG_FORMAT")
        .env_remove("LOG_SPAN_EVENTS")
}

fn assert_framework_started_before_terminal_evidence(
    stream_events: &[Value],
    attempts: &[Value],
    nodes: &[spec::NodeSpec],
    run_id: &RunId,
) {
    for node in nodes.iter().filter(|node| node.framework.is_some()) {
        let required_kind = match &node.framework {
            Some(spec::FrameworkNodeSpec::PublicOutputRender(_)) => "public_output_render",
            Some(spec::FrameworkNodeSpec::ProjectRetentionManifest(_)) => {
                "project_retention_manifest"
            }
            Some(spec::FrameworkNodeSpec::CompleteRun(_)) => "complete_run",
            _ => continue,
        };
        let attempt = attempts
            .iter()
            .find(|attempt| {
                attempt["node_id"].as_str() == Some(node.node_id.as_str())
                    && attempt["disposition"].as_str() == Some("completed")
            })
            .unwrap_or_else(|| {
                panic!(
                    "missing completed {required_kind} framework attempt for {}",
                    node.node_id
                )
            });
        let attempt_id = attempt["attempt_id"].as_str().expect("attempt id");
        let attempt_key = format!("attempt:{}:{}", node.node_id, attempt_id);
        let start_index = stream_event_position(
            stream_events,
            |event| {
                event["logical_key"].as_str() == Some(attempt_key.as_str())
                    && event["event_schema_id"]
                        .as_str()
                        .is_some_and(|schema| schema.contains("state_attempt_started"))
            },
            &format!("framework start {attempt_key}"),
        );
        let completed_index = stream_event_position(
            stream_events,
            |event| {
                event["logical_key"].as_str() == Some(attempt_key.as_str())
                    && event["event_schema_id"]
                        .as_str()
                        .is_some_and(|schema| schema.contains("state_attempt_completed"))
            },
            &format!("framework completion {attempt_key}"),
        );
        assert!(
            start_index < completed_index,
            "framework StateAttemptStarted must precede StateAttemptCompleted for {attempt_key}"
        );

        match &node.framework {
            Some(spec::FrameworkNodeSpec::PublicOutputRender(_)) => {
                let public_output_index = stream_event_position(
                    stream_events,
                    |event| {
                        event["logical_key"]
                            .as_str()
                            .is_some_and(|key| key.starts_with("public_output:"))
                            && event["event_schema_id"]
                                .as_str()
                                .is_some_and(|schema| schema.contains("public_output_produced"))
                    },
                    "public-output terminal evidence",
                );
                assert!(
                    start_index < public_output_index,
                    "public-output framework start must precede public output evidence"
                );
            }
            Some(spec::FrameworkNodeSpec::ProjectRetentionManifest(_)) => {
                let retention_prefix = format!("retention:{}:manifest:", run_id);
                let retention_index = stream_event_position(
                    stream_events,
                    |event| {
                        event["logical_key"]
                            .as_str()
                            .is_some_and(|key| key.starts_with(&retention_prefix))
                            && event["event_schema_id"].as_str().is_some_and(|schema| {
                                schema.contains("retention_manifest_projected")
                            })
                    },
                    "retention manifest terminal evidence",
                );
                assert!(
                    start_index < retention_index,
                    "retention framework start must precede retention manifest evidence"
                );
            }
            Some(spec::FrameworkNodeSpec::CompleteRun(_)) => {
                let completed_run_index = stream_event_position(
                    stream_events,
                    |event| {
                        event["logical_key"].as_str() == Some("run:complete")
                            && event["event_schema_id"]
                                .as_str()
                                .is_some_and(|schema| schema.contains("run_completed"))
                    },
                    "run completion terminal evidence",
                );
                assert!(
                    start_index < completed_run_index,
                    "complete-run framework start must precede run completion evidence"
                );
            }
            _ => {}
        }
    }
}

fn stream_event_position(
    events: &[Value],
    predicate: impl Fn(&Value) -> bool,
    label: &str,
) -> usize {
    events
        .iter()
        .position(predicate)
        .unwrap_or_else(|| panic!("missing stream event for {label}"))
}
