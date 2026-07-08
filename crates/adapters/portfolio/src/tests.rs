use super::*;

use std::{collections::BTreeMap, sync::Arc};

use mfm_btc_capabilities::{
    BtcBalanceReadResponse, BtcBlockHash, BtcChainHeadResponse, BtcSourceBinding, BtcSourceStatus,
    RedactedBtcSourceEvidence,
};
use mfm_evm_capabilities::{
    EvmBalanceReadResponse, EvmBlockReadResponse, EvmCallReadResponse, EvmCapabilityFuture,
};
use mfm_portfolio_model::metadata::PublicMetadata;
use mfm_portfolio_model::portfolio::{NetworkConfig, NetworkFamilyConfig};
use mfm_portfolio_model::symbol::{
    BalanceReaderConfig, QuoteCode, QuoteValuationConfig, SymbolConfig, SymbolKind, SymbolRole,
    SymbolValuationConfig, ValuationReaderConfig,
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

    let anchor =
        poll_ready(backend.read_execution_anchor(&network_read_intent_for_network(&network)))
            .expect("chain head");
    let intent = observe_batch_read_intent(&config, &anchor).expect("balance intent");
    let balance = poll_ready(backend.read_raw_balance(&intent)).expect("balance");

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
fn bitcoin_capability_backend_maps_provider_source_mismatch() {
    let (network, config) = bitcoin_observe_config();
    let btc = Arc::new(MockPortfolioBtc::rejecting_balance());
    let backend = CapabilityPortfolioBackend::new(Arc::new(UnavailablePortfolioEvm), Some(btc));

    let anchor =
        poll_ready(backend.read_execution_anchor(&network_read_intent_for_network(&network)))
            .expect("chain head");
    let intent = observe_batch_read_intent(&config, &anchor).expect("balance intent");
    let error =
        poll_ready(backend.read_raw_balance(&intent)).expect_err("capability source mismatch");

    assert_eq!(error.code, "bitcoin_source_mismatch");
    assert!(error.fatal_attempt_failure);
    assert_eq!(
        error.redacted_details,
        Some(serde_json::json!({
            "diagnostic_kind": "provider_source_mismatch",
            "provider_family": "bitcoin",
            "code": "source_mismatch",
            "operation": null,
            "fields": {
                "network_id": "bitcoin-mainnet",
                "source_identity": "bitcoin-mainnet",
                "bitcoin_network": "main",
                "observed_bitcoin_network": "main",
                "source_status": "synced",
            },
        }))
    );
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
        Some("main".to_owned()),
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
struct MockPortfolioBtc {
    reject_balance: bool,
}

impl MockPortfolioBtc {
    fn matching() -> Self {
        Self {
            reject_balance: false,
        }
    }

    fn rejecting_balance() -> Self {
        Self {
            reject_balance: true,
        }
    }
}

impl PortfolioBtcProviderFactory for MockPortfolioBtc {
    fn validate_btc_source_binding(&self, _binding: &BtcSourceBinding) -> mfm_runtime::Result<()> {
        Ok(())
    }

    fn bind_btc_source(
        &self,
        binding: BtcSourceBinding,
    ) -> mfm_btc_capabilities::Result<Arc<dyn PortfolioBtcProvider>> {
        Ok(Arc::new(MockPortfolioBoundBtc {
            binding,
            reject_balance: self.reject_balance,
        }))
    }
}

struct MockPortfolioBoundBtc {
    binding: BtcSourceBinding,
    reject_balance: bool,
}

impl BtcChainHeadReadProvider for MockPortfolioBoundBtc {
    fn read_chain_head<'a>(
        &'a self,
        request: &'a BtcChainHeadRequest,
    ) -> mfm_btc_capabilities::BtcCapabilityFuture<'a, BtcChainHeadResponse> {
        Box::pin(async move {
            Ok(BtcChainHeadResponse {
                evidence: RedactedBtcSourceEvidence::from_binding(
                    &self.binding,
                    "main",
                    BtcSourceStatus::Synced,
                )
                .expect("evidence"),
                head_kind: request.selection().head_kind(),
                finality: request.selection().finality(),
                block_height: 850_000,
                block_hash: BtcBlockHash::new(BTC_HASH).expect("hash"),
                provider_time_unix_ms: None,
            })
        })
    }
}

impl BtcBalanceReadProvider for MockPortfolioBoundBtc {
    fn read_balance<'a>(
        &'a self,
        request: &'a BtcBalanceReadRequest,
    ) -> mfm_btc_capabilities::BtcCapabilityFuture<'a, BtcBalanceReadResponse> {
        let reject_balance = self.reject_balance;
        Box::pin(async move {
            if reject_balance {
                return Err(BtcCapabilityError::SourceMismatch {
                    diagnostic: RedactedBtcSourceEvidence {
                        network_id: self.binding.network_id().clone(),
                        source_identity: self.binding.source_identity().clone(),
                        bitcoin_network: self.binding.bitcoin_network().as_str().to_owned(),
                        observed_bitcoin_network: self
                            .binding
                            .bitcoin_network()
                            .as_str()
                            .to_owned(),
                        source_status: BtcSourceStatus::Synced,
                    }
                    .source_mismatch_diagnostic(),
                });
            }
            Ok(BtcBalanceReadResponse {
                evidence: RedactedBtcSourceEvidence::from_binding(
                    &self.binding,
                    "main",
                    BtcSourceStatus::Synced,
                )
                .expect("evidence"),
                address: request.address().clone(),
                balance_sats: 123_456_789,
                block_height: request.block_height(),
                block_hash: request.block_hash().clone(),
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

impl PortfolioEvmProviderFactory for UnavailablePortfolioEvm {
    fn validate_evm_network_binding(
        &self,
        _binding: &EvmNetworkBinding,
    ) -> mfm_runtime::Result<()> {
        Ok(())
    }

    fn bind_evm_network(
        &self,
        _binding: EvmNetworkBinding,
    ) -> mfm_evm_capabilities::Result<Arc<dyn PortfolioEvmProvider>> {
        Ok(Arc::new(UnavailablePortfolioEvm))
    }
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
