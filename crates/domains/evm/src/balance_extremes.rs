use super::*;
use mfm_chain::balance::{
    BalanceCollectionMetadata, BalanceContext, BalanceRequest, BalanceSource, CandidateBalance,
    ConsolidateBalanceCollection, DecimalScale, PreparedBalance,
};
use mfm_chain::{BalanceTarget, LedgerIdentity, ObservationPoint};
use mfm_program::{ProposedStateOutcome, PureState, ReadState};
use mfm_values::{Object, Unsigned256};

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
                BalanceSource::new(
                    format!("{index:02}{}", "\"".repeat(254)),
                    BalanceTarget::new(
                        LedgerIdentity::new(
                            Object::from_value(&EvmBalanceLedger::new(
                                NonZeroU64::new(u64::MAX).unwrap(),
                            ))
                            .unwrap(),
                        ),
                        Object::from_value(&EvmBalanceTarget::new(
                            EvmAddress::from_bytes([255; 20]),
                            (index < token_count).then(|| EvmAddress::from_bytes([254; 20])),
                        ))
                        .unwrap(),
                    ),
                )
                .unwrap()
            })
            .collect::<Vec<_>>();
        let request = BalanceRequest::new(sources, DecimalScale::new(30).unwrap()).unwrap();
        let caller = Caller {
            accumulated: "\"".repeat(65536),
        };
        let point = ObservationPoint::new(
            request.sources()[0].target().ledger().clone(),
            Object::from_value(&EvmBlockPoint::new(
                anchor.number.clone(),
                anchor.hash.clone(),
            ))
            .unwrap(),
        );
        let mut context = BalanceContext::new(
            request,
            caller,
            BalanceCollectionMetadata::new(u32::MAX, "\"".repeat(256), route.clone()).unwrap(),
        );
        for _ in 0..63 {
            context = CandidateBalance::new(
                PreparedBalance::new(context, point.clone(), DecimalScale::new(30).unwrap())
                    .unwrap(),
                Unsigned256::new(maximum.as_str()).unwrap(),
            )
            .append_confirmed()
            .unwrap()
            .unwrap();
        }
        let input = CandidateBalance::new(
            PreparedBalance::new(context, point, DecimalScale::new(30).unwrap()).unwrap(),
            Unsigned256::new(maximum.as_str()).unwrap(),
        );
        let intent =
            <ConfirmEvmBalanceAnchor<Caller> as ReadState<EvmAnchorRead>>::prepare(&input).unwrap();
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
            <ConfirmEvmBalanceAnchor<Caller> as ReadState<EvmAnchorRead>>::interpret(
                changed_input,
                &changed_evidence,
            )
            .unwrap()
        else {
            panic!("changed anchor must be a distinct typed failure")
        };
        assert!(matches!(failure, EvmBalanceFailure::AnchorChanged { .. }));
        let mut invalid = serde_json::to_value(&failure).unwrap();
        invalid["anchor_changed"]["observed"] = invalid["anchor_changed"]["previous"].clone();
        assert!(serde_json::from_value::<EvmBalanceFailure>(invalid).is_err());
        let evidence = EvmReadEvidence::returned(intent_ref, EvmReadValue::Anchor(anchor.clone()));
        let ProposedStateOutcome::Success { output } =
            <ConfirmEvmBalanceAnchor<Caller> as ReadState<EvmAnchorRead>>::interpret(
                input, &evidence,
            )
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
