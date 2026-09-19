//! Native qualification derives references from retained endpoint names, without live resources.
use mfm_capabilities::CallbackFailure;
use mfm_chain::balance::{
    BalanceExecutionConfig, BalanceRead, BalanceSource, BalanceSourceDefinition, DecimalScale,
};
use mfm_chain::{BalanceTarget, LedgerIdentity};
use mfm_evm::{
    EvmAddress, EvmBalanceLedger, EvmBalanceRoute, EvmBalanceTarget, EvmEndpoint, EvmNativeBalance,
};
use mfm_program::{NoParams, ResolveReadBinding};
use mfm_values::Object;
use std::num::NonZeroU64;

#[test]
fn retained_public_route_qualifies_expected_reference_and_reconstructs_endpoint_name() {
    let chain = NonZeroU64::new(1).unwrap();
    let route = EvmBalanceRoute::new(chain, EvmEndpoint::new("primary").unwrap());
    let expected = route.route_ref().unwrap();
    let execution =
        BalanceExecutionConfig::new(expected.clone(), Object::from_value(&route).unwrap());
    let cold = Object::from_value(&execution)
        .unwrap()
        .decode::<BalanceExecutionConfig>()
        .unwrap();
    let restored = EvmBalanceRoute::from_execution(&cold).unwrap();
    assert_eq!(restored.endpoint().endpoint_id(), "primary");
    assert_eq!(restored.route_ref().unwrap(), expected);
    assert_eq!(
        restored.physical_target().unwrap(),
        route.physical_target().unwrap()
    );

    let source = BalanceSource::new(
        "account".into(),
        BalanceTarget::new(
            LedgerIdentity::new(Object::from_value(&EvmBalanceLedger::new(chain)).unwrap()),
            Object::from_value(&EvmBalanceTarget::new(
                EvmAddress::from_bytes([1; 20]),
                None,
            ))
            .unwrap(),
        ),
    )
    .unwrap();
    let demand =
        BalanceSourceDefinition::<NoParams>::new(source, 3, DecimalScale::new(18).unwrap(), cold);
    let binding =
        <EvmNativeBalance as ResolveReadBinding<_, BalanceRead>>::binding(&demand).unwrap();
    assert_eq!(binding.source_ordinal(), 3);
    assert_eq!(binding.route(), &route.physical_target().unwrap());

    let other = EvmBalanceRoute::new(chain, EvmEndpoint::new("alternate").unwrap());
    let forged = BalanceExecutionConfig::new(expected.clone(), Object::from_value(&other).unwrap());
    let Err(CallbackFailure::Execute(cause)) = EvmBalanceRoute::from_execution(&forged) else {
        panic!("route mismatch")
    };
    assert_eq!(cause.operation(), "qualify_balance_route");
    assert_eq!(
        cause.details().as_value()["expected"],
        serde_json::to_value(&expected).unwrap()
    );
    assert_eq!(
        cause.details().as_value()["actual"],
        serde_json::to_value(other.route_ref().unwrap()).unwrap()
    );
    let forged = BalanceExecutionConfig::new(expected, Object::from_value(&NoParams).unwrap());
    assert!(matches!(
        EvmBalanceRoute::from_execution(&forged),
        Err(CallbackFailure::Decode(_))
    ));
}
