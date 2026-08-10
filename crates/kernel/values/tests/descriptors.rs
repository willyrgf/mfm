use std::collections::BTreeMap;

use mfm_ids::{
    ArtifactId, ContentDigest, DigestAlgorithm, DigestBytes, NameToken, SchemaId, SchemaVersion,
    SemanticTypeId,
};
use mfm_values::*;

fn schema_version(value: &str) -> Result<SchemaVersion> {
    SchemaVersion::new(value).map_err(|error| ValueError::Identity(error.to_string()))
}

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct ExampleValue {
    amount: String,
    label: String,
}

impl MfmValue for ExampleValue {
    fn semantic_id() -> Result<SemanticTypeId> {
        example_semantic_id()
    }

    fn schema_descriptor() -> Result<SchemaDescriptor> {
        SchemaDescriptor::new(
            SchemaIdentity::new(
                SchemaKind::Value,
                Some(Self::semantic_id()?),
                "mfm.test.example_value",
                schema_version("1")?,
                SchemaShape::named_struct(vec![
                    FieldDescriptor::required(
                        "amount",
                        SchemaShape::DecimalString {
                            scale: DecimalScale::Variable,
                        },
                    ),
                    FieldDescriptor::required("label", SchemaShape::String),
                ])?,
            )?,
            SchemaAudit::__derive_generated("mfm-test", "mfm_test::ExampleValue", "mfm-derive/1"),
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct ExampleConfig {
    enabled: bool,
}

impl MfmConfig for ExampleConfig {
    fn schema_descriptor() -> Result<SchemaDescriptor> {
        SchemaDescriptor::new(
            SchemaIdentity::new(
                SchemaKind::PlanningConfig,
                None,
                "mfm.test.example_config",
                schema_version("1")?,
                SchemaShape::named_struct(vec![FieldDescriptor::required(
                    "enabled",
                    SchemaShape::Bool,
                )])?,
            )?,
            SchemaAudit::__derive_generated("mfm-test", "mfm_test::ExampleConfig", "mfm-derive/1"),
        )
    }
}

struct ExamplePublicOutput;

impl PublicOutputDescriptor for ExamplePublicOutput {
    fn public_schema_descriptor() -> Result<SchemaDescriptor> {
        SchemaDescriptor::new(
            SchemaIdentity::new(
                SchemaKind::PublicOutput,
                None,
                "mfm.test.example_public_output",
                schema_version("1")?,
                SchemaShape::named_struct(vec![FieldDescriptor::required(
                    "artifact",
                    SchemaShape::inline_value::<ExampleValue>()?,
                )])?,
            )?,
            SchemaAudit::__derive_generated(
                "mfm-test",
                "mfm_test::ExamplePublicOutput",
                "mfm-derive/1",
            ),
        )
    }
}

fn example_semantic_id() -> Result<SemanticTypeId> {
    SemanticTypeId::new(
        "mfm.test",
        "example-value",
        "1",
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([0x33; 32]),
    )
    .map_err(|error| ValueError::Identity(error.to_string()))
}

#[test]
fn descriptor_identity_has_golden_canonical_json_and_schema_id() {
    let descriptor = ExampleValue::schema_descriptor().expect("example descriptor");

    assert_eq!(
        descriptor.identity.persisted_surface,
        PersistedSurfacePolicy {
            secrets: SecretPolicy::NoSecrets,
            numbers: NumberPolicy::NoFloats,
        }
    );
    assert_eq!(
        descriptor
            .identity_canonical_json()
            .expect("canonical identity")
            .as_str(),
        "{\"canonicalization\":\"sha256-jcs-v1\",\"encoding\":{\"kind\":\"canonical_json\",\"shape\":{\"fields\":[{\"default\":\"required\",\"name\":\"amount\",\"shape\":{\"kind\":\"decimal_string\",\"scale\":{\"kind\":\"variable\"}}},{\"default\":\"required\",\"name\":\"label\",\"shape\":{\"kind\":\"string\"}}],\"kind\":\"struct\"}},\"persisted_surface\":{\"numbers\":\"no_floats\",\"secrets\":\"no_secrets\"},\"schema_kind\":\"value\",\"schema_name\":\"mfm.test.example_value\",\"schema_version\":\"1\",\"semantic_type_id\":\"semantic:mfm.test:example-value:1:sha256-jcs-v1:3333333333333333333333333333333333333333333333333333333333333333\",\"versioning\":\"manual_version\"}"
    );
    assert_eq!(
        ExampleValue::schema_id()
            .expect("example schema id")
            .as_str(),
        "schema:mfm.test.example_value:1:sha256-jcs-v1:bd1f47f7703b8fbf6ef6de5d18a8c3bc043092ef3fd44087c3252ff246cb2924"
    );
}

#[test]
fn every_schema_identity_encoder_rejects_mutated_oversized_shape() {
    let mut descriptor = ExampleValue::schema_descriptor().expect("example descriptor");
    descriptor.identity.encoding = mfm_values::PersistedEncoding::CanonicalJson {
        shape: SchemaShape::named_struct(
            (0..1_500)
                .map(|index| {
                    FieldDescriptor::required(format!("field_{index:04}"), SchemaShape::Bool)
                })
                .collect(),
        )
        .expect("oversized shape"),
    };

    assert!(descriptor.identity_canonical_json().is_err());
    assert!(serde_json::to_vec(&descriptor.identity).is_err());
}

#[test]
fn schema_id_uses_identity_not_audit() {
    let descriptor = ExampleValue::schema_descriptor().expect("example descriptor");
    let mut changed_audit = descriptor.clone();
    changed_audit.audit =
        SchemaAudit::__derive_generated("other-crate", "other::ExampleValue", "other-derive/1");

    assert_eq!(
        descriptor.schema_id().expect("descriptor schema id"),
        changed_audit.schema_id().expect("changed audit schema id")
    );

    let mut changed_identity = descriptor.clone();
    changed_identity.identity.schema_name =
        NameToken::new("mfm.test.example_value_alternate").expect("schema name");
    assert_ne!(
        descriptor.schema_id().expect("descriptor schema id"),
        changed_identity
            .schema_id()
            .expect("changed identity schema id")
    );
}

#[test]
fn framework_generic_descriptors_have_golden_schema_ids() {
    assert_eq!(
        ArtifactRef::<ExampleValue>::schema_id()
            .expect("artifact ref schema id")
            .as_str(),
        "schema:mfm.kernel.artifact_ref:1:sha256-jcs-v1:c876826bd07c719d662b1487e87fbe7f8d2e5ca404ad095d10a2507dfa25ae1b"
    );
}

#[test]
fn non_empty_value_collection_rejects_empty_runtime_materialization() {
    let value = ExampleValue {
        amount: "1.00".to_owned(),
        label: "cash".to_owned(),
    };
    let non_empty = NonEmpty::new(value.clone(), Vec::new());
    assert_eq!(non_empty.values(), std::slice::from_ref(&value));

    let encoded = serde_json::to_string(&non_empty).expect("non-empty serializes");
    let decoded: NonEmpty<ExampleValue> =
        serde_json::from_str(&encoded).expect("non-empty deserializes");
    assert_eq!(decoded.values(), std::slice::from_ref(&value));

    assert!(NonEmpty::<ExampleValue>::try_from_vec(Vec::new()).is_err());
    assert!(serde_json::from_str::<NonEmpty<ExampleValue>>("[]").is_err());
}

#[test]
fn config_and_public_output_descriptors_have_schema_ids() {
    assert!(ExampleConfig::schema_id()
        .expect("config schema id")
        .as_str()
        .starts_with("schema:mfm.test.example_config:1:sha256-jcs-v1:"));
    assert!(ExamplePublicOutput::public_schema_id()
        .expect("public output schema id")
        .as_str()
        .starts_with("schema:mfm.test.example_public_output:1:sha256-jcs-v1:"));
}

#[test]
fn artifact_ref_checks_schema_and_semantic_ids() {
    let id = ArtifactId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([0xaa; 32]),
    );
    let digest = ContentDigest::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([0xbb; 32]),
    );
    let artifact =
        ArtifactRef::<ExampleValue>::new(id.clone(), digest.clone()).expect("artifact ref");

    let encoded = serde_json::to_string(&artifact).expect("artifact ref serializes");
    let decoded: ArtifactRef<ExampleValue> =
        serde_json::from_str(&encoded).expect("artifact ref deserializes");
    assert_eq!(decoded.id, id);
    assert_eq!(decoded.digest, digest);

    let wrong_schema = SchemaId::new(
        "mfm.test.other",
        "1",
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([0xcc; 32]),
    )
    .expect("wrong schema id fixture");

    let error = ArtifactRef::<ExampleValue>::from_parts(
        decoded.id,
        decoded.digest,
        wrong_schema,
        ExampleValue::semantic_id().expect("example semantic id"),
    )
    .expect_err("wrong schema id must reject");
    assert!(matches!(
        error,
        ValueError::ArtifactTypeMismatch {
            field: "schema_id",
            ..
        }
    ));

    let wrong_semantic = SemanticTypeId::new(
        "mfm.test",
        "other-value",
        "1",
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([0xdd; 32]),
    )
    .expect("wrong semantic id fixture");

    let error = ArtifactRef::<ExampleValue>::from_parts(
        id,
        digest,
        ExampleValue::schema_id().expect("example schema id"),
        wrong_semantic,
    )
    .expect_err("wrong semantic id must reject");
    assert!(matches!(
        error,
        ValueError::ArtifactTypeMismatch {
            field: "semantic_type_id",
            ..
        }
    ));
}

#[test]
fn duplicate_descriptor_names_reject() {
    assert!(SchemaShape::named_struct(vec![
        FieldDescriptor::required("dup", SchemaShape::String),
        FieldDescriptor::required("dup", SchemaShape::Bool),
    ])
    .is_err());

    assert!(SchemaShape::external_enum(vec![
        EnumVariantDescriptor::new("dup", SchemaShape::Unit),
        EnumVariantDescriptor::new("dup", SchemaShape::Unit),
    ])
    .is_err());
}

#[test]
fn secret_marker_scanner_rejects_keys_values_and_mnemonics() {
    assert!(string_contains_secret_marker("password=hidden"));
    assert!(string_contains_secret_marker(
        "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about"
    ));
    assert!(!string_contains_secret_marker("artifact_written"));

    let mut metadata = BTreeMap::new();
    metadata.insert("label".to_string(), "public".to_string());
    assert_eq!(string_map_secret_marker_key(&metadata), None);

    metadata.insert("mnemonic".to_string(), "redacted".to_string());
    assert_eq!(string_map_secret_marker_key(&metadata), Some("mnemonic"));
}

#[test]
fn schema_identity_rejects_invalid_kind_material() {
    let value_without_semantic = SchemaIdentity::new(
        SchemaKind::Value,
        None,
        "mfm.test.invalid",
        schema_version("1").expect("schema version"),
        SchemaShape::Unit,
    );
    assert!(value_without_semantic.is_err());

    let config_with_semantic = SchemaIdentity::new(
        SchemaKind::PlanningConfig,
        Some(ExampleValue::semantic_id().expect("example semantic id")),
        "mfm.test.invalid",
        schema_version("1").expect("schema version"),
        SchemaShape::Unit,
    );
    assert!(config_with_semantic.is_err());
}
