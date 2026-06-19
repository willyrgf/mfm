#[test]
fn raw_signed_transactions_are_not_persisted_surfaces() {
    let tests = trybuild::TestCases::new();
    tests.compile_fail("tests/ui/fail/*.rs");
}
