use super::*;

#[tokio::test]
async fn run_admission_returns_bound_context_with_capability_and_framework_authority() {
    let fixture = fixture();
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let mut store = TestTypedRunStore::new();
    let launch =
        prepare_fixture_launch(&scheduler, &store, &fixture, vec![fixture.seed_ref.clone()])
            .await
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
    assert_eq!(
        run_admitted.capability_implementations,
        authority.bound_context().capability_implementations()
    );
    scheduler
        .validate_admitted_run_binding(&fixture.runtime_spec, &run_admitted)
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
                register_fixture_read_runner(&mut registry, &fixture, "read");
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
        AdapterExecutableMismatch,
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
            Case::AdapterExecutableMismatch,
            "adapter executable identities",
            true,
        ),
        (
            Case::CapabilityImplementationMismatch,
            "capability implementation identities",
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
                register_fixture_read_runner(&mut changed_registry, &fixture, "read");
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
            Case::CapabilityImplementationMismatch => {
                let mut changed_registry = ErasedRunnerRegistry::new();
                let implementation_id =
                    CapabilityImplementationId::new("mfm.test.changed-capability")
                        .expect("changed capability implementation");
                for node in fixture.runtime_spec.executable_nodes() {
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
                register_fixture_read_runner(&mut changed_registry, &fixture, "read");
                test_scheduler(changed_registry)
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
