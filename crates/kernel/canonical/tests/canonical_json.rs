use mfm_canonical::*;

#[test]
fn canonicalizes_object_and_digest_vector() {
    let canonical =
        PlainCanonicalJsonBytes::from_json_str(r#"{"b":2,"a":1}"#).expect("canonical json");
    assert_eq!(canonical.as_str(), r#"{"a":1,"b":2}"#);
    assert_eq!(
        canonical.content_digest().as_str(),
        "content:sha256-jcs-v1:43258cff783fe7036d8a43033f830adfc60ec037382473548ac742b888292777"
    );
    assert_eq!(
        canonical.digest_bytes().to_string(),
        "43258cff783fe7036d8a43033f830adfc60ec037382473548ac742b888292777"
    );
}

#[test]
fn canonicalizes_arrays_and_rejects_noncanonical_input_bytes() {
    let canonical = PlainCanonicalJsonBytes::from_json_str(
        r#"
            [true, false, null, "x", {"z": 0, "a": -3}]
            "#,
    )
    .expect("canonical json");
    assert_eq!(
        canonical.as_str(),
        r#"[true,false,null,"x",{"a":-3,"z":0}]"#
    );

    assert!(PlainCanonicalJsonBytes::from_canonical_json_slice(
        br#"[true,false,null,"x",{"z":0,"a":-3}]"#,
    )
    .is_err());
    assert!(PlainCanonicalJsonBytes::from_canonical_json_slice(canonical.as_bytes()).is_ok());
}

#[test]
fn canonicalizes_bytes_and_decimals_as_typed_strings() {
    let value = CanonicalValue::object([
        (
            "bytes",
            CanonicalValue::Bytes(CanonicalBytes::new([0_u8, 1, 62, 255])),
        ),
        (
            "fixed",
            CanonicalValue::Decimal(DecimalString::new_fixed("-10.50", 2).expect("fixed decimal")),
        ),
        (
            "variable",
            CanonicalValue::Decimal(
                DecimalString::new_variable("12.34").expect("variable decimal"),
            ),
        ),
    ])
    .expect("object");

    let canonical = CanonicalJsonBytes::from_value(&value);
    assert_eq!(
        canonical.as_str(),
        r#"{"bytes":"AAE-_w","fixed":"-10.50","variable":"12.34"}"#
    );
    assert_eq!(
        canonical.digest_bytes().to_string(),
        "beb7444a9fc8846ade8e0c478913d36d5ba13c061881e2e134950d45cbaf8ed2"
    );
}

#[test]
fn plain_json_strings_do_not_enter_the_typed_value_surface() {
    let plain = PlainCanonicalJsonBytes::from_json_str(r#"{"amount":"1.0","bytes":"AAE+/w=="}"#)
        .expect("plain strings remain valid plain JSON");

    assert_eq!(plain.as_str(), r#"{"amount":"1.0","bytes":"AAE+/w=="}"#);

    assert!(DecimalString::new_variable("1.0").is_err());
    assert!(CanonicalBytes::from_base64url_no_pad("AAE+/w==").is_err());
}

#[test]
fn parses_and_rejects_base64url_bytes() {
    let bytes = CanonicalBytes::from_base64url_no_pad("AAE-_w").expect("base64url no padding");
    assert_eq!(bytes.as_bytes(), &[0, 1, 62, 255]);
    assert_eq!(bytes.encoded(), "AAE-_w");

    for value in ["AAE+/w==", "AAE+/w", "AAE-_w=", "A", "AB", "AA?"] {
        assert!(
            CanonicalBytes::from_base64url_no_pad(value).is_err(),
            "expected base64url rejection for {value}"
        );
    }
}

#[test]
fn sorts_object_keys_by_utf16_code_units() {
    let value = CanonicalValue::object([
        ("🦀", CanonicalValue::Unsigned(1)),
        ("\u{e000}", CanonicalValue::Unsigned(2)),
    ])
    .expect("object");

    assert_eq!(
        CanonicalJsonBytes::from_value(&value).as_str(),
        r#"{"🦀":1,"":2}"#
    );
}

#[test]
fn rejects_duplicate_object_keys() {
    let error = PlainCanonicalJsonBytes::from_json_str(r#"{"a":1,"a":2}"#)
        .expect_err("duplicate key must reject");
    assert!(error.message().contains("duplicate object key"));

    assert!(CanonicalValue::object([
        ("a", CanonicalValue::Unsigned(1)),
        ("a", CanonicalValue::Unsigned(2)),
    ])
    .is_err());
}

#[test]
fn rejects_floats_and_unsupported_number_forms() {
    for input in [
        r#"{"value":1.0}"#,
        r#"{"value":1e3}"#,
        r#"{"value":-0}"#,
        r#"{"value":01}"#,
        r#"[0, -0]"#,
        r#"{"value":NaN}"#,
        r#"{"value":Infinity}"#,
        r#"{"value":-Infinity}"#,
    ] {
        assert!(
            PlainCanonicalJsonBytes::from_json_str(input).is_err(),
            "expected rejection for {input}"
        );
    }
}

#[test]
fn rejects_noncanonical_decimal_strings() {
    for value in [
        "",
        "+1",
        "01",
        "0.1",
        "1.",
        "1.0",
        "1e3",
        "-0",
        "-0.0",
        "NaN",
        "Infinity",
        "-Infinity",
    ] {
        assert!(
            DecimalString::new_variable(value).is_err(),
            "expected variable decimal rejection for {value}"
        );
    }

    for value in ["1.00", "0.00", "-0.01", "-10.50"] {
        DecimalString::new_fixed(value, 2)
            .unwrap_or_else(|error| panic!("expected fixed decimal {value}: {error}"));
    }

    for value in ["1", "01.00", "1.0", "1.000", "-0.00"] {
        assert!(
            DecimalString::new_fixed(value, 2).is_err(),
            "expected fixed decimal rejection for {value}"
        );
    }
}
