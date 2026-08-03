use std::marker::PhantomData;

use mfm_ids::{ArtifactId, ContentDigest, DigestAlgorithm, DigestBytes, SchemaId, SemanticTypeId};
use serde::de::{self, Deserializer};
use serde::ser::{SerializeStruct, Serializer};
use serde::{Deserialize, Serialize};

use super::{
    framework_value_descriptor, FieldDescriptor, GenericArgumentDescriptor, MfmValue, Result,
    SchemaDescriptor, SchemaShape, ValueError,
};

/// Runtime non-empty input collection.
///
/// Wire format is a JSON array (not `{ "values": [...] }`) so it matches
/// [`SchemaShape::NonEmptyVec`] materialization used by state-input binding.
#[derive(Debug, Clone, PartialEq, Eq)]
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

impl<T: MfmValue + Serialize> Serialize for NonEmpty<T> {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.values.serialize(serializer)
    }
}

impl<'de, T: MfmValue> Deserialize<'de> for NonEmpty<T> {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let values = Vec::<T>::deserialize(deserializer)?;
        NonEmpty::try_from_vec(values).map_err(de::Error::custom)
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
