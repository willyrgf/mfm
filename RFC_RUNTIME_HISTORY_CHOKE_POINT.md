# RFC: Runtime History Choke Point

Status: proposed target architecture; implementation has not started

Scope: structured operation authoring and expansion, state execution kinds, typed failure
handling, run-history ownership, deterministic scheduling, external-access recording, effect
ambiguity, cross-run resource authority, EVM nonce ownership, adapters, transports, stores,
replay, telemetry boundaries, and deletion of the arbitrary state graph and generic executor

Compatibility: this is a breaking target design. The current code, `docs/design.md`, and
`docs/architecture.md` continue to describe the implemented graph/executor design until one
complete cutover deliberately replaces that contract. This RFC defines no compatibility reader,
dual writer, graph-to-sequence adapter, fallback executor, or parallel execution authority.

## Executive Decision

An MFM operation is a declaration-ordered, typed, structured program.

The operation itself is the sequence. The only explicit control constructs are:

```text
State
Match
FanOut
Return
Fail
```

`Match` performs exhaustive conditional control over an ordinary history-bound value encoded with
the kernel's closed tagged-sum contract; a successful state output may supply that value. The
private `StateOutcome` discriminator itself is not a public `Match` input: the structured state
binding exposes success as its output and routes a typed failure into its one exact structural
failure continuation. That failure continuation's normally completing path reaches the designated
handler. `FanOut` is the only place where more than one state may be eligible at once, and its
lanes are initially restricted to bounded `Pure` and `Read` blocks with collect-all semantics.

An arbitrary DAG is not an execution contract. A graph may be derived for diagnostics or
visualization, but it is never admitted as scheduling authority.

Operations continue to expand purely:

```text
AuthoredProgram
    -> deterministic pure expansion
ExpandedProgram
    -> certification
CertifiedProgram
    -> admission and execution
```

Expansion may inject typed states and structured control for domain requirements, security,
provenance, failure routing, resource acquisition, and durable audit facts. Expansion is
call-site-local, finite, deterministic, bounded, and visible in the admitted program. It is not an
ambient runtime hook or an unrestricted AST-rewriting plugin system.

Only `CertifiedProgram` is execution authority. Runtime does not know whether a state was authored
directly, inlined from a child operation, injected by an EVM capability expansion, or wrapped by a
framework policy.

Runtime is the sole active interpreter and run-history mutation owner. This is not a deployment
singleton. Multiple processes may assemble workers against the same qualified authoritative
lineage, but exact-head compare-and-append and store fencing serialize their durable actions.

Outside `FanOut`, a run has exactly one current state occurrence. Inside `FanOut`, it has one
cursor per declared lane. Runtime executes only the state or lane named by the verified cursor; it
does not scan a global ready set, rank nodes, spread authorizations, or infer control flow from
value availability.

Certified states retain three semantic execution kinds:

```text
Pure
Read
Effect
```

`Effect` means that the state may mutate an external target or consume an exclusive external
capability. Effects are forbidden inside the initial `FanOut` contract.

Every fallible state occurrence and fallible fragment boundary has exactly one exhaustive failure
continuation. Its lexical scope supplies a default typed failure-handler state when the call site
does not override it. A wrapper may carry the protected failure only through its exact affine
fragment boundary to that one call-site handler; it cannot insert a second handler or bypass the
first. The default handler is `Pure`, infallible, consumes the exact producer-bound typed failure,
and maps it into that operation or fan-out lane's declared failure contract. Reads, effects,
retries, fallbacks, and compensations are explicit states in explicit branches; they are never
hidden inside the default handler.

`Read` and `Effect` use one private Runtime-owned access bracket:

```text
Prepared<K>
  -> ExternalAccessAuthorized committed
  -> Authorized<K>
  -> exactly one registered-invoker entry
  -> PendingObservation<K>
  -> ExternalAccessObserved committed
  -> CommittedObservation<K>
       | Returned | SafeFailure
       |   -> state settlement
       |       | StateOutcome
       |       |   -> StateTransitionCommitted
       |       | InvalidEvidence
       |           -> blocked; no semantic transition
       | SupersededBeforeEntry                    [Effect only]
       |   -> Refreshable next physical attempt
       | EntryUnknown                             [Effect only]
       |   -> parked; no semantic transition
       | IntegrityFault
           -> blocked; no semantic transition
```

where `K` is `Read` or `Effect`.

At most one unresolved authorization exists for one current occurrence. A second effect
authorization cannot be constructed while the first attempt may have entered. Process loss after
authorization therefore leaves that run parked at the current effect with no legal successor in
this RFC. A future same-occurrence reconciliation protocol would be required; declaration order
cannot prove whether an external target was entered.

Ordinary definite failures do not park runs. Every definite state-facing completion that the
product expects to handle is either a reviewed typed response or a reviewed redaction-safe
`SafeFailure`. The state deterministically interprets either form into success, typed failure, or
invalid evidence. A typed failure follows the operation's explicit or default failure path, which
may recover or close the current run as failed. A later invocation receives a different `run_id`
and is not blocked by the earlier run's history status. Independent cross-run resource invariants
still apply.

Integrity violations, unavailable run-history persistence, and possible-entry ambiguity are not
ordinary failures. They cannot be converted into state failure merely to obtain liveness.

The generic executor is removed. It must not remain as a second interpreter, a wallet driver, a
retry engine, or an adapter with a renamed lifecycle. Meaningful EVM progression is injected as an
explicit structured program of ordinary `Pure`, `Read`, and `Effect` states.

Cross-run uniqueness remains outside one run's Runtime. An EVM sender uses one narrow durable
wallet-nonce resource authority. It owns only the atomic nonce and writer-fencing invariants.
It does not fold run history, choose transaction steps, query an EVM provider, schedule retries,
or decide terminal run meaning.

The resulting shape is:

```text
operation DSL
    |
    v
pure expansion --> certification --> CertifiedProgram
                                      |
                                      v
                      Runtime -- sole writer --> RunHistory store
                         |
                         +-- authorized Read ----> registered invoker --> transport/provider/store scanner
                         |
                         +-- authorized Effect --> registered invoker --> transport/provider/signer
                         |
                         +-- authorized Effect --> WalletNonce authority
```

PostgreSQL's client and wire protocol are infrastructure transports. A schema-aware PostgreSQL
implementation that owns transactions, compare-and-append, fencing, exact idempotency, or nonce
uniqueness is an authority-bearing adapter/store. Calling PostgreSQL a transport must not hide
those responsibilities or expose a raw pool.

## Why The Existing Design Is Wrong

### Physical persistence was centralized, but the recording obligation was not

The store can be the only component that physically appends run records while a caller still owns
the obligation to remember the second half of an external-access protocol:

```text
caller
  -> append authorization
  -> invoke live boundary
  -> inspect or encode result
  -> append observation
```

An ordinary return between invocation and observation can silently violate the audit contract.
One private component must own authorization, affine invocation authority, totalization, pending
observation material, and observation commit as one no-normal-escape protocol. That owner is
Runtime.

### The executor became a second Runtime

The current generic executor owns or participates in another history, fold, current view,
resource allocation, target-attempt authorization, delivery planning, restart loops, retained
evidence, terminalization, and writer fencing.

Those are interpreter responsibilities. Nesting that lifecycle below Runtime duplicates the
failure protocol:

```text
Runtime authorization
  -> executor ensure
       -> executor authorization
       -> target call
       -> executor observation
       -> executor terminalization
  -> Runtime observation
  -> state settlement
```

Removing the executor does not remove the real work. It gives each responsibility one owner:

- operations and expanders own deterministic structured composition;
- states own reusable domain request and outcome semantics;
- Runtime interprets one certified cursor and orchestrates run-history mutation;
- the store owns legal append, exact-head CAS, and the callback-free run fold;
- adapters execute one already-selected operation;
- transports perform bounded protocol exchanges; and
- narrow resource authorities own cross-run invariants.

### The arbitrary DAG obscures the intended control model

The product's intended default is declaration order. Conditional control exists to branch on
typed state outcomes, and parallelism exists only when explicitly declared.

The current authored representation instead stores unordered node and binding bags, canonicalizes
nodes independently of declaration order, infers readiness from alternative producer groups, and
globally ranks eligible actions. That creates cycle, reachability, dependency-skip,
required-success, all-nodes-terminal, and fairness machinery that is not required by the product's
control model.

It also makes fan-out indirect. A collect-all operation should declare an ordered fan-out and
receive an ordered collection, rather than represent several results as alternative sources for
one destination.

The replacement is a structured program whose declaration order is semantic. A derived graph is
permitted only as a view.

### A state output cannot itself be a cross-run lock

An output such as `ReservedWalletNonce` is necessary but not sufficient. A run journal orders one
run; two runs can otherwise choose the same nonce concurrently.

The output is proof of a durable reservation, not the exclusion primitive. The wallet-nonce
authority's atomic transaction creates cross-run exclusion. The producer-bound typed value
provides within-run causality and provenance.

### Arbitrary wrappers would recreate hidden control

Pure expansion is valuable only while the expanded behavior is ordinary certified structure.
Allowing a wrapper to duplicate its protected effect, reorder siblings, reach across lexical
scopes, inspect live state, introduce an unbounded loop, or execute ambient logging would recreate
the graph/runtime indirection under another name.

Expansion is therefore a typed, finite substitution mechanism. Its power comes from composing
ordinary states and explicit control, not from bypassing them.

## Goals

- Make declaration order the default and authoritative operation execution order.
- Replace the arbitrary execution DAG with one structured `State | Match | FanOut | Return | Fail`
  program.
- Preserve pure deterministic operation and child-operation expansion.
- Allow typed pre-, success-post-, failure-post-, and domain-requirement state injection.
- Make expansion finite, bounded, content-addressed, reproducible, and visible at admission.
- Require one exhaustive typed failure continuation for every fallible state occurrence or
  fragment boundary.
- Provide lexical operation- and lane-scoped default `Pure + Infallible` failure-handler states.
- Keep ordinary definite failures recoverable or terminal rather than indefinitely parked.
- Restrict initial fan-out to bounded, declaration-ordered, collect-all `Pure` and `Read` lanes.
- Make Runtime a cursor interpreter rather than a global graph scheduler.
- Make every normal return after authorized access structurally pass through observation commit.
- Preserve `Pure | Read | Effect` as certified semantic state kinds.
- Keep effect-bearing states visible for audit, security analysis, and recovery.
- Make store validation the authoritative enforcement boundary for persisted history.
- Preserve five append-only run-history record families and exact-head atomic append.
- Keep runtime and store generic with respect to EVM, nonce, expansion origin, and handlers.
- Inject EVM nonce and transaction prerequisites through registered semantic expansion.
- Replace generic executor resource machinery with the smallest resource-specific authority.
- Make normal resource-fence rotation a proven-pre-entry physical continuation rather than a
  semantic failure or integrity poison.
- Keep state logic deterministic and free of ambient IO.
- Separate semantic audit facts from best-effort operational telemetry.
- Keep secrets and provider-controlled diagnostics out of persisted and public surfaces.
- Delete superseded graph and executor concepts rather than preserve compatibility paths.

## Non-Goals

- Supporting arbitrary DAGs, jumps, loops, overlapping joins, or first-available-source control.
- Making declaration order a mere scheduler tie-breaker while retaining graph readiness.
- Allowing effects inside the initial `FanOut` contract.
- Defining fail-fast cancellation, detached fan-out lanes, or completion-order result semantics.
- Adding open-ended retry, polling, backoff, failover, or circuit-breaker policy.
- Automatically re-entering an effect after possible external entry.
- Treating a process lease or Runtime cursor as a target-enforced resource fence.
- Making authorization, external IO, and observation one atomic database transaction.
- Guaranteeing an observation after process, machine, or storage loss.
- Preserving every returned read or effect through a universal outbox.
- Making Runtime a cross-run resource coordinator.
- Allowing wrappers, state callbacks, adapters, transports, or applications to inspect or append
  run history.
- Allowing expansion to inspect a live adapter or transport to choose topology.
- Making best-effort logging or telemetry availability part of run correctness.
- Exposing PostgreSQL pools or generic query authority to Runtime or live adapters.
- Persisting credentials, private keys, signatures, raw signed envelopes, provider bodies, URLs,
  or unreviewed error strings.
- Encoding authorization and observation as fake semantic transitions.
- Defining an implementation plan before the material uncertainties and validation gates in this
  RFC are resolved.

## Terminology

### Structured program

The declaration-ordered authored and expanded forms:

```text
AuthoredProgram<Output, Failure> =
    AuthoredOrderedBlock<
        Return(Output) | Fail(Failure),
        Return(Output) | Fail(Failure),
    >

AuthoredOrderedBlock<Exit, FailureExit> {
    lexical_default_handler: Declared(handler) | Inherited(scope_ref),
    declarations: [AuthoredBinding<FailureExit>],
    exit: Exit,
}

AuthoredFailureChoice<Failure, RecoveredOutput, FailureExit> =
    NoFailure                    when Failure == Never
  | UseLexicalDefault            when Failure != Never
  | ExplicitPureNeverHandler<HandlerRoute> {
        handler_call,
        continuation: AuthoredHandlerContinuationBlock<
            HandlerRoute,
            RecoveredOutput,
            FailureExit,
        >,
    }                            when Failure != Never

AuthoredHandlerContinuationBlock<HandlerRoute, RecoveredOutput, FailureExit> = {
    handler_output_local: TypedLocal<HandlerRoute>,
    body: AuthoredOrderedBlock<
        Recover(RecoveredOutput) | FailureExit,
        FailureExit,
    >,
}

AuthoredBinding<FailureExit> =
    StateCall {
        output_local,
        stable_label,
        call,
        failure: AuthoredFailureChoice<
            call.Failure,
            call.Output,
            FailureExit,
        >,
    }
  | OperationCall {
        output_local,
        stable_label,
        child_operation,
        typed_inputs,
        success_contract,
        failure_contract,
        failure: AuthoredFailureChoice<
            child_operation.Failure,
            child_operation.Output,
            FailureExit,
        >,
    }
  | Match {
        output_local,
        stable_label,
        selector: ClosedSumValue,
        exhaustive_arms: [
            AuthoredTaggedArm {
                canonical_tag,
                stable_arm_label,
                body: AuthoredOrderedBlock<
                    Yield(Value) | FailureExit,
                    FailureExit,
                >,
            },
        ],
    }
  | FanOut {
        output_local,
        stable_group_label,
        bound,
        ordered_lanes: [
            AuthoredLane {
                stable_lane_key,
                declaration_ordinal,
                body: AuthoredOrderedBlock<
                    Yield(StateOutcome<LaneOutput, LaneFailure>),
                    Yield(StateOutcome::Failure(LaneFailure)),
                >,
            },
        ],
    }

OperationProgram<Output, Failure> =
    OrderedBlock<
        SequentialPolicy,
        Return(Output) | Fail(Failure),
        Return(Output) | Fail(Failure),
    >

OrderedBlock<Policy, Exit, FailureExit> =
    ordered [StateBinding<Policy, FailureExit>
           | MatchBinding<Policy, Exit, FailureExit>
           | FanOutBinding
           | FragmentBinding<Policy, FailureExit>]
    followed by one lexical Exit
```

`OperationCall` exists only in `AuthoredProgram`. Expansion replaces it with one
`FragmentBinding` whose boundary has the same typed success/failure contract.
Authored `Match` arms and `FanOut` lanes recursively contain authored ordered blocks; an operation
or lane root declares its lexical default, while nested branch arms inherit the exact enclosing
scope. Expansion resolves every `UseLexicalDefault` and validates every explicit handler before
constructing the sealed failure plans below. An explicit handler carries its typed continuation
block: it may recover the protected call's output or take the exact lexical failure exit. Terminal
mapping sugar supplies the trivial continuation ending in that scope's `Fail` or failure `Yield`;
custom recovery declares its route `Match` and states in the continuation block.
Branch, failure, and lane sub-blocks have narrower local `Yield`/`Recover` exits as defined below.
`FragmentBinding` is an internal lexical composition form produced by expansion, not a sixth
author-visible control construct.
Sequence is implicit in the ordered bindings of a block. There is no public arbitrary `Sequence`
graph node or general jump instruction.

### Authored, expanded, and certified program

- `AuthoredProgram`: the canonical result of pure operation authoring. It may contain typed
  `OperationCall` declarations.
- `ExpandedProgram`: the canonical structured program after semantic capability requirements,
  recursive child-operation substitution, framework policies, and failure completion inject
  their ordinary states. It contains no `OperationCall`.
- `CertifiedProgram`: the validated, content-addressed execution authority admitted for a run.

Admission retains the authored program, expansion profile and manifests, expanded program,
certificate, and implementation closure.

### Structural path and identities

Every occurrence has:

- a `SemanticCallId`, derived from the complete authored call-instance path plus the stable local
  label and preserved for a protected call through wrapping; and
- an `OccurrenceId`, derived from the complete normalized structural path, including expansion
  policy identity, local expansion label, branch arm, fan-out group, and lane ordinal.

Lexical order determines execution. Stable labels anchor semantic identity. Inserting one earlier
step changes the program hash and order but need not rename every later semantic call.
Labels are unique within their lexical authoring or expansion scope. A state introduced by
expansion receives a semantic subcall identity derived from the protected call and the policy's
unique local path.
Stable `Match` arm labels and `FanOut` lane keys extend the authored call-instance path for calls
inside those scopes. Declaration and lane ordinals determine order, but are never substitutes for
those stable identity keys.

### Implementation and physical binding

An immutable implementation binding selects the certified state/capability contract and registered
invoker code. A secret-free physical binding reference qualifies the concrete route, signer, or
resource-lineage head used for one access attempt. The latter may advance under the admitted
stable lineage without changing program semantics. It is not the private signer or writer
credential.

A `QualifiedPhysicalBinding<K>` is a sealed assembly object that combines:

- an immutable secret-free certificate reference registered under that stable lineage and
  implementation binding; and
- the private invoker handle and credential needed to use the concrete target.

The qualification registry retains immutable public certificates and their monotonic lineage
relation. The run-history store receives only a purpose-limited verifier for those certificates,
not signer, resource-mutation, or generic database authority. A public reference alone cannot
construct `QualifiedPhysicalBinding<K>` or call preparation.

### Typed lexical value

A value handle names an admission root or an exact producer that lexically dominates its consumer.
A branch-local value cannot escape its arm except through a certified same-type merge. A fan-out
lane value cannot escape before the join and retains its lane provenance afterward.

### State outcome

The semantic result of one state:

```text
StateOutcome<Output, Failure> =
    Success(Output)
  | Failure(Failure)
```

A state failure is typed domain truth. It selects a failure continuation; it is not automatically
the terminal run result. `Return` and `Fail` terminate the operation.

### Failure handler

An ordinary injected `Pure` state that consumes the exact producer-bound state or fragment-boundary
failure and returns one closed scope-defined failure route. It is infallible and performs no IO.

An operation scope supplies a default that maps to its operation failure. A fan-out lane scope
supplies a default that maps to its lane failure. A custom handler may select an explicit recovery
branch. The recovery states themselves are ordinary states.

### Semantic execution kind

The certified property of a state:

- `Pure`: deterministic local computation with no semantic external IO.
- `Read`: one typed external observation that does not intentionally mutate the target.
- `Effect`: one typed operation that may mutate an external target or consume an exclusive
  capability.

This is semantic metadata, not three independent Runtime engines.

### Fan-out

An explicit bounded set of declaration-ordered lanes. Cardinality and lane identity are frozen
before admission. Each lane contains only `Pure` and `Read` states, has no cross-lane references,
and yields exactly one typed result. The containing block continues only after collect-all join.

Concurrency is permitted operationally; semantic result order is always declaration order.

### Expansion policy

A pure registered transformation that replaces one eligible state slot with a typed structured
fragment. A wrapper receives an affine protected slot that cannot be cloned. It may guard the slot
through an explicit branch, but it cannot duplicate it or move it relative to sibling
declarations.

### Access kind

The sealed Runtime protocol parameter:

```text
AccessKind = Read | Effect
```

Prior-run fact selection is an ordinary `Read` bound to the sealed, purpose-limited RunHistory
fact scanner. Its certified request carries the completeness and source-manifest contract. It uses
the same access bracket and does not create a fourth state or access kind.

### Safe failure

A reviewed, bounded, redaction-safe definite operational completion for which no typed response
is available or appropriate. It contains enough qualified evidence for the state to settle
deterministically. It does not inherently prove non-entry and never authorizes another attempt.

### Protected non-application and possible entry

- `SupersededBeforeEntry`: qualified evidence that the protected semantic mutation or exclusive
  capability consumption was not applied. The resource authority may have executed a read or
  fencing transaction to prove that fact.
- `EntryUnknown`: the available evidence cannot exclude target entry.

On recovery, an unmatched effect authorization is reported as possible-entry ambiguity equivalent
to `EntryUnknown`. An unmatched read is reported as `ReadCompletionUnknown`: no mutation ambiguity
exists, but no response was committed either. The persisted fold state remains
`Authorized<Effect>` or `Authorized<Read>` because history cannot prove process loss. Absence of an
observation never proves non-application.

### One bounded semantic interaction

One already-selected operation with one immutable typed request and one closed completion. An
adapter may use the minimum lower-level primitives needed for that operation, but it may not
select another semantic operation, retry, poll, fail over, fold history, or terminalize a
workflow.

### Resource authority

A narrow durable adapter/store that serializes a cross-run invariant that no individual run
journal can prove. The wallet-nonce authority is the current example.

### Adapter

Private live-crate glue that consumes one Runtime authorization, performs one selected semantic
operation using lower primitives, and totalizes every normal completion. It owns no program,
cursor, scheduling, failure-handler, or run-history lifecycle.

### Transport

A reusable mechanism for bounded encoding, exchange, and checked decoding. It owns no MFM journal,
state, scheduling, semantic retry, or domain progression.

### Semantic and operational telemetry

- Semantic audit data affects or attests run meaning and is represented by ordinary certified
  states, facts, access records, or explicit effects.
- Operational logs, metrics, and spans observe redacted Runtime or committed-history events and
  never affect the program cursor or outcome.

## Required Guarantees

### G-01: One structured execution authority

Only the admitted `CertifiedProgram` defines legal state order, branches, fan-out lanes, terminal
paths, capabilities, and bindings. No graph, adapter, application callback, or runtime-origin flag
is a second authority.

### G-02: Declaration order is semantic

Outside `FanOut`, at most one executable state occurrence is current. A later declaration cannot
execute, authorize, or settle before the current declaration resolves and structured control
advances.

### G-03: Expansion is pure, finite, and visible

Identical canonical authoring input, registry, expansion profile, and manifests produce
byte-identical expanded bytes and hashes. Expansion performs no ambient IO. Every injected state
is present in the admitted program with exact structural identity and bindings.

### G-04: Every fallible boundary is handled

Every state or fragment boundary whose certified failure contract is not the exact kernel `Never`
contract has exactly one sealed `FailurePlan` after expansion. That plan either enters its one
explicit or lexical-default handler, or propagates through the exact affine fragment boundary to
one eventual call-site handler. It cannot do both, bypass the handler, or escape to an unrelated
scope. A `Never`-failing source has `NoFailure` and no handler.

### G-05: One run-history mutation owner

Only Runtime instances hold production `RunHistoryWriter` authority. Applications, schedulers,
states, adapters, transports, replay, CLI, REST, and telemetry observers receive no append
authority.

### G-06: Authorization precedes possible entry

No read, effect, resource transaction, signer operation, provider call, or purpose-limited fact
scan begins until its exact authorization positively commits.

An existing authorization, ambiguous append acknowledgement, or stale candidate cannot mint live
authority.

### G-07: One affine invocation and one closed normal completion

A newly committed authorization mints one non-cloneable `Authorized<K>`. Consuming it permits
exactly one entry into the registered invoker for the exact operation and binding. Typing cannot
prove how many lower-level calls arbitrary adapter code makes; adapter qualification, bounded
operation contracts, and fault-injection tests must prove the declared no-retry/no-duplication
behavior.

After the registered invoker accepts that authority, it has no outer normal error channel. Every
surviving return becomes one bounded persistable completion.

### G-08: Pending observation cannot escape

The invoker completion is immediately owned by Runtime as `PendingObservation<K>`. No normal
successful drive result, state callback, application result, or live value can escape before exact
observation commit or identical-content resolution.

### G-09: Observation is exactly linked

Every observation names exactly one authorization and preserves its access kind, occurrence,
semantic call, operation, binding, request, cursor anchor, and outcome contract. At most one
observation exists per authorization.

### G-10: Only the current cursor may advance

The store accepts a transition or authorization only for the current sequential occurrence or one
eligible active fan-out lane. Runtime cannot ask the store to choose another instruction.

### G-11: Unresolved same-occurrence access overlap is unrepresentable

An Effect occurrence with an unmatched authorization or committed `EntryUnknown` evidence has no
transition that creates another authorization or advances to a later step. This prevents a second
possible external mutation.

A new authorization for the same semantic occurrence is legal only after the previous
authorization committed qualified `SupersededBeforeEntry` evidence and the fold produced the
next `Refreshable` attempt ordinal. It must be prepared by a worker holding the currently
qualified physical binding.

Read recovery is a separate unresolved liveness contract because a new read cannot mutate the
target but can produce a different time-varying value. This RFC authorizes no ad hoc read
reauthorization until the Material Uncertainties section's selection rule is resolved.

### G-12: Branch and fan-out control is deterministic

The store derives a `Match` arm from the canonical tag of a committed closed sum. The certificate
contains the exhaustive tag-to-arm table; the caller does not write a branch choice and the store
runs no discriminator callback.

Fan-out lanes and joined results retain declaration order regardless of authorization, completion,
or append order. Effects are rejected transitively inside fan-out.

### G-13: Non-domain evidence cannot become domain truth

Only a committed typed state or fragment-boundary failure may enter a failure handler. Raw
provider text, audit-only platform faults, integrity violations, process loss, store failure,
unmatched authorization, and `EntryUnknown` are not failure inputs.

### G-14: Ordinary definite failures terminate or follow explicit recovery

Every expected definite state-facing operational completion is admitted as either a reviewed typed
`Returned(Response)` or a reviewed capability `SafeFailure`. The state maps it deterministically
to success, typed failure, or invalid evidence. A typed failure enters the explicit/default
operation failure path and cannot leave the run permanently open merely because it is an ordinary
error.

A closed failed run grants no authority over a later run, and its run-history status cannot block
admission or execution of a later `run_id`. Independent resource authorities may still enforce
their durable invariants.

### G-15: Store validation remains authoritative

Runtime proposes sealed append material. The store alone validates legal record shape, exact
predecessor, logical-key uniqueness, object closure, cursor legality, provenance, access linkage,
semantic versus journal heads, and closure before atomic append.

### G-16: Cross-run resources have one narrow authority

Every resource domain has one qualified durable owner of its uniqueness rule. Every actor capable
of using the protected EVM sender either uses that same authority or is permanently fenced out.

### G-17: Normal resource rotation cannot poison semantics

A resource request first resolves its exact permanent semantic operation key. An already committed
byte-identical result is the original operation result even when the caller's physical binding has
since become stale; internal `ExistingSame` resolution does not perform another mutation or create
another Runtime authorization. If no exact result exists, a stale resource binding has only the
atomic `SupersededBeforeEntry(public_lineage_head_ref)` outcome. That proof cannot construct state
output, state failure, or semantic cursor advancement. The callback-free fold alone converts it
into a sealed `Refreshable` state with the next attempt ordinal. Only a newly assembled worker
holding the current private credential may prepare and authorize that attempt; the stale worker
cannot self-upgrade.

### G-18: No secrets

Authored/expanded programs, manifests, requests, observations, reservation evidence, state
failures, facts, histories, traces, exports, telemetry projections, and public results contain no
credentials, private keys, signatures, raw signed envelopes, provider-controlled text,
secret-bearing endpoints, or secret paths.

### G-19: No generic executor survives

No executor history, fold, scheduler, fence, delivery plan, target authority, frontier, tombstone,
or compatibility path remains after the cutover.

## Target Architecture

### Responsibility placement

| Concern | Sole owner |
| --- | --- |
| Declaration order, branch/fan-out shape, child composition, operation failure contract | operation authoring |
| Required expansion profile, versions, and security/control coverage | qualified entry-point admission policy |
| Pure state and policy injection | deterministic expansion |
| Static structure, type, bounds, capability closure, profile coverage, and provenance | certification |
| Domain request authorship and observation interpretation | state |
| One run's current action | Runtime over a store-minted verified cursor |
| Run-history mutation orchestration | Runtime |
| Legal append, exact-head CAS, callback-free fold, cursor, and closure | `RunHistory` store |
| One already-selected live operation | adapter |
| Protocol exchange | transport |
| Signer generation and secret custody | qualified signer |
| Cross-run sender/nonce uniqueness and physical generation | wallet-nonce authority |
| EVM progression and terminal meaning | EVM states and expansion |
| Recorded verification | store/replay over committed evidence |
| Prior-run fact scan completeness | sealed purpose-limited RunHistory scanner |
| Best-effort logs, metrics, and spans | non-authoritative observers |
| Authentication and rendering | app, CLI, and REST |

No row is owned by an executor.

### Expanded and certified structured program algebra

Conceptually:

```text
OperationProgram<Output, Failure> =
    OrderedBlock<
        SequentialPolicy,
        OperationExit<Output, Failure>,
        OperationExit<Output, Failure>,
    >

SequentialPolicy {
    allowed_state_kinds: Pure | Read | Effect,
    fan_out: Allowed,
}

FanOutLanePolicy {
    allowed_state_kinds: Pure | Read,
    fan_out: Forbidden,
}

OrderedBlock<Policy, Exit, FailureExit> {
    declarations: [Binding<Policy, Exit, FailureExit>],
    exit: Exit,
}

Binding<Policy, Exit, FailureExit> =
    StateBinding {
        output_local,
        stable_label,
        call where call.Kind is in Policy.allowed_state_kinds,
        failure: FailurePlan<
            Policy,
            call.Failure,
            call.Output,
            FailureExit,
        >,
    }
  | MatchBinding {
        output_local,
        stable_label,
        selector: ClosedSumValue,
        exhaustive_arms: [
            TaggedArm {
                canonical_tag,
                stable_arm_label,
                body: ArmBlock<
                    Policy,
                    Value,
                    Exit,
                    FailureExit,
                >,
            },
        ],
    }
  | FanOutBinding {
        exact_policy: Policy.fan_out == Allowed,
        output_local: TypedLocal<
            DeclaredOrderVector<
                StateOutcome<LaneOutput, LaneFailure>,
            >,
        >,
        stable_group_label,
        bound,
        ordered_lanes: [
            Lane {
                stable_lane_key,
                declaration_ordinal,
                body: LaneBlock<Output, LaneFailure>,
            },
        ],
    }
  | FragmentBinding {
        fresh_lexical_region,
        output_local,
        stable_label,
        boundary: FragmentBoundary<
            fresh_lexical_region,
            Input,
            Output,
            Failure,
        >,
        body: FragmentBlock<
            Policy,
            fresh_lexical_region,
            Output,
            Failure,
        >,
        failure: FailurePlan<
            Policy,
            Failure,
            Output,
            FailureExit,
        >,
    }

OperationExit<Output, Failure> =
    Return(Output) | Fail(Failure)

ArmBlock<Policy, Value, EnclosingExit, FailureExit> =
    OrderedBlock<
        Policy,
        Yield(Value) | EnclosingExit,
        FailureExit,
    >

FailurePlan<Policy, Failure, RecoveredOutput, FailureExit> =
    Infallible {
        exact_contract: Failure == Never,
        continuation: NoFailure,
    }
  | Handled {
        exact_contract: Failure != Never,
        before_handler: PreHandlerBlock<
            Policy,
            Failure,
            FailureExit,
        >,
        handler: PureNeverHandlerBinding<
            Policy,
            Failure,
            HandlerRoute,
        >,
        after_handler: HandlerContinuationBlock<
            Policy,
            HandlerRoute,
            RecoveredOutput,
            FailureExit,
        >,
    }
  | Propagate<LexicalRegion, BoundaryFailure> {
        exact_contract: Failure != Never,
        boundary_contract: BoundaryFailure != Never,
        affine_enclosing_boundary:
            EnclosingFragmentBoundaryToken<
                LexicalRegion,
                BoundaryFailure,
            >,
        continuation: PropagatingFailureBlock<
            Policy,
            LexicalRegion,
            Failure,
            BoundaryFailure,
        >,
    }

FragmentFailureExit<Failure> =
    Yield(StateOutcome::Failure(Failure))

PreHandlerBlock<Policy, Failure, FailureExit> =
    OrderedBlock<
        Policy,
        EnterDesignatedHandler(ProducerBound<Failure>),
        FailureExit,
    >

PureNeverHandlerBinding<Policy, Failure, HandlerRoute> =
    StateBinding {
        exact_policy: Pure is in Policy.allowed_state_kinds,
        output_local,
        stable_label,
        call: CertifiedStateCall {
            kind: Pure,
            input: ProducerBound<Failure>,
            output: HandlerRoute where HandlerRoute is a closed tagged sum,
            failure: Never,
        },
        failure: Infallible {
            exact_contract: Never == Never,
            continuation: NoFailure,
        },
    }

HandlerContinuationBlock<
    Policy,
    HandlerRoute,
    RecoveredOutput,
    FailureExit,
> = {
    selector: ProducerBound<HandlerRoute>,
    exhaustive_routes: [
        TaggedRoute {
            canonical_tag,
            stable_route_label,
            body: OrderedBlock<
                Policy,
                Recover(RecoveredOutput) | FailureExit,
                FailureExit,
            >,
        },
    ],
}

PropagatingFailureBlock<
    Policy,
    LexicalRegion,
    SourceFailure,
    BoundaryFailure,
> =
    OrderedBlock<
        Policy,
        PropagateToFragmentBoundary {
            boundary:
                EnclosingFragmentBoundaryToken<
                    LexicalRegion,
                    BoundaryFailure,
                >,
            source: ProducerBound<SourceFailure>,
            boundary_failure: ProducerBound<BoundaryFailure>,
        },
        FragmentFailureExit<BoundaryFailure>,
    >

FragmentBlock<Policy, LexicalRegion, Output, Failure> =
    OrderedBlock<
        Policy,
        Yield(StateOutcome<Output, Failure>),
        FragmentFailureExit<Failure>,
    >

LaneFailureExit<LaneFailure> =
    Yield(StateOutcome::Failure(LaneFailure))

LaneBlock<Output, LaneFailure> =
    OrderedBlock<
        FanOutLanePolicy,
        Yield(StateOutcome<Output, LaneFailure>),
        LaneFailureExit<LaneFailure>,
    >
```

`FailurePlan` is a sealed disjoint certificate witness. Only the exact kernel `Never` contract
constructs `Infallible`. Every other admitted failure contract constructs either `Handled` or,
only for a protected slot inside an affine fragment, `Propagate`. A handled plan's normal
pre-handler exit is exactly `EnterDesignatedHandler`; it cannot jump directly to recovery or a
scope exit. A propagation plan's normal exit names its exact enclosing fragment boundary and
retains the source and mapped-failure provenance. Certification proves that finite nested
propagation ends in exactly one handled call-site plan.

Match tags and arm labels are unique and exhaustive for the certified closed sum. Fan-out group
labels and lane keys are unique in their lexical scope; declaration ordinals freeze result order
but are not substitutes for stable identity.

`Policy` is a sealed type parameter, not descriptive metadata. `MatchBinding`, fragments, failure
posts, and recovery routes retain the containing policy. Only `SequentialPolicy` constructs a
`FanOutBinding`; entering a lane replaces it with `FanOutLanePolicy`, which removes `Effect` and
nested `FanOut` constructors transitively. No child block can widen its policy.

Each `FragmentBinding` mints a fresh private lexical region and gives only its body the
non-cloneable `EnclosingFragmentBoundaryToken` for that region. `Propagate` requires that exact
token and a non-`Never` boundary contract, so it cannot name a sibling, ancestor, unrelated
fragment, or zero-handler boundary. Its boundary failure is either the exact type-compatible
source rebound with fragment provenance or the producer-bound output of an ordinary certified
`Pure` failure-post mapping state; no hidden conversion callback exists.

The parameterized exits are lexical, not general jumps. A binding's successful value is assigned
to its local and execution continues with the next declaration in that ordered block. Each
lexical block fixes the failure exit available to all of its fallible bindings. An operation
failure path may recover or take an operation exit; a lane failure path may recover or yield the
lane's typed failure, but cannot close the operation. `StateBinding` failure enters its sealed
failure plan. A handled or propagating plan may contain certified failure-post states before its
normal exit. If a failure-post state itself fails, that new committed failure follows the
post-state's own exact continuation; otherwise a handled source must enter its mapping handler
before recovery or termination, while a propagation source must reach its exact affine fragment
boundary.
`Recover` supplies the original binding's output type and resumes the remaining declarations;
`Return` or `Fail` terminates the containing operation. A `Match` arm either yields its binding
value or takes an exit explicitly allowed by its enclosing block; a fallible binding inside that
arm uses the separate exact `FailureExit`. Consequently a `Match` inside a pre-handler or
propagation block cannot jump to the containing scope's failure exit and bypass its required
handler or boundary. A lane has only its local typed `Yield`; operation `Return` and `Fail` are not
constructible there.

`PureNeverHandlerBinding` is a complete ordinary `StateBinding`, including stable label, output
local, structurally derived identities, and its `Infallible` plan. Its exact output is the selector
of an exhaustive, stable-labelled route table; a continuation cannot ignore the handler output,
match an unrelated value, or recover before selecting one of those routes.

`FragmentBinding` is the certified lexical composition form for a child operation, semantic
capability expansion, or policy envelope. Its nested block yields one typed `StateOutcome`;
success binds the output local and failure enters the call site's failure continuation. It is not
an authored `OperationCall`, a Runtime action, or a new public control construct. The one shared
continuation remains after the binding, so expansion neither duplicates it nor creates a jump.
Its boundary freezes the semantic call identity and exact input, output, and failure contracts.
Entering or leaving the fragment creates no fake semantic transition or copied value: the fold
binds the boundary local to the exact existing producer reference while retaining both underlying
and fragment-boundary provenance.

This is a structured tree with lexically nested sub-blocks, not graph edges or general bytecode.
Sub-blocks never name instruction IDs, alias an arbitrary continuation, or jump into another
block; the parent encodes its one lexical continuation once. An implementation may derive an
indexed instruction table as a process-local cache, but that table is not separately admitted or
hashed.

Every syntactic path ends in an exit allowed by its lexical scope. There are no backedges,
arbitrary jumps, dependency skips, required-success sets, or implicit terminal nodes. The initial
contract also rejects `FanOut` transitively inside a `LaneBlock`.

### Operation DSL

The public authoring surface should make order and fan-out visually explicit while keeping
sequence implicit:

```rust
operation::<Snapshot, SnapshotFailure>("snapshot", |op| {
    op.default_failure::<MapStateFailure>();

    let anchor = op
        .read::<ReadAnchor>("anchor", input)
        .or_default();

    let balances = op.fan_out::<MAX_ASSETS>(
        "balances",
        assets,
        |lane, asset| {
            lane.default_failure::<MapBalanceFailureToLane>();

            lane.read::<ReadBalance>(
                "read",
                (anchor, asset),
            )
            .or_default()
        },
    );

    let snapshot = op
        .pure::<Aggregate>("aggregate", balances)
        .on_failure::<HandleAggregationFailure>();

    op.return_value(snapshot)
});
```

This is illustrative, not a frozen Rust API. The required properties are:

- builder handles are private or affine enough to prevent forward references and core
  duplication;
- declaration order is retained exactly;
- stable labels are explicit and unique in their lexical scope;
- an authored `OperationCall` has a typed success/failure boundary and cannot survive expansion;
- `or_default` is permitted only when a compatible lexical default handler exists;
- a lane declares its own mapper unless its failure contract is exactly compatible with an
  explicitly inherited default;
- branch arms are exhaustive;
- continuing arms yield compatible types;
- lane values cannot cross lane boundaries; and
- terminal instructions are unavailable inside fan-out lanes.

More complex predicates are computed by an ordinary `Pure` state into a closed enum and then
matched. Closed sums use the kernel-owned canonical tag encoding frozen in the certificate.
Runtime and store never execute an unrecorded branch predicate or discriminator callback.

### Pure expansion

An expansion replaces one typed call slot with a structured fragment having the same external
boundary:

```text
Expand<Call<Input, Output, Failure>>
    -> Fragment<Input, StateOutcome<Output, Failure>>
```

A wrapper receives an affine protected slot:

```text
Around<S>:
    Hole<S> -> Fragment<
        S::Input,
        StateOutcome<S::Output, S::Failure>,
    >
```

The hole occurs structurally once. A precondition may choose a branch that does not execute it,
but no expansion can clone it, execute it twice, move it across sibling declarations, or capture
it inside fan-out.

A representative wrapper normalizes to:

```text
outer.pre
  inner.pre
    protected
  inner.after_success | inner.after_domain_failure
outer.after_success | outer.after_domain_failure
```

Profile order is outer-to-inner on entry and reverses on exit.
Failure-post fragments run before the enclosing lexical failure handler and must preserve or
explicitly map the wrapped call's declared failure boundary. Success-post fragments likewise
yield the wrapped call's declared success boundary.

The protected failure propagates affinely through the `FragmentBinding` as part of the same
failure continuation. Certification inserts no second inner handler for that propagated boundary.
States introduced inside the fragment retain their own lexical failure continuations. If a
failure-post state itself fails, that new failure follows the post-state's continuation; if it
settles successfully, the protected failure proceeds to its one call-site handler.

There is no implicit `finally`. Nothing can promise a post-state after process loss or while an
effect remains possible-entry ambiguous.

Expansion policies may not:

- inspect network, filesystem, clock, environment, store, Runtime, current history, adapter, or
  transport;
- reorder or remove sibling operation declarations;
- create cross-slot dependencies or use non-dominating values;
- introduce loops, jumps, detached work, or unbounded fan-out;
- hide an `Effect` inside fan-out;
- choose hidden, dynamic, or open-ended physical scheduling, retry, polling, backoff, or failover;
- construct live capability authority; or
- retain opaque executable callbacks in the certified program.

An expander may insert a finite, statically bounded sequence of distinct semantic retry, polling,
replacement, or fallback occurrences. Their requests, branches, order, and bound are visible in
the expanded program. No adapter or Runtime loop chooses additional occurrences.

The expansion pipeline is:

1. recursively substitute authored `OperationCall`s;
2. lower registered semantic capability requirements;
3. apply the entry point's required framework/security policies in frozen profile order;
4. insert exact default failure handlers for every remaining uncovered fallible state or fragment
   boundary, following affine propagated boundaries to their one outer call site;
5. normalize, content-address, and certify the final structure.

A child `Return` yields the child call's success value at its call site. A child `Fail` yields the
child call's typed failure at that call site. Neither terminal closes the parent operation.
Expansion preserves the child's lexical defaults internally and leaves no child call in
`ExpandedProgram`.

A policy never reapplies to states it injects itself. Later phases or policies may cover injected
states only through explicit provenance-based eligibility. Expansion dependencies are acyclic and
subject to hard depth, occurrence, branch, and fan-out bounds.

The qualified entry-point admission policy selects the required profile, exact policy versions,
and coverage obligations. Certification proves that the expanded program matches them; admission
rejects an empty, weaker, or caller-substituted profile. The registered semantic state/capability
contract selects capability expansion. Runtime never discovers topology by inspecting which
adapter or transport happens to implement an operation.

### Failure handlers and custom recovery

After state settlement:

```text
StateOutcome<Output, Failure>
    Success(output) -> success continuation
    Failure(failure) -> exact structural failure continuation
                         -> failure-post states, if any
                         -> affine fragment boundary, if wrapped
                         -> designated failure handler
```

This split is part of `StateBinding` normalization, not an author-visible `Match`. No raw failure
arm can bypass the certified failure continuation. If an intervening failure-post state fails, its
new typed failure follows that state's own exact continuation.

The default handler contract is:

```text
DefaultFailureHandler<Source, Scope>:
    Kind    = Pure
    Input   = ProducerBound<Source::Failure>
    Output  = Scope::Failure
    Failure = Never
```

`Source` is a certified `StateBinding` or `FragmentBoundary`.

For an operation scope, the normal default expansion is:

```text
Failure(f):
    operation_failure = DefaultFailureHandler(f)
    Fail(operation_failure)
```

For a fan-out lane scope, the same shape yields
`StateOutcome::Failure(LaneFailure)` from the lane instead of terminating the containing
operation. A custom handler may instead produce a closed scope-defined route enum followed by an
exhaustive `Match`. IO recovery, fallback, compensation, or a changed external request appears as
ordinary states in the selected branch. The handler itself neither performs IO nor returns a
generic `Retry` command.

Defaults are lexical:

- an operation owns defaults for calls it authors;
- a child operation owns its internal defaults;
- an expansion owns failures from states it injects and maps them to the wrapped call boundary;
  and
- certification rejects every uncovered fallible state or fragment boundary.

The failed source transition, exact failure object, handler input, handler output, and subsequent
branch remain independently visible in append-only history. Recovery never erases the original
failure.

### Fan-out

The initial contract is:

```text
FanOut<MAX> {
    ordered lanes fixed before admission,
    each lane: bounded LaneBlock<LaneOutput, LaneFailure>,
    transitive execution kinds: Pure | Read,
    join: collect all in declaration order,
}
```

Rules:

- lane cardinality may derive from canonical planning input but not runtime-discovered values;
- lane keys are unique and stable;
- lanes capture only immutable values that dominate the fan-out;
- lanes cannot reference each other;
- lanes cannot contain nested fan-out in the initial contract;
- lanes cannot `Return` or `Fail` the containing operation;
- every lane yields exactly one typed outcome;
- each lane is its own lexical failure scope whose default pure handler yields a typed lane
  failure rather than terminating the containing operation;
- any custom lane recovery remains transitively `Pure` or `Read`;
- the join waits for every lane; and
- the result vector uses declared lane order, never completion order.

An outer `Pure` state may summarize lane outcomes and select a normal operation branch. This avoids
fail-fast cancellation, incomplete access histories, and nondeterministic “first failure” meaning.

Runtime chooses the lowest declaration-ordered actionable lane. A concurrent driver may advance a
different lane only after the earlier lane is durably waiting on a read observation. Exact-head
CAS prevents two workers from authorizing the same lane at the same cursor.

### Certified program and folded cursor

The callback-free store fold derives:

```text
VerifiedProgramState {
    cursor:
        At(structural_path)
      | InFanOut {
            group_path,
            declaration_ordered_lane_states,
        }
      | Closed {
            outcome_ref,
        },

    live_lexical_bindings,
    per_occurrence_access_state,
    journal_head,
    semantic_head,
    semantic_state_digest,
}
```

After admission or a semantic transition, the fold normalizes through sequence boundaries,
`Match`, fan-out entry/join, yields, `Return`, and `Fail` until it reaches the next executable
occurrence or closure.

No mutable cursor/status row is semantic authority. A backend may materialize an index only when
it is verified against the authoritative prefix and exact head.

### Runtime action derivation

Runtime receives a sealed verified view and derives only:

```text
CommitLocalState
AuthorizeCurrentAccess
SettleCurrentObservation
Closed
Waiting
BlockedIntegrity
```

Inside fan-out, the current access may name one eligible lane. There is no global node scan,
authorization-count spreading, alternative-source readiness, dependency skip, or action-family
fairness rule.

One `drive_once` performs at most one semantic transition or one audited access operation. Pure
control normalization is folded into the transition commit that produced its discriminant; it
does not require fake state or control records.

When a semantic transition normalizes directly to `Return` or `Fail`, `RunClosed` is committed in
the same atomic append. If initial normalization reaches a terminal without a state transition,
admission atomically appends `RunAdmitted` and `RunClosed`. This includes an admission-root match
or an empty fan-out whose join is already defined. Closure is illegal while the current occurrence
or any entered fan-out lane has unresolved access. No semantic or audit record is accepted after
closure.

## State And Access Algebra

### One closed state algebra

```text
StateExecution =
    Pure(PureStateContract)
  | Read(ReadStateContract)
  | Effect(EffectStateContract)
```

Conceptually:

```text
Pure:
    apply(StateFrame)
      -> StateOutcome<Output, Failure>

Read:
    request(StateFrame)
      -> ReadRequest

    settle(
        StateFrame,
        CommittedObservationView<
            Returned(ReadResponse)
          | SafeFailure(ReadSafeFailure)
        >
    ) -> StateOutcome<Output, Failure>
       | InvalidEvidence

Effect:
    request(StateFrame)
      -> EffectRequest

    settle(
        StateFrame,
        CommittedObservationView<
            Returned(EffectResponse)
          | SafeFailure(EffectSafeFailure)
        >
    ) -> StateOutcome<Output, Failure>
       | InvalidEvidence
```

Request authorship is total over the certified frame. A state that may terminate without external
IO is preceded by an explicit `Pure` decision and `Match`, normally injected by expansion.
Conditional progression belongs in the structured operation, not in a state-selected next
action.

There is no generic `BlockedUnresolved` state result. A bounded domain policy that exhausts its
evidence returns a typed failure or closed enum and follows its explicit operation branch.
Physical store inability, integrity failure, and possible-entry ambiguity may still make Runtime
report `Waiting` or `BlockedIntegrity`; those are not domain outcomes.

Every semantic retry, poll, fallback, replacement, or changed request is a distinct explicit state
occurrence inserted by operation authoring or pure expansion.

### Private access protocol

The only live sequence is:

```text
prepare<K>(QualifiedPhysicalBinding<K>)
  -> Prepared<K>

authorize(Prepared<K>)
  -> append ExternalAccessAuthorized
  -> NewlyAppendedAuthorization<K>
  -> Authorized<K>

invoke(Authorized<K>)
  -> AccessCompletion<K>
  -> PendingObservation<K>

commit_observation(PendingObservation<K>)
  -> append or resolve ExternalAccessObserved
  -> CommittedObservation<K>
```

Constructors and fields of authority-bearing types are private.

`Authorized<K>` owns the exact prepared request and binding. Passing a second request beside the
authority is forbidden.

The runtime-facing completions are exhaustive:

```text
AccessCompletion<Read> =
    Returned(ReadResponse)
  | SafeFailure(ReadSafeFailure)
  | IntegrityFault(AccessFaultCode)

AccessCompletion<Effect> =
    Returned(EffectResponse)
  | SafeFailure(EffectSafeFailure)
  | SupersededBeforeEntry(PhysicalBindingRefreshEvidence)
  | EntryUnknown(AccessFaultCode)
  | IntegrityFault(AccessFaultCode)
```

`SupersededBeforeEntry` is available only to qualified resource operations whose authority can
atomically prove that the protected semantic mutation or exclusive capability consumption was not
applied. It is physical control evidence and is never state-consumable. Its evidence contains an
opaque secret-free public lineage-head reference under a manifest-declared generic
physical-binding contract. The store fold retains that reference in `Refreshable` as a monotonic
lower bound; a separately assembled current worker supplies a binding whose public certificate is
that head or a verifier-proven non-rollback descendant in the same admitted stable lineage.
Runtime interprets no resource fields. The private writer credential is never returned or
persisted.

`EntryUnknown` is audit evidence that the authorized call's completion is unavailable and target
entry cannot be excluded. It leaves the occurrence current and permits no new authorization for
that occurrence.

An unmatched read authorization remains folded as `Authorized<Read>`. A recovery view with no
corresponding live affine token reports `ReadCompletionUnknown`, not `EntryUnknown`. The
RFC deliberately defines no successor until the read-recovery policy recorded under Material
Uncertainties is chosen. It must not be treated as an Effect ambiguity or silently reauthorized.

`IntegrityFault` is audit-only and blocks. It never becomes state failure.

There is no outer `Result` after authority consumption. Internal code may use `Result`, but the
private invocation wrapper must totalize every normal return into one bounded canonical completion.

Rust cannot guarantee observation after process death. The enforced type property is narrower:

```text
no constructible normal-return path skips exact observation commit
```

### Prepared access

Before authorization, Runtime and store preparation freeze:

- run, occurrence, semantic call, and current semantic head;
- structured cursor and fan-out lane when applicable;
- certified execution and access kind;
- operation identity and implementation binding;
- immutable typed request and canonical bytes;
- request, response, safe-failure, and fault contracts;
- the optional certified protected-non-application refresh contract and its stable lineage;
- stable qualified routing, signer, and resource-lineage references where applicable;
- the current secret-free physical binding reference;
- a fold-derived access-attempt ordinal and stable attempt identity; and
- exact producer-bound input provenance.

Any failure here happens before live authority and appends no authorization.

There is no raw-reference Runtime preparation API. Runtime-private preparation accepts the sealed
qualified binding and retains its private invoker handle. It sends only the immutable secret-free
public certificate and producer-free request material to the store. The store verifies that
certificate against the admitted stable lineage and implementation binding and authors the
attempt's public binding reference itself. A positive append proof lets Runtime combine the
retained handle with `NewlyAppendedAuthorization<K>` to construct `Authorized<K>`; an ambiguous or
rejected append cannot do so. Replay revalidates the certificate relation without ever obtaining
the private invoker handle.

A generic `AccessAttemptId` is derived by the kernel from the exact run, occurrence, attempt
ordinal, structured cursor/semantic-head anchor, operation, immutable implementation binding,
current physical binding reference, and request digest. The first attempt has ordinal zero.

A definitely rejected stale authorization candidate consumes no ordinal. Runtime reloads and may
prepare that ordinal again; if a fan-out sibling changed the semantic anchor, the rebuilt attempt
identity changes with it. Journal-only rebasing preserves the logical identity and content while
changing only the predecessor-bound append envelope. An ambiguous acknowledgement must resolve
the unchanged original identity before any rebuild. Once an authorization positively exists, its
ordinal is consumed and its identity/content are immutable.

Domain resource identities may additionally use a separately certified permanent semantic intent
identity, but Runtime and the run-history store never inspect domain-specific fields.

### Attempt typestate

The callback-free fold derives one of two sealed states for the current access occurrence:

```text
ReadAttemptState =
    Ready<Read, AttemptOrdinal>
  | Authorized<Read, AccessAttemptId>
  | ObservedForSettlement<Read, AccessAttemptId, ObservationRef>
  | BlockedIntegrity

EffectAttemptState =
    Ready<Effect, AttemptOrdinal>
  | Authorized<Effect, AccessAttemptId>
  | ObservedForSettlement<Effect, AccessAttemptId, ObservationRef>
  | Refreshable<Effect, NextAttemptOrdinal, PublicLineageHeadRef>
  | EntryUnknown<AccessAttemptId>
  | BlockedIntegrity
```

Only `Ready` and `Refreshable` expose the private preparation transition. Only a committed
`SupersededBeforeEntry` observation can create `Refreshable`, and the fold increments its ordinal
exactly once. An unmatched `Authorized<Read>` is reported by the recovery view as
`ReadCompletionUnknown`; it is not a separately persisted transition. `Authorized` and
`EntryUnknown` expose no constructor for another authorization. This prevents overlapping attempts
by construction in process and by candidate validation for hostile persisted bytes.

`Refreshable` and `EntryUnknown` are Effect-only constructors. Certification includes
`SupersededBeforeEntry` in an Effect completion contract only when that contract declares the
generic protected-non-application refresh capability and one admitted stable physical lineage.
An ordinary Effect and every Read reject that outcome variant.

`Refreshable` does not contain a writer credential. A currently qualified worker must present a
new sealed `QualifiedPhysicalBinding<K>`. The purpose-limited certificate verifier proves that its
public head is the recorded `Refreshable` head or a monotonic descendant in the same admitted
stable lineage, rejecting rollback and sibling lineages, before the store prepares the next
attempt. A further rotation racing that attempt returns another qualified
`SupersededBeforeEntry`; it cannot strand or let an old worker self-upgrade. An old worker remains
fenced.

### Observation persistence

`PendingObservation<K>` owns stable logical material independently of a predecessor-bound physical
append candidate.

Runtime:

1. resolves the authorization's observation logical key;
2. accepts an existing observation only when its canonical content is identical;
3. prepares an append against the current verified journal head;
4. rebases after a definite stale-head result without reinvoking;
5. resolves acknowledgement ambiguity using the unchanged physical append attempt; and
6. returns a committed proof only after positive append or exact identical resolution.

While the task lives, store unavailability does not turn pending material into normal success.
Task or process loss may discard pending material and leave the authorization unmatched.

### Attempt and ambiguity rules

For every current occurrence:

- one newly committed authorization mints one live invocation;
- a positive observation resolves that authorization;
- a returned `SafeFailure` is interpreted by the state and never enables reauthorization;
- a returned `SupersededBeforeEntry` resolves the physical attempt but not the semantic
  occurrence and creates only the next fold-derived `Refreshable` ordinal;
- a returned `EntryUnknown` parks the occurrence;
- an unmatched effect authorization remains `Authorized<Effect>` and is reported on recovery as
  possible-entry ambiguous;
- an unmatched read remains `Authorized<Read>` and is reported as `ReadCompletionUnknown`; and
- a later semantic request is always a different certified occurrence.

An ordinary failure handler cannot rewind the cursor or mint access authority.

The initial design performs no automatic same-occurrence Effect re-entry after an unmatched
authorization. A later read after a definite result is a distinct explicit read occurrence;
recovery of an unmatched read remains unresolved rather than inheriting the Effect rule.
`EntryUnknown` cannot advance, close, or reach a later reconciliation state because it leaves the
ambiguous effect current. No manual mutation of history is defined. Any future reconciliation
must extend the same-occurrence access protocol under its own certified contract; it is not part
of this RFC. A process lease or elapsed time is never proof that the original effect was not
applied.

### Failure classification

| Evidence | State-consumable? | Required behavior |
| --- | --- | --- |
| `Returned(Response)` | Yes, through the state's pure settlement callback | Produce success, typed failure, or `InvalidEvidence`. |
| `SafeFailure` | Yes, through the state's pure settlement callback | Produce success, typed failure, or `InvalidEvidence`; never treat it as retry authority. |
| `StateOutcome::Failure(S::Failure)` | Yes, only by the exact failure handler | Follow explicit/default failure path. |
| `SupersededBeforeEntry` | No | Keep the same semantic occurrence current; fold to `Refreshable` with the next ordinal. |
| `EntryUnknown` or unmatched effect authorization | No | Keep the exact effect current with no legal successor in this RFC; a future same-occurrence protocol is required. |
| Unmatched read authorization | No | Keep `Authorized<Read>` in the fold and report `ReadCompletionUnknown`; this RFC defines no successor until the read-recovery uncertainty is resolved. |
| Integrity fault or invalid evidence | No | Block; never invent domain failure. |
| Callback, codec, or contract violation | No | Block and attribute the responsible component. |
| Journal/store interruption | No | Resume the physical persistence protocol; do not fabricate an access result. |

Production qualification must inventory every expected definite operational disposition as a
typed response, state-facing `SafeFailure`, retryable physical-control outcome, possible-entry
ambiguity, or integrity fault. When the product expects the state to fail or recover, the
capability must expose either a reviewed typed response or a reviewed redaction-safe `SafeFailure`
that the state can map into its typed outcome. Leaving such an expected definite condition as
audit-only non-domain evidence is an incomplete product contract.

## Run History And Store Enforcement

### Five record families

The append-only algebra remains:

```text
RunAdmitted
StateTransitionCommitted
ExternalAccessAuthorized
ExternalAccessObserved
RunClosed
```

These are typed events in the existing `run:*` stream family. Past records are never mutated or
reinterpreted.

Authorization and observation are audit records, not fake semantic transitions. They advance the
journal head. `RunAdmitted` initializes:

```text
SemanticHead::Genesis {
    admission_ref,
    genesis_semantic_state_digest,
}
```

That genesis value anchors access before the first state transition.
`StateTransitionCommitted` advances the semantic head thereafter. No record is accepted after
`RunClosed`.

One atomic append candidate may contain the adjacent `RunAdmitted + RunClosed` pair for an
initially terminal program or `StateTransitionCommitted + RunClosed` for a transition that reaches
a terminal. The records remain separately hashed members of the same five-family algebra; the
pair is all-or-nothing.

`RunAdmitted` binds:

- tenant, store, run, invocation, and entry-point identity;
- authored, expanded, and certified program references;
- exact required admission-policy profile, coverage proof, and policy manifests;
- state, capability, adapter, signer, and resource implementation manifests;
- configuration, context roots, initial values, and prior-run source manifests;
- immutable secret-free routing policy and stable resource-lineage references; and
- the canonical genesis semantic-state digest.

Rotating public physical-binding references are bound per access attempt under those stable
lineages. Private signer or writer credentials are assembly-only and are never admission data.

### One callback-free fold

One pure fold validates complete history, previews candidate successors, and supports recorded
replay. Runtime must not implement a competing semantic fold.

The store fold verifies:

- canonical encoding, domain-separated hashes, record and commit identities; structured JSON
  hashing retains the repository's JCS-style canonicalization contract and rejects floats;
- contiguous sequence and exact predecessor relation;
- per-append atomicity and object closure;
- content-addressed manifests, context snapshots, facts, outputs, and retained evidence;
- unique logical keys and exact-content idempotency;
- admitted program and implementation membership;
- exact current cursor and execution kind;
- lexical value dominance and producer-bound references;
- branch selection from the kernel's canonical closed-sum tags and certified arm table;
- fan-out lane eligibility, completeness, and declared result order;
- access-attempt ordinal, physical-binding, authorization, observation, refresh, and settlement
  linkage;
- typed outputs, facts, and failures;
- before/after semantic digest;
- explicit terminal result; and
- atomic closure.

The store is authoritative for persisted protocol shape, provenance, and successor legality. It
does not rerun arbitrary state callbacks during recorded verification and does not prove
mathematical domain truth. Exact reproduction may separately rerun deterministic expansion and
pure callbacks.

### Generic append-boundary rejection

Every backend rejects a candidate unless all of the following hold against the locked current
prefix:

- the transition names the current occurrence or one legal active fan-out lane;
- no earlier declaration on the exact certified lexical path was skipped;
- the occurrence has not already settled;
- the execution kind matches the certified state;
- authorization is for the current `Read` or `Effect`;
- no unresolved authorization for that occurrence already exists, except that a certified
  `Refreshable` state permits exactly its next attempt ordinal;
- an observation names the exact authorization and immutable request;
- `SupersededBeforeEntry` appears only for an Effect whose certified completion and prepared
  access declare that generic refresh contract, and its evidence matches the admitted stable
  lineage and qualified public binding certificate;
- settlement consumes the exact compatible committed observation;
- a state success follows its structural success continuation;
- a state or fragment failure follows its sealed `FailurePlan`;
- a normally completing `Handled` pre-handler path can exit only by entering its exact designated
  handler, and the handler's retained closed tag selects exactly one arm of its exhaustive
  certified continuation;
- a `Propagate` path can exit only through its exact affine enclosing fragment boundary, preserves
  the lexical-region token and source-to-boundary failure provenance, and belongs to a finite chain
  ending in exactly one `Handled` call-site plan;
- a new failure from a failure-post state follows that post-state's own `FailurePlan`;
- every failure-path input names the exact failure-producing occurrence and contract;
- `Match` choice matches the retained canonical closed-sum tag;
- inactive-arm and non-dominating values are unusable;
- fan-out lanes are declared, unique, effect-free, and joined only when complete;
- fan-out output order is declaration order;
- outputs, failures, and facts use the exact occurrence and content contract;
- `Return` or `Fail` is reached before closure; and
- closure is inseparable from either the admission append for an initially terminal program or the
  final semantic transition.

Runtime supplies sealed callback results without caller-authored producer identities. The store
attaches the authoritative producer references and constructs input manifests, transition bodies,
binding deltas, digests, and optional closure. Persisted bytes remain hostile even when Rust
typestate made invalid construction difficult.

### Replay and projections

Recorded replay uses the same callback-free fold and proves what the run admitted, authorized,
observed, and committed. It invokes no adapter, transport, provider, signer, resource authority,
or mutable store path.

Trace, audit, export, CLI, and REST may expose reviewed identifiers, structural paths, state/access
kinds, secret-free physical-binding references, typed public failures, redaction-safe fault codes, and
whether a run is waiting on possible-entry ambiguity.

They never expose provider text, response bodies, endpoint URLs, credentials, raw SQL errors,
filesystem paths, signed payloads, or secret material.

### Prior-run fact selection is an ordinary read

A state that consumes prior-run facts declares an ordinary `Read` request containing its admitted
source manifest, selector contract, completeness mode, and bounds. The registered invoker is the
sealed purpose-limited RunHistory fact scanner rather than a general database adapter. It returns
one canonical typed response through the normal access bracket; it receives no append or generic
query authority.

Certification freezes the selector and source contracts. The scanner proves bounded completeness
against the admitted immutable source heads, and the run-history fold validates the resulting
observation and producer references generically. No `FactSelection` execution kind, alternate
history protocol, or ambient state callback is introduced.

## Expansion For Control, Security, And Telemetry

### Semantic injection

Expansion is the supported mechanism for reusable semantic composition:

- authorization and policy guards;
- provenance and configuration validation;
- nonce observation and reservation;
- reconciliation after a definite state-consumable result;
- deterministic transformations;
- success and domain-failure postconditions;
- compensation branches;
- security checks whose result changes whether work may proceed; and
- durable audit facts.

An injected IO operation is an ordinary `Read` or `Effect`. It receives authorization,
observation, typed failure handling, append-only history, and the same crash semantics as any
authored state.

A security pre-state cannot replace target-side authorization or cross-run resource fencing. A
post-state cannot undo an effect that already entered. No post-state executes while the protected
effect remains possible-entry ambiguous.

### Operational observation

Best-effort logs, metrics, and spans should observe redacted Runtime or committed-history events
outside semantic execution.

Making a logging sink an injected `Effect` would make collector availability part of run
correctness, add authorization and history volume, and create its own ambiguity and failure
handling. That representation is valid only when acknowledgement by the sink is genuinely part of
the operation's required semantics.

Authoritative audit data belongs in state facts, access records, and committed history. Operational
telemetry may be missing or duplicated and must never influence cursor derivation.

## Wallet Nonce Resource Authority

### Ownership and namespace

EVM sender nonces are shared by every actor capable of submitting for the same physical sender.
Declaration order within one run cannot serialize two runs.

The wallet-nonce authority owns one canonical physical namespace:

```text
WalletNonceDomain =
    canonical physical chain lineage
  + sender identity
```

Tenant, wallet alias, route, and signer generation remain authorization and provenance qualifiers,
but they cannot partition the uniqueness namespace unless qualification proves a one-to-one
physical identity.

The writer credential is a monotonic private fencing capability over the same durable resource
lineage. Rotation, restore, and promotion carry every reservation, completion, and high-water mark
forward. The private credential is assembly-only and is not part of semantic reservation identity,
request bytes, or history.

### Operations

The narrow authority exposes idempotent operations conceptually equivalent to:

```text
reserve(
    wallet_nonce_domain,
    assembly_private_current_writer_credential,
    semantic_reservation_key,
    transaction_intent_digest,
    observed_pending_floor,
) -> ReservedWalletNonce
   | SupersededBeforeEntry(PublicLineageHeadRef)
   | SafeFailure
   | EntryUnknown
   | IntegrityFault

complete(
    wallet_nonce_domain,
    assembly_private_current_writer_credential,
    semantic_completion_key,
    provenance_verified_reservation,
    provenance_verified_terminal_evidence,
) -> CompletedWalletNonce
   | SupersededBeforeEntry(PublicLineageHeadRef)
   | SafeFailure
   | EntryUnknown
   | IntegrityFault
```

Both are Runtime-authorized `Effect` accesses. Internal `Applied` versus `ExistingSame` status is
not a semantic output.

For each operation, the serialized resource transaction:

1. resolves an existing permanent semantic operation key first;
2. acquires the domain's serializing lock and re-resolves that key to close the concurrent-insert
   race;
3. rejects a conflicting key, domain, intent, or evidence as integrity failure;
4. validates the current non-rollback private writer credential;
5. returns `SupersededBeforeEntry(public_lineage_head_ref)` if the caller is stale;
6. otherwise performs the mutation and records the operation result atomically; and
7. commits one byte-identical proof for future exact resolution.

This ordering makes all rotation races explicit:

- mutation before rotation resolves to the original result;
- rotation before mutation proves that the protected mutation was not applied;
- lost acknowledgement resolves by permanent operation key; and
- a late old request is rejected before applying the protected mutation or resolves
  `ExistingSame`.

`ExistingSame` is an internal resource-transaction resolution during the already authorized
invoker; it never mints another Runtime authorization.

`SupersededBeforeEntry` is not a state failure or `ResourceHistoryInvalid`. Runtime records the
protected-non-application outcome. The store leaves the semantic cursor at the resource state and
folds to `Refreshable(next_attempt_ordinal, public_lineage_head_ref)`. A separately assembled
current worker may authorize the same semantic request with its current physical binding and the
new attempt identity. The stale worker receives no current credential and cannot retry itself.

### Pending nonce observation and allocation

Every new reservation consumes one fresh:

```text
eth_getTransactionCount(sender, "pending")
```

observation from an injected EVM `Read` state. The same pending RPC is used for first allocation
and for every later local-next check. Its registered invoker strictly decodes the bounded JSON-RPC
quantity; the `Read` settlement and an injected EVM `Pure` state validate the exact chain, sender,
route-generation, and observation provenance, then bind the result to the admitted pending-floor
policy. The RPC call and pure qualification happen before the resource transaction; no database
lock is held across network IO.

For domain `D`, EVM-derived semantic reservation key `K`, intent `I`, and qualified pending value
`Pq`, the resource transaction performs:

```text
resolve existing (D, K) first:
    same intent and domain -> return original proof
    conflict               -> integrity failure

lock and fence D
re-resolve (D, K) under the lock:
    same intent and domain -> return original proof
    conflict               -> integrity failure

if local_high_water is absent:
    require a virgin retained lineage
    candidate = Pq
otherwise:
    local_next = checked_add(local_high_water, 1)
    candidate =
        local_next   if Pq <= local_next
        Pq           if Pq > local_next
                     and the exact admitted provider-ahead policy accepts the jump
        no mutation  otherwise; return the policy's reviewed typed disposition

insert reservation(
    D,
    K,
    I,
    candidate,
    qualified_pending_observation_ref,
    pending_floor_policy_ref,
)
update local_high_water = candidate
commit
```

Two runs may observe the same `Pq`; the resource transaction serializes them to distinct
monotonic nonces. A lagging provider cannot move local state backward.

An absent `local_high_water` is legal only for a virgin retained lineage with no reservation or
completion records. Any disagreement between the high-water mark and retained resource history is
an integrity fault.

The implementation must reject overflow and policy mismatch and retain the exact qualified
chain/sender/route, observation, and policy references. A provider-ahead jump is never adopted
merely because one provider returned it. The exact bounded acceptance, disagreement, and
integrity-review rules remain a material choice below; until selected, such a jump has no
certifiable accepting policy. The provider is evidence for the pending floor, not allocation
authority.

Every sender-capable signer, relayer, operator path, stale deployment, and direct-submit path must
use this authority or be permanently fenced out. A pending provider read cannot close a race with
an uncoordinated actor.

### Typed provenance without EVM-aware Runtime

The EVM domain owns privately constructible types conceptually equivalent to:

```text
ObservedPendingNonceFloor {
    nonce_domain,
    route_generation_ref,
    pending_nonce,
    observation_ref,
}

QualifiedPendingNonceFloor {
    observed: ProducerBound<ObservedPendingNonceFloor>,
    pending_floor_policy_ref,
}

EvmNonceReservationKey {
    semantic_reservation_key,
    derivation_contract_ref,
}

EvmNonceCompletionKey {
    semantic_completion_key,
    derivation_contract_ref,
}

ReserveEvmNonceRequest {
    nonce_domain,
    transaction_intent_digest,
    reservation_key: ProducerBound<EvmNonceReservationKey>,
    qualified_floor: ProducerBound<QualifiedPendingNonceFloor>,
}

ReservedWalletNonce {
    nonce_domain,
    nonce,
    semantic_reservation_key,
    intent_digest,
    observed_floor_ref,
    resource_lineage_ref,
    reservation_evidence_ref,
}

CompleteEvmNonceRequest {
    nonce_domain,
    completion_key: ProducerBound<EvmNonceCompletionKey>,
    reservation: ProducerBound<ReservedWalletNonce>,
    terminal_evidence: ProducerBound<TerminalEvidence>,
}

CompletedWalletNonce {
    nonce_domain,
    nonce,
    semantic_reservation_key,
    semantic_completion_key,
    reservation_evidence_ref,
    terminal_evidence_ref,
    completion_evidence_ref,
}
```

`DeriveEvmNonceReservationKey`, an injected EVM `Pure` state, derives:

```text
semantic_reservation_key =
    domain_separated_hash(
        "mfm.evm.nonce-reservation.v1",
        run_id,
        protected_submission_semantic_call_id,
        reservation_derivation_contract_ref,
        nonce_domain,
        transaction_intent_digest,
    )
```

The key is stable across physical attempt ordinals, exact re-resolution, and physical-binding
refresh for that semantic occurrence. A new invocation's different `run_id` produces a different
key; any future cross-run transfer or reuse requires the separate explicit policy recorded under
Material Uncertainties. Runtime and the run-history store carry the key and producer references as
opaque typed material and never derive them or inspect their EVM fields. The derivation state's
inputs are exact admitted or producer-bound values, including `run_id`; it uses no ambient data.

`DeriveEvmNonceCompletionKey`, also an injected EVM `Pure` state, derives:

```text
semantic_completion_key =
    domain_separated_hash(
        "mfm.evm.nonce-completion.v1",
        run_id,
        protected_submission_semantic_call_id,
        completion_derivation_contract_ref,
        nonce_domain,
        semantic_reservation_key,
        reservation_evidence_ref,
    )
```

The completion key names exactly one permanent completion operation for the reservation. The
terminal evidence remains part of the completion request and committed result, not the key:
repeating byte-identical evidence resolves the original proof, while different evidence under the
same key is an integrity conflict rather than a second completion. The completion key has the same
physical-attempt and binding-refresh stability as the reservation key.

EVM states validate field-level agreement. The wallet-nonce adapter validates its resource
request and durable proof. Runtime and the run-history store validate only generic program,
contract, occurrence, and producer-bound provenance.

Rust nominal types alone cannot prove chain/sender equality after deserialization. Private
constructors, exact certified producers, canonical bytes, store validation, and adapter-side
domain checks form the complete boundary.

### Completion and abandoned reservations

Completion binds exact terminal evidence to the reservation and is permanent and idempotent. It
first resolves and re-resolves the exact semantic completion key under the same serialized
resource transaction rules as reservation. It does not release or recycle the nonce.

The earlier run's failed history status never blocks admission or execution of a new run. The
resource authority may reserve later monotonic nonces, subject to its independent invariants. A
reservation that definitely never reached broadcast can nevertheless create an EVM nonce gap;
cross-run reuse, transfer, or gap-fill policy is a separate material design choice recorded below.
Timeouts never release a nonce.

## EVM Without An Executor

### Registered structured expansion

An authored EVM submission call selects an exact semantic expansion contract. The expander
substitutes a structured fragment at that declaration slot, for example:

```text
ObservePendingNonce            [Read]
QualifyPendingNonceFloor       [Pure]
DeriveEvmNonceReservationKey   [Pure]
ReserveWalletNonce             [Effect]
BuildUnsignedCandidate         [Pure]
BroadcastExactCandidate        [Effect]
ObserveTransaction             [Read]
ObserveReceipt                 [Read]
ObserveFinalizedHead           [Read]
VerifyCanonicalInclusion       [Pure]
DeriveEvmNonceCompletionKey    [Pure]
CompleteWalletNonce            [Effect]
ProjectTransactionResult       [Pure]
```

The exact program may use exhaustive branches for definite rejection, revert, reorganization, or
a bounded replacement policy. Every changed nonce, fee, route, candidate, or semantic request is a
new explicit occurrence.

The expansion fragment owns failure handling for its injected states and preserves the authored
call's external output/failure boundary. Runtime and store see only ordinary certified states and
structured control.

Expansion is selected by the exact semantic EVM operation and capability requirement, not merely
because a live implementation happens to use an EVM JSON-RPC transport.

### Signing and submission

`BroadcastExactCandidate` is one authorized bounded effect. Its invoker passes the exact unsigned
candidate to the qualified signer once, verifies the derived transaction identity, submits those
exact bearer bytes once, discards them, and returns only secret-free proof such as:

```text
SubmittedCandidateProof {
    unsigned_candidate_digest,
    transaction_hash,
    signer_generation_ref,
    signing_contract_ref,
    submission_contract_ref,
}
```

No signature or raw signed transaction enters retained state. There is no earlier
`PrepareSignedCandidate` effect and no requirement to reproduce a signature or signed envelope
later.

The adapter cannot choose another nonce, alter the candidate, replace fees, rotate semantic
routes, poll, rebroadcast in a loop, or decide terminal run meaning.

Process loss after broadcast authorization and before a committed observation leaves the
submission occurrence possible-entry ambiguous. The initial design does not automatically
rebroadcast. A future same-occurrence reconciliation extension must introduce or derive a stable
transaction identity from pre-entry retained material, or use an authoritative idempotent wallet;
it cannot assume that the transaction hash returned after submission survived process loss. A
later program state is not reachable while the current occurrence remains ambiguous, and no
hidden adapter loop is permitted.

### Reads and terminal meaning

Transaction lookup, receipt lookup, and head observation are explicit `Read` states.
`VerifyCanonicalInclusion` is a `Pure` state over their exact committed values; any additional
network observation is a preceding explicit `Read`. A bounded “not yet available” result is a
reviewed safe failure or closed output that feeds an explicit operation branch. Repeated polling
is expressed as a finite expansion of distinct read occurrences.

`CompleteWalletNonce` consumes the exact producer-bound completion key, reservation, and
terminal-evidence references. It does not construct terminal evidence or decide terminality.

`ProjectTransactionResult` and the operation terminal contract require the matching
`CompletedWalletNonce`. No branch may close as success or revert after terminal transaction
evidence but before exact resource completion.

## Store And PostgreSQL Boundary

### RunHistory is an adapter and authority

Runtime depends on a narrow run-history port. Its implementation may use PostgreSQL, but it owns:

- append-only record and object persistence;
- per-run exact-head compare-and-append;
- logical-key idempotency;
- acknowledgement-ambiguity resolution;
- atomic object, fact, binding, transition, and closure visibility;
- writer generation and non-rollback lineage;
- callback-free structural and semantic fold validation; and
- purpose-specific read views.

These responsibilities do not belong to SQL, `sqlx`, or Runtime.

Production assembly exposes separately scoped capabilities:

```text
qualified infrastructure
  -> run-history database role/scope
       -> RunHistoryWriter   // consumed only by Runtime
       -> purpose-limited readers
       -> immutable physical-binding certificate verifier
  -> wallet-nonce database role/scope
       -> WalletNonceAdapter // reachable only by its registered Runtime invoker
```

The certificate verifier can prove only public binding membership and lineage; it cannot sign,
invoke, or mutate a resource. The run-history role cannot mutate nonce authority, and the nonce
role cannot mutate run history. No generic owner, application pool, or query capability survives
assembly.

The two authorities may share one physical PostgreSQL deployment only when qualification proves
non-rollback lineage, stale/sibling-writer exclusion, backup, restore, and promotion for both.
Physical co-location does not merge schemas, roles, fences, or semantic algebras.

### Shared physical primitive, separate semantic authorities

Run history and nonce reservations may share lower-level immutable objects, transactions, or a
generic atomic compare-and-append primitive when that reduces code.

They retain separate semantic validators:

- the run-history store validates the structured five-record run algebra; and
- the wallet-nonce authority validates its resource algebra.

A universal tagged history engine with callbacks, planners, policy hooks, or workflow scheduling
would recreate the executor and is rejected.

### Resource mutation remains causally visible

The nonce transaction is not a `StateTransitionCommitted` record, but the interaction remains
causally visible:

```text
ExternalAccessAuthorized(Effect, ReserveWalletNonce)
  -> atomic resource operation
  -> ExternalAccessObserved
       Returned(reservation) | SafeFailure
         -> StateTransitionCommitted(consumes exact state-consumable observation)
       SupersededBeforeEntry(public_lineage_head_ref)
         -> Refreshable(next_attempt_ordinal)  // no semantic transition
       EntryUnknown | IntegrityFault
         -> parked or blocked                 // no semantic transition
```

Only the resource authority decides cross-run uniqueness. Only Runtime records why one run invoked
it and what completion that run observed.

## Failure And Crash Semantics

| Boundary | Durable run fact | Required behavior |
| --- | --- | --- |
| Expansion or preparation fails | Prior verified history only | No authorization and no live entry. |
| Authorization append is rejected or stale | No new authorization | Reload the verified cursor; do not invoke. |
| Authorization append acknowledgement is ambiguous | Authorization may exist | Resolve the original append identity; mint no authority from ambiguity. |
| Authorization positively commits | `ExternalAccessAuthorized` | Mint one affine authority for the exact operation. |
| Process dies before or during read invocation | Unmatched authorization | Preserve the exact waiting cursor; the read-recovery rule is unresolved, so do not invent an authorization or successor. |
| Process dies before or during effect invocation | Unmatched `Authorized<Effect>` | Keep that fold state and report possible-entry ambiguity; do not authorize another effect attempt. |
| Registered invoker returns | Authorization plus pending completion in memory | Totalize immediately and commit one linked observation before normal return. |
| Observation loses an exact-head race | Authorization plus stable pending material | Rebase append without reinvoking. |
| Observation acknowledgement is ambiguous | Observation may exist | Resolve the unchanged original append before rebase. |
| Journal is unavailable after invoker return | Pending material remains in live task | No normal successful drive result. |
| Process dies after invoker return but before observation commit | Unmatched authorization; pending material lost | Keep `Authorized<K>`; report `ReadCompletionUnknown` for a read or possible-entry ambiguity for an effect. |
| `Returned` or `SafeFailure` observation commits | Linked `ExternalAccessObserved` | Reload and run the pure settlement callback. |
| State commits typed failure | Failed transition and producer-bound failure | Enter only its exact structural failure continuation; do not erase the failure. |
| Default handler commits | Handler transition and scope failure | In an operation scope, atomically follow `Fail`/closure when that is the normalized successor; in a lane, yield the typed lane failure. |
| Custom handler selects recovery | Handler transition and closed route | Execute only the declared recovery branch. |
| Definite ordinary error closes run | `RunClosed(Failure)` | A new invocation may create and execute another run independently. |
| Integrity evidence commits or is detected | Audit evidence or rejected candidate | Block; never construct domain failure. |
| Resource binding is stale | `SupersededBeforeEntry(public_lineage_head_ref)` observation | Keep the same state current; fold to the next `Refreshable` ordinal. Only a current qualified worker may reauthorize. |
| Resource acknowledgement is ambiguous while task lives | Run authorization; resource operation may exist | Resolve the permanent resource key internally. |
| Process dies after a resource result exists but before run observation | Unmatched run authorization plus durable resource proof | Keep that run parked. Recovery of the durable proof requires a separately designed same-occurrence reconciliation contract. |
| Committed observation exists but process dies before settlement | Exact observation in history | Recompute settlement without live IO. |
| Fan-out lanes complete in different physical orders | Ordered lane histories | Join results in declaration order. |
| Current effect is possible-entry ambiguous | Open run at exact effect cursor | Keep it parked with no legal successor in this RFC; do not advance or duplicate. |
| Final transition reaches `Return` or `Fail` | Terminal transition | Append `RunClosed` atomically; accept no later record. |
| Initial normalization reaches `Return` or `Fail` | No state transition is needed | Atomically append `RunAdmitted` and `RunClosed`. |

## Enforcement

The cutover must make these properties structural:

- ordered builder handles prevent forward references and invalid branch/lane escapes;
- sealed block policies prevent lane fragments, matches, failure posts, and recovery routes from
  widening `Pure | Read` or constructing nested fan-out;
- expansion receives an affine protected slot and cannot duplicate an effect;
- expansion dependencies, depth, occurrences, branches, and fan-out are bounded;
- final certification contains no unresolved calls, wrappers, or unhandled fallible boundaries;
- `RunHistoryWriter` is non-cloneable and consumed by Runtime assembly;
- live adapters cannot depend on or construct run-history mutation authority;
- raw PostgreSQL mutation capability remains private to qualified storage assembly;
- `Prepared<K>`, `Authorized<K>`, `PendingObservation<K>`, and
  `CommittedObservation<K>` have private fields and sealed constructors;
- only a positively new authorization append creates `Authorized<K>`;
- the fold alone derives attempt ordinals and only `Refreshable` permits a next attempt;
- private writer credentials never enter programs, requests, observations, or history;
- Runtime dispatch is the only path from authorization to a registered invoker;
- invokers have no outer normal error after accepting authority;
- after authorization, settlement consumes only an exact `CommittedObservation<K>`;
- at most one unresolved authorization exists for an occurrence, and an unresolved
  possible-entry effect can never be bypassed;
- certified failure plans consume only exact producer-bound typed state failures, admit only exact
  handler entry or affine fragment-boundary propagation, prove one eventual handler, and lower
  handlers to ordinary `Pure` state bindings;
- effects cannot occur transitively inside fan-out;
- branch selection is derived, not caller-authored;
- fan-out results are joined in declaration order;
- Runtime and store interpret no EVM, nonce, wrapper, handler-origin, or telemetry-specific
  semantics; they follow only ordinary certified structure and generic provenance;
- all nonce-reserving workflows use the same qualified resource authority;
- all sender-capable actors use that authority or are fenced out;
- run-history and nonce database roles cannot cross-write;
- adapters contain no history fold, next-plan selector, retry loop, or terminalizer; and
- replay, telemetry, and applications receive read-only purpose capabilities.

Architecture scans supplement but do not replace type privacy, crate dependency contracts,
compile-fail tests, hostile-history tests, backend conformance, and fault injection.

## Why Not Encode Every Audit Step As A State?

Authorization and observation surround external IO:

```text
authorization committed
  -> external operation
  -> observation committed
```

Renaming them semantic state transitions does not make them atomic or guarantee the second append.
It would put worker timing and crash mechanics into domain state.

Meaningful policy, validation, and domain work should be small injected states. Runtime mechanics
remain private typed protocol phases.

## Why Not Keep An Arbitrary DAG?

The current product needs:

- declaration-ordered sequencing;
- exhaustive conditional branches;
- bounded pure/read fan-out; and
- typed joins at structured boundaries.

An arbitrary DAG additionally requires cycle/reachability validation, alternative-source
materialization, global readiness, dependency skips, required-success sets, all-nodes-terminal
closure, conflict analysis, and fairness rules.

The current repository inventory has not identified a production operation that requires
overlapping joins, arbitrary producer alternatives, or unstructured acyclic sharing; that claim
must be verified before implementation. If a future use case does, it should first prove that the
structured algebra cannot express the required semantics. It must not reintroduce a graph merely
as an authoring convenience.

## Why Not Keep A Generic Executor?

A separate durable coordinator may be justified for a product requiring:

- arbitrary non-idempotent and non-queryable destinations;
- progress independent of a run being driven;
- cross-run coalescing of one logical delivery;
- independently operated delivery infrastructure; or
- open-ended scheduling.

Those are not current MFM requirements. Keeping a generic executor imposes a second interpreter,
history, fence, recovery algebra, and type system on every effect.

A future product needing those properties should integrate an explicit external authority or
propose a new boundary. It must not grow an adapter into another hidden Runtime.

## Why Not A Universal Outbox?

A universal outbox near every provider could preserve stronger forensics across process loss. It
would also require another durable writer or destination participant, fencing, ambiguous-commit
recovery, broader retention, secret review, and another availability dependency.

The baseline records authorization before entry and every normal completion afterward. Process
loss leaves honest possible-entry ambiguity. The initial design parks an ambiguous effect rather
than automatically re-entering it.

## Persisted Contract And Cutover Boundary

This target changes persisted semantics:

- the authored and certified graph become structured authored/expanded/certified programs;
- declaration order becomes semantic and hashed as order;
- node IDs become structured occurrence identities;
- arbitrary input bindings become lexical producer references and structured joins;
- `DependencySkipped`, blocking-source evidence, and its batch purpose disappear;
- required-success nodes and all-nodes-terminal closure disappear;
- node phase maps become a structured cursor and fan-out lane states;
- branch choice becomes derived from kernel-owned canonical closed-sum tags;
- state failure becomes a first-class producer-bound value for exact handlers;
- wrapper and failure-handler provenance enters the admitted expanded program;
- authored child calls lower completely into lexical structured blocks;
- physical access-attempt ordinals and refresh typestate become generic history state;
- executor `ensure` becomes direct `Effect` access;
- executor-retained closure and histories disappear;
- effect ambiguity no longer permits automatic overlapping re-entry;
- stale resource binding becomes protected-non-application evidence and a fold-derived
  `Refreshable` attempt;
- nonce reservation always consumes a fresh pending provider observation; and
- EVM topology becomes registered structured expansion.

The implementation therefore requires one new sole current schema lineage, annex, corpus,
store/replay fold, trace/export projection, and public contract. Existing bytes are rejected; they
are never reinterpreted.

No dual reader, dual writer, graph lowering compatibility path, fallback executor, or parallel
direct-effect path is part of this RFC.

## Design Cutover Scope

The completed cutover must:

- replace authored/expanded graph public types and builders with the structured program algebra;
- replace arbitrary node bindings with lexical typed handles, branch merges, and fan-out joins;
- rewrite pure operation, child-operation, framework, and capability expansion as typed structured
  substitution;
- bind the exact required expansion profile and coverage policy at entry-point admission;
- add exact expansion profile ordering, bounds, provenance, and affine protected slots;
- add lexical operation/lane default failure-handler state contracts and producer-bound failure
  values;
- rewrite certification for lexical dominance, exhaustive branches, handler coverage, fan-out
  restrictions, capability closure, and terminal totality;
- replace graph readiness and scheduling with cursor interpretation;
- replace graph-shaped store/replay folding with the structured callback-free fold;
- delete dependency skips, alternative-source selection, cycle/reachability proofs,
  required-success sets, authorization-count scheduling, and graph-wide phase maps;
- delete `crates/kernel/executor`;
- delete executor storage crates, schemas, manifests, ledgers, frontiers, target authorities,
  tombstones, fences, and tests;
- remove EVM wallet history folding and next-plan selection from the live layer;
- add direct typed effect access under Runtime's private bracket;
- add registered EVM structured expansions and the narrow wallet-nonce authority;
- keep raw PostgreSQL pools private and expose separately scoped qualified authorities;
- update `docs/design.md`, `docs/architecture.md`, run-execution documentation, persisted-surface
  inventories, qualification docs, app wiring, CLI/REST projections, and relevant READMEs;
- reset the sole current persisted contract deliberately; and
- delete all superseded dependencies, tasks, fixtures, migrations, tests, and terminology.

Nothing in this scope authorizes implementation before the material uncertainties below are
resolved.

## Verification Required Before Implementation Planning

The design must first be validated with small models or tests, not a parallel production path:

- compile every current production operation into the structured algebra;
- prove no current operation requires arbitrary DAG-only behavior;
- model one success/failure branch, one recovery branch, and one nested child-operation failure;
- prove child `Return`/`Fail` lower to call-site success/failure without closing the parent;
- prove byte-identical expansion under registry and map iteration variation;
- prove exact wrapper nesting, affine core use, expansion termination, and hard bounds;
- prove all injected fallible states and fragment boundaries receive exactly one handler;
- reject a live or fallible default handler;
- reject a direct scope exit before the designated handler, a forged affine propagation target,
  and a propagation chain with zero or multiple eventual handlers;
- reject forward references, inactive-arm escape, cross-lane values, incompatible merges,
  effects in fan-out, terminal lane instructions, and unbounded fan-out;
- reject an effect or nested fan-out hidden in a lane fragment, `Match` arm, failure-post path, or
  custom recovery route;
- prove collect-all fan-out under every physical completion-order permutation;
- prove store rejection of skipped pre/post/handler occurrences and forged branch selection;
- prove store rejection of authorization for future instructions, wrong lanes, wrong bindings,
  wrong attempt ordinals, and any second unresolved same-occurrence access;
- crash at every authorization, invocation, observation, settlement, handler, branch, fan-out, and
  closure boundary;
- prove a definite typed negative response and a definite safe failure each reach the default
  handler, close the run, and do not block a new run;
- prove possible-entry effect ambiguity cannot mint another authorization;
- fault-inject resource rotation versus mutation and prove `SupersededBeforeEntry` or exact
  internal `ExistingSame`;
- prove only `SupersededBeforeEntry` creates `Refreshable`, exactly one next ordinal is possible,
  stale workers cannot self-upgrade, and private writer credentials never persist;
- prove initially terminal programs atomically append `RunAdmitted` and `RunClosed`;
- crash an unmatched read inside and outside fan-out and validate the selected read-recovery
  policy;
- model concurrent runs observing the same pending nonce and prove unique monotonic reservations;
- prove first-use and later reservation use the fresh pending-floor algorithm;
- prove the EVM reservation and completion keys are stable across exact re-resolution and physical
  refresh, change with a new `run_id`, and cannot be caller-substituted;
- prove byte-identical nonce completion resolves one permanent result and conflicting reservation
  or terminal evidence under that completion key is rejected;
- test lagging, equal, and provider-ahead pending observations under the selected qualified policy,
  including rejection without mutation;
- reject nonce-domain, intent, observation, reservation, terminal-evidence, and producer
  substitution;
- prove non-rollback resource lineage and stale/sibling-writer exclusion across restore,
  promotion, and generation rotation;
- inventory every signer, relayer, operator, stale deployment, and direct-submit path;
- prove run-history and nonce roles cannot cross-write and no generic pool survives assembly;
- prove best-effort telemetry failure cannot affect a run;
- prove purpose-limited prior-run fact reads are complete against their admitted source heads and
  cannot obtain generic query or append authority;
- prove `BroadcastExactCandidate` signs once, submits the exact derived envelope once, and retains
  no bearer bytes;
- use canary credentials/provider text to prove no secret-bearing value reaches programs,
  histories, objects, errors, traces, exports, or logs;
- inventory every current executor predicate and assign it to expansion, certification, Runtime,
  store, resource authority, explicit state, or deletion; and
- show that no required predicate remains ownerless.

Only after these checks pass should the repository define the implementation's logical commit
sequence and scope-driven verification plan.

## Material Uncertainties

| Choice or assumption | Why uncertain | Consequence if wrong | Resolution or validation |
| --- | --- | --- | --- |
| Every expected definite state-facing operational disposition can be represented by a reviewed typed response or redaction-safe capability `SafeFailure` and interpreted deterministically; cases expected to fail or recover map to typed state failure. | Current fault vocabulary contains audit-only operational outcomes. | An expected ordinary error could still park a run, or unsafe diagnostics could be laundered into domain truth. | Inventory every capability completion and freeze its response, safe-failure, physical-control, possible-entry, and integrity classification before schema work. |
| Each operation or lane failure contract can receive every lexical default-handler mapping, including failures from child operations and injected states. | Reusable states, lanes, and nested operations have heterogeneous failure types. | Expansion may need extra wrapper sums or duplicate mapping states. | Model one nested child operation, one lane failure, one framework precondition, and one EVM injected-state failure end to end before freezing the handler API. |
| The frozen expansion phase order covers every required security policy without recursive application. | Later policies may need to protect states injected by earlier policies. | Support states may be uncovered, or expansion may become cyclic and surprising. | Define the exact phase/eligibility matrix and certify coverage markers for representative nested policies. |
| Planning-fixed, collect-all `Pure`/`Read` fan-out covers current latency and failure requirements. | Nested fan-out, runtime-discovered cardinality, partial results, or fail-fast behavior may be desired. | The cursor and late-observation contracts would need material expansion. | Compile and exercise the largest current portfolio operation, including one lane failure, before freezing the IR. |
| The legal successor, if any, for an unmatched `Read` authorization has not been selected. | A read cannot mutate its target, so a certified abandonment/retry-selection rule may safely improve liveness, especially inside collect-all fan-out; however, a new read can return a different time-varying value. | A process crash can leave a read-heavy run or fan-out permanently open, while a hasty retry rule can make replayed selection nondeterministic. | Choose and model one explicit read policy before implementation: permanent wait, a qualified attempt-supersession record, or a typed run-level interruption. Do not reuse effect `EntryUnknown` and do not invent ad hoc reauthorization. |
| The RFC's safety baseline leaves an unmatched or returned possible-entry effect at the exact current occurrence with no legal successor; the required deployment-time same-occurrence reconciliation contract, if any, is not selected. | Declaration order and typestate prevent overlap but cannot prove that a remote target was not entered after process loss. | Runs may remain open indefinitely, or implementation pressure may recreate overlapping attempts. | Inventory each effect's outcome-query or target-idempotency capability and freeze an explicit certified same-occurrence reconciliation/operator protocol where required. Do not authorize automatic re-entry without target-enforced proof. |
| The structured algebra covers every production operation without DAG-only sharing. | The current inventory found no counterexample, but public or less-traveled consumers may rely on alternative producers, overlapping joins, or acyclic sharing. | The complete cutover could discover an operation that cannot be expressed without changing the IR. | Compile every production and public-library operation into the structured model and record any rejected graph shape before implementation planning. |
| Prior-run fact selection can preserve its completeness contract as an ordinary `Read` through the sealed RunHistory scanner. | Existing fact selection has store-specific source and completeness semantics. | Lowering it casually could lose completeness, create a hidden access kind, or expose generic history query authority. | Model one bounded selector against frozen source heads and prove identical live/recorded results plus capability isolation. |
| The admitted policy for a qualified pending nonce above `local_high_water + 1` is not selected. | A jump may reflect legitimate external use, provider disagreement, rollback, wrong-chain binding, or a malicious/faulty provider. | Automatic adoption can create permanent gaps or exhaust the wallet; unconditional rejection can block a wallet after legitimate external use. | Define qualified provider selection, strict width/bounds, disagreement handling, and a bounded allowed-jump or integrity-review policy. Until then, provider-ahead input has no certifiable accepting policy. |
| A failed run's unbroadcast reservation may remain permanently allocated while later runs reserve higher nonces. | A later Ethereum transaction cannot mine across an unfilled lower nonce. | Run-history liveness would improve while the wallet remains operationally wedged by a nonce gap. | Choose a cross-run semantic intent key, proven-never-exposed reservation transfer/reuse, or explicit gap-fill/cancellation operation. Never use timeout reuse. |
| The wallet-nonce lineage remains non-rollback and exclusive across writer rotation, restore, promotion, and every sender-capable actor. | The guarantee spans database operations, deployment procedures, signers, relayers, and direct-submit paths. | A stale writer or out-of-band actor could reuse a nonce despite correct per-run history. | Produce an operational proof and failure-injection suite covering scoped roles, lineage transfer, and complete sender-path inventory. |
| Operational telemetry can remain non-semantic while required audit evidence is fully derivable from history and facts. | Teams may expect best-effort logging to be both durable and invisible to run outcomes. | Observability availability may accidentally become business correctness, or required evidence may be lost. | Classify every proposed hook as semantic state/fact/effect or non-authoritative observer before adding expansion policies. |
| Existing graph/executor histories and resource ledgers can be drained, exported, or reset outside the new production reader. | Repository policy permits a breaking cutover, but deployed state is not established here. | Removing old readers could strand active effects or operational evidence. | Inventory deployed environments and choose an explicit drain/export/reset procedure; add no compatibility execution path. |
