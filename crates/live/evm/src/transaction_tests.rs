use std::collections::VecDeque;
use std::num::NonZeroU64;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use k256::ecdsa::signature::hazmat::RandomizedPrehashSigner;
use mfm_evm::{
    Eip1559TransactionCommand, EvmAuthorityEpoch, EvmChainInstance, EvmEndpoint,
    EvmTransactionBinding, EvmTransactionOutcome, EvmTransactionRoute, EvmTransactionSettlement,
    EvmU256,
};
use mfm_evm_transaction_authority::{
    AuthorityFuture, AuthorityState, EvmTransactionAuthority, PreparedRecord, Reservation,
    SettledRecord, MAX_EXACT_RAW_TRANSACTION_BYTES,
};
use mfm_ids::{ContentRef, DigestBytes, EffectId, StableId};
use mfm_keystore::{KeystoreOwner, KeystoreSigner, SecretSecp256k1Scalar};
use mfm_signing::{
    CompactRecoverableSignature, Secp256k1PublicKey, Secp256k1Signer, SigningDigest, SigningError,
    SigningFuture,
};
use mfm_values::canonicalize_mfm_value;
use tokio::sync::{Barrier, Notify};

use super::*;
use crate::evm_keccak256;

const GENESIS: &str = "0x1111111111111111111111111111111111111111111111111111111111111111";
const BLOCK: &str = "0x2222222222222222222222222222222222222222222222222222222222222222";

fn nonzero(value: u64) -> NonZeroU64 {
    NonZeroU64::new(value).expect("nonzero fixture")
}

fn command_value_ref(command: &Eip1559TransactionCommand) -> ContentRef {
    canonicalize_mfm_value(command)
        .map(|(_, value_ref)| value_ref)
        .expect("command value ref")
}

async fn execute(
    binding: &EvmTransactionBinding,
    effect_id: &EffectId,
    command: &Eip1559TransactionCommand,
    signer: Arc<dyn Secp256k1Signer>,
    authority: Arc<dyn EvmTransactionAuthority>,
    provider: Arc<dyn EvmTransactionProvider>,
) -> Result<EffectAdapterOutcome<EvmTransactionSettlement>, AdapterError> {
    let executor = CheckedExecutor::new(binding.clone(), signer, authority, provider)
        .map_err(|_| AdapterError::Internal)?;
    super::execute_transaction(&executor, effect_id, &command_value_ref(command), command).await
}

fn checked_executor(
    binding: &EvmTransactionBinding,
    signer: Arc<dyn Secp256k1Signer>,
    authority: Arc<dyn EvmTransactionAuthority>,
    provider: Arc<dyn EvmTransactionProvider>,
) -> mfm_runtime::Result<CheckedExecutor> {
    CheckedExecutor::new(binding.clone(), signer, authority, provider)
}

#[test]
fn public_keccak_and_ethereum_address_helpers_are_exact_and_bounded() {
    assert_eq!(
        evm_keccak256(&[]).expect("empty Keccak").to_string(),
        "0xc5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470"
    );
    assert!(evm_keccak256(&vec![0; MAX_EXACT_RAW_TRANSACTION_BYTES]).is_ok());
    assert_eq!(
        evm_keccak256(&vec![0; MAX_EXACT_RAW_TRANSACTION_BYTES + 1]),
        Err(crate::EvmCodecError::Invalid)
    );

    let public_bytes = alloy_primitives::hex::decode(
        "0479be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798483ada7726a3c4655da4fbfc0e1108a8fd17b448a68554199c47d08ffb10d4b8",
    )
    .expect("generator SEC1");
    let public_key =
        mfm_signing::Secp256k1PublicKey::new(public_bytes.try_into().expect("65 bytes"))
            .expect("generator key");
    assert_eq!(
        ethereum_address(&public_key).to_string(),
        "0x7e5f4552091a69125d5dfcb7b8c2659029395bdf"
    );
}

struct RejectingSigner {
    public_key: Secp256k1PublicKey,
    purpose: StableId,
}

struct WrongDigestSigner {
    inner: KeystoreSigner,
}

struct VaryingSigner {
    signing_key: k256::ecdsa::SigningKey,
    public_key: Secp256k1PublicKey,
    purpose: StableId,
    sequence: AtomicU64,
    barrier: Arc<Barrier>,
}

struct VaryingRng(u64);

impl k256::elliptic_curve::rand_core::RngCore for VaryingRng {
    fn next_u32(&mut self) -> u32 {
        self.next_u64() as u32
    }

    fn next_u64(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn fill_bytes(&mut self, destination: &mut [u8]) {
        for chunk in destination.chunks_mut(8) {
            let bytes = self.next_u64().to_le_bytes();
            chunk.copy_from_slice(&bytes[..chunk.len()]);
        }
    }

    fn try_fill_bytes(
        &mut self,
        destination: &mut [u8],
    ) -> Result<(), k256::elliptic_curve::rand_core::Error> {
        self.fill_bytes(destination);
        Ok(())
    }
}

impl k256::elliptic_curve::rand_core::CryptoRng for VaryingRng {}

impl VaryingSigner {
    fn new(barrier: Arc<Barrier>) -> Self {
        let signing_key = k256::ecdsa::SigningKey::from_bytes((&[7_u8; 32]).into())
            .expect("valid fixture scalar");
        let encoded = signing_key.verifying_key().to_encoded_point(false);
        let public_key =
            Secp256k1PublicKey::new(encoded.as_bytes().try_into().expect("uncompressed key"))
                .expect("checked public key");
        Self {
            signing_key,
            public_key,
            purpose: StableId::new(EVM_EIP1559_SIGNING_PURPOSE_ID).expect("purpose"),
            sequence: AtomicU64::new(1),
            barrier,
        }
    }
}

impl RejectingSigner {
    fn matching(signer: &dyn Secp256k1Signer) -> Self {
        Self {
            public_key: *signer.public_key(),
            purpose: signer.purpose().clone(),
        }
    }
}

impl Secp256k1Signer for RejectingSigner {
    fn public_key(&self) -> &Secp256k1PublicKey {
        &self.public_key
    }

    fn purpose(&self) -> &StableId {
        &self.purpose
    }

    fn sign(&self, _digest: SigningDigest) -> SigningFuture {
        Box::pin(async { Err(SigningError::Failed) })
    }
}

impl Secp256k1Signer for WrongDigestSigner {
    fn public_key(&self) -> &Secp256k1PublicKey {
        self.inner.public_key()
    }

    fn purpose(&self) -> &StableId {
        self.inner.purpose()
    }

    fn sign(&self, _digest: SigningDigest) -> SigningFuture {
        self.inner.sign(SigningDigest::from_bytes([0x44; 32]))
    }
}

impl Secp256k1Signer for VaryingSigner {
    fn public_key(&self) -> &Secp256k1PublicKey {
        &self.public_key
    }

    fn purpose(&self) -> &StableId {
        &self.purpose
    }

    fn sign(&self, digest: SigningDigest) -> SigningFuture {
        let signing_key = self.signing_key.clone();
        let public_key = self.public_key;
        let sequence = self.sequence.fetch_add(1, Ordering::SeqCst);
        let barrier = Arc::clone(&self.barrier);
        Box::pin(async move {
            barrier.wait().await;
            let mut rng = VaryingRng(sequence.wrapping_mul(0x9e37_79b9_7f4a_7c15));
            let signature: k256::ecdsa::Signature = signing_key
                .sign_prehash_with_rng(&mut rng, digest.as_bytes())
                .map_err(|_| SigningError::Failed)?;
            let signature = signature.normalize_s().unwrap_or(signature);
            let bytes: [u8; 64] = signature.to_bytes().into();
            for recovery_id in 0..=1 {
                let checked = CompactRecoverableSignature::new(bytes, recovery_id)?;
                if recover_public_key(digest, checked).ok() == Some(public_key) {
                    return Ok(checked);
                }
            }
            Err(SigningError::Failed)
        })
    }
}

struct MemoryAuthority {
    epoch: EvmAuthorityEpoch,
    state: Mutex<Option<AuthorityState>>,
    loads: AtomicUsize,
    settlement_barrier: Mutex<Option<Arc<Barrier>>>,
}

impl MemoryAuthority {
    fn new(epoch: EvmAuthorityEpoch) -> Self {
        Self {
            epoch,
            state: Mutex::new(None),
            loads: AtomicUsize::new(0),
            settlement_barrier: Mutex::new(None),
        }
    }

    fn state(&self) -> Option<AuthorityState> {
        self.state.lock().expect("authority lock").clone()
    }

    fn race_settlement_once(&self) {
        *self
            .settlement_barrier
            .lock()
            .expect("settlement barrier lock") = Some(Arc::new(Barrier::new(2)));
    }
}

impl EvmTransactionAuthority for MemoryAuthority {
    fn authority_epoch(&self) -> &EvmAuthorityEpoch {
        &self.epoch
    }

    fn load<'a>(&'a self, _effect_id: &'a EffectId) -> AuthorityFuture<'a, Option<AuthorityState>> {
        Box::pin(async move {
            self.loads.fetch_add(1, Ordering::SeqCst);
            Ok(self.state())
        })
    }

    fn reserve_or_compare<'a>(
        &'a self,
        effect_id: &'a EffectId,
        command_value_ref: &'a ContentRef,
        domain: &'a NonceDomain,
        observed_pending_nonce: u64,
    ) -> AuthorityFuture<'a, Reservation> {
        Box::pin(async move {
            let mut state = self.state.lock().expect("authority lock");
            if let Some(existing) = &*state {
                return Ok(state_reservation(existing).clone());
            }
            let reservation = Reservation::new(
                effect_id.clone(),
                command_value_ref.clone(),
                domain.clone(),
                observed_pending_nonce,
            );
            *state = Some(AuthorityState::Reserved(reservation.clone()));
            Ok(reservation)
        })
    }

    fn retain_prepared<'a>(&'a self, candidate: &'a PreparedRecord) -> AuthorityFuture<'a, ()> {
        Box::pin(async move {
            let mut state = self.state.lock().expect("authority lock");
            let retained = state.as_ref().ok_or(AuthorityError::Internal)?;
            if state_reservation(retained) != candidate.reservation() {
                return Err(AuthorityError::Internal);
            }
            match retained {
                AuthorityState::Reserved(_) => {
                    *state = Some(AuthorityState::Prepared(candidate.clone()));
                    Ok(())
                }
                AuthorityState::Prepared(prepared) if prepared == candidate => Ok(()),
                AuthorityState::Settled(settled) if settled.prepared() == candidate => Ok(()),
                AuthorityState::Prepared(_) | AuthorityState::Settled(_) => {
                    Err(AuthorityError::Unavailable)
                }
            }
        })
    }

    fn retain_settlement<'a>(&'a self, candidate: &'a SettledRecord) -> AuthorityFuture<'a, ()> {
        Box::pin(async move {
            let barrier = self
                .settlement_barrier
                .lock()
                .expect("settlement barrier lock")
                .clone();
            if let Some(barrier) = barrier {
                barrier.wait().await;
            }
            let mut state = self.state.lock().expect("authority lock");
            let retained = state.as_ref().ok_or(AuthorityError::Internal)?;
            let retained_prepared = match retained {
                AuthorityState::Prepared(prepared) => prepared,
                AuthorityState::Settled(settled) => settled.prepared(),
                AuthorityState::Reserved(_) => return Err(AuthorityError::Internal),
            };
            if retained_prepared != candidate.prepared() {
                return Err(AuthorityError::Internal);
            }
            match retained {
                AuthorityState::Prepared(_) => {
                    *state = Some(AuthorityState::Settled(candidate.clone()));
                    Ok(())
                }
                AuthorityState::Settled(settled) if settled == candidate => Ok(()),
                AuthorityState::Settled(_) => Err(AuthorityError::Unavailable),
                AuthorityState::Reserved(_) => Err(AuthorityError::Internal),
            }
        })
    }
}

fn state_reservation(state: &AuthorityState) -> &Reservation {
    match state {
        AuthorityState::Reserved(reservation) => reservation,
        AuthorityState::Prepared(prepared) => prepared.reservation(),
        AuthorityState::Settled(settled) => settled.prepared().reservation(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ProviderOperation {
    ChainInstance,
    PendingNonce(EvmAddress),
    Receipt(EvmHash),
    CanonicalBlock(EvmU256),
    SubmitRaw(Vec<u8>),
}

struct ScriptedProvider {
    chain: EvmChainInstance,
    pending: u64,
    receipts: Mutex<VecDeque<Result<Option<ProviderReceipt>, AdapterError>>>,
    canonical: EvmBlockAnchor,
    submissions: Mutex<VecDeque<Result<Option<EvmHash>, AdapterError>>>,
    operations: Mutex<Vec<ProviderOperation>>,
    block_receipt_once: AtomicBool,
    receipt_entered: Notify,
    release_receipt: Notify,
}

impl ScriptedProvider {
    fn new(chain_id: u64) -> Self {
        Self {
            chain: EvmChainInstance::new(
                nonzero(chain_id),
                EvmHash::new(GENESIS).expect("genesis"),
            ),
            pending: 7,
            receipts: Mutex::new(VecDeque::new()),
            canonical: EvmBlockAnchor::new(
                EvmU256::from_u64(9),
                EvmHash::new(BLOCK).expect("block"),
            ),
            submissions: Mutex::new(VecDeque::new()),
            operations: Mutex::new(Vec::new()),
            block_receipt_once: AtomicBool::new(false),
            receipt_entered: Notify::new(),
            release_receipt: Notify::new(),
        }
    }

    fn push_receipt(&self, receipt: Result<Option<ProviderReceipt>, AdapterError>) {
        self.receipts
            .lock()
            .expect("receipts lock")
            .push_back(receipt);
    }

    fn push_submission(&self, result: Result<Option<EvmHash>, AdapterError>) {
        self.submissions
            .lock()
            .expect("submissions lock")
            .push_back(result);
    }

    fn operations(&self) -> Vec<ProviderOperation> {
        self.operations.lock().expect("operations lock").clone()
    }
}

impl EvmTransactionProvider for ScriptedProvider {
    fn chain_instance(&self) -> EvmTransactionProviderFuture<'_, EvmChainInstance> {
        Box::pin(async move {
            self.operations
                .lock()
                .expect("operations lock")
                .push(ProviderOperation::ChainInstance);
            Ok(self.chain.clone())
        })
    }

    fn pending_nonce<'a>(
        &'a self,
        sender: &'a EvmAddress,
    ) -> EvmTransactionProviderFuture<'a, u64> {
        Box::pin(async move {
            self.operations
                .lock()
                .expect("operations lock")
                .push(ProviderOperation::PendingNonce(sender.clone()));
            Ok(self.pending)
        })
    }

    fn receipt<'a>(
        &'a self,
        transaction_hash: &'a EvmHash,
    ) -> EvmTransactionProviderFuture<'a, Option<ProviderReceipt>> {
        Box::pin(async move {
            self.operations
                .lock()
                .expect("operations lock")
                .push(ProviderOperation::Receipt(transaction_hash.clone()));
            if self.block_receipt_once.swap(false, Ordering::SeqCst) {
                self.receipt_entered.notify_one();
                self.release_receipt.notified().await;
            }
            self.receipts
                .lock()
                .expect("receipts lock")
                .pop_front()
                .unwrap_or(Ok(None))
        })
    }

    fn canonical_block<'a>(
        &'a self,
        block_number: &'a EvmU256,
    ) -> EvmTransactionProviderFuture<'a, EvmBlockAnchor> {
        Box::pin(async move {
            self.operations
                .lock()
                .expect("operations lock")
                .push(ProviderOperation::CanonicalBlock(block_number.clone()));
            Ok(self.canonical.clone())
        })
    }

    fn submit_raw<'a>(
        &'a self,
        raw_transaction: &'a ExactRawTransaction,
    ) -> EvmTransactionProviderFuture<'a, EvmHash> {
        Box::pin(async move {
            let raw = raw_transaction.as_bytes().to_vec();
            self.operations
                .lock()
                .expect("operations lock")
                .push(ProviderOperation::SubmitRaw(raw.clone()));
            match self
                .submissions
                .lock()
                .expect("submissions lock")
                .pop_front()
            {
                Some(Ok(Some(hash))) => Ok(hash),
                Some(Ok(None)) | None => evm_keccak256(&raw).map_err(|_| AdapterError::Internal),
                Some(Err(error)) => Err(error),
            }
        })
    }
}

async fn fixture() -> (
    KeystoreOwner,
    Arc<KeystoreSigner>,
    EvmTransactionBinding,
    Eip1559TransactionCommand,
    EffectId,
) {
    let owner = KeystoreOwner::start().expect("owner");
    let signer = owner
        .import_secp256k1(
            SecretSecp256k1Scalar::new([7; 32]).expect("fixture secret"),
            StableId::new(EVM_EIP1559_SIGNING_PURPOSE_ID).expect("signing purpose"),
        )
        .await
        .expect("signer");
    let sender = ethereum_address(signer.public_key());
    let signer = Arc::new(signer);
    let route = EvmTransactionRoute::new(
        EvmChainInstance::new(nonzero(1337), EvmHash::new(GENESIS).expect("genesis")),
        EvmEndpoint::new("transaction-test")
            .expect("endpoint")
            .endpoint_ref()
            .expect("endpoint ref"),
    );
    let binding = EvmTransactionBinding::new(route, EvmAuthorityEpoch::new([3; 32]), sender);
    let command = Eip1559TransactionCommand::create(
        binding.clone(),
        vec![0x60, 0x00],
        EvmU256::from_u64(0),
        nonzero(100_000),
        EvmU256::from_u64(2),
        EvmU256::from_u64(10),
    )
    .expect("command");
    (
        owner,
        signer,
        binding,
        command,
        EffectId::from_digest(DigestBytes::from_array([5; 32])),
    )
}

#[tokio::test]
async fn transaction_registration_uses_the_complete_binding_as_its_only_key() {
    let (_owner, signer, binding, _command, _effect_id) = fixture().await;
    let authority: Arc<dyn EvmTransactionAuthority> =
        Arc::new(MemoryAuthority::new(binding.authority_epoch().clone()));
    let provider: Arc<dyn EvmTransactionProvider> = Arc::new(ScriptedProvider::new(1337));
    let signer: Arc<dyn Secp256k1Signer> = signer;
    let mut builder = RuntimeAssemblyBuilder::new().expect("builder");
    register_evm_transaction_effect(
        &mut builder,
        binding.clone(),
        Arc::clone(&signer),
        Arc::clone(&authority),
        Arc::clone(&provider),
    )
    .expect("transaction callback");
    assert_eq!(
        register_evm_transaction_effect(&mut builder, binding, signer, authority, provider),
        Err(mfm_runtime::RuntimeError::IncompatibleAssembly)
    );
    builder.finish();
}

#[tokio::test]
async fn absent_prepare_submit_resume_settle_and_fast_path_are_phase_exact() {
    let (_owner, signer, binding, command, effect_id) = fixture().await;
    let authority = Arc::new(MemoryAuthority::new(binding.authority_epoch().clone()));
    let provider = Arc::new(ScriptedProvider::new(1337));

    assert_eq!(
        execute(
            &binding,
            &effect_id,
            &command,
            signer.clone(),
            authority.clone(),
            provider.clone(),
        )
        .await,
        Ok(EffectAdapterOutcome::Pending)
    );
    let AuthorityState::Prepared(prepared) = authority.state().expect("prepared") else {
        panic!("first call must retain exact prepared bytes")
    };
    assert_eq!(
        alloy_primitives::hex::encode(prepared.raw_transaction().as_bytes()),
        "02f85382053907020a830186a08080826000c001a07ffd3c6f6e2217de62458b59faca6e9a3a829c7bcf9ebaa04e0414c1eb0d0419a06f5a761a7bb9c0dab816d83eb5472e2dfd61c8da2eca46924c500c54efe5d58b"
    );
    assert_eq!(
        prepared.transaction_hash().to_string(),
        "0x1c491e5220c082f1be50e5e78738147d1416633942ac5e689d65798127a5e71b"
    );
    assert_eq!(prepared.reservation().nonce(), 7);
    let operations = provider.operations();
    assert_eq!(
        operations
            .iter()
            .filter(|operation| matches!(operation, ProviderOperation::ChainInstance))
            .count(),
        1
    );
    let receipt_position = operations
        .iter()
        .position(|operation| matches!(operation, ProviderOperation::Receipt(_)))
        .expect("receipt observation");
    let submissions = operations
        .iter()
        .enumerate()
        .filter_map(|(position, operation)| match operation {
            ProviderOperation::SubmitRaw(raw) => Some((position, raw)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(submissions.len(), 1);
    assert!(receipt_position < submissions[0].0);
    assert_eq!(submissions[0].1, prepared.raw_transaction().as_bytes());
    assert_eq!(signer.purpose().as_str(), EVM_EIP1559_SIGNING_PURPOSE_ID);

    let created = create_address(binding.sender(), 7);
    provider.push_receipt(Ok(Some(ProviderReceipt::new(
        prepared.transaction_hash().clone(),
        binding.sender().clone(),
        ProviderReceiptResult::SuccessCreate {
            contract_address: created.clone(),
        },
        provider.canonical.clone(),
    ))));
    let rejecting_signer = Arc::new(RejectingSigner::matching(signer.as_ref()));
    let EffectAdapterOutcome::Settled(evidence) = execute(
        &binding,
        &effect_id,
        &command,
        rejecting_signer.clone(),
        authority.clone(),
        provider.clone(),
    )
    .await
    .expect("settled receipt") else {
        panic!("receipt must settle the Effect")
    };
    assert_eq!(evidence.nonce(), 7);
    assert!(matches!(
        evidence.outcome(),
        EvmTransactionOutcome::Created { created_address } if created_address == &created
    ));
    let operations = provider.operations();
    let canonical_position = operations
        .iter()
        .rposition(|operation| matches!(operation, ProviderOperation::CanonicalBlock(_)))
        .expect("canonical observation after receipt");
    assert!(operations[..canonical_position]
        .iter()
        .any(|operation| matches!(operation, ProviderOperation::Receipt(_))));
    assert_eq!(
        operations
            .iter()
            .filter(|operation| matches!(operation, ProviderOperation::PendingNonce(_)))
            .count(),
        1
    );
    assert_eq!(
        operations
            .iter()
            .filter(|operation| matches!(operation, ProviderOperation::ChainInstance))
            .count(),
        2
    );
    assert!(matches!(
        authority.state(),
        Some(AuthorityState::Settled(_))
    ));

    let provider_operations = provider.operations();
    assert_eq!(
        execute(
            &binding,
            &effect_id,
            &command,
            rejecting_signer,
            authority.clone(),
            provider.clone(),
        )
        .await,
        Ok(EffectAdapterOutcome::Settled(evidence))
    );
    assert_eq!(provider.operations(), provider_operations);
}

#[tokio::test]
async fn cancellation_after_prepare_leaves_one_resumable_exact_transaction() {
    let (_owner, signer, binding, command, effect_id) = fixture().await;
    let authority = Arc::new(MemoryAuthority::new(binding.authority_epoch().clone()));
    let provider = Arc::new(ScriptedProvider::new(1337));
    provider.block_receipt_once.store(true, Ordering::SeqCst);
    {
        let entered = provider.receipt_entered.notified();
        let invocation = execute(
            &binding,
            &effect_id,
            &command,
            signer.clone(),
            authority.clone(),
            provider.clone(),
        );
        tokio::pin!(entered);
        tokio::pin!(invocation);
        tokio::select! {
            () = &mut entered => {}
            result = &mut invocation => panic!("receipt should still be blocked: {result:?}"),
        }
    }
    assert!(matches!(
        authority.state(),
        Some(AuthorityState::Prepared(_))
    ));

    let rejecting_signer = Arc::new(RejectingSigner::matching(signer.as_ref()));
    assert_eq!(
        execute(
            &binding,
            &effect_id,
            &command,
            rejecting_signer,
            authority.clone(),
            provider.clone(),
        )
        .await,
        Ok(EffectAdapterOutcome::Pending)
    );
    assert_eq!(
        provider
            .operations()
            .iter()
            .filter(|operation| matches!(operation, ProviderOperation::PendingNonce(_)))
            .count(),
        1
    );
    let submitted = provider
        .operations()
        .into_iter()
        .filter_map(|operation| match operation {
            ProviderOperation::SubmitRaw(raw) => Some(raw),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(submitted.len(), 1);
    let AuthorityState::Prepared(prepared) = authority.state().expect("prepared") else {
        panic!("cancellation must retain preparation")
    };
    assert_eq!(submitted[0], prepared.raw_transaction().as_bytes());
}

#[tokio::test]
async fn pending_retries_submit_identical_retained_bytes_without_reserving_again() {
    let (_owner, signer, binding, command, effect_id) = fixture().await;
    let authority = Arc::new(MemoryAuthority::new(binding.authority_epoch().clone()));
    let provider = Arc::new(ScriptedProvider::new(1337));

    assert_eq!(
        execute(
            &binding,
            &effect_id,
            &command,
            signer.clone(),
            authority.clone(),
            provider.clone(),
        )
        .await,
        Ok(EffectAdapterOutcome::Pending)
    );
    assert_eq!(
        provider
            .operations()
            .iter()
            .filter(|operation| matches!(operation, ProviderOperation::SubmitRaw(_)))
            .count(),
        1
    );

    let rejecting_signer = Arc::new(RejectingSigner::matching(signer.as_ref()));
    assert_eq!(
        execute(
            &binding,
            &effect_id,
            &command,
            rejecting_signer,
            authority.clone(),
            provider.clone(),
        )
        .await,
        Ok(EffectAdapterOutcome::Pending)
    );
    let operations = provider.operations();
    let submitted = operations
        .iter()
        .filter_map(|operation| match operation {
            ProviderOperation::SubmitRaw(raw) => Some(raw),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(submitted.len(), 2);
    assert_eq!(submitted[0], submitted[1]);
    assert_eq!(
        operations
            .iter()
            .filter(|operation| matches!(operation, ProviderOperation::PendingNonce(_)))
            .count(),
        1
    );
}

#[tokio::test]
async fn concurrent_varying_signatures_reload_and_submit_only_the_first_prepared_winner() {
    let (_owner, fixture_signer, binding, command, effect_id) = fixture().await;
    let signer = Arc::new(VaryingSigner::new(Arc::new(Barrier::new(2))));
    assert!(signer.public_key() == fixture_signer.public_key());
    let authority = Arc::new(MemoryAuthority::new(binding.authority_epoch().clone()));
    let provider = Arc::new(ScriptedProvider::new(1337));

    let attempts = (0..2)
        .map(|_| {
            let binding = binding.clone();
            let effect_id = effect_id.clone();
            let command = command.clone();
            let signer = signer.clone();
            let authority = authority.clone();
            let provider = provider.clone();
            tokio::spawn(async move {
                execute(&binding, &effect_id, &command, signer, authority, provider).await
            })
        })
        .collect::<Vec<_>>();
    let mut outcomes = Vec::new();
    for attempt in attempts {
        outcomes.push(attempt.await.expect("prepared race join"));
    }
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| matches!(outcome, Ok(EffectAdapterOutcome::Pending)))
            .count(),
        1
    );
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| matches!(outcome, Err(AdapterError::Unavailable)))
            .count(),
        1
    );
    let AuthorityState::Prepared(winner) = authority.state().expect("prepared winner") else {
        panic!("one prepared candidate must win")
    };
    let submitted = provider
        .operations()
        .into_iter()
        .filter_map(|operation| match operation {
            ProviderOperation::SubmitRaw(raw) => Some(raw),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(submitted.len(), 1);
    assert_eq!(submitted[0], winner.raw_transaction().as_bytes());

    let rejecting_signer = Arc::new(RejectingSigner::matching(fixture_signer.as_ref()));
    assert_eq!(
        execute(
            &binding,
            &effect_id,
            &command,
            rejecting_signer,
            authority,
            provider.clone(),
        )
        .await,
        Ok(EffectAdapterOutcome::Pending)
    );
    assert!(provider.operations().iter().all(|operation| {
        !matches!(operation, ProviderOperation::SubmitRaw(raw) if raw != winner.raw_transaction().as_bytes())
    }));
}

#[tokio::test]
async fn concurrent_qualified_settlements_reload_the_first_winner() {
    let (_owner, signer, binding, command, effect_id) = fixture().await;
    let authority = Arc::new(MemoryAuthority::new(binding.authority_epoch().clone()));
    let initial_provider = Arc::new(ScriptedProvider::new(1337));
    assert_eq!(
        execute(
            &binding,
            &effect_id,
            &command,
            signer.clone(),
            authority.clone(),
            initial_provider,
        )
        .await,
        Ok(EffectAdapterOutcome::Pending)
    );
    let AuthorityState::Prepared(prepared) = authority.state().expect("prepared") else {
        panic!("fixture must retain prepared bytes")
    };
    authority.race_settlement_once();

    let created_provider = Arc::new(ScriptedProvider::new(1337));
    created_provider.push_receipt(Ok(Some(ProviderReceipt::new(
        prepared.transaction_hash().clone(),
        binding.sender().clone(),
        ProviderReceiptResult::SuccessCreate {
            contract_address: create_address(binding.sender(), prepared.reservation().nonce()),
        },
        created_provider.canonical.clone(),
    ))));
    let reverted_provider = Arc::new(ScriptedProvider::new(1337));
    reverted_provider.push_receipt(Ok(Some(ProviderReceipt::new(
        prepared.transaction_hash().clone(),
        binding.sender().clone(),
        ProviderReceiptResult::RevertedCreate,
        reverted_provider.canonical.clone(),
    ))));
    let rejecting_signer = Arc::new(RejectingSigner::matching(signer.as_ref()));

    let attempts = [created_provider, reverted_provider]
        .into_iter()
        .map(|provider| {
            let binding = binding.clone();
            let effect_id = effect_id.clone();
            let command = command.clone();
            let signer = rejecting_signer.clone();
            let authority = authority.clone();
            tokio::spawn(async move {
                execute(&binding, &effect_id, &command, signer, authority, provider).await
            })
        })
        .collect::<Vec<_>>();
    let mut outcomes = Vec::new();
    for attempt in attempts {
        outcomes.push(attempt.await.expect("settlement race join"));
    }
    assert_eq!(outcomes.iter().filter(|outcome| outcome.is_ok()).count(), 1);
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| matches!(outcome, Err(AdapterError::Unavailable)))
            .count(),
        1
    );
    let AuthorityState::Settled(winner) = authority.state().expect("settlement winner") else {
        panic!("one settlement candidate must win")
    };
    let recovery_provider = Arc::new(ScriptedProvider::new(1337));
    assert_eq!(
        execute(
            &binding,
            &effect_id,
            &command,
            rejecting_signer,
            authority,
            recovery_provider.clone(),
        )
        .await,
        Ok(EffectAdapterOutcome::Settled(winner.evidence().clone()))
    );
    assert!(recovery_provider.operations().is_empty());
}

#[tokio::test]
async fn preloaded_reservation_skips_pending_nonce_and_resumes_preparation() {
    let (_owner, signer, binding, command, effect_id) = fixture().await;
    let authority = Arc::new(MemoryAuthority::new(binding.authority_epoch().clone()));
    let provider = Arc::new(ScriptedProvider::new(1337));
    let local = checked_executor(
        &binding,
        signer.clone(),
        authority.clone(),
        provider.clone(),
    )
    .expect("local");
    *authority.state.lock().expect("authority lock") =
        Some(AuthorityState::Reserved(Reservation::new(
            effect_id.clone(),
            command_value_ref(&command),
            local.domain.clone(),
            7,
        )));

    assert_eq!(
        execute(
            &binding,
            &effect_id,
            &command,
            signer.clone(),
            authority.clone(),
            provider.clone(),
        )
        .await,
        Ok(EffectAdapterOutcome::Pending)
    );
    assert!(provider
        .operations()
        .iter()
        .all(|operation| !matches!(operation, ProviderOperation::PendingNonce(_))));
    assert!(matches!(
        authority.state(),
        Some(AuthorityState::Prepared(_))
    ));
}

#[tokio::test]
async fn submission_failure_or_mismatched_hash_remains_unavailable() {
    for submission in [
        Err(AdapterError::Unavailable),
        Ok(Some(
            EvmHash::new(format!("0x{}", "99".repeat(32))).expect("mismatched hash"),
        )),
    ] {
        let (_owner, signer, binding, command, effect_id) = fixture().await;
        let authority = Arc::new(MemoryAuthority::new(binding.authority_epoch().clone()));
        let provider = Arc::new(ScriptedProvider::new(1337));
        provider.push_submission(submission);

        assert_eq!(
            execute(
                &binding,
                &effect_id,
                &command,
                signer.clone(),
                authority.clone(),
                provider.clone(),
            )
            .await,
            Err(AdapterError::Unavailable)
        );
        assert!(matches!(
            authority.state(),
            Some(AuthorityState::Prepared(_))
        ));
        assert_eq!(
            provider
                .operations()
                .iter()
                .filter(|operation| matches!(operation, ProviderOperation::SubmitRaw(_)))
                .count(),
            1
        );
    }
}

#[tokio::test]
async fn receipt_shape_validation_covers_create_call_revert_and_mismatch_matrix() {
    let (_owner, signer, binding, create_command, effect_id) = fixture().await;
    let authority = Arc::new(MemoryAuthority::new(binding.authority_epoch().clone()));
    let provider = Arc::new(ScriptedProvider::new(1337));
    assert_eq!(
        execute(
            &binding,
            &effect_id,
            &create_command,
            signer.clone(),
            authority.clone(),
            provider.clone(),
        )
        .await,
        Ok(EffectAdapterOutcome::Pending)
    );
    let AuthorityState::Prepared(prepared) = authority.state().expect("prepared") else {
        panic!("fixture must prepare")
    };
    let local = checked_executor(
        &binding,
        signer.clone(),
        authority.clone(),
        provider.clone(),
    )
    .expect("local binding");
    let target =
        EvmAddress::new("0x4444444444444444444444444444444444444444").expect("call target");
    let call_command = Eip1559TransactionCommand::call(
        binding.clone(),
        target.clone(),
        vec![1, 2],
        EvmU256::from_u64(0),
        nonzero(100_000),
        EvmU256::from_u64(2),
        EvmU256::from_u64(10),
    )
    .expect("call command");
    let created = create_address(binding.sender(), prepared.reservation().nonce());

    enum Expected {
        Created,
        Called,
        Reverted,
        Error,
    }

    let wrong_hash = EvmHash::new(format!("0x{}", "88".repeat(32))).expect("wrong hash");
    let wrong_sender =
        EvmAddress::new("0x5555555555555555555555555555555555555555").expect("wrong sender");
    let wrong_target =
        EvmAddress::new("0x6666666666666666666666666666666666666666").expect("wrong target");
    let wrong_created = EvmAddress::new("0x7777777777777777777777777777777777777777")
        .expect("wrong created address");
    let cases = [
        (
            &create_command,
            prepared.transaction_hash().clone(),
            binding.sender().clone(),
            ProviderReceiptResult::SuccessCreate {
                contract_address: created,
            },
            Expected::Created,
        ),
        (
            &create_command,
            prepared.transaction_hash().clone(),
            binding.sender().clone(),
            ProviderReceiptResult::RevertedCreate,
            Expected::Reverted,
        ),
        (
            &call_command,
            prepared.transaction_hash().clone(),
            binding.sender().clone(),
            ProviderReceiptResult::SuccessCall {
                target: target.clone(),
            },
            Expected::Called,
        ),
        (
            &call_command,
            prepared.transaction_hash().clone(),
            binding.sender().clone(),
            ProviderReceiptResult::RevertedCall {
                target: target.clone(),
            },
            Expected::Reverted,
        ),
        (
            &create_command,
            prepared.transaction_hash().clone(),
            binding.sender().clone(),
            ProviderReceiptResult::SuccessCreate {
                contract_address: wrong_created,
            },
            Expected::Error,
        ),
        (
            &call_command,
            prepared.transaction_hash().clone(),
            binding.sender().clone(),
            ProviderReceiptResult::SuccessCall {
                target: wrong_target,
            },
            Expected::Error,
        ),
        (
            &create_command,
            wrong_hash,
            binding.sender().clone(),
            ProviderReceiptResult::RevertedCreate,
            Expected::Error,
        ),
        (
            &create_command,
            prepared.transaction_hash().clone(),
            wrong_sender,
            ProviderReceiptResult::RevertedCreate,
            Expected::Error,
        ),
    ];

    for (command, hash, sender, result, expected) in cases {
        let receipt = ProviderReceipt::new(hash, sender, result, provider.canonical.clone());
        let actual = validate_receipt(&effect_id, &receipt, &prepared, command, &local);
        match expected {
            Expected::Created => assert!(actual.is_ok_and(|settlement| matches!(
                settlement.outcome(),
                EvmTransactionOutcome::Created { .. }
            ))),
            Expected::Called => assert!(actual.is_ok_and(|settlement| matches!(
                settlement.outcome(),
                EvmTransactionOutcome::Called
            ))),
            Expected::Reverted => assert!(actual.is_ok_and(|settlement| matches!(
                settlement.outcome(),
                EvmTransactionOutcome::Reverted
            ))),
            Expected::Error => assert_eq!(actual, Err(AdapterError::Internal)),
        }
    }
}

#[tokio::test]
async fn noncanonical_receipt_anchor_stops_before_settlement() {
    let (_owner, signer, binding, command, effect_id) = fixture().await;
    let authority = Arc::new(MemoryAuthority::new(binding.authority_epoch().clone()));
    let provider = Arc::new(ScriptedProvider::new(1337));
    assert_eq!(
        execute(
            &binding,
            &effect_id,
            &command,
            signer.clone(),
            authority.clone(),
            provider.clone(),
        )
        .await,
        Ok(EffectAdapterOutcome::Pending)
    );
    let AuthorityState::Prepared(prepared) = authority.state().expect("prepared") else {
        panic!("fixture must prepare")
    };
    let other_anchor = EvmBlockAnchor::new(
        EvmU256::from_u64(10),
        EvmHash::new(format!("0x{}", "33".repeat(32))).expect("other block"),
    );
    provider.push_receipt(Ok(Some(ProviderReceipt::new(
        prepared.transaction_hash().clone(),
        binding.sender().clone(),
        ProviderReceiptResult::RevertedCreate,
        other_anchor.clone(),
    ))));

    assert_eq!(
        execute(
            &binding,
            &effect_id,
            &command,
            signer.clone(),
            authority.clone(),
            provider.clone(),
        )
        .await,
        Err(AdapterError::Unavailable)
    );
    assert!(matches!(
        authority.state(),
        Some(AuthorityState::Prepared(_))
    ));
    let operations = provider.operations();
    let receipt_position = operations
        .iter()
        .rposition(|operation| matches!(operation, ProviderOperation::Receipt(_)))
        .expect("present receipt observation");
    let canonical_position = operations
        .iter()
        .rposition(|operation| {
            operation == &ProviderOperation::CanonicalBlock(other_anchor.number().clone())
        })
        .expect("canonical observation");
    assert!(receipt_position < canonical_position);
}

#[tokio::test]
async fn wrong_returned_signature_is_rejected_before_preparation_or_submission() {
    let (_owner, signer, binding, command, effect_id) = fixture().await;
    let signer = Arc::new(WrongDigestSigner {
        inner: signer.as_ref().clone(),
    });
    let authority = Arc::new(MemoryAuthority::new(binding.authority_epoch().clone()));
    let provider = Arc::new(ScriptedProvider::new(1337));

    assert_eq!(
        execute(
            &binding,
            &effect_id,
            &command,
            signer,
            authority.clone(),
            provider.clone(),
        )
        .await,
        Err(AdapterError::Internal)
    );
    assert!(matches!(
        authority.state(),
        Some(AuthorityState::Reserved(_))
    ));
    assert!(provider.operations().iter().all(|operation| !matches!(
        operation,
        ProviderOperation::Receipt(_) | ProviderOperation::SubmitRaw(_)
    )));
}

#[tokio::test]
async fn local_binding_and_corrupt_prepared_bytes_are_internal_before_provider_entry() {
    let (owner, signer, binding, command, effect_id) = fixture().await;
    let authority = Arc::new(MemoryAuthority::new(binding.authority_epoch().clone()));
    let provider = Arc::new(ScriptedProvider::new(1337));
    let other_endpoint = EvmEndpoint::new("transaction-test-other")
        .expect("other endpoint")
        .endpoint_ref()
        .expect("other endpoint ref");
    let other_genesis = EvmHash::new(format!("0x{}", "aa".repeat(32))).expect("other genesis");
    let wrong_bindings = [
        EvmTransactionBinding::new(
            binding.route().clone(),
            EvmAuthorityEpoch::new([4; 32]),
            binding.sender().clone(),
        ),
        EvmTransactionBinding::new(
            EvmTransactionRoute::new(binding.route().chain_instance().clone(), other_endpoint),
            binding.authority_epoch().clone(),
            binding.sender().clone(),
        ),
        EvmTransactionBinding::new(
            EvmTransactionRoute::new(
                EvmChainInstance::new(nonzero(1338), EvmHash::new(GENESIS).expect("genesis")),
                binding.route().endpoint_ref().clone(),
            ),
            binding.authority_epoch().clone(),
            binding.sender().clone(),
        ),
        EvmTransactionBinding::new(
            EvmTransactionRoute::new(
                EvmChainInstance::new(nonzero(1337), other_genesis),
                binding.route().endpoint_ref().clone(),
            ),
            binding.authority_epoch().clone(),
            binding.sender().clone(),
        ),
        EvmTransactionBinding::new(
            binding.route().clone(),
            binding.authority_epoch().clone(),
            EvmAddress::new("0x9999999999999999999999999999999999999999").expect("other sender"),
        ),
    ];
    for wrong_binding in wrong_bindings {
        assert_eq!(
            execute(
                &wrong_binding,
                &effect_id,
                &command,
                signer.clone(),
                authority.clone(),
                provider.clone(),
            )
            .await,
            Err(AdapterError::Internal)
        );
    }
    assert_eq!(authority.loads.load(Ordering::SeqCst), 0);
    assert!(provider.operations().is_empty());

    let wrong_epoch_authority = Arc::new(MemoryAuthority::new(EvmAuthorityEpoch::new([4; 32])));
    assert_eq!(
        execute(
            &binding,
            &effect_id,
            &command,
            signer.clone(),
            wrong_epoch_authority.clone(),
            provider.clone(),
        )
        .await,
        Err(AdapterError::Internal)
    );
    assert_eq!(wrong_epoch_authority.loads.load(Ordering::SeqCst), 0);
    assert!(provider.operations().is_empty());

    let wrong_purpose = Arc::new(
        owner
            .import_secp256k1(
                SecretSecp256k1Scalar::new([7; 32]).expect("fixture secret"),
                StableId::new("mfm.test.evm/wrong-purpose@1").expect("wrong purpose"),
            )
            .await
            .expect("same key under wrong purpose"),
    );
    assert_eq!(
        execute(
            &binding,
            &effect_id,
            &command,
            wrong_purpose,
            authority.clone(),
            provider.clone(),
        )
        .await,
        Err(AdapterError::Internal)
    );
    assert_eq!(authority.loads.load(Ordering::SeqCst), 0);
    assert!(provider.operations().is_empty());

    let wrong_key = Arc::new(
        owner
            .import_secp256k1(
                SecretSecp256k1Scalar::new([8; 32]).expect("different fixture secret"),
                StableId::new(EVM_EIP1559_SIGNING_PURPOSE_ID).expect("signing purpose"),
            )
            .await
            .expect("different key under correct purpose"),
    );
    assert_eq!(
        execute(
            &binding,
            &effect_id,
            &command,
            wrong_key,
            authority.clone(),
            provider.clone(),
        )
        .await,
        Err(AdapterError::Internal)
    );
    assert_eq!(authority.loads.load(Ordering::SeqCst), 0);
    assert!(provider.operations().is_empty());

    let local = checked_executor(
        &binding,
        signer.clone(),
        authority.clone(),
        provider.clone(),
    )
    .expect("local");
    let wrong_domain = NonceDomain::new(
        binding.authority_epoch().clone(),
        binding.route().chain_instance().clone(),
        EvmAddress::new("0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").expect("wrong sender"),
    );
    *authority.state.lock().expect("authority lock") =
        Some(AuthorityState::Reserved(Reservation::new(
            effect_id.clone(),
            command_value_ref(&command),
            wrong_domain,
            7,
        )));
    assert_eq!(
        execute(
            &binding,
            &effect_id,
            &command,
            signer.clone(),
            authority.clone(),
            provider.clone(),
        )
        .await,
        Err(AdapterError::Internal)
    );
    assert!(provider.operations().is_empty());

    let reservation = Reservation::new(
        effect_id.clone(),
        command_value_ref(&command),
        local.domain.clone(),
        7,
    );
    let raw = ExactRawTransaction::new(vec![0x02, 0xc0]).expect("bounded corrupt raw");
    let prepared = PreparedRecord::new(
        reservation,
        evm_keccak256(raw.as_bytes()).expect("hash"),
        raw,
    );
    *authority.state.lock().expect("authority lock") = Some(AuthorityState::Prepared(prepared));
    assert_eq!(
        execute(
            &binding,
            &effect_id,
            &command,
            signer,
            authority.clone(),
            provider.clone(),
        )
        .await,
        Err(AdapterError::Internal)
    );
    assert!(provider.operations().is_empty());
}
