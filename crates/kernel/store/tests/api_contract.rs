#[test]
fn backend_command_and_retired_store_surface_stay_private() {
    let tests = trybuild::TestCases::new();
    tests.compile_fail("tests/ui/*.rs");
}
