#[test]
fn runtime_authority_requires_certified_inputs_at_compile_time() {
    let tests = trybuild::TestCases::new();
    tests.compile_fail("tests/ui/fail/uncertified_typed_spec.rs");
    tests.compile_fail("tests/ui/fail/hash_only_envelope.rs");
    tests.compile_fail("tests/ui/fail/parsed_persisted_spec.rs");
    tests.compile_fail("tests/ui/fail/lifecycle_payload_not_runner_payload.rs");
    tests.compile_fail("tests/ui/fail/kernel_payload_batch_not_runner_output.rs");
    tests.compile_fail("tests/ui/fail/artifact_reference_not_runner_payload.rs");
    tests.compile_fail("tests/ui/fail/fact_record_input_fields_private.rs");
    tests.compile_fail("tests/ui/fail/erased_runner_output_fields_private.rs");
    tests.compile_fail("tests/ui/fail/runner_capability_binding_fields_private.rs");
    tests.compile_fail("tests/ui/fail/side_effect_artifact_builder_helpers_private.rs");
    tests.compile_fail("tests/ui/fail/side_effect_payload_builder_helpers_private.rs");
    tests.compile_fail("tests/ui/fail/side_effect_callback_dto_fields_private.rs");
    tests.compile_fail("tests/ui/fail/staged_fact_record_removed.rs");
    tests.compile_fail("tests/ui/fail/staged_artifact_metadata_private.rs");
    tests.compile_fail("tests/ui/fail/side_effect_staged_artifact_helpers_private.rs");
    tests.compile_fail("tests/ui/fail/side_effect_attempt_view_private.rs");
    tests.compile_fail("tests/ui/fail/side_effect_preclaim_builder_removed.rs");
    tests.compile_fail("tests/ui/fail/side_effect_prepared_invocation_plan_removed.rs");
    tests.compile_fail("tests/ui/fail/verified_run_history_view_constructor_private.rs");
    tests.compile_fail("tests/ui/fail/verified_run_history_view_fields_private.rs");
}
