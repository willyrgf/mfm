#[test]
fn structured_authority_and_nominal_types_are_not_forgeable() {
    let tests = trybuild::TestCases::new();
    tests.compile_fail("tests/ui/structured/fail/*.rs");
}
