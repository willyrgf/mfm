# RFC: persist the current run state and preserve causal errors

Status: ready for implementation handoff after the 2026-09-12 baseline, contract, and interface
review. The user's behavioral answers and the resulting technical choices are settled. The handoff
sketches are in sections 6.5, 7.4, 8.5-8.6, 9.3-9.4, and 10.1; the ordered plan is section 12.
The implementation is not complete; net simplification and
shipping frame sizes require the core evidence gate before broad owner migration. Readiness of
the plan is not a claim that the design has already passed that gate.

Use `7f71beef` as the implementation baseline, applying this RFC revision on top in an isolated
checkout. Treat the subsequent implementation, including the worktree reviewed at `6573ef68`, as a
discarded approach for planning. Preserve that worktree. Do not reset/clean it, wholesale restore
its implementation, or begin another repository-wide salvage project.

[Design](docs/design.md), [architecture](docs/architecture.md), [code quality](docs/code-quality.md),
and [AGENTS.md](AGENTS.md) govern the repository. This RFC proposes changes to their current
persistence, validation, and failure contracts. Update those documents, their producers, consumers,
and tests together in the implementation cutover. Read their baseline versions at `7f71beef`;
the dirty worktree's documentation is not the restart specification.

The [adapter error audit](docs/adapter-error-audit.md) is the first-loss inventory. The complete
objective still includes closing those causal gaps. Broad owner migrations follow the integrated
persistence/recovery replacement; they are not prerequisites for proving it.

## 1. Objective and target

Persist the Runtime's actual serializable continuation state. Decode that same type on load,
validate the current state, and continue from it. Delete event-to-state reconstruction and all
historical semantic transition checking. There is no independent snapshot model, native-value
cache, or reference-only persistence representation to synchronize with the execution state.

```text
execution:
    current RunState -> permitted typed operation -> next RunState + operation facts
        -> encode one complete commit -> atomic Store append
        -> known insertion -> adopt next RunState

resume:
    load committed bytes -> checked frame decode -> decode RunState
        -> validate current state against admission -> dispatch its permitted next operation
```

Runtime owns continuation and execution rules. Values owns the canonical objects carried in that
state. Journal owns the exact, opaque frame envelope and its codec. Store owns physical loading,
append-only storage, and atomic exact-head insertion. Each invariant has one validation owner;
repeated checks at neighboring layers are not independent guarantees.

Persist original State-domain failures and adapter operational failures before recovery policy
runs. Keep system failures in the invocation's reviewed causal report, outside the run history.
A classifier or public message never replaces the original error. If recording fails, report the
available original error and the recording failure without claiming the outcome became durable.

Complete state still contains continuation data: cursor, current context, active checkpoint
inputs, recovery usage, and unresolved Effect authority. Deleting the old snapshot/fold machinery
does not delete those facts. There is one persisted current state, not a second snapshot facility.

## 2. Confirmed decisions

### 2.1 Confirmed

| Decision | Consequence |
| --- | --- |
| Remove unnecessary qualified/cold/native-cache representations | One non-generic serializable RunState uses one canonical Values object. No RunState<QualifiedValue> versus RunState<ContentRef> pair. |
| Reuse the execution state on restoration | The exact same Rust continuation type is encoded and decoded. Executable State objects, adapters, signers, and secrets remain outside it. |
| Remove duplicated responsibility | Journal has no lifecycle types or semantic validator; Runtime has no second wire model; Store does not interpret Runtime payloads. |
| Remove the fold and historical transition scan | No load path calculates historical successors, reconstructs counters, or compares past transitions. Current-state validation remains. |
| Persist State-domain and adapter operational error originals | Store their reviewed causes in the operation result, before classification/handler/mapping. |
| Report system failures outside history | No durable internal-fault variant, internal-error Program identity, callback-error schema registry, or fault-specific stop lifecycle. |
| Remove adapter_context and IncidentSummary | Preserve actual input, intent/command, original error, and direct Classification in their owners. Remove context-only and summary-only callbacks/types. |
| Remove future-capacity admission | Enforce actual object/frame/run ceilings and semantic recovery limits; delete lifecycle size predictions and capacity-only quotas. |
| Keep acknowledged history append-only | No rollback, old-format reader, compatibility path, or rewriting previous frames. |
| Keep recovery action authorization durable | Commit a normal recovery decision and resulting state before executing the action. A recommendation alone is not authority. |
| Keep error disclosure reviewed and bounded | No raw client dumps or secrets; omissions are explicit. Source preservation does not require persisting system failures. |

### 2.2 Explicit behavioral answers

| ID | User decision | Required behavior |
| --- | --- | --- |
| D1 | Retry from the last committed state after an internal failure | End the failed invocation without append. Explicit resume retries the unfinished acknowledged phase; no hidden permanent-stop marker or automatic retry loop. |
| D2 | Commit settlement first; resume interpretation only | Persist accepted Effect evidence and its awaiting-interpretation continuation before calling the interpreter. An interpreter failure leaves this phase available for explicit resume without adapter reentry. |
| D3 | Load admission and latest state only | One consistent Store read returns admission, latest committed frame, and head metadata. A requested exact-candidate sequence probe is allowed for append reconciliation; ordinary loading performs no full-prefix scan. |

### 2.3 Error routing depends on meaning, not the crate name

A State's declared business failure and an adapter's declared operational failure belong in the
run history. A State bug, adapter invariant violation, decoder rejection, handler/map/task failure,
Store failure, startup/configuration failure, or response-delivery failure is a system failure and
belongs in the invocation report.

The same lower-level database or signer source can occur in either route. A signer rejection nested
inside a declared transaction operational error remains part of that persisted operational error.
A database failure while appending that error is a separate system/recording failure. Do not discard
a cause based on its source library, or turn an internal adapter bug into a recoverable incident.

## 3. Why the previous RFC caused expansion

At the baseline, FoldState stores cursor, checkpoint inputs, recovery usage, and Effect authority.
It consumes recorded results; it does not rerun business callbacks during loading. Consequently,
replacing it with a function that checks every historical successor preserves much of its work.
That was not the deletion previously implied.

The old execution path also classified and mapped errors before appending their outcomes. Fixing
custody around that fused path required preserving more in-memory partial progress. Splitting the
outcome commit first removes much of that need.

The measurements below remain a dated review of the engineer's attempt. They are not a current
working-tree count, a production-only count, or evidence that every added line was unnecessary.

### 3.1 Evidence from the second implementation attempt

The following measurements describe `7f71beef` versus HEAD `6573ef68` and the worktree inspected
on 2026-09-12, before this RFC edit. They are physical changed lines, including tests/docs where
present; they are **not** production-code LOC or a count of unnecessary code.

| Comparison | Files | Added | Deleted | Net |
| --- | ---: | ---: | ---: | ---: |
| Baseline to committed HEAD, 30 commits | 176 | 20,313 | 3,553 | +16,760 |
| HEAD to tracked worktree | 60 | 4,772 | 1,499 | +3,273 |
| Baseline to tracked worktree | 189 | 24,488 | 4,455 | +20,033 |
| Untracked files, additional to the tracked diff | 13 | 2,439 | 0 | +2,439 |

The two diffs have overlapping files/edits: add their net deltas, not their insertion/deletion
totals. Directory totals for the combined tracked diff locate the growth:

| Area, including its tests/docs | Added | Deleted | Net |
| --- | ---: | ---: | ---: |
| EVM domain | 3,953 | 488 | +3,465 |
| Runtime | 3,740 | 714 | +3,026 |
| PostgreSQL adapter | 3,875 | 1,135 | +2,740 |
| Values and program-derive | 2,869 | 223 | +2,646 |
| Application | 1,899 | 139 | +1,760 |

Reproduce tracked counts with `git diff --numstat 7f71beef HEAD`, `git diff --numstat`, and
`git diff --numstat 7f71beef`; enumerate additional files with
`git ls-files --others --exclude-standard`. The numbers above are a dated review, not a live gate.

Concrete source evidence explains why many locally plausible commits did not reach the objective:

- Baseline and reviewed `crates/kernel/runtime/src/engine/fold.rs` are both 648 lines. The fold,
  `adapter_context`, and future-capacity declarations remain. Retained live-progression rules
  must exist once in Runtime; this revision deletes historical checking itself. Deleting the
  fold file still cannot be booked as 648 lines of net simplification without its replacement cost.
- Reviewed `crates/kernel/runtime/src/assembly.rs::ColdQualifier` still returns
  `Result<QualifiedValue, ()>`. `qualify_typed` still uses `serde_json::from_slice::<T>` and unit
  mappings. New `DecodePersisted` implementations are not connected to this consuming boundary.
- `cc84dc79` and `30a025f7` add native construction and its derive; subsequent commits migrate
  nested value families. `6573ef68` adds a 405-line `values/src/error_schema.rs`. Removing the
  requirement for decoder-error Program identities did not remove wire, field-error, schema,
  conversion, and test costs for errors nested inside actual persisted results.
- The worktree's `recording.rs` repeats native/qualified custody in `AdapterReturnedValue` and
  `StateReturnedValue`. `recovery_evaluation.rs` accumulates classification/request/decision
  options and every intermediate map output. Application then renders these additional surfaces.
  This repairs custody around the old fused execution path before replacing that path.
- PostgreSQL acquisition now keeps failures from the acquired connection; signer/custody wrappers
  retain causes that were previously discarded. These are real improvements. Their presence does
  not establish that the complete redesign is integrated, or justify retaining the whole attempt.

### 3.2 Requirements that multiplied implementation work

The section references in this table identify the RFC at `7f71beef`; `36a1e233` is the subsequent
revision that still retained historical checking and persisted internal faults.

| RFC requirement | Why an engineer following it expanded the code | Correction in this revision |
| --- | --- | --- |
| Sections 11 and 13 put broad source-owner capture and checked decoding before persistence deletion | Each provider/database/signer/value family brings its callers, schemas, conversions, fixtures, and transport projections. Many coherent local changes can finish while the old Runtime remains intact. | Complete the core cutover using existing baseline operational errors, then close the remaining first-loss boundaries one owner at a time. |
| Section 9 demands exact native constructor causes across heterogeneous boundaries | Removing recursive decoder identities still leaves raw wire types, field companions, native construction, erasure, nested schemas where errors are persisted, and consuming tests. “Native” did not make that free. | Internal decoder failures are invocation errors without persisted identities/schemas. Migrate only a constructor actually reached by the current slice; preserve its native cause through the real consumer. |
| Sections 7 and 10 require durable internal/callback/recovery faults | Persisting a system failure turns every internal source into a schema problem, changes callback associations, creates fault phases, and forces cold/client support for each. | Internal errors have source-preserving reports only. No internal-result persistence ABI or durable fault phase. |
| Section 6 asks for complete state while sections 8.2-8.3 still verify historical transitions | A new snapshot model and its codec are added while most fold rules survive as predecessor-to-successor validation. Removing the fold file need not remove the work. | Remove semantic history validation itself. Validate current state only; apply execution rules when creating its successor. |
| Sections 8.1 and 9 allow live candidate encode/checked-decode as the ownership route | Native return, canonical object, re-decoded native object, qualified wrapper, and a wire reference can all become separate representations with separate failures. | Canonical objects are the engine's representation. Encode native outputs once, seal the commit, and adopt that candidate after known insertion. Decode native inputs at use; no post-encode round trip. |
| Section 10 protects all completed work before the fused append | General success/result custody, callback-stage options, intermediate mapping outputs, and report variants spread across Runtime and App before the lifecycle is simplified. | Commit original domain/operational failure before policy. Keep one uncommitted original error and its recording result; no intermediate-progress timeline or universal native-success stash. |
| The phrase “same state” left declaration ownership unresolved | `36a1e233` made Journal own generic state declarations, with qualified live values and ref-only wire values. It removed duplicate field declarations but retained projection, resolution, caches, and ownership conversions. | Runtime owns one non-generic state and payload. Journal receives an opaque canonical payload. Inline Values objects remove reference swizzling and object tables. |
| Section 13 defers central deletions; section 15 mixes real deletion and avoided additions | Preparation accumulates indefinitely; preventing archived registries is presented as baseline simplification. | Name actual deletions at every cutover. Report replacement LOC separately; forbidden hypothetical machinery earns no deletion credit. |

The repeated outcome follows these dependency and contract choices, not merely a preference for
verbose code. The engineer should have exposed the cumulative cost earlier, but the RFC authorized
it. This revision removes obligations as well as changing the order. It does not claim the former
contract can be implemented more cheaply by renaming its machinery.

The explicit guarantees being withdrawn are durable system-fault reporting in run history,
permanent stop after such a fault, verification of historical semantic evolution, and advance
history-capacity assurance. Ordinary loading also no longer verifies older physical frame links.
The retained execution, causal, command-authority, and atomicity guarantees are specified below.

## 4. Preservation and disclosure are separate responsibilities

### 4.1 What must survive

The retained error must identify the originating operation, relevant stage, each exposed causal
layer, and reviewed facts that distinguish the failure. Examples include:

- HTTP status and numeric JSON-RPC error code;
- request-send versus response-body failure;
- local deadline expiration versus an actual server response;
- SQLx category, SQLSTATE, and query/commit stage;
- OS error kind and numeric code when exposed;
- parser category and location without copying input;
- expected/observed checked identities or sizes where disclosure is already permitted;
- channel closure, task cancellation, owner failure, and opaque cryptographic rejection.

An error created locally can have no upstream source. Represent its actual checked failure rather
than inventing a transport chain. Expected protocol absence and `Pending` are not errors.

### 4.2 What must not be captured automatically

Do not serialize generic `Debug` or `Display`, arbitrary RPC message/data fields, HTTP bodies,
database messages/details, SQL parameters, URLs, environment values, configuration snippets,
request bodies, channel command objects, or panic payloads.

These values can contain credentials or caller secrets. A secret-marker filter is not proof that
arbitrary text is safe. Encryption is not permission to retain it elsewhere.

Concrete source objects may be inspected transiently within their owning boundary. Only the
reviewed representation crosses the audit boundary. For example, inspect a `reqwest::Error` before
it is consumed, but do not move its credential-bearing request context into a domain error.

### 4.3 Missing evidence must be explicit

Distinguish at least:

| Condition | Meaning |
| --- | --- |
| Withheld | The source exposed a field that this contract does not permit retaining. |
| Opaque upstream source | A source layer is present, but no reviewed structured detail is available. |
| Unavailable upstream evidence | The client API did not return evidence, including internally discarded attempts. |
| Capture bound reached | The retained chain or diagnostic reached its finite bound. |
| Original could not qualify | A proposed original value could not become an admitted canonical value. |

Record known presence and safe size facts when available. Do not invent an omitted count when the
upstream API does not expose it. A bounded or redacted record must never be described as lossless
raw capture.

## 5. Shared causal data, with local ownership

Keep the small `mfm-diagnostics` crate for checked causal data and its exact schema. Its purpose
is to remove duplicated representation, redaction accounting, and bounds validation across ports.

```text
diagnostics -> values -> canonical / ids

program, domains, store, config, signing -> diagnostics
live adapters -> their ports and concrete client libraries
```

The crate depends on existing foundational/serialization facilities, not Program, Runtime, client
libraries, or storage. It performs no IO and owns no error registry or recovery behavior.

Define the shared nested persisted schema once; Serde alone does not establish its exact shape.
Reuse it in owner errors instead of repeating a diagnostic schema in every domain. Nested schema
participation does not require independent Program identity. `EvidenceError` stays a native checked
constructor error, not a Program value or a registered decoder-error contract.

### 5.1 Shape and bounds

The shared representation separates a source's identity in the chain from its facts, and separates
response observations from ancestry:

```rust
struct DiagnosticEvidence {
    response: Option<ResponseContext>,
    sources: SourceChain,
    omissions: Vec<Omission>,
    omissions_truncated: bool,
}
struct ResponseContext {
    status: HttpStatusCode,
    rpc_code: Option<i64>, // only when a checked RPC error envelope was received
}
struct SourceChain {
    layers: Vec<SourceLayer>, // outermost captured error first; at most 32
    end: ChainEnd,
}
struct SourceLayer {
    kind: SourceKind,
    facts: Vec<SourceFact>, // at most 8 reviewed facts about THIS source
    facts_truncated: bool,
}
enum ChainEnd { Complete, Unavailable, BoundReached }
enum SourceKind { Transport, Database, Os, Parse, Task, Channel, Crypto, Opaque }
enum SourceFact {
    Transport { kind: TransportFailureKind },
    Database { kind: DatabaseFailureKind },
    SqlState { code: SqlState },
    Os { kind: OsFailureKind },
    OsCode { code: i32 },
    Parse { category: ParseCategory, location: ParseLocation },
    Size { limit: u64, observed: ObservedSize },
    Task { outcome: TaskFailureKind },
    Channel { outcome: ChannelFailureKind },
}
struct Omission {
    at: EvidenceLocation,
    field: OmittedField,
    reason: OmissionReason,
    observed_bytes: Option<u64>,
}
enum EvidenceLocation { Response, SourceLayer { index: u8 } }
```

Fields are private and checked. `DiagnosticEvidence` owns the **8 KiB total canonical budget**,
including response context, layers, facts and omission metadata. There are at most 32 omission
entries. A layer represents one concrete captured error, not one diagnostic field: an OS error can
retain both kind and numeric code in the same layer. An opaque source has `kind: Opaque` and no
invented facts; preserve any accessible deeper source in its own next layer. Source order follows
actual exposed `source()` links, not the order in which observations arrived. Owner-typed outer
wrappers already represent their own nesting and are not duplicated as fictitious client layers.
An empty chain with `Complete` is legitimate when a local checked failure has no upstream source;
`Unavailable` means the boundary knows upstream evidence was not exposed. Do not manufacture an
opaque source for a local error whose full facts are already in its typed payload.

HTTP status and a received RPC error code belong to response context. They are not automatically
ancestors of a later body or parsing failure. For a checked RPC error response without a client
source object, response context plus an empty complete source chain is valid. For headers followed
by a body failure, retain the status in response context and the body error/source links in the
chain. Do not insert the status as an extra source layer. Local operation/stage and checked domain
facts remain in the owner's exact error type (section 5.2).

Omission locations refer only to an existing layer index or a present response. Message/data/body
withholding on the response uses `Response`; a withheld client URL belongs to the actual client
layer. `ChainEnd::BoundReached` accounts for an unretained suffix without inventing an index or
number of omitted sources. `facts_truncated` accounts for a layer's omitted facts, and
`omissions_truncated` accounts for omitted omission entries. These flags have distinct meanings.

Capture retains response context first, then an outer source prefix, within the shared byte bound.
Reserve space for all fixed bound markers before appending variable entries. Stop before an entire
next layer would exceed the byte/layer bound and mark the chain end; do not merge its facts into the
previous layer. The per-layer fact bound uses `facts_truncated`. Omission overflow uses its marker.
Constructors and deserialization enforce counts, byte bounds, legal kind/fact combinations, unique
fact fields per layer, valid omission locations and response presence. Facts are emitted in their
schema-defined field order so equivalent evidence has one canonical representation. Source kinds
and omission vocabulary are closed; there is no arbitrary string/map escape hatch.

Traversal stops at its own finite bound, including a pathological cyclic source chain. Do not walk
the discarded suffix to count it. Do not format sources to infer identity or classify by messages.
The byte/layer bounds describe only shared diagnostics; owner-typed checked facts are separately
bounded by their exact value and complete persisted-value and frame contracts.

### 5.2 Existing typed wrappers retain operation context

Do not put every EVM, SQL, signer, or Runtime operation into one global stage enum. The source owner
retains its own operation/stage vocabulary:

```rust
struct ProviderFailure {
    method: EvmRpcMethod,
    stage: RpcStage,
    diagnostics: DiagnosticEvidence,
}

enum EvmOperationalKind { Unavailable, Timeout, RateLimited }

struct EvmOperationalError {
    kind: EvmOperationalKind,
    source: Box<ProviderFailure>,
}

enum EvmTransactionOperationalError {
    Provider {
        operation: TransactionProviderOperation,
        cause: EvmOperationalError,
    },
    Authority {
        operation: AuthorityOperation,
        cause: ReviewedAuthorityFailure,
    },
    Signer { cause: ReviewedSigningFailure },
}
```

The operational carrier factors the three legal alternatives into one closed kind and one common
payload: `Unavailable(P) | Timeout(P) | RateLimited(P)` is `Kind × P`. Its sole wire shape is
`{"kind":"timeout","source":{...}}`; boxing changes memory layout only. This avoids repeating the
same diagnostic schema three times. `new(kind, source)`, `kind()` and borrowing
`provider_failure()` expose the current API; no unit or compatibility constructor remains.
Transaction provider, authority and signer alternatives retain their distinct typed payloads.

These sketches preserve the current semantic categories while adding evidence. Do not introduce
a second `Classification` field that can disagree with the intrinsic classifier.

Use ordinary nested sources and borrowing. Remove incidental `Copy` requirements when errors gain
owned data. `Error::source()` should preserve a nested typed source when its type supports that
trait, but generic capability contracts remain usable with operational `MfmValue` types that do
not implement `std::error::Error`. Typed payload access remains part of the contract.

### 5.3 Capture vocabulary and extraction

The omitted-field vocabulary is closed: Message, Data, Body, Url, DatabaseDetail, Parameters,
PanicPayload, and SourceDetail. Reasons are Withheld, Unavailable, and BoundReached. Observed sizes
are integers only when known. SQLSTATE is five checked ASCII alphanumeric bytes; parser locations
retain reviewed line/column/offset. Unknown OS codes may remain numeric; unknown source categories
remain opaque while accessible deeper sources retain their own layers.

Capture walks exposed source links with owner-supplied reviewed extraction, not a shared registry
of client downcasters. The diagnostics crate imports no client libraries. Never format sources to
infer their category, and never walk an unbounded discarded suffix to count it. Retain the existing
checked capture implementation and its consuming tests instead of introducing another capture API.

## 6. One continuation representation and one owner per check

### 6.1 Runtime owns the state and its payload

Runtime owns one non-generic `RunState` and its commit payload. The payload contains the admitted
Program content reference, complete current continuation, and completed-operation facts. Admission
also contains the Program and initial context. Later commits refer to that admitted Program by
identity; they do not repeat the Program or need earlier operation records to restore context.

Continuation fields include the current position/visit, current context, active declaration
checkpoint inputs, semantic recovery usage, and phase-specific pending command or retained result.
Use enum alternatives for phases with different fields. Do not use a status plus options for every
possible callback result. Preserve baseline checkpoint eligibility, forward traversal, fresh visits
on restart, usage charging, and Effect barriers in the live Runtime implementation.

The same `RunState` value type is constructed live and deserialized on restoration. A candidate can
share immutable canonical objects with the acknowledged state. Only known insertion makes that
candidate authoritative. There is no independently authored snapshot struct or wire-only state.

### 6.2 Values objects are the engine's value representation

Unify the existing canonical `ValueView` data into one Values-owned object with private checked
fields, conceptually:

```rust
struct Object {
    value_ref: ContentRef,
    canonical: /* immutable canonical JSON */,
}
```

The nominal contract is derived from the value reference's schema identity and the existing fixed
nominal-contract digest rule. Do not also store `contract_ref` in every object. Retain distinct
contract identities where the Program's declarations require them; removing a redundant object
field does not remove association checks.

Objects are serialized inline, including their complete canonical payload. Shared in-memory
ownership is allowed; it does not create a second persisted form. There is no frame-local object
table, ref-only projection, deduplication map, cross-frame resolver, native `Any` payload, or cache
of a parallel `T`. Delete `QualifiedValue`, `ColdQualifier`, and their sole-purpose plumbing.
Do not recreate their behavior under replacement wrapper names.

At a registered typed callback, decode the required object into its concrete input `T`, execute
typed code, and encode its output or declared error into an object. This is the same route for a
fresh run and a restored run. The engine carries canonical objects, while domain code receives its
own native types. That distinction is deliberate: removing the native cache can require another
native decode when the same object is used again. Do not claim zero decoding or bypass a native
constructor before a typed action to save that cost.

Keep the native domain/operational error locally until recording completes, so a failed encoding or
append can still report its original cause. Do not retain every successful native value or every
intermediate map output in a generic custody container. Successful-value encoding failure is an
invocation failure with the last acknowledged head and the actual encoding cause.

### 6.3 Journal owns an opaque frame envelope

Journal owns frame identity, RunId, sequence, predecessor digest, canonical envelope representation,
frame hashing, and actual frame bounds. Its payload is opaque canonical JSON produced/decoded by
Runtime. Do not encode JSON as a JSON byte array or escaped JSON string; embed its canonical value
without changing the one exact hashing representation.

Move Runtime-specific lifecycle/position/outcome/recovery declarations out of Journal. Journal has
no Program dependency, State schema registry, policy-enum mirrors, execution rules, or historical
semantic verifier. It decodes the envelope, not business continuation. A frame's predecessor digest
remains part of its exact wire and append contract even when ordinary loading does not walk it.

### 6.4 Validation ownership

| Boundary | Sole responsibility |
| --- | --- |
| Journal frame decode/seal | Canonical envelope, exact framing, frame hash, RunId/sequence/predecessor field shape, and complete frame ceiling. |
| Values object admission | Canonical object identity/hash, object/descriptor size, and established value-admission constraints. Values owns the exact schema-check function; Runtime supplies the associated descriptor once at slot admission. |
| Runtime restoration | Admission/Program association, locally valid phase and positions, allowed slot contracts, checkpoint/counter cardinalities and ranges, and current command/authority relationships. |
| Typed State/adapter/policy boundary | Native constructor invariants and capability/response binding checks needed before this particular operation. |
| Runtime live execution | Legal successor construction, retry/restart authorization, usage charging, fresh visits, and Effect restrictions. |
| Store | Consistent physical read, immutable rows, atomic exact-head insertion, and actual cumulative frame/byte accounting. |

Use an existing checked result downstream; do not repeat its hash/schema validation in another
layer. Native materialization of `T` at a later use is not permission to repeat all frame admission
checks. Live output construction and cold object admission converge on Values' one implementation.
A sealed candidate is not decoded back into native objects merely to prove it can be stored.

Restoration checks all present canonical objects' structural/schema contracts. Native owner
constructors run when an object is selected for a typed operation, including recovery input.
Inspection does not claim it eagerly constructed every dormant checkpoint or terminal value.
Failure at either boundary is a causal invocation report and prevents the affected action; it is
not a durable domain failure. This avoids a separate eager native-validation registry.

Current-state validation cannot establish how counters or checkpoint values evolved. A forged but
locally valid counter reset or substituted checkpoint is not detected by historical comparison.
Correct live transitions and the append-only Store are now the authority for that evolution.
Remove old tests and documentation claiming independent historical semantic verification.

### 6.5 Concrete state and commit sketch

The following is the declaration boundary to implement, not an extensible framework. Names of
private helpers may change; ownership, alternatives, and single ownership of continuation facts
must not. `Object` means the Values type from section 6.2, with shared immutable backing.
New Runtime payload enums use ordinary externally tagged snake_case variants, such as
`{"awaiting_recovery":{"domain":{...}}}`;
structs reject unknown/duplicate fields. Reuse existing checked field contracts. All these
Runtime payload declarations have one current exact Serde shape; they are not Program values and
need no Program semantic identity, MfmValue derive, or registered value codec. In the signature
sketches, `Result<T, E>` denotes the standard Rust result, not a crate's single-error alias.

```rust
struct RunState {
    phase: Phase,
    checkpoints: Vec<Checkpoint>,
    usage: Vec<StateUsage>,
    effect_barrier: Option<StatePosition>,
}
struct Checkpoint { position: StatePosition, input: Object }
struct StateUsage { retries: u32, restarts: u32 }
struct Call { position: ExecutionPosition, input: Object }
struct EffectCall { call: Call, effect_id: EffectId, command: Object }
struct Settlement { effect: EffectCall, evidence: Object }

enum Phase {
    Runnable(Call),
    EffectPending(EffectCall),
    AwaitingInterpretation(Settlement),
    AwaitingRecovery(Failure),
    Succeeded(Object),
    Failed(TerminalFailure),
}

// A completed State invocation, including accepted observational evidence.
enum StateCall {
    Pure(Call),
    Read { call: Call, intent: Object, evidence: Object },
    Effect(Settlement),
}
struct DomainFailure { call: StateCall, original: Object }
struct ReadFailure { call: Call, intent: Object, original: Object }
enum Failure {
    Domain(DomainFailure),
    Read(ReadFailure),
    PendingEffect { effect: EffectCall, original: Object },
}
enum TerminalFailure {
    Domain { failure: DomainFailure, reason: StopReason, root: Object },
    Read { failure: ReadFailure, reason: StopReason },
}

struct RunCommit {
    program_ref: ContentRef,
    state: RunState,
    facts: OperationFacts,
}
enum OperationFacts {
    Admitted { program: Object, initial: Object },
    Succeeded { call: StateCall, output: Object },
    Failed(Failure),
    EffectPrepared(EffectCall),
    EffectSettled(Settlement),
    Recovered {
        failure: Failure,
        classification: Classification,
        request: RecoveryRequest,
        decision: RecoveryDecision,
    },
}
```

`RecoveryRequest` and the authorized `RecoveryDecision` are distinct facts: a requested Retry can
be denied and become Stop. Reuse the existing Program request/stop vocabulary. Runtime's moved
decision declaration is `Retry | Restart { checkpoint: StatePosition } | Stop { reason: StopReason }`;
delete Journal's policy mirrors and redundant StopCode conversion. Serialization of a target
position grants no authority: Runtime still checks the request against the admitted Program and
current state. Root mapping appears only in `TerminalFailure::Domain`; do not repeat the root value
in Recovered facts or put it on adapter errors.

Add the sole exact Serialize/Deserialize implementations to the existing Program Classification,
RecoveryRequest, StopReason, RecoveryLimit, and RecoveryDenial types used by this payload; do not
create parallel record DTOs. RecoveryTarget serializes transparently through its checked
StatePosition and gains checked deserialization for that representation. Reading a requested
target grants neither authoring eligibility nor execution authority; current Runtime authorization
is mandatory. These data-serialization changes add no policy callback or internal-error identity.

Admission carries Program canonical bytes in the same Object representation, without making
Program an MfmValue or deriving Serialize over its private implementation fields. Runtime calls
Program::decode_canonical for that object and checks its resulting content reference against the
commit. Its established Program document schema/constructor remain the authority for the sequence.
Program object payload bytes count toward the same object-payload accounting as initial context.

The active checkpoint vector is sorted and unique by declaration position. It retains one complete
input per active checkpoint, replacing that input on re-entry. Active does not mean currently
eligible. Compute eligibility from Program targets, active checkpoints, execution mode, and Effect
barrier; do not persist a second eligible-target collection. `usage` has exactly one entry per
Program declaration. Checkpoints contain no counter/barrier copies. The run-wide decision count
is the checked sum of all retries and restarts in usage; no independent `decisions` field or cache
is stored. Reject sum overflow or a total exceeding the Program's global allowance on restoration.

Preserve the baseline eligibility predicate: the target is declared for the failing declaration,
has an active retained input, is no later than that declaration, is strictly after any Effect
barrier, and the inclusive target-to-failure interval contains a Read. Do not widen Restart to
arbitrary historical visits or a Pure-only interval.

Restart takes the selected stored input, removes checkpoints after the target, advances the visit
from the current execution, and retains current usage/barrier. Forward success and Read Retry also
advance the visit. Pending Effect Retry preserves its existing position, visit, command and
EffectId. Only granted Retry/Restart increments its counter on the failing declaration (not the
restart target), increasing the derived run total once. Stop, denied requests, and internal
failures do not spend another grant.

Admission constructs zeroed usage, no barrier, and the initial active checkpoint when applicable.
A zero-State Program is Succeeded at admission. Later terminal states keep the same continuation
fields; no historical reconstruction is required to inspect their usage or original failure.

The driver retains the loaded admission, latest RunCommit, and mechanical head. It does not keep
an independently mutable RunState beside that commit or retain the whole history. The candidate
owns the next commit; after known insertion it replaces the current commit. Reports borrow the
current commit. In particular, RunnableReason and pending Effect `latest_failure` are derived from
current facts; neither is copied into Phase. Pending without append leaves those facts unchanged.

Use these exhaustive relations to validate the **one loaded commit**, with no predecessor input:

| Current facts | Required current phase and relation |
| --- | --- |
| Admitted | Sequence 1 only, no predecessor; exact admitted Program/initial value; zero usage/no barrier; Runnable at declaration 0/visit 0 with its initial checkpoint, or Succeeded for an empty Program. |
| Succeeded | The completed StateCall mode/position matches its declaration. Its output becomes the next declaration's input with visit + 1; Succeeded is allowed only after the final declaration. Advancing to a different later declaration is invalid even if schemas match. |
| Failed | AwaitingRecovery with the same Failure. Domain failures retain their actual StateCall; Read operational failure has no accepted evidence; pending Effect failure retains its exact EffectCall. |
| EffectPrepared | EffectPending with the same EffectCall and a declaration whose mode is Effect. |
| EffectSettled | AwaitingInterpretation with the same Settlement. |
| Recovered Retry | A Read failure selects the same declaration/input and visit + 1; a pending Effect failure selects its unchanged EffectCall. Pure and settled Effect retries are invalid. The request must be RetryState. |
| Recovered Restart | Runnable at the requested permitted active checkpoint/input and visit + 1 from the failing call; later checkpoints are absent. Pending/settled Effects cannot restart. |
| Recovered Stop | Domain/Read failure produces the corresponding TerminalFailure with the same original/call and reason; only domain Stop has a mapped root. Pending Effect failure returns the same EffectCall to EffectPending. A denied request can produce Stop. |

Active checkpoint positions are declared checkpoints at/before the current continuation position;
if the selected call is itself a checkpoint, its retained input agrees with that call's input.
For terminal success use the completed final call in current facts, not the terminal output as a
checkpoint input. A barrier names an Effect declaration. Pending/settled Effect calls have their
barrier at that Effect; an ordinary Runnable call must be strictly after any barrier. Check allowed
slot contracts for every object in state **and facts**, using the declared operation/position.

Do not reevaluate classification, policy, or root mapping, reconstruct an earlier state, or verify
changes from earlier counters/checkpoints. Current ranges, static mode restrictions, and
same-commit identities are checkable; an otherwise locally valid forged past evolution remains
outside this contract. The table is a current-record invariant, not a restored history validator.

Externally tagged Runtime enums are required for the inline raw-JSON representation. With Serde
1.0.228/serde_json 1.0.150, an adjacent `kind`/`data` enum buffers the payload when canonical key
ordering puts `data` first; nested RawValue decoding then fails. Use ordinary streaming-compatible
Serde decoding, not custom tag buffering, escaped JSON, or an intermediate Value tree that loses
duplicate-key evidence. Public transport `kind` tags remain separate projections.

Keep one private Runtime current-record check, conceptually
`RunCommit::validate_current(&self, run_id: &RunId, sequence: u64, executable: &ExecutableProgram)`.
It receives no predecessor or history. It checks the table, slot identities and static relations,
including EffectId derived from RunId, Program ref, execution position, and command ref. Values
performs schema admission for newly loaded object occurrences once; live objects already admitted
from typed values do not repeat that schema/hash work here. Typed constructor/binding checks run
at the selected operation entries in section 8.6.

## 7. Loading, append authority, and reconciliation

### 7.1 Admission and latest state only

Ordinary show/resume loads admission, latest committed frame, and mechanical head metadata in one
consistent snapshot. If latest is admission, return that one frame without manufacturing another
state. Missing required rows, inconsistent head identity, invalid bytes, and bad payloads are
reported as internal failures before execution.

PostgreSQL retains one repeatable-read load transaction. Use the existing head row's count/byte
accounting maintained by atomic append; do not aggregate or fetch every history row on normal load.
The Store returns bytes and physical metadata without decoding Runtime payloads. The memory Store
provides the same observable snapshot semantics under its existing synchronization.

An outstanding exact append candidate may request one additional sequence row in the same snapshot.
This is a mechanical point lookup. Do not add a semantic-history API, audit explorer, or independent
verification service as compensation for deleting the historical scan.

### 7.2 Append and candidate adoption

Runtime constructs its next complete state and actual operation facts, admits any new Values
objects, and passes the canonical payload to Journal for sealing. Keep the exact sealed candidate
until its append disposition is known. Do not introduce a generic commit facade around these calls.

Store uses the current exact-head protocol: all-or-nothing insertion, immutable prior frames,
advisory-locked synchronous-commit PostgreSQL append, actual cumulative limits, and the existing
definite-versus-indeterminate acknowledgement distinction. It obtains no Program or reducer logic.

| Result | Runtime action |
| --- | --- |
| Known insertion | Adopt the same constructed candidate, then dispatch its permitted next operation. No encode/decode ownership round trip. |
| NotInserted | This append wrote nothing, but identical candidate bytes may already exist. Reload admission/latest with the candidate-sequence probe; follow the collision rules below and perform no further work in this invocation. |
| Store error, including Indeterminate acknowledgement | Return the causal recording error immediately with the exact candidate. Preserve the Store's disposition; perform no automatic reload, probe, retry, or successor action. |
| Candidate encoding/actual-limit failure | Append nothing. Report the original error when present and the recording cause; leave the acknowledged state authoritative. |

Only NotInserted triggers Runtime's automatic reconciliation read in this delivery. Store errors
retain the baseline immediate-return behavior; the native report holds the candidate for caller
custody without adding a reconciliation command or endpoint. A fresh resume follows D1 below.

For a candidate probe, matching exact bytes at the candidate sequence proves insertion. Different
immutable bytes at that sequence exclude this candidate. Absence while the current head remains
below that sequence does not prove a still-in-flight COMMIT failed: retain Indeterminate unless the
Store protocol establishes completion. A missing requested row at or below the head is corruption.
The invocation retaining an unresolved candidate cannot retry provider/recovery work merely
because its row was absent in one snapshot.

After resolved reconciliation, restore the latest returned state, which may be later than the
candidate. Finding the candidate does not authorize execution from a stale candidate state.
These comparisons preserve physical append authority without replaying historical transitions.

`start` retains the baseline collision behavior. On admission NotInserted, compare the loaded
admission's exact Program and initial object with the proposal. A mismatch returns AdmissionConflict;
a match returns the latest checked observation without driving it. Repeating start is not resume.
Missing admission or failed load/decode returns its causal invocation error, never an unrelated run.

For non-admission NotInserted, bind the probed row as specified in section 7.4, then:

- Matching candidate bytes prove that original is durable. Return the latest checked observation
  and yield, even if this invocation would normally continue after its own successful insertion.
- Different bytes at the candidate sequence exclude this candidate. Return an execution-stopped
  recording/conflict error with the available original, candidate, and latest checked observation.
- An absent candidate or failed reload does not prove it was never inserted by another invocation.
  Report the known NotInserted result for this attempt and the snapshot absence or reload cause,
  retaining original/candidate. Do not re-enter policy, State execution, or an adapter.

The baseline Store may return NotInserted for an identical already-stored frame. Do not collapse
this disposition into either global absence or known insertion without the exact probe evidence.
If the NotInserted reload fails, its error is secondary and must not replace the append result.
Include the checked probe's presence/exclusion/absence finding in recording-failure detail when
available; absence of a finding means the probe could not establish one. Store errors have no
automatic probe finding and retain their own definite/indeterminate disposition.

Candidate custody is invocation-local. Keep `resume(run_id)` and its current transport requests;
do not add a request token, persisted ambiguity registry, or candidate-upload endpoint. A later
explicit resume is a new invocation: it loads then-current admission/latest and may progress from
that acknowledged phase without claiming to have resolved an unavailable earlier candidate.
Competing recovery decisions and Effect preparations still require exact-head insertion before
action; pending Effect reconciliation uses the same command/EffectId. A settlement committed after
this fresh read cannot retract reconciliation already started. Once restoration observes
AwaitingInterpretation, that invocation enters no Effect adapter.

### 7.3 Persistence cutover

Change the current persisted contract and reject obsolete data. Update/reset the existing baseline
and its fixtures under the repository's pre-release policy. Do not provide a legacy decoder, mixed
format path, migration of run histories, or writable rollback. Query changes include their SQLx
metadata and managed PostgreSQL checks in the same commit.

### 7.4 Store and Journal interface sketch

Replace the complete-prefix load result in the existing Store trait. The future alias below is
notation for its existing boxed, Send, lifetime-bound future, not a new dependency or executor.
Keep `append_run` and its Inserted/NotInserted/error dispositions.

```rust
trait Store: Send + Sync {
    fn load_run<'a>(
        &'a self,
        run_id: &'a RunId,
        probe_sequence: Option<u64>,
    ) -> BoxFuture<'a, Result<Option<LoadedRun>, StoreError>>;

    fn append_run<'a>(
        &'a self,
        frame: &'a EncodedRunFrame,
    ) -> BoxFuture<'a, Result<AppendResult, StoreError>>;
}
struct LoadedRun {
    head: RunSummary,
    admission: Arc<[u8]>,
    latest: Arc<[u8]>,
    probe: Option<Arc<[u8]>>,
}
impl LoadedRun {
    fn new(
        head: RunSummary,
        admission: Arc<[u8]>,
        latest: Arc<[u8]>,
        probe: Option<Arc<[u8]>>,
    ) -> Result<Self, StoreError>;
}
```

Expose the checked constructor and borrowing accessors to Store implementers; keep fields private.
Construction checks transfer-length/metadata bounds, not Journal bytes or Runtime semantics.
The concrete Store enforces snapshot consistency and requested-row presence. `None` for the outer
result means no run exists. For a requested probe, `probe: None` means no row at that snapshot; without a
probe request it means no probe was performed. The caller retains its request, so another result
status enum is unnecessary. Validate a requested sequence is in the supported frame-count range.
Admission/head/probe referring to the same row share bytes. Store rejects a missing requested row
at/below its reported head; absence above the head remains consistent with an in-flight append.

After Journal decode, Runtime binds the returned envelopes to this read: all RunIds equal the
requested identity; admission has sequence 1; latest sequence/digest equal RunSummary; a present
probe has the requested sequence. Perform those checks before treating unequal candidate bytes
as definite exclusion. Store checks physical rows/metadata without decoding Journal bytes. These
are checks of the bounded returned rows, not a predecessor-chain scan.

Journal exposes sealing and decoding over its existing EncodedRunFrame type:

```rust
fn seal_frame(
    run_id: &RunId,
    sequence: u64,
    previous: Option<&ContentDigest>,
    payload: &PlainCanonicalJsonBytes,
) -> Result<EncodedRunFrame, JournalError>;
fn decode_frame(bytes: &[u8]) -> Result<EncodedRunFrame, JournalError>;
```

The frame exposes checked identity/header fields and a borrowed canonical payload. Its current
wire has `domain`, `run_id`, `run_sequence`, `previous_head_digest`, and `payload`; the digest is
computed from exact canonical frame bytes under the existing hashing rule. Replace the old
record/object-table wire and its domain marker in the single cutover; retain no old reader.
Runtime encodes/decodes RunCommit inside payload and owns the payload's exact field/tag contract.
Keep frame sequence numbering, RunId spelling, digest algorithms and exact-head append checks.

A new admission/latest query must not acquire history semantics indirectly through a count/sum
query or another load helper. Required rows and head metadata come from one repeatable-read
snapshot. The exact candidate probe is the only additional lookup this API permits.

## 8. Execution and recovery lifecycle

### 8.1 Retained operation facts

Every commit contains a complete continuation and the facts explaining the completed operation.
Use the executed position/input, actual result, intent or retained command/EffectId, and accepted
evidence as applicable. A declared failure includes its original typed error and reviewed chain.
A recovery commit includes the originating failure/input/request, classification, authorized
decision or denial, and resulting state. Each commit remains understandable without replaying
earlier operation facts.

Where state and operation facts mention the same object, share immutable ownership in memory and
serialize each required occurrence. Do not make separately mutable context copies or add an
intermediate-recovery timeline. No internal error is an operation-history variant.

### 8.2 Phase behavior

The phase names below describe behavior; use the smallest concrete enum expressing it.

| Acknowledged phase | Permitted work and next durable result |
| --- | --- |
| Runnable | Execute the current Pure/Read State, or prepare the Effect command. Success advances with the complete output/context. A domain/operational failure commits the original and AwaitingRecovery. Effect preparation commits command identity before adapter entry. |
| EffectPending | Reconcile only the retained command/EffectId. Pending retains that authority. An operational failure commits its original under the existing pending-Effect recovery restrictions. Accepted settlement commits its evidence and awaiting-interpretation state before interpretation. |
| AwaitingInterpretation | Interpret the retained accepted Effect evidence. Success advances; a domain failure commits its original for recovery. An internal failure appends nothing and leaves this phase available for explicit resume. |
| AwaitingRecovery | Classify the committed original, invoke the selected handler, and authorize the recommendation. Perform root mapping only if the authorized result is a terminal domain Stop requiring it. Commit the normal decision and resulting state before action. |
| Completed / stopped by a normal decision | Expose the retained result. Resume performs only work permitted by the acknowledged decision and existing unresolved-command rules. It does not invent a new command or recovery grant. |

A Read is duplicate-safe. If its interpretation fails internally before an outcome commit, explicit
resume may perform that Read again. Do not add an intermediate Read-evidence phase solely to make
internal failures durable. Effects need the separate accepted-settlement phase because their
retained command and reconciliation authority have different consequences.

Persisting settlement records the accepted external observation, not the interpreter failure.
When restoration yields AwaitingInterpretation, it invokes interpretation only. This does not claim
that cancellation can retract a concurrently running adapter invocation from another process.

Stop does not revoke an already committed unresolved Effect command. Explicit resume reconciles
that same command under the existing protocol, without reclassifying the stopped error or creating
another command. Accepted settlement then permits interpretation only. Keep this distinction
between stopping recovery and resolving existing Effect authority explicit in the concrete phases.

### 8.3 Failure durability precedes policy

The order is strict:

1. Retain the native declared domain/operational error and executed input/request.
2. Build and append the failure commit with AwaitingRecovery.
3. After known insertion, classify that original and invoke the selected handler.
4. Authorize the recommendation against the current state and semantic limits.
5. Perform root mapping only if the authorized result requires a terminal domain Stop. A denied
   Retry/Restart can result in such a Stop; an authorized retry must not perform root mapping.
6. Commit the decision/resulting state before executing Retry/Restart or other permitted work.

An internal classifier/handler/map failure leaves the original durably AwaitingRecovery. Report the
system failure to this caller. Explicit resume may rerun that unfinished recovery evaluation; no
once-only claim applies to uncommitted in-memory policy progress. Ordinary Stop or denial is a
normal recorded recovery result, not a system fault.

Runtime derives the recovery-context view from RunState and the admitted Program, including current
usage and eligible targets. Handlers receive that view with Classification directly. Delete
IncidentSummary and adapter_context rather than maintaining them beside this route. Keep
intent/command/input consistency checks at the typed owner that can establish
them. Local preflight mismatch performs no provider call or append. Post-response binding rejection
reports actual IO already performed but does not fabricate a durable authenticated observation.

### 8.4 Internal failures and explicit resume

The user selected retry from the last committed state. An internal failure ends the invocation,
retains its causal report, and appends no fault or hidden stop marker. After restart, that report is
not recoverable from run history. A later explicit resume retries the unfinished acknowledged phase.
No automatic retry loop is introduced.

Preparation failure retries preparation; Read/internal evaluation failure may repeat the State;
handler/map failure retries recovery evaluation; interpretation failure after committed Effect
settlement retries interpretation only. Recovery usage is charged by committed normal decisions,
not by an internal error that was never recorded. Pending Effect reconciliation always retains the
same command and EffectId, including after a recording failure.

Cancellation, task failure, an unrecordable result, or process death does not prove a physical
attempt was recorded. Report what is available during a live invocation; do not promise post-crash
custody of uncommitted errors or evidence. Store ambiguity remains governed by section 7.2.

### 8.5 Driver continuation and return points

Retain the existing small driver distinction between continuing this invocation and yielding its
current observation. Do not create a scheduler, persisted job, or additional retry budget.

| Just-acknowledged operation | Next phase | This invocation |
| --- | --- | --- |
| State success | Runnable or Succeeded | Continue the next State, or return success. |
| Domain/operational failure | AwaitingRecovery | Continue recovery evaluation only after insertion is known. |
| Effect preparation | EffectPending | Enter the retained command's adapter. |
| Effect settlement | AwaitingInterpretation | Interpret the committed evidence. |
| Authorized Read Retry or Restart | Runnable | Yield the authorized fresh visit with retained/restored input; a later explicit resume executes it. |
| Pending Effect Retry | EffectPending | Charge the grant once and yield; a later explicit resume reconciles the unchanged command. |
| Pending Effect Stop, including a denied request | EffectPending | Return RecoveryStopped; later explicit resume reconciles the unchanged command without reclassifying this old error. |
| Domain/Read terminal Stop | Failed | Return the durable failed observation. |
| Adapter Pending without new evidence | Unchanged EffectPending | Yield without append or another grant. |
| Internal/recording failure | Last acknowledged phase | Return a causal invocation failure; no automatic retry or fallback append. |

On a fresh resume, dispatch solely from the latest phase. Recovered facts explain the prior decision
for inspection; they are not a command to rerun the old handler or charge its usage again. When
failure recording is ambiguous, stop before policy. When decision recording is ambiguous, stop
before dispatch. When settlement recording is ambiguous, stop before interpretation. Retain the
candidate under section 7.2 without granting work from stale or uncertain state; a Store error
does not start an automatic reconciliation loop.

### 8.6 Typed operation-entry checks

The checked callback-entry rules below complete the workflow in section 8; they are implemented
by existing monomorphized Runtime runners, not by a new validation registry:

| Entry | Typed checks and operation |
| --- | --- |
| Runnable Pure/Read | Decode the selected complete input. Pure evaluates it; Read prepares/encodes its intent, invokes the adapter, then decodes returned evidence and calls C::bind_evidence with the exact intent ref/intent before S::interpret. |
| EffectPending | Decode State input, recompute deterministic S::prepare, and compare its canonical command/ref with the retained command. Use the EffectId already checked against RunId/Program/position/command by current-row validation. Only then enter the adapter with that retained identity/command. |
| Returned Effect settlement | The pending runner decodes command/evidence and calls C::bind_evidence before constructing the settlement commit. A rejected binding appends nothing and retains true post-IO stage information. |
| AwaitingInterpretation | Decode retained State input/command/evidence, call the same C::bind_evidence, then S::interpret. No provider call or command preparation is allowed in this phase. |
| AwaitingRecovery | Decode the selected original error and current policy/map parameters, classify, request, authorize, and map only when required. Do not rerun the completed State or adapter to reconstruct its original. |

The erased adapter encodes its returned evidence but does not repeat State-level command checks or
the runner's evidence binding. Reuse each capability's existing bind_evidence implementation.
Current-row validation owns the EffectId derivation check once per admitted candidate/restoration.

The uniform pending entry deliberately repeats deterministic preparation across the durable
preparation boundary, including immediately after hot preparation. Likewise, binding at settlement
admission and binding at interpretation are two typed uses separated by a durable phase boundary.
Keep this explicit cost of one dispatch route; add no cached proof, native-value cache, or alternate
hot path. Preparation/check failure leaves the acknowledged command intact and performs no IO.

## 9. Native errors and invocation reports

### 9.1 Preserve causes without turning system errors into Program values

Keep actual domain/operational errors as their existing exact persisted contracts, expanded only
where reviewed causes are missing. Their nested diagnostic data needs an exact schema because it
is genuinely stored. Native constructor errors and internal callback failures are different: they
have no `MfmValue` requirement, semantic identity, persisted error schema, or decoder-error codec.

Use one native source-preserving invocation-error boundary through existing Runtime/Application
reporting. A checked decoder returns its owner's concrete constructor error; the erased boundary
retains that native cause and an owner-reviewed diagnostic projection. Owner error variants, child
causes, operation/field location, and reviewed expected/observed facts survive. Serde's formatted
custom error is not sufficient custody when it discarded an available native source.

Values owns the native cause erasure needed by its decoding boundary. Runtime reuses that carrier
with operation context; Application projects it. Keep the concrete reviewed cause and its reporting
operation together, rather than maintaining a second error tree. Values imports no Runtime/domain
variants. Concrete client errors are inspected and converted to reviewed owner data before this
boundary; the carrier does not accept credential-bearing client objects. No new public error trait
or client downcaster registry is needed.

A native checked-construction hook may be added to the existing value codec/authoring path when a
real consuming callback needs it. Prove it with a shipping checked type whose distinct failures
reach Application before migrating another family. Do not add an eager cold qualifier, parallel
value registry, universal internal-result ABI, per-error identity, recursively decoded decoder
error, or automatic migration of every foundational constructor.

Raw wire types or native field-error companions are justified only by an actual checked field on
that consuming path. Reuse the owner's constructor and the same native error in ordinary decoding
and Runtime invocation. Reviewed serialization can project that native error into an invocation
report. Exact persisted schema participation is required only when its data is actually part of a
stored domain/operational result; an independently registered identity is not required for every
nested field. Showing an error in a report does not make it a Program value or require a second
report DTO tree.

Native erasure must have bounded, reviewed reporting. Reuse existing Values byte ceilings and the
shared diagnostic bounds; do not serialize an unbounded native object first and check size later.
No generic Debug/Display capture, unit replacement, source stringification, or second transport
error system. Public rendering may omit permitted detail only with explicit omission accounting.

### 9.2 Report recording failure alongside the original

A report identifies the failed operation/stage, last acknowledged head, actual recording
disposition, and reviewed causal chain. If a domain/operational error could not be recorded, retain
that original with the separate encoding/Store cause. Once a complete candidate exists, retain its
exact identity/bytes within the invocation's existing custody so ambiguity can be reconciled.

Before a canonical original exists, the native error is the available original; report that its
persistence failed. Do not invent a reference. If a callback never returned because of task failure
or cancellation, report that fact rather than fabricate an error or successful result it did not
return. Avoid per-stage native-success stashes and intermediate mapping-output collections.

There is no fallback append, shortened replacement outcome, recursive Store-error record, new
logging system, or second audit store. A Store failure cannot be durably audited through that same
failed Store. An internal report can be delivered to this caller; the run history will not make it
available to a later caller after a crash. An error while rendering/delivering the report likewise
does not prove delivery of the original.

### 9.3 Concrete native-error and callback boundary

Use one Values-owned `NativeCause` with private erased storage. It retains the concrete reviewed
owner value and its bounded serialization operation; it is not itself MfmValue or a persisted
schema. The two construction cases below use the same storage/projection mechanism:

```rust
impl NativeCause {
    fn from_error<E>(error: E) -> Self
    where E: std::error::Error + serde::Serialize + Send + Sync + 'static;

    fn from_original<E: MfmValue>(original: Arc<E>) -> Self;
}

trait MfmValue /* existing bounds retained */ {
    // Existing descriptor/identity methods remain.
    fn decode_native(bytes: &[u8]) -> Result<Self, NativeCause>;
}
```

`from_error` accepts only reviewed owner errors, including reviewed client extraction; implementing
Serialize alone is not disclosure authorization. `from_original` preserves a declared domain or
operational error without introducing an Error bound on every MfmValue/capability error. Both
retain typed downcast access within native library custody and project from that same original.
Any/erasure here holds an invocation error only, never a cache of executable context/success values.

Give `decode_native` a default ordinary Serde implementation with reviewed parse category/location.
An owner whose constructor cause would be flattened overrides that one hook with raw-wire decode
and its existing checked constructor. There is no default-then-fallback retry. An optional native
`DecodePersisted` helper can implement the override; it is not a new mandatory MfmValue supertrait.
The default does not close known checked-constructor loss sites: migrate those actually reached by
the slice, then finish the named remaining owner inventory. A derive emits this override only for
used owner construction; it does not generate decoder identities or an error-schema subsystem.

`Object` deserialization checks canonical bytes and claimed content hash. Values validates its
exact descriptor when Runtime checks the admitted slot, using the already-associated descriptor.
Native materialization selects `T::decode_native`; it checks expected contract identity but does
not repeat canonical hashing or schema validation. Hot `Object` construction from `&T` uses the
existing canonicalize/admit implementation. None of these routes stores native T in Object.

Replace the baseline unit `PreparationError`, `StateExecutionError`, and `AdapterInvariantError`
with this shared native cause at their existing boundaries; do not wrap each in another unit-like
layer or introduce per-callback associated internal-result contracts. The resulting signatures are:

| Existing boundary | Target signature / change |
| --- | --- |
| PureState::evaluate | `(Self::Input) -> Result<ProposedStateOutcome<Self::Output, Self::Failure>, NativeCause>` |
| ReadState<C>::prepare | `(&Self::Input) -> Result<C::Intent, NativeCause>` |
| EffectState<C>::prepare | `(&Self::Input) -> Result<C::Command, NativeCause>` |
| ReadState<C>/EffectState<C>::interpret | `(Self::Input, &C::Evidence) -> Result<ProposedStateOutcome<Self::Output, Self::Failure>, NativeCause>` |
| Handler::handle | `(&Self::Params, Classification, &RecoveryContext<'_>) -> Result<RecoveryRequest, NativeCause>` |
| ValueMap::apply | `(&Self::Params, Self::Input) -> Result<Self::Output, NativeCause>` |
| ClassifyError::classify | Keep `(&self) -> Classification`; it remains deterministic and infallible. Decoder/task failure around invocation is separate. |
| AdapterError<E> | Keep `Operational(E)` and replace the unit `Invariant` payload with NativeCause. |
| Capability evidence binding | Keep each existing intent/command/evidence/ref argument and replace a lossy binding error with NativeCause; do not repeat the check in Journal. |

The State/handler/map implementation and successful/domain-result ABIs keep their existing exact
associations except for the explicitly deleted contextualization fields. Internal errors are not
added to Program authoring, wire identities, or registration keys. Framework boundaries add the
actual operation/stage to RuntimeError while retaining NativeCause. Native port errors that already
preserve typed sources need not be erased before reaching this heterogeneous boundary.

### 9.4 Custody through the erased return and append

A typed original kept only inside a callback would be lost when that callback returns Object to
the driver. Use this one private transient return for declared failures across that boundary:

```rust
struct ReturnedFailure {
    object: Object,
    original: NativeCause,
}
enum AdapterReturn<T> {
    Observed(T),
    Operational(ReturnedFailure),
}
```

This is the explicitly required uncommitted original-error custody. It is not serialized, does
not enter RunState, and holds no successful native object or intermediate callback progress. Both
State-domain and adapter operational returns reuse ReturnedFailure; do not create two parallel
custody structs. A conversion that fails before Object exists instead returns RuntimeError carrying
the original plus the encoding cause. Absence of a canonical reference is represented honestly.

Keep the existing monomorphized async State runners and their driver context. Their typed operation
returns either an Object success, ReturnedFailure, or native RuntimeError; the runner constructs
and appends the operation commit. Erased adapter callbacks use the following private result shapes:

```text
Read:   (&Object) -> BoxFuture<Result<AdapterReturn<Object>, RuntimeError>>
Effect: (&EffectId, &Object)
            -> BoxFuture<Result<AdapterReturn<EffectAdapterOutcome<Object>>, RuntimeError>>
```

The monomorphized adapter closure decodes its concrete request, passes its exact object reference
to the existing typed adapter, and returns canonical evidence or ReturnedFailure. Operational
failure is different from a local invariant/encoding/task RuntimeError. State recovery callbacks
similarly consume the required Object references and return Classification, RecoveryRequest, or
mapped Object through existing registration functions, with native RuntimeError on failure.
No ColdQualifier, returned-success erasure, or second registration family is introduced.

The ownership sequence for an original E is:

```text
typed operation returns E
    -> async runner retains Arc<E>
    -> immediately awaited pure encoding job borrows its own Arc clone
    -> encoding failure: return original + encoding/task cause
    -> encoding success: ReturnedFailure carries Object + NativeCause owning the original Arc
    -> driver retains that original while constructing/sealing/appending the candidate
    -> known insertion: canonical original is durable; drop transient native custody
    -> recording failure/ambiguity: transfer original + cause + available candidate to the report
```

Do not move the sole original into the encoding job. If the operation itself never returned from
its task, no returned original exists and task failure must say so. A later policy failure uses
the already-durable original and its own native cause; it needs no copy of every prior map output.

Extend the existing RuntimeError/InvocationFailure surface, rather than adding a second public
report tree. Internal failure holds operation/stage and NativeCause. Recording failure additionally
holds the available original and distinguishes pre-encoding failure from an append disposition
with a sealed candidate. Candidate bytes remain native library custody; transport exposes only
candidate RunId/sequence/digest and the reviewed causes, not the complete candidate frame.

Use one boxed recording variant inside RuntimeError, retaining InvocationFailure::Execution as
the public invocation container. Its minimum payload is:

```rust
enum RecordingFailure {
    BeforeAppend {
        original: Option<NativeCause>,
        candidate: Option<EncodedRunFrame>,
        cause: NativeCause,
    },
    Append {
        original: Option<NativeCause>,
        candidate: EncodedRunFrame,
        outcome: AppendFailure,
        observation: Option<(RunSummary, CandidatePresence)>,
        reload_cause: Option<NativeCause>,
    },
}
enum AppendFailure {
    NotInserted,
    Store(StoreError),
}
enum CandidatePresence { Present, Excluded, Absent }
```

BeforeAppend's candidate is absent until sealing succeeds; after sealing, retain it even if a
later actual metadata check fails. Append always has exact candidate bytes. An absent original
means this operation returned no declared error, such as successful-value encoding failure;
it is not permission to discard one. observation retains the mechanical probe result separately
from the latest view; the candidate may precede the latest head. Present normally returns the
checked observation successfully, but remains reportable if subsequent payload/view decoding
fails. Excluded/Absent return a recording error. reload_cause includes failed load, qualification,
or projection and is secondary to the known append result. For Store errors, observation and
reload_cause are absent because Runtime returns immediately without probing.
The enclosing error retains its required operation/stage context, and InvocationFailure retains
the latest checked observation. Remove incidental Copy/Eq requirements on errors now owning data.
Classification/size/code accessors borrow these errors; do not consume or clone away their causes
to satisfy baseline helpers such as size_limit and Application's map_runtime_error.

The primary StoreError preserves unavailable versus indeterminate. Do not turn a failed
NotInserted probe into a new claim about insertion. If projection fails after known
insertion, the existing invocation error additionally retains the acknowledged RunSummary and
the projection cause; last_observed may still be the earlier renderable view. That view must not
be presented as the latest acknowledged head. This is invocation data, never another persisted
fault/stop state or a second reporting API.

## 10. Close remaining owner gaps after the core cutover

Work through the existing first-loss inventory. Each slice follows one actual cause from its owner
to either the native invocation report or a persisted domain/operational result, as section 2.3
requires. Update the complete consumer closure and remove its lossy conversion in the same commit.
Do not prepare unrelated type families in advance.

| Owner slice | Required retained evidence and unchanged constraints |
| --- | --- |
| RPC/EVM | Retain the baseline method/stage, response status, numeric RPC code, exposed source layers, checked field/range/size facts, and omissions. Headers survive a later body failure. Keep response/deadline bounds, duplicate-safe Read rules, classification meaning, and command identity. |
| PostgreSQL Store/config/index/custody | One SQLx-specific extraction owner retains category, SQLSTATE, query/transaction stage, and failed gate facts. An acquired connection's gate error must reach its caller; a pool timeout cannot replace it. Keep split roles, authority/epoch/schema checks, latest-state repeatable-read load, and advisory-locked synchronous-commit append. |
| Signing/keystore/channel/task | Preserve reviewed nested signing/owner/channel/task causes without rejected key-bearing commands or panic payloads. Keep keystore non-Send/non-Sync ownership and test it explicitly. Opaque crypto rejection remains explicit. |
| Memory Store/index and application | Retain actual physical/owner/parse/entropy/filesystem/environment-access causes. Expected absence is not failure; exclude paths or input values when they can disclose secrets. |
| CLI/REST delivery | Keep one current safe report surface with causal detail and accurate acknowledgement/delivery disposition. Parse/render only; no separate diagnostic endpoint or recovery implementation. |

Source extraction belongs with the concrete client owner, not in diagnostics or Runtime. Do not
introduce global operation/downcaster registries, substring error routing, concurrent last-error
caches, or an unchecked protected acquisition path. Client-internal attempts inaccessible to that
owner are explicitly unavailable. Preserve each port's own acknowledgement semantics; not every
custody/configuration operation has Store's append disposition.

Tests inject distinguishable nested failures through the public consumer, asserting retained
layers, classification where applicable, redaction, and disposition. Domain/operational routes
also demonstrate cold preservation. Internal routes assert invocation reporting and no appended
fault; they do not need persisted internal-error schemas or hot/cold fault tests.

### 10.1 Current public projection

Keep Runtime `start`, `resume(&RunId)`, and `read(&RunId)` and the existing App/CLI/REST request
surfaces. RunView remains an owned immutable projection of admission, current commit, and head,
optionally sharing Arc-backed commit/object data. Its return type gains no borrowing lifetime and
it is not another mutable continuation model. Do not serialize all checkpoints/usage implementation
fields as the public RunView merely because RunState now implements Serialize.

Add the two new incomplete phase alternatives to the current RunView/App serializer:

| Public state tag | Required projection |
| --- | --- |
| `runnable` | Position and reason derived from the current commit: admission/success means Advance; Recovered Retry/Restart gives that reason. |
| `effect_pending` | Position, EffectId, and latest failure/decision from current Recovered facts when present. Replace `state_context` with actual input/command facts; do not retain a contextualization API. |
| `awaiting_recovery` | Position, original failure and its Pure/Read/Effect operation inputs, intent/command and accepted evidence as applicable; no classification/decision is fabricated before it commits. |
| `awaiting_interpretation` | Position, EffectId, and accepted settlement command/evidence. This is an incomplete state with no recorded internal fault. |
| `succeeded` / `failed` | Existing complete success value or derived terminal failure report, using the current Object and TerminalFailure data. |

Successful observation of either new incomplete phase follows existing nonterminal policy: REST
returns 200 and CLI exits 1. An invocation that failed internally while leaving that phase returns
the existing execution-stopped error surface, CLI exit 2, and the reviewed REST error status.
Pending recovery Stop still returns RecoveryStopped/503. Actual size failure remains 422; ordinary
durable Failed observation remains 200. A new phase does not require a new endpoint or HTTP policy.

Keep the existing error-category/status projection while retaining its source. In particular,
replace Application's lossy AppendIndeterminate payload with
`AppendIndeterminate { recovery: RunRecovery, invocation: InvocationFailure }`; derive last_observed
from the retained invocation instead of storing it twice. Its code remains run_append_indeterminate
and REST status 503. from_invocation recognizes the primary Store disposition even when nested
inside RecordingFailure, then moves the whole invocation into that variant. Other recording Store
errors project their existing categories: unavailable 503, actual size/arithmetic 422, corrupt
physical state 500. Internal constructor/task/encoding failures are 500 unless an actual size
violation supplies the existing 422 category. Never replace a primary append disposition with the
secondary reload/projection error's code.

A NotInserted probe that cannot return a checked matching observation is an execution-stopped
invocation with code run_append_not_inserted and REST 409, retaining its observed presence/exclusion/
absence and secondary cause. Add that one category to the existing projection; do not call it
AdmissionConflict, which remains reserved for an incompatible repeated start. All these failed
invocations use CLI exit 2. Matching repeated start or matching non-admission candidate instead
returns the checked observation under section 7.2, with no further work.

Derive the existing public object `contract_ref` when required by the transport; it is not stored
again in Object. Retain the public raw canonical `value` position and exact value identity. Add
reviewed causal detail to the existing invocation serializer, including original versus recording
cause and known/unknown acknowledgement. Do not put raw candidate bytes or native client objects
in JSON. A postcommit projection/delivery failure retains known insertion and cannot reopen work.

Keep the baseline derived FailureReport's 32 MiB ceiling and content-addressed report contract.
Validate that concrete terminal report before its terminal append; if it cannot fit, leave the
original AwaitingRecovery and report the construction/size cause. Do not add a persisted report
copy or another lifecycle reservation. This is an actual report bound in addition to frame bounds;
include it in the shipping-size proof. Reports for the new incomplete phases derive from their
existing objects without constructing a second independently maintained error tree.

## 11. Actual limits and the bounded core proof

### 11.1 Remove prediction, keep actual limits

Delete `ConclusionBound`, `EffectBounds`, `HistoryBound`, `LifecycleBound`, Program history-bound
authoring/wire fields, `validate_admission_bound`, `current_frame_bound`, declared-frame checks,
`max_pending_failures`, `failure_frame_bytes`, capacity-only pending counters/dispositions, and
shipping-domain calculators existing solely to supply them. Remove their tests/docs. Preserve
semantic retry/restart/global-decision limits, checked arithmetic, and actual byte/count limits.

Keep the baseline object and descriptor ceilings and these actual wire/storage ceilings:

| Quantity | Ceiling / accounting |
| --- | --- |
| Canonical value object | 32 MiB under the existing Values rule. |
| Derived terminal FailureReport | 32 MiB, checked before terminal append under the existing report rule. |
| Complete frame | 134,283,264 bytes. |
| Frame metadata | 65,536 bytes, excluding inline canonical object payload bytes. |
| Frames per run | 65,536. |
| Complete stored run bytes | 512 MiB. |

Preserve the metadata boundary honestly after removing the object table: subtract the actual
canonical payload byte length of each serialized inline object occurrence from the complete frame
size. Repeated inline occurrences count as serialized. Object references, Runtime fields, tags,
and frame headers remain metadata. One small private accounting function over the concrete commit
is sufficient; no reflection, object registry, deduplication framework, or lifecycle-size estimate.
Journal enforces full-frame limits; Runtime supplies payload-aware metadata accounting; Store
checks exact cumulative count/bytes atomically. Do not silently turn the old metadata ceiling into
a header-only ceiling or increase any limit to make a fixture fit.

Admission makes no promise that every future outcome fits. An actual result/decision may exceed a
limit after work occurred. Report the original when present, actual size cause, and unchanged head.
Do not truncate required facts, manufacture a smaller error record, or create replacement Effect
authority. If an externally accepted settlement cannot be recorded, the acknowledged pending
command remains authoritative for later reconciliation; do not claim durable settlement.

### 11.2 Prove the representation before owner fanout

At the start of the core cutover, use actual target types and an existing Runtime public entry
point to exercise a small complete path: admission, successful State/context advance, failed Read
committed before a handler error, restoration, and explicit resume. Use the baseline's available
operational cause. This proves lifecycle/custody integration; it does not claim that all upstream
causal gaps are already fixed.

Include checkpoint restart and one Effect pending/settlement/interpretation path before declaring
the core complete. Use real typed context/result objects, exact encoded frames, and existing Store
contracts. Do not write an independent model, generic benchmark framework, or throwaway parallel
snapshot implementation. The experiment becomes the production cutover and its consuming tests.

Measure encoded admission, success, failure, recovery, preparation, and settlement frames for the
shipping portfolio, anchored-call Read, and transaction fixtures. Include a legal multiple-active-
checkpoint case with distinct contexts and an identical-context control. Report canonical context,
checkpoint, and audit-fact bytes; repeated occurrences; metadata and complete frame bytes; and
actual history totals. Include actual oversize and cumulative-limit rejection with unchanged head.
Use existing fixtures or a temporary measurement harness, not a permanent measurement subsystem.

Also record the load rows/bytes for the same latest state after short and longer legal histories.
Normal loading must stay admission plus latest, independent of history length, apart from an
explicit candidate-sequence probe. This is an IO contract, not a promised latency ratio. Native
input decoding and complete-state size costs still need measurement; avoid full-history validation
benchmarks for a deleted feature.

Before broad owner migration, report the steps 1-3 production-code delta from `7f71beef`, including
replacement code and untracked additions, separately from tests/docs. Name removed public types,
callbacks, code paths, and remaining change sites. The core must have one continuation declaration,
one object representation, one restoration route, and no semantic-history scan. A smaller fold file
or passing local test is not evidence if its responsibilities survive elsewhere.

No net LOC forecast is established. If the core grows, explain the additions by retained guarantee
and identify actual compensating removals. If it needs duplicate models, an unplanned generic layer,
or rejects a shipping fixture, stop the core expansion and resolve that concrete finding before
starting another owner. This gate assesses the changed design; it does not authorize repeated
repository-wide diagnosis or discarding user work.

### 11.3 Handoff review evidence

The 2026-09-12 handoff review exercised a disposable Rust probe outside the production worktree,
using pinned rustc 1.96.0, Serde 1.0.228, serde_json 1.0.150, and the actual baseline canonicalizer,
IDs, and Values contracts. The probe's dependency checkout at 15829d89 has identical crates,
Cargo.toml, and Cargo.lock to 7f71beef. It adds serialization derives to the proposed concrete
declarations and tests a minimal native-cause carrier and erased async return; it implements no
Runtime transition model, Store, or alternate production design.

All ten tests passed under `nix develop -c cargo test --offline --manifest-path <probe>/Cargo.toml`:

- Every proposed Phase, OperationFacts, failure, and nested recovery-policy alternative round-trips
  through real canonical JSON with inline raw objects; duplicate/unknown object fields, unknown or
  multiple phase alternatives, and out-of-range target positions reject.
- The previous adjacent kind/data shape reproduces the RawValue deserialization failure described
  in section 6.5; the selected external shape round-trips.
- A declared MfmValue without an Error implementation retains its original through erasure and
  bounded projection failure. A reviewed native Error retains its exposed source links and fields.
- The original survives an erased async callback's lifetime and a panic in its immediately awaited
  encoding task because the async owner retains its Arc.

These are language, wire-shape, and ownership checks. They do not validate semantic transitions,
shipping values, full Object admission, throughput, production LOC, or the complete C1-C18 contract.
The temporary probe is review evidence, not a new repository dependency or a test framework to
import. Step 3 implements these regressions in the real consuming tests and supplies section 11.2's
integration evidence before owner fanout.

Baseline tracing also settled locally checkable commit/phase relations, the derived recovery total,
binding ownership, idempotent start, NotInserted reconciliation, immediate Store-error return,
error custody through Application, and exact public failure statuses. These choices are specified
above; an engineer should not need to invent another state, registry, recovery workflow, or error
transport to connect the sketches.

## 12. Ordered logical commits

The sequence is mandatory. Steps 1-2 are coherent baseline deletions. Step 3 is an inseparable
persistence/continuation/consumer cutover; preparatory unintegrated schemas are not completed slices.
Steps 4 onward close the remaining auditability inventory through the new route.

Before each commit, write a short checklist: observable change and public consumer; affected owner
files and consumer closure; exact deletions; required cause fields; and narrow verification command.
A new persisted schema must name its consuming stored field. Include production LOC and actual
removals in the result. Do not build a separate progress framework or seek approval for routine
implementation choices already settled here. New architectural conflicts follow AGENTS.md.

### Step 1: `remove prospective history capacity admission`

Cut over Program recovery bounds/authoring/wire, Runtime estimates/quotas, EVM/Portfolio bound
calculators, and their consumers. Preserve the existing fused lifecycle for this commit. Verify
actual object/frame/run rejection and semantic recovery limits. Gate: the deleted capacity symbols
have no production definitions or callers; shipping domains supply no replacement lifecycle bounds.

This precedes contextualization deletion because complete inputs can exceed old context-only frame
estimates. Do not repair estimates that the next step would remove.

### Step 2: `remove adapter contextualization and incident summaries`

Cut over Program Read/Effect traits and authoring, Runtime association/recovery, current Journal
adapter-failure payloads, shipping States, and App projections together. Preserve complete executed
input and intent/command/EffectId. Handlers receive Classification directly with derived context.
Delete context-only values, codecs, registrations, ABI fields, summary-only branches and callbacks.
Keep preparation/response-binding checks at their actual owners.

Use Program `lib.rs`, `authoring.rs`, and `recovery.rs`, Runtime assembly/recovery, EVM
`anchored_call/context.rs` and `transaction/stages.rs`, and their callers as the initial closure.
Verify a cold adapter failure and local mismatch rejection through the consuming Runtime/App path.
Gate: no contextualizer or IncidentSummary path remains, and retained behavior is covered.

### Step 3: `persist current run state and separate recovery commits`

This is one inseparable cutover commit, with the following internal execution order. Intermediate
work is not a completed foundation or an excuse to begin source-owner migrations:

1. Build section 11.2's admission/success/failed-Read/handler-failure/restore/resume path through the
   existing public Runtime entry point and memory Store. Integrate the canonical Values object,
   actual RunState/payload, opaque Journal envelope, admission/latest load, failure-before-policy
   append, and native original-error custody needed by that path together. Remove the replaced
   qualified/cache/ref-table/fold path from this consuming route; do not first build codecs or a
   standalone schema inventory. This working path precedes PostgreSQL and broad consumer fanout.
2. Complete checkpoint restart, terminal recovery, pending Effect, settlement-before-interpretation,
   and append-failure/collision paths using that same representation. Apply sections 6.5 and 8's
   current-state checks, grants, yield points, and typed binding ownership. Exercise native
   constructor reporting at a selected typed callback; the shipping Application proof completes
   in item 4 because Application depends on PostgreSQL. Add no persisted internal-error contract
   or dormant decoder-only migration.
3. Cut PostgreSQL and all Store consumers over to the already-exercised admission/latest plus
   optional probe contract, preserving atomic append authority. Update queries, baseline/fixtures,
   and SQLx metadata together. Run the managed checks and shipping-size/load measurements; do not
   reopen provider/signing capture to make the storage cutover compile.
4. Render Runtime/App/CLI/REST from the current continuation and causal invocation result. Update
   authoritative design, architecture, persistence/transport docs, and AGENTS ownership statements
   that still require a semantic fold, Journal history qualification, or complete-prefix loading.
   Prove C14 with one shipping checked constructor's distinguishable failures through its actual
   Runtime callback and Application report, reusing the route already exercised above.
5. Complete physical deletion of the old fold/accumulator replay, Journal lifecycle declarations,
   qualified/native caches, duplicate contract refs, object-table resolution, full-prefix load,
   superseded APIs/tests/docs, and every replaced producer/consumer in this same commit. No earlier
   intermediate implementation survives as a second current design.

Gate: C1-C14, C16, and C17 pass for the core boundaries; section 11.2's measurements and deletion delta are
reviewable, and the production workspace has one current design. No PostgreSQL capture overhaul,
signer migration, or full Values/derive family rewrite is a prerequisite. The core may use baseline
operational payloads whose upstream first-loss gaps remain explicitly assigned to step 4. C15's
remaining owner coverage belongs to step 4; C18's final integration evidence belongs to step 5.
Existing baseline checks for steps 1-2 run during those steps; their new lifecycle cases complete
here. This assignment must not turn the whole owner inventory into a prerequisite again.

### Step 4: close source-loss owners in coherent commits

Follow section 10: PostgreSQL ports/gates, signing/keystore ownership, nested EVM operational
wrappers, and application/transports, using the baseline inventory to avoid redundant work.
Select one connected owner/consumer slice per commit; its output is a delivered cause, not an
unused error type. Reuse baseline RPC capture and the single native decoder/reporting route.

For each slice, prove distinguishable nested causes at the consumer, secret exclusion, accurate
disposition, and cold preservation only for actual domain/operational results. Delete the owner's
unit/text-only sink and any superseded repeated schema/conversion. Finish that closure before
opening the next family. Do not preserve compatibility constructors or leave two current APIs.

### Step 5: `complete causal auditability integration`

Reconcile the first-loss inventory, retained guarantees, and acceptance evidence. Complete any
remaining consuming scenario and documentation in its owning slice; this step does not postpone
required cutover deletions. Report total production LOC against `7f71beef`, separate tests/docs and
untracked work, and all explicit unavailable/withheld/bounded upstream evidence.

Use [build and verification](docs/build-and-verification.md) and the executable Nix task graph.
All Cargo/Rust tooling runs in the default Nix shell. Begin with affected packages and dependent
consumers; use managed PostgreSQL/SQLx checks when queries change and managed Effect/client scenarios
when those boundaries change. Run one final `nix run .#ci` on the exact implementation candidate.
Do not repeatedly run broad gates at every commit or count local successes as final integration.

## 13. Acceptance evidence

This table replaces the previous A1-A34 contract, including tests for now-removed durable system
faults and historical semantic validation. Preserve retained behavior in consuming tests rather
than retaining fixtures for obsolete machinery.

| ID | Observable acceptance |
| --- | --- |
| C1 | Fresh and restored runs use the same RunState/object types and produce equivalent continuation/output through a real Runtime entry point. Every current enum variant round-trips through canonical JSON with inline objects. No qualified cache, alternate wire state, object resolver, or post-encode native round trip. |
| C2 | Admission/latest loading uses one consistent snapshot. Increasing history length does not increase normal rows loaded. A missing required row, inconsistent head, bad frame/object hash, wrong Program identity, or invalid current phase/slot contract rejects before action. Repeating start with identical Program/input returns the checked observation without driving; different admission returns AdmissionConflict. |
| C3 | Current-state validation rejects the impossible facts/phase relations in section 6.5, invalid checkpoint/barrier relationships, out-of-range counters and checked-sum overflow. The run total has no independently stored counter. No historical successor construction or claim of detecting otherwise locally valid historical counter/checkpoint substitution remains. |
| C4 | Success carries complete context to the next State. Restart selects an eligible declaration checkpoint's stored input, creates a fresh visit, retains current usage/Effect restrictions, and never consults old operation frames. |
| C5 | A State-domain failure and an adapter operational failure each commit original/input/request/evidence as applicable before classification, handler, or mapping. Their reviewed causes survive cold inspection. |
| C6 | Handler/map failure reports its native cause with no fault append; the original remains AwaitingRecovery. Explicit resume reruns unfinished recovery evaluation. An authorized Retry/Restart charges the failing declaration and run once, then yields. Stop/denial spends no grant. Pending Effect Retry preserves visit/command identity. |
| C7 | Effect preparation commits command/EffectId before adapter entry. Pending/error/Stop behavior preserves that same unresolved command and existing reconciliation restrictions; no replacement authority or capacity-only retry quota. |
| C8 | Accepted Effect settlement commits before interpretation. Injected interpreter failure appends no fault; cold resume interprets retained evidence without entering the adapter for that phase. Uncommitted settlement makes no durability claim. |
| C9 | Internal State/preparation/adapter/task/constructor failures retain reviewed causes in the invocation report and append no fault. Explicit resume retries permitted unfinished work; it does not create an automatic retry loop. |
| C10 | Local preflight mismatch performs no provider call/append; post-response binding rejection reports the true stage without appending unauthenticated evidence or falsely claiming no IO occurred. |
| C11 | Encoding/limit/Store failure preserves available original error plus separate recording cause and exact candidate when available, including across the erased callback, failed encoding task, and Application's AppendIndeterminate conversion. Known insertion, this attempt's noninsertion, and Indeterminate are distinct. Store errors return immediately without a probe; no fallback record or speculative adoption. |
| C12 | The NotInserted probe covers exact matching candidate, conflicting occupied sequence, candidate followed by later commits, absent candidate, and failed reload/projection. It retains the physical finding independently of the latest view and never drives further work. A candidate-absence snapshot cannot resolve a still-in-flight COMMIT; the Store-error invocation returns with its ambiguity/custody. Fresh resume may use current phase authority without claiming it resolved an unavailable prior candidate. |
| C13 | Atomic append, immutable earlier frames, exact-head conflict, checked cumulative bounds, consistent loads, and PostgreSQL acknowledgement ambiguity hold through Store public boundaries. |
| C14 | Two distinguishable native constructor failures reach the consuming Runtime/Application report with owner variants, child causes, locations and reviewed facts. No native failure is flattened into unit/custom text, and no decoder-error persisted identity is needed. |
| C15 | Each owner slice preserves available reviewed layers/operation/fields, unchanged intrinsic classification where applicable, and correct internal-versus-operational routing. Diagnostic bounds, omissions, opaque/unavailable evidence, and secret exclusion are tested. |
| C16 | Actual oversize/frame/run rejection leaves the acknowledged head unchanged. Cover a failure that committed but whose recovery cannot fit, and an externally settled Effect whose settlement cannot fit. No future-capacity promise remains. |
| C17 | App/CLI/REST show the retained current result or reviewed causal invocation report with section 10.1's exact statuses and accurate acknowledgement/delivery status, including known insertion followed by projection failure. They do not reconstruct history, generate internal-fault records, or expose raw native source input. |
| C18 | Steps 1-3 show actual baseline removals and replacement costs before owner fanout. Shipping frame sizes and current-state load IO are measured. Required final verification passes on the exact complete implementation. |

No test calls a historical reconstruction implementation merely to compare it with the new one.
Use baseline observable behavior where it remains contractual, concrete expected transitions, and
hostile physical/current-state inputs. Update old-format fixtures under the single-current-design
policy; do not retain a reader to keep obsolete fixtures passing.

## 14. Deletion ledger and completion

| Actual baseline responsibility | Replacement or deletion |
| --- | --- |
| FoldState and history-to-accumulator reconstruction | Delete. Continuation data lives directly in RunState; execution rules remain once in the live driver. Historical semantic checking is removed, not moved. |
| QualifiedValue, ColdQualifier, native Any cache, redundant per-object contract ref | One canonical Values object, native decode at use, Program association at its owner. |
| Journal lifecycle declarations, reference-only object projection/table/resolver | Runtime-owned inline payload and opaque Journal envelope. No generic live/wire state family. |
| Complete-prefix normal Store load and full-prefix reconstruction consumers | Admission/latest read plus optional exact-candidate sequence probe. Store's atomic head/count/byte maintenance remains. |
| Fused outcome/recovery branches | Original failure commit, then normal recovery decision commit. Accepted Effect settlement commits before interpretation. |
| adapter_context and context-only types/codecs/ABI/report plumbing | Existing complete input and intent/command facts with owner checks. |
| IncidentSummary and summary-only callbacks/conversions | Direct Classification and derived recovery context. |
| Prospective bounds, domain calculators, reservation admission, capacity-only pending quota | Actual limits and semantic recovery counters only. |
| Real lossy unit/text-only owner and invocation conversions | Native reviewed causal reporting or actual operational/domain payload data. Count the added guarantee honestly. |

The following proposed/discarded machinery earns **zero baseline deletion credit**: persisted
internal-result ABI and fault schemas, decoder-error identity/registry, generic partial-progress
custody, intermediate-map-output timeline, parallel snapshot model, and independent audit store.
Preventing those additions is a scope reduction, not deletion of baseline production code.

Necessary additions are complete-state serialization, split outcome/recovery and settlement phases,
current-state validation, native error reporting, and missing reviewed owner cause data. Report
their cost separately from deleted code. The goal is fewer concepts, paths, APIs, duplicated checks,
and change sites; production LOC is evidence, not a quota or permission to weaken retained behavior.

Completion requires every named consumer to use the current design, the first-loss inventory and
C1-C18 to be reconciled, all required verification to pass, and the final production delta to include
all changes. No unintegrated decoder foundation, renamed fold, ignored untracked files, or omitted
cause can be described as completion. The broad RFC is complete only after its owner slices finish.

This RFC-only change performs no production Rust implementation and discards no engineer work.
Its verification is link/command/contract review, `git diff --check`, and section 11.3's disposable
interface probe. Full Rust CI remains an implementation requirement.

## 15. Material uncertainties

No known behavioral or ownership choice remains open after the user's answers and the handoff
review. The following are implementation evidence requirements, not permission for an engineer
to choose another architecture.

| Assumption | Why uncertain | Consequence if wrong | Validation and response |
| --- | --- | --- | --- |
| Inline complete state and audit facts fit intended workloads under existing ceilings | Distinct checkpoints and repeated object occurrences may cost more bytes than the old object table/event representation | A shipping outcome or accepted settlement could be unrecordable | Measure the concrete core and shipping fixtures in section 11.2 before owner fanout. Resolve an actual size failure explicitly; do not restore a resolver or increase limits preemptively. |
| Deleting representations/history work yields a simpler integrated core | Actual serialization, local validation, native decoding, and consumer replacement LOC are not implemented | A new layer could hide the old complexity; per-use decoding may have unacceptable cost | Inspect steps 1-3 production delta and real context/load measurements. Reject duplicate models and explain every increase before continuing. No numerical LOC or speed promise. |
| One native error/decoder route suffices for the reached owner families | Concrete checked constructors and erased callback ownership differ | Companion types or reporting code could multiply again | Prove one real constructor's distinct causes through the consuming callback and Application in step 3. Extend only an actually reached family per later owner slice. |
| Client owners expose enough reviewed evidence for the required chain | Libraries can hide retries or return opaque causes | Some evidence cannot be recovered | Inject distinct nested causes at each owner, identify the first unavailable layer, and retain explicit omissions; never invent hidden facts. |

The changed trust contract is explicit, not an unresolved assumption: normal loading does not
verify older frame links or historical semantic evolution. It checks the loaded admission/current
state and relies on correct live progression and append-only storage for the past. Dormant native
constructors are checked at typed use; structural/schema validation still covers their stored
objects on restoration. Internal reports are not promised durable in run history.
