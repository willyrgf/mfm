# MFM Design Contract

Status: authoritative typed-core design contract

This document defines the one current runtime, journal, store, replay, and application design. The
canonical recoverability schemas and identities are frozen in
`contracts/recoverability/v3/annex.json` and `contracts/recoverability/v3/corpus.json`. Where this
document summarizes a frozen encoding, the annex is authoritative.
The typed-core cutover is implemented and no pre-cutover lifecycle remains. Production EVM reads
are current audited graph nodes. EVM transaction submission is a registered recoverable effect
only through the qualified durable wallet executor and its independent deployment fence.

The central rule is:

```text
certified typed graph
  + append-only run journal
  + immutable content-addressed objects
  = the complete semantic authority for one run
```

Operations plan typed state graphs. States own all outcome-affecting computation. Runtime
owns admission and interprets one certified graph and one verified journal view. The store owns
atomic append and structural verification. Replay verifies or recomputes without live semantic IO.
App, CLI, and REST only authorize, invoke, and render those lower contracts.

## Non-Negotiable Invariants

- The journal has exactly five top-level record families:
  `RunAdmitted`, `StateTransitionCommitted`, `ExternalAccessAuthorized`,
  `ExternalAccessObserved`, and `RunClosed`.
- A store append is atomic. Records, commit envelope, fact coordinate, immutable objects, and
  journal-to-object bindings all become visible together or not at all.
- Every run commit is predecessor-linked by exact per-run sequence and commit digest. There is no
  store-global run ordering authority.
- `RunClosed` is appended in the same commit as the final semantic transition.
- Semantic node phase is exactly `Unstarted | AwaitingEffect | Terminal`; run phase is exactly
  `Open | Closed`. Readiness, access eligibility, waiting, success, and failure are derived.
- `drive_once` is stateless across calls and performs at most one semantic transition or one
  audited application-protocol operation.
- Every MFM-controlled semantic external operation is preceded by its own committed
  `ExternalAccessAuthorized` record. Only a directly observed newly appended authorization can
  mint affine live-access authority.
- Every surviving normal live-boundary return is totalized into one exact pending observation.
  The access protocol cannot report normal success until that logical observation is committed or
  resolved as byte-identical committed history.
- A `NonDomainFailure` is audit-only platform evidence. It is never a returned domain value, a
  `SafeFailure`, a typed domain failure, or state-consumable evidence.
- State logic performs no ambient semantic IO. Read and effect requests are pure and total over a
  certified `StateFrame`.
- Effect delivery and cross-effect resource coordination belong to a separately durable typed
  executor ledger. A committed MFM effect request is identity, not target-entry authority.
- Facts are typed emissions of a committed transition. Same-run consumers use certified graph
  edges. Deliberate prior-run selection is an audited read through
  `mfm.journal.fact-selection.v1`.
- Specs, manifests, snapshots, facts, outputs, and retained values are content addressed.
  Structured semantic identities use the frozen domain-separated JCS hash contract. Hashed
  structures contain no floats.
- Secrets never enter specs, journal records, retained objects, facts, context snapshots, public
  outputs, portable exports, or error details.
- Recorded-history verification, exact reproduction, and candidate comparison perform no live
  semantic-capability, provider, executor, domain-filesystem, or signer IO.
- All store-backed authority reads and writes use one qualified, fenced authoritative PostgreSQL
  writer. A replica is not current store authority.

## Authority Surfaces

| Surface | Authority | Contract |
| --- | --- | --- |
| Untrusted or lowered graph/spec bytes | no | Data until certification validates their exact schemas, planner profile, manifests, identities, and hashes. |
| `CertifiedAdmissionArtifacts` | yes | Non-cloneable coherent certification closure for one exact entry point, authored and expanded graph, certificate, and implementation manifests. |
| `QualifiedProgramRegistry` | yes, exact live/reproduction dispatch | One closed executable selection owning the admitted support graph, deterministic state callbacks, semantic read/effect entries, and process-private live invokers. |
| `QualifiedRunStore<B>` | affine bootstrap authority | Owns the only qualified backend handle and mutation seal before runtime assembly; admits support and is consumed by one split. |
| `RunHistoryWriter<B>` | sole process mutation capability | Non-cloneable split result consumed by `Runtime`; never retained by application or replay composition. |
| `RunHistoryReader<B>` | cloneable purpose-read capability | Read, replay, inspection, export, configuration, and source-verification surface with no append or support admission. |
| `AuthorizedAdmissionPlan` | one-use application-to-runtime authority | Opaque owned exact-registry admission prerequisites; carries no support graph, writer, backend, or prepared append. |
| `CommittedRunJournal` | structural authority | One store-verified sequence of native commits and records plus every exactly bound immutable object. |
| `VerifiedRunView` | semantic read authority | Non-cloneable view created by callback-free verification of the committed journal against the exact certified spec. |
| `VerifiedPublicRunView` | reviewed public value | Owned annex-validated status and certified-output projection with no journal, object, or follow-up read authority. |
| `PreparedCommit<Purpose>` | append authority | Sealed purpose-specific candidate carrying exact predecessor, records, object intents, and structural proof. |
| `NewlyAppendedAuthorization` | one-use store permit | Returned only with a directly observed new authorization append; consumed by runtime to mint one `AuthorizedAccess`. |
| `PreparedAccess` / `Prepared<K>` | private preflight proof | Closed runtime-owned sum and kind-typed value whose request, binding, codecs, contracts, routing, and immutable identities were validated before authorization. |
| `AuthorizedAccess<K, T>` | one-use live authority | Private runtime authority for exactly one registered read or ensure operation. |
| `PendingObservation<K>` | private persistence obligation | Immutable exact authorization plus complete persistable outcome. It is retained independently of predecessor-bound append attempts and cannot reach state, application, or a successful drive result. |
| `CommittedObservation<K>` | private committed proof | Constructed only after a positive observation commit or byte-identical logical-key resolution; it is the only normal successful result after invocation. |
| `ContentRef` | content identity only | Executor and general lightweight reference: schema id plus raw-byte content digest. |
| `ValueRef` | journal-retained identity | Full producer-bound artifact, evidence, schema, semantic type, role, length, media type, and producer binding. |
| `VerifiedExecutorBinding` | executor-ledger authority | Exact tenant, deployment generation, evidence contract, and optional typed resource ownership. |
| `EvmWalletRequestQualification` | deployment predicate plus private live transport ownership, not target-entry authority | One secret-free sealed equality proof over the actual route catalog, selected route/chain, executor semantic and evidence closure, derived signer descriptor, nonce policy/configuration, classifier, finality, assurance, generation/fence, tenant, wallet domain, sender, and evidence bounds. The live object separately retains a private clone of that exact transport instance; admission and execution share one qualification `Arc`. |
| `TargetEntryAuthority` | one-use executor authority | Authorization is committed before target entry; a private executor completion seal binds the callback's unbound outcome to that exact entry before observation persistence. |
| Rendered JSON and portable bytes | no live authority | Reviewed output/export representations; identifiers and cursors are never bearer authority. |

Authority types have private construction paths and are non-cloneable or affine where duplicating
them could duplicate a boundary crossing.

## Certified Planning And Closed Execution

An entry-point registration selects one exact `PlanningProfile`:

```text
PlanningProfile {
    planner_contract_ref,
    planner_implementation_ref,
    framework_policy_refs,
    canonical_profile_parameters,
}
```

Planning is deterministic. It receives canonical typed configuration and emits an authored graph,
then expands framework policy and executor support into ordinary typed nodes. The expanded graph,
state and capability implementation manifests, dependency rules, public-output binding, terminal
contract, and exact planning profile are certified together. Runtime never branches on whether a
node originated in domain authoring, framework expansion, or executor expansion.

The conceptual state contract is one closed sum:

```text
trait State {
    type Config;
    type Context;
    type Input;
    type Output;
    type Facts;
    type Failure;

    fn execution() -> StateExecution<Self>;
}

StateExecution::pure(apply)
StateExecution::read<Request, Response, Capability>(request, apply)
StateExecution::effect<Request, Evidence, Executor>(request, settle)
```

Callbacks are:

```text
pure:
  apply(StateFrame) -> Settlement

read:
  request(StateFrame) -> Request
  apply(StateFrame, ObservationView<Response>)
    -> Settlement | InsufficientEvidence | InvalidEvidence

effect:
  request(StateFrame) -> Request
  settle(StateFrame, RequestView<Request>, ObservationView<TerminalEvidence>)
    -> Settlement | InsufficientEvidence | InvalidEvidence
```

`mfm-program` owns `StateFrame`, borrowed `RequestView<T>`, borrowed
`ObservationView<R>`, and `Settlement`. These contain typed values and exact immutable references,
but no append, store, runtime, or external-access authority. Runtime privately owns the committed
request and observation proofs behind those views and binds a callback result to the exact selected
records before constructing a transition.

`Settlement` is closed:

```text
Succeeded {
    output_bindings,
    fact_emissions,
}

Failed {
    typed_failure_ref,
}
```

Each successful settlement's fact surface is certified as an ordered list of homogeneous slots:

```text
CertifiedFactSlot {
    fact_slot_ordinal,
    minimum_emissions,
    maximum_emissions,
    fact_descriptor_ref,
    subject_contract,
    response_contract,
}
```

Fact-slot ordinals are dense from zero. Every proposal in one slot has the same descriptor,
subject contract, and response contract. Each slot has
`0 <= minimum_emissions <= maximum_emissions <= 1024`, with a maximum of at least one; zero is a
legal minimum. An exact-one slot has minimum and maximum `1/1`. Across one settlement, the sum of
every `maximum_emissions` is at most 4,096.

`FactProposal` and `FactSet` are process-only callback values. Proposals are ordered by
nondecreasing `fact_slot_ordinal`; a settlement cannot return to an earlier slot, and order within
one slot group is preserved. The same descriptor plus exact subject and response content and
retained-value contracts cannot be proposed twice. Store-owned preparation rejects any group whose
count falls outside its certified minimum and maximum.

After those checks, the store assigns dense actual `emission_ordinal` values `0..N-1` across the
complete emitted sequence and persists both the actual ordinal and the authorizing
`fact_slot_ordinal`. `FactRef`, fact producer bindings, and fact logical identity use the actual
ordinal. A certified same-run fact dependency also names an actual ordinal. Certification must
prove that the ordinal lies in exactly one slot's invariant ordinal core across every legal
emission-count assignment. For producer slot `j`, define:

```text
L_j = sum(minimum_emissions of slots before j)
U_j = sum(maximum_emissions of slots before j)
```

Actual ordinal `k` is certifiable through slot `j` exactly when
`U_j <= k < L_j + minimum_emissions_j`, and exactly one slot satisfies that relation. The
dependency's descriptor, subject, response, source, and destination contracts must all match that
slot. An ordinal outside every invariant core, or with any contract mismatch, is not a legal
dependency.

A typed failure is domain truth and may commit. Corrupt evidence, callback faults, invalid
canonical output, and broken contract invariants block or reject; they never become an invented
domain failure. `InsufficientEvidence` permits a later audited call for the same immutable request.
`InvalidEvidence` is an integrity verdict and appends no semantic transition.

## Journal Record Algebra

```text
RunJournalRecord ::=
    RunAdmitted
  | StateTransitionCommitted
  | ExternalAccessAuthorized
  | ExternalAccessObserved
  | RunClosed
```

### `RunAdmitted`

The root binds:

- store and tenant scope;
- run id, invocation identity, and stable entry-point operation id;
- certified spec, certificate, authored graph, expanded graph, and planning profile;
- exact configuration, seeds, cross-run source manifest, and initial bindings;
- whole-executable identity and state/capability/executor implementation manifests;
- immutable routing-generation references; and
- the canonical genesis semantic-state digest.

Admission may resolve current configuration and select already qualified immutable local bindings.
It performs no provider, source, chain, signer, route-validity, executor, or other semantic live
probe. Any live bootstrap is an ordinary audited read after admission.

### `StateTransitionCommitted`

One transition records:

- exact node occurrence and transition kind;
- before and after semantic positions;
- complete named input lineage;
- immutable request and selected observation where applicable;
- typed result, output bindings, facts, evidence, or typed failure;
- exact blocking sources for a dependency skip; and
- the binding delta and terminal outcome.

The transition body is exactly:

```text
PureSettled
ReadSettled
EffectRequested
EffectSettled
DependencySkipped
```

Facts, outputs, requests, evidence, and typed failures are fields or referenced objects of the
transition. They are not separate semantic record families.

Successful-settlement output bindings and fact emissions retain semantic ordinal order in the
transition body. The redundant binding delta has a different storage order: the private store
assembler preserves variant-group precedence but canonical-byte sorts repeated output-binding and
fact-binding groups independently. Replay rejects duplicate ordinals, reindexes each repeated
group by its semantic ordinal, and only then compares and applies it. Thus canonical wire order
never becomes output or emission meaning.

### `ExternalAccessAuthorized`

An authorization freezes one request and permits zero or one application-protocol operation:

- a read authorization binds the exact input manifest and immutable typed request;
- an ensure authorization binds the exact `EffectRequested` transition, effect key, executor
  binding, and request digest.

The authorization changes no semantic state. Commit ambiguity, idempotent reload, or
`ExistingSame` never creates live authority.

### `ExternalAccessObserved`

An observation references exactly one authorization and records:

- `Returned`, with one reviewed typed result;
- `DidNotEnter`, with a reviewed safe failure proving no boundary entry; or
- `Indeterminate`, with a reviewed safe failure after entry may have occurred; or
- `NonDomainFailure`, with conservative `ProvenNotEntered | MayHaveEntered` entry status, fixed
  `RetryableOperational | IntegrityBlocked` disposition, and one closed redaction-safe code.

The shared `SafeFailure` envelope is selected by an exact contract reference and contains only a
closed code, failure class, boundary stage, optional coarse size class, and optional bounded typed
diagnostic reference. Provider text, URLs, credentials, bodies, paths, and source chains are
discarded below the journal boundary.

The admitted classifier embeds the optional complete diagnostic `SchemaIdentity`. Candidate
append, committed load, and recorded-history replay invoke no state, capability, adapter, or
classifier callback: they strictly decode that descriptor, derive its schema id, and validate
diagnostic canonical bytes against its complete `SchemaShape`. The same structural path verifies
every observation object's full `ValueRef`, exact producer path, and frozen object intent. It also
reconstructs exact pending and terminal effect-retained closure and the authoritative fact-scan
response, attestation, selected-source closure, and evidence dependencies before granting
observation authority.

The non-domain codes are exactly `adapter_contract_violation`, `result_encoding_failure`,
`fact_store_unavailable`, `fact_history_invalid`, `executor_store_unavailable`,
`executor_contention`, `executor_history_invalid`, and `executor_capacity_exhausted`. Store and
replay validate each code's permitted access layer, entry status, and disposition. A retryable
operational outcome may permit a later separately authorized call only after its observation
commits. An integrity-blocked outcome blocks progress. Neither form is passed to a state callback,
and neither can satisfy a read or effect settlement.

### `RunClosed`

Closure names the independently hashed final semantic transition and its terminal semantic-state
digest. It is legal only when every certified node occurrence is terminal and appears immediately
after that transition in the same commit. It fixes the semantic head permanently.

After closure, no semantic transition or new authorization is legal. One observation may still be
appended for each pre-closure unmatched authorization, and never more than one. These audit-tail
records do not change the semantic head or closure.

## Commit Batch Contract

The exhaustive batch purposes are:

```text
run_admission
  RunAdmitted

pure_settlement
  StateTransitionCommitted(PureSettled)
  RunClosed?

read_settlement
  StateTransitionCommitted(ReadSettled)
  RunClosed?

dependency_skip
  StateTransitionCommitted(DependencySkipped)
  RunClosed?

external_access_authorization
  ExternalAccessAuthorized

external_access_observation
  ExternalAccessObserved

effect_request
  StateTransitionCommitted(EffectRequested)

effect_settlement
  StateTransitionCommitted(EffectSettled)
  RunClosed?
```

There is one top-level record per commit except the terminal settlement plus its `RunClosed`
companion. A fact-emitting settlement carries one tenant fact-publication coordinate. A reserved
fact-selection authorization carries one tenant fact-selection barrier. All other commits carry no
fact coordinate.

`JournalHead` is the latest commit sequence and digest. `SemanticHead` is the latest
`RunAdmitted` or `StateTransitionCommitted` plus its containing commit. Audit-tail observations
advance the journal head but not the semantic head.

## Node And Run Semantics

Node phase is:

```text
Unstarted | AwaitingEffect | Terminal
```

Terminal outcome is:

```text
Succeeded | Failed | Skipped
```

A pure or read state moves directly from `Unstarted` to `Terminal`. An effect moves from
`Unstarted` to `AwaitingEffect` with `EffectRequested`, then to `Terminal` with
`EffectSettled`. A dependency skip moves from `Unstarted` to `Terminal`.

The certified graph owns:

```text
DependencyContract {
    required_input_sources,
    unavailable_input_rule,
}

RunTerminalContract {
    required_success_nodes,
    public_output_binding,
}
```

If every possible producer of a required input is terminal without that output, the consumer
commits `DependencySkipped` naming the complete direct blocking set. Optional absence is an
ordinary typed value, not a scheduler policy. Certification rejects any graph whose terminal
combinations leave readiness, skip legality, public output, or run result undefined.

A closed run succeeds exactly when every required-success node succeeded and the certified public
output binding exists. Otherwise it fails.

## Runtime-Owned Admission And Stateless Drive

Concrete backend qualification returns one `QualifiedRunStore<B>` and its sole issuer. Application
bootstrap provisions current configuration and admits the complete qualified support graph before
consuming the assembly with `split`. Runtime consumes the sole `RunHistoryWriter<B>`; application
and replay services retain only `RunHistoryReader<B>` values.

`Runtime::admit` accepts one opaque owned `AuthorizedAdmissionPlan`, validates its authority,
entry-point operation, invocation, certification artifacts, configured value, sources, and exact
registry, injects only that registry's admitted support, and appends the immutable root. Application
code cannot prepare or append journal material directly. `drive_once` remains stateless across
calls even though the shared process runtime owns the affine writer capability.

Runtime derives one private next action:

```text
CommitPure
ExecuteAccess(PreparedAccess)
SettleRead
CommitEffectRequest
SettleEffect
CommitDependencySkip
Closed
Blocked
```

`drive_once` selects actions in this order:

1. scan every committed observation suffix in physical observation order and block on any invalid
   evidence, including invalid evidence after the first callback-accepted settlement;
2. settle the first callback-accepted structurally consumable observation;
3. commit one ready local pure/effect-request/dependency-skip action;
4. perform one audited read or ensure call, ranked by fewest prior authorizations and then certified
   node order;
5. close only through the final semantic transition; or
6. report a reviewed waiting reason without appending.

Current-candidate unavailability is an operational block. A sealed candidate
identity or callback-integrity failure is an integrity block. Candidate code
execution failure crosses the runtime boundary only as its redaction-safe
typed failure class.

Exact resume rechecks terminal external occurrences against the complete
consumable observation suffix. The first callback-accepted observation must be
the observation recorded by the terminal transition, and any later invalid
evidence still blocks.

For a live call, runtime alone owns the private staged protocol:

```text
Prepared<K>
  -> Authorized<K>
  -> PendingObservation<K>
  -> CommittedObservation<K>
```

Runtime:

1. purely authors and preflights the immutable request and all selected contracts into
   `Prepared<K>`;
2. appends `ExternalAccessAuthorized`;
3. consumes only a directly acknowledged `NewlyAppendedAuthorization` into one affine
   `Authorized<K>`;
4. invokes one registered private boundary and totalizes every surviving return, including
   returned-value encoding failure, into `PendingObservation<K>` with no outer post-invocation
   error channel;
5. commits or exactly resolves the linked `ExternalAccessObserved`; and
6. returns normal success only with `CommittedObservation<K>`, without settling the state in that
   same drive.

The pending value owns one stable logical observation identity: authorization reference plus exact
persistable bytes. A predecessor-bound physical append attempt separately owns its append request
id, predecessor, and candidate. Before each attempt runtime reloads and resolves the authorization
logical key. Identical content completes the protocol; different content is an integrity
conflict. A definite stale predecessor discards only the physical attempt and prepares a new one
over the unchanged pending value. Acknowledgement ambiguity resolves the unchanged original
physical attempt before any rebase. No observation retry reinvokes the live boundary.

Operational store unavailability while the pending obligation is live uses capped exponential
backoff of 10, 20, 40, …, 1,000 milliseconds. Verified head progress or a resolved append resets
the delay. Cancellation drops the in-memory obligation and starts no detached completion work;
task or process loss therefore leaves the committed authorization unmatched rather than
fabricating an outcome.

The next call can consume the observation and settle. A crash at any boundary is recovered from the
journal; no process-local semantic state, worker identity, or claim is needed.

## Read And Effect Workflows

Read:

```text
StateFrame
  -> immutable request
  -> authorization
  -> application-protocol call
  -> observation
  -> state callback
  -> ReadSettled
```

An insufficient observation may be followed by another authorization for the same request.
Invisible transport retry is forbidden.

Effect:

```text
StateFrame
  -> immutable request
  -> EffectRequested
  -> authorization for keyed ensure
  -> executor ensure
  -> observation containing terminal evidence or a pending frontier
  -> state settlement callback
  -> EffectSettled
```

The pending effect transition is the MFM outbox. Runtime does not own mutation protocol phases,
delivery retry policy, resource allocation, signing, nonce selection, or destination convergence.

## Keyed Executor Boundary

`mfm-executor` owns a separate append-only ledger for independently meaningful target operations.
It derives `EffectKey` and `RequestDigest`, verifies an immutable tenant/deployment binding, and
converges repeated `ensure` calls on one request.

The executor protocol is:

```text
committed request identity
  -> exact executor binding
  -> append authorization
  -> affine target-entry authority
  -> target returns an unbound closed outcome
  -> private completion seal validates and binds the outcome
  -> exact observation
  -> bounded delivery frontier and terminal tombstone
  -> verified terminal claim
```

Executor delivery attempts are executor-private audit identities. They are not MFM state phases.
Typed resource allocation is another immutable executor CAS stream whose policy and configuration
references are retained and revalidated on refold.

`KeyedExecutorLedger<Store>` is the sole high-level implementation. It asynchronously loads complete
immutable histories through `ExecutorLedgerStore`, strictly folds them, evaluates typed resource
policy, derives deterministic attempts, and submits one atomic compare-and-append proposal. The raw
store owns only its exact fenced identity, immutable effect/resource records, exact content-object
loads, and CAS; it does not own folding, policy, attempt, observation, or terminal rules.

The sole public target boundary is `execute_target_once`. It commits one target authorization,
consumes the resulting affine authority in one caller-supplied invocation, validates and binds the
returned unbound outcome through its retained private completion seal, and durably records the
exact target observation before returning. A stale append, lost acknowledgement, tombstone
conflict, or restart resolves retained ledger evidence and never calls the target a second time.
Returned target values that are valid for their typed contract but exceed the retained-result
bound become the closed `ResultUnrepresentable` attempt failure.

After request qualification and permanent allocation, the EVM wallet executor consumes every
initial, restored, post-target, authorization/terminalization-conflict, pending, and terminal
decision through one complete domain-history fold before target authorization/observation, signing
or target IO, terminal append, or return. That fold reconstructs the deterministic plan at each
attempt; validates each retained candidate descriptor and typed result; derives terminal request,
prior-result references, candidate lineage, transaction, receipt, finality, inclusion, outcome,
generation, fence, and assurance as one exact relation; and accepts a tombstone only when its
operation, outcome, attempt, returned result, and returned observation match that derived terminal
attempt. Records are folded in immutable append order. Each authorization freezes its expected plan
from authorization-ordered results observed by that point; a later observation is validated against
that frozen plan and never changes an already-authorized plan or its terminal lineage. The first
chronologically observed valid terminal selects terminalization. Every later observation, including
a legal observation after the tombstone, remains validation-only audit evidence. Invalid restored
history is read-only failure. A valid restored tombstone returns without signing, target IO, or
append.

A proposal may bind and allocate in one append, including upgrading a previously bound effect that
has no target attempt. `Applied` is the only append outcome that grants affine target-entry
authority. `AlreadyApplied` and `Conflict` require a fresh load and derivation.
Acknowledgement-ambiguous `OutcomeUnknown` grants no authority. A shared account sequence cannot
advance until the prior allocation's effect has immutable terminal evidence.

Authorization takes one complete schema-qualified target-entry descriptor. Its content object and
the authorization naming its derived reference commit atomically. Recovery resolves the exact
descriptor by effect and attempt before another target entry; a fixed operation-family reference
cannot stand in for candidate-specific public input. Domain descriptors exclude credentials,
signatures, raw signed envelopes, and other bearer material.

Reopen enumerates and strictly refolds the complete effect/resource graph and executor-owned content
inventory under the exact binding. Missing, extra, duplicate, byte-mismatched, forked, or partially
linked content fails closed. Memory and file stores are qualification surfaces only. Production
mutation additionally requires a qualified non-rollback executor generation, stale/sibling-writer
exclusion, authoritative destination convergence/resource fencing, and a tested recovery procedure.
PostgreSQL co-location does not merge executor authority with the MFM journal.

The EVM target preflight is one closed five-variant sum:

```text
Broadcast
TransactionLookup
ReceiptLookup
FinalizedHead
CanonicalInclusion
```

Each variant validates and owns all operation-specific request material before target
authorization. Target invocation makes exactly one transport call and returns one closed result.
After possible exchange, classification is exhaustive: a schema-valid result whose encoding
exceeds the retained bound is `ResultUnrepresentable`; an invalid typed/contract result is
`adapter_contract_violation` with `MayHaveEntered/IntegrityBlocked`; and canonical
encoding/schema construction failure is `result_encoding_failure` with
`MayHaveEntered/IntegrityBlocked`.

## Objects, Facts, And Cross-Run Inputs

Every retained journal value uses the full producer-bound `ValueRef`. Each commit atomically binds
all newly admitted or exact preexisting objects through one `commit_artifact_bindings` relation.
Referenced immutable bytes are retained indefinitely in the current contract; there is no
semantic retention workflow or garbage-collection record.

A transition fact emission carries:

- its dense actual `emission_ordinal` and authorizing `fact_slot_ordinal`;
- descriptor and logical identity;
- complete canonical typed subject;
- typed response value;
- producer transition binding; and
- content identity.

Successful callback proposals are grouped in nondecreasing certified fact-slot order. The store
checks every slot's descriptor and subject/response contracts, enforces its certified minimum and
maximum emission counts, and assigns one dense actual ordinal across all groups. Fact identity uses
that actual ordinal, not the slot ordinal.

Same-run consumers use explicit typed output/fact edges. Deliberate prior-run selection is an
ordinary read:

```text
FactSelectionRequest
  -> ExternalAccessAuthorized at TenantFactFrontier
  -> private authoritative-writer scan
  -> ExternalAccessObserved(FactSelectionResponse)
  -> ReadSettled
```

The reserved capability has no process read invoker. Its admitted binding's
`routing_catalog_ref` is itself the singleton internal routing generation; runtime retains and
passes that exact root after validating the reserved scan contract, without selecting a child
generation or consulting a second catalog.

Only a freshly committed reserved authorization returns the affine, non-cloneable scan permit. The
private scan verifies every dense tenant fact-publication order through the committed barrier,
including the consuming run's publications for coverage while excluding them from selection. Its
private cursor is either the next `(fact_order, fact_ordinal)` or explicit completion. Each step
loads and verifies complete ordered producer publications, then consumes only the cursor-selected
logical emission range. The 4,096-publication and 8,192-emission limits bound one step only; a
publication split by the emission budget is fully reloaded and revalidated before its remaining
range is consumed. Backends choose neither ranges nor budget policy. Continuation state is
in-memory and non-serializable, and there is no public or persisted resume token. Process loss
abandons that authorization as audit-only; the next attempt appends a fresh current-head
authorization and starts a new deterministic scan. Partial work cannot mint a response or
completeness proof.

Once the scan reaches `Complete`, its affine result is consumed exactly once into immutable sealed
pending-observation material. Observation reload, exact-content resolution, stale-head retry, and
acknowledgement recovery reuse those same bytes and never rescan. A normally returned store
unavailability or invalid-history result is totalized into the corresponding fact-layer
`NonDomainFailure`.

Only sources selected by the final top-k results survive. Each selected publication independently
closes its claim, subject, response, descriptor, and transitive typed references against the exact
producer journal prefix ending at that publication. Broader producer-run lookup authority is
private scan-builder state and is discarded before `CompletedFactScan` exists.

Within a source closure, a typed `ContentRef` is a semantic dependency. Its exact-prefix bytes are
resolved through the lexicographically least canonical matching `ValueRef`; that authority is
transport proof only, so it is excluded from semantic object references and its own evidence
metadata is not followed. The bytes are strictly decoded under the `ContentRef` schema and their
annex-projected references are traversed. A typed `ValueRef` is a semantic object reference; its
evidence-contract `ContentRef` is added and its strictly decoded payload references are traversed.
When a `ContentRef` resolves bytes whose schema is exactly `mfm.value-ref.v1`, that decoded wrapper
remains transport-only and traversal stops; the incoming `ContentRef` is already the semantic edge.
Missing or conflicting authority and payload-declared cycles reject the closure. The combined
semantic `ContentRef` and typed `ValueRef` set is limited to 65,536 unique references.

The authored response `ContentRef` is the closure root and is not repeated among ordered
dependencies; its authored `ValueRef` is included among ordered object references. Transport-only
authorities do not enter the source-closure digest but are carried as exact preexisting graph
material. The response, closure attestation, object bindings, observation, and private routing row
are one atomic generic append. `CompletedFactScan` consumes itself into the generic observation
material and derives its sealed authorization reference internally. After runtime selects
compatible returned evidence, live reducer entry and replay completeness reapply the same
predicate to that row against the exact verified consuming-run view and fixed producer prefixes.

Those imported producer authorities remain unbound `RequireExisting` observation graph material;
the response and attestation alone are bound `AdmitOrVerifyExact` products. A single-run physical
replay cannot reprove prior cross-run existence, so it may provisionally accept first-use
`RequireExisting` only for unbound imports in an observation carrying a scan attestation. Before
constructing a verified view, semantic replay must identify the exact reserved authorization,
validate the two produced values, consume the entire imported graph, and reconstruct the exact
selected-source closure and attested digest with no omission, extra, or substitution. Live
backends still require each imported exact authority to exist globally before the atomic append.

Cross-run domain corrections are ordinary separately certified runs. A correction consumes an
exact semantically closed source through `CrossRunSourceRef` in a dedicated correction-evidence
input role. It cannot satisfy an ordinary value, output, or fact slot. A correction invocation
identity is derived from the frozen correction preimage containing source bindings, typed purpose,
affected resource or subject, and optional sequence. Store admission uses the normal logical-start
key; there is no correction-specific store path.

## Store And PostgreSQL

`mfm-store` owns:

- the affine pre-split assembly, sole writer, and purpose-specific readers;
- canonical record and commit hashing;
- exact per-run predecessor validation;
- logical-key and batch legality;
- append idempotency and exact-head compare-and-swap;
- object admission and commit bindings;
- tagged tenant fact coordinates;
- closure and post-closure tail validation;
- the callback-free structural fold; and
- minting `CommittedRunJournal`, `VerifiedRunView`, and direct-new-authorization permits.

The irreducible production schema is:

```text
store_identity
store_schema_metadata
tenant_fact_order_heads
journal_commits
journal_records
artifact_blobs
artifact_admissions
commit_object_authorities
commit_artifact_bindings
qualified_support_members
fact_scan_attestations
configured_values
```

Normalized indexes and routing columns do not create a second semantic model. No persisted
current-status mirror is authoritative.

PostgreSQL append ordering is:

1. stage and verify object bytes without granting authority;
2. begin one transaction; an admission acquires its canonical logical-start advisory lock and then
   the resolved existing or proposed run advisory lock, while a successor acquires only its run
   advisory lock;
3. resolve idempotency, load and verify the complete current journal and reachable object closure,
   and validate the entire sealed candidate;
4. for fact publication or selection barrier only, lock the tenant fact-order head last;
5. derive the coordinate, record ids, hashes, commit digest, rows, and object bindings; and
6. commit atomically.

The implementation never uses the latest commit row as its mutex. Connection ambiguity returns
`OutcomeUnknown`; resolving the append request may prove it exists, but cannot recreate the
directly observed live-access permit.

One `(store_scope_id, store_epoch)` has one authoritative writable lineage. The deployment fence
must prevent stale or sibling writers and prove non-rollback WAL lineage across restore or
promotion. The application role may insert/select only through the qualified store contract and
cannot update, delete, truncate, or directly manipulate identity or fact-head authority.

Every authority-bearing read and write uses that fenced writer. Status, drive, replay loading,
trace, audit, export, object dereference, and fact completeness do not use replicas in the current
contract.
Independent processes may each qualify the same fenced lineage and construct their own sole
runtime writer; PostgreSQL locks, exact-head CAS, and append-request idempotency serialize them.

## One Journal, One Fold, One Verified View

`RunHistoryWriter::load_for_drive`, `RunHistoryReader::load_for_replay`, and
`RunHistoryReader::load_for_export` load native commits, records, and exactly bound objects under
one authoritative snapshot. Public and inspection grants cannot call those seams. The load checks
canonical encodings, identities, order, predecessor links, atomic grouping, object evidence, and
structural legality before returning the opaque, non-cloneable `CommittedRunJournal`.

Consuming that journal with `verify_recorded_history` validates its retained
`ExpandedCertifiedSpec`, `Certificate`, and implementation manifests, performs callback-free
semantic verification, and returns `VerifiedRunView`. The view owns:

- the exact current journal head;
- the latest semantic head;
- an optional fixed closure coordinate;
- the private node/run/binding fold;
- verified object/spec/certificate/evidence access; and
- exact authorization/observation and executor-frontier relationships.

Drive, replay, and export logic may use algorithms over that same privileged view, including:

- `TransitionFrameReader`;
- the reserved private fact-selection scan;
- the folded pending-effect set;
- exact reproduction input reader; and
- `VerifiedComparisonFrameReader`.

The non-privileged purposes are one-call sealed projections over a freshly loaded and verified
view:

- `read_public_run` returns only an owned `VerifiedPublicRunView`;
- `inspect_access_audit` returns a bounded, head-fixed `VerifiedAccessAuditPage`; and
- `discover_transition_trace_sources` followed by `inspect_transition_trace` returns a bounded,
  head-fixed `VerifiedTransitionTracePage`, disclosing source values only under separate exact-run
  inspection authorities.

These projections grant no raw journal or object dereference capability and do not persist or own
another lifecycle snapshot.

## Replay

Replay has three modes:

1. **verify** — callback-free verification of recorded commits, records, objects, graph/certificate
   closure, transition lineage, facts, access audit, executor evidence, and closure;
2. **reproduce** — rerun the exact admitted planner/state canonical computation through the
   matching `QualifiedProgramRegistry` when retained code is available, then compare exact results;
   and
3. **compare current** — self-attest the serving executable and run only the explicitly identified
   current candidate planner/state callbacks, reporting agreement, difference, or non-comparability.

Replay does not invoke the live scheduler, mint access authority, append, resolve current routing,
call a provider or executor, or construct a signer. `unavailable` in the frozen reproduction
response carries no reason field. Recorded verification preserves `NonDomainFailure` as
callback-free audit evidence, applies its fixed retry-or-block projection, and rejects any history
that consumes it in a semantic transition.

## Production Capabilities

Production EVM reads are decomposed into ordinary audited nodes:

```text
routing-generation/source/chain bootstrap
  -> initial anchor
  -> one independently audited metadata or balance RPC node per invocation
  -> final anchor confirmation
  -> pure aggregation
```

Each external method has its own authorization and observation. Fan-out is certified graph
parallelism, not hidden transport concurrency. All reads use the exact admitted routing generation
and anchor; resume cannot reselect a source.

Bitcoin collection is deliberately unregistered under its closed disposition and is outside the
fixed implementation sequence. Its evaluated graph was bootstrap `getblockchaininfo`, one audited
`scantxoutset "start"`, audited block-hash confirmation, then pure aggregation; it did not pass the
repeat-work-safe read qualification described in `docs/btc-rpc-routing.md`. Any future
registration is separately scoped and cannot add another runtime mode.

Production EVM transaction submission is one ordinary effect state:

```text
immutable configured transaction request
  -> exact shared wallet-request qualification before admission
  -> typed sender/nonce allocation in the executor resource stream
  -> guarded deterministic signing before each authorized broadcast
  -> exact-hash transaction and receipt lookup without signer access
  -> finalized-head and canonical-inclusion evidence
  -> terminal success or revert evidence and tombstone
```

The executor PostgreSQL ledger is a separate logical authority from the run journal and uses an
independent writer-generation fence. Every candidate preserves the request, sender, nonce,
destination, value, calldata semantics, access list, gas limit, and signing profile; only the
finite qualified fee schedule may change. A missing transaction, timeout, or observed nonce is
never promoted to a generic not-applied terminal result. Raw signed bytes and signatures remain
transient and zeroizing inside the broadcast target.

`mfm-evm-live` owns one concrete exact-generation transport with a private allowlist of six read
and five wallet operations. `EvmWalletExecutor<Store>` is the only public wallet execution seam and
obtains that concrete transport only through its qualification; no independent transport argument,
generic wallet RPC client, raw response, target wrapper, or target-entry descriptor is public.
MFM-owned authorization, request-body, signed-envelope, and response buffers are bounded and
zeroizing. Decoding retains borrowed raw ranges, closes the JSON-RPC envelope and typed result
shapes, and discards provider text below safe-failure evidence.

The shared qualification is created without signer, route, ledger, or provider IO from the sealed
transport catalog and immutable public deployment values. It binds the complete ordered
route-generation-to-chain map and product object-evidence contract, derives the generic guarded
signer descriptor and EVM nonce-policy pair, and verifies the exact wallet executor semantic
closure. Its canonical proof, content reference, and debug representation exclude transport
internals, endpoints, and authorization. The live qualification privately retains a clone sharing
the exact transport runtime and route catalog, and instance-aware equality prevents an independently
constructed transport with identical public descriptors from comparing equal. The application
rejects a configured mismatch before certification or append; the executor rejects it again before
effect binding or nonce allocation. No caller can supply an independent transport, signer
reference, resource-policy pair, or selected route descriptor.

## Application And Public Surface

The complete run-facing facade is:

```text
entry_points
admit_run
drive_once
read_public_run
replay_run
read_transition_trace
read_access_audit
export_run
```

Every run operation except entry-point discovery authenticates one opaque credential, derives one
tenant, authorizes one exact grant and target, and mints a sealed affine
`RunAccessAuthority<G>`. Grants are `Admit`, `Drive`, `Replay`, `ReadPublic`, `InspectTrace`,
`InspectAudit`, and `Export`.

The published transport contract is:

| Purpose | REST | CLI |
| --- | --- | --- |
| Entry points | `GET /v1/entry-points` | `mfm ops list` |
| Admit | `POST /v1/runs` | `mfm run admit` |
| One action | `POST /v1/runs/{run_id}/drive` | `mfm run drive` |
| Status and public output | `GET /v1/runs/{run_id}` | `mfm run show` |
| Replay | `POST /v1/runs/{run_id}/replay` | `mfm run replay` |
| Transition trace | `GET /v1/runs/{run_id}/trace` | `mfm run trace` |
| Access audit | `GET /v1/runs/{run_id}/audit` | `mfm run audit` |
| Portable export | `POST /v1/runs/{run_id}/exports` | `mfm run export` |

Public status is exactly `active | succeeded | failed`. The ordinary run read exposes only derived
status, reviewed active-run fields, and certified public outputs. Trace, audit, replay, and export
are separately authorized. There is no arbitrary object reader or tenant-wide discovery surface.
The access-audit projection exposes the optional closed `non_domain_failure` value and never
recasts it as `failure`, a returned value, or a domain result.

Portable run transfer is the annex-defined `mfm.portable-run-export-stream.v2` framed JSON text
sequence. It begins with one header, carries root-first journals and deduplicated object payloads
with every logical `ValueRef` authority, and ends with one terminal frame followed immediately by
EOF. The external content digest is SHA-256 over every exact record separator, canonical frame
byte, and line feed; no frame contains a self-digest.

The exact DTOs, disclosure rules, pagination, authentication, and portable-export contract are in
`docs/recoverability-app-surface-v3.md`.

## Security And Redaction

Secrets include passwords, mnemonics, private keys, raw signing material, credentials,
authorization headers, unlock material, signed bearer payloads, and secret-bearing paths. They
remain below typed semantic and diagnostic boundaries.

MFM-owned transient EVM buffers use owner-preserving zeroizing allocations and exact limits. This
guarantee covers allocations controlled by MFM; it does not claim that HTTP/TLS libraries, the
allocator, the operating system, or a remote peer zeroize their own internal buffers.

Capability classifiers discard provider-controlled text and retain only safe values that satisfy
the classifier-bound complete diagnostic identity and structural shape. Append, load, and replay
then recheck that identity and shape, full producer-qualified `ValueRef`s, frozen object intents,
effect-retained closure, and fact-source closure without invoking callbacks. Public errors are
closed redaction-safe codes. A record id, run id, value reference, content digest, cursor,
portable stream, or export content reference grants no access.

## Documentation Update Rules

Update this document with every change to:

- the five-record algebra or batch legality;
- certified state/planning contracts;
- store hashing, object, fact, or writer authority;
- runtime action selection;
- external access or executor semantics;
- replay and correction semantics;
- capability registration;
- public outputs or app/CLI/REST surfaces; or
- ownership boundaries.

Use `docs/architecture.md` for placement rules and
`docs/persisted-public-surfaces.md` for the no-secret review inventory.
