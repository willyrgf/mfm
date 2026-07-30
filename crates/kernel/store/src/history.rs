use std::collections::BTreeMap;
use std::sync::Arc;

use mfm_ids::{AppendRequestId, NodeId};
#[cfg(any(test, feature = "backend-conformance"))]
use mfm_journal::BatchPurpose;
use mfm_journal::JournalHead;

use super::{
    AdmissionMaterial, Admit, AdmitRun, AppendOutcome, AsyncStoreFuture, CommittedJournalLoadGrant,
    Drive, ExistingRunAccessGrant, ExistingRunAppendMaterial, Export, InspectAudit, InspectTrace,
    JournalAppendVerifier, JournalLoadVerifier, PreparedAppendKind, PreparedFrame,
    PreparedJournalAppend, ReadPublic, Replay, RunAccessAuthority, RunJournalBackend, StoreError,
    StoreErrorInspection, StoreIdentity, TransitionTracePageRequest,
    TransitionTraceSourceRequirements, VerifiedAccessAuditPage, VerifiedPublicRunView,
    VerifiedRunView, VerifiedTransitionTracePage,
};

const MAX_ACCESS_AUDIT_PAGE_LIMIT: u16 = 500;

struct HistoryMutationAuthority;

/// Pre-split qualified store assembly.
///
/// The value owns the only backend handle and the store-created mutation authority until
/// [`Self::split`] consumes it. Qualified support must be admitted before that split.
pub struct QualifiedRunStore<B> {
    backend: Arc<B>,
    mutation_authority: HistoryMutationAuthority,
}

impl<B> QualifiedRunStore<B> {
    /// Wraps one deployment-qualified backend in the affine run-history assembly.
    ///
    /// This constructor exists for storage adapters. Concrete backend construction remains
    /// private to the adapter that performed qualification.
    #[doc(hidden)]
    pub fn from_qualified_backend(backend: B) -> Self {
        Self {
            backend: Arc::new(backend),
            mutation_authority: HistoryMutationAuthority,
        }
    }

    /// Consumes the qualified assembly into its sole writer and cloneable reader.
    pub fn split(self) -> (RunHistoryWriter<B>, RunHistoryReader<B>) {
        let Self {
            backend,
            mutation_authority,
        } = self;
        let reader = RunHistoryReader {
            backend: Arc::clone(&backend),
        };
        let writer = RunHistoryWriter {
            backend,
            mutation_authority,
        };
        (writer, reader)
    }

    pub(super) fn backend(&self) -> &B {
        &self.backend
    }
}

impl<B: RunJournalBackend> QualifiedRunStore<B> {
    /// Returns the exact authoritative store lineage.
    pub fn store_identity(&self) -> &StoreIdentity {
        self.backend.store_authority_context().store_identity()
    }
}

/// Sole post-bootstrap run-history mutation capability.
///
/// This value intentionally implements neither `Clone` nor a backend accessor. Runtime consumes
/// it and becomes the only application-visible history mutation path.
pub struct RunHistoryWriter<B> {
    backend: Arc<B>,
    #[allow(dead_code)]
    mutation_authority: HistoryMutationAuthority,
}

impl<B> RunHistoryWriter<B> {
    pub(super) fn backend(&self) -> &B {
        &self.backend
    }
}

impl<B: RunJournalBackend> RunHistoryWriter<B> {
    /// Returns the exact authoritative store lineage.
    pub fn store_identity(&self) -> &StoreIdentity {
        self.backend.store_authority_context().store_identity()
    }

    /// Authors one complete immutable run root from closed producer-free admission material.
    pub fn prepare_admission(
        &self,
        authority: &RunAccessAuthority<Admit>,
        append_request_id: AppendRequestId,
        material: AdmissionMaterial<'_>,
    ) -> super::Result<AdmitRun> {
        let target = self
            .backend
            .store_authority_context()
            .validate_admit(authority)?;
        super::admission_preparation::prepare_admission_material(
            self.backend.store_authority_context(),
            target,
            append_request_id,
            material,
        )
    }

    /// Appends one exact immutable root under admission-purpose authority.
    pub fn append_admission<'a>(
        &'a self,
        authority: &'a RunAccessAuthority<Admit>,
        append: AdmitRun,
    ) -> AsyncStoreFuture<'a, AppendOutcome, B::Error> {
        let checked = (|| {
            let target = self
                .backend
                .store_authority_context()
                .validate_admit(authority)?;
            let root = append.admission().fields()?;
            if append.core.store_identity != *self.store_identity()
                || root.tenant_scope_id != target.tenant_scope_id
                || root.entry_point_operation_id != target.entry_point_operation_id
                || root.invocation_identity != target.invocation_identity
            {
                return Err(StoreError::AdmissionAuthorityMismatch);
            }
            Ok(JournalAppendVerifier::new(PreparedJournalAppend::AdmitRun(
                append,
            )))
        })();
        match checked {
            Ok(verifier) => self.backend.backend_append(verifier),
            Err(error) => Box::pin(async move { Err(error.into()) }),
        }
    }

    /// Loads one exact committed journal under drive-purpose authority.
    pub fn load_for_drive<'a>(
        &'a self,
        authority: &'a RunAccessAuthority<Drive>,
    ) -> AsyncStoreFuture<'a, super::CommittedRunJournal, B::Error> {
        load_committed(self.backend(), authority)
    }

    /// Authors one complete existing-run append from closed producer-free material.
    pub fn prepare_append(
        &self,
        authority: &RunAccessAuthority<Drive>,
        view: &VerifiedRunView,
        append_request_id: AppendRequestId,
        material: ExistingRunAppendMaterial,
    ) -> super::Result<PreparedJournalAppend> {
        let target = self
            .backend
            .store_authority_context()
            .validate_run(authority)?;
        if view.store_identity() != self.store_identity()
            || view.tenant_scope_id() != &target.tenant_scope_id
            || view.run_id() != &target.run_id
        {
            return Err(StoreError::AccessDenied { purpose: "drive" });
        }
        view.prepare_existing_append(
            self.backend.store_authority_context(),
            append_request_id,
            material,
        )
    }

    /// Prepares exact logical observation material for resolution without authorizing append.
    ///
    /// This path accepts an already-observed authorization only so runtime can compare the
    /// retained logical bytes. The resulting candidate remains subject to the normal append
    /// verifier, which rejects a duplicate logical observation.
    pub fn prepare_observation_resolution(
        &self,
        authority: &RunAccessAuthority<Drive>,
        view: &VerifiedRunView,
        append_request_id: AppendRequestId,
        material: super::ObservationMaterial,
    ) -> super::Result<PreparedJournalAppend> {
        let target = self
            .backend
            .store_authority_context()
            .validate_run(authority)?;
        if view.store_identity() != self.store_identity()
            || view.tenant_scope_id() != &target.tenant_scope_id
            || view.run_id() != &target.run_id
        {
            return Err(StoreError::AccessDenied { purpose: "drive" });
        }
        view.prepare_observation_resolution(append_request_id, material)
    }

    /// Reconstructs or authors the exact callback frame for one certified occurrence.
    pub fn prepare_frame<'a>(
        &'a self,
        authority: &'a RunAccessAuthority<Drive>,
        view: &'a VerifiedRunView,
        node_id: &'a NodeId,
    ) -> AsyncStoreFuture<'a, PreparedFrame, B::Error> {
        let checked = (|| {
            let target = self
                .backend
                .store_authority_context()
                .validate_run(authority)?;
            if view.store_identity() != self.store_identity()
                || view.tenant_scope_id() != &target.tenant_scope_id
                || view.run_id() != &target.run_id
            {
                return Err(StoreError::AccessDenied { purpose: "drive" });
            }
            view.prepare_frame_material(self.backend.store_authority_context(), node_id)
        })();
        Box::pin(async move { checked.map_err(Into::into) })
    }

    /// Appends one transition, authorization, or observation under drive-purpose authority.
    pub fn append<'a>(
        &'a self,
        authority: &'a RunAccessAuthority<Drive>,
        append: PreparedJournalAppend,
    ) -> AsyncStoreFuture<'a, AppendOutcome, B::Error> {
        let checked = (|| {
            if append.kind() == PreparedAppendKind::AdmitRun {
                return Err(StoreError::InvalidPreparedAppend {
                    purpose: "drive",
                    message: "run admission requires admission-purpose authority",
                });
            }
            let target = self
                .backend
                .store_authority_context()
                .validate_run(authority)?;
            let core = append.core();
            if core.store_identity != *self.store_identity()
                || core.tenant_scope_id != target.tenant_scope_id
                || core.run_id != target.run_id
            {
                return Err(StoreError::AccessDenied { purpose: "drive" });
            }
            Ok(JournalAppendVerifier::new(append))
        })();
        match checked {
            Ok(verifier) => self.backend.backend_append(verifier),
            Err(error) => Box::pin(async move { Err(error.into()) }),
        }
    }
}

/// Cloneable purpose-specific run-history read capability.
///
/// The reader has no append preparation, append, or support-admission operation.
pub struct RunHistoryReader<B> {
    backend: Arc<B>,
}

impl<B> Clone for RunHistoryReader<B> {
    fn clone(&self) -> Self {
        Self {
            backend: Arc::clone(&self.backend),
        }
    }
}

impl<B> RunHistoryReader<B> {
    pub(super) fn backend(&self) -> &B {
        &self.backend
    }
}

impl<B: RunJournalBackend> RunHistoryReader<B> {
    /// Returns the exact authoritative store lineage.
    pub fn store_identity(&self) -> &StoreIdentity {
        self.backend.store_authority_context().store_identity()
    }

    /// Loads one exact committed journal under replay-purpose authority.
    pub fn load_for_replay<'a>(
        &'a self,
        authority: &'a RunAccessAuthority<Replay>,
    ) -> AsyncStoreFuture<'a, super::CommittedRunJournal, B::Error> {
        load_committed(self.backend(), authority)
    }

    /// Loads one exact committed journal under export-purpose authority.
    pub fn load_for_export<'a>(
        &'a self,
        authority: &'a RunAccessAuthority<Export>,
    ) -> AsyncStoreFuture<'a, super::CommittedRunJournal, B::Error> {
        load_committed(self.backend(), authority)
    }

    /// Reads the sole callback-free public projection through one verified backend snapshot.
    pub fn read_public_run<'a>(
        &'a self,
        authority: &'a RunAccessAuthority<ReadPublic>,
    ) -> AsyncStoreFuture<'a, VerifiedPublicRunView, B::Error> {
        let checked = load_verifier(self.backend(), authority);
        match checked {
            Ok(verifier) => Box::pin(async move {
                let journal = self.backend.backend_load(verifier).await?;
                let view = journal
                    .verify_recorded_history()
                    .map_err(<B::Error as From<StoreError>>::from)?;
                VerifiedPublicRunView::project(&view).map_err(<B::Error as From<StoreError>>::from)
            }),
            Err(error) => Box::pin(async move { Err(error.into()) }),
        }
    }

    /// Reads one bounded access-audit page under exact inspection authority.
    pub fn inspect_access_audit<'a>(
        &'a self,
        authority: &'a RunAccessAuthority<InspectAudit>,
        complete_as_of_journal_head: Option<&'a JournalHead>,
        start: u32,
        limit: u16,
    ) -> AsyncStoreFuture<'a, VerifiedAccessAuditPage, B::Error> {
        let checked = (|| {
            if limit == 0 || limit > MAX_ACCESS_AUDIT_PAGE_LIMIT {
                return Err(StoreError::InvalidAuditPage { field: "limit" });
            }
            load_verifier(self.backend(), authority)
        })();
        match checked {
            Ok(verifier) => Box::pin(async move {
                let journal = self.backend.backend_load(verifier).await?;
                let view = journal
                    .verify_recorded_history()
                    .map_err(<B::Error as From<StoreError>>::from)?;
                let complete_as_of_journal_head = complete_as_of_journal_head
                    .cloned()
                    .unwrap_or_else(|| view.journal_head().clone());
                let entries = view
                    .access_audit_as_of(&complete_as_of_journal_head)
                    .map_err(<B::Error as From<StoreError>>::from)?;
                let start = usize::try_from(start)
                    .map_err(|_| B::Error::from(StoreError::InvalidAuditPage { field: "start" }))?;
                if start > entries.len() {
                    return Err(StoreError::InvalidAuditPage { field: "start" }.into());
                }
                let end = start.saturating_add(usize::from(limit)).min(entries.len());
                let has_more = end < entries.len();
                let next_index = has_more
                    .then(|| {
                        u32::try_from(end).map_err(|_| StoreError::InvalidAuditPage {
                            field: "next_index",
                        })
                    })
                    .transpose()
                    .map_err(<B::Error as From<StoreError>>::from)?;
                let entries = entries.into_iter().skip(start).take(end - start).collect();
                VerifiedAccessAuditPage::new(
                    view.run_id().clone(),
                    complete_as_of_journal_head,
                    entries,
                    has_more,
                    next_index,
                    u32::try_from(start)
                        .map_err(|_| StoreError::InvalidAuditPage { field: "start" })
                        .map_err(<B::Error as From<StoreError>>::from)?,
                )
                .map_err(<B::Error as From<StoreError>>::from)
            }),
            Err(error) => Box::pin(async move { Err(error.into()) }),
        }
    }

    /// Discovers direct source runs named by one exact head-fixed transition page.
    pub fn discover_transition_trace_sources<'a>(
        &'a self,
        authority: &'a RunAccessAuthority<InspectTrace>,
        request: TransitionTracePageRequest,
    ) -> AsyncStoreFuture<'a, TransitionTraceSourceRequirements, B::Error> {
        let checked = load_verifier(self.backend(), authority);
        match checked {
            Ok(verifier) => Box::pin(async move {
                let journal = self.backend.backend_load(verifier).await?;
                let view = journal
                    .verify_recorded_history()
                    .map_err(<B::Error as From<StoreError>>::from)?;
                super::trace::prepare_transition_trace_requirements(
                    self.backend.store_authority_context().clone(),
                    request,
                    view,
                )
                .map_err(<B::Error as From<StoreError>>::from)
            }),
            Err(error) => Box::pin(async move { Err(error.into()) }),
        }
    }

    /// Consumes one exact page token and renders independently authorized source values.
    pub fn inspect_transition_trace<'a>(
        &'a self,
        authority: &'a RunAccessAuthority<InspectTrace>,
        requirements: TransitionTraceSourceRequirements,
        source_authorities: &'a [RunAccessAuthority<InspectTrace>],
    ) -> AsyncStoreFuture<'a, VerifiedTransitionTracePage, B::Error> {
        let checked = (|| {
            requirements.validate_root(self.backend.store_authority_context(), authority)?;
            requirements.validate_source_authorities(
                self.backend.store_authority_context(),
                source_authorities,
            )
        })();
        match checked {
            Ok(source_run_ids) => Box::pin(async move {
                let mut source_views = BTreeMap::new();
                for (source_authority, source_run_id) in
                    source_authorities.iter().zip(source_run_ids)
                {
                    let verifier = JournalLoadVerifier::new(
                        self.store_identity().clone(),
                        source_authority.tenant_scope_id().clone(),
                        source_run_id.clone(),
                    );
                    let journal = match self.backend.backend_load(verifier).await {
                        Ok(journal) => journal,
                        Err(error)
                            if matches!(error.as_store_error(), Some(StoreError::RunNotFound)) =>
                        {
                            continue;
                        }
                        Err(error) => return Err(error),
                    };
                    let source_view = journal
                        .verify_recorded_history()
                        .map_err(<B::Error as From<StoreError>>::from)?;
                    requirements
                        .verify_source_view(&source_run_id, &source_view)
                        .map_err(<B::Error as From<StoreError>>::from)?;
                    source_views.insert(source_run_id, source_view);
                }
                super::trace::render_transition_trace_page(requirements, &source_views)
                    .map_err(<B::Error as From<StoreError>>::from)
            }),
            Err(error) => Box::pin(async move { Err(error.into()) }),
        }
    }
}

/// Trusted backend seam for bounded authoritative-writer readiness.
pub trait RunHistoryReadinessBackend: RunJournalBackend {
    /// Rechecks that the retained backend still names its qualified writable lineage.
    fn backend_check_ready(&self) -> AsyncStoreFuture<'_, (), Self::Error>;
}

impl<B: RunHistoryReadinessBackend> RunHistoryReader<B> {
    /// Rechecks the bounded authoritative-writer readiness proof.
    pub fn check_ready(&self) -> AsyncStoreFuture<'_, (), B::Error> {
        self.backend.backend_check_ready()
    }
}

/// Backend seam used only by atomicity conformance tests to inject exact commit failures.
#[cfg(any(test, feature = "backend-conformance"))]
#[doc(hidden)]
pub trait RunHistoryCommitFailureBackend: RunJournalBackend {
    /// Adapter-owned failure selector.
    type FailurePoint;

    /// Arms one exact run-and-purpose failure.
    fn backend_inject_commit_failure(
        &self,
        run_id: mfm_ids::RunId,
        batch_purpose: BatchPurpose,
        point: Self::FailurePoint,
    ) -> std::result::Result<(), Self::Error>;

    /// Returns whether the exact selector remains armed.
    fn backend_commit_failure_is_armed(&self) -> std::result::Result<bool, Self::Error>;

    /// Consumes the selector only when the exact run and purpose match.
    fn backend_take_commit_failure(
        &self,
        run_id: &mfm_ids::RunId,
        batch_purpose: BatchPurpose,
    ) -> std::result::Result<Option<Self::FailurePoint>, Self::Error>;
}

#[cfg(any(test, feature = "backend-conformance"))]
impl<B: RunHistoryCommitFailureBackend> RunHistoryWriter<B> {
    /// Arms one exact backend conformance failure selector.
    #[doc(hidden)]
    pub fn inject_commit_failure(
        &self,
        run_id: mfm_ids::RunId,
        batch_purpose: BatchPurpose,
        point: B::FailurePoint,
    ) -> std::result::Result<(), B::Error> {
        self.backend()
            .backend_inject_commit_failure(run_id, batch_purpose, point)
    }

    /// Returns whether the backend conformance failure selector remains armed.
    #[doc(hidden)]
    pub fn commit_failure_is_armed(&self) -> std::result::Result<bool, B::Error> {
        self.backend().backend_commit_failure_is_armed()
    }

    /// Exercises exact selector matching without exposing the concrete backend.
    #[doc(hidden)]
    pub fn take_commit_failure(
        &self,
        run_id: &mfm_ids::RunId,
        batch_purpose: BatchPurpose,
    ) -> std::result::Result<Option<B::FailurePoint>, B::Error> {
        self.backend()
            .backend_take_commit_failure(run_id, batch_purpose)
    }
}

/// Backend seam used only to coordinate the admission/run-lock race regression.
#[cfg(any(test, feature = "backend-conformance"))]
#[doc(hidden)]
pub trait RunHistoryAdmissionLockTestBackend: RunJournalBackend {
    /// Adapter-owned lock hook.
    type Hook;

    /// Arms a pause immediately before the exact admission run lock.
    fn backend_inject_before_admission_run_lock(
        &self,
        append_request_id: AppendRequestId,
    ) -> std::result::Result<Self::Hook, Self::Error>;
}

#[cfg(any(test, feature = "backend-conformance"))]
impl<B: RunHistoryAdmissionLockTestBackend> RunHistoryWriter<B> {
    /// Arms the backend's admission/run-lock conformance hook.
    #[doc(hidden)]
    pub fn inject_before_admission_run_lock(
        &self,
        append_request_id: AppendRequestId,
    ) -> std::result::Result<B::Hook, B::Error> {
        self.backend()
            .backend_inject_before_admission_run_lock(append_request_id)
    }
}

fn load_verifier<B, G>(
    backend: &B,
    authority: &RunAccessAuthority<G>,
) -> super::Result<JournalLoadVerifier>
where
    B: RunJournalBackend,
    G: ExistingRunAccessGrant,
{
    backend
        .store_authority_context()
        .validate_run(authority)
        .map(|target| {
            JournalLoadVerifier::new(
                backend.store_authority_context().store_identity().clone(),
                target.tenant_scope_id.clone(),
                target.run_id.clone(),
            )
        })
}

fn load_committed<'a, B, G>(
    backend: &'a B,
    authority: &'a RunAccessAuthority<G>,
) -> AsyncStoreFuture<'a, super::CommittedRunJournal, B::Error>
where
    B: RunJournalBackend,
    G: CommittedJournalLoadGrant,
{
    match load_verifier(backend, authority) {
        Ok(verifier) => backend.backend_load(verifier),
        Err(error) => Box::pin(async move { Err(error.into()) }),
    }
}
