use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use mfm_ids::{AppendRequestId, RunId, TenantScopeId};
use mfm_journal::structured::{CommittedBatch, JournalHead, TenantFactFrontier};

use super::backend::{
    AppendAttemptLookup, BackendAppendOutcome, RawHistoryLoadLimit, RawRunHistory,
    StructuredBackendFuture, StructuredHistoryBackend, StructuredRunSnapshot,
    StructuredStoreIdentity, StructuredStoreRunSnapshot, StructuredStoreSnapshot,
    TenantFactProjectionSnapshot, TenantFactPublication,
};
use super::qualification::StructuredStoreError;
use super::validated_append::{RunCurrentProjection, TenantFactProjectionPlan, ValidatedRunAppend};

#[derive(Default)]
struct MemoryState {
    histories: BTreeMap<RunId, Vec<CommittedBatch>>,
    projections: BTreeMap<RunId, RunCurrentProjection>,
    tenant_fact_heads: BTreeMap<TenantScopeId, u64>,
    tenant_fact_publications: BTreeMap<(TenantScopeId, u64), TenantFactPublication>,
    acknowledge_next_commit_as_unknown: bool,
    unavailable: bool,
}

/// Shared mechanical in-memory backend used by conformance tests.
#[derive(Clone)]
pub struct StructuredMemoryBackend {
    identity: StructuredStoreIdentity,
    state: Arc<Mutex<MemoryState>>,
}

impl mfm_authority_seal::ValidatedAppendConsumerSeal for StructuredMemoryBackend {}

impl std::fmt::Debug for StructuredMemoryBackend {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("StructuredMemoryBackend")
            .field("identity", &self.identity)
            .finish_non_exhaustive()
    }
}

impl StructuredMemoryBackend {
    /// Constructs one memory backend under an exact writer identity.
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

    /// Toggles deterministic backend unavailability.
    #[doc(hidden)]
    pub fn set_unavailable(&self, unavailable: bool) -> super::Result<()> {
        self.lock()?.unavailable = unavailable;
        Ok(())
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

    fn load_snapshot<'a>(
        &'a self,
        run_id: &'a RunId,
        limit: RawHistoryLoadLimit,
    ) -> StructuredBackendFuture<'a, StructuredRunSnapshot> {
        Box::pin(async move {
            let state = self.lock()?;
            if state.unavailable {
                return Err(StructuredStoreError::BackendUnavailable);
            }
            let history = state
                .histories
                .get(run_id)
                .map(|batches| {
                    limit.validate_batches(batches)?;
                    Ok(RawRunHistory {
                        run_id: run_id.clone(),
                        batches: batches.clone(),
                    })
                })
                .transpose()?;
            Ok(StructuredRunSnapshot {
                history,
                current_projection: state.projections.get(run_id).cloned(),
            })
        })
    }

    fn load_prefix<'a>(
        &'a self,
        run_id: &'a RunId,
        through: &'a JournalHead,
        limit: RawHistoryLoadLimit,
    ) -> StructuredBackendFuture<'a, Option<RawRunHistory>> {
        Box::pin(async move {
            let state = self.lock()?;
            if state.unavailable {
                return Err(StructuredStoreError::BackendUnavailable);
            }
            let Some(batches) = state.histories.get(run_id) else {
                return Ok(None);
            };
            let Some(position) = batches.iter().position(|batch| &batch.head == through) else {
                return Ok(None);
            };
            let batches = &batches[..=position];
            limit.validate_batches(batches)?;
            Ok(Some(RawRunHistory {
                run_id: run_id.clone(),
                batches: batches.to_vec(),
            }))
        })
    }

    fn current_run_projection<'a>(
        &'a self,
        run_id: &'a RunId,
    ) -> StructuredBackendFuture<'a, Option<RunCurrentProjection>> {
        Box::pin(async move {
            let state = self.lock()?;
            if state.unavailable {
                return Err(StructuredStoreError::BackendUnavailable);
            }
            Ok(state.projections.get(run_id).cloned())
        })
    }

    fn lookup_append_attempt<'a>(
        &'a self,
        run_id: &'a RunId,
        append_request_id: &'a AppendRequestId,
    ) -> StructuredBackendFuture<'a, Option<AppendAttemptLookup>> {
        Box::pin(async move {
            let state = self.lock()?;
            if state.unavailable {
                return Err(StructuredStoreError::BackendUnavailable);
            }
            let Some(batches) = state.histories.get(run_id) else {
                return Ok(None);
            };
            let Some(position) = batches
                .iter()
                .position(|batch| &batch.append_request_id == append_request_id)
            else {
                return Ok(None);
            };
            let batches = &batches[..=position];
            RawHistoryLoadLimit::run().validate_batches(batches)?;
            Ok(Some(AppendAttemptLookup {
                history: RawRunHistory {
                    run_id: run_id.clone(),
                    batches: batches.to_vec(),
                },
            }))
        })
    }

    fn scan_run_ids<'a>(
        &'a self,
        after_run_id: Option<&'a RunId>,
        maximum_items: u32,
    ) -> StructuredBackendFuture<'a, Vec<RunId>> {
        Box::pin(async move {
            if maximum_items == 0 {
                return Err(StructuredStoreError::InvalidHistory);
            }
            let state = self.lock()?;
            if state.unavailable {
                return Err(StructuredStoreError::BackendUnavailable);
            }
            let keys = state
                .histories
                .keys()
                .chain(state.projections.keys())
                .cloned()
                .collect::<BTreeSet<_>>();
            Ok(keys
                .into_iter()
                .filter(|run_id| after_run_id.is_none_or(|after| run_id > after))
                .take(maximum_items as usize)
                .collect())
        })
    }

    fn load_store_snapshot(&self) -> StructuredBackendFuture<'_, StructuredStoreSnapshot> {
        Box::pin(async move {
            let state = self.lock()?;
            if state.unavailable {
                return Err(StructuredStoreError::BackendUnavailable);
            }
            let run_ids = state
                .histories
                .keys()
                .chain(state.projections.keys())
                .cloned()
                .collect::<BTreeSet<_>>();
            let runs = run_ids
                .into_iter()
                .map(|run_id| StructuredStoreRunSnapshot {
                    history: state.histories.get(&run_id).map(|batches| RawRunHistory {
                        run_id: run_id.clone(),
                        batches: batches.clone(),
                    }),
                    current_projection: state.projections.get(&run_id).cloned(),
                    run_id,
                })
                .collect();
            let tenants = state
                .tenant_fact_heads
                .keys()
                .chain(
                    state
                        .tenant_fact_publications
                        .keys()
                        .map(|(tenant, _)| tenant),
                )
                .cloned()
                .collect::<BTreeSet<_>>();
            let tenant_facts = tenants
                .into_iter()
                .map(|tenant_scope_id| TenantFactProjectionSnapshot {
                    current_frontier: state.tenant_fact_heads.get(&tenant_scope_id).map(|order| {
                        TenantFactFrontier::new(
                            self.identity.store_scope_id.clone(),
                            self.identity.store_epoch,
                            tenant_scope_id.clone(),
                            *order,
                        )
                    }),
                    publications: state
                        .tenant_fact_publications
                        .range(
                            (tenant_scope_id.clone(), u64::MIN)
                                ..=(tenant_scope_id.clone(), u64::MAX),
                        )
                        .map(|(_, publication)| publication.clone())
                        .collect(),
                    tenant_scope_id,
                })
                .collect();
            Ok(StructuredStoreSnapshot { runs, tenant_facts })
        })
    }

    fn validate_authority(&self) -> StructuredBackendFuture<'_, ()> {
        Box::pin(async move {
            if self.lock()?.unavailable {
                Err(StructuredStoreError::BackendUnavailable)
            } else {
                Ok(())
            }
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
                .take(maximum_items as usize)
                .map(|(_, publication)| publication.clone())
                .collect())
        })
    }

    fn append<'a>(
        &'a self,
        command: ValidatedRunAppend,
    ) -> StructuredBackendFuture<'a, BackendAppendOutcome> {
        Box::pin(async move {
            let (committed, run_plan, tenant_plan) = command.into_parts(self);
            if committed.store_scope_id != self.identity.store_scope_id
                || committed.store_epoch != self.identity.store_epoch
            {
                return Ok(BackendAppendOutcome::StaleHead);
            }
            let run_id = run_plan.successor().run_id.clone();
            let mut state = self.lock()?;
            if state.unavailable {
                return Err(StructuredStoreError::BackendUnavailable);
            }
            if let Some(existing) = state.histories.get(&run_id).and_then(|batches| {
                batches
                    .iter()
                    .find(|batch| batch.append_request_id == committed.append_request_id)
            }) {
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
            if current_head != committed.predecessor.as_ref()
                || state.projections.get(&run_id) != run_plan.expected()
            {
                return Ok(BackendAppendOutcome::StaleHead);
            }
            RawHistoryLoadLimit::run().validate_batch_iter(
                state
                    .histories
                    .get(&run_id)
                    .into_iter()
                    .flatten()
                    .chain(std::iter::once(&committed)),
            )?;
            match &tenant_plan {
                TenantFactProjectionPlan::None => {}
                TenantFactProjectionPlan::Barrier { expected_frontier } => {
                    if current_fact_frontier(
                        &self.identity,
                        &state,
                        &expected_frontier.tenant_scope_id,
                    ) != *expected_frontier
                    {
                        return Ok(BackendAppendOutcome::StaleHead);
                    }
                }
                TenantFactProjectionPlan::Publish {
                    expected_predecessor,
                    publication,
                } => {
                    if current_fact_frontier(
                        &self.identity,
                        &state,
                        &expected_predecessor.tenant_scope_id,
                    ) != *expected_predecessor
                        || state.tenant_fact_publications.contains_key(&(
                            publication.frontier.tenant_scope_id.clone(),
                            publication.frontier.fact_order,
                        ))
                    {
                        return Ok(BackendAppendOutcome::StaleHead);
                    }
                }
            }
            state
                .histories
                .entry(run_id.clone())
                .or_default()
                .push(committed.clone());
            state
                .projections
                .insert(run_id, run_plan.successor().clone());
            if let TenantFactProjectionPlan::Publish { publication, .. } = tenant_plan {
                let tenant = publication.frontier.tenant_scope_id.clone();
                let order = publication.frontier.fact_order;
                state
                    .tenant_fact_publications
                    .insert((tenant.clone(), order), publication);
                state.tenant_fact_heads.insert(tenant, order);
            }
            if std::mem::take(&mut state.acknowledge_next_commit_as_unknown) {
                Ok(BackendAppendOutcome::AcknowledgementUnknown)
            } else {
                Ok(BackendAppendOutcome::NewlyCommitted(committed))
            }
        })
    }

    fn scan_effect_entry_attention_routes<'a>(
        &'a self,
        tenant_scope_id: &'a TenantScopeId,
        after_run_id: Option<&'a RunId>,
        maximum_items: u32,
    ) -> StructuredBackendFuture<'a, Vec<super::backend::EffectEntryAttentionRoute>> {
        Box::pin(async move {
            if maximum_items == 0 {
                return Err(StructuredStoreError::InvalidHistory);
            }
            let state = self.lock()?;
            if state.unavailable {
                return Err(StructuredStoreError::BackendUnavailable);
            }
            Ok(state
                .projections
                .values()
                .filter(|projection| {
                    projection.has_effect_entry_attention
                        && &projection.tenant_scope_id == tenant_scope_id
                        && after_run_id.is_none_or(|after| &projection.run_id > after)
                })
                .take(maximum_items as usize)
                .map(|projection| super::backend::EffectEntryAttentionRoute {
                    run_id: projection.run_id.clone(),
                    journal_head: projection.journal_head.clone(),
                })
                .collect())
        })
    }
}

fn current_fact_frontier(
    identity: &StructuredStoreIdentity,
    state: &MemoryState,
    tenant: &TenantScopeId,
) -> TenantFactFrontier {
    TenantFactFrontier::new(
        identity.store_scope_id.clone(),
        identity.store_epoch,
        tenant.clone(),
        state.tenant_fact_heads.get(tenant).copied().unwrap_or(0),
    )
}
