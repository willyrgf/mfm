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
        .map_err(|error| ValueError::Identity(error.to_string()))
    }
}

#[derive(Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct TextValue {
    text: String,
}

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
    assert_eq!(
        canonicalize_mfm_value(&TextValue {
            text: "api_key=do-not-persist".to_owned(),
        }),
        Err(ValueError::SchemaShapeMismatch)
    );
}

#[test]
fn exact_32_mib_object_is_accepted_and_one_more_byte_has_precise_error() {
    let exact = TextValue {
        text: "a".repeat(33_554_432 - 11),
    };
    let (canonical, _) = canonicalize_mfm_value(&exact).expect("exact object limit");
    assert_eq!(canonical.as_bytes().len(), 33_554_432);
    drop(canonical);
    let oversized = TextValue {
        text: "a".repeat(33_554_432 - 10),
    };
    assert_eq!(
        canonicalize_mfm_value(&oversized),
        Err(ValueError::SizeLimit(
            mfm_values::SizeLimitExceeded::check(33_554_433, 33_554_432).unwrap_err()
        ))
    );
}
