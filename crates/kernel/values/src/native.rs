use crate::{MfmValue, ValueError, MAX_RUN_OBJECT_CANONICAL_BYTES};
use serde::{Serialize, Serializer};
use serde_json::value::RawValue;
use std::any::Any;
use std::error::Error;
use std::fmt;
use std::sync::Arc;

trait Captured: Send + Sync {
    fn value(&self) -> &(dyn Any + Send + Sync);
    fn source(&self) -> Option<&(dyn Error + 'static)>;
    fn project(&self) -> Result<Box<RawValue>, NativeCause>;
}
struct Owner<E> {
    value: Arc<E>,
    source: Option<fn(&E) -> &(dyn Error + 'static)>,
    project: fn(&E) -> Result<Box<RawValue>, NativeCause>,
}
impl<E: Send + Sync + 'static> Captured for Owner<E> {
    fn value(&self) -> &(dyn Any + Send + Sync) {
        self.value.as_ref()
    }
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.source.map(|get| get(&self.value))
    }
    fn project(&self) -> Result<Box<RawValue>, NativeCause> {
        (self.project)(self.value.as_ref())
    }
}

/// Invocation-local custody of one concrete reviewed error or declared original.
///
/// The owner must provide a reviewed serializer. Arbitrary client errors, formatted source text,
/// and secret-bearing objects are not admitted by this contract. The carrier is never persisted.
#[derive(Clone)]
pub struct NativeCause(Arc<dyn Captured>);
impl NativeCause {
    /// Retains a reviewed owner error and its own source-preserving public projection.
    pub fn from_error<E: Error + Serialize + Send + Sync + 'static>(error: E) -> Self {
        Self::from_error_with(error, project_serializable::<E>)
    }
    /// Retains an owner whose reviewed projection can return a typed child failure.
    /// The callback must enforce the existing Values byte ceiling while constructing its result.
    pub fn from_error_with<E: Error + Send + Sync + 'static>(
        error: E,
        project: fn(&E) -> Result<Box<RawValue>, NativeCause>,
    ) -> Self {
        Self(Arc::new(Owner {
            value: Arc::new(error),
            source: Some(|value| value),
            project,
        }))
    }
    /// Retains a declared original without imposing an Error bound.
    /// Public projection must still pass Values admission, including its secret policy.
    pub fn from_original<E: MfmValue>(original: Arc<E>) -> Self {
        Self(Arc::new(Owner {
            value: original,
            source: None,
            project: project_original::<E>,
        }))
    }
    /// Borrows the exact native owner in library custody.
    pub fn downcast_ref<E: 'static>(&self) -> Option<&E> {
        self.0.value().downcast_ref()
    }
    /// Serializes only reviewed owner fields, stopping at the existing Values byte ceiling.
    /// The borrowed original remains in custody if projection fails.
    pub fn project(&self) -> Result<Box<RawValue>, NativeCause> {
        // An unwinding callback ends this attempt without surrendering the borrowed owner.
        // Callers must not retry a failed projector, which may have mutated interior state.
        let projected = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.0.project()))
            .map_err(|payload| {
                drop(payload);
                NativeCause::from_error(ProjectionPanicked)
            })??;
        crate::SizeLimitExceeded::check(
            projected.get().len() as u64,
            MAX_RUN_OBJECT_CANONICAL_BYTES as u64,
        )
        .map_err(NativeCause::from_error)?;
        Ok(projected)
    }
}

#[derive(Debug, thiserror::Error)]
#[error("native projection panicked; payload withheld")]
struct ProjectionPanicked;
impl Serialize for ProjectionPanicked {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        #[derive(Serialize)]
        struct Omission {
            field: &'static str,
            reason: &'static str,
        }
        let mut record = serializer.serialize_struct("ProjectionPanicked", 3)?;
        record.serialize_field("operation", "native_projection")?;
        record.serialize_field("outcome", "panicked")?;
        record.serialize_field(
            "omissions",
            &[Omission {
                field: "panic_payload",
                reason: "withheld",
            }],
        )?;
        record.end()
    }
}
fn project_original<E: MfmValue>(value: &E) -> Result<Box<RawValue>, NativeCause> {
    let (canonical, _) = crate::canonicalize_mfm_value(value).map_err(NativeCause::from_error)?;
    serde_json::from_slice(canonical.as_bytes())
        .map_err(|source| NativeCause::from_error(mfm_canonical::JsonError::new(source)))
}
fn project_serializable<E: Serialize>(value: &E) -> Result<Box<RawValue>, NativeCause> {
    let json = mfm_canonical::to_json_bounded(value, MAX_RUN_OBJECT_CANONICAL_BYTES)
        .map_err(NativeCause::from_error)?;
    RawValue::from_string(json)
        .map_err(|source| NativeCause::from_error(mfm_canonical::JsonError::new(source)))
}

impl fmt::Debug for NativeCause {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativeCause { owner: retained }")
    }
}
impl fmt::Display for NativeCause {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("reviewed native failure")
    }
}
impl Error for NativeCause {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.0.source()
    }
}
impl Serialize for ValueError {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        #[serde(rename_all = "snake_case")]
        enum Wire<'a> {
            Descriptor {
                message: &'static str,
                message_bytes: usize,
                upstream: &'static str,
            },
            Identity(&'a mfm_ids::IdentityError),
            CheckedIdentity(&'a mfm_ids::CheckedStringError),
            Canonical(&'a mfm_canonical::CanonicalError),
            InvalidSchemaIdentity,
            SchemaShapeMismatch,
            SizeLimit(&'a crate::SizeLimitExceeded),
            ArtifactTypeMismatch {
                field: &'static str,
                expected: &'a str,
                actual: &'a str,
            },
        }
        let wire = match self {
            Self::Descriptor(message) => Wire::Descriptor {
                message: "withheld",
                message_bytes: message.len(),
                upstream: "unavailable_at_existing_descriptor_boundary",
            },
            Self::Identity(source) => Wire::Identity(source),
            Self::CheckedIdentity(source) => Wire::CheckedIdentity(source),
            Self::Canonical(source) => Wire::Canonical(source),
            Self::InvalidSchemaIdentity => Wire::InvalidSchemaIdentity,
            Self::SchemaShapeMismatch => Wire::SchemaShapeMismatch,
            Self::SizeLimit(size) => Wire::SizeLimit(size),
            Self::ArtifactTypeMismatch {
                field,
                expected,
                actual,
            } => Wire::ArtifactTypeMismatch {
                field,
                expected,
                actual,
            },
        };
        wire.serialize(serializer)
    }
}
