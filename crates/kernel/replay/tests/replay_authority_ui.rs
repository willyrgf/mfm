#[test]
fn replay_authority_boundary_stays_sealed_at_compile_time() {
    let tests = trybuild::TestCases::new();
    tests.compile_fail("tests/ui/fail/*.rs");
}
