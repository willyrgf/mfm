use mfm_capabilities::CapabilitySpec;
use mfm_evm_capabilities::{EvmReadCapability, EvmTransactionCapability};

#[test]
fn capability_surface_has_exactly_two_authority_names() {
    let names = [EvmReadCapability::name(), EvmTransactionCapability::name()];
    assert_eq!(names, ["mfm.evm.read", "mfm.evm.transaction"]);
    assert_ne!(
        EvmReadCapability::kind().expect("read kind"),
        EvmTransactionCapability::kind().expect("transaction kind")
    );
}

#[test]
fn contracts_contain_no_runtime_secret_storage() {
    let source = include_str!("../src/lib.rs");
    for forbidden in [
        concat!("rpc", "_", "url"),
        concat!("author", "ization"),
        concat!("api", "_", "key"),
        concat!("private", "_", "key"),
        concat!("key", "store", "_", "path"),
    ] {
        assert!(!source.contains(forbidden), "forbidden detail: {forbidden}");
    }
}
