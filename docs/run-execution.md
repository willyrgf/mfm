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
  -> callback-free fold derives cursor
  -> repeated drive_once calls
  -> RunClosed beside the transition that derives the root outcome
```

Admission performs no semantic live IO. App resolves one exact append-only configured-value
revision, authors the candidate program, asks the qualified entry registry to certify it, and
submits the complete certified closure and immutable admission material to Runtime. The store
re-certifies the persisted authored object, checks the exact certified root/document, and appends
the admission atomically.

An operation with no state appends `RunAdmitted` and `RunClosed` together. Otherwise the fold
derives the first actionable `State` occurrence. Structural `Match`, `FanOut`, child fragment, join,
and outcome expressions are normalized without control records.

## Cursor selection

Outside fan-out there is one current state. Inside fan-out the fold exposes declaration-ordered
actionable lane paths. Runtime selects the minimum path. A waiting Read does not prevent a later
independent lane from becoming actionable, but a structural barrier or possible Effect entry stops
the scan. A join is derived only after every lane has one canonical lane outcome, and its values
are ordered by declaration rather than completion time.

The fold reports one of:

- actionable state occurrences;
- waiting unmatched Reads;
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
| Effect | `EntryUnknown` | Parks possible target entry. |
| Read/Effect | `IntegrityFault` | Committed evidence blocks the run. |

An unmatched Read waits; it is not silently retried or synthetically completed. Only committed
`SupersededBeforeEntry` permits a new attempt for the same Effect occurrence. A stale worker cannot
self-upgrade its physical authority.

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
same deployment fence, loads the full prefix, re-runs the callback-free fold, and continues from
the same cursor. It never restores callback state or delegates to memory.

## Writer fencing

The run-history store is scoped by a stable store identity and monotonically qualified writer
epoch. Deployment supplies the authoritative writer fence before opening the writer. Stale or
sibling lineages fail closed. This fence is independent from the EVM wallet target/session fence.

## Replay and inspection

Run read, replay verification, transition trace, access audit, and portable export all load a
`VerifiedStructuredRun` through the store reader. They do not execute callbacks or live IO.
Trace/audit pages bind one exact journal head and use stable zero-based positions. Portable exports
contain the exact records and content-addressed object closure under the current structured export
schema.

## Public one-action behavior

`drive_once` returns a reviewed disposition after at most one action:

- transition committed, with whether it also closed;
- access observed;
- concurrent progress;
- waiting Reads;
- possible entry;
- blocked integrity; or
- already closed.

CLI and REST expose this bounded behavior. They do not loop a run to completion.

## Verification map

- `mfm-program`, `mfm-spec`, and `mfm-certify`: authoring, substitution, bounds, failure plans,
  exhaustive Match, nominal results, and expansion/certification goldens.
- `mfm-store`: hostile history, exact closure, atomicity, fold equivalence, and memory semantics.
- `mfm-runtime`: callback counts, affine access, ambiguity, settlement, and concurrent progress.
- `mfm-storage-postgres`: SQL rollback, fresh-process continuation, numeric ordering, and writer
  qualification.
- `mfm-replay` and integration tests: callback-free projections and strict current wire contracts.
