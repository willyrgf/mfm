# RFC part 1: current run continuation and persistence

Status: Part 1 handoff awaits the focused design closures listed in section 15. Acceptance requires
implementation evidence, G1 review and F1 verification. Reviewed 2026-09-14.

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
invocation diagnostics, the shared diagnostic-data replacement in section 5, and their existing
Application/CLI/REST consumers. Part 2 extends evidence at its remaining named execution producers,
including their necessary recording failures. Part 1 acceptance requires its own actual simplification;
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
| Keep one checked value eraser at explicit boundaries | Object retains checked identity/canonical/hash/bound invariants. Use it only for heterogeneous continuation and private registered callback connections; typed operations keep concrete values. No raw/qualified pair, native cache or second value eraser. |
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
| Trust upstream diagnostic content | No generic credential detection, sanitization or secret-free certification of dependency-supplied diagnostics. MFM does not deliberately append its own secrets or full request/connection objects. Actual persistence limits and honest information limits remain; no diagnostic quota or omission ledger. |
| Adapt at the required boundary | Keep concrete library errors where their interfaces support them. Adapt selected internal cause facts once into immutable InvocationDiagnostic data; Runtime/App forward it and clients render it. Delete NativeCause and its opaque owner/projector protocol; no `Box<dyn Error>` replacement. |
| Reuse the admitted original | The complete admitted Failure/Object serves persistence and reporting. No duplicate native original or parallel fallback payload accompanies it. K3 proves the selected cause facts survive admission. |
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
| Remaining core work | K1 deletes decoder seeds while retaining checked Object; K2 unifies duplicated continuation data; K3 removes native error custody/reporting machinery; K4 removes the redundant Read-input decode. Section 14 identifies their concrete deletions. |
| Allowed next work | Execute K1-K4 in section 12, then obtain G1 acceptance and complete F1. Part 2 owner implementation starts only from accepted Part 1 after its own R0 design refinement. |

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
stand in for required causal facts. Malformed persisted framework data has the narrower decoding
contract in section 6.2: parser category, available location and rejection reason, without a
promise of structured nested constructor ancestry or size facts. This accepted boundary does not
weaken State/domain/adapter original preservation or direct typed construction diagnostics.

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

Part 1 replaces the existing persisted diagnostic schema with section 5's shared data and explicit
trusted-text profile. Its finite EVM/CLI recipes define the retained fields; the profile does not
authorize collecting every dependency field. Descriptor, JSON and canonical invocation conversions
reuse that data representation without making their invocation envelopes persistable. Required
semantic/classification fields remain typed in the owner. Ordinary Program/context input checks
remain at their existing boundaries; no provider-specific scrubber or global relaxation is needed.

### 4.3 Information limits are part of the contract

Preserve the exposed source chain and the facts selected in sections 5.3 and 9.6. Unknown native
source types contribute their available message and exposed child links; do not replace them with
an empty Opaque category. Hidden foreign-library attempts or fields cannot be reconstructed.
An MFM mapper that drops a promised exposed fact remains a gap at that producing conversion.
These scope limits belong in the contract and consuming tests, not per-error omission records.

A declared failure that cannot be encoded/admitted follows section 9.2: retain the encoding cause,
known operation/contract context and explicit unavailable original contents/identity. Append and
classify nothing; do not truncate the original, retry its serializer or retain opaque custody.
Malformed stored framework data follows section 6.2's narrower parser/category/location contract.
Invocation field conversion alone has section 9.1's two fixed failure markers; they never stand in
for a successfully preserved operational original. None of these cases needs an omission ledger,
source-count quota, discarded-suffix scan or recursive reporting of missing detail.

### 4.4 One representation at each necessary stage

| Stage | Representation and responsibility |
| --- | --- |
| Inside an owning library | Keep its existing concrete error and sources while the typed interface supports them. Do not introduce an adapter at every call. |
| Declared execution failure | The State's Self::Failure or capability's C::OperationalError owns its actual causal payload and classification. Capture required external facts at their producing boundary, before a lossy conversion. |
| Internal failure crossing a heterogeneous interface | Adapt selected fields to InvocationDiagnostic once while the concrete error is known. It contains immutable report data, no native owner, downcasting interface or stored callback. |
| Encoding a declared failure | The concrete value may move into the immediately awaited pure encoding job. Failure uses the ordinary internal invocation diagnostic with unavailable original detail/identity; no special recording variant, outside-job Arc custody or fallback payload is required. |
| Complete admitted failed outcome | Reuse the existing Failure/Object and operation facts for persistence, restored classification and append-failure reports. No second native original survives admission. |
| Runtime/App/transport reporting | Add actual execution/head/disposition context and forward supplied data. Thin clients choose presentation without rerunning extraction, constructors, Values admission or child projectors. |

Completeness means the selected cause/field contract and its stated information limits, not the
private contents or identity of a dependency's Rust object. K3 compares those facts in the concrete
owner, admitted value, restored error and append-failure report. Missing promised facts require a
specific producing conversion/schema correction, not duplicate custody. The accepted failed-initial-
encoding exception does not weaken normal admitted-original or failed-append preservation.

## 5. Shared causal data, with local ownership

Delete `mfm-diagnostics`. Values owns one small DiagnosticEvidence data type, reused as a nested
field of concrete persisted errors and as InvocationDiagnostic.details. It has no IO, client
library dependency, native owner, error trait, classification or recovery role. Object remains the
sole executable/persisted value eraser; this type carries diagnostic data only.

### 5.1 Minimal data and ordinary admission

```rust
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct DiagnosticEvidence(serde_json::Value);

impl DiagnosticEvidence {
    pub fn from_value(value: serde_json::Value) -> Self {
        Self(value)
    }

    pub fn as_value(&self) -> &serde_json::Value {
        &self.0
    }
}
```

The wrapper supplies a named persisted-field contract; from_value transfers ownership and
as_value borrows the same data. Private storage provides no mutation API. This is one JSON tree,
without an encoded-text copy, generic capture factory or standalone MfmValue/decoder identity.
Construction and Deserialize produce diagnostic data, not proof of Object admission. Neither
canonicalizes, validates persistence rules nor imposes a byte quota.

Implement Values' existing PersistedSchema for this type without adding a derive dependency to
Values. schema_identity() uses SchemaKind::PersistedContract, no semantic type id, schema name
mfm.diagnostics.diagnostic-evidence, version 2, and the existing CanonicalJsonTerminal shape with
CanonicalJsonProfile::DiagnosticFloatFree (wire diagnostic_float_free). validate() delegates to
validate_derived_persisted_owner(self, &Self::schema_identity()?). There is no second JSON grammar
or validator. Nested owner fields use the existing #[mfm(persisted)] path; ordinary whole-owner/
Object admission checks that declared shape, canonical bytes, floats and actual limits once at
its existing boundary. Keep semantic facts such as kind/method/stage in their typed owner fields;
JSON cannot replace executable inputs, declared error types or classification inputs.

Extend the existing canonical terminal validator with that one profile. Reuse GeneralFloatFree's
number and structural rules and existing canonical/global limits; only diagnostic text bypasses
the secret-marker check under section 4.2. GeneralFloatFree and ordinary String/Program/context
rules retain their existing policy. Use DiagnosticFloatFree for the derived FailureReport terminal
schema too: its embedded Objects have already passed their own typed admission, so the report
must not reinterpret trusted diagnostic text using a conflicting generic string policy. Advance
the report schema from version 4 to 5 and its internal domain to mfm.failure-report.v5 in the same
cutover; update affected owner/schema fixtures together. Test the whole error and report contracts,
not just a nested-field serializer. No legacy reader or second
report validation tree is introduced.

Remove the 8 KiB diagnostic budget and all layer/fact/omission counts, reservations, truncation
and completeness accounting. Actual Object, canonical, frame, run and terminal-report limits stay
with their existing owners. If complete operational data cannot be admitted, use the existing
internal encoding/admission failure route; do not persist smaller substitute evidence. The two
invocation-only field-conversion markers in section 9.1 do not apply to operational originals.

Delete the Diagnostics crate's manifest, workspace/dependency entries, code, obsolete tests and
README in K3. Move only EVM's existing two-case ObservedSize payload to its owning EVM module;
it retains its exact/lower-bound meaning and persisted field contract. Live keeps the already
checked reqwest::StatusCode through private helpers and serializes its numeric value at this
boundary: delete HttpStatusCode and AdapterFailure::HttpStatus's redundant reconstruction path.
Use task outcome strings panicked/cancelled in Runtime and Live instead of TaskFailureKind.
Delete the remaining source/fact/omission vocabulary, EvidenceError, builders and budget writers;
none is retained to support hypothetical future owners.

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
need not invent one or include DiagnosticEvidence. Where a cause exists, its promised information
belongs in the owner payload before Object admission.

Runtime commits the original, then decodes that same declared type for classify(), both live and
after restoration. An invocation diagnostic, formatted message, cached classification or mapped
root cannot replace the original. Classify the whole owner error, not automatically its deepest
source: a provider failure in a duplicate-safe Read can be Retryable, while the same cause during
transaction submission is OutcomeUnknown. The owner's operation changes the safe inference.

The ownership path is concrete owner error (including any nested DiagnosticEvidence) -> whole
error admitted/persisted through Object -> same concrete MFM owner restored -> owner.classify()
-> handler(Classification, RecoveryContext). A foreign client error exists concretely inside its
adapter; its selected external cause facts become diagnostic data there. Restoration reconstructs
the MFM owner and those facts, not a live foreign client object. Fields needed for classification
remain typed in the owner; the classifier does not parse diagnostic JSON. The handler receives
neither DiagnosticEvidence nor the original error directly.

An adapter independently supplies C::OperationalError. Runtime requires its ClassifyError bound
at association; do not move that trait into capabilities or add a State wrapper around an adapter
failure. Internal State/adapter failures remain invocation-only and never enter classification.

Keep operation/stage and checked facts at their existing owner. Ordinary typed source nesting and
borrowing remain useful; std::error::Error is not a universal bound, persistence contract or
classification mechanism. Remove incidental Copy requirements only where owned causes require it.
A Box around a concrete payload is not erased native ownership. Do not duplicate Classification
as a mutable owner field. Part 1 applies section 5.3's finite shared-data cutover; remaining selected
producer enrichment belongs to [Part 2 section 3](RFC_CAUSAL_ERROR_PRESERVATION.md#3-owner-representations).

### 5.3 Closed source-data recipes and extraction

Use the two existing extraction sites: [EVM capture](crates/live/evm/src/json_rpc/capture.rs) and
[CLI output](bin/cli/src/output.rs).
Their local helpers construct the selected scalar/source JSON directly, then call
DiagnosticEvidence::from_value. Values imports no client library and supplies no shared native
source walker, downcaster registry, arbitrary-error serializer or evidence-construction error
family. Existing concrete owner serializers still serve section 9's invocation conversions.

| Actual producer | Data retained in this cutover | Boundary and deletion |
| --- | --- | --- |
| EVM JSON-RPC response | response is null or an object containing numeric status and nullable rpc_code; an error envelope also supplies message and nullable data_json. data_json is the original RawValue text, preserving its numeric spelling as text rather than introducing floats into hashed JSON. sources is an ordered array. Response observations are separate from source ancestry. | Keep method, stage, operational kind and RpcRejection typed. Replace ResponseContext and omission construction in json_rpc.rs; do not fetch an extra response body or add IO. |
| Exposed native EVM source | Each sources entry retains message. reqwest entries additionally retain the existing transport_kind category from the current predicates; IO entries retain os_kind and nullable os_code; JSON entries retain category, line and column. Unknown sources retain their exposed message and child links. | Replace the current source/fact enums with these fields at capture.rs. Use native ErrorKind's Debug name for os_kind; do not maintain a mirrored OS enum or parse messages. The existing reqwest category precedence and parser category spellings remain. |
| CLI output failure | Keep typed stream and write/flush stage. Its diagnostic data retains the top IO message, os_kind/os_code and the ordered exposed child sources with message and IO fields when known. | Inline OutputIoError's data in the output error; preserve get_ref() as the initial custom IO source and source() for deeper links. Use the same scalar/array spelling; no generic reporting custody or second capture service. |

Compute the existing timeout/rate-limit owner kind from the concrete client error and checked
HTTP status before constructing response JSON; no classification decision reads that JSON.
The existing CLI error becomes OutputWriteError { stream: OutputStream, stage: WriteStage,
details: DiagnosticEvidence }. Its Serialize includes that data; remove the old source-marked
Box<OutputIoError>. DiagnosticEvidence supplies no std::error::Error source implementation.

Walk source() only at those owning boundaries and preserve each exposed layer in order. Keep a
small list of borrowed source addresses locally; if a reference repeats, stop before duplicating
it and add source_cycle: true to the diagnostic object. No native reference leaves extraction.
Absent that condition there is no cycle field. Do not add a source-count budget, truncation branch,
cycle registry or discarded-suffix traversal. This records what the selected source APIs expose;
it does not promise hidden foreign-library data or arbitrary custom source implementations.

from_value cannot fail: these helpers supply ordinary JSON scalar/array/object data, not callbacks
or arbitrary native serializers. A failed execution job, source formatter or later whole-error
encoding uses the existing internal failure route; partial evidence and invocation fallback
markers are never acknowledged as a complete operational original. Receiver forwarding does not
rerun extraction. Preserve the same cause fields through whole-owner admission, restoration,
classification and the actual invocation/report consumer.

This list is closed. Existing construction callers of the shared EVM helper only adopt its new
return/import path; they do not authorize a startup/config audit. Part 2 subtracts these completed
response/native-source cases before selecting further provider, signing or recording producers.

## 6. One authoritative current record

Object is the sole eraser for executable and persisted MFM values. It retains checked identity,
canonical bytes, matching hash and the object bound; it does not select implementations or schedule
transitions. Runtime associates typed implementations and checks each value's selected slot.

| Boundary | Representation |
| --- | --- |
| State preparation/evaluation/interpretation and typed helpers | Concrete input, context, output and owner error types. Do not accept Object where the helper already knows the concrete type. |
| Public adapter callback | Concrete intent/command, evidence and operational error types; borrow the checked instance ContentRef and EffectId where required. |
| Private connections between independently registered implementations | Object at the existing State/adapter and recovery/map callback seams. Decode at typed entry and encode only to cross the next heterogeneous or persistence boundary. |
| Complete continuation, admission and restoration | Object in heterogeneous value-bearing fields; checked IDs, positions, counters and operation variants everywhere else. |
| Journal/Store | Encoded Runtime record/frame bytes, without domain-value interpretation. |

Independent State and adapter registrations are connected by the Program's capability/binding
identities. Keep that specific private Object boundary; eliminating it must not introduce Any,
another value eraser or a registration redesign in this RFC. Function/future trait objects select
and run code without defining another value representation. InvocationDiagnostic contains final
report data only; it cannot supply executable values, native downcasts or classification.

Concrete values may remain local to one typed invocation. Section 8.6 retains a Read's decoded
input through preparation and interpretation. No native value is cached in Driver or maintained
beside the committed continuation across failure/classification or settlement/interpretation.

### 6.1 Remove the independent phase/facts pair

Runtime owns one complete current record. It contains the latest acknowledged operation together
with complete checkpoint inputs, recovery usage and Effect barrier. The operation determines the
permitted continuation using the admitted Program. Store and restore that same record type.
There is no separately stored Phase, cursor/current-input copy or mutable accumulator beside it.

This is a projection from one current record, not event replay: no predecessor is an input, no
historical counter is reconstructed, and no earlier operation frame is consulted. Public phase
names remain useful projections. Existing Runtime transition/dispatch logic selects the next work
under section 6.5; no new Continuation type is required. A private borrowed helper is
justified only if its callers would otherwise duplicate those rules. It must remove that duplication,
not wrap an equivalent existing selector. Add no serializable phase model, cached selection or
second authoritative state. Selecting work neither reruns recovery nor charges a grant.

### 6.2 Checked Object and ordinary record decoding

Retain the existing Object representation and checked getter contract:

```rust
struct Object {
    value_ref: Arc<ContentRef>,
    canonical: PlainCanonicalJsonBytes,
}
impl Object {
    fn from_value<T: MfmValue>(value: &T) -> Result<Self, ValueError>;
    fn from_canonical(value_ref: ContentRef, bytes: &[u8]) -> Result<Self, ValueError>;
    fn value_ref(&self) -> &ContentRef;
    fn contract_ref(&self) -> Result<ContentRef, ValueError>;
    fn canonical_bytes(&self) -> &[u8];
    fn admit(&self, descriptor: &SchemaDescriptor) -> Result<(), ValueError>;
}
```

Fields stay private and cloning shares immutable storage. Keep the current nested value_ref and
canonical wire fields. Implement ordinary Deserialize with one small local wire containing a
checked ContentRef and raw JSON payload, call Object::from_canonical, and convert its rejection
with serde::de::Error::custom. No Object is returned until identity, canonical bytes/hash and bounds
pass. Derive the enclosing record decoding and require complete input consumption. Reject unknown/
duplicate fields, unknown variants, wrong field types, invalid checked values and invalid current
relationships. Delete ObjectSeed and the parent/collection
seed grammar; do not replace them with a second payload family, error side channel or parser.

Writers emit the current object maps and external snake_case tags. Readers accept the ordinary
Serde structural forms of that same current Rust record, including sequence forms where Serde
supports them. This deliberately drops the old parent visitors' map-only restriction. Accepted
aliases still pass exact canonical frame/hash, Object, slot and current-state checks. They do not
introduce an older record version or bypass a checked constructor. Do not re-encode a loaded
record to compare its spelling, add a map-only visitor framework or rebuild the deleted grammar.

Object::from_value keeps existing typed Values admission. Object::from_canonical proves its own
invariants; admission against the selected descriptor remains necessary. Runtime checks the slot's
contract, and typed decoding enforces the concrete owner's constructor rules. Preserve those
distinct checks. Keep framework ContentRef, EffectId and position types checked; do not replace
them with raw strings/integers or add qualification wrappers, reference caches or checked_ref().

E4 explicitly distinguishes these error routes:

| Producing boundary | Required diagnostic and public treatment |
| --- | --- |
| Malformed persisted framework Object/ID/canonical/hash data rejected inside ordinary record Deserialize | Operation Restore, stage Decode, parser category, available location and rejection reason. Forward the parser diagnostic through the existing internal invocation route: code internal, REST 500, CLI exit 2. No append or execution callback. Independently structured nested constructor variants, ancestry, expected/actual fields and size facts are not promised. |
| A nested stored Object exceeds its 32 MiB bound during Deserialize | Reject it through the same restore/decode route with no structured SizeViolation and no 422 projection. The bound still applies; this is the explicit change from native nested-size propagation. |
| Direct typed Values construction or selected slot/schema admission after decoding | Preserve the selected concrete error fields, including field and expected/actual identity facts where supplied. Actual typed input/output/admission/encoding/append size failures keep structured size evidence and existing 422 treatment. |

The parser reason is diagnostic text, not a protocol for recovering erased fields. Do not parse
messages, stash a native error or add a generic constructor-capture mechanism. Normal diagnostic
construction-failure omissions in section 9 still apply. This limited stored-data contract is
intentional; State/domain/adapter originals and E5's selected native constructor retain their
separate promises.
Rejection, content identity, schema admission and object limits are unchanged. K1 exercises these
routes through Runtime/Application before the record representation is changed in K2.

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
| Values | Object construction owns checked reference/canonical/hash/object bounds; descriptor admission owns schema/descriptor checks. |
| Private Runtime Driver admission | Program association, current operation mode/slot contracts, checkpoint/usage/barrier relations and current Effect identity. |
| Typed operation entry | Native constructor invariants and actual capability evidence binding. |
| Runtime successor construction | Recovery authorization, counter charging, fresh visits and Effect restrictions. |
| Store | Physical snapshot/rows, atomic append, immutable prefix, cumulative count/bytes. |

Admission checks all present object occurrences against their selected contracts without repeating
Object constructor hashing or identity checks. It does not eagerly construct dormant native
checkpoint or terminal values. Objects selected for a typed operation use their owner's native
constructor. No semantic history scan, classifying a past error again, root-map replay or comparison
against an independently reconstructed Phase is allowed.

Locally valid forged counter resets or substituted checkpoint inputs remain outside historical
verification, as explicitly selected by the user. Removing duplicate phase agreement does not
remove actual contract, range, authority or current-record checks.

### 6.5 Record and dispatch sketch for the bounded proof

The field types below are the target ownership contract: retain the existing checked IDs and
Object. Derive ordinary Serialize/Deserialize for the record structures under section 6.2;
there is no raw framework record, parent seed grammar or separate qualified representation.

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
Inline DomainFailure/ReadFailure payloads here as the TerminalFailure copy disappears. Section 10.1
reuses Failure directly for public inspection; retain no separate owned failure wrapper.
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

For example, State 5 fails at visit 12 and recovery commits Restart to State 2. The latest
Recovered record selects State 2 with its checkpoint input at visit 13; it does not say State 2
has succeeded. After that State's success is committed, the latest Succeeded record selects State 3
with the output at visit 14. Loading either record derives the same next work without consulting
earlier frames, replaying the handler or charging the recorded Restart again.

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
objects, and passes the canonical payload to Journal for sealing. Submit that exact candidate to
Store and retain it on append failure under section 9.4. A preparation failure before submission
retains no unsent candidate in the invocation report. Do not introduce a generic commit facade.

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

Retain the implemented admission/latest/optional-probe Store interface and physical query protocol.
The future alias below is notation for its existing boxed, Send, lifetime-bound future, not a new
dependency or executor. Keep `append_run` and its Inserted/NotInserted/error dispositions.

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

The frame exposes checked identity/header fields and a borrowed canonical payload. Retain the
current mfm.run.frame.v6 domain and envelope fields: `domain`, `run_id`, `run_sequence`,
`previous_head_digest`, and `payload`. The digest uses exact canonical frame bytes under the existing
hashing rule. K2 changes the Runtime payload to RunRecord and rejects the obsolete payload shape;
it does not change the Journal envelope protocol or restore an old reader. Keep frame sequence
numbering, RunId spelling, digest algorithms and exact-head append checks.

The retained admission/latest query must not acquire history semantics through a count/sum query
or another load helper. Required rows and head metadata come from one repeatable-read
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

On a fresh resume, dispatch from the latest acknowledged record under section 6.5. Recovered
identifies the authorized work and explains the prior decision; it does not rerun the old handler
or charge its usage again. When failure recording is ambiguous, stop before policy. When decision
recording is ambiguous, stop before dispatch. When settlement recording is ambiguous, stop before
interpretation. Retain the candidate under section 7.2 without granting work from stale or uncertain
state; a Store error
does not start an automatic reconciliation loop.

The final recording entry point is:

```rust
async fn record(
    context: DriverContext<'_>,
    next: RunRecord,
    operation: Operation,
) -> Result<DriverDisposition>;
```

Keep Arc<RunRecord> ownership in Driver and across immediately awaited sealing work. When recording
fails, obtain the admitted Failure from next.operation's Failed or Recovered variant; other variants
have no original failure. Clone its Object handles only when an error needs ownership. Do not pass
an independent original, retain a native original or introduce Arc<Failure>/another custody type.
Derive continuation/yield behavior from the acknowledged operation under the table above; remove
yield_after. Known insertion of a recovery decision yields or returns RecoveryStopped. A matching
NotInserted observation still yields under section 7.2. Retain DriverDisposition's Continue/Yield/
Failed distinction: an assembled InvocationFailure may already contain a newer checked observation
than the outer RuntimeError route's caller holds.

K2 removes yield_after with the record/dispatch cutover. K3 removes the native-original argument
together with ReturnedFailure and the recording payload conversion; preserve the existing native
route until that coherent cutover rather than introducing an intermediate carrier.

### 8.6 Typed operation-entry checks

The checked callback-entry rules below complete the workflow in section 8; they are implemented
by existing monomorphized Runtime runners, not by a new validation registry:

| Entry | Typed checks and operation |
| --- | --- |
| Runnable Pure/Read | Decode the selected complete input. Pure evaluates it. Read retains that typed input locally, prepares/encodes intent, invokes the adapter, then decodes/binds the admitted intent/evidence and interprets using the retained input. |
| EffectPending | Decode State input, recompute deterministic S::prepare, and compare its canonical command/ref with the retained command. Use the EffectId already checked against RunId/Program/position/command by current-row validation. Only then enter the adapter with that retained identity/command. |
| Returned Effect settlement | The pending runner decodes command/evidence and calls C::bind_evidence before constructing the settlement commit. A rejected binding appends nothing and retains true post-IO stage information. |
| AwaitingInterpretation | Decode retained State input/command/evidence, call the same C::bind_evidence, then S::interpret. No provider call or command preparation is allowed in this phase. |
| AwaitingRecovery | Decode the selected original error and current policy/map parameters, classify, request, authorize, and map only when required. Do not rerun the completed State or adapter to reconstruct its original. |

The erased adapter encodes its returned evidence but does not repeat State-level command checks or
the runner's evidence binding. Reuse each capability's existing bind_evidence implementation.
Current-row validation owns the EffectId derivation check once per admitted candidate/restoration.

For Read, the immediately awaited preparation job returns (input, intent_object): the decoded
S::Input and encoded C::Intent. Move that same input into the interpretation job; do not decode
call.input a second time. Adapter failure or cancellation drops the local input. No extra Clone
bound, Driver field, cache or new stored type is required. The private adapter still receives an
Object and its public callback receives concrete values. Bind the admitted/decoded evidence that
interpretation actually consumes; moving binding before evidence encoding changes this guarantee.

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
This is final report data at the invocation boundary, not another executable-value eraser. Keep
compatible typed interfaces concrete; no native owner or value can be recovered from the report.

Delete NativeCause, Captured, `Owner<E>`, from_error/from_original/from_error_with, projectors,
arbitrary native downcasting, and companion owner Wire trees created solely to project children.
Do not replace them with `Box<dyn Error>`, Any custody, another dynamic trait, serializer registry or
one global enum of every owner's errors. A borrowed Error::source() walk inside an owner may help
capture exposed layers; no erased native owner or deferred extractor crosses this boundary.

InvocationDiagnostic is needed only to carry selected internal failure facts through interfaces
that cannot name every producer's concrete error. Its minimal Values-owned representation is one
owned DiagnosticEvidence, constructed once and forwarded immutably. It has no byte-budget policy:

```rust
#[derive(Debug, serde::Serialize)]
pub struct InvocationDiagnostic {
    code: &'static str,
    operation: &'static str,
    details: DiagnosticEvidence,
    size: Option<SizeViolation>,
}
impl InvocationDiagnostic {
    pub fn from_fields<T: serde::Serialize + ?Sized>(
        code: &'static str,
        operation: &'static str,
        fields: &T,
        size: Option<SizeViolation>,
    ) -> Self {
        use std::panic::{catch_unwind, AssertUnwindSafe};
        let details = match catch_unwind(AssertUnwindSafe(|| serde_json::to_value(fields))) {
            Ok(Ok(value)) => value,
            Ok(Err(_)) => serde_json::json!({"omitted": {"reason": "encoding_failed"}}),
            Err(_) => serde_json::json!({"omitted": {"reason": "panicked"}}),
        };
        Self { code, operation, details: DiagnosticEvidence::from_value(details), size }
    }
    pub fn code(&self) -> &'static str;
    pub fn operation(&self) -> &'static str;
    pub fn details(&self) -> &DiagnosticEvidence;
    pub fn size(&self) -> Option<SizeViolation>;
}
```

The getter bodies simply borrow/return their fields; details().as_value() borrows the same tree.
Fields stay private and there is no mutation API, native-owner access, MfmValue identity or custom
error trait. Move or borrow the value through
existing error variants; share it only where an actual owner needs sharing. The JSON value is final
report data, never executable input or a second representation beside RawValue/encoded JSON.
Implement ordinary Display/Error for existing typed error wrappers without an erased source;
the retained causal layers are accessible in diagnostic data. No source traversal during rendering
or recursive error implementation is required.
A local Serialize helper for selected fields is ordinary boundary code, not another error family.
Reuse an owner's existing serializable data when it already expresses exactly the selected field
contract; do not create a companion DTO merely to pass it to from_fields. This is not permission
to use an arbitrary native error's serializer as an unspecified diagnostic contract.
No consumer calls Serialize on an arbitrary original Rust error to discover its required details.

For E4, direct typed construction and postdecode slot/schema admission retain supplied concrete
fields, including expected/actual identities. Ordinary stored-record decoding retains the parser
category, available location and rejection reason under section 6.2, including its explicit
nested-size diagnostic/status exception. Do not reconstruct nested constructor fields from text.
E5 retains the constructor case, metadata.correlation location and,
for an excessive length, limit 256 and actual observed bytes. Owner codes and fields belong to
those selected contracts. An opaque code or message alone does not satisfy these cases. Existing
DiagnosticEvidence is the same Values-owned data in declared error fields and invocation details.
Section 5 owns its one persisted schema/profile; no client library or diagnostic crate dependency
enters Values. Serializing an owner can include its already captured data without native recapture.

Move the existing Serialize-only SizeResource and SizeViolation declarations together from Runtime
to Values and update their actual consumers. Reuse the same measured-versus-serialization-lower-
bound variants and public spellings. Do not add SizeDiagnostic or a second size hierarchy;
section 5.1 relocates the existing ObservedSize to EVM without changing its meaning. Populate size
when the concrete failure is known; delete cause_size and its cross-crate source traversal/downcasts.
Receivers read supplied facts directly.
Runtime adds its own actual execution operation/stage once; owner operation context remains intact.
No secondary reload/report failure can replace the primary disposition or size evidence.

from_fields is the only shared field-conversion helper: it consumes neither a native error owner
nor a serializer callback. The Serialize bound applies to selected report fields; reuse an owner's
existing serializable error only where that is the field contract fixed in section 9.6. Conversion
constructs one JSON tree directly; it does not encode text and parse it back. Final JSON encoding
belongs to the existing transport renderer. Owner codes/operations are fixed identifiers from the
table; no identifier-size type, budget arithmetic or metadata-overflow branch is required.

The two literal objects in the sketch are the complete invocation-only fallback wire. They contain no
secondary error or exception payload. Reserve the top-level omitted field, discard partial details,
and preserve code/operation/size unchanged. Do not retry a serializer or report its failure
recursively. Add no Omission enum, capture-failure type or fallback parser.

Delete the invocation diagnostic's 32 MiB ceiling, bounded writer, metadata reservations,
bound_reached/truncation fields and their tests. There is no new reporting quota. Keep the actual
Object/frame/run limits and the content-addressed FailureReport bound. Section 5 likewise deletes
the independent persisted-diagnostic quota and accounting. Optional SizeViolation describes a
primary execution or Store limit, not a limit on this diagnostic. K3 verifies selected fields,
the two construction-failure markers
and immutable forwarding through the real invocation consumer.

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
fixed invocation marker replaces detail if constructing it fails or panics; primary code,
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
it. E4 uses checked Object Deserialize and direct typed/slot admission as distinguished in section
6.2. No extra error schema/identity is needed.
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

Keep RuntimeError::Recording for a failure to record an admitted original or a failed append of
an exact submitted candidate. Return it through the existing InvocationFailure::Execution route.
These invocation-only payloads preserve independent original/recording facts and physical append
authority; they introduce no persisted error representation or value eraser:

```rust
enum RecordingFailure {
    BeforeAppend {
        original: Failure,
        cause: InvocationDiagnostic,
    },
    Store {
        original: Option<Failure>,
        candidate: EncodedRunFrame,
        cause: StoreError,
    },
    NotInserted {
        original: Option<Failure>,
        candidate: EncodedRunFrame,
        observation: Option<(RunSummary, CandidatePresence)>,
        reload_cause: Option<InvocationDiagnostic>,
    },
}
enum CandidatePresence { Present, Excluded, Absent }
```

BeforeAppend requires an already admitted Failure plus the independent preparation cause, such
as frame encoding or actual metadata-limit rejection. With no admitted original, return the ordinary
internal diagnostic; failed initial declared-error encoding follows section 9.2. Never retain unsent
frame bytes or their identity in this error. Sealing returns its ordinary error; the owning record
call adds the original when present. Delete the seal-error wrapper and its outer unpack/repack path.

Store and NotInserted keep one exact submitted candidate. Their original is optional because a
successful outcome or admission can also fail to record; None invents no operational failure.
Share admitted immutable data where needed; no extra native original survives admission. Public
output exposes submitted candidate identity/sequence/digest, not its body. K3 proves actual sharing
without introducing another error/original/candidate wrapper. Delete AppendFailure and match the
two parent variants directly; Store errors cannot carry probe fields.

NotInserted is a physical disposition and need not invent a cause. Only it probes automatically
under section 7. Store errors, including Indeterminate, return immediately with the actual
disposition. CandidatePresence records the checked exact-sequence finding: Present means identical
bytes, Excluded means different immutable bytes, and Absent means no row in a snapshot whose head
is below that sequence. An unestablished finding is observation: None, never Absent; a missing row
at/below the head is corruption. RunSummary ties a finding to the observed physical head, not an
executable view.

Keep observation and reload_cause independent. A checked finding can survive a later failure to
decode/project the latest state, including Present with a decode failure. If observation is None,
reload_cause must be present. Present with a valid latest view returns that view and yields without
a RecordingFailure; Excluded/Absent may have no secondary cause. Construct these combinations in
the existing probe branches; add no result hierarchy. Do not recursively audit a failed append,
add a second durable sink or give an uncertain candidate execution authority.

Runtime returns the facts from the operations it performed; the invoker presents them. For example,
if append returns NotInserted and the subsequent load fails, return the same NotInserted variant
with observation: None and reload_cause: Some(diagnostic). The client can report both that this
attempt inserted nothing and that checking the stored candidate failed. This does not establish
candidate absence. An admitted Read timeout plus an append error instead uses Store, preserving
the original and the separate StoreError. Neither case needs another reporting layer.

App and both transports render the same supplied diagnostic/admitted data. JSON and text renderers
borrow the typed report; delete CLI Fields/MissingField and serialize-then-reparse text rendering.
They do not reload the run or reconstruct causes to produce this report. After Inserted, a failure
to construct the public view uses the existing RuntimeError::Projection route with its
acknowledged head and an InvocationDiagnostic; it cannot turn that insertion into a recording
failure. Here projection means constructing the public view, not another error-capture mechanism.
Public encoding and writing can still fail: the existing transport owner retains the available
invocation/head and actual terminal encoding/IO cause, then ends there. Keep write-versus-flush and
stdout-versus-stderr facts. Known insertion stays acknowledged and cannot reopen work. No socket
delivery claim follows from REST handoff. Delete `ReportFailure<T>`, ReportStage, IncompleteReport,
ReportDetail and their duplicate omissions/head views; add no generic successful-result custody,
secondary projection attempt or recursive output-error report.

### 9.5 Minimal concrete Values errors

[ValueError](crates/kernel/values/src/lib.rs) remains the Values crate's concrete error, not an
eraser or a diagnostic container.
Object and schema construction need to distinguish these existing failures before crossing the
invocation boundary. Keep its eight current variants and source/From annotations:

```rust
// Add Serialize + snake_case to the existing definition; retain its Error attributes.
enum ValueError {
    Descriptor(String),
    Identity(mfm_ids::IdentityError),
    CheckedIdentity(mfm_ids::CheckedStringError),
    Canonical(mfm_canonical::CanonicalError),
    InvalidSchemaIdentity,
    SchemaShapeMismatch,
    SizeLimit(SizeLimitExceeded),
    ArtifactTypeMismatch { field: &'static str, expected: String, actual: String },
}
```

| Existing variant | Necessary producer/responsibility |
| --- | --- |
| Descriptor | Schema/descriptor construction in values/lib.rs and persisted.rs; retain its concrete reason string. |
| Identity | Checked ContentRef/schema identity construction in Values, Program and program-derive; retain IdentityError. |
| CheckedIdentity | StableId construction for derived context slots and transaction recipes; retain CheckedStringError. |
| Canonical | Object and canonicalize_mfm_value rejection; retain the concrete canonical/encoding cause. |
| InvalidSchemaIdentity | Strict schema-identity decoding and identity validation; keep the existing rejection category. |
| SchemaShapeMismatch | Values' closed-shape/persisted validation; keep its existing category without a new field-error framework. |
| SizeLimit | Actual Object-size rejection, with measured bytes/limit. |
| ArtifactTypeMismatch | Object schema/hash mismatch, with field and expected/actual identity. |

Derive Serialize with external snake_case tags directly on ValueError. Delete its mirror Wire enum
and manual projection in values/native.rs. Descriptor serializes its existing message, replacing
the bespoke withheld/message_bytes/unavailable payload. Keep typed sources in their own variants.
Do not merge them into strings, add MfmValue/Error bounds to State failures, or create another
error family to reduce this enum's variant count. Do not expand lossy descriptor/shape constructors
outside the selected paths; these existing categories are not a new constructor-migration backlog.

One reusable Values adapter is justified by Object::decode, the MfmValue default and Runtime's
existing ValueError conversions. It owns direct size extraction and the same field contract:

```rust
impl ValueError {
    pub fn into_diagnostic(self, operation: &'static str) -> InvocationDiagnostic {
        let size = match &self {
            Self::SizeLimit(size) => Some(SizeViolation::Measured {
                resource: SizeResource::CanonicalObject,
                actual: size.actual(),
                limit: size.limit(),
            }),
            Self::Canonical(source) => source.serialization_bound().map(|(limit, observed)| {
                SizeViolation::SerializationBound {
                    resource: SizeResource::CanonicalObject,
                    limit: limit as u64,
                    observed_at_least: observed as u64,
                }
            }),
            _ => None,
        };
        InvocationDiagnostic::from_fields("value_error", operation, &self, size)
    }
}
```

Keep the current public Values Result alias and constructor signatures. No ValueError registry,
generic into_diagnostic trait, downcasting, or recursive size discovery is required. The operation
argument names the actual existing boundary function adapting this failure; Runtime separately
retains its execution Operation/Stage. Object::decode<T> still returns the concrete T or the
InvocationDiagnostic produced here/by T::decode_native, without another decode-error wrapper.

Child errors need one concrete representation and serializer. JsonError remains a small concrete
adapter for serde_json::Error, which has no Serialize implementation: keep new(source), its typed
source, and one serializer for category, line, column and the available message. Remove its blanket
withheld-message projection. Reuse it for the K1 restore/decode reason and the MfmValue default;
do not create JsonErrorFields, native custody or a second JSON capture service.

Consolidate CanonicalError's current struct + CanonicalSource + serializer Cause into one concrete
owner enum:

```rust
enum CanonicalError {
    Grammar { message: String },
    Json(JsonError),
    Utf8(std::str::Utf8Error),
    SerializationLimit { limit: usize, observed_at_least: usize, source: JsonError },
}
```

Retain typed sources and existing constructor, message() and serialization_bound() signatures.
message() borrows Grammar's string and returns the current static message for each other variant;
store no duplicate message. Derive snake_case Serialize; only Utf8 needs a
small serialize_with function for valid_up_to/error_len because the foreign type lacks Serialize.
Delete CanonicalSource and the companion Cause enum. Do not redesign canonical parsing, bounds,
descriptors or persisted value schemas as part of this consolidation.

### 9.6 Closed adaptation and callback recipes

The following rows replace K3's open-ended owner discovery. They cover existing supplied data,
not upstream enrichment. Preserve each named owner's current variants/fields except section 5's shared-data cutover and
the explicit ValueError/JSON/canonical simplifications above. Other concrete error APIs remain concrete.
In each row, operation is the name of the existing function doing the adaptation; this names a
known boundary rather than inferring an operation from error text. Fixed diagnostic codes are
listed below; they do not replace Runtime Operation/Stage or App's status/disposition code.

| Producer and existing conversion sites | Recipe and retained facts | Consuming evidence/stopping point |
| --- | --- | --- |
| ValueError in values/lib.rs, object.rs; Runtime Values conversions; existing Live assembly binding_ref conversions | value_error through into_diagnostic; exactly section 9.5's variant fields and direct size. MfmValue/derive constructors continue returning ValueError until this boundary. | E4 direct identity/schema/size, E3 encoding failure. Assembly callers only forward this existing contract; no startup/config or constructor enrichment. |
| JSON decode in the MfmValue default, Runtime restore, E5's local parser and App response encoding | json_error; from_fields over the existing JsonError adapter: category/line/column/message, size None. K1 initially uses its existing NativeCause route; K3 substitutes these data directly. | E4 restore/decode reason and internal/500, E5 parser failure, terminal encoding failure. No recovered nested constructor fields or new parser family. |
| Direct CanonicalError from Runtime record/frame encoding and selected value conversion | canonical_error; serialize the four concrete alternatives from section 9.5. Read serialization_bound directly with the actual resource: Values uses CanonicalObject; Runtime frame encoding uses Frame. | E3 encoding and retained live size/lower-bound behavior; no erased source walk or descriptor migration. |
| Runtime StateInvariant, InitialValueMismatch and task failure conversions in engine/state/assembly/recovery | runtime_invariant with existing concrete identity/relation fields; reuse its identity mismatch variant for InitialValueMismatch. task_failure with details equal to the scalar panicked or cancelled. Known size remains a direct Runtime size variant. | E1/E4 actual operation/stage and no fault append; keep local mismatch and cancellation behavior. Delete Runtime TaskFailure; add no enum or Diagnostics dependency to represent these two strings. |
| Existing EVM/Portfolio internal State and capability binding conversions | state_internal over the current serializable EvmDomainError/CapabilityError or existing owner payload at that call. Preserve its fields and nesting; size None unless it is the Values row. | E1 same internal routing and supplied facts. Declared Self::Failure/C::OperationalError stay concrete and follow Object admission, never this diagnostic route. |
| E5 metadata.correlation native hook in evm/balance_decode.rs | constructor_error; retain metadata.correlation location and the existing empty/excessive-length case with limit 256/observed_bytes. Reuse the current BalanceMetadataError data; no mirrored error tree. | E5's two Runtime/App assertions after shape admission; no additional constructor family. |
| Live's existing invariant helper and AdapterFailure/AuthorityError::Internal conversions | adapter_invariant over the existing serializable owner: binding/identity expected/observed, task outcome and supplied typed codec/signing source fields. Use panicked/cancelled strings for task outcome; section 5 removes the redundant HTTP-status reconstruction failure. The helper needs only Serialize, no Error/custody bound. | E1 actual mismatch/preflight/response stage. Apply section 5.3's finite source-data cutover; additional signing, authority or provider enrichment belongs to Part 2. |
| Store load/append and Journal typed ports | Keep StoreError/JournalError and existing variants. Store errors keep their actual disposition and direct size mapping; adapt Journal fields only where a heterogeneous boundary requires it, with journal_error. | E3 append/probe/ambiguity and existing load failures. No Store/provider error schema rewrite in Part 1. |
| CLI write/flush and REST/App terminal report owners | output_io over the output owner's stream/stage and section 5.3's shared message/OS/source data. Inline OutputIoError's data in OutputWriteError; JSON encoding uses the JSON row. | E6 terminal write/flush/stream facts and known insertion. Keep lexical ownership until return/handoff; replace the old capture bookkeeping at this existing boundary and delete generic reporting custody. |

Use from_fields directly for one-off typed conversions. Keep a local helper only when multiple
existing callers share that exact recipe, such as Live's invariant helper. Do not add a trait,
public code enum, codec registration or serializer mirror per row. Existing small child serializers
for foreign fields remain at their owning boundary; a receiver never reopens JSON to discover facts.
Retain the current consuming tests for these facts and add only missing boundary regressions.

Pass failed-original context through the existing private adapter connection as ordinary arguments:

```rust
type ErasedReadAdapterCallback = dyn for<'a> Fn(
    ExecutionPosition, &'a ContentRef, &'a Object,
) -> BoxFuture<'a, Result<std::result::Result<Object, Object>>> + Send + Sync;
type ErasedEffectAdapterCallback = dyn for<'a> Fn(
    ExecutionPosition, &'a ContentRef, &'a EffectId, &'a Object,
) -> BoxFuture<'a, Result<std::result::Result<EffectAdapterOutcome<Object>, Object>>> + Send + Sync;
```

The outer Result is Runtime's internal-error Result. The inner Ok is observed evidence/Pending;
inner Err is the admitted concrete operational failure Object. Delete AdapterReturn and
ReturnedFailure; an internal error never becomes the inner Err. The ContentRef argument is the
already associated operational-failure contract, supplied by the runner together with position.
The closure decodes the intent/command, calls the unchanged typed public adapter, then encodes
evidence or calls section 9.2's encode_failure(error, operation, position, failure_contract).
No context wrapper, new registry state or recomputation from the failed value is needed. For a
State failure, the runner supplies its existing State failure contract to that same encoder.

## 10. Closed Part 1 error scope

Part 1 proves the shared mechanism through the cases below, including section 5.3's finite
EVM/CLI source-data replacement. Part 2 enriches only its remaining selected execution producers. Startup/config/deployment/ingress and broad
constructor audits are outside both RFCs. E5 remains one explicitly selected execution-time decode
case; it does not authorize a general constructor migration. E4 keeps checked Object and ordinary
record decoding with the explicit diagnostic boundary in section 6.2; it does not certify an entire
native parser/source object as secret-free.

| ID | Required core boundary and evidence | Stopping boundary |
| --- | --- | --- |
| E1 | Pure/Read/Effect preparation, execution/interpretation, handler, map and heterogeneous Runtime conversions forward their required InvocationDiagnostic data. Internal failure appends no fault and never enters classification; explicit resume follows acknowledged state. | Section 9.3 callbacks and their invocation consumer. No erased native owner, arbitrary Deserialize provenance, or new upstream error family. |
| E2 | A State's concrete Self::Failure and the shipping Read's concrete operational error retain the complete admitted original through failure-before-recovery and cold inspection. Verify section 5's nested DiagnosticEvidence, response/native-source fields and trusted-text admission through the whole owner and derived report. Restore that concrete type for classification; handler inputs remain Classification/RecoveryContext. Pending Effect preserves command/EffectId and settlement precedes interpretation. | Section 5.3's listed EVM response/source cases are part of the shared-data deletion. Additional provider/authority/signing producers belong to Part 2. No per-State wrapper, diagnostic classifier or standalone diagnostic value registration. |
| E3 | An admitted failed Read plus failed append reports the complete original and separate recording failure with actual acknowledgement. Cover pre-append frame preparation failure with/without an admitted original under section 9.4. A non-Error declared failure whose first encoding fails uses section 9.2's ordinary invocation diagnostic and fixed-marker contract. Also cover success plus append failure, dispositions and terminal output failure. | Existing invocation/reporting route, admitted original when present and exact candidate only after submission. No AppendFailure wrapper, unsent-candidate custody, special first-encoding Runtime variant, serializer retry, native-original custody, producer fallback payload, native-success bag, reporting tree or second audit sink. |
| E4 | Malformed nested stored references, bad hashes and oversized Objects reject through checked Deserialize: Restore/Decode, parser category, available location/reason, internal/500 and CLI 2. Postdecode slot/schema mismatch and direct typed construction retain supplied concrete fields. Live size errors keep structured size/422; nested stored-object oversize has no structured SizeViolation. Assert no execution callback or append on rejected restoration. | Section 6.2's stored-data exception is closed. No native nested-constructor ancestry/field recovery, parent seeds, raw framework IDs, parser side channel, message parsing, broad ID rewrite or source-certification project. E5's selected owner hook is separate. |
| E5 | Shipping EvmBalanceContext metadata.correlation constructor: empty and excessive-length failures retain their case/location and required limit/observed length through the actual typed Runtime callback and Application diagnostic after structural/schema admission. | These two cases only; reuse/extract the existing constructor. Keep minimal local raw-to-checked helpers only where necessary; no other context/value family migration. |
| E6 | Consumers of checked Object Deserialize, shared DiagnosticEvidence/InvocationDiagnostic, size data, Store and callback APIs compile and forward selected facts/classification. Delete mfm-diagnostics and update actual dependency/task/docs references. Read/Effect callbacks retain borrowed checked identity arguments; Read reuses its typed input within one invocation. | Section 5's named schema/source-data cutover is authorized. A needed forwarding/import update adds no producer audit, second value eraser, capture service or registration redesign. |

Sections 5 and 9.1-9.6 fix the shared data, admission, field recipes, callback signatures and
deletions. Apply those recipes to the named current conversions and record E1-E6 results in section
11.3's completion evidence. Producer facts or capture/schema mechanisms beyond those named remain
outside this handoff and need a concrete architect design/cost decision before expansion.

If a promised cause has already been erased, that case is unmet until the producing conversion is
resolved; it cannot pass by relabeling the loss unavailable. Preserve the selected supplied facts
in changed callers, with section 6.2's explicit stored-data exception. Apply section 4's explicit
MFM-input responsibility without turning upstream text into a
new safety audit. Record unselected execution gaps for Part 2 refinement; unrelated platform gaps
stay in the ordinary inventory and are not automatically part of either delivery.

### 10.1 Current public projection

Keep Runtime `start`, `resume(&RunId)`, and `read(&RunId)` and the existing App/CLI/REST request
surfaces. Keep RunView as an owned immutable public projection. It owns its public result fields
and shares Object storage; retain no Arc<RunRecord>, checkpoint collection, usage vector or executable
registration merely to render it. Its return type gains no borrowing lifetime or cached record view.

Use these payloads in the existing public state variants:

- EffectPending owns EffectCall and optional (original Object, RecoveryOutcome) from the latest
  pending-Effect recovery decision. Delete PendingFailureView and owned AdapterIncidentView.
- AwaitingRecovery owns Failure; AwaitingInterpretation owns Settlement; Succeeded owns Object.
- Failed owns FailureReport with the representation below. Delete owned FailureCauseView.

```rust
struct FailureReport {
    failure: Failure,
    reason: StopReason,
    usage: RecoveryUsage,
    root: Option<Object>,
    value_ref: ContentRef,
    canonical: PlainCanonicalJsonBytes,
}
```

Its private constructor takes (failure, reason, usage, root), validates the domain/Read terminal
relation and constructs the bounded canonical report. A domain failure requires root; a Read
operational failure forbids it; a pending Effect never becomes a terminal FailureReport. Derive
position() from failure.call().position(); expose borrowing failure()/root() accessors and retain
reason()/usage()/value_ref()/canonical_bytes(). Keep the existing public report fields and use
borrowing serialization to select them from Failure; storing Failure does not add all its fields
to the report wire. The canonical artifact serves report hashing/output, not a second owned cause
tree or a source to reparse for typed access.

InvocationFailure::RecoveryStopped retains only observed: RunView. Its incident and reason come
from that observation's pending failure/Stop outcome. Remove the duplicate incident/reason fields;
Runtime constructs this result only after observing the committed pending-Effect Stop.

App's Incident/Original serializers already borrow data and represent distinct public wire shapes.
Adapt them to borrow Failure or the pending Effect fields directly after deleting the owned incident
tree. Keep a small borrowing serializer where those field/tag differences require one; reuse the
Object renderer. Do not normalize public formats or introduce another owned reporting model to
delete a serializer name.

Retain the existing public phase tags and required observations:

| Public state tag | Required projection |
| --- | --- |
| `runnable` | Position and reason derived from the current commit: admission/success means Advance; Recovered Retry/Restart gives that reason. |
| `effect_pending` | Position, EffectId, and latest failure/decision from current Recovered facts when present. Replace `state_context` with actual input/command facts; do not retain a contextualization API. |
| `awaiting_recovery` | Position, original failure and its Pure/Read/Effect operation inputs, intent/command and accepted evidence as applicable; no classification/decision is fabricated before it commits. |
| `awaiting_interpretation` | Position, EffectId, and accepted settlement command/evidence. This is an incomplete state with no recorded internal fault. |
| `succeeded` / `failed` | Existing complete success value or derived terminal failure report, using the current operation, original Object and recovery outcome. |

Successful observation of either awaiting phase follows existing nonterminal policy: REST
returns 200 and CLI exits 1. An invocation that failed internally while leaving that phase returns
the existing execution-stopped error surface, CLI exit 2, and the reviewed REST error status.
Pending recovery Stop still returns RecoveryStopped/503. Live size failure remains 422; ordinary
durable Failed observation remains 200. Section 6.2's nested stored-object oversize is a restore/
decode failure, internal/500, with CLI exit 2. A new phase needs no new endpoint or HTTP policy.

Keep the existing error-category/status projection with section 6.2's explicit stored-data exception.
Retain the supplied diagnostic data. In particular,
replace Application's lossy AppendIndeterminate payload with
`AppendIndeterminate { recovery: RunRecovery, invocation: InvocationFailure }`; derive last_observed
from the retained invocation instead of storing it twice. Its code remains run_append_indeterminate
and REST status 503. from_invocation recognizes the primary Store disposition even when nested
inside RecordingFailure::Store, then moves the whole invocation into that variant. Other recording
Store errors project their existing categories: unavailable 503, actual size/arithmetic 422, corrupt
physical state 500. Internal constructor/task/encoding failures are 500 unless an actual size
violation supplies the existing 422 category; a nested stored-object bound rejected by Deserialize
does not supply that category. Never replace a primary append disposition with the secondary
reload/projection error's code.

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

Keep the baseline derived FailureReport's 32 MiB ceiling and content-addressed report contract;
section 5.1 supplies its current diagnostic-text schema profile.
Validate that concrete terminal report before its terminal append; if it cannot fit, leave the
original AwaitingRecovery and report the construction/size cause. Do not add a persisted report
copy or another lifecycle reservation. This is an actual report bound in addition to frame bounds;
include it in the shipping-size proof. Reports for the incomplete phases derive from their
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
or missing promised original information outside sections 6.2/9.2's explicit exceptions fails
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

Execute the bounded K1-K4 work below on the current branch under section 3. Keep already
implemented behaviors and their coverage. K1-K4 must pass before G1/F1 acceptance; this RFC does
not authorize Part 2 producer migrations during core work.

### K1: `remove decoder seeds and keep checked objects`

Implement section 6.2's ordinary checked Object Deserialize and derived parent decoding on the
current continuation. Keep its representation, borrowed value_ref getter, instance identities and
Read/Effect callback signatures. Delete ObjectSeed and the enclosing parent/collection seed grammar
as one cutover, before changing the record shape. Preserve complete-consumption, unknown/duplicate
field, unknown-variant, checked-value, canonical/hash/schema and bound rejection. Apply section
6.2's ordinary Serde structural acceptance; no map-only or full re-encoding validator. Use the existing invocation route here;
K3 later replaces its native custody, without a new intermediate error mechanism.

Exercise E4 through real Runtime/Application loading: malformed nested reference, bad hash and
oversized nested Object report the selected restore/decode diagnostic and internal/500 status,
with no provider call or append. Separately exercise a postdecode slot/schema mismatch and direct
typed size rejection with their retained structured fields/status. Keep E5's selected constructor
coverage. Use JsonError's category/location/message serializer specified in section 9.5 for the
existing parser route; the later custody deletion does not require a temporary error type here.
Update the owning design, architecture, error-audit policy and transport contracts in
this cutover so their general causal-preservation wording cannot reopen the stored-data exception.
Measure the actual grammar deletion; no raw carrier or constructor-capture experiment precedes it.

### K2: `remove duplicate continuation facts`

Replace Phase + OperationFacts with the section 6.5 record in the real Runtime path. Delete the
stored TerminalFailure copy, phase-reconstruction/agreement code and repeated payload accounting
at the same cutover. Inline DomainFailure/ReadFailure into Failure and apply section 10.1's owned
public payloads, deleting the owned incident/cause copies. Remove yield_after under section 8.5.
Keep required public wire distinctions through borrowing serializers. Verify typed failure/root
inspection and the unchanged RecoveryStopped wire/status from its single observed view. Preserve
authorization, checkpoint/usage/Effect checks and observable public phase behavior. Reuse consuming
lifecycle tests, deleting assertions for the superseded duplicated
wire. Use K1's derived decoding; there is no seed grammar to rewrite around the new record.
Measure actual removals and encoded occurrences before starting a new error owner.

### K3: `replace native custody with boundary diagnostic data`

Prove section 9's concrete data shape with E4's already established direct-admission fields and
stored-data parser diagnostic, E5 constructor fields, an already supplied foreign cause from these
selected paths, and measured/lower-bound live size evidence. Preserve K1's nested stored-object
size exception; do not restore native capture to recover fields intentionally outside that contract.
Implement section 5's shared Values data/profile and the two existing source-data conversions,
then section 9.1's from_fields sketch and section 9.5's concrete error simplifications. Apply the
closed recipes/signatures in section 9.6. Keep shared data/profile, affected owner/report schemas,
consumer migration and mfm-diagnostics deletion in one coherent cutover. Do not keep a compatibility
crate or legacy diagnostics reader underneath the new type. Verify that the two literal invocation
encoding/panic markers retain primary code/operation/size without recursively constructing an error. Delete the diagnostic
budgets, metadata accounting, bounded diagnostic writers and bound_reached behavior. No new quota
replaces them. Remove the 8 KiB policy, source/fact/omission hierarchy, EvidenceError and redundant
HTTP-status conversion; relocate only EVM's real ObservedSize and use task outcome strings.

Prove nested-field admission through a real ProviderFailure and its enclosing operational error,
Object restoration and the derived FailureReport. Include exposed nested messages/native fields,
HTTP/RPC facts, RPC data_json with its original numeric spelling, trusted text containing a marker,
source text larger than the deleted 8 KiB quota, and unchanged typed classification/handler inputs.
A same-marker ordinary Program/context string
still follows its existing rejection rule. Floats in diagnostic JSON or an oversized whole error
reject initial Object admission without truncation, append or classification; section 9.2 reports
that encoding/admission failure. Separately test terminal-report overflow under section 10.1:
the original is already admitted and committed, and classification/handler execution may have
occurred. Keep that original AwaitingRecovery and report the projection/size failure without a
terminal append, unavailable-original claim or replay of those callbacks. Retain cause order,
repeated-reference termination and the CLI custom IO source/write/flush checks. Delete quota/
vocabulary/omission fixtures and standalone diagnostic
MfmValue tests rather than carrying their obsolete assertions into the new route.

Cut over the existing callback/decoder/Runtime/App/transport interfaces coherently to immutable
InvocationDiagnostic data. Move the same SizeResource/SizeViolation declarations into Values;
delete cause_size downcasts and keep status/resource spellings. Preserve concrete errors inside
compatible library interfaces, and keep Self::Failure/C::OperationalError as the classifiable
originals. The current selected facts define the work, not every source reachable from an error.

Apply section 9.4's three RecordingFailure variants, deleting AppendFailure, BeforeAppend.candidate
and seal-error unpacking/repacking. Remove record's native-original argument under section 8.5;
derive its admitted Failure from the proposed record, sharing Object bytes. Update docs/design.md
and renderer fixtures from sealed-candidate to submitted-candidate reporting; no unsent frame
identity remains in the invocation contract.
Exercise E3: pre-append frame preparation failure with/without an admitted failed Read, immediate
failed/ambiguous append, non-Error declared original whose first serializer fails, success plus
append failure, NotInserted observation and terminal output failure. Assert the ordinary encoding
diagnostic retains its actual cause/size and section 9.2's known context and unavailable original
detail. It triggers no append, serialization retry or
precommit classification. Otherwise preserve the complete admitted original, independent recording
causes and accurate acknowledgement. No outside-job `Arc<E>` or native-success custody is required.

Delete NativeCause/Captured/Owner, projectors and companion error Wire trees, ReturnedFailure,
ReportFailure/IncompleteReport and related reporting types, CLI Fields/MissingField reconstruction,
and superseded custody tests in the same closure. Retain tests of promised cause fields, bounds,
classification and delivery. Update the affected dependency manifests/lockfile, workspace/task
references and owning documentation as part of removing the crate; do not redesign the build graph.
Section 14 distinguishes real necessary data from removable wrappers.

Use the agreed upstream trust boundary and specified diagnostic/recording/context signatures.
Measure the whole replacement, including owner-local adapters and tests. Do not migrate PostgreSQL,
config or signing families or expand beyond section 5's specified fields and persisted profile.
A further fact contract needs a specific design before implementation.

### K4: `retain typed input through read interpretation`

Apply section 8.6 in the existing Read runner: return the decoded input alongside the encoded
intent from preparation and move it into interpretation. Remove the second S::Input decode; keep
the private Object adapter boundary and binding of the admitted/decoded intent and evidence.
Retain success, operational/internal failure, evidence mismatch and cancellation coverage through
the actual runner. No Clone requirement, Driver cache, new carrier or registration redesign is
needed. Keep the established durable Effect/failure entry paths and their restoration behavior.

K1-K4 are logically ordered work items. Keep an inseparable API/cutover together; do not commit a
broken workspace or preserve two production designs merely to split commits. Retained behavior
must pass the relevant focused checks from [build and verification](docs/build-and-verification.md).
No broad owner migrations, new client extractors or arbitrary constructor families belong here.

### G1: architect acceptance of the corrected core

Apply section 11.3 to the exact K1-K4 candidate. The dedicated architect gives a concrete accepted
or rejected disposition with the outstanding failures/removals; a request for more evidence is not
acceptance. This review decides Part 1, independently of Part 2 readiness or cost. If the corrected
core cannot be justified, return the specific unresolved design to the user. Do not spend another
work period on unrelated small cleanups and then resume fanout.

### F1: complete and accept Part 1

Reconcile C1-C18 with K1-K4, E1-E6 and G1. Update authoritative design/architecture, code-quality
policy, AGENTS ownership/trust and error-preservation wording, and affected transport contracts in
their owning cutovers. State the typed-owner classification rule and accepted unavailable-original
encoding and stored-data decoding exceptions; do not leave universal native-custody or nested
constructor-field requirements in an authoritative guide.
Remove superseded implementations, tests, fixtures and docs while retaining observable coverage.
Part 1 ends here; no owner row from
Part 2 is required or automatically authorized by G1.

Use the narrowest relevant checks under [build and verification](docs/build-and-verification.md),
managed PostgreSQL/SQLx checks for changed persistence queries and managed Effect/client scenarios
for changed execution boundaries. All direct Rust tooling runs in the default Nix shell. Run one
final `nix run .#ci` on the complete Part 1 candidate. If verification changes that candidate,
repeat affected checks and obtain review of changed contracts before recording acceptance.

Record the accepted implementation commit, finalized Object/RunRecord/DiagnosticEvidence/InvocationDiagnostic/Store signatures
and source locations, C1-C18 evidence, real removals/additions and cost against both baselines, and
known information limits/deferred upstream gaps. Use one concise completion record in this RFC, not a new
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
E4 explicitly permits the narrower malformed-storage diagnostic/status contract in section 6.2.
These criteria do not require enriching upstream unit/flattened errors outside that scope. Record
existing gaps in the relevant inventory; only selected execution gaps belong to Part 2. Do not
discard a promised fact or fabricate a placeholder to pass a case. Persistence, authorization and
execution guarantees remain complete for the supported core behavior.

| ID | Observable acceptance |
| --- | --- |
| C1 | Fresh and restored runs use the same RunRecord and checked Object types and produce equivalent continuation/output through a real Runtime entry point. Every current enum variant round-trips through canonical JSON with inline objects. Object stays confined to section 6's boundaries; checked IDs/getters remain intact. No qualified cache, alternate wire state, object resolver or whole-record decode after encoding merely to revalidate it. |
| C2 | Admission/latest loading uses one consistent snapshot. Increasing history length does not increase normal rows loaded. A missing required row, inconsistent head, bad frame/object hash, wrong Program identity, or invalid current phase/slot contract rejects before action. Repeating start with identical Program/input returns the checked observation without driving; different admission returns AdmissionConflict. |
| C3 | Current-record admission rejects invalid operation/mode/slot and request/outcome relations, checkpoint/barrier relationships, out-of-range counters and checked-sum overflow. There is no independent Phase or agreement validator. The run total has no independently stored counter. No historical successor construction or claim of detecting otherwise locally valid historical counter/checkpoint substitution remains. |
| C4 | Success carries complete context to the next State. Restart selects an eligible declaration checkpoint's stored input, creates a fresh visit, retains current usage/Effect restrictions, and never consults old operation frames. |
| C5 | E2's concrete State-domain and adapter operational errors commit original/input/request/evidence as applicable before classification, handler or mapping. The same declared owner error is decoded for live/cold classification, with its causal/operation context retained. No invocation diagnostic, cached classification or mapped root replaces it. |
| C6 | Handler/map failure returns its selected invocation diagnostic without a fault append; the original remains AwaitingRecovery. Explicit resume reruns unfinished recovery evaluation. An authorized Retry/Restart charges the failing declaration and run once, then yields. Stop/denial spends no grant. Pending Effect Retry preserves visit/command identity. |
| C7 | Effect preparation commits command/EffectId before adapter entry. Pending/error/Stop behavior preserves that same unresolved command and existing reconciliation restrictions; no replacement authority or capacity-only retry quota. |
| C8 | Accepted Effect settlement commits before interpretation. Injected interpreter failure appends no fault; cold resume interprets retained evidence without entering the adapter for that phase. Uncommitted settlement makes no durability claim. |
| C9 | E1/E4/E5 internal failures retain their selected diagnostic contract as InvocationDiagnostic and append no fault. E4 distinguishes stored-data decoding from direct typed/slot errors under section 6.2; E5 retains its selected native constructor fields. No native owner/downcast/projector is needed by a receiver. Explicit resume retries permitted unfinished work; there is no automatic retry loop or internal-failure classification. |
| C10 | Local preflight mismatch performs no provider call/append; post-response binding rejection reports the true stage without appending unauthenticated evidence or falsely claiming no IO occurred. |
| C11 | E3 reports complete admitted failed outcomes with independent recording causes. BeforeAppend requires an admitted Failure and preparation diagnostic; other pre-append faults use ordinary internal diagnostics, with no unsent bytes/identity. Failed first encoding of a non-Error declared original uses the ordinary internal diagnostic: encoding_target, known position/contract and unavailable original contents/identity appear when detail construction succeeds; otherwise section 9.1's fixed invocation marker applies. Primary cause code/operation/size and the invocation head survive either case. It retains no arbitrary E, retries no serializer, appends nothing and classifies nothing. Later stages reuse the same Failure/Object. Success plus recording failure invents no original execution error or custody stash. Exact submitted candidate/disposition is retained once in direct Store/NotInserted variants; Store errors return without probe fields or audit retry. |
| C12 | The NotInserted probe covers exact matching candidate, conflicting occupied sequence, candidate followed by later commits, absent candidate, and failed reload/projection. A match with a valid latest view returns normally and yields. A checked finding, including Present, survives later decode/projection failure alongside its secondary cause; failed load/binding supplies no finding, never Absent. Missing rows at/below the head are corruption. No probe path drives further work. A candidate-absence snapshot cannot resolve a still-in-flight COMMIT; the Store-error invocation returns with its ambiguity/custody. Fresh resume may use current phase authority without claiming it resolved an unavailable prior candidate. |
| C13 | Atomic append, immutable earlier frames, exact-head conflict, checked cumulative bounds, consistent loads, and PostgreSQL acknowledgement ambiguity hold through Store public boundaries. |
| C14 | E5's empty and oversized metadata.correlation constructor failures reach Runtime/Application with location and reviewed length facts. K1 proves E4's checked rejection and narrower parser/status contract, including nested-size handling. Ordinary Serde structural forms still obey checked-value/frame/current-state invariants. Read/Effect callbacks keep borrowed checked identity arguments. No parent seeds, re-encoding validator or additional constructor family is implied. |
| C15 | E1-E6 and sections 5/9's closed recipes retain selected causes and typed classification. One Values DiagnosticEvidence supplies nested owner data and InvocationDiagnostic.details with no independent quota. Whole-owner/Object restoration and FailureReport admit the trusted text profile; ordinary Program/context checks and float/actual-size rejection remain. The two invocation-only encoding_failed/panicked markers preserve code/operation/size and never become persisted original substitutes. E4's nested stored-object size exception remains internal/500. No mfm-diagnostics, source/fact/omission framework, NativeCause, projector registry, diagnostic-budget machinery, size-source downcasts, consumer recapture or generic reporting tree remains. |
| C16 | Actual oversize/frame/run rejection leaves the acknowledged head unchanged. Cover a failure that committed but whose recovery cannot fit, and an externally settled Effect whose settlement cannot fit. No future-capacity promise remains. |
| C17 | App/CLI/REST show the retained current result or E1-E6 supplied/required causal invocation report with section 10.1's exact statuses and accurate acknowledgement/delivery status, including known insertion followed by projection failure. They do not reconstruct history, generate internal-fault records, or deliberately append MFM secret inputs. Upstream diagnostic content follows section 4; broad startup/ingress enrichment is outside both RFCs. |
| C18 | K1-K4 have integrated deletion and retained-behavior evidence, including checked decoding, one record, no native custody and local Read-input reuse without a cache or moved evidence binding. G1 accepts the complete corrected core on its own cost and guarantees. F1 records the accepted Part 1 commit, finalized APIs, shipping bytes/load IO, cost against both baselines and final verification on that candidate. Part 2 readiness or completion is not required. |

No test calls a historical reconstruction implementation merely to compare it with the new one.
Use baseline observable behavior where it remains contractual, concrete expected transitions, and
hostile physical/current-state inputs. Update old-format fixtures under the single-current-design
policy; do not retain a reader to keep obsolete fixtures passing.

## 14. Deletion ledger and completion

Preserve these behaviors already present at the implementation baseline. They are not additional
work to repeat or deletion credit against that baseline:

| Established behavior | Contract to preserve |
| --- | --- |
| Current continuation | Store current execution facts; no semantic history scan or fold. K2 removes the remaining duplicate phase/facts representation. |
| Shared live/stored value carrier | Keep Object's checked identity/canonical/hash/bounds and borrowed getters. K1 changes decoding, not the value representation; no qualified/native cache. |
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
| ObjectSeed, CheckpointsSeed/Visitor, generated parent/collection seeds and macros in runtime/state/decode.rs | K1 replaces these with ordinary checked Object Deserialize and derived record decoding before K2 changes the record. Preserve rejection and selected E4/E5 diagnostics; delete seed-specific assertions. |
| mfm-diagnostics; SourceChain/SourceLayer/SourceKind/SourceFact, ResponseContext, omission vocabulary, EvidenceError, builders and capture bookkeeping | Replace with Values' transparent DiagnosticEvidence and the two local recipes. Delete crate/API/test/doc/dependency references. Keep only the owner-local ObservedSize payload; no shared source taxonomy. |
| HttpStatusCode and AdapterFailure::HttpStatus | Keep the already checked reqwest::StatusCode locally; delete reconstruction and its impossible failure path. |
| Read runner's second S::Input decode | K4 retains the first typed input locally through preparation/interpretation. No new Driver field, Clone bound, alternate resume path or change to admitted-evidence binding. |
| NativeCause, Captured, `Owner<E>`, from_error/from_original/from_error_with, ProjectionPanicked and local omission serializer | InvocationDiagnostic contains one selected JSON value and the two literal fallbacks; no native owner or stored callback. No capture-failure subsystem replaces them. |
| Diagnostic quotas and accounting: 8 KiB evidence bound, invocation ceiling, layer/fact/omission counts, bounded diagnostic writer, metadata reservations and bound_reached/truncation | Delete these policies, helpers and obsolete assertions. Actual execution/Object/frame/run/report limits retain their current owners. |
| Runtime RecordingFailure/RuntimeError Wire projectors and Values ValueError Wire projector | Keep ValueError's eight concrete variants with derived Serialize and one typed into_diagnostic helper. Delete serializer mirrors and generic projectors; follow sections 9.5-9.6. |
| AppendFailure; BeforeAppend.candidate and seal-error unpacking/repacking | Section 9.4 uses direct Store/NotInserted variants. BeforeAppend requires an admitted Failure; other pre-append failures use ordinary diagnostics. Retain exact candidate bytes only after submission. |
| ReturnedFailure and extra native-original Arc/custody | Reuse complete admitted Failure/Object. Failed first encoding reports explicit unavailable original detail without retaining arbitrary E. |
| App ReportStage, `ReportFailure<T>`, ReportDetail, IncompleteReport, duplicate Omission/ObservedHead; CLI Ordinary | Remove the generic reporting-failure framework. Use existing invocation/head/disposition and terminal output facts through one renderer. |
| CLI Fields/MissingField | Render text from typed report data; no JSON reparse or missing-own-field failure path. |
| DomainFailure/ReadFailure, PendingFailureView, owned AdapterIncidentView/FailureCauseView; RecoveryStopped incident/reason copies | Section 10.1 reuses Failure/EffectCall/Settlement in owned public observations. RecoveryStopped retains only observed. App's necessary borrowed wire serializers use that data directly. |
| record original/yield_after arguments | Derive the admitted failure and return disposition from the proposed/acknowledged operation under section 8.5. Remove yield_after in K2 and native-original custody in K3; add no replacement carrier. |
| InitialValueMismatch; Runtime TaskFailure and Diagnostics TaskFailureKind | Reuse the identity-mismatch payload. Runtime and Live task diagnostics use panicked/cancelled strings; add no type or dependency. |
| SizeViolation/SizeResource in Runtime; cause_size | Move the existing two data declarations to Values and delete source/downcast discovery. The separate EVM ObservedSize keeps its concrete exact/lower-bound meaning at that owner. No parallel size hierarchy. |
| AdapterReturn | Delete it and ReturnedFailure. Section 9.6's nested Result separates internal failure from observed/admitted operational outcomes using existing types. |
| CanonicalSource and canonical Cause companion; OutputIoError | Flatten the canonical owner as specified in section 9.5; inline the output wrapper's selected data in OutputWriteError using section 5.3. No parser/IO redesign or new capture service. |
| Per-caller upstream sanitizers and native-owner certification | Use the explicit upstream trust boundary and deliberate MFM-input handling; no replacement certification framework. |

The following data and small helpers earn their place through a concrete responsibility. Keep
the smallest visibility and implementation; this does not approve every existing method or copy.

| Retained type/data | Reason and limit |
| --- | --- |
| Object; RunRecord/RecordedOperation | One checked value eraser at section 6's explicit heterogeneous/persistence boundaries and one complete continuation. Typed callbacks keep concrete values; no raw carrier, native cache, second state or persisted diagnostic envelope. |
| DiagnosticEvidence; InvocationDiagnostic | One Values-owned JSON data wrapper with constructor/getter and nested PersistedSchema; one invocation envelope with code/operation/primary size. Share the data, keep classification typed, and add no standalone MfmValue, native owner or capture factory. |
| ValueError; JsonError; CanonicalError | Concrete Values rejection and necessary native JSON/canonical source ownership. Section 9.5 defines the minimal representations; no serializer mirror or arbitrary-error carrier. |
| Call, EffectCall, Settlement, StateCall, Failure | Factor actual input/position, command authority, accepted evidence and original failure once. Distinguish valid operation facts without a bag of optional fields. |
| Checkpoint, StateUsage | Save authorized restart input and committed allowances. They do not restore a periodic-snapshot service, fold or historical counter scan. |
| RecoveryOutcome | The committed authorization differs from the handler request; reuse existing recovery vocabulary. |
| LoadedRun; CandidatePresence, RecordingFailure | One consistent Store observation; exact candidate presence/exclusion/snapshot absence; independent original/recording causes. Only submitted candidates need error custody; no second audit sink or nested append-result wrapper. |
| Driver, ValueContract, StateInvariant | One private executor/association/admission boundary. ValueContract replaces ValueCodec/ColdQualifier. Remove unused data and phase-agreement checks. |
| Journal Envelope, EffectId Preimage, bounded Writer | Exact wire/hash inputs and bounded accumulation have one concrete owner. No generic codec or accounting framework. |
| Operation/Stage, FrameOperation; OutputWriteError, OutputStream, WriteStage | Retain actual local operation and delivery facts once. Use an existing owner variant instead where equally clear; add no global error taxonomy. |
| Prepared, InvocationWire, candidate identity Candidate, Object output Wire | Tiny borrowing views are allowed for a real public wire distinction. Reuse the Object renderer instead of report-specific WireObject reconstruction. |
| BalanceMetadataError; BalanceContextDecodeError, MetadataWire/ContextWire and Object input Wire | Keep actual E5 constructor facts; inline the one-case location wrapper when equivalent. Prove the smallest local parse/admit helpers needed; no parent grammar or general constructor migration. |
| AdapterFailure, local TaskOperation and parser Base/Parse helpers | Keep selected actual adapter check/source fields. Private foreign-error adapters may be needed where Serialize is absent; no new public schema/registry or upstream library rewrite. |

DiagnosticEvidence's old framework is deleted, not retained vocabulary to expand later. Its small
Values replacement has its own actual cost. NoParams still supplies parameterless policy
configuration; CanonicalError and EVM provider errors keep their concrete owner roles. Count
removals only against a baseline containing them, including relocated code in replacement cost.
FailureReport retains the existing Failure and its required canonical artifact under section 10.1;
no owned FailureCauseView remains. Test helpers are reviewed with their retained behavior; their
count is not production growth.

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

Before handoff, designers must close these focused seams; they are not delegated implementation
choices:

- Section 6.1 fixes work selection ownership and rules in existing Runtime transition/dispatch. Check
  the actual callers before specifying any optional borrowed helper: identify duplicated rules it
  removes and pin its minimal interface, or use the existing dispatch directly.
- Runtime's existing projection/reload error routes retain observed facts for the invoker under
  section 9.4. Specify the small conversions replacing into_native, forwarding existing diagnostics,
  operation/stage, primary size and known head/probe facts; no new carrier or reporting subsystem.
- CLI/REST terminal failure handling after deleting ReportFailure: fix concrete local return/body
  ownership and final exit/response behavior without an erased or recursive reporting carrier.

The remaining assumptions below require implementation and acceptance evidence. Part 2 has its
own refinement gate and pending producer designs.

| Assumption | Why uncertain | Consequence if wrong | Validation and response |
| --- | --- | --- | --- |
| The unified record removes more mechanism than its dispatch requires | The implementation baseline contains duplicate phase/facts data; the unified production diff is not implemented | A renamed validator could preserve the same complexity | K2 replaces the real core route, deletes the old pair and compares retained behavior, encoded occurrences and actual code. |
| Ordinary checked decoding removes the seed grammar while preserving required validation | The structural acceptance and diagnostic contracts are settled; the real decoder cutover is not implemented | Invalid values could enter Runtime or obsolete map-only/native-error assertions could recreate decoder machinery | K1 tests accepted Serde forms plus reference/hash/oversize/current-state rejection, direct typed/slot fields, E5 and actual callbacks. No whole-record re-encoding pass. |
| Shared diagnostic data removes more machinery than its conversions require | The minimal wrapper, trusted-text profile and local source recipes are specified but not implemented | A generic capture factory, duplicate serializer or retained diagnostic framework could erase the simplification | K3 verifies E1-E6 fields and two invocation fallbacks, deletes the diagnostics crate, and measures the full shared-data/owner/report replacement. |
| The new nested field and terminal report share the intended admission policy | PersistedSchema/derive and whole-report integration have not been implemented together | Valid diagnostic text could be rejected by an outer validator, or ordinary input/float checks weakened | Admit and restore the real operational owner and FailureReport with selected messages, compare typed classification and test ordinary-input, float and actual-size rejection. Reuse the existing terminal validator; add no second grammar. |
| Part 1 alone meets the simplification objective | The final replacement cost and responsibility removals are not implemented or measured | Reduction against the implementation baseline could still leave an unjustifiably larger core overall | G1 reviews actual Part 1 removals, replacements and total cost against both baselines. Unsupported net reduction or complexity improvement keeps Part 1 unaccepted; no credit from future Part 2 deletions. |

Normal loading deliberately trusts past live progression and append-only storage; it does not
verify historical semantic evolution or older frame links. Internal errors remain outside history.
The agreed upstream diagnostic trust boundary is distinct from deliberate inclusion of MFM secret
inputs and from post-crash delivery.
Concrete owner errors remain the classification input; failed initial encoding explicitly reports
original detail unavailable, with no opaque native custody. Checked Object at explicit boundaries,
ordinary Serde record forms with the narrower malformed-storage diagnostic/status contract, no
diagnostic quotas, one Values-owned diagnostic data type, minimal concrete errors, local Read-input
reuse and deletion-first
order are settled. These are accepted contracts;
implementation evidence and aggregate net simplification remain to be delivered.
