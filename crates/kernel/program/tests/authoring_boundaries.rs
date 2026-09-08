#[test]
fn authoring_authority_remains_scoped() {
    let tests = trybuild::TestCases::new();
    tests.compile_fail("tests/ui/scoped_authoring.rs");
    tests.compile_fail("tests/ui/scoped_escape.rs");
    tests.compile_fail("tests/ui/private_program.rs");
}
