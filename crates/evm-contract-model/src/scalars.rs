use super::*;
use mfm_evm_core::tx::parse_u128_quantity;
use mfm_ids::{LocalPublicId, StableAuthorKey};
use std::fmt;
use std::ops::Deref;
use std::str::FromStr;

/// Error returned when constructing EVM contract scalar authorities.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum EvmContractScalarError {
    /// A string scalar was empty or whitespace-only.
    #[error("{kind} must be non-empty")]
    Empty {
        /// Human-readable scalar kind.
        kind: &'static str,
    },
    /// A wei quantity failed canonical quantity parsing.
    #[error("{kind} must be a valid EVM quantity: {message}")]
    InvalidQuantity {
        /// Human-readable scalar kind.
        kind: &'static str,
        /// Parser diagnostic.
        message: String,
    },
    /// A typed identity failed category-specific parsing.
    #[error("{kind} must be a valid typed identity: {message}")]
    InvalidIdentity {
        /// Human-readable scalar kind.
        kind: &'static str,
        /// Parser diagnostic.
        message: String,
    },
    /// A checked string failed its shared grammar.
    #[error("{kind} must be a valid checked string: {message}")]
    InvalidString {
        /// Human-readable scalar kind.
        kind: &'static str,
        /// Parser diagnostic.
        message: String,
    },
}

fn require_non_empty(kind: &'static str, value: &str) -> Result<(), EvmContractScalarError> {
    if value.trim().is_empty() {
        Err(EvmContractScalarError::Empty { kind })
    } else {
        Ok(())
    }
}

macro_rules! evm_string_scalar {
    ($ty:ident, $validator:ident, $kind:literal, $semantic:literal, $schema:literal, $doc:literal) => {
        #[doc = $doc]
        #[derive(
            Clone,
            Debug,
            PartialEq,
            Eq,
            PartialOrd,
            Ord,
            Hash,
            Serialize,
            Deserialize,
            MfmValue,
        )]
        #[serde(try_from = "String", into = "String")]
        #[mfm(namespace = "mfm.evm.contract", name = $semantic, schema = $schema, transparent_string)]
        pub struct $ty {
            raw: String,
        }

        impl $ty {
            /// Creates a checked EVM contract scalar authority.
            pub fn new(value: impl Into<String>) -> Result<Self, EvmContractScalarError> {
                let raw = value.into();
                $validator($kind, &raw)?;
                Ok(Self { raw })
            }

            /// Returns the canonical string representation.
            pub fn as_str(&self) -> &str {
                &self.raw
            }

            /// Consumes this authority into its canonical string representation.
            pub fn into_string(self) -> String {
                self.raw
            }
        }

        impl AsRef<str> for $ty {
            fn as_ref(&self) -> &str {
                self.as_str()
            }
        }

        impl Deref for $ty {
            type Target = str;

            fn deref(&self) -> &Self::Target {
                self.as_str()
            }
        }

        impl fmt::Display for $ty {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl FromStr for $ty {
            type Err = EvmContractScalarError;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Self::new(value)
            }
        }

        impl TryFrom<String> for $ty {
            type Error = EvmContractScalarError;

            fn try_from(value: String) -> Result<Self, Self::Error> {
                Self::new(value)
            }
        }

        impl From<$ty> for String {
            fn from(value: $ty) -> Self {
                value.raw
            }
        }

        impl PartialEq<&str> for $ty {
            fn eq(&self, other: &&str) -> bool {
                self.as_str() == *other
            }
        }
    };
}

fn require_stable_author_key(
    kind: &'static str,
    value: &str,
) -> Result<(), EvmContractScalarError> {
    StableAuthorKey::new(value)
        .map(|_| ())
        .map_err(|error| EvmContractScalarError::InvalidString {
            kind,
            message: error.to_string(),
        })
}

fn require_local_public_id(kind: &'static str, value: &str) -> Result<(), EvmContractScalarError> {
    LocalPublicId::new(value)
        .map(|_| ())
        .map_err(|error| EvmContractScalarError::InvalidString {
            kind,
            message: error.to_string(),
        })
}

evm_string_scalar!(
    EvmNetworkId,
    require_stable_author_key,
    "network_id",
    "network-id",
    "mfm.evm.contract.id.network",
    "Stable typed EVM network identifier."
);

evm_string_scalar!(
    FunctionName,
    require_non_empty,
    "function",
    "function-name",
    "mfm.evm.contract.id.function",
    "Checked EVM ABI function name."
);

evm_string_scalar!(
    EventName,
    require_non_empty,
    "event",
    "event-name",
    "mfm.evm.contract.id.event",
    "Checked EVM ABI event name."
);

evm_string_scalar!(
    ArtifactPort,
    require_local_public_id,
    "artifact_port",
    "artifact-port",
    "mfm.evm.contract.id.artifact_port",
    "Checked artifact context port name."
);

evm_string_scalar!(
    LifecycleKey,
    require_stable_author_key,
    "lifecycle_key",
    "lifecycle-key",
    "mfm.evm.contract.id.lifecycle_key",
    "Stable contract-state context key."
);

evm_string_scalar!(
    ContractProfileId,
    require_stable_author_key,
    "contract_profile_id",
    "contract-profile-id",
    "mfm.evm.contract.id.contract_profile",
    "Stable contract profile identifier."
);

evm_string_scalar!(
    ChainFingerprint,
    require_non_empty,
    "chain_fingerprint",
    "chain-fingerprint",
    "mfm.evm.contract.value.chain_fingerprint",
    "Non-secret chain fingerprint value used by certified EVM contexts."
);

evm_string_scalar!(
    ObservationPolicyId,
    require_local_public_id,
    "observation_policy_id",
    "observation-policy-id",
    "mfm.evm.contract.id.observation_policy",
    "Checked observation policy identity used by certified EVM contexts."
);

macro_rules! evidence_identity_scalar {
    (
        $ty:ident,
        $target:ty,
        $kind:literal,
        $semantic:literal,
        $schema:literal,
        $doc:literal
    ) => {
        #[doc = $doc]
        #[derive(
            Clone,
            Debug,
            PartialEq,
            Eq,
            PartialOrd,
            Ord,
            Hash,
            Serialize,
            Deserialize,
            MfmValue,
        )]
        #[serde(try_from = "String", into = "String")]
        #[mfm(namespace = "mfm.evm.contract", name = $semantic, schema = $schema, transparent_string)]
        pub struct $ty {
            raw: String,
        }

        impl $ty {
            /// Creates a checked evidence identity reference.
            pub fn new(value: impl Into<String>) -> Result<Self, EvmContractScalarError> {
                let raw = value.into();
                require_non_empty($kind, &raw)?;
                <$target>::parse(&raw).map_err(|error| EvmContractScalarError::InvalidIdentity {
                    kind: $kind,
                    message: error.to_string(),
                })?;
                Ok(Self { raw })
            }

            /// Returns the canonical string representation.
            pub fn as_str(&self) -> &str {
                &self.raw
            }

            /// Parses this checked reference into the corresponding kernel identity.
            pub fn typed(&self) -> Result<$target, String> {
                <$target>::parse(&self.raw).map_err(|error| error.to_string())
            }

            /// Consumes this authority into its canonical string representation.
            pub fn into_string(self) -> String {
                self.raw
            }
        }

        impl AsRef<str> for $ty {
            fn as_ref(&self) -> &str {
                self.as_str()
            }
        }

        impl Deref for $ty {
            type Target = str;

            fn deref(&self) -> &Self::Target {
                self.as_str()
            }
        }

        impl fmt::Display for $ty {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl FromStr for $ty {
            type Err = EvmContractScalarError;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Self::new(value)
            }
        }

        impl TryFrom<String> for $ty {
            type Error = EvmContractScalarError;

            fn try_from(value: String) -> Result<Self, Self::Error> {
                Self::new(value)
            }
        }

        impl From<$target> for $ty {
            fn from(value: $target) -> Self {
                Self {
                    raw: value.to_string(),
                }
            }
        }

        impl From<$ty> for String {
            fn from(value: $ty) -> Self {
                value.raw
            }
        }

        impl PartialEq<&str> for $ty {
            fn eq(&self, other: &&str) -> bool {
                self.as_str() == *other
            }
        }
    };
}

evidence_identity_scalar!(
    ArtifactEvidenceArtifactId,
    ArtifactId,
    "artifact_id",
    "artifact-evidence-artifact-id",
    "mfm.evm.contract.id.artifact_evidence_artifact",
    "Checked artifact id reference carried by lifecycle evidence."
);

evidence_identity_scalar!(
    ArtifactEvidenceContentDigest,
    ContentDigest,
    "content_digest",
    "artifact-evidence-content-digest",
    "mfm.evm.contract.id.artifact_evidence_content_digest",
    "Checked content digest reference carried by lifecycle evidence."
);

evidence_identity_scalar!(
    ArtifactEvidenceSchemaId,
    SchemaId,
    "schema_id",
    "artifact-evidence-schema-id",
    "mfm.evm.contract.id.artifact_evidence_schema",
    "Checked schema id reference carried by lifecycle evidence."
);

evidence_identity_scalar!(
    ArtifactEvidenceSemanticTypeId,
    SemanticTypeId,
    "semantic_type_id",
    "artifact-evidence-semantic-type-id",
    "mfm.evm.contract.id.artifact_evidence_semantic_type",
    "Checked semantic type id reference carried by lifecycle evidence."
);

evidence_identity_scalar!(
    ArtifactEvidenceEvidenceHash,
    ContentDigest,
    "evidence_hash",
    "artifact-evidence-evidence-hash",
    "mfm.evm.contract.id.artifact_evidence_evidence_hash",
    "Checked exact retained-artifact evidence hash carried by lifecycle evidence."
);

evidence_identity_scalar!(
    ContractProfileDigestRef,
    ContentDigest,
    "contract_profile_digest",
    "contract-profile-digest-ref",
    "mfm.evm.contract.id.contract_profile_digest",
    "Checked content digest reference carried by a contract profile."
);

/// Checked wei quantity rendered in the authored EVM quantity format.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, MfmValue)]
#[serde(try_from = "String", into = "String")]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "wei-amount",
    schema = "mfm.evm.contract.value.wei_amount",
    transparent_string
)]
pub struct WeiAmount {
    raw: String,
}

impl WeiAmount {
    /// Creates a checked wei amount.
    pub fn new(value: impl Into<String>) -> Result<Self, EvmContractScalarError> {
        let raw = value.into();
        parse_u128_quantity(&raw, "wei").map_err(|error| {
            EvmContractScalarError::InvalidQuantity {
                kind: "wei",
                message: error.message,
            }
        })?;
        Ok(Self { raw })
    }

    /// Returns the canonical string representation.
    pub fn as_str(&self) -> &str {
        &self.raw
    }

    /// Consumes this authority into its canonical string representation.
    pub fn into_string(self) -> String {
        self.raw
    }
}

impl AsRef<str> for WeiAmount {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl Deref for WeiAmount {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.as_str()
    }
}

impl fmt::Display for WeiAmount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for WeiAmount {
    type Err = EvmContractScalarError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::new(value)
    }
}

impl TryFrom<String> for WeiAmount {
    type Error = EvmContractScalarError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<WeiAmount> for String {
    fn from(value: WeiAmount) -> Self {
        value.raw
    }
}

/// Normalized lowercase `0x`-prefixed EVM contract address.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, MfmValue)]
#[serde(try_from = "String", into = "String")]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "contract-address",
    schema = "mfm.evm.contract.value.contract_address",
    transparent_string
)]
pub struct ContractAddress {
    raw: String,
}

impl ContractAddress {
    /// Creates a normalized EVM contract address.
    pub fn new(value: impl Into<String>) -> Result<Self, EvmContractScalarError> {
        let raw = value.into();
        let normalized = encoding::normalize_address(&raw).map_err(|error| {
            EvmContractScalarError::InvalidString {
                kind: "contract_address",
                message: error.message,
            }
        })?;
        Ok(Self { raw: normalized })
    }

    /// Returns the canonical lowercase address.
    pub fn as_str(&self) -> &str {
        &self.raw
    }

    /// Consumes this authority into its canonical string representation.
    pub fn into_string(self) -> String {
        self.raw
    }
}

impl AsRef<str> for ContractAddress {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl Deref for ContractAddress {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.as_str()
    }
}

impl fmt::Display for ContractAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ContractAddress {
    type Err = EvmContractScalarError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::new(value)
    }
}

impl TryFrom<String> for ContractAddress {
    type Error = EvmContractScalarError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<ContractAddress> for String {
    fn from(value: ContractAddress) -> Self {
        value.raw
    }
}

/// Normalized lowercase `0x`-prefixed 32-byte EVM code hash.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, MfmValue)]
#[serde(try_from = "String", into = "String")]
#[mfm(
    namespace = "mfm.evm.contract",
    name = "evm-code-hash",
    schema = "mfm.evm.contract.value.evm_code_hash",
    transparent_string
)]
pub struct EvmCodeHash {
    raw: String,
}

impl EvmCodeHash {
    /// Creates a normalized 32-byte EVM code hash.
    pub fn new(value: impl Into<String>) -> Result<Self, EvmContractScalarError> {
        let raw = value.into();
        let normalized = common_hex::normalize_hex_str(&raw).map_err(|error| {
            EvmContractScalarError::InvalidString {
                kind: "evm_code_hash",
                message: error.message,
            }
        })?;
        let bytes = common_hex::hex_to_bytes(&normalized).map_err(|error| {
            EvmContractScalarError::InvalidString {
                kind: "evm_code_hash",
                message: error.message,
            }
        })?;
        if bytes.len() != 32 {
            return Err(EvmContractScalarError::InvalidString {
                kind: "evm_code_hash",
                message: "code hash must be 32 bytes".to_owned(),
            });
        }
        Ok(Self { raw: normalized })
    }

    /// Returns the canonical lowercase code hash.
    pub fn as_str(&self) -> &str {
        &self.raw
    }

    /// Consumes this authority into its canonical string representation.
    pub fn into_string(self) -> String {
        self.raw
    }
}

impl AsRef<str> for EvmCodeHash {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl Deref for EvmCodeHash {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.as_str()
    }
}

impl fmt::Display for EvmCodeHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for EvmCodeHash {
    type Err = EvmContractScalarError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::new(value)
    }
}

impl TryFrom<String> for EvmCodeHash {
    type Error = EvmContractScalarError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<EvmCodeHash> for String {
    fn from(value: EvmCodeHash) -> Self {
        value.raw
    }
}
