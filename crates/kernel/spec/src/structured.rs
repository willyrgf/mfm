//! Canonical data contracts for declaration-ordered structured programs.
//!
//! Values in this module are untrusted specification data until
//! `mfm-certify` has validated the complete authored/expanded closure.  The
//! module deliberately contains no callbacks, store access, or execution
//! authority.

use std::collections::{BTreeMap, BTreeSet};

use mfm_canonical::{sha256_digest_bytes, PlainCanonicalJsonBytes};
use mfm_ids::{
    ContentDigest, ContentRef, DigestAlgorithm, FailurePlanId, FragmentBoundaryId, OccurrenceId,
    SchemaId, SemanticCallId, StableId,
};
use mfm_values::{component_object_evidence_contract_ref, MfmValue, RetainedValueContract};
use serde::{de::Error as _, ser::Error as _, Deserialize, Deserializer, Serialize, Serializer};

use crate::{exact_content_ref, schema_id, CanonicalJsonValue, Result, SpecError};

/// Maximum number of executable occurrences in one expanded program.
pub const MAX_STRUCTURED_OCCURRENCES: usize = 4_096;
/// Maximum number of structural declarations in one expanded program.
pub const MAX_STRUCTURED_DECLARATIONS: usize = 8_192;
/// Maximum total fan-out lane count in one expanded program.
pub const MAX_STRUCTURED_LANES: usize = 4_096;
/// Maximum supported fan-out nesting depth.
pub const MAX_FAN_OUT_DEPTH: u8 = 2;
/// Maximum structural path depth.
pub const MAX_STRUCTURAL_PATH_DEPTH: usize = 64;
/// Maximum component objects in one certified-program closure.
pub const MAX_CERTIFIED_COMPONENT_OBJECTS: usize = 65_536;
/// Maximum producer-slot resolution depth during denormalization of expanded
/// provenance tables. Bound is independent of process stack size.
pub const MAX_PROVENANCE_RESOLUTION_DEPTH: usize = MAX_STRUCTURAL_PATH_DEPTH.saturating_mul(4);
/// Maximum JSON nodes visited while normalizing or denormalizing one structured
/// program payload. The JSON budget is separate from the component-definition
/// budget because one persisted definition contains multiple JSON nodes. Bound
/// is independent of process stack size.
pub const MAX_STRUCTURED_JSON_NODES: usize = MAX_CERTIFIED_COMPONENT_OBJECTS * 4;

const NEVER_CONTRACT_SCHEMA_NAME: &str = "mfm.kernel.never-failure-contract";
const NEVER_CONTRACT_BYTES: &[u8] =
    br#"{"kind":"never","version":"mfm.kernel.never-failure-contract.v1"}"#;
const ACCESS_FAULT_CONTRACT_SCHEMA_NAME: &str = "mfm.kernel.access-fault-contract";
const ACCESS_FAULT_CONTRACT_BYTES: &[u8] =
    br#"{"kind":"access_fault","version":"mfm.kernel.access-fault-contract.v1"}"#;

/// One dense, bounded durable-fact emission slot on a structured state.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CertifiedFactSlot {
    fact_slot_ordinal: u32,
    minimum_emissions: u32,
    maximum_emissions: u32,
    fact_descriptor_ref: ContentRef,
    subject_contract: RetainedValueContract,
    response_contract: RetainedValueContract,
}

impl CertifiedFactSlot {
    /// Constructs one exact homogeneous fact slot.
    pub fn new(
        fact_slot_ordinal: u32,
        minimum_emissions: u32,
        maximum_emissions: u32,
        fact_descriptor_ref: ContentRef,
        subject_contract: RetainedValueContract,
        response_contract: RetainedValueContract,
    ) -> Result<Self> {
        if !(1..=1_024).contains(&maximum_emissions) || minimum_emissions > maximum_emissions {
            return Err(SpecError::Invariant(
                "fact-slot emission bounds are invalid".to_owned(),
            ));
        }
        Ok(Self {
            fact_slot_ordinal,
            minimum_emissions,
            maximum_emissions,
            fact_descriptor_ref,
            subject_contract,
            response_contract,
        })
    }

    /// Returns the dense homogeneous fact-slot ordinal.
    pub const fn fact_slot_ordinal(&self) -> u32 {
        self.fact_slot_ordinal
    }

    /// Returns the minimum required emission count.
    pub const fn minimum_emissions(&self) -> u32 {
        self.minimum_emissions
    }

    /// Returns the maximum admitted emission count.
    pub const fn maximum_emissions(&self) -> u32 {
        self.maximum_emissions
    }

    /// Returns the exact fact descriptor.
    pub const fn fact_descriptor_ref(&self) -> &ContentRef {
        &self.fact_descriptor_ref
    }

    /// Returns the exact subject contract.
    pub const fn subject_contract(&self) -> &RetainedValueContract {
        &self.subject_contract
    }

    /// Returns the exact response contract.
    pub const fn response_contract(&self) -> &RetainedValueContract {
        &self.response_contract
    }
}

/// The only three executable state kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StructuredExecutionKind {
    /// Deterministic local computation.
    Pure,
    /// One bounded external read.
    Read,
    /// One bounded operation that may enter a mutable target.
    Effect,
}

/// Exact executable state contract for local work or one registered live capability.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StructuredStateExecutionContract {
    /// Deterministic local computation with no live capability.
    Pure,
    /// One bounded external read through this exact capability contract.
    Read {
        /// Registered semantic capability contract.
        capability_contract_ref: ContentRef,
    },
    /// One bounded external effect through this exact capability contract.
    Effect {
        /// Registered semantic capability contract.
        capability_contract_ref: ContentRef,
    },
}

impl StructuredStateExecutionContract {
    /// Returns the closed semantic execution kind.
    pub const fn kind(&self) -> StructuredExecutionKind {
        match self {
            Self::Pure => StructuredExecutionKind::Pure,
            Self::Read { .. } => StructuredExecutionKind::Read,
            Self::Effect { .. } => StructuredExecutionKind::Effect,
        }
    }

    /// Returns the exact live capability for a Read or Effect state.
    pub const fn capability_contract_ref(&self) -> Option<&ContentRef> {
        match self {
            Self::Pure => None,
            Self::Read {
                capability_contract_ref,
            }
            | Self::Effect {
                capability_contract_ref,
            } => Some(capability_contract_ref),
        }
    }
}

/// Exact static fallibility of a state, fragment, lane, or operation scope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StructuredFailureContract {
    /// The boundary has no failure value, codec, slot, or producer.
    Never,
    /// The boundary produces this exact inhabited retained-value contract.
    Typed {
        /// Complete producer-independent retained-value contract.
        contract: Box<RetainedValueContract>,
        /// Canonical identity of `contract`.
        contract_ref: ContentRef,
    },
}

impl StructuredFailureContract {
    /// Constructs the reserved uninhabited failure contract.
    pub const fn never() -> Self {
        Self::Never
    }

    /// Constructs an inhabited failure contract and binds its canonical identity.
    pub fn typed(contract: RetainedValueContract) -> Result<Self> {
        let canonical = contract.canonical_json()?;
        let contract_ref =
            exact_content_ref(schema_id("mfm.retained-value-contract.v1")?, &canonical)?;
        Ok(Self::Typed {
            contract: Box::new(contract),
            contract_ref,
        })
    }

    /// Returns the exact contract reference, including the kernel `Never` sentinel.
    pub fn contract_ref(&self) -> Result<ContentRef> {
        match self {
            Self::Never => never_failure_contract_ref(),
            Self::Typed { contract_ref, .. } => Ok(contract_ref.clone()),
        }
    }

    /// Returns the typed retained-value contract when the boundary is inhabited.
    pub const fn typed_contract(&self) -> Option<&RetainedValueContract> {
        match self {
            Self::Never => None,
            Self::Typed { contract, .. } => Some(contract),
        }
    }

    /// Revalidates the exact contract/reference relation.
    pub fn validate(&self) -> Result<()> {
        if let Self::Typed {
            contract,
            contract_ref,
        } = self
        {
            let canonical = contract.canonical_json()?;
            let expected =
                exact_content_ref(schema_id("mfm.retained-value-contract.v1")?, &canonical)?;
            if &expected != contract_ref {
                return Err(SpecError::Invariant(
                    "typed failure contract reference mismatch".to_owned(),
                ));
            }
        }
        Ok(())
    }
}

/// Derives the one retained-value contract used by the structured runtime for
/// a concrete MFM value type.
pub fn structured_value_contract<T: MfmValue>() -> Result<RetainedValueContract> {
    RetainedValueContract::new(
        T::schema_id()?,
        T::semantic_id()?,
        StableId::new("structured-value")
            .map_err(|error| SpecError::Identity(error.to_string()))?,
        "application/json",
        component_object_evidence_contract_ref()?,
    )
    .map_err(Into::into)
}

/// Returns the canonical identity of one retained-value contract.
pub fn retained_value_contract_ref(contract: &RetainedValueContract) -> Result<ContentRef> {
    exact_content_ref(
        schema_id("mfm.retained-value-contract.v1")?,
        &contract.canonical_json()?,
    )
}

/// Derives the structured retained-value contract identity for an MFM value
/// type without accepting caller-supplied identity material.
pub fn structured_value_contract_ref<T: MfmValue>() -> Result<ContentRef> {
    retained_value_contract_ref(&structured_value_contract::<T>()?)
}

/// Returns the one semantic adapter contract for the sealed prior-run fact scanner.
pub fn prior_run_fact_scanner_adapter_contract() -> Result<StructuredLiveComponentContract> {
    StructuredLiveComponentContract::new(
        StructuredComponentKind::Adapter,
        StableId::new("mfm.kernel.adapter/prior-run-fact-scanner")
            .map_err(|error| SpecError::Identity(error.to_string()))?,
        Vec::new(),
    )
}

/// Returns the one exact typed Read capability for prior-run fact selection.
pub fn prior_run_fact_selection_capability_contract() -> Result<StructuredLiveComponentContract> {
    StructuredLiveComponentContract::new_read_capability(
        StableId::new("mfm.kernel.capability/prior-run-fact-selection")
            .map_err(|error| SpecError::Identity(error.to_string()))?,
        structured_value_contract_ref::<mfm_facts::FactSelectionRequest>()?,
        structured_value_contract_ref::<mfm_facts::FactSelectionReadResponse>()?,
        structured_value_contract_ref::<mfm_facts::FactSelectionReadFailure>()?,
        prior_run_fact_scanner_adapter_contract()?.content_ref()?,
    )
}

/// Returns the reserved child-program identity used only as the affine
/// `proceed` placeholder inside a qualified policy expansion recipe.
///
/// It is data, not execution authority. Ordinary child substitution rejects
/// it, and policy-recipe qualification requires exactly one structural use.
pub fn policy_proceed_program_ref() -> Result<ContentRef> {
    let canonical = canonical(&"mfm.policy-proceed-placeholder.v1")?;
    structured_content_ref("mfm.policy-proceed-placeholder", &canonical)
}

/// Derives one canonical callback-free policy-recipe identity from its main
/// wrapper program and optional typed failure-post program.
pub fn policy_expansion_recipe_ref<T: Serialize>(recipe: &T) -> Result<ContentRef> {
    let canonical = canonical(recipe)?;
    structured_content_ref("mfm.policy-expansion-recipe", &canonical)
}

/// Derives the exact semantic capability requirement selected by one
/// callback-free authored expansion recipe.
pub fn capability_expansion_requirement_ref(recipe_ref: &ContentRef) -> Result<ContentRef> {
    let canonical = canonical(recipe_ref)?;
    structured_content_ref("mfm.capability-expansion-requirement", &canonical)
}

/// Returns the one content identity reserved for `FailureContract::Never`.
pub fn never_failure_contract_canonical_json() -> Result<PlainCanonicalJsonBytes> {
    PlainCanonicalJsonBytes::from_canonical_json_slice(NEVER_CONTRACT_BYTES)
        .map_err(|error| SpecError::Contract(error.to_string()))
}

/// Returns the one content identity reserved for `FailureContract::Never`.
pub fn never_failure_contract_ref() -> Result<ContentRef> {
    let canonical = never_failure_contract_canonical_json()?;
    let schema_digest =
        sha256_digest_bytes(b"mfm.structured-schema.v1:mfm.kernel.never-failure-contract:1");
    let schema = SchemaId::new(
        NEVER_CONTRACT_SCHEMA_NAME,
        "1",
        DigestAlgorithm::Sha256JcsV1,
        schema_digest,
    )?;
    exact_content_ref(schema, &canonical)
}

/// Returns canonical bytes for the one kernel-owned access-fault contract.
pub fn access_fault_contract_canonical_json() -> Result<PlainCanonicalJsonBytes> {
    PlainCanonicalJsonBytes::from_canonical_json_slice(ACCESS_FAULT_CONTRACT_BYTES)
        .map_err(|error| SpecError::Contract(error.to_string()))
}

/// Returns the one nominal contract admitted for access-integrity faults.
pub fn access_fault_contract_ref() -> Result<ContentRef> {
    let canonical = access_fault_contract_canonical_json()?;
    let schema_digest =
        sha256_digest_bytes(b"mfm.structured-schema.v1:mfm.kernel.access-fault-contract:1");
    let schema = SchemaId::new(
        ACCESS_FAULT_CONTRACT_SCHEMA_NAME,
        "1",
        DigestAlgorithm::Sha256JcsV1,
        schema_digest,
    )?;
    exact_content_ref(schema, &canonical)
}

/// Derives the semantic identity of the ordinary handler injected for one
/// protected authored call.
pub fn failure_handler_semantic_call_id(
    protected_semantic_call_id: &SemanticCallId,
) -> Result<SemanticCallId> {
    let canonical = canonical(protected_semantic_call_id)?;
    Ok(SemanticCallId::from_digest(domain_digest(
        "mfm.failure-handler.v1",
        canonical.as_bytes(),
    )))
}

/// Derives one stable semantic identity for a support state owned by a pure
/// capability or policy expansion.
pub fn expansion_support_semantic_call_id(
    protected_semantic_call_id: &SemanticCallId,
    expansion_ref: &ContentRef,
    recipe_semantic_path: &SemanticCallPath,
) -> Result<SemanticCallId> {
    let canonical = canonical(&(
        protected_semantic_call_id,
        expansion_ref,
        recipe_semantic_path,
    ))?;
    Ok(SemanticCallId::from_digest(domain_digest(
        "mfm.expansion-support-state.v1",
        canonical.as_bytes(),
    )))
}

/// One stable authored call path segment.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct SemanticPathSegment {
    /// Stable local label.
    pub label: StableId,
    /// Stable branch or lane discriminator when this segment is nested.
    pub discriminator: Option<StableId>,
}

/// Complete authored call-instance path used to derive semantic identity.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SemanticCallPath(Vec<SemanticPathSegment>);

impl SemanticCallPath {
    /// Constructs a non-empty bounded semantic path.
    pub fn new(segments: Vec<SemanticPathSegment>) -> Result<Self> {
        if segments.is_empty() || segments.len() > MAX_STRUCTURAL_PATH_DEPTH {
            return Err(SpecError::Invariant(
                "semantic call path is empty or exceeds its bound".to_owned(),
            ));
        }
        Ok(Self(segments))
    }

    /// Returns the ordered stable path segments.
    pub fn segments(&self) -> &[SemanticPathSegment] {
        &self.0
    }

    /// Derives the stable semantic call identity.
    pub fn identity(&self) -> Result<SemanticCallId> {
        let canonical = canonical(self)?;
        Ok(SemanticCallId::from_digest(domain_digest(
            "mfm.semantic-call.v1",
            canonical.as_bytes(),
        )))
    }
}

/// One normalized structural path segment.
///
/// Ordering is ordinal-first for declarations and lanes so diagnostic labels
/// never reorder certified structural identity. Labels remain diagnostics only.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StructuralPathSegment {
    /// Root lexical region of one operation.
    Root {
        /// Stable operation identity.
        operation_id: StableId,
    },
    /// Declaration in one ordered block.
    Declaration {
        /// Stable declaration label.
        label: StableId,
        /// Dense declaration ordinal.
        ordinal: u32,
    },
    /// Selected `Match` arm.
    MatchArm {
        /// Stable arm label.
        label: StableId,
        /// Canonical selector tag.
        tag: String,
    },
    /// Declared fan-out lane.
    FanOutLane {
        /// Stable lane key.
        key: StableId,
        /// Dense declaration ordinal.
        ordinal: u32,
    },
    /// Expanded child, capability, or policy fragment.
    Fragment {
        /// Stable fragment label.
        label: StableId,
        /// Exact expansion policy or child contract identity.
        expansion_ref: ContentRef,
    },
    /// Exact failure-plan region.
    FailurePlan {
        /// Stable local route label.
        label: StableId,
    },
}

impl PartialOrd for StructuralPathSegment {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for StructuralPathSegment {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        use std::cmp::Ordering;
        use StructuralPathSegment::*;
        fn kind_rank(segment: &StructuralPathSegment) -> u8 {
            match segment {
                Root { .. } => 0,
                Declaration { .. } => 1,
                MatchArm { .. } => 2,
                FanOutLane { .. } => 3,
                Fragment { .. } => 4,
                FailurePlan { .. } => 5,
            }
        }
        match kind_rank(self).cmp(&kind_rank(other)) {
            Ordering::Equal => match (self, other) {
                (
                    Root { operation_id: left },
                    Root {
                        operation_id: right,
                    },
                ) => left.cmp(right),
                (
                    Declaration {
                        ordinal: left_ord,
                        label: left_label,
                    },
                    Declaration {
                        ordinal: right_ord,
                        label: right_label,
                    },
                ) => left_ord
                    .cmp(right_ord)
                    .then_with(|| left_label.cmp(right_label)),
                (
                    MatchArm {
                        tag: left_tag,
                        label: left_label,
                    },
                    MatchArm {
                        tag: right_tag,
                        label: right_label,
                    },
                ) => left_tag
                    .cmp(right_tag)
                    .then_with(|| left_label.cmp(right_label)),
                (
                    FanOutLane {
                        ordinal: left_ord,
                        key: left_key,
                    },
                    FanOutLane {
                        ordinal: right_ord,
                        key: right_key,
                    },
                ) => left_ord
                    .cmp(right_ord)
                    .then_with(|| left_key.cmp(right_key)),
                (
                    Fragment {
                        expansion_ref: left_ref,
                        label: left_label,
                    },
                    Fragment {
                        expansion_ref: right_ref,
                        label: right_label,
                    },
                ) => left_ref
                    .cmp(right_ref)
                    .then_with(|| left_label.cmp(right_label)),
                (FailurePlan { label: left }, FailurePlan { label: right }) => left.cmp(right),
                _ => Ordering::Equal,
            },
            order => order,
        }
    }
}

/// Complete normalized structural path for a declaration or lexical region.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct StructuralPath(Vec<StructuralPathSegment>);

impl StructuralPath {
    /// Constructs a non-empty bounded structural path.
    pub fn new(segments: Vec<StructuralPathSegment>) -> Result<Self> {
        if segments.is_empty() || segments.len() > MAX_STRUCTURAL_PATH_DEPTH {
            return Err(SpecError::Invariant(
                "structural path is empty or exceeds its bound".to_owned(),
            ));
        }
        Ok(Self(segments))
    }

    /// Returns the ordered structural segments.
    pub fn segments(&self) -> &[StructuralPathSegment] {
        &self.0
    }

    /// Returns a child path extended by one segment.
    pub fn child(&self, segment: StructuralPathSegment) -> Result<Self> {
        let mut segments = self.0.clone();
        segments.push(segment);
        Self::new(segments)
    }

    /// Derives the exact executable occurrence identity for this path.
    pub fn occurrence_id(&self) -> Result<OccurrenceId> {
        let canonical = canonical(self)?;
        Ok(OccurrenceId::from_digest(domain_digest(
            "mfm.occurrence.v1",
            canonical.as_bytes(),
        )))
    }

    /// Derives the exact non-executable fragment-boundary identity for this path.
    pub fn fragment_boundary_id(&self) -> Result<FragmentBoundaryId> {
        let canonical = canonical(self)?;
        Ok(FragmentBoundaryId::from_digest(domain_digest(
            "mfm.fragment-boundary.v1",
            canonical.as_bytes(),
        )))
    }

    /// Returns the canonical content identity of this complete path.
    pub fn content_ref(&self) -> Result<ContentRef> {
        structured_content_ref("mfm.structured-path", &canonical(self)?)
    }
}

/// Nominal role of a state or fragment result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResultRole {
    /// Successful output channel.
    SuccessOutput,
    /// Typed failure channel.
    TypedFailure,
}

/// Certified producer recipe for one unresolved lexical slot.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LexicalProducer {
    /// Immutable value supplied at admission.
    AdmissionRoot {
        /// Stable admission root identity.
        root_id: StableId,
    },
    /// Unresolved authored state or child-call boundary output.
    AuthoredCallOutput {
        /// Stable semantic call identity.
        semantic_call_id: SemanticCallId,
        /// Nominal authored boundary role.
        role: ResultRole,
    },
    /// Exact state result slot.
    StateOutput {
        /// Producing occurrence.
        occurrence_id: OccurrenceId,
        /// Nominal result role.
        role: ResultRole,
    },
    /// Same-contract alias produced by the selected `Match` arm.
    ArmValue {
        /// Structural path of the selected arm.
        selected_arm_path: StructuralPath,
        /// Exact source slot.
        source: Box<LexicalSlot>,
    },
    /// Statically exhaustive selected-arm merge slot.
    MatchMerge {
        /// Exact `Match` structural path.
        match_path: StructuralPath,
        /// One normal arm-tail slot for every continuing arm in tag order.
        declaration_ordered_arm_slots: Vec<LexicalSlot>,
    },
    /// Exact merge of every current-scope typed failure exit.
    ScopeFailureMerge {
        /// Exact owned scope identity.
        scope_id: StableId,
        /// Every possible failure-exit slot in structural declaration order.
        declaration_ordered_failure_slots: Vec<LexicalSlot>,
    },
    /// Typed payload derived from an exact closed-sum selector entry.
    VariantPayload {
        /// Exact selector slot.
        selector: Box<LexicalSlot>,
        /// Canonical selected tag.
        canonical_tag: String,
        /// Canonical payload path.
        payload_path: Vec<StableId>,
    },
    /// Caller value rebound into a child fragment root.
    FragmentInput {
        /// Exact fragment boundary identity.
        boundary_id: FragmentBoundaryId,
        /// Child input-root identity.
        child_root_id: StableId,
        /// Exact dominating caller slot.
        source: Box<LexicalSlot>,
    },
    /// Exact normal or typed-failure fragment boundary result.
    FragmentBoundary {
        /// Exact fragment boundary identity.
        boundary_id: FragmentBoundaryId,
        /// Nominal boundary result role.
        role: ResultRole,
        /// Exact body-tail source slot.
        source: Box<LexicalSlot>,
    },
    /// Nominal success-or-failure result of one completed fan-out lane.
    LaneOutcome {
        /// Exact declared lane path.
        lane_path: StructuralPath,
        /// Lane normal channel, when statically reachable.
        success_slot: Option<Box<LexicalSlot>>,
        /// Exact merged lane failure channel, when statically reachable.
        failure_slot: Option<Box<LexicalSlot>>,
    },
    /// Declaration-ordered collect-all fan-out join.
    FanOutJoin {
        /// Exact fan-out group path.
        group_path: StructuralPath,
        /// One lane tail per declared lane, in declaration order.
        declaration_ordered_lane_slots: Vec<LexicalSlot>,
    },
}

/// Certified unresolved lexical value slot.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct LexicalSlot {
    /// Lexical region in which this slot is defined.
    pub lexical_path: StructuralPath,
    /// Exact nominal retained-value contract identity.
    pub contract_ref: ContentRef,
    /// Exact producer recipe.
    pub producer: LexicalProducer,
}

impl LexicalSlot {
    /// Returns the canonical identity of this slot's shallow same-program DAG definition.
    pub fn content_ref(&self) -> Result<ContentRef> {
        let mut normalizer = StructuredProgramNormalizer::default();
        Ok(normalizer.normalize_slot(self.clone())?.slot_ref)
    }

    /// Returns whether this slot is the exact typed failure output of `occurrence_id`.
    pub fn is_state_failure_of(&self, occurrence_id: &OccurrenceId) -> bool {
        matches!(
            &self.producer,
            LexicalProducer::StateOutput {
                occurrence_id: producer,
                role: ResultRole::TypedFailure,
            } if producer == occurrence_id
        )
    }

    /// Returns whether this slot's producer is an exact state success output.
    pub fn is_state_success(&self) -> bool {
        matches!(
            self.producer,
            LexicalProducer::StateOutput {
                role: ResultRole::SuccessOutput,
                ..
            }
        )
    }
}

/// One exact closed-sum tag and payload-contract table entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClosedSumVariant {
    /// Canonical variant tag.
    pub canonical_tag: String,
    /// Stable payload paths and their exact nominal contracts.
    pub payloads: Vec<ClosedSumPayload>,
}

/// One exact typed payload inside a closed-sum variant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClosedSumPayload {
    /// Canonical path within the selected variant.
    pub payload_path: Vec<StableId>,
    /// Exact nominal payload contract.
    pub contract_ref: ContentRef,
}

/// Exact canonical tag table admitted for a `Match` selector.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClosedSumContract {
    /// Canonical identity of this complete selector/tag/payload table.
    pub closed_sum_contract_ref: ContentRef,
    /// Selector retained-value contract.
    pub selector_contract_ref: ContentRef,
    /// Declaration-ordered exhaustive variant table.
    pub variants: Vec<ClosedSumVariant>,
}

#[derive(Serialize)]
struct ClosedSumContractPreimage<'a> {
    selector_contract_ref: &'a ContentRef,
    variants: &'a [ClosedSumVariant],
}

impl ClosedSumContract {
    /// Constructs one exact closed selector table and derives its identity.
    pub fn new(selector_contract_ref: ContentRef, variants: Vec<ClosedSumVariant>) -> Result<Self> {
        let canonical = canonical(&ClosedSumContractPreimage {
            selector_contract_ref: &selector_contract_ref,
            variants: &variants,
        })?;
        let contract = Self {
            closed_sum_contract_ref: structured_content_ref("mfm.closed-sum-contract", &canonical)?,
            selector_contract_ref,
            variants,
        };
        contract.validate()?;
        Ok(contract)
    }

    fn preimage(&self) -> ClosedSumContractPreimage<'_> {
        ClosedSumContractPreimage {
            selector_contract_ref: &self.selector_contract_ref,
            variants: &self.variants,
        }
    }

    /// Returns canonical bytes for the complete closed selector table.
    pub fn canonical_contract_json(&self) -> Result<PlainCanonicalJsonBytes> {
        canonical(&self.preimage())
    }

    /// Revalidates identity, tag/path uniqueness, and the non-empty closed table.
    pub fn validate(&self) -> Result<()> {
        if self.variants.is_empty() {
            return Err(SpecError::Invariant(
                "closed-sum contract has no variants".to_owned(),
            ));
        }
        let mut tags = BTreeSet::new();
        for variant in &self.variants {
            if variant.canonical_tag.is_empty() || !tags.insert(variant.canonical_tag.as_str()) {
                return Err(SpecError::Invariant(
                    "closed-sum tags are empty or duplicated".to_owned(),
                ));
            }
            let mut paths = BTreeSet::new();
            for payload in &variant.payloads {
                if payload.payload_path.is_empty() || !paths.insert(payload.payload_path.clone()) {
                    return Err(SpecError::Invariant(
                        "closed-sum payload paths are empty or duplicated".to_owned(),
                    ));
                }
            }
        }
        let expected =
            structured_content_ref("mfm.closed-sum-contract", &self.canonical_contract_json()?)?;
        if self.closed_sum_contract_ref != expected {
            return Err(SpecError::Invariant(
                "closed-sum contract identity mismatch".to_owned(),
            ));
        }
        Ok(())
    }
}

/// Closed semantic component kind admitted by a structured program.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StructuredComponentKind {
    /// Executable expanded state contract.
    State,
    /// Runtime application-protocol capability contract.
    Capability,
    /// Platform adapter contract implementing a capability.
    Adapter,
    /// Qualified signer contract required by an adapter.
    Signer,
    /// Cross-run resource-authority contract required by an adapter.
    Resource,
}

/// Exact typed dependency owned by a live semantic component contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StructuredComponentDependency {
    /// Required component kind.
    pub component_kind: StructuredComponentKind,
    /// Exact required semantic contract.
    pub contract_ref: ContentRef,
}

/// Exact refresh semantics owned by one Effect capability contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum StructuredEffectRefreshContract {
    /// Supersession before entry is not constructible for this Effect.
    NoRefresh {},
    /// Exact public evidence and stable resource-lineage contracts required
    /// to prove supersession before entry.
    Refreshable {
        /// Exact retained refresh-evidence contract.
        refresh_evidence_contract_ref: ContentRef,
        /// Exact semantic Resource contract owning the stable lineage.
        resource_lineage_contract_ref: Box<ContentRef>,
    },
}

/// Exact access-kind-specific protocol contracts for one capability.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "access_kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum StructuredCapabilityProtocolContract {
    /// One non-mutating external observation.
    Read {
        /// Immutable request contract authored before authorization.
        request_contract_ref: ContentRef,
        /// Schema-valid returned-value contract.
        returned_contract_ref: ContentRef,
        /// Reviewed state-facing safe-failure contract.
        safe_failure_contract_ref: ContentRef,
        /// Exact kernel-owned integrity-fault contract.
        access_fault_contract_ref: ContentRef,
    },
    /// One external operation that may enter a mutation boundary.
    Effect {
        /// Immutable request contract authored before authorization.
        request_contract_ref: ContentRef,
        /// Schema-valid returned-value contract.
        returned_contract_ref: ContentRef,
        /// Reviewed state-facing safe-failure contract.
        safe_failure_contract_ref: ContentRef,
        /// Exact kernel-owned integrity-fault contract.
        access_fault_contract_ref: ContentRef,
        /// Exact supersession semantics.
        refresh_contract: StructuredEffectRefreshContract,
    },
}

impl StructuredCapabilityProtocolContract {
    /// Returns the protocol's exact request contract.
    pub const fn request_contract_ref(&self) -> &ContentRef {
        match self {
            Self::Read {
                request_contract_ref,
                ..
            }
            | Self::Effect {
                request_contract_ref,
                ..
            } => request_contract_ref,
        }
    }

    /// Returns the protocol's exact returned-value contract.
    pub const fn returned_contract_ref(&self) -> &ContentRef {
        match self {
            Self::Read {
                returned_contract_ref,
                ..
            }
            | Self::Effect {
                returned_contract_ref,
                ..
            } => returned_contract_ref,
        }
    }

    /// Returns the protocol's exact safe-failure contract.
    pub const fn safe_failure_contract_ref(&self) -> &ContentRef {
        match self {
            Self::Read {
                safe_failure_contract_ref,
                ..
            }
            | Self::Effect {
                safe_failure_contract_ref,
                ..
            } => safe_failure_contract_ref,
        }
    }

    /// Returns the protocol's exact kernel access-fault contract.
    pub const fn access_fault_contract_ref(&self) -> &ContentRef {
        match self {
            Self::Read {
                access_fault_contract_ref,
                ..
            }
            | Self::Effect {
                access_fault_contract_ref,
                ..
            } => access_fault_contract_ref,
        }
    }
}

/// Canonical semantic contract for a capability, adapter, signer, or resource.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StructuredLiveComponentContract {
    /// Closed component responsibility.
    pub component_kind: StructuredComponentKind,
    /// Stable domain-owned semantic identity.
    pub semantic_component_id: StableId,
    /// Exact application-protocol contracts, present only for a capability.
    pub capability_protocol: Option<StructuredCapabilityProtocolContract>,
    /// Exact dependencies in canonical schema order.
    pub dependencies: Vec<StructuredComponentDependency>,
}

impl StructuredLiveComponentContract {
    /// Constructs and validates one live semantic component contract.
    pub fn new(
        component_kind: StructuredComponentKind,
        semantic_component_id: StableId,
        dependencies: Vec<StructuredComponentDependency>,
    ) -> Result<Self> {
        let contract = Self {
            component_kind,
            semantic_component_id,
            capability_protocol: None,
            dependencies,
        };
        contract.validate()?;
        Ok(contract)
    }

    /// Constructs one exact typed Read capability contract.
    pub fn new_read_capability(
        semantic_component_id: StableId,
        request_contract_ref: ContentRef,
        returned_contract_ref: ContentRef,
        safe_failure_contract_ref: ContentRef,
        adapter_contract_ref: ContentRef,
    ) -> Result<Self> {
        let contract = Self {
            component_kind: StructuredComponentKind::Capability,
            semantic_component_id,
            capability_protocol: Some(StructuredCapabilityProtocolContract::Read {
                request_contract_ref,
                returned_contract_ref,
                safe_failure_contract_ref,
                access_fault_contract_ref: access_fault_contract_ref()?,
            }),
            dependencies: vec![StructuredComponentDependency {
                component_kind: StructuredComponentKind::Adapter,
                contract_ref: adapter_contract_ref,
            }],
        };
        contract.validate()?;
        Ok(contract)
    }

    /// Constructs one exact typed non-refreshable Effect capability contract.
    pub fn new_effect_capability_no_refresh(
        semantic_component_id: StableId,
        request_contract_ref: ContentRef,
        returned_contract_ref: ContentRef,
        safe_failure_contract_ref: ContentRef,
        adapter_contract_ref: ContentRef,
    ) -> Result<Self> {
        Self::new_effect_capability(
            semantic_component_id,
            request_contract_ref,
            returned_contract_ref,
            safe_failure_contract_ref,
            StructuredEffectRefreshContract::NoRefresh {},
            adapter_contract_ref,
        )
    }

    /// Constructs one exact typed refreshable Effect capability contract.
    #[allow(clippy::too_many_arguments)]
    pub fn new_effect_capability_refreshable(
        semantic_component_id: StableId,
        request_contract_ref: ContentRef,
        returned_contract_ref: ContentRef,
        safe_failure_contract_ref: ContentRef,
        refresh_evidence_contract_ref: ContentRef,
        resource_lineage_contract_ref: ContentRef,
        adapter_contract_ref: ContentRef,
    ) -> Result<Self> {
        Self::new_effect_capability(
            semantic_component_id,
            request_contract_ref,
            returned_contract_ref,
            safe_failure_contract_ref,
            StructuredEffectRefreshContract::Refreshable {
                refresh_evidence_contract_ref,
                resource_lineage_contract_ref: Box::new(resource_lineage_contract_ref),
            },
            adapter_contract_ref,
        )
    }

    fn new_effect_capability(
        semantic_component_id: StableId,
        request_contract_ref: ContentRef,
        returned_contract_ref: ContentRef,
        safe_failure_contract_ref: ContentRef,
        refresh_contract: StructuredEffectRefreshContract,
        adapter_contract_ref: ContentRef,
    ) -> Result<Self> {
        let contract = Self {
            component_kind: StructuredComponentKind::Capability,
            semantic_component_id,
            capability_protocol: Some(StructuredCapabilityProtocolContract::Effect {
                request_contract_ref,
                returned_contract_ref,
                safe_failure_contract_ref,
                access_fault_contract_ref: access_fault_contract_ref()?,
                refresh_contract,
            }),
            dependencies: vec![StructuredComponentDependency {
                component_kind: StructuredComponentKind::Adapter,
                contract_ref: adapter_contract_ref,
            }],
        };
        contract.validate()?;
        Ok(contract)
    }

    /// Returns the exact canonical semantic contract identity.
    pub fn content_ref(&self) -> Result<ContentRef> {
        self.validate()?;
        structured_content_ref(self.schema_name()?, &canonical(self)?)
    }

    /// Revalidates the closed dependency graph shape owned by this component kind.
    pub fn validate(&self) -> Result<()> {
        if let Some(protocol) = &self.capability_protocol {
            if protocol.access_fault_contract_ref() != &access_fault_contract_ref()? {
                return Err(SpecError::Invariant(
                    "capability protocol does not use the kernel access-fault contract".to_owned(),
                ));
            }
        }
        let valid = match self.component_kind {
            StructuredComponentKind::State => false,
            StructuredComponentKind::Capability => {
                self.capability_protocol.is_some()
                    && matches!(
                        self.dependencies.as_slice(),
                        [StructuredComponentDependency {
                            component_kind: StructuredComponentKind::Adapter,
                            ..
                        }]
                    )
            }
            StructuredComponentKind::Adapter => matches!(
                (
                    self.capability_protocol.as_ref(),
                    self.dependencies.as_slice()
                ),
                (None, [])
                    | (
                        None,
                        [StructuredComponentDependency {
                            component_kind: StructuredComponentKind::Signer,
                            ..
                        }]
                    )
                    | (
                        None,
                        [StructuredComponentDependency {
                            component_kind: StructuredComponentKind::Resource,
                            ..
                        }]
                    )
                    | (
                        None,
                        [
                            StructuredComponentDependency {
                                component_kind: StructuredComponentKind::Signer,
                                ..
                            },
                            StructuredComponentDependency {
                                component_kind: StructuredComponentKind::Resource,
                                ..
                            },
                        ]
                    )
            ),
            StructuredComponentKind::Signer | StructuredComponentKind::Resource => {
                self.capability_protocol.is_none() && self.dependencies.is_empty()
            }
        };
        if !valid {
            return Err(SpecError::Invariant(
                "structured live component dependency shape is not canonical".to_owned(),
            ));
        }
        Ok(())
    }

    fn schema_name(&self) -> Result<&'static str> {
        match self.component_kind {
            StructuredComponentKind::Capability => Ok("mfm.structured-capability-contract"),
            StructuredComponentKind::Adapter => Ok("mfm.structured-adapter-contract"),
            StructuredComponentKind::Signer => Ok("mfm.structured-signer-contract"),
            StructuredComponentKind::Resource => Ok("mfm.structured-resource-contract"),
            StructuredComponentKind::State => Err(SpecError::Invariant(
                "state contracts use StructuredStateContract".to_owned(),
            )),
        }
    }
}

/// One logical semantic component admitted by structural first use.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StructuredComponentManifestEntry {
    /// Exact closed component kind.
    pub component_kind: StructuredComponentKind,
    /// Exact semantic contract.
    pub semantic_contract_ref: ContentRef,
}

/// Canonical state/capability/adapter/signer/resource semantic manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StateCapabilityAdapterSignerResourceManifest {
    /// First-use depth-first component order.
    pub entries: Vec<StructuredComponentManifestEntry>,
}

/// One secret-free implementation selected for an exact semantic component.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SecretFreeImplementationManifestEntry {
    /// Exact closed component kind.
    pub component_kind: StructuredComponentKind,
    /// Exact semantic contract.
    pub semantic_contract_ref: ContentRef,
    /// Exact secret-free implementation contract.
    pub implementation_contract_ref: ContentRef,
}

/// Canonical implementation selection for the complete semantic component manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SecretFreeImplementationManifest {
    /// Entries in exact semantic-manifest order.
    pub entries: Vec<SecretFreeImplementationManifestEntry>,
}

/// Canonical secret-free identity of one executable image or package.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SecretFreeExecutableIdentity {
    /// Stable whole-executable identity selected by process qualification.
    pub executable_id: StableId,
}

impl SecretFreeExecutableIdentity {
    /// Returns the exact canonical object reference.
    pub fn content_ref(&self) -> Result<ContentRef> {
        structured_content_ref("mfm.secret-free-executable-identity", &canonical(self)?)
    }
}

/// Canonical secret-free evidence that one implementation was qualified.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SecretFreeQualificationArtifact {
    /// Stable qualification artifact identity.
    pub qualification_id: StableId,
}

impl SecretFreeQualificationArtifact {
    /// Returns the exact canonical object reference.
    pub fn content_ref(&self) -> Result<ContentRef> {
        structured_content_ref("mfm.secret-free-qualification-artifact", &canonical(self)?)
    }
}

/// Canonical implementation identity bound to one exact semantic component.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SecretFreeImplementationDescriptor {
    /// Closed component responsibility.
    pub component_kind: StructuredComponentKind,
    /// Exact semantic component implemented by this code.
    pub semantic_contract_ref: ContentRef,
    /// Stable implementation identity within the executable.
    pub implementation_id: StableId,
    /// Exact whole-executable identity.
    pub executable_identity_ref: ContentRef,
    /// Exact secret-free qualification evidence.
    pub qualification_artifact_ref: ContentRef,
}

impl SecretFreeImplementationDescriptor {
    /// Returns the exact canonical implementation contract reference.
    pub fn content_ref(&self) -> Result<ContentRef> {
        structured_content_ref("mfm.secret-free-implementation", &canonical(self)?)
    }
}

/// Closed semantic handling promised for admitted safe-failure observations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum StructuredSafeFailureDispositionContract {
    /// `Pure`; no access observation can reach this state.
    NotApplicable {},
    /// Every valid admitted safe failure settles to a successful state outcome.
    AllValidEvidenceSettlesSuccess {},
    /// Valid admitted safe failures may settle to the exact typed state failure.
    MaySettleTypedFailure {},
}

/// Immutable domain fact descriptor bound to exact subject and response contracts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StructuredFactDescriptor {
    /// Content-addressed descriptor identity.
    pub descriptor_ref: ContentRef,
    /// Stable domain-owned fact kind.
    pub kind: StableId,
    /// Exact retained subject contract.
    pub subject_contract_ref: ContentRef,
    /// Exact retained response contract.
    pub response_contract_ref: ContentRef,
}

#[derive(Serialize)]
struct StructuredFactDescriptorPreimage<'a> {
    kind: &'a StableId,
    subject_contract_ref: &'a ContentRef,
    response_contract_ref: &'a ContentRef,
}

impl StructuredFactDescriptor {
    /// Constructs and content-addresses one exact fact descriptor.
    pub fn new(
        kind: StableId,
        subject_contract_ref: ContentRef,
        response_contract_ref: ContentRef,
    ) -> Result<Self> {
        let preimage = StructuredFactDescriptorPreimage {
            kind: &kind,
            subject_contract_ref: &subject_contract_ref,
            response_contract_ref: &response_contract_ref,
        };
        let descriptor_ref =
            structured_content_ref("mfm.structured-fact-descriptor", &canonical(&preimage)?)?;
        Ok(Self {
            descriptor_ref,
            kind,
            subject_contract_ref,
            response_contract_ref,
        })
    }

    /// Returns exact canonical descriptor bytes.
    pub fn canonical_json(&self) -> Result<PlainCanonicalJsonBytes> {
        canonical(&StructuredFactDescriptorPreimage {
            kind: &self.kind,
            subject_contract_ref: &self.subject_contract_ref,
            response_contract_ref: &self.response_contract_ref,
        })
    }

    /// Revalidates content identity and exact subject/response bindings.
    pub fn validate(&self) -> Result<()> {
        let expected =
            structured_content_ref("mfm.structured-fact-descriptor", &self.canonical_json()?)?;
        if expected != self.descriptor_ref {
            return Err(SpecError::Invariant(
                "structured fact descriptor identity mismatch".to_owned(),
            ));
        }
        Ok(())
    }
}

/// Exact executable state contract selected by registry qualification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StructuredStateContract {
    /// Stable semantic state contract identity.
    pub state_contract_ref: ContentRef,
    /// Stable domain-owned semantic state identity.
    pub semantic_state_id: StableId,
    /// Certified local or live-capability execution contract.
    pub execution: StructuredStateExecutionContract,
    /// Exact typed input contract.
    pub input_contract_ref: ContentRef,
    /// Exact successful output contract.
    pub output_contract_ref: ContentRef,
    /// Dense certified fact slots emitted by successful settlement.
    pub fact_slots: Vec<crate::CertifiedFactSlot>,
    /// Explicit fallibility contract.
    pub failure_contract: StructuredFailureContract,
    /// Closed semantic disposition of every admitted safe-failure observation.
    pub safe_failure_disposition: StructuredSafeFailureDispositionContract,
    /// Optional semantic capability requirement lowered before certification.
    pub capability_requirement_ref: Option<ContentRef>,
}

#[derive(Serialize)]
struct StructuredStateContractPreimage<'a> {
    semantic_state_id: &'a StableId,
    execution: &'a StructuredStateExecutionContract,
    input_contract_ref: &'a ContentRef,
    output_contract_ref: &'a ContentRef,
    fact_slots: &'a [crate::CertifiedFactSlot],
    failure_contract: &'a StructuredFailureContract,
    safe_failure_disposition: StructuredSafeFailureDispositionContract,
    capability_requirement_ref: &'a Option<ContentRef>,
}

impl StructuredStateContract {
    /// Constructs a semantic state contract and derives its content identity.
    pub fn new(
        semantic_state_id: StableId,
        execution: StructuredStateExecutionContract,
        input_contract_ref: ContentRef,
        output_contract_ref: ContentRef,
        failure_contract: StructuredFailureContract,
        safe_failure_disposition: StructuredSafeFailureDispositionContract,
        capability_requirement_ref: Option<ContentRef>,
    ) -> Result<Self> {
        Self::new_with_fact_slots(
            semantic_state_id,
            execution,
            input_contract_ref,
            output_contract_ref,
            failure_contract,
            safe_failure_disposition,
            capability_requirement_ref,
            Vec::new(),
        )
    }

    /// Constructs a semantic state contract with exact bounded fact slots.
    #[allow(clippy::too_many_arguments)]
    pub fn new_with_fact_slots(
        semantic_state_id: StableId,
        execution: StructuredStateExecutionContract,
        input_contract_ref: ContentRef,
        output_contract_ref: ContentRef,
        failure_contract: StructuredFailureContract,
        safe_failure_disposition: StructuredSafeFailureDispositionContract,
        capability_requirement_ref: Option<ContentRef>,
        fact_slots: Vec<crate::CertifiedFactSlot>,
    ) -> Result<Self> {
        let canonical = canonical(&StructuredStateContractPreimage {
            semantic_state_id: &semantic_state_id,
            execution: &execution,
            input_contract_ref: &input_contract_ref,
            output_contract_ref: &output_contract_ref,
            fact_slots: &fact_slots,
            failure_contract: &failure_contract,
            safe_failure_disposition,
            capability_requirement_ref: &capability_requirement_ref,
        })?;
        let contract = Self {
            state_contract_ref: structured_content_ref(
                "mfm.structured-state-contract",
                &canonical,
            )?,
            semantic_state_id,
            execution,
            input_contract_ref,
            output_contract_ref,
            fact_slots,
            failure_contract,
            safe_failure_disposition,
            capability_requirement_ref,
        };
        contract.validate()?;
        Ok(contract)
    }

    fn preimage(&self) -> StructuredStateContractPreimage<'_> {
        StructuredStateContractPreimage {
            semantic_state_id: &self.semantic_state_id,
            execution: &self.execution,
            input_contract_ref: &self.input_contract_ref,
            output_contract_ref: &self.output_contract_ref,
            fact_slots: &self.fact_slots,
            failure_contract: &self.failure_contract,
            safe_failure_disposition: self.safe_failure_disposition,
            capability_requirement_ref: &self.capability_requirement_ref,
        }
    }

    /// Returns exact canonical semantic state-contract bytes.
    pub fn canonical_contract_json(&self) -> Result<PlainCanonicalJsonBytes> {
        canonical(&self.preimage())
    }

    fn derived_state_contract_ref(&self) -> Result<ContentRef> {
        structured_content_ref(
            "mfm.structured-state-contract",
            &self.canonical_contract_json()?,
        )
    }

    /// Revalidates the complete contract relation.
    pub fn validate(&self) -> Result<()> {
        self.failure_contract.validate()?;
        let mut total_facts = 0_u64;
        for (ordinal, slot) in self.fact_slots.iter().enumerate() {
            if usize::try_from(slot.fact_slot_ordinal()).ok() != Some(ordinal)
                || !(1..=1024).contains(&slot.maximum_emissions())
                || slot.minimum_emissions() > slot.maximum_emissions()
                || slot.subject_contract().validated().is_err()
                || slot.response_contract().validated().is_err()
            {
                return Err(SpecError::Invariant(
                    "structured state fact slots are not exact dense bounded contracts".to_owned(),
                ));
            }
            total_facts = total_facts
                .checked_add(u64::from(slot.maximum_emissions()))
                .ok_or_else(|| {
                    SpecError::Invariant("structured state fact bound overflowed".to_owned())
                })?;
        }
        if total_facts > 4096 {
            return Err(SpecError::Invariant(
                "structured state emits more than 4096 facts".to_owned(),
            ));
        }
        let disposition_is_exact = matches!(
            (
                self.execution.kind(),
                &self.failure_contract,
                self.safe_failure_disposition,
            ),
            (
                StructuredExecutionKind::Pure,
                _,
                StructuredSafeFailureDispositionContract::NotApplicable {},
            ) | (
                StructuredExecutionKind::Read | StructuredExecutionKind::Effect,
                StructuredFailureContract::Never,
                StructuredSafeFailureDispositionContract::AllValidEvidenceSettlesSuccess {},
            ) | (
                StructuredExecutionKind::Read | StructuredExecutionKind::Effect,
                StructuredFailureContract::Typed { .. },
                StructuredSafeFailureDispositionContract::AllValidEvidenceSettlesSuccess {}
                    | StructuredSafeFailureDispositionContract::MaySettleTypedFailure {},
            )
        );
        if !disposition_is_exact {
            return Err(SpecError::Invariant(
                "state execution, failure contract, and safe-failure disposition mismatch"
                    .to_owned(),
            ));
        }
        if self.state_contract_ref != self.derived_state_contract_ref()? {
            return Err(SpecError::Invariant(
                "structured state contract identity mismatch".to_owned(),
            ));
        }
        Ok(())
    }
}

/// Author-selected handling for one state or child boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AuthoredFailureDirective {
    /// Exact `Never` boundary with no plan.
    NoFailure,
    /// Select the exact matching lexical default mapper.
    Default,
    /// Select one registered custom pure handler and closed route contract.
    Custom {
        /// Handler state contract.
        handler_state_contract_ref: Box<ContentRef>,
        /// Exact closed route table returned by the handler.
        route_contract: Box<ClosedSumContract>,
        /// Exhaustive authored continuation bodies in canonical tag order.
        arms: Vec<AuthoredMatchArm>,
    },
}

/// One exact lexical default-mapper registration owned by a scope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FailureMapperRegistration {
    /// Exact source failure contract.
    pub source_failure_contract_ref: ContentRef,
    /// Registered `Pure + Never` mapper state contract.
    pub mapper_state_contract_ref: ContentRef,
    /// Exact one-tag default propagation route contract.
    pub route_contract: ClosedSumContract,
}

/// Lexical scope owning one failure contract and exact mapper table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FailureScope {
    /// Stable scope identity.
    pub scope_id: StableId,
    /// Declared scope failure contract.
    pub failure_contract: StructuredFailureContract,
    /// Exact finite default mapper table; empty for `Never`.
    pub default_mappers: Vec<FailureMapperRegistration>,
}

/// Ownership of the exact lexical failure map visible to a block.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FailureScopeBinding {
    /// Operation, fragment, or fan-out lane owns this exact scope and map.
    Owns {
        /// Complete owned scope.
        scope: FailureScope,
    },
    /// Match arm or handler route inherits one exact enclosing scope.
    Inherits {
        /// Exact enclosing scope identity.
        scope_id: StableId,
        /// Exact enclosing scope failure contract.
        failure_contract: StructuredFailureContract,
    },
}

impl FailureScopeBinding {
    /// Returns the exact visible scope identity.
    pub const fn scope_id(&self) -> &StableId {
        match self {
            Self::Owns { scope } => &scope.scope_id,
            Self::Inherits { scope_id, .. } => scope_id,
        }
    }

    /// Returns the exact visible failure contract.
    pub const fn failure_contract(&self) -> &StructuredFailureContract {
        match self {
            Self::Owns { scope } => &scope.failure_contract,
            Self::Inherits {
                failure_contract, ..
            } => failure_contract,
        }
    }
}

/// Structural tail of one lexical block.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "slot", rename_all = "snake_case")]
pub enum BlockTail {
    /// Normal value channel.
    Normal(LexicalSlot),
    /// Exact current-scope typed failure channel.
    ScopeFailure(LexicalSlot),
}

/// Authored executable state call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthoredStateCall {
    /// Stable local declaration label.
    pub label: StableId,
    /// Complete authored call path.
    pub semantic_path: SemanticCallPath,
    /// Derived stable semantic identity.
    pub semantic_call_id: SemanticCallId,
    /// Exact semantic state contract.
    pub contract: StructuredStateContract,
    /// Declaration-ordered dominating inputs.
    pub inputs: Vec<LexicalSlot>,
    /// Author-selected failure disposition.
    pub failure_directive: AuthoredFailureDirective,
    /// Unresolved successful output slot.
    pub output_slot: LexicalSlot,
}

/// Authored child-operation call removed by pure expansion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthoredOperationCall {
    /// Stable local declaration label.
    pub label: StableId,
    /// Complete authored call path.
    pub semantic_path: SemanticCallPath,
    /// Derived stable semantic identity.
    pub semantic_call_id: SemanticCallId,
    /// Exact child authored-program reference.
    pub child_program_ref: ContentRef,
    /// One caller slot for every declared child input root.
    pub input_bindings: Vec<FragmentInputBinding>,
    /// Exact child success contract.
    pub output_contract_ref: ContentRef,
    /// Exact child failure contract.
    pub failure_contract: StructuredFailureContract,
    /// Call-site failure disposition.
    pub failure_directive: AuthoredFailureDirective,
    /// Unresolved child-boundary output slot.
    pub output_slot: LexicalSlot,
}

/// One caller-to-child input-root binding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FragmentInputBinding {
    /// Stable child input-root identity.
    pub child_root_id: StableId,
    /// Exact child input contract.
    pub child_contract_ref: ContentRef,
    /// Exact dominating caller slot.
    pub caller_slot: LexicalSlot,
}

/// One exhaustive authored `Match` arm.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthoredMatchArm {
    /// Canonical selector tag.
    pub canonical_tag: String,
    /// Stable arm label.
    pub label: StableId,
    /// Declaration-ordered arm body.
    pub body: AuthoredBlock,
}

/// One authored exhaustive `Match` binding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthoredMatch {
    /// Stable local declaration label.
    pub label: StableId,
    /// Exact selector slot.
    pub selector: LexicalSlot,
    /// Exact registered closed-sum tag table.
    pub selector_contract: ClosedSumContract,
    /// One arm for every canonical tag.
    pub arms: Vec<AuthoredMatchArm>,
    /// Same-contract merged arm value.
    pub output_slot: LexicalSlot,
}

/// One authored fan-out lane.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthoredFanOutLane {
    /// Stable lane key.
    pub key: StableId,
    /// Dense declaration ordinal derived by the builder.
    pub declaration_ordinal: u32,
    /// Lane-owned lexical block.
    pub body: AuthoredBlock,
}

/// One authored non-empty collect-all fan-out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthoredFanOut {
    /// Stable local group label.
    pub label: StableId,
    /// Homogeneous lane success contract.
    pub lane_output_contract_ref: ContentRef,
    /// Homogeneous lane failure contract.
    pub lane_failure_contract: StructuredFailureContract,
    /// Declared lanes in semantic result order.
    pub lanes: Vec<AuthoredFanOutLane>,
    /// Unresolved declaration-ordered join slot.
    pub output_slot: LexicalSlot,
}

/// Author-visible structural declarations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "declaration", rename_all = "snake_case")]
pub enum AuthoredDeclaration {
    /// One authored executable state call.
    State(Box<AuthoredStateCall>),
    /// Child-composition sugar removed during expansion.
    OperationCall(Box<AuthoredOperationCall>),
    /// Exhaustive typed conditional control.
    Match(Box<AuthoredMatch>),
    /// Bounded collect-all `Pure | Read` fan-out.
    FanOut(Box<AuthoredFanOut>),
}

/// One declaration-ordered authored lexical block.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthoredBlock {
    /// Exact lexical region path.
    pub path: StructuralPath,
    /// Owned or exact inherited lexical failure scope.
    pub failure_scope: FailureScopeBinding,
    /// Declarations in semantic execution order.
    pub declarations: Vec<AuthoredDeclaration>,
    /// One exact normal or scope-failure tail expression.
    pub tail: BlockTail,
}

/// Complete canonical authored operation program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthoredStructuredProgram {
    /// Stable operation identity.
    pub operation_id: StableId,
    /// Declaration-ordered admission input roots.
    pub input_roots: Vec<LexicalSlot>,
    /// Exact successful root value contract.
    pub output_contract_ref: ContentRef,
    /// Exact root failure contract.
    pub failure_contract: StructuredFailureContract,
    /// Root lexical block.
    pub root: AuthoredBlock,
}

impl AuthoredStructuredProgram {
    /// Returns exact canonical bytes.
    pub fn canonical_json(&self) -> Result<PlainCanonicalJsonBytes> {
        canonical(self)
    }

    /// Returns the canonical authored-program reference.
    pub fn content_ref(&self) -> Result<ContentRef> {
        structured_content_ref("mfm.authored-structured-program", &self.canonical_json()?)
    }
}

/// Certified no-failure marker for an exact `Never` boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoFailureBoundary {
    /// Exact kernel sentinel contract identity.
    pub never_contract_ref: ContentRef,
}

/// Exact certified failure-plan identity preimage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FailurePlanIdentity {
    /// Exact failing binding semantic identity.
    pub source_semantic_call_id: SemanticCallId,
    /// Exact typed failure source slot.
    pub source_slot: LexicalSlot,
    /// Exact plan structural path.
    pub plan_path: StructuralPath,
}

impl FailurePlanIdentity {
    /// Derives the plan identity from the complete canonical preimage.
    pub fn derive(&self) -> Result<FailurePlanId> {
        let source_slot_ref = self.source_slot.content_ref()?;
        let plan_path_ref = self.plan_path.content_ref()?;
        let canonical = canonical(&FailurePlanIdentityPreimage {
            source_semantic_call_id: &self.source_semantic_call_id,
            source_slot_ref: &source_slot_ref,
            plan_path_ref: &plan_path_ref,
        })?;
        Ok(FailurePlanId::from_digest(domain_digest(
            "mfm.failure-plan.v1",
            canonical.as_bytes(),
        )))
    }
}

#[derive(Serialize)]
struct FailurePlanIdentityPreimage<'a> {
    source_semantic_call_id: &'a SemanticCallId,
    source_slot_ref: &'a ContentRef,
    plan_path_ref: &'a ContentRef,
}

/// One pure mapper link in an affine typed-failure propagation chain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FailureMappingLink {
    /// Exact failure plan that owns this affine mapper link.
    pub plan_id: FailurePlanId,
    /// Ordinary certified `Pure + Never` mapper occurrence.
    pub mapper: Box<ExpandedStateBinding>,
    /// Exact mapper input slot.
    pub input_slot: LexicalSlot,
    /// Exact mapper output slot.
    pub output_slot: LexicalSlot,
}

/// Closed handler continuation selected only by the handler's exact output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HandlerContinuation {
    /// Lexical default with exactly one `Propagate` payload and no intervening state.
    DefaultPropagation {
        /// Exact handler output selector.
        handler_output_slot: LexicalSlot,
        /// Exact registered one-tag default route table.
        route_contract: ClosedSumContract,
        /// Exact payload slot derived from that selector.
        payload_slot: Box<LexicalSlot>,
        /// Exact enclosing scope-failure tail.
        failure_tail: Box<LexicalSlot>,
    },
    /// Explicit closed handler route with exhaustive branch bodies.
    CustomRecovery {
        /// Exact handler output selector.
        handler_output_slot: LexicalSlot,
        /// Exact registered closed route table.
        route_contract: ClosedSumContract,
        /// Exhaustive continuation arms.
        arms: Vec<ExpandedMatchArm>,
    },
}

/// Certified failure plan selected by one exact typed-failure slot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FailurePlan {
    /// Exact mapper/handler execution ending in recovery or current-scope failure.
    Handled {
        /// Derived plan identity.
        plan_id: FailurePlanId,
        /// Exact structural region owned by this plan.
        plan_path: StructuralPath,
        /// Exact typed source slot.
        source_slot: LexicalSlot,
        /// Constrained ordinary bindings that run before the designated handler.
        before_handler: Box<ExpandedBlock>,
        /// Designated ordinary `Pure + Never` handler occurrence.
        handler: Box<ExpandedStateBinding>,
        /// Exhaustive handler-output continuation.
        continuation: Box<HandlerContinuation>,
    },
    /// Exact affine propagation through one enclosing fragment boundary.
    Propagate {
        /// Derived plan identity.
        plan_id: FailurePlanId,
        /// Exact structural region owned by this plan.
        plan_path: StructuralPath,
        /// Exact typed source slot.
        source_slot: LexicalSlot,
        /// Ordinary failure-post structure that must complete before the
        /// protected failure may cross its enclosing fragment boundary.
        before_boundary: Box<ExpandedBlock>,
        /// Pure affine mapper chain, possibly empty for exact-contract identity.
        mapping_chain: Vec<FailureMappingLink>,
        /// Exact enclosing fragment boundary identity.
        boundary_id: FragmentBoundaryId,
        /// Exact typed fragment-failure boundary slot.
        boundary_slot: Box<LexicalSlot>,
    },
}

/// Certified failure boundary for one state or fragment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CertifiedFailureBoundary {
    /// Exact `Never` boundary with no slot or plan.
    NoFailure(NoFailureBoundary),
    /// Exact inhabited failure slot with one sealed plan.
    Typed {
        /// Exact failure contract.
        failure_contract: Box<StructuredFailureContract>,
        /// Exact failure producer slot.
        source_slot: LexicalSlot,
        /// Sole certified continuation.
        plan: Box<FailurePlan>,
    },
}

/// One expanded executable state binding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExpandedStateBinding {
    /// Stable semantic call identity preserved through wrapping.
    pub semantic_call_id: SemanticCallId,
    /// Exact normalized structural occurrence identity.
    pub occurrence_id: OccurrenceId,
    /// Exact normalized structural path.
    pub occurrence_path: StructuralPath,
    /// Stable local semantic label.
    pub label: StableId,
    /// Exact qualified state contract.
    pub contract: StructuredStateContract,
    /// Declaration-ordered exact input slots.
    pub inputs: Vec<LexicalSlot>,
    /// Exact successful output slot.
    pub output_slot: LexicalSlot,
    /// Exact `Never` marker or sole typed failure plan.
    pub failure_boundary: CertifiedFailureBoundary,
}

/// One expanded exhaustive `Match` arm.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExpandedMatchArm {
    /// Canonical selector tag.
    pub canonical_tag: String,
    /// Stable arm label.
    pub label: StableId,
    /// Exact selected-arm path.
    pub path: StructuralPath,
    /// Declaration-ordered arm body.
    pub body: ExpandedBlock,
}

/// Expanded callback-free `Match` binding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExpandedMatch {
    /// Stable local declaration label.
    pub label: StableId,
    /// Exact structural path.
    pub path: StructuralPath,
    /// Exact selector slot.
    pub selector: LexicalSlot,
    /// Exact registered tag/payload table.
    pub selector_contract: ClosedSumContract,
    /// Exhaustive arms in registered tag order.
    pub arms: Vec<ExpandedMatchArm>,
    /// Same-contract selected-arm merge slot.
    pub output_slot: LexicalSlot,
}

/// One expanded fan-out lane.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExpandedFanOutLane {
    /// Stable lane key.
    pub key: StableId,
    /// Dense declaration ordinal.
    pub declaration_ordinal: u32,
    /// Exact lane path.
    pub path: StructuralPath,
    /// Lane-owned lexical block.
    pub body: ExpandedBlock,
    /// Exact nominal lane outcome consumed by the group join.
    pub outcome_slot: LexicalSlot,
}

/// Expanded callback-free fan-out binding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExpandedFanOut {
    /// Stable local group label.
    pub label: StableId,
    /// Exact group path.
    pub path: StructuralPath,
    /// Homogeneous lane success contract.
    pub lane_output_contract_ref: ContentRef,
    /// Homogeneous lane failure contract.
    pub lane_failure_contract: StructuredFailureContract,
    /// Non-empty lanes in semantic join order.
    pub lanes: Vec<ExpandedFanOutLane>,
    /// Exact declaration-ordered structural join slot.
    pub output_slot: LexicalSlot,
}

/// One expanded child/capability/policy fragment boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExpandedFragment {
    /// Stable semantic call identity preserved at the external boundary.
    pub semantic_call_id: SemanticCallId,
    /// Stable local fragment label.
    pub label: StableId,
    /// Exact structural path and boundary identity.
    pub path: StructuralPath,
    /// Exact structural boundary identity.
    pub boundary_id: FragmentBoundaryId,
    /// One binding for every declared fragment input root.
    pub input_bindings: Vec<FragmentInputBinding>,
    /// Declaration-ordered expanded body.
    pub body: ExpandedBlock,
    /// Exact structural success boundary slot.
    pub success_slot: LexicalSlot,
    /// Exact `Never` marker or sole typed boundary failure plan.
    pub failure_boundary: CertifiedFailureBoundary,
}

/// Fully expanded structural declarations; only `State` is executable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "declaration", rename_all = "snake_case")]
pub enum ExpandedDeclaration {
    /// One executable state occurrence.
    State(Box<ExpandedStateBinding>),
    /// Exhaustive callback-free conditional control.
    Match(Box<ExpandedMatch>),
    /// Bounded callback-free collect-all fan-out.
    FanOut(Box<ExpandedFanOut>),
    /// Expanded child, capability, or policy fragment.
    Fragment(Box<ExpandedFragment>),
}

/// One fully expanded declaration-ordered lexical block.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExpandedBlock {
    /// Exact lexical region path.
    pub path: StructuralPath,
    /// Exact owned or inherited lexical failure scope.
    pub failure_scope: FailureScopeBinding,
    /// Declarations in semantic execution order.
    pub declarations: Vec<ExpandedDeclaration>,
    /// Every exact typed failure exit from this scope in structural order.
    pub failure_exits: Vec<LexicalSlot>,
    /// Exact normal or current-scope failure tail.
    pub tail: BlockTail,
}

/// Complete expanded operation program containing no authored child call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpandedStructuredProgram {
    /// Stable operation identity.
    pub operation_id: StableId,
    /// Declaration-ordered admission input roots.
    pub input_roots: Vec<LexicalSlot>,
    /// Exact successful root contract.
    pub output_contract_ref: ContentRef,
    /// Exact root failure contract.
    pub failure_contract: StructuredFailureContract,
    /// Fully expanded root block.
    pub root: ExpandedBlock,
}

impl ExpandedStructuredProgram {
    /// Returns exact canonical bytes.
    pub fn canonical_json(&self) -> Result<PlainCanonicalJsonBytes> {
        canonical(self)
    }

    /// Returns the canonical expanded-program reference.
    pub fn content_ref(&self) -> Result<ContentRef> {
        structured_content_ref("mfm.expanded-structured-program", &self.canonical_json()?)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExpandedStructuralPathDefinition {
    path_ref: ContentRef,
    path: StructuralPath,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExpandedLexicalSlotDefinition {
    slot_ref: ContentRef,
    slot: ExpandedLexicalSlot,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExpandedLexicalSlot {
    lexical_path_ref: ContentRef,
    contract_ref: ContentRef,
    producer: ExpandedLexicalProducer,
}

impl ExpandedLexicalSlot {
    fn content_ref(&self) -> Result<ContentRef> {
        structured_content_ref("mfm.structured-lexical-slot", &canonical(self)?)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum ExpandedLexicalProducer {
    AdmissionRoot {
        root_id: StableId,
    },
    AuthoredCallOutput {
        semantic_call_id: SemanticCallId,
        role: ResultRole,
    },
    StateOutput {
        occurrence_id: OccurrenceId,
        role: ResultRole,
    },
    ArmValue {
        selected_arm_path: ExpandedPathRef,
        source: ExpandedSlotRef,
    },
    MatchMerge {
        match_path: ExpandedPathRef,
        declaration_ordered_arm_slots: Vec<ExpandedSlotRef>,
    },
    ScopeFailureMerge {
        scope_id: StableId,
        declaration_ordered_failure_slots: Vec<ExpandedSlotRef>,
    },
    VariantPayload {
        selector: ExpandedSlotRef,
        canonical_tag: String,
        payload_path: Vec<StableId>,
    },
    FragmentInput {
        boundary_id: FragmentBoundaryId,
        child_root_id: StableId,
        source: ExpandedSlotRef,
    },
    FragmentBoundary {
        boundary_id: FragmentBoundaryId,
        role: ResultRole,
        source: ExpandedSlotRef,
    },
    LaneOutcome {
        lane_path: ExpandedPathRef,
        success_slot: Option<ExpandedSlotRef>,
        failure_slot: Option<ExpandedSlotRef>,
    },
    FanOutJoin {
        group_path: ExpandedPathRef,
        declaration_ordered_lane_slots: Vec<ExpandedSlotRef>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExpandedPathRef {
    path_ref: ContentRef,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExpandedSlotRef {
    slot_ref: ContentRef,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct NormalizedStructuredProgram {
    operation_id: StableId,
    structural_paths: Vec<ExpandedStructuralPathDefinition>,
    lexical_slots: Vec<ExpandedLexicalSlotDefinition>,
    input_root_slot_refs: Vec<ExpandedSlotRef>,
    output_contract_ref: ContentRef,
    failure_contract: StructuredFailureContract,
    root: serde_json::Value,
}

#[derive(Default)]
struct StructuredProgramNormalizer {
    structural_paths: BTreeMap<ContentRef, StructuralPath>,
    lexical_slots: BTreeMap<ContentRef, ExpandedLexicalSlot>,
}

impl StructuredProgramNormalizer {
    fn normalize_path(&mut self, path: StructuralPath) -> Result<ExpandedPathRef> {
        let path_ref = path.content_ref()?;
        insert_normalized_definition(
            &mut self.structural_paths,
            path_ref.clone(),
            path,
            "expanded structural path",
        )?;
        Ok(ExpandedPathRef { path_ref })
    }

    fn normalize_slot(&mut self, slot: LexicalSlot) -> Result<ExpandedSlotRef> {
        let lexical_path_ref = self.normalize_path(slot.lexical_path)?.path_ref;
        let producer = self.normalize_producer(slot.producer)?;
        let slot = ExpandedLexicalSlot {
            lexical_path_ref,
            contract_ref: slot.contract_ref,
            producer,
        };
        let slot_ref = slot.content_ref()?;
        insert_normalized_definition(
            &mut self.lexical_slots,
            slot_ref.clone(),
            slot,
            "expanded lexical slot",
        )?;
        Ok(ExpandedSlotRef { slot_ref })
    }

    fn normalize_producer(&mut self, producer: LexicalProducer) -> Result<ExpandedLexicalProducer> {
        Ok(match producer {
            LexicalProducer::AdmissionRoot { root_id } => {
                ExpandedLexicalProducer::AdmissionRoot { root_id }
            }
            LexicalProducer::AuthoredCallOutput {
                semantic_call_id,
                role,
            } => ExpandedLexicalProducer::AuthoredCallOutput {
                semantic_call_id,
                role,
            },
            LexicalProducer::StateOutput {
                occurrence_id,
                role,
            } => ExpandedLexicalProducer::StateOutput {
                occurrence_id,
                role,
            },
            LexicalProducer::ArmValue {
                selected_arm_path,
                source,
            } => ExpandedLexicalProducer::ArmValue {
                selected_arm_path: self.normalize_path(selected_arm_path)?,
                source: self.normalize_slot(*source)?,
            },
            LexicalProducer::MatchMerge {
                match_path,
                declaration_ordered_arm_slots,
            } => ExpandedLexicalProducer::MatchMerge {
                match_path: self.normalize_path(match_path)?,
                declaration_ordered_arm_slots: declaration_ordered_arm_slots
                    .into_iter()
                    .map(|slot| self.normalize_slot(slot))
                    .collect::<Result<Vec<_>>>()?,
            },
            LexicalProducer::ScopeFailureMerge {
                scope_id,
                declaration_ordered_failure_slots,
            } => ExpandedLexicalProducer::ScopeFailureMerge {
                scope_id,
                declaration_ordered_failure_slots: declaration_ordered_failure_slots
                    .into_iter()
                    .map(|slot| self.normalize_slot(slot))
                    .collect::<Result<Vec<_>>>()?,
            },
            LexicalProducer::VariantPayload {
                selector,
                canonical_tag,
                payload_path,
            } => ExpandedLexicalProducer::VariantPayload {
                selector: self.normalize_slot(*selector)?,
                canonical_tag,
                payload_path,
            },
            LexicalProducer::FragmentInput {
                boundary_id,
                child_root_id,
                source,
            } => ExpandedLexicalProducer::FragmentInput {
                boundary_id,
                child_root_id,
                source: self.normalize_slot(*source)?,
            },
            LexicalProducer::FragmentBoundary {
                boundary_id,
                role,
                source,
            } => ExpandedLexicalProducer::FragmentBoundary {
                boundary_id,
                role,
                source: self.normalize_slot(*source)?,
            },
            LexicalProducer::LaneOutcome {
                lane_path,
                success_slot,
                failure_slot,
            } => ExpandedLexicalProducer::LaneOutcome {
                lane_path: self.normalize_path(lane_path)?,
                success_slot: success_slot
                    .map(|slot| self.normalize_slot(*slot))
                    .transpose()?,
                failure_slot: failure_slot
                    .map(|slot| self.normalize_slot(*slot))
                    .transpose()?,
            },
            LexicalProducer::FanOutJoin {
                group_path,
                declaration_ordered_lane_slots,
            } => ExpandedLexicalProducer::FanOutJoin {
                group_path: self.normalize_path(group_path)?,
                declaration_ordered_lane_slots: declaration_ordered_lane_slots
                    .into_iter()
                    .map(|slot| self.normalize_slot(slot))
                    .collect::<Result<Vec<_>>>()?,
            },
        })
    }

    fn normalize_value(
        &mut self,
        root: serde_json::Value,
        root_field_name: Option<&str>,
    ) -> Result<serde_json::Value> {
        // Explicit work stack: hostile deep JSON fails with a typed bound error
        // rather than exhausting the process stack.
        enum Frame {
            Start {
                value: serde_json::Value,
                field_name: Option<String>,
            },
            ArrayCollect {
                total: usize,
            },
            ObjectCollect {
                keys: Vec<String>,
            },
        }
        let mut stack = vec![Frame::Start {
            value: root,
            field_name: root_field_name.map(str::to_owned),
        }];
        let mut completed = Vec::new();
        let mut nodes = 0usize;
        while let Some(frame) = stack.pop() {
            match frame {
                Frame::Start { value, field_name } => {
                    nodes = nodes.saturating_add(1);
                    if nodes > MAX_STRUCTURED_JSON_NODES {
                        return Err(SpecError::Invariant(
                            "structured program JSON node budget exceeded".to_owned(),
                        ));
                    }
                    if is_inline_lexical_slot(&value) {
                        let slot = serde_json::from_value(value)
                            .map_err(|error| SpecError::Invariant(error.to_string()))?;
                        completed.push(
                            serde_json::to_value(self.normalize_slot(slot)?)
                                .map_err(|error| SpecError::Invariant(error.to_string()))?,
                        );
                        continue;
                    }
                    if field_name.as_deref().is_some_and(is_structural_path_field) {
                        let path = serde_json::from_value(value)
                            .map_err(|error| SpecError::Invariant(error.to_string()))?;
                        completed.push(
                            serde_json::to_value(self.normalize_path(path)?)
                                .map_err(|error| SpecError::Invariant(error.to_string()))?,
                        );
                        continue;
                    }
                    match value {
                        serde_json::Value::Array(values) => {
                            let total = values.len();
                            stack.push(Frame::ArrayCollect { total });
                            for value in values.into_iter().rev() {
                                stack.push(Frame::Start {
                                    value,
                                    field_name: None,
                                });
                            }
                        }
                        serde_json::Value::Object(values) => {
                            let mut keys = Vec::with_capacity(values.len());
                            let mut items = Vec::with_capacity(values.len());
                            for (key, value) in values {
                                keys.push(key.clone());
                                items.push((key, value));
                            }
                            stack.push(Frame::ObjectCollect { keys });
                            for (key, value) in items.into_iter().rev() {
                                stack.push(Frame::Start {
                                    value,
                                    field_name: Some(key),
                                });
                            }
                        }
                        scalar => completed.push(scalar),
                    }
                }
                Frame::ArrayCollect { total } => {
                    let mut items = Vec::with_capacity(total);
                    for _ in 0..total {
                        items.push(completed.pop().ok_or_else(|| {
                            SpecError::Invariant(
                                "structured program JSON array frame underfilled".to_owned(),
                            )
                        })?);
                    }
                    items.reverse();
                    completed.push(serde_json::Value::Array(items));
                }
                Frame::ObjectCollect { keys } => {
                    let mut object = serde_json::Map::with_capacity(keys.len());
                    for key in keys.into_iter().rev() {
                        let value = completed.pop().ok_or_else(|| {
                            SpecError::Invariant(
                                "structured program JSON object frame underfilled".to_owned(),
                            )
                        })?;
                        object.insert(key, value);
                    }
                    completed.push(serde_json::Value::Object(object));
                }
            }
        }
        completed.pop().ok_or_else(|| {
            SpecError::Invariant(
                "structured program JSON normalization produced no root".to_owned(),
            )
        })
    }

    fn finish(
        self,
    ) -> (
        Vec<ExpandedStructuralPathDefinition>,
        Vec<ExpandedLexicalSlotDefinition>,
    ) {
        (
            self.structural_paths
                .into_iter()
                .map(|(path_ref, path)| ExpandedStructuralPathDefinition { path_ref, path })
                .collect(),
            self.lexical_slots
                .into_iter()
                .map(|(slot_ref, slot)| ExpandedLexicalSlotDefinition { slot_ref, slot })
                .collect(),
        )
    }
}

fn expanded_producer_child_slots(producer: &ExpandedLexicalProducer) -> Vec<ExpandedSlotRef> {
    match producer {
        ExpandedLexicalProducer::AdmissionRoot { .. }
        | ExpandedLexicalProducer::AuthoredCallOutput { .. }
        | ExpandedLexicalProducer::StateOutput { .. } => Vec::new(),
        ExpandedLexicalProducer::ArmValue { source, .. } => vec![source.clone()],
        ExpandedLexicalProducer::MatchMerge {
            declaration_ordered_arm_slots,
            ..
        } => declaration_ordered_arm_slots.clone(),
        ExpandedLexicalProducer::ScopeFailureMerge {
            declaration_ordered_failure_slots,
            ..
        } => declaration_ordered_failure_slots.clone(),
        ExpandedLexicalProducer::VariantPayload { selector, .. } => vec![selector.clone()],
        ExpandedLexicalProducer::FragmentInput { source, .. }
        | ExpandedLexicalProducer::FragmentBoundary { source, .. } => vec![source.clone()],
        ExpandedLexicalProducer::LaneOutcome {
            success_slot,
            failure_slot,
            ..
        } => {
            let mut children = Vec::with_capacity(2);
            if let Some(slot) = success_slot {
                children.push(slot.clone());
            }
            if let Some(slot) = failure_slot {
                children.push(slot.clone());
            }
            children
        }
        ExpandedLexicalProducer::FanOutJoin {
            declaration_ordered_lane_slots,
            ..
        } => declaration_ordered_lane_slots.clone(),
    }
}

struct StructuredProgramDenormalizer {
    structural_paths: BTreeMap<ContentRef, StructuralPath>,
    lexical_slots: BTreeMap<ContentRef, ExpandedLexicalSlot>,
    resolved_slots: BTreeMap<ContentRef, LexicalSlot>,
    active_slots: BTreeSet<ContentRef>,
    used_paths: BTreeSet<ContentRef>,
    used_slots: BTreeSet<ContentRef>,
}

impl StructuredProgramDenormalizer {
    fn new(
        structural_paths: Vec<ExpandedStructuralPathDefinition>,
        lexical_slots: Vec<ExpandedLexicalSlotDefinition>,
    ) -> Result<Self> {
        if structural_paths.len() > MAX_CERTIFIED_COMPONENT_OBJECTS
            || lexical_slots.len() > MAX_CERTIFIED_COMPONENT_OBJECTS
        {
            return Err(SpecError::Invariant(
                "expanded provenance table exceeds its bound".to_owned(),
            ));
        }
        ensure_definition_order(
            structural_paths
                .iter()
                .map(|definition| &definition.path_ref),
            "expanded structural path",
        )?;
        ensure_definition_order(
            lexical_slots.iter().map(|definition| &definition.slot_ref),
            "expanded lexical slot",
        )?;

        let mut path_map = BTreeMap::new();
        for definition in structural_paths {
            if definition.path.content_ref()? != definition.path_ref {
                return Err(SpecError::Invariant(
                    "expanded structural path reference does not match its definition".to_owned(),
                ));
            }
            if path_map
                .insert(definition.path_ref, definition.path)
                .is_some()
            {
                return Err(SpecError::Invariant(
                    "duplicate expanded structural path definition".to_owned(),
                ));
            }
        }

        let mut slot_map = BTreeMap::new();
        for definition in lexical_slots {
            if definition.slot.content_ref()? != definition.slot_ref {
                return Err(SpecError::Invariant(
                    "expanded lexical slot reference does not match its definition".to_owned(),
                ));
            }
            if slot_map
                .insert(definition.slot_ref, definition.slot)
                .is_some()
            {
                return Err(SpecError::Invariant(
                    "duplicate expanded lexical slot definition".to_owned(),
                ));
            }
        }

        Ok(Self {
            structural_paths: path_map,
            lexical_slots: slot_map,
            resolved_slots: BTreeMap::new(),
            active_slots: BTreeSet::new(),
            used_paths: BTreeSet::new(),
            used_slots: BTreeSet::new(),
        })
    }

    fn resolve_path(&mut self, reference: ExpandedPathRef) -> Result<StructuralPath> {
        let path = self
            .structural_paths
            .get(&reference.path_ref)
            .cloned()
            .ok_or_else(|| {
                SpecError::Invariant(
                    "expanded structural path reference is not defined locally".to_owned(),
                )
            })?;
        self.used_paths.insert(reference.path_ref);
        Ok(path)
    }

    fn resolve_slot(&mut self, start: ExpandedSlotRef) -> Result<LexicalSlot> {
        // Explicit work stack with a depth budget independent of process stack
        // size: a shallow table can encode a long acyclic reference chain.
        enum Phase {
            Enter,
            Finish(Box<ExpandedLexicalSlot>),
        }
        struct Frame {
            reference: ExpandedSlotRef,
            depth: usize,
            phase: Phase,
        }
        let mut stack = vec![Frame {
            reference: start,
            depth: 0,
            phase: Phase::Enter,
        }];
        let mut last = None;
        while let Some(frame) = stack.pop() {
            match frame.phase {
                Phase::Enter => {
                    if frame.depth > MAX_PROVENANCE_RESOLUTION_DEPTH {
                        return Err(SpecError::Invariant(
                            "expanded provenance resolution depth exceeded".to_owned(),
                        ));
                    }
                    if let Some(slot) = self.resolved_slots.get(&frame.reference.slot_ref) {
                        self.used_slots.insert(frame.reference.slot_ref.clone());
                        last = Some(slot.clone());
                        continue;
                    }
                    if !self.active_slots.insert(frame.reference.slot_ref.clone()) {
                        return Err(SpecError::Invariant(
                            "expanded lexical slot graph contains a cycle".to_owned(),
                        ));
                    }
                    let definition = self
                        .lexical_slots
                        .get(&frame.reference.slot_ref)
                        .cloned()
                        .ok_or_else(|| {
                            SpecError::Invariant(
                                "expanded lexical slot reference is not defined locally".to_owned(),
                            )
                        })?;
                    let children = expanded_producer_child_slots(&definition.producer);
                    stack.push(Frame {
                        reference: frame.reference,
                        depth: frame.depth,
                        phase: Phase::Finish(Box::new(definition)),
                    });
                    for child in children.into_iter().rev() {
                        stack.push(Frame {
                            reference: child,
                            depth: frame.depth.saturating_add(1),
                            phase: Phase::Enter,
                        });
                    }
                }
                Phase::Finish(definition) => {
                    let definition = *definition;
                    let producer = self.assemble_resolved_producer(definition.producer)?;
                    let slot = LexicalSlot {
                        lexical_path: self.resolve_path(ExpandedPathRef {
                            path_ref: definition.lexical_path_ref,
                        })?,
                        contract_ref: definition.contract_ref,
                        producer,
                    };
                    self.active_slots.remove(&frame.reference.slot_ref);
                    self.used_slots.insert(frame.reference.slot_ref.clone());
                    self.resolved_slots
                        .insert(frame.reference.slot_ref.clone(), slot.clone());
                    last = Some(slot);
                }
            }
        }
        last.ok_or_else(|| {
            SpecError::Invariant("expanded provenance resolution produced no slot".to_owned())
        })
    }

    fn take_resolved_slot(&mut self, reference: ExpandedSlotRef) -> Result<LexicalSlot> {
        let slot = self
            .resolved_slots
            .get(&reference.slot_ref)
            .cloned()
            .ok_or_else(|| {
                SpecError::Invariant(
                    "expanded lexical slot dependency was not resolved before assembly".to_owned(),
                )
            })?;
        self.used_slots.insert(reference.slot_ref);
        Ok(slot)
    }

    fn assemble_resolved_producer(
        &mut self,
        producer: ExpandedLexicalProducer,
    ) -> Result<LexicalProducer> {
        Ok(match producer {
            ExpandedLexicalProducer::AdmissionRoot { root_id } => {
                LexicalProducer::AdmissionRoot { root_id }
            }
            ExpandedLexicalProducer::AuthoredCallOutput {
                semantic_call_id,
                role,
            } => LexicalProducer::AuthoredCallOutput {
                semantic_call_id,
                role,
            },
            ExpandedLexicalProducer::StateOutput {
                occurrence_id,
                role,
            } => LexicalProducer::StateOutput {
                occurrence_id,
                role,
            },
            ExpandedLexicalProducer::ArmValue {
                selected_arm_path,
                source,
            } => LexicalProducer::ArmValue {
                selected_arm_path: self.resolve_path(selected_arm_path)?,
                source: Box::new(self.take_resolved_slot(source)?),
            },
            ExpandedLexicalProducer::MatchMerge {
                match_path,
                declaration_ordered_arm_slots,
            } => LexicalProducer::MatchMerge {
                match_path: self.resolve_path(match_path)?,
                declaration_ordered_arm_slots: declaration_ordered_arm_slots
                    .into_iter()
                    .map(|slot| self.take_resolved_slot(slot))
                    .collect::<Result<Vec<_>>>()?,
            },
            ExpandedLexicalProducer::ScopeFailureMerge {
                scope_id,
                declaration_ordered_failure_slots,
            } => LexicalProducer::ScopeFailureMerge {
                scope_id,
                declaration_ordered_failure_slots: declaration_ordered_failure_slots
                    .into_iter()
                    .map(|slot| self.take_resolved_slot(slot))
                    .collect::<Result<Vec<_>>>()?,
            },
            ExpandedLexicalProducer::VariantPayload {
                selector,
                canonical_tag,
                payload_path,
            } => LexicalProducer::VariantPayload {
                selector: Box::new(self.take_resolved_slot(selector)?),
                canonical_tag,
                payload_path,
            },
            ExpandedLexicalProducer::FragmentInput {
                boundary_id,
                child_root_id,
                source,
            } => LexicalProducer::FragmentInput {
                boundary_id,
                child_root_id,
                source: Box::new(self.take_resolved_slot(source)?),
            },
            ExpandedLexicalProducer::FragmentBoundary {
                boundary_id,
                role,
                source,
            } => LexicalProducer::FragmentBoundary {
                boundary_id,
                role,
                source: Box::new(self.take_resolved_slot(source)?),
            },
            ExpandedLexicalProducer::LaneOutcome {
                lane_path,
                success_slot,
                failure_slot,
            } => LexicalProducer::LaneOutcome {
                lane_path: self.resolve_path(lane_path)?,
                success_slot: success_slot
                    .map(|slot| self.take_resolved_slot(slot).map(Box::new))
                    .transpose()?,
                failure_slot: failure_slot
                    .map(|slot| self.take_resolved_slot(slot).map(Box::new))
                    .transpose()?,
            },
            ExpandedLexicalProducer::FanOutJoin {
                group_path,
                declaration_ordered_lane_slots,
            } => LexicalProducer::FanOutJoin {
                group_path: self.resolve_path(group_path)?,
                declaration_ordered_lane_slots: declaration_ordered_lane_slots
                    .into_iter()
                    .map(|slot| self.take_resolved_slot(slot))
                    .collect::<Result<Vec<_>>>()?,
            },
        })
    }

    fn denormalize_value(&mut self, root: serde_json::Value) -> Result<serde_json::Value> {
        // Explicit work stack with a node budget independent of process stack size.
        enum Frame {
            Start(serde_json::Value),
            ArrayCollect { total: usize },
            ObjectCollect { keys: Vec<String> },
        }
        let mut stack = vec![Frame::Start(root)];
        let mut completed = Vec::new();
        let mut nodes = 0usize;
        while let Some(frame) = stack.pop() {
            match frame {
                Frame::Start(value) => {
                    nodes = nodes.saturating_add(1);
                    if nodes > MAX_STRUCTURED_JSON_NODES {
                        return Err(SpecError::Invariant(
                            "structured program JSON node budget exceeded".to_owned(),
                        ));
                    }
                    if is_expanded_slot_reference(&value) {
                        let reference = serde_json::from_value(value)
                            .map_err(|error| SpecError::Invariant(error.to_string()))?;
                        completed.push(
                            serde_json::to_value(self.resolve_slot(reference)?)
                                .map_err(|error| SpecError::Invariant(error.to_string()))?,
                        );
                        continue;
                    }
                    if is_expanded_path_reference(&value) {
                        let reference = serde_json::from_value(value)
                            .map_err(|error| SpecError::Invariant(error.to_string()))?;
                        completed.push(
                            serde_json::to_value(self.resolve_path(reference)?)
                                .map_err(|error| SpecError::Invariant(error.to_string()))?,
                        );
                        continue;
                    }
                    match value {
                        serde_json::Value::Array(values) => {
                            let total = values.len();
                            stack.push(Frame::ArrayCollect { total });
                            for value in values.into_iter().rev() {
                                stack.push(Frame::Start(value));
                            }
                        }
                        serde_json::Value::Object(values) => {
                            let mut keys = Vec::with_capacity(values.len());
                            let mut items = Vec::with_capacity(values.len());
                            for (key, value) in values {
                                keys.push(key.clone());
                                items.push((key, value));
                            }
                            stack.push(Frame::ObjectCollect { keys });
                            for (_, value) in items.into_iter().rev() {
                                stack.push(Frame::Start(value));
                            }
                        }
                        scalar => completed.push(scalar),
                    }
                }
                Frame::ArrayCollect { total } => {
                    let mut items = Vec::with_capacity(total);
                    for _ in 0..total {
                        items.push(completed.pop().ok_or_else(|| {
                            SpecError::Invariant(
                                "structured program JSON array frame underfilled".to_owned(),
                            )
                        })?);
                    }
                    items.reverse();
                    completed.push(serde_json::Value::Array(items));
                }
                Frame::ObjectCollect { keys } => {
                    let mut object = serde_json::Map::with_capacity(keys.len());
                    for key in keys.into_iter().rev() {
                        let value = completed.pop().ok_or_else(|| {
                            SpecError::Invariant(
                                "structured program JSON object frame underfilled".to_owned(),
                            )
                        })?;
                        object.insert(key, value);
                    }
                    completed.push(serde_json::Value::Object(object));
                }
            }
        }
        completed.pop().ok_or_else(|| {
            SpecError::Invariant(
                "structured program JSON denormalization produced no root".to_owned(),
            )
        })
    }

    fn ensure_complete_use(&self) -> Result<()> {
        if self.structural_paths.len() != self.used_paths.len()
            || self.lexical_slots.len() != self.used_slots.len()
        {
            return Err(SpecError::Invariant(
                "expanded provenance table contains an unused definition".to_owned(),
            ));
        }
        Ok(())
    }
}

impl Serialize for ExpandedStructuredProgram {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut normalizer = StructuredProgramNormalizer::default();
        let input_roots = self
            .input_roots
            .iter()
            .cloned()
            .map(|slot| normalizer.normalize_slot(slot).map_err(S::Error::custom))
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let root = normalizer
            .normalize_value(
                serde_json::to_value(&self.root).map_err(S::Error::custom)?,
                None,
            )
            .map_err(S::Error::custom)?;
        let (structural_paths, lexical_slots) = normalizer.finish();
        ensure_expanded_slot_definitions(&lexical_slots).map_err(S::Error::custom)?;
        NormalizedStructuredProgram {
            operation_id: self.operation_id.clone(),
            structural_paths,
            lexical_slots,
            input_root_slot_refs: input_roots,
            output_contract_ref: self.output_contract_ref.clone(),
            failure_contract: self.failure_contract.clone(),
            root,
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for ExpandedStructuredProgram {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let normalized = NormalizedStructuredProgram::deserialize(deserializer)?;
        ensure_expanded_slot_definitions(&normalized.lexical_slots).map_err(D::Error::custom)?;
        let mut denormalizer = StructuredProgramDenormalizer::new(
            normalized.structural_paths,
            normalized.lexical_slots,
        )
        .map_err(D::Error::custom)?;
        let input_roots = normalized
            .input_root_slot_refs
            .into_iter()
            .map(|reference| denormalizer.resolve_slot(reference))
            .collect::<Result<Vec<_>>>()
            .map_err(D::Error::custom)?;
        let root = denormalizer
            .denormalize_value(normalized.root)
            .and_then(|value| {
                serde_json::from_value(value)
                    .map_err(|error| SpecError::Invariant(error.to_string()))
            })
            .map_err(D::Error::custom)?;
        denormalizer
            .ensure_complete_use()
            .map_err(D::Error::custom)?;
        Ok(Self {
            operation_id: normalized.operation_id,
            input_roots,
            output_contract_ref: normalized.output_contract_ref,
            failure_contract: normalized.failure_contract,
            root,
        })
    }
}

fn ensure_expanded_slot_definitions(definitions: &[ExpandedLexicalSlotDefinition]) -> Result<()> {
    if definitions.iter().any(|definition| {
        matches!(
            definition.slot.producer,
            ExpandedLexicalProducer::AuthoredCallOutput { .. }
        )
    }) {
        return Err(SpecError::Invariant(
            "expanded provenance table contains an authored call output".to_owned(),
        ));
    }
    Ok(())
}

impl Serialize for AuthoredStructuredProgram {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut normalizer = StructuredProgramNormalizer::default();
        let input_roots = self
            .input_roots
            .iter()
            .cloned()
            .map(|slot| normalizer.normalize_slot(slot).map_err(S::Error::custom))
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let root = normalizer
            .normalize_value(
                serde_json::to_value(&self.root).map_err(S::Error::custom)?,
                None,
            )
            .map_err(S::Error::custom)?;
        let (structural_paths, lexical_slots) = normalizer.finish();
        NormalizedStructuredProgram {
            operation_id: self.operation_id.clone(),
            structural_paths,
            lexical_slots,
            input_root_slot_refs: input_roots,
            output_contract_ref: self.output_contract_ref.clone(),
            failure_contract: self.failure_contract.clone(),
            root,
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for AuthoredStructuredProgram {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let normalized = NormalizedStructuredProgram::deserialize(deserializer)?;
        let mut denormalizer = StructuredProgramDenormalizer::new(
            normalized.structural_paths,
            normalized.lexical_slots,
        )
        .map_err(D::Error::custom)?;
        let input_roots = normalized
            .input_root_slot_refs
            .into_iter()
            .map(|reference| denormalizer.resolve_slot(reference))
            .collect::<Result<Vec<_>>>()
            .map_err(D::Error::custom)?;
        let root = denormalizer
            .denormalize_value(normalized.root)
            .and_then(|value| {
                serde_json::from_value(value)
                    .map_err(|error| SpecError::Invariant(error.to_string()))
            })
            .map_err(D::Error::custom)?;
        denormalizer
            .ensure_complete_use()
            .map_err(D::Error::custom)?;
        Ok(Self {
            operation_id: normalized.operation_id,
            input_roots,
            output_contract_ref: normalized.output_contract_ref,
            failure_contract: normalized.failure_contract,
            root,
        })
    }
}

fn insert_normalized_definition<T: PartialEq>(
    definitions: &mut BTreeMap<ContentRef, T>,
    reference: ContentRef,
    definition: T,
    kind: &str,
) -> Result<()> {
    match definitions.entry(reference) {
        std::collections::btree_map::Entry::Vacant(entry) => {
            entry.insert(definition);
        }
        std::collections::btree_map::Entry::Occupied(entry) => {
            if entry.get() != &definition {
                return Err(SpecError::Invariant(format!(
                    "{kind} content reference aliases different definitions"
                )));
            }
        }
    }
    Ok(())
}

fn ensure_definition_order<'a>(
    mut references: impl Iterator<Item = &'a ContentRef>,
    kind: &str,
) -> Result<()> {
    let Some(mut previous) = references.next() else {
        return Ok(());
    };
    for current in references {
        if previous >= current {
            return Err(SpecError::Invariant(format!(
                "{kind} definitions are not in strict canonical order"
            )));
        }
        previous = current;
    }
    Ok(())
}

fn is_inline_lexical_slot(value: &serde_json::Value) -> bool {
    value.as_object().is_some_and(|object| {
        object.len() == 3
            && object.contains_key("lexical_path")
            && object.contains_key("contract_ref")
            && object.contains_key("producer")
    })
}

fn is_structural_path_field(field_name: &str) -> bool {
    matches!(
        field_name,
        "path"
            | "occurrence_path"
            | "plan_path"
            | "selected_arm_path"
            | "match_path"
            | "lane_path"
            | "group_path"
    )
}

fn is_expanded_slot_reference(value: &serde_json::Value) -> bool {
    value
        .as_object()
        .is_some_and(|object| object.len() == 1 && object.contains_key("slot_ref"))
}

fn is_expanded_path_reference(value: &serde_json::Value) -> bool {
    value
        .as_object()
        .is_some_and(|object| object.len() == 1 && object.contains_key("path_ref"))
}

/// One boundary-specific declarative recipe selected by an expansion policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyExpansionBinding {
    /// Exact semantic boundary contract eligible for this recipe.
    pub boundary_contract_ref: ContentRef,
    /// Exact canonical authored policy-recipe program.
    pub recipe_ref: ContentRef,
}

/// One exact policy in the frozen expansion profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExpansionPolicyContract {
    /// Exact policy contract identity.
    pub policy_ref: ContentRef,
    /// Exact boundary-to-recipe table in canonical profile order.
    pub boundary_recipes: Vec<PolicyExpansionBinding>,
}

#[derive(Serialize)]
struct ExpansionPolicyContractPreimage<'a> {
    boundary_recipes: &'a [PolicyExpansionBinding],
}

impl ExpansionPolicyContract {
    /// Constructs one exact expansion-policy contract from its closed,
    /// boundary-specific declarative recipe table.
    pub fn new(boundary_recipes: Vec<PolicyExpansionBinding>) -> Result<Self> {
        let canonical = canonical(&ExpansionPolicyContractPreimage {
            boundary_recipes: &boundary_recipes,
        })?;
        let contract = Self {
            policy_ref: structured_content_ref("mfm.structured-expansion-policy", &canonical)?,
            boundary_recipes,
        };
        contract.validate()?;
        Ok(contract)
    }

    fn preimage(&self) -> ExpansionPolicyContractPreimage<'_> {
        ExpansionPolicyContractPreimage {
            boundary_recipes: &self.boundary_recipes,
        }
    }

    /// Returns the exact recipe selected for one semantic boundary contract.
    pub fn recipe_for(&self, boundary_contract_ref: &ContentRef) -> Option<&ContentRef> {
        self.boundary_recipes
            .iter()
            .find(|binding| &binding.boundary_contract_ref == boundary_contract_ref)
            .map(|binding| &binding.recipe_ref)
    }

    /// Returns the exact canonical expansion-policy contract bytes.
    pub fn canonical_contract_json(&self) -> Result<PlainCanonicalJsonBytes> {
        canonical(&self.preimage())
    }

    fn derived_policy_ref(&self) -> Result<ContentRef> {
        structured_content_ref(
            "mfm.structured-expansion-policy",
            &self.canonical_contract_json()?,
        )
    }

    /// Revalidates the policy identity and its closed eligible-boundary set.
    pub fn validate(&self) -> Result<()> {
        let mut eligible = BTreeSet::new();
        if self
            .boundary_recipes
            .iter()
            .any(|binding| !eligible.insert(&binding.boundary_contract_ref))
        {
            return Err(SpecError::Invariant(
                "duplicate eligible policy boundary".to_owned(),
            ));
        }
        if self.policy_ref != self.derived_policy_ref()? {
            return Err(SpecError::Invariant(
                "structured expansion-policy identity mismatch".to_owned(),
            ));
        }
        Ok(())
    }
}

/// Exact trusted pure expansion profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StructuredExpansionProfile {
    /// Policies in outer-to-inner entry order.
    pub policies: Vec<ExpansionPolicyContract>,
    /// Hard total expanded occurrence bound.
    pub max_occurrences: u32,
    /// Hard total structural declaration bound.
    pub max_declarations: u32,
    /// Hard total lane bound.
    pub max_lanes: u32,
    /// Hard fan-out nesting depth.
    pub max_fan_out_depth: u8,
    /// Hard structural branch depth.
    pub max_branch_depth: u8,
}

/// Exact hard structural limits bound directly into a certified-program root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CertifiedStructuralBounds {
    /// Hard total expanded occurrence bound.
    pub max_occurrences: u32,
    /// Hard total structural declaration bound.
    pub max_declarations: u32,
    /// Hard total lane bound.
    pub max_lanes: u32,
    /// Hard fan-out nesting depth.
    pub max_fan_out_depth: u8,
    /// Hard structural branch depth.
    pub max_branch_depth: u8,
}

impl From<&StructuredExpansionProfile> for CertifiedStructuralBounds {
    fn from(profile: &StructuredExpansionProfile) -> Self {
        Self {
            max_occurrences: profile.max_occurrences,
            max_declarations: profile.max_declarations,
            max_lanes: profile.max_lanes,
            max_fan_out_depth: profile.max_fan_out_depth,
            max_branch_depth: profile.max_branch_depth,
        }
    }
}

impl StructuredExpansionProfile {
    /// Returns exact canonical profile bytes.
    pub fn canonical_json(&self) -> Result<PlainCanonicalJsonBytes> {
        canonical(self)
    }

    /// Returns the canonical profile object reference.
    pub fn content_ref(&self) -> Result<ContentRef> {
        structured_content_ref("mfm.structured-expansion-profile", &self.canonical_json()?)
    }
}

/// One exact policy-coverage proof entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyCoverageEntry {
    /// Protected semantic boundary identity.
    pub semantic_call_id: SemanticCallId,
    /// Dense policy ordinal in the exact profile.
    pub profile_ordinal: u32,
    /// Exact policy contract applied at that ordinal.
    pub policy_ref: ContentRef,
}

/// Deterministic expansion proof and complete policy coverage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StructuredExpansionProof {
    /// Exact authored input program.
    pub authored_program_ref: ContentRef,
    /// Exact expanded output program.
    pub expanded_program_ref: ContentRef,
    /// Exact expansion profile.
    pub expansion_profile_ref: ContentRef,
    /// Canonical ordered record of child/capability/policy substitutions.
    pub substitution_trace: Vec<ExpansionTraceEntry>,
}

/// Exact independently recomputed policy-coverage proof.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StructuredPolicyCoverageProof {
    /// Exact authored input program.
    pub authored_program_ref: ContentRef,
    /// Exact expanded output program.
    pub expanded_program_ref: ContentRef,
    /// Exact qualified expansion profile.
    pub expansion_profile_ref: ContentRef,
    /// Semantic-boundary then profile-ordinal coverage entries.
    pub entries: Vec<PolicyCoverageEntry>,
}

/// One pure expansion substitution in canonical pipeline order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExpansionTraceEntry {
    /// Pipeline stage.
    pub stage: ExpansionStage,
    /// Protected semantic boundary.
    pub semantic_call_id: SemanticCallId,
    /// Exact child, capability, or policy contract.
    pub expansion_ref: ContentRef,
    /// Exact state or non-executable fragment boundary affected by this phase.
    pub boundary_id: ExpansionBoundaryId,
}

/// Nominal identity of the state or fragment boundary affected by expansion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum ExpansionBoundaryId {
    /// Exact executable state occurrence.
    State(OccurrenceId),
    /// Exact non-executable fragment boundary.
    Fragment(FragmentBoundaryId),
}

/// Frozen pure expansion pipeline stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExpansionStage {
    /// Authored child-operation substitution.
    ChildSubstitution,
    /// Abstract semantic capability lowering.
    CapabilityLowering,
    /// Profile-ordered policy wrapping.
    PolicyWrapping,
    /// Exact typed failure completion.
    FailureCompletion,
}

/// Registered outbound reference from one certified component object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComponentObjectReference {
    /// Expected registered object-type domain tag.
    pub object_type: StableId,
    /// Exact referenced object.
    pub content_ref: ContentRef,
}

/// One exact secret-free object in the certified-program component closure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CertifiedComponentObject {
    /// Registered object-type domain tag.
    pub object_type: StableId,
    /// Exact content identity.
    pub content_ref: ContentRef,
    /// Float-free canonical structured value.
    pub value: CanonicalJsonValue,
}

/// Canonical public input/output/failure contract bundle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StructuredPublicContractRefs {
    /// Admission-input contracts in declared root order.
    pub input_contract_refs: Vec<ContentRef>,
    /// Exact successful root contract.
    pub output_contract_ref: ContentRef,
    /// Exact root failure contract, including the kernel `Never` sentinel.
    pub failure_contract_ref: ContentRef,
}

/// Exact canonical root preimage for one certified program.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CertifiedProgramComponents {
    /// Certified-program contract identity.
    pub certified_program_contract_ref: ContentRef,
    /// Qualified entry-point contract.
    pub entry_point_contract_ref: ContentRef,
    /// Exact qualified entry-point admission policy.
    pub qualified_entry_point_admission_policy_ref: ContentRef,
    /// Authored program.
    pub authored_program_ref: ContentRef,
    /// Expanded program.
    pub expanded_program_ref: ContentRef,
    /// Exact expansion profile.
    pub expansion_profile_ref: ContentRef,
    /// Exact expansion proof.
    pub expansion_proof_ref: ContentRef,
    /// Exact policy-coverage proof.
    pub policy_coverage_proof_ref: ContentRef,
    /// Canonical public input/output/failure contract bundle.
    pub public_input_output_failure_contract_refs: StructuredPublicContractRefs,
    /// Exact structural bounds selected by the qualified expansion profile.
    pub certified_structural_bounds: CertifiedStructuralBounds,
    /// Logical State/Capability/Adapter/Signer/Resource manifest closure root.
    pub state_capability_adapter_signer_resource_manifest_closure_ref: ContentRef,
    /// Exact secret-free implementation-manifest closure root.
    pub secret_free_implementation_manifest_closure_ref: ContentRef,
    /// Exact certification predicate set.
    pub certification_predicate_set_ref: ContentRef,
}

/// Canonical certified-program authority root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CertifiedProgramRoot {
    /// Exact canonical component reference bundle.
    pub components: CertifiedProgramComponents,
    /// Domain-separated digest of components plus the ordered component closure.
    pub canonical_component_closure_digest: ContentDigest,
}

impl CertifiedProgramRoot {
    /// Returns exact canonical root bytes without embedding its component closure.
    pub fn canonical_json(&self) -> Result<PlainCanonicalJsonBytes> {
        canonical(self)
    }

    /// Returns the sole canonical certified-program authority reference.
    pub fn content_ref(&self) -> Result<ContentRef> {
        let canonical = self.canonical_json()?;
        let mut preimage = b"mfm.certified-program.v1\0".to_vec();
        preimage.extend_from_slice(canonical.as_bytes());
        let schema_name = "mfm.certified-program";
        ContentRef::new(
            SchemaId::new(
                schema_name,
                "1",
                DigestAlgorithm::Sha256JcsV1,
                sha256_digest_bytes(format!("mfm.structured-schema.v1:{schema_name}:1").as_bytes()),
            )?,
            ContentDigest::from_digest(DigestAlgorithm::Sha256V1, sha256_digest_bytes(&preimage)),
        )
        .map_err(Into::into)
    }
}

/// Canonical certified-program document persisted at admission.
///
/// This is data, not process authority. `mfm-certify::CertifiedProgram` is the
/// non-forgeable in-process authority returned only after complete validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CertifiedProgramDocument {
    /// Exact root hashed as the sole `CertifiedProgramRef` authority.
    pub root: CertifiedProgramRoot,
    /// Canonical first-visit depth-first component closure.
    pub component_closure: Vec<CertifiedComponentObject>,
}

impl CertifiedProgramDocument {
    /// Returns the canonical certified-program reference.
    pub fn content_ref(&self) -> Result<ContentRef> {
        self.root.content_ref()
    }
}

/// Producer-free semantic value selected by one state callback.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum ProposedStateValue<T, F> {
    /// Proposed successful value.
    Success(T),
    /// Proposed typed failure value.
    Failure(F),
}

/// Uncommitted state callback proposal with its exact durable fact set.
///
/// It carries no producer, object, transition, or append authority. The store
/// binds accepted values and facts to the current certified occurrence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProposedStateOutcome<T, F> {
    #[serde(flatten)]
    value: ProposedStateValue<T, F>,
    facts: mfm_facts::FactSet,
}

impl<T, F> ProposedStateOutcome<T, F> {
    /// Constructs one successful proposal without facts.
    #[allow(non_snake_case)]
    pub fn Success(value: T) -> Self {
        Self {
            value: ProposedStateValue::Success(value),
            facts: mfm_facts::FactSet::empty(),
        }
    }

    /// Constructs one typed failure proposal. Failed transitions emit no facts.
    #[allow(non_snake_case)]
    pub fn Failure(value: F) -> Self {
        Self {
            value: ProposedStateValue::Failure(value),
            facts: mfm_facts::FactSet::empty(),
        }
    }

    /// Constructs one successful proposal with its exact fact proposals.
    pub fn success_with_facts(value: T, facts: mfm_facts::FactSet) -> Self {
        Self {
            value: ProposedStateValue::Success(value),
            facts,
        }
    }

    /// Returns the producer-free proposed outcome value.
    pub const fn value(&self) -> &ProposedStateValue<T, F> {
        &self.value
    }

    /// Returns facts in exact callback emission order.
    pub const fn facts(&self) -> &mfm_facts::FactSet {
        &self.facts
    }

    /// Consumes the proposal into its value and fact set.
    pub fn into_parts(self) -> (ProposedStateValue<T, F>, mfm_facts::FactSet) {
        (self.value, self.facts)
    }
}

/// Success-only uncommitted proposal admitted by safe-failure settlement under
/// [`super::StructuredSafeFailureDispositionContract::AllValidEvidenceSettlesSuccess`].
///
/// The type has no `Failure` or `InvalidEvidence` variant: every inhabited
/// safe-failure value that reaches this path proposes success.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProposedSuccessOutcome<T> {
    value: T,
    facts: mfm_facts::FactSet,
}

impl<T> ProposedSuccessOutcome<T> {
    /// Constructs one successful proposal without facts.
    pub fn new(value: T) -> Self {
        Self {
            value,
            facts: mfm_facts::FactSet::empty(),
        }
    }

    /// Constructs one successful proposal with its exact fact proposals.
    pub fn with_facts(value: T, facts: mfm_facts::FactSet) -> Self {
        Self { value, facts }
    }

    /// Returns the proposed successful value.
    pub const fn value(&self) -> &T {
        &self.value
    }

    /// Returns facts in exact callback emission order.
    pub const fn facts(&self) -> &mfm_facts::FactSet {
        &self.facts
    }

    /// Consumes the proposal into its value and fact set.
    pub fn into_parts(self) -> (T, mfm_facts::FactSet) {
        (self.value, self.facts)
    }

    /// Lifts this success-only proposal into a general proposed outcome.
    pub fn into_proposed_outcome<F>(self) -> ProposedStateOutcome<T, F> {
        ProposedStateOutcome::success_with_facts(self.value, self.facts)
    }
}

/// Nominal committed state result constructed only by an accepted transition.
///
/// Structurally identical outcomes from another boundary are not substitutable:
///
/// ```compile_fail
/// use mfm_spec::structured::{OperationOutcome, StateOutcome};
///
/// let state = StateOutcome::<u64, u64>::Success(7);
/// let _: OperationOutcome<u64, u64> = state;
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum StateOutcome<T, F> {
    /// Committed successful state output.
    Success(T),
    /// Committed typed state failure.
    Failure(F),
}

/// Nominal structural fan-out lane result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LaneOutcome<T, F> {
    /// Completed lane success.
    Success(T),
    /// Completed lane failure after lane-owned handling.
    Failure(F),
}

/// Nominal root result of one complete operation run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum OperationOutcome<T, F> {
    /// Completed operation success.
    Success(T),
    /// Completed operation failure.
    Failure(F),
}

/// Derives the exact nominal contract of one structural lane outcome.
pub fn lane_outcome_contract_ref(
    success_contract_ref: &ContentRef,
    failure_contract: &StructuredFailureContract,
) -> Result<ContentRef> {
    let canonical = lane_outcome_contract_canonical_json(success_contract_ref, failure_contract)?;
    structured_content_ref("mfm.lane-outcome-contract", &canonical)
}

/// Returns the canonical preimage of one structural lane-outcome contract.
pub fn lane_outcome_contract_canonical_json(
    success_contract_ref: &ContentRef,
    failure_contract: &StructuredFailureContract,
) -> Result<PlainCanonicalJsonBytes> {
    canonical(&(success_contract_ref, failure_contract))
}

/// Derives the nominal non-empty head-plus-tail join contract for one fan-out
/// group from its exact lane outcome contract.
pub fn fan_out_join_contract_ref(
    success_contract_ref: &ContentRef,
    failure_contract: &StructuredFailureContract,
) -> Result<ContentRef> {
    let canonical = fan_out_join_contract_canonical_json(success_contract_ref, failure_contract)?;
    structured_content_ref("mfm.fan-out-join-contract", &canonical)
}

/// Returns the canonical preimage of one nominal non-empty fan-out join contract.
pub fn fan_out_join_contract_canonical_json(
    success_contract_ref: &ContentRef,
    failure_contract: &StructuredFailureContract,
) -> Result<PlainCanonicalJsonBytes> {
    let lane_contract_ref = lane_outcome_contract_ref(success_contract_ref, failure_contract)?;
    canonical(&lane_contract_ref)
}

/// One recursive structured-value algebra node for qualification closure.
///
/// Every admission, state, child-success, Match-result, policy, fan-out lane,
/// and operation-root success boundary is closed by this algebra. Store
/// validation dispatches through the derived contract identity; there is no
/// parallel aggregate append path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum StructuredValueDefinition {
    /// Exact retained MFM value schema and role contract.
    Retained {
        /// Annex-validated retained-value contract and paired schema identity.
        ///
        /// The payload is boxed for enum layout while `flatten` preserves the
        /// established flat persisted contract (`contract` and `schema`).
        #[serde(flatten)]
        payload: Box<RetainedStructuredValueDefinition>,
    },
    /// Non-empty fan-out join of recursively defined lane success values.
    NonEmptyFanOutJoin {
        /// Recursive definition of each lane success body.
        lane_success: Box<StructuredValueDefinition>,
        /// Exact lane failure contract (`Never` or typed retained failure).
        lane_failure: StructuredFailureContract,
    },
}

/// Owned payload for a retained structured-value leaf.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetainedStructuredValueDefinition {
    /// Annex-validated retained-value contract.
    pub contract: RetainedValueContract,
    /// Qualified schema identity paired with the retained contract.
    pub schema: mfm_values::SchemaIdentity,
}

impl StructuredValueDefinition {
    /// Constructs a retained leaf after proving contract/schema agreement.
    pub fn retained(
        contract: RetainedValueContract,
        schema: mfm_values::SchemaIdentity,
    ) -> Result<Self> {
        let schema_id = schema
            .schema_id()
            .map_err(|error| SpecError::Contract(error.to_string()))?;
        if schema_id != *contract.schema_id()
            || schema.semantic_type_id.as_ref() != Some(contract.semantic_type_id())
        {
            return Err(SpecError::Invariant(
                "structured value definition schema differs from its retained contract".to_owned(),
            ));
        }
        Ok(Self::Retained {
            payload: Box::new(RetainedStructuredValueDefinition { contract, schema }),
        })
    }

    /// Constructs a non-empty recursive fan-out join definition.
    pub fn non_empty_fan_out_join(
        lane_success: StructuredValueDefinition,
        lane_failure: StructuredFailureContract,
    ) -> Result<Self> {
        lane_failure.validate()?;
        Ok(Self::NonEmptyFanOutJoin {
            lane_success: Box::new(lane_success),
            lane_failure,
        })
    }

    /// Derives the exact nominal contract identity for this definition.
    pub fn contract_ref(&self) -> Result<ContentRef> {
        match self {
            Self::Retained { payload } => retained_value_contract_ref(&payload.contract),
            Self::NonEmptyFanOutJoin {
                lane_success,
                lane_failure,
            } => fan_out_join_contract_ref(&lane_success.contract_ref()?, lane_failure),
        }
    }

    /// Returns the exact lane-outcome contract for a fan-out join definition.
    pub fn lane_outcome_contract_ref(&self) -> Result<Option<ContentRef>> {
        match self {
            Self::Retained { .. } => Ok(None),
            Self::NonEmptyFanOutJoin {
                lane_success,
                lane_failure,
            } => Ok(Some(lane_outcome_contract_ref(
                &lane_success.contract_ref()?,
                lane_failure,
            )?)),
        }
    }

    /// Walks retained leaves in definition order (left-to-right, depth-first).
    pub fn retained_leaves(&self) -> Vec<(&RetainedValueContract, &mfm_values::SchemaIdentity)> {
        let mut leaves = Vec::new();
        self.collect_retained_leaves(&mut leaves);
        leaves
    }

    fn collect_retained_leaves<'a>(
        &'a self,
        leaves: &mut Vec<(&'a RetainedValueContract, &'a mfm_values::SchemaIdentity)>,
    ) {
        match self {
            Self::Retained { payload } => leaves.push((&payload.contract, &payload.schema)),
            Self::NonEmptyFanOutJoin { lane_success, .. } => {
                lane_success.collect_retained_leaves(leaves);
            }
        }
    }
}

fn canonical<T: Serialize>(value: &T) -> Result<PlainCanonicalJsonBytes> {
    let json =
        serde_json::to_string(value).map_err(|error| SpecError::Contract(error.to_string()))?;
    PlainCanonicalJsonBytes::from_json_str(&json)
        .map_err(|error| SpecError::Contract(error.to_string()))
}

fn domain_digest(domain: &str, canonical: &[u8]) -> mfm_ids::DigestBytes {
    let mut bytes = Vec::with_capacity(domain.len() + canonical.len() + 1);
    bytes.extend_from_slice(domain.as_bytes());
    bytes.push(0);
    bytes.extend_from_slice(canonical);
    sha256_digest_bytes(&bytes)
}

fn structured_content_ref(
    schema_name: &str,
    canonical: &PlainCanonicalJsonBytes,
) -> Result<ContentRef> {
    let schema_digest =
        sha256_digest_bytes(format!("mfm.structured-schema.v1:{schema_name}:1").as_bytes());
    let schema = SchemaId::new(
        schema_name,
        "1",
        DigestAlgorithm::Sha256JcsV1,
        schema_digest,
    )?;
    let content_digest = ContentDigest::from_digest(
        DigestAlgorithm::Sha256V1,
        sha256_digest_bytes(canonical.as_bytes()),
    );
    ContentRef::new(schema, content_digest).map_err(Into::into)
}

#[cfg(test)]
#[path = "structured_normalization_tests.rs"]
mod normalization_tests;
