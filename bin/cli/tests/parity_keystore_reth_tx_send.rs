#![allow(clippy::disallowed_methods)]
#![cfg(feature = "parity-tests")]

use assert_cmd::Command;
use mfm_core::keystore::{Keystore, KeystoreConfig};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Output;
use tempfile::TempDir;

#[path = "support/mod.rs"]
mod support;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

const TEST_PASSWORD: &str = "parity-test-password";

#[test]
fn parity_keystore_cli_tx_sign_writes_eip1559_payload() {
    let temp = TempDir::new().expect("temp dir");
    let keystore_path = temp.path().join("parity.keystore");
    let password_file = write_password_file(temp.path(), TEST_PASSWORD);
    let signed_tx_path = temp.path().join("signed.tx");
    let recipient = "0x1111111111111111111111111111111111111111";
    let private_key = random_private_key_hex();

    let import_output = run_import_private_key(
        &keystore_path,
        &password_file,
        "parity-cli-sender",
        &private_key,
    );
    assert!(
        import_output.status.success(),
        "{}",
        stderr_string(&import_output)
    );
    let import_json = support::parse_success_json(&import_output.stdout);
    let key_id = import_json["id"].as_str().expect("import id").to_string();
    let sender = import_json["address"]
        .as_str()
        .map(normalize_address)
        .expect("imported address");

    let sign_output = run_tx_sign(
        &keystore_path,
        &password_file,
        TxSignArgs {
            id: Some(&key_id),
            to: recipient,
            value_wei: "1000000000000000",
            chain_id: "31337",
            nonce: "0",
            max_fee_per_gas: "2000000000",
            max_priority_fee_per_gas: "1000000000",
            ..TxSignArgs::new(&signed_tx_path)
        },
    );
    assert!(
        sign_output.status.success(),
        "{}",
        stderr_string(&sign_output)
    );
    let sign_json = support::parse_success_json(&sign_output.stdout);
    assert_eq!(
        normalize_address(sign_json["from"].as_str().expect("from")),
        sender
    );
    assert_eq!(
        normalize_address(sign_json["to"].as_str().expect("to")),
        normalize_address(recipient)
    );
    assert_eq!(sign_json["tx_type"].as_str(), Some("0x2"));
    assert_eq!(sign_json["chain_id"].as_u64(), Some(31_337));
    assert_eq!(sign_json["nonce"].as_u64(), Some(0));
    assert!(sign_json["payload_hash"]
        .as_str()
        .is_some_and(|v| v.starts_with("0x")));
    assert!(
        sign_json.get("out_path").is_none(),
        "local output paths must not be emitted"
    );
    assert!(!stdout_string(&sign_output).contains("raw_tx_hex"));
    assert!(!stdout_string(&sign_output).contains(&signed_tx_path.display().to_string()));
    assert!(!stdout_string(&sign_output).contains(&private_key));
    assert!(!stderr_string(&sign_output).contains(&private_key));

    let raw_tx = std::fs::read_to_string(&signed_tx_path).expect("read signed tx file");
    assert!(raw_tx.starts_with("0x02"));
    assert!(raw_tx.len() > 10);
    assert!(!raw_tx.contains(&sender));

    #[cfg(unix)]
    {
        let mode = std::fs::metadata(&signed_tx_path)
            .expect("metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "signed tx file must be 0600");
    }
}

#[test]
fn parity_keystore_tx_sign_fails_with_wrong_password() {
    let temp = TempDir::new().expect("temp dir");
    let keystore_path = temp.path().join("wrong-credential.keystore");
    let good_password_file = write_password_file(temp.path(), TEST_PASSWORD);
    let wrong_password_file = write_password_file(temp.path(), "definitely-wrong-password");
    let out_path = temp.path().join("signed.tx");
    let private_key = random_private_key_hex();

    let import_output = run_import_private_key(
        &keystore_path,
        &good_password_file,
        "wrong-credential-label",
        &private_key,
    );
    assert!(
        import_output.status.success(),
        "{}",
        stderr_string(&import_output)
    );

    let output = run_tx_sign(
        &keystore_path,
        &wrong_password_file,
        TxSignArgs {
            by_label: Some("wrong-credential-label"),
            ..TxSignArgs::new(&out_path)
        },
    );

    assert!(
        !output.status.success(),
        "tx-sign should fail with wrong password"
    );
    let err = parse_error_json(&output.stderr);
    assert_eq!(err["code"].as_str(), Some("keystore_error"));
    let stderr = stderr_string(&output);
    assert!(!stderr.contains(&private_key));
    assert!(!stderr.contains("definitely-wrong-password"));
}

#[test]
fn parity_keystore_tx_sign_fails_with_missing_selector() {
    let temp = TempDir::new().expect("temp dir");
    let keystore_path = temp.path().join("missing-selector.keystore");
    let password_file = write_password_file(temp.path(), TEST_PASSWORD);
    let out_path = temp.path().join("signed.tx");
    let private_key = random_private_key_hex();

    let import_output = run_import_private_key(
        &keystore_path,
        &password_file,
        "missing-selector-label",
        &private_key,
    );
    assert!(
        import_output.status.success(),
        "{}",
        stderr_string(&import_output)
    );

    let output = run_tx_sign(&keystore_path, &password_file, TxSignArgs::new(&out_path));
    assert!(!output.status.success());
    let err = parse_error_json(&output.stderr);
    assert_eq!(err["code"].as_str(), Some("missing_argument"));
}

#[test]
fn parity_keystore_tx_sign_fails_with_ambiguous_label() {
    let temp = TempDir::new().expect("temp dir");
    let keystore_path = temp.path().join("ambiguous-label.keystore");
    let password_file = write_password_file(temp.path(), TEST_PASSWORD);
    let out_path = temp.path().join("signed.tx");

    let first_import = run_import_private_key(
        &keystore_path,
        &password_file,
        "duplicate-label",
        &random_private_key_hex(),
    );
    assert!(
        first_import.status.success(),
        "{}",
        stderr_string(&first_import)
    );

    let second_import = run_import_private_key(
        &keystore_path,
        &password_file,
        "duplicate-label",
        &random_private_key_hex(),
    );
    assert!(
        second_import.status.success(),
        "{}",
        stderr_string(&second_import)
    );

    let output = run_tx_sign(
        &keystore_path,
        &password_file,
        TxSignArgs {
            by_label: Some("duplicate-label"),
            ..TxSignArgs::new(&out_path)
        },
    );
    assert!(!output.status.success());
    let err = parse_error_json(&output.stderr);
    assert_eq!(err["code"].as_str(), Some("ambiguous_label"));
}

fn run_import_private_key(
    keystore_path: &Path,
    password_file: &Path,
    label: &str,
    private_key_hex: &str,
) -> Output {
    let artifact_root = test_artifact_root(keystore_path);
    ensure_fast_keystore_exists(keystore_path, password_file);
    let runtime_config = write_runtime_config(keystore_path, password_file);
    let mut cmd = Command::cargo_bin("mfm_cli").expect("binary exists");
    sanitize_machine_readable_cli_env(&mut cmd)
        .env("MFM_RUNTIME_CONFIG_FILE", &runtime_config)
        .env("MFM_ARTIFACT_ROOT", artifact_root)
        .args([
            "--output-format",
            "json",
            "keystore",
            "import",
            "--import-type",
            "privatekey",
            "--label",
            label,
            "--stdin",
        ])
        .write_stdin(private_key_hex)
        .output()
        .expect("execute import")
}

struct TxSignArgs<'a> {
    by_label: Option<&'a str>,
    id: Option<&'a str>,
    to: &'a str,
    value_wei: &'a str,
    chain_id: &'a str,
    nonce: &'a str,
    max_fee_per_gas: &'a str,
    max_priority_fee_per_gas: &'a str,
    gas_limit: &'a str,
    out: &'a Path,
}

impl<'a> TxSignArgs<'a> {
    fn new(out: &'a Path) -> Self {
        Self {
            by_label: None,
            id: None,
            to: "0x1111111111111111111111111111111111111111",
            value_wei: "1",
            chain_id: "1",
            nonce: "1",
            max_fee_per_gas: "1",
            max_priority_fee_per_gas: "1",
            gas_limit: "21000",
            out,
        }
    }
}

fn run_tx_sign(keystore_path: &Path, password_file: &Path, args: TxSignArgs<'_>) -> Output {
    let artifact_root = test_artifact_root(keystore_path);
    let runtime_config = write_runtime_config(keystore_path, password_file);
    let mut cmd = Command::cargo_bin("mfm_cli").expect("binary exists");
    sanitize_machine_readable_cli_env(&mut cmd)
        .env("MFM_RUNTIME_CONFIG_FILE", &runtime_config)
        .env("MFM_ARTIFACT_ROOT", artifact_root)
        .args([
            "--output-format",
            "json",
            "keystore",
            "tx-sign",
            "--to",
            args.to,
            "--value-wei",
            args.value_wei,
            "--chain-id",
            args.chain_id,
            "--nonce",
            args.nonce,
            "--max-fee-per-gas",
            args.max_fee_per_gas,
            "--max-priority-fee-per-gas",
            args.max_priority_fee_per_gas,
            "--gas-limit",
            args.gas_limit,
            "--out",
            args.out.to_str().expect("path"),
        ]);
    if let Some(label) = args.by_label {
        cmd.args(["--by-label", label]);
    }
    if let Some(id) = args.id {
        cmd.args(["--id", id]);
    }
    cmd.output().expect("execute tx-sign")
}

fn write_password_file(dir: &Path, password: &str) -> PathBuf {
    let path = dir.join(format!("password-{}.txt", uuid::Uuid::new_v4()));
    std::fs::write(&path, password).expect("write password file");
    path
}

fn ensure_fast_keystore_exists(path: &Path, password_file: &Path) {
    if path.exists() {
        return;
    }
    let password = std::fs::read_to_string(password_file).expect("read password file");
    let mut keystore = Keystore::new_with_config(path, KeystoreConfig::insecure_integration_test())
        .expect("create fast keystore");
    keystore
        .unlock(password.trim_end())
        .expect("unlock keystore");
}

fn write_runtime_config(keystore_path: &Path, password_file: &Path) -> PathBuf {
    let runtime_config = keystore_path.with_extension("runtime.toml");
    let config = format!(
        r#"
[keystores.default]
keystore_path = {keystore_path}
unlock_file = {password_file}
"#,
        keystore_path = toml_string(&keystore_path.display().to_string()),
        password_file = toml_string(&password_file.display().to_string()),
    );
    std::fs::write(&runtime_config, config).expect("write runtime config");
    runtime_config
}

fn toml_string(value: &str) -> String {
    serde_json::to_string(value).expect("toml string")
}

fn random_private_key_hex() -> String {
    let mut bytes: [u8; 32] = rand::random();
    if bytes.iter().all(|b| *b == 0) {
        bytes[31] = 1;
    }
    hex::encode(bytes)
}

fn parse_error_json(stderr: &[u8]) -> Value {
    let stderr = std::str::from_utf8(stderr).expect("stderr utf8");
    let trimmed = stderr.trim();
    let parsed = serde_json::from_str::<Value>(trimmed)
        .or_else(|_| {
            let start = trimmed.find('{').ok_or_else(|| {
                serde_json::Error::io(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "missing json object start",
                ))
            })?;
            let end = trimmed.rfind('}').ok_or_else(|| {
                serde_json::Error::io(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "missing json object end",
                ))
            })?;
            serde_json::from_str::<Value>(&trimmed[start..=end])
        })
        .unwrap_or_else(|_| panic!("stderr must contain valid json: {stderr}"));
    assert_eq!(parsed["status"], "error");
    parsed["error"].clone()
}

fn stdout_string(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).expect("stdout utf8")
}

fn stderr_string(output: &Output) -> String {
    String::from_utf8(output.stderr.clone()).expect("stderr utf8")
}

fn normalize_address(address: &str) -> String {
    let trimmed = address.trim();
    let rest = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
        .expect("0x-prefixed address");
    assert_eq!(rest.len(), 40, "address must be 20-byte hex");
    assert!(rest.chars().all(|c| c.is_ascii_hexdigit()));
    format!("0x{}", rest.to_ascii_lowercase())
}

fn test_artifact_root(path: &Path) -> PathBuf {
    path.parent()
        .unwrap_or_else(|| Path::new("."))
        .join("run-artifacts")
}

fn sanitize_machine_readable_cli_env(cmd: &mut Command) -> &mut Command {
    // Keep JSON response channels deterministic for parity tests even when the parent
    // environment enables tracing (e.g. LOG_LEVEL/RUST_LOG in CI debug runs).
    cmd.env_remove("LOG_LEVEL")
        .env_remove("RUST_LOG")
        .env_remove("LOG_FORMAT")
        .env_remove("LOG_SPAN_EVENTS")
}
