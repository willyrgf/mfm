use crate::{
    canonicalize_mfm_value, InvocationDiagnostic, MfmValue, SchemaDescriptor, SizeLimitExceeded,
    ValueError, MAX_RUN_OBJECT_CANONICAL_BYTES,
};
use mfm_canonical::{raw_content_digest, PlainCanonicalJsonBytes};
use mfm_ids::ContentRef;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::value::RawValue;
use std::sync::Arc;

/// One immutable canonical value and its exact content identity, serialized inline.
#[derive(Clone, PartialEq, Eq)]
pub struct Object {
    value_ref: Arc<ContentRef>,
    canonical: PlainCanonicalJsonBytes,
}
impl std::fmt::Debug for Object {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Object")
            .field("value_ref", &self.value_ref)
            .finish_non_exhaustive()
    }
}
impl Object {
    /// Canonicalizes and admits a typed owner through Values' established schema rule.
    pub fn from_value<T: MfmValue>(value: &T) -> Result<Self, ValueError> {
        let (canonical, value_ref) = canonicalize_mfm_value(value)?;
        Ok(Self {
            value_ref: Arc::new(value_ref),
            canonical,
        })
    }
    /// Checks an existing canonical payload and its claimed content hash.
    /// The consuming slot must subsequently admit its associated descriptor.
    pub fn from_canonical(value_ref: ContentRef, bytes: &[u8]) -> Result<Self, ValueError> {
        SizeLimitExceeded::check(bytes.len() as u64, MAX_RUN_OBJECT_CANONICAL_BYTES as u64)?;
        let canonical = PlainCanonicalJsonBytes::from_canonical_json_slice(bytes)
            .map_err(ValueError::Canonical)?;
        let digest = raw_content_digest(bytes);
        if value_ref.content_digest() != &digest {
            return Err(ValueError::ArtifactTypeMismatch {
                field: "content_digest",
                expected: digest.to_string(),
                actual: value_ref.content_digest().to_string(),
            });
        }
        Ok(Self {
            value_ref: Arc::new(value_ref),
            canonical,
        })
    }
    /// Admits the checked object into its selected descriptor without repeating hashing.
    pub fn admit(&self, descriptor: &SchemaDescriptor) -> Result<(), ValueError> {
        if self.value_ref.schema_id() != &descriptor.schema_id()? {
            return Err(ValueError::ArtifactTypeMismatch {
                field: "schema_id",
                expected: descriptor.schema_id()?.to_string(),
                actual: self.value_ref.schema_id().to_string(),
            });
        }
        descriptor
            .identity()
            .validate_canonical_value(self.canonical_bytes())
    }
    /// Materializes its exact native owner without repeating schema/hash admission.
    pub fn decode<T: MfmValue>(&self) -> Result<T, InvocationDiagnostic> {
        let descriptor = T::schema_descriptor().map_err(|error| error.into_diagnostic("decode"))?;
        if self.value_ref.schema_id()
            != &descriptor
                .schema_id()
                .map_err(|error| error.into_diagnostic("decode"))?
        {
            return Err(ValueError::InvalidSchemaIdentity.into_diagnostic("decode"));
        }
        T::decode_native(self.canonical_bytes())
    }
    /// Returns the exact value identity.
    pub fn value_ref(&self) -> &ContentRef {
        &self.value_ref
    }
    /// Derives the fixed nominal contract from the value's schema identity.
    pub fn contract_ref(&self) -> Result<ContentRef, ValueError> {
        ContentRef::new(
            self.value_ref.schema_id().clone(),
            raw_content_digest(b"mfm.contract.v1"),
        )
        .map_err(ValueError::Identity)
    }
    /// Returns its immutable canonical payload bytes.
    pub fn canonical_bytes(&self) -> &[u8] {
        self.canonical.as_bytes()
    }
}
impl Serialize for Object {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct Wire<'a> {
            value_ref: &'a ContentRef,
            canonical: &'a RawValue,
        }
        let canonical = serde_json::from_slice::<&RawValue>(self.canonical_bytes())
            .map_err(serde::ser::Error::custom)?;
        Wire {
            value_ref: &self.value_ref,
            canonical,
        }
        .serialize(serializer)
    }
}
impl<'de> Deserialize<'de> for Object {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            value_ref: ContentRef,
            canonical: Box<RawValue>,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::from_canonical(wire.value_ref, wire.canonical.get().as_bytes())
            .map_err(serde::de::Error::custom)
    }
}
