use super::*;

#[test]
fn runner_registration_builder_preserves_explicit_binding_authority() {
    let fixture = fixture();
    let node = node_by_output(&fixture, &fixture.cell_b);
    let descriptor = fixture
        .runtime_spec
        .state_descriptor_for_node(node)
        .expect("state descriptor");
    let factory_id = events::RunnerFactoryId::new("read").expect("factory");
    let executable = events::ExecutableIdentity {
        factory_id: factory_id.clone(),
        cargo_package_digest: content(0xe1),
        binary_digest: content(0xe2),
        nix_derivation_hash: None,
        nix_output_hash: None,
    };
    let implementation_id = CapabilityImplementationId::new("mfm.test.runner-kit-registration")
        .expect("implementation id");
    let mut registry = ErasedRunnerRegistry::new();

    registry
        .register_capability_set(&node.capability_bindings, implementation_id.clone())
        .expect("capability registration");
    RunnerRegistrationBuilder::new(&mut registry)
        .register_runner(
            node.descriptor_id.clone(),
            factory_id.clone(),
            executable.clone(),
            Arc::new(RecordingRunner {
                expected_caps: vec![(fixture.cap_kind.clone(), fixture.cap_version.clone())],
                output_artifact: artifact(0xb1),
                output_digest: content(0xb2),
            }),
        )
        .expect("runner registration");

    let binding = registry
        .resolve(&fixture.runtime_spec, node, descriptor)
        .expect("registered runner");
    assert_eq!(binding.factory_id(), &factory_id);
    assert_eq!(binding.executable(), &executable);
    let capabilities = registry
        .resolve_capability_implementations(node)
        .expect("capability implementations");
    assert_eq!(
        capabilities.len(),
        node.capability_bindings.capabilities.len()
    );
    for binding in capabilities {
        assert_eq!(binding.implementation_id(), &implementation_id);
    }

    let typed_fixture = fixture_with_first_side_effect_state();
    let typed_node = node_by_output(&typed_fixture, &typed_fixture.cell_b);
    let typed_descriptor = typed_fixture
        .runtime_spec
        .state_descriptor_for_node(typed_node)
        .expect("typed state descriptor");
    let expected_descriptor =
        mfm_program::state_descriptor::<RuntimeReadState>().expect("typed descriptor");
    let typed_factory = events::RunnerFactoryId::new("read_external").expect("typed factory");
    let typed_executable = events::ExecutableIdentity {
        factory_id: typed_factory.clone(),
        cargo_package_digest: content(0xe3),
        binary_digest: content(0xe4),
        nix_derivation_hash: None,
        nix_output_hash: None,
    };
    let typed_implementation_id =
        CapabilityImplementationId::new("mfm.test.runner-kit-typed-registration")
            .expect("typed implementation id");
    let mut typed_registry = ErasedRunnerRegistry::new();
    typed_registry
        .register_capability_set(
            &typed_node.capability_bindings,
            typed_implementation_id.clone(),
        )
        .expect("typed capability registration");
    let registered_descriptor =
        mfm_program::state_descriptor::<RuntimeReadState>().expect("typed descriptor");
    RunnerRegistrationBuilder::new(&mut typed_registry)
        .register_runner(
            registered_descriptor.descriptor_id().clone(),
            typed_factory.clone(),
            typed_executable.clone(),
            Arc::new(RecordingRunner {
                expected_caps: vec![(
                    typed_fixture.cap_kind.clone(),
                    typed_fixture.cap_version.clone(),
                )],
                output_artifact: artifact(0xd1),
                output_digest: content(0xd2),
            }),
        )
        .expect("typed runner registration");
    assert_eq!(
        registered_descriptor.descriptor_id(),
        expected_descriptor.descriptor_id()
    );
    assert_eq!(
        &typed_descriptor.descriptor_id,
        expected_descriptor.descriptor_id()
    );
    let typed_binding = typed_registry
        .resolve(&typed_fixture.runtime_spec, typed_node, typed_descriptor)
        .expect("registered typed runner");
    assert_eq!(typed_binding.factory_id(), &typed_factory);
    assert_eq!(typed_binding.executable(), &typed_executable);
    let typed_capabilities = typed_registry
        .resolve_capability_implementations(typed_node)
        .expect("typed capability implementations");
    assert_eq!(
        typed_capabilities.len(),
        typed_node.capability_bindings.capabilities.len()
    );
    for binding in typed_capabilities {
        assert_eq!(binding.implementation_id(), &typed_implementation_id);
    }

    let wrong_factory = events::RunnerFactoryId::new("pure").expect("factory");
    let mut mismatch_registry = ErasedRunnerRegistry::new();
    let error = match RunnerRegistrationBuilder::new(&mut mismatch_registry).register_runner(
        node.descriptor_id.clone(),
        factory_id,
        events::ExecutableIdentity {
            factory_id: wrong_factory,
            ..executable
        },
        Arc::new(RecordingRunner {
            expected_caps: Vec::new(),
            output_artifact: artifact(0xc1),
            output_digest: content(0xc2),
        }),
    ) {
        Ok(_) => panic!("factory mismatch should be rejected"),
        Err(error) => error,
    };
    assert!(matches!(
        error,
        RuntimeError::RunnerBinding(message)
            if message.contains("does not match binding factory")
    ));
}

#[tokio::test]
async fn launch_rejects_context_bound_output_runner_without_extractor() {
    let fixture = fixture_with_context_bound_states();
    let mut registry = ErasedRunnerRegistry::new();
    register_spec_capabilities(&mut registry, &fixture.runtime_spec);
    registry
        .register(binding(
            fixture.descriptor_a.clone(),
            "pure",
            RecordingRunner {
                expected_caps: Vec::new(),
                output_artifact: artifact(0xa1),
                output_digest: content(0xa2),
            },
        ))
        .expect("source binding");
    registry
        .register(binding(
            fixture.descriptor_b.clone(),
            "pure",
            ContextConsumerRunner,
        ))
        .expect("consumer binding");
    let scheduler = test_scheduler(registry);
    let store = TestTypedRunStore::new();

    let error = match prepare_fixture_launch(
        &scheduler,
        &store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    ) {
        Ok(_) => panic!("context-bound output runner without extractor must reject launch"),
        Err(error) => error,
    };
    assert!(matches!(
        error,
        RuntimeError::RunnerBinding(message)
            if message.contains("context-bound output cell")
                && message.contains("context output extractor")
    ));
}

#[tokio::test]
async fn context_bound_output_rejects_mismatched_artifact_or_payload_context() {
    enum Case {
        ArtifactContext,
        PayloadContext,
    }

    for case in [Case::ArtifactContext, Case::PayloadContext] {
        let fixture = fixture_with_context_bound_states();
        let source_runner = match case {
            Case::ArtifactContext => {
                let wrong_stage = ContextStage::new("wrong_stage").expect("wrong stage");
                ContextSourceRunner::with_stage(wrong_stage)
            }
            Case::PayloadContext => {
                ContextSourceRunner::with_payload_context(spec::CellContextSpec::no_context())
            }
        };
        let registry = registered_context_bound_fixture_runners(&fixture, source_runner);
        let (scheduler, mut store) = started_fixture_run_with_registry(registry, &fixture).await;

        assert_first_node_invalid_after_drive!(
            scheduler,
            store,
            fixture,
            "terminalize mismatched context-bound output"
        );
    }
}

#[tokio::test]
async fn materialization_rejects_context_bound_input_under_wrong_node_context() {
    let mut fixture = fixture_with_context_bound_states();
    let mut envelope = fixture.runtime_spec.envelope().clone();
    let consumer = envelope
        .spec
        .nodes
        .iter_mut()
        .find(|node| node.descriptor_id == fixture.descriptor_b)
        .expect("consumer node");
    consumer.context = spec::NodeContextSpec::no_context();
    let envelope = spec::HashedSpecEnvelope::new(envelope.spec, envelope.audit).expect("rehash");
    fixture.runtime_spec =
        CertifiedRuntimeSpec::from_verified_envelope(envelope).expect("tampered runtime spec");
    refresh_fixture_run_id(&mut fixture);

    let registry = registered_context_bound_fixture_runners(&fixture, ContextSourceRunner::new());
    let (scheduler, mut store) = started_fixture_run_with_registry(registry, &fixture).await;
    assert_drive!(
        scheduler,
        store,
        fixture,
        Advanced,
        "produce context-bound input"
    );
    assert_drive!(
        scheduler,
        store,
        fixture,
        Advanced,
        "terminalize context input materialization failure"
    );
    let consumer = node_by_output(&fixture, &fixture.cell_b);
    assert_node_failed_with_code_and_retryable(
        &store,
        &consumer.node_id,
        "input_materialization_failed",
        true,
    );
    assert!(store
        .projection_snapshot()
        .cell_terminal(&consumer.output_cell)
        .is_none());
}

#[tokio::test]
async fn serial_scheduler_runs_nodes_in_certified_topological_order() {
    let fixture = fixture();
    let (scheduler, mut store) = started_fixture_run(&fixture).await;

    assert_drive!(scheduler, store, fixture, Advanced, "drive a");
    assert!(store
        .projection_snapshot()
        .cell_terminal(&fixture.cell_a)
        .is_some());
    assert!(store
        .projection_snapshot()
        .cell_terminal(&fixture.cell_b)
        .is_none());
    assert_drive!(scheduler, store, fixture, Advanced, "drive b");
    assert!(store
        .projection_snapshot()
        .cell_terminal(&fixture.cell_b)
        .is_some());
    assert_eq!(
        runtime_lifecycle_summary(&store, &fixture.run_id),
        "run=Started attempts[started=0 completed=2 failed=0 interrupted=0 total=2] cells=2 side_effects=0 lanes[run=0 total=0] public_outputs=0 retentions=1"
    );
}

#[tokio::test]
async fn scheduler_completes_run_after_public_output_evidence() {
    let fixture = fixture();
    let (scheduler, mut store) = started_fixture_run(&fixture).await;

    assert_eq!(
        drive_fixture_until_blocked(&scheduler, &mut store, &fixture)
            .await
            .expect("drive to public output"),
        SchedulerStatus::PublicOutputProjected
    );
    assert_eq!(
        store.projection_snapshot().run_state(&fixture.run_id),
        store::RunState::Completed
    );
    assert!(store
        .projection_snapshot()
        .cell_terminal(&fixture.render_cell)
        .is_some());
    assert_eq!(
        runtime_lifecycle_summary(&store, &fixture.run_id),
        "run=Completed attempts[started=0 completed=5 failed=0 interrupted=0 total=5] cells=5 side_effects=0 lanes[run=0 total=0] public_outputs=1 retentions=1"
    );

    let stream = store.load_run_stream(&fixture.run_id);
    let public_output_pos = stream
        .iter()
        .position(|event| {
            matches!(
                event.payload(),
                events::KernelEventPayload::PublicOutputProduced(_)
            )
        })
        .expect("public output produced");
    let completed_pos = stream
        .iter()
        .position(|event| matches!(event.payload(), events::KernelEventPayload::RunCompleted(_)))
        .expect("run completed");
    assert!(public_output_pos < completed_pos);

    let public_event = &stream[public_output_pos];
    let public_payload = match public_event.payload() {
        events::KernelEventPayload::PublicOutputProduced(payload) => payload,
        _ => unreachable!("checked above"),
    };
    assert_eq!(public_payload.node_id, fixture.render_node);
    assert_eq!(public_payload.receipt_cell_id, fixture.render_cell);
    assert!(public_payload.rendered_artifact_id.is_none());

    let completed_payload = match stream[completed_pos].payload() {
        events::KernelEventPayload::RunCompleted(payload) => payload,
        _ => unreachable!("checked above"),
    };
    assert_eq!(
        completed_payload.outcome,
        events::RunCompletionOutcome::Completed(Box::new(events::PublicOutputCompletionEvidence {
            public_output_schema_id: fixture
                .runtime_spec
                .spec()
                .public_outputs
                .public_schema_id
                .clone(),
            public_output_event_id: public_event.event_id().clone(),
        }))
    );
    assert_eq!(completed_pos, stream.len() - 1);

    let complete_node = certified_complete_run_node(&fixture.runtime_spec).expect("complete node");
    let completion_seq = stream[completed_pos].seq();
    let completion_key = stream[completed_pos].commit_key().clone();
    let completion_commit = stream
        .iter()
        .filter(|event| event.seq() == completion_seq && event.commit_key() == &completion_key)
        .collect::<Vec<_>>();
    assert_eq!(completion_commit.len(), 4);
    let complete_attempt = stream
        .iter()
        .find_map(|event| match event.payload() {
            events::KernelEventPayload::StateAttemptStarted(payload)
                if payload.node_id == complete_node.node_id && event.seq() < completion_seq =>
            {
                Some(payload.attempt_id.clone())
            }
            _ => None,
        })
        .expect("complete attempt started");
    let complete_receipt = completion_commit
        .iter()
        .find_map(|event| match event.payload() {
            events::KernelEventPayload::CellProduced(payload)
                if payload.node_id == complete_node.node_id =>
            {
                Some(payload)
            }
            _ => None,
        })
        .expect("complete receipt cell");
    assert_eq!(complete_receipt.attempt_id, complete_attempt);
    assert_eq!(complete_receipt.cell_id, complete_node.output_cell);
    assert!(completion_commit.iter().any(|event| {
        matches!(
            event.payload(),
            events::KernelEventPayload::StateAttemptCompleted(payload)
                if payload.node_id == complete_node.node_id
                    && payload.attempt_id == complete_attempt
                    && payload.output_cell_id == complete_node.output_cell
        )
    }));
    assert!(completion_commit.iter().any(|event| {
        matches!(
            event.payload(),
            events::KernelEventPayload::ArtifactReferenced(payload)
                if payload.node_id.as_ref() == Some(&complete_node.node_id)
                    && payload.attempt_id.as_ref() == Some(&complete_attempt)
                    && payload.artifact_ref.artifact_id == complete_receipt.artifact_id
                    && payload.artifact_ref.content_digest == complete_receipt.content_digest
                    && payload.artifact_ref.role == events::ArtifactRole::StateOutput
        )
    }));
    let stream_len = stream.len();
    assert_drive!(
        scheduler,
        store,
        fixture,
        PublicOutputProjected,
        "drive completed run"
    );
    store.assert_run_stream_len(&fixture.run_id, stream_len);
}

#[tokio::test]
async fn no_second_authority_full_run_stages_and_admits_first_artifact_references() {
    struct InlineRecordingRunner {
        expected_caps: Vec<(CapabilityKind, CapabilityVersion)>,
        output_bytes: Vec<u8>,
    }

    impl ErasedNodeRunner for InlineRecordingRunner {
        fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
            Box::pin(async move {
                for (kind, version) in &self.expected_caps {
                    assert!(ctx.caps().contains(kind, version));
                }
                let artifact = state_output_artifact_for_bytes(
                    ctx.node(),
                    ctx.descriptor(),
                    &self.output_bytes,
                );
                let staged_artifact = StagedArtifact::inline_attempt_artifact(
                    &ctx,
                    self.output_bytes.clone(),
                    artifact.clone(),
                )?;
                Ok(ErasedRunnerOutput::from_parts(
                    vec![staged_artifact],
                    Vec::new(),
                    terminal_payloads(&ctx, &artifact),
                ))
            })
        }
    }

    let fixture = fixture();
    let mut registry = ErasedRunnerRegistry::new();
    registry
        .register(binding(
            fixture.descriptor_a.clone(),
            "pure",
            InlineRecordingRunner {
                expected_caps: Vec::new(),
                output_bytes: br#"{"node":"a"}"#.to_vec(),
            },
        ))
        .expect("binding a");
    registry
        .register(binding(
            fixture.descriptor_b.clone(),
            "read",
            InlineRecordingRunner {
                expected_caps: vec![(fixture.cap_kind.clone(), fixture.cap_version.clone())],
                output_bytes: br#"{"node":"b"}"#.to_vec(),
            },
        ))
        .expect("binding b");
    let scheduler = test_scheduler_with_artifacts(
        register_fixture_capabilities(registry, &fixture),
        Arc::new(TestRuntimeArtifactStore {
            artifacts: Arc::new(Mutex::new(BTreeMap::new())),
        }),
    );
    let store = RecordingTypedRunStore::new();
    start_fixture_run_async_store(&scheduler, &store, &fixture, vec![fixture.seed_ref.clone()])
        .await
        .expect("start run");

    assert_eq!(
        drive_until_blocked_with_claim(&scheduler, &store, &fixture.runtime_spec, &fixture.run_id)
            .await
            .expect("drive full representative run"),
        SchedulerStatus::PublicOutputProjected
    );
    assert_eq!(
        store
            .projection_snapshot(&fixture.run_id)
            .await
            .run_state(&fixture.run_id),
        store::RunState::Completed
    );

    let stream = store.load_run_stream(&fixture.run_id).await;
    validate_runtime_stream_for_tests(&fixture.runtime_spec, &fixture.run_id, &stream)
        .expect("representative stream validates");
    assert_every_certified_node_has_attempt(&fixture.runtime_spec, &stream);
    assert!(node_by_output(&fixture, &fixture.cell_a)
        .framework
        .is_none());
    assert!(matches!(
        &node_by_output(&fixture, &fixture.render_cell).framework,
        Some(spec::FrameworkNodeSpec::PublicOutputRender(_))
    ));
    assert!(matches!(
        &certified_retention_manifest_node(&fixture.runtime_spec)
            .expect("retention node")
            .framework,
        Some(spec::FrameworkNodeSpec::ProjectRetentionManifest(_))
    ));
    assert!(matches!(
        &certified_complete_run_node(&fixture.runtime_spec)
            .expect("complete node")
            .framework,
        Some(spec::FrameworkNodeSpec::CompleteRun(_))
    ));

    let mut first_reference_by_artifact = BTreeMap::<ArtifactId, usize>::new();
    let commits = store.commits();
    for (commit_index, commit) in commits.iter().enumerate() {
        let commit_references = commit
            .payloads
            .iter()
            .flat_map(referenced_artifact_ids_for_payload)
            .collect::<BTreeSet<_>>();
        assert!(
            !commit_references.is_empty() || commit.admitted_artifacts.is_empty(),
            "commit {} admitted artifacts without same-commit references",
            commit.commit_key
        );
        for admitted in &commit.admitted_artifacts {
            assert!(
                commit_references.contains(&admitted.artifact_id),
                "commit {} admitted unreferenced artifact {}",
                commit.commit_key,
                admitted.artifact_id
            );
        }
        for artifact_id in commit_references {
            first_reference_by_artifact
                .entry(artifact_id)
                .or_insert(commit_index);
        }
    }

    assert!(
        !first_reference_by_artifact.is_empty(),
        "representative run should reference artifacts"
    );
    for (artifact_id, commit_index) in first_reference_by_artifact {
        let commit = &commits[commit_index];
        assert!(
            commit
                .admitted_artifacts
                .iter()
                .any(|admitted| admitted.artifact_id == artifact_id),
            "artifact {artifact_id} was first referenced by {} at seq {} but not admitted there",
            commit.commit_key,
            commit.seq.as_u64()
        );
    }
}

#[tokio::test]
async fn run_launch_commits_single_admission_root_and_admits_launch_artifacts() {
    let fixture = fixture();
    let scheduler = test_scheduler_with_artifacts(
        registered_fixture_runners(&fixture),
        Arc::new(TestRuntimeArtifactStore {
            artifacts: Arc::new(Mutex::new(BTreeMap::new())),
        }),
    );
    let store = started_fixture_store(&scheduler, &fixture).await;

    let stream = store.load_run_stream(&fixture.run_id);
    let first = stream.first().expect("stream event");
    let admission_commit = stream
        .iter()
        .take_while(|event| event.seq() == first.seq() && event.commit_key() == first.commit_key())
        .collect::<Vec<_>>();
    assert_eq!(admission_commit.len(), 1);
    assert_eq!(admission_commit[0].ordinal(), store::CommitOrdinal::new(0));

    let run_admitted = match admission_commit[0].payload() {
        events::KernelEventPayload::RunAdmitted(payload) => payload,
        _ => panic!("first admission event must be RunAdmitted"),
    };
    assert_eq!(
        run_admitted.spec_artifact.role,
        events::ArtifactRole::TypedExecutionSpec
    );
    assert_eq!(
        run_admitted.certificate_artifact.role,
        events::ArtifactRole::TypedSpecCertificate
    );
    assert!(run_admitted
        .config_artifacts
        .iter()
        .all(|artifact| artifact.role == events::ArtifactRole::TypedConfig));
    assert_eq!(
        fixture.seed_ref.seed_artifact.role,
        events::ArtifactRole::SeedInput
    );
    assert!(stream.iter().all(|event| {
        !matches!(
            event.payload(),
            events::KernelEventPayload::StateAttemptStarted(_)
                | events::KernelEventPayload::StateAttemptCompleted(_)
                | events::KernelEventPayload::CellProduced(_)
                | events::KernelEventPayload::ArtifactReferenced(_)
                | events::KernelEventPayload::RetentionRefsAppended(_)
        )
    }));
}

#[tokio::test]
async fn scheduler_binds_staged_retention_refs_and_projects_manifest() {
    let fixture = fixture_with_retention_lifecycle_node();
    let (scheduler, mut store) = started_fixture_run(&fixture).await;

    let projection_snapshot = store.projection_snapshot();
    let start_retention = projection_snapshot
        .retention(&fixture.run_id)
        .expect("run-start retention");
    assert!(start_retention
        .refs
        .keys()
        .any(|(artifact_id, _)| artifact_id == &fixture.seed_ref.seed_artifact.artifact_id));

    let status = drive_fixture_until_blocked(&scheduler, &mut store, &fixture)
        .await
        .expect("drive to completion");
    assert_eq!(status, SchedulerStatus::PublicOutputProjected);
    let render_receipt_artifact = match store
        .projection_snapshot()
        .cell_terminal(&fixture.render_cell)
        .expect("render receipt cell")
    {
        store::CellTerminalProjection::Produced { artifact_id, .. } => artifact_id.clone(),
        terminal => panic!("unexpected render terminal: {terminal:?}"),
    };
    assert!(store
        .projection_snapshot()
        .retention(&fixture.run_id)
        .expect("runtime retention")
        .refs
        .keys()
        .any(|(artifact_id, _)| artifact_id == &render_receipt_artifact));

    let projection_snapshot = store.projection_snapshot();
    let projection = projection_snapshot
        .retention(&fixture.run_id)
        .expect("retention projection");
    let manifest = projection.manifest.as_ref().expect("manifest");
    assert_eq!(manifest.manifest_seq, 1);
    assert!(projection
        .refs
        .keys()
        .any(|(artifact_id, _)| artifact_id == &manifest.manifest_artifact_id));

    let retention_node =
        certified_retention_manifest_node(&fixture.runtime_spec).expect("retention node");
    let stream = store.load_run_stream(&fixture.run_id);
    let manifest_event = stream
        .iter()
        .find_map(|event| match event.payload() {
            events::KernelEventPayload::RetentionManifestProjected(payload)
                if payload.manifest_artifact_id == manifest.manifest_artifact_id =>
            {
                Some((event.seq(), payload))
            }
            _ => None,
        })
        .expect("manifest event");
    let retention_seq = manifest_event.0;
    assert_eq!(manifest_event.1.manifest_digest, manifest.manifest_digest);
    let receipt = stream
        .iter()
        .find_map(|event| match event.payload() {
            events::KernelEventPayload::CellProduced(payload)
                if event.seq() == retention_seq && payload.node_id == retention_node.node_id =>
            {
                Some(payload)
            }
            _ => None,
        })
        .expect("retention receipt cell");
    assert_eq!(receipt.cell_id, retention_node.output_cell);
    assert!(stream.iter().any(|event| {
        matches!(
            event.payload(),
            events::KernelEventPayload::StateAttemptStarted(payload)
                if event.seq() < retention_seq
                    && payload.node_id == retention_node.node_id
                    && payload.attempt_id == receipt.attempt_id
        )
    }));
    assert!(stream.iter().any(|event| {
        matches!(
            event.payload(),
            events::KernelEventPayload::StateAttemptCompleted(payload)
                if event.seq() == retention_seq
                    && payload.node_id == retention_node.node_id
                    && payload.attempt_id == receipt.attempt_id
                    && payload.output_cell_id == retention_node.output_cell
        )
    }));
    let manifest_ref = stream
        .iter()
        .find_map(|event| match event.payload() {
            events::KernelEventPayload::RetentionRefsAppended(payload)
                if event.seq() == retention_seq
                    && payload.reason == events::RetentionReason::ManifestProjection =>
            {
                payload.refs.first()
            }
            _ => None,
        })
        .expect("manifest retention ref");
    assert_eq!(manifest_ref.artifact_id, manifest.manifest_artifact_id);
    assert_eq!(manifest_ref.content_digest, manifest.manifest_digest);
    assert_eq!(manifest_ref.role, events::ArtifactRole::RetentionManifest);
}

#[tokio::test]
async fn retention_manifest_projection_retry_is_idempotent_after_current_store_advanced() {
    let fixture = fixture_with_retention_lifecycle_node();
    let (scheduler, mut store) = started_fixture_run(&fixture).await;

    drive_until_public_output_produced(&scheduler, &mut store, &fixture).await;
    let retention_node =
        certified_retention_manifest_node(&fixture.runtime_spec).expect("retention node");
    append_attempt_start(&mut store, &fixture, retention_node, 1);
    let stale_stream = store.load_run_stream(&fixture.run_id);

    drive_ok!(
        scheduler,
        store,
        fixture,
        "advance current store with retention projection"
    );
    assert!(store
        .projection_snapshot()
        .retention(&fixture.run_id)
        .and_then(|retention| retention.manifest.as_ref())
        .is_some());

    let stale_store = StaleStreamStore::new(&mut store, stale_stream);
    assert_eq!(
        drive_once_with_claim(
            &scheduler,
            &stale_store,
            &fixture.runtime_spec,
            &fixture.run_id
        )
        .await
        .expect("idempotent retention retry"),
        SchedulerStatus::Advanced
    );
}

#[tokio::test]
async fn runtime_rejects_standalone_retention_manifest_projection_history() {
    let fixture = fixture_with_retention_lifecycle_node();
    let (scheduler, mut store) = started_fixture_run(&fixture).await;
    drive_until_public_output_produced(&scheduler, &mut store, &fixture).await;

    let manifest = build_retention_manifest_artifact(
        &fixture.runtime_spec,
        &fixture.run_id,
        &store.load_run_stream(&fixture.run_id),
        &store::ArtifactByteAuthorityMap::new(),
    )
    .expect("manifest");
    let manifest_evidence = manifest.evidence.clone();
    let request = store_typed_commit_request! {
        run_id: fixture.run_id.clone(),
        expected_next_seq: store.expected_next_seq(&fixture.run_id),
        commit_key: store::CommitKey::new("synthetic/standalone-retention-projection")
            .expect("commit key"),
        payloads: retention_manifest_payloads(&fixture.runtime_spec, &fixture.run_id, manifest)
            .expect("retention manifest payloads"),
        required_artifacts: vec![manifest_evidence],
        preconditions: store::CommitPreconditions::default(),
    };
    store
        .append_prepared_commit(request)
        .expect("synthetic standalone projection");

    assert!(matches!(
        RuntimeRunView::from_stream(
            &fixture.runtime_spec,
            &fixture.run_id,
            &store.load_run_stream(&fixture.run_id)
        ),
        Err(RuntimeError::InvalidRunStream(message))
            if message.contains("retention manifest projection was not produced")
    ));
}

#[tokio::test]
async fn runtime_rejects_complete_run_receipt_commit_without_run_completed() {
    let fixture = fixture();
    let (scheduler, mut store) = started_fixture_run(&fixture).await;
    drive_fixture_until_blocked(&scheduler, &mut store, &fixture)
        .await
        .expect("drive to completion");

    let valid_stream = store.load_run_stream(&fixture.run_id);
    let corrupt_stream = rewrite_stream_without_payloads(&valid_stream, |payload| {
        matches!(payload, events::KernelEventPayload::RunCompleted(_))
    });

    assert!(matches!(
        validate_runtime_stream_for_tests(&fixture.runtime_spec, &fixture.run_id, &corrupt_stream),
        Err(RuntimeError::InvalidRunStream(message))
            if message.contains("missing RunCompleted")
    ));
}

#[tokio::test]
async fn public_output_render_failure_resumes_and_completes() {
    let fixture = fixture();
    let (scheduler, mut store) = started_fixture_run(&fixture).await;
    drive_ok!(scheduler, store, fixture, "drive a");
    drive_ok!(scheduler, store, fixture, "drive b");
    let render_node = node_by_output(&fixture, &fixture.render_cell);
    let failed_attempt = append_attempt_start(&mut store, &fixture, render_node, 1);
    append_public_output_render_failure(&mut store, &fixture, render_node, &failed_attempt);
    assert!(matches!(
        store.projection_snapshot().public_output(
            &fixture.run_id,
            &fixture.runtime_spec.spec().public_outputs.public_schema_id,
        ),
        Some(store::PublicOutputProjection::RenderFailed { .. })
    ));

    assert_eq!(
        drive_fixture_until_blocked(&scheduler, &mut store, &fixture)
            .await
            .expect("retry render"),
        SchedulerStatus::PublicOutputProjected
    );
    assert_eq!(
        attempt_started_count(&store, &fixture.run_id, &fixture.render_node),
        2
    );
    assert_eq!(
        store.projection_snapshot().run_state(&fixture.run_id),
        store::RunState::Completed
    );
    assert!(matches!(
        store.projection_snapshot().public_output(
            &fixture.run_id,
            &fixture.runtime_spec.spec().public_outputs.public_schema_id,
        ),
        Some(store::PublicOutputProjection::Produced { .. })
    ));
}

#[test]
fn certified_runtime_spec_rejects_hash_mismatch() {
    let fixture = fixture();
    let mut envelope = fixture.runtime_spec.envelope().clone();
    envelope.spec_hash = SpecHash::from_digest(DigestAlgorithm::Sha256JcsV1, D9);
    assert!(matches!(
        CertifiedRuntimeSpec::from_verified_envelope(envelope),
        Err(RuntimeError::SpecHash(_))
    ));
}

#[test]
fn certified_runtime_spec_accepts_certifier_and_verified_persisted_authority() {
    let (certified, registry) = certifier_backed_runtime_authority();
    let persisted_parts = certified
        .to_persisted_parts()
        .expect("persisted spec/certificate parts");
    let verified = mfm_certify::verify_persisted_spec_certificate(
        persisted_parts.spec_bytes(),
        persisted_parts.certificate_bytes(),
        &registry,
    )
    .expect("verified persisted spec/certificate");
    for (source, authority) in [
        ("certifier authority", certified),
        ("verified persisted parts authority", verified),
    ] {
        let runtime = CertifiedRuntimeSpec::new(authority).expect(source);
        assert!(!runtime.topological_order().is_empty(), "{source}");
    }
}

#[tokio::test]
async fn replay_rejects_run_completed_without_public_output_evidence() {
    let fixture = fixture();
    let (scheduler, mut store) = started_fixture_run(&fixture).await;
    store
        .append_prepared_commit(store_typed_commit_request! {
            run_id: fixture.run_id.clone(),
            expected_next_seq: store.expected_next_seq(&fixture.run_id),
            commit_key: store::CommitKey::new("forged-complete-without-public-output")
                .expect("commit key"),
            payloads: vec![events::KernelEventPayload::RunCompleted(
                events::RunCompleted {
                    run_id: fixture.run_id.clone(),
                    spec_hash: fixture.runtime_spec.spec_hash().clone(),
                    outcome: events::RunCompletionOutcome::Completed(Box::new(
                        events::PublicOutputCompletionEvidence {
                            public_output_schema_id: fixture
                                .runtime_spec
                                .spec()
                                .public_outputs
                                .public_schema_id
                                .clone(),
                            public_output_event_id: EventId::from_digest(
                                DigestAlgorithm::Sha256JcsV1,
                                D9,
                            ),
                        },
                    )),
                },
            )],
            required_artifacts: Vec::new(),
            preconditions: store::CommitPreconditions {
                required_run_state: store::RequiredRunState::NotCompleted,
                ..store::CommitPreconditions::default()
            },
        })
        .expect("append forged completion");

    assert!(matches!(
        drive_fixture_once(&scheduler, &mut store, &fixture)
            .await,
        Err(RuntimeError::InvalidRunStream(message))
            if message.contains("RunCompleted appeared before PublicOutputProduced")
    ));
}

#[tokio::test]
async fn scheduler_rejects_uncertified_capability_use() {
    struct BadFactRunner {
        cap_kind: CapabilityKind,
        cap_version: CapabilityVersion,
    }

    impl ErasedNodeRunner for BadFactRunner {
        fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
            Box::pin(async move {
                let (response_evidence, _response_bytes) =
                    test_fact_response_artifact(ctx.node(), 194);
                Ok(ErasedRunnerOutput::new(vec![
                    RunnerEventPayload::FactRecorded(RunnerFactRecorded::new(
                        events::FactRecorded {
                            spec_hash: ctx.spec_hash().clone(),
                            node_id: ctx.node().node_id.clone(),
                            attempt_id: ctx.attempt_id().clone(),
                            claim: test_fact_claim(
                                196,
                                ctx.node().config_ref.schema_id.clone(),
                                content(0xc1),
                                &response_evidence,
                                self.cap_kind.clone(),
                                self.cap_version.clone(),
                                AdapterKind::new(
                                    "mfm.test",
                                    "adapter",
                                    DigestAlgorithm::Sha256JcsV1,
                                    D1,
                                )
                                .expect("adapter"),
                                AdapterVersion::new("mfm.adapter.v1").expect("adapter version"),
                            ),
                        },
                    )),
                ]))
            })
        }
    }

    let fixture = fixture();
    let registry = fixture_registry_with_first_runner(
        &fixture,
        "pure",
        BadFactRunner {
            cap_kind: fixture.cap_kind.clone(),
            cap_version: fixture.cap_version.clone(),
        },
    );
    let (scheduler, mut store) = started_fixture_run_with_registry(registry, &fixture).await;
    assert_first_node_invalid_after_drive!(
        scheduler,
        store,
        fixture,
        "terminalize uncertified capability use"
    );
}

#[tokio::test]
async fn runner_rejects_invalid_artifact_outputs() {
    #[derive(Clone, Copy)]
    enum InvalidArtifactOutputKind {
        ForeignProducer,
        MismatchedInlineBytes,
        MissingStagedArtifact,
    }

    impl InvalidArtifactOutputKind {
        fn label(self) -> &'static str {
            match self {
                Self::ForeignProducer => "foreign producer artifact",
                Self::MismatchedInlineBytes => "mismatched inline artifact",
                Self::MissingStagedArtifact => "missing staged artifact",
            }
        }
    }

    enum InvalidArtifactOutput {
        ForeignProducer { foreign_node_id: NodeId },
        MismatchedInlineBytes,
        MissingStagedArtifact,
    }

    struct InvalidArtifactRunner {
        case: InvalidArtifactOutput,
        output_artifact: ArtifactId,
        output_digest: ContentDigest,
    }

    impl ErasedNodeRunner for InvalidArtifactRunner {
        fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
            Box::pin(async move {
                let staged_artifacts = match &self.case {
                    InvalidArtifactOutput::ForeignProducer { foreign_node_id } => {
                        let mut artifact = state_output_artifact(
                            ctx.node(),
                            ctx.descriptor(),
                            self.output_artifact.clone(),
                            self.output_digest.clone(),
                        );
                        artifact.producer_node_id = Some(foreign_node_id.clone());
                        vec![staged_attempt_artifact(&ctx, artifact)?]
                    }
                    InvalidArtifactOutput::MismatchedInlineBytes => {
                        let artifact = state_output_artifact(
                            ctx.node(),
                            ctx.descriptor(),
                            self.output_artifact.clone(),
                            self.output_digest.clone(),
                        );
                        vec![StagedArtifact::inline_attempt_artifact(
                            &ctx,
                            b"mismatched".to_vec(),
                            artifact,
                        )?]
                    }
                    InvalidArtifactOutput::MissingStagedArtifact => Vec::new(),
                };
                let payload_evidence = state_output_artifact(
                    ctx.node(),
                    ctx.descriptor(),
                    self.output_artifact.clone(),
                    self.output_digest.clone(),
                );
                Ok(ErasedRunnerOutput::from_parts(
                    staged_artifacts,
                    Vec::new(),
                    terminal_payloads(&ctx, &payload_evidence),
                ))
            })
        }
    }

    for kind in [
        InvalidArtifactOutputKind::ForeignProducer,
        InvalidArtifactOutputKind::MismatchedInlineBytes,
        InvalidArtifactOutputKind::MissingStagedArtifact,
    ] {
        let fixture = fixture();
        let case = match kind {
            InvalidArtifactOutputKind::ForeignProducer => InvalidArtifactOutput::ForeignProducer {
                foreign_node_id: node_by_output(&fixture, &fixture.cell_b).node_id.clone(),
            },
            InvalidArtifactOutputKind::MismatchedInlineBytes => {
                InvalidArtifactOutput::MismatchedInlineBytes
            }
            InvalidArtifactOutputKind::MissingStagedArtifact => {
                InvalidArtifactOutput::MissingStagedArtifact
            }
        };
        let registry = fixture_registry_with_first_runner(
            &fixture,
            "pure",
            InvalidArtifactRunner {
                case,
                output_artifact: artifact(0xa1),
                output_digest: content(0xa2),
            },
        );
        let (scheduler, mut store) = started_fixture_run_with_registry(registry, &fixture).await;

        assert_eq!(
            drive_fixture_once(&scheduler, &mut store, &fixture)
                .await
                .unwrap_or_else(|_| panic!("{}", kind.label())),
            SchedulerStatus::Advanced,
            "{}",
            kind.label()
        );
        let node = node_by_output(&fixture, &fixture.cell_a);
        assert_node_failed_with_code(&store, &node.node_id, "runner_output_invalid");
    }
}

#[tokio::test]
async fn runner_can_commit_inline_state_output_artifact() {
    struct InlineArtifactRunner {
        output_bytes: Vec<u8>,
    }

    impl ErasedNodeRunner for InlineArtifactRunner {
        fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
            Box::pin(async move {
                let artifact = state_output_artifact_for_bytes(
                    ctx.node(),
                    ctx.descriptor(),
                    &self.output_bytes,
                );
                let staged_artifact = StagedArtifact::inline_attempt_artifact(
                    &ctx,
                    self.output_bytes.clone(),
                    artifact.clone(),
                )?;
                Ok(ErasedRunnerOutput::from_parts(
                    vec![staged_artifact],
                    Vec::new(),
                    terminal_payloads(&ctx, &artifact),
                ))
            })
        }
    }

    let fixture = fixture();
    let output_bytes = br#"{"inline":true}"#.to_vec();
    let output_digest = digest_for_bytes(&output_bytes);
    let output_artifact =
        ArtifactId::from_digest(output_digest.algorithm(), *output_digest.digest());
    let registry =
        fixture_registry_with_first_runner(&fixture, "pure", InlineArtifactRunner { output_bytes });
    let (scheduler, mut store) = started_fixture_run_with_registry(registry, &fixture).await;

    assert_drive!(scheduler, store, fixture, Advanced, "drive inline output");
    assert!(matches!(
        store.projection_snapshot().cell_terminal(&fixture.cell_a),
        Some(store::CellTerminalProjection::Produced {
            artifact_id,
            content_digest,
            ..
        }) if artifact_id == &output_artifact && content_digest == &output_digest
    ));
}

#[tokio::test]
async fn runner_cannot_stage_reserved_retention_reasons() {
    struct ReservedRetentionReasonRunner {
        output_bytes: Vec<u8>,
        reason: events::RetentionReason,
    }

    impl ErasedNodeRunner for ReservedRetentionReasonRunner {
        fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
            Box::pin(async move {
                let artifact = state_output_artifact_for_bytes(
                    ctx.node(),
                    ctx.descriptor(),
                    &self.output_bytes,
                );
                let staged_artifact = StagedArtifact::inline_attempt_artifact(
                    &ctx,
                    self.output_bytes.clone(),
                    artifact.clone(),
                )?;
                Ok(ErasedRunnerOutput::from_parts(
                    vec![staged_artifact],
                    vec![StagedRetentionRefs {
                        refs: vec![artifact.retention_ref()?],
                        reason: self.reason,
                        authority:
                            crate::artifacts::StagedRetentionRefAuthority::CurrentCommitArtifacts,
                    }],
                    terminal_payloads(&ctx, &artifact),
                ))
            })
        }
    }

    for reason in [
        events::RetentionReason::RunAdmitted,
        events::RetentionReason::ManifestProjection,
        events::RetentionReason::PublicOutput,
    ] {
        let fixture = fixture();
        let node = node_by_output(&fixture, &fixture.cell_a).clone();
        let registry = fixture_registry_with_first_runner(
            &fixture,
            "pure",
            ReservedRetentionReasonRunner {
                output_bytes: br#"{"reserved":true}"#.to_vec(),
                reason,
            },
        );
        let (scheduler, mut store) = started_fixture_run_with_registry(registry, &fixture).await;
        let stream_before = store.load_run_stream(&fixture.run_id);

        assert_drive!(
            scheduler,
            store,
            fixture,
            Advanced,
            "terminalize reserved retention reason"
        );
        store.assert_run_stream_len(&fixture.run_id, stream_before.len() + 4);
        assert_node_failed_with_code(&store, &node.node_id, "runner_output_invalid");
        assert!(store
            .projection_snapshot()
            .cell_terminal(&fixture.cell_a)
            .is_none());
    }
}

#[tokio::test]
async fn rejected_staged_payload_mismatch_does_not_admit_artifact_evidence() {
    struct MismatchedStagedArtifactRunner {
        staged_artifact: ArtifactId,
        staged_digest: ContentDigest,
        payload_artifact: ArtifactId,
        payload_digest: ContentDigest,
    }

    impl ErasedNodeRunner for MismatchedStagedArtifactRunner {
        fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
            Box::pin(async move {
                let artifact = state_output_artifact(
                    ctx.node(),
                    ctx.descriptor(),
                    self.staged_artifact.clone(),
                    self.staged_digest.clone(),
                );
                let staged_artifact = staged_attempt_artifact(&ctx, artifact)?;
                let payload_evidence = state_output_artifact(
                    ctx.node(),
                    ctx.descriptor(),
                    self.payload_artifact.clone(),
                    self.payload_digest.clone(),
                );
                Ok(ErasedRunnerOutput::from_parts(
                    vec![staged_artifact],
                    Vec::new(),
                    terminal_payloads(&ctx, &payload_evidence),
                ))
            })
        }
    }

    let fixture = fixture();
    let node = node_by_output(&fixture, &fixture.cell_a).clone();
    let staged_artifact = artifact(0xa1);
    let staged_digest = content(0xa2);
    let payload_artifact = artifact(0xa3);
    let payload_digest = content(0xa4);
    let registry = fixture_registry_with_first_runner(
        &fixture,
        "pure",
        MismatchedStagedArtifactRunner {
            staged_artifact: staged_artifact.clone(),
            staged_digest: staged_digest.clone(),
            payload_artifact,
            payload_digest,
        },
    );
    let (scheduler, mut store) = started_fixture_run_with_registry(registry, &fixture).await;
    let attempt_id = attempt_id(
        &fixture.run_id,
        fixture.runtime_spec.spec_hash(),
        &node.node_id,
        1,
    )
    .expect("attempt id");

    assert_drive!(
        scheduler,
        store,
        fixture,
        Advanced,
        "terminalize staged payload mismatch"
    );
    assert_node_failed_with_code(&store, &node.node_id, "runner_output_invalid");

    let descriptor = fixture
        .runtime_spec
        .state_descriptor_for_node(&node)
        .expect("descriptor");
    let output_cell = fixture
        .runtime_spec
        .cell(&node.output_cell)
        .expect("output cell");
    let attempt_logical_key =
        store::LogicalEventKey::new(format!("attempt:{}:{}", node.node_id, attempt_id))
            .expect("attempt key");
    let leaked_artifact_request = store_typed_commit_request! {
        run_id: fixture.run_id.clone(),
        expected_next_seq: store.expected_next_seq(&fixture.run_id),
        commit_key: store::CommitKey::new("missing-leaked-staged-artifact").expect("commit key"),
        payloads: vec![
            events::KernelEventPayload::CellProduced(events::CellProduced {
                spec_hash: fixture.runtime_spec.spec_hash().clone(),
                node_id: node.node_id.clone(),
                cell_id: node.output_cell.clone(),
                scope_id: node.scope_id.clone(),
                attempt_id: attempt_id.clone(),
                semantic_type_id: descriptor.output_semantic_type_id.clone(),
                schema_id: descriptor.output_schema_id.clone(),
                value_lineage: output_cell.value_lineage.clone(),
                context: output_cell.context.clone(),
                artifact_id: staged_artifact.clone(),
                content_digest: staged_digest.clone(),
                evidence_hash: staged_digest,
                producer_state_kind: Some(node.state_kind.clone()),
                producer_state_version: Some(node.state_version.clone()),
            }),
            events::KernelEventPayload::StateAttemptCompleted(events::StateAttemptCompleted {
                spec_hash: fixture.runtime_spec.spec_hash().clone(),
                node_id: node.node_id.clone(),
                attempt_id: attempt_id.clone(),
                output_cell_id: node.output_cell.clone(),
            }),
        ],
        required_artifacts: Vec::new(),
        preconditions: store::CommitPreconditions {
            required_run_state: store::RequiredRunState::NotCompleted,
            required_present_logical_keys: vec![attempt_logical_key],
            required_cell_states: vec![store::CellStatePrecondition {
                cell_id: node.output_cell.clone(),
                required: store::RequiredCellState::Absent,
            }],
            ..store::CommitPreconditions::default()
        },
    };
    let artifacts = store::CommitArtifactEvidenceSet::new(
        leaked_artifact_request.required_artifacts().to_vec(),
        Vec::new(),
    )
    .expect("leak probe artifact evidence set");
    let error =
        store::PreparedCommit::<store::AttemptTerminal>::new(leaked_artifact_request, artifacts)
            .expect_err("missing leak evidence rejects before append");
    assert!(matches!(
        error,
        store::StoreError::InvalidPreparedCommitPurpose { message, .. }
            if message.contains("missing required artifact evidence")
                && message.contains(staged_artifact.as_str())
    ));
}

#[test]
fn materialization_rejects_seed_digest_not_certified() {
    let fixture = fixture();
    let mut seed = fixture.seed_ref.clone();
    seed.digest = content(0xee);
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let store = TestTypedRunStore::new();
    assert!(matches!(
        prepare_fixture_launch(&scheduler, &store, &fixture, vec![seed],),
        Err(RuntimeError::InvalidRunStream(_))
    ));
}

#[test]
fn run_start_rejects_missing_config_artifact_evidence() {
    let fixture = fixture();
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let store = TestTypedRunStore::new();
    let mut evidence = run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]);
    evidence.config_artifacts.clear();
    assert!(matches!(
        scheduler.prepare_run_launch(
            &fixture.runtime_spec,
            fixture_run_identity_material(&fixture),
            evidence,
            store.expected_next_seq(&fixture.run_id),
        ),
        Err(RuntimeError::InvalidRunStream(_))
    ));
}

#[test]
fn run_start_rejects_missing_fact_descriptor_artifact_evidence() {
    let (fixture, _descriptor, descriptor_ref) = fixture_with_first_node_fact_descriptor();
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let store = TestTypedRunStore::new();
    let evidence = run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]);

    assert!(matches!(
        scheduler.prepare_run_launch(
            &fixture.runtime_spec,
            fixture_run_identity_material(&fixture),
            evidence,
            store.expected_next_seq(&fixture.run_id),
        ),
        Err(RuntimeError::InvalidRunnerOutput(message))
            if message.contains("missing fact descriptor artifact")
                && message.contains(descriptor_ref.descriptor_hash.as_str())
    ));
}

#[tokio::test]
async fn run_start_admits_certified_fact_descriptor_artifacts() {
    let (fixture, descriptor, descriptor_ref) = fixture_with_first_node_fact_descriptor();
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let mut store = TestTypedRunStore::new();
    let mut evidence = run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]);
    evidence
        .fact_descriptor_artifacts
        .push(fact_descriptor_artifact(&descriptor));

    let launch = scheduler
        .prepare_run_launch(
            &fixture.runtime_spec,
            fixture_run_identity_material(&fixture),
            evidence,
            store.expected_next_seq(&fixture.run_id),
        )
        .expect("descriptor-backed launch prepares");
    scheduler_start_run(&scheduler, &mut store, launch)
        .await
        .expect("descriptor-backed launch commits");

    let run_admitted = store.run_admitted(&fixture.run_id);
    assert_eq!(run_admitted.fact_descriptor_artifacts.len(), 1);
    let admitted_descriptor = &run_admitted.fact_descriptor_artifacts[0];
    assert_eq!(
        admitted_descriptor.role,
        events::ArtifactRole::FactDescriptor
    );
    assert_eq!(
        admitted_descriptor.content_digest,
        descriptor_ref.descriptor_hash
    );
    let committed = block_on_ready(store.load_committed_run_stream(&fixture.run_id))
        .expect("descriptor-backed committed stream");
    RuntimeRunView::from_committed_stream(&fixture.runtime_spec, &committed)
        .expect("descriptor-backed run stream validates");
}

#[tokio::test]
async fn raw_runtime_view_rejects_fact_descriptor_stream_without_artifact_authority() {
    let (fixture, descriptor, _) = fixture_with_first_node_fact_descriptor();
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let mut store = TestTypedRunStore::new();
    let mut evidence = run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]);
    evidence
        .fact_descriptor_artifacts
        .push(fact_descriptor_artifact(&descriptor));

    let launch = scheduler
        .prepare_run_launch(
            &fixture.runtime_spec,
            fixture_run_identity_material(&fixture),
            evidence,
            store.expected_next_seq(&fixture.run_id),
        )
        .expect("descriptor-backed launch prepares");
    scheduler_start_run(&scheduler, &mut store, launch)
        .await
        .expect("descriptor-backed launch commits");

    let stream = store.load_run_stream(&fixture.run_id);
    let error = RuntimeRunView::from_stream(&fixture.runtime_spec, &fixture.run_id, &stream)
        .expect_err("raw descriptor-bearing stream must fail closed");
    assert!(matches!(
        error,
        RuntimeError::InvalidRunStream(message)
            if message.contains("committed stream artifact authority")
    ));
}

#[tokio::test]
async fn run_start_admitted_uses_committed_fact_descriptor_artifacts() {
    let (fixture, descriptor, _) = fixture_with_first_node_fact_descriptor();
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let mut store = TestTypedRunStore::new();
    let mut evidence = run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]);
    evidence
        .fact_descriptor_artifacts
        .push(fact_descriptor_artifact(&descriptor));
    let launch = scheduler
        .prepare_run_launch(
            &fixture.runtime_spec,
            fixture_run_identity_material(&fixture),
            evidence,
            store.expected_next_seq(&fixture.run_id),
        )
        .expect("descriptor-backed launch prepares");

    let authority =
        scheduler_start_run_admitted(&scheduler, &mut store, &fixture.runtime_spec, launch)
            .await
            .expect("descriptor-backed admission uses committed stream authority");

    assert_eq!(authority.run_id(), &fixture.run_id);
    assert_eq!(
        authority.head_seq(),
        store.expected_next_seq(&fixture.run_id)
    );
}

#[tokio::test]
async fn fact_bearing_runtime_prefix_rebuild_uses_retained_artifact_bytes() {
    let (fixture, descriptor, _) = fixture_with_read_node_fact_descriptor();
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let mut store = TestTypedRunStore::new();
    let mut evidence = run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]);
    evidence
        .fact_descriptor_artifacts
        .push(fact_descriptor_artifact(&descriptor));
    let launch = scheduler
        .prepare_run_launch(
            &fixture.runtime_spec,
            fixture_run_identity_material(&fixture),
            evidence,
            store.expected_next_seq(&fixture.run_id),
        )
        .expect("descriptor-backed launch prepares");
    scheduler_start_run(&scheduler, &mut store, launch)
        .await
        .expect("descriptor-backed launch commits");
    let node_a = node_by_output(&fixture, &fixture.cell_a).clone();
    let node_a_attempt = append_attempt_start(&mut store, &fixture, &node_a, 1);
    append_terminal(
        &mut store,
        &fixture,
        &node_a,
        &node_a_attempt,
        artifact(0x70),
        content(0x71),
    );
    let node = node_by_output(&fixture, &fixture.cell_b).clone();
    let attempt_id = append_attempt_start(&mut store, &fixture, &node, 1);
    append_fact(&mut store, &fixture, &node, &attempt_id, 17, 23);

    let stream = store.load_run_stream(&fixture.run_id);
    let raw_error = RuntimeRunView::from_stream(&fixture.runtime_spec, &fixture.run_id, &stream)
        .expect_err("raw fact-bearing stream must fail closed");
    assert!(matches!(
        raw_error,
        RuntimeError::InvalidRunStream(message)
            if message.contains("committed stream artifact authority")
    ));
    let committed = block_on_ready(store.load_committed_run_stream(&fixture.run_id))
        .expect("fact-bearing committed stream");
    RuntimeRunView::from_committed_stream(&fixture.runtime_spec, &committed)
        .expect("fact-bearing runtime view validates with retained bytes");
    build_retention_manifest_artifact(
        &fixture.runtime_spec,
        &fixture.run_id,
        &stream,
        committed.artifact_byte_authority(),
    )
    .expect("fact-bearing prefix rebuilds with retained bytes");

    let missing = store::ArtifactByteAuthorityMap::new();
    build_retention_manifest_artifact(&fixture.runtime_spec, &fixture.run_id, &stream, &missing)
        .expect_err("fact-bearing prefix without retained bytes must fail");

    let (response_evidence, _) = test_fact_response_artifact(&node, 23);
    let response_key = (
        response_evidence.artifact_id.clone(),
        response_evidence
            .evidence_hash()
            .expect("response evidence hash"),
    );
    let mut mismatched = committed.artifact_byte_authority().clone();
    mismatched
        .get_mut(&response_key)
        .expect("response bytes retained")
        .0 = b"{\"amount\":999}".to_vec();
    build_retention_manifest_artifact(&fixture.runtime_spec, &fixture.run_id, &stream, &mismatched)
        .expect_err("fact-bearing prefix with mismatched response bytes must fail");
}

#[test]
fn run_start_rejects_mismatched_staged_launch_bytes() {
    let fixture = fixture();
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let store = TestTypedRunStore::new();
    let base = run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]);

    let mut bad_spec = base.clone();
    bad_spec.spec_artifact.bytes.push(b'\n');
    assert!(matches!(
        scheduler.prepare_run_launch(
            &fixture.runtime_spec,
            fixture_run_identity_material(&fixture),
            bad_spec,
            store.expected_next_seq(&fixture.run_id),
        ),
        Err(RuntimeError::InvalidRunnerOutput(_))
    ));

    let mut bad_certificate = base.clone();
    bad_certificate.certificate_artifact.bytes.push(b'\n');
    assert!(matches!(
        scheduler.prepare_run_launch(
            &fixture.runtime_spec,
            fixture_run_identity_material(&fixture),
            bad_certificate,
            store.expected_next_seq(&fixture.run_id),
        ),
        Err(RuntimeError::InvalidRunnerOutput(_))
    ));

    let mut bad_config = base.clone();
    bad_config
        .config_artifacts
        .first_mut()
        .expect("config artifact")
        .bytes
        .push(b'\n');
    assert!(matches!(
        scheduler.prepare_run_launch(
            &fixture.runtime_spec,
            fixture_run_identity_material(&fixture),
            bad_config,
            store.expected_next_seq(&fixture.run_id),
        ),
        Err(RuntimeError::InvalidRunnerOutput(_))
    ));

    let mut bad_seed = base;
    bad_seed
        .seed_cells
        .first_mut()
        .expect("seed cell")
        .bytes
        .push(b'\n');
    assert!(matches!(
        scheduler.prepare_run_launch(
            &fixture.runtime_spec,
            fixture_run_identity_material(&fixture),
            bad_seed,
            store.expected_next_seq(&fixture.run_id),
        ),
        Err(RuntimeError::InvalidRunnerOutput(_))
    ));
    assert!(store.load_run_stream(&fixture.run_id).is_empty());
}

#[tokio::test]
async fn run_admission_returns_bound_context_with_capability_and_framework_authority() {
    let fixture = fixture();
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let mut store = TestTypedRunStore::new();
    let launch =
        prepare_fixture_launch(&scheduler, &store, &fixture, vec![fixture.seed_ref.clone()])
            .expect("prepare launch");

    let authority =
        scheduler_start_run_admitted(&scheduler, &mut store, &fixture.runtime_spec, launch)
            .await
            .expect("admitted run authority");

    assert_eq!(authority.run_id(), &fixture.run_id);
    assert_eq!(authority.spec_hash(), fixture.runtime_spec.spec_hash());
    assert_eq!(
        authority.head_seq(),
        store.expected_next_seq(&fixture.run_id)
    );

    let node = node_by_output(&fixture, &fixture.cell_b);
    let capability = authority
        .bound_context()
        .capability_authority_for(&node.node_id)
        .expect("capability authority");
    assert_eq!(capability.capabilities(), &node.capability_bindings);
    assert_eq!(capability.implementations().len(), 1);
    assert_eq!(
        capability.implementations()[0].descriptor(),
        &node.capability_bindings.capabilities[0]
    );
    assert_eq!(
        capability.implementations()[0].implementation_id().as_str(),
        "mfm.test.capability"
    );

    let render_node = fixture
        .runtime_spec
        .topological_order()
        .iter()
        .filter_map(|node_id| fixture.runtime_spec.node(node_id))
        .find(|node| {
            matches!(
                node.framework,
                Some(spec::FrameworkNodeSpec::PublicOutputRender(_))
            )
        })
        .expect("public-output render node");
    let framework = authority
        .bound_context()
        .framework_handler_for(&render_node.node_id)
        .expect("framework handler authority");
    assert_eq!(
        framework.kind(),
        BoundFrameworkHandlerKind::PublicOutputRender
    );

    let run_admitted = store.run_admitted(&fixture.run_id);
    assert_eq!(
        run_admitted.runner_executables,
        authority.bound_context().runner_executables()
    );
    assert!(
        !authority.bound_context().adapter_executables().is_empty(),
        "fixture must exercise adapter executable binding evidence"
    );
    assert_eq!(
        run_admitted.adapter_executables,
        authority.bound_context().adapter_executables()
    );
    scheduler
        .validate_admitted_run_binding(&fixture.runtime_spec, &run_admitted)
        .expect("binding validation");
}

#[test]
fn run_start_rejects_invalid_capability_implementation_bindings() {
    #[derive(Clone, Copy, Debug)]
    enum Case {
        MissingImplementation,
        DescriptorMismatch,
    }

    for (case, expected_message) in [
        (
            Case::MissingImplementation,
            "missing capability implementation",
        ),
        (
            Case::DescriptorMismatch,
            "differs from certified descriptor",
        ),
    ] {
        let fixture = fixture();
        let registry = match case {
            Case::MissingImplementation => fixture_registry_with_first_runner(
                &fixture,
                "pure",
                RecordingRunner {
                    expected_caps: Vec::new(),
                    output_artifact: artifact(0xa1),
                    output_digest: content(0xa2),
                },
            ),
            Case::DescriptorMismatch => {
                let mut registry = ErasedRunnerRegistry::new();
                let implementation_id = CapabilityImplementationId::new("mfm.test.capability")
                    .expect("capability implementation");
                registry
                    .register_capability(CapabilityImplementationBinding::new(
                        CapabilityDescriptor::new(
                            fixture.cap_kind.clone(),
                            fixture.cap_version.clone(),
                            CapabilityRole::ReadExternal,
                            "wrong-read-db",
                        )
                        .expect("wrong capability descriptor"),
                        implementation_id,
                    ))
                    .expect("capability implementation");
                register_default_fixture_pure_runner(&mut registry, &fixture);
                register_default_fixture_read_runner(&mut registry, &fixture);
                registry
            }
        };
        let scheduler = test_scheduler(registry);
        let store = TestTypedRunStore::new();
        let error =
            prepare_fixture_launch(&scheduler, &store, &fixture, vec![fixture.seed_ref.clone()])
                .err()
                .expect("invalid capability implementation binding must reject launch");
        assert!(
            matches!(&error, RuntimeError::RunnerBinding(message) if message.contains(expected_message)),
            "{case:?} returned unexpected error: {error:?}"
        );
    }
}

#[tokio::test]
async fn resume_rejects_binding_changes_before_attempt_start() {
    #[derive(Clone, Copy, Debug)]
    enum Case {
        MissingDownstreamBinding,
        RunnerExecutableMismatch,
        AdapterExecutableMismatch,
    }

    for (case, expected_message, direct_validation_rejects) in [
        (
            Case::MissingDownstreamBinding,
            "missing runner binding",
            false,
        ),
        (
            Case::RunnerExecutableMismatch,
            "runner executable identities",
            true,
        ),
        (
            Case::AdapterExecutableMismatch,
            "adapter executable identities",
            true,
        ),
    ] {
        let fixture = fixture();
        let (_, mut store) = started_fixture_run(&fixture).await;
        let stream_len_before = store.load_run_stream(&fixture.run_id).len();

        let resume_scheduler = match case {
            Case::MissingDownstreamBinding => {
                let mut partial_registry = ErasedRunnerRegistry::new();
                register_default_fixture_pure_runner(&mut partial_registry, &fixture);
                test_scheduler(partial_registry)
            }
            Case::RunnerExecutableMismatch => {
                let mut changed_registry = ErasedRunnerRegistry::new();
                let mut changed_a = binding(
                    fixture.descriptor_a.clone(),
                    "pure",
                    RecordingRunner {
                        expected_caps: Vec::new(),
                        output_artifact: artifact(0xa1),
                        output_digest: content(0xa2),
                    },
                );
                changed_a.executable.binary_digest = content(0xee);
                changed_registry.register(changed_a).expect("binding a");
                register_default_fixture_read_runner(&mut changed_registry, &fixture);
                fixture_scheduler(changed_registry, &fixture)
            }
            Case::AdapterExecutableMismatch => {
                let mut changed_adapter = test_adapter_executable_identity();
                changed_adapter.binary_digest = content(0xef);
                test_scheduler(registered_fixture_runners_with_adapter_executable(
                    &fixture,
                    changed_adapter,
                ))
            }
        };

        if direct_validation_rejects {
            let run_admitted = store.run_admitted(&fixture.run_id);
            resume_scheduler
                .validate_admitted_run_binding(&fixture.runtime_spec, &run_admitted)
                .expect_err("binding validation should reject");
        }

        let error = drive_fixture_once(&resume_scheduler, &mut store, &fixture)
            .await
            .expect_err("changed binding should reject bound context");
        assert!(
            matches!(&error, RuntimeError::RunnerBinding(message) if message.contains(expected_message)),
            "{case:?} returned unexpected error: {error:?}"
        );
        store.assert_run_stream_len(&fixture.run_id, stream_len_before);
    }
}

#[tokio::test]
async fn runner_invocation_uses_run_admitted_config_evidence_without_reference_event() {
    let fixture = fixture();
    let (scheduler, mut store) = started_fixture_run(&fixture).await;

    let valid_stream = store.load_run_stream(&fixture.run_id);
    assert!(valid_stream.iter().all(|event| {
        !matches!(
            event.payload(),
            events::KernelEventPayload::ArtifactReferenced(payload)
                if payload.artifact_ref.role == events::ArtifactRole::TypedConfig
        )
    }));
    drive_ok!(
        scheduler,
        store,
        fixture,
        "drive with RunAdmitted config evidence"
    );
}

#[tokio::test]
async fn runner_invocation_requires_committed_produced_input_artifact_reference() {
    let fixture = fixture();
    let (scheduler, mut store) = started_fixture_run(&fixture).await;
    drive_ok!(scheduler, store, fixture, "produce first cell");

    let producer_node_id = node_by_output(&fixture, &fixture.cell_a).node_id.clone();
    let consumer_node_id = node_by_output(&fixture, &fixture.cell_b).node_id.clone();
    let valid_stream = store.load_run_stream(&fixture.run_id);
    let corrupt_stream = rewrite_stream_without_payloads(&valid_stream, |payload| {
        matches!(
            payload,
            events::KernelEventPayload::ArtifactReferenced(payload)
                if payload.artifact_ref.role == events::ArtifactRole::StateOutput
                    && payload.node_id.as_ref() == Some(&producer_node_id)
        )
    });
    {
        let corrupt_store = StaleStreamStore::new(&mut store, corrupt_stream);
        assert!(matches!(
            drive_once_with_claim(&scheduler, &corrupt_store, &fixture.runtime_spec, &fixture.run_id)
                .await,
            Err(RuntimeError::InputMaterialization(message))
                if message.contains("is not committed in the run stream")
        ));
    }
    assert_eq!(
        attempt_started_count(&store, &fixture.run_id, &consumer_node_id),
        1
    );
}

#[tokio::test]
async fn post_start_materialization_failure_terminalizes_attempt() {
    let fixture = fixture();
    let (scheduler, mut store) = started_fixture_run(&fixture).await;
    drive_ok!(scheduler, store, fixture, "produce first cell");

    let producer_node_id = node_by_output(&fixture, &fixture.cell_a).node_id.clone();
    let consumer_node = node_by_output(&fixture, &fixture.cell_b).clone();
    {
        let corrupt_store = MissingInputArtifactRefStore::new(&mut store, producer_node_id);
        assert_eq!(
            drive_once_with_claim(
                &scheduler,
                &corrupt_store,
                &fixture.runtime_spec,
                &fixture.run_id
            )
            .await
            .expect("terminalize materialization failure"),
            SchedulerStatus::Advanced
        );
    }

    assert_node_failed_with_code_and_retryable(
        &store,
        &consumer_node.node_id,
        "input_materialization_failed",
        true,
    );
    assert!(store
        .projection_snapshot()
        .cell_terminal(&consumer_node.output_cell)
        .is_none());
}

#[tokio::test]
async fn post_start_runner_errors_follow_terminal_policy_by_error_class() {
    #[derive(Clone, Copy)]
    enum Case {
        RuntimeValidation,
        InvalidRunStream,
    }

    for case in [Case::RuntimeValidation, Case::InvalidRunStream] {
        let fixture = fixture();
        let error = match case {
            Case::RuntimeValidation => RuntimeError::RuntimeValidation(
                "synthetic post-start validation failure".to_owned(),
            ),
            Case::InvalidRunStream => {
                RuntimeError::InvalidRunStream("synthetic corrupt stream authority".to_owned())
            }
        };
        let registry = fixture_registry_with_first_runner(&fixture, "pure", ErrorRunner { error });
        let (scheduler, mut store) = started_fixture_run_with_registry(registry, &fixture).await;

        match case {
            Case::RuntimeValidation => {
                assert_drive!(
                    scheduler,
                    store,
                    fixture,
                    Advanced,
                    "terminalize runtime validation failure"
                );

                let node = node_by_output(&fixture, &fixture.cell_a);
                assert_node_failed_with_code(&store, &node.node_id, "runtime_validation_failed");
                assert_eq!(
                    runtime_lifecycle_summary(&store, &fixture.run_id),
                    "run=Started attempts[started=0 completed=0 failed=1 interrupted=0 total=1] cells=0 side_effects=0 lanes[run=0 total=0] public_outputs=0 retentions=1"
                );
            }
            Case::InvalidRunStream => {
                assert!(matches!(
                    drive_fixture_once(&scheduler, &mut store, &fixture)
                        .await,
                    Err(RuntimeError::InvalidRunStream(message))
                        if message.contains("synthetic corrupt stream authority")
                ));

                let node = node_by_output(&fixture, &fixture.cell_a);
                let projection_snapshot = store.projection_snapshot();
                let attempts = projection_snapshot
                    .attempts()
                    .filter(|((node_id, _), _)| node_id == &node.node_id)
                    .map(|(_, attempt)| attempt)
                    .collect::<Vec<_>>();
                assert_eq!(attempts.len(), 1);
                assert!(matches!(
                    attempts[0].status,
                    store::AttemptStatus::Started { .. }
                ));
                assert_failure_code_count(&store, "runtime_validation_failed", 0);
                assert_failure_code_count(&store, "runner_output_invalid", 0);
                assert_eq!(
                    runtime_lifecycle_summary(&store, &fixture.run_id),
                    "run=Started attempts[started=1 completed=0 failed=0 interrupted=0 total=1] cells=0 side_effects=0 lanes[run=0 total=0] public_outputs=0 retentions=1"
                );
            }
        }
    }
}

#[tokio::test]
async fn replay_rejects_terminal_cell_producer_outside_certified_spec() {
    let fixture = fixture();
    let (scheduler, mut store) = started_fixture_run(&fixture).await;

    let forged_node = fixture
        .runtime_spec
        .topological_order()
        .iter()
        .filter_map(|node_id| fixture.runtime_spec.node(node_id))
        .find(|node| node.output_cell != fixture.cell_a)
        .expect("second node")
        .clone();
    let certified_cell = fixture
        .runtime_spec
        .cell(&fixture.cell_a)
        .expect("cell a")
        .clone();
    let forged_attempt = AttemptId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([0xfa; 32]),
    );
    let artifact_id = artifact(0xfa);
    let artifact_digest = content(0xfb);
    let forged_artifact = store::ArtifactEvidenceRef {
        artifact_id: artifact_id.clone(),
        digest: artifact_digest.clone(),
        byte_len: 10,
        media_type: spec::MediaType::new("application/json").expect("media"),
        schema_id: Some(certified_cell.schema_id.clone()),
        semantic_type_id: Some(certified_cell.semantic_type_id.clone()),
        producer_node_id: Some(forged_node.node_id.clone()),
        producer_seed_id: None,
        artifact_role: events::ArtifactRole::StateOutput,
    };
    store
        .append_prepared_commit(store_typed_commit_request! {
            run_id: fixture.run_id.clone(),
            expected_next_seq: store.expected_next_seq(&fixture.run_id),
            commit_key: store::CommitKey::new("forged-attempt-start").expect("commit key"),
            payloads: vec![events::KernelEventPayload::StateAttemptStarted(
                events::StateAttemptStarted {
                    spec_hash: fixture.runtime_spec.spec_hash().clone(),
                    node_id: forged_node.node_id.clone(),
                    attempt_id: forged_attempt.clone(),
                    attempt_no: 1,
                    state_kind: forged_node.state_kind.clone(),
                    state_version: forged_node.state_version.clone(),
                },
            )],
            required_artifacts: Vec::new(),
            preconditions: store::CommitPreconditions {
                required_run_state: store::RequiredRunState::NotCompleted,
                ..store::CommitPreconditions::default()
            },
        })
        .expect("append forged attempt start");
    store
        .append_prepared_commit(store_typed_commit_request! {
            run_id: fixture.run_id.clone(),
            expected_next_seq: store.expected_next_seq(&fixture.run_id),
            commit_key: store::CommitKey::new("forged-terminal").expect("commit key"),
            payloads: vec![
                events::KernelEventPayload::CellProduced(events::CellProduced {
                    spec_hash: fixture.runtime_spec.spec_hash().clone(),
                    node_id: forged_node.node_id.clone(),
                    cell_id: fixture.cell_a.clone(),
                    scope_id: certified_cell.scope_id.clone(),
                    attempt_id: forged_attempt.clone(),
                    semantic_type_id: certified_cell.semantic_type_id.clone(),
                    schema_id: certified_cell.schema_id.clone(),
                    value_lineage: certified_cell.value_lineage.clone(),
                    context: certified_cell.context.clone(),
                    artifact_id,
                    content_digest: artifact_digest,
                    evidence_hash: forged_artifact
                        .evidence_hash()
                        .expect("forged terminal evidence hash"),
                    producer_state_kind: Some(forged_node.state_kind.clone()),
                    producer_state_version: Some(forged_node.state_version.clone()),
                }),
                events::KernelEventPayload::StateAttemptCompleted(events::StateAttemptCompleted {
                    spec_hash: fixture.runtime_spec.spec_hash().clone(),
                    node_id: forged_node.node_id.clone(),
                    attempt_id: forged_attempt,
                    output_cell_id: fixture.cell_a.clone(),
                }),
            ],
            required_artifacts: vec![forged_artifact],
            preconditions: store::CommitPreconditions {
                required_run_state: store::RequiredRunState::NotCompleted,
                ..store::CommitPreconditions::default()
            },
        })
        .expect("append forged terminal");

    assert!(matches!(
        drive_fixture_once(&scheduler, &mut store, &fixture).await,
        Err(RuntimeError::InvalidRunStream(_))
    ));
}
