//! Small public transport contracts shared by entry-point discovery and admission.

use std::collections::BTreeSet;

use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_ids::{ContentRef, EntryPointId, SchemaId, StableId};
use mfm_program_derive::PersistedSchema;
use mfm_values::{CanonicalJsonPersistedSchema, CanonicalJsonProfile, SchemaShape};
use serde::{Deserialize, Serialize};

use crate::{Result, SpecError};

const PLANNING_PROFILE_CONTRACT: &str = "mfm.planning-profile.v1";
const PUBLISHED_ENTRY_POINT: &str = "mfm.published-entry-point.v1";

fn canonical<T: Serialize>(value: &T) -> Result<PlainCanonicalJsonBytes> {
    let json =
        serde_json::to_string(value).map_err(|error| SpecError::Contract(error.to_string()))?;
    PlainCanonicalJsonBytes::from_json_str(&json)
        .map_err(|error| SpecError::Contract(error.to_string()))
}

impl mfm_values::PersistedSchema for CanonicalJsonValue {
    fn schema_identity() -> mfm_values::Result<mfm_values::SchemaIdentity> {
        mfm_values::SchemaIdentity::new(
            mfm_values::SchemaKind::PersistedContract,
            None,
            "mfm.public-canonical-value",
            mfm_ids::SchemaVersion::new("1")
                .map_err(|error| mfm_values::ValueError::Identity(error.to_string()))?,
            open_canonical_value_shape(),
        )
    }

    fn validate(&self) -> mfm_values::Result<()> {
        let json = serde_json::to_string(&self.0)
            .map_err(|_| mfm_values::ValueError::SchemaShapeMismatch)?;
        let canonical = PlainCanonicalJsonBytes::from_json_str(&json)
            .map_err(|_| mfm_values::ValueError::SchemaShapeMismatch)?;
        <Self as mfm_values::PersistedSchema>::schema_identity()?
            .validate_canonical_value(canonical.as_bytes())
    }
}

/// The one bounded canonical-JSON terminal admitted at public boundaries.
fn open_canonical_value_shape() -> SchemaShape {
    SchemaShape::CanonicalJsonTerminal {
        profile: CanonicalJsonProfile::GeneralFloatFree,
    }
}

fn validate_open_canonical_value(bytes: &[u8]) -> Result<()> {
    <CanonicalJsonValue as mfm_values::PersistedSchema>::schema_identity()
        .and_then(|identity| identity.validate_canonical_value(bytes))
        .map_err(|error| SpecError::Contract(error.to_string()))
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
        validate_open_canonical_value(bytes.as_bytes())?;
        Ok(Self(value))
    }

    /// Returns the exact empty canonical object.
    pub fn empty_object() -> Self {
        Self(serde_json::Value::Object(serde_json::Map::new()))
    }

    /// Strictly decodes a canonical open value.
    pub fn from_canonical_json(bytes: &[u8]) -> Result<Self> {
        let canonical = PlainCanonicalJsonBytes::from_canonical_json_slice(bytes)
            .map_err(|error| SpecError::Contract(error.to_string()))?;
        validate_open_canonical_value(canonical.as_bytes())?;
        let checked = canonical;
        let value = serde_json::from_slice(checked.as_bytes())
            .map_err(|error| SpecError::Contract(error.to_string()))?;
        Ok(Self(value))
    }

    /// Strictly decodes exact bytes for an already-selected semantic owner.
    #[doc(hidden)]
    pub fn from_exact_bytes(bytes: &[u8]) -> Result<Self> {
        Self::from_canonical_json(bytes)
    }

    /// Returns canonical bytes for this value.
    pub fn canonical_json(&self) -> Result<PlainCanonicalJsonBytes> {
        canonical(&self.0)
    }

    /// Returns the checked JSON value.
    pub const fn as_json(&self) -> &serde_json::Value {
        &self.0
    }

    /// Constructs one closed single-key tagged semantic value.
    #[doc(hidden)]
    pub fn tagged(tag: impl Into<String>, value: Self) -> Result<Self> {
        let mut object = serde_json::Map::new();
        object.insert(tag.into(), value.0);
        Self::new(serde_json::Value::Object(object))
    }

    /// Constructs the canonical non-empty head/tail join value.
    #[doc(hidden)]
    pub fn head_tail(head: Self, tail: Vec<Self>) -> Result<Self> {
        let mut object = serde_json::Map::new();
        object.insert("head".to_owned(), head.0);
        object.insert(
            "tail".to_owned(),
            serde_json::Value::Array(tail.into_iter().map(|value| value.0).collect()),
        );
        Self::new(serde_json::Value::Object(object))
    }

    /// Selects and clones one nested object path.
    #[doc(hidden)]
    pub fn select_path<'a>(&self, path: impl IntoIterator<Item = &'a str>) -> Result<Self> {
        let mut selected = &self.0;
        for segment in path {
            selected = selected.get(segment).ok_or_else(|| {
                SpecError::Contract("canonical semantic path is absent".to_owned())
            })?;
        }
        Self::new(selected.clone())
    }

    /// Returns one string-valued object field.
    #[doc(hidden)]
    pub fn string_field(&self, field: &str) -> Option<&str> {
        self.0.get(field).and_then(serde_json::Value::as_str)
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, PersistedSchema)]
#[serde(deny_unknown_fields)]
#[mfm(schema = "mfm.planning-profile", version = "1")]
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
        self.encode_canonical()
            .map_err(|error| SpecError::Contract(error.to_string()))
    }

    /// Strictly decodes exact canonical profile bytes.
    pub fn from_canonical_json(bytes: &[u8]) -> Result<Self> {
        let value = Self::decode_canonical(bytes)
            .map_err(|error| SpecError::Contract(error.to_string()))?;
        value.validate_invariant()?;
        Ok(value)
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

/// Published structured-program entry point.
///
/// This is output metadata, not retained content: it has no retained-value
/// contract, no content reference, and no decoder. Discovery publishes it; the
/// distinct certified `mfm.structured-entry-point-contract` component is the
/// retained one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PublishedEntryPoint {
    version: String,
    entry_point_id: EntryPointId,
    entry_point_operation_id: StableId,
    planning_profile_ref: ContentRef,
    planning_profile: PlanningProfile,
    input_schema_id: SchemaId,
    public_output_schema_id: SchemaId,
}

impl PublishedEntryPoint {
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
            version: PUBLISHED_ENTRY_POINT.to_owned(),
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

    /// Returns exact canonical output bytes for this published entry point.
    pub fn canonical_json(&self) -> Result<PlainCanonicalJsonBytes> {
        self.validate_invariant()?;
        let json =
            serde_json::to_string(self).map_err(|error| SpecError::Contract(error.to_string()))?;
        PlainCanonicalJsonBytes::from_json_str(&json)
            .map_err(|error| SpecError::Contract(error.to_string()))
    }
}

impl ContractInvariant for PublishedEntryPoint {
    fn validate_invariant(&self) -> Result<()> {
        if self.version != PUBLISHED_ENTRY_POINT
            || self.planning_profile.content_ref()? != self.planning_profile_ref
        {
            return Err(SpecError::Invariant(
                "published entry point does not bind its exact profile".to_owned(),
            ));
        }
        Ok(())
    }
}
