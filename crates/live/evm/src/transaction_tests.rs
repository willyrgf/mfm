use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use mfm_evm::{
    Eip1559TransactionCommand, EvmAuthorityEpoch, EvmChainInstance, EvmEndpoint,
    EvmTransactionAction, EvmTransactionBinding, EvmTransactionRoute, EvmU256, EvmWalletIdentity,
};
use mfm_evm_transaction_authority::{
    AuthorityFuture, AuthorityState, EvmTransactionAuthority, PreparedRecord, Reservation,
    SettledRecord, MAX_EXACT_RAW_TRANSACTION_BYTES,
};
use mfm_ids::{DigestBytes, EffectId};
use mfm_keystore::{KeystoreOwner, KeystoreSigner, SecretSecp256k1Scalar};
use mfm_signing::{PublicSignerIdentity, Signer, SigningDigest, SigningFuture};
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
        mfm_signing::UncompressedSec1PublicKey::new(public_bytes.try_into().expect("65 bytes"))
            .expect("generator key");
    assert_eq!(
        ethereum_address(&public_key).as_str(),
        "0x7e5f4552091a69125d5dfcb7b8c2659029395bdf"
    );
}

struct RecordingSigner {
    inner: KeystoreSigner,
    calls: AtomicUsize,
    purposes: Mutex<Vec<String>>,
}

impl Signer for RecordingSigner {
    fn public_identity(&self) -> &PublicSignerIdentity {
        self.inner.public_identity()
    }

    fn sign(&self, digest: SigningDigest, purpose: StableId) -> SigningFuture {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.purposes
            .lock()
            .expect("purpose lock")
            .push(purpose.as_str().to_owned());
        self.inner.sign(digest, purpose)
    }
}

struct MemoryAuthority {
    epoch: EvmAuthorityEpoch,
    state: Mutex<Option<AuthorityState>>,
    loads: AtomicUsize,
}

impl MemoryAuthority {
    fn new(epoch: EvmAuthorityEpoch) -> Self {
        Self {
            epoch,
            state: Mutex::new(None),
            loads: AtomicUsize::new(0),
        }
    }

    fn state(&self) -> Option<AuthorityState> {
        self.state.lock().expect("authority lock").clone()
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
            self.loads.fetch_add(1, Ordering::SeqCst);
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

struct Provider {
    chain: ObservedChainInstance,
    pending: u64,
    receipt: Mutex<Option<ProviderReceipt>>,
    canonical: EvmBlockAnchor,
    chain_calls: AtomicUsize,
    pending_calls: AtomicUsize,
    receipt_calls: AtomicUsize,
    canonical_calls: AtomicUsize,
    submits: AtomicUsize,
    block_receipt_once: AtomicBool,
    receipt_entered: Notify,
    release_receipt: Notify,
}

impl Provider {
    fn new(chain_id: u64) -> Self {
        Self {
            chain: ObservedChainInstance::new(chain_id, EvmHash::new(GENESIS).expect("genesis"))
                .expect("chain"),
            pending: 7,
            receipt: Mutex::new(None),
            canonical: EvmBlockAnchor::new(
                EvmU256::from_u64(9),
                EvmHash::new(BLOCK).expect("block"),
            ),
            chain_calls: AtomicUsize::new(0),
            pending_calls: AtomicUsize::new(0),
            receipt_calls: AtomicUsize::new(0),
            canonical_calls: AtomicUsize::new(0),
            submits: AtomicUsize::new(0),
            block_receipt_once: AtomicBool::new(false),
            receipt_entered: Notify::new(),
            release_receipt: Notify::new(),
        }
    }
}

impl EvmTransactionProvider for Provider {
    fn chain_instance(&self) -> EvmTransactionProviderFuture<'_, ObservedChainInstance> {
        Box::pin(async move {
            self.chain_calls.fetch_add(1, Ordering::SeqCst);
            Ok(self.chain.clone())
        })
    }

    fn pending_nonce<'a>(
        &'a self,
        _sender: &'a EvmAddress,
    ) -> EvmTransactionProviderFuture<'a, u64> {
        Box::pin(async move {
            self.pending_calls.fetch_add(1, Ordering::SeqCst);
            Ok(self.pending)
        })
    }

    fn receipt<'a>(
        &'a self,
        _transaction_hash: &'a EvmHash,
    ) -> EvmTransactionProviderFuture<'a, Option<ProviderReceipt>> {
        Box::pin(async move {
            self.receipt_calls.fetch_add(1, Ordering::SeqCst);
            if self.block_receipt_once.swap(false, Ordering::SeqCst) {
                self.receipt_entered.notify_one();
                self.release_receipt.notified().await;
            }
            Ok(self.receipt.lock().expect("receipt lock").clone())
        })
    }

    fn canonical_block<'a>(
        &'a self,
        _block_number: &'a EvmU256,
    ) -> EvmTransactionProviderFuture<'a, EvmBlockAnchor> {
        Box::pin(async move {
            self.canonical_calls.fetch_add(1, Ordering::SeqCst);
            Ok(self.canonical.clone())
        })
    }

    fn submit_raw<'a>(
        &'a self,
        raw_transaction: &'a ExactRawTransaction,
    ) -> EvmTransactionProviderFuture<'a, EvmHash> {
        Box::pin(async move {
            self.submits.fetch_add(1, Ordering::SeqCst);
            evm_keccak256(raw_transaction.as_bytes()).map_err(|_| AdapterError::Internal)
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
        .import_secp256k1(SecretSecp256k1Scalar::new([7; 32]).expect("fixture secret"))
        .await
        .expect("signer");
    let sender = ethereum_address(
        &signer
            .public_identity()
            .public_key()
            .public_key()
            .expect("public key"),
    );
    let signer = Arc::new(RecordingSigner {
        inner: signer,
        calls: AtomicUsize::new(0),
        purposes: Mutex::new(Vec::new()),
    });
    let route = EvmTransactionRoute::new(
        EvmChainInstance::new(1337, EvmHash::new(GENESIS).expect("genesis")).expect("chain"),
        EvmEndpoint::new("transaction-test")
            .expect("endpoint")
            .endpoint_ref()
            .expect("endpoint ref"),
    );
    let binding = EvmTransactionBinding::new(
        route,
        EvmAuthorityEpoch::new([3; 32]),
        EvmWalletIdentity::new(sender, signer.public_identity().clone()),
    );
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
    let provider: Arc<dyn EvmTransactionProvider> = Arc::new(Provider::new(1337));
    let signer: Arc<dyn Signer> = signer;
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
    let provider = Provider::new(1337);

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
    assert_eq!(provider.pending_calls.load(Ordering::SeqCst), 1);
    assert_eq!(provider.receipt_calls.load(Ordering::SeqCst), 1);
    assert_eq!(provider.submits.load(Ordering::SeqCst), 1);
    assert_eq!(
        signer.purposes.lock().expect("purposes").as_slice(),
        [EVM_EIP1559_SIGNING_PURPOSE_ID]
    );

    let created = create_address(binding.wallet().sender(), 7).expect("created address");
    *provider.receipt.lock().expect("receipt lock") = Some(ProviderReceipt::new(
        prepared.transaction_hash().clone(),
        binding.wallet().sender().clone(),
        ProviderReceiptResult::SuccessCreate {
            contract_address: created.clone(),
        },
        provider.canonical.clone(),
    ));
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
        evidence.result(),
        EvmTransactionTerminalResult::SuccessCreate { created_address }
            if created_address == &created
    ));
    assert_eq!(signer.calls.load(Ordering::SeqCst), 1);
    assert_eq!(provider.receipt_calls.load(Ordering::SeqCst), 3);
    assert_eq!(provider.canonical_calls.load(Ordering::SeqCst), 2);

    let phase_counts = (
        signer.calls.load(Ordering::SeqCst),
        provider.chain_calls.load(Ordering::SeqCst),
        provider.receipt_calls.load(Ordering::SeqCst),
    );
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
        Ok(EffectAdapterOutcome::Settled(evidence))
    );
    assert_eq!(
        phase_counts,
        (
            signer.calls.load(Ordering::SeqCst),
            provider.chain_calls.load(Ordering::SeqCst),
            provider.receipt_calls.load(Ordering::SeqCst),
        )
    );
}

#[tokio::test]
async fn cancellation_after_prepare_leaves_one_resumable_exact_transaction() {
    let (_owner, signer, binding, command, effect_id) = fixture().await;
    let authority = MemoryAuthority::new(binding.authority_epoch().clone());
    let provider = Provider::new(1337);
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
    assert_eq!(provider.pending_calls.load(Ordering::SeqCst), 1);
    assert_eq!(provider.submits.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn local_binding_and_corrupt_prepared_bytes_are_internal_before_provider_entry() {
    let (_owner, signer, binding, command, effect_id) = fixture().await;
    let authority = MemoryAuthority::new(binding.authority_epoch().clone());
    let provider = Provider::new(1337);
    let wrong_binding = EvmTransactionBinding::new(
        binding.route().clone(),
        EvmAuthorityEpoch::new([4; 32]),
        binding.wallet().clone(),
    );
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
    assert_eq!(authority.loads.load(Ordering::SeqCst), 0);
    assert_eq!(provider.chain_calls.load(Ordering::SeqCst), 0);

    let local = validate_local(&binding, &command, signer.as_ref(), &authority).expect("local");
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
    assert_eq!(provider.chain_calls.load(Ordering::SeqCst), 0);
}
