use mfm_ids::StableId;
use mfm_program::structured::{
    ClosedSum, Direct, FanOutResults, Never, OperationBuilder, Pure, SafeFailureNotApplicable,
    State,
};
use mfm_program_derive::MfmValue;
use mfm_spec::structured::AuthoredDeclaration;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.fixture",
    name = "structured_input",
    version = "1",
    schema = "mfm.fixture.structured_input"
)]
struct Input {
    value: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.fixture",
    name = "structured_output",
    version = "1",
    schema = "mfm.fixture.structured_output"
)]
struct Output {
    value: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[mfm(
    namespace = "mfm.fixture",
    name = "structured_failure",
    version = "1",
    schema = "mfm.fixture.structured_failure"
)]
struct Failure {
    code: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[mfm(
    namespace = "mfm.fixture",
    name = "structured_selector",
    version = "1",
    schema = "mfm.fixture.structured_selector"
)]
enum Selector {
    Left { value: Output },
    Right,
}

impl ClosedSum for Selector {}

struct FirstState;
struct SecondState;
type KernelNever = Never;
type InnerJoin = FanOutResults<Input, Never>;
type OuterJoin = FanOutResults<InnerJoin, Never>;
struct AggregateState;

impl State for FirstState {
    type Input = Input;
    type Output = Output;
    type Failure = KernelNever;
    type Request = ();
    type Returned = ();
    type SafeFailure = ();
    type Execution = Pure;
    type SafeFailureDisposition = SafeFailureNotApplicable;
    type Capability = Direct;

    fn semantic_state_id() -> mfm_program::Result<StableId> {
        Ok(stable("first"))
    }
}

#[test]
fn a_source_alias_to_kernel_never_uses_the_reserved_infallible_contract() {
    let contract = mfm_program::structured::state_contract::<FirstState>()
        .expect("source-alias state contract");
    assert!(matches!(
        contract.failure_contract,
        mfm_spec::structured::StructuredFailureContract::Never
    ));

    let program = two_state_program();
    assert!(matches!(
        program.failure_contract,
        mfm_spec::structured::StructuredFailureContract::Never
    ));
}

impl State for SecondState {
    type Input = Output;
    type Output = Output;
    type Failure = Never;
    type Request = ();
    type Returned = ();
    type SafeFailure = ();
    type Execution = Pure;
    type SafeFailureDisposition = SafeFailureNotApplicable;
    type Capability = Direct;

    fn semantic_state_id() -> mfm_program::Result<StableId> {
        Ok(stable("second"))
    }
}

impl State for AggregateState {
    type Input = OuterJoin;
    type Output = Output;
    type Failure = Never;
    type Request = ();
    type Returned = ();
    type SafeFailure = ();
    type Execution = Pure;
    type SafeFailureDisposition = SafeFailureNotApplicable;
    type Capability = Direct;

    fn semantic_state_id() -> mfm_program::Result<StableId> {
        Ok(stable("aggregate"))
    }
}

#[test]
fn declaration_order_and_semantic_ids_are_stable() {
    let first = two_state_program();
    let second = two_state_program();

    assert_eq!(first, second);
    assert_eq!(
        first.canonical_json().expect("canonical authored bytes"),
        second.canonical_json().expect("canonical authored bytes")
    );
    assert_eq!(
        first.content_ref().expect("authored ref"),
        second.content_ref().expect("authored ref")
    );

    let labels: Vec<&str> = first
        .root
        .declarations
        .iter()
        .map(|declaration| match declaration {
            AuthoredDeclaration::State(state) => state.label.as_str(),
            _ => panic!("fixture contains only states"),
        })
        .collect();
    assert_eq!(labels, ["first", "second"]);

    let semantic_ids: Vec<String> = first
        .root
        .declarations
        .iter()
        .map(|declaration| match declaration {
            AuthoredDeclaration::State(state) => state.semantic_call_id.to_string(),
            _ => panic!("fixture contains only states"),
        })
        .collect();
    assert_ne!(semantic_ids[0], semantic_ids[1]);
}

#[test]
fn match_is_type_derived_exhaustive_and_lexically_sealed() {
    let mut builder =
        OperationBuilder::<Output, Never>::new(stable("mfm.fixture/match"), stable("root"))
            .expect("builder");
    let selector = builder
        .input::<Selector>(stable("selector"))
        .expect("selector input");
    let fallback = builder
        .input::<Output>(stable("fallback"))
        .expect("fallback input");
    let mut leaked = None;
    let merged = builder
        .root()
        .match_value(stable("choice"), &selector, |arms| {
            arms.arm("left", stable("left-arm"), |block, payloads| {
                let payload = payloads.value::<Output>(&[stable("value")])?;
                leaked = Some(payload.clone());
                block.normal(&payload)
            })?;
            arms.arm("right", stable("right-arm"), |block, _| {
                block.normal(&fallback)
            })
        })
        .expect("exhaustive match");
    let escaped = leaked.expect("left payload captured by test");
    let error = builder
        .root()
        .state::<SecondState>(stable("illegal-use"), &escaped)
        .err()
        .expect("inactive branch value must not dominate the parent");
    assert!(error.to_string().contains("does not dominate"));
    let completion = builder.succeed(&merged).expect("merged success");
    builder.finish(completion).expect("valid match program");

    let mut incomplete = OperationBuilder::<Output, Never>::new(
        stable("mfm.fixture/incomplete-match"),
        stable("root"),
    )
    .expect("builder");
    let selector = incomplete
        .input::<Selector>(stable("selector"))
        .expect("selector input");
    let fallback = incomplete
        .input::<Output>(stable("fallback"))
        .expect("fallback input");
    let error = incomplete
        .root()
        .match_value(stable("choice"), &selector, |arms| {
            arms.arm("left", stable("left-arm"), |block, _| {
                block.normal(&fallback)
            })
        })
        .expect_err("missing canonical tag must fail authoring");
    assert!(error.to_string().contains("not exhaustive"));
}

#[test]
fn match_merges_aggregate_results_and_state_returns_aggregate() {
    struct ReturnJoinState;
    impl State for ReturnJoinState {
        type Input = Input;
        type Output = InnerJoin;
        type Failure = Never;
        type Request = ();
        type Returned = ();
        type SafeFailure = ();
        type Execution = Pure;
        type SafeFailureDisposition = SafeFailureNotApplicable;
        type Capability = Direct;

        fn semantic_state_id() -> mfm_program::Result<StableId> {
            Ok(stable("return-join"))
        }
    }

    struct ConsumeJoinState;
    impl State for ConsumeJoinState {
        type Input = InnerJoin;
        type Output = Output;
        type Failure = Never;
        type Request = ();
        type Returned = ();
        type SafeFailure = ();
        type Execution = Pure;
        type SafeFailureDisposition = SafeFailureNotApplicable;
        type Capability = Direct;

        fn semantic_state_id() -> mfm_program::Result<StableId> {
            Ok(stable("consume-join"))
        }
    }

    let mut builder = OperationBuilder::<Output, Never>::new(
        stable("mfm.fixture/aggregate-match"),
        stable("root"),
    )
    .expect("builder");
    let input = builder.input::<Input>(stable("input")).expect("input");
    let selector = builder
        .input::<Selector>(stable("selector"))
        .expect("selector");
    let mut fan_out = builder
        .root()
        .fan_out::<Input, Never>(stable("parallel"))
        .expect("fan-out");
    fan_out
        .lane(stable("lane-a"), |lane| lane.normal(&input))
        .expect("lane a");
    fan_out
        .lane(stable("lane-b"), |lane| lane.normal(&input))
        .expect("lane b");
    let join = fan_out.finish().expect("join");
    let returned = builder
        .root()
        .state::<ReturnJoinState>(stable("return-join"), &input)
        .expect("return join state accepts scalar input")
        .infallible()
        .expect("return join output is aggregate");
    let merged = builder
        .root()
        .match_value(stable("choice"), &selector, |arms| {
            arms.arm("left", stable("left-arm"), |block, _| {
                block.normal(&join)
            })?;
            arms.arm("right", stable("right-arm"), |block, _| {
                block.normal(&returned)
            })
        })
        .expect("Match merges aggregate arm products");
    let consumed = builder
        .root()
        .state::<ConsumeJoinState>(stable("consume-join"), &merged)
        .expect("later state consumes merged aggregate")
        .infallible()
        .expect("consume completion");
    let completion = builder.succeed(&consumed).expect("root success");
    builder
        .finish(completion)
        .expect("aggregate composition through Match and state is admitted");
}

#[test]
fn fan_out_is_non_empty_and_depth_two_join_is_consumable() {
    let mut empty =
        OperationBuilder::<Output, Never>::new(stable("mfm.fixture/empty-fan-out"), stable("root"))
            .expect("builder");
    let error = empty
        .root()
        .fan_out::<Output, Never>(stable("parallel"))
        .expect("fan-out builder")
        .finish()
        .expect_err("empty fan-out must fail");
    assert!(error.to_string().contains("at least one lane"));

    let mut bounded = OperationBuilder::<Output, Never>::new(
        stable("mfm.fixture/depth-two-fan-out"),
        stable("root"),
    )
    .expect("builder");
    let input = bounded.input::<Input>(stable("input")).expect("input");
    let mut outer = bounded
        .root()
        .fan_out::<InnerJoin, Never>(stable("outer"))
        .expect("outer fan-out");
    outer
        .lane(stable("outer-lane"), |outer_lane| {
            let mut inner = outer_lane.fan_out::<Input, Never>(stable("inner"))?;
            inner.lane(stable("inner-lane"), |inner_lane| inner_lane.normal(&input))?;
            let inner_result = inner.finish()?;
            outer_lane.normal(&inner_result)
        })
        .expect("depth-two lane");
    let outer_result = outer.finish().expect("outer join");
    let aggregate = bounded
        .root()
        .state::<AggregateState>(stable("aggregate"), &outer_result)
        .expect("aggregate join input")
        .infallible()
        .expect("aggregate completion");
    let completion = bounded.succeed(&aggregate).expect("root aggregate");
    bounded
        .finish(completion)
        .expect("depth-two fan-out is admitted");
}

#[test]
fn root_success_and_failure_are_affine_tail_choices_without_declarations() {
    let mut success_builder =
        OperationBuilder::<Output, Failure>::new(stable("root-success"), stable("root"))
            .expect("success builder");
    let success = success_builder
        .input::<Output>(stable("success"))
        .expect("success input");
    let completion = success_builder.succeed(&success).expect("root success");
    let success_program = success_builder.finish(completion).expect("success program");
    assert!(success_program.root.declarations.is_empty());
    assert_eq!(
        success_program.root.tail,
        mfm_spec::structured::BlockTail::Normal(success.slot().clone())
    );

    let mut failure_builder =
        OperationBuilder::<Output, Failure>::new(stable("root-failure"), stable("root"))
            .expect("failure builder");
    let failure = failure_builder
        .input::<Failure>(stable("failure"))
        .expect("failure input");
    let completion = failure_builder.fail(&failure).expect("root failure");
    let failure_program = failure_builder.finish(completion).expect("failure program");
    assert!(failure_program.root.declarations.is_empty());
    assert_eq!(
        failure_program.root.tail,
        mfm_spec::structured::BlockTail::ScopeFailure(failure.slot().clone())
    );

    let mut affine =
        OperationBuilder::<Output, Failure>::new(stable("root-affine"), stable("root"))
            .expect("affine builder");
    let first = affine
        .input::<Output>(stable("first"))
        .expect("first success");
    let second = affine
        .input::<Output>(stable("second"))
        .expect("second success");
    let _selected = affine.succeed(&first).expect("first root choice");
    let error = affine
        .succeed(&second)
        .err()
        .expect("a second root completion choice must fail");
    assert!(error.to_string().contains("already selected"));
}

fn two_state_program() -> mfm_spec::structured::AuthoredStructuredProgram {
    let mut builder =
        OperationBuilder::<Output, Never>::new(stable("mfm.fixture/two-state"), stable("root"))
            .expect("builder");
    let input = builder.input::<Input>(stable("input")).expect("input");
    let first = builder
        .root()
        .state::<FirstState>(stable("first"), &input)
        .expect("first state")
        .infallible()
        .expect("first completion");
    let second = builder
        .root()
        .state::<SecondState>(stable("second"), &first)
        .expect("second state")
        .infallible()
        .expect("second completion");
    let completion = builder.succeed(&second).expect("success");
    builder.finish(completion).expect("program")
}

fn stable(value: &str) -> StableId {
    StableId::new(value).expect("stable fixture id")
}
