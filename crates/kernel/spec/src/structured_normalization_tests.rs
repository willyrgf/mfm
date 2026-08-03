use super::*;

fn stable(value: &str) -> StableId {
    StableId::new(value).expect("stable fixture id")
}

fn fixture_ref(value: &str) -> ContentRef {
    let schema_name = format!("mfm.fixture.{value}");
    ContentRef::new(
        SchemaId::new(
            &schema_name,
            "1",
            DigestAlgorithm::Sha256JcsV1,
            sha256_digest_bytes(format!("schema:{value}").as_bytes()),
        )
        .expect("fixture schema"),
        ContentDigest::from_digest(
            DigestAlgorithm::Sha256V1,
            sha256_digest_bytes(format!("content:{value}").as_bytes()),
        ),
    )
    .expect("fixture content ref")
}

fn fixture_program() -> AuthoredStructuredProgram {
    let operation_id = stable("mfm.fixture/normalized-program");
    let root_path = StructuralPath::new(vec![StructuralPathSegment::Root {
        operation_id: operation_id.clone(),
    }])
    .expect("root path");
    let contract_ref = fixture_ref("value");
    let first = LexicalSlot {
        lexical_path: root_path.clone(),
        contract_ref: contract_ref.clone(),
        producer: LexicalProducer::AdmissionRoot {
            root_id: stable("first"),
        },
    };
    let second = LexicalSlot {
        lexical_path: root_path.clone(),
        contract_ref: contract_ref.clone(),
        producer: LexicalProducer::AdmissionRoot {
            root_id: stable("second"),
        },
    };
    let selected_arm_path = root_path
        .child(StructuralPathSegment::MatchArm {
            label: stable("selected"),
            tag: "selected".to_owned(),
        })
        .expect("selected arm path");
    let alias = LexicalSlot {
        lexical_path: root_path.clone(),
        contract_ref: contract_ref.clone(),
        producer: LexicalProducer::ArmValue {
            selected_arm_path,
            source: Box::new(first.clone()),
        },
    };
    AuthoredStructuredProgram {
        operation_id,
        input_roots: vec![first, second],
        output_contract_ref: contract_ref,
        failure_contract: StructuredFailureContract::Never,
        root: AuthoredBlock {
            path: root_path,
            failure_scope: FailureScopeBinding::Owns {
                scope: FailureScope {
                    scope_id: stable("root-scope"),
                    failure_contract: StructuredFailureContract::Never,
                    default_mappers: Vec::new(),
                },
            },
            declarations: Vec::new(),
            tail: BlockTail::Normal(alias),
        },
    }
}

fn normalized_json() -> serde_json::Value {
    serde_json::to_value(fixture_program()).expect("normalized program JSON")
}

fn decode_error(value: serde_json::Value) -> String {
    serde_json::from_value::<AuthoredStructuredProgram>(value)
        .expect_err("hostile normalized program must fail")
        .to_string()
}

#[test]
fn normalized_program_is_strict_and_rejects_hostile_tables() {
    let program = fixture_program();
    let encoded = normalized_json();
    assert!(!contains_inline_slot(&encoded["root"]));
    assert_eq!(
        serde_json::from_value::<AuthoredStructuredProgram>(encoded.clone())
            .expect("strict normalized roundtrip"),
        program
    );

    let mut missing_slot = encoded.clone();
    missing_slot["lexical_slots"]
        .as_array_mut()
        .expect("slot table")
        .remove(0);
    assert!(decode_error(missing_slot).contains("not defined locally"));

    let mut duplicate_slot = encoded.clone();
    let duplicate = duplicate_slot["lexical_slots"][0].clone();
    duplicate_slot["lexical_slots"]
        .as_array_mut()
        .expect("slot table")
        .insert(0, duplicate);
    assert!(decode_error(duplicate_slot).contains("strict canonical order"));

    let mut reversed_slots = encoded.clone();
    reversed_slots["lexical_slots"]
        .as_array_mut()
        .expect("slot table")
        .reverse();
    assert!(decode_error(reversed_slots).contains("strict canonical order"));

    let mut missing_path = encoded.clone();
    missing_path["structural_paths"]
        .as_array_mut()
        .expect("path table")
        .clear();
    assert!(decode_error(missing_path).contains("not defined locally"));

    let mut duplicate_path = encoded.clone();
    let duplicate = duplicate_path["structural_paths"][0].clone();
    duplicate_path["structural_paths"]
        .as_array_mut()
        .expect("path table")
        .insert(0, duplicate);
    assert!(decode_error(duplicate_path).contains("strict canonical order"));

    let mut wrong_contract = encoded.clone();
    let admission = wrong_contract["lexical_slots"]
        .as_array_mut()
        .expect("slot table")
        .iter_mut()
        .find(|definition| definition["slot"]["producer"]["kind"] == "admission_root")
        .expect("admission slot");
    admission["slot"]["contract_ref"] =
        serde_json::to_value(fixture_ref("wrong-contract")).expect("foreign ref JSON");
    assert!(decode_error(wrong_contract).contains("reference does not match its definition"));

    let mut wrong_path = encoded.clone();
    let alias = wrong_path["lexical_slots"]
        .as_array_mut()
        .expect("slot table")
        .iter_mut()
        .find(|definition| definition["slot"]["producer"]["kind"] == "arm_value")
        .expect("alias slot");
    alias["slot"]["producer"]["selected_arm_path"]["path_ref"] =
        serde_json::to_value(fixture_ref("wrong-path")).expect("foreign ref JSON");
    assert!(decode_error(wrong_path).contains("reference does not match its definition"));

    let mut wrong_role = encoded.clone();
    let admission = wrong_role["lexical_slots"]
        .as_array_mut()
        .expect("slot table")
        .iter_mut()
        .find(|definition| definition["slot"]["producer"]["kind"] == "admission_root")
        .expect("admission slot");
    admission["slot"]["producer"] = serde_json::json!({
        "kind": "state_output",
        "occurrence_id": OccurrenceId::from_digest(sha256_digest_bytes(b"wrong-role")),
        "role": "typed_failure",
    });
    assert!(decode_error(wrong_role).contains("reference does not match its definition"));

    let mut wrong_schema = encoded.clone();
    wrong_schema["lexical_slots"][0]["slot_ref"] =
        serde_json::to_value(fixture_ref("wrong-slot-schema")).expect("foreign ref JSON");
    wrong_schema["lexical_slots"]
        .as_array_mut()
        .expect("slot table")
        .sort_by_key(|definition| {
            serde_json::from_value::<ContentRef>(definition["slot_ref"].clone())
                .expect("slot reference")
        });
    assert!(decode_error(wrong_schema).contains("reference does not match its definition"));

    let mut wrong_source = encoded.clone();
    let second_ref = wrong_source["input_root_slot_refs"][1]["slot_ref"].clone();
    let alias = wrong_source["lexical_slots"]
        .as_array_mut()
        .expect("slot table")
        .iter_mut()
        .find(|definition| definition["slot"]["producer"]["kind"] == "arm_value")
        .expect("alias slot");
    alias["slot"]["producer"]["source"]["slot_ref"] = second_ref;
    assert!(decode_error(wrong_source).contains("reference does not match its definition"));

    let mut unused = encoded;
    let mut unused_definition: ExpandedLexicalSlotDefinition = serde_json::from_value(
        unused["lexical_slots"]
            .as_array()
            .expect("slot table")
            .iter()
            .find(|definition| definition["slot"]["producer"]["kind"] == "admission_root")
            .expect("admission slot")
            .clone(),
    )
    .expect("slot definition schema");
    unused_definition.slot.producer = ExpandedLexicalProducer::AdmissionRoot {
        root_id: stable("unused"),
    };
    unused_definition.slot_ref = unused_definition
        .slot
        .content_ref()
        .expect("unused slot ref");
    let table = unused["lexical_slots"].as_array_mut().expect("slot table");
    table.push(serde_json::to_value(unused_definition).expect("unused definition JSON"));
    table.sort_by_key(|definition| {
        serde_json::from_value::<ContentRef>(definition["slot_ref"].clone())
            .expect("slot reference")
    });
    assert!(decode_error(unused).contains("unused definition"));
}

#[test]
fn cyclic_slot_graph_is_rejected_by_resolution() {
    let path = fixture_program().root.path;
    let path_ref = path.content_ref().expect("path ref");
    let first_ref = fixture_ref("cycle-first");
    let second_ref = fixture_ref("cycle-second");
    let contract_ref = fixture_ref("cycle-value");
    let first = ExpandedLexicalSlot {
        lexical_path_ref: path_ref.clone(),
        contract_ref: contract_ref.clone(),
        producer: ExpandedLexicalProducer::ArmValue {
            selected_arm_path: ExpandedPathRef {
                path_ref: path_ref.clone(),
            },
            source: ExpandedSlotRef {
                slot_ref: second_ref.clone(),
            },
        },
    };
    let second = ExpandedLexicalSlot {
        lexical_path_ref: path_ref.clone(),
        contract_ref,
        producer: ExpandedLexicalProducer::ArmValue {
            selected_arm_path: ExpandedPathRef {
                path_ref: path_ref.clone(),
            },
            source: ExpandedSlotRef {
                slot_ref: first_ref.clone(),
            },
        },
    };
    let mut denormalizer = StructuredProgramDenormalizer {
        structural_paths: BTreeMap::from([(path_ref, path)]),
        lexical_slots: BTreeMap::from([(first_ref.clone(), first), (second_ref, second)]),
        resolved_slots: BTreeMap::new(),
        active_slots: BTreeSet::new(),
        used_paths: BTreeSet::new(),
        used_slots: BTreeSet::new(),
    };
    assert!(denormalizer
        .resolve_slot(ExpandedSlotRef {
            slot_ref: first_ref,
        })
        .expect_err("cyclic source graph")
        .to_string()
        .contains("cycle"));
}

#[test]
fn expanded_program_rejects_authored_call_outputs() {
    let authored = fixture_program();
    let hostile_slot = LexicalSlot {
        lexical_path: authored.root.path.clone(),
        contract_ref: authored.output_contract_ref.clone(),
        producer: LexicalProducer::AuthoredCallOutput {
            semantic_call_id: SemanticCallId::from_digest(sha256_digest_bytes(b"authored-call")),
            role: ResultRole::SuccessOutput,
        },
    };
    let hostile = ExpandedStructuredProgram {
        operation_id: authored.operation_id.clone(),
        input_roots: authored.input_roots.clone(),
        output_contract_ref: authored.output_contract_ref.clone(),
        failure_contract: authored.failure_contract.clone(),
        root: ExpandedBlock {
            path: authored.root.path.clone(),
            failure_scope: authored.root.failure_scope.clone(),
            declarations: Vec::new(),
            failure_exits: Vec::new(),
            tail: BlockTail::Normal(hostile_slot.clone()),
        },
    };
    assert!(serde_json::to_value(hostile)
        .expect_err("expanded serialization must reject authored call outputs")
        .to_string()
        .contains("authored call output"));

    let mut hostile_authored = authored;
    hostile_authored.root.tail = BlockTail::Normal(hostile_slot);
    let hostile_json = serde_json::to_value(hostile_authored)
        .expect("authored call output is valid before expansion");
    assert!(
        serde_json::from_value::<ExpandedStructuredProgram>(hostile_json)
            .expect_err("expanded decoding must reject authored call outputs")
            .to_string()
            .contains("authored call output")
    );
}

fn contains_inline_slot(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Array(values) => values.iter().any(contains_inline_slot),
        serde_json::Value::Object(object) => {
            (object.contains_key("lexical_path")
                && object.contains_key("contract_ref")
                && object.contains_key("producer"))
                || object.values().any(contains_inline_slot)
        }
        _ => false,
    }
}
