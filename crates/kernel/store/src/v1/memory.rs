use std::sync::{Arc, Mutex, MutexGuard};

use super::*;

/// Non-durable async wrapper around a private in-memory run store core.
///
/// This helper is compiled only for tests or the `test-support` feature. It must not be used as a
/// production persistence store.
#[derive(Debug, Clone, Default)]
pub struct AsyncInMemoryRunStore {
    inner: Arc<Mutex<RunMemoryCore>>,
}

impl AsyncInMemoryRunStore {
    /// Creates an empty async in-memory typed run store.
    pub fn new() -> Self {
        Self::default()
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
        retained_artifacts: impl IntoIterator<Item = VerifiedRunArtifactBytes>,
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
            let fingerprint = CommitFingerprint(first.payload_hash().clone());
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
            VerifiedRunArtifactBytes::new(bytes.clone(), evidence.clone(), requirement)
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
