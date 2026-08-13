//! The final three-family run journal contract.
//!
//! This module is intentionally independent from Runtime and from any backend. It owns strict
//! bytes, immutable identities, and the logical record algebra; Store owns qualification and
//! reduction.

use mfm_canonical::{raw_content_digest, PlainCanonicalJsonBytes};
pub use mfm_ids::SequentialControlAddress;
use mfm_ids::{
    AppendRequestId, ContentDigest, ContentRef, DigestAlgorithm, DigestBytes, RunId, SchemaId,
    StableId, StoreEpoch, StoreScopeId, TenantScopeId,
};
use mfm_values::string_contains_secret_marker;
use serde::{Deserialize, Serialize};

/// Maximum canonical bytes in one retained frame.
pub const MAX_FRAME_BYTES: usize = 32 * 1024 * 1024;
/// Maximum frames in one run.
pub const MAX_RUN_FRAMES: usize = 65_536;
/// Maximum reachable immutable objects in one run.
pub const MAX_RUN_OBJECTS: usize = 1_048_576;
/// Maximum canonical frame bytes in one run.
pub const MAX_RUN_FRAME_BYTES: usize = 512 * 1024 * 1024;
/// Maximum retained configuration revisions.
pub const MAX_CONFIGURATION_REVISIONS: usize = 1_024;
/// Maximum canonical bytes in one configuration revision.
pub const MAX_CONFIGURATION_REVISION_BYTES: usize = 16 * 1024 * 1024;
/// Maximum cumulative canonical configuration bytes.
pub const MAX_CONFIGURATION_STREAM_BYTES: usize = 64 * 1024 * 1024;

/// Result of strict three-family journal validation.
pub type Result<T> = std::result::Result<T, JournalError>;

/// Redaction-safe error returned by the final journal contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum JournalError {
    /// Canonical JSON was malformed, non-canonical, or exceeded its frame budget.
    #[error("single-trust journal canonical input is invalid")]
    Canonical,
    /// The record shape or identity is not part of the closed contract.
    #[error("single-trust journal record shape is invalid")]
    InvalidRecord,
    /// A frame or run exceeded a fixed outer bound.
    #[error("single-trust journal capacity bound exceeded")]
    Capacity,
    /// A record's identity does not match its logical owner.
    #[error("single-trust journal identity binding is invalid")]
    Identity,
}

/// One opaque typed value identity retained in a record/object closure.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ValueRef {
    contract_ref: ContentRef,
    value_ref: ContentRef,
}

impl ValueRef {
    /// Binds a value content identity to its nominal contract.
    pub const fn new(contract_ref: ContentRef, value_ref: ContentRef) -> Self {
        Self {
            contract_ref,
            value_ref,
        }
    }

    /// Returns the nominal contract identity.
    pub const fn contract_ref(&self) -> &ContentRef {
        &self.contract_ref
    }

    /// Returns the value content identity.
    pub const fn value_ref(&self) -> &ContentRef {
        &self.value_ref
    }

    /// Returns whether the value bytes use the nominal contract's schema.
    pub fn is_schema_bound(&self) -> bool {
        self.contract_ref.schema_id() == self.value_ref.schema_id()
    }
}

/// One secret-free immutable State/capability/adapter association.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BindingDescriptor {
    state_implementation_ref: ContentRef,
    capability_contract_ref: Option<ContentRef>,
    adapter_implementation_ref: Option<ContentRef>,
    physical_target_ref: ContentRef,
    effect_domain: Option<StableId>,
    public_signer_key_instance_ref: Option<ContentRef>,
}

impl BindingDescriptor {
    /// Constructs the sole immutable binding descriptor shape.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        state_implementation_ref: ContentRef,
        capability_contract_ref: Option<ContentRef>,
        adapter_implementation_ref: Option<ContentRef>,
        physical_target_ref: ContentRef,
        effect_domain: Option<StableId>,
        public_signer_key_instance_ref: Option<ContentRef>,
    ) -> Result<Self> {
        if capability_contract_ref.is_none() != adapter_implementation_ref.is_none() {
            return Err(JournalError::InvalidRecord);
        }
        Ok(Self {
            state_implementation_ref,
            capability_contract_ref,
            adapter_implementation_ref,
            physical_target_ref,
            effect_domain,
            public_signer_key_instance_ref,
        })
    }

    /// Validates a descriptor that came from strict retained bytes.
    pub fn validate(&self) -> Result<()> {
        if self.capability_contract_ref.is_none() != self.adapter_implementation_ref.is_none() {
            return Err(JournalError::InvalidRecord);
        }
        Ok(())
    }

    /// Returns the exact State implementation identity.
    pub const fn state_implementation_ref(&self) -> &ContentRef {
        &self.state_implementation_ref
    }

    /// Returns the capability contract identity for Access, if present.
    pub const fn capability_contract_ref(&self) -> Option<&ContentRef> {
        self.capability_contract_ref.as_ref()
    }

    /// Returns the qualified adapter identity for Access, if present.
    pub const fn adapter_implementation_ref(&self) -> Option<&ContentRef> {
        self.adapter_implementation_ref.as_ref()
    }

    /// Returns the immutable physical route/target identity.
    pub const fn physical_target_ref(&self) -> &ContentRef {
        &self.physical_target_ref
    }

    /// Returns the Effect domain, if the State is an Effect.
    pub const fn effect_domain(&self) -> Option<&StableId> {
        self.effect_domain.as_ref()
    }

    /// Returns the public signer key-instance identity, if applicable.
    pub const fn public_signer_key_instance_ref(&self) -> Option<&ContentRef> {
        self.public_signer_key_instance_ref.as_ref()
    }

    /// Returns the content identity of this exact canonical binding descriptor.
    pub fn content_ref(&self) -> Result<ContentRef> {
        let schema = SchemaId::new(
            "mfm.execution-binding",
            "1",
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([0; 32]),
        )
        .map_err(|_| JournalError::Canonical)?;
        let canonical = canonical_json(self)?;
        ContentRef::new(schema, raw_content_digest(canonical.as_bytes()))
            .map_err(|_| JournalError::Canonical)
    }
}

/// One append-atomic immutable object closure member.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImmutableObject {
    object_type: StableId,
    content_ref: ContentRef,
    canonical_json: String,
}

impl ImmutableObject {
    /// Builds one object after checking its canonical, float-free bytes.
    pub fn new(
        object_type: StableId,
        content_ref: ContentRef,
        canonical_json: String,
    ) -> Result<Self> {
        let canonical =
            PlainCanonicalJsonBytes::from_canonical_json_slice(canonical_json.as_bytes())
                .map_err(|_| JournalError::Canonical)?;
        if canonical.as_str() != canonical_json
            || canonical_json.len() > MAX_FRAME_BYTES
            || content_ref.content_digest() != &raw_content_digest(canonical.as_bytes())
            || contains_secret_marker(&canonical_json)
        {
            return Err(JournalError::Canonical);
        }
        Ok(Self {
            object_type,
            content_ref,
            canonical_json,
        })
    }

    fn validate(&self) -> Result<()> {
        Self::new(
            self.object_type.clone(),
            self.content_ref.clone(),
            self.canonical_json.clone(),
        )
        .map(|_| ())
    }

    /// Returns the object kind.
    pub const fn object_type(&self) -> &StableId {
        &self.object_type
    }

    /// Returns its content identity.
    pub const fn content_ref(&self) -> &ContentRef {
        &self.content_ref
    }

    /// Returns the exact canonical bytes.
    pub fn canonical_json(&self) -> &str {
        &self.canonical_json
    }
}

fn contains_secret_marker(input: &str) -> bool {
    string_contains_secret_marker(input)
}

/// The sole genesis record of a run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunAdmitted {
    store_scope_id: StoreScopeId,
    store_epoch: StoreEpoch,
    run_id: RunId,
    tenant_scope_id: TenantScopeId,
    entry_point_id: StableId,
    program_ref: ContentRef,
    admitted_context: ValueRef,
    configuration_ref: ContentRef,
    source_refs: Vec<ContentRef>,
}

impl RunAdmitted {
    /// Constructs one singular-`C0` admission.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        store_scope_id: StoreScopeId,
        store_epoch: StoreEpoch,
        run_id: RunId,
        tenant_scope_id: TenantScopeId,
        entry_point_id: StableId,
        program_ref: ContentRef,
        admitted_context: ValueRef,
        configuration_ref: ContentRef,
        source_refs: Vec<ContentRef>,
    ) -> Result<Self> {
        if source_refs.len() > 64
            || source_refs.windows(2).any(|pair| pair[0] >= pair[1])
            || !admitted_context.is_schema_bound()
        {
            return Err(JournalError::Capacity);
        }
        Ok(Self {
            store_scope_id,
            store_epoch,
            run_id,
            tenant_scope_id,
            entry_point_id,
            program_ref,
            admitted_context,
            configuration_ref,
            source_refs,
        })
    }

    /// Validates an admission decoded from retained bytes.
    pub fn validate(&self) -> Result<()> {
        if self.source_refs.len() > 64
            || self.source_refs.windows(2).any(|pair| pair[0] >= pair[1])
            || !self.admitted_context.is_schema_bound()
        {
            return Err(JournalError::InvalidRecord);
        }
        Ok(())
    }

    /// Returns the admitted Store identity.
    pub const fn store_scope_id(&self) -> &StoreScopeId {
        &self.store_scope_id
    }

    /// Returns the immutable writer epoch.
    pub const fn store_epoch(&self) -> StoreEpoch {
        self.store_epoch
    }

    /// Returns the run identity.
    pub const fn run_id(&self) -> &RunId {
        &self.run_id
    }

    /// Returns the fixed tenant partition.
    pub const fn tenant_scope_id(&self) -> &TenantScopeId {
        &self.tenant_scope_id
    }

    /// Returns the exact entry point.
    pub const fn entry_point_id(&self) -> &StableId {
        &self.entry_point_id
    }

    /// Returns the one domain-planned `C0` identity.
    pub const fn admitted_context(&self) -> &ValueRef {
        &self.admitted_context
    }

    /// Returns the callback-free Program identity.
    pub const fn program_ref(&self) -> &ContentRef {
        &self.program_ref
    }

    /// Returns admitted source dependencies.
    pub fn source_refs(&self) -> &[ContentRef] {
        &self.source_refs
    }

    /// Returns the selected configuration identity.
    pub const fn configuration_ref(&self) -> &ContentRef {
        &self.configuration_ref
    }
}

/// One sealed Access execution mode retained with a preparation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PreparationMode {
    /// A non-mutating provider read with a bounded replacement budget.
    Read {
        /// Total attempts including the first preparation.
        total_attempt_bound: u16,
    },
    /// A provider mutation with exactly one possible entry.
    Effect,
}

impl<'de> Deserialize<'de> for PreparationMode {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
        enum WireMode {
            Read { total_attempt_bound: u16 },
            Effect {},
        }

        match WireMode::deserialize(deserializer)? {
            WireMode::Read {
                total_attempt_bound,
            } => Ok(Self::Read {
                total_attempt_bound,
            }),
            WireMode::Effect {} => Ok(Self::Effect),
        }
    }
}

impl PreparationMode {
    fn validate(&self) -> Result<()> {
        match self {
            Self::Read {
                total_attempt_bound,
            } if (1..=3).contains(total_attempt_bound) => Ok(()),
            Self::Effect => Ok(()),
            _ => Err(JournalError::InvalidRecord),
        }
    }

    /// Returns whether this mode permits another provider entry.
    pub const fn permits_replacement(&self) -> bool {
        matches!(self, Self::Read { .. })
    }

    /// Returns the total entry bound.
    pub const fn total_attempt_bound(&self) -> u16 {
        match self {
            Self::Read {
                total_attempt_bound,
            } => *total_attempt_bound,
            Self::Effect => 1,
        }
    }
}

/// One selected Access preparation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StatePrepared {
    occurrence: SequentialControlAddress,
    preparation_ordinal: u16,
    input: ValueRef,
    intent: ValueRef,
    fact_request: Option<ValueRef>,
    fact_selection: Option<ValueRef>,
    mode: PreparationMode,
    binding: BindingDescriptor,
    execution_binding_ref: ContentRef,
    replaces: Option<PreparationRef>,
    maximum_conclusion_bytes: u64,
}

impl StatePrepared {
    /// Constructs one bounded Read/Effect preparation.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        occurrence: SequentialControlAddress,
        preparation_ordinal: u16,
        input: ValueRef,
        intent: ValueRef,
        fact_request: Option<ValueRef>,
        fact_selection: Option<ValueRef>,
        mode: PreparationMode,
        binding: BindingDescriptor,
        execution_binding_ref: ContentRef,
        replaces: Option<PreparationRef>,
        maximum_conclusion_bytes: u64,
    ) -> Result<Self> {
        mode.validate()?;
        if !input.is_schema_bound()
            || !intent.is_schema_bound()
            || fact_request
                .as_ref()
                .is_some_and(|request| !request.is_schema_bound())
            || fact_selection
                .as_ref()
                .is_some_and(|selection| !selection.is_schema_bound())
            || fact_selection.is_some() && fact_request.is_none()
            || binding.validate().is_err()
            || binding.capability_contract_ref().is_none()
            || binding.adapter_implementation_ref().is_none()
            || binding.content_ref()? != execution_binding_ref
        {
            return Err(JournalError::InvalidRecord);
        }
        if maximum_conclusion_bytes == 0 || maximum_conclusion_bytes as usize > MAX_FRAME_BYTES {
            return Err(JournalError::Capacity);
        }
        if (preparation_ordinal == 0) != replaces.is_none() {
            return Err(JournalError::InvalidRecord);
        }
        Ok(Self {
            occurrence,
            preparation_ordinal,
            input,
            intent,
            fact_request,
            fact_selection,
            mode,
            binding,
            execution_binding_ref,
            replaces,
            maximum_conclusion_bytes,
        })
    }

    /// Returns the selected sequential occurrence.
    pub const fn occurrence(&self) -> &SequentialControlAddress {
        &self.occurrence
    }

    /// Returns the Store-assigned attempt ordinal.
    pub const fn preparation_ordinal(&self) -> u16 {
        self.preparation_ordinal
    }

    /// Returns the exact complete cumulative input.
    pub const fn input(&self) -> &ValueRef {
        &self.input
    }

    /// Returns the canonical capability intent.
    pub const fn intent(&self) -> &ValueRef {
        &self.intent
    }

    /// Returns the fixed prior-fact request identity, if this capability uses one.
    pub const fn fact_request(&self) -> Option<&ValueRef> {
        self.fact_request.as_ref()
    }

    /// Returns the sealed Read/Effect mode.
    pub const fn mode(&self) -> &PreparationMode {
        &self.mode
    }

    /// Returns the fixed prior-fact request identity, if this capability uses one.
    pub const fn fact_selection(&self) -> Option<&ValueRef> {
        self.fact_selection.as_ref()
    }

    /// Returns the immutable binding evidence.
    pub const fn binding(&self) -> &BindingDescriptor {
        &self.binding
    }

    /// Returns the exact immutable execution-binding identity.
    pub const fn execution_binding_ref(&self) -> &ContentRef {
        &self.execution_binding_ref
    }

    /// Returns the selected preparation this attempt replaces.
    pub const fn replaces(&self) -> Option<&PreparationRef> {
        self.replaces.as_ref()
    }

    /// Returns the complete reserved conclusion ceiling.
    pub const fn maximum_conclusion_bytes(&self) -> u64 {
        self.maximum_conclusion_bytes
    }
}

/// Exact identity of one assigned preparation record.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreparationRef {
    run_id: RunId,
    run_sequence: u64,
    record_ordinal: u32,
}

impl PreparationRef {
    /// Constructs one assigned preparation reference.
    pub const fn new(run_id: RunId, run_sequence: u64, record_ordinal: u32) -> Self {
        Self {
            run_id,
            run_sequence,
            record_ordinal,
        }
    }

    /// Returns the owning run.
    pub const fn run_id(&self) -> &RunId {
        &self.run_id
    }

    /// Returns the append sequence.
    pub const fn run_sequence(&self) -> u64 {
        self.run_sequence
    }

    /// Returns the frame record ordinal.
    pub const fn record_ordinal(&self) -> u32 {
        self.record_ordinal
    }
}

/// One append-atomic fact-publication coordinate bound to a conclusion.
///
/// The proposal-set object and proposed fact values live in the same frame object closure.
/// Journal keeps this structural product independent from the higher-level facts crate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FactPublication {
    publication_sequence: u64,
    proposal_set_ref: ValueRef,
}

impl FactPublication {
    /// Constructs one positive tenant-local publication coordinate.
    pub fn new(publication_sequence: u64, proposal_set_ref: ValueRef) -> Result<Self> {
        if publication_sequence == 0 || !proposal_set_ref.is_schema_bound() {
            return Err(JournalError::InvalidRecord);
        }
        Ok(Self {
            publication_sequence,
            proposal_set_ref,
        })
    }

    /// Returns the dense tenant-local publication sequence.
    pub const fn publication_sequence(&self) -> u64 {
        self.publication_sequence
    }

    /// Returns the content identity of the published proposal set.
    pub const fn proposal_set_ref(&self) -> &ValueRef {
        &self.proposal_set_ref
    }
}

/// One typed outcome retained by a conclusion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum StateOutcome {
    /// Complete successor context or declared terminal root result.
    Success(ValueRef),
    /// Typed fail-fast domain failure.
    Failure(ValueRef),
}

/// One selected State occurrence conclusion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[allow(clippy::large_enum_variant)]
pub enum StateConcluded {
    /// Pure conclusion; Pure never has a preparation.
    Pure {
        /// Exact selected occurrence.
        occurrence: SequentialControlAddress,
        /// Complete state result or typed failure.
        outcome: StateOutcome,
        /// Coordinate-free fact proposals emitted by a successful State.
        fact_proposals: Option<ValueRef>,
        /// Optional append-atomic fact publication.
        fact_publication: Option<FactPublication>,
    },
    /// Access conclusion bound to exactly one selected preparation.
    Access {
        /// Exact selected occurrence.
        occurrence: SequentialControlAddress,
        /// Preparation selected by the reducer.
        preparation: PreparationRef,
        /// Capability-owned accepted evidence.
        evidence: ValueRef,
        /// Complete state result or typed failure.
        outcome: StateOutcome,
        /// Coordinate-free fact proposals emitted by a successful State.
        fact_proposals: Option<ValueRef>,
        /// Optional callback-free fact selection evidence.
        fact_selection: Option<ValueRef>,
        /// Optional append-atomic fact publication.
        fact_publication: Option<FactPublication>,
    },
}

impl StateConcluded {
    /// Binds one Store-assigned fact publication coordinate to this semantic conclusion.
    ///
    /// The coordinate must refer to this conclusion's proposal set. Store is the only semantic
    /// caller in the normal append path; the check prevents a publication from being paired with
    /// another conclusion closure.
    pub fn with_fact_publication(self, publication: Option<FactPublication>) -> Result<Self> {
        if let Some(publication) = &publication {
            if self.fact_proposals() != Some(publication.proposal_set_ref()) {
                return Err(JournalError::InvalidRecord);
            }
        }
        Ok(match self {
            Self::Pure {
                occurrence,
                outcome,
                fact_proposals,
                ..
            } => Self::Pure {
                occurrence,
                outcome,
                fact_proposals,
                fact_publication: publication,
            },
            Self::Access {
                occurrence,
                preparation,
                evidence,
                outcome,
                fact_proposals,
                fact_selection,
                ..
            } => Self::Access {
                occurrence,
                preparation,
                evidence,
                outcome,
                fact_proposals,
                fact_selection,
                fact_publication: publication,
            },
        })
    }

    /// Returns the exact concluded occurrence.
    pub fn occurrence(&self) -> &SequentialControlAddress {
        match self {
            Self::Pure { occurrence, .. } | Self::Access { occurrence, .. } => occurrence,
        }
    }

    /// Returns the state outcome.
    pub const fn outcome(&self) -> &StateOutcome {
        match self {
            Self::Pure { outcome, .. } | Self::Access { outcome, .. } => outcome,
        }
    }

    /// Returns the selected Access preparation, if this is an Access conclusion.
    pub const fn preparation(&self) -> Option<&PreparationRef> {
        match self {
            Self::Pure { .. } => None,
            Self::Access { preparation, .. } => Some(preparation),
        }
    }

    /// Returns the accepted Access evidence, if this is an Access conclusion.
    pub const fn evidence(&self) -> Option<&ValueRef> {
        match self {
            Self::Pure { .. } => None,
            Self::Access { evidence, .. } => Some(evidence),
        }
    }

    /// Returns the accepted prior-fact selection, if one was retained.
    pub const fn fact_selection(&self) -> Option<&ValueRef> {
        match self {
            Self::Pure { .. } => None,
            Self::Access { fact_selection, .. } => fact_selection.as_ref(),
        }
    }

    /// Returns the coordinate-free fact proposals emitted by this conclusion, if any.
    pub const fn fact_proposals(&self) -> Option<&ValueRef> {
        match self {
            Self::Pure { fact_proposals, .. } | Self::Access { fact_proposals, .. } => {
                fact_proposals.as_ref()
            }
        }
    }

    /// Returns the append-atomic fact publication, if this conclusion publishes facts.
    pub const fn fact_publication(&self) -> Option<&FactPublication> {
        match self {
            Self::Pure {
                fact_publication, ..
            }
            | Self::Access {
                fact_publication, ..
            } => fact_publication.as_ref(),
        }
    }
}

/// The only run record families.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "payload",
    rename_all = "snake_case",
    deny_unknown_fields
)]
#[allow(clippy::large_enum_variant)]
pub enum RunRecord {
    /// Sole run genesis.
    RunAdmitted(RunAdmitted),
    /// One exact Read/Effect attempt.
    StatePrepared(StatePrepared),
    /// One exact Pure or Access conclusion.
    StateConcluded(StateConcluded),
}

/// Logical idempotency key for one run record.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RecordLogicalKey {
    /// Sole admission key.
    Admission {
        /// Exact run identity.
        run_id: RunId,
    },
    /// Preparation key; replacements use a new ordinal.
    StatePreparation {
        /// Exact sequential occurrence.
        occurrence: SequentialControlAddress,
        /// Store-assigned preparation ordinal.
        preparation_ordinal: u16,
    },
    /// One conclusion across every preparation attempt.
    StateConclusion {
        /// Exact sequential occurrence.
        occurrence: SequentialControlAddress,
    },
}

impl RunRecord {
    /// Returns the closed logical key owned by this record.
    pub fn logical_key(&self) -> RecordLogicalKey {
        match self {
            Self::RunAdmitted(value) => RecordLogicalKey::Admission {
                run_id: value.run_id.clone(),
            },
            Self::StatePrepared(value) => RecordLogicalKey::StatePreparation {
                occurrence: value.occurrence.clone(),
                preparation_ordinal: value.preparation_ordinal,
            },
            Self::StateConcluded(value) => RecordLogicalKey::StateConclusion {
                occurrence: value.occurrence().clone(),
            },
        }
    }

    /// Returns true when this record is the sole genesis family.
    pub const fn is_admission(&self) -> bool {
        matches!(self, Self::RunAdmitted(_))
    }

    /// Returns true when this record is an access preparation.
    pub const fn is_preparation(&self) -> bool {
        matches!(self, Self::StatePrepared(_))
    }

    /// Returns true when this record is a conclusion.
    pub const fn is_conclusion(&self) -> bool {
        matches!(self, Self::StateConcluded(_))
    }

    /// Returns the append-atomic fact publication attached to this conclusion, if any.
    pub const fn fact_publication(&self) -> Option<&FactPublication> {
        match self {
            Self::StateConcluded(conclusion) => conclusion.fact_publication(),
            Self::RunAdmitted(_) | Self::StatePrepared(_) => None,
        }
    }

    /// Returns the coordinate-free fact proposals emitted by this record, if any.
    pub const fn fact_proposals(&self) -> Option<&ValueRef> {
        match self {
            Self::StateConcluded(conclusion) => conclusion.fact_proposals(),
            Self::RunAdmitted(_) | Self::StatePrepared(_) => None,
        }
    }
}

/// One append-atomic frame containing exactly one semantic record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunFrame {
    run_id: RunId,
    store_scope_id: StoreScopeId,
    store_epoch: StoreEpoch,
    expected_sequence: u64,
    append_request_id: AppendRequestId,
    record: RunRecord,
    objects: Vec<ImmutableObject>,
}

impl RunFrame {
    /// Constructs one bounded append frame.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        run_id: RunId,
        store_scope_id: StoreScopeId,
        store_epoch: StoreEpoch,
        expected_sequence: u64,
        append_request_id: AppendRequestId,
        record: RunRecord,
        objects: Vec<ImmutableObject>,
    ) -> Result<Self> {
        if expected_sequence == 0 || objects.len() > MAX_RUN_OBJECTS {
            return Err(JournalError::Capacity);
        }
        if record.is_admission() != (expected_sequence == 1) {
            return Err(JournalError::InvalidRecord);
        }
        match &record {
            RunRecord::RunAdmitted(admission) => {
                if admission.run_id() != &run_id
                    || admission.store_scope_id() != &store_scope_id
                    || admission.store_epoch() != store_epoch
                    || !admission.admitted_context().is_schema_bound()
                {
                    return Err(JournalError::Identity);
                }
            }
            RunRecord::StatePrepared(_) | RunRecord::StateConcluded(_) => {}
        }
        let frame = Self {
            run_id,
            store_scope_id,
            store_epoch,
            expected_sequence,
            append_request_id,
            record,
            objects,
        };
        frame.validate()?;
        Ok(frame)
    }

    /// Validates one frame after strict deserialization or before backend append.
    pub fn validate(&self) -> Result<()> {
        if self.expected_sequence == 0 || self.objects.len() > MAX_RUN_OBJECTS {
            return Err(JournalError::Capacity);
        }
        if self.record.is_admission() != (self.expected_sequence == 1) {
            return Err(JournalError::InvalidRecord);
        }
        match &self.record {
            RunRecord::RunAdmitted(admission) => {
                if admission.run_id() != &self.run_id
                    || admission.store_scope_id() != &self.store_scope_id
                    || admission.store_epoch() != self.store_epoch
                    || !admission.admitted_context().is_schema_bound()
                {
                    return Err(JournalError::Identity);
                }
                admission.validate()?;
            }
            RunRecord::StatePrepared(prepared) => {
                if !prepared.input().is_schema_bound()
                    || !prepared.intent().is_schema_bound()
                    || prepared
                        .fact_request()
                        .is_some_and(|value| !value.is_schema_bound())
                    || prepared
                        .fact_selection()
                        .is_some_and(|value| !value.is_schema_bound())
                    || prepared.fact_request().is_some() != prepared.fact_selection().is_some()
                    || prepared.maximum_conclusion_bytes() == 0
                    || prepared.maximum_conclusion_bytes() as usize > MAX_FRAME_BYTES
                    || prepared.mode.validate().is_err()
                    || prepared.binding().validate().is_err()
                    || prepared.binding().capability_contract_ref().is_none()
                    || prepared.binding().adapter_implementation_ref().is_none()
                    || prepared
                        .binding()
                        .content_ref()
                        .map_err(|_| JournalError::InvalidRecord)?
                        != *prepared.execution_binding_ref()
                    || prepared
                        .replaces()
                        .is_some_and(|replacement| replacement.run_id() != &self.run_id)
                {
                    return Err(JournalError::InvalidRecord);
                }
            }
            RunRecord::StateConcluded(conclusion) => match conclusion {
                StateConcluded::Pure {
                    outcome,
                    fact_proposals,
                    fact_publication,
                    ..
                } => {
                    if fact_proposals
                        .as_ref()
                        .is_some_and(|proposals| !proposals.is_schema_bound())
                        || matches!(outcome, StateOutcome::Failure(_)) && fact_proposals.is_some()
                        || fact_publication.as_ref().is_some_and(|publication| {
                            fact_proposals.as_ref() != Some(publication.proposal_set_ref())
                        })
                    {
                        return Err(JournalError::InvalidRecord);
                    }
                    if fact_publication.as_ref().is_some_and(|publication| {
                        FactPublication::new(
                            publication.publication_sequence,
                            publication.proposal_set_ref.clone(),
                        )
                        .is_err()
                    }) {
                        return Err(JournalError::InvalidRecord);
                    }
                    validate_outcome(outcome)
                }
                StateConcluded::Access {
                    preparation,
                    evidence,
                    outcome,
                    fact_proposals,
                    fact_selection,
                    fact_publication,
                    ..
                } => {
                    if preparation.run_id() != &self.run_id
                        || !evidence.is_schema_bound()
                        || fact_selection
                            .as_ref()
                            .is_some_and(|selection| !selection.is_schema_bound())
                        || fact_proposals
                            .as_ref()
                            .is_some_and(|proposals| !proposals.is_schema_bound())
                        || matches!(outcome, StateOutcome::Failure(_)) && fact_proposals.is_some()
                        || fact_publication.as_ref().is_some_and(|publication| {
                            fact_proposals.as_ref() != Some(publication.proposal_set_ref())
                        })
                    {
                        return Err(JournalError::InvalidRecord);
                    }
                    if fact_publication.as_ref().is_some_and(|publication| {
                        FactPublication::new(
                            publication.publication_sequence,
                            publication.proposal_set_ref.clone(),
                        )
                        .is_err()
                    }) {
                        return Err(JournalError::InvalidRecord);
                    }
                    validate_outcome(outcome)
                }
            }?,
        }
        for object in &self.objects {
            object.validate()?;
        }
        if canonical_json(self)?.as_bytes().len() > MAX_FRAME_BYTES {
            return Err(JournalError::Capacity);
        }
        Ok(())
    }

    /// Returns the run identity.
    pub const fn run_id(&self) -> &RunId {
        &self.run_id
    }

    /// Returns the Store scope.
    pub const fn store_scope_id(&self) -> &StoreScopeId {
        &self.store_scope_id
    }

    /// Returns the writer epoch.
    pub const fn store_epoch(&self) -> StoreEpoch {
        self.store_epoch
    }

    /// Returns the one-based append sequence.
    pub const fn expected_sequence(&self) -> u64 {
        self.expected_sequence
    }

    /// Returns the physical idempotency identity.
    pub const fn append_request_id(&self) -> &AppendRequestId {
        &self.append_request_id
    }

    /// Returns the sole semantic record.
    pub const fn record(&self) -> &RunRecord {
        &self.record
    }

    /// Returns the append-atomic object closure.
    pub fn objects(&self) -> &[ImmutableObject] {
        &self.objects
    }

    /// Encodes the frame exactly once for hashing/storage.
    pub fn canonical_bytes(&self) -> Result<PlainCanonicalJsonBytes> {
        canonical_json(self)
    }

    /// Computes the recursive content address of this append and its exact predecessor head.
    pub fn head_digest(&self, previous: Option<&ContentDigest>) -> Result<ContentDigest> {
        #[derive(Serialize)]
        struct HeadMaterial<'a> {
            previous_head_digest: Option<ContentDigest>,
            run_id: &'a RunId,
            store_scope_id: &'a StoreScopeId,
            store_epoch: StoreEpoch,
            expected_sequence: u64,
            append_request_id: &'a AppendRequestId,
            candidate_digest: ContentDigest,
            record: &'a RunRecord,
            objects: &'a [ImmutableObject],
        }

        let frame_bytes = self.canonical_bytes()?;
        let material = HeadMaterial {
            previous_head_digest: previous.cloned(),
            run_id: &self.run_id,
            store_scope_id: &self.store_scope_id,
            store_epoch: self.store_epoch,
            expected_sequence: self.expected_sequence,
            append_request_id: &self.append_request_id,
            candidate_digest: raw_content_digest(frame_bytes.as_bytes()),
            record: &self.record,
            objects: &self.objects,
        };
        let canonical = canonical_json(&material)?;
        Ok(ContentDigest::from_digest(
            DigestAlgorithm::Sha256V1,
            canonical.digest_bytes(),
        ))
    }
}

fn validate_outcome(outcome: &StateOutcome) -> Result<()> {
    let value = match outcome {
        StateOutcome::Success(value) | StateOutcome::Failure(value) => value,
    };
    value
        .is_schema_bound()
        .then_some(())
        .ok_or(JournalError::InvalidRecord)
}

/// Canonicalizes a strict journal owner value without permitting floats.
pub fn canonical_json<T: Serialize>(value: &T) -> Result<PlainCanonicalJsonBytes> {
    let encoded = serde_json::to_string(value).map_err(|_| JournalError::Canonical)?;
    PlainCanonicalJsonBytes::from_json_str(&encoded).map_err(|_| JournalError::Canonical)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mfm_ids::{ContentDigest, DigestAlgorithm, SchemaId};

    fn ref_for(seed: u8) -> ContentRef {
        let schema = SchemaId::new(
            "mfm.test.value",
            "1",
            DigestAlgorithm::Sha256JcsV1,
            mfm_ids::DigestBytes::from_array([0; 32]),
        )
        .expect("schema");
        ContentRef::new(
            schema,
            ContentDigest::from_digest(
                DigestAlgorithm::Sha256V1,
                mfm_ids::DigestBytes::from_array([seed; 32]),
            ),
        )
        .expect("content ref")
    }

    #[test]
    fn exposes_exactly_three_families_and_occurrence_conclusion_key() {
        let occurrence = SequentialControlAddress::new(4, vec![1]).expect("address");
        let record = RunRecord::StateConcluded(StateConcluded::Pure {
            occurrence: occurrence.clone(),
            outcome: StateOutcome::Success(ValueRef::new(ref_for(1), ref_for(2))),
            fact_proposals: None,
            fact_publication: None,
        });
        assert!(record.is_conclusion());
        assert_eq!(
            record.logical_key(),
            RecordLogicalKey::StateConclusion { occurrence }
        );
        assert!(!record.is_admission());
        assert!(!record.is_preparation());
    }

    #[test]
    fn pure_cannot_carry_a_preparation_by_shape() {
        let occurrence = SequentialControlAddress::new(1, Vec::new()).expect("address");
        let conclusion = StateConcluded::Pure {
            occurrence,
            outcome: StateOutcome::Failure(ValueRef::new(ref_for(3), ref_for(4))),
            fact_proposals: None,
            fact_publication: None,
        };
        assert_eq!(conclusion.occurrence().declaration_ordinal(), 1);
    }

    #[test]
    fn effect_preparation_has_one_nonreplaceable_wire_shape() {
        let mode = PreparationMode::Effect;
        assert_eq!(mode.total_attempt_bound(), 1);
        assert!(!mode.permits_replacement());
        let current = serde_json::to_value(mode).expect("current Effect mode");
        assert_eq!(current, serde_json::json!({"kind": "effect"}));
        assert_eq!(
            serde_json::from_value::<PreparationMode>(current.clone()).expect("current Effect"),
            PreparationMode::Effect
        );
        for (field, value) in [
            ("absorbing", serde_json::json!(false)),
            ("total_attempt_bound", serde_json::json!(1)),
        ] {
            let mut old = current.clone();
            old.as_object_mut()
                .expect("Effect object")
                .insert(field.to_owned(), value);
            assert!(serde_json::from_value::<PreparationMode>(old).is_err());
        }
    }

    #[test]
    fn store_publication_binding_is_coordinate_checked() {
        let proposal = ValueRef::new(ref_for(30), ref_for(31));
        let conclusion = StateConcluded::Pure {
            occurrence: SequentialControlAddress::new(1, Vec::new()).expect("occurrence"),
            outcome: StateOutcome::Success(ValueRef::new(ref_for(32), ref_for(33))),
            fact_proposals: Some(proposal.clone()),
            fact_publication: None,
        };
        let publication = FactPublication::new(1, proposal.clone()).expect("publication");
        let bound = conclusion
            .clone()
            .with_fact_publication(Some(publication))
            .expect("matching publication");
        assert_eq!(
            bound
                .fact_publication()
                .map(FactPublication::proposal_set_ref),
            Some(&proposal)
        );
        assert_eq!(
            bound
                .fact_publication()
                .map(FactPublication::publication_sequence),
            Some(1)
        );
        assert!(conclusion
            .with_fact_publication(Some(
                FactPublication::new(1, ValueRef::new(ref_for(34), ref_for(35)))
                    .expect("foreign publication"),
            ))
            .is_err());
        assert!(bound
            .with_fact_publication(None)
            .expect("clear publication")
            .fact_publication()
            .is_none());
    }

    #[test]
    fn frame_rejects_a_second_genesis_or_zero_sequence() {
        let scope = StoreScopeId::new("mfm.store_scope.v1:0123456789abcdef0123456789abcdef")
            .expect("scope");
        let epoch = StoreEpoch::new(1);
        let run = RunId::parse(
            "run:sha256-jcs-v1:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        )
        .expect("run");
        let append = AppendRequestId::new("append-request-0123456789abcdef").expect("append");
        let admission = RunRecord::RunAdmitted(
            RunAdmitted::new(
                scope.clone(),
                epoch,
                run.clone(),
                TenantScopeId::new("mfm.tenant_scope.v1:0123456789abcdef0123456789abcdef")
                    .expect("tenant"),
                StableId::new("mfm.test-entry-1").expect("entry"),
                ref_for(5),
                ValueRef::new(ref_for(6), ref_for(7)),
                ref_for(8),
                Vec::new(),
            )
            .expect("admission"),
        );
        assert!(RunFrame::new(
            run.clone(),
            scope.clone(),
            epoch,
            2,
            append.clone(),
            admission,
            Vec::new()
        )
        .is_err());
        assert!(RunFrame::new(
            run,
            scope,
            epoch,
            0,
            append,
            RunRecord::StateConcluded(StateConcluded::Pure {
                occurrence: SequentialControlAddress::new(1, Vec::new()).expect("address"),
                outcome: StateOutcome::Failure(ValueRef::new(ref_for(9), ref_for(10))),
                fact_proposals: None,
                fact_publication: None,
            },),
            Vec::new()
        )
        .is_err());
    }

    #[test]
    fn access_frames_bind_preparations_and_execution_descriptors() {
        let scope = StoreScopeId::new("mfm.store_scope.v1:0123456789abcdef0123456789abcdef")
            .expect("scope");
        let epoch = StoreEpoch::new(1);
        let run = RunId::parse(
            "run:sha256-jcs-v1:2123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        )
        .expect("run");
        let occurrence = SequentialControlAddress::new(0, Vec::new()).expect("occurrence");
        let state = ref_for(11);
        let capability = ref_for(12);
        let adapter = ref_for(13);
        let target = ref_for(14);
        let binding =
            BindingDescriptor::new(state, Some(capability), Some(adapter), target, None, None)
                .expect("binding");
        let binding_ref = binding.content_ref().expect("binding ref");
        let input = ValueRef::new(ref_for(15), ref_for(16));
        let intent = ValueRef::new(ref_for(17), ref_for(18));
        let prepared = StatePrepared::new(
            occurrence.clone(),
            0,
            input,
            intent,
            None,
            None,
            PreparationMode::Read {
                total_attempt_bound: 1,
            },
            binding.clone(),
            binding_ref.clone(),
            None,
            4096,
        )
        .expect("prepared");
        assert!(RunFrame::new(
            run.clone(),
            scope.clone(),
            epoch,
            2,
            AppendRequestId::new("journal-prepared-0123456789").expect("request"),
            RunRecord::StatePrepared(prepared),
            Vec::new(),
        )
        .is_ok());

        let request_only = StatePrepared::new(
            occurrence.clone(),
            0,
            ValueRef::new(ref_for(30), ref_for(31)),
            ValueRef::new(ref_for(32), ref_for(33)),
            Some(ValueRef::new(ref_for(34), ref_for(35))),
            None,
            PreparationMode::Read {
                total_attempt_bound: 1,
            },
            binding.clone(),
            binding_ref,
            None,
            4096,
        )
        .expect("request-only preparation candidate");
        assert!(RunFrame::new(
            run.clone(),
            scope.clone(),
            epoch,
            2,
            AppendRequestId::new("journal-request-only-012345").expect("request"),
            RunRecord::StatePrepared(request_only),
            Vec::new(),
        )
        .is_err());

        let unbound = BindingDescriptor::new(ref_for(19), None, None, ref_for(20), None, None)
            .expect("unbound descriptor");
        assert!(StatePrepared::new(
            occurrence.clone(),
            0,
            ValueRef::new(ref_for(21), ref_for(22)),
            ValueRef::new(ref_for(23), ref_for(24)),
            None,
            None,
            PreparationMode::Read {
                total_attempt_bound: 1,
            },
            unbound,
            ref_for(25),
            None,
            4096,
        )
        .is_err());

        let foreign_run = RunId::parse(
            "run:sha256-jcs-v1:3123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        )
        .expect("foreign run");
        let foreign_preparation = PreparationRef::new(foreign_run, 2, 0);
        let foreign_conclusion = RunFrame::new(
            run,
            scope,
            epoch,
            3,
            AppendRequestId::new("journal-conclusion-0123456789").expect("request"),
            RunRecord::StateConcluded(StateConcluded::Access {
                occurrence,
                preparation: foreign_preparation,
                evidence: ValueRef::new(ref_for(26), ref_for(27)),
                outcome: StateOutcome::Failure(ValueRef::new(ref_for(28), ref_for(29))),
                fact_proposals: None,
                fact_selection: None,
                fact_publication: None,
            }),
            Vec::new(),
        );
        assert!(foreign_conclusion.is_err());
    }
}
