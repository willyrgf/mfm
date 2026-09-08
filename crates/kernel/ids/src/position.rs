//! Checked declaration and execution identities, without scheduling semantics.

use serde::{Deserialize, Serialize};

use crate::{IdentityError, Result};

/// Index into the immutable ordered State declarations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct StatePosition(u16);

impl StatePosition {
    /// Checks that a declaration index is representable.
    pub fn new(index: usize) -> Result<Self> {
        u16::try_from(index)
            .map(Self)
            .map_err(|_| IdentityError::new("state position exceeds its bound"))
    }

    /// Returns the declaration index.
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

/// Monotonic run-wide execution identity. Effect prepare and conclusion share a visit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct VisitId(u64);

impl VisitId {
    /// Constructs a visit counter; history qualification checks its progression.
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the counter.
    pub const fn value(self) -> u64 {
        self.0
    }

    /// Selects the next execution without wrapping the run-wide counter.
    pub fn checked_next(self) -> Result<Self> {
        self.0
            .checked_add(1)
            .map(Self)
            .ok_or_else(|| IdentityError::new("visit counter exhausted"))
    }
}

/// Exact declaration occurrence in one run's execution history.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionPosition {
    /// Selected immutable declaration.
    pub state: StatePosition,
    /// Run-wide execution counter.
    pub visit: VisitId,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positions_reject_out_of_range_ingress_and_visits_never_wrap() {
        assert!(StatePosition::new(65_536).is_err());
        assert!(serde_json::from_str::<StatePosition>("65536").is_err());
        assert!(serde_json::from_str::<StatePosition>("-1").is_err());
        assert!(VisitId::new(u64::MAX).checked_next().is_err());
        let position: ExecutionPosition =
            serde_json::from_str(r#"{"state":65535,"visit":18446744073709551614}"#).unwrap();
        assert_eq!(position.state.index(), 65_535);
        assert_eq!(position.visit.checked_next().unwrap().value(), u64::MAX);
        assert!(
            serde_json::from_str::<ExecutionPosition>(r#"{"state":0,"visit":0,"next":1}"#).is_err()
        );
    }
}
