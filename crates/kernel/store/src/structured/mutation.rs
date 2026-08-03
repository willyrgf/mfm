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

pub use mfm_runtime::history::{
    AccessAuthorizationProposal, AccessObservationProposal, ProposedCanonicalValue,
    ProposedObservationOutcome, ProposedTransitionValue, StateTransitionProposal,
    StructuredAdmissionMaterial,
};

/// Complete producer-free request to admit one certified run (store-internal).
///
/// Run identity is supplied only after the adapter derives it from the annex
/// preimage; public callers use [`mfm_runtime::history::StructuredAdmissionCommand`].
pub struct StructuredAdmissionRequest {
    pub(super) run_id: mfm_ids::RunId,
    pub(super) tenant_scope_id: mfm_ids::TenantScopeId,
    pub(super) invocation_identity: mfm_ids::InvocationIdentity,
    pub(super) entry_point_operation_id: mfm_ids::StableId,
    pub(super) certified_program: mfm_spec::structured::CertifiedProgramDocument,
    pub(super) material: StructuredAdmissionMaterial,
    pub(super) initial_values: Vec<ProposedCanonicalValue>,
    pub(super) append_request_id: mfm_ids::AppendRequestId,
}

impl StructuredAdmissionRequest {
    /// Binds one exact qualified program document and declaration-ordered roots.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn new(
        run_id: mfm_ids::RunId,
        tenant_scope_id: mfm_ids::TenantScopeId,
        invocation_identity: mfm_ids::InvocationIdentity,
        entry_point_operation_id: mfm_ids::StableId,
        certified_program: mfm_spec::structured::CertifiedProgramDocument,
        material: StructuredAdmissionMaterial,
        initial_values: Vec<ProposedCanonicalValue>,
        append_request_id: mfm_ids::AppendRequestId,
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

    pub(super) const fn run_id(&self) -> &mfm_ids::RunId {
        &self.run_id
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

    /// Converts a store attempt into the Runtime-facing sealed attempt.
    pub(super) fn into_runtime_attempt(self) -> mfm_runtime::history::StructuredAppendAttempt {
        use mfm_journal::structured::RunRecord;
        use mfm_runtime::history::{HistoryAppendOutcome, NewlyAppendedAuthorization};

        let closed = self
            .committed()
            .is_some_and(|batch| {
                batch
                    .records
                    .iter()
                    .any(|record| matches!(&record.record, RunRecord::RunClosed(_)))
            });
        let outcome = match &self.outcome {
            BackendAppendOutcome::NewlyCommitted(batch) => {
                HistoryAppendOutcome::NewlyCommitted(batch.clone())
            }
            BackendAppendOutcome::ExistingSame(batch) => {
                HistoryAppendOutcome::ExistingSame(batch.clone())
            }
            BackendAppendOutcome::StaleHead => HistoryAppendOutcome::StaleHead,
            BackendAppendOutcome::AcknowledgementUnknown => {
                HistoryAppendOutcome::AcknowledgementUnknown
            }
        };
        let authorization = if matches!(self.outcome, BackendAppendOutcome::NewlyCommitted(_)) {
            let batch = &self.candidate;
            if let [assigned] = batch.records.as_slice() {
                if let RunRecord::ExternalAccessAuthorized(authorization) = &assigned.record {
                    let fact_scan = self.fact_scan_permit.map(|permit| {
                        let port: Box<
                            dyn FnOnce(
                                    mfm_facts::FactSelectionRequest,
                                ) -> std::pin::Pin<
                                    Box<
                                        dyn std::future::Future<
                                                Output = mfm_certify::structured::PriorRunFactScanCompletion,
                                            > + Send
                                            + 'static,
                                    >,
                                > + Send,
                        > = Box::new(move |request| {
                            Box::pin(async move {
                                match permit.invoke(request).await {
                                    super::fact_scan::PriorRunFactScanCompletion::Returned(v) => {
                                        mfm_certify::structured::PriorRunFactScanCompletion::Returned(v)
                                    }
                                    super::fact_scan::PriorRunFactScanCompletion::SafeFailure(v) => {
                                        mfm_certify::structured::PriorRunFactScanCompletion::SafeFailure(
                                            v,
                                        )
                                    }
                                    super::fact_scan::PriorRunFactScanCompletion::IntegrityFault(
                                        code,
                                    ) => {
                                        mfm_certify::structured::PriorRunFactScanCompletion::IntegrityFault(
                                            code,
                                        )
                                    }
                                }
                            })
                        });
                        port
                    });
                    Some(NewlyAppendedAuthorization::from_store_mint(
                        assigned.record_ref.clone(),
                        authorization.clone(),
                        fact_scan,
                    ))
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };
        mfm_runtime::history::StructuredAppendAttempt::from_store_attempt(
            self.append_request_id,
            self.candidate_digest,
            self.candidate,
            outcome,
            authorization,
            closed,
        )
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
        let tenant_fact_coordinate = match proposal.value() {
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
