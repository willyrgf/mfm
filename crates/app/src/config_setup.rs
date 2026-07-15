use std::collections::BTreeMap;

use mfm_catalog_model::CatalogName;
use mfm_evm_contract_model::EvmContractContext;
use mfm_ids::{ContentDigest, SchemaId};
use mfm_op_btc_collectors::BtcAddressBalanceConfig;
use mfm_op_portfolio_tracker::PortfolioConfig;
use mfm_state_evm_contracts::{
    ConfigureAction, DeployAction, ImportConfiguredSpec, ImportDeployedSpec, ValidateAction,
};
use mfm_storage_postgres::{
    CatalogValueKey, CatalogValueRow, PostgresStore, MAX_CATALOG_VALUE_BYTES,
};
use mfm_values::{MfmConfig, ValidatedConfig};
use serde::Deserialize;
use serde_json::Value;

use crate::{AppError, ErrorClass};

const MAX_SETUP_FILE_BYTES: usize = 4 * 1024 * 1024;

/// Identity returned for a catalog value without exposing its canonical payload.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
pub struct CatalogValueIdentity {
    /// Catalog name.
    pub name: String,
    /// Typed config schema identity.
    #[serde(serialize_with = "serialize_schema_id")]
    pub schema_id: SchemaId,
    /// Exact content digest.
    #[serde(serialize_with = "serialize_content_digest")]
    pub digest: ContentDigest,
}

/// Keyset cursor for bounded catalog listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogListCursor {
    /// Last catalog name returned by the previous page.
    pub name: String,
    /// Last schema id returned by the previous page.
    pub schema_id: SchemaId,
    /// Last digest returned by the previous page.
    pub digest: ContentDigest,
}

/// Strict setup document accepted by the application publisher.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SetupDocument {
    values: Vec<SetupEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SetupEntry {
    name: CatalogName,
    #[serde(flatten)]
    value: SetupValue,
}

/// Closed set of catalog resource types accepted by setup import.
#[derive(Debug, Deserialize)]
#[serde(tag = "kind", content = "value", deny_unknown_fields)]
enum SetupValue {
    #[serde(rename = "portfolio")]
    Portfolio(PortfolioConfig),
    #[serde(rename = "btc_address_balance")]
    BtcAddressBalance(BtcAddressBalanceConfig),
    #[serde(rename = "evm_contract_context")]
    EvmContractContext(EvmContractContext),
    #[serde(rename = "evm_deploy_action")]
    EvmDeployAction(DeployAction),
    #[serde(rename = "evm_configure_action")]
    EvmConfigureAction(ConfigureAction),
    #[serde(rename = "evm_validate_action")]
    EvmValidateAction(ValidateAction),
    #[serde(rename = "evm_import_deployed")]
    EvmImportDeployed(ImportDeployedSpec),
    #[serde(rename = "evm_import_configured")]
    EvmImportConfigured(ImportConfiguredSpec),
}

/// Imports and publishes a complete TOML setup document atomically.
pub async fn import_setup_toml(
    store: &PostgresStore,
    bytes: &[u8],
) -> Result<Vec<CatalogValueIdentity>, AppError> {
    let document = parse_setup_document(bytes)?;
    let mut prepared = BTreeMap::<CatalogValueKey, Vec<u8>>::new();
    for entry in document.values {
        let prepared_value = prepare_value(entry.value)?;
        let key = CatalogValueKey::new(
            entry.name.as_str(),
            prepared_value.schema_id,
            prepared_value.digest,
        )
        .map_err(|_| {
            AppError::new(
                ErrorClass::BadRequest,
                "CatalogValueIdentityInvalid",
                "The setup value identity is invalid",
            )
        })?;
        if let Some(previous) = prepared.get(&key) {
            if previous != &prepared_value.canonical_json {
                return Err(AppError::new(
                    ErrorClass::Conflict,
                    "SetupDuplicateConflict",
                    "The setup document contains conflicting duplicate catalog values",
                ));
            }
        } else {
            prepared.insert(key.clone(), prepared_value.canonical_json);
        }
    }
    let rows = prepared
        .into_iter()
        .map(|(key, canonical_json)| CatalogValueRow {
            key,
            canonical_json,
        })
        .collect::<Vec<_>>();
    let identities = rows
        .iter()
        .map(|row| identity_from_key(&row.key))
        .collect::<Vec<_>>();
    store
        .append_catalog_values(&rows)
        .await
        .map_err(AppError::from)?;
    Ok(identities)
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

/// Lists exact catalog identities using bounded keyset pagination.
pub async fn list_catalog_values(
    store: &PostgresStore,
    cursor: Option<&CatalogListCursor>,
    limit: u32,
) -> Result<Vec<CatalogValueIdentity>, AppError> {
    let after = cursor
        .map(|cursor| {
            CatalogValueKey::new(
                cursor.name.clone(),
                cursor.schema_id.clone(),
                cursor.digest.clone(),
            )
        })
        .transpose()
        .map_err(|_| {
            AppError::new(
                ErrorClass::BadRequest,
                "CatalogCursorInvalid",
                "The catalog list cursor is invalid",
            )
        })?;
    store
        .list_catalog_values(after.as_ref(), limit)
        .await
        .map_err(AppError::from)
        .map(|values| values.iter().map(identity_from_key).collect())
}

/// Loads one exact catalog value for export after storage integrity verification.
pub async fn export_catalog_value(
    store: &PostgresStore,
    identity: &CatalogValueIdentity,
) -> Result<Vec<u8>, AppError> {
    let key = CatalogValueKey::new(
        identity.name.clone(),
        identity.schema_id.clone(),
        identity.digest.clone(),
    )
    .map_err(|_| {
        AppError::new(
            ErrorClass::BadRequest,
            "CatalogValueIdentityInvalid",
            "The catalog value identity is invalid",
        )
    })?;
    store
        .export_catalog_value(&key)
        .await
        .map_err(AppError::from)?
        .map(|row| row.canonical_json)
        .ok_or_else(|| {
            AppError::not_found(
                "CatalogValueNotFound",
                "The exact catalog value was not found",
            )
        })
}

struct PreparedValue {
    schema_id: SchemaId,
    digest: ContentDigest,
    canonical_json: Vec<u8>,
}

fn prepare_value(value: SetupValue) -> Result<PreparedValue, AppError> {
    match value {
        SetupValue::Portfolio(config) => prepare_config(config.normalized()),
        SetupValue::BtcAddressBalance(config) => prepare_config(config),
        SetupValue::EvmContractContext(config) => prepare_config(config),
        SetupValue::EvmDeployAction(config) => prepare_config(config),
        SetupValue::EvmConfigureAction(config) => prepare_config(config),
        SetupValue::EvmValidateAction(config) => prepare_config(config),
        SetupValue::EvmImportDeployed(config) => prepare_config(config),
        SetupValue::EvmImportConfigured(config) => prepare_config(config),
    }
}

fn prepare_config<T: MfmConfig>(config: T) -> Result<PreparedValue, AppError> {
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
    if canonical.as_bytes().len() > MAX_CATALOG_VALUE_BYTES {
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
    Ok(PreparedValue {
        schema_id,
        digest: canonical.content_digest(),
        canonical_json: canonical.as_bytes().to_vec(),
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

fn identity_from_key(key: &CatalogValueKey) -> CatalogValueIdentity {
    CatalogValueIdentity {
        name: key.name.clone(),
        schema_id: key.schema_id.clone(),
        digest: key.digest.clone(),
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
    use super::{parse_setup_document, prepare_value, SetupDocument, MAX_SETUP_FILE_BYTES};
    use serde_json::json;

    #[test]
    fn complete_setup_fixture_decodes_and_prepares_every_v1_kind() {
        let document: SetupDocument = toml::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../examples/setup/organization.toml"
        )))
        .expect("complete setup fixture");
        assert_eq!(document.values.len(), 8);
        for entry in document.values {
            let prepared = prepare_value(entry.value).expect("fixture value prepares");
            assert!(!prepared.canonical_json.is_empty());
        }
    }

    #[test]
    fn setup_document_rejects_unknown_fields_at_the_envelope() {
        let error = toml::from_str::<SetupDocument>(
            r#"
                [[values]]
                name = "acme/value"
                kind = "evm_validate_action"
                unexpected = true

                [values.value]
            "#,
        )
        .expect_err("unknown setup field");
        assert!(error.to_string().contains("unknown field"));
    }

    #[test]
    fn registered_nested_setup_types_reject_unknown_fields() {
        for document in [
            r#"
                [[values]]
                name = "acme/portfolio"
                kind = "portfolio"

                [values.value]
                portfolio_id = "portfolio"
                quote_codes = ["USD"]
                networks = []
                symbol_configs = []
                metadata = {}

                [[values.value.wallets]]
                wallet_id = "wallet"
                network_id = "network"
                symbol_ids = []
                metadata = {}

                [values.value.wallets.subject]
                kind = "evm_address"
                address = "0x0000000000000000000000000000000000000001"

                [values.value.wallets.implementation]
                kind = "address_only"
                unexpected = true
            "#,
            r#"
                [[values]]
                name = "acme/context"
                kind = "evm_contract_context"

                [values.value]
                lifecycle_key = "lifecycle"

                [values.value.network]
                network_id = "ethereum-mainnet"
                expected_chain_id = 1
                unexpected = true

                [values.value.contract_profile]
                profile_id = "profile"
            "#,
            r#"
                [[values]]
                name = "acme/deploy"
                kind = "evm_deploy_action"

                [values.value.signer]
                signer_ref = "deployer"
                expected_signer_address = "0x0000000000000000000000000000000000000001"

                [values.value.transaction]
                style = "eip1559"
                unexpected = true
            "#,
            r#"
                [[values]]
                name = "acme/import"
                kind = "evm_import_deployed"

                [values.value]
                kind = "adopt_external_address"

                [values.value.adoption]
                address = "0x0000000000000000000000000000000000000001"
                provenance_label = "audited"
                unexpected = true
            "#,
        ] {
            let error = toml::from_str::<SetupDocument>(document)
                .expect_err("nested setup fields must be rejected");
            assert!(error.to_string().contains("unknown field"), "{error}");
        }
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
        let values = base
            .get("values")
            .and_then(toml::Value::as_array)
            .expect("setup values array");
        assert_eq!(values.len(), 8);

        for index in 0..values.len() {
            let mut document = base.clone();
            let value = document
                .get_mut("values")
                .and_then(toml::Value::as_array_mut)
                .and_then(|values| values.get_mut(index))
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
        let values = base
            .get("values")
            .and_then(toml::Value::as_array)
            .expect("setup values array");
        assert_eq!(values.len(), 8);

        for index in 0..values.len() {
            let mut document = base.clone();
            let value = document
                .get_mut("values")
                .and_then(toml::Value::as_array_mut)
                .and_then(|values| values.get_mut(index))
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
    fn setup_rejects_float_values_in_typed_configs() {
        let error = toml::from_str::<SetupDocument>(
            r#"
                [[values]]
                name = "acme/bitcoin"
                kind = "btc_address_balance"

                [values.value]
                network = "bitcoin-mainnet"
                bitcoin_network = "main"
                semantic_source_identity = "public-bitcoin-core"
                addresses = ["bc1qxy2kgdygjrsqtzq2n0yrf2493p83kkfjhx0wlh"]
                coverage = "configured_only"
                max_source_reads = 1.5
            "#,
        )
        .expect_err("float values must not enter hashed configuration");
        assert!(error.to_string().contains("invalid") || error.to_string().contains("float"));
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
            parse_setup_document(b"[[values]\nname = \"").expect_err("malformed setup TOML");
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
            .get_mut("values")
            .and_then(toml::Value::as_array_mut)
            .and_then(|values| values.first_mut())
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
        let error = prepare_value(
            document
                .values
                .into_iter()
                .next()
                .expect("portfolio entry")
                .value,
        )
        .err()
        .expect("oversized value must be rejected before storage");
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
            .get_mut("values")
            .and_then(toml::Value::as_array_mut)
            .and_then(|values| values.first_mut())
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
        let first = prepare_value(
            first
                .values
                .into_iter()
                .next()
                .expect("base portfolio")
                .value,
        )
        .expect("base portfolio prepares");
        let second = prepare_value(
            second
                .values
                .into_iter()
                .next()
                .expect("reordered portfolio")
                .value,
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
