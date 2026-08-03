//! Private production adapter implementing [`RuntimeHistoryPort`].

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use mfm_canonical::{CanonicalValue, RecoverabilityContract};
use mfm_certify::structured::AdmissionVerificationRegistry;
use mfm_ids::{ContentRef, InvocationIdentity, RunId, StableId, StoreScopeId, TenantScopeId};
use mfm_journal::structured::RecordRef;
use mfm_runtime::history::{
    AccessAuthorizationProposal, AccessObservationProposal, HistoryError, HistoryFuture,
    ObservationCommit, ObservationQualification, ProposedObservationOutcome, RuntimeHistoryPort,
    StateTransitionProposal, StructuredAdmissionCommand, StructuredAppendAttempt,
    StructuredStoreIdentity, VerifiedRunView,
};
use mfm_spec::structured::CertifiedProgramRoot;
use mfm_spec::CanonicalJsonValue;

use super::backend::{StructuredHistoryBackend, StructuredRunHistoryWriter};
use super::fold::{
    ProgramVerifier, StructuredStoreError, VerifiedProgramData, VerifiedStructuredRun,
};
use super::mutation::StructuredAdmissionRequest as StoreAdmissionRequest;

/// Concrete registry-backed program verifier for live assembly and offline trust snapshots.
pub struct RegistryProgramVerifier {
    registry: AdmissionVerificationRegistry,
    verified_programs: Mutex<BTreeMap<ContentRef, Arc<VerifiedProgramData>>>,
}

impl RegistryProgramVerifier {
    /// Wraps one concrete admission-verification registry without live process authority.
    pub fn new(registry: AdmissionVerificationRegistry) -> Self {
        Self {
            registry,
            verified_programs: Mutex::new(BTreeMap::new()),
        }
    }
}

pub(super) fn build_program_verifier(
    registry: AdmissionVerificationRegistry,
) -> Arc<dyn ProgramVerifier> {
    Arc::new(RegistryProgramVerifier::new(registry))
}

impl ProgramVerifier for RegistryProgramVerifier {
    fn verify(
        &self,
        entry_point_id: &StableId,
        root: &CertifiedProgramRoot,
        authored: &CanonicalJsonValue,
    ) -> std::result::Result<Arc<VerifiedProgramData>, StructuredStoreError> {
        let program_ref = root
            .content_ref()
            .map_err(|_| StructuredStoreError::Certification)?;
        if let Some(cached) = self
            .verified_programs
            .lock()
            .map_err(|_| StructuredStoreError::Certification)?
            .get(&program_ref)
            .cloned()
        {
            let authored_matches = cached
                .document()
                .component_closure
                .iter()
                .find(|object| {
                    object.content_ref == cached.document().root.components.authored_program_ref
                })
                .is_some_and(|object| &object.value == authored);
            return if &cached.document().root == root
                && cached.expanded().operation_id == *entry_point_id
                && authored_matches
            {
                Ok(cached)
            } else {
                Err(StructuredStoreError::Certification)
            };
        }
        let certified = self
            .registry
            .verify_root(entry_point_id, root, authored)
            .map_err(|_| StructuredStoreError::Certification)?;
        let (document, expanded, value_schemas) = certified.into_verification_parts();
        // Reject substitution: expanded operation must match entry and document root.
        if expanded.operation_id != *entry_point_id || document.root != *root {
            return Err(StructuredStoreError::Certification);
        }
        let verified = Arc::new(VerifiedProgramData::new(document, expanded, value_schemas));
        let mut cache = self
            .verified_programs
            .lock()
            .map_err(|_| StructuredStoreError::Certification)?;
        match cache.get(&program_ref) {
            Some(existing) if existing.document().root == verified.document().root => {
                Ok(Arc::clone(existing))
            }
            Some(_) => Err(StructuredStoreError::Certification),
            None => {
                cache.insert(program_ref, Arc::clone(&verified));
                Ok(verified)
            }
        }
    }
}

impl VerifiedRunView for VerifiedStructuredRun {
    fn run_id(&self) -> &RunId {
        VerifiedStructuredRun::run_id(self)
    }

    fn admission(&self) -> &mfm_journal::structured::RunAdmitted {
        VerifiedStructuredRun::admission(self)
    }

    fn journal_head(&self) -> &mfm_journal::structured::JournalHead {
        VerifiedStructuredRun::journal_head(self)
    }

    fn semantic_head(&self) -> &mfm_journal::structured::SemanticHead {
        VerifiedStructuredRun::semantic_head(self)
    }

    fn frontier(&self) -> &mfm_runtime::history::StructuredFrontier {
        VerifiedStructuredRun::frontier(self)
    }

    fn object(&self, content_ref: &ContentRef) -> Option<&mfm_journal::structured::HistoryObject> {
        VerifiedStructuredRun::object(self, content_ref)
    }

    fn authorization(
        &self,
        access_attempt_id: &mfm_ids::AccessAttemptId,
    ) -> Option<(
        &RecordRef,
        &mfm_journal::structured::ExternalAccessAuthorized,
    )> {
        VerifiedStructuredRun::authorization(self, access_attempt_id)
    }

    fn observation(
        &self,
        access_attempt_id: &mfm_ids::AccessAttemptId,
    ) -> Option<(&RecordRef, &mfm_journal::structured::ExternalAccessObserved)> {
        VerifiedStructuredRun::observation(self, access_attempt_id)
    }
}

/// Production history port owning the sole fold and backend.
///
/// Constructible only by store assembly. The type may appear in assembled
/// `Runtime` signatures but cannot be built or attached outside store-owned
/// openers.
#[doc(hidden)]
pub struct StoreHistoryAdapter<B: StructuredHistoryBackend> {
    writer: StructuredRunHistoryWriter<B>,
}

impl<B: StructuredHistoryBackend> StoreHistoryAdapter<B> {
    pub(super) fn from_writer(writer: StructuredRunHistoryWriter<B>) -> Self {
        Self { writer }
    }
}

fn derive_run_id(
    store_scope_id: &StoreScopeId,
    tenant_scope_id: &TenantScopeId,
    entry_point_operation_id: &StableId,
    invocation_identity: &InvocationIdentity,
) -> Result<RunId, HistoryError> {
    let preimage = CanonicalValue::object([
        (
            "store_scope_id",
            CanonicalValue::String(store_scope_id.as_str().to_owned()),
        ),
        (
            "tenant_scope_id",
            CanonicalValue::String(tenant_scope_id.as_str().to_owned()),
        ),
        (
            "entry_point_operation_id",
            CanonicalValue::String(entry_point_operation_id.as_str().to_owned()),
        ),
        (
            "invocation_identity",
            CanonicalValue::String(invocation_identity.as_str().to_owned()),
        ),
    ])
    .map_err(|_| HistoryError::InvalidHistory)?;
    let contract = RecoverabilityContract::embedded().map_err(|_| HistoryError::InvalidHistory)?;
    let validated = contract
        .encode("mfm.run-id-preimage.v1", &preimage)
        .map_err(|_| HistoryError::InvalidHistory)?;
    contract
        .derive_run_id(&validated)
        .map_err(|_| HistoryError::InvalidHistory)
}

fn to_store_transition(proposal: &StateTransitionProposal) -> StateTransitionProposal {
    proposal.clone()
}

fn to_store_authorization(proposal: &AccessAuthorizationProposal) -> AccessAuthorizationProposal {
    proposal.clone()
}

fn to_store_observation_outcome(
    outcome: &ProposedObservationOutcome,
) -> ProposedObservationOutcome {
    outcome.clone()
}

fn to_store_observation(proposal: &AccessObservationProposal) -> AccessObservationProposal {
    proposal.clone()
}

impl<B: StructuredHistoryBackend> RuntimeHistoryPort for StoreHistoryAdapter<B> {
    type VerifiedRun = VerifiedStructuredRun;

    fn store_identity(&self) -> &StructuredStoreIdentity {
        self.writer.store_identity()
    }

    fn admit_run<'a>(
        &'a self,
        command: StructuredAdmissionCommand,
    ) -> HistoryFuture<'a, (RunId, StructuredAppendAttempt)> {
        Box::pin(async move {
            let (
                tenant_scope_id,
                invocation_identity,
                entry_point_operation_id,
                certified_program,
                material,
                initial_values,
                append_request_id,
            ) = command.into_parts();
            let run_id = derive_run_id(
                &self.writer.store_identity().store_scope_id,
                &tenant_scope_id,
                &entry_point_operation_id,
                &invocation_identity,
            )?;
            let store_material = material;
            let request = StoreAdmissionRequest::new(
                run_id.clone(),
                tenant_scope_id,
                invocation_identity,
                entry_point_operation_id,
                certified_program,
                store_material,
                initial_values,
                append_request_id,
            );
            let attempt = self.writer.admit_run(request).await?;
            Ok((run_id, attempt.into_runtime_attempt()))
        })
    }

    fn load_verified<'a>(&'a self, run_id: &'a RunId) -> HistoryFuture<'a, Self::VerifiedRun> {
        Box::pin(async move { self.writer.load_verified(run_id).await })
    }

    fn commit_state_transition<'a>(
        &'a self,
        verified: Self::VerifiedRun,
        proposal: &'a StateTransitionProposal,
    ) -> HistoryFuture<'a, StructuredAppendAttempt> {
        Box::pin(async move {
            let store_proposal = to_store_transition(proposal);
            self.writer
                .commit_state_transition(verified, &store_proposal)
                .await
                .map(|attempt| attempt.into_runtime_attempt())
        })
    }

    fn authorize_access<'a>(
        &'a self,
        verified: Self::VerifiedRun,
        proposal: &'a AccessAuthorizationProposal,
    ) -> HistoryFuture<'a, StructuredAppendAttempt> {
        Box::pin(async move {
            let store_proposal = to_store_authorization(proposal);
            self.writer
                .authorize_access(verified, &store_proposal)
                .await
                .map(|attempt| attempt.into_runtime_attempt())
        })
    }

    fn resolve_attempt<'a>(
        &'a self,
        attempt: &'a mut StructuredAppendAttempt,
    ) -> HistoryFuture<'a, bool> {
        Box::pin(async move {
            let run_id = attempt
                .candidate()
                .records
                .first()
                .ok_or(HistoryError::InvalidHistory)?
                .record_ref
                .run_id
                .clone();
            let append_request_id = attempt.append_request_id().clone();
            let candidate_digest = attempt.candidate_digest().clone();
            let resolved = self
                .writer
                .resolve_append(&run_id, &append_request_id, &candidate_digest)
                .await?;
            match resolved {
                Some(batch) if batch == *attempt.candidate() => {
                    attempt.confirm_existing_same();
                    Ok(true)
                }
                Some(_) => Err(HistoryError::InvalidHistory),
                None => Ok(false),
            }
        })
    }

    fn qualify_observation<'a>(
        &'a self,
        verified: &'a Self::VerifiedRun,
        authorization_ref: &'a RecordRef,
        outcome: &'a ProposedObservationOutcome,
    ) -> HistoryFuture<'a, ObservationQualification> {
        Box::pin(async move {
            let store_outcome = to_store_observation_outcome(outcome);
            self.writer
                .qualify_observation(verified, authorization_ref, &store_outcome)
                .await
        })
    }

    fn commit_observation<'a>(
        &'a self,
        verified: Self::VerifiedRun,
        proposal: &'a AccessObservationProposal,
    ) -> HistoryFuture<'a, ObservationCommit> {
        Box::pin(async move {
            let store_proposal = to_store_observation(proposal);
            match self
                .writer
                .commit_observation(verified, &store_proposal)
                .await?
            {
                super::mutation::ObservationCommit::ExistingSame(_) => {
                    Ok(ObservationCommit::ExistingSame)
                }
                super::mutation::ObservationCommit::Attempt(attempt) => Ok(
                    ObservationCommit::Attempt(Box::new(attempt.into_runtime_attempt())),
                ),
            }
        })
    }
}
