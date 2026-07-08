use std::fs;
use std::sync::Mutex;

use mfm_evm_capabilities::{EvmSourcePolicyId, EvmSourceRef};
use mfm_runtime_config::{
    KeystoreRef, RuntimeConfig, RuntimeConfigErrorKind, RuntimeConfigFormat,
    RuntimeConfigRequirement, RuntimeSecretValue, RuntimeValueSourceKind,
};
use mfm_signing::SignerRef;
use tempfile::tempdir;

static ENV_LOCK: Mutex<()> = Mutex::new(());

#[test]
fn toml_and_json_parsing_have_matching_descriptors() {
    let toml = r#"
        [evm.sources.local]
        rpc_url = "http://127.0.0.1:8545"

        [evm.routes.dev]
        source_ref = "local"
    "#;
    let json = r#"
        {
          "evm": {
            "sources": {
              "local": { "rpc_url": "http://127.0.0.1:8545" }
            },
            "routes": {
              "dev": { "source_ref": "local" }
            }
          }
        }
    "#;

    let from_toml = RuntimeConfig::from_str(toml, RuntimeConfigFormat::Toml).expect("toml");
    let from_json = RuntimeConfig::from_str(json, RuntimeConfigFormat::Json).expect("json");

    assert_eq!(from_toml, from_json);
    let evm = from_toml.evm().expect("evm");
    let local = EvmSourceRef::new("local").expect("source ref");
    let local_policy = EvmSourcePolicyId::new("local").expect("policy id");
    assert_eq!(
        evm.sources()
            .get(&local)
            .expect("local source")
            .rpc_url()
            .expose_secret(),
        "http://127.0.0.1:8545"
    );
    assert!(evm
        .policies()
        .get(&local_policy)
        .expect("synthesized policy")
        .is_synthesized());
}

#[test]
fn btc_jsonrpc_runtime_config_parses_from_toml_and_json() {
    let toml = r#"
        [btc.routes.public-bitcoin-core]
        rpc_url = "http://127.0.0.1:8332"
        rpc_user = "rpc-user"
        rpc_password = "rpc-pass"
    "#;
    let json = r#"
        {
          "btc": {
            "routes": {
              "public-bitcoin-core": {
                "rpc_url": "http://127.0.0.1:8332",
                "rpc_user": "rpc-user",
                "rpc_password": "rpc-pass"
              }
            }
          }
        }
    "#;

    let from_toml = RuntimeConfig::from_str(toml, RuntimeConfigFormat::Toml).expect("toml");
    let from_json = RuntimeConfig::from_str(json, RuntimeConfigFormat::Json).expect("json");

    assert_eq!(from_toml, from_json);
    let btc = from_toml
        .btc()
        .expect("btc")
        .routes()
        .get(&"public-bitcoin-core".parse().expect("btc source"))
        .expect("btc route");
    assert_value(
        btc.rpc_url(),
        "http://127.0.0.1:8332",
        RuntimeValueSourceKind::Direct,
    );
    assert_value(
        btc.rpc_user().expect("user"),
        "rpc-user",
        RuntimeValueSourceKind::Direct,
    );
    assert_value(
        btc.rpc_password().expect("password"),
        "rpc-pass",
        RuntimeValueSourceKind::Direct,
    );
}

#[test]
fn old_btc_singleton_json_rpc_shape_is_rejected() {
    let old_shape = r#"
        [btc.json_rpc]
        rpc_url = "http://127.0.0.1:8332"
    "#;

    let err = RuntimeConfig::from_str(old_shape, RuntimeConfigFormat::Toml)
        .expect_err("old singleton btc json rpc config must be rejected");

    assert_eq!(err.kind(), &RuntimeConfigErrorKind::UnknownField);
    assert_eq!(err.location().to_string(), "btc");
}

#[test]
fn dual_mainnet_runtime_example_resolves_from_env() {
    let _env = locked_env([
        ("MFM_ETHEREUM_MAINNET_RPC_URL", "http://127.0.0.1:8545"),
        ("MFM_BITCOIN_RPC_URL", "http://127.0.0.1:8332"),
        ("MFM_BITCOIN_RPC_USER", "rpc-user"),
        ("MFM_BITCOIN_RPC_PASSWORD", "rpc-pass"),
    ]);
    let raw = include_str!("../../../examples/configs/runtime-dual-mainnet.toml");

    let runtime = RuntimeConfig::from_str(raw, RuntimeConfigFormat::Toml).expect("runtime config");

    assert_eq!(runtime.evm().expect("evm").routes().len(), 1);
    let btc = runtime
        .btc()
        .expect("btc")
        .routes()
        .get(&"bitcoin-mainnet".parse().expect("btc source"))
        .expect("btc route");
    assert_value(
        btc.rpc_url(),
        "http://127.0.0.1:8332",
        RuntimeValueSourceKind::Env,
    );
    assert_value(
        btc.rpc_user().expect("user"),
        "rpc-user",
        RuntimeValueSourceKind::Env,
    );
    assert_value(
        btc.rpc_password().expect("password"),
        "rpc-pass",
        RuntimeValueSourceKind::Env,
    );
}

#[test]
fn required_runtime_families_must_exist() {
    for (name, requirement) in [
        ("btc", RuntimeConfigRequirement::btc()),
        ("evm", RuntimeConfigRequirement::evm()),
    ] {
        let err =
            RuntimeConfig::from_str_with_requirements("", RuntimeConfigFormat::Toml, requirement)
                .expect_err(name);
        assert_eq!(err.kind(), &RuntimeConfigErrorKind::MissingFamily, "{name}");
    }
}

#[test]
fn btc_basic_auth_requires_user_and_password_together() {
    let config = r#"
        [btc.routes.public-bitcoin-core]
        rpc_url = "http://127.0.0.1:8332"
        rpc_user = "rpc-user"
    "#;

    let err = RuntimeConfig::from_str(config, RuntimeConfigFormat::Toml)
        .expect_err("incomplete basic auth");

    assert_eq!(err.kind(), &RuntimeConfigErrorKind::IncompleteBasicAuth);
}

#[test]
fn direct_env_file_and_file_env_sources_resolve() {
    let dir = tempdir().expect("tempdir");
    let rpc_file = dir.path().join("rpc-url");
    let auth_file = dir.path().join("auth-header");
    let file_env_target = dir.path().join("file-env-target");
    fs::write(&rpc_file, "http://127.0.0.1:8547\n").expect("rpc file");
    fs::write(&auth_file, "Bearer file-token\n").expect("auth file");
    fs::write(&file_env_target, "http://127.0.0.1:8548\n").expect("file env target");
    let rpc_file = rpc_file.display().to_string();
    let auth_file = auth_file.display().to_string();
    let file_env_target = file_env_target.display().to_string();
    let _env = locked_env([
        ("MFM_RUNTIME_CONFIG_TEST_RPC_ENV", "http://127.0.0.1:8546"),
        ("MFM_RUNTIME_CONFIG_TEST_AUTH_ENV", "Bearer env-token"),
        ("MFM_RUNTIME_CONFIG_TEST_FILE_ENV", file_env_target.as_str()),
    ]);

    let config = format!(
        r#"
        [evm.sources.direct]
        rpc_url = "http://127.0.0.1:8545"

        [evm.sources.env]
        rpc_url_env = "MFM_RUNTIME_CONFIG_TEST_RPC_ENV"
        auth_header_env = "MFM_RUNTIME_CONFIG_TEST_AUTH_ENV"

        [evm.sources.file]
        rpc_url_file = "{rpc_file}"
        auth_header_file = "{auth_file}"

        [evm.sources.file-env]
        rpc_url_file_env = "MFM_RUNTIME_CONFIG_TEST_FILE_ENV"

        [evm.routes.direct]
        source_ref = "direct"

        [evm.routes.env]
        source_ref = "env"

        [evm.routes.file]
        source_ref = "file"

        [evm.routes.file-env]
        source_ref = "file-env"
        "#
    );

    let runtime = RuntimeConfig::from_str(&config, RuntimeConfigFormat::Toml).expect("runtime");
    let evm = runtime.evm().expect("evm");

    assert_value(
        evm.sources()
            .get(&EvmSourceRef::new("direct").expect("direct"))
            .expect("direct source")
            .rpc_url(),
        "http://127.0.0.1:8545",
        RuntimeValueSourceKind::Direct,
    );
    assert_value(
        evm.sources()
            .get(&EvmSourceRef::new("env").expect("env"))
            .expect("env source")
            .rpc_url(),
        "http://127.0.0.1:8546",
        RuntimeValueSourceKind::Env,
    );
    assert_value(
        evm.sources()
            .get(&EvmSourceRef::new("file").expect("file"))
            .expect("file source")
            .rpc_url(),
        "http://127.0.0.1:8547",
        RuntimeValueSourceKind::File,
    );
    assert_value(
        evm.sources()
            .get(&EvmSourceRef::new("file-env").expect("file env"))
            .expect("file env source")
            .rpc_url(),
        "http://127.0.0.1:8548",
        RuntimeValueSourceKind::FileEnv,
    );
    assert_value(
        evm.sources()
            .get(&EvmSourceRef::new("env").expect("env"))
            .expect("env source")
            .auth_header()
            .expect("auth header"),
        "Bearer env-token",
        RuntimeValueSourceKind::Env,
    );
}

#[test]
fn value_sources_enforce_endpoint_and_auth_cardinality() {
    for (name, config, expected) in [
        (
            "missing endpoint source",
            r#"
            [evm.sources.local]

            [evm.routes.dev]
            source_ref = "local"
            "#,
            RuntimeConfigErrorKind::ExactlyOneValueSource,
        ),
        (
            "duplicate endpoint source",
            r#"
            [evm.sources.local]
            rpc_url = "http://127.0.0.1:8545"
            rpc_url_env = "MFM_RUNTIME_CONFIG_TEST_RPC_ENV"

            [evm.routes.dev]
            source_ref = "local"
            "#,
            RuntimeConfigErrorKind::ExactlyOneValueSource,
        ),
        (
            "duplicate auth source",
            r#"
            [evm.sources.local]
            rpc_url = "http://127.0.0.1:8545"
            auth_header = "Bearer direct"
            auth_header_env = "MFM_RUNTIME_CONFIG_TEST_AUTH_ENV"

            [evm.routes.dev]
            source_ref = "local"
            "#,
            RuntimeConfigErrorKind::AtMostOneValueSource,
        ),
    ] {
        let err = RuntimeConfig::from_str(config, RuntimeConfigFormat::Toml).expect_err(name);
        assert_eq!(err.kind(), &expected, "{name}");
    }
}

#[test]
fn urls_with_userinfo_are_rejected() {
    let config = r#"
        [evm.sources.local]
        rpc_url = "https://user:pass@example.invalid"

        [evm.routes.dev]
        source_ref = "local"
    "#;
    let err = RuntimeConfig::from_str(config, RuntimeConfigFormat::Toml).expect_err("userinfo");
    assert_eq!(err.kind(), &RuntimeConfigErrorKind::UrlUserInfo);
}

#[test]
fn route_source_policy_and_signer_ids_are_checked() {
    let bad_source = r#"
        [evm.sources."bad/source"]
        rpc_url = "http://127.0.0.1:8545"
    "#;
    let err = RuntimeConfig::from_str(bad_source, RuntimeConfigFormat::Toml).expect_err("source");
    assert!(matches!(
        err.kind(),
        RuntimeConfigErrorKind::InvalidIdentifier { .. }
    ));

    let bad_policy = r#"
        [evm.sources.local]
        rpc_url = "http://127.0.0.1:8545"

        [evm.policies."BadPolicy"]
        ordered_sources = ["local"]
    "#;
    let err = RuntimeConfig::from_str(bad_policy, RuntimeConfigFormat::Toml).expect_err("policy");
    assert!(matches!(
        err.kind(),
        RuntimeConfigErrorKind::InvalidIdentifier { .. }
    ));

    let bad_signer = r#"
        [keystores.default]
        keystore_path = "/runtime/keystore.json"
        unlock_file = "/runtime/unlock"

        [signers."bad/signer"]
        provider = "keystore"
        keystore_ref = "default"
        entry_id = "00000000-0000-0000-0000-000000000000"
    "#;
    let err = RuntimeConfig::from_str(bad_signer, RuntimeConfigFormat::Toml).expect_err("signer");
    assert!(matches!(
        err.kind(),
        RuntimeConfigErrorKind::InvalidIdentifier { .. }
    ));
}

#[test]
fn same_id_policy_is_synthesized_and_explicit_same_id_requires_policy_id() {
    let synthesized = r#"
        [evm.sources.local]
        rpc_url = "http://127.0.0.1:8545"

        [evm.routes.dev]
        source_ref = "local"
    "#;
    let runtime =
        RuntimeConfig::from_str(synthesized, RuntimeConfigFormat::Toml).expect("synthesized");
    let policy_id = EvmSourcePolicyId::new("local").expect("policy");
    assert!(runtime
        .evm()
        .expect("evm")
        .policies()
        .get(&policy_id)
        .expect("policy")
        .is_synthesized());

    let rejected = r#"
        [evm.sources.local]
        rpc_url = "http://127.0.0.1:8545"

        [evm.policies.local]
        ordered_sources = ["local"]

        [evm.routes.dev]
        source_ref = "local"
    "#;
    let err = RuntimeConfig::from_str(rejected, RuntimeConfigFormat::Toml).expect_err("same id");
    assert_eq!(
        err.kind(),
        &RuntimeConfigErrorKind::SameIdPolicyRequiresExplicitPolicyId
    );
}

#[test]
fn duplicate_policy_sources_are_rejected() {
    let config = r#"
        [evm.sources.local]
        rpc_url = "http://127.0.0.1:8545"

        [evm.policies.local]
        ordered_sources = ["local", "local"]

        [evm.routes.dev]
        source_ref = "local"
        policy_id = "local"
    "#;
    let err = RuntimeConfig::from_str(config, RuntimeConfigFormat::Toml).expect_err("duplicate");
    assert_eq!(err.kind(), &RuntimeConfigErrorKind::DuplicatePolicySource);
}

#[test]
fn signer_shape_is_validated() {
    let valid = r#"
        [keystores.default]
        keystore_path = "/runtime/keystore.json"
        unlock_file = "/runtime/unlock"

        [signers.deployer]
        provider = "keystore"
        keystore_ref = "default"
        entry_id = "00000000-0000-0000-0000-000000000000"
    "#;
    let runtime = RuntimeConfig::from_str(valid, RuntimeConfigFormat::Toml).expect("valid");
    let signer_ref = SignerRef::new("deployer").expect("signer");
    let keystore_ref = KeystoreRef::new("default").expect("keystore");
    assert_eq!(
        runtime
            .signers()
            .get(&signer_ref)
            .expect("signer")
            .as_keystore()
            .entry_id()
            .to_string(),
        "00000000-0000-0000-0000-000000000000"
    );
    assert_eq!(
        runtime
            .signers()
            .get(&signer_ref)
            .expect("signer")
            .as_keystore()
            .keystore_ref(),
        &keystore_ref
    );
    assert_eq!(
        runtime
            .keystores()
            .get(&keystore_ref)
            .expect("keystore")
            .keystore_path()
            .expose_path(),
        std::path::Path::new("/runtime/keystore.json")
    );

    let unsupported = r#"
        [keystores.default]
        keystore_path = "/runtime/keystore.json"
        unlock_file = "/runtime/unlock"

        [signers.deployer]
        provider = "raw-private-key"
        keystore_ref = "default"
        entry_id = "00000000-0000-0000-0000-000000000000"
    "#;
    let err =
        RuntimeConfig::from_str(unsupported, RuntimeConfigFormat::Toml).expect_err("provider");
    assert_eq!(
        err.kind(),
        &RuntimeConfigErrorKind::UnsupportedSignerProvider
    );

    let invalid_entry = valid.replace("00000000-0000-0000-0000-000000000000", "not-a-uuid");
    let err =
        RuntimeConfig::from_str(&invalid_entry, RuntimeConfigFormat::Toml).expect_err("entry");
    assert_eq!(err.kind(), &RuntimeConfigErrorKind::InvalidEntryId);
}

#[test]
fn route_only_evm_requirement_ignores_unused_malformed_signers() {
    let config = r#"
        [evm.sources.local]
        rpc_url = "http://127.0.0.1:8545"

        [evm.routes.dev]
        source_ref = "local"

        [signers.deployer]
        provider = "raw-private-key"
        entry_id = "not-a-uuid"
        private_key = "placeholder-private-key-value"
    "#;

    let runtime = RuntimeConfig::from_str_with_requirements(
        config,
        RuntimeConfigFormat::Toml,
        RuntimeConfigRequirement::evm(),
    )
    .expect("route-only EVM config");
    assert!(runtime.signers().is_empty());
}

#[test]
fn signer_requirement_rejects_malformed_signers() {
    let config = r#"
        [evm.sources.local]
        rpc_url = "http://127.0.0.1:8545"

        [evm.routes.dev]
        source_ref = "local"

        [keystores.default]
        keystore_path = "/runtime/keystore.json"
        unlock_file = "/runtime/unlock"

        [signers.deployer]
        provider = "raw-private-key"
        keystore_ref = "default"
        entry_id = "00000000-0000-0000-0000-000000000000"
    "#;

    let err = RuntimeConfig::from_str_with_requirements(
        config,
        RuntimeConfigFormat::Toml,
        RuntimeConfigRequirement::evm_with_signers(),
    )
    .expect_err("signer-required EVM config");
    assert_eq!(
        err.kind(),
        &RuntimeConfigErrorKind::UnsupportedSignerProvider
    );
}

#[test]
fn secret_material_fields_are_rejected() {
    for field in [
        "password",
        "private_key",
        "mnemonic",
        "signed_material",
        "raw_transaction",
    ] {
        let config = format!(
            r#"
            [keystores.default]
            keystore_path = "/runtime/keystore.json"
            unlock_file = "/runtime/unlock"

            [signers.deployer]
            provider = "keystore"
            keystore_ref = "default"
            entry_id = "00000000-0000-0000-0000-000000000000"
            {field} = "secret-value"
            "#
        );
        let err = RuntimeConfig::from_str(&config, RuntimeConfigFormat::Toml)
            .expect_err("forbidden field");
        assert_eq!(err.kind(), &RuntimeConfigErrorKind::ForbiddenSecretMaterial);
    }
}

#[test]
fn source_level_expected_chain_id_is_rejected() {
    let config = r#"
        [evm.sources.local]
        rpc_url = "http://127.0.0.1:8545"
        expected_chain_id = 31337
    "#;
    let err = RuntimeConfig::from_str(config, RuntimeConfigFormat::Toml).expect_err("chain id");
    assert_eq!(
        err.kind(),
        &RuntimeConfigErrorKind::ForbiddenExpectedChainId
    );
}

#[test]
fn whole_required_family_rejects_unused_malformed_entries() {
    let config = r#"
        [evm.sources.good]
        rpc_url = "http://127.0.0.1:8545"

        [evm.sources.unused-bad]
        rpc_url = "https://user:pass@example.invalid"

        [evm.routes.dev]
        source_ref = "good"
    "#;
    let err = RuntimeConfig::from_str_with_requirements(
        config,
        RuntimeConfigFormat::Toml,
        RuntimeConfigRequirement::evm(),
    )
    .expect_err("unused malformed source");
    assert_eq!(err.kind(), &RuntimeConfigErrorKind::UrlUserInfo);
}

#[test]
fn diagnostics_are_closed_and_redacted() {
    let _env = locked_env([(
        "MFM_RUNTIME_CONFIG_TEST_SECRET_PATH",
        "/tmp/mfm-runtime-config-secret-path",
    )]);
    let dir = tempdir().expect("tempdir");
    let runtime_path = dir.path().join("runtime-secret.toml");
    let config = r#"
        [evm.sources.secret-source]
        rpc_url_file_env = "MFM_RUNTIME_CONFIG_TEST_SECRET_PATH"
        auth_header = "Bearer should-not-leak"
    "#;
    fs::write(&runtime_path, config).expect("runtime config");

    let err = RuntimeConfig::load_path(&runtime_path).expect_err("redacted error");
    assert_error_redacted(&err);

    let userinfo = r#"
        [evm.sources.secret-source]
        rpc_url = "https://user:pass@example.invalid"
        auth_header = "Bearer should-not-leak"
    "#;
    let err = RuntimeConfig::from_str(userinfo, RuntimeConfigFormat::Toml).expect_err("userinfo");
    assert_error_redacted(&err);

    let signer_secret = r#"
        [keystores.default]
        keystore_path = "/very/secret/keystore.json"
        unlock_file = "/very/secret/unlock-file"

        [signers.deployer]
        provider = "keystore"
        keystore_ref = "default"
        entry_id = "00000000-0000-0000-0000-000000000000"
        private_key = "placeholder-private-key-value"
    "#;
    let err =
        RuntimeConfig::from_str(signer_secret, RuntimeConfigFormat::Toml).expect_err("secret");
    assert_error_redacted(&err);
}

fn assert_value(
    value: &RuntimeSecretValue,
    expected_value: &str,
    expected_kind: RuntimeValueSourceKind,
) {
    assert_eq!(value.expose_secret(), expected_value);
    assert_eq!(value.source_kind(), expected_kind);
}

fn assert_redacted(message: &str) {
    for forbidden in [
        "runtime-secret.toml",
        "/tmp/mfm-runtime-config-secret-path",
        "https://user:pass@example.invalid",
        "user:pass",
        "Bearer should-not-leak",
        "/very/secret/keystore.json",
        "/very/secret/unlock-file",
        "placeholder-private-key-value",
    ] {
        assert!(
            !message.contains(forbidden),
            "diagnostic leaked {forbidden}: {message}"
        );
    }
}

fn assert_error_redacted(error: &(impl std::fmt::Debug + std::fmt::Display)) {
    assert_redacted(&error.to_string());
    assert_redacted(&format!("{error:?}"));
}

fn locked_env<const N: usize>(pairs: [(&'static str, &str); N]) -> EnvGuard {
    let guard = ENV_LOCK.lock().expect("env lock");
    let mut keys = Vec::new();
    for (key, value) in pairs {
        set_env(key, value);
        keys.push(key);
    }
    EnvGuard {
        _guard: guard,
        keys,
    }
}

struct EnvGuard {
    _guard: std::sync::MutexGuard<'static, ()>,
    keys: Vec<&'static str>,
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        for key in &self.keys {
            remove_env(key);
        }
    }
}

fn set_env(key: &str, value: &str) {
    // SAFETY: these tests serialize environment mutation through ENV_LOCK and
    // clear each variable before releasing that lock.
    unsafe { std::env::set_var(key, value) };
}

fn remove_env(key: &str) {
    // SAFETY: these tests serialize environment mutation through ENV_LOCK and
    // clear each variable before releasing that lock.
    unsafe { std::env::remove_var(key) };
}
