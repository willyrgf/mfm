#![warn(missing_docs)]
//! Thread-affine in-process custody for recoverable secp256k1 keys.
//!
//! The non-`Send`, non-`Sync` [`Keystore`] exists only inside its dedicated owner thread. Async
//! callers hold bounded command senders and key- and purpose-bound [`KeystoreSigner`] handles;
//! private scalars never cross back out of the owner.

use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::Arc;
use std::thread::JoinHandle;

use k256::ecdsa::SigningKey;
use mfm_ids::{ContentRef, StableId};
use mfm_signing::{
    CompactRecoverableSignature, PublicSignerIdentity, PublicSigningKey, Signer, SigningDigest,
    SigningError, SigningFuture, UncompressedSec1PublicKey, IN_PROCESS_KEYSTORE_SIGNER_ROUTE_ID,
};
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
pub struct SecretSecp256k1Scalar(Zeroizing<[u8; 32]>);

impl SecretSecp256k1Scalar {
    /// Validates a nonzero scalar strictly below the secp256k1 group order.
    pub fn new(bytes: [u8; 32]) -> Result<Self, KeystoreError> {
        let bytes = Zeroizing::new(bytes);
        SigningKey::from_slice(bytes.as_ref()).map_err(|_| KeystoreError::Invalid)?;
        Ok(Self(bytes))
    }

    fn into_signing_key(self) -> Result<SigningKey, KeystoreError> {
        SigningKey::from_slice(self.0.as_ref()).map_err(|_| KeystoreError::Invalid)
    }
}

struct KeyEntry {
    signing_key: SigningKey,
    identity: PublicSignerIdentity,
}

/// Non-`Send`, non-`Sync` secret owner retained only by its OS thread.
pub struct Keystore {
    entries: BTreeMap<ContentRef, KeyEntry>,
    _thread_affinity: Rc<()>,
}

impl Keystore {
    fn new() -> Self {
        Self {
            entries: BTreeMap::new(),
            _thread_affinity: Rc::new(()),
        }
    }

    fn import(
        &mut self,
        secret: SecretSecp256k1Scalar,
    ) -> Result<PublicSignerIdentity, KeystoreError> {
        let signing_key = secret.into_signing_key()?;
        let encoded = signing_key.verifying_key().to_encoded_point(false);
        let public_bytes: [u8; 65] = encoded
            .as_bytes()
            .try_into()
            .map_err(|_| KeystoreError::Internal)?;
        let public_key = PublicSigningKey::new(
            UncompressedSec1PublicKey::new(public_bytes).map_err(map_signing_error)?,
        )
        .map_err(map_signing_error)?;
        let identity = PublicSignerIdentity::new(
            StableId::new(IN_PROCESS_KEYSTORE_SIGNER_ROUTE_ID)
                .map_err(|_| KeystoreError::Internal)?,
            public_key,
        );
        let key_ref = identity.key_instance_ref().map_err(map_signing_error)?;
        if let Some(existing) = self.entries.get(&key_ref) {
            return Ok(existing.identity.clone());
        }
        if self.entries.len() >= MAX_KEY_INSTANCES {
            return Err(KeystoreError::Capacity);
        }
        self.entries.insert(
            key_ref,
            KeyEntry {
                signing_key,
                identity: identity.clone(),
            },
        );
        Ok(identity)
    }

    fn sign(
        &self,
        key_ref: &ContentRef,
        digest: SigningDigest,
    ) -> Result<CompactRecoverableSignature, KeystoreError> {
        let entry = self.entries.get(key_ref).ok_or(KeystoreError::Invalid)?;
        let (signature, recovery_id) = entry
            .signing_key
            .sign_prehash_recoverable(digest.as_bytes())
            .map_err(|_| KeystoreError::Internal)?;
        let bytes: [u8; 64] = signature.to_bytes().into();
        CompactRecoverableSignature::new(bytes, recovery_id.to_byte()).map_err(map_signing_error)
    }
}

enum Command {
    Import {
        secret: SecretSecp256k1Scalar,
        response: oneshot::Sender<Result<PublicSignerIdentity, KeystoreError>>,
    },
    Sign {
        key_ref: ContentRef,
        digest: SigningDigest,
        response: oneshot::Sender<Result<CompactRecoverableSignature, KeystoreError>>,
    },
    Shutdown {
        response: oneshot::Sender<()>,
    },
    #[cfg(test)]
    Panic,
    #[cfg(test)]
    Block {
        entered: std::sync::mpsc::Sender<()>,
        release: std::sync::mpsc::Receiver<()>,
    },
    #[cfg(test)]
    Noop,
}

/// Unique controller for one dedicated keystore owner thread.
pub struct KeystoreOwner {
    sender: Option<mpsc::Sender<Command>>,
    join: Option<JoinHandle<()>>,
}

impl KeystoreOwner {
    /// Starts an empty owner on one dedicated named OS thread.
    pub fn start() -> Result<Self, KeystoreError> {
        let (sender, receiver) = mpsc::channel(MAX_KEY_INSTANCES);
        let join = std::thread::Builder::new()
            .name("mfm-keystore-owner".to_owned())
            .spawn(move || owner_loop(receiver))
            .map_err(|_| KeystoreError::Unavailable)?;
        Ok(Self {
            sender: Some(sender),
            join: Some(join),
        })
    }

    /// Imports one checked secret and returns a key- and purpose-bound signer handle.
    pub async fn import_secp256k1(
        &self,
        secret: SecretSecp256k1Scalar,
        purpose: StableId,
    ) -> Result<KeystoreSigner, KeystoreError> {
        let sender = self.sender.as_ref().ok_or(KeystoreError::Unavailable)?;
        let (response, result) = oneshot::channel();
        sender
            .send(Command::Import { secret, response })
            .await
            .map_err(|_| KeystoreError::Unavailable)?;
        let identity = result.await.map_err(|_| KeystoreError::Internal)??;
        Ok(KeystoreSigner {
            sender: sender.clone(),
            identity: Arc::new(identity),
            purpose,
        })
    }

    /// Requests owner exit and asynchronously joins the dedicated OS thread.
    pub async fn shutdown(mut self) -> Result<(), KeystoreError> {
        let sender = self.sender.take().ok_or(KeystoreError::Unavailable)?;
        let (response, stopped) = oneshot::channel();
        let request = sender
            .send(Command::Shutdown { response })
            .await
            .map_err(|_| KeystoreError::Unavailable);
        drop(sender);
        let acknowledgement = match request {
            Ok(()) => stopped.await.map_err(|_| KeystoreError::Internal),
            Err(error) => Err(error),
        };
        let join = self.join.take().ok_or(KeystoreError::Internal)?;
        let joined = tokio::task::spawn_blocking(move || join.join())
            .await
            .map_err(|_| KeystoreError::Internal)?
            .map_err(|_| KeystoreError::Internal);
        acknowledgement.and(joined)
    }
}

/// Cloneable key- and purpose-bound sender for one retained secret key.
#[derive(Clone)]
pub struct KeystoreSigner {
    sender: mpsc::Sender<Command>,
    identity: Arc<PublicSignerIdentity>,
    purpose: StableId,
}

impl Signer for KeystoreSigner {
    fn public_identity(&self) -> &PublicSignerIdentity {
        &self.identity
    }

    fn purpose(&self) -> &StableId {
        &self.purpose
    }

    fn sign(&self, digest: SigningDigest) -> SigningFuture {
        let sender = self.sender.clone();
        let key_ref = match self.identity.key_instance_ref() {
            Ok(reference) => reference,
            Err(_) => return Box::pin(async { Err(SigningError::Failed) }),
        };
        Box::pin(async move {
            let (response, result) = oneshot::channel();
            sender
                .send(Command::Sign {
                    key_ref,
                    digest,
                    response,
                })
                .await
                .map_err(|_| SigningError::Failed)?;
            result
                .await
                .map_err(|_| SigningError::Failed)?
                .map_err(|_| SigningError::Failed)
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
                key_ref,
                digest,
                response,
            } => {
                let _ = response.send(keystore.sign(&key_ref, digest));
            }
            Command::Shutdown { response } => {
                let _ = response.send(());
                break;
            }
            #[cfg(test)]
            Command::Panic => panic!("injected owner panic"),
            #[cfg(test)]
            Command::Block { entered, release } => {
                let _ = entered.send(());
                let _ = release.recv();
            }
            #[cfg(test)]
            Command::Noop => {}
        }
    }
}

fn map_signing_error(_error: SigningError) -> KeystoreError {
    KeystoreError::Invalid
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn last_sender_exit_and_owner_panic_are_joinable_without_secret_diagnostics() {
        let owner = KeystoreOwner::start().expect("owner");
        let KeystoreOwner { sender, mut join } = owner;
        drop(sender);
        join.take().expect("join").join().expect("last sender exit");

        let owner = KeystoreOwner::start().expect("owner");
        let sender = owner.sender.as_ref().expect("sender").clone();
        sender.blocking_send(Command::Panic).expect("panic command");
        let KeystoreOwner { sender, mut join } = owner;
        drop(sender);
        assert!(join.take().expect("join").join().is_err());
        assert_eq!(
            KeystoreError::Internal.to_string(),
            "keystore operation failed"
        );
    }

    #[test]
    fn owner_command_channel_has_exact_bounded_backpressure() {
        let owner = KeystoreOwner::start().expect("owner");
        let sender = owner.sender.as_ref().expect("sender").clone();
        let (entered, observed_entry) = std::sync::mpsc::channel();
        let (release, await_release) = std::sync::mpsc::channel();
        sender
            .blocking_send(Command::Block {
                entered,
                release: await_release,
            })
            .expect("block command");
        observed_entry.recv().expect("owner entered block");
        for _ in 0..MAX_KEY_INSTANCES {
            sender.try_send(Command::Noop).expect("within capacity");
        }
        assert!(matches!(
            sender.try_send(Command::Noop),
            Err(mpsc::error::TrySendError::Full(Command::Noop))
        ));
        release.send(()).expect("release owner");
        drop(sender);
        let KeystoreOwner { sender, mut join } = owner;
        drop(sender);
        join.take().expect("join").join().expect("owner exit");
    }
}
