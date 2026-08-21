//! CLI-driven end-to-end verification of the stored-config client surface.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

const CLI: &str = env!("CARGO_BIN_EXE_mfm_cli");

fn config_document(portfolio_id: &str) -> String {
    format!(
        r#"{{
  "entry_point": "mfm.portfolio/snapshot@1",
  "input": {{
    "routes": [{{"chain_id": 1337, "endpoint_id": "reth-dev"}}],
    "selector": {{"target": "{portfolio_id}", "quote": "usd"}},
    "portfolio": {{"portfolio_id": "{portfolio_id}", "quotes": ["usd"],
      "collections": [{{"correlation": "native-collection",
        "request": {{"sources": [{{"source_id": "wallet.native", "chain_id": 1337,
          "address": "0x70997970c51812dc3a010c7d01b50e0d17dc79c8", "token": null}}],
          "decimals": 18}}}}]}}
  }}
}}"#
    )
}

#[tokio::test]
#[ignore = "requires the managed PostgreSQL and HTTP Reth services from cli-e2e"]
async fn stored_config_lifecycle_and_runs_are_cli_complete() {
    for name in [
        "MFM_E2E_RUNTIME_POSTGRES_LOCATOR",
        "MFM_E2E_ADMIN_POSTGRES_LOCATOR",
        "MFM_E2E_EVM_ADAPTER_LOCATOR",
    ] {
        std::env::var(name).unwrap_or_else(|_| panic!("cli-e2e must supply {name}"));
    }

    let root = temporary_root();
    let xdg = root.join("xdg");
    let default_deployment = xdg.join("mfm/deployment.toml");
    let override_deployment = root.join("override.toml");
    std::fs::create_dir_all(default_deployment.parent().expect("deployment parent"))
        .expect("create XDG tree");
    let deployment = r#"[postgres]
runtime_locator_env = "MFM_E2E_RUNTIME_POSTGRES_LOCATOR"

[[evm_routes]]
chain_id = 1337
endpoint_id = "reth-dev"
adapter_locator_env = "MFM_E2E_EVM_ADAPTER_LOCATOR"
"#;
    std::fs::write(&default_deployment, deployment).expect("default deployment");
    std::fs::write(&override_deployment, deployment).expect("override deployment");
    let original_document = root.join("original.json");
    let replacement_document = root.join("replacement.json");
    std::fs::write(&original_document, config_document("portfolio-example"))
        .expect("original config");
    std::fs::write(
        &replacement_document,
        config_document("portfolio-replacement"),
    )
    .expect("replacement config");

    let entry_points = run_cli(&["--output", "json", "entry-point", "list"], &xdg);
    assert_success(&entry_points, "entry-point list");
    assert_eq!(
        json(&entry_points)["items"][0]["entry_point"],
        "mfm.portfolio/snapshot@1"
    );
    let rejected_deployment = run_cli(
        &[
            "--deployment",
            path(&override_deployment),
            "entry-point",
            "list",
        ],
        &xdg,
    );
    assert_eq!(rejected_deployment.status.code(), Some(2));

    for label in ["first init", "idempotent init"] {
        let initialized = run_cli(
            &[
                "--deployment",
                path(&override_deployment),
                "postgres",
                "init",
                "--admin-locator-env",
                "MFM_E2E_ADMIN_POSTGRES_LOCATOR",
            ],
            &xdg,
        );
        assert_success(&initialized, label);
        assert!(initialized.stdout.is_empty());
    }

    let bindings = run_cli(&["--output", "json", "binding", "list"], &xdg);
    assert_success(&bindings, "binding list through XDG default");
    assert_eq!(json(&bindings)["items"][0]["kind"], "evm");
    assert_eq!(json(&bindings)["items"][0]["chain_id"], 1337);
    assert_eq!(json(&bindings)["items"][0]["endpoint_id"], "reth-dev");

    for name in ["daily", "weekly"] {
        let imported = run_cli(
            &[
                "--output",
                "json",
                "config",
                "import",
                name,
                "--from",
                path(&original_document),
            ],
            &xdg,
        );
        assert_success(&imported, "config import");
        assert_eq!(json(&imported)["outcome"], "created");
    }
    let unchanged = run_cli(
        &[
            "--output",
            "json",
            "config",
            "import",
            "daily",
            "--from",
            path(&original_document),
        ],
        &xdg,
    );
    assert_success(&unchanged, "unchanged import");
    assert_eq!(json(&unchanged)["outcome"], "unchanged");
    let digest = json(&unchanged)["config"]["digest"]
        .as_str()
        .expect("config digest")
        .to_owned();

    let configs = run_cli(&["--output", "json", "config", "list"], &xdg);
    assert_success(&configs, "complete config list");
    let configs = json(&configs);
    assert_eq!(configs["items"][0]["name"], "daily");
    assert_eq!(configs["items"][1]["name"], "weekly");

    let started = run_cli(
        &["--output", "json", "run", "start", "--config", "daily"],
        &xdg,
    );
    assert_success(&started, "generated-id Current start");
    let started_json = json(&started);
    let generated_id = started_json["run"]["run_id"]
        .as_str()
        .expect("generated run id")
        .to_owned();
    assert!(generated_id.starts_with("run:sha256-jcs-v1:"));
    assert_eq!(started_json["config"]["digest"], digest);
    assert_eq!(started_json["run"]["state"]["kind"], "succeeded");
    assert_eq!(started_json["run"]["head_sequence"], 11);
    assert_eq!(
        started_json["run"]["state"]["value"]["report"]["quote"],
        "usd"
    );

    let explicit_id = format!("run:sha256-jcs-v1:{:064x}", unique_suffix());
    let exact = run_cli(
        &[
            "--output",
            "json",
            "run",
            "start",
            "--config",
            "daily",
            "--config-digest",
            &digest,
            "--run-id",
            &explicit_id,
        ],
        &xdg,
    );
    assert_success(&exact, "explicit-id Exact start");
    assert_eq!(json(&exact)["run"]["run_id"], explicit_id);

    let progressed = run_cli(
        &[
            "--output",
            "json",
            "run",
            "progress",
            "--run-id",
            &explicit_id,
        ],
        &xdg,
    );
    assert_success(&progressed, "progress terminal run");
    assert_eq!(json(&progressed), json(&exact)["run"]);

    let shown = run_cli(&["run", "show", "--run-id", &generated_id], &xdg);
    assert_success(&shown, "text run show");
    let shown = String::from_utf8(shown.stdout).expect("text UTF-8");
    for field in ["run_id=", "contract_ref=", "value_ref=", "value="] {
        assert!(shown.contains(field), "missing {field}: {shown}");
    }

    let run_page = run_cli(&["--output", "json", "run", "list", "--limit", "1"], &xdg);
    assert_success(&run_page, "run page");
    let run_page = json(&run_page);
    assert_eq!(run_page["items"].as_array().expect("items").len(), 1);
    assert!(run_page["items"][0].get("state").is_none());
    let next_after = run_page["next_after"]
        .as_str()
        .expect("next run id")
        .to_owned();
    assert_eq!(next_after, run_page["items"][0]["run_id"]);
    let next_run_page = run_cli(
        &[
            "--output",
            "json",
            "run",
            "list",
            "--after",
            &next_after,
            "--limit",
            "1",
        ],
        &xdg,
    );
    assert_success(&next_run_page, "next run page");
    assert_eq!(
        json(&next_run_page)["items"]
            .as_array()
            .expect("next items")
            .len(),
        1
    );

    let updated = run_cli(
        &[
            "--output",
            "json",
            "config",
            "import",
            "daily",
            "--from",
            path(&replacement_document),
        ],
        &xdg,
    );
    assert_success(&updated, "replace name");
    assert_eq!(json(&updated)["outcome"], "updated");
    let replacement_digest = json(&updated)["config"]["digest"]
        .as_str()
        .expect("replacement digest")
        .to_owned();
    assert_ne!(replacement_digest, digest);

    let mismatch = run_cli(
        &[
            "--output",
            "json",
            "run",
            "start",
            "--config",
            "daily",
            "--config-digest",
            &digest,
            "--run-id",
            &format!("run:sha256-jcs-v1:{:064x}", unique_suffix() + 1),
        ],
        &xdg,
    );
    assert_eq!(mismatch.status.code(), Some(2));
    assert_eq!(json_stderr(&mismatch)["code"], "config_digest_mismatch");

    let retained = run_cli(
        &["--output", "json", "run", "show", "--run-id", &generated_id],
        &xdg,
    );
    assert_success(&retained, "retained run after config replacement");
    assert_eq!(json(&retained)["run_id"], generated_id);

    let old_grammar = run_cli(&["snapshot"], &xdg);
    assert_eq!(old_grammar.status.code(), Some(2));
    let removed_delete = run_cli(&["config", "delete", "daily"], &xdg);
    assert_eq!(removed_delete.status.code(), Some(2));
    std::fs::remove_dir_all(&root).expect("remove isolated e2e tree");
}

fn temporary_root() -> PathBuf {
    std::env::temp_dir().join(format!("mfm-cli-e2e-{:032x}", unique_suffix()))
}

fn unique_suffix() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("wall clock")
        .as_nanos()
        ^ u128::from(std::process::id())
}

fn path(path: &Path) -> &str {
    path.to_str().expect("UTF-8 fixture path")
}

fn run_cli(arguments: &[&str], xdg: &Path) -> Output {
    Command::new(CLI)
        .args(arguments)
        .env("XDG_CONFIG_HOME", xdg)
        .env("HOME", "/nonexistent-hostile-home")
        .env("MFM_DEPLOYMENT", "/nonexistent-hostile-deployment")
        .output()
        .expect("run CLI")
}

fn json(output: &Output) -> serde_json::Value {
    serde_json::from_slice(&output.stdout).expect("stdout JSON")
}

fn json_stderr(output: &Output) -> serde_json::Value {
    serde_json::from_slice(&output.stderr).expect("stderr JSON")
}

fn assert_success(output: &Output, label: &str) {
    assert_eq!(
        output.status.code(),
        Some(0),
        "{label} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.stderr.is_empty(),
        "{label} wrote to stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
