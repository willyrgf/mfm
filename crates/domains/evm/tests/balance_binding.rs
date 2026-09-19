use std::num::NonZeroU64;

use mfm_capabilities::ReadImplementation;
use mfm_chain::balance::{
    BalanceCollectionMetadata, BalanceContext, BalanceRequest, BalanceSource, DecimalScale,
};
use mfm_chain::{BalanceTarget, LedgerIdentity};
use mfm_evm::*;
use mfm_values::{Object, Unsigned256};

#[test]
fn supporting_translation_checks_retained_source_qualification_against_each_selected_binding() {
    let chain = NonZeroU64::new(1).unwrap();
    let endpoint = Object::from_value(&EvmEndpoint::new("fixture-rpc").unwrap()).unwrap();
    let route = EvmPhysicalTarget {
        chain_id: chain,
        endpoint_ref: endpoint.value_ref().clone(),
    };
    let route_object = Object::from_value(&route).unwrap();
    let target = EvmBalanceTarget::new(EvmAddress::from_bytes([1; 20]), None);
    let scale = DecimalScale::new(6).unwrap();
    let source = BalanceSource::new(
        "native".into(),
        BalanceTarget::new(
            LedgerIdentity::new(Object::from_value(&EvmBalanceLedger::new(chain)).unwrap()),
            Object::from_value(&target).unwrap(),
        ),
    )
    .unwrap();
    let context = BalanceContext::new(
        BalanceRequest::new(vec![source], scale).unwrap(),
        Unsigned256::from_u64(42),
        BalanceCollectionMetadata::new(0, "collection".into(), route_object.value_ref().clone())
            .unwrap(),
    );
    let intent = EvmReadIntent::from_context(&context, EvmReadSubject::ChainIdentity).unwrap();
    let binding = EvmBalanceBinding::new(route.clone(), 0, target.clone(), scale);
    let binding_object = Object::from_value(&binding).unwrap();
    let abi = mfm_program::read_implementation_ref::<EvmChainIdentityRead, EvmBalanceObservation>()
        .unwrap();
    let native =
        <EvmBalanceObservation as ReadImplementation<EvmChainIdentityRead>>::encode_intent(
            &abi,
            binding_object.value_ref(),
            &binding,
            &intent,
        )
        .unwrap();
    assert_eq!(native, intent);
    assert_eq!(
        Object::from_value(&native)
            .unwrap()
            .decode::<EvmReadIntent>()
            .unwrap(),
        intent
    );

    let other_endpoint = Object::from_value(&EvmEndpoint::new("other-rpc").unwrap()).unwrap();
    for mismatch in [
        EvmBalanceBinding::new(route.clone(), 1, target.clone(), scale),
        EvmBalanceBinding::new(
            EvmPhysicalTarget {
                chain_id: NonZeroU64::new(2).unwrap(),
                endpoint_ref: route.endpoint_ref.clone(),
            },
            0,
            target.clone(),
            scale,
        ),
        EvmBalanceBinding::new(
            EvmPhysicalTarget {
                chain_id: chain,
                endpoint_ref: other_endpoint.value_ref().clone(),
            },
            0,
            target.clone(),
            scale,
        ),
        EvmBalanceBinding::new(
            route.clone(),
            0,
            EvmBalanceTarget::new(EvmAddress::from_bytes([2; 20]), None),
            scale,
        ),
        EvmBalanceBinding::new(
            route.clone(),
            0,
            target.clone(),
            DecimalScale::new(7).unwrap(),
        ),
    ] {
        let object = Object::from_value(&mismatch).unwrap();
        assert!(
            <EvmBalanceObservation as ReadImplementation<EvmChainIdentityRead>>::encode_intent(
                &abi,
                object.value_ref(),
                &mismatch,
                &intent
            )
            .is_err()
        );
    }
    let wrong_family =
        EvmReadIntent::from_context(&context, EvmReadSubject::InitialAnchor).unwrap();
    assert!(
        <EvmBalanceObservation as ReadImplementation<EvmChainIdentityRead>>::encode_intent(
            &abi,
            binding_object.value_ref(),
            &binding,
            &wrong_family
        )
        .is_err()
    );

    let identity = Object::from_value(&intent).unwrap();
    let evidence = EvmReadEvidence::returned(
        identity.value_ref().clone(),
        EvmReadValue::ChainId(NonZeroU64::new(2).unwrap()),
    );
    let original = Object::from_value(&evidence).unwrap();
    let projected =
        <EvmBalanceObservation as ReadImplementation<EvmChainIdentityRead>>::project_evidence(
            &abi,
            binding_object.value_ref(),
            &binding,
            identity.value_ref(),
            &intent,
            identity.value_ref(),
            &native,
            &evidence,
            &original,
        )
        .unwrap();
    assert_eq!(projected, evidence);
    assert!(
        <EvmBalanceObservation as ReadImplementation<EvmChainIdentityRead>>::project_evidence(
            &abi,
            binding_object.value_ref(),
            &binding,
            identity.value_ref(),
            &intent,
            route_object.value_ref(),
            &native,
            &evidence,
            &original
        )
        .is_err()
    );

    let anchor = EvmBlockAnchor {
        number: EvmU256::new(
            "115792089237316195423570985008687907853269984665640564039457584007913129639935",
        )
        .unwrap(),
        hash: EvmHash::from_bytes([3; 32]),
    };
    let anchored = EvmReadIntent::from_context(
        &context,
        EvmReadSubject::NativeBalance {
            anchor: anchor.clone(),
        },
    )
    .unwrap();
    assert_eq!(
        Object::from_value(&anchored)
            .unwrap()
            .decode::<EvmReadIntent>()
            .unwrap(),
        anchored
    );
    assert!(EvmReadIntent::from_context(
        &context,
        EvmReadSubject::TokenBalance {
            anchor: anchor.clone()
        }
    )
    .is_err());
    let mut wire = serde_json::to_value(&anchored).unwrap();
    wire["subject"]["native_balance"]["source"] = serde_json::json!({});
    assert!(serde_json::from_value::<EvmReadIntent>(wire).is_err());
    let mut old = serde_json::to_value(&intent).unwrap();
    old.as_object_mut().unwrap().remove("source_ordinal");
    assert!(serde_json::from_value::<EvmReadIntent>(old).is_err());
}

#[test]
fn native_scale_rejection_keeps_its_checked_cause_through_original_decoding() {
    use std::error::Error;
    let error = EvmDomainError::from(DecimalScale::new(31).unwrap_err());
    assert!(error.source().is_some());
    assert_eq!(
        serde_json::to_value(error).unwrap(),
        serde_json::json!({"balance_scale":{"actual":31}})
    );
    assert_eq!(
        Object::from_value(&error)
            .unwrap()
            .decode::<EvmDomainError>()
            .unwrap(),
        error
    );
    assert!(serde_json::from_value::<EvmDomainError>(
        serde_json::json!({"balance_scale":{"actual":30}})
    )
    .is_err());
}
