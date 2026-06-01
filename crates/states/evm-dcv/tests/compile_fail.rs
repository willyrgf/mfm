#[test]
fn protected_raw_transaction_is_not_exposed() {
    let target_dir =
        std::env::temp_dir().join(format!("mfm-state-evm-dcv-trybuild-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&target_dir);
    std::fs::create_dir_all(&target_dir).expect("create trybuild target dir");
    std::env::set_var("CARGO_TARGET_DIR", &target_dir);

    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/fail/protected_raw_transaction_value.rs");
}
