#[test]
fn public_output_authority_rejects_raw_inputs_at_compile_time() {
    let target_dir = std::env::temp_dir().join(format!(
        "mfm-app-public-output-trybuild-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&target_dir);
    std::fs::create_dir_all(&target_dir).expect("create trybuild target dir");
    std::env::set_var("CARGO_TARGET_DIR", &target_dir);

    let tests = trybuild::TestCases::new();
    tests.compile_fail("tests/ui/fail/public_output_read_authority_fields_private.rs");
    tests.compile_fail("tests/ui/fail/raw_public_output_read_authority_constructor_removed.rs");
}
