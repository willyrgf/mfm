use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::str::FromStr;

use mfm_canonical::{
    sha256_digest_bytes, CanonicalBytes, CanonicalJsonBytes, CanonicalValue,
    PlainCanonicalJsonBytes,
};
use mfm_ids::{ContentDigest, DigestAlgorithm, RunId, SchemaId};

use crate::scalar::ScalarJsonContext;
use crate::*;

/// Validates a fact descriptor.
pub fn validate_descriptor(descriptor: &FactDescriptor) -> Result<()> {
    let mut fields_by_id = BTreeMap::new();
    let mut subject_fields = 0usize;

    for field in &descriptor.fields {
        field.validate()?;
        if fields_by_id.insert(field.field_id.clone(), field).is_some() {
            return Err(FactDescriptorError::descriptor(format!(
                "duplicate field id {}",
                field.field_id
            )));
        }

        if matches!(field.extraction, FactFieldExtraction::SubjectPath(_)) {
            subject_fields += 1;
            if !field.required {
                return Err(FactDescriptorError::field(
                    field.field_id.clone(),
                    "subject fields must be required in v1",
                ));
            }
        }
    }

    if subject_fields == 0 {
        return Err(FactDescriptorError::descriptor(
            "descriptor must contain at least one subject field",
        ));
    }

    let mut ordering_names = BTreeSet::new();
    for ordering in &descriptor.orderings {
        if !ordering_names.insert(ordering.name.clone()) {
            return Err(FactDescriptorError::descriptor(format!(
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
    SchemaId::new(
        "mfm.fact_descriptor",
        FACTS_KERNEL_CONTRACT_VERSION,
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(
            format!("schema:mfm.fact_descriptor:{FACTS_KERNEL_CONTRACT_VERSION}").as_bytes(),
        ),
    )
    .map_err(|error| FactDescriptorError::descriptor(error.to_string()))
}

/// Returns the schema id for canonical fact query evidence artifacts.
pub fn fact_query_evidence_schema_id() -> Result<SchemaId> {
    SchemaId::new(
        "mfm.fact_query_evidence",
        FACTS_KERNEL_CONTRACT_VERSION,
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(
            format!("schema:mfm.fact_query_evidence:{FACTS_KERNEL_CONTRACT_VERSION}").as_bytes(),
        ),
    )
    .map_err(|error| FactDescriptorError::descriptor(error.to_string()))
}

/// Returns canonical descriptor bytes.
pub fn canonical_fact_descriptor_bytes(descriptor: &FactDescriptor) -> Result<CanonicalJsonBytes> {
    validate_descriptor(descriptor)?;
    canonical_descriptor_value(descriptor).map(|value| CanonicalJsonBytes::from_value(&value))
}

/// Decodes and validates canonical fact descriptor artifact bytes.
pub fn parse_canonical_fact_descriptor_bytes(bytes: &[u8]) -> Result<FactDescriptor> {
    let descriptor = serde_json::from_slice::<FactDescriptor>(bytes)
        .map_err(|error| FactDescriptorError::descriptor(error.to_string()))?;
    let canonical = canonical_fact_descriptor_bytes(&descriptor)?;
    if canonical.as_bytes() != bytes {
        return Err(FactDescriptorError::descriptor(
            "fact descriptor bytes are not canonical",
        ));
    }
    Ok(descriptor)
}

/// Derives the content digest for a fact descriptor.
pub fn fact_descriptor_hash(descriptor: &FactDescriptor) -> Result<ContentDigest> {
    Ok(canonical_fact_descriptor_bytes(descriptor)?.content_digest())
}

/// Builds the subject namespace for a validated fact descriptor.
pub fn fact_subject_namespace(descriptor: &FactDescriptor) -> Result<FactSubjectNamespaceV1> {
    validate_descriptor(descriptor)?;
    let fields = descriptor
        .fields
        .iter()
        .filter(|field| matches!(field.extraction, FactFieldExtraction::SubjectPath(_)))
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

/// Returns canonical subject namespace bytes.
pub fn canonical_fact_subject_namespace_bytes(
    namespace: &FactSubjectNamespaceV1,
) -> Result<CanonicalJsonBytes> {
    canonical_subject_namespace_value(namespace).map(|value| CanonicalJsonBytes::from_value(&value))
}

/// Derives the content digest for a subject namespace.
pub fn fact_subject_namespace_hash(namespace: &FactSubjectNamespaceV1) -> Result<ContentDigest> {
    Ok(canonical_fact_subject_namespace_bytes(namespace)?.content_digest())
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
        .map_err(|error| FactDescriptorError::canonical(error.to_string()))?;
    let object = value
        .as_object()
        .ok_or_else(|| FactDescriptorError::descriptor("subject material must be an object"))?;
    let version = object
        .get("version")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| FactDescriptorError::descriptor("subject material version is required"))?;
    if version != FactSubjectMaterialV1::VERSION {
        return Err(FactDescriptorError::descriptor(format!(
            "unsupported subject material version {version:?}"
        )));
    }
    let values = object
        .get("values")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| FactDescriptorError::descriptor("subject material values are required"))?
        .iter()
        .map(parse_subject_material_value)
        .collect::<Result<Vec<_>>>()?;
    let material = FactSubjectMaterialV1::new(values)?;
    let canonical = canonical_fact_subject_material_bytes(&material)?;
    if canonical.as_bytes() != bytes {
        return Err(FactDescriptorError::descriptor(
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
    let value = canonical_object([
        (
            "version",
            CanonicalValue::String("mfm.fact-claim-id.v1".to_owned()),
        ),
        (
            "source_run_id",
            CanonicalValue::String(claim_id.source_run_id.as_str().to_owned()),
        ),
        ("source_seq", CanonicalValue::Unsigned(claim_id.source_seq)),
        (
            "source_ordinal",
            CanonicalValue::Unsigned(u64::from(claim_id.source_ordinal)),
        ),
    ])?;
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
            FactDescriptorError::ordering(
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
            return Err(FactDescriptorError::field(
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
        let field = required_query_field(&fields_by_id, return_field.field_id())?;
        if field.exposure() != FactFieldExposure::Returnable {
            return Err(FactDescriptorError::field(
                return_field.field_id().clone(),
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
        return Err(FactDescriptorError::descriptor(
            "fact query plan canonical query hash mismatch",
        ));
    }
    let value = serde_json::from_slice::<serde_json::Value>(plan.canonical_query().as_bytes())
        .map_err(|error| FactDescriptorError::canonical(error.to_string()))?;
    let query = CompiledQueryWire::parse(&value)?;
    query.validate_plan(plan)?;
    query.into_shape()
}

/// Parses canonical fact-query evidence bytes into the typed facts-kernel contract.
pub fn parse_canonical_fact_query_evidence_bytes(bytes: &[u8]) -> Result<FactQueryEvidence> {
    PlainCanonicalJsonBytes::from_canonical_json_slice(bytes)
        .map_err(|error| FactDescriptorError::canonical(error.to_string()))?;
    let value = serde_json::from_slice::<serde_json::Value>(bytes)
        .map_err(|error| FactDescriptorError::canonical(error.to_string()))?;
    let object = json_object(&value, "fact query evidence")?;
    require_version(object, "mfm.fact-query-evidence.v1", "fact query evidence")?;
    let plan = parse_canonical_fact_query_plan(json_required(object, "plan")?)?;
    let plan_hash = fact_query_plan_hash(&plan)?;
    let receipt = parse_fact_query_receipt(json_required(object, "receipt")?, &plan_hash)?;
    let selection = parse_fact_selection_evidence(json_required(object, "selection")?)?;
    Ok(FactQueryEvidence::new(plan, receipt, selection))
}

/// Returns canonical receipt body bytes, excluding receipt hash and authentication fields.
pub fn canonical_fact_query_receipt_body_bytes(
    plan_hash: &ContentDigest,
    receipt: &FactQueryReceipt,
) -> Result<CanonicalJsonBytes> {
    FactQueryReceiptBodyParts::from_receipt(plan_hash, receipt).canonical_bytes()
}

/// Returns canonical result-set bytes for the rows and summaries pinned in a receipt.
pub fn canonical_fact_query_result_set_bytes(
    returned_refs: &[InternalFactRef],
    returned_field_summaries: Option<&ReturnedFieldSummaries>,
) -> Result<CanonicalJsonBytes> {
    canonical_fact_query_result_set_value(returned_refs, returned_field_summaries)
        .map(|value| CanonicalJsonBytes::from_value(&value))
}

/// Derives the result-set digest for the rows and summaries pinned in a receipt.
pub fn fact_query_result_set_digest(
    returned_refs: &[InternalFactRef],
    returned_field_summaries: Option<&ReturnedFieldSummaries>,
) -> Result<ContentDigest> {
    Ok(
        canonical_fact_query_result_set_bytes(returned_refs, returned_field_summaries)?
            .content_digest(),
    )
}

/// Returns canonical receipt body bytes from unsigned receipt parts.
#[allow(clippy::too_many_arguments)]
pub fn canonical_fact_query_receipt_body_bytes_from_parts(
    plan_hash: &ContentDigest,
    read_frontier: &StoreReadFrontier,
    frontier_type: StoreReadFrontierType,
    returned_refs: &[InternalFactRef],
    returned_field_summaries: Option<&ReturnedFieldSummaries>,
    result_set_digest: &ContentDigest,
    result_cardinality: QueryResultCardinality,
) -> Result<CanonicalJsonBytes> {
    FactQueryReceiptBodyParts::new(
        plan_hash,
        read_frontier,
        frontier_type,
        returned_refs,
        returned_field_summaries,
        result_set_digest,
        result_cardinality,
    )
    .canonical_bytes()
}

/// Derives the receipt body hash, excluding receipt hash and authentication fields.
pub fn fact_query_receipt_body_hash(
    plan_hash: &ContentDigest,
    receipt: &FactQueryReceipt,
) -> Result<ContentDigest> {
    Ok(canonical_fact_query_receipt_body_bytes(plan_hash, receipt)?.content_digest())
}

/// Derives the receipt body hash from unsigned receipt parts.
#[allow(clippy::too_many_arguments)]
pub fn fact_query_receipt_body_hash_from_parts(
    plan_hash: &ContentDigest,
    read_frontier: &StoreReadFrontier,
    frontier_type: StoreReadFrontierType,
    returned_refs: &[InternalFactRef],
    returned_field_summaries: Option<&ReturnedFieldSummaries>,
    result_set_digest: &ContentDigest,
    result_cardinality: QueryResultCardinality,
) -> Result<ContentDigest> {
    FactQueryReceiptBodyParts::new(
        plan_hash,
        read_frontier,
        frontier_type,
        returned_refs,
        returned_field_summaries,
        result_set_digest,
        result_cardinality,
    )
    .hash()
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
/// This is facts-kernel structural validation only. Store-owned receipt
/// authentication still needs a store trust root and is verified by `mfm-store`.
pub fn validate_fact_query_evidence(evidence: &FactQueryEvidence) -> Result<()> {
    let plan_hash = fact_query_plan_hash(evidence.plan())?;
    let receipt = evidence.receipt();

    if receipt.read_frontier().store_scope() != evidence.plan().store_scope() {
        return Err(FactDescriptorError::descriptor(
            "fact query receipt frontier store scope does not match plan",
        ));
    }
    if receipt.read_frontier().query_scope() != evidence.plan().query_scope() {
        return Err(FactDescriptorError::descriptor(
            "fact query receipt frontier query scope does not match plan",
        ));
    }

    let expected_result_set_digest =
        fact_query_result_set_digest(receipt.returned_refs(), receipt.returned_field_summaries())?;
    if &expected_result_set_digest != receipt.result_set_digest() {
        return Err(FactDescriptorError::descriptor(
            "fact query receipt result-set digest does not match returned refs and summaries",
        ));
    }

    let expected_receipt_hash = fact_query_receipt_body_hash(&plan_hash, receipt)?;
    if &expected_receipt_hash != receipt.store_receipt_hash() {
        return Err(FactDescriptorError::descriptor(
            "fact query receipt body hash does not match store receipt hash",
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
        return Err(FactDescriptorError::descriptor(
            "returned field summaries must align one-for-one with returned refs",
        ));
    }
    for (summary, fact_ref) in summaries.summaries().iter().zip(receipt.returned_refs()) {
        if summary.fact_claim_id() != fact_ref.fact_claim_id() {
            return Err(FactDescriptorError::descriptor(
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
            return Err(FactDescriptorError::descriptor(
                "selected indices must be sorted and unique",
            ));
        }
    }
    for index in selection.selected_indices() {
        if *index >= returned_len {
            return Err(FactDescriptorError::descriptor(
                "selected index is outside returned refs",
            ));
        }
    }

    if let Some(selected_digest) = selection.selected_summaries_digest() {
        let summaries = receipt.returned_field_summaries().ok_or_else(|| {
            FactDescriptorError::descriptor(
                "selected summaries digest requires returned field summaries",
            )
        })?;
        let expected =
            selected_returned_field_summaries_digest(summaries, selection.selected_indices())?;
        if &expected != selected_digest {
            return Err(FactDescriptorError::descriptor(
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
    Ok(canonical_selected_returned_field_summaries_bytes(
        returned_field_summaries,
        selected_indices,
    )?
    .content_digest())
}

/// Returns canonical bytes for the returned field summaries selected by receipt index.
pub fn canonical_selected_returned_field_summaries_bytes(
    returned_field_summaries: &ReturnedFieldSummaries,
    selected_indices: &[u64],
) -> Result<CanonicalJsonBytes> {
    let mut selected = Vec::with_capacity(selected_indices.len());
    for index in selected_indices {
        let index = usize::try_from(*index).map_err(|_| {
            FactDescriptorError::descriptor("selected summary index overflows usize")
        })?;
        let summary = returned_field_summaries
            .summaries()
            .get(index)
            .ok_or_else(|| {
                FactDescriptorError::descriptor("selected summary index is outside summaries")
            })?;
        selected.push(canonical_returned_fact_field_summary_value(summary)?);
    }
    let value = canonical_object([
        (
            "version",
            CanonicalValue::String("mfm.fact-query-selected-summaries.v1".to_owned()),
        ),
        ("summaries", CanonicalValue::Array(selected)),
    ])?;
    Ok(CanonicalJsonBytes::from_value(&value))
}

/// Extracts canonical subject material using the descriptor's subject fields.
pub fn extract_subject_material(
    descriptor: &FactDescriptor,
    subject: &CanonicalValue,
) -> Result<FactSubjectMaterialV1> {
    validate_descriptor(descriptor)?;
    let mut values = Vec::new();
    for field in &descriptor.fields {
        if !matches!(field.extraction, FactFieldExtraction::SubjectPath(_)) {
            continue;
        }
        let scalar = extract_field_scalar(field, subject, &CanonicalValue::Null, None)?
            .ok_or_else(|| {
                FactDescriptorError::field(field.field_id.clone(), "required subject field missing")
            })?;
        validate_extracted_scalar(field, &scalar)?;
        values.push(FactSubjectValueV1::new(
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
    let subject_namespace_hash = fact_subject_namespace_hash(&subject_namespace)?;
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
                return Err(FactDescriptorError::field(
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
        .map_err(|error| FactDescriptorError::canonical(error.to_string()))?;
    let value = serde_json::from_slice::<serde_json::Value>(bytes)
        .map_err(|error| FactDescriptorError::canonical(error.to_string()))?;
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
            FactFieldExtraction::SubjectPath(_) => {
                let Some(value) = subject_values.get(field.field_id()) else {
                    if field.required() {
                        return Err(FactDescriptorError::field(
                            field.field_id.clone(),
                            "required subject field missing",
                        ));
                    }
                    continue;
                };
                if value.value_type() != field.value_type() {
                    return Err(FactDescriptorError::field(
                        field.field_id.clone(),
                        "subject material value type does not match descriptor field",
                    ));
                }
                Some(value.value().clone())
            }
            FactFieldExtraction::ResponsePath(_) | FactFieldExtraction::Metadata(_) => {
                extract_field_scalar(field, &CanonicalValue::Null, response, Some(metadata))?
            }
        };
        match scalar {
            Some(scalar) => terms.push(FactIndexTerm::from_field(field, scalar)?),
            None if field.required() => {
                return Err(FactDescriptorError::field(
                    field.field_id.clone(),
                    "required field missing",
                ));
            }
            None => {}
        }
    }
    Ok(terms)
}

fn canonical_descriptor_value(descriptor: &FactDescriptor) -> Result<CanonicalValue> {
    let mut fields = descriptor.fields.iter().collect::<Vec<_>>();
    fields.sort_by(|left, right| left.field_id.cmp(&right.field_id));
    let fields = fields
        .into_iter()
        .map(canonical_field_descriptor_value)
        .collect::<Result<Vec<_>>>()?;

    let mut orderings = descriptor.orderings.iter().collect::<Vec<_>>();
    orderings.sort_by(|left, right| left.name.cmp(&right.name));
    let orderings = orderings
        .into_iter()
        .map(canonical_ordering_descriptor_value)
        .collect::<Result<Vec<_>>>()?;

    canonical_object([
        (
            "version",
            CanonicalValue::String(FACTS_KERNEL_CONTRACT_VERSION.to_owned()),
        ),
        (
            "fact_kind",
            CanonicalValue::String(descriptor.fact_kind.as_str().to_owned()),
        ),
        (
            "descriptor_schema_id",
            CanonicalValue::String(descriptor.descriptor_schema_id.as_str().to_owned()),
        ),
        (
            "subject_schema_id",
            CanonicalValue::String(descriptor.subject_schema_id.as_str().to_owned()),
        ),
        (
            "response_schema_id",
            CanonicalValue::String(descriptor.response_schema_id.as_str().to_owned()),
        ),
        ("fields", CanonicalValue::Array(fields)),
        ("orderings", CanonicalValue::Array(orderings)),
    ])
}

fn canonical_field_descriptor_value(field: &FactFieldDescriptor) -> Result<CanonicalValue> {
    let mut operators = field.operators.clone();
    operators.sort();
    let operators = operators
        .into_iter()
        .map(|operator| CanonicalValue::String(operator.as_str().to_owned()))
        .collect::<Vec<_>>();

    canonical_object([
        (
            "field_id",
            CanonicalValue::String(field.field_id.as_str().to_owned()),
        ),
        (
            "path",
            CanonicalValue::String(field.path.as_str().to_owned()),
        ),
        (
            "value_type",
            CanonicalValue::String(field.value_type.as_str().to_owned()),
        ),
        ("extraction", canonical_extraction_value(&field.extraction)?),
        ("operators", CanonicalValue::Array(operators)),
        (
            "exposure",
            CanonicalValue::String(field.exposure.as_str().to_owned()),
        ),
        ("unit", optional_checked_string_value(field.unit.as_ref())),
        ("scale", optional_scale_value(field.scale)),
        ("sortable", CanonicalValue::Bool(field.sortable)),
        ("required", CanonicalValue::Bool(field.required)),
    ])
}

fn canonical_extraction_value(extraction: &FactFieldExtraction) -> Result<CanonicalValue> {
    match extraction {
        FactFieldExtraction::SubjectPath(path) => canonical_object([
            (
                "source",
                CanonicalValue::String(FactFieldSource::Subject.as_str().to_owned()),
            ),
            ("path", CanonicalValue::String(path.as_str().to_owned())),
        ]),
        FactFieldExtraction::ResponsePath(path) => canonical_object([
            (
                "source",
                CanonicalValue::String(FactFieldSource::Result.as_str().to_owned()),
            ),
            ("path", CanonicalValue::String(path.as_str().to_owned())),
        ]),
        FactFieldExtraction::Metadata(field) => canonical_object([
            (
                "source",
                CanonicalValue::String(FactFieldSource::Metadata.as_str().to_owned()),
            ),
            ("path", CanonicalValue::String(field.as_str().to_owned())),
        ]),
    }
}

fn canonical_ordering_descriptor_value(ordering: &FactOrderingPolicy) -> Result<CanonicalValue> {
    let terms = ordering
        .terms
        .iter()
        .map(OrderingTermWire::canonical_value)
        .collect::<Result<Vec<_>>>()?;
    canonical_object([
        (
            "name",
            CanonicalValue::String(ordering.name.as_str().to_owned()),
        ),
        ("terms", CanonicalValue::Array(terms)),
    ])
}

fn canonical_subject_namespace_value(namespace: &FactSubjectNamespaceV1) -> Result<CanonicalValue> {
    let fields = namespace
        .fields
        .iter()
        .map(canonical_subject_namespace_field_value)
        .collect::<Result<Vec<_>>>()?;
    canonical_object([
        (
            "version",
            CanonicalValue::String(FactSubjectNamespaceV1::VERSION.to_owned()),
        ),
        (
            "fact_kind",
            CanonicalValue::String(namespace.fact_kind.as_str().to_owned()),
        ),
        ("fields", CanonicalValue::Array(fields)),
    ])
}

fn canonical_subject_namespace_field_value(
    field: &FactSubjectNamespaceFieldV1,
) -> Result<CanonicalValue> {
    canonical_object([
        (
            "field_id",
            CanonicalValue::String(field.field_id.as_str().to_owned()),
        ),
        (
            "value_type",
            CanonicalValue::String(field.value_type.as_str().to_owned()),
        ),
        ("unit", optional_checked_string_value(field.unit.as_ref())),
        ("scale", optional_scale_value(field.scale)),
    ])
}

fn canonical_subject_material_value(material: &FactSubjectMaterialV1) -> Result<CanonicalValue> {
    let values = material
        .values
        .iter()
        .map(canonical_subject_value)
        .collect::<Result<Vec<_>>>()?;
    canonical_object([
        (
            "version",
            CanonicalValue::String(FactSubjectMaterialV1::VERSION.to_owned()),
        ),
        ("values", CanonicalValue::Array(values)),
    ])
}

fn canonical_subject_value(value: &FactSubjectValueV1) -> Result<CanonicalValue> {
    canonical_object([
        (
            "field_id",
            CanonicalValue::String(value.field_id().as_str().to_owned()),
        ),
        (
            "value_type",
            CanonicalValue::String(value.value_type().as_str().to_owned()),
        ),
        ("value", value.value().canonical_value()),
    ])
}

fn parse_subject_material_value(value: &serde_json::Value) -> Result<FactSubjectValueV1> {
    let object = value.as_object().ok_or_else(|| {
        FactDescriptorError::descriptor("subject material value must be an object")
    })?;
    let field_id = object
        .get("field_id")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| FactDescriptorError::descriptor("subject material field_id is required"))
        .and_then(FactFieldId::new)?;
    let value_type = object
        .get("value_type")
        .ok_or_else(|| FactDescriptorError::descriptor("subject material value_type is required"))
        .and_then(parse_field_value_type)?;
    let scalar_value = object
        .get("value")
        .ok_or_else(|| FactDescriptorError::descriptor("subject material value is required"))?;
    let scalar = parse_canonical_scalar_value(value_type, scalar_value, &field_id)?;
    FactSubjectValueV1::new(field_id, value_type, scalar)
}

fn parse_field_value_type(value: &serde_json::Value) -> Result<FactFieldValueType> {
    let value = value
        .as_str()
        .ok_or_else(|| FactDescriptorError::descriptor("field value type must be a string"))?;
    value
        .parse::<FactFieldValueType>()
        .map_err(|_| FactDescriptorError::descriptor(format!("unknown field value type {value:?}")))
}

pub(crate) fn parse_canonical_scalar_value(
    value_type: FactFieldValueType,
    value: &serde_json::Value,
    field_id: &FactFieldId,
) -> Result<FactCanonicalScalar> {
    FactCanonicalScalar::from_json_value(value_type, value, ScalarJsonContext::Field(field_id))
}

fn parse_query_operator(field_id: &FactFieldId, value: &str) -> Result<FactQueryOperator> {
    value.parse::<FactQueryOperator>().map_err(|_| {
        FactDescriptorError::field(
            field_id.clone(),
            format!("unknown query operator {value:?}"),
        )
    })
}

fn parse_canonical_fact_query_plan(value: &serde_json::Value) -> Result<CanonicalFactQueryPlan> {
    let object = json_object(value, "fact query plan")?;
    require_version(object, "mfm.fact-query-plan.v1", "fact query plan")?;
    let canonical_query =
        plain_canonical_json_bytes_from_canonical_json_str(json_str(object, "canonical_query")?)?;
    let expected_query_hash: ContentDigest = parse_json_str(object, "canonical_query_hash")?;
    let query_hash = canonical_query.content_digest();
    if query_hash != expected_query_hash {
        return Err(FactDescriptorError::descriptor(
            "fact query plan canonical query hash mismatch",
        ));
    }
    let plan = CanonicalFactQueryPlan::new(
        StoreScopeRef::new(json_str(object, "store_scope")?)?,
        QueryScopeWire::parse(json_required(object, "query_scope")?)?,
        FactQueryCompilerVersion::new(json_str(object, "query_compiler_version")?)?,
        FactCanonicalizerVersion::new(json_str(object, "canonicalizer_version")?)?,
        parse_json_str(object, "resolved_descriptor")?,
        ScopeDecisionEvidenceWire::parse(json_required(object, "scope_decision_evidence")?)?,
        canonical_query,
        OrderingWire::parse(json_required(object, "ordering")?)?,
        json_optional_u64(object, "limit")?,
    )?;
    Ok(plan)
}

fn parse_fact_query_receipt(
    value: &serde_json::Value,
    plan_hash: &ContentDigest,
) -> Result<FactQueryReceipt> {
    let object = json_object(value, "fact query receipt")?;
    let body = OwnedFactQueryReceiptBodyParts::parse(json_required(object, "body")?, plan_hash)?;
    let store_receipt_hash = parse_json_str(object, "store_receipt_hash")?;
    let expected_hash = body.as_borrowed().hash()?;
    if expected_hash != store_receipt_hash {
        return Err(FactDescriptorError::descriptor(
            "fact query receipt body hash does not match store receipt hash",
        ));
    }
    let authentication =
        parse_store_receipt_authentication(json_required(object, "store_receipt_authentication")?)?;
    Ok(body.into_receipt(store_receipt_hash, authentication))
}

fn parse_fact_selection_evidence(value: &serde_json::Value) -> Result<FactSelectionEvidence> {
    let object = json_object(value, "fact selection evidence")?;
    let selected_indices = json_array(object, "selected_indices")?
        .iter()
        .map(|value| {
            value.as_u64().ok_or_else(|| {
                FactDescriptorError::descriptor("selected index must be an unsigned integer")
            })
        })
        .collect::<Result<Vec<_>>>()?;
    FactSelectionEvidence::new(
        parse_json_str(object, "selection_policy_hash")?,
        selected_indices,
        parse_optional_digest(json_required(object, "selected_summaries_digest")?)?,
    )
}

fn parse_store_read_frontier(value: &serde_json::Value) -> Result<StoreReadFrontier> {
    let object = json_object(value, "store read frontier")?;
    Ok(StoreReadFrontier::new(
        StoreScopeRef::new(json_str(object, "store_scope")?)?,
        QueryScopeWire::parse(json_required(object, "query_scope")?)?,
        DescriptorCatalogWatermark::new(json_u64(object, "descriptor_catalog_watermark")?),
        FactProjectionGeneration::new(json_u64(object, "projection_generation")?),
        json_u64(object, "max_included_store_commit_order")?,
        StoreCommitWatermark::new(json_u64(object, "commit_watermark")?),
    ))
}

fn parse_internal_fact_ref(value: &serde_json::Value) -> Result<InternalFactRef> {
    let object = json_object(value, "internal fact ref")?;
    InternalFactRef::new(InternalFactRefParts {
        fact_claim_id: parse_fact_claim_id(json_required(object, "fact_claim_id")?)?,
        source_event_id: parse_json_str(object, "source_event_id")?,
        recorded_at: json_str(object, "recorded_at")?.to_owned(),
        producer_node_id: parse_json_str(object, "producer_node_id")?,
        observed_at: parse_optional_string(json_required(object, "observed_at")?)?,
        visibility: parse_fact_visibility(json_required(object, "visibility")?)?,
        fact_kind: FactKind::new(json_str(object, "fact_kind")?)?,
        fact_descriptor_hash: parse_json_str(object, "fact_descriptor_hash")?,
        fact_subject_namespace_hash: parse_json_str(object, "fact_subject_namespace_hash")?,
        fact_key: FactKey::from_digest(parse_json_str(object, "fact_key")?),
        subject_material_hash: parse_json_str(object, "subject_material_hash")?,
        request_schema_id: parse_optional_identity(json_required(object, "request_schema_id")?)?,
        request_hash: parse_optional_digest(json_required(object, "request_hash")?)?,
        response_schema_id: parse_json_str(object, "response_schema_id")?,
        response_hash: parse_json_str(object, "response_hash")?,
        artifact_id: parse_json_str(object, "artifact_id")?,
        artifact_evidence_hash: parse_json_str(object, "artifact_evidence_hash")?,
        capability_kind: parse_json_str(object, "capability_kind")?,
        capability_version: parse_json_str(object, "capability_version")?,
        adapter_kind: parse_json_str(object, "adapter_kind")?,
        adapter_version: parse_json_str(object, "adapter_version")?,
    })
}

fn parse_fact_claim_id(value: &serde_json::Value) -> Result<FactClaimId> {
    let object = json_object(value, "fact claim id")?;
    require_version(object, "mfm.fact-claim-id.v1", "fact claim id")?;
    FactClaimId::new(
        parse_json_str(object, "source_run_id")?,
        json_u64(object, "source_seq")?,
        json_u32(object, "source_ordinal")?,
    )
}

fn parse_fact_visibility(value: &serde_json::Value) -> Result<FactVisibility> {
    let object = json_object(value, "fact visibility")?;
    match json_str(object, "kind")? {
        "run_private" => Ok(FactVisibility::RunPrivate),
        "indexed" => Ok(FactVisibility::Indexed {
            audience: parse_fact_audience(json_str(object, "audience")?)?,
            scope: parse_fact_visibility_scope(json_str(object, "scope")?)?,
        }),
        value => Err(FactDescriptorError::descriptor(format!(
            "unknown fact visibility kind {value:?}"
        ))),
    }
}

fn parse_optional_returned_field_summaries(
    value: &serde_json::Value,
) -> Result<Option<ReturnedFieldSummaries>> {
    if value.is_null() {
        return Ok(None);
    }
    let summaries = value
        .as_array()
        .ok_or_else(|| {
            FactDescriptorError::descriptor("returned field summaries must be an array")
        })?
        .iter()
        .map(parse_returned_fact_field_summary)
        .collect::<Result<Vec<_>>>()?;
    Ok(Some(ReturnedFieldSummaries::new(summaries)))
}

fn parse_returned_fact_field_summary(
    value: &serde_json::Value,
) -> Result<ReturnedFactFieldSummary> {
    let object = json_object(value, "returned fact field summary")?;
    let fields = json_array(object, "fields")?
        .iter()
        .map(parse_returned_field_value_summary)
        .collect::<Result<Vec<_>>>()?;
    Ok(ReturnedFactFieldSummary::new(
        parse_fact_claim_id(json_required(object, "fact_claim_id")?)?,
        fields,
    ))
}

fn parse_returned_field_value_summary(
    value: &serde_json::Value,
) -> Result<ReturnedFieldValueSummary> {
    let object = json_object(value, "returned field value summary")?;
    let field_id = FactFieldId::new(json_str(object, "field_id")?)?;
    let value_type = parse_field_value_type(json_required(object, "value_type")?)?;
    let scalar =
        parse_canonical_scalar_value(value_type, json_required(object, "value")?, &field_id)?;
    ReturnedFieldValueSummary::new(field_id, value_type, scalar)
}

fn parse_query_result_cardinality(value: &serde_json::Value) -> Result<QueryResultCardinality> {
    let object = json_object(value, "query result cardinality")?;
    match json_str(object, "kind")? {
        "exact" => Ok(QueryResultCardinality::Exact(json_u64(object, "value")?)),
        "at_least" => Ok(QueryResultCardinality::AtLeast(json_u64(object, "value")?)),
        value => Err(FactDescriptorError::descriptor(format!(
            "unknown query result cardinality kind {value:?}"
        ))),
    }
}

fn parse_store_receipt_authentication(
    value: &serde_json::Value,
) -> Result<StoreReceiptAuthentication> {
    let object = json_object(value, "store receipt authentication")?;
    let scheme = parse_store_receipt_authentication_scheme(json_str(object, "scheme")?)?;
    let key_id = parse_optional_checked_string::<StoreKeyId>(json_required(object, "key_id")?)?;
    let signature = CanonicalBytes::from_base64url_no_pad(json_str(object, "signature_or_mac")?)
        .map_err(|error| FactDescriptorError::canonical(error.to_string()))?;
    StoreReceiptAuthentication::new(
        StoreIdentity::new(json_str(object, "store_identity")?)?,
        scheme,
        key_id,
        signature.as_bytes().to_vec(),
    )
}

fn plain_canonical_json_bytes_from_canonical_json_str(
    value: &str,
) -> Result<PlainCanonicalJsonBytes> {
    PlainCanonicalJsonBytes::from_canonical_json_slice(value.as_bytes())
        .map_err(|error| FactDescriptorError::canonical(error.to_string()))
}

fn parse_fact_audience(value: &str) -> Result<FactAudience> {
    value
        .parse::<FactAudience>()
        .map_err(|_| FactDescriptorError::descriptor(format!("unknown fact audience {value:?}")))
}

fn parse_fact_visibility_scope(value: &str) -> Result<FactVisibilityScope> {
    value.parse::<FactVisibilityScope>().map_err(|_| {
        FactDescriptorError::descriptor(format!("unknown fact visibility scope {value:?}"))
    })
}

fn parse_store_read_frontier_type(value: &str) -> Result<StoreReadFrontierType> {
    value.parse::<StoreReadFrontierType>().map_err(|_| {
        FactDescriptorError::descriptor(format!("unknown store read frontier type {value:?}"))
    })
}

fn parse_store_receipt_authentication_scheme(
    value: &str,
) -> Result<StoreReceiptAuthenticationScheme> {
    value
        .parse::<StoreReceiptAuthenticationScheme>()
        .map_err(|_| {
            FactDescriptorError::descriptor(format!(
                "unknown store receipt authentication scheme {value:?}"
            ))
        })
}

fn parse_sort_direction(value: &str) -> Result<SortDirection> {
    value
        .parse::<SortDirection>()
        .map_err(|_| FactDescriptorError::descriptor(format!("unknown sort direction {value:?}")))
}

fn parse_null_ordering(value: &str) -> Result<NullOrdering> {
    value
        .parse::<NullOrdering>()
        .map_err(|_| FactDescriptorError::descriptor(format!("unknown null ordering {value:?}")))
}

fn json_object<'a>(
    value: &'a serde_json::Value,
    context: &'static str,
) -> Result<&'a serde_json::Map<String, serde_json::Value>> {
    value
        .as_object()
        .ok_or_else(|| FactDescriptorError::descriptor(format!("{context} must be an object")))
}

fn require_version(
    object: &serde_json::Map<String, serde_json::Value>,
    expected: &'static str,
    context: &'static str,
) -> Result<()> {
    let actual = json_str(object, "version")?;
    if actual == expected {
        Ok(())
    } else {
        Err(FactDescriptorError::descriptor(format!(
            "{context} version {actual:?} is unsupported"
        )))
    }
}

fn json_required<'a>(
    object: &'a serde_json::Map<String, serde_json::Value>,
    key: &'static str,
) -> Result<&'a serde_json::Value> {
    object
        .get(key)
        .ok_or_else(|| FactDescriptorError::descriptor(format!("{key} is required")))
}

fn json_str<'a>(
    object: &'a serde_json::Map<String, serde_json::Value>,
    key: &'static str,
) -> Result<&'a str> {
    json_required(object, key)?
        .as_str()
        .ok_or_else(|| FactDescriptorError::descriptor(format!("{key} must be a string")))
}

fn json_array<'a>(
    object: &'a serde_json::Map<String, serde_json::Value>,
    key: &'static str,
) -> Result<&'a [serde_json::Value]> {
    json_required(object, key)?
        .as_array()
        .map(Vec::as_slice)
        .ok_or_else(|| FactDescriptorError::descriptor(format!("{key} must be an array")))
}

fn json_bool(
    object: &serde_json::Map<String, serde_json::Value>,
    key: &'static str,
) -> Result<bool> {
    json_required(object, key)?
        .as_bool()
        .ok_or_else(|| FactDescriptorError::descriptor(format!("{key} must be a bool")))
}

fn json_u64(object: &serde_json::Map<String, serde_json::Value>, key: &'static str) -> Result<u64> {
    json_required(object, key)?
        .as_u64()
        .ok_or_else(|| FactDescriptorError::descriptor(format!("{key} must be a u64")))
}

fn json_u32(object: &serde_json::Map<String, serde_json::Value>, key: &'static str) -> Result<u32> {
    let value = json_u64(object, key)?;
    u32::try_from(value)
        .map_err(|_| FactDescriptorError::descriptor(format!("{key} must fit in u32")))
}

fn json_optional_u64(
    object: &serde_json::Map<String, serde_json::Value>,
    key: &'static str,
) -> Result<Option<u64>> {
    let value = json_required(object, key)?;
    if value.is_null() {
        Ok(None)
    } else {
        value
            .as_u64()
            .map(Some)
            .ok_or_else(|| FactDescriptorError::descriptor(format!("{key} must be null or u64")))
    }
}

fn parse_json_str<T>(
    object: &serde_json::Map<String, serde_json::Value>,
    key: &'static str,
) -> Result<T>
where
    T: FromStr,
    T::Err: fmt::Display,
{
    json_str(object, key)?.parse::<T>().map_err(|error| {
        FactDescriptorError::descriptor(format!("{key} failed validation: {error}"))
    })
}

fn parse_optional_identity<T>(value: &serde_json::Value) -> Result<Option<T>>
where
    T: FromStr,
    T::Err: fmt::Display,
{
    if value.is_null() {
        return Ok(None);
    }
    value
        .as_str()
        .ok_or_else(|| FactDescriptorError::descriptor("optional identity must be null or string"))?
        .parse::<T>()
        .map(Some)
        .map_err(|error| {
            FactDescriptorError::descriptor(format!("identity failed validation: {error}"))
        })
}

fn parse_optional_checked_string<T>(value: &serde_json::Value) -> Result<Option<T>>
where
    T: TryFrom<String, Error = FactDescriptorError>,
{
    if value.is_null() {
        return Ok(None);
    }
    let raw = value.as_str().ok_or_else(|| {
        FactDescriptorError::descriptor("optional checked string must be null or string")
    })?;
    T::try_from(raw.to_owned()).map(Some)
}

fn parse_optional_digest(value: &serde_json::Value) -> Result<Option<ContentDigest>> {
    parse_optional_identity(value)
}

fn parse_optional_string(value: &serde_json::Value) -> Result<Option<String>> {
    if value.is_null() {
        return Ok(None);
    }
    value
        .as_str()
        .map(|value| Some(value.to_owned()))
        .ok_or_else(|| FactDescriptorError::descriptor("optional string must be null or string"))
}

fn descriptor_fields_by_id(
    descriptor: &FactDescriptor,
) -> Result<BTreeMap<FactFieldId, &FactFieldDescriptor>> {
    let mut fields = BTreeMap::new();
    for field in descriptor.fields() {
        if fields.insert(field.field_id().clone(), field).is_some() {
            return Err(FactDescriptorError::field(
                field.field_id().clone(),
                "duplicate descriptor field id",
            ));
        }
    }
    Ok(fields)
}

fn required_query_field<'a>(
    fields_by_id: &'a BTreeMap<FactFieldId, &FactFieldDescriptor>,
    field_id: &FactFieldId,
) -> Result<&'a FactFieldDescriptor> {
    fields_by_id.get(field_id).copied().ok_or_else(|| {
        FactDescriptorError::field(field_id.clone(), "fact query references unknown field")
    })
}

fn require_query_exposed(field: &FactFieldDescriptor, context: &'static str) -> Result<()> {
    if field.exposure() == FactFieldExposure::Hidden {
        return Err(FactDescriptorError::field(
            field.field_id().clone(),
            format!("hidden field cannot be used in fact query {context}"),
        ));
    }
    Ok(())
}

struct CompiledQueryWire {
    fact_kind: FactKind,
    resolved_descriptor: ContentDigest,
    predicates: Vec<FactQueryPredicate>,
    return_fields: Vec<FactQueryReturnField>,
    ordering: FactOrderingName,
    limit: Option<u64>,
}

impl CompiledQueryWire {
    const VERSION: &'static str = "mfm.fact-query.v1";

    fn from_parts(
        descriptor: &FactDescriptor,
        descriptor_hash: &ContentDigest,
        predicates: &[FactQueryPredicate],
        return_fields: &[FactQueryReturnField],
        ordering: &FactOrderingName,
        limit: Option<u64>,
    ) -> Self {
        Self {
            fact_kind: descriptor.fact_kind().clone(),
            resolved_descriptor: descriptor_hash.clone(),
            predicates: predicates.to_vec(),
            return_fields: return_fields.to_vec(),
            ordering: ordering.clone(),
            limit,
        }
    }

    fn parse(value: &serde_json::Value) -> Result<Self> {
        let object = value.as_object().ok_or_else(|| {
            FactDescriptorError::descriptor("canonical fact query must be an object")
        })?;
        let version = object
            .get("version")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                FactDescriptorError::descriptor("canonical fact query version is required")
            })?;
        if version != Self::VERSION {
            return Err(FactDescriptorError::descriptor(format!(
                "unsupported canonical fact query version {version:?}"
            )));
        }
        let fact_kind = object
            .get("fact_kind")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                FactDescriptorError::descriptor("canonical fact query fact_kind is required")
            })
            .and_then(FactKind::new)?;
        let resolved_descriptor = object
            .get("resolved_descriptor")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                FactDescriptorError::descriptor("canonical fact query descriptor hash is required")
            })?
            .parse::<ContentDigest>()
            .map_err(|error| {
                FactDescriptorError::descriptor(format!(
                    "canonical fact query descriptor hash failed validation: {error}"
                ))
            })?;
        let predicates = object
            .get("predicates")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| {
                FactDescriptorError::descriptor("canonical fact query predicates are required")
            })?
            .iter()
            .map(QueryPredicateWire::parse)
            .collect::<Result<Vec<_>>>()?;
        let return_fields = object
            .get("return_fields")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| {
                FactDescriptorError::descriptor("canonical fact query return fields are required")
            })?
            .iter()
            .map(QueryReturnFieldWire::parse)
            .collect::<Result<Vec<_>>>()?;
        let ordering = object
            .get("ordering")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                FactDescriptorError::descriptor("canonical fact query ordering is required")
            })
            .and_then(FactOrderingName::new)?;
        let limit = Self::parse_limit(object)?;
        Ok(Self {
            fact_kind,
            resolved_descriptor,
            predicates,
            return_fields,
            ordering,
            limit,
        })
    }

    fn parse_limit(object: &serde_json::Map<String, serde_json::Value>) -> Result<Option<u64>> {
        let value = object.get("limit").ok_or_else(|| {
            FactDescriptorError::descriptor("canonical fact query limit is required")
        })?;
        if value.is_null() {
            return Ok(None);
        }
        value.as_u64().map(Some).ok_or_else(|| {
            FactDescriptorError::descriptor("canonical fact query limit must be a u64")
        })
    }

    fn validate_plan(&self, plan: &CanonicalFactQueryPlan) -> Result<()> {
        if &self.resolved_descriptor != plan.resolved_descriptor() {
            return Err(FactDescriptorError::descriptor(
                "canonical fact query descriptor hash does not match plan",
            ));
        }
        if &self.ordering != plan.ordering().name() {
            return Err(FactDescriptorError::descriptor(
                "canonical fact query ordering does not match plan",
            ));
        }
        if self.limit != plan.limit() {
            return Err(FactDescriptorError::descriptor(
                "canonical fact query limit does not match plan",
            ));
        }
        Ok(())
    }

    fn into_shape(self) -> Result<CompiledFactQueryShape> {
        CompiledFactQueryShape::new(self.predicates, self.return_fields)
    }

    fn canonical_value(&self) -> Result<CanonicalValue> {
        let predicates = self
            .predicates
            .iter()
            .map(QueryPredicateWire::canonical_value)
            .collect::<Result<Vec<_>>>()?;
        let return_fields = self
            .return_fields
            .iter()
            .map(QueryReturnFieldWire::canonical_value)
            .collect::<Result<Vec<_>>>()?;
        canonical_object([
            ("version", CanonicalValue::String(Self::VERSION.to_owned())),
            (
                "fact_kind",
                CanonicalValue::String(self.fact_kind.as_str().to_owned()),
            ),
            (
                "resolved_descriptor",
                CanonicalValue::String(self.resolved_descriptor.as_str().to_owned()),
            ),
            ("predicates", CanonicalValue::Array(predicates)),
            ("return_fields", CanonicalValue::Array(return_fields)),
            (
                "ordering",
                CanonicalValue::String(self.ordering.as_str().to_owned()),
            ),
            (
                "limit",
                self.limit
                    .map(CanonicalValue::Unsigned)
                    .unwrap_or(CanonicalValue::Null),
            ),
        ])
    }

    fn canonical_bytes(&self) -> Result<PlainCanonicalJsonBytes> {
        let bytes = self
            .canonical_value()
            .map(|value| CanonicalJsonBytes::from_value(&value))?;
        PlainCanonicalJsonBytes::from_canonical_json_slice(bytes.as_bytes())
            .map_err(|error| FactDescriptorError::canonical(error.to_string()))
    }
}

struct QueryPredicateWire;

impl QueryPredicateWire {
    fn parse(value: &serde_json::Value) -> Result<FactQueryPredicate> {
        let object = value
            .as_object()
            .ok_or_else(|| FactDescriptorError::descriptor("query predicate must be an object"))?;
        let field_id = object
            .get("field_id")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| FactDescriptorError::descriptor("query predicate field_id is required"))
            .and_then(FactFieldId::new)?;
        let operator = object
            .get("operator")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                FactDescriptorError::field(field_id.clone(), "query operator is required")
            })
            .and_then(|value| parse_query_operator(&field_id, value))?;
        let value_type = object
            .get("value_type")
            .ok_or_else(|| {
                FactDescriptorError::field(field_id.clone(), "query value_type is required")
            })
            .and_then(parse_field_value_type)?;
        let scalar_value = object.get("value").ok_or_else(|| {
            FactDescriptorError::field(field_id.clone(), "query value is required")
        })?;
        let value = parse_canonical_scalar_value(value_type, scalar_value, &field_id)?;
        Ok(FactQueryPredicate::new(field_id, operator, value))
    }

    fn canonical_value(predicate: &FactQueryPredicate) -> Result<CanonicalValue> {
        canonical_object([
            (
                "field_id",
                CanonicalValue::String(predicate.field_id().as_str().to_owned()),
            ),
            (
                "operator",
                CanonicalValue::String(predicate.operator().as_str().to_owned()),
            ),
            (
                "value_type",
                CanonicalValue::String(predicate.value().value_type().as_str().to_owned()),
            ),
            ("value", predicate.value().canonical_value()),
        ])
    }
}

struct QueryReturnFieldWire;

impl QueryReturnFieldWire {
    fn parse(value: &serde_json::Value) -> Result<FactQueryReturnField> {
        let object = value.as_object().ok_or_else(|| {
            FactDescriptorError::descriptor("query return field must be an object")
        })?;
        let field_id = object
            .get("field_id")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| FactDescriptorError::descriptor("query return field_id is required"))
            .and_then(FactFieldId::new)?;
        Ok(FactQueryReturnField::new(field_id))
    }

    fn canonical_value(field: &FactQueryReturnField) -> Result<CanonicalValue> {
        canonical_object([(
            "field_id",
            CanonicalValue::String(field.field_id().as_str().to_owned()),
        )])
    }
}

struct QueryScopeWire;

impl QueryScopeWire {
    fn parse(value: &serde_json::Value) -> Result<FactQueryScope> {
        let object = json_object(value, "fact query scope")?;
        Ok(FactQueryScope::new(
            parse_fact_audience(json_str(object, "audience")?)?,
            parse_fact_visibility_scope(json_str(object, "scope")?)?,
        ))
    }

    fn canonical_value(scope: &FactQueryScope) -> Result<CanonicalValue> {
        canonical_object([
            (
                "audience",
                CanonicalValue::String(scope.audience().as_str().to_owned()),
            ),
            (
                "scope",
                CanonicalValue::String(scope.scope().as_str().to_owned()),
            ),
        ])
    }
}

struct ScopeDecisionEvidenceWire;

impl ScopeDecisionEvidenceWire {
    fn parse(value: &serde_json::Value) -> Result<ScopeDecisionEvidence> {
        let object = json_object(value, "scope decision evidence")?;
        Ok(ScopeDecisionEvidence::new(parse_json_str(
            object,
            "decision_hash",
        )?))
    }

    fn canonical_value(evidence: &ScopeDecisionEvidence) -> Result<CanonicalValue> {
        canonical_object([(
            "decision_hash",
            CanonicalValue::String(evidence.decision_hash().as_str().to_owned()),
        )])
    }
}

struct OrderingWire;

impl OrderingWire {
    fn parse(value: &serde_json::Value) -> Result<FactOrderingPolicy> {
        let object = json_object(value, "fact ordering")?;
        let terms = json_array(object, "terms")?
            .iter()
            .map(OrderingTermWire::parse)
            .collect::<Result<Vec<_>>>()?;
        FactOrderingPolicy::new(FactOrderingName::new(json_str(object, "name")?)?, terms)
    }

    fn canonical_value(ordering: &FactOrderingPolicy) -> Result<CanonicalValue> {
        let terms = ordering
            .terms()
            .iter()
            .map(OrderingTermWire::canonical_value)
            .collect::<Result<Vec<_>>>()?;
        canonical_object([
            (
                "name",
                CanonicalValue::String(ordering.name().as_str().to_owned()),
            ),
            ("terms", CanonicalValue::Array(terms)),
        ])
    }
}

struct OrderingTermWire;

impl OrderingTermWire {
    fn parse(value: &serde_json::Value) -> Result<FactOrderingTerm> {
        let object = json_object(value, "fact ordering term")?;
        Ok(FactOrderingTerm::new(
            FactFieldId::new(json_str(object, "field_id")?)?,
            parse_sort_direction(json_str(object, "direction")?)?,
            parse_null_ordering(json_str(object, "nulls")?)?,
            json_bool(object, "tie_breaker")?,
        ))
    }

    fn canonical_value(term: &FactOrderingTerm) -> Result<CanonicalValue> {
        canonical_object([
            (
                "field_id",
                CanonicalValue::String(term.field_id().as_str().to_owned()),
            ),
            (
                "direction",
                CanonicalValue::String(term.direction().as_str().to_owned()),
            ),
            (
                "nulls",
                CanonicalValue::String(term.nulls().as_str().to_owned()),
            ),
            ("tie_breaker", CanonicalValue::Bool(term.tie_breaker())),
        ])
    }
}

fn canonical_query_plan_value(plan: &CanonicalFactQueryPlan) -> Result<CanonicalValue> {
    canonical_object([
        (
            "version",
            CanonicalValue::String("mfm.fact-query-plan.v1".to_owned()),
        ),
        (
            "store_scope",
            CanonicalValue::String(plan.store_scope.as_str().to_owned()),
        ),
        (
            "query_scope",
            QueryScopeWire::canonical_value(&plan.query_scope)?,
        ),
        (
            "query_compiler_version",
            CanonicalValue::String(plan.query_compiler_version.as_str().to_owned()),
        ),
        (
            "canonicalizer_version",
            CanonicalValue::String(plan.canonicalizer_version.as_str().to_owned()),
        ),
        (
            "resolved_descriptor",
            CanonicalValue::String(plan.resolved_descriptor.as_str().to_owned()),
        ),
        (
            "scope_decision_evidence",
            ScopeDecisionEvidenceWire::canonical_value(&plan.scope_decision_evidence)?,
        ),
        (
            "canonical_query",
            CanonicalValue::String(plan.canonical_query.as_str().to_owned()),
        ),
        (
            "canonical_query_hash",
            CanonicalValue::String(plan.canonical_query_hash.as_str().to_owned()),
        ),
        ("ordering", OrderingWire::canonical_value(&plan.ordering)?),
        (
            "limit",
            plan.limit
                .map(CanonicalValue::Unsigned)
                .unwrap_or(CanonicalValue::Null),
        ),
    ])
}

fn canonical_query_receipt_body_value(
    plan_hash: &ContentDigest,
    receipt: &FactQueryReceipt,
) -> Result<CanonicalValue> {
    FactQueryReceiptBodyParts::from_receipt(plan_hash, receipt).canonical_value()
}

struct FactQueryReceiptBodyParts<'a> {
    plan_hash: &'a ContentDigest,
    read_frontier: &'a StoreReadFrontier,
    frontier_type: StoreReadFrontierType,
    returned_refs: &'a [InternalFactRef],
    returned_field_summaries: Option<&'a ReturnedFieldSummaries>,
    result_set_digest: &'a ContentDigest,
    result_cardinality: QueryResultCardinality,
}

impl<'a> FactQueryReceiptBodyParts<'a> {
    fn new(
        plan_hash: &'a ContentDigest,
        read_frontier: &'a StoreReadFrontier,
        frontier_type: StoreReadFrontierType,
        returned_refs: &'a [InternalFactRef],
        returned_field_summaries: Option<&'a ReturnedFieldSummaries>,
        result_set_digest: &'a ContentDigest,
        result_cardinality: QueryResultCardinality,
    ) -> Self {
        Self {
            plan_hash,
            read_frontier,
            frontier_type,
            returned_refs,
            returned_field_summaries,
            result_set_digest,
            result_cardinality,
        }
    }

    fn from_receipt(plan_hash: &'a ContentDigest, receipt: &'a FactQueryReceipt) -> Self {
        Self::new(
            plan_hash,
            &receipt.read_frontier,
            receipt.frontier_type,
            &receipt.returned_refs,
            receipt.returned_field_summaries.as_ref(),
            &receipt.result_set_digest,
            receipt.result_cardinality,
        )
    }

    fn canonical_value(&self) -> Result<CanonicalValue> {
        canonical_query_receipt_body_parts_value(self)
    }

    fn canonical_bytes(&self) -> Result<CanonicalJsonBytes> {
        self.canonical_value()
            .map(|value| CanonicalJsonBytes::from_value(&value))
    }

    fn hash(&self) -> Result<ContentDigest> {
        Ok(self.canonical_bytes()?.content_digest())
    }
}

struct OwnedFactQueryReceiptBodyParts {
    plan_hash: ContentDigest,
    read_frontier: StoreReadFrontier,
    frontier_type: StoreReadFrontierType,
    returned_refs: Vec<InternalFactRef>,
    returned_field_summaries: Option<ReturnedFieldSummaries>,
    result_set_digest: ContentDigest,
    result_cardinality: QueryResultCardinality,
}

impl OwnedFactQueryReceiptBodyParts {
    fn parse(value: &serde_json::Value, expected_plan_hash: &ContentDigest) -> Result<Self> {
        let body = json_object(value, "fact query receipt body")?;
        require_version(
            body,
            "mfm.fact-query-receipt-body.v1",
            "fact query receipt body",
        )?;
        let plan_hash = parse_json_str(body, "plan_hash")?;
        if &plan_hash != expected_plan_hash {
            return Err(FactDescriptorError::descriptor(
                "fact query receipt body plan hash does not match plan",
            ));
        }
        Ok(Self {
            plan_hash,
            read_frontier: parse_store_read_frontier(json_required(body, "read_frontier")?)?,
            frontier_type: parse_store_read_frontier_type(json_str(body, "frontier_type")?)?,
            returned_refs: json_array(body, "returned_refs")?
                .iter()
                .map(parse_internal_fact_ref)
                .collect::<Result<Vec<_>>>()?,
            returned_field_summaries: parse_optional_returned_field_summaries(json_required(
                body,
                "returned_field_summaries",
            )?)?,
            result_set_digest: parse_json_str(body, "result_set_digest")?,
            result_cardinality: parse_query_result_cardinality(json_required(
                body,
                "result_cardinality",
            )?)?,
        })
    }

    fn as_borrowed(&self) -> FactQueryReceiptBodyParts<'_> {
        FactQueryReceiptBodyParts::new(
            &self.plan_hash,
            &self.read_frontier,
            self.frontier_type,
            &self.returned_refs,
            self.returned_field_summaries.as_ref(),
            &self.result_set_digest,
            self.result_cardinality,
        )
    }

    fn into_receipt(
        self,
        store_receipt_hash: ContentDigest,
        store_receipt_authentication: StoreReceiptAuthentication,
    ) -> FactQueryReceipt {
        FactQueryReceipt::from_parts(
            self.read_frontier,
            self.frontier_type,
            self.returned_refs,
            self.returned_field_summaries,
            self.result_set_digest,
            self.result_cardinality,
            store_receipt_hash,
            store_receipt_authentication,
        )
    }
}

fn canonical_query_receipt_body_parts_value(
    parts: &FactQueryReceiptBodyParts<'_>,
) -> Result<CanonicalValue> {
    let returned_refs = parts
        .returned_refs
        .iter()
        .map(canonical_internal_fact_ref_value)
        .collect::<Result<Vec<_>>>()?;
    canonical_object([
        (
            "version",
            CanonicalValue::String("mfm.fact-query-receipt-body.v1".to_owned()),
        ),
        (
            "plan_hash",
            CanonicalValue::String(parts.plan_hash.as_str().to_owned()),
        ),
        (
            "read_frontier",
            canonical_store_read_frontier_value(parts.read_frontier)?,
        ),
        (
            "frontier_type",
            CanonicalValue::String(parts.frontier_type.as_str().to_owned()),
        ),
        ("returned_refs", CanonicalValue::Array(returned_refs)),
        (
            "returned_field_summaries",
            optional_returned_field_summaries_value(parts.returned_field_summaries)?,
        ),
        (
            "result_set_digest",
            CanonicalValue::String(parts.result_set_digest.as_str().to_owned()),
        ),
        (
            "result_cardinality",
            canonical_query_result_cardinality_value(parts.result_cardinality)?,
        ),
    ])
}

fn canonical_fact_query_result_set_value(
    returned_refs: &[InternalFactRef],
    returned_field_summaries: Option<&ReturnedFieldSummaries>,
) -> Result<CanonicalValue> {
    let returned_refs = returned_refs
        .iter()
        .map(canonical_internal_fact_ref_value)
        .collect::<Result<Vec<_>>>()?;
    canonical_object([
        (
            "version",
            CanonicalValue::String("mfm.fact-query-result-set.v1".to_owned()),
        ),
        ("returned_refs", CanonicalValue::Array(returned_refs)),
        (
            "returned_field_summaries",
            optional_returned_field_summaries_value(returned_field_summaries)?,
        ),
    ])
}

fn canonical_query_receipt_value(
    plan_hash: &ContentDigest,
    receipt: &FactQueryReceipt,
) -> Result<CanonicalValue> {
    canonical_object([
        (
            "body",
            canonical_query_receipt_body_value(plan_hash, receipt)?,
        ),
        (
            "store_receipt_hash",
            CanonicalValue::String(receipt.store_receipt_hash.as_str().to_owned()),
        ),
        (
            "store_receipt_authentication",
            canonical_store_receipt_authentication_value(&receipt.store_receipt_authentication)?,
        ),
    ])
}

fn canonical_query_evidence_value(evidence: &FactQueryEvidence) -> Result<CanonicalValue> {
    let plan_hash = fact_query_plan_hash(&evidence.plan)?;
    canonical_object([
        (
            "version",
            CanonicalValue::String("mfm.fact-query-evidence.v1".to_owned()),
        ),
        ("plan", canonical_query_plan_value(&evidence.plan)?),
        (
            "receipt",
            canonical_query_receipt_value(&plan_hash, &evidence.receipt)?,
        ),
        (
            "selection",
            canonical_fact_selection_evidence_value(&evidence.selection)?,
        ),
    ])
}

fn canonical_store_read_frontier_value(frontier: &StoreReadFrontier) -> Result<CanonicalValue> {
    canonical_object([
        (
            "store_scope",
            CanonicalValue::String(frontier.store_scope.as_str().to_owned()),
        ),
        (
            "query_scope",
            QueryScopeWire::canonical_value(&frontier.query_scope)?,
        ),
        (
            "descriptor_catalog_watermark",
            CanonicalValue::Unsigned(frontier.descriptor_catalog_watermark.as_u64()),
        ),
        (
            "projection_generation",
            CanonicalValue::Unsigned(frontier.projection_generation.as_u64()),
        ),
        (
            "max_included_store_commit_order",
            CanonicalValue::Unsigned(frontier.max_included_store_commit_order),
        ),
        (
            "commit_watermark",
            CanonicalValue::Unsigned(frontier.commit_watermark.as_u64()),
        ),
    ])
}

fn canonical_internal_fact_ref_value(reference: &InternalFactRef) -> Result<CanonicalValue> {
    let parts = &reference.parts;
    canonical_object([
        (
            "fact_claim_id",
            canonical_fact_claim_id_value(&parts.fact_claim_id)?,
        ),
        (
            "source_event_id",
            CanonicalValue::String(parts.source_event_id.as_str().to_owned()),
        ),
        (
            "recorded_at",
            CanonicalValue::String(parts.recorded_at.clone()),
        ),
        (
            "producer_node_id",
            CanonicalValue::String(parts.producer_node_id.as_str().to_owned()),
        ),
        (
            "observed_at",
            optional_string_value(parts.observed_at.as_deref()),
        ),
        (
            "visibility",
            canonical_fact_visibility_value(&parts.visibility)?,
        ),
        (
            "fact_kind",
            CanonicalValue::String(parts.fact_kind.as_str().to_owned()),
        ),
        (
            "fact_descriptor_hash",
            CanonicalValue::String(parts.fact_descriptor_hash.as_str().to_owned()),
        ),
        (
            "fact_subject_namespace_hash",
            CanonicalValue::String(parts.fact_subject_namespace_hash.as_str().to_owned()),
        ),
        (
            "fact_key",
            CanonicalValue::String(parts.fact_key.as_str().to_owned()),
        ),
        (
            "subject_material_hash",
            CanonicalValue::String(parts.subject_material_hash.as_str().to_owned()),
        ),
        (
            "request_schema_id",
            optional_schema_id_value(parts.request_schema_id.as_ref()),
        ),
        (
            "request_hash",
            optional_digest_value(parts.request_hash.as_ref()),
        ),
        (
            "response_schema_id",
            CanonicalValue::String(parts.response_schema_id.as_str().to_owned()),
        ),
        (
            "response_hash",
            CanonicalValue::String(parts.response_hash.as_str().to_owned()),
        ),
        (
            "artifact_id",
            CanonicalValue::String(parts.artifact_id.as_str().to_owned()),
        ),
        (
            "artifact_evidence_hash",
            CanonicalValue::String(parts.artifact_evidence_hash.as_str().to_owned()),
        ),
        (
            "capability_kind",
            CanonicalValue::String(parts.capability_kind.as_str().to_owned()),
        ),
        (
            "capability_version",
            CanonicalValue::String(parts.capability_version.as_str().to_owned()),
        ),
        (
            "adapter_kind",
            CanonicalValue::String(parts.adapter_kind.as_str().to_owned()),
        ),
        (
            "adapter_version",
            CanonicalValue::String(parts.adapter_version.as_str().to_owned()),
        ),
    ])
}

fn canonical_fact_claim_id_value(claim_id: &FactClaimId) -> Result<CanonicalValue> {
    canonical_object([
        (
            "version",
            CanonicalValue::String("mfm.fact-claim-id.v1".to_owned()),
        ),
        (
            "source_run_id",
            CanonicalValue::String(claim_id.source_run_id.as_str().to_owned()),
        ),
        ("source_seq", CanonicalValue::Unsigned(claim_id.source_seq)),
        (
            "source_ordinal",
            CanonicalValue::Unsigned(u64::from(claim_id.source_ordinal)),
        ),
    ])
}

fn canonical_fact_visibility_value(visibility: &FactVisibility) -> Result<CanonicalValue> {
    match visibility {
        FactVisibility::RunPrivate => {
            canonical_object([("kind", CanonicalValue::String("run_private".to_owned()))])
        }
        FactVisibility::Indexed { audience, scope } => canonical_object([
            ("kind", CanonicalValue::String("indexed".to_owned())),
            (
                "audience",
                CanonicalValue::String(audience.as_str().to_owned()),
            ),
            ("scope", CanonicalValue::String(scope.as_str().to_owned())),
        ]),
    }
}

fn optional_returned_field_summaries_value(
    summaries: Option<&ReturnedFieldSummaries>,
) -> Result<CanonicalValue> {
    summaries
        .map(canonical_returned_field_summaries_value)
        .unwrap_or(Ok(CanonicalValue::Null))
}

fn canonical_returned_field_summaries_value(
    summaries: &ReturnedFieldSummaries,
) -> Result<CanonicalValue> {
    let summaries = summaries
        .summaries
        .iter()
        .map(canonical_returned_fact_field_summary_value)
        .collect::<Result<Vec<_>>>()?;
    Ok(CanonicalValue::Array(summaries))
}

fn canonical_returned_fact_field_summary_value(
    summary: &ReturnedFactFieldSummary,
) -> Result<CanonicalValue> {
    let fields = summary
        .fields
        .iter()
        .map(canonical_returned_field_value_summary_value)
        .collect::<Result<Vec<_>>>()?;
    canonical_object([
        (
            "fact_claim_id",
            canonical_fact_claim_id_value(&summary.fact_claim_id)?,
        ),
        ("fields", CanonicalValue::Array(fields)),
    ])
}

fn canonical_returned_field_value_summary_value(
    summary: &ReturnedFieldValueSummary,
) -> Result<CanonicalValue> {
    canonical_object([
        (
            "field_id",
            CanonicalValue::String(summary.field_id().as_str().to_owned()),
        ),
        (
            "value_type",
            CanonicalValue::String(summary.value_type().as_str().to_owned()),
        ),
        ("value", summary.value().canonical_value()),
    ])
}

fn canonical_query_result_cardinality_value(
    cardinality: QueryResultCardinality,
) -> Result<CanonicalValue> {
    match cardinality {
        QueryResultCardinality::Exact(value) => canonical_object([
            ("kind", CanonicalValue::String("exact".to_owned())),
            ("value", CanonicalValue::Unsigned(value)),
        ]),
        QueryResultCardinality::AtLeast(value) => canonical_object([
            ("kind", CanonicalValue::String("at_least".to_owned())),
            ("value", CanonicalValue::Unsigned(value)),
        ]),
    }
}

fn canonical_store_receipt_authentication_value(
    auth: &StoreReceiptAuthentication,
) -> Result<CanonicalValue> {
    canonical_object([
        (
            "store_identity",
            CanonicalValue::String(auth.store_identity.as_str().to_owned()),
        ),
        (
            "scheme",
            CanonicalValue::String(auth.scheme.as_str().to_owned()),
        ),
        (
            "key_id",
            optional_checked_string_value(auth.key_id.as_ref()),
        ),
        (
            "signature_or_mac",
            CanonicalValue::Bytes(CanonicalBytes::new(auth.signature_or_mac.clone())),
        ),
    ])
}

fn canonical_fact_selection_evidence_value(
    selection: &FactSelectionEvidence,
) -> Result<CanonicalValue> {
    canonical_object([
        (
            "selection_policy_hash",
            CanonicalValue::String(selection.selection_policy_hash.as_str().to_owned()),
        ),
        (
            "selected_indices",
            CanonicalValue::Array(
                selection
                    .selected_indices
                    .iter()
                    .copied()
                    .map(CanonicalValue::Unsigned)
                    .collect(),
            ),
        ),
        (
            "selected_summaries_digest",
            optional_digest_value(selection.selected_summaries_digest.as_ref()),
        ),
    ])
}

fn optional_checked_string_value<T>(value: Option<&T>) -> CanonicalValue
where
    T: AsRef<str>,
{
    value
        .map(|value| CanonicalValue::String(value.as_ref().to_owned()))
        .unwrap_or(CanonicalValue::Null)
}

fn optional_string_value(value: Option<&str>) -> CanonicalValue {
    value
        .map(|value| CanonicalValue::String(value.to_owned()))
        .unwrap_or(CanonicalValue::Null)
}

fn optional_schema_id_value(value: Option<&SchemaId>) -> CanonicalValue {
    value
        .map(|value| CanonicalValue::String(value.as_str().to_owned()))
        .unwrap_or(CanonicalValue::Null)
}

fn optional_digest_value(value: Option<&ContentDigest>) -> CanonicalValue {
    value
        .map(|value| CanonicalValue::String(value.as_str().to_owned()))
        .unwrap_or(CanonicalValue::Null)
}

fn optional_scale_value(scale: Option<FactScale>) -> CanonicalValue {
    scale
        .map(|scale| CanonicalValue::Signed(i64::from(scale.exponent())))
        .unwrap_or(CanonicalValue::Null)
}

pub(crate) fn canonical_object(
    entries: impl IntoIterator<Item = (impl Into<String>, CanonicalValue)>,
) -> Result<CanonicalValue> {
    CanonicalValue::object(entries)
        .map_err(|error| FactDescriptorError::canonical(error.to_string()))
}
