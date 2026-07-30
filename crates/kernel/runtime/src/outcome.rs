//! Public result of one stateless drive operation.

use mfm_journal::v2::{ClosureRef, JournalHead};

/// Why a verified open run cannot make another automatic step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriveWaitReason {
    /// Every currently consumable observation is valid but insufficient.
    RetryableEvidenceGap,
    /// An exact deployment capability or other operational prerequisite is unavailable.
    OperationalBlock,
    /// Invalid evidence or another integrity finding prevents execution.
    IntegrityBlock,
}

/// Result of one post-admission runtime action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DriveOutcome {
    /// One semantic transition or audited protocol operation advanced the journal.
    Advanced {
        /// Exact physical head observed after the action.
        journal_head: JournalHead,
    },
    /// No further automatic action is currently legal.
    Waiting {
        /// Exact physical head at which waiting was derived.
        journal_head: JournalHead,
        /// Closed reason class.
        reason: DriveWaitReason,
    },
    /// The semantic run is closed.
    Closed {
        /// Fixed semantic closure coordinate, independent of a later audit tail.
        closure_ref: ClosureRef,
    },
}
