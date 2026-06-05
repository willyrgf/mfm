#[test]
fn typestate_ordering_compile_failures() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/fail/*.rs");
}
