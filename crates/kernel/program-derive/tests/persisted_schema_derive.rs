//! Evidence that a derived persisted contract follows its Rust definition.
//!
//! The derive exists so a retained owner cannot drift from the bytes it
//! serializes: the shape is generated from the struct, checked identity fields
//! carry their closed grammar, and a nested owner embeds its own declared shape.

use mfm_ids::{ContentDigest, ContentRef, DigestAlgorithm, DigestBytes, RunId, SchemaId, StableId};
use mfm_program_derive::PersistedSchema;
use mfm_values::{PersistedSchema as _, SchemaKind, SchemaShape, StringGrammar};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PersistedSchema)]
#[serde(deny_unknown_fields)]
#[mfm(schema = "mfm.derive.test-nested", version = "1")]
struct Nested {
    label: String,
    ordinal: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PersistedSchema)]
#[serde(deny_unknown_fields)]
#[mfm(schema = "mfm.derive.test-contract", version = "1")]
struct Contract {
    nested: Nested,
    reference: ContentRef,
    run_id: RunId,
    role: StableId,
    optional: Option<Nested>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PersistedSchema)]
#[serde(deny_unknown_fields)]
#[mfm(schema = "mfm.derive.test-contract", version = "1")]
struct WidenedContract {
    nested: WidenedNested,
    reference: ContentRef,
    run_id: RunId,
    role: StableId,
    optional: Option<WidenedNested>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PersistedSchema)]
#[serde(deny_unknown_fields)]
#[mfm(schema = "mfm.derive.test-nested", version = "1")]
struct WidenedNested {
    label: String,
    ordinal: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PersistedSchema)]
#[serde(deny_unknown_fields)]
#[mfm(schema = "mfm.derive.test-closed", version = "1")]
struct ClosedContract {
    #[mfm(literal = "mfm.closed.v1")]
    contract: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PersistedSchema)]
#[serde(deny_unknown_fields)]
#[mfm(schema = "mfm.derive.test-closed", version = "1")]
struct ChangedClosedContract {
    #[mfm(literal = "mfm.closed.v2")]
    contract: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PersistedSchema)]
#[serde(deny_unknown_fields)]
#[mfm(schema = "mfm.derive.test-sequence", version = "1")]
struct BoundedSequenceContract {
    #[mfm(persisted, minimum_items = 1, maximum_items = 2)]
    values: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PersistedSchema)]
#[serde(deny_unknown_fields)]
#[mfm(schema = "mfm.derive.test-sequence", version = "1")]
struct WidenedSequenceContract {
    #[mfm(persisted, minimum_items = 0, maximum_items = 3)]
    values: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PersistedSchema)]
#[serde(transparent)]
#[mfm(
    schema = "mfm.derive.test-bounded-unsigned",
    version = "1",
    unsigned_minimum = 1,
    unsigned_maximum = 128
)]
struct BoundedUnsigned(u8);

fn field<'a>(shape: &'a SchemaShape, name: &str) -> &'a SchemaShape {
    let SchemaShape::Struct { fields } = shape else {
        panic!("derived contract is not a struct shape");
    };
    &fields
        .iter()
        .find(|field| field.name == name)
        .unwrap_or_else(|| panic!("missing derived field {name}"))
        .shape
}

#[test]
fn a_derived_contract_declares_its_kind_name_and_rust_shape() {
    let identity = Contract::schema_identity().expect("derived identity");
    assert_eq!(identity.schema_kind, SchemaKind::PersistedContract);
    assert_eq!(identity.schema_name.as_str(), "mfm.derive.test-contract");
    assert!(
        identity.semantic_type_id.is_none(),
        "a persisted contract is not a semantic value",
    );

    let shape = identity.canonical_json_shape().expect("canonical shape");
    assert!(matches!(field(shape, "nested"), SchemaShape::Struct { .. },));
    assert!(matches!(field(shape, "optional"), SchemaShape::Option(_),));
}

#[test]
fn checked_identity_fields_carry_their_closed_grammar() {
    let identity = Contract::schema_identity().expect("derived identity");
    let shape = identity.canonical_json_shape().expect("canonical shape");

    assert!(matches!(
        field(shape, "run_id"),
        SchemaShape::BoundedString {
            grammar: StringGrammar::RunId,
            ..
        }
    ));
    assert!(matches!(
        field(shape, "role"),
        SchemaShape::BoundedString {
            grammar: StringGrammar::StableId,
            ..
        }
    ));

    let reference = field(shape, "reference");
    assert!(matches!(
        field(reference, "schema_id"),
        SchemaShape::BoundedString {
            grammar: StringGrammar::SchemaId,
            ..
        }
    ));
    assert!(matches!(
        field(reference, "content_digest"),
        SchemaShape::BoundedString {
            grammar: StringGrammar::ContentDigest,
            ..
        }
    ));
}

#[test]
fn a_changed_nested_owner_changes_the_outer_identity() {
    assert_ne!(
        Nested::schema_id().expect("nested schema id"),
        WidenedNested::schema_id().expect("widened nested schema id"),
        "widening a nested field changes that owner's identity",
    );
    assert_ne!(
        Contract::schema_id().expect("contract schema id"),
        WidenedContract::schema_id().expect("widened contract schema id"),
        "a nested change propagates into the outer contract identity",
    );
}

#[test]
fn a_literal_field_is_closed_by_the_derived_shape() {
    let identity = ClosedContract::schema_identity().expect("closed identity");
    identity
        .validate_canonical_value(br#"{"contract":"mfm.closed.v1"}"#)
        .expect("exact literal");
    assert!(identity
        .validate_canonical_value(br#"{"contract":"mfm.closed.v2"}"#)
        .is_err());
    assert_ne!(
        ClosedContract::schema_id().expect("closed schema id"),
        ChangedClosedContract::schema_id().expect("changed literal schema id"),
        "changing a literal changes the persisted schema identity",
    );
}

#[test]
fn sequence_bounds_are_enforced_and_owned_by_the_schema_identity() {
    let identity = BoundedSequenceContract::schema_identity().expect("bounded sequence identity");
    identity
        .validate_canonical_value(br#"{"values":[1]}"#)
        .expect("lower bound");
    identity
        .validate_canonical_value(br#"{"values":[1,2]}"#)
        .expect("upper bound");
    assert!(identity
        .validate_canonical_value(br#"{"values":[]}"#)
        .is_err());
    assert!(identity
        .validate_canonical_value(br#"{"values":[1,2,3]}"#)
        .is_err());
    assert_ne!(
        BoundedSequenceContract::schema_id().expect("bounded sequence schema id"),
        WidenedSequenceContract::schema_id().expect("widened sequence schema id"),
        "changing sequence bounds changes the persisted schema identity",
    );
}

#[test]
fn a_transparent_unsigned_newtype_retains_its_exact_range() {
    let identity = BoundedUnsigned::schema_identity().expect("bounded identity");
    assert_eq!(
        identity.canonical_json_shape().expect("canonical shape"),
        &SchemaShape::UnsignedRange {
            minimum: 1,
            maximum: 128,
        },
    );
    identity
        .validate_canonical_value(b"128")
        .expect("upper bound");
    assert!(identity.validate_canonical_value(b"0").is_err());
    assert!(identity.validate_canonical_value(b"129").is_err());
}

#[test]
fn a_derived_contract_accepts_its_exact_bytes_and_rejects_hostile_ones() {
    let identity = Contract::schema_identity().expect("derived identity");
    let reference = ContentRef::new(
        SchemaId::new(
            "mfm.derive.test",
            "1",
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([0; 32]),
        )
        .expect("schema id"),
        ContentDigest::from_digest(DigestAlgorithm::Sha256V1, DigestBytes::from_array([1; 32])),
    )
    .expect("content ref");
    let value = Contract {
        nested: Nested {
            label: "one".to_owned(),
            ordinal: 1,
        },
        reference,
        run_id: RunId::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([2; 32]),
        ),
        role: StableId::new("mfm.derive/test").expect("role"),
        optional: None,
    };
    let canonical = mfm_canonical::PlainCanonicalJsonBytes::from_json_str(
        &serde_json::to_string(&value).expect("serialize"),
    )
    .expect("canonicalize");
    identity
        .validate_canonical_value(canonical.as_bytes())
        .expect("the derived shape accepts its own bytes");

    let mut hostile: serde_json::Value =
        serde_json::from_slice(canonical.as_bytes()).expect("decode");
    hostile["run_id"] = serde_json::json!("not-a-run-identity");
    let hostile = mfm_canonical::PlainCanonicalJsonBytes::from_json_str(&hostile.to_string())
        .expect("canonicalize hostile");
    assert!(
        identity
            .validate_canonical_value(hostile.as_bytes())
            .is_err(),
        "the checked grammar rejects a foreign identity spelling",
    );
}
