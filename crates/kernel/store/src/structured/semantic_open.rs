//! Sole asynchronous transition from physical usability to semantic authority.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use mfm_journal::structured::{TenantFactCoordinate, TenantFactFrontier};

use super::adapter::StoreHistoryAdapter;
use super::backend::{
    prior_run_fact_source, RawHistoryLoadLimit, RawRunHistory, StructuredBackendFuture,
    StructuredHistoryBackend, StructuredRunHistoryReader, StructuredRunHistoryWriter,
    StructuredSemanticOpenAudit, StructuredStoreIdentity, TenantFactPublication,
    SEMANTIC_OPEN_KEY_PAGE_ITEMS, SEMANTIC_OPEN_ROUTE_PAGE_ITEMS,
};
use super::compiler::{compile_preview, ComparedReduction};
use super::obligations::{discharge, FinalizedReduction, ObligationDischargeScope};
use super::purpose::{
    AuditRunReader, EffectEntryAttentionReader, ExportRunReader, PublicRunReader, ReplayRunReader,
    TraceRunReader,
};
use super::qualification::{
    qualify_recorded_history, PhysicalObligationChecker, ProgramVerificationRegistry,
    QualifiedHistory, StructuredStoreError,
};
use super::reducer::{reduce_event, QualifiedEvent, ReducedRunState};
use super::validated_append::RunCurrentProjection;
use super::VerifiedStructuredRun;

struct StoreAssemblyConsumer;
impl mfm_authority_seal::RuntimeAssemblyConsumerSeal for StoreAssemblyConsumer {}

/// Store-owned Runtime capability with its verification representation hidden.
pub struct StructuredRuntime<B: StructuredHistoryBackend> {
    inner: mfm_runtime::structured::Runtime<StoreHistoryAdapter<B>>,
}

// These signatures deliberately expose the Runtime error contract without another future type.
#[allow(clippy::type_complexity)]
impl<B: StructuredHistoryBackend> StructuredRuntime<B> {
    /// Verifies and atomically admits one exact structured run.
    pub fn admit_run<'a>(
        &'a self,
        command: mfm_runtime::history::StructuredAdmissionCommand,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = mfm_runtime::structured::Result<(
                        mfm_ids::RunId,
                        mfm_runtime::history::StructuredAppendAttempt,
                    )>,
                > + Send
                + 'a,
        >,
    > {
        Box::pin(self.inner.admit_run(command))
    }

    /// Interprets and performs at most one reducer-derived action.
    pub fn drive_once<'a>(
        &'a self,
        run_id: &'a mfm_ids::RunId,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = mfm_runtime::structured::Result<mfm_runtime::structured::DriveOutcome>,
                > + Send
                + 'a,
        >,
    > {
        Box::pin(self.inner.drive_once(run_id))
    }
}

/// Complete authority bundle released only after semantic qualification.
pub struct OpenedStructuredStore<B: StructuredHistoryBackend> {
    /// Sole Runtime mutation authority.
    pub runtime: StructuredRuntime<B>,
    /// Public-read purpose reader.
    pub public_reader: PublicRunReader<B>,
    /// Trace purpose reader.
    pub trace_reader: TraceRunReader<B>,
    /// Audit purpose reader.
    pub audit_reader: AuditRunReader<B>,
    /// Tenant Effect-attention inventory.
    pub effect_entry_attention_reader: EffectEntryAttentionReader<B>,
    /// Replay purpose reader.
    pub replay_reader: ReplayRunReader<B>,
    /// Export purpose reader.
    pub export_reader: ExportRunReader<B>,
}

/// Complete callback-free evidence closure supplied to offline verification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OfflineRunClosure {
    root: RawRunHistory,
    source_prefixes: Vec<RawRunHistory>,
    fact_publications: Vec<TenantFactPublication>,
}

impl OfflineRunClosure {
    /// Binds one root history to every exact producer prefix and dense publication it can read.
    pub fn new(
        root: RawRunHistory,
        source_prefixes: Vec<RawRunHistory>,
        fact_publications: Vec<TenantFactPublication>,
    ) -> Self {
        Self {
            root,
            source_prefixes,
            fact_publications,
        }
    }
}

/// Qualifies every visible run before releasing Runtime or reader capability.
pub async fn qualify_and_open_structured_store<B: StructuredHistoryBackend>(
    backend: B,
    registry: mfm_certify::structured::CertifiedProgramRegistry,
    physical: Arc<dyn PhysicalObligationChecker>,
) -> super::Result<OpenedStructuredStore<B>> {
    let (admission, processes, token) = registry.into_runtime_parts(StoreAssemblyConsumer);
    let programs = Arc::new(ProgramVerificationRegistry::new(admission));
    let backend = Arc::new(backend);
    let identity = backend.identity().clone();
    let audit = backend.begin_semantic_open_audit().await?;
    qualify_store_audit(audit, &identity, &programs, &physical).await?;
    backend.validate_authority().await?;
    let writer = StructuredRunHistoryWriter {
        backend: Arc::clone(&backend),
        programs: Arc::clone(&programs),
        physical: Arc::clone(&physical),
    };
    let reader = StructuredRunHistoryReader {
        backend,
        programs,
        physical,
    };
    let history = StoreHistoryAdapter::from_writer(writer);
    let processes = mfm_runtime::structured::RuntimeProcessRegistry::from_certified(
        processes,
        &token,
        StoreAssemblyConsumer,
    )
    .map_err(|_| StructuredStoreError::InvalidHistory)?;
    let runtime = StructuredRuntime {
        inner: mfm_runtime::structured::Runtime::from_assembled(
            history,
            processes,
            token,
            StoreAssemblyConsumer,
        )
        .map_err(|_| StructuredStoreError::InvalidHistory)?,
    };
    Ok(OpenedStructuredStore {
        runtime,
        public_reader: PublicRunReader::new(reader.clone()),
        trace_reader: TraceRunReader::new(reader.clone()),
        audit_reader: AuditRunReader::new(reader.clone()),
        effect_entry_attention_reader: EffectEntryAttentionReader::new(reader.clone()),
        replay_reader: ReplayRunReader::new(reader.clone()),
        export_reader: ExportRunReader::new(reader),
    })
}

pub(super) async fn load_and_compare<B: StructuredHistoryBackend>(
    backend: &Arc<B>,
    run_id: &mfm_ids::RunId,
    programs: &Arc<ProgramVerificationRegistry>,
    physical: &Arc<dyn PhysicalObligationChecker>,
) -> super::Result<VerifiedStructuredRun> {
    let snapshot = backend
        .load_snapshot(run_id, RawHistoryLoadLimit::run())
        .await?;
    let raw = snapshot.history.ok_or(StructuredStoreError::RunNotFound)?;
    let verified = super::fact_scan::qualify_and_reduce_for_scan(
        prior_run_fact_source(backend),
        raw,
        Arc::clone(programs),
        Arc::clone(physical),
    )
    .await?;
    if snapshot.current_projection.as_ref() != Some(&verified.current_projection()) {
        return Err(StructuredStoreError::InvalidHistory);
    }
    Ok(verified)
}

pub(super) fn verify_qualified(
    raw: RawRunHistory,
    programs: &ProgramVerificationRegistry,
    physical: &dyn PhysicalObligationChecker,
) -> super::Result<VerifiedStructuredRun> {
    let history = qualify_recorded_history(raw, programs)?;
    replay(history, physical).map(|replayed| replayed.verified)
}

pub(super) fn verify_qualified_with_fact_publications(
    raw: RawRunHistory,
    programs: &ProgramVerificationRegistry,
    physical: &dyn PhysicalObligationChecker,
) -> super::Result<(VerifiedStructuredRun, Vec<TenantFactPublication>)> {
    let history = qualify_recorded_history(raw, programs)?;
    replay(history, physical).map(|replayed| (replayed.verified, replayed.fact_publications))
}

struct ReplayedRun {
    verified: VerifiedStructuredRun,
    fact_publications: Vec<TenantFactPublication>,
}

fn replay(
    history: Arc<QualifiedHistory>,
    physical: &dyn PhysicalObligationChecker,
) -> super::Result<ReplayedRun> {
    let mut previous = ReducedRunState::empty(&history.context);
    let mut fact_publications = Vec::new();
    let count = history.batches.len();
    for (index, batch) in history.batches.iter().enumerate() {
        let finalized = replay_step(&history, &previous, batch, physical)?;
        if let super::validated_append::TenantFactProjectionPlan::Publish { publication, .. } =
            finalized.tenant_fact_plan()
        {
            fact_publications.push(publication.clone());
        }
        if index + 1 == count {
            return Ok(ReplayedRun {
                verified: VerifiedStructuredRun::from_finalized(history, finalized),
                fact_publications,
            });
        }
        previous = *finalized.into_reduced();
    }
    Err(StructuredStoreError::InvalidHistory)
}

pub(super) fn replay_step(
    history: &QualifiedHistory,
    previous: &ReducedRunState,
    batch: &super::qualification::QualifiedBatch,
    physical: &dyn PhysicalObligationChecker,
) -> super::Result<FinalizedReduction> {
    let preview = reduce_event(
        &history.context,
        previous,
        &QualifiedEvent::Recorded(&batch.event),
    )?;
    let retained = history
        .object_first_seen_sequence
        .iter()
        .filter(|(_, sequence)| **sequence < batch.committed.head.run_sequence)
        .map(|(reference, _)| reference.clone())
        .collect::<BTreeSet<_>>();
    let compiled = compile_preview(
        &history.store_identity,
        &history.context,
        projection_before(history, previous),
        retained_frontier(history, batch)?,
        &retained,
        &batch.committed.append_request_id,
        &preview,
    )?;
    let compared =
        ComparedReduction::compare(None, preview, &batch.assertions, &batch.committed, compiled)?;
    discharge(
        compared,
        &history.context,
        physical,
        ObligationDischargeScope::RetainedOnly,
    )
}

#[cfg(any(test, feature = "test-support"))]
pub(super) fn verify_incremental_equivalence(
    raw: RawRunHistory,
    programs: &ProgramVerificationRegistry,
    physical: &dyn PhysicalObligationChecker,
) -> super::Result<()> {
    let history = qualify_recorded_history(raw.clone(), programs)?;
    let mut incremental = ReducedRunState::empty(&history.context);
    for (index, batch) in history.batches.iter().enumerate() {
        let finalized = replay_step(&history, &incremental, batch, physical)?;
        let full = verify_qualified(
            RawRunHistory {
                run_id: raw.run_id.clone(),
                batches: raw.batches[..=index].to_vec(),
            },
            programs,
            physical,
        )?;
        let (reduced, compiled) = finalized.into_parts();
        let (committed, run_projection, _tenant_fact_plan, _fact_spec) = compiled.into_parts();
        if reduced.as_ref() != full.reduced.as_ref()
            || run_projection.successor() != &full.current_projection()
            || committed != raw.batches[index]
        {
            return Err(StructuredStoreError::InvalidHistory);
        }
        incremental = *reduced;
    }
    Ok(())
}

#[derive(Clone)]
struct SemanticAuditHandle {
    audit: Arc<Mutex<Option<Box<dyn StructuredSemanticOpenAudit>>>>,
}

impl SemanticAuditHandle {
    fn new(audit: Box<dyn StructuredSemanticOpenAudit>) -> Self {
        Self {
            audit: Arc::new(Mutex::new(Some(audit))),
        }
    }

    fn take(&self) -> super::Result<Box<dyn StructuredSemanticOpenAudit>> {
        self.audit
            .lock()
            .map_err(|_| StructuredStoreError::BackendUnavailable)?
            .take()
            .ok_or(StructuredStoreError::BackendUnavailable)
    }

    fn replace(&self, audit: Box<dyn StructuredSemanticOpenAudit>) -> super::Result<()> {
        let mut slot = self
            .audit
            .lock()
            .map_err(|_| StructuredStoreError::BackendUnavailable)?;
        if slot.replace(audit).is_some() {
            return Err(StructuredStoreError::BackendUnavailable);
        }
        Ok(())
    }

    async fn scan_run_ids(
        &self,
        after: Option<&mfm_ids::RunId>,
    ) -> super::Result<Vec<mfm_ids::RunId>> {
        let mut audit = self.take()?;
        let result = audit.scan_run_ids(after).await;
        self.replace(audit)?;
        result
    }

    async fn load_run_projection(
        &self,
        run_id: &mfm_ids::RunId,
    ) -> super::Result<Option<RunCurrentProjection>> {
        let mut audit = self.take()?;
        let result = audit.load_run_projection(run_id).await;
        self.replace(audit)?;
        result
    }

    async fn load_run_history(
        &self,
        run_id: &mfm_ids::RunId,
        through: Option<&mfm_journal::structured::JournalHead>,
        limit: RawHistoryLoadLimit,
    ) -> super::Result<Option<RawRunHistory>> {
        let mut audit = self.take()?;
        let result = audit.load_run_history_bounded(run_id, through, limit).await;
        self.replace(audit)?;
        result
    }

    async fn scan_run_publications(
        &self,
        run_id: &mfm_ids::RunId,
        after_run_sequence: Option<u64>,
    ) -> super::Result<Vec<TenantFactPublication>> {
        let mut audit = self.take()?;
        let result = audit
            .scan_run_publications(run_id, after_run_sequence)
            .await;
        self.replace(audit)?;
        result
    }

    async fn scan_fact_tenants(
        &self,
        after: Option<&mfm_ids::TenantScopeId>,
    ) -> super::Result<Vec<mfm_ids::TenantScopeId>> {
        let mut audit = self.take()?;
        let result = audit.scan_fact_tenants(after).await;
        self.replace(audit)?;
        result
    }

    async fn load_fact_head(
        &self,
        tenant: &mfm_ids::TenantScopeId,
    ) -> super::Result<Option<TenantFactFrontier>> {
        let mut audit = self.take()?;
        let result = audit.load_fact_head(tenant).await;
        self.replace(audit)?;
        result
    }

    async fn scan_tenant_publications(
        &self,
        tenant: &mfm_ids::TenantScopeId,
        after_fact_order: Option<u64>,
    ) -> super::Result<Vec<TenantFactPublication>> {
        let mut audit = self.take()?;
        let result = audit
            .scan_tenant_publications(tenant, after_fact_order)
            .await;
        self.replace(audit)?;
        result
    }

    async fn finish(self) -> super::Result<()> {
        let audit = self.take()?;
        audit.finish().await
    }
}

impl super::fact_scan::PriorRunFactSource for SemanticAuditHandle {
    fn scan_publications<'a>(
        &'a self,
        tenant_scope_id: &'a mfm_ids::TenantScopeId,
        first_order: u64,
        through_order: u64,
        maximum_items: u32,
    ) -> StructuredBackendFuture<'a, Vec<TenantFactPublication>> {
        Box::pin(async move {
            if first_order == 0
                || maximum_items == 0
                || maximum_items > SEMANTIC_OPEN_ROUTE_PAGE_ITEMS
            {
                return Err(StructuredStoreError::InvalidHistory);
            }
            if first_order > through_order {
                return Ok(Vec::new());
            }
            let page = self
                .scan_tenant_publications(tenant_scope_id, first_order.checked_sub(1))
                .await?;
            validate_route_page_len(&page)?;
            Ok(page
                .into_iter()
                .take_while(|publication| publication.frontier.fact_order <= through_order)
                .take(maximum_items as usize)
                .collect())
        })
    }

    fn load_producer_prefix<'a>(
        &'a self,
        run_id: &'a mfm_ids::RunId,
        through: &'a mfm_journal::structured::JournalHead,
        limit: RawHistoryLoadLimit,
    ) -> StructuredBackendFuture<'a, Option<RawRunHistory>> {
        Box::pin(async move { self.load_run_history(run_id, Some(through), limit).await })
    }
}

struct OfflineClosureFactSource {
    histories: BTreeMap<mfm_ids::RunId, RawRunHistory>,
    publications: BTreeMap<mfm_ids::TenantScopeId, Vec<TenantFactPublication>>,
    scanned_publications: Mutex<BTreeSet<(mfm_ids::TenantScopeId, u64)>>,
}

impl OfflineClosureFactSource {
    fn from_offline_closure(closure: &OfflineRunClosure) -> super::Result<Self> {
        let mut histories = BTreeMap::new();
        for history in std::iter::once(&closure.root).chain(&closure.source_prefixes) {
            if histories
                .insert(history.run_id.clone(), history.clone())
                .is_some()
            {
                return Err(StructuredStoreError::InvalidHistory);
            }
        }
        let mut exact_publications = BTreeMap::new();
        for publication in &closure.fact_publications {
            let key = (
                publication.frontier.tenant_scope_id.clone(),
                publication.frontier.fact_order,
            );
            if exact_publications
                .insert(key, publication.clone())
                .is_some()
            {
                return Err(StructuredStoreError::InvalidHistory);
            }
        }
        let mut publications = BTreeMap::<_, Vec<_>>::new();
        for ((tenant, _), publication) in exact_publications {
            publications.entry(tenant).or_default().push(publication);
        }
        Ok(Self {
            histories,
            publications,
            scanned_publications: Mutex::new(BTreeSet::new()),
        })
    }

    fn scanned_publication_coordinates(
        &self,
    ) -> super::Result<BTreeSet<(mfm_ids::TenantScopeId, u64)>> {
        self.scanned_publications
            .lock()
            .map(|coordinates| coordinates.clone())
            .map_err(|_| StructuredStoreError::InvalidHistory)
    }
}

impl super::fact_scan::PriorRunFactSource for OfflineClosureFactSource {
    fn scan_publications<'a>(
        &'a self,
        tenant_scope_id: &'a mfm_ids::TenantScopeId,
        first_order: u64,
        through_order: u64,
        maximum_items: u32,
    ) -> StructuredBackendFuture<'a, Vec<TenantFactPublication>> {
        Box::pin(async move {
            if first_order == 0 || maximum_items == 0 {
                return Err(StructuredStoreError::InvalidHistory);
            }
            let publications = self
                .publications
                .get(tenant_scope_id)
                .into_iter()
                .flatten()
                .filter(|publication| {
                    publication.frontier.fact_order >= first_order
                        && publication.frontier.fact_order <= through_order
                })
                .take(maximum_items as usize)
                .cloned()
                .collect::<Vec<_>>();
            let mut scanned = self
                .scanned_publications
                .lock()
                .map_err(|_| StructuredStoreError::InvalidHistory)?;
            scanned.extend(publications.iter().map(|publication| {
                (
                    publication.frontier.tenant_scope_id.clone(),
                    publication.frontier.fact_order,
                )
            }));
            Ok(publications)
        })
    }

    fn load_producer_prefix<'a>(
        &'a self,
        run_id: &'a mfm_ids::RunId,
        through: &'a mfm_journal::structured::JournalHead,
        limit: RawHistoryLoadLimit,
    ) -> StructuredBackendFuture<'a, Option<RawRunHistory>> {
        Box::pin(async move {
            let Some(history) = self.histories.get(run_id) else {
                return Ok(None);
            };
            let Some(position) = history
                .batches
                .iter()
                .position(|batch| &batch.head == through)
            else {
                return Ok(None);
            };
            let batches = &history.batches[..=position];
            limit.validate_batches(batches)?;
            Ok(Some(RawRunHistory {
                run_id: run_id.clone(),
                batches: batches.to_vec(),
            }))
        })
    }
}

async fn qualify_store_audit(
    raw_audit: Box<dyn StructuredSemanticOpenAudit>,
    identity: &StructuredStoreIdentity,
    programs: &Arc<ProgramVerificationRegistry>,
    physical: &Arc<dyn PhysicalObligationChecker>,
) -> super::Result<()> {
    let audit = SemanticAuditHandle::new(raw_audit);
    qualify_run_pages(&audit, programs, physical).await?;
    qualify_tenant_pages(&audit, identity).await?;
    audit.finish().await
}

async fn qualify_run_pages(
    audit: &SemanticAuditHandle,
    programs: &Arc<ProgramVerificationRegistry>,
    physical: &Arc<dyn PhysicalObligationChecker>,
) -> super::Result<()> {
    let mut after = None;
    loop {
        let page = audit.scan_run_ids(after.as_ref()).await?;
        validate_key_page(&page, after.as_ref(), SEMANTIC_OPEN_KEY_PAGE_ITEMS)?;
        if page.is_empty() {
            return Ok(());
        }
        for run_id in &page {
            let current = audit
                .load_run_projection(run_id)
                .await?
                .ok_or(StructuredStoreError::InvalidHistory)?;
            let raw = audit
                .load_run_history(run_id, None, RawHistoryLoadLimit::run())
                .await?
                .ok_or(StructuredStoreError::InvalidHistory)?;
            if raw.run_id != *run_id {
                return Err(StructuredStoreError::InvalidHistory);
            }
            let source: Arc<dyn super::fact_scan::PriorRunFactSource> = Arc::new(audit.clone());
            let (verified, expected_publications) =
                super::fact_scan::qualify_and_reduce_for_scan_with_publications_and_memo(
                    source,
                    raw,
                    Arc::clone(programs),
                    Arc::clone(physical),
                    Arc::new(super::fact_scan::PrefixVerificationMemo::default()),
                )
                .await?;
            if current != verified.current_projection() {
                return Err(StructuredStoreError::InvalidHistory);
            }
            compare_run_publications(audit, run_id, &expected_publications).await?;
        }
        after = page.last().cloned();
    }
}

async fn compare_run_publications(
    audit: &SemanticAuditHandle,
    run_id: &mfm_ids::RunId,
    expected: &[TenantFactPublication],
) -> super::Result<()> {
    let mut after_sequence = None;
    let mut expected_index = 0_usize;
    loop {
        let page = audit.scan_run_publications(run_id, after_sequence).await?;
        validate_route_page_len(&page)?;
        if page.is_empty() {
            return if expected_index == expected.len() {
                Ok(())
            } else {
                Err(StructuredStoreError::InvalidHistory)
            };
        }
        for publication in &page {
            let sequence = publication.transition_ref.run_sequence;
            if publication.transition_ref.run_id != *run_id
                || after_sequence.is_some_and(|after| sequence <= after)
                || expected.get(expected_index) != Some(publication)
            {
                return Err(StructuredStoreError::InvalidHistory);
            }
            expected_index = expected_index
                .checked_add(1)
                .ok_or(StructuredStoreError::InvalidHistory)?;
            after_sequence = Some(sequence);
        }
    }
}

async fn qualify_tenant_pages(
    audit: &SemanticAuditHandle,
    identity: &StructuredStoreIdentity,
) -> super::Result<()> {
    let mut after = None;
    loop {
        let page = audit.scan_fact_tenants(after.as_ref()).await?;
        validate_key_page(&page, after.as_ref(), SEMANTIC_OPEN_KEY_PAGE_ITEMS)?;
        if page.is_empty() {
            return Ok(());
        }
        for tenant in &page {
            qualify_tenant_routes(audit, identity, tenant).await?;
        }
        after = page.last().cloned();
    }
}

async fn qualify_tenant_routes(
    audit: &SemanticAuditHandle,
    identity: &StructuredStoreIdentity,
    tenant: &mfm_ids::TenantScopeId,
) -> super::Result<()> {
    let head = audit.load_fact_head(tenant).await?;
    if head.as_ref().is_some_and(|frontier| {
        frontier.store_scope_id != identity.store_scope_id
            || frontier.store_epoch != identity.store_epoch
            || frontier.tenant_scope_id != *tenant
            || frontier.fact_order == 0
    }) {
        return Err(StructuredStoreError::InvalidHistory);
    }
    let mut after_order = None;
    let mut expected_order = 1_u64;
    let mut last_frontier = None;
    loop {
        let page = audit.scan_tenant_publications(tenant, after_order).await?;
        validate_route_page_len(&page)?;
        if page.is_empty() {
            if head != last_frontier {
                return Err(StructuredStoreError::InvalidHistory);
            }
            return Ok(());
        }
        for publication in &page {
            let frontier = &publication.frontier;
            if frontier.store_scope_id != identity.store_scope_id
                || frontier.store_epoch != identity.store_epoch
                || frontier.tenant_scope_id != *tenant
                || frontier.fact_order != expected_order
                || after_order.is_some_and(|after| frontier.fact_order <= after)
            {
                return Err(StructuredStoreError::InvalidHistory);
            }
            expected_order = expected_order
                .checked_add(1)
                .ok_or(StructuredStoreError::InvalidHistory)?;
            after_order = Some(frontier.fact_order);
            last_frontier = Some(frontier.clone());
        }
    }
}

fn validate_key_page<T: Ord>(page: &[T], after: Option<&T>, maximum: u32) -> super::Result<()> {
    if page.len() > maximum as usize
        || page.windows(2).any(|pair| pair[0] >= pair[1])
        || page
            .first()
            .is_some_and(|first| after.is_some_and(|cursor| first <= cursor))
    {
        return Err(StructuredStoreError::InvalidHistory);
    }
    Ok(())
}

fn validate_route_page_len(page: &[TenantFactPublication]) -> super::Result<()> {
    if page.len() > SEMANTIC_OPEN_ROUTE_PAGE_ITEMS as usize {
        return Err(StructuredStoreError::InvalidHistory);
    }
    Ok(())
}

fn projection_before(
    history: &QualifiedHistory,
    reduced: &ReducedRunState,
) -> Option<RunCurrentProjection> {
    reduced
        .journal_head()
        .ok()
        .map(|head| RunCurrentProjection {
            run_id: history.run_id.clone(),
            tenant_scope_id: history.context.admission.tenant_scope_id.clone(),
            journal_head: head.clone(),
            has_effect_entry_attention: reduced.effect_entry_attention().is_some(),
        })
}

fn retained_frontier(
    history: &QualifiedHistory,
    batch: &super::qualification::QualifiedBatch,
) -> super::Result<Option<TenantFactFrontier>> {
    match &batch.committed.tenant_fact_coordinate {
        TenantFactCoordinate::None => Ok(None),
        TenantFactCoordinate::FactSelectionBarrier { frontier } => Ok(Some(frontier.clone())),
        TenantFactCoordinate::FactPublication { frontier } => Ok(Some(TenantFactFrontier::new(
            history.store_identity.store_scope_id.clone(),
            history.store_identity.store_epoch,
            history.context.admission.tenant_scope_id.clone(),
            frontier
                .fact_order
                .checked_sub(1)
                .ok_or(StructuredStoreError::InvalidHistory)?,
        ))),
    }
}

/// Verifies retained fact responses by executing the live selector over the supplied closure.
pub async fn verify_offline_run_closure(
    closure: OfflineRunClosure,
    programs: Arc<ProgramVerificationRegistry>,
    physical: Arc<dyn PhysicalObligationChecker>,
) -> super::Result<super::purpose::OfflineVerifiedRun> {
    if closure.source_prefixes.len() > super::purpose::MAX_PORTABLE_SOURCE_RUNS
        || closure.fact_publications.len() > super::purpose::MAX_PORTABLE_FACT_ROUTES
    {
        return Err(StructuredStoreError::InvalidHistory);
    }
    let mut expected_sources = BTreeMap::new();
    for history in &closure.source_prefixes {
        let head = history
            .batches
            .last()
            .map(|batch| batch.head.clone())
            .ok_or(StructuredStoreError::InvalidHistory)?;
        if history.run_id == closure.root.run_id
            || expected_sources
                .insert(history.run_id.clone(), head)
                .is_some()
        {
            return Err(StructuredStoreError::InvalidHistory);
        }
    }
    let expected_publications = closure
        .fact_publications
        .iter()
        .map(|publication| {
            (
                publication.frontier.tenant_scope_id.clone(),
                publication.frontier.fact_order,
            )
        })
        .collect::<BTreeSet<_>>();
    if expected_publications.len() != closure.fact_publications.len() {
        return Err(StructuredStoreError::InvalidHistory);
    }
    let snapshot = Arc::new(OfflineClosureFactSource::from_offline_closure(&closure)?);
    let source: Arc<dyn super::fact_scan::PriorRunFactSource> = snapshot.clone();
    let prefix_memo = Arc::new(super::fact_scan::PrefixVerificationMemo::default());
    let (verified, _) = super::fact_scan::qualify_and_reduce_for_scan_with_publications_and_memo(
        source,
        closure.root,
        programs,
        physical,
        Arc::clone(&prefix_memo),
    )
    .await?;
    let verified_prefixes = prefix_memo.verified_prefixes()?;
    let verified_source_ids = verified_prefixes
        .iter()
        .map(|source| source.run_id().clone())
        .collect::<BTreeSet<_>>();
    if verified_source_ids != expected_sources.keys().cloned().collect()
        || snapshot.scanned_publication_coordinates()? != expected_publications
    {
        return Err(StructuredStoreError::InvalidHistory);
    }
    let mut heads_by_sequence = BTreeMap::new();
    for source in &verified_prefixes {
        let key = (source.run_id().clone(), source.journal_head().run_sequence);
        if heads_by_sequence
            .insert(key, source.journal_head().clone())
            .is_some_and(|existing| existing != *source.journal_head())
        {
            return Err(StructuredStoreError::InvalidHistory);
        }
    }
    let mut verified_sources = Vec::with_capacity(expected_sources.len());
    for (run_id, expected_head) in expected_sources {
        let source = verified_prefixes
            .iter()
            .find(|source| source.run_id() == &run_id && source.journal_head() == &expected_head)
            .cloned()
            .ok_or(StructuredStoreError::InvalidHistory)?;
        verified_sources.push(source);
    }
    super::purpose::OfflineVerifiedRun::from_verified_closure(verified, verified_sources)
}
