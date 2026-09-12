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

#[test]
fn exact_32_mib_object_is_accepted_and_one_more_byte_retains_encoding_bound() {
    let exact = TextValue {
        text: "a".repeat(33_554_432 - 11),
    };
    let (canonical, _) = canonicalize_mfm_value(&exact).expect("exact object limit");
    assert_eq!(canonical.as_bytes().len(), 33_554_432);
    drop(canonical);
    let oversized = TextValue {
        text: "a".repeat(33_554_432 - 10),
    };
    let error = canonicalize_mfm_value(&oversized).unwrap_err();
    let ValueError::Canonical(cause) = error else {
        panic!("expected bounded serialization rejection")
    };
    assert_eq!(cause.serialization_bound(), Some((33_554_432, 33_554_433)));
}

#[test]
fn rejected_original_projection_keeps_native_custody_without_bypassing_secret_policy() {
    let original = mfm_values::NativeCause::from_original(std::sync::Arc::new(TextValue {
        text: "api_key=must-not-escape".into(),
    }));
    let failure = original
        .project()
        .expect_err("rejected value cannot become public error detail");
    assert!(matches!(
        failure.downcast_ref::<ValueError>(),
        Some(ValueError::SchemaShapeMismatch)
    ));
    assert_eq!(
        original.downcast_ref::<TextValue>().unwrap().text,
        "api_key=must-not-escape"
    );
    assert_eq!(
        failure.project().unwrap().get(),
        "\"schema_shape_mismatch\""
    );
    assert!(!format!("{original:?}").contains("must-not-escape"));
}

#[test]
fn object_seed_separates_native_admission_from_wire_errors() {
    use mfm_values::{Object, ObjectSeed};
    use serde::de::DeserializeSeed;

    let object = Object::from_value(&ExactValue {
        label: "public".into(),
        value: 7,
    })
    .unwrap();
    let reference = serde_json::to_string(object.value_ref()).unwrap();
    let decode = |wire: &str| {
        let mut deserializer = serde_json::Deserializer::from_str(wire);
        let result = ObjectSeed.deserialize(&mut deserializer)?;
        deserializer.end()?;
        Ok::<_, serde_json::Error>(result)
    };
    let roundtrip = serde_json::to_string(&object).unwrap();
    assert_eq!(decode(&roundtrip).unwrap().unwrap(), object);
    let stale =
        format!(r#"{{"value_ref":{reference},"canonical":{{"label":"public","value":8}}}}"#);
    let cause = decode(&stale).unwrap().unwrap_err();
    assert!(matches!(
        cause.downcast_ref::<ValueError>(),
        Some(ValueError::ArtifactTypeMismatch {
            field: "content_digest",
            ..
        })
    ));
    let noncanonical =
        format!(r#"{{"value_ref":{reference},"canonical":{{"value":7,"label":"public"}}}}"#);
    let cause = decode(&noncanonical).unwrap().unwrap_err();
    assert!(matches!(
        cause.downcast_ref::<ValueError>(),
        Some(ValueError::Canonical(_))
    ));

    for wire in [
        format!(r#"{{"value_ref":{reference}}}"#),
        format!(r#"{{"value_ref":{reference},"canonical":null,"canonical":null}}"#),
        format!(r#"{{"value_ref":{reference},"canonical":null,"unknown":null}}"#),
    ] {
        assert_eq!(
            decode(&wire).unwrap_err().classify(),
            serde_json::error::Category::Data
        );
    }
    let malformed = format!(r#"{{"value_ref":{reference},"canonical":[}}"#);
    let error = decode(&malformed).unwrap_err();
    assert_eq!(error.classify(), serde_json::error::Category::Syntax);
    assert_eq!(error.line(), 1);
    assert!(error.column() > 0);

    let oversized = format!(
        r#"{{"value_ref":{reference},"canonical":"{}"}}"#,
        "a".repeat(33_554_431)
    );
    let cause = decode(&oversized).unwrap().unwrap_err();
    let Some(ValueError::SizeLimit(size)) = cause.downcast_ref::<ValueError>() else {
        panic!("expected native measured size rejection");
    };
    assert_eq!(size.actual(), 33_554_433);
    assert_eq!(size.limit(), 33_554_432);
}
