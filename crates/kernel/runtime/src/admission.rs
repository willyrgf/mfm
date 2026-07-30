use mfm_ids::{AppendRequestId, EntryPointId, InvocationIdentity, RunId, StableId};
use mfm_spec::CertifiedAdmissionArtifacts;
use mfm_store::{
    AdmissionMaterial, AppendOutcome, AppendRejection, CommittedAppend, NewlyAppended,
    ProposedAdmissionInput, RunAccessAuthority, RunJournalBackend, StoreError,
    VerifiedAdmissionSources, VerifiedConfiguredValue,
};

use crate::{Result, Runtime, RuntimeError};

/// App-authored, authority-bound material for one runtime-owned admission.
///
/// The plan intentionally contains no support graph, store identity, predecessor, prepared
/// append, writer, or backend handle. Runtime validates it against its current registry and
/// injects only the registry's admitted support.
pub struct AuthorizedAdmissionPlan {
    authority: RunAccessAuthority<mfm_store::Admit>,
    append_request_id: AppendRequestId,
    entry_point_id: EntryPointId,
    entry_point_operation_id: StableId,
    invocation_identity: InvocationIdentity,
    artifacts: CertifiedAdmissionArtifacts,
    input: ProposedAdmissionInput,
    configured: VerifiedConfiguredValue,
    sources: VerifiedAdmissionSources,
}

impl AuthorizedAdmissionPlan {
    /// Constructs one owned plan after app authentication, configuration, certification, and
    /// admission-source verification have completed.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        authority: RunAccessAuthority<mfm_store::Admit>,
        append_request_id: AppendRequestId,
        entry_point_id: EntryPointId,
        entry_point_operation_id: StableId,
        invocation_identity: InvocationIdentity,
        artifacts: CertifiedAdmissionArtifacts,
        input: ProposedAdmissionInput,
        configured: VerifiedConfiguredValue,
        sources: VerifiedAdmissionSources,
    ) -> Self {
        Self {
            authority,
            append_request_id,
            entry_point_id,
            entry_point_operation_id,
            invocation_identity,
            artifacts,
            input,
            configured,
            sources,
        }
    }
}

/// Runtime-resolved admission disposition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdmissionDisposition {
    /// Runtime directly observed a fresh immutable admission commit.
    NewlyAdmitted,
    /// The exact admission append was already committed.
    Attached,
    /// Commit visibility is ambiguous and no head is asserted.
    OutcomeUnknown,
}

/// Runtime-owned result of one admission attempt.
pub struct AdmissionOutcome {
    run_id: RunId,
    committed: Option<CommittedAppend>,
    disposition: AdmissionDisposition,
}

impl AdmissionOutcome {
    /// Returns the deterministic admitted run identity.
    pub const fn run_id(&self) -> &RunId {
        &self.run_id
    }

    /// Returns the exact committed head when commit visibility is known.
    pub const fn committed(&self) -> Option<&CommittedAppend> {
        self.committed.as_ref()
    }

    /// Returns the authority-safe append disposition.
    pub const fn disposition(&self) -> AdmissionDisposition {
        self.disposition
    }
}

impl<B: RunJournalBackend> Runtime<B> {
    /// Validates and appends one admission through the Runtime's sole history writer.
    pub async fn admit(&self, plan: AuthorizedAdmissionPlan) -> Result<AdmissionOutcome> {
        let AuthorizedAdmissionPlan {
            authority,
            append_request_id,
            entry_point_id,
            entry_point_operation_id,
            invocation_identity,
            artifacts,
            input,
            configured,
            sources,
        } = plan;

        let registration = self
            .program_registry
            .entry_point(&entry_point_id)
            .ok_or(RuntimeError::CatalogSelection)?;
        if authority.entry_point_id() != &entry_point_id
            || authority.entry_point_operation_id() != &entry_point_operation_id
            || authority.invocation_identity() != &invocation_identity
            || registration.entry_point().entry_point_operation_id() != &entry_point_operation_id
            || artifacts.entry_point().entry_point_id() != &entry_point_id
            || artifacts.entry_point().entry_point_operation_id() != &entry_point_operation_id
        {
            return Err(RuntimeError::CatalogSelection);
        }
        self.program_registry
            .validate_current_admission_artifacts(&entry_point_id, &artifacts)
            .map_err(|_| RuntimeError::CatalogSelection)?;

        let prepared = self.writer.prepare_admission(
            &authority,
            append_request_id,
            AdmissionMaterial::new(
                artifacts,
                input,
                &configured,
                self.program_registry.admitted_support(),
                &sources,
            ),
        )?;
        let run_id = prepared.admission().fields()?.run_id;
        let outcome = self
            .writer
            .append_admission(&authority, prepared)
            .await
            .map_err(|error| map_admission_store_error(&error))?;
        match outcome {
            AppendOutcome::NewlyAppended(NewlyAppended::RunAdmitted(admitted)) => {
                Ok(AdmissionOutcome {
                    run_id,
                    committed: Some(admitted.committed().clone()),
                    disposition: AdmissionDisposition::NewlyAdmitted,
                })
            }
            AppendOutcome::AlreadyCommitted(committed) => Ok(AdmissionOutcome {
                run_id,
                committed: Some(committed),
                disposition: AdmissionDisposition::Attached,
            }),
            AppendOutcome::OutcomeUnknown => Ok(AdmissionOutcome {
                run_id,
                committed: None,
                disposition: AdmissionDisposition::OutcomeUnknown,
            }),
            AppendOutcome::Rejected(rejection) => Err(RuntimeError::Store(match rejection {
                AppendRejection::AdmissionConflict | AppendRejection::AppendRequestConflict => {
                    StoreError::AdmissionConflict
                }
                AppendRejection::StaleHead { expected, actual } => StoreError::HeadMismatch {
                    expected: Box::new(expected),
                    actual,
                },
                AppendRejection::RunClosed => StoreError::RunClosed,
            })),
            AppendOutcome::NewlyAppended(
                NewlyAppended::Transition(_)
                | NewlyAppended::Authorization(_)
                | NewlyAppended::Observation(_),
            ) => Err(RuntimeError::InvalidCallbackResult),
        }
    }
}

fn map_admission_store_error(error: &impl mfm_store::StoreErrorInspection) -> RuntimeError {
    match error.as_store_error() {
        Some(StoreError::AppendRequestConflict) => {
            RuntimeError::Store(StoreError::AdmissionConflict)
        }
        Some(error) => RuntimeError::Store(error.clone()),
        None => RuntimeError::StoreBackendUnavailable,
    }
}
