//! Small public transport contracts shared by entry-point discovery and admission.

use std::collections::BTreeSet;

use mfm_canonical::{
    sha256_digest_bytes, PlainCanonicalJsonBytes, RecoverabilityContract, ValidatedCanonicalValue,
};
use mfm_ids::{ContentRef, DigestAlgorithm, EntryPointId, SchemaId, SemanticTypeId, StableId};
use mfm_values::{component_object_evidence_contract_ref, RetainedValueContract};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::{Result, SpecError};

const PLANNING_PROFILE_CONTRACT: &str = "mfm.planning-profile.v1";
const ENTRY_POINT_CONTRACT: &str = "mfm.entry-point-contract.v1";

fn contract() -> Result<&'static RecoverabilityContract> {
    RecoverabilityContract::embedded().map_err(Into::into)
}

fn canonical<T: Serialize>(value: &T) -> Result<PlainCanonicalJsonBytes> {
    let json =
        serde_json::to_string(value).map_err(|error| SpecError::Contract(error.to_string()))?;
    PlainCanonicalJsonBytes::from_json_str(&json)
        .map_err(|error| SpecError::Contract(error.to_string()))
}

fn validated<T: Serialize>(schema_contract: &str, value: &T) -> Result<ValidatedCanonicalValue> {
    let canonical = canonical(value)?;
    contract()?
        .strict_decode(schema_contract, canonical.as_bytes())
        .map_err(Into::into)
}

fn decode<T: DeserializeOwned + ContractInvariant>(
    schema_contract: &str,
    bytes: &[u8],
) -> Result<T> {
    let checked = contract()?.strict_decode(schema_contract, bytes)?;
    let value = serde_json::from_slice::<T>(checked.as_bytes())
        .map_err(|error| SpecError::Contract(format!("typed canonical decode failed: {error}")))?;
    value.validate_invariant()?;
    Ok(value)
}

trait ContractInvariant {
    fn validate_invariant(&self) -> Result<()>;
}

/// A checked, float-free canonical JSON value used at public transport boundaries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalJsonValue(serde_json::Value);

impl CanonicalJsonValue {
    /// Validates one JSON value against the current open-value contract.
    pub fn new(value: serde_json::Value) -> Result<Self> {
        let bytes = canonical(&value)?;
        contract()?.strict_decode("mfm.primitive-canonical_value.v1", bytes.as_bytes())?;
        Ok(Self(value))
    }

    /// Returns the exact empty canonical object.
    pub fn empty_object() -> Self {
        Self(serde_json::Value::Object(serde_json::Map::new()))
    }

    /// Strictly decodes a canonical open value.
    pub fn from_canonical_json(bytes: &[u8]) -> Result<Self> {
        let checked = contract()?.strict_decode("mfm.primitive-canonical_value.v1", bytes)?;
        let value = serde_json::from_slice(checked.as_bytes())
            .map_err(|error| SpecError::Contract(error.to_string()))?;
        Ok(Self(value))
    }

    /// Returns canonical bytes for this value.
    pub fn canonical_json(&self) -> Result<PlainCanonicalJsonBytes> {
        canonical(&self.0)
    }

    /// Returns the checked JSON value.
    pub const fn as_json(&self) -> &serde_json::Value {
        &self.0
    }
}

impl Serialize for CanonicalJsonValue {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.0.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for CanonicalJsonValue {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = serde_json::Value::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

/// Exact structured-expansion profile selected by a published entry point.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanningProfile {
    version: String,
    canonical_profile_parameters: CanonicalJsonValue,
    framework_policy_refs: Vec<ContentRef>,
    planner_contract_ref: ContentRef,
    planner_implementation_ref: ContentRef,
}

impl PlanningProfile {
    /// Constructs an exact profile and validates its closed invariants.
    pub fn new(
        planner_contract_ref: ContentRef,
        planner_implementation_ref: ContentRef,
        framework_policy_refs: Vec<ContentRef>,
        canonical_profile_parameters: CanonicalJsonValue,
    ) -> Result<Self> {
        let profile = Self {
            version: PLANNING_PROFILE_CONTRACT.to_owned(),
            canonical_profile_parameters,
            framework_policy_refs,
            planner_contract_ref,
            planner_implementation_ref,
        };
        profile.validate_invariant()?;
        Ok(profile)
    }

    /// Returns canonical expansion-profile parameters.
    pub const fn canonical_profile_parameters(&self) -> &CanonicalJsonValue {
        &self.canonical_profile_parameters
    }

    /// Returns ordered framework-policy references.
    pub fn framework_policy_refs(&self) -> &[ContentRef] {
        &self.framework_policy_refs
    }

    /// Returns the semantic expansion contract.
    pub const fn planner_contract_ref(&self) -> &ContentRef {
        &self.planner_contract_ref
    }

    /// Returns the qualified expansion implementation.
    pub const fn planner_implementation_ref(&self) -> &ContentRef {
        &self.planner_implementation_ref
    }

    /// Returns exact canonical bytes validated by the public contract.
    pub fn canonical_json(&self) -> Result<PlainCanonicalJsonBytes> {
        self.validate_invariant()?;
        let value = validated(PLANNING_PROFILE_CONTRACT, self)?;
        PlainCanonicalJsonBytes::from_canonical_json_slice(value.as_bytes())
            .map_err(|error| SpecError::Contract(error.to_string()))
    }

    /// Returns the raw-byte content identity of this profile.
    pub fn content_ref(&self) -> Result<ContentRef> {
        let value = validated(PLANNING_PROFILE_CONTRACT, self)?;
        contract()?.content_ref(&value).map_err(Into::into)
    }

    /// Strictly decodes exact canonical profile bytes.
    pub fn from_canonical_json(bytes: &[u8]) -> Result<Self> {
        decode(PLANNING_PROFILE_CONTRACT, bytes)
    }
}

impl ContractInvariant for PlanningProfile {
    fn validate_invariant(&self) -> Result<()> {
        if self.version != PLANNING_PROFILE_CONTRACT {
            return Err(SpecError::Invariant(
                "planning profile version mismatch".to_owned(),
            ));
        }
        if self
            .framework_policy_refs
            .iter()
            .collect::<BTreeSet<_>>()
            .len()
            != self.framework_policy_refs.len()
        {
            return Err(SpecError::Invariant(
                "framework policy references must be unique".to_owned(),
            ));
        }
        Ok(())
    }
}

/// Published structured-program entry-point contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EntryPointContract {
    version: String,
    entry_point_id: EntryPointId,
    entry_point_operation_id: StableId,
    planning_profile_ref: ContentRef,
    planning_profile: PlanningProfile,
    input_schema_id: SchemaId,
    public_output_schema_id: SchemaId,
}

impl EntryPointContract {
    /// Returns the retained-value contract used for entry-point discovery data.
    pub fn retained_contract() -> Result<RetainedValueContract> {
        let semantic_type_id = SemanticTypeId::new(
            "mfm.recoverability",
            "entry-point-contract",
            "1",
            DigestAlgorithm::Sha256JcsV1,
            sha256_digest_bytes(b"semantic:mfm.recoverability:entry-point-contract:1"),
        )?;
        RetainedValueContract::new(
            schema_id(ENTRY_POINT_CONTRACT)?,
            semantic_type_id,
            StableId::new("mfm.admission.entry-point-contract")
                .map_err(|error| SpecError::Identity(error.to_string()))?,
            "application/json",
            component_object_evidence_contract_ref()?,
        )
        .map_err(Into::into)
    }

    /// Constructs a complete published entry point.
    pub fn new(
        entry_point_id: EntryPointId,
        entry_point_operation_id: StableId,
        planning_profile: PlanningProfile,
        input_schema_id: SchemaId,
        public_output_schema_id: SchemaId,
    ) -> Result<Self> {
        let planning_profile_ref = planning_profile.content_ref()?;
        let entry = Self {
            version: ENTRY_POINT_CONTRACT.to_owned(),
            entry_point_id,
            entry_point_operation_id,
            planning_profile_ref,
            planning_profile,
            input_schema_id,
            public_output_schema_id,
        };
        entry.validate_invariant()?;
        Ok(entry)
    }

    /// Returns the versioned discovery identifier.
    pub const fn entry_point_id(&self) -> &EntryPointId {
        &self.entry_point_id
    }

    /// Returns the stable operation identifier used for admission.
    pub const fn entry_point_operation_id(&self) -> &StableId {
        &self.entry_point_operation_id
    }

    /// Returns the exact expansion profile.
    pub const fn planning_profile(&self) -> &PlanningProfile {
        &self.planning_profile
    }

    /// Returns the exact profile identity.
    pub const fn planning_profile_ref(&self) -> &ContentRef {
        &self.planning_profile_ref
    }

    /// Returns the admitted input schema.
    pub const fn input_schema_id(&self) -> &SchemaId {
        &self.input_schema_id
    }

    /// Returns the reviewed public-output schema.
    pub const fn public_output_schema_id(&self) -> &SchemaId {
        &self.public_output_schema_id
    }

    /// Returns exact canonical bytes validated by the public contract.
    pub fn canonical_json(&self) -> Result<PlainCanonicalJsonBytes> {
        self.validate_invariant()?;
        let value = validated(ENTRY_POINT_CONTRACT, self)?;
        PlainCanonicalJsonBytes::from_canonical_json_slice(value.as_bytes())
            .map_err(|error| SpecError::Contract(error.to_string()))
    }

    /// Returns the raw-byte content identity of this entry point.
    pub fn content_ref(&self) -> Result<ContentRef> {
        let value = validated(ENTRY_POINT_CONTRACT, self)?;
        contract()?.content_ref(&value).map_err(Into::into)
    }

    /// Strictly decodes exact canonical entry-point bytes.
    pub fn from_canonical_json(bytes: &[u8]) -> Result<Self> {
        decode(ENTRY_POINT_CONTRACT, bytes)
    }
}

impl ContractInvariant for EntryPointContract {
    fn validate_invariant(&self) -> Result<()> {
        if self.version != ENTRY_POINT_CONTRACT
            || self.planning_profile.content_ref()? != self.planning_profile_ref
        {
            return Err(SpecError::Invariant(
                "entry point does not bind its exact profile".to_owned(),
            ));
        }
        Ok(())
    }
}

/// Returns the current schema identity for a named public transport contract.
pub fn schema_id(schema_contract: &str) -> Result<SchemaId> {
    contract()?
        .schema_id(schema_contract)
        .cloned()
        .map_err(Into::into)
}
