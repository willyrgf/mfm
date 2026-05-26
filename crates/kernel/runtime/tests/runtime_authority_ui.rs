#[test]
fn runtime_authority_rejects_raw_inputs_at_compile_time() {
    let tests = trybuild::TestCases::new();
    tests.compile_fail("tests/ui/fail/raw_typed_spec.rs");
    tests.compile_fail("tests/ui/fail/hash_only_envelope.rs");
    tests.compile_fail("tests/ui/fail/parsed_bundle.rs");
}
