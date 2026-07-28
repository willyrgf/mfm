//! Behavioral contract tests for the public Bitcoin transport.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use bitcoin::hashes::{sha256, Hash as _};
use mfm_bitcoin::{
    BitcoinAddress, BitcoinBalanceCollectionEvidence, BitcoinBalanceCollectionRequest,
    BitcoinBalanceSession, BitcoinCapabilityError, BitcoinNetworkId, BitcoinNetworkTag,
    BitcoinSourceIdentity, BITCOIN_BALANCE_COLLECTION_ADDRESS_LIMIT,
};
use mfm_capabilities::ProviderDiagnosticCode;
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::{oneshot, Semaphore};

use super::*;

const LEGACY_MAIN: &str = "1BoatSLRHtKNngkdXEeobR76b53LETtpyT";
const SEGWIT_MAIN: &str = "bc1qvzvkjn4q3nszqxrv3nraga2r822xjty3ykvkuw";
const ANCHOR_HASH: &str = "00000000000000000001b2a7f3e0d5c4b6a897887766554433221100ffeeddcc";
const OTHER_HASH: &str = "00000000000000000002b2a7f3e0d5c4b6a897887766554433221100ffeeddcc";
const TXID_ONE: &str = "1111111111111111111111111111111111111111111111111111111111111111";
const TXID_TWO: &str = "2222222222222222222222222222222222222222222222222222222222222222";
const FIXTURE_ENDPOINT_PATH: &str = "fixture-private-path-letmein";
const FIXTURE_BASIC_USER: &str = "fixture-user";
const FIXTURE_BASIC_PASSWORD: &str = "fixture-password-123456";
const FIXTURE_PROVIDER_BODY: &str = "fixture-raw-provider-payload";
const FIXTURE_PROVIDER_URL: &str =
    "https://fixture-user:fixture-password@provider.invalid/private?access_token=123456";
const FIXTURE_LOW_ENTROPY: &str = "letmein";

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct PrototypeRoutingGenerationRef(String);

#[derive(Debug, Clone, PartialEq, Eq)]
enum PrototypeRouteResolution {
    Resolved(String),
    DidNotEnter,
}

#[derive(Default)]
struct PrototypeRouteResolver {
    current: Option<PrototypeRoutingGenerationRef>,
    routes: BTreeMap<PrototypeRoutingGenerationRef, String>,
}

impl PrototypeRouteResolver {
    fn install_current(&mut self, generation: PrototypeRoutingGenerationRef, endpoint: String) {
        self.routes.insert(generation.clone(), endpoint);
        self.current = Some(generation);
    }

    fn remove(&mut self, generation: &PrototypeRoutingGenerationRef) {
        self.routes.remove(generation);
    }

    fn resolve_exact(
        &self,
        generation: &PrototypeRoutingGenerationRef,
    ) -> PrototypeRouteResolution {
        self.routes
            .get(generation)
            .cloned()
            .map(PrototypeRouteResolution::Resolved)
            .unwrap_or(PrototypeRouteResolution::DidNotEnter)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrototypeScanObservation {
    Returned,
    DidNotEnter,
    Indeterminate,
}

#[derive(Default)]
struct PrototypeScanAuditLedger {
    next_authorization: usize,
    authorizations: Vec<usize>,
    observations: Vec<(usize, PrototypeScanObservation)>,
}

impl PrototypeScanAuditLedger {
    fn authorize(ledger: &Arc<Mutex<Self>>) -> usize {
        let mut state = ledger.lock().expect("scan audit ledger");
        let authorization = state.next_authorization;
        state.next_authorization += 1;
        state.authorizations.push(authorization);
        authorization
    }

    fn observe(
        ledger: &Arc<Mutex<Self>>,
        authorization: usize,
        observation: PrototypeScanObservation,
    ) {
        let mut state = ledger.lock().expect("scan audit ledger");
        assert!(
            state
                .observations
                .iter()
                .all(|(candidate, _)| *candidate != authorization),
            "one authorization permits one observation"
        );
        state.observations.push((authorization, observation));
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PrototypeScanResult {
    height: u64,
    best_block: &'static str,
}

struct PrototypeRemoteScan {
    response: Option<oneshot::Receiver<PrototypeScanResult>>,
}

struct PrototypeScanProvider {
    busy: Arc<AtomicBool>,
    protocol_calls: Arc<AtomicUsize>,
    full_scan_starts: Arc<AtomicUsize>,
    drop_next_response: Arc<AtomicBool>,
    releases: Arc<Semaphore>,
    methods: Arc<Mutex<Vec<&'static str>>>,
}

impl Default for PrototypeScanProvider {
    fn default() -> Self {
        Self {
            busy: Arc::new(AtomicBool::new(false)),
            protocol_calls: Arc::new(AtomicUsize::new(0)),
            full_scan_starts: Arc::new(AtomicUsize::new(0)),
            drop_next_response: Arc::new(AtomicBool::new(false)),
            releases: Arc::new(Semaphore::new(0)),
            methods: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

impl PrototypeScanProvider {
    fn begin(&self) -> Result<PrototypeRemoteScan, ()> {
        self.protocol_calls.fetch_add(1, Ordering::SeqCst);
        self.methods
            .lock()
            .expect("scan methods")
            .push("scantxoutset:start");
        if self
            .busy
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return Err(());
        }
        self.full_scan_starts.fetch_add(1, Ordering::SeqCst);
        let (sender, receiver) = oneshot::channel();
        let busy = Arc::clone(&self.busy);
        let releases = Arc::clone(&self.releases);
        let drop_response = self.drop_next_response.swap(false, Ordering::SeqCst);
        tokio::spawn(async move {
            let permit = releases
                .acquire_owned()
                .await
                .expect("scan release semaphore");
            permit.forget();
            busy.store(false, Ordering::SeqCst);
            if !drop_response {
                let _ = sender.send(PrototypeScanResult {
                    height: 850_000,
                    best_block: ANCHOR_HASH,
                });
            }
        });
        Ok(PrototypeRemoteScan {
            response: Some(receiver),
        })
    }

    fn release_one(&self) {
        self.releases.add_permits(1);
    }
}

struct PrototypeAuditedScan {
    authorization: usize,
    ledger: Arc<Mutex<PrototypeScanAuditLedger>>,
    remote: PrototypeRemoteScan,
    observed: bool,
}

impl PrototypeAuditedScan {
    async fn finish(mut self) -> Option<PrototypeScanResult> {
        let result = self
            .remote
            .response
            .take()
            .expect("one remote response")
            .await;
        let (observation, result) = match result {
            Ok(result) => (PrototypeScanObservation::Returned, Some(result)),
            Err(_) => (PrototypeScanObservation::Indeterminate, None),
        };
        PrototypeScanAuditLedger::observe(&self.ledger, self.authorization, observation);
        self.observed = true;
        result
    }
}

impl Drop for PrototypeAuditedScan {
    fn drop(&mut self) {
        if !self.observed {
            PrototypeScanAuditLedger::observe(
                &self.ledger,
                self.authorization,
                PrototypeScanObservation::Indeterminate,
            );
            self.observed = true;
        }
    }
}

enum PrototypeScanStart {
    Entered(PrototypeAuditedScan),
    DidNotEnter(usize),
}

fn begin_audited_scan(
    provider: &PrototypeScanProvider,
    ledger: &Arc<Mutex<PrototypeScanAuditLedger>>,
) -> PrototypeScanStart {
    let authorization = PrototypeScanAuditLedger::authorize(ledger);
    match provider.begin() {
        Ok(remote) => PrototypeScanStart::Entered(PrototypeAuditedScan {
            authorization,
            ledger: Arc::clone(ledger),
            remote,
            observed: false,
        }),
        Err(()) => {
            PrototypeScanAuditLedger::observe(
                ledger,
                authorization,
                PrototypeScanObservation::DidNotEnter,
            );
            PrototypeScanStart::DidNotEnter(authorization)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrototypeScanQualificationGap {
    BoundedProviderWork,
    RepeatedCostAndSharedConcurrencyAcceptance,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PrototypeBitcoinRegistrationDisposition {
    Registered,
    Unregistered(Vec<PrototypeScanQualificationGap>),
}

struct PrototypeScanQualification {
    no_durable_domain_mutation: bool,
    indivisible_snapshot: bool,
    bounded_descriptors: bool,
    bounded_retained_result: bool,
    start_only: bool,
    no_hidden_retry_or_control_call: bool,
    bounded_provider_work: bool,
    repeated_cost_and_shared_concurrency_accepted: bool,
}

impl PrototypeScanQualification {
    fn disposition(&self) -> PrototypeBitcoinRegistrationDisposition {
        assert!(self.no_durable_domain_mutation);
        assert!(self.indivisible_snapshot);
        assert!(self.bounded_descriptors);
        assert!(self.bounded_retained_result);
        assert!(self.start_only);
        assert!(self.no_hidden_retry_or_control_call);
        let mut gaps = Vec::new();
        if !self.bounded_provider_work {
            gaps.push(PrototypeScanQualificationGap::BoundedProviderWork);
        }
        if !self.repeated_cost_and_shared_concurrency_accepted {
            gaps.push(PrototypeScanQualificationGap::RepeatedCostAndSharedConcurrencyAcceptance);
        }
        if gaps.is_empty() {
            PrototypeBitcoinRegistrationDisposition::Registered
        } else {
            PrototypeBitcoinRegistrationDisposition::Unregistered(gaps)
        }
    }
}

#[derive(Clone, Copy)]
enum Mode {
    Valid,
    Zero,
    ChainMismatch,
    InitialBlockDownload,
    Reorganization,
    ScanBusy,
    NearScanBusy,
    StallScan,
    BadVersion,
    BadId,
    ResultAndError,
    MissingArms,
    HttpFailure,
    Created,
    Accepted,
    NoContent,
    Redirect,
    OversizedLength,
    OversizedChunked,
    AdversarialMalformed,
}

struct TestServer {
    url: String,
    requests: Arc<Mutex<Vec<Value>>>,
    raw_requests: Arc<Mutex<Vec<String>>>,
}

impl TestServer {
    async fn spawn(mode: Mode) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let address = listener.local_addr().expect("address");
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&requests);
        let raw_requests = Arc::new(Mutex::new(Vec::new()));
        let captured_raw = Arc::clone(&raw_requests);
        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    return;
                };
                let captured = Arc::clone(&captured);
                let captured_raw = Arc::clone(&captured_raw);
                tokio::spawn(async move {
                    let bytes = read_http_request(&mut stream).await;
                    captured_raw
                        .lock()
                        .expect("raw requests")
                        .push(String::from_utf8_lossy(&bytes).into_owned());
                    let request: Value =
                        serde_json::from_slice(request_body(&bytes)).expect("request JSON");
                    captured.lock().expect("requests").push(request.clone());
                    if matches!(mode, Mode::StallScan) && request["method"] == "scantxoutset" {
                        std::future::pending::<()>().await;
                    }
                    let response = response(mode, &request);
                    stream
                        .write_all(response.as_bytes())
                        .await
                        .expect("response");
                });
            }
        });
        Self {
            url: format!("http://{address}"),
            requests,
            raw_requests,
        }
    }

    fn requests(&self) -> Vec<Value> {
        self.requests.lock().expect("requests").clone()
    }

    fn raw_requests(&self) -> Vec<String> {
        self.raw_requests.lock().expect("raw requests").clone()
    }
}

fn assert_synthetic_material_absent(rendered: &str, material: &[&str]) {
    for value in material {
        assert!(!rendered.contains(value));
        let fingerprint = sha256::Hash::hash(value.as_bytes()).to_string();
        assert!(!rendered.contains(&fingerprint));
    }
}

fn binding() -> BitcoinSourceBinding {
    BitcoinSourceBinding::new(
        BitcoinNetworkId::new("bitcoin-mainnet").expect("network"),
        BitcoinNetworkTag::Main,
        BitcoinSourceIdentity::new("public-bitcoin-core").expect("source"),
    )
}

fn request() -> BitcoinBalanceCollectionRequest {
    BitcoinBalanceCollectionRequest::new(
        binding(),
        vec![LEGACY_MAIN.to_owned(), SEGWIT_MAIN.to_owned()],
    )
    .expect("request")
}

fn session(server: &TestServer) -> BitcoinRpcSession {
    BitcoinRpcSession::new(
        BitcoinRpcEndpoint::new(&server.url).expect("endpoint"),
        None,
        binding(),
        Duration::from_secs(1),
    )
    .expect("session")
}

fn authenticated_fixture_session(server: &TestServer) -> BitcoinRpcSession {
    BitcoinRpcSession::new(
        BitcoinRpcEndpoint::new(format!("{}/{FIXTURE_ENDPOINT_PATH}", server.url))
            .expect("endpoint"),
        Some(
            BitcoinRpcAuthentication::new(
                FIXTURE_BASIC_USER.to_owned(),
                zeroize::Zeroizing::new(FIXTURE_BASIC_PASSWORD.to_owned()),
            )
            .expect("authentication"),
        ),
        binding(),
        Duration::from_secs(1),
    )
    .expect("session")
}

async fn prototype_blockchain_info(
    session: &BitcoinRpcSession,
) -> Result<BlockchainInfo, BitcoinRpcError> {
    session
        .rpc(
            "getblockchaininfo",
            serde_json::json!([]),
            ORDINARY_REQUEST_TIMEOUT,
        )
        .await
}

async fn prototype_scan(
    session: &BitcoinRpcSession,
    request: &BitcoinBalanceCollectionRequest,
) -> Result<ScanTxOutSetResult, BitcoinRpcError> {
    let descriptors = request
        .addresses()
        .iter()
        .map(BitcoinAddress::scan_descriptor)
        .collect::<Vec<_>>();
    session
        .rpc(
            "scantxoutset",
            serde_json::json!(["start", descriptors]),
            session.scan_timeout,
        )
        .await
}

async fn prototype_block_hash(
    session: &BitcoinRpcSession,
    height: u64,
) -> Result<String, BitcoinRpcError> {
    session
        .rpc(
            "getblockhash",
            serde_json::json!([height]),
            ORDINARY_REQUEST_TIMEOUT,
        )
        .await
}

#[tokio::test]
async fn aggregate_collection_is_exactly_one_sorted_scan_between_anchor_checks() {
    let server = TestServer::spawn(Mode::Valid).await;
    let response = session(&server)
        .collect_balances(&request())
        .await
        .expect("collection");

    assert_eq!(response.binding(), &binding());
    assert_eq!(response.anchor_height(), 850_000);
    assert_eq!(response.anchor_hash().to_string(), ANCHOR_HASH);
    assert_eq!(response.final_canonical_hash(), response.anchor_hash());
    assert_eq!(response.balances().len(), 2);
    assert_eq!(response.balances()[0].address(), LEGACY_MAIN);
    assert_eq!(response.balances()[0].balance_sats(), 3);
    assert_eq!(response.balances()[1].address(), SEGWIT_MAIN);
    assert_eq!(response.balances()[1].balance_sats(), 4);

    let requests = server.requests();
    assert_eq!(
        requests
            .iter()
            .map(|request| request["method"].as_str().expect("method"))
            .collect::<Vec<_>>(),
        ["getblockchaininfo", "scantxoutset", "getblockhash"]
    );
    assert_eq!(
        requests[1]["params"],
        json!([
            "start",
            [
                format!("addr({LEGACY_MAIN})"),
                format!("addr({SEGWIT_MAIN})")
            ]
        ])
    );
    assert_eq!(requests[2]["params"], json!([850_000]));
    assert!(requests.iter().all(|request| request["jsonrpc"] == "2.0"));
}

#[tokio::test]
async fn recoverability_prototype_gives_each_bitcoin_method_one_exchange() {
    let server = TestServer::spawn(Mode::Valid).await;
    let session = session(&server);
    let request = request();

    let info = prototype_blockchain_info(&session)
        .await
        .expect("blockchain-info bootstrap");
    assert_eq!(server.requests().len(), 1);
    assert_eq!(info.chain, "main");
    assert!(!info.initial_block_download);

    let scan = prototype_scan(&session, &request)
        .await
        .expect("one scan operation");
    assert_eq!(server.requests().len(), 2);
    let reduced = reduce_scan(&request, scan).expect("checked scan");

    let final_hash = prototype_block_hash(&session, reduced.height)
        .await
        .expect("one block-hash confirmation");
    assert_eq!(server.requests().len(), 3);
    assert_eq!(final_hash, ANCHOR_HASH);
    assert_eq!(reduced.anchor_hash.to_string(), final_hash);

    let requests = server.requests();
    assert_eq!(
        requests
            .iter()
            .map(|request| request["method"].as_str().expect("method"))
            .collect::<Vec<_>>(),
        ["getblockchaininfo", "scantxoutset", "getblockhash"]
    );
    assert_eq!(
        requests[1]["params"][0], "start",
        "the scan wrapper must not issue status or abort"
    );
}

#[tokio::test]
async fn recoverability_prototype_resolves_only_the_admitted_routing_generation() {
    let first = TestServer::spawn(Mode::Valid).await;
    let current = TestServer::spawn(Mode::Valid).await;
    let first_generation = PrototypeRoutingGenerationRef("sha256:generation-a".to_owned());
    let current_generation = PrototypeRoutingGenerationRef("sha256:generation-b".to_owned());
    let mut resolver = PrototypeRouteResolver::default();
    resolver.install_current(first_generation.clone(), first.url.clone());
    let admitted = resolver.current.clone().expect("admitted generation");
    resolver.install_current(current_generation, current.url.clone());

    let PrototypeRouteResolution::Resolved(resumed_endpoint) = resolver.resolve_exact(&admitted)
    else {
        panic!("admitted routing generation must remain resolvable");
    };
    let resumed = BitcoinRpcSession::new(
        BitcoinRpcEndpoint::new(&resumed_endpoint).expect("endpoint"),
        None,
        binding(),
        Duration::from_secs(1),
    )
    .expect("resume session");
    prototype_blockchain_info(&resumed)
        .await
        .expect("bootstrap uses exact admitted generation");
    assert_eq!(first.requests().len(), 1);
    assert!(current.requests().is_empty());

    resolver.remove(&admitted);
    assert_eq!(
        resolver.resolve_exact(&admitted),
        PrototypeRouteResolution::DidNotEnter,
        "a missing admitted generation must not fall back to current routing"
    );
    assert!(current.requests().is_empty());
}

#[tokio::test]
async fn aggregate_collection_preseeds_zero_balances_at_one_shared_anchor() {
    let server = TestServer::spawn(Mode::Zero).await;
    let response = session(&server)
        .collect_balances(&request())
        .await
        .expect("collection");

    assert_eq!(
        response
            .balances()
            .iter()
            .map(|balance| balance.balance_sats())
            .collect::<Vec<_>>(),
        [0, 0]
    );
    assert_eq!(response.anchor_hash(), response.final_canonical_hash());
    assert_eq!(server.requests().len(), 3);
}

#[tokio::test]
async fn source_checks_stop_before_scan() {
    for mode in [Mode::ChainMismatch, Mode::InitialBlockDownload] {
        let server = TestServer::spawn(mode).await;
        assert_eq!(
            session(&server)
                .collect_balances(&request())
                .await
                .expect_err("source mismatch"),
            BitcoinCapabilityError::SourceMismatch
        );
        assert_eq!(server.requests().len(), 1);
    }
}

#[tokio::test]
async fn scan_busy_is_retryable_only_for_the_exact_core_classification() {
    for (mode, expected_retryable) in [(Mode::ScanBusy, true), (Mode::NearScanBusy, false)] {
        let server = TestServer::spawn(mode).await;
        let error = session(&server)
            .collect_balances(&request())
            .await
            .expect_err("scan failure");
        let BitcoinCapabilityError::Provider {
            diagnostic,
            retryable,
        } = error
        else {
            panic!("expected provider error");
        };
        assert_eq!(diagnostic.code(), ProviderDiagnosticCode::RpcJsonError);
        assert_eq!(retryable, expected_retryable);
        assert_eq!(server.requests().len(), 2);
    }
}

#[tokio::test]
async fn reorganization_fails_after_the_three_call_observation() {
    let server = TestServer::spawn(Mode::Reorganization).await;
    let error = session(&server)
        .collect_balances(&request())
        .await
        .expect_err("reorganization");
    assert_provider(&error, ProviderDiagnosticCode::ResponseInvalid, false);
    assert_eq!(server.requests().len(), 3);
}

#[tokio::test]
async fn scan_timeout_drops_the_request_without_status_or_abort() {
    let server = TestServer::spawn(Mode::StallScan).await;
    let error = session(&server)
        .collect_balances(&request())
        .await
        .expect_err("timeout");
    assert_provider(&error, ProviderDiagnosticCode::TransportFailed, true);
    assert_eq!(
        server
            .requests()
            .iter()
            .map(|request| request["method"].as_str().expect("method").to_owned())
            .collect::<Vec<_>>(),
        ["getblockchaininfo", "scantxoutset"]
    );
}

#[tokio::test]
async fn cancellation_drops_the_request_without_status_or_abort() {
    let server = TestServer::spawn(Mode::StallScan).await;
    let session = Arc::new(session(&server));
    let request = Arc::new(request());
    let task = {
        let session = Arc::clone(&session);
        let request = Arc::clone(&request);
        tokio::spawn(async move { session.collect_balances(&request).await })
    };
    for _ in 0..100 {
        if server.requests().len() == 2 {
            break;
        }
        tokio::task::yield_now().await;
    }
    task.abort();
    assert!(task.await.expect_err("cancelled").is_cancelled());
    assert_eq!(
        server
            .requests()
            .iter()
            .map(|request| request["method"].as_str().expect("method").to_owned())
            .collect::<Vec<_>>(),
        ["getblockchaininfo", "scantxoutset"]
    );
}

#[tokio::test]
async fn recoverability_prototype_preserves_scan_ambiguity_and_delayed_reissue() {
    let provider = PrototypeScanProvider::default();
    let ledger = Arc::new(Mutex::new(PrototypeScanAuditLedger::default()));
    let PrototypeScanStart::Entered(first) = begin_audited_scan(&provider, &ledger) else {
        panic!("first scan must enter");
    };
    let first_authorization = first.authorization;
    let first_task = tokio::spawn(first.finish());
    assert!(provider.busy.load(Ordering::SeqCst));
    assert_eq!(provider.full_scan_starts.load(Ordering::SeqCst), 1);

    let PrototypeScanStart::DidNotEnter(second_authorization) =
        begin_audited_scan(&provider, &ledger)
    else {
        panic!("concurrent scan must report busy");
    };
    assert_ne!(first_authorization, second_authorization);
    assert_eq!(
        provider.full_scan_starts.load(Ordering::SeqCst),
        1,
        "scan-busy must not synthesize or start another full scan"
    );

    first_task.abort();
    assert!(first_task
        .await
        .expect_err("cancelled wrapper")
        .is_cancelled());
    assert!(
        provider.busy.load(Ordering::SeqCst),
        "cancelling MFM's wrapper must not pretend remote work stopped"
    );
    provider.release_one();
    for _ in 0..100 {
        if !provider.busy.load(Ordering::SeqCst) {
            break;
        }
        tokio::task::yield_now().await;
    }
    assert!(!provider.busy.load(Ordering::SeqCst));

    let PrototypeScanStart::Entered(third) = begin_audited_scan(&provider, &ledger) else {
        panic!("delayed reissue must be a new full scan");
    };
    let third_authorization = third.authorization;
    assert_ne!(third_authorization, first_authorization);
    assert_ne!(third_authorization, second_authorization);
    provider.release_one();
    let returned = third.finish().await.expect("delayed scan result");
    assert_eq!(returned.height, 850_000);
    assert_eq!(returned.best_block, ANCHOR_HASH);

    let state = ledger.lock().expect("scan audit ledger");
    assert_eq!(
        state.observations,
        [
            (second_authorization, PrototypeScanObservation::DidNotEnter,),
            (first_authorization, PrototypeScanObservation::Indeterminate,),
            (third_authorization, PrototypeScanObservation::Returned,),
        ]
    );
    assert_eq!(provider.protocol_calls.load(Ordering::SeqCst), 3);
    assert_eq!(provider.full_scan_starts.load(Ordering::SeqCst), 2);
    assert!(provider
        .methods
        .lock()
        .expect("scan methods")
        .iter()
        .all(|method| *method == "scantxoutset:start"));
}

#[tokio::test]
async fn recoverability_prototype_lost_scan_response_is_indeterminate() {
    let provider = PrototypeScanProvider::default();
    provider.drop_next_response.store(true, Ordering::SeqCst);
    let ledger = Arc::new(Mutex::new(PrototypeScanAuditLedger::default()));
    let PrototypeScanStart::Entered(first) = begin_audited_scan(&provider, &ledger) else {
        panic!("first scan must enter");
    };
    let first_authorization = first.authorization;
    provider.release_one();
    assert_eq!(first.finish().await, None);

    let PrototypeScanStart::Entered(reissued) = begin_audited_scan(&provider, &ledger) else {
        panic!("lost response reissue starts a fresh scan");
    };
    let reissued_authorization = reissued.authorization;
    provider.release_one();
    assert!(reissued.finish().await.is_some());

    assert_ne!(first_authorization, reissued_authorization);
    assert_eq!(provider.full_scan_starts.load(Ordering::SeqCst), 2);
    assert_eq!(
        ledger.lock().expect("scan audit ledger").observations,
        [
            (first_authorization, PrototypeScanObservation::Indeterminate,),
            (reissued_authorization, PrototypeScanObservation::Returned,),
        ]
    );
}

#[test]
fn recoverability_prototype_keeps_bitcoin_production_unregistered() {
    assert!(request().addresses().len() <= BITCOIN_BALANCE_COLLECTION_ADDRESS_LIMIT);
    assert_eq!(MAX_BITCOIN_JSON_RPC_BODY_BYTES, 16 * 1024 * 1024);
    let qualification = PrototypeScanQualification {
        no_durable_domain_mutation: true,
        indivisible_snapshot: true,
        bounded_descriptors: true,
        bounded_retained_result: true,
        start_only: true,
        no_hidden_retry_or_control_call: true,
        bounded_provider_work: false,
        repeated_cost_and_shared_concurrency_accepted: false,
    };

    assert_eq!(
        qualification.disposition(),
        PrototypeBitcoinRegistrationDisposition::Unregistered(vec![
            PrototypeScanQualificationGap::BoundedProviderWork,
            PrototypeScanQualificationGap::RepeatedCostAndSharedConcurrencyAcceptance,
        ]),
        "client timeout and body bounds do not bound Bitcoin Core work or accept repeated cost"
    );
}

#[tokio::test]
async fn protocol_and_http_failures_are_closed_and_redacted() {
    for mode in [
        Mode::BadVersion,
        Mode::BadId,
        Mode::ResultAndError,
        Mode::MissingArms,
        Mode::OversizedLength,
        Mode::OversizedChunked,
    ] {
        let server = TestServer::spawn(mode).await;
        let error = session(&server)
            .collect_balances(&request())
            .await
            .expect_err("invalid response");
        assert_provider(&error, ProviderDiagnosticCode::ResponseInvalid, false);
    }

    let server = TestServer::spawn(Mode::HttpFailure).await;
    let error = session(&server)
        .collect_balances(&request())
        .await
        .expect_err("HTTP failure");
    assert_provider(&error, ProviderDiagnosticCode::RpcHttpStatus, true);
    assert_eq!(server.requests().len(), 1, "HTTP failures must not retry");
    let rendered = format!("{error:?} {error}");
    assert!(!rendered.contains("provider-secret"));

    for mode in [Mode::Created, Mode::Accepted, Mode::NoContent] {
        let server = TestServer::spawn(mode).await;
        let error = session(&server)
            .collect_balances(&request())
            .await
            .expect_err("non-200 success status");
        assert_provider(&error, ProviderDiagnosticCode::RpcHttpStatus, false);
        assert_eq!(
            server.requests().len(),
            1,
            "a non-200 success status must fail at the first call"
        );
    }

    let server = TestServer::spawn(Mode::Redirect).await;
    let error = session(&server)
        .collect_balances(&request())
        .await
        .expect_err("redirect");
    assert_provider(&error, ProviderDiagnosticCode::RpcHttpStatus, false);
    assert_eq!(server.requests().len(), 1, "redirects must not be followed");
}

#[tokio::test]
async fn adversarial_provider_material_cannot_reach_retained_failure_surfaces() {
    for (mode, expected_code) in [
        (Mode::HttpFailure, ProviderDiagnosticCode::RpcHttpStatus),
        (
            Mode::ResultAndError,
            ProviderDiagnosticCode::ResponseInvalid,
        ),
        (
            Mode::AdversarialMalformed,
            ProviderDiagnosticCode::ResponseInvalid,
        ),
        (
            Mode::OversizedChunked,
            ProviderDiagnosticCode::ResponseInvalid,
        ),
        (Mode::ScanBusy, ProviderDiagnosticCode::RpcJsonError),
    ] {
        let server = TestServer::spawn(mode).await;
        let session = authenticated_fixture_session(&server);
        let error = session
            .collect_balances(&request())
            .await
            .expect_err("adversarial response");
        let BitcoinCapabilityError::Provider { diagnostic, .. } = &error else {
            panic!("provider failure");
        };
        assert_eq!(diagnostic.code(), expected_code);

        let raw = server.raw_requests().join("\n");
        assert!(raw.contains(FIXTURE_ENDPOINT_PATH));
        let authorization_header = raw
            .lines()
            .find(|line| {
                line.to_ascii_lowercase()
                    .starts_with("authorization: basic ")
            })
            .expect("server observed Basic authorization");
        let authorization_encoding = authorization_header
            .split_once(':')
            .expect("authorization header")
            .1
            .trim();
        let retained_json = serde_json::to_string(diagnostic).expect("typed diagnostic JSON");
        let rendered = format!("{error:?} {error} {diagnostic:?} {diagnostic} {retained_json}");
        let endpoint_url = format!("{}/{FIXTURE_ENDPOINT_PATH}", server.url);
        assert_synthetic_material_absent(
            &rendered,
            &[
                &server.url,
                &endpoint_url,
                FIXTURE_ENDPOINT_PATH,
                FIXTURE_BASIC_USER,
                FIXTURE_BASIC_PASSWORD,
                authorization_header,
                authorization_encoding,
                FIXTURE_PROVIDER_BODY,
                FIXTURE_PROVIDER_URL,
                FIXTURE_LOW_ENTROPY,
                "123456",
            ],
        );
    }
}

#[tokio::test]
async fn retained_bitcoin_evidence_excludes_endpoint_and_basic_authentication() {
    let server = TestServer::spawn(Mode::Valid).await;
    let session = authenticated_fixture_session(&server);
    let response = session
        .collect_balances(&request())
        .await
        .expect("checked response");
    let evidence = BitcoinBalanceCollectionEvidence::from_response(&response);
    let retained_json = serde_json::to_string(&evidence).expect("retained evidence");

    let raw = server.raw_requests().join("\n");
    let authorization_header = raw
        .lines()
        .find(|line| {
            line.to_ascii_lowercase()
                .starts_with("authorization: basic ")
        })
        .expect("server observed Basic authorization");
    let authorization_encoding = authorization_header
        .split_once(':')
        .expect("authorization header")
        .1
        .trim();
    let endpoint_url = format!("{}/{FIXTURE_ENDPOINT_PATH}", server.url);
    let rendered = format!("{session:?} {evidence:?} {retained_json}");
    assert_synthetic_material_absent(
        &rendered,
        &[
            &server.url,
            &endpoint_url,
            FIXTURE_ENDPOINT_PATH,
            FIXTURE_BASIC_USER,
            FIXTURE_BASIC_PASSWORD,
            authorization_header,
            authorization_encoding,
            FIXTURE_LOW_ENTROPY,
            "123456",
        ],
    );
}

#[tokio::test]
async fn transport_failure_discards_the_internal_bitcoin_endpoint_error() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("reserve port");
    let address = listener.local_addr().expect("reserved address");
    drop(listener);
    let endpoint_url = format!("http://{address}/{FIXTURE_ENDPOINT_PATH}");
    let session = BitcoinRpcSession::new(
        BitcoinRpcEndpoint::new(&endpoint_url).expect("endpoint"),
        Some(
            BitcoinRpcAuthentication::new(
                FIXTURE_BASIC_USER.to_owned(),
                zeroize::Zeroizing::new(FIXTURE_BASIC_PASSWORD.to_owned()),
            )
            .expect("authentication"),
        ),
        binding(),
        Duration::from_secs(1),
    )
    .expect("session");
    let error = session
        .collect_balances(&request())
        .await
        .expect_err("connection must fail");
    let BitcoinCapabilityError::Provider { diagnostic, .. } = &error else {
        panic!("provider failure");
    };
    let rendered = format!("{error:?} {error} {diagnostic:?} {diagnostic}");
    assert_eq!(diagnostic.code(), ProviderDiagnosticCode::TransportFailed);
    assert_synthetic_material_absent(
        &rendered,
        &[
            &endpoint_url,
            FIXTURE_ENDPOINT_PATH,
            FIXTURE_BASIC_USER,
            FIXTURE_BASIC_PASSWORD,
            FIXTURE_LOW_ENTROPY,
            "123456",
        ],
    );
}

#[test]
fn configuration_and_debug_surfaces_reject_or_redact_secret_bearing_inputs() {
    for endpoint in [
        "ftp://127.0.0.1",
        "http://user:password@127.0.0.1",
        "http://127.0.0.1?secret=value",
        "http://127.0.0.1#secret",
    ] {
        assert!(matches!(
            BitcoinRpcEndpoint::new(endpoint),
            Err(BitcoinRpcError::InvalidConfiguration)
        ));
    }
    for timeout in [Duration::ZERO, Duration::from_secs(86_401)] {
        assert!(matches!(
            BitcoinRpcSession::new(
                BitcoinRpcEndpoint::new("http://127.0.0.1").expect("endpoint"),
                None,
                binding(),
                timeout
            ),
            Err(BitcoinRpcError::InvalidConfiguration)
        ));
    }
    assert!(matches!(
        BitcoinRpcAuthentication::new(
            "".to_owned(),
            zeroize::Zeroizing::new("password".to_owned())
        ),
        Err(BitcoinRpcError::InvalidConfiguration)
    ));

    let authentication = BitcoinRpcAuthentication::new(
        "secret-user".to_owned(),
        zeroize::Zeroizing::new("secret-password".to_owned()),
    )
    .expect("authentication");
    let session = BitcoinRpcSession::new(
        BitcoinRpcEndpoint::new("http://127.0.0.1:18443/secret-path").expect("endpoint"),
        Some(authentication),
        binding(),
        Duration::from_secs(1),
    )
    .expect("session");
    let rendered = format!("{session:?}");
    for secret in ["secret-user", "secret-password", "secret-path", "18443"] {
        assert!(!rendered.contains(secret));
    }
}

#[test]
fn raw_decimal_parser_is_exact_and_bounded() {
    for (raw, expected) in [
        ("0", 0),
        ("0.", 0),
        ("0.00000001", 1),
        ("1.2", 120_000_000),
        ("21000000", 2_100_000_000_000_000),
    ] {
        assert_eq!(parse_btc_amount(raw).expect(raw), expected);
    }
    for invalid in [
        "",
        ".1",
        "+1",
        "-1",
        "1e-8",
        "1E8",
        "0.000000001",
        "21000000.00000001",
        "18446744073709551616",
        "1.2.3",
    ] {
        assert_eq!(
            parse_btc_amount(invalid),
            Err(BitcoinRpcError::ResponseInvalid),
            "{invalid}"
        );
    }
}

#[test]
fn scan_reduction_rejects_duplicate_outpoints_unknown_scripts_and_bad_totals() {
    let legacy_script = script_hex(LEGACY_MAIN);
    let segwit_script = script_hex(SEGWIT_MAIN);
    let valid = |unspents: &str, total: &str| {
        format!(
            r#"{{"success":true,"height":850000,"bestblock":"{ANCHOR_HASH}","txouts":7,"unspents":[{unspents}],"total_amount":{total}}}"#
        )
    };
    let first = unspent(TXID_ONE, 0, &legacy_script, "0.00000001", 849_999);
    let duplicate = format!("{first},{first}");
    assert_reduce_invalid(&valid(&duplicate, "0.00000002"));

    let unknown = unspent(TXID_TWO, 0, "00", "0.00000001", 849_999);
    assert_reduce_invalid(&valid(&unknown, "0.00000001"));

    let known = format!(
        "{},{}",
        first,
        unspent(TXID_TWO, 1, &segwit_script, "0.00000002", 850_000)
    );
    assert_reduce_invalid(&valid(&known, "0.00000004"));
    assert_reduce_invalid(&valid(
        &unspent(TXID_TWO, u64::from(u32::MAX) + 1, &legacy_script, "1", 1),
        "1",
    ));
    assert_reduce_invalid(&valid(
        &unspent(TXID_TWO, 0, &legacy_script, "1", 850_001),
        "1",
    ));
}

#[test]
fn strict_decoders_require_known_fields_and_reject_duplicate_members_at_any_depth() {
    let required = format!(
        r#"{{"success":true,"height":850000,"bestblock":"{ANCHOR_HASH}","unspents":[],"total_amount":0}}"#
    );
    assert!(matches!(
        decode_unique::<ScanTxOutSetResult>(required.as_bytes()),
        Err(BitcoinRpcError::ResponseInvalid)
    ));
    let negative_txouts = format!(
        r#"{{"success":true,"height":850000,"bestblock":"{ANCHOR_HASH}","txouts":-1,"unspents":[],"total_amount":0}}"#
    );
    assert!(matches!(
        decode_unique::<ScanTxOutSetResult>(negative_txouts.as_bytes()),
        Err(BitcoinRpcError::ResponseInvalid)
    ));

    for duplicate in [
        br#"{"jsonrpc":"2.0","id":1,"id":1,"result":{}}"#.as_slice(),
        br#"{"jsonrpc":"2.0","id":1,"error":{"code":-8,"code":-8,"message":"x"}}"#,
    ] {
        assert!(matches!(
            decode_unique::<RpcEnvelope>(duplicate),
            Err(BitcoinRpcError::ResponseInvalid)
        ));
    }

    let script = script_hex(LEGACY_MAIN);
    let duplicate_unspent = format!(
        r#"{{"success":true,"height":850000,"bestblock":"{ANCHOR_HASH}","txouts":1,"unspents":[{{"txid":"{TXID_ONE}","vout":0,"scriptPubKey":"{script}","scriptPubKey":"{script}","desc":"ignored","amount":1,"height":1}}],"total_amount":1}}"#
    );
    assert!(matches!(
        decode_unique::<ScanTxOutSetResult>(duplicate_unspent.as_bytes()),
        Err(BitcoinRpcError::ResponseInvalid)
    ));

    let additive = format!(
        r#"{{"success":true,"height":850000,"bestblock":"{ANCHOR_HASH}","txouts":0,"unspents":[],"total_amount":0,"future":{{"nested":{{"unique":true}}}}}}"#
    );
    let decoded =
        decode_unique::<ScanTxOutSetResult>(additive.as_bytes()).expect("additive fields");
    assert!(reduce_scan(&request(), decoded).is_ok());

    let duplicate_unknown =
        br#"{"chain":"main","initialblockdownload":false,"future":{"value":1,"value":2}}"#;
    assert!(matches!(
        decode_unique::<BlockchainInfo>(duplicate_unknown),
        Err(BitcoinRpcError::ResponseInvalid)
    ));

    let oversized_message = format!(
        r#"{{"jsonrpc":"2.0","id":1,"error":{{"code":-1,"message":"{}"}}}}"#,
        "x".repeat(MAX_JSON_RPC_ERROR_MESSAGE_BYTES + 1)
    );
    assert!(matches!(
        decode_unique::<RpcEnvelope>(oversized_message.as_bytes()),
        Err(BitcoinRpcError::ResponseInvalid)
    ));
}

#[test]
fn remaining_scan_semantics_fail_closed_before_response_construction() {
    for malformed in ["0", "zz"] {
        assert_eq!(
            decode_script(malformed),
            Err(BitcoinRpcError::ResponseInvalid)
        );
    }
    assert_eq!(
        decode_script(&"00".repeat(MAX_SCRIPT_BYTES + 1)),
        Err(BitcoinRpcError::ResponseInvalid)
    );

    let incomplete = format!(
        r#"{{"success":false,"height":850000,"bestblock":"{ANCHOR_HASH}","txouts":0,"unspents":[],"total_amount":0}}"#
    );
    let scan = decode_unique::<ScanTxOutSetResult>(incomplete.as_bytes()).expect("typed scan");
    assert!(matches!(
        reduce_scan(&request(), scan),
        Err(BitcoinRpcError::OperationIncomplete)
    ));

    let legacy_script = script_hex(LEGACY_MAIN);
    for invalid in [
        r#"{"success":true,"height":850000,"bestblock":"invalid","txouts":1,"unspents":[],"total_amount":0}"#.to_owned(),
        format!(
            r#"{{"success":true,"height":850000,"bestblock":"{ANCHOR_HASH}","txouts":1,"unspents":[{}],"total_amount":0.00000001}}"#,
            unspent("invalid", 0, &legacy_script, "0.00000001", 1)
        ),
        format!(
            r#"{{"success":true,"height":850000,"bestblock":"{ANCHOR_HASH}","txouts":1,"unspents":[{}],"total_amount":0.00000001}}"#,
            unspent(TXID_ONE, 0, "0", "0.00000001", 1)
        ),
        format!(
            r#"{{"success":true,"height":850000,"bestblock":"{ANCHOR_HASH}","txouts":1,"unspents":[{{"txid":"{TXID_ONE}","vout":0,"scriptPubKey":"{legacy_script}","desc":"{}","amount":0.00000001,"height":1}}],"total_amount":0.00000001}}"#,
            "x".repeat(MAX_DESCRIPTOR_BYTES + 1)
        ),
    ] {
        assert_reduce_invalid(&invalid);
    }

    for malformed_txouts in [
        format!(
            r#"{{"success":true,"height":850000,"bestblock":"{ANCHOR_HASH}","txouts":"1","unspents":[],"total_amount":0}}"#
        ),
        format!(
            r#"{{"success":true,"height":850000,"bestblock":"{ANCHOR_HASH}","txouts":18446744073709551616,"unspents":[],"total_amount":0}}"#
        ),
    ] {
        assert!(matches!(
            decode_unique::<ScanTxOutSetResult>(malformed_txouts.as_bytes()),
            Err(BitcoinRpcError::ResponseInvalid)
        ));
    }
}

fn assert_reduce_invalid(raw: &str) {
    let scan = decode_unique::<ScanTxOutSetResult>(raw.as_bytes()).expect("typed scan");
    assert!(matches!(
        reduce_scan(&request(), scan),
        Err(BitcoinRpcError::ResponseInvalid)
    ));
}

fn assert_provider(
    error: &BitcoinCapabilityError,
    expected_code: ProviderDiagnosticCode,
    expected_retryable: bool,
) {
    let BitcoinCapabilityError::Provider {
        diagnostic,
        retryable,
    } = error
    else {
        panic!("expected provider error");
    };
    assert_eq!(diagnostic.code(), expected_code);
    assert_eq!(*retryable, expected_retryable);
}

fn script_hex(address: &str) -> String {
    hex::encode(
        BitcoinAddress::parse(address, BitcoinNetworkTag::Main)
            .expect("address")
            .script_pubkey()
            .as_bytes(),
    )
}

fn unspent(txid: &str, vout: u64, script: &str, amount: &str, height: u64) -> String {
    format!(
        r#"{{"txid":"{txid}","vout":{vout},"scriptPubKey":"{script}","desc":"ignored","amount":{amount},"height":{height}}}"#
    )
}

fn response(mode: Mode, request: &Value) -> String {
    if matches!(mode, Mode::HttpFailure) {
        return http_response(
            "500 Internal Server Error",
            &format!(
                r#"{{"body":"{FIXTURE_PROVIDER_BODY}","url":"{FIXTURE_PROVIDER_URL}","credential":"{FIXTURE_BASIC_PASSWORD}","pin":"{FIXTURE_LOW_ENTROPY}"}}"#
            ),
        );
    }
    if matches!(mode, Mode::Redirect) {
        return "HTTP/1.1 302 Found\r\nlocation: http://127.0.0.1:9/provider-secret\r\ncontent-length: 0\r\nconnection: close\r\n\r\n".to_owned();
    }
    if matches!(mode, Mode::Created | Mode::Accepted | Mode::NoContent) {
        let status = match mode {
            Mode::Created => "201 Created",
            Mode::Accepted => "202 Accepted",
            Mode::NoContent => "204 No Content",
            _ => unreachable!("non-200 success mode"),
        };
        return http_response(status, "");
    }
    if matches!(mode, Mode::OversizedLength) {
        return format!(
            "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
            MAX_BITCOIN_JSON_RPC_BODY_BYTES + 1
        );
    }
    if matches!(mode, Mode::OversizedChunked) {
        let marker = format!("{FIXTURE_PROVIDER_BODY}{FIXTURE_LOW_ENTROPY}");
        let body = marker.repeat(MAX_BITCOIN_JSON_RPC_BODY_BYTES / marker.len() + 1);
        return format!(
            "HTTP/1.1 200 OK\r\ntransfer-encoding: chunked\r\nconnection: close\r\n\r\n{:x}\r\n{}\r\n0\r\n\r\n",
            body.len(),
            body
        );
    }
    if matches!(mode, Mode::AdversarialMalformed) {
        let body = format!(
            "{FIXTURE_PROVIDER_BODY} {FIXTURE_PROVIDER_URL} {FIXTURE_BASIC_PASSWORD} {FIXTURE_LOW_ENTROPY}"
        );
        return http_response("200 OK", &body);
    }

    let id = request["id"].as_u64().expect("request ID");
    if matches!(mode, Mode::BadVersion) {
        return json_response(
            id,
            "1.1",
            r#"{"chain":"main","initialblockdownload":false}"#,
        );
    }
    if matches!(mode, Mode::BadId) {
        return json_response(
            id + 1,
            "2.0",
            r#"{"chain":"main","initialblockdownload":false}"#,
        );
    }
    if matches!(mode, Mode::ResultAndError) {
        let body = format!(
            r#"{{"jsonrpc":"2.0","id":{id},"result":{{}},"error":{{"code":-1,"message":"{FIXTURE_PROVIDER_BODY} {FIXTURE_PROVIDER_URL} {FIXTURE_LOW_ENTROPY}","data":{{"credential":"{FIXTURE_BASIC_PASSWORD}","token":"123456"}}}}}}"#
        );
        return http_response("200 OK", &body);
    }
    if matches!(mode, Mode::MissingArms) {
        let body = format!(r#"{{"jsonrpc":"2.0","id":{id}}}"#);
        return http_response("200 OK", &body);
    }

    let method = request["method"].as_str().expect("method");
    if method == "scantxoutset" && matches!(mode, Mode::ScanBusy | Mode::NearScanBusy) {
        let message = if matches!(mode, Mode::ScanBusy) {
            "Scan already in progress: use status to check"
        } else {
            "scan already in progress"
        };
        let body = format!(
            r#"{{"jsonrpc":"2.0","id":{id},"error":{{"code":-8,"message":"{message}","data":{{"body":"{FIXTURE_PROVIDER_BODY}","url":"{FIXTURE_PROVIDER_URL}","credential":"{FIXTURE_BASIC_PASSWORD}","pin":"{FIXTURE_LOW_ENTROPY}"}}}}}}"#
        );
        return http_response("200 OK", &body);
    }

    let result = match method {
        "getblockchaininfo" => {
            let chain = if matches!(mode, Mode::ChainMismatch) {
                "test"
            } else {
                "main"
            };
            let ibd = matches!(mode, Mode::InitialBlockDownload);
            format!(
                r#"{{"chain":"{chain}","initialblockdownload":{ibd},"blocks":850000,"bestblockhash":"{ANCHOR_HASH}","future":{{"nested":true}}}}"#
            )
        }
        "scantxoutset" if matches!(mode, Mode::Zero) => format!(
            r#"{{"success":true,"height":850000,"bestblock":"{ANCHOR_HASH}","txouts":99,"unspents":[],"total_amount":0,"future":true}}"#
        ),
        "scantxoutset" => valid_scan_result(),
        "getblockhash" => format!(
            "\"{}\"",
            if matches!(mode, Mode::Reorganization) {
                OTHER_HASH
            } else {
                ANCHOR_HASH
            }
        ),
        other => panic!("unexpected method {other}"),
    };
    json_response(id, "2.0", &result)
}

fn valid_scan_result() -> String {
    let legacy_script = script_hex(LEGACY_MAIN);
    let segwit_script = script_hex(SEGWIT_MAIN);
    format!(
        r#"{{"success":true,"height":850000,"bestblock":"{ANCHOR_HASH}","txouts":12345,"unspents":[{},{},{}],"total_amount":0.00000007,"future":{{"nested":true}}}}"#,
        unspent(TXID_ONE, 0, &legacy_script, "0.00000001", 849_998),
        unspent(TXID_TWO, 1, &legacy_script, "0.00000002", 849_999),
        unspent(
            "3333333333333333333333333333333333333333333333333333333333333333",
            2,
            &segwit_script,
            "0.00000004",
            850_000,
        )
    )
}

fn json_response(id: u64, version: &str, result: &str) -> String {
    let body = format!(r#"{{"jsonrpc":"{version}","id":{id},"result":{result}}}"#);
    http_response("200 OK", &body)
}

fn http_response(status: &str, body: &str) -> String {
    format!(
        "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    )
}

async fn read_http_request(stream: &mut tokio::net::TcpStream) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 8 * 1024];
    loop {
        let count = stream.read(&mut chunk).await.expect("request read");
        assert_ne!(count, 0, "request ended before the body was complete");
        bytes.extend_from_slice(&chunk[..count]);
        if request_complete(&bytes) {
            return bytes;
        }
    }
}

fn request_complete(bytes: &[u8]) -> bool {
    let Some(header_end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") else {
        return false;
    };
    let headers = String::from_utf8_lossy(&bytes[..header_end]);
    let content_len = headers
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())
                .flatten()
        })
        .unwrap_or_default();
    bytes.len() >= header_end + 4 + content_len
}

fn request_body(bytes: &[u8]) -> &[u8] {
    let header_end = bytes
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .expect("headers");
    &bytes[header_end + 4..]
}
