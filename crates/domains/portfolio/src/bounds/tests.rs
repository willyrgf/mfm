use super::*;

#[test]
fn calculated_root_bound_covers_completed_prefix_and_snapshot_with_maximum_public_fields() {
    let target = EvmPhysicalTarget {
        chain_id: NonZeroU64::new(u64::MAX).unwrap(),
        endpoint_ref: mfm_evm::EvmEndpoint::new("\"".repeat(256))
            .unwrap()
            .endpoint_ref()
            .unwrap(),
    };
    let maximum_units =
        "115792089237316195423570985008687907853269984665640564039457584007913129639935";
    for collection_count in [1, 64] {
        let sources = (0..64)
            .map(|index| {
                EvmBalanceSource::new(
                    format!("{index:02}{}", "\"".repeat(254)),
                    target.chain_id,
                    mfm_evm::EvmAddress::from_bytes([255; 20]),
                    (index % 2 == 0).then(|| mfm_evm::EvmAddress::from_bytes([254; 20])),
                )
                .unwrap()
            })
            .collect::<Vec<_>>();
        let collections = sources
            .chunks(64 / collection_count)
            .enumerate()
            .map(|(index, sources)| {
                PortfolioCollectionDemand::new(
                    format!("{index:02}{}", "\"".repeat(254)),
                    EvmBalanceRequest::new(sources.to_vec(), 30).unwrap(),
                    target.binding_ref().unwrap(),
                )
                .unwrap()
            })
            .collect::<Vec<_>>();
        let input = PortfolioSnapshotInput::from_demand(
            serde_json::from_value(serde_json::json!("\"".repeat(256))).unwrap(),
            collections,
            serde_json::from_value(serde_json::json!("usd")).unwrap(),
        )
        .unwrap();
        let (bound, _) = conclusion_bounds(&input).unwrap();
        // Use full-width EVM amounts and the broader public Portfolio anchor contract.
        // The sum of 64 U256 values remains within the 80-digit checked total.
        let completed = input
            .collections
            .iter()
            .enumerate()
            .map(|(ordinal, demand)| PortfolioSnapshotCollection {
                collection_ordinal: ordinal as u32,
                chain_id: target.chain_id,
                anchor: PortfolioAnchor::new("9".repeat(80), "\"".repeat(256)).unwrap(),
                holdings: demand
                    .request
                    .sources()
                    .iter()
                    .map(|source| PortfolioHolding {
                        source_id: source.source_id().to_owned(),
                        asset: match source.token() {
                            None => PortfolioAsset::Native,
                            Some(contract) => PortfolioAsset::Token {
                                contract: contract.to_string(),
                            },
                        },
                        decimals: 30,
                        raw_units: maximum_units.to_owned(),
                        amount_dec: decimal_amount(maximum_units, 30),
                    })
                    .collect(),
            })
            .collect();
        let continuation = PortfolioContinuation::new(input, completed).unwrap();
        assert!(bytes(&continuation).unwrap() <= bound.max_frame_bytes());
        let ProposedStateOutcome::Success { output } =
            ConsolidatePortfolio::evaluate(continuation).unwrap()
        else {
            panic!("bounded complete Portfolio must consolidate")
        };
        assert!(bytes(&output).unwrap() <= bound.max_frame_bytes());
        assert_eq!(output.snapshot.collections.len(), collection_count);
    }
}
