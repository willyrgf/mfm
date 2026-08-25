use std::collections::BTreeMap;
use std::num::NonZeroU64;

use mfm_canonical::CanonicalBytes;
use mfm_program_derive::{MfmValue, PersistedSchema};
use mfm_values::{MfmValue as _, PersistedSchema as _};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct Payload {
    value: u64,
}

#[derive(Serialize, Deserialize, MfmValue)]
#[serde(rename_all = "snake_case")]
enum External<T> {
    FirstValue(T),
    #[serde(rename = "renamed")]
    SecondValue(T),
}

#[derive(Serialize, Deserialize, MfmValue)]
#[serde(tag = "kind", content = "payload", rename_all = "kebab-case")]
enum Adjacent<T> {
    SelectedValue(T),
}

#[derive(Serialize, Deserialize, MfmValue)]
#[serde(rename_all = "snake_case")]
#[allow(clippy::redundant_allocation)] // Exercises recursive Box descriptor/projection parity.
enum Boxed {
    BoxedValue(Box<Payload>),
    NestedBoxedValue(Box<Box<Payload>>),
}

#[derive(Serialize, Deserialize, MfmValue)]
#[serde(tag = "kind", rename_all = "camelCase")]
enum Internal {
    NamedValue { value: u64 },
}

#[derive(Serialize, Deserialize, PersistedSchema)]
#[serde(deny_unknown_fields)]
struct Retained<T> {
    value: T,
}

#[derive(Serialize, Deserialize, MfmValue)]
struct FirstRetainedValue {
    first: u64,
}

#[derive(Serialize, Deserialize, MfmValue)]
struct SecondRetainedValue {
    second: String,
}

#[derive(Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct EffectIdentityValue {
    effect_id: mfm_ids::EffectId,
}

#[derive(Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct EffectIdentityContainer {
    effect_ids: Vec<mfm_ids::EffectId>,
}

#[derive(Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct FixedPublicBytes {
    #[mfm(minimum_bytes = 2, maximum_bytes = 2)]
    value: CanonicalBytes,
}

#[derive(Serialize, Deserialize, MfmValue)]
#[serde(transparent)]
struct TransparentFixedPublicBytes {
    #[mfm(minimum_bytes = 2, maximum_bytes = 2)]
    value: CanonicalBytes,
}

#[derive(Serialize, Deserialize, MfmValue)]
#[serde(transparent)]
struct TransparentFixedPublicText {
    #[mfm(minimum_bytes = 2, maximum_bytes = 2)]
    value: String,
}

#[derive(Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct NonzeroPublicNumber {
    value: NonZeroU64,
}

#[derive(Serialize, Deserialize, MfmValue)]
#[serde(transparent)]
struct TransparentNonzero(NonZeroU64);

#[derive(Serialize, Deserialize, MfmValue)]
#[serde(transparent)]
struct TransparentMap(BTreeMap<String, u64>);

#[test]
fn surviving_value_derives_generate_complete_schema_descriptors() {
    assert!(External::<Payload>::schema_descriptor().is_ok());
    assert!(Adjacent::<Payload>::schema_descriptor().is_ok());
    assert!(Boxed::schema_descriptor().is_ok());
    assert!(Internal::schema_descriptor().is_ok());
    assert!(EffectIdentityValue::schema_descriptor()
        .expect("EffectId descriptor")
        .identity_canonical_json()
        .expect("identity")
        .as_str()
        .contains("\"grammar\":\"effect_id\""));
    assert!(EffectIdentityContainer::schema_descriptor()
        .expect("EffectId container descriptor")
        .identity_canonical_json()
        .expect("identity")
        .as_str()
        .contains("\"grammar\":\"effect_id\""));
    let bytes = FixedPublicBytes::schema_descriptor().expect("bytes descriptor");
    assert!(bytes
        .identity()
        .validate_canonical_value(br#"{"value":"AQI"}"#)
        .is_ok());
    assert!(bytes
        .identity()
        .validate_canonical_value(br#"{"value":"AQ"}"#)
        .is_err());
    assert_eq!(
        serde_json::to_string(&FixedPublicBytes {
            value: CanonicalBytes::new([1, 2]),
        })
        .expect("canonical bytes wire"),
        r#"{"value":"AQI"}"#
    );
    let transparent_bytes =
        TransparentFixedPublicBytes::schema_descriptor().expect("transparent bytes descriptor");
    assert!(transparent_bytes
        .identity()
        .validate_canonical_value(br#""AQI""#)
        .is_ok());
    assert!(transparent_bytes
        .identity()
        .validate_canonical_value(br#""AQ""#)
        .is_err());
    let transparent_text =
        TransparentFixedPublicText::schema_descriptor().expect("transparent text descriptor");
    assert!(transparent_text
        .identity()
        .validate_canonical_value(br#""ab""#)
        .is_ok());
    assert!(transparent_text
        .identity()
        .validate_canonical_value(br#""abc""#)
        .is_err());
    let number = NonzeroPublicNumber::schema_descriptor().expect("number descriptor");
    assert!(number
        .identity()
        .validate_canonical_value(br#"{"value":1}"#)
        .is_ok());
    assert!(number
        .identity()
        .validate_canonical_value(br#"{"value":0}"#)
        .is_err());
    assert!(number
        .identity()
        .validate_canonical_value(br#"{"value":18446744073709551615}"#)
        .is_ok());
    let transparent_nonzero =
        TransparentNonzero::schema_descriptor().expect("transparent nonzero descriptor");
    assert!(transparent_nonzero
        .identity()
        .validate_canonical_value(b"1")
        .is_ok());
    assert!(transparent_nonzero
        .identity()
        .validate_canonical_value(b"0")
        .is_err());
    let transparent_map = TransparentMap::schema_descriptor().expect("transparent map descriptor");
    assert!(transparent_map
        .identity()
        .validate_canonical_value(br#"{"key":1}"#)
        .is_ok());
}

#[test]
fn generic_persisted_derives_keep_monomorphization_specific_identity() {
    let second_identity = Retained::<SecondRetainedValue>::schema_identity().expect("second");
    let second_schema = Retained::<SecondRetainedValue>::schema_id().expect("second schema");
    let first_identity = Retained::<FirstRetainedValue>::schema_identity().expect("first");
    let first_schema = Retained::<FirstRetainedValue>::schema_id().expect("first schema");

    assert_ne!(first_identity, second_identity);
    assert_ne!(first_schema, second_schema);
    assert_eq!(
        first_identity.schema_id().expect("first identity schema"),
        first_schema
    );
    assert_eq!(
        second_identity.schema_id().expect("second identity schema"),
        second_schema
    );
}

#[test]
fn removed_derives_do_not_compile() {
    trybuild::TestCases::new().compile_fail("tests/ui/removed_derives.rs");
}
