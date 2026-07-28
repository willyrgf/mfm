use super::*;

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

pub(super) fn rewrite_record_payload(
    record: &store::KernelEventEnvelope,
    payload: events::KernelEventPayload,
) -> store::KernelEventEnvelope {
    test_persisted_event_with_ordinal(
        record.run_id(),
        record.seq().as_u64(),
        record.store_commit_order().as_u64(),
        record.ordinal().as_u32(),
        record.commit_key().clone(),
        payload,
    )
}

pub(super) fn rewrite_records_without_payloads<F>(
    records: &[store::KernelEventEnvelope],
    mut should_remove: F,
) -> Vec<store::KernelEventEnvelope>
where
    F: FnMut(&events::KernelEventPayload) -> bool,
{
    let mut rewritten = Vec::with_capacity(records.len());
    let mut index = 0;
    while index < records.len() {
        let first = &records[index];
        let seq = first.seq();
        let commit_key = first.commit_key().clone();
        let mut end = index + 1;
        while end < records.len()
            && records[end].seq() == seq
            && records[end].commit_key() == &commit_key
        {
            end += 1;
        }
        let commit = &records[index..end];
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
    current: &VerifiedCurrentRun,
) {
    let mut started = BTreeSet::new();
    let mut completed = BTreeSet::new();
    let _ = current.lifecycle().visit_records::<()>(|record| {
        match record.kind() {
            store::current_lifecycle::CurrentRecordKindRef::StateAttemptStarted(payload) => {
                started.insert(payload.node_id.clone());
            }
            store::current_lifecycle::CurrentRecordKindRef::StateAttemptCompleted(payload) => {
                completed.insert(payload.node_id.clone());
            }
            _ => {}
        }
        std::ops::ControlFlow::Continue(())
    });
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
