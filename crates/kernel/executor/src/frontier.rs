use std::collections::BTreeMap;

use mfm_canonical::{
    CanonicalJsonBytes, CanonicalValue, PlainCanonicalJsonBytes, ValidatedCanonicalValue,
};
use mfm_capabilities::{
    BoundaryStage, CoarseSizeClass, FailureClass, NonDomainFailure, NonDomainFailureLayer,
    SafeFailure, SafeFailureCode, SafeFailureOutcome,
};
use mfm_ids::{
    AttemptId, ContentRef, EffectKey, RequestDigest, SchemaId, SemanticDigest, StableId,
};

use crate::codec::{Decoder, Encoder, MAX_DURABLE_SNAPSHOT_BYTES};
use crate::contract::{
    canonical_object, content_ref, content_ref_value, effect_key_value, encode,
    plain_json_to_canonical_value, recoverability_contract, semantic_digest_value,
    validate_reference_effect_identifier, AllocationStateRef, ExecutorBindingRef, FencingRef,
    ResourceKeyRef, ResourceOwnershipRef, SchemaQualifiedCanonicalValue, VerifiedExecutorBinding,
};
use crate::{EffectIdentity, ExecutorError, Result};

const ATTEMPT_PREIMAGE_SCHEMA: &str = "mfm.executor-attempt-id-preimage.v1";
const ATTEMPT_DOMAIN: &str = "mfm.executor-delivery-attempt.v1";
const RETURNED_OUTCOME_SCHEMA: &str = "mfm.executor-returned-outcome.v1";
const SAFE_FAILURE_SCHEMA: &str = "mfm.executor-reference-safe-failure.v1";
const ATTEMPT_OUTCOME_SCHEMA: &str = "mfm.executor-attempt-outcome.v1";
const EFFECT_BOUND_SCHEMA: &str = "mfm.executor-effect-bound.v1";
const RESOURCE_ALLOCATED_SCHEMA: &str = "mfm.executor-resource-allocated.v1";
const ATTEMPT_AUTHORIZED_SCHEMA: &str = "mfm.executor-delivery-attempt-authorized.v1";
const ATTEMPT_OBSERVED_SCHEMA: &str = "mfm.executor-delivery-attempt-observed.v1";
pub(crate) const TERMINAL_PROOF_SCHEMA: &str = "mfm.executor-reference-terminal-proof.v1";
pub(crate) const TERMINAL_TOMBSTONE_SCHEMA: &str = "mfm.executor-terminal-tombstone.v1";
const EVIDENCE_RECORD_SCHEMA: &str = "mfm.executor-evidence-record.v1";
const RECORD_PREIMAGE_SCHEMA: &str = "mfm.executor-record-preimage.v1";
const RECORD_DOMAIN: &str = "mfm.executor-record.v1";
pub(crate) const DELIVERY_FRONTIER_SCHEMA: &str = "mfm.executor-delivery-frontier.v1";
const FRONTIER_PREIMAGE_SCHEMA: &str = "mfm.executor-frontier-preimage.v1";
const FRONTIER_DOMAIN: &str = "mfm.executor-frontier.v1";
const EVIDENCE_BOUNDS_SCHEMA: &str = "mfm.evidence-bounds.v1";
const FRONTIER_DURABLE_MAGIC: &[u8; 8] = b"MFMEFR01";

/// Closed failure codes admitted by the durable reference executor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ReferenceFailureCode {
    /// The destination permanently rejects this ledger generation.
    GenerationFenced,
    /// The destination was unavailable at a reviewed transport boundary.
    DestinationUnavailable,
    /// The semantic destination contains an incompatible request.
    RequestConflict,
    /// Access was cancelled at a reviewed boundary.
    AccessCancelled,
    /// No more specific safe classification survived.
    UnclassifiedFailure,
    /// A target response could not be retained within the reviewed result contract.
    ResultUnrepresentable,
}

impl SafeFailureCode for ReferenceFailureCode {
    fn as_str(&self) -> &'static str {
        match self {
            Self::GenerationFenced => "generation_fenced",
            Self::DestinationUnavailable => "destination_unavailable",
            Self::RequestConflict => "request_conflict",
            Self::AccessCancelled => "access_cancelled",
            Self::UnclassifiedFailure => "unclassified_failure",
            Self::ResultUnrepresentable => "result_unrepresentable",
        }
    }

    fn accepts(
        &self,
        failure_class: FailureClass,
        boundary_stage: BoundaryStage,
        coarse_size_class: Option<CoarseSizeClass>,
        has_diagnostic: bool,
    ) -> bool {
        if coarse_size_class.is_some() || has_diagnostic {
            return false;
        }
        matches!(
            (self, failure_class, boundary_stage),
            (
                Self::GenerationFenced,
                FailureClass::Authorization,
                BoundaryStage::BeforeBoundaryEntry
            ) | (
                Self::DestinationUnavailable,
                FailureClass::Transport,
                BoundaryStage::BeforeBoundaryEntry | BoundaryStage::BoundaryEntry
            ) | (
                Self::RequestConflict,
                FailureClass::Destination,
                BoundaryStage::BoundaryObservation
            ) | (
                Self::AccessCancelled,
                FailureClass::Cancellation,
                BoundaryStage::BeforeBoundaryEntry | BoundaryStage::BoundaryEntry
            ) | (
                Self::UnclassifiedFailure,
                FailureClass::Unclassified,
                BoundaryStage::BoundaryObservation
            ) | (
                Self::ResultUnrepresentable,
                FailureClass::UnrepresentableResponse,
                BoundaryStage::BoundaryObservation
            )
        )
    }
}

/// Redaction-safe failure envelope for the durable reference executor.
pub type ReferenceSafeFailure = SafeFailure<ReferenceFailureCode, ContentRef>;

/// Constructs an annex-valid reference failure tuple.
pub fn reference_safe_failure(
    safe_failure_contract_ref: ContentRef,
    stable_code: ReferenceFailureCode,
    failure_class: FailureClass,
    boundary_stage: BoundaryStage,
) -> Result<ReferenceSafeFailure> {
    let failure = SafeFailure::new(
        safe_failure_contract_ref,
        stable_code,
        failure_class,
        boundary_stage,
        None,
        None,
    )
    .map_err(|_| ExecutorError::InvalidSafeFailure)?;
    validated_safe_failure(&failure)?;
    Ok(failure)
}

/// Reconstructs and verifies one persisted reference-executor failure tuple.
///
/// This is the sole public decoder for journal observation validation. It admits neither
/// diagnostics nor coarse sizes and enforces the exact outcome-specific reference relation.
#[allow(clippy::too_many_arguments)]
pub fn verify_reference_safe_failure_tuple(
    safe_failure_contract_ref: ContentRef,
    stable_code: &StableId,
    outcome: SafeFailureOutcome,
    failure_class: FailureClass,
    boundary_stage: BoundaryStage,
    coarse_size_class: Option<CoarseSizeClass>,
    has_diagnostic: bool,
) -> Result<ReferenceSafeFailure> {
    if coarse_size_class.is_some() || has_diagnostic {
        return Err(ExecutorError::InvalidSafeFailure);
    }
    let code = match stable_code.as_str() {
        "generation_fenced" => ReferenceFailureCode::GenerationFenced,
        "destination_unavailable" => ReferenceFailureCode::DestinationUnavailable,
        "request_conflict" => ReferenceFailureCode::RequestConflict,
        "access_cancelled" => ReferenceFailureCode::AccessCancelled,
        "unclassified_failure" => ReferenceFailureCode::UnclassifiedFailure,
        "result_unrepresentable" => ReferenceFailureCode::ResultUnrepresentable,
        _ => return Err(ExecutorError::InvalidSafeFailure),
    };
    let failure = reference_safe_failure(
        safe_failure_contract_ref,
        code,
        failure_class,
        boundary_stage,
    )?;
    validate_safe_failure_outcome(&failure, outcome)?;
    Ok(failure)
}

fn validate_safe_failure_outcome(
    failure: &ReferenceSafeFailure,
    outcome: SafeFailureOutcome,
) -> Result<()> {
    let legal = matches!(
        (
            outcome,
            failure.stable_code(),
            failure.failure_class(),
            failure.boundary_stage()
        ),
        (
            SafeFailureOutcome::DidNotEnter,
            ReferenceFailureCode::GenerationFenced,
            FailureClass::Authorization,
            BoundaryStage::BeforeBoundaryEntry
        ) | (
            SafeFailureOutcome::DidNotEnter,
            ReferenceFailureCode::DestinationUnavailable,
            FailureClass::Transport,
            BoundaryStage::BeforeBoundaryEntry
        ) | (
            SafeFailureOutcome::DidNotEnter,
            ReferenceFailureCode::AccessCancelled,
            FailureClass::Cancellation,
            BoundaryStage::BeforeBoundaryEntry
        ) | (
            SafeFailureOutcome::Indeterminate,
            ReferenceFailureCode::DestinationUnavailable,
            FailureClass::Transport,
            BoundaryStage::BoundaryEntry
        ) | (
            SafeFailureOutcome::Indeterminate,
            ReferenceFailureCode::RequestConflict,
            FailureClass::Destination,
            BoundaryStage::BoundaryObservation
        ) | (
            SafeFailureOutcome::Indeterminate,
            ReferenceFailureCode::AccessCancelled,
            FailureClass::Cancellation,
            BoundaryStage::BoundaryEntry
        ) | (
            SafeFailureOutcome::Indeterminate,
            ReferenceFailureCode::UnclassifiedFailure,
            FailureClass::Unclassified,
            BoundaryStage::BoundaryObservation
        ) | (
            SafeFailureOutcome::Indeterminate,
            ReferenceFailureCode::ResultUnrepresentable,
            FailureClass::UnrepresentableResponse,
            BoundaryStage::BoundaryObservation
        )
    );
    if !legal {
        return Err(ExecutorError::InvalidSafeFailure);
    }
    Ok(())
}

fn validated_safe_failure(failure: &ReferenceSafeFailure) -> Result<ValidatedCanonicalValue> {
    encode(
        SAFE_FAILURE_SCHEMA,
        &canonical_object([
            (
                "boundary_stage",
                CanonicalValue::String(failure.boundary_stage().as_str().to_owned()),
            ),
            ("coarse_size_class", CanonicalValue::Null),
            ("diagnostic_ref", CanonicalValue::Null),
            (
                "failure_class",
                CanonicalValue::String(failure.failure_class().as_str().to_owned()),
            ),
            (
                "safe_failure_contract_ref",
                content_ref_value(failure.safe_failure_contract_ref())?,
            ),
            (
                "stable_code",
                CanonicalValue::String(failure.stable_code().as_str().to_owned()),
            ),
            (
                "version",
                CanonicalValue::String(SAFE_FAILURE_SCHEMA.to_owned()),
            ),
        ])?,
    )
}

pub(crate) fn validated_safe_failure_for_result(
    failure: &ReferenceSafeFailure,
) -> Result<ValidatedCanonicalValue> {
    validated_safe_failure(failure)
}

/// Safe persisted outcome returned by a target operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReturnedOutcome {
    safe_result_ref: Box<ContentRef>,
    safe_result: SchemaQualifiedCanonicalValue,
}

impl ReturnedOutcome {
    /// Constructs one exact schema-qualified reviewed safe result.
    pub fn new(safe_result: SchemaQualifiedCanonicalValue) -> Result<Self> {
        let safe_result_ref = Box::new(safe_result.reference()?);
        let outcome = Self {
            safe_result_ref,
            safe_result,
        };
        outcome.validated()?;
        Ok(outcome)
    }

    /// Returns the reviewed safe result identity.
    pub const fn safe_result_ref(&self) -> &ContentRef {
        &self.safe_result_ref
    }

    /// Returns the exact retained safe-result object.
    pub const fn safe_result(&self) -> &SchemaQualifiedCanonicalValue {
        &self.safe_result
    }

    /// Returns the exact canonical returned-outcome object.
    pub fn validated(&self) -> Result<ValidatedCanonicalValue> {
        encode(
            RETURNED_OUTCOME_SCHEMA,
            &canonical_object([(
                "safe_result_ref",
                content_ref_value(self.safe_result_ref())?,
            )])?,
        )
    }
}

/// Candidate safe outcome of one authorized target operation.
///
/// The representation is opaque so callers cannot bypass the reviewed
/// constructors. Constructors establish structural safety; the ledger's
/// affine completion seal additionally binds the exact executor coordinates
/// before observation.
///
/// ```compile_fail
/// # use mfm_executor::{DeliveryAttemptOutcome, ReturnedOutcome};
/// # fn cannot_construct(returned: ReturnedOutcome) {
/// let _ = DeliveryAttemptOutcome::Returned(returned);
/// # }
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeliveryAttemptOutcome {
    kind: DeliveryAttemptOutcomeKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum DeliveryAttemptOutcomeKind {
    Returned(ReturnedOutcome),
    DidNotEnter(ReferenceSafeFailure),
    Indeterminate(ReferenceSafeFailure),
    NonDomainFailure(mfm_capabilities::NonDomainFailure),
}

impl DeliveryAttemptOutcome {
    /// Constructs a structurally reviewed returned-outcome candidate.
    ///
    /// The exact admitted domain-evidence schema is checked when fresh target
    /// authority is consumed and again whenever delivery history is refolded.
    pub fn returned(safe_result: SchemaQualifiedCanonicalValue) -> Result<Self> {
        Ok(Self {
            kind: DeliveryAttemptOutcomeKind::Returned(ReturnedOutcome::new(safe_result)?),
        })
    }

    /// Constructs a structurally reviewed did-not-enter candidate.
    ///
    /// Outcome-specific tuple compatibility and the exact safe-failure
    /// contract are checked by the affine completion seal.
    pub fn did_not_enter(failure: ReferenceSafeFailure) -> Result<Self> {
        validated_safe_failure(&failure)?;
        Ok(Self {
            kind: DeliveryAttemptOutcomeKind::DidNotEnter(failure),
        })
    }

    /// Constructs a structurally reviewed indeterminate candidate.
    ///
    /// Outcome-specific tuple compatibility and the exact safe-failure
    /// contract are checked by the affine completion seal.
    pub fn indeterminate(failure: ReferenceSafeFailure) -> Result<Self> {
        validated_safe_failure(&failure)?;
        Ok(Self {
            kind: DeliveryAttemptOutcomeKind::Indeterminate(failure),
        })
    }

    /// Constructs a structurally reviewed audit-only failure candidate.
    ///
    /// The globally closed status/disposition relation is already owned by
    /// [`mfm_capabilities::NonDomainFailure`]. Executor-target layer legality
    /// is checked by the affine completion seal.
    pub fn non_domain_failure(failure: NonDomainFailure) -> Result<Self> {
        let canonical = failure
            .canonical_value()
            .map_err(|_| ExecutorError::InvalidSafeFailure)?;
        if NonDomainFailure::from_canonical_value(&canonical)
            .map_err(|_| ExecutorError::InvalidSafeFailure)?
            != failure
        {
            return Err(ExecutorError::InvalidSafeFailure);
        }
        Ok(Self {
            kind: DeliveryAttemptOutcomeKind::NonDomainFailure(failure),
        })
    }

    /// Returns the returned target result when present.
    pub const fn returned_outcome(&self) -> Option<&ReturnedOutcome> {
        match &self.kind {
            DeliveryAttemptOutcomeKind::Returned(outcome) => Some(outcome),
            DeliveryAttemptOutcomeKind::DidNotEnter(_)
            | DeliveryAttemptOutcomeKind::Indeterminate(_)
            | DeliveryAttemptOutcomeKind::NonDomainFailure(_) => None,
        }
    }

    /// Returns the did-not-enter failure candidate when present.
    pub const fn did_not_enter_failure(&self) -> Option<&ReferenceSafeFailure> {
        match &self.kind {
            DeliveryAttemptOutcomeKind::DidNotEnter(failure) => Some(failure),
            DeliveryAttemptOutcomeKind::Returned(_)
            | DeliveryAttemptOutcomeKind::Indeterminate(_)
            | DeliveryAttemptOutcomeKind::NonDomainFailure(_) => None,
        }
    }

    /// Returns the indeterminate failure candidate when present.
    pub const fn indeterminate_failure(&self) -> Option<&ReferenceSafeFailure> {
        match &self.kind {
            DeliveryAttemptOutcomeKind::Indeterminate(failure) => Some(failure),
            DeliveryAttemptOutcomeKind::Returned(_)
            | DeliveryAttemptOutcomeKind::DidNotEnter(_)
            | DeliveryAttemptOutcomeKind::NonDomainFailure(_) => None,
        }
    }

    /// Returns the audit-only failure candidate when present.
    pub const fn non_domain_failure_value(&self) -> Option<&NonDomainFailure> {
        match &self.kind {
            DeliveryAttemptOutcomeKind::NonDomainFailure(failure) => Some(failure),
            DeliveryAttemptOutcomeKind::Returned(_)
            | DeliveryAttemptOutcomeKind::DidNotEnter(_)
            | DeliveryAttemptOutcomeKind::Indeterminate(_) => None,
        }
    }

    pub(crate) fn validate_for_binding(&self, binding: &VerifiedExecutorBinding) -> Result<()> {
        self.validate_for_contract(
            binding.contract().safe_failure_contract_ref(),
            binding
                .contract()
                .retained_closure_contract()
                .domain_evidence_contract()
                .schema_id(),
        )
    }

    pub(crate) fn validate_for_contract(
        &self,
        safe_failure_contract_ref: &ContentRef,
        domain_evidence_schema_id: &SchemaId,
    ) -> Result<()> {
        match &self.kind {
            DeliveryAttemptOutcomeKind::Returned(returned) => {
                let reconstructed = ReturnedOutcome::new(returned.safe_result().clone())?;
                if &reconstructed != returned
                    || returned.safe_result().schema_id() != domain_evidence_schema_id
                {
                    return Err(ExecutorError::SchemaReferenceMismatch);
                }
            }
            DeliveryAttemptOutcomeKind::DidNotEnter(failure) => {
                validate_bound_safe_failure(
                    failure,
                    SafeFailureOutcome::DidNotEnter,
                    safe_failure_contract_ref,
                )?;
            }
            DeliveryAttemptOutcomeKind::Indeterminate(failure) => {
                validate_bound_safe_failure(
                    failure,
                    SafeFailureOutcome::Indeterminate,
                    safe_failure_contract_ref,
                )?;
            }
            DeliveryAttemptOutcomeKind::NonDomainFailure(failure) => {
                let fields = failure.fields();
                let reconstructed =
                    NonDomainFailure::new(fields.entry_status, fields.disposition, fields.code)
                        .map_err(|_| ExecutorError::InvalidSafeFailure)?;
                if &reconstructed != failure {
                    return Err(ExecutorError::InvalidSafeFailure);
                }
                reconstructed
                    .validate_layer(NonDomainFailureLayer::ExecutorTarget)
                    .map_err(|_| ExecutorError::InvalidSafeFailure)?;
            }
        }
        Ok(())
    }

    const fn kind(&self) -> &DeliveryAttemptOutcomeKind {
        &self.kind
    }

    fn validated(&self) -> Result<ValidatedCanonicalValue> {
        let value = match &self.kind {
            DeliveryAttemptOutcomeKind::Returned(outcome) => canonical_object([
                ("kind", CanonicalValue::String("returned".to_owned())),
                (
                    "returned_outcome",
                    outcome
                        .validated()?
                        .canonical_value()
                        .map_err(crate::contract::contract_error)?,
                ),
            ])?,
            DeliveryAttemptOutcomeKind::DidNotEnter(failure) => canonical_object([
                ("kind", CanonicalValue::String("did_not_enter".to_owned())),
                (
                    "safe_failure",
                    validated_safe_failure(failure)?
                        .canonical_value()
                        .map_err(crate::contract::contract_error)?,
                ),
            ])?,
            DeliveryAttemptOutcomeKind::Indeterminate(failure) => canonical_object([
                ("kind", CanonicalValue::String("indeterminate".to_owned())),
                (
                    "safe_failure",
                    validated_safe_failure(failure)?
                        .canonical_value()
                        .map_err(crate::contract::contract_error)?,
                ),
            ])?,
            DeliveryAttemptOutcomeKind::NonDomainFailure(failure) => canonical_object([
                (
                    "kind",
                    CanonicalValue::String("non_domain_failure".to_owned()),
                ),
                (
                    "non_domain_failure",
                    failure
                        .canonical_value()
                        .map_err(|_| ExecutorError::CanonicalEncoding)?,
                ),
            ])?,
        };
        encode(ATTEMPT_OUTCOME_SCHEMA, &value)
    }
}

fn validate_bound_safe_failure(
    failure: &ReferenceSafeFailure,
    outcome: SafeFailureOutcome,
    expected_contract_ref: &ContentRef,
) -> Result<()> {
    let reconstructed = reference_safe_failure(
        failure.safe_failure_contract_ref().clone(),
        *failure.stable_code(),
        failure.failure_class(),
        failure.boundary_stage(),
    )?;
    if &reconstructed != failure || failure.safe_failure_contract_ref() != expected_contract_ref {
        return Err(ExecutorError::InvalidSafeFailure);
    }
    validate_safe_failure_outcome(failure, outcome)
}

/// Derives one immutable delivery-attempt identity through the frozen domain.
pub fn derive_attempt_id(
    identity: &EffectIdentity,
    attempt_ordinal: u32,
    target_operation_ref: &ContentRef,
) -> Result<AttemptId> {
    if attempt_ordinal >= 64 {
        return Err(ExecutorError::EvidenceBoundsExhausted);
    }
    let preimage = encode(
        ATTEMPT_PREIMAGE_SCHEMA,
        &canonical_object([
            (
                "attempt_ordinal",
                CanonicalValue::Unsigned(u64::from(attempt_ordinal)),
            ),
            ("effect_key", effect_key_value(identity.effect_key())),
            (
                "executor_binding_ref",
                identity.executor_binding_ref().canonical_value()?,
            ),
            (
                "request_digest",
                semantic_digest_value(identity.request_digest().semantic_digest()),
            ),
            (
                "target_operation_ref",
                content_ref_value(target_operation_ref)?,
            ),
        ])?,
    )?;
    let digest = recoverability_contract()?
        .semantic_digest(ATTEMPT_DOMAIN, &preimage)
        .map_err(crate::contract::contract_error)?;
    Ok(AttemptId::from_semantic_digest(digest))
}

/// Exact link from one returned observation to its terminal proof.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReferenceTerminalProof {
    attempt_id: AttemptId,
    returned_outcome: ReturnedOutcome,
    returned_observation_ref: ContentRef,
}

impl ReferenceTerminalProof {
    /// Constructs and annex-validates an exact-attempt terminal proof.
    pub fn new(
        attempt_id: AttemptId,
        returned_outcome: ReturnedOutcome,
        returned_observation_ref: ContentRef,
    ) -> Result<Self> {
        let proof = Self {
            attempt_id,
            returned_outcome,
            returned_observation_ref,
        };
        proof.validated()?;
        Ok(proof)
    }

    /// Returns the exact returned attempt.
    pub const fn attempt_id(&self) -> &AttemptId {
        &self.attempt_id
    }

    /// Returns the exact returned outcome.
    pub const fn returned_outcome(&self) -> &ReturnedOutcome {
        &self.returned_outcome
    }

    /// Returns the exact returned-observation identity.
    pub const fn returned_observation_ref(&self) -> &ContentRef {
        &self.returned_observation_ref
    }

    /// Returns the exact canonical proof object.
    pub fn validated(&self) -> Result<ValidatedCanonicalValue> {
        encode(
            TERMINAL_PROOF_SCHEMA,
            &canonical_object([
                (
                    "attempt_id",
                    CanonicalValue::String(self.attempt_id.as_str().to_owned()),
                ),
                (
                    "returned_observation_ref",
                    content_ref_value(&self.returned_observation_ref)?,
                ),
                (
                    "returned_outcome",
                    self.returned_outcome
                        .validated()?
                        .canonical_value()
                        .map_err(crate::contract::contract_error)?,
                ),
                (
                    "version",
                    CanonicalValue::String(TERMINAL_PROOF_SCHEMA.to_owned()),
                ),
            ])?,
        )
    }

    /// Returns the exact proof content identity.
    pub fn reference(&self) -> Result<ContentRef> {
        content_ref(&self.validated()?)
    }
}

/// Immutable terminal operation and outcome commitment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalTombstone {
    external_operation_identity: String,
    terminal_outcome: String,
    terminal_proof: ReferenceTerminalProof,
}

impl TerminalTombstone {
    /// Constructs and annex-validates a reference-executor tombstone.
    pub fn new(
        external_operation_identity: impl Into<String>,
        terminal_outcome: impl Into<String>,
        terminal_proof: ReferenceTerminalProof,
    ) -> Result<Self> {
        let external_operation_identity = external_operation_identity.into();
        validate_reference_effect_identifier(&external_operation_identity)?;
        let tombstone = Self {
            external_operation_identity,
            terminal_outcome: terminal_outcome.into(),
            terminal_proof,
        };
        tombstone.validated()?;
        Ok(tombstone)
    }

    /// Returns the fixed destination-native operation identity.
    pub fn external_operation_identity(&self) -> &str {
        &self.external_operation_identity
    }

    /// Returns the fixed terminal outcome.
    pub fn terminal_outcome(&self) -> &str {
        &self.terminal_outcome
    }

    /// Returns the exact-attempt terminal proof.
    pub const fn terminal_proof(&self) -> &ReferenceTerminalProof {
        &self.terminal_proof
    }

    /// Returns the exact canonical tombstone object.
    pub fn validated(&self) -> Result<ValidatedCanonicalValue> {
        encode(
            TERMINAL_TOMBSTONE_SCHEMA,
            &canonical_object([
                (
                    "external_operation_identity",
                    CanonicalValue::String(self.external_operation_identity.clone()),
                ),
                (
                    "terminal_outcome",
                    CanonicalValue::String(self.terminal_outcome.clone()),
                ),
                (
                    "terminal_proof_ref",
                    content_ref_value(&self.terminal_proof.reference()?)?,
                ),
            ])?,
        )
    }

    /// Returns the exact tombstone content identity.
    pub fn reference(&self) -> Result<TerminalTombstoneRef> {
        Ok(TerminalTombstoneRef(content_ref(&self.validated()?)?))
    }
}

/// Content identity of one exact retained terminal tombstone.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TerminalTombstoneRef(ContentRef);

impl TerminalTombstoneRef {
    /// Returns the lightweight tombstone content identity.
    pub const fn as_content_ref(&self) -> &ContentRef {
        &self.0
    }

    /// Reconstructs a tombstone reference after checking its exact schema.
    pub fn from_content_ref(value: ContentRef) -> Result<Self> {
        crate::contract::require_schema_ref(&value, TERMINAL_TOMBSTONE_SCHEMA)?;
        Ok(Self(value))
    }
}

/// Closed immutable executor evidence-record algebra.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecutorEvidenceRecord {
    /// Initial immutable effect/request binding.
    EffectBound {
        /// Exact executor binding.
        executor_binding_ref: ExecutorBindingRef,
        /// Kernel-derived effect key.
        effect_key: EffectKey,
        /// Immutable request digest.
        request_digest: RequestDigest,
    },
    /// Durable typed cross-effect resource allocation.
    ResourceAllocated(Box<ResourceAllocatedRecord>),
    /// Positively acknowledged authority for zero or one target entry.
    DeliveryAttemptAuthorized {
        /// Monotonic ordinal within this effect stream.
        attempt_ordinal: u32,
        /// Deterministically derived attempt identity.
        attempt_id: AttemptId,
        /// Reviewed target operation family.
        target_operation_ref: ContentRef,
    },
    /// Surviving safe observation linked to one authorization.
    DeliveryAttemptObserved {
        /// Exact authorized attempt.
        attempt_id: AttemptId,
        /// Safe returned or failure outcome.
        outcome: DeliveryAttemptOutcome,
    },
    /// Immutable terminal operation/outcome commitment.
    TerminalTombstone(TerminalTombstone),
}

/// Exact reviewed fields of one durable cross-effect resource allocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceAllocatedRecord {
    resource_ownership_ref: ResourceOwnershipRef,
    resource_key_ref: ResourceKeyRef,
    typed_allocation_state_ref: AllocationStateRef,
    policy_ref: ContentRef,
    policy_configuration_ref: ContentRef,
    fencing_ref: Option<FencingRef>,
}

impl ResourceAllocatedRecord {
    pub(crate) fn new(
        resource_ownership_ref: ResourceOwnershipRef,
        resource_key_ref: ResourceKeyRef,
        typed_allocation_state_ref: AllocationStateRef,
        policy_ref: ContentRef,
        policy_configuration_ref: ContentRef,
        fencing_ref: Option<FencingRef>,
    ) -> Self {
        Self {
            resource_ownership_ref,
            resource_key_ref,
            typed_allocation_state_ref,
            policy_ref,
            policy_configuration_ref,
            fencing_ref,
        }
    }

    /// Returns the exact admitted resource owner.
    pub const fn resource_ownership_ref(&self) -> &ResourceOwnershipRef {
        &self.resource_ownership_ref
    }

    /// Returns the typed external-resource key.
    pub const fn resource_key_ref(&self) -> &ResourceKeyRef {
        &self.resource_key_ref
    }

    /// Returns the reviewed allocation-state commitment.
    pub const fn typed_allocation_state_ref(&self) -> &AllocationStateRef {
        &self.typed_allocation_state_ref
    }

    /// Returns the exact selected resource policy.
    pub const fn policy_ref(&self) -> &ContentRef {
        &self.policy_ref
    }

    /// Returns the exact selected policy configuration.
    pub const fn policy_configuration_ref(&self) -> &ContentRef {
        &self.policy_configuration_ref
    }

    /// Returns optional destination-enforced fencing evidence.
    pub const fn fencing_ref(&self) -> Option<&FencingRef> {
        self.fencing_ref.as_ref()
    }
}

impl ExecutorEvidenceRecord {
    /// Returns the exact canonical wrapper object retained in a frontier.
    pub fn validated(&self) -> Result<ValidatedCanonicalValue> {
        let (kind, record) = match self {
            Self::EffectBound {
                executor_binding_ref,
                effect_key,
                request_digest,
            } => (
                "effect_bound",
                encode(
                    EFFECT_BOUND_SCHEMA,
                    &canonical_object([
                        ("effect_key", effect_key_value(effect_key)),
                        (
                            "executor_binding_ref",
                            executor_binding_ref.canonical_value()?,
                        ),
                        (
                            "request_digest",
                            semantic_digest_value(request_digest.semantic_digest()),
                        ),
                    ])?,
                )?,
            ),
            Self::ResourceAllocated(record) => {
                let mut fields = vec![
                    (
                        "policy_configuration_ref".to_owned(),
                        content_ref_value(&record.policy_configuration_ref)?,
                    ),
                    (
                        "policy_ref".to_owned(),
                        content_ref_value(&record.policy_ref)?,
                    ),
                    (
                        "resource_key_ref".to_owned(),
                        record.resource_key_ref.canonical_value()?,
                    ),
                    (
                        "resource_ownership_ref".to_owned(),
                        record.resource_ownership_ref.canonical_value()?,
                    ),
                    (
                        "typed_allocation_state_ref".to_owned(),
                        record.typed_allocation_state_ref.canonical_value()?,
                    ),
                ];
                if let Some(fencing_ref) = &record.fencing_ref {
                    fields.push(("fencing_ref".to_owned(), fencing_ref.canonical_value()?));
                }
                (
                    "resource_allocated",
                    encode(RESOURCE_ALLOCATED_SCHEMA, &object(fields)?)?,
                )
            }
            Self::DeliveryAttemptAuthorized {
                attempt_ordinal,
                attempt_id,
                target_operation_ref,
            } => (
                "delivery_attempt_authorized",
                encode(
                    ATTEMPT_AUTHORIZED_SCHEMA,
                    &canonical_object([
                        (
                            "attempt_id",
                            CanonicalValue::String(attempt_id.as_str().to_owned()),
                        ),
                        (
                            "attempt_ordinal",
                            CanonicalValue::Unsigned(u64::from(*attempt_ordinal)),
                        ),
                        (
                            "target_operation_ref",
                            content_ref_value(target_operation_ref)?,
                        ),
                    ])?,
                )?,
            ),
            Self::DeliveryAttemptObserved {
                attempt_id,
                outcome,
            } => (
                "delivery_attempt_observed",
                encode(
                    ATTEMPT_OBSERVED_SCHEMA,
                    &canonical_object([
                        (
                            "attempt_id",
                            CanonicalValue::String(attempt_id.as_str().to_owned()),
                        ),
                        (
                            "outcome",
                            outcome
                                .validated()?
                                .canonical_value()
                                .map_err(crate::contract::contract_error)?,
                        ),
                    ])?,
                )?,
            ),
            Self::TerminalTombstone(tombstone) => ("terminal_tombstone", tombstone.validated()?),
        };
        encode(
            EVIDENCE_RECORD_SCHEMA,
            &canonical_object([
                ("kind", CanonicalValue::String(kind.to_owned())),
                (
                    "record",
                    record
                        .canonical_value()
                        .map_err(crate::contract::contract_error)?,
                ),
            ])?,
        )
    }

    /// Returns the semantic identity of this exact record wrapper.
    pub fn reference(&self) -> Result<ExecutorLedgerRecordRef> {
        let record = self.validated()?;
        let preimage = encode(
            RECORD_PREIMAGE_SCHEMA,
            &canonical_object([(
                "record",
                record
                    .canonical_value()
                    .map_err(crate::contract::contract_error)?,
            )])?,
        )?;
        let digest = recoverability_contract()?
            .semantic_digest(RECORD_DOMAIN, &preimage)
            .map_err(crate::contract::contract_error)?;
        Ok(ExecutorLedgerRecordRef(digest))
    }

    /// Returns the exact observed-record content identity when applicable.
    pub fn observed_content_ref(&self) -> Result<Option<ContentRef>> {
        match self {
            Self::DeliveryAttemptObserved {
                attempt_id,
                outcome,
            } => {
                let observed = encode(
                    ATTEMPT_OBSERVED_SCHEMA,
                    &canonical_object([
                        (
                            "attempt_id",
                            CanonicalValue::String(attempt_id.as_str().to_owned()),
                        ),
                        (
                            "outcome",
                            outcome
                                .validated()?
                                .canonical_value()
                                .map_err(crate::contract::contract_error)?,
                        ),
                    ])?,
                )?;
                Ok(Some(content_ref(&observed)?))
            }
            _ => Ok(None),
        }
    }

    fn content_objects(&self) -> Result<Vec<SchemaQualifiedCanonicalValue>> {
        let mut values = vec![SchemaQualifiedCanonicalValue::from_validated(
            &self.validated()?,
        )?];
        match self {
            Self::DeliveryAttemptObserved { outcome, .. } => {
                append_outcome_content(&mut values, outcome)?;
            }
            Self::TerminalTombstone(tombstone) => {
                values.push(SchemaQualifiedCanonicalValue::from_validated(
                    &tombstone.validated()?,
                )?);
                values.push(SchemaQualifiedCanonicalValue::from_validated(
                    &tombstone.terminal_proof().validated()?,
                )?);
                append_returned_content(
                    &mut values,
                    tombstone.terminal_proof().returned_outcome(),
                )?;
            }
            Self::EffectBound { .. }
            | Self::ResourceAllocated(_)
            | Self::DeliveryAttemptAuthorized { .. } => {}
        }
        Ok(values)
    }
}

fn append_outcome_content(
    values: &mut Vec<SchemaQualifiedCanonicalValue>,
    outcome: &DeliveryAttemptOutcome,
) -> Result<()> {
    values.push(SchemaQualifiedCanonicalValue::from_validated(
        &outcome.validated()?,
    )?);
    match outcome.kind() {
        DeliveryAttemptOutcomeKind::Returned(returned) => append_returned_content(values, returned),
        DeliveryAttemptOutcomeKind::DidNotEnter(failure)
        | DeliveryAttemptOutcomeKind::Indeterminate(failure) => {
            values.push(SchemaQualifiedCanonicalValue::from_validated(
                &validated_safe_failure(failure)?,
            )?);
            Ok(())
        }
        DeliveryAttemptOutcomeKind::NonDomainFailure(_) => Ok(()),
    }
}

fn append_returned_content(
    values: &mut Vec<SchemaQualifiedCanonicalValue>,
    returned: &ReturnedOutcome,
) -> Result<()> {
    values.push(SchemaQualifiedCanonicalValue::from_validated(
        &returned.validated()?,
    )?);
    values.push(returned.safe_result().clone());
    Ok(())
}

fn canonical_content_closure_bytes(values: &[SchemaQualifiedCanonicalValue]) -> Result<usize> {
    let mut objects = BTreeMap::<ContentRef, &SchemaQualifiedCanonicalValue>::new();
    let mut bytes = 0_usize;
    for value in values {
        let reference = value.reference()?;
        match objects.get(&reference) {
            Some(existing) if *existing != value => {
                return Err(ExecutorError::RetainedObjectMismatch);
            }
            Some(_) => {}
            None => {
                bytes = bytes
                    .checked_add(value.as_bytes().len())
                    .ok_or(ExecutorError::EvidenceBoundsExhausted)?;
                objects.insert(reference, value);
            }
        }
    }
    Ok(bytes)
}

#[derive(Default)]
struct CanonicalContentClosure {
    objects: BTreeMap<ContentRef, SchemaQualifiedCanonicalValue>,
    bytes: usize,
}

impl CanonicalContentClosure {
    fn extend(
        &mut self,
        values: impl IntoIterator<Item = SchemaQualifiedCanonicalValue>,
    ) -> Result<()> {
        for value in values {
            let reference = value.reference()?;
            match self.objects.get(&reference) {
                Some(existing) if existing != &value => {
                    return Err(ExecutorError::RetainedObjectMismatch);
                }
                Some(_) => {}
                None => {
                    self.bytes = self
                        .bytes
                        .checked_add(value.as_bytes().len())
                        .ok_or(ExecutorError::EvidenceBoundsExhausted)?;
                    self.objects.insert(reference, value);
                }
            }
        }
        Ok(())
    }

    const fn bytes(&self) -> usize {
        self.bytes
    }
}

/// Semantic identity of one immutable executor ledger record.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ExecutorLedgerRecordRef(SemanticDigest);

impl ExecutorLedgerRecordRef {
    /// Returns the semantic record digest.
    pub const fn semantic_digest(&self) -> &SemanticDigest {
        &self.0
    }
}

/// Recomputed semantic proof over one immutable frontier append.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrontierProof(SemanticDigest);

impl FrontierProof {
    /// Returns the frozen domain-separated frontier digest.
    pub const fn semantic_digest(&self) -> &SemanticDigest {
        &self.0
    }
}

/// One immutable append in a predecessor-linked delivery-audit chain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeliveryAuditFrontier {
    executor_binding_ref: ExecutorBindingRef,
    effect_key: EffectKey,
    request_digest: RequestDigest,
    predecessor_frontier_ref: Option<DeliveryAuditFrontierRef>,
    appended_records: Vec<ExecutorEvidenceRecord>,
    proof_ref: ContentRef,
    proof: FrontierProof,
}

impl DeliveryAuditFrontier {
    pub(crate) fn append(
        identity: &EffectIdentity,
        predecessor_frontier_ref: Option<DeliveryAuditFrontierRef>,
        appended_records: Vec<ExecutorEvidenceRecord>,
        proof_ref: ContentRef,
    ) -> Result<Self> {
        if appended_records.is_empty() || appended_records.len() > 256 {
            return Err(ExecutorError::InvalidFrontier);
        }
        let mut frontier = Self {
            executor_binding_ref: identity.executor_binding_ref().clone(),
            effect_key: identity.effect_key().clone(),
            request_digest: identity.request_digest().clone(),
            predecessor_frontier_ref,
            appended_records,
            proof_ref,
            proof: FrontierProof(identity.request_digest().semantic_digest().clone()),
        };
        frontier.proof = frontier.compute_proof()?;
        Ok(frontier)
    }

    /// Returns the exact executor binding.
    pub const fn executor_binding_ref(&self) -> &ExecutorBindingRef {
        &self.executor_binding_ref
    }

    /// Returns the exact effect key.
    pub const fn effect_key(&self) -> &EffectKey {
        &self.effect_key
    }

    /// Returns the immutable request digest.
    pub const fn request_digest(&self) -> &RequestDigest {
        &self.request_digest
    }

    /// Returns the immediate predecessor frontier, if any.
    pub const fn predecessor_frontier_ref(&self) -> Option<&DeliveryAuditFrontierRef> {
        self.predecessor_frontier_ref.as_ref()
    }

    /// Returns the records appended by this frontier.
    pub fn appended_records(&self) -> &[ExecutorEvidenceRecord] {
        &self.appended_records
    }

    /// Returns the binding-selected proof object identity.
    pub const fn proof_ref(&self) -> &ContentRef {
        &self.proof_ref
    }

    /// Returns the recomputed semantic frontier proof.
    pub const fn proof(&self) -> &FrontierProof {
        &self.proof
    }

    /// Returns the exact canonical frontier object.
    pub fn validated(&self) -> Result<ValidatedCanonicalValue> {
        let records = self
            .appended_records
            .iter()
            .map(|record| {
                record
                    .validated()?
                    .canonical_value()
                    .map_err(crate::contract::contract_error)
            })
            .collect::<Result<Vec<_>>>()?;
        let mut fields = vec![
            (
                "appended_records".to_owned(),
                CanonicalValue::Array(records),
            ),
            ("effect_key".to_owned(), effect_key_value(&self.effect_key)),
            (
                "executor_binding_ref".to_owned(),
                self.executor_binding_ref.canonical_value()?,
            ),
            ("proof_ref".to_owned(), content_ref_value(&self.proof_ref)?),
            (
                "request_digest".to_owned(),
                semantic_digest_value(self.request_digest.semantic_digest()),
            ),
            (
                "version".to_owned(),
                CanonicalValue::String(DELIVERY_FRONTIER_SCHEMA.to_owned()),
            ),
        ];
        if let Some(predecessor) = &self.predecessor_frontier_ref {
            fields.push((
                "predecessor_frontier_ref".to_owned(),
                content_ref_value(predecessor.as_content_ref())?,
            ));
        }
        encode(DELIVERY_FRONTIER_SCHEMA, &object(fields)?)
    }

    /// Computes the exact frontier content identity.
    pub fn reference(&self) -> Result<DeliveryAuditFrontierRef> {
        Ok(DeliveryAuditFrontierRef(content_ref(&self.validated()?)?))
    }

    /// Returns the exact deduplicated executor-owned canonical content-closure length.
    pub fn retained_bytes(&self) -> Result<usize> {
        canonical_content_closure_bytes(&self.content_objects()?)
    }

    /// Encodes this exact frontier into a bounded checksummed backend payload.
    pub fn to_durable_bytes(&self) -> Result<Vec<u8>> {
        let mut encoder = Encoder::new(FRONTIER_DURABLE_MAGIC);
        encoder.content_ref(self.executor_binding_ref.as_content_ref())?;
        encoder.string(self.effect_key.as_str())?;
        encoder.string(self.request_digest.as_str())?;
        encode_optional_content_ref(
            &mut encoder,
            self.predecessor_frontier_ref
                .as_ref()
                .map(DeliveryAuditFrontierRef::as_content_ref),
        )?;
        encoder.usize(self.appended_records.len())?;
        for record in &self.appended_records {
            encode_evidence_record(&mut encoder, record)?;
        }
        encoder.content_ref(&self.proof_ref)?;
        encoder.finish()
    }

    /// Strictly reconstructs one bounded checksummed backend payload.
    pub fn from_durable_bytes(bytes: &[u8]) -> Result<Self> {
        let mut decoder = Decoder::new(bytes, FRONTIER_DURABLE_MAGIC)?;
        let executor_binding_ref = ExecutorBindingRef::from_content_ref(decoder.content_ref()?)?;
        let effect_key = decoder.effect_key()?;
        let request_digest = decoder.request_digest()?;
        let predecessor_frontier_ref = decode_optional_content_ref(&mut decoder)?
            .map(DeliveryAuditFrontierRef::from_content_ref)
            .transpose()?;
        let count = decoder.count(256, 1)?;
        if count == 0 {
            return Err(ExecutorError::InvalidDurableSnapshot);
        }
        let mut appended_records = Vec::with_capacity(count);
        for _ in 0..count {
            appended_records.push(decode_evidence_record(&mut decoder)?);
        }
        let proof_ref = decoder.content_ref()?;
        decoder.finish()?;
        let mut frontier = Self {
            executor_binding_ref,
            effect_key,
            proof: FrontierProof(request_digest.semantic_digest().clone()),
            request_digest,
            predecessor_frontier_ref,
            appended_records,
            proof_ref,
        };
        frontier.proof = frontier.compute_proof()?;
        frontier.verify_proof()?;
        Ok(frontier)
    }

    /// Returns the exact executor-owned content closure introduced by this frontier.
    pub fn content_objects(&self) -> Result<Vec<SchemaQualifiedCanonicalValue>> {
        let mut values = vec![SchemaQualifiedCanonicalValue::from_validated(
            &self.validated()?,
        )?];
        for record in &self.appended_records {
            values.extend(record.content_objects()?);
        }
        Ok(values)
    }

    fn compute_proof(&self) -> Result<FrontierProof> {
        let frontier = self.validated()?;
        let preimage = encode(
            FRONTIER_PREIMAGE_SCHEMA,
            &canonical_object([(
                "frontier",
                frontier
                    .canonical_value()
                    .map_err(crate::contract::contract_error)?,
            )])?,
        )?;
        let digest = recoverability_contract()?
            .semantic_digest(FRONTIER_DOMAIN, &preimage)
            .map_err(crate::contract::contract_error)?;
        Ok(FrontierProof(digest))
    }

    fn verify_proof(&self) -> Result<()> {
        if self.compute_proof()? != self.proof {
            return Err(ExecutorError::InvalidFrontierProof);
        }
        Ok(())
    }
}

/// Content identity of one delivery-audit frontier.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DeliveryAuditFrontierRef(ContentRef);

impl DeliveryAuditFrontierRef {
    /// Returns the lightweight frontier content identity.
    pub const fn as_content_ref(&self) -> &ContentRef {
        &self.0
    }

    /// Reconstructs a frontier reference after checking its exact schema.
    pub fn from_content_ref(value: ContentRef) -> Result<Self> {
        crate::contract::require_schema_ref(&value, DELIVERY_FRONTIER_SCHEMA)?;
        Ok(Self(value))
    }
}

/// Computes the exact deduplicated canonical content-closure bytes for one
/// candidate observed-attempt append.
pub fn observation_completion_closure_bytes(
    identity: &EffectIdentity,
    predecessor_frontier_ref: DeliveryAuditFrontierRef,
    attempt_id: AttemptId,
    outcome: DeliveryAttemptOutcome,
    proof_ref: ContentRef,
) -> Result<usize> {
    DeliveryAuditFrontier::append(
        identity,
        Some(predecessor_frontier_ref),
        vec![ExecutorEvidenceRecord::DeliveryAttemptObserved {
            attempt_id,
            outcome,
        }],
        proof_ref,
    )?
    .retained_bytes()
}

/// Computes the exact deduplicated canonical content-closure bytes for one
/// candidate terminal-tombstone append.
pub fn tombstone_completion_closure_bytes(
    identity: &EffectIdentity,
    predecessor_frontier_ref: DeliveryAuditFrontierRef,
    tombstone: TerminalTombstone,
    proof_ref: ContentRef,
) -> Result<usize> {
    DeliveryAuditFrontier::append(
        identity,
        Some(predecessor_frontier_ref),
        vec![ExecutorEvidenceRecord::TerminalTombstone(tombstone)],
        proof_ref,
    )?
    .retained_bytes()
}

/// Finite bounds fixed by an executor contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidenceBounds {
    max_attempts: u32,
    max_records: u32,
    max_retained_bytes: usize,
    max_completion_record_bytes: usize,
    completion_reserve_records: u32,
    completion_reserve_bytes: usize,
}

impl EvidenceBounds {
    /// Constructs checked finite delivery-audit bounds.
    pub fn new(
        max_attempts: u32,
        max_records: u32,
        max_retained_bytes: u64,
        max_completion_record_bytes: u64,
        completion_reserve_records: u32,
        completion_reserve_bytes: u64,
    ) -> Result<Self> {
        let max_retained_bytes = usize::try_from(max_retained_bytes)
            .map_err(|_| ExecutorError::EvidenceBoundsExhausted)?;
        let max_completion_record_bytes = usize::try_from(max_completion_record_bytes)
            .map_err(|_| ExecutorError::EvidenceBoundsExhausted)?;
        let completion_reserve_bytes = usize::try_from(completion_reserve_bytes)
            .map_err(|_| ExecutorError::EvidenceBoundsExhausted)?;
        if max_attempts > 64
            || max_records == 0
            || max_retained_bytes == 0
            || max_retained_bytes > MAX_DURABLE_SNAPSHOT_BYTES
            || completion_reserve_records > max_records
            || max_completion_record_bytes > completion_reserve_bytes
            || completion_reserve_bytes > max_retained_bytes
            || (max_attempts == 0
                && (max_completion_record_bytes != 0
                    || completion_reserve_records != 0
                    || completion_reserve_bytes != 0))
            || (max_attempts > 0
                && (max_completion_record_bytes == 0
                    || completion_reserve_records != 2
                    || completion_reserve_bytes == 0))
        {
            return Err(ExecutorError::EvidenceBoundsExhausted);
        }
        let bounds = Self {
            max_attempts,
            max_records,
            max_retained_bytes,
            max_completion_record_bytes,
            completion_reserve_records,
            completion_reserve_bytes,
        };
        bounds.validated()?;
        Ok(bounds)
    }

    /// Strictly reconstructs retained frozen evidence-bounds bytes.
    pub fn strict_decode(bytes: &[u8]) -> Result<Self> {
        let validated = recoverability_contract()?
            .strict_decode(EVIDENCE_BOUNDS_SCHEMA, bytes)
            .map_err(crate::contract::contract_error)?;
        let json: serde_json::Value = serde_json::from_slice(validated.as_bytes())
            .map_err(|_| ExecutorError::CanonicalEncoding)?;
        let unsigned = |name: &str| {
            json.get(name)
                .and_then(serde_json::Value::as_u64)
                .ok_or(ExecutorError::CanonicalEncoding)
        };
        let decimal = |name: &str| {
            json.get(name)
                .and_then(serde_json::Value::as_str)
                .ok_or(ExecutorError::CanonicalEncoding)?
                .parse::<u64>()
                .map_err(|_| ExecutorError::CanonicalEncoding)
        };
        let bounds = Self::new(
            u32::try_from(unsigned("max_attempts")?)
                .map_err(|_| ExecutorError::EvidenceBoundsExhausted)?,
            u32::try_from(unsigned("max_records")?)
                .map_err(|_| ExecutorError::EvidenceBoundsExhausted)?,
            decimal("max_retained_bytes")?,
            decimal("max_completion_record_bytes")?,
            u32::try_from(unsigned("completion_reserve_records")?)
                .map_err(|_| ExecutorError::EvidenceBoundsExhausted)?,
            decimal("completion_reserve_bytes")?,
        )?;
        if bounds.validated()? != validated {
            return Err(ExecutorError::CanonicalEncoding);
        }
        Ok(bounds)
    }

    /// Returns the maximum authorized-attempt count.
    pub const fn max_attempts(&self) -> u32 {
        self.max_attempts
    }

    /// Returns the maximum retained record count.
    pub const fn max_records(&self) -> u32 {
        self.max_records
    }

    /// Returns the maximum retained canonical frontier bytes.
    pub const fn max_retained_bytes(&self) -> usize {
        self.max_retained_bytes
    }

    /// Returns the hard canonical content-closure ceiling for one completion append.
    pub const fn max_completion_record_bytes(&self) -> usize {
        self.max_completion_record_bytes
    }

    /// Returns the fixed observation-plus-shared-tombstone structural envelope.
    pub const fn completion_reserve_records(&self) -> u32 {
        self.completion_reserve_records
    }

    /// Returns the worst-case byte reserve per required completion record.
    pub const fn completion_reserve_bytes(&self) -> usize {
        self.completion_reserve_bytes
    }

    /// Returns the exact canonical evidence-bounds object.
    pub fn validated(&self) -> Result<ValidatedCanonicalValue> {
        encode(
            EVIDENCE_BOUNDS_SCHEMA,
            &canonical_object([
                (
                    "completion_reserve_bytes",
                    CanonicalValue::String(self.completion_reserve_bytes.to_string()),
                ),
                (
                    "completion_reserve_records",
                    CanonicalValue::Unsigned(u64::from(self.completion_reserve_records)),
                ),
                (
                    "max_attempts",
                    CanonicalValue::Unsigned(u64::from(self.max_attempts)),
                ),
                (
                    "max_completion_record_bytes",
                    CanonicalValue::String(self.max_completion_record_bytes.to_string()),
                ),
                (
                    "max_records",
                    CanonicalValue::Unsigned(u64::from(self.max_records)),
                ),
                (
                    "max_retained_bytes",
                    CanonicalValue::String(self.max_retained_bytes.to_string()),
                ),
            ])?,
        )
    }

    fn ensure_completion_capacity(
        &self,
        record_count: usize,
        retained_bytes: usize,
        unmatched_attempts: usize,
        tombstone_absent: bool,
    ) -> Result<()> {
        if record_count > self.max_records as usize || retained_bytes > self.max_retained_bytes {
            return Err(ExecutorError::EvidenceBoundsExhausted);
        }
        let structural_debt = unmatched_attempts
            .checked_add(usize::from(tombstone_absent && self.max_attempts > 0))
            .ok_or(ExecutorError::EvidenceBoundsExhausted)?;
        let reserved_records = record_count
            .checked_add(structural_debt)
            .ok_or(ExecutorError::EvidenceBoundsExhausted)?;
        let reserved_bytes = retained_bytes
            .checked_add(
                structural_debt
                    .checked_mul(self.completion_reserve_bytes)
                    .ok_or(ExecutorError::EvidenceBoundsExhausted)?,
            )
            .ok_or(ExecutorError::EvidenceBoundsExhausted)?;
        if reserved_records > self.max_records as usize || reserved_bytes > self.max_retained_bytes
        {
            return Err(ExecutorError::EvidenceBoundsExhausted);
        }
        Ok(())
    }
}

/// Complete bounded delivery-audit chain for one effect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeliveryAudit {
    frontiers: Vec<DeliveryAuditFrontier>,
}

/// Read-only folded view of one deterministic delivery attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeliveryAttemptView<'a> {
    attempt_ordinal: u32,
    attempt_id: &'a AttemptId,
    target_operation_ref: &'a ContentRef,
    outcome: Option<&'a DeliveryAttemptOutcome>,
    observation_ref: Option<ContentRef>,
}

impl<'a> DeliveryAttemptView<'a> {
    /// Returns the monotonic ordinal within this effect.
    pub const fn attempt_ordinal(&self) -> u32 {
        self.attempt_ordinal
    }

    /// Returns the deterministically derived attempt identity.
    pub const fn attempt_id(&self) -> &'a AttemptId {
        self.attempt_id
    }

    /// Returns the reviewed target operation family.
    pub const fn target_operation_ref(&self) -> &'a ContentRef {
        self.target_operation_ref
    }

    /// Returns the surviving safe outcome, when observed.
    pub const fn outcome(&self) -> Option<&'a DeliveryAttemptOutcome> {
        self.outcome
    }

    /// Returns the exact observation identity, when observed.
    pub const fn observation_ref(&self) -> Option<&ContentRef> {
        self.observation_ref.as_ref()
    }

    /// Returns the exact returned-observation identity, when this attempt returned.
    pub fn returned_observation_ref(&self) -> Option<&ContentRef> {
        self.outcome
            .and_then(DeliveryAttemptOutcome::returned_outcome)
            .and(self.observation_ref.as_ref())
    }
}

#[derive(Debug)]
struct AttemptFold {
    authorized_before_tombstone: bool,
    observed: Option<(DeliveryAttemptOutcome, ContentRef)>,
}

impl DeliveryAudit {
    pub(crate) fn from_ledger(frontiers: Vec<DeliveryAuditFrontier>) -> Self {
        Self { frontiers }
    }

    /// Returns all immutable frontier appends from the initial binding.
    pub fn frontiers(&self) -> &[DeliveryAuditFrontier] {
        &self.frontiers
    }

    /// Returns the greatest retained frontier reference.
    pub fn head_ref(&self) -> Result<DeliveryAuditFrontierRef> {
        self.frontiers
            .last()
            .ok_or(ExecutorError::InvalidFrontier)?
            .reference()
    }

    /// Returns the total retained evidence-record count.
    pub fn record_count(&self) -> usize {
        self.frontiers
            .iter()
            .map(|frontier| frontier.appended_records.len())
            .sum()
    }

    /// Returns the number of authorized target attempts.
    pub fn attempt_count(&self) -> usize {
        self.records()
            .filter(|record| {
                matches!(
                    record,
                    ExecutorEvidenceRecord::DeliveryAttemptAuthorized { .. }
                )
            })
            .count()
    }

    /// Returns all deterministic attempts with their optional folded observations.
    pub fn attempts(&self) -> Result<Vec<DeliveryAttemptView<'_>>> {
        let mut attempts = Vec::new();
        let mut indexes = BTreeMap::new();
        for record in self.records() {
            match record {
                ExecutorEvidenceRecord::DeliveryAttemptAuthorized {
                    attempt_ordinal,
                    attempt_id,
                    target_operation_ref,
                } => {
                    if indexes.insert(attempt_id.clone(), attempts.len()).is_some() {
                        return Err(ExecutorError::AttemptIdentityMismatch);
                    }
                    attempts.push(DeliveryAttemptView {
                        attempt_ordinal: *attempt_ordinal,
                        attempt_id,
                        target_operation_ref,
                        outcome: None,
                        observation_ref: None,
                    });
                }
                ExecutorEvidenceRecord::DeliveryAttemptObserved {
                    attempt_id,
                    outcome,
                } => {
                    let index = indexes
                        .get(attempt_id)
                        .copied()
                        .ok_or(ExecutorError::InvalidDeliveryObservation)?;
                    let attempt = attempts
                        .get_mut(index)
                        .ok_or(ExecutorError::InvalidDeliveryObservation)?;
                    if attempt.outcome.is_some() {
                        return Err(ExecutorError::InvalidDeliveryObservation);
                    }
                    attempt.outcome = Some(outcome);
                    attempt.observation_ref = record.observed_content_ref()?;
                }
                ExecutorEvidenceRecord::EffectBound { .. }
                | ExecutorEvidenceRecord::ResourceAllocated(_)
                | ExecutorEvidenceRecord::TerminalTombstone(_) => {}
            }
        }
        Ok(attempts)
    }

    /// Returns the exact deduplicated executor-owned canonical content-closure length.
    pub fn retained_bytes(&self) -> Result<usize> {
        let mut closure = CanonicalContentClosure::default();
        for frontier in &self.frontiers {
            closure.extend(frontier.content_objects()?)?;
        }
        Ok(closure.bytes())
    }

    /// Verifies the complete audit under one exact executor binding.
    pub fn verify(
        &self,
        identity: &EffectIdentity,
        binding: &VerifiedExecutorBinding,
    ) -> Result<()> {
        if identity.executor_binding_ref() != binding.binding_ref()
            || identity.tenant_scope_id() != binding.deployment().tenant_scope_id()
        {
            return Err(ExecutorError::WrongExecutorBinding);
        }
        self.verify_structure(
            identity,
            binding.contract().evidence_bounds(),
            binding.deployment().evidence_authority_ref(),
        )?;
        for record in self.records() {
            if let ExecutorEvidenceRecord::DeliveryAttemptObserved { outcome, .. } = record {
                outcome.validate_for_binding(binding)?;
            }
        }
        Ok(())
    }

    pub(crate) fn verify_structure(
        &self,
        identity: &EffectIdentity,
        bounds: &EvidenceBounds,
        expected_proof_ref: &ContentRef,
    ) -> Result<()> {
        if self.frontiers.is_empty() {
            return Err(ExecutorError::InvalidFrontier);
        }
        let mut previous = None;
        let mut record_index = 0_usize;
        let mut retained_closure = CanonicalContentClosure::default();
        let mut resource_count = 0_u8;
        let mut next_attempt_ordinal = 0_u32;
        let mut attempts = BTreeMap::<AttemptId, AttemptFold>::new();
        let mut tombstone: Option<TerminalTombstone> = None;

        for frontier in &self.frontiers {
            if frontier.executor_binding_ref != *identity.executor_binding_ref()
                || frontier.effect_key != *identity.effect_key()
                || frontier.request_digest != *identity.request_digest()
                || frontier.predecessor_frontier_ref != previous
                || frontier.proof_ref != *expected_proof_ref
            {
                return Err(ExecutorError::InvalidFrontier);
            }
            frontier.verify_proof()?;
            let is_completion = frontier.appended_records.iter().any(|record| {
                matches!(
                    record,
                    ExecutorEvidenceRecord::DeliveryAttemptObserved { .. }
                        | ExecutorEvidenceRecord::TerminalTombstone(_)
                )
            });
            if is_completion && frontier.appended_records.len() != 1 {
                return Err(ExecutorError::InvalidFrontier);
            }

            for record in &frontier.appended_records {
                if tombstone.is_some()
                    && !matches!(
                        record,
                        ExecutorEvidenceRecord::DeliveryAttemptObserved { .. }
                    )
                {
                    return Err(ExecutorError::InvalidFrontier);
                }
                match record {
                    ExecutorEvidenceRecord::EffectBound {
                        executor_binding_ref,
                        effect_key,
                        request_digest,
                    } => {
                        if record_index != 0
                            || executor_binding_ref != identity.executor_binding_ref()
                            || effect_key != identity.effect_key()
                            || request_digest != identity.request_digest()
                        {
                            return Err(ExecutorError::InvalidFrontier);
                        }
                    }
                    ExecutorEvidenceRecord::ResourceAllocated(_) => {
                        if record_index == 0 || next_attempt_ordinal != 0 || resource_count != 0 {
                            return Err(ExecutorError::InvalidFrontier);
                        }
                        resource_count = resource_count.saturating_add(1);
                    }
                    ExecutorEvidenceRecord::DeliveryAttemptAuthorized {
                        attempt_ordinal,
                        attempt_id,
                        target_operation_ref,
                    } => {
                        if record_index == 0
                            || *attempt_ordinal != next_attempt_ordinal
                            || *attempt_ordinal >= 64
                            || next_attempt_ordinal >= bounds.max_attempts
                        {
                            return Err(ExecutorError::InvalidFrontier);
                        }
                        let expected =
                            derive_attempt_id(identity, *attempt_ordinal, target_operation_ref)?;
                        if expected != *attempt_id || attempts.contains_key(attempt_id) {
                            return Err(ExecutorError::AttemptIdentityMismatch);
                        }
                        attempts.insert(
                            attempt_id.clone(),
                            AttemptFold {
                                authorized_before_tombstone: tombstone.is_none(),
                                observed: None,
                            },
                        );
                        next_attempt_ordinal = next_attempt_ordinal
                            .checked_add(1)
                            .ok_or(ExecutorError::EvidenceBoundsExhausted)?;
                    }
                    ExecutorEvidenceRecord::DeliveryAttemptObserved {
                        attempt_id,
                        outcome,
                    } => {
                        let attempt = attempts
                            .get_mut(attempt_id)
                            .ok_or(ExecutorError::InvalidDeliveryObservation)?;
                        if attempt.observed.is_some()
                            || (tombstone.is_some() && !attempt.authorized_before_tombstone)
                        {
                            return Err(ExecutorError::InvalidDeliveryObservation);
                        }
                        let observation_ref = record
                            .observed_content_ref()?
                            .ok_or(ExecutorError::InvalidDeliveryObservation)?;
                        attempt.observed = Some((outcome.clone(), observation_ref));
                    }
                    ExecutorEvidenceRecord::TerminalTombstone(candidate) => {
                        if tombstone.is_some() {
                            return Err(ExecutorError::TerminalTombstoneConflict);
                        }
                        let proof = candidate.terminal_proof();
                        let Some(attempt) = attempts.get(proof.attempt_id()) else {
                            return Err(ExecutorError::TerminalProofMismatch);
                        };
                        let Some((outcome, observation_ref)) = attempt.observed.as_ref() else {
                            return Err(ExecutorError::TerminalProofMismatch);
                        };
                        let Some(returned) = outcome.returned_outcome() else {
                            return Err(ExecutorError::TerminalProofMismatch);
                        };
                        if returned != proof.returned_outcome()
                            || observation_ref != proof.returned_observation_ref()
                        {
                            return Err(ExecutorError::TerminalProofMismatch);
                        }
                        tombstone = Some(candidate.clone());
                    }
                }
                record.reference()?;
                record_index = record_index
                    .checked_add(1)
                    .ok_or(ExecutorError::EvidenceBoundsExhausted)?;
            }

            let frontier_content = frontier.content_objects()?;
            if is_completion
                && canonical_content_closure_bytes(&frontier_content)?
                    > bounds.max_completion_record_bytes
            {
                return Err(ExecutorError::EvidenceBoundsExhausted);
            }
            retained_closure.extend(frontier_content)?;
            let unmatched = attempts
                .values()
                .filter(|attempt| attempt.observed.is_none())
                .count();
            bounds.ensure_completion_capacity(
                record_index,
                retained_closure.bytes(),
                unmatched,
                tombstone.is_none(),
            )?;
            previous = Some(frontier.reference()?);
        }
        Ok(())
    }

    pub(crate) fn records(&self) -> impl Iterator<Item = &ExecutorEvidenceRecord> {
        self.frontiers
            .iter()
            .flat_map(|frontier| frontier.appended_records.iter())
    }
}

/// Relationship of a valid candidate frontier to the retained greatest chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdmitFrontier {
    /// Candidate exactly equals the greatest retained chain.
    Equal,
    /// Candidate is an older valid ancestor and does not regress the greatest.
    StaleAncestor,
    /// Candidate is a valid descendant and becomes the greatest chain.
    Advanced,
}

/// Private greatest-frontier fold used when admitting executor observations.
#[derive(Debug, Clone, Default)]
pub struct DeliveryAuditAccumulator {
    greatest: Option<DeliveryAudit>,
}

impl DeliveryAuditAccumulator {
    /// Returns the greatest admitted delivery audit, if any.
    pub const fn greatest(&self) -> Option<&DeliveryAudit> {
        self.greatest.as_ref()
    }

    /// Verifies and admits one complete bounded candidate chain.
    pub fn admit(
        &mut self,
        candidate: DeliveryAudit,
        identity: &EffectIdentity,
        binding: &VerifiedExecutorBinding,
    ) -> Result<AdmitFrontier> {
        candidate.verify(identity, binding)?;
        self.admit_verified(candidate)
    }

    #[cfg(test)]
    pub(crate) fn admit_structure(
        &mut self,
        candidate: DeliveryAudit,
        identity: &EffectIdentity,
        bounds: &EvidenceBounds,
        expected_proof_ref: &ContentRef,
    ) -> Result<AdmitFrontier> {
        candidate.verify_structure(identity, bounds, expected_proof_ref)?;
        self.admit_verified(candidate)
    }

    fn admit_verified(&mut self, candidate: DeliveryAudit) -> Result<AdmitFrontier> {
        let Some(greatest) = self.greatest.as_ref() else {
            self.greatest = Some(candidate);
            return Ok(AdmitFrontier::Advanced);
        };
        let common = greatest.frontiers.len().min(candidate.frontiers.len());
        for index in 0..common {
            if greatest.frontiers[index].reference()? != candidate.frontiers[index].reference()? {
                return Err(ExecutorError::FrontierFork);
            }
        }
        match candidate.frontiers.len().cmp(&greatest.frontiers.len()) {
            std::cmp::Ordering::Equal => Ok(AdmitFrontier::Equal),
            std::cmp::Ordering::Less => Ok(AdmitFrontier::StaleAncestor),
            std::cmp::Ordering::Greater => {
                self.greatest = Some(candidate);
                Ok(AdmitFrontier::Advanced)
            }
        }
    }
}

pub(crate) fn encode_evidence_record(
    encoder: &mut Encoder,
    record: &ExecutorEvidenceRecord,
) -> Result<()> {
    match record {
        ExecutorEvidenceRecord::EffectBound {
            executor_binding_ref,
            effect_key,
            request_digest,
        } => {
            encoder.u8(0);
            encoder.content_ref(executor_binding_ref.as_content_ref())?;
            encoder.string(effect_key.as_str())?;
            encoder.string(request_digest.as_str())?;
        }
        ExecutorEvidenceRecord::ResourceAllocated(record) => {
            encoder.u8(1);
            encoder.content_ref(record.resource_ownership_ref.as_content_ref())?;
            encoder.content_ref(record.resource_key_ref.as_content_ref())?;
            encoder.content_ref(record.typed_allocation_state_ref.as_content_ref())?;
            encoder.content_ref(&record.policy_ref)?;
            encoder.content_ref(&record.policy_configuration_ref)?;
            encode_optional_content_ref(
                encoder,
                record.fencing_ref.as_ref().map(FencingRef::as_content_ref),
            )?;
        }
        ExecutorEvidenceRecord::DeliveryAttemptAuthorized {
            attempt_ordinal,
            attempt_id,
            target_operation_ref,
        } => {
            encoder.u8(2);
            encoder.u32(*attempt_ordinal);
            encoder.string(attempt_id.as_str())?;
            encoder.content_ref(target_operation_ref)?;
        }
        ExecutorEvidenceRecord::DeliveryAttemptObserved {
            attempt_id,
            outcome,
        } => {
            encoder.u8(3);
            encoder.string(attempt_id.as_str())?;
            encode_outcome(encoder, outcome)?;
        }
        ExecutorEvidenceRecord::TerminalTombstone(tombstone) => {
            encoder.u8(4);
            encoder.string(tombstone.external_operation_identity())?;
            encoder.string(tombstone.terminal_outcome())?;
            encoder.string(tombstone.terminal_proof.attempt_id.as_str())?;
            encoder.schema_qualified(tombstone.terminal_proof.returned_outcome.safe_result())?;
            encoder.content_ref(&tombstone.terminal_proof.returned_observation_ref)?;
        }
    }
    Ok(())
}

pub(crate) fn decode_evidence_record(decoder: &mut Decoder<'_>) -> Result<ExecutorEvidenceRecord> {
    match decoder.u8()? {
        0 => Ok(ExecutorEvidenceRecord::EffectBound {
            executor_binding_ref: ExecutorBindingRef::from_content_ref(decoder.content_ref()?)?,
            effect_key: decoder.effect_key()?,
            request_digest: decoder.request_digest()?,
        }),
        1 => Ok(ExecutorEvidenceRecord::ResourceAllocated(Box::new(
            ResourceAllocatedRecord::new(
                ResourceOwnershipRef::from_content_ref(decoder.content_ref()?)?,
                ResourceKeyRef::from_reviewed(decoder.content_ref()?),
                AllocationStateRef::from_reviewed(decoder.content_ref()?),
                decoder.content_ref()?,
                decoder.content_ref()?,
                decode_optional_content_ref(decoder)?.map(FencingRef::from_reviewed),
            ),
        ))),
        2 => Ok(ExecutorEvidenceRecord::DeliveryAttemptAuthorized {
            attempt_ordinal: decoder.u32()?,
            attempt_id: decoder.attempt_id()?,
            target_operation_ref: decoder.content_ref()?,
        }),
        3 => Ok(ExecutorEvidenceRecord::DeliveryAttemptObserved {
            attempt_id: decoder.attempt_id()?,
            outcome: decode_outcome(decoder)?,
        }),
        4 => {
            let external_operation_identity = decoder.string()?;
            let terminal_outcome = decoder.string()?;
            let proof = ReferenceTerminalProof::new(
                decoder.attempt_id()?,
                ReturnedOutcome::new(decoder.schema_qualified()?)?,
                decoder.content_ref()?,
            )?;
            Ok(ExecutorEvidenceRecord::TerminalTombstone(
                TerminalTombstone::new(external_operation_identity, terminal_outcome, proof)?,
            ))
        }
        _ => Err(ExecutorError::InvalidDurableSnapshot),
    }
}

fn encode_outcome(encoder: &mut Encoder, outcome: &DeliveryAttemptOutcome) -> Result<()> {
    match outcome.kind() {
        DeliveryAttemptOutcomeKind::Returned(returned) => {
            encoder.u8(0);
            encoder.schema_qualified(returned.safe_result())?;
        }
        DeliveryAttemptOutcomeKind::DidNotEnter(failure) => {
            encoder.u8(1);
            encode_safe_failure(encoder, failure)?;
        }
        DeliveryAttemptOutcomeKind::Indeterminate(failure) => {
            encoder.u8(2);
            encode_safe_failure(encoder, failure)?;
        }
        DeliveryAttemptOutcomeKind::NonDomainFailure(failure) => {
            encoder.u8(3);
            let canonical = failure
                .canonical_value()
                .map_err(|_| ExecutorError::CanonicalEncoding)?;
            encoder.bytes(CanonicalJsonBytes::from_value(&canonical).as_bytes())?;
        }
    }
    Ok(())
}

fn decode_outcome(decoder: &mut Decoder<'_>) -> Result<DeliveryAttemptOutcome> {
    match decoder.u8()? {
        0 => DeliveryAttemptOutcome::returned(decoder.schema_qualified()?),
        1 => DeliveryAttemptOutcome::did_not_enter(decode_safe_failure(decoder)?),
        2 => DeliveryAttemptOutcome::indeterminate(decode_safe_failure(decoder)?),
        3 => {
            let bytes = decoder.canonical_bytes()?;
            PlainCanonicalJsonBytes::from_canonical_json_slice(&bytes)
                .map_err(|_| ExecutorError::InvalidDurableSnapshot)?;
            let json = serde_json::from_slice(&bytes)
                .map_err(|_| ExecutorError::InvalidDurableSnapshot)?;
            let canonical = plain_json_to_canonical_value(json)
                .map_err(|_| ExecutorError::InvalidDurableSnapshot)?;
            DeliveryAttemptOutcome::non_domain_failure(
                NonDomainFailure::from_canonical_value(&canonical)
                    .map_err(|_| ExecutorError::InvalidDurableSnapshot)?,
            )
        }
        _ => Err(ExecutorError::InvalidDurableSnapshot),
    }
}

fn encode_safe_failure(encoder: &mut Encoder, failure: &ReferenceSafeFailure) -> Result<()> {
    encoder.content_ref(failure.safe_failure_contract_ref())?;
    encoder.u8(match failure.stable_code() {
        ReferenceFailureCode::GenerationFenced => 0,
        ReferenceFailureCode::DestinationUnavailable => 1,
        ReferenceFailureCode::RequestConflict => 2,
        ReferenceFailureCode::AccessCancelled => 3,
        ReferenceFailureCode::UnclassifiedFailure => 4,
        ReferenceFailureCode::ResultUnrepresentable => 5,
    });
    encoder.u8(match failure.failure_class() {
        FailureClass::Authorization => 0,
        FailureClass::Transport => 1,
        FailureClass::Destination => 2,
        FailureClass::Cancellation => 3,
        FailureClass::Unclassified => 4,
        FailureClass::UnrepresentableResponse => 5,
        _ => return Err(ExecutorError::InvalidSafeFailure),
    });
    encoder.u8(match failure.boundary_stage() {
        BoundaryStage::BeforeBoundaryEntry => 0,
        BoundaryStage::BoundaryEntry => 1,
        BoundaryStage::BoundaryObservation => 2,
    });
    Ok(())
}

fn decode_safe_failure(decoder: &mut Decoder<'_>) -> Result<ReferenceSafeFailure> {
    let contract_ref = decoder.content_ref()?;
    let code = match decoder.u8()? {
        0 => ReferenceFailureCode::GenerationFenced,
        1 => ReferenceFailureCode::DestinationUnavailable,
        2 => ReferenceFailureCode::RequestConflict,
        3 => ReferenceFailureCode::AccessCancelled,
        4 => ReferenceFailureCode::UnclassifiedFailure,
        5 => ReferenceFailureCode::ResultUnrepresentable,
        _ => return Err(ExecutorError::InvalidDurableSnapshot),
    };
    let class = match decoder.u8()? {
        0 => FailureClass::Authorization,
        1 => FailureClass::Transport,
        2 => FailureClass::Destination,
        3 => FailureClass::Cancellation,
        4 => FailureClass::Unclassified,
        5 => FailureClass::UnrepresentableResponse,
        _ => return Err(ExecutorError::InvalidDurableSnapshot),
    };
    let stage = match decoder.u8()? {
        0 => BoundaryStage::BeforeBoundaryEntry,
        1 => BoundaryStage::BoundaryEntry,
        2 => BoundaryStage::BoundaryObservation,
        _ => return Err(ExecutorError::InvalidDurableSnapshot),
    };
    reference_safe_failure(contract_ref, code, class, stage)
}

pub(crate) fn encode_optional_content_ref(
    encoder: &mut Encoder,
    value: Option<&ContentRef>,
) -> Result<()> {
    match value {
        None => encoder.u8(0),
        Some(value) => {
            encoder.u8(1);
            encoder.content_ref(value)?;
        }
    }
    Ok(())
}

pub(crate) fn decode_optional_content_ref(decoder: &mut Decoder<'_>) -> Result<Option<ContentRef>> {
    match decoder.u8()? {
        0 => Ok(None),
        1 => Ok(Some(decoder.content_ref()?)),
        _ => Err(ExecutorError::InvalidDurableSnapshot),
    }
}

fn object(entries: Vec<(String, CanonicalValue)>) -> Result<CanonicalValue> {
    CanonicalValue::object(entries).map_err(|_| ExecutorError::CanonicalEncoding)
}

#[cfg(test)]
#[path = "frontier_tests.rs"]
mod tests;
