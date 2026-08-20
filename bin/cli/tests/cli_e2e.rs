//! CLI-driven end-to-end verification of live EVM native-balance collection.
//!
//! The test operates the binary exactly as a user does: one configuration file, `init`,
//! `snapshot` under an explicit RunId, and a second independent process reading the same run
//! back with `show`. Everything it asserts is observable from the transport contract alone.

use std::path::PathBuf;
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

const CLI: &str = env!("CARGO_BIN_EXE_mfm_cli");

const CONFIG: &str = r#"{
  "portfolio": { "portfolio_id": "portfolio-example", "quotes": ["usd"],
    "collections": [{ "correlation": "native-collection",
      "request": { "sources": [{ "source_id": "wallet.native", "chain_id": 1337,
        "address": "0x70997970c51812dc3a010c7d01b50e0d17dc79c8", "token": null }],
        "decimals": 18 } }] },
  "selector": { "target": "portfolio-example", "quote": "usd" },
  "evm": { "chain_id": 1337, "endpoint_id": "reth-dev", "rpc_url_env": "MFM_E2E_RPC_URL" },
  "store": { "runtime_locator_env": "MFM_E2E_RUNTIME_STORE_LOCATOR" }
}
"#;

#[tokio::test]
#[ignore = "requires the managed postgres and reth services provided by the cli-e2e task"]
async fn native_snapshot_run_and_read_back_via_cli() {
    std::env::var("MFM_E2E_RPC_URL").expect("cli-e2e must supply MFM_E2E_RPC_URL");
    std::env::var("MFM_E2E_RUNTIME_STORE_LOCATOR")
        .expect("cli-e2e must supply the runtime locator");
    std::env::var("MFM_E2E_ADMIN_STORE_LOCATOR").expect("cli-e2e must supply the admin locator");

    let unique = unique_suffix();
    let config = write_config(unique);
    let run_id = format!("run:sha256-jcs-v1:{unique:064x}");

    let first_init = run_cli(&[
        "init",
        "--config",
        config.to_str().expect("config path"),
        "--admin-store-locator-env",
        "MFM_E2E_ADMIN_STORE_LOCATOR",
    ]);
    assert_success(&first_init, "init");
    assert!(
        first_init.stdout.is_empty(),
        "init prints nothing on success"
    );
    assert_success(
        &run_cli(&[
            "init",
            "--config",
            config.to_str().expect("config path"),
            "--admin-store-locator-env",
            "MFM_E2E_ADMIN_STORE_LOCATOR",
        ]),
        "idempotent init",
    );

    let snapshot = run_cli(&[
        "snapshot",
        "--config",
        config.to_str().expect("config path"),
        "--run-id",
        &run_id,
    ]);
    assert_success(&snapshot, "snapshot");
    let rendered = String::from_utf8(snapshot.stdout.clone()).expect("utf8 view");
    let lines = rendered.lines().collect::<Vec<_>>();
    assert_eq!(
        lines.len(),
        5,
        "rendered view is four fields plus one value"
    );
    assert_eq!(lines[0], format!("run_id={run_id}"));
    // The single-native-source Program: admission, three Pure States, four Reads, and the
    // consolidations, all fused into eleven durable frames.
    assert_eq!(lines[1], "head_sequence=11");
    assert!(
        lines[2].starts_with("head_digest=content:sha256-v1:"),
        "unexpected head digest line: {}",
        lines[2]
    );
    assert_eq!(lines[3], "state=succeeded");
    for expected in [
        r#""chain_id":1337"#,
        r#""kind":"native""#,
        r#""decimals":18"#,
        r#""raw_units":"1000000000000000000000000""#,
        r#""total_value_dec":"1000000""#,
    ] {
        assert!(
            lines[4].contains(expected),
            "canonical output is missing {expected}: {}",
            lines[4]
        );
    }

    // Re-admitting the exact same run across processes returns the retained bytes unchanged.
    let readmitted = run_cli(&[
        "snapshot",
        "--config",
        config.to_str().expect("config path"),
        "--run-id",
        &run_id,
    ]);
    assert_success(&readmitted, "durable re-admission");
    assert_eq!(readmitted.stdout, snapshot.stdout);

    // One independent process reads the identical run back out of the durable store.
    let shown = run_cli(&[
        "show",
        "--config",
        config.to_str().expect("config path"),
        "--run-id",
        &run_id,
    ]);
    assert_success(&shown, "show");
    assert_eq!(shown.stdout, snapshot.stdout);

    std::fs::remove_file(&config).expect("remove config");
}

/// Returns one value unique per execution: the store is durable and the chain stays warm.
fn unique_suffix() -> u128 {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("wall clock")
        .as_nanos();
    nanos ^ u128::from(std::process::id())
}

fn write_config(unique: u128) -> PathBuf {
    let path = std::env::temp_dir().join(format!("mfm-cli-e2e-{unique:032x}.json"));
    std::fs::write(&path, CONFIG).expect("write config");
    path
}

fn run_cli(arguments: &[&str]) -> Output {
    Command::new(CLI)
        .args(arguments)
        .output()
        .expect("run the cli binary")
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
