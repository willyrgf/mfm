use super::*;

pub(super) fn payload_canonical_bytes(
    payload: &events::KernelEventPayload,
) -> Result<PlainCanonicalJsonBytes> {
    Ok(mfm_store::v1::payload_canonical_json(payload)?)
}

pub(super) fn payload_json_value(payload: &events::KernelEventPayload) -> Result<Value> {
    let canonical = payload_canonical_bytes(payload)?;
    serde_json::from_slice(canonical.as_bytes()).map_err(|error| {
        PostgresStoreError::Corruption(format!("canonical payload was not JSON: {error}"))
    })
}

pub(super) fn canonical_json(value: serde_json::Value) -> Result<PlainCanonicalJsonBytes> {
    PlainCanonicalJsonBytes::from_json_str(&value.to_string()).map_err(|error| {
        PostgresStoreError::Corruption(format!(
            "failed to canonicalize Postgres authority JSON: {error}"
        ))
    })
}

pub(super) fn next_seq_from_head(head: u64) -> Result<StreamSeq> {
    let next = head.checked_add(1).ok_or(StoreError::SequenceOverflow)?;
    Ok(StreamSeq::new(next)?)
}

pub(super) fn u64_to_i64(value: u64, field: &str) -> Result<i64> {
    i64::try_from(value).map_err(|_| {
        PostgresStoreError::Corruption(format!("{field} exceeded PostgreSQL bigint range"))
    })
}

pub(super) fn i64_to_nonnegative_u64(value: i64, field: &str) -> Result<u64> {
    u64::try_from(value)
        .map_err(|_| PostgresStoreError::Corruption(format!("{field} contained a negative bigint")))
}

pub(super) fn i64_to_positive_u64(value: i64, field: &str) -> Result<u64> {
    let value = i64_to_nonnegative_u64(value, field)?;
    if value == 0 {
        return Err(PostgresStoreError::Corruption(format!(
            "{field} contained a non-positive sequence"
        )));
    }
    Ok(value)
}

pub(super) fn parse_optional_identity<T>(value: Option<String>) -> Result<Option<T>>
where
    T: FromStr<Err = IdentityError>,
{
    Ok(value.as_deref().map(parse_identity).transpose()?)
}

#[derive(Debug, Clone, Copy)]
pub(super) struct PgRowReader<'row> {
    row: &'row PgRow,
    relation: &'static str,
}

impl<'row> PgRowReader<'row> {
    pub(super) fn new(row: &'row PgRow, relation: &'static str) -> Self {
        Self { row, relation }
    }

    pub(super) fn required_string(&self, column: &str) -> Result<String> {
        let field = self.field(column);
        self.row
            .try_get::<String, _>(column)
            .map_err(|_| PostgresStoreError::Corruption(format!("{field} was missing or invalid")))
    }

    pub(super) fn optional_string(&self, column: &str) -> Result<Option<String>> {
        let field = self.field(column);
        self.row
            .try_get::<Option<String>, _>(column)
            .map_err(|_| PostgresStoreError::Corruption(format!("{field} was invalid")))
    }

    pub(super) fn required_bool(&self, column: &str) -> Result<bool> {
        let field = self.field(column);
        self.row
            .try_get::<bool, _>(column)
            .map_err(|_| PostgresStoreError::Corruption(format!("{field} was missing or invalid")))
    }

    pub(super) fn required_i64(&self, column: &str) -> Result<i64> {
        let field = self.field(column);
        self.row
            .try_get::<i64, _>(column)
            .map_err(|_| PostgresStoreError::Corruption(format!("{field} was missing or invalid")))
    }

    pub(super) fn required_i32(&self, column: &str) -> Result<i32> {
        let field = self.field(column);
        self.row
            .try_get::<i32, _>(column)
            .map_err(|_| PostgresStoreError::Corruption(format!("{field} was missing or invalid")))
    }

    pub(super) fn optional_i32(&self, column: &str) -> Result<Option<i32>> {
        let field = self.field(column);
        self.row
            .try_get::<Option<i32>, _>(column)
            .map_err(|_| PostgresStoreError::Corruption(format!("{field} was invalid")))
    }

    pub(super) fn required_identity<T>(&self, column: &str) -> Result<T>
    where
        T: FromStr<Err = IdentityError>,
    {
        Ok(parse_identity(&self.required_string(column)?)?)
    }

    pub(super) fn optional_identity<T>(&self, column: &str) -> Result<Option<T>>
    where
        T: FromStr<Err = IdentityError>,
    {
        parse_optional_identity(self.optional_string(column)?)
    }

    pub(super) fn required_parsed<T>(&self, column: &str) -> Result<T>
    where
        T: FromStr,
        T::Err: std::fmt::Display,
    {
        self.required_string(column)?
            .parse()
            .map_err(|error| PostgresStoreError::Corruption(format!("{error}")))
    }

    pub(super) fn required_positive_u64(&self, column: &str) -> Result<u64> {
        i64_to_positive_u64(self.required_i64(column)?, &self.field(column))
    }

    pub(super) fn required_u32(&self, column: &str) -> Result<u32> {
        u32::try_from(self.required_i32(column)?).map_err(|_| {
            PostgresStoreError::Corruption(format!("{} outside u32 range", self.field(column)))
        })
    }

    fn field(&self, column: &str) -> String {
        format!("{}.{}", self.relation, column)
    }
}

pub(super) fn required_i64(row: &PgRow, column: &str, field: &'static str) -> Result<i64> {
    row.try_get::<i64, _>(column)
        .map_err(|_| PostgresStoreError::Corruption(format!("{field} was missing or invalid")))
}
