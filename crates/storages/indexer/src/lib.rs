//! Derived projection/indexer store scaffold (Milestone 6).
//!
//! This crate is intentionally a scaffold:
//! - projections are derived-only and must never become correctness-critical for resume/replay
//! - wiring and schema migrations will be implemented incrementally

use mfm_machine::errors::{ErrorCategory, ErrorInfo, StorageError};
use mfm_machine::events::EventEnvelope;
use mfm_machine::ids::{ErrorCode, RunId};

fn info(code: &'static str, message: &'static str) -> ErrorInfo {
    ErrorInfo {
        code: ErrorCode(code.to_string()),
        category: ErrorCategory::Storage,
        retryable: false,
        message: message.to_string(),
        details: None,
    }
}

fn other(code: &'static str, message: &'static str) -> StorageError {
    StorageError::Other(info(code, message))
}

#[derive(Clone, Debug)]
pub struct ProjectionIndexer;

impl ProjectionIndexer {
    pub fn new() -> Self {
        Self
    }

    /// Apply schema migrations (scaffold).
    pub async fn migrate(&self) -> Result<(), StorageError> {
        Err(other(
            "projection_indexer_unimplemented",
            "projection indexer is not implemented yet",
        ))
    }

    /// Ingest events for projections (scaffold).
    pub async fn ingest(
        &self,
        _run_id: RunId,
        _events: &[EventEnvelope],
    ) -> Result<(), StorageError> {
        Err(other(
            "projection_indexer_unimplemented",
            "projection indexer is not implemented yet",
        ))
    }
}

impl Default for ProjectionIndexer {
    fn default() -> Self {
        Self::new()
    }
}
