#[test]
fn signing_contract_ui_boundaries_hold() {
    let tests = trybuild::TestCases::new();
    tests.compile_fail("tests/ui/fail/*.rs");
}
