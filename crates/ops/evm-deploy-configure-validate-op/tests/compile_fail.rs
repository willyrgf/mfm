#[test]
fn validate_before_configure_is_a_typestate_error() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/fail/validate_before_configure.rs");
}
