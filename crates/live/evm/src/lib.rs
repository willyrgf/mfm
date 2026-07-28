#![warn(missing_docs)]
//! Exact-generation EVM JSON-RPC transport and audited read bindings.
//!
//! Mutation, replay, reducer, ingress-validation, and aggregate-reader
//! registration surfaces are intentionally absent.

mod adapter;
pub mod transport;

pub use adapter::{
    evm_adapter_callback_surface_canonical, evm_adapter_callback_surface_ref,
    evm_adapter_callback_surface_support_contract, evm_safe_classifier_canonical,
    evm_safe_classifier_contract_ref, evm_safe_classifier_support_contract,
    qualify_evm_read_entries, EvmReadAdapter, EvmReadQualificationArtifacts,
    QualifiedEvmReadEntries, EVM_ADAPTER_CALLBACK_SURFACE_VERSION,
    EVM_SAFE_CLASSIFIER_DESCRIPTOR_VERSION,
};
