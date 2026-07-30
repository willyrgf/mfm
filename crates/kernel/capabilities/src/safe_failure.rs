use mfm_ids::{ContentDigest, ContentRef, DigestAlgorithm, FieldPath, SchemaId, StableId};
use mfm_values::{SchemaIdentity, SchemaKind};
use serde::{Deserialize, Serialize};

use mfm_canonical::{sha256_digest_bytes, PlainCanonicalJsonBytes};

/// Exact version of the domain-free safe-failure classifier descriptor.
pub const SAFE_FAILURE_CLASSIFIER_DESCRIPTOR_VERSION: &str = "mfm.safe-failure-classifier.v1";

/// Maximum canonical byte length of one retained typed safe diagnostic.
pub const MAX_SAFE_FAILURE_DIAGNOSTIC_BYTES: usize = 16_384;

const MAX_SAFE_FAILURE_CLASSIFIER_BYTES: usize = 65_536;
const MAX_SAFE_FAILURE_CLASSIFIER_RULES: usize = 256;

/// Closed coarse classification shared by reviewed capability failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureClass {
    /// Authorization or routing authority was unavailable or rejected.
    Authorization,
    /// Certified configuration was unavailable or invalid.
    Configuration,
    /// The reviewed request was invalid.
    Request,
    /// Cancellation occurred at the capability boundary.
    Cancellation,
    /// Transport entry or observation failed.
    Transport,
    /// The destination returned a reviewed semantic rejection.
    Destination,
    /// A response could not be represented safely.
    UnrepresentableResponse,
    /// Integrity validation failed.
    Integrity,
    /// A typed external resource conflicted.
    ResourceConflict,
    /// No more specific safe class can be retained.
    Unclassified,
}

impl FailureClass {
    /// Returns the frozen recoverability-v3 spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Authorization => "authorization",
            Self::Configuration => "configuration",
            Self::Request => "request",
            Self::Cancellation => "cancellation",
            Self::Transport => "transport",
            Self::Destination => "destination",
            Self::UnrepresentableResponse => "unrepresentable_response",
            Self::Integrity => "integrity",
            Self::ResourceConflict => "resource_conflict",
            Self::Unclassified => "unclassified",
        }
    }
}

/// Reviewed stage at which a capability failure occurred.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BoundaryStage {
    /// Failure occurred before entering the independently meaningful operation.
    BeforeBoundaryEntry,
    /// Failure occurred while entering the independently meaningful operation.
    BoundaryEntry,
    /// Failure occurred while observing an operation whose identity was known.
    BoundaryObservation,
}

impl BoundaryStage {
    /// Returns the frozen recoverability-v3 spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::BeforeBoundaryEntry => "before_boundary_entry",
            Self::BoundaryEntry => "boundary_entry",
            Self::BoundaryObservation => "boundary_observation",
        }
    }
}

/// Reviewed coarse byte-size class for safe diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoarseSizeClass {
    /// No bytes were present.
    Zero,
    /// At most 16 KiB were present.
    UpTo16Kib,
    /// More than 16 KiB and at most 1 MiB were present.
    UpTo1Mib,
    /// More than 1 MiB were present.
    Over1Mib,
}

/// Closed journal observation outcome selected by one classifier rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SafeFailureOutcome {
    /// Boundary entry was proven not to have occurred.
    DidNotEnter,
    /// Boundary entry or the terminal outcome remains indeterminate.
    Indeterminate,
}

impl SafeFailureOutcome {
    /// Returns the frozen journal spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::DidNotEnter => "did_not_enter",
            Self::Indeterminate => "indeterminate",
        }
    }
}

/// Closed rule for the optional provider-envelope size classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SafeFailureSizeRule {
    /// The classifier forbids a retained coarse size.
    None,
    /// The typed failure supplies one reviewed coarse size.
    FromFailure,
}

impl SafeFailureSizeRule {
    const fn accepts(self, value: Option<CoarseSizeClass>) -> bool {
        match self {
            Self::None => value.is_none(),
            Self::FromFailure => value.is_some(),
        }
    }
}

/// One exact string-valued discriminator constraint within a safe diagnostic.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SafeFailureDiagnosticConstraint {
    field_path: FieldPath,
    allowed_values: Vec<String>,
}

impl SafeFailureDiagnosticConstraint {
    /// Constructs one constraint with a nonempty sorted set of allowed string values.
    pub fn new(
        field_path: FieldPath,
        mut allowed_values: Vec<String>,
    ) -> Result<Self, SafeFailureClassifierError> {
        allowed_values.sort();
        let constraint = Self {
            field_path,
            allowed_values,
        };
        constraint.validate_invariant()?;
        Ok(constraint)
    }

    /// Returns the constrained canonical JSON field path.
    pub const fn field_path(&self) -> &FieldPath {
        &self.field_path
    }

    /// Returns the accepted string values in canonical order.
    pub fn allowed_values(&self) -> &[String] {
        &self.allowed_values
    }

    fn validate_invariant(&self) -> Result<(), SafeFailureClassifierError> {
        if self.allowed_values.is_empty()
            || self
                .allowed_values
                .iter()
                .any(|value| value.is_empty() || value.len() > 256)
            || !self.allowed_values.windows(2).all(|pair| pair[0] < pair[1])
        {
            return Err(SafeFailureClassifierError::InvalidDiagnosticRule);
        }
        Ok(())
    }

    fn accepts(&self, diagnostic: &serde_json::Value) -> bool {
        let mut current = diagnostic;
        for segment in self.field_path.as_str().split('.') {
            let serde_json::Value::Object(fields) = current else {
                return false;
            };
            let Some(next) = fields.get(segment) else {
                return false;
            };
            current = next;
        }
        let serde_json::Value::String(value) = current else {
            return false;
        };
        self.allowed_values
            .binary_search_by(|candidate| candidate.as_str().cmp(value.as_str()))
            .is_ok()
    }
}

/// Closed rule for typed diagnostic presence and canonical discriminators.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SafeFailureDiagnosticRule {
    /// The failure must not retain a typed diagnostic.
    Forbidden,
    /// The failure must retain one bounded typed diagnostic satisfying every constraint.
    Required {
        /// Canonical string-field constraints in strict field-path order.
        constraints: Vec<SafeFailureDiagnosticConstraint>,
    },
}

impl SafeFailureDiagnosticRule {
    /// Constructs one required-diagnostic rule with canonical constraints.
    pub fn required(
        mut constraints: Vec<SafeFailureDiagnosticConstraint>,
    ) -> Result<Self, SafeFailureClassifierError> {
        constraints.sort_by(|left, right| left.field_path.cmp(&right.field_path));
        let rule = Self::Required { constraints };
        rule.validate_invariant()?;
        Ok(rule)
    }

    /// Returns whether this rule requires a retained diagnostic.
    pub const fn requires_diagnostic(&self) -> bool {
        matches!(self, Self::Required { .. })
    }

    /// Returns the required discriminator constraints, if any.
    pub fn constraints(&self) -> &[SafeFailureDiagnosticConstraint] {
        match self {
            Self::Forbidden => &[],
            Self::Required { constraints } => constraints,
        }
    }

    const fn accepts_presence(&self, has_diagnostic: bool) -> bool {
        self.requires_diagnostic() == has_diagnostic
    }

    fn validate_invariant(&self) -> Result<(), SafeFailureClassifierError> {
        let Self::Required { constraints } = self else {
            return Ok(());
        };
        if constraints.is_empty()
            || !constraints
                .windows(2)
                .all(|pair| pair[0].field_path < pair[1].field_path)
        {
            return Err(SafeFailureClassifierError::InvalidDiagnosticRule);
        }
        for constraint in constraints {
            constraint.validate_invariant()?;
        }
        Ok(())
    }

    fn accepts_diagnostic(&self, diagnostic: Option<&serde_json::Value>) -> bool {
        match (self, diagnostic) {
            (Self::Forbidden, None) => true,
            (Self::Required { constraints }, Some(diagnostic)) => constraints
                .iter()
                .all(|constraint| constraint.accepts(diagnostic)),
            (Self::Forbidden, Some(_)) | (Self::Required { .. }, None) => false,
        }
    }
}

/// One exact domain-free safe-failure tuple rule.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SafeFailureClassifierRule {
    boundary_stage: BoundaryStage,
    coarse_size: SafeFailureSizeRule,
    code: StableId,
    diagnostic: SafeFailureDiagnosticRule,
    failure_class: FailureClass,
    outcome: SafeFailureOutcome,
}

impl SafeFailureClassifierRule {
    /// Constructs one complete closed classifier rule.
    pub fn new(
        code: StableId,
        outcome: SafeFailureOutcome,
        failure_class: FailureClass,
        boundary_stage: BoundaryStage,
        coarse_size: SafeFailureSizeRule,
        diagnostic: SafeFailureDiagnosticRule,
    ) -> Self {
        Self {
            boundary_stage,
            coarse_size,
            code,
            diagnostic,
            failure_class,
            outcome,
        }
    }

    /// Returns the selected stable code.
    pub const fn code(&self) -> &StableId {
        &self.code
    }

    /// Returns the selected observation outcome.
    pub const fn outcome(&self) -> SafeFailureOutcome {
        self.outcome
    }

    /// Returns the selected universal failure class.
    pub const fn failure_class(&self) -> FailureClass {
        self.failure_class
    }

    /// Returns the selected boundary stage.
    pub const fn boundary_stage(&self) -> BoundaryStage {
        self.boundary_stage
    }

    /// Returns the selected coarse-size rule.
    pub const fn coarse_size_rule(&self) -> SafeFailureSizeRule {
        self.coarse_size
    }

    /// Returns the selected typed-diagnostic presence rule.
    pub const fn diagnostic_rule(&self) -> &SafeFailureDiagnosticRule {
        &self.diagnostic
    }

    /// Returns whether this rule accepts the complete dynamic tuple.
    pub fn accepts(
        &self,
        outcome: SafeFailureOutcome,
        failure_class: FailureClass,
        boundary_stage: BoundaryStage,
        coarse_size: Option<CoarseSizeClass>,
        has_diagnostic: bool,
    ) -> bool {
        self.outcome == outcome
            && self.failure_class == failure_class
            && self.boundary_stage == boundary_stage
            && self.coarse_size.accepts(coarse_size)
            && self.diagnostic.accepts_presence(has_diagnostic)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SafeFailureClassifierWire {
    diagnostic_schema_identity: Option<SchemaIdentity>,
    rules: Vec<SafeFailureClassifierRule>,
    safe_failure_contract_ref: ContentRef,
    version: String,
}

/// One strict callback-free classifier selected by an admitted capability binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SafeFailureClassifierDescriptor {
    diagnostic_schema_identity: Option<SchemaIdentity>,
    safe_failure_contract_ref: ContentRef,
    rules: Vec<SafeFailureClassifierRule>,
}

impl SafeFailureClassifierDescriptor {
    /// Returns the one domain-free serialization schema identity.
    pub fn schema_id() -> Result<SchemaId, SafeFailureClassifierError> {
        SchemaId::new(
            "mfm.safe-failure-classifier",
            "1",
            DigestAlgorithm::Sha256JcsV1,
            sha256_digest_bytes(b"schema:mfm.safe-failure-classifier:1"),
        )
        .map_err(|_| SafeFailureClassifierError::InvalidEncoding)
    }

    /// Constructs one closed classifier with unique, unambiguous tuple rules.
    pub fn new(
        safe_failure_contract_ref: ContentRef,
        diagnostic_schema_identity: Option<SchemaIdentity>,
        mut rules: Vec<SafeFailureClassifierRule>,
    ) -> Result<Self, SafeFailureClassifierError> {
        rules.sort();
        let descriptor = Self {
            diagnostic_schema_identity,
            safe_failure_contract_ref,
            rules,
        };
        descriptor.validate_invariant()?;
        descriptor.canonical()?;
        Ok(descriptor)
    }

    /// Strictly decodes exact canonical descriptor bytes.
    pub fn strict_decode(bytes: &[u8]) -> Result<Self, SafeFailureClassifierError> {
        if bytes.len() > MAX_SAFE_FAILURE_CLASSIFIER_BYTES {
            return Err(SafeFailureClassifierError::OutOfBounds);
        }
        let canonical = PlainCanonicalJsonBytes::from_canonical_json_slice(bytes)
            .map_err(|_| SafeFailureClassifierError::InvalidEncoding)?;
        let wire: SafeFailureClassifierWire = serde_json::from_slice(canonical.as_bytes())
            .map_err(|_| SafeFailureClassifierError::InvalidEncoding)?;
        if wire.version != SAFE_FAILURE_CLASSIFIER_DESCRIPTOR_VERSION {
            return Err(SafeFailureClassifierError::InvalidVersion);
        }
        let descriptor = Self {
            diagnostic_schema_identity: wire.diagnostic_schema_identity,
            safe_failure_contract_ref: wire.safe_failure_contract_ref,
            rules: wire.rules,
        };
        descriptor.validate_invariant()?;
        if descriptor.canonical()?.as_bytes() != bytes {
            return Err(SafeFailureClassifierError::InvalidEncoding);
        }
        Ok(descriptor)
    }

    /// Returns exact canonical descriptor bytes.
    pub fn canonical(&self) -> Result<PlainCanonicalJsonBytes, SafeFailureClassifierError> {
        self.validate_invariant()?;
        let wire = SafeFailureClassifierWire {
            diagnostic_schema_identity: self.diagnostic_schema_identity.clone(),
            rules: self.rules.clone(),
            safe_failure_contract_ref: self.safe_failure_contract_ref.clone(),
            version: SAFE_FAILURE_CLASSIFIER_DESCRIPTOR_VERSION.to_owned(),
        };
        let json = serde_json::to_string(&wire)
            .map_err(|_| SafeFailureClassifierError::InvalidEncoding)?;
        let canonical = PlainCanonicalJsonBytes::from_json_str(&json)
            .map_err(|_| SafeFailureClassifierError::InvalidEncoding)?;
        if canonical.as_bytes().len() > MAX_SAFE_FAILURE_CLASSIFIER_BYTES {
            return Err(SafeFailureClassifierError::OutOfBounds);
        }
        Ok(canonical)
    }

    /// Returns the schema-qualified content identity of the exact descriptor bytes.
    pub fn content_ref(&self) -> Result<ContentRef, SafeFailureClassifierError> {
        let canonical = self.canonical()?;
        ContentRef::new(
            Self::schema_id()?,
            ContentDigest::from_digest(
                DigestAlgorithm::Sha256V1,
                sha256_digest_bytes(canonical.as_bytes()),
            ),
        )
        .map_err(|_| SafeFailureClassifierError::InvalidEncoding)
    }

    /// Returns the exact selected safe-failure contract.
    pub const fn safe_failure_contract_ref(&self) -> &ContentRef {
        &self.safe_failure_contract_ref
    }

    /// Returns the complete retained diagnostic schema identity, when required by any rule.
    pub const fn diagnostic_schema_identity(&self) -> Option<&SchemaIdentity> {
        self.diagnostic_schema_identity.as_ref()
    }

    /// Returns classifier rules in canonical full-tuple order.
    pub fn rules(&self) -> &[SafeFailureClassifierRule] {
        &self.rules
    }

    /// Selects one rule from the dynamic outcome-bearing failure projection.
    pub fn classify(
        &self,
        code: &StableId,
        outcome: SafeFailureOutcome,
        coarse_size: Option<CoarseSizeClass>,
        has_diagnostic: bool,
    ) -> Result<&SafeFailureClassifierRule, SafeFailureClassifierError> {
        let mut matches = self.rules.iter().filter(|rule| {
            &rule.code == code
                && rule.outcome == outcome
                && rule.coarse_size.accepts(coarse_size)
                && rule.diagnostic.accepts_presence(has_diagnostic)
        });
        let rule = matches
            .next()
            .ok_or(SafeFailureClassifierError::UnknownTuple)?;
        if matches.next().is_some() {
            return Err(SafeFailureClassifierError::AmbiguousProjection);
        }
        Ok(rule)
    }

    /// Verifies a persisted safe-failure tuple against its exact rule.
    #[allow(clippy::too_many_arguments)]
    pub fn verify(
        &self,
        safe_failure_contract_ref: &ContentRef,
        code: &StableId,
        outcome: SafeFailureOutcome,
        failure_class: FailureClass,
        boundary_stage: BoundaryStage,
        coarse_size: Option<CoarseSizeClass>,
        diagnostic: Option<(&SchemaId, &[u8])>,
    ) -> Result<(), SafeFailureClassifierError> {
        if safe_failure_contract_ref != &self.safe_failure_contract_ref {
            return Err(SafeFailureClassifierError::ContractMismatch);
        }
        let has_diagnostic = diagnostic.is_some();
        let rule = self.classify(code, outcome, coarse_size, has_diagnostic)?;
        if !rule.accepts(
            outcome,
            failure_class,
            boundary_stage,
            coarse_size,
            has_diagnostic,
        ) {
            return Err(SafeFailureClassifierError::InvalidTuple);
        }
        let decoded = match diagnostic {
            None => None,
            Some((schema_id, bytes)) => {
                let identity = self
                    .diagnostic_schema_identity
                    .as_ref()
                    .ok_or(SafeFailureClassifierError::DiagnosticMismatch)?;
                let expected_schema_id = identity
                    .schema_id()
                    .map_err(|_| SafeFailureClassifierError::DiagnosticMismatch)?;
                if &expected_schema_id != schema_id
                    || bytes.len() > MAX_SAFE_FAILURE_DIAGNOSTIC_BYTES
                {
                    return Err(SafeFailureClassifierError::DiagnosticMismatch);
                }
                identity
                    .validate_canonical_value(bytes)
                    .map_err(|_| SafeFailureClassifierError::DiagnosticMismatch)?;
                let canonical = PlainCanonicalJsonBytes::from_canonical_json_slice(bytes)
                    .map_err(|_| SafeFailureClassifierError::DiagnosticMismatch)?;
                Some(
                    serde_json::from_slice::<serde_json::Value>(canonical.as_bytes())
                        .map_err(|_| SafeFailureClassifierError::DiagnosticMismatch)?,
                )
            }
        };
        if !rule.diagnostic.accepts_diagnostic(decoded.as_ref()) {
            return Err(SafeFailureClassifierError::DiagnosticMismatch);
        }
        Ok(())
    }

    fn validate_invariant(&self) -> Result<(), SafeFailureClassifierError> {
        if self.rules.is_empty() || self.rules.len() > MAX_SAFE_FAILURE_CLASSIFIER_RULES {
            return Err(SafeFailureClassifierError::OutOfBounds);
        }
        if !self.rules.windows(2).all(|pair| pair[0] < pair[1]) {
            return Err(SafeFailureClassifierError::DuplicateOrUnsortedRule);
        }
        for rule in &self.rules {
            rule.diagnostic.validate_invariant()?;
        }
        let requires_diagnostic = self
            .rules
            .iter()
            .any(|rule| rule.diagnostic.requires_diagnostic());
        if requires_diagnostic != self.diagnostic_schema_identity.is_some() {
            return Err(SafeFailureClassifierError::InvalidDiagnosticRule);
        }
        if self
            .diagnostic_schema_identity
            .as_ref()
            .is_some_and(|identity| {
                identity.schema_kind != SchemaKind::Value || identity.canonical_json().is_err()
            })
        {
            return Err(SafeFailureClassifierError::InvalidDiagnosticRule);
        }
        for (index, left) in self.rules.iter().enumerate() {
            if self.rules.iter().skip(index + 1).any(|right| {
                left.code == right.code
                    && left.outcome == right.outcome
                    && left.coarse_size == right.coarse_size
                    && left.diagnostic.requires_diagnostic()
                        == right.diagnostic.requires_diagnostic()
            }) {
                return Err(SafeFailureClassifierError::AmbiguousProjection);
            }
        }
        Ok(())
    }
}

/// Error returned by strict safe-failure classifier construction and verification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SafeFailureClassifierError {
    /// Descriptor bytes were not exact canonical JSON or did not round-trip.
    #[error("safe failure classifier encoding is invalid")]
    InvalidEncoding,
    /// The descriptor version is not the one current version.
    #[error("safe failure classifier version is invalid")]
    InvalidVersion,
    /// The descriptor or a retained diagnostic exceeds its reviewed bound.
    #[error("safe failure classifier data exceeds its reviewed bound")]
    OutOfBounds,
    /// Rules were duplicated or not in strict full-tuple order.
    #[error("safe failure classifier rules are duplicated or unordered")]
    DuplicateOrUnsortedRule,
    /// More than one rule matches the same dynamic classifier projection.
    #[error("safe failure classifier projection is ambiguous")]
    AmbiguousProjection,
    /// No rule admits the supplied dynamic tuple.
    #[error("safe failure classifier tuple is not admitted")]
    UnknownTuple,
    /// The supplied safe-failure contract is not the classifier's exact contract.
    #[error("safe failure classifier contract does not match")]
    ContractMismatch,
    /// The supplied tuple is not admitted by the selected rule.
    #[error("safe failure classifier tuple is not admitted")]
    InvalidTuple,
    /// A diagnostic-presence rule or its canonical constraints are malformed.
    #[error("safe failure diagnostic rule is invalid")]
    InvalidDiagnosticRule,
    /// Retained diagnostic schema, bytes, presence, or constraints do not match.
    #[error("safe failure diagnostic does not match its classifier rule")]
    DiagnosticMismatch,
}

impl CoarseSizeClass {
    /// Returns the frozen recoverability-v3 spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Zero => "zero",
            Self::UpTo16Kib => "up_to_16_kib",
            Self::UpTo1Mib => "up_to_1_mib",
            Self::Over1Mib => "over_1_mib",
        }
    }
}

/// Closed failure-code contract selected by one admitted capability binding.
pub trait SafeFailureCode: Clone + Eq {
    /// Returns the stable code persisted by the selected contract.
    fn as_str(&self) -> &'static str;

    /// Validates the exact class, stage, size, and diagnostic-presence tuple.
    fn accepts(
        &self,
        failure_class: FailureClass,
        boundary_stage: BoundaryStage,
        coarse_size_class: Option<CoarseSizeClass>,
        has_diagnostic: bool,
    ) -> bool;
}

/// Error returned when a reviewed failure tuple violates its selected contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SafeFailureError {
    /// The stable code rejected the supplied closed tuple.
    #[error("safe failure tuple is not admitted by its contract")]
    InvalidTuple,
}

/// Generic redaction-safe failure envelope shared by live capability boundaries.
///
/// The selected contract reference fixes the closed code set, allowed tuple
/// combinations, and any diagnostic schema. This value contains no provider
/// message, body, URL, path, credential, or debug output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SafeFailure<Code, DiagnosticRef = ContentRef> {
    references: Box<SafeFailureReferences<DiagnosticRef>>,
    stable_code: Code,
    failure_class: FailureClass,
    boundary_stage: BoundaryStage,
    coarse_size_class: Option<CoarseSizeClass>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SafeFailureReferences<DiagnosticRef> {
    safe_failure_contract_ref: ContentRef,
    diagnostic_ref: Option<DiagnosticRef>,
}

impl<Code, DiagnosticRef> SafeFailure<Code, DiagnosticRef>
where
    Code: SafeFailureCode,
{
    /// Constructs an envelope after validating the selected closed tuple.
    pub fn new(
        safe_failure_contract_ref: ContentRef,
        stable_code: Code,
        failure_class: FailureClass,
        boundary_stage: BoundaryStage,
        coarse_size_class: Option<CoarseSizeClass>,
        diagnostic_ref: Option<DiagnosticRef>,
    ) -> Result<Self, SafeFailureError> {
        if !stable_code.accepts(
            failure_class,
            boundary_stage,
            coarse_size_class,
            diagnostic_ref.is_some(),
        ) {
            return Err(SafeFailureError::InvalidTuple);
        }
        Ok(Self {
            references: Box::new(SafeFailureReferences {
                safe_failure_contract_ref,
                diagnostic_ref,
            }),
            stable_code,
            failure_class,
            boundary_stage,
            coarse_size_class,
        })
    }

    /// Returns the exact selected safe-failure contract.
    pub const fn safe_failure_contract_ref(&self) -> &ContentRef {
        &self.references.safe_failure_contract_ref
    }

    /// Returns the stable closed code.
    pub const fn stable_code(&self) -> &Code {
        &self.stable_code
    }

    /// Returns the reviewed failure class.
    pub const fn failure_class(&self) -> FailureClass {
        self.failure_class
    }

    /// Returns the reviewed boundary stage.
    pub const fn boundary_stage(&self) -> BoundaryStage {
        self.boundary_stage
    }

    /// Returns the optional reviewed coarse size.
    pub const fn coarse_size_class(&self) -> Option<CoarseSizeClass> {
        self.coarse_size_class
    }

    /// Returns the optional reviewed diagnostic reference.
    pub const fn diagnostic_ref(&self) -> Option<&DiagnosticRef> {
        self.references.diagnostic_ref.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use mfm_canonical::{CanonicalValue, RecoverabilityContractV3};
    use mfm_ids::{DigestBytes, SchemaVersion, SemanticTypeId};
    use mfm_values::{FieldDescriptor, SchemaShape};

    use super::*;

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum TestCode {
        Unavailable,
        InvalidResponse,
    }

    impl SafeFailureCode for TestCode {
        fn as_str(&self) -> &'static str {
            match self {
                Self::Unavailable => "unavailable",
                Self::InvalidResponse => "invalid_response",
            }
        }

        fn accepts(
            &self,
            failure_class: FailureClass,
            boundary_stage: BoundaryStage,
            coarse_size_class: Option<CoarseSizeClass>,
            has_diagnostic: bool,
        ) -> bool {
            failure_class == FailureClass::Transport
                && boundary_stage == BoundaryStage::BeforeBoundaryEntry
                && coarse_size_class.is_none()
                && has_diagnostic == matches!(self, Self::InvalidResponse)
        }
    }

    fn reviewed_ref() -> ContentRef {
        let contract = RecoverabilityContractV3::embedded().expect("contract");
        let value = contract
            .encode(
                "mfm.primitive-stable_id.v1",
                &CanonicalValue::String("test.safe-failure".to_owned()),
            )
            .expect("reviewed value");
        contract.content_ref(&value).expect("content ref")
    }

    #[test]
    fn selected_code_controls_the_complete_safe_tuple() {
        let contract_ref = reviewed_ref();
        let diagnostic_ref = reviewed_ref();
        let failure = SafeFailure::<TestCode>::new(
            contract_ref.clone(),
            TestCode::InvalidResponse,
            FailureClass::Transport,
            BoundaryStage::BeforeBoundaryEntry,
            None,
            Some(diagnostic_ref.clone()),
        )
        .expect("reviewed tuple");
        assert_eq!(failure.safe_failure_contract_ref(), &contract_ref);
        assert_eq!(failure.stable_code(), &TestCode::InvalidResponse);
        assert_eq!(failure.failure_class(), FailureClass::Transport);
        assert_eq!(failure.boundary_stage(), BoundaryStage::BeforeBoundaryEntry);
        assert_eq!(failure.coarse_size_class(), None);
        assert_eq!(failure.diagnostic_ref(), Some(&diagnostic_ref));
        assert_eq!(failure.clone(), failure);

        SafeFailure::<TestCode>::new(
            contract_ref,
            TestCode::Unavailable,
            FailureClass::Transport,
            BoundaryStage::BeforeBoundaryEntry,
            None,
            None,
        )
        .expect("reviewed tuple");
        assert_eq!(
            SafeFailure::<TestCode>::new(
                reviewed_ref(),
                TestCode::Unavailable,
                FailureClass::Transport,
                BoundaryStage::BoundaryEntry,
                None,
                None,
            )
            .expect_err("unreviewed stage"),
            SafeFailureError::InvalidTuple
        );
    }

    fn classifier() -> SafeFailureClassifierDescriptor {
        let constraints = vec![
            SafeFailureDiagnosticConstraint::new(
                FieldPath::new("subkind").expect("field path"),
                vec!["missing".to_owned(), "malformed".to_owned()],
            )
            .expect("constraint"),
            SafeFailureDiagnosticConstraint::new(
                FieldPath::new("kind").expect("field path"),
                vec!["response_invalid".to_owned()],
            )
            .expect("constraint"),
        ];
        SafeFailureClassifierDescriptor::new(
            reviewed_ref(),
            Some(diagnostic_identity()),
            vec![
                SafeFailureClassifierRule::new(
                    StableId::new("destination_unavailable").expect("stable code"),
                    SafeFailureOutcome::DidNotEnter,
                    FailureClass::Transport,
                    BoundaryStage::BeforeBoundaryEntry,
                    SafeFailureSizeRule::None,
                    SafeFailureDiagnosticRule::Forbidden,
                ),
                SafeFailureClassifierRule::new(
                    StableId::new("destination_unavailable").expect("stable code"),
                    SafeFailureOutcome::Indeterminate,
                    FailureClass::Transport,
                    BoundaryStage::BoundaryEntry,
                    SafeFailureSizeRule::None,
                    SafeFailureDiagnosticRule::Forbidden,
                ),
                SafeFailureClassifierRule::new(
                    StableId::new("response_invalid").expect("stable code"),
                    SafeFailureOutcome::Indeterminate,
                    FailureClass::UnrepresentableResponse,
                    BoundaryStage::BoundaryObservation,
                    SafeFailureSizeRule::FromFailure,
                    SafeFailureDiagnosticRule::required(constraints).expect("diagnostic rule"),
                ),
            ],
        )
        .expect("classifier")
    }

    fn diagnostic_identity() -> SchemaIdentity {
        let semantic_type_id = SemanticTypeId::new(
            "mfm.test",
            "safe-diagnostic",
            "1",
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([0x44; 32]),
        )
        .expect("semantic id");
        SchemaIdentity::new(
            SchemaKind::Value,
            Some(semantic_type_id),
            "mfm.test.safe_diagnostic",
            SchemaVersion::new("1").expect("schema version"),
            SchemaShape::named_struct(vec![
                FieldDescriptor::required("kind", SchemaShape::String),
                FieldDescriptor::required("status", SchemaShape::UnsignedInteger { bits: 16 }),
                FieldDescriptor::required("subkind", SchemaShape::String),
            ])
            .expect("shape"),
        )
        .expect("diagnostic identity")
    }

    #[test]
    fn classifier_round_trips_exact_canonical_bytes_and_verifies_the_complete_tuple() {
        let classifier = classifier();
        let canonical = classifier.canonical().expect("canonical classifier");
        let diagnostic_identity = classifier
            .diagnostic_schema_identity()
            .expect("diagnostic identity");
        let diagnostic_schema_id = diagnostic_identity
            .schema_id()
            .expect("diagnostic schema id");
        let diagnostic = br#"{"kind":"response_invalid","status":500,"subkind":"malformed"}"#;
        assert_eq!(
            SafeFailureClassifierDescriptor::strict_decode(canonical.as_bytes())
                .expect("strict classifier"),
            classifier
        );
        classifier
            .verify(
                classifier.safe_failure_contract_ref(),
                &StableId::new("response_invalid").expect("stable code"),
                SafeFailureOutcome::Indeterminate,
                FailureClass::UnrepresentableResponse,
                BoundaryStage::BoundaryObservation,
                Some(CoarseSizeClass::UpTo16Kib),
                Some((&diagnostic_schema_id, diagnostic)),
            )
            .expect("admitted tuple");
        classifier
            .verify(
                classifier.safe_failure_contract_ref(),
                &StableId::new("destination_unavailable").expect("stable code"),
                SafeFailureOutcome::DidNotEnter,
                FailureClass::Transport,
                BoundaryStage::BeforeBoundaryEntry,
                None,
                None,
            )
            .expect("same-code did-not-enter tuple");
        classifier
            .verify(
                classifier.safe_failure_contract_ref(),
                &StableId::new("destination_unavailable").expect("stable code"),
                SafeFailureOutcome::Indeterminate,
                FailureClass::Transport,
                BoundaryStage::BoundaryEntry,
                None,
                None,
            )
            .expect("same-code indeterminate tuple");
        assert_eq!(
            classifier
                .verify(
                    classifier.safe_failure_contract_ref(),
                    &StableId::new("response_invalid").expect("stable code"),
                    SafeFailureOutcome::DidNotEnter,
                    FailureClass::UnrepresentableResponse,
                    BoundaryStage::BoundaryObservation,
                    Some(CoarseSizeClass::UpTo16Kib),
                    Some((&diagnostic_schema_id, diagnostic)),
                )
                .expect_err("wrong outcome"),
            SafeFailureClassifierError::UnknownTuple
        );
    }

    #[test]
    fn classifier_validates_diagnostic_shape_before_rule_constraints() {
        let classifier = classifier();
        let schema_id = classifier
            .diagnostic_schema_identity()
            .expect("diagnostic identity")
            .schema_id()
            .expect("schema id");
        let wrong_type = br#"{"kind":"response_invalid","status":"500","subkind":"malformed"}"#;
        assert_eq!(
            classifier
                .verify(
                    classifier.safe_failure_contract_ref(),
                    &StableId::new("response_invalid").expect("stable code"),
                    SafeFailureOutcome::Indeterminate,
                    FailureClass::UnrepresentableResponse,
                    BoundaryStage::BoundaryObservation,
                    Some(CoarseSizeClass::UpTo16Kib),
                    Some((&schema_id, wrong_type)),
                )
                .expect_err("shape mismatch"),
            SafeFailureClassifierError::DiagnosticMismatch
        );
    }

    #[test]
    fn classifier_accepts_16384_diagnostic_bytes_and_rejects_16385() {
        let identity = SchemaIdentity::new(
            SchemaKind::Value,
            Some(
                SemanticTypeId::new(
                    "mfm.test",
                    "bounded-safe-diagnostic",
                    "1",
                    DigestAlgorithm::Sha256JcsV1,
                    DigestBytes::from_array([0x55; 32]),
                )
                .expect("semantic id"),
            ),
            "mfm.test.bounded_safe_diagnostic",
            SchemaVersion::new("1").expect("schema version"),
            SchemaShape::named_struct(vec![
                FieldDescriptor::required("kind", SchemaShape::String),
                FieldDescriptor::required("payload", SchemaShape::String),
            ])
            .expect("diagnostic shape"),
        )
        .expect("diagnostic identity");
        let schema_id = identity.schema_id().expect("diagnostic schema id");
        let classifier = SafeFailureClassifierDescriptor::new(
            reviewed_ref(),
            Some(identity),
            vec![SafeFailureClassifierRule::new(
                StableId::new("bounded").expect("stable code"),
                SafeFailureOutcome::Indeterminate,
                FailureClass::UnrepresentableResponse,
                BoundaryStage::BoundaryObservation,
                SafeFailureSizeRule::FromFailure,
                SafeFailureDiagnosticRule::required(vec![SafeFailureDiagnosticConstraint::new(
                    FieldPath::new("kind").expect("field path"),
                    vec!["bounded".to_owned()],
                )
                .expect("diagnostic constraint")])
                .expect("diagnostic rule"),
            )],
        )
        .expect("classifier");
        let overhead = br#"{"kind":"bounded","payload":""#.len() + br#""}"#.len();
        let at_limit = format!(
            r#"{{"kind":"bounded","payload":"{}"}}"#,
            "x".repeat(MAX_SAFE_FAILURE_DIAGNOSTIC_BYTES - overhead)
        );
        assert_eq!(at_limit.len(), MAX_SAFE_FAILURE_DIAGNOSTIC_BYTES);
        classifier
            .verify(
                classifier.safe_failure_contract_ref(),
                &StableId::new("bounded").expect("stable code"),
                SafeFailureOutcome::Indeterminate,
                FailureClass::UnrepresentableResponse,
                BoundaryStage::BoundaryObservation,
                Some(CoarseSizeClass::UpTo16Kib),
                Some((&schema_id, at_limit.as_bytes())),
            )
            .expect("exactly bounded diagnostic");

        let over_limit = format!(
            r#"{{"kind":"bounded","payload":"{}"}}"#,
            "x".repeat(MAX_SAFE_FAILURE_DIAGNOSTIC_BYTES + 1 - overhead)
        );
        assert_eq!(over_limit.len(), MAX_SAFE_FAILURE_DIAGNOSTIC_BYTES + 1);
        assert_eq!(
            classifier
                .verify(
                    classifier.safe_failure_contract_ref(),
                    &StableId::new("bounded").expect("stable code"),
                    SafeFailureOutcome::Indeterminate,
                    FailureClass::UnrepresentableResponse,
                    BoundaryStage::BoundaryObservation,
                    Some(CoarseSizeClass::UpTo16Kib),
                    Some((&schema_id, over_limit.as_bytes())),
                )
                .expect_err("oversized diagnostic"),
            SafeFailureClassifierError::DiagnosticMismatch
        );
    }

    #[test]
    fn classifier_strict_decode_rejects_unknown_duplicate_and_noncanonical_shapes() {
        let canonical = classifier().canonical().expect("canonical classifier");
        let mut unknown: serde_json::Value =
            serde_json::from_slice(canonical.as_bytes()).expect("json");
        unknown
            .as_object_mut()
            .expect("object")
            .insert("fallback".to_owned(), serde_json::Value::Bool(true));
        let unknown = PlainCanonicalJsonBytes::from_json_str(&unknown.to_string())
            .expect("canonical hostile value");
        assert_eq!(
            SafeFailureClassifierDescriptor::strict_decode(unknown.as_bytes())
                .expect_err("unknown field"),
            SafeFailureClassifierError::InvalidEncoding
        );

        let mut duplicate: serde_json::Value =
            serde_json::from_slice(canonical.as_bytes()).expect("json");
        let rules = duplicate["rules"].as_array_mut().expect("rules");
        rules.push(rules[0].clone());
        let duplicate = PlainCanonicalJsonBytes::from_json_str(&duplicate.to_string())
            .expect("canonical hostile value");
        assert_eq!(
            SafeFailureClassifierDescriptor::strict_decode(duplicate.as_bytes())
                .expect_err("duplicate code"),
            SafeFailureClassifierError::DuplicateOrUnsortedRule
        );

        let mut unordered_constraints: serde_json::Value =
            serde_json::from_slice(canonical.as_bytes()).expect("json");
        let response_rule = unordered_constraints["rules"]
            .as_array_mut()
            .expect("rules")
            .iter_mut()
            .find(|rule| rule["code"] == "response_invalid")
            .expect("response rule");
        response_rule["diagnostic"]["constraints"]
            .as_array_mut()
            .expect("constraints")
            .swap(0, 1);
        let unordered_constraints =
            PlainCanonicalJsonBytes::from_json_str(&unordered_constraints.to_string())
                .expect("canonical hostile value");
        assert_eq!(
            SafeFailureClassifierDescriptor::strict_decode(unordered_constraints.as_bytes())
                .expect_err("unordered constraints"),
            SafeFailureClassifierError::InvalidDiagnosticRule
        );

        let mut unordered_values: serde_json::Value =
            serde_json::from_slice(canonical.as_bytes()).expect("json");
        let response_rule = unordered_values["rules"]
            .as_array_mut()
            .expect("rules")
            .iter_mut()
            .find(|rule| rule["code"] == "response_invalid")
            .expect("response rule");
        response_rule["diagnostic"]["constraints"][1]["allowed_values"]
            .as_array_mut()
            .expect("allowed values")
            .swap(0, 1);
        let unordered_values =
            PlainCanonicalJsonBytes::from_json_str(&unordered_values.to_string())
                .expect("canonical hostile value");
        assert_eq!(
            SafeFailureClassifierDescriptor::strict_decode(unordered_values.as_bytes())
                .expect_err("unordered allowed values"),
            SafeFailureClassifierError::InvalidDiagnosticRule
        );

        let noncanonical = canonical.as_str().replacen('{', "{ ", 1);
        assert_eq!(
            SafeFailureClassifierDescriptor::strict_decode(noncanonical.as_bytes())
                .expect_err("noncanonical whitespace"),
            SafeFailureClassifierError::InvalidEncoding
        );
    }

    #[test]
    fn classifier_constructor_rejects_an_oversized_canonical_descriptor() {
        let constraints = (0..300)
            .map(|index| {
                SafeFailureDiagnosticConstraint::new(
                    FieldPath::new(format!("field_{index:03}")).expect("field path"),
                    vec!["x".repeat(256)],
                )
                .expect("constraint")
            })
            .collect();
        let rule = SafeFailureClassifierRule::new(
            StableId::new("oversized").expect("stable code"),
            SafeFailureOutcome::Indeterminate,
            FailureClass::UnrepresentableResponse,
            BoundaryStage::BoundaryObservation,
            SafeFailureSizeRule::FromFailure,
            SafeFailureDiagnosticRule::required(constraints).expect("diagnostic rule"),
        );
        assert_eq!(
            SafeFailureClassifierDescriptor::new(
                reviewed_ref(),
                Some(diagnostic_identity()),
                vec![rule],
            )
            .expect_err("oversized classifier"),
            SafeFailureClassifierError::OutOfBounds
        );
    }
}
