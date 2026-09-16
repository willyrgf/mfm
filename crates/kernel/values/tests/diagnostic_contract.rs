use mfm_program_derive::MfmValue;
use mfm_values::{DiagnosticEvidence, InvocationDiagnostic, Object, SizeResource, SizeViolation};
use serde::{Deserialize, Serialize};

#[derive(Debug, PartialEq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct Failure {
    operation: String,
    #[mfm(persisted)]
    details: DiagnosticEvidence,
}

// The diagnostic-text exception must survive whole-object admission without allowing ordinary
// sensitive text or floating-point values.
#[test]
fn whole_owner_admits_diagnostic_text_but_retains_ordinary_text_and_numeric_rules() {
    let mut failure = Failure {
        operation: "read".into(),
        details: DiagnosticEvidence::from_value(serde_json::json!({
            "message": "provider reports api_key missing",
            "data_json": "{\"ratio\":1.2300e-4}",
            "sources": [{"message": "detail".repeat(2000)}],
        })),
    };
    let object = Object::from_value(&failure).unwrap();
    assert_eq!(object.decode::<Failure>().unwrap(), failure);
    failure.operation = "api_key missing".into();
    assert!(Object::from_value(&failure).is_err());
    failure.operation = "read".into();
    failure.details = DiagnosticEvidence::from_value(serde_json::json!({"ratio": 1.25}));
    assert!(Object::from_value(&failure).is_err());
}

// If diagnostic encoding fails or panics, keep the known operation and size facts without
// invoking the failing serializer again.
#[test]
fn invocation_conversion_failure_keeps_primary_facts_without_retrying_serialization() {
    struct Failing {
        panic: bool,
        calls: std::cell::Cell<usize>,
    }
    impl Serialize for Failing {
        fn serialize<S: serde::Serializer>(&self, _: S) -> Result<S::Ok, S::Error> {
            self.calls.set(self.calls.get() + 1);
            if self.panic {
                std::panic::panic_any(17_u8);
            }
            Err(serde::ser::Error::custom("selected field encoding failed"))
        }
    }
    for (panic, reason) in [(false, "encoding_failed"), (true, "panicked")] {
        let fields = Failing {
            panic,
            calls: std::cell::Cell::new(0),
        };
        let size = SizeViolation::Measured {
            resource: SizeResource::CanonicalObject,
            actual: 101,
            limit: 100,
        };
        let diagnostic =
            InvocationDiagnostic::from_fields("value_error", "encode", &fields, Some(size));
        assert_eq!(diagnostic.code(), "value_error");
        assert_eq!(diagnostic.operation(), "encode");
        assert_eq!(diagnostic.size(), Some(size));
        assert_eq!(
            diagnostic.details().as_value(),
            &serde_json::json!({"omitted": {"reason": reason}})
        );
        serde_json::to_value(&diagnostic).unwrap();
        assert_eq!(fields.calls.get(), 1);
    }
}
