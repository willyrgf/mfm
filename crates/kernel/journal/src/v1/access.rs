use mfm_canonical::CanonicalValue;
use mfm_capabilities::{BoundaryStage, CoarseSizeClass, FailureClass};
use mfm_ids::{
    ContentRef, EffectKey, FactQueryDigest, NodeId, RequestDigest, RunSemanticStateDigest,
    SemanticDigest, StableId, StoreEpoch, StoreScopeId, TenantScopeId,
    TerminalEffectEvidenceDigest,
};

use super::codec::{
    content_ref_field, cv_content_ref, cv_decimal, cv_retained_value_contract, cv_string,
    define_schema_value, domain_digest, effect_key_field, node_id_field, nullable_field, object,
    required_field, retained_value_contract_from_canonical, stable_id_field, store_epoch_field,
    store_scope_id_field, string_field, tagged_object, tenant_scope_id_field,
};
use super::{
    AuthorizationRef, CapabilityBindingRef, InputManifestRef, JournalError, JournalHead, NodePhase,
    ObservationRef, Result, RetainedValueContract, TenantFactFrontier, TransitionRef, ValueRef,
};

define_schema_value! {
    /// Exact semantic state anchor for an external authorization.
    pub struct SemanticAnchor => "mfm.semantic-anchor.v1";
    /// Closed scope of one external authorization.
    pub struct AuthorizationScope => "mfm.authorization-scope.v1";
    /// Durable authorization of exactly one ambient operation.
    pub struct ExternalAccessAuthorized => "mfm.external-access-authorized.v1";
    /// Closed observed outcome of one authorized ambient operation.
    pub struct ObservationOutcome => "mfm.observation-outcome.v1";
    /// Durable observation linked to exactly one authorization.
    pub struct ExternalAccessObserved => "mfm.external-access-observed.v1";
    /// Frozen retained-value contracts for the reserved fact-selection read.
    pub struct FactSelectionScanContract => "mfm.fact-selection-scan-contract.v1";
    /// Store-authored proof of one complete fact-selection scan.
    pub struct FactSelectionScanAttestation => "mfm.fact-selection-scan-attestation.v1";
    /// Reviewed bounded redaction-safe failure.
    pub struct SafeFailure => "mfm.safe-failure.v1";
    /// Closed persisted result of one durable executor ensure.
    pub struct ExecutorEnsureResult => "mfm.executor-ensure-result.v1";
    /// Closed proof provenance embedded in terminal effect evidence.
    pub struct ExecutorProofBasis => "mfm.executor-proof-basis.v1";
    /// Immutable intent frozen by the first read authorization.
    pub struct FrozenReadIntent => "mfm.frozen-read-intent.v1";
    /// Immutable admitted read-capability binding.
    pub struct ReadCapabilityBinding => "mfm.read-capability-binding.v1";
    /// Redaction-safe audit entry derived from authorization and observation.
    pub struct AccessAuditEntry => "mfm.access-audit-entry.v1";
    /// Folded pending effect summary.
    pub struct PendingEffect => "mfm.pending-effect.v1";
    /// Complete terminal effect evidence observed through audited access.
    pub struct TerminalEffectEvidence => "mfm.terminal-effect-evidence.v1";
    /// Frozen terminal-effect semantic identity preimage.
    pub struct TerminalEvidencePreimage => "mfm.terminal-evidence-preimage.v1";
}

/// Typed fields of a semantic access anchor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticAnchorFields {
    /// Exact physical journal head.
    pub journal_head: JournalHead,
    /// Exact semantic run-state digest.
    pub run_state_digest: RunSemanticStateDigest,
    /// Target node occurrence.
    pub node_id: NodeId,
    /// Target node phase.
    pub node_phase: NodePhase,
}

impl SemanticAnchor {
    /// Constructs and validates a semantic access anchor.
    pub fn new(
        journal_head: &JournalHead,
        run_state_digest: &RunSemanticStateDigest,
        node_id: &NodeId,
        node_phase: NodePhase,
    ) -> Result<Self> {
        Self::from_canonical_value(object([
            ("journal_head", journal_head.canonical_value()?),
            ("run_state_digest", cv_string(run_state_digest.as_str())),
            ("node_id", cv_string(node_id.as_str())),
            ("node_phase", cv_string(node_phase.as_str())),
        ])?)
    }

    /// Projects every semantic-anchor field.
    pub fn fields(&self) -> Result<SemanticAnchorFields> {
        Ok(SemanticAnchorFields {
            journal_head: JournalHead::from_canonical_value(required_field(self, "journal_head")?)?,
            run_state_digest: RunSemanticStateDigest::parse(string_field(
                self,
                "run_state_digest",
            )?)?,
            node_id: node_id_field(self, "node_id")?,
            node_phase: NodePhase::parse(&string_field(self, "node_phase")?)?,
        })
    }
}

/// Closed authorization scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthorizationScopeFields {
    /// One read against an exact frozen input manifest.
    Read {
        /// Exact input manifest.
        input_manifest_ref: InputManifestRef,
    },
    /// One executor ensure for an exact committed effect request.
    EnsureEffect {
        /// Exact effect-request transition.
        effect_request_transition_ref: TransitionRef,
    },
}

impl AuthorizationScope {
    /// Constructs a read authorization scope.
    pub fn read(input_manifest_ref: &InputManifestRef) -> Result<Self> {
        Self::from_canonical_value(tagged_object(
            "read",
            [("input_manifest_ref", input_manifest_ref.canonical_value()?)],
        )?)
    }

    /// Constructs an effect-ensure authorization scope.
    pub fn ensure_effect(effect_request_transition_ref: &TransitionRef) -> Result<Self> {
        Self::from_canonical_value(tagged_object(
            "ensure_effect",
            [(
                "effect_request_transition_ref",
                effect_request_transition_ref.canonical_value()?,
            )],
        )?)
    }

    /// Projects the closed authorization scope.
    pub fn fields(&self) -> Result<AuthorizationScopeFields> {
        match super::codec::tag(self)?.as_str() {
            "read" => Ok(AuthorizationScopeFields::Read {
                input_manifest_ref: InputManifestRef::from_canonical_value(required_field(
                    self,
                    "input_manifest_ref",
                )?)?,
            }),
            "ensure_effect" => Ok(AuthorizationScopeFields::EnsureEffect {
                effect_request_transition_ref: TransitionRef::from_canonical_value(
                    required_field(self, "effect_request_transition_ref")?,
                )?,
            }),
            _ => Err(super::JournalError::Projection),
        }
    }
}

/// Typed fields of a durable external-access authorization.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalAccessAuthorizedFields {
    /// Exact semantic anchor.
    pub semantic_anchor: SemanticAnchor,
    /// Closed operation scope.
    pub scope: AuthorizationScope,
    /// Immutable admitted capability binding.
    pub capability_binding_ref: CapabilityBindingRef,
    /// Exact reviewed capability operation.
    pub capability_operation_id: StableId,
    /// Full retained request authority.
    pub request_ref: ValueRef,
    /// Full retained immutable-read-intent authority, present exactly for read scope.
    pub frozen_read_intent_ref: Option<ValueRef>,
}

impl ExternalAccessAuthorized {
    /// Constructs and validates a complete external-access authorization record.
    pub fn new(
        semantic_anchor: &SemanticAnchor,
        scope: &AuthorizationScope,
        capability_binding_ref: &CapabilityBindingRef,
        capability_operation_id: &StableId,
        request_ref: &ValueRef,
        frozen_read_intent_ref: Option<&ValueRef>,
    ) -> Result<Self> {
        validate_frozen_read_scope(scope, frozen_read_intent_ref)?;
        Self::from_canonical_value(object([
            (
                "version",
                CanonicalValue::String("mfm.external-access-authorized.v1".to_owned()),
            ),
            ("semantic_anchor", semantic_anchor.canonical_value()?),
            ("scope", scope.canonical_value()?),
            (
                "capability_binding_ref",
                capability_binding_ref.canonical_value()?,
            ),
            (
                "capability_operation_id",
                cv_string(capability_operation_id.as_str()),
            ),
            ("request_ref", request_ref.canonical_value()?),
            (
                "frozen_read_intent_ref",
                frozen_read_intent_ref
                    .map(ValueRef::canonical_value)
                    .transpose()?
                    .unwrap_or(CanonicalValue::Null),
            ),
        ])?)
    }

    /// Projects every authorization field.
    pub fn fields(&self) -> Result<ExternalAccessAuthorizedFields> {
        let fields = ExternalAccessAuthorizedFields {
            semantic_anchor: SemanticAnchor::from_canonical_value(required_field(
                self,
                "semantic_anchor",
            )?)?,
            scope: AuthorizationScope::from_canonical_value(required_field(self, "scope")?)?,
            capability_binding_ref: CapabilityBindingRef::from_canonical_value(required_field(
                self,
                "capability_binding_ref",
            )?)?,
            capability_operation_id: stable_id_field(self, "capability_operation_id")?,
            request_ref: ValueRef::from_canonical_value(required_field(self, "request_ref")?)?,
            frozen_read_intent_ref: nullable_field(self, "frozen_read_intent_ref")?
                .map(ValueRef::from_canonical_value)
                .transpose()?,
        };
        validate_frozen_read_scope(&fields.scope, fields.frozen_read_intent_ref.as_ref())?;
        Ok(fields)
    }
}

fn validate_frozen_read_scope(scope: &AuthorizationScope, frozen: Option<&ValueRef>) -> Result<()> {
    match (scope.fields()?, frozen) {
        (AuthorizationScopeFields::Read { .. }, Some(_)) => Ok(()),
        (AuthorizationScopeFields::EnsureEffect { .. }, None) => Ok(()),
        _ => Err(JournalError::ReferenceMismatch),
    }
}

/// Closed observation outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ObservationOutcomeFields {
    /// The protocol operation returned a typed retained result.
    Returned {
        /// Full result authority.
        result_ref: ValueRef,
    },
    /// The operation was proven not to have entered its external boundary.
    DidNotEnter {
        /// Reviewed bounded failure.
        safe_failure: SafeFailure,
    },
    /// Entry or terminal outcome is indeterminate.
    Indeterminate {
        /// Reviewed bounded failure.
        safe_failure: SafeFailure,
    },
}

impl ObservationOutcome {
    /// Constructs a returned observation.
    pub fn returned(result_ref: &ValueRef) -> Result<Self> {
        Self::from_canonical_value(tagged_object(
            "returned",
            [("result_ref", result_ref.canonical_value()?)],
        )?)
    }

    /// Constructs a proven-not-entered observation.
    pub fn did_not_enter(safe_failure: &SafeFailure) -> Result<Self> {
        Self::from_canonical_value(tagged_object(
            "did_not_enter",
            [("safe_failure", safe_failure.canonical_value()?)],
        )?)
    }

    /// Constructs an indeterminate observation.
    pub fn indeterminate(safe_failure: &SafeFailure) -> Result<Self> {
        Self::from_canonical_value(tagged_object(
            "indeterminate",
            [("safe_failure", safe_failure.canonical_value()?)],
        )?)
    }

    /// Projects the closed observed outcome.
    pub fn fields(&self) -> Result<ObservationOutcomeFields> {
        match super::codec::tag(self)?.as_str() {
            "returned" => Ok(ObservationOutcomeFields::Returned {
                result_ref: ValueRef::from_canonical_value(required_field(self, "result_ref")?)?,
            }),
            "did_not_enter" => Ok(ObservationOutcomeFields::DidNotEnter {
                safe_failure: SafeFailure::from_canonical_value(required_field(
                    self,
                    "safe_failure",
                )?)?,
            }),
            "indeterminate" => Ok(ObservationOutcomeFields::Indeterminate {
                safe_failure: SafeFailure::from_canonical_value(required_field(
                    self,
                    "safe_failure",
                )?)?,
            }),
            _ => Err(super::JournalError::Projection),
        }
    }
}

/// Typed fields of one reviewed redaction-safe failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SafeFailureFields {
    /// Exact selected safe-failure contract.
    pub safe_failure_contract_ref: ContentRef,
    /// Stable code closed by the selected contract.
    pub stable_code: StableId,
    /// Universal reviewed failure class.
    pub failure_class: FailureClass,
    /// Reviewed external-boundary stage.
    pub boundary_stage: BoundaryStage,
    /// Optional reviewed coarse source-envelope size.
    pub coarse_size_class: Option<CoarseSizeClass>,
    /// Optional retained bounded typed diagnostic authority.
    pub diagnostic_ref: Option<ValueRef>,
}

impl SafeFailure {
    /// Constructs one exact annex-owned direct-observation failure envelope.
    ///
    /// The selected capability contract remains responsible for admitting the
    /// stable-code/class/stage/size/diagnostic tuple before this codec boundary.
    /// This type has no provider text, URL, credential, body, path, or arbitrary
    /// diagnostic field.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        safe_failure_contract_ref: &ContentRef,
        stable_code: &StableId,
        failure_class: FailureClass,
        boundary_stage: BoundaryStage,
        coarse_size_class: Option<CoarseSizeClass>,
        diagnostic_ref: Option<&ValueRef>,
    ) -> Result<Self> {
        Self::from_canonical_value(object([
            (
                "version",
                CanonicalValue::String("mfm.safe-failure.v1".to_owned()),
            ),
            (
                "safe_failure_contract_ref",
                cv_content_ref(safe_failure_contract_ref)?,
            ),
            ("stable_code", cv_string(stable_code.as_str())),
            ("failure_class", cv_string(failure_class.as_str())),
            ("boundary_stage", cv_string(boundary_stage.as_str())),
            (
                "coarse_size_class",
                coarse_size_class
                    .map(|value| cv_string(value.as_str()))
                    .unwrap_or(CanonicalValue::Null),
            ),
            (
                "diagnostic_ref",
                diagnostic_ref
                    .map(ValueRef::canonical_value)
                    .transpose()?
                    .unwrap_or(CanonicalValue::Null),
            ),
        ])?)
    }

    /// Projects every reviewed redaction-safe failure field.
    pub fn fields(&self) -> Result<SafeFailureFields> {
        Ok(SafeFailureFields {
            safe_failure_contract_ref: content_ref_field(self, "safe_failure_contract_ref")?,
            stable_code: stable_id_field(self, "stable_code")?,
            failure_class: failure_class_field(self, "failure_class")?,
            boundary_stage: boundary_stage_field(self, "boundary_stage")?,
            coarse_size_class: nullable_field(self, "coarse_size_class")?
                .map(|value| match value {
                    CanonicalValue::String(value) => parse_coarse_size_class(&value),
                    _ => Err(super::JournalError::Projection),
                })
                .transpose()?,
            diagnostic_ref: nullable_field(self, "diagnostic_ref")?
                .map(ValueRef::from_canonical_value)
                .transpose()?,
        })
    }
}

fn failure_class_field(
    value: &impl super::PersistedJournalValue,
    field: &str,
) -> Result<FailureClass> {
    match string_field(value, field)?.as_str() {
        "authorization" => Ok(FailureClass::Authorization),
        "configuration" => Ok(FailureClass::Configuration),
        "request" => Ok(FailureClass::Request),
        "cancellation" => Ok(FailureClass::Cancellation),
        "transport" => Ok(FailureClass::Transport),
        "destination" => Ok(FailureClass::Destination),
        "unrepresentable_response" => Ok(FailureClass::UnrepresentableResponse),
        "integrity" => Ok(FailureClass::Integrity),
        "resource_conflict" => Ok(FailureClass::ResourceConflict),
        "unclassified" => Ok(FailureClass::Unclassified),
        _ => Err(super::JournalError::Projection),
    }
}

fn boundary_stage_field(
    value: &impl super::PersistedJournalValue,
    field: &str,
) -> Result<BoundaryStage> {
    match string_field(value, field)?.as_str() {
        "before_boundary_entry" => Ok(BoundaryStage::BeforeBoundaryEntry),
        "boundary_entry" => Ok(BoundaryStage::BoundaryEntry),
        "boundary_observation" => Ok(BoundaryStage::BoundaryObservation),
        _ => Err(super::JournalError::Projection),
    }
}

fn parse_coarse_size_class(value: &str) -> Result<CoarseSizeClass> {
    match value {
        "zero" => Ok(CoarseSizeClass::Zero),
        "up_to_16_kib" => Ok(CoarseSizeClass::UpTo16Kib),
        "up_to_1_mib" => Ok(CoarseSizeClass::UpTo1Mib),
        "over_1_mib" => Ok(CoarseSizeClass::Over1Mib),
        _ => Err(super::JournalError::Projection),
    }
}

/// Closed executor proof provenance and its exact authority fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecutorProofBasisFields {
    /// Domain evidence verifies without an executor attestation.
    SelfAuthenticatingProof,
    /// The admitted executor evidence authority attests the result.
    ExecutorAttestation {
        /// Exact admitted executor evidence authority.
        evidence_authority_ref: ContentRef,
    },
    /// The admitted trusted observer attests the result.
    TrustedObserver {
        /// Exact admitted trusted-observer authority.
        evidence_authority_ref: ContentRef,
    },
}

impl ExecutorProofBasis {
    /// Constructs self-authenticating proof provenance.
    pub fn self_authenticating_proof() -> Result<Self> {
        Self::from_canonical_value(tagged_object(
            "self_authenticating_proof",
            std::iter::empty::<(String, CanonicalValue)>(),
        )?)
    }

    /// Constructs executor-attestation proof provenance.
    pub fn executor_attestation(evidence_authority_ref: &ContentRef) -> Result<Self> {
        Self::authority("executor_attestation", evidence_authority_ref)
    }

    /// Constructs trusted-observer proof provenance.
    pub fn trusted_observer(evidence_authority_ref: &ContentRef) -> Result<Self> {
        Self::authority("trusted_observer", evidence_authority_ref)
    }

    fn authority(kind: &str, evidence_authority_ref: &ContentRef) -> Result<Self> {
        Self::from_canonical_value(tagged_object(
            kind,
            [(
                "evidence_authority_ref",
                cv_content_ref(evidence_authority_ref)?,
            )],
        )?)
    }

    /// Projects the closed proof variant and its exact authority fields.
    pub fn fields(&self) -> Result<ExecutorProofBasisFields> {
        match super::codec::tag(self)?.as_str() {
            "self_authenticating_proof" => Ok(ExecutorProofBasisFields::SelfAuthenticatingProof),
            "executor_attestation" => Ok(ExecutorProofBasisFields::ExecutorAttestation {
                evidence_authority_ref: content_ref_field(self, "evidence_authority_ref")?,
            }),
            "trusted_observer" => Ok(ExecutorProofBasisFields::TrustedObserver {
                evidence_authority_ref: content_ref_field(self, "evidence_authority_ref")?,
            }),
            _ => Err(super::JournalError::Projection),
        }
    }
}

/// Typed fields of complete terminal executor evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalEffectEvidenceFields {
    /// Exact immutable executor binding.
    pub executor_binding_ref: CapabilityBindingRef,
    /// Deterministic effect identity.
    pub effect_key: EffectKey,
    /// Immutable semantic request digest.
    pub request_digest: RequestDigest,
    /// Complete retained delivery frontier.
    pub delivery_audit_ref: ValueRef,
    /// Terminal tombstone within that frontier.
    pub terminal_tombstone_ref: ValueRef,
    /// Reviewed public external operation identity.
    pub external_operation_identity: StableId,
    /// Closed terminal outcome.
    pub terminal_outcome: StableId,
    /// Exact assurance policy.
    pub assurance_policy_ref: ContentRef,
    /// Closed proof provenance.
    pub proof_basis: ExecutorProofBasis,
    /// Full retained domain evidence authority.
    pub domain_evidence_ref: ValueRef,
}

impl TerminalEffectEvidence {
    /// Constructs and validates complete terminal executor evidence.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        executor_binding_ref: &CapabilityBindingRef,
        effect_key: &EffectKey,
        request_digest: &RequestDigest,
        delivery_audit_ref: &ValueRef,
        terminal_tombstone_ref: &ValueRef,
        external_operation_identity: &StableId,
        terminal_outcome: &StableId,
        assurance_policy_ref: &ContentRef,
        proof_basis: &ExecutorProofBasis,
        domain_evidence_ref: &ValueRef,
    ) -> Result<Self> {
        Self::from_canonical_value(object([
            (
                "version",
                CanonicalValue::String("mfm.terminal-effect-evidence.v1".to_owned()),
            ),
            (
                "executor_binding_ref",
                executor_binding_ref.canonical_value()?,
            ),
            ("effect_key", cv_string(effect_key.as_str())),
            ("request_digest", cv_string(request_digest.as_str())),
            ("delivery_audit_ref", delivery_audit_ref.canonical_value()?),
            (
                "terminal_tombstone_ref",
                terminal_tombstone_ref.canonical_value()?,
            ),
            (
                "external_operation_identity",
                cv_string(external_operation_identity.as_str()),
            ),
            ("terminal_outcome", cv_string(terminal_outcome.as_str())),
            (
                "assurance_policy_ref",
                cv_content_ref(assurance_policy_ref)?,
            ),
            ("proof_basis", proof_basis.canonical_value()?),
            (
                "domain_evidence_ref",
                domain_evidence_ref.canonical_value()?,
            ),
        ])?)
    }

    /// Projects every complete terminal executor-evidence field.
    pub fn fields(&self) -> Result<TerminalEffectEvidenceFields> {
        Ok(TerminalEffectEvidenceFields {
            executor_binding_ref: CapabilityBindingRef::from_canonical_value(required_field(
                self,
                "executor_binding_ref",
            )?)?,
            effect_key: effect_key_field(self, "effect_key")?,
            request_digest: RequestDigest::parse(string_field(self, "request_digest")?)?,
            delivery_audit_ref: ValueRef::from_canonical_value(required_field(
                self,
                "delivery_audit_ref",
            )?)?,
            terminal_tombstone_ref: ValueRef::from_canonical_value(required_field(
                self,
                "terminal_tombstone_ref",
            )?)?,
            external_operation_identity: stable_id_field(self, "external_operation_identity")?,
            terminal_outcome: stable_id_field(self, "terminal_outcome")?,
            assurance_policy_ref: content_ref_field(self, "assurance_policy_ref")?,
            proof_basis: ExecutorProofBasis::from_canonical_value(required_field(
                self,
                "proof_basis",
            )?)?,
            domain_evidence_ref: ValueRef::from_canonical_value(required_field(
                self,
                "domain_evidence_ref",
            )?)?,
        })
    }
}

/// Closed persisted result returned by one durable executor ensure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecutorEnsureResultFields {
    /// Delivery remains pending with a complete frontier audit.
    Pending {
        /// Full retained delivery-audit authority.
        delivery_audit_ref: ValueRef,
    },
    /// Complete terminal evidence is available for settlement.
    Terminal {
        /// Full retained authority for separately stored terminal effect evidence.
        evidence_ref: ValueRef,
    },
}

impl ExecutorEnsureResult {
    /// Constructs a pending ensure result with its mandatory delivery audit.
    pub fn pending(delivery_audit_ref: &ValueRef) -> Result<Self> {
        Self::from_canonical_value(tagged_object(
            "pending",
            [("delivery_audit_ref", delivery_audit_ref.canonical_value()?)],
        )?)
    }

    /// Constructs a terminal ensure result referencing separately retained evidence.
    pub fn terminal(evidence_ref: &ValueRef) -> Result<Self> {
        Self::from_canonical_value(tagged_object(
            "terminal",
            [("evidence_ref", evidence_ref.canonical_value()?)],
        )?)
    }

    /// Projects the closed ensure-result variant and all typed fields.
    pub fn fields(&self) -> Result<ExecutorEnsureResultFields> {
        match super::codec::tag(self)?.as_str() {
            "pending" => Ok(ExecutorEnsureResultFields::Pending {
                delivery_audit_ref: ValueRef::from_canonical_value(required_field(
                    self,
                    "delivery_audit_ref",
                )?)?,
            }),
            "terminal" => Ok(ExecutorEnsureResultFields::Terminal {
                evidence_ref: ValueRef::from_canonical_value(required_field(
                    self,
                    "evidence_ref",
                )?)?,
            }),
            _ => Err(super::JournalError::Projection),
        }
    }
}

/// Typed fields of a durable external-access observation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalAccessObservedFields {
    /// Exact authorization consumed by this observation.
    pub authorization_ref: AuthorizationRef,
    /// Closed observed result.
    pub outcome: ObservationOutcome,
    /// Full retained store-authored scan-attestation authority.
    pub fact_selection_scan_attestation_ref: Option<ValueRef>,
}

impl ExternalAccessObserved {
    /// Constructs and validates a complete external-access observation record.
    pub fn new(
        authorization_ref: &AuthorizationRef,
        outcome: &ObservationOutcome,
        fact_selection_scan_attestation_ref: Option<&ValueRef>,
    ) -> Result<Self> {
        validate_scan_attestation_presence(outcome, fact_selection_scan_attestation_ref)?;
        Self::from_canonical_value(object([
            (
                "version",
                CanonicalValue::String("mfm.external-access-observed.v1".to_owned()),
            ),
            ("authorization_ref", authorization_ref.canonical_value()?),
            ("outcome", outcome.canonical_value()?),
            (
                "fact_selection_scan_attestation_ref",
                fact_selection_scan_attestation_ref
                    .map(ValueRef::canonical_value)
                    .transpose()?
                    .unwrap_or(CanonicalValue::Null),
            ),
        ])?)
    }

    /// Projects every observation field.
    pub fn fields(&self) -> Result<ExternalAccessObservedFields> {
        let fields = ExternalAccessObservedFields {
            authorization_ref: AuthorizationRef::from_canonical_value(required_field(
                self,
                "authorization_ref",
            )?)?,
            outcome: ObservationOutcome::from_canonical_value(required_field(self, "outcome")?)?,
            fact_selection_scan_attestation_ref: nullable_field(
                self,
                "fact_selection_scan_attestation_ref",
            )?
            .map(ValueRef::from_canonical_value)
            .transpose()?,
        };
        validate_scan_attestation_presence(
            &fields.outcome,
            fields.fact_selection_scan_attestation_ref.as_ref(),
        )?;
        Ok(fields)
    }
}

fn validate_scan_attestation_presence(
    outcome: &ObservationOutcome,
    attestation: Option<&ValueRef>,
) -> Result<()> {
    match (outcome.fields()?, attestation) {
        (ObservationOutcomeFields::Returned { .. }, _) | (_, None) => Ok(()),
        _ => Err(JournalError::ReferenceMismatch),
    }
}

/// Typed fields of the reserved fact-selection scan contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactSelectionScanContractFields {
    /// Reserved capability operation that performs the authoritative scan.
    pub capability_operation_id: StableId,
    /// Complete retained request-value contract.
    pub request_contract: RetainedValueContract,
    /// Complete retained response-value contract.
    pub response_contract: RetainedValueContract,
    /// Complete retained scan-attestation contract.
    pub scan_attestation_contract: RetainedValueContract,
}

impl FactSelectionScanContract {
    /// Constructs one frozen reserved fact-selection scan contract.
    pub fn new(
        capability_operation_id: &StableId,
        request_contract: &RetainedValueContract,
        response_contract: &RetainedValueContract,
        scan_attestation_contract: &RetainedValueContract,
    ) -> Result<Self> {
        Self::from_canonical_value(object([
            (
                "version",
                CanonicalValue::String("mfm.fact-selection-scan-contract.v1".to_owned()),
            ),
            (
                "capability_operation_id",
                cv_string(capability_operation_id.as_str()),
            ),
            (
                "request_contract",
                cv_retained_value_contract(request_contract)?,
            ),
            (
                "response_contract",
                cv_retained_value_contract(response_contract)?,
            ),
            (
                "scan_attestation_contract",
                cv_retained_value_contract(scan_attestation_contract)?,
            ),
        ])?)
    }

    /// Projects every reserved fact-selection scan contract field.
    pub fn fields(&self) -> Result<FactSelectionScanContractFields> {
        Ok(FactSelectionScanContractFields {
            capability_operation_id: stable_id_field(self, "capability_operation_id")?,
            request_contract: retained_value_contract_from_canonical(required_field(
                self,
                "request_contract",
            )?)?,
            response_contract: retained_value_contract_from_canonical(required_field(
                self,
                "response_contract",
            )?)?,
            scan_attestation_contract: retained_value_contract_from_canonical(required_field(
                self,
                "scan_attestation_contract",
            )?)?,
        })
    }
}

/// Typed fields of one immutable admitted read-capability binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadCapabilityBindingFields {
    /// Frozen reviewed capability contract.
    pub capability_contract_ref: ContentRef,
    /// Exact admitted implementation identity.
    pub admitted_implementation_ref: ContentRef,
    /// Frozen safe-classifier contract.
    pub safe_classifier_contract_ref: ContentRef,
    /// Frozen safe-failure contract.
    pub safe_failure_contract_ref: ContentRef,
    /// Frozen reviewed source scope.
    pub reviewed_source_scope_ref: ContentRef,
    /// Frozen aggregate catalog of every admitted per-call routing generation.
    pub routing_catalog_ref: ContentRef,
}

impl ReadCapabilityBinding {
    /// Constructs one immutable admitted read-capability binding.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        capability_contract_ref: &ContentRef,
        admitted_implementation_ref: &ContentRef,
        safe_classifier_contract_ref: &ContentRef,
        safe_failure_contract_ref: &ContentRef,
        reviewed_source_scope_ref: &ContentRef,
        routing_catalog_ref: &ContentRef,
    ) -> Result<Self> {
        Self::from_canonical_value(object([
            (
                "version",
                CanonicalValue::String("mfm.read-capability-binding.v1".to_owned()),
            ),
            (
                "capability_contract_ref",
                cv_content_ref(capability_contract_ref)?,
            ),
            (
                "admitted_implementation_ref",
                cv_content_ref(admitted_implementation_ref)?,
            ),
            (
                "safe_classifier_contract_ref",
                cv_content_ref(safe_classifier_contract_ref)?,
            ),
            (
                "safe_failure_contract_ref",
                cv_content_ref(safe_failure_contract_ref)?,
            ),
            (
                "reviewed_source_scope_ref",
                cv_content_ref(reviewed_source_scope_ref)?,
            ),
            ("routing_catalog_ref", cv_content_ref(routing_catalog_ref)?),
        ])?)
    }

    /// Projects every immutable read-capability binding field.
    pub fn fields(&self) -> Result<ReadCapabilityBindingFields> {
        Ok(ReadCapabilityBindingFields {
            capability_contract_ref: content_ref_field(self, "capability_contract_ref")?,
            admitted_implementation_ref: content_ref_field(self, "admitted_implementation_ref")?,
            safe_classifier_contract_ref: content_ref_field(self, "safe_classifier_contract_ref")?,
            safe_failure_contract_ref: content_ref_field(self, "safe_failure_contract_ref")?,
            reviewed_source_scope_ref: content_ref_field(self, "reviewed_source_scope_ref")?,
            routing_catalog_ref: content_ref_field(self, "routing_catalog_ref")?,
        })
    }
}

/// Typed fields of one store-authored fact-selection scan attestation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactSelectionScanAttestationFields {
    /// Exact physical store scope that owns the scanned fact family.
    pub store_scope_id: StoreScopeId,
    /// Exact store epoch containing the complete prefix.
    pub store_epoch: StoreEpoch,
    /// Tenant whose authoritative fact prefix was scanned.
    pub tenant_scope_id: TenantScopeId,
    /// Authorization whose affine scan produced the response.
    pub authorization_ref: AuthorizationRef,
    /// Exact authored fact-selection request digest.
    pub request_digest: FactQueryDigest,
    /// Complete tenant fact frontier used as the scan barrier.
    pub frontier: TenantFactFrontier,
    /// Full retained response authority.
    pub response_ref: ValueRef,
    /// Digest of the complete retained response closure.
    pub response_closure_digest: SemanticDigest,
}

impl FactSelectionScanAttestation {
    /// Constructs one exact store-authored complete-scan attestation.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        store_scope_id: &StoreScopeId,
        store_epoch: &StoreEpoch,
        tenant_scope_id: &TenantScopeId,
        authorization_ref: &AuthorizationRef,
        request_digest: &FactQueryDigest,
        frontier: &TenantFactFrontier,
        response_ref: &ValueRef,
        response_closure_digest: &SemanticDigest,
    ) -> Result<Self> {
        Self::from_canonical_value(object([
            (
                "version",
                CanonicalValue::String("mfm.fact-selection-scan-attestation.v1".to_owned()),
            ),
            ("store_scope_id", cv_string(store_scope_id.as_str())),
            ("store_epoch", cv_decimal(store_epoch.get())?),
            ("tenant_scope_id", cv_string(tenant_scope_id.as_str())),
            ("authorization_ref", authorization_ref.canonical_value()?),
            ("request_digest", cv_string(request_digest.as_str())),
            ("frontier", frontier.canonical_value()?),
            ("response_ref", response_ref.canonical_value()?),
            (
                "response_closure_digest",
                cv_string(response_closure_digest.as_str()),
            ),
        ])?)
    }

    /// Projects every complete-scan attestation field.
    pub fn fields(&self) -> Result<FactSelectionScanAttestationFields> {
        Ok(FactSelectionScanAttestationFields {
            store_scope_id: store_scope_id_field(self, "store_scope_id")?,
            store_epoch: store_epoch_field(self, "store_epoch")?,
            tenant_scope_id: tenant_scope_id_field(self, "tenant_scope_id")?,
            authorization_ref: AuthorizationRef::from_canonical_value(required_field(
                self,
                "authorization_ref",
            )?)?,
            request_digest: FactQueryDigest::parse(string_field(self, "request_digest")?)?,
            frontier: TenantFactFrontier::from_canonical_value(required_field(self, "frontier")?)?,
            response_ref: ValueRef::from_canonical_value(required_field(self, "response_ref")?)?,
            response_closure_digest: SemanticDigest::parse(string_field(
                self,
                "response_closure_digest",
            )?)?,
        })
    }
}

/// Typed fields of an immutable frozen read intent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrozenReadIntentFields {
    /// Exact certified node occurrence.
    pub node_id: NodeId,
    /// Exact frozen input manifest.
    pub input_manifest_ref: InputManifestRef,
    /// State contract that authored the request.
    pub state_contract_ref: ContentRef,
    /// Immutable admitted capability binding.
    pub capability_binding_ref: CapabilityBindingRef,
    /// Immutable admitted routing generation.
    pub routing_generation_ref: ContentRef,
    /// Reviewed capability operation.
    pub capability_operation_id: StableId,
    /// Full retained request authority.
    pub request_ref: ValueRef,
    /// Immutable semantic request digest.
    pub request_digest: RequestDigest,
    /// Complete certified request-value contract.
    pub request_contract: RetainedValueContract,
    /// Complete certified returned-value contract.
    pub returned_contract: RetainedValueContract,
    /// Complete certified safe-failure diagnostic contract.
    pub safe_failure_contract: RetainedValueContract,
}

impl FrozenReadIntent {
    /// Returns the deterministic retained contract for immutable read intents.
    pub fn retained_contract() -> Result<RetainedValueContract> {
        super::runtime_retained_contract(
            "mfm.frozen-read-intent.v1",
            "frozen-read-intent",
            "mfm.runtime.frozen-read-intent",
        )
    }

    /// Constructs and validates one immutable first-authorization read intent.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        node_id: &NodeId,
        input_manifest_ref: &InputManifestRef,
        state_contract_ref: &ContentRef,
        capability_binding_ref: &CapabilityBindingRef,
        routing_generation_ref: &ContentRef,
        capability_operation_id: &StableId,
        request_ref: &ValueRef,
        request_digest: &RequestDigest,
        request_contract: &RetainedValueContract,
        returned_contract: &RetainedValueContract,
        safe_failure_contract: &RetainedValueContract,
    ) -> Result<Self> {
        request_ref.validate_contract(request_contract)?;
        Self::from_canonical_value(object([
            (
                "version",
                CanonicalValue::String("mfm.frozen-read-intent.v1".to_owned()),
            ),
            ("node_id", cv_string(node_id.as_str())),
            ("input_manifest_ref", input_manifest_ref.canonical_value()?),
            ("state_contract_ref", cv_content_ref(state_contract_ref)?),
            (
                "capability_binding_ref",
                capability_binding_ref.canonical_value()?,
            ),
            (
                "routing_generation_ref",
                cv_content_ref(routing_generation_ref)?,
            ),
            (
                "capability_operation_id",
                cv_string(capability_operation_id.as_str()),
            ),
            ("request_ref", request_ref.canonical_value()?),
            ("request_digest", cv_string(request_digest.as_str())),
            (
                "request_contract",
                cv_retained_value_contract(request_contract)?,
            ),
            (
                "returned_contract",
                cv_retained_value_contract(returned_contract)?,
            ),
            (
                "safe_failure_contract",
                cv_retained_value_contract(safe_failure_contract)?,
            ),
        ])?)
    }

    /// Projects every immutable read-intent field.
    pub fn fields(&self) -> Result<FrozenReadIntentFields> {
        let fields = FrozenReadIntentFields {
            node_id: node_id_field(self, "node_id")?,
            input_manifest_ref: InputManifestRef::from_canonical_value(required_field(
                self,
                "input_manifest_ref",
            )?)?,
            state_contract_ref: content_ref_field(self, "state_contract_ref")?,
            capability_binding_ref: CapabilityBindingRef::from_canonical_value(required_field(
                self,
                "capability_binding_ref",
            )?)?,
            routing_generation_ref: content_ref_field(self, "routing_generation_ref")?,
            capability_operation_id: stable_id_field(self, "capability_operation_id")?,
            request_ref: ValueRef::from_canonical_value(required_field(self, "request_ref")?)?,
            request_digest: RequestDigest::parse(string_field(self, "request_digest")?)?,
            request_contract: retained_value_contract_from_canonical(required_field(
                self,
                "request_contract",
            )?)?,
            returned_contract: retained_value_contract_from_canonical(required_field(
                self,
                "returned_contract",
            )?)?,
            safe_failure_contract: retained_value_contract_from_canonical(required_field(
                self,
                "safe_failure_contract",
            )?)?,
        };
        fields
            .request_ref
            .validate_contract(&fields.request_contract)?;
        Ok(fields)
    }
}

/// Redaction-safe audit status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AccessAuditStatus {
    /// Authorization has no committed observation.
    AuthorizedUnobserved,
    /// The operation returned a typed value.
    Returned,
    /// The operation provably did not enter.
    DidNotEnter,
    /// The operation's entry or outcome is indeterminate.
    Indeterminate,
}

impl AccessAuditStatus {
    /// Returns the frozen canonical spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AuthorizedUnobserved => "authorized_unobserved",
            Self::Returned => "returned",
            Self::DidNotEnter => "did_not_enter",
            Self::Indeterminate => "indeterminate",
        }
    }

    fn parse(value: &str) -> Result<Self> {
        match value {
            "authorized_unobserved" => Ok(Self::AuthorizedUnobserved),
            "returned" => Ok(Self::Returned),
            "did_not_enter" => Ok(Self::DidNotEnter),
            "indeterminate" => Ok(Self::Indeterminate),
            _ => Err(super::JournalError::Projection),
        }
    }
}

fn validate_access_audit_delivery(
    status: AccessAuditStatus,
    effect_key: Option<&EffectKey>,
    delivery_audit_ref: Option<&ValueRef>,
) -> Result<()> {
    let requires_delivery_audit = status == AccessAuditStatus::Returned && effect_key.is_some();
    if delivery_audit_ref.is_some() == requires_delivery_audit {
        Ok(())
    } else {
        Err(super::JournalError::AccessAuditMismatch)
    }
}

/// Typed fields of a safe access-audit entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessAuditEntryFields {
    /// Exact authorization.
    pub authorization_ref: AuthorizationRef,
    /// Optional exact observation.
    pub observation_ref: Option<ObservationRef>,
    /// Closed safe status.
    pub status: AccessAuditStatus,
    /// Optional reviewed failure.
    pub failure: Option<SafeFailure>,
    /// Optional effect key.
    pub effect_key: Option<EffectKey>,
    /// Optional greatest retained executor delivery-audit head.
    pub delivery_audit_ref: Option<ValueRef>,
}

impl AccessAuditEntry {
    /// Constructs one redaction-safe audit projection.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        authorization_ref: &AuthorizationRef,
        observation_ref: Option<&ObservationRef>,
        status: AccessAuditStatus,
        failure: Option<&SafeFailure>,
        effect_key: Option<&EffectKey>,
        delivery_audit_ref: Option<&ValueRef>,
    ) -> Result<Self> {
        validate_access_audit_delivery(status, effect_key, delivery_audit_ref)?;
        Self::from_canonical_value(object([
            (
                "version",
                CanonicalValue::String("mfm.access-audit-entry.v1".to_owned()),
            ),
            ("authorization_ref", authorization_ref.canonical_value()?),
            (
                "observation_ref",
                observation_ref
                    .map(ObservationRef::canonical_value)
                    .transpose()?
                    .unwrap_or(CanonicalValue::Null),
            ),
            ("status", cv_string(status.as_str())),
            (
                "failure",
                failure
                    .map(SafeFailure::canonical_value)
                    .transpose()?
                    .unwrap_or(CanonicalValue::Null),
            ),
            (
                "effect_key",
                effect_key
                    .map(|value| cv_string(value.as_str()))
                    .unwrap_or(CanonicalValue::Null),
            ),
            (
                "delivery_audit_ref",
                delivery_audit_ref
                    .map(ValueRef::canonical_value)
                    .transpose()?
                    .unwrap_or(CanonicalValue::Null),
            ),
        ])?)
    }

    /// Projects every safe access-audit field.
    pub fn fields(&self) -> Result<AccessAuditEntryFields> {
        let fields = AccessAuditEntryFields {
            authorization_ref: AuthorizationRef::from_canonical_value(required_field(
                self,
                "authorization_ref",
            )?)?,
            observation_ref: nullable_field(self, "observation_ref")?
                .map(ObservationRef::from_canonical_value)
                .transpose()?,
            status: AccessAuditStatus::parse(&string_field(self, "status")?)?,
            failure: nullable_field(self, "failure")?
                .map(SafeFailure::from_canonical_value)
                .transpose()?,
            effect_key: nullable_field(self, "effect_key")?
                .map(|value| match value {
                    CanonicalValue::String(value) => value.parse().map_err(Into::into),
                    _ => Err(super::JournalError::Projection),
                })
                .transpose()?,
            delivery_audit_ref: nullable_field(self, "delivery_audit_ref")?
                .map(ValueRef::from_canonical_value)
                .transpose()?,
        };
        validate_access_audit_delivery(
            fields.status,
            fields.effect_key.as_ref(),
            fields.delivery_audit_ref.as_ref(),
        )?;
        Ok(fields)
    }

    /// Validates an unobserved audit row against its exact authorization.
    pub fn validate_unobserved(&self, authorization_ref: &AuthorizationRef) -> Result<()> {
        let fields = self.fields()?;
        if fields.authorization_ref == *authorization_ref
            && fields.observation_ref.is_none()
            && fields.status == AccessAuditStatus::AuthorizedUnobserved
            && fields.failure.is_none()
        {
            Ok(())
        } else {
            Err(super::JournalError::AccessAuditMismatch)
        }
    }

    /// Validates an observed audit row against exact journal evidence.
    pub fn validate_observation(
        &self,
        observation_ref: &ObservationRef,
        observation: &ExternalAccessObserved,
    ) -> Result<()> {
        let fields = self.fields()?;
        let observed = observation.fields()?;
        if fields.authorization_ref != observed.authorization_ref
            || fields.observation_ref.as_ref() != Some(observation_ref)
        {
            return Err(super::JournalError::AccessAuditMismatch);
        }

        let outcome_matches = match observed.outcome.fields()? {
            ObservationOutcomeFields::Returned { .. } => {
                fields.status == AccessAuditStatus::Returned && fields.failure.is_none()
            }
            ObservationOutcomeFields::DidNotEnter { safe_failure } => {
                fields.status == AccessAuditStatus::DidNotEnter
                    && fields.failure.as_ref() == Some(&safe_failure)
            }
            ObservationOutcomeFields::Indeterminate { safe_failure } => {
                fields.status == AccessAuditStatus::Indeterminate
                    && fields.failure.as_ref() == Some(&safe_failure)
            }
        };
        if outcome_matches {
            Ok(())
        } else {
            Err(super::JournalError::AccessAuditMismatch)
        }
    }
}

impl TerminalEvidencePreimage {
    /// Constructs the terminal-effect evidence preimage.
    pub fn new(evidence: &TerminalEffectEvidence) -> Result<Self> {
        Self::from_canonical_value(object([("evidence", evidence.canonical_value()?)])?)
    }

    /// Derives the frozen terminal-effect evidence digest.
    pub fn evidence_digest(&self) -> Result<TerminalEffectEvidenceDigest> {
        domain_digest("mfm.terminal-effect-evidence.v1", self)
            .map(TerminalEffectEvidenceDigest::from_semantic_digest)
    }
}
