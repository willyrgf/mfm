use super::*;

#[tokio::test]
async fn app_read_services_reconstruct_fact_bearing_status_from_committed_stream() {
    let (run_id, store, registry) = launch_app_fact_run().await;
    let read_services = make_run_read_services(Arc::new(store.clone()), registry);

    let status = read_services
        .run_status(&run_id)
        .await
        .expect("fact-bearing status uses committed artifact authority");
    assert_eq!(status.run_mode, RunModeStatus::Completed);
    assert!(status.head_seq > store::StreamSeq::FIRST.as_u64());

    let stream = read_services
        .run_stream(&run_id)
        .await
        .expect("fact-bearing stream uses committed artifact authority");
    assert_eq!(stream.run_id, run_id.as_str());
    assert!(stream.events.len() > 1);
}

#[tokio::test]
async fn app_read_services_fail_closed_for_missing_or_tampered_fact_artifacts() {
    let (run_id, store, registry) = launch_app_fact_run().await;

    for mode in [
        CommittedStreamArtifactMode::Missing,
        CommittedStreamArtifactMode::TamperFactResponse,
    ] {
        let overridden = OverriddenCommittedStreamStore::new(store.clone(), mode);
        let read_services = make_run_read_services(Arc::new(overridden), registry.clone());

        let status_error = read_services
            .run_status(&run_id)
            .await
            .expect_err("fact artifact authority failure must close status reads");
        assert_eq!(status_error.code, "RunStoreRejected", "mode {mode:?}");

        let stream_error = read_services
            .run_stream(&run_id)
            .await
            .expect_err("fact artifact authority failure must close stream reads");
        assert_eq!(stream_error.code, "RunStoreRejected", "mode {mode:?}");
    }
}

#[tokio::test]
async fn public_fact_catalog_discovers_projected_descriptors() {
    let (_run_id, store, _registry) = launch_app_fact_run().await;
    let descriptor = AppLaunchFact::descriptor().expect("fact descriptor");
    let projection = store.projection_snapshot().expect("projection snapshot");
    let public_catalog =
        FactCatalogService::from_public_projection(vec![descriptor.clone()], &projection)
            .expect("public catalog");

    assert_eq!(
        public_catalog.list_kinds(),
        vec![PublicFactKindSummary {
            fact_kind: "mfm.app.test.launch".to_owned(),
            descriptor_count: 1,
        }]
    );

    let (_claim_id, entry) = projection
        .fact_query_entries()
        .next()
        .expect("queryable fact projection");
    let projection_without_facts =
        store::ProjectionSnapshot::from_parts(store::ProjectionSnapshotParts {
            fact_descriptors: BTreeMap::from([(
                entry.fact_descriptor_hash().clone(),
                projection
                    .fact_descriptor(entry.fact_descriptor_hash())
                    .expect("descriptor projection")
                    .clone(),
            )]),
            ..store::ProjectionSnapshotParts::default()
        })
        .expect("descriptor-only projection");
    let empty_catalog =
        FactCatalogService::from_public_projection(vec![descriptor], &projection_without_facts)
            .expect("descriptor-only catalog");
    assert!(empty_catalog.list_kinds().is_empty());
}

#[tokio::test]
async fn run_read_services_load_public_fact_catalog_from_retained_projection_authority() {
    let (_run_id, store, registry) = launch_app_fact_run().await;
    let services = make_run_read_services(Arc::new(store.clone()), registry);

    assert_eq!(
        services.fact_kinds().await.expect("fact kinds"),
        vec![PublicFactKindSummary {
            fact_kind: "mfm.app.test.launch".to_owned(),
            descriptor_count: 1,
        }]
    );
    let described = services
        .describe_fact_kind("mfm.app.test.launch")
        .await
        .expect("fact description");
    assert_eq!(described.len(), 1);
    assert_eq!(described[0].fields.len(), 2);
    let explained = services
        .explain_fact_kind("mfm.app.test.launch")
        .await
        .expect("fact explanation");
    assert_eq!(explained.descriptors, described);

    let projection = store.projection_snapshot().expect("projection snapshot");
    let (_claim_id, entry) = projection
        .fact_query_entries()
        .next()
        .expect("queryable fact projection");
    let public_ref =
        public_ref_id(&entry.internal_ref().expect("internal ref")).expect("public ref id");
    let resolved = services
        .resolve_public_fact_ref(&public_ref)
        .await
        .expect("resolve public ref");
    assert_eq!(resolved.fact_kind, "mfm.app.test.launch");
    assert_eq!(resolved.fields.len(), 2);

    let rendered = serde_json::to_string(&resolved).expect("public fact JSON");
    assert_public_fact_json_redacts_private_tokens_for_test(&rendered, std::iter::empty::<&str>());
}

#[tokio::test]
async fn run_read_services_public_fact_reads_are_store_scoped_across_runs() {
    let store = store::AsyncInMemoryRunStore::default();
    let (first_run_id, store, _registry) = launch_app_fact_run_in_store(
        store,
        Some(content_digest_for_bytes(
            b"mfm.app.test.first-public-fact-run",
        )),
    )
    .await;
    let (second_run_id, store, registry) = launch_app_fact_run_in_store_with_state_key(
        store,
        Some(content_digest_for_bytes(
            b"mfm.app.test.second-public-fact-run",
        )),
        "fact-state-second",
        false,
    )
    .await;
    assert_ne!(first_run_id, second_run_id);
    let services = make_run_read_services(Arc::new(store.clone()), registry);

    assert_eq!(
        services.fact_kinds().await.expect("fact kinds"),
        vec![PublicFactKindSummary {
            fact_kind: "mfm.app.test.launch".to_owned(),
            descriptor_count: 1,
        }]
    );

    let page = services
        .query_public_facts(app_launch_fact_query_request())
        .await
        .expect("public fact query");

    assert_eq!(page.facts.len(), 2);
    let public_refs = page
        .facts
        .iter()
        .map(|fact| fact.public_ref.clone())
        .collect::<BTreeSet<_>>();
    assert_eq!(public_refs.len(), 2);
    for public_ref in public_refs {
        let resolved = services
            .resolve_public_fact_ref(&public_ref)
            .await
            .expect("resolve public ref");
        assert_eq!(resolved.fact_kind, "mfm.app.test.launch");
        assert_eq!(resolved.fields.len(), 2);
    }
}

#[tokio::test]
async fn public_fact_query_redacts_internal_projection_fields() {
    let (_run_id, store, registry) = launch_app_fact_run().await;
    let projection = store.projection_snapshot().expect("projection snapshot");
    let (_claim_id, entry) = projection
        .fact_query_entries()
        .next()
        .expect("queryable fact projection");
    let services = make_run_read_services(Arc::new(store.clone()), registry);
    let page = services
        .query_public_facts(app_launch_fact_query_request())
        .await
        .expect("public fact query");

    assert_eq!(page.facts.len(), 1);
    assert_eq!(page.facts[0].fact_kind, "mfm.app.test.launch");
    assert_eq!(page.facts[0].fields.len(), 2);
    assert!(page.facts[0]
        .fields
        .iter()
        .any(|field| field.field_id == "result.amount"
            && field.value == PublicFactScalarValue::UnsignedInteger(15)));

    let rendered = serde_json::to_string(&page).expect("public page JSON");
    assert_public_fact_json_redacts_private_tokens_for_test(
        &rendered,
        [
            entry.artifact_id().as_str(),
            entry.artifact_evidence_hash().as_str(),
            entry.fact_descriptor_hash().as_str(),
            entry.subject_material_hash().as_str(),
        ],
    );
}

#[test]
fn certified_launch_stages_fact_descriptor_artifacts() {
    let request = prepare_app_fact_launch().expect("prepared launch");
    let descriptor = AppLaunchFact::descriptor().expect("fact descriptor");
    let canonical =
        mfm_program::facts::canonical_fact_descriptor_bytes(&descriptor).expect("canonical");
    let descriptor_hash =
        mfm_program::facts::fact_descriptor_hash(&descriptor).expect("descriptor hash");
    let descriptor_schema =
        mfm_program::facts::fact_descriptor_schema_id().expect("descriptor schema");

    assert_eq!(request.evidence.fact_descriptor_artifacts.len(), 1);
    let artifact = &request.evidence.fact_descriptor_artifacts[0];
    assert_eq!(artifact.bytes, canonical.as_bytes());
    assert_eq!(artifact.evidence.digest, descriptor_hash);
    assert_eq!(
        artifact.evidence.artifact_id,
        artifact_id_for_digest(&descriptor_hash)
    );
    assert_eq!(
        artifact.evidence.byte_len,
        canonical.as_bytes().len() as u64
    );
    assert_eq!(
        artifact.evidence.artifact_role,
        events::ArtifactRole::FactDescriptor
    );
    assert_eq!(
        artifact.evidence.schema_id.as_ref(),
        Some(&descriptor_schema)
    );
    assert!(artifact.evidence.semantic_type_id.is_none());
    assert!(artifact.evidence.producer_node_id.is_none());
    assert!(artifact.evidence.producer_seed_id.is_none());
}

#[test]
fn resource_key_status_redacts_raw_key() {
    let raw_key = "0x000000000000000000000000000000000000dead";
    let evidence = events::ResourceKeyEvidence {
        namespace: spec::ResourceNamespace::new("mfm.test.account_nonce")
            .expect("resource namespace"),
        key_schema_id: SchemaId::new(
            "mfm.test.account_nonce.resource_key",
            "1",
            DigestAlgorithm::Sha256JcsV1,
            sha256_digest_bytes(b"mfm.test.account_nonce.resource_key"),
        )
        .expect("schema id"),
        key: events::ResourceKey::new(raw_key).expect("resource key"),
    };

    let status = resource_key_status(&evidence);
    let rendered = serde_json::to_string(&status).expect("status JSON");

    assert_eq!(status.namespace, "mfm.test.account_nonce");
    assert_ne!(status.key_digest, raw_key);
    assert!(rendered.contains("key_digest"));
    assert!(!rendered.contains(raw_key));
    assert!(!rendered.contains("\"key\""));
}

#[test]
fn replay_diagnostic_rejects_digest_matched_malformed_evm_chain_mismatch_details() {
    for details in [
        serde_json::json!({
            "network_id": "bad network id",
            "expected_chain_id": 31337,
            "observed_chain_id": 31338,
            "source_ref": "primary",
        }),
        serde_json::json!({
            "network_id": "rest-control-eth",
            "expected_chain_id": 0,
            "observed_chain_id": 31338,
            "source_ref": "primary",
        }),
        serde_json::json!({
            "network_id": "rest-control-eth",
            "expected_chain_id": 31337,
            "observed_chain_id": 31337,
            "source_ref": "primary",
        }),
        serde_json::json!({
            "network_id": "rest-control-eth",
            "expected_chain_id": 31337,
            "observed_chain_id": 31338,
            "source_ref": "primary",
            "unexpected": true,
        }),
        serde_json::json!({
            "network_id": "rest-control-eth",
            "expected_chain_id": 31337,
            "observed_chain_id": 31338,
            "source_ref": "bad source ref",
        }),
    ] {
        let diagnostics = serde_json::json!([{
                "provider_family": "evm",
                "code": "source_mismatch",
                "operation": null,
                "fields": details,
        }]);
        let expected =
            serde_json::from_value::<Vec<RedactedProviderDiagnostic>>(diagnostics.clone())
                .map(|_| {
                    events::RedactedJson::new(
                        canonical_value_digest(&diagnostics).expect("diagnostic digest"),
                    )
                })
                .unwrap_or_else(|_| {
                    events::RedactedJson::new(
                        canonical_value_digest(&serde_json::Value::Null).expect("null digest"),
                    )
                });
        let error = verify_replay_diagnostic_json(Some(&expected), &diagnostics)
            .expect_err("malformed typed diagnostic must fail replay");

        assert_eq!(error.code, "ReplayDiagnosticInvalid");
    }
}

#[test]
fn replay_diagnostic_accepts_generic_details_with_network_id() {
    let diagnostics = serde_json::json!([{
            "provider_family": "portfolio",
            "code": "response_invalid",
            "operation": null,
            "fields": {
                "domain_code": "missing_fact",
                "network_id": "ethereum-mainnet",
            },
    }]);
    let expected =
        events::RedactedJson::new(canonical_value_digest(&diagnostics).expect("details digest"));

    verify_replay_diagnostic_json(Some(&expected), &diagnostics)
        .expect("generic diagnostic details must not be classified as EVM mismatch evidence");
}

#[tokio::test]
async fn run_read_services_are_evidence_only() {
    let store = store::AsyncInMemoryRunStore::default();
    let services = make_run_read_services(
        Arc::new(store),
        production_certification_registry().expect("cert registry"),
    );
    let run_id = RunId::parse(
        "run:sha256-jcs-v1:0000000000000000000000000000000000000000000000000000000000000001",
    )
    .expect("run id");

    let status = services
        .run_status(&run_id)
        .await
        .expect_err("missing run should come from store evidence");
    assert_eq!(status.code, "RunNotFound");

    let observations = services
        .read_run_observations(store::RunObservationQuery::new(None, 50, 0))
        .await
        .expect("list observations does not construct live drivers");
    assert!(observations.runs.is_empty());
}

#[tokio::test]
async fn launch_run_reaps_expired_execution_claim_and_retries_admission() {
    let store = store::AsyncInMemoryRunStore::default();
    let request = prepare_app_fact_launch().expect("prepared launch");
    let runtime_spec =
        CertifiedRuntimeSpec::new(request.certified_spec.clone()).expect("runtime spec");
    let runners = app_fact_runner_registry(&runtime_spec, Arc::new(store.clone()));
    let services = make_run_services(
        runners,
        Arc::new(store.clone()),
        app_fact_certification_registry(),
    );
    let execution_scope =
        store::ExecutionClaimScope::from_run_identity_material(&request.identity_material);
    let stale_token =
        store::AdmissionToken::new("mfm.app.test.expired-launch-claim").expect("token");
    match store
        .acquire_execution_claim(&execution_scope, &request.run_id, stale_token)
        .await
        .expect("pre-acquire execution claim")
    {
        store::NowaitSkipAdmissionResult::Admitted(_) => {}
        store::NowaitSkipAdmissionResult::Busy(_) => panic!("test execution claim already busy"),
    }
    assert!(
        store
            .expire_execution_claim_for_test(&request.run_id)
            .expect("expire execution claim"),
        "pre-acquired claim should exist"
    );
    let run_id = request.run_id.clone();

    let launch = services
        .launch_run(request)
        .await
        .expect("launch retries after expired claim");

    assert!(matches!(launch, RunLaunchOutcome::Admitted { .. }));
    assert!(store
        .load_run_stream(&run_id)
        .await
        .expect("run stream")
        .iter()
        .any(|event| matches!(event.payload(), events::KernelEventPayload::RunAdmitted(_))));
    assert!(matches!(
        store
            .execution_claim_status(&execution_scope)
            .await
            .expect("execution claim status"),
        store::ExecutionClaimStatus::Unclaimed
    ));
}

#[tokio::test]
async fn postgres_store_authority_error_is_redacted_for_public_app_surface() {
    let database_url = "postgres://mfm_user:super-secret@127.0.0.1:notaport/mfm";
    let error = match connect_production_store(Some(database_url)).await {
        Ok(_) => panic!("invalid postgres URL should not connect"),
        Err(error) => error,
    };

    assert_eq!(error.code, "RunStoreUnavailable");
    assert_eq!(error.message, "Run store is unavailable");
    let rendered = format!("{error:?}\n{error}");
    for forbidden in [
        database_url,
        "mfm_user",
        "super-secret",
        "127.0.0.1",
        "notaport",
    ] {
        assert!(
            !rendered.contains(forbidden),
            "app error leaked `{forbidden}` in {rendered}"
        );
    }
}

#[test]
fn public_status_dto_surfaces_attempt_error_codes() {
    use mfm_ids::{AttemptId, EventId, NodeId};

    let run_id = RunId::parse(
        "run:sha256-jcs-v1:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    )
    .expect("run id");
    let node_id = NodeId::parse(
        "node:sha256-jcs-v1:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
    )
    .expect("node id");
    let attempt_id = AttemptId::parse(
        "attempt:sha256-jcs-v1:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
    )
    .expect("attempt id");
    let event_id = EventId::parse(
        "event:sha256-jcs-v1:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
    )
    .expect("event id");
    let attempt = store::AttemptProjection {
        run_id,
        node_id,
        attempt_id,
        event_id,
        status: store::AttemptStatus::Failed {
            retryable: false,
            error: Box::new(
                events::MfmErrorInfo::new(
                    events::ErrorCode::new("missing_fact").expect("code"),
                    events::ErrorCategory::Validation,
                    false,
                    "holding fact is missing".to_owned(),
                )
                .expect("error info"),
            ),
        },
    };
    let disposition = attempt_disposition(&attempt);
    assert_eq!(disposition.disposition, "failed");
    assert_eq!(disposition.error_code.as_deref(), Some("missing_fact"));
    assert_eq!(disposition.retryable, Some(false));
}
