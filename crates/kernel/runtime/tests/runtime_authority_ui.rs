#[test]
fn runtime_authority_rejects_raw_inputs_at_compile_time() {
    let tests = trybuild::TestCases::new();
    tests.compile_fail("tests/ui/fail/raw_typed_spec.rs");
    tests.compile_fail("tests/ui/fail/hash_only_envelope.rs");
    tests.compile_fail("tests/ui/fail/parsed_bundle.rs");
    tests.compile_fail("tests/ui/fail/lifecycle_payload_not_runner_payload.rs");
    tests.compile_fail("tests/ui/fail/kernel_payload_batch_not_runner_output.rs");
    tests.compile_fail("tests/ui/fail/artifact_reference_not_runner_payload.rs");
    tests.compile_fail("tests/ui/fail/raw_verified_run_history_constructor.rs");
    tests.compile_fail("tests/ui/fail/raw_verified_run_history_view_constructor.rs");
    tests.compile_fail("tests/ui/fail/verified_run_history_view_fields_private.rs");
}
