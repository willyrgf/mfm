#![warn(missing_docs)]
//! Neutral adapter contract identities and binding descriptors.
//!
//! Adapter contracts identify behaviorally relevant adapter obligations without
//! depending on adapter implementation crates. State crates can record these
//! descriptors in certified specs while implementation crates remain under
//! `crates/adapters/*`.
//!
//! ```rust
//! use mfm_adapter_contracts::{
//!     evm_contract_lifecycle_adapter_binding, evm_contract_lifecycle_adapter_kind,
//! };
//!
//! let kind = evm_contract_lifecycle_adapter_kind()?;
//! let binding = evm_contract_lifecycle_adapter_binding()?;
//! assert_eq!(binding.adapter_kind(), &kind);
//! assert!(!binding.required_capabilities().is_empty());
//! # Ok::<(), mfm_adapter_contracts::AdapterContractError>(())
//! ```

use std::collections::BTreeSet;

use mfm_canonical::sha256_digest_bytes;
use mfm_capabilities::{CapabilityError, CapabilitySpec};
use mfm_evm_capabilities::{
    EvmBlockReadCapability, EvmCallReadCapability, EvmChainIdentityCapability,
    EvmFeeReadCapability, EvmGasEstimateCapability, EvmLogsReadCapability,
    EvmNonceOccupancyReadCapability, EvmNonceReadCapability, EvmReceiptReadCapability,
    EvmTransactionSubmitCapability,
};
use mfm_ids::{AdapterKind, AdapterVersion, CapabilityKind, DigestAlgorithm};

/// Result type for adapter contract descriptors.
pub type Result<T> = std::result::Result<T, AdapterContractError>;

/// Namespace used for generic EVM adapter contract identities.
pub const EVM_ADAPTER_NAMESPACE: &str = "mfm.evm";

/// Stable name for the EVM contract lifecycle adapter contract.
pub const EVM_CONTRACT_LIFECYCLE_ADAPTER_NAME: &str = "contract_lifecycle";

/// Stable version for the EVM contract lifecycle adapter contract.
pub const EVM_CONTRACT_LIFECYCLE_ADAPTER_VERSION: &str = "mfm.evm.contract_lifecycle.adapter.v1";

/// Stable adapter contract binding descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdapterBindingDescriptor {
    adapter_kind: AdapterKind,
    adapter_version: AdapterVersion,
    required_capabilities: Vec<CapabilityKind>,
}

impl AdapterBindingDescriptor {
    /// Creates a binding descriptor with no live IO fields.
    ///
    /// The descriptor carries only stable adapter identity plus capability contract
    /// identities. Runtime endpoints, credentials, signer material, and source routing
    /// are intentionally not representable here.
    pub fn new(
        adapter_kind: AdapterKind,
        adapter_version: AdapterVersion,
        required_capabilities: Vec<CapabilityKind>,
    ) -> Result<Self> {
        let descriptor = Self {
            adapter_kind,
            adapter_version,
            required_capabilities,
        };
        descriptor.validate_no_live_io()?;
        Ok(descriptor)
    }

    /// Returns the stable adapter kind id.
    pub const fn adapter_kind(&self) -> &AdapterKind {
        &self.adapter_kind
    }

    /// Returns the stable adapter contract version.
    pub const fn adapter_version(&self) -> &AdapterVersion {
        &self.adapter_version
    }

    /// Returns capability contracts required by this adapter contract.
    pub fn required_capabilities(&self) -> &[CapabilityKind] {
        &self.required_capabilities
    }

    /// Validates that this descriptor remains a no-live-IO contract descriptor.
    ///
    /// Since the type has no endpoint, credential, artifact store, signer provider, or
    /// transport fields, validation focuses on keeping capability authority explicit
    /// and unambiguous.
    pub fn validate_no_live_io(&self) -> Result<()> {
        if self.required_capabilities.is_empty() {
            return Err(AdapterContractError::EmptyCapabilitySet);
        }

        let mut seen = BTreeSet::new();
        for capability in &self.required_capabilities {
            if !seen.insert(capability) {
                return Err(AdapterContractError::DuplicateCapability {
                    capability: capability.clone(),
                });
            }
        }

        Ok(())
    }
}

/// Validates a binding descriptor's no-live-IO adapter contract shape.
pub fn validate_no_live_io_binding(descriptor: &AdapterBindingDescriptor) -> Result<()> {
    descriptor.validate_no_live_io()
}

/// Constructs the stable EVM contract lifecycle adapter kind id.
pub fn evm_contract_lifecycle_adapter_kind() -> Result<AdapterKind> {
    AdapterKind::new(
        EVM_ADAPTER_NAMESPACE,
        EVM_CONTRACT_LIFECYCLE_ADAPTER_NAME,
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(b"mfm.evm.adapter:contract_lifecycle"),
    )
    .map_err(|error| AdapterContractError::Identity(error.to_string()))
}

/// Constructs the stable EVM contract lifecycle adapter contract version.
pub fn evm_contract_lifecycle_adapter_version() -> Result<AdapterVersion> {
    AdapterVersion::new(EVM_CONTRACT_LIFECYCLE_ADAPTER_VERSION)
        .map_err(|error| AdapterContractError::Identity(error.to_string()))
}

/// Constructs the stable EVM contract lifecycle adapter binding descriptor.
pub fn evm_contract_lifecycle_adapter_binding() -> Result<AdapterBindingDescriptor> {
    AdapterBindingDescriptor::new(
        evm_contract_lifecycle_adapter_kind()?,
        evm_contract_lifecycle_adapter_version()?,
        evm_contract_lifecycle_required_capabilities()?,
    )
}

/// Returns capability contracts required by the EVM contract lifecycle adapter.
pub fn evm_contract_lifecycle_required_capabilities() -> Result<Vec<CapabilityKind>> {
    Ok(vec![
        capability_kind::<EvmChainIdentityCapability>()?,
        capability_kind::<EvmBlockReadCapability>()?,
        capability_kind::<EvmCallReadCapability>()?,
        capability_kind::<EvmLogsReadCapability>()?,
        capability_kind::<EvmNonceReadCapability>()?,
        capability_kind::<EvmFeeReadCapability>()?,
        capability_kind::<EvmGasEstimateCapability>()?,
        capability_kind::<EvmTransactionSubmitCapability>()?,
        capability_kind::<EvmReceiptReadCapability>()?,
        capability_kind::<EvmNonceOccupancyReadCapability>()?,
    ])
}

fn capability_kind<C>() -> Result<CapabilityKind>
where
    C: CapabilitySpec,
{
    C::kind().map_err(capability_error)
}

fn capability_error(error: CapabilityError) -> AdapterContractError {
    AdapterContractError::Capability(error.to_string())
}

/// Adapter contract descriptor error.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AdapterContractError {
    /// Adapter identity construction failed.
    #[error("adapter contract identity failed: {0}")]
    Identity(String),
    /// Capability contract identity construction failed.
    #[error("adapter contract capability failed: {0}")]
    Capability(String),
    /// Binding descriptors must declare explicit capability authority.
    #[error("adapter contract binding requires at least one capability")]
    EmptyCapabilitySet,
    /// Binding descriptor declared a capability more than once.
    #[error("adapter contract binding has duplicate capability {capability}")]
    DuplicateCapability {
        /// Duplicate capability kind.
        capability: CapabilityKind,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evm_contract_lifecycle_ids_are_stable_and_not_workflow_named() {
        let kind = evm_contract_lifecycle_adapter_kind().expect("kind");
        let version = evm_contract_lifecycle_adapter_version().expect("version");

        assert_eq!(kind.canonical_name(), Some("mfm.evm/contract_lifecycle"));
        assert_eq!(version.as_str(), EVM_CONTRACT_LIFECYCLE_ADAPTER_VERSION);

        let rendered = format!("{kind} {version}");
        for term in stale_recipe_terms() {
            assert!(!rendered.contains(&term));
        }
    }

    #[test]
    fn evm_contract_lifecycle_binding_records_capability_contracts_only() {
        let descriptor = evm_contract_lifecycle_adapter_binding().expect("binding");

        validate_no_live_io_binding(&descriptor).expect("no live io binding");
        assert_eq!(
            descriptor.adapter_kind(),
            &evm_contract_lifecycle_adapter_kind().expect("kind")
        );
        assert!(descriptor
            .required_capabilities()
            .iter()
            .any(|capability| capability.canonical_name() == Some("mfm.evm/transaction.submit")));
        assert!(descriptor.required_capabilities().iter().any(|capability| {
            capability.canonical_name() == Some("mfm.evm/nonce_occupancy.read")
        }));
    }

    #[test]
    fn binding_descriptor_rejects_duplicate_capabilities() {
        let capability = capability_kind::<EvmChainIdentityCapability>().expect("capability");
        let error = AdapterBindingDescriptor::new(
            evm_contract_lifecycle_adapter_kind().expect("kind"),
            evm_contract_lifecycle_adapter_version().expect("version"),
            vec![capability.clone(), capability.clone()],
        )
        .expect_err("duplicate capability");

        assert_eq!(
            error,
            AdapterContractError::DuplicateCapability { capability }
        );
    }

    #[test]
    fn binding_descriptor_rejects_empty_capability_sets() {
        let error = AdapterBindingDescriptor::new(
            evm_contract_lifecycle_adapter_kind().expect("kind"),
            evm_contract_lifecycle_adapter_version().expect("version"),
            Vec::new(),
        )
        .expect_err("empty capabilities");

        assert_eq!(error, AdapterContractError::EmptyCapabilitySet);
    }

    fn stale_recipe_terms() -> [String; 3] {
        [
            ["d", "c", "v"].concat(),
            ["deploy", "_configure", "_validate"].concat(),
            ["deploy", "-configure", "-validate"].concat(),
        ]
    }
}
