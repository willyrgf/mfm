#![warn(missing_docs)]
//! Thread-affine in-process custody for recoverable secp256k1 keys.
//!
//! A private non-`Send`, non-`Sync` key map exists only inside its dedicated owner thread. Async
//! callers use [`KeystoreOwner`] and key- and purpose-bound [`KeystoreSigner`] handles; private
//! scalars never cross back out of the owner.

use std::collections::BTreeMap;
use std::rc::Rc;
use std::thread::JoinHandle;

use k256::ecdsa::SigningKey;
use mfm_ids::StableId;
use mfm_signing::{
    CompactRecoverableSignature, Secp256k1PublicKey, Secp256k1Signer, SigningDigest, SigningError,
    SigningFuture,
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

/// Non-`Send`, non-`Sync` secret owner retained only by its OS thread.
struct Keystore {
    entries: BTreeMap<[u8; 65], SigningKey>,
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
    ) -> Result<Secp256k1PublicKey, KeystoreError> {
        let signing_key = secret.into_signing_key()?;
        let encoded = signing_key.verifying_key().to_encoded_point(false);
        let public_bytes: [u8; 65] = encoded
            .as_bytes()
            .try_into()
            .map_err(|_| KeystoreError::Internal)?;
        let public_key = Secp256k1PublicKey::new(public_bytes).map_err(map_signing_error)?;
        if self.entries.contains_key(&public_bytes) {
            return Ok(public_key);
        }
        if self.entries.len() >= MAX_KEY_INSTANCES {
            return Err(KeystoreError::Capacity);
        }
        self.entries.insert(public_bytes, signing_key);
        Ok(public_key)
    }

    fn sign(
        &self,
        public_key: &[u8; 65],
        digest: SigningDigest,
    ) -> Result<CompactRecoverableSignature, KeystoreError> {
        let signing_key = self.entries.get(public_key).ok_or(KeystoreError::Invalid)?;
        let (signature, recovery_id) = signing_key
            .sign_prehash_recoverable(digest.as_bytes())
            .map_err(|_| KeystoreError::Internal)?;
        let bytes: [u8; 64] = signature.to_bytes().into();
        CompactRecoverableSignature::new(bytes, recovery_id.to_byte()).map_err(map_signing_error)
    }
}

enum Command {
    Import {
        secret: SecretSecp256k1Scalar,
        response: oneshot::Sender<Result<Secp256k1PublicKey, KeystoreError>>,
    },
    Sign {
        public_key: [u8; 65],
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
        let public_key = result.await.map_err(|_| KeystoreError::Internal)??;
        Ok(KeystoreSigner {
            sender: sender.clone(),
            public_key,
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
        let public_key = *self.public_key.as_bytes();
        Box::pin(async move {
            let (response, result) = oneshot::channel();
            sender
                .send(Command::Sign {
                    public_key,
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
                public_key,
                digest,
                response,
            } => {
                let _ = response.send(keystore.sign(&public_key, digest));
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

    static_assertions::assert_not_impl_any!(Keystore: Send, Sync);
    static_assertions::assert_not_impl_any!(
        SecretSecp256k1Scalar: std::fmt::Debug, std::fmt::Display, serde::Serialize
    );

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
