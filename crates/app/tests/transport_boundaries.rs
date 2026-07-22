#![allow(clippy::disallowed_methods)]

#[test]
fn production_app_links_only_supported_transport_crates() {
    let manifest = include_str!("../Cargo.toml");
    for required in [
        "mfm-adapters-portfolio",
        "mfm-transports-evm",
        "mfm-adapters-btc-jsonrpc",
        "mfm-transports-btc-jsonrpc-http",
    ] {
        assert!(
            manifest.contains(required),
            "production app manifest must link {required}"
        );
    }
    for forbidden in [
        "mfm-transports-exec",
        "mfm-transports-local-evm",
        concat!("mfm-transports-evm-", "d", "cv"),
        concat!("mfm-transports-", "portfolio"),
        "mfm-transports-local-fs",
        "mfm-transports-local-keystore",
        "mfm-transports-rpc-control",
    ] {
        assert!(
            !manifest.contains(forbidden),
            "production app manifest must not link unsupported transport dependency {forbidden}"
        );
    }
}

#[test]
fn portfolio_live_source_excludes_artifact_write_surfaces() {
    let source = include_str!("../../adapters/portfolio/src/lib.rs");
    for banned in [
        concat!("request: serde_json::", "Value"),
        "PortfolioArtifactStore",
        "PortfolioArtifactStoreFuture",
        "put_artifact(",
        "put_verified_artifact(",
        "persist_artifact(",
    ] {
        assert!(
            !source.contains(banned),
            "portfolio live source must not expose artifact write surface {banned}"
        );
    }
}

#[test]
fn transport_provider_boundaries_expose_only_checked_bound_sessions() {
    let evm_capabilities = include_str!("../../evm-capabilities/src/lib.rs");
    assert!(evm_capabilities.contains("EvmReadCapability"));
    assert!(evm_capabilities.contains("pub trait EvmReadSession"));
    assert!(evm_capabilities.contains("EvmTransactionCapability"));
    assert!(evm_capabilities.contains("pub trait EvmTransactionSession"));
    let evm_transport = include_str!("../../transports/evm/src/lib.rs");
    assert!(
        evm_transport.contains("pub struct EvmJsonRpcSession")
            && evm_transport.contains("impl EvmReadSession for EvmJsonRpcSession")
            && evm_transport.contains("impl EvmTransactionSession for EvmJsonRpcSession"),
        "EVM transport must expose the two views on one bound session"
    );
    let chain_identity_probes = evm_transport
        .match_indices(".rpc_call")
        .filter(|(offset, _)| function_args(evm_transport, *offset).contains("\"eth_chainId\""))
        .count();
    assert_eq!(
        chain_identity_probes, 1,
        "a session must probe chain identity exactly once while binding"
    );
    let evm_adapter = include_str!("../../adapters/evm/src/lib.rs");
    let app_evm = include_str!("../src/evm_runtime.rs");
    assert!(
        evm_adapter.contains("register_evm_transaction_runner")
            && evm_adapter.contains("register_evm_balance_runners")
            && evm_adapter.contains("register_evm_validation_runner")
            && evm_adapter.contains("CollectEvmBalancesState")
            && evm_adapter.contains("ValidateEvmContractState")
            && app_evm.contains("register_evm_balance_runners")
            && app_evm.contains("bind_evm_read_session"),
        "adapter foundations must remain explicit while app assembly selects EVM balance reads"
    );
    assert!(
        !app_evm.contains("register_evm_validation_runner")
            && !app_evm.contains("register_evm_transaction_runner")
            && !app_evm.contains("bind_evm_transaction_session"),
        "production app assembly must omit disconnected EVM validation and transaction runners"
    );

    let btc_transport = include_str!("../../transports/btc-jsonrpc-http/src/lib.rs");
    let bitcoin_capabilities = include_str!("../../btc-capabilities/src/lib.rs");
    let bitcoin_adapter = include_str!("../../adapters/btc-jsonrpc/src/lib.rs");
    let app_live = include_str!("../src/live_transports.rs");
    assert!(bitcoin_capabilities.contains("pub trait BitcoinBalanceSession"));
    assert!(bitcoin_capabilities.contains("pub struct BitcoinBalanceCollectionRequest"));
    assert!(
        btc_transport.contains("pub struct BitcoinRpcSession")
            && btc_transport.contains("impl BitcoinBalanceSession for BitcoinRpcSession"),
        "Bitcoin transport must expose one checked endpoint-bound aggregate session"
    );
    assert!(
        btc_transport.contains("async fn rpc<T>(")
            && !btc_transport.contains("pub async fn rpc<T>("),
        "raw Bitcoin RPC must remain private to the checked session"
    );
    let collection = btc_transport
        .split_once("async fn execute_collection")
        .expect("aggregate collection function")
        .1
        .split_once("async fn rpc<T>")
        .expect("private RPC helper")
        .0;
    let mut call_offsets = Vec::new();
    for method in ["getblockchaininfo", "scantxoutset", "getblockhash"] {
        assert_eq!(
            collection.matches(&format!("\"{method}\"")).count(),
            1,
            "aggregate session must have one logical {method} call site"
        );
        call_offsets.push(collection.find(method).expect("method call"));
    }
    assert!(call_offsets.windows(2).all(|pair| pair[0] < pair[1]));
    for forbidden_method in ["getblockheader", "\"status\"", "\"abort\""] {
        assert!(
            !collection.contains(forbidden_method),
            "aggregate session must not call {forbidden_method}"
        );
    }
    assert!(
        bitcoin_adapter.contains("CollectBitcoinBalancesState")
            && bitcoin_adapter.contains("register_bitcoin_jsonrpc_runners")
            && app_live.contains("impl BitcoinBalanceSession for LiveTransportRuntime")
            && app_live.contains("BitcoinRpcSession::new"),
        "adapter and app must retain one routed aggregate Bitcoin session boundary"
    );

    let app = include_str!("../src/services_read.rs");
    assert!(
        app.contains("ReplayVerifierRegistry::production().verify(&broker"),
        "app replay must use the compiled recorded-evidence verifier registry"
    );
}

fn function_args(source: &str, offset: usize) -> &str {
    let Some(open_relative) = source[offset..].find('(') else {
        return "";
    };
    let start = offset + open_relative + 1;
    let mut depth = 1_usize;
    for (relative, ch) in source[start..].char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return &source[start..start + relative];
                }
            }
            _ => {}
        }
    }
    &source[start..source.len().min(start + 240)]
}
