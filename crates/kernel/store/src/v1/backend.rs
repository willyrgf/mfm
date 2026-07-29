use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;

use mfm_ids::{AppendRequestId, NodeId, RunId};
use mfm_journal::v1::JournalHead;

use super::{
    journal::AccessAuditProjection, AdmissionMaterial, Admit, AdmitRun, AppendOutcome,
    CommittedJournalLoadGrant, Drive, ExistingRunAppendMaterial, InspectAudit, InspectTrace,
    JournalAppendVerifier, JournalLoadVerifier, PreparedAppendKind, PreparedFrame,
    PreparedJournalAppend, ReadPublic, RunAccessAuthority, StoreAuthorityContext, StoreError,
    StoreErrorInspection, StoreIdentity, TransitionTracePageRequest,
    TransitionTraceSourceRequirements, VerifiedAccessAuditEntry, VerifiedPublicRunView,
    VerifiedRunView, VerifiedTransitionTracePage,
};

const MAX_ACCESS_AUDIT_PAGE_LIMIT: u16 = 500;

/// Boxed asynchronous store operation.
pub type AsyncStoreFuture<'a, T, E> =
    Pin<Box<dyn Future<Output = std::result::Result<T, E>> + Send + 'a>>;

/// One store-verified, head-fixed access-audit page.
pub struct VerifiedAccessAuditPage {
    run_id: RunId,
    complete_as_of_journal_head: JournalHead,
    entries: Vec<AccessAuditProjection>,
    has_more: bool,
    next_index: Option<u32>,
}

impl VerifiedAccessAuditPage {
    fn new(
        run_id: RunId,
        complete_as_of_journal_head: JournalHead,
        entries: Vec<AccessAuditProjection>,
        has_more: bool,
        next_index: Option<u32>,
        start: u32,
    ) -> Result<Self, StoreError> {
        let expected_next = if has_more {
            let length =
                u32::try_from(entries.len()).map_err(|_| StoreError::InvalidAuditPage {
                    field: "next_index",
                })?;
            Some(
                start
                    .checked_add(length)
                    .ok_or(StoreError::InvalidAuditPage {
                        field: "next_index",
                    })?,
            )
        } else {
            None
        };
        if next_index != expected_next {
            return Err(StoreError::InvalidAuditPage {
                field: "next_index",
            });
        }
        Ok(Self {
            run_id,
            complete_as_of_journal_head,
            entries,
            has_more,
            next_index,
        })
    }

    /// Returns the inspected run identity.
    pub const fn run_id(&self) -> &RunId {
        &self.run_id
    }

    /// Returns the exact physical head fixed by the first page.
    pub const fn complete_as_of_journal_head(&self) -> &JournalHead {
        &self.complete_as_of_journal_head
    }

    /// Returns safe entries in authorization order.
    pub fn entries(&self) -> impl ExactSizeIterator<Item = VerifiedAccessAuditEntry<'_>> + '_ {
        self.entries.iter().map(VerifiedAccessAuditEntry::new)
    }

    /// Returns whether another entry exists after this page.
    pub const fn has_more(&self) -> bool {
        self.has_more
    }

    /// Returns the next zero-based authorization index when another page exists.
    pub const fn next_index(&self) -> Option<u32> {
        self.next_index
    }
}

/// Trusted durable-backend seam behind the sealed journal contract.
///
/// Implementations receive only store-created verifiers. They must perform compare-and-swap,
/// idempotency, object admission, tenant-coordinate assignment, and immutable-row publication in
/// one transaction. There is no raw append method.
pub trait RunJournalBackend: Send + Sync {
    /// Backend-specific error preserving typed store-error inspection.
    type Error: std::error::Error + StoreErrorInspection + From<StoreError> + Send + Sync + 'static;

    /// Returns the private context paired with this exact store's authority issuer.
    fn store_authority_context(&self) -> &StoreAuthorityContext;

    /// Atomically validates, assigns, and publishes one store-created append verifier.
    fn backend_append<'a>(
        &'a self,
        verifier: JournalAppendVerifier,
    ) -> AsyncStoreFuture<'a, AppendOutcome, Self::Error>;

    /// Loads immutable rows and completes one store-created exact-run verifier.
    fn backend_load<'a>(
        &'a self,
        verifier: JournalLoadVerifier,
    ) -> AsyncStoreFuture<'a, super::CommittedRunJournal, Self::Error>;
}
/// Purpose-authorized recoverability-v2 run-journal surface.
///
/// This trait is implemented for every trusted [`RunJournalBackend`]. Callers cannot bypass its
/// authority target checks or construct load/append verifiers.
pub trait RunJournalStore: Send + Sync {
    /// Backend-specific error preserving typed store-error inspection.
    type Error: std::error::Error + StoreErrorInspection + From<StoreError> + Send + Sync + 'static;

    /// Returns the exact authoritative store lineage.
    fn store_identity(&self) -> &StoreIdentity;

    /// Appends one exact immutable root under admission-purpose authority.
    fn append_admission<'a>(
        &'a self,
        authority: &'a RunAccessAuthority<Admit>,
        append: AdmitRun,
    ) -> AsyncStoreFuture<'a, AppendOutcome, Self::Error>;

    /// Authors one complete immutable run root from closed producer-free admission material.
    fn prepare_admission(
        &self,
        authority: &RunAccessAuthority<Admit>,
        append_request_id: mfm_ids::AppendRequestId,
        material: AdmissionMaterial<'_>,
    ) -> super::Result<AdmitRun>;

    /// Appends one transition, authorization, or observation under drive-purpose authority.
    fn append<'a>(
        &'a self,
        authority: &'a RunAccessAuthority<Drive>,
        append: PreparedJournalAppend,
    ) -> AsyncStoreFuture<'a, AppendOutcome, Self::Error>;

    /// Authors one complete existing-run append from closed producer-free material.
    ///
    /// The exact store instance, tenant, run, physical head, and semantic fold are rechecked
    /// before any raw record, reference, object graph, or closure is derived.
    fn prepare_append(
        &self,
        authority: &RunAccessAuthority<Drive>,
        view: &VerifiedRunView,
        append_request_id: AppendRequestId,
        material: ExistingRunAppendMaterial,
    ) -> super::Result<PreparedJournalAppend>;

    /// Reconstructs or authors the exact callback frame for one certified occurrence.
    ///
    /// Unstarted occurrences are assembled only when ready. Awaiting-effect and terminal
    /// occurrences reconstruct the immutable frame retained by their original authorization or
    /// transition, including after-close audit tails.
    fn prepare_frame<'a>(
        &'a self,
        authority: &'a RunAccessAuthority<Drive>,
        view: &'a VerifiedRunView,
        node_id: &'a NodeId,
    ) -> AsyncStoreFuture<'a, PreparedFrame, Self::Error>;

    /// Loads one exact committed journal under a full-journal purpose authority.
    ///
    /// Public and inspection grants cannot use this raw-history seam:
    ///
    /// ```compile_fail
    /// use mfm_store::{ReadPublic, RunAccessAuthority, RunJournalStore};
    ///
    /// async fn raw_public_load_is_forbidden<S: RunJournalStore>(
    ///     store: &S,
    ///     authority: &RunAccessAuthority<ReadPublic>,
    /// ) {
    ///     let _ = store.load_committed_journal(authority).await;
    /// }
    /// ```
    ///
    /// ```compile_fail
    /// use mfm_store::{InspectTrace, RunAccessAuthority, RunJournalStore};
    ///
    /// async fn raw_trace_load_is_forbidden<S: RunJournalStore>(
    ///     store: &S,
    ///     authority: &RunAccessAuthority<InspectTrace>,
    /// ) {
    ///     let _ = store.load_committed_journal(authority).await;
    /// }
    /// ```
    ///
    /// ```compile_fail
    /// use mfm_store::{InspectAudit, RunAccessAuthority, RunJournalStore};
    ///
    /// async fn raw_audit_load_is_forbidden<S: RunJournalStore>(
    ///     store: &S,
    ///     authority: &RunAccessAuthority<InspectAudit>,
    /// ) {
    ///     let _ = store.load_committed_journal(authority).await;
    /// }
    /// ```
    fn load_committed_journal<'a, G: CommittedJournalLoadGrant>(
        &'a self,
        authority: &'a RunAccessAuthority<G>,
    ) -> AsyncStoreFuture<'a, super::CommittedRunJournal, Self::Error>;

    /// Reads the sole callback-free public projection through one verified backend snapshot.
    fn read_public_run<'a>(
        &'a self,
        authority: &'a RunAccessAuthority<ReadPublic>,
    ) -> AsyncStoreFuture<'a, VerifiedPublicRunView, Self::Error>;

    /// Reads one bounded access-audit page under exact inspection authority.
    ///
    /// Omitting `complete_as_of_journal_head` atomically fixes the page to the loaded current
    /// physical head. Continuation calls must supply that exact head, which must remain a verified
    /// ancestor of the current journal.
    fn inspect_access_audit<'a>(
        &'a self,
        authority: &'a RunAccessAuthority<InspectAudit>,
        complete_as_of_journal_head: Option<&'a JournalHead>,
        start: u32,
        limit: u16,
    ) -> AsyncStoreFuture<'a, VerifiedAccessAuditPage, Self::Error>;

    /// Discovers direct source runs named by one exact head-fixed transition page.
    ///
    /// The returned source ids grant no source access. The caller must make one fresh independent
    /// `InspectTrace` decision for each source before consuming the opaque requirements token.
    fn discover_transition_trace_sources<'a>(
        &'a self,
        authority: &'a RunAccessAuthority<InspectTrace>,
        request: TransitionTracePageRequest,
    ) -> AsyncStoreFuture<'a, TransitionTraceSourceRequirements, Self::Error>;

    /// Consumes one exact page token and renders only independently authorized source values.
    ///
    /// Source authorities must be a canonical sorted, duplicate-free subset of the page's direct
    /// requirements. Missing authorities and authorized-but-absent source runs render the same
    /// digest-only redaction. Corrupt authorized source history fails verification.
    fn inspect_transition_trace<'a>(
        &'a self,
        authority: &'a RunAccessAuthority<InspectTrace>,
        requirements: TransitionTraceSourceRequirements,
        source_authorities: &'a [RunAccessAuthority<InspectTrace>],
    ) -> AsyncStoreFuture<'a, VerifiedTransitionTracePage, Self::Error>;
}

impl<B: RunJournalBackend> RunJournalStore for B {
    type Error = B::Error;

    fn store_identity(&self) -> &StoreIdentity {
        self.store_authority_context().store_identity()
    }

    fn append_admission<'a>(
        &'a self,
        authority: &'a RunAccessAuthority<Admit>,
        append: AdmitRun,
    ) -> AsyncStoreFuture<'a, AppendOutcome, Self::Error> {
        let checked = (|| {
            let target = self.store_authority_context().validate_admit(authority)?;
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
            Ok(verifier) => self.backend_append(verifier),
            Err(error) => Box::pin(async move { Err(error.into()) }),
        }
    }

    fn prepare_admission(
        &self,
        authority: &RunAccessAuthority<Admit>,
        append_request_id: mfm_ids::AppendRequestId,
        material: AdmissionMaterial<'_>,
    ) -> super::Result<AdmitRun> {
        let target = self.store_authority_context().validate_admit(authority)?;
        super::admission_preparation::prepare_admission_material(
            self.store_authority_context(),
            target,
            append_request_id,
            material,
        )
    }

    fn append<'a>(
        &'a self,
        authority: &'a RunAccessAuthority<Drive>,
        append: PreparedJournalAppend,
    ) -> AsyncStoreFuture<'a, AppendOutcome, Self::Error> {
        let checked = (|| {
            if append.kind() == PreparedAppendKind::AdmitRun {
                return Err(StoreError::InvalidPreparedAppend {
                    purpose: "drive",
                    message: "run admission requires admission-purpose authority",
                });
            }
            let target = self.store_authority_context().validate_run(authority)?;
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
            Ok(verifier) => self.backend_append(verifier),
            Err(error) => Box::pin(async move { Err(error.into()) }),
        }
    }

    fn prepare_append(
        &self,
        authority: &RunAccessAuthority<Drive>,
        view: &VerifiedRunView,
        append_request_id: AppendRequestId,
        material: ExistingRunAppendMaterial,
    ) -> super::Result<PreparedJournalAppend> {
        let target = self.store_authority_context().validate_run(authority)?;
        if view.store_identity() != self.store_identity()
            || view.tenant_scope_id() != &target.tenant_scope_id
            || view.run_id() != &target.run_id
        {
            return Err(StoreError::AccessDenied { purpose: "drive" });
        }
        view.prepare_existing_append(self.store_authority_context(), append_request_id, material)
    }

    fn prepare_frame<'a>(
        &'a self,
        authority: &'a RunAccessAuthority<Drive>,
        view: &'a VerifiedRunView,
        node_id: &'a NodeId,
    ) -> AsyncStoreFuture<'a, PreparedFrame, Self::Error> {
        let checked = (|| {
            let target = self.store_authority_context().validate_run(authority)?;
            if view.store_identity() != self.store_identity()
                || view.tenant_scope_id() != &target.tenant_scope_id
                || view.run_id() != &target.run_id
            {
                return Err(StoreError::AccessDenied { purpose: "drive" });
            }
            view.prepare_frame_material(self.store_authority_context(), node_id)
        })();
        Box::pin(async move { checked.map_err(Into::into) })
    }

    fn load_committed_journal<'a, G: CommittedJournalLoadGrant>(
        &'a self,
        authority: &'a RunAccessAuthority<G>,
    ) -> AsyncStoreFuture<'a, super::CommittedRunJournal, Self::Error> {
        let checked = self
            .store_authority_context()
            .validate_run(authority)
            .map(|target| {
                JournalLoadVerifier::new(
                    self.store_identity().clone(),
                    target.tenant_scope_id.clone(),
                    target.run_id.clone(),
                )
            });
        match checked {
            Ok(verifier) => self.backend_load(verifier),
            Err(error) => Box::pin(async move { Err(error.into()) }),
        }
    }

    fn read_public_run<'a>(
        &'a self,
        authority: &'a RunAccessAuthority<ReadPublic>,
    ) -> AsyncStoreFuture<'a, VerifiedPublicRunView, Self::Error> {
        let checked = self
            .store_authority_context()
            .validate_run(authority)
            .map(|target| {
                JournalLoadVerifier::new(
                    self.store_identity().clone(),
                    target.tenant_scope_id.clone(),
                    target.run_id.clone(),
                )
            });
        match checked {
            Ok(verifier) => Box::pin(async move {
                let journal = self.backend_load(verifier).await?;
                let view = journal
                    .verify_recorded_history()
                    .map_err(<B::Error as From<StoreError>>::from)?;
                VerifiedPublicRunView::project(&view).map_err(<B::Error as From<StoreError>>::from)
            }),
            Err(error) => Box::pin(async move { Err(error.into()) }),
        }
    }

    fn inspect_access_audit<'a>(
        &'a self,
        authority: &'a RunAccessAuthority<InspectAudit>,
        complete_as_of_journal_head: Option<&'a JournalHead>,
        start: u32,
        limit: u16,
    ) -> AsyncStoreFuture<'a, VerifiedAccessAuditPage, Self::Error> {
        let checked = (|| {
            if limit == 0 || limit > MAX_ACCESS_AUDIT_PAGE_LIMIT {
                return Err(StoreError::InvalidAuditPage { field: "limit" });
            }
            let target = self.store_authority_context().validate_run(authority)?;
            Ok(JournalLoadVerifier::new(
                self.store_identity().clone(),
                target.tenant_scope_id.clone(),
                target.run_id.clone(),
            ))
        })();
        match checked {
            Ok(verifier) => Box::pin(async move {
                let journal = self.backend_load(verifier).await?;
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

    fn discover_transition_trace_sources<'a>(
        &'a self,
        authority: &'a RunAccessAuthority<InspectTrace>,
        request: TransitionTracePageRequest,
    ) -> AsyncStoreFuture<'a, TransitionTraceSourceRequirements, Self::Error> {
        let checked = self
            .store_authority_context()
            .validate_run(authority)
            .map(|target| {
                JournalLoadVerifier::new(
                    self.store_identity().clone(),
                    target.tenant_scope_id.clone(),
                    target.run_id.clone(),
                )
            });
        match checked {
            Ok(verifier) => Box::pin(async move {
                let journal = self.backend_load(verifier).await?;
                let view = journal
                    .verify_recorded_history()
                    .map_err(<B::Error as From<StoreError>>::from)?;
                super::trace::prepare_transition_trace_requirements(
                    self.store_authority_context().clone(),
                    request,
                    view,
                )
                .map_err(<B::Error as From<StoreError>>::from)
            }),
            Err(error) => Box::pin(async move { Err(error.into()) }),
        }
    }

    fn inspect_transition_trace<'a>(
        &'a self,
        authority: &'a RunAccessAuthority<InspectTrace>,
        requirements: TransitionTraceSourceRequirements,
        source_authorities: &'a [RunAccessAuthority<InspectTrace>],
    ) -> AsyncStoreFuture<'a, VerifiedTransitionTracePage, Self::Error> {
        let checked = (|| {
            requirements.validate_root(self.store_authority_context(), authority)?;
            requirements
                .validate_source_authorities(self.store_authority_context(), source_authorities)
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
                    let journal = match self.backend_load(verifier).await {
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

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::task::{Context, Poll, Waker};

    use mfm_ids::{RunId, StoreEpoch, StoreScopeId, TenantScopeId};

    use super::{AsyncStoreFuture, RunJournalStore};
    use crate::v1::{AsyncInMemoryRunStore, StoreError, StoreIdentity};

    fn ready<T>(mut future: AsyncStoreFuture<'_, T, StoreError>) -> Result<T, StoreError> {
        let mut context = Context::from_waker(Waker::noop());
        match Future::poll(future.as_mut(), &mut context) {
            Poll::Ready(result) => result,
            Poll::Pending => panic!("immediate authority validation unexpectedly yielded"),
        }
    }

    #[test]
    fn access_audit_page_rejects_out_of_bounds_limits_before_loading() {
        let (store, issuer) = AsyncInMemoryRunStore::new(store_identity('a'));
        let authority = issuer.authorize_inspect_audit(tenant('b'), run_id('c'));

        for limit in [0, 501] {
            assert_eq!(
                ready(store.inspect_access_audit(&authority, None, 0, limit)).err(),
                Some(StoreError::InvalidAuditPage { field: "limit" })
            );
        }
    }

    #[test]
    fn access_audit_page_rejects_a_foreign_store_instance_seal() {
        let identity = store_identity('a');
        let (_, issuer) = AsyncInMemoryRunStore::new(identity.clone());
        let (store, _) = AsyncInMemoryRunStore::new(identity);
        let authority = issuer.authorize_inspect_audit(tenant('b'), run_id('c'));

        assert_eq!(
            ready(store.inspect_access_audit(&authority, None, 0, 1)).err(),
            Some(StoreError::AccessDenied {
                purpose: "inspect_audit"
            })
        );
    }

    fn store_identity(fill: char) -> StoreIdentity {
        StoreIdentity::new(
            StoreScopeId::new(format!(
                "{}{}",
                StoreScopeId::PREFIX,
                fill.to_string().repeat(32)
            ))
            .expect("store scope"),
            StoreEpoch::new(1),
        )
    }

    fn tenant(fill: char) -> TenantScopeId {
        TenantScopeId::new(format!(
            "{}{}",
            TenantScopeId::PREFIX,
            fill.to_string().repeat(32)
        ))
        .expect("tenant scope")
    }

    fn run_id(fill: char) -> RunId {
        RunId::parse(format!("run:sha256-jcs-v1:{}", fill.to_string().repeat(64))).expect("run id")
    }
}
