use super::*;

#[derive(Debug, Serialize, Deserialize, MfmValue)]
#[serde(transparent)]
struct Caller {
    #[mfm(minimum_bytes = 1, maximum_bytes = 65536)]
    accumulated: String,
}

#[test]
fn full_width_observation_and_consolidation_preserve_checked_values() {
    let maximum = EvmU256::new(
        "115792089237316195423570985008687907853269984665640564039457584007913129639935",
    )
    .unwrap();
    let anchor = EvmBlockAnchor {
        number: maximum.clone(),
        hash: EvmHash::from_bytes([255; 32]),
    };
    let route = EvmEndpoint::new("\"".repeat(256))
        .unwrap()
        .endpoint_ref()
        .unwrap();
    for token_count in [0, 32, 64] {
        let sources = (0..64)
            .map(|index| {
                EvmBalanceSource::new(
                    format!("{index:02}{}", "\"".repeat(254)),
                    NonZeroU64::new(u64::MAX).unwrap(),
                    EvmAddress::from_bytes([255; 20]),
                    (index < token_count).then(|| EvmAddress::from_bytes([254; 20])),
                )
                .unwrap()
            })
            .collect::<Vec<_>>();
        let request = EvmBalanceRequest::new(sources, 30).unwrap();
        let caller = Caller {
            accumulated: "\"".repeat(65536),
        };
        let mut input = EvmBalanceContext::new(
            request.clone(),
            caller,
            u32::MAX,
            "\"".repeat(256),
            route.clone(),
        )
        .unwrap();
        input.completed = request.sources()[..63]
            .iter()
            .map(|source| EvmBalanceResult {
                source: source.clone(),
                decimals: 30,
                raw_units: maximum.clone(),
                anchor: anchor.clone(),
            })
            .collect();
        input.work = EvmBalanceWork::ConfirmAnchor {
            checked_chain_id: NonZeroU64::new(u64::MAX).unwrap(),
            initial_anchor: anchor.clone(),
            source_decimals: 30,
            raw_balance: maximum.clone(),
        };
        input.validate().unwrap();
        let intent =
            <ConfirmBalanceAnchor<Caller> as ReadState<EvmAnchorRead>>::prepare(&input).unwrap();
        let (_, intent_ref) = mfm_values::canonicalize_mfm_value(&intent).unwrap();
        let input_wire = mfm_values::canonicalize_mfm_value(&input).unwrap().0;
        let changed_input = serde_json::from_slice(input_wire.as_bytes()).unwrap();
        let changed = EvmBlockAnchor {
            number: maximum.clone(),
            hash: EvmHash::from_bytes([254; 32]),
        };
        let changed_evidence =
            EvmReadEvidence::returned(intent_ref.clone(), EvmReadValue::Anchor(changed));
        let ProposedStateOutcome::Failure { failure } =
            <ConfirmBalanceAnchor<Caller> as ReadState<EvmAnchorRead>>::interpret(
                changed_input,
                &changed_evidence,
            )
            .unwrap()
        else {
            panic!("changed anchor must be a distinct typed failure")
        };
        assert!(matches!(
            failure,
            EvmBalanceFailure::AnchorChanged {
                collection_ordinal: u32::MAX,
                ..
            }
        ));
        let mut invalid = serde_json::to_value(&failure).unwrap();
        invalid["value"]["observed"] = invalid["value"]["previous"].clone();
        assert!(serde_json::from_value::<EvmBalanceFailure>(invalid).is_err());
        let evidence = EvmReadEvidence::returned(intent_ref, EvmReadValue::Anchor(anchor.clone()));
        let ProposedStateOutcome::Success { output } =
            <ConfirmBalanceAnchor<Caller> as ReadState<EvmAnchorRead>>::interpret(input, &evidence)
                .unwrap()
        else {
            panic!("maximum valid observation must succeed")
        };
        let ProposedStateOutcome::Success { output } =
            ConsolidateBalanceCollection::<Caller>::evaluate(output).unwrap()
        else {
            panic!("maximum valid collection must consolidate")
        };
        mfm_values::canonicalize_mfm_value(&output).unwrap();
    }
}
