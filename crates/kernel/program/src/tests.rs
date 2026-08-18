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

struct InvalidIdentityState;

impl State for InvalidIdentityState {
    type Input = Value;
    type Output = Value;
    type Failure = Never;

    fn state_id() -> mfm_program::Result<StableId> {
        Err(ProgramError::InvalidContract)
    }
}

struct InvalidIdentityRead;

impl ReadCapabilityContract for InvalidIdentityRead {
    type Intent = Value;
    type Evidence = Value;

    fn contract_id() -> mfm_capabilities::Result<StableId> {
        Err(CapabilityError::InvalidContract)
    }

    fn bind_evidence(
        _intent: &Self::Intent,
        _evidence: &Self::Evidence,
    ) -> mfm_capabilities::Result<()> {
        Ok(())
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

#[test]
fn static_state_and_capability_identity_failures_map_at_program_authoring() {
    assert_eq!(
        state_implementation_ref::<InvalidIdentityState>(),
        Err(ProgramError::InvalidContract)
    );
    assert_eq!(
        capability_contract_ref::<InvalidIdentityRead>(),
        Err(ProgramError::InvalidContract)
    );
}

fn state(
    input: ContentRef,
    output: ContentRef,
    failure: ContentRef,
    next: Option<u16>,
    failure_next: Option<u16>,
) -> Declaration {
    Declaration::State(StateDeclaration::new(
        state_implementation_ref::<IdentityState>().expect("state"),
        input,
        output,
        failure,
        Execution::pure(),
        next,
        failure_next,
    ))
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

    let _program = Program::new(
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
        MatchDeclaration::new(
            value.clone(),
            vec![
                MatchVariant::new(StableId::new("same").expect("tag"), 1),
                MatchVariant::new(StableId::new("same").expect("tag"), 2),
            ],
        ),
        Err(ProgramError::InvalidContract)
    );
    let maximum = (0..256)
        .map(|index| MatchVariant::new(StableId::new(format!("tag-{index:03}")).expect("tag"), 1))
        .collect();
    assert!(MatchDeclaration::new(value.clone(), maximum).is_ok());
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
    let _ = StateDeclaration::new(
        state_implementation_ref::<IdentityState>().expect("state"),
        value.clone(),
        value.clone(),
        never.clone(),
        Execution::pure(),
        Some(u16::MAX),
        None,
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
        oversized.push(Declaration::State(StateDeclaration::new(
            implementation.clone(),
            value.clone(),
            value.clone(),
            nominal_contract_ref::<Never>().expect("never"),
            Execution::pure(),
            (index < 11_999).then_some(index + 1),
            None,
        )));
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
    );
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
    );
    let later_back_edge = StateDeclaration::new(
        implementation.clone(),
        value.clone(),
        value.clone(),
        never.clone(),
        Execution::pure(),
        Some(0),
        None,
    );
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
            Declaration::State(StateDeclaration::new(
                implementation,
                value.clone(),
                value.clone(),
                never.clone(),
                Execution::pure(),
                None,
                None,
            ))
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
