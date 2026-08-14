# RFC: simplify operation authoring, runtime control, and application entry points

Status: proposed follow-up target; material blockers under discussion

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
those statements. It does not supersede committed-call gating, acknowledgement safety, fail-fast
State semantics, or any durable trust invariant.

---

## Decision

MFM will have one reusable path at each of three boundaries:

1. `OperationExpansion` is the only Program-authoring and State-setup path. Its small sequence DSL
   accepts child Operations and State occurrences in semantic order. During Access State setup,
   the exact capability may inject ordinary States before and after that occurrence.
2. Runtime owns admission, immediate State advancement, parking, acknowledgement-owner retention,
   resolution, and recovery classification behind one process-facing progression path. Every
   dynamically selected State enters typed execution through one registered `StateStart` boundary
   and one correlated value conversion. App and transports never coordinate Runtime's affine
   lifecycle variants.
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

- **EVM token-read durability and fact granularity.** The simplest State-only balance sequence uses
  one variant-aware `ReadSelectedBalance` Access State with a new
  `EvmSelectedBalanceRead` capability: native evidence carries units, while token evidence carries
  decimals plus units. The current token path persists decimals and balance as two separate
  prepared/concluded States. It is not yet confirmed whether that intermediate checkpoint or
  independently reusable decimals fact is a required domain contract. If it is, consolidating the
  reads would weaken recovery or fact reuse; preserving it requires another fixed-sequence typed
  design without fake provider calls. Resolve by auditing the live adapter, fact selection, retry,
  and replay requirements and approving the exact combined intent/evidence contract before
  implementation planning.
- **Operation failure authoring.** State-owned `FailureNext` defines the final Program contract,
  but the two general streaming actions still do not specify how a parent sends a child
  Operation's success and failure to different later sequences, as Portfolio requires. If this is
  left implicit, implementation will need raw labels, a hidden intermediate declaration algebra,
  or ad hoc address patching at call sites. Resolve by approving one typed
  `Operation::{Input, Output, Failure}` contract and one restricted lexical success/failure
  continuation writer, including canonical emission-order goldens.
- **Cancellation-safe pending-owner cardinality and disposal.** Concurrent same-run CAS attempts
  can create more than one affine append owner, and cancellation can occur while Store
  acknowledgement is awaited. A single `RunId -> owner` entry or post-await insertion can lose
  authority; retaining permanent owners forever can leak bounded capacity. Resolve with a focused
  Runtime/Store custody design and tests covering multiple owners per run, cancellation at every
  append await, restoration during resolution, and one explicit trusted permanent-disposal rule.
- **Progress versus durable query contract.** Admission acknowledgement uncertainty or pre-admit
  capacity failure may have no durable head, and a work bound may stop while a State remains
  runnable. A callback-free `read` also cannot report Runtime-local pending custody. Resolve the
  closed `RunProgress` dispositions, optional last-qualified head, work-bound `Yielded`/`Runnable`
  result, and separate durable Store-owned `RunView` before implementing the process-facing API.
The other previously open choices are fixed by this RFC:

- the State-only Program and ordinal-only occurrence address are one incompatible clean-slate
  format cutover: reset current Program/journal/replay baselines and reject prior `Match` Programs
  and old address records, with no migration, legacy decoder, or dual reducer;
- a Runtime command drains immediately actionable States until the next stable external boundary;
- each admission pins one resolved typed root configuration head;
- the Program's terminal root output is the entry point's public typed success result;
- exact request-specialized Program validation happens before `RunAdmitted`, without a duplicate
  exhaustive Program-closure API;
- numeric EVM and Portfolio State markers are replaced together with numeric capabilities; and
- trusted embedding supplies an already-composed Application to HTTP, while CLI may invoke that
  transport remotely.

## 1. Goals and constraints

### 1.1 Goals

This follow-up has five measurable goals:

- make reusable operation sequencing expressible once and composable everywhere;
- make every dynamically selected State regain its exact typed input through one correlated
  Runtime boundary;
- put each responsibility in one owner instead of duplicating it across domain, App, Runtime,
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

This RFC preserves those invariants while deliberately establishing two new persisted contracts:

- the Program contains only canonically ordered ordinary `State` declarations; and
- each declaration owns its fixed success continuation and terminal, common, or exhaustive
  variant-selected failure continuation.

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
primary evidence is fewer concepts, owners, public coordination types, code paths, dependency
edges, and future change sites. A final LOC reduction is desirable and expected where manual
planning and duplicated coordination disappear; any net increase must be explained by necessary
typed guarantees, validation, or usable transport behavior. Moving code to another crate,
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

| Symptom | Knowledge duplicated outside its owner |
| --- | --- |
| `submission_program` manually creates seven declarations | Capability submission policy |
| `append_balance_fragment` reserves ordinals and mutates a caller vector | Reusable child Operation structure |
| App stores Portfolio/EVM config and binding vectors | Typed entry-point planning dependencies |
| App hard-codes two entry-point branches | Extensible entry-point dispatch |
| `application_catalog` lists every domain value and numeric capability | Trusted domain/catalog composition |
| `mfm-evm-live` imports Portfolio and registers its States | Caller-owned child-Operation instantiation |
| Pure and Access registration each downcast selected State input | One correlated Runtime State-start boundary |
| App retains `SuspendedRun` and normalizes Runtime steps | Runtime owner fate and recovery |
| App tests loop up to a fixed number of States | Runtime progression behavior |
| `EvmCapability<0>`, `<1>`, `<2>`, `<3>`, `<6>`, and `<7>` | Capability semantics |

The additional LOC and layers are not required by the safety model. They compensate for missing
reusable boundaries. The solution is intentionally limited to one sequence expander, one
registered State-start/value boundary, one generic entry-point registration method, one high-level
Runtime progression path, and semantic domain types. Everything else is consolidation or
deletion.

## 3. Target ownership

| Owner | Owns | Does not own |
| --- | --- | --- |
| Domain Operation | Ordered reusable child Operations and States, domain context transitions, and typed construction helpers | Runtime execution or transport decoding |
| `OperationExpansion` | The only recursion/flattening path, State setup, capability injection, contract continuity, bounds, addresses, State-owned transitions, and final Program construction | Domain semantics, provider IO, or persisted expansion metadata |
| State | One reusable domain transition with complete input, output, and failure contracts plus declared success/failure continuations | Expansion or awareness that it was injected |
| Access capability | Intent/evidence/mode/fact discipline and deterministic authoring-time pre/post State injection | Address assignment, scheduling, or runtime expansion |
| Adapter/binding | Exact provider implementation and immutable association for one Access occurrence | Operation composition or entry-point dispatch |
| Trusted composition | Explicit catalog contributions, Store/Runtime assembly, live adapters, resolved configurations, and configured entry-point registration | Transport request handling |
| `ApplicationBuilder` | Fixed-tenant entry-point registration, validation, and private heterogeneous dispatch construction | Domain-specific global registries or Runtime stepping |
| `Application` | Strict public input, entry-point lookup, Runtime invocation, public result/error projection, and read-only public queries | Domain sequencing, Store mutation, or lifecycle-owner coordination |
| Runtime | Exact registered-State binding, the sole State-input value conversion, typed execution, admission, immediate advancement, parking, bounded pending-owner retention/resolution, and recovery classification | Operation authoring, transport concerns, or background scheduling |
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

An Operation is an authoring-only sequence of child Operations and State occurrences. It writes to
one `OperationExpansion` in semantic order:

```text
sequence.operation(child_operation)
sequence.state::<S>(occurrence_setup)
sequence.operation(next_child)
sequence.state::<T>(next_setup)
```

These are the only general composition actions. Calls stream into one bounded authoring buffer;
there is no persisted operation tree, public node enum, second Program algebra, or general workflow
model.

Conceptually:

```text
Operation::expand(sequence):
  sequence.operation(child)
  sequence.state::<StateA>(setup_a)
  sequence.state::<StateB>(setup_b)
```

`sequence.operation` recursively invokes the same API. `sequence.state` invokes the one State setup
path described below. Once the root Operation finishes, the expander validates the complete result,
assigns canonical addresses, connects success and State-owned failure continuations, derives
terminality, and constructs the State-only `ProgramDocument`.

The same State implementation may appear repeatedly with different runtime inputs, explicit
configuration, or binding descriptors. Its input/output/failure contracts and Access capability
pairing remain fixed; a different capability contract requires a different semantic State type. A
State occurrence is identified by its final address and immutable declaration, not by inventing
another State category.

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
an exact capability, binding, bounded domain configuration, and failure continuation. It is not an
AST node or persisted wrapper. Keep its helper representation private or minimally public only
where domain code must construct it.

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
contribute an Operation or transition declaration; allocate addresses; select raw successors;
register catalog entries; inspect runtime values; persist a callback; or define rollback,
compensation, or `finally` behavior.

Injected States recursively receive normal setup, so their capabilities may also inject. A fixed
maximum recursive expansion depth makes direct or mutual injection cycles fail deterministically,
and the existing declaration limit is enforced while expanding. Legitimate repeated State types
remain allowed within those bounds. Any error drops the scratch suffix and returns one typed
authoring error with no partial Program.

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
sequences. A terminal State cannot have a successor.

Each successful nonterminal State constructs the complete next context. The framework does not
merge patches, restore an outer context, search history for earlier outputs, or maintain a generic
context map. If a later State needs earlier information, the preceding complete output carries it
explicitly.

An internal authoring cursor may retain only current contract identity and unresolved continuation
while validating expansion. It is compiler state, not a domain context and not serialized.

Before/original/after is success-path order:

- a failed before State skips the original and all after States;
- a failed original skips all after States;
- a failed after State skips all later States; and
- the failed State's typed failure continuation remains authoritative.

A successful conclusion is durable before context advancement. If broadcast concludes and receipt
or finality later stops, resume begins at the unresolved later occurrence. It does not reserve a
new nonce, derive a new candidate, or rebroadcast. Ordinary State conclusions are the complete
recovery boundary; no composite-operation checkpoint is added.

### 4.6 State-owned transitions and failure variants

There is no separately addressed `Match` declaration. Every final declaration is an ordinary
State and owns its transitions:

```text
StateDeclaration {
  on_success: Next,
  on_failure: FailureNext,
}

Next = State(address) | Terminal
FailureNext = Terminal | State(address) | ByVariant { stable_tag -> Next }
```

Successful closed-sum interpretation is domain behavior. A producing State carries the complete
success context plus a typed enum or option identifying which part is present. Its one declared
successor consumes that complete type and either handles the alternatives itself or propagates the
discriminant through a fixed typed State sequence. If Access behavior differs by alternative, a
variant-aware State prepares the corresponding typed capability intent; the Program does not
branch around Access occurrences and an irrelevant provider call must not be invented merely to
simulate a skipped branch.

Failure routing is different because it decides whether and where recovery continues. A State may
terminate every failure, send every failure to one common handler, or route an exact closed failure
sum by stable variant tag:

```text
sequence
  .state::<SubmitTransfer>(...)
  .on_success::<RecordReceipt>(...)
  .match_failure(|failure| {
      failure
        .variant("insufficient_funds")
        .to::<HandleInsufficientFunds>(...)
        .variant("provider_unavailable")
        .to::<HandleUnavailable>(...)
        .variant("invalid_request")
        .terminal()
  })
```

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
are canonically ordered by stable tag.

During callback-free reduction, Store invokes that exact association-owned structural operation
on the already-qualified canonical failure and selects the declared successor. It does not
hard-code a `{"kind","value"}` shape, perform a domain `Any` downcast, decode or project a payload,
or create a new value object. The complete original failure and `ValueRef` remain unchanged, and
Runtime owns no competing routing decision.

Final State declarations have canonical order. `StateAddress` is the canonical ordinal identity in
that fixed list; Match-arm path components are deleted. `OperationExpansion` assigns it and exposes
no public label, ordinal, or address construction API. Every nonterminal success or failure target
is a later State in canonical order; self-targets and backward transitions are rejected. This RFC
adds no parallel branch, fan-out, join, scheduler, or dynamic workflow structure.

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
collection. Each balance Operation writes one fixed State sequence. Native/token selection remains
inside its complete typed context: `SelectBalanceAsset<K>` returns
`EvmBalanceAsset<K>::Native(context)` or `::Token(context)`, and the single
`ReadSelectedBalance<K>` successor prepares a native-read or token-read typed intent from that
variant. Native evidence contains the raw units; token evidence contains the authenticated token
decimals and raw units. Its typed handler restores one complete `EvmBalanceContext<K>` for anchor
confirmation. The common expander assigns addresses and connects continuations after all child and
capability injection.

Portfolio no longer knows the balance Operation's declaration count, starting ordinal, or reserved
address range. Child composition consumes the caller's exact complete current context and returns
one complete successor context. A mismatch requires an explicit caller-owned State; expansion
never coerces, wraps, merges, or interprets context values.

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

This puts recovery policy and owner fate in a transport-facing crate. It also loses distinctions:
the current mapping collapses several conflicts and permanent owner-bearing rejections into one
public internal error, while the suspended map has no Runtime capacity bound or concurrent
resolution state.

The low-level types are useful inside `mfm-runtime`; their public exposure is not a reason for App
to coordinate them. The duplicated dynamic wrappers are likewise implementation residue, not a
second State model.

### 5.2 Solution: one correlated value and registered State start

Replace the current parallel qualified-value and dynamic-registration concepts with one current
correlated value family and one Runtime-owned registered start function. The existing
`QualifiedTypedValue<T>` and `QualifiedValue` are evolved or fully renamed; a parallel
`ProvenValue` hierarchy must not remain:

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
strict retained-value decoder, and optional closed-sum structural metadata plus
`variant_tag(canonical_bytes)` operation. A value proven under one association cannot be re-erased
or accepted under another association merely because both use the same Rust type. Runtime alone
may invoke its typed decoder/downcast authority; Program qualification and Store may use only its
domain-free structural metadata.

`ErasedProvenValue` is not cloneable, serializable, persisted, or part of any public Application,
adapter, Journal, or transport API. Store may carry its opaque proof/canonical material across a
commit result, but has no API that exposes or downcasts the domain `Any`. Runtime alone consumes
the domain value when entering a selected State.

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
      start_typed::<S>(context, input, &input_association, |context, input| {
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
start_typed<S, Run>(context, input, association, run):
  let input: ProvenValue<S::Input> =
    input.into_typed::<S::Input>(association)?
  run(context, input)
```

Pure registration supplies its Pure typed runner to the same function; it does not introduce a
fake capability. “One downcast” means this one private State-input implementation, exercised each
time Runtime enters a dynamically selected State. Capability intent/evidence erasure remains
separately scoped to exact capability registration.

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
runs directly. The typed outcome is canonicalized and schema-validated exactly once under the
bound output or failure association, producing `ProvenValue<S::Output>` or
`ProvenValue<S::Failure>`.

`StateFuture` is a private Runtime future over this lifecycle. It does not create a second public
cursor algebra or expose State-by-State ownership to Application.

### 5.4 One hot and cold entry path

Hot and cold execution differ only in construction of the correlated input:

```text
hot typed outcome -> ProvenValue<T> -> ErasedProvenValue
cold qualified object -> exact association decoder -> ProvenValue<T> -> ErasedProvenValue
```

Both then use the selected `RegisteredState.start` and the same typed lifecycle. Store, not
Runtime, structurally validates and reduces retained history and selects the current occurrence.
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
acknowledgement-unknown conclusion retains the candidate with its affine pending owner; Runtime
does not invoke the State, adapter, or decoder again merely because acknowledgement is unresolved.

A failure variant uses the same handoff without projection: Store selects the declared route from
the stable tag, and the complete correlated failure enters the selected `RegisteredState.start`.
There is no second payload decode.

Replay remains callback-free and receives no Runtime assembly, registered start function, or
domain `Any`. There is no separate cold executor, but Replay continues to own replay reporting
rather than invoking the live Runtime state machine.

### 5.5 Solution: one process-facing Runtime progression path

Runtime will expose typed admission and run resume methods that both delegate to one private
`advance_until_stable` path. They return one small callback-free `RunProgress` contract rather than
the internal owner algebra.

The progression path drains immediately actionable States until the first stable external
boundary:

- terminal success with the exact qualified root result;
- terminal typed domain failure;
- durable waiting or permanent Effect parking;
- retained acknowledgement uncertainty;
- permanent operational block with owner fate preserved;
- conflict or invalid history; or
- redaction-safe capacity/identity failure.

It stops after a fixed work bound, on any non-progressing durable head, or at one of those
boundaries. The bound derives from Program/declaration and Runtime work limits. Runtime does not
spin on preparation rejection, unresolved provider results, or repeated acknowledgement
uncertainty.

This is caller-driven execution, not a scheduler. Runtime creates no background worker, timer,
queue, general per-run lock, or process-wide ownership lease. An embedding or transport invokes
admission/resume explicitly, but it never asks for one raw State step.

### 5.6 Bounded pending-owner ownership

Runtime, which creates and understands `SuspendedRun`, owns a bounded process-local pending-owner
registry. It is held inside the existing clone-shared Runtime identity.

When admission or advancement returns an acknowledgement-unknown owner, Runtime retains it before
returning `RunProgress`. A later resume for that run resolves the exact retained owner before
attempting cold selection. Repeated uncertainty returns it to the registry without provider or
State re-entry.

Resolution uses a narrow in-resolution marker so concurrent callers cannot consume the same affine
owner. A competing request receives a stable busy/capacity disposition. The marker protects only
pending-owner fate; it is not a general run mutex or scheduler.

The registry has an explicit capacity included in `RuntimeLimits`. Capacity exhaustion fails before
discarding an owner. Permanent conclusion rejection retains its exact owner fate until explicitly
classified or deliberately discarded by the trusted Runtime boundary; App cannot accidentally
drop it while mapping an enum.

To make that guarantee real, Runtime reserves a pending-owner slot before consuming an action whose
Store acknowledgement could become suspended. If no slot is available, it performs no append or
provider entry and returns capacity pressure. A conclusive transition releases the reservation; an
acknowledgement-unknown transition transfers it into the retained registry entry. The slot remains
attached while that owner is in resolution and across repeated uncertainty.

Process loss still destroys process-local owners under the existing durability model. Cold resume
then uses only the retained Program and journal, with the same no-unsafe-Effect-reentry rules as
today.

### 5.7 Public result and callback-free queries

`RunProgress` exposes only stable application-relevant facts: run identity, durable head, one
closed disposition, and the qualified terminal root or typed failure when present. It does not
expose `RunSession`, append owners, pending conclusions, or provider evidence.

The terminal Program root is already the entry point's reviewed public result type. App serializes
its canonical value through a common envelope; there is no optional projector registry or
transport-specific result callback.

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

After the high-level path owns all consumers, stop publicly re-exporting lifecycle owner enums and
types whose only external purpose was coordination: `SpawnStep`, `ResumeStep`, `RuntimeStep`,
`RunSession`, `ParkedRun`, `PendingConclusion`, and `SuspendedRun`. Keep only the private/internal
forms required to implement and test Runtime invariants.

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
entry_point<Request, Configuration, C0, Output>(
  entry_point_id,
  Arc<ResolvedConfiguration<Configuration>>,
  |request, configuration, sequence: &mut OperationExpansion<C0, Output>|
      -> Result<(C0, source_refs)>
)
```

The `OperationExpansion<C0, Output>` type ties the registered public result to the root sequence's
terminal success contract. The resolved configuration is explicitly shared without requiring
`Configuration: Clone`. `Application::invoke` accepts the entry-point ID plus bounded raw request
bytes; Application remains the authoritative duplicate-key, float-free, canonical decoder and
run-identity owner.

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
          sequence.operation(SubmitTransaction(...))
          return (C0, source_refs)
      })
  .entry_point(
      PORTFOLIO_SNAPSHOT,
      portfolio_configuration,
      |request, config, sequence| {
          C0 = construct_portfolio_input(request, config)
          sequence.state::<InitializePortfolio>(...)
          for configured collection:
              sequence.operation(EvmBalanceCollection(...))
              sequence.state::<ResumePortfolioCollection>(...)
          sequence.state::<ConsolidatePortfolio>(...)
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
- the registered request boundary and `C0`/root-output nominal contracts;
- the fixed-tenant association; and
- the bounded total entry-point count.

Application does not inspect erased closure captures or revalidate domain binding completeness.
The captured bundles are already valid by construction. Runtime validates the exact expanded
Program's complete live State/capability/adapter closure before `RunAdmitted`; there is no second
generic binding registry or installed-bundle witness.

After validation, the resolved typed configuration and captured typed bindings are privately
erased with the invocation closure into a map keyed by entry-point ID. The map is a dispatch
implementation detail, not a public configuration or binding registry.

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
7. Expansion produces the immutable State-only Program.
8. Program and `C0` are qualified under the exact catalog.
9. Runtime validates the exact request-specialized Program before `RunAdmitted`, admits the run
   with the registration's configuration head, and advances to a stable boundary.
10. App returns one transport-independent public response.

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

Several entry points may share one resolved configuration through the smallest ownership mechanism
supported by that type, such as `Arc`; no `MfmConfig: Clone` requirement or `Any` map is introduced.

Changing configuration creates a newly composed Application/entry-point registration. Existing
runs retain their admitted configuration head and Program. There is no live mutable entry-point
registry, fallback to current configuration, or binding refresh path.

### 6.6 Resulting Application boundary

The final `Application` contains only:

- fixed tenant identity;
- the exact Runtime;
- the callback-free reader/query capability it actually uses; and
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
invoke(entry_point_id, request) -> RunResponse
resume(run_id)                  -> RunResponse
read(run_id)                    -> RunResponse
```

`invoke` is the only domain-operation entry surface. `resume` asks Runtime to progress to the next
stable boundary; it is not a one-State drive operation. `read` is callback-free. Trace, audit,
replay, and export may be exposed through equally generic read-only Application methods.

Thus a transport's only mutating domain action is invoking a registered entry point. Generic
resume is run-lifecycle continuation, and the remaining methods are read-only projections; none is
a route to construct or execute an unregistered domain operation.

All responses use one reviewed envelope containing run identity, durable head, stable disposition,
and terminal public result or typed failure when available. Domain values remain canonical and
strict; transports do not reinterpret them.

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
- `EvmSelectedBalanceRead`.

`EvmTransactionSubmission` is the broadcast Effect capability and owns the submission pre/post
injection policy. `EvmWalletNonceReservation` is the narrower injected nonce-reservation Effect.
The other capabilities implement empty injection until a real reusable policy requires otherwise.

Subject to the Material uncertainties decision, `EvmSelectedBalanceRead` is a new Read capability,
not a rename or reuse of the current balance-read capability ID:

```text
SelectedBalanceIntent =
  Native { source, anchor }
  | Token { source, anchor }

SelectedBalanceEvidence =
  Native { raw_units }
  | Token { decimals, raw_units }
```

The native variant performs one provider read. The token variant performs decimals and units reads
inside one committed adapter invocation and authenticates evidence only after both succeed. Retry,
failure, and fact retention apply to that combined observation as one unit; no intermediate
decimals conclusion or separately reusable decimals fact remains. If that contract is not
acceptable, implementation planning stops until another State-only sequence is approved.

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
- `SelectBalanceAsset<K>`;
- `ReadSelectedBalance<K>`;
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
ID, intent/evidence type, mode, retry bound, fact rule, schema identity, and binding content. The
State-only EVM balance redesign retires the current balance-read capability ID plus the three
branch-specific balance-read State IDs and gives `EvmSelectedBalanceRead` and
`ReadSelectedBalance<K>` new stable semantic IDs. A retired ID is never reassigned to either new
contract. Their combined intent/evidence, retry, and fact rules require the explicit decision in
Material uncertainties.

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
  -> immutable ProgramDocument { State declarations only }
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
the selected configuration head and exact final Program. Two authoring inputs producing identical
canonical declarations produce the same Program identity.

Runtime, Store, resume, replay, audit, export, and import invoke entry-point planners, Operations,
and capability injection zero times. Cold resume uses the admitted Program even when current
configuration or authoring code differs or is unavailable. `ProgramIngress` remains strict and
callback-free; decoding a Program never expands it.

This RFC changes no journal family, Store database schema, canonical hashing rule, or
replay-report ownership. It deliberately changes the retained `ProgramDocument` declaration
algebra and the occurrence-address fields carried by Program/journal/replay once by removing
`Match`, adding State-owned failure routing, and replacing arm paths with ordinal-only
`StateAddress`. Existing persisted Program, journal, and replay baselines are reset and old bytes
are rejected under the repository's clean-slate cutover policy. Capability intent/evidence changes
only for the unresolved selected-balance contract described in Material uncertainties.

## 10. Placement and complete cutover

### 10.1 `mfm-program`

Add only the authoring surface required for:

- typed `Operation` input/output contracts;
- streaming `OperationExpansion` over child Operations and States;
- fixed State success continuations and terminal/common/exhaustive variant failure routing;
- exact value-association closed-sum metadata and one private structural variant-tag extractor
  shared by Program qualification and Store routing;
- the capability-owned injection trait and before/after State writer;
- centralized Pure and Access State setup;
- centralized capability-mode/configuration/binding validation;
- context continuity, terminality, failure-continuation, and address construction;
- fixed recursive/declaration bounds; and
- typed authoring errors.

Replace the Program-requiring catalog finalizer with
`ProgramCatalogBuilder::finish() -> ProgramCatalog`; qualify each expanded document later through
the finalized catalog. Delete the placeholder Program path rather than retaining both APIs.

Keep helper types private or crate-private unless domain crates must name them. Do not expose an
intermediate expansion algebra for implementation convenience.

Delete `MatchDeclaration`, Match-arm builders, Match-path address components,
payload-projection helpers, and all Match-specific Program and Store reducer paths in the one
State-only format cutover. Replace persisted occurrence uses of `SequentialControlAddress` across
IDs, Program, Journal, Store, and replay with ordinal-only `StateAddress`; reset their fixtures and
retain no old decoder.

### 10.2 `mfm-runtime`

Evolve the existing qualified values into the one correlated typed/erased family. Replace separate
Pure/Access dynamic registration wrappers with `RegisteredState`, one monomorphic `StateStart` per
exact execution association, one generic State-input downcast, and one private typed lifecycle.
Bind declarations by their complete State/capability association rather than stable implementation
reference alone, then select the occurrence's exact adapter under that proven pair.

Add one high-level progress result and typed admission/resume entry points sharing a private
advance-until-stable implementation. Move bounded pending-owner retention into `RuntimeInner` and
extend `RuntimeLimits` with its capacity. Preserve all affine owner and committed-call invariants
internally.

Make superseded lifecycle coordination types and methods private or delete them after all internal
and test consumers migrate. Runtime receives finalized Programs only and never depends on
Operation or Application.

Delete the separate Pure/Access dynamic State-input downcasts, implementation-ID-only registry
selection, and superseded dynamic cursor/driver wrappers after every State uses
`RegisteredState.start`.

### 10.3 Domain and live crates

- replace numeric capability and State types with named contracts;
- express EVM submission and balance collection as Operations;
- express Portfolio as its own States plus child Operations;
- replace native/token Program branching with complete-context, variant-aware State semantics;
- put submission injection on `EvmTransactionSubmission`;
- implement empty injection for other Access pairings;
- retain typed config and binding bundle validation in domain/live composition;
- preserve every semantically unchanged capability/State identity, assign reviewed new IDs without
  reuse, and freeze the new State-only canonical Programs;
  and
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

### 10.4 `mfm-app`

Add one `ApplicationBuilder::entry_point` registration DSL, a bounded private handler map, strict
generic dispatch, and common public run envelopes. Keep domain types out of stored public App APIs.

Delete all domain fields, switches, catalog lists, closure enumeration, Runtime-step coordination,
raw frame interpretation, unused ports, and direct EVM/Portfolio/Capabilities production
dependencies listed in Sections 5 and 6. Remove App's direct Journal dependency when Store-owned
`RunView` replaces frame inspection.

### 10.5 CLI and HTTP

Convert the current REST package into the embeddable HTTP library described in Section 7.2 and
remove its standalone placeholder binary target. Implement only the common Application invocation
and result contracts. Trusted embedding supplies the HTTP Application; CLI uses the HTTP client or
an explicitly injected Application in tests. Do not add domain or Runtime lifecycle dependencies
to make either transport self-compose.

### 10.6 Documentation

Update `docs/design.md` and `docs/architecture.md` in the relevant cutover commits. Their taxonomy
must describe:

- the one streaming Operation DSL instead of manual declaration construction;
- the State-only Program, ordinal-only occurrence address, fixed success transitions, and
  State-owned failure-variant routing;
- capability-owned authoring injection;
- the one correlated typed/erased value family and registered State start boundary;
- Runtime-owned process progression and pending-owner fate;
- typed generic entry-point registration;
- explicit trusted composition versus thin transports;
- one root configuration head and terminal public root output; and
- semantic capability and State names.

Update Runtime/App/CLI/HTTP READMEs and every capability/binding inventory at the same time. Remove
superseded examples instead of documenting two paths.

## 11. Rejected alternatives and non-goals

This RFC rejects:

- abstract, concrete, protected, direct, wrapper, or expandable State categories;
- a State-level `expand` method;
- capability replacement or suppression of the original State;
- wrapping an already-finalized `StateDeclaration`;
- separately addressed `Match` declarations or successful Program-level variant routing;
- a persisted Operation, entry-point, or expansion Program variant;
- a second entry-point AST, node enum, compiler, or public erased-operation trait;
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
- a background scheduler, queue, timer, or general per-run lock;
- runtime/replay invocation of authoring code;
- parallel execution, fan-out, join, or dynamic scheduling;
- an ambient runtime context map or framework-owned context merge;
- operation-level checkpoints, rollback, compensation, or `finally` semantics;
- numeric capability/State aliases or compatibility escape hatches; and
- old/new compatibility authoring, Runtime, Application, or transport paths.

Parallel execution, durable compensation, multiple independent configuration heads, and a generic
deployment plugin system require separate designs.

## 12. Verification plan

### 12.1 Operation authoring and capability injection

Focused `mfm-program` tests prove:

- child Operations and direct States expand in authored order;
- empty capability injection is an identity;
- before/original/after ordering and original-exactly-once behavior;
- pairing-specific typed setup streams heterogeneous before/after States without an erased request
  collection or persisted callback;
- `EvmSubmissionBindings` supplies every injected descriptor and its exact broadcast original;
- before expansion starts at the incoming contract and is validated against `S::Input` only after
  it completes;
- recursive setup and injection for injected Access States;
- exact mode, binding, adapter, effect-domain, attempt-bound, fact-selection, and configuration
  mismatch rejection;
- complete context continuity across direct States, child Operations, injections, and failure
  handler sequences;
- fixed success transitions and exhaustive canonically ordered failure-variant routes;
- missing, duplicate, unknown, self, backward, or wrong-input failure targets fail qualification;
- association-owned tag extraction handles each registered enum-tagging profile and rejects
  malformed or mismatched canonical shapes without a hard-coded `kind` field;
- Store selects a failure route from the authenticated tag while preserving the complete failure
  contract, canonical bytes, and value reference;
- prior `Match` Program bytes and old journal occurrence-address records fail current ingress with
  no legacy decoder or migration path;
- catalog finalization requires no Program and request-specific Programs qualify afterward;
- terminal-successor rejection;
- repeated State occurrences with distinct inputs/config/bindings;
- deterministic depth/declaration bound failures, including direct and mutual injection cycles;
- canonical ordinal-only State addresses independent of caller arithmetic, with no arm-path
  component;
- no partial Program on error; and
- zero catalog mutation or ambient IO.

### 12.2 Runtime progression and owner fate

Runtime tests prove:

- one generic `start_typed` implementation owns every dynamically selected State-input downcast;
- Pure and Access registration both enter through `RegisteredState.start` while retaining their
  distinct typed lifecycles;
- exact association, contract, schema, or `TypeId` mismatch fails before all State, adapter, and
  provider callbacks;
- Runtime assembly derives every registered association from its own catalog; callers cannot
  substitute an association and lookup never falls back to implementation ID alone;
- identical Rust types from different catalog/value associations cannot be transposed;
- two concrete generic State instantiations sharing one stable implementation ID select their
  exact registered start and adapter associations;
- multiple occurrence bindings reuse one semantic State start while selecting their exact adapters;
- a non-clone input moves exactly once through hot direct advancement, cold reconstruction, and
  failure-variant routing;
- hot advancement performs no retained decode after an exact committed conclusion, while a
  no-longer-selected result discards the local candidate;
- failure routing performs no payload projection or second decode;
- typed admission and resume both use the same advance-until-stable behavior;
- multiple immediately actionable States advance without exposing lifecycle enums;
- terminal success, typed failure, waiting, acknowledgement pending, conflict, invalid history,
  absence, identity, and capacity remain distinct;
- Read replacement and Effect permanent parking retain current semantics;
- acknowledgement owners are retained before public return and automatically resolved later;
- repeated uncertainty causes zero duplicate State/provider entry;
- concurrent resolution cannot consume one affine owner twice;
- pending-owner capacity is bounded and exhaustion does not discard an owner;
- pending-owner capacity is reserved before any possibly suspending append or provider entry;
- permanent conclusion rejection preserves owner fate;
- progress stops on work bound or non-progressing head; and
- process loss followed by cold resume preserves all existing safety guarantees.

Low-level affine lifecycle tests remain in Runtime. State-by-State EVM/Portfolio drive loops move
from App into Runtime/live-domain integration coverage.

### 12.3 Generic Application entry points

Application tests register at least two unrelated typed entry points, including a small non-EVM
off-chain operation, and prove:

- both dispatch without App source changes or domain switches;
- an entry point may author a direct State, a child Operation, or both in one ordered sequence;
- bounded raw bytes are decoded exactly once by Application and duplicate keys are never hidden by
  transport parsing;
- the registered `Output` is structurally tied to the root expansion's terminal contract;
- invalid binding bundles fail trusted domain/live construction; duplicate IDs, foreign
  configuration heads, unregistered contracts, unknown IDs, malformed requests, and over-capacity
  registration fail closed at their owning boundaries;
- authoring concurrency is bounded, expansion runs off the async executor, and overload causes no
  admission, State callback, or provider entry;
- exact request-specialized Program validation fails before `RunAdmitted`;
- private erasure cannot transpose request/configuration/`C0` types;
- fixed-tenant run identity and redacted error contracts remain stable;
- the terminal root is returned as the public typed result;
- reconfiguration creates a new registration while old runs retain their admitted head; and
- read/trace/audit/replay/export cause zero live callbacks and use owning query projections.

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
- the old balance-read capability ID is retired and never reassigned;
- `EvmSelectedBalanceRead` has one new reviewed ID and exact closed native/token intent/evidence,
  combined retry, failure, and fact contract;
- every semantically unchanged named State has its exact old stable implementation ID and context
  contracts, while `ReadSelectedBalance` has one new reviewed ID and contract;
- retired native/token branch State IDs are absent and never reassigned;
- no numeric capability/State type or alias remains;
- EVM transaction injection produces the frozen canonical bytes and `ProgramRef` for its new
  State-only seven-occurrence Program;
- Portfolio native/token execution retains correct provider selection, anchor checks, failure
  continuations, ordering, duplicate handling, and public results through variant-aware States;
- native execution performs no token provider request and token execution performs no native
  provider request;
- partial token observation produces no authenticated evidence, retained fact, or State
  conclusion, and retry repeats the combined observation under the same committed call rules;
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
   ownership, deletion scope, verification plan, and complexity/reuse principle.

2. **`replace match and numeric markers with semantic state transitions`**

   After resolving the selected-balance evidence contract, remove `Match` in one complete
   Program/Journal/Store/replay/domain cutover; add fixed success and
   exhaustive stable-tag failure routes to `StateDeclaration`; replace EVM capability kinds and
   EVM/Portfolio State stages with their final semantic types; consolidate native/token reads into
   the approved variant-aware State contract; preserve every unchanged stable identity; freeze
   the new canonical Program goldens; and leave no retired ID reuse, alias, numeric escape hatch,
   old decoder, reducer, builder, or compatibility path.

3. **`unify runtime state entry and typed execution`**

   Evolve the current qualified values into one correlated typed/erased family; install exact
   `RegisteredState.start` closures for Pure and Access execution; move every State-input downcast
   into the one generic start function; bind generic State implementations by their complete
   execution associations; preserve callback APIs over raw typed domain values; and delete the
   superseded dynamic registration/cursor paths.

4. **`move run coordination behind runtime progress`**

   Add bounded advance-until-stable behavior and pending-owner ownership; migrate App and every
   external consumer in the same commit; make low-level lifecycle coordination internal; preserve
   every retry, Effect-entry, acknowledgement, conclusion, and cold-resume invariant; and move
   State-driving tests to Runtime/live-domain ownership.

5. **`cut over authoring to capability state injection`**

   Add the minimal Operation sequence DSL and capability injection trait; centralize State setup,
   context validation, bounds, address assignment, and continuations; migrate EVM submission, EVM
   balance, and Portfolio; make EVM live balance registration generic over the caller continuation
   and remove its Portfolio dependency; reproduce the frozen State-only Program identities; delete
   every manual declaration builder, ordinal calculation, and duplicate binding check; and retain
   any existing App-facing plan facade only as the sole thin consumer of `OperationExpansion`
   until the next commit.

6. **`make application dispatch registered entry points`**

   Add the typed registration DSL and private dispatch; migrate EVM and Portfolio composition;
   remove the temporary thin plan facade, domain fields, switches, catalog lists, closure
   enumeration, unused ports, plan wrappers, raw frame interpretation, and domain production
   dependencies; add the unrelated off-chain boundary test.

7. **`make cli and http thin entry point transports`**

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
- State remains one non-expandable leaf with no State-level expansion;
- the finalized Program contains only a canonically ordered finite sequence of State declarations;
- every State owns one fixed success target and a terminal, common, or exhaustive stable-tag
  failure target;
- successful enum/option interpretation remains typed State behavior rather than Program routing;
- every nonterminal transition points forward to a declared State and no `Match` type or reducer
  path remains;
- every Access occurrence has one exact compatible capability/configuration/binding setup;
- every usable Access capability implements injection, including the empty case;
- injected States recursively receive normal setup;
- the original State is emitted exactly once between ordered before/after injections;
- expansion is deterministic, bounded, ambient-IO-free, and produces no partial Program;
- complete context contracts connect across every final transition; and
- repeated State implementations with distinct occurrence data remain supported.

### 14.3 Runtime ownership

- one correlated value family retains exact association, canonical bytes, reference, and typed or
  erased domain value without a parallel proof hierarchy;
- every dynamically selected State enters through one exact `RegisteredState.start` and the one
  generic State-input downcast;
- downcast failure stops before every State, adapter, or provider callback and has no fallback;
- Pure and Access callbacks remain statically typed and receive no persistence-proof wrapper;
- hot and cold inputs converge before the same typed lifecycle, and Replay never receives Runtime
  assembly or domain `Any`;
- App/transports invoke only high-level admission/resume progress APIs;
- Runtime drains immediately actionable States to a stable boundary under fixed bounds;
- Runtime alone retains and resolves pending acknowledgement owners;
- concurrent resolution cannot duplicate owner consumption or provider entry;
- no public one-State drive or suspended-owner resolution API remains;
- no scheduler or general per-run lock is introduced; and
- all existing Effect, durability, conflict, and cold-resume guarantees remain intact.

### 14.4 Generic Application and transports

- `Application` stores no EVM/Portfolio fields, configs, bindings, or selector branches;
- `mfm-app` has no EVM, Portfolio, or Capabilities production dependency;
- `mfm-evm-live` has no Portfolio production dependency and registers no Portfolio State;
- each entry point retains one exact resolved typed root configuration and typed binding closure;
- private dispatch erasure happens only after typed registration validation;
- the exact expanded Program is validated before `RunAdmitted`;
- terminal Program roots are the public typed success outputs;
- CLI and HTTP use the same generic request/response contract;
- transports receive only minimal transport and invocation configuration; and
- transports contain no domain, Program-authoring, Store-mutation, adapter, or Runtime-lifecycle
  logic.

### 14.5 Domain clarity and stable identity

- numeric EVM capability and EVM/Portfolio State types are absent;
- every capability and State use is readable without a kind/stage comment;
- no compatibility aliases survive;
- every semantically unchanged capability/State retains its stable ID, retired balance capability
  and branch-specific State IDs are not reused, and `EvmSelectedBalanceRead` plus
  `ReadSelectedBalance` have new reviewed IDs;
- every State-only EVM/Portfolio Program matches its checked-in post-cutover canonical golden;
- EVM submission expansion produces the exact post-cutover seven-State Program identity; and
- Portfolio no longer manages child declaration counts or address ranges.

### 14.6 Final trust boundary

- the persisted Program algebra contains only ordinary State declarations with State-owned
  transitions;
- no Operation, entry-point closure, injection hook, authoring cursor, typed config value, binding
  bundle, or expansion provenance is persisted;
- Runtime, Store, query, resume, replay, audit, export, and import invoke authoring hooks zero
  times;
- Store/journal/replay contracts outside the deliberate Program-and-occurrence-address cutover and
  the approved selected-balance capability evidence change are unchanged; and
- manual builders, duplicate validation paths, runtime lifecycle leakage, hard-coded domain
  dispatch, numeric markers, transport placeholders, and compatibility paths are absent from the
  final tree.
