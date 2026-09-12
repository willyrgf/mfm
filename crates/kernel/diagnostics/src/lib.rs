#![warn(missing_docs)]
//! Reviewed causal evidence vocabulary. Concrete IO owners capture sources without formatting them.

use mfm_program_derive::{MfmValue, PersistedSchema};
use serde::{Deserialize, Serialize};

/// Maximum complete canonical diagnostic size, including omission metadata.
pub const MAX_DIAGNOSTIC_BYTES: usize = 8192;

mod capture;
pub use capture::CaptureOmission;
mod vocabulary;
pub use vocabulary::*;

/// Checked evidence construction failure; never contains rejected input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum EvidenceError {
    /// A numeric or string value is outside its declared range.
    #[error("invalid diagnostic value")]
    InvalidValue,
    /// A source contains incompatible or duplicate facts.
    #[error("invalid source facts")]
    InvalidFacts,
    /// An omission names an absent response or source.
    #[error("invalid omission location")]
    InvalidLocation,
    /// A count or byte budget was exceeded.
    #[error("diagnostic bound exceeded")]
    BoundExceeded,
}
impl serde::Serialize for EvidenceError {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut value = serializer.serialize_struct("EvidenceError", 2)?;
        let kind = match self {
            Self::InvalidValue => "invalid_value",
            Self::InvalidFacts => "invalid_facts",
            Self::InvalidLocation => "invalid_location",
            Self::BoundExceeded => "bound_exceeded",
        };
        value.serialize_field("kind", kind)?;
        value.serialize_field("upstream_detail", "unavailable_at_existing_owner_boundary")?;
        value.end()
    }
}

/// HTTP response status received from the server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, PersistedSchema)]
#[serde(transparent)]
#[mfm(
    namespace = "mfm.diagnostics",
    name = "http-status",
    version = "1",
    schema = "mfm.diagnostics.http-status"
)]
pub struct HttpStatusCode(u16);

impl HttpStatusCode {
    /// Checks the three-digit HTTP status range.
    pub fn new(code: u16) -> Result<Self, EvidenceError> {
        if (100..=999).contains(&code) {
            Ok(Self(code))
        } else {
            Err(EvidenceError::InvalidValue)
        }
    }
    /// Returns the received code.
    pub fn get(self) -> u16 {
        self.0
    }
}
impl<'de> Deserialize<'de> for HttpStatusCode {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::new(u16::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

/// Five ASCII alphanumeric SQLSTATE bytes, excluding free-form server details.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, PersistedSchema)]
#[serde(transparent)]
#[mfm(
    namespace = "mfm.diagnostics",
    name = "sql-state",
    version = "1",
    schema = "mfm.diagnostics.sql-state"
)]
pub struct SqlState(#[mfm(minimum_bytes = 5, maximum_bytes = 5)] String);
impl SqlState {
    /// Checks the exact SQLSTATE grammar without retaining rejected text.
    pub fn new(code: &str) -> Result<Self, EvidenceError> {
        if code.len() == 5 && code.bytes().all(|byte| byte.is_ascii_alphanumeric()) {
            Ok(Self(code.to_owned()))
        } else {
            Err(EvidenceError::InvalidValue)
        }
    }
    /// Returns the checked code.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl<'de> Deserialize<'de> for SqlState {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::new(&String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

/// Received response observations, separate from source ancestry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, PersistedSchema)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.diagnostics",
    name = "response-context",
    version = "1",
    schema = "mfm.diagnostics.response-context"
)]
pub struct ResponseContext {
    status: HttpStatusCode,
    rpc_code: Option<i64>,
}
impl ResponseContext {
    /// Received HTTP status.
    pub fn status(&self) -> HttpStatusCode {
        self.status
    }
    /// Code from a checked JSON-RPC error envelope.
    pub fn rpc_code(&self) -> Option<i64> {
        self.rpc_code
    }
}

/// One concrete source and its reviewed facts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, PersistedSchema)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.diagnostics",
    name = "source-layer",
    version = "1",
    schema = "mfm.diagnostics.source-layer"
)]
pub struct SourceLayer {
    kind: SourceKind,
    #[mfm(persisted, maximum_items = 8)]
    facts: Vec<SourceFact>,
    facts_truncated: bool,
}
impl SourceLayer {
    /// Source category.
    pub fn kind(&self) -> SourceKind {
        self.kind
    }
    /// Facts in schema-defined field order.
    pub fn facts(&self) -> &[SourceFact] {
        &self.facts
    }
    /// Additional facts exceeded the capture bound.
    pub fn facts_truncated(&self) -> bool {
        self.facts_truncated
    }
}
impl<'de> Deserialize<'de> for SourceLayer {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            kind: SourceKind,
            facts: Vec<SourceFact>,
            facts_truncated: bool,
        }
        let wire = Wire::deserialize(deserializer)?;
        if wire
            .facts
            .windows(2)
            .any(|pair| pair[0].field_order() >= pair[1].field_order())
        {
            return Err(serde::de::Error::custom(EvidenceError::InvalidFacts));
        }
        Self::new(wire.kind, wire.facts, wire.facts_truncated).map_err(serde::de::Error::custom)
    }
}

/// Outermost-first prefix of the exposed causal chain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, PersistedSchema)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.diagnostics",
    name = "source-chain",
    version = "1",
    schema = "mfm.diagnostics.source-chain"
)]
pub struct SourceChain {
    #[mfm(persisted, maximum_items = 32)]
    layers: Vec<SourceLayer>,
    end: ChainEnd,
}
impl SourceChain {
    /// Retained sources.
    pub fn layers(&self) -> &[SourceLayer] {
        &self.layers
    }
    /// Why the retained prefix ends.
    pub fn end(&self) -> ChainEnd {
        self.end
    }
}
impl<'de> Deserialize<'de> for SourceChain {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            layers: Vec<SourceLayer>,
            end: ChainEnd,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(wire.layers, wire.end).map_err(serde::de::Error::custom)
    }
}

/// Explicit accounting for one unretained field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, PersistedSchema)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.diagnostics",
    name = "omission",
    version = "1",
    schema = "mfm.diagnostics.omission"
)]
pub struct Omission {
    at: EvidenceLocation,
    field: OmittedField,
    reason: OmissionReason,
    observed_bytes: Option<u64>,
}
impl Omission {
    /// Owner of the field.
    pub fn at(&self) -> &EvidenceLocation {
        &self.at
    }
    /// Excluded field.
    pub fn field(&self) -> OmittedField {
        self.field
    }
    /// Reason for exclusion.
    pub fn reason(&self) -> OmissionReason {
        self.reason
    }
    /// Known size, without estimating unavailable data.
    pub fn observed_bytes(&self) -> Option<u64> {
        self.observed_bytes
    }
}

/// Bounded reviewed evidence; contains no arbitrary client text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue, PersistedSchema)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.diagnostics",
    name = "diagnostic-evidence",
    version = "1",
    schema = "mfm.diagnostics.diagnostic-evidence"
)]
pub struct DiagnosticEvidence {
    #[mfm(persisted)]
    response: Option<ResponseContext>,
    #[mfm(persisted)]
    sources: SourceChain,
    #[mfm(persisted, maximum_items = 32)]
    omissions: Vec<Omission>,
    omissions_truncated: bool,
}
impl DiagnosticEvidence {
    /// Received response context.
    pub fn response(&self) -> &Option<ResponseContext> {
        &self.response
    }
    /// Retained source ancestry.
    pub fn sources(&self) -> &SourceChain {
        &self.sources
    }
    /// Explicit missing field accounting.
    pub fn omissions(&self) -> &[Omission] {
        &self.omissions
    }
    /// Additional omission entries exceeded the bound.
    pub fn omissions_truncated(&self) -> bool {
        self.omissions_truncated
    }
}
impl<'de> Deserialize<'de> for DiagnosticEvidence {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            response: Option<ResponseContext>,
            sources: SourceChain,
            omissions: Vec<Omission>,
            omissions_truncated: bool,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(
            wire.response,
            wire.sources,
            wire.omissions,
            wire.omissions_truncated,
        )
        .map_err(serde::de::Error::custom)
    }
}
impl ResponseContext {
    /// Associates received status with an optional checked RPC envelope code.
    pub fn new(status: HttpStatusCode, rpc_code: Option<i64>) -> Self {
        Self { status, rpc_code }
    }
}
impl SourceFact {
    fn field_order(&self) -> u8 {
        match self {
            Self::Transport { .. } => 0,
            Self::Database { .. } => 1,
            Self::SqlState { .. } => 2,
            Self::Os { .. } => 3,
            Self::OsCode { .. } => 4,
            Self::Parse { .. } => 5,
            Self::Size { .. } => 6,
            Self::Task { .. } => 7,
            Self::Channel { .. } => 8,
        }
    }
    fn admits(&self, kind: SourceKind) -> bool {
        match self {
            Self::Transport { .. } => kind == SourceKind::Transport,
            Self::Database { .. } | Self::SqlState { .. } => kind == SourceKind::Database,
            Self::Os { .. } | Self::OsCode { .. } => kind == SourceKind::Os,
            Self::Parse { .. } => kind == SourceKind::Parse,
            Self::Size { .. } => matches!(
                kind,
                SourceKind::Transport | SourceKind::Database | SourceKind::Os | SourceKind::Parse
            ),
            Self::Task { .. } => kind == SourceKind::Task,
            Self::Channel { .. } => kind == SourceKind::Channel,
        }
    }
}
impl SourceLayer {
    /// Checks category, uniqueness and the eight-fact limit, then orders facts canonically.
    pub fn new(
        kind: SourceKind,
        mut facts: Vec<SourceFact>,
        facts_truncated: bool,
    ) -> Result<Self, EvidenceError> {
        if facts.len() > 8 {
            return Err(EvidenceError::BoundExceeded);
        }
        facts.sort_by_key(SourceFact::field_order);
        if facts.iter().any(|fact| !fact.admits(kind))
            || facts
                .windows(2)
                .any(|pair| pair[0].field_order() == pair[1].field_order())
        {
            return Err(EvidenceError::InvalidFacts);
        }
        Ok(Self {
            kind,
            facts,
            facts_truncated,
        })
    }
}
impl SourceChain {
    /// Checks the 32-source bound without walking any discarded suffix.
    pub fn new(layers: Vec<SourceLayer>, end: ChainEnd) -> Result<Self, EvidenceError> {
        if layers.len() > 32 {
            return Err(EvidenceError::BoundExceeded);
        }
        Ok(Self { layers, end })
    }
}
impl Omission {
    /// Records a missing field; the enclosing evidence checks location presence.
    pub fn new(
        at: EvidenceLocation,
        field: OmittedField,
        reason: OmissionReason,
        observed_bytes: Option<u64>,
    ) -> Self {
        Self {
            at,
            field,
            reason,
            observed_bytes,
        }
    }
}
impl DiagnosticEvidence {
    /// Evidence for a local checked failure with no upstream source or response.
    pub fn local() -> Self {
        Self {
            response: None,
            sources: SourceChain {
                layers: Vec::new(),
                end: ChainEnd::Complete,
            },
            omissions: Vec::new(),
            omissions_truncated: false,
        }
    }

    /// Checks omission ownership, count bounds and the complete 8 KiB canonical budget.
    pub fn new(
        response: Option<ResponseContext>,
        sources: SourceChain,
        omissions: Vec<Omission>,
        omissions_truncated: bool,
    ) -> Result<Self, EvidenceError> {
        if omissions.len() > 32 {
            return Err(EvidenceError::BoundExceeded);
        }
        for omission in &omissions {
            let present = match omission.at {
                EvidenceLocation::Response => response.is_some(),
                EvidenceLocation::SourceLayer { index } => {
                    usize::from(index) < sources.layers.len()
                }
            };
            if !present {
                return Err(EvidenceError::InvalidLocation);
            }
        }
        let evidence = Self {
            response,
            sources,
            omissions,
            omissions_truncated,
        };
        // All strings are closed ASCII vocabulary or checked ASCII SQLSTATE. Ordering JSON
        // object keys changes no byte count; compact Serde and canonical JSON lengths agree.
        let bytes = serde_json::to_vec(&evidence).map_err(|_| EvidenceError::InvalidValue)?;
        if bytes.len() > MAX_DIAGNOSTIC_BYTES {
            return Err(EvidenceError::BoundExceeded);
        }
        Ok(evidence)
    }
}

#[cfg(test)]
mod tests;
