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
    projections: &store::ProjectionSnapshot,
    node_id: &NodeId,
) -> events::SideEffectLedgerKey {
    let mut found = None;
    for (_, projection) in projections.side_effects() {
        if projection.intent.node_id == *node_id
            && matches!(
                projection.ledger_purpose,
                events::SideEffectLedgerPurpose::Forward
            )
        {
            assert!(
                found.replace(projection.ledger_key.clone()).is_none(),
                "node {node_id} has multiple forward ledgers"
            );
        }
    }
    found.expect("forward ledger for node")
}

pub(super) fn side_effect_projection_for_run_node<
    P: std::borrow::Borrow<store::ProjectionSnapshot>,
>(
    projections: P,
    run_id: &RunId,
    node_id: &NodeId,
) -> Option<store::SideEffectProjection> {
    let projections = projections.borrow();
    projections.side_effects().find_map(|(_, projection)| {
        (projection.run_id == *run_id && projection.intent.node_id == *node_id)
            .then(|| projection.clone())
    })
}

pub(super) async fn drive_until_side_effect_confirmation_without_output(
    scheduler: &SerialTypedScheduler,
    store: &mut TestTypedRunStore,
    fixture: &Fixture,
    node: &spec::NodeSpec,
    output_cell: &CellId,
    context: &str,
) {
    for _ in 0..8 {
        assert_eq!(
            drive_once(scheduler, store, &fixture.runtime_spec, &fixture.run_id)
                .await
                .expect(context),
            SchedulerStatus::Advanced
        );
        let projection_snapshot = store.projection_snapshot();
        if side_effect_projection_for_run_node(&projection_snapshot, &fixture.run_id, &node.node_id)
            .is_some_and(|projection| {
                matches!(
                    projection.phase,
                    store::SideEffectPhase::ConfirmationObserved { .. }
                ) && projection_snapshot.cell_terminal(output_cell).is_none()
            })
        {
            return;
        }
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
    for _ in 0..24 {
        if cells
            .iter()
            .all(|cell| store.projection_snapshot().cell_terminal(cell).is_some())
        {
            return;
        }
        assert_eq!(
            drive_once(scheduler, store, &fixture.runtime_spec, &fixture.run_id)
                .await
                .expect(context),
            SchedulerStatus::Advanced
        );
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
    for _ in 0..24 {
        let projection_snapshot = store.projection_snapshot();
        if side_effect_projection_for_attempt(
            &fixture.runtime_spec,
            &fixture.run_id,
            &projection_snapshot,
            node,
            attempt_id,
        )
        .expect(context)
        .is_some_and(|projection| phase_matches(&projection.phase))
        {
            return;
        }
        assert_eq!(
            drive_once(scheduler, store, &fixture.runtime_spec, &fixture.run_id)
                .await
                .expect(context),
            SchedulerStatus::Advanced
        );
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
    for _ in 0..8 {
        assert_eq!(
            drive_once(scheduler, store, &fixture.runtime_spec, &fixture.run_id)
                .await
                .expect(context),
            SchedulerStatus::Advanced
        );
        let projection_snapshot = store.projection_snapshot();
        if remediation_projection_for_forward_pair(&projection_snapshot, forward_pair_id)
            .is_some_and(|projection| match checkpoint {
                RemediationPhaseCheckpoint::SubmissionObserved => {
                    matches!(
                        projection.phase,
                        store::SideEffectPhase::SubmissionObserved { .. }
                    )
                }
                RemediationPhaseCheckpoint::ConfirmationObserved => {
                    matches!(
                        projection.phase,
                        store::SideEffectPhase::ConfirmationObserved { .. }
                    )
                }
            })
        {
            return;
        }
    }
    panic!("{context} was not reached");
}

pub(super) async fn drive_until_compensated_before_terminal(
    scheduler: &SerialTypedScheduler,
    store: &mut TestTypedRunStore,
    fixture: &Fixture,
) {
    for _ in 0..24 {
        let saga = derive_fixture_saga(fixture, store.projection_snapshot());
        if saga.run_mode == store::RunMode::Compensated
            && store.projection_snapshot().run_state(&fixture.run_id) != store::RunState::Completed
        {
            return;
        }
        assert_eq!(
            drive_once(scheduler, store, &fixture.runtime_spec, &fixture.run_id)
                .await
                .expect("drive until compensated before terminal"),
            SchedulerStatus::Advanced
        );
    }
    panic!("compensated pre-terminal boundary was not reached");
}

pub(super) fn remediation_intent_forward_links(
    store: &TestTypedRunStore,
    run_id: &RunId,
) -> Vec<SideEffectPairId> {
    store
        .load_run_stream(run_id)
        .iter()
        .filter_map(|event| match event.payload() {
            events::KernelEventPayload::SideEffectIntentPersisted(payload) => {
                match &payload.ledger_purpose {
                    events::SideEffectLedgerPurpose::Remediation { forward_pair_id } => {
                        Some(forward_pair_id.clone())
                    }
                    events::SideEffectLedgerPurpose::Forward => None,
                }
            }
            _ => None,
        })
        .collect()
}

pub(super) fn remediation_projection_for_forward_pair<
    P: std::borrow::Borrow<store::ProjectionSnapshot>,
>(
    projections: P,
    forward_pair_id: &SideEffectPairId,
) -> Option<store::SideEffectProjection> {
    let projections = projections.borrow();
    projections.side_effects().find_map(|(_, projection)| {
        matches!(
            &projection.ledger_purpose,
            events::SideEffectLedgerPurpose::Remediation {
                forward_pair_id: linked,
            } if linked == forward_pair_id
        )
        .then(|| projection.clone())
    })
}

pub(super) fn assert_no_duplicate_side_effect_submissions(
    store: &TestTypedRunStore,
    run_id: &RunId,
) {
    let mut by_ledger = BTreeMap::<events::SideEffectLedgerKey, usize>::new();
    let mut forward_by_node = BTreeMap::<NodeId, usize>::new();
    let mut remediation_by_forward = BTreeMap::<SideEffectPairId, usize>::new();
    for event in store.load_run_stream(run_id) {
        if let events::KernelEventPayload::SideEffectSubmissionObserved(payload) = event.payload() {
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
    }
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
    run_id: &RunId,
    ledger_key: &events::SideEffectLedgerKey,
) -> usize {
    store
        .load_run_stream(run_id)
        .iter()
        .filter(|event| {
            matches!(
                event.payload(),
                events::KernelEventPayload::SideEffectSubmissionObserved(payload)
                    if &payload.ledger_key == ledger_key
            )
        })
        .count()
}

pub(super) fn remediation_submission_count_for_forward_pair(
    store: &TestTypedRunStore,
    run_id: &RunId,
    forward_pair_id: &SideEffectPairId,
) -> usize {
    store
        .load_run_stream(run_id)
        .iter()
        .filter(|event| {
            matches!(
                event.payload(),
                events::KernelEventPayload::SideEffectSubmissionObserved(payload)
                    if matches!(
                        &payload.ledger_purpose,
                        events::SideEffectLedgerPurpose::Remediation {
                            forward_pair_id: linked,
                        } if linked == forward_pair_id
                    )
            )
        })
        .count()
}

pub(super) fn side_effect_fixture_digest(ctx: &ErasedRunCtx<'_>, role: &str) -> ContentDigest {
    content_digest_json(serde_json::json!({
        "attempt": ctx.attempt_id().as_str(),
        "node": ctx.node().node_id.as_str(),
        "role": role,
    }))
    .expect("side-effect fixture digest")
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
    let (intent_artifact_id, intent_hash) = side_effect_fixture_artifact_pair(ctx, "intent");
    let intent_evidence = side_effect_artifact(
        ctx,
        intent_artifact_id.clone(),
        intent_hash.clone(),
        events::ArtifactRole::SideEffectIntent,
    );
    let intent_artifact_evidence_hash = intent_evidence.evidence_hash().map_err(|error| {
        RuntimeError::InvalidRunnerOutput(format!("intent evidence hash: {error}"))
    })?;
    let staged_artifact =
        staged_side_effect_artifact(ctx, intent_evidence, ledger.clone(), invocation_epoch)?;
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
            intent_schema_id: ctx.node().config_ref.schema_id.clone(),
            intent_hash,
            intent_artifact_id,
            intent_artifact_evidence_hash,
            idempotency_input_schema_id: ctx.node().config_ref.schema_id.clone(),
            idempotency_input_hash: side_effect_fixture_digest(ctx, "idempotency"),
            idempotency_key: events::IdempotencyKeyRef::new("idem-1").expect("idempotency key"),
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

pub(super) fn side_effect_fixture_artifact_pair(
    ctx: &ErasedRunCtx<'_>,
    role: &str,
) -> (ArtifactId, ContentDigest) {
    let digest = side_effect_fixture_digest(ctx, role);
    (
        ArtifactId::from_digest(digest.algorithm(), *digest.digest()),
        digest,
    )
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
    projections: &store::ProjectionSnapshot,
    forward_ledger_key: &events::SideEffectLedgerKey,
) -> SideEffectPairId {
    projections
        .side_effects()
        .find_map(|(_, projection)| {
            (projection.ledger_key == *forward_ledger_key).then(|| projection.pair_id.clone())
        })
        .expect("forward pair projection")
}

pub(super) fn linked_forward_pair_for_remediation(
    ctx: &ErasedRunCtx<'_>,
) -> Option<SideEffectPairId> {
    if let Some(projection) = side_effect_projection_for_attempt(
        ctx.runtime_spec(),
        ctx.run_id(),
        ctx.projections(),
        ctx.node(),
        ctx.attempt_id(),
    )
    .expect("remediation projection lookup")
    {
        if let events::SideEffectLedgerPurpose::Remediation { forward_pair_id } =
            &projection.ledger_purpose
        {
            return Some(forward_pair_id.clone());
        }
    }
    let forward_node_id = ctx
        .runtime_spec()
        .forward_node_for_remediation(&ctx.node().node_id)?;
    ctx.projections()
        .side_effects()
        .find_map(|(_, projection)| {
            (projection.intent.node_id == *forward_node_id
                && matches!(
                    &projection.ledger_purpose,
                    events::SideEffectLedgerPurpose::Forward
                )
                && matches!(
                    projection.phase,
                    store::SideEffectPhase::ConfirmationObserved { .. }
                ))
            .then(|| projection.pair_id.clone())
        })
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

pub(super) fn side_effect_artifact(
    ctx: &ErasedRunCtx<'_>,
    artifact_id: ArtifactId,
    digest: ContentDigest,
    role: events::ArtifactRole,
) -> store::ArtifactEvidenceRef {
    store::ArtifactEvidenceRef {
        artifact_id,
        digest,
        byte_len: 19,
        media_type: spec::MediaType::new("application/json").expect("media"),
        schema_id: Some(ctx.node().config_ref.schema_id.clone()),
        semantic_type_id: None,
        producer_node_id: Some(ctx.node().node_id.clone()),
        producer_seed_id: None,
        artifact_role: role,
    }
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

pub(super) fn side_effect_prepared(
    ctx: &ErasedRunCtx<'_>,
    ledger: events::SideEffectLedgerKey,
    invocation_epoch: u32,
    claim_generation: u32,
) -> RunnerEventPayload {
    let claim = runner_claim_binding_for_ctx(ctx, claim_generation);
    RunnerPayloadBuilder::new(ctx)
        .side_effect_invocation_prepared(
            runner_side_effect_binding_for_ctx(ctx, ledger, invocation_epoch),
            None,
            RunnerPreparedInvocationBinding {
                claim_generation: claim.claim_generation,
                claim_fencing_token: claim.claim_fencing_token,
                resource_key: None,
            },
        )
        .expect("side-effect prepared payload")
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
