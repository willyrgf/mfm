#![warn(missing_docs)]
//! Mechanical append-only storage for sealed Journal frames.
//!
//! Store owns physical atomicity and complete-prefix snapshots. Journal alone
//! qualifies the returned bytes; Store has no Program or domain semantics.

use std::collections::BTreeMap;
use std::future::Future;
use std::ops::Bound::{Excluded, Unbounded};
use std::pin::Pin;
use std::sync::Arc;

use mfm_ids::{ContentDigest, RunId};
use mfm_journal::{
    frame_head_digest, EncodedRunFrame, StoredRunBytes, MAX_FRAME_BYTES, MAX_RUN_BYTES,
    MAX_RUN_FRAMES,
};
use serde::Serialize;
use tokio::sync::Mutex;

/// Maximum number of items returned by one run-index page.
pub const MAX_RUN_PAGE_ITEMS: usize = 200;
/// Default number of run heads requested by client surfaces.
pub const DEFAULT_RUN_PAGE_ITEMS: usize = 50;

/// Checked run-page size error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("page limit is invalid")]
pub struct RunPageLimitError;

/// A run-head page size in the inclusive range 1 through 200.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunPageLimit(usize);

impl RunPageLimit {
    /// Checks a requested page size.
    pub const fn new(value: usize) -> Result<Self, RunPageLimitError> {
        if value == 0 || value > MAX_RUN_PAGE_ITEMS {
            return Err(RunPageLimitError);
        }
        Ok(Self(value))
    }

    /// Returns the checked page size.
    pub const fn get(self) -> usize {
        self.0
    }
}

impl Default for RunPageLimit {
    fn default() -> Self {
        Self(DEFAULT_RUN_PAGE_ITEMS)
    }
}

/// Error returned when one run-head projection is structurally invalid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("run summary is invalid")]
pub struct RunSummaryError;

/// Mechanical current-head projection for one run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RunSummary {
    run_id: RunId,
    head_sequence: u64,
    head_digest: ContentDigest,
    total_bytes: u64,
}

impl RunSummary {
    /// Constructs a structurally checked current-head projection.
    pub fn new(
        run_id: RunId,
        head_sequence: u64,
        head_digest: ContentDigest,
        total_bytes: u64,
    ) -> Result<Self, RunSummaryError> {
        if head_sequence == 0
            || head_digest.algorithm() != mfm_ids::DigestAlgorithm::Sha256V1
            || total_bytes == 0
        {
            return Err(RunSummaryError);
        }
        Ok(Self {
            run_id,
            head_sequence,
            head_digest,
            total_bytes,
        })
    }

    /// Returns the run identity.
    pub const fn run_id(&self) -> &RunId {
        &self.run_id
    }

    /// Returns the current retained frame sequence.
    pub const fn head_sequence(&self) -> u64 {
        self.head_sequence
    }

    /// Returns the exact current frame digest.
    pub const fn head_digest(&self) -> &ContentDigest {
        &self.head_digest
    }

    /// Returns cumulative retained frame bytes.
    pub const fn total_bytes(&self) -> u64 {
        self.total_bytes
    }
}

/// Redaction-safe mechanical run-index failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RunIndexError {
    /// Retained head rows were inconsistent.
    #[error("run index is corrupt")]
    Corrupt,
    /// The index could not be read.
    #[error("run index is unavailable")]
    Unavailable,
}

/// One bounded mechanical run-head page with an exclusive continuation identity.
#[derive(Serialize)]
pub struct RunPage {
    items: Vec<RunSummary>,
    next_after: Option<RunId>,
}

impl RunPage {
    /// Constructs a structurally checked run page.
    pub fn new(items: Vec<RunSummary>, next_after: Option<RunId>) -> Result<Self, RunIndexError> {
        if items.len() > MAX_RUN_PAGE_ITEMS
            || items
                .windows(2)
                .any(|pair| pair[0].run_id >= pair[1].run_id)
            || next_after
                .as_ref()
                .is_some_and(|after| items.last().is_none_or(|summary| after != summary.run_id()))
        {
            return Err(RunIndexError::Corrupt);
        }
        Ok(Self { items, next_after })
    }

    /// Returns the ordered summaries.
    pub fn items(&self) -> &[RunSummary] {
        &self.items
    }

    /// Returns the last returned RunId when another run was observed.
    pub const fn next_after(&self) -> Option<&RunId> {
        self.next_after.as_ref()
    }

    /// Consumes the page into owned items and continuation identity.
    pub fn into_parts(self) -> (Vec<RunSummary>, Option<RunId>) {
        (self.items, self.next_after)
    }
}

/// Object-safe mechanical current-head index.
pub trait RunIndex: Send + Sync {
    /// Lists one ascending keyset page without parsing run history.
    fn list_runs<'a>(
        &'a self,
        after: Option<&'a RunId>,
        limit: RunPageLimit,
    ) -> Pin<Box<dyn Future<Output = Result<RunPage, RunIndexError>> + Send + 'a>>;
}

/// The only mechanical append outcomes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppendResult {
    /// This invocation atomically inserted the candidate.
    Inserted,
    /// This invocation wrote nothing.
    NotInserted,
}

/// Redaction-safe Store failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum StoreError {
    /// The insertion candidate exceeded a fixed format capacity.
    #[error("frame {0}")]
    FrameSize(mfm_journal::SizeLimitExceeded),
    /// The accumulated run bytes exceeded their limit.
    #[error("history {0}")]
    HistorySize(mfm_journal::SizeLimitExceeded),
    /// The run frame count exceeded its limit.
    #[error("frame count {0}")]
    FrameCount(mfm_journal::SizeLimitExceeded),
    /// Capacity arithmetic could not represent the result.
    #[error("store capacity arithmetic overflow")]
    ArithmeticOverflow,
    /// Retained physical rows or metadata were inconsistent.
    #[error("store physical state is corrupt")]
    CorruptPhysicalState,
    /// The operation definitely did not complete.
    #[error("store operation is unavailable")]
    Unavailable,
    /// An append may have committed but acknowledgement was unavailable.
    #[error("store append outcome is indeterminate")]
    Indeterminate,
}

/// Object-safe mechanical Store contract.
pub trait Store: Send + Sync {
    /// Loads one atomically captured complete prefix, or absence.
    fn load_run<'a>(
        &'a self,
        run_id: &'a RunId,
    ) -> Pin<
        Box<
            dyn Future<Output = std::result::Result<Option<StoredRunBytes>, StoreError>>
                + Send
                + 'a,
        >,
    >;

    /// Atomically appends one sealed frame.
    fn append_run<'a>(
        &'a self,
        frame: &'a EncodedRunFrame,
    ) -> Pin<Box<dyn Future<Output = std::result::Result<AppendResult, StoreError>> + Send + 'a>>;
}

/// In-memory Store with per-run append serialization.
pub struct MemoryStore {
    runs: Mutex<BTreeMap<RunId, Arc<Mutex<MemoryRun>>>>,
}

struct MemoryRun {
    frames: Vec<Arc<StoredFrame>>,
    head: Option<Head>,
}

struct StoredFrame {
    bytes: Vec<u8>,
    head_digest: ContentDigest,
}

#[derive(Clone)]
struct Head {
    sequence: u64,
    total_bytes: u64,
}

impl MemoryStore {
    /// Constructs an empty Store.
    pub fn new() -> Self {
        Self {
            runs: Mutex::new(BTreeMap::new()),
        }
    }

    async fn run_for(&self, run_id: &RunId) -> Arc<Mutex<MemoryRun>> {
        let mut runs = self.runs.lock().await;
        runs.entry(run_id.clone())
            .or_insert_with(|| {
                Arc::new(Mutex::new(MemoryRun {
                    frames: Vec::new(),
                    head: None,
                }))
            })
            .clone()
    }
}

impl Default for MemoryStore {
    fn default() -> Self {
        Self::new()
    }
}

impl Store for MemoryStore {
    fn load_run<'a>(
        &'a self,
        run_id: &'a RunId,
    ) -> Pin<
        Box<
            dyn Future<Output = std::result::Result<Option<StoredRunBytes>, StoreError>>
                + Send
                + 'a,
        >,
    > {
        Box::pin(async move {
            require_runtime()?;
            let run = {
                let runs = self.runs.lock().await;
                runs.get(run_id).cloned()
            };
            let Some(run) = run else {
                return Ok(None);
            };
            let (head, frames) = {
                let run = run.lock_owned().await;
                if run.head.is_none() && run.frames.is_empty() {
                    return Ok(None);
                }
                let head = run.head.clone();
                let frames = cooperative_frame_snapshot(&run.frames).await?;
                (head, frames)
            };
            run_pure_blocking(move || validate_and_copy_snapshot(head, frames)).await
        })
    }

    fn append_run<'a>(
        &'a self,
        frame: &'a EncodedRunFrame,
    ) -> Pin<Box<dyn Future<Output = std::result::Result<AppendResult, StoreError>> + Send + 'a>>
    {
        Box::pin(async move {
            require_runtime()?;
            let run_id = frame.run_id().clone();
            let sequence = frame.run_sequence();
            let predecessor = frame.previous_head_digest().cloned();
            let head_digest = frame.head_digest().clone();
            let bytes = own_candidate_bytes(frame.canonical_bytes()).await?;
            let run = self.run_for(&run_id).await;
            let mut guard = run.lock_owned().await;
            let head = guard.head.clone();
            let observed_count = guard.frames.len();
            let current = head
                .as_ref()
                .and_then(|head| head.sequence.checked_sub(1))
                .and_then(|index| usize::try_from(index).ok())
                .and_then(|index| guard.frames.get(index))
                .cloned();
            let target = sequence
                .checked_sub(1)
                .and_then(|index| usize::try_from(index).ok())
                .and_then(|index| guard.frames.get(index))
                .cloned();
            let candidate = Candidate {
                sequence,
                predecessor,
                head_digest,
                bytes,
            };
            let plan = run_pure_blocking(move || {
                plan_append(head, observed_count, current, target, candidate)
            })
            .await?;
            match plan {
                AppendPlan::NotInserted => Ok(AppendResult::NotInserted),
                AppendPlan::Insert { stored, head } => {
                    publish_insert(&mut guard, stored, head, |frames| {
                        frames
                            .try_reserve_exact(1)
                            .map_err(|_| StoreError::Unavailable)
                    })?;
                    Ok(AppendResult::Inserted)
                }
            }
        })
    }
}

impl RunIndex for MemoryStore {
    fn list_runs<'a>(
        &'a self,
        after: Option<&'a RunId>,
        limit: RunPageLimit,
    ) -> Pin<Box<dyn Future<Output = Result<RunPage, RunIndexError>> + Send + 'a>> {
        Box::pin(async move {
            let runs = self.runs.lock().await;
            let start = after.map_or(Unbounded, Excluded);
            let mut page = Vec::with_capacity(limit.get() + 1);
            for (run_id, run) in runs.range::<RunId, _>((start, Unbounded)) {
                let run = run.lock().await;
                let Some(head) = &run.head else {
                    if run.frames.is_empty() {
                        continue;
                    }
                    return Err(RunIndexError::Corrupt);
                };
                if head.sequence == 0
                    || head.sequence > MAX_RUN_FRAMES
                    || usize::try_from(head.sequence).ok() != Some(run.frames.len())
                    || head.total_bytes == 0
                    || head.total_bytes > MAX_RUN_BYTES
                {
                    return Err(RunIndexError::Corrupt);
                }
                let current = run.frames.last().ok_or(RunIndexError::Corrupt)?;
                let summary = RunSummary::new(
                    run_id.clone(),
                    head.sequence,
                    current.head_digest.clone(),
                    head.total_bytes,
                )
                .map_err(|_| RunIndexError::Corrupt)?;
                page.push(summary);
                if page.len() > limit.get() {
                    break;
                }
            }
            let has_more = page.len() > limit.get();
            if has_more {
                page.pop();
            }
            let next_after = if has_more {
                page.last().map(|summary| summary.run_id().clone())
            } else {
                None
            };
            RunPage::new(page, next_after)
        })
    }
}

fn publish_insert<F>(
    run: &mut MemoryRun,
    stored: StoredFrame,
    head: Head,
    reserve: F,
) -> std::result::Result<(), StoreError>
where
    F: FnOnce(&mut Vec<Arc<StoredFrame>>) -> std::result::Result<(), StoreError>,
{
    reserve(&mut run.frames)?;
    run.frames.push(Arc::new(stored));
    run.head = Some(head);
    Ok(())
}

struct Candidate {
    sequence: u64,
    predecessor: Option<ContentDigest>,
    head_digest: ContentDigest,
    bytes: Vec<u8>,
}

enum AppendPlan {
    NotInserted,
    Insert { stored: StoredFrame, head: Head },
}

fn plan_append(
    head: Option<Head>,
    observed_count: usize,
    current: Option<Arc<StoredFrame>>,
    target: Option<Arc<StoredFrame>>,
    candidate: Candidate,
) -> std::result::Result<AppendPlan, StoreError> {
    validate_append_observation(
        &head,
        observed_count,
        current.as_deref(),
        target.as_deref(),
        candidate.sequence,
    )?;
    if let Some(existing) = target {
        if existing.bytes == candidate.bytes && existing.head_digest == candidate.head_digest {
            return Ok(AppendPlan::NotInserted);
        }
    }

    let predecessor_matches = match &head {
        None => candidate.sequence == 1 && candidate.predecessor.is_none(),
        Some(head) => {
            candidate.sequence == head.sequence.saturating_add(1)
                && candidate.predecessor.as_ref()
                    == current.as_ref().map(|frame| &frame.head_digest)
        }
    };
    if !predecessor_matches {
        return Ok(AppendPlan::NotInserted);
    }

    let frame_len =
        u64::try_from(candidate.bytes.len()).map_err(|_| StoreError::ArithmeticOverflow)?;
    mfm_journal::SizeLimitExceeded::check(frame_len, MAX_FRAME_BYTES as u64)
        .map_err(StoreError::FrameSize)?;
    mfm_journal::SizeLimitExceeded::check(candidate.sequence, MAX_RUN_FRAMES)
        .map_err(StoreError::FrameCount)?;
    let current_total = head.as_ref().map_or(0, |current| current.total_bytes);
    let remaining = MAX_RUN_BYTES
        .checked_sub(current_total)
        .ok_or(StoreError::CorruptPhysicalState)?;
    if frame_len > remaining {
        let total = current_total
            .checked_add(frame_len)
            .ok_or(StoreError::ArithmeticOverflow)?;
        mfm_journal::SizeLimitExceeded::check(total, MAX_RUN_BYTES)
            .map_err(StoreError::HistorySize)?;
    }
    if frame_head_digest(&candidate.bytes) != candidate.head_digest {
        return Err(StoreError::CorruptPhysicalState);
    }
    let total_bytes = head
        .as_ref()
        .map_or(0, |current| current.total_bytes)
        .checked_add(frame_len)
        .ok_or(StoreError::ArithmeticOverflow)?;
    Ok(AppendPlan::Insert {
        stored: StoredFrame {
            bytes: candidate.bytes,
            head_digest: candidate.head_digest,
        },
        head: Head {
            sequence: candidate.sequence,
            total_bytes,
        },
    })
}

fn validate_append_observation(
    head: &Option<Head>,
    observed_count: usize,
    current: Option<&StoredFrame>,
    target: Option<&StoredFrame>,
    candidate_sequence: u64,
) -> std::result::Result<(), StoreError> {
    let Some(head) = head else {
        return (observed_count == 0 && current.is_none() && target.is_none())
            .then_some(())
            .ok_or(StoreError::CorruptPhysicalState);
    };
    if head.sequence == 0
        || head.sequence > MAX_RUN_FRAMES
        || usize::try_from(head.sequence).ok() != Some(observed_count)
        || head.total_bytes == 0
        || head.total_bytes > MAX_RUN_BYTES
    {
        return Err(StoreError::CorruptPhysicalState);
    }
    let current = current.ok_or(StoreError::CorruptPhysicalState)?;
    if current.bytes.is_empty()
        || current.bytes.len() > MAX_FRAME_BYTES
        || frame_head_digest(&current.bytes) != current.head_digest
        || (candidate_sequence <= head.sequence && target.is_none())
    {
        return Err(StoreError::CorruptPhysicalState);
    }
    if let Some(target) = target {
        if target.bytes.is_empty()
            || target.bytes.len() > MAX_FRAME_BYTES
            || frame_head_digest(&target.bytes) != target.head_digest
        {
            return Err(StoreError::CorruptPhysicalState);
        }
    }
    Ok(())
}

fn validate_physical(
    head: &Option<Head>,
    frames: &[Arc<StoredFrame>],
) -> std::result::Result<(), StoreError> {
    let Some(head) = head else {
        return frames
            .is_empty()
            .then_some(())
            .ok_or(StoreError::CorruptPhysicalState);
    };
    if head.sequence == 0
        || head.sequence > MAX_RUN_FRAMES
        || usize::try_from(head.sequence).ok() != Some(frames.len())
        || head.total_bytes > MAX_RUN_BYTES
        || frames.is_empty()
    {
        return Err(StoreError::CorruptPhysicalState);
    }
    let mut total = 0_u64;
    for frame in frames {
        if frame.bytes.is_empty()
            || frame.bytes.len() > MAX_FRAME_BYTES
            || frame_head_digest(&frame.bytes) != frame.head_digest
        {
            return Err(StoreError::CorruptPhysicalState);
        }
        total = total
            .checked_add(
                u64::try_from(frame.bytes.len()).map_err(|_| StoreError::CorruptPhysicalState)?,
            )
            .ok_or(StoreError::CorruptPhysicalState)?;
    }
    if total != head.total_bytes {
        return Err(StoreError::CorruptPhysicalState);
    }
    Ok(())
}

fn validate_and_copy_snapshot(
    head: Option<Head>,
    frames: Vec<Arc<StoredFrame>>,
) -> std::result::Result<Option<StoredRunBytes>, StoreError> {
    validate_physical(&head, &frames)?;
    if head.is_none() {
        return Ok(None);
    }
    let mut copied = Vec::new();
    copied
        .try_reserve_exact(frames.len())
        .map_err(|_| StoreError::Unavailable)?;
    for frame in frames {
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(frame.bytes.len())
            .map_err(|_| StoreError::Unavailable)?;
        bytes.extend_from_slice(&frame.bytes);
        copied.push(bytes);
    }
    StoredRunBytes::new(copied)
        .map(Some)
        .map_err(|_| StoreError::CorruptPhysicalState)
}

async fn own_candidate_bytes(source: &[u8]) -> std::result::Result<Vec<u8>, StoreError> {
    let length = source.len();
    let mut owned = run_pure_blocking(move || {
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(length)
            .map_err(|_| StoreError::Unavailable)?;
        Ok(bytes)
    })
    .await?;

    for chunk in source.chunks(256 * 1024) {
        owned.extend_from_slice(chunk);
        tokio::task::yield_now().await;
    }
    Ok(owned)
}

async fn cooperative_frame_snapshot(
    source: &[Arc<StoredFrame>],
) -> std::result::Result<Vec<Arc<StoredFrame>>, StoreError> {
    let length = source.len();
    let mut snapshot = run_pure_blocking(move || {
        let mut frames = Vec::new();
        frames
            .try_reserve_exact(length)
            .map_err(|_| StoreError::Unavailable)?;
        Ok(frames)
    })
    .await?;
    for chunk in source.chunks(1_024) {
        snapshot.extend(chunk.iter().cloned());
        tokio::task::yield_now().await;
    }
    Ok(snapshot)
}

async fn run_pure_blocking<T, F>(job: F) -> std::result::Result<T, StoreError>
where
    T: Send + 'static,
    F: FnOnce() -> std::result::Result<T, StoreError> + Send + 'static,
{
    require_runtime()?;
    tokio::task::spawn_blocking(job)
        .await
        .map_err(|_| StoreError::Unavailable)?
}

fn require_runtime() -> std::result::Result<(), StoreError> {
    tokio::runtime::Handle::try_current()
        .map(|_| ())
        .map_err(|_| StoreError::Unavailable)
}

#[cfg(test)]
mod tests;
