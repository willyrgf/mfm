use super::*;
use std::num::NonZeroU64;

use mfm_program::MfmContext;

/// Shared observation identity for EVM lifecycle reads.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, MfmValue)]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "observation-policy",
    schema = "mfm.evm.contract.value.observation_policy"
)]
pub struct FinalityOrObservationPolicy {
    /// Stable policy identity.
    pub policy_id: ObservationPolicyId,
    /// Optional shared block anchor for read observations.
    pub block_anchor: Option<BlockSelector>,
}

impl<'de> Deserialize<'de> for FinalityOrObservationPolicy {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct RawFinalityOrObservationPolicy {
            policy_id: ObservationPolicyId,
            #[serde(default)]
            block_anchor: Option<BlockSelector>,
        }

        let raw = RawFinalityOrObservationPolicy::deserialize(deserializer)?;
        Ok(Self {
            policy_id: raw.policy_id,
            block_anchor: raw.block_anchor,
        })
    }
}

/// Certified EVM network context shared by contract lifecycle phases.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, MfmValue)]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "network-context",
    schema = "mfm.evm.contract.value.network_context"
)]
pub struct EvmNetworkContext {
    /// Stable semantic network identifier.
    pub network_id: EvmNetworkId,
    /// Expected EVM chain id observed by live or replayed capabilities.
    pub expected_chain_id: NonZeroU64,
    /// Optional non-secret chain fingerprint, such as genesis hash.
    pub chain_fingerprint: Option<ChainFingerprint>,
    /// Optional shared observation policy.
    pub finality_or_observation_policy: Option<FinalityOrObservationPolicy>,
}

impl EvmNetworkContext {
    /// Creates a certified EVM network context.
    pub fn new(network_id: EvmNetworkId, expected_chain_id: u64) -> Result<Self, String> {
        let expected_chain_id = NonZeroU64::new(expected_chain_id)
            .ok_or_else(|| "expected_chain_id must be non-zero".to_owned())?;
        Ok(Self {
            network_id,
            expected_chain_id,
            chain_fingerprint: None,
            finality_or_observation_policy: None,
        })
    }

    /// Returns the expected EVM chain id.
    pub const fn expected_chain_id(&self) -> u64 {
        self.expected_chain_id.get()
    }
}

impl<'de> Deserialize<'de> for EvmNetworkContext {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct RawEvmNetworkContext {
            network_id: EvmNetworkId,
            expected_chain_id: u64,
            #[serde(default)]
            chain_fingerprint: Option<ChainFingerprint>,
            #[serde(default)]
            finality_or_observation_policy: Option<FinalityOrObservationPolicy>,
        }

        let raw = RawEvmNetworkContext::deserialize(deserializer)?;
        let expected_chain_id = NonZeroU64::new(raw.expected_chain_id)
            .ok_or_else(|| de::Error::custom("expected_chain_id must be non-zero"))?;
        Ok(Self {
            network_id: raw.network_id,
            expected_chain_id,
            chain_fingerprint: raw.chain_fingerprint,
            finality_or_observation_policy: raw.finality_or_observation_policy,
        })
    }
}

/// Digest-oriented contract profile identity shared by lifecycle phases.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, MfmValue)]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "contract-profile",
    schema = "mfm.evm.contract.value.contract_profile"
)]
pub struct ContractProfile {
    /// Stable profile identifier.
    pub profile_id: ContractProfileId,
    /// Optional retained artifact digest shared by lifecycle phases.
    pub artifact_digest: Option<ContractProfileDigestRef>,
    /// Optional retained artifact evidence reference shared by executable lifecycle phases.
    pub artifact_ref: Option<LifecycleArtifactEvidenceRef>,
    /// Optional ABI or interface digest.
    pub interface_digest: Option<ContractProfileDigestRef>,
    /// Optional creation bytecode digest.
    pub creation_bytecode_digest: Option<ContractProfileDigestRef>,
    /// Optional expected deployed code hash.
    pub deployed_code_hash: Option<EvmCodeHash>,
    /// Optional selector/event compatibility policy digest.
    pub selector_event_policy_digest: Option<ContractProfileDigestRef>,
}

impl<'de> Deserialize<'de> for ContractProfile {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct RawContractProfile {
            profile_id: ContractProfileId,
            #[serde(default)]
            artifact_digest: Option<ContractProfileDigestRef>,
            #[serde(default)]
            artifact_ref: Option<LifecycleArtifactEvidenceRef>,
            #[serde(default)]
            interface_digest: Option<ContractProfileDigestRef>,
            #[serde(default)]
            creation_bytecode_digest: Option<ContractProfileDigestRef>,
            #[serde(default)]
            deployed_code_hash: Option<EvmCodeHash>,
            #[serde(default)]
            selector_event_policy_digest: Option<ContractProfileDigestRef>,
        }

        let raw = RawContractProfile::deserialize(deserializer)?;
        Ok(Self {
            profile_id: raw.profile_id,
            artifact_digest: raw.artifact_digest,
            artifact_ref: raw.artifact_ref,
            interface_digest: raw.interface_digest,
            creation_bytecode_digest: raw.creation_bytecode_digest,
            deployed_code_hash: raw.deployed_code_hash,
            selector_event_policy_digest: raw.selector_event_policy_digest,
        })
    }
}

/// Certified EVM contract lifecycle context.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, MfmValue)]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "contract-context",
    schema = "mfm.evm.contract.value.contract_context"
)]
pub struct EvmContractContext {
    /// Stable lifecycle key within the authored workflow.
    pub lifecycle_key: LifecycleKey,
    /// Semantic network context.
    pub network: EvmNetworkContext,
    /// Required contract profile identity.
    pub contract_profile: ContractProfile,
}

impl<'de> Deserialize<'de> for EvmContractContext {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct RawEvmContractContext {
            lifecycle_key: LifecycleKey,
            network: EvmNetworkContext,
            contract_profile: ContractProfile,
        }

        let raw = RawEvmContractContext::deserialize(deserializer)?;
        Ok(Self {
            lifecycle_key: raw.lifecycle_key,
            network: raw.network,
            contract_profile: raw.contract_profile,
        })
    }
}

impl MfmContext for EvmContractContext {}
