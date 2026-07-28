use super::*;

#[test]
fn runner_registration_builder_preserves_explicit_binding_authority() {
    let fixture = fixture();
    let node = node_by_output(&fixture, &fixture.cell_b);
    let descriptor = fixture
        .runtime_spec
        .state_descriptor_for_node(node)
        .expect("state descriptor");
    let factory_id = events::RunnerFactoryId::new(READ_EXTERNAL_RUNNER).expect("read factory");
    let executable = events::ExecutableIdentity {
        factory_id: factory_id.clone(),
        binary_digest: content(0xe2),
    };
    let implementation_id = CapabilityImplementationId::new("mfm.test.runner-kit-registration")
        .expect("implementation id");
    let mut registry = test_runner_registry();

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
        binary_digest: content(0xe2),
    };
    let typed_implementation_id =
        CapabilityImplementationId::new("mfm.test.runner-kit-typed-registration")
            .expect("typed implementation id");
    let mut typed_registry = test_runner_registry();
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
    let mut mismatch_registry = test_runner_registry();
    let error = match RunnerRegistrationBuilder::new(&mut mismatch_registry).register_runner(
        node.descriptor_id.clone(),
        factory_id,
        events::ExecutableIdentity {
            factory_id: wrong_factory,
            ..executable
        },
        Arc::new(RecordingRunner {
            expected_caps: Vec::new(),
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
    let mut registry = test_runner_registry();
    register_spec_capabilities(&mut registry, &fixture.runtime_spec);
    registry
        .register(binding(
            fixture.descriptor_a.clone(),
            "pure",
            RecordingRunner {
                expected_caps: Vec::new(),
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
async fn generic_pure_runner_preserves_typed_context_output_semantics() {
    let fixture = fixture_with_context_bound_states();
    let mut store = TestTypedRunStore::new();
    let mut registry = test_runner_registry();
    let pure_factory =
        registry.factory_binding(events::RunnerFactoryId::new("pure").expect("pure factory id"));
    {
        let mut registrations = RunnerRegistrationBuilder::new(&mut registry);
        register_pure_state::<RuntimeContextSourceState>(
            &mut registrations,
            &pure_factory,
            Some(Arc::new(
                TypedContextOutputExtractor::<RuntimeContextOutput>::new(),
            )),
        )
        .expect("register generic context source");
        register_pure_state::<RuntimeContextConsumerState>(&mut registrations, &pure_factory, None)
            .expect("register generic context consumer");
    }
    let scheduler = test_scheduler(register_fixture_capabilities(registry, &fixture));
    let current = started_fixture_current(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start generic pure run");

    let result = drive_current_until_blocked_with_claim(&scheduler, &store, current)
        .await
        .expect("drive generic pure run");
    assert_eq!(result.status(), SchedulerStatus::PublicOutputProjected);
    let current = result.into_current_run();
    let lifecycle = current.lifecycle();
    assert_eq!(lifecycle.run_state(), store::RunState::Completed);
    assert!(lifecycle.cell(&fixture.cell_a).is_some());
    assert!(lifecycle.cell(&fixture.cell_b).is_some());
}

#[test]
fn generic_pure_registration_rejects_non_pure_factory() {
    let mut registry = test_runner_registry();
    let wrong_factory = registry
        .factory_binding(events::RunnerFactoryId::new("read_external").expect("read factory id"));
    let error = register_pure_state::<CertifierState>(
        &mut RunnerRegistrationBuilder::new(&mut registry),
        &wrong_factory,
        None,
    )
    .expect_err("non-pure factory must reject");
    assert!(matches!(
        error,
        RuntimeError::RunnerBinding(message) if message.contains("factory id pure")
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
        let scheduler = fixture_scheduler(registry, &fixture);
        let mut store = TestTypedRunStore::new();
        let current = started_fixture_current(
            &scheduler,
            &mut store,
            &fixture,
            vec![fixture.seed_ref.clone()],
        )
        .await
        .expect("start context-bound fixture");
        let result = drive_current_once_with_claim(&scheduler, &store, current)
            .await
            .expect("terminalize mismatched context-bound output");
        assert_eq!(result.status(), SchedulerStatus::Advanced);
        let current = result.into_current_run();
        let node = node_by_output(&fixture, &fixture.cell_a);
        assert_node_failed_with_code(&current, &node.node_id, "runner_output_invalid");
    }
}

#[test]
fn certification_rejects_context_bound_input_under_wrong_node_context() {
    let fixture = fixture_with_context_bound_states();
    let mut typed_spec = fixture.runtime_spec.spec().clone();
    let consumer = typed_spec
        .nodes
        .iter_mut()
        .find(|node| node.descriptor_id == fixture.descriptor_b)
        .expect("consumer node");
    consumer.context = spec::NodeContextSpec::no_context();
    let error = certify_fixture_spec(&fixture.runtime_spec, typed_spec, |registry| {
        registry.register_state::<RuntimeContextSourceState>()?;
        registry.register_state::<RuntimeContextConsumerState>()
    })
    .expect_err("certification must reject the context contract mismatch");
    assert!(
        error.to_string().contains("context"),
        "unexpected certification error: {error}"
    );
}

#[tokio::test]
async fn serial_scheduler_runs_nodes_in_certified_topological_order() {
    let fixture = fixture();
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let mut store = TestTypedRunStore::new();
    let current = started_fixture_current(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start fixture");

    let result = drive_current_once_with_claim(&scheduler, &store, current)
        .await
        .expect("drive a");
    assert_eq!(result.status(), SchedulerStatus::Advanced);
    let current = result.into_current_run();
    assert!(current.lifecycle().cell(&fixture.cell_a).is_some());
    assert!(current.lifecycle().cell(&fixture.cell_b).is_none());
    let result = drive_current_once_with_claim(&scheduler, &store, current)
        .await
        .expect("drive b");
    assert_eq!(result.status(), SchedulerStatus::Advanced);
    let current = result.into_current_run();
    assert!(current.lifecycle().cell(&fixture.cell_b).is_some());
    assert_eq!(
        runtime_lifecycle_summary(&current),
        "run=Started attempts[started=0 completed=2 failed=0 interrupted=0 total=2] cells=2 side_effects=0 lanes=0 public_outputs=0 retentions=1"
    );
}

#[tokio::test]
async fn scheduler_completes_run_after_public_output_evidence() {
    let fixture = fixture();
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let mut store = TestTypedRunStore::new();
    let current = started_fixture_current(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start fixture");
    let result = drive_current_until_blocked_with_claim(&scheduler, &store, current)
        .await
        .expect("drive to public output");
    assert_eq!(result.status(), SchedulerStatus::PublicOutputProjected);
    let current = result.into_current_run();
    let lifecycle = current.lifecycle();
    assert_eq!(lifecycle.run_state(), store::RunState::Completed);
    assert!(lifecycle.cell(&fixture.render_cell).is_some());
    assert_eq!(
        runtime_lifecycle_summary(&current),
        "run=Completed attempts[started=0 completed=5 failed=0 interrupted=0 total=5] cells=5 side_effects=0 lanes=0 public_outputs=1 retentions=1"
    );

    let complete_node = certified_complete_run_node(&fixture.runtime_spec).expect("complete node");
    let current_sequence = current
        .view()
        .current_run_sequence()
        .expect("completed run sequence");
    {
        let lifecycle = current.lifecycle();
        let mut record_count = 0;
        let mut public_output = None;
        let mut completion = None;
        let mut complete_attempt = None;
        let _ = lifecycle.visit_records(|record| {
            record_count += 1;
            match record.kind() {
                store::current_lifecycle::CurrentRecordKindRef::PublicOutputProduced(payload) => {
                    assert_eq!(payload.node_id, fixture.render_node);
                    assert_eq!(payload.receipt_cell_id, fixture.render_cell);
                    assert!(payload.rendered_artifact_id.is_none());
                    public_output = Some((record.position(), record.event_id().clone()));
                }
                store::current_lifecycle::CurrentRecordKindRef::StateAttemptStarted(payload)
                    if payload.node_id == complete_node.node_id =>
                {
                    complete_attempt = Some((record.sequence(), payload.attempt_id.clone()));
                }
                store::current_lifecycle::CurrentRecordKindRef::RunCompleted(payload) => {
                    let (public_position, public_event_id) = public_output
                        .as_ref()
                        .expect("public output produced first");
                    assert!(*public_position < record.position());
                    assert_eq!(
                        payload.outcome,
                        events::RunCompletionOutcome::Completed(Box::new(
                            events::PublicOutputCompletionEvidence {
                                public_output_schema_id: fixture
                                    .runtime_spec
                                    .spec()
                                    .public_outputs
                                    .public_schema_id
                                    .clone(),
                                public_output_event_id: public_event_id.clone(),
                            },
                        ))
                    );
                    completion = Some((
                        record.position(),
                        record.sequence(),
                        record.commit_key().clone(),
                    ));
                }
                _ => {}
            }
            std::ops::ControlFlow::<()>::Continue(())
        });
        let (completion_position, completion_sequence, completion_key) =
            completion.expect("run completed");
        assert_eq!(completion_position, record_count - 1);
        let (attempt_sequence, complete_attempt) =
            complete_attempt.expect("complete attempt started");
        assert!(attempt_sequence < completion_sequence);

        let mut completion_commit_count = 0;
        let mut complete_receipt = None;
        let mut completed_attempt = false;
        let mut referenced_receipt = None;
        let _ = lifecycle.visit_records(|record| {
            if record.sequence() != completion_sequence || record.commit_key() != &completion_key {
                return std::ops::ControlFlow::<()>::Continue(());
            }
            completion_commit_count += 1;
            match record.kind() {
                store::current_lifecycle::CurrentRecordKindRef::CellProduced(payload)
                    if payload.node_id == complete_node.node_id =>
                {
                    assert_eq!(payload.attempt_id, complete_attempt);
                    assert_eq!(payload.cell_id, complete_node.output_cell);
                    complete_receipt =
                        Some((payload.artifact_id.clone(), payload.content_digest.clone()));
                }
                store::current_lifecycle::CurrentRecordKindRef::StateAttemptCompleted(payload)
                    if payload.node_id == complete_node.node_id
                        && payload.attempt_id == complete_attempt
                        && payload.output_cell_id == complete_node.output_cell =>
                {
                    completed_attempt = true;
                }
                store::current_lifecycle::CurrentRecordKindRef::ArtifactReferenced(payload)
                    if payload.node_id.as_ref() == Some(&complete_node.node_id)
                        && payload.attempt_id.as_ref() == Some(&complete_attempt)
                        && payload.artifact_ref.role == events::ArtifactRole::StateOutput =>
                {
                    referenced_receipt = Some((
                        payload.artifact_ref.artifact_id.clone(),
                        payload.artifact_ref.content_digest.clone(),
                    ));
                }
                _ => {}
            }
            std::ops::ControlFlow::<()>::Continue(())
        });
        assert_eq!(completion_commit_count, 4);
        assert_eq!(complete_receipt, referenced_receipt);
        assert!(completed_attempt);
    }

    let result = drive_current_once_with_claim(&scheduler, &store, current)
        .await
        .expect("drive completed run");
    assert_eq!(result.status(), SchedulerStatus::PublicOutputProjected);
    assert_eq!(
        result
            .current_run()
            .view()
            .current_run_sequence()
            .expect("completed run sequence"),
        current_sequence
    );
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
    let mut registry = test_runner_registry();
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
            READ_EXTERNAL_RUNNER,
            InlineRecordingRunner {
                expected_caps: vec![(fixture.cap_kind.clone(), fixture.cap_version.clone())],
                output_bytes: br#"{"node":"b"}"#.to_vec(),
            },
        ))
        .expect("binding b");
    let scheduler = test_scheduler(register_fixture_capabilities(registry, &fixture));
    let store = RecordingTypedRunStore::new();
    start_fixture_run_async_store(&scheduler, &store, &fixture, vec![fixture.seed_ref.clone()])
        .await
        .expect("start run");
    let current = load_fixture_current(&scheduler, &store, &fixture)
        .await
        .expect("load representative current run");
    let result = drive_current_until_blocked_with_claim(&scheduler, &store, current)
        .await
        .expect("drive full representative run");
    assert_eq!(result.status(), SchedulerStatus::PublicOutputProjected);
    let current = result.into_current_run();
    assert_eq!(current.lifecycle().run_state(), store::RunState::Completed);
    assert_every_certified_node_has_attempt(&fixture.runtime_spec, &current);
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
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let store = started_fixture_store(&scheduler, &fixture).await;

    let current = load_fixture_current(&scheduler, &store, &fixture)
        .await
        .expect("load admitted current run");
    let lifecycle = current.lifecycle();
    let mut first_batch = None;
    let mut admission_commit_count = 0;
    let mut first_is_admission = false;
    let mut disallowed_record = false;
    let _ = lifecycle.visit_records(|record| {
        let (first_sequence, first_key) = first_batch.get_or_insert_with(|| {
            assert_eq!(record.ordinal(), store::CommitOrdinal::new(0));
            (record.sequence(), record.commit_key().clone())
        });
        if record.sequence() == *first_sequence && record.commit_key() == &*first_key {
            admission_commit_count += 1;
        }
        if record.position() == 0 {
            first_is_admission = matches!(
                record.kind(),
                store::current_lifecycle::CurrentRecordKindRef::RunAdmitted(_)
            );
        }
        disallowed_record |= matches!(
            record.kind(),
            store::current_lifecycle::CurrentRecordKindRef::StateAttemptStarted(_)
                | store::current_lifecycle::CurrentRecordKindRef::StateAttemptCompleted(_)
                | store::current_lifecycle::CurrentRecordKindRef::CellProduced(_)
                | store::current_lifecycle::CurrentRecordKindRef::ArtifactReferenced(_)
                | store::current_lifecycle::CurrentRecordKindRef::RetentionRefsAppended(_)
        );
        std::ops::ControlFlow::<()>::Continue(())
    });
    assert!(first_is_admission);
    assert_eq!(admission_commit_count, 1);

    let run_admitted = lifecycle.admission().expect("run admission");
    assert_eq!(
        run_admitted.spec_artifact().role,
        events::ArtifactRole::TypedExecutionSpec
    );
    assert_eq!(
        run_admitted.certificate_artifact().role,
        events::ArtifactRole::TypedSpecCertificate
    );
    assert!(run_admitted
        .config_artifacts()
        .all(|artifact| artifact.role == events::ArtifactRole::TypedConfig));
    assert_eq!(
        fixture.seed_ref.seed_artifact.role,
        events::ArtifactRole::SeedInput
    );
    assert!(!disallowed_record);
}

#[tokio::test]
async fn scheduler_binds_staged_retention_refs_and_projects_manifest() {
    let fixture = fixture();
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let mut store = TestTypedRunStore::new();
    let current = started_fixture_current(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start fixture");
    let mut retained_seed = false;
    let _ = current
        .lifecycle()
        .retention()
        .expect("run-start retention")
        .visit_references(|reference| {
            retained_seed |= reference.artifact_id == fixture.seed_ref.seed_artifact.artifact_id;
            std::ops::ControlFlow::<()>::Continue(())
        });
    assert!(retained_seed);

    let result = drive_current_until_blocked_with_claim(&scheduler, &store, current)
        .await
        .expect("drive to completion");
    assert_eq!(result.status(), SchedulerStatus::PublicOutputProjected);
    let current = result.into_current_run();
    let lifecycle = current.lifecycle();
    let render_receipt_artifact = lifecycle
        .cell(&fixture.render_cell)
        .expect("render receipt cell")
        .produced()
        .expect("produced render receipt")
        .artifact_id()
        .clone();
    let retention = lifecycle.retention().expect("runtime retention");
    let mut retained_render_receipt = false;
    let _ = retention.visit_references(|reference| {
        retained_render_receipt |= reference.artifact_id == render_receipt_artifact;
        std::ops::ControlFlow::<()>::Continue(())
    });
    assert!(retained_render_receipt);

    let manifest = retention.current_manifest().expect("manifest");
    assert_eq!(manifest.sequence(), 1);
    let manifest_artifact_id = manifest.artifact_id().clone();
    let manifest_digest = manifest.digest().clone();
    let mut retained_manifest = false;
    let _ = retention.visit_references(|reference| {
        retained_manifest |= reference.artifact_id == manifest_artifact_id;
        std::ops::ControlFlow::<()>::Continue(())
    });
    assert!(retained_manifest);

    let retention_node =
        certified_retention_manifest_node(&fixture.runtime_spec).expect("retention node");
    let mut retention_sequence = None;
    let _ = lifecycle.visit_records(|record| {
        if let store::current_lifecycle::CurrentRecordKindRef::RetentionManifestProjected(payload) =
            record.kind()
        {
            if payload.manifest_artifact_id == manifest_artifact_id {
                assert_eq!(payload.manifest_digest, manifest_digest);
                retention_sequence = Some(record.sequence());
            }
        }
        std::ops::ControlFlow::<()>::Continue(())
    });
    let retention_sequence = retention_sequence.expect("manifest event");
    let mut receipt_attempt = None;
    let mut started_attempts = BTreeSet::new();
    let mut completed_attempts = BTreeSet::new();
    let mut manifest_reference = false;
    let _ = lifecycle.visit_records(|record| {
        match record.kind() {
            store::current_lifecycle::CurrentRecordKindRef::CellProduced(payload)
                if record.sequence() == retention_sequence
                    && payload.node_id == retention_node.node_id =>
            {
                assert_eq!(payload.cell_id, retention_node.output_cell);
                receipt_attempt = Some(payload.attempt_id.clone());
            }
            store::current_lifecycle::CurrentRecordKindRef::StateAttemptStarted(payload)
                if record.sequence() < retention_sequence
                    && payload.node_id == retention_node.node_id =>
            {
                started_attempts.insert(payload.attempt_id.clone());
            }
            store::current_lifecycle::CurrentRecordKindRef::StateAttemptCompleted(payload)
                if record.sequence() == retention_sequence
                    && payload.node_id == retention_node.node_id
                    && payload.output_cell_id == retention_node.output_cell =>
            {
                completed_attempts.insert(payload.attempt_id.clone());
            }
            store::current_lifecycle::CurrentRecordKindRef::RetentionRefsAppended(payload)
                if record.sequence() == retention_sequence
                    && payload.reason == events::RetentionReason::ManifestProjection =>
            {
                manifest_reference = payload.refs.iter().any(|reference| {
                    reference.artifact_id == manifest_artifact_id
                        && reference.content_digest == manifest_digest
                        && reference.role == events::ArtifactRole::RetentionManifest
                });
            }
            _ => {}
        }
        std::ops::ControlFlow::<()>::Continue(())
    });
    let receipt_attempt = receipt_attempt.expect("retention receipt attempt");
    assert!(started_attempts.contains(&receipt_attempt));
    assert!(completed_attempts.contains(&receipt_attempt));
    assert!(manifest_reference);
}

#[tokio::test]
async fn retention_manifest_projection_retry_is_idempotent_after_current_store_advanced() {
    let fixture = fixture();
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let mut store = TestTypedRunStore::new();
    let mut current = started_fixture_current(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start fixture");
    for _ in 0..8 {
        let result = drive_current_once_with_claim(&scheduler, &store, current)
            .await
            .expect("drive until public output");
        current = result.into_current_run();
        let lifecycle = current.lifecycle();
        if lifecycle.run_state() == store::RunState::Started
            && lifecycle
                .public_output(&fixture.runtime_spec.spec().public_outputs.public_schema_id)
                .is_some_and(|output| output.produced().is_some())
        {
            break;
        }
    }
    assert!(
        current
            .lifecycle()
            .public_output(&fixture.runtime_spec.spec().public_outputs.public_schema_id,)
            .is_some_and(|output| output.produced().is_some()),
        "public output was not produced"
    );
    let retention_node =
        certified_retention_manifest_node(&fixture.runtime_spec).expect("retention node");
    append_attempt_start(&mut store, &fixture, retention_node, 1);
    drop(current);
    let stale_current = load_fixture_current(&scheduler, &store, &fixture)
        .await
        .expect("load stale retention authority");
    let advancing_current = load_fixture_current(&scheduler, &store, &fixture)
        .await
        .expect("load independent retention authority");

    let advanced = drive_current_once_with_claim(&scheduler, &store, advancing_current)
        .await
        .expect("advance current store with retention projection");
    assert_eq!(advanced.status(), SchedulerStatus::Advanced);
    assert!(advanced
        .current_run()
        .lifecycle()
        .retention()
        .and_then(|retention| retention.current_manifest())
        .is_some());

    let retried = drive_current_once_with_claim(&scheduler, &store, stale_current)
        .await
        .expect("idempotent retention retry");
    assert_eq!(retried.status(), SchedulerStatus::Advanced);
}

#[tokio::test]
async fn runtime_rejects_standalone_retention_manifest_projection_history() {
    let fixture = fixture();
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let mut store = TestTypedRunStore::new();
    let mut current = started_fixture_current(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start fixture");
    for _ in 0..8 {
        let result = drive_current_once_with_claim(&scheduler, &store, current)
            .await
            .expect("drive until public output");
        current = result.into_current_run();
        if current
            .lifecycle()
            .public_output(&fixture.runtime_spec.spec().public_outputs.public_schema_id)
            .is_some_and(|output| output.produced().is_some())
        {
            break;
        }
    }
    let manifest = build_retention_manifest_artifact(&current.runtime_spec(), &current.lifecycle())
        .expect("manifest");
    let manifest_bytes = manifest.bytes.as_bytes().to_vec();
    let manifest_evidence = manifest.evidence.clone();
    let request = store_typed_commit_request! {
        run_id: fixture.run_id.clone(),
        expected_next_seq: store.expected_next_seq(&fixture.run_id),
        commit_key: store::CommitKey::new("synthetic/standalone-retention-projection")
            .expect("commit key"),
        payloads: retention_manifest_payloads(&fixture.runtime_spec, &fixture.run_id, manifest)
            .expect("retention manifest payloads"),
        required_artifacts: vec![manifest_evidence.clone()],
        preconditions: store::CommitPreconditions::default(),
    };
    drop(current);
    store
        .append_prepared_commit_with_artifacts(request, vec![(manifest_bytes, manifest_evidence)])
        .expect("synthetic standalone projection");

    let error = load_fixture_current(&scheduler, &store, &fixture)
        .await
        .expect_err("standalone retention projection must fail closed");
    assert!(
        matches!(
            &error,
            RuntimeError::Store(message)
                if message.contains(
                    "retention manifest batch lacks exactly one framework receipt cell"
                )
        ),
        "unexpected standalone retention projection error: {error:?}"
    );
}

#[tokio::test]
async fn runtime_rejects_complete_run_receipt_commit_without_run_completed() {
    let fixture = fixture();
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let mut store = TestTypedRunStore::new();
    let current = started_fixture_current(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start fixture");
    let result = drive_current_until_blocked_with_claim(&scheduler, &store, current)
        .await
        .expect("drive to completion");
    assert_eq!(result.status(), SchedulerStatus::PublicOutputProjected);
    drop(result);

    let valid_records = store.committed_records_for_corruption(&fixture.run_id);
    let corrupt_records = rewrite_records_without_payloads(&valid_records, |payload| {
        matches!(payload, events::KernelEventPayload::RunCompleted(_))
    });

    let error = validate_corrupted_journal_for_tests(
        &store,
        &fixture.runtime_spec,
        &fixture.run_id,
        corrupt_records,
    )
    .expect_err("completion receipt commit without RunCompleted must fail closed");
    assert!(
        matches!(
            &error,
            RuntimeError::Store(message)
                if message.contains(
                    "terminal lifecycle batch must contain exactly one RunCompleted"
                )
        ),
        "unexpected incomplete completion-receipt error: {error:?}"
    );
}

#[tokio::test]
async fn public_output_render_failure_resumes_and_completes() {
    let fixture = fixture();
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let mut store = TestTypedRunStore::new();
    let current = started_fixture_current(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start fixture");
    let result = drive_current_once_with_claim(&scheduler, &store, current)
        .await
        .expect("drive a");
    assert_eq!(result.status(), SchedulerStatus::Advanced);
    let result = drive_current_once_with_claim(&scheduler, &store, result.into_current_run())
        .await
        .expect("drive b");
    assert_eq!(result.status(), SchedulerStatus::Advanced);
    let render_node = node_by_output(&fixture, &fixture.render_cell);
    let failed_attempt = append_attempt_start(&mut store, &fixture, render_node, 1);
    append_public_output_render_failure(&mut store, &fixture, render_node, &failed_attempt);
    drop(result);
    let current = load_fixture_current(&scheduler, &store, &fixture)
        .await
        .expect("load render failure");
    assert!(current
        .lifecycle()
        .public_output(&fixture.runtime_spec.spec().public_outputs.public_schema_id,)
        .is_some_and(|output| output.render_failure().is_some()));

    let result = drive_current_until_blocked_with_claim(&scheduler, &store, current)
        .await
        .expect("retry render");
    assert_eq!(result.status(), SchedulerStatus::PublicOutputProjected);
    let current = result.into_current_run();
    assert_eq!(attempt_started_count(&current, &fixture.render_node), 2);
    assert_eq!(current.lifecycle().run_state(), store::RunState::Completed);
    assert!(current
        .lifecycle()
        .public_output(&fixture.runtime_spec.spec().public_outputs.public_schema_id,)
        .is_some_and(|output| output.produced().is_some()));
}
