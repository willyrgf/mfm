# mfm-program

Program owns deterministic typed source authoring and the one checked canonical
`mfm-program-document@2` graph. An `Operation` describes reusable authoring-only composition;
`OperationExpansion` lowers Pure States, exact-pair Reads, child Operations, structured Match
joins, and exact failure handlers through one private flat draft. Operation values, callbacks,
injection setup, and scope metadata are erased before Program construction.

```rust
use mfm_ids::{EntryPointId, StableId};
use mfm_program::{
    expand_program, Never, Operation, OperationExpansion, ProgramError, ProposedStateOutcome,
    PureState, State,
};
use mfm_program_derive::MfmValue;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct Count {
    value: u64,
}

struct Increment;

impl State for Increment {
    type Input = Count;
    type Output = Count;
    type Failure = Never;

    fn state_id() -> mfm_program::Result<StableId> {
        StableId::new("example.program/increment@1")
            .map_err(|_| ProgramError::InvalidContract)
    }
}

impl PureState for Increment {
    fn evaluate(input: Count) -> ProposedStateOutcome<Count, Never> {
        ProposedStateOutcome::Success {
            output: Count {
                value: input.value + 1,
            },
        }
    }
}

struct IncrementTwice;

impl Operation for IncrementTwice {
    type Input = Count;
    type Output = Count;
    type Failure = Never;

    fn expand(
        &self,
        body: &mut OperationExpansion<Self::Input, Self::Output, Self::Failure>,
    ) -> mfm_program::Result<()> {
        body.pure::<Increment>()?;
        body.pure::<Increment>()
    }
}

let program = expand_program(
    EntryPointId::new("example.program/increment-twice@1").expect("entry point"),
    &IncrementTwice,
)
.expect("valid Program");
assert_eq!(program.declarations().len(), 2);
```

Index zero is root. Final declarations are State or closed-sum Match. State successors are
optional forward `u16` indices; absence means that branch's exact root contract. Read declarations
retain capability, intent, evidence, and binding refs. `Program::decode_canonical` is the retained
wire ingress; raw declaration and Program source constructors are private.

Program is content addressed and has no catalog, registry, erased value, runtime implementation,
configuration contract, persisted Operation, or second wire DTO. Capability injection is
deterministic authoring-time topology only and grants no provider, signer, or mutation authority.
Operation implementations compose children only through `OperationExpansion`, and capability
policies emit support States only through `InjectionWriter`. Direct trait callback calls bypass
kernel callback accounting and are forbidden in reviewed production code. This trusted-code rule
is not a security or authorization boundary; checked Program construction remains the persisted
graph boundary.
