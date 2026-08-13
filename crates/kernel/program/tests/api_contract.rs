#[test]
fn catalog_authority_values_stay_non_constructible() {
    let tests = trybuild::TestCases::new();
    tests.compile_fail("tests/ui/*.rs");
}
