# mfm-program

Program owns the immutable `mfm-program-document@7` sequence with the `mfm.program.v7` domain.
An Operation performs deterministic source authoring; its input check and expansion commit the
exact initial value, State contracts, resolved policies, root failure maps, checkpoints and finite
recovery allowances. Runtime associates typed implementations and owns execution and recovery. Old graph bytes
are rejected.

```rust
use mfm_ids::{EntryPointId, StableId};
use mfm_program::{
    expand_program, Identity, Never, NoParams, Occurrence, Operation,
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
        body.pure::<Increment, Identity<Never>>(NoParams, Occurrence::new())?;
        body.pure::<Increment, Identity<Never>>(NoParams, Occurrence::new())
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

`ClassifyError` projects exact typed causes into four intrinsic semantics: Retryable,
OutcomeUnknown, InputInvalidated and Permanent. One static `Handler` consumes the common summary;
Runtime alone authorizes and schedules recovery. `HandlerBinding::new::<H>(params)` is inherited
from the nearest explicit Operation setting; `Occurrence::handler` replaces it for one occurrence.
Parameters and checkpoint targets replace together. Allowance overrides remain independent.
Framework defaults are Stop and zero allowances; StandardRecovery requires explicit selection.
Explicit ValueMaps retain the separate original-to-root failure contract; `FromNever` represents
an uninhabited root path.

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

Recovery allowances are immutable semantic limits. Admission does not reserve future frames or
bytes. Journal checks actual objects and complete frames; Store checks the actual accumulated
run size and frame count atomically. A later result may exceed a limit after work has occurred;
that failure preserves the acknowledged head and any unresolved Effect command authority.
