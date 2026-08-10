//! Descriptor evidence for the closed persisted-contract vocabulary.
//!
//! Every hashed element of a persisted identity is proved to change the derived
//! `SchemaId`, and every bounded form is proved to accept its exact value and
//! reject one step outside it.

use mfm_ids::{SchemaId, SchemaVersion};
use mfm_values::{
    CanonicalJsonLinesBounds, CanonicalJsonProfile, FieldDescriptor, LiteralValue, MediaType,
    PersistedEncoding, SchemaIdentity, SchemaKind, SchemaShape, SequenceOrdering, StringGrammar,
};

fn identity(shape: SchemaShape) -> SchemaIdentity {
    SchemaIdentity::new(
        SchemaKind::PersistedContract,
        None,
        "mfm.values.test-contract",
        SchemaVersion::new("1").expect("schema version"),
        shape,
    )
    .expect("persisted identity")
}

fn schema_id(shape: SchemaShape) -> SchemaId {
    identity(shape).schema_id().expect("schema id")
}

fn accepts(shape: SchemaShape, json: &str) -> bool {
    identity(shape)
        .validate_canonical_value(json.as_bytes())
        .is_ok()
}

#[test]
fn every_hashed_descriptor_element_changes_the_derived_identity() {
    let base = SchemaShape::named_struct(vec![
        FieldDescriptor::required(
            "name",
            SchemaShape::BoundedString {
                minimum_bytes: 1,
                maximum_bytes: 32,
                grammar: StringGrammar::LowerPathToken,
            },
        ),
        FieldDescriptor::required(
            "count",
            SchemaShape::UnsignedRange {
                minimum: 1,
                maximum: 8,
            },
        ),
    ])
    .expect("base shape");
    let baseline = schema_id(base.clone());

    let renamed_field = SchemaShape::named_struct(vec![
        FieldDescriptor::required(
            "label",
            SchemaShape::BoundedString {
                minimum_bytes: 1,
                maximum_bytes: 32,
                grammar: StringGrammar::LowerPathToken,
            },
        ),
        FieldDescriptor::required(
            "count",
            SchemaShape::UnsignedRange {
                minimum: 1,
                maximum: 8,
            },
        ),
    ])
    .expect("renamed field shape");
    let changed_grammar = SchemaShape::named_struct(vec![
        FieldDescriptor::required(
            "name",
            SchemaShape::BoundedString {
                minimum_bytes: 1,
                maximum_bytes: 32,
                grammar: StringGrammar::UnicodeScalarText,
            },
        ),
        FieldDescriptor::required(
            "count",
            SchemaShape::UnsignedRange {
                minimum: 1,
                maximum: 8,
            },
        ),
    ])
    .expect("changed grammar shape");
    let changed_bound = SchemaShape::named_struct(vec![
        FieldDescriptor::required(
            "name",
            SchemaShape::BoundedString {
                minimum_bytes: 1,
                maximum_bytes: 33,
                grammar: StringGrammar::LowerPathToken,
            },
        ),
        FieldDescriptor::required(
            "count",
            SchemaShape::UnsignedRange {
                minimum: 1,
                maximum: 8,
            },
        ),
    ])
    .expect("changed bound shape");
    let changed_range = SchemaShape::named_struct(vec![
        FieldDescriptor::required(
            "name",
            SchemaShape::BoundedString {
                minimum_bytes: 1,
                maximum_bytes: 32,
                grammar: StringGrammar::LowerPathToken,
            },
        ),
        FieldDescriptor::required(
            "count",
            SchemaShape::UnsignedRange {
                minimum: 0,
                maximum: 8,
            },
        ),
    ])
    .expect("changed range shape");

    for mutated in [renamed_field, changed_grammar, changed_bound, changed_range] {
        assert_ne!(baseline, schema_id(mutated));
    }

    let manual_version = SchemaIdentity::new(
        SchemaKind::PersistedContract,
        None,
        "mfm.values.test-contract",
        SchemaVersion::new("2").expect("schema version"),
        base.clone(),
    )
    .expect("versioned identity")
    .schema_id()
    .expect("versioned schema id");
    assert_ne!(baseline, manual_version);

    let other_kind = SchemaIdentity::new(
        SchemaKind::PlanningConfig,
        None,
        "mfm.values.test-contract",
        SchemaVersion::new("1").expect("schema version"),
        base,
    )
    .expect("planning identity")
    .schema_id()
    .expect("planning schema id");
    assert_ne!(baseline, other_kind);
}

#[test]
fn a_stream_encoding_is_a_distinct_identity_from_its_record_shape() {
    let record = SchemaShape::Literal(LiteralValue::String("batch".to_owned()));
    let single = schema_id(record.clone());
    let stream = SchemaIdentity::new_canonical_json_lines(
        SchemaKind::PersistedContract,
        "mfm.values.test-contract",
        SchemaVersion::new("1").expect("schema version"),
        record.clone(),
        CanonicalJsonLinesBounds {
            minimum_records: 1,
            maximum_records: 16,
            maximum_framed_record_bytes: 1_024,
            maximum_stream_bytes: 65_536,
        },
    )
    .expect("stream identity");
    assert_ne!(single, stream.schema_id().expect("stream schema id"));

    let wider = SchemaIdentity::new_canonical_json_lines(
        SchemaKind::PersistedContract,
        "mfm.values.test-contract",
        SchemaVersion::new("1").expect("schema version"),
        record,
        CanonicalJsonLinesBounds {
            minimum_records: 1,
            maximum_records: 17,
            maximum_framed_record_bytes: 1_024,
            maximum_stream_bytes: 65_536,
        },
    )
    .expect("wider stream identity");
    assert_ne!(
        stream.schema_id().expect("stream schema id"),
        wider.schema_id().expect("wider schema id"),
        "a changed stream bound changes the identity",
    );

    assert!(
        stream.canonical_json_shape().is_err(),
        "a stream encoding has no single canonical-JSON shape",
    );
    assert!(
        stream.validate_canonical_value(b"\"batch\"").is_err(),
        "canonical-value validation applies only to the canonical-JSON encoding",
    );
    assert!(matches!(
        stream.encoding,
        PersistedEncoding::CanonicalJsonLines { .. }
    ));
}

#[test]
fn bounded_strings_accept_their_exact_bounds_and_reject_one_step_outside() {
    let shape = SchemaShape::BoundedString {
        minimum_bytes: 2,
        maximum_bytes: 4,
        grammar: StringGrammar::LowerPathToken,
    };
    assert!(accepts(shape.clone(), "\"ab\""));
    assert!(accepts(shape.clone(), "\"abcd\""));
    assert!(!accepts(shape.clone(), "\"a\""));
    assert!(!accepts(shape.clone(), "\"abcde\""));
    assert!(!accepts(shape.clone(), "\"AB\""), "the grammar is enforced");
    assert!(!accepts(shape, "2"));
}

#[test]
fn identity_grammars_delegate_to_their_checked_owner() {
    let run = SchemaShape::BoundedString {
        minimum_bytes: 1,
        maximum_bytes: 128,
        grammar: StringGrammar::RunId,
    };
    assert!(accepts(
        run.clone(),
        "\"run:sha256-jcs-v1:0000000000000000000000000000000000000000000000000000000000000000\""
    ));
    assert!(!accepts(
        run,
        "\"occurrence:sha256-jcs-v1:0000000000000000000000000000000000000000000000000000000000000000\""
    ));

    let media = SchemaShape::BoundedString {
        minimum_bytes: 1,
        maximum_bytes: 64,
        grammar: StringGrammar::MediaType,
    };
    assert!(accepts(media.clone(), "\"application/json\""));
    assert!(!accepts(media, "\"application/json; charset=utf-8\""));
    assert!(MediaType::new("application/json").is_ok());
}

#[test]
fn bounded_bytes_ranges_literals_and_sequences_are_closed() {
    let bytes = SchemaShape::BoundedBytes {
        minimum_decoded_bytes: 1,
        maximum_decoded_bytes: 2,
    };
    assert!(accepts(bytes.clone(), "\"AQ\""));
    assert!(!accepts(bytes.clone(), "\"\""));
    assert!(!accepts(bytes, "\"AQID\""));

    let unsigned = SchemaShape::UnsignedRange {
        minimum: 1,
        maximum: 3,
    };
    assert!(accepts(unsigned.clone(), "1"));
    assert!(accepts(unsigned.clone(), "3"));
    assert!(!accepts(unsigned.clone(), "0"));
    assert!(!accepts(unsigned.clone(), "4"));
    assert!(!accepts(unsigned, "-1"));

    let signed = SchemaShape::SignedRange {
        minimum: -2,
        maximum: 2,
    };
    assert!(accepts(signed.clone(), "-2"));
    assert!(!accepts(signed, "-3"));

    let literal = SchemaShape::Literal(LiteralValue::String("mfm.values.test.v1".to_owned()));
    assert!(accepts(literal.clone(), "\"mfm.values.test.v1\""));
    assert!(!accepts(literal, "\"mfm.values.test.v2\""));

    let sequence = SchemaShape::BoundedSequence {
        element: Box::new(SchemaShape::UnsignedRange {
            minimum: 0,
            maximum: 9,
        }),
        minimum_items: 1,
        maximum_items: 2,
        ordering: SequenceOrdering::Preserved,
        unique: true,
    };
    assert!(accepts(sequence.clone(), "[1]"));
    assert!(accepts(sequence.clone(), "[2,1]"));
    assert!(!accepts(sequence.clone(), "[]"));
    assert!(!accepts(sequence.clone(), "[1,2,3]"));
    assert!(!accepts(sequence, "[1,1]"), "uniqueness is enforced");

    let ordered = SchemaShape::BoundedSequence {
        element: Box::new(SchemaShape::UnsignedRange {
            minimum: 0,
            maximum: 9,
        }),
        minimum_items: 0,
        maximum_items: 4,
        ordering: SequenceOrdering::CanonicalAscending,
        unique: false,
    };
    assert!(accepts(ordered.clone(), "[1,2]"));
    assert!(!accepts(ordered, "[2,1]"));
}

#[test]
fn bounded_string_maps_and_canonical_json_terminals_are_closed() {
    let map = SchemaShape::BoundedStringMap {
        key_grammar: StringGrammar::LowerPathToken,
        key_minimum_bytes: 1,
        key_maximum_bytes: 8,
        value: Box::new(SchemaShape::Bool),
        minimum_entries: 1,
        maximum_entries: 2,
    };
    assert!(accepts(map.clone(), r#"{"a":true}"#));
    assert!(!accepts(map.clone(), "{}"));
    assert!(!accepts(map.clone(), r#"{"a":true,"b":false,"c":true}"#));
    assert!(
        !accepts(map, r#"{"A":true}"#),
        "the key grammar is enforced"
    );

    let general = SchemaShape::CanonicalJsonTerminal {
        profile: CanonicalJsonProfile::GeneralFloatFree,
    };
    assert!(accepts(general.clone(), r#"{"a":-1,"b":2}"#));
    assert!(!accepts(general, r#"{"a":1.5}"#));

    let unsigned_native = SchemaShape::CanonicalJsonTerminal {
        profile: CanonicalJsonProfile::UnsignedNative,
    };
    assert!(accepts(unsigned_native.clone(), r#"{"a":2}"#));
    assert!(
        !accepts(unsigned_native.clone(), r#"{"a":-1}"#),
        "the fact profile rejects signed integers",
    );
    assert!(!accepts(unsigned_native, r#"{"a":1.5}"#));

    assert_ne!(
        schema_id(SchemaShape::CanonicalJsonTerminal {
            profile: CanonicalJsonProfile::GeneralFloatFree,
        }),
        schema_id(SchemaShape::CanonicalJsonTerminal {
            profile: CanonicalJsonProfile::UnsignedNative,
        }),
        "the number profile is part of the hashed identity",
    );
}
