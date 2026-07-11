use super::*;
use mfm_btc_capabilities::{BtcBlockHash, RedactedBtcSourceEvidence};
use mfm_capabilities::CapabilitySpec;
use mfm_effects::EffectSpec;
use mfm_facts::{
    fact_descriptor_hash, CanonicalFactQueryPlan, DescriptorCatalogWatermark, FactClaimId,
    FactFieldExposure, FactFieldExtraction, FactFieldValueType, FactProducerProvenance,
    FactQueryReceipt, FactResponseEvidence, FactSubjectRef, InternalFactRef, InternalFactRefParts,
    StoreCommitOrder, StoreReadFrontier, StoreReadFrontierType,
};
use mfm_ids::{
    ArtifactId, CapabilityKind, CapabilityVersion, ContentDigest, DigestBytes, EventId, RunId,
    SchemaId,
};

fn poll_ready<F>(future: F) -> F::Output
where
    F: std::future::Future,
{
    let waker = std::task::Waker::noop();
    let mut cx = std::task::Context::from_waker(waker);
    let mut future = std::pin::pin!(future);
    match std::future::Future::poll(future.as_mut(), &mut cx) {
        std::task::Poll::Ready(output) => output,
        std::task::Poll::Pending => panic!("test future unexpectedly pending"),
    }
}

fn observe_config() -> ObserveBtcChainHeadConfig {
    ObserveBtcChainHeadConfig {
        network: "bitcoin-mainnet".to_owned(),
        bitcoin_network: "main".to_owned(),
        semantic_source_identity: "public-bitcoin-core".to_owned(),
        head_kind: "best".to_owned(),
        confirmation_depth: None,
        max_source_reads: NonZeroU64::new(1).expect("nonzero"),
    }
}

fn capability_response() -> CapabilityChainHeadResponse {
    let config = observe_config();
    CapabilityChainHeadResponse {
        evidence: RedactedBtcSourceEvidence {
            network_id: BtcNetworkId::new(&config.network).expect("network"),
            source_identity: BtcSourceIdentity::new(&config.semantic_source_identity)
                .expect("source"),
            bitcoin_network: config.bitcoin_network.clone(),
            observed_bitcoin_network: config.bitcoin_network.clone(),
            source_status: BtcSourceStatus::Synced,
        },
        head_kind: BtcHeadKind::Best,
        finality: BtcFinality::BestAvailable,
        block_height: 850_000,
        block_hash: BtcBlockHash::new(
            "00000000000000000001b2a7f3e0d5c4b6a897887766554433221100ffeeddcc",
        )
        .expect("block hash"),
        provider_time_unix_ms: Some(1_720_000_000_000),
    }
}

fn observe_input(checkpoint: Option<CollectorCheckpointFact>) -> ObserveBtcChainHeadInput {
    ObserveBtcChainHeadInput {
        loaded_checkpoint: LoadedCollectorCheckpoint::new(checkpoint),
        context: BtcChainHeadObservationContext {
            observed_at_unix_ms: Some(1_720_000_001_000),
        },
    }
}

fn checkpoint_query_config() -> QueryCollectorCheckpointConfig {
    QueryCollectorCheckpointConfig {
        collector_kind: "btc-chain-head".to_owned(),
        semantic_source_identity: "public-bitcoin-core".to_owned(),
        partition: "chain-head".to_owned(),
        network: "bitcoin-mainnet".to_owned(),
        bitcoin_network: "main".to_owned(),
        store_scope: "mfm.store.default".to_owned(),
    }
}

fn checkpoint_record_config() -> RecordCollectorCheckpointConfig {
    RecordCollectorCheckpointConfig {
        collector_kind: "btc-chain-head".to_owned(),
        partition: "chain-head".to_owned(),
    }
}

#[test]
fn chain_head_fact_descriptor_declares_platform_subject_and_result_fields() {
    let descriptor = BtcChainHeadFact::descriptor().expect("descriptor");

    assert_eq!(descriptor.fact_kind().as_str(), "chain.head");
    assert!(descriptor.fields().iter().any(|field| {
        field.field_id().as_str() == "subject.semantic_source_identity"
            && matches!(field.extraction(), FactFieldExtraction::Subject(_))
            && field.exposure() == FactFieldExposure::QueryOnly
    }));
    assert!(descriptor.fields().iter().any(|field| {
        field.field_id().as_str() == "result.block_height"
            && field.value_type() == FactFieldValueType::UnsignedInteger
            && field.sortable()
    }));
    assert!(descriptor.fields().iter().any(|field| {
        field.field_id().as_str() == "metadata.observed_at"
            && matches!(field.extraction(), FactFieldExtraction::Metadata(_))
            && !field.required()
    }));
    assert_eq!(
        descriptor.orderings()[0].name().as_str(),
        "result.block_height.desc"
    );
}

#[test]
fn collector_checkpoint_descriptor_declares_control_subject_and_high_watermark() {
    let descriptor = CollectorCheckpointFact::descriptor().expect("descriptor");

    assert_eq!(descriptor.fact_kind().as_str(), "collector.checkpoint");
    for field_id in [
        "subject.collector_kind",
        "subject.semantic_source_identity",
        "subject.scope",
        "subject.partition",
        "subject.network",
        "subject.bitcoin_network",
    ] {
        assert!(
            descriptor
                .fields()
                .iter()
                .any(|field| field.field_id().as_str() == field_id
                    && matches!(field.extraction(), FactFieldExtraction::Subject(_))),
            "missing subject field {field_id}"
        );
    }
    assert!(descriptor.fields().iter().any(|field| {
        field.field_id().as_str() == "result.high_watermark_height"
            && field.value_type() == FactFieldValueType::UnsignedInteger
            && field.exposure() == FactFieldExposure::Returnable
            && field.sortable()
    }));
    assert!(descriptor.fields().iter().any(|field| {
        field.field_id().as_str() == "result.predecessor_checkpoint_hash"
            && field.value_type() == FactFieldValueType::Digest
            && field.exposure() == FactFieldExposure::Hidden
            && !field.required()
    }));
}

#[test]
fn normalized_chain_head_fact_material_contains_no_floats_or_runtime_routes() {
    let config = observe_config();
    let response = capability_response();
    let observation = normalize_chain_head_response(&config, &response, &observe_input(None))
        .expect("observation");
    let fact = observation.to_fact();
    let value = serde_json::to_value(&fact).expect("json");

    assert_no_float_numbers(&value);
    let text = serde_json::to_string(&value).expect("json text");
    for forbidden in [
        "rpc_url",
        "auth_header",
        "username",
        "password",
        "runtime_source_ref",
        "http://",
        "https://",
    ] {
        assert!(
            !text.contains(forbidden),
            "fact material leaked forbidden token {forbidden}: {text}"
        );
    }
    assert!(text.contains("public-bitcoin-core"));
}

#[test]
fn checkpoint_query_request_is_control_latest_checkpoint_plan() {
    let config = checkpoint_query_config();
    let request = config.request().expect("request");
    let plan = request.plan();
    let descriptor_hash =
        fact_descriptor_hash(&CollectorCheckpointFact::descriptor().expect("descriptor"))
            .expect("descriptor hash");

    assert_eq!(plan.query_scope().audience(), FactAudience::Control);
    assert_eq!(plan.query_scope().scope(), FactVisibilityScope::Default);
    assert_eq!(plan.resolved_descriptor(), &descriptor_hash);
    assert_eq!(
        plan.ordering().name().as_str(),
        "result.high_watermark_height.desc"
    );
    assert_eq!(plan.limit(), Some(1));
    assert_eq!(plan.store_scope().as_str(), "mfm.store.default");

    let query: serde_json::Value =
        serde_json::from_slice(plan.canonical_query().as_bytes()).expect("query json");
    assert_eq!(query["fact_kind"], "collector.checkpoint");
    assert!(query.get("audience").is_none());
    assert!(query.get("scope").is_none());
    assert_eq!(query["limit"], 1);
    assert_eq!(query["ordering"], "result.high_watermark_height.desc");
    assert_eq!(query["return_fields"][0], "result.high_watermark_height");
    let parsed_shape = mfm_facts::parse_canonical_fact_query_shape(plan).expect("query shape");
    assert_eq!(parsed_shape.return_fields().len(), 1);
    assert!(query["predicates"]
        .as_array()
        .expect("predicates")
        .iter()
        .any(
            |predicate| predicate["field_id"] == "subject.semantic_source_identity"
                && predicate["value"] == "public-bitcoin-core"
        ));
    assert!(query["predicates"]
        .as_array()
        .expect("predicates")
        .iter()
        .any(|predicate| predicate["field_id"] == "subject.partition"
            && predicate["value"] == "chain-head"));
}

#[test]
fn source_identity_configs_reject_runtime_routes_without_leaking_them() {
    let observe_config = ObserveBtcChainHeadConfig {
        semantic_source_identity: "http://user:password@localhost:8332".to_owned(),
        ..observe_config()
    };
    let query_config = QueryCollectorCheckpointConfig {
        semantic_source_identity: "http://user:password@localhost:8332".to_owned(),
        ..checkpoint_query_config()
    };

    for (case, error) in [
        (
            "observe config",
            validate_observe_chain_head_config(&observe_config).expect_err("route"),
        ),
        (
            "checkpoint query config",
            validate_query_collector_checkpoint_config(&query_config).expect_err("route"),
        ),
    ] {
        assert!(
            !error.contains("localhost:8332"),
            "{case} leaked route host: {error}"
        );
        assert!(
            !error.contains("password"),
            "{case} leaked route password: {error}"
        );
    }
}

#[test]
fn checkpoint_query_materializes_empty_and_single_row_receipts() {
    let config = checkpoint_query_config();
    let state = QueryCollectorCheckpointState::new(
        ValidatedConfig::new(config.clone()).expect("validated config"),
    )
    .expect("state");
    let request = config.request().expect("request");

    let empty_response = fact_query_result(fact_query_receipt(request.plan(), Vec::new()));
    let empty = state
        .materialize_response(&empty_response, None)
        .expect("empty materialization");
    assert!(empty.checkpoint().is_none());
    assert_eq!(
        state
            .selection_evidence(&empty_response)
            .expect("empty selection")
            .selected_indices(),
        &[] as &[u64]
    );

    let checkpoint = checkpoint_fact(850_000);
    let single_response = fact_query_result(fact_query_receipt(
        request.plan(),
        vec![internal_fact_ref(1)],
    ));
    let loaded = state
        .materialize_response(&single_response, Some(checkpoint.clone()))
        .expect("single materialization");
    assert_eq!(loaded.checkpoint(), Some(&checkpoint));
    assert_eq!(
        state
            .selection_evidence(&single_response)
            .expect("single selection")
            .selected_indices(),
        &[0]
    );
    assert!(state.materialize_response(&single_response, None).is_err());
}

#[test]
fn observe_rejects_invalid_loaded_checkpoints() {
    let response = capability_response();
    let state =
        ObserveBtcChainHeadState::new(ValidatedConfig::new(observe_config()).expect("config"))
            .expect("state");

    for (case, checkpoint, expected_message) in [
        (
            "incompatible checkpoint source",
            checkpoint_fact_for_source(
                849_999,
                "different-semantic-source",
                "bitcoin-mainnet",
                BtcFinality::BestAvailable,
            ),
            "incompatible with requested Bitcoin source",
        ),
        (
            "head behind checkpoint",
            checkpoint_fact(850_001),
            "observed chain head must not be behind loaded checkpoint",
        ),
    ] {
        let error = state
            .materialize_response(&observe_input(Some(checkpoint)), &response)
            .expect_err(case);

        assert!(
            error.to_string().contains(expected_message),
            "{case} produced unexpected error: {error}"
        );
    }
}

#[test]
fn record_states_transform_typed_upstream_outputs() {
    let config = observe_config();
    let response = capability_response();
    let observation = normalize_chain_head_response(&config, &response, &observe_input(None))
        .expect("observation");

    let chain_head_state = RecordBtcChainHeadFactState::new(
        ValidatedConfig::new(RecordBtcChainHeadFactConfig {}).expect("config"),
    )
    .expect("state");
    let caps = (FactRecordCapability,);
    let context = mfm_program::CertifiedContext::no_context();
    let fact = poll_ready(chain_head_state.run(
        RecordBtcChainHeadFactInput {
            observation: observation.clone(),
        },
        &caps,
        &context,
    ))
    .expect("chain head fact");
    assert_eq!(fact.subject(), observation.subject());
    assert_eq!(fact.response(), observation.response());

    let checkpoint_state = RecordCollectorCheckpointState::new(
        ValidatedConfig::new(checkpoint_record_config()).expect("config"),
    )
    .expect("state");
    let checkpoint = poll_ready(checkpoint_state.run(
        RecordCollectorCheckpointInput {
            chain_head_fact: fact.clone(),
            loaded_checkpoint: LoadedCollectorCheckpoint::new(None),
        },
        &caps,
        &context,
    ))
    .expect("checkpoint fact");

    assert_eq!(checkpoint.subject().collector_kind(), "btc-chain-head");
    assert_eq!(checkpoint.subject().partition(), "chain-head");
    assert_eq!(
        checkpoint.response().high_watermark_height(),
        observation.response().block_height()
    );
    assert_eq!(
        checkpoint.response().high_watermark_hash(),
        fact.response().block_hash()
    );
}

#[test]
fn checkpoint_record_uses_recorded_fact_and_loaded_checkpoint() {
    let config = observe_config();
    let response = capability_response();
    let observation = normalize_chain_head_response(&config, &response, &observe_input(None))
        .expect("observation");
    let fact = observation.to_fact();
    let previous = checkpoint_fact(849_999);
    let state = RecordCollectorCheckpointState::new(
        ValidatedConfig::new(checkpoint_record_config()).expect("config"),
    )
    .expect("state");

    let caps = (FactRecordCapability,);
    let context = mfm_program::CertifiedContext::no_context();
    let checkpoint = poll_ready(state.run(
        RecordCollectorCheckpointInput {
            chain_head_fact: fact,
            loaded_checkpoint: LoadedCollectorCheckpoint::new(Some(previous.clone())),
        },
        &caps,
        &context,
    ))
    .expect("checkpoint");

    assert_eq!(checkpoint.response().high_watermark_height(), 850_000);
    assert_eq!(
        checkpoint.response().predecessor_checkpoint_hash(),
        Some(checkpoint_material_hash(&previous).expect("hash").as_str())
    );
}

#[test]
fn checkpoint_record_rejects_incompatible_loaded_checkpoint() {
    let config = observe_config();
    let response = capability_response();
    let observation = normalize_chain_head_response(&config, &response, &observe_input(None))
        .expect("observation");
    let state = RecordCollectorCheckpointState::new(
        ValidatedConfig::new(checkpoint_record_config()).expect("config"),
    )
    .expect("state");

    let caps = (FactRecordCapability,);
    let context = mfm_program::CertifiedContext::no_context();
    let error = poll_ready(state.run(
        RecordCollectorCheckpointInput {
            chain_head_fact: observation.to_fact(),
            loaded_checkpoint: LoadedCollectorCheckpoint::new(Some(checkpoint_fact_for_source(
                849_999,
                "different-semantic-source",
                "bitcoin-mainnet",
                BtcFinality::BestAvailable,
            ))),
        },
        &caps,
        &context,
    ))
    .expect_err("incompatible checkpoint");

    assert!(error
        .to_string()
        .contains("incompatible with recorded chain-head fact"));
}

#[test]
fn checkpoint_material_contains_no_floats_or_runtime_routes() {
    let subject = CollectorCheckpointSubject::new(
        "btc-chain-head",
        &BtcSourceIdentity::new("public-bitcoin-core").expect("source"),
        "chain-head",
        "main",
        &BtcNetworkId::new("bitcoin-mainnet").expect("network"),
    )
    .expect("checkpoint subject");
    let response = CollectorCheckpointResponse::new(
        850_000,
        "00000000000000000001b2a7f3e0d5c4b6a897887766554433221100ffeeddcc",
        Some("internal-fact-ref".to_owned()),
        Some(
            "content:sha256-jcs-v1:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                .to_owned(),
        ),
        BtcFinality::BestAvailable,
    );
    let value =
        serde_json::to_value(CollectorCheckpointFact::new(subject, response)).expect("json");

    assert_no_float_numbers(&value);
    let text = serde_json::to_string(&value).expect("json text");
    assert!(!text.contains("http://"));
    assert!(!text.contains("password"));
    assert!(text.contains("\"scope\":\"default\""));
}

#[test]
fn state_error_from_capability_provider_failure_is_redacted() {
    let diagnostic = mfm_btc_capabilities::btc_diagnostic(
        mfm_capabilities::ProviderDiagnosticCode::TransportFailed,
    );
    let error = BtcStateError::from(BtcCapabilityError::provider_failure(diagnostic));
    let text = error.to_string();

    assert_eq!(error, BtcStateError::ProviderFailed);
    assert!(!text.contains("password"));
    assert!(!text.contains("localhost"));
    assert!(!text.contains("http://"));
}

#[test]
fn record_states_advertise_only_their_fact_descriptors() {
    let observe =
        ObserveBtcChainHeadState::emitted_fact_descriptors().expect("observe descriptors");
    let query =
        QueryCollectorCheckpointState::emitted_fact_descriptors().expect("query descriptors");
    let chain_head =
        RecordBtcChainHeadFactState::emitted_fact_descriptors().expect("chain head descriptors");
    let checkpoint =
        RecordCollectorCheckpointState::emitted_fact_descriptors().expect("checkpoint descriptors");

    assert!(observe.is_empty());
    assert!(query.is_empty());
    assert_eq!(chain_head.len(), 1);
    assert_eq!(checkpoint.len(), 1);
    assert_ne!(chain_head[0].descriptor_hash, checkpoint[0].descriptor_hash);
}

#[test]
fn query_state_declares_fact_index_read_capability() {
    let descriptor =
        mfm_program::state_descriptor::<QueryCollectorCheckpointState>().expect("descriptor");

    assert_eq!(descriptor.effect().class, ReadExternal::class());
    assert_eq!(descriptor.capabilities().len(), 1);
    assert_eq!(
        descriptor.capabilities().capabilities[0].name.as_str(),
        mfm_fact_capabilities::FactIndexReadCapability::name()
    );
}

#[test]
fn adapter_bound_states_advertise_btc_jsonrpc_adapter() {
    let expected_kind = btc_jsonrpc_adapter_kind().expect("adapter kind");
    let expected_version = btc_jsonrpc_adapter_version().expect("adapter version");

    for bindings in [
        ObserveBtcChainHeadState::adapter_bindings().expect("observe bindings"),
        RecordBtcChainHeadFactState::adapter_bindings().expect("chain head record bindings"),
        RecordCollectorCheckpointState::adapter_bindings().expect("checkpoint record bindings"),
    ] {
        assert_eq!(bindings.len(), 1);
        assert_eq!(bindings[0].adapter_kind, expected_kind);
        assert_eq!(bindings[0].adapter_version, expected_version);
    }
}

#[test]
fn state_crate_manifest_stays_inside_state_boundaries() {
    let manifest = include_str!("../Cargo.toml");

    for forbidden in [
        "mfm-app",
        "mfm-runtime",
        "mfm-store",
        "mfm-stream-store-postgres",
        "mfm-btc-jsonrpc-http",
        "mfm-adapters-btc-jsonrpc",
        "mfm-op-btc-collectors",
    ] {
        assert!(
            !manifest.contains(forbidden),
            "state crate must not depend on forbidden boundary crate {forbidden}"
        );
    }
}

fn checkpoint_fact(height: u64) -> CollectorCheckpointFact {
    checkpoint_fact_for_source(
        height,
        "public-bitcoin-core",
        "bitcoin-mainnet",
        BtcFinality::BestAvailable,
    )
}

fn checkpoint_fact_for_source(
    height: u64,
    source_identity: &str,
    network: &str,
    finality: BtcFinality,
) -> CollectorCheckpointFact {
    let subject = CollectorCheckpointSubject::new(
        "btc-chain-head",
        &BtcSourceIdentity::new(source_identity).expect("source"),
        "chain-head",
        "main",
        &BtcNetworkId::new(network).expect("network"),
    )
    .expect("checkpoint subject");
    let response = CollectorCheckpointResponse::new(
        height,
        "00000000000000000001b2a7f3e0d5c4b6a897887766554433221100ffeeddcc",
        None,
        None,
        finality,
    );
    CollectorCheckpointFact::new(subject, response)
}

fn fact_query_receipt(
    _plan: &CanonicalFactQueryPlan,
    returned_refs: Vec<InternalFactRef>,
) -> FactQueryReceipt {
    let rows = returned_refs
        .into_iter()
        .map(|fact_ref| mfm_facts::FactQueryResultRow::new(fact_ref, Vec::new()))
        .collect::<Vec<_>>();
    let read_frontier = StoreReadFrontier::new(
        StoreScopeRef::new("mfm.store.default").expect("store scope"),
        FactQueryScope::new(FactAudience::Control, FactVisibilityScope::Default),
        DescriptorCatalogWatermark::new(1),
        StoreCommitOrder::new(11),
    );
    FactQueryReceipt::from_rows(
        read_frontier,
        StoreReadFrontierType::Snapshot,
        &rows,
        false,
        None,
    )
    .expect("receipt")
}

fn fact_query_result(receipt: FactQueryReceipt) -> mfm_facts::FactQueryResult {
    mfm_facts::FactQueryResult::new(
        mfm_facts::fact_query_result_rows_from_receipt(&receipt),
        receipt,
    )
    .expect("fact query result")
}

fn internal_fact_ref(seed: u8) -> InternalFactRef {
    InternalFactRef::new(InternalFactRefParts {
        fact_claim_id: FactClaimId::new(run_id(seed), 9, 0).expect("claim id"),
        source_event_id: EventId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(seed + 1)),
        recorded_at: "2026-07-02T00:00:00Z".to_owned(),
        producer_node_id: mfm_ids::NodeId::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            digest_bytes(seed + 2),
        ),
        observed_at: Some("2026-07-02T00:00:00Z".to_owned()),
        visibility: FactVisibility::indexed_default(FactAudience::Control),
        fact_kind: mfm_facts::FactKind::new("collector.checkpoint").expect("kind"),
        fact_descriptor_hash: digest(seed + 2),
        subject: FactSubjectRef::new(
            digest(seed + 3),
            mfm_facts::FactKey::from_digest(digest(seed + 4)),
            digest(seed + 5),
        ),
        request: None,
        response: FactResponseEvidence::new(
            schema_id(seed + 6),
            digest(seed + 7),
            ArtifactId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(seed + 8)),
            digest(seed + 9),
        ),
        producer: FactProducerProvenance::new(
            CapabilityKind::new(
                "mfm.fact",
                "index.read",
                DigestAlgorithm::Sha256JcsV1,
                digest_bytes(seed + 10),
            )
            .expect("capability kind"),
            CapabilityVersion::new("mfm.fact.index.read.v1").expect("capability version"),
            AdapterKind::new(
                "mfm.fact",
                "index.adapter",
                DigestAlgorithm::Sha256JcsV1,
                digest_bytes(seed + 11),
            )
            .expect("adapter kind"),
            AdapterVersion::new("mfm.fact.index.adapter.v1").expect("adapter version"),
        ),
    })
    .expect("fact ref")
}

fn schema_id(seed: u8) -> SchemaId {
    SchemaId::new(
        "mfm.fact.test",
        "v1",
        DigestAlgorithm::Sha256JcsV1,
        digest_bytes(seed),
    )
    .expect("schema id")
}

fn run_id(seed: u8) -> RunId {
    RunId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(seed))
}

fn digest(seed: u8) -> ContentDigest {
    ContentDigest::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(seed))
}

fn digest_bytes(seed: u8) -> DigestBytes {
    DigestBytes::from_array([seed; 32])
}

fn assert_no_float_numbers(value: &serde_json::Value) {
    match value {
        serde_json::Value::Array(values) => {
            for value in values {
                assert_no_float_numbers(value);
            }
        }
        serde_json::Value::Object(values) => {
            for value in values.values() {
                assert_no_float_numbers(value);
            }
        }
        serde_json::Value::Number(number) => {
            assert!(!number.is_f64(), "float-like number found: {number}");
        }
        serde_json::Value::Null | serde_json::Value::Bool(_) | serde_json::Value::String(_) => {}
    }
}
