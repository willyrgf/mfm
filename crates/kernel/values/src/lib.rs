#![warn(missing_docs)]
//! Typed value and descriptor contracts for the MFM typed kernel.
//!
//! This crate owns the first derive-backed value/config/output descriptor
//! contract. Domain crates should get implementations from derives; the
//! hand-written implementations here are framework-owned generic constructors.
//!
//! ```
//! use mfm_values::{DecimalScale, FieldDescriptor, SchemaShape};
//!
//! let shape = SchemaShape::named_struct(vec![FieldDescriptor::required(
//!     "amount",
//!     SchemaShape::DecimalString {
//!         scale: DecimalScale::Variable,
//!     },
//! )])?;
//!
//! assert!(matches!(shape, SchemaShape::Struct { .. }));
//! # Ok::<(), mfm_values::ValueError>(())
//! ```

use std::collections::BTreeMap;

use mfm_canonical::{
    CanonicalBytes, CanonicalJsonBytes, CanonicalValue, DecimalString, PlainCanonicalJsonBytes,
};
use mfm_ids::{DigestAlgorithm, NameToken, SchemaId, SchemaVersion, SemanticTypeId};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

mod generic_values;
pub use self::generic_values::{ArtifactRef, NonEmpty};
mod retained;
pub use self::retained::{
    component_object_evidence_contract_canonical, component_object_evidence_contract_ref,
    RetainedValueContract,
};

// Keep this list intentionally small and high-signal to avoid false positives on public
// descriptive fields while still blocking common secret-bearing persisted surfaces.
const SECRET_MARKERS: &[&str] = &[
    "password",
    "passphrase",
    "mnemonic",
    "private_key",
    "privatekey",
    "seed phrase",
    "seed_phrase",
    "seedphrase",
    "api_key",
    "apikey",
    "x-api-key",
    "x_api_key",
    "access_key",
    "accesskey",
    "secret_key",
    "secretkey",
    "aws_access_key_id",
    "aws_secret_access_key",
    "access_token",
    "refresh_token",
    "id_token",
    "authorization",
    "bearer ",
];
const MAX_SCHEMA_IDENTITY_BYTES: usize = 65_536;
/// Maximum recursive depth admitted by current schema identities and values.
pub const MAX_SCHEMA_DEPTH: usize = 64;

/// Result type for descriptor and value-contract operations.
pub type Result<T> = std::result::Result<T, ValueError>;

/// Error returned by value/config descriptor helpers.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ValueError {
    /// Descriptor construction failed.
    #[error("descriptor error: {0}")]
    Descriptor(String),
    /// Identity parsing failed.
    #[error("identity error: {0}")]
    Identity(String),
    /// A schema identity was not exact canonical descriptor material.
    #[error("invalid schema identity")]
    InvalidSchemaIdentity,
    /// Canonical value bytes did not match the complete closed schema shape.
    #[error("value does not match schema shape")]
    SchemaShapeMismatch,
    /// Artifact reference identity does not match the expected value type.
    #[error("artifact reference {field} mismatch: expected {expected}, got {actual}")]
    ArtifactTypeMismatch {
        /// Field that mismatched.
        field: &'static str,
        /// Expected typed identity.
        expected: String,
        /// Actual typed identity.
        actual: String,
    },
    /// Config validation failed.
    #[error("config error: {0}")]
    Config(String),
    /// A retained-value contract failed exact annex validation.
    #[error("invalid retained-value contract")]
    RetainedValueContract,
    /// The frozen recoverability codec rejected a retained-value contract.
    #[error(transparent)]
    Recoverability(#[from] mfm_canonical::RecoverabilityError),
}

/// Returns `true` when `input` matches MFM's high-signal secret-marker policy.
///
/// This is a conservative persisted-surface guard. It is not intended to prove that arbitrary
/// text is safe; it blocks known secret field markers and mnemonic-shaped phrases before values
/// become canonical artifacts, configs, events, or public outputs.
pub fn string_contains_secret_marker(input: &str) -> bool {
    let lower = input.to_ascii_lowercase();
    if SECRET_MARKERS.iter().any(|marker| lower.contains(marker)) {
        return true;
    }

    looks_like_mnemonic_phrase(input)
}

/// Returns the first string-map key whose key or value matches the secret-marker policy.
pub fn string_map_secret_marker_key(map: &BTreeMap<String, String>) -> Option<&str> {
    map.iter()
        .find(|(key, value)| {
            string_contains_secret_marker(key) || string_contains_secret_marker(value)
        })
        .map(|(key, _)| key.as_str())
}

fn looks_like_mnemonic_phrase(input: &str) -> bool {
    let mut words = input.split_whitespace().peekable();
    if words.peek().is_none() {
        return false;
    }

    let mut count = 0usize;
    for word in words {
        count += 1;
        if !word.chars().all(|ch| ch.is_ascii_lowercase()) {
            return false;
        }
    }

    (12..=24).contains(&count)
}

/// Error returned by typed planning config validation.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct ConfigError {
    message: String,
}

impl ConfigError {
    /// Creates a config validation error.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    /// Returns the stable human-readable diagnostic.
    pub fn message(&self) -> &str {
        &self.message
    }
}

/// Values that may cross typed state boundaries.
pub trait MfmValue: Serialize + DeserializeOwned + Send + Sync + 'static {
    /// Returns the schema descriptor for this value type.
    fn schema_descriptor() -> Result<SchemaDescriptor>;

    /// Returns the stable semantic identity for this value kind.
    fn semantic_id() -> Result<SemanticTypeId>;

    /// Derives this value's schema id from its schema descriptor identity.
    fn schema_id() -> Result<SchemaId> {
        Self::schema_descriptor()?.schema_id()
    }
}

/// Deterministic planning config that is safe to persist in manifests/specs.
pub trait MfmConfig: Serialize + DeserializeOwned + Send + Sync + 'static {
    /// Returns the schema descriptor for this planning config type.
    fn schema_descriptor() -> Result<SchemaDescriptor>;

    /// Derives this config's schema id from its schema descriptor identity.
    fn schema_id() -> Result<SchemaId> {
        Self::schema_descriptor()?.schema_id()
    }

    /// Validates authored config before typed expansion.
    fn validate(&self) -> std::result::Result<(), ConfigError> {
        Ok(())
    }
}

/// Config value that has passed its [`MfmConfig`] semantic validation hook.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedConfig<C: MfmConfig> {
    config: C,
}

impl<C: MfmConfig> ValidatedConfig<C> {
    /// Validates a config value and mints validated config authority.
    pub fn new(config: C) -> std::result::Result<Self, ConfigError> {
        config.validate()?;
        Ok(Self { config })
    }

    /// Returns the validated config value.
    pub const fn as_ref(&self) -> &C {
        &self.config
    }

    /// Consumes this authority into the validated config value.
    pub fn into_inner(self) -> C {
        self.config
    }

    /// Serializes the validated config through the shared canonical JSON path.
    ///
    /// Program certification and configuration publication both use this method so a semantic config
    /// has one canonical byte representation and one content digest implementation.
    pub fn canonical_json(
        &self,
    ) -> std::result::Result<mfm_canonical::PlainCanonicalJsonBytes, ConfigError> {
        let json = serde_json::to_string(&self.config)
            .map_err(|error| ConfigError::new(format!("failed to serialize config: {error}")))?;
        mfm_canonical::PlainCanonicalJsonBytes::from_json_str(&json)
            .map_err(|error| ConfigError::new(format!("failed to canonicalize config: {error}")))
    }
}

/// Descriptor contract for public launch/render output surfaces.
pub trait PublicOutputDescriptor: Send + Sync + 'static {
    /// Returns the public output schema descriptor.
    fn public_schema_descriptor() -> Result<SchemaDescriptor>;

    /// Derives the public output schema id from the descriptor identity.
    fn public_schema_id() -> Result<SchemaId> {
        Self::public_schema_descriptor()?.schema_id()
    }
}

/// Descriptor contract for typed state input structs.
pub trait StateInput: Send + Sync + 'static {
    /// Returns the state input schema descriptor.
    fn input_schema_descriptor() -> Result<SchemaDescriptor>;

    /// Derives the state input schema id from the descriptor identity.
    fn input_schema_id() -> Result<SchemaId> {
        Self::input_schema_descriptor()?.schema_id()
    }

    /// Returns canonical named input destinations in ordinal order.
    fn input_destination_paths() -> Result<Vec<mfm_ids::FieldPath>>;
}

/// Descriptor contract for operation output structs.
pub trait OperationOutput: Send + Sync + 'static {
    /// Returns the operation output schema descriptor.
    fn output_schema_descriptor() -> Result<SchemaDescriptor>;

    /// Derives the operation output schema id from the descriptor identity.
    fn output_schema_id() -> Result<SchemaId> {
        Self::output_schema_descriptor()?.schema_id()
    }
}

/// Public launch/render output contract.
pub trait PublicOutputs: PublicOutputDescriptor {}

/// Schema descriptor with hash-defining identity fields split from audit-only
/// provenance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaDescriptor {
    /// Descriptor grammar version.
    pub descriptor_version: u32,
    /// Canonicalization algorithm used for descriptor identity hashing.
    pub canonicalization: DigestAlgorithm,
    /// Hash-defining schema identity.
    pub identity: SchemaIdentity,
    /// Audit-only descriptor provenance.
    pub audit: SchemaAudit,
}

impl SchemaDescriptor {
    /// Creates a v1 schema descriptor.
    pub fn new(identity: SchemaIdentity, audit: SchemaAudit) -> Result<Self> {
        identity.validate()?;
        Ok(Self {
            descriptor_version: 1,
            canonicalization: DigestAlgorithm::Sha256JcsV1,
            identity,
            audit,
        })
    }

    /// Returns canonical JSON bytes for the hash-defining identity only.
    pub fn identity_canonical_json(&self) -> Result<CanonicalJsonBytes> {
        self.identity.canonical_json()
    }

    /// Returns the complete hash-defining schema identity.
    pub const fn identity(&self) -> &SchemaIdentity {
        &self.identity
    }

    /// Derives the schema id from canonical identity bytes.
    pub fn schema_id(&self) -> Result<SchemaId> {
        self.identity.schema_id()
    }
}

/// Hash-defining schema descriptor identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaIdentity {
    /// Schema surface kind.
    pub schema_kind: SchemaKind,
    /// Semantic type identity for value schemas.
    pub semantic_type_id: Option<SemanticTypeId>,
    /// Stable schema name.
    pub schema_name: NameToken,
    /// Manually assigned schema version.
    pub schema_version: SchemaVersion,
    /// Canonical serialized shape.
    pub shape: SchemaShape,
    /// Canonicalization algorithm.
    pub canonicalization: DigestAlgorithm,
    /// Versioning policy.
    pub versioning: SchemaVersioningPolicy,
    /// Persisted-surface no-secret/no-float policy.
    pub persisted_surface: PersistedSurfacePolicy,
}

impl SchemaIdentity {
    /// Creates a schema identity with strict persisted-surface policy.
    pub fn new(
        schema_kind: SchemaKind,
        semantic_type_id: Option<SemanticTypeId>,
        schema_name: impl Into<String>,
        schema_version: SchemaVersion,
        shape: SchemaShape,
    ) -> Result<Self> {
        let raw_schema_name = schema_name.into();
        let schema_name = NameToken::new(&raw_schema_name).map_err(|_| {
            ValueError::Descriptor(format!("invalid schema name {raw_schema_name:?}"))
        })?;
        let identity = Self {
            schema_kind,
            semantic_type_id,
            schema_name,
            schema_version,
            shape,
            canonicalization: DigestAlgorithm::Sha256JcsV1,
            versioning: SchemaVersioningPolicy::ManualVersion,
            persisted_surface: PersistedSurfacePolicy::strict(),
        };
        identity.validate()?;
        Ok(identity)
    }

    /// Strictly decodes exact canonical schema-identity bytes.
    pub fn strict_decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > MAX_SCHEMA_IDENTITY_BYTES {
            return Err(ValueError::InvalidSchemaIdentity);
        }
        let canonical = PlainCanonicalJsonBytes::from_canonical_json_slice(bytes)
            .map_err(|_| ValueError::InvalidSchemaIdentity)?;
        let identity: Self = serde_json::from_slice(canonical.as_bytes())
            .map_err(|_| ValueError::InvalidSchemaIdentity)?;
        identity
            .validate()
            .map_err(|_| ValueError::InvalidSchemaIdentity)?;
        if identity.canonical_json()?.as_bytes() != bytes {
            return Err(ValueError::InvalidSchemaIdentity);
        }
        Ok(identity)
    }

    /// Returns exact canonical JSON for this hash-defining identity.
    pub fn canonical_json(&self) -> Result<CanonicalJsonBytes> {
        self.validate()?;
        Ok(self.canonical_json_unchecked())
    }

    /// Derives the schema id from this complete identity.
    pub fn schema_id(&self) -> Result<SchemaId> {
        let digest = self.canonical_json()?.digest_bytes();
        SchemaId::new(
            self.schema_name.as_str(),
            self.schema_version.as_str(),
            DigestAlgorithm::Sha256JcsV1,
            digest,
        )
        .map_err(|error| ValueError::Identity(error.to_string()))
    }

    /// Verifies exact canonical value bytes against this identity's complete closed shape.
    pub fn validate_canonical_value(&self, bytes: &[u8]) -> Result<()> {
        self.validate()?;
        let canonical = PlainCanonicalJsonBytes::from_canonical_json_slice(bytes)
            .map_err(|_| ValueError::SchemaShapeMismatch)?;
        let value: serde_json::Value = serde_json::from_slice(canonical.as_bytes())
            .map_err(|_| ValueError::SchemaShapeMismatch)?;
        self.shape
            .validate_json_value(&value, 0)
            .map_err(|_| ValueError::SchemaShapeMismatch)
    }

    fn validate(&self) -> Result<()> {
        if self.canonicalization != DigestAlgorithm::Sha256JcsV1
            || self.versioning != SchemaVersioningPolicy::ManualVersion
            || self.persisted_surface != PersistedSurfacePolicy::strict()
        {
            return Err(ValueError::Descriptor(
                "schema identity uses an unsupported fixed policy".to_owned(),
            ));
        }
        match (self.schema_kind, self.semantic_type_id.as_ref()) {
            (SchemaKind::Value, Some(_)) => {}
            (SchemaKind::Value, None) => Err(ValueError::Descriptor(
                "value schema identities must include a semantic type id".to_owned(),
            ))?,
            (
                SchemaKind::PlanningConfig
                | SchemaKind::StateInput
                | SchemaKind::OperationOutput
                | SchemaKind::PublicOutput,
                None,
            ) => {}
            (
                SchemaKind::PlanningConfig
                | SchemaKind::StateInput
                | SchemaKind::OperationOutput
                | SchemaKind::PublicOutput,
                Some(_),
            ) => Err(ValueError::Descriptor(
                "non-value schema identities must not include a semantic type id".to_owned(),
            ))?,
        }
        self.shape.validate_descriptor(0)?;
        if self.canonical_json_unchecked().as_bytes().len() > MAX_SCHEMA_IDENTITY_BYTES {
            return Err(ValueError::Descriptor(
                "schema identity exceeds the canonical byte bound".to_owned(),
            ));
        }
        Ok(())
    }

    fn canonical_json_unchecked(&self) -> CanonicalJsonBytes {
        CanonicalJsonBytes::from_value(&self.to_canonical_value())
    }

    fn to_canonical_value(&self) -> CanonicalValue {
        canonical_object([
            ("canonicalization", string(self.canonicalization.as_str())),
            (
                "persisted_surface",
                self.persisted_surface.to_canonical_value(),
            ),
            ("schema_kind", string(self.schema_kind.as_str())),
            ("schema_name", string(self.schema_name.as_str())),
            ("schema_version", string(self.schema_version.as_str())),
            (
                "semantic_type_id",
                optional_string(self.semantic_type_id.as_ref().map(SemanticTypeId::as_str)),
            ),
            ("shape", self.shape.to_canonical_value()),
            ("versioning", self.versioning.to_canonical_value()),
        ])
    }
}

/// Audit-only descriptor provenance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaAudit {
    owner_crate: String,
    rust_type_path: String,
    provenance: DescriptorProvenance,
    derive_macro_version: Option<String>,
    source_package: Option<String>,
}

impl SchemaAudit {
    /// Creates framework-owned audit provenance for hand-written descriptors.
    pub(crate) fn framework(
        owner_crate: impl Into<String>,
        rust_type_path: impl Into<String>,
    ) -> Self {
        Self {
            owner_crate: owner_crate.into(),
            rust_type_path: rust_type_path.into(),
            provenance: DescriptorProvenance::FrameworkOwned,
            derive_macro_version: None,
            source_package: None,
        }
    }

    /// Creates derive-generated audit provenance from framework macros.
    ///
    /// This is public so proc-macro expansion can reference it from downstream
    /// crates. It is hidden from normal docs and is paired with source-boundary
    /// checks that reject manual persisted trait impls outside framework
    /// allowlists.
    #[doc(hidden)]
    pub fn __derive_generated(
        owner_crate: impl Into<String>,
        rust_type_path: impl Into<String>,
        derive_macro_version: impl Into<String>,
    ) -> Self {
        Self {
            owner_crate: owner_crate.into(),
            rust_type_path: rust_type_path.into(),
            provenance: DescriptorProvenance::DeriveGenerated,
            derive_macro_version: Some(derive_macro_version.into()),
            source_package: None,
        }
    }

    /// Returns the owner crate.
    pub fn owner_crate(&self) -> &str {
        &self.owner_crate
    }

    /// Returns the Rust type path.
    pub fn rust_type_path(&self) -> &str {
        &self.rust_type_path
    }

    /// Returns descriptor implementation provenance.
    pub const fn provenance(&self) -> DescriptorProvenance {
        self.provenance
    }

    /// Returns the derive macro version, when generated by a derive.
    pub fn derive_macro_version(&self) -> Option<&str> {
        self.derive_macro_version.as_deref()
    }

    /// Returns optional source package/build provenance.
    pub fn source_package(&self) -> Option<&str> {
        self.source_package.as_deref()
    }
}

/// Source of a schema descriptor implementation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DescriptorProvenance {
    /// Framework-owned hand-written descriptor.
    FrameworkOwned,
    /// Derive-generated descriptor.
    DeriveGenerated,
}

/// Schema surface kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SchemaKind {
    /// Runtime value crossing a state boundary.
    Value,
    /// Deterministic planning config.
    PlanningConfig,
    /// Typed state input interface.
    StateInput,
    /// Operation output interface.
    OperationOutput,
    /// Public output surface.
    PublicOutput,
}

impl SchemaKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Value => "value",
            Self::PlanningConfig => "planning_config",
            Self::StateInput => "state_input",
            Self::OperationOutput => "operation_output",
            Self::PublicOutput => "public_output",
        }
    }
}

/// Schema versioning policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SchemaVersioningPolicy {
    /// Breaking shape or semantic changes require a manually assigned new
    /// schema version.
    ManualVersion,
}

impl SchemaVersioningPolicy {
    fn to_canonical_value(self) -> CanonicalValue {
        match self {
            Self::ManualVersion => string("manual_version"),
        }
    }
}

/// Persisted-surface policy combining no-secret and no-float requirements.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PersistedSurfacePolicy {
    /// Secret-bearing values are forbidden.
    pub secrets: SecretPolicy,
    /// Floating point values are forbidden.
    pub numbers: NumberPolicy,
}

impl PersistedSurfacePolicy {
    /// Returns the strict v1 persisted-surface policy.
    pub const fn strict() -> Self {
        Self {
            secrets: SecretPolicy::NoSecrets,
            numbers: NumberPolicy::NoFloats,
        }
    }

    fn to_canonical_value(self) -> CanonicalValue {
        canonical_object([
            ("numbers", self.numbers.to_canonical_value()),
            ("secrets", self.secrets.to_canonical_value()),
        ])
    }
}

/// Secret policy for persisted surfaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SecretPolicy {
    /// Secret-bearing fields are forbidden.
    NoSecrets,
}

impl SecretPolicy {
    fn to_canonical_value(self) -> CanonicalValue {
        match self {
            Self::NoSecrets => string("no_secrets"),
        }
    }
}

/// Number policy for persisted surfaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum NumberPolicy {
    /// Floating point values are forbidden.
    NoFloats,
}

impl NumberPolicy {
    fn to_canonical_value(self) -> CanonicalValue {
        match self {
            Self::NoFloats => string("no_floats"),
        }
    }
}

/// Canonical serialized schema shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SchemaShape {
    /// Unit shape.
    Unit,
    /// Boolean shape.
    Bool,
    /// UTF-8 string shape.
    String,
    /// Base64url-without-padding bytes shape.
    Bytes,
    /// Signed integer shape.
    SignedInteger {
        /// Bit width.
        bits: u16,
    },
    /// Unsigned integer shape.
    UnsignedInteger {
        /// Bit width.
        bits: u16,
    },
    /// Decimal-string shape.
    DecimalString {
        /// Decimal scale policy.
        scale: DecimalScale,
    },
    /// Optional value shape.
    Option(Box<SchemaShape>),
    /// Vector shape.
    Vec(Box<SchemaShape>),
    /// Non-empty vector shape.
    NonEmptyVec(Box<SchemaShape>),
    /// Tuple shape.
    Tuple(Vec<SchemaShape>),
    /// Named struct shape.
    Struct {
        /// Struct fields in canonical wire-name order.
        fields: Vec<FieldDescriptor>,
    },
    /// Enum shape.
    Enum {
        /// Enum tagging policy.
        tagging: EnumTagging,
        /// Variants in canonical wire-name order.
        variants: Vec<EnumVariantDescriptor>,
    },
    /// `BTreeMap<String, V>` shape.
    BTreeMapString {
        /// Value shape.
        value: Box<SchemaShape>,
    },
    /// Inline serialized shape of another value schema.
    InlineValue {
        /// Inlined value schema id.
        schema_id: SchemaId,
        /// Inlined value semantic type id.
        semantic_type_id: SemanticTypeId,
        /// Complete serialized shape of the inlined value.
        serialized_shape: Box<SchemaShape>,
    },
    /// Framework-owned generic constructor shape.
    Generic {
        /// Generic constructor name.
        constructor: String,
        /// Generic arguments.
        arguments: Vec<GenericArgumentDescriptor>,
        /// Serialized representation of the constructed value.
        serialized_shape: Box<SchemaShape>,
    },
}

impl SchemaShape {
    /// Builds the complete inline shape of one nested [`MfmValue`].
    pub fn inline_value<T: MfmValue>() -> Result<Self> {
        let descriptor = T::schema_descriptor()?;
        let semantic_type_id = T::semantic_id()?;
        if descriptor.identity.semantic_type_id.as_ref() != Some(&semantic_type_id) {
            return Err(ValueError::Descriptor(
                "nested value descriptor semantic identity does not match its value type"
                    .to_owned(),
            ));
        }
        let schema_id = descriptor.schema_id()?;
        Ok(Self::InlineValue {
            schema_id,
            semantic_type_id,
            serialized_shape: Box::new(descriptor.identity.shape.clone()),
        })
    }

    /// Builds a named struct shape, rejecting duplicate field names.
    pub fn named_struct(mut fields: Vec<FieldDescriptor>) -> Result<Self> {
        reject_duplicate_names(
            fields.iter().map(|field| field.name.as_str()),
            "struct field",
        )?;
        fields.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(Self::Struct { fields })
    }

    /// Builds an enum shape, rejecting duplicate variant names.
    pub fn external_enum(mut variants: Vec<EnumVariantDescriptor>) -> Result<Self> {
        reject_duplicate_names(
            variants.iter().map(|variant| variant.name.as_str()),
            "enum variant",
        )?;
        variants.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(Self::Enum {
            tagging: EnumTagging::External,
            variants,
        })
    }

    /// Builds an enum shape with the supplied tagging policy, rejecting
    /// duplicate variant names.
    pub fn tagged_enum(
        tagging: EnumTagging,
        mut variants: Vec<EnumVariantDescriptor>,
    ) -> Result<Self> {
        reject_duplicate_names(
            variants.iter().map(|variant| variant.name.as_str()),
            "enum variant",
        )?;
        variants.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(Self::Enum { tagging, variants })
    }

    fn validate_descriptor(&self, depth: usize) -> Result<()> {
        if depth > MAX_SCHEMA_DEPTH {
            return Err(ValueError::Descriptor(
                "schema shape exceeds the recursive depth bound".to_owned(),
            ));
        }
        match self {
            Self::Unit | Self::Bool | Self::String | Self::Bytes => Ok(()),
            Self::SignedInteger { bits } | Self::UnsignedInteger { bits } => {
                if matches!(bits, 8 | 16 | 32 | 64) {
                    Ok(())
                } else {
                    Err(ValueError::Descriptor(
                        "integer shape uses an unsupported bit width".to_owned(),
                    ))
                }
            }
            Self::DecimalString { .. } => Ok(()),
            Self::Option(value)
            | Self::Vec(value)
            | Self::NonEmptyVec(value)
            | Self::BTreeMapString { value } => value.validate_descriptor(depth + 1),
            Self::Tuple(values) => {
                for value in values {
                    value.validate_descriptor(depth + 1)?;
                }
                Ok(())
            }
            Self::Struct { fields } => {
                if !fields.windows(2).all(|pair| pair[0].name < pair[1].name)
                    || fields.iter().any(|field| field.name.is_empty())
                {
                    return Err(ValueError::Descriptor(
                        "struct fields must be nonempty, unique, and strictly ordered".to_owned(),
                    ));
                }
                for field in fields {
                    field.shape.validate_descriptor(depth + 1)?;
                }
                Ok(())
            }
            Self::Enum { tagging, variants } => {
                tagging.validate()?;
                if variants.is_empty()
                    || !variants.windows(2).all(|pair| pair[0].name < pair[1].name)
                    || variants.iter().any(|variant| variant.name.is_empty())
                {
                    return Err(ValueError::Descriptor(
                        "enum variants must be nonempty, unique, and strictly ordered".to_owned(),
                    ));
                }
                for variant in variants {
                    match tagging {
                        EnumTagging::Internal { tag } => match &variant.shape {
                            Self::Unit => {}
                            Self::Struct { fields } => {
                                if fields.iter().any(|field| &field.name == tag) {
                                    return Err(ValueError::Descriptor(
                                        "internal enum tag collides with a variant field"
                                            .to_owned(),
                                    ));
                                }
                            }
                            _ => {
                                return Err(ValueError::Descriptor(
                                    "internally tagged variants must be unit or struct shaped"
                                        .to_owned(),
                                ));
                            }
                        },
                        EnumTagging::External | EnumTagging::Adjacent { .. } => {}
                    }
                    variant.shape.validate_descriptor(depth + 1)?;
                }
                Ok(())
            }
            Self::InlineValue {
                serialized_shape, ..
            }
            | Self::Generic {
                serialized_shape, ..
            } => {
                if let Self::Generic {
                    constructor,
                    arguments,
                    ..
                } = self
                {
                    if !valid_generic_constructor(constructor) || arguments.is_empty() {
                        return Err(ValueError::Descriptor(
                            "generic schema metadata is malformed".to_owned(),
                        ));
                    }
                }
                serialized_shape.validate_descriptor(depth + 1)
            }
        }
    }

    fn validate_json_value(&self, value: &serde_json::Value, depth: usize) -> Result<()> {
        if depth > MAX_SCHEMA_DEPTH {
            return Err(ValueError::SchemaShapeMismatch);
        }
        match self {
            Self::Unit => require(value.is_null()),
            Self::Bool => require(value.is_boolean()),
            Self::String => require(
                value
                    .as_str()
                    .is_some_and(|value| !string_contains_secret_marker(value)),
            ),
            Self::Bytes => require(value.as_str().is_some_and(|value| {
                CanonicalBytes::from_base64url_no_pad(value.to_owned()).is_ok()
            })),
            Self::SignedInteger { bits } => {
                let Some(value) = value.as_i64() else {
                    return Err(ValueError::SchemaShapeMismatch);
                };
                require(signed_integer_in_range(value, *bits))
            }
            Self::UnsignedInteger { bits } => {
                let Some(value) = value.as_u64() else {
                    return Err(ValueError::SchemaShapeMismatch);
                };
                require(unsigned_integer_in_range(value, *bits))
            }
            Self::DecimalString { scale } => {
                let Some(value) = value.as_str() else {
                    return Err(ValueError::SchemaShapeMismatch);
                };
                let valid = match scale {
                    DecimalScale::Variable => DecimalString::new_variable(value.to_owned()).is_ok(),
                    DecimalScale::Fixed(scale) => {
                        DecimalString::new_fixed(value.to_owned(), usize::from(*scale)).is_ok()
                    }
                };
                require(valid)
            }
            Self::Option(element) => {
                if value.is_null() {
                    Ok(())
                } else {
                    element.validate_json_value(value, depth + 1)
                }
            }
            Self::Vec(element) | Self::NonEmptyVec(element) => {
                let Some(values) = value.as_array() else {
                    return Err(ValueError::SchemaShapeMismatch);
                };
                if matches!(self, Self::NonEmptyVec(_)) && values.is_empty() {
                    return Err(ValueError::SchemaShapeMismatch);
                }
                for value in values {
                    element.validate_json_value(value, depth + 1)?;
                }
                Ok(())
            }
            Self::Tuple(elements) => {
                let Some(values) = value.as_array() else {
                    return Err(ValueError::SchemaShapeMismatch);
                };
                if values.len() != elements.len() {
                    return Err(ValueError::SchemaShapeMismatch);
                }
                for (element, value) in elements.iter().zip(values) {
                    element.validate_json_value(value, depth + 1)?;
                }
                Ok(())
            }
            Self::Struct { fields } => {
                let Some(object) = value.as_object() else {
                    return Err(ValueError::SchemaShapeMismatch);
                };
                validate_struct_value(fields, object, None, depth + 1)
            }
            Self::Enum { tagging, variants } => {
                validate_enum_value(tagging, variants, value, depth + 1)
            }
            Self::BTreeMapString { value: element } => {
                let Some(object) = value.as_object() else {
                    return Err(ValueError::SchemaShapeMismatch);
                };
                for (key, value) in object {
                    if string_contains_secret_marker(key) {
                        return Err(ValueError::SchemaShapeMismatch);
                    }
                    element.validate_json_value(value, depth + 1)?;
                }
                Ok(())
            }
            Self::InlineValue {
                serialized_shape, ..
            }
            | Self::Generic {
                serialized_shape, ..
            } => serialized_shape.validate_json_value(value, depth + 1),
        }
    }

    fn to_canonical_value(&self) -> CanonicalValue {
        match self {
            Self::Unit => kind_only("unit"),
            Self::Bool => kind_only("bool"),
            Self::String => kind_only("string"),
            Self::Bytes => kind_only("bytes"),
            Self::SignedInteger { bits } => canonical_object([
                ("bits", CanonicalValue::Unsigned(u64::from(*bits))),
                ("kind", string("signed_integer")),
            ]),
            Self::UnsignedInteger { bits } => canonical_object([
                ("bits", CanonicalValue::Unsigned(u64::from(*bits))),
                ("kind", string("unsigned_integer")),
            ]),
            Self::DecimalString { scale } => canonical_object([
                ("kind", string("decimal_string")),
                ("scale", scale.to_canonical_value()),
            ]),
            Self::Option(value) => canonical_object([
                ("element", value.to_canonical_value()),
                ("kind", string("option")),
            ]),
            Self::Vec(value) => canonical_object([
                ("element", value.to_canonical_value()),
                ("kind", string("vec")),
            ]),
            Self::NonEmptyVec(value) => canonical_object([
                ("element", value.to_canonical_value()),
                ("kind", string("non_empty_vec")),
            ]),
            Self::Tuple(values) => canonical_object([
                (
                    "elements",
                    CanonicalValue::Array(values.iter().map(Self::to_canonical_value).collect()),
                ),
                ("kind", string("tuple")),
            ]),
            Self::Struct { fields } => canonical_object([
                (
                    "fields",
                    CanonicalValue::Array(
                        fields
                            .iter()
                            .map(FieldDescriptor::to_canonical_value)
                            .collect(),
                    ),
                ),
                ("kind", string("struct")),
            ]),
            Self::Enum { tagging, variants } => canonical_object([
                ("kind", string("enum")),
                ("tagging", tagging.to_canonical_value()),
                (
                    "variants",
                    CanonicalValue::Array(
                        variants
                            .iter()
                            .map(EnumVariantDescriptor::to_canonical_value)
                            .collect(),
                    ),
                ),
            ]),
            Self::BTreeMapString { value } => canonical_object([
                ("key", string("string")),
                ("kind", string("btree_map")),
                ("value", value.to_canonical_value()),
            ]),
            Self::InlineValue {
                schema_id,
                semantic_type_id,
                serialized_shape,
            } => canonical_object([
                ("kind", string("inline_value")),
                ("schema_id", string(schema_id.as_str())),
                ("semantic_type_id", string(semantic_type_id.as_str())),
                ("serialized_shape", serialized_shape.to_canonical_value()),
            ]),
            Self::Generic {
                constructor,
                arguments,
                serialized_shape,
            } => canonical_object([
                (
                    "arguments",
                    CanonicalValue::Array(
                        arguments
                            .iter()
                            .map(GenericArgumentDescriptor::to_canonical_value)
                            .collect(),
                    ),
                ),
                ("constructor", string(constructor)),
                ("kind", string("generic")),
                ("serialized_shape", serialized_shape.to_canonical_value()),
            ]),
        }
    }
}

/// Decimal scale descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DecimalScale {
    /// Variable scale with canonical decimal-string grammar.
    Variable,
    /// Fixed scale.
    Fixed(u16),
}

impl DecimalScale {
    fn to_canonical_value(self) -> CanonicalValue {
        match self {
            Self::Variable => canonical_object([("kind", string("variable"))]),
            Self::Fixed(scale) => canonical_object([
                ("kind", string("fixed")),
                ("scale", CanonicalValue::Unsigned(u64::from(scale))),
            ]),
        }
    }
}

/// Field descriptor for named structs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldDescriptor {
    /// Canonical wire name.
    pub name: String,
    /// Field shape.
    pub shape: SchemaShape,
    /// Defaulting policy.
    pub default: FieldDefaultPolicy,
}

impl FieldDescriptor {
    /// Creates a required field descriptor.
    pub fn required(name: impl Into<String>, shape: SchemaShape) -> Self {
        Self {
            name: name.into(),
            shape,
            default: FieldDefaultPolicy::Required,
        }
    }

    /// Creates a field descriptor with framework-recognized default behavior.
    pub fn with_default(name: impl Into<String>, shape: SchemaShape) -> Self {
        Self {
            name: name.into(),
            shape,
            default: FieldDefaultPolicy::MfmDefault,
        }
    }

    fn to_canonical_value(&self) -> CanonicalValue {
        canonical_object([
            ("default", self.default.to_canonical_value()),
            ("name", string(&self.name)),
            ("shape", self.shape.to_canonical_value()),
        ])
    }
}

/// Field defaulting policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FieldDefaultPolicy {
    /// Field must be present.
    Required,
    /// Field may use `MfmDefault`.
    MfmDefault,
}

impl FieldDefaultPolicy {
    fn to_canonical_value(self) -> CanonicalValue {
        match self {
            Self::Required => string("required"),
            Self::MfmDefault => string("mfm_default"),
        }
    }
}

/// Enum variant descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnumVariantDescriptor {
    /// Canonical variant wire name.
    pub name: String,
    /// Variant payload shape.
    pub shape: SchemaShape,
}

impl EnumVariantDescriptor {
    /// Creates an enum variant descriptor.
    pub fn new(name: impl Into<String>, shape: SchemaShape) -> Self {
        Self {
            name: name.into(),
            shape,
        }
    }

    fn to_canonical_value(&self) -> CanonicalValue {
        canonical_object([
            ("name", string(&self.name)),
            ("shape", self.shape.to_canonical_value()),
        ])
    }
}

/// Supported enum tagging policy.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EnumTagging {
    /// Externally tagged enum.
    External,
    /// Internally tagged enum for struct-like variants.
    Internal {
        /// Tag field name.
        tag: String,
    },
    /// Adjacently tagged enum.
    Adjacent {
        /// Tag field name.
        tag: String,
        /// Content field name.
        content: String,
    },
}

impl EnumTagging {
    fn validate(&self) -> Result<()> {
        match self {
            Self::External => Ok(()),
            Self::Internal { tag } => {
                if valid_wire_name(tag) {
                    Ok(())
                } else {
                    Err(ValueError::Descriptor(
                        "internal enum tag is malformed".to_owned(),
                    ))
                }
            }
            Self::Adjacent { tag, content } => {
                if valid_wire_name(tag) && valid_wire_name(content) && tag != content {
                    Ok(())
                } else {
                    Err(ValueError::Descriptor(
                        "adjacent enum tag/content metadata is malformed".to_owned(),
                    ))
                }
            }
        }
    }

    fn to_canonical_value(&self) -> CanonicalValue {
        match self {
            Self::External => canonical_object([("kind", string("external"))]),
            Self::Internal { tag } => {
                canonical_object([("kind", string("internal")), ("tag", string(tag))])
            }
            Self::Adjacent { tag, content } => canonical_object([
                ("content", string(content)),
                ("kind", string("adjacent")),
                ("tag", string(tag)),
            ]),
        }
    }
}

fn validate_struct_value(
    fields: &[FieldDescriptor],
    object: &serde_json::Map<String, serde_json::Value>,
    extra_field: Option<&str>,
    depth: usize,
) -> Result<()> {
    if object
        .keys()
        .any(|name| extra_field != Some(name.as_str()) && !fields.iter().any(|f| f.name == *name))
    {
        return Err(ValueError::SchemaShapeMismatch);
    }
    for field in fields {
        match object.get(&field.name) {
            Some(value) => field.shape.validate_json_value(value, depth)?,
            None if field.default == FieldDefaultPolicy::MfmDefault => {}
            None => return Err(ValueError::SchemaShapeMismatch),
        }
    }
    Ok(())
}

fn validate_enum_value(
    tagging: &EnumTagging,
    variants: &[EnumVariantDescriptor],
    value: &serde_json::Value,
    depth: usize,
) -> Result<()> {
    match tagging {
        EnumTagging::External => match value {
            serde_json::Value::String(name) => {
                let variant = find_enum_variant(variants, name)?;
                require(matches!(variant.shape, SchemaShape::Unit))
            }
            serde_json::Value::Object(object) if object.len() == 1 => {
                let (name, payload) = object
                    .iter()
                    .next()
                    .ok_or(ValueError::SchemaShapeMismatch)?;
                let variant = find_enum_variant(variants, name)?;
                if matches!(variant.shape, SchemaShape::Unit) {
                    return Err(ValueError::SchemaShapeMismatch);
                }
                validate_enum_payload(&variant.shape, payload, depth)
            }
            _ => Err(ValueError::SchemaShapeMismatch),
        },
        EnumTagging::Internal { tag } => {
            let object = value.as_object().ok_or(ValueError::SchemaShapeMismatch)?;
            let name = object
                .get(tag)
                .and_then(serde_json::Value::as_str)
                .ok_or(ValueError::SchemaShapeMismatch)?;
            let variant = find_enum_variant(variants, name)?;
            match &variant.shape {
                SchemaShape::Unit => require(object.len() == 1),
                SchemaShape::Struct { fields } => {
                    validate_struct_value(fields, object, Some(tag), depth)
                }
                _ => Err(ValueError::SchemaShapeMismatch),
            }
        }
        EnumTagging::Adjacent { tag, content } => {
            let object = value.as_object().ok_or(ValueError::SchemaShapeMismatch)?;
            let name = object
                .get(tag)
                .and_then(serde_json::Value::as_str)
                .ok_or(ValueError::SchemaShapeMismatch)?;
            let variant = find_enum_variant(variants, name)?;
            if matches!(variant.shape, SchemaShape::Unit) {
                return require(object.len() == 1 && !object.contains_key(content));
            }
            if object.len() != 2 {
                return Err(ValueError::SchemaShapeMismatch);
            }
            let payload = object.get(content).ok_or(ValueError::SchemaShapeMismatch)?;
            validate_enum_payload(&variant.shape, payload, depth)
        }
    }
}

fn validate_enum_payload(
    shape: &SchemaShape,
    payload: &serde_json::Value,
    depth: usize,
) -> Result<()> {
    match shape {
        SchemaShape::Tuple(elements) if elements.len() == 1 => {
            elements[0].validate_json_value(payload, depth)
        }
        _ => shape.validate_json_value(payload, depth),
    }
}

fn find_enum_variant<'a>(
    variants: &'a [EnumVariantDescriptor],
    name: &str,
) -> Result<&'a EnumVariantDescriptor> {
    variants
        .binary_search_by(|variant| variant.name.as_str().cmp(name))
        .ok()
        .map(|index| &variants[index])
        .ok_or(ValueError::SchemaShapeMismatch)
}

fn signed_integer_in_range(value: i64, bits: u16) -> bool {
    match bits {
        8 => i8::try_from(value).is_ok(),
        16 => i16::try_from(value).is_ok(),
        32 => i32::try_from(value).is_ok(),
        64 => true,
        _ => false,
    }
}

fn unsigned_integer_in_range(value: u64, bits: u16) -> bool {
    match bits {
        8 => u8::try_from(value).is_ok(),
        16 => u16::try_from(value).is_ok(),
        32 => u32::try_from(value).is_ok(),
        64 => true,
        _ => false,
    }
}

fn require(condition: bool) -> Result<()> {
    if condition {
        Ok(())
    } else {
        Err(ValueError::SchemaShapeMismatch)
    }
}

fn valid_wire_name(value: &str) -> bool {
    !value.is_empty() && value.len() <= 256 && !value.chars().any(char::is_control)
}

fn valid_generic_constructor(value: &str) -> bool {
    if value.is_empty() || value.len() > 256 {
        return false;
    }
    let mut segments = value.split('/');
    let Some(namespace) = segments.next() else {
        return false;
    };
    let Some(name) = segments.next() else {
        return false;
    };
    segments.next().is_none()
        && !namespace.is_empty()
        && !name.is_empty()
        && namespace.chars().chain(name.chars()).all(|ch| {
            ch.is_ascii_lowercase() || ch.is_ascii_digit() || matches!(ch, '.' | '_' | '-')
        })
}

/// Generic schema argument identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenericArgumentDescriptor {
    /// Referenced schema id.
    pub schema_id: SchemaId,
    /// Referenced semantic type id.
    pub semantic_type_id: SemanticTypeId,
}

impl GenericArgumentDescriptor {
    /// Creates a generic argument descriptor for an `MfmValue`.
    pub fn for_value<T: MfmValue>() -> Result<Self> {
        Ok(Self {
            schema_id: T::schema_id()?,
            semantic_type_id: T::semantic_id()?,
        })
    }

    fn to_canonical_value(&self) -> CanonicalValue {
        canonical_object([
            ("schema_id", string(self.schema_id.as_str())),
            ("semantic_type_id", string(self.semantic_type_id.as_str())),
        ])
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SchemaIdentityWire {
    canonicalization: String,
    persisted_surface: PersistedSurfaceWire,
    schema_kind: String,
    schema_name: String,
    schema_version: String,
    semantic_type_id: Option<String>,
    shape: SchemaShapeWire,
    versioning: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedSurfaceWire {
    numbers: String,
    secrets: String,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum SchemaShapeWire {
    Unit,
    Bool,
    String,
    Bytes,
    SignedInteger {
        bits: u16,
    },
    UnsignedInteger {
        bits: u16,
    },
    DecimalString {
        scale: DecimalScaleWire,
    },
    Option {
        element: Box<SchemaShapeWire>,
    },
    Vec {
        element: Box<SchemaShapeWire>,
    },
    NonEmptyVec {
        element: Box<SchemaShapeWire>,
    },
    Tuple {
        elements: Vec<SchemaShapeWire>,
    },
    Struct {
        fields: Vec<FieldDescriptorWire>,
    },
    Enum {
        tagging: EnumTaggingWire,
        variants: Vec<EnumVariantDescriptorWire>,
    },
    #[serde(rename = "btree_map")]
    BTreeMap {
        key: String,
        value: Box<SchemaShapeWire>,
    },
    InlineValue {
        schema_id: String,
        semantic_type_id: String,
        serialized_shape: Box<SchemaShapeWire>,
    },
    Generic {
        arguments: Vec<GenericArgumentDescriptorWire>,
        constructor: String,
        serialized_shape: Box<SchemaShapeWire>,
    },
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum DecimalScaleWire {
    Variable,
    Fixed { scale: u16 },
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FieldDescriptorWire {
    default: String,
    name: String,
    shape: SchemaShapeWire,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct EnumVariantDescriptorWire {
    name: String,
    shape: SchemaShapeWire,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum EnumTaggingWire {
    External,
    Internal { tag: String },
    Adjacent { content: String, tag: String },
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct GenericArgumentDescriptorWire {
    schema_id: String,
    semantic_type_id: String,
}

impl Serialize for SchemaIdentity {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.validate().map_err(serde::ser::Error::custom)?;
        SchemaIdentityWire::from(self).serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for SchemaIdentity {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = SchemaIdentityWire::deserialize(deserializer)?;
        Self::try_from(wire).map_err(serde::de::Error::custom)
    }
}

impl From<&SchemaIdentity> for SchemaIdentityWire {
    fn from(identity: &SchemaIdentity) -> Self {
        Self {
            canonicalization: identity.canonicalization.as_str().to_owned(),
            persisted_surface: PersistedSurfaceWire {
                numbers: match identity.persisted_surface.numbers {
                    NumberPolicy::NoFloats => "no_floats".to_owned(),
                },
                secrets: match identity.persisted_surface.secrets {
                    SecretPolicy::NoSecrets => "no_secrets".to_owned(),
                },
            },
            schema_kind: identity.schema_kind.as_str().to_owned(),
            schema_name: identity.schema_name.as_str().to_owned(),
            schema_version: identity.schema_version.as_str().to_owned(),
            semantic_type_id: identity.semantic_type_id.as_ref().map(ToString::to_string),
            shape: SchemaShapeWire::from(&identity.shape),
            versioning: match identity.versioning {
                SchemaVersioningPolicy::ManualVersion => "manual_version".to_owned(),
            },
        }
    }
}

impl TryFrom<SchemaIdentityWire> for SchemaIdentity {
    type Error = ValueError;

    fn try_from(wire: SchemaIdentityWire) -> Result<Self> {
        if wire.canonicalization != DigestAlgorithm::Sha256JcsV1.as_str()
            || wire.versioning != "manual_version"
            || wire.persisted_surface.numbers != "no_floats"
            || wire.persisted_surface.secrets != "no_secrets"
        {
            return Err(ValueError::InvalidSchemaIdentity);
        }
        let schema_kind = match wire.schema_kind.as_str() {
            "value" => SchemaKind::Value,
            "planning_config" => SchemaKind::PlanningConfig,
            "state_input" => SchemaKind::StateInput,
            "operation_output" => SchemaKind::OperationOutput,
            "public_output" => SchemaKind::PublicOutput,
            _ => return Err(ValueError::InvalidSchemaIdentity),
        };
        let semantic_type_id = wire
            .semantic_type_id
            .map(|value| value.parse().map_err(|_| ValueError::InvalidSchemaIdentity))
            .transpose()?;
        let identity = Self {
            schema_kind,
            semantic_type_id,
            schema_name: NameToken::new(&wire.schema_name)
                .map_err(|_| ValueError::InvalidSchemaIdentity)?,
            schema_version: SchemaVersion::new(&wire.schema_version)
                .map_err(|_| ValueError::InvalidSchemaIdentity)?,
            shape: SchemaShape::try_from(wire.shape)?,
            canonicalization: DigestAlgorithm::Sha256JcsV1,
            versioning: SchemaVersioningPolicy::ManualVersion,
            persisted_surface: PersistedSurfacePolicy::strict(),
        };
        identity
            .validate()
            .map_err(|_| ValueError::InvalidSchemaIdentity)?;
        Ok(identity)
    }
}

impl From<&SchemaShape> for SchemaShapeWire {
    fn from(shape: &SchemaShape) -> Self {
        match shape {
            SchemaShape::Unit => Self::Unit,
            SchemaShape::Bool => Self::Bool,
            SchemaShape::String => Self::String,
            SchemaShape::Bytes => Self::Bytes,
            SchemaShape::SignedInteger { bits } => Self::SignedInteger { bits: *bits },
            SchemaShape::UnsignedInteger { bits } => Self::UnsignedInteger { bits: *bits },
            SchemaShape::DecimalString { scale } => Self::DecimalString {
                scale: DecimalScaleWire::from(*scale),
            },
            SchemaShape::Option(element) => Self::Option {
                element: Box::new(Self::from(element.as_ref())),
            },
            SchemaShape::Vec(element) => Self::Vec {
                element: Box::new(Self::from(element.as_ref())),
            },
            SchemaShape::NonEmptyVec(element) => Self::NonEmptyVec {
                element: Box::new(Self::from(element.as_ref())),
            },
            SchemaShape::Tuple(elements) => Self::Tuple {
                elements: elements.iter().map(Self::from).collect(),
            },
            SchemaShape::Struct { fields } => Self::Struct {
                fields: fields.iter().map(FieldDescriptorWire::from).collect(),
            },
            SchemaShape::Enum { tagging, variants } => Self::Enum {
                tagging: EnumTaggingWire::from(tagging),
                variants: variants
                    .iter()
                    .map(EnumVariantDescriptorWire::from)
                    .collect(),
            },
            SchemaShape::BTreeMapString { value } => Self::BTreeMap {
                key: "string".to_owned(),
                value: Box::new(Self::from(value.as_ref())),
            },
            SchemaShape::InlineValue {
                schema_id,
                semantic_type_id,
                serialized_shape,
            } => Self::InlineValue {
                schema_id: schema_id.to_string(),
                semantic_type_id: semantic_type_id.to_string(),
                serialized_shape: Box::new(Self::from(serialized_shape.as_ref())),
            },
            SchemaShape::Generic {
                constructor,
                arguments,
                serialized_shape,
            } => Self::Generic {
                arguments: arguments
                    .iter()
                    .map(GenericArgumentDescriptorWire::from)
                    .collect(),
                constructor: constructor.clone(),
                serialized_shape: Box::new(Self::from(serialized_shape.as_ref())),
            },
        }
    }
}

impl TryFrom<SchemaShapeWire> for SchemaShape {
    type Error = ValueError;

    fn try_from(wire: SchemaShapeWire) -> Result<Self> {
        Ok(match wire {
            SchemaShapeWire::Unit => Self::Unit,
            SchemaShapeWire::Bool => Self::Bool,
            SchemaShapeWire::String => Self::String,
            SchemaShapeWire::Bytes => Self::Bytes,
            SchemaShapeWire::SignedInteger { bits } => Self::SignedInteger { bits },
            SchemaShapeWire::UnsignedInteger { bits } => Self::UnsignedInteger { bits },
            SchemaShapeWire::DecimalString { scale } => Self::DecimalString {
                scale: DecimalScale::from(scale),
            },
            SchemaShapeWire::Option { element } => {
                Self::Option(Box::new(Self::try_from(*element)?))
            }
            SchemaShapeWire::Vec { element } => Self::Vec(Box::new(Self::try_from(*element)?)),
            SchemaShapeWire::NonEmptyVec { element } => {
                Self::NonEmptyVec(Box::new(Self::try_from(*element)?))
            }
            SchemaShapeWire::Tuple { elements } => Self::Tuple(
                elements
                    .into_iter()
                    .map(Self::try_from)
                    .collect::<Result<_>>()?,
            ),
            SchemaShapeWire::Struct { fields } => Self::Struct {
                fields: fields
                    .into_iter()
                    .map(FieldDescriptor::try_from)
                    .collect::<Result<_>>()?,
            },
            SchemaShapeWire::Enum { tagging, variants } => Self::Enum {
                tagging: EnumTagging::from(tagging),
                variants: variants
                    .into_iter()
                    .map(EnumVariantDescriptor::try_from)
                    .collect::<Result<_>>()?,
            },
            SchemaShapeWire::BTreeMap { key, value } => {
                if key != "string" {
                    return Err(ValueError::InvalidSchemaIdentity);
                }
                Self::BTreeMapString {
                    value: Box::new(Self::try_from(*value)?),
                }
            }
            SchemaShapeWire::InlineValue {
                schema_id,
                semantic_type_id,
                serialized_shape,
            } => Self::InlineValue {
                schema_id: schema_id
                    .parse()
                    .map_err(|_| ValueError::InvalidSchemaIdentity)?,
                semantic_type_id: semantic_type_id
                    .parse()
                    .map_err(|_| ValueError::InvalidSchemaIdentity)?,
                serialized_shape: Box::new(Self::try_from(*serialized_shape)?),
            },
            SchemaShapeWire::Generic {
                arguments,
                constructor,
                serialized_shape,
            } => Self::Generic {
                constructor,
                arguments: arguments
                    .into_iter()
                    .map(GenericArgumentDescriptor::try_from)
                    .collect::<Result<_>>()?,
                serialized_shape: Box::new(Self::try_from(*serialized_shape)?),
            },
        })
    }
}

impl From<DecimalScale> for DecimalScaleWire {
    fn from(scale: DecimalScale) -> Self {
        match scale {
            DecimalScale::Variable => Self::Variable,
            DecimalScale::Fixed(scale) => Self::Fixed { scale },
        }
    }
}

impl From<DecimalScaleWire> for DecimalScale {
    fn from(scale: DecimalScaleWire) -> Self {
        match scale {
            DecimalScaleWire::Variable => Self::Variable,
            DecimalScaleWire::Fixed { scale } => Self::Fixed(scale),
        }
    }
}

impl From<&FieldDescriptor> for FieldDescriptorWire {
    fn from(field: &FieldDescriptor) -> Self {
        Self {
            default: match field.default {
                FieldDefaultPolicy::Required => "required".to_owned(),
                FieldDefaultPolicy::MfmDefault => "mfm_default".to_owned(),
            },
            name: field.name.clone(),
            shape: SchemaShapeWire::from(&field.shape),
        }
    }
}

impl TryFrom<FieldDescriptorWire> for FieldDescriptor {
    type Error = ValueError;

    fn try_from(field: FieldDescriptorWire) -> Result<Self> {
        let default = match field.default.as_str() {
            "required" => FieldDefaultPolicy::Required,
            "mfm_default" => FieldDefaultPolicy::MfmDefault,
            _ => return Err(ValueError::InvalidSchemaIdentity),
        };
        Ok(Self {
            name: field.name,
            shape: SchemaShape::try_from(field.shape)?,
            default,
        })
    }
}

impl From<&EnumVariantDescriptor> for EnumVariantDescriptorWire {
    fn from(variant: &EnumVariantDescriptor) -> Self {
        Self {
            name: variant.name.clone(),
            shape: SchemaShapeWire::from(&variant.shape),
        }
    }
}

impl TryFrom<EnumVariantDescriptorWire> for EnumVariantDescriptor {
    type Error = ValueError;

    fn try_from(variant: EnumVariantDescriptorWire) -> Result<Self> {
        Ok(Self {
            name: variant.name,
            shape: SchemaShape::try_from(variant.shape)?,
        })
    }
}

impl From<&EnumTagging> for EnumTaggingWire {
    fn from(tagging: &EnumTagging) -> Self {
        match tagging {
            EnumTagging::External => Self::External,
            EnumTagging::Internal { tag } => Self::Internal { tag: tag.clone() },
            EnumTagging::Adjacent { tag, content } => Self::Adjacent {
                content: content.clone(),
                tag: tag.clone(),
            },
        }
    }
}

impl From<EnumTaggingWire> for EnumTagging {
    fn from(tagging: EnumTaggingWire) -> Self {
        match tagging {
            EnumTaggingWire::External => Self::External,
            EnumTaggingWire::Internal { tag } => Self::Internal { tag },
            EnumTaggingWire::Adjacent { tag, content } => Self::Adjacent { tag, content },
        }
    }
}

impl From<&GenericArgumentDescriptor> for GenericArgumentDescriptorWire {
    fn from(argument: &GenericArgumentDescriptor) -> Self {
        Self {
            schema_id: argument.schema_id.to_string(),
            semantic_type_id: argument.semantic_type_id.to_string(),
        }
    }
}

impl TryFrom<GenericArgumentDescriptorWire> for GenericArgumentDescriptor {
    type Error = ValueError;

    fn try_from(argument: GenericArgumentDescriptorWire) -> Result<Self> {
        Ok(Self {
            schema_id: argument
                .schema_id
                .parse()
                .map_err(|_| ValueError::InvalidSchemaIdentity)?,
            semantic_type_id: argument
                .semantic_type_id
                .parse()
                .map_err(|_| ValueError::InvalidSchemaIdentity)?,
        })
    }
}

/// Marker trait for framework-approved default values.
///
/// Implementations are intentionally not blanket-derived from [`Default`]:
/// derives and framework-owned wrappers must opt in explicitly so secret or
/// unsupported persisted shapes cannot become defaultable by accident.
pub trait MfmDefault: Default {}

impl<T> MfmDefault for Option<T> {}

impl<T> MfmDefault for Vec<T> {}

impl<V> MfmDefault for BTreeMap<String, V> {}

/// Builds a descriptor for a framework-owned, hand-written value contract.
///
/// Domain crates must use the derive path. This hidden helper exists only for
/// typed kernel contracts that cannot use derives because their checked fields
/// intentionally do not implement generic serde traits.
#[doc(hidden)]
pub fn framework_value_descriptor(
    semantic_type_id: SemanticTypeId,
    schema_name: &str,
    shape: SchemaShape,
    rust_type_path: &str,
) -> Result<SchemaDescriptor> {
    SchemaDescriptor::new(
        SchemaIdentity::new(
            SchemaKind::Value,
            Some(semantic_type_id),
            schema_name,
            schema_version("1")?,
            shape,
        )?,
        SchemaAudit::framework("mfm-values", rust_type_path),
    )
}

fn schema_version(value: &str) -> Result<SchemaVersion> {
    SchemaVersion::new(value).map_err(|error| ValueError::Identity(error.to_string()))
}

fn reject_duplicate_names<'a>(
    names: impl IntoIterator<Item = &'a str>,
    label: &'static str,
) -> Result<()> {
    let mut sorted = Vec::new();
    for name in names {
        if name.is_empty() {
            return Err(ValueError::Descriptor(format!(
                "{label} name must not be empty"
            )));
        }
        sorted.push(name);
    }
    sorted.sort_unstable();
    for pair in sorted.windows(2) {
        if pair[0] == pair[1] {
            return Err(ValueError::Descriptor(format!(
                "duplicate {label} name '{}'",
                pair[0]
            )));
        }
    }
    Ok(())
}

fn kind_only(kind: &'static str) -> CanonicalValue {
    canonical_object([("kind", string(kind))])
}

fn string(value: &str) -> CanonicalValue {
    CanonicalValue::String(value.to_owned())
}

fn optional_string(value: Option<&str>) -> CanonicalValue {
    value.map_or(CanonicalValue::Null, string)
}

fn canonical_object<const N: usize>(
    entries: [(&'static str, CanonicalValue); N],
) -> CanonicalValue {
    CanonicalValue::object(entries).expect("static descriptor object keys are unique")
}
