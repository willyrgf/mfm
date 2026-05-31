#[test]
fn replay_authority_rejects_raw_inputs_at_compile_time() {
    let tests = trybuild::TestCases::new();
    tests.compile_fail("tests/ui/fail/old_replay_authority_removed.rs");
    tests.compile_fail("tests/ui/fail/raw_stream_broker_constructor_removed.rs");
    tests.compile_fail("tests/ui/fail/replay_read_authority_fields_private.rs");
    tests.compile_fail("tests/ui/fail/raw_spec_replay_authority_constructor_removed.rs");
}
