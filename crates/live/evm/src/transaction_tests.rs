use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use mfm_evm::{
    Eip1559TransactionCommand, EvmAuthorityEpoch, EvmChainInstance, EvmEndpoint,
    EvmTransactionAction, EvmTransactionBinding, EvmTransactionConfirmation, EvmTransactionRoute,
    EvmTransactionSettlement, EvmU256,
};
use mfm_evm_transaction_authority::{
    AuthorityFuture, AuthorityState, EvmTransactionAuthority, PreparedRecord, Reservation,
    SettledRecord, MAX_EXACT_RAW_TRANSACTION_BYTES,
};
use mfm_ids::{DigestBytes, EffectId, StableId};
use mfm_keystore::{KeystoreOwner, KeystoreSigner, SecretSecp256k1Scalar};
use mfm_signing::{Secp256k1PublicKey, Secp256k1Signer, SigningDigest, SigningFuture};
use tokio::sync::Notify;

use super::*;
use crate::{evm_keccak256, ObservedChainInstance};

const GENESIS: &str = "0x1111111111111111111111111111111111111111111111111111111111111111";
const BLOCK: &str = "0x2222222222222222222222222222222222222222222222222222222222222222";

#[test]
fn public_keccak_and_ethereum_address_helpers_are_exact_and_bounded() {
    assert_eq!(
        evm_keccak256(&[]).expect("empty Keccak").as_str(),
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
        ethereum_address(&public_key).as_str(),
        "0x7e5f4552091a69125d5dfcb7b8c2659029395bdf"
    );
}

struct RecordingSigner {
    inner: KeystoreSigner,
    calls: AtomicUsize,
}

struct WrongDigestSigner {
    inner: KeystoreSigner,
    calls: AtomicUsize,
}

impl Secp256k1Signer for WrongDigestSigner {
    fn public_key(&self) -> &Secp256k1PublicKey {
        self.inner.public_key()
    }

    fn purpose(&self) -> &StableId {
        self.inner.purpose()
    }

    fn sign(&self, _digest: SigningDigest) -> SigningFuture {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.inner.sign(SigningDigest::from_bytes([0x44; 32]))
    }
}

impl Secp256k1Signer for RecordingSigner {
    fn public_key(&self) -> &Secp256k1PublicKey {
        self.inner.public_key()
    }

    fn purpose(&self) -> &StableId {
        self.inner.purpose()
    }

    fn sign(&self, digest: SigningDigest) -> SigningFuture {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.inner.sign(digest)
    }
}

struct MemoryAuthority {
    epoch: EvmAuthorityEpoch,
    state: Mutex<Option<AuthorityState>>,
    operations: Mutex<Vec<AuthorityOperation>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AuthorityOperation {
    Load,
    Reserve,
    RetainPrepared,
    RetainSettlement,
}

impl MemoryAuthority {
    fn new(epoch: EvmAuthorityEpoch) -> Self {
        Self {
            epoch,
            state: Mutex::new(None),
            operations: Mutex::new(Vec::new()),
        }
    }

    fn state(&self) -> Option<AuthorityState> {
        self.state.lock().expect("authority lock").clone()
    }

    fn operations(&self) -> Vec<AuthorityOperation> {
        self.operations.lock().expect("operations lock").clone()
    }
}

impl EvmTransactionAuthority for MemoryAuthority {
    fn authority_epoch(&self) -> &EvmAuthorityEpoch {
        &self.epoch
    }

    fn load<'a>(
        &'a self,
        effect_id: &'a EffectId,
        expected_command_ref: &'a ContentRef,
    ) -> AuthorityFuture<'a, Option<AuthorityState>> {
        Box::pin(async move {
            self.operations
                .lock()
                .expect("operations lock")
                .push(AuthorityOperation::Load);
            let state = self.state();
            if let Some(state) = &state {
                let reservation = state_reservation(state);
                if reservation.effect_id() != effect_id
                    || reservation.command_ref() != expected_command_ref
                {
                    return Err(AuthorityError::Internal);
                }
            }
            Ok(state)
        })
    }

    fn reserve_or_compare<'a>(
        &'a self,
        effect_id: &'a EffectId,
        command_ref: &'a ContentRef,
        domain: &'a NonceDomain,
        observed_pending_nonce: u64,
    ) -> AuthorityFuture<'a, Reservation> {
        Box::pin(async move {
            self.operations
                .lock()
                .expect("operations lock")
                .push(AuthorityOperation::Reserve);
            let mut state = self.state.lock().expect("authority lock");
            if let Some(existing) = &*state {
                return Ok(state_reservation(existing).clone());
            }
            let reservation = Reservation::new(
                effect_id.clone(),
                command_ref.clone(),
                domain.clone(),
                observed_pending_nonce,
            );
            *state = Some(AuthorityState::Reserved(reservation.clone()));
            Ok(reservation)
        })
    }

    fn retain_prepared<'a>(
        &'a self,
        effect_id: &'a EffectId,
        command_ref: &'a ContentRef,
        transaction_hash: &'a EvmHash,
        raw_transaction: &'a ExactRawTransaction,
    ) -> AuthorityFuture<'a, PreparedRecord> {
        Box::pin(async move {
            self.operations
                .lock()
                .expect("operations lock")
                .push(AuthorityOperation::RetainPrepared);
            let mut state = self.state.lock().expect("authority lock");
            match state.as_ref() {
                Some(AuthorityState::Reserved(reservation)) => {
                    if reservation.effect_id() != effect_id
                        || reservation.command_ref() != command_ref
                    {
                        return Err(AuthorityError::Internal);
                    }
                    let prepared = PreparedRecord::new(
                        reservation.clone(),
                        transaction_hash.clone(),
                        raw_transaction.clone(),
                    );
                    *state = Some(AuthorityState::Prepared(prepared.clone()));
                    Ok(prepared)
                }
                Some(AuthorityState::Prepared(prepared))
                    if prepared.transaction_hash() == transaction_hash
                        && prepared.raw_transaction() == raw_transaction =>
                {
                    Ok(prepared.clone())
                }
                _ => Err(AuthorityError::Internal),
            }
        })
    }

    fn retain_settlement<'a>(
        &'a self,
        effect_id: &'a EffectId,
        command_ref: &'a ContentRef,
        evidence: &'a EvmTransactionSettlement,
    ) -> AuthorityFuture<'a, SettledRecord> {
        Box::pin(async move {
            self.operations
                .lock()
                .expect("operations lock")
                .push(AuthorityOperation::RetainSettlement);
            let mut state = self.state.lock().expect("authority lock");
            match state.as_ref() {
                Some(AuthorityState::Prepared(prepared))
                    if prepared.reservation().effect_id() == effect_id
                        && prepared.reservation().command_ref() == command_ref =>
                {
                    let settled = SettledRecord::new(prepared.clone(), evidence.clone())?;
                    *state = Some(AuthorityState::Settled(settled.clone()));
                    Ok(settled)
                }
                Some(AuthorityState::Settled(settled)) if settled.evidence() == evidence => {
                    Ok(settled.clone())
                }
                _ => Err(AuthorityError::Internal),
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
    chain: ObservedChainInstance,
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
            chain: ObservedChainInstance::new(chain_id, EvmHash::new(GENESIS).expect("genesis"))
                .expect("chain"),
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
    fn chain_instance(&self) -> EvmTransactionProviderFuture<'_, ObservedChainInstance> {
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
    Arc<RecordingSigner>,
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
    let signer = Arc::new(RecordingSigner {
        inner: signer,
        calls: AtomicUsize::new(0),
    });
    let route = EvmTransactionRoute::new(
        EvmChainInstance::new(1337, EvmHash::new(GENESIS).expect("genesis")).expect("chain"),
        EvmEndpoint::new("transaction-test")
            .expect("endpoint")
            .endpoint_ref()
            .expect("endpoint ref"),
    );
    let binding = EvmTransactionBinding::new(route, EvmAuthorityEpoch::new([3; 32]), sender);
    let command = Eip1559TransactionCommand::new(
        binding.clone(),
        EvmTransactionAction::create(vec![0x60, 0x00]).expect("initcode"),
        EvmU256::from_u64(0),
        100_000,
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
    let mut builder = RuntimeAssemblyBuilder::new();
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
    builder.finish().expect("assembly");
}

#[tokio::test]
async fn absent_prepare_submit_resume_settle_and_fast_path_are_phase_exact() {
    let (_owner, signer, binding, command, effect_id) = fixture().await;
    let authority = MemoryAuthority::new(binding.authority_epoch().clone());
    let provider = ScriptedProvider::new(1337);

    assert_eq!(
        execute(
            &binding,
            &effect_id,
            &command,
            signer.as_ref(),
            &authority,
            &provider,
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
        prepared.transaction_hash().as_str(),
        "0x1c491e5220c082f1be50e5e78738147d1416633942ac5e689d65798127a5e71b"
    );
    assert_eq!(prepared.reservation().nonce(), 7);
    assert_eq!(signer.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        provider.operations(),
        [
            ProviderOperation::ChainInstance,
            ProviderOperation::PendingNonce(binding.sender().clone()),
            ProviderOperation::Receipt(prepared.transaction_hash().clone()),
            ProviderOperation::SubmitRaw(prepared.raw_transaction().as_bytes().to_vec()),
        ]
    );
    assert_eq!(signer.purpose().as_str(), EVM_EIP1559_SIGNING_PURPOSE_ID);

    let created = create_address(binding.sender(), 7).expect("created address");
    provider.push_receipt(Ok(Some(ProviderReceipt::new(
        prepared.transaction_hash().clone(),
        binding.sender().clone(),
        ProviderReceiptResult::SuccessCreate {
            contract_address: created.clone(),
        },
        provider.canonical.clone(),
    ))));
    let EffectAdapterOutcome::Settled(evidence) = execute(
        &binding,
        &effect_id,
        &command,
        signer.as_ref(),
        &authority,
        &provider,
    )
    .await
    .expect("settled receipt") else {
        panic!("receipt must settle the Effect")
    };
    assert_eq!(evidence.nonce(), 7);
    assert!(matches!(
        &evidence,
        EvmTransactionSettlement::Confirmed {
            confirmation: EvmTransactionConfirmation::Created { created_address, .. },
            ..
        }
            if created_address == &created
    ));
    assert_eq!(signer.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        &provider.operations()[4..],
        [
            ProviderOperation::ChainInstance,
            ProviderOperation::Receipt(prepared.transaction_hash().clone()),
            ProviderOperation::CanonicalBlock(provider.canonical.number().clone()),
        ]
    );

    let signer_calls = signer.calls.load(Ordering::SeqCst);
    let provider_operations = provider.operations();
    assert_eq!(
        execute(
            &binding,
            &effect_id,
            &command,
            &signer.inner,
            &authority,
            &provider,
        )
        .await,
        Ok(EffectAdapterOutcome::Settled(evidence))
    );
    assert_eq!(signer.calls.load(Ordering::SeqCst), signer_calls);
    assert_eq!(provider.operations(), provider_operations);
}

#[tokio::test]
async fn cancellation_after_prepare_leaves_one_resumable_exact_transaction() {
    let (_owner, signer, binding, command, effect_id) = fixture().await;
    let authority = MemoryAuthority::new(binding.authority_epoch().clone());
    let provider = ScriptedProvider::new(1337);
    provider.block_receipt_once.store(true, Ordering::SeqCst);
    {
        let entered = provider.receipt_entered.notified();
        let invocation = execute(
            &binding,
            &effect_id,
            &command,
            signer.as_ref(),
            &authority,
            &provider,
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

    assert_eq!(
        execute(
            &binding,
            &effect_id,
            &command,
            signer.as_ref(),
            &authority,
            &provider,
        )
        .await,
        Ok(EffectAdapterOutcome::Pending)
    );
    assert_eq!(signer.calls.load(Ordering::SeqCst), 1);
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
    let authority = MemoryAuthority::new(binding.authority_epoch().clone());
    let provider = ScriptedProvider::new(1337);

    for _ in 0..2 {
        assert_eq!(
            execute(
                &binding,
                &effect_id,
                &command,
                signer.as_ref(),
                &authority,
                &provider,
            )
            .await,
            Ok(EffectAdapterOutcome::Pending)
        );
    }
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
    assert_eq!(
        authority.operations(),
        [
            AuthorityOperation::Load,
            AuthorityOperation::Reserve,
            AuthorityOperation::RetainPrepared,
            AuthorityOperation::Load,
        ]
    );
}

#[tokio::test]
async fn preloaded_reservation_skips_pending_nonce_and_resumes_preparation() {
    let (_owner, signer, binding, command, effect_id) = fixture().await;
    let authority = MemoryAuthority::new(binding.authority_epoch().clone());
    let provider = ScriptedProvider::new(1337);
    let local = validate_local(&binding, &command, signer.as_ref(), &authority).expect("local");
    *authority.state.lock().expect("authority lock") =
        Some(AuthorityState::Reserved(Reservation::new(
            effect_id.clone(),
            local.command_ref.clone(),
            local.domain,
            7,
        )));

    assert_eq!(
        execute(
            &binding,
            &effect_id,
            &command,
            signer.as_ref(),
            &authority,
            &provider,
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
        let authority = MemoryAuthority::new(binding.authority_epoch().clone());
        let provider = ScriptedProvider::new(1337);
        provider.push_submission(submission);

        assert_eq!(
            execute(
                &binding,
                &effect_id,
                &command,
                signer.as_ref(),
                &authority,
                &provider,
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
    let authority = MemoryAuthority::new(binding.authority_epoch().clone());
    let provider = ScriptedProvider::new(1337);
    assert_eq!(
        execute(
            &binding,
            &effect_id,
            &create_command,
            signer.as_ref(),
            &authority,
            &provider,
        )
        .await,
        Ok(EffectAdapterOutcome::Pending)
    );
    let AuthorityState::Prepared(prepared) = authority.state().expect("prepared") else {
        panic!("fixture must prepare")
    };
    let local = validate_local(&binding, &create_command, signer.as_ref(), &authority)
        .expect("local binding");
    let target =
        EvmAddress::new("0x4444444444444444444444444444444444444444").expect("call target");
    let call_command = Eip1559TransactionCommand::new(
        binding.clone(),
        EvmTransactionAction::call(target.clone(), vec![1, 2]).expect("call action"),
        EvmU256::from_u64(0),
        100_000,
        EvmU256::from_u64(2),
        EvmU256::from_u64(10),
    )
    .expect("call command");
    let created =
        create_address(binding.sender(), prepared.reservation().nonce()).expect("created address");

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
            Expected::Created => assert!(matches!(
                actual,
                Ok(EvmTransactionSettlement::Confirmed {
                    confirmation: EvmTransactionConfirmation::Created { .. },
                    ..
                })
            )),
            Expected::Called => assert!(matches!(
                actual,
                Ok(EvmTransactionSettlement::Confirmed {
                    confirmation: EvmTransactionConfirmation::Called { .. },
                    ..
                })
            )),
            Expected::Reverted => {
                assert!(matches!(
                    actual,
                    Ok(EvmTransactionSettlement::Reverted { .. })
                ))
            }
            Expected::Error => assert_eq!(actual, Err(AdapterError::Unavailable)),
        }
    }
}

#[tokio::test]
async fn noncanonical_receipt_anchor_stops_before_settlement() {
    let (_owner, signer, binding, command, effect_id) = fixture().await;
    let authority = MemoryAuthority::new(binding.authority_epoch().clone());
    let provider = ScriptedProvider::new(1337);
    assert_eq!(
        execute(
            &binding,
            &effect_id,
            &command,
            signer.as_ref(),
            &authority,
            &provider,
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
            signer.as_ref(),
            &authority,
            &provider,
        )
        .await,
        Err(AdapterError::Unavailable)
    );
    assert!(matches!(
        authority.state(),
        Some(AuthorityState::Prepared(_))
    ));
    assert!(provider
        .operations()
        .contains(&ProviderOperation::CanonicalBlock(
            other_anchor.number().clone()
        )));
    assert!(!authority
        .operations()
        .contains(&AuthorityOperation::RetainSettlement));
}

#[tokio::test]
async fn wrong_returned_signature_is_rejected_before_preparation_or_submission() {
    let (_owner, signer, binding, command, effect_id) = fixture().await;
    let signer = WrongDigestSigner {
        inner: signer.inner.clone(),
        calls: AtomicUsize::new(0),
    };
    let authority = MemoryAuthority::new(binding.authority_epoch().clone());
    let provider = ScriptedProvider::new(1337);

    assert_eq!(
        execute(&binding, &effect_id, &command, &signer, &authority, &provider,).await,
        Err(AdapterError::Internal)
    );
    assert_eq!(signer.calls.load(Ordering::SeqCst), 1);
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
    let authority = MemoryAuthority::new(binding.authority_epoch().clone());
    let provider = ScriptedProvider::new(1337);
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
                EvmChainInstance::new(1338, EvmHash::new(GENESIS).expect("genesis"))
                    .expect("other chain"),
                binding.route().endpoint_ref().clone(),
            ),
            binding.authority_epoch().clone(),
            binding.sender().clone(),
        ),
        EvmTransactionBinding::new(
            EvmTransactionRoute::new(
                EvmChainInstance::new(1337, other_genesis).expect("other genesis chain"),
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
                signer.as_ref(),
                &authority,
                &provider,
            )
            .await,
            Err(AdapterError::Internal)
        );
    }
    assert!(authority.operations().is_empty());
    assert!(provider.operations().is_empty());

    let wrong_epoch_authority = MemoryAuthority::new(EvmAuthorityEpoch::new([4; 32]));
    assert_eq!(
        execute(
            &binding,
            &effect_id,
            &command,
            signer.as_ref(),
            &wrong_epoch_authority,
            &provider,
        )
        .await,
        Err(AdapterError::Internal)
    );
    assert!(wrong_epoch_authority.operations().is_empty());
    assert!(provider.operations().is_empty());

    let wrong_purpose = Arc::new(RecordingSigner {
        inner: owner
            .import_secp256k1(
                SecretSecp256k1Scalar::new([7; 32]).expect("fixture secret"),
                StableId::new("mfm.test.evm/wrong-purpose@1").expect("wrong purpose"),
            )
            .await
            .expect("same key under wrong purpose"),
        calls: AtomicUsize::new(0),
    });
    assert_eq!(
        execute(
            &binding,
            &effect_id,
            &command,
            wrong_purpose.as_ref(),
            &authority,
            &provider,
        )
        .await,
        Err(AdapterError::Internal)
    );
    assert_eq!(wrong_purpose.calls.load(Ordering::SeqCst), 0);
    assert!(authority.operations().is_empty());
    assert!(provider.operations().is_empty());

    let wrong_key = Arc::new(RecordingSigner {
        inner: owner
            .import_secp256k1(
                SecretSecp256k1Scalar::new([8; 32]).expect("different fixture secret"),
                StableId::new(EVM_EIP1559_SIGNING_PURPOSE_ID).expect("signing purpose"),
            )
            .await
            .expect("different key under correct purpose"),
        calls: AtomicUsize::new(0),
    });
    assert_eq!(
        execute(
            &binding,
            &effect_id,
            &command,
            wrong_key.as_ref(),
            &authority,
            &provider,
        )
        .await,
        Err(AdapterError::Internal)
    );
    assert_eq!(wrong_key.calls.load(Ordering::SeqCst), 0);
    assert!(authority.operations().is_empty());
    assert!(provider.operations().is_empty());

    let local = validate_local(&binding, &command, signer.as_ref(), &authority).expect("local");
    let wrong_domain = NonceDomain::new(
        binding.authority_epoch().clone(),
        binding.route().chain_instance().clone(),
        EvmAddress::new("0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").expect("wrong sender"),
    );
    *authority.state.lock().expect("authority lock") =
        Some(AuthorityState::Reserved(Reservation::new(
            effect_id.clone(),
            local.command_ref.clone(),
            wrong_domain,
            7,
        )));
    assert_eq!(
        execute(
            &binding,
            &effect_id,
            &command,
            signer.as_ref(),
            &authority,
            &provider,
        )
        .await,
        Err(AdapterError::Internal)
    );
    assert!(provider.operations().is_empty());

    let reservation = Reservation::new(
        effect_id.clone(),
        local.command_ref.clone(),
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
            signer.as_ref(),
            &authority,
            &provider,
        )
        .await,
        Err(AdapterError::Internal)
    );
    assert!(provider.operations().is_empty());
}
