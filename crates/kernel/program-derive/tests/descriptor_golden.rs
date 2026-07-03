use mfm_program::{fact_descriptor_ref, facts, MfmFactType as _};
use mfm_program_derive::{
    MfmConfig, MfmFactType, MfmValue, OperationOutput, PublicOutputs, StateInput,
};
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.test",
    name = "chain_head_subject",
    version = "1",
    schema = "mfm.test.chain_head_subject"
)]
struct ChainHeadSubject {
    chain: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.test",
    name = "chain_head_response",
    version = "1",
    schema = "mfm.test.chain_head_response"
)]
struct ChainHeadResponse {
    height: u64,
    block_hash: String,
}

#[allow(clippy::duplicated_attributes)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue, MfmFactType)]
#[mfm(
    namespace = "mfm.test",
    name = "chain_head_fact",
    version = "1",
    schema = "mfm.test.chain_head_fact"
)]
#[mfm_fact(kind = "chain.head")]
#[mfm_fact(field(
    id = "subject.chain",
    source = "subject",
    path = "chain",
    value_type = "string",
    exposure = "returnable"
))]
#[mfm_fact(field(
    id = "result.height",
    source = "result",
    path = "height",
    value_type = "unsigned_integer",
    operators(equal, greater_than_or_equal),
    exposure = "returnable",
    sortable
))]
#[mfm_fact(field(
    id = "result.block_hash",
    source = "result",
    path = "block_hash",
    value_type = "string",
    exposure = "returnable"
))]
#[mfm_fact(field(
    id = "metadata.observed_at",
    source = "metadata",
    metadata = "observed_at",
    value_type = "timestamp",
    operators(equal, less_than_or_equal),
    exposure = "query_only",
    optional,
    sortable
))]
#[mfm_fact(ordering(
    name = "result.height.desc",
    term(field = "result.height", direction = "descending", nulls = "last")
))]
struct ChainHeadFact {
    subject: ChainHeadSubject,
    response: ChainHeadResponse,
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
        "schema:mfm.test.priced_asset:1:sha256-jcs-v1:2ace2fabd0b5853cf06870b4805ab6a8025fb212ad5a8ce9660b262ec1f673a3"
    );
}

#[test]
fn transparent_string_value_descriptor_uses_string_shape() {
    let descriptor = AccountId::schema_descriptor().expect("descriptor");

    assert_eq!(descriptor.identity.schema_kind, SchemaKind::Value);
    assert_eq!(
        descriptor.identity.schema_name.as_str(),
        "mfm.test.account_id"
    );
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
fn transparent_map_value_descriptor_uses_map_shape() {
    let descriptor = PublicMetadata::schema_descriptor().expect("descriptor");

    assert_eq!(descriptor.identity.schema_kind, SchemaKind::Value);
    assert_eq!(
        descriptor.identity.schema_name.as_str(),
        "mfm.test.public_metadata"
    );
    assert_eq!(
        descriptor.identity.shape,
        SchemaShape::BTreeMapString {
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

#[test]
fn generated_fact_descriptor_is_canonical_descriptor_authority() {
    let descriptor = ChainHeadFact::descriptor().expect("fact descriptor");
    let descriptor_schema = facts::fact_descriptor_schema_id().expect("descriptor schema");

    assert_eq!(descriptor.fact_kind().as_str(), "chain.head");
    assert_eq!(descriptor.descriptor_schema_id(), &descriptor_schema);
    assert_eq!(
        descriptor.subject_schema_id(),
        &ChainHeadSubject::schema_id().expect("subject schema")
    );
    assert_eq!(
        descriptor.response_schema_id(),
        &ChainHeadResponse::schema_id().expect("response schema")
    );

    let fields = descriptor.fields();
    assert_eq!(fields.len(), 4);
    assert_eq!(fields[0].field_id().as_str(), "subject.chain");
    assert_eq!(
        fields[0].extraction(),
        &facts::FactFieldExtraction::SubjectPath(
            facts::CanonicalValuePath::new("chain").expect("subject path")
        )
    );
    assert!(fields[0].required());
    assert!(!fields[0].sortable());

    assert_eq!(fields[1].field_id().as_str(), "result.height");
    assert_eq!(
        fields[1].value_type(),
        facts::FactFieldValueType::UnsignedInteger
    );
    assert_eq!(
        fields[1].operators(),
        &[
            facts::FactQueryOperator::Equal,
            facts::FactQueryOperator::GreaterThanOrEqual,
        ]
    );
    assert_eq!(fields[1].exposure(), facts::FactFieldExposure::Returnable);
    assert!(fields[1].sortable());

    assert_eq!(fields[3].field_id().as_str(), "metadata.observed_at");
    assert_eq!(
        fields[3].extraction(),
        &facts::FactFieldExtraction::Metadata(facts::FactMetadataField::ObservedAt)
    );
    assert_eq!(fields[3].exposure(), facts::FactFieldExposure::QueryOnly);
    assert!(!fields[3].required());
    assert!(fields[3].sortable());

    let ordering = descriptor.orderings().first().expect("ordering");
    assert_eq!(ordering.name().as_str(), "result.height.desc");
    assert_eq!(ordering.terms()[0].field_id().as_str(), "result.height");
    assert_eq!(
        ordering.terms()[0].direction(),
        facts::SortDirection::Descending
    );
    assert_eq!(ordering.terms()[0].nulls(), facts::NullOrdering::Last);

    let canonical = facts::canonical_fact_descriptor_bytes(&descriptor).expect("canonical bytes");
    let canonical_json = std::str::from_utf8(canonical.as_bytes()).expect("utf8 canonical json");
    assert!(canonical_json.contains("\"fact_kind\":\"chain.head\""));
    assert!(canonical_json.contains("\"descriptor_schema_id\":\"schema:mfm.fact_descriptor"));
    assert!(!canonical_json.contains("descriptor_hash"));

    let descriptor_hash = facts::fact_descriptor_hash(&descriptor).expect("descriptor hash");
    let descriptor_ref = fact_descriptor_ref::<ChainHeadFact>().expect("descriptor ref");
    assert_eq!(descriptor_ref.descriptor_hash, descriptor_hash);
}
