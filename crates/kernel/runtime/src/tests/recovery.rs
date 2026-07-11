use super::*;

#[tokio::test]
async fn store_rejects_fact_without_started_attempt() {
    let fixture = fixture();
    let (_, mut store) = started_fixture_run(&fixture).await;
    let node = fixture
        .runtime_spec
        .topological_order()
        .iter()
        .filter_map(|node_id| fixture.runtime_spec.node(node_id))
        .find(|node| node.output_cell == fixture.cell_b)
        .expect("read node")
        .clone();
    let fact_schema = node.config_ref.schema_id.clone();
    let (fact_evidence, _fact_bytes) = test_fact_response_artifact(&node, 210);
    assert!(store
        .append_prepared_commit(store_typed_commit_request! {
            run_id: fixture.run_id.clone(),
            expected_next_seq: store.expected_next_seq(&fixture.run_id),
            commit_key: store::CommitKey::new("forged-fact").expect("commit key"),
            payloads: vec![events::KernelEventPayload::FactRecorded(
                events::FactRecorded {
                    spec_hash: fixture.runtime_spec.spec_hash().clone(),
                    node_id: node.node_id.clone(),
                    attempt_id: AttemptId::from_digest(
                        DigestAlgorithm::Sha256JcsV1,
                        DigestBytes::from_array([0xd3; 32]),
                    ),
                    claim: test_fact_claim(
                        210,
                        fact_schema.clone(),
                        content(0xd4),
                        &fact_evidence,
                        fixture.cap_kind.clone(),
                        fixture.cap_version.clone(),
                        fixture.adapter_kind.clone(),
                        fixture.adapter_version.clone(),
                    ),
                },
            )],
            required_artifacts: vec![fact_evidence],
            preconditions: store::CommitPreconditions {
                required_run_state: store::RequiredRunState::NotCompleted,
                ..store::CommitPreconditions::default()
            },
        })
        .is_err());
}

#[tokio::test]
async fn replay_rejects_public_output_without_render_attempt() {
    let fixture = fixture();
    let (scheduler, mut store) = started_fixture_run(&fixture).await;
    let non_render_node = fixture
        .runtime_spec
        .topological_order()
        .iter()
        .filter_map(|node_id| fixture.runtime_spec.node(node_id))
        .find(|node| node.output_cell == fixture.cell_a)
        .expect("non-render node")
        .clone();
    let public_cell = fixture
        .runtime_spec
        .spec()
        .public_outputs
        .outputs
        .first()
        .expect("public cell")
        .clone();
    let source_artifact = artifact(0xe1);
    let source_digest = content(0xe2);
    let producer_node_id = match &public_cell.producer {
        spec::CellProducer::Node(node_id) => Some(node_id.clone()),
        spec::CellProducer::Seed(_) => None,
    };
    let source_evidence = store::ArtifactEvidenceRef {
        artifact_id: source_artifact.clone(),
        digest: source_digest.clone(),
        byte_len: 10,
        media_type: spec::MediaType::new("application/json").expect("media"),
        schema_id: Some(public_cell.schema_id.clone()),
        semantic_type_id: Some(public_cell.semantic_type_id.clone()),
        producer_node_id,
        producer_seed_id: None,
        artifact_role: events::ArtifactRole::StateOutput,
    };
    let forged_attempt = append_attempt_start(&mut store, &fixture, &non_render_node, 1);
    let output_cell = fixture
        .runtime_spec
        .cell(&non_render_node.output_cell)
        .expect("non-render output")
        .clone();
    let receipt_artifact = artifact(0xe3);
    let receipt_digest = content(0xe4);
    let receipt_evidence = store::ArtifactEvidenceRef {
        artifact_id: receipt_artifact.clone(),
        digest: receipt_digest.clone(),
        byte_len: 10,
        media_type: spec::MediaType::new("application/json").expect("media"),
        schema_id: Some(output_cell.schema_id.clone()),
        semantic_type_id: Some(output_cell.semantic_type_id.clone()),
        producer_node_id: Some(non_render_node.node_id.clone()),
        producer_seed_id: None,
        artifact_role: events::ArtifactRole::StateOutput,
    };
    store
        .append_prepared_commit(store_typed_commit_request! {
            run_id: fixture.run_id.clone(),
            expected_next_seq: store.expected_next_seq(&fixture.run_id),
            commit_key: store::CommitKey::new("forged-public-output").expect("commit key"),
            payloads: vec![
                events::KernelEventPayload::CellProduced(events::CellProduced {
                    spec_hash: fixture.runtime_spec.spec_hash().clone(),
                    node_id: non_render_node.node_id.clone(),
                    cell_id: non_render_node.output_cell.clone(),
                    scope_id: output_cell.scope_id.clone(),
                    attempt_id: forged_attempt.clone(),
                    semantic_type_id: output_cell.semantic_type_id.clone(),
                    schema_id: output_cell.schema_id.clone(),
                    value_lineage: output_cell.value_lineage.clone(),
                    context: output_cell.context.clone(),
                    artifact_id: receipt_artifact,
                    content_digest: receipt_digest,
                    evidence_hash: receipt_evidence
                        .evidence_hash()
                        .expect("forged public output receipt evidence hash"),
                    producer_state_kind: Some(non_render_node.state_kind.clone()),
                    producer_state_version: Some(non_render_node.state_version.clone()),
                }),
                events::KernelEventPayload::PublicOutputProduced(events::PublicOutputProduced {
                    spec_hash: fixture.runtime_spec.spec_hash().clone(),
                    node_id: non_render_node.node_id.clone(),
                    attempt_id: forged_attempt.clone(),
                    receipt_cell_id: non_render_node.output_cell.clone(),
                    public_schema_id: fixture
                        .runtime_spec
                        .spec()
                        .public_outputs
                        .public_schema_id
                        .clone(),
                    output_spec_digest: fixture
                        .runtime_spec
                        .spec()
                        .public_outputs
                        .digest()
                        .expect("public digest"),
                    cells: vec![events::NamedTypedCellRef {
                        public_field_path: public_cell.public_field_path.clone(),
                        cell_id: public_cell.cell_id.clone(),
                        producer: public_cell.producer.clone(),
                        scope_id: public_cell.scope_id.clone(),
                        semantic_type_id: public_cell.semantic_type_id.clone(),
                        schema_id: public_cell.schema_id.clone(),
                        value_lineage: public_cell.value_lineage.clone(),
                        content_digest: source_digest,
                        evidence_hash: source_evidence
                            .evidence_hash()
                            .expect("forged public output source evidence hash"),
                        artifact_id: source_artifact,
                    }],
                    rendered_digest: content(0xe5),
                    rendered_artifact_id: None,
                    rendered_artifact_evidence_hash: None,
                    renderer_descriptor_id: fixture
                        .runtime_spec
                        .spec()
                        .public_outputs
                        .renderer_descriptor
                        .descriptor_id
                        .clone(),
                }),
                events::KernelEventPayload::StateAttemptCompleted(events::StateAttemptCompleted {
                    spec_hash: fixture.runtime_spec.spec_hash().clone(),
                    node_id: non_render_node.node_id.clone(),
                    attempt_id: forged_attempt.clone(),
                    output_cell_id: non_render_node.output_cell.clone(),
                }),
            ],
            required_artifacts: vec![source_evidence, receipt_evidence],
            preconditions: store::CommitPreconditions {
                required_run_state: store::RequiredRunState::NotCompleted,
                required_present_logical_keys: vec![store::LogicalEventKey::new(format!(
                    "attempt:{}:{}",
                    non_render_node.node_id, forged_attempt
                ))
                .expect("attempt key")],
                required_cell_states: vec![store::CellStatePrecondition {
                    cell_id: non_render_node.output_cell.clone(),
                    required: store::RequiredCellState::Absent,
                }],
                required_public_output_absent: true,
                ..store::CommitPreconditions::default()
            },
        })
        .expect("append forged public output");
    assert!(matches!(
        drive_fixture_once(&scheduler, &mut store, &fixture).await,
        Err(RuntimeError::InvalidRunStream(_))
    ));
}

#[tokio::test]
async fn replay_rejects_public_output_with_forged_rendered_digest() {
    let fixture = fixture();
    let (scheduler, mut store) = started_fixture_run(&fixture).await;
    drive_ok!(scheduler, store, fixture, "drive a");
    drive_ok!(scheduler, store, fixture, "drive b");

    let render_node = node_by_output(&fixture, &fixture.render_cell).clone();
    let attempt_id = append_attempt_start(&mut store, &fixture, &render_node, 1);
    let output_cell = fixture
        .runtime_spec
        .cell(&render_node.output_cell)
        .expect("render output")
        .clone();
    let bad_rendered_digest = content(0xf1);
    let bad_receipt_digest = content(0xf2);
    let bad_receipt_artifact =
        ArtifactId::from_digest(bad_receipt_digest.algorithm(), *bad_receipt_digest.digest());
    let bad_receipt_evidence = store::ArtifactEvidenceRef {
        artifact_id: bad_receipt_artifact.clone(),
        digest: bad_receipt_digest.clone(),
        byte_len: 17,
        media_type: spec::MediaType::new("application/json").expect("media"),
        schema_id: Some(output_cell.schema_id.clone()),
        semantic_type_id: Some(output_cell.semantic_type_id.clone()),
        producer_node_id: Some(render_node.node_id.clone()),
        producer_seed_id: None,
        artifact_role: events::ArtifactRole::StateOutput,
    };
    let public_cells = fixture
        .runtime_spec
        .spec()
        .public_outputs
        .outputs
        .iter()
        .map(|public_cell| {
            let projection_snapshot = store.projection_snapshot();
            let Some(store::CellTerminalProjection::Produced {
                artifact_id,
                content_digest,
                ..
            }) = projection_snapshot.cell_terminal(&public_cell.cell_id)
            else {
                panic!("public cell should be produced");
            };
            events::NamedTypedCellRef {
                public_field_path: public_cell.public_field_path.clone(),
                cell_id: public_cell.cell_id.clone(),
                producer: public_cell.producer.clone(),
                scope_id: public_cell.scope_id.clone(),
                semantic_type_id: public_cell.semantic_type_id.clone(),
                schema_id: public_cell.schema_id.clone(),
                value_lineage: public_cell.value_lineage.clone(),
                content_digest: content_digest.clone(),
                evidence_hash: content_digest.clone(),
                artifact_id: artifact_id.clone(),
            }
        })
        .collect::<Vec<_>>();
    let Some(spec::FrameworkNodeSpec::PublicOutputRender(render)) = &render_node.framework else {
        panic!("expected render node");
    };
    let error = store
        .append_prepared_commit(store_typed_commit_request! {
            run_id: fixture.run_id.clone(),
            expected_next_seq: store.expected_next_seq(&fixture.run_id),
            commit_key: store::CommitKey::new("forged-public-output-rendered-digest")
                .expect("commit key"),
            payloads: vec![
                events::KernelEventPayload::CellProduced(events::CellProduced {
                    spec_hash: fixture.runtime_spec.spec_hash().clone(),
                    node_id: render_node.node_id.clone(),
                    cell_id: render_node.output_cell.clone(),
                    scope_id: output_cell.scope_id.clone(),
                    attempt_id: attempt_id.clone(),
                    semantic_type_id: output_cell.semantic_type_id.clone(),
                    schema_id: output_cell.schema_id.clone(),
                    value_lineage: output_cell.value_lineage.clone(),
                    context: output_cell.context.clone(),
                    artifact_id: bad_receipt_artifact,
                    content_digest: bad_receipt_digest.clone(),
                    evidence_hash: bad_receipt_digest,
                    producer_state_kind: Some(render_node.state_kind.clone()),
                    producer_state_version: Some(render_node.state_version.clone()),
                }),
                events::KernelEventPayload::PublicOutputProduced(events::PublicOutputProduced {
                    spec_hash: fixture.runtime_spec.spec_hash().clone(),
                    node_id: render_node.node_id.clone(),
                    attempt_id: attempt_id.clone(),
                    receipt_cell_id: render_node.output_cell.clone(),
                    public_schema_id: render.public_schema_id.clone(),
                    output_spec_digest: render.output_spec_digest.clone(),
                    cells: public_cells,
                    rendered_digest: bad_rendered_digest,
                    rendered_artifact_id: None,
                    rendered_artifact_evidence_hash: None,
                    renderer_descriptor_id: render.renderer_descriptor.descriptor_id.clone(),
                }),
                events::KernelEventPayload::StateAttemptCompleted(events::StateAttemptCompleted {
                    spec_hash: fixture.runtime_spec.spec_hash().clone(),
                    node_id: render_node.node_id.clone(),
                    attempt_id: attempt_id.clone(),
                    output_cell_id: render_node.output_cell.clone(),
                }),
            ],
            required_artifacts: vec![bad_receipt_evidence],
            preconditions: store::CommitPreconditions {
                required_run_state: store::RequiredRunState::NotCompleted,
                required_present_logical_keys: vec![store::LogicalEventKey::new(format!(
                    "attempt:{}:{}",
                    render_node.node_id, attempt_id
                ))
                .expect("attempt key")],
                required_cell_states: vec![store::CellStatePrecondition {
                    cell_id: render_node.output_cell.clone(),
                    required: store::RequiredCellState::Absent,
                }],
                required_public_output_absent: true,
                ..store::CommitPreconditions::default()
            },
        })
        .expect_err("forged public output rejects before replay");
    assert!(matches!(
        error,
        store::StoreError::InvalidPreparedCommitPurpose { message, .. }
            if message.contains("missing required artifact evidence")
    ));
}

#[tokio::test]
async fn recovery_interrupts_started_pure_attempt_before_retrying_fresh_attempt() {
    let fixture = fixture();
    let (scheduler, mut store) = started_fixture_run(&fixture).await;
    let node = node_by_output(&fixture, &fixture.cell_a);
    let interrupted_attempt_id = append_attempt_start(&mut store, &fixture, node, 1);

    assert_drive!(scheduler, store, fixture, Advanced, "interrupt pure");
    let projection_snapshot = store.projection_snapshot();
    let interrupted_attempt = projection_snapshot
        .attempt(&node.node_id, &interrupted_attempt_id)
        .expect("interrupted attempt");
    assert!(matches!(
        interrupted_attempt.status,
        store::AttemptStatus::Interrupted
    ));
    assert!(
        store
            .projection_snapshot()
            .cell_terminal(&fixture.cell_a)
            .is_none(),
        "interruption must not terminalize the output cell"
    );

    assert_drive!(scheduler, store, fixture, Advanced, "retry pure");
    assert_eq!(
        attempt_started_count(&store, &fixture.run_id, &node.node_id),
        2
    );
    let retry_attempt_id = attempt_id(
        &fixture.run_id,
        fixture.runtime_spec.spec_hash(),
        &node.node_id,
        2,
    )
    .expect("retry attempt id");
    match store
        .projection_snapshot()
        .cell_terminal(&fixture.cell_a)
        .expect("terminal cell")
    {
        store::CellTerminalProjection::Produced {
            attempt_id: produced_attempt,
            ..
        } => assert_eq!(produced_attempt, &retry_attempt_id),
        terminal => panic!("unexpected terminal projection: {terminal:?}"),
    }
    assert_eq!(
        runtime_lifecycle_summary(&store, &fixture.run_id),
        "run=Started attempts[started=0 completed=1 failed=0 interrupted=1 total=2] cells=1 side_effects=0 lanes[run=0 total=0] public_outputs=0 retentions=1"
    );
}

#[tokio::test]
async fn recovery_delegates_started_side_effect_attempt_to_side_effect_lifecycle() {
    let fixture = fixture_with_first_exclusive_side_effect_state();
    let (_, mut store) = started_side_effect_fixture_run(&fixture).await;

    let node = node_by_output(&fixture, &fixture.cell_a);
    let (attempt_id, _) = append_synthetic_exclusive_prepare(
        &mut store,
        &fixture,
        &fixture.run_id,
        node,
        "wallet-recovery",
        "sidefx-recovery-open",
    );
    let stream = store.load_run_stream(&fixture.run_id);
    let view = RuntimeRunView::from_stream(&fixture.runtime_spec, &fixture.run_id, &stream)
        .expect("runtime view");
    assert_eq!(
        runtime_lifecycle_summary(&store, &fixture.run_id),
        "run=Started attempts[started=1 completed=0 failed=0 interrupted=0 total=1] cells=0 side_effects=1 lanes[run=1 total=1] public_outputs=0 retentions=1"
    );

    match crate::recovery::AttemptRecoveryLifecycle::next_open_attempt_disposition(
        &fixture.runtime_spec,
        &view,
        &BTreeSet::new(),
    )
    .expect("recovery disposition")
    .expect("open side-effect attempt")
    {
        crate::recovery::OpenAttemptDisposition::DelegateSideEffect {
            node: recovered_node,
            attempt_id: recovered_attempt,
            attempt_no,
        } => {
            assert_eq!(recovered_node.node_id, node.node_id);
            assert_eq!(recovered_attempt, attempt_id);
            assert_eq!(attempt_no, 1);
        }
        crate::recovery::OpenAttemptDisposition::Continue { .. } => {
            panic!("side-effect attempt must delegate to side-effect lifecycle")
        }
        crate::recovery::OpenAttemptDisposition::Interrupt { .. }
        | crate::recovery::OpenAttemptDisposition::RetryTerminalization { .. }
        | crate::recovery::OpenAttemptDisposition::OperationalBlock { .. } => {
            panic!("side-effect attempt must delegate to side-effect lifecycle")
        }
    }
}

#[test]
fn side_effect_attempt_view_from_erased_context_is_empty_before_ledger() {
    let fixture = fixture_with_first_side_effect_state();
    with_runner_erased_ctx(&fixture, &fixture.cell_a, |ctx| {
        let view = SideEffectAttemptView::from_erased_context(&ctx).expect("side-effect view");
        assert!(view.projection().is_none());
        assert!(view.phase().is_none());
    });
}

#[tokio::test]
async fn side_effect_projection_for_attempt_exposes_ledger_state() {
    let fixture = fixture_with_first_exclusive_side_effect_state();
    let (_, mut store) = started_side_effect_fixture_run(&fixture).await;

    let node = node_by_output(&fixture, &fixture.cell_a);
    let (attempt_id, ledger_key) = append_synthetic_exclusive_prepare(
        &mut store,
        &fixture,
        &fixture.run_id,
        node,
        "wallet-view",
        "sidefx-view-open",
    );
    let loader = crate::history::VerifiedRunContextLoader::new(
        crate::binding::BoundRuntimeContextLoader::new(registered_side_effect_fixture_runners(
            &fixture,
        )),
    );
    let context = loader
        .load_async(&fixture.runtime_spec, &fixture.run_id, &store)
        .await
        .expect("verified context");

    let projection = side_effect_projection_for_attempt(
        &fixture.runtime_spec,
        context.run_id(),
        &context.view().projections,
        node,
        &attempt_id,
    )
    .expect("side-effect projection")
    .expect("projection");
    assert_eq!(projection.ledger_key, ledger_key);
    assert_eq!(
        projection.ledger_purpose,
        events::SideEffectLedgerPurpose::Forward
    );
    assert_eq!(projection.intent.attempt_id, attempt_id);
    match projection.ledger_state().expect("ledger state").phase() {
        store::SideEffectLedgerPhase::Prepared {
            claim,
            resource_key,
            ..
        } => {
            assert_eq!(claim.attempt_id, attempt_id);
            assert_eq!(claim.invocation_epoch, 1);
            assert!(resource_key.is_some());
        }
        other => panic!("unexpected side-effect phase: {other:?}"),
    }
}

#[tokio::test]
async fn recovery_sweep_includes_open_remediation_attempts() {
    let fixture = fixture_with_two_side_effects_and_failing_tail();
    let forward_a = node_by_output(&fixture, &fixture.cell_a).clone();
    let forward_b = node_by_output(&fixture, &fixture.cell_b).clone();
    let forward_a_output = effective_output_cell_for_node(&fixture, &forward_a);
    let forward_b_output = effective_output_cell_for_node(&fixture, &forward_b);
    let scheduler = compensated_saga_scheduler(&fixture);
    let mut store = started_fixture_store(&scheduler, &fixture).await;

    drive_until_cells_terminal(
        &scheduler,
        &mut store,
        &fixture,
        &[forward_a_output.clone(), forward_b_output.clone()],
        "drive forward side-effect phase",
    )
    .await;
    assert!(store
        .projection_snapshot()
        .cell_terminal(&forward_a_output)
        .is_some());
    assert!(store
        .projection_snapshot()
        .cell_terminal(&forward_b_output)
        .is_some());

    let failure_node = node_by_output(
        &fixture,
        fixture.cell_c.as_ref().expect("failing output cell"),
    )
    .clone();
    let failure_attempt = append_or_get_started_attempt(&mut store, &fixture, &failure_node, 1);
    append_attempt_failure(&mut store, &fixture, &failure_node, &failure_attempt, false);
    let remediation = fixture
        .runtime_spec
        .remediation_for_forward_node(&forward_b.node_id)
        .expect("remediation for forward b");
    let remediation_attempt = append_attempt_start(&mut store, &fixture, remediation, 1);

    let stream = store.load_run_stream(&fixture.run_id);
    let view = RuntimeRunView::from_stream(&fixture.runtime_spec, &fixture.run_id, &stream)
        .expect("runtime view");
    match crate::recovery::AttemptRecoveryLifecycle::next_open_attempt_disposition(
        &fixture.runtime_spec,
        &view,
        &BTreeSet::new(),
    )
    .expect("recovery disposition")
    .expect("open remediation attempt")
    {
        crate::recovery::OpenAttemptDisposition::Continue {
            node,
            attempt_id,
            attempt_no,
        } => {
            assert_eq!(node.node_id, remediation.node_id);
            assert_eq!(attempt_id, remediation_attempt);
            assert_eq!(attempt_no, 1);
        }
        crate::recovery::OpenAttemptDisposition::DelegateSideEffect { .. }
        | crate::recovery::OpenAttemptDisposition::Interrupt { .. }
        | crate::recovery::OpenAttemptDisposition::RetryTerminalization { .. }
        | crate::recovery::OpenAttemptDisposition::OperationalBlock { .. } => {
            panic!("remediation attempt before ledger should continue through attempt lifecycle")
        }
    }
}

#[tokio::test]
async fn recovery_interrupts_side_effect_attempts_before_prepare() {
    for (case_label, emit_claim) in [
        ("after intent before prepare", false),
        ("after claim before prepare", true),
    ] {
        let fixture = fixture_with_first_side_effect_state();
        let scheduler = test_scheduler(registered_first_side_effect_runners_with(
            &fixture,
            PrePreparedSideEffectRunner { emit_claim },
        ));
        let mut store = started_fixture_store(&scheduler, &fixture).await;
        let node = node_by_output(&fixture, &fixture.cell_a).clone();

        assert_eq!(
            drive_fixture_once(&scheduler, &mut store, &fixture)
                .await
                .expect("append pre-prepared side-effect evidence"),
            SchedulerStatus::Advanced,
            "{case_label}: expected pre-prepared side-effect evidence"
        );
        let stream = store.load_run_stream(&fixture.run_id);
        let view = RuntimeRunView::from_stream(&fixture.runtime_spec, &fixture.run_id, &stream)
            .expect("runtime view");
        let attempt_id = attempt_id(
            &fixture.run_id,
            fixture.runtime_spec.spec_hash(),
            &node.node_id,
            1,
        )
        .expect("attempt id");
        match crate::recovery::AttemptRecoveryLifecycle::next_open_attempt_disposition(
            &fixture.runtime_spec,
            &view,
            &BTreeSet::new(),
        )
        .expect("recovery disposition")
        .expect("open side-effect attempt")
        {
            crate::recovery::OpenAttemptDisposition::Interrupt {
                node: recovered_node,
                attempt_id: recovered_attempt,
                attempt_no,
            } => {
                assert_eq!(recovered_node.node_id, node.node_id);
                assert_eq!(recovered_attempt, attempt_id);
                assert_eq!(attempt_no, 1);
            }
            _ => panic!("{case_label}: pre-prepared side-effect attempt must interrupt"),
        }

        assert_eq!(
            drive_fixture_once(&scheduler, &mut store, &fixture)
                .await
                .expect("interrupt pre-prepared side-effect attempt"),
            SchedulerStatus::Advanced,
            "{case_label}: expected interrupt commit"
        );
        assert!(matches!(
            store
                .projection_snapshot()
                .attempt(&node.node_id, &attempt_id)
                .expect("attempt projection")
                .status,
            store::AttemptStatus::Interrupted
        ));
    }
}

#[tokio::test]
async fn recovery_rejects_split_terminal_cell_and_attempt_completion() {
    let fixture = fixture();
    let (scheduler, mut store) = started_fixture_run(&fixture).await;
    drive_ok!(scheduler, store, fixture, "produce first cell");
    let valid_stream = store.load_run_stream(&fixture.run_id);
    let corrupt_stream = rewrite_stream_without_payloads(&valid_stream, |payload| {
        matches!(
            payload,
            events::KernelEventPayload::StateAttemptCompleted(payload)
                if payload.output_cell_id == fixture.cell_a
        )
    });

    assert!(matches!(
        validate_runtime_stream_for_tests(&fixture.runtime_spec, &fixture.run_id, &corrupt_stream),
        Err(RuntimeError::Store(message))
            if message.contains("terminal cell requires matching attempt completion")
    ));
}

#[tokio::test]
async fn recovery_rejects_attempt_started_before_inputs_were_terminal() {
    let fixture = fixture();
    let (scheduler, mut store) = started_fixture_run(&fixture).await;
    let node_a = node_by_output(&fixture, &fixture.cell_a);
    let node_b = node_by_output(&fixture, &fixture.cell_b);
    append_attempt_start(&mut store, &fixture, node_b, 1);
    let attempt_a = append_attempt_start(&mut store, &fixture, node_a, 1);
    append_terminal(
        &mut store,
        &fixture,
        node_a,
        &attempt_a,
        artifact(0xa1),
        content(0xa2),
    );

    assert!(matches!(
        drive_fixture_once(&scheduler, &mut store, &fixture).await,
        Err(RuntimeError::InvalidRunStream(_))
    ));
}

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

#[tokio::test]
async fn recovery_allows_managed_write_artifact_restage_before_terminal_commit() {
    let fixture = fixture_with_first_managed_write_state();
    let output_artifact = artifact(0xa1);
    let output_digest = content(0xa2);
    let node = node_by_output(&fixture, &fixture.cell_a);
    let expected_caps = node
        .capability_bindings
        .capabilities
        .iter()
        .map(|capability| (capability.kind.clone(), capability.version.clone()))
        .collect();
    let registry = fixture_registry_with_first_runner(
        &fixture,
        "managed-write",
        RecordingRunner {
            expected_caps,
            output_artifact: output_artifact.clone(),
            output_digest: output_digest.clone(),
        },
    );
    let (scheduler, mut store) = started_fixture_run_with_registry(registry, &fixture).await;
    let attempt_id = append_attempt_start(&mut store, &fixture, node, 1);
    assert!(store
        .projection_snapshot()
        .cell_terminal(&fixture.cell_a)
        .is_none());

    drive_ok!(scheduler, store, fixture, "resume managed write");
    assert_eq!(
        attempt_started_count(&store, &fixture.run_id, &node.node_id),
        1
    );
    match store
        .projection_snapshot()
        .cell_terminal(&fixture.cell_a)
        .expect("terminal cell")
    {
        store::CellTerminalProjection::Produced {
            attempt_id: produced_attempt,
            ..
        } => assert_eq!(produced_attempt, &attempt_id),
        terminal => panic!("unexpected terminal projection: {terminal:?}"),
    }
}
