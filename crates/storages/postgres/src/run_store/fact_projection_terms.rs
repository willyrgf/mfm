use super::*;

pub(in crate::run_store) fn push_fact_index_term_projection_select_list(
    builder: &mut QueryBuilder<Postgres>,
) {
    let mut has_column = false;
    for column in FACT_INDEX_TERM_IDENTITY_COLUMNS {
        push_fact_index_term_select_column(builder, column, &mut has_column);
    }
    for column in FactTermValueColumn::ALL {
        push_fact_index_term_select_column(builder, column.storage_column(), &mut has_column);
    }
    for column in FACT_INDEX_TERM_METADATA_COLUMNS {
        push_fact_index_term_select_column(builder, column, &mut has_column);
    }
}

fn push_fact_index_term_select_column(
    builder: &mut QueryBuilder<Postgres>,
    column: &str,
    has_column: &mut bool,
) {
    if *has_column {
        builder.push(", ");
    }
    builder.push(column);
    *has_column = true;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FactTermValueColumn {
    Text,
    Bool,
    I64,
    U64,
    Decimal,
    Timestamp,
    Digest,
}

impl FactTermValueColumn {
    const ALL: [Self; 7] = [
        Self::Text,
        Self::Bool,
        Self::I64,
        Self::U64,
        Self::Decimal,
        Self::Timestamp,
        Self::Digest,
    ];

    fn for_value_type(value_type: mfm_facts::FactFieldValueType) -> Self {
        match value_type {
            mfm_facts::FactFieldValueType::String => Self::Text,
            mfm_facts::FactFieldValueType::Boolean => Self::Bool,
            mfm_facts::FactFieldValueType::SignedInteger => Self::I64,
            mfm_facts::FactFieldValueType::UnsignedInteger => Self::U64,
            mfm_facts::FactFieldValueType::Timestamp => Self::Timestamp,
            mfm_facts::FactFieldValueType::DecimalString => Self::Decimal,
            mfm_facts::FactFieldValueType::Digest => Self::Digest,
        }
    }

    fn storage_column(self) -> &'static str {
        match self {
            Self::Text => "value_text",
            Self::Bool => "value_bool",
            Self::I64 => "value_i64",
            Self::U64 => "value_u64",
            Self::Decimal => "value_decimal",
            Self::Timestamp => "value_timestamp",
            Self::Digest => "value_digest",
        }
    }
}

pub(in crate::run_store) fn descriptor_projection_evidence_hash(
    projection: &mfm_store::v1::FactDescriptorProjection,
) -> Result<ContentDigest> {
    if projection.descriptor_artifact_evidence.artifact_id != projection.descriptor_artifact_id
        || projection.descriptor_artifact_evidence.digest != projection.descriptor_hash
        || projection.descriptor_artifact_evidence.artifact_role
            != events::ArtifactRole::FactDescriptor
    {
        return Err(StoreError::ArtifactEvidenceMismatch {
            artifact_id: projection.descriptor_artifact_id.clone(),
            field: "fact_descriptor",
        }
        .into());
    }
    projection
        .descriptor_artifact_evidence
        .evidence_hash()
        .map_err(Into::into)
}

pub(in crate::run_store) fn descriptor_admission_evidence_hash(
    event: &KernelEventEnvelope,
    projection: &mfm_store::v1::FactDescriptorProjection,
) -> Result<ContentDigest> {
    let events::KernelEventPayload::RunAdmitted(payload) = event.payload() else {
        return Err(PostgresStoreError::Corruption(
            "fact descriptor projection did not point at RunAdmitted".to_owned(),
        ));
    };
    let artifact = payload
        .fact_descriptor_artifacts
        .iter()
        .find(|artifact| {
            artifact.artifact_id == projection.descriptor_artifact_id
                && artifact.content_digest == projection.descriptor_hash
        })
        .ok_or_else(|| {
            PostgresStoreError::Corruption(
                "fact descriptor projection missing RunAdmitted artifact evidence".to_owned(),
            )
        })?;
    let evidence = ArtifactEvidenceRef::from_run_artifact(artifact);
    if evidence != projection.descriptor_artifact_evidence {
        return Err(StoreError::ArtifactEvidenceMismatch {
            artifact_id: evidence.artifact_id,
            field: "fact_descriptor",
        }
        .into());
    }
    evidence.evidence_hash().map_err(Into::into)
}

pub(in crate::run_store) struct TermValueColumns {
    pub(super) value_text: Option<String>,
    pub(super) value_bool: Option<bool>,
    pub(super) value_i64: Option<i64>,
    pub(super) value_u64: Option<String>,
    pub(super) value_decimal: Option<String>,
    pub(super) value_timestamp: Option<String>,
    pub(super) value_digest: Option<String>,
}

impl TermValueColumns {
    pub(super) fn from_scalar(value: &mfm_facts::FactCanonicalScalar) -> Self {
        let mut columns = Self {
            value_text: None,
            value_bool: None,
            value_i64: None,
            value_u64: None,
            value_decimal: None,
            value_timestamp: None,
            value_digest: None,
        };
        match value {
            mfm_facts::FactCanonicalScalar::String(value) => {
                columns.value_text = Some(value.clone())
            }
            mfm_facts::FactCanonicalScalar::Boolean(value) => columns.value_bool = Some(*value),
            mfm_facts::FactCanonicalScalar::SignedInteger(value) => {
                columns.value_i64 = Some(*value);
            }
            mfm_facts::FactCanonicalScalar::UnsignedInteger(value) => {
                columns.value_u64 = Some(value.to_string());
            }
            mfm_facts::FactCanonicalScalar::Timestamp(value) => {
                columns.value_timestamp = Some(value.clone());
            }
            mfm_facts::FactCanonicalScalar::DecimalString(value) => {
                columns.value_decimal = Some(value.as_str().to_owned());
            }
            mfm_facts::FactCanonicalScalar::Digest(value) => {
                columns.value_digest = Some(value.as_str().to_owned());
            }
        }
        columns
    }
}

pub(in crate::run_store) fn fact_term_value_from_reader(
    row: &PgRowReader<'_>,
) -> Result<(
    mfm_facts::FactFieldId,
    mfm_facts::FactFieldValueType,
    mfm_facts::FactCanonicalScalar,
)> {
    let field_id =
        mfm_facts::FactFieldId::new(row.required_string("field_id")?).map_err(fact_error)?;
    let value_type = parse_fact_field_value_type(&row.required_string("value_type")?)?;
    let value = parse_term_value_from_reader(row, value_type)?;
    Ok((field_id, value_type, value))
}

fn parse_term_value_from_reader(
    row: &PgRowReader<'_>,
    value_type: mfm_facts::FactFieldValueType,
) -> Result<mfm_facts::FactCanonicalScalar> {
    let column = FactTermValueColumn::for_value_type(value_type);
    match column {
        FactTermValueColumn::Text => row
            .required_string(column.storage_column())
            .map(mfm_facts::FactCanonicalScalar::String),
        FactTermValueColumn::Bool => row
            .required_bool(column.storage_column())
            .map(mfm_facts::FactCanonicalScalar::Boolean),
        FactTermValueColumn::I64 => row
            .required_i64(column.storage_column())
            .map(mfm_facts::FactCanonicalScalar::SignedInteger),
        FactTermValueColumn::U64 => {
            let value = row
                .required_string(column.storage_column())?
                .parse::<u64>()
                .map_err(|_| {
                    PostgresStoreError::Corruption(format!(
                        "fact_index_terms.{} was not a u64",
                        column.storage_column()
                    ))
                })?;
            Ok(mfm_facts::FactCanonicalScalar::UnsignedInteger(value))
        }
        FactTermValueColumn::Timestamp => {
            mfm_facts::FactCanonicalScalar::timestamp(row.required_string(column.storage_column())?)
                .map_err(fact_error)
        }
        FactTermValueColumn::Decimal => mfm_facts::FactCanonicalScalar::decimal_variable(
            row.required_string(column.storage_column())?,
        )
        .map_err(fact_error),
        FactTermValueColumn::Digest => {
            let digest = row.required_identity(column.storage_column())?;
            Ok(mfm_facts::FactCanonicalScalar::Digest(digest))
        }
    }
}

pub(in crate::run_store) fn parse_fact_audience(value: &str) -> Result<mfm_facts::FactAudience> {
    value
        .parse::<mfm_facts::FactAudience>()
        .map_err(|_| PostgresStoreError::Corruption(format!("unknown fact audience {value}")))
}

pub(in crate::run_store) fn parse_fact_visibility_scope(
    value: &str,
) -> Result<mfm_facts::FactVisibilityScope> {
    value
        .parse::<mfm_facts::FactVisibilityScope>()
        .map_err(|_| {
            PostgresStoreError::Corruption(format!("unknown fact visibility scope {value}"))
        })
}

pub(in crate::run_store) fn parse_fact_field_source(
    value: &str,
) -> Result<mfm_facts::FactFieldSource> {
    value
        .parse::<mfm_facts::FactFieldSource>()
        .map_err(|_| PostgresStoreError::Corruption(format!("unknown fact field source {value}")))
}

pub(super) fn parse_fact_field_value_type(value: &str) -> Result<mfm_facts::FactFieldValueType> {
    value.parse::<mfm_facts::FactFieldValueType>().map_err(|_| {
        PostgresStoreError::Corruption(format!("unknown fact field value type {value}"))
    })
}

pub(in crate::run_store) fn fact_error(error: mfm_facts::FactError) -> PostgresStoreError {
    StoreError::Identity(error.to_string()).into()
}
