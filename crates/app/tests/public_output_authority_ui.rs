#[test]
fn public_output_authority_fields_stay_private_at_compile_time() {
    let tests = trybuild::TestCases::new();
    tests.compile_fail("tests/ui/fail/public_output_read_authority_fields_private.rs");
    tests.compile_fail("tests/ui/fail/production_runner_registry_is_test_only.rs");
}
