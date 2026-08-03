use std::sync::Arc;

use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_facts::FactSet;
use mfm_ids::{
    AccessAttemptId, AppendRequestId, ContentDigest, ContentRef, InvocationIdentity, RunId,
    StableId, TenantScopeId,
};
use mfm_journal::structured::{
    CommittedBatch, ExternalAccessAuthorized, HistoryObject, LexicalValueRef,
    PriorRunFactSourceManifest, RecordRef, RunRecord, TenantFactCoordinate,
    ADMISSION_CONFIGURATION_OBJECT_TYPE, ADMISSION_CONTEXT_MANIFEST_OBJECT_TYPE,
    ADMISSION_PRIOR_RUN_SOURCE_MANIFEST_OBJECT_TYPE, ADMISSION_ROUTING_POLICY_OBJECT_TYPE,
};
use mfm_spec::structured::CertifiedProgramDocument;
use serde::Serialize;

use super::backend::{
    prior_run_fact_source, BackendAppendOutcome, StructuredHistoryBackend,
    StructuredRunHistoryWriter, ValidatedBatch,
};
use super::fact_scan::{fact_scan_permit, FactScanPermit, PriorRunFactScanCompletion};
use super::fold::{
    authorization_requires_fact_selection_barrier, prepare_admission, prepare_authorization,
    prepare_observation, prepare_state_transition,
    qualify_observation as qualify_observation_material, ObservationQualification,
    PreparedObservation, PreparedSuccessor, VerifiedStructuredRun,
};

/// Producer-free canonical value proposed by a qualified callback or admission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProposedCanonicalValue {
    canonical: PlainCanonicalJsonBytes,
}

/// Exact producer-free public objects selected before run admission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructuredAdmissionMaterial {
    pub(super) configuration: HistoryObject,
    pub(super) context_manifest: HistoryObject,
    pub(super) prior_run_source_manifest: HistoryObject,
    pub(super) routing_policy: HistoryObject,
    pub(super) stable_resource_lineage_contract_refs: Vec<ContentRef>,
}

impl StructuredAdmissionMaterial {
    /// Validates and canonically orders one complete non-secret admission bundle.
    pub fn new(
        configuration: HistoryObject,
        context_manifest: HistoryObject,
        prior_run_source_manifest: HistoryObject,
        routing_policy: HistoryObject,
        mut stable_resource_lineage_contract_refs: Vec<ContentRef>,
    ) -> super::Result<Self> {
        validate_admission_object(&configuration, ADMISSION_CONFIGURATION_OBJECT_TYPE)?;
        validate_admission_object(&context_manifest, ADMISSION_CONTEXT_MANIFEST_OBJECT_TYPE)?;
        validate_admission_object(
            &prior_run_source_manifest,
            ADMISSION_PRIOR_RUN_SOURCE_MANIFEST_OBJECT_TYPE,
        )?;
        PriorRunFactSourceManifest::from_history_object(&prior_run_source_manifest)
            .map_err(|_| super::StructuredStoreError::InvalidHistory)?;
        validate_admission_object(&routing_policy, ADMISSION_ROUTING_POLICY_OBJECT_TYPE)?;
        stable_resource_lineage_contract_refs.sort();
        if stable_resource_lineage_contract_refs
            .windows(2)
            .any(|pair| pair[0] == pair[1])
        {
            return Err(super::StructuredStoreError::InvalidHistory);
        }
        let mut all_refs = vec![
            &configuration.content_ref,
            &context_manifest.content_ref,
            &prior_run_source_manifest.content_ref,
            &routing_policy.content_ref,
        ];
        all_refs.extend(stable_resource_lineage_contract_refs.iter());
        all_refs.sort();
        if all_refs.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(super::StructuredStoreError::InvalidHistory);
        }
        Ok(Self {
            configuration,
            context_manifest,
            prior_run_source_manifest,
            routing_policy,
            stable_resource_lineage_contract_refs,
        })
    }
}

fn validate_admission_object(object: &HistoryObject, expected_type: &str) -> super::Result<()> {
    object
        .validate()
        .map_err(|_| super::StructuredStoreError::InvalidHistory)?;
    if object.object_type.as_str() != expected_type {
        return Err(super::StructuredStoreError::InvalidHistory);
    }
    Ok(())
}

impl ProposedCanonicalValue {
    /// Canonicalizes one float-free serializable value.
    pub fn from_value<T: Serialize>(value: &T) -> super::Result<Self> {
        let json = serde_json::to_string(value)
            .map_err(|_| super::StructuredStoreError::InvalidHistory)?;
        Self::from_json(&json)
    }

    /// Parses one exact float-free JSON value into canonical bytes.
    pub fn from_json(json: &str) -> super::Result<Self> {
        let canonical = PlainCanonicalJsonBytes::from_json_str(json)
            .map_err(|_| super::StructuredStoreError::InvalidHistory)?;
        Ok(Self { canonical })
    }

    /// Returns the exact canonical proposal bytes.
    pub const fn canonical(&self) -> &PlainCanonicalJsonBytes {
        &self.canonical
    }
}

/// Complete producer-free request to admit one certified run.
pub struct StructuredAdmissionRequest {
    pub(super) run_id: RunId,
    pub(super) tenant_scope_id: TenantScopeId,
    pub(super) invocation_identity: InvocationIdentity,
    pub(super) entry_point_operation_id: StableId,
    pub(super) certified_program: CertifiedProgramDocument,
    pub(super) material: StructuredAdmissionMaterial,
    pub(super) initial_values: Vec<ProposedCanonicalValue>,
    pub(super) append_request_id: AppendRequestId,
}

impl StructuredAdmissionRequest {
    /// Binds one exact qualified program document and declaration-ordered roots.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        run_id: RunId,
        tenant_scope_id: TenantScopeId,
        invocation_identity: InvocationIdentity,
        entry_point_operation_id: StableId,
        certified_program: CertifiedProgramDocument,
        material: StructuredAdmissionMaterial,
        initial_values: Vec<ProposedCanonicalValue>,
        append_request_id: AppendRequestId,
    ) -> Self {
        Self {
            run_id,
            tenant_scope_id,
            invocation_identity,
            entry_point_operation_id,
            certified_program,
            material,
            initial_values,
            append_request_id,
        }
    }

    /// Returns the exact run this admission would create.
    pub const fn run_id(&self) -> &RunId {
        &self.run_id
    }
}

/// Producer-free state callback result selected for one current occurrence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProposedTransitionValue {
    /// Successful state output plus exact fact proposals.
    Success {
        /// Canonical successful value.
        value: ProposedCanonicalValue,
        /// Exact callback-authored facts.
        facts: FactSet,
    },
    /// Typed state failure; failed transitions cannot emit facts.
    Failure(ProposedCanonicalValue),
}

/// One physical transition append request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateTransitionProposal {
    pub(super) append_request_id: AppendRequestId,
    pub(super) value: ProposedTransitionValue,
}

impl StateTransitionProposal {
    /// Constructs one successful callback proposal.
    pub fn success(
        append_request_id: AppendRequestId,
        value: ProposedCanonicalValue,
        facts: FactSet,
    ) -> Self {
        Self {
            append_request_id,
            value: ProposedTransitionValue::Success { value, facts },
        }
    }

    /// Constructs one typed failure callback proposal.
    pub fn failure(append_request_id: AppendRequestId, value: ProposedCanonicalValue) -> Self {
        Self {
            append_request_id,
            value: ProposedTransitionValue::Failure(value),
        }
    }
}

/// Exact current input, request, and public certificate for one current access.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessAuthorizationProposal {
    pub(super) append_request_id: AppendRequestId,
    pub(super) state_input_ref: LexicalValueRef,
    pub(super) request: ProposedCanonicalValue,
    pub(super) physical_binding_certificate: HistoryObject,
}

impl AccessAuthorizationProposal {
    /// Constructs one access proposal from a sealed binding's public material.
    pub fn new(
        append_request_id: AppendRequestId,
        state_input_ref: LexicalValueRef,
        request: ProposedCanonicalValue,
        physical_binding_certificate: HistoryObject,
    ) -> Self {
        Self {
            append_request_id,
            state_input_ref,
            request,
            physical_binding_certificate,
        }
    }
}

/// Producer-free completion returned after consuming one affine invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProposedObservationOutcome {
    /// Exact returned response.
    Returned(ProposedCanonicalValue),
    /// Exact reviewed state-facing safe failure.
    SafeFailure(ProposedCanonicalValue),
    /// Qualified proof that a refreshable Effect did not enter.
    SupersededBeforeEntry {
        /// Current public lineage-head certificate.
        public_lineage_head: Box<HistoryObject>,
        /// Exact typed refresh evidence.
        evidence: ProposedCanonicalValue,
    },
    /// Effect entry may have happened.
    EntryUnknown {
        /// Stable reviewed redaction-safe fault code.
        fault_code: StableId,
    },
    /// Integrity evidence blocks semantic progress.
    IntegrityFault {
        /// Stable reviewed redaction-safe fault code.
        fault_code: StableId,
    },
}

/// Stable observation material independent of a predecessor-bound envelope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessObservationProposal {
    pub(super) append_request_id: AppendRequestId,
    pub(super) authorization_ref: RecordRef,
    pub(super) outcome: ProposedObservationOutcome,
}

impl AccessObservationProposal {
    /// Binds one completion to the exact committed authorization it consumed.
    pub fn new(
        append_request_id: AppendRequestId,
        authorization_ref: RecordRef,
        outcome: ProposedObservationOutcome,
    ) -> Self {
        Self {
            append_request_id,
            authorization_ref,
            outcome,
        }
    }
}

/// Result of one exact physical append attempt, including ambiguity identity.
#[derive(Debug)]
pub struct StructuredAppendAttempt {
    append_request_id: AppendRequestId,
    candidate_digest: ContentDigest,
    candidate: CommittedBatch,
    outcome: BackendAppendOutcome,
    successor: Option<VerifiedStructuredRun>,
    fact_scan_permit: Option<FactScanPermit>,
}

/// Store-owned resolution of one stable pending observation.
#[derive(Debug)]
pub enum ObservationCommit {
    /// The exact logical observation already exists with identical content.
    ExistingSame(Box<VerifiedStructuredRun>),
    /// One predecessor-bound physical append was attempted.
    Attempt(Box<StructuredAppendAttempt>),
}

/// Non-cloneable proof that this process directly observed one newly committed
/// external-access authorization.
///
/// Existing content, resolved acknowledgement ambiguity, and unrelated append
/// families cannot construct this proof.
pub struct NewlyAppendedAuthorization {
    authorization_ref: RecordRef,
    authorization: ExternalAccessAuthorized,
    fact_scan_permit: Option<FactScanPermit>,
}

impl std::fmt::Debug for NewlyAppendedAuthorization {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NewlyAppendedAuthorization")
            .field("authorization_ref", &self.authorization_ref)
            .field("access_attempt_id", &self.authorization.access_attempt_id)
            .finish_non_exhaustive()
    }
}

impl NewlyAppendedAuthorization {
    /// Returns the exact assigned authorization record reference.
    pub const fn authorization_ref(&self) -> &RecordRef {
        &self.authorization_ref
    }

    /// Returns the kernel-derived access-attempt identity.
    pub const fn access_attempt_id(&self) -> &AccessAttemptId {
        &self.authorization.access_attempt_id
    }

    /// Returns the complete immutable committed authorization.
    #[doc(hidden)]
    pub const fn authorization(&self) -> &ExternalAccessAuthorized {
        &self.authorization
    }

    /// Consumes the store-minted authority for the exact committed prior-run fact Read.
    #[doc(hidden)]
    pub fn invoke_prior_run_fact_scan(
        self,
        request: mfm_facts::FactSelectionRequest,
    ) -> Option<
        std::pin::Pin<
            Box<dyn std::future::Future<Output = PriorRunFactScanCompletion> + Send + 'static>,
        >,
    > {
        let Self {
            fact_scan_permit, ..
        } = self;
        fact_scan_permit.map(|permit| permit.invoke(request))
    }
}

impl StructuredAppendAttempt {
    /// Returns the stable physical append identity.
    pub const fn append_request_id(&self) -> &AppendRequestId {
        &self.append_request_id
    }

    /// Returns the exact candidate digest required for ambiguity resolution.
    pub const fn candidate_digest(&self) -> &ContentDigest {
        &self.candidate_digest
    }

    /// Returns the exact backend disposition.
    pub const fn outcome(&self) -> &BackendAppendOutcome {
        &self.outcome
    }

    pub(super) const fn candidate(&self) -> &CommittedBatch {
        &self.candidate
    }

    /// Returns a positive committed batch proof, when acknowledgement is known.
    pub const fn committed(&self) -> Option<&CommittedBatch> {
        match &self.outcome {
            BackendAppendOutcome::NewlyCommitted(batch)
            | BackendAppendOutcome::ExistingSame(batch) => Some(batch),
            BackendAppendOutcome::StaleHead | BackendAppendOutcome::AcknowledgementUnknown => None,
        }
    }

    /// Consumes a directly acknowledged new authorization append into its
    /// one-use invocation permit.
    pub fn into_newly_appended_authorization(
        self,
    ) -> Option<(NewlyAppendedAuthorization, VerifiedStructuredRun)> {
        if !matches!(self.outcome, BackendAppendOutcome::NewlyCommitted(_)) {
            return None;
        }
        let batch = &self.candidate;
        let [assigned] = batch.records.as_slice() else {
            return None;
        };
        let RunRecord::ExternalAccessAuthorized(authorization) = &assigned.record else {
            return None;
        };
        Some((
            NewlyAppendedAuthorization {
                authorization_ref: assigned.record_ref.clone(),
                authorization: authorization.clone(),
                fact_scan_permit: self.fact_scan_permit,
            },
            self.successor?,
        ))
    }

    /// Consumes an exactly acknowledged append into its incrementally verified
    /// successor. Resolved authorization ambiguity cannot use this method to
    /// mint affine invocation authority.
    #[doc(hidden)]
    pub fn into_committed_successor(self) -> Option<(CommittedBatch, VerifiedStructuredRun)> {
        if !matches!(
            self.outcome,
            BackendAppendOutcome::NewlyCommitted(_) | BackendAppendOutcome::ExistingSame(_)
        ) {
            return None;
        }
        Some((self.candidate, self.successor?))
    }

    pub(super) fn confirm_existing_same(&mut self) {
        self.outcome = BackendAppendOutcome::ExistingSame(self.candidate.clone());
    }
}

impl<B: StructuredHistoryBackend> StructuredRunHistoryWriter<B> {
    /// Verifies and atomically admits one new structured run.
    pub async fn admit_run(
        &self,
        request: StructuredAdmissionRequest,
    ) -> super::Result<StructuredAppendAttempt> {
        let committed = prepare_admission(
            self.backend.identity(),
            request,
            self.program_verifier.as_ref(),
            self.physical_binding_verifier.as_ref(),
        )
        .map_err(candidate_rejected)?;
        self.commit_prepared(committed, None).await
    }

    /// Verifies and atomically commits one exact current callback result.
    pub async fn commit_state_transition(
        &self,
        verified: VerifiedStructuredRun,
        proposal: &StateTransitionProposal,
    ) -> super::Result<StructuredAppendAttempt> {
        let tenant_fact_coordinate = match &proposal.value {
            ProposedTransitionValue::Success { facts, .. } if !facts.as_slice().is_empty() => {
                let current = self
                    .backend
                    .tenant_fact_frontier(&verified.admission().tenant_scope_id)
                    .await?;
                TenantFactCoordinate::FactPublication {
                    frontier: current
                        .next_publication()
                        .map_err(|_| super::StructuredStoreError::InvalidHistory)?,
                }
            }
            ProposedTransitionValue::Success { .. } | ProposedTransitionValue::Failure(_) => {
                TenantFactCoordinate::None
            }
        };
        let PreparedSuccessor { batch, verified } = prepare_state_transition(
            verified,
            self.backend.identity(),
            proposal,
            tenant_fact_coordinate,
        )
        .map_err(candidate_rejected)?;
        self.commit_prepared(batch, Some(verified)).await
    }

    /// Verifies and atomically authorizes one exact current Read or Effect.
    pub async fn authorize_access(
        &self,
        verified: VerifiedStructuredRun,
        proposal: &AccessAuthorizationProposal,
    ) -> super::Result<StructuredAppendAttempt> {
        let tenant_fact_coordinate = if authorization_requires_fact_selection_barrier(&verified)
            .map_err(candidate_rejected)?
        {
            TenantFactCoordinate::FactSelectionBarrier {
                frontier: self
                    .backend
                    .tenant_fact_frontier(&verified.admission().tenant_scope_id)
                    .await?,
            }
        } else {
            TenantFactCoordinate::None
        };
        let PreparedSuccessor { batch, verified } = prepare_authorization(
            verified,
            self.backend.identity(),
            proposal,
            tenant_fact_coordinate,
            self.physical_binding_verifier.as_ref(),
        )
        .map_err(candidate_rejected)?;
        self.commit_prepared(batch, Some(verified)).await
    }

    /// Verifies and atomically records one exact consumed access completion.
    pub async fn commit_observation(
        &self,
        verified: VerifiedStructuredRun,
        proposal: &AccessObservationProposal,
    ) -> super::Result<ObservationCommit> {
        let prepared = prepare_observation(
            verified,
            self.backend.identity(),
            proposal,
            self.physical_binding_verifier.as_ref(),
        )
        .map_err(candidate_rejected)?;
        match prepared {
            PreparedObservation::ExistingSame(verified) => {
                Ok(ObservationCommit::ExistingSame(verified))
            }
            PreparedObservation::Append(prepared) => self
                .commit_prepared(prepared.batch, Some(prepared.verified))
                .await
                .map(|attempt| ObservationCommit::Attempt(Box::new(attempt))),
        }
    }

    /// Validates one invoked completion and resolves its stable logical key
    /// before Runtime freezes pending observation content.
    pub async fn qualify_observation(
        &self,
        verified: &VerifiedStructuredRun,
        authorization_ref: &RecordRef,
        outcome: &ProposedObservationOutcome,
    ) -> super::Result<ObservationQualification> {
        qualify_observation_material(
            verified,
            authorization_ref,
            outcome,
            self.physical_binding_verifier.as_ref(),
        )
        .map_err(candidate_rejected)
    }

    async fn commit_prepared(
        &self,
        committed: CommittedBatch,
        successor: Option<VerifiedStructuredRun>,
    ) -> super::Result<StructuredAppendAttempt> {
        let append_request_id = committed.append_request_id.clone();
        let candidate_digest = committed.candidate_digest.clone();
        let prepared_fact_scan_permit = successor
            .as_ref()
            .map(|verified| {
                fact_scan_permit(
                    prior_run_fact_source(&self.backend),
                    Arc::clone(&self.program_verifier),
                    Arc::clone(&self.physical_binding_verifier),
                    &committed,
                    verified,
                )
            })
            .transpose()?
            .flatten();
        let backend_outcome = self
            .backend
            .append(ValidatedBatch::new(committed.clone()))
            .await?;
        let outcome = match backend_outcome {
            BackendAppendOutcome::NewlyCommitted(returned) => {
                if returned != committed {
                    return Err(super::StructuredStoreError::InvalidHistory);
                }
                BackendAppendOutcome::NewlyCommitted(committed.clone())
            }
            BackendAppendOutcome::ExistingSame(returned) => {
                if returned != committed {
                    return Err(super::StructuredStoreError::InvalidHistory);
                }
                BackendAppendOutcome::ExistingSame(committed.clone())
            }
            BackendAppendOutcome::StaleHead => BackendAppendOutcome::StaleHead,
            BackendAppendOutcome::AcknowledgementUnknown => {
                BackendAppendOutcome::AcknowledgementUnknown
            }
        };
        let fact_scan_permit = if matches!(outcome, BackendAppendOutcome::NewlyCommitted(_)) {
            prepared_fact_scan_permit
        } else {
            None
        };
        Ok(StructuredAppendAttempt {
            append_request_id,
            candidate_digest,
            candidate: committed,
            outcome,
            successor,
            fact_scan_permit,
        })
    }
}

fn candidate_rejected(error: super::StructuredStoreError) -> super::StructuredStoreError {
    match error {
        super::StructuredStoreError::InvalidHistory
        | super::StructuredStoreError::Certification => {
            super::StructuredStoreError::CandidateRejected
        }
        error => error,
    }
}
