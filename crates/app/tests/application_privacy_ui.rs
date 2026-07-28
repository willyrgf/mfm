#[test]
fn credentials_and_backend_composition_stay_private_at_compile_time() {
    let tests = trybuild::TestCases::new();
    tests.compile_fail("tests/ui/fail/secret_credential_cannot_clone.rs");
    tests.compile_fail("tests/ui/fail/application_backend_is_not_public.rs");
}
