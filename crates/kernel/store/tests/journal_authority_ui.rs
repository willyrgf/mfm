#[test]
fn committed_journal_authority_cannot_be_forged_or_duplicated() {
    let tests = trybuild::TestCases::new();
    tests.compile_fail("tests/ui/fail/committed_run_journal_fields_private.rs");
    tests.compile_fail("tests/ui/fail/committed_run_journal_is_not_clone.rs");
    tests.compile_fail("tests/ui/fail/committed_run_journal_raw_constructor_private.rs");
    tests.compile_fail("tests/ui/fail/journal_load_verifier_constructor_private.rs");
    tests.compile_fail("tests/ui/fail/journal_load_verifier_fields_private.rs");
    tests.compile_fail("tests/ui/fail/journal_load_verifier_is_not_clone.rs");
    tests.compile_fail("tests/ui/fail/raw_journal_backend_minter_absent.rs");
    tests.compile_fail("tests/ui/fail/run_journal_store_direct_impl_sealed.rs");
    tests.compile_fail("tests/ui/fail/run_journal_backend_cannot_override_contract.rs");
    tests.compile_fail("tests/ui/fail/verified_run_view_fields_private.rs");
    tests.compile_fail("tests/ui/fail/verified_run_view_is_not_clone.rs");
    tests.compile_fail("tests/ui/fail/legacy_run_history_authorities_absent.rs");
}

#[test]
fn journal_load_verifier_can_cross_the_async_backend_boundary() {
    fn require_send<T: Send>() {}

    require_send::<mfm_store::v1::JournalLoadVerifier>();
}

#[test]
fn manual_terminal_authority_requires_the_verified_view_fold() {
    let tests = trybuild::TestCases::new();
    tests.compile_fail("tests/ui/fail/caller_manual_proof_cannot_mint_terminal_authority.rs");
    tests.compile_fail("tests/ui/fail/projection_cannot_mint_terminal_authority.rs");
}
