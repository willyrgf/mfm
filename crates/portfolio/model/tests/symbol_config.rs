use std::collections::BTreeMap;

use mfm_portfolio_model::symbol::*;

#[test]
fn valuation_source_metadata_rejects_secret_markers() {
    let mut metadata = BTreeMap::new();
    metadata.insert("authorization".to_string(), "redacted".to_string());

    assert_eq!(
        ValuationSourceConfig::new(
            "chainlink_eth_usd".to_string(),
            "ethereum-mainnet".to_string(),
            "eth.native.ethereum-mainnet".to_string(),
            QuoteCode::Usd,
            ValuationSourceReaderConfig::EvmOracle {
                oracle_kind: "chainlink".parse().expect("valid oracle kind"),
                config: EvmOracleConfig {
                    contract_address: "0x0000000000000000000000000000000000000001"
                        .parse()
                        .expect("valid address"),
                },
            },
            metadata,
        )
        .unwrap_err(),
        ValuationSourceRegistryError::MetadataContainsSecret {
            source_id: "chainlink_eth_usd".to_string(),
            key: "authorization".to_string(),
        }
    );
}
