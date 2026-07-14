use super::*;

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
