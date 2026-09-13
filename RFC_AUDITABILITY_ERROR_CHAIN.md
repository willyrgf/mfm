# RFC part 1: current run continuation and persistence

Status: bounded core correction; full implementation readiness remains unproved, reviewed 2026-09-13.
This RFC delivers the continuation/persistence redesign and the error path needed to use it.
[Part 2: execution error preservation](RFC_CAUSAL_ERROR_PRESERVATION.md) closes the remaining
causal losses on selected State/adapter execution paths after Part 1 is accepted. These are independent completion contracts: Part 1 can be
finished, verified and accepted while Part 2 remains a design draft. The former combined RFC's
full-handoff verdict is withdrawn; splitting it does not establish the outstanding core proofs.

Sections 6 and 9 specify the correction to prove; section 10 closes its error scope, and section 12
orders the work through Part 1 acceptance. Part 2 has a finite execution scope; unrelated platform
error remediation is outside both RFCs. Neither RFC currently demonstrates achieved net simplification or complete auditability.

`7f71beef` remains the original comparison baseline. The third attempt applied the previous RFC at
`377f649f`, completed the initial deletions at `6ea98576`, and committed its core at `aae81795`.
The current branch was reset to that core and the two RFCs carried in `1a6bcbc5`; continue the
correction here without another restart or importing the later owner branch. Prior branch/worktree
changes remain archived; do not restore them wholesale or discard other worktrees. Later work may supply a reviewed test or change when its exact scope is needed;
it is not imported wholesale. After acceptance, Part 2 starts from the accepted Part 1 commit.

[Design](docs/design.md), [architecture](docs/architecture.md), [code quality](docs/code-quality.md),
and [AGENTS.md](AGENTS.md) govern the repository. This RFC proposes changes to their persistence,
validation, and error contracts. Implemented contract changes require their documentation and
consumers in the same cutover. Read source contracts at the stated comparison/core commit;
dirty documentation in either prior attempt is not an independently accepted design.

The [adapter error audit](docs/adapter-error-audit.md) supplies evidence of losses. It is not an
instruction to recursively migrate every upstream constructor or every newly discovered caller.
Preserve available causes at changed boundaries; section 10 distinguishes that required closure
from adding upstream facts. Existing losses outside the selected execution paths remain ordinary
audit-inventory items, not completion conditions. No changed conversion may introduce a new loss.
The user-approved diagnostic trust boundary in section 4 replaces the previous universal
secret-free certification requirement for dependency-supplied diagnostics.

## 1. Objective and target

Part 1 owns the current record, admission/load/append, execution/recovery ordering, shared native
cause mechanism, and their existing Application/CLI/REST consumers. Part 2 owns extending evidence
at the named producers on selected execution paths, including their necessary recording failures. Part 1 acceptance requires its own actual simplification;
future Part 2 deletions or estimates cannot be used to justify an unfinished or growing core.

Persist the Runtime's actual serializable continuation state. Decode that same type on load,
validate the current state, and continue from it. Delete event-to-state reconstruction and all
historical semantic transition checking. There is no independent snapshot model, native-value
cache, or reference-only persistence representation to synchronize with the execution state.

```text
execution:
    current RunRecord -> permitted typed operation -> next RunRecord
        -> encode one complete commit -> atomic Store append
        -> known insertion -> adopt next RunRecord

resume:
    load committed bytes -> checked frame decode -> decode RunRecord
        -> validate current state against admission -> dispatch its permitted next operation
```

Runtime owns continuation and execution rules. Values owns the canonical objects carried in that
state. Journal owns the exact, opaque frame envelope and its codec. Store owns physical loading,
append-only storage, and atomic exact-head insertion. Each invariant has one validation owner;
repeated checks at neighboring layers are not independent guarantees.

Persist original State-domain failures and adapter operational failures before recovery policy
runs. Keep internal execution and recording failures in the invocation report, outside the history.
A classifier or public message never replaces the original error. If recording fails, report the
available original failed outcome and the independent recording failure without claiming durability.
Use one capture and rendering mechanism for reported failures. Typed outcomes retain their meaning;
sharing diagnostics does not turn every recoverable outcome into a failed top-level invocation.

Continuation still needs a cursor, current input, checkpoint inputs, recovery usage and unresolved
Effect authority. The current operation supplies the cursor/input; do not store a second copy.
Deleting the old snapshot/fold machinery does not delete the necessary continuation facts.

## 2. Confirmed decisions

### 2.1 Confirmed

| Decision | Consequence |
| --- | --- |
| Remove unnecessary qualified/cold/native-cache representations | One non-generic serializable RunRecord uses one Values carrier. No native cache or generic raw/qualified state pair; section 6.2 must prove the carrier admission API. |
| Reuse the execution state on restoration | The exact same Rust continuation type is encoded and decoded. Executable State objects, adapters, signers, and secrets remain outside it. |
| Remove duplicated responsibility | Journal has no lifecycle types or semantic validator; Runtime has no second wire model; Store does not interpret Runtime payloads. |
| Remove the fold and historical transition scan | No load path calculates historical successors, reconstructs counters, or compares past transitions. Current-state validation remains. |
| Persist State-domain and adapter operational error originals | Store their reviewed causes in the operation result, before classification/handler/mapping. |
| Report system failures outside history | No durable internal-fault variant, internal-error Program identity, callback-error schema registry, or fault-specific stop lifecycle. |
| Remove adapter_context and IncidentSummary | Preserve actual input, intent/command, original error, and direct Classification in their owners. Remove context-only and summary-only callbacks/types. |
| Remove future-capacity admission | Enforce actual object/frame/run ceilings and semantic recovery limits; delete lifecycle size predictions and capacity-only quotas. |
| Keep acknowledged history append-only | No rollback, old-format reader, compatibility path, or rewriting previous frames. |
| Keep recovery action authorization durable | Commit a normal recovery decision and resulting state before executing the action. A recommendation alone is not authority. |
| Scope auditability to execution | Preserve selected State/adapter execution failures and necessary recording/report forwarding. No comprehensive startup, config, deployment, ingress or constructor migration. |
| Trust upstream diagnostic content | No generic credential detection, sanitization or secret-free certification of dependency-supplied diagnostics. MFM does not deliberately append its own secrets or full request/connection objects. Bounds and accurate omissions remain. |
| Share capture and rendering | Capture required diagnostic information once and carry it through the existing report route. Existing typed outcome semantics decide whether the result must be committed. |
| Reuse the admitted original | Hold native E through fallible error encoding. Once the admitted error value contains the promised information, reuse that value through append/reporting and release the additional native copy. K2 must prove completeness first. |

### 2.2 Explicit behavioral answers

| ID | User decision | Required behavior |
| --- | --- | --- |
| D1 | Retry from the last committed state after an internal failure | End the failed invocation without append. Explicit resume retries the unfinished acknowledged phase; no hidden permanent-stop marker or automatic retry loop. |
| D2 | Commit settlement first; resume interpretation only | Persist accepted Effect evidence and its awaiting-interpretation continuation before calling the interpreter. An interpreter failure leaves this phase available for explicit resume without adapter reentry. |
| D3 | Load admission and latest state only | One consistent Store read returns admission, latest committed frame, and head metadata. A requested exact-candidate sequence probe is allowed for append reconciliation; ordinary loading performs no full-prefix scan. |

### 2.3 Error routing depends on meaning, not the crate name

A State's declared business failure and an adapter's declared operational failure belong in the
run history. There are two storage treatments, using the same captured diagnostic data: commit
declared outcomes before recovery; return internal/recording failures without another append. A State bug, adapter invariant violation, decoder rejection, handler/map/task failure,
Store failure, startup/configuration failure, or response-delivery failure is a system failure and
belongs in the invocation report. Ordinary startup/configuration/request failures keep their
existing handling; listing them here does not require comprehensive causal enrichment.

The same lower-level database or signer source can occur in either route. A signer rejection nested
inside a declared transaction operational error remains part of that persisted operational error.
A database failure while appending that error is a separate system/recording failure. Do not discard
a cause based on its source library, or turn an internal adapter bug into a recoverable incident.

## 3. Why the third attempt expanded

The third attempt did complete the initial deletion steps. The expansion occurred in both the core
replacement and the later error-owner migrations. Calling it an unfinished step 2 is inaccurate;
calling every addition unnecessary is also unsupported. The following measurements were reproduced
from `/tmp/mfm-audit-core-first` at `a3e8fd2c` plus its reviewed worktree on 2026-09-13.

### 3.1 Measured costs and their limits

| Checkpoint | Net production Rust code against 7f71beef |
| --- | ---: |
| Steps 1-2, 6ea98576 | -630 |
| Core, aae81795 | +1,222 |
| Current candidate, including untracked production files | +5,998 |
| Growth after the core | +4,776 |

The reproduced counter counts nonblank, non-comment production `src` lines, excluding separate
and trailing inline tests. It is a convention, not a complexity metric or a Rust syntax analysis.
The exact tracked diff against `377f649f` is **223 files, 25,713 additions, 10,442 deletions**;
untracked files are additional to that Git total. Tests and documentation are real migration cost,
even though they are excluded from production-code counts.

Post-core production growth concentrates in PostgreSQL (+1,590), Application (+830), REST (+426),
Runtime (+400), Store (+331), live EVM (+282), and the remaining owners (+917). The core itself
added 1,852 production lines after the 630-line initial reduction. Thus the later owner backlog
is not the sole explanation, and making that backlog finite would not by itself simplify the core.

The second attempt's 189-file/+20,033 tracked-line net growth and the third attempt's measurements
are separate historical observations. Neither is a deletion allowance for the replacement.

### 3.2 Obligations and mechanisms that caused growth

| Requirement in the previous revision | Actual expansion it enabled or required | Required correction |
| --- | --- | --- |
| RunCommit stores RunState.phase and OperationFacts | The same failure, command, settlement or output is serialized twice; the validator reconstructs a phase and checks agreement. The 648-line fold was deleted, but state.rs added 927 physical lines and state/decode.rs another 294. Those files also contain necessary logic; their entire size is not deletion credit. | Store one current record whose operation determines continuation. Delete the independent phase and its agreement checks. Keep actual authorization and current-record admission. |
| Ordinary Serde plus exact native constructor causes at nested Object admission | ObjectSeed and a macro/seed for every enclosing state struct, enum and collection duplicate the payload grammar, add drain/error-precedence logic, and require parallel wire tests. The previous probe tested shape-only objects and did not exercise this conflict. | Prove parse-then-admit with actual Objects and adapter signatures. Do not claim ordinary Serde preserves arbitrary nested constructor causes. Section 6 records the API tradeoff and blocking proof. |
| NativeCause retains a fallible reporting operation | project() calls child project(), serializes an owner Wire, parses RawValue, and can return another NativeCause. Store, Runtime, config and PostgreSQL acquire companion projectors; App retains failures of secondary projection. | Capture reviewed detail once at its owner. Outer errors serialize existing data. Remove the fallible projector ABI and recursive reporting-failure construction. |
| Preserve originals while requiring every reachable native diagnostic to be certified secret-free | Parser/client objects and native downcasts triggered owner audits, sanitizers and reporting work beyond the selected execution failures. | Trust dependency diagnostic content under section 4. Stop deliberately attaching MFM secret inputs; remove blanket upstream getter/source certification and per-caller sanitizers from the delivery obligation. |
| Close every remaining first-loss owner and every consumer | A real caller justified another constructor migration, then another serializer, transport path and test family. C15 had no finite completion set. | Close Part 1 with its finite core cases. Assign selected execution producer enrichment to Part 2, refined against the accepted core; unrelated platform remediation is outside both RFCs. Existing cause forwarding closes a changed API; extending an upstream contract does not silently expand Part 1. |
| Report LOC and explain increases before owner fanout | The engineer supplied measurements and a local rationale, explicitly without establishing overall minimality, then continued. An explanation functioned as acceptance. | Require independent acceptance of the actual corrected core on its own guarantees and cost. Part 2 has a separate design/cost gate after that acceptance. A failed core review does not authorize owner work. |
| Preserve exact errors from capture and reporting themselves | Auxiliary field validation and projection failures became new error families with their own projections and custody tests. | Use bounded, fixed capture-status/omission data for diagnostic construction. They describe missing evidence; they are not new operational causes or another extensible reporting subsystem. |

The RFC author is responsible for these obligations and for the overly strong handoff verdict.
The engineer also continued after acknowledging unresolved aggregate size, but the design supplied
multiple routes by which that continuation appeared compliant. More instructions to prefer fewer
lines, another clean checkout, or another local source-preservation review would not fix this.

### 3.3 What the previous verification established

The ten disposable probe tests established enum wire shape and some ownership mechanics. They did
not exercise checked Object admission inside the real nested payload, shared error serialization
across owners, rejected-data custody, or an integrated deletion. Passing them was insufficient
basis for the previous claim that no important design decisions remained.

The third attempt contains useful consuming tests, frame/load measurements and real deletions.
They are evidence to preserve and reassess against the corrected contract, not proof that the
whole attempt is minimal or complete. Final integrated CI was not established by this review.

## 4. Error preservation and the diagnostic trust boundary

### 4.1 What must survive

Sections 4-5 and 9 define one shared contract for both RFCs. Preserve the original failure's
available cause layers, operation and the fields required by its selected execution case. A local
failure can have no upstream source; Pending and expected absence are not invented errors.
Classification and public codes are projections of the retained failure, not replacements for it.

Capture at the producing boundary before a lossy conversion. Receiving layers carry the supplied
cause and add context only for an actual distinct operation; they do not inspect every producer
their callees use. A source already erased by an MFM mapper cannot be recovered downstream. If a
selected case promises that source or SQLSTATE, the producing conversion must be designed and
fixed before that case passes. Our own loss is not evidence that the dependency never exposed it.

### 4.2 User-approved responsibility for diagnostic content

MFM trusts dependency-supplied diagnostic content. This RFC does not require detecting arbitrary
credentials in provider/database error text, sanitizing every native source, or certifying that
all fields/getters/downcasts reachable from a dependency error are secret-free. This is a trust
choice, not a claim that dependencies cannot return sensitive text.

MFM remains responsible for what it deliberately adds: do not attach its own passwords, private
keys, mnemonic inputs, credentials, full requests, connection objects or keystore commands as
error context. Preserve existing handling of these explicit secret inputs. Do not dump whole
client/request objects to avoid designing an error conversion. Returned diagnostic text is not
permission to append extra application payloads.

The earlier blanket withholding of all provider/database messages and blanket native-owner
certification are superseded. No generic secret scanner, recursive disclosure audit or per-caller
sanitizer family is required for upstream diagnostics. Bounds, canonical encoding and accurate
failure/acknowledgement semantics still apply. Update the affected wording in docs/design.md,
docs/architecture.md, docs/code-quality.md and AGENTS.md in the implementing contract cutover;
their former universal diagnostic promise is not a reason to restore the rejected work.

Trust alone does not require collecting new fields. The current DiagnosticEvidence schema is
closed, and Values scans ordinary strings for secret markers. If a selected case requires returned
diagnostic text, its frozen design must specify one shared bounded field and the corresponding
admission treatment so trusted text is not rejected solely for a marker such as api_key. Do not
work around that mismatch with provider-specific scrubbing, parallel reports or a global disabling
of checks on ordinary Program/context inputs. Do not expand the schema when existing captured
fields already satisfy the case. Any necessary shared change is a named design item before coding.

### 4.3 Missing evidence must be explicit

| Condition | Meaning |
| --- | --- |
| Withheld | A specific field was deliberately excluded under the selected contract, for example an MFM-owned secret input; not a blanket assumption about dependency text. |
| Unavailable upstream evidence | The dependency did not expose the evidence, including hidden attempts. An MFM conversion that discarded exposed information is a remediation gap instead. |
| Opaque source | A source layer exists without the structured facts promised for a known category; retain its available representation and deeper links within the bound. |
| Capture bound reached | The retained source prefix or detail reached its finite bound. |
| Original not admitted | The failed outcome could not be encoded/admitted; its canonical identity is absent. Do not retry its failed serializer to build the report. |

Keep known counts/size facts only when exposed. A bounded, deliberately partial or erased
representation is not complete raw preservation. Optional detail failure does not replace the
original execution failure or become a new recursively reportable operational cause.

### 4.4 One representation at each necessary stage

| Stage | Representation and responsibility |
| --- | --- |
| Producing execution boundary | Preserve the typed domain/operational failure or native invocation cause and capture required diagnostics once. Apply the trust boundary above. |
| Encoding a declared failure | Retain native E across the immediately awaited fallible encoding job. If it fails, retain that original with the actual encoding cause and explicit missing detail/identity. |
| Complete admitted failed outcome | Its existing Object and operation facts become the original used live, restored, and during append-failure reporting. Release the extra native E once K2 proves no promised information is lost. |
| Runtime/App/transport reporting | Render already captured data and admitted original bytes. Do not rerun capture, native constructors, Values admission or a child projector. |

An admitted error's completeness means the selected cause/field contract with declared bounds and
omissions, not byte-for-byte preservation of a dependency's private native object. K2 must compare
that contract with the current native-custody promise before removing the extra copy. If required
information is absent, specify the necessary representation change or explicit guarantee revision;
do not silently erase it or preserve two originals indefinitely as a workaround.

## 5. Shared causal data, with local ownership

Keep the small `mfm-diagnostics` crate for checked causal data and its exact schema. Its purpose
is to remove duplicated diagnostic representation, omission accounting and bounds validation across
selected execution ports. The dependency graph below describes placement, not migration scope.

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
invented structured facts; preserve any accessible deeper source in its own next layer. Source order follows
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

Omission locations refer only to an existing layer index or a present response. An explicitly
omitted response field uses `Response`; an omitted client field belongs to its source layer.
These locations do not mandate secret-based filtering of upstream text under section 4.2.
`ChainEnd::BoundReached` accounts for an unretained suffix without inventing an index or
number of omitted sources. `facts_truncated` accounts for a layer's omitted facts, and
`omissions_truncated` accounts for omitted omission entries. These flags have distinct meanings.

Capture retains response context first, then an outer source prefix, within the shared byte bound.
Reserve space for all fixed bound markers before appending variable entries. Stop before an entire
next layer would exceed the byte/layer bound and mark the chain end; do not merge its facts into the
previous layer. The per-layer fact bound uses `facts_truncated`. Omission overflow uses its marker.
Constructors and deserialization enforce counts, byte bounds, legal kind/fact combinations, unique
fact fields per layer, valid omission locations and response presence. Facts are emitted in their
schema-defined field order so equivalent evidence has one canonical representation. Source kinds
and omission vocabulary are closed; there is no arbitrary map escape hatch. Section 4.2 governs
any explicitly required shared diagnostic-text addition; this existing schema is not a secret-free
certification requirement or permission for a new owner-specific schema family.

Traversal stops at its own finite bound, including a pathological cyclic source chain. Do not walk
the discarded suffix to count it. Do not format sources to infer identity or classify by messages.
The byte/layer bounds describe only shared diagnostics; owner-typed checked facts are separately
bounded by their exact value and complete persisted-value and frame contracts.

### 5.2 Existing typed wrappers retain operation context

Keep operation/stage and checked domain facts at their existing typed owner. Part 1 preserves the
operational payload already supplied by the adapter; it does not redesign provider, signer or
authority error families. Their enrichment and target wrappers belong to
[Part 2 section 3](RFC_CAUSAL_ERROR_PRESERVATION.md#3-owner-representations).

Use ordinary nested sources and borrowing. Remove incidental Copy requirements only where the
changed core API requires owned causes. Do not introduce a second Classification field that can
disagree with the intrinsic classifier. Error::source() preserves a typed source when supported;
operational MfmValue types still need not implement std::error::Error. Typed payload access remains
part of the contract. Sharing captured data must not introduce another error registry or wire tree.

### 5.3 Capture vocabulary and extraction

The omitted-field vocabulary is closed: Message, Data, Body, Url, DatabaseDetail, Parameters,
PanicPayload, and SourceDetail. Reasons are Withheld, Unavailable, and BoundReached. Observed sizes
are integers only when known. SQLSTATE is five checked ASCII alphanumeric bytes; parser locations
retain reviewed line/column/offset. Unknown OS codes may remain numeric; unknown source categories
remain opaque while accessible deeper sources retain their own layers.

Capture walks exposed source links with the selected owner's extraction, not a shared registry
of client downcasters. The diagnostics crate imports no client libraries. Returned diagnostic text may be retained under section 4.2, but never parse messages to
infer their category, and never walk an unbounded discarded suffix to count it. Retain the existing
checked capture implementation and its consuming tests instead of introducing another capture API.

## 6. One authoritative current record

### 6.1 Remove the independent phase/facts pair

Runtime owns one complete current record. It contains the latest acknowledged operation together
with complete checkpoint inputs, recovery usage and Effect barrier. The operation determines the
permitted continuation using the admitted Program. Store and restore that same record type.
There is no separately stored Phase, cursor/current-input copy or mutable accumulator beside it.

This is a projection from one current record, not event replay: no predecessor is an input, no
historical counter is reconstructed, and no earlier operation frame is consulted. Public phase
names remain useful projections. A transient borrowed selector for dispatch is allowed; another
serializable phase model or a second authoritative state is not.

### 6.2 Object decoding is a blocking interface proof

The former combination of a checked Object Deserialize, exact native constructor errors and
ordinary derived parent Deserialize did not work as advertised. Serde's generic error conversion
can erase an Object constructor source. Repeating the complete payload grammar with seeds is not
an acceptable invisible cost of the one-state claim.

The target to prove is one shape-only carrier followed by the existing private Driver admission:

```rust
struct Object {
    schema_id: String,
    content_digest: String,
    canonical: Box<serde_json::value::RawValue>,
}
impl Object {
    fn from_value<T: MfmValue>(value: &T) -> Result<Self, ValueError>;
    fn checked_ref(&self) -> Result<ContentRef, ValueError>;
    fn canonical_bytes(&self) -> &[u8];
    fn admit(&self, descriptor: &SchemaDescriptor) -> Result<(), ValueError>;
}
```

Fields are private. The wire may keep its existing nested value_ref spelling with a small local
Serde representation; it must not create a second Runtime payload family or duplicate native T.
Serde reads shape/raw bytes only. Values admission directly checks reference constructors,
canonical bytes/hash, descriptor and bounds, retaining native errors before any callback or view.
The driver is private: only successful admission establishes its executable current record.
A standalone deserialized Object is explicitly untrusted, not a proof of content identity.

This changes the old getter contract: Object cannot return a permanently checked &ContentRef.
Internal identity comparisons use admitted canonical spellings. A typed adapter entry constructs
one local ContentRef with checked_ref() and borrows it into the existing callback. This deliberately
repeats cheap identity construction; it must not repeat JSON/hash/schema admission or add a checked
reference cache. Hot construction uses the same carrier and established typed Values admission.

Framework record references/positions need the same honest parse/admit treatment wherever a
checked Deserialize would erase a required constructor cause. Use raw reference spellings and
primitive position operands in the private carrier, materializing checked IDs at typed use.
Do not silently change the checked IDs crate's public invariants or migrate domain-native types
into raw strings. The exact field/getter closure is part of the proof, not permission to rewrite IDs.

The proof must exercise malformed nested reference/hash/schema inputs through real Runtime and
Application consumers and compile the actual Read/Effect callback signatures. If this requires a
second state model, seeds for every parent, repeated expensive validation, or a broad identity
rewrite, the proposed decoder correction has failed. Stop for an architect decision; do not
announce another full implementation handoff. Ordinary Serde and arbitrary nested native-cause
preservation are not simultaneously guaranteed without this demonstrated boundary.

### 6.3 Journal and Store ownership

Journal seals/decodes an opaque canonical Runtime payload, with exact envelope RunId, sequence,
predecessor, hashing and actual complete-frame limits. It owns no Program/lifecycle types or
semantic history validation. Embed canonical JSON values, not escaped JSON strings or byte arrays.
Store owns mechanical consistent loading, atomic exact-head append and cumulative bounds only.
Neither owner duplicates Values admission or Runtime authorization.

### 6.4 Validation belongs at admission and typed use

| Owner | Required responsibility |
| --- | --- |
| Journal | Canonical envelope/header and complete-frame bound. |
| Values | Current object reference/hash/schema admission and object/descriptor bounds. |
| Private Runtime Driver admission | Program association, current operation mode/slot contracts, checkpoint/usage/barrier relations and current Effect identity. |
| Typed operation entry | Native constructor invariants and actual capability evidence binding. |
| Runtime successor construction | Recovery authorization, counter charging, fresh visits and Effect restrictions. |
| Store | Physical snapshot/rows, atomic append, immutable prefix, cumulative count/bytes. |

Admission checks all present object occurrences. It does not eagerly construct dormant native
checkpoint or terminal values. Objects selected for a typed operation use their owner's native
constructor. No semantic history scan, classifying a past error again, root-map replay or comparison
against an independently reconstructed Phase is allowed.

Locally valid forged counter resets or substituted checkpoint inputs remain outside historical
verification, as explicitly selected by the user. Removing duplicate phase agreement does not
remove actual contract, range, authority or current-record checks.

### 6.5 Record and dispatch sketch for the bounded proof

The sketch expresses data ownership. Checked ID names below denote their semantic roles; the
shape-only framework fields and checked materialization boundary must be resolved by section 6.2's
proof before this becomes a final Rust signature contract. It must not be implemented by retaining
the old checked parent decoder seeds behind these names.

```rust
struct RunRecord {
    program_ref: ContentRef,
    checkpoints: Vec<Checkpoint>,
    usage: Vec<StateUsage>,
    effect_barrier: Option<StatePosition>,
    operation: RecordedOperation,
}
struct Checkpoint { position: StatePosition, input: Object }
struct StateUsage { retries: u32, restarts: u32 }
struct Call { position: ExecutionPosition, input: Object }
struct EffectCall { call: Call, effect_id: EffectId, command: Object }
struct Settlement { effect: EffectCall, evidence: Object }
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
enum RecordedOperation {
    Admitted { program: Object, initial: Object },
    Succeeded { call: StateCall, output: Object },
    Failed(Failure),
    EffectPrepared(EffectCall),
    EffectSettled(Settlement),
    Recovered {
        failure: Failure,
        classification: Classification,
        request: RecoveryRequest,
        outcome: RecoveryOutcome,
    },
}
enum RecoveryOutcome {
    Retry,
    Restart { checkpoint: StatePosition },
    Stop { reason: StopReason, root: Option<Object> },
}
```

A root is required for domain Stop and forbidden for Read/pending-Effect Stop; validate that local
relation. Request and authorized outcome remain distinct because a request may be denied. Reuse
Program's existing Classification/request/reason vocabulary. Do not add policy mirror enums.
Program appears as its canonical document Object at admission, without making Program an MfmValue.
New Runtime enums use external snake_case tags compatible with inline RawValue, reject unknown
and duplicate fields, and have no MfmValue identity or codec registration.

| Current operation | Continuation derived from this record and admitted Program |
| --- | --- |
| Admitted | Declaration 0/visit 0 using initial input; an empty Program succeeds with that input. |
| Succeeded | Next declaration with output and visit + 1; after the final declaration, terminal success. |
| Failed | AwaitingRecovery of exactly that Failure. |
| EffectPrepared | EffectPending with exactly that command/EffectId. |
| EffectSettled | AwaitingInterpretation of exactly that settlement. |
| Recovered Retry | Same Read input with fresh visit, or unchanged pending Effect authority. |
| Recovered Restart | Selected retained checkpoint input with fresh visit from the failing call. |
| Recovered Stop | Domain/Read terminal result, or unchanged pending Effect with recovery stopped. |

There is no stored phase to compare with this table. Delete TerminalFailure as a second stored
copy; derive the public terminal report from failure/outcome. Current operation mode, position,
slot identity and permitted request/outcome relationships remain checked.

Checkpoints are sorted, unique, declared and no later than the current continuation; replace their
input on re-entry and remove later checkpoints on Restart. The run total is the checked sum of
per-declaration retries/restarts, with no stored global-counter copy. Only authorized Retry/Restart
charges the failing declaration once. Stop, denial and internal failure charge no grant.

Retain baseline restart eligibility: target declared for the failing State, active retained input,
target no later than failure and strictly after the Effect barrier, and a Read in the inclusive
interval. Pending/settled Effects cannot restart. Ordinary Read Retry and forward advancement use
a fresh visit; pending Effect Retry retains visit, command and EffectId. Derive/check EffectId from
the existing RunId/Program/position/command contract. No historical rows establish these facts.

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

Runtime constructs its next complete record, admits any new Values
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
Runtime encodes/decodes RunRecord inside payload and owns the payload's exact field/tag contract.
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

Where a checkpoint and the current operation require the same input, share immutable ownership in
memory and serialize those actual occurrences. Do not add an independent phase copy of the latest
operation or an intermediate-recovery timeline. No internal error is an operation-history variant.

### 8.2 Phase behavior

The phase names below describe behavior derived from the one record in section 6.5. They are not
instructions to introduce another persisted enum or to store the same operation in two places.

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
2. Build and append the Failed record, which derives AwaitingRecovery.
3. After known insertion, classify that original and invoke the selected handler.
4. Authorize the recommendation against the current state and semantic limits.
5. Perform root mapping only if the authorized result requires a terminal domain Stop. A denied
   Retry/Restart can result in such a Stop; an authorized retry must not perform root mapping.
6. Commit the decision/resulting state before executing Retry/Restart or other permitted work.

An internal classifier/handler/map failure leaves the original durably AwaitingRecovery. Report the
system failure to this caller. Explicit resume may rerun that unfinished recovery evaluation; no
once-only claim applies to uncommitted in-memory policy progress. Ordinary Stop or denial is a
normal recorded recovery result, not a system fault.

Runtime derives the recovery-context view from RunRecord and the admitted Program, including current
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

## 9. Capture once; report existing data

### 9.1 Native errors are not executable projectors

Keep exact typed State-domain/adapter operational errors for actual persisted results. Internal
constructor, task, Store and callback failures remain invocation errors, without MfmValue identity,
persisted error schema, decoder registry or a durable fault lifecycle.

Replace the third attempt's fallible reporting operation with immutable captured detail. The
owner prepares its diagnostic representation once while its original is still held. Outer
errors add their local operation context and serialize existing data, including already-captured
children. Reporting does not call owner extraction, native constructors, Values admission or
another error's fallible projector.

Delete NativeCause::from_error_with, project() -> Result<RawValue, NativeCause>, companion owner
Wire/project methods introduced solely to project erased children, and secondary projection
attempts. Do not replace them with a new trait, serializer registry or generic error framework.
The shared DiagnosticEvidence representation and typed owner facts remain the capture vocabulary.

### 9.2 Capture result and failure custody

The native invocation carrier may retain its native cause and immutable captured detail. It is
not a second copy of a successfully admitted declared outcome. Its native capture API and the
shared rendering of already captured/admitted detail are K2 proofs. It stores no callback to
execute during public serialization. Conceptually:

```rust
struct NativeCause {
    owner: /* private erased native cause, when needed under section 4 */,
    detail: CapturedDetail,
}
enum CapturedDetail {
    Available(/* immutable captured JSON, bounded while built */),
    Omitted { reason: CaptureFailure, partial: /* bounded captured prefix, when present */ },
}
enum CaptureFailure {
    Size { limit: u64, observed_at_least: u64 },
    Encoding { category: ParseCategory, location: ParseLocation },
    Panicked, // payload withheld
    Withheld, // explicit contract exclusion, e.g. MFM-owned secret input
}
```

These are internal shapes to prove, not a second public report tree. Omission status retains any
diagnostic prefix already captured plus the known operation/stage. It never asserts an exact
omitted count or full preservation. Use the existing 32 MiB native detail ceiling and 8 KiB shared
diagnostic ceiling; no unbounded serialization before checking the size. A nested native cause's
Serialize writes its captured data/status, not its native owner or another fallible projection.
Only already-complete captured diagnostic data can be a partial prefix. Drop an incomplete JSON
buffer on capture failure; do not add JSON repair, field recovery or streaming introspection.

CaptureFailure is fixed reviewed data. Constructing or reporting it does not invoke another
projector or create another NativeCause with an arbitrary serializer. This fixed status need not
collect serializer/panic payloads. Source diagnostics use section 4's trust boundary; a secondary
serialization failure is not a fabricated ancestor of the original execution failure.

For a complete admitted declared original, reuse its Object bytes, identity and existing failed-
outcome facts. Do not recanonicalize it for reporting or retain a second native E until COMMIT.
During initial encoding, native E remains held outside the immediately awaited pure job. If that
job fails, preserve the available original and actual encoding cause without claiming a canonical
identity; render already available detail or an explicit omission, not a second call to the failed
serializer. Known MFM secret inputs are not copied into the public report as a fallback.

Holding native E across encoding does not require eagerly serializing it a second time. The
EncodeOriginal report can retain that native owner with already available diagnostics or explicit
missing detail; constructing its carrier must not invoke the failed value serializer again. No
Error trait bound is added to declared operational MfmValue types to make that retention work.

K2 must prove the admitted error contains the cause layers/fields promised to its consumers and
that no required native-only information disappears at this ownership change. This is a condition
for deleting the extra native copy, not permission to add a generic successful-result custody bag.
A successful result's encoding/append failure has no original execution error to manufacture.

### 9.3 Callback and decoder boundaries

Keep one native error route through Values, Runtime and Application. The MfmValue native decode
hook has an ordinary Serde default returning reviewed parser information. Override only the named
checked-constructor path E5 in section 10: parse that owner's raw fields, call its existing checked
constructor and preserve its native cause. A checked Object admission failure uses section 6.2's
direct admission route, not a seed for every parent and not a default-then-fallback decode attempt.

The default does not promise native sources that an arbitrary user Deserialize already erased.
Report that limitation honestly; do not use it to drop a source supplied by the selected native
hook. Additional owner constructor migrations are explicit scope changes, not automatic C14 work.

Replace baseline unit PreparationError, StateExecutionError and AdapterInvariantError directly
with the shared native carrier at their existing boundaries. Keep these signatures:

| Boundary | Result |
| --- | --- |
| PureState::evaluate(Input) | Result<ProposedStateOutcome<Output, Failure>, NativeCause> |
| ReadState<C>::prepare(&Input) | Result<C::Intent, NativeCause> |
| EffectState<C>::prepare(&Input) | Result<C::Command, NativeCause> |
| ReadState<C>/EffectState<C>::interpret(Input, &C::Evidence) | Result<ProposedStateOutcome<Output, Failure>, NativeCause> |
| Handler::handle(&Params, Classification, &RecoveryContext) | Result<RecoveryRequest, NativeCause> |
| ValueMap::apply(&Params, Input) | Result<Output, NativeCause> |
| ClassifyError::classify(&self) | Classification; deterministic and infallible, unchanged. |
| AdapterError<E> | Operational(E) or Invariant(NativeCause); E still need not implement Error. |
| Evidence binding | Existing exact intent/command/evidence/ref arguments with NativeCause on internal rejection. |

NativeCause's capture/access methods must be finalized by the integrated proof. Do not preserve
its former fallible projector API simply to avoid changing callers. No callback-associated
internal persisted-error ABI, native-success cache or per-error identity is added.

### 9.4 Recording failure holds two related facts

A Read can time out and then fail to record that timeout because the database is unavailable
before append. The invocation reports both the failed Read and the separate recording failure,
with no claim of a new committed state. The database failure did not cause the provider timeout;
retain each cause chain under its actual operation. Recovery does not run before the failed
outcome commits, and explicit resume uses the last committed state. A successful State followed
by failed recording has only a recording failure, not a fabricated original execution error.

Before error encoding finishes, retain native E as described in section 9.2. After complete
admission, carry the existing failed-outcome type (`Failure` in section 6.5), whose original is an
Object. Remove the ReturnedFailure Object-plus-native pair and its extra native lifetime. Reuse
immutable original/operation data already held by the candidate; no second outcome/report tree,
native-success cache or report-time decode from the encoded frame is required.

Keep the existing RuntimeError::Recording and InvocationFailure::Execution route. The payload
sketch makes the ownership stages explicit without a native-or-Object option in every variant:

```rust
enum RecordingFailure {
    EncodeOriginal {
        original: NativeCause,
        cause: NativeCause,
    },
    BeforeAppend {
        original: Option<Failure>,
        candidate: Option<EncodedRunFrame>,
        cause: NativeCause,
    },
    Append {
        original: Option<Failure>,
        candidate: EncodedRunFrame,
        outcome: AppendFailure,
        observation: Option<(RunSummary, CandidatePresence)>,
        reload_cause: Option<NativeCause>,
    },
}
enum AppendFailure { NotInserted, Store(StoreError) }
enum CandidatePresence { Present, Excluded, Absent }
```

Failure is the existing admitted failed-outcome type, not a new generic error envelope. Final
sharing/borrowing signatures belong to K2. EncodeOriginal has no admitted original or candidate.
Later variants retain no native E. BeforeAppend has a candidate only after sealing; Append holds
one exact candidate. Public output exposes its identity/sequence/digest, not the frame body.
A known successful insertion releases invocation-local recording context and cannot reopen work.

NotInserted is a physical disposition and need not invent a causal error. Only it probes
automatically under section 7; keep the physical finding separate from latest-view admission.
Store errors, including Indeterminate, return immediately with the actual disposition and candidate
when available. Do not append an error about the failed append or introduce a second durable sink.

All reported failures use the same captured-detail/admitted-value rendering. Public encoding or
writing can still fail: keep the available invocation/head and actual terminal encoding/IO cause
in the existing transport owner, report any omitted delivery detail, and end there. Do not retry a
projector, build ReportFailure<T>/IncompleteReport for arbitrary successful values, or claim socket
delivery from REST response handoff. No additional audit write or recursive reporting is required.

## 10. Closed Part 1 error scope

Part 1 proves the shared mechanism through the cases below, preserving already supplied causes.
Part 2 enriches only its selected execution producers. Startup/config/deployment/ingress and broad
constructor audits are outside both RFCs. E5 remains one explicitly selected execution-time decode
case; it does not authorize a general constructor migration. E4 preserves actual admission errors
without certifying the entire native parser/source object as secret-free.

| ID | Required core boundary and evidence | Stopping boundary |
| --- | --- | --- |
| E1 | Pure/Read/Effect preparation, execution/interpretation, handler, map and erased Runtime conversions carry their supplied native cause. Internal failure appends no fault; explicit resume follows acknowledged state. | Section 9.3 callbacks and their invocation consumer. No arbitrary Deserialize provenance or new upstream error family. |
| E2 | A declared State failure and the shipping Read's existing operational failure retain their complete admitted original through failure-before-recovery and cold inspection. Pending Effect preserves command/EffectId and settlement precedes interpretation. | Existing declared payload and exposed cause; selected provider/authority/signing enrichment belongs to Part 2. |
| E3 | A failed Read plus a failed append reports both failures and actual acknowledgement. Also cover original encoding failure, successful outcome plus append failure, NotInserted/Store dispositions and actual terminal output failure. Compare native and admitted required cause facts before dropping extra native custody. | Existing recording/invocation/reporting route, one admitted original and one candidate. No duplicate native original after complete admission, native-success bag, reporting tree or second audit sink. |
| E4 | Actual nested Object reference/hash/schema failures reach Runtime/App with their available admission cause. Parser/canonical errors use the shared native path and section 4's trust boundary. | Existing validation and affected consumers; no parent seed grammar, broad ID rewrite or JsonError/CanonicalError getter/source certification project. |
| E5 | Shipping EvmBalanceContext metadata.correlation constructor: empty and excessive-length failures retain location and required limit/observed length through the actual typed Runtime callback and Application report after structural/schema admission. | These two selected cases only; reuse/extract the existing constructor without migrating other context/value families. |
| E6 | Consumers of changed Object, NativeCause, Store and callback APIs compile and forward already supplied causes/fields/classification using shared rendering. | An unexpected caller still needs the correct forwarding update; it does not authorize examining every producer it calls or adding producer facts, traversal or schemas. |

Before coding a core API cutover, freeze its producing failure, required cause facts, exact
changed API and receiving conversions, terminal observation, finite assertions, reuse and deletions
in the section 11.3 record. A file list or “existing consumers” is insufficient. New producer facts
or capture/schema mechanisms need a concrete architect design/cost decision before expansion.

If a promised cause has already been erased, that case is unmet until the producing conversion is
resolved; it cannot pass by relabeling the loss unavailable. Preserve all supplied causes in changed
callers. Apply section 4's explicit MFM-input responsibility without turning upstream text into a
new safety audit. Record unselected execution gaps for Part 2 refinement; unrelated platform gaps
stay in the ordinary inventory and are not automatically part of either delivery.

### 10.1 Current public projection

Keep Runtime `start`, `resume(&RunId)`, and `read(&RunId)` and the existing App/CLI/REST request
surfaces. RunView remains an owned immutable projection of admission, current commit, and head,
optionally sharing Arc-backed commit/object data. Its return type gains no borrowing lifetime and
it is not another mutable continuation model. Do not serialize all checkpoints/usage implementation
fields as the public RunView merely because RunRecord now implements Serialize.

Add the two new incomplete phase alternatives to the current RunView/App serializer:

| Public state tag | Required projection |
| --- | --- |
| `runnable` | Position and reason derived from the current commit: admission/success means Advance; Recovered Retry/Restart gives that reason. |
| `effect_pending` | Position, EffectId, and latest failure/decision from current Recovered facts when present. Replace `state_context` with actual input/command facts; do not retain a contextualization API. |
| `awaiting_recovery` | Position, original failure and its Pure/Read/Effect operation inputs, intent/command and accepted evidence as applicable; no classification/decision is fabricated before it commits. |
| `awaiting_interpretation` | Position, EffectId, and accepted settlement command/evidence. This is an incomplete state with no recorded internal fault. |
| `succeeded` / `failed` | Existing complete success value or derived terminal failure report, using the current operation, original Object and recovery outcome. |

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
captured causal detail to the existing invocation serializer, including original versus recording
cause and known/unknown acknowledgement. Do not put raw candidate bytes or native client objects
in JSON. App and the transports render already captured/admitted data rather than extracting causes
again. A postcommit projection/delivery failure retains known insertion and cannot reopen work.

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

### 11.2 Prove the complete Part 1 representation

Reuse the third core's actual Runtime entry points and consuming tests. Revalidate the corrected
representation on the small complete path: admission, successful State/context advance, failed Read
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

For Part 1 acceptance, report the corrected core delta against both `aae81795` and `7f71beef`,
including replacement code and untracked additions, separately from tests/docs. Name removed public
types, callbacks, code paths, and remaining change sites. The core must have one continuation declaration,
one object representation, one restoration route, and no semantic-history scan. A smaller fold file
or passing local test is not evidence if its responsibilities survive elsewhere.

Apply section 11.3's acceptance decision after these measurements. An explanation of growth does
not permit continued owner work. This is a review of the bounded correction, not permission for
another repository-wide salvage audit or discarding user work.

### 11.3 An explanation of growth is not acceptance

The previous core supplied measurements and retained tests, yet still introduced duplicate state
agreement and parent decoder/projector machinery. The gate now requires an independent architect
review of the actual diff and cost, not an engineer's assertion that each addition has a rationale.
The review is about convergence of the whole current core, not another local source-custody question.

The reviewed packet is one concise current record: baseline/candidate commits, selected test results,
production delta and total additions/deletions/files including untracked work, removed/added types
and responsibilities, shipping bytes/load rows, E1-E6 results, and any known upstream gaps deferred
to Part 2. Keep this evidence with the Part 1 completion record; update it instead of accumulating
progress or scope-attribution documents. Part 2 estimates or resolved owner contracts are not
required to accept Part 1.

Acceptance must name the actual removals from the third core and identify every replacement.
A renamed validator, another generic decoder family, a fallible projector in a different crate,
or missing required original information fails the gate regardless of LOC. Applying a blanket
upstream secret-free certification would contradict the agreed scope. The corrected core must
remove the identified redundant responsibilities and report a smaller affected core against
`aae81795`. Also show the total against `7f71beef`; improvement over an inflated attempt is not
by itself achievement of the user's net-simplification objective.

No unsubstantiated numerical forecast is established here. The architect must reconcile the
completed Part 1 delta and responsibility removals against both baselines. If Part 1's net reduction
or required complexity improvement cannot be supported, Part 1 remains unaccepted; future owner
work cannot supply hypothetical deletion credit. The engineer cannot waive this by describing
growth as necessary or by removing unrelated code/tests. Conversely, unfinished Part 2 estimates
or source contracts cannot invalidate an otherwise accepted Part 1.

## 12. Ordered work and authority to continue

The next task is the bounded core correction below. **Do not issue another “fully implement this
RFC” goal while K1-K3 remain unproved.** Continue on the current `aae81795`-based branch with both
revised RFCs. Preserve the archived third attempt and its measurements. Steps 1-2 are already
measured at `6ea98576`; do not repeat them or discard their coherent deletions. The Part 2 draft
does not authorize owner implementation during this work.

### K1: `remove duplicate continuation facts`

Replace Phase + OperationFacts with the section 6.5 record in the real Runtime path. Delete the
stored TerminalFailure copy, phase-reconstruction/agreement code and repeated payload accounting
at the same cutover. Preserve authorization, checkpoint/usage/Effect checks and observable public
phase behavior. Reuse consuming lifecycle tests, deleting assertions for the superseded duplicated
wire. Measure actual removals and encoded occurrences before starting a new error owner.

### K2: `share captured error data and release duplicate originals`

Replace fallible NativeCause projection with section 9's captured detail and one rendering route.
Use section 4's upstream trust boundary; remove blanket native-source certification and caller
sanitizers from the obligation. Do not add capture fields or alter shared admission without a
selected execution requirement and a concrete design for that field.

Exercise E3 through actual Runtime/App/CLI/REST consumers: failed execution plus failed encoding
or append, success plus failed append, and terminal reporting failure. Prove the complete admitted
Failure/Object preserves the promised original fields and cause chain, then remove the extra
native original after encoding, the ReturnedFailure pair, recursive projectors/owner Wire trees
and generic successful-result reporting custody in that closure. Do not infer full raw native
fidelity from Serialize or a successful round trip; identify any required missing fact first.
Finalize the NativeCause/captured-detail/RecordingFailure access and sharing signatures in this RFC.
Retain append/probe/delivery facts without adding a reporting or audit service. Do not migrate
PostgreSQL/config/signing families to make this shared-path proof appear comprehensive.

### K3: `prove direct current-record admission`

Resolve section 6.2 using real Object/reference/hash/schema failures inside the current payload,
real typed adapter arguments and the E5 constructor consumer. Compile and exercise both live and
restored calls. Delete ObjectSeed and the enclosing seed grammar only when this actual admission
route preserves the required causes and object/identity invariants. The corrected Object getter,
framework field types and owner capture APIs must be written as final signatures in this RFC.
A toy shape-only round trip or an independent Runtime model does not pass this proof.

K1-K3 are logically ordered work items. Keep an inseparable API/cutover together; do not commit a
broken workspace or preserve two production designs merely to split commits. Retained behavior
must pass the relevant focused checks from [build and verification](docs/build-and-verification.md).
No broad owner migrations, new client extractors or arbitrary constructor families belong here.

### G1: architect acceptance of the corrected core

Apply section 11.3 to the exact K1-K3 candidate. The dedicated architect gives a concrete accepted
or rejected disposition with the outstanding failures/removals; a request for more evidence is not
acceptance. This review decides Part 1, independently of Part 2 readiness or cost. If the corrected
core cannot be justified, return the specific unresolved design to the user. Do not spend another
work period on unrelated small cleanups and then resume fanout.

### F1: complete and accept Part 1

Reconcile C1-C18 with K1-K3, E1-E6 and G1. Update authoritative design/architecture, code-quality
policy, AGENTS ownership/trust wording and affected transport contracts in their owning cutovers. Remove superseded implementations,
tests, fixtures and docs while retaining observable coverage. Part 1 ends here; no owner row from
Part 2 is required or automatically authorized by G1.

Use the narrowest relevant checks under [build and verification](docs/build-and-verification.md),
managed PostgreSQL/SQLx checks for changed persistence queries and managed Effect/client scenarios
for changed execution boundaries. All direct Rust tooling runs in the default Nix shell. Run one
final `nix run .#ci` on the complete Part 1 candidate. If verification changes that candidate,
repeat affected checks and obtain review of changed contracts before recording acceptance.

Record the accepted implementation commit, finalized Object/RunRecord/NativeCause/Store signatures
and source locations, C1-C18 evidence, real removals/additions and cost against both baselines, and
known omissions/deferred upstream gaps. Use one concise completion record in this RFC, not a new
progress-document family. Until completed, that record must say pending rather than cite a proof
or intermediate commit as accepted. At this revision: **Part 1 acceptance is pending.**

This completion record is the input to Part 2's refinement step. Part 2 may remain draft after Part 1
is complete. Once refined and accepted for implementation, it proceeds on the accepted Part 1 commit;
it does not restart the persistence cutover or replace Part 1's accepted design implicitly.

## 13. Acceptance evidence

These C1-C18 criteria apply only to Part 1. The combined RFC's broad C15 is replaced by Part 1's
E1-E6/C15 and Part 2's selected execution B1-B8 criteria; excluded platform migrations are removed,
not transferred. Tests for removed durable system faults and historical semantic validation are
obsolete. Preserve retained behavior in consuming tests rather than retaining obsolete fixtures.

All cause guarantees below, including C5/C9/C11/C17, preserve supplied reviewed causes through
changed core APIs and prove E1-E6's selected admission/constructor/original-retention facts. They
do not require enriching upstream unit/flattened errors outside that scope. Record existing gaps
in the relevant inventory; only selected execution gaps belong to Part 2. Never create a new loss
or fabricate a placeholder to pass a core case. Persistence,
authorization and execution guarantees remain complete for the supported core behavior.

| ID | Observable acceptance |
| --- | --- |
| C1 | Fresh and restored runs use the same RunRecord/object carrier types and produce equivalent continuation/output through a real Runtime entry point. Every current enum variant round-trips through canonical JSON with inline objects. No qualified cache, alternate wire state, object resolver, or post-encode native round trip. |
| C2 | Admission/latest loading uses one consistent snapshot. Increasing history length does not increase normal rows loaded. A missing required row, inconsistent head, bad frame/object hash, wrong Program identity, or invalid current phase/slot contract rejects before action. Repeating start with identical Program/input returns the checked observation without driving; different admission returns AdmissionConflict. |
| C3 | Current-record admission rejects invalid operation/mode/slot and request/outcome relations, checkpoint/barrier relationships, out-of-range counters and checked-sum overflow. There is no independent Phase or agreement validator. The run total has no independently stored counter. No historical successor construction or claim of detecting otherwise locally valid historical counter/checkpoint substitution remains. |
| C4 | Success carries complete context to the next State. Restart selects an eligible declaration checkpoint's stored input, creates a fresh visit, retains current usage/Effect restrictions, and never consults old operation frames. |
| C5 | E2's State-domain and adapter operational failures each commit original/input/request/evidence as applicable before classification, handler, or mapping. Their supplied reviewed causes survive cold inspection. |
| C6 | Handler/map failure reports its native cause with no fault append; the original remains AwaitingRecovery. Explicit resume reruns unfinished recovery evaluation. An authorized Retry/Restart charges the failing declaration and run once, then yields. Stop/denial spends no grant. Pending Effect Retry preserves visit/command identity. |
| C7 | Effect preparation commits command/EffectId before adapter entry. Pending/error/Stop behavior preserves that same unresolved command and existing reconciliation restrictions; no replacement authority or capacity-only retry quota. |
| C8 | Accepted Effect settlement commits before interpretation. Injected interpreter failure appends no fault; cold resume interprets retained evidence without entering the adapter for that phase. Uncommitted settlement makes no durability claim. |
| C9 | E1/E4/E5 internal State/preparation/adapter/task/constructor failures retain supplied or explicitly required native causes in the invocation report and append no fault. Explicit resume retries permitted unfinished work; it does not create an automatic retry loop. |
| C10 | Local preflight mismatch performs no provider call/append; post-response binding rejection reports the true stage without appending unauthenticated evidence or falsely claiming no IO occurred. |
| C11 | E3 preserves the failed execution outcome and independent recording cause across encoding, append and App. Native original survives failed encoding; after complete admission the same Failure/Object serves append-failure reporting with no extra native copy. Success plus recording failure has no fabricated execution error or native-success stash. Candidate/disposition is retained once; Store errors return without a probe or audit retry. |
| C12 | The NotInserted probe covers exact matching candidate, conflicting occupied sequence, candidate followed by later commits, absent candidate, and failed reload/projection. It retains the physical finding independently of the latest view and never drives further work. A candidate-absence snapshot cannot resolve a still-in-flight COMMIT; the Store-error invocation returns with its ambiguity/custody. Fresh resume may use current phase authority without claiming it resolved an unavailable prior candidate. |
| C13 | Atomic append, immutable earlier frames, exact-head conflict, checked cumulative bounds, consistent loads, and PostgreSQL acknowledgement ambiguity hold through Store public boundaries. |
| C14 | E5's empty and oversized metadata.correlation constructor failures reach the actual Runtime/Application report with location and reviewed length facts. K3 separately proves nested framework reference/hash/schema failures through direct admission and real adapter signatures, without parent seeds. No additional constructor family is implied. |
| C15 | E1-E6 use the shared capture/rendering contract and retain classification/disposition/routing. Trust upstream diagnostics without credential scanning or native-source certification; MFM does not attach its own secret inputs. Required admitted-vs-native facts, bounds and omissions are explicit, capture failure is fixed data, and no source information newly disappears. Part 2 execution gaps and unrelated inventory items are distinguished. |
| C16 | Actual oversize/frame/run rejection leaves the acknowledged head unchanged. Cover a failure that committed but whose recovery cannot fit, and an externally settled Effect whose settlement cannot fit. No future-capacity promise remains. |
| C17 | App/CLI/REST show the retained current result or E1-E6 supplied/required causal invocation report with section 10.1's exact statuses and accurate acknowledgement/delivery status, including known insertion followed by projection failure. They do not reconstruct history, generate internal-fault records, or deliberately append MFM secret inputs. Upstream diagnostic content follows section 4; broad startup/ingress enrichment is outside both RFCs. |
| C18 | K1-K3 have integrated deletion and retained-behavior evidence; G1 accepts the complete corrected core on its own cost and guarantees. F1 records the accepted Part 1 commit, finalized APIs, shipping bytes/load IO, cost against both baselines and final verification on that candidate. Part 2 readiness or completion is not required. |

No test calls a historical reconstruction implementation merely to compare it with the new one.
Use baseline observable behavior where it remains contractual, concrete expected transitions, and
hostile physical/current-state inputs. Update old-format fixtures under the single-current-design
policy; do not retain a reader to keep obsolete fixtures passing.

## 14. Deletion ledger and completion

Keep these actual baseline removals in the corrected core:

| Baseline responsibility | Retained replacement |
| --- | --- |
| FoldState/history reconstruction | One current record and live execution rules; no semantic history scan. |
| QualifiedValue/ColdQualifier/native cache | One serializable carrier with direct admission and native decode at typed use. K3 must prove its concrete API. |
| Journal lifecycle/object-table/resolver | Opaque Journal envelope and inline Runtime-owned record. |
| Normal complete-prefix Store load | Admission/latest plus optional exact-candidate probe; physical append accounting remains. |
| Fused outcome/recovery and settlement/interpretation | Original failure and settlement committed before their respective next operation. |
| adapter_context/IncidentSummary | Complete actual operation facts and direct Classification/derived recovery context. |
| Prospective capacity bounds/reservations | Actual bounds and semantic recovery usage only. |

The third attempt contains these redundant mechanisms. Remove those present in the affected core;
do not import later owner additions just to delete them:

| Third-attempt mechanism | Required deletion proof |
| --- | --- |
| Independent Phase plus OperationFacts and TerminalFailure copy | One authoritative operation record; no phase reconstruction/agreement code or duplicated outcome payload. |
| ObjectSeed and enclosing struct/enum/collection seeds | One ordinary wire-shape parse and one direct current-record admission route with actual native error evidence. |
| Fallible NativeCause project/from_error_with and child Wire projectors | Capture once, serialize existing data, reuse admitted original bytes. |
| Secondary projection and generic successful-result reporting custody | Fixed capture omissions, existing invocation/head custody and the actual terminal output failure. |
| Per-caller upstream sanitizers and blanket native-owner certification | The explicit upstream trust boundary and ordinary handling of MFM-owned secret inputs; no replacement certification framework. |
| ReturnedFailure Object-plus-native pair and native E retained until COMMIT | The complete admitted Failure/Object reused for persistence and reporting; native E is held only across fallible initial encoding. |

Count a second-table removal against `aae81795` only if it actually exists there. A mechanism
introduced solely in later owner work earns no core deletion credit. These corrections are not
additional deletions from `7f71beef`. Hypothetical avoided registries, fault schemas, audit stores and decoder
identities likewise earn zero baseline deletion credit. Include every replacement, helper, public
API, schema, test/doc migration and untracked addition in the respective cost report.

Part 1 completion requires the corrected core, E1-E6/C1-C18 evidence, G1 acceptance, current
documentation and F1 verification. The separate Part 2 matrix is excluded. A smaller file, passing
local proof or explanatory cost document alone is insufficient. Do not describe unimplemented
assumptions, deferred owner losses or secret-withheld detail as complete raw preservation or
achieved net simplification.

This RFC revision changes no production Rust and discards no engineer work. Verification is source,
commit, measurement and document-contract review plus `git diff --check`. It performs no new Rust
probe or CI and does not treat the previous ten-test probe as evidence for K1-K3.

## 15. Material uncertainties

The user's execution/recovery decisions remain settled. The following assumptions are open core
proof obligations. Section 12 authorizes bounded correction to resolve them, not an unconditional
full implementation handoff. Part 2 has its own uncertainties and refinement gate.

| Assumption | Why uncertain | Consequence if wrong | Validation and response |
| --- | --- | --- | --- |
| The unified record removes more mechanism than its dispatch requires | The former design serialized derived facts and accumulated agreement checks; the corrected production diff does not exist | A renamed validator could preserve the same complexity | K1 replaces the real core route, deletes the old pair and compares retained behavior, encoded occurrences and actual code. |
| Shape-only carriers and direct admission preserve causes without decoder machinery | Checked ID/getter and nested Object contracts must change; the previous Serde probe omitted admission | Hidden unchecked values, repeated validation or another grammar family could appear | K3 finalizes actual field/getter signatures and exercises native failure plus live/restored typed adapters. Failure returns to architecture before further expansion. |
| Captured/admitted detail can replace projectors and the duplicate native original | A successful encoding does not establish which native-only facts it omitted | Required cause information could disappear or a second representation survive indefinitely | K2 compares the exact selected facts across native failure, admitted value, cold observation and append-failure report; resolve missing facts before deleting the copy. |
| Existing shared schemas/admission suffice for the selected diagnostics under the trust policy | DiagnosticEvidence is closed and ordinary Values strings are secret-marker scanned | A required upstream text field might be rejected or provoke per-provider workarounds | Freeze required fields; if text is needed, design one bounded shared field/admission change before coding. No speculative schema expansion or global relaxation of Program/context checks. |
| Part 1 alone meets the simplification objective | The measured core is +1,222 production lines against the original baseline; the correction is not implemented | A smaller third attempt could still leave an unjustifiably larger core | G1 reviews actual Part 1 removals, replacements and total cost against both baselines. Unsupported net reduction or complexity improvement keeps Part 1 unaccepted; no credit from future Part 2 deletions. |

Normal loading deliberately trusts past live progression and append-only storage; it does not
verify historical semantic evolution or older frame links. Internal errors remain outside history.
The agreed upstream diagnostic trust boundary is distinct from deliberate inclusion of MFM secret
inputs and from post-crash delivery.
Those are explicit contracts, not hidden implementation uncertainties.
