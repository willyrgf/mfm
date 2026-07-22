use super::*;

pub(super) fn started_fixture_projection_and_stream(
    fixture: &Fixture,
) -> (store::ProjectionSnapshot, Vec<store::KernelEventEnvelope>) {
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
    block_on_ready(start_fixture_run(
        &scheduler,
        &mut store,
        fixture,
        vec![fixture.seed_ref.clone()],
    ))
    .expect("start fixture for prepared runner context");
    (
        store.projection_snapshot().clone(),
        store.load_run_stream(&fixture.run_id),
    )
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
    let (projections, run_stream) = started_fixture_projection_and_stream(fixture);
    with_prepared_runner_ctx!(fixture, node, &attempt_id, projections, run_stream, |ctx| {
        test(ctx)
    },)
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
    let (projections, run_stream) = started_fixture_projection_and_stream(fixture);
    with_prepared_runner_ctx!(fixture, node, &attempt_id, projections, run_stream, |ctx| {
        SideEffectDriver::drive(ctx, callbacks).await
    },)
}

pub(super) async fn drive_side_effect_driver_from_store<C>(
    fixture: &Fixture,
    store: &TestTypedRunStore,
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    callbacks: &C,
) -> Result<ErasedRunnerOutput>
where
    C: SideEffectAdapter + ?Sized,
{
    with_prepared_runner_ctx!(
        fixture,
        node,
        attempt_id,
        store.projection_snapshot().clone(),
        store.load_run_stream(&fixture.run_id),
        |ctx| { SideEffectDriver::drive(ctx, callbacks).await },
    )
}

pub(super) async fn drive_side_effect_verify_driver_from_store<C>(
    fixture: &Fixture,
    store: &TestTypedRunStore,
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    callbacks: &C,
) -> Result<ErasedRunnerOutput>
where
    C: SideEffectAdapter + ?Sized,
{
    with_prepared_runner_ctx!(
        fixture,
        node,
        attempt_id,
        store.projection_snapshot().clone(),
        store.load_run_stream(&fixture.run_id),
        |ctx| { SideEffectVerifyDriver::drive(ctx, callbacks).await },
    )
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

    assert_eq!(
        drive_once_with_claim(&scheduler, &store, &fixture.runtime_spec, &fixture.run_id)
            .await
            .expect("drive after injected stale terminal append"),
        SchedulerStatus::Advanced
    );
    assert!(
        store
            .projection_snapshot(&fixture.run_id)
            .await
            .cell_terminal(&fixture.cell_a)
            .is_some(),
        "scheduler must reload and observe the concurrently advanced terminal projection"
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
    drive_once_with_claim(
        &appended_scheduler,
        &appended_store,
        &fixture.runtime_spec,
        &fixture.run_id,
    )
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
    drive_once_with_claim(
        &uncertain_scheduler,
        &uncertain_store,
        &fixture.runtime_spec,
        &fixture.run_id,
    )
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
    let mut registry = ErasedRunnerRegistry::new();
    register_spec_capabilities(&mut registry, &fixture.runtime_spec);
    registry
        .register(binding(
            fixture.descriptor_a.clone(),
            "pure",
            SettlementFixtureRunner {
                inner: RecordingRunner {
                    expected_caps: Vec::new(),
                    output_artifact: artifact(0xa1),
                    output_digest: content(0xa2),
                },
                settled,
            },
        ))
        .expect("settlement runner binding");
    register_fixture_read_runner(&mut registry, fixture, "read");
    registry
}

pub(super) fn attempt_started_count(
    store: &TestTypedRunStore,
    run_id: &RunId,
    node_id: &NodeId,
) -> usize {
    store
        .load_run_stream(run_id)
        .iter()
        .filter(|event| {
            matches!(
                event.payload(),
                events::KernelEventPayload::StateAttemptStarted(payload)
                    if &payload.node_id == node_id
            )
        })
        .count()
}

pub(super) fn runtime_lifecycle_summary(store: &TestTypedRunStore, run_id: &RunId) -> String {
    let projections = store.projection_snapshot();
    let mut started = 0;
    let mut completed = 0;
    let mut failed = 0;
    let mut interrupted = 0;
    for (_, attempt) in projections
        .attempts()
        .filter(|(_, attempt)| &attempt.run_id == run_id)
    {
        match attempt.status {
            store::AttemptStatus::Started { .. } => started += 1,
            store::AttemptStatus::Completed { .. } => completed += 1,
            store::AttemptStatus::Failed { .. } => failed += 1,
            store::AttemptStatus::Interrupted => interrupted += 1,
        }
    }
    let total = started + completed + failed + interrupted;
    let run_attempts = projections
        .attempts()
        .filter(|(_, attempt)| &attempt.run_id == run_id)
        .map(|((node_id, attempt_id), _)| (node_id.clone(), attempt_id.clone()))
        .collect::<BTreeSet<_>>();
    let cells = projections
        .cells()
        .filter(|(_, _, terminal)| match terminal {
            store::CellTerminalProjection::Produced {
                node_id,
                attempt_id,
                ..
            }
            | store::CellTerminalProjection::Skipped {
                node_id,
                attempt_id,
                ..
            } => run_attempts.contains(&(node_id.clone(), attempt_id.clone())),
        })
        .count();
    let side_effects = projections
        .side_effects()
        .filter(|(_, side_effect)| &side_effect.run_id == run_id)
        .count();
    let lanes_total = projections.resource_lanes().count();
    let lanes = projections
        .resource_lanes()
        .filter(|(_, lane)| &lane.holder.run_id == run_id)
        .count();
    let public_outputs = projections.public_outputs().count();
    let retentions = projections
        .retentions()
        .filter(|(retention_run_id, _)| *retention_run_id == run_id)
        .count();
    format!(
        "run={:?} attempts[started={started} completed={completed} failed={failed} interrupted={interrupted} total={total}] cells={cells} side_effects={side_effects} lanes[run={lanes} total={lanes_total}] public_outputs={public_outputs} retentions={retentions}",
        projections.run_state(run_id)
    )
}

pub(super) fn assert_node_failed_with_code(
    store: &TestTypedRunStore,
    node_id: &NodeId,
    code: &str,
) -> AttemptId {
    assert_node_failed_with_code_and_retryable(store, node_id, code, false)
}

pub(super) fn assert_node_failed_with_code_and_retryable(
    store: &TestTypedRunStore,
    node_id: &NodeId,
    code: &str,
    expected_retryable: bool,
) -> AttemptId {
    let failures = store
        .projection_snapshot()
        .attempts()
        .filter_map(|((attempt_node_id, attempt_id), attempt)| {
            if attempt_node_id != node_id {
                return None;
            }
            let store::AttemptStatus::Failed { retryable, error } = &attempt.status else {
                return None;
            };
            Some((attempt_id.clone(), *retryable, error.as_ref().clone()))
        })
        .collect::<Vec<_>>();
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
        diagnostic
            .schema_id
            .as_str()
            .contains("schema:mfm.runtime.redacted_attempt_failure_diagnostic:3:sha256-jcs-v1:"),
        "diagnostic schema id should identify the runtime redacted failure diagnostic schema"
    );
    let retained = store
        .projection_snapshot()
        .retentions()
        .any(|(_, retention)| {
            retention
                .refs
                .values()
                .find(|retention_ref| retention_ref.artifact_id == diagnostic.artifact_id)
                .is_some_and(|retention_ref| {
                    retention_ref.role == events::ArtifactRole::RedactedDiagnostic
                        && retention_ref.content_digest == diagnostic.content_digest
                })
        });
    assert!(
        retained,
        "failure-safe diagnostic artifact should be retained as runtime evidence"
    );
    attempt_id
}

pub(super) fn assert_failure_code_count(store: &TestTypedRunStore, code: &str, expected: usize) {
    let count = store
        .projection_snapshot()
        .attempts()
        .filter(|(_, attempt)| {
            matches!(
                &attempt.status,
                store::AttemptStatus::Failed { error, .. } if error.code.as_str() == code
            )
        })
        .count();
    assert_eq!(count, expected, "failure code count for {code}");
}

#[test]
pub(super) fn runtime_order_is_deterministic_for_reordered_spec_nodes() {
    let fixture = fixture();
    let mut envelope = fixture.runtime_spec.envelope().clone();
    envelope.spec.nodes.reverse();
    let envelope = spec::HashedSpecEnvelope::new(envelope.spec, envelope.audit).expect("rehash");
    let runtime = CertifiedRuntimeSpec::from_verified_envelope(envelope).expect("runtime");
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
    let mut registry = ErasedRunnerRegistry::new();
    register_spec_capabilities_with_adapter_executable(
        &mut registry,
        &fixture.runtime_spec,
        adapter_executable,
    );
    register_default_fixture_pure_runner(&mut registry, fixture);
    register_fixture_read_runner(&mut registry, fixture, "read");
    registry
}

pub(super) fn registered_context_bound_fixture_runners(
    fixture: &Fixture,
    source_runner: ContextSourceRunner,
) -> ErasedRunnerRegistry {
    let mut registry = ErasedRunnerRegistry::new();
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
    let mut registry = ErasedRunnerRegistry::new();
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
    let mut registry = ErasedRunnerRegistry::new();
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
    let mut registry = ErasedRunnerRegistry::new();
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
    let mut registry = ErasedRunnerRegistry::new();
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
    let factory_id = events::RunnerFactoryId::new(factory).expect("factory");
    ErasedRunnerBinding::new(
        descriptor_id,
        factory_id.clone(),
        events::ExecutableIdentity {
            factory_id,
            binary_digest: content(0xe2),
        },
        Arc::new(runner),
    )
    .expect("runner binding")
}
