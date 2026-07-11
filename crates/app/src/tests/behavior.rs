use super::*;

#[tokio::test]
async fn retained_artifact_adapter_preserves_read_request_expectations() {
    let bytes = br#"{"answer":42}"#.to_vec();
    let digest = content_digest_for_bytes(&bytes);
    let schema_id = SchemaId::new(
        "mfm.test.config",
        "1",
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(b"mfm.test.config"),
    )
    .expect("schema id");
    let config_ref = spec::ConfigRef {
        schema_id: schema_id.clone(),
        artifact_id: artifact_id_for_digest(&digest),
        digest: digest.clone(),
        byte_len: bytes.len() as u64,
        media_type: spec::MediaType::new("application/json").expect("media"),
    };
    let evidence = store::ArtifactEvidenceRef {
        artifact_id: config_ref.artifact_id.clone(),
        digest,
        byte_len: bytes.len() as u64,
        media_type: config_ref.media_type.clone(),
        schema_id: Some(schema_id),
        semantic_type_id: None,
        producer_node_id: None,
        producer_seed_id: None,
        artifact_role: events::ArtifactRole::TypedConfig,
    };
    let expected = store::EventArtifactRequirement {
        source: store::EventArtifactReferenceSource::ArtifactReferenced,
        artifact_id: config_ref.artifact_id.clone(),
        evidence_hash: evidence.evidence_hash().expect("config evidence hash"),
        digest: Some(config_ref.digest.clone()),
        byte_len: Some(config_ref.byte_len),
        media_type: Some(config_ref.media_type.clone()),
        schema_id: Some(config_ref.schema_id.clone()),
        semantic_type_id: None,
        producer_node_id: None,
        producer_seed_id: None,
        artifact_role: Some(events::ArtifactRole::TypedConfig),
    };
    let artifact = store::VerifiedRunArtifactBytes::new(bytes, evidence, &expected)
        .expect("verified artifact");
    let seen = Arc::new(Mutex::new(Vec::new()));
    let provider = artifact_read_provider_from_retained(ExpectingRetainedArtifactProvider {
        expected: expected.clone(),
        artifact,
        seen: Arc::clone(&seen),
    });

    provider
        .read_artifact(
            &mfm_artifact_capabilities::ArtifactReadRequest::from_certified_config_ref(&config_ref)
                .expect("config request"),
        )
        .await
        .expect("adapter preserves exact config expectation");

    assert_eq!(*seen.lock().expect("seen lock"), vec![expected]);
}

#[tokio::test]
async fn app_read_services_reconstruct_fact_bearing_status_from_committed_stream() {
    let (run_id, store, registry) = launch_app_fact_run().await;
    let read_services = make_run_read_services(store.clone(), store, registry);

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
        let read_services =
            make_run_read_services(overridden.clone(), overridden, registry.clone());

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
async fn public_fact_catalog_discovers_only_platform_descriptors() {
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

    let (_claim_id, platform_entry) = projection
        .fact_index_entries()
        .next()
        .expect("platform fact index entry");
    let mut control_entry = platform_entry.clone();
    control_entry.audience = mfm_facts::FactAudience::Control;
    let control_projection =
        store::ProjectionSnapshot::from_parts(store::ProjectionSnapshotParts {
            fact_index_entries: BTreeMap::from([(
                control_entry.fact_claim_id.clone(),
                control_entry,
            )]),
            ..store::ProjectionSnapshotParts::default()
        })
        .expect("control projection");
    let control_catalog =
        FactCatalogService::from_public_projection(vec![descriptor], &control_projection)
            .expect("control catalog");

    assert!(control_catalog.list_kinds().is_empty());
    let error = control_catalog
        .describe_kind("mfm.app.test.launch")
        .expect_err("control-only descriptors are not publicly discoverable");
    assert_eq!(error.class, ErrorClass::NotFound);
    assert_eq!(error.code, "FactNotFound");

    let (_record_claim_id, platform_record) = projection
        .fact_records()
        .next()
        .expect("recorded fact projection");
    let run_private_claim = mfm_facts::FactClaim::new(mfm_facts::FactClaimParts {
        visibility: mfm_facts::FactVisibility::RunPrivate,
        fact_kind: platform_record.claim.fact_kind().clone(),
        fact_descriptor_hash: platform_record.claim.fact_descriptor_hash().clone(),
        subject: platform_record.claim.subject().clone(),
        observed_at: platform_record.claim.observed_at().map(str::to_owned),
        request: platform_record.claim.request().cloned(),
        response: platform_record.claim.response().clone(),
        producer: platform_record.claim.producer().clone(),
    })
    .expect("run-private claim");
    let mut run_private_record = platform_record.clone();
    run_private_record.claim = run_private_claim;
    let run_private_projection =
        store::ProjectionSnapshot::from_parts(store::ProjectionSnapshotParts {
            fact_records: BTreeMap::from([(
                run_private_record.fact_claim_id.clone(),
                run_private_record,
            )]),
            ..store::ProjectionSnapshotParts::default()
        })
        .expect("run-private projection");
    let run_private_catalog = FactCatalogService::from_public_projection(
        vec![AppLaunchFact::descriptor().expect("fact descriptor")],
        &run_private_projection,
    )
    .expect("run-private catalog");
    assert!(run_private_catalog.list_kinds().is_empty());
}

#[tokio::test]
async fn run_read_services_load_public_fact_catalog_from_retained_projection_authority() {
    let (_run_id, store, registry) = launch_app_fact_run().await;
    let services = make_run_read_services(store.clone(), store.clone(), registry);

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
        .fact_index_entries()
        .next()
        .expect("platform fact index entry");
    let public_ref =
        public_ref_id(&internal_fact_ref_for_entry(entry).expect("platform internal ref"))
            .expect("public ref id");
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
    let (first_run_id, store, _registry) = launch_app_fact_run_in_store_with_visibility(
        store,
        mfm_program::facts::FactVisibility::indexed_default(
            mfm_program::facts::FactAudience::Platform,
        ),
        Some(content_digest_for_bytes(
            b"mfm.app.test.first-public-fact-run",
        )),
    )
    .await;
    let (second_run_id, store, registry) =
        launch_app_fact_run_in_store_with_visibility_and_state_key(
            store,
            mfm_program::facts::FactVisibility::indexed_default(
                mfm_program::facts::FactAudience::Platform,
            ),
            Some(content_digest_for_bytes(
                b"mfm.app.test.second-public-fact-run",
            )),
            "fact-state-second",
            false,
        )
        .await;
    assert_ne!(first_run_id, second_run_id);
    let services = make_run_read_services(store.clone(), store.clone(), registry);

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
async fn run_read_services_do_not_disclose_non_public_facts() {
    for (case, visibility, has_index_entry) in [
        (
            "control",
            mfm_program::facts::FactVisibility::indexed_default(
                mfm_program::facts::FactAudience::Control,
            ),
            true,
        ),
        (
            "run-private",
            mfm_program::facts::FactVisibility::RunPrivate,
            false,
        ),
    ] {
        let (_run_id, store, registry) = launch_app_fact_run_with_visibility(visibility).await;
        let services = make_run_read_services(store.clone(), store.clone(), registry);

        assert!(
            services
                .fact_kinds()
                .await
                .expect("non-public fact kinds")
                .is_empty(),
            "{case} facts should not be listed"
        );
        let error = services
            .describe_fact_kind("mfm.app.test.launch")
            .await
            .expect_err("non-public descriptors are not public");
        assert_eq!(error.class, ErrorClass::NotFound, "{case}");
        assert_eq!(error.code, "FactNotFound", "{case}");
        let error = services
            .explain_fact_kind("mfm.app.test.launch")
            .await
            .expect_err("non-public descriptors are not explainable");
        assert_eq!(error.class, ErrorClass::NotFound, "{case}");
        assert_eq!(error.code, "FactNotFound", "{case}");

        let projection = store.projection_snapshot().expect("projection snapshot");
        if has_index_entry {
            let (_claim_id, entry) = projection
                .fact_index_entries()
                .next()
                .expect("control fact index entry");
            let public_ref =
                public_ref_id(&internal_fact_ref_for_entry(entry).expect("control internal ref"))
                    .expect("control public ref-shaped id");
            let error = services
                .resolve_public_fact_ref(&public_ref)
                .await
                .expect_err("non-public refs resolve as not found");
            assert_eq!(error.class, ErrorClass::NotFound, "{case}");
            assert_eq!(error.code, "FactNotFound", "{case}");
        } else {
            assert!(
                projection.fact_records().next().is_some(),
                "{case} fact should still be retained internally"
            );
            assert!(
                projection.fact_index_entries().next().is_none(),
                "{case} fact should not be publicly indexed"
            );
        }
    }
}

#[derive(Clone)]
struct FakeFactQueryExecutor {
    rows: Vec<AppFactQueryRow>,
}

impl PublicFactQueryExecutor for FakeFactQueryExecutor {
    fn execute_public_fact_query_plan<'a>(
        &'a self,
        _plan: &'a mfm_facts::CanonicalFactQueryPlan,
    ) -> PublicFactQueryFuture<'a> {
        let rows = self.rows.clone();
        Box::pin(async move { Ok(rows) })
    }
}

#[tokio::test]
async fn public_fact_query_filters_non_public_refs_and_redacts_internal_fields() {
    let (_run_id, store, _registry) = launch_app_fact_run().await;
    let descriptor = AppLaunchFact::descriptor().expect("fact descriptor");
    let projection = store.projection_snapshot().expect("projection snapshot");
    let catalog = FactCatalogService::from_public_projection(vec![descriptor], &projection)
        .expect("public catalog");
    let (_claim_id, platform_entry) = projection
        .fact_index_entries()
        .next()
        .expect("platform fact index entry");
    let platform_ref = internal_fact_ref_for_entry(platform_entry).expect("platform internal ref");
    let platform_fields = returned_fields_for_entry(&projection, platform_entry);
    let mut control_entry = platform_entry.clone();
    control_entry.audience = mfm_facts::FactAudience::Control;
    let control_ref = internal_fact_ref_for_entry(&control_entry).expect("control ref");
    let executor = FakeFactQueryExecutor {
        rows: vec![
            AppFactQueryRow::new(platform_ref, platform_fields.clone()),
            AppFactQueryRow::new(control_ref, platform_fields),
        ],
    };

    let page = query_public_facts(
        &catalog,
        &executor,
        &mfm_facts::StoreScopeRef::new("mfm.store.default").expect("store scope"),
        &mfm_facts::ScopeDecisionEvidence::new(content_digest_for_bytes(
            b"mfm.public-facts.default-scope.v1",
        )),
        app_launch_fact_query_request(),
    )
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
            platform_entry.artifact_id.as_str(),
            platform_entry.artifact_evidence_hash.as_str(),
            platform_entry.fact_descriptor_hash.as_str(),
            platform_entry.subject_material_hash.as_str(),
        ],
    );
}

fn returned_fields_for_entry(
    projection: &store::ProjectionSnapshot,
    entry: &store::FactIndexProjection,
) -> Vec<mfm_facts::FactFieldValue> {
    projection
        .fact_term_entries()
        .filter(|((claim_id, _field_id), _term)| claim_id == &entry.fact_claim_id)
        .filter(|((_claim_id, field_id), _term)| {
            ["subject.amount", "result.amount"].contains(&field_id.as_str())
        })
        .map(|((_claim_id, _field_id), term)| {
            mfm_facts::FactFieldValue::new(
                term.field_id.clone(),
                term.value_type,
                term.value.clone(),
            )
            .expect("returned field")
        })
        .collect()
}

fn internal_fact_ref_for_entry(
    entry: &store::FactIndexProjection,
) -> Result<mfm_facts::InternalFactRef, AppError> {
    entry.internal_ref().map_err(AppError::from)
}

#[test]
fn certified_launch_stages_fact_descriptor_artifacts() {
    let request = prepare_app_fact_launch(true).expect("prepared launch");
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
fn certified_launch_rejects_missing_fact_descriptor_artifacts() {
    let error =
        prepare_app_fact_launch(false).expect_err("hash-only fact descriptor refs must not launch");

    assert_eq!(error.class, ErrorClass::Internal);
    assert_eq!(error.code, "FactDescriptorArtifactMissing");
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
            "policy_id": "primary",
        }),
        serde_json::json!({
            "network_id": "rest-control-eth",
            "expected_chain_id": 0,
            "observed_chain_id": 31338,
            "source_ref": "primary",
            "policy_id": "primary",
        }),
        serde_json::json!({
            "network_id": "rest-control-eth",
            "expected_chain_id": 31337,
            "observed_chain_id": 31337,
            "source_ref": "primary",
            "policy_id": "primary",
        }),
        serde_json::json!({
            "network_id": "rest-control-eth",
            "expected_chain_id": 31337,
            "observed_chain_id": 31338,
            "source_ref": "primary",
            "policy_id": "primary",
            "unexpected": true,
        }),
        serde_json::json!({
            "network_id": "rest-control-eth",
            "expected_chain_id": 31337,
            "observed_chain_id": 31338,
            "source_ref": "bad source ref",
            "policy_id": "primary",
        }),
    ] {
        let envelope = serde_json::json!({
            "kind": "provider",
            "version": 1,
            "details": {
                "diagnostic_kind": "provider_source_mismatch",
                "provider_family": "evm",
                "code": "source_mismatch",
                "operation": null,
                "fields": details,
            },
        });
        let expected = RuntimeDiagnostic::from_json(&envelope)
            .map(|diagnostic| {
                events::RedactedJson::new(
                    canonical_value_digest(&diagnostic.public_details_json())
                        .expect("diagnostic digest"),
                )
            })
            .unwrap_or_else(|_| {
                events::RedactedJson::new(
                    canonical_value_digest(&serde_json::Value::Null).expect("null digest"),
                )
            });
        let error = verify_replay_diagnostic_json(Some(&expected), &envelope)
            .expect_err("malformed typed diagnostic must fail replay");

        assert_eq!(error.code, "ReplayDiagnosticInvalid");
    }
}

#[test]
fn replay_diagnostic_accepts_generic_details_with_network_id() {
    let envelope = serde_json::json!({
        "kind": "provider",
        "version": 1,
        "details": {
            "diagnostic_kind": "provider_failure",
            "provider_family": "portfolio",
            "code": "response_invalid",
            "operation": null,
            "fields": {
                "domain_code": "missing_fact",
                "network_id": "ethereum-mainnet",
            },
        },
    });
    let diagnostic = RuntimeDiagnostic::from_json(&envelope).expect("typed diagnostic");
    let expected = events::RedactedJson::new(
        canonical_value_digest(&diagnostic.public_details_json()).expect("details digest"),
    );

    verify_replay_diagnostic_json(Some(&expected), &envelope)
        .expect("generic diagnostic details must not be classified as EVM mismatch evidence");
}

#[tokio::test]
async fn run_read_services_are_evidence_only() {
    let source = include_str!("../lib.rs");
    let production_read_constructor = source
        .split("pub async fn connect_production_run_read_services")
        .nth(1)
        .expect("production read constructor is present")
        .split("/// Builds the production typed runner registry")
        .next()
        .expect("production read constructor is bounded");
    assert!(!production_read_constructor.contains("production_runner_registry"));
    assert!(!production_read_constructor.contains("std::env"));
    assert!(production_read_constructor.contains("connect_production_run_read_store"));

    let read_services_impl = include_str!("../services.rs")
        .split("pub struct RunReadServices")
        .nth(1)
        .expect("read services are present")
        .split("/// Application facade for certified typed runtime dispatch.")
        .next()
        .expect("read services implementation is bounded");
    assert!(!read_services_impl.contains("production_runner_registry"));
    assert!(!read_services_impl.contains("std::env"));

    let store = store::AsyncInMemoryRunStore::default();
    let services = make_run_read_services(
        store.clone(),
        store,
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

#[test]
fn production_registry_certifies_btc_collector_descriptors() {
    let request =
        prepare_btc_collector_internal_test_launch().expect("btc collector certifies and prepares");

    assert_eq!(
        request.evidence.entry_point.resolved_op_id.as_str(),
        "mfm.bitcoin.btc_chain_head_collector_internal_test"
    );
    assert!(!request.evidence.config_artifacts.is_empty());
    assert!(!request.evidence.seed_cells.is_empty());
}

#[tokio::test]
async fn btc_collector_launch_defers_runtime_config_to_ingress() {
    let store = store::AsyncInMemoryRunStore::default();
    let request =
        prepare_btc_collector_internal_test_launch().expect("btc collector launch request");
    let run_id = request.run_id.clone();
    let fact_index = crate::ProjectionFactIndexProvider::new(store.clone());
    let runners = production_runner_registry(Arc::new(store.clone()), Arc::new(fact_index), None)
        .expect("production runners without BTC config");
    let services = make_run_services(
        runners,
        store.clone(),
        store.clone(),
        production_certification_registry().expect("production registry"),
    );

    let error = services
        .launch_run(request)
        .await
        .expect_err("missing BTC runtime config rejects at ingress before admission");

    assert_eq!(error.code, "LaunchRuntimeError");
    assert!(store
        .load_run_stream(&run_id)
        .await
        .expect("run stream")
        .is_empty());
}

#[tokio::test]
async fn launch_run_reaps_expired_execution_claim_and_retries_admission() {
    let store = store::AsyncInMemoryRunStore::default();
    let request = prepare_app_fact_launch(true).expect("prepared launch");
    let runtime_spec =
        CertifiedRuntimeSpec::new(request.certified_spec.clone()).expect("runtime spec");
    let runners = app_fact_runner_registry(
        &runtime_spec,
        mfm_program::facts::FactVisibility::indexed_default(
            mfm_program::facts::FactAudience::Platform,
        ),
    );
    let services = make_run_services(
        runners,
        store.clone(),
        store.clone(),
        app_fact_certification_registry(true),
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
    let error = match connect_production_run_store(Some(database_url)).await {
        Ok(_) => panic!("invalid postgres URL should not connect"),
        Err(error) => error,
    };

    assert_eq!(error.code, "RunStoreAuthorityInvalid");
    assert_eq!(error.message, "Run store authority could not be validated");
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
