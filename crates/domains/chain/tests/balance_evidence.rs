use mfm_capabilities::ReadCapabilityContract;
use mfm_chain::balance::{BalanceEvidence, BalanceOutcome, BalanceRead, ReadBalanceAt};
use mfm_chain::{BalanceTarget, LedgerIdentity, ObservationPoint};
use mfm_values::{Object, Unsigned256};

#[test]
fn all_balance_outcomes_keep_exact_originals_and_bind_only_to_the_requested_intent_and_point() {
    // These scalar native payloads prove the semantic boundary, not native authentication.
    let original = Object::from_value(&Unsigned256::from_u64(1)).unwrap();
    let different = Object::from_value(&Unsigned256::from_u64(2)).unwrap();
    let ledger = LedgerIdentity::new(original.clone());
    let point = ObservationPoint::new(ledger.clone(), original.clone());
    let target = BalanceTarget::new(ledger.clone(), different.clone());
    let intent =
        ReadBalanceAt::new(target.clone(), point.clone(), original.value_ref().clone()).unwrap();
    let intent_object = Object::from_value(&intent).unwrap();
    let max = Unsigned256::new(
        "115792089237316195423570985008687907853269984665640564039457584007913129639935",
    )
    .unwrap();
    for outcome in [
        BalanceOutcome::Observed {
            observed_at: point.clone(),
            raw_units: max.clone(),
        },
        BalanceOutcome::Rejected,
        BalanceOutcome::SafeFailure,
        BalanceOutcome::IntegrityBlocked,
    ] {
        let evidence = BalanceEvidence::new(
            intent_object.value_ref().clone(),
            different.value_ref().clone(),
            original.clone(),
            outcome.clone(),
        );
        BalanceRead::bind_evidence(
            intent_object.value_ref(),
            &intent,
            original.value_ref(),
            &evidence,
        )
        .unwrap();
        let restored = Object::from_value(&evidence)
            .unwrap()
            .decode::<BalanceEvidence>()
            .unwrap();
        assert_eq!(restored.outcome(), &outcome);
        assert_eq!(
            restored.original().canonical_bytes(),
            original.canonical_bytes()
        );
        assert_eq!(restored.original().value_ref(), original.value_ref());
        assert_eq!(restored.implementation_ref(), different.value_ref());
        assert!(BalanceRead::bind_evidence(
            different.value_ref(),
            &intent,
            original.value_ref(),
            &restored
        )
        .is_err());
        assert!(BalanceRead::bind_evidence(
            intent_object.value_ref(),
            &intent,
            different.value_ref(),
            &restored
        )
        .is_err());
    }
    let wrong_point = BalanceEvidence::new(
        intent_object.value_ref().clone(),
        different.value_ref().clone(),
        original.clone(),
        BalanceOutcome::Observed {
            observed_at: ObservationPoint::new(ledger, different.clone()),
            raw_units: max,
        },
    );
    assert!(BalanceRead::bind_evidence(
        intent_object.value_ref(),
        &intent,
        original.value_ref(),
        &wrong_point
    )
    .is_err());
    let route_ref = original.value_ref().clone();
    let foreign = ObservationPoint::new(LedgerIdentity::new(different), original);
    assert!(ReadBalanceAt::new(target, foreign.clone(), route_ref).is_err());
    let mut wire = serde_json::to_value(intent).unwrap();
    wire["observed_at"] = serde_json::to_value(foreign).unwrap();
    assert!(serde_json::from_str::<ReadBalanceAt>(&wire.to_string()).is_err());
}
