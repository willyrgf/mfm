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
    let (fixture, _descriptor, descriptor_ref) = fixture_with_fact_descriptor();
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
    let (fixture, descriptor, descriptor_ref) = fixture_with_fact_descriptor();
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

    let current = load_fixture_current(&scheduler, &store, &fixture)
        .await
        .expect("descriptor-backed journal verifies");
    let run_admitted = current.lifecycle().admission().expect("run admission");
    assert_eq!(run_admitted.fact_descriptor_artifacts().len(), 1);
    let admitted_descriptor = run_admitted
        .fact_descriptor_artifacts()
        .next()
        .expect("admitted fact descriptor");
    assert_eq!(
        admitted_descriptor.role,
        events::ArtifactRole::FactDescriptor
    );
    assert_eq!(
        admitted_descriptor.content_digest,
        descriptor_ref.descriptor_hash
    );
}

#[tokio::test]
async fn journal_load_rejects_fact_descriptor_without_artifact_authority() {
    let (fixture, descriptor, _) = fixture_with_fact_descriptor();
    let scheduler = test_scheduler(registered_fixture_runners(&fixture));
    let mut store = TestTypedRunStore::new();
    let mut evidence = run_start_evidence(&fixture, vec![fixture.seed_ref.clone()]);
    let descriptor_artifact = fact_descriptor_artifact(&descriptor);
    let descriptor_evidence = descriptor_artifact.evidence.clone();
    evidence.fact_descriptor_artifacts.push(descriptor_artifact);

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

    store.remove_committed_artifact_for_corruption(
        &descriptor_evidence.artifact_id,
        &descriptor_evidence
            .evidence_hash()
            .expect("descriptor evidence hash"),
    );
    let error = load_fixture_current(&scheduler, &store, &fixture)
        .await
        .expect_err("descriptor-bearing journal without its object must fail closed");
    assert!(matches!(
        error,
        RuntimeError::Store(message) if message.contains("missing")
    ));
}

#[tokio::test]
async fn run_start_admitted_uses_committed_fact_descriptor_artifacts() {
    let (fixture, descriptor, _) = fixture_with_fact_descriptor();
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
        .expect("descriptor-backed admission commits");
    let authority = load_fixture_current(&scheduler, &store, &fixture)
        .await
        .expect("descriptor-backed admission uses committed journal authority");

    assert_eq!(authority.view().run_id(), &fixture.run_id);
    assert_eq!(
        authority
            .lifecycle()
            .next_sequence()
            .expect("next journal sequence"),
        store.expected_next_seq(&fixture.run_id)
    );
}

#[tokio::test]
async fn fact_bearing_runtime_settlement_rebuild_uses_retained_artifact_bytes() {
    let (fixture, descriptor, _) = fixture_with_fact_descriptor();
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
    let current = load_fixture_current(&scheduler, &store, &fixture)
        .await
        .expect("load descriptor-backed run");
    let result = drive_current_once_with_claim(&scheduler, &store, current)
        .await
        .expect("produce first input cell");
    assert_eq!(result.status(), SchedulerStatus::Advanced);
    drop(result);
    let node = node_by_output(&fixture, &fixture.cell_b).clone();
    let attempt_id = append_attempt_start(&mut store, &fixture, &node, 1);
    append_fact_settlement(&mut store, &fixture, &node, &attempt_id, 17, 23);

    let current = load_fixture_current(&scheduler, &store, &fixture)
        .await
        .expect("fact-bearing journal validates with retained bytes");
    build_retention_manifest_artifact(&current.runtime_spec(), &current.lifecycle())
        .expect("fact-bearing prefix rebuilds with retained bytes");
    drop(current);

    let (response_evidence, response_bytes) = test_fact_response_artifact(&node, 23);
    let response_evidence_hash = response_evidence
        .evidence_hash()
        .expect("response evidence hash");
    store.replace_committed_artifact_for_corruption(
        &response_evidence.artifact_id,
        &response_evidence_hash,
        b"{\"amount\":999}".to_vec(),
    );
    load_fixture_current(&scheduler, &store, &fixture)
        .await
        .expect_err("fact-bearing journal with mismatched response bytes must fail");

    store.replace_committed_artifact_for_corruption(
        &response_evidence.artifact_id,
        &response_evidence_hash,
        response_bytes,
    );
    load_fixture_current(&scheduler, &store, &fixture)
        .await
        .expect("restored response bytes must validate");
    store.remove_committed_artifact_for_corruption(
        &response_evidence.artifact_id,
        &response_evidence_hash,
    );
    load_fixture_current(&scheduler, &store, &fixture)
        .await
        .expect_err("fact-bearing journal without retained response bytes must fail");
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
    assert_eq!(
        store.expected_next_seq(&fixture.run_id),
        store::StreamSeq::FIRST
    );
}
