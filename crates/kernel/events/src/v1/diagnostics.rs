use super::*;

/// Redaction-safe error information.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MfmErrorInfo {
    /// Stable error code.
    pub code: ErrorCode,
    /// Error category.
    pub category: ErrorCategory,
    /// Whether retry is allowed.
    pub retryable: bool,
    /// Public safe message.
    pub safe_message: String,
    /// Optional redacted public details digest.
    pub public_details: Option<RedactedJson>,
    /// Optional redacted diagnostic artifact reference.
    pub diagnostic_ref: Option<ArtifactEvidenceRef>,
}

impl MfmErrorInfo {
    /// Creates redaction-safe error information without optional details or diagnostics.
    pub fn new(
        code: ErrorCode,
        category: ErrorCategory,
        retryable: bool,
        safe_message: impl Into<String>,
    ) -> Result<Self> {
        let error = Self {
            code,
            category,
            retryable,
            safe_message: safe_message.into(),
            public_details: None,
            diagnostic_ref: None,
        };
        error.validate()?;
        Ok(error)
    }

    /// Adds a redacted public details digest after validating the full public diagnostic.
    pub fn with_public_details(mut self, public_details: RedactedJson) -> Result<Self> {
        self.public_details = Some(public_details);
        self.validate()?;
        Ok(self)
    }

    /// Adds a redacted diagnostic artifact reference after validating the full diagnostic.
    pub fn with_diagnostic_ref(mut self, diagnostic_ref: ArtifactEvidenceRef) -> Result<Self> {
        self.diagnostic_ref = Some(diagnostic_ref);
        self.validate()?;
        Ok(self)
    }

    /// Validates that persisted public diagnostics are redaction-safe.
    pub fn validate(&self) -> Result<()> {
        validate_public_error_text("safe_message", &self.safe_message)?;
        if let Some(details) = &self.public_details {
            details.validate()?;
        }
        if let Some(diagnostic_ref) = &self.diagnostic_ref {
            validate_public_diagnostic_ref(diagnostic_ref)?;
        }
        Ok(())
    }
}

/// Error category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ErrorCategory {
    /// Planning or certification error.
    Planning,
    /// Input, config, or value validation error.
    Validation,
    /// Capability or adapter error.
    Capability,
    /// External side-effect error.
    SideEffect,
    /// Runtime infrastructure error.
    Runtime,
    /// Storage or replay error.
    Storage,
    /// Cancellation.
    Cancelled,
}

/// Redacted JSON public details reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RedactedJson {
    /// Canonical digest of the redacted public details.
    pub content_digest: ContentDigest,
}

impl RedactedJson {
    /// Creates a redacted public details digest reference.
    pub fn new(content_digest: ContentDigest) -> Self {
        Self { content_digest }
    }

    /// Validates the public-details reference.
    pub fn validate(&self) -> Result<()> {
        let _ = &self.content_digest;
        Ok(())
    }
}

const SECRET_SHAPED_DIAGNOSTIC_MARKERS: &[&str] = &[
    "private key",
    "private_key",
    "mnemonic",
    "password",
    "passphrase",
    "seed phrase",
    "seed_phrase",
    "api key",
    "api_key",
    "authorization:",
    "bearer ",
    "raw transaction",
    "raw_transaction",
    "raw_tx",
    "signed payload",
    "signed_payload",
    "keystore path",
    "keystore_path",
    "rpc url",
    "rpc_url",
    "secret=",
    "token=",
    "-----begin",
];

fn validate_public_error_text(field: &'static str, value: &str) -> Result<()> {
    CheckedPrintableAscii512::new(value).map_err(|error| EventError::InvalidPublicDiagnostic {
        field,
        reason: public_error_text_reason(error.reason()),
    })?;
    if contains_secret_shaped_diagnostic(value) {
        return Err(EventError::InvalidPublicDiagnostic {
            field,
            reason: "message resembles secret material",
        });
    }
    Ok(())
}

fn public_error_text_reason(reason: &mfm_ids::CheckedStringErrorReason) -> &'static str {
    match reason {
        mfm_ids::CheckedStringErrorReason::Empty => "message must not be empty",
        mfm_ids::CheckedStringErrorReason::TooLong { .. } => "message exceeds public length limit",
        mfm_ids::CheckedStringErrorReason::SegmentTooLong { .. }
        | mfm_ids::CheckedStringErrorReason::InvalidCharacter { .. }
        | mfm_ids::CheckedStringErrorReason::InvalidStart
        | mfm_ids::CheckedStringErrorReason::InvalidEnd
        | mfm_ids::CheckedStringErrorReason::MissingSeparator { .. }
        | mfm_ids::CheckedStringErrorReason::EmptySegment
        | mfm_ids::CheckedStringErrorReason::ReservedPrefix { .. } => {
            "message must be printable ASCII"
        }
    }
}

fn contains_secret_shaped_diagnostic(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    SECRET_SHAPED_DIAGNOSTIC_MARKERS
        .iter()
        .any(|marker| lower.contains(marker))
        || lower
            .split(is_public_diagnostic_token_boundary)
            .any(|token| is_key_shaped_token(token) || is_hex_secret_shaped_token(token))
}

fn is_public_diagnostic_token_boundary(ch: char) -> bool {
    !(ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
}

fn is_key_shaped_token(token: &str) -> bool {
    token.starts_with("sk_")
        || token.starts_with("akia")
        || token.starts_with("ghp_")
        || token.starts_with("github_pat_")
        || token.starts_with("xoxb-")
}

fn is_hex_secret_shaped_token(token: &str) -> bool {
    let token = token.strip_prefix("0x").unwrap_or(token);
    token.len() >= 64 && token.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn validate_public_diagnostic_ref(reference: &ArtifactEvidenceRef) -> Result<()> {
    if reference.role != ArtifactRole::RedactedDiagnostic {
        return Err(EventError::InvalidPublicDiagnostic {
            field: "diagnostic_ref",
            reason: "diagnostic artifact role must be redacted_diagnostic",
        });
    }
    if reference.semantic_type_id.is_some() {
        return Err(EventError::InvalidPublicDiagnostic {
            field: "diagnostic_ref",
            reason: "diagnostic artifact semantic type must be absent",
        });
    }
    Ok(())
}
