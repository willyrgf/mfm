//! Stable identities shared by the reusable EVM states.

use mfm_canonical::sha256_digest_bytes;
use mfm_ids::{AdapterKind, AdapterVersion, DigestAlgorithm, StateKind, StateVersion};
use mfm_program::AdapterBindingSpec;

const NAMESPACE: &str = "mfm.evm";
const EVM_JSONRPC_ADAPTER_NAME: &str = "jsonrpc";
const EVM_JSONRPC_ADAPTER_VERSION: &str = "mfm.evm.jsonrpc.adapter.v1";

/// Returns the stable EVM JSON-RPC adapter kind.
pub fn evm_jsonrpc_adapter_kind() -> Result<AdapterKind, mfm_ids::IdentityError> {
    AdapterKind::new(
        NAMESPACE,
        EVM_JSONRPC_ADAPTER_NAME,
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(b"mfm.evm.adapter:jsonrpc"),
    )
}

/// Returns the stable EVM JSON-RPC adapter version.
pub fn evm_jsonrpc_adapter_version() -> Result<AdapterVersion, mfm_ids::IdentityError> {
    AdapterVersion::new(EVM_JSONRPC_ADAPTER_VERSION)
}

pub(crate) fn adapter_binding() -> mfm_program::Result<Vec<AdapterBindingSpec>> {
    Ok(vec![AdapterBindingSpec {
        adapter_kind: evm_jsonrpc_adapter_kind().map_err(|error| {
            mfm_program::PlanError::Key(format!("EVM JSON-RPC adapter kind invalid: {error}"))
        })?,
        adapter_version: evm_jsonrpc_adapter_version().map_err(|error| {
            mfm_program::PlanError::Key(format!("EVM JSON-RPC adapter version invalid: {error}"))
        })?,
    }])
}

pub(crate) fn state_kind(name: &'static str) -> mfm_program::Result<StateKind> {
    StateKind::new(
        NAMESPACE,
        name,
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(format!("mfm.evm.state:{name}").as_bytes()),
    )
    .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
}

pub(crate) fn state_version(name: &'static str) -> mfm_program::Result<StateVersion> {
    StateVersion::new(format!("mfm.evm.state.{name}.v1"))
        .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
}
