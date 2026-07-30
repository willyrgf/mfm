//! Frozen recoverability-v1 planning and graph values.

use std::collections::{BTreeMap, BTreeSet};

use mfm_canonical::{
    sha256_digest_bytes, PlainCanonicalJsonBytes, RecoverabilityContract, ValidatedCanonicalValue,
};
pub use mfm_ids::EntryPointId;
use mfm_ids::{
    ContentRef, DigestAlgorithm, FieldPath, NodeId, SchemaId, SemanticTypeId, SpecHash, StableId,
};
use mfm_journal::{FactClaimEnvelope, FrozenReadIntent, InputManifest, ValueRef};
use mfm_values::component_object_evidence_contract_ref;
pub use mfm_values::RetainedValueContract;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::{Result, SpecError};

#[path = "certified_node.rs"]
mod certified_node;
pub use certified_node::*;

const AUTHORED_PROGRAM_CONTRACT: &str = "mfm.canonical-authored-program.v1";
const PLANNING_PROFILE_CONTRACT: &str = "mfm.planning-profile.v1";
const ENTRY_POINT_CONTRACT: &str = "mfm.entry-point-contract.v1";
const EXPANDED_SPEC_CONTRACT: &str = "mfm.expanded-certified-spec.v1";
const COMPONENT_IMPLEMENTATION_CONTRACT: &str = "mfm.component-implementation-descriptor.v1";
const STATE_IMPLEMENTATION_MANIFEST_CONTRACT: &str = "mfm.state-implementation-manifest.v1";
const CAPABILITY_BINDING_MANIFEST_CONTRACT: &str = "mfm.capability-binding-manifest.v1";
const CERTIFICATE_CONTRACT: &str = "mfm.certificate.v1";

fn admission_retained_contract(
    schema_contract: &str,
    semantic_name: &str,
    role: &str,
) -> Result<RetainedValueContract> {
    let semantic_type_id = SemanticTypeId::new(
        "mfm.recoverability",
        semantic_name,
        "1",
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(format!("semantic:mfm.recoverability:{semantic_name}:1").as_bytes()),
    )?;
    Ok(RetainedValueContract::new(
        schema_id(schema_contract)?,
        semantic_type_id,
        StableId::new(role).map_err(|error| SpecError::Identity(error.to_string()))?,
        "application/json",
        component_object_evidence_contract_ref()?,
    )?)
}

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

macro_rules! impl_codec {
    ($ty:ty, $contract:expr) => {
        impl $ty {
            /// Returns exact canonical bytes validated by the frozen annex.
            pub fn canonical_json(&self) -> Result<PlainCanonicalJsonBytes> {
                self.validate_invariant()?;
                let value = validated($contract, self)?;
                PlainCanonicalJsonBytes::from_canonical_json_slice(value.as_bytes())
                    .map_err(|error| SpecError::Contract(error.to_string()))
            }

            /// Returns the raw-byte content reference for the canonical value.
            pub fn content_ref(&self) -> Result<ContentRef> {
                let value = validated($contract, self)?;
                contract()?.content_ref(&value).map_err(Into::into)
            }

            /// Strictly decodes exact canonical bytes under the frozen annex.
            pub fn from_canonical_json(bytes: &[u8]) -> Result<Self> {
                decode($contract, bytes)
            }
        }
    };
}

/// A checked, float-free canonical JSON value used in open typed binding slots.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalJsonValue(serde_json::Value);

impl CanonicalJsonValue {
    /// Validates one JSON value against `mfm.primitive-canonical_value.v1`.
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
    pub fn as_json(&self) -> &serde_json::Value {
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

/// Authored occurrence kind before framework/executor expansion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthoredBaseKind {
    /// Domain-authored state occurrence.
    Authored,
    /// Ordinary typed composition bridge.
    Bridge,
}

/// One authored state occurrence before framework/executor expansion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthoredNode {
    stable_key: StableId,
    local_stable_key: StableId,
    child_path: Vec<StableId>,
    base_kind: AuthoredBaseKind,
    state_contract_ref: ContentRef,
    #[serde(with = "value_ref_serde")]
    config_ref: ValueRef,
    #[serde(with = "optional_value_ref_serde")]
    context_ref: Option<ValueRef>,
}

impl AuthoredNode {
    /// Constructs one fully bound authored occurrence.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        stable_key: StableId,
        local_stable_key: StableId,
        child_path: Vec<StableId>,
        base_kind: AuthoredBaseKind,
        state_contract_ref: ContentRef,
        config_ref: ValueRef,
        context_ref: Option<ValueRef>,
    ) -> Result<Self> {
        let expected_key = scoped_authored_key(&child_path, &local_stable_key)?;
        if stable_key != expected_key {
            return Err(SpecError::Invariant(
                "authored stable key differs from child path and local key".to_owned(),
            ));
        }
        Ok(Self {
            stable_key,
            local_stable_key,
            child_path,
            base_kind,
            state_contract_ref,
            config_ref,
            context_ref,
        })
    }

    /// Returns the globally scoped occurrence key.
    pub const fn stable_key(&self) -> &StableId {
        &self.stable_key
    }

    /// Returns the operation-local occurrence key.
    pub const fn local_stable_key(&self) -> &StableId {
        &self.local_stable_key
    }

    /// Returns ordered child-operation composition scope.
    pub fn child_path(&self) -> &[StableId] {
        &self.child_path
    }

    /// Returns whether this is an authored state or bridge.
    pub const fn base_kind(&self) -> AuthoredBaseKind {
        self.base_kind
    }

    /// Returns the exact state contract.
    pub const fn state_contract_ref(&self) -> &ContentRef {
        &self.state_contract_ref
    }

    /// Returns the exact verified configuration value.
    pub const fn config_ref(&self) -> &ValueRef {
        &self.config_ref
    }

    /// Returns optional exact transition context.
    pub const fn context_ref(&self) -> Option<&ValueRef> {
        self.context_ref.as_ref()
    }
}

/// One exact source selected by an authored input binding.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AuthoredSourceSelector {
    /// A field of the admitted entry-point input root.
    RunAdmission {
        /// `None` selects the complete admitted value.
        source_field_path: Option<FieldPath>,
    },
    /// A field of the exact resolved configured-value root.
    Config {
        /// `None` selects the complete configured value.
        source_field_path: Option<FieldPath>,
    },
    /// A field of one registration-declared admitted support root.
    QualifiedSupport {
        /// Exact member path in the sealed admitted support graph.
        member_path: FieldPath,
        /// `None` selects the complete support root.
        source_field_path: Option<FieldPath>,
    },
    /// A field of the admitted seed root.
    Seed {
        /// `None` selects the complete seed value.
        source_field_path: Option<FieldPath>,
    },
    /// A field of the admitted predecessor-visible context root.
    Context {
        /// `None` selects the complete context value.
        source_field_path: Option<FieldPath>,
    },
    /// A field of one same-run producer output slot.
    NodeOutput {
        /// Authored producer occurrence.
        producer_key: StableId,
        /// Certified producer output slot.
        producer_output_ordinal: u32,
        /// `None` selects the complete output slot.
        source_field_path: Option<FieldPath>,
    },
    /// A field of the admitted cross-run effective-output root.
    CrossRunEffectiveOutput {
        /// `None` selects the complete effective-output root.
        source_field_path: Option<FieldPath>,
    },
    /// A deliberately admitted cross-run evidence value.
    CrossRunEvidence {
        /// `None` selects the complete evidence value.
        source_field_path: Option<FieldPath>,
        /// Exact evidence-only role admitted for this source.
        certified_evidence_role_ref: ContentRef,
    },
}

impl AuthoredSourceSelector {
    /// Returns the selected nested source field, or `None` for the whole value.
    pub const fn source_field_path(&self) -> Option<&FieldPath> {
        match self {
            Self::RunAdmission { source_field_path }
            | Self::Config { source_field_path }
            | Self::QualifiedSupport {
                source_field_path, ..
            }
            | Self::Seed { source_field_path }
            | Self::Context { source_field_path }
            | Self::NodeOutput {
                source_field_path, ..
            }
            | Self::CrossRunEffectiveOutput { source_field_path }
            | Self::CrossRunEvidence {
                source_field_path, ..
            } => source_field_path.as_ref(),
        }
    }

    /// Returns the same-run producer, if this is a node-output selector.
    pub const fn producer_key(&self) -> Option<&StableId> {
        match self {
            Self::NodeOutput { producer_key, .. } => Some(producer_key),
            Self::RunAdmission { .. }
            | Self::Config { .. }
            | Self::QualifiedSupport { .. }
            | Self::Seed { .. }
            | Self::Context { .. }
            | Self::CrossRunEffectiveOutput { .. }
            | Self::CrossRunEvidence { .. } => None,
        }
    }

    /// Returns the selected producer output ordinal, if any.
    pub const fn producer_output_ordinal(&self) -> Option<u32> {
        match self {
            Self::NodeOutput {
                producer_output_ordinal,
                ..
            } => Some(*producer_output_ordinal),
            Self::RunAdmission { .. }
            | Self::Config { .. }
            | Self::QualifiedSupport { .. }
            | Self::Seed { .. }
            | Self::Context { .. }
            | Self::CrossRunEffectiveOutput { .. }
            | Self::CrossRunEvidence { .. } => None,
        }
    }

    /// Returns the admitted cross-run evidence role, if any.
    pub const fn certified_evidence_role_ref(&self) -> Option<&ContentRef> {
        match self {
            Self::CrossRunEvidence {
                certified_evidence_role_ref,
                ..
            } => Some(certified_evidence_role_ref),
            Self::RunAdmission { .. }
            | Self::Config { .. }
            | Self::QualifiedSupport { .. }
            | Self::Seed { .. }
            | Self::Context { .. }
            | Self::NodeOutput { .. }
            | Self::CrossRunEffectiveOutput { .. } => None,
        }
    }

    /// Returns the admitted support member path, if selected.
    pub const fn qualified_support_member_path(&self) -> Option<&FieldPath> {
        match self {
            Self::QualifiedSupport { member_path, .. } => Some(member_path),
            Self::RunAdmission { .. }
            | Self::Config { .. }
            | Self::Seed { .. }
            | Self::Context { .. }
            | Self::NodeOutput { .. }
            | Self::CrossRunEffectiveOutput { .. }
            | Self::CrossRunEvidence { .. } => None,
        }
    }
}

/// One ordered exact source for one authored consumer input destination.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthoredInputBinding {
    consumer_key: StableId,
    consumer_input_ordinal: u32,
    source_ordinal: u32,
    source: AuthoredSourceSelector,
}

impl AuthoredInputBinding {
    /// Constructs one exact ordered authored input binding.
    pub const fn new(
        consumer_key: StableId,
        consumer_input_ordinal: u32,
        source_ordinal: u32,
        source: AuthoredSourceSelector,
    ) -> Self {
        Self {
            consumer_key,
            consumer_input_ordinal,
            source_ordinal,
            source,
        }
    }

    /// Returns the authored consumer key.
    pub const fn consumer_key(&self) -> &StableId {
        &self.consumer_key
    }

    /// Returns the consumer input-destination ordinal.
    pub const fn consumer_input_ordinal(&self) -> u32 {
        self.consumer_input_ordinal
    }

    /// Returns this source's preserved order within the destination.
    pub const fn source_ordinal(&self) -> u32 {
        self.source_ordinal
    }

    /// Returns the exact authored source.
    pub const fn source(&self) -> &AuthoredSourceSelector {
        &self.source
    }
}

/// One typed authored public-output field binding.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthoredPublicOutputBinding {
    field: StableId,
    producer_key: StableId,
    producer_output_ordinal: u32,
    source_field_path: Option<FieldPath>,
}

impl AuthoredPublicOutputBinding {
    /// Constructs one exact public-output field relation.
    pub const fn new(
        field: StableId,
        producer_key: StableId,
        producer_output_ordinal: u32,
        source_field_path: Option<FieldPath>,
    ) -> Self {
        Self {
            field,
            producer_key,
            producer_output_ordinal,
            source_field_path,
        }
    }

    /// Returns the public output field.
    pub const fn field(&self) -> &StableId {
        &self.field
    }

    /// Returns the authored producer key.
    pub const fn producer_key(&self) -> &StableId {
        &self.producer_key
    }

    /// Returns the producer output ordinal.
    pub const fn producer_output_ordinal(&self) -> u32 {
        self.producer_output_ordinal
    }

    /// Returns the optional nested producer field.
    pub const fn source_field_path(&self) -> Option<&FieldPath> {
        self.source_field_path.as_ref()
    }
}

/// Canonical operation-authored program retained before policy expansion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CanonicalAuthoredProgram {
    version: String,
    entry_point_operation_id: StableId,
    nodes: Vec<AuthoredNode>,
    input_bindings: Vec<AuthoredInputBinding>,
    public_output_bindings: Vec<AuthoredPublicOutputBinding>,
    required_success_node_keys: Vec<StableId>,
}

impl CanonicalAuthoredProgram {
    /// Returns the exact retained contract for admission-owned authored programs.
    pub fn retained_contract() -> Result<RetainedValueContract> {
        admission_retained_contract(
            AUTHORED_PROGRAM_CONTRACT,
            "authored-program",
            "mfm.admission.authored-program",
        )
    }

    /// Constructs one fully typed canonical authored graph.
    pub fn new(
        entry_point_operation_id: StableId,
        mut nodes: Vec<AuthoredNode>,
        mut input_bindings: Vec<AuthoredInputBinding>,
        mut public_output_bindings: Vec<AuthoredPublicOutputBinding>,
        mut required_success_node_keys: Vec<StableId>,
    ) -> Result<Self> {
        nodes.sort_by(|left, right| left.stable_key.cmp(&right.stable_key));
        input_bindings.sort();
        public_output_bindings.sort_by(|left, right| left.field.cmp(&right.field));
        required_success_node_keys.sort();
        let program = Self {
            version: AUTHORED_PROGRAM_CONTRACT.to_owned(),
            entry_point_operation_id,
            nodes,
            input_bindings,
            public_output_bindings,
            required_success_node_keys,
        };
        program.validate_invariant()?;
        Ok(program)
    }

    /// Returns the stable entry-point operation identifier.
    pub const fn entry_point_operation_id(&self) -> &StableId {
        &self.entry_point_operation_id
    }

    /// Returns authored nodes in scoped-key order.
    pub fn nodes(&self) -> &[AuthoredNode] {
        &self.nodes
    }

    /// Returns authored input bindings in canonical tuple order.
    pub fn input_bindings(&self) -> &[AuthoredInputBinding] {
        &self.input_bindings
    }

    /// Returns public-output bindings in field order.
    pub fn public_output_bindings(&self) -> &[AuthoredPublicOutputBinding] {
        &self.public_output_bindings
    }

    /// Returns required-success authored node keys.
    pub fn required_success_node_keys(&self) -> &[StableId] {
        &self.required_success_node_keys
    }
}

impl ContractInvariant for CanonicalAuthoredProgram {
    fn validate_invariant(&self) -> Result<()> {
        if self.version != AUTHORED_PROGRAM_CONTRACT
            || self.nodes.is_empty()
            || self.public_output_bindings.is_empty()
            || self.required_success_node_keys.is_empty()
            || !self
                .nodes
                .windows(2)
                .all(|pair| pair[0].stable_key < pair[1].stable_key)
            || !strictly_ordered(&self.input_bindings)
            || !self
                .public_output_bindings
                .windows(2)
                .all(|pair| pair[0].field < pair[1].field)
            || !strictly_ordered(&self.required_success_node_keys)
        {
            return Err(SpecError::Invariant(
                "canonical authored program is not nonempty and uniquely ordered".to_owned(),
            ));
        }
        let known = self
            .nodes
            .iter()
            .map(|node| &node.stable_key)
            .collect::<BTreeSet<_>>();
        if self.input_bindings.iter().any(|binding| {
            !known.contains(&binding.consumer_key)
                || binding
                    .source
                    .producer_key()
                    .is_some_and(|producer| !known.contains(producer))
        }) || self
            .public_output_bindings
            .iter()
            .any(|binding| !known.contains(&binding.producer_key))
            || self
                .required_success_node_keys
                .iter()
                .any(|key| !known.contains(key))
        {
            return Err(SpecError::Invariant(
                "authored relation references an unknown node".to_owned(),
            ));
        }
        for node in &self.nodes {
            if scoped_authored_key(&node.child_path, &node.local_stable_key)? != node.stable_key {
                return Err(SpecError::Invariant(
                    "authored node has a noncanonical scoped key".to_owned(),
                ));
            }
        }
        let mut previous_destination: Option<(StableId, u32)> = None;
        let mut expected_source_ordinal = 0_u32;
        for binding in &self.input_bindings {
            let destination = (binding.consumer_key.clone(), binding.consumer_input_ordinal);
            if previous_destination.as_ref() != Some(&destination) {
                expected_source_ordinal = 0;
                previous_destination = Some(destination);
            }
            if binding.source_ordinal != expected_source_ordinal {
                return Err(SpecError::Invariant(
                    "authored source ordinals must be contiguous from zero".to_owned(),
                ));
            }
            expected_source_ordinal = expected_source_ordinal.checked_add(1).ok_or_else(|| {
                SpecError::Invariant("authored source ordinal overflow".to_owned())
            })?;
        }
        validate_authored_acyclic(&self.nodes, &self.input_bindings)?;
        Ok(())
    }
}

impl_codec!(CanonicalAuthoredProgram, AUTHORED_PROGRAM_CONTRACT);

fn scoped_authored_key(child_path: &[StableId], local: &StableId) -> Result<StableId> {
    if child_path.is_empty() {
        return Ok(local.clone());
    }
    StableId::new(format!(
        "{}/{}",
        child_path
            .iter()
            .map(StableId::as_str)
            .collect::<Vec<_>>()
            .join("/"),
        local.as_str()
    ))
    .map_err(|error| SpecError::Identity(error.to_string()))
}

mod value_ref_serde {
    use super::*;

    pub(super) fn serialize<S>(
        value: &ValueRef,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let json = serde_json::from_slice::<serde_json::Value>(value.as_bytes())
            .map_err(serde::ser::Error::custom)?;
        json.serialize(serializer)
    }

    pub(super) fn deserialize<'de, D>(deserializer: D) -> std::result::Result<ValueRef, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let json = serde_json::Value::deserialize(deserializer)?;
        let bytes = canonical(&json).map_err(serde::de::Error::custom)?;
        ValueRef::strict_decode(bytes.as_bytes()).map_err(serde::de::Error::custom)
    }
}

mod optional_value_ref_serde {
    use super::*;

    pub(super) fn serialize<S>(
        value: &Option<ValueRef>,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let json = value
            .as_ref()
            .map(|value| {
                serde_json::from_slice::<serde_json::Value>(value.as_bytes())
                    .map_err(serde::ser::Error::custom)
            })
            .transpose()?;
        json.serialize(serializer)
    }

    pub(super) fn deserialize<'de, D>(
        deserializer: D,
    ) -> std::result::Result<Option<ValueRef>, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Option::<serde_json::Value>::deserialize(deserializer)?
            .map(|json| {
                let bytes = canonical(&json).map_err(serde::de::Error::custom)?;
                ValueRef::strict_decode(bytes.as_bytes()).map_err(serde::de::Error::custom)
            })
            .transpose()
    }
}

fn strictly_ordered<T: Ord>(values: &[T]) -> bool {
    values.windows(2).all(|pair| pair[0] < pair[1])
}

fn validate_authored_acyclic(
    nodes: &[AuthoredNode],
    bindings: &[AuthoredInputBinding],
) -> Result<()> {
    let mut indegree = nodes
        .iter()
        .map(|node| (node.stable_key.clone(), 0_usize))
        .collect::<BTreeMap<_, _>>();
    let mut consumers = BTreeMap::<StableId, BTreeSet<StableId>>::new();
    for binding in bindings {
        let Some(producer) = binding.source.producer_key() else {
            continue;
        };
        let consumer = &binding.consumer_key;
        let inserted = consumers
            .entry(producer.clone())
            .or_default()
            .insert(consumer.clone());
        if inserted {
            let degree = indegree.get_mut(consumer).ok_or_else(|| {
                SpecError::Invariant("authored input references an unknown consumer".to_owned())
            })?;
            *degree = degree.checked_add(1).ok_or_else(|| {
                SpecError::Invariant("authored dependency count overflow".to_owned())
            })?;
        }
    }
    let mut ready = indegree
        .iter()
        .filter_map(|(node, degree)| (*degree == 0).then_some(node.clone()))
        .collect::<BTreeSet<_>>();
    let mut visited = 0_usize;
    while let Some(node) = ready.pop_first() {
        visited += 1;
        for consumer in consumers.get(&node).into_iter().flatten() {
            let degree = indegree.get_mut(consumer).ok_or_else(|| {
                SpecError::Invariant("authored input references an unknown consumer".to_owned())
            })?;
            *degree = degree.checked_sub(1).ok_or_else(|| {
                SpecError::Invariant("authored dependency count underflow".to_owned())
            })?;
            if *degree == 0 {
                ready.insert(consumer.clone());
            }
        }
    }
    if visited != nodes.len() {
        return Err(SpecError::Invariant(
            "authored node-output graph contains a cycle".to_owned(),
        ));
    }
    Ok(())
}

/// Exact composite-planner profile selected by an entry point.
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
    /// Constructs an exact planning profile.
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

    /// Returns canonical planner parameters.
    pub fn canonical_profile_parameters(&self) -> &CanonicalJsonValue {
        &self.canonical_profile_parameters
    }

    /// Returns ordered framework policies.
    pub fn framework_policy_refs(&self) -> &[ContentRef] {
        &self.framework_policy_refs
    }

    /// Returns the planner semantic contract.
    pub fn planner_contract_ref(&self) -> &ContentRef {
        &self.planner_contract_ref
    }

    /// Returns the exact planner component implementation.
    pub fn planner_implementation_ref(&self) -> &ContentRef {
        &self.planner_implementation_ref
    }
}

impl ContractInvariant for PlanningProfile {
    fn validate_invariant(&self) -> Result<()> {
        if self.version != PLANNING_PROFILE_CONTRACT {
            return Err(SpecError::Invariant(
                "planning profile version mismatch".to_owned(),
            ));
        }
        let unique = self.framework_policy_refs.iter().collect::<BTreeSet<_>>();
        if unique.len() != self.framework_policy_refs.len() {
            return Err(SpecError::Invariant(
                "framework policy references must be unique".to_owned(),
            ));
        }
        Ok(())
    }
}

impl_codec!(PlanningProfile, PLANNING_PROFILE_CONTRACT);

/// Published compiled entry-point contract.
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
    /// Returns the exact retained contract for admission-owned entry-point contracts.
    pub fn retained_contract() -> Result<RetainedValueContract> {
        admission_retained_contract(
            ENTRY_POINT_CONTRACT,
            "entry-point-contract",
            "mfm.admission.entry-point-contract",
        )
    }

    /// Constructs a published entry-point contract.
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
    pub fn entry_point_id(&self) -> &EntryPointId {
        &self.entry_point_id
    }

    /// Returns the stable run-identity operation identifier.
    pub fn entry_point_operation_id(&self) -> &StableId {
        &self.entry_point_operation_id
    }

    /// Returns the exact profile.
    pub fn planning_profile(&self) -> &PlanningProfile {
        &self.planning_profile
    }

    /// Returns the exact profile reference.
    pub fn planning_profile_ref(&self) -> &ContentRef {
        &self.planning_profile_ref
    }

    /// Returns the admitted input schema.
    pub fn input_schema_id(&self) -> &SchemaId {
        &self.input_schema_id
    }

    /// Returns the public output schema.
    pub fn public_output_schema_id(&self) -> &SchemaId {
        &self.public_output_schema_id
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

impl_codec!(EntryPointContract, ENTRY_POINT_CONTRACT);

/// One step in a canonical post-composition expansion path.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CanonicalExpansionStep {
    /// Path root.
    EntryPoint,
    /// Nested child-operation scope.
    NestedChild {
        /// Child stable key.
        stable_key: StableId,
        /// Canonical sibling ordinal.
        ordinal: u32,
    },
    /// Authored state occurrence.
    Authored {
        /// Authored stable key.
        stable_key: StableId,
        /// Canonical sibling ordinal.
        ordinal: u32,
    },
    /// Same-value child composition bridge.
    Bridge {
        /// Bridge stable key.
        stable_key: StableId,
        /// Canonical sibling ordinal.
        ordinal: u32,
    },
    /// Framework pre-state.
    FrameworkPre {
        /// Exact policy.
        policy_ref: ContentRef,
        /// Policy ordinal in the profile.
        policy_ordinal: u32,
        /// State ordinal in that pre-chain.
        state_ordinal: u32,
    },
    /// State protected by a framework policy.
    FrameworkProtected {
        /// Exact policy.
        policy_ref: ContentRef,
        /// Policy ordinal in the profile.
        policy_ordinal: u32,
    },
    /// Framework post-state.
    FrameworkPost {
        /// Exact policy.
        policy_ref: ContentRef,
        /// Policy ordinal in the profile.
        policy_ordinal: u32,
        /// State ordinal in that post-chain.
        state_ordinal: u32,
    },
    /// Executor pre-state.
    ExecutorPre {
        /// Exact executor contract.
        executor_contract_ref: ContentRef,
        /// State ordinal in the pre-chain.
        state_ordinal: u32,
    },
    /// Effect protected by an executor contract.
    ExecutorProtected {
        /// Exact executor contract.
        executor_contract_ref: ContentRef,
    },
    /// Executor post-state.
    ExecutorPost {
        /// Exact executor contract.
        executor_contract_ref: ContentRef,
        /// State ordinal in the post-chain.
        state_ordinal: u32,
    },
}

/// Canonical expansion path used in node identity.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CanonicalExpansionPath(Vec<CanonicalExpansionStep>);

impl CanonicalExpansionPath {
    /// Constructs and validates a path.
    pub fn new(steps: Vec<CanonicalExpansionStep>) -> Result<Self> {
        let path = Self(steps);
        path.validate()?;
        Ok(path)
    }

    /// Returns path steps.
    pub fn steps(&self) -> &[CanonicalExpansionStep] {
        &self.0
    }

    /// Appends one planner-owned step and validates the result.
    pub fn with_step(&self, step: CanonicalExpansionStep) -> Result<Self> {
        let mut steps = self.0.clone();
        steps.push(step);
        Self::new(steps)
    }

    fn validate(&self) -> Result<()> {
        if !(2..=1024).contains(&self.0.len())
            || !matches!(self.0.first(), Some(CanonicalExpansionStep::EntryPoint))
        {
            return Err(SpecError::Invariant(
                "expansion path must begin at entry point and contain a base".to_owned(),
            ));
        }
        let bases = self
            .0
            .iter()
            .filter(|step| {
                matches!(
                    step,
                    CanonicalExpansionStep::Authored { .. } | CanonicalExpansionStep::Bridge { .. }
                )
            })
            .count();
        if bases != 1 {
            return Err(SpecError::Invariant(
                "expansion path must contain exactly one authored or bridge base".to_owned(),
            ));
        }
        let Some(base) = self.0.iter().position(|step| {
            matches!(
                step,
                CanonicalExpansionStep::Authored { .. } | CanonicalExpansionStep::Bridge { .. }
            )
        }) else {
            return Err(SpecError::Invariant(
                "expansion path must contain an authored or bridge base".to_owned(),
            ));
        };
        if self.0[1..base]
            .iter()
            .any(|step| !matches!(step, CanonicalExpansionStep::NestedChild { .. }))
            || self.0[base + 1..]
                .iter()
                .any(|step| matches!(step, CanonicalExpansionStep::NestedChild { .. }))
        {
            return Err(SpecError::Invariant(
                "child steps must precede the authored base".to_owned(),
            ));
        }
        let expansion = &self.0[base + 1..];
        let framework_count = expansion
            .iter()
            .take_while(|step| {
                matches!(
                    step,
                    CanonicalExpansionStep::FrameworkPre { .. }
                        | CanonicalExpansionStep::FrameworkProtected { .. }
                        | CanonicalExpansionStep::FrameworkPost { .. }
                )
            })
            .count();
        let framework = &expansion[..framework_count];
        let executor = &expansion[framework_count..];

        let mut prior_policy_ordinal = None;
        for (index, step) in framework.iter().enumerate() {
            let Some((policy_ordinal, terminal)) = (match step {
                CanonicalExpansionStep::FrameworkProtected { policy_ordinal, .. } => {
                    Some((*policy_ordinal, false))
                }
                CanonicalExpansionStep::FrameworkPre { policy_ordinal, .. }
                | CanonicalExpansionStep::FrameworkPost { policy_ordinal, .. } => {
                    Some((*policy_ordinal, true))
                }
                _ => None,
            }) else {
                return Err(SpecError::Invariant(
                    "framework expansion contains a non-framework step".to_owned(),
                ));
            };
            if prior_policy_ordinal.is_some_and(|prior| prior >= policy_ordinal)
                || (terminal && index + 1 != framework.len())
            {
                return Err(SpecError::Invariant(
                    "framework expansion path is not a strict outer-to-inner chain".to_owned(),
                ));
            }
            prior_policy_ordinal = Some(policy_ordinal);
        }
        if executor.len() > 1
            || executor.iter().any(|step| {
                !matches!(
                    step,
                    CanonicalExpansionStep::ExecutorPre { .. }
                        | CanonicalExpansionStep::ExecutorProtected { .. }
                        | CanonicalExpansionStep::ExecutorPost { .. }
                )
            })
        {
            return Err(SpecError::Invariant(
                "executor expansion path must be one innermost occurrence".to_owned(),
            ));
        }
        Ok(())
    }

    /// Derives the final node id using the frozen universal semantic domain.
    pub fn node_id(&self, state_contract_ref: &ContentRef) -> Result<NodeId> {
        self.validate()?;
        let path =
            serde_json::to_value(self).map_err(|error| SpecError::Contract(error.to_string()))?;
        let preimage = serde_json::json!({
            "identity_contract_version": "mfm.node-occurrence.v1",
            "canonical_expansion_path": path,
            "state_contract_ref": state_contract_ref,
        });
        let checked = validated("mfm.node-identity-preimage.v1", &preimage)?;
        let digest = contract()?.semantic_digest("mfm.node-occurrence.v1", &checked)?;
        Ok(NodeId::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            *digest.digest(),
        ))
    }
}

/// Closed run terminal contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunTerminalContract {
    required_success_nodes: Vec<NodeId>,
    requires_public_output: bool,
}

impl RunTerminalContract {
    /// Constructs a terminal contract.
    pub fn new(mut required_success_nodes: Vec<NodeId>, requires_public_output: bool) -> Self {
        required_success_nodes.sort();
        required_success_nodes.dedup();
        Self {
            required_success_nodes,
            requires_public_output,
        }
    }

    /// Returns nodes required to succeed.
    pub fn required_success_nodes(&self) -> &[NodeId] {
        &self.required_success_nodes
    }

    /// Returns whether closure requires complete public-output assembly.
    pub const fn requires_public_output(&self) -> bool {
        self.requires_public_output
    }
}

/// Exact retained contracts for journal-owned runtime protocol values.
///
/// These contracts are certified with the graph so runtime never infers a
/// journal protocol schema from local implementation details.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CertifiedJournalProtocolContracts {
    input_manifest_contract: RetainedValueContract,
    fact_claim_envelope_contract: RetainedValueContract,
    frozen_read_intent_contract: RetainedValueContract,
}

impl CertifiedJournalProtocolContracts {
    /// Returns the sole deterministic journal protocol contract set.
    pub fn current() -> Result<Self> {
        Self::new(
            InputManifest::retained_contract()?,
            FactClaimEnvelope::retained_contract()?,
            FrozenReadIntent::retained_contract()?,
        )
    }

    /// Constructs the complete journal protocol contract set.
    pub fn new(
        input_manifest_contract: RetainedValueContract,
        fact_claim_envelope_contract: RetainedValueContract,
        frozen_read_intent_contract: RetainedValueContract,
    ) -> Result<Self> {
        if input_manifest_contract != InputManifest::retained_contract()?
            || fact_claim_envelope_contract != FactClaimEnvelope::retained_contract()?
            || frozen_read_intent_contract != FrozenReadIntent::retained_contract()?
        {
            return Err(SpecError::Invariant(
                "journal protocol retained-value contracts differ from their exact factories"
                    .to_owned(),
            ));
        }
        Ok(Self {
            input_manifest_contract,
            fact_claim_envelope_contract,
            frozen_read_intent_contract,
        })
    }

    /// Returns the retained input-manifest contract.
    pub const fn input_manifest_contract(&self) -> &RetainedValueContract {
        &self.input_manifest_contract
    }

    /// Returns the retained fact-claim-envelope contract.
    pub const fn fact_claim_envelope_contract(&self) -> &RetainedValueContract {
        &self.fact_claim_envelope_contract
    }

    /// Returns the retained frozen-read-intent contract.
    pub const fn frozen_read_intent_contract(&self) -> &RetainedValueContract {
        &self.frozen_read_intent_contract
    }
}

/// Final expanded graph and effective-output contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExpandedCertifiedSpec {
    version: String,
    authored_program_ref: ContentRef,
    planning_profile_ref: ContentRef,
    nodes: Vec<CertifiedNodeContract>,
    public_output_contract: PublicOutputContract,
    run_terminal_contract: RunTerminalContract,
    journal_protocol_contracts: CertifiedJournalProtocolContracts,
}

impl ExpandedCertifiedSpec {
    /// Returns the exact retained contract for admission-owned expanded specifications.
    pub fn retained_contract() -> Result<RetainedValueContract> {
        admission_retained_contract(
            EXPANDED_SPEC_CONTRACT,
            "expanded-certified-spec",
            "mfm.admission.expanded-certified-spec",
        )
    }

    /// Constructs a canonical expanded graph.
    pub fn new(
        authored_program_ref: ContentRef,
        planning_profile_ref: ContentRef,
        mut nodes: Vec<CertifiedNodeContract>,
        public_output_contract: PublicOutputContract,
        run_terminal_contract: RunTerminalContract,
        journal_protocol_contracts: CertifiedJournalProtocolContracts,
    ) -> Result<Self> {
        nodes.sort_by(|left, right| left.node_id().cmp(right.node_id()));
        let spec = Self {
            version: EXPANDED_SPEC_CONTRACT.to_owned(),
            authored_program_ref,
            planning_profile_ref,
            nodes,
            public_output_contract,
            run_terminal_contract,
            journal_protocol_contracts,
        };
        spec.validate_invariant()?;
        Ok(spec)
    }

    /// Returns the retained authored-program reference.
    pub fn authored_program_ref(&self) -> &ContentRef {
        &self.authored_program_ref
    }

    /// Returns the exact planning-profile reference.
    pub fn planning_profile_ref(&self) -> &ContentRef {
        &self.planning_profile_ref
    }

    /// Returns expanded nodes in canonical node-id order.
    pub fn nodes(&self) -> &[CertifiedNodeContract] {
        &self.nodes
    }

    /// Returns the exact public-output assembly contract.
    pub const fn public_output_contract(&self) -> &PublicOutputContract {
        &self.public_output_contract
    }

    /// Returns the terminal contract.
    pub fn run_terminal_contract(&self) -> &RunTerminalContract {
        &self.run_terminal_contract
    }

    /// Returns the complete certified journal protocol contracts.
    pub const fn journal_protocol_contracts(&self) -> &CertifiedJournalProtocolContracts {
        &self.journal_protocol_contracts
    }

    /// Derives the frozen semantic spec hash.
    pub fn spec_hash(&self) -> Result<SpecHash> {
        self.validate_invariant()?;
        let expanded =
            serde_json::to_value(self).map_err(|error| SpecError::Contract(error.to_string()))?;
        let checked = validated(
            "mfm.spec-hash-preimage.v1",
            &serde_json::json!({"expanded_spec": expanded}),
        )?;
        let digest = contract()?.semantic_digest("mfm.spec-hash.v1", &checked)?;
        Ok(SpecHash::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            *digest.digest(),
        ))
    }
}

impl ContractInvariant for ExpandedCertifiedSpec {
    fn validate_invariant(&self) -> Result<()> {
        if self.version != EXPANDED_SPEC_CONTRACT || self.nodes.is_empty() {
            return Err(SpecError::Invariant(
                "expanded spec version or node set is invalid".to_owned(),
            ));
        }
        let mut prior = None;
        let known = self
            .nodes
            .iter()
            .map(|node| node.node_id().clone())
            .collect::<BTreeSet<_>>();
        validate_certified_graph(&self.nodes)?;
        for node in &self.nodes {
            if prior.as_ref().is_some_and(|id| id >= node.node_id()) {
                return Err(SpecError::Invariant(
                    "expanded nodes are not strictly node-id ordered".to_owned(),
                ));
            }
            prior = Some(node.node_id().clone());
        }
        for node in &self.run_terminal_contract.required_success_nodes {
            if !known.contains(node) {
                return Err(SpecError::Invariant(
                    "terminal contract names an unknown node".to_owned(),
                ));
            }
        }
        for binding in self.public_output_contract.bindings() {
            let producer = self
                .nodes
                .iter()
                .find(|node| node.node_id() == binding.source_node_id())
                .ok_or_else(|| {
                    SpecError::Invariant("public output names an unknown producer".to_owned())
                })?;
            let output = producer
                .settlement_contract()
                .output_slots()
                .iter()
                .find(|slot| slot.output_ordinal() == binding.output_ordinal())
                .ok_or_else(|| {
                    SpecError::Invariant("public output names an unknown producer slot".to_owned())
                })?;
            if binding.source_field_path().is_none()
                && output.value_contract() != binding.value_contract()
            {
                return Err(SpecError::Invariant(
                    "public output differs from its producer slot contract".to_owned(),
                ));
            }
        }
        if graph_has_cycle(&self.nodes) {
            return Err(SpecError::Invariant(
                "expanded dependency graph contains a cycle".to_owned(),
            ));
        }
        Ok(())
    }
}

impl_codec!(ExpandedCertifiedSpec, EXPANDED_SPEC_CONTRACT);

fn graph_has_cycle(nodes: &[CertifiedNodeContract]) -> bool {
    fn visit(
        node: &NodeId,
        graph: &BTreeMap<NodeId, Vec<NodeId>>,
        visiting: &mut BTreeSet<NodeId>,
        visited: &mut BTreeSet<NodeId>,
    ) -> bool {
        if visited.contains(node) {
            return false;
        }
        if !visiting.insert(node.clone()) {
            return true;
        }
        if graph
            .get(node)
            .into_iter()
            .flatten()
            .any(|next| visit(next, graph, visiting, visited))
        {
            return true;
        }
        visiting.remove(node);
        visited.insert(node.clone());
        false
    }

    let graph = nodes
        .iter()
        .map(|node| {
            (
                node.node_id().clone(),
                node.direct_producer_ids().into_iter().collect(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut visiting = BTreeSet::new();
    let mut visited = BTreeSet::new();
    graph
        .keys()
        .any(|node| visit(node, &graph, &mut visiting, &mut visited))
}

/// Kind of one locally admitted component implementation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComponentKind {
    /// Pure composite planner.
    Planner,
    /// State callback implementation.
    State,
    /// Read capability adapter/verifier.
    ReadCapabilityAdapterVerifier,
    /// Executor client verifier.
    ExecutorClientVerifier,
}

/// Exact component implementation descriptor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComponentImplementationDescriptor {
    version: String,
    component_kind: ComponentKind,
    semantic_contract_ref: ContentRef,
    callback_surface_ref: ContentRef,
    qualification_ref: ContentRef,
}

impl ComponentImplementationDescriptor {
    /// Constructs a component descriptor.
    pub fn new(
        component_kind: ComponentKind,
        semantic_contract_ref: ContentRef,
        callback_surface_ref: ContentRef,
        qualification_ref: ContentRef,
    ) -> Result<Self> {
        let descriptor = Self {
            version: COMPONENT_IMPLEMENTATION_CONTRACT.to_owned(),
            component_kind,
            semantic_contract_ref,
            callback_surface_ref,
            qualification_ref,
        };
        descriptor.validate_invariant()?;
        Ok(descriptor)
    }

    /// Returns the component kind.
    pub fn component_kind(&self) -> ComponentKind {
        self.component_kind
    }

    /// Returns the component semantic contract.
    pub fn semantic_contract_ref(&self) -> &ContentRef {
        &self.semantic_contract_ref
    }

    /// Returns the exact callback surface.
    pub fn callback_surface_ref(&self) -> &ContentRef {
        &self.callback_surface_ref
    }

    /// Returns the qualification evidence.
    pub fn qualification_ref(&self) -> &ContentRef {
        &self.qualification_ref
    }
}

impl ContractInvariant for ComponentImplementationDescriptor {
    fn validate_invariant(&self) -> Result<()> {
        if self.version == COMPONENT_IMPLEMENTATION_CONTRACT {
            Ok(())
        } else {
            Err(SpecError::Invariant(
                "component descriptor version mismatch".to_owned(),
            ))
        }
    }
}

impl_codec!(
    ComponentImplementationDescriptor,
    COMPONENT_IMPLEMENTATION_CONTRACT
);

/// Exact state implementation selected for a state contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StateImplementationManifestEntry {
    /// State semantic contract.
    pub state_contract_ref: ContentRef,
    /// Exact admitted component implementation.
    pub component_implementation_ref: ContentRef,
}

/// Exact per-run state implementation manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StateImplementationManifest {
    version: String,
    entries: Vec<StateImplementationManifestEntry>,
}

impl StateImplementationManifest {
    /// Returns the exact retained contract for admission-owned state manifests.
    pub fn retained_contract() -> Result<RetainedValueContract> {
        admission_retained_contract(
            STATE_IMPLEMENTATION_MANIFEST_CONTRACT,
            "state-implementation-manifest",
            "mfm.admission.state-implementation-manifest",
        )
    }

    /// Constructs a manifest with one entry per used state contract.
    pub fn new(mut entries: Vec<StateImplementationManifestEntry>) -> Result<Self> {
        entries.sort_by(|left, right| {
            compare_content_ref_wire_order(&left.state_contract_ref, &right.state_contract_ref)
        });
        let manifest = Self {
            version: STATE_IMPLEMENTATION_MANIFEST_CONTRACT.to_owned(),
            entries,
        };
        manifest.validate_invariant()?;
        Ok(manifest)
    }

    /// Returns selected entries.
    pub fn entries(&self) -> &[StateImplementationManifestEntry] {
        &self.entries
    }
}

impl ContractInvariant for StateImplementationManifest {
    fn validate_invariant(&self) -> Result<()> {
        if self.version != STATE_IMPLEMENTATION_MANIFEST_CONTRACT
            || self.entries.is_empty()
            || !self.entries.windows(2).all(|pair| {
                compare_content_ref_wire_order(
                    &pair[0].state_contract_ref,
                    &pair[1].state_contract_ref,
                )
                .is_lt()
            })
        {
            return Err(SpecError::Invariant(
                "state implementation manifest is not exact and unique".to_owned(),
            ));
        }
        Ok(())
    }
}

fn compare_content_ref_wire_order(left: &ContentRef, right: &ContentRef) -> std::cmp::Ordering {
    // `field:state_contract_ref` orders the canonical object value. Its frozen
    // JCS field order is content_digest followed by schema_id, which differs
    // from ContentRef's type-level field order.
    left.content_digest()
        .cmp(right.content_digest())
        .then_with(|| left.schema_id().cmp(right.schema_id()))
}

impl_codec!(
    StateImplementationManifest,
    STATE_IMPLEMENTATION_MANIFEST_CONTRACT
);

/// Exact capability operation binding selected for a run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityBindingManifestEntry {
    /// Capability operation id.
    pub operation_id: StableId,
    /// Exact read or executor binding.
    pub binding_ref: ContentRef,
}

/// Exact per-run capability binding manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityBindingManifest {
    version: String,
    entries: Vec<CapabilityBindingManifestEntry>,
}

impl CapabilityBindingManifest {
    /// Returns the exact retained contract for admission-owned capability manifests.
    pub fn retained_contract() -> Result<RetainedValueContract> {
        admission_retained_contract(
            CAPABILITY_BINDING_MANIFEST_CONTRACT,
            "capability-binding-manifest",
            "mfm.admission.capability-binding-manifest",
        )
    }

    /// Constructs a manifest with one binding per operation id.
    pub fn new(mut entries: Vec<CapabilityBindingManifestEntry>) -> Result<Self> {
        entries.sort_by(|left, right| left.operation_id.cmp(&right.operation_id));
        let manifest = Self {
            version: CAPABILITY_BINDING_MANIFEST_CONTRACT.to_owned(),
            entries,
        };
        manifest.validate_invariant()?;
        Ok(manifest)
    }

    /// Returns selected bindings.
    pub fn entries(&self) -> &[CapabilityBindingManifestEntry] {
        &self.entries
    }
}

impl ContractInvariant for CapabilityBindingManifest {
    fn validate_invariant(&self) -> Result<()> {
        if self.version != CAPABILITY_BINDING_MANIFEST_CONTRACT
            || !self
                .entries
                .windows(2)
                .all(|pair| pair[0].operation_id < pair[1].operation_id)
        {
            return Err(SpecError::Invariant(
                "capability binding manifest is not exact and unique".to_owned(),
            ));
        }
        Ok(())
    }
}

impl_codec!(
    CapabilityBindingManifest,
    CAPABILITY_BINDING_MANIFEST_CONTRACT
);

/// One certificate proof-closure entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CertificateProofEntry {
    /// Stable proof kind.
    pub proof_kind: StableId,
    /// Subject proven by this entry.
    pub subject_ref: ContentRef,
    /// Exact proof evidence.
    pub evidence_ref: ContentRef,
}

/// Frozen certificate binding an expanded graph and its complete proof closure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Certificate {
    version: String,
    spec_hash: SpecHash,
    certified_spec_ref: ContentRef,
    proof_closure: Vec<CertificateProofEntry>,
}

impl Certificate {
    /// Returns the exact retained contract for admission-owned certificates.
    pub fn retained_contract() -> Result<RetainedValueContract> {
        admission_retained_contract(
            CERTIFICATE_CONTRACT,
            "certificate",
            "mfm.admission.certificate",
        )
    }

    /// Constructs a certificate.
    pub fn new(
        spec_hash: SpecHash,
        certified_spec_ref: ContentRef,
        mut proof_closure: Vec<CertificateProofEntry>,
    ) -> Result<Self> {
        proof_closure.sort_by(|left, right| {
            (&left.proof_kind, &left.subject_ref).cmp(&(&right.proof_kind, &right.subject_ref))
        });
        let certificate = Self {
            version: CERTIFICATE_CONTRACT.to_owned(),
            spec_hash,
            certified_spec_ref,
            proof_closure,
        };
        certificate.validate_invariant()?;
        Ok(certificate)
    }

    /// Returns the certified spec hash.
    pub fn spec_hash(&self) -> &SpecHash {
        &self.spec_hash
    }

    /// Returns the exact expanded-spec reference.
    pub fn certified_spec_ref(&self) -> &ContentRef {
        &self.certified_spec_ref
    }

    /// Returns sorted proof closure entries.
    pub fn proof_closure(&self) -> &[CertificateProofEntry] {
        &self.proof_closure
    }
}

impl ContractInvariant for Certificate {
    fn validate_invariant(&self) -> Result<()> {
        if self.version != CERTIFICATE_CONTRACT {
            return Err(SpecError::Invariant(
                "certificate version mismatch".to_owned(),
            ));
        }
        if self.proof_closure.is_empty()
            || !self.proof_closure.windows(2).all(|pair| {
                (&pair[0].proof_kind, &pair[0].subject_ref)
                    < (&pair[1].proof_kind, &pair[1].subject_ref)
            })
        {
            return Err(SpecError::Invariant(
                "certificate proof closure must be nonempty and uniquely ordered".to_owned(),
            ));
        }
        Ok(())
    }
}

impl_codec!(Certificate, CERTIFICATE_CONTRACT);

/// Complete coherent certification closure consumed by run admission.
///
/// This value is intentionally non-cloneable. Certification constructs it
/// only after the entry point, authored graph, expanded graph, certificate,
/// and both exact manifests agree.
#[derive(Debug)]
pub struct CertifiedAdmissionArtifacts {
    entry_point: EntryPointContract,
    authored_program: CanonicalAuthoredProgram,
    expanded_spec: ExpandedCertifiedSpec,
    certificate: Certificate,
    state_manifest: StateImplementationManifest,
    capability_manifest: CapabilityBindingManifest,
}

impl CertifiedAdmissionArtifacts {
    /// Constructs the complete artifact closure after deterministic certification.
    ///
    /// This is public only for the framework-owned certification implementation;
    /// other callers should obtain this value from that implementation.
    #[doc(hidden)]
    pub fn from_certification(
        entry_point: EntryPointContract,
        authored_program: CanonicalAuthoredProgram,
        expanded_spec: ExpandedCertifiedSpec,
        certificate: Certificate,
        state_manifest: StateImplementationManifest,
        capability_manifest: CapabilityBindingManifest,
    ) -> Result<Self> {
        let entry_point_ref = entry_point.content_ref()?;
        let authored_program_ref = authored_program.content_ref()?;
        let expanded_spec_ref = expanded_spec.content_ref()?;
        let state_manifest_ref = state_manifest.content_ref()?;
        let capability_manifest_ref = capability_manifest.content_ref()?;
        if authored_program.entry_point_operation_id() != entry_point.entry_point_operation_id()
            || expanded_spec.authored_program_ref() != &authored_program_ref
            || expanded_spec.planning_profile_ref() != entry_point.planning_profile_ref()
            || expanded_spec
                .public_output_contract()
                .value_contract()
                .schema_id()
                != entry_point.public_output_schema_id()
            || !expanded_spec
                .run_terminal_contract()
                .requires_public_output()
            || certificate.certified_spec_ref() != &expanded_spec_ref
            || certificate.spec_hash() != &expanded_spec.spec_hash()?
        {
            return Err(SpecError::Invariant(
                "certified admission artifacts disagree on their root contracts".to_owned(),
            ));
        }

        let expected_states = expanded_spec
            .nodes()
            .iter()
            .map(|node| node.state_contract_ref().clone())
            .collect::<BTreeSet<_>>();
        let actual_states = state_manifest
            .entries()
            .iter()
            .map(|entry| entry.state_contract_ref.clone())
            .collect::<BTreeSet<_>>();
        if expected_states != actual_states {
            return Err(SpecError::Invariant(
                "certified admission state manifest is not exact-total".to_owned(),
            ));
        }

        let mut expected_bindings = BTreeMap::new();
        for node in expanded_spec.nodes() {
            let Some((operation_id, binding_ref)) = node.execution().operation_binding() else {
                continue;
            };
            if expected_bindings
                .insert(operation_id.clone(), binding_ref.clone())
                .is_some_and(|previous| previous != *binding_ref)
            {
                return Err(SpecError::Invariant(
                    "certified graph binds one operation to multiple capabilities".to_owned(),
                ));
            }
        }
        let actual_bindings = capability_manifest
            .entries()
            .iter()
            .map(|entry| (entry.operation_id.clone(), entry.binding_ref.clone()))
            .collect::<BTreeMap<_, _>>();
        if expected_bindings != actual_bindings {
            return Err(SpecError::Invariant(
                "certified admission capability manifest is not exact-total".to_owned(),
            ));
        }

        let required_proofs = [
            (
                "entry-point-profile",
                &entry_point_ref,
                entry_point.planning_profile_ref(),
            ),
            (
                "authored-program",
                &expanded_spec_ref,
                &authored_program_ref,
            ),
            ("state-manifest", &expanded_spec_ref, &state_manifest_ref),
            (
                "capability-manifest",
                &expanded_spec_ref,
                &capability_manifest_ref,
            ),
            ("expanded-spec", &expanded_spec_ref, &expanded_spec_ref),
        ];
        if required_proofs.iter().any(|(kind, subject, evidence)| {
            !certificate.proof_closure().iter().any(|proof| {
                proof.proof_kind.as_str() == *kind
                    && &proof.subject_ref == *subject
                    && &proof.evidence_ref == *evidence
            })
        }) {
            return Err(SpecError::Invariant(
                "certified admission certificate omits a required root proof".to_owned(),
            ));
        }

        Ok(Self {
            entry_point,
            authored_program,
            expanded_spec,
            certificate,
            state_manifest,
            capability_manifest,
        })
    }

    /// Returns the exact published entry-point contract.
    pub const fn entry_point(&self) -> &EntryPointContract {
        &self.entry_point
    }

    /// Returns the exact canonical authored graph.
    pub const fn authored_program(&self) -> &CanonicalAuthoredProgram {
        &self.authored_program
    }

    /// Returns the exact expanded certified graph.
    pub const fn expanded_spec(&self) -> &ExpandedCertifiedSpec {
        &self.expanded_spec
    }

    /// Returns the exact certificate over the expanded graph.
    pub const fn certificate(&self) -> &Certificate {
        &self.certificate
    }

    /// Returns the exact state implementation manifest.
    pub const fn state_manifest(&self) -> &StateImplementationManifest {
        &self.state_manifest
    }

    /// Returns the exact capability binding manifest.
    pub const fn capability_manifest(&self) -> &CapabilityBindingManifest {
        &self.capability_manifest
    }
}

/// Returns the annex schema id for a named frozen contract.
pub fn schema_id(schema_contract: &str) -> Result<SchemaId> {
    contract()?
        .schema_id(schema_contract)
        .cloned()
        .map_err(Into::into)
}
