//! External checkpoint authority for PostgreSQL append acknowledgement.
//!
//! Checkpoint state deliberately lives outside the database process.  A restored or
//! forked database therefore cannot manufacture an acknowledged successor by replaying
//! rows from its own catalog.

#[cfg(feature = "test-support")]
use std::collections::{BTreeMap, BTreeSet};
#[cfg(feature = "test-support")]
use std::fs;
#[cfg(feature = "test-support")]
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(feature = "test-support")]
use std::sync::{Arc, Mutex};

#[cfg(feature = "test-support")]
use mfm_canonical::sha256_digest_bytes;
#[cfg(feature = "test-support")]
use mfm_ids::DigestAlgorithm;
use mfm_ids::{ContentDigest, StoreEpoch, StoreScopeId};
use serde::{Deserialize, Serialize};

/// Stream for which an external append checkpoint is recorded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
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
    pub(crate) store_scope_id: StoreScopeId,
    /// Immutable target store epoch.
    pub(crate) store_epoch: StoreEpoch,
    /// Stable deployment target identity. Fence generations are carried by the
    /// target admission, while this key prevents sibling targets sharing a head.
    pub(crate) target_key: String,
    /// Stream family.
    pub(crate) stream: CheckpointStream,
    /// Logical stream identifier, such as a run or tenant.
    pub(crate) stream_id: String,
    /// Expected predecessor digest.
    pub(crate) predecessor: Option<ContentDigest>,
}

impl CheckpointKey {
    /// Immutable target store scope.
    pub const fn store_scope_id(&self) -> &StoreScopeId {
        &self.store_scope_id
    }

    /// Immutable target store epoch.
    pub const fn store_epoch(&self) -> StoreEpoch {
        self.store_epoch
    }

    /// Stable deployment target identity.
    pub fn target_key(&self) -> &str {
        &self.target_key
    }

    /// Stream family.
    pub const fn stream(&self) -> CheckpointStream {
        self.stream
    }

    /// Logical stream identifier.
    pub fn stream_id(&self) -> &str {
        &self.stream_id
    }

    /// Expected predecessor digest.
    pub const fn predecessor(&self) -> Option<&ContentDigest> {
        self.predecessor.as_ref()
    }
}

#[cfg(feature = "test-support")]
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
struct StreamIdentity {
    store_scope_id: StoreScopeId,
    store_epoch: StoreEpoch,
    target_key: String,
    stream: CheckpointStream,
    stream_id: String,
}

#[cfg(feature = "test-support")]
impl From<&CheckpointKey> for StreamIdentity {
    fn from(key: &CheckpointKey) -> Self {
        Self {
            store_scope_id: key.store_scope_id.clone(),
            store_epoch: key.store_epoch,
            target_key: key.target_key.clone(),
            stream: key.stream,
            stream_id: key.stream_id.clone(),
        }
    }
}

/// One canonical successor prepared as part of an append mutation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckpointMutation {
    /// Stable predecessor key.
    pub(crate) key: CheckpointKey,
    /// Exact deployment target admission used for this successor.
    pub(crate) target: CheckpointTarget,
    /// Complete canonical successor bytes.
    pub(crate) successor_bytes: Vec<u8>,
}

impl CheckpointMutation {
    /// Stable predecessor key.
    pub const fn key(&self) -> &CheckpointKey {
        &self.key
    }

    /// Exact target admission.
    pub const fn target(&self) -> &CheckpointTarget {
        &self.target
    }

    /// Complete canonical successor bytes.
    pub fn successor_bytes(&self) -> &[u8] {
        &self.successor_bytes
    }
}

/// Exact external target identity bound to a checkpoint mutation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckpointTarget {
    /// Immutable store scope.
    pub(crate) store_scope_id: StoreScopeId,
    /// Immutable store epoch.
    pub(crate) store_epoch: StoreEpoch,
    /// Deployment target identity.
    pub(crate) target_key: String,
    /// PostgreSQL database identity observed at admission.
    pub(crate) database_oid: u32,
    /// Qualified schema identity observed at admission.
    pub(crate) schema_name: String,
    /// Current fencing generation.
    pub(crate) fence_generation: u64,
    /// Current deployment release epoch.
    pub(crate) release_epoch: u64,
}

impl CheckpointTarget {
    /// Immutable store scope.
    pub const fn store_scope_id(&self) -> &StoreScopeId {
        &self.store_scope_id
    }

    /// Immutable store epoch.
    pub const fn store_epoch(&self) -> StoreEpoch {
        self.store_epoch
    }

    /// Deployment target identity.
    pub fn target_key(&self) -> &str {
        &self.target_key
    }

    /// Database identity.
    pub const fn database_oid(&self) -> u32 {
        self.database_oid
    }

    /// Qualified schema identity.
    pub fn schema_name(&self) -> &str {
        &self.schema_name
    }

    /// Current fence generation.
    pub const fn fence_generation(&self) -> u64 {
        self.fence_generation
    }

    /// Current deployment release epoch.
    pub const fn release_epoch(&self) -> u64 {
        self.release_epoch
    }
}

/// Short-lived external fixation held while a repeatable-read snapshot is
/// established. The database head must match this value before the fixation
/// is released.
#[derive(Debug, PartialEq, Eq)]
pub struct CheckpointReadFixation {
    store_scope_id: StoreScopeId,
    store_epoch: StoreEpoch,
    target_key: String,
    stream: CheckpointStream,
    stream_id: String,
    successor: Option<ContentDigest>,
    nonce: u64,
}

static NEXT_READ_FIXATION_NONCE: AtomicU64 = AtomicU64::new(1);

impl CheckpointReadFixation {
    /// Creates one authority-issued read fixation.
    ///
    /// The returned value carries an opaque process-local nonce. Callers may
    /// retain the value to release their own fixation, but cannot copy or
    /// reconstruct another live fixation's nonce.
    pub fn issued(
        store_scope_id: StoreScopeId,
        store_epoch: StoreEpoch,
        target_key: String,
        stream: CheckpointStream,
        stream_id: String,
        successor: Option<ContentDigest>,
    ) -> Self {
        Self {
            store_scope_id,
            store_epoch,
            target_key,
            stream,
            stream_id,
            successor,
            nonce: NEXT_READ_FIXATION_NONCE.fetch_add(1, Ordering::Relaxed),
        }
    }

    /// Returns the externally fixed successor digest.
    pub fn successor(&self) -> Option<&ContentDigest> {
        self.successor.as_ref()
    }

    /// Returns the immutable store scope bound to this fixation.
    pub const fn store_scope_id(&self) -> &StoreScopeId {
        &self.store_scope_id
    }

    /// Returns the writer epoch bound to this fixation.
    pub const fn store_epoch(&self) -> StoreEpoch {
        self.store_epoch
    }

    /// Returns the deployment target identity bound to this fixation.
    pub fn target_key(&self) -> &str {
        &self.target_key
    }

    /// Returns the stream family bound to this fixation.
    pub const fn stream(&self) -> CheckpointStream {
        self.stream
    }

    /// Returns the logical stream identity bound to this fixation.
    pub fn stream_id(&self) -> &str {
        &self.stream_id
    }

    /// Returns the authority-issued nonce used to release this fixation.
    pub const fn nonce(&self) -> u64 {
        self.nonce
    }
}

/// RAII guard that releases a read fixation on every exit path.
pub(crate) struct ReadFixationGuard<'a> {
    authority: &'a dyn ExternalCheckpointAuthority,
    fixation: &'a CheckpointReadFixation,
    released: bool,
}

impl<'a> ReadFixationGuard<'a> {
    pub(crate) fn new(
        authority: &'a dyn ExternalCheckpointAuthority,
        fixation: &'a CheckpointReadFixation,
    ) -> Self {
        Self {
            authority,
            fixation,
            released: false,
        }
    }

    pub(crate) fn release(mut self) -> Result<(), CheckpointError> {
        let result = self.authority.release_read(self.fixation);
        self.released = result.is_ok();
        result
    }
}

impl Drop for ReadFixationGuard<'_> {
    fn drop(&mut self) {
        if !self.released {
            let _ = self.authority.release_read(self.fixation);
        }
    }
}

/// Externally durable checkpoint state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

impl CheckpointState {
    /// Returns the exact successor digest retained by the authority.
    pub const fn successor(&self) -> &ContentDigest {
        match self {
            Self::Prepared { successor } | Self::Acknowledged { successor } => successor,
        }
    }
}

/// Deployment-owned authority for prepared and acknowledged append checkpoints.
pub trait ExternalCheckpointAuthority:
    mfm_authority_seal::ExternalCheckpointAuthoritySeal + Send + Sync
{
    /// Registers the deployment-issued target tuple before a session bundle is
    /// returned. Durable authorities persist this admission outside PostgreSQL.
    #[allow(clippy::too_many_arguments)]
    fn register_target(
        &self,
        store_scope_id: &StoreScopeId,
        store_epoch: StoreEpoch,
        target_key: &str,
        database_oid: u32,
        schema_name: &str,
        fence_generation: u64,
        release_epoch: u64,
    ) -> Result<(), CheckpointError>;

    /// Validates the deployment-issued target admission before one protected
    /// transaction. Durable implementations bind this tuple to the target
    /// lease and reject stale promotion generations.
    #[allow(clippy::too_many_arguments)]
    fn validate_target(
        &self,
        store_scope_id: &StoreScopeId,
        store_epoch: StoreEpoch,
        target_key: &str,
        database_oid: u32,
        schema_name: &str,
        fence_generation: u64,
        release_epoch: u64,
    ) -> Result<(), CheckpointError>;

    /// Records a prepared successor before a database commit is attempted.
    fn prepare(&self, mutation: &CheckpointMutation) -> Result<CheckpointState, CheckpointError>;

    /// Promotes a prepared successor to acknowledged state after commit acknowledgement.
    fn acknowledge(&self, mutation: &CheckpointMutation) -> Result<(), CheckpointError>;

    /// Returns the externally retained state for one exact predecessor.
    fn state(&self, key: &CheckpointKey) -> Option<CheckpointState>;

    /// Returns the current acknowledged stream head for a stable stream key.
    /// Implementations must retain this outside the database process.
    fn current_head(&self, key: &CheckpointKey) -> Option<ContentDigest>;

    /// Fixates the externally acknowledged head for one read snapshot.
    fn fixate_read(&self, key: &CheckpointKey) -> Result<CheckpointReadFixation, CheckpointError>;

    /// Releases the short external barrier after the first indexed-head query
    /// has established the database snapshot.
    fn release_read(&self, fixation: &CheckpointReadFixation) -> Result<(), CheckpointError>;

    /// Prepares a sorted multi-stream successor set as one authority operation.
    fn prepare_many(
        &self,
        mutations: &[CheckpointMutation],
    ) -> Result<Vec<CheckpointState>, CheckpointError>;

    /// Acknowledges a sorted multi-stream successor set after SQL commit.
    fn acknowledge_many(&self, mutations: &[CheckpointMutation]) -> Result<(), CheckpointError>;
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
#[cfg(feature = "test-support")]
type TargetAdmissionKey = (StoreScopeId, StoreEpoch, String);
#[cfg(feature = "test-support")]
type TargetAdmissionMap = BTreeMap<TargetAdmissionKey, TargetAdmission>;
#[cfg(feature = "test-support")]
type ReadFixationMap = BTreeMap<StreamIdentity, BTreeSet<u64>>;

#[cfg(feature = "test-support")]
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct PersistedCheckpointLedger {
    states: Vec<(StreamIdentity, CheckpointState)>,
    heads: Vec<(StreamIdentity, ContentDigest)>,
    targets: Vec<(TargetAdmissionKey, TargetAdmission)>,
}

#[cfg(feature = "test-support")]
static PERSISTENCE_GATE: std::sync::OnceLock<Mutex<()>> = std::sync::OnceLock::new();
#[cfg(feature = "test-support")]
static NEXT_PERSIST_TEMP: AtomicU64 = AtomicU64::new(1);

#[cfg(feature = "test-support")]
/// Process-local checkpoint authority used by integration fixtures.
#[derive(Clone, Default)]
pub struct ExternalCheckpointLedger {
    // State is keyed by the stable stream identity. The predecessor belongs
    // to one mutation precondition, not to the stream's durable identity;
    // retaining it in the map key would make an idempotent retry address a
    // different prepared state after the head advances.
    states: Arc<Mutex<BTreeMap<StreamIdentity, CheckpointState>>>,
    heads: Arc<Mutex<BTreeMap<StreamIdentity, ContentDigest>>>,
    targets: Arc<Mutex<TargetAdmissionMap>>,
    read_fixations: Arc<Mutex<ReadFixationMap>>,
    gate: Arc<Mutex<()>>,
    persistence: Option<Arc<PathBuf>>,
}

#[cfg(feature = "test-support")]
impl mfm_authority_seal::ExternalCheckpointAuthoritySeal for ExternalCheckpointLedger {}

#[cfg(feature = "test-support")]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct TargetAdmission {
    database_oid: u32,
    schema_name: String,
    fence_generation: u64,
    release_epoch: u64,
}

#[cfg(feature = "test-support")]
impl ExternalCheckpointLedger {
    /// Creates a fixture ledger whose acknowledged state is retained in a
    /// sidecar outside the PostgreSQL process. The schema name namespaces the
    /// sidecar, allowing a fresh worker process to reopen the same target.
    pub fn for_test(schema_name: &str) -> Self {
        let mut ledger = Self {
            persistence: Some(Arc::new(test_persistence_path(schema_name))),
            ..Self::default()
        };
        ledger.load_persisted();
        ledger
    }

    fn digest(bytes: &[u8]) -> ContentDigest {
        ContentDigest::from_digest(DigestAlgorithm::Sha256V1, sha256_digest_bytes(bytes))
    }

    fn load_persisted(&mut self) {
        let Some(path) = self.persistence.as_deref() else {
            return;
        };
        let Ok(bytes) = fs::read(path) else {
            return;
        };
        let persisted: PersistedCheckpointLedger = serde_json::from_slice(&bytes)
            .expect("test checkpoint sidecar must contain valid checkpoint state");
        self.states
            .lock()
            .expect("checkpoint mutex poisoned")
            .extend(persisted.states);
        self.heads
            .lock()
            .expect("checkpoint mutex poisoned")
            .extend(persisted.heads);
        self.targets
            .lock()
            .expect("checkpoint mutex poisoned")
            .extend(persisted.targets);
    }

    fn persist(&self) {
        let Some(path) = self.persistence.as_deref() else {
            return;
        };
        let gate = PERSISTENCE_GATE.get_or_init(|| Mutex::new(()));
        let _gate = gate.lock().expect("checkpoint persistence mutex poisoned");
        let mut persisted = fs::read(path)
            .ok()
            .map(|bytes| {
                serde_json::from_slice::<PersistedCheckpointLedger>(&bytes)
                    .expect("test checkpoint sidecar must contain valid checkpoint state")
            })
            .unwrap_or_default();
        for (stream, state) in self
            .states
            .lock()
            .expect("checkpoint mutex poisoned")
            .iter()
        {
            if let Some(existing) = persisted
                .states
                .iter_mut()
                .find(|(candidate, _)| candidate == stream)
            {
                existing.1 = state.clone();
            } else {
                persisted.states.push((stream.clone(), state.clone()));
            }
        }
        for (stream, head) in self.heads.lock().expect("checkpoint mutex poisoned").iter() {
            if let Some(existing) = persisted
                .heads
                .iter_mut()
                .find(|(candidate, _)| candidate == stream)
            {
                existing.1 = head.clone();
            } else {
                persisted.heads.push((stream.clone(), head.clone()));
            }
        }
        for (target, admission) in self
            .targets
            .lock()
            .expect("checkpoint mutex poisoned")
            .iter()
        {
            if let Some(existing) = persisted
                .targets
                .iter_mut()
                .find(|(candidate, _)| candidate == target)
            {
                existing.1 = admission.clone();
            } else {
                persisted.targets.push((target.clone(), admission.clone()));
            }
        }
        persisted.states.sort_by(|left, right| left.0.cmp(&right.0));
        persisted.heads.sort_by(|left, right| left.0.cmp(&right.0));
        persisted
            .targets
            .sort_by(|left, right| left.0.cmp(&right.0));
        let bytes = serde_json::to_vec(&persisted).expect("test checkpoint state is serializable");
        let parent = path.parent().expect("test checkpoint sidecar has a parent");
        fs::create_dir_all(parent).expect("create test checkpoint sidecar directory");
        let temporary = path.with_extension(format!(
            "json.{}.{}.tmp",
            std::process::id(),
            NEXT_PERSIST_TEMP.fetch_add(1, Ordering::Relaxed)
        ));
        fs::write(&temporary, bytes).expect("write test checkpoint sidecar");
        fs::rename(temporary, path).expect("replace test checkpoint sidecar");
    }
}

#[cfg(feature = "test-support")]
fn test_persistence_path(schema_name: &str) -> PathBuf {
    let safe_schema = schema_name
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '_' {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();
    std::env::temp_dir()
        .join("mfm-postgres-checkpoints")
        .join(format!("{safe_schema}.json"))
}

#[cfg(feature = "test-support")]
impl ExternalCheckpointAuthority for ExternalCheckpointLedger {
    fn register_target(
        &self,
        store_scope_id: &StoreScopeId,
        store_epoch: StoreEpoch,
        target_key: &str,
        database_oid: u32,
        schema_name: &str,
        fence_generation: u64,
        release_epoch: u64,
    ) -> Result<(), CheckpointError> {
        let mut targets = self.targets.lock().expect("checkpoint mutex poisoned");
        let key = (store_scope_id.clone(), store_epoch, target_key.to_owned());
        let incoming = TargetAdmission {
            database_oid,
            schema_name: schema_name.to_owned(),
            fence_generation,
            release_epoch,
        };
        let result = match targets.get(&key) {
            Some(current)
                if current.database_oid != incoming.database_oid
                    || current.schema_name != incoming.schema_name
                    || incoming.fence_generation < current.fence_generation
                    || incoming.release_epoch < current.release_epoch =>
            {
                Err(CheckpointError::Conflict)
            }
            Some(current)
                if incoming.fence_generation == current.fence_generation
                    && incoming.release_epoch == current.release_epoch =>
            {
                Ok(())
            }
            Some(_) | None => {
                targets.insert(key, incoming);
                Ok(())
            }
        };
        drop(targets);
        if result.is_ok() {
            self.persist();
        }
        result
    }

    fn validate_target(
        &self,
        store_scope_id: &StoreScopeId,
        store_epoch: StoreEpoch,
        target_key: &str,
        database_oid: u32,
        schema_name: &str,
        fence_generation: u64,
        release_epoch: u64,
    ) -> Result<(), CheckpointError> {
        let targets = self.targets.lock().expect("checkpoint mutex poisoned");
        let Some(current) =
            targets.get(&(store_scope_id.clone(), store_epoch, target_key.to_owned()))
        else {
            return Err(CheckpointError::Conflict);
        };
        if current.database_oid == database_oid
            && current.schema_name == schema_name
            && current.fence_generation == fence_generation
            && current.release_epoch == release_epoch
        {
            Ok(())
        } else {
            Err(CheckpointError::Conflict)
        }
    }

    fn prepare(&self, mutation: &CheckpointMutation) -> Result<CheckpointState, CheckpointError> {
        let _gate = self.gate.lock().expect("checkpoint mutex poisoned");
        let key = &mutation.key;
        self.validate_target(
            &mutation.target.store_scope_id,
            mutation.target.store_epoch,
            &mutation.target.target_key,
            mutation.target.database_oid,
            &mutation.target.schema_name,
            mutation.target.fence_generation,
            mutation.target.release_epoch,
        )?;
        if mutation.target.store_scope_id != key.store_scope_id
            || mutation.target.store_epoch != key.store_epoch
            || mutation.target.target_key != key.target_key
        {
            return Err(CheckpointError::Conflict);
        }
        let successor_bytes = &mutation.successor_bytes;
        let successor = Self::digest(successor_bytes);
        let mut states = self.states.lock().expect("checkpoint mutex poisoned");
        let heads = self.heads.lock().expect("checkpoint mutex poisoned");
        let stream = StreamIdentity::from(key);
        if self
            .read_fixations
            .lock()
            .expect("checkpoint mutex poisoned")
            .contains_key(&stream)
        {
            return Err(CheckpointError::Conflict);
        }
        let current_head = heads.get(&stream);
        match states.get(&stream).cloned() {
            Some(existing) if existing.successor() == &successor => {
                if current_head == key.predecessor.as_ref() || current_head == Some(&successor) {
                    Ok(existing)
                } else {
                    Err(CheckpointError::Conflict)
                }
            }
            Some(CheckpointState::Acknowledged { successor: prior })
                if current_head == Some(&prior) && key.predecessor.as_ref() == Some(&prior) =>
            {
                let state = CheckpointState::Prepared { successor };
                states.insert(stream, state.clone());
                drop(states);
                drop(heads);
                self.persist();
                Ok(state)
            }
            Some(_) => Err(CheckpointError::Conflict),
            None => {
                if current_head != key.predecessor.as_ref() {
                    return Err(CheckpointError::Conflict);
                }
                let state = CheckpointState::Prepared { successor };
                states.insert(stream, state.clone());
                drop(states);
                drop(heads);
                self.persist();
                Ok(state)
            }
        }
    }

    fn acknowledge(&self, mutation: &CheckpointMutation) -> Result<(), CheckpointError> {
        let _gate = self.gate.lock().expect("checkpoint mutex poisoned");
        let key = &mutation.key;
        self.validate_target(
            &mutation.target.store_scope_id,
            mutation.target.store_epoch,
            &mutation.target.target_key,
            mutation.target.database_oid,
            &mutation.target.schema_name,
            mutation.target.fence_generation,
            mutation.target.release_epoch,
        )?;
        if mutation.target.store_scope_id != key.store_scope_id
            || mutation.target.store_epoch != key.store_epoch
            || mutation.target.target_key != key.target_key
        {
            return Err(CheckpointError::Conflict);
        }
        let successor_bytes = &mutation.successor_bytes;
        let successor = Self::digest(successor_bytes);
        let mut states = self.states.lock().expect("checkpoint mutex poisoned");
        let mut heads = self.heads.lock().expect("checkpoint mutex poisoned");
        let stream = StreamIdentity::from(key);
        if self
            .read_fixations
            .lock()
            .expect("checkpoint mutex poisoned")
            .contains_key(&stream)
        {
            return Err(CheckpointError::Conflict);
        }
        let current_head = heads.get(&stream).cloned();
        match states.get(&stream).cloned() {
            Some(CheckpointState::Prepared {
                successor: expected,
            }) if expected == successor => {
                if current_head != key.predecessor && current_head != Some(successor.clone()) {
                    return Err(CheckpointError::Conflict);
                }
                states.insert(
                    stream.clone(),
                    CheckpointState::Acknowledged {
                        successor: successor.clone(),
                    },
                );
                heads.insert(stream, successor);
                drop(states);
                drop(heads);
                self.persist();
                Ok(())
            }
            Some(CheckpointState::Acknowledged {
                successor: expected,
            }) if expected == successor => {
                if current_head == Some(successor) {
                    Ok(())
                } else {
                    Err(CheckpointError::Conflict)
                }
            }
            _ => Err(CheckpointError::NotPrepared),
        }
    }

    fn fixate_read(&self, key: &CheckpointKey) -> Result<CheckpointReadFixation, CheckpointError> {
        let _gate = self.gate.lock().expect("checkpoint mutex poisoned");
        let targets = self.targets.lock().expect("checkpoint mutex poisoned");
        if !targets.contains_key(&(
            key.store_scope_id.clone(),
            key.store_epoch,
            key.target_key.clone(),
        )) {
            return Err(CheckpointError::Conflict);
        }
        let stream = StreamIdentity::from(key);
        let successor = self.current_head(key);
        let mut fixations = self
            .read_fixations
            .lock()
            .expect("checkpoint mutex poisoned");
        let fixation = CheckpointReadFixation::issued(
            key.store_scope_id.clone(),
            key.store_epoch,
            key.target_key.clone(),
            key.stream,
            key.stream_id.clone(),
            successor,
        );
        fixations.entry(stream).or_default().insert(fixation.nonce);
        Ok(fixation)
    }

    fn release_read(&self, fixation: &CheckpointReadFixation) -> Result<(), CheckpointError> {
        let _gate = self.gate.lock().expect("checkpoint mutex poisoned");
        let key = CheckpointKey {
            store_scope_id: fixation.store_scope_id.clone(),
            store_epoch: fixation.store_epoch,
            target_key: fixation.target_key.clone(),
            stream: fixation.stream,
            stream_id: fixation.stream_id.clone(),
            predecessor: None,
        };
        let stream = StreamIdentity::from(&key);
        let mut fixations = self
            .read_fixations
            .lock()
            .expect("checkpoint mutex poisoned");
        let Some(nonces) = fixations.get_mut(&stream) else {
            return Err(CheckpointError::Conflict);
        };
        if !nonces.remove(&fixation.nonce) {
            return Err(CheckpointError::Conflict);
        }
        if nonces.is_empty() {
            fixations.remove(&stream);
        }
        Ok(())
    }

    fn state(&self, key: &CheckpointKey) -> Option<CheckpointState> {
        self.states
            .lock()
            .expect("checkpoint mutex poisoned")
            .get(&StreamIdentity::from(key))
            .cloned()
    }

    fn current_head(&self, key: &CheckpointKey) -> Option<ContentDigest> {
        self.heads
            .lock()
            .expect("checkpoint mutex poisoned")
            .get(&StreamIdentity::from(key))
            .cloned()
    }

    fn prepare_many(
        &self,
        mutations: &[CheckpointMutation],
    ) -> Result<Vec<CheckpointState>, CheckpointError> {
        let _gate = self.gate.lock().expect("checkpoint mutex poisoned");
        let mut ordered = mutations.to_vec();
        ordered.sort_by(|left, right| left.key.cmp(&right.key));
        if ordered.windows(2).any(|pair| pair[0].key == pair[1].key) {
            return Err(CheckpointError::Conflict);
        }
        let target_identity = ordered.first().map(|mutation| {
            (
                mutation.target.store_scope_id.clone(),
                mutation.target.store_epoch,
                mutation.target.target_key.clone(),
                mutation.target.database_oid,
                mutation.target.schema_name.clone(),
                mutation.target.fence_generation,
                mutation.target.release_epoch,
            )
        });
        if ordered.iter().skip(1).any(|mutation| {
            target_identity.as_ref()
                != Some(&(
                    mutation.target.store_scope_id.clone(),
                    mutation.target.store_epoch,
                    mutation.target.target_key.clone(),
                    mutation.target.database_oid,
                    mutation.target.schema_name.clone(),
                    mutation.target.fence_generation,
                    mutation.target.release_epoch,
                ))
        }) {
            return Err(CheckpointError::Conflict);
        }
        let mut states = Vec::with_capacity(ordered.len());
        let mut streams = BTreeMap::<StreamIdentity, ()>::new();
        for mutation in &ordered {
            if mutation.target.store_scope_id != mutation.key.store_scope_id
                || mutation.target.store_epoch != mutation.key.store_epoch
                || mutation.target.target_key != mutation.key.target_key
                || self
                    .validate_target(
                        &mutation.target.store_scope_id,
                        mutation.target.store_epoch,
                        &mutation.target.target_key,
                        mutation.target.database_oid,
                        &mutation.target.schema_name,
                        mutation.target.fence_generation,
                        mutation.target.release_epoch,
                    )
                    .is_err()
            {
                return Err(CheckpointError::Conflict);
            }
            if streams
                .insert(StreamIdentity::from(&mutation.key), ())
                .is_some()
            {
                return Err(CheckpointError::Conflict);
            }
            if self
                .read_fixations
                .lock()
                .expect("checkpoint mutex poisoned")
                .contains_key(&StreamIdentity::from(&mutation.key))
            {
                return Err(CheckpointError::Conflict);
            }
        }
        let mut guard = self.states.lock().expect("checkpoint mutex poisoned");
        let heads = self.heads.lock().expect("checkpoint mutex poisoned");
        // Validate the complete set while holding both authority locks before
        // inserting any Prepared state. A later conflict must not leave a
        // partially prepared multi-key append behind.
        for mutation in &ordered {
            let successor = Self::digest(&mutation.successor_bytes);
            if let Some(head) = heads.get(&StreamIdentity::from(&mutation.key)) {
                if mutation.key.predecessor.as_ref() != Some(head) {
                    return Err(CheckpointError::Conflict);
                }
            } else if mutation.key.predecessor.is_some() {
                return Err(CheckpointError::Conflict);
            }
            let stream = StreamIdentity::from(&mutation.key);
            let current_head = heads.get(&stream);
            match guard.get(&stream).cloned() {
                Some(existing) if existing.successor() == &successor => {
                    if current_head == mutation.key.predecessor.as_ref()
                        || current_head == Some(&successor)
                    {
                        states.push(existing);
                    } else {
                        return Err(CheckpointError::Conflict);
                    }
                }
                Some(CheckpointState::Acknowledged { successor: prior })
                    if current_head == Some(&prior)
                        && mutation.key.predecessor.as_ref() == Some(&prior) =>
                {
                    states.push(CheckpointState::Prepared { successor });
                }
                Some(_) => return Err(CheckpointError::Conflict),
                None => {
                    if current_head != mutation.key.predecessor.as_ref() {
                        return Err(CheckpointError::Conflict);
                    }
                    states.push(CheckpointState::Prepared { successor });
                }
            }
        }
        for (mutation, state) in ordered.iter().zip(states.iter()) {
            guard.insert(StreamIdentity::from(&mutation.key), state.clone());
        }
        drop(guard);
        drop(heads);
        self.persist();
        Ok(states)
    }

    fn acknowledge_many(&self, mutations: &[CheckpointMutation]) -> Result<(), CheckpointError> {
        let _gate = self.gate.lock().expect("checkpoint mutex poisoned");
        let mut ordered = mutations.to_vec();
        ordered.sort_by(|left, right| left.key.cmp(&right.key));
        let target_identity = ordered.first().map(|mutation| {
            (
                mutation.target.store_scope_id.clone(),
                mutation.target.store_epoch,
                mutation.target.target_key.clone(),
                mutation.target.database_oid,
                mutation.target.schema_name.clone(),
                mutation.target.fence_generation,
                mutation.target.release_epoch,
            )
        });
        if ordered.iter().skip(1).any(|mutation| {
            target_identity.as_ref()
                != Some(&(
                    mutation.target.store_scope_id.clone(),
                    mutation.target.store_epoch,
                    mutation.target.target_key.clone(),
                    mutation.target.database_oid,
                    mutation.target.schema_name.clone(),
                    mutation.target.fence_generation,
                    mutation.target.release_epoch,
                ))
        }) {
            return Err(CheckpointError::Conflict);
        }
        for mutation in &ordered {
            if mutation.target.store_scope_id != mutation.key.store_scope_id
                || mutation.target.store_epoch != mutation.key.store_epoch
                || mutation.target.target_key != mutation.key.target_key
                || self
                    .validate_target(
                        &mutation.target.store_scope_id,
                        mutation.target.store_epoch,
                        &mutation.target.target_key,
                        mutation.target.database_oid,
                        &mutation.target.schema_name,
                        mutation.target.fence_generation,
                        mutation.target.release_epoch,
                    )
                    .is_err()
            {
                return Err(CheckpointError::Conflict);
            }
        }
        let mut guard = self.states.lock().expect("checkpoint mutex poisoned");
        let mut heads = self.heads.lock().expect("checkpoint mutex poisoned");
        let mut streams = BTreeMap::<StreamIdentity, ()>::new();
        for mutation in &ordered {
            let successor = Self::digest(&mutation.successor_bytes);
            let stream = StreamIdentity::from(&mutation.key);
            let current_head = heads.get(&stream);
            if streams.insert(stream.clone(), ()).is_some() {
                return Err(CheckpointError::Conflict);
            }
            if self
                .read_fixations
                .lock()
                .expect("checkpoint mutex poisoned")
                .contains_key(&stream)
            {
                return Err(CheckpointError::Conflict);
            }
            match guard.get(&stream) {
                Some(CheckpointState::Prepared {
                    successor: expected,
                }) if *expected == successor => {
                    if current_head != mutation.key.predecessor.as_ref()
                        && current_head != Some(&successor)
                    {
                        return Err(CheckpointError::Conflict);
                    }
                }
                Some(CheckpointState::Acknowledged {
                    successor: expected,
                }) if *expected == successor => {
                    if current_head != Some(&successor) {
                        return Err(CheckpointError::Conflict);
                    }
                }
                _ => return Err(CheckpointError::NotPrepared),
            }
        }
        for mutation in ordered {
            let successor = Self::digest(&mutation.successor_bytes);
            heads.insert(StreamIdentity::from(&mutation.key), successor.clone());
            guard.insert(
                StreamIdentity::from(&mutation.key),
                CheckpointState::Acknowledged { successor },
            );
        }
        drop(guard);
        drop(heads);
        self.persist();
        Ok(())
    }
}

#[cfg(all(test, feature = "test-support"))]
fn checkpoint_digest(bytes: &[u8]) -> ContentDigest {
    ContentDigest::from_digest(DigestAlgorithm::Sha256V1, sha256_digest_bytes(bytes))
}

#[cfg(all(test, feature = "test-support"))]
mod tests {
    use super::*;

    fn scope() -> StoreScopeId {
        StoreScopeId::new("mfm.store_scope.v1:0123456789abcdef0123456789abcdef").expect("scope")
    }

    fn other_scope() -> StoreScopeId {
        StoreScopeId::new("mfm.store_scope.v1:fedcba9876543210fedcba9876543210").expect("scope")
    }

    fn target() -> CheckpointTarget {
        CheckpointTarget {
            store_scope_id: scope(),
            store_epoch: StoreEpoch::new(1),
            target_key: "target-a".to_owned(),
            database_oid: 42,
            schema_name: "mfm_test".to_owned(),
            fence_generation: 7,
            release_epoch: 3,
        }
    }

    fn key(predecessor: Option<ContentDigest>) -> CheckpointKey {
        CheckpointKey {
            store_scope_id: scope(),
            store_epoch: StoreEpoch::new(1),
            target_key: "target-a".to_owned(),
            stream: CheckpointStream::Run,
            stream_id: "run-a".to_owned(),
            predecessor,
        }
    }

    fn mutation(predecessor: Option<ContentDigest>, bytes: &[u8]) -> CheckpointMutation {
        CheckpointMutation {
            key: key(predecessor),
            target: target(),
            successor_bytes: bytes.to_vec(),
        }
    }

    fn registered() -> ExternalCheckpointLedger {
        let ledger = ExternalCheckpointLedger::default();
        let target = target();
        ledger
            .register_target(
                &target.store_scope_id,
                target.store_epoch,
                &target.target_key,
                target.database_oid,
                &target.schema_name,
                target.fence_generation,
                target.release_epoch,
            )
            .expect("register target");
        ledger
    }

    fn persisted_registered(label: &str) -> (String, ExternalCheckpointLedger) {
        let schema_name = format!(
            "checkpoint_{label}_{}_{}",
            std::process::id(),
            NEXT_PERSIST_TEMP.fetch_add(1, Ordering::Relaxed)
        );
        let ledger = ExternalCheckpointLedger::for_test(&schema_name);
        let target = target();
        ledger
            .register_target(
                &target.store_scope_id,
                target.store_epoch,
                &target.target_key,
                target.database_oid,
                &target.schema_name,
                target.fence_generation,
                target.release_epoch,
            )
            .expect("register persisted target");
        (schema_name, ledger)
    }

    #[test]
    fn sequential_successors_replace_the_stable_state_slot() {
        let ledger = registered();
        let first = mutation(None, b"first");
        ledger.prepare(&first).expect("prepare first");
        ledger.acknowledge(&first).expect("ack first");
        let first_head = checkpoint_digest(b"first");

        let second = mutation(Some(first_head.clone()), b"second");
        ledger.prepare(&second).expect("prepare second");
        ledger.acknowledge(&second).expect("ack second");
        assert_eq!(
            ledger.current_head(&key(None)),
            Some(checkpoint_digest(b"second"))
        );
        assert_eq!(
            ledger.state(&key(None)).unwrap().successor(),
            &checkpoint_digest(b"second")
        );
    }

    #[test]
    fn prepared_sql_successor_can_be_acknowledged_after_restart() {
        let (schema_name, ledger) = persisted_registered("single");
        let first = mutation(None, b"first");
        ledger.prepare(&first).expect("prepare first");
        let reopened = ExternalCheckpointLedger::for_test(&schema_name);
        assert_eq!(
            reopened.state(&first.key),
            Some(CheckpointState::Prepared {
                successor: checkpoint_digest(b"first"),
            })
        );
        let conflicting = mutation(None, b"different");
        assert_eq!(
            reopened.prepare(&conflicting),
            Err(CheckpointError::Conflict)
        );
        reopened
            .acknowledge(&first)
            .expect("ack prepared successor after restart");
        reopened
            .acknowledge(&first)
            .expect("idempotent acknowledgement");
        assert_eq!(
            reopened.current_head(&key(None)),
            Some(checkpoint_digest(b"first"))
        );
        drop(reopened);
        drop(ledger);
        let _ = fs::remove_file(test_persistence_path(&schema_name));
    }

    #[test]
    fn prepare_many_replaces_acknowledged_slot_with_prepared_successor() {
        let ledger = registered();
        let first = mutation(None, b"first");
        ledger
            .prepare_many(std::slice::from_ref(&first))
            .expect("prepare first");
        ledger
            .acknowledge_many(std::slice::from_ref(&first))
            .expect("acknowledge first");
        let second = mutation(Some(checkpoint_digest(b"first")), b"second");
        let states = ledger
            .prepare_many(std::slice::from_ref(&second))
            .expect("prepare second");
        assert_eq!(
            states,
            vec![CheckpointState::Prepared {
                successor: checkpoint_digest(b"second"),
            }]
        );
        assert_eq!(ledger.state(&second.key), Some(states[0].clone()));
        ledger
            .acknowledge_many(std::slice::from_ref(&second))
            .expect("acknowledge prepared successor");
        assert_eq!(
            ledger.current_head(&second.key),
            Some(checkpoint_digest(b"second"))
        );
    }

    #[test]
    fn prepared_multi_key_successor_survives_fresh_ledger_restart() {
        let (schema_name, ledger) = persisted_registered("multi");
        let first = mutation(None, b"first");
        let mut second = mutation(None, b"second");
        second.key.stream_id = "run-b".to_owned();
        ledger
            .prepare_many(&[first.clone(), second.clone()])
            .expect("prepare multi-key successor");

        let reopened = ExternalCheckpointLedger::for_test(&schema_name);
        assert_eq!(
            reopened.state(&first.key),
            Some(CheckpointState::Prepared {
                successor: checkpoint_digest(b"first"),
            })
        );
        assert_eq!(
            reopened.state(&second.key),
            Some(CheckpointState::Prepared {
                successor: checkpoint_digest(b"second"),
            })
        );
        reopened
            .acknowledge_many(&[first.clone(), second.clone()])
            .expect("acknowledge prepared multi-key successor after restart");
        assert_eq!(
            reopened.current_head(&first.key),
            Some(checkpoint_digest(b"first"))
        );
        assert_eq!(
            reopened.current_head(&second.key),
            Some(checkpoint_digest(b"second"))
        );
        drop(reopened);
        drop(ledger);
        let _ = fs::remove_file(test_persistence_path(&schema_name));
    }

    #[test]
    fn acknowledge_many_rejects_unprepared_successors() {
        let ledger = registered();
        let first = mutation(None, b"first");
        assert_eq!(
            ledger.acknowledge_many(std::slice::from_ref(&first)),
            Err(CheckpointError::NotPrepared)
        );
        ledger.prepare(&first).expect("prepare first");
        ledger.acknowledge(&first).expect("ack first");

        let second = mutation(Some(checkpoint_digest(b"first")), b"second");
        assert_eq!(
            ledger.acknowledge_many(std::slice::from_ref(&second)),
            Err(CheckpointError::NotPrepared)
        );
    }

    #[test]
    fn prepare_many_is_all_or_nothing_and_validates_target() {
        let ledger = registered();
        let first = mutation(None, b"first");
        let mut wrong = mutation(None, b"wrong");
        wrong.key.stream_id = "other-run".to_owned();
        wrong.key.predecessor = Some(checkpoint_digest(b"missing"));
        assert_eq!(
            ledger.prepare_many(&[first.clone(), wrong]),
            Err(CheckpointError::Conflict)
        );
        assert!(ledger.state(&first.key).is_none());

        let mut copied = first.clone();
        copied.target.target_key = "target-b".to_owned();
        let fixation = ledger.fixate_read(&copied.key).expect("fixate read");
        assert_eq!(fixation.successor(), None);
        assert_eq!(ledger.prepare(&first), Err(CheckpointError::Conflict));
        ledger.release_read(&fixation).expect("release fixation");
        assert_eq!(ledger.prepare(&copied), Err(CheckpointError::Conflict));
    }

    #[test]
    fn stale_read_fixation_cannot_release_a_new_barrier() {
        let ledger = registered();
        let key = key(None);
        let first = ledger.fixate_read(&key).expect("first fixation");
        ledger.release_read(&first).expect("release first fixation");
        let second = ledger.fixate_read(&key).expect("second fixation");
        assert_eq!(ledger.release_read(&first), Err(CheckpointError::Conflict));
        let forged = CheckpointReadFixation::issued(
            scope(),
            StoreEpoch::new(1),
            "target-a".to_owned(),
            CheckpointStream::Run,
            "run-a".to_owned(),
            None,
        );
        assert_eq!(ledger.release_read(&forged), Err(CheckpointError::Conflict));
        assert_eq!(
            ledger.prepare(&mutation(None, b"blocked")),
            Err(CheckpointError::Conflict)
        );
        ledger
            .release_read(&second)
            .expect("release second fixation");
        ledger
            .prepare(&mutation(None, b"unblocked"))
            .expect("prepare after release");
    }

    #[test]
    fn concurrent_read_fixations_share_the_active_barrier() {
        let ledger = registered();
        let key = key(None);
        let first = ledger.fixate_read(&key).expect("first fixation");
        let second = ledger.fixate_read(&key).expect("second fixation");
        assert_eq!(
            ledger.prepare(&mutation(None, b"blocked")),
            Err(CheckpointError::Conflict)
        );
        ledger.release_read(&first).expect("release first fixation");
        assert_eq!(
            ledger.prepare(&mutation(None, b"still blocked")),
            Err(CheckpointError::Conflict)
        );
        ledger
            .release_read(&second)
            .expect("release second fixation");
        ledger
            .prepare(&mutation(None, b"unblocked"))
            .expect("prepare after all readers release");
    }

    #[test]
    fn single_checkpoint_mutations_bind_target_scope_and_epoch() {
        let ledger = registered();

        let mut wrong_scope = mutation(None, b"wrong-scope");
        wrong_scope.target.store_scope_id = other_scope();
        ledger
            .register_target(
                &wrong_scope.target.store_scope_id,
                wrong_scope.target.store_epoch,
                &wrong_scope.target.target_key,
                wrong_scope.target.database_oid,
                &wrong_scope.target.schema_name,
                wrong_scope.target.fence_generation,
                wrong_scope.target.release_epoch,
            )
            .expect("register alternate scope");
        assert_eq!(ledger.prepare(&wrong_scope), Err(CheckpointError::Conflict));
        assert_eq!(
            ledger.acknowledge(&wrong_scope),
            Err(CheckpointError::Conflict)
        );

        let mut wrong_epoch = mutation(None, b"wrong-epoch");
        wrong_epoch.target.store_epoch = StoreEpoch::new(2);
        ledger
            .register_target(
                &wrong_epoch.target.store_scope_id,
                wrong_epoch.target.store_epoch,
                &wrong_epoch.target.target_key,
                wrong_epoch.target.database_oid,
                &wrong_epoch.target.schema_name,
                wrong_epoch.target.fence_generation,
                wrong_epoch.target.release_epoch,
            )
            .expect("register alternate epoch");
        assert_eq!(ledger.prepare(&wrong_epoch), Err(CheckpointError::Conflict));
        assert_eq!(
            ledger.acknowledge(&wrong_epoch),
            Err(CheckpointError::Conflict)
        );
    }
}
