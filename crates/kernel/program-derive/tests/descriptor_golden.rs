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
#[mfm(
    namespace = "mfm.test",
    name = "unit_field",
    version = "1",
    schema = "mfm.test.unit_field"
)]
struct UnitField {
    value: (),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(tag = "kind", rename_all = "camelCase")]
#[mfm(
    namespace = "mfm.test",
    name = "variant_field_rename",
    version = "1",
    schema = "mfm.test.variant_field_rename"
)]
enum VariantFieldRename {
    MultiWord {
        some_field: bool,
        #[serde(rename = "explicit-field")]
        explicitly_renamed: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.test",
    name = "default_variant_name",
    version = "1",
    schema = "mfm.test.default_variant_name"
)]
enum DefaultVariantName {
    MixedCase { some_field: bool },
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

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(transparent)]
#[mfm(
    namespace = "mfm.test",
    name = "public_metadata",
    version = "1",
    schema = "mfm.test.public_metadata",
    transparent_map
)]
struct PublicMetadata {
    entries: BTreeMap<String, String>,
}

impl mfm_values::MfmDefault for PublicMetadata {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmConfig)]
struct InlineDefaultConfig {
    #[serde(default)]
    metadata: PublicMetadata,
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
    assert_eq!(
        descriptor.identity.schema_name.as_str(),
        "mfm.test.priced_asset"
    );
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
        "schema:mfm.test.priced_asset:1:sha256-jcs-v1:3fc09529960c3ab2508ef19b3460883b2a3d230df374de5c7496318740c36bed"
    );
}

#[test]
fn empty_rust_tuple_uses_the_exact_null_unit_shape() {
    let descriptor = UnitField::schema_descriptor().expect("descriptor");
    let SchemaShape::Struct { fields } = &canonical_shape(&descriptor) else {
        panic!("expected struct shape");
    };
    assert_eq!(fields[0].shape, SchemaShape::Unit);
    let value = serde_json::to_vec(&UnitField { value: () }).expect("unit value");
    descriptor
        .identity
        .validate_canonical_value(&value)
        .expect("unit value matches descriptor");
    assert_eq!(value, br#"{"value":null}"#);
}

#[test]
fn enum_rename_all_changes_variants_but_not_named_variant_fields() {
    let descriptor = VariantFieldRename::schema_descriptor().expect("descriptor");
    let value = VariantFieldRename::MultiWord {
        some_field: true,
        explicitly_renamed: false,
    };
    let canonical = serde_json::to_vec(&value).expect("enum value");
    assert_eq!(
        canonical,
        br#"{"kind":"multiWord","some_field":true,"explicit-field":false}"#
    );
    let canonical = mfm_canonical::PlainCanonicalJsonBytes::from_json_str(
        std::str::from_utf8(&canonical).expect("UTF-8"),
    )
    .expect("canonicalized enum value");
    descriptor
        .identity
        .validate_canonical_value(canonical.as_bytes())
        .expect("enum value matches descriptor");
}

#[test]
fn absent_enum_rename_all_preserves_the_rust_variant_wire_name() {
    let descriptor = DefaultVariantName::schema_descriptor().expect("descriptor");
    let value = DefaultVariantName::MixedCase { some_field: true };
    let canonical = serde_json::to_vec(&value).expect("enum value");
    assert_eq!(canonical, br#"{"MixedCase":{"some_field":true}}"#);
    descriptor
        .identity
        .validate_canonical_value(&canonical)
        .expect("enum value matches descriptor");
}

#[test]
fn transparent_string_value_descriptor_uses_string_shape() {
    let descriptor = AccountId::schema_descriptor().expect("descriptor");

    assert_eq!(descriptor.identity.schema_kind, SchemaKind::Value);
    assert_eq!(
        descriptor.identity.schema_name.as_str(),
        "mfm.test.account_id"
    );
    assert_eq!(canonical_shape(&descriptor), &SchemaShape::String);
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
fn transparent_map_value_descriptor_uses_map_shape() {
    let descriptor = PublicMetadata::schema_descriptor().expect("descriptor");

    assert_eq!(descriptor.identity.schema_kind, SchemaKind::Value);
    assert_eq!(
        descriptor.identity.schema_name.as_str(),
        "mfm.test.public_metadata"
    );
    assert_eq!(
        canonical_shape(&descriptor),
        &SchemaShape::BTreeMapString {
            value: Box::new(SchemaShape::String)
        }
    );
    assert_eq!(
        serde_json::to_value(PublicMetadata {
            entries: BTreeMap::from([("source".to_owned(), "fixture".to_owned())])
        })
        .expect("json"),
        serde_json::json!({"source": "fixture"})
    );
}

#[test]
fn inline_custom_default_is_an_exact_omission_contract() {
    let descriptor = InlineDefaultConfig::schema_descriptor().expect("descriptor");
    let SchemaShape::Struct { fields } = &canonical_shape(&descriptor) else {
        panic!("expected struct shape");
    };
    assert_eq!(fields.len(), 1);
    assert_eq!(fields[0].default, FieldDefaultPolicy::MfmDefault);
    assert!(matches!(fields[0].shape, SchemaShape::InlineValue { .. }));

    descriptor
        .identity
        .validate_canonical_value(br#"{}"#)
        .expect("omitted custom default");
    let decoded: InlineDefaultConfig =
        serde_json::from_slice(br#"{}"#).expect("typed default decode");
    assert_eq!(decoded.metadata, PublicMetadata::default());

    descriptor
        .identity
        .validate_canonical_value(br#"{"metadata":{"source":"fixture"}}"#)
        .expect("present inline value");
    assert!(
        descriptor
            .identity
            .validate_canonical_value(br#"{"metadata":false}"#)
            .is_err(),
        "a present custom default must still match its exact inline shape"
    );
}

#[test]
fn generated_config_descriptor_resolves_wire_names_and_inline_value_shapes() {
    let descriptor = PortfolioRequest::schema_descriptor().expect("descriptor");
    assert_eq!(descriptor.identity.schema_kind, SchemaKind::PlanningConfig);

    let SchemaShape::Struct { fields } = &canonical_shape(&descriptor) else {
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

    let assets = fields
        .iter()
        .find(|field| field.name == "assets")
        .expect("assets field");
    let SchemaShape::Vec(element) = &assets.shape else {
        panic!("expected vector shape");
    };
    let SchemaShape::InlineValue {
        schema_id,
        semantic_type_id,
        serialized_shape,
    } = element.as_ref()
    else {
        panic!("expected inline value shape");
    };
    let priced_asset = PricedAsset::schema_descriptor().expect("priced asset descriptor");
    assert_eq!(
        schema_id,
        &priced_asset.schema_id().expect("priced asset schema id")
    );
    assert_eq!(
        semantic_type_id,
        priced_asset
            .identity
            .semantic_type_id
            .as_ref()
            .expect("priced asset semantic id")
    );
    assert_eq!(serialized_shape.as_ref(), canonical_shape(&priced_asset));
}

#[test]
fn generated_config_validation_uses_configured_function_for_config_and_value_derives() {
    let valid = CheckedRequest {
        account_id: "acct".to_owned(),
    };
    assert!(valid.validate().is_ok());

    let invalid = CheckedRequest {
        account_id: String::new(),
    };
    let error = invalid.validate().expect_err("empty account id must fail");
    assert_eq!(error.message(), "account_id must be non-empty");

    let descriptor = <DualValidatedConfig as mfm_values::MfmValue>::schema_descriptor()
        .expect("value descriptor");
    assert_eq!(descriptor.identity.schema_kind, SchemaKind::Value);

    let valid = DualValidatedConfig {
        account_id: "acct".to_owned(),
    };
    assert!(valid.validate().is_ok());

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

fn canonical_shape(descriptor: &mfm_values::SchemaDescriptor) -> &SchemaShape {
    canonical_shape_of(&descriptor.identity)
}

fn canonical_shape_of(identity: &mfm_values::SchemaIdentity) -> &SchemaShape {
    identity
        .canonical_json_shape()
        .expect("canonical-JSON descriptor shape")
}
