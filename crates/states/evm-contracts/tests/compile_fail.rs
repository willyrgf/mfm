#[test]
fn typestate_ordering_compile_failures() {
    let cases = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/ui/fail/*.rs");
    trybuild::TestCases::new().compile_fail(cases.to_str().expect("UTF-8 test path"));
}
