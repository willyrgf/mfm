#[test]
fn protected_raw_transaction_is_not_a_typed_value() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/fail/protected_raw_transaction_value.rs");
}
