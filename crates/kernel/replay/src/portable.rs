//! Public transport constants for structured portable history exports.

/// Exact media type of one structured portable run export document.
pub const PORTABLE_RUN_EXPORT_MEDIA_TYPE: &str =
    "application/vnd.mfm.structured-run-export.v1+json";

/// Closed portable-export scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ExportKind {
    /// Export through the current semantic head.
    Semantic,
    /// Export through the exact current physical journal head.
    Audit,
}

impl ExportKind {
    /// Returns the frozen canonical spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Semantic => "semantic",
            Self::Audit => "audit",
        }
    }
}
