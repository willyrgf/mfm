use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use serde::de::{self, Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};
use serde_json::{Map, Number, Value};

use super::value::read_bounded_file;
use super::{Result, RuntimeConfigError, RuntimeConfigErrorKind};

const MAX_CONFIG_DOCUMENT_BYTES: usize = 1024 * 1024;
const MAX_RUNTIME_PATH_BYTES: usize = 4_096;
const TOP_LEVEL_SECTIONS: [&str; 4] = ["bitcoin", "evm", "signers", "keystores"];
const SECRET_MARKERS: [&str; 16] = [
    "password",
    "passphrase",
    "mnemonic",
    "seed_phrase",
    "private_key",
    "secret",
    "credential",
    "api_key",
    "access_key",
    "token",
    "authorization",
    "signed_material",
    "signed_transaction",
    "raw_transaction",
    "raw_tx",
    "signature_scalar",
];

#[derive(Clone, Copy)]
enum RuntimeConfigFormat {
    Toml,
    Json,
}

pub(super) struct RuntimeDocument {
    root: Map<String, Value>,
}

impl RuntimeDocument {
    pub(super) fn load(path: &Path) -> Result<Self> {
        let format = format_from_path(path)?;
        let file = fs::File::open(path)
            .map_err(|_| RuntimeConfigError::new(RuntimeConfigErrorKind::DocumentRead))?;
        let bytes = read_bounded_file(
            file,
            MAX_CONFIG_DOCUMENT_BYTES,
            RuntimeConfigErrorKind::DocumentRead,
            RuntimeConfigErrorKind::DocumentTooLarge,
        )?;
        let raw = bytes.as_utf8()?;
        let value = parse_document(raw, format)?;
        let Value::Object(root) = value else {
            return Err(RuntimeConfigError::new(RuntimeConfigErrorKind::Syntax));
        };
        for key in root.keys() {
            if !TOP_LEVEL_SECTIONS.contains(&key.as_str()) {
                return Err(RuntimeConfigError::new(
                    RuntimeConfigErrorKind::UnknownTopLevel,
                ));
            }
        }
        let root = Value::Object(root);
        enforce_global_field_policy(&root)?;
        let Value::Object(root) = root else {
            unreachable!("root was already checked as an object")
        };
        Ok(Self { root })
    }

    pub(super) fn take_section(mut self, section: &'static str) -> Result<Value> {
        self.root
            .remove(section)
            .ok_or_else(|| RuntimeConfigError::new(RuntimeConfigErrorKind::MissingSection))
    }

    pub(super) fn take_entry(&mut self, section: &'static str, key: &str) -> Result<Value> {
        let section = self
            .root
            .remove(section)
            .ok_or_else(|| RuntimeConfigError::new(RuntimeConfigErrorKind::MissingSection))?;
        let Value::Object(mut entries) = section else {
            return Err(RuntimeConfigError::new(
                RuntimeConfigErrorKind::InvalidSelectedObject,
            ));
        };
        entries
            .remove(key)
            .ok_or_else(|| RuntimeConfigError::new(RuntimeConfigErrorKind::MissingEntry))
    }
}

fn format_from_path(path: &Path) -> Result<RuntimeConfigFormat> {
    let encoded = path
        .to_str()
        .filter(|value| !value.is_empty() && value.len() <= MAX_RUNTIME_PATH_BYTES)
        .ok_or_else(|| RuntimeConfigError::new(RuntimeConfigErrorKind::InvalidPath))?;
    let extension = Path::new(encoded)
        .extension()
        .and_then(|value| value.to_str());
    match extension {
        Some("toml") => Ok(RuntimeConfigFormat::Toml),
        Some("json") => Ok(RuntimeConfigFormat::Json),
        _ => Err(RuntimeConfigError::new(
            RuntimeConfigErrorKind::UnsupportedFormat,
        )),
    }
}

fn parse_document(raw: &str, format: RuntimeConfigFormat) -> Result<Value> {
    match format {
        RuntimeConfigFormat::Toml => {
            let value: toml::Value = toml::from_str(raw)
                .map_err(|_| RuntimeConfigError::new(RuntimeConfigErrorKind::Syntax))?;
            serde_json::to_value(value)
                .map_err(|_| RuntimeConfigError::new(RuntimeConfigErrorKind::Syntax))
        }
        RuntimeConfigFormat::Json => {
            let mut deserializer = serde_json::Deserializer::from_str(raw);
            let value = UniqueJsonValue::deserialize(&mut deserializer)
                .map_err(|error| {
                    if error.to_string().contains("duplicate object key") {
                        RuntimeConfigError::new(RuntimeConfigErrorKind::DuplicateJsonKey)
                    } else {
                        RuntimeConfigError::new(RuntimeConfigErrorKind::Syntax)
                    }
                })?
                .0;
            deserializer
                .end()
                .map_err(|_| RuntimeConfigError::new(RuntimeConfigErrorKind::Syntax))?;
            Ok(value)
        }
    }
}

fn enforce_global_field_policy(value: &Value) -> Result<()> {
    let mut path = Vec::new();
    scan_object(value, &mut path)
}

fn scan_object(value: &Value, path: &mut Vec<String>) -> Result<()> {
    match value {
        Value::Object(object) => {
            let entry_keys = is_entry_map(path);
            for (raw_key, child) in object {
                let normalized = normalize_field(raw_key);
                let path_key = if entry_keys {
                    "*".to_owned()
                } else {
                    normalized.clone()
                };
                path.push(path_key);
                if !entry_keys {
                    if normalized == "expected_chain_id" {
                        return Err(RuntimeConfigError::new(
                            RuntimeConfigErrorKind::ForbiddenExpectedChainId,
                        ));
                    }
                    let reviewed_secret = is_reviewed_secret_slot(path);
                    if !reviewed_secret
                        && SECRET_MARKERS
                            .iter()
                            .any(|marker| normalized.contains(marker))
                    {
                        return Err(RuntimeConfigError::new(
                            RuntimeConfigErrorKind::ForbiddenSecretField,
                        ));
                    }
                    if reviewed_secret && !is_indirect_secret_source_shape(child) {
                        return Err(RuntimeConfigError::new(
                            RuntimeConfigErrorKind::DirectSecretValue,
                        ));
                    }
                }
                scan_object(child, path)?;
                path.pop();
            }
            Ok(())
        }
        Value::Array(values) => {
            for value in values {
                scan_object(value, path)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn is_entry_map(path: &[String]) -> bool {
    matches!(
        path,
        [section] if section == "signers" || section == "keystores"
    ) || matches!(
        path,
        [family, routes]
            if (family == "bitcoin" || family == "evm") && routes == "routes"
    )
}

fn is_reviewed_secret_slot(path: &[String]) -> bool {
    matches!(
        path,
        [family, routes, _, field]
            if routes == "routes"
                && ((family == "bitcoin" && field == "rpc_password")
                    || (family == "evm" && field == "auth_header"))
    )
}

fn is_indirect_secret_source_shape(value: &Value) -> bool {
    let Some(object) = value.as_object() else {
        return false;
    };
    let Some((key, source)) = object.iter().next().filter(|_| object.len() == 1) else {
        return false;
    };
    matches!(normalize_field(key).as_str(), "env" | "file" | "file_env") && source.is_string()
}

fn normalize_field(field: &str) -> String {
    field
        .chars()
        .map(|character| match character {
            'A'..='Z' => character.to_ascii_lowercase(),
            '-' => '_',
            _ => character,
        })
        .collect()
}

struct UniqueJsonValue(Value);

impl<'de> Deserialize<'de> for UniqueJsonValue {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(UniqueJsonVisitor)
    }
}

struct UniqueJsonVisitor;

impl<'de> Visitor<'de> for UniqueJsonVisitor {
    type Value = UniqueJsonValue;

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("one JSON value without duplicate object keys")
    }

    fn visit_bool<E>(self, value: bool) -> std::result::Result<Self::Value, E> {
        Ok(UniqueJsonValue(Value::Bool(value)))
    }

    fn visit_i64<E>(self, value: i64) -> std::result::Result<Self::Value, E> {
        Ok(UniqueJsonValue(Value::Number(Number::from(value))))
    }

    fn visit_u64<E>(self, value: u64) -> std::result::Result<Self::Value, E> {
        Ok(UniqueJsonValue(Value::Number(Number::from(value))))
    }

    fn visit_f64<E>(self, value: f64) -> std::result::Result<Self::Value, E>
    where
        E: de::Error,
    {
        Number::from_f64(value)
            .map(Value::Number)
            .map(UniqueJsonValue)
            .ok_or_else(|| E::custom("invalid JSON number"))
    }

    fn visit_str<E>(self, value: &str) -> std::result::Result<Self::Value, E> {
        Ok(UniqueJsonValue(Value::String(value.to_owned())))
    }

    fn visit_string<E>(self, value: String) -> std::result::Result<Self::Value, E> {
        Ok(UniqueJsonValue(Value::String(value)))
    }

    fn visit_none<E>(self) -> std::result::Result<Self::Value, E> {
        Ok(UniqueJsonValue(Value::Null))
    }

    fn visit_unit<E>(self) -> std::result::Result<Self::Value, E> {
        Ok(UniqueJsonValue(Value::Null))
    }

    fn visit_some<D>(self, deserializer: D) -> std::result::Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        UniqueJsonValue::deserialize(deserializer)
    }

    fn visit_seq<A>(self, mut sequence: A) -> std::result::Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut values = Vec::new();
        while let Some(value) = sequence.next_element::<UniqueJsonValue>()? {
            values.push(value.0);
        }
        Ok(UniqueJsonValue(Value::Array(values)))
    }

    fn visit_map<A>(self, mut object: A) -> std::result::Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut keys = BTreeSet::new();
        let mut values = Map::new();
        while let Some(key) = object.next_key::<String>()? {
            if !keys.insert(key.clone()) {
                return Err(de::Error::custom("duplicate object key"));
            }
            let value = object.next_value::<UniqueJsonValue>()?;
            values.insert(key, value.0);
        }
        Ok(UniqueJsonValue(Value::Object(values)))
    }
}
