use mfm_canonical::{PlainCanonicalJsonBytes, RecoverabilityContractV3, ValidatedCanonicalValueV3};
use mfm_ids::{ContentRef, SchemaId, SemanticTypeId, StableId};
use serde::de::{self, Deserializer};
use serde::{Deserialize, Serialize};

use crate::{Result, ValueError};

const RETAINED_VALUE_CONTRACT_SCHEMA: &str = "mfm.retained-value-contract.v1";
const COMPONENT_OBJECT_EVIDENCE_CONTRACT_SCHEMA: &str = "mfm.component-object-evidence-contract.v1";
const COMPONENT_OBJECT_EVIDENCE_CONTRACT_BYTES: &[u8] =
    br#"{"version":"mfm.component-object-evidence-contract.v1"}"#;

/// Returns the exact canonical component-object-evidence contract.
///
/// This self-describing value is the common evidence contract for framework
/// retained values whose evidence shape is fixed by the product qualification
/// boundary.
pub fn component_object_evidence_contract_canonical() -> Result<PlainCanonicalJsonBytes> {
    PlainCanonicalJsonBytes::from_canonical_json_slice(COMPONENT_OBJECT_EVIDENCE_CONTRACT_BYTES)
        .map_err(|_| ValueError::RetainedValueContract)
}

/// Returns the annex-derived identity of the common component-object-evidence contract.
pub fn component_object_evidence_contract_ref() -> Result<ContentRef> {
    let canonical = component_object_evidence_contract_canonical()?;
    let recoverability = RecoverabilityContractV3::embedded()?;
    ContentRef::new(
        recoverability
            .schema_id(COMPONENT_OBJECT_EVIDENCE_CONTRACT_SCHEMA)?
            .clone(),
        recoverability.raw_content_digest(canonical.as_bytes()),
    )
    .map_err(|error| ValueError::Identity(error.to_string()))
}

/// Exact producer-independent metadata for one retained value.
///
/// This is the single semantic contract shared by certification, execution,
/// journaling, and replay. Producer authority and retained-byte identity remain
/// journal-owned and deliberately do not appear here.
///
/// ```
/// use mfm_values::RetainedValueContract;
///
/// fn retained_role(contract: &RetainedValueContract) -> &str {
///     contract.role().as_str()
/// }
/// ```
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct RetainedValueContract {
    schema_id: SchemaId,
    semantic_type_id: SemanticTypeId,
    role: StableId,
    media_type: String,
    evidence_contract_ref: ContentRef,
}

impl RetainedValueContract {
    /// Constructs and annex-validates one exact retained-value contract.
    pub fn new(
        schema_id: SchemaId,
        semantic_type_id: SemanticTypeId,
        role: StableId,
        media_type: impl Into<String>,
        evidence_contract_ref: ContentRef,
    ) -> Result<Self> {
        let contract = Self {
            schema_id,
            semantic_type_id,
            role,
            media_type: media_type.into(),
            evidence_contract_ref,
        };
        contract.validated()?;
        Ok(contract)
    }

    /// Strictly decodes exact canonical JSON under the frozen annex.
    pub fn strict_decode(bytes: &[u8]) -> Result<Self> {
        let validated = RecoverabilityContractV3::embedded()?
            .strict_decode(RETAINED_VALUE_CONTRACT_SCHEMA, bytes)?;
        Self::from_validated(validated)
    }

    /// Reconstructs this contract from exact annex-validated authority.
    pub fn from_validated(validated: ValidatedCanonicalValueV3) -> Result<Self> {
        if validated.schema_contract() != RETAINED_VALUE_CONTRACT_SCHEMA {
            return Err(ValueError::RetainedValueContract);
        }
        serde_json::from_slice(validated.as_bytes()).map_err(|_| ValueError::RetainedValueContract)
    }

    /// Returns the only admitted schema for the retained bytes.
    pub const fn schema_id(&self) -> &SchemaId {
        &self.schema_id
    }

    /// Returns the certified semantic type identity.
    pub const fn semantic_type_id(&self) -> &SemanticTypeId {
        &self.semantic_type_id
    }

    /// Returns the certified producer-independent retained role.
    pub const fn role(&self) -> &StableId {
        &self.role
    }

    /// Returns the exact lowercase registered media type.
    pub fn media_type(&self) -> &str {
        &self.media_type
    }

    /// Returns the exact object-evidence contract.
    pub const fn evidence_contract_ref(&self) -> &ContentRef {
        &self.evidence_contract_ref
    }

    /// Returns the exact canonical JSON validated by the frozen annex.
    pub fn canonical_json(&self) -> Result<PlainCanonicalJsonBytes> {
        let validated = self.validated()?;
        PlainCanonicalJsonBytes::from_canonical_json_slice(validated.as_bytes())
            .map_err(|_| ValueError::RetainedValueContract)
    }

    /// Returns exact annex-validated authority for canonical embedding.
    pub fn validated(&self) -> Result<ValidatedCanonicalValueV3> {
        let json = serde_json::to_string(self).map_err(|_| ValueError::RetainedValueContract)?;
        let canonical = PlainCanonicalJsonBytes::from_json_str(&json)
            .map_err(|_| ValueError::RetainedValueContract)?;
        RecoverabilityContractV3::embedded()?
            .strict_decode(RETAINED_VALUE_CONTRACT_SCHEMA, canonical.as_bytes())
            .map_err(Into::into)
    }
}

impl<'de> Deserialize<'de> for RetainedValueContract {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            schema_id: SchemaId,
            semantic_type_id: SemanticTypeId,
            role: StableId,
            media_type: String,
            evidence_contract_ref: ContentRef,
        }

        let wire = Wire::deserialize(deserializer)?;
        Self::new(
            wire.schema_id,
            wire.semantic_type_id,
            wire.role,
            wire.media_type,
            wire.evidence_contract_ref,
        )
        .map_err(de::Error::custom)
    }
}
