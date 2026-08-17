use std::sync::atomic::{AtomicUsize, Ordering};

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

static DESCRIPTOR_CALLS: AtomicUsize = AtomicUsize::new(0);

#[derive(Serialize, Deserialize)]
struct CountingValue {
    label: String,
    value: u64,
}

impl mfm_values::MfmValue for CountingValue {
    fn schema_descriptor() -> mfm_values::Result<mfm_values::SchemaDescriptor> {
        DESCRIPTOR_CALLS.fetch_add(1, Ordering::SeqCst);
        <ExactValue as mfm_values::MfmValue>::schema_descriptor()
    }

    fn semantic_id() -> mfm_values::Result<SemanticTypeId> {
        <ExactValue as mfm_values::MfmValue>::semantic_id()
    }
}

#[derive(Serialize, Deserialize)]
struct OversizedShapeInvalid {
    text: String,
}

impl mfm_values::MfmValue for OversizedShapeInvalid {
    fn schema_descriptor() -> mfm_values::Result<mfm_values::SchemaDescriptor> {
        <ExactValue as mfm_values::MfmValue>::schema_descriptor()
    }

    fn semantic_id() -> mfm_values::Result<SemanticTypeId> {
        <ExactValue as mfm_values::MfmValue>::semantic_id()
    }
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
fn valid_object_over_the_shared_ceiling_is_capacity() {
    let oversized = TextValue {
        text: "a".repeat(mfm_values::MAX_RUN_OBJECT_CANONICAL_BYTES),
    };
    assert_eq!(
        canonicalize_mfm_value(&oversized),
        Err(ValueError::Capacity)
    );
}

#[test]
fn canonicalization_obtains_one_descriptor_from_its_value_owner() {
    DESCRIPTOR_CALLS.store(0, Ordering::SeqCst);
    canonicalize_mfm_value(&CountingValue {
        label: "public".to_owned(),
        value: 7,
    })
    .expect("qualified value");
    assert_eq!(DESCRIPTOR_CALLS.load(Ordering::SeqCst), 1);
}

#[test]
fn oversized_shape_invalid_value_is_capacity_before_shape_qualification() {
    let oversized = OversizedShapeInvalid {
        text: "a".repeat(mfm_values::MAX_RUN_OBJECT_CANONICAL_BYTES),
    };
    assert_eq!(
        canonicalize_mfm_value(&oversized),
        Err(ValueError::Capacity)
    );
}
