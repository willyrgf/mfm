#![cfg(feature = "parity-tests")]

use assert_cmd::Command;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Output;
use std::time::Duration;
use tempfile::TempDir;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

const TEST_PASSWORD: &str = "parity-test-password";

#[tokio::test]
async fn parity_keystore_cli_tx_sign_and_send_on_reth() {
    let rpc_url = std::env::var("MFM_EVM_RPC_URL").expect("MFM_EVM_RPC_URL is required");
    let temp = TempDir::new().expect("temp dir");
    let keystore_path = temp.path().join("parity.keystore");
    let password_file = write_password_file(temp.path(), TEST_PASSWORD);
    let signed_tx_path = temp.path().join("signed.tx");

    let chain_id = rpc_hex_u64(&rpc_url, "eth_chainId", serde_json::json!([])).await;
    let accounts = rpc_call(&rpc_url, "eth_accounts", serde_json::json!([])).await;
    let funder = accounts
        .as_array()
        .and_then(|arr| arr.first())
        .and_then(Value::as_str)
        .map(normalize_address)
        .expect("reth must expose account[0]");
    let recipient = accounts
        .as_array()
        .and_then(|arr| arr.get(1))
        .and_then(Value::as_str)
        .map(normalize_address)
        .unwrap_or_else(|| funder.clone());

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
    let import_json = parse_success_json(&import_output.stdout);
    let key_id = import_json["id"].as_str().expect("import id").to_string();
    let sender = import_json["address"]
        .as_str()
        .map(normalize_address)
        .expect("imported address");

    let funding_gas_price = rpc_call(&rpc_url, "eth_gasPrice", serde_json::json!([]))
        .await
        .as_str()
        .map(str::to_owned)
        .expect("funding gas price");
    let funding_hash = rpc_call(
        &rpc_url,
        "eth_sendTransaction",
        serde_json::json!([{
            "from": funder.clone(),
            "to": sender.clone(),
            "value": "0xde0b6b3a7640000",
            "gas": "0x5208",
            "gasPrice": funding_gas_price,
        }]),
    )
    .await
    .as_str()
    .map(str::to_owned)
    .expect("funding tx hash");
    let funding_receipt = wait_for_receipt(&rpc_url, &funding_hash, 120).await;
    assert_eq!(funding_receipt["status"], "0x1");

    let recipient_before = rpc_hex_u128(
        &rpc_url,
        "eth_getBalance",
        serde_json::json!([recipient.clone(), "latest"]),
    )
    .await;
    let sender_nonce = rpc_hex_u64(
        &rpc_url,
        "eth_getTransactionCount",
        serde_json::json!([sender.clone(), "pending"]),
    )
    .await;
    let gas_price = rpc_hex_u128(&rpc_url, "eth_gasPrice", serde_json::json!([])).await;
    let priority_fee =
        rpc_hex_u128_optional(&rpc_url, "eth_maxPriorityFeePerGas", serde_json::json!([]))
            .await
            .unwrap_or_else(|| std::cmp::max(gas_price / 10, 1));
    let max_fee = std::cmp::max(gas_price.saturating_mul(2), priority_fee.saturating_add(1));
    let send_value: u128 = 1_000_000_000_000_000;

    let sign_output = run_tx_sign(
        &keystore_path,
        &password_file,
        &key_id,
        &recipient,
        send_value,
        chain_id,
        sender_nonce,
        max_fee,
        priority_fee,
        21_000,
        &signed_tx_path,
    );
    assert!(
        sign_output.status.success(),
        "{}",
        stderr_string(&sign_output)
    );
    let sign_json = parse_success_json(&sign_output.stdout);
    assert_eq!(
        normalize_address(sign_json["from"].as_str().expect("from")),
        sender
    );
    assert_eq!(
        normalize_address(sign_json["to"].as_str().expect("to")),
        recipient
    );
    assert_eq!(sign_json["tx_type"].as_str(), Some("0x2"));
    assert_eq!(sign_json["chain_id"].as_u64(), Some(chain_id));
    assert_eq!(sign_json["nonce"].as_u64(), Some(sender_nonce));
    assert!(sign_json["payload_hash"]
        .as_str()
        .is_some_and(|v| v.starts_with("0x")));
    assert!(!stdout_string(&sign_output).contains("raw_tx_hex"));

    let raw_tx = std::fs::read_to_string(&signed_tx_path).expect("read signed tx file");
    assert!(raw_tx.starts_with("0x02"));
    assert!(raw_tx.len() > 10);

    #[cfg(unix)]
    {
        let mode = std::fs::metadata(&signed_tx_path)
            .expect("metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "signed tx file must be 0600");
    }

    let send_output = run_tx_send_raw(&signed_tx_path, &rpc_url);
    assert!(
        send_output.status.success(),
        "{}",
        stderr_string(&send_output)
    );
    let send_json = parse_success_json(&send_output.stdout);
    let sent_hash = send_json["tx_hash"]
        .as_str()
        .map(str::to_owned)
        .expect("tx hash");
    assert!(sent_hash.starts_with("0x"));
    assert_eq!(
        send_json["rpc_url_host"].as_str().map(str::to_owned),
        Some(url_host_with_port(&rpc_url))
    );
    assert!(send_json["submitted_at"].as_str().is_some());

    let send_receipt = wait_for_receipt(&rpc_url, &sent_hash, 120).await;
    assert_eq!(send_receipt["status"], "0x1");
    assert_eq!(
        normalize_address(send_receipt["from"].as_str().expect("receipt from")),
        sender
    );
    assert_eq!(
        normalize_address(send_receipt["to"].as_str().expect("receipt to")),
        recipient
    );

    let tx_by_hash = rpc_call(
        &rpc_url,
        "eth_getTransactionByHash",
        serde_json::json!([sent_hash]),
    )
    .await;
    assert_eq!(tx_by_hash["type"].as_str(), Some("0x2"));

    let recipient_after = rpc_hex_u128(
        &rpc_url,
        "eth_getBalance",
        serde_json::json!([recipient.clone(), "latest"]),
    )
    .await;
    assert!(
        recipient_after > recipient_before,
        "recipient balance must increase"
    );

    let sign_stdout = stdout_string(&sign_output);
    let send_stdout = stdout_string(&send_output);
    assert!(
        !sign_stdout.contains(&private_key),
        "private key leaked in sign output"
    );
    assert!(
        !send_stdout.contains(&raw_tx),
        "raw tx leaked in send output"
    );
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

    let output = run_tx_sign_with_selector(
        &keystore_path,
        &wrong_password_file,
        Some("wrong-credential-label"),
        None,
        "0x1111111111111111111111111111111111111111",
        "1",
        "1",
        "1",
        "1",
        "1",
        "21000",
        &out_path,
    );

    assert!(
        !output.status.success(),
        "tx-sign should fail with wrong password"
    );
    let err = parse_error_json(&output.stderr);
    assert_eq!(err["code"].as_str(), Some("KeystoreError"));
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

    let output = run_tx_sign_with_selector(
        &keystore_path,
        &password_file,
        None,
        None,
        "0x1111111111111111111111111111111111111111",
        "1",
        "1",
        "1",
        "1",
        "1",
        "21000",
        &out_path,
    );
    assert!(!output.status.success());
    let err = parse_error_json(&output.stderr);
    assert_eq!(err["code"].as_str(), Some("MissingArgument"));
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

    let output = run_tx_sign_with_selector(
        &keystore_path,
        &password_file,
        Some("duplicate-label"),
        None,
        "0x1111111111111111111111111111111111111111",
        "1",
        "1",
        "1",
        "1",
        "1",
        "21000",
        &out_path,
    );
    assert!(!output.status.success());
    let err = parse_error_json(&output.stderr);
    assert_eq!(err["code"].as_str(), Some("AmbiguousLabel"));
}

#[test]
fn parity_keystore_tx_send_raw_fails_with_malformed_input_file() {
    let temp = TempDir::new().expect("temp dir");
    let raw_file = temp.path().join("invalid.raw");
    std::fs::write(&raw_file, "not-hex").expect("write malformed payload");

    let output = run_tx_send_raw(&raw_file, "http://127.0.0.1:8545");
    assert!(!output.status.success());
    let err = parse_error_json(&output.stderr);
    assert_eq!(err["code"].as_str(), Some("InvalidRawTransaction"));
}

#[test]
fn parity_keystore_tx_send_raw_fails_without_rpc_url() {
    let temp = TempDir::new().expect("temp dir");
    let raw_file = temp.path().join("valid.raw");
    std::fs::write(&raw_file, "0x0201").expect("write payload");

    let mut cmd = Command::cargo_bin("mfm_cli").expect("binary exists");
    let output = sanitize_machine_readable_cli_env(&mut cmd)
        .env_remove("MFM_EVM_RPC_URL")
        .args([
            "--output-format",
            "json",
            "keystore",
            "tx-send-raw",
            "--in",
            raw_file.to_str().expect("path"),
        ])
        .output()
        .expect("execute tx-send-raw without rpc");

    assert!(!output.status.success());
    let err = parse_error_json(&output.stderr);
    assert_eq!(err["code"].as_str(), Some("MissingArgument"));
}

fn run_import_private_key(
    keystore_path: &Path,
    password_file: &Path,
    label: &str,
    private_key_hex: &str,
) -> Output {
    let mut cmd = Command::cargo_bin("mfm_cli").expect("binary exists");
    sanitize_machine_readable_cli_env(&mut cmd)
        .env("MFM_INTEGRATION_TEST", "1")
        .env("MFM_KEYSTORE_PASSWORD_FILE", password_file)
        .args([
            "--output-format",
            "json",
            "keystore",
            "import",
            "--import-type",
            "privatekey",
            "--label",
            label,
            "--keystore",
            keystore_path.to_str().expect("path"),
            "--stdin",
        ])
        .write_stdin(private_key_hex)
        .output()
        .expect("execute import")
}

#[allow(clippy::too_many_arguments)]
fn run_tx_sign(
    keystore_path: &Path,
    password_file: &Path,
    key_id: &str,
    to: &str,
    value_wei: u128,
    chain_id: u64,
    nonce: u64,
    max_fee_per_gas: u128,
    max_priority_fee_per_gas: u128,
    gas_limit: u64,
    out: &Path,
) -> Output {
    run_tx_sign_with_selector(
        keystore_path,
        password_file,
        None,
        Some(key_id),
        to,
        &value_wei.to_string(),
        &chain_id.to_string(),
        &nonce.to_string(),
        &max_fee_per_gas.to_string(),
        &max_priority_fee_per_gas.to_string(),
        &gas_limit.to_string(),
        out,
    )
}

#[allow(clippy::too_many_arguments)]
fn run_tx_sign_with_selector(
    keystore_path: &Path,
    password_file: &Path,
    by_label: Option<&str>,
    id: Option<&str>,
    to: &str,
    value_wei: &str,
    chain_id: &str,
    nonce: &str,
    max_fee_per_gas: &str,
    max_priority_fee_per_gas: &str,
    gas_limit: &str,
    out: &Path,
) -> Output {
    let mut cmd = Command::cargo_bin("mfm_cli").expect("binary exists");
    sanitize_machine_readable_cli_env(&mut cmd)
        .env("MFM_INTEGRATION_TEST", "1")
        .env("MFM_KEYSTORE_PASSWORD_FILE", password_file)
        .args([
            "--output-format",
            "json",
            "keystore",
            "tx-sign",
            "--to",
            to,
            "--value-wei",
            value_wei,
            "--chain-id",
            chain_id,
            "--nonce",
            nonce,
            "--max-fee-per-gas",
            max_fee_per_gas,
            "--max-priority-fee-per-gas",
            max_priority_fee_per_gas,
            "--gas-limit",
            gas_limit,
            "--out",
            out.to_str().expect("path"),
            "--keystore",
            keystore_path.to_str().expect("path"),
        ]);
    if let Some(label) = by_label {
        cmd.args(["--by-label", label]);
    }
    if let Some(id) = id {
        cmd.args(["--id", id]);
    }
    cmd.output().expect("execute tx-sign")
}

fn run_tx_send_raw(input_path: &Path, rpc_url: &str) -> Output {
    let mut cmd = Command::cargo_bin("mfm_cli").expect("binary exists");
    sanitize_machine_readable_cli_env(&mut cmd)
        .args([
            "--output-format",
            "json",
            "keystore",
            "tx-send-raw",
            "--rpc-url",
            rpc_url,
            "--in",
            input_path.to_str().expect("path"),
        ])
        .output()
        .expect("execute tx-send-raw")
}

async fn wait_for_receipt(rpc_url: &str, tx_hash: &str, max_polls: usize) -> Value {
    for _ in 0..max_polls {
        let receipt = rpc_call(
            rpc_url,
            "eth_getTransactionReceipt",
            serde_json::json!([tx_hash]),
        )
        .await;
        if !receipt.is_null() {
            return receipt;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    panic!("timed out waiting for transaction receipt");
}

async fn rpc_call(rpc_url: &str, method: &str, params: Value) -> Value {
    let payload = rpc_request(rpc_url, method, params).await;
    if let Some(error) = payload.get("error") {
        panic!(
            "rpc call {} failed: {}",
            method,
            serde_json::to_string(error).expect("rpc error json")
        );
    }
    payload
        .get("result")
        .cloned()
        .expect("rpc result must exist")
}

async fn rpc_hex_u64(rpc_url: &str, method: &str, params: Value) -> u64 {
    let value = rpc_call(rpc_url, method, params).await;
    parse_hex_u64(value.as_str().expect("hex result"))
}

async fn rpc_hex_u128(rpc_url: &str, method: &str, params: Value) -> u128 {
    let value = rpc_call(rpc_url, method, params).await;
    parse_hex_u128(value.as_str().expect("hex result"))
}

async fn rpc_hex_u128_optional(rpc_url: &str, method: &str, params: Value) -> Option<u128> {
    let payload = rpc_request(rpc_url, method, params).await;
    if payload.get("error").is_some() {
        return None;
    }
    let raw = payload.get("result")?.as_str()?;
    Some(parse_hex_u128(raw))
}

async fn rpc_request(rpc_url: &str, method: &str, params: Value) -> Value {
    let client = reqwest::Client::new();
    let response = client
        .post(rpc_url)
        .json(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": method,
            "params": params,
        }))
        .send()
        .await
        .expect("rpc send");
    assert!(response.status().is_success(), "rpc status must be success");
    response.json::<Value>().await.expect("rpc json")
}

fn write_password_file(dir: &Path, password: &str) -> PathBuf {
    let path = dir.join(format!("password-{}.txt", uuid::Uuid::new_v4()));
    std::fs::write(&path, password).expect("write password file");
    path
}

fn random_private_key_hex() -> String {
    let mut bytes: [u8; 32] = rand::random();
    if bytes.iter().all(|b| *b == 0) {
        bytes[31] = 1;
    }
    hex::encode(bytes)
}

fn parse_success_json(stdout: &[u8]) -> Value {
    let parsed: Value = serde_json::from_slice(stdout).expect("stdout must be valid json");
    assert_eq!(parsed["status"], "success");
    parsed["data"].clone()
}

fn parse_error_json(stderr: &[u8]) -> Value {
    let parsed: Value = serde_json::from_slice(stderr).expect("stderr must be valid json");
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

fn parse_hex_u64(raw: &str) -> u64 {
    let hex = raw
        .strip_prefix("0x")
        .or_else(|| raw.strip_prefix("0X"))
        .expect("hex prefix");
    if hex.is_empty() {
        return 0;
    }
    u64::from_str_radix(hex, 16).expect("u64 hex parse")
}

fn sanitize_machine_readable_cli_env(cmd: &mut Command) -> &mut Command {
    // Keep JSON response channels deterministic for parity tests even when the parent
    // environment enables tracing (e.g. RUST_LOG/MFM_LOG in CI debug runs).
    cmd.env_remove("MFM_LOG")
        .env_remove("RUST_LOG")
        .env_remove("MFM_LOG_FORMAT")
        .env_remove("MFM_LOG_SPAN_EVENTS")
}

fn parse_hex_u128(raw: &str) -> u128 {
    let hex = raw
        .strip_prefix("0x")
        .or_else(|| raw.strip_prefix("0X"))
        .expect("hex prefix");
    if hex.is_empty() {
        return 0;
    }
    u128::from_str_radix(hex, 16).expect("u128 hex parse")
}

fn url_host_with_port(url: &str) -> String {
    let parsed = reqwest::Url::parse(url).expect("valid url");
    let host = parsed.host_str().expect("host");
    match parsed.port() {
        Some(port) => format!("{host}:{port}"),
        None => host.to_string(),
    }
}
