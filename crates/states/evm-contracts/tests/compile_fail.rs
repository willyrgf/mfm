#[test]
fn compile_failures() {
    let temp = std::env::temp_dir().join(format!(
        "mfm-state-evm-contracts-trybuild-{}",
        std::process::id()
    ));
    let cases = trybuild::TestCases::new();
    cases.compile_fail("tests/ui/fail/*.rs");
    drop(std::fs::remove_dir_all(temp));
}
