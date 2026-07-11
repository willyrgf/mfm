#![warn(missing_docs)]
//! Typed kernel event contracts for MFM.
//!
//! This crate owns the closed v1 event payload set used by typed run storage,
//! replay, resume, public output evidence, and retention projection.

use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_facts as facts;
use mfm_ids::{
    AdapterKind, AdapterVersion, ArtifactId, AttemptId, CapabilityKind, CapabilityVersion, CellId,
    ContentDigest, DescriptorId, DigestAlgorithm, EventId, IdentityError, LoweringVersion, NodeId,
    PrintableAscii1024 as CheckedPrintableAscii1024, PrintableAscii512 as CheckedPrintableAscii512,
    RunId, SchemaId, ScopeId, SeedId, SemanticTypeId, SideEffectPairId, SpecHash, SpecVersion,
    StateKind, StateVersion, StoreScopeId, VisibleAscii256 as CheckedVisibleAscii256,
};
use mfm_spec::v1::{
    CanonicalizerIdentity, CellContextSpec, CellProducer, DescriptorIdentity, MediaType,
    PublicFieldPath, ResourceNamespace, ValueLineageRef,
};

/// Result type for typed kernel event helpers.
pub type Result<T> = std::result::Result<T, EventError>;

/// Error returned by typed kernel event helpers.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EventError {
    /// A checked event string field failed validation.
    #[error("invalid {field} string {value:?}")]
    InvalidString {
        /// Field label.
        field: &'static str,
        /// Invalid value.
        value: String,
    },
    /// A public diagnostic field failed redaction-safety validation.
    #[error("invalid public diagnostic {field}: {reason}")]
    InvalidPublicDiagnostic {
        /// Field label.
        field: &'static str,
        /// Stable rejection reason.
        reason: &'static str,
    },
    /// An invocation key failed validation.
    #[error("invalid invocation key: {reason}")]
    InvalidInvocationKey {
        /// Stable rejection reason.
        reason: &'static str,
    },
    /// Identity construction failed.
    #[error("identity error: {0}")]
    Identity(String),
    /// JSON serialization failed before canonicalization.
    #[error("event JSON serialization error: {0}")]
    Serialize(String),
    /// Canonical JSON construction failed.
    #[error("event canonicalization error: {0}")]
    Canonical(String),
}

impl From<IdentityError> for EventError {
    fn from(error: IdentityError) -> Self {
        Self::Identity(error.to_string())
    }
}

fn canonical_json(value: serde_json::Value) -> Result<PlainCanonicalJsonBytes> {
    let json =
        serde_json::to_string(&value).map_err(|error| EventError::Serialize(error.to_string()))?;
    PlainCanonicalJsonBytes::from_json_str(&json)
        .map_err(|error| EventError::Canonical(error.to_string()))
}

macro_rules! checked_string_type {
    ($(#[$doc:meta])* $name:ident, $field:literal) => {
        $(#[$doc])*
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(CheckedVisibleAscii256);

        impl $name {
            #[doc = concat!("Creates a checked `", stringify!($name), "`.")]
            pub fn new(value: impl AsRef<str>) -> Result<Self> {
                let value = value.as_ref();
                CheckedVisibleAscii256::new(value)
                    .map(Self)
                    .map_err(|_| EventError::InvalidString {
                        field: $field,
                        value: value.to_owned(),
                    })
            }

            #[doc = concat!("Returns the persisted `", stringify!($name), "` string.")]
            pub fn as_str(&self) -> &str {
                self.0.as_str()
            }
        }

        impl ::std::fmt::Display for $name {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                f.write_str(self.as_str())
            }
        }
    };
}

macro_rules! checked_text_type {
    ($(#[$doc:meta])* $name:ident, $field:literal) => {
        $(#[$doc])*
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(CheckedPrintableAscii1024);

        impl $name {
            #[doc = concat!("Creates a checked `", stringify!($name), "`.")]
            pub fn new(value: impl AsRef<str>) -> Result<Self> {
                let value = value.as_ref();
                CheckedPrintableAscii1024::new(value)
                    .map(Self)
                    .map_err(|_| EventError::InvalidString {
                        field: $field,
                        value: value.to_owned(),
                    })
            }

            #[doc = concat!("Returns the persisted `", stringify!($name), "` string.")]
            pub fn as_str(&self) -> &str {
                self.0.as_str()
            }
        }

        impl ::std::fmt::Display for $name {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                f.write_str(self.as_str())
            }
        }
    };
}

/// Versioned v1 typed kernel event contracts.
pub mod v1;
