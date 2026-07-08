use std::collections::{BTreeMap, VecDeque};
use std::future;
use std::sync::{Arc, Mutex};

use mfm_btc_capabilities::{
    BtcBlockHash, BtcCapabilityFuture, BtcChainHeadReadProvider, BtcChainHeadRequest,
    BtcChainHeadResponse, BtcSourceStatus, RedactedBtcSourceEvidence,
};
use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_events::v1::{ArtifactRole, KernelEventPayload};
use mfm_facts::{FactAudience, FactCanonicalScalar};
use mfm_ids::SeedId;
use mfm_integration_tests::test_support::{fact_query_evidences, InMemoryControlFactIndexProvider};
use mfm_op_btc_chain_head_collector::{
    btc_chain_head_collector_cycle_program_draft, BtcChainHeadCollectorConfig,
    BtcChainHeadObservationContext,
};
use mfm_program::CanonicalSeed;
use mfm_store::v1::{
    AsyncInMemoryRunStore, ProjectionSnapshot, RetainedArtifactReadProvider, RunEventStore,
    StoreScopeStore,
};
use tokio::sync::oneshot;

const FIRST_HASH: &str = "00000000000000000000000000000000000000000000000000000000000a0001";
const SECOND_HASH: &str = "00000000000000000000000000000000000000000000000000000000000a0002";

#[tokio::test]
async fn bitcoin_chain_head_collector_two_cycles_record_checkpoint_and_public_fact() {
    let store = AsyncInMemoryRunStore::default();
    let artifacts = Arc::new(store.clone());
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
    assert_collector_projection(&first_projection, 1, 1, &[850_000], &[850_000]);

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
    assert_collector_projection(&projection, 2, 2, &[850_001], &[850_001]);

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
        .query_public_facts(
            mfm_app::PublicFactQueryRequest::from_selector(
                "chain.head",
                mfm_app::PublicFactQuerySelector {
                    return_fields: vec![
                        "subject.network".to_owned(),
                        "subject.bitcoin_network".to_owned(),
                        "subject.head_kind".to_owned(),
                        "result.block_height".to_owned(),
                        "result.block_hash".to_owned(),
                    ],
                    ordering: Some("result.block_height.desc".to_owned()),
                    limit: Some(1),
                    ..mfm_app::PublicFactQuerySelector::default()
                },
            )
            .expect("public chain.head request"),
        )
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

#[tokio::test]
async fn bitcoin_chain_head_collector_recovers_interrupted_observation_without_partial_projection()
{
    let store = AsyncInMemoryRunStore::default();
    let artifacts = Arc::new(store.clone());
    let first_btc = Arc::new(MockBtcProvider::new(vec![MockHead {
        height: 850_000,
        hash: FIRST_HASH,
        provider_time_unix_ms: Some(1_720_000_000_000),
    }]));
    let fact_index = Arc::new(InMemoryControlFactIndexProvider::new(store.clone()));
    let first_services = collector_services(
        store.clone(),
        artifacts.clone(),
        first_btc.clone(),
        fact_index.clone(),
    );

    let first = launch_cycle(
        &first_services,
        &store,
        "recovery-cycle-1",
        BtcChainHeadCollectorConfig::default(),
    )
    .await;
    assert_eq!(first.run_mode, mfm_app::RunModeStatus::Completed);
    assert_eq!(first_btc.calls(), 1);
    assert_eq!(fact_index.returned_row_counts(), vec![0]);

    let (observe_called_tx, observe_called_rx) = oneshot::channel();
    let blocking_btc = Arc::new(BlockingBtcProvider::new(observe_called_tx));
    let blocking_services = collector_services(
        store.clone(),
        artifacts.clone(),
        blocking_btc.clone(),
        fact_index.clone(),
    );
    let request = collector_launch_request(
        &store,
        "recovery-cycle-2",
        BtcChainHeadCollectorConfig::default(),
    )
    .await;
    let interrupted_run_id = request.run_id.clone();
    let interrupted_services = blocking_services.clone();
    let launch_task = tokio::spawn(async move { interrupted_services.launch_run(request).await });
    observe_called_rx.await.expect("observe attempt called BTC");
    assert_eq!(blocking_btc.calls(), 1);

    let interrupted_stream = store
        .load_run_stream(&interrupted_run_id)
        .await
        .expect("interrupted stream");
    assert!(
        interrupted_stream.iter().any(|event| matches!(
            event.payload(),
            KernelEventPayload::StateAttemptStarted(payload)
                if payload.state_kind.as_str().contains("chain_head.observe")
        )),
        "observe attempt should have started before simulated crash"
    );
    let query_evidence = fact_query_evidences(&store, &interrupted_stream).await;
    assert_eq!(query_evidence.len(), 1);
    assert_eq!(query_evidence[0].receipt().returned_refs().len(), 1);
    assert_eq!(query_evidence[0].selection().selected_indices(), &[0]);
    assert_eq!(
        query_evidence[0].receipt().returned_refs()[0]
            .fact_kind()
            .as_str(),
        "collector.checkpoint"
    );
    assert_eq!(
        query_evidence[0].receipt().returned_refs()[0].visibility(),
        &mfm_facts::FactVisibility::indexed_default(FactAudience::Control)
    );

    let interrupted_projection = store.projection_snapshot().expect("interrupted projection");
    assert_collector_projection(&interrupted_projection, 1, 1, &[850_000], &[850_000]);

    launch_task.abort();
    let _ = launch_task.await;
    assert!(
        store
            .expire_execution_claim_for_test(&interrupted_run_id)
            .expect("expire interrupted claim"),
        "interrupted run should hold an execution claim"
    );

    let resumed_btc = Arc::new(MockBtcProvider::new(vec![MockHead {
        height: 850_001,
        hash: SECOND_HASH,
        provider_time_unix_ms: Some(1_720_000_600_000),
    }]));
    let resumed_services = collector_services(
        store.clone(),
        artifacts,
        resumed_btc.clone(),
        fact_index.clone(),
    );
    let resumed = resumed_services
        .resume_stored_run(&interrupted_run_id)
        .await
        .expect("resume interrupted collector run");
    assert_eq!(resumed.run_mode, mfm_app::RunModeStatus::Completed);
    assert_eq!(resumed_btc.calls(), 1);
    assert_eq!(fact_index.returned_row_counts(), vec![0, 1]);

    let resumed_stream = store
        .load_run_stream(&interrupted_run_id)
        .await
        .expect("resumed stream");
    assert_eq!(fact_query_evidences(&store, &resumed_stream).await.len(), 1);
    let projection = store.projection_snapshot().expect("resumed projection");
    assert_collector_projection(&projection, 2, 2, &[850_000, 850_001], &[850_000, 850_001]);
}

fn collector_services(
    store: AsyncInMemoryRunStore,
    artifacts: Arc<dyn RetainedArtifactReadProvider>,
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
    invocation_key: &str,
    config: BtcChainHeadCollectorConfig,
) -> mfm_app::RunResponse {
    let request = collector_launch_request(store, invocation_key, config).await;
    let response = services
        .launch_run(request)
        .await
        .unwrap_or_else(|error| panic!("{invocation_key} collector launch: {error:?}"))
        .into_response_parts()
        .1
        .expect("collector launch returns run");
    assert_completed_collector_launch(store, invocation_key, response).await
}

async fn collector_launch_request(
    store: &AsyncInMemoryRunStore,
    invocation_key: &str,
    config: BtcChainHeadCollectorConfig,
) -> mfm_app::RunLaunchRequest {
    let draft = btc_chain_head_collector_cycle_program_draft(config).expect("collector draft");
    let seed_material = collector_seed_material(&draft);
    let store_scope_id = store.load_store_scope_id().await.expect("store scope id");
    mfm_app::prepare_typed_program_run_launch_for_test(
        draft,
        seed_material,
        &mfm_app::production_certification_registry().expect("certification registry"),
        store_scope_id,
        Some(mfm_app::InvocationKey::new(invocation_key).expect("invocation key")),
    )
    .expect("prepared collector launch")
}

async fn assert_completed_collector_launch(
    store: &AsyncInMemoryRunStore,
    invocation_key: &str,
    response: mfm_app::RunResponse,
) -> mfm_app::RunResponse {
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
            "{invocation_key} collector run mode {:?}; failures={failures:?}",
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

fn assert_collector_projection(
    projection: &ProjectionSnapshot,
    expected_platform: usize,
    expected_control: usize,
    expected_chain_head_heights: &[u64],
    expected_checkpoint_heights: &[u64],
) {
    assert_fact_projection_counts(projection, expected_platform, expected_control);
    for expected in expected_chain_head_heights {
        assert_chain_head_height(projection, *expected);
    }
    for expected in expected_checkpoint_heights {
        assert_checkpoint_height(projection, *expected);
    }
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
                    "main",
                    BtcSourceStatus::Synced,
                )
                .expect("evidence"),
                head_kind: request.selection().head_kind(),
                finality: request.selection().finality(),
                block_height: head.height,
                block_hash: BtcBlockHash::new(head.hash).expect("block hash"),
                provider_time_unix_ms: head.provider_time_unix_ms,
            })
        })
    }
}

struct BlockingBtcProvider {
    called: Mutex<Option<oneshot::Sender<()>>>,
    calls: Mutex<usize>,
}

impl BlockingBtcProvider {
    fn new(called: oneshot::Sender<()>) -> Self {
        Self {
            called: Mutex::new(Some(called)),
            calls: Mutex::new(0),
        }
    }

    fn calls(&self) -> usize {
        *self.calls.lock().expect("blocking btc calls")
    }
}

impl BtcChainHeadReadProvider for BlockingBtcProvider {
    fn read_chain_head<'a>(
        &'a self,
        _request: &'a BtcChainHeadRequest,
    ) -> BtcCapabilityFuture<'a, BtcChainHeadResponse> {
        Box::pin(async move {
            *self.calls.lock().expect("blocking btc calls") += 1;
            if let Some(called) = self.called.lock().expect("blocking btc signal").take() {
                let _ = called.send(());
            }
            future::pending::<mfm_btc_capabilities::Result<BtcChainHeadResponse>>().await
        })
    }
}
