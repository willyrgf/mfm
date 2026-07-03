use std::cmp::Ordering;
use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};

use ed25519_dalek::{Signer, SigningKey};
use mfm_artifact_capabilities::ArtifactReadProvider;
use mfm_btc_capabilities::{
    BtcBlockHash, BtcCapabilityFuture, BtcChainHeadReadProvider, BtcChainHeadRequest,
    BtcChainHeadResponse, BtcSourceStatus, RedactedBtcSourceEvidence,
};
use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_events::v1::{ArtifactRole, KernelEventPayload};
use mfm_fact_capabilities::{
    FactIndexReadProvider, FactIndexReadRequest, FactIndexReadResponse,
    FactQueryReceiptTrustRootMaterial,
};
use mfm_facts::{
    DescriptorCatalogWatermark, FactAudience, FactCanonicalScalar, FactClaimId,
    FactProjectionGeneration, FactQueryOperator, InternalFactRef, InternalFactRefParts,
    NullOrdering, QueryResultCardinality, ReturnedFactFieldSummary, ReturnedFieldSummaries,
    ReturnedFieldValueSummary, StoreCommitWatermark, StoreIdentity, StoreKeyId, StoreReadFrontier,
    StoreReadFrontierType, StoreReceiptAuthentication, StoreReceiptAuthenticationScheme,
};
use mfm_ids::SeedId;
use mfm_op_btc_chain_head_collector::{
    btc_chain_head_collector_cycle_program_draft, BtcChainHeadCollectorConfig,
    BtcChainHeadObservationContext, QueryCollectorCheckpointContext,
};
use mfm_program::CanonicalSeed;
use mfm_store::v1::{AsyncInMemoryRunStore, ProjectionSnapshot, RunEventStore, TrustScopeStore};

const FIRST_HASH: &str = "00000000000000000000000000000000000000000000000000000000000a0001";
const SECOND_HASH: &str = "00000000000000000000000000000000000000000000000000000000000a0002";

#[tokio::test]
async fn bitcoin_chain_head_collector_two_cycles_record_checkpoint_and_public_fact() {
    let store = AsyncInMemoryRunStore::default();
    let artifacts = mfm_app::artifact_read_provider_from_retained(store.clone());
    let btc = Arc::new(MockBtcProvider::new(vec![
        MockHead {
            height: 850_000,
            hash: FIRST_HASH,
            provider_time_unix_ms: Some(1_720_000_000_000),
        },
        MockHead {
            height: 850_001,
            hash: SECOND_HASH,
            provider_time_unix_ms: Some(1_720_000_600_000),
        },
    ]));
    let fact_index = Arc::new(InMemoryControlFactIndexProvider::new(store.clone()));
    let services = collector_services(store.clone(), artifacts, btc.clone(), fact_index.clone());

    let first = launch_cycle(
        &services,
        &store,
        "cycle-1",
        BtcChainHeadCollectorConfig::default(),
    )
    .await;
    assert_eq!(first.run_mode, mfm_app::RunModeStatus::Completed);
    assert_eq!(btc.calls(), 1);
    assert_eq!(fact_index.returned_row_counts(), vec![0]);

    let first_projection = store.projection_snapshot().expect("first projection");
    assert_fact_projection_counts(&first_projection, 1, 1);
    assert_chain_head_height(&first_projection, 850_000);
    assert_checkpoint_height(&first_projection, 850_000);

    let second = launch_cycle(
        &services,
        &store,
        "cycle-2",
        BtcChainHeadCollectorConfig::default(),
    )
    .await;
    assert_eq!(second.run_mode, mfm_app::RunModeStatus::Completed);
    assert_eq!(btc.calls(), 2);
    assert_eq!(fact_index.returned_row_counts(), vec![0, 1]);

    let second_stream = store
        .load_run_stream(&second.run_id.parse().expect("run id"))
        .await
        .expect("second run stream");
    assert!(second_stream.iter().any(|event| matches!(
        event.payload(),
        KernelEventPayload::ArtifactReferenced(payload)
            if payload.artifact_ref.role == ArtifactRole::FactQueryEvidence
    )));

    let projection = store.projection_snapshot().expect("projection");
    assert_fact_projection_counts(&projection, 2, 2);
    assert_chain_head_height(&projection, 850_001);
    assert_checkpoint_height(&projection, 850_001);

    let btc_calls_before_replay = btc.calls();
    let fact_index_reads_before_replay = fact_index.returned_row_counts();
    let replay = services
        .verify_replay_for_run(&second.run_id.parse().expect("second run id"))
        .await
        .expect("second cycle replay");
    assert_eq!(replay.run_mode, mfm_app::RunModeStatus::Completed);
    assert_eq!(btc.calls(), btc_calls_before_replay);
    assert_eq!(
        fact_index.returned_row_counts(),
        fact_index_reads_before_replay
    );

    let read_services = mfm_app::make_run_read_services_with_certification_registry(
        store.clone(),
        store.clone(),
        mfm_app::production_certification_registry().expect("certification registry"),
    );
    assert_eq!(
        read_services.fact_kinds().await.expect("fact kinds"),
        vec![mfm_app::PublicFactKindSummary {
            fact_kind: "chain.head".to_owned(),
            descriptor_count: 1,
        }]
    );
    assert!(read_services
        .describe_fact_kind("chain.head")
        .await
        .expect("chain.head descriptor")
        .iter()
        .any(|descriptor| descriptor.fact_kind == "chain.head"));
    assert_eq!(
        read_services
            .describe_fact_kind("collector.checkpoint")
            .await
            .expect_err("control facts are not public")
            .code,
        "FactNotFound"
    );

    let page = read_services
        .query_public_facts(mfm_app::PublicFactQueryRequest {
            fact_kind: "chain.head".to_owned(),
            shape: None,
            predicates: Vec::new(),
            return_fields: vec![
                "subject.chain".to_owned(),
                "subject.network".to_owned(),
                "subject.head_kind".to_owned(),
                "result.block_height".to_owned(),
                "result.block_hash".to_owned(),
            ],
            ordering: "result.block_height.desc".to_owned(),
            limit: Some(1),
        })
        .await
        .expect("public chain.head query");
    assert_eq!(page.facts.len(), 1);
    assert_eq!(page.facts[0].fact_kind, "chain.head");
    assert!(page.facts[0].fields.iter().any(|field| {
        field.field_id == "result.block_height"
            && field.value == mfm_app::PublicFactScalarValue::UnsignedInteger(850_001)
    }));
    assert!(!format!("{:?}", page).contains("collector.checkpoint"));
}

fn collector_services(
    store: AsyncInMemoryRunStore,
    artifacts: Arc<dyn ArtifactReadProvider>,
    btc: Arc<dyn BtcChainHeadReadProvider>,
    fact_index: Arc<InMemoryControlFactIndexProvider>,
) -> mfm_app::RunServices<AsyncInMemoryRunStore, AsyncInMemoryRunStore> {
    let receipt_trust_root = fact_index.receipt_trust_root();
    let capabilities =
        mfm_adapters_btc_jsonrpc::BtcJsonRpcRunnerCapabilities::new(artifacts, btc, fact_index);
    let mut runners = mfm_runtime::ErasedRunnerRegistry::new();
    mfm_adapters_btc_jsonrpc::register_btc_jsonrpc_runners(&mut runners, capabilities)
        .expect("btc runners");
    let runtime_artifacts = Arc::new(store.clone());
    mfm_app::RunServices::new_with_certification_registry_and_fact_query_trust_root(
        mfm_runtime::SerialTypedScheduler::new(runners, runtime_artifacts),
        store.clone(),
        store,
        mfm_app::production_certification_registry().expect("certification registry"),
        Some(receipt_trust_root),
    )
}

async fn launch_cycle(
    services: &mfm_app::RunServices<AsyncInMemoryRunStore, AsyncInMemoryRunStore>,
    store: &AsyncInMemoryRunStore,
    distinct_key: &str,
    config: BtcChainHeadCollectorConfig,
) -> mfm_app::RunResponse {
    let draft = btc_chain_head_collector_cycle_program_draft(config).expect("collector draft");
    let seed_material = collector_seed_material(&draft);
    let trust_scope_id = store.load_trust_scope_id().await.expect("trust scope id");
    let request = mfm_app::prepare_typed_program_run_launch_for_test(
        draft,
        seed_material,
        &mfm_app::production_certification_registry().expect("certification registry"),
        trust_scope_id,
        Some(mfm_app::DistinctRunKey::new(distinct_key).expect("distinct key")),
    )
    .expect("prepared collector launch");
    let response = services
        .launch_run(request)
        .await
        .unwrap_or_else(|error| panic!("{distinct_key} collector launch: {error:?}"))
        .into_response_parts()
        .1;
    if response.run_mode != mfm_app::RunModeStatus::Completed {
        let run_id = response.run_id.parse().expect("run id");
        let stream = store.load_run_stream(&run_id).await.expect("run stream");
        let failures = stream
            .iter()
            .filter_map(|event| match event.payload() {
                KernelEventPayload::StateAttemptFailed(payload) => Some(format!(
                    "{} {} {}",
                    payload.node_id, payload.error.code, payload.error.safe_message
                )),
                _ => None,
            })
            .collect::<Vec<_>>();
        panic!(
            "{distinct_key} collector run mode {:?}; failures={failures:?}",
            response.run_mode
        );
    }
    response
}

fn collector_seed_material(
    draft: &mfm_program::TypedProgramDraft,
) -> BTreeMap<SeedId, PlainCanonicalJsonBytes> {
    draft
        .seeds()
        .iter()
        .map(|seed| {
            let bytes = match seed.key.as_str() {
                "query_context" => CanonicalSeed::from_value(&QueryCollectorCheckpointContext {
                    queried_at_unix_ms: None,
                })
                .expect("query seed")
                .canonical_json()
                .clone(),
                "observation_context" => {
                    CanonicalSeed::from_value(&BtcChainHeadObservationContext {
                        observed_at_unix_ms: None,
                    })
                    .expect("observation seed")
                    .canonical_json()
                    .clone()
                }
                other => panic!("unexpected collector seed {other}"),
            };
            (seed.seed_id.clone(), bytes)
        })
        .collect()
}

fn assert_fact_projection_counts(
    projection: &ProjectionSnapshot,
    expected_platform: usize,
    expected_control: usize,
) {
    let platform = projection
        .fact_index_entries()
        .filter(|(_claim_id, entry)| {
            entry.fact_kind.as_str() == "chain.head" && entry.audience == FactAudience::Platform
        })
        .count();
    let control = projection
        .fact_index_entries()
        .filter(|(_claim_id, entry)| {
            entry.fact_kind.as_str() == "collector.checkpoint"
                && entry.audience == FactAudience::Control
        })
        .count();
    assert_eq!(platform, expected_platform);
    assert_eq!(control, expected_control);
}

fn assert_chain_head_height(projection: &ProjectionSnapshot, expected: u64) {
    assert!(projection.fact_term_entries().any(|(_key, term)| {
        term.field_id.as_str() == "result.block_height"
            && term.value == FactCanonicalScalar::UnsignedInteger(expected)
    }));
}

fn assert_checkpoint_height(projection: &ProjectionSnapshot, expected: u64) {
    assert!(projection.fact_term_entries().any(|(_key, term)| {
        term.field_id.as_str() == "result.high_watermark_height"
            && term.value == FactCanonicalScalar::UnsignedInteger(expected)
    }));
}

#[derive(Debug, Clone)]
struct MockHead {
    height: u64,
    hash: &'static str,
    provider_time_unix_ms: Option<u64>,
}

struct MockBtcProvider {
    heads: Mutex<VecDeque<MockHead>>,
    calls: Mutex<usize>,
}

impl MockBtcProvider {
    fn new(heads: Vec<MockHead>) -> Self {
        Self {
            heads: Mutex::new(heads.into()),
            calls: Mutex::new(0),
        }
    }

    fn calls(&self) -> usize {
        *self.calls.lock().expect("btc calls")
    }
}

impl BtcChainHeadReadProvider for MockBtcProvider {
    fn read_chain_head<'a>(
        &'a self,
        request: &'a BtcChainHeadRequest,
    ) -> BtcCapabilityFuture<'a, BtcChainHeadResponse> {
        Box::pin(async move {
            *self.calls.lock().expect("btc calls") += 1;
            let head = self
                .heads
                .lock()
                .expect("btc heads")
                .pop_front()
                .expect("mock head");
            Ok(BtcChainHeadResponse {
                evidence: RedactedBtcSourceEvidence::from_request(
                    request,
                    Some("main".to_owned()),
                    BtcSourceStatus::Synced,
                ),
                block_height: head.height,
                block_hash: BtcBlockHash::new(head.hash).expect("block hash"),
                provider_time_unix_ms: head.provider_time_unix_ms,
            })
        })
    }
}

struct InMemoryControlFactIndexProvider {
    store: AsyncInMemoryRunStore,
    signing_key: SigningKey,
    store_identity: StoreIdentity,
    key_id: StoreKeyId,
    returned_row_counts: Mutex<Vec<usize>>,
}

impl InMemoryControlFactIndexProvider {
    fn new(store: AsyncInMemoryRunStore) -> Self {
        Self {
            store,
            signing_key: SigningKey::from_bytes(&[0x43; 32]),
            store_identity: StoreIdentity::new("mfm.integration.in_memory")
                .expect("store identity"),
            key_id: StoreKeyId::new("integration.fact.read").expect("store key id"),
            returned_row_counts: Mutex::new(Vec::new()),
        }
    }

    fn returned_row_counts(&self) -> Vec<usize> {
        self.returned_row_counts.lock().expect("row counts").clone()
    }
}

impl FactIndexReadProvider for InMemoryControlFactIndexProvider {
    fn read_fact_index<'a>(
        &'a self,
        request: &'a FactIndexReadRequest,
    ) -> mfm_fact_capabilities::FactIndexReadFuture<'a> {
        Box::pin(async move {
            let projection = self.store.projection_snapshot().expect("projection");
            let shape =
                mfm_facts::parse_canonical_fact_query_shape(request.plan()).expect("query shape");
            let mut rows = projection
                .fact_index_entries()
                .filter(|(_claim_id, entry)| {
                    entry.fact_descriptor_hash == *request.plan().resolved_descriptor()
                        && entry.audience == request.plan().query_scope().audience()
                        && entry.visibility_scope == request.plan().query_scope().scope()
                })
                .filter(|(_claim_id, entry)| entry_matches(&projection, entry, &shape))
                .map(|(_claim_id, entry)| {
                    let record = projection
                        .fact_record(&entry.fact_claim_id)
                        .expect("fact record");
                    let fact_ref = internal_fact_ref_from_projection(entry, record)?;
                    let returned_fields = returned_fields_from_projection(
                        &projection,
                        entry.fact_claim_id.clone(),
                        shape.return_fields(),
                    )?;
                    Ok((fact_ref, returned_fields))
                })
                .collect::<mfm_facts::Result<Vec<_>>>()
                .expect("fact rows");
            rows.sort_by(|left, right| {
                compare_fact_rows(&projection, request.plan(), &left.0, &right.0)
            });
            if let Some(limit) = request.plan().limit() {
                rows.truncate(limit as usize);
            }
            self.returned_row_counts
                .lock()
                .expect("row counts")
                .push(rows.len());
            let response_rows = rows
                .iter()
                .map(|(fact_ref, returned_fields)| {
                    mfm_fact_capabilities::FactIndexReadRow::new(
                        fact_ref.clone(),
                        returned_fields.clone(),
                    )
                })
                .collect::<Vec<_>>();
            let returned_refs = rows
                .iter()
                .map(|(fact_ref, _fields)| fact_ref.clone())
                .collect::<Vec<_>>();
            let returned_field_summaries = (!shape.return_fields().is_empty()).then(|| {
                ReturnedFieldSummaries::new(
                    rows.iter()
                        .map(|(fact_ref, fields)| {
                            ReturnedFactFieldSummary::new(
                                fact_ref.fact_claim_id().clone(),
                                fields.clone(),
                            )
                        })
                        .collect(),
                )
            });
            let receipt = self
                .signed_receipt(
                    request.plan(),
                    &projection,
                    returned_refs,
                    returned_field_summaries,
                )
                .expect("signed receipt");
            let trust_root = mfm_store::v1::FactQueryReceiptTrustRoot::new(
                self.store_identity.clone(),
                StoreReceiptAuthenticationScheme::LocalEd25519Sha256JcsV1,
                self.key_id.clone(),
                self.signing_key.verifying_key().to_bytes(),
            )
            .expect("trust root");
            let plan_hash = mfm_facts::fact_query_plan_hash(request.plan()).expect("plan hash");
            mfm_store::v1::verify_fact_query_receipt_authentication(
                &plan_hash,
                &receipt,
                &trust_root,
            )
            .expect("receipt authentication");
            Ok(
                FactIndexReadResponse::new(response_rows, receipt, self.trust_root())
                    .expect("fact index response"),
            )
        })
    }
}

impl InMemoryControlFactIndexProvider {
    fn trust_root(&self) -> FactQueryReceiptTrustRootMaterial {
        FactQueryReceiptTrustRootMaterial::new(
            self.store_identity.clone(),
            StoreReceiptAuthenticationScheme::LocalEd25519Sha256JcsV1,
            self.key_id.clone(),
            self.signing_key.verifying_key().to_bytes(),
        )
    }

    fn receipt_trust_root(&self) -> mfm_store::v1::FactQueryReceiptTrustRoot {
        mfm_store::v1::FactQueryReceiptTrustRoot::new(
            self.store_identity.clone(),
            StoreReceiptAuthenticationScheme::LocalEd25519Sha256JcsV1,
            self.key_id.clone(),
            self.signing_key.verifying_key().to_bytes(),
        )
        .expect("receipt trust root")
    }

    fn signed_receipt(
        &self,
        plan: &mfm_facts::CanonicalFactQueryPlan,
        projection: &ProjectionSnapshot,
        returned_refs: Vec<InternalFactRef>,
        returned_field_summaries: Option<ReturnedFieldSummaries>,
    ) -> mfm_fact_capabilities::Result<mfm_facts::FactQueryReceipt> {
        let result_cardinality = match plan.limit() {
            Some(limit) if returned_refs.len() as u64 == limit => {
                QueryResultCardinality::AtLeast(returned_refs.len() as u64)
            }
            _ => QueryResultCardinality::Exact(returned_refs.len() as u64),
        };
        let result_set_digest = mfm_facts::fact_query_result_set_digest(
            &returned_refs,
            returned_field_summaries.as_ref(),
        )
        .map_err(mfm_fact_capabilities::FactIndexReadError::redacted_provider_failure)?;
        let max_order = projection
            .fact_index_entries()
            .filter(|(_claim_id, entry)| {
                entry.audience == plan.query_scope().audience()
                    && entry.visibility_scope == plan.query_scope().scope()
            })
            .map(|(_claim_id, entry)| entry.store_commit_order)
            .max()
            .unwrap_or_default();
        let read_frontier = StoreReadFrontier::new(
            plan.store_scope().clone(),
            plan.query_scope().clone(),
            DescriptorCatalogWatermark::new(projection.fact_descriptors().count() as u64),
            FactProjectionGeneration::new(1),
            max_order,
            StoreCommitWatermark::new(max_order),
        );
        let plan_hash = mfm_facts::fact_query_plan_hash(plan)
            .map_err(mfm_fact_capabilities::FactIndexReadError::redacted_provider_failure)?;
        let receipt_hash = mfm_facts::fact_query_receipt_body_hash_from_parts(
            &plan_hash,
            &read_frontier,
            StoreReadFrontierType::Snapshot,
            &returned_refs,
            returned_field_summaries.as_ref(),
            &result_set_digest,
            result_cardinality,
        )
        .map_err(mfm_fact_capabilities::FactIndexReadError::redacted_provider_failure)?;
        let message = mfm_store::v1::fact_query_receipt_authentication_message(
            &self.store_identity,
            StoreReceiptAuthenticationScheme::LocalEd25519Sha256JcsV1,
            &self.key_id,
            &receipt_hash,
        )
        .map_err(mfm_fact_capabilities::FactIndexReadError::redacted_provider_failure)?;
        let auth = StoreReceiptAuthentication::new(
            self.store_identity.clone(),
            StoreReceiptAuthenticationScheme::LocalEd25519Sha256JcsV1,
            Some(self.key_id.clone()),
            self.signing_key
                .sign(message.as_bytes())
                .to_bytes()
                .to_vec(),
        )
        .map_err(mfm_fact_capabilities::FactIndexReadError::redacted_provider_failure)?;
        Ok(mfm_facts::FactQueryReceipt::new(
            read_frontier,
            StoreReadFrontierType::Snapshot,
            returned_refs,
            returned_field_summaries,
            result_set_digest,
            result_cardinality,
            receipt_hash,
            auth,
        ))
    }
}

fn entry_matches(
    projection: &ProjectionSnapshot,
    entry: &mfm_store::v1::FactIndexProjection,
    shape: &mfm_facts::CompiledFactQueryShape,
) -> bool {
    shape.predicates().iter().all(|predicate| {
        projection
            .fact_term_entries()
            .find(|((claim_id, field_id), _term)| {
                claim_id == &entry.fact_claim_id && field_id == predicate.field_id()
            })
            .is_some_and(|(_key, term)| {
                scalar_matches_operator(&term.value, predicate.operator(), predicate.value())
            })
    })
}

fn scalar_matches_operator(
    left: &FactCanonicalScalar,
    operator: FactQueryOperator,
    right: &FactCanonicalScalar,
) -> bool {
    if left.value_type() != right.value_type() {
        return false;
    }
    match operator {
        FactQueryOperator::Equal => left == right,
        FactQueryOperator::LessThan => left < right,
        FactQueryOperator::LessThanOrEqual => left <= right,
        FactQueryOperator::GreaterThan => left > right,
        FactQueryOperator::GreaterThanOrEqual => left >= right,
    }
}

fn returned_fields_from_projection(
    projection: &ProjectionSnapshot,
    claim_id: FactClaimId,
    return_fields: &[mfm_facts::FactQueryReturnField],
) -> mfm_facts::Result<Vec<ReturnedFieldValueSummary>> {
    return_fields
        .iter()
        .filter_map(|return_field| {
            projection
                .fact_term_entries()
                .find(|((term_claim_id, field_id), _term)| {
                    term_claim_id == &claim_id && field_id == return_field.field_id()
                })
                .map(|(_key, term)| {
                    ReturnedFieldValueSummary::new(
                        term.field_id.clone(),
                        term.value_type,
                        term.value.clone(),
                    )
                })
        })
        .collect()
}

fn compare_fact_rows(
    projection: &ProjectionSnapshot,
    plan: &mfm_facts::CanonicalFactQueryPlan,
    left: &InternalFactRef,
    right: &InternalFactRef,
) -> Ordering {
    for term in plan.ordering().terms() {
        let left_value = fact_ordering_value(projection, left.fact_claim_id(), term.field_id());
        let right_value = fact_ordering_value(projection, right.fact_claim_id(), term.field_id());
        let ordering = compare_optional_scalars(left_value, right_value, term.nulls());
        let ordering = match term.direction() {
            mfm_facts::SortDirection::Ascending => ordering,
            mfm_facts::SortDirection::Descending => ordering.reverse(),
        };
        if ordering != Ordering::Equal {
            return ordering;
        }
    }
    left.fact_claim_id().cmp(right.fact_claim_id())
}

fn fact_ordering_value<'a>(
    projection: &'a ProjectionSnapshot,
    claim_id: &FactClaimId,
    field_id: &mfm_facts::FactFieldId,
) -> Option<&'a FactCanonicalScalar> {
    projection
        .fact_term_entries()
        .find(|((term_claim_id, term_field_id), _term)| {
            term_claim_id == claim_id && term_field_id == field_id
        })
        .map(|(_key, term)| &term.value)
}

fn compare_optional_scalars(
    left: Option<&FactCanonicalScalar>,
    right: Option<&FactCanonicalScalar>,
    nulls: NullOrdering,
) -> Ordering {
    match (left, right) {
        (Some(left), Some(right)) => left.cmp(right),
        (None, None) => Ordering::Equal,
        (None, Some(_)) => match nulls {
            NullOrdering::First => Ordering::Less,
            NullOrdering::Last => Ordering::Greater,
        },
        (Some(_), None) => match nulls {
            NullOrdering::First => Ordering::Greater,
            NullOrdering::Last => Ordering::Less,
        },
    }
}

fn internal_fact_ref_from_projection(
    entry: &mfm_store::v1::FactIndexProjection,
    record: &mfm_store::v1::FactRecordProjection,
) -> mfm_facts::Result<InternalFactRef> {
    InternalFactRef::new(InternalFactRefParts {
        fact_claim_id: entry.fact_claim_id.clone(),
        source_event_id: entry.source_event_id.clone(),
        recorded_at: entry.recorded_at.clone(),
        producer_node_id: record.node_id.clone(),
        observed_at: entry.observed_at.clone(),
        visibility: mfm_facts::FactVisibility::Indexed {
            audience: entry.audience,
            scope: entry.visibility_scope,
        },
        fact_kind: entry.fact_kind.clone(),
        fact_descriptor_hash: entry.fact_descriptor_hash.clone(),
        fact_subject_namespace_hash: entry.fact_subject_namespace_hash.clone(),
        fact_key: entry.fact_key.clone(),
        subject_material_hash: entry.subject_material_hash.clone(),
        request_schema_id: entry.request_schema_id.clone(),
        request_hash: entry.request_hash.clone(),
        response_schema_id: entry.response_schema_id.clone(),
        response_hash: entry.response_hash.clone(),
        artifact_id: entry.artifact_id.clone(),
        artifact_evidence_hash: entry.artifact_evidence_hash.clone(),
        capability_kind: entry.capability_kind.clone(),
        capability_version: entry.capability_version.clone(),
        adapter_kind: entry.adapter_kind.clone(),
        adapter_version: entry.adapter_version.clone(),
    })
}
