use super::*;

#[test]
fn artifact_role_contract_store_codec_roundtrips_current_tags() {
    let rows = artifact_role_tag_baselines()
        .iter()
        .map(|(role, tag)| {
            assert_eq!(role.as_str(), *tag);
            assert_eq!(
                ArtifactRole::parse(tag).expect("parse artifact role"),
                *role
            );
            format!("{tag} -> {role:?}")
        })
        .collect::<Vec<_>>()
        .join("\n");

    assert_eq!(
        rows,
        "typed_execution_spec -> TypedExecutionSpec\n\
typed_spec_certificate -> TypedSpecCertificate\n\
typed_config -> TypedConfig\n\
seed_input -> SeedInput\n\
state_output -> StateOutput\n\
fact_response -> FactResponse\n\
fact_query_evidence -> FactQueryEvidence\n\
side_effect_intent -> SideEffectIntent\n\
prepared_invocation -> PreparedInvocation\n\
not_submitted_proof -> NotSubmittedProof\n\
submission -> Submission\n\
submission_unknown_evidence -> SubmissionUnknownEvidence\n\
receipt -> Receipt\n\
confirmation -> Confirmation\n\
ambiguity_evidence -> AmbiguityEvidence\n\
manual_resolution_evidence -> ManualResolutionEvidence\n\
manual_resolution_authorization -> ManualResolutionAuthorization\n\
public_output -> PublicOutput\n\
redacted_diagnostic -> RedactedDiagnostic\n\
retention_manifest -> RetentionManifest"
    );

    assert_eq!(ArtifactRole::parse("resource_touched_set_evidence"), None);
}

#[test]
fn event_artifact_requirements_mark_filterable_sources() {
    let cell_requirements =
        cell_produced(artifact_id(31), content_digest(32)).artifact_requirements();
    assert_eq!(cell_requirements.len(), 1);
    assert_eq!(
        cell_requirements[0].source,
        EventArtifactReferenceSource::StateOutput
    );
    assert!(cell_requirements[0]
        .source
        .is_terminal_lifecycle_receipt_candidate());
    assert_eq!(
        cell_requirements[0].artifact_role,
        Some(ArtifactRole::StateOutput)
    );

    let public_requirements =
        public_output_produced(artifact_id(33), content_digest(34)).artifact_requirements();
    assert_eq!(public_requirements.len(), 1);
    assert_eq!(
        public_requirements[0].source,
        EventArtifactReferenceSource::PublicOutputCell
    );
    assert!(!public_requirements[0]
        .source
        .is_terminal_lifecycle_receipt_candidate());

    let retention_requirements = retention_refs_appended(
        artifact_id(35),
        content_digest(36),
        ArtifactRole::FactResponse,
    )
    .artifact_requirements();
    assert_eq!(retention_requirements.len(), 1);
    assert_eq!(
        retention_requirements[0].source,
        EventArtifactReferenceSource::RetentionRef
    );
    assert!(retention_requirements[0].source.is_retention());

    let mut failure = side_effect_failed(false);
    let KernelEventPayload::SideEffectFailed(payload) = &mut failure else {
        panic!("side-effect failure payload");
    };
    payload.error.diagnostic_ref = Some(event_artifact_ref(artifact_id(37), content_digest(38)));
    let failure_requirements = failure.artifact_requirements();
    assert_eq!(failure_requirements.len(), 1);
    assert_eq!(
        failure_requirements[0].source,
        EventArtifactReferenceSource::SideEffectFailureDiagnostic
    );
    assert!(!failure_requirements[0].source.is_retention());
    assert!(!failure_requirements[0]
        .source
        .is_terminal_lifecycle_receipt_candidate());
}

#[test]
fn fact_recorded_protocol_baselines_cover_codec_requirements_and_projection() {
    let response = fact_artifact_ref();
    let payload = fact_recorded(&response);
    let canonical = payload_canonical_json(&payload).expect("fact payload json");
    assert!(canonical.as_str().contains(r#""subject_material":"#));
    let decoded_json: serde_json::Value =
        serde_json::from_str(canonical.as_str()).expect("payload json");
    assert_eq!(
        payload_from_json_value(&decoded_json).expect("payload roundtrip"),
        payload
    );

    let requirements = payload.artifact_requirements();
    assert_eq!(requirements.len(), 1);
    let requirement = &requirements[0];
    assert_eq!(
        requirement.source,
        EventArtifactReferenceSource::FactResponse
    );
    assert_eq!(requirement.artifact_role, Some(ArtifactRole::FactResponse));
    assert_eq!(requirement.artifact_id, response.artifact_id);
    assert_eq!(requirement.digest, Some(response.digest.clone()));
    assert_eq!(requirement.byte_len, None);
    assert_eq!(requirement.media_type, None);
    assert_eq!(
        requirement.schema_id,
        Some(schema_id("mfm.test.fact_response", 96))
    );
    assert_eq!(requirement.semantic_type_id, None);
    assert_eq!(requirement.producer_node_id, Some(node_id(90)));
    assert_eq!(requirement.producer_seed_id, None);

    let run_id = fact_run_id(250);
    let mut store = admitted_fact_store(&run_id, "fact-baseline-run-start");
    let mut attempt_payloads = vec![fact_attempt_started()];
    store.certify_payloads_for_run(&run_id, &mut attempt_payloads);
    append_run_state_commit(
        &mut store,
        &run_id,
        "fact-baseline-attempt-start",
        attempt_payloads,
        Vec::new(),
        RequiredRunState::NotCompleted,
    )
    .expect("append fact attempt start");
    append_fact_recorded_commit(&mut store, &run_id, "fact-baseline-recorded")
        .expect("append fact recorded");
    let stream = store.load_run_stream(&run_id);
    assert!(ProjectionSnapshot::rebuild_from_run_stream(&stream).is_err());
    let record = store
        .projection_snapshot()
        .fact_records()
        .next()
        .map(|(_, projection)| projection)
        .expect("fact record projection");
    assert_eq!(record.node_id, node_id(90));
    assert_eq!(record.attempt_id, attempt_id(91));
    let projection = store
        .projection_snapshot()
        .fact_index_entries()
        .next()
        .map(|(_, projection)| projection)
        .expect("fact index projection");
    assert_eq!(projection.fact_key, fact_key());
    assert_eq!(
        projection.request_schema_id,
        Some(schema_id("mfm.test.fact_request", 94))
    );
    assert_eq!(projection.request_hash, Some(content_digest(95)));
    assert_eq!(
        projection.response_schema_id,
        schema_id("mfm.test.fact_response", 96)
    );
    assert_eq!(projection.response_hash, response.digest);
    assert_eq!(projection.artifact_id, response.artifact_id);
    assert_eq!(projection.capability_kind, capability_kind(92));
    assert_eq!(
        projection.capability_version,
        CapabilityVersion::new("mfm.test.fact.v1").expect("capability version")
    );
    assert_eq!(projection.adapter_kind, adapter_kind(93));
    assert_eq!(
        projection.adapter_version,
        AdapterVersion::new("mfm.test.adapter.v1").expect("adapter version")
    );
}

#[test]
fn committed_run_stream_exposes_store_owned_authority() {
    let run_id = run_id(141);
    let mut store = admitted_store(&run_id, "committed-stream-run-start");
    append_side_effect_prepare(&mut store, &run_id);

    let stream = store.load_run_stream(&run_id);
    let committed =
        CommittedRunStream::from_events(run_id.clone(), stream.clone()).expect("committed stream");

    assert_eq!(committed.run_id(), &run_id);
    assert_eq!(committed.events(), stream.as_slice());
    assert_eq!(committed.next_seq(), store.expected_next_seq(&run_id));
    assert_eq!(committed.commits().len(), 3);
    assert_eq!(committed.commits()[0].seq(), StreamSeq::FIRST);
    assert_eq!(committed.commits()[0].events().len(), 1);
    assert_eq!(committed.commits()[1].events().len(), 1);
    assert_eq!(committed.commits()[2].events().len(), 3);
    assert_eq!(
        committed.commits()[2].commit_key().as_str(),
        "sidefx-prepare"
    );
    assert_eq!(committed.projection().run_state(&run_id), RunState::Started);
    assert!(committed.projection().saga_engagement(&run_id).is_none());
    assert_eq!(
        committed
            .projection()
            .side_effect_for_pair(&run_id, &side_effect_pair_id())
            .expect("side-effect projection")
            .phase,
        SideEffectPhase::InvocationPrepared {
            invocation_epoch: 1,
            claim_generation: 1,
            claim_fencing_token: side_effect::ClaimFencingToken::new("token-1").expect("token"),
        }
    );
    assert!(committed.artifact_requirements().iter().any(|requirement| {
        requirement.source == EventArtifactReferenceSource::RunSpec
            && requirement.artifact_role == Some(ArtifactRole::TypedExecutionSpec)
    }));
    assert!(committed.artifact_requirements().iter().any(|requirement| {
        requirement.source == EventArtifactReferenceSource::SideEffectIntent
            && requirement.artifact_role == Some(ArtifactRole::SideEffectIntent)
    }));
}

#[test]
fn committed_run_stream_canonical_json_roundtrips_store_authority() {
    let run_id = run_id(144);
    let mut store = admitted_store(&run_id, "committed-stream-json-run-start");
    append_side_effect_prepare(&mut store, &run_id);
    let committed = CommittedRunStream::from_events(run_id.clone(), store.load_run_stream(&run_id))
        .expect("committed stream");

    let encoded = committed_run_stream_canonical_json(&committed).expect("committed stream json");
    let decoded = committed_run_stream_from_canonical_json_slice(
        &run_id,
        encoded.as_bytes(),
        &ArtifactByteAuthorityMap::new(),
    )
    .expect("decode committed stream json");

    assert_eq!(decoded.run_id(), committed.run_id());
    assert_eq!(decoded.events(), committed.events());
    assert_eq!(decoded.commits(), committed.commits());
    assert_eq!(decoded.projection(), committed.projection());
    assert_eq!(decoded.next_seq(), committed.next_seq());
}

#[test]
fn committed_run_stream_canonical_json_rejects_tampered_event_authority() {
    let run_id = run_id(145);
    let mut store = admitted_store(&run_id, "committed-stream-json-tamper-run-start");
    append_side_effect_prepare(&mut store, &run_id);
    let committed = CommittedRunStream::from_events(run_id.clone(), store.load_run_stream(&run_id))
        .expect("committed stream");
    let encoded = committed_run_stream_canonical_json(&committed).expect("committed stream json");
    let mut json: serde_json::Value =
        serde_json::from_slice(encoded.as_bytes()).expect("committed stream value");
    json["events"][0]["payload_hash"] = serde_json::Value::String(content_digest(146).to_string());
    let tampered = PlainCanonicalJsonBytes::from_json_str(
        &serde_json::to_string(&json).expect("tampered json"),
    )
    .expect("tampered canonical json");

    let error = committed_run_stream_from_canonical_json_slice(
        &run_id,
        tampered.as_bytes(),
        &ArtifactByteAuthorityMap::new(),
    )
    .expect_err("tampered envelope rejects");

    assert!(matches!(
        error,
        StoreError::PersistedEventMismatch {
            field: "payload_hash",
            ..
        }
    ));
}

#[test]
fn committed_run_stream_rejects_events_for_a_different_run() {
    let requested_run_id = run_id(142);
    let other_run_id = run_id(143);
    let store = admitted_store(&requested_run_id, "committed-stream-wrong-run-start");

    let error =
        CommittedRunStream::from_events(other_run_id, store.load_run_stream(&requested_run_id))
            .expect_err("wrong run id rejects");
    assert!(matches!(
        error,
        StoreError::PersistedEventMismatch {
            field: "run_id",
            ..
        }
    ));
}

#[test]
fn unknown_run_completion_outcome_tag_is_rejected() {
    let payload = KernelEventPayload::RunCompleted(events::RunCompleted {
        run_id: run_id(95),
        spec_hash: spec_hash(1),
        outcome: events::RunCompletionOutcome::FailedWithoutAcdcClaim,
    });
    let mut json = payload_json_value(&payload);
    json.get_mut("outcome")
        .and_then(serde_json::Value::as_object_mut)
        .expect("outcome object")
        .insert(
            "kind".to_owned(),
            serde_json::Value::String("invented".to_owned()),
        );
    assert!(matches!(
        payload_from_json_value(&json),
        Err(StoreError::Identity(message))
            if message.contains("unknown run completion outcome invented")
    ));
}

#[test]
fn launch_evidence_codec_sorts_deduplicates_and_rejects_legacy_fields() {
    let source_a = events::CatalogSourceEvidence::new(
        "acme/a",
        schema_id("mfm.test.catalog", 101),
        content_digest(102),
    )
    .expect("source a");
    let source_b = events::CatalogSourceEvidence::new(
        "acme/b",
        schema_id("mfm.test.catalog", 103),
        content_digest(104),
    )
    .expect("source b");
    let evidence = events::EntryPointLaunchEvidence::new(
        "mfm.test/catalog-backed@1",
        vec![source_b.clone(), source_a.clone(), source_a],
    )
    .expect("launch evidence");
    assert_eq!(
        evidence
            .catalog_sources
            .iter()
            .map(|source| source.name.as_str())
            .collect::<Vec<_>>(),
        vec!["acme/a", "acme/b"]
    );

    let mut payload = run_admitted(run_id(111));
    if let KernelEventPayload::RunAdmitted(admitted) = &mut payload {
        admitted.entry_point = evidence;
    } else {
        panic!("run-admitted fixture");
    }
    let current = payload_json_value(&payload);
    assert_eq!(
        payload_from_json_value(&current).expect("current payload roundtrip"),
        payload
    );

    for legacy_field in ["resolved_op_id", "entry_point_registry_digest"] {
        let mut legacy = current.clone();
        legacy
            .get_mut("entry_point")
            .and_then(serde_json::Value::as_object_mut)
            .expect("entry point object")
            .insert(
                legacy_field.to_owned(),
                serde_json::Value::String("legacy".to_owned()),
            );
        assert!(
            payload_from_json_value(&legacy).is_err(),
            "legacy field {legacy_field} must be rejected"
        );
    }

    let mut unknown_source = current.clone();
    unknown_source
        .get_mut("entry_point")
        .and_then(serde_json::Value::as_object_mut)
        .and_then(|entry_point| entry_point.get_mut("catalog_sources"))
        .and_then(serde_json::Value::as_array_mut)
        .and_then(|sources| sources.first_mut())
        .and_then(serde_json::Value::as_object_mut)
        .expect("catalog source")
        .insert("unexpected".to_owned(), serde_json::json!(true));
    assert!(payload_from_json_value(&unknown_source).is_err());

    let mut mixed_identity = current.clone();
    mixed_identity
        .get_mut("identity_material")
        .and_then(serde_json::Value::as_object_mut)
        .expect("identity material")
        .insert("legacy_identity_field".to_owned(), serde_json::json!(true));
    assert!(payload_from_json_value(&mixed_identity).is_err());
}

#[test]
fn unrelated_entry_point_summaries_do_not_change_admission_evidence() {
    let payload = run_admitted(run_id(112));
    let current = payload_json_value(&payload);
    let mut with_unrelated_summaries = current.clone();
    with_unrelated_summaries["entry_point_summaries"] = serde_json::json!([
        {"entry_point_id": "mfm.unrelated/entry@1"}
    ]);
    let decoded = payload_from_json_value(&with_unrelated_summaries)
        .expect("unrelated summaries are not admission evidence");
    let KernelEventPayload::RunAdmitted(decoded) = decoded else {
        panic!("run-admitted fixture");
    };
    let KernelEventPayload::RunAdmitted(original) = payload else {
        panic!("run-admitted fixture");
    };
    assert_eq!(decoded.entry_point, original.entry_point);
    assert_eq!(decoded.identity_material, original.identity_material);
    assert_eq!(
        serde_json::to_value(&with_unrelated_summaries["identity_material"])
            .expect("identity material JSON")
            .as_object()
            .expect("identity material object")
            .len(),
        3
    );
}

#[test]
fn manual_resolution_outcome_is_closed() {
    let payload = manual_resolution_recorded_for_run(run_id(96), 96);
    assert!(matches!(
        payload_from_json_value(&payload_json_value(&payload)),
        Ok(KernelEventPayload::ManualResolutionRecorded(_))
    ));

    let mut json = payload_json_value(&payload);
    json.as_object_mut().expect("payload object").insert(
        "outcome".to_owned(),
        serde_json::Value::String("invented".to_owned()),
    );
    assert!(matches!(
        payload_from_json_value(&json),
        Err(StoreError::Identity(message))
            if message.contains("unknown manual resolution outcome invented")
    ));

    let mut missing_authorization = payload_json_value(&payload);
    let object = missing_authorization
        .as_object_mut()
        .expect("payload object");
    object.remove("authorization_schema_id");
    object.remove("authorization_hash");
    object.remove("authorization_artifact_id");
    assert!(matches!(
        payload_from_json_value(&missing_authorization),
        Err(StoreError::Event(message)) if message.contains("authorization_schema_id")
    ));
}
