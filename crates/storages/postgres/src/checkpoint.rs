//! External checkpoint authority for PostgreSQL append acknowledgement.
//!
//! Checkpoint state deliberately lives outside the database process.  A restored or
//! forked database therefore cannot manufacture an acknowledged successor by replaying
//! rows from its own catalog.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use mfm_canonical::sha256_digest_bytes;
use mfm_ids::{ContentDigest, DigestAlgorithm, StoreEpoch, StoreScopeId};

/// Stream for which an external append checkpoint is recorded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CheckpointStream {
    /// Structured run history.
    Run,
    /// Append-only configured values.
    Configuration,
    /// Dense tenant fact publication history.
    TenantFacts,
}

/// Stable key identifying one externally acknowledged stream successor.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct CheckpointKey {
    /// Immutable target store scope.
    pub store_scope_id: StoreScopeId,
    /// Immutable target store epoch.
    pub store_epoch: StoreEpoch,
    /// Stream family.
    pub stream: CheckpointStream,
    /// Logical stream identifier, such as a run or tenant.
    pub stream_id: String,
    /// Expected predecessor digest.
    pub predecessor: ContentDigest,
}

/// Externally durable checkpoint state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckpointState {
    /// The successor is prepared but the database commit acknowledgement is not yet known.
    Prepared {
        /// Digest of the prepared successor bytes.
        successor: ContentDigest,
    },
    /// The successor was acknowledged by the deployment authority.
    Acknowledged {
        /// Digest of the acknowledged successor bytes.
        successor: ContentDigest,
    },
}

/// Deployment-owned authority for prepared and acknowledged append checkpoints.
pub trait ExternalCheckpointAuthority: Send + Sync {
    /// Records a prepared successor before a database commit is attempted.
    fn prepare(&self, key: CheckpointKey, successor_bytes: &[u8]) -> CheckpointState;

    /// Promotes a prepared successor to acknowledged state after commit acknowledgement.
    fn acknowledge(
        &self,
        key: &CheckpointKey,
        successor_bytes: &[u8],
    ) -> Result<(), CheckpointError>;

    /// Returns the externally retained state for one exact predecessor.
    fn state(&self, key: &CheckpointKey) -> Option<CheckpointState>;
}

/// Redaction-safe checkpoint rejection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CheckpointError {
    /// A different successor is already prepared or acknowledged for this predecessor.
    #[error("checkpoint successor conflicts with the retained predecessor")]
    Conflict,
    /// Acknowledgement did not follow the prepared successor byte-for-byte.
    #[error("checkpoint acknowledgement does not match the prepared successor")]
    NotPrepared,
}

/// Small deployment-side in-memory implementation useful for process-local orchestration and
/// deterministic tests. Production deployments replace it with durable external state.
#[derive(Clone, Default)]
pub struct ExternalCheckpointLedger {
    states: Arc<Mutex<BTreeMap<CheckpointKey, CheckpointState>>>,
}

impl ExternalCheckpointLedger {
    fn digest(bytes: &[u8]) -> ContentDigest {
        ContentDigest::from_digest(DigestAlgorithm::Sha256V1, sha256_digest_bytes(bytes))
    }
}

impl ExternalCheckpointAuthority for ExternalCheckpointLedger {
    fn prepare(&self, key: CheckpointKey, successor_bytes: &[u8]) -> CheckpointState {
        let successor = Self::digest(successor_bytes);
        let mut states = self.states.lock().expect("checkpoint mutex poisoned");
        match states.get(&key) {
            Some(existing) => existing.clone(),
            None => {
                let state = CheckpointState::Prepared { successor };
                states.insert(key, state.clone());
                state
            }
        }
    }

    fn acknowledge(
        &self,
        key: &CheckpointKey,
        successor_bytes: &[u8],
    ) -> Result<(), CheckpointError> {
        let successor = Self::digest(successor_bytes);
        let mut states = self.states.lock().expect("checkpoint mutex poisoned");
        match states.get(key) {
            Some(CheckpointState::Prepared {
                successor: expected,
            }) if *expected == successor => {
                states.insert(key.clone(), CheckpointState::Acknowledged { successor });
                Ok(())
            }
            Some(CheckpointState::Acknowledged {
                successor: expected,
            }) if *expected == successor => Ok(()),
            _ => Err(CheckpointError::NotPrepared),
        }
    }

    fn state(&self, key: &CheckpointKey) -> Option<CheckpointState> {
        self.states
            .lock()
            .expect("checkpoint mutex poisoned")
            .get(key)
            .cloned()
    }
}
