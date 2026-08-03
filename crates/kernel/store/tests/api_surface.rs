//! Compile-fail surface proofs for sealed store mutation authority.

#[test]
fn sealed_store_surfaces_fail_to_compile() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/fail/*.rs");
}
