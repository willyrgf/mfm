# RFC: commit complete execution data and preserve causal error chains

Status: proposed for discussion; not implemented.

This RFC develops the agreed remediation plan from the
[adapter error-chain audit](docs/adapter-error-audit.md). It extends the implemented
[classification and handler RFC](RFC_SIMPLIFY_CLASSIFICATION_HANDLER.md) without replacing its
intrinsic classifiers or common static handler.

[Design](docs/design.md) and [architecture](docs/architecture.md) remain authoritative for the
current implementation. Contract changes proposed here must update those documents, repository
instructions, producers, consumers, and tests together when implemented. Rust and wire examples
below are sketches of the target contracts, not compiling implementations or frozen descriptors.

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
| Can every failure always be persisted? | No. Missing admission, invalid history, unavailable storage, exhausted capacity, and interruption have explicit limits. |

The remaining design discussion should evaluate whether the proposed representation and recording
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
    // Private, nonempty, checked; outermost exposed source first.
    causes: Vec<CauseFact>,
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

Proposed fixed bounds are 32 source layers and 8 KiB of encoded causal data, including omission
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
// Conceptual Runtime-owned commit facade; exact Rust generics are not prescribed here.
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

Outcome-first storage does not remove finite-history accounting. A failure and its recovery decision
now need two frames, and a recovery Fault can precede a later successful decision. Repeated explicit
internal attempts and stopped pending Effects also need finite admitted capacity.

Account for both the State execution and its possible recovery successor before entering work:

```text
State execution slot:
    complete outcome frame
    + reserved recovery-result frame when the outcome may need recovery

Further recovery-evaluation slot:
    one complete Decision or Fault frame
```

An internal State outcome consumes its ordinary execution slot. A recovery Fault consumes a recovery
evaluation slot. A later explicit attempt cannot reuse already consumed capacity. A failure that
is permitted under zero retry allowance still needs room for its outcome and terminal Stop result.

Replace the former internal-only eight-record allowance with complete lifecycle accounting. Retain
semantic retry/restart budgets independently: storage capacity does not authorize recovery, and a
Fault does not spend a successful recovery decision. Existing pending-Effect attempt limits must be
reconciled with the common accounting rather than counted twice or used as an internal-error budget.

The selected direction is positive finite bounds for complete State-data/context and recovery-
data/context commits, including full local object closure. Do not use an 8 KiB diagnostic bound as a frame bound or blindly
double existing byte totals: success remains one outcome, while failure paths include potentially
large originals, context, roots, and reports across two commits.

Reserve settlement capacity and the result capacity required by already admitted work. Capacity
checks happen before execution or recovery evaluation, not only after an error is produced. An
unresolved committed failure must not consume the slot reserved for its own recovery result simply
because its State outcome was appended first.

Checked admission must include first attempts, recovery-authorized State visits, pending-Effect
attempts, explicit internal reentries, and recovery Fault reevaluations against current Journal/run
ceilings. Exhaustion ends progression before further relevant work, preserves all acknowledged
facts, and can leave an Effect unresolved. Explicit resume cannot enlarge the immutable Program.

The exact authoring API, shared attempt counters, and shipping numeric allowances need review under
this revised lifecycle. They are a material handoff item, not permission to preserve the old
internal-only allowance or introduce unlimited error records. Section 15 and the uncertainty table
make that remaining work explicit.

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

The proposed logical sequence is:

1. **`preserve rpc and evm error provenance`**: shared causal data, EVM capture/contracts, direct
   consumers, exact identities, and error-aware bounds.
2. **`preserve postgres error provenance`**: storage/custody/config/index port carriers, SQLx capture,
   acquisition gates, provisioning, and all affected consumers.
3. **`preserve signer and local io error chains`**: keystore/signing, memory, deployment, application,
   and transport propagation with strengthened secret tests.
4. **`commit execution data and context before recovery`**: inseparable uniform Runtime commit
   facade, exact serialization contracts, direct-classification handler API, Program/Journal wire,
   lifecycle bounds, fold, invocation, observation, and client-contract changes.
5. **`complete adapter error audit validation`**: remaining funding-helper coverage, inventory
   reconciliation, and final cross-boundary evidence.

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

## 15. Discussion focus

The selected guarantees are fixed for this proposal; these tradeoffs deserve particular scrutiny
before implementation:

- Does one shared causal-data contract remove more duplication than it creates in public types?
- Do the execution-data/context field contracts retain all necessary facts once, with explicit
  exclusions and no fallible incident assembly before commit?
- Does the uniform commit facade replace separate success/error paths without hiding a second
  serialization model or moving IO into States?
- Does the outcome/recovery representation remove the fused and internal-only paths completely,
  with fewer responsibilities and future change sites despite a new recovery-pending cursor?
- What is the smallest finite authoring contract that bounds execution and recovery evaluation,
  including zero-retry Stops, explicit internal reentry, and repeated recovery Faults?
- Are the extra failure-path commit and callback reevaluation after an interrupted recovery step
  acceptable for shipping workloads?
- Can acquisition-time PostgreSQL checks preserve the existing security posture at acceptable cost?
- Are withholding and unavailable-evidence markers clear enough that audit consumers cannot mistake
  them for retained original messages or a record of every physical attempt?

These questions are for reviewing the proposed tradeoffs, not instructions for an implementing
engineer to choose a different architecture silently.

## Material uncertainties

The selected protocol is one `commit(state_data, ctx)` before control handoff, and the same commit
operation for recovery data before its authorized action. The meaning of both inputs and the
removal of `IncidentSummary` are settled. Custody remains secret-free and uses the existing State
commit Store. The following assumptions and remaining lifecycle design work require evidence before
this RFC is treated as a completed handoff:

| Assumption | Why uncertain | Consequence if wrong | Validation |
| --- | --- | --- | --- |
| Each execution-data/context schema captures the required facts without redundant or silently excluded fields | The exact field inventory differs across State modes, accumulated contexts, and adapters | A uniform API can still commit an incomplete audit record or exceed supported bounds | Review each schema's inclusion/exclusion table; test canonical round trips, full error chains, context pairing, and Effect authority |
| The proposed causal vocabulary and 32-layer/8-KiB bound cover ordinary exposed chains | Concrete client source APIs vary; some hide causes internally | Ordinary failures may have explicitly partial diagnostics, or the representation may need a reviewed revision | Inject nested RPC, SQLx, OS, parser, signer, and task failures; inspect each first capture point and assert omission status |
| A common finite lifecycle can replace internal-only bounds without excessive authoring knobs | Failure/decision splitting introduces pending recovery; repeated Faults and explicit internal resumes do not consume semantic retry budgets | Missing counters permit unbounded growth, while excessive reservation can reject supported workloads | Complete the authoring/counter design and checked formulas; test zero-retry Stop, repeated Faults, internal reentry, full object closure, and maximum workloads |
| A unified outcome plus recovery result reduces implementation complexity overall | The original cause commits earlier, but the fold gains an unresolved recovery position and settlement interpretation must remain distinct | Superficial unification can leave duplicate paths or permit Effect reexecution | Prototype the sole-fold transition table and deletion scope; test each crash boundary and settled-evidence reentry before freezing the API |
| Acquisition-time PostgreSQL gates have acceptable cost | Checks move from connection creation to each owned acquisition | More catalog/validation IO can affect latency or throughput | Measure the focused managed Store/custody scenarios and compare gate work; any optimization must preserve same-connection validation and causal attribution |

Unavailable upstream evidence and impossibility of recording through a failed Store are explicit
contract limits, not uncertainties to resolve by adding an independent sink.
