//! Branded mechanical Store/backend boundary.
//!
//! This module contains only physical backend contracts and raw bounded rows.  It never parses a
//! Program, reduces a run, or constructs a Runtime owner.  `OpenedStructuredStore` is the sole
//! semantic bridge: it qualifies raw rows through the Store reducer before exposing them.

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use mfm_canonical::raw_content_digest;
use mfm_facts::{
    FactCompleteness, FactProposalSet, FactProvenance, FactSelection, FactSelectionFrontier,
    FactSelectionRequest,
};
use mfm_ids::{
    short_stable_id_fragment, AppendRequestId, ContentDigest, ContentRef, RunId, StoreEpoch,
    StoreScopeId, TenantScopeId,
};
use mfm_journal::single_trust::{PreparationRef, RunRecord, ValueRef};
use mfm_program::ProgramCatalog;
use mfm_values::MfmValue;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::single_trust::{
    advance_reduced, prepare_access_from_current, prepare_conclusion_from_current,
    reduce_qualified, AppendDisposition, ConfigurationAppendDisposition, ConfigurationRevision,
    ConfigurationSnapshot, QualifiedRun, Result, RunAction, StoreBrand, StoreError,
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
    /// The tenant fact publication head changed while this append was prepared.
    #[error("backend fact frontier changed")]
    FactFrontierChanged,
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
            || self.max_frame_bytes > self.max_run_frame_bytes
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
    fact_frontier: Option<u64>,
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
            fact_frontier: None,
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

    /// Returns the preparation-time fact frontier precondition, if any.
    pub const fn fact_frontier(&self) -> Option<u64> {
        self.fact_frontier
    }

    /// Binds a preparation-time fact frontier to this mechanical command.
    pub(crate) fn with_fact_frontier(mut self, fact_frontier: Option<u64>) -> Self {
        self.fact_frontier = fact_frontier;
        self
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
    Disposition {
        /// The physical append disposition.
        disposition: AppendDisposition,
        /// The exact frame after Store bound any fact publication coordinate.
        frame: mfm_journal::single_trust::RunFrame,
    },
    /// The transaction outcome is unknown; the same semantic owner must be resolved explicitly.
    AcknowledgementUnknown(crate::single_trust::PreparedConclusion),
    /// A physically different append already recorded the same semantic conclusion.
    AlreadyConcludedSame {
        /// Qualified history containing the durable conclusion.
        history: QualifiedRun,
    },
    /// A later Access preparation superseded the conclusion owner.
    NoLongerSelected {
        /// Qualified history containing the selected replacement.
        history: QualifiedRun,
    },
    /// A different durable conclusion won the same occurrence.
    Conflict {
        /// Qualified history at the competing head.
        history: QualifiedRun,
    },
    /// The latest retained prefix cannot satisfy the owner's conclusion contract.
    InvalidHistory {
        /// Qualified history retained for diagnosis and replay.
        history: QualifiedRun,
    },
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
    proposal_set_ref: ContentRef,
}

impl RawFactPublication {
    /// Creates one bounded raw fact coordinate.
    pub fn new(
        publication_sequence: u64,
        run_id: RunId,
        run_sequence: u64,
        proposal_set_ref: ContentRef,
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
            proposal_set_ref,
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
    /// Returns the published proposal-set identity.
    pub const fn proposal_set_ref(&self) -> &ContentRef {
        &self.proposal_set_ref
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
    #[cfg(feature = "test-support")]
    faults: Arc<Mutex<MemoryBackendFaults>>,
}

#[derive(Default)]
struct MemoryBackendState {
    runs: BTreeMap<RunId, Vec<RawFrameBytes>>,
    configurations: Vec<RawConfigurationRevision>,
    facts: Vec<RawFactPublication>,
}

#[cfg(feature = "test-support")]
#[derive(Default)]
struct MemoryBackendFaults {
    history_unknown_after_commit: bool,
    configuration_unknown_after_commit: bool,
}

impl MemoryStructuredBackend {
    /// Creates one empty mechanical backend.
    pub fn new(identity: StructuredStoreIdentity) -> Self {
        Self {
            identity,
            state: Arc::new(Mutex::new(MemoryBackendState::default())),
            #[cfg(feature = "test-support")]
            faults: Arc::new(Mutex::new(MemoryBackendFaults::default())),
        }
    }

    /// Makes the next successful history append report an unknown acknowledgement.
    #[cfg(feature = "test-support")]
    pub fn fail_next_history_acknowledgement(&self) {
        if let Ok(mut faults) = self.faults.lock() {
            faults.history_unknown_after_commit = true;
        }
    }

    /// Makes the next successful configuration append report an unknown acknowledgement.
    #[cfg(feature = "test-support")]
    pub fn fail_next_configuration_acknowledgement(&self) {
        if let Ok(mut faults) = self.faults.lock() {
            faults.configuration_unknown_after_commit = true;
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
                    return Err(BackendError::FactFrontierChanged);
                }
            }
            if command
                .fact_frontier()
                .is_some_and(|frontier| frontier != fact_head)
            {
                return Err(BackendError::FactFrontierChanged);
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
            #[cfg(feature = "test-support")]
            if self
                .faults
                .lock()
                .map(|mut faults| std::mem::take(&mut faults.history_unknown_after_commit))
                .map_err(|_| BackendError::Storage)?
            {
                return Ok(BackendAppendOutcome::AcknowledgementUnknown);
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
            #[cfg(feature = "test-support")]
            if self
                .faults
                .lock()
                .map(|mut faults| std::mem::take(&mut faults.configuration_unknown_after_commit))
                .map_err(|_| BackendError::Storage)?
            {
                return Ok(BackendConfigurationOutcome::AcknowledgementUnknown);
            }
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
    prefix_ingress: Arc<Semaphore>,
}

/// One shared bound for backend prefix ingress and callback-free qualification.
///
/// This is intentionally derived framework capacity rather than a public per-call knob.  A
/// permit remains held through the blocking decode/reduction job, including when its async join
/// handle is cancelled.
const PREFIX_INGRESS_JOBS: usize = 8;

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
                prefix_ingress: Arc::new(Semaphore::new(PREFIX_INGRESS_JOBS)),
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
                prefix_ingress: Arc::new(Semaphore::new(PREFIX_INGRESS_JOBS)),
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

    async fn validate_fact_frontiers(&self, run: &QualifiedRun) -> Result<()> {
        if !run.frames().iter().any(|frame| {
            matches!(
                frame.record(),
                RunRecord::StatePrepared(prepared)
                    if prepared.fact_request().is_some() && prepared.fact_selection().is_some()
            )
        }) {
            return Ok(());
        }
        let snapshot = self
            .inner
            .backend
            .load_facts()
            .await
            .map_err(map_backend_error)?;
        let expected_stream = crate::single_trust::fact_stream_ref(
            self.identity().scope(),
            self.identity().epoch(),
            self.identity().tenant(),
        )?;
        for frame in run.frames() {
            let RunRecord::StatePrepared(prepared) = frame.record() else {
                continue;
            };
            let (Some(request_ref), Some(selection_ref)) =
                (prepared.fact_request(), prepared.fact_selection())
            else {
                continue;
            };
            let request_object = frame
                .objects()
                .iter()
                .find(|object| object.content_ref() == request_ref.value_ref())
                .ok_or(StoreError::InvalidHistory)?;
            let selection_object = frame
                .objects()
                .iter()
                .find(|object| object.content_ref() == selection_ref.value_ref())
                .ok_or(StoreError::InvalidHistory)?;
            let request: FactSelectionRequest =
                serde_json::from_str(request_object.canonical_json())
                    .map_err(|_| StoreError::InvalidHistory)?;
            let selection: FactSelection = serde_json::from_str(selection_object.canonical_json())
                .map_err(|_| StoreError::InvalidHistory)?;
            request.validate().map_err(|_| StoreError::InvalidHistory)?;
            selection
                .validate_for(&request)
                .map_err(|_| StoreError::InvalidHistory)?;
            if selection.frontier.stream_ref != expected_stream {
                return Err(StoreError::InvalidHistory);
            }
            let sequence = selection.frontier.through_sequence;
            if sequence > snapshot.head_sequence() {
                return Err(StoreError::InvalidHistory);
            }
            let publication = snapshot
                .publications()
                .get(
                    usize::try_from(sequence.saturating_sub(1))
                        .map_err(|_| StoreError::InvalidHistory)?,
                )
                .ok_or(StoreError::InvalidHistory)?;
            if publication.proposal_set_ref() != &selection.frontier.head_ref {
                return Err(StoreError::InvalidHistory);
            }
            for (fact, provenance) in selection.facts.iter().zip(&selection.provenance) {
                self.validate_fact_provenance(&snapshot, &selection, fact, provenance)
                    .await?;
            }
        }
        Ok(())
    }

    async fn validate_fact_provenance(
        &self,
        snapshot: &crate::backend::RawFactSnapshot,
        selection: &FactSelection,
        fact: &mfm_facts::FactValue,
        provenance: &FactProvenance,
    ) -> Result<()> {
        if provenance.source_ref != fact.source_ref {
            return Err(StoreError::InvalidHistory);
        }
        let publication = snapshot
            .publications()
            .iter()
            .find(|publication| {
                publication.run_id() == &provenance.producer_run_id
                    && publication.run_sequence() == provenance.producer_record_sequence
                    && publication.proposal_set_ref() == &provenance.producer_record_ref
            })
            .filter(|publication| {
                publication.publication_sequence() <= selection.frontier.through_sequence
            })
            .ok_or(StoreError::InvalidHistory)?;
        let raw = self
            .inner
            .backend
            .load_complete_prefix(&provenance.producer_run_id, self.history_limit())
            .await
            .map_err(map_backend_error)?
            .ok_or(StoreError::InvalidHistory)?;
        let identity = self.inner.identity.clone();
        let producer = tokio::task::spawn_blocking(move || qualify_raw_prefix(&identity, raw))
            .await
            .map_err(|_| StoreError::InvalidHistory)??;
        let RunRecord::RunAdmitted(admission) = producer
            .frames()
            .first()
            .map(mfm_journal::single_trust::RunFrame::record)
            .ok_or(StoreError::InvalidHistory)?
        else {
            return Err(StoreError::InvalidHistory);
        };
        if admission.program_ref() != &provenance.producer_program_ref {
            return Err(StoreError::InvalidHistory);
        }
        let producer_frame = producer
            .frames()
            .get(
                usize::try_from(provenance.producer_record_sequence.saturating_sub(1))
                    .map_err(|_| StoreError::InvalidHistory)?,
            )
            .ok_or(StoreError::InvalidHistory)?;
        if producer.head_digest_at(provenance.producer_record_sequence)?
            != provenance.producer_head_ref
        {
            return Err(StoreError::InvalidHistory);
        }
        let RunRecord::StateConcluded(conclusion) = producer_frame.record() else {
            return Err(StoreError::InvalidHistory);
        };
        let recorded_publication = conclusion
            .fact_publication()
            .ok_or(StoreError::InvalidHistory)?;
        if recorded_publication.publication_sequence() != publication.publication_sequence()
            || recorded_publication.proposal_set_ref().value_ref()
                != &provenance.producer_record_ref
        {
            return Err(StoreError::InvalidHistory);
        }
        let object = producer_frame
            .objects()
            .iter()
            .find(|object| object.content_ref() == &provenance.producer_record_ref)
            .ok_or(StoreError::InvalidHistory)?;
        let proposals: FactProposalSet = serde_json::from_str(object.canonical_json())
            .map_err(|_| StoreError::InvalidHistory)?;
        proposals
            .validate()
            .map_err(|_| StoreError::InvalidHistory)?;
        if !proposals.facts.iter().any(|candidate| candidate == fact) {
            return Err(StoreError::InvalidHistory);
        }
        Ok(())
    }

    /// Appends the sole genesis frame through the branded admission port.
    pub async fn append_admission(
        &self,
        frame: mfm_journal::single_trust::RunFrame,
    ) -> Result<AppendDisposition> {
        self.history_port().append_admission(frame).await
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
            || !current.belongs_to_store(&self.inner.brand)
            || !reduced.belongs_to_store(&self.inner.brand)
        {
            return Err(StoreError::Identity);
        }
        let (prepared, objects) = self
            .store_select_fact_selection(current, prepared, objects)
            .await?;
        let mut candidate = prepare_access_from_current(
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
        candidate.append.bind_store(Arc::clone(&self.inner.brand));
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

    async fn store_select_fact_selection(
        &self,
        current: &QualifiedRun,
        prepared: mfm_journal::single_trust::StatePrepared,
        mut objects: Vec<mfm_journal::single_trust::ImmutableObject>,
    ) -> Result<(
        mfm_journal::single_trust::StatePrepared,
        Vec<mfm_journal::single_trust::ImmutableObject>,
    )> {
        let Some(request_ref) = prepared.fact_request().cloned() else {
            if prepared.fact_selection().is_some() {
                return Err(StoreError::InvalidRecord);
            }
            return Ok((prepared, objects));
        };
        if prepared.fact_selection().is_some() {
            return Err(StoreError::InvalidRecord);
        }
        let request_object = objects
            .iter()
            .find(|object| object.content_ref() == request_ref.value_ref())
            .ok_or(StoreError::InvalidRecord)?;
        let request: FactSelectionRequest = serde_json::from_str(request_object.canonical_json())
            .map_err(|_| StoreError::InvalidRecord)?;
        request.validate().map_err(|_| StoreError::InvalidRecord)?;
        let (selection_ref, selection_object) = self.select_prior_facts(current, &request).await?;
        let finalized = mfm_journal::single_trust::StatePrepared::new(
            prepared.occurrence().clone(),
            prepared.preparation_ordinal(),
            prepared.input().clone(),
            prepared.intent().clone(),
            Some(request_ref),
            Some(selection_ref),
            *prepared.mode(),
            prepared.binding().clone(),
            prepared.execution_binding_ref().clone(),
            prepared.replaces().cloned(),
            prepared.maximum_conclusion_bytes(),
        )
        .map_err(|_| StoreError::InvalidRecord)?;
        objects.push(selection_object);
        Ok((finalized, objects))
    }

    async fn select_prior_facts(
        &self,
        current: &QualifiedRun,
        request: &FactSelectionRequest,
    ) -> Result<(
        mfm_journal::single_trust::ValueRef,
        mfm_journal::single_trust::ImmutableObject,
    )> {
        let RunRecord::RunAdmitted(admission) = current
            .frames()
            .first()
            .map(mfm_journal::single_trust::RunFrame::record)
            .ok_or(StoreError::InvalidHistory)?
        else {
            return Err(StoreError::InvalidHistory);
        };
        if request
            .source_refs
            .iter()
            .any(|source| !admission.source_refs().contains(source))
        {
            return Err(StoreError::InvalidRecord);
        }
        let snapshot = self
            .inner
            .backend
            .load_facts()
            .await
            .map_err(map_backend_error)?;
        let Some(head) = snapshot.publications().last() else {
            return Err(StoreError::NotActionable);
        };
        let mut selected = BTreeMap::new();
        for publication in snapshot.publications().iter().rev() {
            let producer = self.load(publication.run_id()).await?;
            let RunRecord::RunAdmitted(producer_admission) = producer
                .frames()
                .first()
                .map(mfm_journal::single_trust::RunFrame::record)
                .ok_or(StoreError::InvalidHistory)?
            else {
                return Err(StoreError::InvalidHistory);
            };
            let frame = producer
                .frames()
                .get(
                    usize::try_from(publication.run_sequence().saturating_sub(1))
                        .map_err(|_| StoreError::InvalidHistory)?,
                )
                .ok_or(StoreError::InvalidHistory)?;
            let producer_head_ref = producer.head_digest_at(publication.run_sequence())?;
            let RunRecord::StateConcluded(conclusion) = frame.record() else {
                return Err(StoreError::InvalidHistory);
            };
            let Some(recorded_publication) = conclusion.fact_publication() else {
                return Err(StoreError::InvalidHistory);
            };
            if recorded_publication.publication_sequence() != publication.publication_sequence()
                || recorded_publication.proposal_set_ref().value_ref()
                    != publication.proposal_set_ref()
            {
                return Err(StoreError::InvalidHistory);
            }
            let object = frame
                .objects()
                .iter()
                .find(|object| object.content_ref() == publication.proposal_set_ref())
                .ok_or(StoreError::InvalidHistory)?;
            let proposals: FactProposalSet = serde_json::from_str(object.canonical_json())
                .map_err(|_| StoreError::InvalidHistory)?;
            proposals
                .validate()
                .map_err(|_| StoreError::InvalidHistory)?;
            for fact in proposals.facts {
                if request.source_refs.contains(&fact.source_ref)
                    && fact.subject_ref == request.subject_ref
                {
                    let provenance = FactProvenance::new(
                        fact.source_ref.clone(),
                        producer_admission.program_ref().clone(),
                        publication.run_id().clone(),
                        publication.run_sequence(),
                        publication.proposal_set_ref().clone(),
                        producer_head_ref.clone(),
                    )
                    .map_err(|_| StoreError::InvalidHistory)?;
                    selected
                        .entry(fact.source_ref.clone())
                        .or_insert((fact, provenance));
                }
            }
            if selected.len() == request.source_refs.len() {
                break;
            }
        }
        let (facts, provenance): (Vec<_>, Vec<_>) = request
            .source_refs
            .iter()
            .map(|source| selected.remove(source).ok_or(StoreError::NotActionable))
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .unzip();
        let frontier = FactSelectionFrontier::new(
            crate::single_trust::fact_stream_ref(
                self.identity().scope(),
                self.identity().epoch(),
                self.identity().tenant(),
            )?,
            snapshot.head_sequence(),
            head.proposal_set_ref().clone(),
        )
        .map_err(|_| StoreError::InvalidRecord)?;
        let selection = FactSelection::new(
            request,
            facts,
            provenance,
            FactCompleteness {
                through_sequence: snapshot.head_sequence(),
                complete: true,
            },
            frontier,
        )
        .map_err(|_| StoreError::InvalidRecord)?;
        let canonical = mfm_journal::single_trust::canonical_json(&selection)
            .map_err(|_| StoreError::InvalidRecord)?;
        let value_ref = ContentRef::new(
            mfm_facts::FactSelection::schema_id().map_err(|_| StoreError::InvalidRecord)?,
            raw_content_digest(canonical.as_bytes()),
        )
        .map_err(|_| StoreError::InvalidRecord)?;
        let value = ValueRef::new(value_ref.clone(), value_ref.clone());
        let object = mfm_journal::single_trust::ImmutableObject::new(
            mfm_ids::StableId::new("mfm.value").map_err(|_| StoreError::InvalidRecord)?,
            value_ref,
            canonical.as_str().to_owned(),
        )
        .map_err(|_| StoreError::InvalidRecord)?;
        Ok((value, object))
    }

    /// Builds one Store-owned conclusion append owner from the selected prefix.
    #[allow(clippy::too_many_arguments)]
    pub async fn prepare_conclusion(
        &self,
        run_id: &RunId,
        document: &mfm_program::single_trust::ProgramDocument,
        expected_sequence: u64,
        conclusion: mfm_journal::single_trust::StateConcluded,
        objects: Vec<mfm_journal::single_trust::ImmutableObject>,
        maximum_conclusion_bytes: u64,
    ) -> Result<crate::single_trust::PreparedConclusion> {
        let current = self.load(run_id).await?;
        self.prepare_conclusion_qualified(
            &current,
            document,
            expected_sequence,
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
        conclusion: mfm_journal::single_trust::StateConcluded,
        objects: Vec<mfm_journal::single_trust::ImmutableObject>,
        maximum_conclusion_bytes: u64,
    ) -> Result<crate::single_trust::PreparedConclusion> {
        if current.scope() != self.identity().scope()
            || current.epoch() != self.identity().epoch()
            || current.tenant() != self.identity().tenant()
            || !current.belongs_to_store(&self.inner.brand)
        {
            return Err(StoreError::Identity);
        }
        let append_request_id = conclusion_append_request_id(current.run_id(), expected_sequence)?;
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
        mut owner: crate::single_trust::PreparedConclusion,
    ) -> Result<ConclusionCommitOutcome> {
        if !owner.belongs_to(self.identity(), &self.inner.brand) {
            return Ok(ConclusionCommitOutcome::Rejected {
                owner,
                error: StoreError::Identity,
            });
        }
        if let Some(proposals_ref) = owner.frame().record().fact_proposals() {
            let object = match owner
                .frame()
                .objects()
                .iter()
                .find(|object| object.content_ref() == proposals_ref.value_ref())
            {
                Some(object) => object,
                None => {
                    return Ok(ConclusionCommitOutcome::Rejected {
                        owner,
                        error: StoreError::InvalidRecord,
                    })
                }
            };
            let proposals: mfm_facts::FactProposalSet =
                match serde_json::from_str(object.canonical_json()) {
                    Ok(proposals) => proposals,
                    Err(_) => {
                        return Ok(ConclusionCommitOutcome::Rejected {
                            owner,
                            error: StoreError::InvalidRecord,
                        })
                    }
                };
            if proposals.validate().is_err() {
                return Ok(ConclusionCommitOutcome::Rejected {
                    owner,
                    error: StoreError::InvalidRecord,
                });
            }
            if !proposals.is_empty() && owner.frame().record().fact_publication().is_none() {
                let snapshot = match self.inner.backend.load_facts().await {
                    Ok(snapshot) => snapshot,
                    Err(error) => {
                        return Ok(ConclusionCommitOutcome::Rejected {
                            owner,
                            error: map_backend_error(error),
                        })
                    }
                };
                let sequence = match snapshot.head_sequence().checked_add(1) {
                    Some(sequence) => sequence,
                    None => {
                        return Ok(ConclusionCommitOutcome::Rejected {
                            owner,
                            error: StoreError::Capacity,
                        })
                    }
                };
                if owner.bind_fact_publication(sequence).is_err() {
                    return Ok(ConclusionCommitOutcome::Rejected {
                        owner,
                        error: StoreError::InvalidRecord,
                    });
                }
            }
        }
        let frame = owner.frame().clone();
        let disposition = match self.history_port().append_frame(frame).await {
            Ok(disposition) => disposition,
            Err(StoreError::FactFrontierChanged) => {
                if let Err(error) = owner.rebind_fact_publication() {
                    return Ok(ConclusionCommitOutcome::Rejected { owner, error });
                }
                return Ok(ConclusionCommitOutcome::Rejected {
                    owner,
                    error: StoreError::FactFrontierChanged,
                });
            }
            Err(StoreError::Conflict | StoreError::NotActionable) => {
                return self.classify_conclusion_head_race(owner).await;
            }
            Err(error) => return Ok(ConclusionCommitOutcome::Rejected { owner, error }),
        };
        if matches!(disposition, AppendDisposition::AcknowledgementUnknown) {
            return Ok(ConclusionCommitOutcome::AcknowledgementUnknown(owner));
        }
        if matches!(disposition, AppendDisposition::StaleHead { .. }) {
            return self.classify_conclusion_head_race(owner).await;
        }
        Ok(ConclusionCommitOutcome::Disposition {
            disposition,
            frame: owner.frame().clone(),
        })
    }

    async fn classify_conclusion_head_race(
        &self,
        owner: crate::single_trust::PreparedConclusion,
    ) -> Result<ConclusionCommitOutcome> {
        let history = match self.load(owner.run_id()).await {
            Ok(history) => history,
            Err(error) => {
                return Ok(ConclusionCommitOutcome::Rejected { owner, error });
            }
        };
        let occurrence = match owner.frame().record() {
            RunRecord::StateConcluded(conclusion) => conclusion.occurrence().clone(),
            RunRecord::RunAdmitted(_) | RunRecord::StatePrepared(_) => {
                return Ok(ConclusionCommitOutcome::Rejected {
                    owner,
                    error: StoreError::InvalidRecord,
                })
            }
        };
        for frame in history.frames() {
            let RunRecord::StateConcluded(existing) = frame.record() else {
                continue;
            };
            if existing.occurrence() != &occurrence {
                continue;
            }
            let same = match same_conclusion_frame(owner.frame(), frame) {
                Ok(same) => same,
                Err(error) => return Ok(ConclusionCommitOutcome::Rejected { owner, error }),
            };
            return if same {
                Ok(ConclusionCommitOutcome::AlreadyConcludedSame { history })
            } else if let RunRecord::StateConcluded(
                mfm_journal::single_trust::StateConcluded::Access { preparation, .. },
            ) = owner.frame().record()
            {
                if !retained_preparation_ref(&history, &occurrence, preparation) {
                    Ok(ConclusionCommitOutcome::InvalidHistory { history })
                } else if history
                    .selected_preparation(&occurrence)
                    .is_some_and(|(_, selected)| &selected != preparation)
                {
                    Ok(ConclusionCommitOutcome::NoLongerSelected { history })
                } else {
                    Ok(ConclusionCommitOutcome::Conflict { history })
                }
            } else {
                Ok(ConclusionCommitOutcome::Conflict { history })
            };
        }

        let reduced = match self.reduce_qualified(&history, owner.document().clone()) {
            Ok(reduced) => reduced,
            Err(error) => return Ok(ConclusionCommitOutcome::Rejected { owner, error }),
        };
        match owner.frame().record() {
            RunRecord::StateConcluded(mfm_journal::single_trust::StateConcluded::Access {
                occurrence,
                preparation,
                ..
            }) => {
                if !matches!(
                    owner.expected_action(),
                    RunAction::WaitingPreparation {
                        occurrence: expected_occurrence,
                        preparation: expected_preparation,
                    } if expected_occurrence == occurrence && expected_preparation == preparation
                ) {
                    return Ok(ConclusionCommitOutcome::InvalidHistory { history });
                }
                let Some((_, selected)) = history.selected_preparation(occurrence) else {
                    return Ok(ConclusionCommitOutcome::InvalidHistory { history });
                };
                if &selected != preparation {
                    return if retained_preparation_ref(&history, occurrence, preparation) {
                        Ok(ConclusionCommitOutcome::NoLongerSelected { history })
                    } else {
                        Ok(ConclusionCommitOutcome::InvalidHistory { history })
                    };
                }
                if matches!(
                    reduced.action(),
                    RunAction::WaitingPreparation {
                        occurrence: reduced_occurrence,
                        preparation: reduced_preparation,
                    } if reduced_occurrence == occurrence && reduced_preparation == preparation
                ) {
                    Ok(ConclusionCommitOutcome::Conflict { history })
                } else {
                    Ok(ConclusionCommitOutcome::InvalidHistory { history })
                }
            }
            RunRecord::StateConcluded(mfm_journal::single_trust::StateConcluded::Pure {
                occurrence,
                ..
            }) => {
                if !matches!(
                    owner.expected_action(),
                    RunAction::ReadyPure {
                        occurrence: expected_occurrence,
                        ..
                    } if expected_occurrence == occurrence
                ) {
                    return Ok(ConclusionCommitOutcome::InvalidHistory { history });
                }
                if matches!(
                    reduced.action(),
                    RunAction::ReadyPure {
                        occurrence: reduced_occurrence,
                        ..
                    } if reduced_occurrence == occurrence
                ) {
                    Ok(ConclusionCommitOutcome::Conflict { history })
                } else {
                    Ok(ConclusionCommitOutcome::InvalidHistory { history })
                }
            }
            _ => Ok(ConclusionCommitOutcome::InvalidHistory { history }),
        }
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
            || !run.belongs_to_store(&self.inner.brand)
        {
            return Err(StoreError::Identity);
        }
        let mut reduced = reduce_qualified(run, document)?;
        reduced.bind_store(Arc::clone(&self.inner.brand));
        Ok(reduced)
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
            || !previous.belongs_to_store(&self.inner.brand)
            || !next.belongs_to_store(&self.inner.brand)
            || !reduced.belongs_to_store(&self.inner.brand)
        {
            return Err(StoreError::Identity);
        }
        let mut advanced = advance_reduced(reduced, previous, next, document)?;
        advanced.bind_store(Arc::clone(&self.inner.brand));
        Ok(advanced)
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
            || !predecessor.belongs_to_store(&self.inner.brand)
        {
            return Err(StoreError::Identity);
        }
        self.validate_frame_against_limits(&frame, predecessor.total_frame_bytes())?;
        let qualified = predecessor.clone().append_validated(frame)?;
        self.validate_qualified_limits(&qualified)?;
        Ok(qualified)
    }

    /// Qualifies one direct-new admission frame without a post-commit backend read.
    pub fn qualify_admission(
        &self,
        frame: mfm_journal::single_trust::RunFrame,
    ) -> Result<QualifiedRun> {
        if !frame.record().is_admission() {
            return Err(StoreError::InvalidRecord);
        }
        let mut qualified = QualifiedRun::qualify_prefix(
            self.identity().scope().clone(),
            self.identity().epoch(),
            self.identity().tenant().clone(),
            vec![frame],
        )?;
        qualified.bind_store(Arc::clone(&self.inner.brand));
        self.validate_qualified_limits(&qualified)?;
        Ok(qualified)
    }

    fn validate_frame_against_limits(
        &self,
        frame: &mfm_journal::single_trust::RunFrame,
        prior_bytes: usize,
    ) -> Result<usize> {
        let bytes = frame
            .canonical_bytes()
            .map_err(|_| StoreError::InvalidRecord)?
            .as_bytes()
            .len();
        if bytes > self.inner.limits.max_frame_bytes
            || prior_bytes
                .checked_add(bytes)
                .is_none_or(|total| total > self.inner.limits.max_run_frame_bytes)
        {
            return Err(StoreError::Capacity);
        }
        Ok(bytes)
    }

    fn validate_qualified_limits(&self, run: &QualifiedRun) -> Result<()> {
        if run.frames().len() > self.inner.limits.max_run_frames
            || run.total_frame_bytes() > self.inner.limits.max_run_frame_bytes
        {
            return Err(StoreError::Capacity);
        }
        let mut objects = std::collections::BTreeSet::new();
        for frame in run.frames() {
            let bytes = frame
                .canonical_bytes()
                .map_err(|_| StoreError::InvalidHistory)?
                .as_bytes()
                .len();
            if bytes > self.inner.limits.max_frame_bytes {
                return Err(StoreError::Capacity);
            }
            objects.extend(
                frame
                    .objects()
                    .iter()
                    .map(|object| object.content_ref().clone()),
            );
        }
        if objects.len() > self.inner.limits.max_run_objects {
            return Err(StoreError::Capacity);
        }
        Ok(())
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
        let permit = self
            .inner
            .prefix_ingress
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| StoreError::InvalidHistory)?;
        let raw = self
            .inner
            .backend
            .load_complete_prefix(run_id, self.limit())
            .await
            .map_err(map_backend_error)?
            .ok_or(StoreError::NotFound)?;
        let identity = self.inner.identity.clone();
        let mut qualified = qualify_prefix_on_blocking_job(identity, raw, permit).await?;
        qualified.bind_store(Arc::clone(&self.inner.brand));
        let store = OpenedStructuredStore {
            inner: Arc::clone(&self.inner),
        };
        store.validate_fact_frontiers(&qualified).await?;
        store.validate_qualified_limits(&qualified)?;
        Ok(qualified)
    }

    /// Appends the sole genesis frame after checking its Store and tenant identity.
    pub async fn append_admission(
        &self,
        frame: mfm_journal::single_trust::RunFrame,
    ) -> Result<AppendDisposition> {
        let mfm_journal::single_trust::RunRecord::RunAdmitted(admission) = frame.record() else {
            return Err(StoreError::InvalidRecord);
        };
        if frame.expected_sequence() != 1
            || admission.tenant_scope_id() != self.inner.identity.tenant()
            || admission.run_id() != frame.run_id()
            || admission.store_scope_id() != self.inner.identity.scope()
            || admission.store_epoch() != self.inner.identity.epoch()
        {
            return Err(StoreError::Identity);
        }
        self.append_frame(frame).await
    }

    async fn append_frame(
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
        let prior_bytes = current.as_ref().map_or(0, |prefix| {
            prefix
                .frames()
                .iter()
                .map(|stored| stored.frame_bytes().len())
                .sum()
        });
        let store = OpenedStructuredStore {
            inner: Arc::clone(&self.inner),
        };
        store.validate_frame_against_limits(&frame, prior_bytes)?;
        if let Some(prefix) = current.as_ref() {
            if prefix.frames().len() > self.inner.limits.max_run_frames {
                return Err(StoreError::Capacity);
            }
        }
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
        let store = OpenedStructuredStore {
            inner: Arc::clone(&self.inner),
        };
        store.validate_frame_against_limits(&frame, current.total_frame_bytes())?;
        if current.head_sequence() as usize >= self.inner.limits.max_run_frames {
            return Err(StoreError::Capacity);
        }
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
                    publication.proposal_set_ref().value_ref().clone(),
                )
            })
            .transpose()
            .map_err(map_backend_error)?;
        let fact_frontier = self.preparation_fact_frontier(&frame).await?;
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
        )
        .with_fact_frontier(fact_frontier);
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

    async fn preparation_fact_frontier(
        &self,
        frame: &mfm_journal::single_trust::RunFrame,
    ) -> Result<Option<u64>> {
        let RunRecord::StatePrepared(prepared) = frame.record() else {
            return Ok(None);
        };
        let (Some(request_ref), Some(selection_ref)) =
            (prepared.fact_request(), prepared.fact_selection())
        else {
            return Ok(None);
        };
        let request_object = frame
            .objects()
            .iter()
            .find(|object| object.content_ref() == request_ref.value_ref())
            .ok_or(StoreError::InvalidRecord)?;
        let selection_object = frame
            .objects()
            .iter()
            .find(|object| object.content_ref() == selection_ref.value_ref())
            .ok_or(StoreError::InvalidRecord)?;
        let request: FactSelectionRequest = serde_json::from_str(request_object.canonical_json())
            .map_err(|_| StoreError::InvalidRecord)?;
        let selection: FactSelection = serde_json::from_str(selection_object.canonical_json())
            .map_err(|_| StoreError::InvalidRecord)?;
        request.validate().map_err(|_| StoreError::InvalidRecord)?;
        selection
            .validate_for(&request)
            .map_err(|_| StoreError::InvalidRecord)?;
        let expected_stream = crate::single_trust::fact_stream_ref(
            self.inner.identity.scope(),
            self.inner.identity.epoch(),
            self.inner.identity.tenant(),
        )?;
        if selection.frontier.stream_ref != expected_stream {
            return Err(StoreError::InvalidRecord);
        }
        let snapshot = self
            .inner
            .backend
            .load_facts()
            .await
            .map_err(map_backend_error)?;
        let frontier_sequence = selection.frontier.through_sequence;
        if frontier_sequence > snapshot.head_sequence() {
            return Err(StoreError::InvalidHistory);
        }
        let publication = snapshot
            .publications()
            .get(
                usize::try_from(frontier_sequence.saturating_sub(1))
                    .map_err(|_| StoreError::InvalidHistory)?,
            )
            .ok_or(StoreError::FactFrontierChanged)?;
        if publication.proposal_set_ref() != &selection.frontier.head_ref {
            return Err(StoreError::InvalidHistory);
        }
        let store = OpenedStructuredStore {
            inner: Arc::clone(&self.inner),
        };
        for (fact, provenance) in selection.facts.iter().zip(&selection.provenance) {
            store
                .validate_fact_provenance(&snapshot, &selection, fact, provenance)
                .await?;
        }
        if frontier_sequence != snapshot.head_sequence() {
            return Err(StoreError::FactFrontierChanged);
        }
        Ok(Some(frontier_sequence))
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
        let mut snapshot = ConfigurationSnapshot::from_revisions(revisions)?;
        snapshot.bind_store(Arc::clone(&self.inner.brand));
        Ok(snapshot)
    }

    /// Prepares one validated configuration successor from a fixed snapshot without reading the
    /// retained stream again.
    pub fn prepare_append(
        &self,
        current: &ConfigurationSnapshot,
        append_request_id: AppendRequestId,
        canonical_json: String,
    ) -> Result<PreparedConfigurationWrite> {
        if !current.belongs_to_store(&self.inner.brand) {
            return Err(StoreError::Identity);
        }
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

async fn qualify_prefix_on_blocking_job(
    identity: StructuredStoreIdentity,
    raw: RawRunPrefix,
    permit: OwnedSemaphorePermit,
) -> Result<QualifiedRun> {
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        qualify_raw_prefix(&identity, raw)
    })
    .await
    .map_err(|_| StoreError::InvalidHistory)?
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
        BackendError::FactFrontierChanged => StoreError::FactFrontierChanged,
        BackendError::StaleHead => StoreError::NotActionable,
        BackendError::Capacity => StoreError::Capacity,
        BackendError::AcknowledgementUnknown
        | BackendError::Unsupported
        | BackendError::Storage => StoreError::InvalidHistory,
    }
}

fn conclusion_append_request_id(run_id: &RunId, expected_sequence: u64) -> Result<AppendRequestId> {
    let successor_sequence = expected_sequence
        .checked_add(1)
        .ok_or(StoreError::Capacity)?;
    AppendRequestId::new(format!(
        "conclusion-{}-{}",
        short_stable_id_fragment(run_id.as_str(), 96),
        successor_sequence
    ))
    .map_err(|_| StoreError::InvalidRecord)
}

fn same_conclusion_frame(
    candidate_frame: &mfm_journal::single_trust::RunFrame,
    existing_frame: &mfm_journal::single_trust::RunFrame,
) -> Result<bool> {
    let (RunRecord::StateConcluded(candidate), RunRecord::StateConcluded(existing)) =
        (candidate_frame.record(), existing_frame.record())
    else {
        return Ok(false);
    };
    let candidate_record = candidate
        .clone()
        .with_fact_publication(None)
        .map_err(|_| StoreError::InvalidHistory)?;
    let existing_record = existing
        .clone()
        .with_fact_publication(None)
        .map_err(|_| StoreError::InvalidHistory)?;
    Ok(
        candidate_record == existing_record
            && candidate_frame.objects() == existing_frame.objects(),
    )
}

fn retained_preparation_ref(
    history: &QualifiedRun,
    occurrence: &mfm_journal::single_trust::SequentialControlAddress,
    target: &mfm_journal::single_trust::PreparationRef,
) -> bool {
    history.frames().iter().enumerate().any(|(index, frame)| {
        if !matches!(
            frame.record(),
            RunRecord::StatePrepared(prepared) if prepared.occurrence() == occurrence
        ) {
            return false;
        }
        PreparationRef::new(
            history.run_id().clone(),
            frame.expected_sequence(),
            index as u32,
        ) == *target
    })
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU8, Ordering};

    use super::*;
    use crate::single_trust::ConfigurationAppendDisposition;
    use mfm_canonical::raw_content_digest;
    use mfm_ids::{DigestAlgorithm, DigestBytes, SchemaId, StableId};
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Serialize, Deserialize, mfm_program_derive::MfmValue)]
    #[serde(deny_unknown_fields)]
    struct BackendValue {
        value: u64,
    }

    fn backend_catalog_builder() -> mfm_program::ProgramCatalogBuilder {
        let mut builder = ProgramCatalog::builder();
        builder
            .register_value::<BackendValue>()
            .expect("backend value");
        builder
    }

    fn identity() -> StructuredStoreIdentity {
        StructuredStoreIdentity::new(
            StoreScopeId::new("mfm.store_scope.v1:0123456789abcdef0123456789abcdef")
                .expect("scope"),
            StoreEpoch::new(1),
            TenantScopeId::new("mfm.tenant_scope.v1:0123456789abcdef0123456789abcdef")
                .expect("tenant"),
        )
    }

    fn admission_frame(identity: &StructuredStoreIdentity) -> mfm_journal::single_trust::RunFrame {
        let contract = mfm_program::nominal_contract_ref::<BackendValue>().expect("contract");
        let value_bytes = br#"{"value":1}"#;
        let value = ContentRef::new(
            contract.schema_id().clone(),
            raw_content_digest(value_bytes),
        )
        .expect("value");
        let run_id = RunId::parse(
            "run:sha256-jcs-v1:4123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        )
        .expect("run");
        let admitted = mfm_journal::single_trust::RunAdmitted::new(
            identity.scope().clone(),
            identity.epoch(),
            run_id.clone(),
            identity.tenant().clone(),
            mfm_ids::StableId::new("mfm.test.backend-limit-entry").expect("entry"),
            contract.clone(),
            mfm_journal::single_trust::ValueRef::new(contract.clone(), value.clone()),
            contract.clone(),
            Vec::new(),
        )
        .expect("admission");
        mfm_journal::single_trust::RunFrame::new(
            run_id,
            identity.scope().clone(),
            identity.epoch(),
            1,
            AppendRequestId::new("backend-limit-admission-0123456789").expect("request"),
            mfm_journal::single_trust::RunRecord::RunAdmitted(admitted),
            vec![mfm_journal::single_trust::ImmutableObject::new(
                mfm_ids::StableId::new("mfm.value").expect("object"),
                value,
                String::from_utf8(value_bytes.to_vec()).expect("value bytes"),
            )
            .expect("object")],
        )
        .expect("frame")
    }

    fn fact_source(seed: u8) -> ContentRef {
        let schema = SchemaId::new(
            "mfm.test.fact-source",
            "1",
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([seed; 32]),
        )
        .expect("source schema");
        ContentRef::new(schema, raw_content_digest(&[seed])).expect("source")
    }

    fn sourced_admission_frame(
        identity: &StructuredStoreIdentity,
        source: ContentRef,
    ) -> mfm_journal::single_trust::RunFrame {
        let base = admission_frame(identity);
        let RunRecord::RunAdmitted(admitted) = base.record() else {
            panic!("expected admission")
        };
        let admission = mfm_journal::single_trust::RunAdmitted::new(
            identity.scope().clone(),
            identity.epoch(),
            base.run_id().clone(),
            identity.tenant().clone(),
            admitted.entry_point_id().clone(),
            admitted.program_ref().clone(),
            admitted.admitted_context().clone(),
            admitted.configuration_ref().clone(),
            vec![source],
        )
        .expect("sourced admission");
        mfm_journal::single_trust::RunFrame::new(
            base.run_id().clone(),
            identity.scope().clone(),
            identity.epoch(),
            1,
            AppendRequestId::new("backend-fact-admission-0123456789").expect("request"),
            RunRecord::RunAdmitted(admission),
            base.objects().to_vec(),
        )
        .expect("sourced frame")
    }

    #[test]
    fn outer_capacity_accepts_exact_ceiling_and_rejects_each_plus_one() {
        let exact = StoreWorkLimits::default();
        assert_eq!(exact.validate(), Ok(()));

        let cases = [
            StoreWorkLimits::new(
                mfm_journal::single_trust::MAX_FRAME_BYTES + 1,
                mfm_journal::single_trust::MAX_RUN_FRAMES,
                mfm_journal::single_trust::MAX_RUN_OBJECTS,
                mfm_journal::single_trust::MAX_RUN_FRAME_BYTES,
            ),
            StoreWorkLimits::new(
                mfm_journal::single_trust::MAX_FRAME_BYTES,
                mfm_journal::single_trust::MAX_RUN_FRAMES + 1,
                mfm_journal::single_trust::MAX_RUN_OBJECTS,
                mfm_journal::single_trust::MAX_RUN_FRAME_BYTES,
            ),
            StoreWorkLimits::new(
                mfm_journal::single_trust::MAX_FRAME_BYTES,
                mfm_journal::single_trust::MAX_RUN_FRAMES,
                mfm_journal::single_trust::MAX_RUN_OBJECTS + 1,
                mfm_journal::single_trust::MAX_RUN_FRAME_BYTES,
            ),
            StoreWorkLimits::new(
                mfm_journal::single_trust::MAX_FRAME_BYTES,
                mfm_journal::single_trust::MAX_RUN_FRAMES,
                mfm_journal::single_trust::MAX_RUN_OBJECTS,
                mfm_journal::single_trust::MAX_RUN_FRAME_BYTES + 1,
            ),
        ];
        assert!(cases.iter().all(|limits| limits.validate().is_err()));
        eprintln!(
            "capacity-envelope store frame_bytes={} frames={} objects={} run_frame_bytes={}",
            exact.max_frame_bytes(),
            exact.max_run_frames(),
            exact.max_run_objects(),
            exact.max_run_frame_bytes(),
        );
    }

    fn proposal(seed: u8) -> (ValueRef, mfm_journal::single_trust::ImmutableObject) {
        let source = fact_source(seed);
        let subject = fact_source(seed.saturating_add(1));
        let canonical_value = format!(r#"{{"value":{seed}}}"#);
        let value_ref =
            mfm_facts::content_ref_for_value(canonical_value.as_bytes()).expect("fact value ref");
        let fact = mfm_facts::FactValue::new(
            source,
            subject,
            mfm_ids::StableId::new(format!("mfm.test.fact-{seed}")).expect("fact id"),
            value_ref,
            canonical_value,
        )
        .expect("fact");
        let proposals = mfm_facts::FactProposalSet::new(vec![fact]).expect("proposals");
        let canonical =
            mfm_journal::single_trust::canonical_json(&proposals).expect("proposal canonical");
        let proposal_ref = ContentRef::new(
            mfm_facts::FactProposalSet::schema_id().expect("proposal schema"),
            raw_content_digest(canonical.as_bytes()),
        )
        .expect("proposal ref");
        let object = mfm_journal::single_trust::ImmutableObject::new(
            mfm_ids::StableId::new("mfm.value").expect("object type"),
            proposal_ref.clone(),
            canonical.as_str().to_owned(),
        )
        .expect("proposal object");
        (ValueRef::new(proposal_ref.clone(), proposal_ref), object)
    }

    struct InjectingFactBackend {
        inner: Arc<MemoryStructuredBackend>,
        injected: AtomicU8,
    }

    impl InjectingFactBackend {
        fn new(inner: Arc<MemoryStructuredBackend>) -> Self {
            Self {
                inner,
                injected: AtomicU8::new(0),
            }
        }
    }

    impl StructuredStoreBackend for InjectingFactBackend {
        fn identity(&self) -> StructuredStoreIdentity {
            self.inner.identity()
        }

        fn load_complete_prefix<'a>(
            &'a self,
            run_id: &'a RunId,
            limit: RawHistoryLoadLimit,
        ) -> BackendFuture<'a, Option<RawRunPrefix>> {
            self.inner.load_complete_prefix(run_id, limit)
        }

        fn compare_and_append<'a>(
            &'a self,
            command: &'a BackendAppendCommand<'a>,
        ) -> BackendFuture<'a, BackendAppendOutcome> {
            let inner = Arc::clone(&self.inner);
            let injection_index = command
                .fact_publication()
                .map_or(2, |_| self.injected.fetch_add(1, Ordering::SeqCst));
            let inject = injection_index < 2;
            Box::pin(async move {
                if inject {
                    let identity = inner.identity();
                    let run_id = RunId::parse(match injection_index {
                        0 => {
                            "run:sha256-jcs-v1:5123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
                        }
                        _ => {
                            "run:sha256-jcs-v1:5223456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
                        }
                    })
                    .map_err(|_| BackendError::Storage)?;
                    let publication_sequence = inner.load_facts().await?.head_sequence() + 1;
                    let bytes = br#"{}"#;
                    let frame_digest = raw_content_digest(bytes);
                    let head_digest = raw_content_digest(b"injected-head");
                    let request = AppendRequestId::new("injected-fact-0123456789abcdef")
                        .map_err(|_| BackendError::Storage)?;
                    let schema = SchemaId::new(
                        "mfm.test.injected-proposals",
                        "1",
                        DigestAlgorithm::Sha256JcsV1,
                        DigestBytes::from_array([9; 32]),
                    )
                    .map_err(|_| BackendError::Storage)?;
                    let proposal_ref = ContentRef::new(schema, raw_content_digest(b"injected"))
                        .map_err(|_| BackendError::Storage)?;
                    let publication = RawFactPublication::new(
                        publication_sequence,
                        run_id.clone(),
                        1,
                        proposal_ref,
                    )?;
                    let injected = BackendAppendCommand::new(
                        &identity,
                        &run_id,
                        1,
                        &request,
                        bytes,
                        &frame_digest,
                        &head_digest,
                        None,
                        true,
                        Some(&publication),
                    );
                    if !matches!(
                        inner.compare_and_append(&injected).await?,
                        BackendAppendOutcome::NewlyCommitted
                    ) {
                        return Err(BackendError::Conflict);
                    }
                }
                inner.compare_and_append(command).await
            })
        }

        fn load_configuration<'a>(&'a self) -> BackendFuture<'a, Vec<RawConfigurationRevision>> {
            self.inner.load_configuration()
        }

        fn compare_and_append_configuration<'a>(
            &'a self,
            command: &'a ConfigurationAppendCommand<'a>,
        ) -> BackendFuture<'a, BackendConfigurationOutcome> {
            self.inner.compare_and_append_configuration(command)
        }

        fn load_facts<'a>(&'a self) -> BackendFuture<'a, RawFactSnapshot> {
            self.inner.load_facts()
        }

        fn audit_run_ids<'a>(&'a self) -> BackendFuture<'a, Vec<RunId>> {
            self.inner.audit_run_ids()
        }
    }

    #[tokio::test]
    async fn opened_limits_reject_frame_before_backend_ingress() {
        let identity = identity();
        let frame = admission_frame(&identity);
        let frame_bytes = frame
            .canonical_bytes()
            .expect("canonical frame")
            .as_bytes()
            .len();
        let root_contract = match frame.record() {
            mfm_journal::single_trust::RunRecord::RunAdmitted(admitted) => {
                admitted.admitted_context().contract_ref().clone()
            }
            _ => panic!("expected admission"),
        };
        let document = mfm_program::single_trust::ProgramDocument::new(
            mfm_ids::StableId::new("mfm.test.backend-limit-entry").expect("entry"),
            root_contract.clone(),
            root_contract,
            Vec::new(),
        )
        .expect("document");
        let (catalog, _) = backend_catalog_builder().finish(document).expect("catalog");
        let backend = Arc::new(MemoryStructuredBackend::new(identity.clone()));
        let opened = StructuredStore::open(
            backend.clone(),
            identity.clone(),
            catalog,
            StoreWorkLimits::new(
                frame_bytes.saturating_sub(1),
                mfm_journal::single_trust::MAX_RUN_FRAMES,
                mfm_journal::single_trust::MAX_RUN_OBJECTS,
                mfm_journal::single_trust::MAX_RUN_FRAME_BYTES,
            ),
        )
        .await
        .expect("opened store");
        assert_eq!(
            opened.append_admission(frame).await,
            Err(StoreError::Capacity)
        );
        assert!(backend
            .load_complete_prefix(
                &RunId::parse(
                    "run:sha256-jcs-v1:4123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
                )
                .expect("run"),
                RawHistoryLoadLimit::new(
                    mfm_journal::single_trust::MAX_RUN_FRAMES,
                    mfm_journal::single_trust::MAX_RUN_FRAME_BYTES,
                ),
            )
            .await
            .expect("backend load")
            .is_none());
    }

    #[tokio::test]
    async fn admission_port_rejects_non_genesis_frames_before_backend_ingress() {
        let identity = identity();
        let admission = admission_frame(&identity);
        let (contract_ref, value_ref, value_json) = match admission.record() {
            mfm_journal::single_trust::RunRecord::RunAdmitted(admitted) => {
                let value = admitted.admitted_context().value_ref().clone();
                (
                    admitted.admitted_context().contract_ref().clone(),
                    value,
                    r#"{"value":1}"#,
                )
            }
            _ => panic!("expected admission"),
        };
        let non_genesis = mfm_journal::single_trust::RunFrame::new(
            admission.run_id().clone(),
            identity.scope().clone(),
            identity.epoch(),
            2,
            AppendRequestId::new("backend-admission-port-0123456789").expect("request"),
            mfm_journal::single_trust::RunRecord::StateConcluded(
                mfm_journal::single_trust::StateConcluded::Pure {
                    occurrence: mfm_journal::single_trust::SequentialControlAddress::new(
                        0,
                        Vec::new(),
                    )
                    .expect("occurrence"),
                    outcome: mfm_journal::single_trust::StateOutcome::Success(
                        mfm_journal::single_trust::ValueRef::new(
                            contract_ref.clone(),
                            value_ref.clone(),
                        ),
                    ),
                    fact_proposals: None,
                    fact_publication: None,
                },
            ),
            vec![mfm_journal::single_trust::ImmutableObject::new(
                mfm_ids::StableId::new("mfm.value").expect("object"),
                value_ref,
                value_json.to_owned(),
            )
            .expect("object")],
        )
        .expect("non-genesis frame");
        let (catalog, _) = backend_catalog_builder()
            .finish(
                mfm_program::single_trust::ProgramDocument::new(
                    mfm_ids::StableId::new("mfm.test.backend-limit-entry").expect("entry"),
                    contract_ref.clone(),
                    contract_ref,
                    Vec::new(),
                )
                .expect("document"),
            )
            .expect("catalog");
        let backend = Arc::new(MemoryStructuredBackend::new(identity.clone()));
        let opened = StructuredStore::open(
            backend.clone(),
            identity,
            catalog,
            StoreWorkLimits::default(),
        )
        .await
        .expect("opened store");
        assert_eq!(
            opened.append_admission(non_genesis).await,
            Err(StoreError::InvalidRecord)
        );
        assert!(backend
            .load_complete_prefix(
                admission.run_id(),
                RawHistoryLoadLimit::new(
                    mfm_journal::single_trust::MAX_RUN_FRAMES,
                    mfm_journal::single_trust::MAX_RUN_FRAME_BYTES,
                ),
            )
            .await
            .expect("backend load")
            .is_none());
    }

    #[tokio::test]
    async fn qualified_history_and_reduction_cannot_cross_store_openings() {
        let identity = identity();
        let frame = admission_frame(&identity);
        let (contract, value_ref) = match frame.record() {
            mfm_journal::single_trust::RunRecord::RunAdmitted(admitted) => (
                admitted.admitted_context().contract_ref().clone(),
                admitted.admitted_context().value_ref().clone(),
            ),
            _ => panic!("expected admission"),
        };
        let (catalog, _) = backend_catalog_builder()
            .finish(
                mfm_program::single_trust::ProgramDocument::new(
                    mfm_ids::StableId::new("mfm.test.backend-limit-entry").expect("entry"),
                    contract.clone(),
                    contract.clone(),
                    Vec::new(),
                )
                .expect("document"),
            )
            .expect("catalog");
        let backend = Arc::new(MemoryStructuredBackend::new(identity.clone()));
        let first = StructuredStore::open(
            backend.clone(),
            identity.clone(),
            catalog.clone(),
            StoreWorkLimits::default(),
        )
        .await
        .expect("first opening");
        let second = StructuredStore::open(backend, identity, catalog, StoreWorkLimits::default())
            .await
            .expect("second opening");
        assert!(!first.same_open(&second));
        first
            .append_admission(frame.clone())
            .await
            .expect("admission");
        let first_run = first.load(frame.run_id()).await.expect("first run");
        let second_run = second.load(frame.run_id()).await.expect("second run");
        assert_eq!(first_run.frames(), second_run.frames());
        let foreign_candidate = mfm_journal::single_trust::RunFrame::new(
            frame.run_id().clone(),
            frame.store_scope_id().clone(),
            frame.store_epoch(),
            2,
            AppendRequestId::new("backend-store-brand-0123456789").expect("request"),
            mfm_journal::single_trust::RunRecord::StateConcluded(
                mfm_journal::single_trust::StateConcluded::Pure {
                    occurrence: mfm_journal::single_trust::SequentialControlAddress::new(
                        0,
                        Vec::new(),
                    )
                    .expect("occurrence"),
                    outcome: mfm_journal::single_trust::StateOutcome::Success(
                        mfm_journal::single_trust::ValueRef::new(
                            contract.clone(),
                            value_ref.clone(),
                        ),
                    ),
                    fact_proposals: None,
                    fact_publication: None,
                },
            ),
            vec![mfm_journal::single_trust::ImmutableObject::new(
                mfm_ids::StableId::new("mfm.value").expect("object"),
                value_ref,
                r#"{"value":1}"#.to_owned(),
            )
            .expect("object")],
        )
        .expect("candidate");
        assert!(matches!(
            second.qualify_appended(&first_run, foreign_candidate),
            Err(StoreError::Identity)
        ));
    }

    #[tokio::test]
    async fn prior_fact_scan_reads_state_proposal_publications() {
        let identity = identity();
        let source = fact_source(40);
        let admission = sourced_admission_frame(&identity, source.clone());
        let (entry, contract) = match admission.record() {
            RunRecord::RunAdmitted(admitted) => (
                admitted.entry_point_id().clone(),
                admitted.admitted_context().contract_ref().clone(),
            ),
            _ => panic!("expected admission"),
        };
        let document = mfm_program::single_trust::ProgramDocument::new(
            entry,
            contract.clone(),
            contract,
            Vec::new(),
        )
        .expect("document");
        let (catalog, _) = backend_catalog_builder().finish(document).expect("catalog");
        let opened =
            StructuredStore::open_memory(identity.clone(), catalog, StoreWorkLimits::default())
                .expect("opened store");
        opened
            .append_admission(admission.clone())
            .await
            .expect("admission");

        let subject = fact_source(41);
        let value_ref = mfm_facts::content_ref_for_value(br#"{"value":1}"#).expect("fact value");
        let fact = mfm_facts::FactValue::new(
            source.clone(),
            subject.clone(),
            mfm_ids::StableId::new("mfm.test.fact").expect("fact id"),
            value_ref,
            r#"{"value":1}"#.to_owned(),
        )
        .expect("fact");
        let proposals = mfm_facts::FactProposalSet::new(vec![fact]).expect("proposals");
        let canonical =
            mfm_journal::single_trust::canonical_json(&proposals).expect("proposal canonical");
        let proposal_content_ref = ContentRef::new(
            mfm_facts::FactProposalSet::schema_id().expect("proposal schema"),
            raw_content_digest(canonical.as_bytes()),
        )
        .expect("proposal ref");
        let proposal_value =
            ValueRef::new(proposal_content_ref.clone(), proposal_content_ref.clone());
        let proposal_object = mfm_journal::single_trust::ImmutableObject::new(
            mfm_ids::StableId::new("mfm.value").expect("object type"),
            proposal_content_ref.clone(),
            canonical.as_str().to_owned(),
        )
        .expect("proposal object");
        let RunRecord::RunAdmitted(admitted) = admission.record() else {
            panic!("expected admission")
        };
        let outcome = admitted.admitted_context().clone();
        let conclusion = mfm_journal::single_trust::StateConcluded::Pure {
            occurrence: mfm_journal::single_trust::SequentialControlAddress::new(0, Vec::new())
                .expect("occurrence"),
            outcome: mfm_journal::single_trust::StateOutcome::Success(outcome.clone()),
            fact_proposals: Some(proposal_value.clone()),
            fact_publication: Some(
                mfm_journal::single_trust::FactPublication::new(1, proposal_value)
                    .expect("publication"),
            ),
        };
        let conclusion_frame = mfm_journal::single_trust::RunFrame::new(
            admission.run_id().clone(),
            identity.scope().clone(),
            identity.epoch(),
            2,
            AppendRequestId::new("backend-fact-conclusion-0123456789").expect("request"),
            RunRecord::StateConcluded(conclusion),
            vec![admission.objects()[0].clone(), proposal_object],
        )
        .expect("conclusion frame");
        assert_eq!(
            opened
                .history_port()
                .append_frame(conclusion_frame)
                .await
                .expect("publication append"),
            AppendDisposition::NewlyCommitted { sequence: 2 }
        );

        let current = opened.load(admission.run_id()).await.expect("current");
        let request = mfm_facts::FactSelectionRequest::new(
            mfm_ids::StableId::new("mfm.test.request").expect("request id"),
            vec![source],
            subject,
        )
        .expect("selection request");
        let (selection_ref, selection_object) = opened
            .select_prior_facts(&current, &request)
            .await
            .expect("select prior facts");
        assert_eq!(
            selection_ref.contract_ref().schema_id(),
            &mfm_facts::FactSelection::schema_id().expect("selection schema")
        );
        let selection: mfm_facts::FactSelection =
            serde_json::from_str(selection_object.canonical_json()).expect("selection");
        selection
            .validate_for(&request)
            .expect("selection validation");
        assert_eq!(selection.facts.len(), 1);
        assert_eq!(selection.frontier.through_sequence, 1);
    }

    #[tokio::test]
    async fn conclusion_rebinds_only_its_fact_coordinate_after_frontier_race() {
        let identity = identity();
        let base = admission_frame(&identity);
        let (entry, contract, context, configuration) = match base.record() {
            RunRecord::RunAdmitted(admitted) => (
                admitted.entry_point_id().clone(),
                admitted.admitted_context().contract_ref().clone(),
                admitted.admitted_context().clone(),
                admitted.configuration_ref().clone(),
            ),
            _ => panic!("expected admission"),
        };
        let state = mfm_program::single_trust::StateDeclaration::new(
            mfm_journal::single_trust::SequentialControlAddress::new(0, Vec::new())
                .expect("occurrence"),
            fact_source(80),
            contract.clone(),
            contract.clone(),
            None,
            mfm_program::single_trust::ExecutionMode::Pure,
            true,
        )
        .expect("state");
        let document = mfm_program::single_trust::ProgramDocument::new(
            entry,
            contract.clone(),
            contract,
            vec![mfm_program::Declaration::State(Box::new(state))],
        )
        .expect("document");
        let program_ref = document.program_ref().expect("program ref");
        let admission = mfm_journal::single_trust::RunFrame::new(
            base.run_id().clone(),
            identity.scope().clone(),
            identity.epoch(),
            1,
            AppendRequestId::new("backend-rebind-admission-0123456789").expect("request"),
            RunRecord::RunAdmitted(
                mfm_journal::single_trust::RunAdmitted::new(
                    identity.scope().clone(),
                    identity.epoch(),
                    base.run_id().clone(),
                    identity.tenant().clone(),
                    match base.record() {
                        RunRecord::RunAdmitted(admitted) => admitted.entry_point_id().clone(),
                        _ => unreachable!(),
                    },
                    program_ref,
                    context.clone(),
                    configuration,
                    Vec::new(),
                )
                .expect("admission"),
            ),
            vec![base.objects()[0].clone()],
        )
        .expect("admission frame");
        let backend = Arc::new(MemoryStructuredBackend::new(identity.clone()));
        let injecting = Arc::new(InjectingFactBackend::new(Arc::clone(&backend)));
        let (catalog, _) = backend_catalog_builder()
            .finish(document.clone())
            .expect("catalog");
        let opened = StructuredStore::open(
            injecting,
            identity.clone(),
            catalog,
            StoreWorkLimits::default(),
        )
        .await
        .expect("opened store");
        opened
            .append_admission(admission.clone())
            .await
            .expect("admission");
        let current = opened.load(admission.run_id()).await.expect("current");
        let reduced = opened
            .reduce_qualified(&current, document.clone())
            .expect("reduced");
        let (proposal_value, proposal_object) = proposal(90);
        let conclusion = mfm_journal::single_trust::StateConcluded::Pure {
            occurrence: mfm_journal::single_trust::SequentialControlAddress::new(0, Vec::new())
                .expect("occurrence"),
            outcome: mfm_journal::single_trust::StateOutcome::Success(context.clone()),
            fact_proposals: Some(proposal_value.clone()),
            fact_publication: None,
        };
        let owner = opened
            .prepare_conclusion_qualified_with_reduced(
                &current,
                &document,
                &reduced,
                1,
                conclusion,
                vec![base.objects()[0].clone(), proposal_object],
                mfm_journal::single_trust::MAX_FRAME_BYTES as u64,
            )
            .expect("conclusion owner");
        let original_append_request_id = owner.frame().append_request_id().clone();
        let owner = match opened.commit_conclusion(owner).await.expect("first commit") {
            ConclusionCommitOutcome::Rejected {
                owner,
                error: StoreError::FactFrontierChanged,
            } => {
                assert!(owner.frame().record().fact_publication().is_none());
                assert_ne!(
                    owner.frame().append_request_id(),
                    &original_append_request_id
                );
                owner
            }
            other => panic!("unexpected first commit: {other:?}"),
        };
        let first_rebound_append_request_id = owner.frame().append_request_id().clone();
        let owner = match opened
            .commit_conclusion(owner)
            .await
            .expect("second commit")
        {
            ConclusionCommitOutcome::Rejected {
                owner,
                error: StoreError::FactFrontierChanged,
            } => {
                assert!(owner.frame().record().fact_publication().is_none());
                assert_ne!(
                    owner.frame().append_request_id(),
                    &first_rebound_append_request_id
                );
                owner
            }
            other => panic!("unexpected second commit: {other:?}"),
        };
        let (disposition, frame) = match opened
            .commit_conclusion(owner)
            .await
            .expect("rebound commit")
        {
            ConclusionCommitOutcome::Disposition { disposition, frame } => (disposition, frame),
            other => panic!("unexpected rebound commit: {other:?}"),
        };
        assert_eq!(
            disposition,
            AppendDisposition::NewlyCommitted { sequence: 2 }
        );
        assert_eq!(
            frame
                .record()
                .fact_publication()
                .map(|publication| publication.publication_sequence()),
            Some(3)
        );
        assert_eq!(
            frame
                .record()
                .fact_publication()
                .map(|publication| publication.proposal_set_ref()),
            Some(proposal_value.clone()).as_ref()
        );
        let facts = opened.split().into_parts().3.facts().await.expect("facts");
        assert_eq!(facts.head_sequence(), 3);
        assert_eq!(facts.publications().len(), 3);
    }

    #[tokio::test]
    async fn conclusion_head_race_classifies_same_and_conflicting_semantics() {
        let identity = identity();
        let base = admission_frame(&identity);
        let (entry, contract, context, configuration) = match base.record() {
            RunRecord::RunAdmitted(admitted) => (
                admitted.entry_point_id().clone(),
                admitted.admitted_context().contract_ref().clone(),
                admitted.admitted_context().clone(),
                admitted.configuration_ref().clone(),
            ),
            _ => panic!("expected admission"),
        };
        let occurrence = mfm_journal::single_trust::SequentialControlAddress::new(0, Vec::new())
            .expect("occurrence");
        let state = mfm_program::single_trust::StateDeclaration::new(
            occurrence.clone(),
            fact_source(81),
            contract.clone(),
            contract.clone(),
            None,
            mfm_program::single_trust::ExecutionMode::Pure,
            true,
        )
        .expect("state");
        let document = mfm_program::single_trust::ProgramDocument::new(
            entry,
            contract.clone(),
            contract,
            vec![mfm_program::Declaration::State(Box::new(state))],
        )
        .expect("document");
        let program_ref = document.program_ref().expect("program ref");
        let admission = mfm_journal::single_trust::RunFrame::new(
            base.run_id().clone(),
            identity.scope().clone(),
            identity.epoch(),
            1,
            AppendRequestId::new("classification-admission-0123456789").expect("request"),
            RunRecord::RunAdmitted(
                mfm_journal::single_trust::RunAdmitted::new(
                    identity.scope().clone(),
                    identity.epoch(),
                    base.run_id().clone(),
                    identity.tenant().clone(),
                    match base.record() {
                        RunRecord::RunAdmitted(admitted) => admitted.entry_point_id().clone(),
                        _ => unreachable!(),
                    },
                    program_ref,
                    context.clone(),
                    configuration,
                    Vec::new(),
                )
                .expect("admission"),
            ),
            vec![base.objects()[0].clone()],
        )
        .expect("admission frame");
        let (catalog, _) = backend_catalog_builder()
            .finish(document.clone())
            .expect("catalog");
        let opened =
            StructuredStore::open_memory(identity.clone(), catalog, StoreWorkLimits::default())
                .expect("opened store");
        opened
            .append_admission(admission.clone())
            .await
            .expect("admission append");
        let current = opened.load(admission.run_id()).await.expect("current");
        let reduced = opened
            .reduce_qualified(&current, document.clone())
            .expect("reduced");
        let owner = opened
            .prepare_conclusion_qualified_with_reduced(
                &current,
                &document,
                &reduced,
                1,
                mfm_journal::single_trust::StateConcluded::Pure {
                    occurrence: occurrence.clone(),
                    outcome: mfm_journal::single_trust::StateOutcome::Success(context.clone()),
                    fact_proposals: None,
                    fact_publication: None,
                },
                vec![base.objects()[0].clone()],
                mfm_journal::single_trust::MAX_FRAME_BYTES as u64,
            )
            .expect("conclusion owner");
        let alternate = mfm_journal::single_trust::RunFrame::new(
            owner.frame().run_id().clone(),
            owner.frame().store_scope_id().clone(),
            owner.frame().store_epoch(),
            owner.frame().expected_sequence(),
            AppendRequestId::new("classification-alternate-0123456789").expect("request"),
            owner.frame().record().clone(),
            owner.frame().objects().to_vec(),
        )
        .expect("alternate conclusion");
        assert_eq!(
            opened
                .history_port()
                .append_frame(alternate)
                .await
                .expect("alternate append"),
            AppendDisposition::NewlyCommitted { sequence: 2 }
        );
        let history = match opened.commit_conclusion(owner).await.expect("same retry") {
            ConclusionCommitOutcome::AlreadyConcludedSame { history } => history,
            other => panic!("unexpected same retry: {other:?}"),
        };
        assert_eq!(history.head_sequence(), 2);

        let alternate_json = r#"{"value":2}"#;
        let alternate_content = ContentRef::new(
            context.value_ref().schema_id().clone(),
            raw_content_digest(alternate_json.as_bytes()),
        )
        .expect("alternate value");
        let alternate_context =
            ValueRef::new(context.contract_ref().clone(), alternate_content.clone());
        let alternate_object = mfm_journal::single_trust::ImmutableObject::new(
            StableId::new("mfm.value").expect("object type"),
            alternate_content,
            alternate_json.to_owned(),
        )
        .expect("alternate object");
        let conflicting_owner = opened
            .prepare_conclusion_qualified_with_reduced(
                &current,
                &document,
                &reduced,
                1,
                mfm_journal::single_trust::StateConcluded::Pure {
                    occurrence,
                    outcome: mfm_journal::single_trust::StateOutcome::Success(alternate_context),
                    fact_proposals: None,
                    fact_publication: None,
                },
                vec![base.objects()[0].clone(), alternate_object],
                mfm_journal::single_trust::MAX_FRAME_BYTES as u64,
            )
            .expect("conflicting owner");
        match opened
            .commit_conclusion(conflicting_owner)
            .await
            .expect("conflicting retry")
        {
            ConclusionCommitOutcome::Conflict { history } => assert_eq!(history.head_sequence(), 2),
            other => panic!("unexpected conflicting retry: {other:?}"),
        }
    }

    #[tokio::test]
    async fn conclusion_head_race_classifies_superseded_access_without_reentry() {
        let identity = identity();
        let base = admission_frame(&identity);
        let (entry, contract, context, configuration) = match base.record() {
            RunRecord::RunAdmitted(admitted) => (
                admitted.entry_point_id().clone(),
                admitted.admitted_context().contract_ref().clone(),
                admitted.admitted_context().clone(),
                admitted.configuration_ref().clone(),
            ),
            _ => panic!("expected admission"),
        };
        let occurrence = mfm_journal::single_trust::SequentialControlAddress::new(0, Vec::new())
            .expect("occurrence");
        let implementation_ref = fact_source(82);
        let capability_ref = fact_source(83);
        let adapter_ref = fact_source(84);
        let binding = mfm_journal::single_trust::BindingDescriptor::new(
            implementation_ref.clone(),
            Some(capability_ref.clone()),
            Some(adapter_ref.clone()),
            fact_source(85),
            None,
            None,
        )
        .expect("binding");
        let binding_ref = binding.content_ref().expect("binding ref");
        let state = mfm_program::single_trust::StateDeclaration::new(
            occurrence.clone(),
            implementation_ref,
            contract.clone(),
            contract.clone(),
            None,
            mfm_program::single_trust::ExecutionMode::Read {
                capability_contract_ref: capability_ref,
                total_attempt_bound: 2,
                fact_selection_required: false,
            },
            true,
        )
        .expect("state")
        .with_execution_binding(binding_ref.clone())
        .expect("execution binding");
        let maximum_conclusion_bytes = state.maximum_conclusion_bytes();
        let document = mfm_program::single_trust::ProgramDocument::new(
            entry,
            contract.clone(),
            contract,
            vec![mfm_program::Declaration::State(Box::new(state))],
        )
        .expect("document");
        let admission = mfm_journal::single_trust::RunFrame::new(
            base.run_id().clone(),
            identity.scope().clone(),
            identity.epoch(),
            1,
            AppendRequestId::new("access-classification-admission-0123456789").expect("request"),
            RunRecord::RunAdmitted(
                mfm_journal::single_trust::RunAdmitted::new(
                    identity.scope().clone(),
                    identity.epoch(),
                    base.run_id().clone(),
                    identity.tenant().clone(),
                    match base.record() {
                        RunRecord::RunAdmitted(admitted) => admitted.entry_point_id().clone(),
                        _ => unreachable!(),
                    },
                    document.program_ref().expect("program"),
                    context.clone(),
                    configuration,
                    Vec::new(),
                )
                .expect("admission"),
            ),
            vec![base.objects()[0].clone()],
        )
        .expect("admission frame");
        let (catalog, _) = backend_catalog_builder()
            .finish(document.clone())
            .expect("catalog");
        let opened =
            StructuredStore::open_memory(identity.clone(), catalog, StoreWorkLimits::default())
                .expect("opened store");
        opened
            .append_admission(admission.clone())
            .await
            .expect("admission append");
        let current = opened.load(admission.run_id()).await.expect("current");
        let reduced = opened
            .reduce_qualified(&current, document.clone())
            .expect("reduced ready");
        let prepared = mfm_journal::single_trust::StatePrepared::new(
            occurrence.clone(),
            0,
            context.clone(),
            context.clone(),
            None,
            None,
            mfm_journal::single_trust::PreparationMode::Read {
                total_attempt_bound: 2,
            },
            binding.clone(),
            binding_ref.clone(),
            None,
            maximum_conclusion_bytes,
        )
        .expect("initial preparation");
        let initial = opened
            .prepare_access_qualified_with_reduced(
                &current,
                &document,
                &reduced,
                1,
                AppendRequestId::new("access-classification-preparation-0-0123456789")
                    .expect("request"),
                prepared,
                vec![base.objects()[0].clone()],
            )
            .await
            .expect("initial preparation append");
        let initial_ref = initial.preparation().cloned().expect("initial ref");
        let prepared_history = opened
            .load(admission.run_id())
            .await
            .expect("prepared history");
        let prepared_reduced = opened
            .reduce_qualified(&prepared_history, document.clone())
            .expect("prepared reduction");
        let owner = opened
            .prepare_conclusion_qualified_with_reduced(
                &prepared_history,
                &document,
                &prepared_reduced,
                2,
                mfm_journal::single_trust::StateConcluded::Access {
                    occurrence: occurrence.clone(),
                    preparation: initial_ref.clone(),
                    evidence: context.clone(),
                    outcome: mfm_journal::single_trust::StateOutcome::Success(context.clone()),
                    fact_proposals: None,
                    fact_selection: None,
                    fact_publication: None,
                },
                vec![base.objects()[0].clone()],
                maximum_conclusion_bytes,
            )
            .expect("conclusion owner");
        let replacement = mfm_journal::single_trust::StatePrepared::new(
            occurrence,
            1,
            context.clone(),
            context,
            None,
            None,
            mfm_journal::single_trust::PreparationMode::Read {
                total_attempt_bound: 2,
            },
            binding,
            binding_ref,
            Some(initial_ref),
            maximum_conclusion_bytes,
        )
        .expect("replacement preparation");
        opened
            .prepare_access_qualified_with_reduced(
                &prepared_history,
                &document,
                &prepared_reduced,
                2,
                AppendRequestId::new("access-classification-preparation-1-0123456789")
                    .expect("request"),
                replacement,
                vec![base.objects()[0].clone()],
            )
            .await
            .expect("replacement append");
        match opened
            .commit_conclusion(owner)
            .await
            .expect("classification")
        {
            ConclusionCommitOutcome::NoLongerSelected { history } => {
                assert_eq!(history.head_sequence(), 3)
            }
            other => panic!("unexpected superseded conclusion result: {other:?}"),
        }
    }

    #[cfg(feature = "test-support")]
    #[tokio::test]
    async fn unknown_conclusion_retry_finds_same_frame_without_republishing_facts() {
        let identity = identity();
        let schema = SchemaId::new(
            "mfm.test.unknown-conclusion",
            "1",
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([7; 32]),
        )
        .expect("schema");
        let contract =
            ContentRef::new(schema.clone(), raw_content_digest(b"contract")).expect("contract");
        let context_content =
            ContentRef::new(schema, raw_content_digest(br#"{"value":1}"#)).expect("context");
        let context = ValueRef::new(contract.clone(), context_content.clone());
        let occurrence = mfm_journal::single_trust::SequentialControlAddress::new(0, Vec::new())
            .expect("occurrence");
        let state = mfm_program::single_trust::StateDeclaration::new(
            occurrence.clone(),
            contract.clone(),
            contract.clone(),
            contract.clone(),
            None,
            mfm_program::single_trust::ExecutionMode::Pure,
            true,
        )
        .expect("state");
        let maximum_conclusion_bytes = state.maximum_conclusion_bytes();
        let document = mfm_program::single_trust::ProgramDocument::new(
            StableId::new("mfm.test.unknown-conclusion-entry").expect("entry"),
            contract.clone(),
            contract,
            vec![mfm_program::Declaration::State(Box::new(state))],
        )
        .expect("document");
        let program_ref = document.program_ref().expect("program ref");
        let run_id = RunId::parse(
            "run:sha256-jcs-v1:6123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        )
        .expect("run id");
        let admission = mfm_journal::single_trust::RunFrame::new(
            run_id.clone(),
            identity.scope().clone(),
            identity.epoch(),
            1,
            AppendRequestId::new("unknown-conclusion-admission-012345").expect("request"),
            RunRecord::RunAdmitted(
                mfm_journal::single_trust::RunAdmitted::new(
                    identity.scope().clone(),
                    identity.epoch(),
                    run_id.clone(),
                    identity.tenant().clone(),
                    StableId::new("mfm.test.unknown-conclusion-entry").expect("entry"),
                    program_ref,
                    context.clone(),
                    ContentRef::new(
                        SchemaId::new(
                            "mfm.test.unknown-conclusion-config",
                            "1",
                            DigestAlgorithm::Sha256JcsV1,
                            DigestBytes::from_array([8; 32]),
                        )
                        .expect("configuration schema"),
                        raw_content_digest(b"configuration"),
                    )
                    .expect("configuration"),
                    Vec::new(),
                )
                .expect("admission"),
            ),
            vec![mfm_journal::single_trust::ImmutableObject::new(
                StableId::new("mfm.value").expect("object type"),
                context_content,
                r#"{"value":1}"#.to_owned(),
            )
            .expect("context object")],
        )
        .expect("admission frame");
        let (catalog, _) = backend_catalog_builder()
            .finish(document.clone())
            .expect("catalog");
        let backend = Arc::new(MemoryStructuredBackend::new(identity.clone()));
        let opened = StructuredStore::open(
            backend.clone(),
            identity,
            catalog,
            StoreWorkLimits::default(),
        )
        .await
        .expect("opened store");
        opened
            .append_admission(admission.clone())
            .await
            .expect("admission");
        let current = opened.load(&run_id).await.expect("current");
        let reduced = opened
            .reduce_qualified(&current, document.clone())
            .expect("reduced");
        let (proposal_value, proposal_object) = proposal(100);
        let owner = opened
            .prepare_conclusion_qualified_with_reduced(
                &current,
                &document,
                &reduced,
                1,
                mfm_journal::single_trust::StateConcluded::Pure {
                    occurrence,
                    outcome: mfm_journal::single_trust::StateOutcome::Success(context),
                    fact_proposals: Some(proposal_value),
                    fact_publication: None,
                },
                vec![admission.objects()[0].clone(), proposal_object],
                maximum_conclusion_bytes,
            )
            .expect("conclusion owner");
        backend.fail_next_history_acknowledgement();
        let owner = match opened
            .commit_conclusion(owner)
            .await
            .expect("unknown commit")
        {
            ConclusionCommitOutcome::AcknowledgementUnknown(owner) => {
                assert_eq!(
                    owner
                        .frame()
                        .record()
                        .fact_publication()
                        .map(|publication| publication.publication_sequence()),
                    Some(1)
                );
                owner
            }
            other => panic!("unexpected unknown outcome: {other:?}"),
        };
        let (disposition, frame) = match opened
            .commit_conclusion(owner)
            .await
            .expect("unknown resolution")
        {
            ConclusionCommitOutcome::Disposition { disposition, frame } => (disposition, frame),
            other => panic!("unexpected resolution outcome: {other:?}"),
        };
        assert_eq!(disposition, AppendDisposition::Found { sequence: 2 });
        assert_eq!(
            frame
                .record()
                .fact_publication()
                .map(|publication| publication.publication_sequence()),
            Some(1)
        );
        let facts = opened
            .clone()
            .split()
            .into_parts()
            .3
            .facts()
            .await
            .expect("facts");
        assert_eq!(facts.head_sequence(), 1);
        assert_eq!(facts.publications().len(), 1);
        assert_eq!(
            opened
                .load(&run_id)
                .await
                .expect("retained")
                .head_sequence(),
            2
        );
    }

    #[tokio::test]
    async fn configuration_snapshots_cannot_cross_store_openings() {
        let identity = identity();
        let frame = admission_frame(&identity);
        let contract = match frame.record() {
            mfm_journal::single_trust::RunRecord::RunAdmitted(admitted) => {
                admitted.admitted_context().contract_ref().clone()
            }
            _ => panic!("expected admission"),
        };
        let (catalog, _) = backend_catalog_builder()
            .finish(
                mfm_program::single_trust::ProgramDocument::new(
                    mfm_ids::StableId::new("mfm.test.configuration-brand-entry").expect("entry"),
                    contract.clone(),
                    contract,
                    Vec::new(),
                )
                .expect("document"),
            )
            .expect("catalog");
        let backend = Arc::new(MemoryStructuredBackend::new(identity.clone()));
        let first = StructuredStore::open(
            backend.clone(),
            identity.clone(),
            catalog.clone(),
            StoreWorkLimits::default(),
        )
        .await
        .expect("first opening");
        let second = StructuredStore::open(backend, identity, catalog, StoreWorkLimits::default())
            .await
            .expect("second opening");
        let (_, _, first_configuration, _) = first.split().into_parts();
        let (_, _, second_configuration, _) = second.split().into_parts();
        let first_snapshot = first_configuration.load().await.expect("first snapshot");
        let second_snapshot = second_configuration.load().await.expect("second snapshot");
        assert_eq!(first_snapshot, second_snapshot);
        assert!(matches!(
            first_configuration.prepare_append(
                &second_snapshot,
                AppendRequestId::new("configuration-foreign-0123456789").expect("request"),
                "{\"mode\":\"foreign\"}".to_owned(),
            ),
            Err(StoreError::Identity)
        ));
        assert!(matches!(
            second_configuration.prepare_append(
                &first_snapshot,
                AppendRequestId::new("configuration-foreign-012345678a").expect("request"),
                "{\"mode\":\"foreign\"}".to_owned(),
            ),
            Err(StoreError::Identity)
        ));
    }

    #[tokio::test]
    async fn configuration_owner_promotes_direct_success_and_retains_stale_owner() {
        let contract = mfm_program::nominal_contract_ref::<BackendValue>().expect("contract");
        let document = mfm_program::single_trust::ProgramDocument::new(
            mfm_ids::StableId::new("mfm.test.configuration-entry").expect("entry"),
            contract.clone(),
            contract,
            Vec::new(),
        )
        .expect("document");
        let (catalog, _) = backend_catalog_builder().finish(document).expect("catalog");
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

#[cfg(all(test, feature = "test-support"))]
mod fault_tests {
    use super::*;
    use mfm_canonical::raw_content_digest;
    use mfm_ids::{DigestAlgorithm, DigestBytes, SchemaId};

    #[tokio::test]
    async fn memory_unknown_acknowledgements_retain_physical_identity() {
        let identity = StructuredStoreIdentity::new(
            StoreScopeId::new("mfm.store_scope.v1:0123456789abcdef0123456789abcdef")
                .expect("scope"),
            mfm_ids::StoreEpoch::new(1),
            TenantScopeId::new("mfm.tenant_scope.v1:0123456789abcdef0123456789abcdef")
                .expect("tenant"),
        );
        let backend = MemoryStructuredBackend::new(identity.clone());
        let run_id = RunId::parse(
            "run:sha256-jcs-v1:3123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        )
        .expect("run");
        let bytes = br#"{"kind":"unknown-history"}"#;
        let frame_digest = raw_content_digest(bytes);
        let head_digest = ContentDigest::parse(
            "content:sha256-v1:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
        )
        .expect("head");
        let request = AppendRequestId::new("memory-unknown-history-0123456789").expect("request");
        let command = BackendAppendCommand::new(
            &identity,
            &run_id,
            1,
            &request,
            bytes,
            &frame_digest,
            &head_digest,
            None,
            true,
            None,
        );
        backend.fail_next_history_acknowledgement();
        assert_eq!(
            backend.compare_and_append(&command).await.expect("append"),
            BackendAppendOutcome::AcknowledgementUnknown
        );
        assert!(matches!(
            backend.compare_and_append(&command).await.expect("retry"),
            BackendAppendOutcome::Found(_)
        ));
        let second_bytes = br#"{"kind":"fact-frontier-check"}"#;
        let second_digest = raw_content_digest(second_bytes);
        let second_head = raw_content_digest(b"second-head");
        let second_request =
            AppendRequestId::new("memory-fact-frontier-0123456789").expect("request");
        let second = BackendAppendCommand::new(
            &identity,
            &run_id,
            2,
            &second_request,
            second_bytes,
            &second_digest,
            &second_head,
            Some(&head_digest),
            false,
            None,
        )
        .with_fact_frontier(Some(1));
        assert_eq!(
            backend.compare_and_append(&second).await,
            Err(BackendError::FactFrontierChanged)
        );

        let schema = SchemaId::new(
            "mfm.test.unknown-configuration",
            "1",
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([0; 32]),
        )
        .expect("schema");
        let configuration_bytes = br#"{"unknown":true}"#;
        let configuration_ref =
            ContentRef::new(schema, raw_content_digest(configuration_bytes)).expect("ref");
        let configuration_request =
            AppendRequestId::new("memory-unknown-configuration-012345").expect("request");
        let configuration = ConfigurationAppendCommand::new(
            &identity,
            0,
            &configuration_request,
            configuration_bytes,
            &configuration_ref,
        );
        backend.fail_next_configuration_acknowledgement();
        assert_eq!(
            backend
                .compare_and_append_configuration(&configuration)
                .await
                .expect("configuration append"),
            BackendConfigurationOutcome::AcknowledgementUnknown
        );
        assert!(matches!(
            backend
                .compare_and_append_configuration(&configuration)
                .await
                .expect("configuration retry"),
            BackendConfigurationOutcome::Found { sequence: 1 }
        ));
    }
}
