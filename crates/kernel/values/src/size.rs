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
