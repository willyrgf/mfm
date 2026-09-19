//! Observes real managed-provider calls without changing their results or error ownership.
use super::*;
use mfm_evm::{EvmBlockAnchor, EvmChainInstance};
use mfm_evm_live::{ProviderFuture, ProviderReceipt};
use std::sync::{atomic::AtomicUsize, Mutex};

#[derive(Default)]
pub(super) struct ProviderCalls {
    pub(super) absent_receipts: AtomicUsize,
    pub(super) known_transactions: AtomicUsize,
    pub(super) submitted: Mutex<Vec<EvmHash>>,
    pub(super) broadcast: tokio::sync::Notify,
}
pub(super) struct ObservedProvider {
    pub(super) inner: Arc<JsonRpcEvmProvider>,
    pub(super) calls: Arc<ProviderCalls>,
}
impl EvmTransactionProvider for ObservedProvider {
    fn chain_instance(&self) -> ProviderFuture<'_, EvmChainInstance> {
        self.inner.chain_instance()
    }
    fn pending_nonce<'a>(&'a self, sender: &'a EvmAddress) -> ProviderFuture<'a, u64> {
        self.inner.pending_nonce(sender)
    }
    fn receipt<'a>(&'a self, hash: &'a EvmHash) -> ProviderFuture<'a, Option<ProviderReceipt>> {
        Box::pin(async move {
            let receipt = self.inner.receipt(hash).await?;
            if receipt.is_none() {
                self.calls.absent_receipts.fetch_add(1, Ordering::SeqCst);
            }
            Ok(receipt)
        })
    }
    fn transaction_known<'a>(&'a self, hash: &'a EvmHash) -> ProviderFuture<'a, bool> {
        Box::pin(async move {
            let known = self.inner.transaction_known(hash).await?;
            if known {
                self.calls.known_transactions.fetch_add(1, Ordering::SeqCst);
            }
            Ok(known)
        })
    }
    fn canonical_block<'a>(&'a self, number: &'a EvmU256) -> ProviderFuture<'a, EvmBlockAnchor> {
        self.inner.canonical_block(number)
    }
    fn submit_raw<'a>(
        &'a self,
        raw: &'a mfm_evm::custody::ExactRawTransaction,
    ) -> ProviderFuture<'a, EvmHash> {
        Box::pin(async move {
            let hash = self.inner.submit_raw(raw).await?;
            self.calls.submitted.lock().unwrap().push(hash.clone());
            self.calls.broadcast.notify_one();
            Ok(hash)
        })
    }
}
pub(super) struct ObservedSigner {
    pub(super) inner: Arc<dyn Secp256k1Signer>,
    pub(super) calls: Arc<AtomicUsize>,
}
impl Secp256k1Signer for ObservedSigner {
    fn public_key(&self) -> &mfm_signing::Secp256k1PublicKey {
        self.inner.public_key()
    }
    fn purpose(&self) -> &StableId {
        self.inner.purpose()
    }
    fn sign(&self, digest: mfm_signing::SigningDigest) -> mfm_signing::SigningFuture {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.inner.sign(digest)
    }
}
