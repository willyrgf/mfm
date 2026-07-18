#![warn(missing_docs)]
//! Generic internal fact capability contracts.
//!
//! This crate defines the state/adapter-facing authority contracts for:
//! - internal Control and Platform reads from the MFM fact index
//! - managed Platform fact publication
//!
//! Concrete store implementations, SQL query execution, app wiring, and public
//! fact DTO services live outside this crate. Platform index reads still require
//! certified evidence and retained response artifacts; they are not a
//! public-facts authority path.
//!
//! ```rust
//! use mfm_capabilities::CapabilitySpec;
//! use mfm_fact_capabilities::{FactIndexReadCapability, FactRecordCapability};
//!
//! assert_eq!(FactIndexReadCapability::name(), "mfm.fact.index.read");
//! assert_eq!(FactRecordCapability::name(), "mfm.fact.record");
//! ```

use std::fmt;
use std::future::Future;
use std::pin::Pin;

use mfm_canonical::sha256_digest_bytes;
use mfm_capabilities::{
    CapabilityError, CapabilitySpec, ManagedPlatformWriteRole, ReadExternalRole,
};
use mfm_facts::{CanonicalFactQueryPlan, FactAudience, FactQueryResult};
use mfm_ids::{CapabilityKind, CapabilityVersion, DigestAlgorithm};

/// Result type for fact-index read capability contracts.
pub type Result<T> = std::result::Result<T, FactIndexReadError>;

/// Boxed future returned by fact-index read providers.
pub type FactIndexReadFuture<'a> =
    Pin<Box<dyn Future<Output = Result<FactQueryResult>> + Send + 'a>>;

/// Boxed future returned by batch fact-index read providers.
pub type FactIndexReadBatchFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Vec<FactQueryResult>>> + Send + 'a>>;

/// Internal fact-index read authority.
pub struct FactIndexReadCapability;

impl CapabilitySpec for FactIndexReadCapability {
    type Role = ReadExternalRole;

    fn kind() -> mfm_capabilities::Result<CapabilityKind> {
        CapabilityKind::new(
            "mfm.fact",
            "index.read",
            DigestAlgorithm::Sha256JcsV1,
            sha256_digest_bytes(b"mfm.fact.capability:index.read"),
        )
        .map_err(|error| CapabilityError::Identity(error.to_string()))
    }

    fn version() -> mfm_capabilities::Result<CapabilityVersion> {
        CapabilityVersion::new("mfm.fact.index.read.v1")
            .map_err(|error| CapabilityError::Identity(error.to_string()))
    }

    fn name() -> &'static str {
        "mfm.fact.index.read"
    }
}

/// Shared managed Platform-write capability for recording fact claims.
///
/// Used by Bitcoin fact-record states and portfolio EVM publication. Family
/// monomorphism lives in fact types, not in a second capability identity.
pub struct FactRecordCapability;

impl CapabilitySpec for FactRecordCapability {
    type Role = ManagedPlatformWriteRole;

    fn kind() -> mfm_capabilities::Result<CapabilityKind> {
        CapabilityKind::new(
            "mfm.fact",
            "record",
            DigestAlgorithm::Sha256JcsV1,
            sha256_digest_bytes(b"mfm.fact.capability:record"),
        )
        .map_err(|error| CapabilityError::Identity(error.to_string()))
    }

    fn version() -> mfm_capabilities::Result<CapabilityVersion> {
        CapabilityVersion::new("mfm.fact.record.v1")
            .map_err(|error| CapabilityError::Identity(error.to_string()))
    }

    fn name() -> &'static str {
        "mfm.fact.record"
    }
}

/// Provider interface for internal fact-index reads.
pub trait FactIndexReadProvider: Send + Sync {
    /// Returns the stable identity of the concrete process-level fact-index implementation.
    fn implementation_id(&self) -> &'static str;

    /// Executes an already-compiled canonical fact query plan.
    ///
    /// Providers must apply the complete canonical query shape, including any exact fact-content
    /// identity narrowing evidence, before ordering and limiting returned rows.
    ///
    /// Default path runs a one-element [`Self::read_fact_index_batch`] so single and multi
    /// reads share one provider implementation.
    fn read_fact_index<'a>(&'a self, request: &'a FactIndexReadRequest) -> FactIndexReadFuture<'a> {
        Box::pin(async move {
            let mut responses = self
                .read_fact_index_batch(std::slice::from_ref(request))
                .await?;
            responses.pop().ok_or_else(|| {
                FactIndexReadError::redacted_provider_failure(
                    "fact-index batch returned no response for single request",
                )
            })
        })
    }

    /// Executes multiple plans under **one shared store read snapshot**.
    ///
    /// Multi-holding portfolio selection must use this entry point so candidate sets share
    /// one selection frontier. Implementations must not open independent snapshots per
    /// request when `requests.len() > 1`. Empty input returns an empty vec.
    fn read_fact_index_batch<'a>(
        &'a self,
        requests: &'a [FactIndexReadRequest],
    ) -> FactIndexReadBatchFuture<'a>;
}

/// Request to read internal Control or Platform facts from the fact index.
///
/// Certified states may query both audiences through this single capability.
/// Platform reads still require certified evidence and retained response
/// artifacts; they are not a public-facts authority path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactIndexReadRequest {
    plan: CanonicalFactQueryPlan,
}

impl FactIndexReadRequest {
    /// Creates a fact-index read request from an already-compiled plan.
    ///
    /// Accepts plans scoped to [`FactAudience::Control`] or
    /// [`FactAudience::Platform`]. Other audiences fail closed.
    pub fn new(plan: CanonicalFactQueryPlan) -> Result<Self> {
        if !is_supported_fact_index_audience(plan.query_scope().audience()) {
            return Err(FactIndexReadError::InvalidRequest {
                reason: FactIndexInvalidRequest::UnsupportedAudience,
            });
        }
        Ok(Self { plan })
    }

    /// Returns the compiled canonical query plan.
    pub const fn plan(&self) -> &CanonicalFactQueryPlan {
        &self.plan
    }
}

/// Closed invalid-request reasons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FactIndexInvalidRequest {
    /// The compiled plan audience is outside the certified fact-index allowlist.
    UnsupportedAudience,
}

/// Returns whether an audience may be used with certified fact-index reads.
///
/// Allowlist: Control (operational cursors) and Platform (reportable holdings).
const fn is_supported_fact_index_audience(audience: FactAudience) -> bool {
    matches!(audience, FactAudience::Control | FactAudience::Platform)
}

/// Closed provider failure reasons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FactIndexProviderFailure {
    /// Provider failed without exposing SQL, URLs, store internals, or raw backend diagnostics.
    Failed,
}

/// Redaction-safe fact-index read capability error.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FactIndexReadError {
    /// Request failed contract validation.
    #[error("fact-index read request was invalid")]
    InvalidRequest {
        /// Closed invalid-request reason.
        reason: FactIndexInvalidRequest,
    },
    /// Provider failed without exposing backend details.
    #[error("fact-index read provider failed")]
    Provider {
        /// Closed provider failure reason.
        reason: FactIndexProviderFailure,
    },
}

impl FactIndexReadError {
    /// Builds a redacted provider failure, discarding raw backend diagnostics.
    pub fn redacted_provider_failure(_source: impl fmt::Display) -> Self {
        Self::Provider {
            reason: FactIndexProviderFailure::Failed,
        }
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
