//! Serializable redacted provenance for a source-bound EVM session.

use mfm_evm_capabilities::{EvmNetworkBinding, EvmSessionEvidence};
use mfm_ids::LocalPublicId;
use mfm_program_derive::MfmValue;
use serde::{de, Deserialize, Serialize};

use crate::EvmStateError;

/// Persisted redacted session evidence used by the pre-collapse collector graph.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[mfm(
    namespace = "mfm.evm",
    name = "redacted_session_evidence",
    version = "1",
    schema = "mfm.evm.state.value.redacted_session_evidence"
)]
pub struct RedactedEvmSessionEvidence {
    network: String,
    chain_id: u64,
    source_ref: String,
    implementation_id: String,
}

impl RedactedEvmSessionEvidence {
    /// Creates checked redacted session evidence.
    pub fn new(
        network: impl Into<String>,
        chain_id: u64,
        source_ref: impl Into<String>,
        implementation_id: impl Into<String>,
    ) -> Result<Self, EvmStateError> {
        let network = network.into();
        let source_ref = source_ref.into();
        let implementation_id = implementation_id.into();
        let network_id = LocalPublicId::new(&network).map_err(|_| invalid_binding())?;
        EvmNetworkBinding::new(network_id, chain_id).map_err(|_| invalid_binding())?;
        LocalPublicId::new(&source_ref).map_err(|_| invalid_binding())?;
        LocalPublicId::new(&implementation_id).map_err(|_| invalid_binding())?;
        Ok(Self {
            network,
            chain_id,
            source_ref,
            implementation_id,
        })
    }

    /// Converts bind-time capability evidence into persisted provenance.
    pub fn from_session(evidence: &EvmSessionEvidence) -> Result<Self, EvmStateError> {
        Self::new(
            evidence.network_id().as_str(),
            evidence.chain_id(),
            evidence.source_ref().as_str(),
            evidence.implementation_id().as_str(),
        )
    }

    /// Reconstructs the checked evidence value for replay reducers.
    pub fn to_session(&self) -> Result<EvmSessionEvidence, EvmStateError> {
        let network_id = LocalPublicId::new(&self.network).map_err(|_| invalid_binding())?;
        let binding =
            EvmNetworkBinding::new(network_id, self.chain_id).map_err(|_| invalid_binding())?;
        Ok(EvmSessionEvidence::new(
            &binding,
            LocalPublicId::new(&self.source_ref).map_err(|_| invalid_binding())?,
            LocalPublicId::new(&self.implementation_id).map_err(|_| invalid_binding())?,
        ))
    }

    /// Returns whether this evidence belongs to a semantic network binding.
    pub fn is_bound_to(&self, network: &str, chain_id: u64) -> bool {
        self.network == network && self.chain_id == chain_id
    }

    /// Returns whether checked session evidence matches exactly.
    pub fn matches_session(&self, evidence: &EvmSessionEvidence) -> bool {
        evidence.network_id().as_str() == self.network
            && evidence.chain_id() == self.chain_id
            && evidence.source_ref().as_str() == self.source_ref
            && evidence.implementation_id().as_str() == self.implementation_id
    }

    /// Returns the semantic network id.
    pub fn network(&self) -> &str {
        &self.network
    }

    /// Returns the chain id.
    pub const fn chain_id(&self) -> u64 {
        self.chain_id
    }

    /// Returns the process-local source reference.
    pub fn source_ref(&self) -> &str {
        &self.source_ref
    }

    /// Returns the certified session implementation id.
    pub fn implementation_id(&self) -> &str {
        &self.implementation_id
    }
}

impl<'de> Deserialize<'de> for RedactedEvmSessionEvidence {
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
            implementation_id: String,
        }

        let wire = Wire::deserialize(deserializer)?;
        Self::new(
            wire.network,
            wire.chain_id,
            wire.source_ref,
            wire.implementation_id,
        )
        .map_err(de::Error::custom)
    }
}

fn invalid_binding() -> EvmStateError {
    EvmStateError::InvalidInput {
        reason: "EVM redacted session evidence was invalid".to_owned(),
    }
}
