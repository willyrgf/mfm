use mfm_program_derive::{MfmConfig, MfmValue, OperationOutput, PublicOutputs, StateInput};
use mfm_values::{
    DescriptorProvenance, FieldDefaultPolicy, MfmConfig as _, MfmValue as _, OperationOutput as _,
    PublicOutputDescriptor as _, SchemaKind, SchemaShape, StateInput as _,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::num::NonZeroU64;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.test",
    name = "priced_asset",
    version = "1",
    schema = "mfm.test.priced_asset"
)]
struct PricedAsset {
    asset: String,
    amount_minor: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(try_from = "String", into = "String")]
#[mfm(
    namespace = "mfm.test",
    name = "account_id",
    version = "1",
    schema = "mfm.test.account_id",
    transparent_string
)]
struct AccountId {
    raw: String,
}

impl TryFrom<String> for AccountId {
    type Error = &'static str;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value.is_empty() {
            Err("account id must be non-empty")
        } else {
            Ok(Self { raw: value })
        }
    }
}

impl From<AccountId> for String {
    fn from(value: AccountId) -> Self {
        value.raw
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmConfig)]
#[serde(rename_all = "camelCase")]
struct PortfolioRequest {
    account_id: String,
    revision: NonZeroU64,
    assets: Vec<PricedAsset>,
    weights: BTreeMap<String, u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmConfig)]
#[mfm(validate = "validate_checked_request")]
struct CheckedRequest {
    account_id: String,
}

fn validate_checked_request(request: &CheckedRequest) -> Result<(), String> {
    if request.account_id.is_empty() {
        Err("account_id must be non-empty".to_owned())
    } else {
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue, MfmConfig)]
#[mfm(
    namespace = "mfm.test",
    name = "dual_validated_config",
    schema = "mfm.test.dual_validated_config",
    validate = "validate_dual_validated_config"
)]
struct DualValidatedConfig {
    account_id: String,
}

fn validate_dual_validated_config(request: &DualValidatedConfig) -> Result<(), String> {
    if request.account_id.is_empty() {
        Err("account_id must be non-empty".to_owned())
    } else {
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, StateInput)]
struct SnapshotInput {
    request: PortfolioRequestValue,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.test",
    name = "portfolio_request",
    version = "1",
    schema = "mfm.test.portfolio_request"
)]
struct PortfolioRequestValue {
    account_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, OperationOutput)]
struct SnapshotOutput {
    report: PricedAsset,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, PublicOutputs)]
struct PublicReport {
    #[serde(rename = "report_id")]
    id: String,
}

#[test]
fn generated_value_descriptor_is_stable() {
    let descriptor = PricedAsset::schema_descriptor().expect("descriptor");

    assert_eq!(descriptor.identity.schema_kind, SchemaKind::Value);
    assert_eq!(descriptor.identity.schema_name, "mfm.test.priced_asset");
    assert_eq!(descriptor.identity.schema_version.as_str(), "1");
    assert_eq!(
        descriptor.audit.provenance(),
        DescriptorProvenance::DeriveGenerated
    );
    assert_eq!(
        descriptor.audit.derive_macro_version(),
        Some("mfm-program-derive/0.1.0")
    );
    assert_eq!(
        descriptor.schema_id().expect("schema id").as_str(),
        "schema:mfm.test.priced_asset:1:sha256-jcs-v1:498b3d755f53fb1fa27079fb7b993b97bd4d27198e3162b5aaacd70a612362c2"
    );
}

#[test]
fn transparent_string_value_descriptor_uses_string_shape() {
    let descriptor = AccountId::schema_descriptor().expect("descriptor");

    assert_eq!(descriptor.identity.schema_kind, SchemaKind::Value);
    assert_eq!(descriptor.identity.schema_name, "mfm.test.account_id");
    assert_eq!(descriptor.identity.shape, SchemaShape::String);
    assert_eq!(
        serde_json::to_value(AccountId {
            raw: "acct".to_owned()
        })
        .expect("json"),
        serde_json::json!("acct")
    );
    assert!(serde_json::from_value::<AccountId>(serde_json::json!("")).is_err());
}

#[test]
fn generated_config_descriptor_resolves_wire_names_and_value_refs() {
    let descriptor = PortfolioRequest::schema_descriptor().expect("descriptor");
    assert_eq!(descriptor.identity.schema_kind, SchemaKind::PlanningConfig);

    let SchemaShape::Struct { fields } = &descriptor.identity.shape else {
        panic!("expected struct shape");
    };
    let names = fields
        .iter()
        .map(|field| (field.name.as_str(), field.default))
        .collect::<Vec<_>>();

    assert_eq!(
        names,
        vec![
            ("accountId", FieldDefaultPolicy::Required),
            ("assets", FieldDefaultPolicy::Required),
            ("revision", FieldDefaultPolicy::Required),
            ("weights", FieldDefaultPolicy::Required),
        ]
    );
}

#[test]
fn generated_config_validation_delegates_to_configured_function() {
    let valid = CheckedRequest {
        account_id: "acct".to_owned(),
    };
    assert!(valid.validate().is_ok());

    let invalid = CheckedRequest {
        account_id: String::new(),
    };
    let error = invalid.validate().expect_err("empty account id must fail");
    assert_eq!(error.message(), "account_id must be non-empty");
}

#[test]
fn generated_config_validation_composes_with_value_derive() {
    let descriptor = <DualValidatedConfig as mfm_values::MfmValue>::schema_descriptor()
        .expect("value descriptor");
    assert_eq!(descriptor.identity.schema_kind, SchemaKind::Value);

    let invalid = DualValidatedConfig {
        account_id: String::new(),
    };
    let error = invalid.validate().expect_err("empty account id must fail");
    assert_eq!(error.message(), "account_id must be non-empty");
}

#[test]
fn generated_non_value_descriptors_use_distinct_schema_kinds() {
    assert_eq!(
        SnapshotInput::input_schema_descriptor()
            .expect("state input descriptor")
            .identity
            .schema_kind,
        SchemaKind::StateInput
    );
    assert_eq!(
        SnapshotOutput::output_schema_descriptor()
            .expect("operation output descriptor")
            .identity
            .schema_kind,
        SchemaKind::OperationOutput
    );
    assert_eq!(
        PublicReport::public_schema_descriptor()
            .expect("public descriptor")
            .identity
            .schema_kind,
        SchemaKind::PublicOutput
    );
}
