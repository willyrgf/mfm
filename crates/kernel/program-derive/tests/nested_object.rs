use mfm_canonical::{raw_content_digest, PlainCanonicalJsonBytes};
use mfm_ids::ContentRef;
use mfm_program_derive::MfmValue;
use mfm_values::{MfmValue as _, Object};
use serde::{Deserialize, Serialize};

#[derive(Debug, PartialEq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct Native {
    first: u64,
    second: u64,
}

#[derive(Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct Envelope {
    native: Object,
}

#[derive(Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct QualifiedEnvelope {
    native: mfm_values::Object,
}

#[test]
fn nested_objects_preserve_exact_bytes_and_require_the_selected_native_contract() {
    let native = Object::from_value(&Native {
        first: 1,
        second: 2,
    })
    .unwrap();
    let envelope = Object::from_value(&Envelope {
        native: native.clone(),
    })
    .unwrap();
    let decoded = envelope.decode::<Envelope>().unwrap();
    assert_eq!(decoded.native.value_ref(), native.value_ref());
    assert_eq!(
        decoded.native.canonical_bytes(),
        br#"{"first":1,"second":2}"#
    );
    assert_eq!(
        decoded.native.decode::<Native>().unwrap(),
        Native {
            first: 1,
            second: 2
        }
    );
    assert!(decoded.native.decode::<Envelope>().is_err());
    let qualified = Object::from_value(&QualifiedEnvelope {
        native: native.clone(),
    })
    .unwrap();
    assert_eq!(
        qualified.decode::<QualifiedEnvelope>().unwrap().native,
        native
    );
}

#[test]
fn structural_admission_does_not_replace_nested_digest_validation() {
    let native = Object::from_value(&Native {
        first: 1,
        second: 2,
    })
    .unwrap();
    let mut wire = serde_json::to_value(Envelope { native }).unwrap();
    wire["native"]["canonical"]["first"] = 3.into();
    let canonical = PlainCanonicalJsonBytes::from_json_str(&wire.to_string()).unwrap();
    let descriptor = Envelope::schema_descriptor().unwrap();
    let reference = ContentRef::new(
        descriptor.schema_id().unwrap(),
        raw_content_digest(canonical.as_bytes()),
    )
    .unwrap();
    let object = Object::from_canonical(reference, canonical.as_bytes()).unwrap();
    object.admit(&descriptor).unwrap();
    assert!(object.decode::<Envelope>().is_err());
}

#[test]
fn typed_envelopes_reject_noncanonical_payloads_invalid_references_and_unknown_fields() {
    let native = Object::from_value(&Native {
        first: 1,
        second: 2,
    })
    .unwrap();
    let wire = serde_json::to_string(&Envelope { native }).unwrap();
    let noncanonical = wire.replace(r#"{"first":1,"second":2}"#, r#"{"second":2,"first":1}"#);
    assert_ne!(wire, noncanonical);
    assert!(serde_json::from_str::<Envelope>(&noncanonical).is_err());
    let mut unknown: serde_json::Value = serde_json::from_str(&wire).unwrap();
    unknown["native"]["extra"] = true.into();
    assert!(serde_json::from_str::<Envelope>(&unknown.to_string()).is_err());
    let mut invalid: serde_json::Value = serde_json::from_str(&wire).unwrap();
    invalid["native"]["value_ref"] = "invalid".into();
    assert!(serde_json::from_str::<Envelope>(&invalid.to_string()).is_err());
}

#[test]
fn generic_object_shape_preserves_trusted_text_without_certifying_native_meaning() {
    // This checks generic custody only. The deliberately unadmitted payload has no Native claim.
    let bytes = br#"{"message":"provider mentioned password field"}"#;
    let reference = ContentRef::new(
        Native::schema_descriptor().unwrap().schema_id().unwrap(),
        raw_content_digest(bytes),
    )
    .unwrap();
    let native = Object::from_canonical(reference, bytes).unwrap();
    let envelope = Object::from_value(&Envelope { native }).unwrap();
    let decoded = envelope.decode::<Envelope>().unwrap();
    assert_eq!(decoded.native.canonical_bytes(), bytes);
    assert!(decoded.native.decode::<Native>().is_err());
}
