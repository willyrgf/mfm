#![warn(missing_docs)]
//! Canonicalization contracts for the MFM typed kernel.
//!
//! This crate owns the first typed-core canonical JSON contract. Typed persisted
//! surfaces use [`CanonicalValue`], whose bytes and decimals are explicit typed
//! variants with checked constructors. Raw JSON parsing is available through
//! [`PlainCanonicalJsonBytes`] only for already-lowered framework payloads; it
//! does not infer bytes or decimals from ordinary JSON strings.
//!
//! ```
//! use mfm_canonical::{CanonicalJsonBytes, CanonicalValue};
//!
//! let value = CanonicalValue::object([
//!     ("b", CanonicalValue::Unsigned(2)),
//!     ("a", CanonicalValue::Unsigned(1)),
//! ])?;
//! let canonical = CanonicalJsonBytes::from_value(&value);
//! assert_eq!(canonical.as_str(), r#"{"a":1,"b":2}"#);
//! assert_eq!(
//!     canonical.content_digest().as_str(),
//!     "content:sha256-jcs-v1:43258cff783fe7036d8a43033f830adfc60ec037382473548ac742b888292777"
//! );
//! # Ok::<(), mfm_canonical::CanonicalError>(())
//! ```
//!
//! ```compile_fail
//! // Raw JSON is intentionally not a typed persisted-value canonicalizer.
//! let _ = mfm_canonical::CanonicalJsonBytes::from_json_str("{}");
//! ```

use std::cmp::Ordering;
use std::collections::BTreeSet;
use std::fmt;

use mfm_ids::{ContentDigest, DigestAlgorithm, DigestBytes};
use ring::digest::{digest, SHA256};
use serde::de::{self, Deserialize, Deserializer, Error as _, MapAccess, SeqAccess, Visitor};

/// Result type for canonicalization operations.
pub type Result<T> = std::result::Result<T, CanonicalError>;

/// Error returned when canonical JSON, decimal, or byte grammar validation
/// fails.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalError {
    message: String,
}

impl CanonicalError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    /// Returns a stable human-readable diagnostic.
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for CanonicalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for CanonicalError {}

/// Canonical JSON bytes that have passed the typed-kernel v1
/// canonicalization contract.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CanonicalJsonBytes {
    bytes: String,
}

impl CanonicalJsonBytes {
    /// Serializes a typed canonical value into canonical JSON bytes.
    pub fn from_value(value: &CanonicalValue) -> Self {
        let mut bytes = String::new();
        value.write_json(&mut bytes);
        Self { bytes }
    }

    /// Returns the canonical JSON bytes.
    pub fn as_bytes(&self) -> &[u8] {
        self.bytes.as_bytes()
    }

    /// Returns the canonical JSON as a string slice.
    pub fn as_str(&self) -> &str {
        &self.bytes
    }

    /// Copies the canonical JSON bytes into an owned vector.
    pub fn to_vec(&self) -> Vec<u8> {
        self.bytes.as_bytes().to_vec()
    }

    /// Computes the raw SHA-256 digest of these canonical bytes.
    pub fn digest_bytes(&self) -> DigestBytes {
        sha256_digest_bytes(self.as_bytes())
    }

    /// Computes a typed content digest using `sha256-jcs-v1`.
    pub fn content_digest(&self) -> ContentDigest {
        ContentDigest::from_digest(DigestAlgorithm::Sha256JcsV1, self.digest_bytes())
    }
}

/// Canonical JSON bytes parsed from plain JSON text.
///
/// This wrapper is for framework-owned payloads whose typed/schema semantics
/// have already been established elsewhere, such as lowered certified specs.
/// It canonicalizes JSON syntax and rejects duplicate keys, floats, and
/// unsupported number spellings. It does not reinterpret ordinary JSON strings
/// as typed bytes or decimals; typed persisted value surfaces must use
/// [`CanonicalJsonBytes::from_value`] with [`CanonicalBytes`] and
/// [`DecimalString`] constructors.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PlainCanonicalJsonBytes {
    bytes: String,
}

impl PlainCanonicalJsonBytes {
    /// Parses plain JSON text, rejects unsupported grammar, and returns
    /// canonical bytes.
    pub fn from_json_str(input: &str) -> Result<Self> {
        validate_number_tokens(input)?;
        let value: PlainJsonValue = serde_json::from_str(input)
            .map_err(|error| CanonicalError::new(format!("invalid canonical JSON: {error}")))?;
        Ok(Self::from_value(&value))
    }

    /// Validates that the supplied plain JSON bytes are already canonical.
    pub fn from_canonical_json_slice(bytes: &[u8]) -> Result<Self> {
        let input = std::str::from_utf8(bytes).map_err(|error| {
            CanonicalError::new(format!("canonical JSON must be UTF-8: {error}"))
        })?;
        let canonical = Self::from_json_str(input)?;
        if canonical.as_bytes() != bytes {
            return Err(CanonicalError::new(
                "JSON bytes are valid but not in canonical form",
            ));
        }
        Ok(canonical)
    }

    fn from_value(value: &PlainJsonValue) -> Self {
        let mut bytes = String::new();
        value.write_json(&mut bytes);
        Self { bytes }
    }

    /// Returns the canonical JSON bytes.
    pub fn as_bytes(&self) -> &[u8] {
        self.bytes.as_bytes()
    }

    /// Returns the canonical JSON as a string slice.
    pub fn as_str(&self) -> &str {
        &self.bytes
    }

    /// Copies the canonical JSON bytes into an owned vector.
    pub fn to_vec(&self) -> Vec<u8> {
        self.bytes.as_bytes().to_vec()
    }

    /// Computes the raw SHA-256 digest of these canonical bytes.
    pub fn digest_bytes(&self) -> DigestBytes {
        sha256_digest_bytes(self.as_bytes())
    }

    /// Computes a typed content digest using `sha256-jcs-v1`.
    pub fn content_digest(&self) -> ContentDigest {
        ContentDigest::from_digest(DigestAlgorithm::Sha256JcsV1, self.digest_bytes())
    }
}

impl fmt::Debug for PlainCanonicalJsonBytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("PlainCanonicalJsonBytes")
            .field(&self.bytes)
            .finish()
    }
}

impl fmt::Display for PlainCanonicalJsonBytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.bytes)
    }
}

impl fmt::Debug for CanonicalJsonBytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("CanonicalJsonBytes")
            .field(&self.bytes)
            .finish()
    }
}

impl fmt::Display for CanonicalJsonBytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.bytes)
    }
}

/// Typed value tree accepted by the canonical JSON writer.
///
/// Bytes and decimals serialize as JSON strings. Their Rust variants remain
/// separate so descriptor code can distinguish plain strings from typed
/// base64url bytes and decimal-string fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CanonicalValue {
    /// JSON null.
    Null,
    /// JSON boolean.
    Bool(bool),
    /// JSON string.
    String(String),
    /// Binary data encoded as base64url without padding.
    Bytes(CanonicalBytes),
    /// Signed integer JSON number.
    Signed(i64),
    /// Unsigned integer JSON number.
    Unsigned(u64),
    /// Decimal string with RFC-validated spelling.
    Decimal(DecimalString),
    /// JSON array.
    Array(Vec<CanonicalValue>),
    /// JSON object with duplicate-free, canonical UTF-16 key ordering.
    Object(CanonicalObject),
}

impl CanonicalValue {
    /// Builds a duplicate-free canonical object from key/value pairs.
    pub fn object(
        entries: impl IntoIterator<Item = (impl Into<String>, CanonicalValue)>,
    ) -> Result<Self> {
        Ok(Self::Object(CanonicalObject::new(entries)?))
    }

    fn write_json(&self, out: &mut String) {
        match self {
            Self::Null => out.push_str("null"),
            Self::Bool(value) => out.push_str(if *value { "true" } else { "false" }),
            Self::String(value) => write_json_string(value, out),
            Self::Bytes(value) => write_json_string(value.encoded(), out),
            Self::Signed(value) => out.push_str(&value.to_string()),
            Self::Unsigned(value) => out.push_str(&value.to_string()),
            Self::Decimal(value) => write_json_string(value.as_str(), out),
            Self::Array(values) => {
                out.push('[');
                for (index, value) in values.iter().enumerate() {
                    if index > 0 {
                        out.push(',');
                    }
                    value.write_json(out);
                }
                out.push(']');
            }
            Self::Object(object) => {
                out.push('{');
                for (index, entry) in object.entries.iter().enumerate() {
                    if index > 0 {
                        out.push(',');
                    }
                    write_json_string(&entry.key, out);
                    out.push(':');
                    entry.value.write_json(out);
                }
                out.push('}');
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PlainJsonValue {
    Null,
    Bool(bool),
    String(String),
    Signed(i64),
    Unsigned(u64),
    Array(Vec<PlainJsonValue>),
    Object(PlainJsonObject),
}

impl PlainJsonValue {
    fn write_json(&self, out: &mut String) {
        match self {
            Self::Null => out.push_str("null"),
            Self::Bool(value) => out.push_str(if *value { "true" } else { "false" }),
            Self::String(value) => write_json_string(value, out),
            Self::Signed(value) => out.push_str(&value.to_string()),
            Self::Unsigned(value) => out.push_str(&value.to_string()),
            Self::Array(values) => {
                out.push('[');
                for (index, value) in values.iter().enumerate() {
                    if index > 0 {
                        out.push(',');
                    }
                    value.write_json(out);
                }
                out.push(']');
            }
            Self::Object(object) => {
                out.push('{');
                for (index, entry) in object.entries.iter().enumerate() {
                    if index > 0 {
                        out.push(',');
                    }
                    write_json_string(&entry.key, out);
                    out.push(':');
                    entry.value.write_json(out);
                }
                out.push('}');
            }
        }
    }
}

impl<'de> Deserialize<'de> for PlainJsonValue {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(PlainJsonValueVisitor)
    }
}

struct PlainJsonValueVisitor;

impl<'de> Visitor<'de> for PlainJsonValueVisitor {
    type Value = PlainJsonValue;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("canonical JSON without floats or duplicate object keys")
    }

    fn visit_unit<E>(self) -> std::result::Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(PlainJsonValue::Null)
    }

    fn visit_bool<E>(self, value: bool) -> std::result::Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(PlainJsonValue::Bool(value))
    }

    fn visit_i64<E>(self, value: i64) -> std::result::Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(PlainJsonValue::Signed(value))
    }

    fn visit_u64<E>(self, value: u64) -> std::result::Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(PlainJsonValue::Unsigned(value))
    }

    fn visit_i128<E>(self, _value: i128) -> std::result::Result<Self::Value, E>
    where
        E: de::Error,
    {
        Err(E::custom("integers outside i64/u64 are unsupported"))
    }

    fn visit_u128<E>(self, _value: u128) -> std::result::Result<Self::Value, E>
    where
        E: de::Error,
    {
        Err(E::custom("integers outside i64/u64 are unsupported"))
    }

    fn visit_f32<E>(self, _value: f32) -> std::result::Result<Self::Value, E>
    where
        E: de::Error,
    {
        Err(E::custom("floats are not allowed in canonical JSON"))
    }

    fn visit_f64<E>(self, _value: f64) -> std::result::Result<Self::Value, E>
    where
        E: de::Error,
    {
        Err(E::custom("floats are not allowed in canonical JSON"))
    }

    fn visit_str<E>(self, value: &str) -> std::result::Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(PlainJsonValue::String(value.to_owned()))
    }

    fn visit_string<E>(self, value: String) -> std::result::Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(PlainJsonValue::String(value))
    }

    fn visit_none<E>(self) -> std::result::Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(PlainJsonValue::Null)
    }

    fn visit_some<D>(self, deserializer: D) -> std::result::Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        PlainJsonValue::deserialize(deserializer)
    }

    fn visit_seq<A>(self, mut seq: A) -> std::result::Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut values = Vec::new();
        while let Some(value) = seq.next_element()? {
            values.push(value);
        }
        Ok(PlainJsonValue::Array(values))
    }

    fn visit_map<A>(self, mut map: A) -> std::result::Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut entries = Vec::new();
        let mut seen = BTreeSet::new();
        while let Some((key, value)) = map.next_entry::<String, PlainJsonValue>()? {
            if !seen.insert(key.clone()) {
                return Err(A::Error::custom(format!("duplicate object key '{key}'")));
            }
            entries.push((key, value));
        }
        PlainJsonObject::new(entries)
            .map(PlainJsonValue::Object)
            .map_err(A::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PlainJsonObject {
    entries: Vec<PlainJsonObjectEntry>,
}

impl PlainJsonObject {
    fn new(entries: impl IntoIterator<Item = (impl Into<String>, PlainJsonValue)>) -> Result<Self> {
        let mut seen = BTreeSet::new();
        let mut entries = entries
            .into_iter()
            .map(|(key, value)| {
                let key = key.into();
                if !seen.insert(key.clone()) {
                    return Err(CanonicalError::new(format!("duplicate object key '{key}'")));
                }
                Ok(PlainJsonObjectEntry { key, value })
            })
            .collect::<Result<Vec<_>>>()?;

        entries.sort_by(|left, right| compare_json_keys(&left.key, &right.key));
        Ok(Self { entries })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PlainJsonObjectEntry {
    key: String,
    value: PlainJsonValue,
}

/// Duplicate-free canonical object entries sorted by RFC 8785 UTF-16 key order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalObject {
    entries: Vec<CanonicalObjectEntry>,
}

impl CanonicalObject {
    /// Builds a canonical object from key/value pairs, rejecting duplicate keys.
    pub fn new(
        entries: impl IntoIterator<Item = (impl Into<String>, CanonicalValue)>,
    ) -> Result<Self> {
        let mut seen = BTreeSet::new();
        let mut entries = entries
            .into_iter()
            .map(|(key, value)| {
                let key = key.into();
                if !seen.insert(key.clone()) {
                    return Err(CanonicalError::new(format!("duplicate object key '{key}'")));
                }
                Ok(CanonicalObjectEntry { key, value })
            })
            .collect::<Result<Vec<_>>>()?;

        entries.sort_by(|left, right| compare_json_keys(&left.key, &right.key));
        Ok(Self { entries })
    }

    /// Returns the sorted object entries.
    pub fn entries(&self) -> impl Iterator<Item = (&str, &CanonicalValue)> {
        self.entries
            .iter()
            .map(|entry| (entry.key.as_str(), &entry.value))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CanonicalObjectEntry {
    key: String,
    value: CanonicalValue,
}

/// Binary data encoded as base64url without padding in canonical JSON.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CanonicalBytes {
    bytes: Vec<u8>,
    encoded: String,
}

impl CanonicalBytes {
    /// Creates canonical bytes from raw binary data.
    pub fn new(bytes: impl Into<Vec<u8>>) -> Self {
        let bytes = bytes.into();
        let encoded = encode_base64url_no_pad(&bytes);
        Self { bytes, encoded }
    }

    /// Parses base64url-without-padding bytes and rejects any non-canonical
    /// spelling.
    pub fn from_base64url_no_pad(value: impl Into<String>) -> Result<Self> {
        let encoded = value.into();
        validate_base64url_no_pad(&encoded)?;
        let bytes = decode_base64url_no_pad(&encoded)?;
        Ok(Self { bytes, encoded })
    }

    /// Returns the raw bytes.
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Returns the base64url-without-padding spelling.
    pub fn encoded(&self) -> &str {
        &self.encoded
    }
}

/// Decimal string accepted by typed-kernel persisted surfaces.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DecimalString {
    value: String,
}

impl DecimalString {
    /// Creates a variable-scale decimal string using the RFC grammar.
    pub fn new_variable(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        validate_variable_decimal(&value)?;
        Ok(Self { value })
    }

    /// Creates a fixed-scale decimal string using the RFC grammar.
    pub fn new_fixed(value: impl Into<String>, scale: usize) -> Result<Self> {
        let value = value.into();
        validate_fixed_decimal(&value, scale)?;
        Ok(Self { value })
    }

    /// Returns the canonical decimal spelling.
    pub fn as_str(&self) -> &str {
        &self.value
    }
}

impl fmt::Display for DecimalString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.value)
    }
}

/// Computes SHA-256 digest bytes for canonical or artifact bytes.
pub fn sha256_digest_bytes(bytes: &[u8]) -> DigestBytes {
    let digest = digest(&SHA256, bytes);
    let mut output = [0_u8; 32];
    output.copy_from_slice(digest.as_ref());
    DigestBytes::from_array(output)
}

fn validate_number_tokens(input: &str) -> Result<()> {
    let bytes = input.as_bytes();
    let mut index = 0;
    let mut in_string = false;
    let mut escaped = false;

    while index < bytes.len() {
        let byte = bytes[index];

        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
            index += 1;
            continue;
        }

        if byte == b'"' {
            in_string = true;
            index += 1;
            continue;
        }

        if byte == b'-' || byte.is_ascii_digit() {
            index = validate_number_token(bytes, index)?;
            continue;
        }

        index += 1;
    }

    Ok(())
}

fn validate_number_token(bytes: &[u8], start: usize) -> Result<usize> {
    let mut index = start;
    let negative = bytes[index] == b'-';
    if negative {
        index += 1;
        if index == bytes.len() || !bytes[index].is_ascii_digit() {
            return Ok(index);
        }
    }

    let integer_start = index;
    if bytes[index] == b'0' {
        index += 1;
        if index < bytes.len() && bytes[index].is_ascii_digit() {
            return Err(CanonicalError::new("leading zeroes are not allowed"));
        }
        if negative {
            return Err(CanonicalError::new("negative zero is not allowed"));
        }
    } else {
        index += 1;
        while index < bytes.len() && bytes[index].is_ascii_digit() {
            index += 1;
        }
    }

    if index < bytes.len() && matches!(bytes[index], b'.' | b'e' | b'E') {
        return Err(CanonicalError::new(
            "floats and exponent number forms are not allowed",
        ));
    }

    if integer_start == index {
        return Ok(index);
    }

    Ok(index)
}

fn validate_variable_decimal(value: &str) -> Result<()> {
    if value == "0" {
        return Ok(());
    }

    let Some(rest) = value.strip_prefix('-') else {
        return validate_unsigned_variable_decimal(value);
    };

    if rest.is_empty() {
        return Err(CanonicalError::new("decimal must not be empty"));
    }
    if rest == "0" {
        return Err(CanonicalError::new("negative zero decimal is not allowed"));
    }
    if let Some(fraction) = rest.strip_prefix("0.") {
        return validate_nonzero_fraction(fraction);
    }

    validate_unsigned_variable_decimal(rest)
}

fn validate_unsigned_variable_decimal(value: &str) -> Result<()> {
    if value.is_empty() {
        return Err(CanonicalError::new("decimal must not be empty"));
    }
    if value.starts_with('0') {
        return Err(CanonicalError::new(
            "variable decimal has a leading zero or unsupported zero fraction",
        ));
    }

    let mut parts = value.split('.');
    let integer = parts.next().expect("split always yields first part");
    let fraction = parts.next();
    if parts.next().is_some() {
        return Err(CanonicalError::new("decimal has multiple decimal points"));
    }

    validate_nonzero_integer(integer)?;
    if let Some(fraction) = fraction {
        validate_nonzero_fraction(fraction)?;
    }
    Ok(())
}

fn validate_fixed_decimal(value: &str, scale: usize) -> Result<()> {
    if scale == 0 {
        return Err(CanonicalError::new(
            "fixed decimal scale must be greater than zero",
        ));
    }

    let (negative, unsigned) = if let Some(rest) = value.strip_prefix('-') {
        (true, rest)
    } else {
        (false, value)
    };

    let mut parts = unsigned.split('.');
    let integer = parts.next().expect("split always yields first part");
    let fraction = parts
        .next()
        .ok_or_else(|| CanonicalError::new("fixed decimal must contain a decimal point"))?;
    if parts.next().is_some() {
        return Err(CanonicalError::new("decimal has multiple decimal points"));
    }

    validate_unsigned_integer_component(integer)?;
    if fraction.len() != scale || !fraction.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(CanonicalError::new(format!(
            "fixed decimal must have exactly {scale} fractional digits"
        )));
    }

    if negative
        && integer.bytes().all(|byte| byte == b'0')
        && fraction.bytes().all(|byte| byte == b'0')
    {
        return Err(CanonicalError::new("negative zero decimal is not allowed"));
    }

    Ok(())
}

fn validate_nonzero_integer(value: &str) -> Result<()> {
    validate_unsigned_integer_component(value)?;
    if value == "0" {
        return Err(CanonicalError::new("integer component must be non-zero"));
    }
    Ok(())
}

fn validate_unsigned_integer_component(value: &str) -> Result<()> {
    if value.is_empty() {
        return Err(CanonicalError::new("integer component must not be empty"));
    }
    if !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(CanonicalError::new(
            "integer component must contain only digits",
        ));
    }
    if value.len() > 1 && value.starts_with('0') {
        return Err(CanonicalError::new("leading zeroes are not allowed"));
    }
    Ok(())
}

fn validate_nonzero_fraction(value: &str) -> Result<()> {
    if value.is_empty() {
        return Err(CanonicalError::new(
            "fractional component must not be empty",
        ));
    }
    if !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(CanonicalError::new(
            "fractional component must contain only digits",
        ));
    }
    if value.ends_with('0') {
        return Err(CanonicalError::new(
            "variable decimal fractional component must end in a non-zero digit",
        ));
    }
    if !value.bytes().any(|byte| byte != b'0') {
        return Err(CanonicalError::new("decimal must contain a non-zero digit"));
    }
    Ok(())
}

fn compare_json_keys(left: &str, right: &str) -> Ordering {
    left.encode_utf16().cmp(right.encode_utf16())
}

fn write_json_string(value: &str, out: &mut String) {
    let encoded = serde_json::to_string(value)
        .expect("serializing a string into an owned buffer cannot fail");
    out.push_str(&encoded);
}

fn encode_base64url_no_pad(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

    let mut output = String::with_capacity((bytes.len() * 4).div_ceil(3));
    let mut chunks = bytes.chunks_exact(3);
    for chunk in &mut chunks {
        let triple = ((chunk[0] as u32) << 16) | ((chunk[1] as u32) << 8) | (chunk[2] as u32);
        output.push(TABLE[((triple >> 18) & 0x3f) as usize] as char);
        output.push(TABLE[((triple >> 12) & 0x3f) as usize] as char);
        output.push(TABLE[((triple >> 6) & 0x3f) as usize] as char);
        output.push(TABLE[(triple & 0x3f) as usize] as char);
    }

    let remainder = chunks.remainder();
    if remainder.len() == 1 {
        let triple = (remainder[0] as u32) << 16;
        output.push(TABLE[((triple >> 18) & 0x3f) as usize] as char);
        output.push(TABLE[((triple >> 12) & 0x3f) as usize] as char);
    } else if remainder.len() == 2 {
        let triple = ((remainder[0] as u32) << 16) | ((remainder[1] as u32) << 8);
        output.push(TABLE[((triple >> 18) & 0x3f) as usize] as char);
        output.push(TABLE[((triple >> 12) & 0x3f) as usize] as char);
        output.push(TABLE[((triple >> 6) & 0x3f) as usize] as char);
    }

    output
}

fn validate_base64url_no_pad(value: &str) -> Result<()> {
    if value.contains('=') {
        return Err(CanonicalError::new(
            "base64url bytes must not contain padding",
        ));
    }
    if value.len() % 4 == 1 {
        return Err(CanonicalError::new(
            "base64url bytes have an invalid unpadded length",
        ));
    }
    if let Some(ch) = value
        .bytes()
        .find(|byte| decode_base64url_char(*byte).is_none())
    {
        return Err(CanonicalError::new(format!(
            "base64url bytes contain invalid character '{}'",
            ch as char
        )));
    }
    Ok(())
}

fn decode_base64url_no_pad(value: &str) -> Result<Vec<u8>> {
    validate_base64url_no_pad(value)?;

    let bytes = value.as_bytes();
    let mut output = Vec::with_capacity(value.len() * 3 / 4);
    let mut chunks = bytes.chunks_exact(4);
    for chunk in &mut chunks {
        let a = decode_base64url_char(chunk[0]).expect("validated base64url char") as u32;
        let b = decode_base64url_char(chunk[1]).expect("validated base64url char") as u32;
        let c = decode_base64url_char(chunk[2]).expect("validated base64url char") as u32;
        let d = decode_base64url_char(chunk[3]).expect("validated base64url char") as u32;
        let triple = (a << 18) | (b << 12) | (c << 6) | d;
        output.push(((triple >> 16) & 0xff) as u8);
        output.push(((triple >> 8) & 0xff) as u8);
        output.push((triple & 0xff) as u8);
    }

    let remainder = chunks.remainder();
    if remainder.len() == 2 {
        let a = decode_base64url_char(remainder[0]).expect("validated base64url char") as u32;
        let b = decode_base64url_char(remainder[1]).expect("validated base64url char") as u32;
        let pair = (a << 6) | b;
        output.push(((pair >> 4) & 0xff) as u8);
    } else if remainder.len() == 3 {
        let a = decode_base64url_char(remainder[0]).expect("validated base64url char") as u32;
        let b = decode_base64url_char(remainder[1]).expect("validated base64url char") as u32;
        let c = decode_base64url_char(remainder[2]).expect("validated base64url char") as u32;
        let triple = (a << 12) | (b << 6) | c;
        output.push(((triple >> 10) & 0xff) as u8);
        output.push(((triple >> 2) & 0xff) as u8);
    }

    if encode_base64url_no_pad(&output) != value {
        return Err(CanonicalError::new(
            "base64url bytes are not in canonical no-padding form",
        ));
    }

    Ok(output)
}

fn decode_base64url_char(byte: u8) -> Option<u8> {
    match byte {
        b'A'..=b'Z' => Some(byte - b'A'),
        b'a'..=b'z' => Some(byte - b'a' + 26),
        b'0'..=b'9' => Some(byte - b'0' + 52),
        b'-' => Some(62),
        b'_' => Some(63),
        _ => None,
    }
}

#[cfg(test)]
mod tests;
