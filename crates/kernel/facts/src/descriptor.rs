use std::collections::BTreeSet;

use mfm_ids::SchemaId;

use crate::*;

/// One term in a descriptor-defined fact ordering policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactOrderingTerm {
    pub(crate) field_id: FactFieldId,
    pub(crate) direction: SortDirection,
    pub(crate) nulls: NullOrdering,
    pub(crate) tie_breaker: bool,
}

impl FactOrderingTerm {
    /// Creates an ordering term for one field.
    pub fn new(
        field_id: FactFieldId,
        direction: SortDirection,
        nulls: NullOrdering,
        tie_breaker: bool,
    ) -> Self {
        Self {
            field_id,
            direction,
            nulls,
            tie_breaker,
        }
    }

    /// Returns the field id used by this ordering term.
    pub const fn field_id(&self) -> &FactFieldId {
        &self.field_id
    }

    /// Returns the sort direction.
    pub const fn direction(&self) -> SortDirection {
        self.direction
    }

    /// Returns the null ordering.
    pub const fn nulls(&self) -> NullOrdering {
        self.nulls
    }

    /// Returns whether this term is a deterministic tie-breaker.
    pub const fn tie_breaker(&self) -> bool {
        self.tie_breaker
    }

    /// Compares optional fact scalars using this ordering term's direction and null policy.
    ///
    /// Sort direction applies only to present scalar values. Null placement follows the term's
    /// `NULLS FIRST`/`NULLS LAST` policy independently, matching SQL ordering semantics.
    pub fn compare_values(
        &self,
        left: Option<&FactCanonicalScalar>,
        right: Option<&FactCanonicalScalar>,
    ) -> Option<std::cmp::Ordering> {
        Some(match (left, right) {
            (Some(left), Some(right)) => {
                let ordering = left.query_cmp(right)?;
                match self.direction {
                    SortDirection::Ascending => ordering,
                    SortDirection::Descending => ordering.reverse(),
                }
            }
            (None, None) => std::cmp::Ordering::Equal,
            (None, Some(_)) => match self.nulls {
                NullOrdering::First => std::cmp::Ordering::Less,
                NullOrdering::Last => std::cmp::Ordering::Greater,
            },
            (Some(_), None) => match self.nulls {
                NullOrdering::First => std::cmp::Ordering::Greater,
                NullOrdering::Last => std::cmp::Ordering::Less,
            },
        })
    }
}

/// Descriptor-defined fact ordering policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactOrderingPolicy {
    pub(crate) name: FactOrderingName,
    pub(crate) terms: Vec<FactOrderingTerm>,
}

impl FactOrderingPolicy {
    /// Creates a descriptor-defined ordering policy.
    pub fn new(name: FactOrderingName, terms: Vec<FactOrderingTerm>) -> Result<Self> {
        if terms.is_empty() {
            return Err(FactDescriptorError::ordering(
                name,
                "ordering must contain at least one term",
            ));
        }

        Ok(Self { name, terms })
    }

    /// Returns this ordering name.
    pub const fn name(&self) -> &FactOrderingName {
        &self.name
    }

    /// Returns ordering terms in retained order.
    pub fn terms(&self) -> &[FactOrderingTerm] {
        &self.terms
    }
}

/// One descriptor-declared field that can produce fact index terms.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactFieldDescriptor {
    pub(crate) field_id: FactFieldId,
    pub(crate) path: FactFieldPath,
    pub(crate) value_type: FactFieldValueType,
    pub(crate) extraction: FactFieldExtraction,
    pub(crate) operators: Vec<FactQueryOperator>,
    pub(crate) exposure: FactFieldExposure,
    pub(crate) unit: Option<FactUnit>,
    pub(crate) scale: Option<FactScale>,
    pub(crate) sortable: bool,
    pub(crate) required: bool,
}

impl FactFieldDescriptor {
    /// Creates a fact field descriptor with validation.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        field_id: FactFieldId,
        path: FactFieldPath,
        value_type: FactFieldValueType,
        extraction: FactFieldExtraction,
        operators: Vec<FactQueryOperator>,
        exposure: FactFieldExposure,
        unit: Option<FactUnit>,
        scale: Option<FactScale>,
        sortable: bool,
        required: bool,
    ) -> Result<Self> {
        let descriptor = Self {
            field_id,
            path,
            value_type,
            extraction,
            operators,
            exposure,
            unit,
            scale,
            sortable,
            required,
        };
        descriptor.validate()?;
        Ok(descriptor)
    }

    /// Returns this field id.
    pub const fn field_id(&self) -> &FactFieldId {
        &self.field_id
    }

    /// Returns this field path.
    pub const fn path(&self) -> &FactFieldPath {
        &self.path
    }

    /// Returns this field value type.
    pub const fn value_type(&self) -> FactFieldValueType {
        self.value_type
    }

    /// Returns this field extraction recipe.
    pub const fn extraction(&self) -> &FactFieldExtraction {
        &self.extraction
    }

    /// Returns allowed query operators in retained order.
    pub fn operators(&self) -> &[FactQueryOperator] {
        &self.operators
    }

    /// Returns this field exposure policy.
    pub const fn exposure(&self) -> FactFieldExposure {
        self.exposure
    }

    /// Returns this field unit.
    pub const fn unit(&self) -> Option<&FactUnit> {
        self.unit.as_ref()
    }

    /// Returns this field scale.
    pub const fn scale(&self) -> Option<FactScale> {
        self.scale
    }

    /// Returns whether this field may be used in descriptor orderings.
    pub const fn sortable(&self) -> bool {
        self.sortable
    }

    /// Returns whether indexed facts must provide this field.
    pub const fn required(&self) -> bool {
        self.required
    }

    pub(crate) fn validate(&self) -> Result<()> {
        let expected_prefix = self.extraction.source().path_prefix();
        if self.path.source_prefix() != Some(expected_prefix) {
            return Err(FactDescriptorError::field(
                self.field_id.clone(),
                format!(
                    "path prefix must be {expected_prefix:?} for {:?} extraction",
                    self.extraction.source()
                ),
            ));
        }

        if matches!(self.extraction, FactFieldExtraction::Metadata(_))
            && self.extraction_metadata_value_type() != Some(self.value_type)
        {
            return Err(FactDescriptorError::field(
                self.field_id.clone(),
                "metadata field value type does not match metadata source",
            ));
        }

        if self.operators.is_empty() {
            return Err(FactDescriptorError::field(
                self.field_id.clone(),
                "field must allow at least one query operator",
            ));
        }

        let mut seen = BTreeSet::new();
        for operator in &self.operators {
            if !seen.insert(*operator) {
                return Err(FactDescriptorError::field(
                    self.field_id.clone(),
                    format!("duplicate operator {operator:?}"),
                ));
            }
            if !self.value_type.supports_operator(*operator) {
                return Err(FactDescriptorError::field(
                    self.field_id.clone(),
                    format!(
                        "operator {operator:?} is incompatible with value type {:?}",
                        self.value_type
                    ),
                ));
            }
        }

        if self.sortable && !self.value_type.is_sortable() {
            return Err(FactDescriptorError::field(
                self.field_id.clone(),
                format!("value type {:?} is not sortable", self.value_type),
            ));
        }

        Ok(())
    }

    fn extraction_metadata_value_type(&self) -> Option<FactFieldValueType> {
        match self.extraction {
            FactFieldExtraction::Metadata(field) => Some(field.value_type()),
            _ => None,
        }
    }
}

/// Durable descriptor for a fact shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactDescriptor {
    pub(crate) fact_kind: FactKind,
    pub(crate) descriptor_schema_id: SchemaId,
    pub(crate) subject_schema_id: SchemaId,
    pub(crate) response_schema_id: SchemaId,
    pub(crate) fields: Vec<FactFieldDescriptor>,
    pub(crate) orderings: Vec<FactOrderingPolicy>,
}

impl FactDescriptor {
    /// Creates a validated fact descriptor.
    pub fn new(
        fact_kind: FactKind,
        descriptor_schema_id: SchemaId,
        subject_schema_id: SchemaId,
        response_schema_id: SchemaId,
        fields: Vec<FactFieldDescriptor>,
        orderings: Vec<FactOrderingPolicy>,
    ) -> Result<Self> {
        let descriptor = Self {
            fact_kind,
            descriptor_schema_id,
            subject_schema_id,
            response_schema_id,
            fields,
            orderings,
        };
        validate_descriptor(&descriptor)?;
        Ok(descriptor)
    }

    /// Returns the fact kind.
    pub const fn fact_kind(&self) -> &FactKind {
        &self.fact_kind
    }

    /// Returns the descriptor schema id.
    pub const fn descriptor_schema_id(&self) -> &SchemaId {
        &self.descriptor_schema_id
    }

    /// Returns the subject schema id.
    pub const fn subject_schema_id(&self) -> &SchemaId {
        &self.subject_schema_id
    }

    /// Returns the response schema id.
    pub const fn response_schema_id(&self) -> &SchemaId {
        &self.response_schema_id
    }

    /// Returns fields in retained descriptor order.
    pub fn fields(&self) -> &[FactFieldDescriptor] {
        &self.fields
    }

    /// Returns ordering policies in retained descriptor order.
    pub fn orderings(&self) -> &[FactOrderingPolicy] {
        &self.orderings
    }
}
