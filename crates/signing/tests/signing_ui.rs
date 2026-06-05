#[test]
fn signing_contract_ui_boundaries_hold() {
    let target_dir =
        std::env::temp_dir().join(format!("mfm-signing-trybuild-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&target_dir);
    std::fs::create_dir_all(&target_dir).expect("create trybuild target dir");
    std::env::set_var("CARGO_TARGET_DIR", &target_dir);

    let tests = trybuild::TestCases::new();
    tests.compile_fail("tests/ui/fail/*.rs");
}
