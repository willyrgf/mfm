use super::*;

#[test]
fn runner_kit_artifact_lookup_uses_exact_evidence_identity() {
    let fixture = fixture();
    let node = node_by_output(&fixture, &fixture.cell_a);
    let bytes = br#"{"amount":4}"#;
    let matching = runner_kit_value_artifact(
        bytes,
        events::ArtifactRole::StateOutput,
        Some(node.node_id.clone()),
        None,
    );
    let other_producer = NodeId::from_digest(DigestAlgorithm::Sha256JcsV1, D8);
    let mismatched_producer = runner_kit_value_artifact(
        bytes,
        events::ArtifactRole::StateOutput,
        Some(other_producer),
        None,
    );
    assert_eq!(matching.artifact_id, mismatched_producer.artifact_id);
    let matching_hash = matching.evidence_hash().expect("matching evidence hash");
    let mismatched_hash = mismatched_producer
        .evidence_hash()
        .expect("mismatched evidence hash");
    assert_ne!(matching_hash, mismatched_hash);

    let artifacts = RunnerKitArtifactProvider::new(vec![(bytes.to_vec(), matching.clone())]);
    let hit = test_event_artifact_requirement(&matching, matching_hash);
    block_on_ready(artifacts.read_retained_artifact(&hit)).expect("exact evidence hit");

    let miss = test_event_artifact_requirement(&matching, mismatched_hash);
    let error =
        block_on_ready(artifacts.read_retained_artifact(&miss)).expect_err("wrong evidence");
    assert!(matches!(error, store::StoreError::MissingArtifact { .. }));
}

fn test_event_artifact_requirement(
    evidence: &store::ArtifactEvidenceRef,
    evidence_hash: ContentDigest,
) -> store::EventArtifactRequirement {
    store::EventArtifactRequirement {
        source: store::EventArtifactReferenceSource::ArtifactReferenced,
        artifact_id: evidence.artifact_id.clone(),
        evidence_hash,
        digest: Some(evidence.digest.clone()),
        byte_len: Some(evidence.byte_len),
        media_type: Some(evidence.media_type.clone()),
        schema_id: evidence.schema_id.clone(),
        semantic_type_id: evidence.semantic_type_id.clone(),
        producer_node_id: evidence.producer_node_id.clone(),
        producer_seed_id: evidence.producer_seed_id.clone(),
        artifact_role: Some(evidence.artifact_role),
    }
}

#[test]
fn runner_kit_loads_config_and_materialized_inputs() {
    let fixture = fixture();
    let node = node_by_output(&fixture, &fixture.cell_a);
    let left_bytes = br#"{"amount":4}"#;
    let right_a_bytes = br#"{"amount":7}"#;
    let right_b_bytes = br#"{"amount":9}"#;

    let config_evidence = runner_kit_config_artifact(node, TEST_CONFIG_BYTES);
    let left_evidence = runner_kit_value_artifact(
        left_bytes,
        events::ArtifactRole::StateOutput,
        Some(node.node_id.clone()),
        None,
    );
    let right_a_evidence = runner_kit_value_artifact(
        right_a_bytes,
        events::ArtifactRole::SeedInput,
        None,
        Some(fixture.seed_ref.seed_id.clone()),
    );
    let right_b_evidence = runner_kit_value_artifact(
        right_b_bytes,
        events::ArtifactRole::StateOutput,
        Some(node.node_id.clone()),
        None,
    );
    let artifacts = RunnerKitArtifactProvider::new(vec![
        (TEST_CONFIG_BYTES.to_vec(), config_evidence),
        (left_bytes.to_vec(), left_evidence.clone()),
        (right_a_bytes.to_vec(), right_a_evidence.clone()),
        (right_b_bytes.to_vec(), right_b_evidence.clone()),
    ]);

    let config = block_on_ready(load_runner_config_for_node::<RunnerKitEmptyConfig>(
        node, &artifacts,
    ))
    .expect("runner config");
    assert_eq!(config.into_inner(), RunnerKitEmptyConfig {});

    let left_node = runner_kit_input_cell(
        &left_evidence,
        MaterializedCellTerminal::Produced {
            producer_node_id: node.node_id.clone(),
            artifact_id: left_evidence.artifact_id.clone(),
            content_digest: left_evidence.digest.clone(),
            evidence_hash: left_evidence.evidence_hash().expect("left evidence hash"),
        },
    );
    let right_a_node = runner_kit_input_cell(
        &right_a_evidence,
        MaterializedCellTerminal::Seed {
            seed_id: fixture.seed_ref.seed_id.clone(),
            artifact_id: right_a_evidence.artifact_id.clone(),
            content_digest: right_a_evidence.digest.clone(),
            evidence_hash: right_a_evidence
                .evidence_hash()
                .expect("right_a evidence hash"),
        },
    );
    let right_b_node = runner_kit_input_cell(
        &right_b_evidence,
        MaterializedCellTerminal::Produced {
            producer_node_id: node.node_id.clone(),
            artifact_id: right_b_evidence.artifact_id.clone(),
            content_digest: right_b_evidence.digest.clone(),
            evidence_hash: right_b_evidence
                .evidence_hash()
                .expect("right_b evidence hash"),
        },
    );
    let inputs = MaterializedInputs {
        input_schema_id: fixture_value_schema_id(),
        root: MaterializedInputNode::Struct(vec![
            NamedMaterializedInput {
                field_path: spec::PublicFieldPath::new("left").expect("field path"),
                node: left_node.clone(),
            },
            NamedMaterializedInput {
                field_path: spec::PublicFieldPath::new("right").expect("field path"),
                node: MaterializedInputNode::Vec(vec![right_a_node.clone(), right_b_node.clone()]),
            },
        ]),
    };

    let decoded = block_on_ready(load_materialized_struct_input::<RunnerKitStructInput>(
        &inputs, &artifacts,
    ))
    .expect("struct input");
    assert_eq!(decoded.left, CertifierValue { amount: 4 });
    assert_eq!(
        decoded.right,
        vec![CertifierValue { amount: 7 }, CertifierValue { amount: 9 }]
    );

    let left = block_on_ready(load_materialized_struct_field_value::<CertifierValue>(
        &inputs, "left", &artifacts,
    ))
    .expect("struct field");
    assert_eq!(left, CertifierValue { amount: 4 });

    let non_empty_inputs = MaterializedInputs {
        input_schema_id: fixture_value_schema_id(),
        root: MaterializedInputNode::NonEmptyVec(vec![right_a_node, right_b_node]),
    };
    let values = block_on_ready(load_non_empty_materialized_input::<CertifierValue>(
        &non_empty_inputs,
        &artifacts,
    ))
    .expect("non-empty input");
    assert_eq!(
        values.values(),
        &[CertifierValue { amount: 7 }, CertifierValue { amount: 9 }]
    );
}

#[test]
fn runner_kit_rejects_skipped_materialized_input_cell() {
    let artifacts = RunnerKitArtifactProvider::default();
    let error = block_on_ready(load_materialized_node_value::<CertifierValue>(
        &runner_kit_skipped_cell(),
        &artifacts,
    ))
    .expect_err("skipped cells cannot be loaded");

    assert!(matches!(
        error,
        RuntimeError::InvalidRunnerOutput(message) if message.contains("skipped")
    ));
}

#[test]
fn runner_kit_builders_create_context_bound_artifacts_payloads_and_output() {
    let fixture = fixture();
    let fact_descriptor_ref =
        mfm_program::fact_descriptor_ref::<RuntimeTestFact>().expect("fact descriptor ref");
    let mut node = node_by_output(&fixture, &fixture.cell_a).clone();
    node.fact_descriptor_allowlist = vec![fact_descriptor_ref.clone()];

    with_runner_erased_ctx_for_node(&fixture, &node, |ctx| {
        let artifacts = RunnerArtifactBuilder::new(&ctx);
        let payloads = RunnerPayloadBuilder::new(&ctx);
        let value = CertifierValue { amount: 42 };
        let binding = RunnerCapabilityBinding {
            capability_kind: fixture.cap_kind.clone(),
            capability_version: fixture.cap_version.clone(),
            adapter_kind: fixture.adapter_kind.clone(),
            adapter_version: fixture.adapter_version.clone(),
        };

        let state = artifacts.state_output(&value).expect("state output");
        assert_eq!(
            state.evidence().artifact_role,
            events::ArtifactRole::StateOutput
        );
        assert_eq!(
            state.evidence().producer_node_id.as_ref(),
            Some(&ctx.node().node_id)
        );
        assert_eq!(
            artifacts.content_digest(&value).expect("content digest"),
            state.evidence().digest
        );

        let cell = payloads.cell_produced(&state).expect("cell payload");
        match &cell {
            RunnerEventPayload::CellProduced(payload) => {
                assert_eq!(payload.spec_hash, *ctx.spec_hash());
                assert_eq!(payload.node_id, ctx.node().node_id);
                assert_eq!(payload.cell_id, ctx.node().output_cell);
                assert_eq!(payload.schema_id, ctx.output_cell().schema_id);
                assert_eq!(payload.semantic_type_id, ctx.output_cell().semantic_type_id);
                assert_eq!(payload.artifact_id, state.evidence().artifact_id);
                assert_eq!(payload.content_digest, state.evidence().digest);
            }
            _ => panic!("expected cell produced payload"),
        }

        let response = artifacts.fact_response(&value).expect("fact response");
        assert_eq!(
            response.evidence().artifact_role,
            events::ArtifactRole::FactResponse
        );
        assert_eq!(
            response.evidence().producer_node_id.as_ref(),
            Some(&ctx.node().node_id)
        );

        let fact = RuntimeTestFact {
            subject: CertifierValue { amount: 7 },
            response: CertifierValue { amount: 9 },
        };
        let expected_descriptor =
            <RuntimeTestFact as mfm_program::MfmFactType>::descriptor().expect("fact descriptor");
        let expected_descriptor_hash =
            mfm_facts::fact_descriptor_hash(&expected_descriptor).expect("descriptor hash");
        let expected_subject = test_fact_subject_evidence(7);
        assert_eq!(
            fact_descriptor_ref.descriptor_hash,
            expected_descriptor_hash
        );

        let mut fact_output = RunnerOutputBuilder::new(&ctx);
        fact_output
            .record_fact(
                FactRecordInput::new(
                    fact,
                    mfm_facts::FactVisibility::indexed_default(mfm_facts::FactAudience::Platform),
                )
                .observed_at("2026-01-02T03:04:05Z"),
                binding.clone(),
            )
            .expect("record typed fact");
        let fact_output = fact_output.finish();
        assert_eq!(fact_output.staged_artifacts().len(), 1);
        assert_eq!(fact_output.staged_retention_refs().len(), 0);
        assert_eq!(fact_output.payloads().len(), 1);
        let fact_artifact = &fact_output.staged_artifacts()[0];
        assert_eq!(
            fact_artifact.evidence().artifact_role,
            events::ArtifactRole::FactResponse
        );
        match &fact_output.payloads()[0] {
            RunnerEventPayload::FactRecorded(recorded) => {
                let payload = recorded.payload();
                assert_eq!(payload.spec_hash, *ctx.spec_hash());
                assert_eq!(payload.node_id, ctx.node().node_id);
                assert_eq!(payload.attempt_id, *ctx.attempt_id());
                assert_eq!(
                    payload.claim.fact_descriptor_hash(),
                    &expected_descriptor_hash
                );
                assert_eq!(
                    payload.claim.subject().fact_key(),
                    expected_subject.fact_key()
                );
                assert_eq!(payload.claim.observed_at(), Some("2026-01-02T03:04:05Z"));
                assert!(payload.claim.request().is_none());
                assert_eq!(
                    payload.claim.response().response_schema_id(),
                    &<CertifierValue as mfm_values::MfmValue>::schema_id()
                        .expect("response schema")
                );
                assert_eq!(
                    payload.claim.response().response_hash(),
                    &fact_artifact.evidence().digest
                );
                assert_eq!(
                    payload.claim.response().artifact_id(),
                    &fact_artifact.evidence().artifact_id
                );
                assert_eq!(
                    payload.claim.response().artifact_evidence_hash(),
                    &fact_artifact
                        .evidence()
                        .evidence_hash()
                        .expect("artifact evidence hash")
                );
                assert_eq!(
                    payload.claim.producer().capability_kind(),
                    &fixture.cap_kind
                );
                assert_eq!(
                    payload.claim.producer().capability_version(),
                    &fixture.cap_version
                );
                assert_eq!(
                    payload.claim.producer().adapter_kind(),
                    &fixture.adapter_kind
                );
                assert_eq!(
                    payload.claim.producer().adapter_version(),
                    &fixture.adapter_version
                );
            }
            _ => panic!("expected fact recorded payload"),
        }

        let composed_fact = RuntimeTestFact {
            subject: CertifierValue { amount: 8 },
            response: CertifierValue { amount: 10 },
        };
        let mut composed_output = RunnerOutputBuilder::new(&ctx);
        composed_output
            .state_output_and_record_fact(
                FactRecordInput::new(
                    composed_fact,
                    mfm_facts::FactVisibility::indexed_default(mfm_facts::FactAudience::Platform),
                ),
                binding.clone(),
            )
            .expect("state output and fact record");
        let composed_output = composed_output.finish();
        assert_eq!(composed_output.staged_artifacts().len(), 2);
        assert_eq!(composed_output.staged_retention_refs().len(), 1);
        assert_eq!(composed_output.payloads().len(), 2);
        assert_eq!(
            composed_output.staged_artifacts()[0]
                .evidence()
                .artifact_role,
            events::ArtifactRole::StateOutput
        );
        assert_eq!(
            composed_output.staged_artifacts()[1]
                .evidence()
                .artifact_role,
            events::ArtifactRole::FactResponse
        );
        assert!(matches!(
            composed_output.payloads()[0],
            RunnerEventPayload::FactRecorded(_)
        ));
        assert!(matches!(
            composed_output.payloads()[1],
            RunnerEventPayload::CellProduced(_)
        ));

        let query_evidence = test_fact_query_evidence();
        let expected_query_evidence_hash =
            mfm_facts::fact_query_evidence_hash(&query_evidence).expect("query evidence hash");
        let expected_query_evidence_schema =
            mfm_facts::fact_query_evidence_schema_id().expect("query evidence schema");
        let mut query_output = RunnerOutputBuilder::new(&ctx);
        query_output
            .record_fact_query_evidence(query_evidence)
            .expect("record fact query evidence");
        let query_output = query_output.finish();
        assert_eq!(query_output.staged_artifacts().len(), 1);
        assert_eq!(query_output.staged_retention_refs().len(), 1);
        assert!(query_output.payloads().is_empty());
        let query_artifact = &query_output.staged_artifacts()[0];
        assert_eq!(
            query_artifact.evidence().artifact_role,
            events::ArtifactRole::FactQueryEvidence
        );
        assert_eq!(
            query_artifact.evidence().schema_id.as_ref(),
            Some(&expected_query_evidence_schema)
        );
        assert!(query_artifact.evidence().semantic_type_id.is_none());
        assert_eq!(
            query_artifact.evidence().digest,
            expected_query_evidence_hash
        );
        let expected_query_retention = events::RetentionRef {
            artifact_id: query_artifact.evidence().artifact_id.clone(),
            role: events::ArtifactRole::FactQueryEvidence,
            evidence_hash: query_artifact
                .evidence()
                .evidence_hash()
                .expect("query evidence hash"),
            content_digest: query_artifact.evidence().digest.clone(),
        };
        assert_eq!(
            query_output.staged_retention_refs()[0].refs(),
            &[expected_query_retention]
        );

        let role_mismatch = payloads
            .cell_produced(&response)
            .expect_err("fact response cannot produce a cell");
        assert!(matches!(
            role_mismatch,
            RuntimeError::InvalidRunnerOutput(message)
                if message.contains("fact_response")
                    && message.contains("state_output")
        ));

        let mut state_output = RunnerOutputBuilder::new(&ctx);
        state_output
            .stage_attempt_artifact(&state)
            .expect("stage state output")
            .retain_runtime_evidence(&state)
            .expect("retain state output")
            .payload(cell.clone());
        let state_output = state_output.finish();
        assert_eq!(state_output.staged_artifacts().len(), 1);
        assert_eq!(state_output.staged_retention_refs().len(), 1);
        assert_eq!(state_output.payloads(), vec![cell]);
    });
}

#[tokio::test]
async fn fact_query_evidence_retains_non_empty_returned_fact_authority() {
    let fixture = fixture();
    let node = node_by_output(&fixture, &fixture.cell_a);
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let mut store = started_fixture_store(&scheduler, &fixture).await;
    let attempt_id = append_attempt_start(&mut store, &fixture, node, 1);
    let projections = store.projection_snapshot().clone();
    let (fact_ref, descriptor_projection, record_projection, index_projection, term_projections) =
        test_returned_fact_authority(&fixture, node);
    let projections = projection_snapshot_with_returned_fact_authority(
        &projections,
        descriptor_projection.clone(),
        record_projection,
        index_projection.clone(),
        term_projections,
    );
    let run_stream = store.load_run_stream(&fixture.run_id);
    let committed =
        store::CommittedRunStream::from_events(fixture.run_id.clone(), run_stream.clone())
            .expect("committed stream");
    let view = RuntimeRunView::from_committed_stream(&fixture.runtime_spec, &committed)
        .expect("runtime view");
    let view = RuntimeRunView {
        projections: projections.clone(),
        ..view
    };
    let descriptor = fixture
        .runtime_spec
        .state_descriptor_for_node(node)
        .expect("state descriptor");
    let output_cell = fixture
        .runtime_spec
        .cell(&node.output_cell)
        .expect("output cell");
    let config_artifact = config_artifact(&fixture.runtime_spec, &node.config_ref).evidence;
    let caps = CertifiedRuntimeCapabilities::for_node(node);
    let recorded_facts = RecordedFacts::default();
    let invocation = PreparedRunnerInvocation {
        runtime_spec: &fixture.runtime_spec,
        run_id: &fixture.run_id,
        spec_hash: fixture.runtime_spec.spec_hash(),
        node,
        descriptor,
        output_cell,
        context: fixture
            .runtime_spec
            .invocation_context_for_node(node)
            .expect("invocation context"),
        attempt_id: &attempt_id,
        attempt_no: 1,
        config_artifact,
        inputs: MaterializedInputs {
            input_schema_id: node.input_bindings.input_schema_id.clone(),
            root: MaterializedInputNode::Unit,
        },
        caps,
        recorded_facts,
        projections: &projections,
        run_stream: &run_stream,
        view: &view,
    };
    let ctx = ErasedRunCtx::from_prepared(&invocation);
    let output_bytes = br#"{"amount":11}"#.to_vec();
    let state_evidence =
        state_output_artifact_for_bytes(ctx.node(), ctx.descriptor(), &output_bytes);
    let state_artifact =
        StagedArtifact::inline_attempt_artifact(&ctx, output_bytes, state_evidence.clone())
            .expect("stage state output");
    let query_evidence = test_fact_query_evidence_with_returned_refs(vec![fact_ref.clone()]);
    let mut query_output = RunnerOutputBuilder::new(&ctx);
    query_output
        .record_fact_query_evidence(query_evidence)
        .expect("record query evidence");
    let query_output = query_output.finish();
    let mut staged_artifacts = vec![state_artifact];
    staged_artifacts.extend(query_output.staged_artifacts().iter().cloned());
    let mut missing_query_retention_refs = query_output.staged_retention_refs().to_vec();
    missing_query_retention_refs[0].refs =
        vec![state_evidence.retention_ref().expect("state retention ref")];
    let missing_query_output = fact_query_terminal_output(
        &ctx,
        &state_evidence,
        staged_artifacts.clone(),
        missing_query_retention_refs,
    );
    let missing_query_error =
        match prepare_runner_output_for_invocation(&invocation, missing_query_output) {
            Ok(_) => panic!("missing query evidence retention authority rejects at commit prep"),
            Err(error) => error,
        };
    assert!(matches!(
        missing_query_error,
        RuntimeError::InvalidRunnerOutput(message)
            if message.contains("missing query evidence artifact")
    ));

    let mut tampered_retention_refs = query_output.staged_retention_refs().to_vec();
    tampered_retention_refs[0]
        .refs
        .retain(|reference| reference.role != events::ArtifactRole::FactDescriptor);
    let tampered_output = fact_query_terminal_output(
        &ctx,
        &state_evidence,
        staged_artifacts.clone(),
        tampered_retention_refs,
    );
    let tampered_error = match prepare_runner_output_for_invocation(&invocation, tampered_output) {
        Ok(_) => panic!("missing descriptor retention authority rejects at commit prep"),
        Err(error) => error,
    };
    assert!(matches!(
        tampered_error,
        RuntimeError::InvalidRunnerOutput(message)
            if message.contains("missing descriptor artifact authority")
    ));

    let output = fact_query_terminal_output(
        &ctx,
        &state_evidence,
        staged_artifacts,
        query_output.staged_retention_refs().to_vec(),
    );
    let prepared = prepare_runner_output_for_invocation(&invocation, output)
        .expect("prepare runner output with returned fact query refs");
    let query_reference = prepared
        .request()
        .payloads()
        .iter()
        .find_map(|payload| match payload {
            events::KernelEventPayload::ArtifactReferenced(payload)
                if payload.artifact_ref.role == events::ArtifactRole::FactQueryEvidence =>
            {
                Some(payload)
            }
            _ => None,
        })
        .expect("fact query evidence artifact reference");
    assert_eq!(
        query_reference.artifact_ref.schema_id,
        mfm_facts::fact_query_evidence_schema_id().expect("query evidence schema")
    );
    assert!(query_reference.artifact_ref.semantic_type_id.is_none());
    let retained_refs = prepared
        .request()
        .payloads()
        .iter()
        .find_map(|payload| match payload {
            events::KernelEventPayload::RetentionRefsAppended(payload)
                if payload.reason == events::RetentionReason::RuntimeEvidence =>
            {
                Some(payload.refs.as_slice())
            }
            _ => None,
        })
        .expect("runtime evidence retention refs");

    assert!(retained_refs.contains(&events::RetentionRef {
        artifact_id: query_reference.artifact_ref.artifact_id.clone(),
        role: events::ArtifactRole::FactQueryEvidence,
        evidence_hash: prepared
            .request()
            .required_artifacts()
            .iter()
            .find(|evidence| {
                evidence.artifact_id == query_reference.artifact_ref.artifact_id
                    && evidence.digest == query_reference.artifact_ref.content_digest
            })
            .expect("query evidence admission")
            .evidence_hash()
            .expect("query evidence hash"),
        content_digest: query_reference.artifact_ref.content_digest.clone(),
    }));
    assert!(retained_refs.contains(&events::RetentionRef {
        artifact_id: descriptor_projection.descriptor_artifact_id.clone(),
        role: events::ArtifactRole::FactDescriptor,
        evidence_hash: descriptor_projection
            .descriptor_artifact_evidence
            .evidence_hash()
            .expect("descriptor evidence hash"),
        content_digest: descriptor_projection.descriptor_hash.clone(),
    }));
    assert!(retained_refs.contains(&events::RetentionRef {
        artifact_id: index_projection.artifact_id.clone(),
        role: events::ArtifactRole::FactResponse,
        evidence_hash: index_projection.artifact_evidence_hash.clone(),
        content_digest: index_projection.response_hash.clone(),
    }));
    assert_eq!(retained_refs.len(), 3);
    assert!(!prepared
        .request()
        .payloads()
        .iter()
        .any(|payload| matches!(payload, events::KernelEventPayload::FactRecorded(_))));
}
