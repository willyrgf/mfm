#![allow(clippy::disallowed_methods, clippy::disallowed_types)]
#![allow(clippy::needless_borrows_for_generic_args)]

use assert_cmd::Command;
use mfm_app::{initialize_insecure_keystore_for_test, SecretInput};
use predicates::prelude::*;
use tempfile::TempDir;

const PASSWORD: &str = "config_password_123";
const PRIVATE_KEY: &str = "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef";

#[test]
fn runtime_config_selection_requires_an_explicit_path() {
    let mut list_cmd = Command::cargo_bin("mfm_cli").unwrap();
    list_cmd.args(["keystore", "list"]);

    list_cmd
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "provide --keystore or --runtime-config",
        ))
        .stdout(predicate::str::contains(PASSWORD).not())
        .stderr(predicate::str::contains(PASSWORD).not());

    let fixture = KeystoreFixture::new("runtime-arg-test");
    let mut list_cmd = Command::cargo_bin("mfm_cli").unwrap();
    list_cmd.args([
        "keystore",
        "list",
        "--runtime-config",
        fixture.runtime_config.to_str().unwrap(),
    ]);

    list_cmd
        .assert()
        .success()
        .stdout(predicate::str::contains("runtime-arg-test"))
        .stdout(predicate::str::contains(PASSWORD).not())
        .stderr(predicate::str::contains(PASSWORD).not());
}

#[test]
fn runtime_config_empty_unlock_file_fails() {
    let fixture = KeystoreFixture::new("empty-unlock-file-test");
    std::fs::write(&fixture.password_file, "").expect("empty password file");

    let mut list_cmd = Command::cargo_bin("mfm_cli").unwrap();
    list_cmd.args([
        "keystore",
        "list",
        "--runtime-config",
        fixture.runtime_config.to_str().unwrap(),
    ]);

    list_cmd
        .assert()
        .failure()
        .stderr(predicate::str::contains("credential file was empty"))
        .stdout(predicate::str::contains(PASSWORD).not())
        .stderr(predicate::str::contains(PASSWORD).not());
}

struct KeystoreFixture {
    _dir: TempDir,
    password_file: std::path::PathBuf,
    runtime_config: std::path::PathBuf,
}

impl KeystoreFixture {
    fn new(label: &str) -> Self {
        let dir = TempDir::new().expect("temp dir");
        let keystore_path = dir.path().join("test.keystore");
        let password_file = dir.path().join("password.txt");
        let runtime_config = dir.path().join("runtime.toml");
        std::fs::write(&password_file, format!("{PASSWORD}\n")).expect("password file");

        initialize_insecure_keystore_for_test(
            keystore_path.clone(),
            SecretInput::new(PASSWORD.to_owned()),
            vec![(
                Some(label.to_owned()),
                SecretInput::new(PRIVATE_KEY.to_owned()),
            )],
        )
        .expect("keystore fixture");

        write_runtime_config(&runtime_config, &keystore_path, &password_file);
        Self {
            _dir: dir,
            password_file,
            runtime_config,
        }
    }
}

fn write_runtime_config(
    runtime_config: &std::path::Path,
    keystore_path: &std::path::Path,
    password_file: &std::path::Path,
) {
    let config = format!(
        r#"
[keystores.default]
keystore_path = {{ direct = {keystore_path} }}
unlock_file_path = {{ direct = {password_file} }}
"#,
        keystore_path = toml_string(&keystore_path.display().to_string()),
        password_file = toml_string(&password_file.display().to_string()),
    );
    std::fs::write(runtime_config, config).expect("runtime config");
}

fn toml_string(value: &str) -> String {
    serde_json::to_string(value).expect("toml string")
}
