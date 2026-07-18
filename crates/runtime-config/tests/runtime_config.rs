use std::fs;
use std::sync::Mutex;

use mfm_ids::LocalPublicId;
use mfm_runtime_config::{
    RuntimeConfig, RuntimeConfigErrorKind, RuntimeConfigFormat, RuntimeConfigRequirement,
    RuntimeSecretValue, RuntimeValueSourceKind,
};
use mfm_signing::SignerRef;
use tempfile::tempdir;

static ENV_LOCK: Mutex<()> = Mutex::new(());

#[test]
fn toml_and_json_direct_routes_match() {
    let toml = r#"
        [evm.routes.dev]
        source_ref = "local"
        rpc_url = "http://127.0.0.1:8545"
    "#;
    let json = r#"
        {"evm":{"routes":{"dev":{
          "source_ref":"local",
          "rpc_url":"http://127.0.0.1:8545"
        }}}}
    "#;
    let from_toml = RuntimeConfig::from_str(toml, RuntimeConfigFormat::Toml).expect("TOML");
    let from_json = RuntimeConfig::from_str(json, RuntimeConfigFormat::Json).expect("JSON");
    assert_eq!(from_toml, from_json);

    let route = from_toml
        .evm()
        .expect("EVM")
        .route(&LocalPublicId::new("dev").expect("network"))
        .expect("route");
    assert_eq!(route.source_ref().as_str(), "local");
    assert_value(
        route.rpc_url(),
        "http://127.0.0.1:8545",
        RuntimeValueSourceKind::Direct,
    );
}

#[test]
fn exact_route_selection_ignores_every_unrelated_malformed_entry() {
    let config = r#"
        [evm.routes.dev]
        source_ref = "local"
        rpc_url = "http://127.0.0.1:8545"

        [evm.routes.unused]
        source_ref = "bad/source"
        rpc_url = "https://user:pass@example.invalid"
        unknown = true

        [signers.unused]
        provider = "raw-private-key"
        entry_id = "not-a-uuid"
        private_key = "must-not-be-read"

        [keystores.unused]
        malformed = true
    "#;
    let route = RuntimeConfig::evm_route_from_str(
        config,
        RuntimeConfigFormat::Toml,
        &LocalPublicId::new("dev").expect("network"),
    )
    .expect("exact route");
    assert_eq!(route.source_ref().as_str(), "local");

    let full = RuntimeConfig::from_str(config, RuntimeConfigFormat::Toml)
        .expect_err("full parse validates every present family");
    assert!(matches!(
        full.kind(),
        RuntimeConfigErrorKind::InvalidIdentifier { .. }
            | RuntimeConfigErrorKind::UnknownField
            | RuntimeConfigErrorKind::UrlUserInfo
    ));
}

#[test]
fn exact_signer_selection_ignores_unrelated_signers_keystores_and_routes() {
    let config = r#"
        [keystores.primary]
        keystore_path = "/run/mfm/wallet.keystore"
        unlock_file = "/run/mfm/wallet.password"

        [keystores.unused]
        malformed = true

        [signers.deployer]
        provider = "keystore"
        keystore_ref = "primary"
        entry_id = "67e55044-10b1-426f-9247-bb680e5fe0c8"

        [signers.unused]
        provider = "raw-private-key"
        private_key = "must-not-be-read"

        [evm.routes.unused]
        malformed = true
    "#;
    let signer_ref = SignerRef::new("deployer").expect("signer");
    let binding =
        RuntimeConfig::signer_binding_from_str(config, RuntimeConfigFormat::Toml, &signer_ref)
            .expect("exact signer");
    assert_eq!(binding.signer_ref(), &signer_ref);
    assert_eq!(
        binding.signer().entry_id().to_string(),
        "67e55044-10b1-426f-9247-bb680e5fe0c8"
    );
    assert_eq!(
        binding.keystore().keystore_path().expose_path(),
        std::path::Path::new("/run/mfm/wallet.keystore")
    );
}

#[test]
fn exact_selection_requires_requested_route_signer_and_referenced_keystore() {
    let empty = "";
    let route_error = RuntimeConfig::evm_route_from_str(
        empty,
        RuntimeConfigFormat::Toml,
        &LocalPublicId::new("dev").expect("network"),
    )
    .expect_err("missing EVM family");
    assert_eq!(route_error.kind(), &RuntimeConfigErrorKind::MissingFamily);

    let signer_error = RuntimeConfig::signer_binding_from_str(
        "[signers.other]\nprovider='keystore'\nkeystore_ref='missing'\nentry_id='67e55044-10b1-426f-9247-bb680e5fe0c8'",
        RuntimeConfigFormat::Toml,
        &SignerRef::new("deployer").expect("signer"),
    )
    .expect_err("missing exact signer");
    assert_eq!(signer_error.kind(), &RuntimeConfigErrorKind::MissingSigner);

    let keystore_error = RuntimeConfig::signer_binding_from_str(
        "[signers.deployer]\nprovider='keystore'\nkeystore_ref='missing'\nentry_id='67e55044-10b1-426f-9247-bb680e5fe0c8'",
        RuntimeConfigFormat::Toml,
        &SignerRef::new("deployer").expect("signer"),
    )
    .expect_err("missing keystore family");
    assert_eq!(
        keystore_error.kind(),
        &RuntimeConfigErrorKind::MissingFamily
    );
}

#[test]
fn route_value_sources_resolve_direct_env_file_and_file_env() {
    let dir = tempdir().expect("tempdir");
    let rpc_file = dir.path().join("rpc-url");
    let file_env_target = dir.path().join("file-env-target");
    fs::write(&rpc_file, "http://127.0.0.1:8547\n").expect("RPC file");
    fs::write(&file_env_target, "http://127.0.0.1:8548\n").expect("file env target");
    let _env = locked_env([
        ("MFM_RUNTIME_CONFIG_TEST_RPC_ENV", "http://127.0.0.1:8546"),
        (
            "MFM_RUNTIME_CONFIG_TEST_FILE_ENV",
            file_env_target.to_str().expect("UTF-8 path"),
        ),
        ("MFM_RUNTIME_CONFIG_TEST_AUTH_ENV", "Bearer env-token"),
    ]);
    let config = format!(
        r#"
        [evm.routes.direct]
        source_ref = "direct"
        rpc_url = "http://127.0.0.1:8545"

        [evm.routes.env]
        source_ref = "env"
        rpc_url_env = "MFM_RUNTIME_CONFIG_TEST_RPC_ENV"
        auth_header_env = "MFM_RUNTIME_CONFIG_TEST_AUTH_ENV"

        [evm.routes.file]
        source_ref = "file"
        rpc_url_file = "{}"

        [evm.routes.file-env]
        source_ref = "file-env"
        rpc_url_file_env = "MFM_RUNTIME_CONFIG_TEST_FILE_ENV"
        "#,
        rpc_file.display()
    );
    let runtime = RuntimeConfig::from_str(&config, RuntimeConfigFormat::Toml).expect("runtime");
    let routes = runtime.evm().expect("EVM").routes();
    for (id, value, kind) in [
        (
            "direct",
            "http://127.0.0.1:8545",
            RuntimeValueSourceKind::Direct,
        ),
        ("env", "http://127.0.0.1:8546", RuntimeValueSourceKind::Env),
        (
            "file",
            "http://127.0.0.1:8547",
            RuntimeValueSourceKind::File,
        ),
        (
            "file-env",
            "http://127.0.0.1:8548",
            RuntimeValueSourceKind::FileEnv,
        ),
    ] {
        assert_value(
            routes
                .get(&LocalPublicId::new(id).expect("route id"))
                .expect("route")
                .rpc_url(),
            value,
            kind,
        );
    }
    assert_value(
        routes
            .get(&LocalPublicId::new("env").expect("route id"))
            .expect("route")
            .auth_header()
            .expect("authorization"),
        "Bearer env-token",
        RuntimeValueSourceKind::Env,
    );
}

#[test]
fn route_shape_and_urls_fail_closed() {
    for (name, route, expected) in [
        (
            "missing endpoint",
            "source_ref='local'",
            RuntimeConfigErrorKind::ExactlyOneValueSource,
        ),
        (
            "duplicate endpoint",
            "source_ref='local'\nrpc_url='http://127.0.0.1:8545'\nrpc_url_env='MISSING'",
            RuntimeConfigErrorKind::ExactlyOneValueSource,
        ),
        (
            "userinfo",
            "source_ref='local'\nrpc_url='https://user:pass@example.invalid'",
            RuntimeConfigErrorKind::UrlUserInfo,
        ),
        (
            "source chain",
            "source_ref='local'\nrpc_url='http://127.0.0.1:8545'\nexpected_chain_id=1",
            RuntimeConfigErrorKind::ForbiddenExpectedChainId,
        ),
    ] {
        let config = format!("[evm.routes.dev]\n{route}");
        let error = RuntimeConfig::from_str(&config, RuntimeConfigFormat::Toml).expect_err(name);
        assert_eq!(error.kind(), &expected, "{name}");
    }
}

#[test]
fn bitcoin_runtime_and_auth_contract_are_unchanged() {
    let config = r#"
        [btc.routes.public-bitcoin-core]
        rpc_url = "http://127.0.0.1:8332"
        rpc_user = "rpc-user"
        rpc_password = "rpc-pass"
    "#;
    let runtime = RuntimeConfig::from_str(config, RuntimeConfigFormat::Toml).expect("runtime");
    let route = runtime
        .btc()
        .expect("BTC")
        .routes()
        .get(&"public-bitcoin-core".parse().expect("source"))
        .expect("route");
    assert_value(
        route.rpc_url(),
        "http://127.0.0.1:8332",
        RuntimeValueSourceKind::Direct,
    );

    let incomplete = "[btc.routes.node]\nrpc_url='http://127.0.0.1:8332'\nrpc_user='user'";
    let error = RuntimeConfig::from_str(incomplete, RuntimeConfigFormat::Toml)
        .expect_err("incomplete auth");
    assert_eq!(error.kind(), &RuntimeConfigErrorKind::IncompleteBasicAuth);
}

#[test]
fn required_full_families_must_exist() {
    for requirement in [
        RuntimeConfigRequirement::btc(),
        RuntimeConfigRequirement::evm(),
    ] {
        let error =
            RuntimeConfig::from_str_with_requirements("", RuntimeConfigFormat::Toml, requirement)
                .expect_err("missing family");
        assert_eq!(error.kind(), &RuntimeConfigErrorKind::MissingFamily);
    }
}

#[test]
fn full_signer_parse_and_secret_field_rejection_remain_strict() {
    let valid = r#"
        [keystores.default]
        keystore_path = "/runtime/keystore.json"
        unlock_file = "/runtime/unlock"

        [signers.deployer]
        provider = "keystore"
        keystore_ref = "default"
        entry_id = "00000000-0000-0000-0000-000000000000"
    "#;
    let runtime = RuntimeConfig::from_str(valid, RuntimeConfigFormat::Toml).expect("runtime");
    assert_eq!(runtime.signers().len(), 1);
    assert_eq!(runtime.keystores().len(), 1);

    for field in ["password", "private_key", "mnemonic", "raw_transaction"] {
        let config = valid.replace(
            "entry_id = \"00000000-0000-0000-0000-000000000000\"",
            &format!("entry_id = \"00000000-0000-0000-0000-000000000000\"\n{field} = \"secret\""),
        );
        let error = RuntimeConfig::from_str(&config, RuntimeConfigFormat::Toml)
            .expect_err("secret-shaped field");
        assert_eq!(
            error.kind(),
            &RuntimeConfigErrorKind::ForbiddenSecretMaterial
        );
    }
}

#[test]
fn shipped_runtime_examples_resolve() {
    let _env = locked_env([
        ("MFM_ETHEREUM_MAINNET_RPC_URL", "http://127.0.0.1:8545"),
        ("MFM_BITCOIN_RPC_URL", "http://127.0.0.1:8332"),
        ("MFM_BITCOIN_RPC_USER", "rpc-user"),
        ("MFM_BITCOIN_RPC_PASSWORD", "rpc-pass"),
    ]);
    for raw in [
        include_str!("../../../examples/configs/runtime-dual-mainnet.toml"),
        include_str!("../../../examples/configs/runtime-ethereum-mainnet.toml"),
    ] {
        let runtime = RuntimeConfig::from_str(raw, RuntimeConfigFormat::Toml).expect("example");
        assert_eq!(runtime.evm().expect("EVM").routes().len(), 1);
    }
}

#[test]
fn diagnostics_are_closed_and_redacted() {
    let config = r#"
        [evm.routes.secret]
        source_ref = "secret-source"
        rpc_url = "https://user:pass@example.invalid"
        auth_header = "Bearer should-not-leak"
    "#;
    let error = RuntimeConfig::from_str(config, RuntimeConfigFormat::Toml).expect_err("userinfo");
    let rendered = format!("{error:?} {error}");
    for forbidden in [
        "https://user:pass@example.invalid",
        "user:pass",
        "Bearer should-not-leak",
    ] {
        assert!(!rendered.contains(forbidden), "leaked {forbidden}");
    }
}

fn assert_value(
    value: &RuntimeSecretValue,
    expected_value: &str,
    expected_kind: RuntimeValueSourceKind,
) {
    assert_eq!(value.expose_secret(), expected_value);
    assert_eq!(value.source_kind(), expected_kind);
}

struct EnvGuard {
    names: Vec<&'static str>,
    _lock: std::sync::MutexGuard<'static, ()>,
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        for name in &self.names {
            std::env::remove_var(name);
        }
    }
}

fn locked_env<const N: usize>(values: [(&'static str, &str); N]) -> EnvGuard {
    let lock = ENV_LOCK.lock().expect("environment lock");
    let mut names = Vec::new();
    for (name, value) in values {
        std::env::set_var(name, value);
        names.push(name);
    }
    EnvGuard { names, _lock: lock }
}
