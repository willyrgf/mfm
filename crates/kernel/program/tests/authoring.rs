use std::cell::Cell;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use mfm_capabilities::{CapabilityError, ReadCapabilityContract};
use mfm_ids::{ContentRef, DigestAlgorithm, DigestBytes, EntryPointId, SemanticTypeId, StableId};
use mfm_program::{
    expand_program, nominal_contract_ref, state_implementation_ref, CapabilityInjection,
    Declaration, InjectionWriter, Never, Operation, OperationExpansion, ProgramError,
    ProposedStateOutcome, PureState, ReadPreparationError, ReadState, State,
};
use mfm_program_derive::MfmValue;
use mfm_values::{framework_value_descriptor, EnumTagging, EnumVariantDescriptor, SchemaShape};
use serde::{Deserialize, Serialize};

macro_rules! value {
    ($name:ident) => {
        #[derive(Debug, Clone, Serialize, Deserialize, MfmValue)]
        #[serde(deny_unknown_fields)]
        struct $name {
            value: u8,
        }
    };
}

value!(A);
value!(B);
value!(C);
value!(D);
value!(Failure);

#[derive(Debug, Clone, Serialize, Deserialize, MfmValue)]
struct Tiny {}

macro_rules! pure_state {
    ($state:ident, $input:ty, $output:ty, $failure:ty, $id:literal) => {
        struct $state;

        impl State for $state {
            type Input = $input;
            type Output = $output;
            type Failure = $failure;

            fn state_id() -> mfm_program::Result<StableId> {
                StableId::new($id).map_err(|_| ProgramError::InvalidContract)
            }
        }

        impl PureState for $state {
            fn evaluate(input: Self::Input) -> ProposedStateOutcome<Self::Output, Self::Failure> {
                ProposedStateOutcome::Success {
                    output: Self::Output { value: input.value },
                }
            }
        }
    };
}

pure_state!(AtoB, A, B, Never, "mfm.test.authoring/a-to-b@1");
pure_state!(BtoC, B, C, Never, "mfm.test.authoring/b-to-c@1");
pure_state!(BtoD, B, D, Never, "mfm.test.authoring/b-to-d@1");
pure_state!(DtoC, D, C, Never, "mfm.test.authoring/d-to-c@1");
pure_state!(IdentityA, A, A, Never, "mfm.test.authoring/identity-a@1");
pure_state!(BtoA, B, A, Never, "mfm.test.authoring/b-to-a@1");
pure_state!(CtoD, C, D, Never, "mfm.test.authoring/c-to-d@1");

struct IdentityTiny;

impl State for IdentityTiny {
    type Input = Tiny;
    type Output = Tiny;
    type Failure = Never;

    fn state_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.test.authoring/identity-tiny@1")
            .map_err(|_| ProgramError::InvalidContract)
    }
}

impl PureState for IdentityTiny {
    fn evaluate(input: Self::Input) -> ProposedStateOutcome<Self::Output, Self::Failure> {
        ProposedStateOutcome::Success { output: input }
    }
}
pure_state!(
    FailureToB,
    Failure,
    B,
    Never,
    "mfm.test.authoring/failure-to-b@1"
);
pure_state!(
    FailureToC,
    Failure,
    C,
    Never,
    "mfm.test.authoring/failure-to-c@1"
);
pure_state!(
    FailureToA,
    Failure,
    A,
    Never,
    "mfm.test.authoring/failure-to-a@1"
);

struct FallibleAtoB;

impl State for FallibleAtoB {
    type Input = A;
    type Output = B;
    type Failure = Failure;

    fn state_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.test.authoring/fallible-a-to-b@1")
            .map_err(|_| ProgramError::InvalidContract)
    }
}

impl PureState for FallibleAtoB {
    fn evaluate(input: Self::Input) -> ProposedStateOutcome<Self::Output, Self::Failure> {
        ProposedStateOutcome::Success {
            output: B { value: input.value },
        }
    }
}

struct Empty;

impl Operation for Empty {
    type Input = A;
    type Output = A;
    type Failure = Never;

    fn expand(
        &self,
        _body: &mut OperationExpansion<Self::Input, Self::Output, Self::Failure>,
    ) -> mfm_program::Result<()> {
        Ok(())
    }
}

struct AtoBOperation;

impl Operation for AtoBOperation {
    type Input = A;
    type Output = B;
    type Failure = Never;

    fn expand(
        &self,
        body: &mut OperationExpansion<Self::Input, Self::Output, Self::Failure>,
    ) -> mfm_program::Result<()> {
        body.pure::<AtoB>()
    }
}

struct Parent;

impl Operation for Parent {
    type Input = A;
    type Output = C;
    type Failure = Never;

    fn expand(
        &self,
        body: &mut OperationExpansion<Self::Input, Self::Output, Self::Failure>,
    ) -> mfm_program::Result<()> {
        body.operation(&AtoBOperation)?;
        body.pure::<BtoC>()
    }
}

struct CountedRoot<'a> {
    calls: &'a Cell<usize>,
    fail: bool,
}

impl Operation for CountedRoot<'_> {
    type Input = A;
    type Output = A;
    type Failure = Never;

    fn expand(
        &self,
        _body: &mut OperationExpansion<Self::Input, Self::Output, Self::Failure>,
    ) -> mfm_program::Result<()> {
        self.calls.set(self.calls.get() + 1);
        if self.fail {
            Err(ProgramError::Canonical)
        } else {
            Ok(())
        }
    }
}

struct Recursive(u8);

impl Operation for Recursive {
    type Input = A;
    type Output = A;
    type Failure = Never;

    fn expand(
        &self,
        body: &mut OperationExpansion<Self::Input, Self::Output, Self::Failure>,
    ) -> mfm_program::Result<()> {
        if self.0 == 0 {
            body.pure::<IdentityA>()
        } else {
            body.operation(&Self(self.0 - 1))
        }
    }
}

struct MutualA(u8);
struct MutualB(u8);

impl Operation for MutualA {
    type Input = A;
    type Output = A;
    type Failure = Never;

    fn expand(
        &self,
        body: &mut OperationExpansion<Self::Input, Self::Output, Self::Failure>,
    ) -> mfm_program::Result<()> {
        if self.0 == 0 {
            body.pure::<IdentityA>()
        } else {
            body.operation(&MutualB(self.0 - 1))
        }
    }
}

impl Operation for MutualB {
    type Input = A;
    type Output = A;
    type Failure = Never;

    fn expand(
        &self,
        body: &mut OperationExpansion<Self::Input, Self::Output, Self::Failure>,
    ) -> mfm_program::Result<()> {
        if self.0 == 0 {
            body.pure::<IdentityA>()
        } else {
            body.operation(&MutualA(self.0 - 1))
        }
    }
}

struct InvalidEmpty;

impl Operation for InvalidEmpty {
    type Input = A;
    type Output = B;
    type Failure = Never;

    fn expand(
        &self,
        _body: &mut OperationExpansion<Self::Input, Self::Output, Self::Failure>,
    ) -> mfm_program::Result<()> {
        Ok(())
    }
}

struct ConfiguredIdentity(usize);

impl Operation for ConfiguredIdentity {
    type Input = A;
    type Output = A;
    type Failure = Never;

    fn expand(
        &self,
        body: &mut OperationExpansion<Self::Input, Self::Output, Self::Failure>,
    ) -> mfm_program::Result<()> {
        for _ in 0..self.0 {
            body.pure::<IdentityA>()?;
        }
        Ok(())
    }
}

struct RepeatedConfiguredChildren;

impl Operation for RepeatedConfiguredChildren {
    type Input = A;
    type Output = A;
    type Failure = Never;

    fn expand(
        &self,
        body: &mut OperationExpansion<Self::Input, Self::Output, Self::Failure>,
    ) -> mfm_program::Result<()> {
        body.operation(&ConfiguredIdentity(1))?;
        body.operation(&ConfiguredIdentity(2))
    }
}

struct EmptyChild;

impl Operation for EmptyChild {
    type Input = A;
    type Output = A;
    type Failure = Never;

    fn expand(
        &self,
        _body: &mut OperationExpansion<Self::Input, Self::Output, Self::Failure>,
    ) -> mfm_program::Result<()> {
        Ok(())
    }
}

struct CallbackErrorChild<'a>(&'a Cell<usize>);

impl Operation for CallbackErrorChild<'_> {
    type Input = A;
    type Output = A;
    type Failure = Never;

    fn expand(
        &self,
        body: &mut OperationExpansion<Self::Input, Self::Output, Self::Failure>,
    ) -> mfm_program::Result<()> {
        self.0.set(self.0.get() + 1);
        body.pure::<IdentityA>()?;
        Err(ProgramError::Canonical)
    }
}

struct AtomicChildren;

impl Operation for AtomicChildren {
    type Input = A;
    type Output = A;
    type Failure = Never;

    fn expand(
        &self,
        body: &mut OperationExpansion<Self::Input, Self::Output, Self::Failure>,
    ) -> mfm_program::Result<()> {
        assert_eq!(
            body.operation(&EmptyChild),
            Err(ProgramError::InvalidContract)
        );
        let calls = Cell::new(0);
        assert_eq!(
            body.operation(&CallbackErrorChild(&calls)),
            Err(ProgramError::InvalidContract)
        );
        assert_eq!(calls.get(), 1);
        body.pure::<IdentityA>()
    }
}

struct ReadCapability;

impl ReadCapabilityContract for ReadCapability {
    type Intent = B;
    type Evidence = C;

    fn contract_id() -> mfm_capabilities::Result<StableId> {
        StableId::new("mfm.test.authoring/read@1").map_err(|_| CapabilityError::InvalidContract)
    }

    fn bind_evidence(
        _intent: &Self::Intent,
        _evidence: &Self::Evidence,
    ) -> mfm_capabilities::Result<()> {
        Ok(())
    }
}

struct ReadBtoC;

impl State for ReadBtoC {
    type Input = B;
    type Output = C;
    type Failure = Failure;

    fn state_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.test.authoring/read-b-to-c@1").map_err(|_| ProgramError::InvalidContract)
    }
}

impl ReadState<ReadCapability> for ReadBtoC {
    fn prepare(input: &Self::Input) -> Result<B, ReadPreparationError> {
        Ok(input.clone())
    }

    fn interpret(
        input: Self::Input,
        _evidence: &C,
    ) -> ProposedStateOutcome<Self::Output, Self::Failure> {
        ProposedStateOutcome::Success {
            output: C { value: input.value },
        }
    }
}

struct Before;

impl State for Before {
    type Input = A;
    type Output = B;
    type Failure = Failure;

    fn state_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.test.authoring/before@1").map_err(|_| ProgramError::InvalidContract)
    }
}

impl PureState for Before {
    fn evaluate(input: Self::Input) -> ProposedStateOutcome<Self::Output, Self::Failure> {
        ProposedStateOutcome::Success {
            output: B { value: input.value },
        }
    }
}

struct After;

impl State for After {
    type Input = C;
    type Output = D;
    type Failure = Never;

    fn state_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.test.authoring/after@1").map_err(|_| ProgramError::InvalidContract)
    }
}

impl PureState for After {
    fn evaluate(input: Self::Input) -> ProposedStateOutcome<Self::Output, Self::Failure> {
        ProposedStateOutcome::Success {
            output: D { value: input.value },
        }
    }
}

impl CapabilityInjection<ReadBtoC> for ReadCapability {
    type Setup = ContentRef;
    type ExpandedInput = A;
    type ExpandedOutput = D;

    fn original_binding_ref(setup: &Self::Setup) -> mfm_program::Result<ContentRef> {
        Ok(setup.clone())
    }

    fn write_before(_setup: &Self::Setup, writer: &mut InjectionWriter) -> mfm_program::Result<()> {
        writer.pure::<Before>()
    }

    fn write_after(_setup: &Self::Setup, writer: &mut InjectionWriter) -> mfm_program::Result<()> {
        writer.pure::<After>()
    }
}

struct ReadOperation {
    binding: ContentRef,
}

struct ControlledReadCapability;

impl ReadCapabilityContract for ControlledReadCapability {
    type Intent = A;
    type Evidence = A;

    fn contract_id() -> mfm_capabilities::Result<StableId> {
        StableId::new("mfm.test.authoring/controlled-read@1")
            .map_err(|_| CapabilityError::InvalidContract)
    }

    fn bind_evidence(
        _intent: &Self::Intent,
        _evidence: &Self::Evidence,
    ) -> mfm_capabilities::Result<()> {
        Ok(())
    }
}

struct ControlledRead;

impl State for ControlledRead {
    type Input = A;
    type Output = A;
    type Failure = Never;

    fn state_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.test.authoring/controlled-read-state@1")
            .map_err(|_| ProgramError::InvalidContract)
    }
}

struct ForeignBefore;

impl State for ForeignBefore {
    type Input = A;
    type Output = A;
    type Failure = D;

    fn state_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.test.authoring/foreign-before@1")
            .map_err(|_| ProgramError::InvalidContract)
    }
}

impl PureState for ForeignBefore {
    fn evaluate(input: Self::Input) -> ProposedStateOutcome<Self::Output, Self::Failure> {
        ProposedStateOutcome::Success { output: input }
    }
}

impl ReadState<ControlledReadCapability> for ControlledRead {
    fn prepare(input: &Self::Input) -> Result<A, ReadPreparationError> {
        Ok(input.clone())
    }

    fn interpret(
        input: Self::Input,
        _evidence: &A,
    ) -> ProposedStateOutcome<Self::Output, Self::Failure> {
        ProposedStateOutcome::Success { output: input }
    }
}

#[derive(Clone, Copy)]
enum HookFailure {
    None,
    Before,
    ForeignFailure,
    Binding,
    After,
}

struct HookCounts {
    before: Cell<usize>,
    binding: Cell<usize>,
    after: Cell<usize>,
    failure: HookFailure,
}

impl HookCounts {
    fn new(failure: HookFailure) -> Self {
        Self {
            before: Cell::new(0),
            binding: Cell::new(0),
            after: Cell::new(0),
            failure,
        }
    }
}

impl CapabilityInjection<ControlledRead> for ControlledReadCapability {
    type Setup = HookCounts;
    type ExpandedInput = A;
    type ExpandedOutput = A;

    fn original_binding_ref(setup: &Self::Setup) -> mfm_program::Result<ContentRef> {
        setup.binding.set(setup.binding.get() + 1);
        if matches!(setup.failure, HookFailure::Binding) {
            Err(ProgramError::Canonical)
        } else {
            nominal_contract_ref::<A>()
        }
    }

    fn write_before(setup: &Self::Setup, writer: &mut InjectionWriter) -> mfm_program::Result<()> {
        setup.before.set(setup.before.get() + 1);
        if matches!(setup.failure, HookFailure::Before) {
            Err(ProgramError::Canonical)
        } else if matches!(setup.failure, HookFailure::ForeignFailure) {
            writer.pure::<ForeignBefore>()
        } else {
            writer.pure::<IdentityA>()
        }
    }

    fn write_after(setup: &Self::Setup, writer: &mut InjectionWriter) -> mfm_program::Result<()> {
        setup.after.set(setup.after.get() + 1);
        if matches!(setup.failure, HookFailure::After) {
            Err(ProgramError::Canonical)
        } else {
            writer.pure::<IdentityA>()
        }
    }
}

struct AtomicRead<'a> {
    setup: &'a HookCounts,
}

impl Operation for AtomicRead<'_> {
    type Input = A;
    type Output = A;
    type Failure = Never;

    fn expand(
        &self,
        body: &mut OperationExpansion<Self::Input, Self::Output, Self::Failure>,
    ) -> mfm_program::Result<()> {
        assert_eq!(
            body.read::<ControlledRead, ControlledReadCapability>(self.setup),
            Err(ProgramError::InvalidContract)
        );
        body.pure::<IdentityA>()
    }
}

struct RepeatedReads<'a> {
    setup: &'a HookCounts,
}

impl Operation for RepeatedReads<'_> {
    type Input = A;
    type Output = A;
    type Failure = Never;

    fn expand(
        &self,
        body: &mut OperationExpansion<Self::Input, Self::Output, Self::Failure>,
    ) -> mfm_program::Result<()> {
        body.read::<ControlledRead, ControlledReadCapability>(self.setup)?;
        body.read::<ControlledRead, ControlledReadCapability>(self.setup)
    }
}

impl Operation for ReadOperation {
    type Input = A;
    type Output = D;
    type Failure = Failure;

    fn expand(
        &self,
        body: &mut OperationExpansion<Self::Input, Self::Output, Self::Failure>,
    ) -> mfm_program::Result<()> {
        body.read::<ReadBtoC, ReadCapability>(&self.binding)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, MfmValue)]
#[serde(rename_all = "snake_case")]
enum Choice {
    Left(A),
    Right(B),
}

#[derive(Debug, Clone, Serialize, Deserialize, MfmValue)]
#[serde(rename_all = "snake_case")]
enum GenericExternal<T> {
    Selected(T),
}

#[derive(Debug, Clone, Serialize, Deserialize, MfmValue)]
#[serde(tag = "kind", content = "payload", rename_all = "snake_case")]
enum GenericAdjacent<T> {
    Selected(T),
}

#[derive(Debug, Clone, Serialize, Deserialize, MfmValue)]
#[serde(rename_all = "snake_case")]
enum UnsortedChoice {
    Zed(A),
    Alpha(B),
}

#[derive(Debug, Clone, Serialize, Deserialize, MfmValue)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum InternalChoice {
    Selected { value: u8 },
}

#[derive(Debug, Clone, Serialize, Deserialize, MfmValue)]
#[serde(rename_all = "snake_case")]
enum UnitChoice {
    Selected,
}

#[derive(Debug, Clone, Serialize, Deserialize, MfmValue)]
#[serde(rename_all = "snake_case")]
enum NamedChoice {
    Selected { value: u8 },
}

#[derive(Debug, Clone, Serialize, Deserialize, MfmValue)]
#[serde(rename_all = "snake_case")]
enum MultiFieldChoice {
    Selected(A, A),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LargeSelector<const N: usize>;

impl<const N: usize> mfm_values::MfmValue for LargeSelector<N> {
    fn schema_descriptor() -> mfm_values::Result<mfm_values::SchemaDescriptor> {
        let payload = <Tiny as mfm_values::MfmValue>::schema_descriptor()?;
        let payload_shape = payload.identity().canonical_json_shape()?.clone();
        let payload_schema = payload.schema_id()?;
        let payload_semantic = <Tiny as mfm_values::MfmValue>::semantic_id()?;
        let variants = (0..N)
            .map(|index| {
                EnumVariantDescriptor::new(
                    format!("a{index:03}"),
                    SchemaShape::InlineValue {
                        schema_id: payload_schema.clone(),
                        semantic_type_id: payload_semantic.clone(),
                        serialized_shape: Box::new(payload_shape.clone()),
                    },
                )
            })
            .collect();
        framework_value_descriptor(
            "mfm-program",
            Self::semantic_id()?,
            &format!("mfm.test.authoring.large-selector-{N}"),
            SchemaShape::Enum {
                tagging: EnumTagging::External,
                variants,
            },
            std::any::type_name::<Self>(),
        )
    }

    fn semantic_id() -> mfm_values::Result<SemanticTypeId> {
        SemanticTypeId::new(
            "mfm.test.authoring",
            &format!("large-selector-{N}"),
            "1",
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([(N % 251) as u8; 32]),
        )
        .map_err(|error| mfm_values::ValueError::Identity(error.to_string()))
    }
}

struct MatchOperation;

impl Operation for MatchOperation {
    type Input = Choice;
    type Output = C;
    type Failure = Never;

    fn expand(
        &self,
        body: &mut OperationExpansion<Self::Input, Self::Output, Self::Failure>,
    ) -> mfm_program::Result<()> {
        body.match_join::<Choice, C>(|arms| {
            arms.arm::<A>(
                StableId::new("left").map_err(|_| ProgramError::InvalidContract)?,
                |arm| {
                    arm.pure::<AtoB>()?;
                    arm.pure::<BtoC>()
                },
            )?;
            arms.arm::<B>(
                StableId::new("right").map_err(|_| ProgramError::InvalidContract)?,
                |arm| {
                    arm.pure::<BtoD>()?;
                    arm.pure::<DtoC>()
                },
            )
        })
    }
}

struct GenericExternalOperation;

impl Operation for GenericExternalOperation {
    type Input = GenericExternal<A>;
    type Output = A;
    type Failure = Never;

    fn expand(
        &self,
        body: &mut OperationExpansion<Self::Input, Self::Output, Self::Failure>,
    ) -> mfm_program::Result<()> {
        body.match_join::<GenericExternal<A>, A>(|arms| {
            arms.arm::<A>(
                StableId::new("selected").map_err(|_| ProgramError::InvalidContract)?,
                |arm| arm.pure::<IdentityA>(),
            )
        })
    }
}

struct GenericAdjacentOperation;

impl Operation for GenericAdjacentOperation {
    type Input = GenericAdjacent<A>;
    type Output = A;
    type Failure = Never;

    fn expand(
        &self,
        body: &mut OperationExpansion<Self::Input, Self::Output, Self::Failure>,
    ) -> mfm_program::Result<()> {
        body.match_join::<GenericAdjacent<A>, A>(|arms| {
            arms.arm::<A>(
                StableId::new("selected").map_err(|_| ProgramError::InvalidContract)?,
                |arm| arm.pure::<IdentityA>(),
            )
        })
    }
}

struct UnsortedMatchOperation;

impl Operation for UnsortedMatchOperation {
    type Input = UnsortedChoice;
    type Output = A;
    type Failure = Never;

    fn expand(
        &self,
        body: &mut OperationExpansion<Self::Input, Self::Output, Self::Failure>,
    ) -> mfm_program::Result<()> {
        body.match_join::<UnsortedChoice, A>(|arms| {
            arms.arm::<A>(
                StableId::new("zed").map_err(|_| ProgramError::InvalidContract)?,
                |arm| arm.pure::<IdentityA>(),
            )?;
            arms.arm::<B>(
                StableId::new("alpha").map_err(|_| ProgramError::InvalidContract)?,
                |arm| arm.pure::<BtoA>(),
            )
        })
    }
}

struct BadMatch(u8);

impl Operation for BadMatch {
    type Input = Choice;
    type Output = C;
    type Failure = Never;

    fn expand(
        &self,
        body: &mut OperationExpansion<Self::Input, Self::Output, Self::Failure>,
    ) -> mfm_program::Result<()> {
        body.match_join::<Choice, C>(|arms| match self.0 {
            0 => arms.arm::<A>(
                StableId::new("left").map_err(|_| ProgramError::InvalidContract)?,
                |arm| {
                    arm.pure::<AtoB>()?;
                    arm.pure::<BtoC>()
                },
            ),
            1 => arms.arm::<A>(
                StableId::new("unknown").map_err(|_| ProgramError::InvalidContract)?,
                |_| panic!("unknown arm callback must not run"),
            ),
            2 => {
                let left = StableId::new("left").map_err(|_| ProgramError::InvalidContract)?;
                arms.arm::<A>(left.clone(), |arm| {
                    arm.pure::<AtoB>()?;
                    arm.pure::<BtoC>()
                })?;
                arms.arm::<A>(left, |_| panic!("duplicate arm callback must not run"))
            }
            3 => arms.arm::<B>(
                StableId::new("left").map_err(|_| ProgramError::InvalidContract)?,
                |_| panic!("wrong payload callback must not run"),
            ),
            _ => unreachable!(),
        })
    }
}

struct AtomicMatch;

impl Operation for AtomicMatch {
    type Input = Choice;
    type Output = C;
    type Failure = Never;

    fn expand(
        &self,
        body: &mut OperationExpansion<Self::Input, Self::Output, Self::Failure>,
    ) -> mfm_program::Result<()> {
        assert_eq!(
            body.match_join::<Choice, C>(|arms| {
                arms.arm::<A>(
                    StableId::new("left").map_err(|_| ProgramError::InvalidContract)?,
                    |arm| {
                        arm.pure::<AtoB>()?;
                        arm.pure::<BtoC>()
                    },
                )?;
                Err(ProgramError::Canonical)
            }),
            Err(ProgramError::InvalidContract)
        );
        body.operation(&MatchOperation)
    }
}

macro_rules! unsupported_selector_operation {
    ($operation:ident, $selector:ty) => {
        struct $operation;

        impl Operation for $operation {
            type Input = $selector;
            type Output = A;
            type Failure = Never;

            fn expand(
                &self,
                body: &mut OperationExpansion<Self::Input, Self::Output, Self::Failure>,
            ) -> mfm_program::Result<()> {
                body.match_join::<$selector, A>(|_| {
                    panic!("unsupported selector callback must not run")
                })
            }
        }
    };
}

unsupported_selector_operation!(NonEnumOperation, A);
unsupported_selector_operation!(InternalOperation, InternalChoice);
unsupported_selector_operation!(UnitOperation, UnitChoice);
unsupported_selector_operation!(NamedOperation, NamedChoice);
unsupported_selector_operation!(MultiFieldOperation, MultiFieldChoice);

struct LargeMatch<const N: usize>;

impl<const N: usize> Operation for LargeMatch<N> {
    type Input = LargeSelector<N>;
    type Output = Tiny;
    type Failure = Never;

    fn expand(
        &self,
        body: &mut OperationExpansion<Self::Input, Self::Output, Self::Failure>,
    ) -> mfm_program::Result<()> {
        body.match_join::<LargeSelector<N>, Tiny>(|arms| {
            for index in 0..N {
                arms.arm::<Tiny>(
                    StableId::new(format!("a{index:03}"))
                        .map_err(|_| ProgramError::InvalidContract)?,
                    |arm| arm.pure::<IdentityTiny>(),
                )?;
            }
            Ok(())
        })
    }
}

struct EarlyChild;

impl Operation for EarlyChild {
    type Input = Choice;
    type Output = C;
    type Failure = Never;

    fn expand(
        &self,
        body: &mut OperationExpansion<Self::Input, Self::Output, Self::Failure>,
    ) -> mfm_program::Result<()> {
        body.match_join::<Choice, B>(|arms| {
            arms.arm::<A>(
                StableId::new("left").map_err(|_| ProgramError::InvalidContract)?,
                |arm| arm.pure::<AtoB>(),
            )?;
            arms.arm::<B>(
                StableId::new("right").map_err(|_| ProgramError::InvalidContract)?,
                |arm| arm.pure::<BtoC>(),
            )
        })?;
        body.pure::<BtoC>()
    }
}

struct EarlyParent;

impl Operation for EarlyParent {
    type Input = Choice;
    type Output = D;
    type Failure = Never;

    fn expand(
        &self,
        body: &mut OperationExpansion<Self::Input, Self::Output, Self::Failure>,
    ) -> mfm_program::Result<()> {
        body.operation(&EarlyChild)?;
        body.pure::<CtoD>()
    }
}

struct RecoveringHandler;

impl Operation for RecoveringHandler {
    type Input = A;
    type Output = C;
    type Failure = Never;

    fn expand(
        &self,
        body: &mut OperationExpansion<Self::Input, Self::Output, Self::Failure>,
    ) -> mfm_program::Result<()> {
        body.with_failure_handler::<Failure, B>(
            |protected| protected.pure::<FallibleAtoB>(),
            |handler| handler.pure::<FailureToB>(),
        )?;
        body.pure::<BtoC>()
    }
}

struct TerminalHandler;

impl Operation for TerminalHandler {
    type Input = A;
    type Output = C;
    type Failure = Never;

    fn expand(
        &self,
        body: &mut OperationExpansion<Self::Input, Self::Output, Self::Failure>,
    ) -> mfm_program::Result<()> {
        body.with_failure_handler::<Failure, B>(
            |protected| protected.pure::<FallibleAtoB>(),
            |handler| handler.pure::<FailureToC>(),
        )?;
        body.pure::<BtoC>()
    }
}

struct FallibleFailureToB;

impl State for FallibleFailureToB {
    type Input = Failure;
    type Output = B;
    type Failure = Failure;

    fn state_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.test.authoring/fallible-failure-to-b@1")
            .map_err(|_| ProgramError::InvalidContract)
    }
}

struct FallibleAtoC;

impl State for FallibleAtoC {
    type Input = A;
    type Output = C;
    type Failure = Failure;

    fn state_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.test.authoring/fallible-a-to-c@1")
            .map_err(|_| ProgramError::InvalidContract)
    }
}

impl PureState for FallibleAtoC {
    fn evaluate(input: Self::Input) -> ProposedStateOutcome<Self::Output, Self::Failure> {
        ProposedStateOutcome::Success {
            output: C { value: input.value },
        }
    }
}

impl PureState for FallibleFailureToB {
    fn evaluate(input: Self::Input) -> ProposedStateOutcome<Self::Output, Self::Failure> {
        ProposedStateOutcome::Failure { failure: input }
    }
}

struct NestedSameFailure;

impl Operation for NestedSameFailure {
    type Input = A;
    type Output = C;
    type Failure = Never;

    fn expand(
        &self,
        body: &mut OperationExpansion<Self::Input, Self::Output, Self::Failure>,
    ) -> mfm_program::Result<()> {
        body.with_failure_handler::<Failure, B>(
            |outer| {
                outer.with_failure_handler::<Failure, B>(
                    |inner| inner.pure::<FallibleAtoB>(),
                    |inner_handler| inner_handler.pure::<FallibleFailureToB>(),
                )
            },
            |outer_handler| outer_handler.pure::<FailureToB>(),
        )?;
        body.pure::<BtoC>()
    }
}

struct SelfFailingHandler;

impl Operation for SelfFailingHandler {
    type Input = A;
    type Output = B;
    type Failure = Never;

    fn expand(
        &self,
        body: &mut OperationExpansion<Self::Input, Self::Output, Self::Failure>,
    ) -> mfm_program::Result<()> {
        body.with_failure_handler::<Failure, B>(
            |protected| protected.pure::<FallibleAtoB>(),
            |handler| handler.pure::<FallibleFailureToB>(),
        )
    }
}

struct AtomicHandler;

impl Operation for AtomicHandler {
    type Input = A;
    type Output = C;
    type Failure = Never;

    fn expand(
        &self,
        body: &mut OperationExpansion<Self::Input, Self::Output, Self::Failure>,
    ) -> mfm_program::Result<()> {
        assert_eq!(
            body.with_failure_handler::<Failure, B>(
                |protected| protected.pure::<FallibleAtoB>(),
                |handler| {
                    handler.pure::<FailureToB>()?;
                    Err(ProgramError::Canonical)
                },
            ),
            Err(ProgramError::InvalidContract)
        );
        body.operation(&RecoveringHandler)
    }
}

struct EqualRootFailure;

impl Operation for EqualRootFailure {
    type Input = A;
    type Output = C;
    type Failure = Failure;

    fn expand(
        &self,
        body: &mut OperationExpansion<Self::Input, Self::Output, Self::Failure>,
    ) -> mfm_program::Result<()> {
        body.with_failure_handler::<Failure, B>(
            |protected| protected.pure::<FallibleAtoB>(),
            |handler| handler.pure::<FailureToB>(),
        )?;
        body.pure::<BtoC>()
    }
}

struct DirectProtectedOutput;

impl Operation for DirectProtectedOutput {
    type Input = A;
    type Output = C;
    type Failure = Never;

    fn expand(
        &self,
        body: &mut OperationExpansion<Self::Input, Self::Output, Self::Failure>,
    ) -> mfm_program::Result<()> {
        body.with_failure_handler::<Failure, B>(
            |protected| protected.pure::<FallibleAtoC>(),
            |handler| handler.pure::<FailureToB>(),
        )
    }
}

struct EmptyHandler;

impl Operation for EmptyHandler {
    type Input = A;
    type Output = B;
    type Failure = Never;

    fn expand(
        &self,
        body: &mut OperationExpansion<Self::Input, Self::Output, Self::Failure>,
    ) -> mfm_program::Result<()> {
        body.with_failure_handler::<Failure, B>(
            |protected| protected.pure::<FallibleAtoB>(),
            |_| Ok(()),
        )
    }
}

struct NeverHandler<'a> {
    protected_calls: &'a Cell<usize>,
    handler_calls: &'a Cell<usize>,
}

impl Operation for NeverHandler<'_> {
    type Input = A;
    type Output = A;
    type Failure = Never;

    fn expand(
        &self,
        body: &mut OperationExpansion<Self::Input, Self::Output, Self::Failure>,
    ) -> mfm_program::Result<()> {
        body.with_failure_handler::<Never, A>(
            |_| {
                self.protected_calls.set(self.protected_calls.get() + 1);
                Ok(())
            },
            |_| {
                self.handler_calls.set(self.handler_calls.get() + 1);
                Ok(())
            },
        )
    }
}

struct DepthCapability;

impl ReadCapabilityContract for DepthCapability {
    type Intent = A;
    type Evidence = A;

    fn contract_id() -> mfm_capabilities::Result<StableId> {
        StableId::new("mfm.test.authoring/depth-capability@1")
            .map_err(|_| CapabilityError::InvalidContract)
    }

    fn bind_evidence(
        _intent: &Self::Intent,
        _evidence: &Self::Evidence,
    ) -> mfm_capabilities::Result<()> {
        Ok(())
    }
}

struct DepthRead;

impl State for DepthRead {
    type Input = A;
    type Output = A;
    type Failure = Failure;

    fn state_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.test.authoring/depth-read@1").map_err(|_| ProgramError::InvalidContract)
    }
}

impl ReadState<DepthCapability> for DepthRead {
    fn prepare(input: &Self::Input) -> Result<A, ReadPreparationError> {
        Ok(input.clone())
    }

    fn interpret(
        input: Self::Input,
        _evidence: &A,
    ) -> ProposedStateOutcome<Self::Output, Self::Failure> {
        ProposedStateOutcome::Success { output: input }
    }
}

#[derive(Default)]
struct DepthCounters {
    before: AtomicUsize,
    binding: AtomicUsize,
    after: AtomicUsize,
}

impl CapabilityInjection<DepthRead> for DepthCapability {
    type Setup = Arc<DepthCounters>;
    type ExpandedInput = A;
    type ExpandedOutput = A;

    fn original_binding_ref(setup: &Self::Setup) -> mfm_program::Result<ContentRef> {
        setup.binding.fetch_add(1, Ordering::SeqCst);
        nominal_contract_ref::<A>()
    }

    fn write_before(setup: &Self::Setup, writer: &mut InjectionWriter) -> mfm_program::Result<()> {
        setup.before.fetch_add(1, Ordering::SeqCst);
        writer.pure::<IdentityA>()
    }

    fn write_after(setup: &Self::Setup, writer: &mut InjectionWriter) -> mfm_program::Result<()> {
        setup.after.fetch_add(1, Ordering::SeqCst);
        writer.pure::<IdentityA>()
    }
}

struct MixedDepth<'a> {
    remaining: u8,
    setup: &'a Arc<DepthCounters>,
}

impl Operation for MixedDepth<'_> {
    type Input = Choice;
    type Output = A;
    type Failure = Never;

    fn expand(
        &self,
        body: &mut OperationExpansion<Self::Input, Self::Output, Self::Failure>,
    ) -> mfm_program::Result<()> {
        if self.remaining > 0 {
            return body.operation(&MixedDepth {
                remaining: self.remaining - 1,
                setup: self.setup,
            });
        }
        body.match_join::<Choice, A>(|arms| {
            arms.arm::<A>(
                StableId::new("left").map_err(|_| ProgramError::InvalidContract)?,
                |branch| {
                    branch.with_failure_handler::<Failure, A>(
                        |protected| protected.read::<DepthRead, DepthCapability>(self.setup),
                        |handler| handler.pure::<FailureToA>(),
                    )
                },
            )?;
            arms.arm::<B>(
                StableId::new("right").map_err(|_| ProgramError::InvalidContract)?,
                |branch| branch.pure::<BtoA>(),
            )
        })?;
        body.pure::<IdentityA>()
    }
}

static DIRECT_DESCRIPTOR_CALLS: AtomicUsize = AtomicUsize::new(0);
static DIRECT_SEMANTIC_CALLS: AtomicUsize = AtomicUsize::new(0);
static DIRECT_STATE_CALLS: AtomicUsize = AtomicUsize::new(0);
static MEMO_DESCRIPTOR_CALLS: AtomicUsize = AtomicUsize::new(0);
static MEMO_SEMANTIC_CALLS: AtomicUsize = AtomicUsize::new(0);
static MEMO_STATE_CALLS: AtomicUsize = AtomicUsize::new(0);
static MEMO_CAPABILITY_CALLS: AtomicUsize = AtomicUsize::new(0);

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(transparent)]
struct DirectCountedValue(u8);

impl mfm_values::MfmValue for DirectCountedValue {
    fn schema_descriptor() -> mfm_values::Result<mfm_values::SchemaDescriptor> {
        DIRECT_DESCRIPTOR_CALLS.fetch_add(1, Ordering::SeqCst);
        framework_value_descriptor(
            "mfm-program",
            Self::semantic_id()?,
            "mfm.test.authoring.direct-counted-value",
            SchemaShape::UnsignedInteger { bits: 8 },
            std::any::type_name::<Self>(),
        )
    }

    fn semantic_id() -> mfm_values::Result<SemanticTypeId> {
        DIRECT_SEMANTIC_CALLS.fetch_add(1, Ordering::SeqCst);
        SemanticTypeId::new(
            "mfm.test.authoring",
            "direct-counted-value",
            "1",
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([0x41; 32]),
        )
        .map_err(|error| mfm_values::ValueError::Identity(error.to_string()))
    }
}

struct DirectCountedState;

impl State for DirectCountedState {
    type Input = DirectCountedValue;
    type Output = DirectCountedValue;
    type Failure = Never;

    fn state_id() -> mfm_program::Result<StableId> {
        DIRECT_STATE_CALLS.fetch_add(1, Ordering::SeqCst);
        StableId::new("mfm.test.authoring/direct-counted-state@1")
            .map_err(|_| ProgramError::InvalidContract)
    }
}

impl PureState for DirectCountedState {
    fn evaluate(input: Self::Input) -> ProposedStateOutcome<Self::Output, Self::Failure> {
        ProposedStateOutcome::Success { output: input }
    }
}

struct DirectCountedOperation;

impl Operation for DirectCountedOperation {
    type Input = DirectCountedValue;
    type Output = DirectCountedValue;
    type Failure = Never;

    fn expand(
        &self,
        body: &mut OperationExpansion<Self::Input, Self::Output, Self::Failure>,
    ) -> mfm_program::Result<()> {
        body.pure::<DirectCountedState>()?;
        body.pure::<DirectCountedState>()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(transparent)]
struct MemoValue(u8);

impl mfm_values::MfmValue for MemoValue {
    fn schema_descriptor() -> mfm_values::Result<mfm_values::SchemaDescriptor> {
        MEMO_DESCRIPTOR_CALLS.fetch_add(1, Ordering::SeqCst);
        framework_value_descriptor(
            "mfm-program",
            Self::semantic_id()?,
            "mfm.test.authoring.memo-value",
            SchemaShape::UnsignedInteger { bits: 8 },
            std::any::type_name::<Self>(),
        )
    }

    fn semantic_id() -> mfm_values::Result<SemanticTypeId> {
        MEMO_SEMANTIC_CALLS.fetch_add(1, Ordering::SeqCst);
        SemanticTypeId::new(
            "mfm.test.authoring",
            "memo-value",
            "1",
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([0x42; 32]),
        )
        .map_err(|error| mfm_values::ValueError::Identity(error.to_string()))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, MfmValue)]
#[serde(rename_all = "snake_case")]
enum MemoChoice {
    Selected(MemoValue),
}

struct MemoIdentity;

impl State for MemoIdentity {
    type Input = MemoValue;
    type Output = MemoValue;
    type Failure = Never;

    fn state_id() -> mfm_program::Result<StableId> {
        MEMO_STATE_CALLS.fetch_add(1, Ordering::SeqCst);
        StableId::new("mfm.test.authoring/memo-identity@1")
            .map_err(|_| ProgramError::InvalidContract)
    }
}

impl PureState for MemoIdentity {
    fn evaluate(input: Self::Input) -> ProposedStateOutcome<Self::Output, Self::Failure> {
        ProposedStateOutcome::Success { output: input }
    }
}

struct MemoFallibleIdentity;

impl State for MemoFallibleIdentity {
    type Input = MemoValue;
    type Output = MemoValue;
    type Failure = MemoValue;

    fn state_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.test.authoring/memo-fallible-identity@1")
            .map_err(|_| ProgramError::InvalidContract)
    }
}

impl PureState for MemoFallibleIdentity {
    fn evaluate(input: Self::Input) -> ProposedStateOutcome<Self::Output, Self::Failure> {
        ProposedStateOutcome::Success { output: input }
    }
}

struct MemoChild;

impl Operation for MemoChild {
    type Input = MemoValue;
    type Output = MemoValue;
    type Failure = Never;

    fn expand(
        &self,
        body: &mut OperationExpansion<Self::Input, Self::Output, Self::Failure>,
    ) -> mfm_program::Result<()> {
        body.pure::<MemoIdentity>()
    }
}

struct MemoCapability;

impl ReadCapabilityContract for MemoCapability {
    type Intent = MemoValue;
    type Evidence = MemoValue;

    fn contract_id() -> mfm_capabilities::Result<StableId> {
        MEMO_CAPABILITY_CALLS.fetch_add(1, Ordering::SeqCst);
        StableId::new("mfm.test.authoring/memo-capability@1")
            .map_err(|_| CapabilityError::InvalidContract)
    }

    fn bind_evidence(
        _intent: &Self::Intent,
        _evidence: &Self::Evidence,
    ) -> mfm_capabilities::Result<()> {
        Ok(())
    }
}

struct MemoRead;

impl State for MemoRead {
    type Input = MemoValue;
    type Output = MemoValue;
    type Failure = Never;

    fn state_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.test.authoring/memo-read@1").map_err(|_| ProgramError::InvalidContract)
    }
}

impl ReadState<MemoCapability> for MemoRead {
    fn prepare(input: &Self::Input) -> Result<MemoValue, ReadPreparationError> {
        Ok(input.clone())
    }

    fn interpret(
        input: Self::Input,
        _evidence: &MemoValue,
    ) -> ProposedStateOutcome<Self::Output, Self::Failure> {
        ProposedStateOutcome::Success { output: input }
    }
}

impl CapabilityInjection<MemoRead> for MemoCapability {
    type Setup = ContentRef;
    type ExpandedInput = MemoValue;
    type ExpandedOutput = MemoValue;

    fn original_binding_ref(setup: &Self::Setup) -> mfm_program::Result<ContentRef> {
        Ok(setup.clone())
    }

    fn write_before(_setup: &Self::Setup, writer: &mut InjectionWriter) -> mfm_program::Result<()> {
        writer.pure::<MemoIdentity>()
    }

    fn write_after(_setup: &Self::Setup, writer: &mut InjectionWriter) -> mfm_program::Result<()> {
        writer.pure::<MemoIdentity>()
    }
}

struct MemoAcrossScopes {
    binding: ContentRef,
}

impl Operation for MemoAcrossScopes {
    type Input = MemoChoice;
    type Output = MemoValue;
    type Failure = Never;

    fn expand(
        &self,
        body: &mut OperationExpansion<Self::Input, Self::Output, Self::Failure>,
    ) -> mfm_program::Result<()> {
        body.match_join::<MemoChoice, MemoValue>(|arms| {
            arms.arm::<MemoValue>(
                StableId::new("selected").map_err(|_| ProgramError::InvalidContract)?,
                |arm| arm.operation(&MemoChild),
            )
        })?;
        body.with_failure_handler::<MemoValue, MemoValue>(
            |protected| protected.pure::<MemoFallibleIdentity>(),
            |handler| handler.pure::<MemoIdentity>(),
        )?;
        body.read::<MemoRead, MemoCapability>(&self.binding)?;
        body.read::<MemoRead, MemoCapability>(&self.binding)
    }
}

struct RetryWrongJoin(bool);

impl Operation for RetryWrongJoin {
    type Input = Choice;
    type Output = C;
    type Failure = Never;

    fn expand(
        &self,
        body: &mut OperationExpansion<Self::Input, Self::Output, Self::Failure>,
    ) -> mfm_program::Result<()> {
        body.match_join::<Choice, C>(|arms| {
            let left = StableId::new("left").map_err(|_| ProgramError::InvalidContract)?;
            if self.0 {
                assert_eq!(
                    arms.arm::<A>(left.clone(), |arm| arm.pure::<AtoB>()),
                    Err(ProgramError::InvalidContract)
                );
            }
            arms.arm::<A>(left, |arm| {
                arm.pure::<AtoB>()?;
                arm.pure::<BtoC>()
            })?;
            arms.arm::<B>(
                StableId::new("right").map_err(|_| ProgramError::InvalidContract)?,
                |arm| arm.pure::<BtoC>(),
            )
        })
    }
}

struct WrongTerminalOutput;

impl Operation for WrongTerminalOutput {
    type Input = A;
    type Output = C;
    type Failure = Never;

    fn expand(
        &self,
        body: &mut OperationExpansion<Self::Input, Self::Output, Self::Failure>,
    ) -> mfm_program::Result<()> {
        body.pure::<AtoB>()
    }
}

fn entry() -> EntryPointId {
    EntryPointId::new("mfm.test/authoring@1").expect("entry")
}

fn state(program: &mfm_program::Program, index: usize) -> &mfm_program::StateDeclaration {
    let Declaration::State(state) = &program.declarations()[index] else {
        panic!("declaration {index} is not a State");
    };
    state
}

#[test]
fn root_and_child_callbacks_are_exact_and_depth_bounded() {
    let calls = Cell::new(0);
    let empty = expand_program(
        entry(),
        &CountedRoot {
            calls: &calls,
            fail: false,
        },
    )
    .expect("empty identity");
    assert!(empty.declarations().is_empty());
    assert_eq!(calls.get(), 1);
    assert_eq!(
        expand_program(
            entry(),
            &CountedRoot {
                calls: &calls,
                fail: true,
            },
        ),
        Err(ProgramError::InvalidContract)
    );
    assert_eq!(calls.get(), 2);

    let parent = expand_program(entry(), &Parent).expect("nested parent");
    assert_eq!(parent.declarations().len(), 2);
    assert_eq!(state(&parent, 0).next_index(), Some(1));
    assert_eq!(state(&parent, 1).next_index(), None);
    assert!(expand_program(entry(), &Recursive(63)).is_ok());
    assert_eq!(
        expand_program(entry(), &Recursive(64)),
        Err(ProgramError::Capacity)
    );
    assert!(expand_program(entry(), &MutualA(63)).is_ok());
    assert_eq!(
        expand_program(entry(), &MutualA(64)),
        Err(ProgramError::Capacity)
    );
    assert!(expand_program(entry(), &Empty).is_ok());
    assert_eq!(
        expand_program(entry(), &InvalidEmpty),
        Err(ProgramError::InvalidContract)
    );
    assert_eq!(
        expand_program(entry(), &WrongTerminalOutput),
        Err(ProgramError::InvalidContract)
    );
}

#[test]
fn one_expansion_owns_one_top_level_identity_memo() {
    DIRECT_DESCRIPTOR_CALLS.store(0, Ordering::SeqCst);
    DIRECT_SEMANTIC_CALLS.store(0, Ordering::SeqCst);
    DIRECT_STATE_CALLS.store(0, Ordering::SeqCst);

    expand_program(entry(), &DirectCountedOperation).expect("first counted expansion");
    assert_eq!(DIRECT_DESCRIPTOR_CALLS.load(Ordering::SeqCst), 1);
    assert_eq!(DIRECT_SEMANTIC_CALLS.load(Ordering::SeqCst), 2);
    assert_eq!(DIRECT_STATE_CALLS.load(Ordering::SeqCst), 1);

    expand_program(entry(), &DirectCountedOperation).expect("second counted expansion");
    assert_eq!(DIRECT_DESCRIPTOR_CALLS.load(Ordering::SeqCst), 2);
    assert_eq!(DIRECT_SEMANTIC_CALLS.load(Ordering::SeqCst), 4);
    assert_eq!(DIRECT_STATE_CALLS.load(Ordering::SeqCst), 2);

    MEMO_DESCRIPTOR_CALLS.store(0, Ordering::SeqCst);
    MEMO_SEMANTIC_CALLS.store(0, Ordering::SeqCst);
    MEMO_STATE_CALLS.store(0, Ordering::SeqCst);
    MEMO_CAPABILITY_CALLS.store(0, Ordering::SeqCst);
    let program = expand_program(
        entry(),
        &MemoAcrossScopes {
            binding: nominal_contract_ref::<A>().expect("binding"),
        },
    )
    .expect("memo shared across nested scopes");
    assert_eq!(program.declarations().len(), 10);
    assert_eq!(MEMO_DESCRIPTOR_CALLS.load(Ordering::SeqCst), 2);
    assert_eq!(MEMO_SEMANTIC_CALLS.load(Ordering::SeqCst), 4);
    assert_eq!(MEMO_STATE_CALLS.load(Ordering::SeqCst), 1);
    assert_eq!(MEMO_CAPABILITY_CALLS.load(Ordering::SeqCst), 1);
}

#[test]
fn configured_children_repeat_without_parent_size_knowledge_and_fail_atomically() {
    let repeated = expand_program(entry(), &RepeatedConfiguredChildren).expect("repeated children");
    assert_eq!(repeated.declarations().len(), 3);
    assert_eq!(state(&repeated, 0).next_index(), Some(1));
    assert_eq!(state(&repeated, 1).next_index(), Some(2));
    assert_eq!(state(&repeated, 2).next_index(), None);

    let atomic = expand_program(entry(), &AtomicChildren).expect("atomic child failures");
    assert_eq!(atomic.declarations().len(), 1);
    assert_eq!(
        state(&atomic, 0).state_implementation_ref(),
        &state_implementation_ref::<IdentityA>().expect("identity")
    );
    assert_eq!(
        atomic.canonical_bytes(),
        expand_program(entry(), &ConfiguredIdentity(1))
            .expect("clean child retry")
            .canonical_bytes()
    );
}

#[test]
fn nonempty_injection_wraps_one_kernel_owned_read_in_exact_order() {
    let binding = nominal_contract_ref::<A>().expect("binding");
    let program = expand_program(
        entry(),
        &ReadOperation {
            binding: binding.clone(),
        },
    )
    .expect("read program");
    assert_eq!(program.declarations().len(), 3);
    assert_eq!(
        state(&program, 0).state_implementation_ref(),
        &state_implementation_ref::<Before>().expect("before")
    );
    assert_eq!(
        state(&program, 1).state_implementation_ref(),
        &state_implementation_ref::<ReadBtoC>().expect("read")
    );
    assert_eq!(
        state(&program, 2).state_implementation_ref(),
        &state_implementation_ref::<After>().expect("after")
    );
    assert!(state(&program, 0).execution().is_pure());
    assert_eq!(state(&program, 1).execution().binding_ref(), Some(&binding));
    assert!(state(&program, 2).execution().is_pure());
    assert_eq!(state(&program, 0).next_index(), Some(1));
    assert_eq!(state(&program, 1).next_index(), Some(2));
    assert_eq!(state(&program, 2).next_index(), None);
    assert_eq!(state(&program, 0).failure_next_index(), None);
    assert_eq!(state(&program, 1).failure_next_index(), None);
}

#[test]
fn repeated_injection_is_exact_and_hook_failures_are_once_short_circuited_and_atomic() {
    let successful = HookCounts::new(HookFailure::None);
    let first =
        expand_program(entry(), &RepeatedReads { setup: &successful }).expect("repeated Reads");
    assert_eq!(first.declarations().len(), 6);
    assert_eq!(successful.before.get(), 2);
    assert_eq!(successful.binding.get(), 2);
    assert_eq!(successful.after.get(), 2);
    let second_setup = HookCounts::new(HookFailure::None);
    let second = expand_program(
        entry(),
        &RepeatedReads {
            setup: &second_setup,
        },
    )
    .expect("identical repeated Reads");
    assert_eq!(first.canonical_bytes(), second.canonical_bytes());
    assert_eq!(first.content_ref(), second.content_ref());

    for (failure, expected) in [
        (HookFailure::Before, (1, 0, 0)),
        (HookFailure::ForeignFailure, (1, 0, 0)),
        (HookFailure::Binding, (1, 1, 0)),
        (HookFailure::After, (1, 1, 1)),
    ] {
        let setup = HookCounts::new(failure);
        let program = expand_program(entry(), &AtomicRead { setup: &setup })
            .expect("failed injected occurrence leaves parent unchanged");
        assert_eq!(program.declarations().len(), 1);
        assert_eq!(
            (setup.before.get(), setup.binding.get(), setup.after.get()),
            expected
        );
    }
}

#[test]
fn structured_match_keeps_physical_arm_order_and_exact_join() {
    let program = expand_program(entry(), &MatchOperation).expect("Match Program");
    assert_eq!(program.declarations().len(), 5);
    let Declaration::Match(selector) = &program.declarations()[0] else {
        panic!("root must be Match");
    };
    let variants = selector
        .variants()
        .iter()
        .map(|variant| (variant.tag().as_str(), variant.entry_index()))
        .collect::<Vec<_>>();
    assert_eq!(variants, [("left", 1), ("right", 3)]);
    assert_eq!(state(&program, 1).next_index(), Some(2));
    assert_eq!(state(&program, 2).next_index(), None);
    assert_eq!(state(&program, 3).next_index(), Some(4));
    assert_eq!(state(&program, 4).next_index(), None);
}

#[test]
fn generic_tagging_metadata_order_and_match_atomicity_are_exact() {
    assert!(expand_program(entry(), &GenericExternalOperation).is_ok());
    assert!(expand_program(entry(), &GenericAdjacentOperation).is_ok());

    let unsorted = expand_program(entry(), &UnsortedMatchOperation).expect("unsorted callbacks");
    let Declaration::Match(selector) = &unsorted.declarations()[0] else {
        panic!("root Match");
    };
    assert_eq!(
        selector
            .variants()
            .iter()
            .map(|variant| (variant.tag().as_str(), variant.entry_index()))
            .collect::<Vec<_>>(),
        [("alpha", 2), ("zed", 1)]
    );
    assert_eq!(
        state(&unsorted, 1).state_implementation_ref(),
        &state_implementation_ref::<IdentityA>().expect("zed body")
    );
    assert_eq!(
        state(&unsorted, 2).state_implementation_ref(),
        &state_implementation_ref::<BtoA>().expect("alpha body")
    );

    for mode in 0..=3 {
        assert_eq!(
            expand_program(entry(), &BadMatch(mode)),
            Err(ProgramError::InvalidContract),
            "hostile Match mode {mode}"
        );
    }
    for result in [
        expand_program(entry(), &NonEnumOperation),
        expand_program(entry(), &InternalOperation),
        expand_program(entry(), &UnitOperation),
        expand_program(entry(), &NamedOperation),
        expand_program(entry(), &MultiFieldOperation),
    ] {
        assert_eq!(result, Err(ProgramError::InvalidContract));
    }
    assert!(expand_program(entry(), &LargeMatch::<1>).is_ok());
    let large = expand_program(entry(), &LargeMatch::<128>).expect("large Match");
    assert_eq!(large.declarations().len(), 129);
    // The existing 65,536-byte schema identity bound rejects this selector before
    // Program's raw 256-arm Match ceiling is reached.
    assert_eq!(
        expand_program(entry(), &LargeMatch::<257>),
        Err(ProgramError::InvalidContract)
    );
    let atomic = expand_program(entry(), &AtomicMatch).expect("failed Match is atomic");
    assert_eq!(
        atomic.canonical_bytes(),
        expand_program(entry(), &MatchOperation)
            .expect("ordinary Match")
            .canonical_bytes()
    );
    assert_eq!(
        expand_program(entry(), &RetryWrongJoin(true))
            .expect("wrong join can be retried")
            .canonical_bytes(),
        expand_program(entry(), &RetryWrongJoin(false))
            .expect("clean Match")
            .canonical_bytes()
    );
}

#[test]
fn child_relative_terminal_success_reopens_for_the_parent_continuation() {
    let child = expand_program(entry(), &EarlyChild).expect("early child root");
    assert_eq!(child.declarations().len(), 4);
    let program = expand_program(entry(), &EarlyParent).expect("early child success");
    assert_eq!(program.declarations().len(), 5);
    assert_eq!(state(&program, 1).next_index(), Some(3));
    assert_eq!(state(&program, 2).next_index(), Some(4));
    assert_eq!(state(&program, 3).next_index(), Some(4));
    assert_eq!(state(&program, 4).next_index(), None);
}

#[test]
fn failure_handlers_rejoin_or_terminate_without_entering_success_path() {
    let recovered = expand_program(entry(), &RecoveringHandler).expect("recovering handler");
    assert_eq!(recovered.declarations().len(), 3);
    assert_eq!(state(&recovered, 0).next_index(), Some(2));
    assert_eq!(state(&recovered, 0).failure_next_index(), Some(1));
    assert_eq!(state(&recovered, 1).next_index(), Some(2));
    assert_eq!(state(&recovered, 2).next_index(), None);

    let terminal = expand_program(entry(), &TerminalHandler).expect("terminal handler");
    assert_eq!(terminal.declarations().len(), 3);
    assert_eq!(state(&terminal, 0).next_index(), Some(2));
    assert_eq!(state(&terminal, 0).failure_next_index(), Some(1));
    assert_eq!(state(&terminal, 1).next_index(), None);
    assert_eq!(state(&terminal, 2).next_index(), None);
}

#[test]
fn nested_same_failure_uses_nearest_then_outer_handler_and_late_errors_are_atomic() {
    let nested = expand_program(entry(), &NestedSameFailure).expect("nested same-E handlers");
    assert_eq!(nested.declarations().len(), 4);
    assert_eq!(state(&nested, 0).next_index(), Some(3));
    assert_eq!(state(&nested, 0).failure_next_index(), Some(1));
    assert_eq!(state(&nested, 1).next_index(), Some(3));
    assert_eq!(state(&nested, 1).failure_next_index(), Some(2));
    assert_eq!(state(&nested, 2).next_index(), Some(3));
    assert_eq!(state(&nested, 2).failure_next_index(), None);
    assert_eq!(state(&nested, 3).next_index(), None);

    assert_eq!(
        expand_program(entry(), &SelfFailingHandler),
        Err(ProgramError::InvalidContract),
        "a handler cannot catch its own exact failure"
    );
    let atomic = expand_program(entry(), &AtomicHandler).expect("late handler error is atomic");
    assert_eq!(
        atomic.canonical_bytes(),
        expand_program(entry(), &RecoveringHandler)
            .expect("recovering handler")
            .canonical_bytes()
    );

    let equal = expand_program(entry(), &EqualRootFailure).expect("E equals root F");
    assert_eq!(state(&equal, 0).failure_next_index(), Some(1));
    assert_eq!(
        expand_program(entry(), &DirectProtectedOutput),
        Err(ProgramError::InvalidContract)
    );
    assert_eq!(
        expand_program(entry(), &EmptyHandler),
        Err(ProgramError::InvalidContract)
    );

    let protected_calls = Cell::new(0);
    let handler_calls = Cell::new(0);
    assert_eq!(
        expand_program(
            entry(),
            &NeverHandler {
                protected_calls: &protected_calls,
                handler_calls: &handler_calls,
            },
        ),
        Err(ProgramError::InvalidContract)
    );
    assert_eq!(protected_calls.get(), 0);
    assert_eq!(handler_calls.get(), 0);
}

#[test]
fn callback_depth_is_shared_across_child_match_handler_and_injection_scopes() {
    let accepted = Arc::new(DepthCounters::default());
    let accepted_program = expand_program(
        entry(),
        &MixedDepth {
            remaining: 59,
            setup: &accepted,
        },
    )
    .expect("callback depth 64");
    assert_eq!(accepted.before.load(Ordering::SeqCst), 1);
    assert_eq!(accepted.binding.load(Ordering::SeqCst), 1);
    assert_eq!(accepted.after.load(Ordering::SeqCst), 1);

    let Declaration::Match(selector) = &accepted_program.declarations()[0] else {
        panic!("root Match after erased child scopes");
    };
    assert_eq!(selector.variants()[0].entry_index(), 1);
    assert_eq!(selector.variants()[1].entry_index(), 5);
    assert_eq!(state(&accepted_program, 1).next_index(), Some(2));
    assert_eq!(state(&accepted_program, 2).next_index(), Some(3));
    assert_eq!(state(&accepted_program, 3).next_index(), Some(6));
    assert_eq!(state(&accepted_program, 1).failure_next_index(), None);
    assert_eq!(state(&accepted_program, 2).failure_next_index(), Some(4));
    assert_eq!(state(&accepted_program, 4).next_index(), Some(6));
    assert_eq!(state(&accepted_program, 5).next_index(), Some(6));
    assert_eq!(state(&accepted_program, 6).next_index(), None);

    let rejected = Arc::new(DepthCounters::default());
    assert_eq!(
        expand_program(
            entry(),
            &MixedDepth {
                remaining: 60,
                setup: &rejected,
            },
        ),
        Err(ProgramError::Capacity)
    );
    assert_eq!(rejected.before.load(Ordering::SeqCst), 0);
    assert_eq!(rejected.binding.load(Ordering::SeqCst), 0);
    assert_eq!(rejected.after.load(Ordering::SeqCst), 0);
}

#[test]
fn unavailable_authoring_surfaces_do_not_compile() {
    let tests = trybuild::TestCases::new();
    tests.compile_fail("tests/ui/removed_program_api.rs");
    tests.compile_fail("tests/ui/scoped_authoring.rs");
}
