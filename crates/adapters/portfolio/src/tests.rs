use super::*;

#[test]
fn portfolio_capability_implementation_id_is_stable() {
    assert_eq!(CAPABILITY_IMPLEMENTATION_ID, "mfm.portfolio.runtime.v1");
    assert_eq!(PURE_FACTORY, "pure");
    assert_eq!(READ_FACTORY, "read_external");
    assert_eq!(ADAPTER_FACTORY, "portfolio_adapter");
}
