# Structured Run Execution

Status: current execution guide

## Material uncertainties

none

## From admission to closure

```text
typed request
  -> deterministic authored program
  -> qualified pure certification
  -> RunAdmitted
  -> qualify -> reduce -> cursor
  -> repeated drive_once calls
  -> RunClosed beside the transition that derives the root outcome
```

Admission performs no semantic live IO. App resolves one exact append-only configured-value
revision, authors the candidate program, asks the qualified entry registry to certify it, and
submits the complete certified closure and immutable admission material to Runtime. The store
re-certifies the persisted authored object, checks the exact certified root/document, and appends
the admission atomically.

An operation with no state appends `RunAdmitted` and `RunClosed` together. Otherwise the reducer
derives the first actionable `State` occurrence. Structural `Match`, `FanOut`, child fragment, join,
and outcome expressions are normalized without control records.

## Cursor selection

Outside fan-out there is one current state. Inside fan-out reduction exposes declaration-ordered
actionable lane paths. Runtime selects the minimum path. A structural barrier or possible Effect
entry stops the scan, and an unobserved Read keeps its lane actionable rather than exposing a later
one, because it is re-assertable rather than waiting. A join is derived only after every lane has one canonical lane outcome, and its values
are ordered by declaration rather than completion time.

The reducer reports one of:

- actionable state occurrences;
- possible Effect entry;
- committed integrity blocking; or
- complete root outcome.

There is no global ready queue, arbitrary edge traversal, or mutable cursor row.

## Pure state

For a ready `Pure` state, Runtime loads the exact current input object and invokes the registered
deterministic callback. The callback returns one proposed typed success or failure and optional
facts. The store validates the exact cursor, contracts, provenance, semantic head, object/fact
closure, and append head before committing `StateTransitionCommitted`.

A proposal has no authority before append. Concurrent progress yields a stale-head disposition and
does not commit the callback result against a different semantic state.

## Read and Effect state

For a ready `Read` or `Effect`, the registered state first authors an exact request. Runtime obtains
the current qualified public physical-binding certificate and prepares an authorization candidate.
Only a newly committed or exact already-committed authorization can proceed.

```text
ExternalAccessAuthorized
  -> one affine invoker authority
  -> one bounded live operation
  -> immutable pending observation
  -> ExternalAccessObserved
  -> registered state settlement
  -> StateTransitionCommitted
```

Runtime owns every stage. A process callback, app, or adapter cannot receive a successful result
between invocation and observation persistence. If the observation acknowledgement is ambiguous,
Runtime resolves the original append identity. A stale observation envelope may be rebuilt, but
the invoker is never called again.

The current completion variants are:

| Kind | Variant | Cursor effect |
| --- | --- | --- |
| Read/Effect | `Returned` | Exact reviewed value may reach settlement. |
| Read/Effect | `SafeFailure` | Exact reviewed failure may reach settlement. |
| Effect | `SupersededBeforeEntry` | Purpose-limited proof creates the next ordinal. |
| Effect | `EntryUnknown` | Parks possible target entry, or re-asserts at the next ordinal under a declared `EntryAbsorbing` with budget remaining. |
| Read/Effect | `IntegrityFault` | Committed evidence blocks the run. |

An unmatched Read is re-assertable at the next ordinal, because a Read consumes no externally
meaningful state; nothing is synthetically completed. Only committed `SupersededBeforeEntry` permits
a new attempt for the same Effect occurrence. A stale worker cannot self-upgrade its physical
authority.

Current Effect-entry attention is derived from the reduced leaf, never from an index or transport:

| Reduced Effect state | Resolution | Required action |
| --- | --- | --- |
| Authorized without observation, or terminal `EntryUnknown` | `Manual` | Inspect and resolve without automatic re-entry. |
| Absorbing attempt authorized without observation, with budget remaining | `CloseThenReassert` | Commit the reserved lost-invoker closure, then re-assert. |
| Absorbing attempt observed `EntryUnknown`, with budget remaining | `Reassert` | Re-authorize the byte-identical request at the next ordinal. |

The inventory carries the exact occurrence, path, attempt, and capability contract plus this closed
resolution. Its route and keyset cursor are disposable locations; each returned item is re-derived
from qualified history at the exact reported head.

## Settlement and failure handling

Only `Returned` and `SafeFailure` reach registered state settlement, through distinct callbacks:

- **Returned-value settlement** receives one schema-valid returned observation and may propose
  success, propose typed failure, or reject as `InvalidEvidence`.
- **Safe-failure settlement** receives every inhabited admitted safe-failure value. Its return type
  is disposition-typed: under `SafeFailureSuccessOnly` it is a success-only proposal (no `Failure`
  or `InvalidEvidence` variant); under `SafeFailureMayFail` it is a proposed success or typed
  failure. Totality is type-enforced for every valid value; qualification does not rely on a
  reviewed sample corpus.

Invalid returned evidence does not append a diagnostic event and leaves the authoritative cursor
unchanged. Adapter unavailability and other infrastructure faults leave the attempt uncommitted;
they are never translated into a semantic failure record by settlement.

Typed state failure follows its certified failure plan. A default mapper is an ordinary infallible
Pure state. Custom recovery is ordinary Match/State structure. An ordinary failed root closes
normally and does not block a different `run_id`.

If the same append first derives the root operation outcome, the candidate must include
`RunClosed`; omitting or delaying it is invalid. Nothing can append after closure.

## Facts

Facts are proposed only by a state callback and share its transition append. Each committed fact
binds its certified slot ordinal, emission ordinal, descriptor, typed subject and response, and
content-addressed claim. The append object set must equal the exact newly reachable closure.

Prior-run fact selection is an ordinary Read. Its authorization captures the exact tenant frontier
and grants a purpose-limited scanner; it is not a special execution kind.

## Concurrency and recovery

Each append uses an exact expected head. Concurrent writers can produce only one committed
successor. A backend response is normalized against the store-retained candidate; a positive
backend response cannot substitute different records or objects.

Runtime distinguishes:

- `NewlyCommitted`;
- `ExistingSame` for exact idempotent resolution;
- `StaleHead` for concurrent progress; and
- `AcknowledgementUnknown` when durability cannot yet be determined.

The PostgreSQL backend persists raw assigned batches and objects. A fresh process opens through the
same deployment fence, loads the full prefix, re-qualifies and re-reduces it, and continues from
the same cursor. It never restores callback state or delegates to memory.

## Writer fencing

The run-history store is scoped by a stable store identity and monotonically qualified writer
epoch. Deployment supplies the authoritative writer fence before opening the writer. Stale or
sibling lineages fail closed. This fence is independent from the EVM wallet target/session fence.

## Replay and inspection

Run read, replay verification, transition trace, access audit, and portable export each load
through a distinct purpose reader and receive only that purpose's sealed evidence newtype. The
shared reduction still derives one internal `VerifiedStructuredRun`, but purpose APIs never return it:
public-read, trace, audit, replay, and export evidence types are not interchangeable. They do not
execute callbacks or live IO. Trace/audit pages bind one exact journal head and use stable
zero-based positions. Portable exports contain the exact records and content-addressed object
closure under the current structured export schema.

## Public one-action behavior

`drive_once` returns a reviewed disposition after at most one action:

- transition committed, with whether it also closed;
- access observed;
- concurrent progress;
- possible entry;
- blocked integrity; or
- already closed.

CLI and REST expose this bounded behavior. They do not loop a run to completion.

## Verification map

- `mfm-program`, `mfm-spec`, and `mfm-certify`: authoring, substitution, bounds, failure plans,
  exhaustive Match, nominal results, and expansion/certification goldens.
- `mfm-store`: hostile history, exact closure, atomicity, reduction equivalence, and memory semantics.
- `mfm-runtime`: callback counts, affine access, ambiguity, settlement, and concurrent progress.
- `mfm-storage-postgres`: SQL rollback, fresh-process continuation, numeric ordering, and writer
  qualification.
- `mfm-replay` and integration tests: callback-free projections and strict current wire contracts.
