use mfm_ids::{DigestAlgorithm, DigestBytes, SemanticTypeId};
use mfm_program_derive::MfmValue;
use mfm_values::{canonicalize_mfm_value, ValueError};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct ExactValue {
    label: String,
    value: u64,
}

#[derive(Serialize, Deserialize)]
struct DishonestValue {
    label: String,
    value: u64,
}

impl mfm_values::MfmValue for DishonestValue {
    fn schema_descriptor() -> mfm_values::Result<mfm_values::SchemaDescriptor> {
        <ExactValue as mfm_values::MfmValue>::schema_descriptor()
    }

    fn semantic_id() -> mfm_values::Result<SemanticTypeId> {
        SemanticTypeId::new(
            "mfm.test",
            "dishonest-value",
            "1",
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([9; 32]),
        )
        .map_err(ValueError::Identity)
    }
}

#[derive(Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct TextValue {
    text: String,
}

// Content addressing must use the declared schema and exact canonical bytes, and reject values
// that fail admission.
#[test]
fn canonicalization_proves_descriptor_bytes_secret_policy_and_exact_digest() {
    let value = ExactValue {
        label: "public".to_owned(),
        value: 7,
    };
    let (canonical, reference) = canonicalize_mfm_value(&value).expect("qualified value");
    assert_eq!(canonical.as_bytes(), br#"{"label":"public","value":7}"#);
    assert_eq!(
        reference.content_digest().as_str(),
        "content:sha256-v1:848c161c2ee07303ec36cfa929542e95b4dab1618fce1c85d1cf10121cb351ae"
    );
    assert!(matches!(
        canonicalize_mfm_value(&DishonestValue {
            label: "public".to_owned(),
            value: 7,
        }),
        Err(ValueError::Descriptor(_))
    ));
    assert!(matches!(
        canonicalize_mfm_value(&TextValue {
            text: "api_key=do-not-persist".to_owned(),
        }),
        Err(ValueError::SchemaShapeMismatch)
    ));
}

// Loading an Object must validate its claimed digest and canonical payload while preserving
// useful parser rejection details.
#[test]
fn object_deserialize_checks_admission_and_reports_parser_errors() {
    use mfm_values::Object;

    let object = Object::from_value(&ExactValue {
        label: "public".into(),
        value: 7,
    })
    .unwrap();
    let reference = serde_json::to_string(object.value_ref()).unwrap();
    let decode = |wire: &str| serde_json::from_str::<Object>(wire);
    let roundtrip = serde_json::to_string(&object).unwrap();
    assert_eq!(decode(&roundtrip).unwrap(), object);
    let sequence = format!(r#"[{reference},{{"label":"public","value":7}}]"#);
    assert_eq!(decode(&sequence).unwrap(), object);
    let stale =
        format!(r#"{{"value_ref":{reference},"canonical":{{"label":"public","value":8}}}}"#);
    let error = decode(&stale).unwrap_err();
    assert_eq!(error.classify(), serde_json::error::Category::Data);
    assert!(error.to_string().contains("content_digest"));
    let noncanonical =
        format!(r#"{{"value_ref":{reference},"canonical":{{"value":7,"label":"public"}}}}"#);
    assert_eq!(
        decode(&noncanonical).unwrap_err().classify(),
        serde_json::error::Category::Data
    );

    for wire in [
        format!(r#"{{"value_ref":{reference}}}"#),
        format!(r#"{{"value_ref":{reference},"canonical":null,"canonical":null}}"#),
        format!(r#"{{"value_ref":{reference},"canonical":null,"unknown":null}}"#),
        r#"{"value_ref":"invalid","canonical":null}"#.into(),
    ] {
        assert_eq!(
            decode(&wire).unwrap_err().classify(),
            serde_json::error::Category::Data
        );
    }
    assert!(decode(&format!("{roundtrip} null")).is_err());
    let malformed = format!(r#"{{"value_ref":{reference},"canonical":[}}"#);
    let error = decode(&malformed).unwrap_err();
    assert_eq!(error.classify(), serde_json::error::Category::Syntax);
    assert_eq!(error.line(), 1);
    assert!(error.column() > 0);
}
