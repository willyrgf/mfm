use super::*;

pub(super) async fn prepare_fixture_launch(
    scheduler: &SerialTypedScheduler,
    store: &TestTypedRunStore,
    fixture: &Fixture,
    seed_cells: Vec<events::SeedCellRef>,
) -> Result<PreparedRunLaunch> {
    scheduler
        .prepare_run_launch(
            &fixture.runtime_spec,
            fixture_run_identity_material(fixture),
            run_start_evidence(fixture, seed_cells),
            store.expected_next_seq(&fixture.run_id),
        )
        .await
}

pub(super) async fn start_fixture_run(
    scheduler: &SerialTypedScheduler,
    store: &mut TestTypedRunStore,
    fixture: &Fixture,
    seed_cells: Vec<events::SeedCellRef>,
) -> Result<store::CommitOutcome> {
    let launch = prepare_fixture_launch(scheduler, store, fixture, seed_cells).await?;
    scheduler_start_run(scheduler, store, launch).await
}

pub(super) async fn started_fixture_store(
    scheduler: &SerialTypedScheduler,
    fixture: &Fixture,
) -> TestTypedRunStore {
    let mut store = TestTypedRunStore::new();
    start_fixture_run(
        scheduler,
        &mut store,
        fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");
    store
}

pub(super) async fn started_fixture_run(
    fixture: &Fixture,
) -> (SerialTypedScheduler, TestTypedRunStore) {
    let scheduler = test_scheduler(registered_fixture_runners(fixture));
    let store = started_fixture_store(&scheduler, fixture).await;
    (scheduler, store)
}

pub(super) async fn started_fixture_run_with_registry(
    registry: ErasedRunnerRegistry,
    fixture: &Fixture,
) -> (SerialTypedScheduler, TestTypedRunStore) {
    let scheduler = fixture_scheduler(registry, fixture);
    let store = started_fixture_store(&scheduler, fixture).await;
    (scheduler, store)
}

pub(super) async fn started_side_effect_fixture_run(
    fixture: &Fixture,
) -> (SerialTypedScheduler, TestTypedRunStore) {
    let scheduler = test_scheduler(registered_side_effect_fixture_runners(fixture));
    let store = started_fixture_store(&scheduler, fixture).await;
    (scheduler, store)
}

pub(super) async fn start_fixture_run_async_store<S: store::RunEventStore + ?Sized>(
    scheduler: &SerialTypedScheduler,
    store: &S,
    fixture: &Fixture,
    seed_cells: Vec<events::SeedCellRef>,
) -> Result<store::CommitOutcome> {
    let expected_next_seq = store
        .expected_next_seq(&fixture.run_id)
        .await
        .map_err(crate::error::async_store_error)?;
    let launch = scheduler
        .prepare_run_launch(
            &fixture.runtime_spec,
            fixture_run_identity_material(fixture),
            run_start_evidence(fixture, seed_cells),
            expected_next_seq,
        )
        .await?;
    scheduler.start_run(store, launch).await
}

pub(super) async fn scheduler_start_run(
    scheduler: &SerialTypedScheduler,
    store: &mut TestTypedRunStore,
    launch: PreparedRunLaunch,
) -> Result<store::CommitOutcome> {
    scheduler.start_run(&*store, launch).await
}

pub(super) async fn scheduler_start_run_admitted(
    scheduler: &SerialTypedScheduler,
    store: &mut TestTypedRunStore,
    runtime_spec: &CertifiedRuntimeSpec,
    launch: PreparedRunLaunch,
) -> Result<RunAdmissionAuthority> {
    scheduler
        .start_run_admitted(&*store, runtime_spec, launch)
        .await
}

pub(super) async fn drive_once(
    scheduler: &SerialTypedScheduler,
    store: &mut TestTypedRunStore,
    runtime_spec: &CertifiedRuntimeSpec,
    run_id: &RunId,
) -> Result<SchedulerStatus> {
    drive_once_with_claim(scheduler, &*store, runtime_spec, run_id).await
}

pub(super) async fn drive_fixture_once(
    scheduler: &SerialTypedScheduler,
    store: &mut TestTypedRunStore,
    fixture: &Fixture,
) -> Result<SchedulerStatus> {
    drive_once(scheduler, store, &fixture.runtime_spec, &fixture.run_id).await
}

pub(super) async fn assert_invalid_output_after(
    scheduler: &SerialTypedScheduler,
    store: &mut TestTypedRunStore,
    fixture: &Fixture,
    drive_count: usize,
    expected_message: &str,
) {
    for _ in 0..drive_count {
        assert_eq!(
            drive_fixture_once(scheduler, store, fixture)
                .await
                .expect("advance before invalid output"),
            SchedulerStatus::Advanced
        );
    }
    assert!(matches!(
        drive_fixture_once(scheduler, store, fixture).await,
        Err(RuntimeError::InvalidRunnerOutput(message))
            if message.contains(expected_message)
    ));
}

pub(super) async fn drive_until_blocked(
    scheduler: &SerialTypedScheduler,
    store: &mut TestTypedRunStore,
    runtime_spec: &CertifiedRuntimeSpec,
    run_id: &RunId,
) -> Result<SchedulerStatus> {
    drive_until_blocked_with_claim(scheduler, &*store, runtime_spec, run_id).await
}

pub(super) async fn drive_fixture_until_blocked(
    scheduler: &SerialTypedScheduler,
    store: &mut TestTypedRunStore,
    fixture: &Fixture,
) -> Result<SchedulerStatus> {
    drive_until_blocked(scheduler, store, &fixture.runtime_spec, &fixture.run_id).await
}

pub(super) async fn drive_once_with_claim<S>(
    scheduler: &SerialTypedScheduler,
    store: &S,
    runtime_spec: &CertifiedRuntimeSpec,
    run_id: &RunId,
) -> Result<SchedulerStatus>
where
    S: store::RunEventStore + store::ExecutionClaimStore + ?Sized,
{
    let execution_scope = execution_claim_scope(store, run_id).await?;
    let token = execution_claim_token(store, &execution_scope, run_id).await?;
    scheduler
        .drive_once(store, runtime_spec, run_id, &execution_scope, token)
        .await
}

pub(super) async fn drive_until_blocked_with_claim<S>(
    scheduler: &SerialTypedScheduler,
    store: &S,
    runtime_spec: &CertifiedRuntimeSpec,
    run_id: &RunId,
) -> Result<SchedulerStatus>
where
    S: store::RunEventStore + store::ExecutionClaimStore + ?Sized,
{
    let execution_scope = execution_claim_scope(store, run_id).await?;
    let token = execution_claim_token(store, &execution_scope, run_id).await?;
    scheduler
        .drive_until_blocked(store, runtime_spec, run_id, &execution_scope, token)
        .await
}

pub(super) async fn execution_claim_scope<S>(
    store: &S,
    run_id: &RunId,
) -> Result<store::ExecutionClaimScope>
where
    S: store::RunEventStore + ?Sized,
{
    let committed = store
        .load_committed_run_stream(run_id)
        .await
        .map_err(crate::error::async_store_error)?;
    committed
        .events()
        .iter()
        .find_map(|event| match event.payload() {
            events::KernelEventPayload::RunAdmitted(payload) => Some(
                store::ExecutionClaimScope::from_run_identity_material(&payload.identity_material),
            ),
            _ => None,
        })
        .ok_or_else(|| {
            RuntimeError::InvalidRunStream(format!("run {run_id} has no RunAdmitted event"))
        })
}

pub(super) async fn execution_claim_token<S>(
    store: &S,
    execution_scope: &store::ExecutionClaimScope,
    run_id: &RunId,
) -> Result<store::AdmissionToken>
where
    S: store::ExecutionClaimStore + ?Sized,
{
    loop {
        match store
            .execution_claim_status(execution_scope)
            .await
            .map_err(crate::error::async_store_error)?
        {
            store::ExecutionClaimStatus::Live(lease) => return Ok(lease.token),
            store::ExecutionClaimStatus::Expired(lease) => {
                store
                    .reap_expired_execution_claim(
                        execution_scope,
                        &lease.holder_run_id,
                        &lease.token,
                    )
                    .await
                    .map_err(crate::error::async_store_error)?;
            }
            store::ExecutionClaimStatus::Unclaimed => {
                let token = store::AdmissionToken::new(format!(
                    "mfm.test.runtime.execution_claim:{run_id}"
                ))?;
                match store
                    .acquire_execution_claim(execution_scope, run_id, token)
                    .await
                    .map_err(crate::error::async_store_error)?
                {
                    store::NowaitSkipAdmissionResult::Admitted(lease) => return Ok(lease.token),
                    store::NowaitSkipAdmissionResult::Busy(_) => {}
                }
            }
        }
    }
}

pub(super) async fn record_manual_resolution(
    scheduler: &SerialTypedScheduler,
    store: &mut TestTypedRunStore,
    runtime_spec: &CertifiedRuntimeSpec,
    run_id: &RunId,
    request: ManualResolutionRequest,
) -> Result<store::CommitOutcome> {
    scheduler
        .record_manual_resolution(&*store, runtime_spec, run_id, request)
        .await
}

pub(super) fn build_manual_resolution_prefix_authority_for_tests(
    runtime_spec: &CertifiedRuntimeSpec,
    run_id: &RunId,
    store: &TestTypedRunStore,
    manual: spec::ManualResolutionEvidenceSpec,
) -> Result<mfm_manual_auth::ManualResolutionPrefixAuthority> {
    let projection_snapshot = store.projection_snapshot();
    crate::manual_resolution::build_manual_resolution_prefix_authority_from_parts(
        runtime_spec,
        run_id,
        manual,
        &store.load_run_stream(run_id),
        store.expected_next_seq(run_id),
        &projection_snapshot,
    )
}

pub(super) fn run_start_evidence(
    fixture: &Fixture,
    seed_cells: Vec<events::SeedCellRef>,
) -> RunLaunchEvidence {
    RunLaunchEvidence {
        entry_point: entry_point_launch_evidence(),
        spec_artifact: spec_artifact(&fixture.runtime_spec),
        certificate_artifact: certificate_artifact(&fixture.runtime_spec),
        config_artifacts: fixture
            .runtime_spec
            .spec()
            .config_refs
            .iter()
            .map(|config| config_artifact(&fixture.runtime_spec, config))
            .collect(),
        fact_descriptor_artifacts: Vec::new(),
        seed_cells: seed_cells.into_iter().map(seed_launch_cell).collect(),
    }
}

pub(super) fn entry_point_launch_evidence() -> events::EntryPointLaunchEvidence {
    events::EntryPointLaunchEvidence::new("mfm.test/portfolio_snapshot@1", Vec::new())
        .expect("entry-point evidence")
}

pub(super) fn spec_artifact(runtime_spec: &CertifiedRuntimeSpec) -> RunLaunchArtifact {
    let canonical = runtime_spec
        .spec()
        .canonical_json()
        .expect("canonical spec");
    let digest = canonical.content_digest();
    RunLaunchArtifact {
        bytes: canonical.to_vec(),
        evidence: store::ArtifactEvidenceRef {
            artifact_id: ArtifactId::from_digest(digest.algorithm(), *digest.digest()),
            digest,
            byte_len: canonical.as_bytes().len() as u64,
            media_type: runtime_spec.spec().media_type.clone(),
            schema_id: Some(spec::typed_execution_spec_schema_id().expect("typed spec schema")),
            semantic_type_id: None,
            producer_node_id: None,
            producer_seed_id: None,
            artifact_role: events::ArtifactRole::TypedExecutionSpec,
        },
    }
}

pub(super) fn certificate_artifact(runtime_spec: &CertifiedRuntimeSpec) -> RunLaunchArtifact {
    let canonical = runtime_spec
        .certificate()
        .canonical_json()
        .expect("canonical certificate");
    let digest = canonical.content_digest();
    RunLaunchArtifact {
        bytes: canonical.to_vec(),
        evidence: store::ArtifactEvidenceRef {
            artifact_id: ArtifactId::from_digest(digest.algorithm(), *digest.digest()),
            digest,
            byte_len: canonical.as_bytes().len() as u64,
            media_type: spec::MediaType::new(mfm_certify::CERTIFICATE_MEDIA_TYPE)
                .expect("certificate media type"),
            schema_id: Some(
                mfm_certify::typed_spec_certificate_schema_id().expect("typed certificate schema"),
            ),
            semantic_type_id: None,
            producer_node_id: None,
            producer_seed_id: None,
            artifact_role: events::ArtifactRole::TypedSpecCertificate,
        },
    }
}

pub(super) fn config_artifact(
    runtime_spec: &CertifiedRuntimeSpec,
    config: &spec::ConfigRef,
) -> RunLaunchArtifact {
    let bytes = runtime_spec
        .spec()
        .nodes
        .iter()
        .find(|node| node.config_ref == *config && node.framework.is_some())
        .and_then(|node| {
            node.framework.as_ref().map(|framework| {
                spec::framework_config_canonical_json(framework.config_kind(), &node.node_id)
                    .expect("framework config")
                    .to_vec()
            })
        })
        .unwrap_or_else(|| TEST_CONFIG_BYTES.to_vec());
    let bytes = if digest_for_bytes(&bytes) == config.digest {
        bytes
    } else {
        [
            CONFIG_MULTIPLIER_1_BYTES,
            CONFIG_MULTIPLIER_3_BYTES,
            CONFIG_MULTIPLIER_5_BYTES,
            CONFIG_MULTIPLIER_7_BYTES,
        ]
        .into_iter()
        .find(|candidate| digest_for_bytes(candidate) == config.digest)
        .expect("known test config bytes")
        .to_vec()
    };
    assert_eq!(digest_for_bytes(&bytes), config.digest);
    RunLaunchArtifact {
        bytes,
        evidence: store::ArtifactEvidenceRef {
            artifact_id: config.artifact_id.clone(),
            digest: config.digest.clone(),
            byte_len: config.byte_len,
            media_type: config.media_type.clone(),
            schema_id: Some(config.schema_id.clone()),
            semantic_type_id: None,
            producer_node_id: None,
            producer_seed_id: None,
            artifact_role: events::ArtifactRole::TypedConfig,
        },
    }
}

pub(super) fn fact_descriptor_artifact(
    descriptor: &mfm_facts::FactDescriptor,
) -> RunLaunchArtifact {
    let canonical =
        mfm_facts::canonical_fact_descriptor_bytes(descriptor).expect("canonical descriptor");
    let digest = mfm_facts::fact_descriptor_hash(descriptor).expect("descriptor hash");
    RunLaunchArtifact {
        bytes: canonical.to_vec(),
        evidence: store::ArtifactEvidenceRef {
            artifact_id: ArtifactId::from_digest(digest.algorithm(), *digest.digest()),
            digest,
            byte_len: canonical.as_bytes().len() as u64,
            media_type: spec::MediaType::new("application/json").expect("media"),
            schema_id: Some(mfm_facts::fact_descriptor_schema_id().expect("descriptor schema")),
            semantic_type_id: None,
            producer_node_id: None,
            producer_seed_id: None,
            artifact_role: events::ArtifactRole::FactDescriptor,
        },
    }
}

pub(super) fn seed_launch_cell(seed: events::SeedCellRef) -> RunLaunchSeedCell {
    let bytes = if seed.digest == digest_for_bytes(TEST_SEED_BYTES) {
        TEST_SEED_BYTES.to_vec()
    } else if seed.digest == digest_for_bytes(CERTIFIER_SEED_BYTES) {
        CERTIFIER_SEED_BYTES.to_vec()
    } else {
        TEST_SEED_BYTES.to_vec()
    };
    RunLaunchSeedCell { bytes, cell: seed }
}

pub(super) fn seed_cell_artifact_evidence(
    seed_id: &SeedId,
    schema_id: SchemaId,
    semantic_type_id: SemanticTypeId,
    content_digest: ContentDigest,
    byte_len: u64,
) -> events::ArtifactEvidenceRef {
    let store_evidence = store::ArtifactEvidenceRef {
        artifact_id: ArtifactId::from_digest(content_digest.algorithm(), *content_digest.digest()),
        digest: content_digest.clone(),
        byte_len,
        media_type: spec::MediaType::new("application/json").expect("media"),
        schema_id: Some(schema_id.clone()),
        semantic_type_id: Some(semantic_type_id),
        producer_node_id: None,
        producer_seed_id: Some(seed_id.clone()),
        artifact_role: events::ArtifactRole::SeedInput,
    };
    events::ArtifactEvidenceRef {
        artifact_id: store_evidence.artifact_id.clone(),
        role: store_evidence.artifact_role,
        schema_id,
        semantic_type_id: store_evidence.semantic_type_id.clone(),
        content_digest,
        evidence_hash: store_evidence
            .evidence_hash()
            .expect("seed artifact evidence hash"),
        byte_len,
        media_type: store_evidence.media_type,
    }
}

pub(super) fn terminal_payloads(
    ctx: &ErasedRunCtx<'_>,
    state_evidence: &store::ArtifactEvidenceRef,
) -> Vec<RunnerEventPayload> {
    vec![RunnerEventPayload::CellProduced(events::CellProduced {
        spec_hash: ctx.spec_hash().clone(),
        node_id: ctx.node().node_id.clone(),
        cell_id: ctx.node().output_cell.clone(),
        scope_id: ctx.node().scope_id.clone(),
        attempt_id: ctx.attempt_id().clone(),
        semantic_type_id: ctx.descriptor().output_semantic_type_id.clone(),
        schema_id: ctx.descriptor().output_schema_id.clone(),
        value_lineage: ctx.output_cell().value_lineage.clone(),
        context: ctx.output_cell().context.clone(),
        artifact_id: state_evidence.artifact_id.clone(),
        content_digest: state_evidence.digest.clone(),
        evidence_hash: state_evidence
            .evidence_hash()
            .expect("state output evidence hash"),
        producer_state_kind: Some(ctx.node().state_kind.clone()),
        producer_state_version: Some(ctx.node().state_version.clone()),
    })]
}

pub(super) fn fact_query_terminal_output(
    ctx: &ErasedRunCtx<'_>,
    state_evidence: &store::ArtifactEvidenceRef,
    staged_artifacts: Vec<StagedArtifact>,
    staged_retention_refs: Vec<StagedRetentionRefs>,
) -> ErasedRunnerOutput {
    ErasedRunnerOutput::from_parts(
        staged_artifacts,
        staged_retention_refs,
        terminal_payloads(ctx, state_evidence),
    )
}

pub(super) fn prepare_runner_output_for_invocation(
    invocation: &PreparedRunnerInvocation<'_>,
    output: ErasedRunnerOutput,
) -> Result<crate::commit::PreparedRunnerOutput> {
    CommitPlanner::prepare_runner_output(RunnerOutputCommitInput {
        runtime_spec: invocation.runtime_spec,
        run_id: invocation.run_id,
        node: invocation.node,
        attempt_id: invocation.attempt_id,
        caps: &invocation.caps,
        view: invocation.view,
        context_output_extractor: None,
        saga_terminal_proof: None,
        output,
    })
}

pub(super) fn state_output_artifact(
    node: &spec::NodeSpec,
    descriptor: &spec::StateDescriptorIdentity,
    artifact_id: ArtifactId,
    digest: ContentDigest,
) -> store::ArtifactEvidenceRef {
    store::ArtifactEvidenceRef {
        artifact_id,
        digest,
        byte_len: 17,
        media_type: spec::MediaType::new("application/json").expect("media"),
        schema_id: Some(descriptor.output_schema_id.clone()),
        semantic_type_id: Some(descriptor.output_semantic_type_id.clone()),
        producer_node_id: Some(node.node_id.clone()),
        producer_seed_id: None,
        artifact_role: events::ArtifactRole::StateOutput,
    }
}

pub(super) fn state_output_artifact_for_bytes(
    node: &spec::NodeSpec,
    descriptor: &spec::StateDescriptorIdentity,
    bytes: &[u8],
) -> store::ArtifactEvidenceRef {
    let digest = digest_for_bytes(bytes);
    let artifact_id = ArtifactId::from_digest(digest.algorithm(), *digest.digest());
    store::ArtifactEvidenceRef {
        artifact_id,
        digest,
        byte_len: bytes.len() as u64,
        media_type: spec::MediaType::new("application/json").expect("media"),
        schema_id: Some(descriptor.output_schema_id.clone()),
        semantic_type_id: Some(descriptor.output_semantic_type_id.clone()),
        producer_node_id: Some(node.node_id.clone()),
        producer_seed_id: None,
        artifact_role: events::ArtifactRole::StateOutput,
    }
}

pub(super) fn digest_for_bytes(bytes: &[u8]) -> ContentDigest {
    ContentDigest::from_digest(DigestAlgorithm::Sha256JcsV1, sha256_digest_bytes(bytes))
}

pub(super) fn runtime_staging_class(role: events::ArtifactRole) -> &'static str {
    match role.contract().staging {
        events::ArtifactStagingClass::AttemptStateOutput
        | events::ArtifactStagingClass::AttemptFactResponse
        | events::ArtifactStagingClass::AttemptExternalReadEvidence
        | events::ArtifactStagingClass::AttemptFactQueryEvidence
        | events::ArtifactStagingClass::AttemptPublicOutput
        | events::ArtifactStagingClass::AttemptRedactedDiagnostic => {
            assert!(staged_artifact_binding_kind(role).is_some());
            assert!(staged_side_effect_artifact_phase(role).is_none());
        }
        events::ArtifactStagingClass::SideEffectIntent
        | events::ArtifactStagingClass::SideEffectPreparedInvocation
        | events::ArtifactStagingClass::SideEffectNotSubmittedProof
        | events::ArtifactStagingClass::SideEffectSubmission
        | events::ArtifactStagingClass::SideEffectSubmissionUnknown
        | events::ArtifactStagingClass::SideEffectReceipt
        | events::ArtifactStagingClass::SideEffectConfirmation
        | events::ArtifactStagingClass::SideEffectAmbiguity => {
            assert!(staged_artifact_binding_kind(role).is_none());
            assert!(staged_side_effect_artifact_phase(role).is_some());
        }
        events::ArtifactStagingClass::RunAdmission
        | events::ArtifactStagingClass::ManualResolution
        | events::ArtifactStagingClass::MiddlewareRetentionManifest => {
            assert!(staged_artifact_binding_kind(role).is_none());
            assert!(staged_side_effect_artifact_phase(role).is_none());
        }
    }
    role.contract().staging.as_str()
}

#[test]
pub(super) fn artifact_role_contract_runtime_staging_matches_current_helpers() {
    let rows = events::ArtifactRole::ALL
        .iter()
        .copied()
        .map(|role| format!("{role:?} -> {}", runtime_staging_class(role)))
        .collect::<Vec<_>>()
        .join("\n");

    assert_eq!(
        rows,
        "TypedExecutionSpec -> run_admission\n\
TypedSpecCertificate -> run_admission\n\
TypedConfig -> run_admission\n\
FactDescriptor -> run_admission\n\
SeedInput -> run_admission\n\
StateOutput -> attempt_state_output\n\
FactResponse -> attempt_fact_response\n\
ExternalReadEvidence -> attempt_external_read_evidence\n\
FactQueryEvidence -> attempt_fact_query_evidence\n\
SideEffectIntent -> side_effect_intent\n\
PreparedInvocation -> side_effect_prepared_invocation\n\
NotSubmittedProof -> side_effect_not_submitted_proof\n\
Submission -> side_effect_submission\n\
SubmissionUnknownEvidence -> side_effect_submission_unknown\n\
Receipt -> side_effect_receipt\n\
Confirmation -> side_effect_confirmation\n\
AmbiguityEvidence -> side_effect_ambiguity\n\
ManualResolutionEvidence -> manual_resolution\n\
ManualResolutionAuthorization -> manual_resolution\n\
PublicOutput -> attempt_public_output\n\
RedactedDiagnostic -> attempt_redacted_diagnostic\n\
RetentionManifest -> middleware_retention_manifest"
    );
}

pub(super) fn staged_attempt_artifact(
    ctx: &ErasedRunCtx<'_>,
    evidence: store::ArtifactEvidenceRef,
) -> Result<StagedArtifact> {
    let binding = staged_artifact_binding_kind(evidence.artifact_role).expect("staged role");
    StagedArtifact::finalized_attempt_artifact_for_tests(ctx, evidence, binding)
}

pub(super) fn staged_side_effect_artifact(
    ctx: &ErasedRunCtx<'_>,
    evidence: store::ArtifactEvidenceRef,
    ledger_key: events::SideEffectLedgerKey,
    invocation_epoch: u32,
) -> Result<StagedArtifact> {
    let phase =
        staged_side_effect_artifact_phase(evidence.artifact_role).expect("staged side-effect role");
    StagedArtifact::finalized_attempt_artifact_for_tests(
        ctx,
        evidence,
        StagedArtifactBindingKind::SideEffectEvidence {
            ledger_key,
            invocation_epoch,
            phase,
        },
    )
}

pub(super) fn node_by_output<'a>(fixture: &'a Fixture, cell_id: &CellId) -> &'a spec::NodeSpec {
    fixture
        .runtime_spec
        .topological_order()
        .iter()
        .filter_map(|node_id| fixture.runtime_spec.node(node_id))
        .find(|node| &node.output_cell == cell_id)
        .expect("node by output")
}

pub(super) fn side_effect_verify_node_for_submit<'a>(
    fixture: &'a Fixture,
    submit: &spec::NodeSpec,
) -> &'a spec::NodeSpec {
    fixture
        .runtime_spec
        .topological_order()
        .iter()
        .filter_map(|node_id| fixture.runtime_spec.node(node_id))
        .find(|node| {
            matches!(
                &node.framework,
                Some(spec::FrameworkNodeSpec::SideEffectVerify(verify))
                    if verify.submit_node_id == submit.node_id
            )
        })
        .expect("side-effect verify node")
}

pub(super) fn effective_output_cell_for_node(fixture: &Fixture, node: &spec::NodeSpec) -> CellId {
    if node.side_effect.is_some() {
        side_effect_verify_node_for_submit(fixture, node)
            .output_cell
            .clone()
    } else {
        node.output_cell.clone()
    }
}

pub(super) fn append_attempt_start(
    store: &mut TestTypedRunStore,
    fixture: &Fixture,
    node: &spec::NodeSpec,
    attempt_no: u32,
) -> AttemptId {
    let attempt_id = attempt_id(
        &fixture.run_id,
        fixture.runtime_spec.spec_hash(),
        &node.node_id,
        attempt_no,
    )
    .expect("attempt id");
    store
        .append_prepared_commit(store_typed_commit_request! {
            run_id: fixture.run_id.clone(),
            expected_next_seq: store.expected_next_seq(&fixture.run_id),
            commit_key: store::CommitKey::new(format!(
                "manual-attempt-start:{}:{}",
                node.node_id, attempt_id
            ))
            .expect("commit key"),
            payloads: vec![events::KernelEventPayload::StateAttemptStarted(
                events::StateAttemptStarted {
                    spec_hash: fixture.runtime_spec.spec_hash().clone(),
                    node_id: node.node_id.clone(),
                    attempt_id: attempt_id.clone(),
                    attempt_no,
                    state_kind: node.state_kind.clone(),
                    state_version: node.state_version.clone(),
                },
            )],
            required_artifacts: Vec::new(),
            preconditions: store::CommitPreconditions {
                required_run_state: store::RequiredRunState::NotCompleted,
                ..store::CommitPreconditions::default()
            },
        })
        .expect("append attempt start");
    attempt_id
}

pub(super) fn append_or_get_started_attempt(
    store: &mut TestTypedRunStore,
    fixture: &Fixture,
    node: &spec::NodeSpec,
    attempt_no: u32,
) -> AttemptId {
    let attempt_id = attempt_id(
        &fixture.run_id,
        fixture.runtime_spec.spec_hash(),
        &node.node_id,
        attempt_no,
    )
    .expect("attempt id");
    if let Some(attempt) = store
        .projection_snapshot()
        .attempt(&node.node_id, &attempt_id)
        .cloned()
    {
        assert!(
            matches!(attempt.status, store::AttemptStatus::Started { .. }),
            "attempt {attempt_id} for node {} is not active",
            node.node_id
        );
        return attempt_id;
    }
    append_attempt_start(store, fixture, node, attempt_no)
}

pub(super) fn append_or_get_first_attempt(
    store: &mut TestTypedRunStore,
    fixture: &Fixture,
    node: &spec::NodeSpec,
) -> AttemptId {
    append_or_get_started_attempt(store, fixture, node, 1)
}

pub(super) fn append_attempt_failure(
    store: &mut TestTypedRunStore,
    fixture: &Fixture,
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
    retryable: bool,
) {
    store
        .append_prepared_commit(store_typed_commit_request! {
            run_id: fixture.run_id.clone(),
            expected_next_seq: store.expected_next_seq(&fixture.run_id),
            commit_key: store::CommitKey::new(format!(
                "manual-attempt-failure:{}:{}",
                node.node_id, attempt_id
            ))
            .expect("commit key"),
            payloads: vec![events::KernelEventPayload::StateAttemptFailed(
                events::StateAttemptFailed {
                    spec_hash: fixture.runtime_spec.spec_hash().clone(),
                    node_id: node.node_id.clone(),
                    attempt_id: attempt_id.clone(),
                    retryable,
                    error: events::MfmErrorInfo {
                        retryable,
                        ..public_output_error()
                    },
                },
            )],
            required_artifacts: Vec::new(),
            preconditions: store::CommitPreconditions {
                required_run_state: store::RequiredRunState::NotCompleted,
                required_present_logical_keys: vec![store::LogicalEventKey::new(format!(
                    "attempt:{}:{}",
                    node.node_id, attempt_id
                ))
                .expect("attempt logical key")],
                required_cell_states: vec![store::CellStatePrecondition {
                    cell_id: node.output_cell.clone(),
                    required: store::RequiredCellState::Absent,
                }],
                ..store::CommitPreconditions::default()
            },
        })
        .expect("append attempt failure");
}
