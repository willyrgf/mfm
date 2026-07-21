use super::*;

#[derive(Clone, Copy, Debug)]
pub(super) enum CommittedStreamArtifactMode {
    Missing,
    TamperFactResponse,
}

#[derive(Clone)]
pub(super) struct OverriddenCommittedStreamStore {
    inner: store::AsyncInMemoryRunStore,
    mode: CommittedStreamArtifactMode,
}

impl OverriddenCommittedStreamStore {
    pub(super) fn new(
        inner: store::AsyncInMemoryRunStore,
        mode: CommittedStreamArtifactMode,
    ) -> Self {
        Self { inner, mode }
    }
}

impl store::RunEventStore for OverriddenCommittedStreamStore {
    type Error = store::StoreError;

    fn append_prepared_commit_bundle<'a>(
        &'a self,
        bundle: store::PreparedCommitBundle,
    ) -> store::AsyncStoreFuture<'a, store::CommitOutcome, Self::Error> {
        self.inner.append_prepared_commit_bundle(bundle)
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
        Box::pin(async move {
            let committed = self.inner.load_committed_run_stream(run_id).await?;
            let mut artifact_bytes = committed.artifact_byte_authority().clone();
            match self.mode {
                CommittedStreamArtifactMode::Missing => artifact_bytes.clear(),
                CommittedStreamArtifactMode::TamperFactResponse => {
                    let (_, (bytes, _)) = artifact_bytes
                        .iter_mut()
                        .find(|(_, (_, evidence))| {
                            evidence.artifact_role == events::ArtifactRole::FactResponse
                        })
                        .expect("fact response artifact bytes");
                    bytes.push(b'\n');
                }
            }
            store::CommittedRunStream::from_events_with_artifact_bytes(
                run_id.clone(),
                committed.events().to_vec(),
                &artifact_bytes,
            )
        })
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

impl store::StoreScopeStore for OverriddenCommittedStreamStore {
    type Error = store::StoreError;

    fn load_store_scope_id<'a>(&'a self) -> store::AsyncStoreFuture<'a, StoreScopeId, Self::Error> {
        self.inner.load_store_scope_id()
    }
}

impl store::RetainedArtifactReadProvider for OverriddenCommittedStreamStore {
    fn read_retained_artifact<'a>(
        &'a self,
        requirement: &'a store::EventArtifactRequirement,
    ) -> store::RetainedArtifactReadFuture<'a> {
        self.inner.read_retained_artifact(requirement)
    }
}
