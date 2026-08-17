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

#[test]
fn surviving_value_derives_generate_complete_schema_descriptors() {
    assert!(External::<Payload>::schema_descriptor().is_ok());
    assert!(Adjacent::<Payload>::schema_descriptor().is_ok());
    assert!(Boxed::schema_descriptor().is_ok());
    assert!(Internal::schema_descriptor().is_ok());
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
