use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use mfm_ids::{AppendRequestId, ContentDigest, RunId, TenantScopeId};
use mfm_journal::structured::{
    CommittedBatch, JournalHead, RunRecord, TenantFactCoordinate, TenantFactFrontier,
};

use super::backend::{
    BackendAppendOutcome, RawRunHistory, StructuredBackendFuture, StructuredHistoryBackend,
    StructuredRunSnapshot, StructuredStoreIdentity, TenantFactPublication,
};
use super::canonical_append::CanonicalRunAppend;
use super::fold::StructuredStoreError;

type AppendKey = (RunId, AppendRequestId);

#[derive(Default)]
struct MemoryState {
    histories: BTreeMap<RunId, Vec<CommittedBatch>>,
    appends: BTreeMap<AppendKey, CommittedBatch>,
    full_loads: usize,
    tenant_fact_heads: BTreeMap<TenantScopeId, u64>,
    tenant_fact_publications: BTreeMap<(TenantScopeId, u64), TenantFactPublication>,
    acknowledge_next_commit_as_unknown: bool,
    unavailable: bool,
}

/// Shared exact-head in-memory backend used by conformance and Runtime tests.
#[derive(Clone)]
pub struct StructuredMemoryBackend {
    identity: StructuredStoreIdentity,
    state: Arc<Mutex<MemoryState>>,
}

impl std::fmt::Debug for StructuredMemoryBackend {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("StructuredMemoryBackend")
            .field("identity", &self.identity)
            .finish_non_exhaustive()
    }
}

impl StructuredMemoryBackend {
    /// Constructs one shared memory backend under an exact writer identity.
    pub fn new(identity: StructuredStoreIdentity) -> Self {
        Self {
            identity,
            state: Arc::new(Mutex::new(MemoryState::default())),
        }
    }

    /// Makes the next successful commit lose only its acknowledgement.
    #[doc(hidden)]
    pub fn acknowledge_next_commit_as_unknown(&self) -> super::Result<()> {
        self.lock()?.acknowledge_next_commit_as_unknown = true;
        Ok(())
    }

    /// Toggles deterministic backend unavailability for fail-closed tests.
    #[doc(hidden)]
    pub fn set_unavailable(&self, unavailable: bool) -> super::Result<()> {
        self.lock()?.unavailable = unavailable;
        Ok(())
    }

    /// Returns the exact current head without performing a semantic fold.
    #[doc(hidden)]
    pub fn raw_head(&self, run_id: &RunId) -> super::Result<Option<JournalHead>> {
        Ok(self
            .lock()?
            .histories
            .get(run_id)
            .and_then(|batches| batches.last())
            .map(|batch| batch.head.clone()))
    }

    /// Returns the number of complete-prefix loads performed by this backend.
    #[doc(hidden)]
    pub fn full_loads(&self) -> super::Result<usize> {
        Ok(self.lock()?.full_loads)
    }

    fn lock(&self) -> super::Result<std::sync::MutexGuard<'_, MemoryState>> {
        self.state
            .lock()
            .map_err(|_| StructuredStoreError::BackendUnavailable)
    }
}

impl StructuredHistoryBackend for StructuredMemoryBackend {
    fn identity(&self) -> &StructuredStoreIdentity {
        &self.identity
    }

    fn load<'a>(&'a self, run_id: &'a RunId) -> StructuredBackendFuture<'a, Option<RawRunHistory>> {
        Box::pin(async move {
            let mut state = self.lock()?;
            if state.unavailable {
                return Err(StructuredStoreError::BackendUnavailable);
            }
            state.full_loads = state.full_loads.saturating_add(1);
            Ok(state.histories.get(run_id).map(|batches| RawRunHistory {
                run_id: run_id.clone(),
                batches: batches.clone(),
            }))
        })
    }

    fn load_snapshot<'a>(
        &'a self,
        run_id: &'a RunId,
    ) -> StructuredBackendFuture<'a, StructuredRunSnapshot> {
        Box::pin(async move {
            let mut state = self.lock()?;
            if state.unavailable {
                return Err(StructuredStoreError::BackendUnavailable);
            }
            state.full_loads = state.full_loads.saturating_add(1);
            let history = state.histories.get(run_id).map(|batches| RawRunHistory {
                run_id: run_id.clone(),
                batches: batches.clone(),
            });
            let head = history
                .as_ref()
                .and_then(|raw| raw.batches.last().map(|batch| batch.head.clone()));
            Ok(StructuredRunSnapshot { history, head })
        })
    }

    fn current_head<'a>(
        &'a self,
        run_id: &'a RunId,
    ) -> StructuredBackendFuture<'a, Option<JournalHead>> {
        Box::pin(async move {
            let state = self.lock()?;
            if state.unavailable {
                return Err(StructuredStoreError::BackendUnavailable);
            }
            Ok(state
                .histories
                .get(run_id)
                .and_then(|batches| batches.last().map(|batch| batch.head.clone())))
        })
    }

    fn load_prefix<'a>(
        &'a self,
        run_id: &'a RunId,
        through_sequence: u64,
    ) -> StructuredBackendFuture<'a, Option<RawRunHistory>> {
        Box::pin(async move {
            if through_sequence == 0 {
                return Err(StructuredStoreError::InvalidHistory);
            }
            let state = self.lock()?;
            if state.unavailable {
                return Err(StructuredStoreError::BackendUnavailable);
            }
            Ok(state.histories.get(run_id).and_then(|batches| {
                let batches = batches
                    .iter()
                    .take_while(|batch| batch.head.run_sequence <= through_sequence)
                    .cloned()
                    .collect::<Vec<_>>();
                (!batches.is_empty()).then(|| RawRunHistory {
                    run_id: run_id.clone(),
                    batches,
                })
            }))
        })
    }

    fn tenant_fact_frontier<'a>(
        &'a self,
        tenant_scope_id: &'a TenantScopeId,
    ) -> StructuredBackendFuture<'a, TenantFactFrontier> {
        Box::pin(async move {
            let state = self.lock()?;
            if state.unavailable {
                return Err(StructuredStoreError::BackendUnavailable);
            }
            Ok(TenantFactFrontier::new(
                self.identity.store_scope_id.clone(),
                self.identity.store_epoch,
                tenant_scope_id.clone(),
                state
                    .tenant_fact_heads
                    .get(tenant_scope_id)
                    .copied()
                    .unwrap_or(0),
            ))
        })
    }

    fn scan_fact_publications<'a>(
        &'a self,
        tenant_scope_id: &'a TenantScopeId,
        first_order: u64,
        through_order: u64,
        maximum_items: u32,
    ) -> StructuredBackendFuture<'a, Vec<TenantFactPublication>> {
        Box::pin(async move {
            if first_order == 0 || maximum_items == 0 || maximum_items > 1_024 {
                return Err(StructuredStoreError::InvalidHistory);
            }
            let maximum_items =
                usize::try_from(maximum_items).map_err(|_| StructuredStoreError::InvalidHistory)?;
            let state = self.lock()?;
            if state.unavailable {
                return Err(StructuredStoreError::BackendUnavailable);
            }
            if first_order > through_order {
                return Ok(Vec::new());
            }
            Ok(state
                .tenant_fact_publications
                .range(
                    (tenant_scope_id.clone(), first_order)
                        ..=(tenant_scope_id.clone(), through_order),
                )
                .take(maximum_items)
                .map(|(_, publication)| publication.clone())
                .collect())
        })
    }

    fn append<'a>(
        &'a self,
        batch: CanonicalRunAppend,
    ) -> StructuredBackendFuture<'a, BackendAppendOutcome> {
        Box::pin(async move {
            let committed = batch.into_committed();
            if committed.store_scope_id != self.identity.store_scope_id
                || committed.store_epoch != self.identity.store_epoch
            {
                return Err(StructuredStoreError::StaleHead);
            }
            let run_id = committed
                .records
                .first()
                .ok_or(StructuredStoreError::InvalidHistory)?
                .record_ref
                .run_id
                .clone();
            if committed
                .records
                .iter()
                .any(|record| record.record_ref.run_id != run_id)
            {
                return Err(StructuredStoreError::InvalidHistory);
            }
            super::canonical_append::validate_append_objects(&committed.objects)?;

            let mut state = self.lock()?;
            if state.unavailable {
                return Err(StructuredStoreError::BackendUnavailable);
            }
            let append_key = (run_id.clone(), committed.append_request_id.clone());
            if let Some(existing) = state.appends.get(&append_key) {
                return if existing == &committed {
                    Ok(BackendAppendOutcome::ExistingSame(existing.clone()))
                } else {
                    Err(StructuredStoreError::AppendConflict)
                };
            }
            let current_head = state
                .histories
                .get(&run_id)
                .and_then(|batches| batches.last())
                .map(|batch| &batch.head);
            if current_head != committed.predecessor.as_ref() {
                return Ok(BackendAppendOutcome::StaleHead);
            }

            let tenant_fact_publication = match &committed.tenant_fact_coordinate {
                TenantFactCoordinate::None => None,
                TenantFactCoordinate::FactSelectionBarrier { frontier } => {
                    let current_order = state
                        .tenant_fact_heads
                        .get(&frontier.tenant_scope_id)
                        .copied()
                        .unwrap_or(0);
                    if frontier.store_scope_id != self.identity.store_scope_id
                        || frontier.store_epoch != self.identity.store_epoch
                        || frontier.fact_order != current_order
                    {
                        return Ok(BackendAppendOutcome::StaleHead);
                    }
                    None
                }
                TenantFactCoordinate::FactPublication { frontier } => {
                    let current_order = state
                        .tenant_fact_heads
                        .get(&frontier.tenant_scope_id)
                        .copied()
                        .unwrap_or(0);
                    if frontier.store_scope_id != self.identity.store_scope_id
                        || frontier.store_epoch != self.identity.store_epoch
                        || current_order.checked_add(1) != Some(frontier.fact_order)
                    {
                        return Ok(BackendAppendOutcome::StaleHead);
                    }
                    let Some(transition) = committed.records.first() else {
                        return Err(StructuredStoreError::InvalidHistory);
                    };
                    let RunRecord::StateTransitionCommitted(record) = &transition.record else {
                        return Err(StructuredStoreError::InvalidHistory);
                    };
                    if record.facts.is_empty() {
                        return Err(StructuredStoreError::InvalidHistory);
                    }
                    Some(TenantFactPublication {
                        frontier: frontier.clone(),
                        transition_ref: transition.record_ref.clone(),
                    })
                }
            };
            if tenant_fact_publication.as_ref().is_some_and(|publication| {
                state.tenant_fact_publications.contains_key(&(
                    publication.frontier.tenant_scope_id.clone(),
                    publication.frontier.fact_order,
                ))
            }) {
                return Err(StructuredStoreError::InvalidHistory);
            }

            state
                .histories
                .entry(run_id)
                .or_default()
                .push(committed.clone());
            state.appends.insert(append_key, committed.clone());
            if let Some(publication) = tenant_fact_publication {
                let tenant_scope_id = publication.frontier.tenant_scope_id.clone();
                let fact_order = publication.frontier.fact_order;
                state
                    .tenant_fact_publications
                    .insert((tenant_scope_id.clone(), fact_order), publication);
                state.tenant_fact_heads.insert(tenant_scope_id, fact_order);
            }
            if std::mem::take(&mut state.acknowledge_next_commit_as_unknown) {
                Ok(BackendAppendOutcome::AcknowledgementUnknown)
            } else {
                Ok(BackendAppendOutcome::NewlyCommitted(committed))
            }
        })
    }

    fn resolve_append<'a>(
        &'a self,
        run_id: &'a RunId,
        append_request_id: &'a AppendRequestId,
        candidate_digest: &'a ContentDigest,
    ) -> StructuredBackendFuture<'a, Option<CommittedBatch>> {
        Box::pin(async move {
            let state = self.lock()?;
            if state.unavailable {
                return Err(StructuredStoreError::BackendUnavailable);
            }
            match state
                .appends
                .get(&(run_id.clone(), append_request_id.clone()))
            {
                Some(batch) if &batch.candidate_digest == candidate_digest => {
                    Ok(Some(batch.clone()))
                }
                Some(_) => Err(StructuredStoreError::AppendConflict),
                None => Ok(None),
            }
        })
    }
}

/// Test-support assembly of an in-memory Runtime and purpose readers.
#[cfg(any(test, feature = "test-support"))]
pub fn assemble_in_memory_runtime(
    identity: super::StructuredStoreIdentity,
    registry: mfm_certify::structured::QualifiedProgramRegistry,
    physical_binding_verifier: std::sync::Arc<dyn super::PublicPhysicalBindingVerifier>,
) -> super::AssembledStructuredRuntime<StructuredMemoryBackend> {
    let backend = StructuredMemoryBackend::new(identity);
    super::assemble_structured_runtime(backend, registry, physical_binding_verifier)
}
