use super::*;

#[test]
fn side_effect_ledger_purpose_is_required_and_closed() {
    let payload = side_effect_claim();
    let mut missing = payload_json_value(&payload);
    missing
        .as_object_mut()
        .expect("payload object")
        .remove("ledger_purpose");
    assert!(matches!(
        payload_from_json_value(&missing),
        Err(StoreError::Event(message)) if message.contains("ledger_purpose")
    ));

    let mut unknown = payload_json_value(&payload);
    unknown
        .get_mut("ledger_purpose")
        .and_then(serde_json::Value::as_object_mut)
        .expect("ledger purpose object")
        .insert(
            "kind".to_owned(),
            serde_json::Value::String("other".to_owned()),
        );
    assert!(matches!(
        payload_from_json_value(&unknown),
        Err(StoreError::Identity(message))
            if message.contains("unknown side-effect ledger purpose")
    ));
}

#[test]
fn side_effect_ledger_purpose_cannot_change_after_intent() {
    let run_id = run_id(92);
    let artifact_id = artifact_id(93);
    let artifact_digest = content_digest(94);
    let mut store = admitted_store(&run_id, "purpose-run-start");
    append_side_effect_attempt_started(&mut store, &run_id, "purpose-attempt-start")
        .expect("append sidefx attempt start");
    append_certified_side_effect_commit(
        &mut store,
        &run_id,
        "purpose-intent",
        vec![side_effect_intent(
            artifact_id.clone(),
            artifact_digest.clone(),
        )],
        vec![intent_artifact_ref(artifact_id, artifact_digest)],
    )
    .expect("append intent");

    let mut changed = side_effect_claim();
    let KernelEventPayload::SideEffectClaimed(payload) = &mut changed else {
        unreachable!("helper returns claimed payload");
    };
    payload.ledger_purpose = events::SideEffectLedgerPurpose::Remediation {
        forward_pair_id: side_effect_pair_id(),
    };
    let error = append_certified_side_effect_commit(
        &mut store,
        &run_id,
        "purpose-changed-claim",
        vec![changed],
        Vec::new(),
    )
    .expect_err("ledger purpose change rejects");
    assert!(matches!(
        error,
        StoreError::ProjectionConflict { message, .. }
            if message.contains("ledger purpose changed")
    ));
}

#[test]
fn forward_fence_rejects_boundary_events_after_engagement() {
    fn started_store() -> (RunId, StoreContractRunStore) {
        let run_id = run_id(120);
        let mut store = admitted_store(&run_id, "fence-run-start");
        append_side_effect_attempt_started(&mut store, &run_id, "fence-sidefx-attempt-start")
            .expect("append sidefx attempt start");
        (run_id, store)
    }

    fn intent_with_ref(byte: u8) -> (KernelEventPayload, ArtifactEvidenceRef) {
        let artifact_id = artifact_id(byte);
        let digest = content_digest(byte + 1);
        (
            side_effect_intent(artifact_id.clone(), digest.clone()),
            intent_artifact_ref(artifact_id, digest),
        )
    }

    fn append_before_engagement(
        store: &mut StoreContractRunStore,
        run_id: &RunId,
        commit_key: &str,
        artifact_byte: u8,
        extra_payloads: Vec<KernelEventPayload>,
    ) {
        let (intent, evidence) = intent_with_ref(artifact_byte);
        let mut payloads = vec![intent];
        payloads.extend(extra_payloads);
        append_certified_side_effect_commit(store, run_id, commit_key, payloads, vec![evidence])
            .expect("append before engagement");
    }

    fn reject_after_engagement(
        store: &mut StoreContractRunStore,
        run_id: &RunId,
        engagement_key: &str,
        payloads: Vec<KernelEventPayload>,
    ) {
        append_generic_nonretryable_failure(store, run_id, engagement_key);
        let commit_key = format!("{engagement_key}-reject");
        assert_certified_side_effect_projection_conflict(
            store,
            run_id,
            &commit_key,
            payloads,
            Vec::new(),
            "forward side-effect boundary",
        );
    }

    let (run_id, mut store) = started_store();
    let (intent, intent_ref) = intent_with_ref(130);
    append_generic_nonretryable_failure(&mut store, &run_id, "fence-intent");
    assert_certified_side_effect_projection_conflict(
        &mut store,
        &run_id,
        "fence-intent-reject",
        vec![intent],
        vec![intent_ref],
        "forward side-effect boundary",
    );

    let (run_id, mut store) = started_store();
    append_before_engagement(&mut store, &run_id, "fence-claim-intent", 132, Vec::new());
    reject_after_engagement(
        &mut store,
        &run_id,
        "fence-claim",
        vec![side_effect_claim()],
    );

    let (run_id, mut store) = started_store();
    append_before_engagement(
        &mut store,
        &run_id,
        "fence-takeover-intent-claim",
        134,
        vec![side_effect_claim()],
    );
    reject_after_engagement(
        &mut store,
        &run_id,
        "fence-takeover",
        vec![side_effect_claim_taken_over()],
    );

    let (run_id, mut store) = started_store();
    append_before_engagement(
        &mut store,
        &run_id,
        "fence-prepared-intent-claim",
        136,
        vec![side_effect_claim()],
    );
    reject_after_engagement(
        &mut store,
        &run_id,
        "fence-prepared",
        vec![side_effect_prepared(1, "token-1")],
    );

    let (run_id, mut store) = started_store();
    append_before_engagement(
        &mut store,
        &run_id,
        "fence-started-prepare",
        138,
        vec![side_effect_claim(), side_effect_prepared(1, "token-1")],
    );
    reject_after_engagement(
        &mut store,
        &run_id,
        "fence-started",
        vec![side_effect_started("owner-1", 1, "token-1")],
    );
}

#[test]
fn remediation_intent_requires_engaged_confirmed_forward_and_unique_link() {
    let policy = compensate_saga_policy();
    let run_id = run_id_with_saga_policy(120, &policy);
    let mut store = admitted_store_with_saga_policy(&run_id, "remediation-run-start", &policy);
    append_forward_confirmation(&mut store, &run_id);
    append_remediation_attempts_started(&mut store, &run_id, "remediation-attempts-started");

    let mut remediation_intent = side_effect_intent(artifact_id(140), content_digest(141));
    set_remediation_purpose(&mut remediation_intent, remediation_ledger_key(1));
    assert_certified_side_effect_projection_conflict(
        &mut store,
        &run_id,
        "remediation-before-engagement",
        vec![remediation_intent],
        vec![remediation_intent_artifact_ref(
            artifact_id(140),
            content_digest(141),
        )],
        "prior saga engagement",
    );

    append_generic_nonretryable_failure(&mut store, &run_id, "remediation-engagement");
    let mut wrong_pair = side_effect_intent(artifact_id(148), content_digest(149));
    set_remediation_purpose(&mut wrong_pair, remediation_ledger_key(11));
    let KernelEventPayload::SideEffectIntentPersisted(payload) = &mut wrong_pair else {
        unreachable!("helper returns side-effect intent");
    };
    payload.ledger_purpose = events::SideEffectLedgerPurpose::Remediation {
        forward_pair_id: remediation_pair_id(),
    };
    assert_certified_side_effect_projection_conflict(
        &mut store,
        &run_id,
        "remediation-wrong-pair",
        vec![wrong_pair],
        vec![remediation_intent_artifact_ref(
            artifact_id(148),
            content_digest(149),
        )],
        "forward pair",
    );

    let mut remediation_intent = side_effect_intent(artifact_id(142), content_digest(143));
    set_remediation_purpose(&mut remediation_intent, remediation_ledger_key(1));
    append_certified_side_effect_commit(
        &mut store,
        &run_id,
        "remediation-admitted",
        vec![remediation_intent],
        vec![remediation_intent_artifact_ref(
            artifact_id(142),
            content_digest(143),
        )],
    )
    .expect("confirmed forward remediation is admitted after engagement");

    let mut duplicate = side_effect_intent(artifact_id(144), content_digest(145));
    set_remediation_purpose(&mut duplicate, remediation_ledger_key(2));
    let error = append_certified_side_effect_commit(
        &mut store,
        &run_id,
        "remediation-duplicate",
        vec![duplicate],
        vec![remediation_intent_artifact_ref(
            artifact_id(144),
            content_digest(145),
        )],
    )
    .expect_err("duplicate remediation rejects");
    match error {
        StoreError::LogicalKeyConflict { logical_key }
            if logical_key.as_str().contains("sidefx:remediation") => {}
        StoreError::ProjectionConflict { message, .. }
            if message.contains("already exists for forward pair") => {}
        other => panic!("unexpected duplicate remediation error: {other:?}"),
    }

    let unconfirmed_run_id = run_id_with_saga_policy(121, &policy);
    let mut unconfirmed =
        admitted_store_with_saga_policy(&unconfirmed_run_id, "unconfirmed-run-start", &policy);
    append_side_effect_prepare(&mut unconfirmed, &unconfirmed_run_id);
    append_generic_nonretryable_failure(
        &mut unconfirmed,
        &unconfirmed_run_id,
        "unconfirmed-engagement",
    );
    let mut remediation_intent = side_effect_intent(artifact_id(146), content_digest(147));
    set_remediation_purpose(&mut remediation_intent, remediation_ledger_key(3));
    append_remediation_attempts_started(
        &mut unconfirmed,
        &unconfirmed_run_id,
        "unconfirmed-remediation-attempts-started",
    );
    assert_certified_side_effect_projection_conflict(
        &mut unconfirmed,
        &unconfirmed_run_id,
        "remediation-unconfirmed",
        vec![remediation_intent],
        vec![remediation_intent_artifact_ref(
            artifact_id(146),
            content_digest(147),
        )],
        "requires terminal forward ledger",
    );
}

#[test]
fn remediation_intent_rejects_saga_token_from_same_policy_different_spec_hash() {
    let policy = compensate_saga_policy();
    let run_id = run_id_with_saga_policy(222, &policy);
    let mut store =
        admitted_store_with_saga_policy(&run_id, "remediation-spec-authority-run-start", &policy);
    append_forward_confirmation(&mut store, &run_id);
    append_generic_nonretryable_failure(&mut store, &run_id, "remediation-spec-authority");
    append_remediation_attempts_started(
        &mut store,
        &run_id,
        "remediation-spec-authority-attempts-started",
    );

    let alternate_spec =
        saga_authority_spec_with_authoring_config_hash(policy.clone(), content_digest(188));
    let alternate_spec_hash = alternate_spec
        .spec_hash()
        .expect("alternate saga authority spec hash");
    assert_ne!(
        store
            .projection_snapshot()
            .run_spec_hash(&run_id)
            .expect("run-start spec hash"),
        &alternate_spec_hash
    );

    let mut remediation_intent = side_effect_intent(artifact_id(188), content_digest(189));
    set_remediation_purpose(&mut remediation_intent, remediation_ledger_key(18));
    let KernelEventPayload::SideEffectIntentPersisted(payload) = &mut remediation_intent else {
        unreachable!("helper returns side-effect intent");
    };
    payload.spec_hash = alternate_spec_hash;
    payload.ledger_purpose = events::SideEffectLedgerPurpose::Remediation {
        forward_pair_id: side_effect_pair_id(),
    };

    let error = store
        .append_prepared_commit(typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(&run_id),
            commit_key: CommitKey::new("remediation-spec-authority-reject").expect("commit key"),
            payloads: vec![remediation_intent],
            required_artifacts: vec![remediation_intent_artifact_ref(
                artifact_id(188),
                content_digest(189),
            )],
            preconditions: CommitPreconditions {
                certified_run_authority: Some(
                    CertifiedRunStoreAuthority::from_spec(run_id.clone(), &alternate_spec)
                        .expect("alternate certified run authority"),
                ),
                ..CommitPreconditions::default()
            },
        })
        .expect_err("alternate-spec saga token rejects");
    assert_projection_conflict_contains(error, "spec hash does not match run start");
}
