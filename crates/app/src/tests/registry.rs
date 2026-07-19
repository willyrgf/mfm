use super::*;

#[test]
fn production_runner_registry_defers_malformed_runtime_config() {
    let dir = tempfile::tempdir().expect("tempdir");
    let config_path = dir.path().join("runtime.toml");
    std::fs::write(&config_path, "not valid toml = [").expect("write runtime config");
    let store = store::AsyncInMemoryRunStore::default();

    production_runner_registry(
        Arc::new(store),
        crate::ProjectionFactIndexProvider::empty_arc(),
        Some(&config_path),
    )
    .expect("runner registration must not parse live runtime config");
}

#[test]
fn production_runner_registry_covers_runtime_family_matrix() {
    let cases = [
        ("portfolio-only", None, false, false),
        (
            "portfolio-btc",
            Some(
                r#"
[btc.routes.public-bitcoin-core]
rpc_url = "http://127.0.0.1:8332"
"#,
            ),
            true,
            false,
        ),
        (
            "portfolio-evm",
            Some(
                r#"
[evm.routes.ethereum-mainnet]
source_ref = "ethereum-mainnet"
rpc_url = "http://127.0.0.1:8545"
"#,
            ),
            false,
            true,
        ),
        (
            "portfolio-btc-evm",
            Some(
                r#"
[evm.routes.ethereum-mainnet]
source_ref = "ethereum-mainnet"
rpc_url = "http://127.0.0.1:8545"

[btc.routes.public-bitcoin-core]
rpc_url = "http://127.0.0.1:8332"
"#,
            ),
            true,
            true,
        ),
    ];

    for (name, raw_config, expect_btc, expect_evm) in cases {
        let config_path = raw_config.map(|raw| {
            let dir = tempfile::tempdir().expect("runtime config tempdir");
            let path = dir.path().join("runtime.toml");
            std::fs::write(&path, raw).expect("write runtime config");
            (dir, path)
        });
        let parsed = config_path.as_ref().map(|(_, path)| {
            mfm_runtime_config::RuntimeConfig::load_path(path)
                .unwrap_or_else(|error| panic!("{name} runtime config: {error}"))
        });
        assert_eq!(
            parsed.as_ref().and_then(|config| config.btc()).is_some(),
            expect_btc,
            "{name} Bitcoin family"
        );
        assert_eq!(
            parsed.as_ref().and_then(|config| config.evm()).is_some(),
            expect_evm,
            "{name} EVM family"
        );

        let store = store::AsyncInMemoryRunStore::default();
        production_runner_registry(
            Arc::new(store),
            crate::ProjectionFactIndexProvider::empty_arc(),
            config_path.as_ref().map(|(_, path)| path.as_path()),
        )
        .unwrap_or_else(|error| panic!("{name} production registry: {error}"));
    }
}
