#[test]
fn structured_authorities_are_not_constructible_cloneable_split_or_directly_invocable() {
    let tests = trybuild::TestCases::new();
    tests.compile_fail("tests/ui/structured/fail/*.rs");
}
