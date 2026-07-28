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

use mfm_canonical::{CanonicalJsonBytes, CanonicalValue};
use mfm_ids::{
    ContextRef, ContextResourceKind, ContextStage, DigestAlgorithm, NameToken, SchemaId,
    SchemaVersion, SemanticTypeId,
};
use serde::de::DeserializeOwned;
use serde::Serialize;

mod generic_values;
pub use self::generic_values::{
    ArtifactRef, ContextRefValue, MaybeValue, NonEmpty, SkipCode, SkipReason,
};
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

/// Terminal policy for cells containing a value type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueTerminalPolicy {
    /// The cell must be produced with value bytes.
    ProducedOnly,
    /// The cell may be produced or skipped with explicit skip provenance.
    MaybeSkipped,
}

/// Values that may cross typed state boundaries.
pub trait MfmValue: Serialize + DeserializeOwned + Send + Sync + 'static {
    /// Returns the schema descriptor for this value type.
    fn schema_descriptor() -> Result<SchemaDescriptor>;

    /// Returns the stable semantic identity for this value kind.
    fn semantic_id() -> Result<SemanticTypeId>;

    /// Returns the terminal policy for cells containing this value type.
    fn terminal_policy() -> ValueTerminalPolicy {
        ValueTerminalPolicy::ProducedOnly
    }

    /// Derives this value's schema id from its schema descriptor identity.
    fn schema_id() -> Result<SchemaId> {
        Self::schema_descriptor()?.schema_id()
    }
}

/// Typed state output that carries certified context-resource metadata.
pub trait ContextBoundOutput: MfmValue {
    /// Returns the context ref embedded in the output value.
    fn context_ref(&self) -> &ContextRef;

    /// Returns the context resource kind embedded in the output value.
    fn context_resource_kind(&self) -> &ContextResourceKind;

    /// Returns the context resource stage embedded in the output value.
    fn context_stage(&self) -> &ContextStage;
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
}

impl StateInput for () {
    fn input_schema_descriptor() -> Result<SchemaDescriptor> {
        framework_input_descriptor("mfm.kernel.state_input.unit", SchemaShape::Unit, "()")
    }
}

impl<T: MfmValue> StateInput for T {
    fn input_schema_descriptor() -> Result<SchemaDescriptor> {
        framework_input_descriptor(
            "mfm.kernel.state_input.value",
            SchemaShape::ValueRef {
                schema_id: T::schema_id()?,
                semantic_type_id: T::semantic_id()?,
            },
            "mfm_values::MfmValue",
        )
    }
}

impl<T: MfmValue> StateInput for Vec<T> {
    fn input_schema_descriptor() -> Result<SchemaDescriptor> {
        framework_input_descriptor(
            "mfm.kernel.state_input.vec",
            SchemaShape::Vec(Box::new(SchemaShape::ValueRef {
                schema_id: T::schema_id()?,
                semantic_type_id: T::semantic_id()?,
            })),
            "alloc::vec::Vec",
        )
    }
}

macro_rules! impl_tuple_state_input {
    ($($name:ident),+ $(,)?) => {
        impl<$($name),+> StateInput for ($($name,)+)
        where
            $($name: MfmValue,)+
        {
            fn input_schema_descriptor() -> Result<SchemaDescriptor> {
                framework_input_descriptor(
                    "mfm.kernel.state_input.tuple",
                    SchemaShape::Tuple(vec![
                        $(SchemaShape::ValueRef {
                            schema_id: $name::schema_id()?,
                            semantic_type_id: $name::semantic_id()?,
                        },)+
                    ]),
                    "tuple",
                )
            }
        }
    };
}

impl_tuple_state_input!(A);
impl_tuple_state_input!(A, B);
impl_tuple_state_input!(A, B, C);
impl_tuple_state_input!(A, B, C, D);
impl_tuple_state_input!(A, B, C, D, E);
impl_tuple_state_input!(A, B, C, D, E, F);
impl_tuple_state_input!(A, B, C, D, E, F, G);
impl_tuple_state_input!(A, B, C, D, E, F, G, H);
impl_tuple_state_input!(A, B, C, D, E, F, G, H, I);
impl_tuple_state_input!(A, B, C, D, E, F, G, H, I, J);
impl_tuple_state_input!(A, B, C, D, E, F, G, H, I, J, K);
impl_tuple_state_input!(A, B, C, D, E, F, G, H, I, J, K, L);

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
    pub fn identity_canonical_json(&self) -> CanonicalJsonBytes {
        CanonicalJsonBytes::from_value(&self.identity.to_canonical_value())
    }

    /// Derives the schema id from canonical identity bytes.
    pub fn schema_id(&self) -> Result<SchemaId> {
        self.identity.validate()?;
        let digest = self.identity_canonical_json().digest_bytes();
        SchemaId::new(
            self.identity.schema_name.as_str(),
            self.identity.schema_version.as_str(),
            DigestAlgorithm::Sha256JcsV1,
            digest,
        )
        .map_err(|error| ValueError::Identity(error.to_string()))
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

    fn validate(&self) -> Result<()> {
        match (self.schema_kind, self.semantic_type_id.as_ref()) {
            (SchemaKind::Value, Some(_)) => Ok(()),
            (SchemaKind::Value, None) => Err(ValueError::Descriptor(
                "value schema identities must include a semantic type id".to_owned(),
            )),
            (
                SchemaKind::PlanningConfig
                | SchemaKind::StateInput
                | SchemaKind::OperationOutput
                | SchemaKind::PublicOutput,
                None,
            ) => Ok(()),
            (
                SchemaKind::PlanningConfig
                | SchemaKind::StateInput
                | SchemaKind::OperationOutput
                | SchemaKind::PublicOutput,
                Some(_),
            ) => Err(ValueError::Descriptor(
                "non-value schema identities must not include a semantic type id".to_owned(),
            )),
        }
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
    /// Reference to another value schema.
    ValueRef {
        /// Referenced schema id.
        schema_id: SchemaId,
        /// Referenced semantic type id.
        semantic_type_id: SemanticTypeId,
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
            Self::ValueRef {
                schema_id,
                semantic_type_id,
            } => canonical_object([
                ("kind", string("value_ref")),
                ("schema_id", string(schema_id.as_str())),
                ("semantic_type_id", string(semantic_type_id.as_str())),
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EnumTagging {
    /// Externally tagged enum.
    External,
    /// Internally tagged enum for struct-like variants.
    Internal {
        /// Tag field name.
        tag: &'static str,
    },
    /// Adjacently tagged enum.
    Adjacent {
        /// Tag field name.
        tag: &'static str,
        /// Content field name.
        content: &'static str,
    },
}

impl EnumTagging {
    fn to_canonical_value(self) -> CanonicalValue {
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

fn framework_input_descriptor(
    schema_name: &str,
    shape: SchemaShape,
    rust_type_path: &str,
) -> Result<SchemaDescriptor> {
    SchemaDescriptor::new(
        SchemaIdentity::new(
            SchemaKind::StateInput,
            None,
            schema_name,
            schema_version("1")?,
            shape,
        )?,
        SchemaAudit::framework("mfm-values", rust_type_path),
    )
}

fn skip_reason_shape() -> Result<SchemaShape> {
    SchemaShape::named_struct(vec![
        FieldDescriptor::required(
            "code",
            SchemaShape::external_enum(vec![
                EnumVariantDescriptor::new("dependency_skipped", SchemaShape::Unit),
                EnumVariantDescriptor::new("filtered", SchemaShape::Unit),
                EnumVariantDescriptor::new("not_applicable", SchemaShape::Unit),
                EnumVariantDescriptor::new("policy", SchemaShape::Unit),
            ])?,
        ),
        FieldDescriptor::required("explanation", SchemaShape::String),
    ])
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
