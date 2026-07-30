#![warn(missing_docs)]
//! Fsync-qualified executor conformance storage.
//!
//! Immutable checksummed snapshots are published under one cross-process file
//! lock. This crate exercises crash/ack ambiguity and strict refold behavior;
//! it is not production HA storage and does not claim backup anti-rollback,
//! sibling-writer fencing, or destination-owner coverage.

use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};
use std::sync::Arc;

use fs2::FileExt;
use mfm_executor::{
    ContentRef, EffectKey, ExecutorAppendOutcome, ExecutorEffectSnapshot, ExecutorError,
    ExecutorFuture, ExecutorLedgerAppend, ExecutorLedgerStore, ExecutorLedgerStoreIdentity,
    ExecutorResourceSnapshot, MemoryConvergentDestination, MemoryDestinationCheckpoint,
    MemoryExecutorStore, MemoryLedgerCheckpoint, ReferenceContract, ReferenceDestination,
    ReferenceDestinationReturn, ReferenceRequest, ReferenceTargetBehavior, ResourceKeyRef,
    ResourceOwnershipRef, Result, SchemaQualifiedCanonicalValue, TargetEntryAuthority,
    VerifiedExecutorBinding,
};

const MAX_SNAPSHOT_BYTES: u64 = 16 * 1024 * 1024;
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// One deterministic conformance fault around snapshot publication.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileFaultPoint {
    /// Fail before the immutable snapshot is linked into the sequence.
    BeforeSnapshotPublish,
    /// Publish durably, then lose the caller acknowledgement.
    AfterSnapshotPublishBeforeAck,
}

impl FileFaultPoint {
    const fn code(self) -> u8 {
        match self {
            Self::BeforeSnapshotPublish => 1,
            Self::AfterSnapshotPublishBeforeAck => 2,
        }
    }
}

/// Fsync-qualified append/CAS ledger conformance backend.
#[derive(Debug, Clone)]
pub struct FileExecutorStore {
    binding: VerifiedExecutorBinding,
    identity: ExecutorLedgerStoreIdentity,
    snapshots: SnapshotDirectory,
}

impl FileExecutorStore {
    /// Opens or creates one conformance raw-store directory.
    pub async fn open(path: impl AsRef<Path>, binding: VerifiedExecutorBinding) -> Result<Self> {
        let path = path.as_ref().to_owned();
        tokio::task::spawn_blocking(move || Self::open_blocking(&path, binding))
            .await
            .map_err(|_| ExecutorError::DurableBackendUnavailable)?
    }

    fn open_blocking(path: &Path, binding: VerifiedExecutorBinding) -> Result<Self> {
        let identity = ExecutorLedgerStoreIdentity::from_binding(&binding);
        let snapshots = SnapshotDirectory::open(path, "ledger")?;
        let store = Self {
            binding,
            identity,
            snapshots,
        };
        store.initialize()?;
        Ok(store)
    }

    /// Injects one fault into the next snapshot-changing operation.
    pub fn inject_next_fault(&self, point: FileFaultPoint) {
        self.snapshots.inject_next_fault(point);
    }

    /// Returns the greatest durable snapshot sequence.
    pub fn snapshot_sequence(&self) -> Result<u64> {
        self.snapshots.with_lock(|directory| {
            directory
                .latest()?
                .map(|snapshot| snapshot.sequence)
                .ok_or(ExecutorError::InvalidDurableSnapshot)
        })
    }

    fn initialize(&self) -> Result<()> {
        self.snapshots.with_lock(|directory| {
            if let Some(snapshot) = directory.latest()? {
                let checkpoint = MemoryLedgerCheckpoint::from_durable_bytes(&snapshot.bytes)?;
                self.restore(checkpoint)?;
                return Ok(());
            }
            let checkpoint = MemoryExecutorStore::new(&self.binding).checkpoint()?;
            directory.publish(&checkpoint.to_durable_bytes()?)?;
            Ok(())
        })
    }

    fn transaction<Output>(
        &self,
        operation: impl FnOnce(&MemoryExecutorStore) -> Result<Output>,
    ) -> Result<Output> {
        self.snapshots.with_lock(|directory| {
            let latest = directory
                .latest()?
                .ok_or(ExecutorError::InvalidDurableSnapshot)?;
            let checkpoint = MemoryLedgerCheckpoint::from_durable_bytes(&latest.bytes)?;
            let ledger = self.restore(checkpoint)?;
            let output = operation(&ledger)?;
            let after = ledger.checkpoint()?.to_durable_bytes()?;
            if after != latest.bytes {
                directory.publish(&after)?;
            }
            Ok(output)
        })
    }

    fn compare_and_append_transaction(
        &self,
        append: ExecutorLedgerAppend,
    ) -> Result<ExecutorAppendOutcome> {
        self.snapshots.with_lock(|directory| {
            let latest = directory
                .latest()?
                .ok_or(ExecutorError::InvalidDurableSnapshot)?;
            let checkpoint = MemoryLedgerCheckpoint::from_durable_bytes(&latest.bytes)?;
            let store = self.restore(checkpoint)?;
            let outcome = store.compare_and_append_snapshot(append)?;
            if outcome != ExecutorAppendOutcome::Applied {
                return Ok(outcome);
            }
            let after = store.checkpoint()?.to_durable_bytes()?;
            match directory.publish_classified(&after) {
                Ok(_) => Ok(ExecutorAppendOutcome::Applied),
                Err(SnapshotPublishError::Definite(error)) => Err(error),
                Err(SnapshotPublishError::OutcomeUnknown) => {
                    Ok(ExecutorAppendOutcome::OutcomeUnknown)
                }
            }
        })
    }

    fn restore(&self, checkpoint: MemoryLedgerCheckpoint) -> Result<MemoryExecutorStore> {
        MemoryExecutorStore::restore(&self.binding, checkpoint)
    }
}

impl ExecutorLedgerStore for FileExecutorStore {
    fn store_identity(&self) -> &ExecutorLedgerStoreIdentity {
        &self.identity
    }

    fn load_effect<'a>(
        &'a self,
        effect_key: &'a EffectKey,
    ) -> ExecutorFuture<'a, Result<Option<ExecutorEffectSnapshot>>> {
        let store = self.clone();
        let effect_key = effect_key.clone();
        Box::pin(async move {
            tokio::task::spawn_blocking(move || {
                store.transaction(|memory| memory.load_effect_snapshot(&effect_key))
            })
            .await
            .map_err(|_| ExecutorError::DurableBackendUnavailable)?
        })
    }

    fn load_resource<'a>(
        &'a self,
        resource_ownership_ref: &'a ResourceOwnershipRef,
        resource_key_ref: &'a ResourceKeyRef,
    ) -> ExecutorFuture<'a, Result<ExecutorResourceSnapshot>> {
        let store = self.clone();
        let resource_ownership_ref = resource_ownership_ref.clone();
        let resource_key_ref = resource_key_ref.clone();
        Box::pin(async move {
            tokio::task::spawn_blocking(move || {
                store.transaction(|memory| {
                    memory.load_resource_snapshot(&resource_ownership_ref, &resource_key_ref)
                })
            })
            .await
            .map_err(|_| ExecutorError::DurableBackendUnavailable)?
        })
    }

    fn load_content<'a>(
        &'a self,
        content_ref: &'a ContentRef,
    ) -> ExecutorFuture<'a, Result<Option<SchemaQualifiedCanonicalValue>>> {
        let store = self.clone();
        let content_ref = content_ref.clone();
        Box::pin(async move {
            tokio::task::spawn_blocking(move || {
                store.transaction(|memory| memory.load_content_snapshot(&content_ref))
            })
            .await
            .map_err(|_| ExecutorError::DurableBackendUnavailable)?
        })
    }

    fn compare_and_append<'a>(
        &'a self,
        append: ExecutorLedgerAppend,
    ) -> ExecutorFuture<'a, Result<ExecutorAppendOutcome>> {
        let store = self.clone();
        Box::pin(async move {
            tokio::task::spawn_blocking(move || store.compare_and_append_transaction(append))
                .await
                .map_err(|_| ExecutorError::DurableBackendUnavailable)?
        })
    }
}

/// Fsync-qualified convergence-safe queue conformance destination.
#[derive(Debug, Clone)]
pub struct FileConvergentDestination {
    snapshots: SnapshotDirectory,
}

impl FileConvergentDestination {
    /// Opens or creates one conformance destination directory.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let destination = Self {
            snapshots: SnapshotDirectory::open(path.as_ref(), "destination")?,
        };
        destination.initialize()?;
        Ok(destination)
    }

    /// Injects one fault into the next snapshot-changing operation.
    pub fn inject_next_fault(&self, point: FileFaultPoint) {
        self.snapshots.inject_next_fault(point);
    }

    /// Admits the first durable executor generation.
    pub fn activate_generation(&self, generation_ref: ContentRef) -> Result<()> {
        self.transaction(|destination| destination.activate_generation(generation_ref))
    }

    /// Permanently fences one active generation and selects its successor.
    pub fn fence_and_activate(
        &self,
        expected_generation_ref: &ContentRef,
        replacement_generation_ref: ContentRef,
    ) -> Result<()> {
        self.transaction(|destination| {
            destination.fence_and_activate(expected_generation_ref, replacement_generation_ref)
        })
    }

    /// Returns the durable count of actual target entries.
    pub fn target_entry_count(&self) -> Result<u64> {
        self.transaction(|destination| destination.target_entry_count())
    }

    /// Returns the durable count of distinct semantic queue mutations.
    pub fn semantic_mutation_count(&self) -> Result<u64> {
        self.transaction(|destination| destination.semantic_mutation_count())
    }

    /// Returns the greatest durable snapshot sequence.
    pub fn snapshot_sequence(&self) -> Result<u64> {
        self.snapshots.with_lock(|directory| {
            directory
                .latest()?
                .map(|snapshot| snapshot.sequence)
                .ok_or(ExecutorError::InvalidDurableSnapshot)
        })
    }

    fn initialize(&self) -> Result<()> {
        self.snapshots.with_lock(|directory| {
            if let Some(snapshot) = directory.latest()? {
                let checkpoint = MemoryDestinationCheckpoint::from_durable_bytes(&snapshot.bytes)?;
                let _ = MemoryConvergentDestination::restore(checkpoint);
                return Ok(());
            }
            let checkpoint = MemoryConvergentDestination::new().checkpoint()?;
            directory.publish(&checkpoint.to_durable_bytes()?)?;
            Ok(())
        })
    }

    fn transaction<Output>(
        &self,
        operation: impl FnOnce(&MemoryConvergentDestination) -> Result<Output>,
    ) -> Result<Output> {
        self.snapshots.with_lock(|directory| {
            let latest = directory
                .latest()?
                .ok_or(ExecutorError::InvalidDurableSnapshot)?;
            let checkpoint = MemoryDestinationCheckpoint::from_durable_bytes(&latest.bytes)?;
            let destination = MemoryConvergentDestination::restore(checkpoint);
            let output = operation(&destination)?;
            let after = destination.checkpoint()?.to_durable_bytes()?;
            if after != latest.bytes {
                directory.publish(&after)?;
            }
            Ok(output)
        })
    }
}

impl ReferenceDestination for FileConvergentDestination {
    fn enqueue<'a>(
        &'a self,
        authority: TargetEntryAuthority,
        request: &'a ReferenceRequest,
        contract: &'a ReferenceContract,
        behavior: ReferenceTargetBehavior,
    ) -> ExecutorFuture<'a, ReferenceDestinationReturn> {
        let destination = self.clone();
        let request = request.clone();
        let contract = contract.clone();
        Box::pin(async move {
            let fallback_contract = contract.clone();
            let lock_failure_contract = contract.clone();
            let join_failure_contract = contract.clone();
            let returned = tokio::task::spawn_blocking(move || {
                destination.snapshots.with_lock_input(
                    authority,
                    |directory, authority| {
                        let latest = match directory.latest() {
                            Ok(Some(latest)) => latest,
                            Ok(None) | Err(_) => {
                                return fallback_contract.adapter_failure_return();
                            }
                        };
                        let checkpoint =
                            match MemoryDestinationCheckpoint::from_durable_bytes(&latest.bytes) {
                                Ok(checkpoint) => checkpoint,
                                Err(_) => {
                                    return fallback_contract.adapter_failure_return();
                                }
                            };
                        let memory = MemoryConvergentDestination::restore(checkpoint);
                        memory.enter_with_finalizer(
                            authority,
                            &request,
                            &contract,
                            behavior,
                            |memory| {
                                let after = memory.checkpoint()?.to_durable_bytes()?;
                                if after != latest.bytes {
                                    directory.publish(&after)?;
                                }
                                Ok(())
                            },
                        )
                    },
                    |_authority| lock_failure_contract.adapter_failure_return(),
                )
            })
            .await;
            match returned {
                Ok(returned) => returned,
                // The engine retains the private completion seal, so a
                // wrapper failure can return only an unbound adapter outcome.
                Err(_) => join_failure_contract.adapter_failure_return(),
            }
        })
    }
}

#[derive(Debug, Clone)]
struct SnapshotDirectory {
    path: PathBuf,
    prefix: &'static str,
    fault: Arc<AtomicU8>,
}

impl SnapshotDirectory {
    fn open(path: &Path, prefix: &'static str) -> Result<Self> {
        match fs::symlink_metadata(path) {
            Ok(metadata) if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() => {
            }
            Ok(_) => {
                return Err(ExecutorError::DurableBackendUnavailable);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir(path).map_err(|_| ExecutorError::DurableBackendUnavailable)?;
                sync_parent(path)?;
                sync_directory(path)?;
            }
            Err(_) => {
                return Err(ExecutorError::DurableBackendUnavailable);
            }
        }
        Ok(Self {
            path: path.to_owned(),
            prefix,
            fault: Arc::new(AtomicU8::new(0)),
        })
    }

    fn inject_next_fault(&self, point: FileFaultPoint) {
        self.fault.store(point.code(), Ordering::SeqCst);
    }

    fn take_fault(&self, point: FileFaultPoint) -> bool {
        self.fault
            .compare_exchange(point.code(), 0, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
    }

    fn with_lock<Output>(
        &self,
        operation: impl FnOnce(&LockedSnapshotDirectory<'_>) -> Result<Output>,
    ) -> Result<Output> {
        let _lock_file = self.lock_file()?;
        operation(&LockedSnapshotDirectory { owner: self })
    }

    fn with_lock_input<Input, Output>(
        &self,
        input: Input,
        operation: impl FnOnce(&LockedSnapshotDirectory<'_>, Input) -> Output,
        lock_failure: impl FnOnce(Input) -> Output,
    ) -> Output {
        let _lock_file = match self.lock_file() {
            Ok(lock_file) => lock_file,
            Err(_) => return lock_failure(input),
        };
        operation(&LockedSnapshotDirectory { owner: self }, input)
    }

    fn lock_file(&self) -> Result<File> {
        let lock_path = self.path.join(format!("{}.lock", self.prefix));
        reject_symlink_or_nonregular_if_present(&lock_path)?;
        let lock_file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&lock_path)
            .map_err(|_| ExecutorError::DurableBackendUnavailable)?;
        reject_symlink_or_nonregular(&lock_path)?;
        if !lock_file
            .metadata()
            .map_err(|_| ExecutorError::DurableBackendUnavailable)?
            .is_file()
        {
            return Err(ExecutorError::DurableBackendUnavailable);
        }
        lock_file
            .lock_exclusive()
            .map_err(|_| ExecutorError::DurableBackendUnavailable)?;
        Ok(lock_file)
    }
}

struct LockedSnapshotDirectory<'a> {
    owner: &'a SnapshotDirectory,
}

enum SnapshotPublishError {
    Definite(ExecutorError),
    OutcomeUnknown,
}

impl LockedSnapshotDirectory<'_> {
    fn latest(&self) -> Result<Option<Snapshot>> {
        let mut sequences = Vec::new();
        let entries =
            fs::read_dir(&self.owner.path).map_err(|_| ExecutorError::DurableBackendUnavailable)?;
        for entry in entries {
            let entry = entry.map_err(|_| ExecutorError::DurableBackendUnavailable)?;
            let Some(sequence) = snapshot_sequence(&entry.file_name(), self.owner.prefix) else {
                continue;
            };
            let file_type = entry
                .file_type()
                .map_err(|_| ExecutorError::DurableBackendUnavailable)?;
            if !file_type.is_file() || file_type.is_symlink() {
                return Err(ExecutorError::InvalidDurableSnapshot);
            }
            let metadata = entry
                .metadata()
                .map_err(|_| ExecutorError::DurableBackendUnavailable)?;
            if !metadata.is_file() || metadata.len() > MAX_SNAPSHOT_BYTES {
                return Err(ExecutorError::InvalidDurableSnapshot);
            }
            sequences.push((sequence, entry.path()));
        }
        sequences.sort_by_key(|(sequence, _)| *sequence);
        for (expected, (actual, _)) in sequences.iter().enumerate() {
            if *actual
                != u64::try_from(expected).map_err(|_| ExecutorError::InvalidDurableSnapshot)?
            {
                return Err(ExecutorError::InvalidDurableSnapshot);
            }
        }
        let Some((sequence, path)) = sequences.last() else {
            return Ok(None);
        };
        reject_symlink_or_nonregular(path).map_err(|_| ExecutorError::InvalidDurableSnapshot)?;
        let file = File::open(path).map_err(|_| ExecutorError::DurableBackendUnavailable)?;
        if !file
            .metadata()
            .map_err(|_| ExecutorError::DurableBackendUnavailable)?
            .is_file()
        {
            return Err(ExecutorError::InvalidDurableSnapshot);
        }
        let mut bytes = Vec::new();
        file.take(MAX_SNAPSHOT_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| ExecutorError::DurableBackendUnavailable)?;
        if u64::try_from(bytes.len()).map_err(|_| ExecutorError::InvalidDurableSnapshot)?
            > MAX_SNAPSHOT_BYTES
        {
            return Err(ExecutorError::InvalidDurableSnapshot);
        }
        Ok(Some(Snapshot {
            sequence: *sequence,
            bytes,
        }))
    }

    fn publish(&self, bytes: &[u8]) -> Result<u64> {
        self.publish_classified(bytes).map_err(|error| match error {
            SnapshotPublishError::Definite(error) => error,
            SnapshotPublishError::OutcomeUnknown => ExecutorError::DurableBackendUnavailable,
        })
    }

    fn publish_classified(&self, bytes: &[u8]) -> std::result::Result<u64, SnapshotPublishError> {
        if u64::try_from(bytes.len())
            .map_err(|_| SnapshotPublishError::Definite(ExecutorError::InvalidDurableSnapshot))?
            > MAX_SNAPSHOT_BYTES
        {
            return Err(SnapshotPublishError::Definite(
                ExecutorError::InvalidDurableSnapshot,
            ));
        }
        if self.owner.take_fault(FileFaultPoint::BeforeSnapshotPublish) {
            return Err(SnapshotPublishError::Definite(
                ExecutorError::DurableBackendUnavailable,
            ));
        }
        let next_sequence = self
            .latest()
            .map_err(SnapshotPublishError::Definite)?
            .map_or(Ok(0_u64), |latest| {
                latest
                    .sequence
                    .checked_add(1)
                    .ok_or(ExecutorError::InvalidDurableSnapshot)
            })
            .map_err(SnapshotPublishError::Definite)?;
        let final_path = self
            .owner
            .path
            .join(format!("{}-{next_sequence:020}.snap", self.owner.prefix));
        let mut opened_temp = None;
        for _ in 0..128 {
            let temp_sequence = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
            let temp_path = self.owner.path.join(format!(
                ".{}-{}-{temp_sequence}.tmp",
                self.owner.prefix,
                std::process::id()
            ));
            match OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&temp_path)
            {
                Ok(file) => {
                    opened_temp = Some((temp_path, file));
                    break;
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(_) => {
                    return Err(SnapshotPublishError::Definite(
                        ExecutorError::DurableBackendUnavailable,
                    ));
                }
            }
        }
        let (temp_path, mut temp) = opened_temp.ok_or(SnapshotPublishError::Definite(
            ExecutorError::DurableBackendUnavailable,
        ))?;
        temp.write_all(bytes).map_err(|_| {
            SnapshotPublishError::Definite(ExecutorError::DurableBackendUnavailable)
        })?;
        temp.sync_all().map_err(|_| {
            SnapshotPublishError::Definite(ExecutorError::DurableBackendUnavailable)
        })?;
        reject_symlink_or_nonregular(&temp_path).map_err(SnapshotPublishError::Definite)?;
        fs::hard_link(&temp_path, &final_path).map_err(|_| {
            SnapshotPublishError::Definite(ExecutorError::DurableBackendUnavailable)
        })?;
        sync_directory(&self.owner.path).map_err(|_| SnapshotPublishError::OutcomeUnknown)?;
        fs::remove_file(&temp_path).map_err(|_| SnapshotPublishError::OutcomeUnknown)?;
        sync_directory(&self.owner.path).map_err(|_| SnapshotPublishError::OutcomeUnknown)?;
        if self
            .owner
            .take_fault(FileFaultPoint::AfterSnapshotPublishBeforeAck)
        {
            return Err(SnapshotPublishError::OutcomeUnknown);
        }
        Ok(next_sequence)
    }
}

struct Snapshot {
    sequence: u64,
    bytes: Vec<u8>,
}

fn snapshot_sequence(file_name: &OsStr, prefix: &str) -> Option<u64> {
    let name = file_name.to_str()?;
    let sequence = name
        .strip_prefix(prefix)?
        .strip_prefix('-')?
        .strip_suffix(".snap")?;
    if sequence.len() != 20 || !sequence.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    sequence.parse().ok()
}

fn reject_symlink_or_nonregular_if_present(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_file() && !metadata.file_type().is_symlink() => {
            Ok(())
        }
        Ok(_) => Err(ExecutorError::DurableBackendUnavailable),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(ExecutorError::DurableBackendUnavailable),
    }
}

fn reject_symlink_or_nonregular(path: &Path) -> Result<()> {
    let metadata =
        fs::symlink_metadata(path).map_err(|_| ExecutorError::DurableBackendUnavailable)?;
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        return Err(ExecutorError::DurableBackendUnavailable);
    }
    Ok(())
}

fn sync_parent(path: &Path) -> Result<()> {
    let parent = path
        .parent()
        .ok_or(ExecutorError::DurableBackendUnavailable)?;
    sync_directory(parent)
}

fn sync_directory(path: &Path) -> Result<()> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|_| ExecutorError::DurableBackendUnavailable)
}
