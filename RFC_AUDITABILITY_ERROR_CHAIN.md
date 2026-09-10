# RFC: commit complete execution data and preserve causal error chains

Status: ready for engineering implementation; not implemented.

This RFC develops the agreed remediation plan from the
[adapter error-chain audit](docs/adapter-error-audit.md). It extends the implemented
[classification and handler RFC](RFC_SIMPLIFY_CLASSIFICATION_HANDLER.md) without replacing its
intrinsic classifiers or common static handler.

[Design](docs/design.md) and [architecture](docs/architecture.md) remain authoritative for the
current implementation. Contract changes proposed here must update those documents, repository
instructions, producers, consumers, and tests together when implemented. Rust and wire examples
below specify the target contracts. They omit derives, rustdoc, and mechanical error conversions;
they are not a claim that a prototype compiles. The ownership, legal variants, sequencing, bounds,
and deletion requirements are normative. Freeze generated exact descriptors with their consuming
tests in the implementation commits.

## 1. Summary

Every first-party adapter and boundary conversion must preserve the available causal provenance of
an error. Classification, recovery decisions, and public error codes are projections of that error;
none may replace its cause.

Use one uniform commit operation for the State's serializable execution data and its context:

```rust
journal.commit(&state_data, &ctx).await?;
```

`state_data` is what was meant by `self` in discussion: the execution data, including its typed
result. It is not the executable State implementation object. `ctx` is the separate, checked
serializable context used by that execution. Their exact serialization contracts define what is
retained, with no separate incident or failure object constructed merely for persistence.

Commit these data before handing control to a successor. On success, that successor is the next
State; on a recoverable-path failure, it is the selected recovery handler. Recovery uses the same
commit operation for its evaluation data and context before executing the authorized action.
Both records belong in the existing Journal and State commit Store.

Internal execution errors are a distinct failure case in the State outcome, not an independent
audit event. They remain internal and do not acquire automatic recovery permission merely because
their cause is now durable.

```text
State execution data + ctx -> commit
    -> Ok(output): next State
    -> Err(error): classify error -> handler
                    -> recovery data + ctx -> commit -> authorized action
```

The proposed implementation has three responsibilities:

1. The owner of an IO or validation boundary captures its available, reviewed causal facts once.
2. The State's typed execution data and context retain those facts under one reviewed serialization
   contract; concrete error nesting adds only necessary causal context.
3. Runtime commits the declared data before control passes onward and explicitly reports whether
   recording succeeded, failed, or has ambiguous acknowledgement.

There is no independent audit store, spool, raw diagnostic archive, generic logger, or classifier
registry. `IncidentSummary` is removed from the proposed handler API: the classifier reads the
retained error, and the handler receives `Classification` directly with Runtime-owned recovery
context. States remain deterministic and perform no ambient IO.

The guarantee is complete auditability of committed execution boundaries, within the declared
secret-free and bounded representations. It is not proof that every physical attempt reached a
commit, or permission to describe withheld client diagnostics as lossless raw evidence.

## 2. Decisions established in discussion

| Question | Selected contract |
| --- | --- |
| What does “whole error stack” mean? | Available causal layers and reviewed diagnostic facts, with explicit accounting for missing details. It is not a Rust backtrace or a promise of byte-exact raw evidence. |
| What happens to secret-bearing diagnostics? | Withhold them explicitly. Error preservation does not authorize storing credentials or arbitrary provider/client text. |
| Where are run errors stored? | In the existing State commit history, through Journal and the existing Store. |
| What is `self`? | The State's serializable execution data, including its typed success/error result; not its implementation, callbacks, or service handles. |
| What is `ctx`? | The separate admitted context used by execution, with Runtime-validated execution metadata. It remains distinct from IO services and credentials. |
| How are success and failure committed? | Through the same `commit(state_data, ctx)` operation and reviewed serialization contracts. |
| Is a separate incident required? | No. The typed result already distinguishes success and error; the handler receives classification directly. |
| Are internal State failures audited? | Yes, as State failure outcomes when execution has a valid admitted cursor and the Store can acknowledge the record. |
| Does an internal failure advance execution? | No. Its outcome advances the history head while retaining execution identity and the actual Effect phase, including any accepted settlement. |
| When does the handler run? | After the original failure outcome is known to be committed. Its decision must commit before any recovery action. |
| Does richer evidence enable new retries? | No. Classification remains error-owned and recovery remains Runtime-authorized. This cutover preserves current classification semantics. |
| Is the additional storage cost acceptable? | Yes. Fuller execution/context snapshots and additional failure/recovery records are an accepted tradeoff for auditability and simpler contracts. Capacity correctness remains required; storage savings are not a condition for proceeding. |
| Can every failure always be persisted? | No. Missing admission, invalid history, unavailable storage, exhausted capacity, and interruption have explicit limits. |

Implementation review must verify that the selected representation and recording
mechanics achieve these decisions with the least complexity. It should not silently substitute raw
secret custody or a second persistence service for the selected contract.

## 3. Problem in the current implementation

The recovery cutover preserves the typed operational value that reaches Runtime. The value is
often already lossy:

```text
RPC error: code + message + data
    -> EvmOperationalError::Unavailable
    -> EvmTransactionOperationalError::Provider { cause: Unavailable }
    -> qualified error value
    -> durable failure frame
```

The final record faithfully retains `Unavailable`; it cannot recover the discarded RPC error.
Similar losses occur when SQLx errors become unit Store/custody errors, keystore failures become
`SigningError::Failed`, or an application boundary replaces an error with a public code.

There are also losses after an adapter returns:

- A State contextualizer or recovery callback can fail after receiving an operational cause.
- Failure qualification, report construction, or append can fail and replace the original cause.
- A competing append can win; returning only its view discards this caller's losing failure.
- Internal State errors currently return without a Journal record.

The [audit inventory](docs/adapter-error-audit.md) identifies concrete source locations across all
first-party adapter families and adjacent application/transport boundaries. It is the completion
checklist for this RFC, not evidence that those locations have already been fixed.

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

Introduce a small `mfm-diagnostics` crate for checked causal data and its exact schema. Its purpose
is to remove duplicated representation, redaction accounting, and bounds validation across ports.

```text
diagnostics -> values -> canonical / ids

program, domains, store, config, signing -> diagnostics
live adapters -> their ports and concrete client libraries
```

The crate depends on existing foundational/serialization facilities, not Program, Runtime, client
libraries, or storage. It performs no IO and owns no error registry or recovery behavior.

Normal nested value derives require schema participation, not just Serde. Define the shared
representation's `MfmValue` and `PersistedSchema` contracts once. Do not repeat a handwritten
diagnostic schema in every domain or add optional schema implementations that can drift.

### 5.1 Shape and bounds

The conceptual shared shape is:

```rust
struct CauseChain {
    // Private checked fields; outermost exposed source first.
    head: CauseFact,
    tail: Vec<CauseFact>,
    omissions: OmittedEvidence,
}

enum CauseFact {
    HttpStatus { status: HttpStatusCode },
    RpcError { code: i64 },
    Transport { kind: TransportFailureKind },
    Database { kind: DatabaseFailureKind, sqlstate: Option<SqlState> },
    Os { kind: OsFailureKind, code: Option<i32> },
    Parse { category: ParseCategory, location: ParseLocation },
    Size { limit: u64, observed: ObservedSize },
    Task { outcome: TaskFailureKind },
    Channel { outcome: ChannelFailureKind },
    OpaqueSource,
}
```

The sketch names categories, not a mandate to expose every helper type publicly. Facts use closed
enums, checked codes, and integers. There is no arbitrary string/map escape hatch. Local checked
failure details stay in their owning error types where this avoids centralizing domain vocabulary.

Fixed bounds are 32 source layers and 8 KiB of encoded causal data, including omission
metadata. Constructors, capture, and deserialization enforce them. Bounded capture retains the
available outer chain prefix and records the limit; it does not claim an omitted root survived.
Capture must stop within its own bound even for a pathological source chain.

Client-specific capture helpers live beside the client. Walk exposed source layers using reviewed
structured extraction. An opaque layer must not hide an accessible deeper source. Do not format a
source to infer its identity or route a classifier by matching its message.

### 5.2 Existing typed wrappers retain operation context

Do not put every EVM, SQL, signer, or Runtime operation into one global stage enum. The source owner
retains its own operation/stage vocabulary:

```rust
struct ProviderFailure {
    method: EvmRpcMethod,
    stage: RpcStage,
    causes: CauseChain,
}

enum EvmOperationalError {
    Unavailable { source: ProviderFailure },
    Timeout { source: ProviderFailure },
    RateLimited { source: ProviderFailure },
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

These sketches preserve the current semantic categories while adding evidence. Do not introduce
a second `Classification` field that can disagree with the intrinsic classifier.

Use ordinary nested sources and borrowing. Remove incidental `Copy` requirements when errors gain
owned data. `Error::source()` should preserve a nested typed source when its type supports that
trait, but generic capability contracts remain usable with operational `MfmValue` types that do
not implement `std::error::Error`. Typed payload access remains part of the contract.

## 6. Example: commit the data, then classify and handle

The application-facing idea is ordinary typed execution data paired with its context. A balance
Read can be sketched as:

```rust
struct BalanceExecution {
    // The observation retains accepted evidence and the resulting balance on success.
    // BalanceError retains the concrete causal chain on failure.
    result: Result<BalanceObservation, BalanceError>,
}

struct BalanceExecutionContext {
    input: BalanceInput,
    intent: BalanceIntent,
}
```

The exact production types must distinguish domain, adapter, and internal errors and retain the
Read's checked evidence. Those distinctions belong in the declared result/phase types; this sketch
does not prescribe new generic `StateFailure`, `Incident`, or `StateResult` wrappers for authors to
construct around an already typed result.

Suppose the RPC source returned HTTP 200 and JSON-RPC error code -32000. The adapter constructs its
reviewed nested cause before consuming the client response. The State execution data now contains:

```text
result: Err(
    Provider {
        method: GetBalance,
        stage: RpcResponse,
        causes: [HttpStatus(200), RpcError(-32000)],
        omissions: message/data withheld
    }
)
```

The code is evidence; this example assigns no universal meaning to that numeric RPC code.
Runtime's sequencing is conceptually:

```rust
// This is the same call for Ok and Err. The committed result retains both inputs.
let committed = journal.commit(&state_data, &ctx).await?;

match committed.state_data().result() {
    Ok(observation) => {
        advance(observation.output());
    }
    Err(error) => {
        // This branch illustrates a recoverable-path domain/adapter error.
        // Internal errors end the invocation without invoking the user handler.
        let classification = error.classify();
        let evaluation = handler(&params, classification, committed.recovery_context())
            .and_then(|request| runtime.authorize(request, &committed));
        let recovery_data = RecoveryExecution { result: evaluation };

        // Runtime supplies a serializable recovery context linked to this failure.
        // Its append authority uses the new head, not the pre-outcome head.
        let recovery_ctx = runtime.recovery_execution_context(&committed);
        let recovered = journal.commit(&recovery_data, &recovery_ctx).await?;
        dispatch_committed_recovery(recovered);
    }
}
```

The sketch omits concrete stage-preserving error conversions. Handler, authorization, and
report-construction errors become the typed error in recovery execution data rather than returning
before that data can be committed. Dispatch executes a committed decision or ends the invocation
on a committed recovery Fault; it never recursively routes that Fault to a handler (section 7.4).

There is no separate `commit_failure` call and no transformation of the original error into a
summary for storage. `committed.state_data()` still contains the complete reviewed typed error
chain after `classify()` reads it. The handler receives only the projection it needs.

The classifier remains a pure projection:

```rust
impl ClassifyError for EvmOperationalError {
    fn classify(&self) -> Classification {
        match self {
            Self::Unavailable { .. }
            | Self::Timeout { .. }
            | Self::RateLimited { .. } => Classification::Retryable,
        }
    }
}

fn handle(
    params: &HandlerParams,
    classification: Classification,
    recovery: &RecoveryContext<'_>,
) -> Result<RecoveryRequest, StateExecutionError>;
```

This is the existing duplicate-safe Read policy, not the transaction Effect classifier. An
Operation supplies the handler default; a State occurrence can override it. No handler needs to
understand `BalanceError` to apply this common policy. Remove `IncidentSummary` construction and
its summary-only source plumbing; Runtime retains origin in the typed result for validation and
reporting. Do not copy origin into a second persisted summary. Any policy-facing origin accessor
must be a projection of that existing data, not an independently supplied fact.

The base `ctx` above is not the current fallible `State::adapter_context` reporting callback.
Replace that mandatory incident-context callback with the explicit persisted execution/context
fields. The original input, intent, result, and source facts must already be present without asking
another callback to assemble an incident. Remove redundant `AdapterContext` types and mappings
that repackage these facts; do not recreate them as a postcommit audit callback.

If a State needs additional domain-specific explanatory facts, give those facts an explicit owner
and field in its execution data or context. Ordinary State computation may produce such fields,
but there is no generic callback that decides what was important after the execution finished.
The schema review must show where every previously retained adapter-context fact now resides.

For a State-owned failure such as `AnchorChanged`, retain the exact typed expected/observed facts
in the error and the associated execution context. Its intrinsic `InputInvalidated` classification
remains a separate read-only projection.

### 6.1 Timeout evidence must not become permission by accident

| Observation | Retained distinction | Recovery consequence in this cutover |
| --- | --- | --- |
| Client deadline while sending | Local timeout, send stage, exposed sources | Duplicate-safe Reads retain their existing classification; transaction uncertainty is not erased. |
| Client deadline while reading body | Local timeout, body stage, status if already received | Receipt of headers does not prove whether a mutation executed. |
| Server HTTP/RPC error response | Actual status/code and response stage | A server error alone is not universal proof of nonacceptance. |
| Signer failure before prepared-wire retention | Signer/owner cause and transaction stage | Preserve the existing pre-retention classification. |

Case-by-case classification refinements can be reviewed against the richer evidence later. They
must revise the relevant exact error contract if semantics change. This RFC adds no `RecoveryAdvice`
and no provider-message-based retry rule.

## 7. Commit outcomes before handing off control

### 7.1 One commit operation over declared data

The rule is: finish the current execution step, commit its execution data and context, then hand
control to its successor. “Transition” includes handing a failure to recovery; it does not only
mean incrementing the State index.

```rust
// Runtime-owned commit façade; section 16 specifies its private typed contract.
let committed = journal.commit(&state_data, &ctx).await?;
```

The two inputs stay separate in memory and are committed atomically together. The operation handles
success and failure identically as persistence: qualify the declared data, construct the exact
Journal frame, validate its proposed fold, and append at the exact head. Neither context nor result
can be acknowledged independently. If the second value fails serialization or qualification,
there is no partial first-value append.

Use the existing ownership boundaries even if the call site names the facade `journal`:

- Runtime associates the concrete types and owns execution position, phase, expected head,
  qualification orchestration, pre-append semantic validation, and control-flow handoff.
- Journal encodes and qualifies the exact frame and complete local object closure.
- Store performs the mechanical atomic exact-head append.

Do not move Store IO into State implementations or make the Journal crate interpret domain values.
`commit` accepts the selected execution's exact declared data/context contracts, not arbitrary
`Serialize` objects or a mutable application object graph. Serializing an executable State, callback,
provider client, signer, or connection is outside this API.

The result remains typed: `Result<Output, Error>` or an equivalent existing tagged representation.
Error types retain source distinctions and the original causal chain. Removing a redundant incident
wrapper does not remove failure schema qualification or allow cold reconstruction to guess whether
a value represents success. Program and Runtime still associate exact success/error/context schemas.

Mode-specific data retains the Read's intent and accepted evidence, or the Effect's actual
prepared/pending/settled facts. These remain typed legal combinations. They do not become a bag of
optional fields or a second generic envelope that authors must manually fill in. In particular,
adapter failure cannot fabricate evidence and internal failure cannot masquerade as a domain result.

A successful commit advances to the next State or establishes terminal success; there is no
separate success-transition append. A committed domain/adapter error enters `AwaitingRecovery`,
retaining the original result, context, and mode facts. This commit contains no speculative handler
decision and spends no recovery allowance.

### 7.2 What serialization must preserve

Serialization is a reviewed persistence contract, not an incidental implementation detail. For
each execution-data and context type, define the following placement explicitly:

| Data | Authoritative retained location |
| --- | --- |
| The typed success/error result and produced State facts | State execution data |
| Inputs and admitted context used to obtain the result | `ctx`, unless a field is already authoritatively retained in the execution data |
| Prepared intent/command and accepted evidence needed to explain the result | The applicable typed execution/context variant, under Runtime's binding checks |
| Run identity, sequence, predecessor head, State position, and authoritative phase | Runtime-validated frame metadata or the applicable qualified mode value; do not accept conflicting caller copies |
| The original error chain and omission accounting | The typed error value, never only its classification or display text |
| Authorized recovery decision, evaluation Fault, and required terminal mapping/report facts | Subsequent recovery execution data linked to the committed original |
| Connections, callbacks, credentials, private signing material, and ambient services | Outside the persisted execution/context contract |

Persist the exact context used for the completed step. Freeze or borrow immutable values through
qualification and append; do not serialize a later-mutated context beside an earlier result. Only
the acknowledged success output becomes the next step's input. A failed append provides no
permission to advance an in-memory cursor as though the corresponding history existed.

Each fact has one semantic authority. Avoid storing separately supplied copies of input, source,
classification, or position that can disagree. This does not introduce a delta format, cross-frame
object cache, or new reference-resolution mechanism: retain current canonical value and frame-local
closure rules, including repeated bytes where those rules require them. Any future deduplication
change needs its own persistence design.

Derives or custom serializers may implement the wire representation, but must obey these rules:

1. Keep success/error tags, outputs, causal sources, required context, and Effect-authority facts.
   A `#[serde(skip)]` on the only error source or required execution input violates the contract.
2. Exclude runtime-only data by type separation where possible. Any deliberate serialization
   exclusion must be documented and either irrelevant to audit/reconstruction, derivable from
   retained authoritative data, or accounted for as withheld/unavailable evidence.
3. Check deserialization and exact schema shape together. Do not let skipped/defaulted fields,
   arbitrary JSON maps, or permissive custom decoding silently produce a different result or phase.
4. Retain canonical encoding, secret exclusion, and complete value/frame/run bounds. Serialization
   cannot recover a source the adapter already discarded, nor make arbitrary client text safe.
5. Prove that serializing and reconstructing both inputs preserves the reviewed result, error
   chain, execution context, and continuation decision. Identical admitted data must produce the
   same canonical bytes; semantically relevant changes must affect the committed representation.

The contract is complete auditability of committed boundaries. Missing admission, invalid history,
failed storage, finite capacity, and interruption still have the explicit limits in section 10.
A JSON serialization success by itself proves neither qualification nor durable insertion.

### 7.3 Commit before policy-dependent processing

The first commit requires only the exact execution data and its already available checked context.
Do not construct a new incident or run fallible reporting callbacks merely to decide what to store.
Commit before classification, handler execution, root failure mapping, and report construction so
these later callbacks cannot erase an acquired original cause. Remove mandatory adapter-context
assembly from this lifecycle; the persisted execution/context schemas already own those facts.

Qualification necessary for the first commit still happens before its append. If the original
cannot qualify, bounded internal execution data may account for the rejected original explicitly.
Never bypass bounds or secret checks to retain it. If even that safe representation cannot be
constructed and committed, return the available primary and recording errors without recursion.

### 7.4 Recovery uses the same commit operation

For a committed domain or adapter failure, Runtime evaluates the selected recovery policy from
that retained execution and context. This work includes intrinsic classification, the static
handler, authorization, and any root mapping needed for terminal reporting. It does not reconstruct
an incident context that should already have been committed.

```rust
struct RecoveryExecution {
    result: Result<AuthorizedRecovery, RecoveryFailure>,
}

struct RecoveryFailure {
    stage: RecoveryStage,
    diagnostic: InternalFailure,
}

// Its checked context links to the original execution and current expected head.
let committed_recovery = journal.commit(&recovery_data, &recovery_ctx).await?;
```

The success case is called a Decision below, and the typed error case a Fault. These names describe
the two recovery outcomes; they do not require authors to construct a separate Journal result
wrapper. Required terminal mapping/report facts belong in the applicable authorized result variant.

The recovery context identifies the one unresolved committed execution at this cursor in the
same qualified run, with its original sequence/reference and the current expected head. It cannot
reuse stale pre-outcome append metadata. This is an explicit record link, not arbitrary cross-
history object lookup. Each frame retains complete local closure for its own value objects.

The same commit facade serializes recovery data/context and validates their applicable record kind.
Uniform authoring does not make execution data interchangeable with a recovery decision: the
associated contracts and sole fold retain that distinction without exposing separate success/error
commit APIs or requiring authors to construct Journal wire records.

A Decision records the final authorized Retry, Restart, or Stop, with applicable root/report facts.
Only known insertion can spend recovery allowance, change a visit/checkpoint, establish a terminal
failure, or release a recovery action. Retain existing phase restrictions and Stop semantics:
Stop for a pending Effect ends the invocation and keeps its command; it does not manufacture
settlement. A pending-Effect Retry spends the existing allowance and yields with the same command.

A Fault records an internal failure of recovery evaluation and its stage. The primary error remains
in the linked State outcome. A Fault ends the invocation, keeps recovery unresolved, and is never
classified or recursively handed to the same handler. A later explicit resume may reevaluate that
unresolved recovery while admitted capacity remains. It does not execute the failed State again.

If encoding or appending the recovery result fails, return the causal failure and recording status.
Do not attempt to recover the failed recording through another recovery invocation.

### 7.5 Internal State results

Preparation, interpretation, adapter invariant, and hot qualification errors use the internal case
of the State outcome. Recording them preserves execution identity and ends the invocation. There
is no user-handler call for an internal execution failure, no invented `Permanent` domain error,
and no automatic Retry/Restart. Explicit resume can make another attempt only where the actual
execution phase and existing authority permit it.

This replaces the separate internal audit event and `latest_internal_failure` side channel from
the earlier proposal. Internal outcomes participate in the ordinary run history and outcome view.
They do not silently become terminal domain failure reports, nor do they advance to the next State.

### 7.6 Effect authority is a prerequisite, not a success transition

The command still commits before external IO. That commit authorizes entry to the Effect adapter;
a commit after the adapter returns cannot replace it.

```text
commit command + EffectId
    -> execute Effect adapter
    -> commit State outcome
        -> success: advance
        -> operational failure: evaluate recovery, commit decision, act
        -> internal failure: end invocation with retained authority
```

An actual `Pending` response remains an unchanged polling result. It is neither a failed State nor
permission to append invented evidence.

Distinguish an unresolved command from accepted settlement. If accepted settlement evidence is
followed by a domain or internal interpretation failure, the outcome must retain that evidence
and the settled phase. A later recovery fault cannot turn it back into an unexecuted command.

After a committed settled outcome, never reenter the Effect adapter to recreate the observation.
A settled domain failure permits only the existing authorized terminal handling. If internal
interpretation needs a later explicit attempt, use the retained evidence to reevaluate deterministic
interpretation, not another external execution. The EffectId and command remain correlated audit
facts, not fresh execution authority.

These distinctions belong in tagged mode outcomes and the sole Runtime fold. A generic `Failure`
without execution phase would be smaller syntactically but would lose a necessary safety contract.

### 7.7 Crash boundaries and cold reconstruction

| Last known committed point | Read-only reconstruction | Next explicit progression |
| --- | --- | --- |
| Before the State outcome | Restore the preceding authorized execution position | Execute according to that position; a physical attempt may have occurred without a committed outcome |
| Success outcome | Restore the next State or terminal success | Execute only the next authorized State |
| Domain/adapter failure outcome | Restore `AwaitingRecovery` and the original error | Evaluate recovery; do not repeat the failed State/adapter |
| Recovery Fault | Restore the original failure and latest recovery fault | Reevaluate unresolved recovery within admitted bounds; no State/adapter reentry |
| Recovery Decision | Restore the exact authorized action and counters | Follow that action without deciding again |
| Internal State outcome | Restore the internal failure and retained execution facts | Only explicit phase-permitted reentry; reuse settled evidence instead of rerunning an Effect adapter |

Read-only reconstruction never invokes classifiers, handlers, adapters, or failed
interpretation callbacks. Progress after a committed failure but before a committed decision is
new recovery work, not replay of a previously decided action. A crash before decision insertion can
therefore cause deterministic recovery evaluation to run again; committed decisions are final.

A checkpoint restart creates a new authorized visit and does not erase the failed outcome. Pending
Effect retries retain the same command and EffectId. Earlier outcomes and recovery results remain
available in Journal after later success.

Expose pending recovery, the original cause, and any recovery fault through the ordinary qualified
run view. Historical failure evidence must be labeled with position and sequence so it is not
mistaken for the current terminal status. Do not add a second internal-only observation mechanism.

### 7.8 Current contracts and proposed machinery to replace

The current implementation fuses failure and decision into one frame. Replace the nested decision
parts of `DomainConclusion`, `ReadConclusion::AdapterFailed`, and pending Effect failure records
with a committed failure outcome and its linked recovery result. Success needs no second append.

The old assertion that cold reconstruction never executes a handler must distinguish read-only
folding from progression of a newly exposed `AwaitingRecovery` cursor. The former stays callback-
free; the latter executes only recovery that has no committed decision yet.

The adapter rule in `AGENTS.md` currently says local mismatch has “no provider call or append.”
Implementation must revise it to permit an internal State outcome for a qualified admitted cursor,
while still forbidding provider entry and fabricated external evidence. The current internal-error
no-append contract changes accordingly.

Delete the earlier RFC's proposed `InvocationFailed`, `InternalFailureBounds`, separate internal
counter, and `RunView.latest_internal_failure` design rather than retaining it alongside outcomes.
Remove `IncidentSummary` and its construction plumbing, the mandatory `adapter_context` callback
and redundant `AdapterContext` types, and mandatory `Incident` or `StateFailure` wrappers used
solely to assemble persistence data. Keep concrete error contracts and typed success/error tags.
Replace the redundant wrappers with direct serialized execution data and context,
not another generic payload bag around the same fields.

The unavoidable addition is the explicit recovery-pending cursor and its result, not a parallel
audit pipeline. This proposal trades an additional commit on recovery paths for an original failure
that is durable before policy code runs.

## 8. Bound execution and recovery together

Keep semantic recovery decisions separate from storage repetition. Add this Program-wide contract:

```rust
struct ExecutionCapacity {
    explicit_reattempts: u32,
    recovery_reevaluations: u32,
}

struct StateCommitBounds {
    outcome: ConclusionBound,   // complete data + context + local closure
    recovery: ConclusionBound,  // larger of Decision and Fault, including mapped root
}

struct EffectCommitBounds {
    prepare: ConclusionBound,
    execution: StateCommitBounds,
}
```

`ConclusionBound` retains its checked nonzero byte-bound role; rename it `FrameBound` in this
cutover because preparation and recovery are not conclusions. Pure/Read declare `StateCommitBounds`;
Effect declares `EffectCommitBounds`. There is no per-Effect failure counter. Keep
`ProgramLimits::max_recovery_decisions` (`G`) and add `execution_capacity` to the same admitted
limits. Shipping Program constructors explicitly supply **8 explicit reattempts (`X`) and 8 recovery
reevaluations (`Y`)**. Zero is legal for either: it disables extra attempts, never the first result
of already admitted work. No hidden Runtime default or mutable resume override exists.

For State `i`, let `P_i` be its preparation bound (zero except Effect), `O_i` its outcome bound,
`R_i` its recovery bound, `L_i = P_i + O_i + R_i`, and `k_i = 2 + is_effect(i)`. Admission computes:

```text
nonempty Program:
  frames = 1 + (G + 1) * sum(k_i) + X * max(k_i) + Y
  bytes  = genesis + (G + 1) * sum(L_i) + X * max(L_i) + Y * max(R_i)

empty Program:
  frames = 1
  bytes  = genesis
```

This conservatively reserves a whole sequence for every authorized recovery decision, a whole
largest State lifecycle for each explicit reattempt, and a largest recovery result for each
reevaluation. Use checked addition/multiplication and the existing object, descriptor, frame-count,
frame-byte, and run-byte ceilings. Overflow or excess is an admission error, never saturation.
For example, one Pure State with `G=0, X=8, Y=8` reserves 27 frames; one Effect reserves 36. A normal
success does not append unused reservations. A zero-retry failure still has its first Stop slot.

Charge counters only in the sole fold of committed records:

| Work | Reservation and charge |
| --- | --- |
| Initial or recovery-authorized execution | Existing sequence/recovery reservation; no extra counter |
| Explicit reentry after Internal or pending Stop | Check `X` before work; charge on its first committed record |
| Reentry that prepares an Effect | Preparation spends `X` once; its later outcome/recovery use the reserved remainder |
| Reentry of a pending Effect or settled interpretation | Outcome spends `X`; no second preparation or charge |
| First recovery evaluation for a committed failure | Uses the result slot reserved by that outcome; no `Y` charge |
| Evaluation after a committed recovery Fault | Check `Y` first; its Decision or Fault spends one `Y` |
| Adapter returns Pending, or interruption before any commit | No new record or charge |

The preceding cursor establishes the charge; do not accept author-supplied attempt IDs or flags.
Internal or pending Stop establishes `RequiresExplicitReentry`; a recovery Fault retains
`AwaitingRecovery` with its last Fault. Initial recovery stays available even after extra counters
are exhausted. A committed preparation retains enough capacity to finish its admitted lifecycle.
Checks precede IO and callbacks. Exhaustion returns the qualified observation without entering
further work. Storage slots never grant Retry/Restart permission or enlarge the immutable Program.

Bounds include exact context snapshots, outcome alternatives, retained evidence, mapped roots,
diagnostics, and complete frame-local closure. Do not substitute the 8 KiB causal bound for a
complete frame bound. Size the maximum concrete schemas as well as payloads: current accumulated
context descriptors are already near the 64 KiB descriptor ceiling. Use the existing schema
composition rules and reject excess; do not quietly raise ceilings, skip context, or introduce
cross-frame references to make an example fit. Update each shipping bound and its consuming maximum
workload test in the same commit. Accepted storage cost does not waive finite admission.

## 9. When recording fails or another caller wins

Preserve the primary failure and recording outcome separately. A Store failure during audit is a
second failure, not a replacement cause and not evidence that the original mutation did not happen.

The conceptual invocation detail is a flat sum:

```rust
enum FailureAudit {
    Committed { sequence: u64 },
    NotCommitted { reason: RecordingFailure },
    Indeterminate { source: ReviewedStoreFailure },
}
```

Attach it to the existing invocation failure alongside the primary error, any qualified original
incident, and `last_observed`. Absence of an audit attempt is distinct from these attempted-record
outcomes. Use tagged variants/private construction instead of a status plus loosely correlated
optional errors. There is no recursive `RuntimeError` audit tree.

| Outcome | Required returned evidence and behavior |
| --- | --- |
| Inserted | Return the failure with its committed sequence and newly qualified observation. |
| Definite rejection | Retain primary plus recording cause; identify the candidate as not committed. |
| Ambiguous acknowledgement | Preserve Indeterminate, recovery identity, and the last qualified observation. Do not assert insertion or noninsertion. |
| Another exact-head candidate wins | Reload the winning history; return this caller's original cause as uncommitted evidence alongside that observation. |
| State outcome construction exceeds bounds | Retain the primary and size/qualification cause; do not claim the original was recorded. A bounded internal outcome is eligible only if it can itself be prepared safely. |
| Recovery construction fails | The original failure is already durable. Record a bounded recovery Fault when safely representable and within reserved capacity. |
| Internal outcome or recovery Fault construction/append fails | Return available primary and secondary evidence. Do not recursively attempt another error record. |

A losing failed candidate must not reenter the provider, silently rebase its append, or return only
the winning view and discard its own cause. This explicitly changes that losing-failure response;
the winner's qualified history remains authoritative.

## 10. Same-Store audit limits

| Failure boundary | Persistence contract |
| --- | --- |
| Domain/adapter failure with an admitted cursor | Execution data and context commit the original before recovery evaluation; linked recovery data retain the authorized decision or fault. |
| Internal hot execution failure with an admitted cursor | Internal State outcome, subject to qualification, capacity, and known insertion; no user-handler recovery. |
| Before genesis, including deployment and bootstrap | Return reviewed causal evidence. There is no admitted State history to append to. |
| Store load failure or invalid/unqualified history | Return causal evidence. Do not append using an untrusted or unavailable head. |
| Store append failure | Return primary and persistence causes, preserving definite/indeterminate semantics. Do not recursively audit through that failed Store. |
| Config/index/provisioning outside run execution | Preserve the full reviewed port/invocation cause; do not create a synthetic run to audit it. |
| Response delivery after a commit | Retain the committed outcome and delivery cause in the invocation boundary; do not reopen a terminal State. |
| Cancellation or process death before append acknowledgement | No guarantee that the physical attempt has a committed failure record. |

No independent spool or later automatic replay is implied. A failure that cannot be stored is not
durably audited merely because a causal error was returned. Total storage/process failure cannot
be repaired by another error wrapper.

## 11. Boundary implementation and deletion inventory

### 11.1 EVM transport and execution

- Replace RPC envelope disposal with reviewed code/status capture and explicit message/data
  withholding. Use checked envelope decoding that preserves the actual parse/protocol failure
  instead of collapsing failures through an untagged fallback.
- Capture send/body source chains before consuming `reqwest::Error`. Preserve already received
  HTTP facts if subsequent body handling fails.
- Process non-success bodies within the existing response-size/deadline limits. Do not introduce
  unbounded buffering, new requests, or transport retries for diagnostic capture.
- Retain malformed quantity/hash/receipt stage, missing-field category, and known size facts.
- Preserve provider, custody, and signer causes through transaction wrapping, including the
  originating transaction operation and checked mismatch facts.
- Replace unit invariant construction with reviewed internal provenance. Registration bridges that
  already preserve a cause should continue to pass it through rather than adding duplicate layers.
- Revise exact operational schemas and their consuming bounds/fixtures together.

### 11.2 PostgreSQL Store, index, config, custody, gates, and provisioning

- Replace generic `unavailable(_: impl Sized)`, `internal(_: impl Sized)`, and equivalent source
  sinks with one SQLx-specific capture owner and explicit port-level dispositions.
- Preserve SQLSTATE and exposed source layers while retaining current precommit, database rejection,
  and ambiguous COMMIT classification. Never infer noncommit from a network timeout.
- Preserve failed schema/role/epoch/durability check identifiers without copying database detail,
  credentials, SQL parameters, or private locator values.
- Remove `Protocol` marker construction and substring-based gate routing.

SQLx 0.9.0's pool catches errors from `after_connect`, logs their display value, closes the
connection, and retries. Merely boxing a typed gate error in `sqlx::Error` does not make that cause
reach the caller; a later pool timeout can replace it.

Move fallible reviewed gates into owned acquisition functions. Each function validates the same
acquired connection before protected IO, returns its typed gate cause directly, and disposes of a
failed connection. Cut over every acquisition, including custody's separate pool. Keep split-role,
authority-epoch, snapshot, transaction, and ambient-input contracts intact.

Do not add a concurrent “last callback error” registry. A different acquisition could overwrite it,
so it cannot establish the cause of this invocation. If SQLx internally hides connection-attempt
details that never reach an owned capture point, mark those unavailable rather than fabricate them.

### 11.3 Signing, keystore, and memory implementations

- Replace unit thread/channel/owner/signing translations with reviewed causal variants and preserve
  them through the EVM preparation adapter.
- Inspect safe channel failure facts without retaining the rejected command. In particular, an
  import command can own a private scalar.
- Represent an opaque cryptographic error honestly. Do not invent details or expose key material.
- Preserve task/allocation/physical-validation causes in memory Store/index paths. Ordinary absence
  in memory configuration remains a valid outcome.
- Keep the keystore non-`Send`/non-`Sync` ownership design and strengthen secret-exclusion tests.

### 11.4 Runtime, application, and transports

- Replace source-erasing hot preparation, interpretation, codec, callback, and Journal translations
  with reviewed causal carriers. Preserve errors acquired before a later callback or append fails.
- Extend existing CLI/REST structured error envelopes with bounded safe causal detail and recording
  status where applicable. Retain current public codes, status policy, and exit-code policy.
- Preserve filesystem, environment, UTF-8/TOML/JSON parsing, entropy, listener, and response delivery
  causes. Do not expose source input to make these errors more descriptive.
- Keep rendering a projection of the retained error. A failed attempt to print an error must not
  replace the primary failure or claim the operation did not commit.
- Remove `IncidentSummary` from handler signatures and call sites. Pass `Classification` directly
  with Runtime-owned recovery context; keep the original in committed execution data.
- Remove persistence-only incident assembly, mandatory adapter-context callbacks, and redundant
  context types. Move any unique required facts into their explicit execution/context schema owner;
  prove the new representation retains every fact formerly supplied by those callbacks.
- Add no diagnostics endpoint, transport-specific recovery, or alternate composition path.

### 11.5 Development-only real IO

Fix causal capture in the existing E2E funding helper as part of this inventory. Do not defer its
source-erasing pattern to a future production adapter, expose new production APIs solely for test
convenience, or change funding retry semantics. Building the planned development funding adapter
remains a separate feature.

## 12. Complexity budget and rejected alternatives

The implementation must remove the old lossy plumbing rather than place wrappers around it.

| Necessary addition | Complexity it replaces or responsibility it makes explicit |
| --- | --- |
| One checked causal-data contract | Duplicated ad hoc capture, omission, and bounds conventions across ports. |
| Richer existing typed errors | Unit replacement helpers and repeated loss of source categories. |
| Owned PostgreSQL acquisition gates | Callback marker routing and the false assumption that callback errors propagate. |
| One `commit(state_data, ctx)` operation | Separate success/error commit APIs, persistence-only incident assembly, and internal audit events. |
| Exact execution/context serialization contracts | Ad hoc field selection and redundant independently supplied copies of causal/phase facts. |
| Direct classification argument to the handler | `IncidentSummary` and summary-only construction/source plumbing. |
| One linked recovery result and pending cursor | Fused decision fields and loss of an original cause when policy work fails. |
| Complete execution/recovery lifecycle bounds | Internal-only audit allowances and unaccounted repeat evaluation growth. |
| Explicit recording outcome | Ambiguous conflation of the primary failure, audit failure, and winning history. |

Do not add a universal exception bag, arbitrary extension fields, error registry, per-State handler
family, secondary logger, raw archive, replay service, or compatibility adapter. Do not rewrite
unrelated success paths to make the error cutover appear more comprehensive.

LOC is evidence, not a quota. Report production LOC removed from source-loss plumbing separately
from causal capture, persistence, and validation additions. Also report changes in public types,
configuration points, code paths, and future change sites. A net increase needs an explanation;
compressing code or weakening tests does not count as simplification.

## 13. Contract cutover and logical commits

Use one current contract. Program v6 becomes v7 for the revised lifecycle bounds, and Journal frame
v4 becomes v5 for the execution/context and recovery records and adjacency. Revise affected exact
execution, context, and error schemas, and handler implementation identities for the direct-
classification signature, together with their consumers. Do not bump an unrelated envelope
merely because a nested exact error contract changes.

Store continues to load complete opaque prefixes and atomically append exact-head frames. No
separate audit table or PostgreSQL schema migration follows from adding a Journal record. Do not
rewrite acknowledged histories or introduce legacy readers; superseded formats are rejected.

The ordered implementation commits are:

1. **`preserve rpc and evm error provenance`**: shared causal data, EVM capture/contracts, direct
   consumers, exact identities, and error-aware bounds.
2. **`preserve postgres error provenance`**: storage/custody/config/index port carriers, SQLx capture,
   acquisition gates, provisioning, and all affected consumers.
3. **`preserve signer and local io error chains`**: keystore/signing, memory, deployment, application,
   and transport propagation with strengthened secret tests.
4. **`commit execution data and context before recovery`**: inseparable uniform Runtime commit
   facade, exact serialization contracts, direct-classification handler API, Program/Journal wire,
   lifecycle bounds, fold, invocation, observation, and client-contract changes.
5. **`complete adapter error audit validation`**: final cross-boundary scenarios, inventory
   reconciliation, and handoff evidence. Funding-helper capture belongs in commit 1.

| Commit | Coherent completion boundary | Required focused evidence |
| --- | --- | --- |
| 1 | Add the diagnostics crate and its single schema/checked constructors; revise RPC/EVM errors and direct consumers under the existing runtime wire. Fix the funding helper now. Update exact error identities and current bounds wherever payloads grow. | A2, provider portion of A3, A6; fake HTTP server covers send/body/parse/code/status/size and omission facts. Cold round trip proves richer errors survive today's committed path. |
| 2 | Enrich Store/config/index/custody/provision errors and all conversions, including transaction wrappers. Replace all gate callback routing with checked owned acquisition. Keep the current Journal protocol working. | A4 and custody portion of A3; managed PostgreSQL exercises gate refusal before protected IO, SQLSTATE/decode failures, and definite versus ambiguous COMMIT. |
| 3 | Enrich signer/keystore/memory/application/transport errors and all consumers. Update public safe-detail schemas and source-preserving library access together. | A5 and signer portion of A3; channel/thread/task/crypto/local-IO injection, secret sentinels, public code/status/exit policy and response-delivery failures. |
| 4 | Replace State/handler signatures, derive Result support, assembly, Program v7, Journal v5, sole fold, bounds, views and clients in one cutover. Remove every old persistence/context/incident path from section 17. Update design, architecture, all fixtures and recovery validation. | A7–A12, A16–A25; consuming nested-Result tests, exact wire rejection, maximum-context admission, Read recovery Fault/resume, settled Effect interpretation reentry, cancellation and hostile Store conflicts/ambiguity. Run affected Program/Journal/Runtime/domain/app/client tests and managed Effect/client scenarios. |
| 5 | Reconcile every audit first-loss row with implementation location and evidence; add only missing cross-boundary regressions. Report removals/additions and explicit upstream limits. | A1 and A13–A15 end to end, A1–A25 traceability, final `nix run .#ci` on the exact candidate. A prior commit's missing unit/boundary test is not deferred here. |

Use `nix develop -c cargo test -p <affected-package> <filter>` while iterating and the managed tasks
in [build and verification](docs/build-and-verification.md) for service-backed cases. Do not substitute
ignored local tests for those managed scenarios. Commit 4 starts only after the causal carriers are
available; its API, wire, fold, capacity and client changes are inseparable. Each earlier commit
must update downstream consumers of its changed error types and remove superseded constructors.
If an error schema cannot fit current bounds, solve that coherently in the owning commit rather
than landing an admitted Program that cannot execute its declared error path.

Every commit includes its meaningful tests and documentation. The last commit is not permission to
defer a changed boundary's tests or leave its consumers lossy. Merge inseparable steps where a
public type cutover requires it; do not keep old constructors or unit-only fallbacks for sequencing.

Implementation updates `AGENTS.md`, the authoritative design/architecture documents, this RFC,
client contracts, recovery validation, the audit inventory, and known gaps consistently. The
existing recovery RFC remains the rationale for intrinsic classification and static handlers.

## 14. Acceptance criteria and verification

| ID | Evidence required |
| --- | --- |
| A1 | Two RPC errors with equal classifications retain distinguishable numeric codes and reviewed facts through the committed State outcome and cold reconstruction before any handler runs. |
| A2 | HTTP status, send/body timeout, parser location/category, malformed field, and response bound remain distinguishable; a secondary body failure does not erase status. |
| A3 | Provider/custody/signer causes retain transaction operation context through all relevant adapters; classification remains unchanged unless separately reviewed and versioned. |
| A4 | SQLx connect/query/decode/COMMIT failures retain reviewed sources and exact definite/indeterminate behavior. Acquisition-gate failures reach the caller without protected IO. |
| A5 | Thread/channel/owner/crypto and memory/task failures remain distinguishable. Synthetic secret sentinels are absent from canonical data, formatting, and client responses. |
| A6 | Constructor and deserializer tests enforce causal bounds and explicit withheld/opaque/unavailable/bounded metadata. No arbitrary diagnostic text path is accepted. |
| A7 | Success commits before successor entry; domain/adapter failure commits before any recovery callback. Internal State outcomes preserve execution identity and invoke no user handler. |
| A8 | Classifier/handler/root-mapping failures retain the already committed original/context and record a linked recovery Fault without recursive recovery. No mandatory adapter-context callback remains. |
| A9 | Every outcome/decision crash boundary has equivalent cold observation and correct resumed work. Read-only inspection runs no callbacks; progression reevaluates only unresolved recovery. |
| A10 | Rejection, append ambiguity, and exact-head conflict preserve primary and secondary causes with accurate recording status. A losing failure returns the winning observation without reentering the provider. |
| A11 | Maximum workloads fit complete outcome/recovery bounds; zero-retry failures still have Stop capacity, repeated Faults consume finite slots, exhaustion prevents work, and settlement reservation remains intact. |
| A12 | Oversized/unqualifiable originals and failed diagnostic writes never produce a false preservation claim or recursive audit loop. |
| A13 | Pre-admission, invalid-history, unavailable-Store, cancellation, and postcommit delivery scenarios obey the same-Store limits. Expected absence and Pending remain valid outcomes. |
| A14 | CLI/REST/library callers retain their reviewed public dispositions while receiving the causal and audit details permitted by their existing surface. |
| A15 | Every first lossy conversion in the audit inventory is traced to its replacement and consuming evidence. Remaining upstream visibility limits are explicit rather than marked fixed. |
| A16 | A committed settled Effect followed by interpretation or recovery failure never reenters its adapter; any permitted interpretation reentry uses retained evidence. |
| A17 | Decision ambiguity/conflict cannot spend a budget twice, repeat a completed handler decision, or dispatch an action before known insertion. |
| A18 | The same commit entry point retains Ok and Err execution data with their exact context. No success/error-specific author path or persistence-only incident construction is required. |
| A19 | Serialization/deserialization preserves result tags, nested causes, required context, and Effect phase. Exclusions are explicit; skipped/defaulted fields cannot erase required facts or substitute a different outcome. |
| A20 | Execution data/context commit atomically: failure qualifying either side appends nothing; mismatched position, phase, contract, or stale head cannot be acknowledged. Subsequent context mutation cannot change committed bytes. |
| A21 | Direct-classification handlers preserve Operation defaults, occurrence overrides, and existing recovery semantics while the committed original remains unchanged and accessible. |
| A22 | Every required fact formerly supplied by `adapter_context` is present under one declared execution/context owner; removing the callback does not remove intent, source position, or other audit evidence. |
| A23 | Resuming a committed failure with unresolved recovery requires the exact associated policy contracts. Incompatible assembly is rejected before callback execution; a committed decision is never replaced by a newly selected policy. |
| A24 | Preparation failure without intent, adapter failure without accepted evidence, and interpretation failure with accepted evidence each have a valid exact representation. Reconstruction rejects fabricated defaults and illegal result/phase combinations. |
| A25 | Maximum accumulated-context workloads retain the exact pre-execution context without introducing mandatory `Clone` bounds. Measure retained bytes and append counts for success, failure, and recovery Fault paths against the current implementation. |

Use boundary-focused tests through actual consuming APIs, not a second model of the error pipeline.
Use synthetic injected failures and fake servers; do not collect real secret-bearing diagnostics
as test evidence. Tests must preserve existing guarantees rather than replace them with narrower
assertions about new fields.

Follow [build and verification](docs/build-and-verification.md): focused affected-crate tests through
the default Nix shell, managed PostgreSQL and Effect/client tests for their affected boundaries,
and one final `nix run .#ci` on the exact implementation candidate. Update SQLx metadata only if
queries change. Avoid redundant broad gates immediately before CI.

Creating or revising this proposal is documentation-only and needs link/contract review and
`git diff --check`, not Rust CI.

## 15. Structural consequences

This proposal changes the durable execution protocol as well as the commit API. A uniform
`commit(state_data, ctx)` call can remove caller-side packaging. Committing an outcome before
recovery changes which histories are valid, what work remains after a crash, and what callers can
observe. Those consequences must be assessed independently of the method's short signature.

### 15.1 Recovery becomes a resumable execution phase

An original error can be committed while its recovery decision is still absent. That prefix is
valid and must identify recovery as the next work, rather than reentering the failed State. Program
remains an immutable linear State sequence; the intermediate execution phase belongs to Runtime's
existing sole fold, not a new authoring State or a second execution engine.

Recovery evaluation may run again after interruption if its result was never committed. Only the
committed decision advances counters or authorizes action. Handlers must therefore remain
deterministic and free of ambient IO; notification, provider mutation, or other external work
cannot be hidden inside policy evaluation. Read-only reconstruction remains callback-free.

### 15.2 Execution data is an explicit persisted interface

Every failure boundary must have a valid representation even when normal success fields do not
exist:

| Boundary | Available facts |
| --- | --- |
| Intent preparation failed | Input/context and an internal cause; no prepared intent |
| Adapter failed | Prepared intent or command and its causal error; no accepted evidence |
| Interpretation failed | Accepted evidence and its binding, context, and an internal cause; no successful output |
| Execution succeeded | The applicable checked evidence and successful output |

Do not fill missing facts with defaults to make every record fit one struct. Typed result/phase
variants must preserve these differences while the commit API stays uniform. This is a stronger
contract than deriving serialization on whatever temporary struct happens to exist today.

### 15.3 Serialization and policy identity affect execution correctness

Omitting a field can change how a run resumes, not merely reduce the detail in its audit record.
Exact schema, canonical encoding, result tags, and field meaning must therefore be reviewed as
execution contracts. Successfully deserializing old-shaped data is not evidence of equivalent
semantics. Required fields cannot disappear through `serde(skip)` or permissive defaults.

There is also a longer interval in which deployment changes can matter: a failure can be committed
before its handler ever runs. Resume must associate the exact policy contracts selected by the
Program. Changes to policy semantics require the corresponding implementation identity change;
an unchanged Rust function name or compatible-looking payload is insufficient. Contract identities
depend on this versioning discipline and do not independently prove arbitrary code equivalence.
Once a decision commits, a later deployment cannot replace it by evaluating a different policy.

### 15.4 A recorded error and a failed run are different observations

Callers need to distinguish an execution awaiting recovery, an authorized retry/restart, a terminal
failure, a stopped pending Effect, and an internal recovery Fault. RunView, CLI/REST responses, and
monitoring must not label every error entry as a terminal run failure or an immediately runnable
State.

Root mapping and terminal report construction become work after the original error is durable.
That preserves the cause if reporting fails, but also permits a run to remain unresolved at that
step. Repeated explicit progression can encounter the same reporting Fault within finite bounds.
The audit history must show that distinction without inventing a domain outcome.

### 15.5 Accepted Effect settlement separates external work from interpretation

If accepted settlement evidence commits with an interpretation failure, the remaining work is
deterministic processing of that evidence. A recovery or reporting error cannot make the external
operation unexecuted again. Explicit interpretation reentry reuses the evidence and never reenters
the Effect adapter to recreate it.

This distinction can simplify reconciliation, but only if the actual phase is retained and checked
by the sole fold. The generic commit entry point does not remove the need to distinguish prepared,
unresolved, and settled commands, or to commit authority before external IO.

### 15.6 Storage growth is accepted; capacity and ownership remain explicit

The additional storage cost is accepted. Retain the complete declared execution/context data and
the necessary recovery records without weakening audit evidence to reduce retained bytes. Do not
add deduplication, deltas, or selective history retention merely to justify this design economically.
Storage-cost benchmarking is not an approval gate for this RFC.

A recoverable failure now requires an outcome append followed by a recovery-result append. That
adds a durability operation, a contention point, and a crash boundary. Success still needs only its
outcome append, but retaining additional execution/context fields may increase its bytes too.

Measure bytes and record counts to establish correct finite admission bounds and preserve reserved
recovery/settlement capacity. These measurements establish that supported workloads fit the
declared limits; they do not reopen the accepted storage tradeoff. Additional commit boundaries
still require correctness tests for acknowledgement, interruption, and concurrency.

Accumulating contexts already repeat full snapshots across frames. Copying the growing context
into multiple independently supplied fields can produce quadratic retained-byte growth. Keeping
one semantic authority per fact does not automatically remove physical repetition required by
frame-local closure. This RFC introduces no deduplication or delta storage protocol.

Current State evaluation and interpretation consume typed input. Retaining the exact context used
by that execution requires preserving its qualified representation or arranging ownership before
the input is consumed. Prefer the existing immutable qualified bytes/shared ownership where
applicable; do not impose `Clone` on every State context or blindly duplicate accumulated payloads.

### 15.7 Uniform serialization does not establish transition authority

A value can match its schema and still describe an impossible execution transition. Runtime must
validate the current head, position, context/result pairing, evidence binding, Effect phase, and
recovery permissions. Journal qualifies the wire and Store performs the atomic append.

The simplification succeeds when these responsibilities have one owner and the new entry point
replaces duplicated paths. A facade wrapped around all the existing special cases would improve
the call site without reducing the underlying complexity.

### 15.8 Evidence needed before claiming a simpler implementation

The governing rule is simpler, and preserving the original error no longer depends on policy or
reporting success. A net reduction in implementation complexity or production LOC is not yet
proven. The new recovery phase and legal mode combinations may offset some deletions.

Before finalizing implementation, validate the sole-fold transition table with two consuming
scenarios: Read failure followed by recovery Fault and resumed decision; and Effect settlement
followed by interpretation failure and retained-evidence reentry. Compare public types, callbacks,
branches, append counts, retained bytes, and production LOC against the current design. This is
evidence for the selected protocol, not an invitation to keep a parallel prototype in production.

## 16. Engineering contract and core sketches

This section resolves the implementation choices left abstract in the introductory balance example.
The implementation target is Runtime-assembled data from existing typed State methods. State authors
own input/output/failure fields; they do not construct a framework incident, invoke Store, or select
a serialization callback after failure. Additional explanatory facts belong in those owned types.

### 16.1 State and recovery signatures

```rust
trait State: Send + Sync + 'static {
    type Input: MfmValue;
    type Output: MfmValue;
    type Failure: ClassifyError;
    fn state_id() -> Result<StableId>;
}

trait PureState: State {
    fn evaluate(input: Self::Input)
        -> Result<Result<Self::Output, Self::Failure>, StateExecutionError>;
}

trait ReadState<C: ReadCapability>: State {
    fn prepare(input: &Self::Input) -> Result<C::Intent, PreparationError>;
    fn interpret(input: Self::Input, evidence: &C::Evidence)
        -> Result<Result<Self::Output, Self::Failure>, StateExecutionError>;
}

// EffectState uses the same preparation/interpretation pattern with C::Command.
// Keep its existing capability bounds; neither mode declares AdapterContext.
trait Handler {
    type Params: MfmValue;
    fn handle(params: &Self::Params, classification: Classification,
              context: &RecoveryContext<'_>)
        -> Result<RecoveryRequest, StateExecutionError>;
}
```

The two `Result` layers distinguish internal failure from an ordinary typed domain result. Retain
separate `PreparationError` and `StateExecutionError` names for their existing API roles, but replace
their unit payloads with reviewed internal provenance. They wrap `InternalFailure`, a checked
`{ kind: InternalFailureKind, causes: CauseChain }` value. Its closed kinds distinguish local
invariant, preparation, interpretation, qualification, task failure, and capture failure; owner-local
errors carry any finer checked operation facts. Operational client errors stay in their existing
typed ports rather than entering this internal category. Intrinsic classification remains infallible
on a decoded typed error. Decode/association failures while reaching that classifier are recovery
Faults. Handler defaults remain Operation-owned with State-occurrence overrides; root `ValueMap`
association and recovery-target validation remain required.

Add `Result<T,E>` handling to `program-derive/src/shape.rs`, recursively using the existing external
enum schema for Serde's `{"Ok": value}` / `{"Err": value}` representation. Enclosing exact types own
semantic identities. Do not add a blanket semantic identity for every standard-library Result or
a new schema-language construct. Consuming derive tests cover nested results and rejection of
unknown/missing tags and invalid payloads.

### 16.2 Exact data and context ownership

Use separate concrete mode types. This sketch shows their required alternatives, with generic
parameters standing for the associated exact contracts, not dynamically typed payloads:

```rust
struct StateContext<I> { input: I }
struct PureData<O, F> { result: Result<Result<O, F>, StateExecutionError> }

enum ReadData<T, E, A, O, F> {
    PreparationFailed { error: PreparationError },
    AdapterFailed { intent: T, error: A },
    AdapterInternal { intent: T, error: InternalFailure },
    Interpreted {
        intent: T, evidence: E,
        result: Result<Result<O, F>, StateExecutionError>,
    },
    CaptureFailed { error: InternalFailure },
}

struct EffectPreparation<Q> { command: Q }
enum EffectData<E, A, O, F> {
    PreparationFailed { error: PreparationError },
    PendingFailed { error: A },
    PendingInternal { error: InternalFailure },
    Settled {
        evidence: E,
        result: Result<Result<O, F>, StateExecutionError>,
    },
    CaptureFailed { error: InternalFailure },
}

enum EffectContext<I, Q> {
    BeforePreparation { input: I },
    Prepared { input: I, effect_id: EffectId, command: Q },
}

struct RecoveryData<R> {
    result: Result<AuthorizedRecovery<R>, RecoveryFailure>,
}
struct RecoveryFailure { stage: RecoveryStage, diagnostic: InternalFailure }
enum RecoveryStage { Classification, Handler, Authorization, RootMapping, Encoding }

struct RecoveryExecutionContext<I> {
    input: I,
    failed_outcome: FrameRef,
    recovery: PersistedRecoveryContext,
}
```

`PureData`'s outer internal result also represents checked capture failure. `CaptureFailed` is the
bounded fallback for a Read/Effect value that cannot qualify and has no further qualified mode facts; its diagnostic explicitly says
which evidence is unavailable. It must not claim to retain an unqualified original or accepted
settlement. For Reads, retain qualified intent through `AdapterInternal`; retain qualified accepted
evidence and intent through `Interpreted` with an internal result. A later qualification failure
cannot discard those facts or mark them unavailable. Already qualified settlement facts cannot be
discarded into that fallback: use the
`Settled` internal alternative with retained evidence. Failure to encode that alternative returns
uncommitted evidence, not a fabricated pending record. Preparation/data/context variants are
validated against the current phase; sharing an enum is not permission to use every variant at every
cursor. No fabricated intent, command, evidence, output, or default error fills an absent field.

The frame envelope owns run sequence and State position. State context owns the exact pre-execution
input, including the accumulated context already carried by that input. Read data owns intent and
accepted evidence. Prepared Effect context owns command/EffectId; settled data owns evidence.
`PersistedRecoveryContext` is the exact serializable projection of Runtime's phase, recovery
allowances/usage, barrier and authorized targets used by the handler; no caller can supply those
facts independently. Recovery references the original outcome by exact frame identity in the
qualified prefix. It must carry its own complete value/schema closure; this reference is a causal
link, not a new object lookup or deduplication mechanism.

`AuthorizedRecovery<R>` is a private-construction sum: `Retry`, `Restart { target }`,
`StopDomain { reason, root: R }`, or `StopAdapter { reason }`. Each variant contains exactly the
existing checked authorization facts needed by the fold. The original remains in the outcome;
Stop does not copy it into a second incident. Root mapping runs only where existing policy requires
it. Pending Stop preserves command authority, whereas terminal Stop finishes the run. The current
phase determines the permitted variants. A Fault carries no authorized action.

Runtime preserves qualified input bytes before invoking a consuming State callback and decodes an
execution copy. Do not require `Input: Clone`. Immutable shared qualified values can support commit
construction and views; later mutation of user values cannot change committed bytes. Mode-specific
typed decoders are associated with the executable's exact contracts. They project data for the sole
fold without calling State, classifier, handler, or root mapper during cold reconstruction.

### 16.3 One commit façade and one acknowledged fold

The following methods are **private Runtime infrastructure**, not a new public authoring registry:

```rust
trait CommitData<C>: sealed::Sealed {
    // Qualify the selected exact mode/recovery contract and complete local closure.
    fn encode(&self, context: &C, current: &Accumulator) -> Result<EncodedRunFrame>;
}

impl RuntimeJournal<'_> {
    async fn commit<D, C>(&mut self, data: &D, ctx: &C)
        -> Result<CommittedStep, CommitFailure>
    where D: CommitData<C> {
        let frame = data.encode(ctx, self.acknowledged)?;
        self.check_reserved_bound(&frame)?;
        let candidate = self.fold_candidate(&frame)?;
        // No user callback occurs in encode/qualification/fold_candidate.
        match self.store.append(self.exact_head(), &frame).await {
            Ok(Inserted) => self.adopt(frame, candidate),
            Ok(NotInserted) => Err(self.conflict_with_winning_observation(frame).await),
            Err(error) => Err(self.recording_failure(frame, error)),
        }
    }
}
```

These names describe stages, not new independently configurable services. Implement `CommitData`
only for the associated mode/preparation/recovery types. Encoding rejects wrong context, phase,
contract and binding before Store. Candidate construction must leave acknowledged history and
counters unchanged. Only a known insertion adopts the candidate. Preserve the existing exact-head
Store API and use its actual insertion/ambiguity types; the sketch abbreviates those names.
The caller retains ownership of primary typed/qualified data across the await so an encoding or
append failure cannot erase it. Do not use an early `?` that replaces the primary with a Store error.

Journal frame v5 has `RunAdmitted`, `EffectPrepared`, `StateCommitted`, and `RecoveryCommitted`
record kinds. The latter two contain State position and exact data/context references with full
frame-local closure; the preparation record contains its data/context pair too. Journal owns
structural wire and reference qualification. Runtime validates the selected mode and semantic
adjacency in its sole fold. There are no `PureFailed`, internal-only event, or fused decision
records. Encoding success and error follows the same selected State contract.

On qualification failure, try the already-declared bounded internal alternative once, retaining
safe phase facts and an omission reason. If it cannot qualify or append, return primary plus
secondary failures. Do not recursively commit a failed audit attempt. `CommitFailure` distinguishes
qualification/rejection, conflicting qualified winner, and indeterminate acknowledgement; it does
not contain a recursively nested `RuntimeError`. Attach section 9's recording disposition to the
invocation's retained primary evidence. An original not yet serializable can be returned in its
reviewed in-memory carrier, but must not be described as durably preserved.

### 16.4 Recovery driver and cursor table

```rust
async fn recover(journal: &mut RuntimeJournal<'_>, pending: &AwaitingRecovery) -> Invocation {
    journal.check_recovery_capacity(pending)?; // first reserved result or a Y slot
    let evaluation = (|| {
        let classification = pending.classify_original().at(Classification)?;
        let request = pending.handler(classification).at(Handler)?;
        let authorized = pending.authorize(request).at(Authorization)?;
        pending.map_required_root(authorized).at(RootMapping)
    })();
    let data = RecoveryData { result: evaluation };
    let ctx = pending.checked_recovery_context();
    let committed = journal.commit(&data, &ctx).await
        .preserving_original(pending)?;
    committed.dispatch() // Fault stops; only committed Decision can authorize action
}
```

Stage annotation and `preserving_original` abbreviate ordinary typed conversions, not new public
extension traits. Encoding the recovery value may itself require its bounded `Encoding` Fault;
apply the same single-fallback rule. On resume, dispatch by the acknowledged cursor:

| Current authority | Next permitted work | Committed result |
| --- | --- | --- |
| Runnable Pure/Read | Evaluate/prepare/read/interpret within its reserved slot | Success advances; typed failure awaits recovery; Internal requires explicit reentry |
| Runnable Effect | Prepare command, then commit preparation before adapter IO | Pending command authority; preparation failure requires explicit reentry |
| EffectPending | Invoke adapter with the same command/EffectId | Pending yields without record; failure awaits recovery; invariant failure requires explicit reentry; accepted settlement commits interpreted result |
| AwaitingRecovery, no result | Evaluate associated policy | Authorized Decision or linked Fault |
| AwaitingRecovery after Fault | Explicit reevaluation within `Y` | Decision or another Fault; never reenter State to reevaluate policy |
| RequiresExplicitReentry, Pure/Read or pre-prepare Effect | Explicit execution within `X` | New outcome or preparation, charging once |
| RequiresExplicitReentry, pending Effect | Explicit attempt within `X` using retained authority | No new command/EffectId |
| RequiresExplicitReentry, settled Effect | Explicit deterministic interpretation within `X` | Retained evidence; no adapter call |
| Committed Retry/Restart | Continue exactly as authorized, charging semantic counters once | Existing visit/checkpoint/barrier rules apply |
| Committed pending Stop | End invocation, retain pending command | Later explicit entry requires `X` |
| Terminal success/domain Stop/settled Stop | Return terminal view | No further State/recovery work |

Evidence qualification and binding precede accepting settlement. A crash after external settlement
but before its outcome commits retains the existing uncertainty boundary; it cannot claim durable
settlement. Once settlement commits, every hot/cold path prohibits another adapter invocation for
that settled command. Keep restart checkpoints: they retain the admitted input at authorized State
positions. They are execution authority, not an alternative audit history, and this RFC does not
remove them.

`RunView` must expose `AwaitingRecovery` (original outcome and optional last recovery Fault) and
`RequiresExplicitReentry` (internal cause or pending Stop with actual phase), alongside runnable,
pending and terminal states. Optional last Fault expresses an actual absence, not an optional-field
substitute for phase variants. Reuse the retained outcome in failure reports; remove separately
assembled adapter incident/context reports. CLI/REST render those dispositions without inventing a
terminal failure. Read-only load never performs recovery; explicit progression does.

### 16.5 Capture implementations at owned boundaries

Capture uses a checked nonempty chain (head plus bounded tail) of closed facts. Freeze omission
metadata as a bounded list of `(layer index, field, reason)` entries plus a chain-truncation marker;
fields are `Message`, `Data`, `Body`, `Url`, `DatabaseDetail`, `Parameters`, `PanicPayload`, and
`SourceDetail`; reasons are `Withheld`, `Unavailable`, and `BoundReached`. Use at most 32 layers and
32 omission entries within the **same 8 KiB canonical budget**. If omission metadata fills, set its
own truncation marker rather than silently dropping omissions. Optional observed sizes are integers
only when actually known. Check these invariants on construction and deserialization. An opaque
layer remains explicit even when traversal finds a reviewed deeper source. Traversal stops at the
layer bound, including cycles; never walk an unbounded chain to count what was omitted.

Client extraction stays local. `CauseFact` uses the closed families in section 5, existing safe
numeric code wrappers, parser line/column/offset, and explicit unknown categories. SQLSTATE is five
validated ASCII alphanumeric bytes. OS numeric codes supplement the closed kind; unknown codes are
not formatted into arbitrary messages. Owner-local check/stage enums retain domain vocabulary.
Do not add client-library dependencies to diagnostics or a registry of downcasters.

```rust
fn provider_transport(method: EvmRpcMethod, stage: RpcStage,
                      error: &reqwest::Error) -> EvmOperationalError {
    let causes = capture_reqwest(error); // structured predicates + reviewed source traversal
    let source = ProviderFailure { method, stage, causes };
    if error.is_timeout() { EvmOperationalError::Timeout { source } }
    else { EvmOperationalError::Unavailable { source } }
}

async fn checked_acquire(pool: &PgPool) -> Result<CheckedConnection, GateFailure> {
    let mut connection = pool.acquire().await.map_err(capture_acquire)?;
    if let Err(cause) = verify_connection(&mut connection).await {
        connection.close_on_drop();
        return Err(cause); // retain typed gate cause; no protected query ran
    }
    Ok(CheckedConnection::new(connection))
}
```

Use the appropriate Store/config/index/custody gate on that same connection; a private checked
connection wrapper owns the successful gate and exposes only the intended internal access. Do not
leave an unchecked acquisition path beside it. Existing transaction/snapshot checks still apply.
SQLx-hidden attempts are explicitly unavailable. Do not route gate errors through `after_connect`
or parse `Protocol` text. Do not log source Display while capturing or closing.

For RPC responses, retain status before bounded body read, parse the checked envelope without
untagged fallback loss, retain numeric RPC code, and mark exposed message/data withheld. A body
failure adds its cause to the status; it never replaces it. Keep current deadlines, body bounds and
classification rules. A server error code alone is not proof that repeating a mutation is safe.
For SQLx, capture category, SQLSTATE and exposed reviewed sources before selecting the existing
port disposition. A network error acknowledging COMMIT remains Indeterminate. For channel errors,
inspect closure/category and drop the command without formatting or retaining it. For crypto,
retain the local operation and opaque rejection, never key-bearing payloads.

## 17. Exact deletion and replacement checklist

Paths below are relative to the repository. Delete the named obsolete implementation, its imports,
constructors, fixtures and doc examples in the same commit as its replacement. A renamed wrapper
around the old branch does not complete the deletion. Follow symbol references into all consumers;
the adapter audit remains the exhaustive first-loss checklist for capture sites.

| Location / old machinery | Required replacement or retained invariant | Commit |
| --- | --- | --- |
| `crates/live/evm/src/json_rpc.rs`: disposal of RPC code/message/data, unit `map_transport_error`, untagged error fallback | Owned bounded transport/envelope capture; preserve status and secondary body cause | 1 |
| `crates/live/evm/src/transaction.rs`: unit provider/authority/signer and mismatch mappings | Existing typed categories with operation and nested source; keep command authority | 1, 2, 3 by source owner |
| `crates/storages/postgres/src/lib.rs`: `after_connect` gate errors, `Protocol` markers and substring `classify_open_error` routing | Owned same-connection checked acquisition; direct gate causes | 2 |
| `crates/storages/postgres/src/evm_tx.rs`: `unavailable(_: impl Sized)`, `internal(_: impl Sized)` and source-erasing commit mappings | One SQLx capture owner; typed authority dispositions, exact ambiguity | 2 |
| `crates/storages/postgres/src/{config,index,provision,catalog}.rs`: query/decode/gate source sinks | Reviewed local stage plus common causal capture; preserve existing transaction and role checks | 2 |
| `crates/{keystore,signing}/src/lib.rs`: unit channel/thread/signing conversions | Reviewed source carriers, opaque crypto facts, no retained secret command | 3 |
| `crates/kernel/store/src/lib.rs`, `crates/config/src/lib.rs`: unit IO/task/allocation source sinks | Rich existing port errors; legitimate absence remains absence | 2, 3 |
| `crates/app/src/{lib,deployment}.rs`, `bin/{cli,rest-api}/src`: source-erasing composition/input/delivery maps | Preserve reviewed causal carrier and project existing public disposition | 3, 4 |
| `crates/live/evm/tests/evm_contract_effect_e2e.rs`: unit funding error capture | Fix existing real-IO test helper; no new production funding adapter | 1 |
| `crates/kernel/program/src/lib.rs`: `ProposedStateOutcome`, `AdapterContext`, `adapter_context` | Native nested Result and Runtime-assembled mode data/context | 4 |
| `crates/domains/evm/src/recovery.rs`: `EvmBalanceAdapterContext`, `AnchoredCallAdapterContext`, `EvmTransactionAdapterContext` and equivalent consumer contexts | Input/intent/command/typed error fields under section 16's explicit owners; retain every unique fact | 4 |
| `crates/kernel/program/src/recovery.rs` and `recovery/defaults.rs`: `IncidentSummary`, summary-only `IncidentSource` and old handler arguments | Direct `Classification`; preserve default/override selection and policy semantics | 4 |
| `crates/kernel/runtime/src/assembly/recovery.rs`: `QualifiedIncident`, summary-producing `ClassifyCallback`, fused request path | Decode retained typed error, project classification, evaluate only after outcome commit | 4 |
| Runtime assembly adapter contextualizer registrations and callbacks | Associated exact mode codecs; no replacement contextualizer registry | 4 |
| `crates/kernel/runtime/src/engine.rs`: `conclude`, `decide`, `domain_objects` as fused precommit policy/report paths | Outcome commit, then one recovery evaluator and recovery commit | 4 |
| Same file: separate mode conclusion/failure append construction, old `prepare_append`/`finish_append` candidate adoption behavior | One private commit façade; acknowledged state unchanged until known insertion | 4 |
| `crates/kernel/runtime/src/engine/fold.rs`: decision embedded in outcome, `PendingFailure` error/context copies and per-Effect failure count | Sole fold over outcome/recovery records and phase-aware cursors | 4 |
| Runtime `AdapterIncidentView`/report assembly solely repackaging original/context | Views of exact retained outcome; preserve root mapping and public causal detail | 4 |
| `crates/kernel/journal/src/lib.rs`: `PureConcluded`, `ReadConcluded`, `EffectAdapterFailed`, `EffectConcluded` records and encoders | v5 data/context commit records; keep preparation-before-IO and local closure | 4 |
| `crates/kernel/journal/src/conclusion.rs`: `DomainConclusion`, `ReadConclusion`, `EffectConclusion`, `PendingDecision` and fused map-ref helpers | Checked recovery result; keep Stop reason and authorization semantics under one owner | 4 |
| `crates/kernel/program/src/program.rs`: adapter `context_contract_ref` and old execution bound fields | Exact selected execution-data/context contracts and complete commit bounds | 4 |
| `crates/kernel/program/src/recovery/bounds.rs`: `max_pending_failures`, `failure_frame`, old lifecycle formula | Section 8's `P/O/R`, global `X/Y`, checked admission; rename generic frame bound | 4 |
| Earlier RFC's `InvocationFailed`, separate `InternalFailureBounds`, `commit_failure`, recovery advice or audit-sink proposal | Delete from current design/examples if present; no production replacement parallel path | 4 |
| Superseded v6/v4 descriptors, fixtures and examples | Current v7/v5 baseline plus hostile old-format rejection cases, no reader/migration | 4 |

**Keep:** intrinsic classifiers, static handlers, Operation defaults and State overrides, root maps,
retry/restart authorization, checkpoints/barriers, exact evidence binding, Store atomicity,
Indeterminate acknowledgement, redacted surface policy and keystore ownership. They carry necessary
contracts. This simplification removes duplicate representations and responsibility paths, not the
checks enforcing those contracts.

## 18. Handoff completion and material uncertainties

The engineer implements this target, rather than selecting a different storage or recovery model.
Each commit closes its changed boundary and includes tests/docs; section 13 fixes ordering and
section 17 fixes deletion scope. Before final CI, reconcile every audit row and A1–A25 with concrete
consuming evidence and report the complexity changes specified in section 12. No placeholder unit
error, compatibility constructor, or second commit path may remain to make sequencing easier.

### Material uncertainties

No unresolved architecture or ownership choice remains. The following are implementation validation
risks with fixed responses, not permission to weaken the target:

| Assumption | Why uncertain | Consequence if wrong | Validation / required response |
| --- | --- | --- | --- |
| Shipping maximum data and descriptors fit existing ceilings with `X=8, Y=8` | Exact generated schemas and closure grow, and current context descriptors approach the cap | Supported Program admission fails | Measure concrete maximum schemas/frames and checked formulas in commit 4; simplify redundant schema composition within current contracts. Report a blocker if supported workloads cannot fit; do not silently raise ceilings or remove evidence |
| Reviewed causal extraction covers exposed client layers | Client APIs can hide attempts or expose only opaque errors | A record contains explicitly partial provenance | Inject failures at each owner; assert retained fields and omission markers. Hidden evidence is an explicit limit, never fabricated |
| Same-connection acquisition gates have acceptable operational cost | Catalog checks occur at owned acquisition rather than only connection creation | Higher latency or query count | Exercise managed Store and custody scenarios; report measurements. Any later optimization must preserve gate attribution and authority |
| The resulting implementation reduces complexity overall | New recovery cursors and audit data add necessary code | LOC can grow despite removing old paths | Report deleted/added production LOC, public types, callbacks and change sites separately; explain additions and remove duplicated owners. Do not weaken audit coverage to meet a line quota |

Storage cost is accepted. Store failure, pre-admission errors, unacknowledged physical attempts, and
unavailable upstream evidence remain section 10's explicit limits.
