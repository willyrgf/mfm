# RFC: make Runtime, Journal, and Store one typed proof path

Status: proposed breaking architecture; implementation pending

Relationship: this RFC replaces the Runtime/Store/Journal ownership, erased-value flow, Store
identity model, fact-publication protocol, configuration persistence boundary, and replay layering
described by `RFC_REFACTOR_SINGLE_TRUST_BOUNDARY.md`,
`RFC_FOLLOWUPS_FROM_REFACT_RUNTIME.md`, `docs/design.md`, and `docs/architecture.md` where they
conflict with this target. The implementation must update the authoritative documents in the same
cutover. It must leave one current design, with no compatibility facade, legacy decoder, dual
schema, fallback identity, or parallel reducer.

---

## Decision

MFM will use strong Rust types as process-local proof that a value has already crossed its trust
boundary. A fact is checked where it first becomes untrusted, represented by a type with private
fields, and then carried without erasing and reconstructing that proof at every crate boundary.

The execution and persistence path has four owners:

| Owner | Sole responsibility |
| --- | --- |
| Program | The immutable, deterministic `State | Match` graph and its declared contracts |
| Runtime | Program reduction, next-State selection, typed State/capability execution, hot advancement, cold resume, and semantic replay |
| Journal | Canonical durable representation and one-time structural qualification of retained bytes |
| Store/PostgreSQL | Mechanical load, exact-head compare-and-append, idempotency, atomicity, durability, and database-wide sequence allocation |

The proof flow is:

```text
retained bytes
  -> Journal structurally qualifies bytes once
  -> Runtime reifies and semantically folds them once
  -> Runtime owns a typed execution cursor
  -> Runtime projects one typed transition into a Journal frame once
  -> Store/PostgreSQL mechanically appends it
```

PostgreSQL is one global authority for the MFM platform. Multiple processes and multiple `RunId`s
may execute concurrently against it. There is no MFM tenant namespace, Store scope, writer epoch,
active-identity row, or identity rotation protocol. `RunId` is the complete durable run namespace;
database compare-and-append linearizes concurrent writers.

Store does not reduce a Program and does not return `SelectedRun`, `RunAction`, or any equivalent
answer about what executes next. Runtime owns the fixed sequential state machine, so Runtime alone
derives the next action.

There is exactly one unavoidable execution-layer erasure boundary: Runtime's private heterogeneous
State registry. A retained Program selects implementations at runtime, while different States have
different `Input`, `Output`, `Failure`, capability `Intent`, and capability `Evidence` types. Rust
cannot represent that data-selected heterogeneous graph as one static generic chain. The erased
value therefore exists only long enough for a registered typed driver to recover its concrete
State proof. It never crosses into Store, Journal, PostgreSQL, App, replay, or an adapter.

This is not a claim that all runtime validation can disappear. Persisted bytes, provider responses,
transport input, and concurrent database state are untrusted. They must still be checked. The rule
is narrower and enforceable:

> Validate when trust changes; preserve the resulting proof; do not erase it merely to validate the
> same proposition again in another internal layer.

## Material uncertainties

The layer ownership, one private erasure boundary, Store selection deletion, global PostgreSQL
authority, and removal of scope/epoch/tenant are not uncertain. Four product semantics must be
ratified before implementation begins. The target sections below use the recommended choice so the
RFC remains one concrete design rather than a menu.

1. **Durable Access preparation recovery.** Recommended choice: preserve direct-new-only entry, so
   a process death after preparation commit and before provider entry leaves the run parked. It is
   uncertain whether the product instead requires automatic recovery of that attempt. If the
   recommendation is wrong, recreating `CommittedCall` from history would violate the safety proof;
   a durable outbox/lease/claim protocol is required and materially changes Runtime and PostgreSQL.
   Resolve by documenting the availability guarantee for current Read and Effect capabilities and,
   for any recoverable Effect, proving its absorption/idempotency contract before implementation.
2. **Configuration currentness.** Recommended choice: bind admission to the exact immutable
   revision used to construct `C0`, without requiring it to remain latest at commit. It is uncertain
   whether an entry point has a business rule requiring latest-at-admission. If it does,
   configuration-head CAS must join the admission transaction. Resolve by auditing the two current
   entry-point contracts and recording whether either promises latest-at-commit semantics.
3. **Fact-selection freshness.** Recommended choice: bind selection to the immutable snapshot
   Runtime read; later publications do not invalidate preparation. It is uncertain whether a
   capability requires every fact committed before its preparation append. If it does, a fact-head
   CAS and preparation retry remain necessary, although Store still does no semantic selection.
   Resolve by documenting each current fact consumer's freshness contract and adding a concurrent
   publication fixture.
4. **Global `RunId` creation and repetition.** Recommended choice: `RunId` is globally unique and is
   itself the admission idempotency identity; an independent repetition uses a new `RunId`. It is
   uncertain whether current entry points rely on deriving the same run from only entry point plus
   application selector. If they do, removing tenant may collapse intended independent runs.
   Resolve by inventorying current derivation/caller retry behavior and either accepting explicit
   `RunId` control or adding one explicit domain idempotency key—never another ambient namespace.

One further choice is settled: PostgreSQL assigns one global dense fact-publication sequence. That
coordinate is physical database index metadata and is not embedded in the content-addressed State
conclusion.

## 1. Problem situation

### 1.1 The system discards its own proof

The domain APIs begin with strong associations:

- `State::Input`, `State::Output`, and `State::Failure`;
- `AccessCapabilityContract::Intent`, `Evidence`, `Facts`, and `Mode`;
- `ProposedStateOutcome<O, F>`;
- typed access owners such as `CommittedCall<S, C>`; and
- `QualifiedTypedValue<T>` after canonical qualification.

The current implementation then erases these associations into contract references, canonical
bytes, `TypeId`, `Box<dyn Any>`, option bags, catalog brands, and Store-branded wrappers. Later
layers re-establish the same association with catalog lookups, contract comparisons, validation,
and downcasts. This creates a recurring pattern:

```text
typed value
  -> qualify
  -> erase type
  -> split value, contract, bytes, and catalog identity
  -> pass through a coordination DTO
  -> compare the pieces again
  -> downcast/reify
  -> wrap in another qualified owner
```

Most of the repeated validation is not protecting a new trust boundary. It compensates for an
earlier internal layer having discarded the proof.

### 1.2 Store became the semantic state machine

Store currently does substantially more than persistence. It:

- opens under a scope/epoch/tenant identity and brands every later owner;
- loads and validates raw Journal prefixes;
- retains and reifies the Program;
- validates typed admission, intent, evidence, outcomes, fact selections, and object closure;
- reduces `State | Match`;
- decides whether a run is ready, waiting, terminal, or failed;
- constructs `SelectedRun` and `RunAction`;
- prepares admission, preparation, and conclusion owners;
- allocates and rebinds fact-publication coordinates;
- exposes separate history, configuration, mutation, and audit ports; and
- maps all of that onto a second backend command/result vocabulary.

This is why `crates/kernel/store/src/backend.rs` is roughly four thousand lines and
`crates/kernel/store/src/single_trust.rs` is roughly twenty-six hundred lines in the current tree.
The problem is not primarily that one file needs to be split. The files contain responsibilities
from Runtime, Journal, configuration, facts, replay, authorization-like branding, and physical
storage. Splitting those responsibilities into more files without deleting them would preserve the
same conceptual and LOC cost.

### 1.3 Journal DTOs are treated as both wire data and semantic proof

Journal's durable types are necessary: a run must have a canonical, append-only representation.
The problem is that open DTO construction and repeated `validate()` calls make every downstream
consumer responsible for determining whether the same frame is trustworthy.

For example, a frame may be checked while being constructed, checked again before Store append,
wrapped as `RawFrameBytes`, checked again when loaded as a `RawRunPrefix`, and checked again while
Store qualifies and reduces the run. Portable replay adds another decode/validate/qualify path.

A wire type and a process-local semantic proof are not the same thing. Journal should construct an
opaque valid frame when encoding trusted input and an opaque qualified history when decoding
untrusted bytes. Store should not reopen either proof.

### 1.4 PostgreSQL identity is modeling authorities that do not exist

`StoreScopeId`, `StoreEpoch`, `TenantScopeId`, and `StructuredStoreIdentity` currently flow through
Run admission, every frame, Store opening, backend commands, SQL keys and predicates, facts,
configuration, portable replay, application facades, and EVM nonce authority.

Those values imply three distinctions:

- multiple logical platform authorities in one database;
- a tenant partition inside an authority; and
- a writer generation that makes older runs read-only.

They are not part of the intended product model. One PostgreSQL instance is the authority. Its
many processes are peers, and its many runs are distinguished by `RunId`. Exact-head CAS already
provides writer concurrency control. Persisted Program and binding content identities already pin
the implementation contract needed to resume a run. Scope, epoch, and tenant therefore add fields,
hash inputs, indexes, validation branches, error variants, and lifecycle rules without proving a
required property.

### 1.5 The current reverse flow puts Store after PostgreSQL semantically

The current cold path is effectively:

```text
PostgreSQL
  -> backend rows
  -> RawRunPrefix
  -> Store structural validation
  -> Store Program lookup/reification
  -> Store reducer
  -> QualifiedRun + RunSelection + RunAction
  -> SelectedRun
  -> Runtime dynamic reconstruction
  -> execute the Store-selected State
```

That makes PostgreSQL the source, Store the state-machine selector, and Runtime an executor of
Store's decision. This reverses ownership. Persistence should return retained data; Runtime should
interpret that data under the admitted Program and decide what runs next.

### 1.6 Several registries describe the same association

Program's catalog currently records type IDs, schemas, reifiers, and erased capability binding
callbacks. Runtime separately records State and adapter implementations in its process registry.
Store then uses Program's catalog to qualify retained values before Runtime uses its own registry to
recover concrete implementations.

The same association is therefore represented as:

```text
stable persisted contract ref
  <-> ProgramCatalog ValueAssociation / CapabilityAssociation
  <-> catalog Arc identity
  <-> Store qualification
  <-> Runtime RegisteredState / RegisteredAdapter
  <-> TypeId / Any downcast
```

Only two things are required: stable contract identities in the persisted Program, and one trusted
Runtime assembly that associates those identities with concrete Rust implementations. The middle
catalog/reifier route is duplicated authority.

## 2. Current erasure and indirection inventory

This section is intentionally explicit. The cutover is incomplete if the named concepts merely
move to another module or are renamed one-for-one.

### 2.1 Program erasure

| Current concept | Current role | Target disposition |
| --- | --- | --- |
| `ReifyValue` | Function pointer returning `Box<dyn Any + Send + Sync>` | Delete from Program's supported API; reification belongs to a private Runtime registration |
| `ValueAssociation` | Stores `TypeId`, schema, and erased reifier | Delete; stable contract metadata remains in Program, concrete type association moves to Runtime assembly |
| `CapabilityAssociation` | Stores `TypeId` and `bind_evidence(&dyn Any, &dyn Any)` | Delete erased callbacks; registered typed Runtime driver invokes `C` directly |
| `ProgramCatalogInner` | Maps stable references to the erased associations | Delete as execution authority |
| `ProgramCatalog` process-local `Arc` identity | Brands values and proves they came from the same catalog instance | Delete as a cross-layer proof; validate one `ExecutableProgram` against one Runtime assembly |
| `ProgramCatalogBuilder` | Builds the duplicate value/capability execution association registry | Delete as a separate registry; typed registrations belong to `RuntimeAssemblyBuilder` |
| `ProgramIngress` bound to `ProgramCatalog` | Couples hostile Program decoding to process-local execution associations | Replace with a strict catalog-independent Program decoder; Runtime separately creates `ExecutableProgram` |
| `qualify_retained_erased` | Turns retained bytes into a generic value before later downcast | Delete from public flow; the selected Runtime driver reifies its exact input once |
| `QualifiedTypedValue<T>::erase` | Discards `T` while retaining bytes and catalog | Remove from supported API |
| `QualifiedValue` | Public erased value containing refs, bytes, `Any`, and catalog | Delete from Store/Journal/App/replay APIs; at most retain a sealed Runtime-internal existential payload |
| `QualifiedValue::try_downcast` | Reconstructs a typed value after internal erasure | Replace with the one private registry handoff |
| caller-supplied contract ref plus erased value | Allows correlated facts to travel independently | Replace with one proof object whose private fields cannot disagree |

Program construction still validates graph shape and adjacent stable contracts. Persisted Program
decoding still validates its canonical document. Neither operation needs an erased process-local
value catalog.

### 2.2 Runtime erasure and dynamic middlemen

| Current concept | Why it exists now | Target disposition |
| --- | --- | --- |
| `ErasedValue = QualifiedValue` | Common currency between Runtime and Store | Delete as a cross-layer currency |
| `DynamicOutcome` | Erased success/failure values | Delete; retain `ProposedStateOutcome<S::Output, S::Failure>` until Journal encoding |
| `DynamicPreparationFailure` | Optional erased failure plus coordination error | Replace with the registered State's typed failure/result |
| `DynamicStateRegistration` | Erased State dispatch and qualification callbacks | Replace with a private object-safe driver whose concrete implementation immediately enters `S`/`C` typed code |
| `DynamicPrepared` | Erased prepared execution | Replace with `PreparedAccess<S, C>` behind the private cursor existential |
| `DynamicCommit` | Combines erased call/prepared owners with `SelectedRun` | Delete; Runtime's affine cursor already owns the correlated values |
| `DynamicCall` | Erased provider-call authority | Delete; retain typed `CommittedCall<S, C>` |
| `DynamicPure<S>` / `DynamicAccess<S, C>` | Adapters from typed implementations to erased lifecycle | Replace with private `ReadyPure<S>` / `ReadyAccess<S, C>` proof states |
| `TypedPrepared<S, C>` / `TypedCall<S, C>` | Typed islands inside an erased outer protocol | Collapse into the Runtime cursor's typed lifecycle |
| `DynamicResolution` | Correlated `input`, `evidence`, `outcome`, `classification`, and fact continuation stored as independent `Option`s | Delete the option bag; a concrete typed conclusion owner carries only a valid combination |
| `RunSession.latest: ErasedValue` | Keeps the current context after Store selection | Replace with a private current-context proof owned by `ExecutionCursor` |
| `RunSession.selected: SelectedRun` | Carries Store's semantic decision | Delete; cursor state is derived by Runtime |
| duplicate Program catalog and Runtime process registry | Separates contract reification from implementation dispatch | Keep one Runtime assembly for concrete dispatch; Program retains only stable declarations |

`Box<dyn Any>` is not forbidden as an implementation technique. Its spread is. If the cross-crate
boundary forces a sealed erased payload to live in `mfm-program`, it is an implementation-only type
usable solely by `mfm-runtime`, not a supported public data model. It may not be stored, cloned into
DTOs, inspected by Store, or supplied with an independently forgeable contract reference.

The current public lifecycle vocabulary is also wider than the process-facing contract requires:

| Current lifecycle concept | Target disposition |
| --- | --- |
| `AdmissionInput<T>` | Collapse to the one typed Runtime start request; qualification immediately yields the cursor's admitted `ProvenValue<T>` |
| `RunSession` | Replace as a public coordination object with Runtime's private `ExecutionCursor` |
| `PendingConclusion` | Retain only as the typed affine owner needed for unknown acknowledgement and durable-result release |
| `SuspendedRun` | Keep private if needed to retain an ambiguous append owner; App must not coordinate it |
| `ParkedRun` / `ParkReason` | Project to one read-only process-facing status, not a Store-selected owner |
| `TerminalRun` | Project the typed durable result/failure; do not carry Store selection internals |
| `SpawnStep` / `ResumeStep` / `RuntimeStep` | Collapse to one high-level Runtime progression result; start and resume do not expose parallel lifecycle algebras |
| `AdmissionFailure` / `AdmissionConflict` / `ResumeFailure` | Collapse overlapping transport-shaped classifications into one typed Runtime error/progress contract |
| `RuntimeLimits` | Retain only Runtime work/concurrency policy; do not duplicate Journal format or Store physical limits |

App asks Runtime to start, resume, or inspect a run. Runtime internally drains immediately
actionable sequential States until a stable external boundary. App does not hold `RunSession`,
match `SpawnStep` versus `ResumeStep`, or shuttle Store owners back into Runtime.

### 2.3 Store semantic middlemen

| Current concept | Target disposition |
| --- | --- |
| `StoreBrand` | Delete; it proves only that callers passed through one Store facade |
| `StructuredStoreIdentity` | Delete with scope/epoch/tenant |
| `StructuredStore` / `OpenedStructuredStore` / `OpenedStoreInner` | Replace with one configured mechanical `Store` implementation |
| `StoreParts` | Delete the port-splitting ceremony |
| `QualifiedHistoryPort` / `HistoryReader` / `StoreAuditPort` | Collapse to mechanical load/list methods plus Runtime inspection |
| `QualifiedRun` | Delete; Journal owns structural qualification and Runtime owns semantic qualification |
| `RunReducer` / `advance_selected` | Move the one reducer to Runtime, then delete Store's implementation |
| `RunSelection` | Delete |
| `RunAction` / `AccessActionMode` | Delete from Store; Runtime-private cursor variants express the action |
| `SelectedRun` | Delete; Store never selects execution |
| `PreparedAdmission` / `AdmissionOutcome` | Move admission ownership and classification to Runtime; do not reproduce Store-branded wrappers |
| `PreparationAppend` / `AccessPreparationCandidate` | Replace with typed `PreparedAccess<S, C>` owned by Runtime |
| `AccessPreparationOutcome` | Replace with mechanical append outcome plus Runtime promotion |
| `PreparedConclusion` | Replace with a typed Runtime conclusion owner and direct Journal encoding |
| `SelectedConclusion` and `SelectedConclusionOutcome` | Delete; Runtime classifies a conclusion race by folding the reloaded history |
| `SelectedConclusionPreparationOutcome` | Delete |
| `FactContinuation` | Delete Store-branded continuation; typed Runtime preparation retains the selection it used |
| fact publication bind/rebind ordinal | Delete by moving coordinate allocation out of the Journal frame |
| `SemanticStore` or equivalent semantic facade | Delete rather than rename |
| `retained_program` | Move retained Program decoding/association to Runtime's cold fold |
| `replay_terminality` | Replace with Runtime's one callback-free reducer/inspection path |
| `validate_prefix` | Replace with Journal structural qualification followed by Runtime semantic fold |

These types are the primary reason Store appears after PostgreSQL and before Runtime in the
semantic flow. Removing them is the architectural change; placing them in a `selection.rs` file is
not.

### 2.4 Store backend DTO and port inventory

Some storage command/result data is necessary. The target is one DTO per real database operation,
private implementation row structs, and no duplicated semantic wrapper.

| Current exported concept | Target disposition |
| --- | --- |
| `StructuredStoreBackend` | Replace with the small `Store` trait |
| `MemoryStructuredBackend` | Replace with `MemoryStore` implementing the same contract as PostgreSQL |
| `BackendFuture` / `BackendResult` | Delete public aliases; use ordinary async trait methods and one typed error |
| `BackendError` / `StoreOpenError` / overlapping `StoreError` classifications | Collapse to errors at the mechanical Store boundary; Runtime owns semantic failures |
| `StoreWorkLimits` | Delete duplicated limit ownership; Journal owns format limits and PostgreSQL enforces matching physical ceilings |
| `RawHistoryLoadLimit` | Delete the special wrapper; use the protocol limit owned by Journal/load policy |
| `RawFrameBytes` | Replace with a private loaded-row byte type handed directly to Journal decoding |
| `RawRunPrefix` | Replace with one `StoredRunBytes` load result; it carries bytes, not semantic validity |
| `BackendAppendCommand` | Replace with `RunAppend` |
| `BackendAppendOutcome` | Replace with `AppendOutcome` and `AppendReceipt` |
| `AppendDisposition` | Fold its mechanical variants into `AppendOutcome`; keep no second result enum |
| `BackendConfigurationOutcome` | Collapse into the one configuration append result |
| `ConfigurationAppendCommand` | Replace with a minimal mechanical configuration append command |
| `ConfigurationCommitOutcome<C>` | Remove the generic Store proof; Runtime reifies the exact committed revision when needed |
| `ConfigurationAppendDisposition` | Fold into the one mechanical configuration append result |
| `RawConfigurationRevision` | Keep only a private/opaque stored-byte row at the Store boundary |
| `RawFactPublication` / `RawFactSnapshot` | Replace with one structural fact snapshot load result; Runtime owns `FactSelection` semantics |

The replacement is intentionally small:

```text
Store
  load_run
  append_run
  list_run_ids
  load_fact_snapshot
  load_configuration
  append_configuration
  check_ready
```

An illustrative append contract is:

```text
RunAppend {
  expected_head,
  append_request_id,
  journal_frame,
  optional_conclusion_reservation,
  optional_fact_proposal_ref,
}

AppendOutcome {
  Inserted(AppendReceipt),
  Existing(AppendReceipt),
  Stale { actual_head },
  AcknowledgementUnknown,
}

AppendReceipt {
  run_sequence,
  run_head,
  optional_fact_publication_sequence,
}
```

The exact Rust spelling is not normative. The absence of Program, State, capability, contract,
evidence, selection, and catalog types from this API is normative. The optional reservation and
fact reference are physical instructions computed by Runtime; Store does not interpret State
semantics to derive them.

### 2.5 Configuration middlemen

| Current concept | Target disposition |
| --- | --- |
| `ConfigurationStore` | Delete the separate semantic Store facade |
| `ResolvedConfigurationHead` | Replace with a small immutable persisted revision reference |
| `ResolvedConfiguration<C>` | Move typed reification to Runtime/application composition |
| `ConfigurationWriteSession<C>` | Collapse into one Runtime-owned typed pending write |
| `PreparedConfigurationAppend<C>` | Collapse into the same pending write |
| `SuspendedConfigurationAppend<C>` | Retain unknown-ack ownership only as a state of that one pending write, not another Store layer |
| Store brand/global-head fields in configuration proofs | Delete |

Configuration persistence is global. A revision has one global sequence and content reference.
Admission records the exact immutable revision it used. Store handles bytes and configuration-head
CAS; Runtime or trusted application composition decodes `C` once.

### 2.6 Journal DTO inventory

The following are genuine durable representation concepts and are not deleted merely because they
are DTO-shaped:

- `ValueRef` and `ImmutableObject`;
- `RunAdmitted`, `StatePrepared`, and `StateConcluded`;
- `StateOutcome`;
- `FactSelection`, fact provenance, and fact proposal-set references; and
- `RunFrame` and its recursive head.

They change in two ways. First, their fields become private and their constructors/decoders return
valid-by-construction Journal types. Second, fields that duplicate the surrounding frame, admitted
Program, or physical Store protocol are deleted.

| Current Journal concept/field | Target disposition |
| --- | --- |
| `ValueRef` | Retain as persisted identity; consider the clearer name `PersistedValueRef` |
| `ImmutableObject` | Retain as durable object closure; consider `JournalObject` |
| `RunAdmitted.scope`, `.epoch`, `.tenant`, duplicated `.run_id` | Delete |
| `RunAdmitted` entry point, Program ref, admitted context, configuration ref, and source refs | Retain |
| `ConfigurationHeadProjection` | Replace with the single immutable `ConfigurationRef { revision_sequence, content_ref }` |
| `PreparationMode` | Delete; mode is declared by the admitted State/capability |
| duplicated prepared input | Delete; Runtime derives it from the predecessor context |
| preparation ordinal/replacement ref | Delete; one frame has one record and its sequence is the preparation identity |
| duplicated execution-binding ref | Delete; the admitted Program pins it |
| duplicated fact request | Delete; retain the exact selected facts used by the attempt |
| persisted maximum-conclusion bytes | Remove from semantic Journal bytes; keep any required reservation as physical append metadata |
| `PreparationRef { run_id, run_sequence, record_ordinal }` | Reduce to preparation frame sequence; containing run supplies `RunId` |
| duplicated fact selection in `StateConcluded` | Delete; Access conclusion refers to its preparation |
| `FactPublication` in `StateConcluded` | Delete; PostgreSQL publication index owns the coordinate |
| `FactSelectionFrontier.stream_ref` / `.head_ref` | Delete under the singleton global fact stream; publication sequence and per-fact provenance carry the required evidence |
| `FactCompleteness { through_sequence, complete }` | Delete the always-true validity flag and fold `through_publication_sequence` into valid `FactSelection` |
| `RecordLogicalKey` | Delete from public/persisted vocabulary; Runtime derives occurrence identity while folding |
| `RunFrame.scope`, `.epoch`, and append-request ID | Delete; append ID is Store protocol metadata, not semantic content |
| public `validate()` choreography | Delete; construction or strict decode yields an opaque valid frame/history |

Journal decoding must still reject malformed or non-canonical data, floats in hashed structures,
invalid content references, object collisions, missing object closure, sequence gaps, wrong
recursive heads, protocol-limit violations, and invalid intrinsic record shape. Those are byte-
ingress checks. Journal does not validate State transitions or capability meaning; Runtime does so
while folding the qualified history.

### 2.7 Typed owners that carry real proof

The cutover does not delete types simply to reduce a count. A type remains when possession of it is
the enforcement mechanism for a meaningful state transition.

| Typed concept | Disposition |
| --- | --- |
| `ProposedStateOutcome<O, F>` | Retain; it preserves the State's success/failure association |
| `CommittedCall<S, C>` | Retain as an affine provider-entry authority, without scope/epoch/tenant fields |
| `AcceptedOutcomeAccess<S, C>` / `AcceptedIntegrityAccess<S, C>` | Retain: they distinguish conclusive typed outcome evidence from a non-success integrity block |
| `UnresolvedAccess<S, C>` / `AccessResolution<S, C>` | Retain: they keep unresolved provider results from becoming conclusive evidence or retry authority |
| `QualifiedRecordedEvidence<C>` | Delete; its currently unused wrapper duplicates the preparation/evidence binding owned by Runtime's cold typed fold |
| `PreparedExecution<S, C>` / `OpenedPreparationCommit<S, C>` | Consolidate into `PreparedAccess<S, C>` plus the append result that promotes it |
| `PendingConclusion` | Keep a typed Runtime owner; do not erase its evidence/outcome before encoding |
| `QualifiedTypedValue<T>` | Replace or seal as a private-field `ProvenValue<T>`; no public erase/downcast protocol |
| `QualifiedAdapter<S, C>` | Retain a typed inert adapter gate if it still binds one registered implementation to `CommittedCall<S, C>`; remove forwarding-only catalog/Store identity checks |
| `RuntimeAssembly` / `RuntimeAssemblyBuilder` | Retain as the sole concrete State/capability/adapter registry and the owner of any private process-local assembly proof |

Public surface reduction is not the goal by itself. The goal is fewer forgeable combinations and
fewer translations. An affine owner that makes a forbidden operation impossible is useful; a DTO
that copies the same fields between owners is not.

### 2.8 Replay and portable middlemen

`mfm-replay` currently uses Store qualification and Store's reducer, while its `PortableRun` /
`PortableEnvelope` adds scope, epoch, and tenant to another representation of the same history.

The target deletes the `mfm-replay` middle layer:

- move portable structural encoding/decoding to Journal;
- remove scope, epoch, and tenant from portable history;
- expose callback-free semantic inspection from Runtime's one reducer; and
- have App's replay/status surface call that inspection path.

Portable semantic history does not require a fact-publication coordinate to reproduce the run.
An audit export may additionally carry the PostgreSQL publication-index row, but it is explicitly
physical audit metadata rather than part of `StateConcluded`.

## 3. Proposed solution

This is a breaking replacement of the current responsibility chain, not an adapter around it. The
solution has six coupled moves:

1. keep values typed from their first trusted construction until Journal encoding;
2. confine heterogeneous erasure to one private Runtime registry handoff;
3. move the sole Program reducer and next-State selection from Store to Runtime;
4. make Journal construction/decoding the only structural frame/history validation boundary;
5. replace semantic Store/backend layers with one mechanical persistence contract; and
6. make one PostgreSQL authority global by deleting tenant, scope, epoch, and identity rotation.

The following validation split is the test for whether those moves were implemented rather than
renamed.

### 3.1 Validations that are necessary

Strong typing cannot prove facts about bytes or concurrent external state before those inputs are
observed. These checks remain:

| Proposition | Checked by | Proof/result |
| --- | --- | --- |
| Transport input is bounded and conforms to its request schema | CLI/REST/application ingress | Typed request |
| A domain value obeys `MfmValue`, canonical encoding, size, and no-float persisted rules | Its first typed persistence/ingress constructor | `ProvenValue<T>` |
| Provider bytes are authentic enough for the capability and bind to the exact intent | Adapter/capability ingress | Typed accepted evidence |
| Retained Program bytes are canonical and graph/contract structure is valid | Program decoder | Validated Program document |
| Retained Journal bytes, hashes, object closure, sequence, and recursive head are valid | Journal decoder | `JournalHistory` |
| Retained values match the exact State contracts selected by the Program | Runtime's typed registry during one cold fold | `ExecutionCursor` or terminal inspection |
| Retained transitions are legal for `State | Match` | Runtime reducer during one cold fold | Reduced semantic history |
| Expected run head is still current | PostgreSQL transaction | Inserted/existing/stale append outcome |
| Append request is new or an identical retry | PostgreSQL unique key and transaction | Append receipt |
| Frame, object, fact-index, reservation, and head writes are atomic | PostgreSQL transaction / Memory conformance | Durable receipt |
| Fact and configuration global sequences are dense/current where required | PostgreSQL singleton heads and CAS | Mechanical snapshot/receipt |

Cold resume necessarily repeats semantic work performed by a process that no longer exists. That
is not duplicate validation inside one proof lifetime; it is creation of a new process-local proof
from untrusted retained bytes.

### 3.2 Validations that disappear

The following checks are redundant once their input is a private-field proof type:

- Store revalidating a typed admission value just qualified by Runtime;
- Store revalidating Runtime-produced intent, evidence, output, or failure against the same Program
  catalog;
- Runtime revalidating the `SelectedRun` that Store just derived from that same Program;
- catalog-`Arc` equality checks at each internal handoff;
- `RunFrame::new(...).validate()` followed by Store frame validation;
- `RawFrameBytes` rechecking canonical bytes and digest produced by Journal encoding in the same
  process;
- `RawRunPrefix`, `QualifiedRun`, and replay independently checking the same loaded prefix;
- contract refs carried separately from a typed value and compared again downstream;
- Store fact-publication coordinate binding, frame rebuilding, and coordinate revalidation;
- scope/epoch/tenant equality checks in every open, load, append, hash, and query; and
- Store replay terminality checks duplicated by Runtime's execution reducer.

The implementation should inventory each `validate`, `qualify`, `reify`, `try_downcast`, contract
comparison, and catalog-brand comparison. Every remaining call must name the trust transition it
protects. “Another crate called us” is not a trust transition.

## 4. Target Runtime design

### 4.1 One affine execution cursor

Runtime owns one internal `ExecutionCursor`. It retains:

- the Journal-qualified history;
- the retained Program qualified against the current Runtime assembly;
- current Program address;
- exact current run head;
- latest typed context proof; and
- exactly one private typed step owner.

Its conceptual states are:

```text
ReadyPure<S> {
  input: ProvenValue<S::Input>,
}

ReadyAccess<S, C> {
  input: ProvenValue<S::Input>,
}

PreparedAccess<S, C> {
  input: ProvenValue<S::Input>,
  intent: ProvenValue<C::Intent>,
  binding,
  fact_selection,
}

CommittedCall<S, C> {
  exact preparation identity and typed call authority,
}

TypedConclusion<S, C> {
  evidence: ProvenValue<C::Evidence>,
  outcome: ProposedStateOutcome<S::Output, S::Failure>,
  fact_proposals,
}
```

These names are explanatory, not a requirement to expose five new public structs. They may be
private variants implemented by concrete generic drivers. The normative property is that input,
intent, evidence, outcome, Program occurrence, and preparation identity remain correlated by a
single owner.

### 4.2 One private heterogeneous dispatch

Runtime assembly maps each stable State declaration to one registered concrete driver. Lookup is
dynamic; execution after lookup is typed:

```text
Program StateRef
  -> Runtime assembly lookup
  -> private existential driver
  -> recover ProvenValue<S::Input>
  -> ReadyPure<S> or ReadyAccess<S, C>
  -> typed State/capability methods
```

The driver reports an internal typed error on a mismatch and never panics. Such a mismatch means
retained data, Program association, or trusted assembly is inconsistent. Store cannot discover or
repair it.

On the hot path, Runtime promotes the typed output it already owns into the next cursor state. It
does not serialize, erase, ask Store to requalify, and then deserialize its own output. Journal
bytes are built for durability in parallel with that retained proof.

On cold resume, the selected driver decodes each required retained value into its exact type once
while the reducer folds history. Once the cursor exists, hot execution uses that proof.

### 4.3 Runtime owns selection

There is one reducer for execution, cold resume, status, and replay. It consumes a validated Program
and `JournalHistory` and yields either:

- a private ready typed step;
- a waiting preparation/conclusion condition;
- a parked unknown-ack owner when one exists in the current process;
- terminal typed success;
- declared typed failure; or
- a redaction-safe inconsistency error.

There is no public Store-level `RunAction` algebra. Process-facing Runtime APIs may still expose
stable lifecycle results needed by App, but those results describe Runtime progress, not Store
selection internals.

### 4.4 Direct-new provider-entry proof

Direct-new gating remains because it proves a real safety property:

```text
PreparedAccess<S, C>
  -> Journal encodes StatePrepared
  -> Store append
       Inserted  -> consume PreparedAccess -> CommittedCall<S, C> -> provider entry
       Existing  -> do not mint call; fold durable history
       Stale     -> do not mint call; reload and fold
       Unknown   -> preserve exact pending owner; do not mint call
```

Only Runtime can perform the promotion because Runtime owns the typed preparation. Store returns a
mechanical result; it does not construct `AccessPreparationOutcome` or another semantic owner.

### 4.5 Conclusion races

Runtime encodes its typed conclusion against an exact expected head. If append is stale or its
acknowledgement is ambiguous, Runtime reloads retained bytes, Journal qualifies them, and the same
Runtime reducer classifies the result:

- the identical append is present;
- a competing valid conclusion won;
- history is still at the expected head and the exact append may be retried; or
- retained history is inconsistent.

Store does not return `SelectedConclusion`. PostgreSQL's append receipt and retained rows contain
all mechanical evidence needed for Runtime to decide.

## 5. Target Journal design

### 5.1 One valid-by-construction codec boundary

Journal exposes two conceptual directions:

```text
trusted Runtime projection -> JournalFrame -> canonical bytes
stored bytes -> strict decode/qualification -> JournalHistory
```

`JournalFrame` and `JournalHistory` have private fields. There is no workflow in which callers
construct a partially checked public DTO, call `validate`, add more fields, and ask Store to check
it again.

Journal owns only intrinsic durable rules:

- canonical JSON and content hashing;
- float-free hashed structures;
- frame/object/per-run bounds;
- record-family shape;
- content-addressed object closure;
- run ID and dense frame sequence;
- recursive run head; and
- strict portable encoding/decoding.

Runtime owns all meaning that requires the admitted Program: current context, occurrence
reachability, State/capability contract association, preparation-to-conclusion legality, Match
selection, and terminality.

### 5.2 Lean record shapes

The target semantic shapes are:

```text
RunAdmitted {
  entry_point,
  program_ref,
  admitted_context_ref,
  configuration_ref,
  source_refs,
}

StatePrepared {
  occurrence,
  intent_ref,
  fact_selection,
}

StateConcluded::Pure {
  occurrence,
  outcome,
  fact_proposal_set_ref,
}

StateConcluded::Access {
  occurrence,
  preparation_sequence,
  evidence_ref,
  outcome,
  fact_proposal_set_ref,
}

RunFrame {
  run_id,
  run_sequence,
  record,
  object_closure,
}
```

Exact field names and whether an absent optional reference is omitted from canonical JSON remain
codec details. No scope, epoch, tenant, append request ID, publication sequence, record ordinal, or
duplicated Program-derived metadata is part of the semantic frame.

## 6. Target Store and PostgreSQL design

### 6.1 Store is a mechanical port

Store accepts already encoded Journal frames and returns retained bytes or append receipts. It may
enforce physical byte ceilings and SQL-safe integer ranges, but it does not inspect the Program or
reconstruct domain values.

The target dependency direction is:

```text
Program ----\
Capabilities --\
Facts ---------> Runtime -> Journal representation -> Store trait
Values --------/                                  -> PostgreSQL / Memory

Journal -> ids + canonical/value representation
Store   -> ids + Journal frame/storage representation
Replay/status -> Runtime's pure inspection path
```

In particular, `mfm-store` must stop depending on Program's runtime catalog, State semantics,
capability semantics, and Runtime selection types.

### 6.2 Global schema

The baseline PostgreSQL schema is replaced, not migrated through a dual model. Its logical keys are:

| Data | Key/constraint |
| --- | --- |
| run frames | primary key `(run_id, run_sequence)` |
| append idempotency | unique `(run_id, append_request_id)` |
| run heads | primary key `run_id` |
| fact publications | primary key global `publication_sequence`; unique `(run_id, run_sequence)` |
| fact head | one singleton row |
| configuration revisions | primary key global `revision_sequence`; unique `append_request_id` |
| configuration head | one singleton row |
| EVM nonce domains | `(sender_id, nonce_domain_id)` |
| EVM nonce operations | `(sender_id, nonce_domain_id, operation_key)` |

Run heads retain the exact recursive head and current sequence and add cumulative `total_bytes`, so
append does not sum all historical frames. Required conclusion reservation may be stored as physical
run metadata keyed by its preparation sequence. None of this metadata changes Program semantics.

Existing databases and portable histories are deliberately incompatible. Rewrite the pre-release
baseline migrations and fixtures. Do not ship an online legacy adapter, identity fallback, old
hash reader, or both key layouts.

### 6.3 Concurrency

Multiple Runtime processes may load and attempt the same next State. They race one exact run head:

1. Runtime folds the same retained prefix.
2. Each builds a candidate append against that head.
3. PostgreSQL locks/checks the head and unique append identity.
4. At most one non-identical successor is inserted.
5. Losers reload and Runtime folds the winning history.

No epoch elects a writer and no scope rotation invalidates old runs. A process can resume a run
only if its trusted Runtime assembly matches the Program and binding identities retained by that
run; otherwise it reports a typed incompatibility and does not write.

### 6.4 Fact publication without frame rebinding

The current conclusion protocol asks Store to obtain a fact publication coordinate, insert that
coordinate into the conclusion, rebuild the hashed frame, rotate/rebind append metadata, and retry
if the global fact frontier changed. This gives Store semantic mutation authority and makes global
concurrency alter canonical run bytes.

The target conclusion contains only the content-addressed fact proposal-set reference. PostgreSQL
allocates publication order in the same transaction:

```text
1. lock and compare the run head
2. for a non-empty proposal set, lock the singleton fact head
3. allocate publication_sequence = current + 1
4. insert the immutable run frame and its objects
5. insert (publication_sequence, run_id, run_sequence, proposal_set_ref)
6. advance run and fact heads
7. commit
8. return the assigned coordinate in AppendReceipt
```

An idempotent retry uses the same `(run_id, append_request_id)`, finds the same frame and fact row,
and returns the same receipt. Frame and publication either both commit or both roll back.

This deletes:

- `PreparedConclusion::bind_fact_publication` and rebind logic;
- fact-rebind ordinals and rotated append IDs;
- `FactFrontierChanged` conclusion retries;
- fact publication coordinates from Journal hashing;
- Store-side coordinate validation; and
- `fact_frontier` as an append precondition.

Runtime reads a structural fact snapshot and performs semantic fact selection. The selection records
the publication sequence through which it was made and the exact provenance of selected values.
Later publications do not invalidate it. With one database authority,
`FactSelectionFrontier.stream_ref` disappears; one `through_publication_sequence` is sufficient.

### 6.5 Configuration

Configuration uses one global append-only revision stream and singleton head. Store loads and
appends canonical revision bytes mechanically. Runtime/application composition qualifies bytes to
`C` once and obtains:

```text
ConfigurationRef {
  revision_sequence,
  content_ref,
}
```

Admission persists that immutable reference. A newer global revision does not make the admitted
configuration invalid and does not require Store to requalify it.

## 7. Target execution flows

### 7.1 New run

```text
typed domain C0
  -> Runtime constructs/retains ProvenValue<C0>
  -> Journal encodes RunAdmitted
  -> Store.append_run
  -> PostgreSQL CAS + durable receipt
  -> Runtime promotes its retained typed admission cursor
  -> Runtime reducer chooses the first State
```

Store never interprets `C0` or selects the first State.

### 7.2 Hot Pure State

```text
ExecutionCursor::ReadyPure<S>
  -> S evaluates ProvenValue<S::Input>
  -> ProposedStateOutcome<S::Output, S::Failure>
  -> Runtime retains the typed outcome
  -> Journal encodes StateConcluded::Pure
  -> Store/PostgreSQL append
  -> Runtime promotes the retained typed successor
  -> Runtime reducer chooses the next State or terminal result
```

There is no erase -> Store qualify -> `SelectedRun` -> Runtime downcast loop.

### 7.3 Hot Access State

```text
ExecutionCursor::ReadyAccess<S, C>
  -> typed preparation produces C::Intent and fact request
  -> Runtime selects facts from one immutable Store snapshot
  -> PreparedAccess<S, C>
  -> Journal encodes StatePrepared
  -> Store/PostgreSQL append
       Inserted -> Runtime mints CommittedCall<S, C>
       otherwise -> no provider-entry authority
  -> typed adapter/capability ingress produces C::Evidence
  -> typed State interpretation produces ProposedStateOutcome<S::Output, S::Failure>
  -> typed conclusion owner
  -> Journal encodes coordinate-free StateConcluded::Access
  -> PostgreSQL atomically appends frame and assigns fact publication coordinate
  -> Runtime promotes the retained typed successor
```

### 7.4 Cold resume

```text
Runtime.resume(run_id)
  -> Store.load_run(run_id)
  -> PostgreSQL returns stored rows/bytes
  -> Journal strictly decodes one JournalHistory
  -> Program decoder validates retained Program
  -> Runtime assembly associates stable declarations with concrete drivers
  -> Runtime reducer folds the prefix once
  -> selected driver reifies the exact current typed input once
  -> ExecutionCursor::Ready* / waiting / parked / terminal
```

Store returns no semantic selection. PostgreSQL knows no State types.

### 7.5 Callback-free replay/status

```text
portable or stored bytes
  -> Journal qualification
  -> Runtime pure reducer/inspection
  -> status, trace, terminal result, or inconsistency
```

The inspection path never invokes State or provider callbacks. It is the same semantic fold used by
cold resume, configured in inspect-only mode, not a second Store or replay reducer.

## 8. Identity deletion scope

The cutover removes `StoreScopeId`, `StoreEpoch`, and `TenantScopeId` completely from core and
persisted vocabulary:

- ID newtypes, parsers, serde, grammar variants, generators, fixtures, and tests;
- `StructuredStoreIdentity` and Store open/rotate APIs;
- Runtime sessions, `CommittedCall`, prepared/conclusion owners, and errors;
- `RunAdmitted`, `RunFrame`, hashes, logical keys, and portable envelopes;
- backend command/result DTOs and Memory keys;
- PostgreSQL constructors, active identity table, `rotate_identity`, transaction checks, columns,
  predicates, indexes, and composite keys;
- fact stream references and configuration partitioning;
- App's fixed-tenant facade and CLI/REST request or output fields;
- Program derivation/catalog inputs that include tenant identity; and
- EVM wallet nonce domains, effect-domain hashing, SQL rows, and operation keys.

The generic English word “scope” may remain where it describes a lexical or domain concept, but no
Store authority namespace or `StoreScopeId` equivalent may survive under a new name.

Removing tenant does not permit secrets or private user data to become cross-run facts. Secret-free
persisted-surface rules remain unchanged. Product isolation, if ever required, must be designed as
an explicit deployment/access-control boundary rather than smuggled back as an identifier in every
core type.

## 9. Package and API cutover

### 9.1 `mfm-program`

- retain the immutable `State | Match` document and compile-time typed State contracts;
- validate Program graph shape and stable contract continuity;
- remove erased value/capability execution callbacks and catalog branding;
- expose no general public erase/downcast workflow; and
- leave concrete implementation association to Runtime assembly.

### 9.2 `mfm-runtime`

- receive the sole reducer from Store;
- own hot execution, cold resume, semantic inspection, typed configuration, and fact selection;
- replace erased option bags with affine typed cursor states;
- own the sole private heterogeneous registry;
- retain direct-new `CommittedCall` gating; and
- return only process-facing lifecycle/results required by App.

### 9.3 `mfm-journal`

- own strict canonical frame/history and portable codecs;
- make constructed/decoded frames valid by construction;
- remove identity and redundant record fields;
- remove `FactPublication` from conclusions; and
- expose persisted representations, not semantic Runtime selections.

### 9.4 `mfm-store`

- reduce to the mechanical Store contract and Memory implementation;
- remove Program/capability/catalog/reducer dependencies;
- remove open brands, port splits, semantic owner types, and duplicated validators;
- return bytes, snapshots, and mechanical receipts only; and
- keep backend conformance focused on atomicity, CAS, idempotency, limits, and unknown ack.

### 9.5 PostgreSQL adapters

- implement Store directly rather than a second broad backend SPI;
- use the global schema and no active identity;
- allocate fact/configuration sequences transactionally;
- keep SQL row structs private; and
- perform no Program or capability validation.

### 9.6 App and transports

- remove tenant-scoped construction and request fields;
- generate or accept one global `RunId` for admission/retry;
- call Runtime for start, resume, status, and replay; and
- remain transport-only wrappers over typed entry points.

## 10. Complete deletion checklist

The implementation is not complete until the following are absent, except where this RFC explicitly
retains a sealed/private Runtime implementation detail:

```text
StoreScopeId
StoreEpoch
TenantScopeId
StructuredStoreIdentity
rotate_identity
active Store identity persistence

ReifyValue
ValueAssociation execution reifier
CapabilityAssociation erased bind callback
ProgramCatalog execution branding
ProgramCatalogBuilder as a second execution registry
catalog-bound ProgramIngress
qualify_retained_erased public flow
QualifiedTypedValue::erase public flow
QualifiedValue cross-layer flow

ErasedValue cross-layer alias
DynamicOutcome
DynamicPreparationFailure
DynamicResolution option bag
DynamicCommit
erased DynamicPrepared / DynamicCall result flow
DynamicPure / DynamicAccess adapter layer
TypedPrepared / TypedCall islands inside the erased protocol
PreparedExecution / OpenedPreparationCommit wrapper pair
QualifiedRecordedEvidence forwarding wrapper
RunSession.selected
public RunSession coordination owner
parallel SpawnStep / ResumeStep / RuntimeStep lifecycle algebras
App-owned SuspendedRun coordination

StoreBrand
StructuredStore
OpenedStructuredStore
OpenedStoreInner
MemoryStructuredBackend name/second-level backend role
StoreParts
QualifiedHistoryPort
HistoryReader semantic facade
ConfigurationStore semantic facade
StoreAuditPort semantic facade
QualifiedRun
RunReducer in Store
advance_selected in Store
retained_program in Store
replay_terminality in Store
validate_prefix public Store path
RunSelection
RunAction in Store
AccessActionMode in Store
SelectedRun
PreparedAdmission in Store
AdmissionOutcome in Store
PreparationAppend
AccessPreparationCandidate
AccessPreparationOutcome
PreparedConclusion in Store
SelectedConclusion
SelectedConclusionOutcome
SelectedConclusionPreparationOutcome
FactContinuation in Store
fact publication bind/rebind protocol

RawHistoryLoadLimit
RawFrameBytes public validation layer
RawRunPrefix public validation layer
RawConfigurationRevision public row DTO
RawFactPublication semantic layer
RawFactSnapshot semantic layer
BackendAppendCommand broad DTO
BackendAppendOutcome broad DTO
BackendConfigurationOutcome
BackendError / BackendFuture / BackendResult public aliases
StoreOpenError as a separate opening error layer
StoreWorkLimits as duplicate format/physical policy
separate AppendDisposition
StructuredStoreBackend broad SPI
public backend row/result aliases

PreparationMode
FactPublication in Journal conclusion
FactSelectionFrontier stream/head identity
separate FactCompleteness validity flag
ConfigurationHeadProjection
RecordLogicalKey persisted/public DTO
scope/epoch/tenant Journal fields
append_request_id in canonical RunFrame
duplicated preparation/conclusion fields listed in section 2.6
public repeated Journal validate choreography

ConfigurationWriteSession
PreparedConfigurationAppend
separate Store-branded SuspendedConfigurationAppend
ResolvedConfigurationHead Store brand
ResolvedConfiguration<C> Store qualification
ConfigurationCommitOutcome<C> Store proof
ConfigurationAppendCommand broad DTO
separate ConfigurationAppendDisposition

mfm-replay reducer/middle layer
scope/epoch/tenant portable fields
tenant-derived EVM nonce authority
```

The following small replacement vocabulary is expected:

```text
Runtime: ExecutionCursor and private typed step drivers
Journal: JournalFrame, JournalHistory, portable codec
Store: Store, RunAppend, AppendOutcome, AppendReceipt
Configuration: immutable ConfigurationRef plus one pending typed write owner
```

This list is a ceiling, not a requirement to create a public type for every name.

## 11. Security and failure semantics

The simplification preserves the existing high-risk invariants:

- run streams remain append-only;
- each append remains all-or-nothing;
- manifests, values, facts, outputs, and Journal objects remain content-addressed;
- structured hashes use canonical, float-free JSON;
- secrets never enter manifests, events, objects, facts, snapshots, outputs, or error details;
- provider IO occurs only through explicit adapters/capabilities;
- no provider entry occurs before a newly inserted matching preparation;
- an unknown acknowledgement never mints provider-entry authority;
- no successful result becomes public before its conclusion is durable or found identical; and
- replay performs no ambient IO or callback execution.

Removing scope/epoch/tenant removes no cryptographic or authorization boundary because those IDs do
not authenticate callers or encrypt rows. Deployment access control remains the responsibility of
the trusted embedding and PostgreSQL deployment boundary.

PostgreSQL corruption or hostile row mutation is still treated as untrusted byte ingress on load.
Journal and Runtime fail closed with redaction-safe typed errors. Process-local typed proof is not a
substitute for retained-data validation after a restart.

## 12. Tests and verification contract

### 12.1 Compile-time and unit proof tests

- typed Runtime output reaches Journal encoding without Store requalification;
- no supported API can pair an arbitrary contract ref with an unrelated value;
- the private heterogeneous dispatch rejects a mismatched retained type without panic;
- typed input, intent, evidence, and outcome remain correlated through preparation/conclusion;
- only `Inserted` can consume `PreparedAccess<S, C>` into `CommittedCall<S, C>`;
- `Existing`, `Stale`, and unknown acknowledgement cannot mint a call;
- unknown acknowledgement retains the exact affine pending owner; and
- hot advancement does not decode/reify Runtime's own just-produced output.

Use compile-fail tests where ownership, visibility, or consuming APIs are the proof mechanism.

### 12.2 Journal tests

- canonical fixtures for all three record families;
- exact-bound and bound-plus-one frame/object/history cases;
- non-canonical JSON, floats, hash mismatch, object collision, missing closure, sequence gap, and
  wrong recursive-head rejection;
- one validation pass per decoded history;
- no identity or fact-publication-coordinate fields in canonical output; and
- portable round trips through Journal only.

### 12.3 Runtime semantic tests

- Pure and Access hot paths preserve typed context;
- cold resume reifies retained values once and selects the same next State;
- `State | Match` sequential/fail-fast behavior;
- zero-state and terminal result rules;
- preparation/conclusion race classification after reload;
- callback-free inspection produces the same terminality as cold resume; and
- assembly mismatch fails before append.

### 12.4 Store conformance tests

Run identical Memory and PostgreSQL tests for:

- exact-head CAS;
- dense per-run sequence;
- per-run append-request idempotency;
- identical retry versus conflicting reuse;
- atomic frame/object/head/reservation writes;
- inserted/existing/stale/unknown outcomes;
- cumulative byte ceiling; and
- concurrent processes writing the same and different `RunId`s.

No conformance test may require a Program catalog or `SelectedRun`.

### 12.5 PostgreSQL fact/configuration tests

- concurrent conclusions allocate at most one dense global fact coordinate each;
- an identical retry returns the original coordinate;
- run frame and fact publication roll back together;
- later fact publications do not invalidate an earlier immutable selection;
- configuration revisions use one dense global head and idempotent append ID;
- admission remains bound to an older exact configuration revision after a newer one commits;
- database restart preserves all runs without identity activation/rotation; and
- no scope/epoch/tenant column, predicate, key, or hash input remains.

### 12.6 Cross-repository absence and complexity checks

The implementation report must include:

- production Rust LOC before and after;
- public exported types before and after;
- direct dependency edges removed from Store;
- `rg` evidence for deleted identity/erasure/middleman concepts;
- files required to add a new State or entry point; and
- confirmation that `backend.rs` was reduced by responsibility deletion, not merely split.

Production LOC must be net-negative. Tests and explicit boundary checks may not be deleted merely to
manufacture that result.

## 13. Ordered implementation commits

This RFC is documentation only. Implementation must be divided into coherent logical commits:

1. Simplify Journal records and introduce the one-valid-by-construction frame/history codec,
   updating all consumers in the same cutover.
2. Replace Store with the minimal mechanical API and update Memory conformance; delete Store
   branding, semantic port splits, raw validation wrappers, and public backend SPI.
3. Move reduction, cold qualification, semantic inspection/replay, typed configuration, and fact
   selection into Runtime; delete Store `SelectedRun` and its entire selection algebra.
4. Cut Runtime execution over to its affine typed cursor and one private heterogeneous driver;
   delete the cross-layer erased outcome/preparation/conclusion path and duplicate catalog
   callbacks.
5. Make PostgreSQL assign coordinate-free fact publications atomically with conclusion append;
   delete fact frontier/rebind/retry layers.
6. Rewrite the PostgreSQL, Memory, Journal, portable, configuration, and EVM nonce schemas without
   scope, epoch, or tenant, updating all callers in the same breaking change.
7. Move portable encoding to Journal, switch all status/replay callers to Runtime inspection, and
   delete `mfm-replay`.
8. Delete every superseded API, dependency, fixture, test helper, and compatibility path; update
   `docs/design.md`, `docs/architecture.md`, crate READMEs, and CLI/REST documentation.
9. Run scope-selected verification from `docs/build-and-verification.md`, then run the composed CI
   gate once after narrower failures are resolved.

If a public cutover cannot compile halfway, its deletion and replacement belong in the same commit.
Do not temporarily maintain an erased and typed execution route or an identity and identity-free
schema in parallel.

## 14. Rejected alternatives

### Split `backend.rs` without changing ownership

Rejected. It improves navigation while retaining every validation, DTO, dependency, and future
change site that caused the size.

### Keep Store reduction but rename `SelectedRun`

Rejected. Runtime would still execute a persistence layer's semantic decision, and replay would
still depend on Store's Program interpretation.

### Remove all erasure

Rejected. A data-selected heterogeneous Program needs one existential dispatch boundary in stable
Rust. Pretending otherwise would produce a larger generated enum, unsafe casts, or a closed set of
States. The correct target is one private, correlated erasure point.

### Keep `QualifiedValue` public for convenience

Rejected. It makes value/type/contract correlation reconstructible by convention and invites new
Store/App DTOs to use erased values as a common currency.

### Make Journal validate Program semantics

Rejected. Journal would depend on Runtime/Program execution meaning and become a second reducer.
Journal validates representation; Runtime validates execution semantics.

### Let Store return a typed next action

Rejected. Store cannot construct that proof without depending on Program catalogs and State
registries. It should return retained bytes and mechanical write outcomes.

### Retain scope/epoch as harmless metadata

Rejected. Metadata that participates in schemas, hashes, keys, errors, and every API is an active
namespace and lifecycle policy. It is not free, and the platform has no corresponding authority.

### Retain tenant only for future isolation

Rejected. A dormant tenant ID does not provide access control or deployment isolation. A future
multi-authority product must define an explicit boundary and migration rather than tax every
current core path.

### Put the PostgreSQL fact coordinate in `StateConcluded`

Rejected. It forces a persistence-assigned value back into semantic canonical bytes, requires Store
to rebuild typed output, and creates avoidable frontier/rebind retries. The proposal set is
semantic; its global publication position is physical index metadata.

### Keep `mfm-replay` as a second semantic reducer

Rejected. Replay and cold resume answer the same deterministic question. Journal owns portable
bytes and Runtime owns the one callback-free fold.

## 15. Acceptance criteria

The architecture is complete when all of the following are true:

1. Runtime is the only layer that evaluates `State | Match` and determines the next State.
2. Store and PostgreSQL have no dependency on Program catalogs, State/capability semantics,
   `QualifiedValue`, `RunAction`, or `SelectedRun`.
3. One private Runtime registry handoff is the only execution-layer type erasure.
4. A hot typed value is encoded once and never requalified by Store.
5. A cold retained history is structurally qualified once by Journal and semantically folded once
   by Runtime per process proof lifetime.
6. Journal frames are valid by construction and have no public repeated validation choreography.
7. Store exposes only mechanical persistence operations and outcomes.
8. PostgreSQL atomically appends run frames and global fact publication rows without modifying
   canonical conclusion bytes.
9. Direct-new typed gating remains the only path to `CommittedCall<S, C>`.
10. `StoreScopeId`, `StoreEpoch`, `TenantScopeId`, and equivalent persisted namespaces are absent
    from Rust APIs, JSON, hashes, SQL, transports, facts, configuration, replay, and EVM nonce keys.
11. `RunId` alone identifies a run within the database authority, and concurrent processes
    linearize through exact-head CAS.
12. Configuration and facts each use one global append-only PostgreSQL sequence.
13. Portable decoding lives in Journal and semantic replay/status uses Runtime's sole reducer.
14. The complete deletion checklist has no compatibility aliases or forwarding wrappers.
15. Authoritative design/architecture documents, fixtures, READMEs, and transport schemas describe
    only the new design.
16. Backend conformance, Runtime semantic, Journal canonical, PostgreSQL transaction, ownership,
    and absence tests pass under the repository's scope-selected verification policy.
17. Production Rust LOC and public coordination surface are net smaller than before the cutover.

The intended end state can be summarized in one sentence:

> Runtime decides and retains typed meaning, Journal proves bytes, and PostgreSQL proves atomic
> persistence; no layer erases another layer's proof merely to reconstruct it later.
