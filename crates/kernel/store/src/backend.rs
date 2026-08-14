//! Branded mechanical Store/backend boundary.
//!
//! This module contains only physical backend contracts and raw bounded rows.  It never parses a
//! Program, reduces a run, or constructs a Runtime owner.  `OpenedStructuredStore` is the sole
//! semantic bridge: it qualifies raw rows through the Store reducer before exposing them.

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use mfm_canonical::{raw_content_digest, PlainCanonicalJsonBytes};
use mfm_capabilities::ProposedStateOutcome;
use mfm_facts::{
    FactCompleteness, FactProposalSet, FactProvenance, FactSelection, FactSelectionFrontier,
    FactSelectionRequest,
};
use mfm_ids::{
    short_stable_id_fragment, AppendRequestId, ContentDigest, ContentRef, RunId, StoreEpoch,
    StoreScopeId, TenantScopeId,
};
use mfm_journal::single_trust::{
    ConfigurationHeadProjection, ImmutableObject, PreparationMode, PreparationRef, RunRecord,
    StateOutcome, StatePrepared, ValueRef,
};
use mfm_program::{canonical_value, ProgramCatalog, QualifiedValue};
use mfm_values::{string_contains_secret_marker, MfmConfig, MfmValue, ValidatedConfig};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::single_trust::{
    advance_selected, object_for_value, prepare_access_from_current,
    prepare_conclusion_from_current, reduce_qualified, retained_program, AppendDisposition,
    QualifiedRun, Result, RunAction, SelectedRun, StoreBrand, StoreError,
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
    pub fn identity(&self) -> &StructuredStoreIdentity {
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackendConfigurationOutcome {
    /// The candidate crossed the backend commit point.
    NewlyCommitted,
    /// The same physical configuration append was retained.
    Found(Box<RawConfigurationRevision>),
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
pub(crate) enum ConclusionCommitOutcome {
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
pub enum ConfigurationCommitOutcome<C: MfmConfig> {
    /// The candidate became the direct durable successor.
    NewlyCommitted(ResolvedConfiguration<C>),
    /// The backend found and Store ingressed the exact retained successor.
    Found(ResolvedConfiguration<C>),
    /// The candidate was prepared against an older global head and promoted nothing.
    StaleHead {
        /// The current one-based global configuration sequence.
        actual_sequence: u64,
    },
    /// The transaction outcome is unknown; the exact semantic owner remains available.
    AcknowledgementUnknown(SuspendedConfigurationAppend<C>),
    /// A known Store failure occurred before an append could be accepted; the exact owner remains
    /// available for explicit retry or supervisor classification.
    Rejected {
        /// The unchanged configuration owner.
        owner: PreparedConfigurationAppend<C>,
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
    pub fn identity(&self) -> &StructuredStoreIdentity {
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
    total_bytes: usize,
}

impl RawConfigurationRevision {
    /// Constructs one bounded raw configuration revision.
    pub fn new(
        sequence: u64,
        append_request_id: AppendRequestId,
        canonical_bytes: Vec<u8>,
        content_ref: ContentRef,
        total_bytes: usize,
    ) -> BackendResult<Self> {
        if sequence == 0
            || sequence as usize > mfm_journal::single_trust::MAX_CONFIGURATION_REVISIONS
            || canonical_bytes.is_empty()
            || canonical_bytes.len() > mfm_journal::single_trust::MAX_CONFIGURATION_REVISION_BYTES
            || total_bytes < canonical_bytes.len()
            || total_bytes > mfm_journal::single_trust::MAX_CONFIGURATION_STREAM_BYTES
        {
            return Err(BackendError::Capacity);
        }
        Ok(Self {
            sequence,
            append_request_id,
            canonical_bytes,
            content_ref,
            total_bytes,
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
    /// Returns the cumulative canonical bytes through this revision.
    pub const fn total_bytes(&self) -> usize {
        self.total_bytes
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
            let total_bytes = state.configurations.iter().enumerate().try_fold(
                0usize,
                |total, (index, revision)| {
                    let next = total
                        .checked_add(revision.canonical_bytes().len())
                        .filter(|bytes| {
                            *bytes <= mfm_journal::single_trust::MAX_CONFIGURATION_STREAM_BYTES
                        })
                        .ok_or(BackendError::Capacity)?;
                    if revision.sequence() != index as u64 + 1 || revision.total_bytes() != next {
                        return Err(BackendError::Storage);
                    }
                    Ok(next)
                },
            )?;
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
                    return Ok(BackendConfigurationOutcome::Found(Box::new(
                        existing.clone(),
                    )));
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
                    .last()
                    .map_or(0, RawConfigurationRevision::total_bytes)
                    .checked_add(command.canonical_bytes().len())
                    .filter(|bytes| {
                        *bytes <= mfm_journal::single_trust::MAX_CONFIGURATION_STREAM_BYTES
                    })
                    .is_none()
            {
                return Err(BackendError::Capacity);
            }
            let revision = RawConfigurationRevision::new(
                actual + 1,
                command.append_request_id().clone(),
                command.canonical_bytes().to_vec(),
                command.content_ref().clone(),
                state
                    .configurations
                    .last()
                    .map_or(command.canonical_bytes().len(), |revision| {
                        revision
                            .total_bytes()
                            .saturating_add(command.canonical_bytes().len())
                    }),
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
pub struct OpenedStructuredStore {
    inner: Arc<OpenedStoreInner>,
}

impl std::fmt::Debug for OpenedStructuredStore {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("OpenedStructuredStore")
            .field("identity", &self.inner.identity)
            .finish_non_exhaustive()
    }
}

struct OpenedStoreInner {
    backend: Arc<dyn StructuredStoreBackend>,
    identity: StructuredStoreIdentity,
    catalog: ProgramCatalog,
    limits: StoreWorkLimits,
    brand: Arc<StoreBrand>,
    prefix_ingress: Arc<Semaphore>,
}

impl std::fmt::Debug for OpenedStoreInner {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Store")
            .field("identity", &self.identity)
            .finish_non_exhaustive()
    }
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
    pub(crate) fn identity(&self) -> &StructuredStoreIdentity {
        &self.inner.identity
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
    pub(crate) fn history_limit(&self) -> RawHistoryLoadLimit {
        RawHistoryLoadLimit::new(
            self.inner.limits.max_run_frames(),
            self.inner.limits.max_run_frame_bytes(),
        )
    }

    /// Loads one complete run prefix through the branded semantic port.
    pub(crate) async fn load(&self, run_id: &RunId) -> Result<QualifiedRun> {
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

    /// Constructs a persisted projection only after checking opaque process-local head evidence.
    pub(crate) fn configuration_projection(
        &self,
        head: &ResolvedConfigurationHead,
    ) -> Result<ConfigurationHeadProjection> {
        if !Arc::ptr_eq(&head.brand, &self.inner.brand) || !Arc::ptr_eq(&head.store, &self.inner) {
            return Err(StoreError::Identity);
        }
        ConfigurationHeadProjection::new(head.sequence, head.content_ref.clone())
            .map_err(|_| StoreError::InvalidRecord)
    }

    async fn verify_configuration_head(&self, head: &ResolvedConfigurationHead) -> Result<()> {
        self.configuration_projection(head)?;
        let rows = self
            .inner
            .backend
            .load_configuration()
            .await
            .map_err(map_backend_error)?;
        validate_configuration_rows(&rows, None)?;
        let retained = head
            .sequence
            .checked_sub(1)
            .and_then(|index| usize::try_from(index).ok())
            .and_then(|index| rows.get(index))
            .ok_or(StoreError::InvalidHistory)?;
        let captured_global = head
            .global_sequence
            .checked_sub(1)
            .and_then(|index| usize::try_from(index).ok())
            .and_then(|index| rows.get(index))
            .ok_or(StoreError::InvalidHistory)?;
        if retained.sequence() != head.sequence
            || retained.content_ref() != &head.content_ref
            || captured_global.sequence() != head.global_sequence
            || captured_global.total_bytes() != head.total_bytes
        {
            return Err(StoreError::InvalidHistory);
        }
        Ok(())
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

    /// Commits one conclusion owner through this exact opened Store.
    pub(crate) async fn commit_conclusion(
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

        let selected = match reduce_qualified(&history, owner.document().clone()) {
            Ok(selected) => selected,
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
                let Some((_, selected_preparation)) = history.selected_preparation(occurrence)
                else {
                    return Ok(ConclusionCommitOutcome::InvalidHistory { history });
                };
                if &selected_preparation != preparation {
                    return if retained_preparation_ref(&history, occurrence, preparation) {
                        Ok(ConclusionCommitOutcome::NoLongerSelected { history })
                    } else {
                        Ok(ConclusionCommitOutcome::InvalidHistory { history })
                    };
                }
                if matches!(
                    selected.action(),
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
                    selected.action(),
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

    #[cfg(test)]
    fn test_configuration(&self) -> ConfigurationStore {
        ConfigurationStore {
            inner: Arc::clone(&self.inner),
        }
    }

    #[cfg(test)]
    fn test_same_open(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner.brand, &other.inner.brand)
    }

    #[cfg(test)]
    async fn test_append_admission(
        &self,
        frame: mfm_journal::single_trust::RunFrame,
        configuration: &ResolvedConfigurationHead,
    ) -> Result<AppendDisposition> {
        self.history_port()
            .append_admission(frame, configuration)
            .await
    }

    #[cfg(test)]
    fn test_select_qualified(&self, run: &QualifiedRun) -> Result<SelectedRun> {
        self.history_port().select_qualified(run.clone())
    }

    #[cfg(test)]
    fn test_qualify_appended(
        &self,
        predecessor: &QualifiedRun,
        frame: mfm_journal::single_trust::RunFrame,
    ) -> Result<QualifiedRun> {
        if !predecessor.belongs_to_store(&self.inner.brand) {
            return Err(StoreError::Identity);
        }
        predecessor.clone().append_validated(frame)
    }

    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    async fn test_prepare_access_fixture(
        &self,
        current: &QualifiedRun,
        document: &mfm_program::single_trust::ProgramDocument,
        selected: &SelectedRun,
        expected_sequence: u64,
        append_request_id: AppendRequestId,
        prepared: mfm_journal::single_trust::StatePrepared,
        objects: Vec<mfm_journal::single_trust::ImmutableObject>,
    ) -> Result<crate::single_trust::PreparationAppend> {
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
            selected.selection(),
            expected_sequence,
            append_request_id,
            prepared,
            objects,
        )?;
        candidate.append.bind_store(Arc::clone(&self.inner.brand));
        let Some(frame) = candidate.frame else {
            return Ok(candidate.append);
        };
        let disposition = self
            .history_port()
            .append_against_qualified_prefix(current, frame)
            .await?;
        Ok(candidate.append.with_disposition(disposition))
    }

    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    fn test_prepare_conclusion_fixture(
        &self,
        current: &QualifiedRun,
        document: &mfm_program::single_trust::ProgramDocument,
        selected: &SelectedRun,
        expected_sequence: u64,
        conclusion: mfm_journal::single_trust::StateConcluded,
        objects: Vec<mfm_journal::single_trust::ImmutableObject>,
        maximum_conclusion_bytes: u64,
    ) -> Result<crate::single_trust::PreparedConclusion> {
        let append_request_id = conclusion_append_request_id(current.run_id(), expected_sequence)?;
        prepare_conclusion_from_current(
            self.identity().scope(),
            self.identity().epoch(),
            self.identity().tenant(),
            current,
            current.run_id(),
            document,
            selected.selection(),
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
}

/// The consuming branded Store port split.
pub struct StoreParts {
    history: QualifiedHistoryPort,
    reader: HistoryReader,
    configuration: ConfigurationStore,
    audit: StoreAuditPort,
}

impl StoreParts {
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

/// Exhaustive result of consuming one selected Access owner into Store-owned preparation.
#[derive(Debug)]
#[allow(clippy::large_enum_variant)]
pub enum AccessPreparationOutcome {
    /// The preparation was appended directly and the returned owner selects that durable head.
    Committed {
        /// The sole selected authority at the preparation head.
        selected: SelectedRun,
        /// The Store-assigned preparation identity.
        preparation: PreparationRef,
        /// Optional one-use prior-fact continuation selected by Store.
        fact_continuation: Option<crate::single_trust::FactContinuation>,
    },
    /// No direct-new append crossed the durability point; the predecessor selection is retained.
    Retained {
        /// The unchanged selected predecessor.
        selected: SelectedRun,
        /// The mechanical disposition that prevented provider entry.
        disposition: AppendDisposition,
    },
    /// Store rejected the coordinate-free proposal before a direct-new append was established.
    Rejected {
        /// The unchanged selected predecessor.
        selected: SelectedRun,
        /// Redaction-safe rejection.
        error: StoreError,
    },
}

/// Affine Store-owned conclusion append paired with its exact selected predecessor.
#[derive(Debug)]
pub struct SelectedConclusion {
    owner: crate::single_trust::PreparedConclusion,
    selected: SelectedRun,
}

impl SelectedConclusion {
    /// Returns the durable run identity retained by this append owner.
    pub fn run_id(&self) -> &RunId {
        self.selected.run_id()
    }
}

/// Result of validating coordinate-free conclusion material against one selected owner.
#[derive(Debug)]
#[allow(clippy::large_enum_variant)]
pub enum SelectedConclusionPreparationOutcome {
    /// Store accepted the proposal and built the complete append owner.
    Prepared(SelectedConclusion),
    /// Store rejected the proposal before append ownership was established.
    Rejected {
        /// The unchanged selected owner.
        selected: SelectedRun,
        /// Redaction-safe rejection.
        error: StoreError,
    },
}

/// Result of resolving one selected conclusion append.
#[derive(Debug)]
pub enum SelectedConclusionOutcome {
    /// The conclusion is durable and the returned owner selects its exact successor head.
    Committed(SelectedRun),
    /// The exact append outcome remains unknown and must be resolved with this same owner.
    AcknowledgementUnknown(SelectedConclusion),
    /// The same semantic conclusion is durable and execution may continue from this fresh owner.
    AlreadyConcludedSame(SelectedRun),
    /// A later Read preparation is selected and execution may continue from this fresh owner.
    NoLongerSelected(SelectedRun),
    /// A different semantic conclusion won; only callback-free evidence remains.
    Conflict(QualifiedRun),
    /// Retained history is invalid; only callback-free evidence remains.
    InvalidHistory(QualifiedRun),
    /// The unchanged append owner remains available after a known rejection.
    Rejected {
        /// Exact selected conclusion owner.
        owner: SelectedConclusion,
        /// Redaction-safe rejection.
        error: StoreError,
    },
}

/// Opaque Store-built admission append retained across acknowledgement uncertainty.
#[derive(Debug)]
pub struct PreparedAdmission {
    frame: mfm_journal::single_trust::RunFrame,
    configuration: ResolvedConfigurationHead,
}

impl PreparedAdmission {
    /// Returns the deterministic run identity carried by this Store-built admission.
    pub fn run_id(&self) -> &RunId {
        self.frame.run_id()
    }
}

/// Exhaustive semantic outcome of a Store-built admission.
#[derive(Debug)]
pub enum AdmissionOutcome {
    /// The exact admission is durable and selected at its genesis head.
    Selected(SelectedRun, AppendDisposition),
    /// The physical result is unknown; the same Store-built owner must be resolved.
    AcknowledgementUnknown(PreparedAdmission),
    /// A different semantic genesis already owns the run id.
    Conflict(QualifiedRun),
    /// Admission failed before a semantic owner was established.
    Rejected(StoreError),
    /// A retryable Store-built owner remains after a known resolution failure.
    RetainedRejected {
        /// The unchanged admission owner.
        owner: PreparedAdmission,
        /// Redaction-safe failure.
        error: StoreError,
    },
}

impl QualifiedHistoryPort {
    /// Returns immutable metadata for trusted composition without exposing another mutation port.
    pub fn identity(&self) -> &StructuredStoreIdentity {
        &self.inner.identity
    }

    /// Returns the exact callback-free catalog associated with this mutation port.
    pub fn catalog(&self) -> &ProgramCatalog {
        &self.inner.catalog
    }

    /// Returns whether opaque configuration evidence belongs to this exact Store opening.
    pub fn accepts_configuration_head(&self, head: &ResolvedConfigurationHead) -> bool {
        Arc::ptr_eq(&head.brand, &self.inner.brand) && Arc::ptr_eq(&head.store, &self.inner)
    }

    /// Builds and appends one genesis frame from an exact catalog-qualified typed value.
    pub async fn admit<T: MfmValue>(
        &self,
        run_id: RunId,
        program: &mfm_program::Program,
        value: &mfm_program::QualifiedTypedValue<T>,
        configuration: ResolvedConfigurationHead,
        source_refs: Vec<ContentRef>,
    ) -> AdmissionOutcome {
        if !program.belongs_to_catalog(&self.inner.catalog)
            || !value.belongs_to_catalog(&self.inner.catalog)
            || value.contract_ref() != program.document().admitted_context_contract_ref()
        {
            return AdmissionOutcome::Rejected(StoreError::Identity);
        }
        let projection = match (OpenedStructuredStore {
            inner: Arc::clone(&self.inner),
        })
        .configuration_projection(&configuration)
        {
            Ok(value) => value,
            Err(error) => return AdmissionOutcome::Rejected(error),
        };
        let admitted = match mfm_journal::single_trust::RunAdmitted::new(
            self.inner.identity.scope().clone(),
            self.inner.identity.epoch(),
            run_id.clone(),
            self.inner.identity.tenant().clone(),
            program.document().entry_point_id().clone(),
            program.program_ref().content_ref().clone(),
            ValueRef::new(value.contract_ref().clone(), value.value_ref().clone()),
            projection,
            source_refs,
        ) {
            Ok(value) => value,
            Err(_) => return AdmissionOutcome::Rejected(StoreError::InvalidRecord),
        };
        let append_request_id = match AppendRequestId::new(format!(
            "admission-{}-1",
            short_stable_id_fragment(run_id.as_str(), 96)
        )) {
            Ok(value) => value,
            Err(_) => return AdmissionOutcome::Rejected(StoreError::InvalidRecord),
        };
        let canonical = match std::str::from_utf8(value.canonical_bytes()) {
            Ok(value) => value.to_owned(),
            Err(_) => return AdmissionOutcome::Rejected(StoreError::InvalidRecord),
        };
        let object_kind = match mfm_ids::StableId::new("mfm.value") {
            Ok(value) => value,
            Err(_) => return AdmissionOutcome::Rejected(StoreError::InvalidRecord),
        };
        let object = match ImmutableObject::new(object_kind, value.value_ref().clone(), canonical) {
            Ok(value) => value,
            Err(_) => return AdmissionOutcome::Rejected(StoreError::InvalidRecord),
        };
        let program_canonical = match program.document().canonical_bytes() {
            Ok(value) => value,
            Err(_) => return AdmissionOutcome::Rejected(StoreError::InvalidRecord),
        };
        let program_kind = match mfm_ids::StableId::new("mfm.program") {
            Ok(value) => value,
            Err(_) => return AdmissionOutcome::Rejected(StoreError::InvalidRecord),
        };
        let program_object = match ImmutableObject::new(
            program_kind,
            program.program_ref().content_ref().clone(),
            program_canonical.as_str().to_owned(),
        ) {
            Ok(value) => value,
            Err(_) => return AdmissionOutcome::Rejected(StoreError::InvalidRecord),
        };
        let frame = match mfm_journal::single_trust::RunFrame::new(
            run_id,
            self.inner.identity.scope().clone(),
            self.inner.identity.epoch(),
            1,
            append_request_id,
            RunRecord::RunAdmitted(admitted),
            vec![object, program_object],
        ) {
            Ok(value) => value,
            Err(_) => return AdmissionOutcome::Rejected(StoreError::InvalidRecord),
        };
        self.resolve_prepared_admission(PreparedAdmission {
            frame,
            configuration,
        })
        .await
    }

    /// Resolves one exact Store-built admission owner without rebuilding journal coordinates.
    pub async fn resolve_admission(&self, owner: PreparedAdmission) -> AdmissionOutcome {
        self.resolve_prepared_admission(owner).await
    }

    async fn resolve_prepared_admission(&self, owner: PreparedAdmission) -> AdmissionOutcome {
        let disposition = match self
            .append_admission(owner.frame.clone(), &owner.configuration)
            .await
        {
            Ok(value) => value,
            Err(StoreError::Conflict | StoreError::NotActionable) => {
                let history = match self.load(owner.frame.run_id()).await {
                    Ok(history) => history,
                    Err(error) => return AdmissionOutcome::RetainedRejected { owner, error },
                };
                if history.frames().first() == Some(&owner.frame) {
                    return match self.select_history(history) {
                        Ok(selected) => AdmissionOutcome::Selected(
                            selected,
                            AppendDisposition::Found { sequence: 1 },
                        ),
                        Err(error) => AdmissionOutcome::Rejected(error),
                    };
                }
                return AdmissionOutcome::Conflict(history);
            }
            Err(error) => return AdmissionOutcome::RetainedRejected { owner, error },
        };
        match disposition {
            AppendDisposition::NewlyCommitted { .. } => {
                let mut run = match QualifiedRun::qualify_prefix(
                    self.inner.identity.scope().clone(),
                    self.inner.identity.epoch(),
                    self.inner.identity.tenant().clone(),
                    vec![owner.frame.clone()],
                ) {
                    Ok(value) => value,
                    Err(error) => return AdmissionOutcome::RetainedRejected { owner, error },
                };
                run.bind_store(Arc::clone(&self.inner.brand));
                match self.select_history(run) {
                    Ok(selected) => AdmissionOutcome::Selected(selected, disposition),
                    Err(error) => AdmissionOutcome::Rejected(error),
                }
            }
            AppendDisposition::Found { .. } | AppendDisposition::StaleHead { .. } => {
                let history = match self.load(owner.frame.run_id()).await {
                    Ok(value) => value,
                    Err(error) => return AdmissionOutcome::RetainedRejected { owner, error },
                };
                if history.frames().first() == Some(&owner.frame) {
                    match self.select_history(history) {
                        Ok(selected) => AdmissionOutcome::Selected(selected, disposition),
                        Err(error) => AdmissionOutcome::Rejected(error),
                    }
                } else {
                    AdmissionOutcome::Conflict(history)
                }
            }
            AppendDisposition::AcknowledgementUnknown => {
                AdmissionOutcome::AcknowledgementUnknown(owner)
            }
        }
    }

    /// Loads, qualifies, and selects one complete prefix using its retained exact Program.
    pub async fn select(&self, run_id: &RunId) -> Result<SelectedRun> {
        let run = self.load(run_id).await?;
        self.select_qualified(run)
    }

    /// Selects callback-free evidence that was loaded through this exact mutation port.
    fn select_qualified(&self, run: QualifiedRun) -> Result<SelectedRun> {
        if !run.belongs_to_store(&self.inner.brand) {
            return Err(StoreError::Identity);
        }
        let program = retained_program(&run, &self.inner.catalog)?;
        self.validate_typed_material(&run, program.document())?;
        let document = program.document().clone();
        let selection = reduce_qualified(&run, document.clone())?;
        Ok(SelectedRun::new(
            run,
            program,
            selection,
            Arc::clone(&self.inner.brand),
        ))
    }

    /// Consumes the selected Access owner and coordinate-free intent material.
    ///
    /// Store derives the occurrence, predecessor, sequence, ordinal, replacement, mode, reserved
    /// conclusion capacity, and append identity from the selected owner and Program.
    pub fn prepare_selected_access<'a>(
        &'a self,
        selected: SelectedRun,
        intent: &'a QualifiedValue,
    ) -> Pin<Box<dyn Future<Output = AccessPreparationOutcome> + Send + 'a>> {
        Box::pin(self.prepare_selected_access_inner(selected, intent))
    }

    async fn prepare_selected_access_inner(
        &self,
        selected: SelectedRun,
        intent: &QualifiedValue,
    ) -> AccessPreparationOutcome {
        if !selected.belongs_to_store(&self.inner.brand) {
            return AccessPreparationOutcome::Rejected {
                selected,
                error: StoreError::Identity,
            };
        }
        let (run, program, selection) = selected.into_parts();
        let reselect = |run, program, selection| {
            SelectedRun::new(run, program, selection, Arc::clone(&self.inner.brand))
        };
        let (occurrence, input) = match selection.action() {
            RunAction::ReadyAccess {
                occurrence, input, ..
            } => (occurrence.clone(), input.clone()),
            RunAction::WaitingPreparation { occurrence, .. } => {
                let Some((prepared, _)) = run.selected_preparation(occurrence) else {
                    return AccessPreparationOutcome::Rejected {
                        selected: reselect(run, program, selection),
                        error: StoreError::InvalidHistory,
                    };
                };
                (occurrence.clone(), prepared.input().clone())
            }
            _ => {
                return AccessPreparationOutcome::Rejected {
                    selected: reselect(run, program, selection),
                    error: StoreError::NotActionable,
                }
            }
        };
        let Some(mfm_program::Declaration::State(state)) =
            program.document().declaration(&occurrence)
        else {
            return AccessPreparationOutcome::Rejected {
                selected: reselect(run, program, selection),
                error: StoreError::InvalidHistory,
            };
        };
        let mode = match state.execution() {
            mfm_program::ExecutionMode::Read {
                total_attempt_bound,
                ..
            } => PreparationMode::Read {
                total_attempt_bound: *total_attempt_bound,
            },
            mfm_program::ExecutionMode::Effect { .. } => PreparationMode::Effect,
            mfm_program::ExecutionMode::Pure => {
                return AccessPreparationOutcome::Rejected {
                    selected: reselect(run, program, selection),
                    error: StoreError::NotActionable,
                }
            }
        };
        let Some(binding) = state.execution_binding() else {
            return AccessPreparationOutcome::Rejected {
                selected: reselect(run, program, selection),
                error: StoreError::InvalidHistory,
            };
        };
        let Some(capability_contract_ref) = state.execution().capability_contract_ref() else {
            return AccessPreparationOutcome::Rejected {
                selected: reselect(run, program, selection),
                error: StoreError::InvalidHistory,
            };
        };
        let fact_request = match self
            .inner
            .catalog
            .validate_access_intent(capability_contract_ref, intent)
        {
            Ok(request) if request.is_some() == state.fact_selection_required() => request,
            _ => {
                return AccessPreparationOutcome::Rejected {
                    selected: reselect(run, program, selection),
                    error: StoreError::InvalidRecord,
                }
            }
        };
        let (intent_ref, intent_object) = match qualified_material(&self.inner.catalog, intent) {
            Ok(value) => value,
            Err(error) => {
                return AccessPreparationOutcome::Rejected {
                    selected: reselect(run, program, selection),
                    error,
                }
            }
        };
        let fact_request = match fact_request.as_ref().map(concrete_material).transpose() {
            Ok(value) => value,
            Err(error) => {
                return AccessPreparationOutcome::Rejected {
                    selected: reselect(run, program, selection),
                    error,
                }
            }
        };
        let execution_binding_ref = match binding.content_ref() {
            Ok(value) => value,
            Err(_) => {
                return AccessPreparationOutcome::Rejected {
                    selected: reselect(run, program, selection),
                    error: StoreError::InvalidRecord,
                }
            }
        };
        let (ordinal, replaces) = match run.selected_preparation(&occurrence) {
            Some((previous, previous_ref)) => (
                previous.preparation_ordinal().saturating_add(1),
                Some(previous_ref),
            ),
            None => (0, None),
        };
        let prepared = match StatePrepared::new(
            occurrence.clone(),
            ordinal,
            input,
            intent_ref,
            fact_request.as_ref().map(|(request, _)| request.clone()),
            None,
            mode,
            execution_binding_ref,
            replaces,
            state.maximum_conclusion_bytes(),
        ) {
            Ok(prepared) => prepared,
            Err(_) => {
                return AccessPreparationOutcome::Rejected {
                    selected: reselect(run, program, selection),
                    error: StoreError::InvalidRecord,
                }
            }
        };
        let mut objects = vec![intent_object];
        if let Some((_, object)) = fact_request {
            objects.push(object);
        }
        let store = OpenedStructuredStore {
            inner: Arc::clone(&self.inner),
        };
        let (prepared, objects) = match store
            .store_select_fact_selection(&run, prepared, objects)
            .await
        {
            Ok(value) => value,
            Err(error) => {
                return AccessPreparationOutcome::Rejected {
                    selected: SelectedRun::new(
                        run,
                        program,
                        selection,
                        Arc::clone(&self.inner.brand),
                    ),
                    error,
                }
            }
        };
        let append_request_id = match AppendRequestId::new(format!(
            "preparation-{}-{}",
            short_stable_id_fragment(run.run_id().as_str(), 96),
            run.head_sequence().saturating_add(1)
        )) {
            Ok(value) => value,
            Err(_) => {
                return AccessPreparationOutcome::Rejected {
                    selected: SelectedRun::new(
                        run,
                        program,
                        selection,
                        Arc::clone(&self.inner.brand),
                    ),
                    error: StoreError::InvalidRecord,
                }
            }
        };
        let mut candidate = match prepare_access_from_current(
            self.inner.identity.scope(),
            self.inner.identity.epoch(),
            self.inner.identity.tenant(),
            &run,
            run.run_id(),
            program.document(),
            &selection,
            run.head_sequence(),
            append_request_id,
            prepared,
            objects,
        ) {
            Ok(value) => value,
            Err(error) => {
                return AccessPreparationOutcome::Rejected {
                    selected: SelectedRun::new(
                        run,
                        program,
                        selection,
                        Arc::clone(&self.inner.brand),
                    ),
                    error,
                }
            }
        };
        candidate.append.bind_store(Arc::clone(&self.inner.brand));
        let Some(frame) = candidate.frame else {
            return AccessPreparationOutcome::Retained {
                selected: SelectedRun::new(run, program, selection, Arc::clone(&self.inner.brand)),
                disposition: candidate.append.disposition(),
            };
        };
        let disposition = match self
            .append_against_qualified_prefix(&run, frame.clone())
            .await
        {
            Ok(value) => value,
            Err(error) => {
                return AccessPreparationOutcome::Rejected {
                    selected: SelectedRun::new(
                        run,
                        program,
                        selection,
                        Arc::clone(&self.inner.brand),
                    ),
                    error,
                }
            }
        };
        if !matches!(disposition, AppendDisposition::NewlyCommitted { .. }) {
            return AccessPreparationOutcome::Retained {
                selected: SelectedRun::new(run, program, selection, Arc::clone(&self.inner.brand)),
                disposition,
            };
        }
        let Some(preparation) = candidate.append.preparation().cloned() else {
            return AccessPreparationOutcome::Rejected {
                selected: SelectedRun::new(run, program, selection, Arc::clone(&self.inner.brand)),
                error: StoreError::InvalidHistory,
            };
        };
        let fact_continuation = candidate.append.into_fact_continuation();
        let next_run = match run.clone().append_validated(frame) {
            Ok(value) => value,
            Err(error) => {
                return AccessPreparationOutcome::Rejected {
                    selected: SelectedRun::new(
                        run,
                        program,
                        selection,
                        Arc::clone(&self.inner.brand),
                    ),
                    error,
                }
            }
        };
        let next_selection = selection.waiting_preparation(occurrence, preparation.clone());
        AccessPreparationOutcome::Committed {
            selected: SelectedRun::new(
                next_run,
                program,
                next_selection,
                Arc::clone(&self.inner.brand),
            ),
            preparation,
            fact_continuation,
        }
    }

    /// Consumes a selected Pure action and coordinate-free typed outcome material.
    pub fn prepare_selected_pure_conclusion(
        &self,
        selected: SelectedRun,
        proposal: ProposedStateOutcome<QualifiedValue, QualifiedValue>,
    ) -> SelectedConclusionPreparationOutcome {
        let occurrence = match selected.action() {
            RunAction::ReadyPure { occurrence, .. } => occurrence.clone(),
            _ => {
                return SelectedConclusionPreparationOutcome::Rejected {
                    selected,
                    error: StoreError::NotActionable,
                }
            }
        };
        let (outcome, outcome_object, fact_proposals) = match proposed_outcome(
            &self.inner.catalog,
            selected.document(),
            &occurrence,
            proposal,
        ) {
            Ok(value) => value,
            Err(error) => {
                return SelectedConclusionPreparationOutcome::Rejected { selected, error }
            }
        };
        let conclusion = mfm_journal::single_trust::StateConcluded::Pure {
            occurrence,
            outcome,
            fact_proposals: fact_proposals.as_ref().map(|(value, _)| value.clone()),
            fact_publication: None,
        };
        let mut objects = vec![outcome_object];
        if let Some((_, object)) = fact_proposals {
            objects.push(object);
        }
        match self.prepare_selected_conclusion(selected, conclusion, objects) {
            Ok(owner) => SelectedConclusionPreparationOutcome::Prepared(owner),
            Err((selected, error)) => {
                SelectedConclusionPreparationOutcome::Rejected { selected, error }
            }
        }
    }

    /// Consumes a selected Access conclusion and its coordinate-free typed result material.
    pub fn prepare_selected_access_conclusion(
        &self,
        selected: SelectedRun,
        evidence: QualifiedValue,
        proposal: ProposedStateOutcome<QualifiedValue, QualifiedValue>,
        fact_continuation: Option<crate::single_trust::FactContinuation>,
    ) -> SelectedConclusionPreparationOutcome {
        let (occurrence, preparation) = match selected.action() {
            RunAction::WaitingPreparation {
                occurrence,
                preparation,
            } => (occurrence.clone(), preparation.clone()),
            _ => {
                return SelectedConclusionPreparationOutcome::Rejected {
                    selected,
                    error: StoreError::NotActionable,
                }
            }
        };
        let Some((prepared, _)) = selected.qualified_run().selected_preparation(&occurrence) else {
            return SelectedConclusionPreparationOutcome::Rejected {
                selected,
                error: StoreError::InvalidHistory,
            };
        };
        let Some(mfm_program::Declaration::State(state)) =
            selected.document().declaration(&occurrence)
        else {
            return SelectedConclusionPreparationOutcome::Rejected {
                selected,
                error: StoreError::InvalidHistory,
            };
        };
        let Some(capability_contract_ref) = state.execution().capability_contract_ref() else {
            return SelectedConclusionPreparationOutcome::Rejected {
                selected,
                error: StoreError::InvalidHistory,
            };
        };
        let intent_object = match object_for_value(selected.qualified_run(), prepared.intent()) {
            Ok(value) => value,
            Err(error) => {
                return SelectedConclusionPreparationOutcome::Rejected { selected, error }
            }
        };
        let intent = match self.inner.catalog.qualify_retained_erased(
            prepared.intent().contract_ref().clone(),
            intent_object.canonical_json().as_bytes(),
        ) {
            Ok(value) => value,
            Err(_) => {
                return SelectedConclusionPreparationOutcome::Rejected {
                    selected,
                    error: StoreError::InvalidHistory,
                }
            }
        };
        if self
            .inner
            .catalog
            .validate_access_evidence(capability_contract_ref, &intent, &evidence)
            .is_err()
        {
            return SelectedConclusionPreparationOutcome::Rejected {
                selected,
                error: StoreError::InvalidRecord,
            };
        }
        let (evidence_ref, evidence_object) =
            match qualified_material(&self.inner.catalog, &evidence) {
                Ok(value) => value,
                Err(error) => {
                    return SelectedConclusionPreparationOutcome::Rejected { selected, error }
                }
            };
        let (outcome, outcome_object, fact_proposals) = match proposed_outcome(
            &self.inner.catalog,
            selected.document(),
            &occurrence,
            proposal,
        ) {
            Ok(value) => value,
            Err(error) => {
                return SelectedConclusionPreparationOutcome::Rejected { selected, error }
            }
        };
        let fact_selection = match fact_continuation {
            Some(continuation) => {
                if !continuation.belongs_to_store(&self.inner.brand)
                    || continuation.scope() != self.inner.identity.scope()
                    || continuation.epoch() != self.inner.identity.epoch()
                    || continuation.tenant() != self.inner.identity.tenant()
                    || continuation.run_id() != selected.run_id()
                    || continuation.preparation() != &preparation
                {
                    return SelectedConclusionPreparationOutcome::Rejected {
                        selected,
                        error: StoreError::Identity,
                    };
                }
                Some(continuation.selection_ref().clone())
            }
            None => None,
        };
        let conclusion = mfm_journal::single_trust::StateConcluded::Access {
            occurrence,
            preparation,
            evidence: evidence_ref,
            outcome,
            fact_proposals: fact_proposals.as_ref().map(|(value, _)| value.clone()),
            fact_selection,
            fact_publication: None,
        };
        let mut objects = vec![evidence_object, outcome_object];
        if let Some((_, object)) = fact_proposals {
            objects.push(object);
        }
        match self.prepare_selected_conclusion(selected, conclusion, objects) {
            Ok(owner) => SelectedConclusionPreparationOutcome::Prepared(owner),
            Err((selected, error)) => {
                SelectedConclusionPreparationOutcome::Rejected { selected, error }
            }
        }
    }

    #[allow(clippy::result_large_err)]
    fn prepare_selected_conclusion(
        &self,
        selected: SelectedRun,
        conclusion: mfm_journal::single_trust::StateConcluded,
        objects: Vec<ImmutableObject>,
    ) -> std::result::Result<SelectedConclusion, (SelectedRun, StoreError)> {
        if !selected.belongs_to_store(&self.inner.brand) {
            return Err((selected, StoreError::Identity));
        }
        let expected_sequence = selected.head_sequence();
        let append_request_id =
            match conclusion_append_request_id(selected.run_id(), expected_sequence) {
                Ok(append_request_id) => append_request_id,
                Err(error) => return Err((selected, error)),
            };
        let maximum_conclusion_bytes =
            match selected.document().declaration(conclusion.occurrence()) {
                Some(mfm_program::Declaration::State(state)) => state.maximum_conclusion_bytes(),
                _ => return Err((selected, StoreError::InvalidHistory)),
            };
        let owner = prepare_conclusion_from_current(
            self.inner.identity.scope(),
            self.inner.identity.epoch(),
            self.inner.identity.tenant(),
            selected.qualified_run(),
            selected.run_id(),
            selected.document(),
            selected.selection(),
            expected_sequence,
            append_request_id,
            conclusion,
            objects,
            maximum_conclusion_bytes,
        );
        let mut owner = match owner {
            Ok(owner) => owner,
            Err(error) => return Err((selected, error)),
        };
        owner.bind_store(Arc::clone(&self.inner.brand));
        Ok(SelectedConclusion { owner, selected })
    }

    /// Resolves one exact selected conclusion owner through this mutation port.
    pub async fn commit_selected_conclusion(
        &self,
        pending: SelectedConclusion,
    ) -> SelectedConclusionOutcome {
        let SelectedConclusion { owner, selected } = pending;
        let store = OpenedStructuredStore {
            inner: Arc::clone(&self.inner),
        };
        let outcome = match store.commit_conclusion(owner).await {
            Ok(outcome) => outcome,
            Err(_) => {
                return SelectedConclusionOutcome::InvalidHistory(selected.into_qualified_run())
            }
        };
        match outcome {
            ConclusionCommitOutcome::Disposition {
                disposition: AppendDisposition::NewlyCommitted { .. },
                frame,
            } => {
                let (previous, program, selection) = selected.into_parts();
                let next = match previous.clone().append_validated(frame) {
                    Ok(next) => next,
                    Err(_) => return SelectedConclusionOutcome::InvalidHistory(previous),
                };
                let next_selection =
                    match advance_selected(&selection, &previous, &next, program.document()) {
                        Ok(selection) => selection,
                        Err(_) => return SelectedConclusionOutcome::InvalidHistory(next),
                    };
                SelectedConclusionOutcome::Committed(SelectedRun::new(
                    next,
                    program,
                    next_selection,
                    Arc::clone(&self.inner.brand),
                ))
            }
            ConclusionCommitOutcome::Disposition {
                disposition: AppendDisposition::Found { .. },
                ..
            } => match self.load(selected.run_id()).await {
                Ok(history) => match self.select_history(history.clone()) {
                    Ok(selected) => SelectedConclusionOutcome::Committed(selected),
                    Err(_) => SelectedConclusionOutcome::InvalidHistory(history),
                },
                Err(_) => SelectedConclusionOutcome::InvalidHistory(selected.into_qualified_run()),
            },
            ConclusionCommitOutcome::Disposition { .. } => {
                SelectedConclusionOutcome::InvalidHistory(selected.into_qualified_run())
            }
            ConclusionCommitOutcome::AcknowledgementUnknown(owner) => {
                SelectedConclusionOutcome::AcknowledgementUnknown(SelectedConclusion {
                    owner,
                    selected,
                })
            }
            ConclusionCommitOutcome::AlreadyConcludedSame { history } => {
                match self.select_history(history.clone()) {
                    Ok(selected) => SelectedConclusionOutcome::AlreadyConcludedSame(selected),
                    Err(_) => SelectedConclusionOutcome::InvalidHistory(history),
                }
            }
            ConclusionCommitOutcome::NoLongerSelected { history } => {
                match self.select_history(history.clone()) {
                    Ok(selected) => SelectedConclusionOutcome::NoLongerSelected(selected),
                    Err(_) => SelectedConclusionOutcome::InvalidHistory(history),
                }
            }
            ConclusionCommitOutcome::Conflict { history } => {
                SelectedConclusionOutcome::Conflict(history)
            }
            ConclusionCommitOutcome::InvalidHistory { history } => {
                SelectedConclusionOutcome::InvalidHistory(history)
            }
            ConclusionCommitOutcome::Rejected { owner, error } => {
                SelectedConclusionOutcome::Rejected {
                    owner: SelectedConclusion { owner, selected },
                    error,
                }
            }
        }
    }

    fn select_history(&self, history: QualifiedRun) -> Result<SelectedRun> {
        self.select_qualified(history)
    }

    fn validate_typed_material(
        &self,
        run: &QualifiedRun,
        document: &mfm_program::ProgramDocument,
    ) -> Result<()> {
        let qualify = |value: &ValueRef| {
            let object = object_for_value(run, value)?;
            self.inner
                .catalog
                .qualify_retained_erased(
                    value.contract_ref().clone(),
                    object.canonical_json().as_bytes(),
                )
                .map_err(|_| StoreError::InvalidHistory)
        };
        let state = |occurrence| {
            document
                .declaration(occurrence)
                .and_then(|declaration| match declaration {
                    mfm_program::Declaration::State(state) => Some(state.as_ref()),
                    mfm_program::Declaration::Match(_) => None,
                })
                .ok_or(StoreError::InvalidHistory)
        };
        for frame in run.frames() {
            match frame.record() {
                RunRecord::RunAdmitted(admitted) => {
                    if admitted.admitted_context().contract_ref()
                        != document.admitted_context_contract_ref()
                    {
                        return Err(StoreError::InvalidHistory);
                    }
                    qualify(admitted.admitted_context())?;
                }
                RunRecord::StatePrepared(prepared) => {
                    let state = state(prepared.occurrence())?;
                    if prepared.input().contract_ref() != state.input_contract_ref() {
                        return Err(StoreError::InvalidHistory);
                    }
                    qualify(prepared.input())?;
                    let intent = qualify(prepared.intent())?;
                    let capability = state
                        .execution()
                        .capability_contract_ref()
                        .ok_or(StoreError::InvalidHistory)?;
                    let request = self
                        .inner
                        .catalog
                        .validate_access_intent(capability, &intent)
                        .map_err(|_| StoreError::InvalidHistory)?;
                    let request_ref = request
                        .as_ref()
                        .map(concrete_material)
                        .transpose()
                        .map_err(|_| StoreError::InvalidHistory)?;
                    if prepared.fact_request() != request_ref.as_ref().map(|(value, _)| value) {
                        return Err(StoreError::InvalidHistory);
                    }
                }
                RunRecord::StateConcluded(concluded) => {
                    let state = state(concluded.occurrence())?;
                    let (outcome, expected_contract) = match concluded.outcome() {
                        StateOutcome::Success(value) => (value, Some(state.output_contract_ref())),
                        StateOutcome::Failure(value) => (value, state.failure_contract_ref()),
                    };
                    if expected_contract != Some(outcome.contract_ref()) {
                        return Err(StoreError::InvalidHistory);
                    }
                    qualify(outcome)?;
                    if let mfm_journal::StateConcluded::Access { evidence, .. } = concluded {
                        let (prepared, _) = run
                            .selected_preparation(concluded.occurrence())
                            .ok_or(StoreError::InvalidHistory)?;
                        let intent = qualify(prepared.intent())?;
                        let evidence = qualify(evidence)?;
                        self.inner
                            .catalog
                            .validate_access_evidence(
                                state
                                    .execution()
                                    .capability_contract_ref()
                                    .ok_or(StoreError::InvalidHistory)?,
                                &intent,
                                &evidence,
                            )
                            .map_err(|_| StoreError::InvalidHistory)?;
                    }
                }
            }
        }
        Ok(())
    }

    async fn load(&self, run_id: &RunId) -> Result<QualifiedRun> {
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
    async fn append_admission(
        &self,
        frame: mfm_journal::single_trust::RunFrame,
        configuration: &ResolvedConfigurationHead,
    ) -> Result<AppendDisposition> {
        let store = OpenedStructuredStore {
            inner: Arc::clone(&self.inner),
        };
        store.verify_configuration_head(configuration).await?;
        let mfm_journal::single_trust::RunRecord::RunAdmitted(admission) = frame.record() else {
            return Err(StoreError::InvalidRecord);
        };
        if frame.expected_sequence() != 1
            || admission.tenant_scope_id() != self.inner.identity.tenant()
            || admission.run_id() != frame.run_id()
            || admission.store_scope_id() != self.inner.identity.scope()
            || admission.store_epoch() != self.inner.identity.epoch()
            || !Arc::ptr_eq(&configuration.brand, &self.inner.brand)
            || admission.configuration().sequence() != configuration.sequence
            || admission.configuration().content_ref() != &configuration.content_ref
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

fn qualified_material(
    catalog: &ProgramCatalog,
    value: &QualifiedValue,
) -> Result<(ValueRef, ImmutableObject)> {
    if !value.belongs_to_catalog(catalog) {
        return Err(StoreError::Identity);
    }
    let value_ref = ValueRef::new(value.contract_ref().clone(), value.value_ref().clone());
    let object = ImmutableObject::new(
        mfm_ids::StableId::new("mfm.value").map_err(|_| StoreError::InvalidRecord)?,
        value.value_ref().clone(),
        std::str::from_utf8(value.canonical_bytes())
            .map_err(|_| StoreError::InvalidRecord)?
            .to_owned(),
    )
    .map_err(|_| StoreError::InvalidRecord)?;
    Ok((value_ref, object))
}

fn concrete_material<T: MfmValue>(value: &T) -> Result<(ValueRef, ImmutableObject)> {
    let canonical = canonical_value(value).map_err(|_| StoreError::InvalidRecord)?;
    let content_ref = ContentRef::new(
        T::schema_id().map_err(|_| StoreError::InvalidRecord)?,
        raw_content_digest(canonical.as_bytes()),
    )
    .map_err(|_| StoreError::InvalidRecord)?;
    let value_ref = ValueRef::new(content_ref.clone(), content_ref.clone());
    let object = ImmutableObject::new(
        mfm_ids::StableId::new("mfm.value").map_err(|_| StoreError::InvalidRecord)?,
        content_ref,
        canonical.as_str().to_owned(),
    )
    .map_err(|_| StoreError::InvalidRecord)?;
    Ok((value_ref, object))
}

#[allow(clippy::type_complexity)]
fn proposed_outcome(
    catalog: &ProgramCatalog,
    document: &mfm_program::ProgramDocument,
    occurrence: &mfm_program::SequentialControlAddress,
    proposal: ProposedStateOutcome<QualifiedValue, QualifiedValue>,
) -> Result<(
    StateOutcome,
    ImmutableObject,
    Option<(ValueRef, ImmutableObject)>,
)> {
    let Some(mfm_program::Declaration::State(state)) = document.declaration(occurrence) else {
        return Err(StoreError::InvalidHistory);
    };
    match proposal {
        ProposedStateOutcome::Success { output, facts } => {
            if output.contract_ref() != state.output_contract_ref() {
                return Err(StoreError::InvalidRecord);
            }
            facts.validate().map_err(|_| StoreError::InvalidRecord)?;
            let (output_ref, object) = qualified_material(catalog, &output)?;
            let facts = (!facts.is_empty())
                .then(|| concrete_material(&facts))
                .transpose()?;
            Ok((StateOutcome::Success(output_ref), object, facts))
        }
        ProposedStateOutcome::Failure { failure } => {
            if state.failure_contract_ref() != Some(failure.contract_ref()) {
                return Err(StoreError::InvalidRecord);
            }
            let (failure_ref, object) = qualified_material(catalog, &failure)?;
            Ok((StateOutcome::Failure(failure_ref), object, None))
        }
    }
}

/// Cloneable callback-free history reader.
#[derive(Clone)]
pub struct HistoryReader {
    inner: Arc<OpenedStoreInner>,
}

impl HistoryReader {
    /// Returns immutable Store identity metadata for trusted composition checks.
    pub fn identity(&self) -> &StructuredStoreIdentity {
        &self.inner.identity
    }

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

/// Cloneable, opaque evidence of one exact resolved global configuration head.
#[derive(Debug, Clone)]
pub struct ResolvedConfigurationHead {
    sequence: u64,
    global_sequence: u64,
    total_bytes: usize,
    content_ref: ContentRef,
    brand: Arc<StoreBrand>,
    store: Arc<OpenedStoreInner>,
}

impl ResolvedConfigurationHead {
    /// Returns the exact one-based stream position of this typed configuration.
    pub const fn sequence(&self) -> u64 {
        self.sequence
    }

    /// Returns the exact typed content identity selected at this head.
    pub const fn content_ref(&self) -> &ContentRef {
        &self.content_ref
    }
}

/// One typed configuration value resolved from or promoted into the retained stream.
#[derive(Debug)]
pub struct ResolvedConfiguration<C: MfmConfig> {
    value: C,
    canonical_json: PlainCanonicalJsonBytes,
    head: ResolvedConfigurationHead,
}

impl<C: MfmConfig> ResolvedConfiguration<C> {
    /// Returns the validated typed configuration.
    pub const fn value(&self) -> &C {
        &self.value
    }

    /// Returns the exact canonical retained bytes.
    pub fn canonical_json(&self) -> &str {
        self.canonical_json.as_str()
    }

    /// Returns the opaque resolved head evidence.
    pub const fn head(&self) -> &ResolvedConfigurationHead {
        &self.head
    }

    /// Consumes this resolved value into its cloneable erased head evidence.
    pub fn into_head(self) -> ResolvedConfigurationHead {
        self.head
    }
}

/// Affine authority to prepare one exact successor against a resolved global head.
#[derive(Debug)]
pub struct ConfigurationWriteSession<C: MfmConfig> {
    expected_sequence: u64,
    prior_total_bytes: usize,
    brand: Arc<StoreBrand>,
    store: Arc<OpenedStoreInner>,
    _config: std::marker::PhantomData<fn() -> C>,
}

/// One affine typed configuration successor and its fixed physical append identity.
#[derive(Debug)]
pub struct PreparedConfigurationAppend<C: MfmConfig> {
    expected_sequence: u64,
    total_bytes: usize,
    append_request_id: AppendRequestId,
    value: C,
    canonical_json: PlainCanonicalJsonBytes,
    content_ref: ContentRef,
    brand: Arc<StoreBrand>,
    store: Arc<OpenedStoreInner>,
}

impl<C: MfmConfig> PreparedConfigurationAppend<C> {
    /// Returns the expected configuration head sequence.
    pub const fn expected_sequence(&self) -> u64 {
        self.expected_sequence
    }

    /// Returns the fixed physical append identity.
    pub const fn append_request_id(&self) -> &AppendRequestId {
        &self.append_request_id
    }

    /// Returns the cumulative byte count after this direct successor.
    pub const fn total_bytes(&self) -> usize {
        self.total_bytes
    }
}

/// Unknown-acknowledgement owner retaining the exact typed append for explicit resolution.
#[derive(Debug)]
pub struct SuspendedConfigurationAppend<C: MfmConfig> {
    owner: PreparedConfigurationAppend<C>,
}

impl<C: MfmConfig> SuspendedConfigurationAppend<C> {
    /// Returns the fixed physical append identity whose outcome must be resolved.
    pub const fn append_request_id(&self) -> &AppendRequestId {
        &self.owner.append_request_id
    }

    /// Consumes the suspension back into the only retryable append owner.
    pub fn into_owner(self) -> PreparedConfigurationAppend<C> {
        self.owner
    }
}

impl<C: MfmConfig> ConfigurationWriteSession<C> {
    /// Consumes this session and a locally validated typed value into one exact append owner.
    pub fn prepare_local(
        self,
        append_request_id: AppendRequestId,
        value: ValidatedConfig<C>,
    ) -> Result<PreparedConfigurationAppend<C>> {
        let canonical = value
            .canonical_json()
            .map_err(|_| StoreError::InvalidRecord)?;
        prepare_configuration_append(self, append_request_id, value.into_inner(), canonical)
    }

    /// Consumes this session and strict external canonical JSON into one exact append owner.
    pub fn prepare_external(
        self,
        append_request_id: AppendRequestId,
        source: &[u8],
    ) -> Result<PreparedConfigurationAppend<C>> {
        if source.is_empty()
            || source.len() > mfm_journal::single_trust::MAX_CONFIGURATION_REVISION_BYTES
        {
            return Err(StoreError::Capacity);
        }
        let text = std::str::from_utf8(source).map_err(|_| StoreError::InvalidRecord)?;
        let canonical =
            PlainCanonicalJsonBytes::from_json_str(text).map_err(|_| StoreError::InvalidRecord)?;
        if canonical.as_bytes() != source || string_contains_secret_marker(text) {
            return Err(StoreError::InvalidRecord);
        }
        let value: C = serde_json::from_slice(source).map_err(|_| StoreError::InvalidRecord)?;
        let value = ValidatedConfig::new(value).map_err(|_| StoreError::InvalidRecord)?;
        prepare_configuration_append(self, append_request_id, value.into_inner(), canonical)
    }
}

fn prepare_configuration_append<C: MfmConfig>(
    session: ConfigurationWriteSession<C>,
    append_request_id: AppendRequestId,
    value: C,
    canonical_json: PlainCanonicalJsonBytes,
) -> Result<PreparedConfigurationAppend<C>> {
    let bytes = canonical_json.as_bytes();
    if bytes.is_empty()
        || bytes.len() > mfm_journal::single_trust::MAX_CONFIGURATION_REVISION_BYTES
        || string_contains_secret_marker(canonical_json.as_str())
    {
        return Err(StoreError::InvalidRecord);
    }
    if session.expected_sequence as usize >= mfm_journal::single_trust::MAX_CONFIGURATION_REVISIONS
    {
        return Err(StoreError::Capacity);
    }
    let total_bytes = session
        .prior_total_bytes
        .checked_add(bytes.len())
        .filter(|total| *total <= mfm_journal::single_trust::MAX_CONFIGURATION_STREAM_BYTES)
        .ok_or(StoreError::Capacity)?;
    let schema = C::schema_id().map_err(|_| StoreError::InvalidRecord)?;
    let content_ref = ContentRef::new(schema, raw_content_digest(bytes))
        .map_err(|_| StoreError::InvalidRecord)?;
    Ok(PreparedConfigurationAppend {
        expected_sequence: session.expected_sequence,
        total_bytes,
        append_request_id,
        value,
        canonical_json,
        content_ref,
        brand: session.brand,
        store: session.store,
    })
}

impl ConfigurationStore {
    /// Returns immutable Store identity metadata for trusted composition checks.
    pub fn identity(&self) -> &StructuredStoreIdentity {
        &self.inner.identity
    }

    /// Returns whether opaque head evidence belongs to this exact Store opening.
    pub fn owns_head(&self, head: &ResolvedConfigurationHead) -> bool {
        Arc::ptr_eq(&head.brand, &self.inner.brand) && Arc::ptr_eq(&head.store, &self.inner)
    }
    /// Creates an affine first-write session. Backend CAS rejects it when the stream is nonempty.
    pub fn initial_write_session<C: MfmConfig>(&self) -> ConfigurationWriteSession<C> {
        ConfigurationWriteSession {
            expected_sequence: 0,
            prior_total_bytes: 0,
            brand: Arc::clone(&self.inner.brand),
            store: Arc::clone(&self.inner),
            _config: std::marker::PhantomData,
        }
    }

    /// Creates one affine typed successor session from an exact resolved head.
    pub fn write_session<C: MfmConfig>(
        &self,
        head: &ResolvedConfigurationHead,
    ) -> Result<ConfigurationWriteSession<C>> {
        if !Arc::ptr_eq(&head.brand, &self.inner.brand) || !Arc::ptr_eq(&head.store, &self.inner) {
            return Err(StoreError::Identity);
        }
        Ok(ConfigurationWriteSession {
            expected_sequence: head.global_sequence,
            prior_total_bytes: head.total_bytes,
            brand: Arc::clone(&head.brand),
            store: Arc::clone(&head.store),
            _config: std::marker::PhantomData,
        })
    }

    /// Loads the complete bounded stream and resolves its latest revision as exactly `C`.
    pub async fn load<C: MfmConfig>(&self) -> Result<ResolvedConfiguration<C>> {
        let raw = self
            .inner
            .backend
            .load_configuration()
            .await
            .map_err(map_backend_error)?;
        if raw.is_empty() {
            return Err(StoreError::NotFound);
        }
        let schema = C::schema_id().map_err(|_| StoreError::InvalidRecord)?;
        let (selected_index, selected_canonical) =
            validate_configuration_rows(&raw, Some(&schema))?.ok_or(StoreError::InvalidRecord)?;
        let selected = raw.get(selected_index).ok_or(StoreError::InvalidHistory)?;
        let global = raw.last().ok_or(StoreError::NotFound)?;
        ingress_retained_configuration_canonical::<C>(
            selected,
            selected_canonical,
            global.sequence(),
            global.total_bytes(),
            Arc::clone(&self.inner),
        )
    }

    /// Consumes one Store-branded configuration owner through the exact backend command.
    pub async fn commit<C: MfmConfig>(
        &self,
        owner: PreparedConfigurationAppend<C>,
    ) -> Result<ConfigurationCommitOutcome<C>> {
        if !Arc::ptr_eq(&owner.brand, &self.inner.brand) || !Arc::ptr_eq(&owner.store, &self.inner)
        {
            return Ok(ConfigurationCommitOutcome::Rejected {
                owner,
                error: StoreError::Identity,
            });
        }
        let command = ConfigurationAppendCommand::new(
            &self.inner.identity,
            owner.expected_sequence,
            &owner.append_request_id,
            owner.canonical_json.as_bytes(),
            &owner.content_ref,
        );
        let result = match self
            .inner
            .backend
            .compare_and_append_configuration(&command)
            .await
        {
            Ok(result) => result,
            Err(BackendError::AcknowledgementUnknown) => {
                return Ok(ConfigurationCommitOutcome::AcknowledgementUnknown(
                    SuspendedConfigurationAppend { owner },
                ));
            }
            Err(error) => {
                return Ok(ConfigurationCommitOutcome::Rejected {
                    owner,
                    error: map_backend_error(error),
                })
            }
        };
        match result {
            BackendConfigurationOutcome::NewlyCommitted => Ok(
                ConfigurationCommitOutcome::NewlyCommitted(promote_owner(owner)),
            ),
            BackendConfigurationOutcome::Found(raw) => {
                if raw.sequence() != owner.expected_sequence.saturating_add(1)
                    || raw.append_request_id() != &owner.append_request_id
                    || raw.canonical_bytes() != owner.canonical_json.as_bytes()
                    || raw.content_ref() != &owner.content_ref
                    || raw.total_bytes() != owner.total_bytes
                {
                    return Ok(ConfigurationCommitOutcome::Rejected {
                        owner,
                        error: StoreError::Conflict,
                    });
                }
                let resolved = ingress_retained_configuration::<C>(&raw, owner.store.clone());
                match resolved {
                    Ok(resolved) => Ok(ConfigurationCommitOutcome::Found(resolved)),
                    Err(error) => Ok(ConfigurationCommitOutcome::Rejected { owner, error }),
                }
            }
            BackendConfigurationOutcome::StaleHead { actual_sequence } => {
                Ok(ConfigurationCommitOutcome::StaleHead { actual_sequence })
            }
            BackendConfigurationOutcome::AcknowledgementUnknown => {
                Ok(ConfigurationCommitOutcome::AcknowledgementUnknown(
                    SuspendedConfigurationAppend { owner },
                ))
            }
        }
    }
}

fn promote_owner<C: MfmConfig>(owner: PreparedConfigurationAppend<C>) -> ResolvedConfiguration<C> {
    ResolvedConfiguration {
        value: owner.value,
        canonical_json: owner.canonical_json,
        head: ResolvedConfigurationHead {
            sequence: owner.expected_sequence.saturating_add(1),
            global_sequence: owner.expected_sequence.saturating_add(1),
            total_bytes: owner.total_bytes,
            content_ref: owner.content_ref,
            brand: owner.brand,
            store: owner.store,
        },
    }
}

fn validate_configuration_rows(
    rows: &[RawConfigurationRevision],
    selected_schema: Option<&mfm_ids::SchemaId>,
) -> Result<Option<(usize, PlainCanonicalJsonBytes)>> {
    if rows.len() > mfm_journal::single_trust::MAX_CONFIGURATION_REVISIONS {
        return Err(StoreError::Capacity);
    }
    let mut total_bytes = 0usize;
    let mut selected = None;
    for (index, row) in rows.iter().enumerate() {
        total_bytes = total_bytes
            .checked_add(row.canonical_bytes().len())
            .filter(|total| *total <= mfm_journal::single_trust::MAX_CONFIGURATION_STREAM_BYTES)
            .ok_or(StoreError::Capacity)?;
        if row.sequence() != index as u64 + 1 || row.total_bytes() != total_bytes {
            return Err(StoreError::InvalidHistory);
        }
        let canonical = validate_retained_configuration_bytes(row)?;
        if selected_schema.is_some_and(|schema| row.content_ref().schema_id() == schema) {
            selected = Some((index, canonical));
        }
    }
    Ok(selected)
}

fn validate_retained_configuration_bytes(
    row: &RawConfigurationRevision,
) -> Result<PlainCanonicalJsonBytes> {
    let text =
        std::str::from_utf8(row.canonical_bytes()).map_err(|_| StoreError::InvalidHistory)?;
    let canonical =
        PlainCanonicalJsonBytes::from_json_str(text).map_err(|_| StoreError::InvalidHistory)?;
    if canonical.as_bytes() != row.canonical_bytes()
        || string_contains_secret_marker(text)
        || row.content_ref().content_digest() != &raw_content_digest(row.canonical_bytes())
        || row
            .content_ref()
            .schema_id()
            .as_str()
            .contains(":mfm.configuration:")
    {
        return Err(StoreError::InvalidHistory);
    }
    Ok(canonical)
}

fn ingress_retained_configuration<C: MfmConfig>(
    row: &RawConfigurationRevision,
    store: Arc<OpenedStoreInner>,
) -> Result<ResolvedConfiguration<C>> {
    let canonical_json = validate_retained_configuration_bytes(row)?;
    ingress_retained_configuration_canonical::<C>(
        row,
        canonical_json,
        row.sequence(),
        row.total_bytes(),
        store,
    )
}

fn ingress_retained_configuration_canonical<C: MfmConfig>(
    row: &RawConfigurationRevision,
    canonical_json: PlainCanonicalJsonBytes,
    global_sequence: u64,
    global_total_bytes: usize,
    store: Arc<OpenedStoreInner>,
) -> Result<ResolvedConfiguration<C>> {
    if row.content_ref().schema_id() != &C::schema_id().map_err(|_| StoreError::InvalidRecord)? {
        return Err(StoreError::InvalidRecord);
    }
    let value: C =
        serde_json::from_slice(row.canonical_bytes()).map_err(|_| StoreError::InvalidHistory)?;
    let value = ValidatedConfig::new(value)
        .map_err(|_| StoreError::InvalidHistory)?
        .into_inner();
    Ok(ResolvedConfiguration {
        value,
        canonical_json,
        head: ResolvedConfigurationHead {
            sequence: row.sequence(),
            global_sequence,
            total_bytes: global_total_bytes,
            content_ref: row.content_ref().clone(),
            brand: Arc::clone(&store.brand),
            store,
        },
    })
}

/// Non-Clone fixed-snapshot audit port.
pub struct StoreAuditPort {
    inner: Arc<OpenedStoreInner>,
}

impl StoreAuditPort {
    /// Returns immutable Store identity metadata for trusted composition checks.
    pub fn identity(&self) -> &StructuredStoreIdentity {
        &self.inner.identity
    }

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
#[path = "../tests/backend_unit.rs"]
mod tests;

#[cfg(all(test, feature = "test-support"))]
mod fault_tests {
    use super::*;
    use mfm_canonical::raw_content_digest;
    use mfm_ids::{DigestAlgorithm, DigestBytes, SchemaId};
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Serialize, Deserialize, mfm_program_derive::MfmValue)]
    #[serde(deny_unknown_fields)]
    struct FaultValue {
        value: u64,
    }

    #[derive(Debug, Serialize, Deserialize, mfm_program_derive::MfmConfig)]
    #[serde(deny_unknown_fields)]
    struct FaultConfig {
        value: u64,
    }

    #[tokio::test]
    async fn configuration_unknown_acknowledgement_retains_owner_until_found() {
        let identity = StructuredStoreIdentity::new(
            StoreScopeId::new("mfm.store_scope.v1:0123456789abcdef0123456789abcdef")
                .expect("scope"),
            StoreEpoch::new(1),
            TenantScopeId::new("mfm.tenant_scope.v1:0123456789abcdef0123456789abcdef")
                .expect("tenant"),
        );
        let contract = mfm_program::nominal_contract_ref::<FaultValue>().expect("contract");
        let document = mfm_program::single_trust::ProgramDocument::new(
            mfm_ids::StableId::new("mfm.test.configuration-unknown-entry").expect("entry"),
            contract.clone(),
            contract,
            Vec::new(),
        )
        .expect("document");
        let mut builder = ProgramCatalog::builder();
        builder
            .register_value::<FaultValue>()
            .expect("registration");
        let (catalog, _) = builder.finish(document).expect("catalog");
        let backend = Arc::new(MemoryStructuredBackend::new(identity.clone()));
        let opened = StructuredStore::open(
            backend.clone(),
            identity,
            catalog,
            StoreWorkLimits::default(),
        )
        .await
        .expect("store");
        let configuration = opened.test_configuration();
        let request = AppendRequestId::new("configuration-unknown-owner-000001").expect("request");
        let owner = configuration
            .initial_write_session::<FaultConfig>()
            .prepare_local(
                request.clone(),
                ValidatedConfig::new(FaultConfig { value: 1 }).expect("config"),
            )
            .expect("owner");
        backend.fail_next_configuration_acknowledgement();
        let suspended = match configuration.commit(owner).await.expect("commit") {
            ConfigurationCommitOutcome::AcknowledgementUnknown(suspended) => suspended,
            other => panic!("unexpected unknown outcome: {other:?}"),
        };
        assert_eq!(suspended.append_request_id(), &request);
        match configuration
            .commit(suspended.into_owner())
            .await
            .expect("resolution")
        {
            ConfigurationCommitOutcome::Found(resolved) => {
                assert_eq!(resolved.head().sequence(), 1);
                assert_eq!(resolved.value().value, 1);
            }
            other => panic!("unexpected resolution outcome: {other:?}"),
        }
    }

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
            BackendConfigurationOutcome::Found(revision) if revision.sequence() == 1
        ));
    }
}
