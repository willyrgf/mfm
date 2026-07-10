//! Shared holding-observation status vocabulary and write-admission rules.

use serde::{Deserialize, Serialize};

macro_rules! holding_status_tag {
    ($type:ty, $error_label:literal, {
        $($variant:path => $tag:literal),+ $(,)?
    }) => {
        impl $type {
            /// Returns the canonical snake-case status tag.
            pub const fn as_str(self) -> &'static str {
                match self {
                    $($variant => $tag),+
                }
            }
        }

        impl std::fmt::Display for $type {
            fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str(self.as_str())
            }
        }

        impl std::str::FromStr for $type {
            type Err = &'static str;

            fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
                match value {
                    $($tag => Ok($variant),)+
                    _ => Err($error_label),
                }
            }
        }
    };
}

/// Coverage claim carried on holding observations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoverageStatus {
    /// Observation is complete for the subject at the recorded anchor.
    CompleteAtAnchor,
    /// Observation covers only the configured asset set at the recorded anchor.
    ConfiguredOnly,
    /// Observation was truncated relative to the intended subject set.
    Truncated,
    /// Observation is incomplete.
    Incomplete,
}

holding_status_tag!(CoverageStatus, "unknown coverage status", {
    CoverageStatus::CompleteAtAnchor => "complete_at_anchor",
    CoverageStatus::ConfiguredOnly => "configured_only",
    CoverageStatus::Truncated => "truncated",
    CoverageStatus::Incomplete => "incomplete",
});

impl CoverageStatus {
    /// Returns whether a writer may admit a Platform holding fact with this coverage.
    pub const fn is_admissible_for_write(self) -> bool {
        matches!(self, Self::CompleteAtAnchor | Self::ConfiguredOnly)
    }
}

/// Source status for holding observations, not Bitcoin IBD or sync status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HoldingSourceStatus {
    /// Source produced a usable observation at the recorded anchor.
    Ok,
    /// Source cannot satisfy the requested observation shape.
    Unsupported,
    /// Source failed while producing the observation.
    Failed,
}

holding_status_tag!(HoldingSourceStatus, "unknown holding source status", {
    HoldingSourceStatus::Ok => "ok",
    HoldingSourceStatus::Unsupported => "unsupported",
    HoldingSourceStatus::Failed => "failed",
});

impl HoldingSourceStatus {
    /// Returns whether a writer may admit a Platform holding fact with this status.
    pub const fn is_admissible_for_write(self) -> bool {
        matches!(self, Self::Ok)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn holding_status_tags_are_closed_and_write_admission_is_explicit() {
        assert_eq!(
            CoverageStatus::CompleteAtAnchor.as_str(),
            "complete_at_anchor"
        );
        assert_eq!(
            "configured_only".parse::<CoverageStatus>(),
            Ok(CoverageStatus::ConfiguredOnly)
        );
        assert!(CoverageStatus::ConfiguredOnly.is_admissible_for_write());
        assert!(!CoverageStatus::Truncated.is_admissible_for_write());
        assert_eq!(HoldingSourceStatus::Ok.as_str(), "ok");
        assert_eq!(
            "ok".parse::<HoldingSourceStatus>(),
            Ok(HoldingSourceStatus::Ok)
        );
        assert!(!HoldingSourceStatus::Failed.is_admissible_for_write());
    }
}
