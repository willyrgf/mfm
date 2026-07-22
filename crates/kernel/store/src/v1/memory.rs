use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use super::*;

/// Private state behind the async typed store test backend.
#[derive(Debug, Clone)]
struct RunMemoryCore {
    store_scope_id: StoreScopeId,
    next_store_commit_order: StoreCommitOrder,
    streams: BTreeMap<RunId, Vec<CommittedBatch>>,
    commit_keys: BTreeMap<(RunId, CommitKey), CommitKeyRecord>,
    artifacts: ArtifactAuthorityMap,
    artifact_bytes: BTreeMap<(ArtifactId, ContentDigest), (Vec<u8>, ArtifactEvidenceRef)>,
    logical_keys: BTreeSet<(RunId, LogicalEventKey)>,
    unique_logical_payloads: BTreeMap<(RunId, LogicalEventKey), ContentDigest>,
    projections: ProjectionSnapshot,
    resource_lane_authority: ResourceLaneAuthoritySet,
    execution_claims: BTreeMap<AdmissionLaneKey, MemoryExecutionClaim>,
}

impl Default for RunMemoryCore {
    fn default() -> Self {
        Self {
            store_scope_id: StoreScopeId::new(
                "mfm.store_scope.v1:00000000000000000000000000000000",
            )
            .expect("test store scope"),
            next_store_commit_order: StoreCommitOrder::FIRST,
            streams: BTreeMap::new(),
            commit_keys: BTreeMap::new(),
            artifacts: BTreeMap::new(),
            artifact_bytes: BTreeMap::new(),
            logical_keys: BTreeSet::new(),
            unique_logical_payloads: BTreeMap::new(),
            projections: ProjectionSnapshot::default(),
            resource_lane_authority: ResourceLaneAuthoritySet::default(),
            execution_claims: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MemoryExecutionClaim {
    lane: ExecutionClaimAdmissionLane,
    holder_run_id: RunId,
    token: AdmissionToken,
    lease_expires_at_unix_ms: i64,
}

impl RunMemoryCore {
    fn stream_events(&self, run_id: &RunId) -> Vec<KernelEventEnvelope> {
        self.streams
            .get(run_id)
            .into_iter()
            .flat_map(|batches| batches.iter())
            .flat_map(|batch| batch.events.iter().cloned())
            .collect()
    }

    fn acquire_execution_claim(
        &mut self,
        scope: &ExecutionClaimScope,
        holder_run_id: &RunId,
        token: AdmissionToken,
    ) -> Result<NowaitSkipAdmissionResult> {
        let lane = ExecutionClaimAdmissionLane::from_scope(scope)?;
        let lane_key = lane.erased_key();
        if let Some(holder) = self.execution_claims.get(&lane_key) {
            return Ok(NowaitSkipAdmissionResult::Busy(NowaitSkipAdmissionBusy {
                lane,
                holder: Some(execution_claim_lease(holder)),
            }));
        }
        let lease = AdmissionLease {
            lane: lane_key.clone(),
            holder_run_id: holder_run_id.clone(),
            token,
            lease_expires_at_unix_ms: execution_claim_expiry_unix_ms()?,
        };
        self.execution_claims.insert(
            lane_key,
            MemoryExecutionClaim {
                lane,
                holder_run_id: holder_run_id.clone(),
                token: lease.token.clone(),
                lease_expires_at_unix_ms: lease.lease_expires_at_unix_ms,
            },
        );
        Ok(NowaitSkipAdmissionResult::Admitted(lease))
    }

    fn execution_claim_status(&self, scope: &ExecutionClaimScope) -> Result<ExecutionClaimStatus> {
        let lane = ExecutionClaimAdmissionLane::from_scope(scope)?;
        let Some(holder) = self.execution_claims.get(&lane.erased_key()) else {
            return Ok(ExecutionClaimStatus::Unclaimed);
        };
        let lease = execution_claim_lease(holder);
        if holder.lease_expires_at_unix_ms <= unix_time_ms()? {
            Ok(ExecutionClaimStatus::Expired(lease))
        } else {
            Ok(ExecutionClaimStatus::Live(lease))
        }
    }

    fn renew_execution_claim(
        &mut self,
        scope: &ExecutionClaimScope,
        holder_run_id: &RunId,
        token: &AdmissionToken,
    ) -> Result<Option<AdmissionLease>> {
        let lane = ExecutionClaimAdmissionLane::from_scope(scope)?;
        let Some(holder) = self.execution_claims.get_mut(&lane.erased_key()) else {
            return Ok(None);
        };
        if &holder.holder_run_id != holder_run_id || &holder.token != token {
            return Ok(None);
        }
        holder.lease_expires_at_unix_ms = execution_claim_expiry_unix_ms()?;
        Ok(Some(execution_claim_lease(holder)))
    }

    fn release_execution_claim(
        &mut self,
        scope: &ExecutionClaimScope,
        holder_run_id: &RunId,
        token: &AdmissionToken,
    ) -> Result<bool> {
        let lane = ExecutionClaimAdmissionLane::from_scope(scope)?;
        let lane_key = lane.erased_key();
        let matches = self
            .execution_claims
            .get(&lane_key)
            .map(|holder| &holder.holder_run_id == holder_run_id && &holder.token == token)
            .unwrap_or(false);
        if matches {
            self.execution_claims.remove(&lane_key);
        }
        Ok(matches)
    }

    fn expired_execution_claims(&self) -> Result<Vec<ExpiredExecutionClaim>> {
        let now = unix_time_ms()?;
        Ok(self
            .execution_claims
            .iter()
            .filter(|(_, holder)| holder.lease_expires_at_unix_ms <= now)
            .map(|(_, holder)| ExpiredExecutionClaim {
                run_id: holder.holder_run_id.clone(),
                lane: holder.lane.clone(),
                lease: execution_claim_lease(holder),
            })
            .collect())
    }

    fn reap_expired_execution_claim(
        &mut self,
        scope: &ExecutionClaimScope,
        holder_run_id: &RunId,
        token: &AdmissionToken,
    ) -> Result<bool> {
        let lane = ExecutionClaimAdmissionLane::from_scope(scope)?;
        let lane_key = lane.erased_key();
        let now = unix_time_ms()?;
        let matches = self
            .execution_claims
            .get(&lane_key)
            .map(|holder| {
                &holder.holder_run_id == holder_run_id
                    && &holder.token == token
                    && holder.lease_expires_at_unix_ms <= now
            })
            .unwrap_or(false);
        if matches {
            self.execution_claims.remove(&lane_key);
        }
        Ok(matches)
    }
}

fn execution_claim_lease(holder: &MemoryExecutionClaim) -> AdmissionLease {
    AdmissionLease {
        lane: holder.lane.erased_key(),
        holder_run_id: holder.holder_run_id.clone(),
        token: holder.token.clone(),
        lease_expires_at_unix_ms: holder.lease_expires_at_unix_ms,
    }
}

fn execution_claim_expiry_unix_ms() -> Result<i64> {
    let ttl_ms = i64::try_from(EXECUTION_CLAIM_LEASE_TTL_SECS)
        .ok()
        .and_then(|secs| secs.checked_mul(1000))
        .ok_or(StoreError::SequenceOverflow)?;
    unix_time_ms()?
        .checked_add(ttl_ms)
        .ok_or(StoreError::SequenceOverflow)
}

fn unix_time_ms() -> Result<i64> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| StoreError::Event("system time before unix epoch".to_owned()))?;
    i64::try_from(duration.as_millis()).map_err(|_| StoreError::SequenceOverflow)
}

impl RunMemoryCore {
    fn projection_snapshot(&self) -> &ProjectionSnapshot {
        &self.projections
    }

    fn append_prepared_commit_bundle(
        &mut self,
        bundle: PreparedCommitBundle,
    ) -> Result<CommitOutcome> {
        let plan = bundle.plan();
        let request = plan.request();
        let fingerprint = prepared_commit_plan_fingerprint(plan)?;

        if let Some(record) = self
            .commit_keys
            .get(&(request.run_id.clone(), request.commit_key.clone()))
        {
            if record.fingerprint == fingerprint {
                return Ok(CommitOutcome::Idempotent(record.batch.clone()));
            }
            return Err(StoreError::CommitConflict {
                commit_key: request.commit_key.clone(),
            });
        }

        let mut artifacts = self.artifacts.clone();
        verify_existing_artifact_admissions(&artifacts, &bundle)?;
        admit_artifact_evidence(&mut artifacts, bundle.admitted_artifacts())?;
        let artifact_bytes = artifact_byte_authority_for_bundle(&self.artifact_bytes, &bundle)?;
        let base = CommitBase {
            artifacts,
            artifact_bytes: artifact_bytes.clone(),
            logical_keys: self.logical_keys.clone(),
            unique_logical_payloads: self.unique_logical_payloads.clone(),
            projections: self.projections.clone(),
            resource_lane_authority: self.resource_lane_authority.clone(),
            actual_next_seq: self.expected_next_seq(&request.run_id),
            store_commit_order: self.next_store_commit_order,
        };
        let staged = match stage_prepared_commit_plan(&base, plan)? {
            StagedCommitOutcome::Staged(staged) => *staged,
            StagedCommitOutcome::AdmissionBlocked(block) => {
                return Ok(CommitOutcome::AdmissionBlocked(block));
            }
        };
        if let Some(claim) = bundle.execution_claim() {
            match self.acquire_execution_claim(
                &claim.scope,
                &claim.holder_run_id,
                claim.token.clone(),
            )? {
                NowaitSkipAdmissionResult::Admitted(_) => {}
                NowaitSkipAdmissionResult::Busy(busy) => {
                    return Ok(CommitOutcome::ExecutionClaimBusy(Box::new(busy)));
                }
            }
        }
        let (
            batch,
            staged_logical_keys,
            staged_unique_payloads,
            staged_projections,
            staged_resource_lane_authority,
        ) = staged.into_parts();
        self.next_store_commit_order = batch
            .store_commit_order()
            .checked_next()
            .ok_or(StoreError::SequenceOverflow)?;
        self.artifact_bytes = artifact_bytes;
        self.streams
            .entry(request.run_id.clone())
            .or_default()
            .push(batch.clone());
        self.commit_keys.insert(
            (request.run_id.clone(), request.commit_key.clone()),
            CommitKeyRecord {
                fingerprint,
                batch: batch.clone(),
            },
        );
        self.artifacts = base.artifacts;
        self.logical_keys = staged_logical_keys;
        self.unique_logical_payloads = staged_unique_payloads;
        self.projections = staged_projections;
        self.resource_lane_authority = staged_resource_lane_authority;
        Ok(CommitOutcome::Appended(batch))
    }

    fn load_run_stream(&self, run_id: &RunId) -> Vec<KernelEventEnvelope> {
        self.stream_events(run_id)
    }

    fn expected_next_seq(&self, run_id: &RunId) -> StreamSeq {
        let Some(batches) = self.streams.get(run_id) else {
            return StreamSeq::FIRST;
        };
        batches
            .last()
            .map(|batch| batch.seq.checked_next().unwrap_or(StreamSeq::MAX))
            .unwrap_or(StreamSeq::FIRST)
    }
}

/// Non-durable async wrapper around a private in-memory run store core.
///
/// This helper is compiled only for tests or the `test-support` feature. It must not be used as a
/// production persistence store.
#[derive(Clone, Default)]
pub struct AsyncInMemoryRunStore {
    inner: Arc<Mutex<RunMemoryCore>>,
    fact_query_snapshot_hook: FactQuerySnapshotHook,
}

#[derive(Clone, Default)]
struct FactQuerySnapshotHook {
    callback: Arc<Mutex<Option<Box<dyn FnOnce() + Send + 'static>>>>,
}

impl std::fmt::Debug for FactQuerySnapshotHook {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("FactQuerySnapshotHook")
    }
}

impl std::fmt::Debug for AsyncInMemoryRunStore {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AsyncInMemoryRunStore")
            .finish_non_exhaustive()
    }
}

impl AsyncInMemoryRunStore {
    /// Creates an empty async in-memory typed run store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Installs a one-shot synchronization hook after the next fact-query snapshot is fixed.
    ///
    /// This test-only seam executes while the store lock is held, allowing concurrency tests to
    /// prove that an overlapping append cannot partially enter the returned snapshot.
    #[doc(hidden)]
    pub fn set_fact_query_snapshot_hook_for_test(
        &self,
        callback: impl FnOnce() + Send + 'static,
    ) -> Result<()> {
        let mut hook =
            self.fact_query_snapshot_hook.callback.lock().map_err(|_| {
                StoreError::Event("fact-query snapshot hook lock poisoned".to_owned())
            })?;
        *hook = Some(Box::new(callback));
        Ok(())
    }

    /// Returns the current in-memory projection snapshot.
    pub fn projection_snapshot(&self) -> Result<ProjectionSnapshot> {
        self.lock_inner()
            .map(|store| store.projection_snapshot().clone())
    }

    /// Seeds exact artifact evidence for tests that need pre-existing artifact authority.
    pub fn seed_artifact_evidence_for_test(
        &self,
        admitted_artifacts: &[ArtifactEvidenceRef],
    ) -> Result<()> {
        self.lock_inner()
            .and_then(|mut store| admit_artifact_evidence(&mut store.artifacts, admitted_artifacts))
    }

    /// Seeds a projection snapshot and exact retained artifact bytes for tests.
    pub fn seed_projection_snapshot_for_test(
        &self,
        projection: ProjectionSnapshot,
        retained_artifacts: impl IntoIterator<Item = VerifiedRetainedArtifactBytes>,
    ) -> Result<()> {
        let mut store = self.lock_inner()?;
        store.projections = projection;
        for artifact in retained_artifacts {
            let evidence = artifact.evidence().clone();
            let evidence_hash = evidence.evidence_hash()?;
            store.artifact_bytes.insert(
                (evidence.artifact_id.clone(), evidence_hash),
                (artifact.into_bytes(), evidence),
            );
        }
        Ok(())
    }

    /// Injects pre-built envelopes into run streams without re-applying projections.
    ///
    /// Used by Platform holding seed helpers so `verify_replay` can load cross-run
    /// `FactRecorded` source events for fact-query evidence, while live SelectHoldings
    /// continues to use the already-merged projection authority.
    pub fn seed_run_stream_envelopes_for_test(
        &self,
        envelopes: impl IntoIterator<Item = KernelEventEnvelope>,
    ) -> Result<()> {
        let mut store = self.lock_inner()?;
        let mut by_run: BTreeMap<RunId, Vec<KernelEventEnvelope>> = BTreeMap::new();
        for envelope in envelopes {
            by_run
                .entry(envelope.run_id().clone())
                .or_default()
                .push(envelope);
        }
        for (run_id, mut events) in by_run {
            events.sort_by_key(|event| (event.seq().as_u64(), event.ordinal().as_u32()));
            let Some(first) = events.first() else {
                continue;
            };
            let seq = first.seq();
            let commit_key = first.commit_key().clone();
            let fingerprint = CommitFingerprint::from_digest(first.payload_hash().clone());
            let batch = CommittedBatch::from_persisted_events(
                run_id.clone(),
                commit_key,
                fingerprint,
                seq,
                first.store_commit_order(),
                events,
            )?;
            store.next_store_commit_order = store.next_store_commit_order.max(
                batch
                    .store_commit_order()
                    .checked_next()
                    .ok_or(StoreError::SequenceOverflow)?,
            );
            store.streams.entry(run_id).or_default().push(batch);
        }
        Ok(())
    }

    /// Marks an execution claim expired for tests that need stale-claim recovery without sleeping.
    pub fn expire_execution_claim_for_test(&self, run_id: &RunId) -> Result<bool> {
        let mut store = self.lock_inner()?;
        let Some(holder) = store
            .execution_claims
            .values_mut()
            .find(|holder| &holder.holder_run_id == run_id)
        else {
            return Ok(false);
        };
        holder.lease_expires_at_unix_ms = unix_time_ms()?.saturating_sub(1);
        Ok(true)
    }

    fn lock_inner(&self) -> Result<MutexGuard<'_, RunMemoryCore>> {
        self.inner
            .lock()
            .map_err(|_| StoreError::Event("async in-memory run store lock poisoned".to_owned()))
    }

    fn run_fact_query_snapshot_hook(&self) -> Result<()> {
        let callback = self
            .fact_query_snapshot_hook
            .callback
            .lock()
            .map_err(|_| StoreError::Event("fact-query snapshot hook lock poisoned".to_owned()))?
            .take();
        if let Some(callback) = callback {
            callback();
        }
        Ok(())
    }
}

impl RunEventStore for AsyncInMemoryRunStore {
    type Error = StoreError;

    fn append_prepared_commit_bundle<'a>(
        &'a self,
        bundle: PreparedCommitBundle,
    ) -> AsyncStoreFuture<'a, CommitOutcome, Self::Error> {
        let result = self
            .lock_inner()
            .and_then(|mut store| store.append_prepared_commit_bundle(bundle));
        Box::pin(std::future::ready(result))
    }

    fn load_run_stream<'a>(
        &'a self,
        run_id: &'a RunId,
    ) -> AsyncStoreFuture<'a, Vec<KernelEventEnvelope>, Self::Error> {
        let result = self.lock_inner().map(|store| store.load_run_stream(run_id));
        Box::pin(std::future::ready(result))
    }

    fn load_committed_run_stream<'a>(
        &'a self,
        run_id: &'a RunId,
    ) -> AsyncStoreFuture<'a, CommittedRunStream, Self::Error> {
        let result = self.lock_inner().and_then(|store| {
            CommittedRunStream::from_events_with_artifact_bytes(
                run_id.clone(),
                store.load_run_stream(run_id),
                &store.artifact_bytes,
            )
        });
        Box::pin(std::future::ready(result))
    }

    fn expected_next_seq<'a>(
        &'a self,
        run_id: &'a RunId,
    ) -> AsyncStoreFuture<'a, StreamSeq, Self::Error> {
        let result = self
            .lock_inner()
            .map(|store| store.expected_next_seq(run_id));
        Box::pin(std::future::ready(result))
    }

    fn status_projection_snapshot<'a>(
        &'a self,
        run_id: &'a RunId,
    ) -> AsyncStoreFuture<'a, ProjectionSnapshot, Self::Error> {
        let result = self
            .lock_inner()
            .map(|store| {
                let stream = store.load_run_stream(run_id);
                let committed = CommittedRunStream::from_events_with_artifact_bytes(
                    run_id.clone(),
                    stream,
                    &store.artifact_bytes,
                )?;
                let run_projection = committed.projection().clone();
                projection_with_store_authority(&run_projection, store.projection_snapshot())
            })
            .and_then(|result| result);
        Box::pin(std::future::ready(result))
    }

    fn fact_projection_snapshot<'a>(
        &'a self,
    ) -> AsyncStoreFuture<'a, ProjectionSnapshot, Self::Error> {
        let result = self
            .lock_inner()
            .map(|store| store.projection_snapshot().clone());
        Box::pin(std::future::ready(result))
    }
}

impl StoreScopeStore for AsyncInMemoryRunStore {
    type Error = StoreError;

    fn load_store_scope_id<'a>(&'a self) -> AsyncStoreFuture<'a, StoreScopeId, Self::Error> {
        let result = self.lock_inner().map(|store| store.store_scope_id.clone());
        Box::pin(std::future::ready(result))
    }
}

impl FactQueryStore for AsyncInMemoryRunStore {
    type Error = StoreError;

    fn fact_query_implementation_id(&self) -> &'static str {
        "mfm.store.memory.fact-query.v1"
    }

    fn execute_fact_queries<'a>(
        &'a self,
        plans: &'a [mfm_facts::CanonicalFactQueryPlan],
    ) -> Pin<
        Box<
            dyn Future<Output = std::result::Result<Vec<mfm_facts::FactQueryResult>, Self::Error>>
                + Send
                + 'a,
        >,
    > {
        if plans.is_empty() {
            return Box::pin(std::future::ready(Ok(Vec::new())));
        }
        Box::pin(async move {
            let store = self.lock_inner()?;
            let frontier = mfm_facts::StoreReadFrontier::new(
                store.store_scope_id.clone(),
                StoreCommitOrder::new(store.next_store_commit_order.as_u64().saturating_sub(1)),
            );
            self.run_fact_query_snapshot_hook()?;
            plans
                .iter()
                .map(|plan| {
                    super::fact_query::execute_fact_query_projection_result(
                        &store.projections,
                        frontier.clone(),
                        plan,
                    )
                })
                .collect()
        })
    }
}

impl ExecutionClaimStore for AsyncInMemoryRunStore {
    type Error = StoreError;

    fn acquire_execution_claim<'a>(
        &'a self,
        scope: &'a ExecutionClaimScope,
        holder_run_id: &'a RunId,
        token: AdmissionToken,
    ) -> AsyncStoreFuture<'a, NowaitSkipAdmissionResult, Self::Error> {
        let result = self
            .lock_inner()
            .and_then(|mut store| store.acquire_execution_claim(scope, holder_run_id, token));
        Box::pin(std::future::ready(result))
    }

    fn execution_claim_status<'a>(
        &'a self,
        scope: &'a ExecutionClaimScope,
    ) -> AsyncStoreFuture<'a, ExecutionClaimStatus, Self::Error> {
        let result = self
            .lock_inner()
            .and_then(|store| store.execution_claim_status(scope));
        Box::pin(std::future::ready(result))
    }

    fn renew_execution_claim<'a>(
        &'a self,
        scope: &'a ExecutionClaimScope,
        holder_run_id: &'a RunId,
        token: &'a AdmissionToken,
    ) -> AsyncStoreFuture<'a, Option<AdmissionLease>, Self::Error> {
        let result = self
            .lock_inner()
            .and_then(|mut store| store.renew_execution_claim(scope, holder_run_id, token));
        Box::pin(std::future::ready(result))
    }

    fn release_execution_claim<'a>(
        &'a self,
        scope: &'a ExecutionClaimScope,
        holder_run_id: &'a RunId,
        token: &'a AdmissionToken,
    ) -> AsyncStoreFuture<'a, bool, Self::Error> {
        let result = self
            .lock_inner()
            .and_then(|mut store| store.release_execution_claim(scope, holder_run_id, token));
        Box::pin(std::future::ready(result))
    }

    fn expired_execution_claims<'a>(
        &'a self,
    ) -> AsyncStoreFuture<'a, Vec<ExpiredExecutionClaim>, Self::Error> {
        let result = self
            .lock_inner()
            .and_then(|store| store.expired_execution_claims());
        Box::pin(std::future::ready(result))
    }

    fn reap_expired_execution_claim<'a>(
        &'a self,
        scope: &'a ExecutionClaimScope,
        holder_run_id: &'a RunId,
        token: &'a AdmissionToken,
    ) -> AsyncStoreFuture<'a, bool, Self::Error> {
        let result = self
            .lock_inner()
            .and_then(|mut store| store.reap_expired_execution_claim(scope, holder_run_id, token));
        Box::pin(std::future::ready(result))
    }
}

impl RunObservationStore for AsyncInMemoryRunStore {
    type Error = StoreError;

    fn read_run_observations<'a>(
        &'a self,
        query: RunObservationQuery,
    ) -> AsyncStoreFuture<'a, RunObservationPage, Self::Error> {
        let result = self.lock_inner().and_then(|store| {
            const MAX_LIMIT: u32 = 100;
            if query.limit == 0 || query.limit > MAX_LIMIT {
                return Err(StoreError::LimitOutOfRange {
                    limit: query.limit,
                    max: MAX_LIMIT,
                });
            }
            let mut commits = Vec::<(&RunId, &CommittedBatch)>::new();
            for (run_id, batches) in &store.streams {
                for batch in batches {
                    commits.push((run_id, batch));
                }
            }
            commits.sort_by(|left, right| {
                left.1
                    .seq
                    .cmp(&right.1.seq)
                    .then_with(|| left.0.cmp(right.0))
            });
            let total_commits = commits.len();
            let timestamp = "1970-01-01T00:00:00.000000Z".to_owned();
            let rows = if let Some(cursor) = query.cursor.as_deref() {
                let start = parse_memory_observation_cursor(cursor)?;
                commits
                    .into_iter()
                    .enumerate()
                    .skip(start)
                    .take(query.limit as usize)
                    .map(|(index, (run_id, batch))| {
                        memory_observation_row(
                            &store.projections,
                            run_id,
                            batch.seq,
                            timestamp.clone(),
                            Some(format!("memory:{index}")),
                        )
                    })
                    .collect::<Result<Vec<_>>>()?
            } else {
                store
                    .streams
                    .iter()
                    .filter_map(|(run_id, batches)| {
                        batches.last().map(|batch| (run_id.clone(), batch.seq))
                    })
                    .take(query.limit as usize)
                    .map(|(run_id, seq)| {
                        memory_observation_row(
                            &store.projections,
                            &run_id,
                            seq,
                            timestamp.clone(),
                            None,
                        )
                    })
                    .collect::<Result<Vec<_>>>()?
            };
            Ok(RunObservationPage {
                next_cursor: format!("memory:{total_commits}"),
                runs: rows,
            })
        });
        Box::pin(std::future::ready(result))
    }
}

fn parse_memory_observation_cursor(cursor: &str) -> Result<usize> {
    let Some(value) = cursor.strip_prefix("memory:") else {
        return Err(StoreError::InvalidCursor {
            message: "in-memory observation cursor has an unsupported format".to_owned(),
        });
    };
    value
        .parse::<usize>()
        .map_err(|_| StoreError::InvalidCursor {
            message: "in-memory observation cursor has an invalid position".to_owned(),
        })
}

fn memory_observation_row(
    projections: &ProjectionSnapshot,
    run_id: &RunId,
    head_seq: StreamSeq,
    timestamp: String,
    change_id: Option<String>,
) -> Result<RunObservation> {
    let observed_status = match projections.run_state(run_id) {
        RunState::Started => ObservedRunStatus::Started,
        RunState::Completed => ObservedRunStatus::Completed,
        RunState::Absent => {
            return Err(StoreError::ObservationUnavailable {
                message: "in-memory observation had absent run state".to_owned(),
            })
        }
    };
    Ok(RunObservation {
        run_id: run_id.clone(),
        head_seq,
        observed_status,
        started_at: timestamp.clone(),
        updated_at: timestamp.clone(),
        completed_at: (observed_status == ObservedRunStatus::Completed).then_some(timestamp),
        change_id,
    })
}

impl RetainedArtifactReadProvider for AsyncInMemoryRunStore {
    fn read_retained_artifact<'a>(
        &'a self,
        requirement: &'a EventArtifactRequirement,
    ) -> RetainedArtifactReadFuture<'a> {
        let result = self.lock_inner().and_then(|store| {
            let key = (
                requirement.artifact_id.clone(),
                requirement.evidence_hash.clone(),
            );
            let Some((bytes, evidence)) = store.artifact_bytes.get(&key) else {
                return Err(StoreError::MissingArtifact {
                    artifact_id: requirement.artifact_id.clone(),
                });
            };
            VerifiedRetainedArtifactBytes::new(bytes.clone(), evidence.clone(), requirement)
        });
        Box::pin(std::future::ready(result))
    }
}

fn projection_with_store_authority(
    snapshot: &ProjectionSnapshot,
    authority: &ProjectionSnapshot,
) -> Result<ProjectionSnapshot> {
    snapshot.with_store_authority_from(authority)
}
