#[test]
fn runtime_and_view_authority_remain_private() {
    trybuild::TestCases::new().compile_fail("tests/ui/private_runtime_authority.rs");
}
