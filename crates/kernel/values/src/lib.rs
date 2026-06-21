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
use std::marker::PhantomData;

use mfm_canonical::{CanonicalJsonBytes, CanonicalValue};
use mfm_ids::{
    ArtifactId, ContentDigest, DigestAlgorithm, DigestBytes, SchemaId, SchemaVersion,
    SemanticTypeId,
};
use serde::de::{self, DeserializeOwned, Deserializer};
use serde::ser::{SerializeStruct, Serializer};
use serde::{Deserialize, Serialize};

#[cfg(test)]
mod tests;

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
            &self.identity.schema_name,
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
    pub schema_name: String,
    /// Manually assigned schema version.
    pub schema_version: SchemaVersion,
    /// Canonical serialized shape.
    pub shape: SchemaShape,
    /// Canonicalization algorithm.
    pub canonicalization: DigestAlgorithm,
    /// Compatibility policy.
    pub compatibility: CompatibilityPolicy,
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
        let identity = Self {
            schema_kind,
            semantic_type_id,
            schema_name: schema_name.into(),
            schema_version,
            shape,
            canonicalization: DigestAlgorithm::Sha256JcsV1,
            compatibility: CompatibilityPolicy::ManualVersion,
            persisted_surface: PersistedSurfacePolicy::strict(),
        };
        identity.validate()?;
        Ok(identity)
    }

    fn validate(&self) -> Result<()> {
        validate_schema_name(&self.schema_name)?;
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
            ("compatibility", self.compatibility.to_canonical_value()),
            (
                "persisted_surface",
                self.persisted_surface.to_canonical_value(),
            ),
            ("schema_kind", string(self.schema_kind.as_str())),
            ("schema_name", string(&self.schema_name)),
            ("schema_version", string(self.schema_version.as_str())),
            (
                "semantic_type_id",
                optional_string(self.semantic_type_id.as_ref().map(SemanticTypeId::as_str)),
            ),
            ("shape", self.shape.to_canonical_value()),
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

    /// Creates derive-generated audit provenance.
    #[cfg(test)]
    pub(crate) fn derive_generated(
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

/// Schema compatibility policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CompatibilityPolicy {
    /// Breaking shape or semantic changes require a manually assigned new
    /// schema version.
    ManualVersion,
}

impl CompatibilityPolicy {
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

/// State-boundary optional value with explicit skip provenance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(bound(
    serialize = "T: Serialize",
    deserialize = "T: serde::de::DeserializeOwned"
))]
pub enum MaybeValue<T: MfmValue> {
    /// A produced value.
    Produced(T),
    /// A skipped cell with replayable reason.
    Skipped(SkipReason),
}

impl<T: MfmValue> MfmValue for MaybeValue<T> {
    fn schema_descriptor() -> Result<SchemaDescriptor> {
        let produced = SchemaShape::Tuple(vec![SchemaShape::ValueRef {
            schema_id: T::schema_id()?,
            semantic_type_id: T::semantic_id()?,
        }]);
        let skipped = skip_reason_shape()?;
        let serialized_shape = SchemaShape::external_enum(vec![
            EnumVariantDescriptor::new("Produced", produced),
            EnumVariantDescriptor::new("Skipped", skipped),
        ])?;

        framework_value_descriptor(
            Self::semantic_id()?,
            "mfm.kernel.maybe_value",
            SchemaShape::Generic {
                constructor: "mfm.kernel/maybe-value".to_owned(),
                arguments: vec![GenericArgumentDescriptor::for_value::<T>()?],
                serialized_shape: Box::new(serialized_shape),
            },
            "mfm_values::MaybeValue",
        )
    }

    fn semantic_id() -> Result<SemanticTypeId> {
        SemanticTypeId::new(
            "mfm.kernel",
            "maybe-value",
            "1",
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([0x11; 32]),
        )
        .map_err(|error| ValueError::Identity(error.to_string()))
    }

    fn terminal_policy() -> ValueTerminalPolicy {
        ValueTerminalPolicy::MaybeSkipped
    }
}

/// Skip reason stored for skipped optional cells.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkipReason {
    /// Stable skip code.
    pub code: SkipCode,
    /// Human-readable non-secret explanation.
    pub explanation: String,
}

/// Stable skip code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkipCode {
    /// Upstream dependency was skipped.
    DependencySkipped,
    /// Domain filter intentionally skipped this value.
    Filtered,
    /// Policy prevented production.
    Policy,
    /// Value is not applicable for this run.
    NotApplicable,
}

/// Runtime non-empty input collection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(bound(serialize = "T: Serialize"))]
pub struct NonEmpty<T: MfmValue> {
    values: Vec<T>,
}

impl<T: MfmValue> NonEmpty<T> {
    /// Creates a non-empty collection from a first value and optional rest.
    pub fn new(first: T, mut rest: Vec<T>) -> Self {
        let mut values = Vec::with_capacity(rest.len() + 1);
        values.push(first);
        values.append(&mut rest);
        Self { values }
    }

    /// Attempts to create a non-empty collection from a vector.
    pub fn try_from_vec(values: Vec<T>) -> Result<Self> {
        if values.is_empty() {
            return Err(ValueError::Config(
                "non-empty value collection cannot be empty".to_owned(),
            ));
        }
        Ok(Self { values })
    }

    /// Returns values in their retained order.
    pub fn values(&self) -> &[T] {
        &self.values
    }
}

impl<'de, T: MfmValue> Deserialize<'de> for NonEmpty<T> {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(bound(deserialize = "T: serde::de::DeserializeOwned"))]
        struct NonEmptyWire<T: MfmValue> {
            values: Vec<T>,
        }

        let wire = NonEmptyWire::<T>::deserialize(deserializer)?;
        NonEmpty::try_from_vec(wire.values).map_err(de::Error::custom)
    }
}

impl<T: MfmValue> StateInput for NonEmpty<T> {
    fn input_schema_descriptor() -> Result<SchemaDescriptor> {
        framework_input_descriptor(
            "mfm.kernel.state_input.non_empty",
            SchemaShape::NonEmptyVec(Box::new(SchemaShape::ValueRef {
                schema_id: T::schema_id()?,
                semantic_type_id: T::semantic_id()?,
            })),
            "mfm_values::NonEmpty",
        )
    }
}

/// Typed artifact reference that must match the referenced value type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactRef<T: MfmValue> {
    /// Artifact storage identity.
    pub id: ArtifactId,
    /// Artifact content digest.
    pub digest: ContentDigest,
    /// Referenced value schema id.
    pub schema_id: SchemaId,
    /// Referenced value semantic type id.
    pub semantic_type_id: SemanticTypeId,
    _value: PhantomData<fn(T) -> T>,
}

impl<T: MfmValue> ArtifactRef<T> {
    /// Creates an artifact reference for `T`.
    pub fn new(id: ArtifactId, digest: ContentDigest) -> Result<Self> {
        Ok(Self {
            id,
            digest,
            schema_id: T::schema_id()?,
            semantic_type_id: T::semantic_id()?,
            _value: PhantomData,
        })
    }

    /// Creates an artifact reference from persisted parts and verifies the
    /// schema and semantic ids against `T`.
    pub fn from_parts(
        id: ArtifactId,
        digest: ContentDigest,
        schema_id: SchemaId,
        semantic_type_id: SemanticTypeId,
    ) -> Result<Self> {
        let expected_schema_id = T::schema_id()?;
        if schema_id != expected_schema_id {
            return Err(ValueError::ArtifactTypeMismatch {
                field: "schema_id",
                expected: expected_schema_id.to_string(),
                actual: schema_id.to_string(),
            });
        }

        let expected_semantic_type_id = T::semantic_id()?;
        if semantic_type_id != expected_semantic_type_id {
            return Err(ValueError::ArtifactTypeMismatch {
                field: "semantic_type_id",
                expected: expected_semantic_type_id.to_string(),
                actual: semantic_type_id.to_string(),
            });
        }

        Ok(Self {
            id,
            digest,
            schema_id,
            semantic_type_id,
            _value: PhantomData,
        })
    }
}

impl<T: MfmValue> Serialize for ArtifactRef<T> {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("ArtifactRef", 4)?;
        state.serialize_field("id", self.id.as_str())?;
        state.serialize_field("digest", self.digest.as_str())?;
        state.serialize_field("schema_id", self.schema_id.as_str())?;
        state.serialize_field("semantic_type_id", self.semantic_type_id.as_str())?;
        state.end()
    }
}

impl<'de, T: MfmValue> Deserialize<'de> for ArtifactRef<T> {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = ArtifactRefWire::deserialize(deserializer)?;
        let id = wire.id.parse().map_err(de::Error::custom)?;
        let digest = wire.digest.parse().map_err(de::Error::custom)?;
        let schema_id = wire.schema_id.parse().map_err(de::Error::custom)?;
        let semantic_type_id = wire.semantic_type_id.parse().map_err(de::Error::custom)?;
        Self::from_parts(id, digest, schema_id, semantic_type_id).map_err(de::Error::custom)
    }
}

#[derive(Deserialize)]
struct ArtifactRefWire {
    id: String,
    digest: String,
    schema_id: String,
    semantic_type_id: String,
}

impl<T: MfmValue> MfmValue for ArtifactRef<T> {
    fn schema_descriptor() -> Result<SchemaDescriptor> {
        let serialized_shape = SchemaShape::named_struct(vec![
            FieldDescriptor::required("digest", SchemaShape::String),
            FieldDescriptor::required("id", SchemaShape::String),
            FieldDescriptor::required("schema_id", SchemaShape::String),
            FieldDescriptor::required("semantic_type_id", SchemaShape::String),
        ])?;

        framework_value_descriptor(
            Self::semantic_id()?,
            "mfm.kernel.artifact_ref",
            SchemaShape::Generic {
                constructor: "mfm.kernel/artifact-ref".to_owned(),
                arguments: vec![GenericArgumentDescriptor::for_value::<T>()?],
                serialized_shape: Box::new(serialized_shape),
            },
            "mfm_values::ArtifactRef",
        )
    }

    fn semantic_id() -> Result<SemanticTypeId> {
        SemanticTypeId::new(
            "mfm.kernel",
            "artifact-ref",
            "1",
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([0x22; 32]),
        )
        .map_err(|error| ValueError::Identity(error.to_string()))
    }
}

fn framework_value_descriptor(
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

fn validate_schema_name(value: &str) -> Result<()> {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return Err(ValueError::Descriptor(
            "schema name must not be empty".to_owned(),
        ));
    };

    if !first.is_ascii_lowercase() && !first.is_ascii_digit() {
        return Err(ValueError::Descriptor(format!(
            "schema name must start with [a-z0-9], got '{value}'"
        )));
    }

    for ch in chars {
        if ch.is_ascii_lowercase() || ch.is_ascii_digit() || matches!(ch, '.' | '_' | '-' | '/') {
            continue;
        }
        return Err(ValueError::Descriptor(format!(
            "schema name contains invalid character '{ch}' in '{value}'"
        )));
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
