use std::ffi::OsString;
use std::io::{self, Cursor, Read};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use mfm_bitcoin::BitcoinSourceIdentity;
use mfm_ids::LocalPublicId;
use mfm_signing::SignerRef;
use static_assertions::assert_not_impl_any;

use super::value::{copy_utf8, read_bounded_with_witness, ResolvedValue};
use super::*;

static ENV_LOCK: Mutex<()> = Mutex::new(());

assert_not_impl_any!(ResolvedValue: Clone, Copy, std::fmt::Debug, std::fmt::Display, serde::Serialize);
assert_not_impl_any!(ResolvedValue: serde::de::DeserializeOwned);
assert_not_impl_any!(ResolvedValue: AsRef<str>, std::borrow::Borrow<str>);

#[test]
fn toml_and_json_select_the_identical_evm_route() {
    let directory = tempfile::tempdir().expect("tempdir");
    let toml = directory.path().join("runtime.toml");
    let json = directory.path().join("runtime.json");
    std::fs::write(
        &toml,
        r#"
[evm.routes.ethereum-mainnet]
source_ref = "primary"
rpc_url = { direct = "https://rpc.example.invalid" }
auth_header = { env = "MFM_TEST_AUTH" }
"#,
    )
    .expect("TOML");
    std::fs::write(
        &json,
        r#"{
  "evm": {"routes": {"ethereum-mainnet": {
    "source_ref": "primary",
    "rpc_url": {"direct": "https://rpc.example.invalid"},
    "auth_header": {"env": "MFM_TEST_AUTH"}
  }}}
}"#,
    )
    .expect("JSON");
    let _env = locked_env("MFM_TEST_AUTH", "Bearer protected");
    for path in [&toml, &json] {
        let route = load_evm_route(
            path,
            &LocalPublicId::new("ethereum-mainnet").expect("network"),
        )
        .expect("selected route");
        let (source, endpoint, authorization) = route.into_parts();
        assert_eq!(source.as_str(), "primary");
        assert_eq!(endpoint.into_string(), "https://rpc.example.invalid");
        assert_eq!(
            authorization
                .expect("authorization")
                .into_protected()
                .as_str(),
            "Bearer protected"
        );
    }
}

#[test]
fn json_duplicate_keys_fail_at_every_nesting_level() {
    let directory = tempfile::tempdir().expect("tempdir");
    for (name, raw) in [
        ("root", r#"{"evm":{},"evm":{"routes":{}}}"#),
        (
            "selected entry",
            r#"{"evm":{"routes":{"dev":{"source_ref":"one","source_ref":"two","rpc_url":{"direct":"http://127.0.0.1"}}}}}"#,
        ),
        (
            "value source",
            r#"{"evm":{"routes":{"dev":{"source_ref":"one","rpc_url":{"direct":"http://127.0.0.1","direct":"http://127.0.0.2"}}}}}"#,
        ),
        (
            "unselected nested object",
            r#"{"evm":{"routes":{"dev":{"source_ref":"one","rpc_url":{"direct":"http://127.0.0.1"}},"unused":{"nested":{"field":1,"field":2}}}}}"#,
        ),
    ] {
        let path = directory.path().join(format!("{name}.json"));
        std::fs::write(&path, raw).expect("JSON fixture");
        let error = load_evm_route(&path, &LocalPublicId::new("dev").expect("network"))
            .err()
            .expect(name);
        assert_eq!(error.kind(), RuntimeConfigErrorKind::DuplicateJsonKey);
    }
}

#[test]
fn selected_entries_are_strict_while_unselected_entries_are_isolated() {
    let directory = tempfile::tempdir().expect("tempdir");
    let valid = directory.path().join("selective.toml");
    std::fs::write(
        &valid,
        r#"
[evm.routes.dev]
source_ref = "primary"
rpc_url = { direct = "http://127.0.0.1:8545" }

[evm.routes.unused]
malformed = [1, 2, 3]

[bitcoin.routes.unused]
unknown = true

[signers.unused]
provider = 42

[keystores.unused]
malformed = true
"#,
    )
    .expect("config");
    load_evm_route(&valid, &LocalPublicId::new("dev").expect("network"))
        .expect("unselected entries are isolated");

    let selected_unknown = directory.path().join("selected-unknown.toml");
    std::fs::write(
        &selected_unknown,
        r#"
[evm.routes.dev]
source_ref = "primary"
rpc_url = { direct = "http://127.0.0.1:8545" }
extra = true
"#,
    )
    .expect("config");
    assert_eq!(
        load_evm_route(
            &selected_unknown,
            &LocalPublicId::new("dev").expect("network")
        )
        .err()
        .expect("selected unknown field")
        .kind(),
        RuntimeConfigErrorKind::UnknownSelectedField
    );
}

#[test]
fn global_secret_field_policy_applies_to_unselected_entries() {
    let directory = tempfile::tempdir().expect("tempdir");
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
            r#"
[evm.routes.dev]
source_ref = "primary"
rpc_url = {{ direct = "http://127.0.0.1:8545" }}

[evm.routes.unused]
unsafe_{marker}_field = "must-not-be-admitted"
"#
        );
        std::fs::write(&path, raw).expect("config");
        assert_eq!(
            load_evm_route(&path, &LocalPublicId::new("dev").expect("network"))
                .err()
                .expect(marker)
                .kind(),
            RuntimeConfigErrorKind::ForbiddenSecretField,
            "{marker}"
        );
    }

    for (name, field) in [
        (
            "bitcoin",
            "[bitcoin.routes.unused]\nrpc_password = { direct = \"plaintext\" }",
        ),
        (
            "evm",
            "[evm.routes.unused]\nauth_header = { direct = \"Bearer plaintext\" }",
        ),
    ] {
        let path = directory.path().join(format!("direct-{name}.toml"));
        std::fs::write(
            &path,
            format!(
                "[evm.routes.dev]\nsource_ref = \"primary\"\nrpc_url = {{ direct = \"http://127.0.0.1:8545\" }}\n\n{field}\n"
            ),
        )
        .expect("config");
        assert_eq!(
            load_evm_route(&path, &LocalPublicId::new("dev").expect("network"))
                .err()
                .expect(name)
                .kind(),
            RuntimeConfigErrorKind::DirectSecretValue
        );
    }
}

#[test]
fn expected_chain_id_and_unknown_top_level_sections_fail_globally() {
    let directory = tempfile::tempdir().expect("tempdir");
    let expected_chain = directory.path().join("expected-chain.toml");
    std::fs::write(
        &expected_chain,
        r#"
[evm.routes.dev]
source_ref = "primary"
rpc_url = { direct = "http://127.0.0.1:8545" }

[evm.routes.unused.nested]
expected-chain-id = 1
"#,
    )
    .expect("config");
    assert_eq!(
        load_evm_route(
            &expected_chain,
            &LocalPublicId::new("dev").expect("network")
        )
        .err()
        .expect("expected chain id")
        .kind(),
        RuntimeConfigErrorKind::ForbiddenExpectedChainId
    );

    let unknown = directory.path().join("unknown.toml");
    std::fs::write(&unknown, "[btc.routes.legacy]\nvalue = true\n").expect("config");
    assert_eq!(
        load_evm_route(&unknown, &LocalPublicId::new("dev").expect("network"))
            .err()
            .expect("legacy top level")
            .kind(),
        RuntimeConfigErrorKind::UnknownTopLevel
    );
}

#[test]
fn value_sources_require_exactly_one_known_key() {
    let directory = tempfile::tempdir().expect("tempdir");
    for (name, source, expected) in [
        ("empty", "{}", RuntimeConfigErrorKind::InvalidValueSource),
        (
            "multiple",
            "{ direct = \"http://127.0.0.1\", env = \"MFM_RPC\" }",
            RuntimeConfigErrorKind::InvalidValueSource,
        ),
        (
            "unknown",
            "{ fallback = \"http://127.0.0.1\" }",
            RuntimeConfigErrorKind::UnknownSelectedField,
        ),
    ] {
        let path = directory.path().join(format!("{name}.toml"));
        std::fs::write(
            &path,
            format!("[evm.routes.dev]\nsource_ref = \"primary\"\nrpc_url = {source}\n"),
        )
        .expect("config");
        assert_eq!(
            load_evm_route(&path, &LocalPublicId::new("dev").expect("network"))
                .err()
                .expect(name)
                .kind(),
            expected
        );
    }
}

#[test]
fn direct_env_file_and_file_env_preserve_the_defined_bytes() {
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
        "[keystores.default]\nkeystore_path = { env = \"lowercase\" }\nunlock_file_path = { direct = \"/tmp/unlock\" }\n",
    )
    .expect("config");
    assert_eq!(
        load_keystore_profile(&invalid_name, "default")
            .err()
            .expect("invalid env name")
            .kind(),
        RuntimeConfigErrorKind::InvalidEnvironmentValue
    );

    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        std::env::set_var("MFM_TEST_NON_UNICODE", OsString::from_vec(vec![0xff]));
        let non_unicode = directory.path().join("non-unicode.toml");
        std::fs::write(
            &non_unicode,
            "[keystores.default]\nkeystore_path = { env = \"MFM_TEST_NON_UNICODE\" }\nunlock_file_path = { direct = \"/tmp/unlock\" }\n",
        )
        .expect("config");
        assert_eq!(
            load_keystore_profile(&non_unicode, "default")
                .err()
                .expect("non-Unicode env")
                .kind(),
            RuntimeConfigErrorKind::InvalidEnvironmentValue
        );
        std::env::remove_var("MFM_TEST_NON_UNICODE");
    }
}

#[test]
fn bitcoin_selection_enforces_timeout_and_complete_basic_auth() {
    let directory = tempfile::tempdir().expect("tempdir");
    for timeout in [0, 86_401] {
        let path = directory.path().join(format!("timeout-{timeout}.toml"));
        std::fs::write(
            &path,
            format!(
                "[bitcoin.routes.primary]\nrpc_url = {{ direct = \"http://127.0.0.1:8332\" }}\nscan_timeout_seconds = {timeout}\n"
            ),
        )
        .expect("config");
        assert_eq!(
            load_bitcoin_route(
                &path,
                &BitcoinSourceIdentity::new("primary").expect("source")
            )
            .err()
            .expect("invalid timeout")
            .kind(),
            RuntimeConfigErrorKind::InvalidScanTimeout
        );
    }

    let incomplete = directory.path().join("incomplete.toml");
    std::fs::write(
        &incomplete,
        "[bitcoin.routes.primary]\nrpc_url = { direct = \"http://127.0.0.1:8332\" }\nrpc_user = { direct = \"user\" }\nscan_timeout_seconds = 30\n",
    )
    .expect("config");
    assert_eq!(
        load_bitcoin_route(
            &incomplete,
            &BitcoinSourceIdentity::new("primary").expect("source")
        )
        .err()
        .expect("incomplete auth")
        .kind(),
        RuntimeConfigErrorKind::IncompleteBasicAuth
    );
}

#[test]
fn document_and_selected_value_limits_use_limit_plus_one() {
    let directory = tempfile::tempdir().expect("tempdir");
    let oversized_document = directory.path().join("oversized.toml");
    std::fs::write(&oversized_document, vec![b' '; 1024 * 1024 + 1]).expect("document");
    assert_eq!(
        load_evm_route(
            &oversized_document,
            &LocalPublicId::new("dev").expect("network")
        )
        .err()
        .expect("oversized document")
        .kind(),
        RuntimeConfigErrorKind::DocumentTooLarge
    );

    let selected = directory.path().join("selected-large.toml");
    std::fs::write(
        &selected,
        format!(
            "[keystores.default]\nkeystore_path = {{ direct = {} }}\nunlock_file_path = {{ direct = \"/tmp/unlock\" }}\n",
            toml_string(&"x".repeat(64 * 1024 + 1))
        ),
    )
    .expect("config");
    assert_eq!(
        load_keystore_profile(&selected, "default")
            .err()
            .expect("selected value limit")
            .kind(),
        RuntimeConfigErrorKind::ResolvedValueTooLarge
    );

    let isolated = directory.path().join("isolated-large.toml");
    std::fs::write(
        &isolated,
        format!(
            "[evm.routes.dev]\nsource_ref = \"primary\"\nrpc_url = {{ direct = \"http://127.0.0.1:8545\" }}\n\n[evm.routes.unused]\nrpc_url = {{ direct = {} }}\n",
            toml_string(&"x".repeat(70_000))
        ),
    )
    .expect("config");
    load_evm_route(&isolated, &LocalPublicId::new("dev").expect("network"))
        .expect("unselected direct value uses only document bound");
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
                "[keystores.default]\nkeystore_path = {{ file = {} }}\nunlock_file_path = {{ direct = \"/tmp/unlock\" }}\n",
                toml_string(&value_file.display().to_string())
            ),
        )
        .expect("config");
        assert_eq!(
            load_keystore_profile(&config, "default")
                .err()
                .expect(name)
                .kind(),
            expected,
            "{name}"
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
fn signer_selection_decodes_only_the_signer_and_referenced_keystore() {
    let directory = tempfile::tempdir().expect("tempdir");
    let config = directory.path().join("signer.toml");
    std::fs::write(
        &config,
        r#"
[keystores.primary]
keystore_path = { direct = "/run/mfm/primary.keystore" }
unlock_file_path = { direct = "/run/mfm/primary.unlock" }

[keystores.unused]
malformed = true

[signers.deployer]
provider = "keystore"
keystore_ref = "primary"
entry_id = "67e55044-10b1-426f-9247-bb680e5fe0c8"

[signers.unused]
provider = 42
"#,
    )
    .expect("config");
    let (entry_id, keystore, unlock) =
        load_signer_binding(&config, &SignerRef::new("deployer").expect("signer"))
            .expect("selected signer")
            .into_parts();
    assert_eq!(entry_id.to_string(), "67e55044-10b1-426f-9247-bb680e5fe0c8");
    assert_eq!(keystore, std::path::Path::new("/run/mfm/primary.keystore"));
    assert_eq!(unlock, std::path::Path::new("/run/mfm/primary.unlock"));
}

#[test]
fn paths_formats_and_errors_are_closed_and_redacted() {
    let directory = tempfile::tempdir().expect("tempdir");
    let upper = directory.path().join("private-runtime.TOML");
    std::fs::write(&upper, "").expect("config");
    let error = load_evm_route(&upper, &LocalPublicId::new("dev").expect("network"))
        .err()
        .expect("case-sensitive extension");
    assert_eq!(error.kind(), RuntimeConfigErrorKind::UnsupportedFormat);

    let secret_path = directory.path().join("private-runtime.toml");
    std::fs::write(
        &secret_path,
        "[evm.routes.dev]\nsource_ref = \"primary\"\nrpc_url = { env = \"MFM_PRIVATE_ENV_NAME\" }\n",
    )
    .expect("config");
    let error = load_evm_route(&secret_path, &LocalPublicId::new("dev").expect("network"))
        .err()
        .expect("missing environment value");
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

struct EnvGuard {
    name: &'static str,
    _guard: std::sync::MutexGuard<'static, ()>,
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        std::env::remove_var(self.name);
    }
}

fn locked_env(name: &'static str, value: &str) -> EnvGuard {
    let guard = ENV_LOCK.lock().expect("environment lock");
    std::env::set_var(name, value);
    EnvGuard {
        name,
        _guard: guard,
    }
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
