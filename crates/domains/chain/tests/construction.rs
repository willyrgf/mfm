use mfm_chain::{CheckedAdd, CheckedAddition};
use mfm_ids::{EntryPointId, StatePosition};
use mfm_program::{
    compile, load, Checkpoint, CheckpointMarker, Identity, NoParams, Operation, OperationDefaults,
    OperationDefinition, PolicyValues, ProgramEnvironment, ProgramLimits, Pure, ResolveDefaults,
    Stop,
};
use mfm_values::Unsigned256;

struct BeforeAdd;
impl CheckpointMarker for BeforeAdd {
    type Context = CheckedAddition;
}
struct AfterAdd;
impl CheckpointMarker for AfterAdd {
    type Context = Unsigned256;
}
struct Policies;
impl OperationDefaults for Policies {
    type Handler = Stop;
    type Targets = (BeforeAdd,);
}
impl<C: ?Sized> ResolveDefaults<C> for Policies {
    fn resolve(_: &C) -> mfm_program::Result<PolicyValues<Stop>> {
        Ok(PolicyValues {
            handler: Some(NoParams),
            retries: Some(0),
            restarts: Some(1),
        })
    }
}

// No Plan implementation exists for this installed definition. Cold construction only visits types.
struct Installed;
impl OperationDefinition for Installed {
    type Body = (Checkpoint<BeforeAdd>, Pure<CheckedAdd>);
}
struct Resources;
impl ProgramEnvironment for Resources {
    type Sources = (Operation<Installed, Policies>, Identity<Unsigned256>);
}
struct Empty;
impl ProgramEnvironment for Empty {
    type Sources = ();
}

#[test]
fn checked_add_compiles_and_loads_without_planning_or_live_handles() {
    let input = CheckedAddition::new("42", "42").unwrap();
    let source = Operation::new((
        Identity::<CheckedAddition>::default(),
        Pure::<CheckedAdd>::default(),
    ));
    let program = compile(
        EntryPointId::new("mfm.chain/add@1").unwrap(),
        &source,
        &input,
        &Resources,
        ProgramLimits::new(0),
    )
    .unwrap();
    let cold = load(program.canonical_bytes(), &Resources).unwrap();
    assert_eq!(cold.content_ref(), program.content_ref());
    assert_eq!(cold.canonical_bytes(), program.canonical_bytes());
    assert!(cold.bindings().is_empty());
    assert!(cold.executable(StatePosition::new(0).unwrap()).is_some());
    assert!(load(program.canonical_bytes(), &Empty).is_err());
    let object = mfm_values::Object::from_value(&input).unwrap();
    assert_eq!(program.initial_value_ref(), object.value_ref());
    cold.admit(&object, cold.admitted_context_contract_ref())
        .unwrap();
    assert!(cold
        .admit(&object, cold.root_success_contract_ref())
        .is_err());
}

#[test]
fn typed_checkpoint_targets_lower_and_survive_cold_construction() {
    type Scoped = Operation<(Checkpoint<BeforeAdd>, Pure<CheckedAdd>), Policies>;
    let input = CheckedAddition::new("42", "42").unwrap();
    let program = compile(
        EntryPointId::new("mfm.chain/scoped-add@1").unwrap(),
        &Scoped::default(),
        &input,
        &Resources,
        ProgramLimits::new(1),
    )
    .unwrap();
    let cold = load(program.canonical_bytes(), &Resources).unwrap();
    let state = &cold.declarations()[0];
    assert_eq!(state.recovery_targets()[0].position().index(), 0);
    assert_eq!(state.allowances().retries(), 0);
    assert_eq!(state.allowances().restarts(), 1);
    assert!(cold
        .executable(StatePosition::new(0).unwrap())
        .unwrap()
        .is_checkpoint());
    assert_eq!(cold.content_ref(), program.content_ref());

    type Missing = Operation<(Pure<CheckedAdd>,), Policies>;
    assert!(compile(
        EntryPointId::new("mfm.chain/missing-target@1").unwrap(),
        &Missing::default(),
        &input,
        &Resources,
        ProgramLimits::new(1)
    )
    .is_err());
    type Duplicate = Operation<
        (
            Checkpoint<BeforeAdd>,
            Checkpoint<BeforeAdd>,
            Pure<CheckedAdd>,
        ),
        Policies,
    >;
    assert!(compile(
        EntryPointId::new("mfm.chain/duplicate-target@1").unwrap(),
        &Duplicate::default(),
        &input,
        &Resources,
        ProgramLimits::new(1)
    )
    .is_err());
    assert!(compile(
        EntryPointId::new("mfm.chain/terminal-marker@1").unwrap(),
        &(
            Pure::<CheckedAdd>::default(),
            Checkpoint::<AfterAdd>::default()
        ),
        &input,
        &Resources,
        ProgramLimits::new(1)
    )
    .is_err());
}
