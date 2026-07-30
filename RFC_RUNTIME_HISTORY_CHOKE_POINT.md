# RFC: Runtime History Choke Point

Status: proposed target architecture; implementation has not started

Scope: state execution kinds, run-history ownership, external-access recording, effect recovery,
cross-run resource authority, EVM nonce ownership, adapters, transports, stores, replay, and the
deletion of the generic executor

Compatibility: this is a breaking target design. The current code, `docs/design.md`, and
`docs/architecture.md` continue to describe the executor-based implementation until one complete
cutover deliberately replaces that contract. This RFC defines no compatibility reader, dual
writer, alternate effect path, or implementation sequence.

## Executive Decision

MFM will have one interpreter code path and authority-owning type for a run: `Runtime`.

This is not a deployment singleton. Multiple processes may independently assemble Runtime workers
against the same qualified authoritative lineage. Each worker receives its own non-cloneable
writer capability; store fencing and exact-head compare-and-append serialize their durable
actions. “One interpreter” means no scheduler, adapter, executor, or application implements a
second interpretation path.

Certified states retain three semantic execution kinds:

```text
Pure
Read
Effect
```

`Effect` is not removed or renamed to `Read`. It means that executing the state may mutate an
external system or consume an exclusive external capability. The distinction is required for
auditing, qualification, future concurrency analysis, and safe recovery.

`Read` and `Effect` use the same private Runtime-owned access bracket:

```text
Prepared<K>
  -> ExternalAccessAuthorized committed
  -> Authorized<K>
  -> exactly one adapter invocation
  -> PendingObservation<K>
  -> ExternalAccessObserved committed
  -> CommittedObservation<K>
  -> StateTransitionCommitted
```

where `K` is `Read`, `Effect`, or the reserved `FactSelection` access kind.

The shared mechanism does not erase semantics. The certified state kind, authorization, adapter
registration, request contract, observation contract, and consumed transition all continue to say
whether an operation was a read or an effect.

The generic executor is removed. It must not remain as a second durable interpreter, a wallet
driver, a retry engine, or an adapter with a renamed lifecycle. Meaningful EVM progression becomes
an explicit certified graph of ordinary `Pure`, `Read`, and `Effect` states.

Cross-run resource uniqueness cannot be moved into one run's `Runtime`. A locally controlled EVM
sender therefore uses one narrow durable nonce-reservation adapter. It owns only reservation and
completion invariants. It does not fold run history, choose transaction steps, call an EVM
provider, retry delivery, or decide terminal state.

The resulting shape is:

```text
certified graph
    |
    v
Runtime -- sole writer --> RunHistory adapter --> PostgreSQL implementation
    |
    +-- authorized Read ----> one domain adapter ----> transport/provider
    |
    +-- authorized Effect --> one domain adapter ----> transport/provider/signer
    |
    +-- authorized Effect --> WalletNonce adapter --> PostgreSQL implementation
```

PostgreSQL's client and wire protocol are infrastructure transports. A schema-aware PostgreSQL
implementation that owns transactions, compare-and-append, fencing, exact idempotency, or nonce
uniqueness is an authority-bearing adapter/store. Calling PostgreSQL a transport must not hide
those responsibilities or expose a raw pool.

This RFC accepts the existing forensic baseline:

- every normally returned access result or error is durably observed before normal Runtime
  success;
- process loss can leave an unmatched authorization;
- absence of an observation never proves that an external effect did not happen; and
- no universal outbox is added to preserve every returned read or mutation result across process
  loss.

Every admitted effect must prove convergence when the identical qualified request is entered more
than once. Effect re-entry after an unmatched authorization is therefore legal only for that
identical request and binding. Arbitrary non-repeat-safe mutations are unsupported by this
baseline. This is a safety qualification, not a retry policy.

## Why The Existing Design Is Wrong

### Physical persistence was centralized, but the recording obligation was not

The store can be the only component that physically appends run records while callers still own
the obligation to remember the second half of an external-access protocol:

```text
caller
  -> append authorization
  -> invoke live boundary
  -> inspect or encode result
  -> append observation
```

Any ordinary return between invocation and observation can silently violate the audit contract.
An especially dangerous form is an outer Rust `Result`:

```text
let completion = adapter.invoke(authority).await?;
append_observation(completion).await?;
```

If the first `?` returns after the external boundary was entered, the effect or provider error
survives while no observation is journaled. “Surviving effect failures are never journaled” was
one instance of this ownership flaw, not an isolated error mapping bug.

The fix is not another helper convention. One private component must own authorization, affine
invocation authority, totalization, the pending observation, and observation commit as one
uninterruptible normal-return protocol. That owner is `Runtime`.

### The executor became a second Runtime

The existing generic executor does substantially more than adapt one operation. It owns or
participates in:

- a second append-only history;
- its own fold and verified current view;
- resource allocation;
- target-attempt authorization and observation;
- delivery planning and convergence;
- restart and compare-and-append loops;
- retained evidence and frontiers;
- terminalization and tombstones;
- an independent writer fence; and
- wallet-specific next-action selection.

Those are interpreter responsibilities. Nesting that lifecycle below Runtime duplicates the exact
failure protocol this RFC is trying to make singular:

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

It also hides outcome-affecting EVM topology outside the certified graph. Removing the executor
does not remove the real EVM work. It places each responsibility in one honest owner:

- states own domain progression and evidence interpretation;
- Runtime owns run interpretation and run-history mutation;
- adapters execute one already-selected operation;
- transports perform bounded protocol exchanges;
- a narrow resource adapter owns cross-run nonce uniqueness; and
- the destination supplies idempotency or reconciliation needed for effect recovery.

### A state output cannot itself be a cross-run lock

An output such as `ReservedWalletNonce` is necessary but not sufficient. A run journal only orders
one run; two runs can otherwise emit the same nonce concurrently.

The output is therefore proof of a durable reservation, not the lock itself. The
wallet-nonce adapter's atomic store transaction creates the cross-run exclusion. The certified
edge carrying `ReservedWalletNonce` provides within-run causality and exact provenance.

### A different module name cannot remove irreducible authority

Moving delivery planning into an “adapter” would preserve the executor under a less accurate name.
Moving nonce allocation into a “transport” would hide durable authority in infrastructure.
Moving cross-run allocation into Runtime would make Runtime coordinate unrelated runs.

The target instead applies a narrowness test:

> An adapter accepts one already-selected typed operation, performs one bounded semantic
> interaction, and returns one closed result. It owns no workflow history or next-action policy.

A resource adapter may perform one atomic transaction because that transaction is the operation.
It remains narrow as long as it owns only its resource invariant.

## Goals

- Make `Runtime` the sole active interpreter and sole run-history mutation owner.
- Make every normal return after authorized access structurally pass through observation commit.
- Preserve `Pure | Read | Effect` as certified semantic state kinds.
- Use one typed access protocol for reads and effects without conflating their semantics.
- Keep effect-bearing nodes visible for scheduling, audit, concurrency, and recovery analysis.
- Represent meaningful EVM progression as explicit certified states and graph dependencies.
- Replace generic executor resource machinery with the smallest resource-specific authority.
- Make provider, transport, signer, adapter, and resource-store faults durably classifiable.
- Add no retry scheduling, backoff, failover, or circuit-breaker policy.
- Preserve honest ambiguity after cancellation, crash, process loss, and lost acknowledgement.
- Keep state logic deterministic and free of ambient IO.
- Preserve five append-only run-history record families and separate audit from semantic progress.
- Keep secrets and provider-controlled diagnostic text out of every persisted and public surface.
- Delete superseded executor concepts rather than preserve compatibility layers.

## Non-Goals

- Making authorization, external IO, and observation one atomic transaction.
- Guaranteeing an observation after process termination, machine loss, or storage loss.
- Preserving every returned read through a universal outbox.
- Supporting arbitrary non-idempotent and non-queryable external mutations.
- Adding a retry policy or interpreting fault codes as retry instructions.
- Adding concurrent state execution in this RFC.
- Making state callbacks, adapters, transports, or applications append run history.
- Making Runtime a cross-run resource coordinator.
- Turning all durable authorities into one generic semantic store.
- Exposing PostgreSQL pools or generic query authority to Runtime or live adapters.
- Persisting raw signed transactions, signatures, credentials, provider response bodies, URLs, or
  unreviewed error strings.
- Encoding authorization and observation as fake semantic transitions.
- Defining an implementation or commit plan before the design gates in this RFC are resolved.

## Terminology

### Semantic execution kind

The certified property of a state:

- `Pure`: deterministic local computation with no semantic external IO.
- `Read`: one typed external observation that does not intentionally mutate the target.
- `Effect`: one typed operation that may mutate an external target or consume exclusive external
  capability.

This is semantic metadata, not three independent Runtime engines.

### Access kind

The sealed Runtime protocol parameter:

```text
AccessKind = Read | Effect | FactSelection
```

`FactSelection` is a reserved audited read with store-owned completion semantics. It remains
separate because it consumes a private affine scan, but it uses the same authorization and
observation bracket.

### One bounded semantic interaction

One already-selected operation with one stable request and one closed completion. An adapter may
use the minimum lower-level primitives needed to implement that operation—for example a qualified
signer followed by one JSON-RPC submission—but it may not select another semantic operation,
retry, poll, fail over, fold history, or terminalize a workflow.

### Repeat-safe effect

An effect for which qualification proves that re-entering the exact same immutable request under
the same semantic key cannot create a second semantic mutation. Destination idempotency, exact raw
transaction rebroadcast, or an authoritative idempotent resource operation can establish this
property.

Repeat safety applies only to byte- and binding-identical requests. It does not permit a different
nonce, candidate, fee, route, signer generation, or provider binding.

### Semantic retry

A later domain decision that changes the external request, such as an EVM fee replacement. It is a
new explicit state or certified graph branch with its own authorization. It is not an invisible
adapter retry.

### Physical re-entry

A later authorization for the identical request while previous authorizations have no compatible
committed state-consumable observation. It exists only for crash or concurrent-worker recovery,
not for a committed fault or domain-directed retry. For effects it is permitted only by the
qualified repeat-safety rule.

### Resource authority

A narrow durable service or adapter that serializes a cross-run invariant that no individual run
journal can prove. The wallet-nonce adapter is the current example.

### Adapter

An implementation of a semantic port. It binds one Runtime-authorized request to lower primitives,
totalizes every normal result, and owns no graph or workflow lifecycle.

### Transport

A reusable lower-level mechanism for encoding, sending, receiving, and checked decoding. It owns
no MFM journal, state, scheduling, or domain progression.

### RunHistory adapter/store

The passive authority that verifies and atomically persists legal run-history commits. Runtime
alone holds its mutation capability. “Adapter” describes placement; “store” describes its durable
responsibility.

## Required Guarantees

### G-01: One run-history mutation owner

Only `Runtime` instances can hold a production `RunHistoryWriter`. Admission and every existing-run
mutation route through Runtime. Application, scheduler helpers, state callbacks, adapters,
transports, replay, CLI, and REST receive no append authority.

### G-02: Authorization precedes possible entry

No read, effect, fact selection, resource transaction, signer operation, or provider call may
begin until its exact authorization is positively committed.

An already-existing authorization, an ambiguous append acknowledgement, or a stale candidate
cannot mint live authority.

### G-03: One affine invocation

A newly committed authorization mints one non-cloneable `Authorized<K>`. Consuming it permits at
most one invocation of the exact registered operation and binding.

### G-04: Every normal completion is closed

After the adapter accepts `Authorized<K>`, its Runtime-facing invocation has no outer error
channel. Every normally returned provider result, transport failure, signer failure, adapter
failure, resource-store failure, encoding failure, and contract failure is represented in one
closed persistable completion.

### G-05: Pending observation cannot escape

The adapter completion is immediately owned as `PendingObservation<K>`. No normal successful
Runtime result, state callback, application result, or live value can escape before exact
observation commit or identical-content resolution.

### G-06: Observation is exactly linked

Every observation names exactly one authorization and preserves its access kind, node occurrence,
operation, binding, request, semantic anchor, and outcome contract. At most one observation exists
for an authorization.

### G-07: Non-domain faults are recorded but not consumed

Every normally returned external provider, transport, signer, adapter, or resource-store fault
that is not admitted by a certified `SafeFailure` contract is persisted as
`AccessNonDomainFault`.

State logic cannot see it as a domain result or failure. Classification exists for audit,
operations, and possible future policy; it grants no retry authority.

### G-08: Process loss remains honest

Cancellation, panic, abort, or process loss may leave a committed authorization without an
observation. Verification preserves that unmatched authorization. It never infers non-entry,
success, failure, or a lost return value.

### G-09: Effect re-entry is safety-qualified

Every registered effect operation must be qualified as exact-request convergent. An unmatched
effect authorization has no classified entry status and is conservatively treated as possible
entry. Runtime may then authorize only the byte- and binding-identical effect request again.

Different requests are semantic retries and require distinct certified state intent.

An arbitrary non-idempotent, non-queryable mutation that cannot satisfy exact-request convergence
is not certifiable as an MFM `Effect` under this RFC.

A committed `AccessNonDomainFault` does not authorize re-entry. It blocks this RFC's Runtime
projection until a future policy is deliberately specified.

### G-10: Invoked settlement consumes committed proof

After any access authorization exists, a `Read` or `Effect` transition may settle only from a
freshly verified committed observation for that exact state, request, operation, binding, and
contract. Pending material, unmatched authorization, and non-domain faults cannot settle the
state. A local pure settlement remains legal only before the node has any authorization history.

### G-11: Cross-run resources have one narrow authority

Every resource domain has one qualified durable owner of its uniqueness rule. Every actor capable
of signing or submitting for that sender—including other runs, relayers, operators, and stale
deployments—must use that same authority through Runtime-authorized `Effect` states or be
permanently fenced. Qualification proves signer-generation custody and the absence of an alternate
direct allocation/submission path.

### G-12: State kind remains available for scheduling

The certified graph and every access record retain `Read` versus `Effect`. One drive selects at
most one audited access, and an unresolved effect node prevents a different dependent effect from
advancing. Any future parallel execution of distinct effect states must additionally prove graph
independence, compatible resource claims, and target repeat safety. Classification alone is not a
concurrency proof.

### G-13: Store validation remains authoritative

Runtime proposes commits. The store alone validates legal record shape, exact predecessor,
logical-key uniqueness, object closure, transition legality, semantic versus audit heads, and
closure rules before atomic append.

### G-14: No secrets

Requests, observations, reservation evidence, fault classifications, traces, exports, and public
projections contain no credentials, private keys, signatures, raw signed envelopes,
provider-controlled text, secret paths, or secret-bearing endpoint details.

## Target Architecture

### Responsibility placement

| Concern | Sole owner |
| --- | --- |
| Certified graph topology and execution kind | operation planning and certification |
| Domain request authorship and observation interpretation | state |
| One run's next action | `Runtime` over `VerifiedRunView` |
| Run-history mutation orchestration | `Runtime` |
| Legal append, exact-head CAS, and run fold | `RunHistory` store |
| One already-selected live operation | adapter |
| Protocol exchange | transport |
| Signer generation and secret custody | qualified signer |
| Cross-run sender/nonce uniqueness | wallet-nonce resource adapter |
| EVM progression and terminal meaning | explicit EVM states and graph edges |
| Recorded verification | store/replay over committed evidence |
| Authentication and rendering | app, CLI, and REST |

No row is owned by an executor.

### One closed state algebra

Conceptually:

```text
StateExecution =
    Pure(PureStateContract)
  | Read(ReadStateContract)
  | Effect(EffectStateContract)
```

The callbacks remain pure. Read and effect states first make one minimal closed access decision:

```text
AccessPlan<Request> =
    Invoke(Request)
  | Settle(Settlement)
  | BlockedUnresolved
```

`Settle` lets a statically certified read or effect node become inapplicable or terminal from its
already committed inputs. It prevents no-op live calls and permits early-terminal branches in a
finite graph without adding a dynamic runner. `BlockedUnresolved` honestly leaves the node and run
open when the certified finite policy has no legal access or settlement.

The contracts are:

```text
Pure:
  apply(StateFrame) -> Settlement

Read:
  plan(StateFrame) -> AccessPlan<ReadRequest>
  settle(
      StateFrame,
      CommittedObservationView<
          Returned(ReadResponse)
        | DidNotEnter(ReadSafeFailure)
        | Indeterminate(ReadSafeFailure)
      >
  ) -> Settlement | InvalidEvidence

Effect:
  plan(StateFrame) -> AccessPlan<EffectRequest>
  settle(
      StateFrame,
      CommittedObservationView<
          Returned(EffectResponse)
        | DidNotEnter(EffectSafeFailure)
        | Indeterminate(EffectSafeFailure)
      >
  ) -> Settlement | InvalidEvidence
```

An effect state authors one bounded external operation. It does not author a hidden delivery
program. If a different operation is meaningful—reserve a nonce, submit a transaction, inspect a
receipt, or mark a reservation complete—it is a different state.

`AccessPlan` is not a new execution kind, workflow DSL, or open action sum. A read or effect node
has one statically certified operation. Runtime can only commit the supplied pure settlement or
invoke that one operation through the shared access bracket. The only third choice is the
stable no-action `BlockedUnresolved`; there is no “choose another operation,” retry, or loop
variant.

`BlockedUnresolved` performs no IO and appends no history. It defines no wake-up, timer, or retry
policy. Repeated drive calls report the same stable block; this RFC provides no source of progress
from it. Recovery requires a future explicit design, not repeated planning.

Each bounded poll, submission, or replacement is a distinct certified node. Once a compatible
returned or safe-failure observation commits, this node must settle or block as invalid evidence;
it cannot silently invoke the same operation again. Multiple authorizations for one node exist
only to recover an unmatched crash/concurrent prefix before a consumable observation is present.

### One terminal transition shape

The target semantic phase is:

```text
NodePhase = Unstarted | Terminal
RunPhase  = Open | Closed
```

The target transition body is:

```text
TransitionBody =
    StateSettled {
        settlement_source:
            LocalPlan
          | CommittedObservation(ObservationRef)
    }
  | DependencySkipped {
        blocking_sources
    }
```

The certified node kind determines the legal form:

- `Pure` settles only from `LocalPlan`;
- `Read` and `Effect` may settle from their pure `AccessPlan::Settle`;
- an invoked `Read` settles from one compatible committed read observation;
- an invoked `Effect` settles from one compatible committed effect observation; and
- `DependencySkipped` records exact terminal blockers without invoking a callback.

For `CommittedObservation`, the store rejects a missing, extra, wrong-kind, wrong-request,
wrong-binding, non-domain, or uncommitted observation reference. `LocalPlan` carries no implied
external access and must be reproduced by the node's pure callback when that verification mode is
requested.

`LocalPlan` is legal only while the node occurrence has no authorization history. Once live access
has been authorized, the node can settle only from a compatible committed observation. This
prevents a later pure branch from ignoring an ambiguous or already-entered effect.

There is no generic `EffectRequested`, `AwaitingEffect`, executor `ensure`, or `EffectSettled`
phase. `ExternalAccessAuthorized` already commits the exact effect request before target entry.
The terminal `StateSettled` transition already records the semantic outcome. Duplicating those
facts creates lifecycle without adding authority.

### Runtime action derivation

For one ready node, Runtime derives actions in this order:

1. validate the complete access suffix for the node;
2. block if any pre-settlement integrity-class non-domain fault exists;
3. select the earliest compatible state-consumable observation in authorization order;
4. if one exists, call the pure `settle` callback and commit its settlement or block on invalid
   evidence;
5. otherwise, block if any operational-class non-domain fault exists;
6. otherwise, evaluate the pure `AccessPlan`; and
7. commit its local settlement, report `BlockedUnresolved`, or prepare its one statically
   registered access, including exact recovery of an unmatched prefix.

One `drive_once` still performs at most one semantic transition or one audited access operation.
Other observations for duplicate crash/concurrent entries remain validation-only audit evidence.
Pure evaluation cannot append history or invoke live IO.

An integrity fault always blocks an unsettled node. An operational fault may be bypassed only by an
already committed compatible consumable observation from an overlapping exact-request invocation;
otherwise it blocks. Neither class schedules or authorizes another call. A fault arriving after
`StateSettled` is late audit evidence and cannot reopen semantics.

### Prepared access

After `AccessPlan::Invoke` and before authorization, Runtime validates and freezes:

- run, node occurrence, and semantic head;
- certified execution and access kind;
- operation identity and implementation binding;
- immutable typed request and canonical bytes;
- request and response contracts;
- routing and signer generation references where applicable;
- result encoder and safe-failure classifier;
- non-domain fault classifier;
- stable call or reservation key;
- effect repeat-safety qualification; and
- required resource-output provenance.

Any failure here happens before live authority exists and appends no authorization.

The state and adapter do not choose the stable access key. One kernel identity function derives:

```text
AccessCallKey = H(
    "mfm.access-call.v1",
    tenant + run + node occurrence + access kind
    + operation identity + stable implementation-binding identity
    + canonical semantic-request digest
)
```

All recovery authorizations for the identical node request share this key while retaining distinct
authorization identities. A destination idempotency key, when required, is derived from this
sealed value. Adapter-chosen or caller-supplied idempotency keys are not authoritative.

### Private access protocol

The only live sequence is:

```text
prepare<K>()
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

Constructors and fields of every authority-bearing type are private. `Authorized<K>` and
`PendingObservation<K>` cannot be cloned or returned to application code.

The runtime-facing adapter contract is exhaustive:

```text
AccessCompletion<K> =
    Returned(TypedResponse<K>)
  | DidNotEnter(TypedSafeFailure<K>)
  | Indeterminate(TypedSafeFailure<K>)
  | AccessNonDomainFault

invoke(Authorized<K>) -> AccessCompletion<K>
```

`Authorized<K>` owns the exact prepared request and binding. Passing a separate request would
weaken that authority relation and is forbidden.

There is no outer `Result`. Internal transport and encoding code may use `Result`, but the private
adapter wrapper must convert every normal error into the closed completion before returning. The
non-domain variants have bounded, infallibly encodable canonical forms so totalization cannot
itself introduce another post-access escape.

Rust cannot guarantee observation after process death. The enforced type property is narrower and
honest:

```text
no constructible normal-success path skips exact observation commit
```

### Observation persistence

`PendingObservation<K>` owns stable logical material independently of a predecessor-bound physical
append candidate.

Runtime:

1. resolves the authorization's observation logical key;
2. accepts an existing observation only when its canonical content is identical;
3. prepares an append against the current verified journal head;
4. rebases after a definite stale-head result without reinvoking;
5. resolves an acknowledgement-ambiguous append using the unchanged physical attempt; and
6. returns a committed proof only after positive append or exact identical resolution.

While the task lives, store unavailability does not turn a pending observation into normal
success. If the task or process dies, the pending value may be lost and the authorization remains
unmatched.

### Run closure and late audit

The five run-history families remain:

```text
RunAdmitted
StateTransitionCommitted
ExternalAccessAuthorized
ExternalAccessObserved
RunClosed
```

`RunClosed` is committed atomically with the final semantic transition. A legal pre-closure
authorization may receive one linked observation after closure. That record advances only the
journal head; it cannot reopen the semantic head or change the run result.

Authorization and observation therefore remain audit records rather than no-op semantic
transitions.

## Effect Safety Without A Retry Policy

### Qualification

Every effect operation requires a qualification proof that identical re-entry converges on one
semantic operation. Examples include:

- an idempotent API keyed by the exact operation identity;
- exact rebroadcast of one deterministic EVM transaction; and
- an idempotent nonce reservation keyed by one permanent reservation identity.

The proof has explicit owners:

- the effect operation manifest names the exact convergence contract and stable-key preimage;
- certification binds the effect state to that operation manifest;
- deployment qualification validates the chosen adapter, destination, signer, route, and resource
  implementation against the contract;
- Runtime enforces byte-, binding-, and key-identical re-entry; and
- destination-specific crash and concurrent-entry tests prove the claimed predicate.

Runtime does not infer convergence from an adapter name or a fault code.

The proof answers “is another identical entry safe?” It does not answer:

- when to drive again;
- how many attempts to make;
- how long to wait;
- which fault code deserves another attempt;
- which provider to select;
- whether to back off or add jitter; or
- when to trip a circuit breaker.

Those policies are absent from this RFC. An effect that cannot supply the proof is rejected during
qualification. It must instead use an authoritative idempotent external service or wait for a
separate durable-delivery design.

### Exact request means exact

Re-entry freezes:

- semantic operation key;
- canonical request bytes;
- target binding and routing generation;
- resource reservation;
- signer generation;
- chain and sender;
- transaction candidate and fee parameters; and
- every response and safety contract.

A fee replacement or different route is not re-entry. It is a new semantic state with its own
authorization.

A resource store's monotonic writer-generation credential is physical fencing, not part of the
semantic resource request or stable binding identity. It may advance across failover only over the
same non-rollback resource lineage. Existing reservation resolution still returns the original
canonical proof; generation rotation never changes the reservation key or creates a new namespace.

### Initial concurrency rule

This RFC does not add parallel execution of distinct state actions. One `drive_once` authorizes at
most one access, and an unresolved effect remains the current semantic node.

Two processes can nevertheless overlap invocations of the identical effect request: neither can
distinguish a dead worker from a slow worker merely from an unmatched authorization. This is why
exact-request convergence is mandatory rather than an optimization. The overlap does not permit a
different effect request or state to pass the unresolved node.

Future parallelization may use the retained state kind plus explicit graph dependencies and
qualified resource claims. Two states both being `Effect` says that a hazard may exist; it does
not prove that they conflict or that they are independent.

Cross-run conflicts are never decided from per-run scheduling. They are serialized by the narrow
resource authority or by the external destination's idempotency contract.

## Non-Domain Fault Tracking

### Persisted relation

Every non-domain normal completion is represented as:

```text
AccessNonDomainFault {
    code: AccessNonDomainFaultCode,
    entry_status: ProvenNotEntered | MayHaveEntered,
}
```

`AccessNonDomainFaultCode` is a closed, redaction-safe vocabulary. Each code has one frozen derived
origin and class:

```text
origin = Adapter | Transport | Provider | Signer | RunHistoryStore | ResourceStore
class  = Operational | Integrity
```

Origin and class are projections of the code rather than independently writable fields. The
authorization already supplies exact access kind, operation, request, route/binding generation,
and semantic anchor, so the observation does not duplicate them.

The initial target vocabulary is:

| Code | Derived origin | Derived class |
| --- | --- | --- |
| `AdapterContractViolation` | Adapter | Integrity |
| `ResultEncodingFailure` | Adapter | Integrity |
| `TransportUnavailable` | Transport | Operational |
| `TransportProtocolFailure` | Transport | Operational |
| `ProviderUnavailable` | Provider | Operational |
| `ProviderRateLimited` | Provider | Operational |
| `ProviderProtocolViolation` | Provider | Integrity |
| `SignerUnavailable` | Signer | Operational |
| `SignerContractViolation` | Signer | Integrity |
| `FactSelectionUnavailable` | RunHistoryStore | Operational |
| `FactHistoryInvalid` | RunHistoryStore | Integrity |
| `ResourceStoreUnavailable` | ResourceStore | Operational |
| `ResourceGenerationRejected` | ResourceStore | Integrity |
| `ResourceCapacityExhausted` | ResourceStore | Operational |
| `ResourceHistoryInvalid` | ResourceStore | Integrity |

The origin boundary is exact:

- `Adapter` means local MFM binding, returned-value validation, or canonicalization failed.
- `Transport` means connection, session, framing, timeout, or checked protocol delivery failed
  before a valid provider-level completion was classified.
- `Provider` means the external provider returned a validly attributable provider-level response
  whose reviewed class is unavailable, rate-limited, or contract-violating.
- `Signer` means the separately qualified signer boundary failed.
- `RunHistoryStore` here means the reserved fact-selection scan source, not journal append.
- `ResourceStore` means the narrow nonce authority's transaction or history.

There is no `Executor` origin because no executor boundary remains.

The exact entry status is classified from affine authority and boundary progress, not guessed from
the error name. A connection failure after bytes may have reached a provider is
`MayHaveEntered`. A locally rejected request before target entry may be `ProvenNotEntered`.

`FactSelectionUnavailable` and `FactHistoryInvalid` describe the reserved post-authorization fact
scan returning to Runtime. They do not describe failure of the core authorization or observation
append. A run-history persistence failure cannot be recorded in the unavailable journal as its own
access fault; it remains a Runtime interruption, retaining pending material while the task lives.

For nonce `reserve` and `complete`, a normal `ResourceStoreUnavailable` is restricted to proven
pre-entry failure. Resource-commit ambiguity stays inside the resource adapter's exact-resolution
protocol described below.

### Safe failure versus non-domain fault

A certified `SafeFailure` is typed domain-relevant evidence that state logic is explicitly allowed
to interpret, such as a reviewed “transaction not found” observation.

An `AccessNonDomainFault` is platform or integration evidence. State logic never consumes it.
Provider-controlled strings are not made domain evidence merely because a provider returned them.

The boundary classifier must either:

- map a reviewed response into the exact certified returned or safe-failure contract; or
- map it into one closed non-domain fault code.

There is no third normal return channel.

### No retry semantics

`Operational` means the fault is not itself evidence of a corrupted contract. It does not mean
“retry now” or even “retry this operation.” With no already-committed compatible consumable
observation from an overlapping invocation, it blocks the node in this RFC. `Integrity`
deterministically blocks an unsettled node even if an overlapping invocation also returned a
consumable value. A fault committed after settlement remains audit-only and cannot reopen the
node.

Any future retry policy may query these codes, but adding that policy requires a separate RFC.
Changing the closed code relation is a persisted-schema change.

### Public and forensic projection

Trace, audit, replay, CLI, and REST may expose:

- authorization and observation references;
- state and access kind;
- operation and secret-free binding generation;
- fault code, derived origin/class, and entry status; and
- whether the authorization remains unmatched.

They must not expose provider text, response bodies, URLs, credentials, raw SQL errors, file
paths, signed payloads, or secret material.

This provides trackability of external-provider and transport failures without turning diagnostic
text into protocol or retry policy.

## Wallet Nonce Resource Authority

### Why it exists

EVM sender nonces are shared by every actor capable of submitting for the same sender.
A graph edge orders states within one run but cannot exclude another run. Provider reads also
cannot allocate a nonce atomically across MFM workers.

The wallet-nonce adapter is the single authority for one stable identity:

```text
WalletNonceDomain =
    tenant
  + chain identity
  + sender identity
  + stable wallet-domain identity
```

Writer generation is deliberately not part of `WalletNonceDomain`. It is a monotonic fencing
credential over the same durable resource lineage. Rotation, restore, and promotion must carry
forward every reservation, completion, and high-water mark; changing generation cannot create a
fresh nonce namespace.

It exposes exactly two idempotent semantic operations:

```text
reserve(
    wallet_nonce_domain,
    [private current resource-writer fence],
    runtime_derived_reservation_key,
    intent_digest,
    observed_pending_floor?
) -> ReservedWalletNonce
   | AccessNonDomainFault

complete(
    wallet_nonce_domain,
    [private current resource-writer fence],
    provenance_verified_reservation,
    provenance_verified_terminal_evidence
) -> CompletedWalletNonce
   | AccessNonDomainFault
```

Both are `Effect` accesses and therefore receive ordinary Runtime authorization and observation.
The database transaction may internally distinguish `Applied` from `ExistingSame`, but the adapter
returns the same canonical semantic evidence for both. Insertion status is not part of the state
result.

### Durable invariants

The adapter/store enforces:

- `(wallet_nonce_domain, reservation_key)` maps permanently to one reservation;
- `(wallet_nonce_domain, nonce)` is unique across every writer generation;
- the same key, intent, and domain return byte-identical reservation evidence;
- the same key with different intent or domain is `ResourceHistoryInvalid`;
- allocation is linearizable across processes and runs;
- the current non-rollback writer generation is checked by every mutation;
- stale and sibling writers cannot mutate the resource lineage;
- backup, restore, and promotion cannot roll the reservation prefix or high-water mark backward;
- completion is exact and idempotent;
- the same completion key with different terminal evidence is `ResourceHistoryInvalid`;
- completion never changes the assigned nonce;
- a reservation is never automatically released or reused; and
- ambiguous database acknowledgement is resolved by the original operation identity before a new
  transaction is attempted.

An acknowledgement-ambiguous reserve or complete never normally returns
`ResourceStoreUnavailable(MayHaveEntered)`. The adapter retains and resolves the original resource
operation identity until it obtains the identical proof or an integrity conflict. Cancellation or
process loss abandons that in-memory obligation and leaves the Runtime authorization unmatched, so
exact re-entry can resolve it. A normal resource-store availability fault is legal only when
non-entry is positively established.

The adapter owns no EVM delivery records, target attempts, candidate selection, polling,
replacement, retry timing, terminal-evidence construction, or run settlement.

### Provider nonce is an observed floor

`eth_getTransactionCount(sender, "pending")` is a read observation. It may lag, disagree across
providers, or race with another workflow. It is not a lock and is not allocation authority.

The reserve transaction treats a verified provider value as an observed lower bound. A new
allocation cannot be below that floor, but the durable local high-water mark or concurrent
reservations may require a higher nonce.

If product policy requires equality with a particular provider observation, that is an explicit
precondition that may conflict and be re-read. It must not weaken the unique durable reservation.

Qualification must also prove exclusive sender control: every signer, relayer, operator path,
stale deployment, and direct-submit path capable of using the sender either consumes this same
wallet-nonce authority or is fenced out. The provider floor cannot close a read/allocation race
with an uncoordinated actor.

### Typed state provenance

The graph uses:

```text
ObservePendingNonceState [Read]
  -> ObservedPendingNonceFloor

ReserveWalletNonceState [Effect]
  consumes: transaction intent + optional observed floor
  produces: ReservedWalletNonce

PrepareSignedCandidateState [Effect]
  consumes: exact transaction intent + ReservedWalletNonce
  produces: SignedCandidateProof

BroadcastExactCandidateState [Effect]
  consumes: exact transaction intent + ReservedWalletNonce + SignedCandidateProof

CompleteWalletNonceState [Effect]
  consumes: ReservedWalletNonce + terminal transaction evidence
```

`ReservedWalletNonce` conceptually contains only secret-free evidence:

```text
ReservedWalletNonce {
    wallet_nonce_domain,
    resource_lineage_ref,
    allocated_under_generation_ref,
    signer_generation_ref,
    chain_id,
    sender,
    nonce,
    reservation_key,
    intent_digest,
    reservation_evidence_ref,
}
```

The state does not author `reservation_key`. Runtime derives it with the nonce contract's frozen
domain-separated identity function from the stable wallet nonce domain, run, reserve-node
occurrence, adapter binding, and exact intent digest. The nonce adapter recomputes or validates
that sealed key; it never accepts an arbitrary caller-selected identity.

The consumer does not trust matching field shape alone. Certification and Runtime preflight
require the value to come through the exact graph edge from the registered
`ReserveWalletNonceState`, under the same qualified wallet domain, intent, signer generation, and
resource lineage.

The durable reservation is the cross-run exclusion. The typed value is the proof carried through
the run.

### Completion and abandoned reservations

`CompleteWalletNonceState` records that exact terminal evidence was associated with the
reservation. Before authorization, Runtime verifies the evidence's producer run, node,
transition, transaction hash, chain, sender, nonce, and graph edge, then seals that proof into the
authorized completion request. The resource adapter never accepts a reference-shaped value on
field equality alone. Completion does not release or recycle the nonce.

The baseline allocator may reserve later nonces while earlier reservations remain incomplete.
Whether a product deliberately permits only one incomplete reservation per wallet is a qualified
resource policy choice, not a Runtime or transport responsibility.

Abandoned reservations therefore create durable gaps rather than unsafe reuse. Any administrative
repair requires a separately designed, explicitly authorized, evidence-based operation. Timeouts
alone never release a nonce.

## EVM Without An Executor

### Explicit certified progression

A representative EVM operation becomes a graph such as:

```text
ObservePendingNonce [Read]
  -> ReserveWalletNonce [Effect]
  -> BuildUnsignedCandidate [Pure]
  -> PrepareSignedCandidate [Effect]
  -> BroadcastExactCandidate [Effect]
  -> ObserveTransaction [Read]
  -> ObserveReceipt [Read]
  -> ObserveFinalizedHead [Read]
  -> VerifyCanonicalInclusion [Read]
  -> CompleteWalletNonce [Effect]
  -> ProjectTransactionResult [Pure]
```

The exact graph may branch for rejection, revert, reorganization, or a bounded replacement policy.
Every branch that changes the candidate, nonce, fee, or semantic request is visible and certified.
Nodes made irrelevant by an earlier branch use `AccessPlan::Settle`; they do not make placeholder
adapter calls.

Each bounded poll is a distinct certified read state. Multiple authorization/observation pairs for
one read or effect state are legal only when crash/concurrent recovery authorized the identical
request before a compatible consumable observation committed. Absent a pre-settlement integrity
fault, the earliest compatible observation settles the node; later duplicates are audit-only. A
different replacement candidate is a different state, not another physical attempt hidden in an
adapter.

If the complete certified convergence policy is exhausted without terminal evidence, its
remaining access state returns `AccessPlan::BlockedUnresolved`. The run and nonce reservation
remain open. Repeated drives do not poll. MFM does not fabricate non-application, complete the
reservation, or hide an unbounded loop in the adapter.

### Signing and submission boundaries

`PrepareSignedCandidate` invokes the qualified signer for the exact unsigned candidate and returns
only a secret-free proof:

```text
SignedCandidateProof {
    unsigned_candidate_digest,
    transaction_hash,
    signer_generation_ref,
    signing_contract_ref,
}
```

It never persists a signature or raw signed envelope. The state exists separately so signer
failure and the exact pre-broadcast transaction identity are visible in run history.

`BroadcastExactCandidate` receives:

- the exact transaction intent;
- a provenance-verified `ReservedWalletNonce`;
- the exact unsigned candidate and provenance-verified `SignedCandidateProof`;
- immutable chain and sender identity;
- one qualified signer generation;
- one immutable route generation; and
- the exact candidate and repeat-safety contract.

Because bearer signed bytes cannot be retained, the broadcast adapter deterministically signs the
same candidate again inside its authorized operation, verifies that the derived transaction hash
equals `SignedCandidateProof`, submits that exact envelope once, and discards the bytes. The
adapter cannot select a nonce, alter the candidate, replace fees, rotate routes, poll, rebroadcast
in a loop, or decide terminality.

For direct EVM submission to be `ExactRequestConvergent`, the qualified signer and candidate must
reproduce the same signed transaction bytes and hash. Re-entry may submit only that exact
transaction. Provider responses such as an exact “already known” relation must be normalized into
reviewed evidence for the same hash; provider text is never persisted.

If exact reproduction or destination convergence cannot be proved, direct submission is not
admissible under this baseline. The product must use an authoritative idempotent wallet/relayer or
adopt a stronger durable-delivery design in a separate RFC.

### Reads and terminal meaning

Transaction lookup, receipt lookup, finalized-head observation, and canonical-inclusion checking
are explicit `Read` states. Their typed requests bind sender, nonce, transaction hash, chain, and
required block relation as applicable.

Pure state logic derives terminal success, revert, replacement, or unresolved status only from
committed compatible observations. No adapter folds wallet history or selects the next call.

`CompleteWalletNonceState` receives the resulting terminal evidence. It does not construct that
evidence or decide that the transaction is terminal.

`ProjectTransactionResult` and the run terminal contract require the exact
`CompletedWalletNonce` produced from that same reservation and terminal-evidence reference.
Certification proves the graph edge and contracts; Runtime/store verification rejects reservation,
completion, producer, or evidence substitution. No alternate success/revert branch may close the
run after terminal transaction evidence but before resource completion.

## Store And PostgreSQL Boundary

### RunHistory is an adapter and an authority

Runtime depends on a narrow run-history port. Its implementation may use PostgreSQL, but it owns:

- append-only record and object persistence;
- per-run exact-head compare-and-append;
- logical-key idempotency;
- acknowledgement-ambiguity resolution;
- atomic object and record visibility;
- writer generation and non-rollback lineage;
- structural and semantic fold validation; and
- purpose-specific read views.

These responsibilities do not belong to SQL, `sqlx`, or Runtime.

The production assembly consumes bootstrap database authority and exposes separately scoped
capabilities:

```text
qualified infrastructure
  -> run-history database role/scope
       -> RunHistoryWriter   // consumed only by Runtime
       -> RunHistoryReader   // purpose-limited readers
  -> wallet-nonce database role/scope
       -> WalletNonceAdapter // reachable only by its registered Runtime invoker
```

The run-history role cannot mutate nonce authority, and the nonce role cannot name or mutate run
history. No generic owner, application pool, or query capability survives assembly. Provisioning
authority is not retained by Runtime, application, or live adapters.

The two scopes may share one physical PostgreSQL deployment only when qualification proves
non-rollback lineage, stale/sibling-writer exclusion, backup, restore, and promotion for both.
Physical co-location does not merge their schemas, roles, fences, or semantic algebras.

### One physical primitive is optional, one semantic store is not

Run history and nonce reservations may share lower-level immutable objects, transactions, or a
generic atomic compare-and-append primitive if that genuinely reduces code.

They must retain separate semantic validators:

- the run-history store validates the five-record run algebra; and
- the wallet-nonce adapter validates its two-operation resource algebra.

A universal tagged “history engine” with callbacks, planners, or policy hooks would recreate the
generic executor and is rejected.

### The resource store is not a run-history escape

The nonce adapter necessarily mutates its own durable resource authority. That mutation is not a
`StateTransitionCommitted` record.

It remains causally visible in the run:

```text
ExternalAccessAuthorized(Effect, ReserveWalletNonce)
  -> atomic resource transaction
  -> ExternalAccessObserved(reservation or closed fault)
  -> StateTransitionCommitted(consumes reservation observation)
```

Only the resource store decides cross-run uniqueness, while only Runtime records why a run invoked
it and what normal result the run observed. No state, app, or adapter can directly write both
authorities.

## Replay And Verification

Recorded verification is callback-free. It validates:

- all five record families;
- exact predecessor and logical-key relations;
- certified state and access kind;
- request, operation, binding, and contract membership;
- authorization before observation;
- one observation per authorization;
- unmatched authorizations;
- late audit observations after closure;
- closed returned, safe-failure, and non-domain-fault outcomes;
- fault code, derived origin/class, and entry status;
- observation provenance consumed by a transition;
- typed nonce-reservation output provenance;
- signed-candidate proof and transaction-hash provenance;
- exact nonce-completion and terminal-evidence provenance; and
- the non-consumability of non-domain faults.

Exact reproduction may rerun deterministic planning and pure state callbacks. It never invokes an
adapter, transport, provider, signer, nonce store, or run-history mutation path.

Run-scoped replay proves what the run authorized and observed. It does not independently reconstruct
the complete global nonce-allocation history unless a portable resource attestation is explicitly
included in the export contract.

## Failure And Crash Semantics

| Boundary | Durable run fact | Required behavior |
| --- | --- | --- |
| Preparation fails | Prior verified history only | No authorization and no live entry. |
| Authorization append is rejected or stale | No new authorization | Reload and rederive; do not invoke. |
| Authorization append acknowledgement is ambiguous | Authorization may exist | Resolve the original append identity; mint no authority from ambiguity. |
| Authorization positively commits | `ExternalAccessAuthorized` | Mint one affine authority for the exact operation. |
| Process dies before invocation | Unmatched authorization | Preserve ambiguity; re-enter only under G-09. |
| Process dies during invocation | Unmatched authorization | Preserve possible entry; never fabricate a result. |
| Adapter returns a value or error | Authorization plus pending value in memory | Convert to one closed pending observation before any normal return. |
| Observation loses an exact-head race | Authorization plus stable pending value | Rebase the physical append without reinvoking. |
| Observation acknowledgement is ambiguous | Observation may exist | Resolve the unchanged original append before rebase. |
| Journal is unavailable after adapter return | Pending value remains in the live task | No normal successful drive result. |
| Process dies after adapter return, before observation commit | Unmatched authorization; pending value lost | Preserve ambiguity; use only qualified exact re-entry. |
| Returned or safe-failure observation commits | Linked `ExternalAccessObserved` | Reload and settle this node; another bounded poll or replacement is a different node. |
| Operational non-domain fault commits with no overlapping consumable observation | Linked audit-only fault | Block; do not authorize another invocation from the fault. |
| Integrity non-domain fault commits before settlement | Linked audit-only fault | Block even if an overlapping invocation returned a value. |
| Process dies before state transition | Committed observation | Recompute settlement without live access. |
| Observation arrives after closure | Fixed semantic closure plus audit tail | Record only if linked to a legal pre-closure authorization. |
| Nonce-store acknowledgement is ambiguous while task lives | Runtime authorization; resource operation may exist | Resolve the original resource operation internally; do not normally return a fault. |
| Nonce reservation commits but its run observation is lost | Unmatched authorization plus durable resource reservation | Exact re-entry returns the identical reservation. |
| EVM submission may have entered and observation is lost | Unmatched effect authorization | Authorize only the exact identical qualified submission. |
| Finite EVM graph exhausts without terminal evidence | Committed nonterminal observations and durable reservation | Report `BlockedUnresolved`; do not poll, complete, or fabricate a terminal result. |

The row immediately after adapter return is an explicit acceptance test for the forensic baseline.
It proves that normal execution cannot escape observation while process loss remains visible as
unmatched authorization.

## Enforcement

The cutover must make these properties structural:

- `RunHistoryWriter` is non-cloneable and consumed by Runtime assembly.
- live adapters cannot depend on or construct the run-history writer.
- raw PostgreSQL mutation capability is private to qualified storage assembly.
- `Prepared<K>`, `Authorized<K>`, `PendingObservation<K>`, and
  `CommittedObservation<K>` have private fields and sealed constructors.
- only a positively new authorization append creates `Authorized<K>`.
- Runtime dispatch is the only path from authorization to a registered invoker.
- invokers have no outer error after accepting authority.
- after authorization, state settlement accepts only `CommittedObservation<K>`; local settlement
  is pre-authorization only.
- Runtime/kernel derives stable call and reservation keys; adapters and state code cannot choose
  them.
- effect operations cannot be registered without a manifest-bound, qualification-proved exact
  convergence contract.
- a committed non-domain fault cannot cause another invocation.
- a committed returned or safe-failure observation cannot cause implicit same-node polling.
- `ReservedWalletNonce` consumers require exact producer and binding provenance.
- nonce completion requires sealed reservation and terminal-evidence provenance.
- all nonce-reserving workflows use the same qualified adapter.
- all capable sender actors use that nonce authority or are fenced out.
- run-history and nonce database roles cannot mutate each other's authority.
- adapters contain no history fold, next-plan selector, retry loop, or terminalizer.
- replay and applications receive read-only purpose capabilities.

Architecture scans supplement, but do not replace, type privacy, crate dependency checks,
compile-fail tests, and fault injection.

## Why Not Encode Every Audit Step As A State?

Authorization and observation surround external IO:

```text
authorization committed
  -> external operation
  -> observation committed
```

Renaming those records `StateTransitionCommitted` cannot make them atomic or guarantee the second
append. It either creates no-op semantic transitions or puts worker timing and crash phases into
domain state.

The journal deliberately has separate heads:

- the journal head advances for every durable audit fact; and
- the semantic head advances only for a state transition.

Late observations after semantic closure make that distinction unavoidable.

Meaningful operations should be small states. Runtime mechanics should remain private typed
protocol phases.

## Why Not Keep A Generic Executor?

A separate durable coordinator would be justified for a product that requires:

- arbitrary non-idempotent and non-queryable destinations;
- progress independent of a run being driven;
- cross-run coalescing of one logical delivery;
- an independently operated delivery service; or
- open-ended scheduling that cannot be represented by the certified graph.

Those are not current MFM requirements. Keeping a generic executor for hypothetical use imposes a
second interpreter, history, fence, recovery algebra, and public type system on every effect.

If a future product needs those properties, it should integrate an explicit external service or
propose a new authority boundary. It must not grow an adapter into another hidden Runtime.

## Why Not A Universal Outbox?

A universal outbox close enough to every provider and read source could preserve stronger
forensics across process loss. It would also require:

- another durable writer or destination participant;
- fencing and ambiguous-commit recovery for that writer;
- retention of every read and provider completion;
- broader secret and data-minimization review; and
- a second operational availability dependency.

The baseline instead records authorization before entry and every normal completion afterward.
Unmatched authorization is the honest state after loss. Exact effect re-entry is admitted only
when separately qualified safe.

This choice must be tested by killing the process immediately after the adapter wrapper returns
and before run observation commit. The expected result is unmatched authorization, not recovered
return data.

## Persisted Contract And Cutover Boundary

This target changes persisted semantics:

- executor `ensure` becomes direct `Effect` access;
- executor-retained closure is removed;
- `EffectRequested` and `EffectSettled` become one `StateSettled` transition;
- node phase loses `AwaitingEffect`;
- non-domain failure vocabulary becomes `AccessNonDomainFaultCode`;
- effect authorization carries exact repeat-safety and request binding;
- transitions consume direct effect observations; and
- nonce reservation evidence gains exact producer and resource-domain provenance.

The eventual implementation therefore requires one new sole current schema lineage, annex, corpus,
store/replay validation, and public projection. Existing executor histories cannot be silently
reinterpreted.

No dual reader, dual writer, fallback executor, compatibility alias, or parallel direct-effect path
is part of this RFC.

## Design Cutover Scope

This is architectural scope, not an implementation sequence.

The completed cutover must:

- delete `crates/kernel/executor`;
- delete executor file and PostgreSQL storage crates and schemas;
- delete `RecoverableEffectExecutor`, `AuthorizedEnsureAccess`, executor registration, executor
  expansion, executor manifests, ledgers, frontiers, target authorities, resource policies,
  tombstones, and executor fences;
- remove EVM wallet history folding and next-plan selection from the live layer;
- add direct typed effect access under Runtime's existing private bracket;
- add explicit EVM states and typed dependencies;
- add the narrow wallet-nonce port and PostgreSQL adapter;
- keep raw PostgreSQL pools private and expose scoped qualified adapters;
- replace executor-specific fault layers and codes with direct access fault classification;
- remove executor-retained replay/export closure;
- update `docs/design.md`, `docs/architecture.md`, persisted-surface inventories, qualification,
  app wiring, CLI/REST projections, and all relevant README files;
- reset the sole current persisted contract deliberately; and
- delete all superseded dependencies, tasks, fixtures, migrations, tests, and terminology.

Nothing in this scope authorizes implementation before the material uncertainties below are
resolved.

## Verification Required Before Implementation Planning

The design must first be validated with small throwaway or test-only models, not a parallel
production path:

- express the complete current EVM transaction lifecycle as explicit `Pure`, `Read`, and `Effect`
  states, including every bounded poll, replacement, reorganization branch, and stable unresolved
  ending;
- prove deterministic signing and exact transaction-hash reproduction across restart;
- fault-inject process loss immediately after exact submission and verify identical re-entry;
- prove that a committed operational fault blocks and never triggers same-node re-entry;
- model concurrent runs reserving the same wallet domain and prove stable-key idempotency plus
  unique nonces;
- prove Runtime-derived reservation identity and reject caller-selected or substituted keys;
- model provider pending-nonce lag and disagreement without treating a read as a lock;
- prove `ReservedWalletNonce` cannot be substituted by a same-shape value from another producer,
  wallet generation, intent, or run;
- prove completion accepts only sealed terminal evidence for the same reservation and that run
  closure requires the matching `CompletedWalletNonce`;
- fault-inject resource acknowledgement ambiguity and prove no normal `MayHaveEntered` fault
  escapes from `reserve` or `complete`;
- prove non-rollback resource lineage, stale/sibling-writer exclusion, and high-water preservation
  across backup, restore, promotion, and writer-generation rotation;
- inventory every signer, relayer, operator, stale deployment, and direct-submit path and prove it
  uses the same nonce authority or is fenced out;
- prove separate database roles cannot cross-write run-history and nonce schemas and that no
  generic pool survives assembly;
- inventory every current executor predicate and assign it to certification, Runtime, store,
  nonce adapter, explicit EVM state, or deletion;
- show that no required predicate remains ownerless after executor deletion; and
- calculate the certified graph and journal bounds for the largest admitted replacement policy.

Only after those design checks pass should the repository define a logical commit sequence and
scope-driven verification plan.

## Material Uncertainties

| Choice or assumption | Why uncertain | Consequence if wrong | Resolution or validation |
| --- | --- | --- | --- |
| Direct EVM submission is exact-request convergent because the qualified signer reproduces identical signed bytes and transaction hash. | Signer implementations, remote signers, generation changes, and provider handling of an already-known transaction may differ. | Re-entry after an unmatched authorization could create a different transaction or strand an ambiguous one; the executor cannot be removed safely for direct submission. | Build deterministic signer goldens and provider-parity crash tests around the exact candidate. If the proof fails, require an authoritative idempotent wallet/relayer or a separate stronger-delivery RFC. |
| The complete current wallet policy can be expressed as a finite, comprehensible certified graph without a hidden reducer in an adapter. | Replacement bounds, polling, reorganization, and unresolved endings are currently encoded by wallet executor logic. | Graph size may become unreasonable, or next-action policy may leak back into the live layer under another name. | Prototype the maximum admitted operation graph and review every branch and bound before changing schemas. |
| A stable `BlockedUnresolved` open run is acceptable when the finite EVM policy has no terminal evidence. | The current executor can remain pending, but product and operator expectations for permanently unresolved runs are not yet established. | Runs and nonce reservations may remain open indefinitely, or implementation may be tempted to add an implicit polling loop. | Decide whether the product accepts a stable open block or requires a distinct typed terminal “unresolved” result or explicit administrative recovery operation; freeze that outcome before graph/schema work. |
| Run-scoped reservation provenance is sufficient for the baseline replay contract. | Cross-run uniqueness is a global property, while portable run export is intentionally run-scoped. | An offline export could prove that the run observed a reservation but not independently prove that no other run received the same nonce. | Decide whether portable global uniqueness proof is a product requirement. If it is, define a bounded resource attestation or resource-prefix closure before implementation planning. |
| Allowing later nonce reservations while an earlier reservation is incomplete is acceptable baseline product policy. | Serial submission avoids gaps, while concurrent pending nonces improve throughput; abandoned workflows exist in either model. | The chosen behavior may unnecessarily block a wallet or allow operationally difficult nonce gaps. | Model the current product's concurrency and recovery requirements, then freeze either concurrent monotonic reservation or one-incomplete-reservation gating in wallet-resource qualification. Never add timeout reuse. |
| The qualified nonce resource lineage can remain non-rollback and exclusive across writer rotation, restore, promotion, and every sender-capable actor. | The current executor uses a separately qualified fence, while the target introduces a narrower resource fence and may co-locate it with run history. | Rollback, a stale writer, or an out-of-band signer could reuse a nonce despite correct per-run history. | Produce an operational proof and failure-injection suite covering scoped database roles, lineage transfer, signer/relayer inventory, and direct-submit exclusion before removing the executor fence. |
| Existing deployed executor histories and reservations can be drained, exported, or reset outside the new production reader. | Repository policy permits a breaking cutover, but deployed state is not established by this RFC. | Removing the executor reader could strand active effects or lose operational evidence. | Inventory deployed environments before implementation planning and choose an explicit drain/export/reset procedure; do not add a compatibility reader to this design. |

## Current Decision Summary

The target architecture is:

- one run-interpreter code path and authority-owning type: `Runtime`, with multiple fenced workers
  allowed;
- one non-cloneable run-history mutation capability per qualified worker, held only by Runtime;
- one private typed authorization/invocation/observation bracket;
- three retained certified state kinds: `Pure`, `Read`, and `Effect`;
- one minimal `Invoke | Settle | BlockedUnresolved` access plan with no alternate-operation or
  retry variant;
- one terminal state-transition shape plus dependency skip;
- direct, closed, auditable effect adapter calls;
- no outer post-authorization error path;
- durable provider, transport, signer, adapter, and resource fault classification through
  `AccessNonDomainFaultCode`;
- no retry policy;
- committed non-domain faults never trigger re-entry;
- honest unmatched authorization after process loss;
- exact effect re-entry only under qualified safety;
- explicit, finitely bounded EVM poll, signer, broadcast, observation, and completion states;
- one narrow, non-rollback wallet-nonce authority with Runtime-derived keys and
  provenance-bound reservation/completion output;
- PostgreSQL infrastructure behind mutually isolated scoped authority-bearing adapters;
- no generic executor, executor ledger, executor fence, or hidden wallet driver;
- five distinct run-history record families;
- no universal outbox or stronger read forensics; and
- implementation planning deferred until the material uncertainties and validation checks are
  resolved.

This is simpler because it removes one interpreter rather than moving it. The remaining complexity
is visible at the boundary where its invariant is real: domain progression in states, run
recording in Runtime, protocol IO in adapters and transports, and cross-run uniqueness in the
wallet-nonce resource authority.
