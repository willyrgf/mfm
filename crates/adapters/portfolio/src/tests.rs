use super::*;

use std::{collections::BTreeMap, sync::Arc};

use mfm_btc_capabilities::{
    BtcBalanceReadResponse, BtcBlockHash, BtcChainHeadResponse, BtcSourceStatus,
    RedactedBtcSourceEvidence,
};
use mfm_evm_capabilities::{
    EvmBalanceReadResponse, EvmBlockReadResponse, EvmCallReadResponse, EvmCapabilityFuture,
};
use mfm_portfolio_model::metadata::PublicMetadata;
use mfm_portfolio_model::portfolio::NetworkFamilyConfig;
use mfm_portfolio_model::symbol::{
    QuoteCode, QuoteValuationConfig, SymbolConfig, SymbolKind, SymbolRole, SymbolValuationConfig,
    ValuationReaderConfig,
};
use mfm_portfolio_model::wallet::{
    WalletConfig, WalletImplementationConfig, WalletSubject, WalletSubjectKind,
};

#[test]
fn executable_identity_summary_matches_golden() {
    assert_eq!(
        executable_identity_summary([READ_FACTORY, PURE_FACTORY]),
        [
            "factory=read_external;cargo_digest=content:sha256-jcs-v1:9cee33a7e03231e1baeb99725c9724361ba5763bc7f3cbe11698fa96696e6f1a;binary_digest=content:sha256-jcs-v1:ded559fbdf36801d1e3c95838014c5e7b94e50655014d7ddc612f288a508988e;nix_derivation=false;nix_output=false",
            "factory=pure;cargo_digest=content:sha256-jcs-v1:9cee33a7e03231e1baeb99725c9724361ba5763bc7f3cbe11698fa96696e6f1a;binary_digest=content:sha256-jcs-v1:ded559fbdf36801d1e3c95838014c5e7b94e50655014d7ddc612f288a508988e;nix_derivation=false;nix_output=false",
        ]
    );
}

#[test]
fn bitcoin_capability_backend_observes_native_balance_with_anchor() {
    let (network, config) = bitcoin_observe_config();
    let btc = Arc::new(MockPortfolioBtc::matching());
    let backend = CapabilityPortfolioBackend::new(Arc::new(UnavailablePortfolioEvm), Some(btc));

    let anchor = poll_ready(backend.read_execution_anchor(&network)).expect("chain head");
    let balance = poll_ready(backend.read_raw_balance(&config, &anchor)).expect("balance");

    assert_eq!(
        anchor,
        ExecutionAnchor::Bitcoin {
            height: 850_000,
            block_hash: BTC_HASH.to_owned()
        }
    );
    assert_eq!(balance.raw, U256::from(123_456_789_u64));
    assert_eq!(balance.decimals, 8);
    assert_eq!(
        balance.anchor,
        Some(ExecutionAnchor::Bitcoin {
            height: 850_000,
            block_hash: BTC_HASH.to_owned()
        })
    );
}

#[test]
fn bitcoin_capability_backend_rejects_balance_anchor_drift() {
    let (network, config) = bitcoin_observe_config();
    let btc = Arc::new(MockPortfolioBtc::mismatched_balance_anchor());
    let backend = CapabilityPortfolioBackend::new(Arc::new(UnavailablePortfolioEvm), Some(btc));

    let anchor = poll_ready(backend.read_execution_anchor(&network)).expect("chain head");
    let error =
        poll_ready(backend.read_raw_balance(&config, &anchor)).expect_err("anchor mismatch");

    assert_eq!(error.code, "observation_anchor_mismatch");
    assert!(error.message.contains("pinned execution anchor"));
    assert!(error.message.contains("height=850000"));
    assert!(error.message.contains("height=850001"));
    assert!(error.message.contains(BTC_HASH));
    assert!(error.message.contains(BTC_NEXT_HASH));
}

#[test]
fn bitcoin_provider_diagnostic_maps_to_snapshot_error_code_and_details() {
    let diagnostic = mfm_btc_capabilities::btc_diagnostic(
        mfm_capabilities::ProviderDiagnosticCode::RpcHttpStatus,
    )
    .with_operation(diagnostic_id("scantxoutset"))
    .with_field(
        diagnostic_id("http_status"),
        mfm_capabilities::ProviderDiagnosticValue::U64(403),
    );

    let error = portfolio_btc_capability_error(BtcCapabilityError::provider_failure(diagnostic));

    assert_eq!(error.code, "bitcoin_rpc_http_status");
    assert_eq!(
        error.message,
        "portfolio Bitcoin read capability failed: rpc_http_status operation=scantxoutset http_status=403"
    );
    assert_eq!(
        error.redacted_details,
        Some(serde_json::json!({
            "diagnostic_kind": "provider_failure",
            "provider_family": "bitcoin",
            "code": "rpc_http_status",
            "operation": "scantxoutset",
            "fields": {
                "http_status": 403,
            },
        }))
    );
    assert!(!error.fatal_attempt_failure);
}

#[test]
fn source_mismatch_diagnostics_are_attempt_failures_for_each_provider_family() {
    let btc_error = portfolio_btc_capability_error(BtcCapabilityError::SourceMismatch {
        diagnostic: mfm_btc_capabilities::btc_diagnostic(
            mfm_capabilities::ProviderDiagnosticCode::SourceMismatch,
        ),
    });
    let evm_error = portfolio_evm_capability_error(EvmCapabilityError::SourceMismatch {
        diagnostic: mfm_evm_capabilities::evm_diagnostic(
            mfm_capabilities::ProviderDiagnosticCode::SourceMismatch,
        ),
    });

    assert_eq!(btc_error.code, "bitcoin_source_mismatch");
    assert_eq!(evm_error.code, "evm_source_mismatch");
    assert_eq!(
        btc_error.redacted_details.as_ref().expect("btc details")["diagnostic_kind"],
        "provider_source_mismatch"
    );
    assert_eq!(
        evm_error.redacted_details.as_ref().expect("evm details")["diagnostic_kind"],
        "provider_source_mismatch"
    );
    assert!(btc_error.fatal_attempt_failure);
    assert!(evm_error.fatal_attempt_failure);
}

fn bitcoin_observe_config() -> (NetworkConfig, ObserveBatchConfig) {
    let network = NetworkConfig::new(
        "bitcoin-mainnet".to_owned(),
        NetworkFamilyConfig::Bitcoin,
        None,
        "bitcoin-mainnet".to_owned(),
        BTreeMap::new(),
    )
    .expect("network");
    let wallet = WalletConfig {
        wallet_id: "wallet_btc_mainnet".parse().expect("wallet id"),
        subject: WalletSubject::new(
            "bc1qns9f7yfx3ry9lj6yz7c9er0vwa0ye2eklpzqfw",
            WalletSubjectKind::BitcoinAddress,
        )
        .expect("wallet subject"),
        network_id: "bitcoin-mainnet".parse().expect("network id"),
        implementation: WalletImplementationConfig::AddressOnly {},
        symbol_ids: vec!["btc.native.bitcoin-mainnet".parse().expect("symbol id")],
        metadata: PublicMetadata::default(),
    };
    let symbol = SymbolConfig {
        symbol_id: "btc.native.bitcoin-mainnet".parse().expect("symbol id"),
        display_symbol: Some("BTC".to_owned()),
        kind: SymbolKind::NativeBalance,
        role: SymbolRole::Native,
        network_id: "bitcoin-mainnet".parse().expect("network id"),
        protocol: None,
        balance_reader: BalanceReaderConfig::NativeBalance {},
        valuation: SymbolValuationConfig {
            quotes: vec![QuoteValuationConfig {
                quote: QuoteCode::Usd,
                priced_symbol_id: "btc.native.bitcoin-mainnet".parse().expect("priced symbol"),
                reader: ValuationReaderConfig::FixedUnitPrice {
                    unit_price_dec: "0.00".parse().expect("unit price"),
                },
            }],
        },
        decimals: Some(8),
        underlying_symbol_id: None,
        metadata: PublicMetadata::default(),
    };
    let config = ObserveBatchConfig::new(wallet, symbol, network.clone()).expect("observe config");
    (network, config)
}

fn diagnostic_id(value: &str) -> mfm_ids::LocalPublicId {
    mfm_ids::LocalPublicId::new(value).expect("test diagnostic id")
}

fn poll_ready<F>(future: F) -> F::Output
where
    F: std::future::Future,
{
    let waker = std::task::Waker::noop();
    let mut cx = std::task::Context::from_waker(waker);
    let mut future = std::pin::pin!(future);
    match std::future::Future::poll(future.as_mut(), &mut cx) {
        std::task::Poll::Ready(output) => output,
        std::task::Poll::Pending => panic!("test future unexpectedly pending"),
    }
}

const BTC_HASH: &str = "00000000000000000001b2a7f3e0d5c4b6a897887766554433221100ffeeddcc";
const BTC_NEXT_HASH: &str = "00000000000000000002b2a7f3e0d5c4b6a897887766554433221100ffeeddcc";

struct MockPortfolioBtc {
    balance_height: u64,
    balance_hash: &'static str,
}

impl MockPortfolioBtc {
    fn matching() -> Self {
        Self {
            balance_height: 850_000,
            balance_hash: BTC_HASH,
        }
    }

    fn mismatched_balance_anchor() -> Self {
        Self {
            balance_height: 850_001,
            balance_hash: BTC_NEXT_HASH,
        }
    }
}

impl BtcChainHeadReadProvider for MockPortfolioBtc {
    fn read_chain_head<'a>(
        &'a self,
        request: &'a BtcChainHeadRequest,
    ) -> mfm_btc_capabilities::BtcCapabilityFuture<'a, BtcChainHeadResponse> {
        Box::pin(async move {
            Ok(BtcChainHeadResponse {
                evidence: RedactedBtcSourceEvidence::from_request(
                    request,
                    Some("main".to_owned()),
                    BtcSourceStatus::Synced,
                ),
                block_height: 850_000,
                block_hash: BtcBlockHash::new(BTC_HASH).expect("hash"),
                provider_time_unix_ms: None,
            })
        })
    }
}

impl BtcBalanceReadProvider for MockPortfolioBtc {
    fn read_balance<'a>(
        &'a self,
        request: &'a BtcBalanceReadRequest,
    ) -> mfm_btc_capabilities::BtcCapabilityFuture<'a, BtcBalanceReadResponse> {
        let balance_height = self.balance_height;
        let balance_hash = self.balance_hash;
        Box::pin(async move {
            Ok(BtcBalanceReadResponse {
                evidence: RedactedBtcSourceEvidence::from_request(
                    &BtcChainHeadRequest {
                        guard: request.guard.clone(),
                        selection: request.selection,
                    },
                    Some("main".to_owned()),
                    BtcSourceStatus::Synced,
                ),
                address: request.address.clone(),
                balance_sats: 123_456_789,
                block_height: balance_height,
                block_hash: BtcBlockHash::new(balance_hash).expect("hash"),
            })
        })
    }
}

struct UnavailablePortfolioEvm;

fn unavailable_evm_error() -> EvmCapabilityError {
    EvmCapabilityError::provider_failure(mfm_evm_capabilities::evm_diagnostic(
        mfm_capabilities::ProviderDiagnosticCode::SourceUnavailable,
    ))
}

impl EvmBlockReadProvider for UnavailablePortfolioEvm {
    fn read_block<'a>(
        &'a self,
        _request: &'a EvmBlockReadRequest,
    ) -> EvmCapabilityFuture<'a, EvmBlockReadResponse> {
        Box::pin(async { Err(unavailable_evm_error()) })
    }
}

impl EvmBalanceReadProvider for UnavailablePortfolioEvm {
    fn read_balance<'a>(
        &'a self,
        _request: &'a EvmBalanceReadRequest,
    ) -> EvmCapabilityFuture<'a, EvmBalanceReadResponse> {
        Box::pin(async { Err(unavailable_evm_error()) })
    }
}

impl EvmCallReadProvider for UnavailablePortfolioEvm {
    fn read_call<'a>(
        &'a self,
        _request: &'a EvmCallReadRequest,
    ) -> EvmCapabilityFuture<'a, EvmCallReadResponse> {
        Box::pin(async { Err(unavailable_evm_error()) })
    }
}

#[test]
fn decode_replay_config_rejects_invalid_serialized_config() {
    let error = decode_replay_config::<ProjectReportConfig>(br#"{"report_version":0}"#)
        .expect_err("zero report version must fail decoding");
    assert!(error.to_string().contains("nonzero u64"));
}

fn executable_identity_summary(factories: [&str; 2]) -> Vec<String> {
    let executable_identities = RunnerExecutableIdentityTemplate::new(
        "mfm-adapters-portfolio",
        "typed-portfolio",
        env!("CARGO_PKG_VERSION"),
    )
    .expect("executable identity template");
    factories
        .into_iter()
        .map(|factory| {
            let identity = executable_identities
                .executable(events::RunnerFactoryId::new(factory).expect("factory id"));
            format!(
                "factory={};cargo_digest={};binary_digest={};nix_derivation={};nix_output={}",
                identity.factory_id,
                identity.cargo_package_digest,
                identity.binary_digest,
                identity.nix_derivation_hash.is_some(),
                identity.nix_output_hash.is_some()
            )
        })
        .collect()
}
