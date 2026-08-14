# RFC: simplify operation authoring, runtime control, and application entry points

Status: ready for implementation planning

Relationship: this RFC follows `RFC_REFACTOR_SINGLE_TRUST_BOUNDARY.md`. It completes boundaries
that the runtime refactor left at the wrong level: reusable Operation authoring, capability-owned
State injection, one correlated typed Runtime entry, process-facing Runtime control, generic
Application entry points, thin transports, and expressive domain contract types.

This RFC replaces the finalized `State | Match` Program algebra with one fixed ordered sequence of
ordinary `State` declarations. Successful variant handling belongs to typed State semantics;
failure variants may select a declared State-owned failure continuation. It preserves complete
cumulative State contexts, capability intent/evidence and provider-entry guarantees, the
three-family journal, Store atomicity, Program qualification, content identities, and
callback-free replay. It changes how trusted code authors Programs, binds dynamically selected
States to typed execution, exposes configured entry points, and asks Runtime to progress a run.
Persisted formats and replay do not gain an expansion or Application concept.

Where the prior RFC or current design documents prescribe separately addressed `Match`
declarations, a fixed EVM/Portfolio App facade, duplicated dynamic State-entry paths, external
coordination of one-State Runtime steps, or public suspended-owner handling, this RFC supersedes
those statements. It also supersedes cross-request acknowledgement-token retention and its
stronger recovery-liveness guarantee. It does not supersede direct-new committed-call gating,
durability before public success, fail-fast State semantics, Store CAS, or no-unsafe-Effect-reentry.

---

## Decision

MFM will have one reusable path at each of three boundaries:

1. `OperationExpansion` is the only Program-authoring and State-setup path. Its small sequence DSL
   accepts child Operations and State occurrences in semantic order, connects lexical success, and
   lowers one typed failure policy. During Access State setup, the exact capability may inject
   ordinary States before and after that occurrence.
2. Runtime owns admission, immediate State advancement, parking, and recovery classification
   behind one process-facing progression path. Every dynamically selected State enters typed
   execution through one registered `StateStart` boundary and one correlated value conversion. A
   call retains only its private ephemeral execution proofs;
   acknowledgement uncertainty discards them and later calls restart from Store-qualified history.
   App and transports never coordinate Runtime's affine lifecycle variants.
3. `ApplicationBuilder::entry_point` is the only public entry-point registration path. Each
   registration combines a strict typed request, one resolved typed root configuration, typed
   domain bindings, `C0` construction, and the same `[Operations + States]` sequence DSL. Private
   type erasure occurs only after typed registration.

CLI and HTTP consume the same Application request/response contract. They parse and render
transport data and invoke entry points; they do not plan Programs, select domain configuration or
bindings, drive States, retain suspended owners, open Store mutation, or install adapters.

Numeric domain markers are deleted. EVM capabilities and EVM/Portfolio States use semantic Rust
types while preserving stable contract IDs wherever the semantic contract is unchanged. New or
consolidated States receive new IDs and never reuse retired IDs. Canonical Program identities and
persisted occurrence addresses are recalculated once for the deliberate State-only format
cutover; source-level renaming alone must not change any stable identity.

The cutover must fundamentally reduce complexity and increase reuse rather than relocate
coordination. The final tree must have fewer concepts, duplicated code paths, public lifecycle and
domain-wrapper types, crate dependency edges from App/transports into domains, and source files
that must change to add an entry point. It should reduce net production Rust LOC wherever that
follows from deleting duplication and obsolete coordination, but LOC is evidence rather than a
forecast or acceptance gate. Tests and explicit safety checks are never removed to improve a
count.

## Material uncertainties

none.

The previously open choices are fixed by this RFC:

- Operations have exact authoring-only input, output, and unhandled-failure contracts. Success is
  ordinary lexical sequencing. A parent may give each child Operation or State occurrence one
  typed failure-policy scope; successful recovery reconstructs the item's exact externally visible
  output and rejoins the lexical continuation, while propagated handler failures use the parent's
  exact failure contract; only propagated root failure becomes a terminal Program route;
- EVM balance Operations specialize their fixed State sequence from the exact validated request:
  token sources retain a separate token-decimals checkpoint, native sources omit it, and both use
  one `ReadAssetBalance` State without a successful runtime branch;
- Runtime has no exclusive run ownership, per-run lock, cross-request execution cursor, suspended
  append registry, or owner-resolution API; Store exact-head CAS remains the concurrency boundary;
- acknowledgement uncertainty ends the current invocation, discards every hot candidate, and
  resumes only from freshly qualified retained history; losing an entry-once Effect result may
  therefore park that run permanently rather than authorize provider re-entry; cancellation after
  direct-new preparation has the same conservative outcome;
- `invoke`, `resume`, and `read` return the same Store-derived `RunView`; a qualified ready head is
  `Runnable`, a retained head with no currently permitted action is `Waiting`, and an uncertain
  admission with no qualified genesis returns `RunError::Indeterminate { run_id }` rather than a
  fabricated head or `RunProgress`;
- the State-only Program and ordinal-only occurrence address are one incompatible clean-slate
  format cutover: reset current Program/journal/replay baselines and reject prior `Match` Programs
  and old address records, with no migration, legacy decoder, or dual reducer;
- a Runtime command drains immediately actionable States until the next stable external boundary;
- each admission pins one resolved typed root configuration head;
- the Program's terminal root success and failure are the entry point's reviewed public typed
  contracts;
- exact request-specialized Program validation happens before `RunAdmitted`, without a duplicate
  exhaustive Program-closure API;
- numeric EVM and Portfolio State markers are replaced together with numeric capabilities; and
- trusted embedding supplies an already-composed Application to HTTP, while the production CLI
  invokes that transport remotely.

## 1. Goals and constraints

### 1.1 Goals

This follow-up has five measurable goals:

- make reusable operation sequencing expressible once and composable everywhere;
- make every dynamically selected State regain its exact typed input through one correlated
  Runtime boundary;
- put each responsibility in one component instead of duplicating it across domain, App, Runtime,
  tests, and transports;
- let a new on-chain or off-chain entry point be installed without editing generic platform or
  transport source; and
- reduce net production LOC where simplification permits while improving names, type safety, and
  diagnostics.

The architectural test is:

> Adding a new entry point requires its domain implementation and integration in one trusted
> composition module. It requires zero source changes to `mfm-app`, `mfm-runtime`, CLI, or HTTP.

### 1.2 Preserved and changed invariants

The runtime refactor established the correct durable and execution boundaries:

- every State has explicit complete input, output, and failure contracts;
- every selected run path is deterministic, sequential, and fail-fast;
- capability intent, evidence, Read/Effect mode, fact behavior, and provider-entry discipline are
  explicit;
- Program qualification remains the type/contract trust boundary;
- each successful State conclusion is durable before its successor context advances;
- Store append atomicity and content addressing remain unchanged; and
- Runtime, Store, import, audit, and replay never invoke authoring callbacks.

This RFC preserves those invariants while deliberately establishing these new persisted contracts:

- the Program contains only canonically ordered ordinary `State` declarations;
- each declaration owns its fixed success continuation and terminal, common, or exhaustive
  variant-selected failure continuation;
- the Program retains exact root input, terminal success, and terminal failure contracts; and
- State occurrences use ordinal-only `StateAddress` throughout Program, Journal, Store, and replay.

### 1.3 Complexity and reuse accounting

Production LOC means non-test Rust in workspace crates and binaries, excluding generated files.
The implementation report must compare the final tree with the tree immediately before the first
code cutover and include:

- production Rust LOC added and deleted;
- exported public types and functions added and deleted;
- direct production dependency edges added and deleted;
- superseded helpers and compatibility paths deleted; and
- files that must be edited to register a small unrelated off-chain entry point.

The public-surface accounting distinguishes semantic domain contracts from coordination wrappers.
Replacing three numeric marker families with named capabilities and States intentionally creates
more readable semantic type names; the cut must still delete more public lifecycle, plan-wrapper,
registry, and compatibility concepts than it adds.

LOC is not forecast before implementation and its final sign is not a standalone merge gate. The
primary evidence is fewer concepts, duplicated responsibility holders, public coordination types,
code paths, dependency edges, and future change sites. A final LOC reduction is desirable and
expected where manual planning and duplicated coordination disappear; any net increase must be
explained by necessary typed guarantees, validation, or usable transport behavior. Moving code to
another crate,
compressing readable code, deleting boundary tests, or hiding duplication in a macro does not
demonstrate simplification.

## 2. Why the current complexity appeared

The problems are connected rather than independent cleanup opportunities.

1. Capability-owned expansion disappeared, so EVM and Portfolio planners manually unrolled State
   sequences, addresses, continuations, execution modes, and binding checks.
2. There was no generic entry-point registration boundary, so `Application` became the central
   switch for every domain planner, configuration type, binding set, and catalog contract.
3. Pure and Access Runtime registration grew separate erased-input conversions and dynamic cursor
   paths, while live selection keyed too narrowly on State implementation identity.
4. Runtime exposed its internal affine step algebra without a process-facing progression command,
   so App methods and integration tests became examples of how to coordinate `SpawnStep`,
   `ResumeStep`, `RuntimeStep`, and `SuspendedRun`.
5. CLI and HTTP had no generic operation surface to invoke. Avoiding duplicated trusted
   composition left both standalone binaries unavailable rather than thin and useful.
6. Numeric const parameters reduced declarations locally but moved semantic meaning into comments,
   registration tables, binding calls, tests, and every future change site.

The result is accidental responsibility sharing:

| Symptom | Knowledge duplicated outside its responsible component |
| --- | --- |
| `submission_program` manually creates seven declarations | Capability submission policy |
| `append_balance_fragment` reserves ordinals and mutates a caller vector | Reusable child Operation structure |
| App stores Portfolio/EVM config and binding vectors | Typed entry-point planning dependencies |
| App hard-codes two entry-point branches | Extensible entry-point dispatch |
| `application_catalog` lists every domain value and numeric capability | Trusted domain/catalog composition |
| `mfm-evm-live` imports Portfolio and registers its States | Caller-owned child-Operation instantiation |
| Pure and Access registration each downcast selected State input | One correlated Runtime State-start boundary |
| App retains `SuspendedRun` and normalizes Runtime steps | One-call Runtime progression and cold recovery |
| App tests loop up to a fixed number of States | Runtime progression behavior |
| `EvmCapability<0>`, `<1>`, `<2>`, `<3>`, `<6>`, and `<7>` | Capability semantics |

The additional LOC and layers are not required by the safety model. They compensate for missing
reusable boundaries. The solution is intentionally limited to one sequence expander, one
registered State-start/value boundary, one generic entry-point registration method, one high-level
Runtime progression path, and semantic domain types. Everything else is consolidation or
deletion.

## 3. Target responsibilities

| Component | Owns | Does not own |
| --- | --- | --- |
| Domain Operation | Exact Input/Output/unhandled-Failure contracts, ordered reusable child Operations and States, domain context transitions, and typed construction helpers | Runtime execution, final addresses, or transport decoding |
| `OperationExpansion` | The only recursion/flattening path, lexical success and typed failure-policy lowering, State setup, capability injection, contract continuity, bounds, addresses, final State-declaration transitions, and Program construction | Domain semantics, provider IO, or persisted expansion metadata |
| State | One reusable domain transition with complete input, output, and failure contracts | Expansion, final addresses, final continuations, or awareness that it was injected |
| Final State declaration | One occurrence's immutable execution data and fixed success/failure transitions | Domain behavior or authoring callbacks |
| Access capability | Intent/evidence/mode/fact discipline and deterministic authoring-time pre/post State injection | Address assignment, scheduling, or runtime expansion |
| Adapter/binding | Exact provider implementation and immutable association for one Access occurrence | Operation composition or entry-point dispatch |
| Trusted composition | Explicit catalog contributions, Store/Runtime assembly, live adapters, resolved configurations, and configured entry-point registration | Transport request handling |
| `ApplicationBuilder` | Fixed-tenant entry-point registration, validation, and private heterogeneous dispatch construction | Domain-specific global registries or Runtime stepping |
| `Application` | Strict public input, entry-point lookup, Runtime invocation, public result/error projection, and read-only public queries | Domain sequencing, Store mutation, or Runtime lifecycle-step coordination |
| Runtime | Exact registered-State binding, the sole State-input value conversion, typed execution, admission, bounded immediate advancement, conservative parking, and cold recovery | Operation authoring, transport concerns, cross-request execution custody, or background scheduling |
| Store/replay query code | Callback-free State success/failure routing, run status, result, trace, audit, and export derivation | Domain-value downcasts or live State/adapter invocation |
| CLI/HTTP | Transport parsing, invocation, status mapping, and rendering | Catalogs, configs, bindings, adapters, expanders, or Runtime internals |

## 4. Problem group A: fragmented Program authoring

### 4.1 Problem situation

The refactor removed capability-owned authoring expansion. Domain planners now construct final
State declarations directly.

EVM transaction submission manually declares:

1. reserve the wallet nonce;
2. derive the transaction candidate;
3. broadcast the transaction;
4. read the transaction receipt;
5. read the finalized head;
6. read the canonical inclusion block; and
7. consolidate the public result.

EVM balance collection appends a reusable fragment into a caller-owned declaration vector,
calculates ordinals and successors, reconstructs access modes, and repeats binding validation.
Portfolio must know that fragment's declaration count and reserve address ranges around it.

This makes every operation planner understand capability internals and final Program mechanics.
Changing a reusable sequence requires coordinated edits in the capability/domain implementation,
the planner, parent planners, live registration, catalog composition, tests, and documentation.

Wrapping an already-finalized broadcast declaration cannot fix submission safely. Nonce
reservation consumes `EvmSubmissionRequest`, candidate derivation produces broadcast input, and
confirmation continues changing `EvmSubmissionProgress` after broadcast. A wrapper cannot pretend
that the broadcast State alone owns those outer contracts.

### 4.2 Solution: one streaming `[Operations + States]` DSL

An Operation is an authoring-only sequence with three exact nominal contracts:

```text
Operation {
  Input: MfmValue
  Output: MfmValue
  Failure: FailureValue
  expand(&mut OperationExpansion<Input, Output, Failure>)
}
```

`Failure` is the one type of failure still unhandled when that Operation finishes expanding. It is
not a runtime Operation result or a persisted declaration. The final Program still contains only
ordinary States and their transitions.

An Operation writes child Operations and State occurrences in semantic order:

```text
sequence.operation(child_operation, child_failure_policy)
sequence.state::<S>(occurrence_setup, state_failure_policy)
sequence.operation(next_child, next_child_failure_policy)
sequence.state::<T>(next_setup, next_state_failure_policy)
```

Every action receives exactly one failure policy. `Propagate` is the zero-sized common policy and
implements the policy contract only when the source failure and enclosing Operation failure are
the same Rust type; handled policies use the scoped writer. There is no omitted/default argument,
method overloading, or third composition action. Child Operations and States remain the only
general sequence items.

Success is always lexical. The successful externally visible output of one item becomes the input
of the next item; there is no success callback or caller-supplied destination. For a child
Operation this is `Child::Output`. For an authored State occurrence it is the final output of the
complete injected-before/original/injected-after suffix, which equals `S::Output` only when no
after State changes it. A failure policy is needed only when the item failure must be handled,
adapted, recovered, or routed by variant before the enclosing Operation can continue.

Conceptually:

```text
Operation::expand(sequence):
  sequence.operation(
    child,
    |failure| failure.state::<MapChildFailure>(mapping_setup, Propagate),
  )
  sequence.state::<NextState>(next_setup, Propagate)
```

The failure scope begins with the complete unchanged item failure. A common or variant handler is
an ordinary `[Operations + States]` sequence. Every successful handler path must finish with the
item's exact externally visible output; it then rejoins the same next lexical item as ordinary
item success. Every failure propagated by the handler must be the exact enclosing
`Operation::Failure`. `Propagate` or an explicit variant propagation arm is legal only when the
source failure equals the enclosing failure contract. Adapting a different failure requires an
ordinary typed mapper State.

The same rule applies to an authored State occurrence: capability setup normalizes every
propagated before/original/after failure to `S::Failure`, while successful recovery must produce
the completed occurrence's externally visible output. That failure must equal the enclosing
Operation failure or its typed policy must handle/map it. Exhaustive variant policies use the
source failure association's stable tags, and every nonterminal handler receives the complete
unchanged source failure rather than a projected payload.

`sequence.operation` recursively invokes the same expansion path. `sequence.state` invokes the one
State setup path described below. Calls stream into one bounded flat draft buffer. Each private
draft is only the immutable State-declaration core plus unresolved success/failure slots; it is not
a public node enum, nested tree, alternate Program algebra, or independently reusable plan.
Finalization consumes that sole buffer into `Vec<StateDeclaration>` once, after every forward slot
is resolved. There is never a second finalized declaration representation.

Canonical lowering order is:

```text
child States
common failure-handler States
or variant handlers in stable-tag order
following lexical States
```

Item success skips the handler declarations. Item failure targets the selected handler or remains
propagated from the current Operation. Successful recovery and ordinary item success target the
same following lexical State. Propagation never means "terminate the run here": it exits one
typed Operation scope and may be handled by its caller. Only after root expansion does remaining
success become terminal `Output` and remaining propagated failure become terminal `Failure`. An
empty Operation is legal only as the identity `Input == Output` and exposes no failure source. Any
failed item/policy expansion drops its entire scratch suffix.

Once root expansion finishes, the expander assigns canonical forward addresses, validates both
terminal contracts and every transition, and constructs the State-only `ProgramDocument`. No
Operation, failure-policy closure, unfinished index, or expansion callback survives.

The same State implementation may appear repeatedly with different runtime inputs,
configuration-derived context, or binding descriptors. Its input/output/failure contracts and
Access capability pairing remain fixed; a different capability contract requires a different
semantic State type. Occurrence-specific configuration is explicit in the cumulative context or
immutable binding rather than captured by the registered State implementation. A State occurrence
is identified by its final address and immutable declaration, not by inventing another State
category.

### 4.3 State is the non-expandable leaf

There is one State concept. A State is always a non-expandable authoring leaf with one complete
input contract, output contract, and failure contract.

There is no:

- abstract versus concrete State;
- protected or direct State;
- expandable State method;
- wrapper State;
- special injected-State category; or
- replacement of an original State by a capability recipe.

An occurrence setup is transient typed arguments used to finalize an ordinary State. It may carry
an exact capability, binding, and bounded domain configuration. The sequence action supplies any
typed failure policy separately; setup never contains a destination, label, ordinal, or raw
continuation. It is not an AST node or persisted wrapper. Keep its helper representation private or
minimally public only where domain code must construct it.

Configuration that affects runtime behavior must lower into `C0`, a complete successor context, or
an existing immutable binding descriptor. It cannot remain hidden in an authoring closure after
the finalized Program is admitted.

### 4.4 Capability-owned pre/post injection

Every Access capability usable during Operation authoring implements the same injection contract
for each supported State/capability pairing:

```text
CapabilityExpansion<S> for C {
  type Setup
  original_binding(&Setup)
  write_before(&Setup, &mut StateSetupWriter)
  write_after(&Setup, &mut StateSetupWriter)
}
```

`Setup` is one bounded domain-owned checked product for that exact `S, C` pair. The two methods
stream zero or more heterogeneous States directly through the ordinary recursive State-setup path;
they do not return a collection, erased request, stored callback, or intermediate declaration
enum. Empty writes are the ordinary identity behavior. The policy implementation is associated
with the capability type, while `OperationExpansion` alone controls recursion, ordering,
validation, addresses, and final Program construction.

The authoring trait and restricted `StateSetupWriter` belong in `mfm-program`, where State setup is
known, and are implemented/used by the domain-owned capability type. This avoids making
`mfm-capabilities` depend on Program authoring types or putting authoring callbacks into the runtime
capability ABI. It is a required generic bound of State setup, not a side registry.

State setup is exactly:

```text
setup_state<S, C>(incoming_contract, setup):
  derive State and capability contract identities
  validate the exact pairing, checked setup, original binding, mode, and catalog membership

  open one scratch suffix
  C::write_before(setup, restricted_before_writer)
  require the before suffix's output contract == S::Input
  append the original State exactly once
  C::write_after(setup, restricted_after_writer starting at S::Output)
  apply the authored occurrence failure policy against S::Failure and the completed suffix output
  commit the suffix only after its complete validation succeeds
```

Before invoking either hook, setup proves intrinsic facts that do not depend on the surrounding
context:

1. exact State implementation and input/output/failure contract identities;
2. exact capability contract identity and validity;
3. a supported State/capability pairing;
4. matching Read or Effect mode, attempt bounds, fact selection, and effect domain;
5. a structurally valid original binding descriptor for that same State/capability; and
6. valid bounded, secret-free typed setup.

The incoming context is not compared with `S::Input` before `write_before`: injected nonce
reservation intentionally transforms `EvmSubmissionRequest` into the broadcast State's
`EvmSubmissionProgress` input. Recursive before expansion begins at the incoming contract and must
end at `S::Input`; recursive after expansion begins at `S::Output` and its final output becomes the
whole occurrence's output. Actual live State and adapter availability is validated later by
Runtime against the exact finalized Program before admission.

The hook is deterministic and performs no ambient IO. Its restricted writer exposes only normal
typed `state` setup. It cannot replace, suppress, duplicate, or mutate the original State;
contribute an Operation; allocate addresses; select raw successors; register catalog entries;
inspect runtime values; persist a callback; or define rollback, compensation, or `finally`
behavior.

Injected States recursively receive normal setup, so their capabilities may also inject. A fixed
maximum recursive expansion depth makes direct or mutual injection cycles fail deterministically,
and the existing declaration limit is enforced while expanding. Legitimate repeated State types
remain allowed within those bounds. Any error drops the scratch suffix and returns one typed
authoring error with no partial Program.

The restricted writer applies the same typed State failure-policy rule but may write only States.
Every failure propagated by an injected before/after State must equal the original `S::Failure`
before the scratch suffix commits. A typed mapper State may adapt it; no capability hook receives
or constructs a final address. Consequently all propagated failures exposed by the completed
before/original/after suffix have the original State's exact failure contract and participate in
the authored occurrence policy normally. Successful recovery from that outer policy produces the
completed suffix output, not merely `S::Output`, and skips the unfinished remainder of the failed
suffix.

Pure States retain their current meaning. They require no Access capability and create no durable
preparation. Their setup uses the same path with an empty capability phase. Do not introduce a fake
`NoCapability`, `Direct`, or identity capability.

### 4.5 Context accumulation across authored and injected States

Authoring validates context contracts; Runtime carries context values. No runtime context values
exist during expansion.

For an authored State `S`, before States `P1..Pn`, and after States `Q1..Qm`, the final selected
sequence is simply:

```text
Cin
  -> P1 -> ... -> Pn
  -> S
  -> Q1 -> ... -> Qm
  -> Cout
```

Every predecessor's successful output contract must equal its successor's input contract. The
Operation input contract equals the first selected State input, and its output contract equals the
last successful State output. The rule also applies across child Operations and failure-handler
sequences. `Next::Terminal` and `Next::State` are exclusive by representation.

Each successful nonterminal State constructs the complete next context. The framework does not
merge patches, restore an outer context, search history for earlier outputs, or maintain a generic
context map. If a later State needs earlier information, the preceding complete output carries it
explicitly.

An internal authoring cursor may retain only the current success contract and private indices of
unfinished transition slots while validating expansion. It is bounded compiler state over the one
flat draft buffer, not a domain context or second Program representation, and is not serialized.

Before/original/after is success-path order:

- a failed before State skips the original and all after States;
- a failed original skips all after States;
- a failed after State skips all later States; and
- the failed State's typed failure policy determines propagation, mapping, recovery, or
  variant-specific routing.

A successful conclusion is durable before context advancement. If broadcast concludes and receipt
or finality later stops, resume begins at the unresolved later occurrence. It does not reserve a
new nonce, derive a new candidate, or rebroadcast. Ordinary State conclusions are the complete
recovery boundary; no composite-operation checkpoint is added.

### 4.6 State-owned transitions and failure variants

There is no separately addressed `Match` declaration. Every final declaration is an ordinary
State and owns its transitions:

```text
StateDeclaration {
  input_contract_ref,
  output_contract_ref,
  failure_contract_ref,
  on_success: Next,
  on_failure: FailureNext,
}

Next = State(address) | Terminal
FailureNext = Terminal | State(address) | ByVariant { stable_tag -> Next }
```

The failure contract and `on_failure` are non-optional because every `State::Failure` is an exact
`FailureValue`. Terminality is represented only by `Next::Terminal`; the old parallel
`terminal` flag and optional next/failure fields are deleted.

Successful closed-sum interpretation is domain behavior. A producing State carries the complete
success context plus a typed enum or option identifying which part is present. Its one declared
successor consumes that complete type and either handles the alternatives itself or propagates the
discriminant through a fixed typed State sequence. If Access behavior differs by alternative, a
variant-aware State prepares the corresponding typed capability intent; the Program does not
branch around Access occurrences and an irrelevant provider call must not be invented merely to
simulate a skipped branch.

Failure routing is different because it decides whether and where recovery continues. Authoring
may propagate the complete failure from the current Operation scope, send every failure to one
common handler, or route an exact closed failure sum by stable variant tag. Final root lowering
turns any still-propagated route into `FailureNext::Terminal`:

```text
sequence
  .state::<SubmitTransfer>(..., |failure| {
      failure
        .variant("insufficient_funds")
        .to::<HandleInsufficientFunds>(..., Propagate)
        .variant("provider_unavailable")
        .to::<HandleUnavailable>(..., Propagate)
        .variant("invalid_request")
        .propagate()
  })
  .state::<RecordReceipt>(..., Propagate)
```

`RecordReceipt` is the lexical success continuation. Each successful recovery arm must construct
the exact completed `SubmitTransfer` occurrence output consumed by it. A propagation arm is legal
only when the complete `SubmitTransfer::Failure` is also the enclosing Operation's exact
`Failure`; otherwise the arm must use a mapper whose failure is the enclosing contract. If an
enclosing caller handles that Operation failure, the arm targets the caller's handler. It becomes
`FailureNext::Terminal` only if it remains propagated at the root.

Each variant carries the complete recovery context required by its declared path. A routed State
consumes the complete unchanged failure type, not a projected payload: the same contract, value
reference, canonical bytes, and domain value cross the transition. The handler may pattern-match
normally after the one Runtime State-input conversion. A common handler is simply one fixed
failure successor.

`ByVariant` is valid only when the failure's exact catalog `ValueAssociation` contains closed-sum
metadata derived from its registered `SchemaShape::Enum`, including its external, internal, or
adjacent tagging profile and complete stable-tag set. That association exposes one private
structural `variant_tag(canonical_bytes)` operation. Program qualification uses the same metadata
to prove that every registered stable tag occurs exactly once, there are no unknown or duplicate
arms, and every nonterminal destination consumes the complete source failure contract. The arms
are canonically ordered by stable tag. There is no inherited `MAX_MATCH_ARMS` concept: the exact
registered enum descriptor bound plus the existing canonical Program byte/declaration bounds own
capacity.

During callback-free reduction, Store invokes that exact association-owned structural operation
on the already-qualified canonical failure and selects the declared successor. It does not
hard-code a `{"kind","value"}` shape, perform a domain `Any` downcast, decode or project a payload,
or create a new value object. The complete original failure and `ValueRef` remain unchanged, and
Runtime owns no competing routing decision.

Final State declarations have canonical list order. `StateAddress` is a private-field newtype over
the declaration's zero-based `u32` list index and its wire form is that canonical unsigned integer.
The declaration carries no redundant address field. `OperationExpansion` assigns indices and
exposes no public label, ordinal, or address construction API; ingress requires the nonempty entry
to be index zero. Every nonterminal success or failure target must be in bounds and greater than
the source index, and every declaration must be reachable from index zero by following the fixed
transitions. Gaps, reordered declarations, unreachable declarations, self-targets, and backward
targets are rejected. Match-arm path components and `SequentialControlAddress` are deleted.

This RFC adds no parallel branch, fan-out, multi-input merge, scheduler, or dynamic workflow
structure. Failure recovery is one deterministically selected forward path that may rejoin lexical
execution only after reconstructing its exact input contract; it is never concurrent with ordinary
success.

The target retained document is conceptually:

```text
ProgramDocument {
  entry_point_id,
  input_contract_ref,
  output_contract_ref,
  failure_contract_ref,
  states: Vec<StateDeclaration>,
}
```

`ProgramDocument` retains those exact root contracts even for a zero-State identity Program.
Qualification proves that every root success terminates with the declared Output and every root
failure terminates with the declared Failure. Those contracts come from the typed root
`OperationExpansion<C0, Output, Failure>` and are not inferred from whichever terminal State
happens to be encountered first.

### 4.7 EVM transaction submission target

The transaction-submission Operation authors the ordinary broadcast State with
`EvmTransactionSubmission`. For the exact
`CapabilityExpansion<BroadcastTransaction>` pairing, `Setup = EvmSubmissionBindings`; this checked
fixed product supplies reserve, broadcast, receipt, finalized-head, and canonical-inclusion
descriptors, and `original_binding` returns its broadcast descriptor. The capability injects:

```text
before broadcast:
  reserve wallet nonce
  derive transaction candidate

original:
  broadcast transaction

after broadcast:
  read transaction receipt
  read finalized head
  read canonical inclusion block
  consolidate submission result
```

The final runtime contexts remain:

```text
EvmSubmissionRequest
  -> reserve wallet nonce
  -> EvmSubmissionProgress::Reserved
  -> derive transaction candidate
  -> EvmSubmissionProgress::Candidate
  -> broadcast transaction
  -> EvmSubmissionProgress::Broadcast
  -> read transaction receipt
  -> EvmSubmissionProgress::Receipt
  -> read finalized head
  -> EvmSubmissionProgress::Finalized
  -> read canonical inclusion block
  -> EvmSubmissionProgress::Canonical
  -> consolidate submission result
  -> EvmSubmissionOutput
```

Reserve, broadcast, receipt, finalized-head, and canonical-inclusion occurrences retain exact
Access capabilities and bindings. Derive and consolidate remain Pure. The injected Access States
receive recursive setup even when their own injection is empty.

For identical input shape and bindings, expansion must retain the current seven semantic State
occurrences, context contracts, capability bindings, and ordering. Its canonical bytes and
`ProgramRef` are frozen under the one new State-only format rather than compared with the removed
`State | Match` encoding.

### 4.8 Portfolio and reusable child Operations

Portfolio writes its own States and composes one child EVM balance Operation per configured
collection. The exact validated `EvmBalanceRequest` used to construct the collection's initial
context also specializes that child Operation. For each source, authoring writes:

```text
CheckChainIdentity<K>
ReadInitialAnchor<K>
[ReadTokenDecimals<K> only when source.token is present]
ReadAssetBalance<K>
ConfirmBalanceAnchor<K>
```

After the last source it writes `ConsolidateBalanceCollection<K>`. This conditional exists only
during request-specific Operation expansion. The finalized Program is one fixed State sequence;
Runtime does not select the native/token shape or insert/omit a source State. It only follows the
fixed success and failure transitions already retained in that Program.

`ReadInitialAnchor<K>` constructs the complete next context expected by the statically selected
source shape. `ReadTokenDecimals<K>` remains its own Access State and durable conclusion, so token
decimals and token units retain independent preparation, replacement, failure, and cold-resume
boundaries. `ReadAssetBalance<K>` prepares a native-units or token-units intent from the complete
context and its occurrence uses the corresponding immutable binding. It never performs both
provider reads in one call. Its typed handler constructs the common complete context consumed by
`ConfirmBalanceAnchor<K>`.

The Operation constructor privately derives both the initial context and expanded source shapes
from one validated request value. A caller cannot supply an independently constructed shape list.
`SelectBalanceAsset<K>`, `EvmBalanceAsset<K>`, the successful `Match`, and the separate native-unit
and token-unit State implementations are deleted. The common expander assigns addresses and
connects continuations after all child and capability injection.

Portfolio composes each child with one caller-owned typed failure policy:

```text
sequence.state::<EnterPortfolioCollection>(enter_setup, Propagate)
sequence.operation(
  EvmBalanceCollection(...),
  |failure| failure.state::<MapEvmBalanceFailure>(mapping_setup, Propagate),
)
sequence.state::<ResumePortfolioCollection>(resume_setup, Propagate)
```

`EnterPortfolioCollection` transforms the complete Portfolio continuation into the exact child
EVM context. Ordinary child success goes directly to `ResumePortfolioCollection`. Every EVM
failure propagated from inside the child targets `MapEvmBalanceFailure`, which consumes the
complete unchanged `EvmBalanceFailure`. Its success contract is
`EvmBalanceCollectionCompletion<PortfolioContinuation>`, the exact value required to recover and
rejoin `ResumePortfolioCollection`; its failure contract is `PortfolioSnapshotFailure`. The current
handler always returns that parent failure, but the authoring contract permits a real typed recovery
without another DSL feature.

Portfolio no longer knows the balance Operation's declaration count, starting ordinal, or reserved
address range. Child composition consumes the caller's exact complete current context and returns
one complete successor context. A mismatch requires an explicit caller-owned State; expansion
never coerces, wraps, merges, or interprets context values.

For `C` collections, `S` total sources, and `T` token sources, the resulting Portfolio count is
`2 + 4*C + 4*S + T`: initialize/consolidate; enter, child consolidation, mapper, and resume per
collection; four common source States; and one additional decimals State per token. At the existing
ceilings `C = S = T = 64`, this is exactly 578 States.

## 5. Problem group B: Runtime typed entry and lifecycle leakage into App

### 5.1 Problem situation

Runtime currently has separate dynamic registration implementations for Pure and Access States.
Both recover the selected State input through their own erased-value downcast and then construct
different dynamic lifecycle wrappers. Registration is keyed too narrowly by State implementation
identity even though a concrete live pairing is also correlated with exact input/output/failure
contracts and capability contract, while each occurrence separately selects an exact adapter
binding.

The Program layer already retains typed values together with canonical bytes and content
identities, but dynamic transfer does not yet have one explicit correlated boundary shared by hot
advancement and cold reconstruction. That leaves more source locations able to pair a Rust type
with a retained contract than the trust model requires.

Runtime currently exposes a low-level affine lifecycle suitable for implementing Runtime itself,
but App has become its process supervisor:

- admission matches `SpawnStep`;
- `drive` cold-resumes and consumes one `RunSession` State step;
- `finish_runtime_step` translates every `RuntimeStep` variant;
- `resolve_suspended_run` separately resolves acknowledgement owners;
- App retains `SuspendedRun` values in its own unbounded map; and
- App tests manually loop over States to reach a terminal result.

This puts recovery and acknowledgement policy in a transport-facing crate. It also loses
distinctions: the current mapping collapses several conflicts and retained append rejections into
one public internal error, while the suspended map invents cross-request process state that the
durable run does not need.

The low-level types are useful inside `mfm-runtime`; their public exposure is not a reason for App
to coordinate them. The duplicated dynamic wrappers are likewise implementation residue, not a
second State model.

### 5.2 Solution: one correlated value and registered State start

Replace the current parallel qualified-value and dynamic-registration concepts with one current
correlated value family and one Runtime-owned registered start function. Rename and reshape
`QualifiedTypedValue<T>` into `ProvenValue<T>` and replace `QualifiedValue` with the private
Runtime-only `ErasedProvenValue`; retain neither old name nor an alias:

```text
ProvenValue<T> {
  association: Arc<ValueAssociation>,
  value: T,
  canonical: CanonicalBytes,
  value_ref: ContentRef,
}

ErasedProvenValue {
  association: Arc<ValueAssociation>,
  canonical: CanonicalBytes,
  value_ref: ContentRef,
  value: Box<dyn Any + Send + Sync>,
}
```

The exact same association brands both forms. `ValueAssociation` inseparably owns the exact
catalog identity, value contract reference, schema-descriptor identity, process-local `TypeId`,
one monomorphic strict validate-and-drop function over retained canonical bytes, one strict owned
decoder for Runtime, and optional closed-sum structural metadata plus
`variant_tag(canonical_bytes)` operation. A value proven under one association cannot be re-erased
or accepted under another association merely because both use the same Rust type. Store invokes
the exact validate-and-drop function while qualifying retained material, so a schema/Serde drift
still fails closed, but no decoded domain value or `Any` leaves that call. Runtime alone may invoke
the owned decoder/downcast authority.

Because `mfm-program` and `mfm-runtime` are separate crates, the association is one opaque kernel
bridge type with private fields and constructors, not a publicly forgeable record. The catalog is
the only source of handles; the narrow kernel API exposes only the operations Runtime needs, and
higher layers do not re-export it. Compile-fail coverage proves that callers cannot construct an
association or perform an unchecked downcast. Architecture/dependency checks keep Runtime as the
only production consumer of decode authority. This deliberately replaces the public erased value
wrappers; it does not add a second association registry or public proof hierarchy.

`ErasedProvenValue` is not cloneable, serializable, persisted, or part of any public Application,
adapter, Store, Journal, replay, or transport API. Runtime may retain a hot value locally while it
awaits Store I/O, but Store receives only canonical objects and value references and never carries
the domain `Any` in an argument or result. Runtime alone consumes the domain value when entering a
selected State.

Capability intent/evidence qualification remains a separate exact capability concern and does not
reuse `ErasedProvenValue`. Each catalog `CapabilityAssociation` owns monomorphic, bounded
functions over canonical bytes that strictly decode `C::Intent` and `C::Evidence`, derive any
prior-fact selection, and run `C::bind_evidence`; the typed temporaries are dropped inside that
function. Store may invoke those functions while qualifying preparation/evidence history, but it
never receives or returns a domain `Any`, performs a downcast, or invokes a State/domain handler.
The closed-sum tag extractor is likewise one generic structural operation derived from the exact
`SchemaShape::Enum`, not a domain-provided callback. Therefore Store and replay remain
callback-free while the unique dynamically selected State-context downcast remains in
`start_typed`.

Runtime assembly installs one `RegisteredState` per exact concrete State execution association:

```text
RegisteredState {
  implementation_ref: ContentRef,
  input_association: Arc<ValueAssociation>,
  output_association: Arc<ValueAssociation>,
  failure_association: Arc<ValueAssociation>,
  capability_association: Option<Arc<CapabilityAssociation>>,
  start: StateStart,
}

StateStart = Arc<
  dyn Fn(StartContext, ErasedProvenValue) -> StateFuture
    + Send
    + Sync
    + 'static,
>

StateFuture = one private boxed future whose output is Runtime's typed Result
```

`implementation_ref` remains the stable persisted State identity, but it is not by itself a
sufficient live registry key. The private registry key is the tuple of implementation reference,
exact input/output/failure association identities, and optional exact capability association.
Pure uses `None`; Access uses its exact `C`. Duplicate or partially colliding keys fail assembly,
and lookup has no implementation-ID-only fallback.

Adapter selection remains occurrence-specific and separate: after the State key matches, Runtime
qualifies the declaration's exact binding reference under the already-selected State/capability
types and places that adapter plus the bound occurrence in `StartContext`. Registering a second
binding does not duplicate the semantic State start. `StartContext` contains Runtime authority;
it is not an ambient domain context map.

Registering concrete `S` installs a monomorphic start closure. Access registration specializes the
typed runner with the exact `S, C` pair:

```text
register_access<S, C>(assembly, implementation):
  let input_association = assembly.catalog.value::<S::Input>()?
  let output_association = assembly.catalog.value::<S::Output>()?
  let failure_association = assembly.catalog.value::<S::Failure>()?
  let capability_association = assembly.catalog.capability::<C>()?

  RegisteredState {
    implementation_ref: state_implementation_ref::<S>(),
    input_association: input_association.clone(),
    output_association,
    failure_association,
    capability_association: Some(capability_association),
    start: Arc::new(move |context, input| {
      start_typed::<S>(context, input, input_association.clone(), |context, input| {
        run_typed_state::<S, C>(context, input, implementation.clone())
      })
    }),
}
```

All associations are derived from Runtime assembly's own exact catalog. A caller cannot supply or
substitute one. Distinct generic instantiations sharing a stable implementation reference remain
selectable because their complete value associations differ.

The one generic start function contains the only dynamically selected State-input domain-value
downcast:

```text
start_typed<S, Run>(context, input, association: Arc<ValueAssociation>, run):
  StateFuture(async move {
    let input: ProvenValue<S::Input> =
      input.into_typed::<S::Input>(association)?
    run(context, input).await
  })
```

Pure registration supplies its Pure typed runner to the same function; it does not introduce a
fake capability. “One downcast” means this one private State-input implementation, exercised each
time Runtime enters a dynamically selected State. Capability intent/evidence retained-byte
qualification remains separately scoped to exact capability registration and performs no
State-context downcast.

The conversion checks the exact association identity, contract, schema, and `TypeId` before
moving the value. Failure is an internal Program/assembly invariant error: Runtime fails closed
before invoking the State, adapter, or provider and never tries another decoder.

### 5.3 Typed State lifecycle

After `start_typed` succeeds, the complete lifecycle is statically typed. Provenance remains
Runtime-owned; domain callbacks receive only domain values so State semantics cannot depend on
canonical serialization or content references.

For an Access State:

```text
state.prepare(&S::Input) -> C::Intent
append StatePrepared
direct-new acknowledgement -> CommittedCall<S, C>
typed adapter invocation -> authenticated C::Evidence
state.handler(S::Input, &C::Evidence) -> Outcome<S::Output, S::Failure>
```

Provider entry occurs only after the exact direct-new preparation acknowledgement. For a Pure
State, preparation, committed-call minting, and provider entry do not exist; its typed handler
runs directly. Runtime canonicalizes and schema-validates the typed outcome once under the bound
output or failure association, producing `ProvenValue<S::Output>` or
`ProvenValue<S::Failure>`. This is one Runtime construction pass; Store independently performs its
strict validate-and-drop check whenever those bytes cross or reenter the retained-history trust
boundary.

`StateFuture` is a private Runtime future over this lifecycle. It does not create a second public
cursor algebra or expose State-by-State ownership to Application.

### 5.4 One hot and cold entry path

Hot and cold execution differ only in construction of the correlated input:

```text
hot typed outcome -> ProvenValue<T> -> ErasedProvenValue
cold qualified object -> exact association decoder -> ProvenValue<T> -> ErasedProvenValue
```

Both then use the selected `RegisteredState.start` and the same typed lifecycle. Store, not
Runtime, strictly qualifies and reduces retained history and selects the current occurrence.
Cold resume is therefore:

```text
PostgreSQL rows
  -> Journal structural validation and Store qualification/reduction
  -> selected occurrence and retained object
  -> RuntimeAssembly exact association decode
  -> ErasedProvenValue
  -> RegisteredState.start
  -> typed State lifecycle
```

A hot outcome may cross directly to its successor only after Store durably accepts the conclusion
or returns the exact idempotent already-concluded result. A conflicting or no-longer-selected
result discards the local typed candidate and follows the Store-selected retained value. An
acknowledgement-unknown result ends the invocation and discards the hot candidate and every
ephemeral execution proof. Runtime performs no further State, adapter, provider, or append work
from that cursor. A later call starts only from freshly qualified retained history.

A failure variant uses the same handoff without projection: Store selects the declared route from
the stable tag, and the complete correlated failure enters the selected `RegisteredState.start`.
There is no second payload decode.

Replay remains callback-free and receives no Runtime assembly, registered start function, or
domain `Any`. There is no separate cold executor, but Replay continues to own replay reporting
rather than invoking the live Runtime state machine.

### 5.5 Solution: one process-facing Runtime progression path

Runtime will expose typed admission and run resume methods that both delegate to one private
`advance_until_stable` path. They return the same callback-free, Store-derived `RunView` as the
read path rather than the internal step algebra.

The progression path drains immediately actionable States until the first stable external
boundary:

- terminal success with the exact qualified root result;
- terminal failure with the exact registered root domain contract;
- durable waiting or permanent Effect parking;
- a fixed work bound reached at a qualified runnable head;
- acknowledgement uncertainty followed by a callback-free Store reload;
- conflict or invalid history; or
- redaction-safe capacity/identity failure.

`RuntimeLimits` owns one nonzero `max_state_starts_per_call`, fixed by trusted assembly rather than
the request. The progression path checks it only at a durable boundary before calling another
`RegisteredState.start` and counts each start once. It never interrupts a State or provider future
to manufacture a yield. Reaching the bound returns the qualified head as `RunState::Runnable`; the
next `resume` continues from that retained head. Runtime does not spin on preparation rejection,
unresolved provider results, or acknowledgement uncertainty.

This is caller-driven execution, not a scheduler. Runtime creates no background worker, timer,
queue, general per-run lock, cross-request cursor, or process-wide lease. An embedding or transport
invokes admission/resume explicitly, but it never asks for one raw State step.

### 5.6 No cross-request run custody

Runtime has no exclusive run owner, run lock, suspended-owner map, pending-append registry,
resolution endpoint, or administrative disposal API. Concurrent workers may select the same
qualified head and race; Store append identity and exact-head compare-and-append remain the sole
linearization boundary.

One invocation still uses private non-Clone affine proofs while it is active. In particular, only
the direct-new durable preparation result can mint the exact `CommittedCall` consumed by one
provider entry, and a selected-prefix proof prevents same-call reuse or Store-opening
transposition. These are execution-local implementation types, not ownership of the run, and never
escape the Store/Runtime boundary or survive cancellation, acknowledgement uncertainty, or a
public return.

Store admission, preparation, and conclusion methods consume those invocation-local inputs and
return no retry token on acknowledgement uncertainty. Store performs any bounded mechanical
fact-frontier rebind or same-call append retry internally, without invoking a State or provider.
The exported split `prepare conclusion -> commit owner -> resolve owner` protocol and its
`PreparedAdmission`/`SelectedConclusion` retry values are deleted. Any affine append-attempt state
needed for one call is an unexported local inside that Store future and cannot be returned. An
acknowledgement-unknown result returns only its redaction-safe classification and run identity for
the callback-free reload. A definitive rejection returns its distinct redaction-safe error and no
retry authority; Runtime does not reinterpret it as acknowledgement uncertainty.

Dropping those proofs deliberately gives up recovery liveness without weakening Effect safety:

- an unknown preparation acknowledgement mints no `CommittedCall`, so that invocation enters no
  provider; if that preparation is nevertheless durable, an entry-once run may park even though
  the provider was never entered;
- cancellation after a direct-new preparation becomes durable may occur before provider entry,
  during the provider future, or after a result but before its conclusion is durable; every such
  case discards the invocation-local proof and may park the run permanently;
- after provider entry, the exact preparation is already durable;
- cold resume never reconstructs entry-once authority from a retained preparation, so an
  unresolved entry-once Effect parks permanently;
- an absorbing Effect may be replaced only under its already-declared bounded absorption contract;
- Pure work may be recomputed and Read work may use its declared replacement budget; and
- a typed outcome whose conclusion was not durably retained is lost.

Runtime never claims whether the provider was entered after such cancellation. The retained
preparation proves only that re-entry is unsafe, so cold recovery parks rather than guessing. This
is the explicit safety-over-liveness consequence of removing cross-request custody.

Public success or domain failure is returned only after its conclusion appears in qualified
history. Acknowledgement uncertainty first discards the hot cursor and performs one callback-free
Store reload. If a qualified run exists, Runtime returns its current `RunView`. If no qualified
genesis exists after a successful authoritative reload, it returns
`RunError::Indeterminate { run_id }`; it never invents head sequence zero or exposes the
uncommitted candidate. A failed reload remains the distinct redacted infrastructure error and is
not evidence of absence. Repeated calls are new cold attempts, not resolution of a retained
process-local token.

### 5.7 One durable public run view

Store owns one callback-free projection from qualified reduction into:

```text
RunView {
  run_id,
  head_sequence,
  head_digest,
  state: Runnable | Waiting | Succeeded(RetainedValueView) | Failed(RetainedValueView),
}

RetainedValueView {
  contract_ref,
  value_ref,
  canonical,
}
```

Every `RunView` has a real qualified genesis and head. `Runnable` means the retained head selects
another State, including when the current call stopped at its work bound. `Waiting` means retained
history conservatively permits no immediate action, including an unresolved entry-once Effect.
`Succeeded` contains only the registered root `Output`; `Failed` contains only the registered root
`Failure`. Both retain the exact contract-qualified canonical terminal value. A routed nonterminal
failure remains `Runnable`.

`invoke`, `resume`, and `read` all return this contract. Runtime may project it from an
already-qualified selected head; `read`, replay, and cold recovery use the same Store-owned
projection. App never examines the last `RunFrame` or infers status. Operational absence,
indeterminate admission, conflict, invalid history, identity, capacity, and infrastructure
unavailability remain distinct redaction-safe errors rather than `RunView` variants.

The terminal Program success and failure contracts are already the entry point's reviewed public
types. App serializes either canonical value through the common envelope; there is no optional
projector registry or transport-specific result callback.

Status, terminal result, trace, audit, replay, and export are callback-free projections over
qualified retained history. Their derivation belongs in Store/replay query code. App may invoke
those APIs and map their reviewed result, but it does not inspect `RunFrame`, duplicate reducer
logic, or infer state-machine status itself.

### 5.8 Runtime/App deletion result

Delete from App:

- `suspended`;
- `AdmitRunRequest`, `AdmitRunResponse`, `DriveResponse`, and `PublicRunView` after their one current
  replacements own the transport-independent contract;
- `drive` as a one-State command;
- `finish_runtime_step`;
- `resolve_suspended_run`;
- `retain_suspended`;
- `status_from_frames`; and
- every match over low-level Runtime lifecycle variants.

After the high-level path owns all consumers, stop publicly re-exporting `SpawnStep`, `ResumeStep`,
`RuntimeStep`, `RunSession`, and `ParkedRun`; keep only a smaller private cursor if the high-level
implementation needs one. Delete `PendingConclusion`, `SuspendedRun`, their resolution methods,
and every cross-request replacement. Private Store selection, append-attempt, and `CommittedCall`
proofs remain scoped to one Runtime invocation.

Move State-by-State drive loops from App integration tests to Runtime/live-domain integration
tests. App tests exercise one public invocation or resume result.

## 6. Problem group C: domain-coupled Application composition

### 6.1 Problem situation

`Application` is nominally a generic fixed-tenant facade but currently contains the complete list
of supported use cases:

- Portfolio and EVM configuration values and resolved heads;
- EVM submission and balance binding vectors;
- hard-coded `if` branches for two entry-point IDs;
- planner-specific selector decoding;
- a catalog builder that lists every current domain value and numeric capability;
- central enumeration of every supported Program closure; and
- direct production dependencies on EVM, Portfolio, and Capabilities.

Adding one unrelated off-chain entry point therefore requires editing App fields, its constructor,
catalog registration, dispatch, validation, tests, and eventually both transports. `_configuration`
and `_audit` are retained only to prove constructor identity and are not used for their named
responsibilities.

The coupling also crosses the live/domain boundary: `mfm-evm-live` imports Portfolio and registers
Portfolio Pure States so it can instantiate EVM balance States with `PortfolioContinuation`.
Reusable EVM behavior therefore knows one current caller. Adding another caller would require
editing the EVM live adapter crate.

Replacing these fields with a JSON configuration bag, `Any` map, string-key binding registry, or
generic callback catalog would make the code superficially generic while weakening the exact typed
contracts that the runtime refactor established.

### 6.2 Solution: a typed entry-point registration DSL

`ApplicationBuilder` exposes one generic `entry_point` method. It privately wraps a typed planning
closure after construction succeeds; no public `OperationEntryPoint` trait or second authoring AST
is added.

Conceptually:

```text
entry_point<Request, Configuration, C0, Output, Failure>(
  entry_point_id,
  Arc<ResolvedConfiguration<Configuration>>,
  |request, configuration, sequence: &mut OperationExpansion<C0, Output, Failure>|
      -> Result<(C0, source_refs)>
)
```

`Request`, `C0`, and `Output` use their exact registered `MfmValue` contracts;
`Configuration: MfmConfig`; and `Failure: FailureValue`. The request contract is an ingress type,
not necessarily a State context or retained Program root.

The `OperationExpansion<C0, Output, Failure>` type ties the registered public success and domain
failure contracts to every terminal root path. `Failure` implements the reviewed public failure
value contract; internal failures must pass through typed mapper States before terminality. The
resolved configuration is explicitly shared without requiring `Configuration: Clone`.
`Application::invoke` accepts the entry-point ID plus bounded raw request bytes; Application
remains the authoritative duplicate-key, float-free, canonical decoder and run-identity derivation
boundary.

The closure:

1. receives one strictly decoded typed request;
2. borrows the exact registered typed configuration;
3. captures domain-owned typed binding witnesses;
4. constructs the singular `C0` and its explicit source references; and
5. writes the root `[Operations + States]` through the same `OperationExpansion` DSL.

For example:

```text
Application::builder(reader, runtime)
  .entry_point(
      EVM_SUBMIT_TRANSACTION,
      evm_configuration,
      |request, config, sequence| {
          C0 = construct_submission_input(request, config, submission_bindings)
          sequence.operation(SubmitTransaction(...), Propagate)
          return (C0, source_refs)
      })
  .entry_point(
      PORTFOLIO_SNAPSHOT,
      portfolio_configuration,
      |request, config, sequence| {
          C0 = construct_portfolio_input(request, config)
          sequence.state::<InitializePortfolio>(..., Propagate)
          for configured collection:
              sequence.state::<EnterPortfolioCollection>(..., Propagate)
              sequence.operation(
                  EvmBalanceCollection(...),
                  |failure| failure.state::<MapEvmBalanceFailure>(..., Propagate),
              )
              sequence.state::<ResumePortfolioCollection>(..., Propagate)
          sequence.state::<ConsolidatePortfolio>(..., Propagate)
          return (C0, source_refs)
      })
  .finish()
```

The closure is only the typed external-request-to-root-Operation boundary. It does not define a
second Operation representation. Direct States and child Operations share the same sequence and
the same context continuity checks.

### 6.3 Trusted registration order

Trusted startup composition proceeds in this order:

1. domain composition explicitly contributes required values and capabilities to
   `ProgramCatalogBuilder`;
2. `ProgramCatalogBuilder::finish()` finalizes that catalog without requiring a placeholder
   Program; request-specific documents are qualified later through that catalog;
3. the Store opens with the finalized catalog;
4. trusted domain/live constructors validate their own typed route/binding bundles, and live
   modules install State implementations and exact adapter bindings into
   `RuntimeAssemblyBuilder`;
5. typed configuration revisions are resolved through the Store configuration boundary;
6. Runtime is constructed over that exact assembly and Store opening; and
7. `ApplicationBuilder::entry_point` registers each externally exposed root operation.

Catalog contribution remains explicit trusted composition. Operation expansion and entry-point
invocation do not discover or mutate catalog entries.

Each entry-point registration validates:

- a unique stable entry-point ID;
- Runtime, reader, and configuration-head ownership by the same Store opening through one narrow
  composition check, without a public brand token;
- the registered request boundary and `C0`/root-success/root-failure nominal contracts;
- the fixed-tenant association; and
- the bounded total entry-point count.

Application does not inspect erased closure captures or revalidate domain binding completeness.
The captured bundles are already valid by construction. Runtime validates the exact expanded
Program's complete live State/capability/adapter closure before `RunAdmitted`; there is no second
generic binding registry or installed-bundle witness.

After validation, the resolved typed configuration, root success/failure associations, and
captured typed bindings are privately erased with the invocation closure into a map keyed by
entry-point ID. The map is a dispatch implementation detail, not a public configuration or binding
registry.

### 6.4 Invocation path

`ApplicationBuilder` requires one nonzero bounded authoring-concurrency limit. Invocation acquires
that permit before executing the typed entry-point closure and runs request-specific construction
and expansion off the async executor. Capacity exhaustion returns one redaction-safe Application
overload result before authoring, admission, State execution, or provider entry. The permit is
released after the qualified `C0` and Program are ready; Runtime's own limits continue to own only
Runtime work. Authoring closures never enter Runtime merely to borrow its capacity machinery.

One invocation follows one path:

1. App enforces request byte, canonical JSON, duplicate-key, float, and structural bounds.
2. App finds the privately erased handler by exact entry-point ID.
3. The handler decodes once into its registered `Request` type.
4. It borrows the registered `ResolvedConfiguration<C>` value.
5. The closure constructs `C0` and streams child Operations/States through
   `OperationExpansion`.
6. Capability injection completes during each Access State setup.
7. Expansion produces the immutable State-only Program with exact root success and failure
   contracts.
8. Program and `C0` are qualified under the exact catalog.
9. Runtime validates the exact request-specialized Program before `RunAdmitted`, admits the run
   with the registration's configuration head, and advances to a stable boundary.
10. App returns one transport-independent `RunView` whose terminal value, if present, matches the
    registered success or failure contract.

App continues deriving the deterministic run identity from the fixed tenant, exact entry-point ID,
and canonical request. The generic dispatch path replaces domain switches without changing that
identity rule.

The implementation does not maintain a second `closure(configuration)` API that enumerates every
possible Program. Exact typed live binding bundles are validated when trusted composition
constructs them; the exact request-specialized Program is validated before any durable admission.
An incomplete assembly therefore fails closed before `RunAdmitted`, without duplicating planning
logic or excluding bounded request-specialized Program shapes.

No registration closure, Operation, expander, resolved configuration value, or binding bundle
enters Runtime, Store, replay, or retained Program data.

### 6.5 Configuration and binding rules

Each registered entry point owns one exact `ResolvedConfiguration<C>` and pins that head in every
admission it creates. An operation needing several domain settings defines one typed root
configuration that owns their composition. This RFC adds no multi-head configuration manifest.

Child Operation configuration is explicit typed authoring input. There is no ambient lookup by
type, schema, name, or string key. Bindings are exact domain/live witness types captured by the
configured closure and become existing immutable descriptors in final Access declarations.

Several entry points may share one resolved configuration through the smallest sharing mechanism
supported by that type, such as `Arc`; no `MfmConfig: Clone` requirement or `Any` map is introduced.

Changing configuration creates a newly composed Application/entry-point registration. Existing
runs retain their admitted configuration head and Program. There is no live mutable entry-point
registry, fallback to current configuration, or binding refresh path.

### 6.6 Resulting Application boundary

The final `Application` contains only:

- fixed tenant identity;
- the exact Runtime;
- the callback-free reader/query capability it actually uses;
- one bounded authoring-work permit set; and
- bounded privately erased entry-point handlers.

Delete:

- Portfolio/EVM configuration fields and constructor parameters;
- Portfolio/EVM configuration-head fields;
- submission and balance binding vectors;
- `_configuration` and `_audit`;
- hard-coded entry-point branches and selector types;
- `application_catalog` and `register_catalog_values`;
- `validate_runtime_closure`;
- domain-specific plan wrappers duplicated by the generic registration result;
- `submission_closure_documents` and `snapshot_closure_document` after exact invocation-time
  validation replaces them; and
- production dependencies from `mfm-app` to `mfm-evm`, `mfm-portfolio`, and
  `mfm-capabilities`.

Application remains non-generic as a stored public facade. Genericity exists only in its builder
methods before private type erasure.

## 7. Problem group D: transport layers cannot remain thin

### 7.1 Problem situation

The current CLI and REST binaries are deliberately unavailable because no trusted composition is
supplied. Without a generic Application entry-point surface, making them functional would force
each transport to learn the supported domains, configuration, bindings, Runtime assembly, Store,
and drive loop.

That would duplicate the same switch and lifecycle knowledge already misplaced in App, produce
different CLI/HTTP behavior, and make every new entry point a multi-crate transport change.

### 7.2 Solution: invoke only the common Application contract

Application exposes transport-independent operations conceptually equivalent to:

```text
invoke(entry_point_id, request) -> Result<RunView, RunError>
resume(run_id)                  -> Result<RunView, RunError>
read(run_id)                    -> Result<RunView, RunError>
```

`invoke` is the only domain-operation entry surface. `resume` asks Runtime to progress to the next
stable boundary; it is not a one-State drive operation. `read` is callback-free. Trace, audit,
replay, and export may be exposed through equally generic read-only Application methods.

Thus a transport's only domain-specific mutating action is invoking a registered entry point.
`resume` is a generic mutating run-lifecycle continuation; the remaining methods are read-only
projections. None is a route to construct or execute an unregistered domain operation.

Every successful response is the same reviewed Store-derived `RunView`. Domain values remain
contract-qualified, canonical, and strict; transports do not reinterpret them. In particular,
`Runnable` is enough to request another resume: transports do not need a separate yielded status.
`RunError::Indeterminate { run_id }` reports acknowledgement uncertainty when no qualified genesis
is observable and carries no fabricated view.

Any returned `RunView`, including `Runnable`, `Waiting`, or a durable domain `Failed`, is a
successful application exchange. HTTP returns it as a success response and CLI renders it without
inventing transport failure semantics. `Indeterminate` is a retryable transport error that must
include the run id (HTTP `503`; a stable nonzero CLI exit); transports do not automatically invoke
or resume again. Malformed input, absence, conflict, capacity, and redacted infrastructure errors
retain their distinct reviewed mappings.

CLI owns only:

- command and argument parsing;
- remote endpoint selection when used as an HTTP client;
- bounded request-body loading;
- invocation of the common contract;
- text/JSON rendering; and
- exit-code mapping.

HTTP owns only:

- listener address, timeouts, and body limits;
- path and body extraction;
- invocation of the same Application methods;
- public error-to-status mapping; and
- response serialization.

Neither transport owns Program/catalog construction, domain switches, configuration heads,
bindings, adapters, Store mutation, Runtime loops, suspended owners, expansion, provider clients,
signers, nonce authorities, or credentials.

The user's invocation contains only the entry-point ID and that entry point's strict request.
Selectors and idempotency keys are part of the request only when the domain contract requires them.
CLI transport configuration is limited to endpoint/output behavior. HTTP transport configuration
is limited to listener/request behavior.

Application remains fixed-tenant and credential-free. Requests cannot select a tenant, capability,
binding, adapter, or configuration head. Deployment authentication and public network exposure
remain the trusted embedding's separate responsibility and are not reintroduced as transport or
entry-point configuration by this RFC.

Trusted deployment composition constructs `Application` and supplies it to the HTTP serving
library. Provider endpoints, signers, nonce authorities, Store credentials, and entry-point
configuration remain in that embedding/live-adapter boundary. CLI is the runnable remote client
of the HTTP surface and therefore requires no trusted local composition.

`mfm-rest-api` becomes an embeddable transport library exposing
`serve(Arc<Application>, HttpConfig)` and its small client contract. Its current standalone
placeholder binary target is removed. A real deployment executable may exist outside this thin
transport package, but deployment composition is not added to this RFC's transport cutover.

Transport crates depend on the common Application DTO/client boundary, not EVM, Portfolio,
Capabilities, Program authoring, Runtime lifecycle, Store mutation, or live adapters.

## 8. Problem group E: numeric domain types hide semantics

### 8.1 Problem situation

`EvmCapability<const KIND: u8>` uses the values `0`, `1`, `2`, `3`, `6`, and `7` for unrelated
capability contracts. A reader must find a comment or implementation block to learn that, for
example, `<1>` means transaction broadcast and `<3>` means submission-status reads.

The same issue exists in `EvmState<FAMILY, STAGE>` and `PortfolioState<STAGE>`. Binding validation,
runtime registration, adapters, tests, and the new Operation DSL would otherwise continue to
contain opaque pairs such as `EvmState<0, 3>, EvmCapability<3>`.

The const generics save a few marker declarations but impose cognitive cost at every use and make
illegal numeric combinations syntactically easy to write.

### 8.2 Solution: semantic capability types

Delete `EvmCapability<const KIND: u8>` and replace it with named zero-sized types:

- `EvmWalletNonceReservation`;
- `EvmTransactionSubmission`;
- `EvmChainIdentityRead`;
- `EvmSubmissionStatusRead`;
- `EvmAnchorRead`; and
- `EvmBalanceRead`.

`EvmTransactionSubmission` is the broadcast Effect capability and owns the submission pre/post
injection policy. `EvmWalletNonceReservation` is the narrower injected nonce-reservation Effect.
The other capabilities implement empty injection until a real reusable policy requires otherwise.

`EvmBalanceRead` is the semantic name for the existing Read capability used by token-decimals,
native-units, and token-units observations. It preserves that capability's stable ID, intent and
evidence contracts, Read replacement bound, fact rule, and adapter boundary. Each Access State
prepares exactly one provider observation. Request-specific Operation expansion decides whether a
token-decimals occurrence exists; the capability does not combine decimals and units or select a
runtime branch.

The existing macro for repetitive Read capability implementations may remain if it produces
clear diagnostics and less code. Its invocations use named types and semantic families.

### 8.3 Solution: semantic State types

Delete the numeric EVM and Portfolio State marker families. Use named State types for each current
domain transition.

EVM submission:

- `ReserveWalletNonce`;
- `DeriveTransactionCandidate`;
- `BroadcastTransaction`;
- `ReadTransactionReceipt`;
- `ReadFinalizedHead`;
- `ReadCanonicalInclusionBlock`; and
- `ConsolidateSubmission`.

EVM balance collection, retaining the required caller-continuation type parameter:

- `CheckChainIdentity<K>`;
- `ReadInitialAnchor<K>`;
- `ReadTokenDecimals<K>`;
- `ReadAssetBalance<K>`;
- `ConfirmBalanceAnchor<K>`; and
- `ConsolidateBalanceCollection<K>`.

Portfolio:

- `InitializePortfolio`;
- `EnterPortfolioCollection`;
- `ResumePortfolioCollection`;
- `MapEvmBalanceFailure`; and
- `ConsolidatePortfolio`.

Small macros may implement repeated `State` boilerplate, but the public types and every call site
remain semantic. Do not retain numeric aliases, deprecated compatibility names, const-kind escape
hatches, or generic constructors accepting arbitrary family/stage numbers.

### 8.4 Stable identity preservation

Source-name-only replacements preserve every existing stable capability ID, State implementation
ID, intent/evidence type, mode, retry bound, fact rule, schema identity, and binding content.
`EvmBalanceRead` and `ReadTokenDecimals<K>` are such source-level replacements. The State-only EVM
balance redesign retires the successful selector/Match contracts and the separate native-unit and
token-unit State implementation IDs. `ReadAssetBalance<K>` receives one new stable semantic State
ID and a retired ID is never reassigned. Its occurrence still performs exactly one existing
capability observation with the source-specific immutable binding.

`MapEvmBalanceFailure` also receives a new stable State implementation ID because its previously
unused success contract changes to
`EvmBalanceCollectionCompletion<PortfolioContinuation>` so typed recovery can rejoin the normal
Portfolio continuation. Its old implementation ID is retired and never reused; the handler's
current always-fail behavior does not make its declared output contract semantically invisible.

The Program-and-occurrence-address format cutover recalculates canonical Program bytes and
`ProgramRef`s once. Renaming a surviving Rust type must not cause any additional persisted identity
change.

Rust `TypeId` values are process-local associations and may change with the source types. Trusted
composition consistently registers the new types. Persisted identities must not change merely
because source names improve.

## 9. Final Program, admission, and replay boundary

The complete flow is:

```text
strict external request
  -> registered typed entry point
  -> C0 + [Operations and States]
  -> OperationExpansion and capability injection
  -> immutable ProgramDocument { root input/success/failure + State declarations only }
  -> ProgramCatalog and live Runtime validation
  -> RunAdmitted
  -> Store-selected RegisteredState.start
  -> typed Runtime progression over ordinary States
  -> Store / callback-free replay and queries
```

The following may remain process-local in Application for later invocations, but never cross one
invocation's authoring boundary into the finalized Program, Runtime execution input, Store, or
replay:

- Operation values or child boundaries;
- entry-point planning closures;
- capability injection hooks;
- before/after group markers;
- occurrence setup helpers;
- authoring cursors;
- resolved configuration values and typed binding bundles; and
- expansion provenance.

Final Access declarations retain their existing immutable binding descriptors. Admission retains
the selected configuration head and exact final Program. The Program retains the root input,
terminal success, and terminal failure contract references produced by typed expansion. Two
authoring inputs for the same entry-point ID that produce identical canonical declarations and
root contracts produce the same Program identity.

Runtime, Store, resume, replay, audit, export, and import invoke entry-point planners, Operations,
and capability injection zero times. Cold resume uses the admitted Program even when current
configuration or authoring code differs or is unavailable. `ProgramIngress` remains strict and
callback-free; decoding a Program never expands it.

This RFC changes no journal family, Store database schema, canonical hashing rule, capability
intent/evidence contract, or replay-report ownership. It deliberately changes the retained
`ProgramDocument` declaration algebra and the occurrence-address fields carried by
Program/journal/replay once by removing `Match`, adding State-owned failure routing, and replacing
arm paths with ordinal-only `StateAddress`; it also adds the exact terminal failure contract beside
the existing root contracts. The State-only document uses the existing persisted-contract name
`mfm-program-document` at current schema version `2`; its decoder accepts only the new fields and
unsigned-index address representation.
Existing persisted Program, journal, and replay baselines are reset and old bytes are rejected
under the repository's clean-slate cutover policy. There is no version-one decoder under the new
identity and no fallback from the old identity.

## 10. Placement and complete cutover

### 10.1 `mfm-program`

Add only the authoring surface required for:

- typed `Operation` input/output/unhandled-failure contracts;
- streaming `OperationExpansion` over child Operations, States, and their one typed failure-policy
  scope;
- fixed State success continuations and terminal/common/exhaustive variant failure routing;
- exact value-association closed-sum metadata and one private structural variant-tag extractor
  shared by Program qualification and Store routing;
- opaque catalog association handles for Runtime plus canonical-byte capability intent/evidence
  verifiers for Store qualification, with no Store-facing domain `Any`;
- the capability-owned injection trait and before/after State writer;
- centralized Pure and Access State setup;
- centralized capability-mode/configuration/binding validation;
- context continuity, exact root success/failure terminality, failure-policy lowering, and address
  construction;
- fixed recursive/declaration bounds; and
- typed authoring errors.

Replace the Program-requiring catalog finalizer with
`ProgramCatalogBuilder::finish() -> ProgramCatalog`; qualify each expanded document later through
the finalized catalog. Delete the placeholder Program path rather than retaining both APIs.

`OperationExpansion::finish` is the only trusted constructor for a new `ProgramDocument`.
`ProgramIngress` remains the only byte decoder and fully validates the same invariants; final
document/declaration/address fields expose read-only accessors but no public raw constructor or
transition mutator. Hostile ingress tests forge canonical bytes rather than retaining a second
manual authoring API.

Keep helper types private or crate-private unless domain crates must name them. Do not expose an
intermediate expansion algebra for implementation convenience.

Delete the `Declaration` wrapper enum, `MatchDeclaration`, `MAX_MATCH_ARMS`, Match-arm builders,
Match-path address components, payload-projection helpers, and all Match-specific Program and
Store reducer paths in the one State-only format cutover. `ProgramDocument` stores
`Vec<StateDeclaration>` directly. Replace persisted occurrence uses of
`SequentialControlAddress` across
IDs, Program, Journal, Store, and replay with the unsigned-index `StateAddress`; remove the
redundant address and terminal fields plus optional next/failure encoding from each declaration,
require list-index canonicality, and replace address-map/root-discovery/cycle traversal with one
indexed forward validation pass. Assign the new State-only Program schema identity, reset all
affected fixtures, and retain no old decoder.

### 10.2 `mfm-store`

Qualify the retained root input, terminal success, and terminal failure contracts and reduce the
State-only Program through the same callback-free reducer used by live selection and replay.
Expose one projection from a qualified reduction into `RunView`; App and Runtime do not duplicate
it.

Replace Store-facing erased context values with exact canonical objects/references. Retained
State-context material passes the exact association's monomorphic strict validate-and-drop byte
function. Retained capability intent/evidence binding and prior-fact derivation use the catalog's
monomorphic canonical-byte verifier; failure-variant selection uses the schema-derived structural
tag extractor. Store receives no `ErasedProvenValue` or other domain `Any`, and invokes no State or
domain handler.

Collapse admission and conclusion preparation/resolution APIs around one async Store call per
append boundary. Invocation-local attempt material may remain private inside that future, but
acknowledgement-unknown and rejection outcomes return no retry authority. Delete exported
`PreparedAdmission`, `SelectedConclusion`, owner-bearing outcome variants, and resolver methods
after Runtime migrates. Preserve direct-new preparation evidence, append-id uniqueness, exact-head
CAS, same-conclusion/no-longer-selected/conflict classification, and bounded internal fact-frontier
rebinding.

### 10.3 `mfm-runtime`

Replace `QualifiedTypedValue`/`QualifiedValue` with the one
`ProvenValue`/Runtime-private `ErasedProvenValue` correlated family. Replace separate
Pure/Access dynamic registration wrappers with `RegisteredState`, one monomorphic `StateStart` per
exact execution association, one generic State-input downcast, and one private typed lifecycle.
Bind declarations by their complete State/capability association rather than stable implementation
reference alone, then select the occurrence's exact adapter under that proven pair.

Add typed admission/resume entry points sharing a private advance-until-stable implementation and
the Store-owned `RunView` projection used by reads. On acknowledgement uncertainty, discard the
hot cursor and reload qualified history once; return `RunError::Indeterminate { run_id }` when no
qualified genesis exists. Add the nonzero assembly-owned `max_state_starts_per_call`; add no
pending-owner storage or corresponding capacity. Preserve the invocation-local affine
selection/append proofs and direct-new-only `CommittedCall` invariant internally.

Make superseded lifecycle coordination types and methods private or delete them after all internal
and test consumers migrate. Runtime receives finalized Programs only and never depends on
Operation or Application.

Delete the separate Pure/Access dynamic State-input downcasts, implementation-ID-only registry
selection, and superseded dynamic cursor/driver wrappers after every State uses
`RegisteredState.start`.

### 10.4 Domain and live crates

- replace numeric capability and State types with named contracts;
- express EVM submission and balance collection as Operations;
- express Portfolio as its own States plus child Operations;
- specialize native/token balance State sequences from the exact validated request while retaining
  complete contexts and the separate token-decimals checkpoint;
- put submission injection on `EvmTransactionSubmission`;
- implement empty injection for other Access pairings;
- retain typed config and binding bundle validation in domain/live composition;
- preserve every semantically unchanged capability/State identity, assign reviewed new IDs without
  reuse, and freeze the new State-only canonical Programs; and
- expose explicit catalog contributions and the smallest typed entry-point input helpers needed by
  trusted composition.

EVM live composition exposes balance-State/adapter installation parameterized by the caller's
continuation `K`. Trusted composition instantiates it for `PortfolioContinuation`; `mfm-evm-live`
does not import Portfolio or register Portfolio States. Each domain's semantic State registration
belongs to that domain's trusted composition contribution.

Submission and balance expose independent catalog contributions and Runtime installation
functions. Each returns the already-validated typed binding bundle needed by its entry-point
composition; installing one use case does not require configuration or bindings for the other.
Do not replace the current monolith with an optional-field mega-builder.

Delete:

- manual `submission_program` unrolling;
- `append_balance_fragment` and caller-owned declaration vectors;
- manual ordinal and address arithmetic;
- domain-local `pure_state`, `access_state`, and Portfolio equivalents;
- duplicated `validate_access_binding` paths replaced by State setup;
- `EvmSubmissionPlan`, `PortfolioAdmissionPlan`, `plan_submission`, and `plan_snapshot` after their
  typed input construction and Operations move behind entry-point registration;
- `submission_closure_documents`, `snapshot_closure_document`, and all duplicate Program-closure
  enumeration helpers;
- Portfolio State registration and the `mfm-portfolio` production dependency from `mfm-evm-live`;
- numeric capability/State families and all aliases; and
- tests or docs teaching manual declaration construction.

### 10.5 `mfm-app`

Add one `ApplicationBuilder::entry_point` registration DSL, a bounded private handler map, strict
generic dispatch, and the common public `RunView`/`RunError` contracts. Keep domain types out of
stored public App APIs.

Delete all domain fields, switches, catalog lists, closure enumeration, Runtime-step coordination,
raw frame interpretation, unused ports, and direct EVM/Portfolio/Capabilities production
dependencies listed in Sections 5 and 6. Remove App's direct Journal dependency when Store-owned
`RunView` replaces frame inspection.

### 10.6 CLI and HTTP

Convert the current REST package into the embeddable HTTP library described in Section 7.2 and
remove its standalone placeholder binary target. Implement only the common Application invocation
and result contracts. Trusted embedding supplies the HTTP Application; CLI uses the HTTP client or
an explicitly injected Application in tests. Do not add domain or Runtime lifecycle dependencies
to make either transport self-compose.

### 10.7 Documentation

Update `docs/design.md` and `docs/architecture.md` in the relevant cutover commits. Their taxonomy
must describe:

- the one streaming Operation DSL instead of manual declaration construction;
- the State-only Program, ordinal-only occurrence address, fixed success transitions, and
  State-owned failure-variant routing;
- capability-owned authoring injection;
- the one correlated typed/erased value family and registered State start boundary;
- one-call Runtime progression, no cross-request run custody, and conservative cold recovery;
- typed generic entry-point registration;
- explicit trusted composition versus thin transports;
- one root configuration head and terminal public root success/failure contracts; and
- semantic capability and State names.

Update Runtime/App/CLI/HTTP READMEs and every capability/binding inventory at the same time. Remove
superseded examples instead of documenting two paths. Update the checked-in capacity envelope with
request-specialized State counts, including the 578-State maximum Portfolio shape.

## 11. Rejected alternatives and non-goals

This RFC rejects:

- abstract, concrete, protected, direct, wrapper, or expandable State categories;
- a State-level `expand` method;
- capability replacement or suppression of the original State;
- wrapping an already-finalized `StateDeclaration`;
- separately addressed `Match` declarations or successful Program-level variant routing;
- a persisted Operation, entry-point, or expansion Program variant;
- a second entry-point AST, node enum, compiler, or public erased-operation trait;
- caller-supplied child completion/failure addresses, public labels, or declaration-count
  arithmetic;
- separate success/failure continuation writers when success is already lexical;
- child Operations parameterized by caller-specific failure mappers or terminal-only failure
  handlers;
- a recipe registry, `RequiresCapability` inventory, or certification pass;
- automatic catalog discovery or mutation during expansion/invocation;
- JSON/`Any` configuration bags or string-key binding lookup;
- ambient child-configuration discovery;
- multiple independently pinned configuration heads in this cut;
- live mutable entry-point registration or binding refresh;
- exhaustive duplicate Program-closure enumeration;
- transport-specific terminal result projectors;
- public one-State drive APIs;
- App- or transport-owned suspended-owner maps;
- Runtime-owned cross-request append registries, owner-resolution APIs, or permanent-owner disposal;
- a separate process-local `RunProgress`, yielded status, or optional/fabricated durable head;
- a background scheduler, queue, timer, or general per-run lock;
- runtime/replay invocation of authoring code;
- parallel execution, fan-out, multi-input merge, or dynamic scheduling;
- an ambient runtime context map or framework-owned context merge;
- operation-level checkpoints, rollback, compensation, or `finally` semantics;
- numeric capability/State aliases or compatibility escape hatches; and
- old/new compatibility authoring, Runtime, Application, or transport paths.

Parallel execution, durable compensation, multiple independent configuration heads, and a generic
deployment plugin system require separate designs.

## 12. Verification plan

### 12.1 Operation authoring and capability injection

Focused `mfm-program` and `mfm-store` tests prove:

- child Operations and direct States expand in authored order;
- empty capability injection is an identity;
- before/original/after ordering and original-exactly-once behavior;
- pairing-specific typed setup streams heterogeneous before/after States without an erased request
  collection or persisted callback;
- `EvmSubmissionBindings` supplies every injected descriptor and its exact broadcast original;
- before expansion starts at the incoming contract and is validated against `S::Input` only after
  it completes;
- recursive setup and injection for injected Access States;
- exact mode, binding, effect-domain, attempt-bound, fact-selection, and configuration mismatch
  rejection during setup, with live adapter availability rejected later by Runtime validation;
- complete context continuity across direct States, child Operations, injections, and failure
  handler sequences;
- ordinary child success and successful failure recovery both target the next lexical State, while
  child failures target the typed handler and skip the normal continuation;
- a handler consumes the complete item failure, succeeds only with the item's exact externally
  visible output, and propagates only the enclosing Operation's exact failure;
- compile-fail coverage makes `Propagate` unavailable for unequal source/enclosing failure types,
  while qualification rejects a forged unequal explicit variant propagation arm;
- a nested propagated arm targets its caller's failure handler, while only a route still
  propagated after root expansion lowers to `FailureNext::Terminal`;
- common handlers emit after child States, variant handlers emit in stable-tag order, and both are
  skipped by ordinary success;
- injected-State failures either equal the original State failure or are mapped to it before the
  before/original/after suffix commits;
- when an after injection changes the occurrence output, its outer recovery handler must produce
  that completed suffix output and is rejected if it produces only the original `S::Output`;
- fixed success transitions and exhaustive canonically ordered failure-variant routes;
- missing, duplicate, unknown, self, backward, or wrong-input failure targets fail qualification;
- association-owned tag extraction handles each registered enum-tagging profile and rejects
  malformed or mismatched canonical shapes without a hard-coded `kind` field;
- Store selects a failure route from the authenticated tag while preserving the complete failure
  contract, canonical bytes, and value reference;
- prior `Match` Program bytes and old journal occurrence-address records fail current ingress with
  no legacy decoder or migration path;
- catalog finalization requires no Program and request-specific Programs qualify afterward;
- compile-fail coverage exposes no raw trusted `ProgramDocument`/`StateDeclaration` authoring
  constructor outside `OperationExpansion`, while hostile canonical bytes still reenter only
  through `ProgramIngress`;
- terminal-successor rejection;
- every root success/failure terminates under the exact `OperationExpansion` Output/Failure
  contract retained by `ProgramDocument`;
- repeated State occurrences with distinct inputs/config/bindings;
- deterministic depth/declaration bound failures, including direct and mutual injection cycles;
- canonical ordinal-only State addresses independent of caller arithmetic, with unsigned-index
  wire form, no redundant declaration address, no gaps/reordering, and no arm-path component;
- no partial Program on error; and
- zero catalog mutation or ambient IO.

### 12.2 Runtime progression and cold recovery

Runtime tests prove:

- one generic `start_typed` implementation owns every dynamically selected State-input downcast;
- Pure and Access registration both enter through `RegisteredState.start` while retaining their
  distinct typed lifecycles;
- exact association, contract, schema, or `TypeId` mismatch fails before all State, adapter, and
  provider callbacks;
- Runtime assembly derives every registered association from its own catalog; callers cannot
  substitute an association and lookup never falls back to implementation ID alone;
- association handles are opaque and nonconstructible, Runtime is the only production consumer of
  their decode authority, and Store capability verification receives canonical bytes rather than
  a domain `Any`;
- Store's exact association validate-and-drop path rejects retained schema/Serde drift without
  returning an erased or typed domain value;
- live and retained capability intent/evidence use the same exact monomorphic canonical-byte
  verifier for strict decoding, prior-fact selection, and evidence binding;
- identical Rust types from different catalog/value associations cannot be transposed;
- two concrete generic State instantiations sharing one stable implementation ID select their
  exact registered start and adapter associations;
- multiple occurrence bindings reuse one semantic State start while selecting their exact adapters;
- an expanded Program with a missing or mismatched exact State/capability/adapter association is
  rejected before `RunAdmitted`;
- a non-clone input moves exactly once through hot direct advancement, cold reconstruction, and
  failure-variant routing;
- hot advancement performs no retained decode after an exact committed conclusion, while a
  no-longer-selected result discards the local candidate;
- failure routing performs no payload projection or second decode;
- typed admission and resume both use the same advance-until-stable behavior;
- multiple immediately actionable States advance without exposing lifecycle enums;
- terminal success, typed failure, runnable, waiting, indeterminate admission, conflict, invalid
  history, absence, identity, and capacity remain distinct;
- `Succeeded` and `Failed` carry only the Program's exact registered root Output and Failure;
- Read replacement and Effect permanent parking retain current semantics;
- acknowledgement uncertainty discards the hot cursor, exposes no uncommitted candidate, and
  performs no later callback from that cursor;
- cancellation at each point after direct-new Effect preparation performs no cold provider re-entry
  and may leave the run permanently waiting;
- a qualified reload returns the same `RunView` projection as `read`, while uncertain admission
  without qualified genesis returns `RunError::Indeterminate { run_id }` and no fake head;
- unknown preparation acknowledgement produces zero provider entry from that invocation;
- unknown conclusion after an entry-once provider call leaves a durable preparation and cold
  resume performs zero further provider calls;
- an eventually visible conclusion is discovered through cold resume, while an absent conclusion
  leaves the entry-once run waiting permanently;
- Pure recomputation, Read replacement, and absorbing-Effect replacement occur only under their
  existing semantic contracts and bounds;
- concurrent same-head calls still mint at most one direct-new `CommittedCall` through Store CAS;
- the exact `max_state_starts_per_call` is enforced before the next start; a bound stop returns a
  qualified `Runnable` view and resumes without replaying a durable conclusion;
- Runtime and App contain no cross-request cursor, pending-append registry, resolution endpoint,
  or permanent-owner disposal path; and
- acknowledgement-unknown Store outcomes return no append token, and no exported admission or
  conclusion resolver protocol remains.

Invocation-local affine selection, append, and committed-call tests remain in Runtime.
State-by-State EVM/Portfolio drive loops move from App into Runtime/live-domain integration
coverage.

### 12.3 Generic Application entry points

Application tests register at least two unrelated typed entry points, including a small non-EVM
off-chain operation, and prove:

- both dispatch without App source changes or domain switches;
- an entry point may author a direct State, a child Operation, or both in one ordered sequence;
- bounded raw bytes are decoded exactly once by Application and duplicate keys are never hidden by
  transport parsing;
- the registered `Output` and `Failure` are structurally tied to every root terminal contract;
- invalid binding bundles fail trusted domain/live construction; duplicate IDs, foreign
  configuration heads, unregistered contracts, unknown IDs, malformed requests, and over-capacity
  registration fail closed at their owning boundaries;
- authoring concurrency is bounded, expansion runs off the async executor, and overload causes no
  admission, State callback, or provider entry;
- exact request-specialized Program validation fails before `RunAdmitted`;
- private erasure cannot transpose request/configuration/`C0`/Output/Failure associations;
- fixed-tenant run identity and redacted error contracts remain stable;
- terminal roots return only the registered public typed success or failure;
- reconfiguration creates a new registration while old runs retain their admitted head; and
- read/trace/audit/replay/export cause zero live callbacks and use their Store/replay-owned query
  projections.

An architectural dependency test or manifest check proves `mfm-app` has no production dependency
on EVM, Portfolio, or Capabilities.

### 12.4 Thin transports

CLI and HTTP tests use the same fake or real Application contract and prove:

- identical entry-point request/response semantics;
- strict body/argument bounds and redacted error/status mappings;
- CLI needs only endpoint, entry-point request, and rendering options;
- HTTP needs only listener/request settings plus an injected Application;
- neither transport selects domain configuration/bindings or drives Runtime steps; and
- transport production manifests contain no domain, live-adapter, Store-mutation, or Runtime
  lifecycle dependencies.

### 12.5 Named EVM/Portfolio contracts and domain equivalence

Tests prove:

- every semantically unchanged named capability has the exact old stable contract ID, mode,
  attempts, facts, intent/evidence types, and evidence binding;
- `EvmBalanceRead` retains the exact existing balance-read capability ID and contract;
- every semantically unchanged named State has its exact old stable implementation ID and context
  contracts, including the separate `ReadTokenDecimals` checkpoint, while `ReadAssetBalance` has
  one new reviewed State ID;
- retired selector, Match, native-unit, and token-unit State IDs are absent and never reassigned;
- `MapEvmBalanceFailure` has one new reviewed ID and the exact recovery-output/parent-failure
  contracts, while its prior ID is absent and never reassigned;
- no numeric capability/State type or alias remains;
- EVM transaction injection produces the frozen canonical bytes and `ProgramRef` for its new
  State-only seven-occurrence Program;
- native-only, token-only, and mixed-source requests expand to the exact ordered fixed State lists;
- the same validated request privately supplies both the initial context and every source shape;
- the maximum 64 one-token-source Portfolio collections expand to exactly 578 States
  (`2 + 64 * 9`: root endpoints plus enter, six child States, failure mapper, and resume per
  collection), remain below declaration and canonical Program-byte admission bounds, and every
  relevant `+1` input rejects before admission;
- advancing that maximum Program respects the independent per-call Runtime work bound, returning a
  qualified `Runnable` head as often as necessary rather than treating total Program length as one
  call's work budget;
- Portfolio native/token execution retains correct provider selection, anchor checks, failure
  continuations, ordering, duplicate handling, and public results;
- native execution performs no token provider request and token execution performs no native
  provider request;
- token decimals conclude durably before token units, cold resume after decimals invokes only the
  units read, and the two occurrences retain independent replacement exhaustion;
- a context/source-shape mismatch fails before adapter or provider entry;
- EVM balance live registration accepts a caller continuation type without importing that caller,
  and `mfm-evm-live` has no Portfolio production dependency;
- nonce acknowledgement uncertainty, one reservation, candidate lineage, signer binding,
  committed-call gating, confirmation ordering, and no rebroadcast on resume remain intact; and
- live adapter role validation uses only semantic type names.

### 12.6 Final Program and replay isolation

A test-only observable planner/Operation/capability hook proves invocation during authoring and
zero calls during:

- Program qualification;
- live State execution after admission;
- hot and cold resume;
- Store reduction;
- callback-free queries;
- replay and audit; and
- export/import.

Unrelated journal atomicity, persistence, canonicalization, capacity, and security tests remain
unchanged in semantics. Program-ingress and Store/replay routing fixtures are replaced completely
for the one State-only format and contain no legacy decoder, expansion, or entry-point branch.

### 12.7 Complexity audit

The final audit records the Section 1.3 outcomes and demonstrates:

- reduced concepts, duplicated responsibilities, public coordination surface, dependency edges,
  and future entry-point change sites;
- the final net production Rust LOC change, with any increase explained rather than treated as a
  predeclared failure;
- fewer exported lifecycle, coordination, plan-wrapper, registry, and compatibility types/functions,
  with every added semantic named contract justified explicitly;
- removal of App/transport domain dependency edges;
- removal of every superseded helper and compatibility alias; and
- addition of the off-chain test entry point without editing generic App/Runtime/transport source.

### 12.8 Verification commands

Select commands from `docs/build-and-verification.md` based on each commit's affected boundary. Use
the narrowest Nix task first and expand only when risk requires it. Run `nix run .#ci` once on the
final complete tree; do not immediately precede it with redundant `.#check`, `.#test`, and
`.#test-db` runs.

For RFC-only edits, validate repository links and claims and run `git diff --check`.

## 13. Logical implementation sequence

Every commit includes its directly affected contracts, tests, documentation, and deletion scope.
Each leaves one coherent current design; no compatibility alias, old/new API pair, or fallback
survives a cutover commit.

1. **`document reusable operation and entry point follow-up`**

   Replace the authoring-only follow-up with this complete problem/solution contract, settled
   responsibilities, deletion scope, verification plan, and complexity/reuse principle.

2. **`unify runtime state entry and typed execution`**

   Replace the current qualified values with one correlated typed/erased family; install exact
   `RegisteredState.start` closures for Pure and Access execution; move every State-input downcast
   into the one generic start function; bind generic State implementations by their complete
   execution associations; replace Store-facing erased context material with canonical
   objects/references and monomorphic canonical-byte capability verifiers; preserve callback APIs
   over raw typed domain values; and delete the superseded qualified-value and dynamic
   registration/cursor paths in the same Program/Store/Runtime cutover.

3. **`replace app driving with one-call runtime advancement`**

   Collapse Store's exported owner/resolver outcomes; add bounded advance-until-stable behavior and
   the Store-owned `RunView`; migrate the current App and every external consumer in the same
   commit; discard hot execution state on acknowledgement uncertainty; delete suspended-owner
   retention and resolution; preserve direct-new-only Effect entry and conservative cold parking
   while explicitly accepting the documented liveness loss; and move State-driving tests to
   Runtime/live-domain ownership.

4. **`cut over state-only operation authoring and semantic domains`**

   In one inseparable Program/Journal/Store/replay/domain cutover, add the minimal typed Operation
   sequence/failure-policy DSL and capability injection trait; remove `Match`; add root Failure,
   fixed success, and exhaustive stable-tag failure routes; replace occurrence addresses and reset
   old bytes; centralize State setup, context validation, bounds, and address assignment; replace
   numeric EVM/Portfolio contracts; migrate submission, request-specialized balance, and Portfolio;
   make live balance registration generic over the caller continuation; and freeze the new
   canonical Programs. Migrate the existing hard-coded App branches directly to
   `OperationExpansion` so the tree has one authoring path; add no plan facade, alias, old builder,
   decoder, reducer, or compatibility wrapper.

5. **`make application dispatch registered entry points`**

   Add the typed registration DSL and private dispatch; migrate EVM and Portfolio composition;
   remove domain fields, switches, catalog lists, closure enumeration, unused ports, raw frame
   interpretation, and domain production dependencies; add the unrelated off-chain boundary test.

6. **`make cli and http thin entry point transports`**

   Implement the common invocation/resume/read contract, convert REST into an injectable HTTP
   library and remove its placeholder binary, make CLI a thin remote client, and prove both
   transports have no domain or Runtime-lifecycle responsibility.

The final commit report includes the complexity/reuse, LOC, public-surface, dependency, and
change-site audit.

## 14. Acceptance criteria

### 14.1 Simplicity and reuse

- the final tree has fewer concepts, duplicated code paths, public coordination types, dependency
  edges, and entry-point change sites under Section 1.3;
- net production Rust LOC is reduced where possible and its final change is reported without a
  speculative forecast gate;
- one streaming DSL expresses root entry-point sequences, child Operations, and direct States;
- no second entry-point AST, recipe registry, Program compiler, or generic config/binding map
  exists;
- adding the off-chain test entry point changes no generic App, Runtime, CLI, or HTTP source; and
- superseded public types, helpers, dependencies, aliases, tests, and docs are deleted.

### 14.2 Operation and State authoring

- every domain Program is authored through `OperationExpansion`;
- Operations disappear completely after expansion;
- every Operation declares exact authoring-only Input, Output, and unhandled Failure contracts;
- success is lexical; child failure mapping uses one typed scope with no success callback, raw
  destination, or persisted Operation boundary;
- every Operation/State action supplies exactly one policy, with zero-sized `Propagate` available
  only for exact source/enclosing failure equality;
- successful failure recovery reconstructs the item's exact externally visible Output and rejoins
  its normal lexical continuation, while propagated failure is the enclosing Operation's exact
  Failure and only root propagation becomes terminal;
- State remains one non-expandable leaf with no State-level expansion;
- the finalized Program contains a canonically ordered `Vec<StateDeclaration>` with no declaration
  wrapper enum;
- every State owns one fixed success target and a terminal, common, or exhaustive stable-tag
  failure target;
- every Program retains exact root input/success/failure contracts and every root terminal path
  matches them;
- successful enum/option interpretation remains typed State behavior rather than Program routing;
- every nonterminal transition points forward to a declared State and no `Match` type or reducer
  path remains;
- `StateAddress` is only the declaration's canonical unsigned list index; declarations carry no
  redundant address and ingress rejects gaps, reordering, unreachable States, or non-forward
  targets;
- every Access occurrence has one exact compatible capability/configuration/binding setup;
- every usable Access capability implements injection, including the empty case;
- injected States recursively receive normal setup;
- injected-State propagated failures equal or are mapped to the original State failure before
  their setup suffix commits;
- the original State is emitted exactly once between ordered before/after injections;
- expansion is deterministic, bounded, ambient-IO-free, and produces no partial Program;
- complete context contracts connect across every final transition; and
- repeated State implementations with distinct occurrence data remain supported.

### 14.3 Runtime execution and recovery

- one correlated value family retains exact association, canonical bytes, reference, and typed or
  erased domain value without a parallel proof hierarchy;
- every dynamically selected State enters through one exact `RegisteredState.start` and the one
  generic State-input downcast;
- downcast failure stops before every State, adapter, or provider callback and has no fallback;
- Pure and Access callbacks remain statically typed and receive no persistence-proof wrapper;
- hot and cold inputs converge before the same typed lifecycle, and Replay never receives Runtime
  assembly or domain `Any`;
- Store receives canonical objects/references rather than erased State contexts and verifies
  every retained value through its exact strict byte association plus capability binding through
  the exact canonical-byte capability association;
- App/transports invoke only high-level admission/resume APIs returning the Store-derived
  `RunView`;
- Runtime drains immediately actionable States to a stable boundary under fixed bounds;
- acknowledgement uncertainty discards hot execution state and subsequent calls use only freshly
  qualified history;
- cancellation after direct-new Effect preparation may sacrifice liveness but never mints cold
  provider re-entry authority;
- no cross-request run custody, pending-append registry, or resolution API remains;
- only a direct-new preparation can mint the invocation-local `CommittedCall`, so cold resume
  cannot duplicate entry-once provider execution;
- no public one-State drive or suspended-owner resolution API remains;
- no scheduler or general per-run lock is introduced; and
- public success/failure still requires a qualified durable conclusion, while the explicitly
  accepted acknowledgement/cancellation liveness loss is documented and tested.

### 14.4 Generic Application and transports

- `Application` stores no EVM/Portfolio fields, configs, bindings, or selector branches;
- `mfm-app` has no EVM, Portfolio, or Capabilities production dependency;
- `mfm-evm-live` has no Portfolio production dependency and registers no Portfolio State;
- each entry point retains one exact resolved typed root configuration and typed binding closure;
- private dispatch erasure happens only after typed registration validation;
- the exact expanded Program is validated before `RunAdmitted`;
- terminal Program roots are the registered public typed success outputs or domain failures;
- CLI and HTTP use the same generic request/response contract;
- transports receive only minimal transport and invocation configuration; and
- transports contain no domain, Program-authoring, Store-mutation, adapter, or Runtime-lifecycle
  logic.

### 14.5 Domain clarity and stable identity

- numeric EVM capability and EVM/Portfolio State types are absent;
- every capability and State use is readable without a kind/stage comment;
- no compatibility aliases survive;
- every semantically unchanged capability/State retains its stable ID, `EvmBalanceRead` retains the
  existing balance capability contract, retired selector/native-unit/token-unit State IDs are not
  reused, and `ReadAssetBalance` has one new reviewed State ID;
- `MapEvmBalanceFailure` uses a new reviewed ID for its changed recovery output and its old ID is
  not reused;
- every State-only EVM/Portfolio Program matches its checked-in post-cutover canonical golden;
- EVM submission expansion produces the exact post-cutover seven-State Program identity; and
- native/token source shapes are fixed during Operation expansion, token decimals remain a
  separate durable State, and Portfolio no longer manages child declaration counts or address
  ranges.

### 14.6 Final trust boundary

- the persisted Program algebra contains only ordinary State declarations with State-owned
  transitions;
- no Operation, entry-point closure, injection hook, authoring cursor, typed config value, binding
  bundle, or expansion provenance is persisted;
- Runtime, Store, query, resume, replay, audit, export, and import invoke authoring hooks zero
  times;
- journal families, Store backend schema/atomicity, replay-report ownership, and capability
  intent/evidence contracts remain unchanged outside the deliberate
  Program-and-occurrence-address cutover; and
- manual builders, duplicate validation paths, runtime lifecycle leakage, hard-coded domain
  dispatch, numeric markers, transport placeholders, and compatibility paths are absent from the
  final tree.
