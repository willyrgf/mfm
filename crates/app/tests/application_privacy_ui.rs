#[test]
fn credentials_and_backend_composition_stay_private_at_compile_time() {
    let tests = trybuild::TestCases::new();
    tests.compile_fail("tests/ui/fail/secret_credential_cannot_clone.rs");
    tests.compile_fail("tests/ui/fail/application_backend_is_not_public.rs");
    tests.compile_fail("tests/ui/fail/qualified_run_assembly_cannot_clone.rs");
    tests.compile_fail("tests/ui/fail/run_history_writer_cannot_clone.rs");
    tests.compile_fail("tests/ui/fail/run_history_reader_cannot_append.rs");
    tests.compile_fail("tests/ui/fail/postgres_backend_pool_is_not_public.rs");
    tests.compile_fail("tests/ui/fail/retired_postgres_store_facade_is_absent.rs");
    tests.compile_fail("tests/ui/fail/evm_wallet_raw_apis_are_not_public.rs");
    tests.compile_fail("tests/ui/fail/evm_wallet_transport_injection_is_closed.rs");
    tests.pass("tests/ui/pass/evm_wallet_executor_uses_qualified_transport.rs");
}
