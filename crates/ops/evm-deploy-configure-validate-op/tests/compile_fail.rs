#[test]
fn validate_before_configure_is_a_typestate_error() {
    let target_dir = std::env::temp_dir().join(format!(
        "mfm-op-evm-deploy-configure-validate-trybuild-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&target_dir);
    std::fs::create_dir_all(&target_dir).expect("create trybuild target dir");
    std::env::set_var("CARGO_TARGET_DIR", &target_dir);

    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/fail/validate_before_configure.rs");
}
