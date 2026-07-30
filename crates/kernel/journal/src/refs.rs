use mfm_canonical::CanonicalValue;
use mfm_ids::{ContentRef, JournalRecordHash, NodeId, RunId, SemanticDigest, SpecHash, StoreEpoch};

use super::codec::{
    cv_content_ref, cv_decimal, cv_string, define_schema_value, node_id_field, object,
    required_field, run_id_field, spec_hash_field, store_epoch_field, string_field, u32_field,
    unsigned_field,
};
use super::{PersistedJournalValue, Result, ValueRef};

/// Semantic phase of an admitted run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RunPhase {
    /// The run may still commit semantic transitions.
    Open,
    /// The run has committed its terminal transition and closure.
    Closed,
}

impl RunPhase {
    /// Returns the frozen canonical spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Closed => "closed",
        }
    }

    pub(crate) fn parse(value: &str) -> Result<Self> {
        match value {
            "open" => Ok(Self::Open),
            "closed" => Ok(Self::Closed),
            _ => Err(super::JournalError::Projection),
        }
    }
}

/// Semantic phase of one certified node occurrence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum NodePhase {
    /// The occurrence has not committed a request or settlement.
    Unstarted,
    /// The occurrence has committed an effect request and awaits settlement.
    AwaitingEffect,
    /// The occurrence has committed its sole settlement.
    Terminal,
}

impl NodePhase {
    /// Returns the frozen canonical spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unstarted => "unstarted",
            Self::AwaitingEffect => "awaiting_effect",
            Self::Terminal => "terminal",
        }
    }

    pub(crate) fn parse(value: &str) -> Result<Self> {
        match value {
            "unstarted" => Ok(Self::Unstarted),
            "awaiting_effect" => Ok(Self::AwaitingEffect),
            "terminal" => Ok(Self::Terminal),
            _ => Err(super::JournalError::Projection),
        }
    }
}

/// Logical transition slot occupied by a node record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TransitionSlot {
    /// The effect-request slot.
    Request,
    /// The terminal settlement or dependency-skip slot.
    Settlement,
}

impl TransitionSlot {
    /// Returns the frozen canonical spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Request => "request",
            Self::Settlement => "settlement",
        }
    }

    pub(crate) fn parse(value: &str) -> Result<Self> {
        match value {
            "request" => Ok(Self::Request),
            "settlement" => Ok(Self::Settlement),
            _ => Err(super::JournalError::Projection),
        }
    }
}

define_schema_value! {
    /// Exact reference to an assigned journal record.
    pub struct RecordRef => "mfm.record-ref.v1";
    /// Exact reference to a transition record.
    pub struct TransitionRef => "mfm.transition-ref.v1";
    /// Exact reference to an authorization record.
    pub struct AuthorizationRef => "mfm.authorization-ref.v1";
    /// Exact reference to an observation record.
    pub struct ObservationRef => "mfm.observation-ref.v1";
    /// Exact reference to a closure record.
    pub struct ClosureRef => "mfm.closure-ref.v1";
    /// Exact retained input-manifest authority.
    pub struct InputManifestRef => "mfm.input-manifest-ref.v1";
    /// Lightweight admitted capability-binding reference.
    pub struct CapabilityBindingRef => "mfm.capability-binding-ref.v1";
    /// Exact output coordinate within a transition.
    pub struct OutputRef => "mfm.output-ref.v1";
    /// Typed reference to an object supplied by a closed source run.
    pub struct SourceObjectRef => "mfm.source-object-ref.v1";
    /// Closed cross-run source authority.
    pub struct CrossRunSourceRef => "mfm.cross-run-source-ref.v1";
}

/// Typed fields of a record reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordRefFields {
    /// Admitted run containing the record.
    pub run_id: RunId,
    /// Per-run commit sequence.
    pub run_sequence: u64,
    /// Record ordinal within the commit.
    pub ordinal: u32,
    /// Semantic hash of the exact record.
    pub record_hash: JournalRecordHash,
}

impl RecordRef {
    /// Constructs and annex-validates an exact record reference.
    pub fn new(
        run_id: &RunId,
        run_sequence: u64,
        ordinal: u32,
        record_hash: &JournalRecordHash,
    ) -> Result<Self> {
        Self::from_canonical_value(object([
            ("run_id", cv_string(run_id.as_str())),
            ("run_sequence", cv_decimal(run_sequence)?),
            ("ordinal", CanonicalValue::Unsigned(u64::from(ordinal))),
            ("record_hash", cv_string(record_hash.as_str())),
        ])?)
    }

    /// Projects the exact typed record-reference fields.
    pub fn fields(&self) -> Result<RecordRefFields> {
        Ok(RecordRefFields {
            run_id: run_id_field(self, "run_id")?,
            run_sequence: unsigned_field(self, "run_sequence")?,
            ordinal: u32_field(self, "ordinal")?,
            record_hash: JournalRecordHash::parse(string_field(self, "record_hash")?)?,
        })
    }
}

macro_rules! typed_record_ref {
    ($name:ident) => {
        impl $name {
            /// Constructs this typed record reference from an exact generic reference.
            pub fn new(record_ref: &RecordRef) -> Result<Self> {
                Self::from_canonical_value(record_ref.canonical_value()?)
            }

            /// Projects the generic exact record reference.
            pub fn record_ref(&self) -> Result<RecordRef> {
                RecordRef::from_canonical_value(self.canonical_value()?)
            }

            /// Projects the exact typed record-reference fields.
            pub fn fields(&self) -> Result<RecordRefFields> {
                self.record_ref()?.fields()
            }
        }
    };
}

typed_record_ref!(TransitionRef);
typed_record_ref!(AuthorizationRef);
typed_record_ref!(ObservationRef);
typed_record_ref!(ClosureRef);

/// Typed fields of an input-manifest reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputManifestRefFields {
    /// Complete producer-bound retained input-manifest authority.
    pub value_ref: ValueRef,
}

impl InputManifestRef {
    /// Constructs and validates an input-manifest reference.
    pub fn new(value_ref: &ValueRef) -> Result<Self> {
        Self::from_canonical_value(value_ref.canonical_value()?)
    }

    /// Projects the typed input-manifest fields.
    pub fn fields(&self) -> Result<InputManifestRefFields> {
        Ok(InputManifestRefFields {
            value_ref: ValueRef::from_canonical_value(self.canonical_value()?)?,
        })
    }

    /// Projects the complete producer-bound retained input-manifest authority.
    pub fn value_ref(&self) -> Result<ValueRef> {
        self.fields().map(|fields| fields.value_ref)
    }
}

impl CapabilityBindingRef {
    /// Constructs a capability-binding reference from a lightweight content reference.
    pub fn new(content_ref: &ContentRef) -> Result<Self> {
        Self::from_canonical_value(cv_content_ref(content_ref)?)
    }

    /// Projects the lightweight content reference.
    pub fn fields(&self) -> Result<ContentRef> {
        super::codec::content_ref_from_canonical(self.canonical_value()?)
    }
}

/// Typed fields of an output reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputRefFields {
    /// Producing transition.
    pub transition_ref: TransitionRef,
    /// Output ordinal within the transition settlement.
    pub output_ordinal: u32,
}

impl OutputRef {
    /// Constructs and validates an output reference.
    pub fn new(transition_ref: &TransitionRef, output_ordinal: u32) -> Result<Self> {
        Self::from_canonical_value(object([
            ("transition_ref", transition_ref.canonical_value()?),
            (
                "output_ordinal",
                CanonicalValue::Unsigned(u64::from(output_ordinal)),
            ),
        ])?)
    }

    /// Projects the typed output-reference fields.
    pub fn fields(&self) -> Result<OutputRefFields> {
        Ok(OutputRefFields {
            transition_ref: TransitionRef::from_canonical_value(required_field(
                self,
                "transition_ref",
            )?)?,
            output_ordinal: u32_field(self, "output_ordinal")?,
        })
    }
}

/// Common source-run identity projected from either cross-run source variant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrossRunSourceIdentity {
    /// Source store deployment identity.
    pub source_store_scope_id: mfm_ids::StoreScopeId,
    /// Source store epoch.
    pub source_store_epoch: StoreEpoch,
    /// Exact source admission.
    pub source_admission_ref: RecordRef,
    /// Closed source run.
    pub source_run_id: RunId,
    /// Source certified spec.
    pub source_spec_hash: SpecHash,
    /// Source node occurrence.
    pub source_node_id: NodeId,
    /// Source closure.
    pub source_closure_ref: ClosureRef,
}

/// Closed cross-run source role and all of its typed lineage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CrossRunSourceRefFields {
    /// Effective output selected from one semantically closed source node.
    EffectiveOutput {
        /// Identity shared by every source role.
        identity: CrossRunSourceIdentity,
        /// Exact effective source transition.
        effective_transition_ref: TransitionRef,
        /// Exact output within the effective transition.
        effective_output_ref: OutputRef,
    },
    /// Evidence retained from one source transition without effective-output authority.
    EvidenceOnly {
        /// Identity shared by every source role.
        identity: CrossRunSourceIdentity,
        /// Exact raw source transition.
        raw_transition_ref: TransitionRef,
        /// Exact retained result or evidence object.
        raw_result_or_evidence_ref: SourceObjectRef,
        /// Certified evidence role admitted for this source.
        certified_evidence_role_ref: ContentRef,
    },
}

impl CrossRunSourceRef {
    /// Derives the frozen redaction identity of this complete source reference.
    pub fn redaction_digest(&self) -> Result<SemanticDigest> {
        super::codec::contract()?
            .derive_cross_run_source_redaction_digest(self.validated())
            .map_err(Into::into)
    }

    /// Projects the closed source role and its complete typed lineage.
    pub fn fields(&self) -> Result<CrossRunSourceRefFields> {
        let identity = self.common_identity()?;
        match super::codec::tag(self)?.as_str() {
            "effective_output" => Ok(CrossRunSourceRefFields::EffectiveOutput {
                identity,
                effective_transition_ref: TransitionRef::from_canonical_value(required_field(
                    self,
                    "effective_transition_ref",
                )?)?,
                effective_output_ref: OutputRef::from_canonical_value(required_field(
                    self,
                    "effective_output_ref",
                )?)?,
            }),
            "evidence_only" => Ok(CrossRunSourceRefFields::EvidenceOnly {
                identity,
                raw_transition_ref: TransitionRef::from_canonical_value(required_field(
                    self,
                    "raw_transition_ref",
                )?)?,
                raw_result_or_evidence_ref: SourceObjectRef::from_canonical_value(required_field(
                    self,
                    "raw_result_or_evidence_ref",
                )?)?,
                certified_evidence_role_ref: super::codec::content_ref_field(
                    self,
                    "certified_evidence_role_ref",
                )?,
            }),
            _ => Err(super::JournalError::Projection),
        }
    }

    /// Projects the identity fields shared by every source role.
    pub fn identity(&self) -> Result<CrossRunSourceIdentity> {
        self.common_identity()
    }

    fn common_identity(&self) -> Result<CrossRunSourceIdentity> {
        Ok(CrossRunSourceIdentity {
            source_store_scope_id: super::codec::store_scope_id_field(
                self,
                "source_store_scope_id",
            )?,
            source_store_epoch: store_epoch_field(self, "source_store_epoch")?,
            source_admission_ref: RecordRef::from_canonical_value(required_field(
                self,
                "source_admission_ref",
            )?)?,
            source_run_id: run_id_field(self, "source_run_id")?,
            source_spec_hash: spec_hash_field(self, "source_spec_hash")?,
            source_node_id: node_id_field(self, "source_node_id")?,
            source_closure_ref: ClosureRef::from_canonical_value(required_field(
                self,
                "source_closure_ref",
            )?)?,
        })
    }
}
