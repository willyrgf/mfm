# RFC: preserve causal error chains in State commit history

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

Run failure records belong in the existing Journal and State commit Store. Operational failures
continue to use their existing records, enriched with the causes currently discarded upstream.
Internal failures during execution of an admitted State gain a bounded Journal record that retains
the failure while preserving the State cursor and any pending Effect command.

The proposed implementation has three responsibilities:

1. The owner of an IO or validation boundary captures its available, reviewed causal facts once.
2. Typed error wrappers preserve those facts and add their own operation context where needed.
3. Runtime records eligible run failures through Journal and Store, and explicitly reports whether
   recording succeeded, failed, or has ambiguous acknowledgement.

There is no independent audit store, spool, raw diagnostic archive, generic logger, or classifier
registry. States remain deterministic and perform no ambient IO.

## 2. Decisions established in discussion

| Question | Selected contract |
| --- | --- |
| What does “whole error stack” mean? | Available causal layers and reviewed diagnostic facts, with explicit accounting for missing details. It is not a Rust backtrace or a promise of byte-exact raw evidence. |
| What happens to secret-bearing diagnostics? | Withhold them explicitly. Error preservation does not authorize storing credentials or arbitrary provider/client text. |
| Where are run errors stored? | In the existing State commit history, through Journal and the existing Store. |
| Are internal State failures audited? | Yes, when execution has a valid admitted cursor and the Store can acknowledge the record. |
| Does an internal failure advance execution? | No. Its record advances the history head but preserves execution position, input, visit, command, and EffectId. |
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

## 6. Example: adapter error, State context, and static handler

Consider a balance Read that receives a JSON-RPC error response:

```text
method: eth_getBalance
stage: decode_rpc_error
HTTP status: 200
RPC code: -32000
message/data: explicitly withheld under the diagnostic contract
```

The numeric code is retained as evidence; this example assigns no universal meaning to that code.
The flow becomes:

```text
HTTP / RPC ingress
    -> Unavailable {
         method: GetBalance,
         stage: RpcResponse,
         causes: [HttpStatus(200), RpcError(-32000)],
         omissions: message/data withheld
       }
    -> Balance State's existing adapter context
    -> intrinsic classify() -> Retryable
    -> selected static handler -> Stop or requested recovery
    -> Runtime authorization
    -> existing Read failure frame containing original error + context + decision
```

The State does not perform IO or rebuild the source. Its existing adapter-context callback adds
the checked balance intent and collection/source position. If that callback fails, both the
adapter cause and the internal callback error must survive through the internal-failure path.

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
```

This is the existing duplicate-safe Read policy. It is not the transaction Effect classifier.
The common handler still receives only the summary and Runtime-owned recovery context:

```rust
fn handle(
    params: &HandlerParams,
    incident: &IncidentSummary,
    recovery: &RecoveryContext<'_>,
) -> Result<RecoveryRequest, StateExecutionError>;
```

An Operation supplies the handler default; a State occurrence can override it. The handler does
not need to understand `ProviderFailure` to apply a generic policy. The original value remains
available to audit consumers independently of the summary passed to that handler.

For a State-owned failure such as `AnchorChanged`, retain the exact typed expected/observed facts
and State context. Its existing `InputInvalidated` classification remains separate from those facts.

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

## 7. Auditability in the existing State commit Store

### 7.1 Operational failures

Enrich the original values already stored by Pure/Read failure conclusions and pending Effect
failure records. Do not add a second audit append for the same operational outcome. The original
cause, State context, and authorized decision belong in their existing atomic commit.

Classification and handler execution remain hot-path behavior. Cold reconstruction uses retained
values and committed decisions rather than reclassifying or rerunning a handler.

### 7.2 Internal State execution failures

Add one Journal record, conceptually:

```rust
InvocationFailed {
    position: ExecutionPosition,
    diagnostic: QualifiedInternalFailure,
    original: Option<OriginalFailure>,
}

enum OriginalFailure {
    Domain { error: QualifiedValue },
    Adapter {
        error: QualifiedValue,
        // Present only if contextualization already completed successfully.
        state_context: Option<QualifiedValue>,
    },
}
```

The tagged original keeps State context attached to an adapter cause rather than permitting an
unrelated standalone context. Program owns the exact standalone internal diagnostic value contract;
Journal owns the frame wire and object closure. The frame carries all referenced objects locally.

This record is permitted only during hot execution against an already admitted and qualified
nonterminal cursor. It covers preparation, interpretation, adapter invariants, hot qualification,
and recovery callback failures. It does not require inventing a domain failure for an implementation
error.

On known insertion:

- the head and sequence advance;
- State position, visit, current input, and recovery counters remain unchanged;
- any pending command and EffectId remain authoritative;
- the invocation ends with its internal error and the newly qualified observation;
- no classifier or handler runs for the internal failure.

The applicable pending structure is:

```text
EffectPrepared
    -> (EffectAdapterFailed | InvocationFailed)*
    -> EffectConcluded
```

An internal record can also occur while a Pure/Read State or an unprepared Effect is Runnable.
Journal must qualify adjacency across internal records without mistaking the latest physical frame
for a different execution phase. Runtime's existing sole fold validates cursor position, counters,
and pending authority. There is no second diagnostic reducer.

### 7.3 Secondary failures retain the primary cause

Suppose the provider returns a qualified error and `adapter_context` then fails:

```text
provider failure P
    -> contextualizer failure C
    -> InvocationFailed { diagnostic: C, original: Adapter(P, no context) }
```

If context succeeds but the handler fails, retain P and that context alongside the handler failure.
Do not rerun a failed contextualizer merely to fill in the record.

If the original value cannot qualify, store the bounded qualification failure and explicit omission
status when that diagnostic can be recorded. Retain safe primary evidence in the returned failure
where representable. Do not bypass secret validation, frame limits, or type qualification to save
the rejected value.

### 7.4 Cold observations

Add a top-level `RunView.latest_internal_failure` containing the committed diagnostic, any retained
original, its execution position, and its Journal sequence. Preserve it after later progress,
including success, as historical evidence. It must not change the current State status to Failed.

All earlier records remain in Journal. Cold reconstruction does not rerun failed callbacks. Existing
pending-command qualification still applies; this RFC does not remove command-identity checks.
Read-only run inspection neither appends diagnostics nor consumes an audit allowance.

### 7.5 Exact current contracts that change

The current design says internal execution failures leave the head unchanged. This RFC changes
that to cursor preservation with a new failure frame when recording succeeds.

The adapter rule in `AGENTS.md` currently says local mismatch has “no provider call or append.”
Implementation must revise that rule: no provider call, operational conclusion, or fabricated
integrity evidence is allowed, but an internal audit record is permitted for a qualified admitted
cursor. Merely adding the new frame while leaving this instruction unchanged is an incomplete
cutover.

## 8. Finite capacity and the cost of durable internal failures

Add explicit positive authoring bounds:

```rust
InternalFailureBounds::new(max_records, max_complete_frame_bytes)
```

`ProgramLimits` must include these bounds. There is no hidden unlimited diagnostic allowance.
The complete frame bound includes the internal diagnostic, optional original/context objects,
references, and envelope. An 8 KiB causal-data bound alone does not bound that frame.

If the existing admission estimate is F frames and B bytes, reserve:

```text
total_frames = F + max_records
total_bytes  = B + max_records * max_complete_frame_bytes
```

The additional count is Program-wide, so it is not multiplied again by recovery segments. Runtime
reconstructs usage from committed internal records, separately from recovery decisions and pending
operational failure counts. All arithmetic and resulting limits are checked against existing
Journal/run ceilings.

The proposed shipping Portfolio allowance is eight records, with a caller-derived complete bound:

```text
maximum authored complete State-frame bound
    + 8 KiB diagnostic payload
    + 64 KiB additional envelope/closure allowance
```

This derivation assumes the retained original/context closure fits its applicable ordinary failure
bound. Domain bound helpers must account for richer errors, and tests must prove that assumption
for every supported maximum workload. Runtime checks actual candidate sizes. Generic authors
supply explicit conservative bounds rather than relying on this Portfolio-specific choice.

Before a new hot execution entry, Runtime requires room for another internal failure record.
Exhaustion prevents execution, including pending Effect polling. It leaves the acknowledged history
and command intact. Success or settlement capacity cannot be consumed by diagnostic overflow.

This has a deliberate liveness cost: a pending Effect can remain unresolved after audit capacity
is exhausted. Explicit resume cannot enlarge an immutable Program's allowance. This is the same
fundamental finite-history tradeoff already present for pending operational failure records; the
new allowance must be sized and tested honestly.

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
| Candidate construction exceeds bounds | Retain the primary and size/qualification cause; do not claim the original was recorded. A bounded internal diagnostic is eligible only if it can itself be prepared safely. |
| Internal diagnostic construction/append fails | Return available primary and secondary evidence. Do not recursively attempt another diagnostic record. |

A losing failed candidate must not reenter the provider, silently rebase its append, or return only
the winning view and discard its own cause. This explicitly changes that losing-failure response;
the winner's qualified history remains authoritative.

## 10. Same-Store audit limits

| Failure boundary | Persistence contract |
| --- | --- |
| Operational State/adapter failure with an admitted cursor | Existing failure commit retains its enriched cause, context, and decision. |
| Internal hot execution failure with an admitted cursor | Proposed internal record, subject to qualification, capacity, and known insertion. |
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
| One internal Journal record | The missing durable representation of internal State execution failure. |
| One Program-wide finite allowance | Unaccounted diagnostic storage growth. |
| Explicit recording outcome | Ambiguous conflation of the primary failure, audit failure, and winning history. |

Do not add a universal exception bag, arbitrary extension fields, error registry, per-State handler
family, secondary logger, raw archive, replay service, or compatibility adapter. Do not rewrite
unrelated success paths to make the error cutover appear more comprehensive.

LOC is evidence, not a quota. Report production LOC removed from source-loss plumbing separately
from causal capture, persistence, and validation additions. Also report changes in public types,
configuration points, code paths, and future change sites. A net increase needs an explanation;
compressing code or weakening tests does not count as simplification.

## 13. Contract cutover and logical commits

Use one current contract. Program v6 becomes v7 for the explicit internal bounds, and Journal frame
v4 becomes v5 for the new record/adjacency. Revise affected exact error schemas and implementation
identities where their ABI or classification semantics change. Do not bump an unrelated envelope
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
4. **`audit internal failures in run histories`**: inseparable Program/Journal/Runtime wire, bounds,
   fold, invocation, observation, and client-contract changes.
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
| A1 | Two RPC errors with equal classifications retain distinguishable numeric codes and reviewed facts through the State's existing failure commit and cold reconstruction. |
| A2 | HTTP status, send/body timeout, parser location/category, malformed field, and response bound remain distinguishable; a secondary body failure does not erase status. |
| A3 | Provider/custody/signer causes retain transaction operation context through all relevant adapters; classification remains unchanged unless separately reviewed and versioned. |
| A4 | SQLx connect/query/decode/COMMIT failures retain reviewed sources and exact definite/indeterminate behavior. Acquisition-gate failures reach the caller without protected IO. |
| A5 | Thread/channel/owner/crypto and memory/task failures remain distinguishable. Synthetic secret sentinels are absent from canonical data, formatting, and client responses. |
| A6 | Constructor and deserializer tests enforce causal bounds and explicit withheld/opaque/unavailable/bounded metadata. No arbitrary diagnostic text path is accepted. |
| A7 | Internal failures at Runnable and EffectPending positions commit diagnostics while preserving position, visit, input, command, EffectId, and recovery counters. |
| A8 | Contextualizer/handler failures retain a previously acquired original incident and any successfully qualified context; missing context is not recomputed. |
| A9 | Hot/cold views agree on committed internal evidence, including after later success. Read-only inspection performs no append and invokes no failed callback. |
| A10 | Rejection, append ambiguity, and exact-head conflict preserve primary and secondary causes with accurate recording status. A losing failure returns the winning observation without reentering the provider. |
| A11 | Maximum supported workloads fit revised bounds; overflow is checked, internal exhaustion stops execution before IO, and settlement reservation remains intact. |
| A12 | Oversized/unqualifiable originals and failed diagnostic writes never produce a false preservation claim or recursive audit loop. |
| A13 | Pre-admission, invalid-history, unavailable-Store, cancellation, and postcommit delivery scenarios obey the same-Store limits. Expected absence and Pending remain valid outcomes. |
| A14 | CLI/REST/library callers retain their reviewed public dispositions while receiving the causal and audit details permitted by their existing surface. |
| A15 | Every first lossy conversion in the audit inventory is traced to its replacement and consuming evidence. Remaining upstream visibility limits are explicit rather than marked fixed. |

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
- Are local operation enums and typed wrapping sufficient to preserve useful context without
  copying it into every source layer?
- Is the internal record the smallest change that delivers the agreed same-Store audit guarantee?
- Are eight records and the proposed complete-frame derivation appropriate for shipping workloads,
  given that exhaustion intentionally blocks pending settlement polling?
- Can acquisition-time PostgreSQL checks preserve the existing security posture at acceptable cost?
- Are withholding and unavailable-evidence markers clear enough that audit consumers cannot mistake
  them for retained original messages or a record of every physical attempt?

These questions are for reviewing the proposed tradeoffs, not instructions for an implementing
engineer to choose a different architecture silently.

## Material uncertainties

No architecture or custody choice remains unresolved: the proposal uses secret-free causal data,
the existing State commit Store, and internal failure records. The following implementation
assumptions require evidence before this RFC is treated as a completed handoff:

| Assumption | Why uncertain | Consequence if wrong | Validation |
| --- | --- | --- | --- |
| The proposed causal vocabulary and 32-layer/8-KiB bound cover ordinary exposed chains | Concrete client source APIs vary; some hide causes internally | Ordinary failures may have explicitly partial diagnostics, or the representation may need a reviewed revision | Inject nested RPC, SQLx, OS, parser, signer, and task failures; inspect each first capture point and assert omission status |
| Shipping internal frames fit the proposed eight-record allowance and derived complete bound | Original/context closure varies by State and caller continuation; richer schemas add bytes | Admission can reject a supported workload or recording can fail on a valid original | Prove revised bound helpers against maximum supported inputs, full original/context closure, and candidate frame sizes |
| Acquisition-time PostgreSQL gates have acceptable cost | Checks move from connection creation to each owned acquisition | More catalog/validation IO can affect latency or throughput | Measure the focused managed Store/custody scenarios and compare gate work; any optimization must preserve same-connection validation and causal attribution |

Unavailable upstream evidence and impossibility of recording through a failed Store are explicit
contract limits, not uncertainties to resolve by adding an independent sink.
