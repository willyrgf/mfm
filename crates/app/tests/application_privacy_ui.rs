#[test]
fn structured_application_authorities_remain_closed() {
    let tests = trybuild::TestCases::new();
    tests.compile_fail("tests/ui/structured/fail/*.rs");
}
