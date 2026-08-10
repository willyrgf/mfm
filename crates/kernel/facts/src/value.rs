use std::collections::BTreeMap;

use mfm_canonical::{CanonicalJsonBytes, CanonicalValue};
use mfm_values::{
    string_contains_secret_marker, CanonicalJsonPersistedSchema, CanonicalJsonProfile,
    PersistedSchema, SchemaIdentity, SchemaKind, SchemaShape, ValueError, MAX_ARRAY_ITEMS,
    MAX_CANONICAL_OBJECT_KEY_UTF8_BYTES, MAX_OBJECT_ENTRIES, MAX_SCHEMA_DEPTH,
    MAX_STRING_UTF8_BYTES,
};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::{FactError, Result};

/// The direct unsigned-native JSON tree admitted by fact subjects and predicates.
///
/// This type owns the one recursive fact-value schema. Wrappers retain the typed
/// tree itself; canonical bytes are produced only when a persistence boundary
/// needs them.
#[derive(Debug, Clone, PartialEq, Eq)]
enum FactValue {
    Null,
    Bool(bool),
    String(String),
    Unsigned(u64),
    Array(Vec<Self>),
    Object(BTreeMap<String, Self>),
}

impl FactValue {
    fn from_canonical(value: CanonicalValue) -> Result<Self> {
        let value = match value {
            CanonicalValue::Null => Self::Null,
            CanonicalValue::Bool(value) => Self::Bool(value),
            CanonicalValue::String(value) => Self::String(value),
            CanonicalValue::Unsigned(value) => Self::Unsigned(value),
            CanonicalValue::Array(values) => Self::Array(
                values
                    .into_iter()
                    .map(Self::from_canonical)
                    .collect::<Result<Vec<_>>>()?,
            ),
            CanonicalValue::Object(entries) => Self::Object(
                entries
                    .entries()
                    .map(|(key, value)| Ok((key.to_owned(), Self::from_canonical(value.clone())?)))
                    .collect::<Result<BTreeMap<_, _>>>()?,
            ),
            CanonicalValue::Signed(_) | CanonicalValue::Decimal(_) | CanonicalValue::Bytes(_) => {
                return Err(FactError::Canonical)
            }
        };
        value
            .validate_at_depth(0)
            .map_err(|_| FactError::Canonical)?;
        Ok(value)
    }

    fn from_json(value: serde_json::Value) -> std::result::Result<Self, &'static str> {
        let value = match value {
            serde_json::Value::Null => Self::Null,
            serde_json::Value::Bool(value) => Self::Bool(value),
            serde_json::Value::String(value) => Self::String(value),
            serde_json::Value::Number(value) => Self::Unsigned(
                value
                    .as_u64()
                    .ok_or("fact numbers must be unsigned integers")?,
            ),
            serde_json::Value::Array(values) => Self::Array(
                values
                    .into_iter()
                    .map(Self::from_json)
                    .collect::<std::result::Result<Vec<_>, _>>()?,
            ),
            serde_json::Value::Object(entries) => Self::Object(
                entries
                    .into_iter()
                    .map(|(key, value)| Ok((key, Self::from_json(value)?)))
                    .collect::<std::result::Result<BTreeMap<_, _>, _>>()?,
            ),
        };
        value
            .validate_at_depth(0)
            .map_err(|_| "fact value is outside its closed bounds")?;
        Ok(value)
    }

    fn to_canonical(&self) -> CanonicalValue {
        match self {
            Self::Null => CanonicalValue::Null,
            Self::Bool(value) => CanonicalValue::Bool(*value),
            Self::String(value) => CanonicalValue::String(value.clone()),
            Self::Unsigned(value) => CanonicalValue::Unsigned(*value),
            Self::Array(values) => {
                CanonicalValue::Array(values.iter().map(Self::to_canonical).collect())
            }
            Self::Object(entries) => CanonicalValue::object(
                entries
                    .iter()
                    .map(|(key, value)| (key.clone(), value.to_canonical())),
            )
            .expect("a BTreeMap has unique keys"),
        }
    }

    fn validate_at_depth(&self, depth: usize) -> mfm_values::Result<()> {
        if depth > MAX_SCHEMA_DEPTH {
            return Err(ValueError::SchemaShapeMismatch);
        }
        match self {
            Self::Null | Self::Bool(_) | Self::Unsigned(_) => Ok(()),
            Self::String(value) => {
                if value.len() > MAX_STRING_UTF8_BYTES || string_contains_secret_marker(value) {
                    return Err(ValueError::SchemaShapeMismatch);
                }
                Ok(())
            }
            Self::Array(values) => {
                if values.len() > MAX_ARRAY_ITEMS {
                    return Err(ValueError::SchemaShapeMismatch);
                }
                values
                    .iter()
                    .try_for_each(|value| value.validate_at_depth(depth + 1))
            }
            Self::Object(entries) => {
                if entries.len() > MAX_OBJECT_ENTRIES {
                    return Err(ValueError::SchemaShapeMismatch);
                }
                for (key, value) in entries {
                    if key.is_empty()
                        || key.len() > MAX_CANONICAL_OBJECT_KEY_UTF8_BYTES
                        || key.chars().any(char::is_control)
                    {
                        return Err(ValueError::SchemaShapeMismatch);
                    }
                    value.validate_at_depth(depth + 1)?;
                }
                Ok(())
            }
        }
    }

    fn encode(&self) -> Result<CanonicalJsonBytes> {
        self.validate_at_depth(0)
            .map_err(|_| FactError::Canonical)?;
        Ok(CanonicalJsonBytes::from_value(&self.to_canonical()))
    }
}

impl Serialize for FactValue {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::Null => serializer.serialize_unit(),
            Self::Bool(value) => serializer.serialize_bool(*value),
            Self::String(value) => serializer.serialize_str(value),
            Self::Unsigned(value) => serializer.serialize_u64(*value),
            Self::Array(values) => values.serialize(serializer),
            Self::Object(entries) => entries.serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for FactValue {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = serde_json::Value::deserialize(deserializer)?;
        Self::from_json(value).map_err(serde::de::Error::custom)
    }
}

impl PersistedSchema for FactValue {
    fn schema_identity() -> mfm_values::Result<SchemaIdentity> {
        SchemaIdentity::new(
            SchemaKind::PersistedContract,
            None,
            "mfm.fact-value",
            mfm_ids::SchemaVersion::new("1")
                .map_err(|error| ValueError::Identity(error.to_string()))?,
            SchemaShape::CanonicalJsonTerminal {
                profile: CanonicalJsonProfile::UnsignedNative,
            },
        )
    }

    fn validate(&self) -> mfm_values::Result<()> {
        self.validate_at_depth(0)
    }
}

/// One bounded, float-free scalar usable as exact fact subject material.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct FactScalar(FactValue);

impl<'de> Deserialize<'de> for FactScalar {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = FactValue::deserialize(deserializer)?;
        if !matches!(
            value,
            FactValue::Bool(_) | FactValue::String(_) | FactValue::Unsigned(_)
        ) {
            return Err(serde::de::Error::custom(
                "fact scalar must be a boolean, string, or unsigned integer",
            ));
        }
        Ok(Self(value))
    }
}

impl FactScalar {
    /// Creates a checked string scalar.
    pub fn string(value: impl Into<String>) -> Result<Self> {
        Self::from_canonical_value(CanonicalValue::String(value.into()))
    }

    /// Creates a checked boolean scalar.
    pub fn boolean(value: bool) -> Result<Self> {
        Self::from_canonical_value(CanonicalValue::Bool(value))
    }

    /// Creates a checked unsigned-integer scalar.
    pub fn unsigned(value: u64) -> Result<Self> {
        Self::from_canonical_value(CanonicalValue::Unsigned(value))
    }

    /// Validates one canonical scalar against the fact value shape.
    pub fn from_canonical_value(value: CanonicalValue) -> Result<Self> {
        let value = FactValue::from_canonical(value)?;
        if !matches!(
            value,
            FactValue::Bool(_) | FactValue::String(_) | FactValue::Unsigned(_)
        ) {
            return Err(FactError::Descriptor(
                "fact scalar must be a boolean, string, or unsigned integer",
            ));
        }
        Ok(Self(value))
    }

    /// Encodes the exact float-free canonical JSON scalar.
    pub fn canonical_json(&self) -> Result<CanonicalJsonBytes> {
        self.0.encode()
    }

    /// Returns the checked canonical scalar.
    pub fn canonical_value(&self) -> CanonicalValue {
        self.0.to_canonical()
    }
}

/// Exact canonical subject material used by a fact descriptor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FactSubject(FactValue);

impl FactSubject {
    /// Strictly decodes exact canonical subject bytes against the fact value shape.
    pub fn from_canonical_json(bytes: &[u8]) -> Result<Self> {
        FactValue::decode_canonical(bytes)
            .map(Self)
            .map_err(|_| FactError::Canonical)
    }

    /// Validates and canonicalizes typed subject material.
    pub fn from_canonical_value(value: CanonicalValue) -> Result<Self> {
        FactValue::from_canonical(value).map(Self)
    }

    /// Creates subject material from one checked scalar.
    pub fn from_scalar(value: &FactScalar) -> Self {
        Self(value.0.clone())
    }

    /// Encodes exact canonical JSON bytes.
    pub fn canonical_json(&self) -> Result<CanonicalJsonBytes> {
        self.0.encode()
    }

    /// Returns the checked canonical subject.
    pub fn canonical_value(&self) -> CanonicalValue {
        self.0.to_canonical()
    }
}

/// Exact state-authored predicate for the reserved fact-selection capability.
///
/// Baseline matching is exact typed equality with the producer's subject
/// material. Adaptive or range selection is represented by another typed read
/// state, not a mutable or store-injected query language.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CanonicalFactPredicate(FactValue);

impl PersistedSchema for CanonicalFactPredicate {
    fn schema_identity() -> mfm_values::Result<SchemaIdentity> {
        SchemaIdentity::new(
            SchemaKind::PersistedContract,
            None,
            "mfm.canonical-fact-predicate",
            mfm_ids::SchemaVersion::new("1")
                .map_err(|error| ValueError::Identity(error.to_string()))?,
            FactValue::schema_shape()?,
        )
    }

    fn validate(&self) -> mfm_values::Result<()> {
        self.0.validate_at_depth(0)
    }
}

impl CanonicalFactPredicate {
    /// Strictly decodes exact canonical predicate bytes against the fact value shape.
    pub fn from_canonical_json(bytes: &[u8]) -> Result<Self> {
        FactValue::decode_canonical(bytes)
            .map(Self)
            .map_err(|_| FactError::Canonical)
    }

    /// Validates and canonicalizes a typed predicate.
    pub fn from_canonical_value(value: CanonicalValue) -> Result<Self> {
        FactValue::from_canonical(value).map(Self)
    }

    /// Builds an exact predicate from canonical fact subject material.
    pub fn exact_subject(subject: &FactSubject) -> Self {
        Self(subject.0.clone())
    }

    /// Encodes exact canonical JSON bytes.
    pub fn canonical_json(&self) -> Result<CanonicalJsonBytes> {
        self.0.encode()
    }

    /// Returns the checked canonical predicate.
    pub fn canonical_value(&self) -> CanonicalValue {
        self.0.to_canonical()
    }

    /// Returns whether exact canonical subject material satisfies this predicate.
    pub fn matches(&self, subject: &FactSubject) -> bool {
        self.0 == subject.0
    }
}
