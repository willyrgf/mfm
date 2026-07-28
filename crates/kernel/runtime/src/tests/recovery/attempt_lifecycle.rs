use super::*;

#[tokio::test]
async fn recovery_interrupts_started_pure_attempt_before_retrying_fresh_attempt() {
    let fixture = fixture();
    let (scheduler, mut store) = started_fixture_run(&fixture).await;
    let node = node_by_output(&fixture, &fixture.cell_a);
    let interrupted_attempt_id = append_attempt_start(&mut store, &fixture, node, 1);
    let mut current = load_fixture_current(&scheduler, &store, &fixture)
        .await
        .expect("load started attempt");

    assert_drive!(scheduler, store, current, Advanced, "interrupt pure");
    let interrupted = current
        .lifecycle()
        .attempt(&node.node_id, &interrupted_attempt_id)
        .expect("interrupted attempt");
    assert!(matches!(
        interrupted.status(),
        store::current_lifecycle::CurrentAttemptStatusRef::Interrupted
    ));
    assert!(
        current.lifecycle().cell(&fixture.cell_a).is_none(),
        "interruption must not terminalize the output cell"
    );

    assert_drive!(scheduler, store, current, Advanced, "retry pure");
    assert_eq!(attempt_started_count(&current, &node.node_id), 2);
    let retry_attempt_id = attempt_id(
        &fixture.run_id,
        fixture.runtime_spec.spec_hash(),
        &node.node_id,
        2,
    )
    .expect("retry attempt id");
    let produced = current
        .lifecycle()
        .cell(&fixture.cell_a)
        .and_then(|cell| cell.produced())
        .expect("produced retry cell");
    assert_eq!(produced.attempt_id(), &retry_attempt_id);
    assert_eq!(
        runtime_lifecycle_summary(&current),
        "run=Started attempts[started=0 completed=1 failed=0 interrupted=1 total=2] cells=1 side_effects=0 lanes=0 public_outputs=0 retentions=1"
    );
}

#[tokio::test]
async fn recovery_delegates_prepared_side_effect_attempt_to_its_lifecycle() {
    let fixture = fixture_with_first_exclusive_side_effect_state();
    let scheduler = test_scheduler(registered_side_effect_fixture_runners(&fixture));
    let mut store = started_fixture_store(&scheduler, &fixture).await;
    let node = node_by_output(&fixture, &fixture.cell_a);
    let (attempt_id, ledger_key) = append_synthetic_exclusive_prepare(
        &mut store,
        &fixture,
        &fixture.run_id,
        node,
        "wallet-recovery",
        "sidefx-recovery-open",
    );
    let current = load_fixture_current(&scheduler, &store, &fixture)
        .await
        .expect("load prepared side effect");
    assert_eq!(
        runtime_lifecycle_summary(&current),
        "run=Started attempts[started=1 completed=0 failed=0 interrupted=0 total=1] cells=0 side_effects=1 lanes=1 public_outputs=0 retentions=1"
    );

    let runtime_spec = current.runtime_spec();
    let lifecycle = current.lifecycle();
    match crate::recovery::AttemptRecoveryLifecycle::next_open_attempt_disposition(
        &runtime_spec,
        &lifecycle,
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
        _ => panic!("prepared side-effect attempt must delegate to side-effect recovery"),
    }

    let ledger = side_effect_for_attempt(&runtime_spec, &lifecycle, node, &attempt_id)
        .expect("side-effect lookup")
        .expect("side-effect ledger");
    assert_eq!(ledger.ledger_key(), &ledger_key);
    match ledger.ledger_state().expect("ledger state").phase() {
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

#[test]
fn side_effect_attempt_view_from_erased_context_is_empty_before_ledger() {
    let fixture = fixture_with_first_side_effect_state();
    with_runner_erased_ctx(&fixture, &fixture.cell_a, |ctx| {
        let view = SideEffectAttemptView::from_erased_context(&ctx).expect("side-effect view");
        assert!(view.ledger().is_none());
        assert!(view.phase().is_none());
    });
}

#[tokio::test]
async fn recovery_preserves_committed_claim_before_prepare() {
    for (case_label, emit_claim) in [
        ("after intent before prepare", false),
        ("after claim before prepare", true),
    ] {
        let fixture = fixture_with_first_side_effect_state();
        let scheduler = test_scheduler(registered_first_side_effect_runners_with(
            &fixture,
            PrePreparedSideEffectRunner { emit_claim },
        ));
        let store = started_fixture_store(&scheduler, &fixture).await;
        let node = node_by_output(&fixture, &fixture.cell_a).clone();
        let mut current = load_fixture_current(&scheduler, &store, &fixture)
            .await
            .expect("load current run");

        assert_drive!(
            scheduler,
            store,
            current,
            Advanced,
            "append pre-prepared side-effect evidence"
        );
        let attempt_id = attempt_id(
            &fixture.run_id,
            fixture.runtime_spec.spec_hash(),
            &node.node_id,
            1,
        )
        .expect("attempt id");
        {
            let runtime_spec = current.runtime_spec();
            let lifecycle = current.lifecycle();
            let disposition =
                crate::recovery::AttemptRecoveryLifecycle::next_open_attempt_disposition(
                    &runtime_spec,
                    &lifecycle,
                    &BTreeSet::new(),
                )
                .expect("recovery disposition")
                .expect("open side-effect attempt");
            match (emit_claim, disposition) {
                (
                    false,
                    crate::recovery::OpenAttemptDisposition::Interrupt {
                        node: recovered_node,
                        attempt_id: recovered_attempt,
                        attempt_no,
                    },
                ) => {
                    assert_eq!(recovered_node.node_id, node.node_id);
                    assert_eq!(recovered_attempt, attempt_id);
                    assert_eq!(attempt_no, 1);
                }
                (
                    true,
                    crate::recovery::OpenAttemptDisposition::DelegateSideEffect {
                        node: recovered_node,
                        attempt_id: recovered_attempt,
                        attempt_no,
                    },
                ) => {
                    assert_eq!(recovered_node.node_id, node.node_id);
                    assert_eq!(recovered_attempt, attempt_id);
                    assert_eq!(attempt_no, 1);
                    continue;
                }
                _ => panic!("{case_label}: unexpected recovery disposition"),
            }
        }

        assert_drive!(
            scheduler,
            store,
            current,
            Advanced,
            "interrupt pre-prepared side-effect attempt"
        );
        assert!(matches!(
            current
                .lifecycle()
                .attempt(&node.node_id, &attempt_id)
                .expect("attempt")
                .status(),
            store::current_lifecycle::CurrentAttemptStatusRef::Interrupted
        ));
    }
}
