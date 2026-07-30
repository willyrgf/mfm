use std::str::FromStr;

use mfm_canonical::{
    CanonicalValue, RecoverabilityContract, RecoverabilityError, ValidatedCanonicalValue,
};
use mfm_ids::{
    AppendRequestId, ArtifactId, CheckedStringError, ContentDigest, ContentRef, EffectKey,
    EntryPointId, FieldPath, IdentityError, InvocationIdentity, NodeId, RunId, SchemaId,
    SemanticDigest, SemanticTypeId, SpecHash, StableId, StoreEpoch, StoreScopeId, TenantScopeId,
};

macro_rules! define_schema_value {
    (
        $(
            $(#[$meta:meta])*
            $visibility:vis struct $name:ident => $contract:literal;
        )+
    ) => {
        $(
            $(#[$meta])*
            #[derive(Clone, PartialEq, Eq)]
            $visibility struct $name {
                validated: mfm_canonical::ValidatedCanonicalValue,
            }

            impl $name {
                /// Strictly decodes exact canonical JSON with the embedded annex.
                pub fn strict_decode(bytes: &[u8]) -> $crate::Result<Self> {
                    $crate::codec::decode(bytes)
                }

                /// Encodes a canonical value only after annex schema validation.
                pub fn from_canonical_value(
                    value: mfm_canonical::CanonicalValue,
                ) -> $crate::Result<Self> {
                    $crate::codec::encode(&value)
                }

                /// Returns the exact annex-validated canonical bytes.
                pub fn as_bytes(&self) -> &[u8] {
                    self.validated.as_bytes()
                }

                /// Returns the annex-derived schema identity.
                pub fn schema_id(&self) -> &mfm_ids::SchemaId {
                    self.validated.schema_id()
                }

                /// Reconstructs the validated canonical tree for composition.
                pub fn canonical_value(
                    &self,
                ) -> $crate::Result<mfm_canonical::CanonicalValue> {
                    self.validated.canonical_value().map_err(Into::into)
                }

                /// Returns the raw-byte content reference for this exact value.
                pub fn content_ref(&self) -> $crate::Result<mfm_ids::ContentRef> {
                    $crate::codec::contract()?
                        .content_ref(&self.validated)
                        .map_err(Into::into)
                }
            }

            impl $crate::codec::sealed::Sealed for $name {}

            impl $crate::PersistedJournalValue for $name {
                fn validated(&self) -> &mfm_canonical::ValidatedCanonicalValue {
                    &self.validated
                }
            }

            impl $crate::codec::SchemaValue for $name {
                const CONTRACT: &'static str = $contract;

                fn from_validated(
                    validated: mfm_canonical::ValidatedCanonicalValue,
                ) -> Self {
                    Self { validated }
                }
            }

            impl std::fmt::Debug for $name {
                fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                    formatter
                        .debug_struct(stringify!($name))
                        .field("schema_id", self.validated.schema_id())
                        .finish_non_exhaustive()
                }
            }
        )+
    };
}

pub(crate) use define_schema_value;

/// Result type for journal value construction and projection.
pub type Result<T> = std::result::Result<T, JournalError>;

/// Failure returned by an annex-backed journal value operation.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum JournalError {
    /// The frozen recoverability codec rejected a value.
    #[error(transparent)]
    Recoverability(#[from] RecoverabilityError),
    /// A fixed canonical object could not be constructed.
    #[error("canonical journal object construction failed")]
    CanonicalConstruction,
    /// A checked identity could not be reconstructed from validated canonical data.
    #[error("validated journal identity failed checked reconstruction")]
    Identity,
    /// An annex-validated value could not be projected into its closed Rust view.
    #[error("annex-validated journal value failed typed projection")]
    Projection,
    /// A fact-selection response does not exactly cover its authored request.
    #[error("fact-selection response does not exactly cover its request")]
    FactSelectionCoverage,
    /// Two repeated exact references in one persisted value disagree.
    #[error("persisted journal references do not identify the same record")]
    ReferenceMismatch,
    /// A derived access-audit entry disagrees with its authorization or observation.
    #[error("access-audit entry does not match its journal evidence")]
    AccessAuditMismatch,
    /// A non-domain failure violates its closed relation or contextual layer.
    #[error("non-domain failure relation is invalid")]
    NonDomainFailureMismatch,
    /// A fact-selection request could not provide its frozen identity.
    #[error("fact-selection request identity is invalid")]
    FactSelectionRequest(#[source] mfm_facts::FactError),
    /// A shared retained-value contract failed exact validation.
    #[error(transparent)]
    RetainedValueContract(#[from] mfm_values::ValueError),
}

impl JournalError {
    /// Returns the exact recoverability-codec error, when structural validation failed.
    pub const fn recoverability_error(&self) -> Option<&RecoverabilityError> {
        match self {
            Self::Recoverability(error) => Some(error),
            Self::CanonicalConstruction
            | Self::Identity
            | Self::Projection
            | Self::FactSelectionCoverage
            | Self::ReferenceMismatch
            | Self::AccessAuditMismatch
            | Self::NonDomainFailureMismatch
            | Self::FactSelectionRequest(_)
            | Self::RetainedValueContract(_) => None,
        }
    }
}

impl From<IdentityError> for JournalError {
    fn from(_: IdentityError) -> Self {
        Self::Identity
    }
}

impl From<CheckedStringError> for JournalError {
    fn from(_: CheckedStringError) -> Self {
        Self::Identity
    }
}

impl From<mfm_capabilities::NonDomainFailureError> for JournalError {
    fn from(_: mfm_capabilities::NonDomainFailureError) -> Self {
        Self::NonDomainFailureMismatch
    }
}

pub(crate) mod sealed {
    pub trait Sealed {}
}

/// Shared read-only surface implemented by every persisted journal value.
///
/// Implementations have no Serde encoding. The exact bytes and schema identity
/// come only from the embedded recoverability annex.
pub trait PersistedJournalValue: sealed::Sealed {
    /// Returns the annex-validated value that is the sole persisted authority.
    fn validated(&self) -> &ValidatedCanonicalValue;

    /// Returns the registered annex contract selected during validation.
    fn schema_contract(&self) -> &str {
        self.validated().schema_contract()
    }

    /// Returns the annex-derived schema identity.
    fn schema_id(&self) -> &SchemaId {
        self.validated().schema_id()
    }

    /// Returns the exact canonical JSON bytes.
    fn as_bytes(&self) -> &[u8] {
        self.validated().as_bytes()
    }

    /// Reconstructs the canonical value tree for composition into another
    /// annex-validated value.
    fn canonical_value(&self) -> Result<CanonicalValue> {
        self.validated().canonical_value().map_err(Into::into)
    }

    /// Returns the lightweight schema/raw-byte content identity.
    fn content_ref(&self) -> Result<ContentRef> {
        contract()?
            .content_ref(self.validated())
            .map_err(Into::into)
    }
}

pub(crate) trait SchemaValue: PersistedJournalValue + Sized {
    const CONTRACT: &'static str;

    fn from_validated(validated: ValidatedCanonicalValue) -> Self;
}

pub(crate) fn contract() -> Result<&'static RecoverabilityContract> {
    RecoverabilityContract::embedded().map_err(Into::into)
}

pub(crate) fn decode<T: SchemaValue>(bytes: &[u8]) -> Result<T> {
    contract()?
        .strict_decode(T::CONTRACT, bytes)
        .map(T::from_validated)
        .map_err(Into::into)
}

pub(crate) fn encode<T: SchemaValue>(value: &CanonicalValue) -> Result<T> {
    contract()?
        .encode(T::CONTRACT, value)
        .map(T::from_validated)
        .map_err(Into::into)
}

pub(crate) fn domain_digest(
    domain: &str,
    value: &impl PersistedJournalValue,
) -> Result<SemanticDigest> {
    contract()?
        .semantic_digest(domain, value.validated())
        .map_err(Into::into)
}

pub(crate) fn object(
    entries: impl IntoIterator<Item = (impl Into<String>, CanonicalValue)>,
) -> Result<CanonicalValue> {
    CanonicalValue::object(entries).map_err(|_| JournalError::CanonicalConstruction)
}

pub(crate) fn tagged_object(
    kind: &str,
    entries: impl IntoIterator<Item = (impl Into<String>, CanonicalValue)>,
) -> Result<CanonicalValue> {
    let mut values = vec![("kind".to_owned(), CanonicalValue::String(kind.to_owned()))];
    values.extend(entries.into_iter().map(|(key, value)| (key.into(), value)));
    object(values)
}

pub(crate) fn object_entries(
    value: &impl PersistedJournalValue,
) -> Result<Vec<(String, CanonicalValue)>> {
    let CanonicalValue::Object(object) = value.canonical_value()? else {
        return Err(JournalError::Projection);
    };
    Ok(object
        .entries()
        .map(|(key, value)| (key.to_owned(), value.clone()))
        .collect())
}

pub(crate) fn required_field(
    value: &impl PersistedJournalValue,
    field: &str,
) -> Result<CanonicalValue> {
    object_entries(value)?
        .into_iter()
        .find_map(|(key, value)| (key == field).then_some(value))
        .ok_or(JournalError::Projection)
}

pub(crate) fn nullable_field(
    value: &impl PersistedJournalValue,
    field: &str,
) -> Result<Option<CanonicalValue>> {
    match required_field(value, field)? {
        CanonicalValue::Null => Ok(None),
        value => Ok(Some(value)),
    }
}

pub(crate) fn tag(value: &impl PersistedJournalValue) -> Result<String> {
    string_field(value, "kind")
}

pub(crate) fn string_field(value: &impl PersistedJournalValue, field: &str) -> Result<String> {
    match required_field(value, field)? {
        CanonicalValue::String(value) => Ok(value),
        CanonicalValue::Decimal(value) => Ok(value.as_str().to_owned()),
        _ => Err(JournalError::Projection),
    }
}

pub(crate) fn bool_field(value: &impl PersistedJournalValue, field: &str) -> Result<bool> {
    match required_field(value, field)? {
        CanonicalValue::Bool(value) => Ok(value),
        _ => Err(JournalError::Projection),
    }
}

pub(crate) fn unsigned_field(value: &impl PersistedJournalValue, field: &str) -> Result<u64> {
    match required_field(value, field)? {
        CanonicalValue::Unsigned(value) => Ok(value),
        CanonicalValue::String(value) => value.parse().map_err(|_| JournalError::Projection),
        CanonicalValue::Decimal(value) => {
            value.as_str().parse().map_err(|_| JournalError::Projection)
        }
        _ => Err(JournalError::Projection),
    }
}

pub(crate) fn u32_field(value: &impl PersistedJournalValue, field: &str) -> Result<u32> {
    unsigned_field(value, field)?
        .try_into()
        .map_err(|_| JournalError::Projection)
}

pub(crate) fn array_field(
    value: &impl PersistedJournalValue,
    field: &str,
) -> Result<Vec<CanonicalValue>> {
    match required_field(value, field)? {
        CanonicalValue::Array(values) => Ok(values),
        _ => Err(JournalError::Projection),
    }
}

pub(crate) fn run_id_field(value: &impl PersistedJournalValue, field: &str) -> Result<RunId> {
    RunId::from_str(&string_field(value, field)?).map_err(Into::into)
}

pub(crate) fn node_id_field(value: &impl PersistedJournalValue, field: &str) -> Result<NodeId> {
    NodeId::from_str(&string_field(value, field)?).map_err(Into::into)
}

pub(crate) fn spec_hash_field(value: &impl PersistedJournalValue, field: &str) -> Result<SpecHash> {
    SpecHash::from_str(&string_field(value, field)?).map_err(Into::into)
}

pub(crate) fn content_digest_field(
    value: &impl PersistedJournalValue,
    field: &str,
) -> Result<ContentDigest> {
    ContentDigest::from_str(&string_field(value, field)?).map_err(Into::into)
}

pub(crate) fn schema_id_field(value: &impl PersistedJournalValue, field: &str) -> Result<SchemaId> {
    SchemaId::from_str(&string_field(value, field)?).map_err(Into::into)
}

pub(crate) fn artifact_id_field(
    value: &impl PersistedJournalValue,
    field: &str,
) -> Result<ArtifactId> {
    ArtifactId::from_str(&string_field(value, field)?).map_err(Into::into)
}

pub(crate) fn semantic_type_id_field(
    value: &impl PersistedJournalValue,
    field: &str,
) -> Result<SemanticTypeId> {
    SemanticTypeId::from_str(&string_field(value, field)?).map_err(Into::into)
}

pub(crate) fn effect_key_field(
    value: &impl PersistedJournalValue,
    field: &str,
) -> Result<EffectKey> {
    EffectKey::from_str(&string_field(value, field)?).map_err(Into::into)
}

pub(crate) fn store_scope_id_field(
    value: &impl PersistedJournalValue,
    field: &str,
) -> Result<StoreScopeId> {
    StoreScopeId::from_str(&string_field(value, field)?).map_err(Into::into)
}

pub(crate) fn tenant_scope_id_field(
    value: &impl PersistedJournalValue,
    field: &str,
) -> Result<TenantScopeId> {
    TenantScopeId::from_str(&string_field(value, field)?).map_err(Into::into)
}

pub(crate) fn store_epoch_field(
    value: &impl PersistedJournalValue,
    field: &str,
) -> Result<StoreEpoch> {
    StoreEpoch::parse(string_field(value, field)?).map_err(Into::into)
}

pub(crate) fn stable_id_field(value: &impl PersistedJournalValue, field: &str) -> Result<StableId> {
    StableId::new(string_field(value, field)?).map_err(Into::into)
}

pub(crate) fn append_request_id_field(
    value: &impl PersistedJournalValue,
    field: &str,
) -> Result<AppendRequestId> {
    AppendRequestId::new(string_field(value, field)?).map_err(Into::into)
}

pub(crate) fn invocation_identity_field(
    value: &impl PersistedJournalValue,
    field: &str,
) -> Result<InvocationIdentity> {
    InvocationIdentity::new(string_field(value, field)?).map_err(Into::into)
}

pub(crate) fn field_path_field(
    value: &impl PersistedJournalValue,
    field: &str,
) -> Result<FieldPath> {
    FieldPath::new(string_field(value, field)?).map_err(Into::into)
}

pub(crate) fn entry_point_id_field(
    value: &impl PersistedJournalValue,
    field: &str,
) -> Result<EntryPointId> {
    EntryPointId::new(string_field(value, field)?).map_err(Into::into)
}

pub(crate) fn content_ref_field(
    value: &impl PersistedJournalValue,
    field: &str,
) -> Result<ContentRef> {
    content_ref_from_canonical(required_field(value, field)?)
}

pub(crate) fn cv_string(value: impl ToString) -> CanonicalValue {
    CanonicalValue::String(value.to_string())
}

pub(crate) fn cv_decimal(value: u64) -> Result<CanonicalValue> {
    mfm_canonical::DecimalString::new_variable(value.to_string())
        .map(CanonicalValue::Decimal)
        .map_err(|_| JournalError::CanonicalConstruction)
}

pub(crate) fn cv_array(
    values: impl IntoIterator<Item = Result<CanonicalValue>>,
) -> Result<CanonicalValue> {
    values
        .into_iter()
        .collect::<Result<Vec<_>>>()
        .map(CanonicalValue::Array)
}

pub(crate) fn cv_content_ref(value: &ContentRef) -> Result<CanonicalValue> {
    object([
        ("schema_id", cv_string(value.schema_id().as_str())),
        ("content_digest", cv_string(value.content_digest().as_str())),
    ])
}

pub(crate) fn cv_retained_value_contract(
    value: &mfm_values::RetainedValueContract,
) -> Result<CanonicalValue> {
    value.validated()?.canonical_value().map_err(Into::into)
}

pub(crate) fn retained_value_contract_from_canonical(
    value: CanonicalValue,
) -> Result<mfm_values::RetainedValueContract> {
    let validated = contract()?.encode("mfm.retained-value-contract.v1", &value)?;
    mfm_values::RetainedValueContract::from_validated(validated).map_err(Into::into)
}

pub(crate) fn content_ref_from_canonical(value: CanonicalValue) -> Result<ContentRef> {
    let validated = contract()?.encode("mfm.content-ref.v1", &value)?;
    let CanonicalValue::Object(object) = validated.canonical_value()? else {
        return Err(JournalError::Projection);
    };
    let mut schema_id = None;
    let mut content_digest = None;
    for (key, value) in object.entries() {
        match (key, value) {
            ("schema_id", CanonicalValue::String(value)) => {
                schema_id = Some(SchemaId::from_str(value)?);
            }
            ("content_digest", CanonicalValue::String(value)) => {
                content_digest = Some(ContentDigest::from_str(value)?);
            }
            _ => {}
        }
    }
    ContentRef::new(
        schema_id.ok_or(JournalError::Projection)?,
        content_digest.ok_or(JournalError::Projection)?,
    )
    .map_err(Into::into)
}
