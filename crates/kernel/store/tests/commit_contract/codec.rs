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
        event_artifact_requirements(&cell_produced(artifact_id(31), content_digest(32)));
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
        event_artifact_requirements(&public_output_produced(artifact_id(33), content_digest(34)));
    assert_eq!(public_requirements.len(), 1);
    assert_eq!(
        public_requirements[0].source,
        EventArtifactReferenceSource::PublicOutputCell
    );
    assert!(!public_requirements[0]
        .source
        .is_terminal_lifecycle_receipt_candidate());

    let retention_requirements = event_artifact_requirements(&retention_refs_appended(
        artifact_id(35),
        content_digest(36),
        ArtifactRole::FactResponse,
    ));
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
    let failure_requirements = event_artifact_requirements(&failure);
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
fn event_artifact_requirement_derivation_covers_artifact_bearing_variants() {
    let sources = |payload: &KernelEventPayload| {
        event_artifact_requirements(payload)
            .into_iter()
            .map(|requirement| requirement.source)
            .collect::<Vec<_>>()
    };
    let diagnostic_ref = |byte: u8| {
        let evidence = ArtifactEvidenceRef {
            artifact_id: artifact_id(byte),
            digest: content_digest(byte.wrapping_add(1)),
            byte_len: 64,
            media_type: media_type("application/json"),
            schema_id: Some(schema_id("mfm.test.diagnostic", byte.wrapping_add(2))),
            semantic_type_id: None,
            producer_node_id: Some(node_id(20)),
            producer_seed_id: None,
            artifact_role: ArtifactRole::RedactedDiagnostic,
        };
        events::ArtifactEvidenceRef {
            artifact_id: evidence.artifact_id.clone(),
            role: evidence.artifact_role,
            schema_id: evidence.schema_id.clone().expect("diagnostic schema"),
            semantic_type_id: None,
            content_digest: evidence.digest.clone(),
            evidence_hash: evidence.evidence_hash().expect("diagnostic evidence hash"),
            byte_len: evidence.byte_len,
            media_type: evidence.media_type,
        }
    };
    let diagnostic_error = |byte: u8| events::MfmErrorInfo {
        code: events::ErrorCode::new("diagnostic_failure").expect("error code"),
        category: events::ErrorCategory::Runtime,
        retryable: false,
        safe_message: "diagnostic failure".to_owned(),
        public_details: None,
        diagnostic_ref: Some(diagnostic_ref(byte)),
    };

    let mut run_payload = run_admitted(run_id(58));
    let KernelEventPayload::RunAdmitted(run_admitted) = &mut run_payload else {
        panic!("run-admitted payload");
    };
    let mut config = store_artifact_ref(artifact_id(59), content_digest(60));
    config.artifact_role = ArtifactRole::TypedConfig;
    config.semantic_type_id = None;
    config.producer_node_id = None;
    run_admitted
        .config_artifacts
        .push(run_artifact_ref(&config));

    let referenced = KernelEventPayload::ArtifactReferenced(events::ArtifactReferenced {
        spec_hash: spec_hash(1),
        node_id: Some(node_id(61)),
        attempt_id: Some(attempt_id(62)),
        artifact_ref: event_artifact_ref(artifact_id(63), content_digest(64)),
    });
    let public_output = public_output_produced_with_rendered_artifact(
        artifact_id(65),
        content_digest(66),
        artifact_id(67),
        content_digest(68),
    );
    let public_failure =
        KernelEventPayload::PublicOutputRenderFailed(events::PublicOutputRenderFailed {
            spec_hash: spec_hash(1),
            node_id: node_id(20),
            attempt_id: attempt_id(23),
            public_schema_id: schema_id("mfm.test.public_output", 3),
            renderer_descriptor_id: descriptor_id(69),
            error: diagnostic_error(70),
        });
    let mut attempt_failure = fact_attempt_failed(false);
    let KernelEventPayload::StateAttemptFailed(attempt_failure_payload) = &mut attempt_failure
    else {
        panic!("attempt-failure payload");
    };
    attempt_failure_payload.error.diagnostic_ref = Some(diagnostic_ref(73));
    let mut receipt = side_effect_receipt(artifact_id(74), content_digest(75));
    let KernelEventPayload::SideEffectReceiptObserved(receipt_payload) = &mut receipt else {
        panic!("receipt payload");
    };
    receipt_payload.resource_touched_set = Some(resource_touched_set(76));
    let mut confirmation = side_effect_confirmation(artifact_id(77), content_digest(78));
    let KernelEventPayload::SideEffectConfirmationObserved(confirmation_payload) =
        &mut confirmation
    else {
        panic!("confirmation payload");
    };
    confirmation_payload.resource_touched_set = Some(resource_touched_set(79));
    let mut side_effect_failure = side_effect_failed(false);
    let KernelEventPayload::SideEffectFailed(side_effect_failure_payload) =
        &mut side_effect_failure
    else {
        panic!("side-effect failure payload");
    };
    side_effect_failure_payload.error.diagnostic_ref = Some(diagnostic_ref(80));

    let cases = [
        (
            run_payload,
            vec![
                EventArtifactReferenceSource::RunSpec,
                EventArtifactReferenceSource::RunCertificate,
                EventArtifactReferenceSource::RunConfig,
            ],
        ),
        (
            run_admitted_with_fact_descriptor_for_node(
                fact_run_id_for_node(81, node_id(90)),
                node_id(90),
            ),
            vec![
                EventArtifactReferenceSource::RunSpec,
                EventArtifactReferenceSource::RunCertificate,
                EventArtifactReferenceSource::FactDescriptor,
            ],
        ),
        (
            fact_recorded(&fact_artifact_ref()),
            vec![EventArtifactReferenceSource::FactResponse],
        ),
        (
            referenced,
            vec![EventArtifactReferenceSource::ArtifactReferenced],
        ),
        (
            cell_produced(artifact_id(82), content_digest(83)),
            vec![EventArtifactReferenceSource::StateOutput],
        ),
        (
            public_output,
            vec![
                EventArtifactReferenceSource::PublicOutputCell,
                EventArtifactReferenceSource::PublicOutputRendered,
            ],
        ),
        (
            public_failure,
            vec![EventArtifactReferenceSource::PublicOutputRenderFailureDiagnostic],
        ),
        (
            attempt_failure,
            vec![EventArtifactReferenceSource::StateAttemptFailureDiagnostic],
        ),
        (
            manual_resolution_recorded_for_run(run_id(84), 84),
            vec![
                EventArtifactReferenceSource::ManualResolutionEvidence,
                EventArtifactReferenceSource::ManualResolutionAuthorization,
            ],
        ),
        (
            side_effect_intent(artifact_id(87), content_digest(88)),
            vec![EventArtifactReferenceSource::SideEffectIntent],
        ),
        (
            side_effect_prepared(1, "token-1"),
            vec![EventArtifactReferenceSource::PreparedInvocation],
        ),
        (
            side_effect_not_submitted(artifact_id(89), content_digest(90)),
            vec![EventArtifactReferenceSource::NotSubmittedProof],
        ),
        (
            side_effect_submission_observed(artifact_id(91), content_digest(92)),
            vec![EventArtifactReferenceSource::Submission],
        ),
        (
            side_effect_submission_unknown(artifact_id(93), content_digest(94)),
            vec![EventArtifactReferenceSource::SubmissionUnknownEvidence],
        ),
        (
            receipt,
            vec![
                EventArtifactReferenceSource::Receipt,
                EventArtifactReferenceSource::ResourceTouchedSet,
            ],
        ),
        (
            confirmation,
            vec![
                EventArtifactReferenceSource::Confirmation,
                EventArtifactReferenceSource::ResourceTouchedSet,
            ],
        ),
        (
            side_effect_ambiguous(artifact_id(95), content_digest(96)),
            vec![EventArtifactReferenceSource::AmbiguityEvidence],
        ),
        (
            side_effect_failure,
            vec![EventArtifactReferenceSource::SideEffectFailureDiagnostic],
        ),
        (
            retention_refs_appended(
                artifact_id(97),
                content_digest(98),
                ArtifactRole::FactResponse,
            ),
            vec![EventArtifactReferenceSource::RetentionRef],
        ),
        (
            retention_manifest_projected(1, content_digest(99), None, artifact_id(100)),
            vec![EventArtifactReferenceSource::RetentionManifest],
        ),
    ];

    for (payload, expected) in cases {
        assert_eq!(sources(&payload), expected);
    }
}

#[test]
fn store_owned_requirement_constructors_preserve_exact_bindings() {
    let run_payload = run_admitted(run_id(39));
    let KernelEventPayload::RunAdmitted(run_admitted) = &run_payload else {
        panic!("run-admitted payload");
    };
    assert_eq!(
        run_artifact_requirement(
            EventArtifactReferenceSource::RunSpec,
            &run_admitted.spec_artifact,
            ArtifactRole::TypedExecutionSpec,
        ),
        event_artifact_requirements(&run_payload)[0]
    );

    let config_ref = spec::ConfigRef {
        schema_id: schema_id("mfm.test.config", 40),
        artifact_id: artifact_id(41),
        digest: content_digest(42),
        byte_len: 128,
        media_type: media_type("application/json"),
    };
    let config_evidence = ArtifactEvidenceRef {
        artifact_id: config_ref.artifact_id.clone(),
        digest: config_ref.digest.clone(),
        byte_len: config_ref.byte_len,
        media_type: config_ref.media_type.clone(),
        schema_id: Some(config_ref.schema_id.clone()),
        semantic_type_id: None,
        producer_node_id: None,
        producer_seed_id: None,
        artifact_role: ArtifactRole::TypedConfig,
    };
    validate_artifact_requirement_against_evidence(
        &config_ref_artifact_requirement(&config_ref).expect("config requirement"),
        &config_evidence,
    )
    .expect("config binding");

    let referenced = events::ArtifactReferenced {
        spec_hash: spec_hash(1),
        node_id: Some(node_id(40)),
        attempt_id: Some(attempt_id(41)),
        artifact_ref: event_artifact_ref(artifact_id(42), content_digest(43)),
    };
    assert_eq!(
        artifact_referenced_artifact_requirement(&referenced),
        event_artifact_requirements(&KernelEventPayload::ArtifactReferenced(referenced.clone()))[0]
    );

    let seed_id = SeedId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(44));
    let seed_spec = spec::SeedSpec {
        seed_id: seed_id.clone(),
        seed_key: spec::StableAuthorKey::new("seed").expect("seed key"),
        cell_id: cell_id(44),
        scope_id: scope_id(45),
        semantic_type_id: semantic_id("seed", 46),
        schema_id: schema_id("mfm.test.seed", 47),
        required_digest: Some(content_digest(48)),
    };
    let seed_evidence = ArtifactEvidenceRef {
        artifact_id: artifact_id(49),
        digest: content_digest(48),
        byte_len: 64,
        media_type: media_type("application/json"),
        schema_id: Some(seed_spec.schema_id.clone()),
        semantic_type_id: Some(seed_spec.semantic_type_id.clone()),
        producer_node_id: None,
        producer_seed_id: Some(seed_id.clone()),
        artifact_role: ArtifactRole::SeedInput,
    };
    let seed_requirement =
        seed_artifact_requirement(&seed_spec, &seed_evidence).expect("seed requirement");
    validate_artifact_requirement_against_evidence(&seed_requirement, &seed_evidence)
        .expect("seed binding");
    let seed_cell = events::SeedCellRef {
        seed_id,
        cell_id: seed_spec.cell_id,
        scope_id: seed_spec.scope_id,
        semantic_type_id: seed_spec.semantic_type_id,
        schema_id: seed_spec.schema_id.clone(),
        digest: seed_evidence.digest.clone(),
        seed_artifact: events::ArtifactEvidenceRef {
            artifact_id: seed_evidence.artifact_id.clone(),
            role: seed_evidence.artifact_role,
            schema_id: seed_spec.schema_id,
            semantic_type_id: seed_evidence.semantic_type_id.clone(),
            content_digest: seed_evidence.digest.clone(),
            evidence_hash: seed_evidence.evidence_hash().expect("seed evidence hash"),
            byte_len: seed_evidence.byte_len,
            media_type: seed_evidence.media_type.clone(),
        },
    };
    assert_eq!(seed_cell_artifact_requirement(&seed_cell), seed_requirement);

    let public_payload = public_output_produced_with_rendered_artifact(
        artifact_id(50),
        content_digest(51),
        artifact_id(52),
        content_digest(53),
    );
    let KernelEventPayload::PublicOutputProduced(public_output) = &public_payload else {
        panic!("public-output payload");
    };
    let public_requirements = event_artifact_requirements(&public_payload);
    let cell_requirement = public_output_cell_artifact_requirement(&public_output.cells[0]);
    let mut event_cell_requirement = public_requirements[0].clone();
    event_cell_requirement.artifact_role = Some(ArtifactRole::StateOutput);
    assert_eq!(cell_requirement, event_cell_requirement);
    let rendered_requirement = public_output_rendered_artifact_requirement(
        public_output,
        public_output
            .rendered_artifact_id
            .as_ref()
            .expect("rendered artifact id"),
        &public_output.rendered_digest,
        media_type("application/json"),
    )
    .expect("rendered requirement");
    assert_eq!(
        rendered_requirement.artifact_id,
        public_requirements[1].artifact_id
    );
    assert_eq!(
        rendered_requirement.evidence_hash,
        public_requirements[1].evidence_hash
    );

    let descriptor = fact_descriptor_fixture();
    let descriptor_requirement =
        fact_descriptor_artifact_requirement(&descriptor.projection).expect("descriptor");
    validate_artifact_requirement_against_evidence(
        &descriptor_requirement,
        &descriptor.descriptor_evidence,
    )
    .expect("descriptor binding");

    let fact_evidence = fact_artifact_ref();
    let fact_payload = fact_recorded(&fact_evidence);
    let KernelEventPayload::FactRecorded(fact_recorded) = &fact_payload else {
        panic!("fact-recorded payload");
    };
    let fact_ref = mfm_facts::InternalFactRef::from_claim(
        mfm_facts::FactClaimId::new(run_id(56), 1, 0).expect("fact claim id"),
        event_id(57),
        "2026-07-21T00:00:00Z".to_owned(),
        fact_recorded.node_id.clone(),
        &fact_recorded.claim,
    )
    .expect("internal fact ref");
    assert_eq!(
        fact_response_artifact_requirement(&fact_ref),
        event_artifact_requirements(&fact_payload)[0]
    );

    let diagnostic = event_artifact_ref(artifact_id(54), content_digest(55));
    let diagnostic_requirement = diagnostic_artifact_requirement(&diagnostic);
    assert_eq!(diagnostic_requirement.artifact_id, diagnostic.artifact_id);
    assert_eq!(
        diagnostic_requirement.evidence_hash,
        diagnostic.evidence_hash
    );
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

    let requirements = event_artifact_requirements(&payload);
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
    let projection = store
        .projection_snapshot()
        .fact_query_entries()
        .next()
        .map(|(_, projection)| projection)
        .expect("fact query projection");
    assert_eq!(projection.producer_node_id(), &node_id(90));
    assert_eq!(projection.attempt_id(), &attempt_id(91));
    assert_eq!(projection.fact_key(), &fact_key());
    assert_eq!(
        projection.response_schema_id(),
        &schema_id("mfm.test.fact_response", 96)
    );
    assert_eq!(projection.response_hash(), &response.digest);
    assert_eq!(projection.artifact_id(), &response.artifact_id);
    assert_eq!(projection.response_artifact_evidence(), Some(&response));
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
    let source_a = events::ConfiguredTargetEvidence::new(
        StableAuthorKey::new("acme/a").expect("target a"),
        schema_id("mfm.test.catalog", 101),
        content_digest(102),
    );
    let source_b = events::ConfiguredTargetEvidence::new(
        StableAuthorKey::new("acme/b").expect("target b"),
        schema_id("mfm.test.catalog", 103),
        content_digest(104),
    );
    let evidence = events::EntryPointLaunchEvidence::new(
        "mfm.test/configured-target@1",
        vec![source_b.clone(), source_a.clone(), source_a],
    )
    .expect("launch evidence");
    assert_eq!(
        evidence
            .configured_targets
            .iter()
            .map(|source| source.target.as_str())
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
        .and_then(|entry_point| entry_point.get_mut("configured_targets"))
        .and_then(serde_json::Value::as_array_mut)
        .and_then(|sources| sources.first_mut())
        .and_then(serde_json::Value::as_object_mut)
        .expect("configured target")
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
fn event_payload_codec_rejects_unknown_and_deleted_fact_fields() {
    let payload = run_admitted(run_id(112));
    let current = payload_json_value(&payload);
    let mut with_unrelated_summaries = current.clone();
    with_unrelated_summaries["entry_point_summaries"] = serde_json::json!([
        {"entry_point_id": "mfm.unrelated/entry@1"}
    ]);
    assert!(payload_from_json_value(&with_unrelated_summaries).is_err());

    let fact_payload = fact_recorded(&fact_artifact_ref());
    let fact_json = payload_json_value(&fact_payload);
    for deleted_field in ["visibility", "request", "producer"] {
        let mut legacy = fact_json.clone();
        legacy["claim"]
            .as_object_mut()
            .expect("fact claim object")
            .insert(deleted_field.to_owned(), serde_json::json!({}));
        assert!(
            payload_from_json_value(&legacy).is_err(),
            "deleted fact field {deleted_field} must be rejected"
        );
    }
}

#[test]
fn committed_stream_codec_rejects_unknown_envelope_fields() {
    let run_id = run_id(113);
    let store = admitted_store(&run_id, "committed-stream-unknown-field");
    let committed = CommittedRunStream::from_events(run_id.clone(), store.load_run_stream(&run_id))
        .expect("committed stream");
    let encoded = committed_run_stream_canonical_json(&committed).expect("committed stream json");
    let mut json: serde_json::Value =
        serde_json::from_slice(encoded.as_bytes()).expect("committed stream value");
    json["events"][0]["unexpected"] = serde_json::json!(true);
    let unknown = PlainCanonicalJsonBytes::from_json_str(
        &serde_json::to_string(&json).expect("unknown-field json"),
    )
    .expect("unknown-field canonical json");

    assert!(committed_run_stream_from_canonical_json_slice(
        &run_id,
        unknown.as_bytes(),
        &ArtifactByteAuthorityMap::new(),
    )
    .is_err());
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
