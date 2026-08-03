use std::fs;
use std::path::{Path, PathBuf};

#[test]
fn application_facade_keeps_only_the_exact_run_surface() {
    let source = read("crates/app/src/application.rs");

    for method in [
        "pub fn entry_points(",
        "pub async fn check_ready(",
        "pub async fn admit_run(",
        "pub async fn drive_once(",
        "pub async fn read_public_run(",
        "pub async fn replay_run(",
        "pub async fn read_transition_trace(",
        "pub async fn read_access_audit(",
        "pub async fn export_run(",
    ] {
        assert!(
            source.contains(method),
            "application facade must retain {method}"
        );
    }

    for removed in [
        "pub async fn list_runs(",
        "pub async fn resume_run(",
        "pub async fn read_facts(",
        "pub async fn read_stream(",
        "pub async fn read_status(",
        "pub async fn read_public_output(",
        "pub async fn record_manual_resolution(",
    ] {
        assert!(
            !source.contains(removed),
            "removed application method {removed} must not survive the cutover"
        );
    }
}

#[test]
fn rest_router_has_only_the_recoverability_run_routes() {
    let source = read("bin/rest-api/src/lib.rs");

    for route in [
        "/v1/health",
        "/v1/ready",
        "/v1/entry-points",
        "/v1/runs",
        "/v1/runs/:run_id",
        "/v1/runs/:run_id/drive",
        "/v1/runs/:run_id/replay",
        "/v1/runs/:run_id/trace",
        "/v1/runs/:run_id/audit",
        "/v1/runs/:run_id/exports",
    ] {
        assert!(
            source.contains(&format!("\"{route}\"")),
            "REST router must register {route}"
        );
    }

    for removed in [
        "/v1/facts",
        "/v1/runs/start",
        "/v1/runs/:run_id/resume",
        "/v1/runs/:run_id/manual-resolution",
        "/v1/runs/:run_id/status",
        "/v1/runs/:run_id/stream",
        "/v1/runs/:run_id/public-output",
    ] {
        assert!(
            !source.contains(removed),
            "removed REST route {removed} must not survive the cutover"
        );
    }
    assert!(
        !source.contains(".route(\"/v1/runs\", get("),
        "GET /v1/runs list/watch must not survive the cutover"
    );
}

#[test]
fn cli_run_tree_has_only_exact_run_commands() {
    let root = repository_root();
    let run_dir = root.join("bin/cli/src/commands/run");
    let run_module = fs::read_to_string(run_dir.join("mod.rs")).expect("read run command module");
    let variants = run_module
        .split_once("pub(crate) enum RunCommand {")
        .and_then(|(_, rest)| rest.split_once("\n}\n\n/// CLI replay modes"))
        .map(|(variants, _)| variants)
        .expect("locate closed RunCommand enum");

    for variant in [
        "Admit", "Audit", "Drive", "Export", "Replay", "Show", "Trace",
    ] {
        assert!(
            variants.contains(&format!("\n    {variant} {{")),
            "RunCommand must register {variant}"
        );
    }

    for (variant, module) in [
        ("List", "list"),
        ("ManualResolution", "manual_resolution"),
        ("PublicOutput", "public_output"),
        ("Resume", "resume"),
        ("Start", "start"),
        ("Status", "status"),
        ("Stream", "stream"),
    ] {
        assert!(
            !variants.contains(&format!("\n    {variant}")),
            "removed RunCommand variant {variant} must not survive the cutover"
        );
        assert!(
            !run_dir.join(format!("{module}.rs")).exists(),
            "removed run command module {module} must be deleted"
        );
    }

    assert!(
        !root.join("bin/cli/src/commands/facts.rs").exists(),
        "the public fact command must be deleted"
    );
    let command_root = read("bin/cli/src/commands/mod.rs");
    assert!(
        !command_root.contains("mod facts;"),
        "the public fact command must not remain registered"
    );

    let standalone_connection = read("bin/cli/src/support/application.rs");
    assert!(
        standalone_connection.contains("ErrorClass::ServiceUnavailable")
            && standalone_connection.contains("\"AuthoritativeWriterFenceUnavailable\""),
        "the standalone CLI must fail closed without a deployment-owned writer fence"
    );
    let ops = read("bin/cli/src/commands/ops.rs");
    assert!(
        ops.contains("connect_application(connection)")
            && ops.contains("application.entry_points()")
            && !ops.contains("Application::entry_points()"),
        "ops discovery must read the cache only after the shared production bootstrap"
    );
}

#[test]
fn projection_renderers_use_only_the_current_structured_wrappers() {
    let renderer = read("crates/app/src/render.rs");
    for field in [
        "run_id",
        "at_journal_head",
        "transitions",
        "complete_as_of_journal_head",
        "entries",
        "next_cursor",
    ] {
        assert!(
            renderer.contains(&format!("\"{field}\"")),
            "projection renderer must retain {field}"
        );
    }
    for removed in [
        "authorization_commit",
        "observation_commit",
        "capability_id",
        "operation_id",
        "request_identity",
        "result_identity",
        "non_domain_failure",
        "effect_key",
        "delivery_audit_ref",
        "delivery_audit_terminal",
        "executor_frontier_ref",
        "executor_frontier_sealed_terminal",
    ] {
        assert!(
            !renderer.contains(&format!("\"{removed}\"")),
            "superseded audit field {removed} must not survive"
        );
    }
}

#[tokio::test]
async fn standalone_rest_bootstrap_fails_closed_without_a_writer_fence() {
    let error = match mfm_rest_api::make_default_app_state(None).await {
        Ok(_) => panic!("standalone process has no deployment-owned writer fence"),
        Err(error) => error,
    };
    assert_eq!(error.status(), axum::http::StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        error.public_error().code(),
        "AuthoritativeWriterFenceUnavailable"
    );
}

fn read(relative: &str) -> String {
    fs::read_to_string(repository_root().join(relative))
        .unwrap_or_else(|error| panic!("read {relative}: {error}"))
}

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("integration crate must live under tests/integration")
        .to_path_buf()
}
