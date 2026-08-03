use mfm_evm::{EvmRoutingCatalogDescriptor, EvmWalletReference};
use mfm_storage_evm_postgres::QualifiedEvmRoutingCatalog;

fn descriptor() -> EvmRoutingCatalogDescriptor {
    panic!("compile-only descriptor")
}

fn fence_head() -> EvmWalletReference {
    panic!("compile-only fence head")
}

fn main() {
    let _ = QualifiedEvmRoutingCatalog {
        descriptor: descriptor(),
        provider_fence_head_ref: fence_head(),
    };
}
