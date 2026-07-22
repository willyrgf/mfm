use super::*;

#[tokio::test]
async fn materialization_rejects_seed_digest_not_certified() {
    let fixture = fixture();
    let mut seed = fixture.seed_ref.clone();
    seed.digest = content(0xee);
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let store = TestTypedRunStore::new();
    assert!(matches!(
        prepare_fixture_launch(&scheduler, &store, &fixture, vec![seed],).await,
        Err(RuntimeError::InvalidRunStream(_))
    ));
}

#[tokio::test]
async fn run_start_rejects_missing_config_artifact_evidence() {
    let fixture = fixture();
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let store = TestTypedRunStore::new();
    let mut evidence = run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]);
    evidence.config_artifacts.clear();
    assert!(matches!(
        scheduler
            .prepare_run_launch(
                &fixture.runtime_spec,
                fixture_run_identity_material(&fixture),
                evidence,
                store.expected_next_seq(&fixture.run_id),
            )
            .await,
        Err(RuntimeError::InvalidRunStream(_))
    ));
}

#[tokio::test]
async fn run_start_rejects_missing_fact_descriptor_artifact_evidence() {
    let (fixture, _descriptor, descriptor_ref) = fixture_with_first_node_fact_descriptor();
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let store = TestTypedRunStore::new();
    let evidence = run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]);

    assert!(matches!(
        scheduler.prepare_run_launch(
            &fixture.runtime_spec,
            fixture_run_identity_material(&fixture),
            evidence,
            store.expected_next_seq(&fixture.run_id),
        )
        .await,
        Err(RuntimeError::InvalidRunnerOutput(message))
            if message.contains("missing fact descriptor artifact")
                && message.contains(descriptor_ref.descriptor_hash.as_str())
    ));
}

#[tokio::test]
async fn run_start_admits_certified_fact_descriptor_artifacts() {
    let (fixture, descriptor, descriptor_ref) = fixture_with_first_node_fact_descriptor();
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let mut store = TestTypedRunStore::new();
    let mut evidence = run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]);
    evidence
        .fact_descriptor_artifacts
        .push(fact_descriptor_artifact(&descriptor));

    let launch = scheduler
        .prepare_run_launch(
            &fixture.runtime_spec,
            fixture_run_identity_material(&fixture),
            evidence,
            store.expected_next_seq(&fixture.run_id),
        )
        .await
        .expect("descriptor-backed launch prepares");
    scheduler_start_run(&scheduler, &mut store, launch)
        .await
        .expect("descriptor-backed launch commits");

    let run_admitted = store.run_admitted(&fixture.run_id);
    assert_eq!(run_admitted.fact_descriptor_artifacts.len(), 1);
    let admitted_descriptor = &run_admitted.fact_descriptor_artifacts[0];
    assert_eq!(
        admitted_descriptor.role,
        events::ArtifactRole::FactDescriptor
    );
    assert_eq!(
        admitted_descriptor.content_digest,
        descriptor_ref.descriptor_hash
    );
    let committed = block_on_ready(store.load_committed_run_stream(&fixture.run_id))
        .expect("descriptor-backed committed stream");
    RuntimeRunView::from_committed_stream(&fixture.runtime_spec, &committed)
        .expect("descriptor-backed run stream validates");
}

#[tokio::test]
async fn raw_runtime_view_rejects_fact_descriptor_stream_without_artifact_authority() {
    let (fixture, descriptor, _) = fixture_with_first_node_fact_descriptor();
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let mut store = TestTypedRunStore::new();
    let mut evidence = run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]);
    evidence
        .fact_descriptor_artifacts
        .push(fact_descriptor_artifact(&descriptor));

    let launch = scheduler
        .prepare_run_launch(
            &fixture.runtime_spec,
            fixture_run_identity_material(&fixture),
            evidence,
            store.expected_next_seq(&fixture.run_id),
        )
        .await
        .expect("descriptor-backed launch prepares");
    scheduler_start_run(&scheduler, &mut store, launch)
        .await
        .expect("descriptor-backed launch commits");

    let stream = store.load_run_stream(&fixture.run_id);
    let error = RuntimeRunView::from_stream(&fixture.runtime_spec, &fixture.run_id, &stream)
        .expect_err("raw descriptor-bearing stream must fail closed");
    assert!(matches!(
        error,
        RuntimeError::InvalidRunStream(message)
            if message.contains("committed stream artifact authority")
    ));
}

#[tokio::test]
async fn run_start_admitted_uses_committed_fact_descriptor_artifacts() {
    let (fixture, descriptor, _) = fixture_with_first_node_fact_descriptor();
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let mut store = TestTypedRunStore::new();
    let mut evidence = run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]);
    evidence
        .fact_descriptor_artifacts
        .push(fact_descriptor_artifact(&descriptor));
    let launch = scheduler
        .prepare_run_launch(
            &fixture.runtime_spec,
            fixture_run_identity_material(&fixture),
            evidence,
            store.expected_next_seq(&fixture.run_id),
        )
        .await
        .expect("descriptor-backed launch prepares");

    let authority =
        scheduler_start_run_admitted(&scheduler, &mut store, &fixture.runtime_spec, launch)
            .await
            .expect("descriptor-backed admission uses committed stream authority");

    assert_eq!(authority.run_id(), &fixture.run_id);
    assert_eq!(
        authority.head_seq(),
        store.expected_next_seq(&fixture.run_id)
    );
}

#[tokio::test]
async fn fact_bearing_runtime_settlement_rebuild_uses_retained_artifact_bytes() {
    let (fixture, descriptor, _) = fixture_with_read_node_fact_descriptor();
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let mut store = TestTypedRunStore::new();
    let mut evidence = run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]);
    evidence
        .fact_descriptor_artifacts
        .push(fact_descriptor_artifact(&descriptor));
    let launch = scheduler
        .prepare_run_launch(
            &fixture.runtime_spec,
            fixture_run_identity_material(&fixture),
            evidence,
            store.expected_next_seq(&fixture.run_id),
        )
        .await
        .expect("descriptor-backed launch prepares");
    scheduler_start_run(&scheduler, &mut store, launch)
        .await
        .expect("descriptor-backed launch commits");
    let node_a = node_by_output(&fixture, &fixture.cell_a).clone();
    let node_a_attempt = append_attempt_start(&mut store, &fixture, &node_a, 1);
    append_terminal(
        &mut store,
        &fixture,
        &node_a,
        &node_a_attempt,
        artifact(0x70),
        content(0x71),
    );
    let node = node_by_output(&fixture, &fixture.cell_b).clone();
    let attempt_id = append_attempt_start(&mut store, &fixture, &node, 1);
    append_fact_settlement(&mut store, &fixture, &node, &attempt_id, 17, 23);

    let stream = store.load_run_stream(&fixture.run_id);
    let raw_error = RuntimeRunView::from_stream(&fixture.runtime_spec, &fixture.run_id, &stream)
        .expect_err("raw fact-bearing stream must fail closed");
    assert!(matches!(
        raw_error,
        RuntimeError::InvalidRunStream(message)
            if message.contains("committed stream artifact authority")
    ));
    let committed = block_on_ready(store.load_committed_run_stream(&fixture.run_id))
        .expect("fact-bearing committed stream");
    RuntimeRunView::from_committed_stream(&fixture.runtime_spec, &committed)
        .expect("fact-bearing runtime view validates with retained bytes");
    build_retention_manifest_artifact(
        &fixture.runtime_spec,
        &fixture.run_id,
        &stream,
        committed.artifact_byte_authority(),
    )
    .expect("fact-bearing prefix rebuilds with retained bytes");

    let missing = store::ArtifactByteAuthorityMap::new();
    build_retention_manifest_artifact(&fixture.runtime_spec, &fixture.run_id, &stream, &missing)
        .expect_err("fact-bearing prefix without retained bytes must fail");

    let (response_evidence, _) = test_fact_response_artifact(&node, 23);
    let response_key = (
        response_evidence.artifact_id.clone(),
        response_evidence
            .evidence_hash()
            .expect("response evidence hash"),
    );
    let mut mismatched = committed.artifact_byte_authority().clone();
    mismatched
        .get_mut(&response_key)
        .expect("response bytes retained")
        .0 = b"{\"amount\":999}".to_vec();
    build_retention_manifest_artifact(&fixture.runtime_spec, &fixture.run_id, &stream, &mismatched)
        .expect_err("fact-bearing prefix with mismatched response bytes must fail");
}

#[tokio::test]
async fn run_start_rejects_mismatched_staged_launch_bytes() {
    let fixture = fixture();
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let store = TestTypedRunStore::new();
    let base = run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]);

    let mut bad_spec = base.clone();
    bad_spec.spec_artifact.bytes.push(b'\n');
    assert!(matches!(
        scheduler
            .prepare_run_launch(
                &fixture.runtime_spec,
                fixture_run_identity_material(&fixture),
                bad_spec,
                store.expected_next_seq(&fixture.run_id),
            )
            .await,
        Err(RuntimeError::InvalidRunnerOutput(_))
    ));

    let mut bad_certificate = base.clone();
    bad_certificate.certificate_artifact.bytes.push(b'\n');
    assert!(matches!(
        scheduler
            .prepare_run_launch(
                &fixture.runtime_spec,
                fixture_run_identity_material(&fixture),
                bad_certificate,
                store.expected_next_seq(&fixture.run_id),
            )
            .await,
        Err(RuntimeError::InvalidRunnerOutput(_))
    ));

    let mut bad_config = base.clone();
    bad_config
        .config_artifacts
        .first_mut()
        .expect("config artifact")
        .bytes
        .push(b'\n');
    assert!(matches!(
        scheduler
            .prepare_run_launch(
                &fixture.runtime_spec,
                fixture_run_identity_material(&fixture),
                bad_config,
                store.expected_next_seq(&fixture.run_id),
            )
            .await,
        Err(RuntimeError::InvalidRunnerOutput(_))
    ));

    let mut bad_seed = base;
    bad_seed
        .seed_cells
        .first_mut()
        .expect("seed cell")
        .bytes
        .push(b'\n');
    assert!(matches!(
        scheduler
            .prepare_run_launch(
                &fixture.runtime_spec,
                fixture_run_identity_material(&fixture),
                bad_seed,
                store.expected_next_seq(&fixture.run_id),
            )
            .await,
        Err(RuntimeError::InvalidRunnerOutput(_))
    ));
    assert!(store.load_run_stream(&fixture.run_id).is_empty());
}
