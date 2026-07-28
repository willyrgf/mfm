use super::*;

fn append_fact_attempt_start(
    store: &mut StoreContractRunStore,
    run_id: &RunId,
    commit_key: &str,
    attempt_id: AttemptId,
) {
    let mut payloads = vec![fact_attempt_started_for(attempt_id)];
    store.certify_payloads_for_run(run_id, &mut payloads);
    append_run_state_commit(
        store,
        run_id,
        commit_key,
        payloads,
        Vec::new(),
        RequiredRunState::NotCompleted,
    )
    .expect("append fact attempt start");
}

fn fact_event(outcome: &CommitOutcome) -> &KernelEventEnvelope {
    let batch = match outcome {
        CommitOutcome::Appended(batch) | CommitOutcome::Idempotent(batch) => batch,
        CommitOutcome::AdmissionBlocked(_) => panic!("fact append must not admission-block"),
        CommitOutcome::ExecutionClaimBusy(_) => {
            panic!("fact append must not hit execution claim admission")
        }
    };
    assert_eq!(batch.events().len(), 3);
    batch
        .events()
        .iter()
        .find(|event| matches!(event.payload(), KernelEventPayload::FactRecorded(_)))
        .expect("fact-recorded event")
}

fn memory_fact_query(
    store: &AsyncInMemoryRunStore,
    plan: &mfm_facts::CanonicalFactQueryPlan,
) -> mfm_facts::FactQueryResult {
    let results = poll_ready_store_future(mfm_store::v1::FactQueryStore::execute_fact_queries(
        store,
        std::slice::from_ref(plan),
    ))
    .expect("memory fact query");
    results.into_iter().next().expect("one aligned result")
}

fn memory_query_overlapping_append(
    store: &AsyncInMemoryRunStore,
    plan: &mfm_facts::CanonicalFactQueryPlan,
    bundle: PreparedCommitBundle,
) -> (mfm_facts::FactQueryResult, CommitOutcome) {
    store
        .seed_artifact_evidence_for_test(bundle.admitted_artifacts())
        .expect("seed existing artifact authority");
    let snapshot_fixed = std::sync::Arc::new(std::sync::Barrier::new(2));
    let release_snapshot = std::sync::Arc::new(std::sync::Barrier::new(2));
    store
        .set_fact_query_snapshot_hook_for_test({
            let snapshot_fixed = std::sync::Arc::clone(&snapshot_fixed);
            let release_snapshot = std::sync::Arc::clone(&release_snapshot);
            move || {
                snapshot_fixed.wait();
                release_snapshot.wait();
            }
        })
        .expect("install memory query barrier");

    let query_store = store.clone();
    let query_plan = plan.clone();
    let query = std::thread::spawn(move || memory_fact_query(&query_store, &query_plan));
    snapshot_fixed.wait();

    let append_store = store.clone();
    let append_started = std::sync::Arc::new(std::sync::Barrier::new(2));
    let append = std::thread::spawn({
        let append_started = std::sync::Arc::clone(&append_started);
        move || {
            append_started.wait();
            poll_ready_store_future(append_store.append_prepared_commit_bundle(bundle))
                .expect("overlapping memory append")
        }
    });
    append_started.wait();
    release_snapshot.wait();

    (
        query.join().expect("memory query thread"),
        append.join().expect("memory append thread"),
    )
}

fn fact_attempt_start_bundle(run_id: &RunId) -> PreparedCommitBundle {
    let authority = CertifiedRunStoreAuthority::from_spec(
        run_id.clone(),
        &fact_authority_spec_with_node(node_id(90)),
    )
    .expect("fact authority");
    let mut attempt_started = fact_attempt_started();
    let KernelEventPayload::StateAttemptStarted(payload) = &mut attempt_started else {
        unreachable!("fact attempt fixture is an attempt-start payload")
    };
    payload.spec_hash = authority.spec_hash().clone();
    let request = CommitRequest::from_payloads(
        run_id.clone(),
        StreamSeq::new(2).expect("attempt-start sequence"),
        CommitKey::new("memory-fact-attempt-start").expect("commit key"),
        vec![attempt_started],
        Vec::new(),
        CommitPreconditions {
            required_run_state: RequiredRunState::Started,
            certified_run_authority: Some(authority),
            ..CommitPreconditions::default()
        },
    )
    .expect("fact attempt-start request");
    test_bundle_from_plan(
        test_prepared_commit_plan(request, Vec::new()).expect("fact attempt-start plan"),
    )
    .expect("fact attempt-start bundle")
}

fn fact_settlement_bundle(run_id: &RunId) -> PreparedCommitBundle {
    let response = fact_artifact_ref();
    let output = fact_output_artifact_ref_for_height(850000);
    let authority = CertifiedRunStoreAuthority::from_spec(
        run_id.clone(),
        &fact_authority_spec_with_node(node_id(90)),
    )
    .expect("fact authority");
    let mut payloads = vec![
        fact_recorded(&response),
        fact_cell_produced_for_height(850000, attempt_id(91)),
        fact_attempt_completed(attempt_id(91)),
    ];
    for payload in &mut payloads {
        match payload {
            KernelEventPayload::FactRecorded(payload) => {
                payload.spec_hash = authority.spec_hash().clone();
            }
            KernelEventPayload::CellProduced(payload) => {
                payload.spec_hash = authority.spec_hash().clone();
            }
            KernelEventPayload::StateAttemptCompleted(payload) => {
                payload.spec_hash = authority.spec_hash().clone();
            }
            _ => unreachable!("fact settlement fixture has a closed payload set"),
        }
    }
    let request = CommitRequest::from_payloads(
        run_id.clone(),
        StreamSeq::new(3).expect("fact-settlement sequence"),
        CommitKey::new("memory-fact-settlement").expect("commit key"),
        payloads,
        vec![response.clone(), output.clone()],
        CommitPreconditions {
            required_run_state: RequiredRunState::Started,
            certified_run_authority: Some(authority),
            ..CommitPreconditions::default()
        },
    )
    .expect("fact settlement request");
    let plan = test_prepared_commit_plan(request, vec![response.clone(), output.clone()])
        .expect("fact settlement plan");
    PreparedCommitBundle::new(
        plan,
        vec![
            PreparedArtifactBytes::new(fact_response_bytes(), response)
                .expect("fact response bytes"),
            PreparedArtifactBytes::new(fact_response_bytes(), output).expect("fact output bytes"),
        ],
        Vec::new(),
    )
    .expect("fact settlement bundle")
}

#[test]
fn memory_fact_queries_use_one_before_or_after_snapshot_for_overlapping_appends() {
    let plan = fact_query_plan();
    let descriptor_store = AsyncInMemoryRunStore::new();
    let descriptor_run = fact_run_id(101);
    let (before_descriptor, descriptor_append) = memory_query_overlapping_append(
        &descriptor_store,
        &plan,
        fact_run_start_bundle_with_node(descriptor_run, "memory-descriptor-admission", node_id(90)),
    );
    assert!(matches!(descriptor_append, CommitOutcome::Appended(_)));
    assert!(before_descriptor.rows().is_empty());
    assert_eq!(
        before_descriptor
            .receipt()
            .read_frontier()
            .store_commit_order()
            .as_u64(),
        0
    );
    let after_descriptor = memory_fact_query(&descriptor_store, &plan);
    assert!(after_descriptor.rows().is_empty());
    assert_eq!(
        after_descriptor
            .receipt()
            .read_frontier()
            .store_commit_order()
            .as_u64(),
        1
    );
    assert_eq!(
        descriptor_store
            .projection_snapshot()
            .expect("descriptor projection")
            .fact_descriptors()
            .count(),
        1
    );

    let fact_store = AsyncInMemoryRunStore::new();
    let fact_run = fact_run_id(102);
    let start =
        fact_run_start_bundle_with_node(fact_run.clone(), "memory-fact-run-start", node_id(90));
    fact_store
        .seed_artifact_evidence_for_test(start.admitted_artifacts())
        .expect("seed run-start authority");
    poll_ready_store_future(fact_store.append_prepared_commit_bundle(start))
        .expect("append fact run start");
    poll_ready_store_future(
        fact_store.append_prepared_commit_bundle(fact_attempt_start_bundle(&fact_run)),
    )
    .expect("append fact attempt start");

    let (before_fact, fact_append) =
        memory_query_overlapping_append(&fact_store, &plan, fact_settlement_bundle(&fact_run));
    assert!(matches!(fact_append, CommitOutcome::Appended(_)));
    assert!(before_fact.rows().is_empty());
    assert_eq!(
        before_fact
            .receipt()
            .read_frontier()
            .store_commit_order()
            .as_u64(),
        2
    );
    let after_fact = memory_fact_query(&fact_store, &plan);
    assert_eq!(after_fact.rows().len(), 1);
    assert_eq!(
        after_fact
            .receipt()
            .read_frontier()
            .store_commit_order()
            .as_u64(),
        3
    );
}

#[test]
fn fact_recorded_projects_reusable_fact_evidence() {
    let run_id = fact_run_id(95);
    let mut store = admitted_fact_store(&run_id, "run-start");
    let response = fact_artifact_ref();
    append_fact_attempt_start(&mut store, &run_id, "fact-attempt-start", attempt_id(91));
    append_fact_recorded_commit(&mut store, &run_id, "fact-recorded").expect("append fact");

    let snapshot = store.projection_snapshot();
    assert_eq!(snapshot.fact_descriptors().count(), 1);
    assert_eq!(snapshot.fact_query_entries().count(), 1);
    let projection = snapshot
        .fact_query_entries()
        .next()
        .map(|(_, projection)| projection)
        .expect("fact query projection");
    assert_eq!(projection.terms().count(), 2);
    assert_eq!(projection.artifact_id(), &response.artifact_id);
    assert_eq!(projection.response_hash(), &response.digest);
    assert_eq!(projection.fact_key(), &fact_key());
    assert_eq!(projection.response_artifact_evidence(), Some(&response));
}

#[test]
fn fact_query_projection_constructor_derives_event_coordinates() {
    let run_id = fact_run_id(96);
    let mut store = admitted_fact_store(&run_id, "constructor-run-start");
    let response = fact_artifact_ref();
    append_fact_attempt_start(
        &mut store,
        &run_id,
        "constructor-attempt-start",
        attempt_id(91),
    );
    let outcome = append_fact_recorded_commit(&mut store, &run_id, "constructor-fact-recorded")
        .expect("append fact");
    let CommitOutcome::Appended(batch) = outcome else {
        panic!("fact append should be new");
    };
    assert_eq!(batch.events().len(), 3);
    assert!(matches!(
        batch.events()[0].payload(),
        KernelEventPayload::FactRecorded(_)
    ));
    assert!(matches!(
        batch.events()[1].payload(),
        KernelEventPayload::CellProduced(_)
    ));
    assert!(matches!(
        batch.events()[2].payload(),
        KernelEventPayload::StateAttemptCompleted(_)
    ));
    let event = &batch.events()[0];
    let KernelEventPayload::FactRecorded(payload) = event.payload() else {
        panic!("fact event payload");
    };
    let fact_claim_id = mfm_facts::derive_fact_claim_id(
        event.run_id().clone(),
        event.seq().as_u64(),
        event.ordinal().as_u32(),
    )
    .expect("claim id");
    let terms = store
        .projection_snapshot()
        .fact_query_entry(&fact_claim_id)
        .expect("projected query")
        .terms()
        .cloned()
        .collect::<Vec<_>>();

    let with_evidence = mfm_store::v1::FactQueryProjection::from_recorded_event(
        event,
        payload,
        Some(response.clone()),
        terms.clone(),
    )
    .expect("fact query projection");
    assert_eq!(with_evidence.fact_claim_id(), &fact_claim_id);
    assert_eq!(with_evidence.source_event_id(), event.event_id());
    assert_eq!(with_evidence.source_run_id(), event.run_id());
    assert_eq!(with_evidence.source_seq(), event.seq().as_u64());
    assert_eq!(with_evidence.source_ordinal(), event.ordinal().as_u32());
    assert_eq!(with_evidence.producer_node_id(), &payload.node_id);
    assert_eq!(with_evidence.attempt_id(), &payload.attempt_id);
    assert_eq!(with_evidence.commit_id(), event.commit_key());
    assert_eq!(
        with_evidence.store_commit_order(),
        event.store_commit_order().as_u64()
    );
    assert_eq!(with_evidence.response_artifact_evidence(), Some(&response));
    let fact_ref = with_evidence.internal_ref().expect("internal fact ref");
    assert_eq!(fact_ref.fact_kind(), payload.claim.fact_kind());
    assert_eq!(fact_ref.fact_key(), payload.claim.subject().fact_key());
    assert_eq!(
        fact_ref.subject_material_hash(),
        payload.claim.subject().subject_material_hash()
    );
    assert_eq!(
        fact_ref.response_schema_id(),
        payload.claim.response().response_schema_id()
    );
    assert_eq!(
        fact_ref.response_hash(),
        payload.claim.response().response_hash()
    );
    assert_eq!(
        store
            .projection_snapshot()
            .fact_query_entry(with_evidence.fact_claim_id()),
        Some(&with_evidence)
    );

    let without_evidence =
        mfm_store::v1::FactQueryProjection::from_recorded_event(event, payload, None, terms)
            .expect("fact query projection without retained evidence");
    assert_eq!(without_evidence.response_artifact_evidence(), None);
    assert_eq!(
        without_evidence.internal_ref().expect("internal fact ref"),
        fact_ref
    );

    let duplicate_term = mfm_facts::FactQueryTerm::from_parts(
        mfm_facts::FactFieldId::new("subject.account").expect("field id"),
        mfm_facts::FactFieldSource::Subject,
        mfm_facts::FactFieldValueType::String,
        mfm_facts::FactCanonicalScalar::String("alice".to_owned()),
        None,
        None,
    )
    .expect("query term");
    let error = mfm_store::v1::FactQueryProjection::from_recorded_event(
        event,
        payload,
        None,
        vec![duplicate_term.clone(), duplicate_term],
    )
    .expect_err("duplicate query field");
    assert!(matches!(error, StoreError::ProjectionConflict { .. }));
}

#[test]
fn fact_recorded_same_subject_claims_are_claim_id_distinct() {
    fn expected_fact_logical_key(event: &KernelEventEnvelope) -> String {
        let claim_id = mfm_facts::derive_fact_claim_id(
            event.run_id().clone(),
            event.seq().as_u64(),
            event.ordinal().as_u32(),
        )
        .expect("claim id");
        format!(
            "fact:{}:{}:{}",
            claim_id.source_run_id(),
            claim_id.source_seq(),
            claim_id.source_ordinal()
        )
    }

    let first_run_id = fact_run_id(98);
    let second_run_id = fact_run_id(198);
    let mut store = admitted_fact_store(&first_run_id, "same-subject-first-run-start");
    store
        .append_test_commit_bundle(fact_run_start_bundle_with_node(
            second_run_id.clone(),
            "same-subject-second-run-start",
            node_id(90),
        ))
        .expect("append second fact run start");
    append_fact_attempt_start(
        &mut store,
        &first_run_id,
        "same-subject-first-attempt-start",
        attempt_id(91),
    );
    append_fact_attempt_start(
        &mut store,
        &second_run_id,
        "same-subject-second-attempt-start",
        attempt_id(191),
    );

    let first =
        append_fact_recorded_commit_for_height(&mut store, &first_run_id, "same-subject-1", 850000)
            .expect("append first fact");
    let second = append_fact_recorded_commit_for_height_and_attempt(
        &mut store,
        &second_run_id,
        "same-subject-2",
        850001,
        attempt_id(191),
    )
    .expect("append second same-subject fact");

    let first_event = fact_event(&first);
    let second_event = fact_event(&second);
    assert_ne!(first_event.logical_key(), second_event.logical_key());
    assert_eq!(
        first_event.logical_key().as_str(),
        expected_fact_logical_key(first_event)
    );
    assert_eq!(
        second_event.logical_key().as_str(),
        expected_fact_logical_key(second_event)
    );
    assert!(!first_event
        .logical_key()
        .as_str()
        .contains(fact_key().as_str()));
    assert!(!second_event
        .logical_key()
        .as_str()
        .contains(fact_key().as_str()));

    let snapshot = store.projection_snapshot();
    assert_eq!(snapshot.fact_query_entries().count(), 2);
    let claim_ids = snapshot
        .fact_query_entries()
        .map(|(claim_id, projection)| {
            assert_eq!(projection.fact_key(), &fact_key());
            assert_eq!(claim_id, projection.fact_claim_id());
            claim_id.clone()
        })
        .collect::<Vec<_>>();
    assert_eq!(claim_ids.len(), 2);
    assert_ne!(claim_ids[0], claim_ids[1]);
}

#[test]
fn byte_identical_fact_responses_are_reusable_across_append_occurrences() {
    let first_run_id = fact_run_id(99);
    let second_run_id = fact_run_id(199);
    let mut store = admitted_fact_store(&first_run_id, "shared-response-first-run-start");
    store
        .append_test_commit_bundle(fact_run_start_bundle_with_node(
            second_run_id.clone(),
            "shared-response-second-run-start",
            node_id(90),
        ))
        .expect("append second fact run start");
    append_fact_attempt_start(
        &mut store,
        &first_run_id,
        "shared-response-first-attempt-start",
        attempt_id(91),
    );
    append_fact_attempt_start(
        &mut store,
        &second_run_id,
        "shared-response-second-attempt-start",
        attempt_id(191),
    );

    append_fact_recorded_commit_for_height(
        &mut store,
        &first_run_id,
        "shared-response-first",
        850000,
    )
    .expect("append first fact occurrence");
    append_fact_recorded_commit_for_height_and_attempt(
        &mut store,
        &second_run_id,
        "shared-response-second",
        850000,
        attempt_id(191),
    )
    .expect("append byte-identical fact occurrence");

    let snapshot = store.projection_snapshot();
    assert_eq!(snapshot.fact_query_entries().count(), 2);
    let response_artifacts = snapshot
        .fact_query_entries()
        .map(|(_, projection)| projection.artifact_id().clone())
        .collect::<Vec<_>>();
    assert_eq!(response_artifacts.len(), 2);
    assert_eq!(response_artifacts[0], response_artifacts[1]);
}

#[test]
fn fact_recorded_requires_started_attempt_projection() {
    let run_id = fact_run_id(96);
    let mut store = admitted_fact_store(&run_id, "run-start");

    let error = append_fact_recorded_commit(&mut store, &run_id, "fact-before-attempt")
        .expect_err("fact before attempt must reject");
    assert!(matches!(error, StoreError::ProjectionConflict { .. }));
}

#[test]
fn fact_settlement_requires_one_ordered_terminal_bundle_and_unique_keys() {
    let run_id = fact_run_id(100);
    let mut store = admitted_fact_store(&run_id, "run-start");
    append_fact_attempt_start(&mut store, &run_id, "fact-attempt-start", attempt_id(91));
    let response = fact_artifact_ref();
    let output = fact_output_artifact_ref_for_height(850000);

    let reject = |commit_key: &str,
                  mut payloads: Vec<KernelEventPayload>,
                  required_artifacts: Vec<ArtifactEvidenceRef>,
                  expected: &str| {
        store.certify_payloads_for_run(&run_id, &mut payloads);
        let mut preconditions = store.certified_preconditions(&run_id);
        preconditions.required_run_state = RequiredRunState::Started;
        let request = typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(&run_id),
            commit_key: CommitKey::new(commit_key).expect("commit key"),
            payloads: payloads,
            required_artifacts: required_artifacts.clone(),
            preconditions: preconditions,
        };
        let error = test_prepared_commit_plan(request, required_artifacts)
            .expect_err("invalid fact settlement must be rejected before append");
        assert!(
            error.to_string().contains(expected),
            "expected `{expected}` in {error}"
        );
    };

    reject(
        "fact-without-cell",
        vec![
            fact_recorded(&response),
            fact_attempt_completed(attempt_id(91)),
        ],
        vec![response.clone()],
        "attempt completion requires matching terminal cell",
    );
    reject(
        "fact-without-completion",
        vec![
            fact_recorded(&response),
            fact_cell_produced_for_height(850000, attempt_id(91)),
        ],
        vec![response.clone(), output.clone()],
        "terminal cell requires matching attempt completion",
    );
    reject(
        "fact-after-cell",
        vec![
            fact_cell_produced_for_height(850000, attempt_id(91)),
            fact_recorded(&response),
            fact_attempt_completed(attempt_id(91)),
        ],
        vec![response.clone(), output.clone()],
        "facts, produced cell, then completion",
    );
    reject(
        "duplicate-fact-key",
        vec![
            fact_recorded(&response),
            fact_recorded(&response),
            fact_cell_produced_for_height(850000, attempt_id(91)),
            fact_attempt_completed(attempt_id(91)),
        ],
        vec![response, output],
        "fact keys must be unique within one settlement",
    );
}

#[test]
fn fact_recorded_requires_certified_node_allowlist() {
    let run_id = fact_run_id(97);
    let mut store = admitted_fact_store(&run_id, "run-start");
    append_fact_attempt_start(&mut store, &run_id, "fact-attempt-start", attempt_id(91));

    let response = fact_artifact_ref();
    let output = fact_output_artifact_ref_for_height(850000);
    let mut payloads = vec![
        fact_recorded(&response),
        fact_cell_produced_for_height(850000, attempt_id(91)),
        fact_attempt_completed(attempt_id(91)),
    ];
    store.certify_payloads_for_run(&run_id, &mut payloads);
    let request = typed_commit_request! {
        run_id: run_id.clone(),
        expected_next_seq: store.expected_next_seq(&run_id),
        commit_key: CommitKey::new("fact-missing-authority").expect("commit key"),
        payloads: payloads,
        required_artifacts: vec![response.clone(), output.clone()],
        preconditions: run_state_preconditions(RequiredRunState::Started),
    };
    let plan =
        test_prepared_commit_plan(request, vec![response.clone(), output.clone()]).expect("plan");
    let bundle = PreparedCommitBundle::new(
        plan,
        vec![
            PreparedArtifactBytes::new(fact_response_bytes(), response)
                .expect("fact response bytes"),
            PreparedArtifactBytes::new(fact_response_bytes(), output).expect("fact output bytes"),
        ],
        Vec::new(),
    )
    .expect("bundle");
    let error = store
        .append_test_commit_bundle(bundle)
        .expect_err("missing fact authority rejects");
    assert!(matches!(error, StoreError::ProjectionConflict { .. }));
}

#[test]
fn fact_recorded_rejects_descriptor_allowed_for_other_node() {
    let run_id = fact_run_id_for_node(98, node_id(99));
    let mut store = admitted_fact_store_with_node(&run_id, "run-start", node_id(99));
    append_fact_attempt_start(&mut store, &run_id, "fact-attempt-start", attempt_id(91));

    let response = fact_artifact_ref();
    let output = fact_output_artifact_ref_for_height(850000);
    let mut payloads = vec![
        fact_recorded(&response),
        fact_cell_produced_for_height(850000, attempt_id(91)),
        fact_attempt_completed(attempt_id(91)),
    ];
    store.certify_payloads_for_run(&run_id, &mut payloads);
    let mut preconditions = store.certified_preconditions(&run_id);
    preconditions.required_run_state = RequiredRunState::Started;
    let request = typed_commit_request! {
        run_id: run_id.clone(),
        expected_next_seq: store.expected_next_seq(&run_id),
        commit_key: CommitKey::new("fact-wrong-node-authority").expect("commit key"),
        payloads: payloads,
        required_artifacts: vec![response.clone(), output.clone()],
        preconditions: preconditions,
    };
    let plan =
        test_prepared_commit_plan(request, vec![response.clone(), output.clone()]).expect("plan");
    let bundle = PreparedCommitBundle::new(
        plan,
        vec![
            PreparedArtifactBytes::new(fact_response_bytes(), response)
                .expect("fact response bytes"),
            PreparedArtifactBytes::new(fact_response_bytes(), output).expect("fact output bytes"),
        ],
        Vec::new(),
    )
    .expect("bundle");
    let error = store
        .append_test_commit_bundle(bundle)
        .expect_err("wrong node authority rejects");
    assert!(matches!(error, StoreError::ProjectionConflict { .. }));
}

#[test]
fn retention_refs_are_projected_from_authoritative_stream() {
    let run_id = run_id(120);
    let artifact_id = artifact_id(121);
    let digest = content_digest(122);
    let mut store = admitted_store(&run_id, "run-start");
    let evidence = store_artifact_ref(artifact_id.clone(), digest.clone());
    append_run_state_commit(
        &mut store,
        &run_id,
        "retention-refs",
        vec![retention_refs_appended(
            artifact_id.clone(),
            digest.clone(),
            ArtifactRole::StateOutput,
        )],
        vec![evidence.clone()],
        RequiredRunState::Started,
    )
    .expect("append retention refs");

    let records = store.committed_records_for_projection_test(&run_id);
    let snapshot =
        ProjectionSnapshot::rebuild_from_run_stream(&records).expect("retention snapshot");
    let retention = snapshot.retention(&run_id).expect("retention projection");
    let evidence_hash = evidence.evidence_hash().expect("retention evidence hash");
    assert_eq!(
        retention
            .refs
            .get(&(artifact_id.clone(), evidence_hash.clone()))
            .expect("retention ref")
            .role,
        ArtifactRole::StateOutput
    );
    assert_eq!(
        retention
            .refs
            .get(&(artifact_id, evidence_hash))
            .expect("retention ref")
            .content_digest,
        digest
    );
}

#[test]
fn retention_refs_validate_role_contract_shape_without_repeating_exact_fields() {
    fn reject_state_output_shape(
        commit_key: &str,
        mut mutate: impl FnMut(&mut ArtifactEvidenceRef),
        expected_field: &'static str,
    ) {
        let run_id = run_id(120);
        let artifact_id = artifact_id(222);
        let digest = content_digest(223);
        let mut evidence = store_artifact_ref(artifact_id.clone(), digest.clone());
        mutate(&mut evidence);
        let run_start_key = format!("{commit_key}-run-start");
        let mut store = admitted_store(&run_id, &run_start_key);

        let mut retention_payload =
            retention_refs_appended(artifact_id, digest, ArtifactRole::StateOutput);
        let KernelEventPayload::RetentionRefsAppended(payload) = &mut retention_payload else {
            panic!("retention payload");
        };
        payload.refs[0].evidence_hash = evidence.evidence_hash().expect("retention evidence hash");

        let error = append_run_state_commit(
            &mut store,
            &run_id,
            commit_key,
            vec![retention_payload],
            vec![evidence],
            RequiredRunState::Started,
        )
        .expect_err("invalid retention evidence shape rejects");
        assert_invalid_prepared_commit_contains(error, expected_field);
    }

    reject_state_output_shape(
        "retention-state-output-missing-schema",
        |evidence| evidence.schema_id = None,
        "schema_id",
    );
    reject_state_output_shape(
        "retention-state-output-missing-semantic",
        |evidence| evidence.semantic_type_id = None,
        "semantic_type_id",
    );
    reject_state_output_shape(
        "retention-state-output-missing-node",
        |evidence| evidence.producer_node_id = None,
        "producer_node_id",
    );

    fn reject_manifest_shape(
        commit_key: &str,
        mut mutate: impl FnMut(&mut ArtifactEvidenceRef),
        expected_field: &'static str,
    ) {
        let run_id = run_id(120);
        let artifact_id = artifact_id(224);
        let digest = content_digest(225);
        let mut evidence = retention_manifest_artifact_ref(artifact_id.clone(), digest.clone());
        mutate(&mut evidence);
        let run_start_key = format!("{commit_key}-run-start");
        let mut store = admitted_store(&run_id, &run_start_key);

        let mut payloads = retention_manifest_commit_payloads(1, digest, None, artifact_id);
        let evidence_hash = evidence.evidence_hash().expect("manifest evidence hash");
        for payload in &mut payloads {
            match payload {
                KernelEventPayload::RetentionManifestProjected(projected) => {
                    projected.manifest_artifact_evidence_hash = evidence_hash.clone();
                }
                KernelEventPayload::RetentionRefsAppended(refs) => {
                    refs.refs[0].evidence_hash = evidence_hash.clone();
                }
                _ => {}
            }
        }

        let error = append_run_state_commit(
            &mut store,
            &run_id,
            commit_key,
            payloads,
            vec![evidence],
            RequiredRunState::Started,
        )
        .expect_err("invalid retention manifest evidence shape rejects");
        assert_invalid_prepared_commit_contains(error, expected_field);
    }

    reject_manifest_shape(
        "retention-manifest-schema-present",
        |evidence| evidence.schema_id = Some(schema_id("mfm.test.retention_manifest", 224)),
        "schema_id",
    );
    reject_manifest_shape(
        "retention-manifest-producer-present",
        |evidence| evidence.producer_node_id = Some(node_id(224)),
        "producer_node_id",
    );
}

#[test]
fn retention_manifest_projection_must_chain_append_only() {
    let run_id = run_id(120);
    let first_artifact = artifact_id(123);
    let first_digest = content_digest(124);
    let second_artifact = artifact_id(125);
    let second_digest = content_digest(126);
    let mut store = admitted_store(&run_id, "run-start");
    let first_evidence =
        retention_manifest_artifact_ref(first_artifact.clone(), first_digest.clone());
    let second_evidence =
        retention_manifest_artifact_ref(second_artifact.clone(), second_digest.clone());

    let skipped_first = append_run_state_commit(
        &mut store,
        &run_id,
        "retention-manifest-skipped-first",
        retention_manifest_commit_payloads(2, first_digest.clone(), None, first_artifact.clone()),
        vec![first_evidence.clone()],
        RequiredRunState::Started,
    )
    .expect_err("first manifest must be seq 1");
    assert!(matches!(
        skipped_first,
        StoreError::ProjectionConflict { .. }
    ));

    append_run_state_commit(
        &mut store,
        &run_id,
        "retention-manifest-1",
        retention_manifest_commit_payloads(1, first_digest.clone(), None, first_artifact),
        vec![first_evidence],
        RequiredRunState::Started,
    )
    .expect("append first manifest");

    let wrong_previous = append_run_state_commit(
        &mut store,
        &run_id,
        "retention-manifest-wrong-prev",
        retention_manifest_commit_payloads(
            2,
            second_digest.clone(),
            Some(content_digest(128)),
            second_artifact.clone(),
        ),
        vec![second_evidence.clone()],
        RequiredRunState::Started,
    )
    .expect_err("wrong previous digest rejects");
    assert!(matches!(
        wrong_previous,
        StoreError::ProjectionConflict { .. }
    ));

    append_run_state_commit(
        &mut store,
        &run_id,
        "retention-manifest-2",
        retention_manifest_commit_payloads(
            2,
            second_digest.clone(),
            Some(first_digest.clone()),
            second_artifact,
        ),
        vec![second_evidence],
        RequiredRunState::Started,
    )
    .expect("append second manifest");

    let retention = store
        .projection_snapshot()
        .retention(&run_id)
        .expect("retention projection");
    assert_eq!(retention.manifests.len(), 2);
    assert_eq!(
        retention.manifest.as_ref().expect("latest").manifest_digest,
        second_digest
    );
    assert_eq!(
        retention
            .manifest
            .as_ref()
            .expect("latest")
            .previous_manifest_digest,
        Some(first_digest)
    );
    assert!(retention.manifests.values().any(|manifest| {
        manifest.manifest_artifact_id == artifact_id(125)
            && manifest.manifest_digest == second_digest
    }));
}

#[test]
fn retention_manifest_projection_requires_same_commit_retention_ref() {
    let run_id = run_id(120);
    let artifact_id = artifact_id(129);
    let digest = content_digest(130);
    let mut store = admitted_store(&run_id, "run-start");
    let evidence = retention_manifest_artifact_ref(artifact_id.clone(), digest.clone());

    let missing_ref = append_run_state_commit(
        &mut store,
        &run_id,
        "retention-manifest-missing-ref",
        vec![retention_manifest_projected(1, digest, None, artifact_id)],
        vec![evidence],
        RequiredRunState::Started,
    )
    .expect_err("manifest without retention ref rejects");
    assert!(matches!(missing_ref, StoreError::ProjectionConflict { .. }));
}

#[test]
fn projections_rebuild_from_authoritative_run_stream() {
    let run_id = run_id(120);
    let output_artifact_id = artifact_id(61);
    let output_digest = content_digest(62);
    let manifest_artifact_id = artifact_id(65);
    let manifest_digest = content_digest(66);
    let rendered_artifact_id = artifact_id(67);
    let rendered_digest = content_digest(68);
    let evidence = store_artifact_ref(output_artifact_id.clone(), output_digest.clone());
    let manifest_evidence =
        retention_manifest_artifact_ref(manifest_artifact_id.clone(), manifest_digest.clone());
    let rendered_evidence =
        public_output_artifact_ref(rendered_artifact_id.clone(), rendered_digest.clone());
    let mut store = admitted_store(&run_id, "run-start");
    append_run_state_commit(
        &mut store,
        &run_id,
        "attempt-start",
        vec![state_attempt_started()],
        Vec::new(),
        RequiredRunState::NotCompleted,
    )
    .expect("append attempt start");
    append_side_effect_prepare_for_ledger(
        &mut store,
        &run_id,
        "projection-sidefx",
        side_effect_ledger_key_with_suffix(67),
        resource_key("account-1", 68),
        69,
        true,
    );
    let mut terminal_payloads =
        terminal_cell_commit_payloads(output_artifact_id.clone(), output_digest.clone());
    terminal_payloads.push(public_output_produced_with_rendered_artifact(
        output_artifact_id.clone(),
        output_digest.clone(),
        rendered_artifact_id,
        rendered_digest,
    ));
    append_run_state_commit(
        &mut store,
        &run_id,
        "cell-produced",
        terminal_payloads,
        vec![evidence, rendered_evidence],
        RequiredRunState::NotCompleted,
    )
    .expect("append cell produced");
    append_run_state_commit(
        &mut store,
        &run_id,
        "retention-manifest",
        retention_manifest_commit_payloads(1, manifest_digest, None, manifest_artifact_id),
        vec![manifest_evidence],
        RequiredRunState::NotCompleted,
    )
    .expect("append retention manifest");

    let records = store.committed_records_for_projection_test(&run_id);
    let summary = projection_differential_summary(&store, &run_id, &records);
    let expected_summary = [
        "committed run_state=Started commits=6 events=12 next_seq=7".to_owned(),
        format!(
            "side_effect pair={} phase=invocation_prepared prepared=true resource_key=true touched_set=false",
            side_effect_pair_id()
        ),
        format!(
            "resource_lane mfm.test.account_nonce:account-1 holder={} phase_epoch=1",
            side_effect_pair_id()
        ),
        format!(
            "public_output run={} schema={} rendered_artifact=true",
            run_id,
            schema_id("mfm.test.public_output", 3)
        ),
        format!("retention run={} refs=3 manifests=1 latest_seq=1", run_id),
    ]
    .join("\n");
    assert_eq!(summary, expected_summary);

    let rebuilt =
        ProjectionSnapshot::rebuild_from_run_stream(&records).expect("rebuild projections");
    assert_eq!(rebuilt.side_effects().count(), 1);
    assert_eq!(rebuilt.resource_lanes().count(), 1);
    assert_eq!(rebuilt.public_outputs().count(), 1);
    assert_eq!(rebuilt.retentions().count(), 1);
    assert!(matches!(
        rebuilt.cell_terminal(&cell_id(21)),
        Some(CellTerminalProjection::Produced { .. })
    ));
    assert_projection_codecs_round_trip(&rebuilt);
}
