use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::str::FromStr;

use mfm_canonical::{
    sha256_digest_bytes, CanonicalJsonBytes, CanonicalValue, PlainCanonicalJsonBytes,
};
use mfm_ids::{ContentDigest, DigestAlgorithm, RunId, SchemaId};

use crate::extraction::{
    extract_field_scalar, json_to_fact_canonical_value, response_typed_paths, subject_typed_paths,
    validate_extracted_scalar, validate_ordering, FactJsonPathContext,
};
use crate::scalar::ScalarJsonContext;
use crate::subject::{FactSubjectNamespaceFieldV1, FactSubjectNamespaceV1};
use crate::*;

#[path = "codec_canonical.rs"]
mod canonical;
use self::canonical::{
    canonical_descriptor_value, canonical_fact_claim_id_value,
    canonical_fact_query_result_set_value, canonical_query_evidence_value,
    canonical_query_plan_value, canonical_returned_fact_field_summary_value,
    canonical_subject_material_value, canonical_subject_namespace_value,
};

#[path = "codec_parse.rs"]
mod parse;
#[cfg(test)]
pub(crate) use self::parse::parse_canonical_scalar_value;
use self::parse::{
    descriptor_fields_by_id, json_object, json_required, parse_canonical_fact_query_plan,
    parse_fact_field_value, parse_fact_query_receipt, parse_fact_selection_evidence,
    require_query_exposed, require_version, required_query_field, CompiledQueryWire,
};

/// Validates a fact descriptor.
pub fn validate_descriptor(descriptor: &FactDescriptor) -> Result<()> {
    let mut fields_by_id = BTreeMap::new();
    let mut subject_fields = 0usize;

    for field in &descriptor.fields {
        field.validate()?;
        if fields_by_id.insert(field.field_id.clone(), field).is_some() {
            return Err(FactError::descriptor(format!(
                "duplicate field id {}",
                field.field_id
            )));
        }

        if matches!(field.extraction, FactFieldExtraction::Subject(_)) {
            subject_fields += 1;
            if !field.required {
                return Err(FactError::field(
                    field.field_id.clone(),
                    "subject fields must be required in v1",
                ));
            }
        }
    }

    if subject_fields == 0 {
        return Err(FactError::descriptor(
            "descriptor must contain at least one subject field",
        ));
    }

    let mut ordering_names = BTreeSet::new();
    for ordering in &descriptor.orderings {
        if !ordering_names.insert(ordering.name.clone()) {
            return Err(FactError::descriptor(format!(
                "duplicate ordering name {}",
                ordering.name
            )));
        }
        validate_ordering(ordering, &fields_by_id)?;
    }

    Ok(())
}

/// Returns the schema id for fact descriptor artifacts.
pub fn fact_descriptor_schema_id() -> Result<SchemaId> {
    facts_schema_id("mfm.fact_descriptor", FACTS_KERNEL_CONTRACT_VERSION)
}

/// Returns the schema id for canonical fact query evidence artifacts.
pub fn fact_query_evidence_schema_id() -> Result<SchemaId> {
    facts_schema_id(
        "mfm.fact_query_evidence",
        FACT_QUERY_EVIDENCE_CONTRACT_VERSION,
    )
}

fn facts_schema_id(name: &'static str, version: &'static str) -> Result<SchemaId> {
    SchemaId::new(
        name,
        version,
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(format!("schema:{name}:{version}").as_bytes()),
    )
    .map_err(|error| FactError::descriptor(error.to_string()))
}

/// Returns canonical descriptor bytes.
pub fn canonical_fact_descriptor_bytes(descriptor: &FactDescriptor) -> Result<CanonicalJsonBytes> {
    validate_descriptor(descriptor)?;
    canonical_descriptor_value(descriptor).map(|value| CanonicalJsonBytes::from_value(&value))
}

/// Decodes and validates canonical fact descriptor artifact bytes.
pub fn parse_canonical_fact_descriptor_bytes(bytes: &[u8]) -> Result<FactDescriptor> {
    let descriptor = serde_json::from_slice::<FactDescriptor>(bytes)
        .map_err(|error| FactError::descriptor(error.to_string()))?;
    let canonical = canonical_fact_descriptor_bytes(&descriptor)?;
    if canonical.as_bytes() != bytes {
        return Err(FactError::descriptor(
            "fact descriptor bytes are not canonical",
        ));
    }
    Ok(descriptor)
}

/// Derives the content digest for a fact descriptor.
pub fn fact_descriptor_hash(descriptor: &FactDescriptor) -> Result<ContentDigest> {
    Ok(canonical_fact_descriptor_bytes(descriptor)?.content_digest())
}

fn fact_subject_namespace(descriptor: &FactDescriptor) -> Result<FactSubjectNamespaceV1> {
    validate_descriptor(descriptor)?;
    let fields = descriptor
        .fields
        .iter()
        .filter(|field| matches!(field.extraction, FactFieldExtraction::Subject(_)))
        .map(|field| {
            FactSubjectNamespaceFieldV1::new(
                field.field_id.clone(),
                field.value_type,
                field.unit.clone(),
                field.scale,
            )
        })
        .collect();
    FactSubjectNamespaceV1::new(descriptor.fact_kind.clone(), fields)
}

fn canonical_fact_subject_namespace_bytes(
    namespace: &FactSubjectNamespaceV1,
) -> Result<CanonicalJsonBytes> {
    canonical_subject_namespace_value(namespace).map(|value| CanonicalJsonBytes::from_value(&value))
}

fn subject_namespace_hash(namespace: &FactSubjectNamespaceV1) -> Result<ContentDigest> {
    Ok(canonical_fact_subject_namespace_bytes(namespace)?.content_digest())
}

/// Derives the content digest for a descriptor's canonical subject namespace.
pub fn fact_subject_namespace_hash(descriptor: &FactDescriptor) -> Result<ContentDigest> {
    let namespace = fact_subject_namespace(descriptor)?;
    subject_namespace_hash(&namespace)
}

/// Returns canonical subject material bytes.
pub fn canonical_fact_subject_material_bytes(
    material: &FactSubjectMaterialV1,
) -> Result<CanonicalJsonBytes> {
    canonical_subject_material_value(material).map(|value| CanonicalJsonBytes::from_value(&value))
}

/// Decodes and validates canonical subject material bytes.
pub fn parse_canonical_fact_subject_material_bytes(bytes: &[u8]) -> Result<FactSubjectMaterialV1> {
    let value = serde_json::from_slice::<serde_json::Value>(bytes)
        .map_err(|error| FactError::canonical(error.to_string()))?;
    let object = value
        .as_object()
        .ok_or_else(|| FactError::descriptor("subject material must be an object"))?;
    let version = object
        .get("version")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| FactError::descriptor("subject material version is required"))?;
    if version != FactSubjectMaterialV1::VERSION {
        return Err(FactError::descriptor(format!(
            "unsupported subject material version {version:?}"
        )));
    }
    let values = object
        .get("values")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| FactError::descriptor("subject material values are required"))?
        .iter()
        .map(|value| parse_fact_field_value(value, "subject material value"))
        .collect::<Result<Vec<_>>>()?;
    let material = FactSubjectMaterialV1::new(values)?;
    let canonical = canonical_fact_subject_material_bytes(&material)?;
    if canonical.as_bytes() != bytes {
        return Err(FactError::descriptor(
            "subject material bytes are not canonical",
        ));
    }
    Ok(material)
}

/// Derives the content digest for subject material.
pub fn subject_material_hash(material: &FactSubjectMaterialV1) -> Result<ContentDigest> {
    Ok(canonical_fact_subject_material_bytes(material)?.content_digest())
}

/// Derives a fact key from a subject namespace hash and subject material hash.
pub fn derive_fact_key(
    fact_subject_namespace_hash: ContentDigest,
    subject_material_hash: ContentDigest,
) -> Result<FactKey> {
    let value = canonical_object([
        (
            "version",
            CanonicalValue::String("mfm.fact-key.v1".to_owned()),
        ),
        (
            "fact_subject_namespace_hash",
            CanonicalValue::String(fact_subject_namespace_hash.as_str().to_owned()),
        ),
        (
            "subject_material_hash",
            CanonicalValue::String(subject_material_hash.as_str().to_owned()),
        ),
    ])?;
    let bytes = CanonicalJsonBytes::from_value(&value);
    Ok(FactKey::from_digest(ContentDigest::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(bytes.as_bytes()),
    )))
}

/// Derives a fact claim id from run-stream coordinates.
pub fn derive_fact_claim_id(
    source_run_id: RunId,
    source_seq: u64,
    source_ordinal: u32,
) -> Result<FactClaimId> {
    FactClaimId::new(source_run_id, source_seq, source_ordinal)
}

/// Returns canonical fact claim id bytes.
pub fn canonical_fact_claim_id_bytes(claim_id: &FactClaimId) -> Result<CanonicalJsonBytes> {
    let value = canonical_fact_claim_id_value(claim_id)?;
    Ok(CanonicalJsonBytes::from_value(&value))
}

/// Returns canonical query plan bytes.
pub fn canonical_fact_query_plan_bytes(
    plan: &CanonicalFactQueryPlan,
) -> Result<CanonicalJsonBytes> {
    canonical_query_plan_value(plan).map(|value| CanonicalJsonBytes::from_value(&value))
}

/// Derives the content digest for a canonical query plan.
pub fn fact_query_plan_hash(plan: &CanonicalFactQueryPlan) -> Result<ContentDigest> {
    Ok(canonical_fact_query_plan_bytes(plan)?.content_digest())
}

/// Compiles a descriptor-scoped query input into a canonical fact query plan.
pub fn compile_fact_query_plan(
    descriptor: &FactDescriptor,
    input: FactQueryInput,
) -> Result<CanonicalFactQueryPlan> {
    validate_descriptor(descriptor)?;
    let descriptor_hash = fact_descriptor_hash(descriptor)?;
    let fields_by_id = descriptor_fields_by_id(descriptor)?;
    let ordering_descriptor = descriptor
        .orderings()
        .iter()
        .find(|ordering| ordering.name() == input.ordering())
        .ok_or_else(|| {
            FactError::ordering(
                input.ordering().clone(),
                "fact query ordering is not declared by descriptor",
            )
        })?;

    let mut predicates = input.predicates().to_vec();
    predicates.sort();
    for predicate in &predicates {
        let field = required_query_field(&fields_by_id, predicate.field_id())?;
        require_query_exposed(field, "predicate")?;
        if !field.operators().contains(&predicate.operator()) {
            return Err(FactError::field(
                predicate.field_id().clone(),
                format!(
                    "operator {:?} is not declared for field {}",
                    predicate.operator(),
                    predicate.field_id()
                ),
            ));
        }
        validate_extracted_scalar(field, predicate.value())?;
    }

    for return_field in input.return_fields() {
        let field = required_query_field(&fields_by_id, return_field)?;
        if field.exposure() != FactFieldExposure::Returnable {
            return Err(FactError::field(
                return_field.clone(),
                "field is not returnable by descriptor exposure policy",
            ));
        }
    }

    for term in ordering_descriptor.terms() {
        let field = required_query_field(&fields_by_id, term.field_id())?;
        require_query_exposed(field, "ordering")?;
    }

    let ordering = ordering_descriptor.clone();
    let canonical_query = CompiledQueryWire::from_parts(
        descriptor,
        &descriptor_hash,
        &predicates,
        input.return_fields(),
        ordering.name(),
        input.limit(),
    )
    .canonical_bytes()?;
    CanonicalFactQueryPlan::new(
        input.store_scope,
        input.query_scope,
        FactQueryCompilerVersion::new(FACT_QUERY_COMPILER_VERSION)?,
        FactCanonicalizerVersion::new(FACT_QUERY_CANONICALIZER_VERSION)?,
        descriptor_hash,
        input.scope_decision_evidence,
        canonical_query,
        ordering,
        input.limit,
    )
}

/// Parses and validates the canonical query shape embedded in a query plan.
pub fn parse_canonical_fact_query_shape(
    plan: &CanonicalFactQueryPlan,
) -> Result<CompiledFactQueryShape> {
    if plan.canonical_query().content_digest() != *plan.canonical_query_hash() {
        return Err(FactError::descriptor(
            "fact query plan canonical query hash mismatch",
        ));
    }
    let value = serde_json::from_slice::<serde_json::Value>(plan.canonical_query().as_bytes())
        .map_err(|error| FactError::canonical(error.to_string()))?;
    let query = CompiledQueryWire::parse(&value)?;
    query.validate_plan(plan)?;
    query.into_shape()
}

/// Parses canonical fact-query evidence bytes into the typed facts-kernel contract.
pub fn parse_canonical_fact_query_evidence_bytes(bytes: &[u8]) -> Result<FactQueryEvidence> {
    PlainCanonicalJsonBytes::from_canonical_json_slice(bytes)
        .map_err(|error| FactError::canonical(error.to_string()))?;
    let value = serde_json::from_slice::<serde_json::Value>(bytes)
        .map_err(|error| FactError::canonical(error.to_string()))?;
    let object = json_object(&value, "fact query evidence")?;
    require_version(
        object,
        FACT_QUERY_EVIDENCE_CONTRACT_VERSION,
        "fact query evidence",
    )?;
    let plan = parse_canonical_fact_query_plan(json_required(object, "plan")?)?;
    let plan_hash = fact_query_plan_hash(&plan)?;
    let receipt = parse_fact_query_receipt(json_required(object, "receipt")?, &plan_hash)?;
    let selection = parse_fact_selection_evidence(json_required(object, "selection")?)?;
    let evidence = FactQueryEvidence::new(plan, receipt, selection);
    validate_fact_query_evidence(&evidence)?;
    Ok(evidence)
}

/// Derives the result-set digest for the rows and summaries pinned in a receipt.
pub fn fact_query_result_set_digest(
    returned_refs: &[InternalFactRef],
    returned_field_summaries: Option<&ReturnedFieldSummaries>,
) -> Result<ContentDigest> {
    canonical_fact_query_result_set_value(returned_refs, returned_field_summaries)
        .map(|value| CanonicalJsonBytes::from_value(&value).content_digest())
}

/// Returns canonical query evidence bytes.
pub fn canonical_fact_query_evidence_bytes(
    evidence: &FactQueryEvidence,
) -> Result<CanonicalJsonBytes> {
    canonical_query_evidence_value(evidence).map(|value| CanonicalJsonBytes::from_value(&value))
}

/// Derives the content digest for fact query evidence.
pub fn fact_query_evidence_hash(evidence: &FactQueryEvidence) -> Result<ContentDigest> {
    Ok(canonical_fact_query_evidence_bytes(evidence)?.content_digest())
}

/// Validates fact-query replay evidence before it can be recorded or replayed.
///
/// This is facts-kernel structural validation only; the evidence artifact's content digest and
/// retained authority bindings provide its integrity.
pub fn validate_fact_query_evidence(evidence: &FactQueryEvidence) -> Result<()> {
    let receipt = evidence.receipt();

    if receipt.read_frontier().store_scope() != evidence.plan().store_scope() {
        return Err(FactError::descriptor(
            "fact query receipt frontier store scope does not match plan",
        ));
    }
    if receipt.read_frontier().query_scope() != evidence.plan().query_scope() {
        return Err(FactError::descriptor(
            "fact query receipt frontier query scope does not match plan",
        ));
    }

    let expected_result_set_digest =
        fact_query_result_set_digest(receipt.returned_refs(), receipt.returned_field_summaries())?;
    if &expected_result_set_digest != receipt.result_set_digest() {
        return Err(FactError::descriptor(
            "fact query receipt result-set digest does not match returned refs and summaries",
        ));
    }

    validate_returned_summaries(receipt)?;
    validate_fact_selection_evidence(receipt, evidence.selection())
}

fn validate_returned_summaries(receipt: &FactQueryReceipt) -> Result<()> {
    let Some(summaries) = receipt.returned_field_summaries() else {
        return Ok(());
    };
    if summaries.summaries().len() != receipt.returned_refs().len() {
        return Err(FactError::descriptor(
            "returned field summaries must align one-for-one with returned refs",
        ));
    }
    for (summary, fact_ref) in summaries.summaries().iter().zip(receipt.returned_refs()) {
        if summary.fact_claim_id() != fact_ref.fact_claim_id() {
            return Err(FactError::descriptor(
                "returned field summary claim id does not match returned ref",
            ));
        }
    }
    Ok(())
}

fn validate_fact_selection_evidence(
    receipt: &FactQueryReceipt,
    selection: &FactSelectionEvidence,
) -> Result<()> {
    let returned_len = receipt.returned_refs().len() as u64;
    for window in selection.selected_indices().windows(2) {
        if window[0] >= window[1] {
            return Err(FactError::descriptor(
                "selected indices must be sorted and unique",
            ));
        }
    }
    for index in selection.selected_indices() {
        if *index >= returned_len {
            return Err(FactError::descriptor(
                "selected index is outside returned refs",
            ));
        }
    }

    if let Some(selected_digest) = selection.selected_summaries_digest() {
        let summaries = receipt.returned_field_summaries().ok_or_else(|| {
            FactError::descriptor("selected summaries digest requires returned field summaries")
        })?;
        let expected =
            selected_returned_field_summaries_digest(summaries, selection.selected_indices())?;
        if &expected != selected_digest {
            return Err(FactError::descriptor(
                "selected summaries digest does not match selected returned summaries",
            ));
        }
    }
    Ok(())
}

/// Derives a digest over the returned field summaries selected by receipt index.
pub fn selected_returned_field_summaries_digest(
    returned_field_summaries: &ReturnedFieldSummaries,
    selected_indices: &[u64],
) -> Result<ContentDigest> {
    let mut selected = Vec::with_capacity(selected_indices.len());
    for index in selected_indices {
        let index = usize::try_from(*index)
            .map_err(|_| FactError::descriptor("selected summary index overflows usize"))?;
        let summary = returned_field_summaries
            .summaries()
            .get(index)
            .ok_or_else(|| FactError::descriptor("selected summary index is outside summaries"))?;
        selected.push(canonical_returned_fact_field_summary_value(summary)?);
    }
    let value = canonical_object([
        (
            "version",
            CanonicalValue::String("mfm.fact-query-selected-summaries.v1".to_owned()),
        ),
        ("summaries", CanonicalValue::Array(selected)),
    ])?;
    Ok(CanonicalJsonBytes::from_value(&value).content_digest())
}

/// Extracts canonical subject material using the descriptor's subject fields.
pub fn extract_subject_material(
    descriptor: &FactDescriptor,
    subject: &CanonicalValue,
) -> Result<FactSubjectMaterialV1> {
    validate_descriptor(descriptor)?;
    let mut values = Vec::new();
    for field in &descriptor.fields {
        if !matches!(field.extraction, FactFieldExtraction::Subject(_)) {
            continue;
        }
        let scalar = extract_field_scalar(field, subject, &CanonicalValue::Null, None)?
            .ok_or_else(|| {
                FactError::field(field.field_id.clone(), "required subject field missing")
            })?;
        validate_extracted_scalar(field, &scalar)?;
        values.push(FactFieldValue::new(
            field.field_id.clone(),
            field.value_type,
            scalar,
        )?);
    }
    FactSubjectMaterialV1::new(values)
}

/// Builds descriptor-derived subject evidence from an already-canonical subject value.
pub fn fact_subject_evidence(
    descriptor: &FactDescriptor,
    subject: &CanonicalValue,
) -> Result<FactSubjectEvidence> {
    let subject_material = extract_subject_material(descriptor, subject)?;
    fact_subject_evidence_from_material(descriptor, &subject_material)
}

/// Builds descriptor-derived subject evidence from already-extracted subject material.
pub fn fact_subject_evidence_from_material(
    descriptor: &FactDescriptor,
    subject_material: &FactSubjectMaterialV1,
) -> Result<FactSubjectEvidence> {
    let subject_namespace = fact_subject_namespace(descriptor)?;
    let subject_namespace_hash = subject_namespace_hash(&subject_namespace)?;
    FactSubjectEvidence::from_material(subject_namespace_hash, subject_material)
}

/// Normalizes a typed fact subject JSON value using descriptor-declared subject scalar types.
///
/// This preserves non-field JSON structure while converting descriptor-declared subject paths to
/// canonical scalar values. Null is rejected at typed subject paths because subject material only
/// admits concrete scalar values.
pub fn typed_fact_subject_value(
    descriptor: &FactDescriptor,
    subject: &serde_json::Value,
) -> Result<CanonicalValue> {
    validate_descriptor(descriptor)?;
    let typed_paths = subject_typed_paths(descriptor)?;
    json_to_fact_canonical_value(subject, "", &typed_paths, FactJsonPathContext::Subject)
}

/// Builds descriptor-derived subject evidence from a typed fact subject JSON value.
pub fn typed_fact_subject_evidence(
    descriptor: &FactDescriptor,
    subject: &serde_json::Value,
) -> Result<FactSubjectEvidence> {
    let subject = typed_fact_subject_value(descriptor, subject)?;
    fact_subject_evidence(descriptor, &subject)
}

/// Extracts subject, result, and metadata terms for an indexed fact claim.
pub fn extract_terms(
    descriptor: &FactDescriptor,
    subject: &CanonicalValue,
    response: &CanonicalValue,
    metadata: &FactExtractionMetadata,
) -> Result<Vec<FactIndexTerm>> {
    validate_descriptor(descriptor)?;
    let mut terms = Vec::new();
    for field in &descriptor.fields {
        match extract_field_scalar(field, subject, response, Some(metadata))? {
            Some(scalar) => terms.push(FactIndexTerm::from_field(field, scalar)?),
            None if field.required => {
                return Err(FactError::field(
                    field.field_id.clone(),
                    "required field missing",
                ));
            }
            None => {}
        }
    }
    Ok(terms)
}

/// Decodes canonical fact response bytes using descriptor-declared response scalar types.
pub fn parse_canonical_fact_response_bytes(
    descriptor: &FactDescriptor,
    bytes: &[u8],
) -> Result<CanonicalValue> {
    validate_descriptor(descriptor)?;
    PlainCanonicalJsonBytes::from_canonical_json_slice(bytes)
        .map_err(|error| FactError::canonical(error.to_string()))?;
    let value = serde_json::from_slice::<serde_json::Value>(bytes)
        .map_err(|error| FactError::canonical(error.to_string()))?;
    let typed_paths = response_typed_paths(descriptor)?;
    json_to_fact_canonical_value(&value, "", &typed_paths, FactJsonPathContext::Response)
}

/// Extracts index terms from persisted subject material plus canonical response material.
pub fn extract_terms_from_material(
    descriptor: &FactDescriptor,
    subject_material: &FactSubjectMaterialV1,
    response: &CanonicalValue,
    metadata: &FactExtractionMetadata,
) -> Result<Vec<FactIndexTerm>> {
    validate_descriptor(descriptor)?;
    let subject_values = subject_material
        .values()
        .iter()
        .map(|value| (value.field_id().clone(), value))
        .collect::<BTreeMap<_, _>>();
    let mut terms = Vec::new();
    for field in &descriptor.fields {
        let scalar = match field.extraction() {
            FactFieldExtraction::Subject(_) => {
                let Some(value) = subject_values.get(field.field_id()) else {
                    if field.required() {
                        return Err(FactError::field(
                            field.field_id.clone(),
                            "required subject field missing",
                        ));
                    }
                    continue;
                };
                if value.value_type() != field.value_type() {
                    return Err(FactError::field(
                        field.field_id.clone(),
                        "subject material value type does not match descriptor field",
                    ));
                }
                Some(value.value().clone())
            }
            FactFieldExtraction::Response(_) | FactFieldExtraction::Metadata(_) => {
                extract_field_scalar(field, &CanonicalValue::Null, response, Some(metadata))?
            }
        };
        match scalar {
            Some(scalar) => terms.push(FactIndexTerm::from_field(field, scalar)?),
            None if field.required() => {
                return Err(FactError::field(
                    field.field_id.clone(),
                    "required field missing",
                ));
            }
            None => {}
        }
    }
    Ok(terms)
}

pub(crate) fn canonical_object(
    entries: impl IntoIterator<Item = (impl Into<String>, CanonicalValue)>,
) -> Result<CanonicalValue> {
    CanonicalValue::object(entries).map_err(|error| FactError::canonical(error.to_string()))
}
