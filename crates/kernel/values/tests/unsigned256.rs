use mfm_values::{Object, Unsigned256, Unsigned256Error};

#[test]
fn decimal_construction_and_decoding_reject_noncanonical_and_out_of_range_values() {
    for value in ["", "00", "01", "-1", "+1", "1.0", " 1", "1 ", "1e2", "１２"] {
        assert_eq!(Unsigned256::new(value), Err(Unsigned256Error::NonCanonical));
        assert!(serde_json::from_value::<Unsigned256>(serde_json::json!(value)).is_err());
    }
    for value in [
        "115792089237316195423570985008687907853269984665640564039457584007913129639936",
        "1000000000000000000000000000000000000000000000000000000000000000000000000000000",
    ] {
        assert_eq!(Unsigned256::new(value), Err(Unsigned256Error::OutOfRange));
        assert!(serde_json::from_value::<Unsigned256>(serde_json::json!(value)).is_err());
    }
    assert!(serde_json::from_str::<Unsigned256>("42").is_err());
    assert!(serde_json::from_str::<Unsigned256>("null").is_err());
}

#[test]
fn checked_add_preserves_decimal_carry_zero_and_full_width_overflow_boundaries() {
    for (left, right, expected) in [
        ("0", "0", Some("0")),
        ("0", "42", Some("42")),
        ("42", "42", Some("84")),
        ("99", "1", Some("100")),
        ("99999999999999999999", "123", Some("100000000000000000122")),
        (
            "340282366920938463463374607431768211455",
            "1",
            Some("340282366920938463463374607431768211456"),
        ),
        (
            "115792089237316195423570985008687907853269984665640564039457584007913129639934",
            "1",
            Some("115792089237316195423570985008687907853269984665640564039457584007913129639935"),
        ),
        (
            "115792089237316195423570985008687907853269984665640564039457584007913129639935",
            "0",
            Some("115792089237316195423570985008687907853269984665640564039457584007913129639935"),
        ),
        (
            "115792089237316195423570985008687907853269984665640564039457584007913129639935",
            "1",
            None,
        ),
        (
            "99999999999999999999999999999999999999999999999999999999999999999999999999999",
            "99999999999999999999999999999999999999999999999999999999999999999999999999999",
            None,
        ),
    ] {
        let left = Unsigned256::new(left).unwrap();
        let right = Unsigned256::new(right).unwrap();
        let sum = left.checked_add(&right);
        assert_eq!(sum.as_ref().map(Unsigned256::as_str), expected);
        assert_eq!(right.checked_add(&left), sum);
        if let Some(sum) = sum {
            let object = Object::from_value(&sum).unwrap();
            assert_eq!(object.decode::<Unsigned256>().unwrap(), sum);
        }
    }
}

#[test]
fn addition_matches_machine_arithmetic_where_both_ranges_overlap() {
    for left in [0_u64, 1, 9, 10, 99, 100, u64::MAX / 2, u64::MAX] {
        for right in [0_u64, 1, 9, 999, u64::MAX] {
            let expected = (u128::from(left) + u128::from(right)).to_string();
            assert_eq!(
                Unsigned256::from_u64(left)
                    .checked_add(&Unsigned256::from_u64(right))
                    .unwrap()
                    .as_str(),
                expected,
            );
        }
    }
}

#[test]
fn scalar_and_rejection_objects_have_checked_canonical_wire() {
    let value = Unsigned256::new("340282366920938463463374607431768211455").unwrap();
    assert_eq!(value.to_u128(), Some(u128::MAX));
    let next = value.checked_add(&Unsigned256::from_u64(1)).unwrap();
    assert_eq!(next.to_u128(), None);
    assert_eq!(
        next.into_string(),
        "340282366920938463463374607431768211456"
    );
    let zero = Unsigned256::from_u64(0);
    assert!(zero.is_zero());
    assert_eq!(zero.to_string(), "0");
    assert_eq!(
        Object::from_value(&zero).unwrap().canonical_bytes(),
        br#""0""#
    );
    assert!(!Unsigned256::from_u64(1).is_zero());
    for (error, wire) in [
        (
            Unsigned256Error::NonCanonical,
            br#""non_canonical""#.as_slice(),
        ),
        (
            Unsigned256Error::OutOfRange,
            br#""out_of_range""#.as_slice(),
        ),
    ] {
        let object = Object::from_value(&error).unwrap();
        assert_eq!(object.canonical_bytes(), wire);
        assert_eq!(object.decode::<Unsigned256Error>().unwrap(), error);
    }
}
