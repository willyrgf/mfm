use super::*;

pub(super) fn side_effect_capability_kind() -> CapabilityKind {
    CapabilityKind::new(
        "mfm.test",
        "external-mutation",
        DigestAlgorithm::Sha256JcsV1,
        D9,
    )
    .expect("side-effect cap kind")
}

pub(super) fn side_effect_capability_version() -> CapabilityVersion {
    CapabilityVersion::new("mfm.cap.external_mutation.v1").expect("side-effect cap version")
}

pub(super) fn exclusive_resource_namespace() -> spec::ResourceNamespace {
    spec::ResourceNamespace::new("mfm.test.wallet_nonce").expect("resource namespace")
}

pub(super) fn exact_touched_set_resource_namespace() -> spec::ResourceNamespace {
    spec::ResourceNamespace::new("mfm.test.wallet_nonce").expect("resource namespace")
}

pub(super) fn exclusive_resource_key(
    fixture: &Fixture,
    value: &str,
) -> events::ResourceKeyEvidence {
    resource_key_in_namespace(fixture, exclusive_resource_namespace(), value)
}

pub(super) fn resource_key_in_namespace(
    fixture: &Fixture,
    namespace: spec::ResourceNamespace,
    value: &str,
) -> events::ResourceKeyEvidence {
    events::ResourceKeyEvidence {
        namespace,
        key_schema_id: fixture.seed_ref.schema_id.clone(),
        key: events::ResourceKey::new(value).expect("resource key"),
    }
}

pub(super) fn side_effect_ledger_key(attempt_no: u32) -> events::SideEffectLedgerKey {
    events::SideEffectLedgerKey::new(format!("ledger-{attempt_no}")).expect("ledger key")
}

pub(super) fn side_effect_ledger_purpose() -> events::SideEffectLedgerPurpose {
    events::SideEffectLedgerPurpose::Forward
}

pub(super) fn forward_ledger_for_node(
    current: &VerifiedCurrentRun,
    node_id: &NodeId,
) -> events::SideEffectLedgerKey {
    let mut found = None;
    let _ = current.lifecycle().visit_side_effects::<()>(|side_effect| {
        if side_effect.intent().node_id() == node_id
            && matches!(
                side_effect.ledger_purpose(),
                events::SideEffectLedgerPurpose::Forward
            )
        {
            assert!(
                found.replace(side_effect.ledger_key().clone()).is_none(),
                "node {node_id} has multiple forward ledgers"
            );
        }
        std::ops::ControlFlow::Continue(())
    });
    found.expect("forward ledger for node")
}

pub(super) fn side_effect_for_node<'view>(
    current: &'view VerifiedCurrentRun,
    node_id: &NodeId,
) -> Option<store::current_lifecycle::CurrentSideEffectRef<'view>> {
    let mut found = None;
    let _ = current.lifecycle().visit_side_effects::<()>(|side_effect| {
        if side_effect.intent().node_id() == node_id {
            assert!(
                found.replace(side_effect).is_none(),
                "node {node_id} has multiple side-effect ledgers"
            );
        }
        std::ops::ControlFlow::Continue(())
    });
    found
}

pub(super) fn verified_current_for_store(
    store: &TestTypedRunStore,
    fixture: &Fixture,
) -> VerifiedCurrentRun {
    let journal = block_on_ready(store.load_committed_journal(&fixture.run_id))
        .expect("load committed fixture journal");
    verify_current_run(journal, recertified_runtime_spec(&fixture.runtime_spec))
        .expect("verify fixture current run")
}

pub(super) fn with_fixture_saga<T>(
    current: &VerifiedCurrentRun,
    fixture: &Fixture,
    read: impl FnOnce(store::current_lifecycle::CurrentSagaRef<'_>) -> T,
) -> T {
    current
        .lifecycle()
        .with_saga(
            &fixture.runtime_spec.spec().saga,
            &fixture_terminal_policies(fixture),
            read,
        )
        .expect("derive fixture saga")
}

pub(super) async fn drive_until_side_effect_confirmation_without_output(
    scheduler: &SerialTypedScheduler,
    store: &mut TestTypedRunStore,
    fixture: &Fixture,
    node: &spec::NodeSpec,
    output_cell: &CellId,
    context: &str,
) {
    let mut current = load_fixture_current(scheduler, &*store, fixture)
        .await
        .expect("load current run");
    for _ in 0..8 {
        if side_effect_for_node(&current, &node.node_id).is_some_and(|side_effect| {
            matches!(
                side_effect.phase(),
                store::SideEffectPhase::ConfirmationObserved { .. }
            ) && current.lifecycle().cell(output_cell).is_none()
        }) {
            return;
        }
        let result = drive_current_once_with_claim(scheduler, &*store, current)
            .await
            .expect(context);
        assert_eq!(result.status(), SchedulerStatus::Advanced);
        current = result.into_current_run();
    }
    panic!("{context} was not reached");
}

pub(super) async fn drive_until_cells_terminal(
    scheduler: &SerialTypedScheduler,
    store: &mut TestTypedRunStore,
    fixture: &Fixture,
    cells: &[CellId],
    context: &str,
) {
    let mut current = load_fixture_current(scheduler, &*store, fixture)
        .await
        .expect("load current run");
    for _ in 0..24 {
        if cells
            .iter()
            .all(|cell| current.lifecycle().cell(cell).is_some())
        {
            return;
        }
        let result = drive_current_once_with_claim(scheduler, &*store, current)
            .await
            .expect(context);
        assert_eq!(result.status(), SchedulerStatus::Advanced);
        current = result.into_current_run();
    }
    panic!("{context} did not become terminal");
}

pub(super) async fn drive_until_side_effect_attempt_phase(
    scheduler: &SerialTypedScheduler,
    store: &mut TestTypedRunStore,
    fixture: &Fixture,
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    mut phase_matches: impl FnMut(&store::SideEffectPhase) -> bool,
    context: &str,
) {
    let mut current = load_fixture_current(scheduler, &*store, fixture)
        .await
        .expect("load current run");
    for _ in 0..24 {
        if side_effect_for_attempt(
            &current.runtime_spec(),
            &current.lifecycle(),
            node,
            attempt_id,
        )
        .expect(context)
        .is_some_and(|side_effect| phase_matches(side_effect.phase()))
        {
            return;
        }
        let result = drive_current_once_with_claim(scheduler, &*store, current)
            .await
            .expect(context);
        assert_eq!(result.status(), SchedulerStatus::Advanced);
        current = result.into_current_run();
    }
    panic!("{context} was not reached");
}

#[derive(Clone, Copy)]
pub(super) enum RemediationPhaseCheckpoint {
    SubmissionObserved,
    ConfirmationObserved,
}

pub(super) async fn drive_until_remediation_phase(
    scheduler: &SerialTypedScheduler,
    store: &mut TestTypedRunStore,
    fixture: &Fixture,
    forward_pair_id: &SideEffectPairId,
    checkpoint: RemediationPhaseCheckpoint,
    context: &str,
) {
    let mut current = load_fixture_current(scheduler, &*store, fixture)
        .await
        .expect("load current run");
    for _ in 0..8 {
        if remediation_side_effect_for_forward_pair(&current, forward_pair_id).is_some_and(
            |side_effect| match checkpoint {
                RemediationPhaseCheckpoint::SubmissionObserved => {
                    matches!(
                        side_effect.phase(),
                        store::SideEffectPhase::SubmissionObserved { .. }
                    )
                }
                RemediationPhaseCheckpoint::ConfirmationObserved => {
                    matches!(
                        side_effect.phase(),
                        store::SideEffectPhase::ConfirmationObserved { .. }
                    )
                }
            },
        ) {
            return;
        }
        let result = drive_current_once_with_claim(scheduler, &*store, current)
            .await
            .expect(context);
        assert_eq!(result.status(), SchedulerStatus::Advanced);
        current = result.into_current_run();
    }
    panic!("{context} was not reached");
}

pub(super) async fn drive_until_compensated_before_terminal(
    scheduler: &SerialTypedScheduler,
    store: &mut TestTypedRunStore,
    fixture: &Fixture,
) {
    let mut current = load_fixture_current(scheduler, &*store, fixture)
        .await
        .expect("load current run");
    let terminal_policies = fixture_terminal_policies(fixture);
    for _ in 0..24 {
        let run_mode = current
            .lifecycle()
            .with_saga(
                &fixture.runtime_spec.spec().saga,
                &terminal_policies,
                |saga| saga.run_mode(),
            )
            .expect("derive saga");
        if run_mode == store::RunMode::Compensated
            && current.lifecycle().run_state() != store::RunState::Completed
        {
            return;
        }
        let result = drive_current_once_with_claim(scheduler, &*store, current)
            .await
            .expect("drive until compensated before terminal");
        assert_eq!(result.status(), SchedulerStatus::Advanced);
        current = result.into_current_run();
    }
    panic!("compensated pre-terminal boundary was not reached");
}

pub(super) fn remediation_intent_forward_links(
    store: &TestTypedRunStore,
    fixture: &Fixture,
) -> Vec<SideEffectPairId> {
    let current = verified_current_for_store(store, fixture);
    let mut links = Vec::new();
    let _ = current.lifecycle().visit_records::<()>(|record| {
        if let store::current_lifecycle::CurrentRecordKindRef::SideEffectIntentPersisted(payload) =
            record.kind()
        {
            match &payload.ledger_purpose {
                events::SideEffectLedgerPurpose::Remediation { forward_pair_id } => {
                    links.push(forward_pair_id.clone());
                }
                events::SideEffectLedgerPurpose::Forward => {}
            }
        }
        std::ops::ControlFlow::Continue(())
    });
    links
}

pub(super) fn remediation_side_effect_for_forward_pair<'view>(
    current: &'view VerifiedCurrentRun,
    forward_pair_id: &SideEffectPairId,
) -> Option<store::current_lifecycle::CurrentSideEffectRef<'view>> {
    let mut found = None;
    let _ = current.lifecycle().visit_side_effects::<()>(|side_effect| {
        if matches!(
            side_effect.ledger_purpose(),
            events::SideEffectLedgerPurpose::Remediation {
                forward_pair_id: linked,
            } if linked == forward_pair_id
        ) {
            assert!(
                found.replace(side_effect).is_none(),
                "forward pair {forward_pair_id} has multiple remediation ledgers"
            );
        }
        std::ops::ControlFlow::Continue(())
    });
    found
}

pub(super) fn assert_no_duplicate_side_effect_submissions(
    store: &TestTypedRunStore,
    fixture: &Fixture,
) {
    let current = verified_current_for_store(store, fixture);
    let mut by_ledger = BTreeMap::<events::SideEffectLedgerKey, usize>::new();
    let mut forward_by_node = BTreeMap::<NodeId, usize>::new();
    let mut remediation_by_forward = BTreeMap::<SideEffectPairId, usize>::new();
    let _ = current.lifecycle().visit_records::<()>(|record| {
        if let store::current_lifecycle::CurrentRecordKindRef::SideEffectSubmissionObserved(
            payload,
        ) = record.kind()
        {
            *by_ledger.entry(payload.ledger_key.clone()).or_default() += 1;
            match &payload.ledger_purpose {
                events::SideEffectLedgerPurpose::Forward => {
                    *forward_by_node.entry(payload.node_id.clone()).or_default() += 1;
                }
                events::SideEffectLedgerPurpose::Remediation { forward_pair_id } => {
                    *remediation_by_forward
                        .entry(forward_pair_id.clone())
                        .or_default() += 1;
                }
            }
        }
        std::ops::ControlFlow::Continue(())
    });
    for (ledger, count) in by_ledger {
        assert_eq!(count, 1, "duplicate submission for ledger {ledger}");
    }
    for (node, count) in forward_by_node {
        assert_eq!(count, 1, "duplicate forward submission for node {node}");
    }
    for (forward_pair, count) in remediation_by_forward {
        assert_eq!(
            count, 1,
            "duplicate remediation submission for forward pair {forward_pair}"
        );
    }
}

pub(super) fn side_effect_submission_count_for_ledger(
    store: &TestTypedRunStore,
    fixture: &Fixture,
    ledger_key: &events::SideEffectLedgerKey,
) -> usize {
    let current = verified_current_for_store(store, fixture);
    let mut count = 0;
    let _ = current.lifecycle().visit_records::<()>(|record| {
        if matches!(
            record.kind(),
            store::current_lifecycle::CurrentRecordKindRef::SideEffectSubmissionObserved(payload)
                if &payload.ledger_key == ledger_key
        ) {
            count += 1;
        }
        std::ops::ControlFlow::Continue(())
    });
    count
}

pub(super) fn remediation_submission_count_for_forward_pair(
    store: &TestTypedRunStore,
    fixture: &Fixture,
    forward_pair_id: &SideEffectPairId,
) -> usize {
    let current = verified_current_for_store(store, fixture);
    let mut count = 0;
    let _ = current.lifecycle().visit_records::<()>(|record| {
        if matches!(
            record.kind(),
            store::current_lifecycle::CurrentRecordKindRef::SideEffectSubmissionObserved(payload)
                if matches!(
                    &payload.ledger_purpose,
                    events::SideEffectLedgerPurpose::Remediation {
                        forward_pair_id: linked,
                    } if linked == forward_pair_id
                )
        ) {
            count += 1;
        }
        std::ops::ControlFlow::Continue(())
    });
    count
}

pub(super) struct SideEffectFixtureIntentOutput {
    pub(super) ledger: events::SideEffectLedgerKey,
    pub(super) staged_artifact: StagedArtifact,
    pub(super) payload: RunnerEventPayload,
}

pub(super) fn side_effect_fixture_intent_output(
    ctx: &ErasedRunCtx<'_>,
    ledger_purpose: events::SideEffectLedgerPurpose,
    invocation_epoch: u32,
) -> Result<SideEffectFixtureIntentOutput> {
    let ledger = side_effect_ledger_key_for_ctx(ctx);
    let (pair_id, pair_role) =
        side_effect_pair_fields_for_ctx(ctx, &ledger_purpose, events::SideEffectPairRole::Submit);
    let intent =
        fixture_side_effect_evidence(21, ctx.node().node_id.as_str(), ctx.attempt_id().as_str());
    let idempotency =
        fixture_side_effect_evidence(34, ctx.node().node_id.as_str(), ctx.attempt_id().as_str());
    let artifact_builder = RunnerArtifactBuilder::new(ctx);
    let intent_artifact = artifact_builder.side_effect_intent(&intent)?;
    let intent_evidence = intent_artifact.evidence();
    let intent_hash = intent_evidence.digest.clone();
    let intent_artifact_id = intent_evidence.artifact_id.clone();
    let intent_artifact_evidence_hash = intent_evidence.evidence_hash().map_err(|error| {
        RuntimeError::InvalidRunnerOutput(format!("intent evidence hash: {error}"))
    })?;
    let staged_artifact =
        artifact_builder.staged_side_effect(&intent_artifact, ledger.clone(), invocation_epoch)?;
    let adapter_binding = ctx
        .node()
        .adapter_bindings
        .first()
        .expect("side-effect adapter");
    let payload =
        RunnerEventPayload::SideEffectIntentPersisted(events::side_effect::IntentPersisted {
            spec_hash: ctx.spec_hash().clone(),
            node_id: ctx.node().node_id.clone(),
            scope_id: ctx.node().scope_id.clone(),
            attempt_id: ctx.attempt_id().clone(),
            ledger_key: ledger.clone(),
            ledger_purpose,
            pair_id,
            pair_role,
            invocation_epoch,
            intent_schema_id: <FixtureSideEffectEvidence as mfm_values::MfmValue>::schema_id()
                .expect("side-effect intent schema"),
            intent_hash,
            intent_artifact_id,
            intent_artifact_evidence_hash,
            idempotency_input_schema_id:
                <FixtureSideEffectEvidence as mfm_values::MfmValue>::schema_id()
                    .expect("side-effect idempotency schema"),
            idempotency_input_hash: content_digest_json(
                serde_json::to_value(&idempotency).expect("side-effect idempotency value"),
            )
            .expect("side-effect idempotency digest"),
            idempotency_key: crate::side_effect_driver::side_effect_idempotency_key(&idempotency)
                .expect("side-effect idempotency key"),
            capability_kind: side_effect_capability_kind(),
            capability_version: side_effect_capability_version(),
            adapter_kind: adapter_binding.adapter_kind.clone(),
            adapter_version: adapter_binding.adapter_version.clone(),
        });
    Ok(SideEffectFixtureIntentOutput {
        ledger,
        staged_artifact,
        payload,
    })
}

pub(super) fn side_effect_ledger_key_for_ctx(
    ctx: &ErasedRunCtx<'_>,
) -> events::SideEffectLedgerKey {
    if let Some(forward_pair_id) = linked_forward_pair_for_remediation(ctx) {
        events::SideEffectLedgerKey::new(format!(
            "remediation-{}-{}",
            forward_pair_id,
            ctx.attempt_no()
        ))
        .expect("remediation ledger key")
    } else {
        events::SideEffectLedgerKey::new(format!(
            "forward-{}-{}-{}",
            ctx.run_id(),
            ctx.node().node_id,
            ctx.attempt_no()
        ))
        .expect("forward ledger key")
    }
}

pub(super) fn side_effect_ledger_purpose_for_ctx(
    ctx: &ErasedRunCtx<'_>,
) -> events::SideEffectLedgerPurpose {
    linked_forward_pair_for_remediation(ctx)
        .map(|forward_pair_id| events::SideEffectLedgerPurpose::Remediation { forward_pair_id })
        .unwrap_or(events::SideEffectLedgerPurpose::Forward)
}

pub(super) fn forward_pair_for_ledger(
    current: &VerifiedCurrentRun,
    forward_ledger_key: &events::SideEffectLedgerKey,
) -> SideEffectPairId {
    let mut pair_id = None;
    let _ = current.lifecycle().visit_side_effects::<()>(|side_effect| {
        if side_effect.ledger_key() == forward_ledger_key {
            assert!(
                pair_id.replace(side_effect.pair_id().clone()).is_none(),
                "ledger {forward_ledger_key} has multiple side-effect pairs"
            );
        }
        std::ops::ControlFlow::Continue(())
    });
    pair_id.expect("forward side-effect pair")
}

pub(super) fn linked_forward_pair_for_remediation(
    ctx: &ErasedRunCtx<'_>,
) -> Option<SideEffectPairId> {
    if let Some(side_effect) = side_effect_for_attempt(
        &ctx.runtime_spec(),
        ctx.lifecycle(),
        ctx.node(),
        ctx.attempt_id(),
    )
    .expect("remediation side-effect lookup")
    {
        if let events::SideEffectLedgerPurpose::Remediation { forward_pair_id } =
            side_effect.ledger_purpose()
        {
            return Some(forward_pair_id.clone());
        }
    }
    let forward_node_id = ctx
        .runtime_spec()
        .forward_node_for_remediation(&ctx.node().node_id)?;
    let mut pair_id = None;
    let _ = ctx.lifecycle().visit_side_effects::<()>(|side_effect| {
        if side_effect.intent().node_id() == forward_node_id
            && matches!(
                side_effect.ledger_purpose(),
                events::SideEffectLedgerPurpose::Forward
            )
            && matches!(
                side_effect.phase(),
                store::SideEffectPhase::ConfirmationObserved { .. }
            )
        {
            pair_id = Some(side_effect.pair_id().clone());
            return std::ops::ControlFlow::Break(());
        }
        std::ops::ControlFlow::Continue(())
    });
    pair_id
}

pub(super) fn side_effect_claim_owner(
    attempt_no: u32,
    generation: u32,
) -> events::RunnerInvocationId {
    events::RunnerInvocationId::new(format!("owner-{attempt_no}-{generation}"))
        .expect("claim owner")
}

pub(super) fn side_effect_fencing_token(
    attempt_no: u32,
    generation: u32,
) -> events::side_effect::ClaimFencingToken {
    events::side_effect::ClaimFencingToken::new(format!("token-{attempt_no}-{generation}"))
        .expect("fencing token")
}

pub(super) fn runner_side_effect_binding_for_ctx(
    ctx: &ErasedRunCtx<'_>,
    ledger: events::SideEffectLedgerKey,
    invocation_epoch: u32,
) -> RunnerSideEffectBinding {
    let ledger_purpose = side_effect_ledger_purpose_for_ctx(ctx);
    let (pair_id, _) =
        side_effect_pair_fields_for_ctx(ctx, &ledger_purpose, events::SideEffectPairRole::Submit);
    RunnerSideEffectBinding {
        ledger_key: ledger,
        ledger_purpose,
        pair_id,
        invocation_epoch,
    }
}

pub(super) fn runner_claim_binding_for_ctx(
    ctx: &ErasedRunCtx<'_>,
    claim_generation: u32,
) -> RunnerClaimBinding {
    RunnerClaimBinding {
        claim_owner: side_effect_claim_owner(ctx.attempt_no(), claim_generation),
        claim_generation,
        claim_fencing_token: side_effect_fencing_token(ctx.attempt_no(), claim_generation),
    }
}

pub(super) fn side_effect_claimed(
    ctx: &ErasedRunCtx<'_>,
    ledger: events::SideEffectLedgerKey,
    invocation_epoch: u32,
    claim_generation: u32,
) -> RunnerEventPayload {
    RunnerPayloadBuilder::new(ctx).side_effect_claimed(
        runner_side_effect_binding_for_ctx(ctx, ledger, invocation_epoch),
        runner_claim_binding_for_ctx(ctx, claim_generation),
    )
}

pub(super) struct SideEffectFixturePreparedOutput {
    pub(super) staged_artifact: StagedArtifact,
    pub(super) payload: RunnerEventPayload,
}

pub(super) fn side_effect_prepared_output(
    ctx: &ErasedRunCtx<'_>,
    ledger: events::SideEffectLedgerKey,
    invocation_epoch: u32,
    claim_generation: u32,
) -> Result<SideEffectFixturePreparedOutput> {
    let prepared = fixture_side_effect_evidence_for_ctx(ctx, 35);
    let artifact_builder = RunnerArtifactBuilder::new(ctx);
    let artifact = artifact_builder.prepared_invocation(&prepared)?;
    let evidence = artifact.evidence().clone();
    let staged_artifact =
        artifact_builder.staged_side_effect(&artifact, ledger.clone(), invocation_epoch)?;
    let claim = runner_claim_binding_for_ctx(ctx, claim_generation);
    let binding = runner_side_effect_binding_for_ctx(ctx, ledger, invocation_epoch);
    let payload =
        RunnerEventPayload::SideEffectInvocationPrepared(events::side_effect::InvocationPrepared {
            spec_hash: ctx.spec_hash().clone(),
            node_id: ctx.node().node_id.clone(),
            attempt_id: ctx.attempt_id().clone(),
            ledger_key: binding.ledger_key,
            ledger_purpose: binding.ledger_purpose,
            pair_id: binding.pair_id,
            pair_role: events::SideEffectPairRole::Submit,
            invocation_epoch: binding.invocation_epoch,
            claim_generation: claim.claim_generation,
            claim_fencing_token: claim.claim_fencing_token,
            resource_key: None,
            prepared_schema_id: evidence.schema_id.clone().expect("prepared schema"),
            prepared_artifact_id: evidence.artifact_id.clone(),
            prepared_hash: evidence.digest.clone(),
            prepared_artifact_evidence_hash: evidence.evidence_hash().map_err(|error| {
                RuntimeError::InvalidRunnerOutput(format!("prepared evidence hash: {error}"))
            })?,
        });
    Ok(SideEffectFixturePreparedOutput {
        staged_artifact,
        payload,
    })
}

pub(super) fn side_effect_failed(
    ctx: &ErasedRunCtx<'_>,
    ledger: events::SideEffectLedgerKey,
    invocation_epoch: u32,
    failure_phase: events::side_effect::FailurePhase,
    retryable: bool,
) -> RunnerEventPayload {
    RunnerPayloadBuilder::new(ctx).side_effect_failed(
        runner_side_effect_binding_for_ctx(ctx, ledger, invocation_epoch),
        events::SideEffectPairRole::Submit,
        failure_phase,
        retryable,
        side_effect_error(retryable),
    )
}

pub(super) fn side_effect_error(retryable: bool) -> events::MfmErrorInfo {
    events::MfmErrorInfo {
        code: events::ErrorCode::new("sidefx_failed").expect("error code"),
        category: events::ErrorCategory::SideEffect,
        retryable,
        safe_message: "side-effect failed".to_owned(),
        public_details: None,
        diagnostic_ref: None,
    }
}
