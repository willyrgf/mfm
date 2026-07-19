#![allow(clippy::disallowed_methods)]
#![cfg(feature = "parity-tests")]

use alloy_primitives::{keccak256, Address, Bytes, TxKind, U256};
use assert_cmd::Command;
use mfm_core::keystore::{Keystore, KeystoreConfig};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Output;
use std::str::FromStr;
use tempfile::TempDir;

#[path = "support/mod.rs"]
mod support;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

const TEST_PASSWORD: &str = "parity-test-password";
const SIGNER_REF: &str = "parity-sender";

#[test]
fn parity_keystore_cli_uses_canonical_eip1559_signing_service() {
    let temp = TempDir::new().expect("temp dir");
    let keystore_path = temp.path().join("parity.keystore");
    let password_file = write_password_file(temp.path(), TEST_PASSWORD);
    let signed_tx_path = temp.path().join("signed.tx");
    let recipient = "0x1111111111111111111111111111111111111111";
    let private_key = random_private_key_hex();

    let (key_id, sender) = import_sender(
        &keystore_path,
        &password_file,
        "parity-cli-sender",
        &private_key,
    );
    let args = TxSignArgs {
        signer_ref: SIGNER_REF,
        expected_from: &sender,
        to: recipient,
        value_wei: "1000000000000000",
        chain_id: "31337",
        nonce: "0",
        max_fee_per_gas: "2000000000",
        max_priority_fee_per_gas: "1000000000",
        gas_limit: "21000",
        out: &signed_tx_path,
    };
    let sign_output = run_tx_sign(&keystore_path, &password_file, SIGNER_REF, &key_id, args);
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
    assert_eq!(sign_json["chain_id"].as_str(), Some("31337"));
    assert_eq!(sign_json["nonce"].as_str(), Some("0"));
    assert!(sign_json.get("tx_type").is_none());
    assert!(sign_json.get("payload_hash").is_none());
    let signing_digest = sign_json["signing_digest"]
        .as_str()
        .expect("signing digest");
    let transaction_hash = sign_json["transaction_hash"]
        .as_str()
        .expect("transaction hash");
    assert_ne!(signing_digest, transaction_hash);
    assert!(sign_json.get("out_path").is_none());

    let unsigned = mfm_evm_signing::UnsignedEip1559Envelope::new(
        U256::from(31_337),
        U256::ZERO,
        U256::from(1_000_000_000_u64),
        U256::from(2_000_000_000_u64),
        U256::from(21_000),
        TxKind::Call(Address::from_str(recipient).expect("recipient")),
        U256::from(1_000_000_000_000_000_u64),
        Default::default(),
        Bytes::new(),
    )
    .expect("canonical unsigned envelope");
    assert_eq!(signing_digest, format!("{:?}", unsigned.signing_digest()));

    let raw_tx_hex = std::fs::read_to_string(&signed_tx_path).expect("signed tx file");
    let raw_tx = hex::decode(raw_tx_hex.strip_prefix("0x").expect("hex prefix")).expect("raw tx");
    assert!(raw_tx.starts_with(&[0x02]));
    assert_eq!(transaction_hash, format!("{:?}", keccak256(&raw_tx)));
    assert!(!stdout_string(&sign_output).contains(&raw_tx_hex));
    assert!(!stdout_string(&sign_output).contains(&signed_tx_path.display().to_string()));
    assert!(!stdout_string(&sign_output).contains(&private_key));
    assert!(!stderr_string(&sign_output).contains(&private_key));

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
fn parity_keystore_tx_sign_fails_closed_with_wrong_password() {
    let temp = TempDir::new().expect("temp dir");
    let keystore_path = temp.path().join("wrong-credential.keystore");
    let good_password_file = write_password_file(temp.path(), TEST_PASSWORD);
    let wrong_password_file = write_password_file(temp.path(), "definitely-wrong-password");
    let out_path = temp.path().join("signed.tx");
    let private_key = random_private_key_hex();
    let (key_id, sender) = import_sender(
        &keystore_path,
        &good_password_file,
        "wrong-credential-label",
        &private_key,
    );

    let output = run_tx_sign(
        &keystore_path,
        &wrong_password_file,
        SIGNER_REF,
        &key_id,
        TxSignArgs::new(&out_path, SIGNER_REF, &sender),
    );

    assert!(!output.status.success());
    let error = parse_error_json(&output.stderr);
    assert_eq!(error["code"].as_str(), Some("SignerUnavailable"));
    let rendered = stderr_string(&output);
    assert!(!rendered.contains(&private_key));
    assert!(!rendered.contains("definitely-wrong-password"));
    assert!(!rendered.contains(&wrong_password_file.display().to_string()));
    assert!(!out_path.exists());
}

#[test]
fn parity_keystore_tx_sign_resolves_only_the_requested_signer_ref() {
    let temp = TempDir::new().expect("temp dir");
    let keystore_path = temp.path().join("missing-signer.keystore");
    let password_file = write_password_file(temp.path(), TEST_PASSWORD);
    let out_path = temp.path().join("signed.tx");
    let (key_id, sender) = import_sender(
        &keystore_path,
        &password_file,
        "configured-sender",
        &random_private_key_hex(),
    );

    let output = run_tx_sign(
        &keystore_path,
        &password_file,
        SIGNER_REF,
        &key_id,
        TxSignArgs::new(&out_path, "missing-signer", &sender),
    );

    assert!(!output.status.success());
    assert_eq!(
        parse_error_json(&output.stderr)["code"].as_str(),
        Some("SignerNotConfigured")
    );
    assert!(!out_path.exists());
}

#[test]
fn parity_keystore_tx_sign_rejects_sender_binding_mismatch() {
    let temp = TempDir::new().expect("temp dir");
    let keystore_path = temp.path().join("wrong-sender.keystore");
    let password_file = write_password_file(temp.path(), TEST_PASSWORD);
    let out_path = temp.path().join("signed.tx");
    let (key_id, _) = import_sender(
        &keystore_path,
        &password_file,
        "configured-sender",
        &random_private_key_hex(),
    );

    let output = run_tx_sign(
        &keystore_path,
        &password_file,
        SIGNER_REF,
        &key_id,
        TxSignArgs::new(
            &out_path,
            SIGNER_REF,
            "0x2222222222222222222222222222222222222222",
        ),
    );

    assert!(!output.status.success());
    assert_eq!(
        parse_error_json(&output.stderr)["code"].as_str(),
        Some("SignerIdentityMismatch")
    );
    assert!(!out_path.exists());
}

fn import_sender(
    keystore_path: &Path,
    password_file: &Path,
    label: &str,
    private_key_hex: &str,
) -> (String, String) {
    let output = run_import_private_key(keystore_path, password_file, label, private_key_hex);
    assert!(output.status.success(), "{}", stderr_string(&output));
    let json = support::parse_success_json(&output.stdout);
    (
        json["id"].as_str().expect("import id").to_owned(),
        normalize_address(json["address"].as_str().expect("address")),
    )
}

fn run_import_private_key(
    keystore_path: &Path,
    password_file: &Path,
    label: &str,
    private_key_hex: &str,
) -> Output {
    let artifact_root = test_artifact_root(keystore_path);
    ensure_fast_keystore_exists(keystore_path, password_file);
    let runtime_config = write_runtime_config(keystore_path, password_file, None);
    let mut command = Command::cargo_bin("mfm_cli").expect("binary exists");
    support::sanitize_machine_readable_cli_env(&mut command)
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
    signer_ref: &'a str,
    expected_from: &'a str,
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
    fn new(out: &'a Path, signer_ref: &'a str, expected_from: &'a str) -> Self {
        Self {
            signer_ref,
            expected_from,
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

fn run_tx_sign(
    keystore_path: &Path,
    password_file: &Path,
    configured_signer_ref: &str,
    entry_id: &str,
    args: TxSignArgs<'_>,
) -> Output {
    let artifact_root = test_artifact_root(keystore_path);
    let runtime_config = write_runtime_config(
        keystore_path,
        password_file,
        Some((configured_signer_ref, entry_id)),
    );
    let mut command = Command::cargo_bin("mfm_cli").expect("binary exists");
    support::sanitize_machine_readable_cli_env(&mut command)
        .env("MFM_RUNTIME_CONFIG_FILE", &runtime_config)
        .env("MFM_ARTIFACT_ROOT", artifact_root)
        .args([
            "--output-format",
            "json",
            "keystore",
            "tx-sign",
            "--signer-ref",
            args.signer_ref,
            "--from",
            args.expected_from,
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
        ])
        .output()
        .expect("execute tx-sign")
}

fn write_password_file(directory: &Path, password: &str) -> PathBuf {
    let path = directory.join(format!("password-{}.txt", uuid::Uuid::new_v4()));
    std::fs::write(&path, password).expect("password file");
    path
}

fn ensure_fast_keystore_exists(path: &Path, password_file: &Path) {
    if path.exists() {
        return;
    }
    let password = std::fs::read_to_string(password_file).expect("password file");
    let mut keystore = Keystore::new_with_config(path, KeystoreConfig::insecure_integration_test())
        .expect("fast keystore");
    keystore.unlock(password.trim_end()).expect("unlock");
}

fn write_runtime_config(
    keystore_path: &Path,
    password_file: &Path,
    signer: Option<(&str, &str)>,
) -> PathBuf {
    let runtime_config = keystore_path.with_extension("runtime.toml");
    let signer = signer
        .map(|(signer_ref, entry_id)| {
            format!(
                r#"
[signers.{signer_ref}]
provider = "keystore"
keystore_ref = "default"
entry_id = {entry_id}
"#,
                entry_id = toml_string(entry_id),
            )
        })
        .unwrap_or_default();
    let config = format!(
        r#"
[keystores.default]
keystore_path = {keystore_path}
unlock_file = {password_file}
{signer}
"#,
        keystore_path = toml_string(&keystore_path.display().to_string()),
        password_file = toml_string(&password_file.display().to_string()),
    );
    std::fs::write(&runtime_config, config).expect("runtime config");
    runtime_config
}

fn toml_string(value: &str) -> String {
    serde_json::to_string(value).expect("toml string")
}

fn random_private_key_hex() -> String {
    let mut bytes: [u8; 32] = rand::random();
    if bytes.iter().all(|byte| *byte == 0) {
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
        .expect("0x address");
    assert_eq!(rest.len(), 40);
    assert!(rest.chars().all(|character| character.is_ascii_hexdigit()));
    format!("0x{}", rest.to_ascii_lowercase())
}

fn test_artifact_root(path: &Path) -> PathBuf {
    path.parent()
        .unwrap_or_else(|| Path::new("."))
        .join("run-artifacts")
}
