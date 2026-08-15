# RFC: establish the core Runtime, Journal, and Store proof path

Status: selected implementation target; the material validation artifact below remains required
before implementation planning

This RFC is the clean-slate target for the MFM core proof path. It replaces the conflicting runtime,
storage, replay, configuration, fact, tenant, scope, and writer-epoch contracts in
RFC_REFACTOR_SINGLE_TRUST_BOUNDARY.md, docs/design.md, and docs/architecture.md. Implementation must
update those authoritative documents in the same cutover so the repository ends with one current
contract.

There is no compatibility period, legacy decoder, dual schema, or fallback execution path.

## Decision

The target has four semantic owners:

| Owner | Sole responsibility |
| --- | --- |
| Program | The immutable State-or-Match graph and its exact persisted value, State, capability, and binding associations |
| Runtime | Program association, the sole semantic reducer, exact registered State entry, typed execution, bounded caller-driven progression, and RunView |
| Journal | Exact canonical run-frame wire, content addressing, structural qualification, recursive heads, frame-local object closure, and fixed format limits |
| Store | Mechanical complete loads, exact-head compare-and-append, idempotent physical equality, durable reservation arithmetic, and atomic PostgreSQL or Memory persistence |

Application and transports parse requests, supply explicit identities, call Runtime, and render
reviewed outputs. They own no execution session, pending append, recovery token, reducer, or retained
frame interpretation.

One PostgreSQL database is one MFM authority. RunId is the complete durable run namespace. The core
contains no tenant, Store scope, writer epoch, active deployment identity, identity rotation, or
replacement incarnation concept.

Facts are not part of this target. No current admitted capability consumes prior-run facts, so the
fact selection/publication/proof system is deleted rather than rebuilt speculatively.

Independently published configuration is not part of this target. Domain material required to
reproduce a run is retained in the Program or typed admitted context `C0`; deployment clients,
credentials, and other live bindings remain process-local in RuntimeAssembly and adapters. The
MfmConfig contract, configuration streams, latest lookup, and configuration persistence are
deleted.

Portable bundles and offline inspection are not part of this target. No current core consumer
requires a second complete-history ingress or export format. Runtime reads and qualifies durable
history only through Store. A future portability feature requires a consumer-driven RFC rather
than reserving another wire surface now.

Runtime has no cancellation concept. A supported caller that starts a mutating Runtime future must
drive it to completion. Runtime does not add cancellation tokens, timeout wrappers, detached
completion tasks, pending-result custody, or Runtime-owned semaphores. Synchronous State work is
awaited through spawn_blocking only to protect the async executor. Adapters and Store own the
waits, timeouts, pools, and explicit ambiguous-result classifications for their live IO.

Process termination is outside the completion contract. After restart, durable history is the only
recovery authority.

## Material uncertainties

No material architecture choice remains open. This concrete validation artifact is still required
before implementation planning:

1. **Transport retry inventory.** Runtime retains nothing after indeterminate admission. Callers
   must reproduce the same explicit RunId, Program, and canonical typed `C0`. Current CLI and REST
   retry behavior has not yet been inventoried against that rule. If callers cannot reproduce those
   inputs, an indeterminate request cannot be retried honestly. Resolve by freezing request,
   response, and retry behavior for every current transport.

The following are settled contracts, not uncertainties:

- supported callers drive every started mutating Runtime future to completion;
- Runtime has no cancellation API, timeout, detached completion, or Runtime-owned semaphore;
- Application/composition may bound top-level concurrency, while Adapter and Store own live-IO
  wait control;
- an Effect may park after Adapter returns Unresolved or after its conclusion append is
  indeterminate and later absent;
- blocking State/codec callbacks are trusted, bounded-input, pure, and terminating;
- facts, independently published configuration, portable bundles, tenant, scope, epoch, identity
  rotation, and database incarnation are absent;
- every frame carries all canonical objects directly named by its record; there is no
  history-dependent first-reference dictionary, object-count limit, or object reservation;
- exact qualified record equality uses the closed record fields and content references directly;
  there is no separate record digest;
- trace and access-audit DTOs are not core Runtime APIs;
- independent execution uses a fresh caller-supplied RunId.

## 1. Problem situation

### 1.1 Internal layers discard typed proof

The domain surface begins with exact associations:

- State input, output, and failure types;
- capability intent, evidence, Read-or-Effect mode, and attempt bound;
- typed ProposedStateOutcome;
- affine PreparedAccess and CommittedCall owners; and
- typed values that already passed canonical qualification.

The current implementation repeatedly turns those associations into contract references, canonical
strings, TypeId, Box of Any, option bags, catalog brands, Store brands, and backend DTOs. Later
layers compare the same pieces and downcast them again.

That is useful only at a real trust or heterogeneous dispatch boundary. Inside one trusted process
it creates more invalid combinations, validators, translations, public types, and future change
sites than the proof requires.

### 1.2 Store became the semantic state machine

Store currently opens under identity brands, qualifies Journal history, retains and reifies the
Program, reduces State-or-Match, selects the next action, qualifies configuration and facts,
constructs semantic owners, and then maps those owners onto a second backend vocabulary.

The resulting cold path is backwards:

~~~text
PostgreSQL rows
  -> backend DTOs
  -> Store qualification
  -> Store Program reduction and next-action selection
  -> Runtime reconstruction
  -> State execution
~~~

Persistence should return retained bytes. Runtime should interpret those bytes under the retained
Program and decide what runs next.

### 1.3 Journal mixes wire data with process-local proof

Journal records must be canonical and append-only, but open DTO constructors and repeated validate
methods make every consumer re-decide whether the same bytes are structurally valid. Construction,
Store append, Store load, replay, and Runtime each repeat parts of the same validation.

Journal needs two opaque directions instead:

~~~text
trusted typed inputs -> Journal encoding -> EncodedRunFrame
untrusted retained bytes -> Journal decoding -> JournalHistory
~~~

Store must not reopen either proof.

### 1.4 Duplicate registries describe one association

Program currently owns value/capability reifiers and TypeId associations while Runtime owns State
and adapter registrations. Store uses the first registry before Runtime uses the second.

Only two representations are necessary:

- stable contract and implementation associations in the persisted Program; and
- one trusted RuntimeAssembly associating them with concrete Rust implementations.

### 1.5 Runtime lifecycle leaks into Application

Application currently drives one State at a time and retains suspended Runtime/Store owners. It
infers status from frames and participates in acknowledgement recovery.

Runtime already owns all information needed to advance or inspect a run. Application should see
only start, resume, read, RunView, and reviewed errors.

### 1.6 Unused protocols dominate the core

The current fact surface and the proposed authenticated fact redesign introduce selection modes,
proposal sets, publication coordinates, global heads, Merkle structures, dependency DAGs, proof
limits, global serialization, and portable dependency material. Every reachable production
capability uses no prior facts and every current success proposes an empty set.

The target removes that protocol. A future consumer-driven fact RFC may introduce a new clean
format only after defining its real key, multiplicity, freshness, retention, threat model, and
operational envelope.

The current independently published configuration protocol similarly adds a second value
lifecycle, latest-versus-exact lookup, CAS, revisions, SQL, retry behavior, and admission coupling.
No retained domain value needs that lifecycle: reproducible policy belongs in Program or `C0`, and
live deployment material belongs in assembly/adapters. The target deletes the protocol instead of
preserving a unit or empty configuration.

Portable bundles add another complete-history wire, decode budget, and semantic ingress solely for
an unused export/inspection surface. Store-backed Runtime read is the one supported inspection
path. Portability may return only with a concrete external consumer and its own threat and
transport contract.

### 1.7 Persisted identities model authorities outside the product

StoreScopeId, StoreEpoch, TenantScopeId, and StructuredStoreIdentity flow through frames, hashes,
SQL, facts, configuration, App, replay, and EVM nonce authority even though the selected product
has one database authority and no in-core tenant isolation model.

Their removal is a semantic deletion, not a rename. Database restore while an old process remains
able to write is unsupported operator behavior; the core does not pretend to fence it.

## 2. Architecture and dependency direction

### 2.1 Target dependency graph

| Crate/layer | May depend on |
| --- | --- |
| Values/ids | foundational parsing, schema, and canonical primitives only |
| Program | Values/ids and capability contracts |
| Capabilities | Values/ids |
| Journal | Values/ids canonical primitives and its own record types |
| Store trait | ids and opaque Journal representations |
| Memory/PostgreSQL | Store trait and storage-driver primitives |
| Runtime | Values, Program, Capabilities, Journal, and Store trait |
| Domain operations/States | Values, Program, and Capabilities; never Store |
| Adapters | Capability/Runtime ingress plus reusable transports/signers |
| App/transports | Runtime plus explicitly composed domains/adapters |

Program does not depend on Runtime or Store. Journal depends only on stable ids, canonical/value
representation, and its own record types. Store depends on stable ids and opaque Journal
representations, never on Program, State, capabilities, domains, or Runtime selection types.
Runtime receives retained bytes only from Store, passes them through Journal structural decoding,
and then performs semantic reduction.

### 2.2 Validation ownership

Validation remains where trust changes:

| Proposition | Owner and result |
| --- | --- |
| Transport request is bounded and syntactically valid | App/transport typed request |
| A value is canonical, float-free, contract-bound, and content-addressed | Runtime value registration yields ProvenValue of T |
| A Program document is canonical and structurally valid | Program decoder yields Program |
| A Program is completely supported by one process assembly | Runtime yields ExecutableProgram |
| A frame is exact canonical target wire | Journal yields an opaque qualified type |
| A retained run is a complete structurally valid hash chain | Journal yields JournalHistory |
| A history is semantically valid under its retained Program | Runtime fold yields a private reduced state or RunView |
| Provider evidence matches the exact call | Typed adapter/capability ingress |
| The expected durable head remains current | Store transaction |
| A persisted row set matches its canonical bytes and physical metadata | Store load plus Journal qualification |

The following repeated validations disappear:

- Store reifying Program values before Runtime does so again;
- Store reducing the Program before Runtime reconstructs the selected State;
- backend DTO layers reopening Journal frames;
- replay maintaining another reducer;
- catalog and Store brands standing in for exact type association;
- process-local identity fields rechecked in every owner;
- fact completeness and publication checks for an unused protocol; and
- App inspecting frames to recover Runtime status.

### 2.3 Concepts that remain intentionally private

Runtime may use small private enums or local variables for fold state and one active State driver.
Their exact names are not architecture. They must not become public lifecycle algebras, persisted
owners, Store inputs, App tokens, or another erasure protocol.

There is no Runtime pending-owner table of any kind.

## 3. Program, values, and Runtime assembly

### 3.1 State failure contract and Never

The core State contract is:

~~~text
State {
  Input: MfmValue
  Output: MfmValue
  Failure: MfmValue
  stable State implementation identity
}
~~~

Failure no longer implements a framework FailureValue constructor.

The framework owns one uninhabited Never type with the reserved semantic contract mfm.never. It has
no valid canonical value. RuntimeAssembly installs its value registration automatically.

- Pure States may use Never as their exact failure contract.
- Access registration must provide a pure BlockedIntegrityProjection for its exact State and
  capability association.
- Its exact shape is `(&S::Input, &C::Evidence) -> S::Failure`; it performs no IO and cannot choose
  a successor, retry, or alternate evidence. Using the State input lets failures retain public
  correlation such as EVM stage and collection ordinal.
- RuntimeAssembly rejects Access registration when the State failure contract is Never.
- A retained failure outcome for a Pure State whose failure contract is Never is invalid history.

This separates ordinary State fallibility from the Access requirement to represent an accepted
integrity block.

### 3.2 Persisted Program associations

Program retains one strict immutable State-or-Match document. Every State declaration persists:

- stable State implementation reference;
- exact input, output, and non-optional failure contracts;
- Pure or Access kind;
- for Access, the exact capability contract, intent contract, evidence contract, Read-or-Effect
  mode, a total attempt bound for Read (Effect is exactly one), adapter/binding association, and
  physical public target identity; and
- the next control edge or terminal contract.

The Program also persists its one entry point and exact admitted-context contract. `C0` is the
domain-owned typed admitted-context value and the first complete State context, not a transport
request wrapper. Every secret-free domain input needed to reproduce this run but not fixed by the
Program is carried in `C0`.

Program validates graph shape, reachable declarations, stable contract continuity, Match tag and
payload continuity, and fixed graph/size limits. It performs no State execution, provider IO,
Store access, TypeId lookup, or erased value reification.

ProgramCatalog, catalog branding, public value reifiers, erased capability callbacks, and
process-local catalog identity are deleted.

### 3.3 One Runtime assembly

RuntimeAssemblyBuilder owns:

- one exact value codec per stable value contract, schema, and Rust type association;
- one exact State registration per complete persisted State execution association;
- one exact adapter/capability registration per complete Access association;
- one pure schema-derived Match projection for each registered closed-sum value.

Assembly finalization rejects duplicate or inconsistent registrations. Associating a retained
Program produces ExecutableProgram only when every reachable declaration has one exact compatible
registration.

The lookup key is the complete persisted association, with no implementation-only or
capability-only fallback:

~~~text
StateExecutionKey {
  implementation_ref,
  input_contract_ref,
  output_contract_ref,
  failure_contract_ref,
  kind:
      Pure
    | Access {
        capability_contract_ref,
        intent_contract_ref,
        evidence_contract_ref,
        mode: Read { total_attempt_bound } | Effect,
        binding_ref,
        physical_target_ref
      }
}
~~~

`binding_ref` selects the compiled adapter association. `physical_target_ref` commits to the exact
secret-free public target descriptor consumed by that adapter. Credentials and clients remain
process-local and are never key material.

### 3.4 One private heterogeneous driver boundary

The retained Program selects a concrete Rust State at runtime, so one private object-safe dispatch
is unavoidable. RuntimeAssembly stores one private `RegisteredState` driver per exact execution
key. That single driver boundary owns the whole selected State attempt:

- decode or downcast the exact input;
- prepare or evaluate `S`;
- for Access, retain `PreparedAccess<S, C>` through preparation append and retain
  `CommittedCall<S, C>` through provider ingress;
- interpret and qualify the exact typed outcome;
- encode and append the resulting Journal frame; and
- return only a small non-generic durable disposition to the outer progression loop.

`PreparedAccess`, `CommittedCall`, accepted evidence, and typed conclusions never cross that
boundary. They live only in the active caller-owned monomorphic driver future and are consumed
before that supported run-to-completion future returns. No existential owner table or second
dynamic lifecycle protocol exists.

One private `HotValue` wrapper may carry a just-qualified value across a successfully durable
append into the next selected driver:

~~~text
ProvenValue of T
  -> private short-lived HotValue
  -> exact RegisteredState.start
  -> ProvenValue of S::Input inside monomorphic S/C code
~~~

Cold values are decoded directly by the selected monomorphic RegisteredState.start from their
Journal-qualified canonical object. `HotValue` is usable only when the subsequent fold proves that
its contract and content reference are the exact selected input at the exact durable head;
otherwise Runtime discards it and uses the cold path. It is never stored, cloned into a DTO, passed
to Journal or Store, exposed to App, or retained across an await outside the active progression
future.

Box of Any is not prohibited inside this one private driver boundary. Any second supported
erase/downcast workflow is prohibited.

### 3.5 No independent configuration lifecycle

The core has no MfmConfig trait, ProvenConfiguration owner, ConfigurationKey, mutable latest
configuration, or configuration registry. Existing authoring structures named Config are migrated
according to what they mean:

- secret-free material required to interpret every run using one Program is persisted in that
  Program;
- secret-free material selected independently for one run is part of typed `C0`; and
- clients, credentials, connection material, and other live bindings remain process-local in
  RuntimeAssembly or adapters; the exact secret-free physical target identity required by an
  adapter association is already persisted in Program.

Concretely, EvmConfig and PortfolioConfig remain ordinary process-local composition/authoring
inputs only. They lose MfmValue/MfmConfig derives and are never passed to Runtime, Journal, or
Store. `plan_submission` and `plan_snapshot` validate them and consume their selected public
material into one exact Program and typed `C0`; their admission plans return only those two durable
inputs. Resume and read therefore need neither authoring structure. Existing `source_refs` plan
outputs disappear with facts.

Runtime never consults ambient or latest configuration while starting, resuming, or reading a
run. A retained Program plus `C0` is the complete durable admission input.

## 4. Runtime design

### 4.1 Sole semantic reducer

Runtime owns one pure fold over:

- an ExecutableProgram;
- one JournalHistory;
- the compatible RuntimeAssembly.

The fold yields one private state:

- Runnable with occurrence, StateExecutionKey, and current retained value;
- Waiting on an unresolved Access preparation;
- Succeeded with the exact retained terminal value;
- Failed with the exact retained declared failure; or
- an explicit semantic inconsistency.

The reducer validates:

- genesis/entry point/Program/context association;
- State-or-Match occurrence order;
- input/output/failure contract continuity;
- exact Access preparation and conclusion relationship;
- Read replacement count and Effect one-entry discipline;
- evidence and outcome contracts;
- Match selector/payload continuity; and
- terminality.

It does not execute State preparation/evaluation/interpretation, call an adapter, perform provider
IO, or re-author a Program.

Execution, resume, and read all use this reducer. There is no Store reducer or replay reducer.

### 4.2 Direct-new provider-entry proof

Only this transition grants provider-entry authority:

~~~text
local PreparedAccess of S/C
  + Store returned Inserted for that exact preparation append
  -> local affine CommittedCall of S/C
~~~

The following never mint CommittedCall:

- Existing;
- Stale;
- Indeterminate;
- retained history;
- process restart; or
- finding the same preparation during cold fold.

Store can return Inserted at most once for one exact preparation position. PreparedAccess is
affine, and promotion consumes it. Therefore MFM grants at most one provider-entry authority for
each preparation.

Read may append a bounded new preparation after an earlier attempt returned Unresolved. Unresolved
ends the current top-level call as Waiting; a later resume may create at most one replacement for
that occurrence in that call while the total attempt bound remains. Effect and a Read at its bound
remain Waiting and never replace their preparation.

### 4.3 One bounded caller-driven progression loop

start and resume call one private advance_until_stable loop. It repeatedly:

1. loads and qualifies the current durable run;
2. folds it under the retained Program;
3. enters at most one selected RegisteredState at a time;
4. appends its preparation or conclusion mechanically; and
5. folds the resulting durable state before continuing.

Runtime stops at:

- terminal success or declared failure;
- an unresolved preparation with no immediately permitted action;
- the nonzero per-call State-start bound;
- an indeterminate append;
- semantic conflict or invalid history;
- incompatible assembly;
- capacity; or
- infrastructure failure.

Runtime owns no active-progression, CPU-job, planning-job, or provider-ingress semaphore.
Application/composition may impose one operational bound on simultaneous top-level Runtime calls.
Adapter clients and Store connection pools independently bound their own live IO. None of those
resource policies is a Runtime semantic owner or persisted contract.

The per-call State-start bound remains only as a deterministic yield point for a long run. It is
not concurrency admission, does not allocate a permit, and is the only Runtime execution-limit
setting in this target.

There is no scheduler, timer, background run progression, process-wide writer lease, process-local
per-run lock, pending append table, resolver queue, retained recovery token, cancellation token, or
detached completion task.

### 4.4 Completion and live-IO wait ownership

Every mutating Runtime operation is run to completion by its supported caller. Application and
transport code must not race that future against a timeout, abort it, drop it after a client
disconnect, or treat dropping it as a domain operation. Runtime exposes no cancellation API and
makes no completion or recovery promise for a caller that violates this contract.

Runtime itself owns no timeout policy. Wait control exists only at live-IO boundaries:

- an Adapter owns provider/client deadlines, connection limits, protocol acknowledgement, and the
  exact mapping to accepted evidence, BlockedIntegrity, or Unresolved;
- Store owns database/pool deadlines and the exact mapping to UnavailableBeforeSubmission or
  Indeterminate; and
- Application/composition may bound simultaneous top-level Runtime calls without becoming an
  execution lifecycle owner.

An IO timeout is therefore an Adapter or Store result, not Runtime cancellation. Unresolved permits
only the Program-declared Read replacement rule; it never recreates Effect authority.
UnavailableBeforeSubmission proves no candidate mutation was submitted. Indeterminate means a
Store mutation may have committed and is resolved only by exact retry or a later durable load.

Process termination is unsupported as an operation-completion mechanism. No volatile typed owner
or command is promised to survive it; after restart Runtime uses only durable history.

### 4.5 Blocking work contract

State preparation, Pure evaluation, Access interpretation, integrity projection, value
qualification, and substantial canonical work run through spawn_blocking when they may block the
async executor. Runtime immediately awaits the JoinHandle before using the result.

The blocking closure contains only trusted, bounded-input, pure, terminating synchronous work. It
owns no Store, Adapter, provider client, async runtime handle, or append authority; it never invokes
block_on. Store append and provider ingress remain ordinary async IO after the blocking result is
observed. Runtime adds no CPU semaphore or blocking-job timeout. A callback that does not terminate
violates its trusted registration contract; Runtime does not attempt to interrupt it.

### 4.6 Admission and conclusion races

Admission uses expected position Absent.

- Inserted commits the exact genesis.
- Existing proves the exact same physical admission command is already durable.
- Stale causes Runtime to load genesis. Exact semantic admission is accepted; a different genesis
  under the same RunId is AdmissionConflict.
- Indeterminate retains nothing. The caller repeats exact start inputs or later reads the RunId.

Conclusion uses the exact selected head. After Stale, Runtime may reload while the caller remains
present and classify:

- AlreadyConcludedSame when the exact semantic conclusion record already owns the occurrence;
- NoLongerSelected when a permitted Read replacement superseded the preparation;
- Conflict when a different valid conclusion owns it; or
- InvalidHistory.

If that load fails, Runtime returns the classified error and retains no owner. The next request
starts from a fresh complete load.

### 4.7 RunView and public Runtime surface

RunView is a private-field, non-Serde trusted-core type:

~~~text
RunView {
  run_id,
  head_sequence,
  head_digest,
  state:
      Runnable
    | Waiting
    | Succeeded(RetainedValueView)
    | Failed(RetainedValueView)
}

RetainedValueView {
  contract_ref,
  value_ref,
  canonical
}
~~~

Every RunView names one real qualified durable head. It is a snapshot: a transaction whose
acknowledgement was previously indeterminate may commit after an earlier view was captured.

The core process-facing surface is:

~~~text
start(run_id, Program, typed_c0) -> RunView
resume(run_id) -> RunView
read(run_id) -> RunView
~~~

The operation/domain layer deterministically constructs Program and `C0`; Runtime does not own an
entry-point planner registry. start verifies the Program entry point and exact `C0` contract before
admission. start and resume may execute State/provider work. read does not.

Absence, AdmissionConflict, semantic Conflict, Indeterminate, InvalidHistory,
IncompatibleAssembly, Capacity, Unavailable, and redaction-safe Internal failure remain distinct
errors rather than RunView states.

Trace, access audit, raw frame inspection, and a separate replay response are not core APIs. A
future required trace or audit must be a pure Runtime-owned projection over the same qualified
history, never another reducer or Store port.

## 5. Journal design

### 5.1 Canonical wire rules

All hashed structured data uses the repository canonical JSON implementation with JCS-style
semantics:

- UTF-8 canonical JSON only;
- no floats;
- no duplicate or unknown object fields;
- exact tagged variants;
- no serializer-dependent optional-field omission;
- arrays in their specified canonical order;
- bounded strings, arrays, maps, and nesting;
- canonical bytes must equal strict decode and re-encode; and
- construction inputs include only exact content references and canonical values supplied by the
  Runtime-qualified path.

Secret exclusion is a layered input contract, not a semantic property discoverable by a generic
codec. Domain types and adapters keep secrets outside MfmValue inputs; Runtime value qualification
rejects the repository's mechanically identifiable secret markers before Journal construction;
and Journal never logs canonical values or includes them in diagnostics. An arbitrary secret
encoded as an ordinary string is not claimed to be detectable by Journal.

Protocol counters are non-negative JSON integers bounded by the fixed limits below. Domain values
retain the canonical representation fixed by their MfmValue schemas.

Journal public construction and strict decoding produce opaque valid types. Open public DTO
construction followed by validate choreography is deleted.

### 5.2 Run frame wire

The exact target shape is:

~~~text
RunFrameV1 {
  domain: "mfm.run.frame.v1",
  run_id,
  run_sequence,
  previous_head_digest: null | digest,
  record,
  objects: sorted-unique [
    {
      content_ref,
      canonical: raw canonical no-float JSON value
    }
  ]
}
~~~

Exact record tags and fields are:

~~~text
RunAdmitted {
  kind: "run_admitted",
  entry_point,
  program_ref: content_ref,
  admitted_context: content_ref
}

StatePrepared {
  kind: "state_prepared",
  occurrence,
  intent: content_ref
}

StateConcludedPure {
  kind: "state_concluded_pure",
  occurrence,
  outcome
}

StateConcludedAccess {
  kind: "state_concluded_access",
  occurrence,
  preparation_sequence,
  evidence: content_ref,
  outcome
}

Outcome =
    { kind: "success", value: content_ref }
  | { kind: "failure", value: content_ref }
~~~

Program supplies preparation mode, attempt bound, input/output/failure contracts, execution
binding, and replacement semantics. ExecutableProgram deterministically derives the maximum
complete conclusion-frame size from those bounded contracts and fixed frame overhead. None of that
material is duplicated in the semantic frame.

An accepted integrity block is an Access conclusion with its accepted evidence and the exact
registered failure outcome. It never invokes the ordinary Access interpreter.

### 5.3 Exact frame-local object closure

Let R_i be the set of data-object content references directly named by record i and C_i be the
object map embedded in the same frame. The complete rule is:

~~~text
keys(C_i) = R_i
~~~

Direct data-object references are:

- Program and admitted-context objects for RunAdmitted;
- intent object for StatePrepared;
- outcome object for StateConcludedPure; and
- evidence and outcome objects for StateConcludedAccess.

Stable contract and implementation references are external identities, not closure objects. There
is no implicit transitive object graph.

Every direct content reference has exactly one object in that frame whose nested canonical value
hashes to the reference. Missing, duplicate, conflicting, out-of-order, or unreferenced objects are
invalid. A reference used by a later frame is embedded again in that later frame; cross-frame
deduplication is deliberately not a wire invariant.

The objects array is sorted by complete content reference. Because RunFrameV1 names at most two
direct objects, frame count and canonical-byte limits already bound object processing. There is no
first-reference dictionary, cumulative object-count limit, object slot reservation, or object
accounting in Store. JournalHistory exposes only the qualified record and frame-local objects needed
by Runtime; it has no cumulative object-store contract. Repeated bytes count fully toward
MAX_RUN_BYTES, which is the accepted cost of deleting cross-frame dictionary state.

### 5.4 Recursive run head and exact record equality

The frame head is:

~~~text
run_head_digest = SHA-256(JCS(RunFrameV1))
~~~

The domain field prevents cross-format reuse. The frame does not contain its own digest.

Genesis requires:

- sequence one;
- previous_head_digest null; and
- RunAdmitted.

Every later frame requires the exact previous sequence plus one and the previous frame digest.

The Store-facing positions are exact derived values, not additional wire fields:

~~~text
RunPosition {
  run_sequence,
  head_digest
}
~~~

Runtime race classification compares the closed qualified record fields directly. Value references
already commit to exact canonical objects, so exact record equality defines AlreadyConcludedSame
and identical admission classification without another digest, persisted identity, or public proof
type.

### 5.5 Fixed run limits

The clean format freezes:

| Resource | Exact limit |
| --- | ---: |
| One canonical frame | 33,554,432 bytes |
| Frames per run | 65,536 |
| Canonical frame bytes per run | 536,870,912 bytes |

The per-run ceilings are MAX_RUN_FRAMES and MAX_RUN_BYTES in table order. Frame bytes already
contain every frame-local object value, so objects are charged exactly where their bytes occur and
never require separate accounting.

Changing a limit requires another deliberate persisted-format cutover.

## 6. Store and PostgreSQL design

### 6.1 Mechanical Store surface

The target Store operations are:

~~~text
load_run(run_id, Current | Exact(RunPosition))
  -> Absent | StoredRunBytes

append_run(&RunAppend)
  -> AppendResult

list_run_ids(PageRequest)
  -> BoundedRunIdPage

check_ready()
  -> Result
~~~

StoredRunBytes contains one atomically captured complete prefix and its captured RunPosition. It
does not claim semantic validity. Store returns a complete value or Capacity, never truncation.

PostgreSQL performs Current and Exact loads in one database snapshot. It captures the requested
head row, returns Absent when the requested Exact position is not present, reads every frame from
sequence one through a present position, and mechanically checks contiguity, stored
head/predecessor columns, byte lengths, and cumulative physical accounting. A gap, duplicate, or
internal metadata inconsistency is CorruptPhysicalState. Only then does it return the complete
ordered canonical frame bytes for Journal qualification. A concurrent append is therefore wholly
before or wholly after the captured prefix, never a truncated mix.

Store implementations have a non-persisted read budget no larger than the format ceilings. A valid
run larger than the configured process budget produces load Capacity, not InvalidHistory.

Store does not decode Program, reduce State-or-Match, select facts, reify domain values, or
construct provider authority.

### 6.2 Run append command

RunAppend is one private-field physical command:

~~~text
RunAppend {
  expected_position: Absent | RunPosition,
  frame: EncodedRunFrame,
  reservation_instruction:
      None
    | Open { preparation_sequence, maximum_bytes }
    | Replace { old_preparation_sequence, new_preparation_sequence, maximum_bytes }
    | Consume { preparation_sequence }
}
~~~

Runtime derives the reservation instruction from ExecutableProgram and the selected durable
history. Store checks only sealed frame kind/position correlation and physical arithmetic.

The physical command digest is:

~~~text
command_digest = SHA-256(JCS {
  domain: "mfm.run.append-command.v1",
  run_id,
  expected_position,
  frame_head_digest,
  frame_byte_length,
  reservation_instruction
})
~~~

It is physical metadata, not a public authority or semantic frame field. There is no
AppendRequestId, caller-supplied command id, retry token, append-request table, or receipt table.

### 6.3 Append results and errors

~~~text
AppendResult =
    Inserted(RunPosition)
  | Existing(RunPosition)
  | Stale { actual_position: Absent | RunPosition }

StoreError =
    InvalidPhysicalCommand
  | Capacity
  | CorruptPhysicalState
  | UnavailableBeforeSubmission
  | Indeterminate
~~~

Meanings are normative:

- Inserted proves this exact command committed.
- Existing proves the exact command is already represented by one immutable frame row.
- Stale proves this invocation wrote nothing.
- InvalidPhysicalCommand and Capacity prove this invocation wrote nothing.
- CorruptPhysicalState means retained rows or metadata violate the physical contract and fails
  closed; an append may return it only before candidate mutation or after proven rollback, so this
  invocation wrote nothing.
- UnavailableBeforeSubmission is allowed only when Store proves no transaction was submitted.
- Indeterminate means the candidate mutation may have committed. Every Store timeout, connection
  loss, or driver failure observed after that point is classified only as Indeterminate.

AcknowledgementUnknown is not an AppendResult. Runtime retains no command after Indeterminate.

### 6.4 Idempotency and transaction order

PostgreSQL serializes all appends for one RunId, including absent genesis, with one transaction
advisory key or equivalent protocol. Under that serialization it:

1. computes and validates target sequence, frame position, byte length, and command digest;
2. looks up the matching per-run command-digest index;
3. when that digest exists, compares stored frame bytes, head, expected predecessor, reservation
   instruction, length, and digest, returning Existing only when all material is exact;
4. treats an existing digest with different material, or internally inconsistent retained physical
   metadata, as CorruptPhysicalState;
5. otherwise reads the current run head and compares expected_position;
6. returns Stale without mutation when it differs, including when a different valid successor
   already occupies the candidate sequence;
7. treats an occupied candidate sequence while expected_position is still current as
   CorruptPhysicalState;
8. validates fixed physical limits and reservation arithmetic;
9. inserts the immutable frame row;
10. applies the reservation transition;
11. updates the run head/accounting row; and
12. commits those changes atomically.

The frame row is its own immutable idempotency receipt. Exact retry remains Existing after later
frames advance the run. A different candidate for the same predecessor races head CAS and becomes
Stale.

Memory implements the same ordering and outcomes. At most one caller observes Inserted for one
exact preparation.

### 6.5 Run capacity and reservations

Run head metadata contains:

~~~text
{
  sequence,
  head_digest,
  total_bytes,
  reserved_bytes,
  reserved_frames
}
~~~

Because every frame contains its complete frame-local closure:

~~~text
new_total_bytes = old_total_bytes + frame_byte_length
~~~

Reservation arithmetic under the same head transition is:

~~~text
new_reserved_bytes =
    old_reserved_bytes
    - released_reservation_bytes
    + opened_reservation_bytes

new_reserved_frames =
    old_reserved_frames
    - released_reservation_frames
    + opened_reservation_frames

new_total_bytes + new_reserved_bytes <= MAX_RUN_BYTES
new_sequence + new_reserved_frames <= MAX_RUN_FRAMES
~~~

maximum_bytes means the maximum complete canonical conclusion-frame length, including its object
closure. Every active Access reservation also reserves exactly one conclusion frame. That fixed
frame liability is not a caller-supplied command field.

ReservationKey is mechanically derived from RunId and preparation sequence:

~~~text
SHA-256(JCS {
  domain: "mfm.run.conclusion-reservation.v1",
  run_id,
  preparation_sequence
})
~~~

Open accompanies an Access preparation and creates one active row. Replace atomically removes the
old Read row and creates the new row. Consume removes the exact active row and requires a conclusion
frame no larger than its byte maximum. The byte and frame liabilities move in the same transaction;
underflow, duplicate open, missing old row, or immutable limit failure rejects without mutation.
Exact Existing never reapplies a transition.

### 6.6 PostgreSQL run schema

The logical schema contains:

| Table | Key and retained material |
| --- | --- |
| run_frames | primary key (run_id, run_sequence); canonical bytes, head/predecessor, command digest, reservation instruction, and byte length |
| run_heads | primary key run_id; current position and cumulative bytes plus active byte/frame reservation accounting |
| run_reservations | primary key (run_id, preparation_sequence); active immutable maximum conclusion bytes |

A unique per-run command-digest index supports exact historical Existing lookup. No separate
command or receipt row exists.

### 6.7 EVM nonce persistence

Tenant removal must not merge wallets across chains. The exact public domain is:

~~~text
WalletNonceDomainId = SHA-256(JCS {
  domain: "mfm.evm.wallet-nonce-domain.v1",
  chain_id,
  sender,
  nonce_domain
})
~~~

Nonce heads key by WalletNonceDomainId. Operation rows key by
(WalletNonceDomainId, operation_key). The same domain identity is used by the EVM binding/effect
contract.

Endpoint identity does not split one chain wallet nonce sequence. Credentials and signer secrets
remain outside every persisted key.

### 6.8 Global schema deletion

The target schema has no:

- tenant, Store scope, writer epoch, active identity, rotation, or database incarnation tables;
- global object or per-run object-membership tables;
- append-request or separate receipt tables;
- configuration revision, head, request, or receipt tables;
- fact head, publication, sparse-tree, CT-log, proof, or node tables; or
- any global sequencing table.

check_ready verifies only the one current schema/format baseline and connectivity. Schema version is
a static compatibility check, not a writer authority.

## 7. Execution flows

### 7.1 New run

~~~text
operation/domain layer deterministically constructs Program and typed C0
  -> explicit fresh RunId
  -> Program validation and Runtime association
  -> qualify C0 against Program's exact admitted-context contract
  -> verify Program's exact entry point
  -> Journal encode genesis with complete frame-local Program/C0 objects
  -> Store append expected Absent

Inserted
  -> fold exact durable genesis
  -> continue bounded progression

Existing
  -> load/fold current run

Stale
  -> load genesis
  -> exact admission or AdmissionConflict

Indeterminate
  -> retain nothing
  -> caller repeats exact start or reads RunId
~~~

Core never derives RunId. Every independent execution uses a new caller-supplied value. Retry
reuses the exact RunId and admission inputs.

### 7.2 Hot Pure State

~~~text
Runnable Pure occurrence
  -> exact RegisteredState.start
  -> spawn_blocking evaluation
  -> ProposedStateOutcome<Output, Failure>
  -> Runtime value qualification
  -> Journal encode Pure conclusion
  -> Store append exact head

Inserted | Existing
  -> load/fold durable current run

Stale
  -> load/fold winner

Indeterminate
  -> retain nothing
  -> later resume recomputes if conclusion is absent
~~~

Pure evaluation must be deterministic and perform no ambient IO.

### 7.3 Hot Access State

~~~text
Runnable Access occurrence
  -> exact RegisteredState.start
  -> spawn_blocking preparation
  -> PreparedAccess<S,C>
  -> Journal encode StatePrepared
  -> Store append exact head with Open/Replace reservation

Inserted
  -> consume local PreparedAccess
  -> mint one CommittedCall<S,C>

Existing | Stale | Indeterminate
  -> no CommittedCall
~~~

Only the Inserted branch continues:

~~~text
CommittedCall<S,C>
  -> typed adapter/provider ingress
  -> AccessResolution<S,C>

Outcome(accepted evidence)
  -> spawn_blocking ordinary interpretation
  -> typed outcome

BlockedIntegrity(accepted evidence)
  -> bounded pure registered integrity projection
  -> typed failure

Unresolved
  -> append no conclusion
  -> Read may later replace
  -> Effect parks
~~~

For accepted Outcome or BlockedIntegrity:

~~~text
typed evidence/outcome qualification
  -> Journal encode Access conclusion
  -> Store append exact preparation head with Consume reservation

Inserted | Existing
  -> load/fold durable current run

Stale
  -> load/fold and classify same/replaced/conflict/invalid

Indeterminate
  -> retain nothing
  -> later history reveals conclusion or unresolved preparation
~~~

No path from retained preparation recreates the original provider entry.

### 7.4 Resume and read

resume:

~~~text
Store complete current load
  -> JournalHistory
  -> retained Program decode
  -> Runtime association
  -> Runtime fold
  -> bounded advance_until_stable
~~~

read performs the same load, qualification, and fold but starts no State/provider work.

There is no pending-owner resolution step.

## 8. Facts are deleted

The cutover deletes:

- the mfm-facts crate;
- FactSelectionMode, NoPriorFacts, PriorRunFacts, and the capability Facts associated type;
- prior_fact_selection callbacks;
- FactSelectionRequest, FactSelection, FactValue, FactProposalSet, provenance, completeness, and
  frontier types;
- facts from ProposedStateOutcome;
- source refs used only as fact allow-lists;
- fact fields from Program and every Journal record;
- Store fact query/publication ports;
- publication attachments and coordinate allocation;
- fact-head CAS and rebinding;
- sparse Merkle and CT log structures;
- producer dependency DAGs and portable fact proofs;
- fact limits, schema, SQL, migrations, fixtures, and tests; and
- facts dependencies from every surviving crate.

The target does not retain empty placeholder fields, NoPriorFacts markers, reserved tables, or a
generic extension hook.

Facts may return only through a separate future RFC with a reachable consumer and a new deliberate
persisted contract.

## 9. Identity and restore contract

The cutover deletes StoreScopeId, StoreEpoch, TenantScopeId, StructuredStoreIdentity, Store opening
brands, active-identity rows, rotation methods, identity retries, and every related field from:

- Program and Journal;
- hashes and canonical fixtures;
- Store APIs and backend rows;
- Runtime values and errors;
- App/transports;
- EVM nonce authority.

No replacement identifier may provide the same function under another name.

One database is one authority and RunId is global within it. Deployment credentials and network
exposure are external security boundaries.

An in-place database restore requires all old peers to stop before the restored database is made
writable. Restore with a surviving writer is unsupported and has no core safety claim.

## 10. Package and API cutover

### 10.1 mfm-values and derive

- add the reserved Never value contract;
- retain strict canonical, schema, no-float, bounded-input, and content-reference rules;
- reject mechanically identifiable secret markers during persistable value qualification without
  claiming semantic secret discovery;
- delete MfmConfig, ValidatedConfig, ProvenConfiguration, and the MfmConfig derive;
- remove facts-related value/schema exports; and
- update derives and compile-fail coverage.

### 10.2 mfm-program

- retain the immutable State-or-Match document;
- make every failure contract non-optional;
- persist complete Access association;
- remove the Program configuration contract and migrate durable policy into Program or `C0`;
- remove FailureValue and State::integrity_failure;
- remove Program execution catalogs, reifiers, TypeId maps, brands, and erased callbacks;
- remove fact behavior; and
- expose no supported erase/downcast workflow.

### 10.3 mfm-runtime

- own RuntimeAssembly, Program association, the sole reducer, typed State drivers, bounded
  progression, and RunView;
- register the exact Access integrity projection;
- keep only one private object-safe State driver/hot-value handoff;
- run blocking deterministic callbacks outside the async executor and immediately await them;
- retain no pending append or conclusion owner;
- own no cancellation API, timeout policy, detached completion task, or Runtime semaphore;
- perform no background progression; and
- expose no public State-by-State lifecycle algebra.

### 10.4 mfm-journal

- own only the exact RunFrameV1 codec;
- own strict construction/decoding, recursive run heads, exact frame-local object closure, and
  fixed frame/run limits;
- expose only opaque EncodedRunFrame and JournalHistory proof types; and
- remove open DTO validation choreography, configuration and portable codecs, record digests,
  first-reference state, identity fields, request ids, facts, and publication rebinding.

### 10.5 mfm-store

- expose only the mechanical Store trait and physical command/result types;
- implement identical Memory/PostgreSQL append semantics;
- store frame command digests on their immutable rows;
- own database/pool wait control and classify failures as UnavailableBeforeSubmission or
  Indeterminate at the exact submission boundary;
- own no Program, reducer, State, capability, fact, configuration, portability, or replay
  semantics; and
- delete semantic Store facades, selection owners, brands, duplicated backend DTOs, and split ports.

### 10.6 Adapters and live IO

- own provider/client deadlines, connection limits, and protocol acknowledgement;
- map every completed wait to accepted evidence, BlockedIntegrity, or Unresolved;
- permit an internal retry only when the Adapter proves an Effect request was not submitted;
- return Unresolved after any ambiguous Effect submission or response wait; and
- expose no cancellation token or generic retry authority to Runtime.

### 10.7 App and transports

- establish an explicit fresh RunId before submission and make that same value reproducible to the
  caller for an indeterminate retry;
- call only Runtime start/resume/read APIs;
- drive every started mutating Runtime future to completion without racing a timeout, aborting it,
  or tying it to client-disconnect cancellation;
- optionally apply one coarse operational bound to simultaneous top-level Runtime calls;
- render RunView and reviewed redaction-safe errors;
- retain no RunSession, SuspendedRun, pending owner, or frame-derived status;
- remove fixed-tenant facade state;
- remove current trace/audit/replay endpoints unless separately reintroduced under a reviewed
  transport contract; and
- update CLI/REST documentation and fixtures in the same cutover.

### 10.8 Replay

Delete mfm-replay, its workspace membership, dependencies, DTOs, and reducer. Runtime read is the
one supported Store-backed semantic inspection path; no portable replacement remains.

## 11. Complete deletion checklist

The cutover is incomplete while any of these supported concepts remains:

~~~text
ProgramCatalog
ProgramCatalogBuilder
ProgramCatalogInner
ValueAssociation
CapabilityAssociation
ReifyValue
QualifiedValue
QualifiedTypedValue::erase
public try_downcast
catalog brand

DynamicOutcome
DynamicPreparationFailure
DynamicStateRegistration
DynamicPrepared
DynamicCommit
DynamicCall
DynamicResolution
RunSession
SuspendedRun
ParkedRun
SpawnStep
ResumeStep
RuntimeStep
PendingAppend
PendingWork
PendingKey
acknowledgement lease
completion/finalization cell
pending-owner limit or permit
resolver token
Runtime cancellation API/token/state
Runtime active-session/CPU/planning/ingress semaphores
max_active_sessions
max_cpu_jobs
max_planning_jobs
max_ingress_jobs
active_sessions
cpu_jobs
planning_jobs
ingress_jobs
Runtime semaphore acquire helpers and OwnedSemaphorePermit fields
Runtime timeout policy
Runtime detached completion task/finalizer

StoreBrand
StructuredStore
OpenedStructuredStore
StoreParts
QualifiedHistoryPort
HistoryReader
StoreAuditPort
QualifiedRun
RunReducer
RunSelection
RunAction
SelectedRun
PreparedAdmission
PreparedConclusion
SelectedConclusion
FactContinuation
SemanticStore
replay_terminality

StructuredStoreBackend
BackendAppendCommand
BackendAppendOutcome
AppendDisposition
RawRunPrefix
RawFrameBytes
RawFactPublication
RawFactSnapshot
append request row
configuration request row
separate receipt row
AppendRequestId
IdempotencyConflict
AcknowledgementUnknown result

FailureValue
optional State failure contract

mfm-facts
FactSelection
FactProposalSet
FactHead
fact publication
fact proof
fact SQL

MfmConfig
ValidatedConfig
ProvenConfiguration
ConfigurationKey
ConfigurationPosition
ConfigurationRef
ConfigurationRevision
ConfigurationAppend
ConfigurationHeadProjection
ConfigurationStore
ResolvedConfiguration
ResolvedConfigurationHead
ConfigurationWriteSession
PreparedConfigurationAppend
SuspendedConfigurationAppend
ConfigurationAppendCommand
ConfigurationCommitOutcome
ConfigurationAppendDisposition
BackendConfigurationOutcome
RawConfigurationRevision
configuration publication/load API
configuration revision/head/request/receipt SQL
Program configuration contract
configuration envelope middleman
duplicated configuration schema ref

PortableRunBundle
PortableRun
ExportedRun
export_run
portable export/inspection API
portable decoder or semantic ingress

record_digest
separate semantic record hash/proof type

first-reference object closure/dictionary
MAX_RUN_OBJECTS
object_count
reserved_objects
object-slot reservation/accounting

StoreScopeId
StoreEpoch
TenantScopeId
StructuredStoreIdentity
active identity
identity rotation
database incarnation

global object table
per-run object membership

mfm-replay
separate replay reducer
App suspended map
App frame-derived status
~~~

Generic words such as state, scope, tenant, fact, trace, or audit may remain in unrelated prose or
domain concepts. No supported core equivalent of a deleted responsibility may survive under a new
name.

Production LOC, exported type count, dependency edges, and files required to add a State must all
decrease.

## 12. Security and failure semantics

- Secrets never enter Program, `C0`, Journal frames, RunView, outputs, Store physical metadata, SQL
  keys, logs, or error details. Domain types and adapters keep secret material outside persistable
  MfmValue values; Runtime qualification rejects known secret markers before Journal construction.
  Journal makes no dishonest claim to discover an arbitrary secret encoded as an ordinary string.
- Hashed structured values contain no floats.
- Journal and value canonicalization use fixed domain-separated SHA-256 recurrences and strict
  canonical bytes.
- Content hashes prove byte correlation and detect corruption; they are not external signatures
  and do not claim Byzantine database authorship.
- Runtime State logic performs no ambient IO. Provider, network, filesystem, signer, and storage IO
  cross explicit adapters or Store.
- Only a freshly observed preparation Inserted grants CommittedCall.
- Existing, Stale, Indeterminate, history, and restart grant no provider authority.
- No success is exposed before a durable conclusion is observed.
- Every Store failure after the candidate mutation may have committed is Indeterminate.
- Read provider access is declared non-mutating and replacement after Unresolved is bounded. Effect
  has one possible provider entry and parks after Unresolved or an absent indeterminate conclusion.
- Supported callers drive mutating Runtime futures to completion; Runtime owns no cancellation or
  timeout mechanism.
- spawn_blocking owns only synchronous pure work, is immediately awaited, and contains no IO or
  persistence authority.
- Store append is atomic per frame and exact-head linearized.
- Frame rows are immutable and exact retry never reapplies reservation/accounting changes.
- Process termination has no operation-completion guarantee; restart trusts only durable history.
- Restore with a surviving old writer is unsupported.
- Public errors are reviewed and redaction-safe.

## 13. Tests and verification contract

### 13.1 Program, value, and compile-time proofs

- Program rejects missing/extra/incompatible input, output, failure, Access, and Match contracts.
- Never has no constructible or decodable value.
- Pure registration with Never succeeds.
- Access registration with Never fails.
- Access integrity projection is exact for S/C and may preserve allowed input correlation.
- persistable value qualification rejects every repository-defined secret marker, and no test
  claims arbitrary-string secret detection.
- PreparedAccess and CommittedCall constructors remain private and owners remain affine.
- Private State-driver results cannot contain PreparedAccess, CommittedCall, evidence, or a typed
  conclusion.
- No public erase/downcast or catalog branding path compiles.

### 13.2 Journal goldens

Golden fixtures freeze:

- every RunFrameV1 record and outcome variant;
- null genesis predecessor and non-genesis predecessor representation;
- canonical field names, tagged variants, number representation, and object order;
- nested raw canonical object representation;
- repeated cross-frame references with complete frame-local objects;
- the run-head recurrence; and
- exact frame/run bounds and independent bound-plus-one behavior.

Negative fixtures cover:

- non-canonical bytes, floats, duplicate/unknown fields, and invalid UTF-8;
- wrong sequence/predecessor/head/domain;
- missing, duplicate, conflicting, out-of-order, and extra frame-local closure objects;
- bad object content digest or contract/schema binding;
- invalid record tag/shape and genesis kind;
- failure under Never.

### 13.3 Runtime semantic tests

- hot and cold fold produce the same RunView;
- Store-backed read invokes no State, adapter, or provider callback;
- Match reduction uses the one schema-derived projection;
- exact State key dispatch rejects partial or implementation-only matches;
- only preparation Inserted mints CommittedCall;
- Existing, Stale, Indeterminate, and cold history mint none;
- concurrent identical preparations cause at most one provider entry;
- Adapter Unresolved never permits Effect re-entry;
- an indeterminate Effect conclusion later found absent leaves Effect parked;
- Read alone may replace, at most once per occurrence per resume, and never exceeds its total
  attempt bound;
- old Read conclusion versus replacement commits at most one exact-head successor;
- Pure work safely recomputes after an indeterminate conclusion later found absent;
- no RunView success appears before durable conclusion;
- an earlier RunView remains a valid captured snapshot if a formerly indeterminate transaction
  commits later; and
- the per-call State-start bound returns Runnable and a later resume continues from that exact head.

### 13.4 Store conformance

The same suite runs against Memory and PostgreSQL:

- absent-genesis race;
- exact same-command retry before and after later head advancement;
- different-command same-head race;
- command digest/material corruption;
- command and reservation digest golden vectors;
- historical Existing;
- stale performs no mutation;
- Inserted writes frame, head, reservation, and accounting atomically;
- rollback leaves no partial frame/head/reservation state;
- exact Existing never reapplies reservation arithmetic;
- Open, Replace, Consume, late old conclusion, and byte/frame bound-plus-one capacity behavior;
- frame-local object bytes are charged exactly once as part of their containing frame;
- complete Current and Exact loads;
- load budget returns Capacity without truncation;
- stable bounded RunId pagination;
- every pre-submission fault classified UnavailableBeforeSubmission performs no write;
- every injected post-submission acknowledgement loss is Indeterminate;
- later load distinguishes committed from rolled-back indeterminate cases; and
- no AppendRequestId, request row, receipt row, object table, membership table, configuration
  table, identity, or fact table exists.

### 13.5 Completion, blocking, and live-IO tests

Boundary-focused tests and repository checks cover:

- every started mutating Runtime future is directly driven to completion by supported App and
  transport paths;
- Runtime exposes no cancellation token, timeout wrapper, detached completion task, or semaphore;
- spawn_blocking closures contain only synchronous deterministic work and are awaited before
  provider or Store IO;
- Adapter deadline/transport ambiguity returns Unresolved through the capability contract;
- Store pre-submission timeout returns UnavailableBeforeSubmission and possible post-submission
  loss returns Indeterminate;
- post-commit/pre-ack preparation Indeterminate causes zero provider entries on exact Existing;
- rolled-back preparation Indeterminate permits one later Inserted and exactly one provider entry;
- Effect Unresolved leaves the durable preparation and no re-entry;
- accepted Effect evidence followed by a rolled-back indeterminate conclusion leaves Effect parked;
- conclusion post-commit acknowledgement loss is later observed durably;
- conclusion rollback leaves Effect waiting and Read replacement-eligible;
- Pure conclusion commit/rollback ambiguity is safely folded/recomputed; and
- an optional composition-level top-call bound does not enter Runtime proof or persisted state.

### 13.6 EVM persistence tests

- WalletNonceDomainId differs across chain, sender, and nonce-domain changes;
- endpoint changes do not split the same chain wallet nonce sequence;
- nonce operations remain idempotent and secret-free; and
- no tenant field survives in EVM binding or SQL.

### 13.7 Absence and complexity evidence

Record:

- workspace dependency graph before and after;
- exported production types before and after;
- net production LOC before and after;
- files/registrations required to add one Pure and one Access State;
- Store production LOC removed by semantic/fact/identity/request/object deletion;
- one canonical Journal construction/decoding path;
- one Runtime reducer;
- one Runtime assembly registry;
- zero Runtime semaphore/cancellation/detached-completion paths;
- one Store command per real operation; and
- repository searches proving every concept in the deletion checklist is absent from supported
  core APIs, wire, hashes, schema, App, and transports.

Verification follows docs/build-and-verification.md. Run narrow owner-specific checks first,
expand only for affected boundaries, and run the composed CI gate once after narrower failures are
resolved.

## 14. Ordered implementation commits

### 14.1 freeze exact executable program contracts

In one coherent Program/values/derive/domain cutover:

- make State failure contracts mandatory MfmValue contracts;
- add framework Never;
- move accepted integrity failure to exact Access registration;
- persist complete Access associations;
- update every current domain implementation and Program fixture; and
- update authoritative design/architecture and crate documentation for that current intermediate
  contract.

The existing Store remains the sole semantic owner in this commit. No second Runtime reducer is
introduced yet.

### 14.2 cut over minimal journal and persistence contracts

In one clean-slate Program/values/derive/domains/Journal/Store/Memory/PostgreSQL/Runtime/App/EVM
cutover:

- install RunFrameV1, exact recursive heads, frame-local closure, and goldens;
- embed every directly named object in its frame and charge only complete frame bytes;
- make EvmConfig and PortfolioConfig process-local authoring inputs, have their planners return only
  the exact Program and typed `C0`, keep live bindings in assembly/adapters, and delete MfmConfig
  plus the complete configuration subsystem, Program configuration contract, and admission field;
- delete portable export, bundle codecs, offline semantic ingress, and mfm-replay;
- store physical command digests on frame rows;
- install the final reservation/accounting contract;
- make RunId global and EVM nonce identity chain-aware;
- delete tenant, scope, epoch, identities, rotation, facts, request ids/tables, receipt tables,
  global object/membership tables, configuration tables, first-reference/object accounting, and
  record digests;
- rewrite baseline schemas and fixtures with no legacy reader; and
- keep Store as the one explicitly documented temporary semantic reducer over the final persisted
  contract.

Every producer and consumer moves together. No old/new codec, schema, or compatibility adapter
coexists.

At this commit boundary Store still owns exactly one reducer and the existing caller-driven
lifecycle surface, but that reducer consumes only the final JournalHistory and emits only the final
RunAppend commands. Runtime and App are adapted to that one current surface. The intermediate has
no configuration, portable bundle, replay crate, facts, identity brands, request owners, old wire
types, or alternate persistence API. Its tests and authoritative docs describe that state
explicitly, so the next commit deletes ownership rather than choosing between two implementations.

### 14.3 move execution and read into Runtime

In one inseparable Program/Runtime/Store/App cutover:

- add the final RuntimeAssembly, Program association, sole reducer, direct-new gate,
  advance_until_stable, run-to-completion caller contract, RunView, and Store-backed read;
- delete Runtime cancellation, timeout, detached-completion, and semaphore machinery;
- reduce Store to its final mechanical trait;
- migrate App/transports to explicit RunId and Runtime APIs;
- delete Program execution catalogs/reifiers, Store semantics, public lifecycle/session/suspension
  types, and App suspended/status logic; and
- update every test, README, docs/design.md, docs/architecture.md, transport contract, and absence
  check in the same commit.

No commit leaves zero semantic owners or two semantic owners. No compatibility path is staged
between commits.

Commit subjects are lower case exactly as shown by the section titles.

## 15. Rejected alternatives

### Split Store files without changing ownership

Rejected. Navigation improves while the same reducers, validators, DTOs, registries, and future
change sites remain.

### Keep Store reduction

Rejected. Runtime would still execute a persistence layer's semantic decision and inspection would
still require Store semantics.

### Remove every erasure

Rejected. A retained heterogeneous Program selects different Rust State families at runtime. One
private exact-key registry dispatch is unavoidable.

### Add Runtime cancellation or pending-result custody

Rejected. Supported callers drive mutating Runtime futures to completion. Runtime cancellation
tokens, timeout branches, owner tables, retry leases, and cancellation-specific states would model
an unsupported operation and duplicate live-IO wait ownership.

### Put provider interpretation or append in a detached finalizer

Rejected. A detached finalizer creates another lifecycle owner without solving process termination.
Synchronous pure work is awaited through spawn_blocking; provider and Store work remains awaited
live IO under Adapter and Store policy.

### Wrap State execution and Store append inside spawn_blocking

Rejected. spawn_blocking protects the async executor from synchronous CPU work; it is not a wait or
durability owner. Store append is async live IO and must not be driven with block_on or a blocking
database client inside the closure. The supported caller awaits the ordinary Runtime future through
both phases.

### Keep random append request ids

Rejected. The canonical physical command digest plus the immutable target frame row already
provides exact idempotency. Separate request identities add conflict states and tables without
proving another property.

### Keep a separate command/receipt table

Rejected. Every successful command creates exactly one immutable frame. That row can retain the
command digest, physical instruction, and resulting position.

### Keep a first-reference object dictionary

Rejected. Cross-frame deduplication makes frame construction and qualification depend on a
cumulative object map and adds object-count/reservation arithmetic. RunFrameV1 names at most two
direct objects. Re-embedding every directly named object makes closure local and lets the existing
canonical-byte limit account for the complete cost.

### Keep a separate semantic record digest

Rejected. The closed qualified record fields and their content references already define exact
semantic equality. Another digest adds a golden contract and identity without avoiding any
comparison or persistence requirement.

### Store objects globally

Rejected. Exact frame-local objects make retained frames self-contained and give one byte-accounting
model. Physical deduplication is not a semantic requirement.

### Retain facts as empty extension points

Rejected. Empty markers, fields, crates, tables, and capability modes preserve concepts and future
change sites without a consumer. A future fact system must be designed from its real contract.

### Keep independently published configuration

Rejected. Reproducible secret-free policy belongs in Program or typed `C0`; deployment clients and
credentials belong in assembly/adapters. A configuration stream adds a second value lifecycle,
latest lookup, CAS, revisions, SQL, retry semantics, and admission coupling without a current
consumer that needs independent publication.

### Keep portable export and offline inspection

Rejected. No current core consumer needs a second complete-history ingress or export format.
Runtime read over Store is the one semantic inspection path. A future portability feature must
start from a concrete external consumer and cannot reserve codecs or Runtime entry points now.

### Keep tenant, scope, epoch, or a renamed incarnation

Rejected. They do not belong to the selected one-database core authority. Restore fencing is an
operator/deployment concern outside this contract.

### Keep mfm-replay or public trace/audit lifecycle DTOs

Rejected. Runtime owns the only semantic fold. Future projections must reuse it rather than retain
another reducer or Store port.

## 16. Acceptance criteria

Implementation is accepted only when:

1. Program persists one exact input, output, mandatory failure, and complete Access association for
   every State.
2. Pure Never registration works, Access Never registration fails, and FailureValue is absent.
3. RuntimeAssembly is the only concrete value/State/capability registry.
4. Exactly one private object-safe State driver boundary and its HotValue handoff exist at
   RegisteredState.start; typed execution owners never cross it.
5. Runtime owns the only semantic reducer used by execution and read.
6. Store exposes only the mechanical operations in this RFC and depends on no Program, State,
   capability, domain, fact, configuration, portability, or Runtime-selection type.
7. RunFrameV1, frame-local closure, run-head recurrence, and every fixed frame/run limit match
   golden fixtures.
8. Every frame embeds exactly the canonical objects directly named by its record; first-reference
   state, a separate record digest, object-count limits, and object reservations are absent.
9. Frame rows retain their physical command digest and provide exact historical Existing without
   request or receipt tables.
10. PostgreSQL and Memory have identical absent-head, exact retry, stale, indeterminate,
    reservation, and capacity behavior.
11. Only a locally observed preparation Inserted mints CommittedCall.
12. Existing, Stale, Indeterminate, history, and restart never mint provider authority.
13. Runtime retains no pending append/conclusion owner, lease, permit, completion cell, resolver,
    cancellation/timeout API, detached completion task, or Runtime semaphore.
14. spawn_blocking work is synchronously pure, contains no block_on, Adapter, Store, or persistence
    authority, and is awaited before dependent IO.
15. Adapter and Store own live-IO wait classification; Read replacement is bounded after
    Unresolved, while Effect has one possible provider entry and parks after Unresolved or an absent
    indeterminate conclusion.
16. start, resume, and read return the same Runtime-derived RunView contract.
17. App/transports own no Runtime lifecycle or frame interpretation, drive every started mutating
    Runtime future to completion, and keep any top-call concurrency bound outside Runtime semantics.
18. mfm-replay, portable bundles, export, offline inspection, and every replacement semantic
    ingress are deleted.
19. MfmConfig, configuration publication/load, Program configuration association, configuration
    wire, and configuration SQL are deleted; durable policy is in Program or typed `C0`.
20. mfm-facts and every fact selection/publication/proof/storage concept are deleted.
21. tenant, scope, epoch, identity rotation, and database incarnation are absent from APIs, wire,
    hashes, SQL, App, transports, and EVM.
22. AppendRequestId, request/receipt tables, global objects/membership, and global configuration
    head/sequence are absent.
23. EVM nonce/effect identity includes chain, sender, and nonce domain and contains no tenant or
    secret.
24. Secret-bearing types remain outside persistable values, known secret markers fail before
    Journal construction, and Journal does not claim arbitrary-string secret detection.
25. docs/design.md, docs/architecture.md, transport documentation, crate READMEs, schemas, and
    fixtures describe only this current design.
26. Production LOC, exported types, dependency edges, and files required to add a State decrease.
27. Scope-selected checks and final composed CI pass under docs/build-and-verification.md.
