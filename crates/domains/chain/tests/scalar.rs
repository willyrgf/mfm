use mfm_chain::{AdditionInputError, AdditionOverflow, CheckedAdd, CheckedAddition};
use mfm_program::{Classification, ClassifyError, ProposedStateOutcome, PureState};
use mfm_values::{Object, Unsigned256, Unsigned256Error};

use mfm_program as source;

#[test]
fn checked_add_is_a_source_with_real_input_and_full_width_output_contracts() {
    fn accept<S: source::AuthoringSource<Input = CheckedAddition, Output = Unsigned256>>(_: &S) {}
    accept(&source::Pure::<CheckedAdd>::default());
    let input = CheckedAddition::new("42", "42").unwrap();
    let admitted = Object::from_value(&input).unwrap();
    let ProposedStateOutcome::Success { output } =
        CheckedAdd::evaluate(admitted.decode().unwrap()).unwrap()
    else {
        panic!("42 + 42 must succeed")
    };
    let result = Object::from_value(&output).unwrap();
    assert_eq!(result.decode::<Unsigned256>().unwrap().to_string(), "84");
    assert_eq!(admitted.canonical_bytes(), br#"{"left":"42","right":"42"}"#);
    assert!(result.decode::<CheckedAddition>().is_err());
}

#[test]
fn overflow_is_a_permanent_original_and_input_keeps_both_operands() {
    let input = CheckedAddition::new(
        "115792089237316195423570985008687907853269984665640564039457584007913129639935",
        "1",
    )
    .unwrap();
    let admitted = Object::from_value(&input).unwrap();
    let ProposedStateOutcome::Failure { failure } =
        CheckedAdd::evaluate(admitted.decode().unwrap()).unwrap()
    else {
        panic!("max + 1 must overflow")
    };
    let original = Object::from_value(&failure).unwrap();
    let cold = original.decode::<AdditionOverflow>().unwrap();
    assert_eq!(cold.classify(), Classification::Permanent);
    assert_eq!(original.canonical_bytes(), b"{}");
    assert_eq!(admitted.decode::<CheckedAddition>().unwrap(), input);
}

#[test]
fn addition_input_construction_and_decoding_share_scalar_bounds_and_closed_fields() {
    assert_eq!(
        CheckedAddition::new("01", "1"),
        Err(AdditionInputError::Left(Unsigned256Error::NonCanonical))
    );
    assert_eq!(
        CheckedAddition::new("1", "-1"),
        Err(AdditionInputError::Right(Unsigned256Error::NonCanonical))
    );
    for wire in [
        r#"{"left":"01","right":"1"}"#,
        r#"{"left":"1","right":"-1"}"#,
        r#"{"left":"1"}"#,
        r#"{"left":"1","right":"2","extra":0}"#,
        r#"{"left":"1","right":"2","left":"3"}"#,
    ] {
        assert!(serde_json::from_str::<CheckedAddition>(wire).is_err());
    }
}

#[test]
fn constructor_rejection_keeps_the_operand_and_concrete_scalar_cause() {
    let error = CheckedAddition::new("1", "01").unwrap_err();
    assert_eq!(
        std::error::Error::source(&error).unwrap().downcast_ref(),
        Some(&Unsigned256Error::NonCanonical)
    );
    assert_eq!(
        serde_json::to_value(&error).unwrap(),
        serde_json::json!({"right": "non_canonical"})
    );
}
