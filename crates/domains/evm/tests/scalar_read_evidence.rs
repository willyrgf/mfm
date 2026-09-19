//! Native projection checks exact anchored evidence; this does not claim live provider execution.
use mfm_capabilities::{CallbackFailure, ReadCapabilityContract, ReadImplementation};
use mfm_chain::transaction::{
    ContractRead, ContractValueEvidence, ContractValueOutcome, ReadContractValue,
};
use mfm_chain::{ContractLocator, LedgerIdentity, ObservationPoint};
use mfm_evm::{
    AnchoredContractCallEvidence, AnchoredContractCallIntent, AnchoredContractCallResult,
    EvmAddress, EvmBlockAnchor, EvmBlockPoint, EvmChainInstance,
    EvmContractReadImplementation as Native, EvmHash, EvmTransactionRoute, EvmU256,
};
use mfm_values::Object;
use std::num::NonZeroU64;

#[test]
fn scalar_projection_preserves_all_outcomes_and_rejects_mismatched_or_malformed_evidence() {
    let route = EvmTransactionRoute {
        chain_instance: EvmChainInstance {
            chain_id: NonZeroU64::new(1).unwrap(),
            expected_genesis_hash: EvmHash::from_bytes([1; 32]),
        },
        endpoint_ref: Object::from_value(&EvmU256::from_u64(1))
            .unwrap()
            .value_ref()
            .clone(),
    };
    let binding = Object::from_value(&route).unwrap();
    let implementation = mfm_program::read_implementation_ref::<ContractRead, Native>().unwrap();
    let ledger = LedgerIdentity::new(Object::from_value(&route.chain_instance).unwrap());
    let point = EvmBlockPoint::new(EvmU256::from_u64(7), EvmHash::from_bytes([2; 32]));
    let intent = ReadContractValue::new(
        binding.value_ref().clone(),
        ContractLocator::new(
            ledger.clone(),
            Object::from_value(&EvmAddress::from_bytes([3; 20])).unwrap(),
        ),
        ObservationPoint::new(ledger, Object::from_value(&point).unwrap()),
    )
    .unwrap();
    for malformed in 0..3 {
        let wrong = Object::from_value(&EvmU256::from_u64(99)).unwrap();
        let ledger = if malformed == 0 {
            LedgerIdentity::new(wrong.clone())
        } else {
            intent.target().ledger().clone()
        };
        let forged = ReadContractValue::new(
            binding.value_ref().clone(),
            ContractLocator::new(
                ledger.clone(),
                if malformed == 1 {
                    wrong.clone()
                } else {
                    intent.target().native().clone()
                },
            ),
            ObservationPoint::new(
                ledger,
                if malformed == 2 {
                    wrong
                } else {
                    intent.at().native().clone()
                },
            ),
        )
        .unwrap();
        assert!(matches!(
            Native::encode_intent(&implementation, binding.value_ref(), &route, &forged),
            Err(CallbackFailure::Decode(_))
        ));
    }
    let semantic = Object::from_value(&intent).unwrap();
    let native =
        Native::encode_intent(&implementation, binding.value_ref(), &route, &intent).unwrap();
    assert_eq!(native.calldata(), [0x3f, 0xa4, 0xf2, 0x45]);
    assert_eq!(native.route_ref(), binding.value_ref());
    let native_object = Object::from_value(&native).unwrap();
    let project = |evidence: &AnchoredContractCallEvidence| {
        let original = Object::from_value(evidence).unwrap();
        Native::project_evidence(
            &implementation,
            binding.value_ref(),
            &route,
            semantic.value_ref(),
            &intent,
            native_object.value_ref(),
            &native,
            evidence,
            &original,
        )
    };
    for (evidence, outcome) in [
        (
            AnchoredContractCallEvidence::rejected(native_object.value_ref().clone()),
            ContractValueOutcome::Rejected,
        ),
        (
            AnchoredContractCallEvidence::safe_failure(native_object.value_ref().clone()),
            ContractValueOutcome::SafeFailure,
        ),
        (
            AnchoredContractCallEvidence::integrity_blocked(native_object.value_ref().clone()),
            ContractValueOutcome::IntegrityBlocked,
        ),
    ] {
        let original = Object::from_value(&evidence).unwrap();
        let projected = project(&evidence).unwrap();
        ContractRead::bind_evidence(
            semantic.value_ref(),
            &intent,
            original.value_ref(),
            &projected,
        )
        .unwrap();
        let cold = Object::from_value(&projected)
            .unwrap()
            .decode::<ContractValueEvidence>()
            .unwrap();
        assert_eq!(cold.outcome(), &outcome);
        assert_eq!(cold.original(), &original);
        assert_eq!(
            cold.original()
                .decode::<AnchoredContractCallEvidence>()
                .unwrap(),
            evidence
        );
    }
    let mut forty_two = [0; 32];
    forty_two[31] = 42;
    for (bytes, expected) in [
        ([0; 32], "0"),
        (forty_two, "42"),
        (
            [255; 32],
            "115792089237316195423570985008687907853269984665640564039457584007913129639935",
        ),
    ] {
        let evidence = AnchoredContractCallEvidence::returned(
            native_object.value_ref().clone(),
            AnchoredContractCallResult::new(native.anchor().clone(), bytes.to_vec()).unwrap(),
        );
        let projected = project(&evidence).unwrap();
        let ContractValueOutcome::Observed { observed_at, value } = projected.outcome() else {
            panic!("observed scalar")
        };
        assert_eq!(observed_at, intent.at());
        assert_eq!(value.to_string(), expected);
        assert_eq!(
            projected.original(),
            &Object::from_value(&evidence).unwrap()
        );
    }
    for length in [0, 31, 33] {
        let evidence = AnchoredContractCallEvidence::returned(
            native_object.value_ref().clone(),
            AnchoredContractCallResult::new(native.anchor().clone(), vec![0; length]).unwrap(),
        );
        let CallbackFailure::Decode(error) = project(&evidence).unwrap_err() else {
            panic!("native scalar decoding must retain Decode phase")
        };
        assert_eq!(error.operation(), "decode_scalar_word");
        assert_eq!(error.details().as_value()["actual_bytes"], length);
    }
    // Route, anchor, target and calldata changes cannot reuse the retained native request.
    let evidence = AnchoredContractCallEvidence::returned(
        native_object.value_ref().clone(),
        AnchoredContractCallResult::new(native.anchor().clone(), vec![0; 32]).unwrap(),
    );
    let original = Object::from_value(&evidence).unwrap();
    for alternate in [
        AnchoredContractCallIntent::new(
            native.chain_id(),
            semantic.value_ref().clone(),
            native.anchor().clone(),
            native.target().clone(),
            native.calldata().to_vec(),
        )
        .unwrap(),
        AnchoredContractCallIntent::new(
            native.chain_id(),
            native.route_ref().clone(),
            EvmBlockAnchor {
                number: EvmU256::from_u64(99),
                hash: EvmHash::from_bytes([8; 32]),
            },
            native.target().clone(),
            native.calldata().to_vec(),
        )
        .unwrap(),
        AnchoredContractCallIntent::new(
            native.chain_id(),
            native.route_ref().clone(),
            native.anchor().clone(),
            EvmAddress::from_bytes([8; 20]),
            native.calldata().to_vec(),
        )
        .unwrap(),
        AnchoredContractCallIntent::new(
            native.chain_id(),
            native.route_ref().clone(),
            native.anchor().clone(),
            native.target().clone(),
            vec![0xca, 0xfe],
        )
        .unwrap(),
    ] {
        let reference = Object::from_value(&alternate).unwrap();
        assert!(Native::project_evidence(
            &implementation,
            binding.value_ref(),
            &route,
            semantic.value_ref(),
            &intent,
            reference.value_ref(),
            &alternate,
            &evidence,
            &original
        )
        .is_err());
    }
    let wrong = AnchoredContractCallEvidence::rejected(binding.value_ref().clone());
    assert_eq!(
        execute_cause(project(&wrong).unwrap_err()).operation(),
        "project_scalar_intent"
    );
    let wrong = AnchoredContractCallEvidence::returned(
        native_object.value_ref().clone(),
        AnchoredContractCallResult::new(
            EvmBlockAnchor {
                number: EvmU256::from_u64(8),
                hash: EvmHash::from_bytes([4; 32]),
            },
            vec![0; 32],
        )
        .unwrap(),
    );
    assert_eq!(
        execute_cause(project(&wrong).unwrap_err()).operation(),
        "project_scalar_anchor"
    );
    let mut foreign = route.clone();
    foreign.chain_instance.expected_genesis_hash = EvmHash::from_bytes([9; 32]);
    assert_eq!(
        execute_cause(
            Native::encode_intent(&implementation, binding.value_ref(), &foreign, &intent)
                .unwrap_err()
        )
        .operation(),
        "scalar_read_ledger"
    );

    // Same-ledger endpoint changes cannot replace the caller-owned expectation.
    let mut alternate = route.clone();
    alternate.endpoint_ref = Object::from_value(&EvmU256::from_u64(2))
        .unwrap()
        .value_ref()
        .clone();
    let alternate_binding = Object::from_value(&alternate).unwrap();
    let mismatch = Native::encode_intent(
        &implementation,
        alternate_binding.value_ref(),
        &alternate,
        &intent,
    )
    .unwrap_err();
    let mismatch = execute_cause(mismatch);
    assert_eq!(mismatch.operation(), "qualify_scalar_route");
    assert_eq!(
        mismatch.details().as_value()["expected"],
        serde_json::to_value(binding.value_ref()).unwrap()
    );
    assert_eq!(
        mismatch.details().as_value()["actual"],
        serde_json::to_value(alternate_binding.value_ref()).unwrap()
    );
}

fn execute_cause(error: CallbackFailure) -> mfm_values::InvocationDiagnostic {
    let CallbackFailure::Execute(cause) = error else {
        panic!("semantic binding failure must retain Execute phase")
    };
    cause
}
