use std::collections::BTreeMap;
use std::ops::Bound::{Excluded, Unbounded};
use std::sync::Arc;

use mfm_ids::{AppendRequestId, RunId, TenantScopeId};
use mfm_journal::structured::{CommittedBatch, JournalHead, TenantFactFrontier};

use super::backend::{
    AppendAttemptLookup, BackendAppendOutcome, RawHistoryLoadLimit, RawRunHistory,
    StructuredBackendFuture, StructuredHistoryBackend, StructuredRunSnapshot,
    StructuredSemanticOpenAudit, StructuredStoreIdentity, TenantFactPublication,
    SEMANTIC_OPEN_KEY_PAGE_ITEMS, SEMANTIC_OPEN_ROUTE_PAGE_ITEMS,
};
use super::qualification::StructuredStoreError;
use super::validated_append::{RunCurrentProjection, TenantFactProjectionPlan, ValidatedRunAppend};

#[derive(Default)]
struct MemoryState {
    histories: BTreeMap<RunId, Vec<CommittedBatch>>,
    projections: BTreeMap<RunId, RunCurrentProjection>,
    tenant_fact_heads: BTreeMap<TenantScopeId, u64>,
    tenant_fact_publications: BTreeMap<(TenantScopeId, u64), TenantFactPublication>,
    run_fact_publications: BTreeMap<(RunId, u64), TenantFactPublication>,
    acknowledge_next_commit_as_unknown: bool,
    unavailable: bool,
}

/// Shared mechanical in-memory backend used by conformance tests.
#[derive(Clone)]
pub struct StructuredMemoryBackend {
    identity: StructuredStoreIdentity,
    state: Arc<tokio::sync::RwLock<MemoryState>>,
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
            state: Arc::new(tokio::sync::RwLock::new(MemoryState::default())),
        }
    }

    /// Makes the next successful commit lose only its acknowledgement.
    #[doc(hidden)]
    pub fn acknowledge_next_commit_as_unknown(&self) -> super::Result<()> {
        self.try_write()?.acknowledge_next_commit_as_unknown = true;
        Ok(())
    }

    /// Toggles deterministic backend unavailability.
    #[doc(hidden)]
    pub fn set_unavailable(&self, unavailable: bool) -> super::Result<()> {
        self.try_write()?.unavailable = unavailable;
        Ok(())
    }

    fn try_write(&self) -> super::Result<tokio::sync::RwLockWriteGuard<'_, MemoryState>> {
        self.state
            .try_write()
            .map_err(|_| StructuredStoreError::BackendUnavailable)
    }
}

struct MemorySemanticOpenAudit {
    identity: StructuredStoreIdentity,
    state: tokio::sync::OwnedRwLockReadGuard<MemoryState>,
}

impl StructuredSemanticOpenAudit for MemorySemanticOpenAudit {
    fn scan_run_ids<'a>(
        &'a mut self,
        after_run_id: Option<&'a RunId>,
    ) -> StructuredBackendFuture<'a, Vec<RunId>> {
        Box::pin(async move {
            let mut cursor = after_run_id.cloned();
            let mut page = Vec::with_capacity(SEMANTIC_OPEN_KEY_PAGE_ITEMS as usize);
            while page.len() < SEMANTIC_OPEN_KEY_PAGE_ITEMS as usize {
                let Some(run_id) = next_run_id(&self.state, cursor.as_ref()) else {
                    break;
                };
                cursor = Some(run_id.clone());
                page.push(run_id);
            }
            Ok(page)
        })
    }

    fn load_run_projection<'a>(
        &'a mut self,
        run_id: &'a RunId,
    ) -> StructuredBackendFuture<'a, Option<RunCurrentProjection>> {
        Box::pin(async move { Ok(self.state.projections.get(run_id).cloned()) })
    }

    fn load_run_history_bounded<'a>(
        &'a mut self,
        run_id: &'a RunId,
        through: Option<&'a JournalHead>,
        limit: RawHistoryLoadLimit,
    ) -> StructuredBackendFuture<'a, Option<RawRunHistory>> {
        Box::pin(async move {
            let Some(stored) = self.state.histories.get(run_id) else {
                return Ok(None);
            };
            let batches = match through {
                Some(head) => {
                    let Some(position) = stored.iter().position(|batch| &batch.head == head) else {
                        return Ok(None);
                    };
                    &stored[..=position]
                }
                None => stored.as_slice(),
            };
            limit.validate_batches(batches)?;
            Ok(Some(RawRunHistory {
                run_id: run_id.clone(),
                batches: batches.to_vec(),
            }))
        })
    }

    fn scan_run_publications<'a>(
        &'a mut self,
        run_id: &'a RunId,
        after_run_sequence: Option<u64>,
    ) -> StructuredBackendFuture<'a, Vec<TenantFactPublication>> {
        Box::pin(async move {
            Ok(self
                .state
                .run_fact_publications
                .range((run_id.clone(), u64::MIN)..=(run_id.clone(), u64::MAX))
                .filter(|((_, sequence), _)| {
                    after_run_sequence.is_none_or(|after| *sequence > after)
                })
                .take(SEMANTIC_OPEN_ROUTE_PAGE_ITEMS as usize)
                .map(|(_, publication)| publication.clone())
                .collect())
        })
    }

    fn scan_fact_tenants<'a>(
        &'a mut self,
        after_tenant_scope_id: Option<&'a TenantScopeId>,
    ) -> StructuredBackendFuture<'a, Vec<TenantScopeId>> {
        Box::pin(async move {
            let mut cursor = after_tenant_scope_id.cloned();
            let mut page = Vec::with_capacity(SEMANTIC_OPEN_KEY_PAGE_ITEMS as usize);
            while page.len() < SEMANTIC_OPEN_KEY_PAGE_ITEMS as usize {
                let Some(tenant) = next_tenant(&self.state, cursor.as_ref()) else {
                    break;
                };
                cursor = Some(tenant.clone());
                page.push(tenant);
            }
            Ok(page)
        })
    }

    fn load_fact_head<'a>(
        &'a mut self,
        tenant_scope_id: &'a TenantScopeId,
    ) -> StructuredBackendFuture<'a, Option<TenantFactFrontier>> {
        Box::pin(async move {
            Ok(self
                .state
                .tenant_fact_heads
                .get(tenant_scope_id)
                .map(|order| {
                    TenantFactFrontier::new(
                        self.identity.store_scope_id.clone(),
                        self.identity.store_epoch,
                        tenant_scope_id.clone(),
                        *order,
                    )
                }))
        })
    }

    fn scan_tenant_publications<'a>(
        &'a mut self,
        tenant_scope_id: &'a TenantScopeId,
        after_fact_order: Option<u64>,
    ) -> StructuredBackendFuture<'a, Vec<TenantFactPublication>> {
        Box::pin(async move {
            Ok(self
                .state
                .tenant_fact_publications
                .range((tenant_scope_id.clone(), u64::MIN)..=(tenant_scope_id.clone(), u64::MAX))
                .filter(|((_, order), _)| after_fact_order.is_none_or(|after| *order > after))
                .take(SEMANTIC_OPEN_ROUTE_PAGE_ITEMS as usize)
                .map(|(_, publication)| publication.clone())
                .collect())
        })
    }

    fn finish(self: Box<Self>) -> StructuredBackendFuture<'static, ()> {
        Box::pin(async move {
            drop(self);
            Ok(())
        })
    }
}

fn next_run_id(state: &MemoryState, after: Option<&RunId>) -> Option<RunId> {
    let history = match after {
        Some(after) => state
            .histories
            .range((Excluded(after.clone()), Unbounded))
            .next()
            .map(|(run_id, _)| run_id),
        None => state.histories.keys().next(),
    };
    let projection = match after {
        Some(after) => state
            .projections
            .range((Excluded(after.clone()), Unbounded))
            .next()
            .map(|(run_id, _)| run_id),
        None => state.projections.keys().next(),
    };
    let publication = match after {
        Some(after) => state
            .run_fact_publications
            .range((Excluded((after.clone(), u64::MAX)), Unbounded))
            .next()
            .map(|((run_id, _), _)| run_id),
        None => state
            .run_fact_publications
            .keys()
            .next()
            .map(|(run_id, _)| run_id),
    };
    [history, projection, publication]
        .into_iter()
        .flatten()
        .min()
        .cloned()
}

fn next_tenant(state: &MemoryState, after: Option<&TenantScopeId>) -> Option<TenantScopeId> {
    let head = match after {
        Some(after) => state
            .tenant_fact_heads
            .range((Excluded(after.clone()), Unbounded))
            .next()
            .map(|(tenant, _)| tenant),
        None => state.tenant_fact_heads.keys().next(),
    };
    let publication = match after {
        Some(after) => state
            .tenant_fact_publications
            .range((Excluded((after.clone(), u64::MAX)), Unbounded))
            .next()
            .map(|((tenant, _), _)| tenant),
        None => state
            .tenant_fact_publications
            .keys()
            .next()
            .map(|(tenant, _)| tenant),
    };
    [head, publication].into_iter().flatten().min().cloned()
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
            let state = self.state.read().await;
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
            let state = self.state.read().await;
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
            let state = self.state.read().await;
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
            let state = self.state.read().await;
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

    fn begin_semantic_open_audit(
        &self,
    ) -> StructuredBackendFuture<'_, Box<dyn StructuredSemanticOpenAudit>> {
        Box::pin(async move {
            let state = Arc::clone(&self.state).read_owned().await;
            if state.unavailable {
                return Err(StructuredStoreError::BackendUnavailable);
            }
            Ok(Box::new(MemorySemanticOpenAudit {
                identity: self.identity.clone(),
                state,
            }) as Box<dyn StructuredSemanticOpenAudit>)
        })
    }

    fn validate_authority(&self) -> StructuredBackendFuture<'_, ()> {
        Box::pin(async move {
            if self.state.read().await.unavailable {
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
            let state = self.state.read().await;
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
            let state = self.state.read().await;
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
            let mut state = self.state.write().await;
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
                let run_id = publication.transition_ref.run_id.clone();
                let run_sequence = publication.transition_ref.run_sequence;
                state
                    .tenant_fact_publications
                    .insert((tenant.clone(), order), publication.clone());
                state
                    .run_fact_publications
                    .insert((run_id, run_sequence), publication);
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
            let state = self.state.read().await;
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
