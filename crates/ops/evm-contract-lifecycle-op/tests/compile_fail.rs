#[test]
fn typestate_ordering_compile_failures() {
    let t = trybuild::TestCases::new();
    let cases = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/ui/fail/*.rs");
    t.compile_fail(cases.to_str().expect("utf-8 ui test path"));
}
