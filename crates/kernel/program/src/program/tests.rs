use super::*;
use serde::ser::SerializeSeq;
use std::sync::atomic::{AtomicUsize, Ordering};

fn wire<B>(bindings: B) -> ProgramWire<Vec<StateData>, B> {
    let reference = crate::nominal_contract_ref::<crate::NoParams>().unwrap();
    ProgramWire {
        domain: "mfm.program.v9".into(),
        entry_point_id: EntryPointId::new("mfm.test/capacity@1").unwrap(),
        admitted_context_contract_ref: reference.clone(),
        initial_value_ref: mfm_values::Object::from_value(&crate::NoParams)
            .unwrap()
            .value_ref()
            .clone(),
        root_success_contract_ref: reference,
        declarations: vec![],
        bindings,
        limits: ProgramLimits::new(0),
    }
}

#[test]
fn complete_wire_bounds_aggregate_bindings_before_serializing_the_suffix() {
    struct Bindings<'a> {
        object: &'a mfm_values::Object,
        calls: &'a AtomicUsize,
    }
    impl Serialize for Bindings<'_> {
        fn serialize<S: serde::Serializer>(
            &self,
            serializer: S,
        ) -> std::result::Result<S::Ok, S::Error> {
            let mut sequence = serializer.serialize_seq(Some(100))?;
            for _ in 0..100 {
                self.calls.fetch_add(1, Ordering::SeqCst);
                sequence.serialize_element(self.object)?;
            }
            sequence.end()
        }
    }
    let object = mfm_values::Object::from_value(&crate::NoParams).unwrap();
    let single = wire(vec![object.clone()]);
    let limit = serde_json::to_vec(&single).unwrap().len();
    assert!(single.canonical(limit).is_ok());
    let calls = AtomicUsize::new(0);
    let error = wire(Bindings {
        object: &object,
        calls: &calls,
    })
    .canonical(limit)
    .unwrap_err();
    let ProgramError::Encoding(error) = error else {
        panic!("missing serializer cause")
    };
    let (reported_limit, observed_at_least) = error.serialization_bound().unwrap();
    assert_eq!(reported_limit, limit);
    assert!(observed_at_least > limit);
    assert!(calls.load(Ordering::SeqCst) < 100);
    assert!(std::error::Error::source(&error)
        .unwrap()
        .source()
        .is_some());
}

#[test]
fn complete_wire_retains_non_capacity_serializer_causes() {
    struct Reject;
    impl Serialize for Reject {
        fn serialize<S: serde::Serializer>(&self, _: S) -> std::result::Result<S::Ok, S::Error> {
            Err(serde::ser::Error::custom(
                "binding codec rejected supplied value",
            ))
        }
    }
    let ProgramError::Encoding(error) = wire(Reject).canonical(8192).unwrap_err() else {
        panic!("missing serializer cause")
    };
    assert!(error.serialization_bound().is_none());
    assert!(serde_json::to_string(&error)
        .unwrap()
        .contains("binding codec rejected supplied value"));
}

#[test]
fn oversized_cold_document_is_rejected_before_json_parsing() {
    let invalid_json = vec![b'!'; MAX_RUN_OBJECT_CANONICAL_BYTES + 1];
    assert!(matches!(
        ProgramDocument::decode(&invalid_json),
        Err(ProgramError::Value(mfm_values::ValueError::SizeLimit(_)))
    ));
}

#[test]
fn bounded_wire_preserves_nested_objects_and_exact_canonical_identity() {
    #[derive(serde::Serialize, serde::Deserialize, mfm_program_derive::MfmValue)]
    #[serde(deny_unknown_fields)]
    struct Escaped {
        text: String,
    }
    let object = mfm_values::Object::from_value(&Escaped {
        text: "\"\n\t\\é".into(),
    })
    .unwrap();
    let document = wire(vec![object.clone()]);
    let expected =
        PlainCanonicalJsonBytes::from_json_str(&serde_json::to_string(&document).unwrap()).unwrap();
    let canonical = document.canonical(8192).unwrap();
    assert_eq!(canonical.as_bytes(), expected.as_bytes());
    let decoded: ProgramWire = serde_json::from_slice(canonical.as_bytes()).unwrap();
    assert_eq!(decoded.bindings[0], object);
}
