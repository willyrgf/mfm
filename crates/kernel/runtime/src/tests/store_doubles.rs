use super::*;

pub(super) trait TestPreparedCommitExt {
    fn append_prepared_commit(
        &mut self,
        request: store::CommitRequest,
    ) -> store::Result<store::CommitOutcome>;
}

impl TestPreparedCommitExt for TestTypedRunStore {
    fn append_prepared_commit(
        &mut self,
        request: store::CommitRequest,
    ) -> store::Result<store::CommitOutcome> {
        let admitted_artifacts = request.required_artifacts().to_vec();
        let plan = test_prepared_commit_plan(request, admitted_artifacts)?;
        self.append_test_commit_plan(plan)
    }
}
#[derive(Clone, Default)]
pub(super) struct RunnerKitArtifactProvider {
    artifacts: BTreeMap<store::ArtifactAuthorityKey, (Vec<u8>, store::ArtifactEvidenceRef)>,
}

impl RunnerKitArtifactProvider {
    pub(super) fn new(artifacts: Vec<(Vec<u8>, store::ArtifactEvidenceRef)>) -> Self {
        Self {
            artifacts: artifacts
                .into_iter()
                .map(|(bytes, evidence)| {
                    (
                        test_artifact_authority_key(&evidence)
                            .expect("test artifact evidence hash"),
                        (bytes, evidence),
                    )
                })
                .collect(),
        }
    }
}

impl store::RetainedArtifactReadProvider for RunnerKitArtifactProvider {
    fn read_retained_artifact<'a>(
        &'a self,
        requirement: &'a store::EventArtifactRequirement,
    ) -> store::RetainedArtifactReadFuture<'a> {
        Box::pin(async move {
            let key = (
                requirement.artifact_id.clone(),
                requirement.evidence_hash.clone(),
            );
            let Some((bytes, evidence)) = self.artifacts.get(&key) else {
                return Err(store::StoreError::MissingArtifact {
                    artifact_id: requirement.artifact_id.clone(),
                });
            };
            store::VerifiedRetainedArtifactBytes::new(bytes.clone(), evidence.clone(), requirement)
        })
    }
}

fn test_artifact_authority_key(
    evidence: &store::ArtifactEvidenceRef,
) -> store::Result<store::ArtifactAuthorityKey> {
    Ok((evidence.artifact_id.clone(), evidence.evidence_hash()?))
}
#[derive(Clone, Default)]
pub(super) struct TestTypedRunStore {
    inner: store::AsyncInMemoryRunStore,
}

impl TestTypedRunStore {
    pub(super) fn new() -> Self {
        Self::default()
    }

    fn append_test_commit_plan(
        &self,
        plan: store::PreparedCommitPlan,
    ) -> store::Result<store::CommitOutcome> {
        self.inner
            .seed_artifact_evidence_for_test(plan.admitted_artifacts())?;
        let bundle = test_bundle_from_plan(plan)?;
        block_on_ready(self.inner.append_prepared_commit_bundle(bundle))
    }

    pub(super) fn load_run_stream(&self, run_id: &RunId) -> Vec<store::KernelEventEnvelope> {
        block_on_ready(self.inner.load_run_stream(run_id)).expect("test store read")
    }

    pub(super) fn run_admitted(&self, run_id: &RunId) -> Box<events::RunAdmitted> {
        self.load_run_stream(run_id)
            .into_iter()
            .find_map(|event| match event.payload().clone() {
                events::KernelEventPayload::RunAdmitted(payload) => Some(payload),
                _ => None,
            })
            .expect("RunAdmitted payload")
    }

    pub(super) fn assert_run_stream_len(&self, run_id: &RunId, expected: usize) {
        assert_eq!(self.load_run_stream(run_id).len(), expected);
    }

    pub(super) fn expected_next_seq(&self, run_id: &RunId) -> store::StreamSeq {
        block_on_ready(self.inner.expected_next_seq(run_id)).expect("test store next seq")
    }

    pub(super) fn projection_snapshot(&self) -> store::ProjectionSnapshot {
        self.inner
            .projection_snapshot()
            .expect("test store projection")
    }
}

impl store::RunEventStore for TestTypedRunStore {
    type Error = store::StoreError;

    fn append_prepared_commit_bundle<'a>(
        &'a self,
        bundle: store::PreparedCommitBundle,
    ) -> store::AsyncStoreFuture<'a, store::CommitOutcome, Self::Error> {
        Box::pin(async move {
            self.inner
                .seed_artifact_evidence_for_test(bundle.admitted_artifacts())?;
            self.inner.append_prepared_commit_bundle(bundle).await
        })
    }

    fn load_run_stream<'a>(
        &'a self,
        run_id: &'a RunId,
    ) -> store::AsyncStoreFuture<'a, Vec<store::KernelEventEnvelope>, Self::Error> {
        self.inner.load_run_stream(run_id)
    }

    fn load_committed_run_stream<'a>(
        &'a self,
        run_id: &'a RunId,
    ) -> store::AsyncStoreFuture<'a, store::CommittedRunStream, Self::Error> {
        self.inner.load_committed_run_stream(run_id)
    }

    fn expected_next_seq<'a>(
        &'a self,
        run_id: &'a RunId,
    ) -> store::AsyncStoreFuture<'a, store::StreamSeq, Self::Error> {
        self.inner.expected_next_seq(run_id)
    }

    fn status_projection_snapshot<'a>(
        &'a self,
        run_id: &'a RunId,
    ) -> store::AsyncStoreFuture<'a, store::ProjectionSnapshot, Self::Error> {
        self.inner.status_projection_snapshot(run_id)
    }

    fn fact_projection_snapshot<'a>(
        &'a self,
    ) -> store::AsyncStoreFuture<'a, store::ProjectionSnapshot, Self::Error> {
        self.inner.fact_projection_snapshot()
    }
}

delegate_execution_claim_store!(TestTypedRunStore, delegate_execution_claim_direct);

impl store::RetainedArtifactReadProvider for TestTypedRunStore {
    fn read_retained_artifact<'a>(
        &'a self,
        requirement: &'a store::EventArtifactRequirement,
    ) -> store::RetainedArtifactReadFuture<'a> {
        self.inner.read_retained_artifact(requirement)
    }
}
#[derive(Clone)]
pub(super) struct RecordedPreparedCommit {
    pub(super) seq: store::StreamSeq,
    pub(super) commit_key: store::CommitKey,
    pub(super) payloads: Vec<events::KernelEventPayload>,
    pub(super) admitted_artifacts: Vec<store::ArtifactEvidenceRef>,
}

pub(super) struct RecordingTypedRunStore {
    inner: store::AsyncInMemoryRunStore,
    commits: Arc<Mutex<Vec<RecordedPreparedCommit>>>,
}

impl RecordingTypedRunStore {
    pub(super) fn new() -> Self {
        Self {
            inner: store::AsyncInMemoryRunStore::new(),
            commits: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub(super) fn commits(&self) -> Vec<RecordedPreparedCommit> {
        self.commits
            .lock()
            .expect("recording store commits lock")
            .clone()
    }

    pub(super) async fn load_run_stream(&self, run_id: &RunId) -> Vec<store::KernelEventEnvelope> {
        self.inner
            .load_run_stream(run_id)
            .await
            .expect("recording store read")
    }

    pub(super) async fn projection_snapshot(&self, run_id: &RunId) -> store::ProjectionSnapshot {
        self.inner
            .status_projection_snapshot(run_id)
            .await
            .expect("recording store projection")
    }
}
pub(super) struct StaleOnceTypedRunStore {
    inner: store::AsyncInMemoryRunStore,
    stale_injected: Mutex<bool>,
    target: StaleAppendTarget,
}

#[derive(Clone, Copy)]
enum StaleAppendTarget {
    Terminal,
    SideEffectPreparation,
}

impl StaleOnceTypedRunStore {
    pub(super) fn new() -> Self {
        Self {
            inner: store::AsyncInMemoryRunStore::new(),
            stale_injected: Mutex::new(false),
            target: StaleAppendTarget::Terminal,
        }
    }

    pub(super) fn for_side_effect_preparation() -> Self {
        Self {
            inner: store::AsyncInMemoryRunStore::new(),
            stale_injected: Mutex::new(false),
            target: StaleAppendTarget::SideEffectPreparation,
        }
    }

    pub(super) async fn projection_snapshot(&self, run_id: &RunId) -> store::ProjectionSnapshot {
        self.inner
            .status_projection_snapshot(run_id)
            .await
            .expect("stale-once store projection")
    }
}

impl store::RunEventStore for RecordingTypedRunStore {
    type Error = store::StoreError;

    fn append_prepared_commit_bundle<'a>(
        &'a self,
        bundle: store::PreparedCommitBundle,
    ) -> store::AsyncStoreFuture<'a, store::CommitOutcome, Self::Error> {
        let payloads = bundle.request().payloads().to_vec();
        let admitted_artifacts = bundle.admitted_artifacts().to_vec();
        Box::pin(async move {
            self.inner
                .seed_artifact_evidence_for_test(bundle.admitted_artifacts())?;
            let outcome = self.inner.append_prepared_commit_bundle(bundle).await?;
            if let store::CommitOutcome::Appended(batch) = &outcome {
                self.commits
                    .lock()
                    .map_err(|_| {
                        store::StoreError::Event("recording store lock poisoned".to_owned())
                    })?
                    .push(RecordedPreparedCommit {
                        seq: batch.seq(),
                        commit_key: batch.commit_key().clone(),
                        payloads,
                        admitted_artifacts,
                    });
            }
            Ok(outcome)
        })
    }

    fn load_run_stream<'a>(
        &'a self,
        run_id: &'a RunId,
    ) -> store::AsyncStoreFuture<'a, Vec<store::KernelEventEnvelope>, Self::Error> {
        self.inner.load_run_stream(run_id)
    }

    fn load_committed_run_stream<'a>(
        &'a self,
        run_id: &'a RunId,
    ) -> store::AsyncStoreFuture<'a, store::CommittedRunStream, Self::Error> {
        self.inner.load_committed_run_stream(run_id)
    }

    fn expected_next_seq<'a>(
        &'a self,
        run_id: &'a RunId,
    ) -> store::AsyncStoreFuture<'a, store::StreamSeq, Self::Error> {
        self.inner.expected_next_seq(run_id)
    }

    fn status_projection_snapshot<'a>(
        &'a self,
        run_id: &'a RunId,
    ) -> store::AsyncStoreFuture<'a, store::ProjectionSnapshot, Self::Error> {
        self.inner.status_projection_snapshot(run_id)
    }

    fn fact_projection_snapshot<'a>(
        &'a self,
    ) -> store::AsyncStoreFuture<'a, store::ProjectionSnapshot, Self::Error> {
        self.inner.fact_projection_snapshot()
    }
}

delegate_execution_claim_store!(RecordingTypedRunStore, delegate_execution_claim_direct);

impl store::RunEventStore for StaleOnceTypedRunStore {
    type Error = store::StoreError;

    fn append_prepared_commit_bundle<'a>(
        &'a self,
        bundle: store::PreparedCommitBundle,
    ) -> store::AsyncStoreFuture<'a, store::CommitOutcome, Self::Error> {
        Box::pin(async move {
            let is_run_start = bundle
                .request()
                .payloads()
                .iter()
                .any(|payload| matches!(payload, events::KernelEventPayload::RunAdmitted(_)));
            let has_target = bundle
                .request()
                .payloads()
                .iter()
                .any(|payload| match self.target {
                    StaleAppendTarget::Terminal => matches!(
                        payload,
                        events::KernelEventPayload::StateAttemptCompleted(_)
                            | events::KernelEventPayload::StateAttemptFailed(_)
                            | events::KernelEventPayload::StateAttemptInterrupted(_)
                    ),
                    StaleAppendTarget::SideEffectPreparation => matches!(
                        payload,
                        events::KernelEventPayload::SideEffectInvocationPrepared(_)
                    ),
                });
            let should_inject = {
                let mut injected = self.stale_injected.lock().map_err(|_| {
                    store::StoreError::Event("stale-once store lock poisoned".to_owned())
                })?;
                let should_inject = !*injected && !is_run_start && has_target;
                if should_inject {
                    *injected = true;
                }
                should_inject
            };
            if should_inject {
                let expected = bundle.request().expected_next_seq();
                let run_id = bundle.request().run_id().clone();
                self.inner
                    .seed_artifact_evidence_for_test(bundle.admitted_artifacts())?;
                self.inner.append_prepared_commit_bundle(bundle).await?;
                return Err(store::StoreError::StaleExpectedNextSeq {
                    expected,
                    actual: self.inner.expected_next_seq(&run_id).await?,
                });
            }
            self.inner
                .seed_artifact_evidence_for_test(bundle.admitted_artifacts())?;
            self.inner.append_prepared_commit_bundle(bundle).await
        })
    }

    fn load_run_stream<'a>(
        &'a self,
        run_id: &'a RunId,
    ) -> store::AsyncStoreFuture<'a, Vec<store::KernelEventEnvelope>, Self::Error> {
        self.inner.load_run_stream(run_id)
    }

    fn load_committed_run_stream<'a>(
        &'a self,
        run_id: &'a RunId,
    ) -> store::AsyncStoreFuture<'a, store::CommittedRunStream, Self::Error> {
        self.inner.load_committed_run_stream(run_id)
    }

    fn expected_next_seq<'a>(
        &'a self,
        run_id: &'a RunId,
    ) -> store::AsyncStoreFuture<'a, store::StreamSeq, Self::Error> {
        self.inner.expected_next_seq(run_id)
    }

    fn status_projection_snapshot<'a>(
        &'a self,
        run_id: &'a RunId,
    ) -> store::AsyncStoreFuture<'a, store::ProjectionSnapshot, Self::Error> {
        self.inner.status_projection_snapshot(run_id)
    }

    fn fact_projection_snapshot<'a>(
        &'a self,
    ) -> store::AsyncStoreFuture<'a, store::ProjectionSnapshot, Self::Error> {
        self.inner.fact_projection_snapshot()
    }
}

delegate_execution_claim_store!(StaleOnceTypedRunStore, delegate_execution_claim_direct);
pub(super) type TestArtifactMap =
    BTreeMap<store::ArtifactAuthorityKey, (Vec<u8>, store::ArtifactEvidenceRef)>;

#[derive(Clone, Default)]
pub(super) struct TestRetainedArtifactStore {
    pub(super) artifacts: Arc<Mutex<TestArtifactMap>>,
}

impl store::RetainedArtifactReadProvider for TestRetainedArtifactStore {
    fn read_retained_artifact<'a>(
        &'a self,
        requirement: &'a store::EventArtifactRequirement,
    ) -> store::RetainedArtifactReadFuture<'a> {
        Box::pin(async move {
            let key = (
                requirement.artifact_id.clone(),
                requirement.evidence_hash.clone(),
            );
            let (bytes, evidence) = self
                .artifacts
                .lock()
                .map_err(|_| store::StoreError::ArtifactReadFailed {
                    artifact_id: requirement.artifact_id.clone(),
                })?
                .get(&key)
                .cloned()
                .ok_or_else(|| store::StoreError::MissingArtifact {
                    artifact_id: requirement.artifact_id.clone(),
                })?;
            store::VerifiedRetainedArtifactBytes::new(bytes, evidence, requirement)
        })
    }
}

#[derive(Clone)]
pub(super) struct FilteringRetainedArtifactStore {
    source: TestTypedRunStore,
    missing_artifacts: Arc<Mutex<BTreeSet<ArtifactId>>>,
}

impl FilteringRetainedArtifactStore {
    pub(super) fn new(source: TestTypedRunStore) -> Self {
        Self {
            source,
            missing_artifacts: Arc::new(Mutex::new(BTreeSet::new())),
        }
    }

    pub(super) fn hide_artifact(&self, artifact_id: ArtifactId) {
        self.missing_artifacts
            .lock()
            .expect("filtering artifact store")
            .insert(artifact_id);
    }
}

impl store::RetainedArtifactReadProvider for FilteringRetainedArtifactStore {
    fn read_retained_artifact<'a>(
        &'a self,
        requirement: &'a store::EventArtifactRequirement,
    ) -> store::RetainedArtifactReadFuture<'a> {
        Box::pin(async move {
            if self
                .missing_artifacts
                .lock()
                .map_err(|_| store::StoreError::ArtifactReadFailed {
                    artifact_id: requirement.artifact_id.clone(),
                })?
                .contains(&requirement.artifact_id)
            {
                return Err(store::StoreError::MissingArtifact {
                    artifact_id: requirement.artifact_id.clone(),
                });
            }
            self.source.read_retained_artifact(requirement).await
        })
    }
}
