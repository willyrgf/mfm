use super::*;

#[tokio::test]
async fn recovery_reuses_committed_read_facts_for_same_attempt() {
    struct FactReuseRunner {
        fact_key: mfm_facts::FactKey,
        output_artifact: ArtifactId,
        output_digest: ContentDigest,
    }

    impl ErasedNodeRunner for FactReuseRunner {
        fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
            Box::pin(async move {
                let fact = ctx
                    .recorded_facts()
                    .by_fact_key(&self.fact_key)
                    .next()
                    .map(|(_, fact)| fact)
                    .expect("recorded fact");
                assert_eq!(fact.fact_key, self.fact_key);
                assert_eq!(
                    fact.request_schema_id,
                    Some(ctx.node().config_ref.schema_id.clone())
                );
                assert_eq!(ctx.recorded_facts().iter().count(), 1);
                let artifact = store::ArtifactEvidenceRef {
                    artifact_id: self.output_artifact.clone(),
                    digest: self.output_digest.clone(),
                    byte_len: 17,
                    media_type: spec::MediaType::new("application/json").expect("media"),
                    schema_id: Some(ctx.descriptor().output_schema_id.clone()),
                    semantic_type_id: Some(ctx.descriptor().output_semantic_type_id.clone()),
                    producer_node_id: Some(ctx.node().node_id.clone()),
                    producer_seed_id: None,
                    artifact_role: events::ArtifactRole::StateOutput,
                };
                let staged_artifact = staged_attempt_artifact(&ctx, artifact.clone())?;
                Ok(ErasedRunnerOutput::from_parts(
                    vec![staged_artifact],
                    Vec::new(),
                    terminal_payloads(&ctx, &artifact),
                ))
            })
        }
    }

    let (fixture, fact_descriptor, _descriptor_ref) = fixture_with_read_node_fact_descriptor();
    let fact_key = test_fact_key(212);
    let mut registry = ErasedRunnerRegistry::new();
    register_default_fixture_pure_runner(&mut registry, &fixture);
    registry
        .register(binding(
            fixture.descriptor_b.clone(),
            "read",
            FactReuseRunner {
                fact_key: fact_key.clone(),
                output_artifact: artifact(0xb1),
                output_digest: content(0xb2),
            },
        ))
        .expect("binding b");
    let (scheduler, mut store) = started_fixture_run_with_registry_and_fact_descriptors(
        registry,
        &fixture,
        &[fact_descriptor],
    )
    .await;
    drive_ok!(scheduler, store, fixture, "produce input");
    let node = node_by_output(&fixture, &fixture.cell_b);
    let attempt_id = append_attempt_start(&mut store, &fixture, node, 1);
    append_fact(&mut store, &fixture, node, &attempt_id, 212, 212);

    drive_ok!(scheduler, store, fixture, "resume read");
    assert_eq!(fact_recorded_count(&store, &fact_key), 1);
    match store
        .projection_snapshot()
        .cell_terminal(&fixture.cell_b)
        .expect("terminal cell")
    {
        store::CellTerminalProjection::Produced {
            attempt_id: produced_attempt,
            ..
        } => assert_eq!(produced_attempt, &attempt_id),
        terminal => panic!("unexpected terminal projection: {terminal:?}"),
    }
}

#[tokio::test]
async fn recovery_retains_same_subject_facts_by_claim_id_for_same_attempt() {
    struct SameSubjectFactsRunner {
        fact_key: mfm_facts::FactKey,
        output_artifact: ArtifactId,
        output_digest: ContentDigest,
    }

    impl ErasedNodeRunner for SameSubjectFactsRunner {
        fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
            Box::pin(async move {
                let same_subject_facts = ctx
                    .recorded_facts()
                    .by_fact_key(&self.fact_key)
                    .collect::<Vec<_>>();
                assert_eq!(same_subject_facts.len(), 2);
                assert_eq!(ctx.recorded_facts().iter().count(), 2);

                let claim_ids = same_subject_facts
                    .iter()
                    .map(|(claim_id, _)| (*claim_id).clone())
                    .collect::<BTreeSet<_>>();
                assert_eq!(claim_ids.len(), 2);

                let artifact_ids = same_subject_facts
                    .iter()
                    .map(|(claim_id, fact)| {
                        assert_eq!(&fact.fact_claim_id, *claim_id);
                        assert_eq!(&fact.fact_key, &self.fact_key);
                        fact.artifact_id.clone()
                    })
                    .collect::<BTreeSet<_>>();
                assert_eq!(artifact_ids.len(), 2);

                let artifact = store::ArtifactEvidenceRef {
                    artifact_id: self.output_artifact.clone(),
                    digest: self.output_digest.clone(),
                    byte_len: 17,
                    media_type: spec::MediaType::new("application/json").expect("media"),
                    schema_id: Some(ctx.descriptor().output_schema_id.clone()),
                    semantic_type_id: Some(ctx.descriptor().output_semantic_type_id.clone()),
                    producer_node_id: Some(ctx.node().node_id.clone()),
                    producer_seed_id: None,
                    artifact_role: events::ArtifactRole::StateOutput,
                };
                let staged_artifact = staged_attempt_artifact(&ctx, artifact.clone())?;
                Ok(ErasedRunnerOutput::from_parts(
                    vec![staged_artifact],
                    Vec::new(),
                    terminal_payloads(&ctx, &artifact),
                ))
            })
        }
    }

    let (fixture, fact_descriptor, _descriptor_ref) = fixture_with_read_node_fact_descriptor();
    let fact_key = test_fact_key(212);
    let mut registry = ErasedRunnerRegistry::new();
    register_default_fixture_pure_runner(&mut registry, &fixture);
    registry
        .register(binding(
            fixture.descriptor_b.clone(),
            "read",
            SameSubjectFactsRunner {
                fact_key: fact_key.clone(),
                output_artifact: artifact(0xb3),
                output_digest: content(0xb4),
            },
        ))
        .expect("binding b");
    let (scheduler, mut store) = started_fixture_run_with_registry_and_fact_descriptors(
        registry,
        &fixture,
        &[fact_descriptor],
    )
    .await;
    drive_ok!(scheduler, store, fixture, "produce input");
    let node = node_by_output(&fixture, &fixture.cell_b);
    let attempt_id = append_attempt_start(&mut store, &fixture, node, 1);
    append_fact(&mut store, &fixture, node, &attempt_id, 212, 212);
    append_fact(&mut store, &fixture, node, &attempt_id, 212, 213);

    assert_eq!(fact_recorded_count(&store, &fact_key), 2);
    let projected_claim_ids = store
        .projection_snapshot()
        .fact_records()
        .filter(|(_, fact)| fact.claim.subject().fact_key() == &fact_key)
        .map(|(claim_id, _)| claim_id.clone())
        .collect::<BTreeSet<_>>();
    assert_eq!(projected_claim_ids.len(), 2);

    drive_ok!(
        scheduler,
        store,
        fixture,
        "resume read with same-subject facts"
    );
    assert_eq!(fact_recorded_count(&store, &fact_key), 2);
    match store
        .projection_snapshot()
        .cell_terminal(&fixture.cell_b)
        .expect("terminal cell")
    {
        store::CellTerminalProjection::Produced {
            attempt_id: produced_attempt,
            ..
        } => assert_eq!(produced_attempt, &attempt_id),
        terminal => panic!("unexpected terminal projection: {terminal:?}"),
    }
}

#[tokio::test]
async fn runtime_history_rejects_fact_descriptor_allowed_only_for_other_node() {
    let (fixture, read_descriptor, other_descriptor) =
        fixture_with_read_node_and_other_node_fact_descriptors();
    let (scheduler, mut store) = started_fixture_run_with_registry_and_fact_descriptors(
        registered_fixture_runners(&fixture),
        &fixture,
        &[read_descriptor, other_descriptor.clone()],
    )
    .await;
    drive_ok!(scheduler, store, fixture, "produce input");
    let node = node_by_output(&fixture, &fixture.cell_b);
    let attempt_id = append_attempt_start(&mut store, &fixture, node, 1);
    append_fact(&mut store, &fixture, node, &attempt_id, 212, 212);

    let (response_evidence, _response_bytes) = test_fact_response_artifact(node, 212);
    let corrupt_claim = test_fact_claim_for_descriptor(
        &other_descriptor,
        212,
        node.config_ref.schema_id.clone(),
        content(0xd4),
        &response_evidence,
        fixture.cap_kind.clone(),
        fixture.cap_version.clone(),
        fixture.adapter_kind.clone(),
        fixture.adapter_version.clone(),
    );
    let valid_stream = store.load_run_stream(&fixture.run_id);
    let corrupt_stream = rewrite_stream_payloads(&valid_stream, |payload| match payload {
        events::KernelEventPayload::FactRecorded(recorded) if recorded.node_id == node.node_id => {
            let mut recorded = recorded.clone();
            recorded.claim = corrupt_claim.clone();
            Some(events::KernelEventPayload::FactRecorded(recorded))
        }
        _ => None,
    });
    let committed =
        block_on_ready(store.load_committed_run_stream(&fixture.run_id)).expect("committed stream");
    let corrupt_committed = store::CommittedRunStream::from_events_with_artifact_bytes(
        fixture.run_id.clone(),
        corrupt_stream,
        committed.artifact_byte_authority(),
    )
    .expect("corrupt committed stream remains structurally valid");

    assert!(matches!(
        RuntimeRunView::from_committed_stream(&fixture.runtime_spec, &corrupt_committed),
        Err(RuntimeError::InvalidRunStream(message))
            if message.contains("fact descriptor")
                && message.contains("not certified for producing node")
    ));
}

#[tokio::test]
async fn recovery_rejects_new_fact_after_same_attempt_fact_exists() {
    struct NewFactRunner {
        cap_kind: CapabilityKind,
        cap_version: CapabilityVersion,
        adapter_kind: AdapterKind,
        adapter_version: AdapterVersion,
    }

    impl ErasedNodeRunner for NewFactRunner {
        fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
            Box::pin(async move {
                let (response_evidence, _response_bytes) =
                    test_fact_response_artifact(ctx.node(), 224);
                Ok(ErasedRunnerOutput::new(vec![
                    RunnerEventPayload::FactRecorded(RunnerFactRecorded::new(
                        events::FactRecorded {
                            spec_hash: ctx.spec_hash().clone(),
                            node_id: ctx.node().node_id.clone(),
                            attempt_id: ctx.attempt_id().clone(),
                            claim: test_fact_claim(
                                224,
                                ctx.node().config_ref.schema_id.clone(),
                                content(0xe1),
                                &response_evidence,
                                self.cap_kind.clone(),
                                self.cap_version.clone(),
                                self.adapter_kind.clone(),
                                self.adapter_version.clone(),
                            ),
                        },
                    )),
                ]))
            })
        }
    }

    let (fixture, fact_descriptor, _descriptor_ref) = fixture_with_read_node_fact_descriptor();
    let mut registry = ErasedRunnerRegistry::new();
    register_default_fixture_pure_runner(&mut registry, &fixture);
    registry
        .register(binding(
            fixture.descriptor_b.clone(),
            "read",
            NewFactRunner {
                cap_kind: fixture.cap_kind.clone(),
                cap_version: fixture.cap_version.clone(),
                adapter_kind: fixture.adapter_kind.clone(),
                adapter_version: fixture.adapter_version.clone(),
            },
        ))
        .expect("binding b");
    let (scheduler, mut store) = started_fixture_run_with_registry_and_fact_descriptors(
        registry,
        &fixture,
        &[fact_descriptor],
    )
    .await;
    drive_ok!(scheduler, store, fixture, "produce input");
    let node = node_by_output(&fixture, &fixture.cell_b);
    let attempt_id = append_attempt_start(&mut store, &fixture, node, 1);
    append_fact(&mut store, &fixture, node, &attempt_id, 214, 214);

    assert_drive!(
        scheduler,
        store,
        fixture,
        Advanced,
        "terminalize duplicate fact output"
    );
    assert_node_failed_with_code(&store, &node.node_id, "runner_output_invalid");
    assert_eq!(store.projection_snapshot().fact_records().count(), 1);
}
