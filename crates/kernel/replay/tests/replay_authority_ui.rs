#[test]
fn replay_authority_fields_stay_private_at_compile_time() {
    let tests = trybuild::TestCases::new();
    tests.compile_fail("tests/ui/fail/replay_read_authority_fields_private.rs");
}
