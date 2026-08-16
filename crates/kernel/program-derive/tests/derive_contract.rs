use mfm_program_derive::{MfmValue, PersistedSchema};
use mfm_values::{MatchPayloadVisitor, MfmValue as _, PersistedSchema as _};
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
struct Retained {
    value: u64,
}

struct Capture;

impl MatchPayloadVisitor for Capture {
    type Output = (&'static str, Vec<u8>);

    fn visit<T: mfm_values::MfmValue>(self, tag: &'static str, payload: T) -> Self::Output {
        let (canonical, _) = mfm_values::canonicalize_mfm_value(&payload).expect("payload");
        (tag, canonical.to_vec())
    }
}

#[test]
fn surviving_derives_generate_exact_static_match_hooks() {
    const {
        assert!(<External<Payload> as mfm_values::MfmValue>::__MFM_MATCH_PROJECTION_SUPPORTED);
        assert!(!<Internal as mfm_values::MfmValue>::__MFM_MATCH_PROJECTION_SUPPORTED);
    }

    let external = External::SecondValue(Payload { value: 7 })
        .__mfm_visit_match_payload(Capture)
        .expect("external payload");
    assert_eq!(external, ("renamed", br#"{"value":7}"#.to_vec()));

    let adjacent = Adjacent::SelectedValue(Payload { value: 8 })
        .__mfm_visit_match_payload(Capture)
        .expect("adjacent payload");
    assert_eq!(adjacent, ("selected-value", br#"{"value":8}"#.to_vec()));

    let boxed = Boxed::BoxedValue(Box::new(Payload { value: 10 }))
        .__mfm_visit_match_payload(Capture)
        .expect("boxed payload");
    assert_eq!(boxed, ("boxed_value", br#"{"value":10}"#.to_vec()));

    let nested_boxed = Boxed::NestedBoxedValue(Box::new(Box::new(Payload { value: 11 })))
        .__mfm_visit_match_payload(Capture)
        .expect("nested boxed payload");
    assert_eq!(
        nested_boxed,
        ("nested_boxed_value", br#"{"value":11}"#.to_vec())
    );

    assert!(Internal::NamedValue { value: 9 }
        .__mfm_visit_match_payload(Capture)
        .is_none());
    assert!(Retained::schema_identity().is_ok());
}

#[test]
fn removed_derives_do_not_compile() {
    trybuild::TestCases::new().compile_fail("tests/ui/removed_derives.rs");
}
