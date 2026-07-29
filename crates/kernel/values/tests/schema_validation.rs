use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_ids::{DigestAlgorithm, DigestBytes, SchemaVersion, SemanticTypeId};
use mfm_values::{
    DecimalScale, EnumTagging, EnumVariantDescriptor, FieldDescriptor, GenericArgumentDescriptor,
    SchemaIdentity, SchemaKind, SchemaShape, MAX_SCHEMA_DEPTH,
};

fn semantic_id() -> SemanticTypeId {
    SemanticTypeId::new(
        "mfm.test",
        "schema-validation",
        "1",
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([0x71; 32]),
    )
    .expect("semantic id")
}

fn identity(name: &str, shape: SchemaShape) -> SchemaIdentity {
    SchemaIdentity::new(
        SchemaKind::Value,
        Some(semantic_id()),
        name,
        SchemaVersion::new("1").expect("schema version"),
        shape,
    )
    .expect("schema identity")
}

fn canonical(value: serde_json::Value) -> Vec<u8> {
    PlainCanonicalJsonBytes::from_json_str(&value.to_string())
        .expect("canonical JSON")
        .to_vec()
}

#[test]
fn strict_identity_codec_rejects_policy_shape_order_and_depth_tampering() {
    let strict_identity = identity(
        "mfm.test.strict_identity",
        SchemaShape::named_struct(vec![
            FieldDescriptor::required("left", SchemaShape::Bool),
            FieldDescriptor::required("right", SchemaShape::UnsignedInteger { bits: 16 }),
        ])
        .expect("shape"),
    );
    let canonical_identity = strict_identity
        .canonical_json()
        .expect("canonical identity");
    assert_eq!(
        SchemaIdentity::strict_decode(canonical_identity.as_bytes()).expect("strict identity"),
        strict_identity
    );

    let mut wrong_policy: serde_json::Value =
        serde_json::from_slice(canonical_identity.as_bytes()).expect("identity JSON");
    wrong_policy["persisted_surface"]["numbers"] =
        serde_json::Value::String("floats_allowed".to_owned());
    assert!(SchemaIdentity::strict_decode(&canonical(wrong_policy)).is_err());

    let mut wrong_width: serde_json::Value =
        serde_json::from_slice(canonical_identity.as_bytes()).expect("identity JSON");
    wrong_width["shape"]["fields"][1]["shape"]["bits"] = serde_json::json!(7);
    assert!(SchemaIdentity::strict_decode(&canonical(wrong_width)).is_err());

    let mut unsorted: serde_json::Value =
        serde_json::from_slice(canonical_identity.as_bytes()).expect("identity JSON");
    unsorted["shape"]["fields"]
        .as_array_mut()
        .expect("fields")
        .swap(0, 1);
    assert!(SchemaIdentity::strict_decode(&canonical(unsorted)).is_err());

    let mut unknown_identity: serde_json::Value =
        serde_json::from_slice(canonical_identity.as_bytes()).expect("identity JSON");
    unknown_identity
        .as_object_mut()
        .expect("identity object")
        .insert("unknown".to_owned(), serde_json::Value::Bool(true));
    assert!(SchemaIdentity::strict_decode(&canonical(unknown_identity)).is_err());

    let mut unknown_shape: serde_json::Value =
        serde_json::from_slice(canonical_identity.as_bytes()).expect("identity JSON");
    unknown_shape["shape"]
        .as_object_mut()
        .expect("shape object")
        .insert("unknown".to_owned(), serde_json::Value::Bool(true));
    assert!(SchemaIdentity::strict_decode(&canonical(unknown_shape)).is_err());

    let mut duplicate_field: serde_json::Value =
        serde_json::from_slice(canonical_identity.as_bytes()).expect("identity JSON");
    let fields = duplicate_field["shape"]["fields"]
        .as_array_mut()
        .expect("fields");
    fields.push(fields[0].clone());
    assert!(SchemaIdentity::strict_decode(&canonical(duplicate_field)).is_err());

    let mut at_bound = SchemaShape::Bool;
    for _ in 0..MAX_SCHEMA_DEPTH {
        at_bound = SchemaShape::Option(Box::new(at_bound));
    }
    let at_bound = identity("mfm.test.at_depth_bound", at_bound);
    let canonical_at_bound = at_bound
        .canonical_json()
        .expect("bounded canonical identity");
    assert_eq!(
        SchemaIdentity::strict_decode(canonical_at_bound.as_bytes())
            .expect("bounded strict decode"),
        at_bound
    );
    at_bound
        .validate_canonical_value(&canonical(serde_json::json!(true)))
        .expect("bounded nested value");

    let too_deep = SchemaShape::Option(Box::new(at_bound.shape));
    assert!(SchemaIdentity::new(
        SchemaKind::Value,
        Some(semantic_id()),
        "mfm.test.too_deep",
        SchemaVersion::new("1").expect("schema version"),
        too_deep,
    )
    .is_err());

    let oversized_shape = SchemaShape::named_struct(
        (0..1_500)
            .map(|index| FieldDescriptor::required(format!("field_{index:04}"), SchemaShape::Bool))
            .collect(),
    )
    .expect("oversized shape");
    assert!(SchemaIdentity::new(
        SchemaKind::Value,
        Some(semantic_id()),
        "mfm.test.oversized",
        SchemaVersion::new("1").expect("schema version"),
        oversized_shape.clone(),
    )
    .is_err());
    let mut oversized_identity = strict_identity;
    oversized_identity.shape = oversized_shape;
    assert!(oversized_identity.canonical_json().is_err());
}

#[test]
fn structs_enforce_required_default_exactness_and_numeric_ranges() {
    let identity = identity(
        "mfm.test.struct_validation",
        SchemaShape::named_struct(vec![
            FieldDescriptor::required("bytes", SchemaShape::Bytes),
            FieldDescriptor::with_default(
                "defaulted",
                SchemaShape::Vec(Box::new(SchemaShape::Bool)),
            ),
            FieldDescriptor::required(
                "decimal",
                SchemaShape::DecimalString {
                    scale: DecimalScale::Fixed(2),
                },
            ),
            FieldDescriptor::required(
                "nullable",
                SchemaShape::Option(Box::new(SchemaShape::String)),
            ),
            FieldDescriptor::required("signed", SchemaShape::SignedInteger { bits: 8 }),
            FieldDescriptor::required("unsigned", SchemaShape::UnsignedInteger { bits: 8 }),
        ])
        .expect("shape"),
    );

    let valid = canonical(serde_json::json!({
        "bytes": "AAE-_w",
        "decimal": "-10.50",
        "nullable": null,
        "signed": -128,
        "unsigned": 255
    }));
    identity
        .validate_canonical_value(&valid)
        .expect("exact valid value");

    let mut missing_required: serde_json::Value =
        serde_json::from_slice(&valid).expect("valid JSON");
    missing_required
        .as_object_mut()
        .expect("object")
        .remove("nullable");
    assert!(identity
        .validate_canonical_value(&canonical(missing_required))
        .is_err());

    let mut extra: serde_json::Value = serde_json::from_slice(&valid).expect("valid JSON");
    extra
        .as_object_mut()
        .expect("object")
        .insert("extra".to_owned(), serde_json::Value::Bool(true));
    assert!(identity
        .validate_canonical_value(&canonical(extra))
        .is_err());

    let mut signed_overflow: serde_json::Value =
        serde_json::from_slice(&valid).expect("valid JSON");
    signed_overflow["signed"] = serde_json::json!(-129);
    assert!(identity
        .validate_canonical_value(&canonical(signed_overflow))
        .is_err());

    let mut unsigned_overflow: serde_json::Value =
        serde_json::from_slice(&valid).expect("valid JSON");
    unsigned_overflow["unsigned"] = serde_json::json!(256);
    assert!(identity
        .validate_canonical_value(&canonical(unsigned_overflow))
        .is_err());

    let noncanonical =
        br#"{ "bytes":"AAE-_w","decimal":"-10.50","nullable":null,"signed":-128,"unsigned":255}"#;
    assert!(identity.validate_canonical_value(noncanonical).is_err());
}

#[test]
fn every_enum_tagging_mode_enforces_exact_serde_wire_forms() {
    let variants = || {
        vec![
            EnumVariantDescriptor::new(
                "named",
                SchemaShape::named_struct(vec![FieldDescriptor::required(
                    "enabled",
                    SchemaShape::Bool,
                )])
                .expect("named payload"),
            ),
            EnumVariantDescriptor::new(
                "newtype",
                SchemaShape::Tuple(vec![SchemaShape::UnsignedInteger { bits: 8 }]),
            ),
            EnumVariantDescriptor::new("unit", SchemaShape::Unit),
        ]
    };

    let external = identity(
        "mfm.test.external_enum",
        SchemaShape::tagged_enum(EnumTagging::External, variants()).expect("external enum"),
    );
    for value in [
        serde_json::json!("unit"),
        serde_json::json!({"newtype": 7}),
        serde_json::json!({"named": {"enabled": true}}),
    ] {
        external
            .validate_canonical_value(&canonical(value))
            .expect("external wire");
    }
    assert!(external
        .validate_canonical_value(&canonical(serde_json::json!({"unit": null})))
        .is_err());
    assert!(external
        .validate_canonical_value(&canonical(serde_json::json!({"newtype": [7]})))
        .is_err());

    let internal = identity(
        "mfm.test.internal_enum",
        SchemaShape::tagged_enum(
            EnumTagging::Internal {
                tag: "kind".to_owned(),
            },
            vec![
                EnumVariantDescriptor::new(
                    "named",
                    SchemaShape::named_struct(vec![FieldDescriptor::required(
                        "enabled",
                        SchemaShape::Bool,
                    )])
                    .expect("named payload"),
                ),
                EnumVariantDescriptor::new("unit", SchemaShape::Unit),
            ],
        )
        .expect("internal enum"),
    );
    for value in [
        serde_json::json!({"kind": "unit"}),
        serde_json::json!({"enabled": true, "kind": "named"}),
    ] {
        internal
            .validate_canonical_value(&canonical(value))
            .expect("internal wire");
    }
    assert!(internal
        .validate_canonical_value(&canonical(
            serde_json::json!({"kind": "unit", "payload": null})
        ))
        .is_err());

    let adjacent = identity(
        "mfm.test.adjacent_enum",
        SchemaShape::tagged_enum(
            EnumTagging::Adjacent {
                tag: "kind".to_owned(),
                content: "payload".to_owned(),
            },
            variants(),
        )
        .expect("adjacent enum"),
    );
    for value in [
        serde_json::json!({"kind": "unit"}),
        serde_json::json!({"kind": "newtype", "payload": 7}),
        serde_json::json!({"kind": "named", "payload": {"enabled": true}}),
    ] {
        adjacent
            .validate_canonical_value(&canonical(value))
            .expect("adjacent wire");
    }
    assert!(adjacent
        .validate_canonical_value(&canonical(
            serde_json::json!({"kind": "unit", "payload": null})
        ))
        .is_err());
}

#[test]
fn inline_and_generic_shapes_validate_nested_values_without_a_registry() {
    let nested = identity(
        "mfm.test.nested",
        SchemaShape::named_struct(vec![
            FieldDescriptor::required("count", SchemaShape::UnsignedInteger { bits: 16 }),
            FieldDescriptor::required("label", SchemaShape::String),
        ])
        .expect("nested shape"),
    );
    let inline = SchemaShape::InlineValue {
        schema_id: nested.schema_id().expect("nested schema id"),
        semantic_type_id: nested.semantic_type_id.clone().expect("nested semantic id"),
        serialized_shape: Box::new(nested.shape.clone()),
    };
    let generic = SchemaShape::Generic {
        constructor: "mfm.test/wrapper".to_owned(),
        arguments: vec![GenericArgumentDescriptor {
            schema_id: nested.schema_id().expect("nested schema id"),
            semantic_type_id: nested.semantic_type_id.clone().expect("nested semantic id"),
        }],
        serialized_shape: Box::new(inline),
    };
    let outer = identity("mfm.test.generic", generic);

    outer
        .validate_canonical_value(&canonical(serde_json::json!({
            "count": 42,
            "label": "public"
        })))
        .expect("nested value");
    assert!(outer
        .validate_canonical_value(&canonical(serde_json::json!({
            "count": 42,
            "extra": true,
            "label": "public"
        })))
        .is_err());
}

#[test]
fn descriptor_validation_rejects_enum_collisions_and_malformed_generic_metadata() {
    let colliding = SchemaShape::tagged_enum(
        EnumTagging::Internal {
            tag: "kind".to_owned(),
        },
        vec![EnumVariantDescriptor::new(
            "named",
            SchemaShape::named_struct(vec![FieldDescriptor::required("kind", SchemaShape::String)])
                .expect("named payload"),
        )],
    )
    .expect("enum shape");
    assert!(SchemaIdentity::new(
        SchemaKind::Value,
        Some(semantic_id()),
        "mfm.test.colliding_enum",
        SchemaVersion::new("1").expect("schema version"),
        colliding,
    )
    .is_err());

    let enum_identity = identity(
        "mfm.test.duplicate_variant",
        SchemaShape::external_enum(vec![
            EnumVariantDescriptor::new("first", SchemaShape::Unit),
            EnumVariantDescriptor::new("second", SchemaShape::Unit),
        ])
        .expect("enum"),
    );
    let mut duplicate_variant: serde_json::Value = serde_json::from_slice(
        enum_identity
            .canonical_json()
            .expect("canonical enum identity")
            .as_bytes(),
    )
    .expect("identity JSON");
    let variants = duplicate_variant["shape"]["variants"]
        .as_array_mut()
        .expect("variants");
    variants.push(variants[0].clone());
    assert!(SchemaIdentity::strict_decode(&canonical(duplicate_variant)).is_err());

    let malformed_generic = SchemaShape::Generic {
        constructor: "not a constructor".to_owned(),
        arguments: Vec::new(),
        serialized_shape: Box::new(SchemaShape::Bool),
    };
    assert!(SchemaIdentity::new(
        SchemaKind::Value,
        Some(semantic_id()),
        "mfm.test.malformed_generic",
        SchemaVersion::new("1").expect("schema version"),
        malformed_generic,
    )
    .is_err());
}
