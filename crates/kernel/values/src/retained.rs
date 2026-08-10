use mfm_ids::{ContentRef, SchemaId, SemanticTypeId, StableId};
use serde::{Deserialize, Serialize};

use crate::{
    CanonicalJsonPersistedSchema, FieldDescriptor, MediaType, PersistedSchema, Result,
    SchemaIdentity, SchemaKind, SchemaShape, StringGrammar, ValueError, MAX_MEDIA_TYPE_BYTES,
};

const COMPONENT_OBJECT_EVIDENCE_CONTRACT_SCHEMA: &str = "mfm.component-object-evidence-contract.v1";

/// One retained component-object-evidence contract.
///
/// The document is one exact literal, so its persisted shape is a closed literal
/// rather than an open string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComponentObjectEvidence {
    version: ComponentObjectEvidenceVersion,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum ComponentObjectEvidenceVersion {
    #[serde(rename = "mfm.component-object-evidence-contract.v1")]
    V1,
}

impl PersistedSchema for ComponentObjectEvidence {
    fn schema_identity() -> Result<SchemaIdentity> {
        SchemaIdentity::new(
            SchemaKind::PersistedContract,
            None,
            "mfm.component-object-evidence-contract",
            mfm_ids::SchemaVersion::new("1")
                .map_err(|error| ValueError::Identity(error.to_string()))?,
            SchemaShape::named_struct(vec![FieldDescriptor::required(
                "version",
                SchemaShape::Literal(crate::LiteralValue::String(
                    COMPONENT_OBJECT_EVIDENCE_CONTRACT_SCHEMA.to_owned(),
                )),
            )])?,
        )
    }

    fn validate(&self) -> Result<()> {
        match self.version {
            ComponentObjectEvidenceVersion::V1 => Ok(()),
        }
    }
}

impl ComponentObjectEvidence {
    /// Returns the one current literal evidence payload.
    pub const fn current() -> Self {
        Self {
            version: ComponentObjectEvidenceVersion::V1,
        }
    }
}

impl crate::PersistedObjectPayload for ComponentObjectEvidence {
    fn object_type() -> Result<StableId> {
        StableId::new("structured.data_contract")
            .map_err(|error| ValueError::Identity(error.to_string()))
    }
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
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetainedValueContract {
    schema_id: SchemaId,
    semantic_type_id: SemanticTypeId,
    role: StableId,
    media_type: MediaType,
    evidence_contract_ref: ContentRef,
}

impl PersistedSchema for RetainedValueContract {
    fn schema_identity() -> Result<SchemaIdentity> {
        SchemaIdentity::new(
            SchemaKind::PersistedContract,
            None,
            "mfm.retained-value-contract",
            mfm_ids::SchemaVersion::new("1")
                .map_err(|error| ValueError::Identity(error.to_string()))?,
            SchemaShape::named_struct(vec![
                FieldDescriptor::required("evidence_contract_ref", SchemaShape::content_ref()?),
                FieldDescriptor::required(
                    "media_type",
                    SchemaShape::BoundedString {
                        minimum_bytes: 1,
                        maximum_bytes: MAX_MEDIA_TYPE_BYTES as u32,
                        grammar: StringGrammar::MediaType,
                    },
                ),
                FieldDescriptor::required(
                    "role",
                    SchemaShape::identity_string(StringGrammar::StableId, 256),
                ),
                FieldDescriptor::required(
                    "schema_id",
                    SchemaShape::identity_string(StringGrammar::SchemaId, 512),
                ),
                FieldDescriptor::required(
                    "semantic_type_id",
                    SchemaShape::identity_string(StringGrammar::SemanticTypeId, 512),
                ),
            ])?,
        )
    }

    fn validate(&self) -> Result<()> {
        MediaType::new(self.media_type.as_str()).map(drop)
    }
}

impl crate::PersistedObjectPayload for RetainedValueContract {
    fn object_type() -> Result<StableId> {
        StableId::new("structured.data_contract")
            .map_err(|error| ValueError::Identity(error.to_string()))
    }
}

impl RetainedValueContract {
    /// Constructs and validates one exact retained-value contract.
    pub fn new(
        schema_id: SchemaId,
        semantic_type_id: SemanticTypeId,
        role: StableId,
        media_type: MediaType,
        evidence_contract_ref: ContentRef,
    ) -> Result<Self> {
        let contract = Self {
            schema_id,
            semantic_type_id,
            role,
            media_type,
            evidence_contract_ref,
        };
        contract
            .encode_canonical()
            .map_err(|_| ValueError::RetainedValueContract)?;
        Ok(contract)
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
    pub const fn media_type(&self) -> &MediaType {
        &self.media_type
    }

    /// Returns the exact object-evidence contract.
    pub const fn evidence_contract_ref(&self) -> &ContentRef {
        &self.evidence_contract_ref
    }
}
