use super::*;

pub(super) trait TestPreparedCommitExt {
    fn append_prepared_commit(
        &mut self,
        request: store::CommitRequest,
    ) -> store::Result<store::CommitOutcome>;

    fn append_prepared_commit_with_artifacts(
        &mut self,
        request: store::CommitRequest,
        artifacts: Vec<(Vec<u8>, store::ArtifactEvidenceRef)>,
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

    fn append_prepared_commit_with_artifacts(
        &mut self,
        request: store::CommitRequest,
        artifacts: Vec<(Vec<u8>, store::ArtifactEvidenceRef)>,
    ) -> store::Result<store::CommitOutcome> {
        let admitted_artifacts = request.required_artifacts().to_vec();
        let plan = test_prepared_commit_plan(request, admitted_artifacts)?;
        let mut artifact_bytes = Vec::with_capacity(artifacts.len());
        let mut staged_keys = BTreeSet::new();
        for (bytes, evidence) in artifacts {
            staged_keys.insert((evidence.artifact_id.clone(), evidence.evidence_hash()?));
            artifact_bytes.push(store::PreparedArtifactBytes::new(bytes, evidence)?);
        }
        let existing = plan
            .admitted_artifacts()
            .iter()
            .filter_map(|evidence| {
                let evidence_hash = evidence.evidence_hash().ok()?;
                (!staged_keys.contains(&(evidence.artifact_id.clone(), evidence_hash.clone())))
                    .then(|| {
                        store::ExistingArtifactAdmission::new(
                            evidence.artifact_id.clone(),
                            evidence_hash,
                        )
                    })
            })
            .collect::<Vec<_>>();
        self.inner.seed_artifact_evidence_for_test(
            plan.admitted_artifacts()
                .iter()
                .filter(|evidence| {
                    evidence.evidence_hash().is_ok_and(|hash| {
                        !staged_keys.contains(&(evidence.artifact_id.clone(), hash))
                    })
                })
                .cloned()
                .collect::<Vec<_>>()
                .as_slice(),
        )?;
        let bundle = store::PreparedCommitBundle::new(plan, artifact_bytes, existing)?;
        block_on_ready(self.inner.append_prepared_commit_bundle(bundle))
    }
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

    pub(super) fn expected_next_seq(&self, run_id: &RunId) -> store::StreamSeq {
        self.inner
            .expected_next_sequence_for_test(run_id)
            .expect("test store next seq")
    }

    pub(super) fn committed_records_for_corruption(
        &self,
        run_id: &RunId,
    ) -> Vec<store::KernelEventEnvelope> {
        self.inner
            .committed_records_for_test(run_id)
            .expect("test committed records")
    }

    pub(super) fn committed_artifact_authority_for_corruption(
        &self,
        run_id: &RunId,
    ) -> store::ArtifactByteAuthorityMap {
        self.inner
            .committed_artifact_byte_authority_for_test(run_id)
            .expect("test committed artifact authority")
    }

    pub(super) fn remove_committed_artifact_for_corruption(
        &self,
        artifact_id: &ArtifactId,
        evidence_hash: &ContentDigest,
    ) {
        assert!(
            self.inner
                .remove_retained_artifact_bytes_for_test(artifact_id, evidence_hash)
                .expect("remove retained test artifact"),
            "retained test artifact must exist"
        );
    }

    pub(super) fn replace_committed_artifact_for_corruption(
        &self,
        artifact_id: &ArtifactId,
        evidence_hash: &ContentDigest,
        bytes: Vec<u8>,
    ) {
        assert!(
            self.inner
                .replace_retained_artifact_bytes_for_test(artifact_id, evidence_hash, bytes)
                .expect("replace retained test artifact"),
            "retained test artifact must exist"
        );
    }
}

impl store::RunJournalBackend for TestTypedRunStore {
    type Error = store::StoreError;

    fn backend_append<'a>(
        &'a self,
        bundle: store::PreparedCommitBundle,
    ) -> store::AsyncStoreFuture<'a, store::CommitOutcome, Self::Error> {
        Box::pin(async move {
            self.inner
                .seed_artifact_evidence_for_test(bundle.admitted_artifacts())?;
            self.inner.append_prepared_commit_bundle(bundle).await
        })
    }

    fn backend_load<'a>(
        &'a self,
        verifier: store::JournalLoadVerifier,
    ) -> store::AsyncStoreFuture<'a, store::CommittedRunJournal, Self::Error> {
        Box::pin(async move {
            let run_id = verifier.run_id().clone();
            let journal = self.inner.load_committed_journal(&run_id).await?;
            verifier.accept_verified(journal)
        })
    }
}

delegate_execution_claim_store!(TestTypedRunStore, delegate_execution_claim_direct);
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
}

impl store::RunJournalBackend for RecordingTypedRunStore {
    type Error = store::StoreError;

    fn backend_append<'a>(
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

    fn backend_load<'a>(
        &'a self,
        verifier: store::JournalLoadVerifier,
    ) -> store::AsyncStoreFuture<'a, store::CommittedRunJournal, Self::Error> {
        Box::pin(async move {
            let run_id = verifier.run_id().clone();
            let journal = self.inner.load_committed_journal(&run_id).await?;
            verifier.accept_verified(journal)
        })
    }
}

delegate_execution_claim_store!(RecordingTypedRunStore, delegate_execution_claim_direct);

impl store::RunJournalBackend for StaleOnceTypedRunStore {
    type Error = store::StoreError;

    fn backend_append<'a>(
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
                    actual: self.inner.expected_next_sequence_for_test(&run_id)?,
                });
            }
            self.inner
                .seed_artifact_evidence_for_test(bundle.admitted_artifacts())?;
            self.inner.append_prepared_commit_bundle(bundle).await
        })
    }

    fn backend_load<'a>(
        &'a self,
        verifier: store::JournalLoadVerifier,
    ) -> store::AsyncStoreFuture<'a, store::CommittedRunJournal, Self::Error> {
        Box::pin(async move {
            let run_id = verifier.run_id().clone();
            let journal = self.inner.load_committed_journal(&run_id).await?;
            verifier.accept_verified(journal)
        })
    }
}

delegate_execution_claim_store!(StaleOnceTypedRunStore, delegate_execution_claim_direct);
