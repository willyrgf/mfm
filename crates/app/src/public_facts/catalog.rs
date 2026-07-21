use std::collections::{BTreeMap, BTreeSet};

use mfm_ids::ContentDigest;
use mfm_store::v1 as store;

use super::dto::{
    PublicFactDescriptorRef, PublicFactDescriptorSummary, PublicFactExplain,
    PublicFactFieldSummary, PublicFactFieldValue, PublicFactKindSummary, PublicFactOrderingSummary,
    PublicFactOrderingTermSummary, PublicFactRef, PublicFactScalarValue,
};
use super::query::PublicFactQueryRequest;
use super::ref_id::{public_ref_id, PublicFactRefId};
use super::service::AppFactQueryRow;
use crate::{async_app_store_error, ErrorClass, PublicError};

/// App-owned public descriptor catalog for facts.
#[derive(Debug, Clone, Default)]
pub struct FactCatalogService {
    descriptors: BTreeMap<ContentDigest, mfm_facts::FactDescriptor>,
}

impl FactCatalogService {
    /// Creates a catalog from descriptors that are already authorized for public discovery.
    pub fn new<I>(descriptors: I) -> Result<Self, PublicError>
    where
        I: IntoIterator<Item = mfm_facts::FactDescriptor>,
    {
        let descriptors = descriptors
            .into_iter()
            .map(|descriptor| {
                let hash = mfm_facts::fact_descriptor_hash(&descriptor)?;
                Ok((hash, descriptor))
            })
            .collect::<mfm_facts::Result<BTreeMap<_, _>>>()?;
        Ok(Self { descriptors })
    }

    /// Loads a public catalog from retained descriptor artifacts and store projection authority.
    ///
    /// Only descriptors with indexed `Platform` facts in the default visibility scope are included.
    pub async fn from_retained_public_projection<A>(
        artifacts: &A,
        projection: &store::ProjectionSnapshot,
    ) -> Result<Self, PublicError>
    where
        A: store::RetainedArtifactReadProvider + ?Sized,
    {
        let public_hashes = public_fact_descriptor_hashes(projection);
        let mut descriptors = BTreeMap::new();
        for descriptor_hash in public_hashes {
            let projection = projection
                .fact_descriptor(&descriptor_hash)
                .ok_or_else(fact_descriptor_projection_missing)?;
            let descriptor = load_projected_fact_descriptor(artifacts, projection).await?;
            descriptors.insert(descriptor_hash, descriptor);
        }
        Ok(Self { descriptors })
    }

    /// Creates a catalog filtered to descriptors with Platform facts in the projection snapshot.
    pub fn from_public_projection<I>(
        descriptors: I,
        projection: &store::ProjectionSnapshot,
    ) -> Result<Self, PublicError>
    where
        I: IntoIterator<Item = mfm_facts::FactDescriptor>,
    {
        let public_hashes = public_fact_descriptor_hashes(projection);
        let descriptors = descriptors
            .into_iter()
            .filter_map(|descriptor| {
                let hash = mfm_facts::fact_descriptor_hash(&descriptor).ok()?;
                public_hashes.contains(&hash).then_some((hash, descriptor))
            })
            .collect();
        Ok(Self { descriptors })
    }

    /// Lists public fact kinds.
    pub fn list_kinds(&self) -> Vec<PublicFactKindSummary> {
        let mut counts = BTreeMap::<String, usize>::new();
        for descriptor in self.descriptors.values() {
            *counts
                .entry(descriptor.fact_kind().as_str().to_owned())
                .or_default() += 1;
        }
        counts
            .into_iter()
            .map(|(fact_kind, descriptor_count)| PublicFactKindSummary {
                fact_kind,
                descriptor_count,
            })
            .collect()
    }

    /// Describes public descriptors for one kind.
    pub fn describe_kind(
        &self,
        fact_kind: &str,
    ) -> Result<Vec<PublicFactDescriptorSummary>, PublicError> {
        let descriptors = self
            .descriptors
            .values()
            .filter(|descriptor| descriptor.fact_kind().as_str() == fact_kind)
            .map(public_descriptor_summary)
            .collect::<Vec<_>>();
        if descriptors.is_empty() {
            return Err(redacted_fact_not_found());
        }
        Ok(descriptors)
    }

    /// Explains public query and return fields for one kind.
    pub fn explain_kind(&self, fact_kind: &str) -> Result<PublicFactExplain, PublicError> {
        Ok(PublicFactExplain {
            fact_kind: fact_kind.to_owned(),
            descriptors: self.describe_kind(fact_kind)?,
        })
    }

    pub(super) fn resolve_descriptor(
        &self,
        request: &PublicFactQueryRequest,
    ) -> Result<(&ContentDigest, &mfm_facts::FactDescriptor), PublicError> {
        let matches = self
            .descriptors
            .iter()
            .filter(|(_hash, descriptor)| descriptor.fact_kind() == &request.fact_kind)
            .filter(|(_hash, descriptor)| {
                request
                    .shape
                    .as_ref()
                    .is_none_or(|shape| shape.matches_descriptor(descriptor))
            })
            .collect::<Vec<_>>();
        match matches.as_slice() {
            [(hash, descriptor)] => Ok((hash, descriptor)),
            [] => Err(redacted_fact_not_found()),
            _ => Err(PublicError::new(
                ErrorClass::BadRequest,
                "FactDescriptorAmbiguous",
                "Fact kind resolves to more than one public descriptor; provide a shape",
            )),
        }
    }

    pub(super) fn descriptor_by_hash(
        &self,
        descriptor_hash: &ContentDigest,
    ) -> Option<&mfm_facts::FactDescriptor> {
        self.descriptors.get(descriptor_hash)
    }
}

/// Resolves opaque public fact refs against a projection snapshot.
#[derive(Debug, Clone)]
pub struct FactPublicRefResolver {
    catalog: FactCatalogService,
    projection: store::ProjectionSnapshot,
}

impl FactPublicRefResolver {
    /// Creates a resolver over public projection data and a public catalog.
    pub fn new(catalog: FactCatalogService, projection: store::ProjectionSnapshot) -> Self {
        Self {
            catalog,
            projection,
        }
    }

    /// Resolves a public fact ref, returning the same not-found class for unknown and non-public refs.
    pub fn resolve(&self, public_ref: &PublicFactRefId) -> Result<PublicFactRef, PublicError> {
        for (_claim_id, entry) in self.projection.fact_index_entries() {
            if !is_public_default_fact_index_entry(entry) {
                continue;
            }
            let fact_ref = entry.internal_ref()?;
            if public_ref_id(&fact_ref)? != *public_ref {
                continue;
            }
            let descriptor = self
                .catalog
                .descriptor_by_hash(&entry.fact_descriptor_hash)
                .ok_or_else(redacted_fact_not_found)?;
            let fields = public_fields_from_values(
                descriptor,
                self.projection
                    .fact_terms_for_claim(&entry.fact_claim_id)
                    .map(|term| (&term.field_id, &term.value)),
            )?;
            return public_fact_from_parts(&fact_ref, descriptor, fields);
        }
        Err(redacted_fact_not_found())
    }
}

pub(super) fn public_fact_query_scope() -> mfm_facts::FactQueryScope {
    mfm_facts::FactQueryScope::new(
        mfm_facts::FactAudience::Platform,
        mfm_facts::FactVisibilityScope::Default,
    )
}

pub(super) fn is_public_default_fact_visibility(
    audience: mfm_facts::FactAudience,
    scope: mfm_facts::FactVisibilityScope,
) -> bool {
    audience == mfm_facts::FactAudience::Platform
        && scope == mfm_facts::FactVisibilityScope::Default
}

fn is_public_default_fact_index_entry(entry: &store::FactIndexProjection) -> bool {
    is_public_default_fact_visibility(entry.audience, entry.visibility_scope)
}

fn is_public_default_internal_fact_ref(fact_ref: &mfm_facts::InternalFactRef) -> bool {
    let mfm_facts::FactVisibility::Indexed { audience, scope } = fact_ref.visibility() else {
        return false;
    };
    is_public_default_fact_visibility(*audience, *scope)
}

fn public_fact_descriptor_hashes(
    projection: &store::ProjectionSnapshot,
) -> BTreeSet<ContentDigest> {
    projection
        .fact_index_entries()
        .filter(|(_claim_id, entry)| is_public_default_fact_index_entry(entry))
        .map(|(_claim_id, entry)| entry.fact_descriptor_hash.clone())
        .collect()
}

async fn load_projected_fact_descriptor<A>(
    artifacts: &A,
    projection: &store::FactDescriptorProjection,
) -> Result<mfm_facts::FactDescriptor, PublicError>
where
    A: store::RetainedArtifactReadProvider + ?Sized,
{
    let artifact = artifacts
        .read_retained_artifact(&store::fact_descriptor_artifact_requirement(projection)?)
        .await
        .map_err(async_app_store_error)?;
    let descriptor = mfm_facts::parse_canonical_fact_descriptor_bytes(artifact.bytes())
        .map_err(|_| fact_descriptor_artifact_invalid())?;
    validate_projected_fact_descriptor(&descriptor, projection)?;
    Ok(descriptor)
}

fn validate_projected_fact_descriptor(
    descriptor: &mfm_facts::FactDescriptor,
    projection: &store::FactDescriptorProjection,
) -> Result<(), PublicError> {
    let descriptor_hash = mfm_facts::fact_descriptor_hash(descriptor)
        .map_err(|_| fact_descriptor_artifact_invalid())?;
    let namespace_hash = mfm_facts::fact_subject_namespace_hash(descriptor)
        .map_err(|_| fact_descriptor_artifact_invalid())?;
    if descriptor_hash != projection.descriptor_hash
        || descriptor.fact_kind() != &projection.fact_kind
        || descriptor.descriptor_schema_id() != &projection.descriptor_schema_id
        || descriptor.subject_schema_id() != &projection.subject_schema_id
        || descriptor.response_schema_id() != &projection.response_schema_id
        || namespace_hash != projection.fact_subject_namespace_hash
    {
        return Err(PublicError::backend(
            ErrorClass::Internal,
            "FactDescriptorProjectionMismatch",
            "Fact descriptor projection did not match retained descriptor authority",
        ));
    }
    Ok(())
}

fn fact_descriptor_projection_missing() -> PublicError {
    PublicError::backend(
        ErrorClass::Internal,
        "FactDescriptorProjectionMissing",
        "Fact descriptor projection was missing retained descriptor authority",
    )
}

fn fact_descriptor_artifact_invalid() -> PublicError {
    PublicError::backend(
        ErrorClass::Internal,
        "FactDescriptorArtifactInvalid",
        "Fact descriptor artifact failed verification",
    )
}

fn public_descriptor_summary(
    descriptor: &mfm_facts::FactDescriptor,
) -> PublicFactDescriptorSummary {
    let fields_by_id = descriptor_fields_by_id(descriptor);
    PublicFactDescriptorSummary {
        fact_kind: descriptor.fact_kind().as_str().to_owned(),
        descriptor: public_descriptor_ref(descriptor),
        fields: descriptor
            .fields()
            .iter()
            .filter(|field| field.exposure() != mfm_facts::FactFieldExposure::Hidden)
            .map(public_field_summary)
            .collect(),
        orderings: descriptor
            .orderings()
            .iter()
            .filter_map(|ordering| public_ordering_summary(&fields_by_id, ordering))
            .collect(),
    }
}

fn public_descriptor_ref(descriptor: &mfm_facts::FactDescriptor) -> PublicFactDescriptorRef {
    PublicFactDescriptorRef {
        descriptor_schema_id: descriptor.descriptor_schema_id().as_str().to_owned(),
        subject_schema_id: descriptor.subject_schema_id().as_str().to_owned(),
        response_schema_id: descriptor.response_schema_id().as_str().to_owned(),
    }
}

fn public_field_summary(field: &mfm_facts::FactFieldDescriptor) -> PublicFactFieldSummary {
    PublicFactFieldSummary {
        field_id: field.field_id().as_str().to_owned(),
        path: field.path(),
        source: field.extraction().source().path_prefix().to_owned(),
        value_type: field.value_type().as_str().to_owned(),
        exposure: field.exposure().as_str().to_owned(),
        operators: field
            .operators()
            .iter()
            .map(|operator| operator.as_str().to_owned())
            .collect(),
        unit: field.unit().map(|unit| unit.as_str().to_owned()),
        scale: field.scale().map(mfm_facts::FactScale::exponent),
        sortable: field.sortable(),
    }
}

fn public_ordering_summary(
    fields: &BTreeMap<&mfm_facts::FactFieldId, &mfm_facts::FactFieldDescriptor>,
    ordering: &mfm_facts::FactOrderingPolicy,
) -> Option<PublicFactOrderingSummary> {
    let terms = ordering
        .terms()
        .iter()
        .map(|term| {
            let field = fields.get(term.field_id())?;
            (field.exposure() != mfm_facts::FactFieldExposure::Hidden).then_some(
                PublicFactOrderingTermSummary {
                    field_id: term.field_id().as_str().to_owned(),
                    direction: term.direction().as_str().to_owned(),
                    nulls: term.nulls().as_str().to_owned(),
                    tie_breaker: term.tie_breaker(),
                },
            )
        })
        .collect::<Option<Vec<_>>>()?;
    Some(PublicFactOrderingSummary {
        name: ordering.name().as_str().to_owned(),
        terms,
    })
}

fn descriptor_fields_by_id(
    descriptor: &mfm_facts::FactDescriptor,
) -> BTreeMap<&mfm_facts::FactFieldId, &mfm_facts::FactFieldDescriptor> {
    descriptor
        .fields()
        .iter()
        .map(|field| (field.field_id(), field))
        .collect()
}

pub(super) fn public_query_input(
    request: &PublicFactQueryRequest,
    store_scope: mfm_facts::StoreScopeRef,
    scope_decision_evidence: mfm_facts::ScopeDecisionEvidence,
) -> Result<mfm_facts::FactQueryInput, PublicError> {
    let predicates = request.predicates.clone();
    let return_fields = request
        .return_fields
        .iter()
        .cloned()
        .map(mfm_facts::FactFieldId::new)
        .collect::<mfm_facts::Result<Vec<_>>>()?;
    Ok(mfm_facts::FactQueryInput::new(
        store_scope,
        public_fact_query_scope(),
        scope_decision_evidence,
        predicates,
        return_fields,
        request.ordering.clone(),
        request.limit.map(std::num::NonZeroU64::get),
    )?)
}

pub(super) fn public_fact_from_query_row(
    catalog: &FactCatalogService,
    row: &AppFactQueryRow,
) -> Result<Option<PublicFactRef>, PublicError> {
    if !is_public_default_internal_fact_ref(row.fact_ref()) {
        return Ok(None);
    }
    let Some(descriptor) = catalog.descriptor_by_hash(row.fact_ref().fact_descriptor_hash()) else {
        return Ok(None);
    };
    let fields = public_fields_from_summaries(descriptor, row.returned_fields())?;
    public_fact_from_parts(row.fact_ref(), descriptor, fields).map(Some)
}

fn public_fact_from_parts(
    fact_ref: &mfm_facts::InternalFactRef,
    descriptor: &mfm_facts::FactDescriptor,
    fields: Vec<PublicFactFieldValue>,
) -> Result<PublicFactRef, PublicError> {
    Ok(PublicFactRef {
        public_ref: public_ref_id(fact_ref)?,
        fact_kind: fact_ref.fact_kind().as_str().to_owned(),
        descriptor: public_descriptor_ref(descriptor),
        recorded_at: fact_ref.recorded_at().to_owned(),
        observed_at: fact_ref.observed_at().map(str::to_owned),
        fields,
    })
}

fn public_fields_from_summaries(
    descriptor: &mfm_facts::FactDescriptor,
    summaries: &[mfm_facts::FactFieldValue],
) -> Result<Vec<PublicFactFieldValue>, PublicError> {
    public_fields_from_values(
        descriptor,
        summaries
            .iter()
            .map(|summary| (summary.field_id(), summary.value())),
    )
}

type PublicFieldSource<'a> = (
    &'a mfm_facts::FactFieldId,
    &'a mfm_facts::FactCanonicalScalar,
);

fn public_fields_from_values<'a>(
    descriptor: &mfm_facts::FactDescriptor,
    values: impl Iterator<Item = PublicFieldSource<'a>>,
) -> Result<Vec<PublicFactFieldValue>, PublicError> {
    let fields = descriptor_fields_by_id(descriptor);
    values
        .filter_map(|term| {
            let field = fields.get(term.0)?;
            (field.exposure() == mfm_facts::FactFieldExposure::Returnable)
                .then(|| public_field_value(field, term.1))
        })
        .collect::<Result<Vec<_>, _>>()
}

fn public_field_value(
    field: &mfm_facts::FactFieldDescriptor,
    value: &mfm_facts::FactCanonicalScalar,
) -> Result<PublicFactFieldValue, PublicError> {
    Ok(PublicFactFieldValue {
        field_id: field.field_id().as_str().to_owned(),
        path: field.path().as_str().to_owned(),
        source: field.extraction().source().path_prefix().to_owned(),
        value_type: field.value_type().as_str().to_owned(),
        value: PublicFactScalarValue::from(value),
        unit: field.unit().map(|unit| unit.as_str().to_owned()),
        scale: field.scale().map(mfm_facts::FactScale::exponent),
    })
}

fn redacted_fact_not_found() -> PublicError {
    PublicError::not_found(
        "FactNotFound",
        "Fact was not found or is not available through the public fact service",
    )
}
