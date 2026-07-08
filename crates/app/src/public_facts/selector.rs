use std::num::NonZeroU64;

use super::query::{PublicFactPredicate, PublicFactQueryRequest, PublicFactShapeSelector};
use crate::{AppError, ErrorClass};

const FACT_QUERY_INVALID_PARAMETER_CODE: &str = "FactQueryInvalidParameter";

/// Transport-neutral selector material for building a public fact query request.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PublicFactQuerySelector {
    /// Optional public shape selector matching descriptor schema id.
    pub shape: Option<String>,
    /// Subject predicates. Values may omit the `subject.` field prefix.
    pub subject_predicates: Vec<String>,
    /// Result predicates. Values may omit the `result.` field prefix.
    pub result_predicates: Vec<String>,
    /// Fully qualified descriptor field predicates.
    pub field_predicates: Vec<String>,
    /// Returnable field ids to include in each fact.
    pub return_fields: Vec<String>,
    /// Descriptor ordering policy name.
    pub ordering: Option<String>,
    /// Optional non-zero limit.
    pub limit: Option<u64>,
}

impl PublicFactQueryRequest {
    /// Builds a public fact query request directly from URL-style query pairs.
    pub fn from_query_pairs<K, V>(
        fact_kind: impl AsRef<str>,
        pairs: impl IntoIterator<Item = (K, V)>,
        limit_override: Option<u64>,
    ) -> Result<Self, AppError>
    where
        K: AsRef<str>,
        V: AsRef<str>,
    {
        Self::from_selector(
            fact_kind,
            PublicFactQuerySelector::from_query_pairs(pairs)?.with_limit_override(limit_override),
        )
    }

    /// Builds a public fact query request from transport-parsed selector fields.
    pub fn from_selector(
        fact_kind: impl AsRef<str>,
        selector: PublicFactQuerySelector,
    ) -> Result<Self, AppError> {
        let limit = selector
            .limit
            .map(|limit| NonZeroU64::new(limit).ok_or_else(fact_query_limit_invalid))
            .transpose()?;
        let request = Self {
            fact_kind: mfm_facts::FactKind::new(fact_kind.as_ref())?,
            shape: selector.shape.map(PublicFactShapeSelector::new),
            predicates: parse_public_fact_predicates(
                selector.subject_predicates.iter().map(String::as_str),
                selector.result_predicates.iter().map(String::as_str),
                selector.field_predicates.iter().map(String::as_str),
            )?,
            return_fields: selector
                .return_fields
                .iter()
                .map(mfm_facts::FactFieldId::new)
                .collect::<mfm_facts::Result<Vec<_>>>()?,
            ordering: mfm_facts::FactOrderingName::new(
                selector.ordering.ok_or_else(fact_ordering_missing)?,
            )?,
            limit,
        };
        request.validate()?;
        Ok(request)
    }
}

impl PublicFactQuerySelector {
    /// Parses URL query pairs into a transport-neutral public fact query selector.
    pub fn from_query_pairs<K, V>(pairs: impl IntoIterator<Item = (K, V)>) -> Result<Self, AppError>
    where
        K: AsRef<str>,
        V: AsRef<str>,
    {
        let mut selector = Self::default();
        for (key, value) in pairs {
            selector.push_query_pair(key.as_ref(), value.as_ref())?;
        }
        Ok(selector)
    }

    /// Applies a forced limit, preserving the existing selector limit when no override is supplied.
    pub fn with_limit_override(mut self, limit: Option<u64>) -> Self {
        self.limit = limit.or(self.limit);
        self
    }

    fn push_query_pair(&mut self, key: &str, value: &str) -> Result<(), AppError> {
        match key {
            "shape" => self.shape = Some(value.to_owned()),
            "order" => self.ordering = Some(value.to_owned()),
            "subject" => self.subject_predicates.push(value.to_owned()),
            "result" => self.result_predicates.push(value.to_owned()),
            "where" => self.field_predicates.push(value.to_owned()),
            "field" | "fields" => self.return_fields.push(value.to_owned()),
            "limit" => {
                self.limit = Some(
                    value
                        .parse::<u64>()
                        .map_err(|_| fact_query_parse_invalid())?,
                );
            }
            _ => return Err(fact_query_parse_invalid()),
        }
        Ok(())
    }
}

fn fact_ordering_missing() -> AppError {
    AppError::new(
        ErrorClass::BadRequest,
        "FactOrderingMissing",
        "Fact queries must provide an explicit order parameter",
    )
}

fn fact_query_limit_invalid() -> AppError {
    AppError::new(
        ErrorClass::BadRequest,
        "FactQueryLimitInvalid",
        "Fact query limit must be greater than zero",
    )
}

fn fact_query_parse_invalid() -> AppError {
    AppError::new(
        ErrorClass::BadRequest,
        FACT_QUERY_INVALID_PARAMETER_CODE,
        "Fact query parameters are invalid",
    )
}

/// Returns whether an app error represents malformed public fact query parameters.
pub fn is_public_fact_query_parameter_error(error: &AppError) -> bool {
    error.code == FACT_QUERY_INVALID_PARAMETER_CODE
}

/// Parses public fact predicate strings used by CLI and REST query surfaces.
///
/// Subject and result predicates may omit their `subject.` or `result.` prefix. Full predicates
/// must include the descriptor field id explicitly.
pub fn parse_public_fact_predicates<'a>(
    subjects: impl IntoIterator<Item = &'a str>,
    results: impl IntoIterator<Item = &'a str>,
    where_predicates: impl IntoIterator<Item = &'a str>,
) -> Result<Vec<PublicFactPredicate>, AppError> {
    let mut predicates = Vec::new();
    for value in subjects {
        predicates.push(parse_prefixed_public_fact_predicate("subject", value)?);
    }
    for value in results {
        predicates.push(parse_prefixed_public_fact_predicate("result", value)?);
    }
    for value in where_predicates {
        predicates.push(parse_public_fact_predicate(value)?);
    }
    Ok(predicates)
}

fn parse_prefixed_public_fact_predicate(
    prefix: &'static str,
    value: &str,
) -> Result<PublicFactPredicate, AppError> {
    let predicate = parse_public_fact_predicate(value)?;
    let field_id = if predicate.field_id().as_str().starts_with("subject.")
        || predicate.field_id().as_str().starts_with("result.")
    {
        predicate.field_id().clone()
    } else {
        mfm_facts::FactFieldId::new(format!("{prefix}.{}", predicate.field_id().as_str()))?
    };
    Ok(mfm_facts::FactQueryPredicate::new(
        field_id,
        predicate.operator(),
        predicate.value().clone(),
    ))
}

fn parse_public_fact_predicate(value: &str) -> Result<PublicFactPredicate, AppError> {
    let (left, raw_value) = value
        .split_once('=')
        .ok_or_else(invalid_public_fact_predicate)?;
    if left.is_empty() {
        return Err(invalid_public_fact_predicate());
    }
    let (field_id, operator) = parse_public_fact_field_and_operator(left)?;
    Ok(mfm_facts::FactQueryPredicate::new(
        field_id,
        operator,
        parse_public_fact_scalar_value(raw_value)?,
    ))
}

fn parse_public_fact_field_and_operator(
    value: &str,
) -> Result<(mfm_facts::FactFieldId, mfm_facts::FactQueryOperator), AppError> {
    let operators = [
        (".lte", mfm_facts::FactQueryOperator::LessThanOrEqual),
        (".gte", mfm_facts::FactQueryOperator::GreaterThanOrEqual),
        (".eq", mfm_facts::FactQueryOperator::Equal),
        (".lt", mfm_facts::FactQueryOperator::LessThan),
        (".gt", mfm_facts::FactQueryOperator::GreaterThan),
    ];
    for (suffix, operator) in operators {
        if let Some(field_id) = value.strip_suffix(suffix) {
            if field_id.is_empty() {
                return Err(invalid_public_fact_predicate());
            }
            return Ok((mfm_facts::FactFieldId::new(field_id)?, operator));
        }
    }
    Ok((
        mfm_facts::FactFieldId::new(value)?,
        mfm_facts::FactQueryOperator::Equal,
    ))
}

fn parse_public_fact_scalar_value(value: &str) -> Result<mfm_facts::FactCanonicalScalar, AppError> {
    if let Some((type_name, typed_value)) = value.split_once(':') {
        return match type_name {
            "string" => Ok(mfm_facts::FactCanonicalScalar::String(
                typed_value.to_owned(),
            )),
            "bool" => typed_value
                .parse::<bool>()
                .map(mfm_facts::FactCanonicalScalar::Boolean)
                .map_err(|_| public_fact_predicate_error("Boolean fact value is invalid")),
            "i64" => typed_value
                .parse::<i64>()
                .map(mfm_facts::FactCanonicalScalar::SignedInteger)
                .map_err(|_| public_fact_predicate_error("Signed integer fact value is invalid")),
            "u64" => typed_value
                .parse::<u64>()
                .map(mfm_facts::FactCanonicalScalar::UnsignedInteger)
                .map_err(|_| public_fact_predicate_error("Unsigned integer fact value is invalid")),
            "timestamp" => Ok(mfm_facts::FactCanonicalScalar::timestamp(
                typed_value.to_owned(),
            )?),
            "decimal" => Ok(mfm_facts::FactCanonicalScalar::decimal_variable(
                typed_value.to_owned(),
            )?),
            "digest" => Ok(mfm_facts::FactCanonicalScalar::Digest(
                mfm_ids::ContentDigest::parse(typed_value).map_err(|_| {
                    AppError::new(
                        ErrorClass::BadRequest,
                        "FactQueryDigestInvalid",
                        "Fact query digest value is invalid",
                    )
                })?,
            )),
            _ => Ok(infer_public_fact_scalar_value(value)),
        };
    }
    Ok(infer_public_fact_scalar_value(value))
}

fn infer_public_fact_scalar_value(value: &str) -> mfm_facts::FactCanonicalScalar {
    if let Ok(parsed) = value.parse::<bool>() {
        return mfm_facts::FactCanonicalScalar::Boolean(parsed);
    }
    if value.starts_with('-') {
        if let Ok(parsed) = value.parse::<i64>() {
            return mfm_facts::FactCanonicalScalar::SignedInteger(parsed);
        }
    } else if let Ok(parsed) = value.parse::<u64>() {
        return mfm_facts::FactCanonicalScalar::UnsignedInteger(parsed);
    }
    if value.contains('.') && value.chars().all(|ch| ch.is_ascii_digit() || ch == '.') {
        return mfm_facts::FactCanonicalScalar::decimal_variable(value.to_owned())
            .unwrap_or_else(|_| mfm_facts::FactCanonicalScalar::String(value.to_owned()));
    }
    mfm_facts::FactCanonicalScalar::String(value.to_owned())
}

fn invalid_public_fact_predicate() -> AppError {
    public_fact_predicate_error("Fact predicates must use `field[.operator]=value`")
}

fn public_fact_predicate_error(message: &'static str) -> AppError {
    AppError::new(ErrorClass::BadRequest, "FactPredicateInvalid", message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn predicate_strings_decode_to_typed_query_primitives() {
        let predicates = parse_public_fact_predicates(
            ["network=bitcoin-mainnet"],
            ["amount_sat.gt=1000"],
            ["metadata.observed_at.lte=timestamp:2026-07-02T00:00:00Z"],
        )
        .expect("predicates");

        assert_eq!(predicates[0].field_id().as_str(), "subject.network");
        assert_eq!(
            predicates[0].operator(),
            mfm_facts::FactQueryOperator::Equal
        );
        assert_eq!(
            predicates[0].value(),
            &mfm_facts::FactCanonicalScalar::String("bitcoin-mainnet".to_owned())
        );
        assert_eq!(predicates[1].field_id().as_str(), "result.amount_sat");
        assert_eq!(
            predicates[1].operator(),
            mfm_facts::FactQueryOperator::GreaterThan
        );
        assert_eq!(
            predicates[1].value(),
            &mfm_facts::FactCanonicalScalar::UnsignedInteger(1000)
        );
        assert_eq!(predicates[2].field_id().as_str(), "metadata.observed_at");
        assert_eq!(
            predicates[2].operator(),
            mfm_facts::FactQueryOperator::LessThanOrEqual
        );
        assert_eq!(
            predicates[2].value(),
            &mfm_facts::FactCanonicalScalar::Timestamp("2026-07-02T00:00:00Z".to_owned())
        );
    }

    #[test]
    fn selector_builds_typed_request_and_rejects_front_door_invariants() {
        let base_selector = PublicFactQuerySelector {
            return_fields: vec!["result.amount".to_owned()],
            ordering: Some("result.amount.asc".to_owned()),
            limit: Some(10),
            ..PublicFactQuerySelector::default()
        };

        let base =
            PublicFactQueryRequest::from_selector("mfm.app.test.launch", base_selector.clone())
                .expect("base request");
        assert_eq!(base.fact_kind.as_str(), "mfm.app.test.launch");
        assert_eq!(base.return_fields[0].as_str(), "result.amount");
        assert_eq!(base.ordering.as_str(), "result.amount.asc");
        assert_eq!(base.limit.map(NonZeroU64::get), Some(10));

        let mut missing_order = base_selector.clone();
        missing_order.ordering = None;
        assert_eq!(
            PublicFactQueryRequest::from_selector("mfm.app.test.launch", missing_order)
                .expect_err("missing order")
                .code,
            "FactOrderingMissing"
        );

        let mut zero_limit = base_selector;
        zero_limit.limit = Some(0);
        assert_eq!(
            PublicFactQueryRequest::from_selector("mfm.app.test.launch", zero_limit)
                .expect_err("zero limit")
                .code,
            "FactQueryLimitInvalid"
        );
    }

    #[test]
    fn selector_parses_public_fact_query_pairs() {
        let selector = PublicFactQuerySelector::from_query_pairs([
            ("shape", "mfm.app.test.shape"),
            ("subject", "account=alice"),
            ("result", "amount.gte=u64:10"),
            ("where", "result.status=settled"),
            ("field", "result.amount"),
            ("fields", "result.status"),
            ("order", "result.amount.asc"),
            ("limit", "25"),
        ])
        .expect("query selector");

        assert_eq!(selector.shape.as_deref(), Some("mfm.app.test.shape"));
        assert_eq!(selector.subject_predicates, ["account=alice"]);
        assert_eq!(selector.result_predicates, ["amount.gte=u64:10"]);
        assert_eq!(selector.field_predicates, ["result.status=settled"]);
        assert_eq!(
            selector.return_fields,
            ["result.amount".to_owned(), "result.status".to_owned()]
        );
        assert_eq!(selector.ordering.as_deref(), Some("result.amount.asc"));
        assert_eq!(selector.limit, Some(25));
        assert_eq!(selector.with_limit_override(Some(1)).limit, Some(1));
        assert_eq!(
            PublicFactQuerySelector::from_query_pairs([("unknown", "value")])
                .expect_err("unknown key")
                .code,
            "FactQueryInvalidParameter"
        );
        assert_eq!(
            PublicFactQuerySelector::from_query_pairs([("limit", "not-a-number")])
                .expect_err("invalid limit")
                .code,
            "FactQueryInvalidParameter"
        );
    }
}
