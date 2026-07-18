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

    let error =
        match prepare_fixture_launch(&scheduler, &store, &fixture, vec![fixture.seed_ref.clone()])
            .await
        {
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
    let fixture = fixture();
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
    let fixture = fixture();
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
    let fixture = fixture();
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
