#![warn(missing_docs)]
//! Thread-affine in-process custody for recoverable secp256k1 keys.
//!
//! A private non-`Send`, non-`Sync` key container exists only inside its dedicated owner thread. Async
//! callers use [`KeystoreOwner`] and key- and purpose-bound [`KeystoreSigner`] handles; private
//! scalars never cross back out of the owner.

use std::marker::PhantomData;
use std::rc::Rc;
use std::thread::JoinHandle;

use k256::ecdsa::SigningKey;
use mfm_ids::StableId;
use mfm_signing::{
    CompactRecoverableSignature, Secp256k1PublicKey, Secp256k1Signer, SigningDigest, SigningError,
    SigningFuture,
};
use mfm_values::DiagnosticEvidence;
use serde_json::json;
use tokio::sync::{mpsc, oneshot};
use zeroize::Zeroizing;

/// Maximum number of distinct public key instances retained by one owner.
pub const MAX_KEY_INSTANCES: usize = 64;

/// Redaction-safe keystore error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum KeystoreError {
    /// A scalar or public signing value was invalid.
    #[error("keystore input is invalid")]
    Invalid,
    /// The fixed key-instance capacity was exhausted.
    #[error("keystore capacity exceeded")]
    Capacity,
    /// The owner thread or bounded command channel was unavailable.
    #[error("keystore is unavailable")]
    Unavailable,
    /// The owner thread violated a trusted local invariant.
    #[error("keystore operation failed")]
    Internal,
}

/// Owned zeroizing secp256k1 secret scalar accepted by the owner.
///
/// This type intentionally implements neither `Debug`, `Display`, nor serde and exposes no byte
/// accessor.
pub struct SecretSecp256k1Scalar(SigningKey);

impl SecretSecp256k1Scalar {
    /// Validates a nonzero scalar strictly below the secp256k1 group order.
    pub fn new(bytes: [u8; 32]) -> Result<Self, KeystoreError> {
        let bytes = Zeroizing::new(bytes);
        SigningKey::from_slice(bytes.as_ref())
            .map(Self)
            .map_err(|_| KeystoreError::Invalid)
    }

    fn into_signing_key(self) -> SigningKey {
        self.0
    }
}

/// Non-`Send`, non-`Sync` secret owner retained only by its OS thread.
struct Keystore {
    entries: Vec<SigningKey>,
    _thread_affinity: PhantomData<Rc<()>>,
}

impl Keystore {
    fn new() -> Self {
        Self {
            entries: Vec::new(),
            _thread_affinity: PhantomData,
        }
    }

    fn import(&mut self, secret: SecretSecp256k1Scalar) -> Result<ImportedKey, KeystoreError> {
        let signing_key = secret.into_signing_key();
        let public_key = Secp256k1PublicKey::try_from(*signing_key.verifying_key())
            .map_err(map_checked_signing_error)?;
        if let Some(slot) = self
            .entries
            .iter()
            .position(|entry| entry.verifying_key() == signing_key.verifying_key())
        {
            return Ok(ImportedKey {
                slot: KeySlot(slot),
                public_key,
            });
        }
        if self.entries.len() >= MAX_KEY_INSTANCES {
            return Err(KeystoreError::Capacity);
        }
        let slot = KeySlot(self.entries.len());
        self.entries.push(signing_key);
        Ok(ImportedKey { slot, public_key })
    }

    fn sign(
        &self,
        slot: KeySlot,
        digest: SigningDigest,
    ) -> Result<CompactRecoverableSignature, SigningError> {
        let signing_key = self.entries.get(slot.0).ok_or_else(|| {
            SigningError::SignFailed(DiagnosticEvidence::from_value(json!({
                "operation": "sign", "stage": "key_lookup", "kind": "missing_key"
            })))
        })?;
        let (signature, recovery_id) = signing_key
            .sign_prehash_recoverable(digest.as_bytes())
            .map_err(|error| {
                SigningError::SignFailed(DiagnosticEvidence::from_value(json!({
                    "operation": "sign",
                    "stage": "sign_prehash_recoverable",
                    "kind": "signature_error",
                    "message": error.to_string()
                })))
            })?;
        CompactRecoverableSignature::try_from((signature, recovery_id))
    }
}

#[derive(Clone, Copy)]
struct KeySlot(usize);

struct ImportedKey {
    slot: KeySlot,
    public_key: Secp256k1PublicKey,
}

enum Command {
    Import {
        secret: SecretSecp256k1Scalar,
        response: oneshot::Sender<Result<ImportedKey, KeystoreError>>,
    },
    Sign {
        slot: KeySlot,
        digest: SigningDigest,
        response: oneshot::Sender<Result<CompactRecoverableSignature, SigningError>>,
    },
    Shutdown,
    #[cfg(test)]
    Panic,
}

/// Unique controller for one dedicated keystore owner thread.
pub struct KeystoreOwner {
    sender: mpsc::Sender<Command>,
    join: JoinHandle<()>,
}

impl KeystoreOwner {
    /// Starts an empty owner on one dedicated named OS thread.
    pub fn start() -> Result<Self, KeystoreError> {
        let (sender, receiver) = mpsc::channel(MAX_KEY_INSTANCES);
        let join = std::thread::Builder::new()
            .name("mfm-keystore-owner".to_owned())
            .spawn(move || owner_loop(receiver))
            .map_err(|_| KeystoreError::Unavailable)?;
        Ok(Self { sender, join })
    }

    /// Imports one checked secret and returns a key- and purpose-bound signer handle.
    pub async fn import_secp256k1(
        &self,
        secret: SecretSecp256k1Scalar,
        purpose: StableId,
    ) -> Result<KeystoreSigner, KeystoreError> {
        let (response, result) = oneshot::channel();
        self.sender
            .send(Command::Import { secret, response })
            .await
            .map_err(|_| KeystoreError::Unavailable)?;
        let imported = result.await.map_err(|_| KeystoreError::Internal)??;
        Ok(KeystoreSigner {
            sender: self.sender.clone(),
            slot: imported.slot,
            public_key: imported.public_key,
            purpose,
        })
    }

    /// Requests owner exit and asynchronously joins the dedicated OS thread.
    pub async fn shutdown(self) -> Result<(), KeystoreError> {
        let Self { sender, join } = self;
        let sent = sender.send(Command::Shutdown).await;
        drop(sender);
        let joined = tokio::task::spawn_blocking(move || join.join())
            .await
            .map_err(|_| KeystoreError::Internal)?;
        if joined.is_err() {
            return Err(KeystoreError::Internal);
        }
        sent.map_err(|_| KeystoreError::Unavailable)
    }
}

/// Cloneable key- and purpose-bound sender for one retained secret key.
#[derive(Clone)]
pub struct KeystoreSigner {
    sender: mpsc::Sender<Command>,
    slot: KeySlot,
    public_key: Secp256k1PublicKey,
    purpose: StableId,
}

impl Secp256k1Signer for KeystoreSigner {
    fn public_key(&self) -> &Secp256k1PublicKey {
        &self.public_key
    }

    fn purpose(&self) -> &StableId {
        &self.purpose
    }

    fn sign(&self, digest: SigningDigest) -> SigningFuture {
        let sender = self.sender.clone();
        let slot = self.slot;
        Box::pin(async move {
            let (response, result) = oneshot::channel();
            sender
                .send(Command::Sign {
                    slot,
                    digest,
                    response,
                })
                .await
                .map_err(|error| {
                    SigningError::SignFailed(DiagnosticEvidence::from_value(json!({
                        "operation": "sign",
                        "stage": "request_send",
                        "kind": "channel_closed",
                        "message": error.to_string()
                    })))
                })?;
            result.await.map_err(|error| {
                SigningError::SignFailed(DiagnosticEvidence::from_value(json!({
                    "operation": "sign",
                    "stage": "reply_receive",
                    "kind": "channel_closed",
                    "message": error.to_string()
                })))
            })?
        })
    }
}

fn owner_loop(mut receiver: mpsc::Receiver<Command>) {
    let mut keystore = Keystore::new();
    while let Some(command) = receiver.blocking_recv() {
        match command {
            Command::Import { secret, response } => {
                let _ = response.send(keystore.import(secret));
            }
            Command::Sign {
                slot,
                digest,
                response,
            } => {
                let _ = response.send(keystore.sign(slot, digest));
            }
            Command::Shutdown => break,
            #[cfg(test)]
            Command::Panic => panic!("injected owner panic"),
        }
    }
}

fn map_checked_signing_error(_error: SigningError) -> KeystoreError {
    KeystoreError::Internal
}

#[cfg(test)]
mod tests {
    use super::*;

    static_assertions::assert_not_impl_any!(Keystore: Send, Sync);
    static_assertions::assert_not_impl_any!(
        SecretSecp256k1Scalar: std::fmt::Debug, std::fmt::Display, serde::Serialize
    );

    #[test]
    fn last_sender_permits_owner_exit() {
        let owner = KeystoreOwner::start().expect("owner");
        let KeystoreOwner { sender, join } = owner;
        drop(sender);
        join.join().expect("last sender exit");
    }

    #[tokio::test]
    async fn owner_panic_precedes_failed_shutdown_send_without_secret_diagnostics() {
        let owner = KeystoreOwner::start().expect("owner");
        owner
            .sender
            .send(Command::Panic)
            .await
            .expect("panic command");
        while !owner.join.is_finished() {
            tokio::task::yield_now().await;
        }
        assert_eq!(owner.shutdown().await, Err(KeystoreError::Internal));
        assert_eq!(
            KeystoreError::Internal.to_string(),
            "keystore operation failed"
        );
    }
    #[tokio::test]
    async fn sign_reply_closure_retains_its_actual_channel_stage() {
        let owner = KeystoreOwner::start().unwrap();
        let mut signer = owner
            .import_secp256k1(
                SecretSecp256k1Scalar::new([1; 32]).unwrap(),
                StableId::new("mfm.test/reply-closed@1").unwrap(),
            )
            .await
            .unwrap();
        let (sender, mut requests) = mpsc::channel(1);
        signer.sender = sender;
        let reply_closer = tokio::spawn(async move {
            let Some(Command::Sign { response, .. }) = requests.recv().await else {
                panic!("one sign request")
            };
            drop(response);
        });
        let error = signer
            .sign(SigningDigest::from_bytes([0xa5; 32]))
            .await
            .err()
            .unwrap();
        reply_closer.await.unwrap();
        assert_eq!(
            serde_json::to_value(&error).unwrap(),
            json!({
                "kind": "sign_failed",
                "cause": {"operation": "sign", "stage": "reply_receive", "kind": "channel_closed", "message": "channel closed"}
            })
        );
        owner.shutdown().await.unwrap();
    }

    #[test]
    fn missing_slot_and_primitive_errors_have_only_available_nonsecret_facts() {
        let error = Keystore::new()
            .sign(KeySlot(0), SigningDigest::from_bytes([0xa5; 32]))
            .err()
            .unwrap();
        assert_eq!(
            serde_json::to_value(error).unwrap(),
            json!({
                "kind": "sign_failed",
                "cause": {"operation": "sign", "stage": "key_lookup", "kind": "missing_key"}
            })
        );
        // This primitive rejects an invalid prehash length; no signing fault harness is needed.
        let key = SecretSecp256k1Scalar::new([1; 32])
            .unwrap()
            .into_signing_key();
        let error = key.sign_prehash_recoverable(&[]).err().unwrap();
        assert_eq!(error.to_string(), "signature error");
        assert!(std::error::Error::source(&error).is_none());
    }
}
