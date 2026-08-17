# RFC: semantic domain contracts, Operation expansion, and capability-owned injection

Status: accepted design; implementation pending

Relationship: this RFC follows the implemented
`RFC_RUNTIME_STORE_JOURNAL_TYPED_PROOF.md`,
`IMPL_PLAN_RFC_RUNTIME_STORE_JOURNAL_TYPED_PROOF.md`, and
`FIXES_TT1_IMPL_PLAN_RFC_RUNTIME_STORE_JOURNAL_TYPED_PROOF.md`. Those cutovers own Program v2,
Runtime association and progression, the exact Journal wire, mechanical Store persistence,
PostgreSQL durability, explicit `RunId`, and the current Application boundary. This RFC does not
restate or reopen those contracts.

This RFC replaces every earlier revision of the deferred follow-up. Git history is the archive for
its State-only Program, numeric-address, broad failure-policy, EVM submission, generic Application,
transport, configuration, fact, replay, lifecycle, and `ProgramAuthor` proposals. None of that
superseded text is an implementation input.

---

## Decision

MFM will make exactly two current changes:

1. Replace the surviving numeric EVM capability, EVM State, and Portfolio State families with
   semantic Rust types. Every stable identity, associated value contract, Program declaration,
   canonical Program byte, and Program content reference remains unchanged.
2. Add one small authoring-only `Operation` DSL. A reusable typed `Operation` expands States and
   child Operations through the sole `OperationExpansion` compiler context. Each typed Read
   occurrence invokes the exact capability/State pair's deterministic before/original/after
   injection policy during expansion. Expansion produces one private flat symbolic draft, then
   resolves it once into the existing immutable Program v2 graph.

There is no `ProgramAuthor`, no `OperationExpansion -> ProgramAuthor -> Program` handoff, and no
second authoring authority. `OperationExpansion` owns typed occurrence setup, nested expansion,
capability injection, structured Match joins, scoped failure handlers, atomic scratch scopes, fixed
bounds, final `u16` resolution, and the one call into Program's existing
validator/canonical encoder.

An Operation is not persisted and is not a Runtime concept. It is ordinary deterministic Rust
authoring code with exact associated Input, Output, and Failure value contracts. An Operation value
owns or borrows checked per-occurrence configuration. Repeating the same Operation with different
configuration is an ordinary bounded Rust loop over distinct Operation values. Repeating the same
State type with different checked capability setup is likewise ordinary and legal.

Capability injection is authoring-time topology only. It performs no IO, reserves no nonce, signs
nothing, calls no provider, registers no adapter, and grants no execution authority. The expansion
kernel emits the designated wrapped Read occurrence exactly once between its before and after
suffixes. Hooks cannot access, replace, suppress, retarget, or emit that private Read occurrence.
Repeated occurrences of the same Read type remain legal through ordinary
`OperationExpansion::read`; each independently applies its exact pairing policy.

The DSL uses the same structured `match_join` construct for ordinary value-dependent topology and
for recovery selected by a Pure failure-classifier State. `with_failure_handler` routes an exact
failure contract from one State, child Operation, or larger protected region into such a handler.
There is no public label, route, successor, branch outcome, or failure-policy algebra.

The retained Program algebra remains exactly `Declaration::{State, Match}` with declaration-array
identity, root index zero, strictly forward `u16` successors, optional success/failure successors,
and the existing Match projection. Operation boundaries, configuration, hooks, structured scopes,
and private draft routes are erased before Program construction. No Program schema, Program bytes,
Journal frame, Store row, PostgreSQL table, Runtime reducer, or persisted identity changes in this
RFC.

## Material uncertainties

1. **Future durable Effect shape**
   - **Choice:** freeze the Operation compiler and deterministic capability-owned
     before/original/after expansion for the current Read-only system. A future Effect emission
     method is not added here; nested Read emission is likewise deferred until a concrete injected
     topology requires it.
   - **Why uncertain:** the transaction-authority/outbox contract has not fixed its Effect ABI,
     durable command proof, nonce fencing, ambiguous provider acknowledgement, signer custody,
     reconciliation, or exact failure contracts.
   - **Consequence if wrong:** the future Effect RFC may need to extend the injection writer or add
     one deliberate Operation failure construct. It must not add a second compiler or reinterpret
     authoring topology as execution authority.
   - **Resolution:** before approving any Effect Program/Runtime/Journal cutover, prototype the
     exact `reserve -> derive/sign -> broadcast -> observe/confirm -> consolidate` Operation and
     its durable authority boundary against this expansion model.

2. **Forward-only recovery semantics**
   - **Choice:** a handled failure may recover only by producing the handler scope's exact join
     contract `J`, or may terminate successfully by producing a distinct enclosing Operation
     output `O`. When `J == O`, an open result is always a join; direct same-contract termination is
     unavailable, while a nested already-closed terminal remains closed. It cannot retry or jump
     backward.
   - **Why uncertain:** Portfolio's current failure handler is terminal; there is no current
     production handler that recovers to a join, and `EvmBalanceFailure` does not currently carry
     a recoverable continuation context.
   - **Consequence if wrong:** retry or loop semantics require a deliberate Program contract
     change. A real `J == O` early-exit need would require one separately justified terminal
     mechanism; neither can be smuggled into this cut.
   - **Resolution:** exercise the first concrete classified recovery with a domain-owned complete
     failure/selector payload and retain a synthetic hot/cold recovery proof now. Add no public
     route escape hatch or kernel-owned ambient context.

3. **Selector descriptor breadth**
   - **Choice:** source authoring supports the same external/adjacent, single embedded nominal
     payload selector shapes that Runtime Match association supports today.
   - **Why uncertain:** generic selector descriptors must be proven across both tagging styles and
     future payload types, not only `EvmBalanceAsset<K>`.
   - **Consequence if wrong:** a future selector shape may need descriptor support, but does not
     justify raw labels or an alternate branch API.
   - **Resolution:** add generic external/adjacent authoring and Runtime association tests plus
     hostile internal/unit/unsupported-shape cases.

4. **Trusted open authoring callbacks**
   - **Choice:** treat `Operation::expand` and capability-injection hook implementations as trusted
     deterministic authoring code. Child composition must use `OperationExpansion::operation`, and
     hooks may emit support topology only through `InjectionWriter`. The repository scanner
     forbids direct hook-to-hook composition in supported production roots.
   - **Why uncertain:** Rust cannot let downstream crates implement these open traits while also
     making direct calls to another public `expand`, `original_binding_ref`, `write_before`, or
     `write_after` method mechanically impossible.
   - **Consequence if wrong:** direct `Operation::expand` can bypass child-scope atomicity and
     associated-contract/depth accounting. Direct hook calls retain the restricted writer's suffix
     continuity checks but bypass capability-policy ownership and kernel callback-depth accounting.
     The final Program validator still enforces persisted graph structure.
   - **Resolution:** approve the same trusted-code boundary already used for State implementations.
     If arbitrary downstream Operation implementations must be mechanically sandboxed, redesign
     the invocation API before implementation planning rather than adding a second builder or a
     partial runtime check.

No other material uncertainty blocks the current semantic-name and Read-only Operation-authoring
cutover.

## 1. Goals and hard constraints

### 1.1 Goals

The implementation must:

- replace opaque numeric contract spellings with semantic Rust names;
- expose one understandable DSL for reusable typed Operations composed from States and child
  Operations;
- permit repeated State and Operation occurrences with distinct checked authoring configuration;
- give each exact capability/State pair one deterministic owner for authoring-time injection;
- make injected declaration-count changes invisible to parent Operations;
- centralize typed State/capability derivation, symbolic routing, bounds, and final Program
  construction in `OperationExpansion`;
- remove caller-managed declaration indices and parent knowledge of child declaration counts;
- preserve exact existing Program v2 bytes and identities for every current valid plan;
- keep all authoring values, callbacks, structured scopes, and scratch state out of Program,
  Runtime, Journal, Store, and cold recovery; and
- reduce combined kernel/domain authoring complexity and future change sites without introducing a
  second Program algebra.

### 1.2 Non-negotiable preserved contracts

This RFC does not change:

- the `Program` schema identity/version or canonical wire;
- `Declaration::{State, Match}` or Match payload projection;
- declaration-array identity, root index zero, forward `u16` edges, reachability, or root
  success/failure rules;
- State execution semantics, Read capability ABI, or stable-ID derivations;
- `RuntimeAssemblyBuilder`, Program association, `Runtime::start/resume/read`, `RuntimeError`,
  `RunView`, or caller-driven progression;
- `mfm.run.frame.v1`, Journal qualification, frame heads, Store APIs, PostgreSQL schema, limits, or
  durability classification;
- `EvmPhysicalTarget` binding identity or live provider association;
- the current Portfolio request, C0, success/failure values, public JSON fixtures, or provider
  behavior; or
- the retirement of submission, nonce storage, broadcast/status paths, generic Effect, facts,
  replay, configuration publication, and lifecycle ownership.

### 1.3 Simplicity budget

The cutover adds five logical public authoring concepts in `mfm-program`:

- `Operation`;
- scoped `OperationExpansion`;
- scoped `MatchJoin`;
- `CapabilityInjection<S>`; and
- restricted `InjectionWriter`.

It adds one root function, `expand_program`, and reuses `ProgramError` plus the crate `Result`. It
adds no authoring error enum, public node/recipe enum, public draft, public raw route, address type,
dynamic registry, trait-object collection, stored callback, execution mode, persisted Operation,
or dependency edge.

The callable-item ceiling is exact. `Operation` exposes only `expand`; `CapabilityInjection`
exposes only `original_binding_ref`, `write_before`, and `write_after`; `OperationExpansion`
exposes `pure`, `read`, `operation`, `match_join`, and `with_failure_handler`; `MatchJoin` exposes
only `arm`; and `InjectionWriter` exposes only `pure`. That is eleven trait/compiler/writer methods
or associated functions plus the one root function. Exceeding five
concepts or this callable inventory is an architect `BLOCK`, not an invitation to add aliases.

The same cutover makes the existing raw Program source constructors crate-private and deletes
domain-local declaration constructors, fragment-size forecasts, and index arithmetic. The
implementation report must record:

- production Rust LOC added/deleted per commit and cumulatively;
- public types/functions/methods added and made private/deleted;
- normal internal dependency edges before/after;
- domain files and call sites needed to add one State, one Operation, and one injected pairing; and
- exact Program byte/content-ref equality evidence.

The semantic-name commit intentionally replaces three generic public families with sixteen
explicit semantic types, a net increase of thirteen public names. That increase is accepted only
for semantic domain contracts. Public coordination or wrapper growth is not.

## 2. Implemented baseline and remaining problem

The core proof path is complete:

- Program v2 contains checked State or Match declarations;
- Runtime pre-associates exact value codecs, State drivers, Match projections, and adapters;
- one Runtime fold owns hot/cold progression;
- Journal owns exact canonical history qualification;
- Store owns only complete load and atomic append; and
- Application stores only Runtime and exposes Portfolio start plus Runtime resume/read.

Two source-authoring problems remain.

First, contract semantics are hidden behind numeric axes:

```rust
EvmCapability<2>
EvmState<1, 4, PortfolioContinuation>
PortfolioState<3>
```

Those spellings admit meaningless combinations and make trusted composition, adapter registration,
Program authoring, tests, and review depend on comments.

Second, current domain planners directly construct final declarations. EVM manually derives every
internal balance-fragment index, and Portfolio separately predicts fragment size plus
enter/resume/mapper/terminal positions. The reusable child boundary exists only as a function that
mutates the parent's final declaration vector. A capability-owned pre/post suffix would change
those counts and force edits in the capability, child fragment, parent planner, capacity fixtures,
and tests.

The required abstraction is one pre-Program streaming compiler, not a friendly layer over a second
builder. Typed Operations stream semantic occurrences into `OperationExpansion`; it retains only
one private symbolic draft until every child and injection has expanded. Program is conceived only
when `expand_program` resolves that draft and invokes the existing Program validator.

## 3. Responsibility table

| Owner | Owns | Must not own |
| --- | --- | --- |
| `Operation` | Reusable domain authoring boundary, exact Input/Output/Failure contracts, checked per-instance configuration, and deterministic semantic expansion order | Final indices, raw declarations, IO, Runtime registration, persisted metadata, or provider handles |
| `OperationExpansion` | One private flat draft, child scopes, structured Match/failure scopes, capability setup/injection, atomic merge, bounds, final index resolution, and Program construction | Domain behavior, provider selection, Runtime execution, or a second finalized representation |
| `MatchJoin` | One scoped set of typed selector arms with a common success join and optional exact Operation success termination | Public labels/routes, Runtime Match projection, failure policy, or a second branch representation |
| `CapabilityInjection<S> for C` | Checked setup for the exact capability/State pairing, original binding derivation, and deterministic before/after policy | Runtime capability execution, adapter registration, nonce allocation, signing, provider calls, raw routes, or Program finalization |
| `InjectionWriter` | Restricted lexical Pure State emission into one atomic occurrence suffix | Match, failure handlers, child composition, termination, original-State access, Program finish, Runtime handles, or IO |
| Domain State | One reusable typed Pure or Read transition | Operation expansion, final topology, or knowledge that it was injected |
| Runtime/Journal/Store | The implemented association, execution, history, and persistence boundaries | Any Operation, structured authoring scope, authoring setup, or injection hook |
| Future Effect authority | Durable command authority, idempotence, reservation/fencing, signer/mutation rules, and ambiguity recovery | Treating authoring injection as execution evidence |

## 4. Semantic contract names

### 4.1 Capability mapping

Replace exactly:

| Current source type | Current semantic ID | New source type |
| --- | --- | --- |
| `EvmCapability<2>` | `mfm.evm.capability.read-chain-identity@1` | `EvmChainIdentityRead` |
| `EvmCapability<6>` | `mfm.evm.capability.read-anchor@1` | `EvmAnchorRead` |
| `EvmCapability<7>` | `mfm.evm.capability.read-balance@1` | `EvmBalanceRead` |

The new zero-sized types retain the existing `Intent = EvmReadIntent`,
`Evidence = EvmReadEvidence`, binding checks, operation-family checks, and stable IDs.

### 4.2 EVM State mapping

Replace exactly:

| Current source type | New source type |
| --- | --- |
| `EvmState<1, 0, K>` | `CheckChainIdentity<K>` |
| `EvmState<1, 1, K>` | `ReadInitialAnchor<K>` |
| `EvmState<1, 2, K>` | `SelectBalanceAsset<K>` |
| `EvmState<1, 3, K>` | `ReadNativeBalance<K>` |
| `EvmState<1, 4, K>` | `ReadTokenDecimals<K>` |
| `EvmState<1, 5, K>` | `ReadTokenBalance<K>` |
| `EvmState<1, 6, K>` | `ConfirmBalanceAnchor<K>` |
| `EvmState<1, 7, K>` | `ConsolidateBalanceCollection<K>` |

Every new type preserves its existing exact `state_id()`, Input, Output, Failure, Pure/Read
classification, capability pairing, and evaluation/prepare/interpret implementation. Keep
`EvmBalanceAsset<K>`, the native/token Match, and the separate native, decimals, and token balance
States.

Do not introduce the superseded consolidated `ReadAssetBalance`, request-specialized native/token
Program shapes, or any new State implementation ID.

### 4.3 Portfolio State mapping

Replace exactly:

| Current source type | New source type |
| --- | --- |
| `PortfolioState<0>` | `InitializePortfolio` |
| `PortfolioState<1>` | `EnterPortfolioCollection` |
| `PortfolioState<2>` | `ResumePortfolioCollection` |
| `PortfolioState<3>` | `MapEvmBalanceFailure` |
| `PortfolioState<4>` | `ConsolidatePortfolio` |

`MapEvmBalanceFailure` keeps its current always-fail behavior, Input/Output/Failure contracts, and
stable ID. Its declared success type remains `PortfolioSnapshotOutput`; authoring must not assume
that an implementation always fails and reinterpret it as a recovering mapper.

### 4.4 Complete name cutover

Delete without aliases:

```text
EvmCapability<const KIND: u8>
EvmState<const FAMILY: u8, const STAGE: u8, K>
PortfolioState<const STAGE: u8>
```

Delete numeric-stage comments, macros accepting arbitrary kind/stage parameters, numeric
constructor spellings, compatibility exports, and tests/docs teaching ordinal meanings. Small
macros may generate repeated implementations only when their invocations use semantic types
directly and diagnostics remain clear.

## 5. The Operation DSL

### 5.1 Public model

The public API is conceptually:

```rust
/// A reusable deterministic authoring-only composition of States and child Operations.
pub trait Operation: Sized {
    /// Complete input value expected by this Operation.
    type Input: MfmValue;
    /// Complete success value exposed by this Operation.
    type Output: MfmValue;
    /// Complete failure value propagated by this Operation.
    type Failure: MfmValue;

    /// Expands this configured occurrence into the supplied scoped compiler context.
    fn expand(
        &self,
        expansion: &mut OperationExpansion<
            Self::Input,
            Self::Output,
            Self::Failure,
        >,
    ) -> Result<()>;
}

/// Deterministically expands one root Operation and constructs Program v2.
pub fn expand_program<O: Operation>(
    entry_point_id: EntryPointId,
    root: &O,
) -> Result<Program>;

/// The one scoped pre-Program expansion and lowering context.
pub struct OperationExpansion<I, O, F>
where
    I: MfmValue,
    O: MfmValue,
    F: MfmValue,
{
    // private root/scope contracts, draft, frontiers, handlers, counts, and depth
}

/// One scoped typed Match whose nonterminal arms rejoin at one exact contract.
pub struct MatchJoin<I, O, F>
where
    I: MfmValue,
    O: MfmValue,
    F: MfmValue,
{
    // owned private Match scratch plus root/scope contract markers
}
```

The exact private field/module layout is implementation detail. `OperationExpansion` owns its
scratch; it exposes no public lifetime parameter. All public items require complete rustdoc under
`#![warn(missing_docs)]`. `MatchJoin` owns its scratch, has no public constructor, `Default`, or
`Clone`, and is supplied only by mutable borrow to its one callback. `Operation: Sized`
deliberately rules out `dyn Operation` registries and boxed heterogeneous Operation collections.

Private fields must represent `I`, `O`, and `F` (with a non-owning marker when no typed private
field already does) so the generic scope is a real Rust type contract rather than unused syntax.

`expand_program` is the sole trusted source ingress. It derives the root Input/Output/Failure refs,
creates one root scope, invokes `root.expand` exactly once, seals its remaining scope exits,
resolves the draft, and invokes the existing strict Program validator/encoder once. A failed
expansion or final encoding returns `ProgramError` and exposes no partial Program.

### 5.2 State execution remains explicit and typed

The generic scope parameters are proof of the Operation boundary, not persisted types or an
alternate graph. They prevent an Operation with unrelated root contracts from receiving another
Operation's expansion scope. The DSL does not dynamically inspect an erased State to discover an
execution class. Common lexical methods remain explicit:

```rust
impl<I, O, F> OperationExpansion<I, O, F>
where
    I: MfmValue,
    O: MfmValue,
    F: MfmValue,
{
    pub fn pure<S>(&mut self) -> Result<()>
    where
        S: PureState;

    pub fn read<S, C>(
        &mut self,
        setup: &<C as CapabilityInjection<S>>::Setup,
    ) -> Result<()>
    where
        S: ReadState<C>,
        C: ReadCapabilityContract + CapabilityInjection<S>;

    pub fn operation<Op: Operation>(&mut self, child: &Op) -> Result<()>;
}
```

`pure::<S>` proves Pure execution through the trait bound. `read::<S, C>` proves the exact State,
Read capability, Intent/Evidence ABI, and injection policy pairing through its bounds. A future
Effect RFC may add a separately reviewed typed Effect method; this RFC adds no placeholder.

Rust call order is the sequence DSL. A successful lexical item feeds the next item. A lexical
State or child failure resolves to the nearest active exact handler, the scope's exact Failure
contract, or no edge for `Never`. Any other failure rejects. Every predecessor output must equal
the next item's input. These nominal checks occur during expansion; callers never pass raw
contract refs.

Pure States receive no hidden occurrence configuration because Program v2 has nowhere to persist
it. Runtime-relevant parameters must be in C0, a complete State context, or an immutable binding.
Read setup may vary per occurrence and must lower deterministically into the original binding and
injected topology.

### 5.3 Reusable configured Operations and repetition

An Operation value is the checked configuration for one authoring occurrence. It may own or borrow
bounded, secret-free domain values through ordinary private fields and a fallible domain
constructor. There is no configuration registry, erased config map, or persisted Operation object.

Repetition is ordinary Rust:

```rust
for step in &self.checked_steps {
    expansion.operation(step)?;
}
```

All configured child values are constructed and validated before `expand_program`; their domain
constructor errors remain domain/planner errors. `Operation::expand` never invokes a fallible
domain constructor and does not add a conversion from a domain error into `ProgramError`.

The same Operation type or value may be expanded more than once. Each call opens a private child
scratch scope, invokes `child.expand` once, validates its exact associated contracts and all exits,
then atomically flattens it into the parent's single draft. Operation boundaries do not consume a
declaration and are not persisted.

When child expansion returns, an open lexical tail becomes that child scope's success exit only if
its complete current contract equals `Op::Output`. Every propagated inhabited failure must equal
`Op::Failure`; `Never` contributes no exit. `operation` connects those exits to the parent's
lexical continuation and nearest exact active failure handler or scope failure. No caller receives
or rewrites the child's internal routes.

The same State type may likewise occur repeatedly. A different Read setup may produce a different
binding or injection topology. A different Pure runtime behavior parameter must instead be carried
by the typed input context; it cannot be captured invisibly in authoring code.

`Operation` implementations are trusted deterministic authoring code, like State implementations.
They must compose children only through `OperationExpansion::operation`. The open trait method
cannot prevent a malicious implementation from directly calling another
`Operation::expand` method or recursing in ordinary Rust. Supported-current production code must
contain no such direct call outside `mfm-program`; the absence scanner enforces that boundary.

The private `MAX_AUTHORING_CALLBACK_DEPTH` is exactly 64 simultaneously active kernel-entered
authoring callbacks. The root/child `Operation::expand`, Match definition/arm, protected/handler,
and capability `original_binding_ref`/before/after callbacks each count while active. Before
invoking the 65th callback, expansion returns `ProgramError::Capacity`. Direct or mutual recursion
entered through compiler methods is therefore deterministic; direct trusted-code method calls
remain the explicit bypass in Material uncertainty 4. An empty root Operation remains legal only
under the existing zero-State root identity rules. Empty nested Operations reject in this cut so
they do not require zero-length child-scope semantics.

The current Portfolio root uses `operation` inside an exact failure-handler scope, so the method has
a production caller. The configured EVM child may repeat with distinct target/setup values without
parent knowledge of its declaration count.

### 5.4 Structured Match joins and failure handlers

Program v2 contains two intentional non-linear shapes:

- a typed value selects one of several State/Operation sequences that rejoin at one contract; and
- an exact failure from a State, child Operation, or protected region enters a State-first handler
  that either recovers to one join contract or terminates the Operation.

The DSL exposes those shapes directly and no lower-level route mechanism:

```rust
impl<I, O, F> OperationExpansion<I, O, F>
where
    I: MfmValue,
    O: MfmValue,
    F: MfmValue,
{
    pub fn match_join<T, J>(
        &mut self,
        define: impl FnOnce(&mut MatchJoin<I, O, F>) -> Result<()>,
    ) -> Result<()>
    where
        T: MfmValue,
        J: MfmValue;

    pub fn with_failure_handler<E, J>(
        &mut self,
        protected: impl FnOnce(&mut Self) -> Result<()>,
        handler: impl FnOnce(&mut Self) -> Result<()>,
    ) -> Result<()>
    where
        E: MfmValue,
        J: MfmValue;
}

impl<I, O, F> MatchJoin<I, O, F>
where
    I: MfmValue,
    O: MfmValue,
    F: MfmValue,
{
    pub fn arm<P>(
        &mut self,
        tag: StableId,
        branch: impl FnOnce(&mut OperationExpansion<I, O, F>) -> Result<()>,
    ) -> Result<()>
    where
        P: MfmValue;
}
```

Callbacks are synchronous, never stored or boxed, and never retried. A successful enclosing
construct invokes each supplied callback exactly once. On failure, an entered callback runs at
most once and later branch/handler callbacks need not run. Their `OperationExpansion` values are
private scratch scopes initialized at the contracts described below. The public `I`, `O`, and `F`
parameters retain the enclosing Operation boundary; the scratch cursor and join contracts are
checked by exact nominal refs during expansion.

#### Match join

`match_join::<T, J>` consumes an open frontier whose exact current contract is `T`, emits one
Program v2 Match, and returns the union of its nonterminal arm-success frontiers as exact `J`.

For each `arm::<P>(tag, branch)`:

- `tag` must name exactly one variant in `T`'s retained closed descriptor;
- the variant's embedded nominal payload contract must equal `P`;
- only the external/adjacent single-payload shapes already supported by Runtime are accepted;
- the branch scratch starts at exact `P`;
- the flattened branch must be nonempty and State-first; an injected-before State counts, while an
  empty branch or Match-first child does not;
- a branch ending at exact `J` remains open for the common join; and
- a branch ending at a distinct exact enclosing Operation `O` terminates at `ScopeSuccess`.

Every selector variant must appear exactly once. Unknown, duplicate, missing, unit, unsupported
shape/tagging, wrong-payload, and wrong-join arms reject. `MAX_MATCH_ARMS` and the existing Program
limits apply. Match variant records are sorted/deduplicated by raw tag bytes exactly as today, but
physical branch declaration blocks remain in `arm` call order and are never sorted with metadata.

If at least one branch remains open, the enclosing frontier becomes exact `J`. If every branch
ends at a distinct `O`, the enclosing path is closed. When `J == O`, every such result is a join;
this cut deliberately exposes no direct terminal choice for an open equal-contract frontier. An
already-closed `ScopeSuccess` returned by a nested all-terminal construct remains closed.

This construct owns both ordinary and error-classified branching. State-local Rust matching remains
appropriate when it changes only data. `match_join` is required when a value changes future
topology, capability use, or durable observation boundaries. There are no conditional/no-op Reads:
both branch declaration blocks and injection suffixes are authored; at Runtime the unselected
branch executes no State and causes no adapter, provider, or Journal activity.

Runtime retains its independent hostile-wire Match association proof. Source authoring validates
the typed descriptor so a wrong `P` fails before Program construction; Runtime still rejects any
forged retained Program with unsupported tagging/shape, incomplete tags, or payload/target/codec
mismatch before admission/provider/append IO.

#### Scoped failure handling

`with_failure_handler::<E, J>` creates three physical regions in exact order:

```text
protected declarations
handler declarations
outer continuation
```

The protected callback may contain one State/child occurrence or a larger bounded sequence of
States and Operations. The same method is therefore the per-occurrence handler and the default
handler for an Operation region; there is no second handler policy or overload.

The handler routes only declared domain `State::Failure` edges in the authored Program. It does
not catch `ProgramError`, `RuntimeError`, adapter/Store errors, callback panics, cancellation, or
authoring callback failures; those retain their existing owners and abort without being converted
into a domain selector.

The protected callback starts at the caller's current contract. Inhabited failures resolve by
searching active handlers from innermost to outermost:

- exact `E` enters this handler;
- an exact enclosing-handler error enters that enclosing handler;
- exact Operation `F` becomes the scope failure when no handler intercepts it;
- `Never` has no edge; and
- any other failure contract rejects.

`E == F` is legal and intercepts the protected region's failures. A handler is active only while
expanding its protected callback, not while expanding its own handler callback, so it never catches
itself recursively. Nested protected scopes may intentionally use the same `E`: the innermost
active handler wins, while that inner handler's own `E`/`F` failure may enter an enclosing same-`E`
default handler that is still active before it exits the Operation.

The protected region must expose at least one reachable `E` edge and every open protected-success
path must end at exact `J`. The handler starts at exact `E`, must be nonempty and State-first, and
may:

- recover by leaving an open success frontier of exact `J`;
- terminate successfully by ending at a distinct exact `O`;
- propagate an exact `F` State failure through the ordinary nearest-enclosing-handler or scope
  failure rule; or
- install a nested handler for any exact failure contract.

Protected success and recovered handler success join at the next outer occurrence and skip the
physically adjacent handler. A Pure classifier followed by `match_join` may therefore send
recoverable variants to Operations that produce `J`, send fatal variants to an Operation that
produces a distinct `O`, and propagate classifier Failure as exact `F`. Program failure edges still
target the classifier's first State, never Match directly.

`E` and every recovery selector payload must carry the complete public context required by its
handler and recovery Operation. The expansion kernel and Runtime retain no ambient pre-failure
input to merge back into a partial error.

Conceptually:

```rust
body.with_failure_handler::<ReadFailure, Context>(
    |protected| protected.read::<Observe, ObservationRead>(setup),
    |handler| {
        handler.pure::<ClassifyReadFailure>()?;
        handler.match_join::<ReadFailureClass, Context>(|arms| {
            arms.arm::<RetryableFailure>(retryable_tag()?, |retryable| {
                retryable.operation(&recover_to_context)
            })?;

            arms.arm::<FatalFailure>(fatal_tag()?, |fatal| {
                fatal.operation(&map_to_output)
            })
        })
    },
)?;
```

Recovery is forward-only. The handler cannot retry the protected occurrence, jump backward,
implement `finally`, or infer compensation/rollback semantics.

#### Exit inference and atomicity

Branch and handler exit classification occurs only after the callback returns. An already-closed
`ScopeSuccess` remains terminal. Otherwise an open exact `J` remains open for the join, an open
distinct exact `O` becomes `ScopeSuccess`, and every other contract rejects. Reaching `O`
temporarily does not terminate a callback if later lexical States move the frontier. When
`J == O`, an open frontier is always the join. Nonempty/State-first rules prevent inference from
creating an empty terminal arm or handler.

Every State occurrence, child Operation, Match arm set, and handler region builds in isolated
scratch and commits once. Callback, descriptor, child, injection, depth, contract, or capacity
failure discards the entire construct and leaves the parent draft, current frontier, and active
handler stack unchanged. Side effects in captured callback code are outside rollback and forbidden
by the deterministic-authoring trust contract.

### 5.5 One private draft and one final validator

`OperationExpansion` owns exactly one private compiler representation conceptually containing:

```text
ExpansionDraft {
  declarations: Vec<DraftDeclaration>,
  entry: optional { DraftId, contract, declaration kind },
  open_success: optional Frontier { contract, edge slots },
  scoped exits and intercepted failure slots,
  counts and depth,
}

DraftDeclaration = State(DraftState) | Match(DraftMatch)
DraftRoute = Open | Target(DraftId) | ScopeSuccess | ScopeFailure | NoEdge
```

This is explanatory private structure, not a frozen public type layout. After a typed State or
Operation call returns, all heterogeneous Rust types have already been reduced to exact immutable
refs and private routes. Do not retain `Any`, boxed Operations, erased authoring callbacks, a nested
Operation tree, public label table, or a second finalized declaration vector. `DraftId` is a
private local ordinal; it is neither public nor persisted.

Private lowering must preserve these exact rules:

- Match is appended before every arm body;
- each Match variant targets that arm's first State;
- one arm's open `J` slots never bind to the next arm body;
- protected `E` failure slots target the handler's first State;
- protected-success and recovered-handler `J` slots skip the handler and remain open for the outer
  continuation;
- a child scope's success/failure exits remain symbolic until kernel-owned parent merge; and
- finalization converts private targets once to forward `u16` indices and calls `Program::new`
  once.

Every State occurrence, injected suffix, child Operation, Match, and failure-handler region builds
in scratch. Validate typed context/failure continuity, descriptor/entry constraints, callback
depth, counts, and capacity before one infallible merge into the parent draft. Errors leave the
parent semantically unchanged; there is no poisoned writer state.

A direct State occurrence needs only local fallible construction before append; do not introduce a
separate State-scratch type or nested draft abstraction for that one declaration. Owned child,
Match, handler, and injection scratch exists only where multiple declarations or exits must commit
atomically.

Contract, descriptor, scope, and deterministic expansion defects map to
`ProgramError::InvalidContract`; fixed depth/count/byte ceilings map to `ProgramError::Capacity`;
the existing final canonical encoding/decoding taxonomy remains unchanged. Do not add an Operation
error family or leak domain/debug detail through Program errors.

Finalization resolves each private route exactly once to the existing forward `u16` declaration
index, constructs the ordinary `Declaration::{State, Match}` vector, and delegates complete graph,
reachability, terminal, Never, canonical-byte, and Program-object capacity validation to the
existing Program owner. It never reorders sibling declarations or normalizes authored order.

### 5.6 Sole source-authoring authority

After every production author and test migrates, make these construction methods crate-private:

```text
Program::new
StateDeclaration::new
Execution::pure
Execution::read
MatchDeclaration::new
MatchVariant::new
```

The declaration/Program types and read-only accessors remain public for Runtime association and
inspection. `Program::decode_canonical` remains the sole retained-byte ingress and invokes no
Operation or injection hook. Program-internal tests may use private helpers; cross-crate hostile
tests forge canonical bytes and enter only through `decode_canonical`.

Do not retain `ProgramAuthor`, a public raw constructor, or another builder as an escape hatch.

## 6. Capability-owned injection

### 6.1 Exact pairing trait

The policy belongs in `mfm-program`, beside typed source authoring, and is implemented by the
domain-owned capability type for one exact State pairing. It does not belong in
`mfm-capabilities`, whose responsibility remains only the Read Intent/Evidence ABI and binding
proof.

```rust
pub trait CapabilityInjection<S>
where
    S: State,
{
    /// Checked authoring input for this exact capability/State pairing.
    type Setup;

    /// Complete input contract exposed by the expanded occurrence.
    type ExpandedInput: MfmValue;

    /// Complete success contract exposed by the expanded occurrence.
    type ExpandedOutput: MfmValue;

    /// Derives the original occurrence's immutable persisted binding.
    fn original_binding_ref(setup: &Self::Setup) -> Result<ContentRef>;

    /// Writes zero or more ordinary States before the original State.
    fn write_before(
        _setup: &Self::Setup,
        _writer: &mut InjectionWriter,
    ) -> Result<()> {
        Ok(())
    }

    /// Writes zero or more ordinary States after original success.
    fn write_after(
        _setup: &Self::Setup,
        _writer: &mut InjectionWriter,
    ) -> Result<()> {
        Ok(())
    }
}
```

There is no blanket identity implementation. Every supported pairing implements the trait
explicitly, so unsupported pairings fail at compile time and a later pair-specific policy is not
blocked by Rust coherence.

`Setup` is a sized domain-owned checked, bounded, deterministic, secret-free product. It may contain
public content/binding references but never providers, clients, signers, private keys, credentials,
nonce-authority handles, Store connections, Runtime handles, or mutable ambient state.
`original_binding_ref` derives the binding from the same setup used by the hooks. It neither
registers nor selects a live adapter.

Setup/binding continuity applies only to the designated original occurrence; hooks inherit no
execution authority from it.

### 6.2 Restricted injection writer

```rust
pub struct InjectionWriter {
    // owned private scratch suffix plus current/required-failure refs
}

impl InjectionWriter {
    pub fn pure<S>(&mut self) -> Result<()>
    where
        S: PureState;
}
```

It owns the temporary suffix and is passed to each hook by one ordinary mutable borrow. Its
constructor and fields are private, and it implements neither `Default` nor `Clone`. It exposes no
Match, failure handler, child Operation, termination, raw declaration, original-State access,
Program finalization, Runtime registration, provider handle, runtime value inspection, retained
callback, or IO.

The initial cut permits only lexical Pure State emission through this writer. A synthetic nonempty
policy proves the before/original/after contract. Add Read, Effect, or child-Operation emission only
with a real production policy that requires it and preserves the same single
input/output/common-failure suffix contract with lower combined API/LOC.

Each injected Pure State failure must be exact `Never` or the designated original
`S::Failure` stored as the writer's private required-failure ref. `Never` receives no failure edge;
an inhabited exact match uses the designated occurrence's active failure route. Any other failure
contract returns `ProgramError::InvalidContract` before the parent draft changes.

### 6.3 Expansion algorithm

`OperationExpansion::read` owns one private `expand_read_occurrence::<S, C>(setup)` implementation.

For one designated `read::<S, C>(setup)` occurrence:

1. Derive `<C as CapabilityInjection<S>>::ExpandedInput`, `ExpandedOutput`, and `S::Failure` refs.
2. Open one empty scratch suffix at the expanded input contract.
3. Call `<C as CapabilityInjection<S>>::write_before(setup, writer)` exactly once.
4. Require the writer's current success contract to equal `S::Input`.
5. Derive S's exact implementation ref, C's exact Read ABI, and
   `<C as CapabilityInjection<S>>::original_binding_ref(setup)`.
6. Append the designated original S occurrence exactly once in kernel-owned scratch code. Hooks
   never receive its seed or declaration and cannot emit a Read. A separate same-type occurrence
   must enter through ordinary `OperationExpansion::read` and independently apply its policy.
7. Continue the scratch writer at `S::Output` and call
   `<C as CapabilityInjection<S>>::write_after(setup, writer)` exactly once.
8. Require the final success contract to equal
   `<C as CapabilityInjection<S>>::ExpandedOutput`.
9. Incrementally enforce scratch bounds, then checked-add the complete suffix to the parent
   declaration/u16 limits.
10. Resolve every inhabited suffix failure through the active exact handler stack or enclosing
    scope Failure without exposing that routing to the hook.
11. Only after all fallible checks succeed, merge the suffix into the parent draft in one
    infallible commit.

Success order is exact:

```text
ExpandedInput
  -> before State 1 -> ... -> before State N
  -> one designated original S declaration
  -> after State 1 -> ... -> after State M
  -> lexical continuation or scope success exit
```

Any inhabited before/original/after failure skips the unfinished suffix and uses the occurrence's
one active structured handler or scope-failure route. There is no `finally`, rollback,
compensation, retry, recovery, or variant failure policy inside injection. An explicit handler
State or child Operation belongs to the enclosing Operation graph.

### 6.4 Context continuity and limits

The writer carries one complete current success contract; it never merges patches or keeps an
ambient context map.

- `write_before` begins at `ExpandedInput` and must end at `S::Input`.
- The original consumes `S::Input` and produces `S::Output`.
- `write_after` begins at `S::Output` and must end at `ExpandedOutput`.
- Each predecessor output must equal its successor input.
- Every State output is the complete next context.

Scratch growth checks declaration capacity incrementally. Exact whole-Program canonical-byte
capacity is checked only after private target resolution by Program's existing encoder/validator.

Hook error, binding failure, context/failure mismatch, or capacity failure drops the suffix and
leaves the enclosing Operation expansion unchanged.

### 6.5 Current Read identity policies

Implement explicit empty before/after policies for the six surviving EVM pairings:

```text
CheckChainIdentity<K>       / EvmChainIdentityRead
ReadInitialAnchor<K>        / EvmAnchorRead
ReadNativeBalance<K>        / EvmBalanceRead
ReadTokenDecimals<K>        / EvmBalanceRead
ReadTokenBalance<K>         / EvmBalanceRead
ConfirmBalanceAnchor<K>     / EvmAnchorRead
```

For each pairing:

```text
Setup = EvmPhysicalTarget
ExpandedInput = S::Input
ExpandedOutput = S::Output
original_binding_ref(setup) = setup.binding_ref()
write_before = empty
write_after = empty
```

The impls may be generated by the existing semantic Read macro if generated diagnostics remain
clear. Empty identity injection produces exactly one unchanged Read declaration and the same
Program bytes as today.

### 6.6 Trust boundary

Program decode, Runtime association, start, hot advancement, cold resume, read, Journal
qualification, Store load/append, and provider ingress invoke Operation/injection code zero times.
They use only the retained immutable Program.

Supported production hooks emit support States only through `InjectionWriter::pure`; direct
hook-to-hook method calls are forbidden trusted-code bypasses, just as direct child
`Operation::expand` composition is forbidden. No runtime claim depends on arbitrary downstream
authoring code obeying more than the final Program validator enforces.

Because strict canonical Program decode remains public, injection is not an authorization boundary.
A hostile but structurally valid Program can omit an expected injected State. The future
transaction-authority RFC must define authenticated durable authorization, concurrency/fencing,
replay resistance, idempotency, and orphan/reconciliation behavior. Its mutation boundary must
reject before provider entry whenever that authorization is invalid.

This RFC does not freeze bearer versus non-bearer representation, proof fields, lease/expiry
semantics, fencing mechanism, or an impossible atomic transaction spanning local authority and an
external provider. A future reservation completed before later failure or cancellation may become
orphaned because this authoring model intentionally has no `finally`; the future authority owner
must define its release or reconciliation behavior.

## 7. EVM and Portfolio Operation migration

### 7.1 `CollectEvmBalances<K>`

Replace public `append_balance_fragment` with one semantic configured child Operation, named
exactly `CollectEvmBalances<K>`. It owns private invariant-bearing fields and has a checked
constructor for:

- the `EvmPhysicalTarget`; and
- source count in `1..=EVM_BALANCE_SOURCE_LIMIT`.

The owned target keeps the public Operation type lifetime-free. A private non-owning marker
represents `K` when no other field does.

Its exact contracts are:

```text
Input   = EvmBalanceContext<K>
Output  = EvmBalanceCollectionCompletion<K>
Failure = EvmBalanceFailure
```

Expansion uses ordinary Rust repetition over the checked source count. Each source emits:

```text
CheckChainIdentity
-> ReadInitialAnchor
-> SelectBalanceAsset
-> Match(native | token)
   native -> ReadNativeBalance
   token  -> ReadTokenDecimals -> ReadTokenBalance
-> ConfirmBalanceAnchor
```

After all sources, emit `ConsolidateBalanceCollection`. Use lexical `pure`/`read` calls for ordinary
success/failure flow and `match_join` for the retained branch and shared Confirm rejoin. All six
Reads go through capability injection. Both native and token branch Reads are authored. Native
execution performs no token Read/provider call; token execution performs no native Read/provider
call.

The configured Operation is conceptually:

```rust
for _ in 0..self.source_count {
    body.read::<CheckChainIdentity<K>, EvmChainIdentityRead>(&self.target)?;
    body.read::<ReadInitialAnchor<K>, EvmAnchorRead>(&self.target)?;
    body.pure::<SelectBalanceAsset<K>>()?;

    body.match_join::<EvmBalanceAsset<K>, EvmBalanceContext<K>>(|arms| {
        arms.arm::<EvmBalanceContext<K>>(native_tag()?, |native| {
            native.read::<ReadNativeBalance<K>, EvmBalanceRead>(&self.target)
        })?;

        arms.arm::<EvmBalanceContext<K>>(token_tag()?, |token| {
            token.read::<ReadTokenDecimals<K>, EvmBalanceRead>(&self.target)?;
            token.read::<ReadTokenBalance<K>, EvmBalanceRead>(&self.target)
        })
    })?;

    body.read::<ConfirmBalanceAnchor<K>, EvmAnchorRead>(&self.target)?;
}

body.pure::<ConsolidateBalanceCollection<K>>()
```

This is explanatory source shape, not a new `native_tag`/`token_tag` public helper requirement.
Production uses the existing checked StableId construction and exact tags. Match is necessary
because the selected value changes future Read topology: neither branch executes, calls, or
journals the other branch's capability path at Runtime.

Delete fragment entry/count parameters, caller-provided completion/failure indices, `start_index`,
`base`, offset closures, per-source multiplication, consolidate-index arithmetic, raw declaration
mutation, and EVM-local `pure_state`/`read_state` helpers.

### 7.2 Portfolio root Operation

`plan_snapshot` retains checked selector/config/target selection and returns the exact `(Program,
C0)` pair. Its authoring implementation constructs a private configured Portfolio root Operation
and all checked `CollectEvmBalances` child values before calling `expand_program` once. A child
constructor defect maps to `PortfolioError::Program` before Operation expansion; it is not
converted into `ProgramError` inside `Operation::expand`.

Conceptually:

```rust
impl Operation for PortfolioSnapshotOperation<'_> {
    type Input = PortfolioSnapshotInput;
    type Output = PortfolioSnapshotOutput;
    type Failure = PortfolioSnapshotFailure;

    fn expand(
        &self,
        body: &mut OperationExpansion<
            Self::Input,
            Self::Output,
            Self::Failure,
        >,
    ) -> mfm_program::Result<()> {
        body.pure::<InitializePortfolio>()?;

        for child in &self.checked_collections {
            body.pure::<EnterPortfolioCollection>()?;

            body.with_failure_handler::<
                EvmBalanceFailure,
                PortfolioContinuation,
            >(
                |protected| {
                    protected.operation(child)?;
                    protected.pure::<ResumePortfolioCollection>()
                },
                |handler| {
                    handler.pure::<MapEvmBalanceFailure>()
                },
            )?;
        }

        body.pure::<ConsolidatePortfolio>()
    }
}
```

The exact physical declaration order remains:

```text
EnterPortfolioCollection
CollectEvmBalances child declarations
ResumePortfolioCollection
MapEvmBalanceFailure
next EnterPortfolioCollection | ConsolidatePortfolio
```

Child success targets Resume. Child failure targets the mapper and skips Resume. Resume success
targets the next collection or final consolidation. The mapper's declared success and failure both
exit through the exact root contracts; authoring does not assume its implementation always fails.

For each collection, protected success and handler recovery would rejoin as exact
`PortfolioContinuation` at the next `EnterPortfolioCollection` or final `ConsolidatePortfolio`.
This handler is instead terminal: `MapEvmBalanceFailure` reaches exact root output and
the distinct-`O` exit rule seals that path. Its declared `PortfolioSnapshotFailure` still exits
through the root failure contract. The DSL does not assume which variant the State implementation
returns.

Delete `CollectionLayout`, numeric cursor, fragment-length, terminal-index and capacity forecasts,
direct `Vec<Declaration>` mutation, and `portfolio_pure_state`. Portfolio knows the child
Operation's semantic contracts and handler boundary, never its declaration count.

### 7.3 Exact identity freeze

For every currently valid input, semantic renaming and Operation lowering produce bytes identical
to the pre-cutover Program, not merely an equivalent graph. Preserve the representative
two-source golden:

```text
canonical Program bytes: 36,560
Program content digest:
  content:sha256-v1:59c503d1d8faa1a7afc33d9bb0bcad42d053e7cecb8571023bbc0cef42ab4beb
```

Preserve the Program schema ID, declaration order, Match-arm ordering, all binding refs, every
State/capability ref, C0, public result fixture, and hot/cold behavior. The current declaration
formula remains `2 + 4*C + 8*S`; one collection with 64 sources remains 518 declarations and 65
sources rejects.

## 8. Future durable Effect extension

This RFC adds no Effect capability, Effect State, Program execution variant, Runtime driver,
Journal record, Store command, outbox, nonce type, signer integration, mutation API, migration, or
submission entry point.

The later transaction-authority RFC necessarily owns a deliberate Program execution-contract and
schema decision plus Runtime and Journal consequences. Store remains mechanical unless separately
changed. That RFC may add typed Effect occurrence emission to `OperationExpansion` and
`InjectionWriter`, may add Read emission to the writer only if its concrete topology needs it, and
may add the first non-empty production injection policy. Those are illustrative possibilities,
not reserved API spellings.

Conceptually, a future capability pairing may author:

```text
before:
  ReserveNonce
  -> DeriveSignedCandidate

one authored Broadcast declaration:
  Broadcast

after successful Broadcast conclusion:
  ObserveReceipt
  -> ObserveFinality
  -> ConfirmCanonicality
  -> ConsolidateSubmission
```

“One authored declaration” is not an external execution-cardinality guarantee. The future mutation
runner must prove durable authority and idempotence independently and reject before mutation when
the required authenticated proof is missing or stale.

Before approving that future cut, its architect must prove:

- one durable authority/idempotency owner;
- exact command identity and signed-byte custody;
- the concurrency/fencing model, if any, and orphan authorization/reconciliation behavior;
- definite versus ambiguous provider acknowledgement;
- crash/restart behavior at every durable boundary;
- retry/reconciliation rules that cannot duplicate mutation;
- replay resistance and mutation authorization, including whichever command/domain bindings that
  future contract selects;
- one compatible typed failure shape or a separately justified authoring extension; and
- exact Program/Runtime/Journal schema consequences under one clean-slate cutover.

Do not restore the retired generic Effect preparation/replacement protocol or nonce-only authority.

## 9. Explicitly deferred work

### 9.1 Additional DSL sugar

Do not add a macro language, stored Operation AST, HList/type-level sequence, public recipe/node
enum, dynamic Operation registry, `Box<dyn Operation>` collection, generic failure-policy object,
variant-failure router, public route/label token, raw successor method, separate branch compiler,
or second terminal/handler policy.

Common composition remains ordinary Rust call order and loops. `match_join` owns every current
value-selected topology and `with_failure_handler` owns exact scoped interception. Add label-free
lexical conveniences only after repeated production call sites prove they delete more concepts,
methods, and combined production LOC than they add. Any convenience must lower through the same
private draft and cannot become another authoring authority.

### 9.2 Generic Application entry points

Current Application remains the thin Portfolio facade over Runtime. Defer a private typed handler
registry until a real second production entry point or generic transport route exists.

A future handler may capture checked process-local authoring inputs, decode one bounded typed
request, call its root Operation to obtain `(Program, C0)`, forward the explicit caller RunId, and
return the existing `RunView`. It needs no configuration revision, source refs, output/failure
registry, status projection, lifecycle wrapper, or second Runtime error type.

### 9.3 CLI and HTTP

Keep current diagnostic-only binaries. Functional HTTP/CLI needs a separate product RFC with an
exact versioned request/response contract, bounds, authentication/exposure boundary, deployment
composition owner, error/status mapping, and server/client framework.

Transports remain thin: no Operation expansion, Runtime assembly, Store mutation, adapter
selection, credentials, or State-by-State progression. No scheduler, timer, queue, background
progression, automatic provider retry, or detached run worker is added.

## 10. Placement and deletion scope

### 10.1 `mfm-program`

Add `Operation`, `OperationExpansion`, `MatchJoin`, `expand_program`,
`CapabilityInjection`, and `InjectionWriter` in one private authoring module re-exported only at the
crate root. Reuse existing State/capability/value-ref helpers. Keep the draft, target fixups,
frontiers, handler stack, scopes, and depth/capacity machinery private.

Make the six raw constructors in Section 5.6 crate-private after all consumers migrate. Delete all
`ProgramAuthor` text/symbols if any prototype created them. Do not add a second public module path,
authoring error, public constants, or a second Program representation.

Constructor privacy includes the test cutover; do not add a `cfg(test)` public escape hatch:

- move raw graph-validator cases from `crates/kernel/program/tests/contracts.rs` into a private
  `#[cfg(test)]` module owned by `mfm-program`;
- keep external hostile-wire coverage by forging canonical bytes and entering only through
  `Program::decode_canonical`;
- migrate valid fixtures in `crates/kernel/runtime/src/assembly/tests.rs` and
  `crates/kernel/runtime/tests/runtime_contract.rs` to Operations and read-only declaration
  accessors; and
- express structurally valid hostile Runtime association cases through forged retained bytes, and
  move or delete cases that no admitted Program can expose.

### 10.2 `mfm-evm`

Add semantic capability/State types, `CollectEvmBalances<K>`, and exact-pair identity injection
impls. Delete numeric families, `append_balance_fragment`, raw declaration helpers, and index
arithmetic. Keep all domain values, State behavior, Match selector, target, stable IDs, and provider
contracts.

### 10.3 `mfm-portfolio`

Add semantic Portfolio State types and a private configured root Operation. Delete numeric State
markers, layout/cursor arithmetic, child-size prediction, direct declaration construction, and
duplicate contract derivation. Keep request/config/C0/output/failure semantics unchanged.

### 10.4 Live EVM and App tests

Change trusted registration and composition tests to semantic types. Live EVM retains only adapter
registration and no Portfolio dependency. Application production API/dependencies do not change.

### 10.5 Unchanged production boundaries

No production change belongs in:

- `mfm-runtime`;
- `mfm-journal`;
- `mfm-store`;
- `mfm-storage-postgres`;
- CLI or REST binaries; or
- signing/keystore.

Runtime tests may change only where public Program source construction moves to `expand_program`.
Execution behavior and proof assertions remain unchanged.

### 10.6 Documentation and absence enforcement

Update `docs/design.md`, `docs/architecture.md`, Portfolio/EVM/Program READMEs, and directly
affected examples. Document Operations as deterministic authoring-only composition, States as
execution leaves, and Program as the only persisted graph.

Extend the supported-current cutover manifest/scanner for the numeric families, `ProgramAuthor`,
`ForwardLabel`, `label`, `place`, `_to` authoring methods, raw `select`, raw source constructors,
direct declaration-vector authoring, direct `.expand(...)` or `Operation::expand(...)` composition
outside the `mfm-program` owner, direct hook-to-hook `original_binding_ref`/`write_before`/
`write_after` calls, and current docs teaching numeric indices or public routes.
Refresh its fingerprint/coverage under the existing scanner contract. Trait implementations and
narrow negative tests necessarily contain the `expand` spelling; the rule targets method-call
syntax in supported production roots, not definitions. Hostile fixtures may name rejected inputs
under narrow test scopes; production exports and current docs may not teach them.

Generic spellings such as `label`, `place`, `select`, and `_to` are forbidden only as the removed
`mfm-program` authoring exports/calls or exact retired owner-qualified symbols. The scanner must not
globally reject unrelated domain prose or ordinary collection/API methods with the same words.

## 11. Ordered implementation commits

### 11.1 Commit 1 — `name surviving evm and portfolio contracts`

Change the EVM/Portfolio domain types, live registration, App composition tests, focused tests,
directly affected rustdoc/READMEs, and cutover rules for numeric families. Delete every numeric
type/use/comment/alias in Section 4.

Preserve exact IDs, Program bytes, fixtures, provider behavior, and dependencies. This commit is
independently compile-green and introduces no Operation authoring API early.

Architect gate: a non-author domain-contract reviewer compares every old/new type association and
returns `APPROVE` or `BLOCK`, covering stable identity, public-surface tradeoff, change sites,
deletion completeness, and verification.

### 11.2 Commit 2 — `centralize operation expansion and capability injection`

Change `mfm-program`, its focused tests, EVM injection/child Operation authoring, Portfolio root
authoring, consumer tests, current architecture/design docs, crate READMEs, and cutover rules.

Add the five public concepts and root function, migrate every source author, make raw constructors
private, and delete every replaced helper/index forecast in the same commit. Do not stage aliases,
a parallel raw builder, `ProgramAuthor`, or a compatibility feature.

Architect gate: a non-author Program/capability reviewer traces:

- one configured Operation repeated with distinct configuration;
- one nested child success/failure scope;
- the native/token `match_join` and Confirm rejoin;
- one terminal Portfolio failure handler and one synthetic recovering classifier/Match handler;
- one synthetic non-empty injection from checked setup to final bytes;
- exact designated-original emission;
- one private draft and one Program finalization path;
- atomic failures and the exact 64-callback depth bound; and
- combined kernel/domain authoring LOC and public-method tradeoffs. Every structured expansion
  method names its current production caller. The recovering-handler proof and the first non-empty
  injection policy are explicitly identified as synthetic; `InjectionWriter::pure` is the only
  public method without a current production call site.

The reviewer `BLOCK`s any alternate Program representation, dynamic authoring registry, Runtime
hook, raw-index exposure, source-constructor bypass, placeholder Effect API, or unjustified
LOC/public-surface growth.

After both commits, a non-author cumulative reviewer checks exact Program-byte identity,
dependency direction, authoring dexterity, deletions, and the final complexity report.

Reviewer records name the commit/tree, reviewer, decision, correctness findings, dexterity
findings, deletion/LOC findings, verification inspected, and resolution of every prior `BLOCK`.

## 12. Verification plan

### 12.1 Semantic-name tests

Prove:

- exact stable IDs for all three capabilities and thirteen States;
- exact Input/Output/Failure and Read pairings;
- every named type is usable at trusted registration sites;
- all numeric families and aliases are absent;
- representative Program canonical bytes/content ref are unchanged;
- the 64-source capacity Program remains 518 declarations and 65 sources rejects;
- public EVM/Portfolio JSON fixtures are byte-identical; and
- live registration plus App hot/cold behavior remain unchanged.

### 12.2 Operation and lowering tests

Prove:

- `expand_program` invokes the configured root once and is the only trusted source ingress;
- zero-State root identity and one-State root behavior remain exact;
- lexical Pure, Read, and child Operation success/failure continuity;
- the same State and Operation may repeat with distinct checked setup/configuration;
- each `operation` call invokes its configured child exactly once;
- configured child domain values are validated before `expand_program`, and expansion introduces
  no domain-error-to-`ProgramError` conversion;
- nested Operations flatten without persisted boundary declarations, and root/child exact success
  and failure scope exits are each rewritten to their intended parent or terminal destinations;
- child error/capacity/contract failure leaves the parent draft unchanged;
- direct and mutual Operation recursion entered through `operation`, plus mixed nested Match,
  handler, and hook callbacks, accept depth 64 and reject the 65th callback before invocation;
- production code contains no direct `.expand(...)` or hook-to-hook composition call that bypasses
  this depth accounting;
- empty nested Operations reject while root zero-State remains valid;
- source order is identity-bearing and never normalized;
- typed external and adjacent Match descriptors with heterogeneous payloads, State arms, child
  Operation arms, and exact common joins;
- unknown, duplicate, missing, unit, unsupported-shape/tagging, wrong-`P`, wrong-`J`, empty, and
  Match-first arms reject atomically;
- Match metadata sorts/deduplicates by raw tag bytes while physical arm declaration blocks retain
  callback order;
- an injected-before State is the arm target, and nonempty injection does not change semantic join
  destinations;
- mixed join/terminal arms, all-terminal closure, distinct-`O` terminal inference, `J == O`
  join-only inference for open frontiers, and nested already-closed terminal composition;
- protected success skips the physically adjacent handler and recovered handler success rejoins
  the same exact `J` continuation;
- exact handled `E`, `Never`, outer `F` bypass, `E == F`, foreign failures, nearest nested-handler
  behavior, and same-`E` inner override/outer fallback;
- unreachable, empty, Match-first, wrong-input, wrong-join, and wrong-handler failure contracts
  reject atomically;
- terminal-to-`O`, recovery-to-`J`, whole-region and one-occurrence handlers, and a Pure classifier
  followed by mixed recovering/terminal Match arms;
- the synthetic classified recovery produces identical hot and cold branch, join, and terminal
  views;
- the handler cannot intercept its own failure recursively, retry protected work, or jump backward;
- native/token Match branches of unequal length rejoin the exact Confirm State;
- EVM child success enters Resume while failure skips Resume and enters the State-first mapper;
- Resume precedes Mapper physically, Resume success skips Mapper, and both mapper exits obey exact
  root contracts;
- in a two-collection graph, the first Resume targets the second Enter, each child failure targets
  only its own Mapper, both Mappers terminate, and the final Resume targets Consolidate;
- successful constructs invoke every supplied callback once; failures never retry an entered
  callback, may short-circuit later callbacks, and preserve the parent frontier, handler stack,
  counts, and draft;
- Runtime hostile association still rejects unsupported Match tagging/shape, missing/unknown tags,
  payload-target mismatch, and missing/mismatched codecs after raw constructors become private;
- declaration/u16/Program-byte limits retain existing error classifications; and
- raw Program/State/Execution/Match constructors, `ProgramAuthor`, `ForwardLabel`, public labels,
  raw routes, and raw `select` are inaccessible outside `mfm-program`; compile-fail tests also prove
  callers cannot construct or retain `MatchJoin`/`InjectionWriter` beyond their callback scopes.

### 12.3 Injection tests

Prove:

- every current identity policy emits exactly one unchanged original Read declaration;
- a synthetic non-empty policy emits exact before/original/after order;
- the designated kernel-owned occurrence is inserted once and is not directly exposed or writable
  by a hook; two ordinary same-type Read occurrences each apply their policy and emit their own
  designated occurrence;
- successful source authoring runs each applicable hook exactly once and zero times afterward;
- `ExpandedInput -> before -> S::Input` and `S::Output -> after -> ExpandedOutput` continuity;
- `Never` and exact original failure are accepted while foreign failure rejects atomically;
- a before/original/after failure skips the unfinished suffix and takes one exact failure route;
- binding ref derives from the same setup used by the designated occurrence's hooks;
- multiple injected Pure States preserve exact lexical order and use no hidden setup/binding;
- non-empty injection shifts counts while Match-arm entry, join, completion, and active-handler
  targets still resolve semantically;
- hook error, binding error, contract mismatch, and capacity leave the parent unchanged;
- an entered failing hook is not retried, and later hooks short-circuit when an earlier stage fails;
- identical checked setup produces identical Program bytes;
- `InjectionWriter` cannot construct itself, Match, install a failure handler, compose a child
  Operation, terminate a scope, choose successors, finish, emit raw declarations, access the
  designated original, or obtain any IO authority/handle through this API; hooks receive no such
  authority through the expansion contract; and
- hook counters remain unchanged during decode, Runtime association/start/resume/read, Journal,
  Store, and provider entry.

### 12.4 Domain equivalence and dexterity

Prove exact native, token, mixed-source, multi-source, failure-mapper, and maximum-capacity graphs.
Confirm native execution skips token Reads, token execution skips native Reads, token decimals
remain separately durable, wrong route/chain remains pre-provider `Internal`, and hot/cold views
remain equal.

Record the complete files/registrations changed to add:

- one new Pure State occurrence;
- one new Read State/capability pair;
- one reusable configured child Operation;
- one repeated occurrence with distinct setup; and
- one non-empty injection policy.

The reviewer must confirm those changes remain owner-local and require no Runtime fold, Journal,
Store, schema, or App production edit.

### 12.5 Complexity evidence

Using identical reviewed roots/commands before and after, report:

- tracked source-tree LOC and external-test LOC;
- all Commit-2 production Rust additions/deletions, so code movement cannot escape the measure;
- production additions/deletions for the `mfm-program` authoring module plus affected EVM/Portfolio
  authoring code;
- public logical items/methods added versus made private/deleted;
- the current production call site for every public authoring method, with the restricted writer
  methods and synthetic recovering-handler fixture recorded honestly;
- normal internal dependency edges;
- numeric/index-authoring call sites; and
- State/Operation/injection change sites from Section 12.4.

Target a non-positive combined production-authoring LOC delta. Moving arithmetic from a domain into
the kernel is not deletion. A positive delta is `BLOCK` until the commit architect enumerates every
irreducible invariant-bearing addition, confirms no public concept/method/private helper or
duplicated check can be removed, and explains why the smallest coherent implementation remains
positive. Hypothetical future Effect reuse alone is not justification.

### 12.6 Focused commands

Commit 1:

```text
nix develop -c cargo test \
  -p mfm-evm -p mfm-portfolio -p mfm-evm-live -p mfm-app --all-targets
nix run .#run -- --task negative-scan
```

Commit 2 while iterating:

```text
nix develop -c cargo test -p mfm-program --all-targets
nix develop -c cargo test \
  -p mfm-runtime -p mfm-evm -p mfm-portfolio -p mfm-evm-live -p mfm-app --all-targets
nix run .#run -- --task capacity-app
nix run .#run -- --task negative-scan
```

Run `nix run .#model-check` if task or manifest integration changes. After focused checks are green,
run `git diff --check` and exactly one final `nix run .#ci`; do not immediately precede CI with
redundant broad component gates.

## 13. Rejected alternatives and non-goals

This RFC explicitly rejects:

- `ProgramAuthor` or a separate Program builder beneath/after `OperationExpansion`;
- State-only Program or removal of Match;
- a new Program version/schema/address wire, Journal record, Store table, or migration;
- public raw Program/State/Execution/Match constructors beside `expand_program`;
- public numeric indices, declaration reservations, or parent child-size forecasts;
- public labels, raw routes/successors, `_to` emitters, or raw `select` beside the structured DSL;
- a persisted/public Operation AST, node/recipe enum, HList, dynamic registry, or boxed Operation
  collection;
- a second expansion/lowering representation or finalized declaration form;
- dynamic execution-mode discovery from erased States;
- a broad failure-policy hierarchy, variant-failure Runtime reducer, `FailureNext::ByVariant`,
  backward retry/recovery routes, `finally`, rollback, compensation, or automatic retry;
- separate ordinary-Match and failure-classifier branch APIs instead of the same `match_join`;
- conditional/no-op Reads, `NotApplicable` evidence, a Pure/Read hybrid execution mode, or a linear
  native-plus-token superset;
- a macro language before ordinary Rust DSL repetition proves insufficient;
- a blanket empty injection impl or fake identity capability;
- raw routes, Match, Program finalization, Runtime handles, or IO through `InjectionWriter`;
- authoring hooks during decode, Runtime, Journal, Store, or cold recovery;
- treating injection as proof that a nonce was reserved or mutation authorized;
- placeholder Effect/nonce/submission/outbox APIs before the durable authority RFC;
- revival of facts, configuration publication, replay, audit/export, lifecycle, or compatibility
  surfaces;
- numeric compatibility aliases;
- generic Application dispatch before a second entry point;
- functional transports before a deployment/product contract; and
- schedulers, queues, timers, detached progression, or transport-owned Runtime loops.

## 14. Acceptance criteria

The RFC is implemented only when:

1. all surviving capabilities and States have the semantic names in Section 4;
2. every old stable capability/State ID and associated contract remains exact;
3. numeric families, aliases, ordinal comments, and numeric call sites are absent;
4. `Operation` is the reusable typed authoring boundary and `OperationExpansion` the sole compiler
   context;
5. `ProgramAuthor`, parallel builders, stored Operation representations, and dynamic authoring
   registries are absent;
6. `expand_program` is the sole source-authoring ingress and `Program::decode_canonical` the sole
   retained-byte ingress;
7. all raw source constructors in Section 5.6 are non-public;
8. repeated Operations/States with distinct checked configuration expand deterministically;
9. child Operation boundaries flatten atomically and persist no metadata;
10. lexical State/Operation continuity and exact scope success/failure contracts are enforced;
11. `match_join` is the sole value-selected topology API, proves exact typed arms/common joins,
    preserves physical branch order, and has no public label/raw-route escape hatch;
12. `with_failure_handler` handles one exact failure contract from a State, child, or region and
    supports State-first terminal or forward-recovery topology through the same `match_join`;
13. arm/handler exit inference maps open `J` to a join and distinct open `O` to terminal success,
    preserves nested closed success, and cannot create an empty arm or handler;
14. Program v2 State/Match wire, schema, declaration order, root rules, and limits remain exact;
15. `CapabilityInjection` is exact-pair, setup-bound, deterministic, and authoring-only;
16. `InjectionWriter` exposes only restricted lexical Pure State emission;
17. before/original/after context and common-failure continuity are exact;
18. the designated wrapped Read occurrence is emitted once by kernel-owned code and never exposed
    or writable through a hook, while each ordinary repeated same-type Read independently applies
    its policy;
19. expansion/injection failure is atomic, kernel-entered authoring callback depth accepts 64 and
    rejects the 65th callback before invocation, and supported production code never composes
    through direct `.expand(...)` or hook-to-hook calls;
20. all six current EVM Read pairings use explicit identity injection;
21. EVM/Portfolio authoring contains no declaration-index/count coordination;
22. native/token topology authors both paths, while Runtime executes only the selected path;
23. representative and capacity Program bytes/content refs remain exact;
24. Runtime, Journal, Store, PostgreSQL, App production API, CLI, and REST behavior are unchanged;
25. no Effect, nonce, submission, outbox, config/fact/replay/lifecycle/compatibility surface is
    reintroduced;
26. current docs and the negative scanner teach only this authoring model;
27. the combined complexity report achieves a non-positive production-authoring LOC delta or
    records the mandatory architect justification for every irreducible positive line; and
28. both commit-level reviews, cumulative review, focused verification, and final CI are recorded
    and green.

## 15. Cutover and rollback

Both commits are source/API cutovers with intentionally identical persisted Program and run bytes.
They require no database migration, history conversion, dual decoder, feature flag, compatibility
alias, or mixed authoring path.

Rollback is whole-commit only. The commits are ordered and independently coherent; do not
cherry-pick part of the Operation-authoring cutover or retain raw constructors beside
`expand_program`.

A future Effect/transaction-authority release is a separate persisted/security cutover with its own
rollback and database rules. This RFC neither authorizes nor anticipates mixed old/new Effect
execution.
