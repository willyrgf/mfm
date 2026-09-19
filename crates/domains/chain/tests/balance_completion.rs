use mfm_chain::balance::*;
use mfm_chain::{BalanceTarget, LedgerIdentity, ObservationPoint};
use mfm_program::{Classification, ClassifyError, ProposedStateOutcome, PureState};
use mfm_values::{Object, Unsigned256};

#[test]
fn consolidation_requires_complete_confirmation_and_retains_checked_total_and_caller() {
    // Shared semantic fixture: scalar native payloads do not authenticate a provider observation.
    let native = Object::from_value(&Unsigned256::from_u64(1)).unwrap();
    let ledger = LedgerIdentity::new(native.clone());
    let target = BalanceTarget::new(ledger.clone(), native.clone());
    let point = ObservationPoint::new(ledger, native.clone());
    for (amounts, scale, expected) in [
        (["9".to_owned(), "100".to_owned()], 0, Some("109")),
        (["9".repeat(50), "1".to_owned()], 30, None),
    ] {
        let request = BalanceRequest::new(
            vec![
                BalanceSource::new("first".into(), target.clone()).unwrap(),
                BalanceSource::new("second".into(), target.clone()).unwrap(),
            ],
            DecimalScale::new(scale).unwrap(),
        )
        .unwrap();
        let metadata =
            BalanceCollectionMetadata::new(7, "collection".into(), native.value_ref().clone())
                .unwrap();
        let mut context = BalanceContext::new(request, Unsigned256::from_u64(42), metadata);
        for (completed, amount) in amounts.into_iter().enumerate() {
            let incomplete = Object::from_value(&context).unwrap();
            let error =
                ConsolidateBalanceCollection::<Unsigned256>::evaluate(incomplete.decode().unwrap())
                    .unwrap_err();
            assert_eq!(error.operation(), "consolidate_balances");
            assert_eq!(
                error.details().as_value(),
                &serde_json::json!({
                    "Incomplete": {"completed": completed, "expected": 2}
                })
            );
            let prepared =
                PreparedBalance::new(context, point.clone(), DecimalScale::new(0).unwrap())
                    .unwrap();
            context = CandidateBalance::new(prepared, Unsigned256::new(amount).unwrap())
                .append_confirmed()
                .unwrap()
                .unwrap();
        }
        if expected.is_none() {
            // A complete prefix may overflow only during consolidation. No persisted success
            // total, including an invented zero, may turn that context into a valid completion.
            let wire = serde_json::json!({"context": context, "total_scaled": "0"});
            assert!(
                serde_json::from_str::<BalanceCollectionCompletion<Unsigned256>>(&wire.to_string())
                    .is_err()
            );
        }
        let outcome = ConsolidateBalanceCollection::<Unsigned256>::evaluate(context).unwrap();
        match (outcome, expected) {
            (ProposedStateOutcome::Success { output }, Some(expected)) => {
                assert_eq!(output.total_scaled(), expected);
                let cold = Object::from_value(&output)
                    .unwrap()
                    .decode::<BalanceCollectionCompletion<Unsigned256>>()
                    .unwrap();
                assert_eq!(cold.context().caller().as_str(), "42");
                assert_eq!(cold.context().completed()[0].source().source_id(), "first");
                assert_eq!(cold.context().completed()[1].source().source_id(), "second");
                let wire = serde_json::to_value(&cold).unwrap();
                for forged in ["110", "0109", "-109"] {
                    let mut changed = wire.clone();
                    changed["total_scaled"] = serde_json::json!(forged);
                    assert!(
                        serde_json::from_str::<BalanceCollectionCompletion<Unsigned256>>(
                            &changed.to_string()
                        )
                        .is_err()
                    );
                }
                let mut incomplete = wire;
                incomplete["context"]["completed"]
                    .as_array_mut()
                    .unwrap()
                    .pop();
                assert!(
                    serde_json::from_str::<BalanceCollectionCompletion<Unsigned256>>(
                        &incomplete.to_string()
                    )
                    .is_err()
                );
                let (context, total) = cold.into_parts();
                let (request, caller, metadata, confirmed) = context.into_parts();
                assert_eq!(total, "109");
                assert_eq!(caller.as_str(), "42");
                assert_eq!(metadata.collection_ordinal(), 7);
                assert_eq!(confirmed.len(), request.sources().len());
            }
            (ProposedStateOutcome::Failure { failure }, None) => {
                assert_eq!(failure.classify(), Classification::Permanent);
                let BalanceCollectionFailure::DecimalCapacityExceeded {
                    operation, size, ..
                } = &failure
                else {
                    panic!("expected aggregate capacity rejection")
                };
                assert_eq!(*operation, BalanceArithmetic::Sum);
                assert_eq!((size.actual(), size.limit()), (81, 80));
                assert_eq!(
                    Object::from_value(&failure)
                        .unwrap()
                        .decode::<BalanceCollectionFailure>()
                        .unwrap(),
                    failure
                );
            }
            _ => panic!("unexpected consolidation outcome"),
        }
    }
}
