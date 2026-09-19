// Public DSL composition fixtures; these are not maintained lifecycle or Runtime evidence.
#[allow(dead_code)]
#[path = "phase_a/contracts.rs"]
mod contracts;
use mfm_program as source;

use contracts::*;
use source::*;

// Publication of real maintained definitions replaces these nominal fixtures in A1's product proof.
type Child = Operation<(Effect<Configure, Transaction>, Read<Observe, Observation>)>;
type Lifecycle = Operation<(
    Effect<Deploy, Transaction>,
    Child,
    Pure<Validate>,
    Pure<Finish>,
)>;

fn accepts<S: AuthoringSource>(_: &S) {}
fn from_input<S: AuthoringSource<Input = Input, Output = Report>>(_: &S) {}
fn from_deployed<S: AuthoringSource<Input = Deployed, Output = Observed>>(_: &S) {}

#[test]
fn source_shapes_connect_without_state_clone_or_default_bounds() {
    accepts(&Pure::<Add>::default());
    accepts(&Effect::<Deploy, Transaction>::default());
    from_input(&Lifecycle::default());
    from_deployed(&Child::default());
    from_input(&Operation::new((
        Effect::<Deploy, Transaction>::default(),
        Pure::<Add>::default(),
        Child::default(),
        Pure::<Validate>::default(),
        Pure::<Finish>::default(),
    )));
    from_input(&Operation::new((
        Effect::<Deploy, Transaction>::default(),
        Pure::<Add>::default(),
        Pure::<Custom>::default(),
        Child::default(),
        Pure::<Validate>::default(),
        Pure::<Finish>::default(),
    )));
    from_input(&(
        (
            Effect::<Deploy, Transaction>::default(),
            Pure::<Add>::default(),
        ),
        (
            Child::default(),
            (Pure::<Validate>::default(), Pure::<Finish>::default()),
        ),
    ));
}

#[derive(Clone)]
struct Collection {
    demand: u32,
}
impl OperationDefinition for Collection {
    type Body = Operation<(Pure<Add>, Pure<Custom>), LocalDefaults>;
}
impl<Parent: ?Sized> Plan<Parent> for Collection {
    type Config = u32;
    fn plan<'a>(&'a self, _: &'a Parent) -> mfm_program::Result<(&'a u32, Self::Body)> {
        Ok((&self.demand, Self::Body::default()))
    }
}
struct LocalDefaults;
impl OperationDefaults for LocalDefaults {
    type Handler = Stop;
    type Targets = ();
}
impl ResolveDefaults<u32> for LocalDefaults {
    fn resolve(demand: &u32) -> source::Result<PolicyValues<Stop>> {
        Ok(PolicyValues {
            handler: None,
            retries: Some(*demand),
            restarts: None,
        })
    }
}
struct Collections;
impl OperationDefinition for Collections {
    type Body = Vec<Operation<Collection>>;
}
impl Plan<Deployed> for Collections {
    type Config = Deployed;
    fn plan<'a>(&'a self, parent: &'a Deployed) -> source::Result<(&'a Deployed, Self::Body)> {
        let demands = if parent.value == 0 {
            vec![]
        } else {
            vec![17, 3, 29]
        };
        Ok((
            parent,
            demands
                .into_iter()
                .map(|demand| Operation::new(Collection { demand }))
                .collect(),
        ))
    }
}
struct Installed;
impl OperationDefinition for Installed {
    type Body = (Pure<Add>, Pure<Custom>);
}
struct Resources;
impl ProgramEnvironment for Resources {
    // Installed has no Plan, and LocalDefaults has no resolver for this environment's input.
    type Sources = Operation<Installed, LocalDefaults>;
}

#[test]
fn construction_resolves_local_demands_in_order_and_cold_load_needs_no_plan() {
    let input = Deployed { value: 17 };
    let program = compile(
        mfm_ids::EntryPointId::new("mfm.proof/collections@1").unwrap(),
        &Operation::new(Collections),
        &input,
        &Resources,
        ProgramLimits::new(0),
    )
    .unwrap();
    let cold = load(program.canonical_bytes(), &Resources).unwrap();
    let retries: Vec<_> = cold
        .declarations()
        .iter()
        .map(|state| state.allowances().retries())
        .collect();
    assert_eq!(retries, [17, 17, 3, 3, 29, 29]);
    assert_eq!(cold.canonical_bytes(), program.canonical_bytes());
    assert_eq!(
        program.initial_value_ref(),
        mfm_values::Object::from_value(&input).unwrap().value_ref()
    );
    let empty = compile(
        mfm_ids::EntryPointId::new("mfm.proof/collections@1").unwrap(),
        &Operation::new(Collections),
        &Deployed { value: 0 },
        &Resources,
        ProgramLimits::new(0),
    )
    .unwrap();
    assert!(empty.declarations().is_empty());
    assert_eq!(
        load(empty.canonical_bytes(), &Resources)
            .unwrap()
            .content_ref(),
        empty.content_ref()
    );
}

#[test]
fn all_supported_tuple_arities_have_identity_planning() {
    macro_rules! check {
        ($($item:expr),+) => {{
            let source = ($($item,)+);
            accepts(&source);
            let parent = "borrowed configuration";
            let (same, body) = source.plan(parent).unwrap();
            accepts(&body);
            assert!(std::ptr::eq(parent, same));
        }};
    }
    let s = Pure::<Add>::default();
    check!(s);
    check!(s, s);
    check!(s, s, s);
    check!(s, s, s, s);
    check!(s, s, s, s, s);
    check!(s, s, s, s, s, s);
    check!(s, s, s, s, s, s, s);
    check!(s, s, s, s, s, s, s, s);
    check!(s, s, s, s, s, s, s, s, s);
    check!(s, s, s, s, s, s, s, s, s, s);
    check!(s, s, s, s, s, s, s, s, s, s, s);
    check!(s, s, s, s, s, s, s, s, s, s, s, s);
}

// Definition-only discovery must not acquire a fresh Plan<Root> bound.
struct StoredOnly;
impl OperationDefinition for StoredOnly {
    type Body = (Pure<Add>,);
}
#[test]
fn source_endpoints_do_not_require_fresh_planning() {
    accepts(&Operation::new(StoredOnly));
}

#[test]
fn incompatible_source_connections_are_rejected_by_rust() {
    let tests = trybuild::TestCases::new();
    tests.compile_fail("tests/ui/source_*.rs");
}
