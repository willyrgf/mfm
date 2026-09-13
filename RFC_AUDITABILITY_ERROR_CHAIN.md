# RFC part 1: current run continuation and persistence

Status: bounded K1-K3 correction; full implementation readiness remains unproved. Acceptance
requires their proofs, G1 review and F1 verification. Reviewed 2026-09-13.

Part 1 delivers current continuation/persistence and the invocation error boundary.
[Part 2](RFC_CAUSAL_ERROR_PRESERVATION.md) addresses selected execution-error producers after
Part 1 is accepted; its completion is independent. Continue on the current branch from the
implementation state pinned in section 3, using this RFC's current revision.

Read sections 1-3 for the objective, required behavior and starting point; sections 4-10 for the
contracts; section 12 for ordered execution; and sections 11, 13-15 for evidence, deletions and
remaining proofs. Earlier diagnoses and measurements are optional background in the
[implementation history](docs/auditability-implementation-history.md), not prerequisites or scope.

[Design](docs/design.md), [architecture](docs/architecture.md), [code quality](docs/code-quality.md),
and [AGENTS.md](AGENTS.md) govern the repository. Implement the contract changes proposed here with
their owning documentation and consumers in the same cutover. The [adapter error audit](docs/adapter-error-audit.md)
is evidence of gaps; only section 10's selected conversions and supplied facts belong to Part 1.
Unselected losses are not completion conditions. Section 4 defines the diagnostic trust contract.

## 1. Objective and target

Part 1 owns the current record, admission/load/append, execution/recovery ordering, concrete
invocation diagnostics, and their existing Application/CLI/REST consumers. Part 2 owns extending evidence
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
Use two explicit adaptation boundaries: concrete owner failures enter persistence as their declared
error types; internal failures cross heterogeneous interfaces as already constructed invocation
diagnostics. Receivers forward those facts and thin clients choose presentation. Neither boundary
requires opaque native-error custody, a projector, or a second error tree.
A source needed by a declared operational error enters that owner's typed payload before any
invocation-only adaptation. Never reconstruct a classifiable original from diagnostic JSON.

Continuation still needs a cursor, current input, checkpoint inputs, recovery usage and unresolved
Effect authority. The current operation supplies the cursor/input; do not store a second copy.
Deleting the old snapshot/fold machinery does not delete the necessary continuation facts.

## 2. Required behavior

### 2.1 Design requirements

| Decision | Consequence |
| --- | --- |
| Remove unnecessary qualified/cold/native-cache representations | One non-generic serializable RunRecord uses one Values carrier. No native cache or generic raw/qualified state pair; section 6.2 must prove the carrier admission API. |
| Reuse the execution state on restoration | The exact same Rust continuation type is encoded and decoded. Executable State objects, adapters, signers, and secrets remain outside it. |
| Remove duplicated responsibility | Journal has no lifecycle types or semantic validator; Runtime has no second wire model; Store does not interpret Runtime payloads. |
| Remove the fold and historical transition scan | No load path calculates historical successors, reconstructs counters, or compares past transitions. Current-state validation remains. |
| Persist concrete State-domain and adapter operational errors | Each owner selects its concrete error with actual causal payload. Commit it before classification/handler/mapping; restore that same declared type for classification. |
| Report system failures outside history | No durable internal-fault variant, internal-error Program identity, callback-error schema registry, or fault-specific stop lifecycle. |
| Remove adapter_context and IncidentSummary | Preserve actual input, intent/command, original error, and direct Classification in their owners. Remove context-only and summary-only callbacks/types. |
| Remove future-capacity admission | Enforce actual object/frame/run ceilings and semantic recovery limits; delete lifecycle size predictions and capacity-only quotas. |
| Keep acknowledged history append-only | No rollback, old-format reader, compatibility path, or rewriting previous frames. |
| Keep recovery action authorization durable | Commit a normal recovery decision and resulting state before executing the action. A recommendation alone is not authority. |
| Scope auditability to execution | Preserve selected State/adapter execution failures and necessary recording/report forwarding. No comprehensive startup, config, deployment, ingress or constructor migration. |
| Trust upstream diagnostic content | No generic credential detection, sanitization or secret-free certification of dependency-supplied diagnostics. MFM does not deliberately append its own secrets or full request/connection objects. Bounds and accurate omissions remain. |
| Adapt at the required boundary | Keep concrete library errors where their interfaces support them. Adapt selected internal cause facts once into immutable InvocationDiagnostic data; Runtime/App forward it and clients render it. Delete NativeCause and its opaque owner/projector protocol; no `Box<dyn Error>` replacement. |
| Reuse the admitted original | The complete admitted Failure/Object serves persistence and reporting. No duplicate native original or parallel fallback payload accompanies it. K2 proves the selected cause facts survive admission. |
| Report failed initial encoding honestly | Use the ordinary internal invocation diagnostic for the encoding cause, known execution/contract context and unavailable original detail/identity. No additional Runtime variant for this case, arbitrary native-original custody, serializer retry or new producer fallback contract. |
| Keep owner classification | Self::Failure and C::OperationalError are concrete owner errors. MfmValue supplies persistence, not erasure; invocation diagnostics never replace these errors or enter classification. |

### 2.2 Resume and loading semantics

| ID | Contract | Required behavior |
| --- | --- | --- |
| D1 | Retry from the last committed state after an internal failure | End the failed invocation without append. Explicit resume retries the unfinished acknowledged phase; no hidden permanent-stop marker or automatic retry loop. |
| D2 | Commit settlement first; resume interpretation only | Persist accepted Effect evidence and its awaiting-interpretation continuation before calling the interpreter. An interpreter failure leaves this phase available for explicit resume without adapter reentry. |
| D3 | Load admission and latest state only | One consistent Store read returns admission, latest committed frame, and head metadata. A requested exact-candidate sequence probe is allowed for append reconciliation; ordinary loading performs no full-prefix scan. |

### 2.3 Error routing depends on meaning, not the crate name

A State's declared business failure and an adapter's declared operational failure belong in the
run history. Their concrete owner types remain authoritative for classification. Commit declared
outcomes before recovery; return internal/recording diagnostic data without another append.
A State bug, adapter invariant violation, decoder rejection, handler/map/task failure,
Store failure, startup/configuration failure, or response-delivery failure is a system failure and
belongs in the invocation report. Ordinary startup/configuration/request failures keep their
existing handling; listing them here does not require comprehensive causal enrichment.

The same lower-level database or signer source can occur in either route. A signer rejection nested
inside a declared transaction operational error remains part of that persisted operational error.
A database failure while appending that error is a separate system/recording failure. Do not discard
a cause based on its source library, or turn an internal adapter bug into a recoverable incident.

## 3. Starting point and work boundary

| Item | Handoff contract |
| --- | --- |
| Working tree | Continue on the current branch with both current RFCs. Do not reset or restart the implementation; preserve unrelated work. |
| Implementation baseline | 5de114d0 pins the existing implementation for Part 1 cost and deletion measurements. It is a comparison reference, not an instruction to check out an older RFC. |
| Original comparison baseline | 7f71beef is used only to measure cumulative simplification. It is not a restart point or an alternative contract. |
| Already implemented | Current-state persistence, opaque Journal, admission/latest/optional-probe Store loading, and failure/settlement-before-recovery/interpretation ordering. Preserve these behaviors and complete their current corrections. |
| Remaining core work | K1 unifies duplicated continuation data; K2 replaces native error custody/reporting machinery; K3 proves direct Object/current-record admission. Section 14 identifies their concrete deletions. |
| Allowed next work | Execute K1-K3 in section 12, then obtain G1 acceptance and complete F1. Part 2 owner implementation starts only from accepted Part 1 after its own R0 design refinement. |

Measure Part 1 against both listed baselines, including all replacement code. Compare source using
one consistent production-LOC convention and report tests/docs/churn separately. Deletions already
present at the implementation baseline are retained work, not fresh reduction against that baseline.
The original comparison prevents a smaller local change from being mistaken for net simplification.

Scope is the current contracts and finite E1-E6 cases. A changed caller needs correct forwarding;
it does not authorize following every upstream producer. New producer facts, schemas or mechanisms
require an explicit design/cost decision before expansion. Section 11.3 defines acceptance and the
response to failed proofs or unsupported complexity. Part 2 estimates cannot justify an unfinished
or growing Part 1.

## 4. Error preservation and the diagnostic trust boundary

### 4.1 What must survive

Sections 4-5 and 9 define one shared contract for both RFCs. Preserve the original failure's
available cause layers, operation and the fields required by its selected execution case. A local
failure can have no upstream source; Pending and expected absence are not invented errors.
Classification operates on the whole concrete declared owner error, including its causal context.
For internal invocation failures, the selected diagnostic facts are the preservation contract;
there is no requirement to return arbitrary downcastable Rust error objects. Failed initial
encoding has the explicit unavailable-original contract in section 9.2. Public codes alone never
stand in for required causal facts.

Capture at the producing boundary before a lossy conversion. Receiving layers carry the supplied
cause and add context only for an actual distinct operation; they do not inspect every producer
their callees use. A source already erased by an MFM mapper cannot be recovered downstream. If a
selected case promises that source or SQLSTATE, the producing conversion must be designed and
fixed before that case passes. Our own loss is not evidence that the dependency never exposed it.

### 4.2 Responsibility for diagnostic content

MFM trusts dependency-supplied diagnostic content. This RFC does not require detecting arbitrary
credentials in provider/database error text, sanitizing every native source, or certifying that
all fields/getters/downcasts reachable from a dependency error are secret-free. This is a trust
choice, not a claim that dependencies cannot return sensitive text.

MFM remains responsible for what it deliberately adds: do not attach its own passwords, private
keys, mnemonic inputs, credentials, full requests, connection objects or keystore commands as
error context. Preserve existing handling of these explicit secret inputs. Do not dump whole
client/request objects to avoid designing an error conversion. Returned diagnostic text is not
permission to append extra application payloads.

No blanket withholding of provider/database messages, native-owner certification, generic secret
scanner, recursive disclosure audit or per-caller sanitizer family is required for upstream
diagnostics. Bounds, canonical encoding and accurate failure/acknowledgement semantics still apply.
Update the affected wording in docs/design.md,
docs/architecture.md, docs/code-quality.md and AGENTS.md in the implementing contract cutover;
conflicting diagnostic wording in those guides must not extend the implementation scope.

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
| Original not admitted | A declared failure was returned but could not be encoded/admitted. Original contents and canonical identity are unavailable; retain known operation/contract context and the encoding cause. Do not retry the serializer or retain an opaque original. |

Keep known counts/size facts only when exposed. A bounded, deliberately partial or erased
representation is not complete raw preservation. Optional detail failure does not replace the
original execution failure or become a new recursively reportable operational cause.

### 4.4 One representation at each necessary stage

| Stage | Representation and responsibility |
| --- | --- |
| Inside an owning library | Keep its existing concrete error and sources while the typed interface supports them. Do not introduce an adapter at every call. |
| Declared execution failure | The State's Self::Failure or capability's C::OperationalError owns its actual causal payload and classification. Capture required external facts at their producing boundary, before a lossy conversion. |
| Internal failure crossing a heterogeneous interface | Adapt selected fields to InvocationDiagnostic once while the concrete error is known. It contains immutable report data, no native owner, downcasting interface or stored callback. |
| Encoding a declared failure | The concrete value may move into the immediately awaited pure encoding job. Failure uses the ordinary internal invocation diagnostic with unavailable original detail/identity; no special recording variant, outside-job Arc custody or fallback payload is required. |
| Complete admitted failed outcome | Reuse the existing Failure/Object and operation facts for persistence, restored classification and append-failure reports. No second native original survives admission. |
| Runtime/App/transport reporting | Add actual execution/head/disposition context and forward supplied data. Thin clients choose presentation without rerunning extraction, constructors, Values admission or child projectors. |

Completeness means the selected cause/field contract with explicit bounds and omissions, not the
private contents or identity of a dependency's Rust object. K2 compares those facts in the concrete
owner, admitted value, restored error and append-failure report. Missing promised facts require a
specific producing conversion/schema correction, not duplicate custody. The accepted failed-initial-
encoding exception does not weaken normal admitted-original or failed-append preservation.

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
InvocationDiagnostic belongs to Values at the existing heterogeneous value/decoder boundary; it
uses only existing foundational/Serde data. Values must not depend on Diagnostics. Section 9
defines this invocation-only data separately from the existing persisted DiagnosticEvidence schema.

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

### 5.2 Concrete owner errors supply classification

A declared State failure is the State's selected concrete owner error. It retains its actual
causal payload and provides intrinsic classification. The existing associated-type contract is:

```rust
trait State {
    type Input: MfmValue;
    type Output: MfmValue;
    type Failure: ClassifyError;
    // Existing identity and execution interfaces remain.
}
trait ClassifyError: MfmValue {
    fn classify(&self) -> Classification;
}
```

Self::Failure can be an existing domain error shared by related States. Generic encoding notation
such as E: MfmValue does not erase that concrete type: MfmValue supplies its schema/persistence
capability. Keep the associated types and ClassifyError; add no new failure trait, PersistedError
wrapper, associated-type rename or error enum per State. A local failure with no upstream cause
need not invent one. Where a cause exists, its promised information belongs in the owner payload.

Runtime commits the original, then decodes that same declared type for classify(), both live and
after restoration. An invocation diagnostic, formatted message, cached classification or mapped
root cannot replace the original. Classify the whole owner error, not automatically its deepest
source: a provider failure in a duplicate-safe Read can be Retryable, while the same cause during
transaction submission is OutcomeUnknown. The owner's operation changes the safe inference.

An adapter independently supplies C::OperationalError. Runtime requires its ClassifyError bound
at association; do not move that trait into capabilities or add a State wrapper around an adapter
failure. Internal State/adapter failures remain invocation-only and never enter classification.

Keep operation/stage and checked facts at their existing owner. Ordinary typed source nesting and
borrowing remain useful; std::error::Error is not a universal bound, persistence contract or
classification mechanism. Remove incidental Copy requirements only where owned causes require it.
A Box around a concrete payload is not erased native ownership. Do not duplicate Classification
as a mutable owner field. Part 1 preserves already supplied payloads; selected producer enrichment
belongs to [Part 2 section 3](RFC_CAUSAL_ERROR_PRESERVATION.md#3-owner-representations).

### 5.3 Capture vocabulary and extraction

The omitted-field vocabulary is closed: Message, Data, Body, Url, DatabaseDetail, Parameters,
PanicPayload, and SourceDetail. Reasons are Withheld, Unavailable, and BoundReached. Observed sizes
are integers only when known. SQLSTATE is five checked ASCII alphanumeric bytes; parser locations
retain reviewed line/column/offset. Unknown OS codes may remain numeric; unknown source categories
remain opaque while accessible deeper sources retain their own layers.

Capture walks exposed source links with the selected owner's extraction, not a shared registry
of client downcasters. The diagnostics crate imports no client libraries. Returned diagnostic text may be retained under section 4.2, but never parse messages to
infer their category, and never walk an unbounded discarded suffix to count it. For this persisted
DiagnosticEvidence, reuse the existing checked capture implementation and consuming tests. The
invocation-only boundary in section 9 does not replace or duplicate this schema/capture service.

## 6. One authoritative current record

Object is the common value carrier inside the live continuation and its stored representation.
It erases a value's concrete Rust type while retaining its schema identity, content hash and
encoded data. Runtime associates typed implementations, admits values against their selected
contracts and decodes each concrete value at typed use. Object does not select implementations or
schedule transitions. States and adapters still receive and return their concrete Rust values.

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

Serde's generic error conversion can erase a checked Object constructor source inside a derived
parent decoder. Separate wire-shape parsing from checked admission so the selected cause facts
survive without repeating the parent payload grammar through seeds.

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

This changes the current getter contract: Object cannot return a permanently checked &ContentRef.
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
claim completion of the decoding proof. Ordinary Serde and arbitrary nested native-cause
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
enum Failure {
    Domain { call: StateCall, original: Object },
    Read { call: Call, intent: Object, original: Object },
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
Inline DomainFailure/ReadFailure payloads here as the TerminalFailure copy disappears; retain a
separate wrapper only for demonstrated independent reuse, not a differently named extraction view.
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
| Candidate encoding/actual-limit failure | Append nothing. Report an already admitted original and recording cause; failed initial error encoding reports unavailable original detail under section 9.2. Leave the acknowledged state authoritative. |

Only NotInserted triggers Runtime's automatic reconciliation read in this delivery. Store errors
retain the baseline immediate-return behavior; the invocation holds the candidate for caller
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

1. Obtain the concrete declared owner error, with its causal payload, and executed input/request.
2. Build and append the Failed record, which derives AwaitingRecovery.
3. After known insertion, decode the admitted original as Self::Failure or C::OperationalError,
   classify that whole owner error, and invoke the selected handler.
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

## 9. Boundary adaptation and invocation diagnostics

### 9.1 Adapt concrete fields once; forward data

Keep concrete Value/Journal/Store/library errors where existing typed interfaces can carry them.
At a heterogeneous callback, decoder or port boundary, the source owner selects the required code,
operation, fields and cause layers while their concrete types are known. One InvocationDiagnostic
then crosses Runtime/App to the thin client. Classification and durable originals still follow
section 5.2; invocation data is neither a declared failure type nor a new persistence format.

Delete NativeCause, Captured, `Owner<E>`, from_error/from_original/from_error_with, projectors,
arbitrary native downcasting, and companion owner Wire trees created solely to project children.
Do not replace them with `Box<dyn Error>`, Any custody, another dynamic trait, serializer registry or
one global enum of every owner's errors. A borrowed Error::source() walk inside an owner may help
capture exposed layers; no erased native owner or deferred extractor crosses this boundary.

The Values-owned target uses immutable JSON data, avoiding a second parsed JSON tree:

```rust
#[derive(Debug, serde::Serialize)]
pub struct InvocationDiagnostic {
    code: &'static str,
    operation: &'static str,
    details: Box<serde_json::value::RawValue>,
    size: Option<SizeViolation>,
}
impl InvocationDiagnostic {
    pub fn from_fields<T: serde::Serialize + ?Sized>(
        code: &'static str,
        operation: &'static str,
        fields: &T,
        size: Option<SizeViolation>,
    ) -> Self;
    pub fn code(&self) -> &'static str;
    pub fn operation(&self) -> &'static str;
    pub fn details(&self) -> &serde_json::value::RawValue;
    pub fn size(&self) -> Option<SizeViolation>;
}
```

This is the target shape, not a proved constructor implementation. Fields stay private and the
value has no native-owner access, MfmValue identity or custom error trait. Move or borrow it through
existing error variants; sharing immutable data is allowed only where an actual owner needs it.
Implement ordinary Display/Error for existing typed error wrappers without an erased source;
the retained causal layers are accessible in diagnostic data. No source traversal during rendering
or recursive error implementation is required.
A local Serialize helper for selected fields is ordinary boundary code, not another error family.
Reuse an owner's existing serializable data when it already expresses exactly the selected field
contract; do not create a companion DTO merely to pass it to from_fields. This is not permission
to use an arbitrary native error's serializer as an unspecified diagnostic contract.
No consumer calls Serialize on an arbitrary original Rust error to discover its required details.

For E4, retain field, expected and actual identity facts, or the concrete
parser category/location. E5 retains the constructor case, metadata.correlation location and,
for an excessive length, limit 256 and actual observed bytes. Owner codes and fields belong to
those selected contracts. An opaque code or message alone does not satisfy these cases. Existing
DiagnosticEvidence can be included by its owner as already captured source data; Values imports
neither that crate nor any client library. Part 1 adds no persisted diagnostics schema extension.

Move the existing Serialize-only SizeResource and SizeViolation declarations together from Runtime
to Values and update their actual consumers. Reuse the same measured-versus-serialization-lower-
bound variants and public spellings. Do not add SizeDiagnostic, move Diagnostics::ObservedSize,
or create a second size hierarchy. Populate size when the concrete failure is known; delete
cause_size and its cross-crate source traversal/downcasts. Receivers read supplied facts directly.
Runtime adds its own actual execution operation/stage once; owner operation context remains intact.
No secondary reload/report failure can replace the primary disposition or size evidence.

Construct selected field JSON once with the existing bounded canonical writer before creating
RawValue. No unbounded to_value(error), whole-object dump, serialize-then-size-check, or stored
Serialize callback is allowed. The complete invocation diagnostic uses the existing 32 MiB ceiling;
account for its fixed code/operation/size metadata before writing variable detail. The source owner
supplies fixed reviewed identifiers. Keep the existing 8 KiB DiagnosticEvidence budget unchanged.

If selected-detail encoding exceeds the bound, fails, or unwinds, keep the original code/operation
and primary size facts. Substitute one fixed small details object with reserved omitted metadata:
reason bound_reached, encoding_failed or panicked, and actual limit/observed-at-least facts when
known. Owner detail contracts reserve this omission spelling. Drop incomplete JSON. Do not repair
it, retry the serializer, traverse discarded suffixes, or create another diagnostic to describe
this omission. No CapturedDetail/CaptureFailure/ProjectionPanicked error family is needed. A detail-
size omission does not turn the primary failure into a size error or change its transport status.
K2 must prove whole-diagnostic bounds, fixed omission construction and immutable forwarding.

### 9.2 Failed initial encoding has an explicit information limit

A State returns its concrete Self::Failure and an adapter its C::OperationalError. Runtime's
generic encoder does not change their ownership or classification meaning. If that original
cannot be encoded/admitted, however, its persistence interface supplies no independent way to
recover arbitrary contents. The user accepted this contract:

- Report that a declared failure was returned but its original detail and canonical identity are
  unavailable. Retain the known operation, failure slot/contract when already available, last
  acknowledged state and concrete encoding/admission diagnostic.
- Move the concrete error into the immediately awaited pure encoding job if convenient. It may be
  dropped when the attempt ends; no `Arc<E>` or native owner must survive outside that job.
- Do not retry its serializer, invent a canonical identity, classify before commit, invoke policy,
  or require a fallback payload/new trait/Error bound from every producer.
- A successful result whose encoding fails has only an encoding/recording failure. Do not invent
  an original execution error or retain generic successful-result custody.

Use the existing internal RuntimeError route with the actual operation, Stage::Encode and one
InvocationDiagnostic. No EncodeOriginal variant, encoding-target enum or additional error/context
wrapper is required. Neither recovery nor transport routing branches on the fact that the encoded
value was a declared failure. The actual encoding/task cause and its size facts remain primary.

The private encoder returns the admitted Object directly:

```rust
async fn encode_failure<E: MfmValue>(
    error: E,
    operation: Operation,
    position: ExecutionPosition,
    failure_contract: &ContentRef,
) -> Result<Object>;
```

Supply position and the already associated failure contract at this boundary, including through
the private erased adapter callbacks; public provider callback arguments remain unchanged. The
contract comes from association, not a second call to the failed value. At encoding/task failure,
construct selected diagnostic fields once from the concrete cause and this known context:
`encoding_target` is `declared_failure`, `position` and `failure_contract` identify the attempted
slot, `original_detail` and `original_identity` are `unavailable`, and `encoding` holds the selected
cause fields. Keep the last acknowledged head in the existing invocation context. Section 9.1's
fixed omission replaces detail if constructing it fails or exceeds its bound; primary code,
operation, size and the existing invocation head remain intact. No receiver reopens the original
or recaptures an already constructed diagnostic to supply these facts.

After complete admission, the same Failure/Object serves persistence, restored classification and
append-failure reporting. It must contain the selected promised causal facts: prove those facts
rather than infer completeness from successful serialization. Delete ReturnedFailure and its
Object-plus-native pair. No native original remains through COMMIT, and report construction does
not decode the candidate frame or reserialize its original. This explicit encoding exception does
not permit omitting supplied facts from normal declared originals or invocation diagnostics.

### 9.3 Callback and decoder boundaries

Keep the existing native-construction entry point MfmValue::decode_native, returning
Result<Self, InvocationDiagnostic>. The name describes construction of a typed value, not native
error custody. Its default adapts the parser's available category/location/source data. The
selected E5 hook returns its actual constructor case/location/length data before Serde can flatten
it; E4 admits actual Objects directly under section 6.2. No extra error schema/identity is needed.
Do not promise to recover a cause already discarded by an arbitrary user Deserialize implementation
or use that limitation to discard one supplied by the selected hook.

Replace NativeCause at the existing heterogeneous internal boundaries directly:

| Boundary | Result |
| --- | --- |
| PureState::evaluate(Input) | Result<ProposedStateOutcome<Output, Self::Failure>, InvocationDiagnostic> |
| ReadState<C>::prepare(&Input) | Result<C::Intent, InvocationDiagnostic> |
| EffectState<C>::prepare(&Input) | Result<C::Command, InvocationDiagnostic> |
| ReadState<C>/EffectState<C>::interpret(Input, &C::Evidence) | Result<ProposedStateOutcome<Output, Self::Failure>, InvocationDiagnostic> |
| Handler::handle(&Params, Classification, &RecoveryContext) | Result<RecoveryRequest, InvocationDiagnostic> |
| ValueMap::apply(&Params, Input) | Result<Output, InvocationDiagnostic> |
| ClassifyError::classify(&self) | Classification of the concrete declared owner error; deterministic and infallible, unchanged. |
| AdapterError<E> | Operational(E) or Invariant(InvocationDiagnostic); E is the capability's concrete error, with ClassifyError required at Runtime association. |
| Evidence binding | Existing exact intent/command/evidence/ref arguments with InvocationDiagnostic on internal rejection. |

Keep concrete errors and source-preserving conversions inside compatible library interfaces.
Only an actual boundary needing invocation data gets an owner-local field adapter. No associated
internal-error types, universal Error bound, per-State wrapper, public downcasting contract,
callback-error ABI or second representation for declared originals is introduced. Existing erased
execution callbacks can remain; this removes erased error custody, not executable association.

### 9.4 Recording failure holds two related facts

A Read can time out and then fail to record that timeout because the database is unavailable.
When the Read original was admitted, the invocation reports that complete original and the separate
recording cause. The database failure did not cause the provider timeout. Neither cause is replaced
by the other's code, and there is no claim of a new committed state. Recovery waits for a committed
failed outcome; explicit resume follows the last committed state.

Keep RuntimeError::Recording for an admitted original, an exact candidate or a physical append
outcome. Return these failures through the existing InvocationFailure::Execution route. The
recording payloads are:

```rust
enum RecordingFailure {
    BeforeAppend {
        original: Option<Failure>,
        candidate: Option<EncodedRunFrame>,
        cause: InvocationDiagnostic,
    },
    Append {
        original: Option<Failure>,
        candidate: EncodedRunFrame,
        outcome: AppendFailure,
        observation: Option<(RunSummary, CandidatePresence)>,
        reload_cause: Option<InvocationDiagnostic>,
    },
}
enum AppendFailure { NotInserted, Store(StoreError) }
enum CandidatePresence { Present, Excluded, Absent }
```

BeforeAppend retains an admitted Failure and/or a sealed candidate. If neither exists, use the
ordinary internal diagnostic route; declared-failure encoding follows section 9.2. Do not construct
an empty recording container.
Share admitted immutable data where needed; no extra native original survives admission. A
candidate exists only after sealing, and Append keeps one exact candidate. Public output exposes
candidate identity/sequence/digest, not its body. K2 proves actual sharing without introducing
another error/original/candidate wrapper.

NotInserted is a physical disposition and need not invent a cause. Only it probes automatically
under section 7. Store errors, including Indeterminate, return immediately with the actual
disposition. Keep physical probe findings separate from latest-view admission. Do not recursively
audit a failed append, add a second durable sink or give an uncertain candidate execution authority.

App and both transports render the same supplied diagnostic/admitted data. JSON and text renderers
borrow the typed report; delete CLI Fields/MissingField and serialize-then-reparse text rendering.
Public encoding and writing can still fail: the existing transport owner retains the available
invocation/head and actual terminal encoding/IO cause, then ends there. Keep write-versus-flush and
stdout-versus-stderr facts. Known insertion stays acknowledged and cannot reopen work. No socket
delivery claim follows from REST handoff. Delete `ReportFailure<T>`, ReportStage, IncompleteReport,
ReportDetail and their duplicate omissions/head views; add no generic successful-result custody,
secondary projection attempt or recursive output-error report.

## 10. Closed Part 1 error scope

Part 1 proves the shared mechanism through the cases below, preserving already supplied causes.
Part 2 enriches only its selected execution producers. Startup/config/deployment/ingress and broad
constructor audits are outside both RFCs. E5 remains one explicitly selected execution-time decode
case; it does not authorize a general constructor migration. E4 preserves actual admission errors
without certifying the entire native parser/source object as secret-free.

| ID | Required core boundary and evidence | Stopping boundary |
| --- | --- | --- |
| E1 | Pure/Read/Effect preparation, execution/interpretation, handler, map and heterogeneous Runtime conversions forward their required InvocationDiagnostic data. Internal failure appends no fault and never enters classification; explicit resume follows acknowledged state. | Section 9.3 callbacks and their invocation consumer. No erased native owner, arbitrary Deserialize provenance, or new upstream error family. |
| E2 | A State's concrete Self::Failure and the shipping Read's concrete operational error retain their admitted original through failure-before-recovery and cold inspection. Decode that declared owner type for classification, preserving causal/operation context. Pending Effect preserves command/EffectId and settlement precedes interpretation. | Existing declared payload and exposed cause; selected provider/authority/signing enrichment belongs to Part 2. No per-State error wrapper or universal Error bound. |
| E3 | An admitted failed Read plus failed append reports the complete original and separate recording failure with actual acknowledgement. A non-Error declared failure whose first encoding fails uses section 9.2's ordinary invocation diagnostic and omission contract. Also cover success plus append failure, dispositions and terminal output failure. | Existing invocation/reporting route, one admitted original and candidate. No special first-encoding Runtime variant, serializer retry, native-original custody, producer fallback payload, native-success bag, reporting tree or second audit sink. |
| E4 | Actual nested Object reference/hash/schema failures preserve field and expected/observed facts through direct admission and concrete boundary adaptation to Runtime/App. Parser/canonical errors preserve available category/location/source facts using section 9. | Existing validation and affected consumers; no parent seed grammar, broad ID rewrite, arbitrary error downcasting or JsonError/CanonicalError source-certification project. |
| E5 | Shipping EvmBalanceContext metadata.correlation constructor: empty and excessive-length failures retain their case/location and required limit/observed length through the actual typed Runtime callback and Application diagnostic after structural/schema admission. | These two cases only; reuse/extract the existing constructor. Keep minimal local raw-to-checked helpers only where necessary; no other context/value family migration. |
| E6 | Consumers of changed Object, InvocationDiagnostic, shared size data, Store and callback APIs compile and forward supplied causes/fields/classification using shared rendering. | A needed forwarding/import update does not authorize inspecting every producer the caller uses or adding another adapter/schema/capture layer. |

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

Keep the existing error-category/status projection while retaining its supplied diagnostic data. In particular,
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
cause and known/unknown acknowledgement. Failed initial encoding uses the ordinary invocation
diagnostic with explicit unavailable original detail/identity, not an empty alleged original. Do not
put raw candidate bytes or native client objects in JSON. App and the transports borrow already
constructed/admitted data rather than extracting causes, reparsing text-rendering JSON or walking
size-error sources again. A postcommit projection/delivery failure retains known insertion and cannot reopen work.

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
limit after work occurred. Report the admitted original when present, actual size cause, and
unchanged head; first encoding failure uses section 9.2's unavailable-original contract.
Do not truncate required facts, manufacture a smaller error record, or create replacement Effect
authority. If an externally accepted settlement cannot be recorded, the acknowledged pending
command remains authoritative for later reconciliation; do not claim durable settlement.

### 11.2 Prove the complete Part 1 representation

Use the current Runtime entry points and consuming tests. Revalidate the corrected
representation on the small complete path: admission, successful State/context advance, failed Read
committed before a handler error, restoration, and explicit resume. Use the baseline's available
operational cause. This proves lifecycle/reporting integration; it does not claim that all upstream
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

For Part 1 acceptance, report the complete delta against both baselines in section 3,
including replacement code and untracked additions, separately from tests/docs. Name removed public
types, callbacks, code paths, and remaining change sites. The core must have one continuation declaration,
one object representation, one restoration route, and no semantic-history scan. A smaller fold file
or passing local test is not evidence if its responsibilities survive elsewhere.

Apply section 11.3's acceptance decision after these measurements. An explanation of growth does
not permit continued owner work. This is a review of the bounded correction, not permission for
another repository-wide salvage audit or discarding user work.

### 11.3 Core acceptance and scope control

Part 1 requires independent architect review of the complete implementation, actual diff and cost
against section 3's baselines. Passing a local test or explaining individual additions does not
establish aggregate simplification. The review must assess the complete design and its removals.

The reviewed packet is one concise current record: baseline/candidate commits, selected test results,
production delta and total additions/deletions/files including untracked work, removed/added types
and responsibilities, shipping bytes/load rows, E1-E6 results, and any known upstream gaps deferred
to Part 2. Keep this evidence with the Part 1 completion record; update it instead of accumulating
progress or scope-attribution documents. Part 2 estimates or resolved owner contracts are not
required to accept Part 1.

Acceptance must name actual removals from the implementation baseline and identify every replacement.
A renamed validator, another generic decoder family, a fallible projector in a different crate,
or missing promised original information outside section 9.2's explicit encoding exception fails
the gate regardless of LOC. Applying blanket upstream secret-free certification would contradict
the agreed scope. The corrected core must
remove the identified redundant responsibilities and demonstrate a smaller affected core against
the implementation baseline. Also report the original-baseline total: local reduction alone does
not establish the required cumulative simplification.

No unsubstantiated numerical forecast is established here. The architect must reconcile the
completed Part 1 delta and responsibility removals against both baselines. If Part 1's net reduction
or required complexity improvement cannot be supported, Part 1 remains unaccepted; future owner
work cannot supply hypothetical deletion credit. The engineer cannot waive this by describing
growth as necessary or by removing unrelated code/tests. Conversely, unfinished Part 2 estimates
or source contracts cannot invalidate an otherwise accepted Part 1.

## 12. Ordered work and authority to continue

Execute the bounded K1-K3 work below on the current branch under section 3. Keep already
implemented behaviors and their coverage. K1-K3 must pass before G1/F1 acceptance; this RFC does
not authorize Part 2 producer migrations during core work.

### K1: `remove duplicate continuation facts`

Replace Phase + OperationFacts with the section 6.5 record in the real Runtime path. Delete the
stored TerminalFailure copy, phase-reconstruction/agreement code and repeated payload accounting
at the same cutover. Inline DomainFailure/ReadFailure into Failure where their independent reuse
disappears; do not replace them with new wrapper views. Preserve authorization, checkpoint/usage/
Effect checks and observable public phase behavior. Reuse consuming lifecycle tests, deleting assertions for the superseded duplicated
wire. Measure actual removals and encoded occurrences before starting a new error owner.

### K2: `replace native custody with boundary diagnostic data`

First prove section 9's concrete data shape with real E4 identity/parser fields, E5 constructor
fields, an already supplied foreign cause from these selected paths, and measured/lower-bound
size evidence. This proves the diagnostic adapter; K3 proves its actual Object admission route.
Finalize whole-diagnostic bounded construction, encoding/panic/bound omissions and actual access/
sharing signatures. Verify omissions retain the primary code/operation/size and do not recursively
construct an error. Use the existing bounded writer; add no arbitrary-error serializer API.

Cut over the existing callback/decoder/Runtime/App/transport interfaces coherently to immutable
InvocationDiagnostic data. Move the same SizeResource/SizeViolation declarations into Values;
delete cause_size downcasts and keep status/resource spellings. Preserve concrete errors inside
compatible library interfaces, and keep Self::Failure/C::OperationalError as the classifiable
originals. The current selected facts define the work, not every source reachable from an error.

Exercise E3: admitted failed Read plus failed/ambiguous append, non-Error declared original whose
first serializer fails, success plus append failure, NotInserted observation and terminal output
failure. Assert the ordinary encoding diagnostic retains its actual cause/size and section 9.2's
known context and unavailable original detail. It triggers no append, serialization retry or
precommit classification. Otherwise preserve the complete admitted original, independent recording
causes and accurate acknowledgement. No outside-job `Arc<E>` or native-success custody is required.

Delete NativeCause/Captured/Owner, projectors and companion error Wire trees, ReturnedFailure,
ReportFailure/IncompleteReport and related reporting types, CLI Fields/MissingField reconstruction,
and superseded custody tests in the same closure. Retain tests of promised cause fields, bounds,
classification and delivery. Section 14 distinguishes real necessary data from removable wrappers.

Use the agreed upstream trust boundary. Finalize diagnostic/recording/context signatures here and
measure the whole replacement, including owner-local adapters and tests. Do not migrate PostgreSQL,
config or signing families, add diagnostic fields, or expand persisted schema to make K2 appear
comprehensive. A new fact contract needs a specific design before implementation.

### K3: `prove direct current-record admission`

Resolve section 6.2 using real Object/reference/hash/schema failures inside the current payload,
real typed adapter arguments and the E5 constructor consumer. Compile and exercise both live and
restored calls. Delete ObjectSeed and the enclosing seed grammar only when this actual admission
route preserves the required causes and object/identity invariants. The corrected Object getter,
framework field types and owner diagnostic adapters must be written as final signatures in this RFC.
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
policy, AGENTS ownership/trust and error-preservation wording, and affected transport contracts in
their owning cutovers. State the typed-owner classification rule and accepted unavailable-original
encoding exception; do not leave a universal native-custody requirement in an authoritative guide.
Remove superseded implementations, tests, fixtures and docs while retaining observable coverage.
Part 1 ends here; no owner row from
Part 2 is required or automatically authorized by G1.

Use the narrowest relevant checks under [build and verification](docs/build-and-verification.md),
managed PostgreSQL/SQLx checks for changed persistence queries and managed Effect/client scenarios
for changed execution boundaries. All direct Rust tooling runs in the default Nix shell. Run one
final `nix run .#ci` on the complete Part 1 candidate. If verification changes that candidate,
repeat affected checks and obtain review of changed contracts before recording acceptance.

Record the accepted implementation commit, finalized Object/RunRecord/InvocationDiagnostic/Store signatures
and source locations, C1-C18 evidence, real removals/additions and cost against both baselines, and
known omissions/deferred upstream gaps. Use one concise completion record in this RFC, not a new
progress-document family. Until completed, that record must say pending rather than cite a proof
or intermediate commit as accepted. At this revision: **Part 1 acceptance is pending.**

This completion record is the input to Part 2's refinement step. Part 2 may remain draft after Part 1
is complete. Once refined and accepted for implementation, it proceeds on the accepted Part 1 commit;
it does not restart the persistence cutover or replace Part 1's accepted design implicitly.

## 13. Acceptance evidence

These C1-C18 criteria apply only to Part 1. E1-E6/C15 define its error scope; Part 2's B1-B8 define
that independent delivery. Excluded platform migrations are outside both RFCs. Tests for removed
durable system faults and historical semantic validation are obsolete. Preserve retained behavior
in consuming tests rather than retaining obsolete fixtures.

All cause guarantees below, including C5/C9/C11/C17, preserve the selected causal facts through
changed core APIs and prove E1-E6's admission/constructor/original-retention contracts. C11 explicitly
permits unavailable original contents after failed initial encoding; it requires no opaque custody.
These criteria do not require enriching upstream unit/flattened errors outside that scope. Record existing gaps
in the relevant inventory; only selected execution gaps belong to Part 2. Never create a new loss
or fabricate a placeholder to pass a core case. Persistence, authorization and execution guarantees
remain complete for the supported core behavior.

| ID | Observable acceptance |
| --- | --- |
| C1 | Fresh and restored runs use the same RunRecord/object carrier types and produce equivalent continuation/output through a real Runtime entry point. Every current enum variant round-trips through canonical JSON with inline objects. No qualified cache, alternate wire state, object resolver, or post-encode native round trip. |
| C2 | Admission/latest loading uses one consistent snapshot. Increasing history length does not increase normal rows loaded. A missing required row, inconsistent head, bad frame/object hash, wrong Program identity, or invalid current phase/slot contract rejects before action. Repeating start with identical Program/input returns the checked observation without driving; different admission returns AdmissionConflict. |
| C3 | Current-record admission rejects invalid operation/mode/slot and request/outcome relations, checkpoint/barrier relationships, out-of-range counters and checked-sum overflow. There is no independent Phase or agreement validator. The run total has no independently stored counter. No historical successor construction or claim of detecting otherwise locally valid historical counter/checkpoint substitution remains. |
| C4 | Success carries complete context to the next State. Restart selects an eligible declaration checkpoint's stored input, creates a fresh visit, retains current usage/Effect restrictions, and never consults old operation frames. |
| C5 | E2's concrete State-domain and adapter operational errors commit original/input/request/evidence as applicable before classification, handler or mapping. The same declared owner error is decoded for live/cold classification, with its causal/operation context retained. No invocation diagnostic, cached classification or mapped root replaces it. |
| C6 | Handler/map failure returns its selected invocation diagnostic without a fault append; the original remains AwaitingRecovery. Explicit resume reruns unfinished recovery evaluation. An authorized Retry/Restart charges the failing declaration and run once, then yields. Stop/denial spends no grant. Pending Effect Retry preserves visit/command identity. |
| C7 | Effect preparation commits command/EffectId before adapter entry. Pending/error/Stop behavior preserves that same unresolved command and existing reconciliation restrictions; no replacement authority or capacity-only retry quota. |
| C8 | Accepted Effect settlement commits before interpretation. Injected interpreter failure appends no fault; cold resume interprets retained evidence without entering the adapter for that phase. Uncommitted settlement makes no durability claim. |
| C9 | E1/E4/E5 internal State/preparation/adapter/task/constructor failures retain required code, operation, fields and source facts as InvocationDiagnostic data and append no fault. No native owner/downcast/projector is needed by a receiver. Explicit resume retries permitted unfinished work; there is no automatic retry loop or internal-failure classification. |
| C10 | Local preflight mismatch performs no provider call/append; post-response binding rejection reports the true stage without appending unauthenticated evidence or falsely claiming no IO occurred. |
| C11 | E3 reports complete admitted failed outcomes with independent recording causes. Failed first encoding of a non-Error declared original uses the ordinary internal diagnostic: encoding_target, known position/contract and unavailable original contents/identity appear when detail construction succeeds; otherwise section 9.1's fixed omission applies. Primary cause code/operation/size and the invocation head survive either case. It retains no arbitrary E, retries no serializer, appends nothing and classifies nothing. Later stages reuse the same Failure/Object. Success plus recording failure invents no original execution error or custody stash. Candidate/disposition is retained once; Store errors return without probe or audit retry. |
| C12 | The NotInserted probe covers exact matching candidate, conflicting occupied sequence, candidate followed by later commits, absent candidate, and failed reload/projection. It retains the physical finding independently of the latest view and never drives further work. A candidate-absence snapshot cannot resolve a still-in-flight COMMIT; the Store-error invocation returns with its ambiguity/custody. Fresh resume may use current phase authority without claiming it resolved an unavailable prior candidate. |
| C13 | Atomic append, immutable earlier frames, exact-head conflict, checked cumulative bounds, consistent loads, and PostgreSQL acknowledgement ambiguity hold through Store public boundaries. |
| C14 | E5's empty and oversized metadata.correlation constructor failures reach the actual Runtime/Application report with location and reviewed length facts. K3 separately proves nested framework reference/hash/schema failures through direct admission and real adapter signatures, without parent seeds. No additional constructor family is implied. |
| C15 | The finite E1-E6 conversions and consumers preserve promised data, existing operational/internal routing and declared-error classification. InvocationDiagnostic construction is bounded while writing; size/encoding/panic omission terminates with fixed data and preserves the primary code/operation/size. Measured size and a serialization lower bound retain their existing projection. No NativeCause, opaque owner, projector registry, size-source downcasts, consumer recapture or generic fallback/reporting tree remains. |
| C16 | Actual oversize/frame/run rejection leaves the acknowledged head unchanged. Cover a failure that committed but whose recovery cannot fit, and an externally settled Effect whose settlement cannot fit. No future-capacity promise remains. |
| C17 | App/CLI/REST show the retained current result or E1-E6 supplied/required causal invocation report with section 10.1's exact statuses and accurate acknowledgement/delivery status, including known insertion followed by projection failure. They do not reconstruct history, generate internal-fault records, or deliberately append MFM secret inputs. Upstream diagnostic content follows section 4; broad startup/ingress enrichment is outside both RFCs. |
| C18 | K1-K3 have integrated deletion and retained-behavior evidence; G1 accepts the complete corrected core on its own cost and guarantees. F1 records the accepted Part 1 commit, finalized APIs, shipping bytes/load IO, cost against both baselines and final verification on that candidate. Part 2 readiness or completion is not required. |

No test calls a historical reconstruction implementation merely to compare it with the new one.
Use baseline observable behavior where it remains contractual, concrete expected transitions, and
hostile physical/current-state inputs. Update old-format fixtures under the single-current-design
policy; do not retain a reader to keep obsolete fixtures passing.

## 14. Deletion ledger and completion

Preserve these behaviors already present at the implementation baseline. They are not additional
work to repeat or deletion credit against that baseline:

| Established behavior | Contract to preserve |
| --- | --- |
| Current continuation | Store current execution facts; no semantic history scan or fold. K1 removes the remaining duplicate phase/facts representation. |
| Shared live/stored value carrier | No qualified/native cache; K3 completes the direct-admission API for the existing Object boundary. |
| Opaque Journal payload | Keep the exact envelope and inline Runtime-owned record, without lifecycle or object-resolution semantics. |
| Bounded Store loading | Admission/latest plus optional exact-candidate probe; physical append accounting remains. |
| Durable operation ordering | Original failure and settlement committed before their respective next operation. |
| Actual operation context | Keep input/request/evidence and direct Classification/derived recovery context; no separate contextualization service. |
| Actual capacity accounting | Enforce actual bounds and semantic recovery usage without future-capacity predictions. |

The current implementation contains these mechanisms to remove or consolidate. Apply the type
review to the affected boundary, preserving actual facts and meaningful tests. A fewer-type count
is not proof if another wrapper or duplicate responsibility appears underneath.

| Retained-core mechanism/types | Required correction |
| --- | --- |
| RunState/RunCommit, Phase, OperationFacts, TerminalFailure | One RunRecord/RecordedOperation; delete agreement code and duplicate payloads. RecoveryOutcome replaces RecoveryDecision. |
| ObjectSeed, CheckpointsSeed/Visitor, generated parent/collection seeds and macros in runtime/state/decode.rs | K3 proves direct admission of actual Objects, then deletes this grammar as a unit. Keep hostile-input/constructor coverage, not seed-specific tests. |
| NativeCause, Captured, `Owner<E>`, from_error/from_original/from_error_with, ProjectionPanicked and local omission serializer | InvocationDiagnostic contains bounded selected data, no erased owner or stored callback. No CapturedDetail/CaptureFailure subsystem replaces them. |
| Runtime RecordingFailure/RuntimeError Wire projectors and Values ValueError Wire projector | Serialize supplied data at the existing owner/report boundary; no recursive companion error tree. |
| ReturnedFailure and extra native-original Arc/custody | Reuse complete admitted Failure/Object. Failed first encoding reports explicit unavailable original detail without retaining arbitrary E. |
| App ReportStage, `ReportFailure<T>`, ReportDetail, IncompleteReport, duplicate Omission/ObservedHead; CLI Ordinary | Remove the generic reporting-failure framework. Use existing invocation/head/disposition and terminal output facts through one renderer. |
| CLI Fields/MissingField | Render text from typed report data; no JSON reparse or missing-own-field failure path. |
| DomainFailure/ReadFailure; duplicate owned AdapterIncidentView and App Incident/Original views | Inline Failure payloads where independent reuse disappears; borrow the authoritative error through one public view. No extraction-wrapper replacement. |
| InitialValueMismatch; TaskFailure | Reuse the identity-mismatch payload with correct operation/category, and existing TaskFailureKind in the actual Runtime error. |
| SizeViolation/SizeResource in Runtime; cause_size | Move the existing two data declarations to Values, update consumers and delete source/downcast discovery. No parallel size hierarchy or Diagnostics schema move. |
| AdapterReturn | Prefer the existing Result/AdapterError route with the admitted Object. Retain an extra private enum only if the actual callback proof shows simpler semantics; never retain ReturnedFailure duplication. |
| JsonError, CanonicalSource and canonical Cause companion; OutputIoError | Remove redaction-only/capture wrappers, retain native JSON/IO sources locally and adapt selected fields once. Consolidate the canonical owner where this removes duplicated variants; no unrelated canonical/IO redesign. |
| Per-caller upstream sanitizers and native-owner certification | Use the explicit upstream trust boundary and deliberate MFM-input handling; no replacement certification framework. |

The following data and small helpers earn their place through a concrete responsibility. Keep
the smallest visibility and implementation; this does not approve every existing method or copy.

| Retained type/data | Reason and limit |
| --- | --- |
| Object; RunRecord/RecordedOperation | One value carrier and one complete continuation for execution/restoration. No native cache, second state or persisted diagnostic envelope. |
| Call, EffectCall, Settlement, StateCall, Failure | Factor actual input/position, command authority, accepted evidence and original failure once. Distinguish valid operation facts without a bag of optional fields. |
| Checkpoint, StateUsage | Save authorized restart input and committed allowances. They do not restore a periodic-snapshot service, fold or historical counter scan. |
| RecoveryOutcome | The committed authorization differs from the handler request; reuse existing recovery vocabulary. |
| LoadedRun; AppendFailure, CandidatePresence, RecordingFailure | One consistent Store observation and accurate physical acknowledgement/recording facts; no second audit sink. |
| Driver, ValueContract, StateInvariant | One private executor/association/admission boundary. ValueContract replaces ValueCodec/ColdQualifier. Remove unused data and phase-agreement checks. |
| Journal Envelope, EffectId Preimage, bounded Writer | Exact wire/hash inputs and bounded accumulation have one concrete owner. No generic codec or accounting framework. |
| Operation/Stage, FrameOperation; OutputWriteError, OutputStream, WriteStage | Retain actual local operation and delivery facts once. Use an existing owner variant instead where equally clear; add no global error taxonomy. |
| Prepared, InvocationWire, candidate identity Candidate, Object output Wire | Tiny borrowing views are allowed for a real public wire distinction. Reuse the Object renderer instead of report-specific WireObject reconstruction. |
| BalanceMetadataError; BalanceContextDecodeError, MetadataWire/ContextWire and Object input Wire | Keep actual E5 constructor facts; inline the one-case location wrapper when equivalent. Prove the smallest local parse/admit helpers needed; no parent grammar or general constructor migration. |
| AdapterFailure, local TaskOperation and parser Base/Parse helpers | Keep selected actual adapter check/source fields. Private foreign-error adapters may be needed where Serialize is absent; no new public schema/registry or upstream library rewrite. |

DiagnosticEvidence and its vocabulary, CanonicalError, NoParams and the EVM provider owner types
are existing shared contracts. NoParams still supplies parameterless
policy configuration; CanonicalError remains the concrete canonical owner. Reuse these types;
do not count them as new concepts or hypothetical baseline deletion credit. Likewise, existing
FailureReport/FailureCauseView may render retained data but must not become another owned failure
tree. Test helpers are reviewed with their retained behavior; their count is not production growth.

Count each removal only against a baseline where that code exists. Use both baselines in section 3
and include the replacement cost. Hypothetical avoided registries, fault schemas, audit stores and
decoder identities earn zero deletion credit. Include every replacement, helper, public
API, schema, test/doc migration and untracked addition in the respective cost report.

Part 1 completion requires the corrected core, E1-E6/C1-C18 evidence, G1 acceptance, current
documentation and F1 verification. The separate Part 2 matrix is excluded. A smaller file, passing
local proof or explanatory cost document alone is insufficient. Do not describe unimplemented
assumptions, deferred owner losses or secret-withheld detail as complete raw preservation or
achieved net simplification.

## 15. Material uncertainties

The execution/recovery contracts remain settled. The following assumptions are open core
proof obligations. Section 12 authorizes bounded correction to resolve them, not an unconditional
full implementation handoff. Part 2 has its own uncertainties and refinement gate.

| Assumption | Why uncertain | Consequence if wrong | Validation and response |
| --- | --- | --- | --- |
| The unified record removes more mechanism than its dispatch requires | The implementation baseline contains duplicate phase/facts data; the unified production diff is not implemented | A renamed validator could preserve the same complexity | K1 replaces the real core route, deletes the old pair and compares retained behavior, encoded occurrences and actual code. |
| Shape-only carriers and direct admission preserve causes without decoder machinery | Checked ID/getter and nested Object contracts must change; the direct admission path is not implemented | Hidden unchecked values, repeated validation or another grammar family could appear | K3 finalizes actual field/getter signatures and exercises native failure plus live/restored typed adapters. Failure returns to architecture before further expansion. |
| Concrete invocation data can replace native custody with less machinery | Bounded construction, fixed omissions and actual owner adapters are not implemented | A generic capture framework or lost selected facts could reappear under a new name | K2 proves E4/E5/source fields, size/status, encoding/panic/bound termination and aggregate deletions. The unavailable-unencodable-original decision is settled; no opaque fallback restores the old promise. |
| Existing shared schemas/admission suffice for the selected diagnostics under the trust policy | DiagnosticEvidence is closed and ordinary Values strings are secret-marker scanned | A required upstream text field might be rejected or provoke per-provider workarounds | Freeze required fields; if text is needed, design one bounded shared field/admission change before coding. No speculative schema expansion or global relaxation of Program/context checks. |
| Part 1 alone meets the simplification objective | The final replacement cost and responsibility removals are not implemented or measured | Reduction against the implementation baseline could still leave an unjustifiably larger core overall | G1 reviews actual Part 1 removals, replacements and total cost against both baselines. Unsupported net reduction or complexity improvement keeps Part 1 unaccepted; no credit from future Part 2 deletions. |

Normal loading deliberately trusts past live progression and append-only storage; it does not
verify historical semantic evolution or older frame links. Internal errors remain outside history.
The agreed upstream diagnostic trust boundary is distinct from deliberate inclusion of MFM secret
inputs and from post-crash delivery.
Concrete owner errors remain the classification input; failed initial encoding explicitly reports
original detail unavailable, with no opaque native custody. Those are accepted
contracts, not hidden implementation uncertainties.
