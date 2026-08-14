#[test]
fn keystore_remains_thread_affine() {
    let tests = trybuild::TestCases::new();
    tests.compile_fail("tests/ui/*.rs");
}
