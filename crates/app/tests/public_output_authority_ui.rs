#[test]
fn public_output_authority_rejects_raw_inputs_at_compile_time() {
    let tests = trybuild::TestCases::new();
    tests.compile_fail("tests/ui/fail/public_output_read_authority_fields_private.rs");
    tests.compile_fail("tests/ui/fail/raw_public_output_read_authority_constructor_removed.rs");
}
