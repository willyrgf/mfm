use super::*;

pub(super) struct StaleStreamStore<'a> {
    inner: RefCell<&'a mut TestTypedRunStore>,
    stream: Vec<store::KernelEventEnvelope>,
}

impl<'a> StaleStreamStore<'a> {
    pub(super) fn new(
        inner: &'a mut TestTypedRunStore,
        stream: Vec<store::KernelEventEnvelope>,
    ) -> Self {
        Self {
            inner: RefCell::new(inner),
            stream,
        }
    }
}

impl store::RunEventStore for StaleStreamStore<'_> {
    type Error = store::StoreError;

    fn append_prepared_commit_bundle<'a>(
        &'a self,
        bundle: store::PreparedCommitBundle,
    ) -> store::AsyncStoreFuture<'a, store::CommitOutcome, Self::Error> {
        let result = block_on_ready(
            self.inner
                .borrow_mut()
                .append_prepared_commit_bundle(bundle),
        );
        Box::pin(std::future::ready(result))
    }

    fn load_run_stream<'a>(
        &'a self,
        run_id: &'a RunId,
    ) -> store::AsyncStoreFuture<'a, Vec<store::KernelEventEnvelope>, Self::Error> {
        let _ = run_id;
        Box::pin(std::future::ready(Ok(self.stream.clone())))
    }

    fn load_committed_run_stream<'a>(
        &'a self,
        run_id: &'a RunId,
    ) -> store::AsyncStoreFuture<'a, store::CommittedRunStream, Self::Error> {
        let result = store::CommittedRunStream::from_events(run_id.clone(), self.stream.clone());
        Box::pin(std::future::ready(result))
    }

    fn expected_next_seq<'a>(
        &'a self,
        run_id: &'a RunId,
    ) -> store::AsyncStoreFuture<'a, store::StreamSeq, Self::Error> {
        let result = Ok(self.inner.borrow().expected_next_seq(run_id));
        Box::pin(std::future::ready(result))
    }

    fn status_projection_snapshot<'a>(
        &'a self,
        _run_id: &'a RunId,
    ) -> store::AsyncStoreFuture<'a, store::ProjectionSnapshot, Self::Error> {
        let result = Ok(self.inner.borrow().projection_snapshot().clone());
        Box::pin(std::future::ready(result))
    }

    fn fact_projection_snapshot<'a>(
        &'a self,
    ) -> store::AsyncStoreFuture<'a, store::ProjectionSnapshot, Self::Error> {
        let result = Ok(self.inner.borrow().projection_snapshot().clone());
        Box::pin(std::future::ready(result))
    }
}

delegate_execution_claim_store!(StaleStreamStore<'_>, delegate_execution_claim_refcell);

pub(super) struct MissingInputArtifactRefStore<'a> {
    inner: RefCell<&'a mut TestTypedRunStore>,
    producer_node_id: NodeId,
}

impl<'a> MissingInputArtifactRefStore<'a> {
    pub(super) fn new(inner: &'a mut TestTypedRunStore, producer_node_id: NodeId) -> Self {
        Self {
            inner: RefCell::new(inner),
            producer_node_id,
        }
    }
}

impl store::RunEventStore for MissingInputArtifactRefStore<'_> {
    type Error = store::StoreError;

    fn append_prepared_commit_bundle<'a>(
        &'a self,
        bundle: store::PreparedCommitBundle,
    ) -> store::AsyncStoreFuture<'a, store::CommitOutcome, Self::Error> {
        let result = block_on_ready(
            self.inner
                .borrow_mut()
                .append_prepared_commit_bundle(bundle),
        );
        Box::pin(std::future::ready(result))
    }

    fn load_run_stream<'a>(
        &'a self,
        run_id: &'a RunId,
    ) -> store::AsyncStoreFuture<'a, Vec<store::KernelEventEnvelope>, Self::Error> {
        let result = rewrite_stream_without_payloads(
            &self.inner.borrow().load_run_stream(run_id),
            |payload| {
                matches!(
                    payload,
                    events::KernelEventPayload::ArtifactReferenced(payload)
                        if payload.artifact_ref.role == events::ArtifactRole::StateOutput
                            && payload.node_id.as_ref() == Some(&self.producer_node_id)
                )
            },
        );
        Box::pin(std::future::ready(Ok(result)))
    }

    fn load_committed_run_stream<'a>(
        &'a self,
        run_id: &'a RunId,
    ) -> store::AsyncStoreFuture<'a, store::CommittedRunStream, Self::Error> {
        let stream = rewrite_stream_without_payloads(
            &self.inner.borrow().load_run_stream(run_id),
            |payload| {
                matches!(
                    payload,
                    events::KernelEventPayload::ArtifactReferenced(payload)
                        if payload.artifact_ref.role == events::ArtifactRole::StateOutput
                            && payload.node_id.as_ref() == Some(&self.producer_node_id)
                )
            },
        );
        let result = store::CommittedRunStream::from_events(run_id.clone(), stream);
        Box::pin(std::future::ready(result))
    }

    fn expected_next_seq<'a>(
        &'a self,
        run_id: &'a RunId,
    ) -> store::AsyncStoreFuture<'a, store::StreamSeq, Self::Error> {
        let result = Ok(self.inner.borrow().expected_next_seq(run_id));
        Box::pin(std::future::ready(result))
    }

    fn status_projection_snapshot<'a>(
        &'a self,
        _run_id: &'a RunId,
    ) -> store::AsyncStoreFuture<'a, store::ProjectionSnapshot, Self::Error> {
        let result = Ok(self.inner.borrow().projection_snapshot().clone());
        Box::pin(std::future::ready(result))
    }

    fn fact_projection_snapshot<'a>(
        &'a self,
    ) -> store::AsyncStoreFuture<'a, store::ProjectionSnapshot, Self::Error> {
        let result = Ok(self.inner.borrow().projection_snapshot().clone());
        Box::pin(std::future::ready(result))
    }
}

delegate_execution_claim_store!(
    MissingInputArtifactRefStore<'_>,
    delegate_execution_claim_refcell
);

pub(super) fn rewrite_envelope(
    event: &store::KernelEventEnvelope,
    seq: store::StreamSeq,
    ordinal: store::CommitOrdinal,
    commit_key: store::CommitKey,
) -> store::KernelEventEnvelope {
    store::KernelEventEnvelope::from_persisted_record(store::PersistedKernelEventRecord {
        event_id: test_event_id_for_envelope_inputs(
            event.run_id(),
            seq,
            ordinal,
            event.event_schema_id(),
            event.payload_hash(),
        ),
        event_schema_id: event.event_schema_id().clone(),
        run_id: event.run_id().clone(),
        seq,
        store_commit_order: event.store_commit_order(),
        ordinal,
        spec_hash: event.spec_hash().clone(),
        commit_key,
        logical_key: event.logical_key().clone(),
        payload_hash: event.payload_hash().clone(),
        payload: event.payload().clone(),
    })
    .expect("rewritten envelope")
}

pub(super) fn rewrite_envelope_payload(
    event: &store::KernelEventEnvelope,
    payload: events::KernelEventPayload,
) -> store::KernelEventEnvelope {
    let payload_hash = store::payload_canonical_json(&payload)
        .expect("payload canonical")
        .content_digest();
    let event_schema_id = payload.event_schema_id().expect("event schema");
    store::KernelEventEnvelope::from_persisted_record(store::PersistedKernelEventRecord {
        event_id: test_event_id_for_envelope_inputs(
            event.run_id(),
            event.seq(),
            event.ordinal(),
            &event_schema_id,
            &payload_hash,
        ),
        event_schema_id,
        run_id: event.run_id().clone(),
        seq: event.seq(),
        store_commit_order: event.store_commit_order(),
        ordinal: event.ordinal(),
        spec_hash: payload.spec_hash().clone(),
        commit_key: event.commit_key().clone(),
        logical_key: event.logical_key().clone(),
        payload_hash,
        payload,
    })
    .expect("rewritten envelope payload")
}

pub(super) fn rewrite_stream_payloads<F>(
    stream: &[store::KernelEventEnvelope],
    mut rewrite: F,
) -> Vec<store::KernelEventEnvelope>
where
    F: FnMut(&events::KernelEventPayload) -> Option<events::KernelEventPayload>,
{
    stream
        .iter()
        .map(|event| {
            rewrite(event.payload())
                .map(|payload| rewrite_envelope_payload(event, payload))
                .unwrap_or_else(|| event.clone())
        })
        .collect()
}

pub(super) fn rewrite_stream_without_payloads<F>(
    stream: &[store::KernelEventEnvelope],
    mut should_remove: F,
) -> Vec<store::KernelEventEnvelope>
where
    F: FnMut(&events::KernelEventPayload) -> bool,
{
    let mut rewritten = Vec::with_capacity(stream.len());
    let mut index = 0;
    while index < stream.len() {
        let first = &stream[index];
        let seq = first.seq();
        let commit_key = first.commit_key().clone();
        let mut end = index + 1;
        while end < stream.len()
            && stream[end].seq() == seq
            && stream[end].commit_key() == &commit_key
        {
            end += 1;
        }
        let commit = &stream[index..end];
        let payloads = commit
            .iter()
            .filter(|event| !should_remove(event.payload()))
            .map(|event| event.payload().clone())
            .collect::<Vec<_>>();
        assert!(
            !payloads.is_empty(),
            "test corruption helper must not remove an entire commit"
        );
        if payloads.len() == commit.len() {
            rewritten.extend(commit.iter().cloned());
        } else {
            let mut ordinal = 0;
            for event in commit {
                if should_remove(event.payload()) {
                    continue;
                }
                rewritten.push(rewrite_envelope(
                    event,
                    seq,
                    store::CommitOrdinal::new(ordinal),
                    commit_key.clone(),
                ));
                ordinal += 1;
            }
        }
        index = end;
    }
    rewritten
}

pub(super) fn assert_every_certified_node_has_attempt(
    runtime_spec: &CertifiedRuntimeSpec,
    stream: &[store::KernelEventEnvelope],
) {
    let mut started = BTreeSet::new();
    let mut completed = BTreeSet::new();
    for event in stream {
        match event.payload() {
            events::KernelEventPayload::StateAttemptStarted(payload) => {
                started.insert(payload.node_id.clone());
            }
            events::KernelEventPayload::StateAttemptCompleted(payload) => {
                completed.insert(payload.node_id.clone());
            }
            _ => {}
        }
    }
    for node in &runtime_spec.spec().nodes {
        if matches!(
            node.framework,
            Some(spec::FrameworkNodeSpec::ResolveSagaTerminal(_))
        ) {
            continue;
        }
        assert!(
            started.contains(&node.node_id),
            "node {} has no StateAttemptStarted",
            node.node_id
        );
        assert!(
            completed.contains(&node.node_id),
            "node {} has no StateAttemptCompleted",
            node.node_id
        );
    }
}

pub(super) fn referenced_artifact_ids_for_payload(
    payload: &events::KernelEventPayload,
) -> Vec<ArtifactId> {
    store::event_artifact_requirements(payload)
        .into_iter()
        .map(|requirement| requirement.artifact_id)
        .collect()
}
