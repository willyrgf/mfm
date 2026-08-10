//! Sole asynchronous transition from physical usability to semantic authority.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use mfm_journal::structured::{TenantFactCoordinate, TenantFactFrontier};

use super::adapter::StoreHistoryAdapter;
use super::backend::{
    prior_run_fact_source, RawRunHistory, StructuredBackendFuture, StructuredHistoryBackend,
    StructuredRunHistoryReader, StructuredRunHistoryWriter, StructuredStoreSnapshot,
    TenantFactProjectionSnapshot, TenantFactPublication,
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
    qualify_store_snapshot(backend.load_store_snapshot().await?, &programs, &physical).await?;
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
    let snapshot = backend.load_snapshot(run_id).await?;
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
            &finalized.tenant_fact_plan
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

struct SnapshotFactSource {
    histories: BTreeMap<mfm_ids::RunId, RawRunHistory>,
    publications: BTreeMap<mfm_ids::TenantScopeId, Vec<TenantFactPublication>>,
}

impl SnapshotFactSource {
    fn new(snapshot: &StructuredStoreSnapshot) -> super::Result<Self> {
        let mut histories = BTreeMap::new();
        for run in &snapshot.runs {
            if let Some(history) = &run.history {
                if history.run_id != run.run_id
                    || histories
                        .insert(run.run_id.clone(), history.clone())
                        .is_some()
                {
                    return Err(StructuredStoreError::InvalidHistory);
                }
            }
        }
        let mut publications = BTreeMap::new();
        for tenant in &snapshot.tenant_facts {
            if publications
                .insert(tenant.tenant_scope_id.clone(), tenant.publications.clone())
                .is_some()
            {
                return Err(StructuredStoreError::InvalidHistory);
            }
        }
        Ok(Self {
            histories,
            publications,
        })
    }

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
                .insert(key.clone(), publication.clone())
                .is_some_and(|existing| existing != *publication)
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
        })
    }
}

impl super::fact_scan::PriorRunFactSource for SnapshotFactSource {
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
            Ok(self
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
                .collect())
        })
    }

    fn load_producer_prefix<'a>(
        &'a self,
        run_id: &'a mfm_ids::RunId,
        through: &'a mfm_journal::structured::JournalHead,
    ) -> StructuredBackendFuture<'a, Option<RawRunHistory>> {
        Box::pin(async move {
            Ok(self.histories.get(run_id).and_then(|history| {
                let position = history
                    .batches
                    .iter()
                    .position(|batch| &batch.head == through)?;
                Some(RawRunHistory {
                    run_id: run_id.clone(),
                    batches: history.batches[..=position].to_vec(),
                })
            }))
        })
    }
}

async fn qualify_store_snapshot(
    snapshot: StructuredStoreSnapshot,
    programs: &Arc<ProgramVerificationRegistry>,
    physical: &Arc<dyn PhysicalObligationChecker>,
) -> super::Result<()> {
    let source: Arc<dyn super::fact_scan::PriorRunFactSource> =
        Arc::new(SnapshotFactSource::new(&snapshot)?);
    let prefix_memo = Arc::new(super::fact_scan::PrefixVerificationMemo::default());
    let mut seen_runs = BTreeSet::new();
    let mut expected_publications = BTreeMap::new();
    for run in &snapshot.runs {
        if !seen_runs.insert(run.run_id.clone()) {
            return Err(StructuredStoreError::InvalidHistory);
        }
        let (raw, current) = match (&run.history, &run.current_projection) {
            (Some(raw), Some(current)) if raw.run_id == run.run_id => (raw.clone(), current),
            _ => return Err(StructuredStoreError::InvalidHistory),
        };
        let (verified, publications) =
            super::fact_scan::qualify_and_reduce_for_scan_with_publications_and_memo(
                Arc::clone(&source),
                raw,
                Arc::clone(programs),
                Arc::clone(physical),
                Arc::clone(&prefix_memo),
            )
            .await?;
        if current != &verified.current_projection() {
            return Err(StructuredStoreError::InvalidHistory);
        }
        for publication in publications {
            let key = (
                publication.frontier.tenant_scope_id.clone(),
                publication.frontier.fact_order,
            );
            if expected_publications.insert(key, publication).is_some() {
                return Err(StructuredStoreError::InvalidHistory);
            }
        }
    }
    compare_fact_projections(&snapshot.tenant_facts, expected_publications)
}

fn compare_fact_projections(
    actual: &[TenantFactProjectionSnapshot],
    expected: BTreeMap<(mfm_ids::TenantScopeId, u64), TenantFactPublication>,
) -> super::Result<()> {
    let mut actual_by_tenant = BTreeMap::new();
    for projection in actual {
        if actual_by_tenant
            .insert(projection.tenant_scope_id.clone(), projection)
            .is_some()
        {
            return Err(StructuredStoreError::InvalidHistory);
        }
    }
    let tenants = actual_by_tenant
        .keys()
        .chain(expected.keys().map(|(tenant, _)| tenant))
        .cloned()
        .collect::<BTreeSet<_>>();
    for tenant in tenants {
        let expected_publications = expected
            .range((tenant.clone(), u64::MIN)..=(tenant.clone(), u64::MAX))
            .map(|(_, publication)| publication.clone())
            .collect::<Vec<_>>();
        if expected_publications
            .iter()
            .enumerate()
            .any(|(index, publication)| {
                publication.frontier.fact_order != (index as u64).saturating_add(1)
            })
        {
            return Err(StructuredStoreError::InvalidHistory);
        }
        let expected_frontier = expected_publications
            .last()
            .map(|publication| publication.frontier.clone());
        let (actual_frontier, actual_publications) =
            actual_by_tenant
                .get(&tenant)
                .map_or((None, &[][..]), |projection| {
                    (
                        projection.current_frontier.as_ref(),
                        projection.publications.as_slice(),
                    )
                });
        if actual_frontier != expected_frontier.as_ref()
            || actual_publications != expected_publications
        {
            return Err(StructuredStoreError::InvalidHistory);
        }
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

pub(super) fn verify_offline_history(
    raw: RawRunHistory,
    programs: &ProgramVerificationRegistry,
    physical: &dyn PhysicalObligationChecker,
) -> super::Result<super::purpose::OfflineVerifiedRun> {
    super::purpose::OfflineVerifiedRun::from_verified(verify_qualified(raw, programs, physical)?)
}

/// Verifies retained fact responses by executing the live selector over the supplied closure.
pub async fn verify_offline_run_closure(
    closure: OfflineRunClosure,
    programs: Arc<ProgramVerificationRegistry>,
    physical: Arc<dyn PhysicalObligationChecker>,
) -> super::Result<super::purpose::OfflineVerifiedRun> {
    let expected_sources = closure
        .source_prefixes
        .iter()
        .map(|history| history.run_id.clone())
        .collect::<BTreeSet<_>>();
    if expected_sources.len() != closure.source_prefixes.len()
        || expected_sources.contains(&closure.root.run_id)
    {
        return Err(StructuredStoreError::InvalidHistory);
    }
    let source: Arc<dyn super::fact_scan::PriorRunFactSource> =
        Arc::new(SnapshotFactSource::from_offline_closure(&closure)?);
    let prefix_memo = Arc::new(super::fact_scan::PrefixVerificationMemo::default());
    let (verified, _) = super::fact_scan::qualify_and_reduce_for_scan_with_publications_and_memo(
        source,
        closure.root,
        programs,
        physical,
        Arc::clone(&prefix_memo),
    )
    .await?;
    if prefix_memo.verified_run_ids()? != expected_sources {
        return Err(StructuredStoreError::InvalidHistory);
    }
    super::purpose::OfflineVerifiedRun::from_verified(verified)
}
