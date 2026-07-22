#[test]
fn public_output_authority_fields_stay_private_at_compile_time() {
    let tests = trybuild::TestCases::new();
    tests.compile_fail("tests/ui/fail/public_output_read_authority_fields_private.rs");
    tests.compile_fail("tests/ui/fail/production_assembly_is_not_public.rs");
}
