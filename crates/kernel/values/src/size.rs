//! Numeric evidence for a measured size-limit violation.

/// A measured size strictly greater than its allowed limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, thiserror::Error)]
#[error("size limit exceeded: actual {actual}, limit {limit}")]
pub struct SizeLimitExceeded {
    actual: u64,
    limit: u64,
}

impl SizeLimitExceeded {
    /// Checks a measured byte size or count against its inclusive limit.
    pub const fn check(actual: u64, limit: u64) -> Result<(), Self> {
        if actual > limit {
            Err(Self { actual, limit })
        } else {
            Ok(())
        }
    }

    /// Returns the measured size or count.
    pub const fn actual(self) -> u64 {
        self.actual
    }

    /// Returns the inclusive allowed size or count.
    pub const fn limit(self) -> u64 {
        self.limit
    }
}

/// Resource measured by a Runtime size-limit failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SizeResource {
    /// One canonical typed value.
    CanonicalObject,
    /// One complete Journal frame.
    Frame,
    /// Non-payload frame metadata.
    FrameEnvelope,
    /// The complete run's accumulated frame bytes.
    HistoryBytes,
    /// The complete run's frame count.
    FrameCount,
    /// The derived inline terminal failure report.
    FailureReport,
}

impl std::fmt::Display for SizeResource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::CanonicalObject => "canonical_object",
            Self::Frame => "frame",
            Self::FrameEnvelope => "frame_envelope",
            Self::HistoryBytes => "history_bytes",
            Self::FrameCount => "frame_count",
            Self::FailureReport => "failure_report",
        })
    }
}

/// Borrowed-cause projection of a measured size or an early serialization stop.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(untagged)]
pub enum SizeViolation {
    /// The complete representation was measured.
    Measured {
        /// Resource whose ceiling was exceeded.
        resource: SizeResource,
        /// Complete measured size.
        actual: u64,
        /// Inclusive byte or count ceiling.
        limit: u64,
    },
    /// Serialization stopped before the complete size was known.
    SerializationBound {
        /// Resource being serialized.
        resource: SizeResource,
        /// Lower bound observed when accumulation stopped.
        observed_at_least: u64,
        /// Inclusive byte ceiling.
        limit: u64,
    },
}
impl SizeViolation {
    /// Constructs a violation from an already measured owner limit.
    pub fn measured(resource: SizeResource, size: crate::SizeLimitExceeded) -> Self {
        Self::Measured {
            resource,
            actual: size.actual(),
            limit: size.limit(),
        }
    }
}
