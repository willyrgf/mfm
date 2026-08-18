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

macro_rules! state {
    ($state:ty, $input:ty, $output:ty, $failure:ty, $id:literal) => {
        impl State for $state {
            type Input = $input;
            type Output = $output;
            type Failure = $failure;

            fn state_id() -> mfm_program::Result<StableId> {
                StableId::new($id).map_err(|_| ProgramError::InvalidContract)
            }
        }
    };
}

macro_rules! pure_state {
    ($state:ident, $input:ty, $output:ty, $failure:ty, $id:literal, |$value:ident| $evaluate:expr) => {
        struct $state;
        state!($state, $input, $output, $failure, $id);

        impl PureState for $state {
            fn evaluate($value: Self::Input) -> ProposedStateOutcome<Self::Output, Self::Failure> {
                $evaluate
            }
        }
    };
    ($state:ident, $input:ty, $output:ty, $failure:ty, $id:literal) => {
        struct $state;

        state!($state, $input, $output, $failure, $id);

        impl PureState for $state {
            fn evaluate(input: Self::Input) -> ProposedStateOutcome<Self::Output, Self::Failure> {
                ProposedStateOutcome::Success {
                    output: Self::Output { value: input.value },
                }
            }
        }
    };
}

macro_rules! operation {
    ($operation:ty, $input:ty, $output:ty, $failure:ty, |$this:ident, $body:ident| $expand:expr) => {
        impl Operation for $operation {
            type Input = $input;
            type Output = $output;
            type Failure = $failure;

            fn expand(
                &self,
                $body: &mut OperationExpansion<Self::Input, Self::Output, Self::Failure>,
            ) -> mfm_program::Result<()> {
                let $this = self;
                $expand
            }
        }
    };
}

macro_rules! read_capability {
    ($capability:ident, $intent:ty, $evidence:ty, $id:literal) => {
        struct $capability;

        impl ReadCapabilityContract for $capability {
            type Intent = $intent;
            type Evidence = $evidence;

            fn contract_id() -> mfm_capabilities::Result<StableId> {
                StableId::new($id).map_err(|_| CapabilityError::InvalidContract)
            }

            fn bind_evidence(
                _intent: &Self::Intent,
                _evidence: &Self::Evidence,
            ) -> mfm_capabilities::Result<()> {
                Ok(())
            }
        }
    };
}

macro_rules! read_state {
    ($state:ident, $capability:ty, $input:ty, $output:ty, $failure:ty, $id:literal, |$input_value:ident, $evidence:ident| $interpret:expr) => {
        struct $state;
        state!($state, $input, $output, $failure, $id);

        impl ReadState<$capability> for $state {
            fn prepare(
                input: &Self::Input,
            ) -> Result<<$capability as ReadCapabilityContract>::Intent, ReadPreparationError> {
                Ok(input.clone())
            }

            fn interpret(
                $input_value: Self::Input,
                $evidence: &<$capability as ReadCapabilityContract>::Evidence,
            ) -> ProposedStateOutcome<Self::Output, Self::Failure> {
                $interpret
            }
        }
    };
}

macro_rules! counted_value {
    ($value:ident, $descriptor_calls:ident, $semantic_calls:ident, $schema_name:literal, $semantic_name:literal, $digest_byte:literal) => {
        #[derive(Debug, Clone, Serialize, Deserialize)]
        #[serde(transparent)]
        struct $value(u8);

        impl mfm_values::MfmValue for $value {
            fn schema_descriptor() -> mfm_values::Result<mfm_values::SchemaDescriptor> {
                $descriptor_calls.fetch_add(1, Ordering::SeqCst);
                framework_value_descriptor(
                    "mfm-program",
                    Self::semantic_id()?,
                    $schema_name,
                    SchemaShape::UnsignedInteger { bits: 8 },
                    std::any::type_name::<Self>(),
                )
            }

            fn semantic_id() -> mfm_values::Result<SemanticTypeId> {
                $semantic_calls.fetch_add(1, Ordering::SeqCst);
                SemanticTypeId::new(
                    "mfm.test.authoring",
                    $semantic_name,
                    "1",
                    DigestAlgorithm::Sha256JcsV1,
                    DigestBytes::from_array([$digest_byte; 32]),
                )
                .map_err(|error| mfm_values::ValueError::Identity(error.to_string()))
            }
        }
    };
}

macro_rules! injection {
    (
        $capability:ty => $state:ty,
        $setup:ty,
        $input:ty => $output:ty,
        |$binding_setup:ident| $binding:expr,
        |$before_setup:ident, $before_writer:ident| $before:expr,
        |$after_setup:ident, $after_writer:ident| $after:expr
    ) => {
        impl CapabilityInjection<$state> for $capability {
            type Setup = $setup;
            type ExpandedInput = $input;
            type ExpandedOutput = $output;

            fn original_binding_ref(
                $binding_setup: &Self::Setup,
            ) -> mfm_program::Result<ContentRef> {
                $binding
            }

            fn write_before(
                $before_setup: &Self::Setup,
                $before_writer: &mut InjectionWriter,
            ) -> mfm_program::Result<()> {
                $before
            }

            fn write_after(
                $after_setup: &Self::Setup,
                $after_writer: &mut InjectionWriter,
            ) -> mfm_program::Result<()> {
                $after
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
pure_state!(
    FallibleAtoBWithD,
    A,
    B,
    D,
    "mfm.test.authoring/fallible-a-to-b-with-d@1"
);
pure_state!(
    FallibleBtoCWithFailure,
    B,
    C,
    Failure,
    "mfm.test.authoring/fallible-b-to-c@1"
);
pure_state!(DtoB, D, B, Never, "mfm.test.authoring/d-to-b@1");
pure_state!(
    FallibleIdentityB,
    B,
    B,
    Failure,
    "mfm.test.authoring/fallible-identity-b@1"
);
pure_state!(ForeignBtoC, B, C, A, "mfm.test.authoring/foreign-b-to-c@1");
pure_state!(
    SupportNever,
    A,
    A,
    Never,
    "mfm.test.authoring/support-never@1"
);
pure_state!(
    SupportHandled,
    A,
    A,
    Failure,
    "mfm.test.authoring/support-handled@1"
);

pure_state!(
    IdentityTiny,
    Tiny,
    Tiny,
    Never,
    "mfm.test.authoring/identity-tiny@1",
    |input| ProposedStateOutcome::Success { output: input }
);
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

pure_state!(
    FallibleAtoB,
    A,
    B,
    Failure,
    "mfm.test.authoring/fallible-a-to-b@1"
);

struct Empty;
operation!(Empty, A, A, Never, |_this, _body| Ok(()));

struct AtoBOperation;
operation!(AtoBOperation, A, B, Never, |_this, body| body
    .pure::<AtoB>());

struct Parent;
operation!(Parent, A, C, Never, |_this, body| {
    body.operation(&AtoBOperation)?;
    body.pure::<BtoC>()
});

struct CountedRoot<'a> {
    calls: &'a Cell<usize>,
    fail: bool,
}

operation!(CountedRoot<'_>, A, A, Never, |this, _body| {
    this.calls.set(this.calls.get() + 1);
    if this.fail {
        Err(ProgramError::Canonical)
    } else {
        Ok(())
    }
});

struct Recursive(u8);
operation!(Recursive, A, A, Never, |this, body| {
    if this.0 == 0 {
        body.pure::<IdentityA>()
    } else {
        body.operation(&Self(this.0 - 1))
    }
});

struct MutualA(u8);
struct MutualB(u8);

operation!(MutualA, A, A, Never, |this, body| {
    if this.0 == 0 {
        body.pure::<IdentityA>()
    } else {
        body.operation(&MutualB(this.0 - 1))
    }
});

operation!(MutualB, A, A, Never, |this, body| {
    if this.0 == 0 {
        body.pure::<IdentityA>()
    } else {
        body.operation(&MutualA(this.0 - 1))
    }
});

struct InvalidEmpty;
operation!(InvalidEmpty, A, B, Never, |_this, _body| Ok(()));

struct ConfiguredIdentity(usize);
operation!(ConfiguredIdentity, A, A, Never, |this, body| {
    for _ in 0..this.0 {
        body.pure::<IdentityA>()?;
    }
    Ok(())
});

struct RepeatedConfiguredChildren;
operation!(RepeatedConfiguredChildren, A, A, Never, |_this, body| {
    body.operation(&ConfiguredIdentity(1))?;
    body.operation(&ConfiguredIdentity(2))
});

struct CallbackErrorChild<'a>(&'a Cell<usize>);

operation!(CallbackErrorChild<'_>, A, A, Never, |this, body| {
    this.0.set(this.0.get() + 1);
    body.pure::<IdentityA>()?;
    Err(ProgramError::Canonical)
});

struct AtomicChildren;
operation!(AtomicChildren, A, A, Never, |_this, body| {
    assert_eq!(body.operation(&Empty), Err(ProgramError::InvalidContract));
    let calls = Cell::new(0);
    assert_eq!(
        body.operation(&CallbackErrorChild(&calls)),
        Err(ProgramError::InvalidContract)
    );
    assert_eq!(calls.get(), 1);
    body.pure::<IdentityA>()
});

struct WrongInputChild<'a>(&'a Cell<usize>);

operation!(WrongInputChild<'_>, B, B, Never, |this, _body| {
    this.0.set(this.0.get() + 1);
    panic!("wrong-input child callback must not run")
});

struct WrongOutputChild<'a>(&'a Cell<usize>);

operation!(WrongOutputChild<'_>, A, B, Never, |this, body| {
    this.0.set(this.0.get() + 1);
    body.pure::<IdentityA>()
});

struct ChildBoundaryMatrix<'a> {
    wrong_input_calls: &'a Cell<usize>,
    wrong_output_calls: &'a Cell<usize>,
}

operation!(ChildBoundaryMatrix<'_>, A, A, Never, |this, body| {
    assert_eq!(
        body.operation(&WrongInputChild(this.wrong_input_calls)),
        Err(ProgramError::InvalidContract)
    );
    assert_eq!(
        body.operation(&WrongOutputChild(this.wrong_output_calls)),
        Err(ProgramError::InvalidContract)
    );
    body.pure::<IdentityA>()
});

struct CatchChildDepth(bool);
operation!(CatchChildDepth, A, A, Never, |this, body| {
    if this.0 {
        assert_eq!(body.operation(&Recursive(63)), Err(ProgramError::Capacity));
    }
    body.operation(&Recursive(62))
});

read_capability!(ReadCapability, B, C, "mfm.test.authoring/read@1");
read_state!(
    ReadBtoC,
    ReadCapability,
    B,
    C,
    Failure,
    "mfm.test.authoring/read-b-to-c@1",
    |input, _evidence| ProposedStateOutcome::Success {
        output: C { value: input.value },
    }
);

pure_state!(Before, A, B, Failure, "mfm.test.authoring/before@1");
pure_state!(After, C, D, Never, "mfm.test.authoring/after@1");

injection!(
    ReadCapability => ReadBtoC,
    ContentRef,
    A => D,
    |setup| Ok(setup.clone()),
    |_setup, writer| writer.pure::<Before>(),
    |_setup, writer| writer.pure::<After>()
);

struct ReadOperation {
    binding: ContentRef,
}

read_capability!(
    ControlledReadCapability,
    A,
    A,
    "mfm.test.authoring/controlled-read@1"
);

read_state!(
    ControlledRead,
    ControlledReadCapability,
    A,
    A,
    Never,
    "mfm.test.authoring/controlled-read-state@1",
    |input, _evidence| ProposedStateOutcome::Success { output: input }
);
pure_state!(
    ForeignBefore,
    A,
    A,
    D,
    "mfm.test.authoring/foreign-before@1"
);

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

injection!(
    ControlledReadCapability => ControlledRead,
    HookCounts,
    A => A,
    |setup| {
        setup.binding.set(setup.binding.get() + 1);
        if matches!(setup.failure, HookFailure::Binding) {
            Err(ProgramError::Canonical)
        } else {
            nominal_contract_ref::<A>()
        }
    },
    |setup, writer| {
        setup.before.set(setup.before.get() + 1);
        if matches!(setup.failure, HookFailure::Before) {
            Err(ProgramError::Canonical)
        } else if matches!(setup.failure, HookFailure::ForeignFailure) {
            writer.pure::<ForeignBefore>()
        } else {
            writer.pure::<IdentityA>()
        }
    },
    |setup, writer| {
        setup.after.set(setup.after.get() + 1);
        if matches!(setup.failure, HookFailure::After) {
            Err(ProgramError::Canonical)
        } else {
            writer.pure::<IdentityA>()
        }
    }
);

struct AtomicRead<'a> {
    setup: &'a HookCounts,
}

operation!(AtomicRead<'_>, A, A, Never, |this, body| {
    assert_eq!(
        body.read::<ControlledRead, ControlledReadCapability>(this.setup),
        Err(ProgramError::InvalidContract)
    );
    body.pure::<IdentityA>()
});

struct RepeatedReads<'a> {
    setup: &'a HookCounts,
}

operation!(RepeatedReads<'_>, A, A, Never, |this, body| {
    body.read::<ControlledRead, ControlledReadCapability>(this.setup)?;
    body.read::<ControlledRead, ControlledReadCapability>(this.setup)
});

operation!(ReadOperation, A, D, Failure, |this, body| body
    .read::<ReadBtoC, ReadCapability>(
    &this.binding
));

read_capability!(
    HandledReadCapability,
    A,
    A,
    "mfm.test.authoring/handled-read@1"
);

read_state!(
    HandledRead,
    HandledReadCapability,
    A,
    A,
    Failure,
    "mfm.test.authoring/handled-read-state@1",
    |input, _evidence| ProposedStateOutcome::Success { output: input }
);

injection!(
    HandledReadCapability => HandledRead,
    ContentRef,
    A => A,
    |setup| Ok(setup.clone()),
    |_setup, writer| {
        writer.pure::<SupportNever>()?;
        writer.pure::<SupportHandled>()
    },
    |_setup, writer| {
        writer.pure::<SupportNever>()
    }
);

struct HandledInjectedRead {
    binding: ContentRef,
}

operation!(HandledInjectedRead, A, B, Never, |this, body| {
    body.with_failure_handler::<Failure, A>(
        |protected| protected.read::<HandledRead, HandledReadCapability>(&this.binding),
        |handler| handler.pure::<FailureToA>(),
    )?;
    body.pure::<AtoB>()
});

#[derive(Debug, Clone, Serialize, Deserialize, MfmValue)]
#[serde(rename_all = "snake_case")]
enum Choice {
    Left(A),
    Right(B),
}

#[derive(Debug, Clone, Serialize, Deserialize, MfmValue)]
#[serde(rename_all = "snake_case")]
enum HandlerChoice {
    Recover(Failure),
}

pure_state!(
    FallibleAtoBWithHandlerChoice,
    A,
    B,
    HandlerChoice,
    "mfm.test.authoring/fallible-a-to-b-with-handler-choice@1"
);

pure_state!(
    HandlerChoiceToA,
    HandlerChoice,
    A,
    Never,
    "mfm.test.authoring/handler-choice-to-a@1",
    |input| {
        let HandlerChoice::Recover(failure) = input;
        ProposedStateOutcome::Success {
            output: A {
                value: failure.value,
            },
        }
    }
);

pure_state!(
    HandlerChoiceToBForeign,
    HandlerChoice,
    B,
    D,
    "mfm.test.authoring/handler-choice-to-b-foreign@1",
    |input| {
        let HandlerChoice::Recover(failure) = input;
        ProposedStateOutcome::Success {
            output: B {
                value: failure.value,
            },
        }
    }
);

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
#[serde(tag = "kind", content = "payload", rename_all = "snake_case")]
enum AdjacentChoice {
    Left(A),
    Right(B),
}

pure_state!(
    GenericToA,
    GenericExternal<A>,
    A,
    Never,
    "mfm.test.authoring/generic-to-a@1",
    |input| {
        let GenericExternal::Selected(output) = input;
        ProposedStateOutcome::Success { output }
    }
);

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
operation!(MatchOperation, Choice, C, Never, |_this, body| {
    body.match_join::<Choice, C>(|arms| {
        arms.arm::<A>(tag("left")?, |arm| {
            arm.operation(&AtoBOperation)?;
            arm.pure::<BtoC>()
        })?;
        arms.arm::<B>(tag("right")?, |arm| {
            arm.pure::<BtoD>()?;
            arm.pure::<DtoC>()
        })
    })
});

macro_rules! selected_match_operation {
    ($operation:ident, $selector:ty) => {
        struct $operation;
        operation!($operation, $selector, A, Never, |_this, body| {
            body.match_join::<$selector, A>(|arms| {
                arms.arm::<A>(tag("selected")?, |arm| arm.pure::<IdentityA>())
            })
        });
    };
}

selected_match_operation!(GenericExternalOperation, GenericExternal<A>);
selected_match_operation!(GenericAdjacentOperation, GenericAdjacent<A>);

struct AdjacentChoiceOperation;
operation!(
    AdjacentChoiceOperation,
    AdjacentChoice,
    C,
    Never,
    |_this, body| {
        body.match_join::<AdjacentChoice, C>(|arms| {
            arms.arm::<A>(tag("left")?, |arm| {
                arm.pure::<AtoB>()?;
                arm.pure::<BtoC>()
            })?;
            arms.arm::<B>(tag("right")?, |arm| arm.pure::<BtoC>())
        })
    }
);

struct RetryMatchFirst(bool);
operation!(
    RetryMatchFirst,
    GenericExternal<GenericExternal<A>>,
    A,
    Never,
    |this, body| {
        body.match_join::<GenericExternal<GenericExternal<A>>, A>(|arms| {
            let selected = tag("selected")?;
            if this.0 {
                assert_eq!(
                    arms.arm::<GenericExternal<A>>(selected.clone(), |branch| {
                        branch.match_join::<GenericExternal<A>, A>(|nested| {
                            nested.arm::<A>(selected.clone(), |arm| arm.pure::<IdentityA>())
                        })
                    }),
                    Err(ProgramError::InvalidContract)
                );
            }
            arms.arm::<GenericExternal<A>>(selected, |branch| branch.pure::<GenericToA>())
        })
    }
);

struct AllTerminalMatch;
operation!(AllTerminalMatch, Choice, C, Never, |_this, body| {
    body.match_join::<Choice, B>(|arms| {
        arms.arm::<A>(tag("left")?, |arm| {
            arm.pure::<AtoB>()?;
            arm.pure::<BtoC>()
        })?;
        arms.arm::<B>(tag("right")?, |arm| arm.pure::<BtoC>())
    })
});

struct UnsortedMatchOperation;
operation!(
    UnsortedMatchOperation,
    UnsortedChoice,
    A,
    Never,
    |_this, body| {
        body.match_join::<UnsortedChoice, A>(|arms| {
            arms.arm::<A>(tag("zed")?, |arm| arm.pure::<IdentityA>())?;
            arms.arm::<B>(tag("alpha")?, |arm| arm.pure::<BtoA>())
        })
    }
);

struct BadMatch(u8);
operation!(BadMatch, Choice, C, Never, |this, body| {
    body.match_join::<Choice, C>(|arms| match this.0 {
        0 => arms.arm::<A>(tag("left")?, |arm| {
            arm.pure::<AtoB>()?;
            arm.pure::<BtoC>()
        }),
        1 => arms.arm::<A>(tag("unknown")?, |_| {
            panic!("unknown arm callback must not run")
        }),
        2 => {
            let left = tag("left")?;
            arms.arm::<A>(left.clone(), |arm| {
                arm.pure::<AtoB>()?;
                arm.pure::<BtoC>()
            })?;
            arms.arm::<A>(left, |_| panic!("duplicate arm callback must not run"))
        }
        3 => arms.arm::<B>(tag("left")?, |_| {
            panic!("wrong payload callback must not run")
        }),
        _ => unreachable!(),
    })
});

struct AtomicMatch;
operation!(AtomicMatch, Choice, C, Never, |_this, body| {
    assert_eq!(
        body.match_join::<Choice, C>(|arms| {
            arms.arm::<A>(tag("left")?, |arm| {
                arm.pure::<AtoB>()?;
                arm.pure::<BtoC>()
            })?;
            Err(ProgramError::Canonical)
        }),
        Err(ProgramError::InvalidContract)
    );
    body.operation(&MatchOperation)
});

macro_rules! unsupported_selector_operation {
    ($operation:ident, $selector:ty) => {
        struct $operation;
        operation!($operation, $selector, A, Never, |_this, body| {
            body.match_join::<$selector, A>(|_| {
                panic!("unsupported selector callback must not run")
            })
        });
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
operation!(EarlyChild, Choice, C, Never, |_this, body| {
    body.match_join::<Choice, B>(|arms| {
        arms.arm::<A>(tag("left")?, |arm| arm.pure::<AtoB>())?;
        arms.arm::<B>(tag("right")?, |arm| arm.pure::<BtoC>())
    })?;
    body.pure::<BtoC>()
});

struct EarlyParent;
operation!(EarlyParent, Choice, D, Never, |_this, body| {
    body.operation(&EarlyChild)?;
    body.pure::<CtoD>()
});

macro_rules! handler_operation {
    ($operation:ident, $handler:ty) => {
        struct $operation;
        operation!($operation, A, C, Never, |_this, body| {
            body.with_failure_handler::<Failure, B>(
                |protected| protected.pure::<FallibleAtoB>(),
                |handler| handler.pure::<$handler>(),
            )?;
            body.pure::<BtoC>()
        });
    };
}

handler_operation!(RecoveringHandler, FailureToB);
handler_operation!(TerminalHandler, FailureToC);

pure_state!(
    FallibleFailureToB,
    Failure,
    B,
    Failure,
    "mfm.test.authoring/fallible-failure-to-b@1",
    |input| ProposedStateOutcome::Failure { failure: input }
);

pure_state!(
    FallibleAtoC,
    A,
    C,
    Failure,
    "mfm.test.authoring/fallible-a-to-c@1"
);

struct NestedSameFailure;
operation!(NestedSameFailure, A, C, Never, |_this, body| {
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
});

struct SelfFailingHandler;
operation!(SelfFailingHandler, A, B, Never, |_this, body| {
    body.with_failure_handler::<Failure, B>(
        |protected| protected.pure::<FallibleAtoB>(),
        |handler| handler.pure::<FallibleFailureToB>(),
    )
});

struct AtomicHandler;
operation!(AtomicHandler, A, C, Never, |_this, body| {
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
});

struct EqualRootFailure;
operation!(EqualRootFailure, A, C, Failure, |_this, body| {
    body.with_failure_handler::<Failure, B>(
        |protected| protected.pure::<FallibleAtoB>(),
        |handler| handler.pure::<FailureToB>(),
    )?;
    body.pure::<BtoC>()
});

struct DirectProtectedOutput;
operation!(DirectProtectedOutput, A, C, Never, |_this, body| {
    body.with_failure_handler::<Failure, B>(
        |protected| protected.pure::<FallibleAtoC>(),
        |handler| handler.pure::<FailureToB>(),
    )
});

struct EmptyHandler;
operation!(EmptyHandler, A, B, Never, |_this, body| {
    body.with_failure_handler::<Failure, B>(
        |protected| protected.pure::<FallibleAtoB>(),
        |_| Ok(()),
    )
});

struct NeverHandler<'a> {
    protected_calls: &'a Cell<usize>,
    handler_calls: &'a Cell<usize>,
}

operation!(NeverHandler<'_>, A, A, Never, |this, body| {
    body.with_failure_handler::<Never, A>(
        |_| {
            this.protected_calls.set(this.protected_calls.get() + 1);
            Ok(())
        },
        |_| {
            this.handler_calls.set(this.handler_calls.get() + 1);
            Ok(())
        },
    )
});

enum HandlerMatrix<'a> {
    NoReachable(&'a Cell<usize>),
    OuterFailureBypass,
    ForeignProtected,
    Distinct,
    Bad(u8),
}

operation!(HandlerMatrix<'_>, A, C, Failure, |this, body| {
    match this {
        Self::NoReachable(handler_calls) => body.with_failure_handler::<D, C>(
            |protected| {
                protected.pure::<AtoB>()?;
                protected.pure::<BtoC>()
            },
            |handler| {
                handler_calls.set(handler_calls.get() + 1);
                handler.pure::<DtoC>()
            },
        ),
        Self::OuterFailureBypass => body.with_failure_handler::<D, C>(
            |protected| {
                protected.pure::<FallibleAtoBWithD>()?;
                protected.pure::<FallibleBtoCWithFailure>()
            },
            |handler| handler.pure::<DtoC>(),
        ),
        Self::ForeignProtected => body.with_failure_handler::<D, C>(
            |protected| {
                protected.pure::<FallibleAtoBWithD>()?;
                protected.pure::<ForeignBtoC>()
            },
            |handler| handler.pure::<DtoC>(),
        ),
        Self::Distinct => {
            body.with_failure_handler::<Failure, B>(
                |outer| {
                    outer.with_failure_handler::<D, B>(
                        |inner| inner.pure::<FallibleAtoBWithD>(),
                        |inner_handler| inner_handler.pure::<DtoB>(),
                    )?;
                    outer.pure::<FallibleIdentityB>()
                },
                |outer_handler| outer_handler.pure::<FailureToB>(),
            )?;
            body.pure::<BtoC>()
        }
        Self::Bad(mode) => {
            body.with_failure_handler::<HandlerChoice, B>(
                |protected| protected.pure::<FallibleAtoBWithHandlerChoice>(),
                |handler| match mode {
                    0 => handler.match_join::<HandlerChoice, B>(|arms| {
                        arms.arm::<Failure>(tag("recover")?, |arm| arm.pure::<FailureToB>())
                    }),
                    1 => handler.pure::<FailureToB>(),
                    2 => handler.pure::<HandlerChoiceToA>(),
                    3 => handler.pure::<HandlerChoiceToBForeign>(),
                    _ => unreachable!(),
                },
            )?;
            body.pure::<BtoC>()
        }
    }
});

read_capability!(
    DepthCapability,
    A,
    A,
    "mfm.test.authoring/depth-capability@1"
);

read_state!(
    DepthRead,
    DepthCapability,
    A,
    A,
    Failure,
    "mfm.test.authoring/depth-read@1",
    |input, _evidence| ProposedStateOutcome::Success { output: input }
);

#[derive(Default)]
struct DepthCounters {
    before: AtomicUsize,
    binding: AtomicUsize,
    after: AtomicUsize,
}

injection!(
    DepthCapability => DepthRead,
    Arc<DepthCounters>,
    A => A,
    |setup| {
        setup.binding.fetch_add(1, Ordering::SeqCst);
        nominal_contract_ref::<A>()
    },
    |setup, writer| {
        setup.before.fetch_add(1, Ordering::SeqCst);
        writer.pure::<IdentityA>()
    },
    |setup, writer| {
        setup.after.fetch_add(1, Ordering::SeqCst);
        writer.pure::<IdentityA>()
    }
);

struct MixedDepth<'a> {
    remaining: u8,
    setup: &'a Arc<DepthCounters>,
}

operation!(MixedDepth<'_>, Choice, A, Never, |this, body| {
    if this.remaining > 0 {
        return body.operation(&MixedDepth {
            remaining: this.remaining - 1,
            setup: this.setup,
        });
    }
    body.match_join::<Choice, A>(|arms| {
        arms.arm::<A>(tag("left")?, |branch| {
            branch.with_failure_handler::<Failure, A>(
                |protected| protected.read::<DepthRead, DepthCapability>(this.setup),
                |handler| handler.pure::<FailureToA>(),
            )
        })?;
        arms.arm::<B>(tag("right")?, |branch| branch.pure::<BtoA>())
    })?;
    body.pure::<IdentityA>()
});

static DIRECT_DESCRIPTOR_CALLS: AtomicUsize = AtomicUsize::new(0);
static DIRECT_SEMANTIC_CALLS: AtomicUsize = AtomicUsize::new(0);
static DIRECT_STATE_CALLS: AtomicUsize = AtomicUsize::new(0);
static MEMO_DESCRIPTOR_CALLS: AtomicUsize = AtomicUsize::new(0);
static MEMO_SEMANTIC_CALLS: AtomicUsize = AtomicUsize::new(0);
static MEMO_STATE_CALLS: AtomicUsize = AtomicUsize::new(0);
static MEMO_CAPABILITY_CALLS: AtomicUsize = AtomicUsize::new(0);

counted_value!(
    DirectCountedValue,
    DIRECT_DESCRIPTOR_CALLS,
    DIRECT_SEMANTIC_CALLS,
    "mfm.test.authoring.direct-counted-value",
    "direct-counted-value",
    0x41
);

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
operation!(
    DirectCountedOperation,
    DirectCountedValue,
    DirectCountedValue,
    Never,
    |_this, body| {
        body.pure::<DirectCountedState>()?;
        body.pure::<DirectCountedState>()
    }
);

counted_value!(
    MemoValue,
    MEMO_DESCRIPTOR_CALLS,
    MEMO_SEMANTIC_CALLS,
    "mfm.test.authoring.memo-value",
    "memo-value",
    0x42
);

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

pure_state!(
    MemoFallibleIdentity,
    MemoValue,
    MemoValue,
    MemoValue,
    "mfm.test.authoring/memo-fallible-identity@1",
    |input| ProposedStateOutcome::Success { output: input }
);

struct MemoChild;
operation!(MemoChild, MemoValue, MemoValue, Never, |_this, body| {
    body.pure::<MemoIdentity>()
});

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

read_state!(
    MemoRead,
    MemoCapability,
    MemoValue,
    MemoValue,
    Never,
    "mfm.test.authoring/memo-read@1",
    |input, _evidence| ProposedStateOutcome::Success { output: input }
);

injection!(
    MemoCapability => MemoRead,
    ContentRef,
    MemoValue => MemoValue,
    |setup| Ok(setup.clone()),
    |_setup, writer| writer.pure::<MemoIdentity>(),
    |_setup, writer| writer.pure::<MemoIdentity>()
);

struct MemoAcrossScopes {
    binding: ContentRef,
}

operation!(
    MemoAcrossScopes,
    MemoChoice,
    MemoValue,
    Never,
    |this, body| {
        body.match_join::<MemoChoice, MemoValue>(|arms| {
            arms.arm::<MemoValue>(tag("selected")?, |arm| arm.operation(&MemoChild))
        })?;
        body.with_failure_handler::<MemoValue, MemoValue>(
            |protected| protected.pure::<MemoFallibleIdentity>(),
            |handler| handler.pure::<MemoIdentity>(),
        )?;
        body.read::<MemoRead, MemoCapability>(&this.binding)?;
        body.read::<MemoRead, MemoCapability>(&this.binding)
    }
);

struct RetryWrongJoin(bool);
operation!(RetryWrongJoin, Choice, C, Never, |this, body| {
    body.match_join::<Choice, C>(|arms| {
        let left = tag("left")?;
        if this.0 {
            assert_eq!(
                arms.arm::<A>(left.clone(), |_| Ok(())),
                Err(ProgramError::InvalidContract)
            );
            assert_eq!(
                arms.arm::<A>(left.clone(), |arm| arm.pure::<AtoB>()),
                Err(ProgramError::InvalidContract)
            );
        }
        arms.arm::<A>(left, |arm| {
            arm.pure::<AtoB>()?;
            arm.pure::<BtoC>()
        })?;
        arms.arm::<B>(tag("right")?, |arm| arm.pure::<BtoC>())
    })
});

struct WrongTerminalOutput;
operation!(WrongTerminalOutput, A, C, Never, |_this, body| body
    .pure::<AtoB>());

fn entry() -> EntryPointId {
    EntryPointId::new("mfm.test/authoring@1").expect("entry")
}

fn tag(value: &str) -> mfm_program::Result<StableId> {
    StableId::new(value).map_err(|_| ProgramError::InvalidContract)
}

fn valid_program<O: Operation>(operation: &O) -> mfm_program::Program {
    expand_program(entry(), operation).expect("valid Program")
}

fn program_error<O: Operation>(operation: &O) -> ProgramError {
    expand_program(entry(), operation).expect_err("invalid Program")
}

fn program_bytes<O: Operation>(operation: &O) -> Vec<u8> {
    valid_program(operation).canonical_bytes().to_vec()
}

fn assert_invalid<O: Operation>(operation: &O) {
    assert_eq!(program_error(operation), ProgramError::InvalidContract);
}

fn assert_same_program<L: Operation, R: Operation>(left: &L, right: &R) {
    assert_eq!(program_bytes(left), program_bytes(right));
}

macro_rules! assert_states {
    ($program:expr, $($state:ty),+ $(,)?) => {{
        let actual = $program
            .declarations()
            .iter()
            .filter_map(|declaration| match declaration {
                Declaration::State(state) => Some(state.state_implementation_ref().clone()),
                Declaration::Match(_) => None,
            })
            .collect::<Vec<_>>();
        let expected = vec![$(state_implementation_ref::<$state>().expect("State ref")),+];
        assert_eq!(actual, expected);
    }};
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
    let empty = valid_program(&CountedRoot {
        calls: &calls,
        fail: false,
    });
    assert!(empty.declarations().is_empty());
    assert_eq!(calls.get(), 1);
    assert_eq!(
        program_error(&CountedRoot {
            calls: &calls,
            fail: true,
        }),
        ProgramError::InvalidContract
    );
    assert_eq!(calls.get(), 2);

    let parent = valid_program(&Parent);
    assert_eq!(parent.declarations().len(), 2);
    assert_eq!(state(&parent, 0).next_index(), Some(1));
    assert_eq!(state(&parent, 1).next_index(), None);
    valid_program(&Recursive(63));
    assert_eq!(program_error(&Recursive(64)), ProgramError::Capacity);
    valid_program(&MutualA(63));
    assert_eq!(program_error(&MutualA(64)), ProgramError::Capacity);
    valid_program(&Empty);
    assert_invalid(&InvalidEmpty);
    assert_invalid(&WrongTerminalOutput);
}

#[test]
fn one_expansion_owns_one_top_level_identity_memo() {
    DIRECT_DESCRIPTOR_CALLS.store(0, Ordering::SeqCst);
    DIRECT_SEMANTIC_CALLS.store(0, Ordering::SeqCst);
    DIRECT_STATE_CALLS.store(0, Ordering::SeqCst);

    valid_program(&DirectCountedOperation);
    assert_eq!(DIRECT_DESCRIPTOR_CALLS.load(Ordering::SeqCst), 1);
    assert_eq!(DIRECT_SEMANTIC_CALLS.load(Ordering::SeqCst), 2);
    assert_eq!(DIRECT_STATE_CALLS.load(Ordering::SeqCst), 1);

    valid_program(&DirectCountedOperation);
    assert_eq!(DIRECT_DESCRIPTOR_CALLS.load(Ordering::SeqCst), 2);
    assert_eq!(DIRECT_SEMANTIC_CALLS.load(Ordering::SeqCst), 4);
    assert_eq!(DIRECT_STATE_CALLS.load(Ordering::SeqCst), 2);

    MEMO_DESCRIPTOR_CALLS.store(0, Ordering::SeqCst);
    MEMO_SEMANTIC_CALLS.store(0, Ordering::SeqCst);
    MEMO_STATE_CALLS.store(0, Ordering::SeqCst);
    MEMO_CAPABILITY_CALLS.store(0, Ordering::SeqCst);
    let program = valid_program(&MemoAcrossScopes {
        binding: nominal_contract_ref::<A>().expect("binding"),
    });
    assert_eq!(program.declarations().len(), 10);
    assert_eq!(MEMO_DESCRIPTOR_CALLS.load(Ordering::SeqCst), 2);
    assert_eq!(MEMO_SEMANTIC_CALLS.load(Ordering::SeqCst), 4);
    assert_eq!(MEMO_STATE_CALLS.load(Ordering::SeqCst), 1);
    assert_eq!(MEMO_CAPABILITY_CALLS.load(Ordering::SeqCst), 1);
}

#[test]
fn configured_children_repeat_without_parent_size_knowledge_and_fail_atomically() {
    let repeated = valid_program(&RepeatedConfiguredChildren);
    assert_eq!(repeated.declarations().len(), 3);
    assert_eq!(state(&repeated, 0).next_index(), Some(1));
    assert_eq!(state(&repeated, 1).next_index(), Some(2));
    assert_eq!(state(&repeated, 2).next_index(), None);

    let atomic = valid_program(&AtomicChildren);
    assert_eq!(atomic.declarations().len(), 1);
    assert_states!(&atomic, IdentityA);
    assert_same_program(&AtomicChildren, &ConfiguredIdentity(1));

    let wrong_input_calls = Cell::new(0);
    let wrong_output_calls = Cell::new(0);
    let boundaries = valid_program(&ChildBoundaryMatrix {
        wrong_input_calls: &wrong_input_calls,
        wrong_output_calls: &wrong_output_calls,
    });
    assert_eq!(wrong_input_calls.get(), 0);
    assert_eq!(wrong_output_calls.get(), 1);
    assert_eq!(
        boundaries.canonical_bytes(),
        program_bytes(&ConfiguredIdentity(1))
    );
    assert_same_program(&CatchChildDepth(true), &CatchChildDepth(false));
}

#[test]
fn nonempty_injection_wraps_one_kernel_owned_read_in_exact_order() {
    let binding = nominal_contract_ref::<A>().expect("binding");
    let program = valid_program(&ReadOperation {
        binding: binding.clone(),
    });
    assert_eq!(program.declarations().len(), 3);
    assert_states!(&program, Before, ReadBtoC, After);
    assert!(state(&program, 0).execution().is_pure());
    assert_eq!(state(&program, 1).execution().binding_ref(), Some(&binding));
    assert!(state(&program, 2).execution().is_pure());
    assert_eq!(state(&program, 0).next_index(), Some(1));
    assert_eq!(state(&program, 1).next_index(), Some(2));
    assert_eq!(state(&program, 2).next_index(), None);
    assert_eq!(state(&program, 0).failure_next_index(), None);
    assert_eq!(state(&program, 1).failure_next_index(), None);

    let handled = valid_program(&HandledInjectedRead {
        binding: nominal_contract_ref::<A>().expect("handled binding"),
    });
    assert_eq!(handled.declarations().len(), 6);
    assert_states!(
        &handled,
        SupportNever,
        SupportHandled,
        HandledRead,
        SupportNever,
        FailureToA,
        AtoB,
    );
    assert_eq!(
        state(&handled, 1).failure_contract_ref(),
        state(&handled, 2).failure_contract_ref()
    );
    assert_eq!(state(&handled, 1).failure_next_index(), Some(4));
    assert_eq!(state(&handled, 2).failure_next_index(), Some(4));
    assert_eq!(state(&handled, 3).next_index(), Some(5));
    assert_eq!(state(&handled, 4).next_index(), Some(5));
}

#[test]
fn repeated_injection_is_exact_and_hook_failures_are_once_short_circuited_and_atomic() {
    let successful = HookCounts::new(HookFailure::None);
    let first = valid_program(&RepeatedReads { setup: &successful });
    assert_eq!(first.declarations().len(), 6);
    assert_eq!(successful.before.get(), 2);
    assert_eq!(successful.binding.get(), 2);
    assert_eq!(successful.after.get(), 2);
    let second_setup = HookCounts::new(HookFailure::None);
    let second = valid_program(&RepeatedReads {
        setup: &second_setup,
    });
    assert_eq!(first.canonical_bytes(), second.canonical_bytes());
    assert_eq!(first.content_ref(), second.content_ref());

    for (failure, expected) in [
        (HookFailure::Before, (1, 0, 0)),
        (HookFailure::ForeignFailure, (1, 0, 0)),
        (HookFailure::Binding, (1, 1, 0)),
        (HookFailure::After, (1, 1, 1)),
    ] {
        let setup = HookCounts::new(failure);
        let program = valid_program(&AtomicRead { setup: &setup });
        assert_eq!(program.declarations().len(), 1);
        assert_eq!(
            (setup.before.get(), setup.binding.get(), setup.after.get()),
            expected
        );
    }
}

#[test]
fn structured_match_keeps_physical_arm_order_and_exact_join() {
    let program = valid_program(&MatchOperation);
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

    let adjacent = valid_program(&AdjacentChoiceOperation);
    assert_eq!(adjacent.declarations().len(), 4);
    assert_same_program(&RetryMatchFirst(true), &RetryMatchFirst(false));

    let terminal = valid_program(&AllTerminalMatch);
    assert_eq!(terminal.declarations().len(), 4);
    assert_eq!(state(&terminal, 2).next_index(), None);
    assert_eq!(state(&terminal, 3).next_index(), None);
}

#[test]
fn generic_tagging_metadata_order_and_match_atomicity_are_exact() {
    valid_program(&GenericExternalOperation);
    valid_program(&GenericAdjacentOperation);

    let unsorted = valid_program(&UnsortedMatchOperation);
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
    assert_states!(&unsorted, IdentityA, BtoA);

    for mode in 0..=3 {
        assert_invalid(&BadMatch(mode));
    }
    for result in [
        program_error(&NonEnumOperation),
        program_error(&InternalOperation),
        program_error(&UnitOperation),
        program_error(&NamedOperation),
        program_error(&MultiFieldOperation),
    ] {
        assert_eq!(result, ProgramError::InvalidContract);
    }
    valid_program(&LargeMatch::<1>);
    let large = valid_program(&LargeMatch::<128>);
    assert_eq!(large.declarations().len(), 129);
    // The existing 65,536-byte schema identity bound rejects this selector before
    // Program's raw 256-arm Match ceiling is reached.
    assert_invalid(&LargeMatch::<257>);
    assert_same_program(&AtomicMatch, &MatchOperation);
    assert_same_program(&RetryWrongJoin(true), &RetryWrongJoin(false));
}

#[test]
fn child_relative_terminal_success_reopens_for_the_parent_continuation() {
    let child = valid_program(&EarlyChild);
    assert_eq!(child.declarations().len(), 4);
    let program = valid_program(&EarlyParent);
    assert_eq!(program.declarations().len(), 5);
    assert_eq!(state(&program, 1).next_index(), Some(3));
    assert_eq!(state(&program, 2).next_index(), Some(4));
    assert_eq!(state(&program, 3).next_index(), Some(4));
    assert_eq!(state(&program, 4).next_index(), None);
}

#[test]
fn failure_handlers_rejoin_or_terminate_without_entering_success_path() {
    let recovered = valid_program(&RecoveringHandler);
    assert_eq!(recovered.declarations().len(), 3);
    assert_eq!(state(&recovered, 0).next_index(), Some(2));
    assert_eq!(state(&recovered, 0).failure_next_index(), Some(1));
    assert_eq!(state(&recovered, 1).next_index(), Some(2));
    assert_eq!(state(&recovered, 2).next_index(), None);

    let terminal = valid_program(&TerminalHandler);
    assert_eq!(terminal.declarations().len(), 3);
    assert_eq!(state(&terminal, 0).next_index(), Some(2));
    assert_eq!(state(&terminal, 0).failure_next_index(), Some(1));
    assert_eq!(state(&terminal, 1).next_index(), None);
    assert_eq!(state(&terminal, 2).next_index(), None);
}

#[test]
fn nested_same_failure_uses_nearest_then_outer_handler_and_late_errors_are_atomic() {
    let nested = valid_program(&NestedSameFailure);
    assert_eq!(nested.declarations().len(), 4);
    assert_eq!(state(&nested, 0).next_index(), Some(3));
    assert_eq!(state(&nested, 0).failure_next_index(), Some(1));
    assert_eq!(state(&nested, 1).next_index(), Some(3));
    assert_eq!(state(&nested, 1).failure_next_index(), Some(2));
    assert_eq!(state(&nested, 2).next_index(), Some(3));
    assert_eq!(state(&nested, 2).failure_next_index(), None);
    assert_eq!(state(&nested, 3).next_index(), None);

    assert_invalid(&SelfFailingHandler);
    assert_same_program(&AtomicHandler, &RecoveringHandler);

    let equal = valid_program(&EqualRootFailure);
    assert_eq!(state(&equal, 0).failure_next_index(), Some(1));
    assert_invalid(&DirectProtectedOutput);
    assert_invalid(&EmptyHandler);

    let protected_calls = Cell::new(0);
    let handler_calls = Cell::new(0);
    assert_eq!(
        program_error(&NeverHandler {
            protected_calls: &protected_calls,
            handler_calls: &handler_calls,
        }),
        ProgramError::InvalidContract
    );
    assert_eq!(protected_calls.get(), 0);
    assert_eq!(handler_calls.get(), 0);

    let unreachable_handler_calls = Cell::new(0);
    assert_invalid(&HandlerMatrix::NoReachable(&unreachable_handler_calls));
    assert_eq!(unreachable_handler_calls.get(), 0);

    let bypass = valid_program(&HandlerMatrix::OuterFailureBypass);
    assert_eq!(bypass.declarations().len(), 3);
    assert_eq!(state(&bypass, 0).failure_next_index(), Some(2));
    assert_eq!(state(&bypass, 1).failure_next_index(), None);
    assert_invalid(&HandlerMatrix::ForeignProtected);

    let distinct = valid_program(&HandlerMatrix::Distinct);
    assert_eq!(distinct.declarations().len(), 5);
    assert_eq!(state(&distinct, 0).failure_next_index(), Some(1));
    assert_eq!(state(&distinct, 2).failure_next_index(), Some(3));
    assert_eq!(state(&distinct, 1).next_index(), Some(2));
    assert_eq!(state(&distinct, 3).next_index(), Some(4));

    for mode in 0..=3 {
        assert_invalid(&HandlerMatrix::Bad(mode));
    }
}

#[test]
fn callback_depth_is_shared_across_child_match_handler_and_injection_scopes() {
    let accepted = Arc::new(DepthCounters::default());
    let accepted_program = valid_program(&MixedDepth {
        remaining: 59,
        setup: &accepted,
    });
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
        program_error(&MixedDepth {
            remaining: 60,
            setup: &rejected,
        }),
        ProgramError::Capacity
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
    tests.compile_fail("tests/ui/scoped_escape.rs");
}
