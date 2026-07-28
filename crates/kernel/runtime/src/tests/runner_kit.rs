use super::*;

fn started_current(fixture: &Fixture) -> VerifiedCurrentRun {
    let scheduler = test_scheduler(registered_fixture_runners(fixture));
    let mut store = TestTypedRunStore::new();
    block_on_ready(started_fixture_current(
        &scheduler,
        &mut store,
        fixture,
        vec![fixture.seed_ref.clone()],
    ))
    .expect("started fixture current run")
}

#[test]
fn exact_object_lookup_rejects_same_content_with_wrong_evidence_hash() {
    let fixture = fixture();
    let current = started_current(&fixture);
    let lifecycle = current.lifecycle();
    let mut requirement = store::seed_cell_artifact_requirement(&fixture.seed_ref);
    assert!(
        lifecycle.object_for_requirement(&requirement).is_some(),
        "the exact admitted seed requirement resolves"
    );

    requirement.evidence_hash = content(0xee);
    assert!(
        lifecycle.object_for_requirement(&requirement).is_none(),
        "the same artifact id and content must not authorize a different evidence identity"
    );
}

#[test]
fn exact_object_lookup_rejects_object_absent_from_the_exact_event_requirement() {
    let fixture = fixture();
    let current = started_current(&fixture);
    let lifecycle = current.lifecycle();
    let admitted = store::seed_cell_artifact_requirement(&fixture.seed_ref);
    let mut unrelated = admitted.clone();
    unrelated.source = store::EventArtifactReferenceSource::ArtifactReferenced;

    assert!(lifecycle.object_for_requirement(&admitted).is_some());
    assert!(
        lifecycle.object_for_requirement(&unrelated).is_none(),
        "object presence alone must not mint event-requirement authority"
    );
}

#[test]
fn admitted_seed_input_resolves_its_full_run_admitted_requirement() {
    let fixture = fixture();
    let current = started_current(&fixture);
    let requirement = store::seed_cell_artifact_requirement(&fixture.seed_ref);
    let object = current
        .lifecycle()
        .object_for_requirement(&requirement)
        .expect("full RunAdmitted seed requirement");

    assert_eq!(object.bytes(), CERTIFIER_SEED_BYTES);
    assert_eq!(
        object.evidence().artifact_id,
        fixture.seed_ref.seed_artifact.artifact_id
    );
    assert_eq!(
        object.evidence().evidence_hash().expect("evidence hash"),
        requirement.evidence_hash
    );
}

#[test]
fn runner_kit_loads_certified_config_and_materialized_seed_input() {
    let fixture = fixture();
    let node = node_by_output(&fixture, &fixture.cell_a);
    with_runner_erased_ctx_for_node(&fixture, node, |ctx| {
        let config =
            load_runner_config_for_node::<CertifierConfig>(&ctx, ctx.node()).expect("config");
        assert_eq!(config.into_inner(), CertifierConfig { multiplier: 3 });
        let input = load_materialized_input::<CertifierValue>(&ctx).expect("seed input");
        assert_eq!(input, CertifierValue { amount: 2 });
    });
}

#[test]
fn runner_kit_rejects_non_certified_node_config_load() {
    let fixture = fixture();
    let node = node_by_output(&fixture, &fixture.cell_a);
    with_runner_erased_ctx_for_node(&fixture, node, |ctx| {
        let mut non_certified = ctx.node().clone();
        non_certified.config_ref = node_by_output(&fixture, &fixture.cell_b).config_ref.clone();
        let error = load_runner_config_for_node::<CertifierConfig>(&ctx, &non_certified)
            .expect_err("a non-certified NodeSpec must not load configuration");
        assert!(matches!(
            error,
            RuntimeError::InvalidRunnerOutput(message)
                if message.contains("not part of the certified runtime spec")
        ));
    });
}

#[test]
fn runner_kit_rejects_skipped_materialized_input_cell() {
    let fixture = fixture();
    let node = node_by_output(&fixture, &fixture.cell_a);
    with_runner_erased_ctx_for_node(&fixture, node, |ctx| {
        let error =
            load_materialized_node_value::<CertifierValue>(&ctx, &runner_kit_skipped_cell())
                .expect_err("skipped cells cannot be loaded");
        assert!(matches!(
            error,
            RuntimeError::InvalidRunnerOutput(message) if message.contains("skipped")
        ));
    });
}

#[test]
fn runner_kit_builders_create_context_bound_artifacts_payloads_and_output() {
    let fixture = fixture();
    let node = node_by_output(&fixture, &fixture.cell_a).clone();

    with_runner_erased_ctx_for_node(&fixture, &node, |ctx| {
        let artifacts = RunnerArtifactBuilder::new(&ctx);
        let payloads = RunnerPayloadBuilder::new(&ctx);
        let value = CertifierValue { amount: 42 };
        let state = artifacts.state_output(&value).expect("state output");
        assert_eq!(
            state.evidence().artifact_role,
            events::ArtifactRole::StateOutput
        );
        assert_eq!(
            state.evidence().producer_node_id.as_ref(),
            Some(&ctx.node().node_id)
        );
        assert_eq!(
            artifacts.content_digest(&value).expect("content digest"),
            state.evidence().digest
        );

        let cell = payloads.cell_produced(&state).expect("cell payload");
        match &cell {
            RunnerEventPayload::CellProduced(payload) => {
                assert_eq!(payload.spec_hash, *ctx.spec_hash());
                assert_eq!(payload.node_id, ctx.node().node_id);
                assert_eq!(payload.cell_id, ctx.node().output_cell);
                assert_eq!(payload.schema_id, ctx.output_cell().schema_id);
                assert_eq!(payload.semantic_type_id, ctx.output_cell().semantic_type_id);
                assert_eq!(payload.artifact_id, state.evidence().artifact_id);
                assert_eq!(payload.content_digest, state.evidence().digest);
            }
            _ => panic!("expected cell produced payload"),
        }

        let response = artifacts.fact_response(&value).expect("fact response");
        assert_eq!(
            response.evidence().artifact_role,
            events::ArtifactRole::FactResponse
        );
        assert_eq!(
            response.evidence().producer_node_id.as_ref(),
            Some(&ctx.node().node_id)
        );

        let query_evidence = test_fact_query_evidence();
        let expected_query_evidence_hash =
            mfm_facts::fact_query_evidence_hash(&query_evidence).expect("query evidence hash");
        let expected_query_evidence_schema =
            mfm_facts::fact_query_evidence_schema_id().expect("query evidence schema");
        let mut query_output = RunnerOutputBuilder::new(&ctx);
        query_output
            .record_fact_query_evidence(query_evidence)
            .expect("record fact query evidence");
        let query_output = query_output.finish();
        assert_eq!(query_output.staged_artifacts().len(), 1);
        assert_eq!(query_output.staged_retention_refs().len(), 1);
        assert!(query_output.payloads().is_empty());
        let query_artifact = &query_output.staged_artifacts()[0];
        assert_eq!(
            query_artifact.evidence().artifact_role,
            events::ArtifactRole::FactQueryEvidence
        );
        assert_eq!(
            query_artifact.evidence().schema_id.as_ref(),
            Some(&expected_query_evidence_schema)
        );
        assert!(query_artifact.evidence().semantic_type_id.is_none());
        assert_eq!(
            query_artifact.evidence().digest,
            expected_query_evidence_hash
        );
        let expected_query_retention = events::RetentionRef {
            artifact_id: query_artifact.evidence().artifact_id.clone(),
            role: events::ArtifactRole::FactQueryEvidence,
            evidence_hash: query_artifact
                .evidence()
                .evidence_hash()
                .expect("query evidence hash"),
            content_digest: query_artifact.evidence().digest.clone(),
        };
        assert_eq!(
            query_output.staged_retention_refs()[0].refs(),
            &[expected_query_retention]
        );

        let role_mismatch = payloads
            .cell_produced(&response)
            .expect_err("fact response cannot produce a cell");
        assert!(matches!(
            role_mismatch,
            RuntimeError::InvalidRunnerOutput(message)
                if message.contains("fact_response") && message.contains("state_output")
        ));

        let mut state_output = RunnerOutputBuilder::new(&ctx);
        state_output
            .stage_attempt_artifact(&state)
            .expect("stage state output")
            .retain_runtime_evidence(&state)
            .expect("retain state output")
            .payload(cell.clone());
        let state_output = state_output.finish();
        assert_eq!(state_output.staged_artifacts().len(), 1);
        assert_eq!(state_output.staged_retention_refs().len(), 1);
        assert_eq!(state_output.payloads(), vec![cell]);
    });
}

struct PendingIngressExecutor {
    started: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    release: Mutex<Option<tokio::sync::oneshot::Receiver<()>>>,
}

impl ExternalReadPlanExecutor<RuntimeReadState> for PendingIngressExecutor {
    fn validate_ingress<'a>(
        &'a self,
        _ctx: RunnerIngressContext<'a>,
        _state: &'a RuntimeReadState,
    ) -> RunnerIngressFuture<'a> {
        Box::pin(async move {
            self.started
                .lock()
                .expect("started signal")
                .take()
                .expect("one ingress validation")
                .send(())
                .expect("started receiver");
            let release = self
                .release
                .lock()
                .expect("release signal")
                .take()
                .expect("one ingress validation");
            release.await.expect("release sender");
            Ok(())
        })
    }

    fn execute<'a>(
        &'a self,
        _plan: &'a FixtureOutputValue,
        _ctx: &'a ErasedRunCtx<'_>,
    ) -> ExternalReadExecutionFuture<'a, FixtureOutputValue> {
        Box::pin(async { unreachable!("ingress validation must not execute the read plan") })
    }
}

#[tokio::test]
async fn external_read_runner_awaits_pending_async_ingress_validation() {
    let fixture = fixture_with_first_side_effect_state();
    let node = node_by_output(&fixture, &fixture.cell_b);
    let launch = run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]);
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel();
    let runner = ExternalReadRunner::<RuntimeReadState, _>::new(PendingIngressExecutor {
        started: Mutex::new(Some(started_tx)),
        release: Mutex::new(Some(release_rx)),
    });
    let context = RunnerIngressContext::new(&fixture.runtime_spec, node, &launch);
    let mut validation = ErasedNodeRunner::validate_ingress(&runner, context);

    tokio::select! {
        result = &mut validation => panic!("ingress returned before async validation: {result:?}"),
        started = started_rx => started.expect("pending validator started"),
    }
    release_tx.send(()).expect("release pending validator");
    validation.await.expect("async ingress validation");
}
