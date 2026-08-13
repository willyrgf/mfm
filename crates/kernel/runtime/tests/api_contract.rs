#[test]
fn affine_runtime_surface_stays_non_constructible() {
    let tests = trybuild::TestCases::new();
    tests.compile_fail("tests/ui/*.rs");
}
