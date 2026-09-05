use std::collections::VecDeque;
use std::num::NonZeroU64;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use k256::ecdsa::signature::hazmat::RandomizedPrehashSigner;
use mfm_evm::custody::{
    AuthorityFuture, EvmTransactionAuthority, LoadedTransaction, PreparedRecord, Reservation,
    MAX_EXACT_RAW_TRANSACTION_BYTES,
};
use mfm_evm::{
    Eip1559TransactionCommand, EvmAuthorityEpoch, EvmChainInstance, EvmEndpoint,
    EvmTransactionBinding, EvmTransactionRoute, EvmU256,
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
                if recover_public_key(digest, &checked).ok().as_ref() == Some(&public_key) {
                    return Ok(checked);
                }
            }
            Err(SigningError::Failed)
        })
    }
}

struct MemoryAuthority {
    epoch: EvmAuthorityEpoch,
    state: Mutex<Option<LoadedTransaction>>,
    fault: AtomicUsize,
}
impl MemoryAuthority {
    fn new(epoch: EvmAuthorityEpoch) -> Self {
        Self {
            epoch,
            state: Mutex::new(None),
            fault: AtomicUsize::new(0),
        }
    }
    fn state(&self) -> Option<LoadedTransaction> {
        self.state.lock().unwrap().clone()
    }
    fn acknowledge(&self, stage: usize) -> Result<(), AuthorityError> {
        if self
            .fault
            .compare_exchange(stage, 0, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            Err(AuthorityError::Unavailable)
        } else {
            Ok(())
        }
    }
}
impl EvmTransactionAuthority for MemoryAuthority {
    fn authority_epoch(&self) -> &EvmAuthorityEpoch {
        &self.epoch
    }
    fn load<'a>(&'a self, id: &'a EffectId) -> AuthorityFuture<'a, Option<LoadedTransaction>> {
        Box::pin(async move { Ok(self.state().filter(|s| s.reservation.effect_id() == id)) })
    }
    fn reserve_or_compare<'a>(
        &'a self,
        id: &'a EffectId,
        reference: &'a ContentRef,
        domain: &'a NonceDomain,
        observed: u64,
    ) -> AuthorityFuture<'a, Reservation> {
        Box::pin(async move {
            let mut state = self.state.lock().unwrap();
            let reservation = if let Some(state) = state.as_ref() {
                if state.reservation.effect_id() != id
                    || state.reservation.command_value_ref() != reference
                    || state.reservation.domain() != domain
                {
                    return Err(AuthorityError::Internal);
                }
                state.reservation.clone()
            } else {
                let reservation =
                    Reservation::new(id.clone(), reference.clone(), domain.clone(), observed)
                        .map_err(|_| AuthorityError::Unavailable)?;
                *state = Some(LoadedTransaction {
                    reservation: reservation.clone(),
                    prepared: None,
                });
                reservation
            };
            self.acknowledge(1)?;
            Ok(reservation)
        })
    }
    fn retain_prepared<'a>(
        &'a self,
        reservation: &'a Reservation,
        candidate: &'a PreparedRecord,
    ) -> AuthorityFuture<'a, PreparedRecord> {
        Box::pin(async move {
            let mut state = self.state.lock().unwrap();
            let state = state.as_mut().ok_or(AuthorityError::Internal)?;
            if &state.reservation != reservation {
                return Err(AuthorityError::Internal);
            }
            let winner = state
                .prepared
                .get_or_insert_with(|| candidate.clone())
                .clone();
            self.acknowledge(2)?;
            Ok(winner)
        })
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
    register_evm_transaction_adapters(
        &mut builder,
        binding.clone(),
        Arc::clone(&signer),
        Arc::clone(&authority),
        Arc::clone(&provider),
    )
    .expect("transaction callback");
    assert_eq!(
        register_evm_transaction_adapters(&mut builder, binding, signer, authority, provider),
        Err(mfm_runtime::RuntimeError::IncompatibleAssembly)
    );
    builder.finish();
}

use mfm_evm::{
    EvmTransactionCompletion, EvmTransactionContext, EvmTransactionReversion,
    ExecuteEvmTransaction, PrepareEvmTransaction, ProjectEvmTransactionOutcome, ReserveEvmNonce,
};
use mfm_ids::{EntryPointId, RunId};
use mfm_program::{expand_program, Operation, OperationExpansion};
use mfm_runtime::{RunViewState, Runtime};
use mfm_store::MemoryStore;

struct TransactionProgram(EvmTransactionBinding);
impl Operation for TransactionProgram {
    type Input = EvmTransactionContext<EvmU256>;
    type Output = EvmTransactionCompletion<EvmU256>;
    type Failure = EvmTransactionReversion<EvmU256>;
    fn expand(
        &self,
        body: &mut OperationExpansion<Self::Input, Self::Output, Self::Failure>,
    ) -> mfm_program::Result<()> {
        body.effect::<ExecuteEvmTransaction<EvmU256>, EvmTransactionEffect>(&self.0)
    }
}
fn runtime(
    binding: &EvmTransactionBinding,
    signer: Arc<dyn Secp256k1Signer>,
    authority: Arc<MemoryAuthority>,
    provider: Arc<ScriptedProvider>,
    store: Arc<dyn mfm_store::Store>,
) -> Runtime {
    let mut builder = RuntimeAssemblyBuilder::new().unwrap();
    builder
        .register_effect::<ReserveEvmNonce<EvmU256>, EvmNonceReservationEffect>()
        .unwrap();
    builder
        .register_effect::<PrepareEvmTransaction<EvmU256>, EvmTransactionPreparationEffect>()
        .unwrap();
    builder
        .register_effect::<ExecuteEvmTransaction<EvmU256>, EvmTransactionEffect>()
        .unwrap();
    builder
        .register_pure::<ProjectEvmTransactionOutcome<EvmU256>>()
        .unwrap();
    register_evm_transaction_adapters(&mut builder, binding.clone(), signer, authority, provider)
        .unwrap();
    Runtime::new(builder.finish(), store)
}
fn program(binding: &EvmTransactionBinding) -> mfm_program::Program {
    expand_program(
        EntryPointId::new("mfm.test/transaction@1").unwrap(),
        &TransactionProgram(binding.clone()),
    )
    .unwrap()
}
fn run_id() -> RunId {
    RunId::from_digest(DigestBytes::from_array([8; 32]))
}
fn settled<T>(result: Result<EffectAdapterOutcome<T>, AdapterError>) -> T {
    match result.unwrap() {
        EffectAdapterOutcome::Settled(value) => value,
        _ => panic!("expected settled stage"),
    }
}
async fn reserve(
    binding: &EvmTransactionBinding,
    authority: &MemoryAuthority,
    provider: &ScriptedProvider,
    id: &EffectId,
    command: &Eip1559TransactionCommand,
) -> ReservedEvmTransaction {
    let reservation = settled(
        reserve_nonce(
            binding,
            authority,
            provider,
            id,
            &command_value_ref(command),
            command,
        )
        .await,
    );
    ReservedEvmTransaction::new(command.clone(), reservation).unwrap()
}

#[tokio::test]
async fn graph_retries_identical_wire_and_cold_projection_needs_no_signer_call() {
    for reverted in [false, true] {
        let (_owner, signer, binding, command, _) = fixture().await;
        let authority = Arc::new(MemoryAuthority::new(binding.authority_epoch().clone()));
        let provider = Arc::new(ScriptedProvider::new(1337));
        let store = Arc::new(MemoryStore::new());
        let hot = runtime(
            &binding,
            signer.clone(),
            authority.clone(),
            provider.clone(),
            store.clone(),
        );
        let _view = hot
            .start(
                run_id(),
                program(&binding),
                EvmTransactionContext::new(EvmU256::from_u64(42), command),
            )
            .await
            .unwrap();
        let prepared = authority.state().unwrap().prepared.unwrap();
        let reject = Arc::new(RejectingSigner::matching(signer.as_ref()));
        let cold = runtime(
            &binding,
            reject,
            authority.clone(),
            provider.clone(),
            store.clone(),
        );
        let _view = cold.resume(&run_id()).await.unwrap();
        let submitted: Vec<_> = provider
            .operations()
            .into_iter()
            .filter_map(|op| {
                if let ProviderOperation::SubmitRaw(raw) = op {
                    Some(raw)
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(submitted.len(), 2);
        assert!(submitted
            .iter()
            .all(|raw| raw == prepared.raw_transaction().as_bytes()));
        assert_eq!(
            provider
                .operations()
                .iter()
                .filter(|op| matches!(op, ProviderOperation::PendingNonce(_)))
                .count(),
            1
        );
        let result = if reverted {
            ProviderReceiptResult::RevertedCreate
        } else {
            ProviderReceiptResult::SuccessCreate {
                contract_address: create_address(binding.sender(), 7),
            }
        };
        provider.push_receipt(Ok(Some(ProviderReceipt::new(
            prepared.transaction_hash().clone(),
            binding.sender().clone(),
            result,
            provider.canonical.clone(),
        ))));
        let terminal = cold.resume(&run_id()).await.unwrap();
        assert!(if reverted {
            matches!(terminal.state(), RunViewState::Failed(_))
        } else {
            matches!(terminal.state(), RunViewState::Succeeded(_))
        });
        let operation_count = provider.operations().len();
        assert_eq!(
            cold.read(&run_id()).await.unwrap().head_digest(),
            terminal.head_digest()
        );
        assert_eq!(provider.operations().len(), operation_count);
        let wire = match terminal.state() {
            RunViewState::Succeeded(value) | RunViewState::Failed(value) => {
                std::str::from_utf8(value.canonical_bytes()).unwrap()
            }
            _ => panic!("terminal result"),
        };
        assert!(!wire.contains("raw_transaction"));
    }
}

#[tokio::test]
async fn custody_acknowledgement_loss_recovers_each_stage() {
    for fault in [1, 2] {
        let (_owner, signer, binding, command, _) = fixture().await;
        let authority = Arc::new(MemoryAuthority::new(binding.authority_epoch().clone()));
        authority.fault.store(fault, Ordering::SeqCst);
        let provider = Arc::new(ScriptedProvider::new(1337));
        let store = Arc::new(MemoryStore::new());
        let hot = runtime(
            &binding,
            signer.clone(),
            authority.clone(),
            provider.clone(),
            store.clone(),
        );
        assert!(matches!(
            hot.start(
                run_id(),
                program(&binding),
                EvmTransactionContext::new(EvmU256::from_u64(0), command)
            )
            .await,
            Err(RuntimeError::Unavailable)
        ));
        let signer: Arc<dyn Secp256k1Signer> = if fault == 2 {
            Arc::new(RejectingSigner::matching(signer.as_ref()))
        } else {
            signer
        };
        let cold = runtime(&binding, signer, authority.clone(), provider.clone(), store);
        cold.resume(&run_id()).await.unwrap();
        assert!(authority.state().unwrap().prepared.is_some());
        assert_eq!(
            provider
                .operations()
                .iter()
                .filter(|op| matches!(op, ProviderOperation::PendingNonce(_)))
                .count(),
            1
        );
        assert!(provider
            .operations()
            .iter()
            .any(|op| matches!(op, ProviderOperation::SubmitRaw(_))));
    }
}

#[tokio::test]
async fn concurrent_varying_signatures_return_the_immutable_first_winner() {
    let (_owner, _, binding, command, id) = fixture().await;
    let authority = Arc::new(MemoryAuthority::new(binding.authority_epoch().clone()));
    let provider = ScriptedProvider::new(1337);
    let reserved = reserve(&binding, &authority, &provider, &id, &command).await;
    let signer = Arc::new(VaryingSigner::new(Arc::new(Barrier::new(2))));
    let mut tasks = Vec::new();
    for _ in 0..2 {
        let (authority, signer, binding, reserved, id) = (
            authority.clone(),
            signer.clone(),
            binding.clone(),
            reserved.clone(),
            id.clone(),
        );
        tasks.push(tokio::spawn(async move {
            settled(
                prepare_transaction(
                    &binding,
                    authority.as_ref(),
                    signer.as_ref(),
                    &id,
                    &reserved,
                )
                .await,
            )
        }));
    }
    let winner = tasks.remove(0).await.unwrap();
    assert_eq!(tasks.remove(0).await.unwrap(), winner);
    assert_eq!(signer.sequence.load(Ordering::SeqCst), 3);
    assert_eq!(
        authority
            .state()
            .unwrap()
            .prepared
            .unwrap()
            .transaction_hash(),
        winner.transaction_hash()
    );
}

#[tokio::test]
async fn receipt_shape_canonicality_and_submission_failures_preserve_prepared_bytes() {
    let (_owner, signer, binding, command, id) = fixture().await;
    let authority = MemoryAuthority::new(binding.authority_epoch().clone());
    let provider = ScriptedProvider::new(1337);
    let reserved = reserve(&binding, &authority, &provider, &id, &command).await;
    let evidence =
        settled(prepare_transaction(&binding, &authority, signer.as_ref(), &id, &reserved).await);
    let prepared = PreparedEvmTransaction::new(reserved, evidence.transaction_hash().clone());
    for result in [
        ProviderReceiptResult::SuccessCall {
            target: binding.sender().clone(),
        },
        ProviderReceiptResult::RevertedCall {
            target: binding.sender().clone(),
        },
        ProviderReceiptResult::SuccessCreate {
            contract_address: binding.sender().clone(),
        },
    ] {
        provider.push_receipt(Ok(Some(ProviderReceipt::new(
            prepared.transaction_hash().clone(),
            binding.sender().clone(),
            result,
            provider.canonical.clone(),
        ))));
        assert_eq!(
            execute_transaction(&binding, &authority, &provider, &id, &prepared)
                .await
                .err(),
            Some(AdapterError::Internal)
        );
    }
    provider.push_receipt(Ok(Some(ProviderReceipt::new(
        prepared.transaction_hash().clone(),
        binding.sender().clone(),
        ProviderReceiptResult::RevertedCreate,
        EvmBlockAnchor::new(EvmU256::from_u64(9), EvmHash::from_bytes([0x44; 32])),
    ))));
    assert_eq!(
        execute_transaction(&binding, &authority, &provider, &id, &prepared)
            .await
            .err(),
        Some(AdapterError::Unavailable)
    );
    for submission in [
        Err(AdapterError::Unavailable),
        Ok(Some(EvmHash::from_bytes([0x55; 32]))),
    ] {
        provider.push_submission(submission);
        assert_eq!(
            execute_transaction(&binding, &authority, &provider, &id, &prepared)
                .await
                .err(),
            Some(AdapterError::Unavailable)
        );
    }
    assert_eq!(
        authority
            .state()
            .unwrap()
            .prepared
            .unwrap()
            .transaction_hash(),
        prepared.transaction_hash()
    );
}

#[tokio::test]
async fn incorrect_signatures_and_corrupt_retained_wire_fail_before_provider_entry() {
    let (owner, signer, binding, command, id) = fixture().await;
    let authority = MemoryAuthority::new(binding.authority_epoch().clone());
    let provider = ScriptedProvider::new(1337);
    let reserved = reserve(&binding, &authority, &provider, &id, &command).await;
    let wrong = WrongDigestSigner {
        inner: owner
            .import_secp256k1(
                SecretSecp256k1Scalar::new([7; 32]).unwrap(),
                StableId::new(EVM_EIP1559_SIGNING_PURPOSE_ID).unwrap(),
            )
            .await
            .unwrap(),
    };
    assert_eq!(
        prepare_transaction(&binding, &authority, &wrong, &id, &reserved)
            .await
            .err(),
        Some(AdapterError::Internal)
    );
    assert!(authority.state().unwrap().prepared.is_none());
    let evidence =
        settled(prepare_transaction(&binding, &authority, signer.as_ref(), &id, &reserved).await);
    let prepared = PreparedEvmTransaction::new(reserved, evidence.transaction_hash().clone());
    authority.state.lock().unwrap().as_mut().unwrap().prepared = Some(PreparedRecord::new(
        prepared.transaction_hash().clone(),
        ExactRawTransaction::new(vec![2, 0xc0]).unwrap(),
    ));
    let operations = provider.operations();
    assert_eq!(
        execute_transaction(&binding, &authority, &provider, &id, &prepared)
            .await
            .err(),
        Some(AdapterError::Internal)
    );
    assert_eq!(provider.operations(), operations);
}

struct FaultStore {
    inner: MemoryStore,
    fail_sequence: AtomicU64,
    commit: bool,
}
impl mfm_store::Store for FaultStore {
    fn load_run<'a>(
        &'a self,
        id: &'a RunId,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<Option<mfm_journal::StoredRunBytes>, mfm_store::StoreError>>
                + Send
                + 'a,
        >,
    > {
        self.inner.load_run(id)
    }
    fn append_run<'a>(
        &'a self,
        frame: &'a mfm_journal::EncodedRunFrame,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<mfm_store::AppendResult, mfm_store::StoreError>> + Send + 'a,
        >,
    > {
        Box::pin(async move {
            assert!(!frame
                .canonical_bytes()
                .windows(b"raw_transaction".len())
                .any(|window| window == b"raw_transaction"));
            if self
                .fail_sequence
                .compare_exchange(frame.run_sequence(), 0, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
            {
                if self.commit {
                    self.inner.append_run(frame).await?;
                }
                return Err(mfm_store::StoreError::Indeterminate);
            }
            self.inner.append_run(frame).await
        })
    }
}

#[tokio::test]
async fn every_transaction_journal_boundary_recovers_after_ambiguous_append() {
    for commit in [false, true] {
        for sequence in 2..=8 {
            let (_owner, signer, binding, command, _) = fixture().await;
            let authority = Arc::new(MemoryAuthority::new(binding.authority_epoch().clone()));
            let provider = Arc::new(ScriptedProvider::new(1337));
            let store = Arc::new(FaultStore {
                inner: MemoryStore::new(),
                fail_sequence: AtomicU64::new(sequence),
                commit,
            });
            let hot = runtime(
                &binding,
                signer.clone(),
                authority.clone(),
                provider.clone(),
                store.clone(),
            );
            let _ = hot
                .start(
                    run_id(),
                    program(&binding),
                    EvmTransactionContext::new(EvmU256::from_u64(42), command),
                )
                .await;
            let mut completed = None;
            for _ in 0..5 {
                let prepared = authority.state().and_then(|state| state.prepared);
                let next_signer: Arc<dyn Secp256k1Signer> = if let Some(prepared) = prepared {
                    provider.push_receipt(Ok(Some(ProviderReceipt::new(
                        prepared.transaction_hash().clone(),
                        binding.sender().clone(),
                        ProviderReceiptResult::SuccessCreate {
                            contract_address: create_address(binding.sender(), 7),
                        },
                        provider.canonical.clone(),
                    ))));
                    Arc::new(RejectingSigner::matching(signer.as_ref()))
                } else {
                    signer.clone()
                };
                let cold = runtime(
                    &binding,
                    next_signer,
                    authority.clone(),
                    provider.clone(),
                    store.clone(),
                );
                match cold.resume(&run_id()).await {
                    Ok(view) if matches!(view.state(), RunViewState::Succeeded(_)) => {
                        completed = Some(view);
                        break;
                    }
                    Ok(view) => assert!(matches!(view.state(), RunViewState::Runnable)),
                    Err(RuntimeError::Indeterminate) => {}
                    Err(error) => panic!("unexpected recovery result: {error:?}"),
                }
            }
            let completed = completed.expect("bounded cold recovery");
            assert_eq!(completed.head_sequence(), 8);
            assert_eq!(store.fail_sequence.load(Ordering::SeqCst), 0);
            assert_eq!(authority.state().unwrap().reservation.nonce(), 7);
        }
    }
}

#[tokio::test]
async fn cancelled_receipt_wait_resumes_exact_prepared_wire() {
    let (_owner, signer, binding, command, _) = fixture().await;
    let authority = Arc::new(MemoryAuthority::new(binding.authority_epoch().clone()));
    let provider = Arc::new(ScriptedProvider::new(1337));
    provider.block_receipt_once.store(true, Ordering::SeqCst);
    let store = Arc::new(MemoryStore::new());
    let hot = runtime(
        &binding,
        signer.clone(),
        authority.clone(),
        provider.clone(),
        store.clone(),
    );
    let entry = program(&binding);
    let task = tokio::spawn(async move {
        hot.start(
            run_id(),
            entry,
            EvmTransactionContext::new(EvmU256::from_u64(42), command),
        )
        .await
    });
    provider.receipt_entered.notified().await;
    task.abort();
    assert!(matches!(task.await, Err(error) if error.is_cancelled()));
    let prepared = authority.state().unwrap().prepared.unwrap();
    let cold = runtime(
        &binding,
        Arc::new(RejectingSigner::matching(signer.as_ref())),
        authority,
        provider.clone(),
        store,
    );
    assert!(matches!(
        cold.resume(&run_id()).await.unwrap().state(),
        RunViewState::Runnable
    ));
    assert!(provider.operations().iter().any(|op| matches!(op, ProviderOperation::SubmitRaw(raw) if raw == prepared.raw_transaction().as_bytes())));
}
