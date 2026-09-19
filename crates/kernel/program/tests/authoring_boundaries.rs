// Consumers cannot implement a source traversal or forge executable Program internals.
#[test]
fn authoring_authority_remains_private() {
    let tests = trybuild::TestCases::new();
    tests.compile_fail("tests/ui/private_draft.rs");
    tests.compile_fail("tests/ui/sealed_source.rs");
    tests.compile_fail("tests/ui/private_program.rs");
}
