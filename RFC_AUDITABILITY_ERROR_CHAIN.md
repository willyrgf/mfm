# RFC: preserve causal errors and reconstruct recovery from committed history

Status: revised proposal following the architecture review. This document consolidates the agreed
simplification; it does not claim that the stopped implementation satisfies it. Implementation is
reviewed against `15829d89` (`preserve rpc and evm error provenance`), which already contains the
RPC/EVM capture cutover. The large uncommitted expansion is not the implementation foundation.
Its focused test counts, descriptor sizes, and completion claims are not acceptance evidence for
this target. Do not land or extend its machinery merely because it already exists.

The [adapter error audit](docs/adapter-error-audit.md) remains the first-loss inventory.
[Design](docs/design.md), [architecture](docs/architecture.md), [code quality](docs/code-quality.md),
and [AGENTS.md](AGENTS.md) govern the repository. This RFC is a proposed replacement contract;
implementation must update the authoritative documents, producers, consumers, and tests in the
same coherent cutover. Existing worktree documentation of the stopped design is not evidence that
that design should be retained. Remaining decisions are explicit in section 18.

## 1. Summary

Commit new execution facts and decisions. Reconstruct execution context from the authoritative
append-only history. Preserve the complete available reviewed causal chain; classification,
recovery requests, and public rendering are projections, not replacements for the original error.

A State is a deterministic associated implementation operating on typed input. Its executable
object is not a persistence snapshot. Context is already ordinary immutable value data inside
State inputs and outputs. Genesis retains the complete initial context, successful outcomes retain
complete outputs, and the existing fold reconstructs the input at each active checkpoint. Required
input fields cannot be omitted, but the same input need not be committed again beside each outcome.

Use the existing concrete Journal records, qualified values, sole semantic fold, and common
`prepare_append` / `finish_append` path as the starting point. Separate outcome from recovery:

```text
execute -> commit outcome
    success -> next State
    domain/operational failure -> classify -> handler request -> Runtime authorization
        -> commit recovery decision or fault -> authorized transition or end invocation
    internal failure -> end invocation with its actual execution phase retained
```

If an outcome cannot be constructed, qualified, or encoded, stop and return the available original
and recording cause. Do not construct a substitute audit record. A Store failure also ends that
recording attempt, with definite rejection and ambiguous acknowledgement kept distinct.

No new state/context snapshot API, generic commit facade, partial-capture framework, decoder-error
registry, independent audit store, or cross-frame object resolver is required by this design.

## 2. Decisions established in discussion

| Question | Selected contract |
| --- | --- |
| What must recovery restore? | The selected execution position, its complete typed input/context, original failure, and actual phase, from the acknowledged prefix. |
| Must the whole State object be stored? | No. Runtime associates the executable implementation; history retains its required data. |
| Must the whole context be copied at every outcome? | No. Genesis and complete predecessor outputs already retain it. The fold reconstructs it without executing completed States. |
| What is a restart target? | An eligible active declaration checkpoint, with its retained input; not an arbitrary historical visit. |
| Does restart roll back the machine? | No. It appends a new visit and preserves current history, usage, and Effect constraints. |
| What does the handler do? | Recommends Stop, Retry, or Restart at a permitted target. Runtime authorizes and the transition applies the committed decision. |
| When does recovery run? | After the original outcome is known to be committed. Its decision commits before any recovery action. |
| How are internal outcomes treated? | Valid typed internal outcomes are durable at an admitted cursor, subject to qualification/capacity; they grant no handler permission. Preflight mismatches cause no provider call/append; post-response local binding rejection also forbids append without claiming IO did not occur. |
| How is persistence selected? | Existing concrete record and value types declare required facts. A new `codec::from` projection API is not mandated. |
| What do codecs enforce? | Exact type/schema and canonical value admission. Candidate qualification and the sole fold remain before append. |
| Are decoder errors Program values? | No. Checked decoding retains native constructor causes; no decoder-error codec or identity is registered. |
| Is Clone required? | Not by this RFC. Native copying is a separate ownership choice, never a replacement for candidate validation. |
| What if recording fails? | Return the original when available, candidate when produced, and causal recording result. No substitute record or recursive append. |
| Is bounded/redacted capture lossless? | No. Account explicitly for withheld, opaque, unavailable, and bound-limited evidence. |

## 3. Baseline facts and actual gaps

At `15829d89`, `JournalRecord::RunAdmitted` retains Program and initial context.
`FoldState::succeed` passes each retained output to the next State; `enter` retains active checkpoint
inputs; `recover` restores the selected input while charging current usage and creating a fresh
visit. Transaction States use `ContextSlot::replace` to preserve unchanged context siblings.
These mechanisms already reconstruct the data needed for current restart semantics.

The gaps are narrower:

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

## 6. What history must retain

### 6.1 New facts at each boundary

| Boundary | New facts to commit | Already retained or reconstructed |
| --- | --- | --- |
| Admission | Program and complete initial context | Program pins implementations, policy parameters, targets, and bounds. |
| Pure success | Execution position and complete output | Active input from the preceding prefix. |
| Pure domain/internal failure | Position and original typed error | Active input/context. |
| Read preparation failure | Position, preparation stage, typed internal cause | Input; no valid intent is invented. |
| Read adapter failure | Exact intent and original typed error | Input and any contextual fields derived from input/intent. |
| Read evidence accepted | Intent, accepted evidence, and interpretation result, including internal failure | Input. |
| Effect preparation | Exact command, position, and derived EffectId, before adapter entry | Input. |
| Pending Effect failure | Original cause and originating stage/phase | Acknowledged command, EffectId, and input. |
| Accepted Effect settlement | Accepted evidence and interpretation result, including internal failure | Acknowledged command, EffectId, and input. |
| Recovery evaluation | Connection to unresolved outcome, completed classification/request where available, and authorized decision or typed evaluation fault | Input, phase, allowances, checkpoints, and eligible targets from Program and prefix. |
| Terminal root mapping | Mapped root, or failing mapper step with its input and typed cause | Original error remains in its execution record. |

Use concrete phase alternatives: preparation failure has no fabricated intent; adapter failure has
no fabricated accepted evidence; interpretation failure retains the accepted evidence. Position,
sequence, predecessor, and run identity remain checked frame/fold facts, not competing copies in
owner payloads. An outcome frame and its complete local objects append atomically.

The recovery result is associated with the currently unresolved outcome by record structure and
the sole fold. A fault followed by explicit reevaluation, if enabled, remains associated with that
same outcome. Do not copy the original error merely to establish this relationship. Any object
references actually included in a frame retain the existing exact frame-local closure rules; this
adds no lookup of arbitrary objects in earlier frames.

### 6.2 Context is reconstructed, not supplied again

```text
genesis(C0)
A succeeds(C1)
B succeeds(C2)
C fails(E)
recovery selects checkpoint B
new visit to B starts with C1
```

The prefix already contains C1. The fold rebuilds its active checkpoint inputs from genesis,
successful outputs, and committed transitions. It does not rerun A to reconstruct C1. Restart
retains the current recovery usage and Effect barriers; neither history nor counters roll back.
Checkpoint selection retains the baseline's active declaration semantics, not an arbitrary old
visit selected by frame number.

Recovery context is a view derived from the immutable Program and acknowledged prefix. Do not
persist another authoritative snapshot of allowances, usage, eligible targets, or checkpoint maps.
A returned handler request and the authorized decision are distinct facts: a denied Restart may
produce Stop, so retain the request rather than inferring it from the final action.

Do not omit required fields from State inputs or successful outputs to save bytes. Their exact
contracts must suffice for later preparation, interpretation, and checkpoint restoration. Additional
facts genuinely produced by execution belong in the existing owner's result/evidence contract.
Transient locals that neither affect reconstruction nor add required causal evidence stay transient.

### 6.3 Remove redundant contextualization, retain its checks

Audit every `adapter_context` implementation before deletion. The inspected balance contextualizer
derives collection/source ordinals from input and copies intent; anchored-call context derives from
intent; transaction context derives from command. Those facts need no second reporting object.

Remove the mandatory fallible contextualizer callback, associated context-only ABI entries, and
redundant wrappers once their facts are available from the reconstructed input and recorded objects.
Keep command/input and evidence-binding validation at its owning boundary. A contextualizer that
contains a unique fact requires an explicit producer-owned field, not silent removal. Do not add
an equivalent postcommit callback or registry under another name.

## 7. Outcome, recovery, and transition ownership

### 7.1 Handler recommendations are not transitions

The handler receives Classification directly and Runtime's derived recovery context. It recommends
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
can authorize a new visit, restart, or stop. A competing/ambiguous decision cannot release an action.

Extend existing concrete Journal alternatives, assembly associations, and the sole semantic fold.
Do not introduce independently identified generic ExecutionData/context values around objects the
Journal already retains. The existing State outcome API is not replaced merely to standardize a
Rust spelling such as nested Result. Wire changes must follow the required outcome alternatives.

Keep `prepare_append` / `finish_append` as the common mechanism, fixing custody and acknowledgement
handling there as needed. Candidate validation must not mutate the caller's acknowledged snapshot.
Do not add a generic commit facade over this path or a second transition implementation.

### 7.3 Internal outcomes and Effects

Replace unit State preparation/execution and adapter internal errors with owner-typed causes.
Distinct operations and reviewed expected/observed facts remain distinct despite an equal Internal
disposition. Framework qualification, Journal, and task errors keep their own native owners; an
owner's domain invariant must not become a generic framework code.

A valid internal execution outcome at an admitted cursor is committed and ends the invocation.
It does not invoke classification or a recovery handler. As required by AGENTS.md, preflight local
association mismatches reject before provider entry and never append. A local evidence-binding
rejection after a provider response also authorizes no append: retain its reviewed cause and
available reviewed observation in the invocation, without claiming the provider was never called.
These exceptions cannot discard mismatch facts or fabricate authenticated external integrity evidence.

Preparation acknowledges the exact command and EffectId before Effect adapter entry. Pending is
not an error and returns the unchanged acknowledged pending view without an extra record. Pending
failures retain command authority. Accepted settlement evidence belongs in the outcome even if
interpretation returns an internal error. After that evidence commits, permitted interpretation
reentry reuses it and never reenters the Effect adapter.

Settlement observed only in memory is not durable settlement. A crash or failed encoding before
its record commits leaves the preceding pending command authoritative. Preserve existing
reconciliation/ambiguity semantics; never claim every physical attempt was recorded.

## 8. Reconstruction and interruption table

| Last acknowledged boundary | Reconstructed facts | Permitted next work |
| --- | --- | --- |
| Before outcome | Runnable input or acknowledged pending command | Only phase-permitted execution; an uncommitted physical attempt may have occurred. |
| Success outcome | Output and next position, or terminal success | Next State only. |
| Domain/operational failure | Original error, input, phase, awaiting recovery | Evaluate unresolved recovery without rerunning the failed State/provider to recreate the error. |
| Recovery decision | Decision, counters, and resulting cursor | Follow committed continuation; do not ask the handler to replace it. |
| Recovery fault | Original outcome and evaluation fault | End invocation; later explicit reevaluation requires the finite contract in section 12. |
| Internal outcome | Cause and actual phase | No handler; only explicitly permitted phase-preserving reentry. |
| Internal interpretation outcome with committed settlement | Input, command/EffectId, and accepted evidence | If reentry is enabled, deterministic interpretation only; no Effect adapter call. |
| Outcome construction/qualification/encoding failure | No newly acknowledged outcome | Return available original plus recording cause; no substitute record. |
| Ambiguous append | Last acknowledged observation and uncertain candidate | Reconcile through existing exact-head semantics before dispatch. |

Cold observation never executes classifiers, handlers, root maps, or completed State interpretation.
Retain existing pure evidence checks and the baseline's final pending-Effect preparation check
(`FoldState::validate_pending`); deleting contextualizers must not remove that verification exception.
Runtime association must match the Program's exact selected contracts before any unresolved work.
No historical decision is replaced by an updated or newly selected policy.

## 9. Checked values, codecs, and native decoder errors

Persistence selection and value validation are separate. Existing concrete record types declare
which facts are retained. Canonical encoding, exact schema/reference checks, checked owner
construction, and the sole fold validate their representation and legal transition before append.
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
candidate/fold validation. No new projection API or qualified-record layer is mandated here.

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

After a failed outcome commits at H, a handler failure can produce this concrete recovery result:

```text
original outcome: H
classification: completed classification
result: handler failed { exact typed handler error }
```

If a handler returns Restart but Runtime denies it, retain the returned request and authorized
Stop/reason. Expected denial is an authorization result, not an invented callback failure.
If terminal root mapping fails, retain:

```text
original outcome: H
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
A valid recovery fault is committed and ends the invocation without recursive handler entry. If
that fault cannot qualify/encode/append, apply section 10.1. H remains the durable original failure.

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
Update owner schemas and complete consuming bounds together; retain the existing development
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

## 12. Capacity and implementation complexity

Add the recovery result's maximum frame cost to the existing complete lifecycle admission
calculation. Include concrete outcome/internal alternatives, evidence, diagnostics, mapped roots,
and all frame-local objects. A zero-retry failure must still have room for its first Stop result.
A prepared Effect must retain enough admitted capacity to complete its allowed lifecycle.

Any repeated explicit internal reentry or recovery-fault reevaluation needs a finite admission
bound, an exact charging event in the sole fold, and checks before work begins. Storage allowance
must not grant semantic retry/restart permission. Pending without a record spends no new history
slot; committed decisions cannot spend their allowance twice after reconciliation.

The earlier X/Y fields, defaults of eight, formula, and generic capacity type hierarchy are not
accepted requirements. Section 18 records the remaining reentry decision. Settle its transition
table and checked capacity equation before implementing that boundary; do not silently enable
unbounded reevaluation or disable existing pending-Effect recovery. Reuse existing limits and
arithmetic where they express the selected guarantee without adding another reservation framework.

Keep existing object/descriptor/frame/run ceilings and checked overflow rejection. Measure complete
shipping schemas and maximum workloads. Do not import descriptor graphs or raise limits to fit
unnecessary execution/context/error wrappers. A concrete retained schema that cannot fit requires
an owner-level simplification or an explicit design decision, not omitted evidence.

Every nontrivial implementation report must identify removed code, necessary additions, production
Rust LOC change, and remaining public types/callbacks/change sites. No numerical LOC forecast is
claimed here. Tests and documentation must not be removed merely to improve the metric. Reject a
layer whose only justification is supporting another unneeded layer.

## 13. Logical commits and complete cutovers

Start review from `15829d89`. Preserve stopped work separately if needed for reference; this RFC
does not authorize destructive checkout operations or adoption of that expansion. Do not implement
on top of its decoder-error/capture machinery as a shortcut.

1. **Owner capture cutovers:** finish PostgreSQL/custody and signer/local-IO families in dependency
   order, each with all affected port consumers, redacted surface conversions, schemas, bounds,
   tests, and deletion of its lossy constructors. The baseline RPC/EVM cutover is retained subject
   to verification, not repeated.
2. **Native checked decoding, where required:** keep constructor causes outside Serde text-only
   conversions, with native field companions and source-preserving admission/candidate errors.
   Update every affected consumer in the same commit; no decoder-error identities or temporary
   error registry. Keep this separate only if it leaves a coherent usable boundary.
3. **One lifecycle cutover:** split concrete outcome/recovery records, add awaiting-recovery and
   phase-specific internal outcomes, move policy evaluation after commit, and update the existing
   fold/append path, exact associations, bounds, views, clients, and docs together. Remove fused
   decision/context machinery in the same change. Resolve section 18's affected decisions first.
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
contracts, and audit inventory with implementation. Retain AGENTS.md's local-mismatch rule.

## 14. Acceptance criteria and verification

| ID | Required consuming evidence |
| --- | --- |
| A1 | Equal classifications with different provider codes/causes remain distinguishable in committed outcomes and cold observations. |
| A2 | HTTP status survives body failure; send/body/parse/field/size stages and omission ownership remain distinct. |
| A3 | Provider/custody/signer wrappers retain operation and nested causes without altering existing classifications or command authority. |
| A4 | SQLx/gate/decode/COMMIT failures preserve reviewed facts, same-connection gate refusal, and exact acknowledgement semantics. |
| A5 | Signing/channel/task/crypto/local-IO causes remain distinct; synthetic secrets are absent from persisted data, formatting, and responses. |
| A6 | Diagnostic constructors/decoders enforce source order, opaque intermediate ancestry, field compatibility, and independent byte/layer/fact/omission limits. |
| A7 | Success commits before successor entry; original failure commits before classification, handler, and root mapping; recovery decision commits before transition. |
| A8 | Handler/map faults retain the committed original, completed evaluation facts, exact failed step/input/cause, and do not recursively invoke policy. |
| A9 | Every section 8 prefix reconstructs the same observation; cold inspection executes no recovery/completed-interpretation callbacks and preserves pending-command validation. |
| A10 | Encoding failure, definite append failure, conflict, and ambiguity retain originals/causes and truthful status; no substitute append or speculative action. |
| A11 | Capacity covers first recovery results, permitted finite reentries, and prepared settlement; overflow/exhaustion is checked before affected work. |
| A12 | Unqualifiable originals and unencodable recovery faults end recording without fallback records; task failure does not claim an unreturned native value survived. |
| A13 | Pre-admission, invalid-history, unavailable-Store, interruption, and postcommit delivery failures obey section 10.4. |
| A14 | Existing CLI/REST/library public dispositions remain projections with permitted causal/recording detail; no alternate audit surface. |
| A15 | Every inventory first-loss row maps to its replacement and consuming test; hidden upstream evidence is marked unavailable, not fixed. |
| A16 | Committed settlement plus interpretation failure retains evidence and permits no Effect adapter reentry; in-memory-only settlement makes no durability claim. |
| A17 | Reconciliation cannot spend a decision twice or replace it through a second policy evaluation; losing candidate evidence is preserved even if reload fails. |
| A18 | Existing concrete records/common append path handle success/internal/domain/operational outcomes without a new generic snapshot or commit framework. |
| A19 | Exact value contracts preserve required input/output fields, result tags, causes, and phase through canonical round trips; native errors retain reviewed constructor facts. |
| A20 | Candidate validation and local closure remain atomic; invalid contract, phase, binding, or position cannot authorize append/IO, and stale heads cannot be adopted. |
| A21 | Direct-classification handlers preserve defaults/overrides, request versus authorization, and existing target restrictions. |
| A22 | Every deleted contextualizer's facts are retained or derived from existing authoritative input/intent/command; its consistency checks remain enforced. |
| A23 | Checkpoint restart restores the exact active input with a fresh visit and current usage/barriers; no repeated input snapshot or arbitrary historical-visit feature. |
| A24 | Preparation failure, pending failure, and accepted-evidence interpretation failure have legal distinct representations without fabricated defaults. |
| A25 | Shipping accumulated-context workloads reconstruct complete inputs from history, fit exact bounds, and introduce no mandatory Clone or projection API. |
| A26 | Distinct owner internal causes survive ordinary outcome commit/cold observation without classification; preflight mismatch causes no provider call/append, and post-response local binding rejection retains facts without append or a false no-IO claim. |
| A27 | No decoder-error Program values, direct/terminal error codecs, recursive companion identities, or constructor-error association remain. |
| A28 | Valid recovery faults use existing exact owner associations and local objects; unqualifiable faults retain invocation causes without a dynamic error/schema capture framework. |

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
| Fused failure/decision branches in Journal conclusions and engine conclude/decide | Concrete outcome first, then recovery result; same fold and append path. |
| Unit PreparationError / StateExecutionError / AdapterInvariantError | Exact owner internal causes with phase-specific outcome rules; no global domain-error bag. |
| IncidentSummary and summary-only source plumbing | Direct Classification from the retained original, plus derived recovery context. |
| adapter_context, redundant AdapterContext values, and context-only registration | Existing input/intent/command facts; preserve unique facts and all consistency checks. |
| Independently associated ExecutionData/context wrappers and PersistedRecoveryContext snapshots | Concrete Journal objects and reconstruction through the existing prefix fold. |
| New generic commit facade over prepare_append/finish_append | Extend the existing append path and preserve originals/acknowledgement there. |
| DecodePersisted::Error: MfmValue and derive-generated error identities | Native checked-construction errors and, where useful, native field companions. |
| ValueCodec decoder-error registration and terminal codecs | Existing value codecs with native source-preserving failure returns. |
| DecoderTarget/owner-closure machinery solely for decoder-error claims and frozen giant identities | Delete with its dedicated schemas/fixtures; preserve ordinary object/schema/binding validation. |
| Generic capture/progress roles, CaptureFailed and partial fallback recording | Concrete ordinary outcome/fault variants; stop failed recording with invocation custody. |
| Repeated recovery/context/budget snapshots and unconditional X/Y defaults | Derived authoritative facts and an explicitly settled finite reentry contract. |
| Superseded tests/docs promising these mechanisms | Replace with retained-behavior boundary evidence in the same cutover. |

Keep intrinsic classifiers, static handlers, root mapping, active checkpoints, exact schemas and
value association, canonical qualification, frame-local object closure, evidence binding, Effect
barriers, Store atomicity, acknowledgement ambiguity, secret exclusion, and keystore ownership.
Deletion of decoder-error machinery does not authorize deletion of these independent guarantees.
Do not force unrelated API renames or remove ProposedStateOutcome simply because the stopped
implementation replaced it. Every replacement must remove a required source-loss or lifecycle gap.

## 16. Completion evidence

The implementation is complete only when the consuming workspace is integrated, the inventory and
A1-A28 are reconciled, required verification passes on the exact candidate, and the final report
identifies remaining upstream limits and actual production LOC change. Do not use old focused test
counts, partially migrated clients, or large-but-admissible descriptors as evidence of completion.

## 17. Agreed changes from the superseded proposal

This is the single revised target, not a second implementation option. The preceding sections
replace the former full execution/context snapshot contract, independent recovery-context schema,
generic commit facade, decoder-error Program identity/closure, and partial fallback recorder.

The revised target preserves candidate validation, causal capture, commit-before-recovery, and
phase-specific internal outcomes. It derives past execution input from already committed values
and keeps current transition authority. Selective codec projection and Clone were discussed as
implementation ideas, not accepted persistence requirements. No new snapshot, copy, or schema
framework may be justified by treating those exploratory ideas as a mandate.

## 18. Material uncertainties

| Assumption or unresolved choice | Why uncertain | Consequence if wrong | Validation / resolution |
| --- | --- | --- | --- |
| Active declaration-checkpoint semantics satisfy recovery to an earlier point | Informal wording could mean any historical visit | Arbitrary visit targeting would change Program targets and fold retention | Retain the existing semantics in this RFC; require a separate explicit target change before adding arbitrary historical selection. |
| Explicit internal reentry and recovery-fault reevaluation have a finite useful contract | Exact enabled phases, limits, and charging events were not settled in the review | Repeated records could exceed admitted capacity or incorrectly release Effect authority | Finalize one transition table and checked capacity equation before implementing reentry; preserve existing pending-Effect semantics and test zero/exhausted limits and settlement reservation. |
| Every contextualizer's facts derive from existing authoritative values | Only representative balance, anchored-call, and transaction consumers were inspected | Deleting an uninspected unique field could remove causal/recovery information | Inventory every implementation; move any unique fact to its actual producer before deleting the callback. |
| Current consuming schemas fit after the smaller cutover | Exact error growth and full workload bounds have not been measured | Supported Programs could fail admission | Measure actual complete descriptors/frames against unchanged ceilings before expanding implementation; simplify the responsible owner instead of importing the stopped graph machinery. |
| A live ownership change is useful and compatible, if proposed | Consuming generic inputs and custom value semantics need checking | Added Clone bounds could exclude valid inputs or conceal mutation | Do not require the change for this RFC; separately compile consumers and test retained-original semantics without weakening candidate validation. |
| Reviewed external capture covers exposed evidence | Clients may hide attempts or only expose opaque sources | Some causal detail remains unavailable | Inject distinguishable nested causes at each owner and assert explicit omissions; never fabricate hidden evidence. |

No fixed LOC forecast or claim of a closed architecture is made while the reentry contract remains
unsettled. Resolve affected architecture choices through the repository's architect rule before
implementation; difficulty is not permission to recreate the rejected machinery.
