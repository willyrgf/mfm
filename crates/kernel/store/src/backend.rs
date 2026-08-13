//! Branded mechanical Store/backend boundary.
//!
//! This module contains only physical backend contracts and raw bounded rows.  It never parses a
//! Program, reduces a run, or constructs a Runtime owner.  `OpenedStructuredStore` is the sole
//! semantic bridge: it qualifies raw rows through the Store reducer before exposing them.

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use mfm_ids::{
    AppendRequestId, ContentDigest, ContentRef, RunId, StoreEpoch, StoreScopeId, TenantScopeId,
};
use mfm_program::ProgramCatalog;

use crate::single_trust::{
    advance_reduced, prepare_access_from_current, prepare_conclusion_from_current,
    reduce_qualified, AppendDisposition, ConfigurationAppendDisposition, ConfigurationRevision,
    ConfigurationSnapshot, QualifiedRun, Result, StoreError,
};

/// Bounded asynchronous backend result used by every mechanical Store capability.
pub type BackendFuture<'a, T> = Pin<Box<dyn Future<Output = BackendResult<T>> + Send + 'a>>;

/// Redaction-safe physical backend failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum BackendError {
    /// The backend belongs to another Store scope, epoch, or tenant.
    #[error("backend identity is invalid")]
    Identity,
    /// The physical append or configuration identity conflicts.
    #[error("backend append conflicts")]
    Conflict,
    /// The expected physical head is no longer current.
    #[error("backend head is stale")]
    StaleHead,
    /// A physical bound or query result limit was exceeded.
    #[error("backend capacity bound exceeded")]
    Capacity,
    /// The transaction outcome is unknown after submission.
    #[error("backend acknowledgement is unknown")]
    AcknowledgementUnknown,
    /// The backend cannot provide the requested mechanical capability.
    #[error("backend capability is unavailable")]
    Unsupported,
    /// The backend failed without exposing driver details.
    #[error("backend operation failed")]
    Storage,
}

/// Result returned by one mechanical backend call.
pub type BackendResult<T> = std::result::Result<T, BackendError>;

/// Immutable physical identity admitted by one Store open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructuredStoreIdentity {
    scope: StoreScopeId,
    epoch: StoreEpoch,
    tenant: TenantScopeId,
}

impl StructuredStoreIdentity {
    /// Creates one fixed Store identity.
    pub const fn new(scope: StoreScopeId, epoch: StoreEpoch, tenant: TenantScopeId) -> Self {
        Self {
            scope,
            epoch,
            tenant,
        }
    }

    /// Returns the Store scope.
    pub const fn scope(&self) -> &StoreScopeId {
        &self.scope
    }

    /// Returns the writer epoch.
    pub const fn epoch(&self) -> StoreEpoch {
        self.epoch
    }

    /// Returns the fixed tenant partition.
    pub const fn tenant(&self) -> &TenantScopeId {
        &self.tenant
    }
}

/// Bounded work envelope applied before raw backend allocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StoreWorkLimits {
    max_frame_bytes: usize,
    max_run_frames: usize,
    max_run_objects: usize,
    max_run_frame_bytes: usize,
}

impl StoreWorkLimits {
    /// Creates one bounded work envelope.
    pub const fn new(
        max_frame_bytes: usize,
        max_run_frames: usize,
        max_run_objects: usize,
        max_run_frame_bytes: usize,
    ) -> Self {
        Self {
            max_frame_bytes,
            max_run_frames,
            max_run_objects,
            max_run_frame_bytes,
        }
    }

    /// Returns the default RFC envelope.
    pub const fn default_envelope() -> Self {
        Self::new(
            mfm_journal::single_trust::MAX_FRAME_BYTES,
            mfm_journal::single_trust::MAX_RUN_FRAMES,
            mfm_journal::single_trust::MAX_RUN_OBJECTS,
            mfm_journal::single_trust::MAX_RUN_FRAME_BYTES,
        )
    }

    /// Validates that the requested envelope cannot exceed the protocol ceiling.
    pub fn validate(&self) -> BackendResult<()> {
        if self.max_frame_bytes == 0
            || self.max_frame_bytes > mfm_journal::single_trust::MAX_FRAME_BYTES
            || self.max_run_frames == 0
            || self.max_run_frames > mfm_journal::single_trust::MAX_RUN_FRAMES
            || self.max_run_objects == 0
            || self.max_run_objects > mfm_journal::single_trust::MAX_RUN_OBJECTS
            || self.max_run_frame_bytes == 0
            || self.max_run_frame_bytes > mfm_journal::single_trust::MAX_RUN_FRAME_BYTES
        {
            return Err(BackendError::Capacity);
        }
        Ok(())
    }

    /// Returns the maximum frame bytes.
    pub const fn max_frame_bytes(&self) -> usize {
        self.max_frame_bytes
    }

    /// Returns the maximum frame count.
    pub const fn max_run_frames(&self) -> usize {
        self.max_run_frames
    }

    /// Returns the maximum reachable object count.
    pub const fn max_run_objects(&self) -> usize {
        self.max_run_objects
    }

    /// Returns the maximum cumulative frame bytes.
    pub const fn max_run_frame_bytes(&self) -> usize {
        self.max_run_frame_bytes
    }
}

impl Default for StoreWorkLimits {
    fn default() -> Self {
        Self::default_envelope()
    }
}

/// Limit supplied to one complete-prefix backend load.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawHistoryLoadLimit {
    max_frames: usize,
    max_bytes: usize,
}

impl RawHistoryLoadLimit {
    /// Creates one bounded raw prefix limit.
    pub const fn new(max_frames: usize, max_bytes: usize) -> Self {
        Self {
            max_frames,
            max_bytes,
        }
    }

    /// Returns the maximum frames.
    pub const fn max_frames(&self) -> usize {
        self.max_frames
    }

    /// Returns the maximum bytes.
    pub const fn max_bytes(&self) -> usize {
        self.max_bytes
    }
}

/// One opaque raw frame returned by a mechanical backend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawFrameBytes {
    sequence: u64,
    append_request_id: AppendRequestId,
    frame_bytes: Vec<u8>,
    frame_digest: ContentDigest,
    head_digest: ContentDigest,
}

impl RawFrameBytes {
    /// Constructs one bounded raw frame returned by a mechanical backend.
    pub fn new(
        sequence: u64,
        append_request_id: AppendRequestId,
        frame_bytes: Vec<u8>,
        frame_digest: ContentDigest,
        head_digest: ContentDigest,
    ) -> BackendResult<Self> {
        if sequence == 0
            || sequence as usize > mfm_journal::single_trust::MAX_RUN_FRAMES
            || frame_bytes.is_empty()
            || frame_bytes.len() > mfm_journal::single_trust::MAX_FRAME_BYTES
            || frame_digest != mfm_canonical::raw_content_digest(&frame_bytes)
        {
            return Err(BackendError::Capacity);
        }
        Ok(Self {
            sequence,
            append_request_id,
            frame_bytes,
            frame_digest,
            head_digest,
        })
    }

    /// Returns the one-based sequence.
    pub const fn sequence(&self) -> u64 {
        self.sequence
    }

    /// Returns the physical append identity.
    pub const fn append_request_id(&self) -> &AppendRequestId {
        &self.append_request_id
    }

    /// Returns the immutable frame bytes.
    pub fn frame_bytes(&self) -> &[u8] {
        &self.frame_bytes
    }

    /// Returns the recorded frame digest.
    pub const fn frame_digest(&self) -> &ContentDigest {
        &self.frame_digest
    }

    /// Returns the recursive head digest after this frame.
    pub const fn head_digest(&self) -> &ContentDigest {
        &self.head_digest
    }
}

/// One complete raw prefix returned by a backend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawRunPrefix {
    frames: Vec<RawFrameBytes>,
}

impl RawRunPrefix {
    /// Constructs one ordered raw prefix and records its physical head digest.
    pub fn new(frames: Vec<RawFrameBytes>) -> BackendResult<Self> {
        if frames.is_empty()
            || frames.len() > mfm_journal::single_trust::MAX_RUN_FRAMES
            || frames
                .iter()
                .enumerate()
                .any(|(index, frame)| frame.sequence() != index as u64 + 1)
            || frames
                .iter()
                .try_fold(0usize, |total, frame| {
                    total
                        .checked_add(frame.frame_bytes().len())
                        .filter(|bytes| *bytes <= mfm_journal::single_trust::MAX_RUN_FRAME_BYTES)
                        .ok_or(BackendError::Capacity)
                })
                .is_err()
        {
            return Err(BackendError::Capacity);
        }
        Ok(Self { frames })
    }

    /// Returns the ordered raw frames.
    pub fn frames(&self) -> &[RawFrameBytes] {
        &self.frames
    }
}

/// Borrowed mechanical history append command.
pub struct BackendAppendCommand<'a> {
    identity: &'a StructuredStoreIdentity,
    run_id: &'a RunId,
    expected_sequence: u64,
    append_request_id: &'a AppendRequestId,
    frame_bytes: &'a [u8],
    frame_digest: &'a ContentDigest,
    head_digest: &'a ContentDigest,
    previous_head_digest: Option<&'a ContentDigest>,
    admission: bool,
    fact_publication: Option<&'a RawFactPublication>,
}

impl<'a> BackendAppendCommand<'a> {
    /// Creates one borrowed command from already-qualified frame material.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        identity: &'a StructuredStoreIdentity,
        run_id: &'a RunId,
        expected_sequence: u64,
        append_request_id: &'a AppendRequestId,
        frame_bytes: &'a [u8],
        frame_digest: &'a ContentDigest,
        head_digest: &'a ContentDigest,
        previous_head_digest: Option<&'a ContentDigest>,
        admission: bool,
        fact_publication: Option<&'a RawFactPublication>,
    ) -> Self {
        Self {
            identity,
            run_id,
            expected_sequence,
            append_request_id,
            frame_bytes,
            frame_digest,
            head_digest,
            previous_head_digest,
            admission,
            fact_publication,
        }
    }

    /// Returns the Store identity.
    pub const fn identity(&self) -> &StructuredStoreIdentity {
        self.identity
    }

    /// Returns the run identity.
    pub const fn run_id(&self) -> &RunId {
        self.run_id
    }

    /// Returns the expected one-based sequence.
    pub const fn expected_sequence(&self) -> u64 {
        self.expected_sequence
    }

    /// Returns the append identity.
    pub const fn append_request_id(&self) -> &AppendRequestId {
        self.append_request_id
    }

    /// Returns the canonical frame bytes.
    pub const fn frame_bytes(&self) -> &[u8] {
        self.frame_bytes
    }

    /// Returns the frame digest.
    pub const fn frame_digest(&self) -> &ContentDigest {
        self.frame_digest
    }

    /// Returns the recursive candidate head digest.
    pub const fn head_digest(&self) -> &ContentDigest {
        self.head_digest
    }

    /// Returns the predecessor head digest, if this is not genesis.
    pub const fn previous_head_digest(&self) -> Option<&ContentDigest> {
        self.previous_head_digest
    }

    /// Returns whether this command is the sole genesis append.
    pub const fn is_admission(&self) -> bool {
        self.admission
    }

    /// Returns the optional dense fact publication coordinate.
    pub const fn fact_publication(&self) -> Option<&RawFactPublication> {
        self.fact_publication
    }
}

/// Physical append result with no semantic payload echo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackendAppendOutcome {
    /// The candidate crossed the backend commit point.
    NewlyCommitted,
    /// The same physical append was retained; the stored bytes are returned only for resolution.
    Found(Box<RawFrameBytes>),
    /// The predecessor head no longer matches.
    StaleHead {
        /// The current one-based route head sequence.
        actual_sequence: u64,
    },
    /// The transaction outcome is unknown.
    AcknowledgementUnknown,
}

/// Physical configuration append result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendConfigurationOutcome {
    /// The candidate crossed the backend commit point.
    NewlyCommitted,
    /// The same physical configuration append was retained.
    Found {
        /// The one-based retained configuration sequence.
        sequence: u64,
    },
    /// The expected configuration head is stale.
    StaleHead {
        /// The current configuration head sequence.
        actual_sequence: u64,
    },
    /// The transaction outcome is unknown.
    AcknowledgementUnknown,
}

/// Store conclusion outcome that retains the exact append owner across an unknown acknowledgement.
#[derive(Debug)]
pub enum ConclusionCommitOutcome {
    /// The conclusion append has a known mechanical disposition.
    Disposition(AppendDisposition),
    /// The transaction outcome is unknown; the same semantic owner must be resolved explicitly.
    AcknowledgementUnknown(crate::single_trust::PreparedConclusion),
    /// A known Store failure occurred before an append could be accepted; the exact owner remains
    /// available for explicit retry or supervisor classification.
    Rejected {
        /// The unchanged conclusion owner.
        owner: crate::single_trust::PreparedConclusion,
        /// The redaction-safe Store failure.
        error: StoreError,
    },
}

/// Configuration append outcome that retains the exact owner after an unknown acknowledgement.
#[derive(Debug)]
pub enum ConfigurationCommitOutcome {
    /// The configuration append has a known mechanical disposition.
    Disposition {
        /// The exact known physical disposition.
        disposition: ConfigurationAppendDisposition,
        /// The already-ingressed revision when the backend found or committed this exact owner.
        /// Stale heads expose no revision and therefore cannot promote a caller snapshot.
        revision: Option<ConfigurationRevision>,
    },
    /// The transaction outcome is unknown; the exact semantic owner remains available.
    AcknowledgementUnknown(PreparedConfigurationWrite),
    /// A known Store failure occurred before an append could be accepted; the exact owner remains
    /// available for explicit retry or supervisor classification.
    Rejected {
        /// The unchanged configuration owner.
        owner: PreparedConfigurationWrite,
        /// The redaction-safe Store failure.
        error: StoreError,
    },
}

/// One borrowed mechanical configuration append command.
pub struct ConfigurationAppendCommand<'a> {
    identity: &'a StructuredStoreIdentity,
    expected_sequence: u64,
    append_request_id: &'a AppendRequestId,
    canonical_bytes: &'a [u8],
    content_ref: &'a ContentRef,
}

impl<'a> ConfigurationAppendCommand<'a> {
    /// Creates one bounded configuration command.
    pub(crate) const fn new(
        identity: &'a StructuredStoreIdentity,
        expected_sequence: u64,
        append_request_id: &'a AppendRequestId,
        canonical_bytes: &'a [u8],
        content_ref: &'a ContentRef,
    ) -> Self {
        Self {
            identity,
            expected_sequence,
            append_request_id,
            canonical_bytes,
            content_ref,
        }
    }

    /// Returns the Store identity.
    pub const fn identity(&self) -> &StructuredStoreIdentity {
        self.identity
    }
    /// Returns the expected revision sequence.
    pub const fn expected_sequence(&self) -> u64 {
        self.expected_sequence
    }
    /// Returns the append identity.
    pub const fn append_request_id(&self) -> &AppendRequestId {
        self.append_request_id
    }
    /// Returns canonical revision bytes.
    pub const fn canonical_bytes(&self) -> &[u8] {
        self.canonical_bytes
    }
    /// Returns the revision content identity.
    pub const fn content_ref(&self) -> &ContentRef {
        self.content_ref
    }
}

/// One raw configuration revision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawConfigurationRevision {
    sequence: u64,
    append_request_id: AppendRequestId,
    canonical_bytes: Vec<u8>,
    content_ref: ContentRef,
}

impl RawConfigurationRevision {
    /// Constructs one bounded raw configuration revision.
    pub fn new(
        sequence: u64,
        append_request_id: AppendRequestId,
        canonical_bytes: Vec<u8>,
        content_ref: ContentRef,
    ) -> BackendResult<Self> {
        if sequence == 0
            || sequence as usize > mfm_journal::single_trust::MAX_CONFIGURATION_REVISIONS
            || canonical_bytes.is_empty()
            || canonical_bytes.len() > mfm_journal::single_trust::MAX_CONFIGURATION_REVISION_BYTES
        {
            return Err(BackendError::Capacity);
        }
        Ok(Self {
            sequence,
            append_request_id,
            canonical_bytes,
            content_ref,
        })
    }

    /// Returns the revision sequence.
    pub const fn sequence(&self) -> u64 {
        self.sequence
    }
    /// Returns the append identity.
    pub const fn append_request_id(&self) -> &AppendRequestId {
        &self.append_request_id
    }
    /// Returns canonical revision bytes.
    pub fn canonical_bytes(&self) -> &[u8] {
        &self.canonical_bytes
    }
    /// Returns the revision content identity.
    pub const fn content_ref(&self) -> &ContentRef {
        &self.content_ref
    }
}

/// One raw fact publication coordinate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawFactPublication {
    publication_sequence: u64,
    run_id: RunId,
    run_sequence: u64,
    selection_ref: ContentRef,
}

impl RawFactPublication {
    /// Creates one bounded raw fact coordinate.
    pub fn new(
        publication_sequence: u64,
        run_id: RunId,
        run_sequence: u64,
        selection_ref: ContentRef,
    ) -> BackendResult<Self> {
        if publication_sequence == 0
            || publication_sequence as usize > mfm_journal::single_trust::MAX_RUN_FRAMES
            || run_sequence == 0
            || run_sequence as usize > mfm_journal::single_trust::MAX_RUN_FRAMES
        {
            return Err(BackendError::Capacity);
        }
        Ok(Self {
            publication_sequence,
            run_id,
            run_sequence,
            selection_ref,
        })
    }

    /// Returns the publication sequence.
    pub const fn publication_sequence(&self) -> u64 {
        self.publication_sequence
    }
    /// Returns the run identity.
    pub const fn run_id(&self) -> &RunId {
        &self.run_id
    }
    /// Returns the run frame sequence.
    pub const fn run_sequence(&self) -> u64 {
        self.run_sequence
    }
    /// Returns the selected response identity.
    pub const fn selection_ref(&self) -> &ContentRef {
        &self.selection_ref
    }
}

/// Raw fact-head snapshot for callback-free Store qualification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawFactSnapshot {
    head_sequence: u64,
    publications: Vec<RawFactPublication>,
}

impl RawFactSnapshot {
    /// Constructs one raw fact snapshot.
    pub fn new(head_sequence: u64, publications: Vec<RawFactPublication>) -> BackendResult<Self> {
        if head_sequence != publications.len() as u64
            || publications
                .iter()
                .enumerate()
                .any(|(index, publication)| publication.publication_sequence() != index as u64 + 1)
            || publications.len() > mfm_journal::single_trust::MAX_RUN_FRAMES
        {
            return Err(BackendError::Capacity);
        }
        Ok(Self {
            head_sequence,
            publications,
        })
    }
    /// Returns the dense head sequence.
    pub const fn head_sequence(&self) -> u64 {
        self.head_sequence
    }
    /// Returns all retained publication coordinates.
    pub fn publications(&self) -> &[RawFactPublication] {
        &self.publications
    }
}

/// One composite mechanical backend object.
pub trait StructuredStoreBackend: Send + Sync + 'static {
    /// Returns the immutable identity owned by this backend.
    fn identity(&self) -> StructuredStoreIdentity;

    /// Checks schema/channel/durability readiness without enumerating run history.
    fn check_ready<'a>(&'a self) -> BackendFuture<'a, ()> {
        Box::pin(async { Ok(()) })
    }

    /// Loads one bounded complete run prefix without decoding it.
    fn load_complete_prefix<'a>(
        &'a self,
        run_id: &'a RunId,
        limit: RawHistoryLoadLimit,
    ) -> BackendFuture<'a, Option<RawRunPrefix>>;

    /// Compare-and-appends one borrowed raw frame.
    fn compare_and_append<'a>(
        &'a self,
        command: &'a BackendAppendCommand<'a>,
    ) -> BackendFuture<'a, BackendAppendOutcome>;

    /// Loads one bounded configuration stream.
    fn load_configuration<'a>(&'a self) -> BackendFuture<'a, Vec<RawConfigurationRevision>>;

    /// Compare-and-appends one borrowed configuration revision.
    fn compare_and_append_configuration<'a>(
        &'a self,
        command: &'a ConfigurationAppendCommand<'a>,
    ) -> BackendFuture<'a, BackendConfigurationOutcome>;

    /// Loads the fixed-snapshot fact publication coordinates.
    fn load_facts<'a>(&'a self) -> BackendFuture<'a, RawFactSnapshot>;

    /// Lists run identities for one explicit audit snapshot.
    fn audit_run_ids<'a>(&'a self) -> BackendFuture<'a, Vec<RunId>>;
}

/// Reference implementation of the mechanical backend used by Store tests.
pub struct MemoryStructuredBackend {
    identity: StructuredStoreIdentity,
    state: Arc<Mutex<MemoryBackendState>>,
}

#[derive(Default)]
struct MemoryBackendState {
    runs: BTreeMap<RunId, Vec<RawFrameBytes>>,
    configurations: Vec<RawConfigurationRevision>,
    facts: Vec<RawFactPublication>,
}

impl MemoryStructuredBackend {
    /// Creates one empty mechanical backend.
    pub fn new(identity: StructuredStoreIdentity) -> Self {
        Self {
            identity,
            state: Arc::new(Mutex::new(MemoryBackendState::default())),
        }
    }
}

impl StructuredStoreBackend for MemoryStructuredBackend {
    fn identity(&self) -> StructuredStoreIdentity {
        self.identity.clone()
    }

    fn load_complete_prefix<'a>(
        &'a self,
        run_id: &'a RunId,
        limit: RawHistoryLoadLimit,
    ) -> BackendFuture<'a, Option<RawRunPrefix>> {
        Box::pin(async move {
            let state = self.state.lock().map_err(|_| BackendError::Storage)?;
            let Some(frames) = state.runs.get(run_id) else {
                return Ok(None);
            };
            if frames.len() > limit.max_frames() {
                return Err(BackendError::Capacity);
            }
            let total = frames.iter().try_fold(0usize, |total, frame| {
                total
                    .checked_add(frame.frame_bytes().len())
                    .ok_or(BackendError::Capacity)
            })?;
            if total > limit.max_bytes() {
                return Err(BackendError::Capacity);
            }
            RawRunPrefix::new(frames.clone()).map(Some)
        })
    }

    fn compare_and_append<'a>(
        &'a self,
        command: &'a BackendAppendCommand<'a>,
    ) -> BackendFuture<'a, BackendAppendOutcome> {
        Box::pin(async move {
            if command.identity() != &self.identity {
                return Err(BackendError::Identity);
            }
            if command.frame_bytes().is_empty()
                || command.frame_bytes().len() > mfm_journal::single_trust::MAX_FRAME_BYTES
                || command.expected_sequence() == 0
            {
                return Err(BackendError::Capacity);
            }
            let mut state = self.state.lock().map_err(|_| BackendError::Storage)?;
            let fact_head = state.facts.len() as u64;
            let publication = command.fact_publication().cloned();
            let frames = state.runs.entry(command.run_id().clone()).or_default();
            if let Some(existing) = frames
                .iter()
                .find(|frame| frame.append_request_id() == command.append_request_id())
            {
                if existing.frame_bytes() == command.frame_bytes()
                    && existing.frame_digest() == command.frame_digest()
                    && existing.head_digest() == command.head_digest()
                {
                    return Ok(BackendAppendOutcome::Found(Box::new(existing.clone())));
                }
                return Err(BackendError::Conflict);
            }
            let actual = frames.len() as u64;
            if command.expected_sequence() != actual.saturating_add(1) {
                return Ok(BackendAppendOutcome::StaleHead {
                    actual_sequence: actual,
                });
            }
            if frames.len() >= mfm_journal::single_trust::MAX_RUN_FRAMES
                || frames
                    .iter()
                    .try_fold(command.frame_bytes().len(), |total, frame| {
                        total
                            .checked_add(frame.frame_bytes().len())
                            .filter(|bytes| {
                                *bytes <= mfm_journal::single_trust::MAX_RUN_FRAME_BYTES
                            })
                            .ok_or(BackendError::Capacity)
                    })
                    .is_err()
            {
                return Err(BackendError::Capacity);
            }
            if command.is_admission() != (actual == 0) {
                return Err(BackendError::Storage);
            }
            if actual > 0 {
                let previous = frames.last().map(RawFrameBytes::head_digest);
                if command.previous_head_digest() != previous {
                    return Ok(BackendAppendOutcome::StaleHead {
                        actual_sequence: actual,
                    });
                }
            }
            if let Some(publication) = publication.as_ref() {
                if publication.publication_sequence() != fact_head + 1 {
                    return Err(BackendError::Conflict);
                }
            }
            let raw = RawFrameBytes::new(
                command.expected_sequence(),
                command.append_request_id().clone(),
                command.frame_bytes().to_vec(),
                command.frame_digest().clone(),
                command.head_digest().clone(),
            )?;
            frames.push(raw);
            if let Some(publication) = publication {
                state.facts.push(publication);
            }
            Ok(BackendAppendOutcome::NewlyCommitted)
        })
    }

    fn load_configuration<'a>(&'a self) -> BackendFuture<'a, Vec<RawConfigurationRevision>> {
        Box::pin(async move {
            let state = self.state.lock().map_err(|_| BackendError::Storage)?;
            if state.configurations.len() > mfm_journal::single_trust::MAX_CONFIGURATION_REVISIONS {
                return Err(BackendError::Capacity);
            }
            let total_bytes = state
                .configurations
                .iter()
                .try_fold(0usize, |total, revision| {
                    total
                        .checked_add(revision.canonical_bytes().len())
                        .filter(|bytes| {
                            *bytes <= mfm_journal::single_trust::MAX_CONFIGURATION_STREAM_BYTES
                        })
                        .ok_or(BackendError::Capacity)
                })?;
            if total_bytes > mfm_journal::single_trust::MAX_CONFIGURATION_STREAM_BYTES {
                return Err(BackendError::Capacity);
            }
            Ok(state.configurations.clone())
        })
    }

    fn compare_and_append_configuration<'a>(
        &'a self,
        command: &'a ConfigurationAppendCommand<'a>,
    ) -> BackendFuture<'a, BackendConfigurationOutcome> {
        Box::pin(async move {
            if command.identity() != &self.identity {
                return Err(BackendError::Identity);
            }
            if command.canonical_bytes().is_empty() {
                return Err(BackendError::Capacity);
            }
            let mut state = self.state.lock().map_err(|_| BackendError::Storage)?;
            if let Some(existing) = state
                .configurations
                .iter()
                .find(|revision| revision.append_request_id() == command.append_request_id())
            {
                if existing.canonical_bytes() == command.canonical_bytes()
                    && existing.content_ref() == command.content_ref()
                {
                    return Ok(BackendConfigurationOutcome::Found {
                        sequence: existing.sequence(),
                    });
                }
                return Err(BackendError::Conflict);
            }
            let actual = state.configurations.len() as u64;
            if command.expected_sequence() != actual {
                return Ok(BackendConfigurationOutcome::StaleHead {
                    actual_sequence: actual,
                });
            }
            if state.configurations.len() >= mfm_journal::single_trust::MAX_CONFIGURATION_REVISIONS
                || state
                    .configurations
                    .iter()
                    .try_fold(command.canonical_bytes().len(), |total, revision| {
                        total
                            .checked_add(revision.canonical_bytes().len())
                            .filter(|bytes| {
                                *bytes <= mfm_journal::single_trust::MAX_CONFIGURATION_STREAM_BYTES
                            })
                            .ok_or(BackendError::Capacity)
                    })
                    .is_err()
            {
                return Err(BackendError::Capacity);
            }
            let revision = RawConfigurationRevision::new(
                actual + 1,
                command.append_request_id().clone(),
                command.canonical_bytes().to_vec(),
                command.content_ref().clone(),
            )?;
            state.configurations.push(revision);
            Ok(BackendConfigurationOutcome::NewlyCommitted)
        })
    }

    fn load_facts<'a>(&'a self) -> BackendFuture<'a, RawFactSnapshot> {
        Box::pin(async move {
            let state = self.state.lock().map_err(|_| BackendError::Storage)?;
            RawFactSnapshot::new(state.facts.len() as u64, state.facts.clone())
        })
    }

    fn audit_run_ids<'a>(&'a self) -> BackendFuture<'a, Vec<RunId>> {
        Box::pin(async move {
            Ok(self
                .state
                .lock()
                .map_err(|_| BackendError::Storage)?
                .runs
                .keys()
                .cloned()
                .collect())
        })
    }
}

/// Error returned while opening a branded semantic Store.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum StoreOpenError {
    /// The requested identity differs from the backend identity.
    #[error("Store identity is invalid")]
    Identity,
    /// The requested work envelope is invalid.
    #[error("Store work limits are invalid")]
    Capacity,
    /// The backend did not meet its admitted schema or durability contract.
    #[error("Store backend readiness is invalid")]
    Backend,
}

/// One opaque semantic Store opening.
#[derive(Clone)]
pub struct OpenedStructuredStore {
    inner: Arc<OpenedStoreInner>,
}

struct OpenedStoreInner {
    backend: Arc<dyn StructuredStoreBackend>,
    identity: StructuredStoreIdentity,
    catalog: ProgramCatalog,
    limits: StoreWorkLimits,
    brand: Arc<StoreBrand>,
}

#[derive(Debug)]
pub(crate) struct StoreBrand;

/// Entry point for one composite Store open.
pub struct StructuredStore;

impl StructuredStore {
    /// Opens the bounded in-memory mechanical backend synchronously for trusted composition.
    pub fn open_memory(
        expected_identity: StructuredStoreIdentity,
        catalog: ProgramCatalog,
        limits: StoreWorkLimits,
    ) -> std::result::Result<OpenedStructuredStore, StoreOpenError> {
        limits.validate().map_err(|_| StoreOpenError::Capacity)?;
        let backend = Arc::new(MemoryStructuredBackend::new(expected_identity.clone()));
        Ok(OpenedStructuredStore {
            inner: Arc::new(OpenedStoreInner {
                backend,
                identity: expected_identity,
                catalog,
                limits,
                brand: Arc::new(StoreBrand),
            }),
        })
    }

    /// Opens one exact backend, identity, catalog, and bounded work envelope.
    pub async fn open(
        backend: Arc<dyn StructuredStoreBackend>,
        expected_identity: StructuredStoreIdentity,
        catalog: ProgramCatalog,
        limits: StoreWorkLimits,
    ) -> std::result::Result<OpenedStructuredStore, StoreOpenError> {
        limits.validate().map_err(|_| StoreOpenError::Capacity)?;
        if backend.identity() != expected_identity {
            return Err(StoreOpenError::Identity);
        }
        backend
            .check_ready()
            .await
            .map_err(|_| StoreOpenError::Backend)?;
        Ok(OpenedStructuredStore {
            inner: Arc::new(OpenedStoreInner {
                backend,
                identity: expected_identity,
                catalog,
                limits,
                brand: Arc::new(StoreBrand),
            }),
        })
    }
}

impl OpenedStructuredStore {
    /// Returns the exact Store identity.
    pub fn identity(&self) -> &StructuredStoreIdentity {
        &self.inner.identity
    }

    /// Returns the exact Program catalog brand.
    pub fn catalog(&self) -> &ProgramCatalog {
        &self.inner.catalog
    }

    /// Returns whether two handles share the exact private Store opening.
    pub fn same_open(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner.brand, &other.inner.brand)
    }

    /// Consumes this open into its branded Store ports.
    pub fn split(self) -> StoreParts {
        StoreParts {
            history: QualifiedHistoryPort {
                inner: Arc::clone(&self.inner),
            },
            reader: HistoryReader {
                inner: Arc::clone(&self.inner),
            },
            configuration: ConfigurationStore {
                inner: Arc::clone(&self.inner),
            },
            audit: StoreAuditPort { inner: self.inner },
        }
    }

    /// Returns the configured complete-prefix limit.
    pub fn history_limit(&self) -> RawHistoryLoadLimit {
        RawHistoryLoadLimit::new(
            self.inner.limits.max_run_frames(),
            self.inner.limits.max_run_frame_bytes(),
        )
    }

    /// Loads one complete run prefix through the branded semantic port.
    pub async fn load(&self, run_id: &RunId) -> Result<QualifiedRun> {
        self.history_port().load(run_id).await
    }

    /// Appends one already-constructed strict frame through the branded mechanical port.
    pub async fn append(
        &self,
        frame: mfm_journal::single_trust::RunFrame,
    ) -> Result<AppendDisposition> {
        self.history_port().append(frame).await
    }

    /// Creates one access preparation after callback-free semantic qualification.
    #[allow(clippy::too_many_arguments)]
    pub async fn prepare_access(
        &self,
        run_id: &RunId,
        document: &mfm_program::single_trust::ProgramDocument,
        expected_sequence: u64,
        append_request_id: AppendRequestId,
        prepared: mfm_journal::single_trust::StatePrepared,
        objects: Vec<mfm_journal::single_trust::ImmutableObject>,
    ) -> Result<crate::single_trust::PreparationAppend> {
        let current = self.load(run_id).await?;
        self.prepare_access_qualified(
            &current,
            document,
            expected_sequence,
            append_request_id,
            prepared,
            objects,
        )
        .await
    }

    /// Prepares an access append against a prefix already qualified by Runtime.
    #[allow(clippy::too_many_arguments)]
    pub async fn prepare_access_qualified(
        &self,
        current: &QualifiedRun,
        document: &mfm_program::single_trust::ProgramDocument,
        expected_sequence: u64,
        append_request_id: AppendRequestId,
        prepared: mfm_journal::single_trust::StatePrepared,
        objects: Vec<mfm_journal::single_trust::ImmutableObject>,
    ) -> Result<crate::single_trust::PreparationAppend> {
        let reduced = self.reduce_qualified(current, document.clone())?;
        self.prepare_access_qualified_with_reduced(
            current,
            document,
            &reduced,
            expected_sequence,
            append_request_id,
            prepared,
            objects,
        )
        .await
    }

    /// Prepares an access append using the exact reduction already retained by Runtime.
    #[allow(clippy::too_many_arguments)]
    pub async fn prepare_access_qualified_with_reduced(
        &self,
        current: &QualifiedRun,
        document: &mfm_program::single_trust::ProgramDocument,
        reduced: &crate::single_trust::ReducedRunState,
        expected_sequence: u64,
        append_request_id: AppendRequestId,
        prepared: mfm_journal::single_trust::StatePrepared,
        objects: Vec<mfm_journal::single_trust::ImmutableObject>,
    ) -> Result<crate::single_trust::PreparationAppend> {
        if current.scope() != self.identity().scope()
            || current.epoch() != self.identity().epoch()
            || current.tenant() != self.identity().tenant()
        {
            return Err(StoreError::Identity);
        }
        let candidate = prepare_access_from_current(
            self.identity().scope(),
            self.identity().epoch(),
            self.identity().tenant(),
            current,
            current.run_id(),
            document,
            reduced,
            expected_sequence,
            append_request_id,
            prepared,
            objects,
        )?;
        let Some(frame) = candidate.frame else {
            return Ok(candidate.append);
        };
        let physical = self
            .history_port()
            .append_against_qualified_prefix(current, frame.clone())
            .await?;
        let mut append = candidate.append;
        append.set_frame(frame);
        Ok(append.with_disposition(physical))
    }

    /// Builds one Store-owned conclusion append owner from the selected prefix.
    #[allow(clippy::too_many_arguments)]
    pub async fn prepare_conclusion(
        &self,
        run_id: &RunId,
        document: &mfm_program::single_trust::ProgramDocument,
        expected_sequence: u64,
        append_request_id: AppendRequestId,
        conclusion: mfm_journal::single_trust::StateConcluded,
        objects: Vec<mfm_journal::single_trust::ImmutableObject>,
        maximum_conclusion_bytes: u64,
    ) -> Result<crate::single_trust::PreparedConclusion> {
        let current = self.load(run_id).await?;
        self.prepare_conclusion_qualified(
            &current,
            document,
            expected_sequence,
            append_request_id,
            conclusion,
            objects,
            maximum_conclusion_bytes,
        )
    }

    /// Builds a conclusion owner against a prefix already qualified by Runtime.
    #[allow(clippy::too_many_arguments)]
    pub fn prepare_conclusion_qualified(
        &self,
        current: &QualifiedRun,
        document: &mfm_program::single_trust::ProgramDocument,
        expected_sequence: u64,
        append_request_id: AppendRequestId,
        conclusion: mfm_journal::single_trust::StateConcluded,
        objects: Vec<mfm_journal::single_trust::ImmutableObject>,
        maximum_conclusion_bytes: u64,
    ) -> Result<crate::single_trust::PreparedConclusion> {
        let reduced = self.reduce_qualified(current, document.clone())?;
        self.prepare_conclusion_qualified_with_reduced(
            current,
            document,
            &reduced,
            expected_sequence,
            append_request_id,
            conclusion,
            objects,
            maximum_conclusion_bytes,
        )
    }

    /// Builds a conclusion owner using the exact reduction already retained by Runtime.
    #[allow(clippy::too_many_arguments)]
    pub fn prepare_conclusion_qualified_with_reduced(
        &self,
        current: &QualifiedRun,
        document: &mfm_program::single_trust::ProgramDocument,
        reduced: &crate::single_trust::ReducedRunState,
        expected_sequence: u64,
        append_request_id: AppendRequestId,
        conclusion: mfm_journal::single_trust::StateConcluded,
        objects: Vec<mfm_journal::single_trust::ImmutableObject>,
        maximum_conclusion_bytes: u64,
    ) -> Result<crate::single_trust::PreparedConclusion> {
        if current.scope() != self.identity().scope()
            || current.epoch() != self.identity().epoch()
            || current.tenant() != self.identity().tenant()
        {
            return Err(StoreError::Identity);
        }
        prepare_conclusion_from_current(
            self.identity().scope(),
            self.identity().epoch(),
            self.identity().tenant(),
            current,
            current.run_id(),
            document,
            reduced,
            expected_sequence,
            append_request_id,
            conclusion,
            objects,
            maximum_conclusion_bytes,
        )
        .map(|mut owner| {
            owner.bind_store(Arc::clone(&self.inner.brand));
            owner
        })
    }

    /// Commits one conclusion owner through this exact opened Store.
    pub async fn commit_conclusion(
        &self,
        owner: crate::single_trust::PreparedConclusion,
    ) -> Result<ConclusionCommitOutcome> {
        if !owner.belongs_to(self.identity(), &self.inner.brand) {
            return Ok(ConclusionCommitOutcome::Rejected {
                owner,
                error: StoreError::Identity,
            });
        }
        let frame = owner.frame().clone();
        let disposition = match self.append(frame).await {
            Ok(disposition) => disposition,
            Err(error) => return Ok(ConclusionCommitOutcome::Rejected { owner, error }),
        };
        if matches!(disposition, AppendDisposition::AcknowledgementUnknown) {
            return Ok(ConclusionCommitOutcome::AcknowledgementUnknown(owner));
        }
        Ok(ConclusionCommitOutcome::Disposition(disposition))
    }

    /// Reduces one qualified prefix without exposing a history handle to State callbacks.
    pub async fn reduce(
        &self,
        run_id: &RunId,
        document: mfm_program::single_trust::ProgramDocument,
    ) -> Result<crate::single_trust::ReducedRunState> {
        let current = self.load(run_id).await?;
        self.reduce_qualified(&current, document)
    }

    /// Reduces one already-qualified prefix without reading or reloading the backend.
    pub fn reduce_qualified(
        &self,
        run: &QualifiedRun,
        document: mfm_program::single_trust::ProgramDocument,
    ) -> Result<crate::single_trust::ReducedRunState> {
        if run.scope() != self.identity().scope()
            || run.epoch() != self.identity().epoch()
            || run.tenant() != self.identity().tenant()
        {
            return Err(StoreError::Identity);
        }
        reduce_qualified(run, document)
    }

    /// Advances a retained reducer result over one hot conclusion without re-folding its prefix.
    pub fn advance_reduced(
        &self,
        reduced: &crate::single_trust::ReducedRunState,
        previous: &QualifiedRun,
        next: &QualifiedRun,
        document: &mfm_program::single_trust::ProgramDocument,
    ) -> Result<crate::single_trust::ReducedRunState> {
        if previous.scope() != self.identity().scope()
            || previous.epoch() != self.identity().epoch()
            || previous.tenant() != self.identity().tenant()
            || next.scope() != self.identity().scope()
            || next.epoch() != self.identity().epoch()
            || next.tenant() != self.identity().tenant()
        {
            return Err(StoreError::Identity);
        }
        advance_reduced(reduced, previous, next, document)
    }

    /// Qualifies a candidate prefix whose predecessor was already held by Runtime.
    pub fn qualify_appended(
        &self,
        predecessor: &QualifiedRun,
        frame: mfm_journal::single_trust::RunFrame,
    ) -> Result<QualifiedRun> {
        if predecessor.scope() != self.identity().scope()
            || predecessor.epoch() != self.identity().epoch()
            || predecessor.tenant() != self.identity().tenant()
            || frame.run_id() != predecessor.run_id()
            || frame.expected_sequence() != predecessor.head_sequence().saturating_add(1)
        {
            return Err(StoreError::Identity);
        }
        predecessor.clone().append_validated(frame)
    }

    /// Qualifies one direct-new admission frame without a post-commit backend read.
    pub fn qualify_admission(
        &self,
        frame: mfm_journal::single_trust::RunFrame,
    ) -> Result<QualifiedRun> {
        if !frame.record().is_admission() {
            return Err(StoreError::InvalidRecord);
        }
        QualifiedRun::qualify_prefix(
            self.identity().scope().clone(),
            self.identity().epoch(),
            self.identity().tenant().clone(),
            vec![frame],
        )
    }

    fn history_port(&self) -> QualifiedHistoryPort {
        QualifiedHistoryPort {
            inner: Arc::clone(&self.inner),
        }
    }
}

/// The consuming branded Store port split.
pub struct StoreParts {
    history: QualifiedHistoryPort,
    reader: HistoryReader,
    configuration: ConfigurationStore,
    audit: StoreAuditPort,
}

impl StoreParts {
    /// Returns the non-Clone Runtime history port.
    /// Consumes the split into its four branded ports.
    pub fn into_parts(
        self,
    ) -> (
        QualifiedHistoryPort,
        HistoryReader,
        ConfigurationStore,
        StoreAuditPort,
    ) {
        (self.history, self.reader, self.configuration, self.audit)
    }
}

/// Non-Clone Runtime-facing history owner.
pub struct QualifiedHistoryPort {
    inner: Arc<OpenedStoreInner>,
}

impl QualifiedHistoryPort {
    /// Loads and qualifies one complete retained prefix.
    pub async fn load(&self, run_id: &RunId) -> Result<QualifiedRun> {
        let raw = self
            .inner
            .backend
            .load_complete_prefix(run_id, self.limit())
            .await
            .map_err(map_backend_error)?
            .ok_or(StoreError::NotFound)?;
        qualify_raw_prefix(&self.inner.identity, raw)
    }

    /// Performs one semantic-free mechanical append after strict local frame validation.
    pub async fn append(
        &self,
        frame: mfm_journal::single_trust::RunFrame,
    ) -> Result<AppendDisposition> {
        frame.validate().map_err(|_| StoreError::InvalidRecord)?;
        if frame.store_scope_id() != self.inner.identity.scope()
            || frame.store_epoch() != self.inner.identity.epoch()
        {
            return Err(StoreError::Identity);
        }
        let current = self
            .inner
            .backend
            .load_complete_prefix(frame.run_id(), self.limit())
            .await
            .map_err(map_backend_error)?;
        let previous_digest = current
            .as_ref()
            .and_then(|prefix| {
                frame
                    .expected_sequence()
                    .checked_sub(2)
                    .and_then(|index| usize::try_from(index).ok())
                    .and_then(|index| prefix.frames().get(index))
            })
            .map(RawFrameBytes::head_digest);
        self.append_against_previous_digest(previous_digest, frame)
            .await
    }

    async fn append_against_qualified_prefix(
        &self,
        current: &QualifiedRun,
        frame: mfm_journal::single_trust::RunFrame,
    ) -> Result<AppendDisposition> {
        let previous_digest = if frame.expected_sequence() == 1 {
            None
        } else {
            Some(current.head_digest()?)
        };
        self.append_against_previous_digest(previous_digest.as_ref(), frame)
            .await
    }

    async fn append_against_previous_digest(
        &self,
        previous_digest: Option<&ContentDigest>,
        frame: mfm_journal::single_trust::RunFrame,
    ) -> Result<AppendDisposition> {
        let candidate_head = frame
            .head_digest(previous_digest)
            .map_err(|_| StoreError::InvalidRecord)?;
        let bytes = frame
            .canonical_bytes()
            .map_err(|_| StoreError::InvalidRecord)?;
        let digest = mfm_canonical::raw_content_digest(bytes.as_bytes());
        let fact_publication = frame
            .record()
            .fact_publication()
            .map(|publication| {
                RawFactPublication::new(
                    publication.publication_sequence(),
                    frame.run_id().clone(),
                    frame.expected_sequence(),
                    publication.selection().value_ref().clone(),
                )
            })
            .transpose()
            .map_err(map_backend_error)?;
        let command = BackendAppendCommand::new(
            &self.inner.identity,
            frame.run_id(),
            frame.expected_sequence(),
            frame.append_request_id(),
            bytes.as_bytes(),
            &digest,
            &candidate_head,
            previous_digest,
            frame.record().is_admission(),
            fact_publication.as_ref(),
        );
        let result = match self.inner.backend.compare_and_append(&command).await {
            Ok(result) => result,
            Err(BackendError::AcknowledgementUnknown) => {
                return Ok(AppendDisposition::AcknowledgementUnknown);
            }
            Err(error) => return Err(map_backend_error(error)),
        };
        match result {
            BackendAppendOutcome::NewlyCommitted => Ok(AppendDisposition::NewlyCommitted {
                sequence: frame.expected_sequence(),
            }),
            BackendAppendOutcome::Found(raw) => Ok(AppendDisposition::Found {
                sequence: raw.sequence(),
            }),
            BackendAppendOutcome::StaleHead { actual_sequence } => {
                Ok(AppendDisposition::StaleHead { actual_sequence })
            }
            BackendAppendOutcome::AcknowledgementUnknown => {
                Ok(AppendDisposition::AcknowledgementUnknown)
            }
        }
    }

    fn limit(&self) -> RawHistoryLoadLimit {
        RawHistoryLoadLimit::new(
            self.inner.limits.max_run_frames(),
            self.inner.limits.max_run_frame_bytes(),
        )
    }
}

/// Cloneable callback-free history reader.
#[derive(Clone)]
pub struct HistoryReader {
    inner: Arc<OpenedStoreInner>,
}

impl HistoryReader {
    /// Loads one qualified run without producing a mutation owner.
    pub async fn load(&self, run_id: &RunId) -> Result<QualifiedRun> {
        QualifiedHistoryPort {
            inner: Arc::clone(&self.inner),
        }
        .load(run_id)
        .await
    }
}

/// Affine configuration mechanical port.
pub struct ConfigurationStore {
    inner: Arc<OpenedStoreInner>,
}

/// One affine configuration append owner branded to an opened Store.
#[derive(Debug)]
pub struct PreparedConfigurationWrite {
    expected_sequence: u64,
    total_bytes: usize,
    append_request_id: AppendRequestId,
    revision: ConfigurationRevision,
    brand: Arc<StoreBrand>,
}

impl PreparedConfigurationWrite {
    /// Returns the expected configuration head sequence.
    pub const fn expected_sequence(&self) -> u64 {
        self.expected_sequence
    }

    /// Returns the fixed physical append identity.
    pub const fn append_request_id(&self) -> &AppendRequestId {
        &self.append_request_id
    }

    /// Returns the validated canonical configuration revision.
    pub const fn revision(&self) -> &ConfigurationRevision {
        &self.revision
    }

    /// Returns the cumulative byte count after this direct successor.
    pub const fn total_bytes(&self) -> usize {
        self.total_bytes
    }
}

impl ConfigurationStore {
    /// Loads and qualifies one complete configuration snapshot.
    pub async fn load(&self) -> Result<ConfigurationSnapshot> {
        let raw = self
            .inner
            .backend
            .load_configuration()
            .await
            .map_err(map_backend_error)?;
        let revisions = raw
            .into_iter()
            .map(|revision| {
                let canonical_json = String::from_utf8(revision.canonical_bytes().to_vec())
                    .map_err(|_| StoreError::InvalidHistory)?;
                ConfigurationRevision::from_parts(
                    revision.sequence(),
                    revision.append_request_id().clone(),
                    canonical_json,
                    revision.content_ref().clone(),
                )
            })
            .collect::<Result<Vec<_>>>()?;
        ConfigurationSnapshot::from_revisions(revisions)
    }

    /// Prepares one validated configuration successor from a fixed snapshot without reading the
    /// retained stream again.
    pub fn prepare_append(
        &self,
        current: &ConfigurationSnapshot,
        append_request_id: AppendRequestId,
        canonical_json: String,
    ) -> Result<PreparedConfigurationWrite> {
        if current.head_sequence() as usize
            >= mfm_journal::single_trust::MAX_CONFIGURATION_REVISIONS
        {
            return Err(StoreError::Capacity);
        }
        let revision = ConfigurationRevision::new(
            current.head_sequence().saturating_add(1),
            append_request_id.clone(),
            canonical_json,
        )?;
        let total_bytes = current
            .total_bytes()
            .checked_add(revision.canonical_json().len())
            .ok_or(StoreError::Capacity)?;
        if total_bytes > mfm_journal::single_trust::MAX_CONFIGURATION_STREAM_BYTES {
            return Err(StoreError::Capacity);
        }
        Ok(PreparedConfigurationWrite {
            expected_sequence: current.head_sequence(),
            total_bytes,
            append_request_id,
            revision,
            brand: Arc::clone(&self.inner.brand),
        })
    }

    /// Consumes one Store-branded configuration owner through the exact backend command.
    pub async fn commit(
        &self,
        owner: PreparedConfigurationWrite,
    ) -> Result<ConfigurationCommitOutcome> {
        if !Arc::ptr_eq(&owner.brand, &self.inner.brand) {
            return Ok(ConfigurationCommitOutcome::Rejected {
                owner,
                error: StoreError::Identity,
            });
        }
        let command = ConfigurationAppendCommand::new(
            &self.inner.identity,
            owner.expected_sequence,
            &owner.append_request_id,
            owner.revision.canonical_json().as_bytes(),
            owner.revision.content_ref(),
        );
        let result = match self
            .inner
            .backend
            .compare_and_append_configuration(&command)
            .await
        {
            Ok(result) => result,
            Err(BackendError::AcknowledgementUnknown) => {
                return Ok(ConfigurationCommitOutcome::AcknowledgementUnknown(owner));
            }
            Err(error) => {
                return Ok(ConfigurationCommitOutcome::Rejected {
                    owner,
                    error: map_backend_error(error),
                })
            }
        };
        let (disposition, revision) = match result {
            BackendConfigurationOutcome::NewlyCommitted => (
                ConfigurationAppendDisposition::NewlyCommitted {
                    sequence: owner.expected_sequence.saturating_add(1),
                },
                Some(owner.revision.clone()),
            ),
            BackendConfigurationOutcome::Found { sequence } => (
                ConfigurationAppendDisposition::Found { sequence },
                Some(owner.revision.clone()),
            ),
            BackendConfigurationOutcome::StaleHead { actual_sequence } => (
                ConfigurationAppendDisposition::StaleHead { actual_sequence },
                None,
            ),
            BackendConfigurationOutcome::AcknowledgementUnknown => {
                return Ok(ConfigurationCommitOutcome::AcknowledgementUnknown(owner));
            }
        };
        Ok(ConfigurationCommitOutcome::Disposition {
            disposition,
            revision,
        })
    }
}

/// Non-Clone fixed-snapshot audit port.
pub struct StoreAuditPort {
    inner: Arc<OpenedStoreInner>,
}

impl StoreAuditPort {
    /// Lists run identities in one explicit backend snapshot.
    pub async fn run_ids(&self) -> BackendResult<Vec<RunId>> {
        self.inner.backend.audit_run_ids().await
    }

    /// Loads the fixed fact publication snapshot.
    pub async fn facts(&self) -> BackendResult<RawFactSnapshot> {
        self.inner.backend.load_facts().await
    }
}

fn qualify_raw_prefix(
    identity: &StructuredStoreIdentity,
    raw: RawRunPrefix,
) -> Result<QualifiedRun> {
    QualifiedRun::qualify_prefix(
        identity.scope().clone(),
        identity.epoch(),
        identity.tenant().clone(),
        decode_raw_frames(identity, &raw)?,
    )
}

fn decode_raw_frames(
    identity: &StructuredStoreIdentity,
    raw: &RawRunPrefix,
) -> Result<Vec<mfm_journal::single_trust::RunFrame>> {
    let mut frames = Vec::with_capacity(raw.frames().len());
    let mut previous_head = None;
    for stored in raw.frames() {
        if stored.frame_digest() != &mfm_canonical::raw_content_digest(stored.frame_bytes()) {
            return Err(StoreError::InvalidHistory);
        }
        let canonical =
            mfm_canonical::PlainCanonicalJsonBytes::from_canonical_json_slice(stored.frame_bytes())
                .map_err(|_| StoreError::InvalidHistory)?;
        let frame: mfm_journal::single_trust::RunFrame =
            serde_json::from_slice(canonical.as_bytes()).map_err(|_| StoreError::InvalidHistory)?;
        if frame
            .canonical_bytes()
            .map_err(|_| StoreError::InvalidHistory)?
            .as_bytes()
            != stored.frame_bytes()
            || frame.expected_sequence() != stored.sequence()
            || frame.append_request_id() != stored.append_request_id()
            || frame.store_scope_id() != identity.scope()
            || frame.store_epoch() != identity.epoch()
        {
            return Err(StoreError::InvalidHistory);
        }
        let expected_head = frame
            .head_digest(previous_head.as_ref())
            .map_err(|_| StoreError::InvalidHistory)?;
        if stored.head_digest() != &expected_head {
            return Err(StoreError::InvalidHistory);
        }
        previous_head = Some(expected_head);
        frames.push(frame);
    }
    Ok(frames)
}

fn map_backend_error(error: BackendError) -> StoreError {
    match error {
        BackendError::Identity => StoreError::Identity,
        BackendError::Conflict => StoreError::Conflict,
        BackendError::StaleHead => StoreError::NotActionable,
        BackendError::Capacity => StoreError::Capacity,
        BackendError::AcknowledgementUnknown
        | BackendError::Unsupported
        | BackendError::Storage => StoreError::InvalidHistory,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::single_trust::ConfigurationAppendDisposition;
    use mfm_ids::{DigestAlgorithm, DigestBytes, SchemaId};

    fn identity() -> StructuredStoreIdentity {
        StructuredStoreIdentity::new(
            StoreScopeId::new("mfm.store_scope.v1:0123456789abcdef0123456789abcdef")
                .expect("scope"),
            StoreEpoch::new(1),
            TenantScopeId::new("mfm.tenant_scope.v1:0123456789abcdef0123456789abcdef")
                .expect("tenant"),
        )
    }

    #[tokio::test]
    async fn configuration_owner_promotes_direct_success_and_retains_stale_owner() {
        let contract = ContentRef::new(
            SchemaId::new(
                "mfm.test.configuration",
                "1",
                DigestAlgorithm::Sha256JcsV1,
                DigestBytes::from_array([0; 32]),
            )
            .expect("schema"),
            ContentDigest::from_digest(DigestAlgorithm::Sha256V1, DigestBytes::from_array([1; 32])),
        )
        .expect("contract");
        let document = mfm_program::single_trust::ProgramDocument::new(
            mfm_ids::StableId::new("mfm.test.configuration-entry").expect("entry"),
            contract.clone(),
            contract,
            Vec::new(),
        )
        .expect("document");
        let (catalog, _) = ProgramCatalog::builder().finish(document).expect("catalog");
        let opened = StructuredStore::open_memory(identity(), catalog, StoreWorkLimits::default())
            .expect("opened store");
        let (_, _, configuration, _) = opened.split().into_parts();
        let empty = configuration.load().await.expect("empty snapshot");
        let owner = configuration
            .prepare_append(
                &empty,
                AppendRequestId::new("configuration-direct-0123456789ab").expect("request"),
                "{\"mode\":\"direct\"}".to_owned(),
            )
            .expect("direct owner");
        let revision = match configuration.commit(owner).await.expect("commit") {
            ConfigurationCommitOutcome::Disposition {
                disposition: ConfigurationAppendDisposition::NewlyCommitted { sequence: 1 },
                revision: Some(revision),
            } => revision,
            other => panic!("unexpected direct outcome: {other:?}"),
        };
        let promoted = empty.with_successor(revision).expect("local promotion");
        assert_eq!(promoted.head_sequence(), 1);

        let stale_owner = configuration
            .prepare_append(
                &empty,
                AppendRequestId::new("configuration-stale-0123456789ab").expect("request"),
                "{\"mode\":\"stale\"}".to_owned(),
            )
            .expect("stale owner");
        match configuration
            .commit(stale_owner)
            .await
            .expect("stale result")
        {
            ConfigurationCommitOutcome::Disposition {
                disposition: ConfigurationAppendDisposition::StaleHead { actual_sequence: 1 },
                revision: None,
                ..
            } => {}
            other => panic!("unexpected stale outcome: {other:?}"),
        }
        assert_eq!(empty.head_sequence(), 0);
    }
}
