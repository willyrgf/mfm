# mfm-program

`read_implementation_ref<C, I>()` and `effect_implementation_ref<C, I>()` derive the same exact
native reference retained by construction. Its v2 descriptor includes the selected family identity,
semantic capability/mode and request/evidence, native request/evidence, operational error and binding
contracts. One native family specialized for different semantic requests therefore has distinct
references. Supporting native preparation can derive this identity without Runtime lookup or a
second hashing recipe. Duplicate family claims remain checked independently of this qualified hash.

Program owns an immutable linear document, exact value contracts and one executable occurrence per
State. Typed sources construct the complete in-process Program; Runtime owns continuation,
acknowledgement and recovery. The current document is `mfm.program.v9`; the
[authoring guide](../../../docs/capability-authoring.md) describes selection, composition, extension
and cold loading. [Verification evidence](../../../docs/dsl-phase-b.md) records the tested revision.

```rust
use mfm_ids::{EntryPointId, StableId};
use mfm_program::{
    compile, load, Never, Operation, ProgramEnvironment, ProgramLimits,
    ProposedStateOutcome, Pure, PureState, State,
};
use mfm_program_derive::MfmValue;
use mfm_values::InvocationDiagnostic;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
struct Count { value: u64 }
struct KeepCount;
impl State for KeepCount {
    type Input = Count;
    type Output = Count;
    type Failure = Never;
    fn state_id() -> mfm_program::Result<StableId> {
        Ok(StableId::new("mfm.example.keep-count@1")?)
    }
}
impl PureState for KeepCount {
    fn evaluate(input: Count) -> Result<ProposedStateOutcome<Count, Never>, InvocationDiagnostic> {
        Ok(ProposedStateOutcome::Success { output: input })
    }
}
type Source = Operation<(Pure<KeepCount>, Pure<KeepCount>)>;
struct Resources;
impl ProgramEnvironment for Resources {
    type Sources = Source;
}
let input = Count { value: 10 };
let program = compile(
    EntryPointId::new("mfm.example/keep-count@1").unwrap(),
    &Source::default(), &input, &Resources, ProgramLimits::new(0),
).unwrap();
let cold = load(program.canonical_bytes(), &Resources).unwrap();
assert_eq!(cold.content_ref(), program.content_ref());
```

`OperationDefinition` declares a statically discoverable body; `Plan<Parent>` borrows or derives its
local configuration for fresh construction. Cold loading needs installed body types, not a source
value, configuration or Plan implementation. `Operation::<Definition, Defaults>::from(definition)`
retains maintained defaults for explicitly constructed definitions without a `Default` bound. The
single active nesting guard permits 16 Operation/native injection scopes, checked before planning
or injection and unwound between siblings. Tuples prove adjacent endpoints. A homogeneous vector
requires equal outer endpoints; its elements retain ordinary State and Operation boundaries.

Maintained `OperationDefaults` and `ResolveDefaults<Config>` select a handler and finite allowances.
Handler parameters and typed checkpoint targets replace together; retry/restart allowance overrides
remain independent. Leaving a child restores its parent's policy. Framework defaults are Stop and
zero allowances. `Checkpoint<Marker>` declares a typed boundary; construction lowers permitted
markers to checked sequence positions. Runtime alone authorizes activation and recovery.

`ClassifyError` projects exact acknowledged originals into Retryable, OutcomeUnknown,
InputInvalidated or Permanent. It does not replace the original or its causal audit representation.
Program's invocation callbacks perform typed preparation, native translation, evidence projection
and interpretation. Pure blocking work runs in immediately awaited workers; async adapters retain
explicit IO ownership. Effect native extraction is available before command acknowledgement, and
settlement binding always projects and checks the exact native original before admission.

There is no public mutable declaration builder, root failure mapper or incomplete Program decoder.
Fresh construction and cold discovery converge on the same executable constructors. Journal and
Store remain outside Program; callers obtain admitted bytes from Runtime before calling `load`.

The complete Program document, including aggregate binding Objects, uses the existing bounded JSON
serializer before canonicalization. Cold loading checks borrowed byte length before parsing or
copying. Capacity and codec failures retain structured causes. Selection, installed-code association
and checkpoint rejection diagnostics retain their reason, available public identities and positions;
none of these local failures authorize provider IO or a durable outcome.
