use super::*;

#[test]
fn fact_recorded_projects_reusable_fact_evidence() {
    let run_id = fact_run_id(95);
    let mut store = admitted_fact_store(&run_id, "run-start");
    let response = fact_artifact_ref();
    let mut attempt_payloads = vec![fact_attempt_started()];
    store.certify_payloads_for_run(&run_id, &mut attempt_payloads);
    append_run_state_commit(
        &mut store,
        &run_id,
        "fact-attempt-start",
        attempt_payloads,
        Vec::new(),
        RequiredRunState::Started,
    )
    .expect("append fact attempt start");
    append_fact_recorded_commit(&mut store, &run_id, "fact-recorded").expect("append fact");

    let snapshot = store.projection_snapshot();
    assert_eq!(snapshot.fact_descriptors().count(), 1);
    assert_eq!(snapshot.fact_records().count(), 1);
    assert_eq!(snapshot.fact_index_entries().count(), 1);
    assert_eq!(snapshot.fact_term_entries().count(), 2);
    let projection = snapshot
        .fact_index_entries()
        .next()
        .map(|(_, projection)| projection)
        .expect("fact index projection");
    assert_eq!(projection.artifact_id, response.artifact_id);
    assert_eq!(projection.response_hash, response.digest);
    assert_eq!(projection.fact_key, fact_key());
}

#[test]
fn fact_record_projection_constructor_derives_event_coordinates() {
    let run_id = fact_run_id(96);
    let mut store = admitted_fact_store(&run_id, "constructor-run-start");
    let response = fact_artifact_ref();
    let mut attempt_payloads = vec![fact_attempt_started()];
    store.certify_payloads_for_run(&run_id, &mut attempt_payloads);
    append_run_state_commit(
        &mut store,
        &run_id,
        "constructor-attempt-start",
        attempt_payloads,
        Vec::new(),
        RequiredRunState::Started,
    )
    .expect("append fact attempt start");
    let outcome = append_fact_recorded_commit(&mut store, &run_id, "constructor-fact-recorded")
        .expect("append fact");
    let CommitOutcome::Appended(batch) = outcome else {
        panic!("fact append should be new");
    };
    assert_eq!(batch.events().len(), 1);
    let event = &batch.events()[0];
    let KernelEventPayload::FactRecorded(payload) = event.payload() else {
        panic!("fact event payload");
    };

    let with_evidence = mfm_store::v1::FactRecordProjection::from_recorded_event(
        event,
        payload,
        Some(response.clone()),
    )
    .expect("fact record projection");
    assert_eq!(
        with_evidence.fact_claim_id,
        mfm_facts::derive_fact_claim_id(
            event.run_id().clone(),
            event.seq().as_u64(),
            event.ordinal().as_u32(),
        )
        .expect("claim id")
    );
    assert_eq!(&with_evidence.source_event_id, event.event_id());
    assert_eq!(&with_evidence.source_run_id, event.run_id());
    assert_eq!(with_evidence.source_seq, event.seq().as_u64());
    assert_eq!(with_evidence.source_ordinal, event.ordinal().as_u32());
    assert_eq!(&with_evidence.node_id, &payload.node_id);
    assert_eq!(&with_evidence.attempt_id, &payload.attempt_id);
    assert_eq!(with_evidence.response_artifact_evidence, Some(response));
    assert_eq!(&with_evidence.claim, &payload.claim);
    assert_eq!(
        store
            .projection_snapshot()
            .fact_record(&with_evidence.fact_claim_id),
        Some(&with_evidence)
    );

    let without_evidence =
        mfm_store::v1::FactRecordProjection::from_recorded_event(event, payload, None)
            .expect("fact record projection without retained evidence");
    let mut expected_without_evidence = with_evidence;
    expected_without_evidence.response_artifact_evidence = None;
    assert_eq!(without_evidence, expected_without_evidence);

    let index = mfm_store::v1::FactIndexProjection::from_record_projection(
        &without_evidence,
        event.commit_key().clone(),
        event.store_commit_order().as_u64(),
        "2026-01-02T03:04:06Z",
    )
    .expect("index constructor")
    .expect("indexed fact projection");
    assert!(without_evidence.matches_index_projection(&index));
    assert_eq!(index.fact_claim_id, without_evidence.fact_claim_id);
    assert_eq!(
        index.store_commit_order,
        event.store_commit_order().as_u64()
    );
    assert_eq!(index.recorded_at, "2026-01-02T03:04:06Z");

    let mut private_record = without_evidence.clone();
    private_record.claim = mfm_facts::FactClaim::new(mfm_facts::FactClaimParts {
        visibility: mfm_facts::FactVisibility::RunPrivate,
        fact_kind: without_evidence.claim.fact_kind().clone(),
        fact_descriptor_hash: without_evidence.claim.fact_descriptor_hash().clone(),
        subject: without_evidence.claim.subject().clone(),
        observed_at: without_evidence.claim.observed_at().map(str::to_owned),
        request: without_evidence.claim.request().cloned(),
        response: without_evidence.claim.response().clone(),
        producer: without_evidence.claim.producer().clone(),
    })
    .expect("private claim");
    assert!(mfm_store::v1::FactIndexProjection::from_record_projection(
        &private_record,
        event.commit_key().clone(),
        event.seq().as_u64(),
        "2026-01-02T03:04:06Z",
    )
    .expect("private constructor")
    .is_none());
    assert!(mfm_store::v1::FactIndexProjection::from_record_projection(
        &without_evidence,
        event.commit_key().clone(),
        event.seq().as_u64(),
        "",
    )
    .is_err());
}

#[test]
fn fact_recorded_same_subject_claims_are_claim_id_distinct() {
    fn single_event(outcome: &CommitOutcome) -> &KernelEventEnvelope {
        let batch = match outcome {
            CommitOutcome::Appended(batch) | CommitOutcome::Idempotent(batch) => batch,
            CommitOutcome::AdmissionBlocked(_) => panic!("fact append must not admission-block"),
            CommitOutcome::ExecutionClaimBusy(_) => {
                panic!("fact append must not hit execution claim admission")
            }
        };
        assert_eq!(batch.events().len(), 1);
        &batch.events()[0]
    }

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

    let run_id = fact_run_id(98);
    let mut store = admitted_fact_store(&run_id, "same-subject-run-start");
    let mut attempt_payloads = vec![fact_attempt_started()];
    store.certify_payloads_for_run(&run_id, &mut attempt_payloads);
    append_run_state_commit(
        &mut store,
        &run_id,
        "same-subject-attempt-start",
        attempt_payloads,
        Vec::new(),
        RequiredRunState::Started,
    )
    .expect("append fact attempt start");

    let first =
        append_fact_recorded_commit_for_height(&mut store, &run_id, "same-subject-1", 850000)
            .expect("append first fact");
    let second =
        append_fact_recorded_commit_for_height(&mut store, &run_id, "same-subject-2", 850001)
            .expect("append second same-subject fact");

    let first_event = single_event(&first);
    let second_event = single_event(&second);
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
    assert_eq!(snapshot.fact_records().count(), 2);
    assert_eq!(snapshot.fact_index_entries().count(), 2);
    let claim_ids = snapshot
        .fact_index_entries()
        .map(|(claim_id, projection)| {
            assert_eq!(projection.fact_key, fact_key());
            assert_eq!(claim_id, &projection.fact_claim_id);
            claim_id.clone()
        })
        .collect::<Vec<_>>();
    assert_eq!(claim_ids.len(), 2);
    assert_ne!(claim_ids[0], claim_ids[1]);
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
fn fact_recorded_requires_certified_node_allowlist() {
    let run_id = fact_run_id(97);
    let mut store = admitted_fact_store(&run_id, "run-start");
    let mut attempt_payloads = vec![fact_attempt_started()];
    store.certify_payloads_for_run(&run_id, &mut attempt_payloads);
    append_run_state_commit(
        &mut store,
        &run_id,
        "fact-attempt-start",
        attempt_payloads,
        Vec::new(),
        RequiredRunState::Started,
    )
    .expect("append fact attempt start");

    let response = fact_artifact_ref();
    let mut payloads = vec![fact_recorded(&response)];
    store.certify_payloads_for_run(&run_id, &mut payloads);
    let request = typed_commit_request! {
        run_id: run_id.clone(),
        expected_next_seq: store.expected_next_seq(&run_id),
        commit_key: CommitKey::new("fact-missing-authority").expect("commit key"),
        payloads: payloads,
        required_artifacts: vec![response.clone()],
        preconditions: run_state_preconditions(RequiredRunState::Started),
    };
    let plan = test_prepared_commit_plan(request, vec![response.clone()]).expect("plan");
    let bundle = PreparedCommitBundle::new(
        plan,
        vec![PreparedArtifactBytes::new(fact_response_bytes(), response)
            .expect("fact response bytes")],
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
    let mut attempt_payloads = vec![fact_attempt_started()];
    store.certify_payloads_for_run(&run_id, &mut attempt_payloads);
    append_run_state_commit(
        &mut store,
        &run_id,
        "fact-attempt-start",
        attempt_payloads,
        Vec::new(),
        RequiredRunState::Started,
    )
    .expect("append fact attempt start");

    let response = fact_artifact_ref();
    let mut payloads = vec![fact_recorded(&response)];
    store.certify_payloads_for_run(&run_id, &mut payloads);
    let mut preconditions = store.certified_preconditions(&run_id);
    preconditions.required_run_state = RequiredRunState::Started;
    let request = typed_commit_request! {
        run_id: run_id.clone(),
        expected_next_seq: store.expected_next_seq(&run_id),
        commit_key: CommitKey::new("fact-wrong-node-authority").expect("commit key"),
        payloads: payloads,
        required_artifacts: vec![response.clone()],
        preconditions: preconditions,
    };
    let plan = test_prepared_commit_plan(request, vec![response.clone()]).expect("plan");
    let bundle = PreparedCommitBundle::new(
        plan,
        vec![PreparedArtifactBytes::new(fact_response_bytes(), response)
            .expect("fact response bytes")],
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

    let snapshot = ProjectionSnapshot::rebuild_from_run_stream(&store.load_run_stream(&run_id))
        .expect("retention snapshot");
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

    let stream = store.load_run_stream(&run_id);
    let summary = projection_differential_summary(&store, &run_id, &stream);
    let expected_summary = [
        "committed run_state=Started commits=6 events=12 next_seq=7".to_owned(),
        format!(
            "side_effect pair={} phase=invocation_prepared prepared=false resource_key=true touched_set=false",
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
        ProjectionSnapshot::rebuild_from_run_stream(&stream).expect("rebuild projections");
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
