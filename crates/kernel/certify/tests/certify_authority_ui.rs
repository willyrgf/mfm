#[test]
fn certifier_authority_rejects_forged_public_inputs() {
    let tests = trybuild::TestCases::new();
    tests.compile_fail("tests/ui/fail/*.rs");
}
