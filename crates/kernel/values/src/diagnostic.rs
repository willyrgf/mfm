//! Immutable diagnostic data; concrete execution errors retain classification ownership.

use crate::{
    CanonicalJsonProfile, PersistedSchema, SchemaIdentity, SchemaKind, SchemaShape, SizeResource,
    SizeViolation, ValueError,
};
use serde::{Deserialize, Serialize};

/// Selected diagnostic fields, admitted only as part of their concrete persisted owner.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DiagnosticEvidence(serde_json::Value);
impl DiagnosticEvidence {
    /// Takes ownership of diagnostic data without claiming persistence admission.
    pub fn from_value(value: serde_json::Value) -> Self {
        Self(value)
    }
    /// Borrows the selected data without copying or recapturing it.
    pub fn as_value(&self) -> &serde_json::Value {
        &self.0
    }
}
impl PersistedSchema for DiagnosticEvidence {
    fn schema_identity() -> crate::Result<SchemaIdentity> {
        SchemaIdentity::new(
            SchemaKind::PersistedContract,
            None,
            "mfm.diagnostics.diagnostic-evidence",
            mfm_ids::SchemaVersion::new("2").map_err(ValueError::Identity)?,
            SchemaShape::CanonicalJsonTerminal {
                profile: CanonicalJsonProfile::DiagnosticFloatFree,
            },
        )
    }
    fn validate(&self) -> crate::Result<()> {
        crate::validate_derived_persisted_owner(self, &Self::schema_identity()?)
    }
}

/// Final invocation-only diagnostic data, without native custody or classification authority.
#[derive(Debug, Serialize)]
pub struct InvocationDiagnostic {
    code: &'static str,
    operation: &'static str,
    details: DiagnosticEvidence,
    size: Option<SizeViolation>,
}
impl InvocationDiagnostic {
    /// Captures selected serializable fields once, preserving primary facts if conversion fails.
    pub fn from_fields<T: Serialize + ?Sized>(
        code: &'static str,
        operation: &'static str,
        fields: &T,
        size: Option<SizeViolation>,
    ) -> Self {
        use std::panic::{catch_unwind, AssertUnwindSafe};
        let details = match catch_unwind(AssertUnwindSafe(|| serde_json::to_value(fields))) {
            Ok(Ok(value)) => value,
            Ok(Err(_)) => serde_json::json!({"omitted": {"reason": "encoding_failed"}}),
            Err(_) => serde_json::json!({"omitted": {"reason": "panicked"}}),
        };
        Self {
            code,
            operation,
            details: DiagnosticEvidence::from_value(details),
            size,
        }
    }
    /// Returns the originating owner's fixed diagnostic code.
    pub fn code(&self) -> &'static str {
        self.code
    }
    /// Returns the boundary operation that supplied these facts.
    pub fn operation(&self) -> &'static str {
        self.operation
    }
    /// Borrows the captured detail tree.
    pub fn details(&self) -> &DiagnosticEvidence {
        &self.details
    }
    /// Returns size evidence supplied by the concrete owner.
    pub fn size(&self) -> Option<SizeViolation> {
        self.size
    }
}
impl ValueError {
    /// Adapts concrete Values fields and direct size evidence at an invocation boundary.
    pub fn into_diagnostic(self, operation: &'static str) -> InvocationDiagnostic {
        let size = match &self {
            Self::SizeLimit(size) => Some(SizeViolation::Measured {
                resource: SizeResource::CanonicalObject,
                actual: size.actual(),
                limit: size.limit(),
            }),
            Self::Canonical(source) => source.serialization_bound().map(|(limit, observed)| {
                SizeViolation::SerializationBound {
                    resource: SizeResource::CanonicalObject,
                    limit: limit as u64,
                    observed_at_least: observed as u64,
                }
            }),
            _ => None,
        };
        InvocationDiagnostic::from_fields("value_error", operation, &self, size)
    }
}
