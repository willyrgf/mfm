use mfm_chain::transaction::ConfigurationValue;
use mfm_values::{Object, Unsigned256, Unsigned256Error};

#[test]
fn configuration_values_keep_full_width_arithmetic_and_their_own_contract() {
    let value = ConfigurationValue::new("18446744073709551616").unwrap();
    let doubled = value.checked_add(&value).unwrap();
    assert_eq!(doubled.to_string(), "36893488147419103232");
    assert!(!doubled.is_zero());
    assert!(ConfigurationValue::new("0").unwrap().is_zero());
    assert_eq!(
        serde_json::to_string(&doubled).unwrap(),
        "\"36893488147419103232\""
    );
    let object = Object::from_value(&doubled).unwrap();
    assert_eq!(object.decode::<ConfigurationValue>().unwrap(), doubled);
    // Equal decimal bytes do not permit exchanging purpose-specific value contracts.
    assert!(object.decode::<Unsigned256>().is_err());
}

#[test]
fn configuration_value_construction_and_decoding_reject_noncanonical_or_overflow_values() {
    for text in ["", "01", "-1", "1.0", " 1"] {
        assert_eq!(
            ConfigurationValue::new(text).unwrap_err(),
            Unsigned256Error::NonCanonical
        );
        assert!(
            serde_json::from_str::<ConfigurationValue>(&serde_json::to_string(text).unwrap())
                .is_err()
        );
    }
    let max = ConfigurationValue::new(
        "115792089237316195423570985008687907853269984665640564039457584007913129639935",
    )
    .unwrap();
    assert!(max
        .checked_add(&ConfigurationValue::new("1").unwrap())
        .is_none());
    let above = "115792089237316195423570985008687907853269984665640564039457584007913129639936";
    assert_eq!(
        ConfigurationValue::new(above).unwrap_err(),
        Unsigned256Error::OutOfRange
    );
    assert!(
        serde_json::from_str::<ConfigurationValue>(&serde_json::to_string(above).unwrap()).is_err()
    );
}
