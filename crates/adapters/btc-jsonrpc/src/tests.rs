use super::*;
use mfm_artifact_capabilities::{ArtifactEvidenceRef, VerifiedArtifactBytes};
use mfm_btc_capabilities::{
    BtcChain, BtcChainGuard, BtcHeadSelection, BtcNetworkId, BtcSourceIdentity,
};
use mfm_canonical::sha256_digest_bytes;
use mfm_facts::{
    fact_descriptor_hash, DescriptorCatalogWatermark, FactAudience, FactClaimId,
    FactProjectionGeneration, FactQueryReceipt, FactQueryScope, FactSelectionEvidence,
    FactVisibility, FactVisibilityScope, InternalFactRef, InternalFactRefParts,
    QueryResultCardinality, StoreCommitWatermark, StoreIdentity, StoreKeyId, StoreReadFrontier,
    StoreReadFrontierType, StoreReceiptAuthentication, StoreScopeRef,
};
use mfm_ids::{
    AdapterKind, AdapterVersion, ArtifactId, CapabilityKind, CapabilityVersion, ContentDigest,
    DigestAlgorithm, DigestBytes, EventId, RunId,
};
use mfm_spec::v1::MediaType;
use std::sync::Mutex;

const BEST_HASH: &str = "00000000000000000001b2a7f3e0d5c4b6a897887766554433221100ffeeddcc";
const CONFIRMED_HASH: &str = "00000000000000000002b2a7f3e0d5c4b6a897887766554433221100ffeeddcc";
const ED25519_TEST_VERIFYING_KEY: [u8; 32] = [
    0xd7, 0x5a, 0x98, 0x01, 0x82, 0xb1, 0x0a, 0xb7, 0xd5, 0x4b, 0xfe, 0xd3, 0xc9, 0x64, 0x07, 0x3a,
    0x0e, 0xe1, 0x72, 0xf3, 0xda, 0xa6, 0x23, 0x25, 0xaf, 0x02, 0x1a, 0x68, 0xf7, 0x07, 0x51, 0x1a,
];

#[derive(Default)]
struct MockTransport {
    calls: Mutex<Vec<String>>,
    fail_info: bool,
}

impl MockTransport {
    fn calls(&self) -> Vec<String> {
        self.calls.lock().expect("calls").clone()
    }
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
        let bytes = serde_json::to_vec(response).expect("response json");
        let digest =
            ContentDigest::from_digest(DigestAlgorithm::Sha256JcsV1, sha256_digest_bytes(&bytes));
        let artifact_id = ArtifactId::from_digest(digest.algorithm(), *digest.digest());
        let evidence = ArtifactEvidenceRef {
            artifact_id: artifact_id.clone(),
            digest: digest.clone(),
            byte_len: bytes.len() as u64,
            media_type: MediaType::new("application/json").expect("media type"),
            schema_id: Some(CollectorCheckpointResponse::schema_id().expect("schema")),
            semantic_type_id: None,
            producer_node_id: None,
            producer_seed_id: None,
            artifact_role: events::ArtifactRole::FactResponse,
        };
        let fact_ref = internal_fact_ref(1, artifact_id, digest);
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

impl ArtifactReadProvider for MockArtifacts {
    fn read_artifact<'a>(
        &'a self,
        request: &'a ArtifactReadRequest,
    ) -> mfm_artifact_capabilities::ArtifactReadFuture<'a> {
        Box::pin(async move {
            self.calls
                .lock()
                .expect("calls")
                .push(request.artifact_id().clone());
            let Some((bytes, evidence)) = &self.artifact else {
                return Err(mfm_artifact_capabilities::ArtifactReadError::NotFound {
                    artifact_id: Box::new(request.artifact_id().clone()),
                });
            };
            VerifiedArtifactBytes::new(bytes.clone(), evidence.clone(), request)
        })
    }
}

impl BtcJsonRpcChainHeadTransport for MockTransport {
    fn get_blockchain_info<'a>(&'a self) -> BtcTransportFuture<'a, BlockchainInfo> {
        Box::pin(async move {
            self.calls.lock().expect("calls").push("info".to_owned());
            if self.fail_info {
                return Err(BtcRpcError::Http(
                    concat!(
                        "http://user:password@localhost:8332 ",
                        "Authorization: Bearer secret"
                    )
                    .to_owned(),
                ));
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
}

fn make_request(selection: BtcHeadSelection) -> BtcChainHeadRequest {
    BtcChainHeadRequest {
        guard: BtcChainGuard::new(
            BtcChain::Bitcoin,
            BtcNetworkId::new("bitcoin-mainnet").expect("network"),
            BtcSourceIdentity::new("public-bitcoin-core").expect("source"),
        ),
        selection,
    }
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
        BtcChain::Bitcoin,
        &network,
    );
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
        store_scope: "mfm.store.default".to_owned(),
    }
}

fn checkpoint_query_input() -> QueryCollectorCheckpointInput {
    QueryCollectorCheckpointInput {
        context: mfm_states_btc::QueryCollectorCheckpointContext {
            queried_at_unix_ms: Some(1_720_000_001_000),
        },
    }
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
    let bytes = serde_json::to_vec(&response).expect("response json");
    let response_digest =
        ContentDigest::from_digest(DigestAlgorithm::Sha256JcsV1, sha256_digest_bytes(&bytes));
    let fact_ref = internal_fact_ref(
        2,
        ArtifactId::from_digest(response_digest.algorithm(), *response_digest.digest()),
        response_digest,
    );
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
    let cardinality = QueryResultCardinality::Exact(returned_refs.len() as u64);
    let result_set_digest =
        mfm_facts::fact_query_result_set_digest(&returned_refs, None).expect("result digest");
    let read_frontier = StoreReadFrontier::new(
        StoreScopeRef::new("mfm.store.default").expect("store scope"),
        FactQueryScope::new(FactAudience::Control, FactVisibilityScope::Default),
        DescriptorCatalogWatermark::new(1),
        FactProjectionGeneration::new(1),
        11,
        StoreCommitWatermark::new(11),
    );
    let plan_hash = mfm_facts::fact_query_plan_hash(plan).expect("plan hash");
    let receipt_hash = mfm_facts::fact_query_receipt_body_hash_from_parts(
        &plan_hash,
        &read_frontier,
        StoreReadFrontierType::Snapshot,
        &returned_refs,
        None,
        &result_set_digest,
        cardinality,
    )
    .expect("receipt hash");
    FactQueryReceipt::new(
        read_frontier,
        StoreReadFrontierType::Snapshot,
        returned_refs,
        None,
        result_set_digest,
        cardinality,
        receipt_hash,
        StoreReceiptAuthentication::new(
            StoreIdentity::new("store.default").expect("store identity"),
            mfm_facts::StoreReceiptAuthenticationScheme::LocalEd25519Sha256JcsV1,
            Some(StoreKeyId::new("fact.read.key").expect("key id")),
            vec![0x11; 64],
        )
        .expect("auth"),
    )
}

fn trust_root() -> FactQueryReceiptTrustRootMaterial {
    FactQueryReceiptTrustRootMaterial::new(
        StoreIdentity::new("store.default").expect("store identity"),
        mfm_facts::StoreReceiptAuthenticationScheme::LocalEd25519Sha256JcsV1,
        StoreKeyId::new("fact.read.key").expect("key id"),
        ED25519_TEST_VERIFYING_KEY,
    )
}

fn internal_fact_ref(
    seed: u8,
    artifact_id: ArtifactId,
    response_hash: ContentDigest,
) -> InternalFactRef {
    let descriptor_hash =
        fact_descriptor_hash(&CollectorCheckpointFact::descriptor().expect("descriptor"))
            .expect("descriptor hash");
    InternalFactRef::new(InternalFactRefParts {
        fact_claim_id: FactClaimId::new(run_id(seed), 9, 0).expect("claim id"),
        source_event_id: EventId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(seed + 1)),
        recorded_at: "2026-07-02T00:00:00Z".to_owned(),
        observed_at: Some("2026-07-02T00:00:00Z".to_owned()),
        visibility: FactVisibility::indexed_default(FactAudience::Control),
        fact_kind: mfm_facts::FactKind::new("collector.checkpoint").expect("kind"),
        fact_descriptor_hash: descriptor_hash,
        fact_subject_namespace_hash: digest(seed + 3),
        fact_key: mfm_facts::FactKey::from_digest(digest(seed + 4)),
        subject_material_hash: digest(seed + 5),
        request_schema_id: None,
        request_hash: None,
        response_schema_id: CollectorCheckpointResponse::schema_id().expect("schema"),
        response_hash,
        artifact_id,
        artifact_evidence_hash: digest(seed + 9),
        capability_kind: CapabilityKind::new(
            "mfm.bitcoin",
            "chain_head.read",
            DigestAlgorithm::Sha256JcsV1,
            digest_bytes(seed + 10),
        )
        .expect("capability kind"),
        capability_version: CapabilityVersion::new("mfm.bitcoin.chain_head.read.v1")
            .expect("capability version"),
        adapter_kind: AdapterKind::new(
            "mfm.bitcoin",
            "jsonrpc",
            DigestAlgorithm::Sha256JcsV1,
            digest_bytes(seed + 11),
        )
        .expect("adapter kind"),
        adapter_version: AdapterVersion::new("mfm.bitcoin.jsonrpc.adapter.v1")
            .expect("adapter version"),
    })
    .expect("fact ref")
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
async fn provider_maps_best_head_request_to_blockchain_info_and_header() {
    let transport = Arc::new(MockTransport::default());
    let provider = BtcJsonRpcChainHeadProvider::new(transport.clone());
    let response = provider
        .read_chain_head(&make_request(BtcHeadSelection::best()))
        .await
        .expect("best head");

    assert_eq!(response.block_height, 850_000);
    assert_eq!(response.block_hash.as_str(), BEST_HASH);
    assert_eq!(response.provider_time_unix_ms, Some(1_720_000_000_000));
    assert_eq!(
        transport.calls(),
        vec!["info".to_owned(), format!("header:{BEST_HASH}")]
    );
}

#[tokio::test]
async fn provider_maps_confirmed_head_request_to_selected_height() {
    let transport = Arc::new(MockTransport::default());
    let provider = BtcJsonRpcChainHeadProvider::new(transport.clone());
    let response = provider
        .read_chain_head(&make_request(
            BtcHeadSelection::confirmed(6).expect("confirmed"),
        ))
        .await
        .expect("confirmed head");

    assert_eq!(response.block_height, 849_994);
    assert_eq!(response.block_hash.as_str(), CONFIRMED_HASH);
    assert_eq!(
        transport.calls(),
        vec![
            "info".to_owned(),
            "hash:849994".to_owned(),
            format!("header:{CONFIRMED_HASH}")
        ]
    );
}

#[tokio::test]
async fn provider_errors_discard_transport_secret_details() {
    let transport = Arc::new(MockTransport {
        calls: Mutex::new(Vec::new()),
        fail_info: true,
    });
    let provider = BtcJsonRpcChainHeadProvider::new(transport);
    let error = provider
        .read_chain_head(&make_request(BtcHeadSelection::best()))
        .await
        .expect_err("provider failure");
    let rendered = format!("{error:?} {error}");

    assert!(matches!(error, BtcCapabilityError::Provider { .. }));
    assert!(!rendered.contains("localhost"));
    assert!(!rendered.contains("password"));
    assert!(!rendered.contains("secret"));
    assert!(!rendered.contains("Authorization"));
}

#[test]
fn replay_helper_verifies_recorded_evidence_without_transport() {
    let request = make_request(BtcHeadSelection::best());
    let response = BtcChainHeadResponse {
        evidence: RedactedBtcSourceEvidence::from_request(
            &request,
            Some("main".to_owned()),
            BtcSourceStatus::Synced,
        ),
        block_height: 850_000,
        block_hash: BtcBlockHash::new(BEST_HASH).expect("hash"),
        provider_time_unix_ms: Some(1_720_000_000_000),
    };

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
    let response = BtcChainHeadResponse {
        evidence: RedactedBtcSourceEvidence::from_request(
            &mismatched_request,
            Some("main".to_owned()),
            BtcSourceStatus::Synced,
        ),
        block_height: 849_994,
        block_hash: BtcBlockHash::new(CONFIRMED_HASH).expect("hash"),
        provider_time_unix_ms: None,
    };

    let error = verify_recorded_chain_head_evidence(&request, &response).expect_err("mismatch");
    assert_eq!(error, BtcJsonRpcAdapterError::ReplayEvidenceMismatch);
}

#[test]
fn replay_helper_rejects_checkpoint_source_incompatibility() {
    let request = make_request(BtcHeadSelection::best());
    let response = BtcChainHeadResponse {
        evidence: RedactedBtcSourceEvidence::from_request(
            &request,
            Some("main".to_owned()),
            BtcSourceStatus::Synced,
        ),
        block_height: 850_000,
        block_hash: BtcBlockHash::new(BEST_HASH).expect("hash"),
        provider_time_unix_ms: Some(1_720_000_000_000),
    };
    let input = observe_input(
        Some(checkpoint_fact_for_source(849_999, "other-bitcoin-core")),
        Some(1_720_000_001_000),
    );

    let error =
        replay_chain_head_fact_from_evidence(&request, &response, &input).expect_err("checkpoint");

    assert_eq!(error, BtcJsonRpcAdapterError::ReplayEvidenceMismatch);
}

#[test]
fn replay_helper_rejects_observation_behind_loaded_checkpoint() {
    let request = make_request(BtcHeadSelection::best());
    let response = BtcChainHeadResponse {
        evidence: RedactedBtcSourceEvidence::from_request(
            &request,
            Some("main".to_owned()),
            BtcSourceStatus::Synced,
        ),
        block_height: 850_000,
        block_hash: BtcBlockHash::new(BEST_HASH).expect("hash"),
        provider_time_unix_ms: Some(1_720_000_000_000),
    };
    let input = observe_input(Some(checkpoint_fact(850_001)), Some(1_720_000_001_000));

    let error =
        replay_chain_head_fact_from_evidence(&request, &response, &input).expect_err("checkpoint");

    assert_eq!(error, BtcJsonRpcAdapterError::ReplayEvidenceMismatch);
}

#[test]
fn checkpoint_record_fixture_uses_recorded_chain_head_fact() {
    let request = make_request(BtcHeadSelection::best());
    let response = BtcChainHeadResponse {
        evidence: RedactedBtcSourceEvidence::from_request(
            &request,
            Some("main".to_owned()),
            BtcSourceStatus::Synced,
        ),
        block_height: 850_000,
        block_hash: BtcBlockHash::new(BEST_HASH).expect("hash"),
        provider_time_unix_ms: Some(1_720_000_000_000),
    };
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

    let checkpoint = state
        .run(RecordCollectorCheckpointInput {
            chain_head_fact,
            loaded_checkpoint: LoadedCollectorCheckpoint::new(Some(checkpoint_fact(849_999))),
        })
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
    let binding = btc_capability_binding().expect("binding");

    assert_eq!(
        binding.capability_kind.canonical_name(),
        Some("mfm.bitcoin/chain_head.read")
    );
    assert_eq!(
        binding.adapter_kind,
        btc_jsonrpc_adapter_kind().expect("adapter kind")
    );
    assert_eq!(
        binding.adapter_version,
        btc_jsonrpc_adapter_version().expect("adapter version")
    );
}
