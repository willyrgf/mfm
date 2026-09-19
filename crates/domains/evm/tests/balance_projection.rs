use std::num::NonZeroU64;

use mfm_capabilities::ReadImplementation;
use mfm_chain::balance::{
    BalanceEvidence, BalanceOutcome, BalanceRead, DecimalScale, ReadBalanceAt,
};
use mfm_chain::{BalanceTarget, LedgerIdentity, ObservationPoint};
use mfm_evm::*;
use mfm_values::{Object, Unsigned256};

#[test]
fn balance_projection_preserves_full_width_originals_and_rejects_unrelated_native_evidence() {
    let chain = NonZeroU64::new(1).unwrap();
    let endpoint = Object::from_value(&EvmEndpoint::new("fixture-rpc").unwrap()).unwrap();
    let route = EvmPhysicalTarget {
        chain_id: chain,
        endpoint_ref: endpoint.value_ref().clone(),
    };
    let target = EvmBalanceTarget::new(EvmAddress::from_bytes([1; 20]), None);
    let ledger = LedgerIdentity::new(Object::from_value(&EvmBalanceLedger::new(chain)).unwrap());
    let point = ObservationPoint::new(
        ledger.clone(),
        Object::from_value(&EvmBlockPoint::new(
            EvmU256::new("18446744073709551616").unwrap(),
            EvmHash::from_bytes([3; 32]),
        ))
        .unwrap(),
    );
    let intent = ReadBalanceAt::new(
        BalanceTarget::new(ledger, Object::from_value(&target).unwrap()),
        point.clone(),
        Object::from_value(&route).unwrap().value_ref().clone(),
    )
    .unwrap();
    let intent_object = Object::from_value(&intent).unwrap();
    let binding = EvmBalanceBinding::new(
        route.clone(),
        0,
        target.clone(),
        DecimalScale::new(0).unwrap(),
    );
    let binding_object = Object::from_value(&binding).unwrap();
    let abi = mfm_program::read_implementation_ref::<BalanceRead, EvmNativeBalance>().unwrap();
    let native =
        EvmNativeBalance::encode_intent(&abi, binding_object.value_ref(), &binding, &intent)
            .unwrap();
    let native_object = Object::from_value(&native).unwrap();
    let maximum = "115792089237316195423570985008687907853269984665640564039457584007913129639935";
    for (evidence, outcome) in [
        (
            EvmReadEvidence::returned(
                native_object.value_ref().clone(),
                EvmReadValue::RawUnits(EvmU256::new(maximum).unwrap()),
            ),
            BalanceOutcome::Observed {
                observed_at: point,
                raw_units: Unsigned256::new(maximum).unwrap(),
            },
        ),
        (
            EvmReadEvidence::rejected(native_object.value_ref().clone()),
            BalanceOutcome::Rejected,
        ),
        (
            EvmReadEvidence::safe_failure(native_object.value_ref().clone()),
            BalanceOutcome::SafeFailure,
        ),
        (
            EvmReadEvidence::integrity_blocked(native_object.value_ref().clone()),
            BalanceOutcome::IntegrityBlocked,
        ),
    ] {
        let original = Object::from_value(&evidence).unwrap();
        let projected = EvmNativeBalance::project_evidence(
            &abi,
            binding_object.value_ref(),
            &binding,
            intent_object.value_ref(),
            &intent,
            native_object.value_ref(),
            &native,
            &evidence,
            &original,
        )
        .unwrap();
        let cold = Object::from_value(&projected)
            .unwrap()
            .decode::<BalanceEvidence>()
            .unwrap();
        assert_eq!(cold.outcome(), &outcome);
        assert_eq!(cold.original(), &original);
        assert_eq!(
            cold.original().decode::<EvmReadEvidence>().unwrap(),
            evidence
        );
        assert_eq!(cold.intent_ref(), intent_object.value_ref());
        assert_eq!(cold.implementation_ref(), &abi);
    }
    for evidence in [
        EvmReadEvidence::rejected(intent_object.value_ref().clone()),
        EvmReadEvidence::returned(
            native_object.value_ref().clone(),
            EvmReadValue::ChainId(chain),
        ),
    ] {
        let original = Object::from_value(&evidence).unwrap();
        assert!(EvmNativeBalance::project_evidence(
            &abi,
            binding_object.value_ref(),
            &binding,
            intent_object.value_ref(),
            &intent,
            native_object.value_ref(),
            &native,
            &evidence,
            &original,
        )
        .is_err());
    }
    assert!(
        EvmTokenBalance::encode_intent(&abi, binding_object.value_ref(), &binding, &intent)
            .is_err()
    );
    for mismatch in [
        EvmBalanceBinding::new(
            route.clone(),
            0,
            EvmBalanceTarget::new(EvmAddress::from_bytes([9; 20]), None),
            DecimalScale::new(0).unwrap(),
        ),
        EvmBalanceBinding::new(
            EvmPhysicalTarget {
                chain_id: NonZeroU64::new(2).unwrap(),
                endpoint_ref: route.endpoint_ref.clone(),
            },
            0,
            target,
            DecimalScale::new(0).unwrap(),
        ),
    ] {
        let object = Object::from_value(&mismatch).unwrap();
        assert!(
            EvmNativeBalance::encode_intent(&abi, object.value_ref(), &mismatch, &intent).is_err()
        );
    }
}
