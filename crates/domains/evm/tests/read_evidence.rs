use std::num::NonZeroU64;

use mfm_capabilities::ReadCapabilityContract;
use mfm_evm::{
    EvmChainIdentityRead, EvmReadEvidence, EvmReadIntent, EvmReadSubject, EvmReadValue, EvmU256,
};
use mfm_values::Object;

#[test]
fn wrong_chain_evidence_remains_admissible_for_domain_rejection_but_wrong_intents_and_types_do_not()
{
    let route = Object::from_value(&EvmU256::from_u64(1)).unwrap();
    let intent = EvmReadIntent::new(
        0,
        mfm_chain::balance::DecimalScale::new(0).unwrap(),
        NonZeroU64::new(1).unwrap(),
        mfm_evm::EvmBalanceTarget::new(mfm_evm::EvmAddress::from_bytes([1; 20]), None),
        route.value_ref().clone(),
        EvmReadSubject::ChainIdentity,
    )
    .unwrap();
    let identity = Object::from_value(&intent).unwrap();
    let evidence = EvmReadEvidence::returned(
        identity.value_ref().clone(),
        EvmReadValue::ChainId(NonZeroU64::new(2).unwrap()),
    );
    let original = Object::from_value(&evidence).unwrap();
    EvmChainIdentityRead::bind_evidence(
        identity.value_ref(),
        &intent,
        original.value_ref(),
        &evidence,
    )
    .unwrap();
    assert_eq!(original.decode::<EvmReadEvidence>().unwrap(), evidence);
    assert!(EvmChainIdentityRead::bind_evidence(
        route.value_ref(),
        &intent,
        original.value_ref(),
        &evidence
    )
    .is_err());
    let wrong_type = EvmReadEvidence::returned(
        identity.value_ref().clone(),
        EvmReadValue::RawUnits(EvmU256::from_u64(2)),
    );
    let wrong_original = Object::from_value(&wrong_type).unwrap();
    assert!(EvmChainIdentityRead::bind_evidence(
        identity.value_ref(),
        &intent,
        wrong_original.value_ref(),
        &wrong_type
    )
    .is_err());
}
