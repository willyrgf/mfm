use super::*;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, AtomicUsize, Ordering};

use alloy_eips::eip2930::AccessList;
use alloy_primitives::{
    address, b256, hex, keccak256, Address, PrimitiveSignature, TxKind, B256, U256,
};
use mfm_evm_capabilities::{
    EvmBlock, EvmFeeInputs, EvmObservedTransaction, EvmSessionEvidence, EvmSessionFuture,
    EvmTransactionEstimate,
};
use mfm_ids::LocalPublicId;
use mfm_signing::{
    DeterministicSigningProvider, PublicSigningIdentity, SignatureBytes, SigningFuture,
    SigningProvider, SigningRequest, SigningResult, SECP256K1_RFC6979_LOW_S_PROFILE_ID,
};

const EXPECTED_SENDER: Address = address!("dd6b8b3dc6b7ad97db52f08a275ff4483e024cea");
const DESTINATION: Address = address!("6069a6c32cf691f5982febae4faf8a6f3ab2f0f6");
const EXPECTED_HASH: B256 =
    b256!("0ec0b6a2df4d87424e5f6ad2a654e27aaeb7dac20ae9e8385cc09087ad532ee0");

#[derive(Clone, Copy)]
enum LookupMode {
    Missing = 0,
    Observed = 1,
    Mismatched = 2,
}

struct MockSession {
    evidence: EvmSessionEvidence,
    lookup_mode: AtomicU8,
    return_wrong_hash: AtomicBool,
    pending_nonce: AtomicU64,
    pending_nonce_calls: AtomicUsize,
    submitted: Mutex<Vec<Vec<u8>>>,
}

impl MockSession {
    fn new(mode: LookupMode) -> Self {
        Self::with_evidence_chain(mode, 1)
    }

    fn with_evidence_chain(mode: LookupMode, chain_id: u64) -> Self {
        let binding = EvmNetworkBinding::new(
            LocalPublicId::new("ethereum-mainnet").expect("network"),
            chain_id,
        )
        .expect("binding");
        Self {
            evidence: EvmSessionEvidence::new(
                &binding,
                LocalPublicId::new("primary").expect("source"),
                LocalPublicId::new(EVM_JSONRPC_SESSION_IMPLEMENTATION_ID).expect("implementation"),
            ),
            lookup_mode: AtomicU8::new(mode as u8),
            return_wrong_hash: AtomicBool::new(false),
            pending_nonce: AtomicU64::new(0x42),
            pending_nonce_calls: AtomicUsize::new(0),
            submitted: Mutex::new(Vec::new()),
        }
    }

    fn set_lookup_mode(&self, mode: LookupMode) {
        self.lookup_mode.store(mode as u8, Ordering::SeqCst);
    }

    fn set_pending_nonce(&self, nonce: u64) {
        self.pending_nonce.store(nonce, Ordering::SeqCst);
    }

    fn observation(&self) -> EvmObservedTransaction {
        let mut nonce = U256::from(0x42);
        if self.lookup_mode.load(Ordering::SeqCst) == LookupMode::Mismatched as u8 {
            nonce += U256::from(1);
        }
        EvmObservedTransaction {
            transaction_hash: EXPECTED_HASH,
            chain_id: U256::from(1),
            nonce,
            from: EXPECTED_SENDER,
            to: TxKind::Call(DESTINATION),
            value: U256::ZERO,
            input: vector_calldata().into(),
            gas_limit: U256::from(44_386),
            max_fee_per_gas: U256::from(0x4a817c800_u64),
            max_priority_fee_per_gas: U256::from(0x3b9aca00_u64),
            access_list: AccessList::default(),
            placement: None,
        }
    }
}

impl EvmTransactionSession for MockSession {
    fn evidence(&self) -> &EvmSessionEvidence {
        &self.evidence
    }

    fn pending_nonce<'a>(&'a self, _account: Address) -> EvmSessionFuture<'a, U256> {
        self.pending_nonce_calls.fetch_add(1, Ordering::SeqCst);
        let nonce = self.pending_nonce.load(Ordering::SeqCst);
        Box::pin(async move { Ok(U256::from(nonce)) })
    }

    fn fee_inputs(&self) -> EvmSessionFuture<'_, EvmFeeInputs> {
        Box::pin(async {
            EvmFeeInputs::from_base_and_priority(
                U256::from(9_500_000_000_u64),
                U256::from(1_000_000_000_u64),
            )
        })
    }

    fn estimate_gas<'a>(
        &'a self,
        request: &'a EvmTransactionEstimate,
    ) -> EvmSessionFuture<'a, U256> {
        assert_eq!(request.from(), EXPECTED_SENDER);
        assert_eq!(request.to(), TxKind::Call(DESTINATION));
        assert_eq!(request.input().as_ref(), vector_calldata());
        Box::pin(async { Ok(U256::from(44_386)) })
    }

    fn submit_raw_transaction<'a>(
        &'a self,
        signed_bytes: &'a [u8],
        expected_hash: B256,
    ) -> EvmSessionFuture<'a, B256> {
        assert_eq!(expected_hash, EXPECTED_HASH);
        assert_eq!(keccak256(signed_bytes), EXPECTED_HASH);
        self.submitted
            .lock()
            .expect("submitted lock")
            .push(signed_bytes.to_vec());
        let returned = if self.return_wrong_hash.load(Ordering::SeqCst) {
            B256::from([0xee; 32])
        } else {
            expected_hash
        };
        Box::pin(async move { Ok(returned) })
    }

    fn transaction_by_hash(
        &self,
        _transaction_hash: B256,
    ) -> EvmSessionFuture<'_, Option<EvmObservedTransaction>> {
        let observation = match self.lookup_mode.load(Ordering::SeqCst) {
            value if value == LookupMode::Missing as u8 => None,
            _ => Some(self.observation()),
        };
        Box::pin(async move { Ok(observation) })
    }

    fn receipt_by_hash(
        &self,
        _transaction_hash: B256,
    ) -> EvmSessionFuture<'_, Option<mfm_evm_capabilities::EvmReceipt>> {
        Box::pin(async { Ok(None) })
    }

    fn read_block<'a>(&'a self, _selector: &'a EvmBlockSelector) -> EvmSessionFuture<'a, EvmBlock> {
        Box::pin(async {
            Ok(EvmBlock {
                number: U256::ZERO,
                hash: B256::ZERO,
            })
        })
    }
}

struct FixedProvider {
    calls: AtomicUsize,
}

impl FixedProvider {
    fn new() -> Self {
        Self {
            calls: AtomicUsize::new(0),
        }
    }
}

impl SigningProvider for FixedProvider {
    fn sign<'a>(&'a self, request: &'a SigningRequest) -> SigningFuture<'a> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let signature = PrimitiveSignature::from_scalars_and_parity(
            b256!("840cfc572845f5786e702984c2a582528cad4b49b2a10b9db1be7fca90058565"),
            b256!("25e7109ceb98168d95b09b18bbf6b685130e0562f233877d492b94eee0c5b6d1"),
            false,
        );
        let identity = PublicSigningIdentity::new(
            request.algorithm().clone(),
            None,
            Some(format!("{EXPECTED_SENDER:?}")),
        )
        .expect("identity");
        let result = SigningResult::for_request(
            request,
            identity,
            SignatureBytes::new(signature.as_bytes().to_vec()).expect("signature"),
        );
        Box::pin(async move { result })
    }
}

impl DeterministicSigningProvider for FixedProvider {
    fn implementation_id(&self) -> &'static str {
        "mfm.test.deterministic-signer"
    }

    fn deterministic_profile_id(&self) -> &'static str {
        SECP256K1_RFC6979_LOW_S_PROFILE_ID
    }
}

struct MismatchedProvider(Arc<FixedProvider>);

impl SigningProvider for MismatchedProvider {
    fn sign<'a>(&'a self, request: &'a SigningRequest) -> SigningFuture<'a> {
        self.0.sign(request)
    }
}

impl DeterministicSigningProvider for MismatchedProvider {
    fn implementation_id(&self) -> &'static str {
        "mfm.test.mismatched-signer"
    }

    fn deterministic_profile_id(&self) -> &'static str {
        SECP256K1_RFC6979_LOW_S_PROFILE_ID
    }
}

struct MissingArtifacts;

impl store::RetainedArtifactReadProvider for MissingArtifacts {
    fn read_retained_artifact<'a>(
        &'a self,
        requirement: &'a store::EventArtifactRequirement,
    ) -> store::RetainedArtifactReadFuture<'a> {
        let artifact_id = requirement.artifact_id.clone();
        Box::pin(async move { Err(store::StoreError::MissingArtifact { artifact_id }) })
    }
}

fn vector_calldata() -> Vec<u8> {
    hex!("a22cb4650000000000000000000000005eee75727d804a2b13038928d36f8b188945a57a0000000000000000000000000000000000000000000000000000000000000000").to_vec()
}

fn intent() -> EvmTransactionIntent {
    let config = EvmTransactionConfig::new(
        "ethereum-mainnet",
        1,
        EXPECTED_SENDER,
        mfm_signing::SignerRef::new("deployer").expect("signer"),
        Vec::new(),
    )
    .expect("config");
    EvmTransactionIntent::from_config(
        &config,
        &EvmTransactionAction::call(DESTINATION, vector_calldata(), U256::ZERO).expect("call"),
    )
    .expect("intent")
}

fn other_intent() -> EvmTransactionIntent {
    let config = EvmTransactionConfig::new(
        "ethereum-mainnet",
        1,
        EXPECTED_SENDER,
        mfm_signing::SignerRef::new("deployer").expect("signer"),
        Vec::new(),
    )
    .expect("config");
    EvmTransactionIntent::from_config(
        &config,
        &EvmTransactionAction::call(DESTINATION, [0x01], U256::ZERO).expect("other call"),
    )
    .expect("other intent")
}

fn make_adapter(session: Arc<MockSession>, signer: Arc<FixedProvider>) -> EvmTransactionAdapter {
    let bind_session = Arc::clone(&session);
    let bind_signer = Arc::clone(&signer);
    let signer_binder = mfm_signing::DeterministicSigningProviderBinder::new(
        "mfm.test.deterministic-signer",
        move |_signer_ref| {
            let signer = Arc::clone(&bind_signer);
            Box::pin(async move { Ok(signer as Arc<dyn DeterministicSigningProvider>) })
        },
    )
    .expect("signer binder");
    EvmTransactionAdapter::new(EvmTransactionRunnerCapabilities::new(
        Arc::new(MissingArtifacts),
        signer_binder,
        |_binding, _signer_ref| Box::pin(async { Ok(()) }),
        move |_binding| {
            let session = Arc::clone(&bind_session);
            Box::pin(async move { Ok(session as Arc<dyn EvmTransactionSession>) })
        },
    ))
}

async fn prepare_and_commit(adapter: &EvmTransactionAdapter) -> EvmPreparedTransaction {
    let prepared = adapter
        .prepare_transaction(&intent())
        .await
        .expect("prepare");
    let (prepared, settlement) = prepared.into_parts();
    settlement
        .expect("EVM preparation settlement")
        .settle_appended();
    prepared
}

#[tokio::test]
async fn normal_path_signs_once_and_submits_cached_exact_bytes() {
    let session = Arc::new(MockSession::new(LookupMode::Observed));
    let signer = Arc::new(FixedProvider::new());
    let adapter = make_adapter(Arc::clone(&session), Arc::clone(&signer));

    let prepared = prepare_and_commit(&adapter).await;
    assert_eq!(prepared.expected_hash().expect("hash"), EXPECTED_HASH);
    assert_eq!(signer.calls.load(Ordering::SeqCst), 1);
    let decision = adapter.submit_transaction(prepared).await.expect("submit");

    assert!(matches!(
        decision,
        SideEffectSubmissionDecision::Observed(_)
    ));
    assert_eq!(signer.calls.load(Ordering::SeqCst), 1);
    assert_eq!(session.submitted.lock().expect("submitted").len(), 1);
}

#[tokio::test]
async fn external_nonce_conflict_stays_unknown_and_rebroadcasts_only_the_prepared_envelope() {
    let session = Arc::new(MockSession::new(LookupMode::Missing));
    let signer = Arc::new(FixedProvider::new());
    let adapter = make_adapter(Arc::clone(&session), Arc::clone(&signer));
    let prepared = prepare_and_commit(&adapter).await;
    let recovery_prepared = prepared.clone();

    let decision = adapter.submit_transaction(prepared).await.expect("submit");
    assert!(matches!(decision, SideEffectSubmissionDecision::Unknown(_)));
    session.set_pending_nonce(0x43);
    let recovery = adapter
        .recover_transaction(recovery_prepared)
        .await
        .expect("recover");

    assert!(matches!(
        recovery,
        SideEffectUnknownSubmissionDecision::StillUnknown
    ));
    assert_eq!(signer.calls.load(Ordering::SeqCst), 1);
    assert_eq!(session.pending_nonce_calls.load(Ordering::SeqCst), 1);
    let submissions = session.submitted.lock().expect("submitted");
    assert_eq!(submissions.len(), 2);
    assert_eq!(submissions[0], submissions[1]);
}

#[tokio::test]
async fn resumed_started_submission_looks_up_before_signing_or_broadcasting() {
    let session = Arc::new(MockSession::new(LookupMode::Missing));
    let preparation_signer = Arc::new(FixedProvider::new());
    let preparation_adapter = make_adapter(Arc::clone(&session), preparation_signer);
    let prepared = prepare_and_commit(&preparation_adapter).await;
    session.set_lookup_mode(LookupMode::Observed);
    let resumed_signer = Arc::new(FixedProvider::new());
    let resumed_adapter = make_adapter(Arc::clone(&session), Arc::clone(&resumed_signer));

    let decision = resumed_adapter
        .submit_transaction(prepared)
        .await
        .expect("resume started submission");

    assert!(matches!(
        decision,
        SideEffectSubmissionDecision::Observed(_)
    ));
    assert_eq!(resumed_signer.calls.load(Ordering::SeqCst), 0);
    assert!(session.submitted.lock().expect("submitted").is_empty());
}

#[tokio::test]
async fn discarded_preparation_settlements_release_signed_envelope_capacity() {
    let session = Arc::new(MockSession::new(LookupMode::Missing));
    let signer = Arc::new(FixedProvider::new());
    let adapter = make_adapter(Arc::clone(&session), Arc::clone(&signer));

    for _ in 0..SIGNED_ENVELOPE_CACHE_CAPACITY * 2 {
        let unsettled = adapter
            .prepare_transaction(&intent())
            .await
            .expect("unsettled preparation");
        drop(unsettled);
    }
    let result = adapter.prepare_transaction(&intent()).await;

    assert!(result.is_ok());
    assert_eq!(
        session.pending_nonce_calls.load(Ordering::SeqCst),
        SIGNED_ENVELOPE_CACHE_CAPACITY * 2 + 1
    );
    assert_eq!(
        signer.calls.load(Ordering::SeqCst),
        SIGNED_ENVELOPE_CACHE_CAPACITY * 2 + 1
    );
}

#[tokio::test]
async fn recovery_observes_expected_hash_before_any_rebroadcast() {
    let session = Arc::new(MockSession::new(LookupMode::Missing));
    let signer = Arc::new(FixedProvider::new());
    let adapter = make_adapter(Arc::clone(&session), Arc::clone(&signer));
    let prepared = prepare_and_commit(&adapter).await;
    let recovery_prepared = prepared.clone();
    assert!(matches!(
        adapter.submit_transaction(prepared).await.expect("submit"),
        SideEffectSubmissionDecision::Unknown(_)
    ));
    session.set_lookup_mode(LookupMode::Observed);

    let recovery = adapter
        .recover_transaction(recovery_prepared)
        .await
        .expect("recover");

    assert!(matches!(
        recovery,
        SideEffectUnknownSubmissionDecision::Observed(_)
    ));
    assert_eq!(signer.calls.load(Ordering::SeqCst), 1);
    assert_eq!(session.submitted.lock().expect("submitted").len(), 1);
}

#[tokio::test]
async fn same_process_retry_looks_up_uncertain_envelope_before_rebroadcasting() {
    let session = Arc::new(MockSession::new(LookupMode::Missing));
    let signer = Arc::new(FixedProvider::new());
    let adapter = make_adapter(Arc::clone(&session), Arc::clone(&signer));
    let prepared = prepare_and_commit(&adapter).await;
    let retry_prepared = prepared.clone();

    assert!(matches!(
        adapter.submit_transaction(prepared).await.expect("submit"),
        SideEffectSubmissionDecision::Unknown(_)
    ));
    session.set_lookup_mode(LookupMode::Observed);
    let retry = adapter
        .submit_transaction(retry_prepared)
        .await
        .expect("same-process retry");

    assert!(matches!(retry, SideEffectSubmissionDecision::Observed(_)));
    assert_eq!(signer.calls.load(Ordering::SeqCst), 1);
    assert_eq!(session.submitted.lock().expect("submitted").len(), 1);
}

#[tokio::test]
async fn signer_implementation_mismatch_fails_before_requesting_a_signature() {
    let session = Arc::new(MockSession::new(LookupMode::Missing));
    let signer = Arc::new(FixedProvider::new());
    let bind_session = Arc::clone(&session);
    let mismatched = Arc::new(MismatchedProvider(Arc::clone(&signer)));
    let signer_binder = mfm_signing::DeterministicSigningProviderBinder::new(
        "mfm.test.deterministic-signer",
        move |_signer_ref| {
            let mismatched = Arc::clone(&mismatched);
            Box::pin(async move { Ok(mismatched as Arc<dyn DeterministicSigningProvider>) })
        },
    )
    .expect("signer binder");
    let adapter = EvmTransactionAdapter::new(EvmTransactionRunnerCapabilities::new(
        Arc::new(MissingArtifacts),
        signer_binder,
        |_binding, _signer_ref| Box::pin(async { Ok(()) }),
        move |_binding| {
            let session = Arc::clone(&bind_session);
            Box::pin(async move { Ok(session as Arc<dyn EvmTransactionSession>) })
        },
    ));

    let error = match adapter.prepare_transaction(&intent()).await {
        Ok(_) => panic!("implementation mismatch must fail"),
        Err(error) => error,
    };

    assert!(matches!(
        error,
        mfm_runtime::RuntimeError::InvalidRunnerOutput(_)
    ));
    assert_eq!(signer.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn wrong_session_binding_fails_before_using_transaction_or_signing_authority() {
    let session = Arc::new(MockSession::with_evidence_chain(LookupMode::Missing, 2));
    let signer = Arc::new(FixedProvider::new());
    let adapter = make_adapter(Arc::clone(&session), Arc::clone(&signer));

    let error = match adapter.prepare_transaction(&intent()).await {
        Ok(_) => panic!("wrong session binding must fail"),
        Err(error) => error,
    };

    assert!(matches!(
        error,
        mfm_runtime::RuntimeError::InvalidRunnerOutput(_)
    ));
    assert_eq!(session.pending_nonce_calls.load(Ordering::SeqCst), 0);
    assert!(session.submitted.lock().expect("submitted").is_empty());
    assert_eq!(signer.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn replay_rejects_intent_or_idempotency_different_from_certified_authorship() {
    let authored = intent();
    let expected_key = side_effect_idempotency_key(&authored).expect("idempotency key");

    let intent_error =
        verify_authored_transaction_replay(&authored, &authored, &other_intent(), &expected_key)
            .expect_err("changed authored input must reject");
    assert_eq!(
        intent_error.kind,
        replay::ReplayErrorKind::SideEffectMismatch
    );

    let wrong_key = events::IdempotencyKeyRef::new("wrong-idempotency-key").expect("wrong key");
    let idempotency_error =
        verify_authored_transaction_replay(&authored, &authored, &authored, &wrong_key)
            .expect_err("changed idempotency key must reject");
    assert_eq!(
        idempotency_error.kind,
        replay::ReplayErrorKind::SideEffectMismatch
    );
}

#[tokio::test]
async fn replay_rejects_prepared_intent_different_from_retained_intent() {
    let session = Arc::new(MockSession::new(LookupMode::Missing));
    let signer = Arc::new(FixedProvider::new());
    let adapter = make_adapter(session, signer);
    let prepared = prepare_and_commit(&adapter).await;

    let error = verify_prepared_intent(&other_intent(), &prepared)
        .expect_err("prepared intent mismatch must reject");

    assert_eq!(error.kind, replay::ReplayErrorKind::SideEffectMismatch);
}

#[test]
fn replay_rejects_prepared_sender_lane_different_from_certified_lane() {
    let lane = intent().sender_lane();
    let expected = events::ResourceKeyEvidence {
        namespace: spec::ResourceNamespace::new(EVM_SENDER_LANE_NAMESPACE).expect("lane namespace"),
        key_schema_id: EvmSenderLane::schema_id().expect("lane schema"),
        key: events::ResourceKey::new(lane.resource_key()).expect("lane key"),
    };
    let mut wrong = expected.clone();
    wrong.key =
        events::ResourceKey::new("ethereum-mainnet:1:0x0000000000000000000000000000000000000000")
            .expect("wrong lane key");

    assert!(verify_prepared_sender_lane(Some(&expected), &expected).is_ok());
    for retained in [None, Some(&wrong)] {
        let error = verify_prepared_sender_lane(retained, &expected)
            .expect_err("missing or changed sender lane must reject");
        assert_eq!(error.kind, replay::ReplayErrorKind::SideEffectMismatch);
    }
}

#[tokio::test]
async fn provider_hash_or_transaction_field_mismatch_becomes_ambiguity() {
    let session = Arc::new(MockSession::new(LookupMode::Missing));
    session.return_wrong_hash.store(true, Ordering::SeqCst);
    let signer = Arc::new(FixedProvider::new());
    let adapter = make_adapter(Arc::clone(&session), signer);
    let prepared = prepare_and_commit(&adapter).await;
    assert!(matches!(
        adapter.submit_transaction(prepared).await.expect("submit"),
        SideEffectSubmissionDecision::Ambiguous { .. }
    ));

    let session = Arc::new(MockSession::new(LookupMode::Mismatched));
    let signer = Arc::new(FixedProvider::new());
    let adapter = make_adapter(session, signer);
    let prepared = prepare_and_commit(&adapter).await;
    assert!(matches!(
        adapter.submit_transaction(prepared).await.expect("submit"),
        SideEffectSubmissionDecision::Ambiguous { .. }
    ));
}
