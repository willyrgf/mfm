use std::collections::BTreeMap;

use mfm_ids::{ContentDigest, SchemaId, StableAuthorKey};
use mfm_portfolio_model::portfolio::PortfolioConfig;
use mfm_storage_postgres::{
    ConfiguredValuePublication, ConfiguredValuePublicationStatus, ConfiguredValueRow,
    PostgresStore, MAX_CONFIGURED_VALUE_BYTES,
};
use mfm_values::{MfmConfig, ValidatedConfig};
use serde::Deserialize;
use serde_json::Value;

use crate::{AppError, ErrorClass};

const MAX_SETUP_FILE_BYTES: usize = 4 * 1024 * 1024;

/// The result of publishing one setup configuration without exposing its canonical payload.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct SetupConfigPublication {
    /// Stable target derived from the typed configuration's intrinsic domain id.
    pub target: String,
    /// Typed config schema identity.
    #[serde(serialize_with = "serialize_schema_id")]
    pub schema_id: SchemaId,
    /// Exact content digest.
    #[serde(serialize_with = "serialize_content_digest")]
    pub digest: ContentDigest,
    /// Whether import created, updated, or left the current value unchanged.
    pub status: SetupConfigPublicationStatus,
}

/// Observable result of publishing a current setup configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SetupConfigPublicationStatus {
    /// The target had no current configuration before import.
    Created,
    /// The target's current configuration changed.
    Updated,
    /// The target already held identical canonical configuration.
    Unchanged,
}

/// Strict setup document accepted by the application publisher.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SetupDocument {
    configs: Vec<SetupConfig>,
}

/// Closed set of concrete configuration types accepted by setup import.
#[derive(Debug, Deserialize)]
#[serde(tag = "kind", content = "value", deny_unknown_fields)]
enum SetupConfig {
    #[serde(rename = "portfolio")]
    Portfolio(PortfolioConfig),
}

/// Imports and publishes a complete TOML setup document atomically.
pub async fn import_setup_toml(
    store: &PostgresStore,
    bytes: &[u8],
) -> Result<Vec<SetupConfigPublication>, AppError> {
    let document = parse_setup_document(bytes)?;
    let mut prepared = BTreeMap::<StableAuthorKey, ConfiguredValueRow>::new();
    for config in document.configs {
        let prepared_value = prepare_setup_config(config)?;
        if prepared
            .insert(prepared_value.target.clone(), prepared_value)
            .is_some()
        {
            return Err(AppError::new(
                ErrorClass::Conflict,
                "SetupDuplicateTarget",
                "The setup document contains a duplicate configuration target",
            ));
        }
    }
    let publications = store
        .publish_configured_values(&prepared.into_values().collect::<Vec<_>>())
        .await
        .map_err(AppError::from)?;
    Ok(publications.into_iter().map(Into::into).collect())
}

fn parse_setup_document(bytes: &[u8]) -> Result<SetupDocument, AppError> {
    if bytes.len() > MAX_SETUP_FILE_BYTES {
        return Err(AppError::new(
            ErrorClass::BadRequest,
            "SetupFileTooLarge",
            "The setup file exceeds the permitted size",
        ));
    }
    let text = std::str::from_utf8(bytes).map_err(|_| {
        AppError::new(
            ErrorClass::BadRequest,
            "SetupDocumentInvalid",
            "The setup document is not valid UTF-8",
        )
    })?;
    toml::from_str(text).map_err(|_| {
        AppError::new(
            ErrorClass::BadRequest,
            "SetupDocumentInvalid",
            "The setup document is invalid TOML",
        )
    })
}

/// Lists current configured targets in stable target order.
pub async fn list_setup_targets(store: &PostgresStore) -> Result<Vec<String>, AppError> {
    store
        .list_configured_targets()
        .await
        .map_err(AppError::from)
        .map(|targets| {
            targets
                .into_iter()
                .map(|target| target.into_string())
                .collect()
        })
}

/// Exports the current configuration for one target after storage integrity verification.
pub async fn export_setup_target(store: &PostgresStore, target: &str) -> Result<Vec<u8>, AppError> {
    let target = StableAuthorKey::new(target).map_err(|_| {
        AppError::new(
            ErrorClass::BadRequest,
            "ConfiguredTargetInvalid",
            "The configuration target is invalid",
        )
    })?;
    store
        .load_configured_value(&target)
        .await
        .map_err(AppError::from)?
        .map(|row| row.canonical_json)
        .ok_or_else(|| {
            AppError::not_found(
                "ConfiguredValueNotFound",
                "The current configuration target was not found",
            )
        })
}

fn prepare_setup_config(value: SetupConfig) -> Result<ConfiguredValueRow, AppError> {
    let target = target_for_setup_config(&value)?;
    match value {
        SetupConfig::Portfolio(config) => prepare_config(target, config.normalized()),
    }
}

fn target_for_setup_config(value: &SetupConfig) -> Result<StableAuthorKey, AppError> {
    let target = match value {
        SetupConfig::Portfolio(config) => config.portfolio_id.as_str(),
    };
    StableAuthorKey::new(target).map_err(|_| {
        AppError::backend(
            ErrorClass::Internal,
            "SetupTargetInvalid",
            "A typed setup configuration has an invalid target",
        )
    })
}

fn prepare_config<T: MfmConfig>(
    target: StableAuthorKey,
    config: T,
) -> Result<ConfiguredValueRow, AppError> {
    let validated = ValidatedConfig::new(config).map_err(|_| {
        AppError::new(
            ErrorClass::BadRequest,
            "SetupValueValidationFailed",
            "A setup value failed semantic validation",
        )
    })?;
    let canonical = validated.canonical_json().map_err(|_| {
        AppError::new(
            ErrorClass::BadRequest,
            "SetupValueCanonicalizationFailed",
            "A setup value could not be canonicalized",
        )
    })?;
    if canonical.as_bytes().len() > MAX_CONFIGURED_VALUE_BYTES {
        return Err(AppError::new(
            ErrorClass::BadRequest,
            "SetupValueTooLarge",
            "A setup value exceeds the permitted size",
        ));
    }
    let value: Value = serde_json::from_slice(canonical.as_bytes()).map_err(|_| {
        AppError::backend(
            ErrorClass::Internal,
            "SetupValueCanonicalJsonInvalid",
            "A canonical setup value could not be inspected",
        )
    })?;
    if let Some(path) = secret_field_path(&value, "$", None) {
        return Err(AppError::new(
            ErrorClass::BadRequest,
            "SetupSecretFieldRejected",
            format!("Setup value contains a prohibited field at {path}"),
        ));
    }
    let schema_id = T::schema_id().map_err(|_| {
        AppError::backend(
            ErrorClass::Internal,
            "SetupValueSchemaInvalid",
            "A setup value schema is invalid",
        )
    })?;
    ConfiguredValueRow::new(
        target,
        schema_id,
        canonical.content_digest(),
        canonical.as_bytes().to_vec(),
    )
    .map_err(|_| {
        AppError::backend(
            ErrorClass::Internal,
            "SetupValueStorageInvalid",
            "A prepared setup value is invalid for configuration storage",
        )
    })
}

fn secret_field_path(value: &Value, path: &str, field: Option<&str>) -> Option<String> {
    if let Some(field) = field {
        let normalized = field.to_ascii_lowercase().replace(['-', ' ', '.'], "_");
        let prohibited = [
            "password",
            "passphrase",
            "mnemonic",
            "private",
            "secret",
            "credential",
            "unlock",
            "api_key",
            "access_token",
            "refresh_token",
            "authorization",
            "bearer",
        ];
        if prohibited.iter().any(|marker| normalized.contains(marker)) {
            return Some(path.to_owned());
        }
    }
    match value {
        Value::Object(map) => map.iter().find_map(|(key, child)| {
            let child_path = format!("{path}.{key}");
            secret_field_path(child, &child_path, Some(key))
        }),
        Value::Array(values) => values
            .iter()
            .enumerate()
            .find_map(|(index, child)| secret_field_path(child, &format!("{path}[{index}]"), None)),
        Value::String(text) => uri_contains_userinfo(text).then(|| path.to_owned()),
        _ => None,
    }
}

fn uri_contains_userinfo(value: &str) -> bool {
    let Ok(url) = url::Url::parse(value) else {
        return false;
    };
    !url.username().is_empty() || url.password().is_some()
}

impl From<ConfiguredValuePublication> for SetupConfigPublication {
    fn from(value: ConfiguredValuePublication) -> Self {
        Self {
            target: value.row.target.into_string(),
            schema_id: value.row.schema_id,
            digest: value.row.digest,
            status: match value.status {
                ConfiguredValuePublicationStatus::Created => SetupConfigPublicationStatus::Created,
                ConfiguredValuePublicationStatus::Updated => SetupConfigPublicationStatus::Updated,
                ConfiguredValuePublicationStatus::Unchanged => {
                    SetupConfigPublicationStatus::Unchanged
                }
            },
        }
    }
}

fn serialize_schema_id<S>(value: &SchemaId, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serializer.serialize_str(value.as_str())
}

fn serialize_content_digest<S>(value: &ContentDigest, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serializer.serialize_str(value.as_str())
}

#[cfg(test)]
mod tests {
    use super::secret_field_path;
    use super::{parse_setup_document, prepare_setup_config, SetupDocument, MAX_SETUP_FILE_BYTES};
    use serde_json::json;

    #[test]
    fn setup_fixtures_decode_and_prepare_the_portfolio_value() {
        for fixture in [
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../examples/setup/organization.toml"
            )),
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../examples/setup/portfolio-erc20.toml"
            )),
        ] {
            let document: SetupDocument = toml::from_str(fixture).expect("setup fixture");
            assert_eq!(document.configs.len(), 1);
            for config in document.configs {
                let prepared = prepare_setup_config(config).expect("portfolio value prepares");
                assert!(!prepared.canonical_json.is_empty());
            }
        }
    }

    #[test]
    fn setup_rejects_every_removed_collector_and_contract_kind() {
        for kind in [
            "btc_address_balance",
            "evm_native_balance",
            "evm_contract_context",
            "evm_deploy_action",
            "evm_configure_action",
            "evm_validate_action",
            "evm_import_deployed",
            "evm_import_configured",
        ] {
            let document = format!("[[configs]]\nkind = \"{kind}\"\n\n[configs.value]\n");
            let error = parse_setup_document(document.as_bytes())
                .expect_err("removed setup kind must not decode");
            assert_eq!(error.code, "SetupDocumentInvalid", "{kind}");
        }
    }

    #[test]
    fn setup_rejects_removed_catalog_name_and_values_envelopes() {
        let fixture = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../examples/setup/organization.toml"
        ));
        let named = fixture.replacen("[[configs]]", "[[configs]]\nname = \"acme/legacy-name\"", 1);
        let error = parse_setup_document(named.as_bytes())
            .expect_err("independent catalog names must be rejected");
        assert_eq!(error.code, "SetupDocumentInvalid");

        let values = fixture.replace("configs", "values");
        let error = parse_setup_document(values.as_bytes())
            .expect_err("the removed values envelope must be rejected");
        assert_eq!(error.code, "SetupDocumentInvalid");
    }

    #[test]
    fn setup_document_rejects_unknown_fields_at_the_envelope() {
        let mut document: toml::Value = toml::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../examples/setup/organization.toml"
        )))
        .expect("setup fixture as toml value");
        document
            .get_mut("configs")
            .and_then(toml::Value::as_array_mut)
            .and_then(|configs| configs.first_mut())
            .and_then(toml::Value::as_table_mut)
            .expect("setup entry")
            .insert("unexpected".to_owned(), toml::Value::Boolean(true));
        let document = toml::to_string(&document).expect("setup document serializes");
        toml::from_str::<SetupDocument>(&document).expect_err("unknown setup field");
    }

    #[test]
    fn every_setup_family_with_nested_values_rejects_unknown_nested_fields() {
        fn add_unknown_field(value: &mut toml::Value) -> bool {
            match value {
                toml::Value::Table(table) => {
                    for (key, child) in table.iter_mut() {
                        if key == "metadata" {
                            continue;
                        }
                        if let toml::Value::Table(child_table) = child {
                            child_table.insert(
                                "unexpected_nested_setup_field".to_owned(),
                                toml::Value::Boolean(true),
                            );
                            return true;
                        }
                        if add_unknown_field(child) {
                            return true;
                        }
                    }
                    false
                }
                toml::Value::Array(values) => values.iter_mut().any(add_unknown_field),
                _ => false,
            }
        }

        let base: toml::Value = toml::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../examples/setup/organization.toml"
        )))
        .expect("setup fixture as toml value");
        let configs = base
            .get("configs")
            .and_then(toml::Value::as_array)
            .expect("setup configs array");
        assert_eq!(configs.len(), 1);

        for index in 0..configs.len() {
            let mut document = base.clone();
            let value = document
                .get_mut("configs")
                .and_then(toml::Value::as_array_mut)
                .and_then(|configs| configs.get_mut(index))
                .and_then(toml::Value::as_table_mut)
                .and_then(|entry| entry.get_mut("value"))
                .expect("setup value");
            if !add_unknown_field(value) {
                continue;
            }
            let bytes = toml::to_string(&document).expect("setup document serializes");
            let error = toml::from_str::<SetupDocument>(&bytes)
                .expect_err("unknown nested field must be rejected");
            assert!(error.to_string().contains("unknown field"), "{error}");
        }
    }

    #[test]
    fn every_registered_setup_kind_rejects_unknown_value_fields() {
        let base: toml::Value = toml::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../examples/setup/organization.toml"
        )))
        .expect("setup fixture as toml value");
        let configs = base
            .get("configs")
            .and_then(toml::Value::as_array)
            .expect("setup configs array");
        assert_eq!(configs.len(), 1);

        for index in 0..configs.len() {
            let mut document = base.clone();
            let value = document
                .get_mut("configs")
                .and_then(toml::Value::as_array_mut)
                .and_then(|configs| configs.get_mut(index))
                .and_then(toml::Value::as_table_mut)
                .and_then(|entry| entry.get_mut("value"))
                .and_then(toml::Value::as_table_mut)
                .expect("setup value table");
            value.insert(
                "unexpected_setup_field".to_owned(),
                toml::Value::Boolean(true),
            );
            let bytes = toml::to_string(&document).expect("setup document serializes");
            let error = toml::from_str::<SetupDocument>(&bytes)
                .expect_err("unknown field must be rejected for every setup kind");
            assert!(error.to_string().contains("unknown field"), "{error}");
        }
    }

    #[test]
    fn setup_parser_rejects_oversized_files_invalid_utf8_and_malformed_toml() {
        let oversized = parse_setup_document(&vec![b'x'; MAX_SETUP_FILE_BYTES + 1])
            .expect_err("oversized setup file");
        assert_eq!(oversized.code, "SetupFileTooLarge");

        let invalid_utf8 =
            parse_setup_document(&[0xff, 0xfe]).expect_err("invalid UTF-8 setup file");
        assert_eq!(invalid_utf8.code, "SetupDocumentInvalid");

        let malformed =
            parse_setup_document(b"[[configs]\nkind = \"").expect_err("malformed setup TOML");
        assert_eq!(malformed.code, "SetupDocumentInvalid");
    }

    #[test]
    fn setup_rejects_an_oversized_individual_value_before_storage() {
        let mut document: toml::Value = toml::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../examples/setup/organization.toml"
        )))
        .expect("setup fixture as toml value");
        let metadata = document
            .get_mut("configs")
            .and_then(toml::Value::as_array_mut)
            .and_then(|configs| configs.first_mut())
            .and_then(toml::Value::as_table_mut)
            .and_then(|entry| entry.get_mut("value"))
            .and_then(toml::Value::as_table_mut)
            .and_then(|value| value.get_mut("metadata"))
            .and_then(toml::Value::as_table_mut)
            .expect("portfolio metadata table");
        metadata.insert(
            "large_public_note".to_owned(),
            toml::Value::String("x".repeat(256 * 1024)),
        );
        let bytes = toml::to_string(&document).expect("oversized setup serializes");
        let document = parse_setup_document(bytes.as_bytes()).expect("oversized value parses");
        let error = prepare_setup_config(
            document
                .configs
                .into_iter()
                .next()
                .expect("portfolio config"),
        )
        .expect_err("oversized value must be rejected before storage");
        assert_eq!(error.code, "SetupValueTooLarge");
        assert!(!error.message.contains("large_public_note"));
    }

    #[test]
    fn normalized_portfolio_revisions_have_one_stable_digest() {
        let base: toml::Value = toml::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../examples/setup/organization.toml"
        )))
        .expect("setup fixture as toml value");
        let mut reordered = base.clone();
        let value = reordered
            .get_mut("configs")
            .and_then(toml::Value::as_array_mut)
            .and_then(|configs| configs.first_mut())
            .and_then(toml::Value::as_table_mut)
            .and_then(|entry| entry.get_mut("value"))
            .and_then(toml::Value::as_table_mut)
            .expect("portfolio value table");
        for key in ["quote_codes", "networks", "wallets", "symbol_configs"] {
            value
                .get_mut(key)
                .and_then(toml::Value::as_array_mut)
                .expect("portfolio array")
                .reverse();
        }

        let first = parse_setup_document(
            toml::to_string(&base)
                .expect("base setup serializes")
                .as_bytes(),
        )
        .expect("base setup parses");
        let second = parse_setup_document(
            toml::to_string(&reordered)
                .expect("reordered setup serializes")
                .as_bytes(),
        )
        .expect("reordered setup parses");
        let first = prepare_setup_config(first.configs.into_iter().next().expect("base portfolio"))
            .expect("base portfolio prepares");
        let second = prepare_setup_config(
            second
                .configs
                .into_iter()
                .next()
                .expect("reordered portfolio"),
        )
        .expect("reordered portfolio prepares");
        assert_eq!(first.digest, second.digest);
        assert_eq!(first.canonical_json, second.canonical_json);
    }

    #[test]
    fn secret_scan_reports_only_the_field_path() {
        let value = json!({"nested": [{"private_key": "never-report-this"}]});
        assert_eq!(
            secret_field_path(&value, "$", None).as_deref(),
            Some("$.nested[0].private_key")
        );
    }

    #[test]
    fn uri_scan_rejects_userinfo() {
        let value = json!({"endpoint": "https://user:password@example.test"});
        assert_eq!(
            secret_field_path(&value, "$", None).as_deref(),
            Some("$.endpoint")
        );
    }

    #[test]
    fn secret_scan_rejects_all_secret_marker_families_without_values() {
        for field in [
            "password",
            "mnemonic",
            "private_key",
            "unlock_path",
            "api-key",
            "bearerToken",
            "unknown_credential_field",
        ] {
            let value = json!({field: "secret-payload-sentinel"});
            let path = secret_field_path(&value, "$", None).expect("secret marker");
            assert_eq!(path, format!("$.{field}"));
            assert!(!path.contains("secret-payload-sentinel"));
        }
    }
}
