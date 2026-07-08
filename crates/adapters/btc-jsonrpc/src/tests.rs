use super::*;
use ed25519_dalek::SigningKey;
use mfm_artifact_capabilities::ArtifactEvidenceRef;
use mfm_btc_capabilities::{
    BtcAddress, BtcBalanceReadProvider, BtcBalanceReadRequest, BtcBlockHash, BtcChainGuard,
    BtcFinality, BtcHeadSelection, BtcNetworkId, BtcSourceIdentity, BtcSourceStatus,
    RedactedBtcSourceEvidence,
};
use mfm_canonical::sha256_digest_bytes;
use mfm_facts::{
    fact_descriptor_hash, DescriptorCatalogWatermark, FactAudience, FactClaimId,
    FactProducerProvenance, FactProjectionGeneration, FactQueryReceipt, FactQueryScope,
    FactResponseEvidence, FactSelectionEvidence, FactSubjectRef, FactVisibility,
    FactVisibilityScope, InternalFactRef, InternalFactRefParts, StoreCommitWatermark,
    StoreIdentity, StoreKeyId, StoreReadFrontier, StoreScopeRef,
};
use mfm_ids::{
    AdapterKind, AdapterVersion, ArtifactId, CapabilityKind, CapabilityVersion, ContentDigest,
    DigestAlgorithm, DigestBytes, EventId, RunId,
};
use mfm_spec::v1::MediaType;
use mfm_states_btc::{
    CollectorCheckpointSubject, RecordCollectorCheckpointConfig, RecordCollectorCheckpointInput,
};
use mfm_store::v1::{
    self as store,
    test_support::{signed_fact_query_receipt_for_test, SignedFactQueryReceiptFixtureInputForTest},
};
use mfm_transports_btc_jsonrpc_http::{
    BlockHeaderInfo, BlockchainInfo, BtcJsonRpcChainHeadProvider, BtcJsonRpcChainHeadTransport,
    BtcRpcError, BtcTransportFuture, ScanTxOutSetResult,
};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

const BEST_HASH: &str = "00000000000000000001b2a7f3e0d5c4b6a897887766554433221100ffeeddcc";
const CONFIRMED_HASH: &str = "00000000000000000002b2a7f3e0d5c4b6a897887766554433221100ffeeddcc";

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

#[derive(Default)]
struct MockTransport {
    calls: Mutex<Vec<String>>,
    info_failure: Option<MockInfoFailure>,
}

impl MockTransport {
    fn calls(&self) -> Vec<String> {
        self.calls.lock().expect("calls").clone()
    }
}

#[derive(Clone, Copy)]
enum MockInfoFailure {
    Http,
    HttpStatus,
    JsonRpc,
}

struct MockFactIndex {
    calls: Mutex<usize>,
    refs: Vec<InternalFactRef>,
    fail: bool,
}

impl MockFactIndex {
    fn empty() -> Self {
        Self {
            calls: Mutex::new(0),
            refs: Vec::new(),
            fail: false,
        }
    }

    fn with_refs(refs: Vec<InternalFactRef>) -> Self {
        Self {
            calls: Mutex::new(0),
            refs,
            fail: false,
        }
    }

    fn failing() -> Self {
        Self {
            calls: Mutex::new(0),
            refs: Vec::new(),
            fail: true,
        }
    }

    fn calls(&self) -> usize {
        *self.calls.lock().expect("calls")
    }
}

impl FactIndexReadProvider for MockFactIndex {
    fn read_fact_index<'a>(
        &'a self,
        request: &'a mfm_fact_capabilities::FactIndexReadRequest,
    ) -> mfm_fact_capabilities::FactIndexReadFuture<'a> {
        Box::pin(async move {
            *self.calls.lock().expect("calls") += 1;
            if self.fail {
                return Err(
                    mfm_fact_capabilities::FactIndexReadError::redacted_provider_failure(
                        "postgres://user:password@localhost/mfm Authorization: Bearer secret",
                    ),
                );
            }
            Ok(mfm_fact_capabilities::FactIndexReadResponse::from_receipt(
                fact_query_receipt(request.plan(), self.refs.clone()),
                trust_root(),
            ))
        })
    }
}

#[derive(Default)]
struct MockArtifacts {
    calls: Mutex<Vec<ArtifactId>>,
    artifact: Option<(Vec<u8>, ArtifactEvidenceRef)>,
}

impl MockArtifacts {
    fn with_checkpoint_response(response: &CollectorCheckpointResponse) -> (Self, InternalFactRef) {
        let (bytes, evidence, fact_ref) = checkpoint_artifact_and_ref(response, 1);
        (
            Self {
                calls: Mutex::new(Vec::new()),
                artifact: Some((bytes, evidence)),
            },
            fact_ref,
        )
    }

    fn calls(&self) -> Vec<ArtifactId> {
        self.calls.lock().expect("calls").clone()
    }
}

fn checkpoint_artifact_and_ref(
    response: &CollectorCheckpointResponse,
    seed: u8,
) -> (Vec<u8>, ArtifactEvidenceRef, InternalFactRef) {
    let bytes = serde_json::to_vec(response).expect("response json");
    let digest =
        ContentDigest::from_digest(DigestAlgorithm::Sha256JcsV1, sha256_digest_bytes(&bytes));
    let artifact_id = ArtifactId::from_digest(digest.algorithm(), *digest.digest());
    let producer_node_id = producer_node_id(seed);
    let evidence = checkpoint_response_evidence(
        bytes.len() as u64,
        artifact_id.clone(),
        digest.clone(),
        producer_node_id.clone(),
    );
    let fact_ref = internal_fact_ref(
        seed,
        artifact_id,
        digest,
        producer_node_id,
        artifact_evidence_hash(&evidence),
    );
    (bytes, evidence, fact_ref)
}

impl store::RetainedArtifactReadProvider for MockArtifacts {
    fn read_retained_artifact<'a>(
        &'a self,
        requirement: &'a store::EventArtifactRequirement,
    ) -> store::RetainedArtifactReadFuture<'a> {
        Box::pin(async move {
            self.calls
                .lock()
                .expect("calls")
                .push(requirement.artifact_id.clone());
            let Some((bytes, evidence)) = &self.artifact else {
                return Err(store::StoreError::MissingArtifact {
                    artifact_id: requirement.artifact_id.clone(),
                });
            };
            store::VerifiedRunArtifactBytes::new(
                bytes.clone(),
                evidence.clone().into(),
                requirement,
            )
        })
    }
}

impl BtcJsonRpcChainHeadTransport for MockTransport {
    fn get_blockchain_info<'a>(&'a self) -> BtcTransportFuture<'a, BlockchainInfo> {
        Box::pin(async move {
            self.calls.lock().expect("calls").push("info".to_owned());
            if let Some(failure) = self.info_failure {
                return Err(match failure {
                    MockInfoFailure::Http => BtcRpcError::Http(
                        concat!(
                            "http://user:password@localhost:8332 ",
                            "Authorization: Bearer secret"
                        )
                        .to_owned(),
                    ),
                    MockInfoFailure::HttpStatus => BtcRpcError::HttpStatus {
                        status: 403,
                        body_len: Some(64),
                        content_type: Some("text/plain".to_owned()),
                    },
                    MockInfoFailure::JsonRpc => BtcRpcError::JsonRpcError {
                        code: -32601,
                        message: "secret provider message".to_owned(),
                    },
                });
            }
            Ok(BlockchainInfo {
                blocks: 850_000,
                bestblockhash: BEST_HASH.to_owned(),
                chain: "main".to_owned(),
                initialblockdownload: Some(false),
            })
        })
    }

    fn get_block_hash<'a>(&'a self, height: u64) -> BtcTransportFuture<'a, String> {
        Box::pin(async move {
            self.calls
                .lock()
                .expect("calls")
                .push(format!("hash:{height}"));
            Ok(CONFIRMED_HASH.to_owned())
        })
    }

    fn get_block_header<'a>(
        &'a self,
        block_hash: &'a str,
    ) -> BtcTransportFuture<'a, BlockHeaderInfo> {
        Box::pin(async move {
            self.calls
                .lock()
                .expect("calls")
                .push(format!("header:{block_hash}"));
            let height = if block_hash == CONFIRMED_HASH {
                849_994
            } else {
                850_000
            };
            Ok(BlockHeaderInfo {
                hash: block_hash.to_owned(),
                height,
                time: 1_720_000_000,
            })
        })
    }

    fn scan_tx_out_set<'a>(
        &'a self,
        address: &'a str,
    ) -> BtcTransportFuture<'a, ScanTxOutSetResult> {
        Box::pin(async move {
            self.calls
                .lock()
                .expect("calls")
                .push(format!("scan:{address}"));
            Ok(ScanTxOutSetResult {
                success: true,
                height: 850_000,
                bestblock: BEST_HASH.to_owned(),
                total_amount_sats: 123_456_789,
                unspents: Vec::new(),
            })
        })
    }
}

fn make_request(selection: BtcHeadSelection) -> BtcChainHeadRequest {
    BtcChainHeadRequest {
        guard: BtcChainGuard::new(
            BtcNetworkId::new("bitcoin-mainnet").expect("network"),
            BtcSourceIdentity::new("public-bitcoin-core").expect("source"),
            "main",
        )
        .expect("guard"),
        selection,
    }
}

fn make_balance_request(address: &str) -> BtcBalanceReadRequest {
    BtcBalanceReadRequest {
        guard: BtcChainGuard::new(
            BtcNetworkId::new("bitcoin-mainnet").expect("network"),
            BtcSourceIdentity::new("public-bitcoin-core").expect("source"),
            "main",
        )
        .expect("guard"),
        address: BtcAddress::new(address).expect("address"),
        block_height: 850_000,
        block_hash: BtcBlockHash::new(BEST_HASH).expect("hash"),
    }
}

fn chain_head_response(
    request: &BtcChainHeadRequest,
    block_height: u64,
    block_hash: &str,
    provider_time_unix_ms: Option<u64>,
) -> BtcChainHeadResponse {
    BtcChainHeadResponse {
        evidence: RedactedBtcSourceEvidence::from_request(request, "main", BtcSourceStatus::Synced)
            .expect("evidence"),
        head_kind: request.selection.head_kind(),
        finality: request.selection.finality(),
        block_height,
        block_hash: BtcBlockHash::new(block_hash).expect("hash"),
        provider_time_unix_ms,
    }
}

fn routed_provider(transport: Arc<MockTransport>) -> BtcJsonRpcChainHeadProvider {
    let mut routes = BTreeMap::new();
    routes.insert(
        BtcSourceIdentity::new("public-bitcoin-core").expect("source"),
        transport as Arc<dyn BtcJsonRpcChainHeadTransport>,
    );
    BtcJsonRpcChainHeadProvider::new(routes)
}

fn observe_input(
    checkpoint: Option<CollectorCheckpointFact>,
    observed_at_unix_ms: Option<u64>,
) -> ObserveBtcChainHeadInput {
    ObserveBtcChainHeadInput {
        loaded_checkpoint: LoadedCollectorCheckpoint::new(checkpoint),
        context: mfm_states_btc::BtcChainHeadObservationContext {
            observed_at_unix_ms,
        },
    }
}

fn checkpoint_fact(height: u64) -> CollectorCheckpointFact {
    checkpoint_fact_for_source(height, "public-bitcoin-core")
}

fn checkpoint_fact_for_source(
    height: u64,
    semantic_source_identity: &str,
) -> CollectorCheckpointFact {
    let source_identity = BtcSourceIdentity::new(semantic_source_identity).expect("source");
    let network = BtcNetworkId::new("bitcoin-mainnet").expect("network");
    let subject = CollectorCheckpointSubject::new(
        "btc-chain-head",
        &source_identity,
        "chain-head",
        "main",
        &network,
    )
    .expect("checkpoint subject");
    let response =
        CollectorCheckpointResponse::new(height, BEST_HASH, None, None, BtcFinality::BestAvailable);
    CollectorCheckpointFact::new(subject, response)
}

fn checkpoint_record_config() -> RecordCollectorCheckpointConfig {
    RecordCollectorCheckpointConfig {
        collector_kind: "btc-chain-head".to_owned(),
        partition: "chain-head".to_owned(),
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

fn checkpoint_query_input() -> QueryCollectorCheckpointInput {
    QueryCollectorCheckpointInput {}
}

#[tokio::test]
async fn query_runner_empty_result_pins_evidence_and_returns_no_checkpoint() {
    let config = checkpoint_query_config();
    let provider = MockFactIndex::empty();
    let artifacts = MockArtifacts::default();

    let (loaded, evidence) = query_collector_checkpoint(
        ValidatedConfig::new(config).expect("config"),
        checkpoint_query_input(),
        &artifacts,
        &provider,
    )
    .await
    .expect("query checkpoint");

    assert_eq!(provider.calls(), 1);
    assert!(artifacts.calls().is_empty());
    assert!(loaded.checkpoint().is_none());
    assert!(evidence
        .query_evidence()
        .receipt()
        .returned_refs()
        .is_empty());
    assert_eq!(
        evidence.query_evidence().selection().selected_indices(),
        &[] as &[u64]
    );
    fact_query_trust_root(evidence.trust_root()).expect("store trust root");
}

#[tokio::test]
async fn query_runner_single_row_pins_evidence_and_returns_checkpoint() {
    let config = checkpoint_query_config();
    let response = CollectorCheckpointResponse::new(
        850_000,
        BEST_HASH,
        Some("previous-checkpoint".to_owned()),
        Some("previous-checkpoint-hash".to_owned()),
        BtcFinality::BestAvailable,
    );
    let (artifacts, fact_ref) = MockArtifacts::with_checkpoint_response(&response);
    let provider = MockFactIndex::with_refs(vec![fact_ref.clone()]);

    let (loaded, evidence) = query_collector_checkpoint(
        ValidatedConfig::new(config).expect("config"),
        checkpoint_query_input(),
        &artifacts,
        &provider,
    )
    .await
    .expect("query checkpoint");

    let checkpoint = loaded.checkpoint().expect("checkpoint");
    assert_eq!(provider.calls(), 1);
    assert_eq!(artifacts.calls(), vec![fact_ref.artifact_id().clone()]);
    assert_eq!(checkpoint.response().high_watermark_height(), 850_000);
    assert_eq!(checkpoint.response().high_watermark_hash(), BEST_HASH);
    assert_eq!(
        evidence.query_evidence().receipt().returned_refs(),
        &[fact_ref]
    );
    assert_eq!(
        evidence.query_evidence().selection().selected_indices(),
        &[0]
    );
}

#[tokio::test]
async fn query_runner_provider_errors_are_redacted() {
    let config = checkpoint_query_config();
    let provider = MockFactIndex::failing();
    let artifacts = MockArtifacts::default();

    let error = query_collector_checkpoint(
        ValidatedConfig::new(config).expect("config"),
        checkpoint_query_input(),
        &artifacts,
        &provider,
    )
    .await
    .expect_err("provider failure");
    let rendered = format!("{error:?} {error}");

    assert_eq!(provider.calls(), 1);
    assert!(artifacts.calls().is_empty());
    assert!(!rendered.contains("localhost"));
    assert!(!rendered.contains("password"));
    assert!(!rendered.contains("secret"));
    assert!(!rendered.contains("Authorization"));
}

#[test]
fn checkpoint_replay_helper_uses_recorded_evidence_without_provider() {
    let config = checkpoint_query_config();
    let response = CollectorCheckpointResponse::new(
        850_001,
        BEST_HASH,
        None,
        None,
        BtcFinality::BestAvailable,
    );
    let (_, _, fact_ref) = checkpoint_artifact_and_ref(&response, 2);
    let plan = config.request().expect("request").plan().clone();
    let selection = FactSelectionEvidence::new(digest(0x71), vec![0], None).expect("selection");
    let evidence = mfm_facts::FactQueryEvidence::new(
        plan.clone(),
        fact_query_receipt(&plan, vec![fact_ref]),
        selection,
    );

    let loaded = replay_loaded_checkpoint_from_evidence(&config, &evidence, Some(response.clone()))
        .expect("replay checkpoint");

    assert_eq!(
        loaded.checkpoint().expect("checkpoint").response(),
        &response
    );
}

fn fact_query_receipt(
    plan: &mfm_facts::CanonicalFactQueryPlan,
    returned_refs: Vec<InternalFactRef>,
) -> FactQueryReceipt {
    let key = signing_key();
    let store_identity = StoreIdentity::new("store.default").expect("store identity");
    let key_id = StoreKeyId::new("fact.read.key").expect("key id");
    let read_frontier = StoreReadFrontier::new(
        StoreScopeRef::new("mfm.store.default").expect("store scope"),
        FactQueryScope::new(FactAudience::Control, FactVisibilityScope::Default),
        DescriptorCatalogWatermark::new(1),
        FactProjectionGeneration::new(1),
        11,
        StoreCommitWatermark::new(11),
    );
    let plan_hash = mfm_facts::fact_query_plan_hash(plan).expect("plan hash");
    let rows = returned_refs
        .into_iter()
        .map(|fact_ref| mfm_facts::FactQueryResultRow::new(fact_ref, Vec::new()))
        .collect::<Vec<_>>();
    signed_fact_query_receipt_for_test(SignedFactQueryReceiptFixtureInputForTest {
        plan_hash: &plan_hash,
        key: &key,
        store_identity,
        key_id,
        read_frontier,
        rows: &rows,
        include_returned_field_summaries: false,
        limit: None,
    })
}

fn signing_key() -> SigningKey {
    SigningKey::from_bytes(&[7; 32])
}

fn trust_root() -> FactQueryReceiptTrustRootMaterial {
    let key = signing_key();
    FactQueryReceiptTrustRootMaterial::new(
        StoreIdentity::new("store.default").expect("store identity"),
        mfm_facts::StoreReceiptAuthenticationScheme::LocalEd25519Sha256JcsV1,
        StoreKeyId::new("fact.read.key").expect("key id"),
        key.verifying_key().to_bytes(),
    )
}

fn internal_fact_ref(
    seed: u8,
    artifact_id: ArtifactId,
    response_hash: ContentDigest,
    producer_node_id: mfm_ids::NodeId,
    artifact_evidence_hash: ContentDigest,
) -> InternalFactRef {
    let descriptor_hash =
        fact_descriptor_hash(&CollectorCheckpointFact::descriptor().expect("descriptor"))
            .expect("descriptor hash");
    InternalFactRef::new(InternalFactRefParts {
        fact_claim_id: FactClaimId::new(run_id(seed), 9, 0).expect("claim id"),
        source_event_id: EventId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(seed + 1)),
        recorded_at: "2026-07-02T00:00:00Z".to_owned(),
        producer_node_id,
        observed_at: Some("2026-07-02T00:00:00Z".to_owned()),
        visibility: FactVisibility::indexed_default(FactAudience::Control),
        fact_kind: mfm_facts::FactKind::new("collector.checkpoint").expect("kind"),
        fact_descriptor_hash: descriptor_hash,
        subject: FactSubjectRef::new(
            digest(seed + 3),
            mfm_facts::FactKey::from_digest(digest(seed + 4)),
            digest(seed + 5),
        ),
        request: None,
        response: FactResponseEvidence::new(
            CollectorCheckpointResponse::schema_id().expect("schema"),
            response_hash,
            artifact_id,
            artifact_evidence_hash,
        ),
        producer: FactProducerProvenance::new(
            CapabilityKind::new(
                "mfm.bitcoin",
                "chain_head.read",
                DigestAlgorithm::Sha256JcsV1,
                digest_bytes(seed + 10),
            )
            .expect("capability kind"),
            CapabilityVersion::new("mfm.bitcoin.chain_head.read.v1").expect("capability version"),
            AdapterKind::new(
                "mfm.bitcoin",
                "jsonrpc",
                DigestAlgorithm::Sha256JcsV1,
                digest_bytes(seed + 11),
            )
            .expect("adapter kind"),
            AdapterVersion::new("mfm.bitcoin.jsonrpc.adapter.v1").expect("adapter version"),
        ),
    })
    .expect("fact ref")
}

fn checkpoint_response_evidence(
    byte_len: u64,
    artifact_id: ArtifactId,
    digest: ContentDigest,
    producer_node_id: mfm_ids::NodeId,
) -> ArtifactEvidenceRef {
    ArtifactEvidenceRef {
        artifact_id,
        digest,
        byte_len,
        media_type: MediaType::new("application/json").expect("media type"),
        schema_id: Some(CollectorCheckpointResponse::schema_id().expect("schema")),
        semantic_type_id: None,
        producer_node_id: Some(producer_node_id),
        producer_seed_id: None,
        artifact_role: events::ArtifactRole::FactResponse,
    }
}

fn artifact_evidence_hash(evidence: &ArtifactEvidenceRef) -> ContentDigest {
    mfm_store::v1::ArtifactEvidenceRef::from(evidence.clone())
        .evidence_hash()
        .expect("artifact evidence hash")
}

fn producer_node_id(seed: u8) -> mfm_ids::NodeId {
    mfm_ids::NodeId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(seed + 2))
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

#[tokio::test]
async fn provider_maps_head_requests_to_blockchain_info_and_headers() {
    for (name, selection, expected_height, expected_hash, expected_calls) in [
        (
            "best head",
            BtcHeadSelection::best(),
            850_000,
            BEST_HASH,
            vec!["info".to_owned(), format!("header:{BEST_HASH}")],
        ),
        (
            "confirmed head",
            BtcHeadSelection::confirmed(6).expect("confirmed"),
            849_994,
            CONFIRMED_HASH,
            vec![
                "info".to_owned(),
                "hash:849994".to_owned(),
                format!("header:{CONFIRMED_HASH}"),
            ],
        ),
    ] {
        let transport = Arc::new(MockTransport::default());
        let provider = routed_provider(transport.clone());
        let response = provider
            .read_chain_head(&make_request(selection))
            .await
            .expect(name);

        assert_eq!(response.block_height, expected_height, "{name}");
        assert_eq!(response.block_hash.as_str(), expected_hash, "{name}");
        assert_eq!(
            response.provider_time_unix_ms,
            Some(1_720_000_000_000),
            "{name}"
        );
        assert_eq!(transport.calls(), expected_calls, "{name}");
    }
}

#[tokio::test]
async fn provider_rejects_exact_balance_requests_without_utxo_scan() {
    let address = "bc1qns9f7yfx3ry9lj6yz7c9er0vwa0ye2eklpzqfw";
    let transport = Arc::new(MockTransport::default());
    let provider = routed_provider(transport.clone());
    let request = make_balance_request(address);
    let error = provider
        .read_balance(&request)
        .await
        .expect_err("exact anchored balance unsupported");

    let BtcCapabilityError::Provider { diagnostic } = error else {
        panic!("expected provider diagnostic");
    };
    assert_eq!(
        diagnostic.stable_error_code(),
        "bitcoin_unsupported_operation"
    );
    assert_eq!(transport.calls(), vec!["info".to_owned()]);
}

#[tokio::test]
async fn provider_errors_discard_transport_secret_details() {
    let transport = Arc::new(MockTransport {
        calls: Mutex::new(Vec::new()),
        info_failure: Some(MockInfoFailure::Http),
    });
    let provider = routed_provider(transport);
    let error = provider
        .read_chain_head(&make_request(BtcHeadSelection::best()))
        .await
        .expect_err("provider failure");
    let rendered = format!("{error:?} {error}");

    let BtcCapabilityError::Provider { diagnostic } = error else {
        panic!("expected provider diagnostic");
    };
    assert_eq!(diagnostic.stable_error_code(), "bitcoin_transport_failed");
    assert_eq!(
        diagnostic.summary(),
        "transport_failed operation=getblockchaininfo"
    );
    assert!(!rendered.contains("localhost"));
    assert!(!rendered.contains("password"));
    assert!(!rendered.contains("secret"));
    assert!(!rendered.contains("Authorization"));
}

#[tokio::test]
async fn provider_classifies_http_status_failure_without_body() {
    let transport = Arc::new(MockTransport {
        calls: Mutex::new(Vec::new()),
        info_failure: Some(MockInfoFailure::HttpStatus),
    });
    let provider = routed_provider(transport);
    let error = provider
        .read_chain_head(&make_request(BtcHeadSelection::best()))
        .await
        .expect_err("provider failure");
    let BtcCapabilityError::Provider { diagnostic } = error else {
        panic!("expected provider diagnostic");
    };

    assert_eq!(diagnostic.stable_error_code(), "bitcoin_rpc_http_status");
    assert_eq!(
        diagnostic.summary(),
        "rpc_http_status operation=getblockchaininfo http_status=403"
    );
}

#[tokio::test]
async fn provider_classifies_json_rpc_failure_without_message() {
    let transport = Arc::new(MockTransport {
        calls: Mutex::new(Vec::new()),
        info_failure: Some(MockInfoFailure::JsonRpc),
    });
    let provider = routed_provider(transport);
    let error = provider
        .read_chain_head(&make_request(BtcHeadSelection::best()))
        .await
        .expect_err("provider failure");
    let rendered = format!("{error:?} {error}");
    let BtcCapabilityError::Provider { diagnostic } = error else {
        panic!("expected provider diagnostic");
    };

    assert_eq!(diagnostic.stable_error_code(), "bitcoin_rpc_json_error");
    assert_eq!(
        diagnostic.summary(),
        "rpc_json_error operation=getblockchaininfo rpc_code=-32601"
    );
    assert!(!rendered.contains("secret provider message"));
}

#[test]
fn replay_helper_verifies_recorded_evidence_without_transport() {
    let request = make_request(BtcHeadSelection::best());
    let response = chain_head_response(&request, 850_000, BEST_HASH, Some(1_720_000_000_000));

    verify_recorded_chain_head_evidence(&request, &response).expect("evidence");
    let fact = replay_chain_head_fact_from_evidence(
        &request,
        &response,
        &observe_input(None, Some(1_720_000_001_000)),
    )
    .expect("replay fact");
    assert_eq!(fact.response().block_height(), 850_000);
}

#[test]
fn replay_helper_rejects_mismatched_recorded_evidence() {
    let request = make_request(BtcHeadSelection::best());
    let mismatched_request = make_request(BtcHeadSelection::confirmed(6).expect("confirmed"));
    let response = chain_head_response(&mismatched_request, 849_994, CONFIRMED_HASH, None);

    let error = verify_recorded_chain_head_evidence(&request, &response).expect_err("mismatch");
    assert_eq!(error, BtcJsonRpcAdapterError::ReplayEvidenceMismatch);
}

#[test]
fn replay_helper_rejects_incompatible_loaded_checkpoints() {
    let request = make_request(BtcHeadSelection::best());
    let response = chain_head_response(&request, 850_000, BEST_HASH, Some(1_720_000_000_000));

    for (name, input) in [
        (
            "checkpoint source incompatibility",
            observe_input(
                Some(checkpoint_fact_for_source(849_999, "other-bitcoin-core")),
                Some(1_720_000_001_000),
            ),
        ),
        (
            "observation behind loaded checkpoint",
            observe_input(Some(checkpoint_fact(850_001)), Some(1_720_000_001_000)),
        ),
    ] {
        let error =
            replay_chain_head_fact_from_evidence(&request, &response, &input).expect_err(name);

        assert_eq!(
            error,
            BtcJsonRpcAdapterError::ReplayEvidenceMismatch,
            "{name}"
        );
    }
}

#[test]
fn checkpoint_record_fixture_uses_recorded_chain_head_fact() {
    let request = make_request(BtcHeadSelection::best());
    let response = chain_head_response(&request, 850_000, BEST_HASH, Some(1_720_000_000_000));
    let chain_head_fact = replay_chain_head_fact_from_evidence(
        &request,
        &response,
        &observe_input(None, Some(1_720_000_001_000)),
    )
    .expect("chain-head fact");
    let state = RecordCollectorCheckpointState::new(
        ValidatedConfig::new(checkpoint_record_config()).expect("config"),
    )
    .expect("state");

    let caps = (BtcFactRecordCapability,);
    let context = mfm_program::CertifiedContext::no_context();
    let checkpoint = poll_ready(state.run(
        RecordCollectorCheckpointInput {
            chain_head_fact,
            loaded_checkpoint: LoadedCollectorCheckpoint::new(Some(checkpoint_fact(849_999))),
        },
        &caps,
        &context,
    ))
    .expect("checkpoint fact");

    assert_eq!(checkpoint.response().high_watermark_height(), 850_000);
    assert_eq!(checkpoint.response().high_watermark_hash(), BEST_HASH);
    assert!(checkpoint
        .response()
        .predecessor_checkpoint_hash()
        .is_some());
}

#[test]
fn capability_binding_uses_btc_jsonrpc_adapter_identity() {
    let binding = btc_fact_record_capability_binding().expect("binding");

    assert_eq!(
        binding.capability_kind().canonical_name(),
        Some("mfm.bitcoin/fact.record")
    );
    assert_eq!(
        binding.adapter_kind(),
        &btc_jsonrpc_adapter_kind().expect("adapter kind")
    );
    assert_eq!(
        binding.adapter_version(),
        &btc_jsonrpc_adapter_version().expect("adapter version")
    );
}
