use std::collections::{BTreeMap, BTreeSet};

use mfm_canonical::CanonicalValue;

use crate::codec::canonical_object;
use crate::scalar::ScalarJsonContext;
use crate::*;

/// Claim and store metadata available to descriptor field extraction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactExtractionMetadata {
    pub(crate) recorded_at: String,
    pub(crate) store_commit_order: u64,
}

impl FactExtractionMetadata {
    /// Creates extraction metadata for fact term derivation.
    pub fn new(recorded_at: impl Into<String>, store_commit_order: u64) -> Result<Self> {
        let recorded_at = recorded_at.into();
        if recorded_at.is_empty() {
            return Err(FactError::descriptor(
                "recorded_at metadata must not be empty",
            ));
        }
        Ok(Self {
            recorded_at,
            store_commit_order,
        })
    }

    /// Returns the store-assigned recorded time.
    pub fn recorded_at(&self) -> &str {
        &self.recorded_at
    }

    /// Returns the store-owned commit ordering coordinate.
    pub const fn store_commit_order(&self) -> u64 {
        self.store_commit_order
    }
}

/// One extracted index term for an indexed fact claim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactIndexTerm {
    pub(crate) field_id: FactFieldId,
    pub(crate) source: FactFieldSource,
    pub(crate) value_type: FactFieldValueType,
    pub(crate) value: FactCanonicalScalar,
    pub(crate) unit: Option<FactUnit>,
    pub(crate) scale: Option<FactScale>,
}

impl FactIndexTerm {
    /// Creates a term from a descriptor field and extracted scalar.
    pub fn from_field(field: &FactFieldDescriptor, value: FactCanonicalScalar) -> Result<Self> {
        validate_extracted_scalar(field, &value)?;
        Ok(Self {
            field_id: field.field_id.clone(),
            source: field.extraction.source(),
            value_type: field.value_type,
            value,
            unit: field.unit.clone(),
            scale: field.scale,
        })
    }

    /// Returns this term's field id.
    pub const fn field_id(&self) -> &FactFieldId {
        &self.field_id
    }

    /// Returns the source category for this term.
    pub const fn source(&self) -> FactFieldSource {
        self.source
    }

    /// Returns this term's value type.
    pub const fn value_type(&self) -> FactFieldValueType {
        self.value_type
    }

    /// Returns this term's scalar value.
    pub const fn value(&self) -> &FactCanonicalScalar {
        &self.value
    }

    /// Returns this term's unit.
    pub const fn unit(&self) -> Option<&FactUnit> {
        self.unit.as_ref()
    }

    /// Returns this term's scale.
    pub const fn scale(&self) -> Option<FactScale> {
        self.scale
    }
}

pub(crate) fn extract_field_scalar(
    field: &FactFieldDescriptor,
    subject: &CanonicalValue,
    response: &CanonicalValue,
    metadata: Option<&FactExtractionMetadata>,
) -> Result<Option<FactCanonicalScalar>> {
    match &field.extraction {
        FactFieldExtraction::Subject(path) => extract_path_scalar(field, subject, path),
        FactFieldExtraction::Response(path) => extract_path_scalar(field, response, path),
        FactFieldExtraction::Metadata(metadata_field) => {
            let metadata = metadata.ok_or_else(|| {
                FactError::field(
                    field.field_id.clone(),
                    "metadata extraction requires metadata",
                )
            })?;
            metadata_scalar(field, *metadata_field, metadata)
        }
    }
}

fn extract_path_scalar(
    field: &FactFieldDescriptor,
    root: &CanonicalValue,
    path: &CanonicalValuePath,
) -> Result<Option<FactCanonicalScalar>> {
    let Some(value) = extract_path_value(field, root, path)? else {
        return Ok(None);
    };
    if matches!(value, CanonicalValue::Null) {
        return Ok(None);
    }
    scalar_from_value(field, value).map(Some)
}

fn extract_path_value<'a>(
    field: &FactFieldDescriptor,
    root: &'a CanonicalValue,
    path: &CanonicalValuePath,
) -> Result<Option<&'a CanonicalValue>> {
    let mut current = root;
    for segment in path.as_str().split('.') {
        match current {
            CanonicalValue::Object(object) => {
                let next = object
                    .entries()
                    .find(|(key, _)| *key == segment)
                    .map(|(_, value)| value);
                let Some(next) = next else {
                    return Ok(None);
                };
                current = next;
            }
            CanonicalValue::Null => return Ok(None),
            CanonicalValue::Array(_) => {
                return Err(FactError::field(
                    field.field_id.clone(),
                    "arrays, wildcards, slices, and repeated values are unsupported in fact paths",
                ));
            }
            _ => {
                return Err(FactError::field(
                    field.field_id.clone(),
                    format!("path segment {segment:?} traversed a non-object scalar"),
                ));
            }
        }
    }
    Ok(Some(current))
}

fn metadata_scalar(
    field: &FactFieldDescriptor,
    metadata_field: FactMetadataField,
    metadata: &FactExtractionMetadata,
) -> Result<Option<FactCanonicalScalar>> {
    let scalar = match metadata_field {
        FactMetadataField::RecordedAt => {
            Some(FactCanonicalScalar::Timestamp(metadata.recorded_at.clone()))
        }
        FactMetadataField::StoreCommitOrder => Some(FactCanonicalScalar::UnsignedInteger(
            metadata.store_commit_order,
        )),
    };
    if let Some(scalar) = &scalar {
        validate_extracted_scalar(field, scalar)?;
    }
    Ok(scalar)
}

fn scalar_from_value(
    field: &FactFieldDescriptor,
    value: &CanonicalValue,
) -> Result<FactCanonicalScalar> {
    let scalar = FactCanonicalScalar::from_canonical_value(
        field.value_type,
        value,
        ScalarJsonContext::Field(&field.field_id),
    )?;
    validate_extracted_scalar(field, &scalar)?;
    Ok(scalar)
}

pub(crate) fn response_typed_paths(
    descriptor: &FactDescriptor,
) -> Result<BTreeMap<String, FactFieldValueType>> {
    typed_paths_for_extraction(descriptor, FactJsonPathContext::Response)
}

pub(crate) fn subject_typed_paths(
    descriptor: &FactDescriptor,
) -> Result<BTreeMap<String, FactFieldValueType>> {
    typed_paths_for_extraction(descriptor, FactJsonPathContext::Subject)
}

#[derive(Clone, Copy)]
pub(crate) enum FactJsonPathContext {
    Subject,
    Response,
}

impl FactJsonPathContext {
    const fn label(self) -> &'static str {
        match self {
            Self::Subject => "subject",
            Self::Response => "response",
        }
    }

    const fn scalar_context<'a>(self, path: &'a str) -> ScalarJsonContext<'a> {
        match self {
            Self::Subject => ScalarJsonContext::Subject(path),
            Self::Response => ScalarJsonContext::Response(path),
        }
    }
}

fn typed_paths_for_extraction(
    descriptor: &FactDescriptor,
    context: FactJsonPathContext,
) -> Result<BTreeMap<String, FactFieldValueType>> {
    let mut typed_paths = BTreeMap::new();
    for field in descriptor.fields() {
        let path = match (context, field.extraction()) {
            (FactJsonPathContext::Subject, FactFieldExtraction::Subject(path))
            | (FactJsonPathContext::Response, FactFieldExtraction::Response(path)) => path,
            _ => continue,
        };
        if let Some(previous) = typed_paths.insert(path.as_str().to_owned(), field.value_type()) {
            if previous != field.value_type() {
                return Err(FactError::field(
                    field.field_id().clone(),
                    format!(
                        "{} path {} is declared with incompatible value types",
                        context.label(),
                        path
                    ),
                ));
            }
        }
    }
    Ok(typed_paths)
}

pub(crate) fn json_to_fact_canonical_value(
    value: &serde_json::Value,
    path: &str,
    typed_paths: &BTreeMap<String, FactFieldValueType>,
    context: FactJsonPathContext,
) -> Result<CanonicalValue> {
    if let Some(value_type) = typed_paths.get(path) {
        return json_to_typed_fact_scalar_with_context(
            value,
            *value_type,
            context.scalar_context(path),
        );
    }

    match value {
        serde_json::Value::Null => Ok(CanonicalValue::Null),
        serde_json::Value::Bool(value) => Ok(CanonicalValue::Bool(*value)),
        serde_json::Value::Number(value) => {
            if let Some(value) = value.as_u64() {
                Ok(CanonicalValue::Unsigned(value))
            } else if let Some(value) = value.as_i64() {
                Ok(CanonicalValue::Signed(value))
            } else {
                Err(FactError::descriptor(format!(
                    "fact {} path {path} contains unsupported floating-point number",
                    context.label()
                )))
            }
        }
        serde_json::Value::String(value) => Ok(CanonicalValue::String(value.clone())),
        serde_json::Value::Array(values) => values
            .iter()
            .enumerate()
            .map(|(index, value)| {
                let child_path = if path.is_empty() {
                    index.to_string()
                } else {
                    format!("{path}.{index}")
                };
                json_to_fact_canonical_value(value, &child_path, typed_paths, context)
            })
            .collect::<Result<Vec<_>>>()
            .map(CanonicalValue::Array),
        serde_json::Value::Object(entries) => {
            let values = entries
                .iter()
                .map(|(key, value)| {
                    let child_path = if path.is_empty() {
                        key.clone()
                    } else {
                        format!("{path}.{key}")
                    };
                    Ok((
                        key.clone(),
                        json_to_fact_canonical_value(value, &child_path, typed_paths, context)?,
                    ))
                })
                .collect::<Result<Vec<_>>>()?;
            canonical_object(values)
        }
    }
}

#[cfg(test)]
pub(crate) fn json_to_typed_fact_scalar(
    value: &serde_json::Value,
    value_type: FactFieldValueType,
    path: &str,
) -> Result<CanonicalValue> {
    json_to_typed_fact_scalar_with_context(value, value_type, ScalarJsonContext::Response(path))
}

fn json_to_typed_fact_scalar_with_context(
    value: &serde_json::Value,
    value_type: FactFieldValueType,
    context: ScalarJsonContext<'_>,
) -> Result<CanonicalValue> {
    if value.is_null() && matches!(context, ScalarJsonContext::Response(_)) {
        return Ok(CanonicalValue::Null);
    }
    FactCanonicalScalar::from_json_value(value_type, value, context)
        .map(|scalar| scalar.canonical_value())
}

pub(crate) fn validate_extracted_scalar(
    field: &FactFieldDescriptor,
    scalar: &FactCanonicalScalar,
) -> Result<()> {
    if scalar.value_type() != field.value_type {
        return Err(FactError::field(
            field.field_id.clone(),
            format!(
                "field expects {:?} but extracted {:?}",
                field.value_type,
                scalar.value_type()
            ),
        ));
    }
    validate_scalar_size(&field.field_id, scalar)?;
    validate_decimal_scale(field, scalar)?;
    Ok(())
}

pub(crate) fn validate_scalar_size(
    field_id: &FactFieldId,
    scalar: &FactCanonicalScalar,
) -> Result<()> {
    let len = scalar.canonical_text_len();
    if len > MAX_FACT_SCALAR_BYTES {
        return Err(FactError::field(
            field_id.clone(),
            format!("scalar exceeds {MAX_FACT_SCALAR_BYTES} byte limit"),
        ));
    }
    Ok(())
}

fn validate_decimal_scale(field: &FactFieldDescriptor, scalar: &FactCanonicalScalar) -> Result<()> {
    let Some(scale) = field.scale else {
        return Ok(());
    };
    let FactCanonicalScalar::DecimalString(value) = scalar else {
        return Ok(());
    };
    if scale.exponent() >= 0 {
        if decimal_fraction_digits(value.as_str()) != 0 {
            return Err(FactError::field(
                field.field_id.clone(),
                "decimal field with non-negative scale exponent must not contain fractional digits",
            ));
        }
        return Ok(());
    }

    let expected_digits = usize::from(scale.exponent().unsigned_abs());
    let actual_digits = decimal_fraction_digits(value.as_str());
    if actual_digits != expected_digits {
        return Err(FactError::field(
            field.field_id.clone(),
            format!(
                "decimal field scale expects {expected_digits} fractional digits but found {actual_digits}"
            ),
        ));
    }
    Ok(())
}

fn decimal_fraction_digits(value: &str) -> usize {
    value
        .split_once('.')
        .map(|(_, fraction)| fraction.len())
        .unwrap_or(0)
}

pub(crate) fn validate_ordering(
    ordering: &FactOrderingPolicy,
    fields_by_id: &BTreeMap<FactFieldId, &FactFieldDescriptor>,
) -> Result<()> {
    let mut seen_fields = BTreeSet::new();
    for term in &ordering.terms {
        if !seen_fields.insert(term.field_id.clone()) {
            return Err(FactError::ordering(
                ordering.name.clone(),
                format!("duplicate ordering field {}", term.field_id),
            ));
        }

        let Some(field) = fields_by_id.get(&term.field_id) else {
            return Err(FactError::ordering(
                ordering.name.clone(),
                format!("ordering references unknown field {}", term.field_id),
            ));
        };

        if !field.sortable {
            return Err(FactError::ordering(
                ordering.name.clone(),
                format!("ordering references non-sortable field {}", term.field_id),
            ));
        }
    }

    Ok(())
}
