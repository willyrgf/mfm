# RFC: Runtime History Choke Point

Status: accepted and implemented current recoverability-v3 contract

Scope: run-history ownership, runtime mutation authority, external-access orchestration, typed
completion, state-transition boundaries, store append APIs, replay, and application admission

This RFC records the accepted breaking internal architecture that enforces the external-access
audit contract established by `RFC_REFACTOR_RECOVERABILITY.md`. It is implemented by the current
code, `docs/design.md`, `docs/architecture.md`, and the sole production
`contracts/recoverability/v3` annex and corpus. The byte-identical v1 and v2 artifacts are archival
and hostile-input references only; no production reader accepts them.

This design does not weaken the existing requirements for append-only history, pre-access durable
authorization, per-append atomicity, affine live authority, content addressing, callback-free
recorded-history verification, independently durable effect execution, or no-secret persistence.
It closes the former gap in which physical persistence was centralized while the obligation to
persist was distributed across several runtime branches.

## Executive Decision

MFM keeps one active state-machine interpreter: `Runtime`.

`Runtime` is the sole production code-path and type owner of run-history mutation and authorized
live-access orchestration. Each process assembly consumes its mutation-capable history handle into
`Runtime` during construction. Application services, replay, trace, audit, export, CLI, and REST
retain purpose-specific read-only facades rather than a clone that can append.

This is not a deployment-wide singleton-writer claim. Independently assembled runtime workers may
hold their own non-cloneable mutation handles under the same qualified authoritative writer lineage.
Exact-head compare-and-swap and the deployment fence remain the cross-process concurrency
authority. The ownership rule is that only `Runtime` code can use such a handle, not that only one
`Runtime` value may exist.

The mutation handle may be represented by a private non-cloneable `RunHistoryWriter<S>`, or by the
same store plus a private non-cloneable mutation authority held directly by `Runtime`. If retained
as a type, `RunHistoryWriter<S>` is passive: it is analogous to an unforgeable file descriptor. It
owns no scheduling, state interpretation, live invocation, completion classification, cross-phase
retry, or semantic decision.

One drive reduces to:

```rust
let decision = self.derive_decision(authority, &view, admitted).await?;
match select_action(decision) {
    SelectedAction::Access { candidate, .. } => {
        self.perform_access(authority, &view, candidate).await
    }
    // The remaining closed variants commit local/settlement work or return waiting/closed.
}
```

For every live access, `Runtime` alone executes one sealed private protocol:

```text
Prepared<K>
  -> durably append authorization
  -> Authorized<K>
  -> invoke one registered boundary
  -> totalize one persistable result
  -> PendingObservation<K>
  -> durably append the linked observation
  -> CommittedObservation<K>
```

The diagram shows the fresh-authorization branch. Authorization may instead resolve as already
committed progress, a stale plan requiring re-derivation, or interruption. None of those outcomes
constructs `Authorized<K>` or invokes the boundary.

`K` is one sealed access kind: read, ensure, or fact selection. All expected fallible request,
registry, route, binding, contract, and codec qualification, including selection of the exact total
completion encoder, finishes while constructing `Prepared<K>`, before live authority exists. Only
a directly acknowledged new authorization append constructs `Authorized<K>`. Invocation and
totalization consume that affine authority and have no normal return other than one closed
`PendingObservation<K>`; there is no outer error channel. Applying the selected encoder happens
after invocation, and any surviving returned-value encoding or validation failure becomes one
non-domain failure. `PendingObservation<K>` is the non-escaping obligation to persist the exact
redaction-safe result. Only a positively committed or exactly resolved identical linked observation
constructs `CommittedObservation<K>`.

This is the useful form of breaking execution into smaller states: they are private runtime
authority states, mechanically composed by one interpreter. They are not certified domain graph
nodes and do not create another scheduler or configurable protocol language.

The five top-level record families remain distinct:

```text
RunAdmitted
StateTransitionCommitted
ExternalAccessAuthorized
ExternalAccessObserved
RunClosed
```

They represent different authority facts. Calling all of them “state transitions” would not force
runtime code through the required protocol and would erase the useful distinction between
semantic state and access audit.

The intermediate values are private, non-cloneable where authority or a persistence obligation is
involved, and cannot leave the runtime access protocol. A successful drive result after live access
can be constructed only from `CommittedObservation<K>`. Semantic settlement occurs only on a later
drive from a freshly loaded, callback-free `VerifiedRunView`, never from an in-memory
`PendingObservation<K>`.

The intended guarantee is:

> No surviving live result can escape its private pending-observation obligation, influence
> semantic state, or produce a successful drive result unless its exact linked observation was
> durably committed or resolved as already committed with identical content.

Process death and journal unavailability remain real failure boundaries. If no completion survives
to a committed observation, the prior unmatched authorization is the authoritative fact. This RFC
does not claim cross-system atomicity that a local Rust type system and local database cannot
provide.

## Resolved Problem

### The contract was stronger than the former ownership structure

The accepted recoverability design requires:

1. one durable authorization before every independently meaningful MFM-controlled live operation;
2. live authority only from a directly acknowledged new authorization append;
3. at most one live operation per affine authority;
4. one durable linked observation for every surviving wrapper result; and
5. semantic reduction only from committed observations.

The store strongly enforces the first, second, and fifth parts once the relevant append reaches it.
The runtime API also makes live authority non-cloneable and difficult to forge.

The remaining obligation is procedural:

```text
if an authorized wrapper returns,
runtime must not leave the access path before the observation commits
```

That obligation was implemented by ordinary control flow in several functions. The accepted
cutover moved it behind one type and interpreter boundary.

### Physical persistence was centralized but recording obligation was distributed

The pre-cutover run-history model had one sealed backend append seam:

```text
RunJournalBackend::backend_append
```

PostgreSQL and the in-memory backend implement that seam. The store validates the candidate,
assigns its sequence and hashes, binds immutable objects, and publishes the append atomically.

The production code that decided which candidate to construct was more distributed. The
meaningful pre-cutover authoring sites were:

1. application admission;
2. semantic transitions through the shared transition helper;
3. read authorization;
4. read observation;
5. effect-ensure authorization;
6. effect-ensure observation;
7. fact-selection authorization; and
8. fact-selection observation.

`RunClosed` is store-derived in the final transition commit and has no independent caller.

The physical write implementation is therefore centralized, but the answer to “must this
post-authorization result now become an observation?” is spread across read, effect, fact-scan,
erased-dispatch, encoding, and error-propagation paths.

### A normal Rust error could bypass the observation append

The current runtime-facing effect executor returns:

```rust
Future<Output = Result<EffectExecutorOutcome, ExecutorError>>
```

`EffectExecutorOutcome` is closed and persistable:

```text
Returned
DidNotEnter
Indeterminate
```

Those outcomes reach `ExternalAccessObserved`. The outer `ExecutorError`, however, can propagate
after the MFM authorization was committed and consumed. An ordinary `?` can therefore leave
runtime before observation construction.

The distinction is not safely expressed by the return type:

```text
closed effect failure       -> observation
outer executor error        -> possibly no observation
```

Some outer errors are genuine local integrity failures rather than domain evidence, but that does
not justify making the completed authorized interaction disappear. It means the persisted audit
contract needs a non-consumable integrity representation.

### The same structural risk existed outside effects

Reads already return a closed capability outcome, but later runtime work can still fail before
append:

- type-erased request or response checks;
- result or diagnostic encoding;
- contract or binding comparison;
- candidate callback selection; or
- observation preparation.

Fact selection has its own sequence:

```text
append authorization
  -> mint scan permit
  -> perform authoritative scan
  -> build completed scan
  -> prepare observation
  -> append observation
```

A scan or returned-scan conversion error after authorization can also leave that path without a
linked observation. Failure of the later run-history append is a distinct persistence boundary:
it cannot be recursively represented by another observation in the same unavailable or
integrity-blocked history.

Some of these exits may correctly represent interruption rather than a surviving completion. The
problem is that this classification is made implicitly by each branch instead of exhaustively by
one protocol owner.

### Store failure after a live result is a separate crash boundary

Even a fully closed capability result does not make its subsequent journal append infallible.
After the external call:

- the journal head may have changed;
- the append acknowledgement may be ambiguous;
- the journal writer may be unavailable;
- candidate verification may find corruption; or
- the task or process may be cancelled before commit.

While the process retains the sealed outcome, stale-head and acknowledgement ambiguity can be
resolved without invoking the live operation again. If the process disappears, no local type can
write the lost value after death.

The correct durable representation is then the already committed unmatched authorization. For a
recoverable effect, the independently durable keyed executor may retain enough target evidence to
return it on a later `ensure`. A read may require a new, separately authorized attempt.

The contract must distinguish:

```text
normal completed access step
    observation must commit before success can escape

interrupted access step
    unmatched authorization remains authoritative
```

### State transitions are not the enforcement point

`StateTransitionCommitted` records semantic truth:

- before and after semantic state;
- exact inputs and lineage;
- selected committed evidence;
- typed result, output, facts, or domain failure; and
- the resulting node and run phase.

An authorization grants live authority but changes no semantic state. An observation records audit
evidence but may be insufficient, invalid, late, or permanently non-consumable. It also changes no
semantic state by itself.

Using the semantic transition path as the only write point would therefore require either:

- performing the external operation before durable authorization;
- pretending an unknown result is already known;
- holding a database transaction over external IO;
- adding artificial no-op semantic transitions;
- making audit attempt count affect semantic state; or
- redefining “transition” to mean every history record, which only renames the current algebra.

None of those choices makes observation persistence structurally mandatory.

### Former affine types proved ordering but not complete consumption

The current `NewlyAppendedAuthorization` and `AuthorizedReadAccess` /
`AuthorizedEnsureAccess` types prove important facts:

- live access cannot be minted from a stale or ambiguous authorization append;
- an access authority cannot be cloned;
- the exact request and authorization stay bound; and
- state callbacks never receive live authority.

Rust values are affine rather than linear. An affine value may be dropped. `#[must_use]` adds a
lint, not an execution proof. A private type-state sequence helps only if intermediate values never
escape into branches that may return early.

The stronger design must combine:

- private construction;
- exclusive ownership;
- a closed action algebra;
- a closed persistable outcome algebra;
- one non-escaping pending-observation obligation after every surviving invocation;
- no outer live-boundary or returned-value-totalization error channel;
- one interpreter that owns every phase; and
- successful return types constructible only from committed evidence.

## Goals

- Make `Runtime` the sole active interpreter and exclusive production code-path owner of every
  run-history mutation while preserving independently assembled workers under the same qualified
  writer lineage.
- Separate active orchestration from passive mutation-capability custody.
- Make every registered live operation pass through one authorization/invocation/observation
  protocol composed once for read, ensure, and fact selection.
- Constrain each in-process authority phase through sealed consuming types and reconstruct the
  durable protocol position from verified history after restart.
- Make an unrecorded normal completion unrepresentable outside that bracket.
- Preserve a durable authorization before every possible external boundary entry.
- Preserve one exact linked observation for every completion that survives to a normal runtime
  completion.
- Preserve honest unmatched authorization for task loss, process loss, or unresolved persistence.
- Move every fallible request, route, binding, type, codec, and contract precondition before
  authorization where possible.
- Replace every surviving live-boundary or returned-value totalization error with a closed,
  stage-aware, redaction-safe pending observation.
- Keep retryable operational faults and integrity/contract violations auditable but impossible for
  state logic to consume.
- Preserve one stable, queryable classification for every normally returned provider or transport
  fault through either its certified `SafeFailure` contract or the closed non-domain fault
  code/context relation, without retaining provider-controlled diagnostics.
- Keep retry legality in the persisted disposition while allowing a future qualified scheduling
  policy to consult the closed fault code after commit; every retry remains a new authorization.
- Distinguish one observation's stable logical identity from each predecessor-bound physical append
  attempt.
- Let semantic transitions consume only freshly verified committed observations.
- Keep state callbacks pure and independent of store, journal, scheduler, and live authority.
- Keep effect delivery and resource coordination in the separately fenced executor ledger.
- Keep store append legality, atomicity, hashing, object binding, and structural folding in the
  store.
- Route run admission through the same mutation owner so application code has no parallel append
  path.
- Retain one current design and delete superseded APIs without compatibility paths.
- Reduce mutation call sites, public types, duplicated outcome wrappers, and future change sites.
- State the strongest guarantee the architecture can honestly provide without claiming
  cross-system atomicity.

## Non-Goals

- Making a local journal and a remote system one atomic transaction.
- Proving that every authorization caused a physical remote request.
- Proving that every physical packet or provider-side action was observed.
- Persisting a value after the task, process, and every durable participant have lost it.
- Holding a PostgreSQL transaction open across network, signer, wallet, filesystem, or executor IO.
- Giving capabilities, adapters, transports, or executors general run-history append authority.
- Moving external IO into state callbacks.
- Turning runtime authorization, retry, observation, or persistence phases into domain behavior.
- Pre-expanding an unbounded number of access attempts into a finite certified graph.
- Making audit attempt count, latency, worker identity, or retry timing part of semantic state.
- Treating a persisted fault code as an instruction to retry, or permitting an adapter to retry or
  fail over invisibly inside one authorization.
- Absorbing executor delivery or resource streams into the MFM run history.
- Persisting raw provider text, bodies, paths, endpoints, credentials, signatures, signed
  envelopes, or arbitrary diagnostics.
- Treating `#[must_use]`, comments, code review, or architectural convention alone as the safety
  mechanism.
- Adding best-effort audit mode or an unaudited fallback when the history store is unavailable.
- Retaining recoverability v1 or v2 as a production reader after the v3 persisted-schema cutover.

## Terminology

**Run history**
: The append-only authoritative sequence of admission, semantic transitions, external-access
  authorizations, external-access observations, and structural closure for one run.

**Semantic transition**
: One deterministic change to the certified node/run semantic state. It may consume committed
  evidence but never performs ambient IO.

**Access protocol**
: `Runtime`'s sealed authorization, one affine live invocation, completion, and observation
  persistence sequence.

**`Prepared<K>`**
: A private operation value whose request, route, binding, types, codecs, contracts, and immutable
  identities were validated before authorization.

**`Authorized<K>`**
: Affine authority for at most one exact registered application-protocol operation.

**`PendingObservation<K>`**
: The private non-cloneable obligation created after one authorized invocation and totalization. It
  binds the exact authorization to one complete redaction-safe persistable outcome. It is not
  semantic authority, cannot be passed to state logic, and must remain owned by the access bracket
  while physical append attempts are prepared, retried, or resolved.

**`CommittedObservation<K>`**
: Store-verified proof that one pending observation was durably appended, or that an already
  committed observation under the same authorization has exactly identical content.

**Non-domain failure**
: A reviewed audit-only outcome for a surviving operational, protocol, integrity, or contract
  fault. It carries conservative entry status, one closed disposition
  (`RetryableOperational | IntegrityBlocked`), and one closed redaction-safe code. State callbacks
  never receive it.

**Non-domain failure code**
: `NonDomainFailureCode`, the current concrete type for the RFC-level access non-domain fault
  classification. Its variant plus the history-derived `NonDomainFailureLayer` preserve one closed,
  queryable origin/category. Each legal code/layer/status/disposition relation is frozen; there is
  no open string, provider message, or unknown fallback.

**Logical observation identity**
: The exact authorization reference plus the complete persistable observation content and digest.
  It remains stable across head changes.

**Physical append attempt**
: One append request id, predecessor, candidate digest, and candidate body. It remains stable while
  resolving acknowledgement ambiguity but is replaced after a definite stale-head rejection.

**Passive mutation capability**
: A private non-cloneable writer handle or mutation authority consumed by one assembled `Runtime`.
  It can reach store preparation and append operations but makes no scheduling, invocation,
  classification, retry, or semantic decision. Other processes may assemble their own runtimes
  under the same qualified authoritative writer lineage.

**Interruption**
: Loss of the executing task/process or inability to complete the observation protocol. Its durable
  run-history representation is an unmatched authorization, not a fabricated completion.

**Choke point**
: The sole code and type boundary capable of transforming a planned run action into a run-history
  append or an authorized live invocation.

## Required Guarantees

### G-01: One mutation owner

Only `Runtime` may orchestrate or submit run-history mutations in production execution. Its
scheduler and decision helpers cannot access the passive writer directly. Capability adapters,
executor adapters, application services, replay, CLI, and REST have no mutation path.
This is a code-path ownership invariant, not a deployment-wide singleton: multiple runtimes may
append concurrently under store compare-and-swap and the same qualified writer fence.

### G-02: Authorization precedes possible entry

Every direct MFM-controlled live application-protocol operation has one committed
`ExternalAccessAuthorized` before any possible boundary entry. Only a directly acknowledged new
append can mint live authority.

### G-03: One affine invocation

Each fresh authorization can produce at most one private `Authorized<K>`, and that authority
can enter at most one registered application-protocol operation.

### G-04: Closed surviving outcome

Once live authority is consumed, every normal runtime-facing adapter return is one exhaustive
closed kind outcome, which the private adapter totalizes and binds into `PendingObservation<K>`.
No capability or executor trait has an outer error channel that can bypass outcome classification
or pending-observation construction.

### G-05: No pending observation escape

`PendingObservation<K>` remains private to the runtime access-protocol module. It cannot reach state
logic, application output, or a successful `drive_once` result. After live invocation, the only
successful protocol result is `CommittedObservation<K>`. A non-invoking `Advanced` result may
report an authorization that was already committed.

### G-06: Exact observation linkage

Every committed observation references the exact authorization whose affine access produced it.
At most one observation is legal per authorization. Runtime resolves that logical key before each
physical attempt. Identical committed content constructs committed proof; changed content
conflicts. A stale predecessor creates a new physical append attempt over the same pending
observation, while acknowledgement ambiguity resolves the unchanged original attempt.

### G-07: Non-domain failures are audit-only

Every surviving operational, protocol, integrity, or contract failure produced by the live
boundary or returned-value totalization becomes a closed `NonDomainFailure` with conservative
entry status, closed disposition, and `NonDomainFailureCode`. The code plus its history-derived
`NonDomainFailureLayer` preserves the reviewed origin/category without provider-controlled text.
It cannot yield normal success until that observation commits, and it cannot become a domain
failure, accepted result, or state-consumable safe failure. `RetryableOperational` permits, but
does not command, a later separately authorized attempt after commit; `IntegrityBlocked`
deterministically blocks. A future qualified scheduler may use the committed code to select
operational policy, but the code cannot mint authority, override the disposition, or cause an
invisible adapter retry. Journal corruption, journal unavailability, an unresolved append, or a
changed-content observation conflict is instead a history-persistence interruption and may prevent
any new observation from being committed; in those cases no pending value escapes and durable
history remains authoritative.

### G-08: Process loss is not fabricated

If no completion survives, runtime does not invent `DidNotEnter`, `Indeterminate`, cancellation, or
non-domain failure. The unmatched authorization remains the complete MFM fact.

### G-09: Semantic consumption requires committed proof

State settlement receives only observation views reconstructed from a fresh callback-free
`VerifiedRunView`. An in-memory pending observation or observation candidate has no semantic
authority.

### G-10: Store validation remains authoritative

`Runtime` interprets the already-derived action. The store still owns append legality,
predecessor comparison, idempotency, object binding, hashes, fact coordinates, closure, and
structural verification.

### G-11: Executor authority remains independent

The run history audits the MFM-to-executor `ensure` call. The executor ledger separately owns
effect binding, resource allocation, target-attempt authorization, target observation, retained
frontier, and terminal tombstone under its independent fence.

### G-12: No-secret persistence

Every surviving outcome is converted below the history boundary into a reviewed typed value or
closed safe metadata. Provider-controlled text and secret-bearing material have no persisted
representation.

## Accepted Architecture

### One active owner: `Runtime`

`Runtime` is both the state-machine driver and the sole active run-history mutation code-path
owner:

```rust
pub struct Runtime<S> {
    writer: RunHistoryWriter<S>,
    program_registry: Arc<QualifiedProgramRegistry>,
}
```

`RunHistoryWriter<S>` is illustrative rather than a required public type. The same invariant may be
implemented by storing `S` and a non-cloneable mutation authority directly in `Runtime`. What
matters within each process assembly is:

- construction consumes that assembly's production mutation capability;
- `Runtime` never exposes or clones it;
- scheduler and decision helpers receive immutable verified views, not the writer;
- app, replay, trace, audit, export, CLI, and REST receive purpose-specific readers; and
- only the private runtime execution module can reach mutation operations.

Another process may assemble another `Runtime` under the same qualified writer lineage. The
non-cloneable handle prevents parallel mutation ownership inside one assembly; it does not replace
the deployment fence or exact-head compare-and-swap.

The writer is passive. It may give `Runtime` access to store preparation and append operations, but
it does not schedule, invoke, classify, retry a multi-phase protocol, or interpret semantic state.
All active orchestration remains in `Runtime`; store legality remains in `mfm-store`.

`Runtime` owns:

- the drive loop and next-step derivation;
- state callback dispatch;
- append-request identity derivation;
- admission, transition, authorization, and observation orchestration;
- access preflight;
- fresh-authorization witness consumption;
- affine live-access construction;
- registered invoker dispatch;
- exhaustive totalization into closed persistable outcomes;
- post-invocation pending-observation retention and lossless physical append retry without
  reinvocation; and
- the rule that normal access progress cannot escape before observation commit.

`Runtime` does not own:

- graph authorship or certification;
- domain request or settlement semantics;
- provider-specific transport semantics;
- executor delivery or resource policy;
- low-level executor failure-stage knowledge;
- store candidate legality, hashing, atomicity, or folding;
- replay policy;
- application authentication; or
- public rendering.

The runtime-facing capability or executor adapter owns the translation from its internal failures,
actual affine-authority consumption, and exact boundary stage into one closed redaction-safe outcome.
The private runtime adapter totalizes that value into `PendingObservation<K>`. `Runtime`
exhaustively persists that obligation; it does not infer entry status or retry disposition from an
error name.

### One closed planned-step algebra

Next-action derivation remains pure over a `VerifiedRunView` and produces one closed plan:

```rust
enum PlannedRunStep {
    CommitTransition(TransitionPlan),
    ExecuteAccess(PreparedAccess),
    Closed,
    Waiting(WaitReason),
}

enum PreparedAccess {
    Read(Prepared<Read>),
    Ensure(Prepared<Ensure>),
    FactSelection(Prepared<FactSelection>),
}
```

The planned access form is a private closed concrete sum, not an erased lifecycle object. It has no
`execute` hook and cannot supply a custom append lifecycle. The runtime's private selected-action
match dispatches all three variants through the same sealed stage protocol and shared generic
observation-commit bracket. Read, ensure, and fact selection retain kind-specific preparation,
typed material, and registered invocation.

`drive_once` remains the sole active driver:

```rust
pub async fn drive_once(
    &self,
    authority: RunAccessAuthority<Drive>,
) -> Result<DriveOutcome> {
    loop {
        let view = self.load_verified(&authority).await?;
        let decision = self.derive_decision(&authority, &view, admitted).await?;
        let result = match select_action(decision) {
            SelectedAction::Access { candidate, .. } => {
                self.perform_access(&authority, &view, candidate).await?
            }
            // The remaining closed variants handle settlement, local work, or waiting.
        };
        match result {
            ActionResult::Outcome(outcome) => return Ok(outcome),
            ActionResult::Retry => {}
        }
    }
}
```

The pseudocode is ownership-oriented, not a commitment to borrowing or allocation details.

The selected-action match inside `drive_once` is the sole private interpreter of this conceptual
algebra. A genuine head advance before invocation invalidates the plan and retries the runtime loop
against a freshly loaded view. It may resolve acknowledgement ambiguity for the identical append
identity without invoking.

After a live invocation, the rule is different: the kind-specific runtime access method retains the
sealed `PendingObservation<K>` and delegates to `commit_observation<K>`, which reloads as required
and retries only its exact linked observation. It may not return the pending value for re-planning
and may not invoke the external operation again.

### Transition execution

`CommitTransition` contains one of the existing semantic transition materials:

```text
PureSettled
ReadSettled
EffectRequested
EffectSettled
DependencySkipped
```

`Runtime`:

1. derives the append request identity;
2. asks the store to prepare and validate the transition;
3. appends it under exact-head compare-and-swap;
4. resolves identical-append idempotency and acknowledgement ambiguity;
5. returns `StepResult::Replan` if a genuine head advance invalidates the plan; and
6. otherwise returns only the committed result.

If the transition closes the run, the store derives `RunClosed` and includes it in the same atomic
commit. No runtime caller can prepare standalone closure.

### Access preparation

All checks that can be completed before authorization must happen while constructing
`Prepared<K>`:

- exact certified node and execution kind;
- registered operation lookup;
- request type and downcast;
- request canonical encoding;
- request content and semantic digest;
- capability or executor binding;
- returned and safe-failure contracts;
- routing-generation selection and admission;
- input-manifest and request-transition reference;
- effect key and request digest;
- fact-selection contract and barrier prerequisites; and
- selection and qualification of the complete total encoder from a closed boundary return to
  producer-free observation material.

This prevents a known local catalog or encoding mismatch from being discovered only after live
authority has been minted.

Preparation may fail with an ordinary typed runtime error because no live authority exists and no
external boundary can have been entered.

The selected encoder is applied only after invocation because its input does not yet exist during
preflight. Any surviving returned-value encoding, identity, or contract failure must become
`NonDomainFailure { disposition: IntegrityBlocked, .. }`; it cannot reintroduce an outer
invocation-or-totalization error.

### Typed access protocol

The private protocol is deliberately small:

```rust
trait AccessKind: sealed::Sealed {
    type Request;
    type Returned;
    type SafeDiagnostic;
}

struct Prepared<K: AccessKind> {
    // Exact immutable request and all qualified preflight proofs.
}

struct Authorized<K: AccessKind> {
    // Affine authority scoped to one invocation.
}

enum PersistableAccessOutcome<K: AccessKind> {
    Returned(PersistableReturned<K>),
    DidNotEnter(PersistableSafeFailure<K>),
    Indeterminate(PersistableSafeFailure<K>),
    NonDomainFailure {
        entry_status: NonDomainEntryStatus,
        disposition: NonDomainDisposition,
        code: NonDomainFailureCode,
    },
}

struct PendingObservation<K: AccessKind> {
    // Exact authorization reference plus one complete persistable outcome.
}

struct CommittedObservation<K: AccessKind> {
    // Private constructor and exact committed observation reference.
}

```

Only a directly acknowledged new authorization append can consume `Prepared<K>` and construct
`Authorized<K>`. An identical already-committed authorization advances without invocation; a
genuine head advance or closure retries derivation; and a store failure or acknowledgement
ambiguity before fresh authority returns a reviewed `RuntimeError`. None of those non-fresh
outcomes can invoke.

Invocation plus totalization consumes `Authorized<K>` and has no normal return other than
`PendingObservation<K>`. Observation persistence retains `PendingObservation<K>` across physical
append attempts until it can construct `CommittedObservation<K>` or the task is interrupted.

The concrete type erasure needed inside a heterogeneous registered invoker remains private to
invocation and totalization. It does not represent a planned access, cannot customize the
lifecycle, and may not add another fallible invocation-or-totalization result channel.

### One reusable access composition

`perform_access` exhaustively dispatches the closed `PreparedAccess` sum. Each private kind-specific
method performs the same fixed order and delegates persistence to the one generic
`commit_observation<K>` implementation.

The stage operations consume their inputs:

```text
authorize:           Prepared<K>           -> Fresh(Authorized<K>) | Advanced | Replan | RuntimeError
invoke_and_totalize: Authorized<K>         -> PendingObservation<K>
commit_observation: PendingObservation<K>  -> CommittedObservation<K> | exact conflict
```

The awaits and their order are irreducible: authorization must physically commit before IO, and
observation can begin only after the invocation returns. Typing does not remove that causality. It
makes the one implementation reusable and prevents other code from expressing an invalid order.

The implementation must preserve `PendingObservation<K>` while preparing and resolving physical
observation append attempts. It must not repeat the live invocation after a stale observation
append or acknowledgement ambiguity.

The boundary between the two failure classes is deliberate:

- `NonDomainFailure` totalizes a surviving live-boundary return or the producer-free conversion of
  that return into persistable material; and
- before authority exists, a reviewed runtime/store error may return without a live result;
- after `PendingObservation<K>` exists, transient load, verification, preparation, append, and
  acknowledgement failures remain internal retry state. Only exact changed-content conflict
  returns; task or process loss drops the in-memory obligation and leaves the durable unmatched
  authorization.

A journal failure must not be recursively converted into `NonDomainFailure` in the journal it
cannot safely append. Conversely, a live adapter or returned-value encoder must not label an
ordinary return as an outer runtime error merely to bypass pending-observation construction.

### Fixed state-kind composition

The runtime does not need a general-purpose `Then<A, B>` DSL, free monad, configurable protocol
graph, or second scheduler. The certified execution kind selects one fixed lifecycle:

```text
Pure
  = Commit<PureSettled>

Read<K: Read | FactSelection>
  = Repeat<RecordedAccess<K>>
  ; Commit<ReadSettled>

Effect
  = Commit<EffectRequested>
  ; Repeat<RecordedAccess<Ensure>>
  ; Commit<EffectSettled>
```

`RecordedAccess<K>` is the sealed
`Prepared<K> -> Authorized<K> -> PendingObservation<K> -> CommittedObservation<K>` protocol.
`Repeat` is value-level scheduling over committed history: an insufficient observation or a
committed `RetryableOperational` non-domain failure may permit another separately authorized
attempt. It is not a statically pre-expanded graph and does not expose retry policy to domain state
authors.

Each drive reconstructs the current protocol position from `VerifiedRunView` and performs at most
one selected action. Rust types constrain the in-process authority sequence. Store and replay
validation constrain the persisted sequence across restart. Domain state implementations continue
to provide only pure request authorship and committed-evidence settlement.

### Closed runtime-facing invocation

The read capability already approximates the desired boundary:

```rust
fn call(
    AuthorizedReadAccess<Request>,
) -> Future<Output = ReadCapabilityOutcome<Response, Diagnostic>>
```

The effect boundary must change from:

```rust
fn ensure(
    AuthorizedEnsureAccess<Request>,
) -> Future<Output = Result<EffectExecutorOutcome, ExecutorError>>
```

to a closed stage-aware completion:

```rust
fn ensure(
    AuthorizedEnsureAccess<Request>,
) -> Future<Output = EffectEnsureCompletion>
```

Concrete executors may continue using `ExecutorError` internally. The runtime-facing adapter must
exhaustively project every surviving internal error according to actual authority and receipt
custody:

- `DidNotEnter` only when target entry is proven impossible;
- `Indeterminate` when target entry may have occurred and the failure is an admitted
  state-consumable safe failure;
- `NonDomainFailure { disposition: RetryableOperational, .. }` for a surviving operational failure
  that state must not consume but a later drive may retry under a new authorization;
- `NonDomainFailure { disposition: IntegrityBlocked, .. }` for identity, contract, corruption, or
  integrity failures that state must not consume; and
- no completion for abort, panic, task loss, or process disappearance.

An error variant name alone is insufficient proof of boundary stage. Classification must use the
executor's actual protocol position. The private runtime adapter binds the authorization reference
to this closed value and constructs `PendingObservation<Ensure>`.

### Audit-only non-domain failures

The current observation outcomes are:

```text
Returned
DidNotEnter
Indeterminate
```

The accepted outcome algebra adds:

```text
NonDomainFailure {
    entry_status:
        ProvenNotEntered
      | MayHaveEntered,

    disposition:
        RetryableOperational
      | IntegrityBlocked,

    code:
        NonDomainFailureCode
}
```

`NonDomainFailure` means:

- an authorized invocation produced a surviving operational, protocol, integrity, or contract
  failure that is not domain evidence;
- the qualified adapter derived a conservative entry-status claim from actual affine-authority
  consumption and boundary progress;
- the qualified adapter selected a closed disposition from the reviewed fault taxonomy;
- the state callback never receives it as ordinary evidence; and
- replay reproduces its retryable-operational or integrity-blocked projection without invoking
  callbacks or live code.

`RetryableOperational` is non-consumable but permits a later separately authorized attempt after
the observation commits. `IntegrityBlocked` is non-consumable and deterministically blocks the run.
Neither disposition fabricates a domain failure or safe failure.

The current recoverability-v3 code vocabulary is:

```text
NonDomainFailureCode =
    AdapterContractViolation
  | ResultEncodingFailure
  | FactStoreUnavailable
  | FactHistoryInvalid
  | ExecutorStoreUnavailable
  | ExecutorContention
  | ExecutorHistoryInvalid
  | ExecutorCapacityExhausted
```

The record context derives one closed validation layer:

```text
NonDomainFailureLayer =
    Read
  | Fact
  | Ensure
  | ExecutorTarget
```

The layer is not a caller-authored diagnostic field. Store and replay derive it from the
authorization/observation context and reject any code that is illegal for that layer. The code
vocabulary must remain closed, bounded, and free of provider text, paths, endpoint details,
credentials, response bodies, source chains, or arbitrary debug data.

Store and replay validate the closed code, status, disposition, binding, and permitted
code/status/disposition relation. They do not independently prove whether a remote target was
physically entered.

#### Provider and transport trackability

Every normally returned external provider or transport fault has exactly one reviewed route:

- if the certified capability contract admits it as state-facing evidence, it becomes
  `DidNotEnter` or `Indeterminate` with that `SafeFailure` contract's stable code,
  `FailureClass`, `BoundaryStage`, and optional bounded typed diagnostic reference; or
- otherwise it becomes audit-only `NonDomainFailure` with `NonDomainFailureCode`, conservative
  entry status, and fixed disposition.

The linked authorization already identifies the exact qualified operation, request, and binding.
Together, that linkage and the closed failure classification make the incident queryable without
persisting an endpoint, provider message, response body, source chain, or arbitrary diagnostic
map. A normally returned provider or transport error may not use an outer `Result` to bypass both
routes. Panic, abort, task loss, and process loss return no classification and remain an unmatched
authorization.

`NonDomainDisposition` is the authority-level retry gate. `RetryableOperational` only makes a later
freshly authorized attempt legal; it does not require immediate retry. The closed code and layer
may support future qualified choices such as backoff or circuit breaking after the observation
commits, but they cannot override `IntegrityBlocked`, alter replay, reuse an authorization, or hide
another provider call inside the original attempt.

The design uses a distinct outcome rather than reusing `DidNotEnter` or `Indeterminate`, because
certified state policy may consume an admitted safe failure. Operational platform faults and
integrity corruption must not become domain evidence through that path.

### Observation persistence

Once `Runtime` holds `PendingObservation<K>`, observation persistence is mandatory while that task
continues. Two identities remain distinct:

```text
logical observation:
  authorization_ref + exact persistable content and digest

physical append attempt:
  append_request_id + predecessor + candidate digest and body
```

Runtime:

1. retains the immutable pending observation independently of any prepared store candidate;
2. loads the fresh verified head;
3. resolves the authorization's observation logical key;
4. on identical committed content, constructs `CommittedObservation<K>`;
5. on different committed content, reports an integrity conflict without reinvoking;
6. when absent, prepares one physical append attempt against the current head;
7. on a definite stale-head rejection, discards only that physical attempt, reloads, and prepares a
   new attempt over the same pending observation;
8. on acknowledgement ambiguity, retains and resolves the unchanged original physical attempt
   before any rebase;
9. on a positive commit, constructs `CommittedObservation<K>`; and
10. never exposes the pending observation as semantic or public data before commit.

Store preparation must borrow or derive from stable pending material without consuming the sole
copy needed for retry. A consumed affine kind-specific result, especially `CompletedFactScan`,
must first totalize into immutable sealed pending-observation material before any predecessor-bound
candidate is built.

If the store remains unavailable, the task cannot truthfully report a successful access step.
Cancellation or process loss may drop the in-memory pending observation, leaving the authorization
unmatched.

Absolute persistence across that loss would require a second durable outbox or direct durable
participation by the capability. That stronger design is discussed under alternatives and material
uncertainties.

### Read recovery

For a read:

- a committed observation can be evaluated by state;
- an unmatched authorization proves only that zero or one read may have occurred;
- recovery never fabricates the lost response;
- a later access-eligible drive may append a new authorization and perform a new immutable read;
  and
- every new application-protocol call still has its own authorization.

This preserves audit truth but does not provide forensic retention of a response lost with the
process.

### Effect execution and recovery

An effect state spans several drives under one fixed runtime-provided lifecycle:

1. **Author request.** For an `Unstarted` effect node, `Runtime` invokes the state's pure request
   callback over a verified frame.
2. **Commit request.** `Runtime` derives and commits
   `StateTransitionCommitted(EffectRequested)`. The node becomes `AwaitingEffect`; no external IO
   has occurred. This transition is the stable MFM outbox.
3. **Prepare ensure.** On a later drive, `Runtime` reconstructs the exact request and executor
   binding and produces `Prepared<Ensure>`. Every expected local validation finishes here.
4. **Authorize.** The kind-specific runtime access method appends `ExternalAccessAuthorized`.
   Only a positively acknowledged new append constructs `Authorized<Ensure>`.
5. **Invoke executor.** The affine authority is consumed by one `ensure`. The independently fenced
   executor ledger owns effect binding, resource allocation, target-attempt authorization, target
   entry, target observation, retained frontier, and terminal tombstone.
6. **Commit observation.** The runtime-facing adapter returns one stage-aware closed value.
   `Runtime` totalizes it into `PendingObservation<Ensure>`, retains it across physical append
   attempts, and commits `ExternalAccessObserved`. Only that positive commit or exact identical
   logical resolution constructs `CommittedObservation<Ensure>` and permits successful access
   progress.
7. **Settle semantically.** On a later drive, `Runtime` reloads `VerifiedRunView`. Pending or
   insufficient evidence may permit another separately authorized ensure. Accepted terminal
   evidence enters the pure state settlement callback and produces
   `StateTransitionCommitted(EffectSettled)`.

`Runtime` does not inspect or choose executor delivery policy. Every MFM `ensure` has its own
run-history authorization and observation. Every independently meaningful target attempt has its
own executor authorization and observation.

If the process dies after target entry but before the MFM observation, the run history contains an
unmatched authorization. A later authorized `ensure` asks the executor to converge the same keyed
request. The executor reloads retained evidence rather than blindly repeating target entry, and
`Runtime` records the newly returned pending observation before semantic settlement.

### Fact-selection recovery

Fact selection becomes the reserved implementation of the same access protocol:

```text
Prepared<FactSelection>
  -> committed barrier authorization
  -> Authorized<FactSelection> with affine scan permit
  -> complete authoritative scan
  -> totalize the affine `CompletedFactScan`
  -> PendingObservation<FactSelection>
  -> CommittedObservation<FactSelection> with linked attestation
```

Partial scan state remains private and non-authoritative. Process loss abandons that scan and leaves
the authorization unmatched. A later attempt receives a new barrier and authorization.

Every normally returned fact-store scan or scan-result conversion failure after authorization must
become a closed `PendingObservation<FactSelection>`, normally a `NonDomainFailure` or an explicitly
admitted safe failure. A complete affine scan must be converted exactly once into immutable sealed
pending material before a predecessor-bound observation candidate is prepared; stale-head retry
retains that material and never rescans. Store unavailability and unresolved acknowledgement after
the scan remain inside observation retry; task or process loss leaves the authorization unmatched.
A returned scan error may not escape through a fact-selection-only lifecycle.

### Admission through the same mutation owner

Application code authenticates, resolves immutable configuration, and authors/certifies artifacts.
It constructs the authorized admission plan; runtime owns the append.

The implemented split is:

```text
application:
  authenticate
  authorize admission
  resolve exact immutable configuration
  author and certify the run
  construct AuthorizedAdmissionPlan

Runtime:
  validate the plan against admission authority
  prepare RunAdmitted
  append the root
  return committed admission proof
```

`Runtime::admit` is mutation orchestration, not authentication or graph authorship. Application
code does not retain a direct run-history mutation handle after constructing `Runtime`. Read,
replay, trace, audit, and export use purpose-specific reader facades rather than a clone that can
also append.

### Store boundary

The store remains the sole owner of:

- candidate and commit canonicalization;
- per-run predecessor and exact-head compare-and-swap;
- predecessor-bound physical append-request idempotency and acknowledgement resolution;
- legal record and batch shapes;
- logical-key uniqueness;
- observation-to-authorization uniqueness and exact identical-content resolution;
- object admission and producer binding;
- fact coordinates and scan attestation;
- closure and late-audit-tail legality;
- backend atomicity; and
- callback-free structural/semantic folding.

The storage backend continues to receive only a store-created sealed verifier. It does not receive
state callbacks, live invokers, or a generic event payload.

Where crate visibility prevents strict privacy, construction authority must remain sealed and an
architecture test must prove that only the private runtime execution module can use the passive
writer or production mutation surface.

### State boundary

Domain states remain:

```text
pure
read
effect
```

They may:

- purely author one typed request;
- interpret committed compatible observations;
- return insufficient or invalid evidence verdicts;
- construct typed settlement, output, facts, or domain failure; and
- remain reusable independently of CLI, REST, store, and concrete transport.

They may not:

- append history;
- receive authorization or observation-draft types;
- invoke transports or executors;
- model runtime retries or worker lifecycle; or
- consume `NonDomainFailure`.

The existing “one state request maps to one independently meaningful capability operation” rule
is retained and strengthened through registration and certification tests. State size is a
domain-composition concern, not the access-audit enforcement mechanism.

### Replay boundary

Recorded-history verification must understand the complete outcome algebra without live callbacks:

- authorization and observation linkage;
- exact binding, operation, request, and semantic anchor;
- `Returned`, `DidNotEnter`, `Indeterminate`, and `NonDomainFailure`;
- non-domain fault code, conservative entry status, and disposition;
- object and executor-retained closure;
- one observation per authorization;
- unmatched authorization;
- late audit tail after closure; and
- semantic non-consumability plus fixed retry/block projection of non-domain failures.

Exact reproduction may rerun pure state computations but never invokes capabilities, executors, or
the runtime mutation path.

## Why Not Record Everything as State Transitions?

### The appealing version

The useful intuition was:

> If states were smaller, each meaningful step could produce one state transition, and all
> persistence could happen at the transition choke point.

A read might appear as:

```text
PrepareRequestState
  -> AuthorizeReadState
  -> PerformReadState
  -> ObserveReadState
  -> ApplyReadState
```

An effect might use a similar sequence.

This makes every durable event look uniform and appears to remove authorization and observation
recording branches.

### The external call still splits the transaction

The call must remain between two durable facts:

```text
authorization committed
  -> external IO
  -> observation committed
```

Changing both records to `StateTransitionCommitted` does not make those operations atomic and does
not force the second append to happen.

The same bug remains possible:

```text
append authorization transition
  -> call external system
  -> return early before observation transition
```

The enforcement mechanism must therefore own the complete call bracket, regardless of record name.

### Authorization and observation are not semantic changes

An authorization says an operation may occur. An observation says a reviewed result was seen.
Neither necessarily changes domain state.

Encoding them as semantic transitions requires one of two choices:

1. allow no-op semantic transitions with identical before/after semantic digests; or
2. add runtime access phases to the semantic state.

The first makes “semantic transition” mean audit event. The second makes retries, ambiguity, and
worker timing part of domain state.

The current separate `JournalHead` and `SemanticHead` distinction exists precisely because audit
history may advance without semantic state changing.

### Attempts are not finitely known graph nodes

A read observation may be insufficient. An effect executor may return `Pending`. Another audited
attempt may then be legal:

```text
Authorized[0]
  -> ExternalAccessObserved[0: insufficient]
  -> Authorized[1]
  -> ExternalAccessObserved[1: insufficient]
  -> ...
```

The number of attempts is not generally known when the graph is certified. Pre-expanding one state
per attempt would require:

- a fixed maximum in every domain graph;
- graph-level retry policy;
- dynamic graph mutation; or
- semantic state-machine loops.

All four add concepts and move runtime/executor policy into domain topology.

### Late audit observations survive semantic closure

A pre-closure authorization may receive its observation after `RunClosed`. The observation is
legal audit evidence but cannot change semantic state.

A transition-only design must either reopen the run, allow semantic transitions after closure, or
admit that some “transitions” are audit-only. The last option reconstructs the existing
distinction under a less precise name.

### Executor target attempts are a separate authority domain

An MFM effect state does not own:

- nonce or sequence allocation;
- target-attempt count;
- signing;
- rebroadcast or replacement policy;
- destination convergence;
- terminal tombstones; or
- the executor writer fence.

Representing executor delivery attempts as MFM states would merge distinct failure and fencing
domains and make the runtime choose executor policy.

### Recommended use of smaller states

Smaller states remain useful when they expose independently meaningful domain operations:

- one bootstrap read;
- one anchored balance read;
- one finality check;
- one pure aggregation;
- one recoverable transaction submission effect.

They do not represent runtime implementation phases such as “authorization append” or
“observation append.”

The certified execution kind instead selects a reusable runtime-supplied composition. State code
authors the request and interprets freshly verified committed evidence; it cannot customize or
bypass the middle:

```text
Effect<Request, Evidence, Settlement>
  = CommitRequest
  ; Repeat<RecordedEnsure<Request, Evidence>>
  ; CommitSettlement
```

The number of `RecordedEnsure` attempts is reconstructed from history and remains value-level.
Typing relates the request, executor completion, retained evidence, and settlement contracts for
each attempt; it does not attempt to encode an unbounded attempt count in the certified graph.

The accepted design adopts the useful part of the intuition at the runtime-protocol layer:

> Make the private runtime protocol phases small and typed; do not make them domain states.

## Failure And Crash Semantics

| Boundary | Durable run-history fact | Live authority | Required recovery |
| --- | --- | --- | --- |
| Preparation fails before authorization | Prior verified history only | None | Return reviewed runtime block/error; no live operation occurred. |
| Authorization append is rejected or stale | No new authorization | None | Reload and rederive; do not invoke. |
| Authorization append acknowledgement is ambiguous | Authorization may or may not exist | None | Resolve original append identity; never recreate the fresh permit. |
| Authorization positively commits | `ExternalAccessAuthorized` | One fresh affine permit | Invoke at most one registered operation. |
| Process dies before invocation | Unmatched authorization | Permit lost | Later drive may authorize a fresh attempt; never claim non-entry from absence alone. |
| Process dies during invocation | Unmatched authorization | Lost/consumed | Preserve ambiguity; effect executor reconciles its own target ledger. |
| Wrapper returns `DidNotEnter` | Authorization plus `PendingObservation<K>` in memory | Consumed | Append exact linked observation before successful return. |
| Wrapper returns `Returned` | Authorization plus `PendingObservation<K>` in memory | Consumed | Append exact linked observation before successful return. |
| Wrapper returns `Indeterminate` | Authorization plus `PendingObservation<K>` in memory | Consumed | Append exact linked observation before successful return. |
| Wrapper returns retryable operational failure | Authorization plus `PendingObservation<K>` containing non-domain failure | Consumed | Append audit-only observation; after commit a later drive may authorize a new attempt. |
| Wrapper returns protocol/integrity failure | Authorization plus `PendingObservation<K>` containing non-domain failure | Consumed | Append audit-only observation with `IntegrityBlocked`; block semantic consumption and further automatic progress. |
| Observation logical key already has identical content | Linked `ExternalAccessObserved`; pending observation still held | None | Construct committed proof; do not append or reinvoke. |
| Observation logical key has different content | Conflicting linked `ExternalAccessObserved`; pending observation still held | None | Report integrity conflict; do not append or reinvoke. |
| Observation physical append loses exact-head race | Authorization; pending observation still held | None | Discard only the stale physical attempt, reload, and prepare a new attempt over the same pending observation. |
| Observation physical append acknowledgement is ambiguous | Observation may or may not exist; pending observation and original physical attempt still held | None | Resolve the unchanged original attempt before any rebase; do not reinvoke. |
| Journal unavailable while pending observation is held | Authorization; pending observation in memory only | None | Retry while task lives; no successful drive result. |
| Task/process dies before observation commit | Unmatched authorization; pending observation lost unless another durable participant retained it | None | Preserve ambiguity; never fabricate the lost value. |
| Observation commits or resolves identically | Linked `ExternalAccessObserved` | None | Return committed proof; later drive may evaluate state-consumable outcomes or apply the fixed non-domain disposition. |
| Process dies after observation, before settlement | Committed observation | None | Reload and retry pure semantic evaluation without live access. |
| Observation arrives after semantic closure | Fixed closure plus pre-closure authorization | None | Permit one linked audit-tail observation; semantic head remains fixed. |

## Type-Level Enforcement

### Private constructors and sealed kinds

`AccessKind`, `Prepared<K>`, `Authorized<K>`, `PendingObservation<K>`, and
`CommittedObservation<K>` must have private fields and sealed construction.

External crates may implement only the narrow registered capability/executor contracts selected by
qualification. They cannot construct an authorization, observation, or committed proof.

### No outer invocation or totalization `Result`

After `Authorized<K>` is minted, the live invocation and producer-free totalization must produce
one `PendingObservation<K>`.

Internal helpers may remain fallible. The adapter owning the boundary must convert every surviving
error into the exhaustive persistable outcome algebra before returning to `Runtime`. The adapter
may not return a raw completion plus a separate fallible encoding or validation step.

This rule prevents `?` from accidentally becoming an unjournaled completion policy.
`commit_observation` retries store unavailability, stale predecessors, and acknowledgement
ambiguity while retaining the pending value. It returns only committed identical content or an
exact integrity conflict; dropping its task starts no detached completion.

### Unforgeable success

The private access bracket succeeds only with `CommittedObservation<K>`, whose constructor requires
a positively committed store result or an exact identical-content resolution under the
authorization logical key.

No other successful post-access return type exists.

### Intermediate values do not escape

`Authorized<K>` and `PendingObservation<K>` remain within the private runtime access methods,
`commit_observation<K>`, or their private helpers. No caller receives a value and a separate
obligation to remember the next method.

This is stronger than publishing a typestate builder such as:

```text
authorized.invoke().await?.record().await?
```

because a public caller could still return between those steps.

### Affine rather than falsely linear

Rust cannot prove that a task or process will not drop an affine value. This RFC does not claim a
linear-language guarantee.

The type-level safety property is:

```text
no constructible normal success path skips observation commit
```

Dropping the task produces no success proof and leaves the durable authorization unmatched.

### Compile-time and architecture enforcement

The cutover must include compile-fail and architecture tests proving:

- callers cannot construct fresh authorization;
- callers cannot clone live authority;
- callers cannot construct `CommittedObservation<K>`;
- callers cannot invoke without `Authorized<K>`;
- state settlement cannot accept a pending observation;
- capability and executor traits cannot return an outer invocation error after accepting
  `Authorized<K>`;
- application code cannot call run-history mutation APIs;
- scheduler, decision, callback, replay, and application modules cannot access the passive writer;
- only the private `Runtime::perform_access` dispatch can reach registered live invokers; and
- only the private runtime execution module reaches the production append surface.

An architecture text scan alone is not sufficient where type/module privacy can enforce the same
boundary, but it remains useful as a regression inventory.

## Record Algebra Decision

### Retain the five families

This RFC retains:

```text
RunAdmitted
StateTransitionCommitted
ExternalAccessAuthorized
ExternalAccessObserved
RunClosed
```

The single choke point is an execution-ownership decision, not a request for one undifferentiated
persisted event type.

The distinctions preserve:

- genesis authority;
- semantic versus audit heads;
- authorization before live access;
- observation without semantic mutation;
- one fixed closure;
- late audit-only observations; and
- clear replay and disclosure policy.

### Retain one audit-only `NonDomainFailure` outcome

The current v3 design includes `NonDomainFailure`. The former three outcomes had no clearly
non-consumable representation for a surviving operational, protocol, integrity, or contract
failure. Reusing `DidNotEnter` or `Indeterminate` could turn platform unavailability or corruption
into state-consumable evidence.

The completed cutover inventories every error reachable from the registered live boundary through
pending-observation construction. The v3 annex freezes `NonDomainFailureCode`, the
history-derived `NonDomainFailureLayer`, conservative entry status, and the
`RetryableOperational | IntegrityBlocked` disposition relation. Provider and transport faults
admitted by a certified `SafeFailure` contract remain in that separately closed
code/class/stage/diagnostic relation; every other normally returned external fault must use the
non-domain relation.

The change created one current v3 schema lineage and corpus, explicitly rejects the v1 and v2
archives, and has no dual reader or compatibility writer. Archived bytes cannot be silently
reinterpreted.

## Ownership After Cutover

| Responsibility | Sole owner |
| --- | --- |
| Drive loop and pure next-step derivation | `Runtime` over `VerifiedRunView` |
| Run-history mutation orchestration | `Runtime::drive_once` selected-action match and `Runtime::admit` |
| Passive mutation capability custody | `Runtime` |
| Typed authorization/invocation/observation protocol | `Runtime::perform_access` and `Runtime::commit_observation<K>` |
| Stable pending-observation ownership and physical append retry | `Runtime::commit_observation<K>` |
| Live request and outcome semantics | Certified state and capability/executor contracts |
| Low-level boundary-stage and non-domain disposition classification | Registered runtime-facing adapter or executor |
| Affine MFM live-access minting | `Runtime` consuming a fresh store witness |
| Candidate legality and atomic append | `mfm-store` |
| Observation logical-key resolution and physical append idempotency | `mfm-store` |
| Structural `RunClosed` derivation | `mfm-store` with the final transition append |
| Persisted record/value schemas | `mfm-journal` and recoverability annex |
| Concrete run-history persistence | Qualified storage backend |
| Effect delivery/resource convergence | Independently fenced executor ledger |
| Semantic evidence acceptance | Pure state callback over committed observation view |
| Recorded verification | Store fold and replay over committed history |
| Authentication and public transport | App, CLI, and REST |

## Complete Cutover Scope

### Runtime

- Keep `Runtime` as the sole active interpreter.
- Make each assembled `Runtime` consume and privately own its passive mutation capability.
- Preserve independently assembled runtime workers under the same qualified writer fence and store
  compare-and-swap contract.
- Add one private sealed access-protocol module.
- Centralize all semantic transition commits under `Runtime::commit_transition`.
- Route the closed read, effect, and fact-selection sum through `Runtime::perform_access` and the
  shared `Runtime::commit_observation<K>` bracket.
- Keep kind-specific preflight and invocation beneath that shared typed sequence.
- Return genuine pre-invocation head advances to the runtime loop as `StepResult::Replan`.
- Retain post-invocation `PendingObservation<K>` independently of predecessor-bound candidates and
  retry its exact logical observation without reinvocation.
- Route admission through `Runtime::admit`.
- Ensure scheduler, decision, callback, and replay helpers cannot access the passive writer.
- Delete distributed `perform_read`, `append_read_observation`, `perform_ensure`,
  `append_effect_observation`, and fact-selection append orchestration.
- Delete every early live-boundary or returned-value-totalization exit not represented by the
  closed persistable algebra.

### Runtime access contracts

- Replace the effect executor's outer `Result` return with a closed stage-aware completion.
- Move every request-side binding, type, codec, contract, and encoder-selection validation before
  authorization.
- Apply the selected result encoder after invocation and convert every surviving result-side
  encoding or validation failure to an integrity-blocked `NonDomainFailure`.
- Make boundary-return-to-pending-observation conversion total over every admitted outcome variant.
- Keep `Authorized<K>`, `PendingObservation<K>`, and physical observation-candidate types private to
  the runtime protocol.
- Delete superseded `call_read`, `call_ensure`, `UncommittedAccessObservation`, erased outer-error
  routes, and duplicate read/effect/fact observation wrappers.

### Executor

- Retain internal typed `ExecutorError`.
- Inventory every error site after MFM authorization.
- Record whether target authority was minted and whether the invocation returned to the executor's
  retained private completion seal.
- Derive boundary stage and non-domain disposition from affine-authority consumption, completion
  progress, and fault semantics, not error names.
- Project every surviving error into `DidNotEnter`, `Indeterminate`, or `NonDomainFailure`.
- Preserve independent executor ledger, resource stream, target authorization, target observation,
  frontier, tombstone, and fence.
- Strengthen wallet executor failure-injection tests at every await and append boundary.

### Store authority

- Split mutation capability from read, replay, trace, audit, and export facades.
- Make the passive production writer or mutation authority non-cloneable and consume it into
  `Runtime`.
- Require that passive capability for admission and existing-run mutation.
- Keep store preparation and append APIs inaccessible to scheduler, decision, callback, replay,
  application, CLI, and REST code.
- Remove every app-retained mutation-capable store handle.

### Store and journal

- Retain the five record families.
- Add `NonDomainFailure` through one complete current-schema cutover.
- Validate its closed `NonDomainFailureCode`, history-derived `NonDomainFailureLayer`, entry status,
  disposition, binding, non-consumability, and fixed retry/block projection.
- Resolve observation logical keys by authorization reference and exact content before preparing a
  physical append attempt.
- Keep physical append-request idempotency bound to one predecessor and candidate. On stale head,
  permit a new physical attempt over the same pending observation; on acknowledgement ambiguity,
  require resolution of the unchanged original attempt.
- Preserve one observation per authorization and late-tail legality.
- Preserve atomic object admission and executor-retained closure.
- Let observation candidate preparation borrow or derive from stable pending material without
  consuming the only retryable copy.
- Return kind-typed fresh authorization witnesses where doing so removes runtime rematching.
- Keep the backend seam sealed behind store-created verifiers.
- Remove public mutation material or constructors no longer required outside the private runtime
  execution boundary.

### Replay

- Verify the complete observation outcome algebra callback-free.
- Project each `NonDomainFailure` as fixed retryable-operational or integrity-blocked audit state.
- Prove no `NonDomainFailure` can satisfy read or effect settlement.
- Preserve unmatched authorization as ambiguity.
- Preserve semantic/journal head separation and late audit tails.
- Remove any duplicate completion classification from replay.

### Application

- Replace direct admission preparation/append with one authorized admission plan passed to
  `Runtime::admit`.
- Keep authentication, tenant derivation, immutable configuration resolution, and artifact
  certification in app assembly.
- Remove direct application access to run-history mutation.
- Use purpose-specific read/replay/trace/audit/export facades.
- Keep CLI and REST as decode/invoke/render boundaries.

### Documentation and schemas

- Update `docs/design.md` and `docs/architecture.md` with the accepted ownership and guarantee.
- Update `docs/run-execution.md`, persisted/public surface inventory, predicate-owner inventory,
  cutover gates, and affected product capability documents.
- Update the recoverability annex and corpus for the new observation schema.
- Regenerate every positive and negative golden vector.
- Do not retain contradictory descriptions of the old distributed obligation.

## Verification Requirements

### Compile-fail boundaries

- `Authorized<K>` cannot be constructed or cloned.
- `PendingObservation<K>` cannot be supplied directly to a state callback or application output.
- `CommittedObservation<K>` cannot be constructed outside positive store commit or exact
  identical-content resolution.
- Live invocation cannot occur without `Authorized<K>`.
- A pending observation cannot construct `DriveOutcome::Advanced`.
- An effect executor implementation cannot retain the old outer-error signature.
- App/CLI/REST cannot import a mutation-capable history facade.

### Runtime unit and conformance tests

- Every `PlannedRunStep` variant is exhaustively interpreted.
- Every access kind passes through `Runtime::perform_access` and
  `Runtime::commit_observation<K>`.
- Every authorization append outcome is exhaustively interpreted.
- Only a directly acknowledged `NewlyAppended::Authorization` dispatches a live invoker.
- `Advanced`, `Replan`, rejection, and unresolved acknowledgement never mint live authority.
- Every persistable outcome variant produces exactly one linked pending observation.
- `NonDomainFailure` is never state-consumable.
- Every `NonDomainFailureCode` is legal only for its history-derived layer and exact
  code/status/disposition relation.
- `RetryableOperational` permits only a later separately authorized attempt after its observation
  commits.
- `IntegrityBlocked` deterministically blocks.
- All preflight failures occur before authorization and invoke no live boundary.
- No ordinary live-boundary or returned-value-totalization error escapes without classification.
- Definite stale-head observation rejection creates a new physical attempt over the same pending
  observation without reinvocation.
- Observation acknowledgement ambiguity resolves the unchanged physical attempt before any rebase.
- Identical logical observation resolution returns committed proof.
- Changed-content second observation blocks as integrity failure.
- No raw return or pending observation reaches application output.

### Crash-boundary tests

Inject failure:

1. before authorization preparation;
2. before authorization commit;
3. after authorization commit before acknowledgement;
4. after fresh permit before invocation;
5. during invocation before possible entry;
6. after possible entry before wrapper return;
7. immediately after each boundary-return variant is totalized into a pending observation;
8. during observation preparation;
9. before observation commit;
10. after observation commit before acknowledgement;
11. after observation commit before `drive_once` return;
12. before semantic settlement;
13. after final transition before closure acknowledgement; and
14. for a legal post-closure observation.

Each test must assert the exact durable prefix, whether live authority existed, whether another live
operation is legal, and whether recovery repeats IO.

### Effect tests

- Every surviving `ExecutorError` site reachable after MFM authorization receives an exact
  entry-stage, closed fault-code, and non-domain disposition classification.
- Every normally returned provider or transport fault becomes either its exact certified
  `SafeFailure` or one linked `NonDomainFailure`; no outer transport error bypasses observation.
- Restored terminal evidence returns without target entry.
- Target mutation followed by executor crash converges without duplicate logical effect.
- MFM observation failure does not discard executor-retained evidence.
- Resource/fence failures cannot be mistaken for target non-entry.
- Retryable operational executor faults cannot settle the state but may permit a later separately
  authorized ensure.
- Integrity-blocked executor faults cannot settle the state or trigger another automatic ensure.

### Read and fact-selection tests

- A lost read result leaves only unmatched authorization.
- A fresh read attempt has a new authorization.
- A surviving closed read result cannot produce successful drive without observation commit.
- Normally returned provider and transport faults preserve their closed safe-failure or
  non-domain classification without provider-controlled text.
- Fact-scan partial work cannot mint completion or observation.
- Fact-scan completion and attestation commit atomically.
- A complete fact scan totalizes once into stable pending material and survives stale-head
  observation retry without rescanning.
- Store/scan integrity failures classify consistently through the shared runtime protocol.

### Store and PostgreSQL tests

- Every append remains all-or-nothing.
- `OutcomeUnknown` never mints fresh live authority.
- Observation ambiguity resolves the unchanged original physical append attempt.
- A definite stale predecessor permits a new physical attempt over identical pending content.
- Existing identical observation content resolves to committed proof even when it was committed
  under another physical attempt.
- Existing changed observation content conflicts.
- One observation per authorization remains enforced under concurrency.
- Non-domain-failure rows and objects have memory/PostgreSQL parity.
- Application role cannot update, delete, truncate, or bypass the append contract.
- No production path uses a replica for authority-bearing work.

### Replay and security tests

- Recorded-history verification invokes no state, capability, executor, classifier, signer, or
  transport callback.
- Every non-domain failure deterministically yields the same retryable-operational or
  integrity-blocked projection.
- Unknown non-domain codes, illegal contextual layers, and invalid code/status/disposition
  relations are rejected rather than projected through a fallback.
- Any future code-guided operational retry policy must prove that it acts only after the prior
  observation commits and only under a new authorization.
- No non-domain failure can be reinterpreted as a domain failure or safe failure.
- Provider text, endpoints, paths, credentials, raw response bodies, signatures, and signed
  envelopes never enter records, objects, diagnostics, errors, audit DTOs, or exports.
- Portable export retains exact history linkage without granting live authority.

### Architecture tests

- Only the private runtime execution module uses the passive writer or calls mutation APIs.
- Scheduler, decision, callback, and replay modules cannot import the passive writer.
- Only `Runtime::perform_access` and its private kind-specific methods can dispatch a registered
  live invoker.
- Planned access is one closed concrete sum and no erased invoker can supply an append lifecycle.
- App has no admission or drive append call.
- Separately assembled runtime workers may each receive a non-cloneable mutation handle under the
  same qualified writer lineage; no non-runtime assembly component can retain one.
- Capability and executor crates depend on no run-history mutation surface.
- Concrete storage depends only on the raw sealed backend contract.
- Runtime and replay do not construct competing folds.

## Acceptance State

The implementation satisfies the following accepted criteria:

- production run-history mutation has one code-path/type owner;
- `Runtime` is the sole active interpreter and mutation code-path owner while independently
  assembled runtimes remain legal under the same qualified writer fence;
- any separate writer type is passive and non-cloneable;
- admission and post-admission mutation pass through that owner;
- every live access kind uses the staged `Runtime::perform_access` and
  `Runtime::commit_observation<K>` path;
- no runtime-facing live invocation returns an outer error after accepting `Authorized<K>`;
- every surviving return totalizes into one non-escaping `PendingObservation<K>`;
- no normal successful drive path can contain a pending observation;
- stale-head retry preserves the same logical observation while using a new physical attempt;
- acknowledgement ambiguity resolves the unchanged original physical attempt before rebase;
- only positive commit or exact identical-content resolution constructs
  `CommittedObservation<K>`;
- non-domain failures are never state-consumable and have one fixed retryable-operational or
  integrity-blocked disposition;
- every normally returned provider or transport fault has a linked, closed, queryable
  safe-failure or non-domain classification with no open diagnostic escape;
- a non-domain code may refine future qualified retry scheduling but cannot itself authorize,
  require, or hide a retry;
- only committed observations can enter semantic evaluation;
- process loss remains an unmatched authorization without invented evidence;
- effect recovery still uses the independently fenced executor ledger;
- state callbacks remain pure;
- store and replay validation remain callback-free;
- every superseded mutation path and type is deleted;
- the persisted schema has one current identity;
- no compatibility reader, writer, alias, or fallback remains;
- documentation describes one current design; and
- scope-selected verification passes.

## Alternatives Considered

### Patch each current early return

Rejected.

Adding local error mapping around the current read, effect, and fact-selection paths retains several
obligation owners. A later access kind, validation step, or refactor can recreate the same gap.

### Extract an active `RunHistoryGate`

Rejected.

Moving transition commits, authorization, invocation, classification, observation retry, fact
scanning, and admission into a runtime-owned gate creates a second active coordinator. It relocates
the current imperative work without reducing responsibility or code paths. Exclusive mutation
custody needs only a passive capability; orchestration remains in `Runtime`.

### Add a general typed state-composition language

Rejected.

A configurable `Then<A, B>` DSL, free monad, or protocol graph would require its own validation and
interpreter, becoming another scheduler. The runtime needs only the one fixed
`Prepared -> Authorized -> PendingObservation -> CommittedObservation` access composition selected
by a closed `AccessKind`.

### Let an erased planned access execute itself

Rejected.

An erased `PreparedAccess` trait object with an `execute`, `invoke`, `observe`, or retry hook would
become another lifecycle owner and could reintroduce kind-specific post-authorization exits. The
planned-step algebra instead uses one closed concrete sum. Type erasure remains only inside the
already-selected registered invoker and cannot control authorization or observation persistence.

### Record only through semantic state transitions

Rejected.

Authorization and observation are not necessarily semantic changes, repeated attempts are not
finitely known graph nodes, and late observations can occur after semantic closure. The external
call still sits between two durable writes, so renaming them does not enforce the bracket.

### Generalize every history record into one `Transition` variant

Rejected.

This may simplify terminology but does not reduce the number of authority facts or make the
post-call append mandatory. It also obscures semantic versus audit heads.

### Let capabilities append their own observations

Rejected.

Capabilities would gain journal/store authority, couple reusable transports to MFM persistence,
duplicate retry and append logic, and become able to author their own audit evidence outside the
runtime/store validation boundary.

### Pass a narrow observation continuation into the capability

Not selected.

A one-use continuation could make a normally returned success impossible without calling the
continuation, but it moves persistence waiting and failure handling into the capability future.
The private runtime access bracket and shared `commit_observation<K>` provide the same normal-return guarantee while
preserving the capability boundary.

### Use `Drop` or `#[must_use]` to force observation

Rejected.

`Drop` cannot perform reliable asynchronous persistence and does not run after process death.
`#[must_use]` is a lint and can be bypassed by explicit drop.

### Hold one database transaction over authorization, IO, and observation

Rejected.

It creates long-lived locks, couples database availability to remote latency, still cannot make the
remote participant atomic, and violates the existing transaction boundary.

### Add a durable outbox for every read and fact scan

Deferred unless the stronger forensic guarantee is explicitly required.

It could retain a completion outside the run journal before process return, but introduces another
durable writer, recovery protocol, fence, replay relationship, and obligation owner. It also cannot
atomically cover a remote operation that does not participate in its protocol.

The implemented baseline guarantees no successful escape without committed observation and
uses unmatched authorization for interruption.

### Treat journaling as best-effort telemetry

Rejected.

The run history is authority, not logging. A journal outage must fail closed for new live
authorization and cannot silently degrade audit coverage.

### Merge the executor ledger into the run history

Rejected.

The executor owns delivery/resource policy, target-entry authority, and an independent writer fence.
Merging it into run semantic authority would weaken ownership and recovery boundaries.

## Security Considerations

Centralization increases the impact of the `drive_once` selected-action match,
`Runtime::perform_access`, and `Runtime::commit_observation`, so
their accepted plan and persistable outcome algebras must remain deliberately small and closed.

The runtime protocol must:

- accept only certified/qualified requests and bindings;
- retain no credential or signer material;
- persist only reviewed typed values and closed codes;
- never log raw boundary returns, pending observations, or debug values;
- avoid formatting provider or executor errors into persisted/public messages;
- preserve provider and transport trackability only through the exact authorization linkage,
  certified `SafeFailure` relation, or closed non-domain code/layer/status/disposition relation;
- preserve zeroizing transient buffers where required;
- never expose append or access authority through trace, audit, replay, or export;
- fail closed when authorization append is unavailable or ambiguous;
- conservatively classify entry status and non-domain disposition;
- never interpret a non-domain code as fresh authority or permission to bypass its persisted
  disposition; and
- keep every non-domain failure non-consumable.

The passive writer must never be exposed through trace, audit, replay, export, application, CLI, or
REST surfaces. It must not become a general “record anything” logger.

## Performance And Availability

This RFC retains two run-history appends around every live MFM access:

```text
authorization append
  -> external operation
  -> observation append
```

The shared runtime protocol may reduce duplicate loads, validation, wrapper allocation, and call
sites, but must not optimize away either authority boundary.

Strict audit availability remains part of live-operation availability. If authorization cannot
commit, live access must not occur. If observation cannot commit, the access step cannot report
success.

Observation retry cannot repeat external IO. A definite exact-head conflict retains
the pending observation and prepares a new physical attempt against the new head. Acknowledgement
ambiguity must resolve the unchanged old attempt before any such rebase.

Future code-guided backoff or circuit-breaking policy operates only after a retryable observation
commits. It must not add an invisible transport retry within one authorization or make timing,
worker identity, or transient routing choice part of semantic state.

Any future batching optimization must retain one authorization and one observation identity per
independently meaningful operation and prove partial-return and cancellation semantics separately.

## Persisted-Schema And Rollout Policy

`NonDomainFailure` requires a complete current-schema cutover:

1. define the new observation outcome, entry-status, disposition, and closed-code schema;
2. update the recoverability annex and complete corpus;
3. regenerate schema and identity vectors;
4. update journal codecs and constructors;
5. update store preparation, load, fold, and replay;
6. update audit/export projections;
7. explicitly reject old current data;
8. inventory and export any evidence that must survive reset; and
9. remove v1 and v2 as production readers/writers.

No persisted bytes are rewritten in place. No dual reader, dual writer, fallback decoder, alias, or
compatibility mode is permitted.

Adding a provider- or transport-specific `NonDomainFailureCode`, changing a code's legal layer,
entry status, or disposition, or adding an unknown-code fallback changes persisted semantics and
requires a new current schema lineage, annex, corpus, store/replay validation, and public
projection. The current v3 enum must not gain an open string, catch-all provider error, or
compatibility interpretation.

## Logical Commit Sequence

### 1. `make runtime the only run history mutation path`

- split the passive mutation capability from purpose-specific reader facades;
- consume each process assembly's passive capability into its `Runtime`;
- preserve independently assembled runtime workers under the qualified writer fence and store CAS;
- centralize current transition and access mutation orchestration in private runtime execution
  methods without changing persisted behavior;
- route authorized admission through `Runtime::admit`;
- remove app, reader, scheduler, decision, callback, and replay access to mutation;
- add compile and architecture ownership tests; and
- delete direct admission and distributed mutation paths in the same commit.

### 2. `seal and persist every authorized access outcome`

- introduce `Prepared<K>`, `Authorized<K>`, `PendingObservation<K>`, and
  `CommittedObservation<K>`;
- use one closed concrete `PreparedAccess` sum with no erased lifecycle hook;
- front-load every expected fallible precondition;
- close the executor return type;
- classify every surviving live-boundary and returned-value-totalization fault by entry status,
  non-domain disposition, and safe consumability;
- add `NonDomainFailure` and its closed code/status/disposition taxonomy;
- separate stable logical observation identity from predecessor-bound physical append attempts;
- retain pending material across stale-head rebase and resolve acknowledgement ambiguity against
  the unchanged original attempt;
- totalize `CompletedFactScan` before any observation candidate consumes its affine proof;
- route read, ensure, and fact-selection through `Runtime::perform_access` and the shared
  `Runtime::commit_observation<K>` implementation;
- perform the inseparable journal/store/replay/schema/golden-vector cutover;
- update EVM wallet execution and public audit projection;
- add the complete crash/failure matrix;
- add the generic conformance, compile-fail, and cancellation tests;
- make all intermediate values non-escaping; and
- delete the old outer-error, old-schema, minting, erased-observation, wrapper, and per-kind
  orchestration paths.

Each commit must leave one coherent current design. Verification is selected by changed boundary,
not by commit creation, and broad gates must not be redundantly run before `.#ci`.

## Accepted Decisions And Closure

The following decisions are accepted by this RFC:

- the guarantee is “no normal successful access result without committed observation”; task or
  process loss may leave an unmatched authorization, and no universal result outbox is added;
- mutation ownership is a code-path/type invariant per assembled runtime, not a deployment-wide
  singleton-writer claim;
- planned access is one closed concrete `PreparedAccess` sum rather than an erased lifecycle
  object;
- the post-invocation obligation is `PendingObservation<K>`;
- logical observation identity is distinct from predecessor-bound physical append-attempt
  identity;
- non-domain failures are never state-consumable and have a fixed
  `RetryableOperational | IntegrityBlocked` disposition;
- `NonDomainFailureCode` plus the history-derived contextual layer is the closed non-domain fault
  classification; normally returned provider/transport faults use either that relation or their
  certified `SafeFailure` relation, and a code may guide future policy only after commit and under a
  new authorization; and
- completion algebra, shared access composition, observation retry, and persisted-schema changes
  are one inseparable implementation commit.

The implementation closes every former decision gate:

- app assembly gives mutation custody to `Runtime` and exposes purpose-specific readers elsewhere;
- fact selection consumes its affine scan into one sealed pending result before observation
  preparation, so stale-head retry never rescans;
- the v3 annex freezes the complete `NonDomainFailure` code, entry-status, disposition, and
  contextual-layer relation;
- provider and transport failures are queryable through exact authorization linkage and one closed
  safe-failure or non-domain classification, never provider-controlled text;
- runtime-facing executor adapters classify fresh material separately from retained-history
  corruption and use exact affine-authority consumption and completion progress;
- app policy produces an authorized admission plan while runtime alone commits it;
- production readers accept only the destructive v3 lineage; v1 and v2 are archive-only; and
- CLI, REST, replay, trace, and audit expose the reviewed non-domain projection without provider
  text or secret-bearing detail.

## Material Uncertainties

none

## Current Decision Summary

The implemented current design is:

- one `Runtime` type as the sole active state-machine interpreter and run-history mutation
  code-path owner;
- one passive non-cloneable mutation capability consumed by each assembled `Runtime`, while
  independently assembled workers remain legal under the same qualified writer fence;
- purpose-specific read-only facades everywhere else;
- one closed planned-step interpreter;
- one closed concrete `PreparedAccess` sum with no erased lifecycle hook;
- one private sealed
  `Prepared<K> -> Authorized<K> -> PendingObservation<K> -> CommittedObservation<K>` protocol for
  read, ensure, and fact selection;
- one fixed runtime-provided lifecycle per certified state execution kind, with no configurable
  protocol DSL or second scheduler;
- all fallible preflight work before authorization;
- one positively committed authorization before live authority;
- one affine registered invocation;
- one exhaustive persistable outcome with no outer invocation-or-totalization error;
- one non-escaping pending-observation obligation after every surviving return;
- one stable logical observation identity independent of predecessor-bound physical append
  attempts;
- stale-head rebase over the same pending observation and unchanged-attempt resolution for
  acknowledgement ambiguity;
- one mandatory linked observation before successful escape;
- one mandatory audit-only `NonDomainFailure` representation with fixed
  `RetryableOperational | IntegrityBlocked` disposition;
- one closed `NonDomainFailureCode` plus history-derived layer for non-domain fault trackability,
  with code-guided future retry policy subordinate to committed disposition and fresh
  authorization;
- semantic settlement only from freshly verified committed observations;
- honest unmatched authorization for interruption and process loss;
- five distinct persisted run-history record families;
- one store-owned legal append/fold implementation;
- one independently fenced executor delivery/resource history;
- no direct mutation from scheduler helpers, state callbacks, app, readers, capabilities,
  executors, CLI, or REST;
- no transition-only encoding of runtime audit mechanics;
- no compatibility path; and
- one ordered two-commit cutover with complete failure, crash, replay, PostgreSQL, architecture,
  and no-secret verification.

The design centralizes obligation in the existing runtime rather than introducing
another active coordinator or merely centralizing storage. It preserves the distributed-systems
boundary that requires separate authorization and observation commits, while making a normal
unrecorded completion impossible to represent outside the one runtime protocol that owns both.
