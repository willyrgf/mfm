# RFC: commit complete runtime state and preserve causal errors

Status: target design approved for engineer handoff; implementation is not complete. Implement
from current HEAD. At handoff, the working tree is clean and `15829d89..HEAD` changes only this RFC.
Use `15829d89` (`preserve rpc and evm error provenance`) as the production-code review baseline;
it already contains the RPC/EVM capture cutover. No rollback is needed. The stopped expansion
was removed from the worktree and archived separately for consultation only. Its test counts,
descriptor sizes, and completion claims are not acceptance evidence for this target.

The [adapter error audit](docs/adapter-error-audit.md) remains the first-loss inventory.
[Design](docs/design.md), [architecture](docs/architecture.md), [code quality](docs/code-quality.md),
and [AGENTS.md](AGENTS.md) govern the repository. This RFC is a proposed replacement contract;
implementation must update the authoritative documents, producers, consumers, and tests in the
same coherent cutover. Existing authoritative documents describe the baseline implementation;
update their superseded contracts during implementation. Remaining validation items are explicit
in section 18.

## 1. Summary

Persist the complete Runtime continuation state and the completed operation's audit facts at each
commit. Runtime uses the same logical state in memory and persistence; there is no second snapshot
model beside an authoritative event-replay model. Repeated context and checkpoint data are accepted
for self-contained auditability and direct restoration. Secrets and executable service objects
remain outside the persisted representation.

Remove FoldState and the accumulating history-to-state reconstruction path. Keep exactly one pure
Runtime transition implementation, used to construct live candidate states and to verify adjacent
stored states. Historical checking remains mandatory: complete snapshots eliminate reconstruction
replay, not the requirement to reject illegal recorded transitions.

```text
live execution:
    acknowledged state + completed operation facts
        -> Runtime transition -> candidate state
        -> validate and encode commit -> Store -> PostgreSQL
        -> known insertion -> adopt candidate and dispatch its permitted next work

loading:
    Store complete prefix -> Journal byte/chain qualification
        -> qualify genesis and verify each adjacent stored state transition
        -> restore the final stored state directly -> inspect or resume
```

Execution outcome commits before classification, handler invocation, or root mapping. Recovery
commits the returned request, authorized decision or fault, and resulting Runtime state before
any recovery action. Pure candidate construction may precede append; candidate adoption and
external work require known insertion. Store stays mechanical and Journal owns the exact wire,
not another execution layer.

Preserve the complete available reviewed causal chain. Classification, recovery requests, and
public rendering are projections, not replacements for the original error. If construction,
qualification, or encoding fails, return the available original and recording cause without a
substitute record. Store rejection and ambiguous acknowledgement also retain their exact status.
No generic snapshot registry, commit facade, partial-capture framework, decoder-error registry,
independent audit store, or cross-frame object resolver is introduced.

## 2. Decisions established in discussion

| Question | Selected contract |
| --- | --- |
| What is persisted at each commit? | Complete Runtime continuation state plus the completed operation's input/context, result, evidence, and recovery facts as applicable. |
| Must the executable State object be stored? | No. Runtime associates implementations; state/context here means their serializable execution and continuation data. |
| Is repeated context/checkpoint data acceptable? | Yes. Every commit carries the data needed for direct restoration and its own audit explanation. |
| Is persistence a separate snapshot cache? | No. The persisted Runtime state is the current representation, not a second independently maintained model. |
| Is a fold retained? | No accumulating reconstruction fold. One Runtime transition implementation also checks adjacent stored states. |
| When is history checked? | On cold inspection/resume and any loaded competing/reconciled history, before exposing qualified state or executing further work. |
| Is every append followed by a full history check? | No. Live construction uses the already acknowledged predecessor; known insertion permits adoption of that checked candidate. |
| Is only the latest row trusted on load? | No. Store still loads a complete prefix and Runtime independently checks historical transitions. |
| What is a restart target? | An eligible active declaration checkpoint with its persisted input; not an arbitrary historical visit. |
| Does restart roll back the machine? | No. It appends a new visit and preserves current history, usage, and Effect restrictions. |
| What does the handler do? | Recommends Stop, Retry, or Restart. Runtime authorizes, constructs the candidate transition, commits, and only then dispatches. |
| How are internal outcomes treated? | Valid typed internal outcomes commit with their actual phase and no handler permission. Preflight mismatch causes no provider call/append; post-response local binding rejection also forbids append without claiming IO did not occur. |
| What do codecs enforce? | Exact type/schema, canonical value admission, and faithful wire representation. Runtime owns transition legitimacy. |
| Are decoder errors Program values? | No. Checked decoding retains native constructor causes; no decoder-error codec or identity is registered. |
| Does admission reserve future history capacity? | No. Check actual values, frames, and accumulated history against existing ceilings; do not estimate or reserve a complete execution lifecycle. |
| Is Clone or codec::from required? | No. These are ownership/projection implementation choices, not replacements for complete persistence or candidate validation. |
| What can resume continue? | Only unfinished work described by acknowledged history, including unresolved Effect reconciliation; committed internal/recovery/task faults are not retried. |
| Is adapter_context retained? | No. Delete the callback and all context-only representations and associations; retain facts in their existing owners and preserve consistency checks. |
| What if recording fails? | Return available original/candidate and causal recording result. No substitute record, recursive append, or speculative action. |
| Is bounded/redacted capture lossless? | No. Account for withheld, opaque, unavailable, and bound-limited evidence. |

## 3. Baseline facts and actual gaps

At `15829d89`, `JournalRecord::RunAdmitted` retains Program and initial context.
`FoldState::succeed` passes each retained output to the next State; `enter` retains active checkpoint
inputs; `recover` restores the selected input while charging current usage and creating a fresh
visit. Transaction States use `ContextSlot::replace` to preserve unchanged context siblings.
These mechanisms define the transition rules to retain, but reconstruction from accumulated events
is superseded by directly persisted Runtime state. Baseline FoldState contains cursor, checkpoint
inputs, per-State usage, global decisions, and Effect barrier. Those are continuation facts to
persist, not incidental bookkeeping that can be omitted when removing the fold.

The causal/lifecycle gaps are:

1. `engine.rs::conclude` and `decide` perform policy/root mapping before outcome append; their
   failure can prevent the original cause from becoming durable.
2. Unit preparation/internal errors and lossy boundary conversions discard reviewed causes.
3. Effect interpretation failure can leave only preparation committed, without accepted evidence.
4. Fused outcome/decision records cannot represent failure acknowledged with recovery pending.
5. Handler/map failures lack their own durable recovery result.

The RPC/EVM baseline already captures provider causes. PostgreSQL, signer, custody, application,
transport, and adjacent Runtime boundaries still require the inventory's source-preserving
cutovers. No causal representation can recover facts discarded by an earlier owner.

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
facts remain in the owner's exact error type (sections 5.2 and 7.3).

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
bounded by their exact value and complete outcome/recovery frame contracts.

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

## 6. Complete commits and direct restoration

### 6.1 One persisted Runtime state

The Runtime state used for continuation is itself persisted. Its concrete phase alternatives carry
only the fields valid for that phase, but a commit must not rely on replay to recover missing
continuation data. The name RuntimeState below describes that one representation, not a mandated
generic framework or a second DTO with independent conversion rules.

| Runtime state fact | Why it must be present without reconstruction replay |
| --- | --- |
| Current State position, visit, and phase | Select the permitted next operation. |
| Complete current input/context | Execute or interpret directly. |
| Active checkpoint positions and their complete input/contexts | Restore a selected checkpoint without rebuilding earlier outputs. |
| Per-State retry/restart usage and global decision usage | Preserve current allowances across restart. |
| Effect barrier/restrictions | Preserve irreversible execution constraints. |
| Pending command and EffectId | Reconcile the exact acknowledged operation. |
| Accepted settlement evidence while interpretation remains | Continue interpretation without another Effect adapter call. |
| Unresolved original failure and relevant recovery decision/fault | Resume the correct phase without recreating completed execution or policy work. |
| Terminal output or root failure | Observe terminal state directly. |

Program and complete initial context remain in admission. Every commit is bound to that admitted
Program through the checked run/frame identity; it need not copy immutable Program policy, limits,
and implementation descriptors. Load admission once and associate its exact implementations.
A single final row without admission is not the loading contract.

Run identity, sequence, predecessor, and head remain checked envelope facts, not conflicting copies
inside Runtime state. Concrete persisted values keep exact contracts and existing frame-local
object closure. Equal objects may be shared within a frame under that existing mechanism; sharing
is not required to avoid repeating data across commits. Add no cross-frame object resolver.

Recovery context is a borrowed view of the one Runtime state and admitted Program. Do not maintain
another independently supplied recovery snapshot. Persist usage and checkpoint data in Runtime
state; derive remaining allowances and eligibility from those fields and the static Program.

### 6.2 Audit facts accompany the resulting state

The next continuation may have different input from the operation just completed. Preserve that
operation's facts so the commit explains itself without searching earlier outcome records. These
are distinct semantic roles, not competing copies of one authority. Runtime constructs them from
the retained execution and checks their relationship through its single transition implementation.

| Commit boundary | Completed operation's audit facts, alongside complete resulting Runtime state |
| --- | --- |
| Admission | Program and complete initial context; initial Runtime state must agree with them. |
| Pure success/failure | Executed position, complete input/context, and output or original typed domain/internal error. |
| Read preparation failure | Position, input/context, preparation stage, and typed internal cause; no invented intent. |
| Read adapter failure | Position, input/context, exact intent, and original typed error. |
| Read evidence accepted | Position, input/context, intent, accepted evidence, and interpretation result including internal failure. |
| Effect preparation | Position, input/context, exact command, and derived EffectId, before adapter entry. |
| Pending Effect failure | Input/context, command/EffectId, original cause, and stage/phase. |
| Accepted Effect settlement | Input/context, command/EffectId, accepted evidence, and interpretation result including internal failure. |
| Recovery evaluation | Original failure and its originating execution context/facts, connection to that outcome, completed classification/request, authorization, and decision or typed fault. |
| Terminal root mapping | Original recovery facts and mapped root, or failing mapper step with its input and typed cause. |

Required input/output fields cannot be skipped to reduce storage. Preserve the unresolved original
and its context in recovery commits even when they were already stored. A link to the original
outcome identifies provenance; it is not a substitute for the committed causal data. Where audit
and continuation refer to the same value, use their actual shared owner/object rather than accept
independently supplied mismatching copies. The complete commit appends atomically.

For example, after A succeeds with C1, the persisted continuation may point to B with input C1,
while the same commit's audit facts retain A's input C0 and its result C1. After a restart to B,
Runtime uses the checkpoint input already present in its state, preserves current counters and
Effect barriers, and commits the new visit plus the recovery facts. It never restores budgets or
history to their earlier values. Active declaration-checkpoint semantics remain unchanged.

Executable State objects, callbacks, adapters, connections, signer handles, credentials, and
unneeded transient locals remain excluded. This complete-state contract does not revive the
stopped implementation's generic execution wrappers, recursive error schemas, or partial recorder.

### 6.3 Delete contextualization completely, retain its checks

The repository-wide review inspected every shipping `adapter_context` definition: the balance Read
macro, anchored Read, and nonce-reservation, transaction-preparation, and transaction-execution
Effects. All ignore the operational-error argument and derive their facts from existing input,
intent, or command. None supplies a unique production diagnostic fact.

Delete `adapter_context`, the associated AdapterContext types, context-only value types,
codecs, ABI entries, registrations, and redundant reporting/transport plumbing. Trace and update
all consumers and test fixtures in the same cutover. Retain original typed errors and expose the
required facts from their existing authoritative input/intent/command owners.

Preserve the callbacks' intent/command equality checks and checked ordinal/intent invariants at
existing preparation, input-validation, and binding boundaries. Do not replace the callback with
another fallible contextualizer or registry under a different name. This deletion does not require
reexecuting completed preparation on historical inspection; retain the independent checks in
section 8, including final pending-command verification.

## 7. Outcome, recovery, and transition ownership

### 7.1 Handler recommendations are not transitions

The handler receives Classification directly and a recovery-context view of Runtime state. It recommends
the existing Stop, RetryState, or Restart(target) request. Runtime alone checks allowances,
eligibility, and Effect barriers. Existing Operation defaults, occurrence overrides, intrinsic
classification semantics, and root-map selection remain intact.

Remove `IncidentSummary` and summary-only source plumbing. The original typed result already owns
its origin and causal evidence. Do not add a second classifier, provider-message retry rule, or
policy-facing error bag. Richer timeout evidence does not establish mutation nonacceptance.
Duplicate-safe Reads and transaction Effects keep their existing distinct classification semantics.

### 7.2 Split records, reuse the existing append path

Remove decisions and terminal root mapping from execution conclusions. A successful outcome advances
in its one append. A domain/operational failure enters AwaitingRecovery after its outcome commits.
The driver then performs classification, handler invocation, authorization, and any terminal root
mapping; the resulting decision or fault uses the same append path. Only an acknowledged decision
can release execution under a new visit, restart, or stop. A competing/ambiguous decision cannot
release an action.

Replace the outcome-replay representation with complete Runtime state and concrete audit facts.
Remove FoldState and history accumulation; retain its required rules in one Runtime transition
implementation. Do not keep both replay and snapshot restoration or introduce a generic snapshot
registry. The existing State outcome API is not replaced merely to standardize a Rust spelling
such as nested Result. Wire changes follow the required state and audit alternatives.

Keep `prepare_append` / `finish_append` as the common mechanism, fixing custody and acknowledgement
handling there as needed. Pure transition construction and encoding must not mutate the caller's
acknowledged state. Adopt and dispatch the candidate only after known insertion. Do not add a
generic commit facade over this path or a second transition implementation.

### 7.3 Internal outcomes and Effects

Replace unit State preparation/execution and adapter internal errors with owner-typed causes.
Distinct operations and reviewed expected/observed facts remain distinct despite an equal Internal
disposition. Framework qualification, Journal, and task errors keep their own native owners; an
owner's domain invariant must not become a generic framework code.

A valid internal execution outcome at an admitted cursor is committed and stops execution progress.
Explicit resume returns that recorded fault without rerunning execution, classification, or a
recovery handler; it does not manufacture a mapped terminal domain failure. As required by
AGENTS.md, preflight local association mismatches reject before provider entry and never append. A local evidence-binding
rejection after a provider response also authorizes no append: retain its reviewed cause and
available reviewed observation in the invocation, without claiming the provider was never called.
These exceptions cannot discard mismatch facts or fabricate authenticated external integrity evidence.

Preparation acknowledges the exact command and EffectId before Effect adapter entry. Pending is
not an error and returns the unchanged acknowledged pending view without an extra record. Pending
failures retain command authority. Accepted settlement evidence belongs in the outcome even if
interpretation returns an internal error. Once that fault and evidence commit, resume returns the
recorded fault without rerunning interpretation or reentering the Effect adapter.

Settlement observed only in memory is not durable settlement. A crash or failed encoding before
its record commits leaves the preceding pending command authoritative. Preserve existing
reconciliation/ambiguity semantics; never claim every physical attempt was recorded.

## 8. Historical checking and interruption workflow

### 8.1 One transition implementation, two uses

Use one pure Runtime operation to determine the successor from the acknowledged predecessor and
completed execution/recovery facts:

```text
transition(previous Runtime state, recorded operation facts) -> next Runtime state
```

It owns position/visit changes, checkpoint updates, recovery authorization/usage, phase changes,
and preservation of Effect authority. It rejects audit input/original/command copies inconsistent
with the predecessor. It consumes recorded results; it does not execute State
business logic, classifiers, handlers, root maps, or providers. Live execution produces those
facts before invoking the transition operation. Expected denial and actual returned request remain
distinct recorded facts that the transition checks against the preceding state and Program.

During live execution:

1. Retain the acknowledged state and the operation's returned facts.
2. Use the transition implementation to construct a separate candidate state.
3. Encode and qualify the exact candidate bytes through checked decoding, value/evidence checks,
   and the existing frame contracts. Verify they represent the constructed successor and matching
   audit facts, including input/original/command copies that agree with the acknowledged
   predecessor. Reuse the same value and transition checks; do not validate only the native value
   and assume its serializer preserved the contract.
4. Append at the acknowledged head through Store.
5. On known insertion, adopt that qualified candidate and dispatch its permitted work. On rejection or
   ambiguity, retain custody/status and reconcile without speculative adoption.

No full-history reread/check is required after each known inserted live append. The predecessor is
already acknowledged and qualified. Snapshot construction does not weaken existing candidate value
checks or grant authority before Store acknowledgement.

### 8.2 Loading checks stored transitions, then restores the stored state

Store still returns one complete prefix. Before exposing a qualified observation or entering work:

1. Journal qualifies canonical frames, sequence, predecessor links, hashes, and exact local objects.
2. Runtime associates the admitted Program and qualifies the stored values. Construct the expected
   initial state from Program/initial context and compare it with the persisted genesis state.
3. For each adjacent pair, use the **stored predecessor** and the successor commit's recorded audit
   facts as input to the same transition implementation. Compare the resulting state's exact
   canonical representation with the **stored successor**. Reject mismatches with reviewed causes.
4. Preserve independent value/evidence checks and final pending-command verification.
5. Restore the final **stored** Runtime state directly, then expose its view or resume its phase.

```text
expected S2 = transition(stored S1, facts recorded with S2)
require canonical(expected S2) == canonical(stored S2)

expected S3 = transition(stored S2, facts recorded with S3)
require canonical(expected S3) == canonical(stored S3)

restore stored S3
```

Each check starts from its persisted predecessor, not the calculated successor of the prior check.
The temporary expected state is verification evidence, not an accumulating reconstruction model.
Do not implement another snapshot-transition validator with a second copy of the rules.

A valid hash/schema does not establish transition legitimacy. For example, if stored S1 has one
retry used and the recorded action is Retry, a stored S2 claiming zero retries is well-typed but
invalid: the transition requires two. Apply the same principle to checkpoint input substitution,
illegal visits, removed Effect barriers, and a settled Effect becoming transmissible again.
Preserve rejection coverage such as the baseline tests
`cold_fold_validates_pending_failure_position_and_stop_reasons` and
`retained_effect_facts_are_validated_without_adapter_io`; their fold names are not a reason to
keep the old implementation.

### 8.3 When historical checking runs

Run full-prefix qualification and adjacent-state checks whenever persisted history is loaded for
inspection, resume, a competing admission/append, or reconciliation of an uncertain append. Complete
the checks before returning that history as qualified or performing recovery/provider work.
If loading or verification fails, retain the rejected candidate/original and load/verification
cause without inventing a winning observation. Reconciliation must match the actual candidate
before making a known-insertion claim; loading some valid head alone is insufficient.

This still scans historical transitions. It is not constant-time or latest-row-only loading.
The contract does not trust a last snapshot merely because Runtime should have validated its writer.
No signatures, trusted checkpoint service, or database-side transition logic are introduced to
replace independent history checks. Store's complete-prefix contract remains unchanged.

Cold checking never executes classifiers, handlers, root maps, or completed State interpretation.
Retain pure evidence-binding checks and the baseline's final pending-Effect preparation verification
(formerly `FoldState::validate_pending`) as ordinary Runtime checks after deleting the fold.
Runtime association must match exact selected contracts. These checks establish legal consistency,
not proof that external events occurred or recomputation of completed business results.

### 8.4 Last acknowledged state and permitted continuation

| Last acknowledged boundary | Facts restored from the stored state/commit | Permitted next work |
| --- | --- | --- |
| Before outcome | Runnable input or acknowledged pending command | Only phase-permitted execution; an uncommitted physical attempt may have occurred. |
| Success outcome | Output and next position, or terminal success | Next State only. |
| Domain/operational failure | Original error, input, phase, awaiting recovery | Evaluate unresolved recovery without rerunning the failed State/provider. |
| Recovery decision other than unresolved pending-Effect Stop | Decision, updated counters, and resulting continuation | Follow committed continuation; terminal Stop performs no further work. Do not ask the handler to replace the decision. |
| Operational Stop with an unresolved pending Effect | Original failure, Stop decision, retained command and EffectId | End this invocation. Explicit resume may reconcile that same command; Stop does not settle or cancel it, authorize a replacement, or release an automatic retry. |
| Recovery fault, including a committed task fault | Original outcome/context and actual evaluation fault | Return the recorded fault; no classification, handler, or mapper reevaluation. |
| Internal outcome | Cause, context, and actual phase | Return the recorded fault; no State/preparation/adapter retry or handler. |
| Internal interpretation outcome with committed settlement | Input, command/EffectId, accepted evidence, and fault | Return the recorded fault; no interpretation or Effect adapter reentry. |
| Construction/qualification/encoding failure | No newly acknowledged state | Return available original plus recording cause; no substitute record. |
| Ambiguous append | Last acknowledged state and uncertain candidate | Validate reconciled history and candidate acknowledgement before dispatch. |

### 8.5 Resume continues unfinished work, not committed faults

Resume has no general internal-failure retry or implementation-repair mode. Admission and a
successful transition may leave a State runnable; an original domain/operational failure may leave
recovery awaiting evaluation; an acknowledged decision may leave its selected continuation ready.
Resume follows those existing phases. Terminal success, terminal domain failure, and committed
internal/recovery faults perform no further work and append no repeated observation record.

An interruption with no committed result leaves the last acknowledged continuation authoritative.
A callback may therefore run again if its result never committed; this is not evidence that it
never ran. Reconcile ambiguous acknowledgement before deciding which continuation is current.
If a task failure is itself successfully committed as a fault, resume returns that fault even when
no callback result was returned. Do not fabricate a returned result, add a task-retry phase, or
persist generic partial progress to make that fault resumable. Unrecorded cancellation/process
failure makes no claim to an audit record and follows the existing acknowledged-state rule.

An unresolved pending Effect remains a special authority obligation: Pending and an operational
recovery Stop end the invocation without extinguishing the command. Explicit resume may reconcile
that same command and EffectId under the existing protocol. It does not authorize a replacement
command or an automatic retry loop. This operational Stop is distinct from a committed internal
or recovery fault, which blocks execution progress without asserting that an external command was
cancelled or settled. Accepted settlement committed with an interpretation fault is never replayed.

The baseline's resume-after-internal-callback-error behavior is deliberately replaced. Update its
head-preservation/retry tests to assert durable fault observation and no callback rerun, while
preserving interruption, ambiguity, and unresolved pending-command tests. Fixing an implementation
and continuing its faulted run is outside this contract.

## 9. Checked values, codecs, and native decoder errors

Persistence selection and value validation are separate. Existing concrete record types declare
which facts are retained. Canonical encoding, exact schema/reference checks, checked owner
construction, and the single Runtime transition implementation validate representation and legal
succession before append.
Keep these candidate checks and cold qualification. Type declarations cannot by themselves prove
that an arbitrary custom serializer retained every meaningful field; consuming round-trip tests
must establish the reviewed value contract. Do not silently weaken validation to avoid an error path.

`DecodePersisted` separates raw wire deserialization from checked owner construction where Serde
custom text would discard a constructor cause. Its associated Error is native. It has no MfmValue,
persisted identity, or registered decoder-error requirement. Native derive companions may preserve
field/container locations and child causes, but do not derive Program-value or recursive error
codec machinery for those companions. Hex, quantity, epoch, diagnostics, Values, and Journal
constructor errors do not become independent Program contracts merely because decoding can fail.

Preserve exact reviewed constructor variants, nested causes, operations, locations, and available
expected/observed fields across heterogeneous Runtime boundaries. Native error retention is not
permission to keep rejected secret input or raw formatted errors. Shared DiagnosticEvidence cannot
replace known owner-specific facts with an opaque marker. Actual State/adapter/handler/map result
errors still need their declared retained contracts; constructor errors are not new result roles.

| Decode boundary | Failure contract |
| --- | --- |
| Program/parameter admission or association | Return native reviewed cause; do not enter execution. |
| Loaded history | Fail qualification with the native cause; append nothing through an untrusted head. |
| Proposed candidate | Return native cause with available original/candidate; append nothing. |
| Live owned copy, if retained by the implementation | Preserve source operation and original custody; use the same no-substitute recording rule, not a fabricated InvalidHistory claim. |

Decoding to obtain an owned callback argument is not persistence validation. Native cloning at a
consuming State/map input may remove that copy round trip, but is not required by this RFC. Do not
add a global MfmValue: Clone bound, clone-codec registry, or borrowing rewrite. Any narrow ownership
change must prove immutable value semantics and preserve retained originals; it must not remove
candidate/transition validation. A separate codec::from API or qualified-record layer is not
mandated; persisted Runtime state is the one continuation representation.

## 10. Recording failures and concrete recovery faults

### 10.1 End the failed recording attempt

Retain native originals outside fallible encoding/callback tasks, borrowing or sharing immutable
ownership where appropriate. Immediately await unavoidable pure blocking work; IO and append
authority remain outside that task. If a callback never returned a result, do not claim a native
result survived. Preserve the available input, originating operation, and reviewed task cause.
Never inspect or format arbitrary panic payloads.

If record construction, qualification, or encoding fails, return the available primary and cause.
Do not construct CaptureFailed, UnretainedOwnerEvidence, or another partial substitute to append.
The same stop rule applies when a valid internal outcome or recovery fault cannot be recorded.
It does not prevent committing the ordinary typed internal outcome returned by execution.

| Recording result | Required custody and behavior |
| --- | --- |
| Inserted | Adopt the checked candidate and acknowledge its actual head before dispatch. If subsequent local adoption fails, retain known insertion with that cause. |
| No candidate produced | Retain available native original and construction/qualification/task cause; no insertion claim. |
| Definite append failure | Retain primary, candidate, and complete Store cause; identify definite noninsertion. |
| Ambiguous acknowledgement | Retain those facts with Indeterminate; assert neither insertion nor noninsertion. |
| Exact-head conflict | Preserve this caller's rejected candidate/original separately from the winning observation. No provider reentry, silent rebase, or loss of the losing cause. |
| Winner cannot be loaded/qualified | Retain rejected candidate plus reload/qualification cause and last acknowledged observation; do not invent a winning view. |

Use the existing invocation boundary to distinguish these outcomes without correlated optional
fields, recursive RuntimeError audit trees, or a general public capture framework. Retaining an
error in memory does not claim it was durably audited. A Store cannot persist its own failed append
through that same unavailable Store.

### 10.2 Recording example

Suppose a Read returns a reviewed RPC timeout. Runtime retains the native outcome while encoding
and candidate validation run. The following is control-flow pseudocode for the existing append
path, not a new public API or required type hierarchy:

```text
original = retain(returned outcome)
try construct, encode, and validate candidate while original remains retained
    failure -> return original + recording cause; no append
try append candidate at acknowledged head
    inserted -> adopt candidate; recovery may now start
    not inserted -> return rejected original/candidate and separate winning observation
    Store error -> return original/candidate + Store cause with exact acknowledgement status
```

If the timeout exceeds its declared bound, the invocation retains the timeout and measured size
rejection. If append times out ambiguously, it retains the candidate and Store cause. Neither case
calls the handler or attempts to record a smaller substitute error.

### 10.3 Recovery faults are ordinary recovery results

The examples below accompany the complete resulting Runtime state specified in section 6.
After a failed outcome commits at H, a handler failure can produce this concrete recovery result:

```text
original outcome: H
original error and execution context/facts: retained in this commit
classification: completed classification
result: handler failed { exact typed handler error }
```

If a handler returns Restart but Runtime denies it, retain the returned request and authorized
Stop/reason. Expected denial is an authorization result, not an invented callback failure.
If terminal root mapping fails, retain:

```text
original outcome: H
original error and execution context/facts: retained in this commit
classification: completed classification
request: Stop
authorization: Stop { reason: Requested }
result: mapping failed { mapper step, input to that step, exact typed mapper error }
```

This example starts with a requested Stop. Root mapping can also follow an authorized Stop after
a denied Retry/Restart request; retain the actual request and completed authorization/reason in
that fault rather than replacing the request with Stop or losing the denial reason.
Root mapping converts terminal domain failure into the Program's root failure type; it is not the
mechanism that selects or performs Restart. Preserve completed evaluation facts in concrete result
alternatives, with existing exact callback/value associations and frame-local objects. Ordinary
local ownership retains callback inputs and returned values across subsequent work; no generic
persisted progress/capture state machine or decoder-error schema registry is needed.

A classification panic or task failure retains the known stage and reviewed task evidence, with
explicit payload withholding; it does not manufacture a returned classification or request.
A valid recovery fault is committed and blocks further execution; explicit resume returns it
without reevaluation, including when its cause is a task failure rather than a returned callback
error. If that fault cannot qualify/encode/append, apply section 10.1. H remains the durable
original failure.

### 10.4 Same-Store limits

Pre-genesis, invalid/unavailable history, local mismatch, and configuration/index/provisioning
outside run execution return causal invocation errors without synthetic runs or audit appends.
Response delivery failure after commit preserves the committed outcome and delivery cause without
reopening execution. Cancellation/process death before acknowledgement cannot guarantee a record
of the physical attempt. No independent spool, raw archive, retrying logger, or second audit store
is introduced to hide these limits.

## 11. Owner capture and adjacent boundary cutovers

### 11.1 RPC and EVM

Keep the baseline's reviewed provider capture: method/stage, HTTP status, numeric RPC code, exposed
source chain, checked local field/range/size facts, and explicit omissions. Status is retained before
bounded body reading and survives a later body error. Do not restore untagged envelope fallbacks,
unit transport mappings, repeated provider schemas, or duplicate funding RPC implementations.

Preserve provider, custody, and signer causes through transaction wrapping, adding operation context
once. Keep classification, command identity, duplicate-safe Read rules, response/deadline bounds,
and transaction reconciliation unchanged. A server error or timeout does not prove nonacceptance.
Update owner schemas and actual-limit consuming tests together; retain the existing development
funding helper's safe capture without adding a production adapter solely for that test.

### 11.2 PostgreSQL Store, configuration, index, custody, and gates

Use one SQLx-specific extraction owner and existing typed port dispositions. Retain SQLSTATE,
reviewed client/source categories, operation/transaction stage, and concrete failed gate predicates.
Share the nested database evidence rather than duplicate capture or merge port/domain ownership.
A new crate or global database-operation registry is not a prerequisite; justify any dependency
layer against the existing owners before adding it.

Delete generic unavailable/internal source sinks, Protocol markers, and substring-based gate
routing. Fallible gate errors must reach their caller from the same acquired connection before
protected IO; a pool callback that logs/retries and replaces its error with a timeout does not
satisfy custody. Use owned checked acquisition for affected paths, reject/close failed connections,
and leave no unchecked protected acquisition path. Do not add a concurrent last-error registry.
Client-internal attempts that never reach an owned capture point are explicitly unavailable.

Preserve repeatable-read complete loads, advisory-locked synchronous-commit exact-head appends,
split roles, separate custody authority, epoch/schema checks, and ambient-input exclusion. Keep
precommit rejection versus ambiguous COMMIT unchanged. Preserve the custody port's own established
acknowledgement/reconciliation disposition instead of imposing Store semantics on every port.
Database messages, SQL parameters, locators, and client Display are not reviewed evidence.

### 11.3 Signing, keystore, memory, application, and transports

Replace unit signing/thread/channel/task/owner conversions with reviewed nested causes. Do not
retain a rejected channel command containing key material. Opaque cryptographic rejection is
explicit, not invented detail. Preserve the non-Send/non-Sync keystore design and strengthen tests
for secret exclusion and ownership.

Memory Store/index failures retain task/allocation/physical-validation causes; ordinary absence
and Pending keep their protocol meaning. Application and CLI/REST conversions retain reviewed
filesystem, environment-access, parse, entropy, listener, and delivery causes without source input.
Public code/status/exit policy remains a projection of the retained error. Update structured safe
detail and recording status within the existing surfaces; add no diagnostics endpoint or alternate
composition/recovery path. Failure to render an error must not replace its primary cause.

## 12. Actual size limits, without future-capacity admission

Remove prospective whole-run capacity admission. Admission qualifies the actual Program, initial
context, and genesis; it does not promise that every possible future outcome, recovery decision,
or Effect settlement will fit. Effect preparation acknowledges command authority, not a storage
reservation. The extra snapshot and recovery records need no replacement lifecycle equation.

Keep existing object/descriptor/frame/run ceilings and checked overflow rejection at their owning
boundaries. Qualify each actual complete candidate, including its local object closure, and check
the resulting history's actual frame count and bytes before append. Store remains mechanical;
no quota table, reservation service, or second capacity model is added. Preserve bounded diagnostic
capture and semantic retry/restart/global-decision limits: these constrain evidence and permitted
execution, respectively, rather than predict future storage consumption.

Delete ConclusionBound, EffectBounds, HistoryBound, LifecycleBound, Program::history_bound,
Runtime::validate_admission_bound, current_frame_bound, and the associated authoring arguments,
Execution fields, declared-frame checks, wire/schema fields, exports, fixtures, and documentation.
Delete max_pending_failures, failure_frame_bytes, the capacity-only pending-failure counter and
PendingFailures size disposition. Remove shipping-domain bound calculators that exist solely to
supply these declarations. Retain actual wire/schema/value checks and any independently required
protocol response bounds. Remove tests of the deleted estimates and quotas; retain hostile-input,
actual-limit, command-authority, and semantic-recovery coverage in their consuming boundaries.

If an outcome or recovery candidate exceeds an actual limit, use section 10's recording stop rule:
retain the available original and size cause, append nothing, and preserve the last acknowledged
state. A failure may have committed while its recovery decision cannot fit. An Effect may have
settled externally while its settlement record cannot fit; the acknowledged pending command stays
authoritative and the invocation must not claim durable settlement. Do not truncate the required
record, manufacture a smaller failure, create replacement command authority, or roll back history.
This is an explicit removal of advance history-capacity assurance, not a claim that exhaustion is
impossible. The old calculation did not reserve physical database capacity or guarantee Store
availability either.

Explicit resume of an unresolved pending Effect retains the same command and EffectId under the
existing authority protocol, without a capacity-only failure quota. It may encounter the same
recording limit again; progress is not guaranteed. Removing a storage quota does not authorize an
automatic retry loop or retry of a committed fault. Section 8.5 defines the resume contract.
Pending without a record consumes no history bytes or frame; reconciliation must still avoid
charging a semantic recovery decision twice.

Measure complete shipping schemas, full checkpoint contexts, and representative large actual
commits against current ceilings as an early encoding milestone. Frame-local sharing deduplicates
equal complete objects, not common fields within distinct checkpoint contexts. Exact candidate
sizes require the concrete wire; do not require a complete implementation before handoff.
Report actual-limit rejection and full-prefix verification cost;
do not require proof that every possible future history fits. Resolve any unacceptable snapshot
size through owner-level simplification or an explicit design decision, never omitted evidence or
an unreviewed limit increase.

Every nontrivial implementation report must identify removed code, necessary additions, production
Rust LOC change, and remaining public types/callbacks/change sites. No numerical LOC forecast is
claimed here. Tests and documentation must not be removed merely to improve the metric. Reject a
layer whose only justification is supporting another unneeded layer.


### 12.1 Early sizing fixtures and report

Use the existing consuming fixtures as seeds, with synthetic public data and fake adapters:

| Fixture | Required samples |
| --- | --- |
| Portfolio snapshot (`crates/app/tests/portfolio_runtime.rs`) | Existing fixture and variants with 1, 16, and 64 balance sources; measure accumulated context, an operational failure, and its recovery commit. Record actual active checkpoint counts. |
| Anchored contract-call Read (`crates/domains/evm/src/anchored_call/context.rs`) | Success and provider failure using the same input/intent; include accepted evidence and the retained original in recovery. |
| Transaction Effect (`crates/live/evm/tests/generic_transaction_runtime.rs`) | Both generic context shapes; preparation, pending failure, operational Stop, successful settlement, and settlement with an interpretation fault. |
| Checkpoint duplication stress | Three legal synthetic Programs with (active checkpoints, target canonical context size) of (1, 1 KiB), (4, 64 KiB), and (16, 1 MiB). Use distinct checkpoint values with mostly equal fields, plus an identical-value control, to expose whole-object sharing limits. Report actual encoded sizes. |

For each sample report exact descriptor bytes, current/checkpoint value bytes, audit-fact bytes,
unique frame-local object count, complete canonical frame bytes, and complete history bytes/frame
count. Identify shared objects so component totals are not confused with complete frame size.
Include an actual candidate exceeding the frame ceiling and a valid-prefix extension exceeding a
run ceiling; record the rejection and unchanged acknowledged head. These are actual-limit tests,
not estimates of all possible execution paths.

Measure full-prefix qualification and transition verification separately from Store loading, using
valid histories targeting 1, 100, and 1,000 frames, plus the largest tested valid prefix within the
existing ceilings. If a target cannot fit, report the first limiting resource and the largest
measured valid prefix rather than omit the sample. Use enough semantic recovery allowance for the
fixture; do not weaken transition rules to manufacture long histories. Report exact frame/byte
counts, elapsed verification time, build profile, hardware, and the measurement command. Use the
same fixtures/environment for a baseline comparison where the old and new contracts both apply;
report median and range from five measured runs after one warm-up.

Deliver this table with the early encoding/deletion diff, before broad consumer migration. Exact
shipping fixture sizes are measured results, not assumed budgets. There is no established latency
SLO in this RFC: report absolute times and comparable baseline ratios, and explicitly flag any
shipping fixture rejected by size or slower than baseline for review. Do not call cost acceptable
without the measurements, invent a passing threshold, silently raise limits, or add storage layers
to hide an unfavorable result. This bounded experiment is implementation evidence, not a new
benchmark framework or a requirement to finish persistence before handoff.

## 13. Logical commits and complete cutovers

Implement from current HEAD, retaining the committed RFC revisions. Use `15829d89` as the
production-code review baseline; the intervening commits change only this RFC at handoff. The
worktree is clean and the stopped expansion is already archived outside it. No reset, rollback,
or restoration of that expansion is required. Consult the archive only for individually reviewed
ideas or test cases; do not restore its decoder-error/capture machinery as a shortcut.

1. **Owner capture cutovers:** finish PostgreSQL/custody and signer/local-IO families in dependency
   order, each with all affected port consumers, redacted surface conversions, schemas, bounds,
   tests, and deletion of its lossy constructors. The baseline RPC/EVM cutover is retained subject
   to verification, not repeated.
2. **Native checked decoding, where required:** keep constructor causes outside Serde text-only
   conversions, with native field companions and source-preserving admission/candidate errors.
   Update every affected consumer in the same commit; no decoder-error identities or temporary
   error registry. Keep this separate only if it leaves a coherent usable boundary.
3. **One persistence/lifecycle cutover:** persist complete Runtime state and concrete audit facts,
   remove FoldState/accumulating replay, and implement the single transition used for live
   construction and adjacent-history checking. Split outcome/recovery commits, add awaiting-recovery
   and phase-specific internal states, and move policy evaluation after outcome acknowledgement.
   Update the existing append path, associations, actual-limit checks, views, clients, docs, and
   tests together; remove prospective lifecycle admission and capacity-only quotas from all
   producers and consumers, alongside fused decisions, reconstruction, all contextualizer machinery,
   and internal-fault reentry paths. Apply section 8.5 and update baseline resume expectations.
   Address section 18's affected validation items before expanding the implementation.
4. **Completion audit:** reconcile all first-loss rows with consuming evidence and run selected
   integration verification. This step is not permission to defer each prior boundary's tests,
   leave consumers unported, or accumulate a noncompiling workspace.

Merge only genuinely inseparable producer/consumer changes. Do not preassign the entire remaining
repository migration to one enormous commit. Each commit leaves one current design and uses a
lower-case subject. Revise only affected exact wire/ABI identities, rejecting superseded formats;
no legacy reader, migration, history rewrite, or unrelated envelope bump. The final exact version
identifiers follow the implemented contract, not the abandoned Program v7/Journal v5 sketches.

Store remains mechanical complete-prefix load and atomic exact-head append. Journal record changes
do not imply a new audit table or PostgreSQL migration. Update design, architecture, transport
contracts, and audit inventory with implementation. Replace AGENTS.md/design/architecture claims
of a sole semantic fold with one Runtime transition and independent adjacent-history verification;
retain their source-preservation, Store-boundary, and local-mismatch rules. This proposal does not
claim the current source implementation already follows the revised architecture.

## 14. Acceptance criteria and verification

| ID | Required consuming evidence |
| --- | --- |
| A1 | Equal classifications with different provider codes/causes remain distinguishable in committed outcomes and cold observations. |
| A2 | HTTP status survives body failure; send/body/parse/field/size stages and omission ownership remain distinct. |
| A3 | Provider/custody/signer wrappers retain operation and nested causes without altering existing classifications or command authority. |
| A4 | SQLx/gate/decode/COMMIT failures preserve reviewed facts, same-connection gate refusal, and exact acknowledgement semantics. |
| A5 | Signing/channel/task/crypto/local-IO causes remain distinct; synthetic secrets are absent from persisted data, formatting, and responses. |
| A6 | Diagnostic constructors/decoders enforce source order, opaque intermediate ancestry, field compatibility, and independent byte/layer/fact/omission limits. |
| A7 | Success commits before successor entry; original failure commits before classification/handler/map; recovery candidate may be constructed before append, but adoption/dispatch require known insertion. |
| A8 | Handler/map faults retain the committed original, completed evaluation facts, exact failed step/input/cause; explicit resume returns the committed fault without policy/mapper reevaluation. |
| A9 | Every section 8 prefix restores the final stored state after genesis/adjacent checks; cold inspection executes no recovery/completed-interpretation callbacks and preserves pending-command verification. |
| A10 | Encoding failure, definite append failure, conflict, and ambiguity retain originals/causes and truthful status; no substitute append or speculative action. |
| A11 | Admission performs no future-lifecycle reservation; actual candidate/history limits and checked overflow reject before append. Test failure committed but recovery unable to fit, and externally settled Effect whose record cannot fit, preserving originals, nonacknowledgement, and command authority. |
| A12 | Unqualifiable originals and unencodable recovery faults end recording without fallback records; task failure does not claim an unreturned native value survived. |
| A13 | Pre-admission, invalid-history, unavailable-Store, interruption, and postcommit delivery failures obey section 10.4. |
| A14 | Existing CLI/REST/library public dispositions remain projections with permitted causal/recording detail; no alternate audit surface. |
| A15 | Every inventory first-loss row maps to its replacement and consuming test; hidden upstream evidence is marked unavailable, not fixed. |
| A16 | Committed settlement plus interpretation failure retains evidence; resume returns the fault without interpretation or Effect adapter reentry. In-memory-only settlement makes no durability claim. |
| A17 | Reconciliation cannot spend a decision twice or replace it through a second policy evaluation; losing candidate evidence is preserved even if reload fails. |
| A18 | One actual Runtime state is persisted with audit facts through the common append path; no FoldState accumulator, parallel reconstruction model, generic snapshot registry, or second transition implementation remains. |
| A19 | Exact value contracts preserve required input/output fields, result tags, causes, and phase through canonical round trips; native errors retain reviewed constructor facts. |
| A20 | Exact encoded candidates pass checked decoding/evidence/frame qualification and match constructed state and predecessor audit facts before append; invalid contract/phase/binding/position or stale head cannot be adopted. |
| A21 | Direct-classification handlers preserve defaults/overrides, request versus authorization, and existing target restrictions. |
| A22 | No adapter_context callback, context-only type/codec/ABI/registration, or redundant reporting plumbing remains; facts come from existing input/intent/command and preparation/binding consistency checks remain enforced. |
| A23 | Checkpoint restart directly uses its persisted complete input with a fresh visit and current usage/barriers; adjacent verification rejects checkpoint substitution, counter reset, and removed Effect authority. |
| A24 | Preparation failure, pending failure, and accepted-evidence interpretation failure have legal distinct representations without fabricated defaults. |
| A25 | Representative large complete current/checkpoint contexts and repeated originals are measured against actual frame/run ceilings; oversize rejection and full-prefix verification cost are reported, with no future-history fit promise or mandatory Clone/projection API. |
| A26 | Distinct owner internal causes survive ordinary outcome commit/cold observation without classification; preflight mismatch causes no provider call/append, and post-response local binding rejection retains facts without append or a false no-IO claim. |
| A27 | No decoder-error Program values, direct/terminal error codecs, recursive companion identities, or constructor-error association remain. |
| A28 | Valid recovery faults retain original/context plus exact owner facts and local objects; unqualifiable faults retain invocation causes without a dynamic error/schema capture framework. |
| A29 | Structurally valid stored snapshots with wrong visit, impossible stop reason, reset usage, substituted checkpoint context, or revived Effect authority fail historical verification before observation/IO. |
| A30 | Live construction and adjacent checking call the same transition implementation; each history check starts from its stored predecessor and direct restoration adopts the final stored state. |
| A31 | Loaded inspection/resume/conflict/ambiguity histories are fully checked; known inserted live append does not reread the whole history, and reconciliation cannot claim insertion from an unrelated valid head. |
| A32 | Resume follows section 8.5: unfinished work continues, committed internal/recovery/task faults return without work or append, and an uncommitted interruption preserves its acknowledged continuation. Operational pending Stop retains same-command reconciliation; repair-and-resume is unsupported. |

Use boundary-focused consuming tests and synthetic/fake-server evidence, not a parallel model of
the pipeline or real secret-bearing diagnostic dumps. Retain hostile-input tests for the actual
canonical wire, frame closure, and authority contracts after removing superseded machinery tests.

Follow [build and verification](docs/build-and-verification.md): narrow affected commands in the
Nix shell, affected dependent compilation/tests, managed PostgreSQL and Effect/client scenarios,
and one final `nix run .#ci` on the exact implementation candidate. Do not substitute repeated
focused success for workspace integration. Update SQLx metadata only for actual query changes.
This RFC-only revision requires link/contract review and `git diff --check`, not Rust CI.

## 15. Deletion and retention checklist

| Baseline or stopped machinery | Target |
| --- | --- |
| Lossy adapter/port/application conversions | Typed owner causes with reviewed nested diagnostics and unchanged public projections. |
| Fused failure/decision branches in Journal conclusions and engine conclude/decide | Outcome commit, then recovery commit, each retaining complete Runtime state and audit facts through the common append path. |
| Unit PreparationError / StateExecutionError / AdapterInvariantError | Exact owner internal causes with phase-specific outcome rules; no global domain-error bag. |
| IncidentSummary and summary-only source plumbing | Direct Classification from the retained original, plus derived recovery context. |
| adapter_context, AdapterContext/context-only values, codecs, ABI entries, registration, and reporting plumbing | Delete completely; retain facts in existing input/intent/command owners and preserve preparation/binding checks. |
| Internal/recovery/task-fault reentry paths and repair-and-resume expectations | Committed faults stop progress; resume follows only unfinished acknowledged work and unresolved command authority. |
| FoldState and engine history-to-accumulator replay | Persist actual Runtime state; retain transition rules once and check adjacent stored states without accumulation. |
| Separate live-state/snapshot models and independently supplied PersistedRecoveryContext | One complete Runtime state; recovery context borrows it and Program. |
| New generic commit facade over prepare_append/finish_append | Extend the existing append path and preserve originals/acknowledgement there. |
| DecodePersisted::Error: MfmValue and derive-generated error identities | Native checked-construction errors and, where useful, native field companions. |
| ValueCodec decoder-error registration and terminal codecs | Existing value codecs with native source-preserving failure returns. |
| DecoderTarget/owner-closure machinery solely for decoder-error claims and frozen giant identities | Delete with its dedicated schemas/fixtures; preserve ordinary object/schema/binding validation. |
| Generic capture/progress roles, CaptureFailed and partial fallback recording | Concrete ordinary outcome/fault variants; stop failed recording with invocation custody. |
| Prospective lifecycle bounds, declared-frame budgets, pending-failure quotas, and X/Y capacity defaults | Delete their APIs, calculations, fields, checks, and dedicated fixtures; retain actual size ceilings and semantic recovery limits. |
| Rejection of repeated context/checkpoint data | Complete persisted continuation including semantic usage and checkpoints, with actual candidate checks. |
| Superseded tests/docs promising these mechanisms | Replace with retained-behavior boundary evidence in the same cutover. |

Keep intrinsic classifiers, static handlers, root mapping, active checkpoints, exact schemas and
value association, canonical qualification, frame-local object closure, evidence binding, Effect
barriers, Store atomicity, acknowledgement ambiguity, secret exclusion, and keystore ownership.
Deletion of decoder-error machinery does not authorize deletion of these independent guarantees.
Do not force unrelated API renames or remove ProposedStateOutcome simply because the stopped
implementation replaced it. Every replacement must remove a required source-loss or lifecycle gap.

## 16. Completion evidence

The implementation is complete only when the consuming workspace is integrated, the inventory and
A1-A32 are reconciled, required verification passes on the exact candidate, and the final report
identifies remaining upstream limits and actual production LOC change. Do not use old focused test
counts, partially migrated clients, or large-but-admissible descriptors as evidence of completion.

## 17. Agreed changes from superseded proposals

This is one revised target. It replaces both the stopped generic snapshot/capture design and the
subsequent proposal to reconstruct context from outcome history. Complete state/context commits
and duplication are now deliberate requirements for auditability and direct restoration.

Remove the fold as a reconstruction component, not Runtime's transition rules or independent
history checks. One persisted Runtime state and one transition implementation replace replay and
separate state models. Journal encoding and Store append remain mechanical; no generic snapshot,
proof, or commit framework is added. Native decoder errors and the no-substitute recording rule
remain unchanged. Clone and codec::from remain optional implementation ideas, not mandates.

## 18. Material uncertainties

| Assumption or unresolved choice | Why uncertain | Consequence if wrong | Validation / resolution |
| --- | --- | --- | --- |
| Active declaration-checkpoint semantics satisfy recovery to an earlier point | Informal wording could mean any historical visit | Arbitrary visit targeting would change Program targets and persisted checkpoint selection | Retain the existing semantics in this RFC; require a separate explicit target change before adding arbitrary historical selection. |
| Complete Runtime states and audit facts fit current ceilings for intended workloads | Full checkpoint contexts and repeated originals may greatly exceed prior outcome-only frames | Actual commits could be rejected even when admission succeeded | Run section 12.1's named fixtures and report before implementation expansion; resolve unacceptable sizes explicitly without omitted state or unreviewed limit increases. |
| Direct state restoration and adjacent checks simplify implementation at acceptable cost | The complete persisted shape and verification cost have not been measured | Serialization/checking could add complexity or excessive load cost despite removing reconstruction | Review the concrete deletion/replacement diff and section 12.1's timed prefixes and baseline comparison; no constant-time, latency-SLO, or fixed LOC claim. |
| A live ownership change is useful and compatible, if proposed | Consuming generic inputs and custom value semantics need checking | Added Clone bounds could exclude valid inputs or conceal mutation | Do not require the change for this RFC; separately compile consumers and test retained-original semantics without weakening candidate validation. |
| Reviewed external capture covers exposed evidence | Clients may hide attempts or only expose opaque sources | Some causal detail remains unavailable | Inject distinguishable nested causes at each owner and assert explicit omissions; never fabricate hidden evidence. |

Resume semantics and complete contextualizer deletion are settled in sections 6.3 and 8.5.
Remaining workload-size and implementation-cost checks are bounded engineering evidence, not
permission to reopen fault reentry or retain contextualizer machinery. No fixed LOC forecast is
claimed. If implementation exposes a new ownership conflict, apply the repository's architect
rule; difficulty is not permission to recreate the rejected machinery.
