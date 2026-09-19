//! The designated intent commits the collection route before native translation.
use mfm_chain::balance::{
    BalanceCollectionMetadata, BalanceContext, BalanceRequest, BalanceSource, DecimalScale,
    ObserveBalance, PreparedBalance,
};
use mfm_chain::{BalanceTarget, LedgerIdentity, ObservationPoint};
use mfm_program::ReadState;
use mfm_values::{Object, Unsigned256};

#[test]
fn different_collection_routes_produce_distinct_designated_balance_intents() {
    // Scalar native payloads isolate the information boundary; no provider authentication is claimed.
    let native = Object::from_value(&Unsigned256::from_u64(1)).unwrap();
    let other_route = Object::from_value(&Unsigned256::from_u64(2)).unwrap();
    let ledger = LedgerIdentity::new(native.clone());
    let target = BalanceTarget::new(ledger.clone(), native.clone());
    let request = BalanceRequest::new(
        vec![BalanceSource::new("source".into(), target).unwrap()],
        DecimalScale::new(0).unwrap(),
    )
    .unwrap();
    let point = ObservationPoint::new(ledger, native.clone());
    let mut intents = Vec::new();
    for route in [native.value_ref(), other_route.value_ref()] {
        let metadata =
            BalanceCollectionMetadata::new(0, "collection".into(), route.clone()).unwrap();
        let context = BalanceContext::new(request.clone(), Unsigned256::from_u64(42), metadata);
        let prepared =
            PreparedBalance::new(context, point.clone(), DecimalScale::new(0).unwrap()).unwrap();
        assert_eq!(prepared.context().metadata().route_ref(), route);
        let intent = ObserveBalance::<Unsigned256>::prepare(&prepared).unwrap();
        assert_eq!(intent.route_ref(), route);
        intents.push(Object::from_value(&intent).unwrap());
    }
    assert_ne!(native.value_ref(), other_route.value_ref());
    assert_ne!(intents[0], intents[1]);
}
