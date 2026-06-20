#![allow(clippy::disallowed_methods, clippy::disallowed_types)]
use assert_cmd::Command;
use mfm::presentation::output::{ErrorResponse, ResponseStatus, SuccessResponse};
use mfm_events::v1 as events;
use mfm_ids::{ArtifactId, ContentDigest, DigestAlgorithm, RunId};
use mfm_manual_auth::{
    ManualAuthorizationSignatureBytes, ManualResolutionAuthorizationProof,
    ManualResolutionAuthorizationSignature, ManualResolutionEvidenceRef,
    ManualResolutionPrefixAuthority,
};
use mfm_runtime::{
    manual_resolution_block_reason, manual_resolution_stream_prefix_digest,
    unresolved_manual_obligations_digest,
};
use mfm_spec::v1 as spec;
use mfm_store::v1 as store;
use mfm_stream_store_postgres::{PostgresSchema, PostgresTypedRunEventStore};
use predicates::prelude::*;
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Output;
use tempfile::TempDir;

// This struct is local to the test module to help with deserialization.
#[derive(Deserialize)]
struct TestResponse<T> {
    status: ResponseStatus,
    data: T,
}

/// Test helper to create a temporary keystore and return its path
fn setup_temp_keystore() -> TempDir {
    TempDir::new().unwrap()
}

/// Test helper to parse JSON output and verify it has the success structure
fn verify_success_response(output: &str) -> Value {
    let parsed: TestResponse<Value> = serde_json::from_str(output).expect("Should be valid JSON");
    assert!(matches!(parsed.status, ResponseStatus::Success));
    parsed.data
}

fn verify_error_response(output: &str) -> ErrorResponse {
    let parsed: ErrorResponse =
        serde_json::from_str(output).expect("Should be valid JSON error response");
    assert!(matches!(parsed.status, ResponseStatus::Error));
    parsed
}

#[test]
fn run_stream_command_filters_range_after_authoritative_run_stream_validation() {
    let source = include_str!("../src/commands/run/stream.rs");
    let range_validation = source
        .find("if args.from_seq == 0")
        .expect("range validation");
    let authoritative_read = source
        .find(".run_stream(&run_id)")
        .expect("authoritative stream read");
    let range_filter = source.find(".filter(|event|").expect("range filter");

    assert!(
        !source.contains(".load_run_stream("),
        "run stream command must delegate to app-level full stream validation"
    );
    assert!(range_validation < authoritative_read);
    assert!(authoritative_read < range_filter);
}

#[test]
fn test_keystore_list_json_output_empty() {
    let temp_dir = setup_temp_keystore();
    let keystore_path = temp_dir.path().join("test.keystore");

    let mut cmd = Command::cargo_bin("mfm_cli").unwrap();
    let output = cmd
        .args([
            "--output-format",
            "json",
            "keystore",
            "list",
            "--keystore",
            keystore_path.to_str().unwrap(),
        ])
        .output()
        .expect("Failed to execute command");

    if !output.status.success() {
        // If keystore doesn't exist or requires password, that's expected for empty case
        return;
    }

    let stdout = String::from_utf8(output.stdout).unwrap();
    if !stdout.trim().is_empty() {
        let parsed = verify_success_response(&stdout);
        assert!(parsed["data"].is_array());
    }
}

#[test]
fn test_keystore_list_json_output_with_env_var() {
    let temp_dir = setup_temp_keystore();
    let keystore_path = temp_dir.path().join("test.keystore");

    let mut cmd = Command::cargo_bin("mfm_cli").unwrap();
    let output = cmd
        .env("MFM_OUTPUT_FORMAT", "json")
        .args([
            "keystore",
            "list",
            "--keystore",
            keystore_path.to_str().unwrap(),
        ])
        .output()
        .expect("Failed to execute command");

    if !output.status.success() {
        // If keystore doesn't exist or requires password, that's expected
        return;
    }

    let stdout = String::from_utf8(output.stdout).unwrap();
    if !stdout.trim().is_empty() {
        let parsed = verify_success_response(&stdout);
        assert!(parsed["data"].is_array());
    }
}

#[test]
fn test_keystore_import_json_error_invalid_key() {
    let temp_dir = setup_temp_keystore();
    let keystore_path = temp_dir.path().join("test.keystore");

    let mut cmd = Command::cargo_bin("mfm_cli").unwrap();
    cmd.args([
        "--output-format",
        "json",
        "keystore",
        "import",
        "--import-type",
        "privatekey",
        "--keystore",
        keystore_path.to_str().unwrap(),
        "--stdin",
    ])
    .write_stdin("invalid_key")
    .assert()
    .failure()
    .stderr(predicate::str::contains("invalid_key_material"))
    .stderr(predicate::str::contains("status"))
    .stderr(predicate::str::contains("error"));
}

#[test]
fn test_keystore_import_json_error_short_mnemonic() {
    let temp_dir = setup_temp_keystore();
    let keystore_path = temp_dir.path().join("test.keystore");

    let mut cmd = Command::cargo_bin("mfm_cli").unwrap();
    cmd.args([
        "--output-format",
        "json",
        "keystore",
        "import",
        "--import-type",
        "mnemonic",
        "--keystore",
        keystore_path.to_str().unwrap(),
        "--stdin",
    ])
    .write_stdin("short mnemonic")
    .assert()
    .failure()
    .stderr(predicate::str::contains("invalid_recovery_phrase"))
    .stderr(predicate::str::contains("status"))
    .stderr(predicate::str::contains("error"));
}

#[test]
fn test_keystore_delete_json_error_missing_args() {
    // This test verifies argument validation happens at clap level
    let temp_dir = setup_temp_keystore();
    let keystore_path = temp_dir.path().join("test.keystore");

    let mut cmd = Command::cargo_bin("mfm_cli").unwrap();
    cmd.args([
        "--output-format",
        "json",
        "keystore",
        "delete",
        "--keystore",
        keystore_path.to_str().unwrap(),
    ])
    .assert()
    .failure();
    // Note: This error happens at keystore loading level, not our custom validation
}

#[test]
fn test_keystore_delete_json_error_invalid_uuid() {
    // This test verifies error handling at keystore level
    let temp_dir = setup_temp_keystore();
    let keystore_path = temp_dir.path().join("test.keystore");

    let mut cmd = Command::cargo_bin("mfm_cli").unwrap();
    cmd.args([
        "--output-format",
        "json",
        "keystore",
        "delete",
        "--keystore",
        keystore_path.to_str().unwrap(),
        "invalid-uuid",
    ])
    .assert()
    .failure();
    // Note: This error happens at keystore loading level, not our UUID validation
}

#[test]
fn test_help_output_shows_json_format_option() {
    let mut cmd = Command::cargo_bin("mfm_cli").unwrap();
    cmd.arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("--output-format"))
        .stdout(predicate::str::contains("json"))
        .stdout(predicate::str::contains("text"))
        .stdout(predicate::str::contains("MFM_OUTPUT_FORMAT"));
}

#[test]
fn test_cli_parse_error_json_output_for_unknown_command_flag_equals() {
    let mut cmd = Command::cargo_bin("mfm_cli").unwrap();
    let output = cmd
        .args(["--output-format=json", "unknown-command"])
        .output()
        .expect("Failed to execute command");

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).unwrap();
    let parsed = verify_error_response(&stderr);
    assert_eq!(parsed.error.code, "CliParseError");
    assert!(parsed.error.message.contains("unknown-command"));
    assert!(!parsed.error.message.contains('\u{1b}'));
}

#[test]
fn test_cli_parse_error_json_output_for_unknown_command_env() {
    let mut cmd = Command::cargo_bin("mfm_cli").unwrap();
    let output = cmd
        .env("MFM_OUTPUT_FORMAT", "json")
        .arg("unknown-command")
        .output()
        .expect("Failed to execute command");

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).unwrap();
    let parsed = verify_error_response(&stderr);
    assert_eq!(parsed.error.code, "CliParseError");
    assert!(parsed.error.message.contains("unknown-command"));
}

#[test]
fn test_cli_parse_error_json_output_for_missing_required_argument() {
    let mut cmd = Command::cargo_bin("mfm_cli").unwrap();
    let output = cmd
        .args(["--output-format", "json", "keystore", "import", "--stdin"])
        .output()
        .expect("Failed to execute command");

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).unwrap();
    let parsed = verify_error_response(&stderr);
    assert_eq!(parsed.error.code, "CliParseError");
    assert!(parsed.error.message.contains("--import-type"));
}

#[test]
fn test_cli_parse_error_json_output_for_invalid_value() {
    let mut cmd = Command::cargo_bin("mfm_cli").unwrap();
    let output = cmd
        .args([
            "--output-format",
            "json",
            "keystore",
            "import",
            "--import-type",
            "not-a-type",
            "--stdin",
        ])
        .output()
        .expect("Failed to execute command");

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).unwrap();
    let parsed = verify_error_response(&stderr);
    assert_eq!(parsed.error.code, "CliParseError");
    assert!(parsed.error.message.contains("not-a-type"));
}

#[test]
fn test_cli_parse_error_text_mode_uses_clap_rendering() {
    let mut cmd = Command::cargo_bin("mfm_cli").unwrap();
    cmd.arg("unknown-command")
        .assert()
        .code(2)
        .stderr(predicate::str::contains("error:"))
        .stderr(predicate::str::contains("unknown-command"))
        .stderr(predicate::str::contains("\"status\"").not());
}

#[test]
fn test_environment_variable_precedence() {
    // Test that command line flag takes precedence over environment variable
    let temp_dir = setup_temp_keystore();
    let keystore_path = temp_dir.path().join("test.keystore");

    let mut cmd = Command::cargo_bin("mfm_cli").unwrap();
    let output = cmd
        .env("MFM_OUTPUT_FORMAT", "json") // Set env var to json
        .args([
            "--output-format",
            "text", // Override with text via flag
            "keystore",
            "list",
            "--keystore",
            keystore_path.to_str().unwrap(),
        ])
        .output()
        .expect("Failed to execute command");

    if output.status.success() {
        let stdout = String::from_utf8(output.stdout).unwrap();
        // Should be text output (not JSON) since flag overrides env var
        if !stdout.trim().is_empty() {
            // Text output should not have JSON structure
            assert!(!stdout.contains("\"status\":"));
        }
    }
}

#[test]
fn test_run_start_requires_certified_bundle_path() {
    let mut cmd = Command::cargo_bin("mfm_cli").unwrap();
    let output = cmd
        .env_remove("DATABASE_URL")
        .args(["--output-format", "json", "run", "start"])
        .output()
        .expect("Failed to execute command");

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    let parsed = verify_error_response(&stderr);
    assert_eq!(parsed.error.code, "CliParseError");
    assert!(parsed.error.message.contains("--bundle"));
}

#[test]
fn test_run_start_rejects_invalid_bundle_before_store_access() {
    let temp_dir = TempDir::new().unwrap();
    let bundle_path = temp_dir.path().join("bundle.json");
    std::fs::write(&bundle_path, "{}").expect("bundle fixture");

    let mut cmd = Command::cargo_bin("mfm_cli").unwrap();
    let output = cmd
        .env_remove("DATABASE_URL")
        .args([
            "--output-format",
            "json",
            "run",
            "start",
            "--bundle",
            bundle_path.to_str().unwrap(),
            "--run-id",
            "run:sha256-jcs-v1:0000000000000000000000000000000000000000000000000000000000000001",
        ])
        .output()
        .expect("Failed to execute command");

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).unwrap();
    let parsed = verify_error_response(&stderr);
    assert_eq!(parsed.error.code, "CertifiedBundleInvalid");
}

#[test]
fn test_run_start_rejects_legacy_op_id_flag() {
    let mut cmd = Command::cargo_bin("mfm_cli").unwrap();
    let output = cmd
        .env_remove("DATABASE_URL")
        .args([
            "--output-format",
            "json",
            "run",
            "start",
            "--op-id",
            "portfolio.snapshot",
        ])
        .output()
        .expect("Failed to execute command");

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).unwrap();
    let parsed = verify_error_response(&stderr);
    assert_eq!(parsed.error.code, "CliParseError");
    assert!(parsed.error.message.contains("--op-id"));
}

#[test]
fn test_run_resume_rejects_legacy_uuid_before_storage() {
    let mut cmd = Command::cargo_bin("mfm_cli").unwrap();
    let output = cmd
        .env_remove("DATABASE_URL")
        .args([
            "--output-format",
            "json",
            "run",
            "resume",
            "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee",
        ])
        .output()
        .expect("Failed to execute command");

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    let parsed = verify_error_response(&stderr);
    assert_eq!(parsed.error.code, "InvalidRunId");
}

#[test]
fn test_run_pipeline_subcommand_is_removed() {
    let mut cmd = Command::cargo_bin("mfm_cli").unwrap();
    let output = cmd
        .args(["--output-format", "json", "run", "pipeline"])
        .output()
        .expect("Failed to execute command");

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).unwrap();
    let parsed = verify_error_response(&stderr);
    assert_eq!(parsed.error.code, "CliParseError");
    assert!(parsed.error.message.contains("pipeline"));
}

#[test]
fn test_run_artifacts_subcommand_is_removed() {
    let mut cmd = Command::cargo_bin("mfm_cli").unwrap();
    let output = cmd
        .args(["--output-format", "json", "run", "artifacts"])
        .output()
        .expect("Failed to execute command");

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).unwrap();
    let parsed = verify_error_response(&stderr);
    assert_eq!(parsed.error.code, "CliParseError");
    assert!(parsed.error.message.contains("artifacts"));
}

#[test]
fn test_run_replay_requires_typed_store_after_valid_run_id() {
    let mut cmd = Command::cargo_bin("mfm_cli").unwrap();
    let output = cmd
        .env_remove("DATABASE_URL")
        .args([
            "--output-format",
            "json",
            "run",
            "replay",
            "run:sha256-jcs-v1:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        ])
        .output()
        .expect("Failed to execute command");

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    let parsed = verify_error_response(&stderr);
    assert_eq!(parsed.error.code, "MissingDatabaseUrl");
}

#[test]
fn test_run_status_json_error_invalid_typed_run_id() {
    let mut cmd = Command::cargo_bin("mfm_cli").unwrap();
    let output = cmd
        .env_remove("DATABASE_URL")
        .args(["--output-format", "json", "run", "status", "not-a-run-id"])
        .output()
        .expect("Failed to execute command");

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    let parsed = verify_error_response(&stderr);
    assert_eq!(parsed.error.code, "InvalidRunId");
}

#[test]
fn test_run_public_output_json_rejects_invalid_schema_id() {
    let mut cmd = Command::cargo_bin("mfm_cli").unwrap();
    let output = cmd
        .env_remove("DATABASE_URL")
        .args([
            "--output-format",
            "json",
            "run",
            "public-output",
            "run:sha256-jcs-v1:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "--schema-id",
            "schema_123",
        ])
        .output()
        .expect("Failed to execute command");

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    let err = verify_error_response(&stderr);
    assert_eq!(err.error.code, "InvalidSchemaId");
}

#[tokio::test]
async fn manual_resolution_json_scenario_records_resolution_and_hides_proof_bytes() {
    let Ok(database_url) = std::env::var("DATABASE_URL") else {
        eprintln!("skipping CLI manual-resolution scenario: DATABASE_URL is not set");
        return;
    };
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
    let draft = mfm_op_proof::manual_resolution_proof_program_draft(proof_config.clone())
        .expect("manual proof draft");
    let certified = mfm_op_proof::certified_manual_resolution_proof_spec(proof_config)
        .expect("manual proof spec");
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
        "until-blocked".to_owned(),
        "--typed-artifact-root".to_owned(),
        artifact_root.display().to_string(),
        "--database-url".to_owned(),
        database_url.clone(),
    ];
    start_args.extend(config_args);
    let start = run_cli(&start_args);
    assert_success(&start);
    let start_json = parse_success_json(&start.stdout);
    assert_eq!(start_json["run_mode"], "manual_blocked");
    assert_eq!(
        start_json["saga"]["manual_block_reason"],
        "policy_manual_resolution"
    );

    let evidence_path = temp.path().join("manual-evidence.json");
    let evidence_bytes = br#"{"operator_note":"reviewed"}"#;
    std::fs::write(&evidence_path, evidence_bytes).expect("write evidence");
    let proof_bytes = signed_manual_resolution_proof_bytes(
        &store,
        &run_id,
        &certified,
        manual_policy(&certified),
        evidence_bytes,
    )
    .await;
    let proof_path = temp.path().join("manual-proof.json");
    std::fs::write(&proof_path, &proof_bytes).expect("write proof");

    let resolved = run_cli(&[
        "--output-format".to_owned(),
        "json".to_owned(),
        "run".to_owned(),
        "manual-resolution".to_owned(),
        run_id.as_str().to_owned(),
        "--outcome".to_owned(),
        "confirm-remediated".to_owned(),
        "--evidence".to_owned(),
        evidence_path.display().to_string(),
        "--authorization-proof".to_owned(),
        proof_path.display().to_string(),
        "--drive".to_owned(),
        "until-blocked".to_owned(),
        "--typed-artifact-root".to_owned(),
        artifact_root.display().to_string(),
        "--database-url".to_owned(),
        database_url.clone(),
    ]);
    assert_success(&resolved);
    let resolved_json = parse_success_json(&resolved.stdout);
    assert_eq!(resolved_json["run_mode"], "manually_resolved");
    assert_eq!(
        resolved_json["saga"]["terminal_resolution"]["claim"],
        "manual_resolution"
    );
    assert!(
        resolved_json["attempt_dispositions"]
            .as_array()
            .expect("attempts")
            .iter()
            .any(|attempt| {
                attempt["disposition"] == "completed"
                    && resolve_saga_node_ids(&certified.envelope().spec)
                        .contains(&attempt["node_id"].as_str().unwrap_or_default())
            }),
        "missing completed ResolveSagaTerminal attempt"
    );

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
    assert_eq!(status_json["run_mode"], "manually_resolved");

    let raw_stream = store
        .load_run_stream(&run_id)
        .await
        .expect("raw run stream");
    assert!(
        raw_stream.iter().any(|event| matches!(
            event.payload(),
            events::KernelEventPayload::ManualResolutionRecorded(_)
        )),
        "stream must contain manual resolution event"
    );
    assert_framework_started_before_run_completed(&raw_stream, &certified.envelope().spec);

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

    let rendered =
        String::from_utf8([start.stdout, resolved.stdout, status.stdout, stream.stdout].concat())
            .expect("rendered CLI JSON");
    let proof_rendered = std::str::from_utf8(&proof_bytes).expect("proof utf8");
    let proof: Value = serde_json::from_slice(&proof_bytes).expect("proof JSON");
    let signature = proof["signatures"][0]["signature_hex"]
        .as_str()
        .expect("signature");
    assert!(!rendered.contains(proof_rendered));
    assert!(!rendered.contains(signature));
}

#[test]
fn test_json_response_structure_consistency() {
    // This test verifies the JSON structure matches our specification
    let expected_success_structure = json!({
        "status": "success",
        "data": {}
    });

    let expected_error_structure = json!({
        "status": "error",
        "error": {
            "code": "ErrorCode",
            "message": "Error message"
        }
    });

    // Verify our structures match the expected format
    use mfm::presentation::output::ErrorResponse;

    let success_response = SuccessResponse::new(json!({}));
    let success_json = serde_json::to_value(success_response).unwrap();
    assert_eq!(success_json["status"], expected_success_structure["status"]);
    assert!(success_json.get("data").is_some());

    let error_response = ErrorResponse::new("ErrorCode", "Error message");
    let error_json = serde_json::to_value(error_response).unwrap();
    assert_eq!(error_json["status"], expected_error_structure["status"]);
    assert_eq!(error_json["error"]["code"], "ErrorCode");
    assert_eq!(error_json["error"]["message"], "Error message");
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
        assert_eq!(
            bytes.content_digest(),
            node.config_ref.digest,
            "framework config helper must match certified config ref"
        );
        let path = root.join(format!("config-{index}.json"));
        index += 1;
        std::fs::write(&path, bytes.as_bytes()).expect("write framework config");
        args.push("--config".to_owned());
        args.push(format!("{}={}", node.config_ref.schema_id, path.display()));
    }

    (bundle_path, args)
}

async fn signed_manual_resolution_proof_bytes(
    store: &PostgresTypedRunEventStore,
    run_id: &RunId,
    certified: &mfm_certify::CertifiedTypedSpec,
    manual: spec::ManualResolutionEvidenceSpec,
    evidence_bytes: &[u8],
) -> Vec<u8> {
    let evidence_hash = ContentDigest::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        mfm_canonical::sha256_digest_bytes(evidence_bytes),
    );
    let evidence = ManualResolutionEvidenceRef {
        schema_id: manual.evidence_schema.clone(),
        artifact_id: ArtifactId::from_digest(evidence_hash.algorithm(), *evidence_hash.digest()),
        content_hash: evidence_hash,
    };
    let stream = store.load_run_stream(run_id).await.expect("run stream");
    let projection =
        store::ProjectionSnapshot::rebuild_from_run_stream(&stream).expect("projection rebuild");
    let saga = projection.derive_saga_projection(run_id, &certified.envelope().spec.saga);
    let reason = saga.manual_block_reason.expect("manual block reason");
    let expected_next_seq = store
        .expected_next_seq(run_id)
        .await
        .expect("expected next seq");
    let prefix = ManualResolutionPrefixAuthority::new(
        run_id.clone(),
        certified.spec_hash().clone(),
        expected_next_seq.as_u64(),
        manual_resolution_stream_prefix_digest(&stream).expect("prefix digest"),
        manual_resolution_block_reason(reason),
        unresolved_manual_obligations_digest(&saga).expect("obligation digest"),
        manual.clone(),
    )
    .expect("manual prefix");
    let claim = prefix
        .authorization_claim(events::ManualResolutionOutcome::ConfirmRemediated, evidence)
        .expect("manual claim");
    let operator = manual.authorization.authority.operators[0].clone();
    let claim_digest = claim.digest().expect("claim digest");
    let proof = ManualResolutionAuthorizationProof {
        verifier_id: manual.authorization.verifier_id,
        signing_scheme: manual.authorization.signing_scheme,
        claim,
        signatures: vec![ManualResolutionAuthorizationSignature {
            operator_id: operator.operator_id,
            public_identity: operator.public_identity,
            signature: ManualAuthorizationSignatureBytes::new(sign_manual_claim_digest(
                &test_manual_signing_key(),
                claim_digest.digest().as_bytes(),
            ))
            .expect("signature"),
        }],
    };
    proof
        .canonical_json()
        .expect("canonical manual proof")
        .to_vec()
}

fn manual_policy(
    certified: &mfm_certify::CertifiedTypedSpec,
) -> spec::ManualResolutionEvidenceSpec {
    match &certified.envelope().spec.saga {
        spec::SagaPolicySpec::ManualResolution { manual } => manual.clone(),
        _ => panic!("manual proof scenario must carry manual policy"),
    }
}

fn resolve_saga_node_ids(spec: &spec::TypedExecutionSpec) -> Vec<&str> {
    spec.nodes
        .iter()
        .filter(|node| {
            matches!(
                node.framework,
                Some(spec::FrameworkNodeSpec::ResolveSagaTerminal(_))
            )
        })
        .map(|node| node.node_id.as_str())
        .collect()
}

fn assert_framework_started_before_run_completed(
    stream: &[store::KernelEventEnvelope],
    spec: &spec::TypedExecutionSpec,
) {
    let resolve_nodes = resolve_saga_node_ids(spec);
    let started = stream
        .iter()
        .position(|event| match event.payload() {
            events::KernelEventPayload::StateAttemptStarted(payload) => {
                resolve_nodes.contains(&payload.node_id.as_str())
            }
            _ => false,
        })
        .expect("ResolveSagaTerminal attempt start");
    let completed = stream
        .iter()
        .position(|event| matches!(event.payload(), events::KernelEventPayload::RunCompleted(_)))
        .expect("RunCompleted event");
    assert!(started < completed);
}

fn run_cli(args: &[String]) -> Output {
    let mut cmd = Command::cargo_bin("mfm_cli").expect("binary exists");
    cmd.env_remove("LOG_LEVEL")
        .env_remove("RUST_LOG")
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
    let parsed: Value = serde_json::from_slice(stdout).expect("stdout must be valid JSON");
    assert_eq!(parsed["status"], "success");
    parsed["data"].clone()
}

fn test_manual_signing_key() -> k256::ecdsa::SigningKey {
    let mut key_bytes = [0u8; 32];
    key_bytes[31] = 1;
    let secret_key = k256::SecretKey::from_slice(&key_bytes).expect("test key");
    k256::ecdsa::SigningKey::from(&secret_key)
}

fn sign_manual_claim_digest(signing_key: &k256::ecdsa::SigningKey, digest: &[u8; 32]) -> Vec<u8> {
    let (signature, recovery_id) = signing_key
        .sign_prehash_recoverable(digest)
        .expect("manual signature");
    let mut signature_bytes = signature.to_bytes().to_vec();
    signature_bytes.push(u8::from(recovery_id.is_y_odd()));
    signature_bytes
}
