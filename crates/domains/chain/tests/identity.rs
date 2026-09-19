use mfm_chain::{ContractLocator, LedgerIdentity, ObservationPoint};
use mfm_values::{Object, Unsigned256};

#[test]
fn shared_envelopes_retain_native_identity_without_conflating_purposes() {
    // Scalar stand-ins exercise envelope custody, not EVM ledger or location validation.
    let ledger_native = Object::from_value(&Unsigned256::new("1").unwrap()).unwrap();
    let location_native = Object::from_value(&Unsigned256::new("42").unwrap()).unwrap();
    let locator = ContractLocator::new(
        LedgerIdentity::new(ledger_native.clone()),
        location_native.clone(),
    );
    let stored = Object::from_value(&locator).unwrap();
    let decoded = stored.decode::<ContractLocator>().unwrap();
    assert_eq!(decoded.ledger().native(), &ledger_native);
    assert_eq!(decoded.native(), &location_native);
    assert!(stored.decode::<ObservationPoint>().is_err());

    let mut wire = serde_json::to_value(&locator).unwrap();
    wire["ledger"]["native"]["canonical"] = "2".into();
    assert!(serde_json::from_str::<ContractLocator>(&wire.to_string()).is_err());
}
