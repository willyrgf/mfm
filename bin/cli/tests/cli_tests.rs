#![allow(clippy::needless_borrows_for_generic_args)]

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

fn create_test_keystore() -> (TempDir, std::path::PathBuf) {
    let temp_dir = TempDir::new().expect("Failed to create temp directory");
    let keystore_path = temp_dir.path().join("test_keystore");
    (temp_dir, keystore_path)
}

#[test]
fn test_cli_help() {
    let mut cmd = Command::cargo_bin("mfm_cli").unwrap();
    cmd.arg("--help");

    cmd.assert()
        .success()
        .stdout(predicate::str::contains("MFM - On-chain operations tool"))
        .stdout(predicate::str::contains("keystore"))
        .stdout(predicate::str::contains("ops"))
        .stdout(predicate::str::contains("setup").not())
        .stdout(predicate::str::contains("run"))
        .stdout(predicate::str::contains("facts").not());
}

#[test]
fn ops_list_uses_the_production_connection_surface_and_fails_closed_standalone() {
    let mut help = Command::cargo_bin("mfm_cli").unwrap();
    help.args(["ops", "--help"]);
    help.assert()
        .success()
        .stdout(predicate::str::contains(
            "Public entry-point operation discovery",
        ))
        .stdout(predicate::str::contains("list"));

    let mut list_help = Command::cargo_bin("mfm_cli").unwrap();
    list_help.args(["ops", "list", "--help"]);
    list_help
        .assert()
        .success()
        .stdout(predicate::str::contains("--database-url"))
        .stdout(predicate::str::contains("--runtime-config"));

    let mut list = Command::cargo_bin("mfm_cli").unwrap();
    list.args(["ops", "list"])
        .assert()
        .failure()
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains(
            "AuthoritativeWriterFenceUnavailable",
        ));
}

#[test]
fn run_help_has_only_the_recoverability_v1_commands() {
    let mut cmd = Command::cargo_bin("mfm_cli").unwrap();
    let output = cmd
        .args(["run", "--help"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let rendered = String::from_utf8(output).expect("run help is UTF-8");
    for command in [
        "admit", "drive", "show", "replay", "trace", "audit", "export",
    ] {
        assert!(rendered.contains(command), "missing {command}: {rendered}");
    }
    for removed in [
        "start",
        "resume",
        "status",
        "stream",
        "manual-resolution",
        "public-output",
        "list",
        "watch",
    ] {
        assert!(
            !rendered
                .lines()
                .any(|line| line.trim_start().starts_with(removed)),
            "removed run command {removed} survived: {rendered}"
        );
    }
}

#[test]
fn run_admit_help_describes_generic_entry_point_target() {
    let mut cmd = Command::cargo_bin("mfm_cli").unwrap();
    let output = cmd
        .args(["run", "admit", "--help"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let rendered = String::from_utf8(output).expect("run admit help is UTF-8");
    assert!(
        rendered.contains("Configured entry-point target"),
        "generic target help is missing: {rendered}"
    );
    assert!(
        rendered.contains("--caller-submission-token"),
        "EVM caller-token help is missing: {rendered}"
    );
    assert!(
        !rendered.to_ascii_lowercase().contains("portfolio target"),
        "portfolio-only target help survived: {rendered}"
    );
}

#[test]
fn test_keystore_help_surfaces() {
    for (args, expected) in [
        (
            vec!["keystore", "--help"],
            vec!["Keystore management operations", "import", "delete", "list"],
        ),
        (
            vec!["keystore", "import", "--help"],
            vec![
                "Import a private key or mnemonic",
                "--import-type",
                "--passphrase-file",
                "--passphrase-prompt",
                "privatekey",
                "mnemonic",
            ],
        ),
        (
            vec!["keystore", "list", "--help"],
            vec!["List keys in the keystore", "--show-addresses"],
        ),
        (
            vec!["keystore", "delete", "--help"],
            vec!["Delete a key from the keystore", "--by-label", "--yes"],
        ),
    ] {
        let mut cmd = Command::cargo_bin("mfm_cli").unwrap();
        cmd.args(args);

        let output = cmd.assert().success().get_output().stdout.clone();
        let rendered = String::from_utf8(output).expect("help output is UTF-8");
        for expected in expected {
            assert!(
                rendered.contains(expected),
                "help output missing {expected:?}: {rendered}"
            );
        }
    }

    let mut command = Command::cargo_bin("mfm_cli").unwrap();
    command
        .args(["keystore", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("tx-sign").not());

    let mut removed = Command::cargo_bin("mfm_cli").unwrap();
    removed
        .args(["keystore", "tx-sign"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("unrecognized subcommand"));
}

#[test]
fn test_list_missing_keystore_error_cases() {
    for args in [
        vec!["keystore", "list"],
        vec!["--output-format", "json", "keystore", "list"],
    ] {
        let (_temp_dir, keystore_path) = create_test_keystore();

        let mut cmd = Command::cargo_bin("mfm_cli").unwrap();
        cmd.args(args)
            .args(&["--keystore", keystore_path.to_str().unwrap()]);

        cmd.assert()
            .failure()
            .stderr(predicate::str::contains("Keystore not found"));
    }
}

#[test]
fn test_import_invalid_private_key_format() {
    let (_temp_dir, keystore_path) = create_test_keystore();

    let mut cmd = Command::cargo_bin("mfm_cli").unwrap();
    cmd.args(&[
        "keystore",
        "import",
        "--import-type",
        "privatekey",
        "--keystore",
        keystore_path.to_str().unwrap(),
        "--stdin",
    ]);
    cmd.write_stdin("invalid_key");

    // This will fail during keystore creation since we need interactive password input
    // In a real scenario, we'd need to mock the password input
    cmd.assert().failure();
}

#[test]
fn test_import_missing_type() {
    let (_temp_dir, keystore_path) = create_test_keystore();

    let mut cmd = Command::cargo_bin("mfm_cli").unwrap();
    cmd.args(&[
        "keystore",
        "import",
        "--keystore",
        keystore_path.to_str().unwrap(),
    ]);

    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("required"));
}

#[test]
fn test_delete_error_cases() {
    for (args, expected) in [
        (Vec::new(), "Must specify either key ID or --by-label"),
        (vec!["invalid-uuid"], "Invalid UUID format"),
    ] {
        let (_temp_dir, keystore_path) = create_test_keystore();

        let mut cmd = Command::cargo_bin("mfm_cli").unwrap();
        cmd.args(&["keystore", "delete"])
            .args(args)
            .args(&["--keystore", keystore_path.to_str().unwrap()]);

        cmd.assert()
            .failure()
            .stderr(predicate::str::contains(expected));
    }
}

#[test]
fn test_list_invalid_format() {
    let (_temp_dir, keystore_path) = create_test_keystore();

    let mut cmd = Command::cargo_bin("mfm_cli").unwrap();
    cmd.args(&[
        "--output-format",
        "invalid",
        "keystore",
        "list",
        "--keystore",
        keystore_path.to_str().unwrap(),
    ]);

    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("invalid value 'invalid'"));
}

#[test]
fn test_version_flag() {
    let mut cmd = Command::cargo_bin("mfm_cli").unwrap();
    cmd.arg("--version");

    cmd.assert()
        .success()
        .stdout(predicate::str::contains("mfm"));
}

#[test]
fn test_invalid_subcommands() {
    for args in [vec!["invalid"], vec!["keystore", "invalid"]] {
        let mut cmd = Command::cargo_bin("mfm_cli").unwrap();
        cmd.args(args);

        cmd.assert()
            .failure()
            .stderr(predicate::str::contains("unrecognized subcommand"));
    }
}
