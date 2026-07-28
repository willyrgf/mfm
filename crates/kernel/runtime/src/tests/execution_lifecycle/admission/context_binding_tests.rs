use super::*;

#[tokio::test]
async fn run_admission_commits_evidence_for_the_bound_runtime_context() {
    let fixture = fixture();
    let registry = registered_fixture_runners(&fixture);
    let bound_context =
        BoundRuntimeContext::bind(&fixture.runtime_spec, &registry).expect("bound runtime context");
    let scheduler = test_scheduler(registry);
    let mut store = TestTypedRunStore::new();
    let launch =
        prepare_fixture_launch(&scheduler, &store, &fixture, vec![fixture.seed_ref.clone()])
            .await
            .expect("prepare launch");
    scheduler_start_run(&scheduler, &mut store, launch)
        .await
        .expect("commit admission");
    let authority = load_fixture_current(&scheduler, &store, &fixture)
        .await
        .expect("admitted run authority");

    assert_eq!(authority.view().run_id(), &fixture.run_id);
    assert_eq!(
        authority.view().spec_hash(),
        fixture.runtime_spec.spec_hash()
    );
    assert_eq!(
        authority
            .lifecycle()
            .next_sequence()
            .expect("next journal sequence"),
        store.expected_next_seq(&fixture.run_id)
    );

    let node = node_by_output(&fixture, &fixture.cell_b);
    let capability = bound_context
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
    let framework = bound_context
        .framework_handler_for(&render_node.node_id)
        .expect("framework handler authority");
    assert_eq!(
        framework.kind(),
        BoundFrameworkHandlerKind::PublicOutputRender
    );

    let lifecycle = authority.lifecycle();
    let run_admitted = lifecycle.admission().expect("run admission");
    assert!(run_admitted
        .runner_executables()
        .eq(bound_context.runner_executables().iter()));
    assert!(
        !bound_context.adapter_executables().is_empty(),
        "fixture must exercise adapter executable binding evidence"
    );
    assert!(run_admitted
        .adapter_executables()
        .eq(bound_context.adapter_executables().iter()));
    let executables = bound_context
        .runner_executables()
        .iter()
        .chain(bound_context.adapter_executables());
    let factory_ids = executables
        .clone()
        .map(|executable| executable.factory_id.as_str())
        .collect::<BTreeSet<_>>();
    assert!(
        factory_ids.len() > 2,
        "fixture must bind distinct factories"
    );
    assert!(executables
        .clone()
        .all(|executable| executable.binary_digest == content(0xe2)));
    assert!(run_admitted
        .capability_implementations()
        .eq(bound_context.capability_implementations().iter()));
    scheduler
        .validate_admitted_run_binding(&authority)
        .expect("binding validation");
}

#[tokio::test]
async fn run_start_rejects_invalid_capability_implementation_bindings() {
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
                    output_digest: content(0xa2),
                },
            ),
            Case::DescriptorMismatch => {
                let mut registry = test_runner_registry();
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
                register_fixture_read_runner(&mut registry, &fixture, READ_EXTERNAL_RUNNER);
                registry
            }
        };
        let scheduler = test_scheduler(registry);
        let store = TestTypedRunStore::new();
        let error =
            prepare_fixture_launch(&scheduler, &store, &fixture, vec![fixture.seed_ref.clone()])
                .await
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
        CapabilityImplementationMismatch,
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
            Case::CapabilityImplementationMismatch,
            "capability implementation identities",
            true,
        ),
    ] {
        let fixture = fixture();
        let original_scheduler = test_scheduler(registered_fixture_runners(&fixture));
        let mut store = TestTypedRunStore::new();
        let current = started_fixture_current(
            &original_scheduler,
            &mut store,
            &fixture,
            vec![fixture.seed_ref.clone()],
        )
        .await
        .expect("start original fixture");
        let mut record_count_before = 0;
        let _ = current.lifecycle().visit_records(|_| {
            record_count_before += 1;
            std::ops::ControlFlow::<()>::Continue(())
        });

        let resume_scheduler = match case {
            Case::MissingDownstreamBinding => {
                let mut partial_registry = test_runner_registry();
                register_default_fixture_pure_runner(&mut partial_registry, &fixture);
                test_scheduler(partial_registry)
            }
            Case::RunnerExecutableMismatch => {
                let changed_digest = content(0xee);
                let mut changed_registry = test_runner_registry_with_digest(changed_digest.clone());
                let changed_a = binding_with_digest(
                    fixture.descriptor_a.clone(),
                    "pure",
                    changed_digest.clone(),
                    RecordingRunner {
                        expected_caps: Vec::new(),
                        output_digest: content(0xa2),
                    },
                );
                changed_registry.register(changed_a).expect("binding a");
                changed_registry
                    .register(binding_with_digest(
                        fixture.descriptor_b.clone(),
                        READ_EXTERNAL_RUNNER,
                        changed_digest.clone(),
                        RecordingRunner {
                            expected_caps: vec![(
                                fixture.cap_kind.clone(),
                                fixture.cap_version.clone(),
                            )],
                            output_digest: content(0xb2),
                        },
                    ))
                    .expect("changed read binding");
                register_spec_capabilities_with_adapter_executable(
                    &mut changed_registry,
                    &fixture.runtime_spec,
                    test_adapter_executable_identity_with_digest(changed_digest),
                );
                let changed_scheduler = test_scheduler(changed_registry);
                let mut changed_store = TestTypedRunStore::new();
                let changed_current = started_fixture_current(
                    &changed_scheduler,
                    &mut changed_store,
                    &fixture,
                    vec![fixture.seed_ref.clone()],
                )
                .await
                .expect("admit fixture under changed executable digest");
                let original_admission =
                    current.lifecycle().admission().expect("original admission");
                let changed_admission = changed_current
                    .lifecycle()
                    .admission()
                    .expect("changed admission");
                assert!(!original_admission
                    .runner_executables()
                    .eq(changed_admission.runner_executables()));
                assert!(!original_admission
                    .adapter_executables()
                    .eq(changed_admission.adapter_executables()));
                changed_scheduler
            }
            Case::CapabilityImplementationMismatch => {
                let mut changed_registry = test_runner_registry();
                let implementation_id =
                    CapabilityImplementationId::new("mfm.test.changed-capability")
                        .expect("changed capability implementation");
                for node in fixture
                    .runtime_spec
                    .executable_nodes()
                    .expect("certified executable nodes")
                {
                    changed_registry
                        .register_capability_set(
                            &node.capability_bindings,
                            implementation_id.clone(),
                        )
                        .expect("changed capability binding");
                    for adapter in &node.adapter_bindings {
                        changed_registry
                            .register_adapter_executable(AdapterExecutableBinding::new(
                                adapter.adapter_kind.clone(),
                                adapter.adapter_version.clone(),
                                test_adapter_executable_identity(),
                            ))
                            .expect("adapter executable binding");
                    }
                }
                register_default_fixture_pure_runner(&mut changed_registry, &fixture);
                register_fixture_read_runner(&mut changed_registry, &fixture, READ_EXTERNAL_RUNNER);
                test_scheduler(changed_registry)
            }
        };

        if direct_validation_rejects {
            resume_scheduler
                .validate_admitted_run_binding(&current)
                .expect_err("binding validation should reject");
        }

        let error = drive_current_once_with_claim(&resume_scheduler, &store, current)
            .await
            .expect_err("changed binding should reject bound context");
        assert!(
            matches!(&error, RuntimeError::RunnerBinding(message) if message.contains(expected_message)),
            "{case:?} returned unexpected error: {error:?}"
        );
        let reloaded = load_fixture_current(&original_scheduler, &store, &fixture)
            .await
            .expect("reload unchanged original run");
        let mut record_count_after = 0;
        let _ = reloaded.lifecycle().visit_records(|_| {
            record_count_after += 1;
            std::ops::ControlFlow::<()>::Continue(())
        });
        assert_eq!(record_count_after, record_count_before);
    }
}
