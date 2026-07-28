use alloy_primitives::{Address, B256, U256};
use mfm_evm::{
    reduce_evm_balance_collection, CollectEvmBalancesState, EvmBalanceAsset,
    EvmBalanceCollectionConfig, EvmBalanceCollectionEvidence, EvmBalanceCollectionPlan,
    EvmBalanceReadEvidence, EvmBalanceSnapshotFact, EvmBalanceSource, EvmBlockAnchor,
    EvmSessionEvidence, EVM_JSONRPC_SESSION_IMPLEMENTATION_ID,
};
use mfm_facts::MfmFactType;
use mfm_ids::LocalPublicId;
use mfm_portfolio::{
    PortfolioConfig, SelectHoldingsConfig, SelectHoldingsFactDescriptors, SelectHoldingsInput,
    ValidatedPortfolioConfig,
};
use mfm_program::{CertifiedContext, ReadState, StateSpec, ValidatedConfig};
use serde_json::{json, Value};

pub(super) const BITCOIN_MAINNET_ADDRESS: &str = "bc1qxy2kgdygjrsqtzq2n0yrf2493p83kkfjhx0wlh";

pub(super) fn evm_address(index: usize) -> Address {
    let ordinal = u64::try_from(index)
        .expect("test address index fits u64")
        .checked_add(1)
        .expect("test address index does not overflow");
    let mut bytes = [0_u8; 20];
    bytes[12..].copy_from_slice(&ordinal.to_be_bytes());
    Address::from(bytes)
}

pub(super) fn evm_collection_config(
    source_count: usize,
    token_mask: usize,
) -> EvmBalanceCollectionConfig {
    let token = evm_address(10_000);
    let sources = (0..source_count)
        .map(|index| {
            let asset = if token_mask & (1 << index) == 0 {
                EvmBalanceAsset::Native
            } else {
                EvmBalanceAsset::erc20(token).expect("checked test token")
            };
            EvmBalanceSource::new(evm_address(index), asset).expect("checked EVM source")
        })
        .collect();
    EvmBalanceCollectionConfig::new("ethereum-mainnet", 1, 18, sources)
        .expect("checked EVM collection")
}

pub(super) fn evm_plan(config: &EvmBalanceCollectionConfig) -> EvmBalanceCollectionPlan {
    let state = <CollectEvmBalancesState as StateSpec>::new(
        ValidatedConfig::new(config.clone()).expect("validated EVM config"),
    )
    .expect("constructed EVM state");
    state
        .plan(&(), &CertifiedContext::no_context())
        .expect("validated config authors a plan")
}

pub(super) fn evm_portfolio_config(holding_count: usize) -> PortfolioConfig {
    assert!(holding_count > 0, "published portfolio demand is non-empty");
    let wallets = (0..holding_count)
        .map(|index| {
            json!({
                "wallet_id": format!("wallet_{index:04}"),
                "network_id": "ethereum-mainnet",
                "symbol_ids": ["eth.native.ethereum-mainnet"],
                "subject": {
                    "kind": "evm_address",
                    "address": format!("{:#x}", evm_address(index)),
                },
                "implementation": {"kind": "address_only"},
                "metadata": {},
            })
        })
        .collect::<Vec<_>>();
    normalized_portfolio(json!({
        "portfolio_id": "request-totality-evm",
        "quote_codes": ["USD"],
        "networks": [{
            "network_id": "ethereum-mainnet",
            "family": "evm",
            "chain_id": 1,
            "native_decimals": 18,
            "metadata": {},
        }],
        "wallets": wallets,
        "symbol_configs": [symbol(
            "eth.native.ethereum-mainnet",
            "ethereum-mainnet",
            json!({"kind": "native"}),
        )],
        "metadata": {},
    }))
}

pub(super) fn bitcoin_portfolio_config() -> PortfolioConfig {
    normalized_portfolio(json!({
        "portfolio_id": "request-totality-bitcoin",
        "quote_codes": ["USD"],
        "networks": [{
            "network_id": "bitcoin-mainnet",
            "family": "bitcoin",
            "bitcoin_network": "main",
            "source_identity": "public-bitcoin-core",
            "metadata": {},
        }],
        "wallets": [{
            "wallet_id": "wallet_bitcoin",
            "network_id": "bitcoin-mainnet",
            "symbol_ids": ["btc.native.bitcoin-mainnet"],
            "subject": {
                "kind": "bitcoin_address",
                "address": BITCOIN_MAINNET_ADDRESS,
            },
            "implementation": {"kind": "address_only"},
            "metadata": {},
        }],
        "symbol_configs": [symbol(
            "btc.native.bitcoin-mainnet",
            "bitcoin-mainnet",
            json!({"kind": "native"}),
        )],
        "metadata": {},
    }))
}

pub(super) fn selection_material(
    holding_count: usize,
) -> (SelectHoldingsConfig, SelectHoldingsInput) {
    let portfolio = evm_portfolio_config(holding_count);
    let collection = evm_collection_config(holding_count, 0);
    let plan = evm_plan(&collection);
    let binding = collection.binding().expect("checked binding");
    let session = EvmSessionEvidence::new(
        &binding,
        LocalPublicId::new("routing-generation-a").expect("source ref"),
        LocalPublicId::new(EVM_JSONRPC_SESSION_IMPLEMENTATION_ID).expect("implementation id"),
    );
    let anchor = EvmBlockAnchor::new(U256::from(42), B256::from([0x11; 32]));
    let balances = plan
        .sources()
        .iter()
        .cloned()
        .enumerate()
        .map(|(index, source)| EvmBalanceReadEvidence::new(source, U256::from(index + 1)))
        .collect();
    let evidence =
        EvmBalanceCollectionEvidence::new(&session, anchor.clone(), Vec::new(), balances, anchor);
    let (receipt, _) =
        reduce_evm_balance_collection(&plan, &evidence).expect("checked EVM receipt");
    let fact_descriptors = SelectHoldingsFactDescriptors::new(
        &mfm_bitcoin::BitcoinBalanceSnapshotFact::descriptor().expect("Bitcoin fact descriptor"),
        &EvmBalanceSnapshotFact::descriptor().expect("EVM fact descriptor"),
    )
    .expect("checked selection descriptors");
    let config = SelectHoldingsConfig::new(portfolio, fact_descriptors).expect("selection config");
    let input = SelectHoldingsInput {
        bitcoin_receipts: Vec::new(),
        evm_receipts: vec![receipt],
    };
    (config, input)
}

fn normalized_portfolio(value: Value) -> PortfolioConfig {
    let config: PortfolioConfig = serde_json::from_value(value).expect("portfolio JSON");
    ValidatedPortfolioConfig::new(config)
        .expect("validated portfolio")
        .into_config()
}

fn symbol(symbol_id: &str, network_id: &str, source: Value) -> Value {
    json!({
        "symbol_id": symbol_id,
        "display_symbol": symbol_id,
        "network_id": network_id,
        "source": source,
        "valuation": {
            "quotes": [{
                "quote": "USD",
                "priced_symbol_id": symbol_id,
                "unit_price_dec": "1.00",
            }],
        },
        "metadata": {},
    })
}
