# mfm-program

Program owns the immutable `mfm-program-document@5` sequence with the `mfm.program.v5` domain.
An Operation performs deterministic source authoring; its input check and expansion commit the
exact initial value, State contracts, resolved policies, root failure maps, checkpoints and finite
bounds. Runtime associates typed implementations and owns execution and recovery. Old graph bytes
are rejected.

```rust
use mfm_ids::{EntryPointId, StableId};
use mfm_program::{
    expand_program, ConclusionBound, Identity, Never, NoParams, Occurrence, Operation,
    OperationExpansion, ProgramError, ProgramLimits, ProposedStateOutcome, PureState, State,
    StateExecutionError,
};
use mfm_program_derive::MfmValue;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct Count { value: u64 }
struct Increment;
impl State for Increment {
    type Input = Count;
    type Output = Count;
    type Failure = Never;
    fn state_id() -> mfm_program::Result<StableId> {
        StableId::new("mfm.example.increment@1").map_err(|_| ProgramError::InvalidContract)
    }
}
impl PureState for Increment {
    fn evaluate(input: Count) -> Result<ProposedStateOutcome<Count, Never>, StateExecutionError> {
        Ok(ProposedStateOutcome::Success {
            output: Count { value: input.value.checked_add(1).ok_or(StateExecutionError)? },
        })
    }
}
struct IncrementTwice;
impl Operation for IncrementTwice {
    type Input = Count;
    type Output = Count;
    type Failure = Never;
    fn validate_input(&self, input: &Count) -> mfm_program::Result<()> {
        if input.value > u64::MAX - 2 { return Err(ProgramError::InvalidContract); }
        Ok(())
    }
    fn expand(&self, body: &mut OperationExpansion<Count, Count, Never>) -> mfm_program::Result<()> {
        let bound = ConclusionBound::new(4096)?;
        body.pure::<Increment, Identity<Never>>(NoParams, Occurrence::new(), bound)?;
        body.pure::<Increment, Identity<Never>>(NoParams, Occurrence::new(), bound)
    }
}
let input = Count { value: 10 };
let program = expand_program(
    EntryPointId::new("mfm.example/increment-twice@1").unwrap(),
    &IncrementTwice,
    &input,
    ProgramLimits::new(0),
).unwrap();
assert_eq!(program.declarations().len(), 2);
```

Classifiers select recoverability; handlers request retry, restart or Stop. They do not authorize
execution. Independent scoped classifier/handler families and occurrence overrides resolve during
expansion, with exact typed parameters and explicit domain/context maps. Framework defaults are
NoRecovery and Stop with zero allowances. Explicit ValueMaps compose each original domain failure
into the root contract; `FromNever` represents the uninhabited path.

Checkpoint tokens belong to their authoring scope. Installed inherited handler bindings retain
that owner; direct parent/sibling token capture in another scope is rejected. Final lowering resolves
permitted tokens to typed sequence boundaries. Runtime qualifies activation, restored inputs,
visits, budgets and Effect barriers from retained history.

Capability injection authors before/after scopes around one designated Read or Effect. The
expanded failure contract and designated failure conversion are explicit; suffixes are successful
continuations. The complete scratch expansion must validate before merging into its caller. Hooks
perform no IO or adapter registration.

Synchronous authoring supports 16 callback levels including the root. Child Operations and injection
hooks share that limit; before and after hooks are siblings. Nested entry is checked before its
policy/execution descriptors are built, and suspended designated descriptors live in one boxed
payload. A rejected expansion returns Capacity without leaking prefix, checkpoint, designated or
suffix declarations. This bounds framework composition; trusted Rust callbacks must not recurse
outside OperationExpansion or assume arbitrary stack allocation is sandboxed.

Each Pure/Read conclusion has a complete frame bound; Effects separately bound prepare and
conclusion. For global recovery limit G, Program conservatively reserves genesis plus G+1 full
sequence segments. Bounds include retained objects, original/root failures and envelopes. Journal
owns format ceilings and Runtime checks concrete admission before genesis or adapter IO. Recovery
allowances and frame bounds are immutable Program data.
