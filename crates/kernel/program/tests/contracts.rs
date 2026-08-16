use mfm_capabilities::{CapabilityError, ReadCapabilityContract};
use mfm_ids::{
    ContentRef, DigestAlgorithm, DigestBytes, EntryPointId, SchemaVersion, SemanticTypeId, StableId,
};
use mfm_program::{
    capability_contract_ref, nominal_contract_ref, state_implementation_ref, Declaration,
    Execution, MatchDeclaration, MatchVariant, Never, Program, ProgramError, State,
    StateDeclaration,
};
use mfm_program_derive::MfmValue;
use mfm_values::{
    EnumTagging, EnumVariantDescriptor, FieldDescriptor, MfmValue as _, SchemaIdentity, SchemaKind,
    SchemaShape,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct Value {
    value: u64,
}

#[derive(Debug, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct OtherValue {
    value: String,
}

struct IdentityState;

struct LeftIdentityState;

impl State for LeftIdentityState {
    type Input = Value;
    type Output = Value;
    type Failure = Never;

    fn state_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.test.identity/left@1").map_err(|_| ProgramError::InvalidContract)
    }
}

struct RightIdentityState;

impl State for RightIdentityState {
    type Input = Value;
    type Output = Value;
    type Failure = Never;

    fn state_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.test.identity/right@1").map_err(|_| ProgramError::InvalidContract)
    }
}

impl State for IdentityState {
    type Input = Value;
    type Output = Value;
    type Failure = Never;

    fn state_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.test.identity/state@1").map_err(|_| ProgramError::InvalidContract)
    }
}

struct IdentityRead;

impl ReadCapabilityContract for IdentityRead {
    type Intent = Value;
    type Evidence = Value;

    fn contract_id() -> mfm_capabilities::Result<StableId> {
        StableId::new("mfm.test.identity/read@1").map_err(|_| CapabilityError::InvalidContract)
    }

    fn bind_evidence(
        intent: &Self::Intent,
        evidence: &Self::Evidence,
    ) -> mfm_capabilities::Result<()> {
        (intent.value == evidence.value)
            .then_some(())
            .ok_or(CapabilityError::EvidenceBinding)
    }
}

#[test]
fn never_identity_is_the_exact_reserved_root_exception() {
    let descriptor = Never::schema_descriptor().expect("Never descriptor");
    assert_eq!(
        Never::semantic_id().expect("semantic").as_str(),
        "semantic:mfm.kernel:never:1:sha256-jcs-v1:527c198ca1e6170ddf8966dd86ebd418b62226f14f6ef34d7b7ddd7b91a36835"
    );
    assert_eq!(
        descriptor.schema_id().expect("schema").as_str(),
        "schema:mfm.kernel.never:1:sha256-jcs-v1:00e01bfecb60f575bba12886041598ad758074b704806a8af2c036172c01a4d8"
    );
    assert_eq!(
        descriptor
            .identity_canonical_json()
            .expect("identity")
            .as_str(),
        "{\"canonicalization\":\"sha256-jcs-v1\",\"encoding\":{\"kind\":\"canonical_json\",\"shape\":{\"kind\":\"enum\",\"tagging\":{\"kind\":\"external\"},\"variants\":[]}},\"persisted_surface\":{\"numbers\":\"no_floats\",\"secrets\":\"no_secrets\"},\"schema_kind\":\"value\",\"schema_name\":\"mfm.kernel.never\",\"schema_version\":\"1\",\"semantic_type_id\":\"semantic:mfm.kernel:never:1:sha256-jcs-v1:527c198ca1e6170ddf8966dd86ebd418b62226f14f6ef34d7b7ddd7b91a36835\",\"versioning\":\"manual_version\"}"
    );
    assert_eq!(
        nominal_contract_ref::<Never>()
            .expect("nominal")
            .content_digest()
            .as_str(),
        "content:sha256-v1:804c7a33a2bc23a692444fcc2833f71d96f8315e6529523a7f884e13e6559927"
    );
    for value in [
        b"null".as_slice(),
        b"true".as_slice(),
        b"7".as_slice(),
        b"[]".as_slice(),
        b"{}".as_slice(),
        b"\"never\"".as_slice(),
    ] {
        assert!(descriptor
            .identity()
            .validate_canonical_value(value)
            .is_err());
    }
}

#[test]
fn foreign_and_nested_empty_enums_remain_invalid() {
    let reserved_never_with_variant = SchemaIdentity::new(
        SchemaKind::Value,
        Some(Never::semantic_id().expect("Never semantic")),
        "mfm.kernel.never",
        SchemaVersion::new("1").expect("version"),
        SchemaShape::Enum {
            tagging: EnumTagging::External,
            variants: vec![EnumVariantDescriptor::new("impossible", SchemaShape::Unit)],
        },
    );
    assert!(reserved_never_with_variant.is_err());

    let semantic = SemanticTypeId::new(
        "mfm.test",
        "foreign-never",
        "1",
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([3; 32]),
    )
    .expect("semantic");
    let foreign = SchemaIdentity::new(
        SchemaKind::Value,
        Some(semantic.clone()),
        "mfm.test.foreign-never",
        SchemaVersion::new("1").expect("version"),
        SchemaShape::Enum {
            tagging: EnumTagging::External,
            variants: Vec::new(),
        },
    );
    assert!(foreign.is_err());

    let nested = SchemaIdentity::new(
        SchemaKind::Value,
        Some(semantic),
        "mfm.test.nested-never",
        SchemaVersion::new("1").expect("version"),
        SchemaShape::named_struct(vec![FieldDescriptor::required(
            "nested",
            SchemaShape::Enum {
                tagging: EnumTagging::External,
                variants: Vec::new(),
            },
        )])
        .expect("struct shape"),
    );
    assert!(nested.is_err());
}

#[test]
fn implementation_reference_preimages_remain_v1_exact() {
    assert_eq!(
        state_implementation_ref::<IdentityState>()
            .expect("state ref")
            .content_digest()
            .as_str(),
        "content:sha256-v1:fab16c95f74ec061a9482105a20937043dd820e983a35ae63d235d69e53c540a"
    );
    assert_eq!(
        capability_contract_ref::<IdentityRead>()
            .expect("capability ref")
            .content_digest()
            .as_str(),
        "content:sha256-v1:70c931a7ae3e52ce943296347e6a3f0ab04425cfb8a390428145ff34753ced54"
    );
}

fn state_program() -> Program {
    let value = nominal_contract_ref::<Value>().expect("value contract");
    let never = nominal_contract_ref::<Never>().expect("never contract");
    let state = StateDeclaration::new(
        state_implementation_ref::<IdentityState>().expect("state"),
        value.clone(),
        value.clone(),
        never.clone(),
        Execution::pure(),
        None,
        None,
    )
    .expect("declaration");
    Program::new(
        EntryPointId::new("mfm.test/program@1").expect("entry"),
        value.clone(),
        value.clone(),
        never.clone(),
        vec![Declaration::State(state)],
    )
    .expect("program")
}

fn read_match_program() -> Program {
    let value = nominal_contract_ref::<Value>().expect("value contract");
    let never = nominal_contract_ref::<Never>().expect("never contract");
    let read = StateDeclaration::new(
        state_implementation_ref::<IdentityState>().expect("read state"),
        value.clone(),
        value.clone(),
        never.clone(),
        Execution::read(
            capability_contract_ref::<IdentityRead>().expect("capability"),
            value.clone(),
            value.clone(),
            value.clone(),
        ),
        Some(1),
        None,
    )
    .expect("read declaration");
    let selector = MatchDeclaration::new(
        value.clone(),
        vec![
            MatchVariant::new(StableId::new("right").expect("right"), 3),
            MatchVariant::new(StableId::new("left").expect("left"), 2),
        ],
    )
    .expect("match declaration");
    let terminal = |implementation| {
        Declaration::State(
            StateDeclaration::new(
                implementation,
                value.clone(),
                value.clone(),
                never.clone(),
                Execution::pure(),
                None,
                None,
            )
            .expect("terminal"),
        )
    };
    Program::new(
        EntryPointId::new("mfm.test/read-match-program@1").expect("entry"),
        value.clone(),
        value.clone(),
        never.clone(),
        vec![
            Declaration::State(read),
            Declaration::Match(selector),
            terminal(state_implementation_ref::<LeftIdentityState>().expect("left state")),
            terminal(state_implementation_ref::<RightIdentityState>().expect("right state")),
        ],
    )
    .expect("read Match program")
}

#[test]
fn read_and_match_program_v2_wire_is_exact() {
    let program = read_match_program();
    assert_eq!(
        std::str::from_utf8(program.canonical_bytes()).expect("utf8"),
        r#"{"admitted_context_contract_ref":{"content_digest":"content:sha256-v1:804c7a33a2bc23a692444fcc2833f71d96f8315e6529523a7f884e13e6559927","schema_id":"schema:mfm.derived.value:1:sha256-jcs-v1:e48ceec83bc342688038e9ad63e1720d80fe62978f7b9af0a4d7bc53628f4add"},"declarations":[{"kind":"state","value":{"execution":{"binding_ref":{"content_digest":"content:sha256-v1:804c7a33a2bc23a692444fcc2833f71d96f8315e6529523a7f884e13e6559927","schema_id":"schema:mfm.derived.value:1:sha256-jcs-v1:e48ceec83bc342688038e9ad63e1720d80fe62978f7b9af0a4d7bc53628f4add"},"capability_contract_ref":{"content_digest":"content:sha256-v1:70c931a7ae3e52ce943296347e6a3f0ab04425cfb8a390428145ff34753ced54","schema_id":"schema:mfm.capability-contract:1:sha256-jcs-v1:0000000000000000000000000000000000000000000000000000000000000000"},"evidence_contract_ref":{"content_digest":"content:sha256-v1:804c7a33a2bc23a692444fcc2833f71d96f8315e6529523a7f884e13e6559927","schema_id":"schema:mfm.derived.value:1:sha256-jcs-v1:e48ceec83bc342688038e9ad63e1720d80fe62978f7b9af0a4d7bc53628f4add"},"intent_contract_ref":{"content_digest":"content:sha256-v1:804c7a33a2bc23a692444fcc2833f71d96f8315e6529523a7f884e13e6559927","schema_id":"schema:mfm.derived.value:1:sha256-jcs-v1:e48ceec83bc342688038e9ad63e1720d80fe62978f7b9af0a4d7bc53628f4add"},"kind":"read"},"failure_contract_ref":{"content_digest":"content:sha256-v1:804c7a33a2bc23a692444fcc2833f71d96f8315e6529523a7f884e13e6559927","schema_id":"schema:mfm.kernel.never:1:sha256-jcs-v1:00e01bfecb60f575bba12886041598ad758074b704806a8af2c036172c01a4d8"},"failure_next_index":null,"input_contract_ref":{"content_digest":"content:sha256-v1:804c7a33a2bc23a692444fcc2833f71d96f8315e6529523a7f884e13e6559927","schema_id":"schema:mfm.derived.value:1:sha256-jcs-v1:e48ceec83bc342688038e9ad63e1720d80fe62978f7b9af0a4d7bc53628f4add"},"next_index":1,"output_contract_ref":{"content_digest":"content:sha256-v1:804c7a33a2bc23a692444fcc2833f71d96f8315e6529523a7f884e13e6559927","schema_id":"schema:mfm.derived.value:1:sha256-jcs-v1:e48ceec83bc342688038e9ad63e1720d80fe62978f7b9af0a4d7bc53628f4add"},"state_implementation_ref":{"content_digest":"content:sha256-v1:fab16c95f74ec061a9482105a20937043dd820e983a35ae63d235d69e53c540a","schema_id":"schema:mfm.state-implementation:1:sha256-jcs-v1:0000000000000000000000000000000000000000000000000000000000000000"}}},{"kind":"match","value":{"selector_contract_ref":{"content_digest":"content:sha256-v1:804c7a33a2bc23a692444fcc2833f71d96f8315e6529523a7f884e13e6559927","schema_id":"schema:mfm.derived.value:1:sha256-jcs-v1:e48ceec83bc342688038e9ad63e1720d80fe62978f7b9af0a4d7bc53628f4add"},"variants":[{"entry_index":2,"tag":"left"},{"entry_index":3,"tag":"right"}]}},{"kind":"state","value":{"execution":{"kind":"pure"},"failure_contract_ref":{"content_digest":"content:sha256-v1:804c7a33a2bc23a692444fcc2833f71d96f8315e6529523a7f884e13e6559927","schema_id":"schema:mfm.kernel.never:1:sha256-jcs-v1:00e01bfecb60f575bba12886041598ad758074b704806a8af2c036172c01a4d8"},"failure_next_index":null,"input_contract_ref":{"content_digest":"content:sha256-v1:804c7a33a2bc23a692444fcc2833f71d96f8315e6529523a7f884e13e6559927","schema_id":"schema:mfm.derived.value:1:sha256-jcs-v1:e48ceec83bc342688038e9ad63e1720d80fe62978f7b9af0a4d7bc53628f4add"},"next_index":null,"output_contract_ref":{"content_digest":"content:sha256-v1:804c7a33a2bc23a692444fcc2833f71d96f8315e6529523a7f884e13e6559927","schema_id":"schema:mfm.derived.value:1:sha256-jcs-v1:e48ceec83bc342688038e9ad63e1720d80fe62978f7b9af0a4d7bc53628f4add"},"state_implementation_ref":{"content_digest":"content:sha256-v1:882a30a8a3e07f3171a8675eb01814cce42412690088fdc5348a335ee1d18fc5","schema_id":"schema:mfm.state-implementation:1:sha256-jcs-v1:0000000000000000000000000000000000000000000000000000000000000000"}}},{"kind":"state","value":{"execution":{"kind":"pure"},"failure_contract_ref":{"content_digest":"content:sha256-v1:804c7a33a2bc23a692444fcc2833f71d96f8315e6529523a7f884e13e6559927","schema_id":"schema:mfm.kernel.never:1:sha256-jcs-v1:00e01bfecb60f575bba12886041598ad758074b704806a8af2c036172c01a4d8"},"failure_next_index":null,"input_contract_ref":{"content_digest":"content:sha256-v1:804c7a33a2bc23a692444fcc2833f71d96f8315e6529523a7f884e13e6559927","schema_id":"schema:mfm.derived.value:1:sha256-jcs-v1:e48ceec83bc342688038e9ad63e1720d80fe62978f7b9af0a4d7bc53628f4add"},"next_index":null,"output_contract_ref":{"content_digest":"content:sha256-v1:804c7a33a2bc23a692444fcc2833f71d96f8315e6529523a7f884e13e6559927","schema_id":"schema:mfm.derived.value:1:sha256-jcs-v1:e48ceec83bc342688038e9ad63e1720d80fe62978f7b9af0a4d7bc53628f4add"},"state_implementation_ref":{"content_digest":"content:sha256-v1:890cdf6de49a4abb8e12671ecd4bf8c6d28e028adc2a150e14dc4cd69c752737","schema_id":"schema:mfm.state-implementation:1:sha256-jcs-v1:0000000000000000000000000000000000000000000000000000000000000000"}}}],"entry_point_id":"mfm.test/read-match-program@1","root_failure_contract_ref":{"content_digest":"content:sha256-v1:804c7a33a2bc23a692444fcc2833f71d96f8315e6529523a7f884e13e6559927","schema_id":"schema:mfm.kernel.never:1:sha256-jcs-v1:00e01bfecb60f575bba12886041598ad758074b704806a8af2c036172c01a4d8"},"root_success_contract_ref":{"content_digest":"content:sha256-v1:804c7a33a2bc23a692444fcc2833f71d96f8315e6529523a7f884e13e6559927","schema_id":"schema:mfm.derived.value:1:sha256-jcs-v1:e48ceec83bc342688038e9ad63e1720d80fe62978f7b9af0a4d7bc53628f4add"}}"#
    );
    assert_eq!(
        program.content_ref().schema_id().as_str(),
        "schema:mfm-program-document:2:sha256-jcs-v1:fc3c33ba3470e25a166df52597c4b45a723dd6d0823d1318938c29225323076c"
    );
    assert_eq!(
        program.content_ref().content_digest().as_str(),
        "content:sha256-v1:82aa393ce2010c625726fd23faba701844f7164c4c3798425cdc8b6bd3ef2940"
    );

    let Declaration::State(read) = &program.declarations()[0] else {
        panic!("read declaration");
    };
    let value = nominal_contract_ref::<Value>().expect("value contract");
    assert_eq!(
        read.execution().capability_contract_ref(),
        Some(&capability_contract_ref::<IdentityRead>().expect("capability"))
    );
    assert_eq!(read.execution().intent_contract_ref(), Some(&value));
    assert_eq!(read.execution().evidence_contract_ref(), Some(&value));
    assert_eq!(read.execution().binding_ref(), Some(&value));
    let Declaration::Match(selector) = &program.declarations()[1] else {
        panic!("Match declaration");
    };
    assert_eq!(selector.selector_contract_ref(), &value);
    assert_eq!(selector.variants()[0].tag().as_str(), "left");
    assert_eq!(selector.variants()[0].entry_index(), 2);
    assert_eq!(selector.variants()[1].tag().as_str(), "right");
    assert_eq!(selector.variants()[1].entry_index(), 3);

    let decoded = Program::decode_canonical(program.canonical_bytes()).expect("decode");
    assert_eq!(decoded.canonical_bytes(), program.canonical_bytes());
    assert_eq!(decoded.content_ref(), program.content_ref());
}

#[test]
fn program_v2_round_trips_and_rejects_hostile_wire() {
    let program = state_program();
    assert_eq!(
        std::str::from_utf8(program.canonical_bytes()).expect("utf8"),
        r#"{"admitted_context_contract_ref":{"content_digest":"content:sha256-v1:804c7a33a2bc23a692444fcc2833f71d96f8315e6529523a7f884e13e6559927","schema_id":"schema:mfm.derived.value:1:sha256-jcs-v1:e48ceec83bc342688038e9ad63e1720d80fe62978f7b9af0a4d7bc53628f4add"},"declarations":[{"kind":"state","value":{"execution":{"kind":"pure"},"failure_contract_ref":{"content_digest":"content:sha256-v1:804c7a33a2bc23a692444fcc2833f71d96f8315e6529523a7f884e13e6559927","schema_id":"schema:mfm.kernel.never:1:sha256-jcs-v1:00e01bfecb60f575bba12886041598ad758074b704806a8af2c036172c01a4d8"},"failure_next_index":null,"input_contract_ref":{"content_digest":"content:sha256-v1:804c7a33a2bc23a692444fcc2833f71d96f8315e6529523a7f884e13e6559927","schema_id":"schema:mfm.derived.value:1:sha256-jcs-v1:e48ceec83bc342688038e9ad63e1720d80fe62978f7b9af0a4d7bc53628f4add"},"next_index":null,"output_contract_ref":{"content_digest":"content:sha256-v1:804c7a33a2bc23a692444fcc2833f71d96f8315e6529523a7f884e13e6559927","schema_id":"schema:mfm.derived.value:1:sha256-jcs-v1:e48ceec83bc342688038e9ad63e1720d80fe62978f7b9af0a4d7bc53628f4add"},"state_implementation_ref":{"content_digest":"content:sha256-v1:fab16c95f74ec061a9482105a20937043dd820e983a35ae63d235d69e53c540a","schema_id":"schema:mfm.state-implementation:1:sha256-jcs-v1:0000000000000000000000000000000000000000000000000000000000000000"}}}],"entry_point_id":"mfm.test/program@1","root_failure_contract_ref":{"content_digest":"content:sha256-v1:804c7a33a2bc23a692444fcc2833f71d96f8315e6529523a7f884e13e6559927","schema_id":"schema:mfm.kernel.never:1:sha256-jcs-v1:00e01bfecb60f575bba12886041598ad758074b704806a8af2c036172c01a4d8"},"root_success_contract_ref":{"content_digest":"content:sha256-v1:804c7a33a2bc23a692444fcc2833f71d96f8315e6529523a7f884e13e6559927","schema_id":"schema:mfm.derived.value:1:sha256-jcs-v1:e48ceec83bc342688038e9ad63e1720d80fe62978f7b9af0a4d7bc53628f4add"}}"#
    );
    assert_eq!(
        program.content_ref().schema_id().as_str(),
        "schema:mfm-program-document:2:sha256-jcs-v1:fc3c33ba3470e25a166df52597c4b45a723dd6d0823d1318938c29225323076c"
    );
    assert_eq!(
        program.content_ref().content_digest().as_str(),
        "content:sha256-v1:e7faf0b0db6284824c9d75baf9cd237a43fdf3627ac71f0ea5be6d958d4d035f"
    );
    let decoded = Program::decode_canonical(program.canonical_bytes()).expect("decode");
    assert_eq!(decoded.content_ref(), program.content_ref());
    assert_eq!(decoded.canonical_bytes(), program.canonical_bytes());

    let mut unknown: serde_json::Value =
        serde_json::from_slice(program.canonical_bytes()).expect("json");
    unknown
        .as_object_mut()
        .expect("object")
        .insert("version".to_owned(), serde_json::json!(1));
    let unknown = mfm_canonical::PlainCanonicalJsonBytes::from_json_str(
        &serde_json::to_string(&unknown).expect("json"),
    )
    .expect("canonical");
    assert_eq!(
        Program::decode_canonical(unknown.as_bytes()),
        Err(ProgramError::Canonical)
    );

    let mut noncanonical = program.canonical_bytes().to_vec();
    noncanonical.push(b' ');
    assert_eq!(
        Program::decode_canonical(&noncanonical),
        Err(ProgramError::Canonical)
    );

    let raw: serde_json::Value =
        serde_json::from_slice(program.canonical_bytes()).expect("program json");
    for field in ["next_index", "failure_next_index"] {
        let mut omitted = raw.clone();
        omitted["declarations"][0]["value"]
            .as_object_mut()
            .expect("state")
            .remove(field);
        assert_eq!(
            Program::decode_canonical(&canonical_json(&omitted)),
            Err(ProgramError::Canonical),
            "missing mandatory nullable field {field}"
        );
    }
    let mut overflow = raw.clone();
    overflow["declarations"][0]["value"]["next_index"] = serde_json::json!(65_536_u64);
    assert_eq!(
        Program::decode_canonical(&canonical_json(&overflow)),
        Err(ProgramError::Canonical)
    );
    let mut nested_unknown = raw;
    nested_unknown["declarations"][0]["value"]["execution"]["legacy"] = serde_json::json!(true);
    assert_eq!(
        Program::decode_canonical(&canonical_json(&nested_unknown)),
        Err(ProgramError::Canonical)
    );

    for retired_tag in ["state_prepared", "state_concluded_access"] {
        let mut retired: serde_json::Value =
            serde_json::from_slice(program.canonical_bytes()).expect("program json");
        retired["declarations"][0]["kind"] = serde_json::json!(retired_tag);
        assert_eq!(
            Program::decode_canonical(&canonical_json(&retired)),
            Err(ProgramError::Canonical),
            "retired v1 tag {retired_tag}"
        );
    }
    for retired_field in ["occurrence", "preparation"] {
        let mut retired: serde_json::Value =
            serde_json::from_slice(program.canonical_bytes()).expect("program json");
        retired["declarations"][0]["value"][retired_field] = serde_json::json!({});
        assert_eq!(
            Program::decode_canonical(&canonical_json(&retired)),
            Err(ProgramError::Canonical),
            "retired v1 field {retired_field}"
        );
    }
}

fn canonical_json(value: &serde_json::Value) -> Vec<u8> {
    mfm_canonical::PlainCanonicalJsonBytes::from_json_str(
        &serde_json::to_string(value).expect("json"),
    )
    .expect("canonical")
    .to_vec()
}

fn state(
    input: ContentRef,
    output: ContentRef,
    failure: ContentRef,
    next: Option<u16>,
    failure_next: Option<u16>,
) -> Declaration {
    Declaration::State(
        StateDeclaration::new(
            state_implementation_ref::<IdentityState>().expect("state"),
            input,
            output,
            failure,
            Execution::pure(),
            next,
            failure_next,
        )
        .expect("declaration"),
    )
}

#[test]
fn match_branch_rejoin_common_failure_and_tag_order_are_checked() {
    let value = nominal_contract_ref::<Value>().expect("value");
    let never = nominal_contract_ref::<Never>().expect("never");
    let selector = MatchDeclaration::new(
        value.clone(),
        vec![
            MatchVariant::new(StableId::new("z-branch").expect("tag"), 2),
            MatchVariant::new(StableId::new("a.branch").expect("tag"), 1),
        ],
    )
    .expect("selector");
    assert_eq!(selector.variants()[0].tag().as_str(), "a.branch");
    assert_eq!(selector.variants()[1].tag().as_str(), "z-branch");

    let byte_order = MatchDeclaration::new(
        value.clone(),
        ["a0", "a.0", "a-0", "a"]
            .into_iter()
            .enumerate()
            .map(|(index, tag)| {
                MatchVariant::new(
                    StableId::new(tag).expect("raw byte tag"),
                    u16::try_from(index + 1).expect("index"),
                )
            })
            .collect(),
    )
    .expect("raw byte ordering");
    assert_eq!(
        byte_order
            .variants()
            .iter()
            .map(|variant| variant.tag().as_str())
            .collect::<Vec<_>>(),
        ["a", "a-0", "a.0", "a0"]
    );

    let program = Program::new(
        EntryPointId::new("mfm.test/branch-rejoin@1").expect("entry"),
        value.clone(),
        value.clone(),
        value.clone(),
        vec![
            Declaration::Match(selector),
            state(
                value.clone(),
                value.clone(),
                value.clone(),
                Some(3),
                Some(4),
            ),
            state(
                value.clone(),
                value.clone(),
                value.clone(),
                Some(3),
                Some(4),
            ),
            state(value.clone(), value.clone(), never.clone(), None, None),
            state(value.clone(), value.clone(), never, None, None),
        ],
    )
    .expect("branch/rejoin program");
    assert_eq!(
        Program::decode_canonical(program.canonical_bytes())
            .expect("round trip")
            .content_ref(),
        program.content_ref()
    );

    let mut hostile_order: serde_json::Value =
        serde_json::from_slice(program.canonical_bytes()).expect("program json");
    hostile_order["declarations"][0]["value"]["variants"]
        .as_array_mut()
        .expect("variants")
        .swap(0, 1);
    assert_eq!(
        Program::decode_canonical(&canonical_json(&hostile_order)),
        Err(ProgramError::InvalidContract)
    );

    assert_eq!(
        MatchDeclaration::new(
            value.clone(),
            vec![
                MatchVariant::new(StableId::new("same").expect("tag"), 1),
                MatchVariant::new(StableId::new("same").expect("tag"), 2),
            ],
        ),
        Err(ProgramError::InvalidContract)
    );
    let too_many = (0..257)
        .map(|index| MatchVariant::new(StableId::new(format!("tag-{index:03}")).expect("tag"), 1))
        .collect();
    assert_eq!(
        MatchDeclaration::new(value, too_many),
        Err(ProgramError::Capacity)
    );
}

#[test]
fn program_bounds_and_edge_targets_are_exact() {
    let value = nominal_contract_ref::<Value>().expect("value");
    let never = nominal_contract_ref::<Never>().expect("never");
    let out_of_bounds = state(value.clone(), value.clone(), never.clone(), Some(1), None);
    assert_eq!(
        Program::new(
            EntryPointId::new("mfm.test/out-of-bounds@1").expect("entry"),
            value.clone(),
            value.clone(),
            never.clone(),
            vec![out_of_bounds],
        ),
        Err(ProgramError::InvalidContract)
    );
    assert!(StateDeclaration::new(
        state_implementation_ref::<IdentityState>().expect("state"),
        value.clone(),
        value.clone(),
        never.clone(),
        Execution::pure(),
        Some(u16::MAX),
        None,
    )
    .is_ok());
    assert_eq!(
        Program::decode_canonical(&vec![b' '; 8_388_609]),
        Err(ProgramError::Capacity)
    );

    let match_declaration = Declaration::Match(
        MatchDeclaration::new(
            value.clone(),
            vec![MatchVariant::new(StableId::new("only").expect("tag"), 1)],
        )
        .expect("match"),
    );
    assert_eq!(
        Program::new(
            EntryPointId::new("mfm.test/declaration-capacity@1").expect("entry"),
            value.clone(),
            value.clone(),
            never,
            vec![match_declaration; 65_537],
        ),
        Err(ProgramError::Capacity)
    );

    let over_state_limit = vec![
        state(
            value.clone(),
            value.clone(),
            nominal_contract_ref::<Never>().expect("never"),
            None,
            None,
        );
        65_536
    ];
    assert_eq!(
        Program::new(
            EntryPointId::new("mfm.test/state-capacity@1").expect("entry"),
            value.clone(),
            value.clone(),
            nominal_contract_ref::<Never>().expect("never"),
            over_state_limit,
        ),
        Err(ProgramError::Capacity)
    );

    let implementation = state_implementation_ref::<IdentityState>().expect("implementation");
    let mut oversized = Vec::with_capacity(12_000);
    for index in 0..12_000_u16 {
        oversized.push(Declaration::State(
            StateDeclaration::new(
                implementation.clone(),
                value.clone(),
                value.clone(),
                nominal_contract_ref::<Never>().expect("never"),
                Execution::pure(),
                (index < 11_999).then_some(index + 1),
                None,
            )
            .expect("oversized declaration"),
        ));
    }
    assert_eq!(
        Program::new(
            EntryPointId::new("mfm.test/encoded-capacity@1").expect("entry"),
            value.clone(),
            value.clone(),
            nominal_contract_ref::<Never>().expect("never"),
            oversized,
        ),
        Err(ProgramError::Capacity)
    );
}

#[test]
fn program_rejects_back_edges_and_unreachable_declarations() {
    let value = nominal_contract_ref::<Value>().expect("value");
    let never = nominal_contract_ref::<Never>().expect("never");
    let implementation = state_implementation_ref::<IdentityState>().expect("implementation");
    let back_edge = StateDeclaration::new(
        implementation.clone(),
        value.clone(),
        value.clone(),
        never.clone(),
        Execution::pure(),
        Some(0),
        None,
    )
    .expect("local declaration");
    assert_eq!(
        Program::new(
            EntryPointId::new("mfm.test/back-edge@1").expect("entry"),
            value.clone(),
            value.clone(),
            never.clone(),
            vec![Declaration::State(back_edge)],
        ),
        Err(ProgramError::InvalidContract)
    );

    let forward = StateDeclaration::new(
        implementation.clone(),
        value.clone(),
        value.clone(),
        never.clone(),
        Execution::pure(),
        Some(1),
        None,
    )
    .expect("forward declaration");
    let later_back_edge = StateDeclaration::new(
        implementation.clone(),
        value.clone(),
        value.clone(),
        never.clone(),
        Execution::pure(),
        Some(0),
        None,
    )
    .expect("later declaration");
    assert_eq!(
        Program::new(
            EntryPointId::new("mfm.test/later-back-edge@1").expect("entry"),
            value.clone(),
            value.clone(),
            never.clone(),
            vec![
                Declaration::State(forward),
                Declaration::State(later_back_edge),
            ],
        ),
        Err(ProgramError::InvalidContract)
    );

    let terminal = || {
        StateDeclaration::new(
            implementation.clone(),
            value.clone(),
            value.clone(),
            never.clone(),
            Execution::pure(),
            None,
            None,
        )
        .expect("terminal")
    };
    assert_eq!(
        Program::new(
            EntryPointId::new("mfm.test/unreachable@1").expect("entry"),
            value.clone(),
            value.clone(),
            never.clone(),
            vec![
                Declaration::State(terminal()),
                Declaration::State(terminal()),
            ],
        ),
        Err(ProgramError::InvalidContract)
    );
}

#[test]
fn declaration_sibling_order_is_identity_bearing() {
    let value = nominal_contract_ref::<Value>().expect("value");
    let never = nominal_contract_ref::<Never>().expect("never");
    let build = |reversed: bool| {
        let (first, second, first_target, second_target) = if reversed {
            (
                state_implementation_ref::<RightIdentityState>().expect("right"),
                state_implementation_ref::<LeftIdentityState>().expect("left"),
                2,
                1,
            )
        } else {
            (
                state_implementation_ref::<LeftIdentityState>().expect("left"),
                state_implementation_ref::<RightIdentityState>().expect("right"),
                1,
                2,
            )
        };
        let terminal = |implementation| {
            Declaration::State(
                StateDeclaration::new(
                    implementation,
                    value.clone(),
                    value.clone(),
                    never.clone(),
                    Execution::pure(),
                    None,
                    None,
                )
                .expect("terminal"),
            )
        };
        Program::new(
            EntryPointId::new("mfm.test/sibling-order@1").expect("entry"),
            value.clone(),
            value.clone(),
            never.clone(),
            vec![
                Declaration::Match(
                    MatchDeclaration::new(
                        value.clone(),
                        vec![
                            MatchVariant::new(StableId::new("left").expect("tag"), first_target),
                            MatchVariant::new(StableId::new("right").expect("tag"), second_target),
                        ],
                    )
                    .expect("match"),
                ),
                terminal(first),
                terminal(second),
            ],
        )
        .expect("valid sibling order")
    };
    let authored = build(false);
    let reversed = build(true);
    assert_ne!(authored.canonical_bytes(), reversed.canonical_bytes());
    assert_ne!(authored.content_ref(), reversed.content_ref());
}

#[test]
fn empty_program_is_only_identity_success_with_never_failure() {
    let value: ContentRef = nominal_contract_ref::<Value>().expect("value");
    let never = nominal_contract_ref::<Never>().expect("never");
    assert!(Program::new(
        EntryPointId::new("mfm.test/empty@1").expect("entry"),
        value.clone(),
        value.clone(),
        never,
        Vec::new(),
    )
    .is_ok());
    assert_eq!(
        Program::new(
            EntryPointId::new("mfm.test/empty-invalid@1").expect("entry"),
            value.clone(),
            value.clone(),
            value,
            Vec::new(),
        ),
        Err(ProgramError::InvalidContract)
    );
}

#[test]
fn graph_contract_mismatches_and_never_placements_are_rejected() {
    let value = nominal_contract_ref::<Value>().expect("value");
    let other = nominal_contract_ref::<OtherValue>().expect("other");
    let never = nominal_contract_ref::<Never>().expect("never");
    let entry =
        |suffix: &str| EntryPointId::new(format!("mfm.test/{suffix}@1")).expect("entry point");
    let invalid = |suffix: &str,
                   admitted: ContentRef,
                   success: ContentRef,
                   failure: ContentRef,
                   declarations: Vec<Declaration>| {
        assert_eq!(
            Program::new(entry(suffix), admitted, success, failure, declarations),
            Err(ProgramError::InvalidContract),
            "{suffix}"
        );
    };

    invalid(
        "match-targets-match",
        value.clone(),
        value.clone(),
        never.clone(),
        vec![
            Declaration::Match(
                MatchDeclaration::new(
                    value.clone(),
                    vec![MatchVariant::new(StableId::new("outer").expect("tag"), 1)],
                )
                .expect("outer match"),
            ),
            Declaration::Match(
                MatchDeclaration::new(
                    value.clone(),
                    vec![MatchVariant::new(StableId::new("inner").expect("tag"), 2)],
                )
                .expect("inner match"),
            ),
            state(value.clone(), value.clone(), never.clone(), None, None),
        ],
    );
    invalid(
        "success-contract-mismatch",
        value.clone(),
        other.clone(),
        never.clone(),
        vec![
            state(value.clone(), value.clone(), never.clone(), Some(1), None),
            state(other.clone(), other.clone(), never.clone(), None, None),
        ],
    );
    invalid(
        "failure-contract-mismatch",
        value.clone(),
        value.clone(),
        never.clone(),
        vec![
            state(
                value.clone(),
                value.clone(),
                value.clone(),
                Some(2),
                Some(1),
            ),
            state(other.clone(), value.clone(), never.clone(), None, None),
            state(value.clone(), value.clone(), never.clone(), None, None),
        ],
    );
    invalid(
        "terminal-success-mismatch",
        value.clone(),
        other.clone(),
        never.clone(),
        vec![state(
            value.clone(),
            value.clone(),
            never.clone(),
            None,
            None,
        )],
    );
    invalid(
        "terminal-failure-mismatch",
        value.clone(),
        value.clone(),
        other.clone(),
        vec![state(
            value.clone(),
            value.clone(),
            value.clone(),
            None,
            None,
        )],
    );
    invalid(
        "failure-targets-match",
        value.clone(),
        value.clone(),
        never.clone(),
        vec![
            state(
                value.clone(),
                value.clone(),
                value.clone(),
                Some(2),
                Some(1),
            ),
            Declaration::Match(
                MatchDeclaration::new(
                    value.clone(),
                    vec![MatchVariant::new(StableId::new("route").expect("tag"), 2)],
                )
                .expect("match"),
            ),
            state(value.clone(), value.clone(), never.clone(), None, None),
        ],
    );

    invalid(
        "never-admitted",
        never.clone(),
        value.clone(),
        never.clone(),
        Vec::new(),
    );
    invalid(
        "never-root-success",
        value.clone(),
        never.clone(),
        never.clone(),
        Vec::new(),
    );
    invalid(
        "never-input",
        never.clone(),
        value.clone(),
        never.clone(),
        vec![state(
            never.clone(),
            value.clone(),
            never.clone(),
            None,
            None,
        )],
    );
    invalid(
        "never-output",
        value.clone(),
        value.clone(),
        never.clone(),
        vec![state(
            value.clone(),
            never.clone(),
            never.clone(),
            None,
            None,
        )],
    );
    invalid(
        "never-selector",
        never.clone(),
        value.clone(),
        never.clone(),
        vec![Declaration::Match(
            MatchDeclaration::new(
                never.clone(),
                vec![MatchVariant::new(StableId::new("never").expect("tag"), 1)],
            )
            .expect("match"),
        )],
    );
    invalid(
        "never-failure-successor",
        value.clone(),
        value.clone(),
        never.clone(),
        vec![
            state(
                value.clone(),
                value.clone(),
                never.clone(),
                Some(1),
                Some(1),
            ),
            state(value.clone(), value, never.clone(), None, None),
        ],
    );
}

#[test]
fn removed_program_lifecycle_does_not_compile() {
    trybuild::TestCases::new().compile_fail("tests/ui/removed_program_api.rs");
}
