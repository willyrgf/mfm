//! Adapter tests for the portfolio-owned EVM vertical slice.

use super::*;

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use alloy_primitives::{address, b256, Address, B256};
use mfm_evm_capabilities::{
    evm_diagnostic, EvmBlock, EvmCode, EvmSessionEvidence, EvmSessionFuture,
};
use mfm_fact_capabilities::{
    FactIndexReadBatchFuture, FactIndexReadProvider, FactIndexReadRequest,
};
use mfm_ids::LocalPublicId;
use mfm_portfolio_model::metadata::PublicMetadata;
use mfm_portfolio_model::portfolio::{NetworkConfig, NetworkFamilyConfig, PortfolioConfig};
use mfm_portfolio_model::symbol::{
    HoldingSourceConfig, QuoteCode, QuoteValuationConfig, SymbolConfig, SymbolValuationConfig,
};
use mfm_portfolio_model::wallet::{
    WalletConfig, WalletImplementationConfig, WalletSubject, WalletSubjectKind,
};
use mfm_program::{MfmFactType, ReadState, StateSpec, ValidatedConfig};
use mfm_state_portfolio::{
    reduce_evm_network_collection, CollectEvmNetworkInput, EvmBalanceSource,
    PortfolioCollectionReceipt, SelectHoldingsFactDescriptors,
};
use mfm_states_btc::BtcAddressBalanceSnapshotFact;

const ACCOUNT: Address = address!("000000000000000000000000000000000000dead");
const TOKEN: Address = address!("0000000000000000000000000000000000000001");
const ANCHOR_HASH: B256 = b256!("1111111111111111111111111111111111111111111111111111111111111111");
const REORG_HASH: B256 = b256!("2222222222222222222222222222222222222222222222222222222222222222");

#[derive(Clone, Debug, PartialEq, Eq)]
enum ReadRecord {
    Block(EvmBlockSelector),
    Balance(Address, EvmBlockSelector),
    Call(Vec<u8>, EvmBlockSelector),
}

struct RecordingSession {
    evidence: EvmSessionEvidence,
    records: Mutex<Vec<ReadRecord>>,
    active: AtomicUsize,
    max_active: AtomicUsize,
    final_hash: B256,
}

impl RecordingSession {
    fn new(binding: &EvmNetworkBinding, final_hash: B256) -> Self {
        Self {
            evidence: EvmSessionEvidence::new(
                binding,
                LocalPublicId::new("primary").expect("source ref"),
                LocalPublicId::new(EVM_JSONRPC_SESSION_IMPLEMENTATION_ID)
                    .expect("implementation id"),
            ),
            records: Mutex::new(Vec::new()),
            active: AtomicUsize::new(0),
            max_active: AtomicUsize::new(0),
            final_hash,
        }
    }

    fn record(&self, record: ReadRecord) {
        self.records.lock().expect("records").push(record);
    }

    fn records(&self) -> Vec<ReadRecord> {
        self.records.lock().expect("records").clone()
    }

    async fn concurrent_read(&self) {
        let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.max_active.fetch_max(active, Ordering::SeqCst);
        tokio::task::yield_now().await;
        self.active.fetch_sub(1, Ordering::SeqCst);
    }
}

impl EvmReadSession for RecordingSession {
    fn evidence(&self) -> &EvmSessionEvidence {
        &self.evidence
    }

    fn read_block<'a>(&'a self, selector: &'a EvmBlockSelector) -> EvmSessionFuture<'a, EvmBlock> {
        self.record(ReadRecord::Block(selector.clone()));
        let result = match selector {
            EvmBlockSelector::Latest => Ok(EvmBlock {
                number: U256::from(100),
                hash: ANCHOR_HASH,
            }),
            EvmBlockSelector::Number(number) if *number == U256::from(100) => Ok(EvmBlock {
                number: U256::from(100),
                hash: self.final_hash,
            }),
            _ => Err(provider_failure()),
        };
        Box::pin(std::future::ready(result))
    }

    fn read_balance<'a>(
        &'a self,
        account: Address,
        block: &'a EvmBlockSelector,
    ) -> EvmSessionFuture<'a, U256> {
        let selector = block.clone();
        self.record(ReadRecord::Balance(account, selector));
        Box::pin(async move {
            self.concurrent_read().await;
            Ok(U256::from(42))
        })
    }

    fn read_code<'a>(
        &'a self,
        _address: Address,
        _block: &'a EvmBlockSelector,
    ) -> EvmSessionFuture<'a, EvmCode> {
        Box::pin(std::future::ready(Err(provider_failure())))
    }

    fn call<'a>(&'a self, request: &'a EvmCall) -> EvmSessionFuture<'a, Bytes> {
        let input = request.input().to_vec();
        self.record(ReadRecord::Call(input.clone(), request.block().clone()));
        Box::pin(async move {
            self.concurrent_read().await;
            let value = if input.starts_with(&ERC20_DECIMALS_SELECTOR) {
                U256::from(6)
            } else if input.starts_with(&ERC20_BALANCE_OF_SELECTOR) {
                U256::from(99)
            } else {
                return Err(provider_failure());
            };
            let word = value.to_be_bytes::<32>();
            Ok(Bytes::copy_from_slice(&word))
        })
    }
}

struct EmptyFactIndex {
    calls: AtomicUsize,
}

impl EmptyFactIndex {
    fn new() -> Self {
        Self {
            calls: AtomicUsize::new(0),
        }
    }
}

impl FactIndexReadProvider for EmptyFactIndex {
    fn implementation_id(&self) -> &'static str {
        "mfm.adapters.portfolio.test.empty-fact-index"
    }

    fn read_fact_index_batch<'a>(
        &'a self,
        _requests: &'a [FactIndexReadRequest],
    ) -> FactIndexReadBatchFuture<'a> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(std::future::ready(Ok(Vec::new())))
    }
}

fn provider_failure() -> EvmCapabilityError {
    EvmCapabilityError::provider_failure(evm_diagnostic(
        ProviderDiagnosticCode::ProviderConfigurationInvalid,
    ))
}

fn network() -> NetworkConfig {
    NetworkConfig::new(
        "ethereum-mainnet".to_owned(),
        NetworkFamilyConfig::Evm,
        Some(1),
        Some(18),
        None,
        None,
        BTreeMap::new(),
    )
    .expect("EVM network")
}

fn source(account: Address, asset: EvmBalanceAsset) -> EvmBalanceSource {
    EvmBalanceSource::new(format!("{account:#x}"), asset).expect("EVM source")
}

fn plan_for(sources: Vec<EvmBalanceSource>) -> CollectEvmNetworkPlan {
    let config = EvmNetworkCollectionConfig::new(&network(), sources).expect("collection config");
    let state = <CollectEvmNetworkState as StateSpec>::new(
        ValidatedConfig::new(config).expect("validated config"),
    )
    .expect("collection state");
    state
        .plan(
            &CollectEvmNetworkInput::default(),
            &mfm_program::CertifiedContext::no_context(),
        )
        .expect("collection plan")
}

fn capabilities(
    session: Arc<RecordingSession>,
    binds: Arc<AtomicUsize>,
    fact_index: Arc<EmptyFactIndex>,
) -> PortfolioRunnerCapabilities {
    let artifacts: Arc<dyn store::RetainedArtifactReadProvider> =
        Arc::new(store::AsyncInMemoryRunStore::default());
    PortfolioRunnerCapabilities::new(
        artifacts,
        fact_index,
        |binding| {
            if binding.network_id().as_str() == "ethereum-mainnet"
                && binding.expected_chain_id() == 1
            {
                Ok(())
            } else {
                Err(provider_failure())
            }
        },
        move |_binding| {
            let session = Arc::clone(&session);
            let binds = Arc::clone(&binds);
            Box::pin(async move {
                binds.fetch_add(1, Ordering::SeqCst);
                Ok(session as Arc<dyn EvmReadSession>)
            })
        },
    )
}

#[test]
fn portfolio_runner_registration_keeps_factory_identity_explicit() {
    assert_eq!(PURE_FACTORY, "pure");
    assert_eq!(READ_FACTORY, "read_external");
    assert_eq!(MANAGED_WRITE_FACTORY, "managed_platform_write");
    assert_eq!(ADAPTER_FACTORY, "portfolio_adapter");
}

#[tokio::test]
async fn one_session_uses_latest_then_exact_hash_reads_then_number_recheck() {
    let plan = plan_for(vec![
        source(ACCOUNT, EvmBalanceAsset::Native),
        source(
            ACCOUNT,
            EvmBalanceAsset::erc20(format!("{TOKEN:#x}")).expect("token asset"),
        ),
    ]);
    let binding = plan.binding().expect("binding");
    let session = Arc::new(RecordingSession::new(&binding, ANCHOR_HASH));
    let binds = Arc::new(AtomicUsize::new(0));
    let fact_index = Arc::new(EmptyFactIndex::new());
    let capabilities = capabilities(Arc::clone(&session), Arc::clone(&binds), fact_index);

    let evidence = collect_evm_network(&plan, &capabilities)
        .await
        .expect("collection evidence");
    let batch = reduce_evm_network_collection(&plan, &evidence).expect("reduced evidence");
    assert_eq!(batch.balances().len(), 2);
    assert_eq!(binds.load(Ordering::SeqCst), 1);

    let records = session.records();
    assert_eq!(
        records.first(),
        Some(&ReadRecord::Block(EvmBlockSelector::Latest))
    );
    assert_eq!(
        records.last(),
        Some(&ReadRecord::Block(EvmBlockSelector::Number(U256::from(
            100
        ))))
    );
    let mut decimals_calls = 0;
    let mut balance_of_calls = 0;
    for record in &records[1..records.len() - 1] {
        match record {
            ReadRecord::Balance(_, EvmBlockSelector::ExactHash(hash)) => {
                assert_eq!(*hash, ANCHOR_HASH);
            }
            ReadRecord::Call(input, EvmBlockSelector::ExactHash(hash)) => {
                assert_eq!(*hash, ANCHOR_HASH);
                decimals_calls += usize::from(input.starts_with(&ERC20_DECIMALS_SELECTOR));
                balance_of_calls += usize::from(input.starts_with(&ERC20_BALANCE_OF_SELECTOR));
            }
            unexpected => panic!("unexpected EVM read between anchor checks: {unexpected:?}"),
        }
    }
    assert_eq!(decimals_calls, 1);
    assert_eq!(balance_of_calls, 1);
}

#[tokio::test]
async fn final_number_recheck_exposes_reorg_to_the_deterministic_reducer() {
    let plan = plan_for(vec![source(ACCOUNT, EvmBalanceAsset::Native)]);
    let session = Arc::new(RecordingSession::new(
        &plan.binding().expect("binding"),
        REORG_HASH,
    ));
    let capabilities = capabilities(
        session,
        Arc::new(AtomicUsize::new(0)),
        Arc::new(EmptyFactIndex::new()),
    );
    let evidence = collect_evm_network(&plan, &capabilities)
        .await
        .expect("retained evidence");
    assert!(reduce_evm_network_collection(&plan, &evidence)
        .expect_err("reorg must fail reduction")
        .to_string()
        .contains("no longer canonical"));
}

#[tokio::test]
async fn balance_reads_never_exceed_the_hard_concurrency_limit() {
    let sources = (1_u64..=40)
        .map(|index| {
            source(
                Address::from_word(U256::from(index).into()),
                EvmBalanceAsset::Native,
            )
        })
        .collect();
    let plan = plan_for(sources);
    let session = Arc::new(RecordingSession::new(
        &plan.binding().expect("binding"),
        ANCHOR_HASH,
    ));
    let capabilities = capabilities(
        Arc::clone(&session),
        Arc::new(AtomicUsize::new(0)),
        Arc::new(EmptyFactIndex::new()),
    );
    collect_evm_network(&plan, &capabilities)
        .await
        .expect("bounded collection");
    let max_active = session.max_active.load(Ordering::SeqCst);
    assert!(max_active > 1, "test must exercise concurrent reads");
    assert!(max_active <= EVM_READ_CONCURRENCY_LIMIT);
}

#[tokio::test]
async fn all_evm_portfolios_do_not_call_the_fact_index() {
    let portfolio = PortfolioConfig {
        portfolio_id: "evm-only".parse().expect("portfolio id"),
        quote_codes: vec![QuoteCode::Usd],
        networks: vec![network()],
        wallets: vec![WalletConfig {
            wallet_id: "wallet_eth".parse().expect("wallet id"),
            subject: WalletSubject::new(format!("{ACCOUNT:#x}"), WalletSubjectKind::EvmAddress)
                .expect("wallet subject"),
            network_id: "ethereum-mainnet".parse().expect("network id"),
            implementation: WalletImplementationConfig::AddressOnly {},
            symbol_ids: vec!["eth.native.ethereum-mainnet".parse().expect("symbol id")],
            metadata: PublicMetadata::default(),
        }],
        symbol_configs: vec![SymbolConfig {
            symbol_id: "eth.native.ethereum-mainnet".parse().expect("symbol id"),
            display_symbol: Some("ETH".to_owned()),
            network_id: "ethereum-mainnet".parse().expect("network id"),
            source: HoldingSourceConfig::Native,
            valuation: SymbolValuationConfig {
                quotes: vec![QuoteValuationConfig {
                    quote: QuoteCode::Usd,
                    priced_symbol_id: "eth.native.ethereum-mainnet"
                        .parse()
                        .expect("priced symbol id"),
                    unit_price_dec: "1".parse().expect("unit price"),
                }],
            },
            metadata: PublicMetadata::default(),
        }],
        metadata: PublicMetadata::default(),
    };
    let config = SelectHoldingsConfig::new(
        portfolio,
        SelectHoldingsFactDescriptors::new(
            &BtcAddressBalanceSnapshotFact::descriptor().expect("Bitcoin descriptor"),
        )
        .expect("selection descriptors"),
    )
    .expect("selection config");
    let receipt = PortfolioCollectionReceipt::new(&[], Vec::new(), Vec::new())
        .expect("empty Bitcoin receipt");
    let artifacts = store::AsyncInMemoryRunStore::default();
    let fact_index = EmptyFactIndex::new();

    let (selected, evidence) = select_holdings(
        ValidatedConfig::new(config).expect("validated config"),
        SelectHoldingsInput { receipt },
        &artifacts,
        &fact_index,
    )
    .await
    .expect("empty selection");
    assert!(selected.observations.is_empty());
    assert!(evidence.is_empty());
    assert_eq!(fact_index.calls.load(Ordering::SeqCst), 0);
}
