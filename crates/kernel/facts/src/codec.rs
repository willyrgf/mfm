use std::str::FromStr;

use mfm_canonical::{
    CanonicalObject, CanonicalValue, RecoverabilityContractV2, ValidatedCanonicalValueV2,
};
use mfm_ids::{ContentDigest, ContentRef, SchemaId};

use crate::{FactError, Result};

pub(crate) fn contract() -> Result<&'static RecoverabilityContractV2> {
    RecoverabilityContractV2::embedded().map_err(Into::into)
}

pub(crate) fn encode(
    schema_contract: &str,
    value: &CanonicalValue,
) -> Result<ValidatedCanonicalValueV2> {
    contract()?
        .encode(schema_contract, value)
        .map_err(Into::into)
}

pub(crate) fn strict_decode(
    schema_contract: &str,
    bytes: &[u8],
) -> Result<ValidatedCanonicalValueV2> {
    contract()?
        .strict_decode(schema_contract, bytes)
        .map_err(Into::into)
}

pub(crate) fn object(
    entries: impl IntoIterator<Item = (impl Into<String>, CanonicalValue)>,
) -> Result<CanonicalValue> {
    CanonicalValue::object(entries).map_err(|_| FactError::Canonical)
}

pub(crate) fn canonical_content_ref(value: &ContentRef) -> Result<CanonicalValue> {
    object([
        (
            "schema_id",
            CanonicalValue::String(value.schema_id().as_str().to_owned()),
        ),
        (
            "content_digest",
            CanonicalValue::String(value.content_digest().as_str().to_owned()),
        ),
    ])
}

pub(crate) fn required_object(value: &CanonicalValue) -> Result<&CanonicalObject> {
    match value {
        CanonicalValue::Object(object) => Ok(object),
        _ => Err(FactError::Canonical),
    }
}

pub(crate) fn required_field<'a>(
    object: &'a CanonicalObject,
    name: &str,
) -> Result<&'a CanonicalValue> {
    object
        .entries()
        .find_map(|(key, value)| (key == name).then_some(value))
        .ok_or(FactError::Canonical)
}

pub(crate) fn optional_field<'a>(
    object: &'a CanonicalObject,
    name: &str,
) -> Option<&'a CanonicalValue> {
    object
        .entries()
        .find_map(|(key, value)| (key == name).then_some(value))
}

pub(crate) fn string(value: &CanonicalValue) -> Result<&str> {
    match value {
        CanonicalValue::String(value) => Ok(value),
        _ => Err(FactError::Canonical),
    }
}

pub(crate) fn unsigned(value: &CanonicalValue) -> Result<u64> {
    match value {
        CanonicalValue::Unsigned(value) => Ok(*value),
        _ => Err(FactError::Canonical),
    }
}

pub(crate) fn content_ref(value: &CanonicalValue) -> Result<ContentRef> {
    let object = required_object(value)?;
    let schema_id = SchemaId::from_str(string(required_field(object, "schema_id")?)?)
        .map_err(|_| FactError::Identity)?;
    let content_digest =
        ContentDigest::from_str(string(required_field(object, "content_digest")?)?)
            .map_err(|_| FactError::Identity)?;
    ContentRef::new(schema_id, content_digest).map_err(|_| FactError::Identity)
}
