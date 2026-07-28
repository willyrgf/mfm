use super::*;

fn started_fixture_context(fixture: &Fixture) -> VerifiedCurrentRun {
    let has_side_effect_nodes = fixture.runtime_spec.spec().nodes.iter().any(|node| {
        node.side_effect.is_some()
            || matches!(
                node.framework,
                Some(spec::FrameworkNodeSpec::SideEffectVerify(_))
            )
    });
    let registry = if has_side_effect_nodes {
        registered_side_effect_fixture_runners(fixture)
    } else {
        registered_fixture_runners(fixture)
    };
    let scheduler = test_scheduler(registry);
    let mut store = TestTypedRunStore::new();
    block_on_ready(started_fixture_current(
        &scheduler,
        &mut store,
        fixture,
        vec![fixture.seed_ref.clone()],
    ))
    .expect("start fixture current-run context")
}

fn with_current_runner_erased_ctx<R>(
    current: &VerifiedCurrentRun,
    node_id: &NodeId,
    attempt_id: &AttemptId,
    test: impl for<'a> FnOnce(ErasedRunCtx<'a>) -> R,
) -> R {
    let runtime_spec = current.runtime_spec();
    let node = runtime_spec.node(node_id).expect("certified node");
    let descriptor = runtime_spec
        .state_descriptor_for_node(node)
        .expect("state descriptor");
    let output_cell = runtime_spec.cell(&node.output_cell).expect("output cell");
    let lifecycle = current.lifecycle();
    let invocation =
        crate::invocation::InvocationBuilder::new(crate::invocation::InvocationBuilderInput {
            runtime_spec,
            run_id: current.view().run_id(),
            node,
            descriptor,
            output_cell,
            attempt_id,
            attempt_no: 1,
            lifecycle,
        })
        .build()
        .expect("prepared runner invocation");
    test(ErasedRunCtx::from_prepared(&invocation))
}

pub(super) fn with_runner_erased_ctx<R, F>(fixture: &Fixture, cell_id: &CellId, test: F) -> R
where
    F: for<'a> FnOnce(ErasedRunCtx<'a>) -> R,
{
    let node = node_by_output(fixture, cell_id);
    with_runner_erased_ctx_for_node(fixture, node, test)
}

pub(super) fn with_runner_erased_ctx_for_node<R, F>(
    fixture: &Fixture,
    node: &spec::NodeSpec,
    test: F,
) -> R
where
    F: for<'a> FnOnce(ErasedRunCtx<'a>) -> R,
{
    let attempt_id = attempt_id(
        &fixture.run_id,
        fixture.runtime_spec.spec_hash(),
        &node.node_id,
        1,
    )
    .expect("attempt id");
    let current = started_fixture_context(fixture);
    with_current_runner_erased_ctx(&current, &node.node_id, &attempt_id, test)
}

pub(super) async fn drive_side_effect_driver_empty<C>(
    fixture: &Fixture,
    cell_id: &CellId,
    callbacks: &C,
) -> Result<ErasedRunnerOutput>
where
    C: SideEffectAdapter + ?Sized,
{
    let node = node_by_output(fixture, cell_id);
    let attempt_id = attempt_id(
        &fixture.run_id,
        fixture.runtime_spec.spec_hash(),
        &node.node_id,
        1,
    )
    .expect("attempt id");
    let current = started_fixture_context(fixture);
    let runtime_spec = current.runtime_spec();
    let current_node = runtime_spec.node(&node.node_id).expect("certified node");
    let descriptor = runtime_spec
        .state_descriptor_for_node(current_node)
        .expect("state descriptor");
    let output_cell = runtime_spec
        .cell(&current_node.output_cell)
        .expect("output cell");
    let invocation =
        crate::invocation::InvocationBuilder::new(crate::invocation::InvocationBuilderInput {
            runtime_spec,
            run_id: current.view().run_id(),
            node: current_node,
            descriptor,
            output_cell,
            attempt_id: &attempt_id,
            attempt_no: 1,
            lifecycle: current.lifecycle(),
        })
        .build()?;
    SideEffectDriver::drive(ErasedRunCtx::from_prepared(&invocation), callbacks).await
}

pub(super) async fn drive_side_effect_driver_from_current<C>(
    current: &VerifiedCurrentRun,
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    callbacks: &C,
) -> Result<ErasedRunnerOutput>
where
    C: SideEffectAdapter + ?Sized,
{
    let runtime_spec = current.runtime_spec();
    let current_node = runtime_spec
        .node(&node.node_id)
        .ok_or_else(|| RuntimeError::InvalidSpec("test node is not certified".to_owned()))?;
    let descriptor = runtime_spec.state_descriptor_for_node(current_node)?;
    let output_cell = runtime_spec
        .cell(&current_node.output_cell)
        .ok_or_else(|| RuntimeError::InvalidSpec("test output cell is not certified".to_owned()))?;
    let invocation =
        crate::invocation::InvocationBuilder::new(crate::invocation::InvocationBuilderInput {
            runtime_spec,
            run_id: current.view().run_id(),
            node: current_node,
            descriptor,
            output_cell,
            attempt_id,
            attempt_no: 1,
            lifecycle: current.lifecycle(),
        })
        .build()?;
    SideEffectDriver::drive(ErasedRunCtx::from_prepared(&invocation), callbacks).await
}

pub(super) async fn drive_side_effect_verify_driver_from_current<C>(
    current: &VerifiedCurrentRun,
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    callbacks: &C,
) -> Result<ErasedRunnerOutput>
where
    C: SideEffectAdapter + ?Sized,
{
    let runtime_spec = current.runtime_spec();
    let current_node = runtime_spec
        .node(&node.node_id)
        .ok_or_else(|| RuntimeError::InvalidSpec("test node is not certified".to_owned()))?;
    let descriptor = runtime_spec.state_descriptor_for_node(current_node)?;
    let output_cell = runtime_spec
        .cell(&current_node.output_cell)
        .ok_or_else(|| RuntimeError::InvalidSpec("test output cell is not certified".to_owned()))?;
    let invocation =
        crate::invocation::InvocationBuilder::new(crate::invocation::InvocationBuilderInput {
            runtime_spec,
            run_id: current.view().run_id(),
            node: current_node,
            descriptor,
            output_cell,
            attempt_id,
            attempt_no: 1,
            lifecycle: current.lifecycle(),
        })
        .build()?;
    SideEffectVerifyDriver::drive(ErasedRunCtx::from_prepared(&invocation), callbacks).await
}

#[tokio::test]
pub(super) async fn scheduler_reloads_and_redecides_after_stale_expected_sequence_on_terminal_append(
) {
    let fixture = fixture();
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let store = StaleOnceTypedRunStore::new();
    start_fixture_run_async_store(&scheduler, &store, &fixture, vec![fixture.seed_ref.clone()])
        .await
        .expect("start run");
    let current = load_fixture_current(&scheduler, &store, &fixture)
        .await
        .expect("load current run");
    let result = drive_current_once_with_claim(&scheduler, &store, current)
        .await
        .expect("drive after injected stale terminal append");
    assert_eq!(result.status(), SchedulerStatus::Advanced);
    assert!(
        result
            .current_run()
            .lifecycle()
            .cell(&fixture.cell_a)
            .is_some(),
        "scheduler must reload and observe the concurrently advanced terminal cell"
    );
}

#[tokio::test]
async fn runner_output_settlement_requires_an_appended_outcome() {
    let fixture = fixture();

    let appended_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let appended_scheduler = test_scheduler(settlement_fixture_runners(
        &fixture,
        Arc::clone(&appended_count),
    ));
    let appended_store = RecordingTypedRunStore::new();
    start_fixture_run_async_store(
        &appended_scheduler,
        &appended_store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start appended settlement run");
    let appended_current = load_fixture_current(&appended_scheduler, &appended_store, &fixture)
        .await
        .expect("load appended settlement run");
    drive_current_once_with_claim(&appended_scheduler, &appended_store, appended_current)
        .await
        .expect("drive appended settlement");
    assert_eq!(appended_count.load(std::sync::atomic::Ordering::SeqCst), 1);

    let uncertain_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let uncertain_scheduler = test_scheduler(settlement_fixture_runners(
        &fixture,
        Arc::clone(&uncertain_count),
    ));
    let uncertain_store = StaleOnceTypedRunStore::new();
    start_fixture_run_async_store(
        &uncertain_scheduler,
        &uncertain_store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start uncertain settlement run");
    let uncertain_current = load_fixture_current(&uncertain_scheduler, &uncertain_store, &fixture)
        .await
        .expect("load uncertain settlement run");
    drive_current_once_with_claim(&uncertain_scheduler, &uncertain_store, uncertain_current)
        .await
        .expect("drive uncertain settlement");
    assert_eq!(
        uncertain_count.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "an append error must discard process-local authority even if a retry observes the commit"
    );
}

struct SettlementFixtureRunner {
    inner: RecordingRunner,
    settled: Arc<std::sync::atomic::AtomicUsize>,
}

impl ErasedNodeRunner for SettlementFixtureRunner {
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move {
            let output = self.inner.run_erased(ctx).await?;
            let settled = Arc::clone(&self.settled);
            Ok(
                output.with_settlement(RunnerOutputSettlement::on_appended(move || {
                    settled.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                })),
            )
        })
    }
}

fn settlement_fixture_runners(
    fixture: &Fixture,
    settled: Arc<std::sync::atomic::AtomicUsize>,
) -> ErasedRunnerRegistry {
    let mut registry = test_runner_registry();
    register_spec_capabilities(&mut registry, &fixture.runtime_spec);
    registry
        .register(binding(
            fixture.descriptor_a.clone(),
            "pure",
            SettlementFixtureRunner {
                inner: RecordingRunner {
                    expected_caps: Vec::new(),
                    output_digest: content(0xa2),
                },
                settled,
            },
        ))
        .expect("settlement runner binding");
    register_fixture_read_runner(&mut registry, fixture, READ_EXTERNAL_RUNNER);
    registry
}

pub(super) fn attempt_started_count(current: &VerifiedCurrentRun, node_id: &NodeId) -> usize {
    let mut count = 0;
    let _ = current.lifecycle().visit_attempts::<()>(|attempt| {
        if attempt.node_id() == node_id {
            count += 1;
        }
        std::ops::ControlFlow::Continue(())
    });
    count
}

pub(super) fn runtime_lifecycle_summary(current: &VerifiedCurrentRun) -> String {
    let lifecycle = current.lifecycle();
    let mut started = 0;
    let mut completed = 0;
    let mut failed = 0;
    let mut interrupted = 0;
    let _ = lifecycle.visit_attempts::<()>(|attempt| {
        match attempt.status() {
            store::current_lifecycle::CurrentAttemptStatusRef::Started { .. } => started += 1,
            store::current_lifecycle::CurrentAttemptStatusRef::Completed { .. } => completed += 1,
            store::current_lifecycle::CurrentAttemptStatusRef::Failed { .. } => failed += 1,
            store::current_lifecycle::CurrentAttemptStatusRef::Interrupted => interrupted += 1,
        }
        std::ops::ControlFlow::Continue(())
    });
    let total = started + completed + failed + interrupted;
    let runtime_spec = current.runtime_spec();
    let cells = runtime_spec
        .spec()
        .cells
        .iter()
        .filter(|cell| lifecycle.cell(&cell.cell_id).is_some())
        .count();
    let mut side_effects = 0;
    let _ = lifecycle.visit_side_effects::<()>(|_| {
        side_effects += 1;
        std::ops::ControlFlow::Continue(())
    });
    let mut lanes = 0;
    let _ = lifecycle.visit_resource_lanes::<()>(|_| {
        lanes += 1;
        std::ops::ControlFlow::Continue(())
    });
    let public_outputs = usize::from(
        lifecycle
            .public_output(&runtime_spec.spec().public_outputs.public_schema_id)
            .is_some(),
    );
    let retentions = usize::from(lifecycle.retention().is_some());
    format!(
        "run={:?} attempts[started={started} completed={completed} failed={failed} interrupted={interrupted} total={total}] cells={cells} side_effects={side_effects} lanes={lanes} public_outputs={public_outputs} retentions={retentions}",
        lifecycle.run_state()
    )
}

pub(super) fn assert_node_failed_with_code(
    current: &VerifiedCurrentRun,
    node_id: &NodeId,
    code: &str,
) -> AttemptId {
    assert_node_failed_with_code_and_retryable(current, node_id, code, false)
}

pub(super) fn assert_node_failed_with_code_and_retryable(
    current: &VerifiedCurrentRun,
    node_id: &NodeId,
    code: &str,
    expected_retryable: bool,
) -> AttemptId {
    let mut failures = Vec::new();
    let _ = current.lifecycle().visit_attempts::<()>(|attempt| {
        if attempt.node_id() == node_id {
            if let store::current_lifecycle::CurrentAttemptStatusRef::Failed { retryable, error } =
                attempt.status()
            {
                failures.push((attempt.attempt_id().clone(), retryable, error.clone()));
            }
        }
        std::ops::ControlFlow::Continue(())
    });
    assert_eq!(
        failures.len(),
        1,
        "expected one failed attempt for node {node_id}"
    );
    let (attempt_id, retryable, error) = failures.into_iter().next().expect("failure");
    assert_eq!(
        retryable, expected_retryable,
        "failure-safe retryability for {code}"
    );
    assert_eq!(error.code.as_str(), code);
    assert_eq!(error.retryable, retryable);
    let diagnostic = error
        .diagnostic_ref
        .as_ref()
        .expect("failure-safe terminalization records redacted diagnostic evidence");
    assert_eq!(diagnostic.role, events::ArtifactRole::RedactedDiagnostic);
    assert_eq!(diagnostic.semantic_type_id, None);
    assert_eq!(
        diagnostic.media_type,
        spec::MediaType::new("application/json").expect("media type")
    );
    assert!(
        diagnostic.schema_id.as_str().contains(&format!(
            "schema:{REDACTED_ATTEMPT_FAILURE_DIAGNOSTIC_SCHEMA}:{REDACTED_ATTEMPT_FAILURE_DIAGNOSTIC_VERSION}:sha256-jcs-v1:"
        )),
        "diagnostic schema id should identify the runtime redacted failure diagnostic schema"
    );
    let mut retained = false;
    if let Some(retention) = current.lifecycle().retention() {
        let _ = retention.visit_references::<()>(|retention_ref| {
            if retention_ref.artifact_id == diagnostic.artifact_id
                && retention_ref.role == events::ArtifactRole::RedactedDiagnostic
                && retention_ref.content_digest == diagnostic.content_digest
            {
                retained = true;
            }
            std::ops::ControlFlow::Continue(())
        });
    }
    assert!(
        retained,
        "failure-safe diagnostic artifact should be retained as runtime evidence"
    );
    attempt_id
}

pub(super) fn assert_failure_code_count(current: &VerifiedCurrentRun, code: &str, expected: usize) {
    let mut count = 0;
    let _ = current.lifecycle().visit_attempts::<()>(|attempt| {
        if matches!(
            attempt.status(),
            store::current_lifecycle::CurrentAttemptStatusRef::Failed { error, .. }
                if error.code.as_str() == code
        ) {
            count += 1;
        }
        std::ops::ControlFlow::Continue(())
    });
    assert_eq!(count, expected, "failure code count for {code}");
}

#[test]
pub(super) fn runtime_order_is_deterministic_for_reordered_spec_nodes() {
    let fixture = fixture();
    let mut reordered = fixture.runtime_spec.spec().clone();
    reordered.nodes.reverse();
    let certified = certify_fixture_spec(&fixture.runtime_spec, reordered, |_| Ok(()))
        .expect("certified reordered runtime spec");
    let runtime = CertifiedRuntimeSpec::new(certified).expect("runtime");
    assert_eq!(
        runtime.topological_order(),
        fixture.runtime_spec.topological_order()
    );
}

pub(super) fn registered_fixture_runners(fixture: &Fixture) -> ErasedRunnerRegistry {
    registered_fixture_runners_with_adapter_executable(fixture, test_adapter_executable_identity())
}

pub(super) fn registered_fixture_runners_with_adapter_executable(
    fixture: &Fixture,
    adapter_executable: events::ExecutableIdentity,
) -> ErasedRunnerRegistry {
    let mut registry = test_runner_registry();
    register_spec_capabilities_with_adapter_executable(
        &mut registry,
        &fixture.runtime_spec,
        adapter_executable,
    );
    register_default_fixture_pure_runner(&mut registry, fixture);
    register_fixture_read_runner(&mut registry, fixture, READ_EXTERNAL_RUNNER);
    registry
}

pub(super) fn registered_context_bound_fixture_runners(
    fixture: &Fixture,
    source_runner: ContextSourceRunner,
) -> ErasedRunnerRegistry {
    let mut registry = test_runner_registry();
    register_spec_capabilities(&mut registry, &fixture.runtime_spec);
    registry
        .register(binding(fixture.descriptor_a.clone(), "pure", source_runner))
        .expect("context source binding");
    registry
        .register(binding(
            fixture.descriptor_b.clone(),
            "pure",
            ContextConsumerRunner,
        ))
        .expect("context consumer binding");
    registry
}

pub(super) fn registered_side_effect_fixture_runners(fixture: &Fixture) -> ErasedRunnerRegistry {
    let mut registry = test_runner_registry();
    register_spec_capabilities(&mut registry, &fixture.runtime_spec);
    register_side_effect_verify_fixture_runner(&mut registry, fixture);
    registry
        .register(binding(
            fixture.descriptor_a.clone(),
            APPLY_SIDE_EFFECT_RUNNER,
            DriverSideEffectRunner::new(fixture),
        ))
        .expect("binding a");
    register_fixture_read_runner(&mut registry, fixture, READ_EXTERNAL_RUNNER);
    registry
}

pub(super) fn registered_first_side_effect_runners_with<R: ErasedNodeRunner + 'static>(
    fixture: &Fixture,
    runner: R,
) -> ErasedRunnerRegistry {
    let mut registry = test_runner_registry();
    register_spec_capabilities(&mut registry, &fixture.runtime_spec);
    register_side_effect_verify_fixture_runner(&mut registry, fixture);
    registry
        .register(binding(
            fixture.descriptor_a.clone(),
            APPLY_SIDE_EFFECT_RUNNER,
            runner,
        ))
        .expect("binding a");
    register_fixture_read_runner(&mut registry, fixture, READ_EXTERNAL_RUNNER);
    registry
}

pub(super) fn registered_first_side_effect_and_verify_runners_with<
    R: ErasedNodeRunner + 'static,
    V: ErasedNodeRunner + 'static,
>(
    fixture: &Fixture,
    runner: R,
    verify_runner: V,
) -> ErasedRunnerRegistry {
    let mut registry = test_runner_registry();
    register_spec_capabilities(&mut registry, &fixture.runtime_spec);
    let submit_descriptor_id = side_effect_submit_descriptor_ids(fixture)
        .into_iter()
        .next()
        .expect("side-effect submit descriptor");
    register_side_effect_verify_runner_with(&mut registry, submit_descriptor_id, verify_runner);
    registry
        .register(binding(
            fixture.descriptor_a.clone(),
            APPLY_SIDE_EFFECT_RUNNER,
            runner,
        ))
        .expect("binding a");
    register_fixture_read_runner(&mut registry, fixture, READ_EXTERNAL_RUNNER);
    registry
}

pub(super) fn compensated_saga_scheduler(fixture: &Fixture) -> SerialTypedScheduler {
    let mut registry = test_runner_registry();
    register_spec_capabilities(&mut registry, &fixture.runtime_spec);
    register_side_effect_verify_fixture_runner(&mut registry, fixture);
    registry
        .register(binding(
            fixture.descriptor_a.clone(),
            APPLY_SIDE_EFFECT_RUNNER,
            DriverSideEffectRunner::new(fixture),
        ))
        .expect("binding forward a");
    registry
        .register(binding(
            fixture.descriptor_b.clone(),
            APPLY_SIDE_EFFECT_RUNNER,
            DriverSideEffectRunner::new(fixture),
        ))
        .expect("binding forward b");
    registry
        .register(binding(
            fixture
                .descriptor_c
                .clone()
                .expect("failing node descriptor"),
            "pure",
            BlockingRunner,
        ))
        .expect("binding failure node");
    test_scheduler(registry)
}

pub(super) fn register_side_effect_verify_fixture_runner(
    registry: &mut ErasedRunnerRegistry,
    fixture: &Fixture,
) {
    for descriptor_id in side_effect_submit_descriptor_ids(fixture) {
        register_side_effect_verify_runner_with(
            registry,
            descriptor_id,
            DriverSideEffectVerifyRunner::new(fixture),
        );
    }
}

pub(super) fn register_side_effect_verify_runner_with<R: ErasedNodeRunner + 'static>(
    registry: &mut ErasedRunnerRegistry,
    submit_descriptor_id: DescriptorId,
    runner: R,
) {
    let factory_id = events::RunnerFactoryId::new("read_external").expect("factory");
    registry
        .register_side_effect_verify_runner(
            submit_descriptor_id,
            factory_id.clone(),
            events::ExecutableIdentity {
                factory_id,
                binary_digest: content(0xe2),
            },
            Arc::new(runner),
        )
        .expect("side-effect verify binding");
}

pub(super) fn side_effect_submit_descriptor_ids(fixture: &Fixture) -> Vec<DescriptorId> {
    let mut descriptors = BTreeSet::new();
    for node in fixture
        .runtime_spec
        .spec()
        .nodes
        .iter()
        .chain(fixture.runtime_spec.spec().remediations.values())
    {
        if node.side_effect.is_some() {
            descriptors.insert(node.descriptor_id.clone());
        }
    }
    descriptors.into_iter().collect()
}

pub(super) fn binding<R: ErasedNodeRunner + 'static>(
    descriptor_id: DescriptorId,
    factory: &str,
    runner: R,
) -> ErasedRunnerBinding {
    binding_with_digest(descriptor_id, factory, content(0xe2), runner)
}

pub(super) fn binding_with_digest<R: ErasedNodeRunner + 'static>(
    descriptor_id: DescriptorId,
    factory: &str,
    binary_digest: ContentDigest,
    runner: R,
) -> ErasedRunnerBinding {
    let factory_id = events::RunnerFactoryId::new(factory).expect("factory");
    ErasedRunnerBinding::new(
        descriptor_id,
        factory_id.clone(),
        events::ExecutableIdentity {
            factory_id,
            binary_digest,
        },
        Arc::new(runner),
    )
    .expect("runner binding")
}
