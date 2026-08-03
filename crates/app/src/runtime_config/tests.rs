use std::io::{self, Cursor, Read};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use super::value::{copy_utf8, read_bounded_with_witness};
use super::*;

static ENV_LOCK: Mutex<()> = Mutex::new(());

#[test]
fn toml_and_json_resolve_the_same_keystore_profile() {
    let directory = tempfile::tempdir().expect("tempdir");
    let toml = directory.path().join("runtime.toml");
    let json = directory.path().join("runtime.json");
    std::fs::write(
        &toml,
        r#"
[keystores.default]
keystore_path = { direct = "/run/mfm/wallet.keystore" }
unlock_file_path = { direct = "/run/mfm/wallet.unlock" }
"#,
    )
    .expect("TOML config");
    std::fs::write(
        &json,
        r#"{"keystores":{"default":{"keystore_path":{"direct":"/run/mfm/wallet.keystore"},"unlock_file_path":{"direct":"/run/mfm/wallet.unlock"}}}}"#,
    )
    .expect("JSON config");

    for path in [&toml, &json] {
        let (keystore, unlock) = load_keystore_profile(path, "default")
            .expect("profile")
            .into_paths();
        assert_eq!(keystore, std::path::Path::new("/run/mfm/wallet.keystore"));
        assert_eq!(unlock, std::path::Path::new("/run/mfm/wallet.unlock"));
    }
}

#[test]
fn json_duplicate_keys_fail_at_every_nesting_level() {
    let directory = tempfile::tempdir().expect("tempdir");
    for (name, raw) in [
        ("root", r#"{"keystores":{},"keystores":{"default":{}}}"#),
        (
            "profile",
            r#"{"keystores":{"default":{"keystore_path":{"direct":"/one"},"keystore_path":{"direct":"/two"},"unlock_file_path":{"direct":"/unlock"}}}}"#,
        ),
        (
            "source",
            r#"{"keystores":{"default":{"keystore_path":{"direct":"/one","direct":"/two"},"unlock_file_path":{"direct":"/unlock"}}}}"#,
        ),
        (
            "unselected",
            r#"{"keystores":{"default":{"keystore_path":{"direct":"/one"},"unlock_file_path":{"direct":"/unlock"}},"unused":{"nested":{"field":1,"field":2}}}}"#,
        ),
    ] {
        let path = directory.path().join(format!("{name}.json"));
        std::fs::write(&path, raw).expect("fixture");
        assert_eq!(
            load_keystore_profile(&path, "default")
                .expect_err(name)
                .kind(),
            RuntimeConfigErrorKind::DuplicateJsonKey,
            "{name}",
        );
    }
}

#[test]
fn selected_profile_and_document_shape_fail_closed() {
    let directory = tempfile::tempdir().expect("tempdir");
    for (name, raw, profile, expected) in [
        (
            "unknown-field",
            "[keystores.default]\nkeystore_path = { direct = \"/key\" }\nunlock_file_path = { direct = \"/unlock\" }\nextra = true\n",
            "default",
            RuntimeConfigErrorKind::UnknownSelectedField,
        ),
        (
            "missing-field",
            "[keystores.default]\nkeystore_path = { direct = \"/key\" }\n",
            "default",
            RuntimeConfigErrorKind::MissingRequiredField,
        ),
        (
            "missing-profile",
            "[keystores.default]\nkeystore_path = { direct = \"/key\" }\nunlock_file_path = { direct = \"/unlock\" }\n",
            "other",
            RuntimeConfigErrorKind::MissingKeystore,
        ),
        (
            "missing-section",
            "",
            "default",
            RuntimeConfigErrorKind::MissingSection,
        ),
        (
            "unknown-top-level",
            "[signers.default]\nprovider = \"retired\"\n",
            "default",
            RuntimeConfigErrorKind::UnknownTopLevel,
        ),
    ] {
        let path = directory.path().join(format!("{name}.toml"));
        std::fs::write(&path, raw).expect("fixture");
        assert_eq!(
            load_keystore_profile(&path, profile)
                .expect_err(name)
                .kind(),
            expected,
            "{name}",
        );
    }

    let invalid_ref = directory.path().join("invalid-ref.toml");
    std::fs::write(
        &invalid_ref,
        "[keystores.default]\nkeystore_path = { direct = \"/key\" }\nunlock_file_path = { direct = \"/unlock\" }\n",
    )
    .expect("fixture");
    assert_eq!(
        load_keystore_profile(&invalid_ref, "Not Valid")
            .expect_err("invalid identifier")
            .kind(),
        RuntimeConfigErrorKind::InvalidIdentifier,
    );
}

#[test]
fn unselected_profiles_are_isolated_but_secret_fields_are_rejected_globally() {
    let directory = tempfile::tempdir().expect("tempdir");
    let isolated = directory.path().join("isolated.toml");
    std::fs::write(
        &isolated,
        r#"
[keystores.default]
keystore_path = { direct = "/key" }
unlock_file_path = { direct = "/unlock" }

[keystores.unused]
malformed = true
"#,
    )
    .expect("config");
    load_keystore_profile(&isolated, "default").expect("selected profile only");

    for marker in [
        "password",
        "passphrase",
        "mnemonic",
        "seed_phrase",
        "private_key",
        "secret",
        "credential",
        "api_key",
        "access_key",
        "token",
        "authorization",
        "signed_material",
        "signed_transaction",
        "raw_transaction",
        "raw_tx",
        "signature_scalar",
    ] {
        let path = directory.path().join(format!("{marker}.toml"));
        let raw = format!(
            "[keystores.default]\nkeystore_path = {{ direct = \"/key\" }}\nunlock_file_path = {{ direct = \"/unlock\" }}\n\n[keystores.unused]\nunsafe_{marker}_field = \"must-not-be-admitted\"\n"
        );
        std::fs::write(&path, raw).expect("fixture");
        assert_eq!(
            load_keystore_profile(&path, "default")
                .expect_err(marker)
                .kind(),
            RuntimeConfigErrorKind::ForbiddenSecretField,
            "{marker}",
        );
    }
}

#[test]
fn value_sources_require_exactly_one_known_key() {
    let directory = tempfile::tempdir().expect("tempdir");
    for (name, source, expected) in [
        ("empty", "{}", RuntimeConfigErrorKind::InvalidValueSource),
        (
            "multiple",
            "{ direct = \"/key\", env = \"MFM_KEY_PATH\" }",
            RuntimeConfigErrorKind::InvalidValueSource,
        ),
        (
            "unknown",
            "{ fallback = \"/key\" }",
            RuntimeConfigErrorKind::UnknownSelectedField,
        ),
    ] {
        let path = directory.path().join(format!("{name}.toml"));
        std::fs::write(
            &path,
            format!(
                "[keystores.default]\nkeystore_path = {source}\nunlock_file_path = {{ direct = \"/unlock\" }}\n"
            ),
        )
        .expect("fixture");
        assert_eq!(
            load_keystore_profile(&path, "default")
                .expect_err(name)
                .kind(),
            expected,
            "{name}",
        );
    }
}

#[test]
fn direct_env_file_and_file_env_preserve_defined_path_bytes() {
    let _lock = ENV_LOCK.lock().expect("environment lock");
    let directory = tempfile::tempdir().expect("tempdir");
    let value_file = directory.path().join("value-file");
    let file_env_target = directory.path().join("file-env-target");
    std::fs::write(&value_file, "/tmp/file-value\r\n").expect("value file");
    std::fs::write(&file_env_target, "/tmp/file-env-value\n\n").expect("file env target");
    std::env::set_var("MFM_TEST_PATH_VALUE", " /tmp/env-value ");
    std::env::set_var("MFM_TEST_PATH_FILE", file_env_target.as_os_str());
    let config = directory.path().join("values.toml");
    std::fs::write(
        &config,
        format!(
            r#"
[keystores.direct]
keystore_path = {{ direct = " /tmp/direct-value " }}
unlock_file_path = {{ direct = "/tmp/unlock" }}

[keystores.env]
keystore_path = {{ env = "MFM_TEST_PATH_VALUE" }}
unlock_file_path = {{ direct = "/tmp/unlock" }}

[keystores.file]
keystore_path = {{ file = {} }}
unlock_file_path = {{ direct = "/tmp/unlock" }}

[keystores.file-env]
keystore_path = {{ file_env = "MFM_TEST_PATH_FILE" }}
unlock_file_path = {{ direct = "/tmp/unlock" }}
"#,
            toml_string(&value_file.display().to_string())
        ),
    )
    .expect("config");

    for (profile, expected) in [
        ("direct", " /tmp/direct-value "),
        ("env", " /tmp/env-value "),
        ("file", "/tmp/file-value"),
        ("file-env", "/tmp/file-env-value\n"),
    ] {
        let (path, _) = load_keystore_profile(&config, profile)
            .expect(profile)
            .into_paths();
        assert_eq!(path.to_str().expect("UTF-8 path"), expected, "{profile}");
    }
    std::env::remove_var("MFM_TEST_PATH_VALUE");
    std::env::remove_var("MFM_TEST_PATH_FILE");
}

#[test]
fn environment_names_values_and_paths_fail_closed() {
    let _lock = ENV_LOCK.lock().expect("environment lock");
    let directory = tempfile::tempdir().expect("tempdir");
    let invalid_name = directory.path().join("invalid-name.toml");
    std::fs::write(
        &invalid_name,
        "[keystores.default]\nkeystore_path = { env = \"lowercase\" }\nunlock_file_path = { direct = \"/unlock\" }\n",
    )
    .expect("config");
    assert_eq!(
        load_keystore_profile(&invalid_name, "default")
            .expect_err("invalid env name")
            .kind(),
        RuntimeConfigErrorKind::InvalidEnvironmentValue,
    );

    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        std::env::set_var(
            "MFM_TEST_NON_UNICODE",
            std::ffi::OsString::from_vec(vec![0xff]),
        );
        let non_unicode = directory.path().join("non-unicode.toml");
        std::fs::write(
            &non_unicode,
            "[keystores.default]\nkeystore_path = { env = \"MFM_TEST_NON_UNICODE\" }\nunlock_file_path = { direct = \"/unlock\" }\n",
        )
        .expect("config");
        assert_eq!(
            load_keystore_profile(&non_unicode, "default")
                .expect_err("non-Unicode env")
                .kind(),
            RuntimeConfigErrorKind::InvalidEnvironmentValue,
        );
        std::env::remove_var("MFM_TEST_NON_UNICODE");
    }
}

#[test]
fn document_and_resolved_value_limits_use_limit_plus_one() {
    let directory = tempfile::tempdir().expect("tempdir");
    let oversized_document = directory.path().join("oversized.toml");
    std::fs::write(&oversized_document, vec![b' '; 1024 * 1024 + 1]).expect("document");
    assert_eq!(
        load_keystore_profile(&oversized_document, "default")
            .expect_err("oversized document")
            .kind(),
        RuntimeConfigErrorKind::DocumentTooLarge,
    );

    let selected = directory.path().join("selected-large.toml");
    std::fs::write(
        &selected,
        format!(
            "[keystores.default]\nkeystore_path = {{ direct = {} }}\nunlock_file_path = {{ direct = \"/unlock\" }}\n",
            toml_string(&"x".repeat(64 * 1024 + 1))
        ),
    )
    .expect("config");
    assert_eq!(
        load_keystore_profile(&selected, "default")
            .expect_err("selected value limit")
            .kind(),
        RuntimeConfigErrorKind::ResolvedValueTooLarge,
    );
}

#[test]
fn indirection_files_reject_empty_invalid_utf8_and_limit_plus_one() {
    let directory = tempfile::tempdir().expect("tempdir");
    for (name, contents, expected) in [
        (
            "empty",
            Vec::new(),
            RuntimeConfigErrorKind::EmptyResolvedValue,
        ),
        (
            "invalid-utf8",
            vec![0xff],
            RuntimeConfigErrorKind::InvalidUtf8,
        ),
        (
            "oversized",
            vec![b'x'; 64 * 1024 + 1],
            RuntimeConfigErrorKind::ResolvedValueTooLarge,
        ),
    ] {
        let value_file = directory.path().join(name);
        std::fs::write(&value_file, contents).expect("value file");
        let config = directory.path().join(format!("{name}.toml"));
        std::fs::write(
            &config,
            format!(
                "[keystores.default]\nkeystore_path = {{ file = {} }}\nunlock_file_path = {{ direct = \"/unlock\" }}\n",
                toml_string(&value_file.display().to_string())
            ),
        )
        .expect("config");
        assert_eq!(
            load_keystore_profile(&config, "default")
                .expect_err(name)
                .kind(),
            expected,
            "{name}",
        );
    }
}

#[test]
fn protected_read_buffers_zeroize_on_success_and_every_error_path() {
    for (name, reader, limit, succeeds) in [
        ("success", PartialReader::success(b"protected"), 64, true),
        ("empty", PartialReader::success(b""), 64, false),
        (
            "invalid UTF-8",
            PartialReader::success(b"\xffprotected"),
            64,
            false,
        ),
        (
            "partial failure",
            PartialReader::failure_after(b"partial-secret"),
            64,
            false,
        ),
        ("limit plus one", PartialReader::success(b"12345"), 4, false),
    ] {
        let witness = Arc::new(AtomicBool::new(false));
        let result = read_bounded_with_witness(reader, limit, Arc::clone(&witness))
            .and_then(|bytes| copy_utf8(&bytes));
        assert_eq!(result.is_ok(), succeeds, "{name}");
        drop(result);
        assert!(witness.load(Ordering::SeqCst), "{name} zeroize witness");
    }
}

#[test]
fn paths_formats_and_errors_are_closed_and_redacted() {
    let directory = tempfile::tempdir().expect("tempdir");
    let upper = directory.path().join("private-runtime.TOML");
    std::fs::write(&upper, "").expect("config");
    let error = load_keystore_profile(&upper, "default").expect_err("case-sensitive extension");
    assert_eq!(error.kind(), RuntimeConfigErrorKind::UnsupportedFormat);

    let secret_path = directory.path().join("private-runtime.toml");
    std::fs::write(
        &secret_path,
        "[keystores.default]\nkeystore_path = { env = \"MFM_PRIVATE_ENV_NAME\" }\nunlock_file_path = { direct = \"/unlock\" }\n",
    )
    .expect("config");
    let error =
        load_keystore_profile(&secret_path, "default").expect_err("missing environment value");
    let rendered = format!("{error:?} {error}");
    for forbidden in [
        "private-runtime.toml",
        "MFM_PRIVATE_ENV_NAME",
        directory.path().to_str().expect("path"),
    ] {
        assert!(!rendered.contains(forbidden), "leaked {forbidden}");
    }
}

fn toml_string(value: &str) -> String {
    serde_json::to_string(value).expect("TOML-compatible string")
}

struct PartialReader {
    bytes: Cursor<Vec<u8>>,
    fail_after_payload: bool,
}

impl PartialReader {
    fn success(bytes: &[u8]) -> Self {
        Self {
            bytes: Cursor::new(bytes.to_vec()),
            fail_after_payload: false,
        }
    }

    fn failure_after(bytes: &[u8]) -> Self {
        Self {
            bytes: Cursor::new(bytes.to_vec()),
            fail_after_payload: true,
        }
    }
}

impl Read for PartialReader {
    fn read(&mut self, target: &mut [u8]) -> io::Result<usize> {
        let read = self.bytes.read(target)?;
        if read == 0 && self.fail_after_payload {
            self.fail_after_payload = false;
            return Err(io::Error::other("injected partial read failure"));
        }
        Ok(read)
    }
}
