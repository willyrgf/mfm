use mfm_chain::balance::*;
use mfm_chain::{BalanceTarget, LedgerIdentity, ObservationPoint};
use mfm_program::{Classification, ClassifyError, ProposedStateOutcome, ReadState};
use mfm_values::{Object, Unsigned256};

#[test]
fn observed_candidates_survive_cold_loading_without_confirmation_or_early_amount_admission() {
    // Scalar native payloads and continuation test shared contracts, not a native confirmation.
    let native = Object::from_value(&Unsigned256::from_u64(1)).unwrap();
    let ledger = LedgerIdentity::new(native.clone());
    let target = BalanceTarget::new(ledger.clone(), native.clone());
    let request = BalanceRequest::new(
        vec![BalanceSource::new("source".into(), target).unwrap()],
        DecimalScale::new(0).unwrap(),
    )
    .unwrap();
    let metadata =
        BalanceCollectionMetadata::new(7, "collection".into(), native.value_ref().clone()).unwrap();
    let metadata_wire = serde_json::to_value(&metadata).unwrap();
    for bad in ["", "bad\ncorrelation", "password=deliberate-test-marker"] {
        let mut wire = metadata_wire.clone();
        wire["correlation"] = serde_json::json!(bad);
        assert!(serde_json::from_str::<BalanceCollectionMetadata>(&wire.to_string()).is_err());
    }
    let context = BalanceContext::new(request, Unsigned256::from_u64(42), metadata);
    let point = ObservationPoint::new(ledger, native.clone());
    let prepared =
        PreparedBalance::new(context, point.clone(), DecimalScale::new(2).unwrap()).unwrap();
    let prepared_object = Object::from_value(&prepared).unwrap();
    let intent = ObserveBalance::<Unsigned256>::prepare(&prepared).unwrap();
    let intent = Object::from_value(&intent).unwrap();
    for (outcome, expected) in [
        (
            BalanceOutcome::Rejected,
            ObserveBalanceFailure::ObservationUnavailable,
        ),
        (
            BalanceOutcome::SafeFailure,
            ObserveBalanceFailure::ObservationUnavailable,
        ),
        (
            BalanceOutcome::IntegrityBlocked,
            ObserveBalanceFailure::IntegrityBlocked,
        ),
    ] {
        let evidence = BalanceEvidence::new(
            intent.value_ref().clone(),
            native.value_ref().clone(),
            native.clone(),
            outcome,
        );
        let input = prepared_object
            .decode::<PreparedBalance<Unsigned256>>()
            .unwrap();
        let ProposedStateOutcome::Failure { failure } =
            ObserveBalance::<Unsigned256>::interpret(input, &evidence).unwrap()
        else {
            panic!("expected semantic rejection")
        };
        assert_eq!(failure, expected);
        assert_eq!(failure.classify(), Classification::Permanent);
    }
    let evidence = BalanceEvidence::new(
        intent.value_ref().clone(),
        native.value_ref().clone(),
        native.clone(),
        BalanceOutcome::Observed {
            observed_at: point.clone(),
            raw_units: Unsigned256::from_u64(101),
        },
    );
    let ProposedStateOutcome::Success { output: candidate } =
        ObserveBalance::<Unsigned256>::interpret(prepared, &evidence).unwrap()
    else {
        panic!("expected candidate before scaling")
    };
    let candidate = Object::from_value(&candidate)
        .unwrap()
        .decode::<CandidateBalance<Unsigned256>>()
        .unwrap();
    assert!(candidate.prepared().context().completed().is_empty());
    assert_eq!(candidate.prepared().context().caller().as_str(), "42");
    assert_eq!(
        candidate
            .prepared()
            .context()
            .metadata()
            .collection_ordinal(),
        7
    );
    assert_eq!(
        candidate.prepared().context().metadata().correlation(),
        "collection"
    );
    assert_eq!(
        candidate.prepared().context().metadata().route_ref(),
        native.value_ref()
    );
    assert_eq!(candidate.raw_units().as_str(), "101");
    assert!(matches!(
        candidate.append_confirmed().unwrap(),
        Err(BalanceCollectionFailure::InexactScale { .. })
    ));

    let input = prepared_object
        .decode::<PreparedBalance<Unsigned256>>()
        .unwrap();
    let wrong = BalanceEvidence::new(
        native.value_ref().clone(),
        native.value_ref().clone(),
        native,
        BalanceOutcome::SafeFailure,
    );
    assert!(ObserveBalance::<Unsigned256>::interpret(input, &wrong).is_err());
    let input = prepared_object
        .decode::<PreparedBalance<Unsigned256>>()
        .unwrap();
    let context = CandidateBalance::new(input, Unsigned256::from_u64(100))
        .append_confirmed()
        .unwrap()
        .unwrap();
    assert_eq!(context.completed().len(), 1);
    assert_eq!(context.completed()[0].source().source_id(), "source");
    assert_eq!(context.completed()[0].raw_units().as_str(), "100");
    assert!(matches!(
        context.active_source(),
        Err(BalanceContextError::Exhausted)
    ));
    assert!(matches!(
        PreparedBalance::new(context, point, DecimalScale::new(2).unwrap()),
        Err(BalanceContextError::Exhausted)
    ));
}

#[test]
fn completed_prefix_decoding_checks_order_points_and_amounts_but_leaves_total_overflow_to_consolidation(
) {
    let native = Object::from_value(&Unsigned256::from_u64(1)).unwrap();
    let ledger = LedgerIdentity::new(native.clone());
    let target = BalanceTarget::new(ledger.clone(), native.clone());
    let request = BalanceRequest::new(
        vec![
            BalanceSource::new("first".into(), target.clone()).unwrap(),
            BalanceSource::new("second".into(), target).unwrap(),
        ],
        DecimalScale::new(30).unwrap(),
    )
    .unwrap();
    let metadata =
        BalanceCollectionMetadata::new(0, "collection".into(), native.value_ref().clone()).unwrap();
    let context = BalanceContext::new(request, Unsigned256::from_u64(84), metadata);
    let point = ObservationPoint::new(ledger.clone(), native.clone());
    let prepared =
        PreparedBalance::new(context, point.clone(), DecimalScale::new(0).unwrap()).unwrap();
    let context = CandidateBalance::new(prepared, Unsigned256::new("9".repeat(50)).unwrap())
        .append_confirmed()
        .unwrap()
        .unwrap();
    let object = Object::from_value(&context).unwrap();
    let other = Object::from_value(&Unsigned256::from_u64(2)).unwrap();
    for mismatch in [
        ObservationPoint::new(ledger, other.clone()),
        ObservationPoint::new(LedgerIdentity::new(other), native),
    ] {
        assert!(PreparedBalance::new(
            object.decode::<BalanceContext<Unsigned256>>().unwrap(),
            mismatch,
            DecimalScale::new(0).unwrap()
        )
        .is_err());
    }
    let prepared = PreparedBalance::new(context, point, DecimalScale::new(0).unwrap()).unwrap();
    let context = CandidateBalance::new(prepared, Unsigned256::from_u64(1))
        .append_confirmed()
        .unwrap()
        .unwrap();
    let context = Object::from_value(&context)
        .unwrap()
        .decode::<BalanceContext<Unsigned256>>()
        .unwrap();
    let error = context
        .request()
        .total_scaled(
            context
                .completed()
                .iter()
                .map(|entry| (entry.raw_units(), entry.source_decimals())),
        )
        .unwrap_err();
    assert!(matches!(
        error,
        BalanceCollectionFailure::DecimalCapacityExceeded {
            operation: BalanceArithmetic::Sum,
            ..
        }
    ));

    let wire = serde_json::to_value(&context).unwrap();
    let mut reordered = wire.clone();
    reordered["completed"].as_array_mut().unwrap().swap(0, 1);
    let mut point = wire.clone();
    point["completed"][1]["observed_at"]["native"] =
        serde_json::to_value(Object::from_value(&Unsigned256::from_u64(3)).unwrap()).unwrap();
    let mut amount = wire.clone();
    amount["completed"][0]["raw_units"] = serde_json::json!(
        "115792089237316195423570985008687907853269984665640564039457584007913129639935"
    );
    let mut excessive = wire;
    let extra = excessive["completed"][1].clone();
    excessive["completed"].as_array_mut().unwrap().push(extra);
    for invalid in [reordered, point, amount, excessive] {
        assert!(serde_json::from_str::<BalanceContext<Unsigned256>>(&invalid.to_string()).is_err());
    }
}
