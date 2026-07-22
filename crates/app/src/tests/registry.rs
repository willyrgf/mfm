use super::*;

#[test]
fn production_semantic_registries_equal_the_published_catalog() {
    let catalog = production_authoring_catalog().expect("published production catalog");
    let certification =
        production_certification_registry().expect("production certification registry");
    certification
        .validate_authoring_catalog(&catalog)
        .expect("exact certification catalog coverage");
    replay_verifiers::ReplayVerifierRegistry::production()
        .validate_authoring_catalog(&catalog)
        .expect("exact replay catalog coverage");
}

#[tokio::test]
async fn production_runner_registry_defers_malformed_runtime_config() {
    let dir = tempfile::tempdir().expect("tempdir");
    let config_path = dir.path().join("runtime.toml");
    std::fs::write(&config_path, "not valid toml = [").expect("write runtime config");
    let store = store::AsyncInMemoryRunStore::default();

    production_runner_registry_for_test(Arc::new(store), Some(&config_path))
        .await
        .expect("runner registration must not parse live runtime config");
}

#[tokio::test]
async fn production_runner_registry_covers_runtime_family_matrix() {
    let cases = [
        ("portfolio-only", None, false, false),
        (
            "portfolio-btc",
            Some(
                r#"
[bitcoin.routes.public-bitcoin-core]
rpc_url = { direct = "http://127.0.0.1:8332" }
scan_timeout_seconds = 30
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
rpc_url = { direct = "http://127.0.0.1:8545" }
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
rpc_url = { direct = "http://127.0.0.1:8545" }

[bitcoin.routes.public-bitcoin-core]
rpc_url = { direct = "http://127.0.0.1:8332" }
scan_timeout_seconds = 30
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
        if let Some((_, path)) = &config_path {
            let bitcoin = runtime_config::load_bitcoin_route(
                path,
                &mfm_bitcoin::BitcoinSourceIdentity::new("public-bitcoin-core").expect("source"),
            );
            assert_eq!(bitcoin.is_ok(), expect_btc, "{name} Bitcoin family");
            let evm = runtime_config::load_evm_route(
                path,
                &LocalPublicId::new("ethereum-mainnet").expect("network"),
            );
            assert_eq!(evm.is_ok(), expect_evm, "{name} EVM family");
        } else {
            assert!(!expect_btc && !expect_evm);
        }

        let store = store::AsyncInMemoryRunStore::default();
        production_runner_registry_for_test(
            Arc::new(store),
            config_path.as_ref().map(|(_, path)| path.as_path()),
        )
        .await
        .unwrap_or_else(|error| panic!("{name} production registry: {error}"));
    }
}

#[test]
fn production_certification_registers_balance_read_but_not_other_evm_states() {
    let registry = production_certification_registry().expect("production certification registry");
    let production_digest = registry.digest().expect("production registry digest");

    let mut with_balance_read = registry.clone();
    with_balance_read
        .register_state::<mfm_evm::CollectEvmBalancesState>()
        .expect("balance read descriptor");
    assert_eq!(
        with_balance_read
            .digest()
            .expect("balance read registry digest"),
        production_digest,
        "portfolio composition must register its EVM balance read"
    );

    for (name, digest) in [
        ("validation", {
            let mut expanded = registry.clone();
            expanded
                .register_state::<mfm_evm::ValidateEvmContractState>()
                .expect("validation descriptor can be registered explicitly");
            expanded.digest().expect("validation registry digest")
        }),
        ("transaction", {
            let mut expanded = registry.clone();
            expanded
                .register_state::<mfm_evm::SubmitEvmTransactionState>()
                .expect("transaction descriptor can be registered explicitly");
            expanded.digest().expect("transaction registry digest")
        }),
    ] {
        assert_ne!(
            digest, production_digest,
            "production certification must not register the {name} foundation"
        );
    }
}
