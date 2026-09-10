#[test]
fn runtime_and_view_authority_remain_private() {
    trybuild::TestCases::new().compile_fail("tests/ui/private_runtime_authority.rs");
}

#[test]
fn executable_integration_requires_intrinsic_error_classification() {
    trybuild::TestCases::new().compile_fail("tests/ui/classification_required.rs");
}
