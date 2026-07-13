use super::*;

pub(super) fn parse_vec<T>(
    value: &serde_json::Value,
    parser: fn(&serde_json::Value) -> Result<T>,
) -> Result<Vec<T>> {
    array(value, "array")?.iter().map(parser).collect()
}

pub(super) fn parse_vec_with<T>(
    value: &serde_json::Value,
    parser: impl Fn(&serde_json::Value) -> Result<T>,
) -> Result<Vec<T>> {
    array(value, "array")?.iter().map(parser).collect()
}

pub(super) fn parse_identity_vec<T>(value: &serde_json::Value) -> Result<Vec<T>>
where
    T: std::str::FromStr,
    T::Err: std::fmt::Display,
{
    array(value, "identity array")?
        .iter()
        .map(|value| identity(string(value, "identity")?))
        .collect()
}

pub(super) fn optional_parse<T>(
    object: &serde_json::Map<String, serde_json::Value>,
    field: &'static str,
    parser: fn(&serde_json::Value) -> Result<T>,
) -> Result<Option<T>> {
    match object.get(field) {
        Some(serde_json::Value::Null) | None => Ok(None),
        Some(value) => parser(value).map(Some),
    }
}

pub(super) fn optional_parse_with<T>(
    object: &serde_json::Map<String, serde_json::Value>,
    field: &'static str,
    parser: impl Fn(&serde_json::Value) -> Result<T>,
) -> Result<Option<T>> {
    match object.get(field) {
        Some(serde_json::Value::Null) | None => Ok(None),
        Some(value) => parser(value).map(Some),
    }
}

pub(super) fn optional_identity<T>(
    object: &serde_json::Map<String, serde_json::Value>,
    field: &'static str,
) -> Result<Option<T>>
where
    T: std::str::FromStr,
    T::Err: std::fmt::Display,
{
    match object.get(field) {
        Some(serde_json::Value::Null) | None => Ok(None),
        Some(value) => identity(string(value, field)?).map(Some),
    }
}

pub(super) fn identity<T>(value: &str) -> Result<T>
where
    T: std::str::FromStr,
    T::Err: std::fmt::Display,
{
    parse_string(value)
}

pub(super) fn version<T>(value: &str) -> Result<T>
where
    T: std::str::FromStr,
    T::Err: std::fmt::Display,
{
    parse_string(value)
}

pub(super) fn parse_string<T>(value: &str) -> Result<T>
where
    T: std::str::FromStr,
    T::Err: std::fmt::Display,
{
    match value.parse::<T>() {
        Ok(parsed) => Ok(parsed),
        Err(error) => Err(SpecError::Identity(error.to_string())),
    }
}

pub(super) fn object<'a>(
    value: &'a serde_json::Value,
    context: &'static str,
) -> Result<&'a serde_json::Map<String, serde_json::Value>> {
    value
        .as_object()
        .ok_or_else(|| json_error(format!("{context} must be a JSON object")))
}

pub(super) fn array<'a>(
    value: &'a serde_json::Value,
    context: &'static str,
) -> Result<&'a Vec<serde_json::Value>> {
    value
        .as_array()
        .ok_or_else(|| json_error(format!("{context} must be a JSON array")))
}

pub(super) fn required<'a>(
    object: &'a serde_json::Map<String, serde_json::Value>,
    field: &'static str,
) -> Result<&'a serde_json::Value> {
    object
        .get(field)
        .ok_or_else(|| json_error(format!("missing required field {field}")))
}

pub(super) fn required_str<'a>(
    object: &'a serde_json::Map<String, serde_json::Value>,
    field: &'static str,
) -> Result<&'a str> {
    string(required(object, field)?, field)
}

pub(super) fn required_u64(
    object: &serde_json::Map<String, serde_json::Value>,
    field: &'static str,
) -> Result<u64> {
    required(object, field)?
        .as_u64()
        .ok_or_else(|| json_error(format!("{field} must be an unsigned integer")))
}

pub(super) fn required_bool(
    object: &serde_json::Map<String, serde_json::Value>,
    field: &'static str,
) -> Result<bool> {
    required(object, field)?
        .as_bool()
        .ok_or_else(|| json_error(format!("{field} must be a boolean")))
}

pub(super) fn string<'a>(value: &'a serde_json::Value, field: &'static str) -> Result<&'a str> {
    value
        .as_str()
        .ok_or_else(|| json_error(format!("{field} must be a string")))
}
