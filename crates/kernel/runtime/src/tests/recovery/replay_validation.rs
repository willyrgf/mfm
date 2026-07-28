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
                    attempt_id: AttemptId::from_digest(DigestBytes::from_array([0xd3; 32])),
                    claim: test_fact_claim(210, &fact_evidence),
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
