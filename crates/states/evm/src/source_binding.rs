//! Serializable, redaction-safe EVM provider-source bindings.
//!
//! A joint tip establishes the one provider source that may supply every
//! hash-pinned read for its collection. The binding deliberately contains only
//! public process-local identifiers; it never serializes an endpoint,
//! credential, or transport configuration.

use mfm_evm_capabilities::{
    EvmNetworkBinding, EvmNetworkId, EvmSourcePolicyId, EvmSourceRef, RedactedEvmSourceEvidence,
};
use mfm_program_derive::MfmValue;
use serde::{de, Deserialize, Serialize};

use crate::EvmStateError;

/// Exact redacted provider-source identity for one EVM network and chain.
///
/// This value is carried by the resolved joint tip, ERC-20 metadata, and
/// collection receipts. Every live or replayed capability response must match
/// it exactly before it can influence a holding fact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[mfm(
    namespace = "mfm.evm",
    name = "redacted_provider_source_binding",
    version = "1",
    schema = "mfm.evm.state.value.redacted_provider_source_binding"
)]
pub struct RedactedEvmProviderSourceBinding {
    network: String,
    chain_id: u64,
    source_ref: String,
    policy_id: String,
}

impl RedactedEvmProviderSourceBinding {
    /// Creates a checked redacted provider-source binding.
    pub fn new(
        network: impl Into<String>,
        chain_id: u64,
        source_ref: impl Into<String>,
        policy_id: impl Into<String>,
    ) -> Result<Self, EvmStateError> {
        let network = network.into();
        let source_ref = source_ref.into();
        let policy_id = policy_id.into();
        let network_id = EvmNetworkId::new(&network).map_err(|_| invalid_binding())?;
        EvmNetworkBinding::new(network_id, chain_id).map_err(|_| invalid_binding())?;
        EvmSourceRef::new(&source_ref).map_err(|_| invalid_binding())?;
        EvmSourcePolicyId::new(&policy_id).map_err(|_| invalid_binding())?;
        Ok(Self {
            network,
            chain_id,
            source_ref,
            policy_id,
        })
    }

    /// Converts checked capability evidence into its persisted redacted binding.
    pub fn from_capability(evidence: &RedactedEvmSourceEvidence) -> Result<Self, EvmStateError> {
        let binding = Self::new(
            evidence.network_id.as_str(),
            evidence.expected_chain_id,
            evidence.source_ref.as_str(),
            evidence.policy_id.as_str(),
        )?;
        if evidence.observed_chain_id != binding.chain_id {
            return Err(EvmStateError::InvalidInput {
                reason: "EVM source evidence observed chain did not match its provider binding"
                    .to_owned(),
            });
        }
        Ok(binding)
    }

    /// Reconstructs checked capability evidence for evidence-only replay.
    pub fn to_capability(&self) -> Result<RedactedEvmSourceEvidence, EvmStateError> {
        let network_id = EvmNetworkId::new(&self.network).map_err(|_| invalid_binding())?;
        let network_binding =
            EvmNetworkBinding::new(network_id, self.chain_id).map_err(|_| invalid_binding())?;
        let source_ref = EvmSourceRef::new(&self.source_ref).map_err(|_| invalid_binding())?;
        let policy_id = EvmSourcePolicyId::new(&self.policy_id).map_err(|_| invalid_binding())?;
        RedactedEvmSourceEvidence::from_binding(
            &network_binding,
            self.chain_id,
            source_ref,
            policy_id,
        )
        .map_err(|_| invalid_binding())
    }

    /// Returns whether this binding belongs to one certified EVM network.
    pub fn is_bound_to(&self, network: &str, chain_id: u64) -> bool {
        self.network == network && self.chain_id == chain_id
    }

    /// Returns whether capability evidence exactly matches this provider source.
    pub fn matches_capability(&self, evidence: &RedactedEvmSourceEvidence) -> bool {
        evidence.network_id.as_str() == self.network
            && evidence.expected_chain_id == self.chain_id
            && evidence.observed_chain_id == self.chain_id
            && evidence.source_ref.as_str() == self.source_ref
            && evidence.policy_id.as_str() == self.policy_id
    }

    /// Returns the semantic EVM network id.
    pub fn network(&self) -> &str {
        &self.network
    }

    /// Returns the EVM chain id.
    pub const fn chain_id(&self) -> u64 {
        self.chain_id
    }

    /// Returns the redacted process-local source reference.
    pub fn source_ref(&self) -> &str {
        &self.source_ref
    }

    /// Returns the redacted source-policy id.
    pub fn policy_id(&self) -> &str {
        &self.policy_id
    }
}

impl<'de> Deserialize<'de> for RedactedEvmProviderSourceBinding {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            network: String,
            chain_id: u64,
            source_ref: String,
            policy_id: String,
        }

        let wire = Wire::deserialize(deserializer)?;
        Self::new(wire.network, wire.chain_id, wire.source_ref, wire.policy_id)
            .map_err(de::Error::custom)
    }
}

fn invalid_binding() -> EvmStateError {
    EvmStateError::InvalidInput {
        reason: "EVM redacted provider-source binding was invalid".to_owned(),
    }
}
