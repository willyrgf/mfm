# RFC: establish the core Runtime, Journal, and Store proof path

Status: selected first implementation target; product and proof-capacity ratifications below remain pending

Relationship: this is the first core cutover after `RFC_REFACTOR_SINGLE_TRUST_BOUNDARY.md`. It is
the sole owner of Runtime reduction, typed execution entry and progression, acknowledgement-owner
fate, `RunView`, Journal qualification, mechanical Store persistence, fact append correlation,
configuration persistence, and replay inspection. It incorporates and supersedes former Sections 5.2--5.8
and 10.2--10.3 of `RFC_FOLLOWUPS_FROM_REFACT_RUNTIME.md`; that RFC remains responsible for later
Operation authoring, deterministic authoring/composition-time capability injection, domain naming,
Application entry-point registration, and transport work and must consume this core contract rather
than restate it. Runtime capability drivers, fact/evidence qualification, and occurrence-specific
adapter association belong exclusively to this core RFC.

When implemented, this RFC deliberately replaces the conflicting Runtime/Store/Journal ownership,
erased-value flow, Store identity model, fact-publication protocol, configuration persistence
boundary, and replay layering in `RFC_REFACTOR_SINGLE_TRUST_BOUNDARY.md`, `docs/design.md`, and
`docs/architecture.md`. Until then, the authoritative documents continue to describe the current
implementation. Each implementation commit must update its affected authoritative contracts in the
same cutover. The result leaves one current design, with no compatibility facade, legacy decoder,
dual schema, fallback identity, parallel reducer, or second Runtime progression path.

---

## Decision

MFM will use strong Rust types as process-local proof that a value has already crossed its trust
boundary. A fact is checked where it first becomes untrusted, represented by a type with private
fields, and then carried without erasing and reconstructing that proof at every crate boundary.

The execution and persistence path has four owners:

| Owner | Sole responsibility |
| --- | --- |
| Program | The immutable, deterministic State-or-Match graph and its declared contracts |
| Runtime | Program reduction, next-State selection, exact registered State entry, typed State/capability execution, bounded process-facing progression, acknowledgement-owner fate, `RunView`, typed configuration, fact selection, hot advancement, cold resume, and semantic replay |
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
derives the next action and the callback-free public view of retained progress.

There is exactly one unavoidable execution-layer erasure boundary: Runtime's private heterogeneous
State registry. A retained Program selects implementations at runtime, while different States have
different `Input`, `Output`, `Failure`, capability `Intent`, and capability `Evidence` types. Rust
cannot represent that data-selected heterogeneous graph as one static generic chain. The erased
value therefore exists only long enough for one exact `RegisteredState.start` function to recover
its concrete State proof. The registration is keyed by the complete persisted execution
association, never by implementation identity alone. The erased value never crosses into Store,
Journal, PostgreSQL, App, replay inspection, or an adapter.

Runtime exposes one high-level start/resume/read contract. Start and resume share one bounded
`advance_until_stable` implementation and return the same Runtime-derived `RunView` as read when a
qualified durable view exists. App and transports never coordinate `ExecutionCursor`, typed step
owners, pending append owners, or State-by-State lifecycle variants.

This is not a claim that all runtime validation can disappear. Persisted bytes, provider responses,
transport input, and concurrent database state are untrusted. They must still be checked. The rule
is narrower and enforceable:

> Validate when trust changes; preserve the resulting proof; do not erase it merely to validate the
> same proposition again in another internal layer.

## Material uncertainties

The layer ownership, one private erasure boundary, Store selection deletion, global PostgreSQL
authority, removal of scope/epoch/tenant, private Runtime acknowledgement custody, and requirement
for compatible semantic registrations during inspection are not uncertain. Five product semantics
and one persisted proof-capacity ceiling must be ratified before implementation begins. The target
sections below use the recommended product choices so the RFC remains one concrete design rather
than a menu.

1. **Durable Access preparation recovery.** Recommended choice: a retained preparation never
   recreates its original `CommittedCall`. A Read may append a fresh direct-new replacement only
   within its declared replacement bound; an entry-once Effect parks after process-owner loss unless
   a separately proven absorption contract permits a new attempt. It is uncertain whether the
   product instead requires automatic recovery of the original attempt. If so, a durable
   outbox/lease/claim protocol is required and materially changes Runtime and PostgreSQL. Resolve
   by documenting each current Read/Effect availability guarantee and proving any recoverable
   Effect's absorption/idempotency contract.
2. **Configuration currentness.** Recommended choice: bind admission to the exact immutable
   revision used to construct `C0`, without requiring it to remain latest at commit. It is uncertain
   whether an entry point has a business rule requiring latest-at-admission. If it does,
   configuration-head CAS must join a genuinely new admission transaction, while a retry of an
   already admitted `RunId` still needs its original immutable revision. Resolve by auditing the
   current entry-point contracts and recording whether either promises latest-at-commit semantics
   and how retries recover the admitted revision.
3. **Fact-selection freshness.** Recommended choice: bind selection to the complete immutable
   snapshot Runtime captured; later publications do not invalidate preparation. It is uncertain
   whether a capability requires every fact committed before its preparation append. If it does, a
   fact-head CAS and preparation retry remain necessary, although Store still performs no semantic
   selection. Resolve by documenting every current fact consumer's freshness contract and adding a
   concurrent-publication fixture.
4. **Global `RunId` creation and repetition.** Recommended choice: `RunId` is the database-global
   semantic admission/idempotency identity and every independent repetition uses a new `RunId`.
   It is uncertain whether current entry points rely on deriving the same run from only entry point
   plus application selector, and whether a retry after configuration/authoring change must reuse
   the exact admitted genesis rather than conflict with it. If they do, removing tenant may collapse
   intended independent runs or make ordinary retries conflict. Resolve by inventorying current
   derivation/caller retry behavior and choosing explicit `RunId` plus exact retry inputs, an
   authenticated existing-admission lookup, or one explicit domain idempotency key—never another
   ambient namespace.
5. **Fact dependency proof ceilings.** Ownership and immutability of
   `MAX_FACT_DEPENDENCY_NODES`, `MAX_FACT_DEPENDENCY_DEPTH`, and
   `MAX_FACT_DEPENDENCY_BYTES` are settled, but their numeric values are not yet supported by
   representative nested-producer measurements. Values that are too small reject legitimate
   histories; values that are too large make the replay resource guarantee impractical. Resolve by
   building the maximum current and adversarial nested-producer fixtures during implementation
   planning, recording their proof sizes, and freezing reviewed values in the clean-slate persisted
   format before code implementation begins.
6. **Configuration instance namespace.** Recommended choice: latest configuration is partitioned
   by one explicit stable `ConfigurationInstanceId` plus exact contract/schema, never by ambient
   tenant or by Rust type alone. It is uncertain whether the product intentionally permits only one
   database-global series for each configuration contract. If that assumption is wrong, networks,
   accounts, or entry points using the same `C` would overwrite one another's notion of latest.
   Resolve by inventorying current EVM/Portfolio configuration cardinality and assigning each
   independently current series an explicit stable instance identity before freezing its codec.

Two consequences of settled architecture are explicit: PostgreSQL's global dense
fact-publication sequence is physical index metadata rather than conclusion content, and an
in-place database restore requires every old Runtime/Store peer to stop before the restored
authority starts because epoch fencing is removed.

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

### 1.7 Runtime lifecycle leaks into Application

Runtime currently exposes parallel `SpawnStep`, `ResumeStep`, and `RuntimeStep` algebras plus
`RunSession`, `SuspendedRun`, and Store-selected owners. Application consequently acts as a process
supervisor: it drives one State at a time, retains suspended owners, resolves acknowledgement
boundaries, inspects frames to infer status, and translates internal lifecycle distinctions into
public results.

That is not an Application responsibility. Runtime already owns the typed input, adapter entry,
append candidate, retained Program, and recovery classification needed to advance safely. The core
must therefore expose one bounded process-facing progression path and one callback-free durable
view while keeping every affine execution and acknowledgement owner private. Generic entry-point
registration and transport rendering remain later Application work; the core API they consume is
settled here.

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
| `DynamicStateRegistration` | Erased State dispatch and qualification callbacks | Replace with one exact-key `RegisteredState` and monomorphic `StateStart` whose concrete implementation immediately enters `S`/`C` typed code |
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
| `PendingConclusion` | Retain only as one private typed pending-append variant until the physical and semantic append fate is exhaustive |
| `SuspendedRun` | Delete as a public coordination object; private acknowledgement custody is an implementation state in Runtime's bounded pending-owner table |
| `ParkedRun` / `ParkReason` | Project to one read-only process-facing status, not a Store-selected owner |
| `TerminalRun` | Project the typed durable result/failure; do not carry Store selection internals |
| `SpawnStep` / `ResumeStep` / `RuntimeStep` | Collapse to one high-level Runtime progression result; start and resume do not expose parallel lifecycle algebras |
| `AdmissionFailure` / `AdmissionConflict` / `ResumeFailure` | Collapse overlapping transport-shaped classifications into one typed Runtime error/progress contract |
| `RuntimeLimits` | Retain Runtime work/concurrency policy, including nonzero State-start and pending-owner bounds; do not duplicate Journal format or Store physical limits |

App asks Runtime to start, resume, or inspect a run and receives `RunView` or one reviewed Runtime
error. Runtime internally drains immediately actionable sequential States until a stable external
boundary. App does not hold `RunSession`, match `SpawnStep` versus `ResumeStep`, or shuttle Store
owners back into Runtime. Unknown acknowledgement authority is retained, resolved, and bounded
inside Runtime; it never becomes an Application token.

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
| `RawFactPublication` / `RawFactSnapshot` | Replace with one bounded authenticated route-index proof; Runtime owns completeness correlation and `FactSelection` semantics |

The replacement is intentionally small:

```text
Store
  load_run(run_id, target: Current | Exact { sequence, digest }, limits)
    -> Result<Absent | Present(StoredRunBytes), StoreError>
  append_run(run_id, &RunAppend) -> Result<AppendOutcome, StoreError>
  list_run_ids(&PageRequest)
    -> Result<BoundedRunIdPage, StoreError>
  load_fact_snapshot(&FactSnapshotQuery)
    -> Result<FactIndexProof, StoreError>
  load_fact_log_consistency(earlier: FactHead, later: FactHead)
    -> Result<FactLogConsistencyProof, StoreError>
  load_configuration_head
    -> Result<Absent | Present(ConfigurationHead), StoreError>
  load_latest_configuration(routing_key, through: Absent | Present(ConfigurationHead))
    -> Result<Absent | Present(ConfigurationRef), StoreError>
  load_configuration(ConfigurationRef)
    -> Result<StoredConfigurationRevision, StoreError>
  append_configuration(&ConfigurationAppend)
    -> Result<ConfigurationAppendOutcome, StoreError>
  check_ready -> Result<(), StoreError>
```

The normative command/result shape is:

```text
RunAppend {
  expected_head: Absent | Present { sequence, digest },
  append_request_id,
  frame: EncodedRunFrame,
  reservation_instruction: None | Open | Replace | Consume,
}

EncodedRunFrame = opaque Journal-owned frame/object bytes
                + None | NonEmptyFactPublicationAttachment

AppendOutcome {
  Inserted(AppendReceipt),
  Existing(AppendReceipt),
  Stale { actual_head },
  AcknowledgementUnknown,
}

AppendReceipt {
  run_sequence,
  run_head,
  optional_fact_publication: None | Present {
    publication_sequence,
    resulting_fact_head,
  },
}

FactHead {
  publication_sequence,
  index_root_digest,
  log_root_digest,
}

FactPublicationHeader {
  publication_sequence,
  previous_fact_head,
  producer_run_id,
  producer_run_sequence,
  producer_run_head,
  proposal_set_ref,
  route_update_set_digest,
  resulting_index_root_digest,
  descriptor_digest,
  resulting_fact_head,
}

FactRouteKey {
  source_ref,
  subject_ref,
  digest: H(fact-route-domain, canonical(source_ref, subject_ref)),
}

FactSnapshotQuery {
  bounded_sorted_unique_route_keys,
  through: CaptureCurrent | Exact(FactHead),
}

FactIndexLeaf {
  route_key,
  publication_sequence,
  producer_run_id,
  producer_run_sequence,
  producer_run_head,
  proposal_set_ref,
  proposal_ordinal,
  value_ref,
}

FactIndexProof {
  through: FactHead,
  ordered_results: Absent(route_key) | Present(FactIndexLeaf),
  canonical_sparse_merkle_multiproof,
  frontier_header_witness: Genesis | Published { header, log_inclusion_witness },
  deduplicated_publication_header_inclusion_witnesses,
}

FactLogConsistencyProof {
  earlier: FactHead,
  later: FactHead,
  canonical_append_only_log_consistency_witness,
}

ConfigurationInstanceId = explicit stable domain/composition identity

ConfigurationRoutingKey {
  instance_id: ConfigurationInstanceId,
  contract_ref,
  schema_ref,
  digest,
}

ConfigurationHead {
  revision_sequence,
  head_digest,
}

ConfigurationRevisionHeader {
  revision_sequence,
  previous_head_digest,
  envelope_ref,
  routing_key,
  head_digest,
}

ConfigurationRef {
  revision_sequence,
  envelope_ref,
  revision_head_digest,
}

ConfigurationAppend {
  expected_head: Absent | Present(ConfigurationHead),
  append_request_id,
  envelope: EncodedConfigurationEnvelope,
}

ConfigurationAppendOutcome {
  Inserted(ConfigurationAppendReceipt),
  Existing(ConfigurationAppendReceipt),
  Stale { actual_head },
  AcknowledgementUnknown,
}

ConfigurationAppendReceipt {
  configuration_head: ConfigurationHead,
  configuration_ref: ConfigurationRef,
}

StoreError = InvalidPhysicalCommand
           | IdempotencyConflict
           | Capacity
           | CorruptPhysicalState
           | UnavailableBeforeSubmission
```

Exact Rust spelling and async-trait mechanics are not normative. The operations, correlations,
outcomes, and error fates above are normative. Append commands are borrowed so Runtime retains the
affine owner required for post-ack promotion. `EncodedRunFrame` has private fields and belongs to
Journal. Its sealed publication attachment contains the all-and-only route-update templates derived
from the same proposal-set object: exact route key, proposal ordinal, and value reference, with the
producer and publication coordinates left for the transaction. A proposal set may contain at most
one fact for a `(source_ref, subject_ref)` route. Thus no caller can pair frame bytes with independent
fact metadata. `RunAppend` is Store's physical command and adds Runtime's reservation instruction
beside, not inside, the Journal proof. Store consumes both mechanically and interprets no Program,
State, capability, contract, evidence, reservation-key, routing-key, or fact meaning.

`EncodedConfigurationEnvelope` likewise belongs to Journal. It exposes only the bounded opaque
instance/contract/schema routing key and physical references Store needs to index and correlate its rows;
Store never decodes `C` or constructs a semantic configuration proof.

The routing key also commits to an explicit `ConfigurationInstanceId`. Composition supplies that
stable identity for the independently current network/account/entry-point series; it is not inferred
from process configuration, tenant, or Rust `TypeId`. Runtime derives and checks the complete key
under the exact `C` registration, Journal seals it into the envelope/revision header, and Store
compares/indexes it opaquely.

Every `FactPublicationHeader` is canonical physical proof material. Its descriptor digest commits
to its sequence, complete previous `FactHead`, producer run/frame/head, proposal-set reference, the
complete sealed route-update-set digest, and the resulting authenticated index root. That descriptor
is one leaf in a domain-separated append-only Merkle log; `resulting_fact_head` contains the new log
root and index root without creating a hash cycle. The empty fact stream has one specified log root
and one specified sparse-tree root. Journal owns the route-key, leaf, node, header, inclusion/
consistency/multiproof codecs, and both pure transition algorithms; Store supplies only
transaction-assigned coordinates and persists the resulting opaque structures. The append-only log
remains available for ordering/audit and logarithmic branch-consistency proof, but fact selection
never scans it.

The route index is a versioned 256-level sparse Merkle map using the current format's SHA-256. A
domain-separated `FactRouteKey`
commits to the complete canonical `(source_ref, subject_ref)`, and its leaf names the latest
publication for that exact route. The path consumes digest bits most-significant first. Journal
fixes domain-separated leaf/branch hashes (including branch depth), one empty-subtree hash per depth,
and a sorted minimal multiproof with no duplicate, unused, or alternative-default node encoding.
The append-only log uses the Certificate-Transparency largest-power-of-two split rule with separate
empty/leaf/branch domains and canonical minimal inclusion/consistency witnesses. These are Journal
format constants. A
membership or non-membership multiproof against the retained `FactHead.index_root_digest`, plus
inclusion witnesses for its frontier header and every returned publication, detects an omitted,
substituted, or cross-branch requested route without work proportional to database age. Log
consistency witnesses prove that an earlier dependency frontier is a prefix of the later producer
frontier rather than merely having a smaller sequence. These proofs establish consistency with the
retained MFM roots; they do not add an external signature or Byzantine database-authorship claim.

The hash recurrence is normative. `H` is SHA-256 and `JCS` is the repository's canonical,
float-free object encoding. Every object below includes the shown literal domain before its fields:

```text
route_digest = H(JCS { domain: "mfm.fact.route.v1", source_ref, subject_ref })

E[256] = H(JCS { domain: "mfm.fact.index.empty.v1" })
E[d]   = H(JCS { domain: "mfm.fact.index.branch.v1", depth: d,
                  left: E[d + 1], right: E[d + 1] })
leaf    = H(JCS { domain: "mfm.fact.index.leaf.v1", depth: 256,
                  route_digest, canonical_leaf })
branch  = H(JCS { domain: "mfm.fact.index.branch.v1", depth, left, right })

route_update_set_digest = H(JCS {
  domain: "mfm.fact.route-updates.v1",
  updates: sorted-unique-by-route-digest [
    { route_key: { source_ref, subject_ref, digest }, proposal_ordinal, value_ref }
  ]
})
descriptor_digest = H(JCS {
  domain: "mfm.fact.publication.v1",
  publication_sequence,
  previous_fact_head,
  producer_run_id,
  producer_run_sequence,
  producer_run_head,
  proposal_set_ref,
  route_update_set_digest,
  resulting_index_root_digest
})
log_empty = H(JCS { domain: "mfm.fact.log.empty.v1" })
log_leaf  = H(JCS { domain: "mfm.fact.log.leaf.v1", descriptor_digest })
log_node  = H(JCS { domain: "mfm.fact.log.branch.v1", left, right })
```

The zero head is exactly `{ publication_sequence: 0, index_root_digest: E[0],
log_root_digest: log_empty }`. At nonzero sequence the CT split recurrence over `log_leaf` values
produces `log_root_digest`; sparse batch replacement produces `index_root_digest`; and the header's
`resulting_fact_head` contains exactly those roots and sequence. The genesis
`frontier_header_witness` is valid only for that zero head and every result must be `Absent` under
the empty-root multiproof. A nonzero head requires the exact last publication header and its log
inclusion witness. Golden fixtures freeze the integer/string representation and proof-node order;
no implementation-defined concatenation is permitted.

`Stale` and every `StoreError` prove that this call did not commit. Once a backend may have
submitted the transaction, every transport loss, cancellation, timeout, or indeterminate driver
error is `AcknowledgementUnknown`; it must never be downgraded to `UnavailableBeforeSubmission`.
`Inserted` and `Existing` contain an immutable receipt. `Existing` requires exact command identity.
All loads, lists, and proofs have explicit nonzero row/byte/proof bounds and return capacity rather
than truncating a value that their result type labels complete.

Runtime derives reservation bounds from `ExecutableProgram` before Access provider entry. An
`Open` instruction reserves the maximum conclusion charge under an opaque preparation key; a Read
`Replace` atomically transfers an old key to a new key; and `Consume` requires the exact active key,
checks the sealed conclusion charge against that key's reservation, and releases it with the
conclusion append.

`MAX_RUN_BYTES` is an immutable constant of the current Journal/storage format. It is identical in
Memory and PostgreSQL, survives restart unchanged, and can change only in another clean-slate
persisted-contract cutover; it is not a mutable Store-opening option. Under the same run-head CAS,
Store charges every frame byte plus each object on its first reference by that run and enforces:

```text
new_total_bytes    = old_total_bytes + newly_charged_bytes
new_reserved_bytes = old_reserved_bytes - released_reservation + opened_reservation
new_total_bytes + new_reserved_bytes <= MAX_RUN_BYTES
```

Charging is logical and per-run. Global physical blob deduplication may save disk but never changes
which bytes count against a run: `(run_id, object_ref)` membership determines first-reference
charging. Active reservation rows and their aggregate are durable run metadata, so restart cannot
forget liability created by `Open` or `Replace`.

Reservation keys are recoverable physical identities, not random process tokens:

```text
ReservationKey = H(JCS {
  domain: "mfm.run.conclusion-reservation.v1",
  run_id,
  preparation_sequence
})
```

Runtime knows a new preparation's sequence as `expected_head.sequence + 1`; cold history supplies
the prior preparation sequence named by a replacement or conclusion. `Open` uses the new sequence,
`Replace` uses the retained old and deterministic new keys, and `Consume` uses the exact preparation
sequence referenced by the conclusion. Thus cold Read replacement can name the durable liability
without loading semantic reservation rows or persisting a random key in Journal history. Store
validates only this target/sequence/key correlation and its active-row arithmetic; it still
interprets no occurrence or capability meaning. Canonical goldens freeze the derivation.

| Instruction | Required atomic transition |
| --- | --- |
| `None` | Charge the admission/Pure frame and objects; change no reservation |
| `Open { key, maximum }` | Require absent key, charge preparation, and add `maximum` |
| `Replace { old, new, maximum }` | Require active `old`, charge replacement, release `old`, and add `new` |
| `Consume { key }` | Require active key, require actual conclusion charge not greater than its maximum, charge conclusion, and release key |

Stale or definite-error attempts mutate neither accounting value nor reservation rows. An exact
retry returns its original receipt without applying the transition twice. A late old conclusion
cannot consume a replacement key. Store treats keys and arithmetic mechanically; only Runtime
knows what occurrence they represent.

For idempotency, Store first looks up `(run_id, append_request_id)`. An existing row returns its
original receipt only when expected head, complete frame bytes and object closure, reservation
instruction, and sealed publication attachment are identical; conflicting reuse fails. Only a new
request proceeds to the exact-head comparison. This ordering makes an identical retry succeed even
after its original append advanced the head.

That ordering is serialized, not a racy read-before-lock optimization. Before the first lookup,
Store acquires transaction-scoped serialization for the complete request key; after it waits, it
looks up the request row again. A new request then acquires transaction-scoped serialization for
the target run-head key, including an absent head, before comparing the expected head. PostgreSQL
may implement these two keyed critical sections with transaction advisory locks or an equivalent
unique-claim protocol; Memory uses an equivalent keyed lock. The choice is private, but two
overlapping identical request IDs must end as `Inserted` then `Existing`, never `Inserted` then
`Stale`. A conflicting reuse that waited for the winner must observe the committed row and return
`IdempotencyConflict`. No incomplete request row may become visible or survive rollback.

The append-request row atomically persists a canonical command digest that commits to the target,
expected head, complete encoded frame/object closure and publication attachment, reservation
instruction, plus the immutable receipt. Configuration append-request rows likewise commit to
expected configuration head and the complete sealed envelope plus their receipt. Store recomputes
and compares that digest before returning `Existing`; the unique request ID alone is insufficient.
A structurally valid stored command with a different digest is `IdempotencyConflict`; an internally
inconsistent digest, receipt, or referenced row is `CorruptPhysicalState` and never masquerades as
caller conflict.

These identities are not interchangeable. `RunId` is the semantic admission identity: the same
`RunId` with byte-identical genesis is idempotent even under another physical append request, while
a different genesis is `AdmissionConflict`. `append_request_id` identifies one physical CAS and
acknowledgement attempt; reusing it requires complete command equality. A new independent domain
repetition always uses a new `RunId`.

The borrowed command lets Runtime retain every affine semantic owner while Store performs IO.
`load_run(Current)` atomically captures one real run head; `load_run(Exact { sequence, digest })`
verifies that historical head and captures exactly through it. Either form returns every frame and
referenced object through its captured head. It returns `Capacity`, never a truncated object
labeled as a complete run, when the configured nonzero frame/byte bounds are insufficient. The
bytes carry no semantic validity.
A run append is accepted only when the opaque Journal projection's `run_id`, next sequence, and
`previous_head` agree with the command target and expected head; this is mechanical correlation,
not Program reduction. Bounded run-ID pages use stable keyset continuation and report whether that
page's statement snapshot contains another greater key. This is an operational weak listing, not a
transaction-spanning snapshot: a concurrent insertion may appear on a later page or, if its key
sorts behind the cursor, be absent from that enumeration. No semantic proof, replay, or recovery
decision relies on list completeness.

Runtime derives a bounded sorted/unique `FactSnapshotQuery` containing one exact `FactRouteKey` for
each admitted source and requested subject. Its frontier is either `CaptureCurrent` for a new
selection or the complete retained `FactHead` when validating history. `load_fact_snapshot`
captures or verifies that immutable head and returns exactly one ordered membership/non-membership
result per query key, one canonical sparse-tree multiproof, the frontier-header inclusion witness,
and deduplicated inclusion witnesses for the present leaves' publications. The proof size is bounded
by `MAX_FACT_SOURCES` times the fixed index/log depths (the log sequence is a bounded integer),
independent of global publication count; a response
is complete or fails, never paged/truncated or silently substituted with an older leaf. Newer
publications create a new immutable root and do not disturb a captured proof. Historical roots and
the immutable nodes reachable from them remain retained for the lifetime of this clean-slate format;
missing reachable nodes are corruption, not absence. Portable inspection carries the same bounded
head/proof bytes. Dependency bundles additionally carry canonical logarithmic log-consistency
witnesses for every distinct earlier-frontier to later-frontier edge.

Store validates SQL/Memory row ranges, ordering, dense physical heads, exact head/query/result
correlation, duplicate keys, reservation-row transitions, configuration routing indexes, command
digests, and receipt identity for non-Journal rows. The Journal-sealed publication attachment
projects the opaque route updates from the same proposal objects so Store can persist/query the
authenticated index without interpreting them. Journal validates canonical
run/configuration/object and proof bytes, and Runtime validates
all Program, source-manifest, fact, capability, and typed configuration meaning.

`ConfigurationHead` is physical global-CAS state. `ConfigurationRevisionHeader` is the immutable
Journal-qualified structural link between one assigned sequence, predecessor digest, opaque routing
key, envelope reference, and revision head digest. `ConfigurationRef` identifies that exact
revision by sequence, envelope reference, and revision head digest and is the only configuration
identity retained by admission. Load by the full `ConfigurationRef` returns its header plus canonical
revision-envelope bytes and rejects any sequence, head-digest, routing, or envelope-reference
mismatch. Latest-by-instance-and-contract lookup takes an opaque Journal-projected routing key and
a captured global head, and returns the greatest matching revision at or below that head. Its
append outcome mirrors run idempotency: inserted or
exact existing returns one immutable receipt, a stale head returns the actual physical head,
acknowledgement may be unknown, and conflicting append-ID reuse is a typed Store error. Store never
returns a decoded `C`.

### 2.5 Configuration middlemen

| Current concept | Target disposition |
| --- | --- |
| `ConfigurationStore` | Delete the separate semantic Store facade |
| `ResolvedConfigurationHead` | Split physical `ConfigurationHead` CAS state from immutable typed `ConfigurationRef` |
| `ResolvedConfiguration<C>` | Replace with one Runtime-owned typed proof carrying `C` and its immutable `ConfigurationRef` |
| `ConfigurationWriteSession<C>` | Replace with one Runtime-owned precommit `PendingConfiguration<C>` |
| `PreparedConfigurationAppend<C>` | Collapse into that pending owner |
| `SuspendedConfigurationAppend<C>` | Retain unknown-ack ownership only as a state of that pending owner, not another Store layer |
| Store brand/global-head fields in configuration proofs | Delete |

Configuration persistence is global. A revision has one global sequence and content reference.
Admission records the exact immutable revision it used. Store handles bytes and configuration-head
CAS; Journal owns the canonical revision-envelope codec; Runtime is the sole typed configuration
ingress and decodes or constructs `C` once. Trusted application composition may retain and borrow
the Runtime-issued proof but cannot decode retained configuration bytes, fabricate a reference, or
create a second configuration registry.

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
| `ConfigurationHeadProjection` | Replace with physical `ConfigurationHead` for CAS plus immutable `ConfigurationRef { revision_sequence, envelope_ref, revision_head_digest }` for admission |
| `PreparationMode` | Delete; mode is declared by the admitted State/capability |
| duplicated prepared input | Delete; Runtime derives it from the predecessor context |
| preparation ordinal/replacement ref | Delete; one frame has one record and its sequence is the preparation identity |
| duplicated execution-binding ref | Delete; the admitted Program pins it |
| duplicated fact request | Delete; retain the exact selected facts used by the attempt |
| persisted maximum-conclusion bytes | Remove from semantic Journal bytes; keep any required reservation as physical append metadata |
| `PreparationRef { run_id, run_sequence, record_ordinal }` | Reduce to preparation frame sequence; containing run supplies `RunId` |
| duplicated fact selection in `StateConcluded` | Delete; Access conclusion refers to its preparation |
| `FactPublication` in `StateConcluded` | Delete; PostgreSQL publication index owns the coordinate |
| `FactSelectionFrontier.stream_ref` / proposal `.head_ref` | Delete the redundant stream ID; retain the exact complete global `FactHead` in `FactSelection` |
| `FactCompleteness { through_sequence, complete }` | Delete the always-true validity flag; a valid `FactSelection` carries its authenticated `FactHead` and exact aligned facts/provenance |
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
| `QualifiedAdapter<S, C>` | Retain the minimal typed inert gate used by monomorphic Access `StateStart` to bind one registered implementation to `CommittedCall<S, C>`; remove forwarding-only catalog/Store identity checks |
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
- include every externally referenced configuration revision header and envelope, authenticated
  fact-index proof, and dependency-closed producer history needed to prove the consumer history;
- expose callback-free semantic inspection from Runtime's one reducer; and
- have App's replay/status surface call that inspection path.

The portable bundle does not put a conclusion-assigned fact-publication coordinate into
`StateConcluded`. When a retained `FactSelection` exists, however, its exact `FactHead`, canonical
route-index multiproof, and dependency-closed Journal-qualified producer histories are semantic
dependencies: omitting or substituting them means Runtime cannot return a proven `RunView`.
Publication headers beyond those exact dependencies may accompany an audit export as explicitly
physical audit metadata.

## 3. Proposed solution

This is a breaking replacement of the current responsibility chain, not an adapter around it. The
solution has seven coupled moves:

1. keep values typed from their first trusted construction until Journal encoding;
2. confine heterogeneous erasure to one private Runtime registry handoff;
3. move the sole Program reducer and next-State selection from Store to Runtime;
4. make Journal construction/decoding the only structural frame/history validation boundary;
5. replace semantic Store/backend layers with one mechanical persistence contract;
6. collapse public State-by-State lifecycle coordination into one bounded Runtime progression and
   inspection contract; and
7. make one PostgreSQL authority global by deleting tenant, scope, epoch, and identity rotation.

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
| Retained State-or-Match transitions are legal | Runtime reducer during one cold fold | Reduced semantic history |
| External configuration and retained fact selections match their exact refs, authenticated index/log frontiers, and producer histories | Journal structural qualification plus Runtime semantic correlation | `QualifiedRunDependencies` |
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

### 3.3 Absorbed Followups scope

The moved sections are represented once in this RFC:

| Former Followups section | Normative home here |
| --- | --- |
| 5.2 correlated value and registered start | Sections 4.2 and 12.1 |
| 5.3 typed lifecycle and 5.4 hot/cold convergence | Sections 4.1--4.4 and 7.2--7.4 |
| 5.5 bounded progression | Sections 4.6, 4.8, and 7.6 |
| 5.6 acknowledgement/custody | Section 4.7, preserving this RFC's private-owner policy rather than the superseded discard policy |
| 5.7 durable public view | Section 4.8, owned by Runtime rather than Store |
| 5.8 Runtime/App deletions | Sections 9.2, 9.6, and 10 |
| 10.2 Store placement | Sections 2.4, 6, and 9.4 |
| 10.3 Runtime placement | Sections 4, 7, and 9.2 |

The deferred Followups RFC points to these contracts and adds no parallel implementation.

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

Runtime assembly is the sole concrete association between persisted contracts and Rust execution
types. It registers one exact value codec per stable contract, schema identity, and Rust `TypeId`
association, one exact capability driver per complete capability association, and one
`RegisteredState` per complete State execution association. Program keeps
stable declarations and schema identities, but no Program-owned reifier, `TypeId` registry,
execution callback, or process-local catalog brand remains.

`RuntimeAssemblyBuilder::register_value<T>()` is the one trusted way to install an exact value
registration in that same assembly. Pure/Access State registration resolves its input, output, and
failure slots from the table; capability registration resolves intent/evidence slots and the exact
Read/Effect, attempt-bound, and fact-selection behavior.
Explicit registration also covers root admission/result values in a zero-State Program and values
used only by Match declarations. Assembly finalization rejects duplicate or inconsistent slots;
later `ExecutableProgram` association fails if that Program needs an exact slot that is absent.
This is one internal assembly table, not a second public value registry.

Every persisted State declaration carries an exact failure contract. An infallible State uses the
reserved closed `mfm.never` contract rather than absence, so a complete execution key is always
derivable from retained Program bytes. Access declarations also persist the complete capability
association needed below; intent/evidence types are never recovered from an
implementation-reference-only or capability-reference-only fallback.

A value registration whose contract is a closed sum also contains one pure
`MatchProjection` derived from that exact schema descriptor. It strictly projects the canonical
selector into its stable tag and canonical payload, and declares the exhaustive tag-to-payload
contract table. It performs no domain callback, State execution, adapter lookup, provider IO, or
authoring. `ExecutableProgram` association proves that each `MatchDeclaration` has exactly that
tag set, the declared payload contract for each tag, a registered payload codec, and the common
continuation contract. The reducer then projects and qualifies the selected payload through those
registrations; it never parses an ad hoc `serde_json::Value` or asks Program/Store for a callback.

The correlated value family is conceptual rather than a required public API:

```text
ProvenValue<T> {
  private exact Runtime value registration,
  value: T,
  canonical,
  value_ref,
}

ErasedProvenValue {
  private exact Runtime value registration,
  canonical,
  value_ref,
  value: Box<dyn Any + Send + Sync>,
}
```

`ProvenValue<T>` is constructed only by the exact typed Runtime registration after strict
canonicalization/validation of a trusted `T`, or after that registration strictly decodes retained
canonical bytes. `ErasedProvenValue` is non-Clone, non-Serde, and private to Runtime. It exists only
across the data-selected State registry handoff; it is never a Store, Journal, App, replay-report,
or adapter currency. A contract reference cannot be supplied separately from either proof.

The registry key is the complete persisted execution association:

```text
StateExecutionKey {
  implementation_ref,
  input_contract_ref,
  output_contract_ref,
  failure_contract_ref,
  capability: None | Some {
    capability_contract_ref,
    intent_contract_ref,
    evidence_contract_ref,
    mode: Read {
      total_attempt_bound,
      fact_selection_required,
    } | Effect {
      fact_selection_required,
    },
  },
}

RegisteredState {
  key: StateExecutionKey,
  private exact input/output/failure registrations,
  private optional exact capability registration,
  start: StateStart,
}

StateStart = Arc<
  dyn Fn(StartContext, ErasedProvenValue) -> StateFuture
    + Send
    + Sync
    + 'static,
>
```

Pure uses no capability key. Access uses its complete exact capability association. Duplicate
complete keys, or one complete key resolving to different codec/type metadata, fail assembly.
Sharing `implementation_ref` across distinct complete keys is allowed. Lookup never uses a partial
prefix and never falls back to implementation reference alone. Missing registrations and
incompatible schema/type registrations fail association before execution.

`effect_domain`, the immutable binding reference, and the maximum conclusion reservation are
occurrence data rather than State-driver identity. Runtime still validates them exactly against
the selected Access registration and retained occurrence before work.

Adapter selection remains occurrence-specific. After the State key matches, Runtime validates the
declaration's exact immutable binding reference under the already-selected State/capability
driver. The monomorphic Access `StateStart` captures that driver's typed adapter table;
`StartContext` supplies the validated occurrence and `binding_ref`, and the start resolves a
`QualifiedAdapter<S, C>` directly without another `Any`, erased adapter, or downcast. Registering
several bindings does not duplicate the semantic State driver. `StartContext` carries only Runtime
authority for that exact occurrence; it is not an ambient domain context map.

Registering concrete `S` installs one monomorphic start function. An Access registration is
conceptually:

```text
register_access<S, C>(assembly, implementation):
  input_registration = assembly.registered_value::<S::Input>()?
  output = assembly.registered_value::<S::Output>()?
  failure = assembly.registered_value::<S::Failure>()?
  capability = assembly.registered_capability::<C>()?
  adapters = assembly.registered_adapters::<S, C>()?

  RegisteredState {
    key: exact key derived from S and C,
    exact registrations,
    start: move |context, erased_input| {
      start_typed::<S>(context, erased_input, input_registration, |context, typed_input| {
        adapter = adapters.resolve(context.occurrence, context.binding_ref)?
        run_typed_access::<S, C>(context, typed_input, implementation, adapter)
      })
    },
  }
```

Exact Rust spelling is intentionally non-normative. Pure registration supplies its typed Pure
runner to the same `start_typed` implementation and introduces no fake capability. `StateFuture`
is private and does not create a second public cursor or lifecycle algebra.

`start_typed` contains the only dynamically selected State-context downcast. It checks the private
registration identity, stable contract, schema identity, assembly identity, and `TypeId` before
moving the value into `ProvenValue<S::Input>`. A mismatch is a redaction-safe internal
Program/assembly inconsistency and stops before State, adapter, provider, or append work. There is
no alternate decoder or fallback key.

The proof wrapper remains Runtime-private. State and capability callbacks receive only their
ordinary typed domain inputs (`S::Input`, `C::Intent`, `C::Evidence`, or borrows thereof); they
cannot observe or alter canonical bytes, content references, registration witnesses, or cursor
ownership. Runtime retains that correlated metadata around the callback and qualifies the returned
typed value once.

Capability intent and evidence stay typed inside the selected Access driver. The driver strictly
decodes retained `C::Intent` and `C::Evidence`, verifies exact request/call/evidence binding, and
derives fact requests through `C`; it does not route those values through the State-context
existential. Runtime qualifies a typed `ProposedStateOutcome<S::Output, S::Failure>` into the exact
output or failure `ProvenValue` and a correlated fact-proposal proof before Journal encoding or hot
promotion.

Lookup is dynamic; execution after lookup is typed:

```text
Program declaration
  -> exact StateExecutionKey
  -> RegisteredState.start
  -> start_typed::<S>
  -> ReadyPure<S> or ReadyAccess<S, C>
  -> typed State/capability methods
```

Runtime first associates the complete supplied Program with this assembly: every State execution
key, Match projector, capability, value/schema contract, binding contract, and entry point must
resolve semantically. Before admission, Runtime additionally requires every Access binding in that
Program to resolve to its exact live adapter. Admission then retains exactly one canonical Program
object in the genesis closure. On cold resume, Journal proves that object is present and
content-addressed, Program strictly decodes it and verifies the recorded reference, and Runtime
compares its entry point with `RunAdmitted` before repeating semantic association and folding. A
live adapter is required on new-run admission and when cold execution is about to enter the
selected Access occurrence. Inspection never requires or invokes one. No current-code planner may
substitute a Program.

The resulting private `ExecutableProgram` is the process-local proof that every retained
declaration has one exact `RegisteredState`, every Access occurrence's immutable binding reference
is compatible with that State/capability key, every Match projector is exhaustive and exact, and
the root admission contract matches the selected entry point. It caches stable occurrence and
binding references plus semantic compatibility, never an erased adapter handle. Missing,
ambiguous, or mismatched semantic association fails before `RunAdmitted`; cold resume and semantic
inspection report assembly incompatibility and perform no write. Missing live adapter availability
prevents only admission or actual Access entry. The proof is derived from the retained Program on
cold paths and never serialized.

On the hot path, Runtime promotes the typed output it already owns into the next cursor state. It
does not serialize, ask Store to requalify, and deserialize its own output. Journal bytes are built
for durability while Runtime retains the correlated proof. When the next State is data-selected,
the value crosses the one private erased handoff and immediately re-enters typed execution.

On cold resume, the exact selected driver decodes retained current context into its concrete type
once while the reducer establishes the new process-local cursor. Historical semantic validation
uses the same registered codecs without invoking State, adapter, or provider callbacks. Once the
cursor exists, hot execution uses the retained proof.

### 4.3 Runtime owns selection

There is one callback-free reducer for execution, cold resume, status, and replay. It consumes an
`ExecutableProgram`, the consumer `JournalHistory`, and exact `QualifiedRunDependencies` and yields
one private `ReducedRun` containing either:

- a runnable selection `{ occurrence, StateExecutionKey, current value proof }`;
- a waiting preparation/conclusion condition;
- terminal typed success;
- declared typed failure; or
- a redaction-safe inconsistency error.

`QualifiedRunDependencies` is a process-local proof bundle, not a second history or reducer. Its
root contains the Journal-qualified configuration revision header and envelope matching the
consumer admission's
immutable `ConfigurationRef` and one `ProvenFactSnapshot` for every retained preparation that
carries a `FactSelection`. Each snapshot binds the retained request and frontier to authenticated
physical index coverage and the exact qualified producer nodes named by its provenance. Missing,
extra, conflicting, or insufficient dependency material is a typed
`MissingDependencies`/invalid-history result; Runtime never treats a consumer run alone as proof of
external facts.

The bundle is a bounded, deduplicated dependency DAG. Each producer node contains its
Journal-qualified run prefix through the referenced producer frame, that prefix's retained Program
associated into its own `ExecutableProgram` under the same `RuntimeAssembly`, its admission's exact
Journal-qualified configuration revision header and envelope, and the fact snapshots for any
selections retained by that prefix. Configuration revision headers/envelopes are deduplicated by
full immutable `ConfigurationRef`; Program association proofs remain node-specific and
process-local. Producer nodes are semantically folded only through the referenced frame.
Append-only-log consistency proofs and strictly earlier publication frontiers make the graph
acyclic; Runtime rejects cycles, non-prefix/forward/self dependencies, and node/depth/byte limits
plus one.

Store-backed resume/read constructs the root and every producer node by loading each exact
configuration reference, obtaining the authenticated fact-index proof at each recorded frontier,
and loading each producer run through the recorded producer head. Journal structurally qualifies
all envelopes, histories, and index proofs; Program decodes every retained Program; and Runtime
performs every Program association and cross-history semantic correlation. Journal's portable codec
carries the equivalent bounded dependency bytes. Therefore portable semantic inspection can return
a proven `RunView` only from a complete portable bundle; a run-only export remains useful as
structural data but is not semantic inspection input when external dependencies are referenced.

The reducer never reads Runtime's process-local pending-owner table. The acknowledgement
coordinator joins a `ReducedRun` with an exact pending append only after the durable fold, and live
cursor materialization happens only after that classification. A runnable `ReducedRun` is not a
typed State step and contains no `StateFuture`; only `start`/`resume` may pass its selected value to
the exact `RegisteredState.start`. `read`, status, replay, trace, and audit map the same selection to
`RunView::Runnable` without starting it. Semantic inspection therefore requires compatible
semantic codec/contract registrations from one `RuntimeAssembly`, but no live adapter;
incompatibility is a typed result, not a reason to fall back to a second reducer or current
authoring code.

There is no public Store-level `RunAction` algebra. Process-facing Runtime APIs expose only the
durable view and reviewed Runtime errors described below, not Store selection internals.

### 4.4 Direct-new provider-entry proof

Direct-new gating remains because it proves a real safety property:

```text
PreparedAccess<S, C>
  -> Journal encodes StatePrepared
  -> Store append
       Inserted  -> consume PreparedAccess -> CommittedCall<S, C> -> provider entry
       Existing  -> do not mint call; fold durable history
       Stale     -> do not mint call; reload and fold
       AcknowledgementUnknown -> preserve exact pending owner; do not mint call
```

Only Runtime can perform the promotion because Runtime owns the typed preparation. Store returns a
mechanical result; it does not construct `AccessPreparationOutcome` or another semantic owner.

### 4.5 Admission and conclusion races

Admission uses expected head `Absent`. Store's exact-ID `Existing` yields `CommittedExact` only
after command-digest equality. If another append identity already created the run, Runtime loads and
qualifies its genesis and classifies `AlreadyAdmittedSame` only when entry point, exact retained
Program/object closure, admitted context, configuration reference, and sources are identical.
Different genesis under the same `RunId` is `AdmissionConflict`. An unknown acknowledgement is
resolved only by reissuing the identical command through Store's idempotency-first transaction;
that transaction returns `Inserted`, exact `Existing`, `Stale`, a definite no-commit error, or
another unknown. Missing/corrupt/non-genesis-first history is `InvalidHistory`. Concurrent starts
therefore converge on the existing semantic admission or one distinct conflict and never expose an
unqualified head.

Runtime encodes its typed conclusion against an exact expected head. `Stale` keeps the owner leased
while Runtime reloads retained bytes, Journal qualifies them, and the same Runtime reducer
classifies the result below. `AcknowledgementUnknown` first reissues the identical command through
the idempotency-first Store path; another unknown returns `Indeterminate`, exact
`Inserted`/`Existing` proves the command durable, and only a resulting `Stale` enters reload
classification:

1. `CommittedExact`: the same append identity contains the byte-identical frame and physical
   attachment;
2. `AlreadyConcludedSame`: another append identity durably recorded the same semantic conclusion
   for the occurrence;
3. `NoLongerSelected`: a permitted Access replacement superseded the preparation;
4. `Conflict`: another valid non-identical conclusion owns the occurrence;
5. `InvalidHistory`: structural or semantic qualification fails; or
6. a typed permanent rejection when the exact pending append is not admissible.

Store does not return `SelectedConclusion`. PostgreSQL's append receipt and retained rows contain
all mechanical evidence needed for Runtime to decide.

`CommittedExact` and `AlreadyConcludedSame` release the durable result without callback re-entry.
`NoLongerSelected` discards the local result and continues from retained history. `Conflict` is a
distinct operational error. `InvalidHistory` and permanent rejection fail closed. Reusing an
append identity with different expected head, bytes, closure, reservation, or sealed publication
attachment is `IdempotencyConflict`, never `Existing`; `Stale` remains mechanical evidence that
Runtime must classify semantically. Finding a preparation in history, including the exact one,
never recreates provider-entry authority.

An exact-ID `Existing` receipt is intentionally returned before current-head comparison and may be
historical after later appends. It proves only that its exact candidate committed. Before returning
a current `RunView` or starting further work, Runtime authoritatively loads and folds `Current`
unless that same authority separately proves the receipt head is still current. This rule applies to
admission and Pure/Access conclusions; configuration `Existing` differs because its result is one
immutable revision proof rather than a run-progression cursor.

Retrying the same owner means Runtime reissues the byte-identical borrowed `RunAppend` to the same
primary Store authority. Store's mandatory idempotency-first transaction either finds the durable
command, inserts it only if the head is still the exact predecessor, returns stale, proves a
definite no-commit error, or remains unknown. `UnavailableBeforeSubmission` retains the owner for a
later identical retry; only permanent command rejection (`InvalidPhysicalCommand`,
`IdempotencyConflict`, or immutable append-capacity failure) consumes it. Corruption before
exhaustive classification retains custody and fails closed. There is no
replica/cache lookup or snapshot miss that authorizes retry, and no retry uses a new append identity.
Another unknown result leaves Runtime `Indeterminate` with the same owner.

None of these branches re-enters a State, adapter, provider, fact selector, or authoring callback.

### 4.6 One bounded process-facing progression path

Runtime exposes typed start and resume methods that delegate to one private
`advance_until_stable`. Read/inspect uses the same reducer without executing work. Start and resume
drain immediately actionable sequential States until the first stable external boundary:

- terminal success or declared terminal failure whose exact retained value is durable;
- a durable preparation or unresolved provider result that permits no immediate action;
- the nonzero per-call State-start bound reached at a durable runnable head;
- an acknowledgement owner still indeterminate after its bounded resolution attempt;
- a same-run conflict or invalid history;
- absence, assembly incompatibility, or a redaction-safe capacity/infrastructure failure.

`RuntimeLimits` owns nonzero bounds for active progression calls, deterministic CPU work, provider
ingress, `max_state_starts_per_call`, and retained pending append owners. The State-start bound is
checked only at a durable boundary before entering another `RegisteredState.start` and counts each
start once. Runtime never interrupts an active State or provider future merely to manufacture a
yield. Reaching the bound returns a `Runnable` durable view; a later resume continues from that
head without replaying a durable conclusion.

This remains caller-driven execution, not scheduling. Runtime creates no background worker, timer,
queue, general per-run execution lock, or process-wide writer lease. Concurrent callers and
processes may fold the same head and race one mechanical compare-and-append. App requests progress;
it never requests one raw State step.

### 4.7 Private acknowledgement-owner fate

Runtime, not App or Store, owns one bounded process-local table of affine admission, preparation,
conclusion, and configuration append owners whose physical or semantic append fate is not yet
exhaustive. That includes in-flight/unknown acknowledgement, retryable definite no-commit failure,
and stale-result classification. The table is keyed by complete physical append identity, permits
several concurrent owners for one `RunId`, and is bounded before submission. Capacity failure
occurs before append or provider work.

Conceptually, its key and value families are:

```text
PendingKey = Run { run_id, append_request_id }
           | Configuration { append_request_id }

PendingAppend = Admission { proven_c0, executable_program, run_append }
              | Preparation { prepared_access, run_append }
              | Conclusion { typed_conclusion, successor, run_append }
              | Configuration { pending_configuration, configuration_append }
```

Run progression indexes only `Run` entries. Repeated high-level configuration publication indexes
only `Configuration` entries and returns its separate typed configuration result/error contract.

The capacity permit is acquired before work that can create the corresponding append authority and
travels with that authority. For Access it follows
`PreparedAccess -> CommittedCall -> TypedConclusion -> PendingAppend`, so a provider result can
always enter acknowledgement custody without a post-provider capacity failure. Admission,
configuration, and Pure conclusion paths likewise reserve before the operation that can produce an
append candidate. An inserted preparation transfers, rather than releases, its same permit into
`CommittedCall` and then `TypedConclusion`; it is released only after that conclusion is durably
qualified or reaches an exhaustive terminal fate. Admission, Pure conclusion, and configuration
permits release after exact durable qualification, definite rejection, or explicit trusted-process
discard. Stale/reload classification either transfers the permit into the next named owner or
releases it; no branch silently loses or duplicates capacity liability.

Ownership transfers into the table before the append await. An in-flight resolution uses a
cancellation-safe lease that returns the exact owner to the table if the caller future is dropped.
The lease stays live until all Store loading, Journal/dependency qualification, and Runtime folding
needed for the branch has completed:

| Observation | Affine owner fate |
| --- | --- |
| `Inserted` | Verify receipt/command correlation and the already Journal-sealed candidate, then transfer/promote/release into the exact durable fate. |
| run command-identical `Existing` | The candidate is durable, but its immutable receipt may name a historical head. Keep the lease through an authoritative current-head load, dependency qualification, and fold unless that load separately proves the receipt still current. Never hot-promote from an old receipt; `Existing` preparation never mints a call. Configuration `Existing` instead promotes its exact immutable revision after header/envelope qualification. |
| run `Stale` | Keep the lease through current-run/dependency load and semantic classification. Cancellation, capacity, corruption, or infrastructure failure before classification returns it to the table. |
| configuration `Stale` | The exact expected global configuration head is no longer current, so this is an exhaustive typed configuration conflict; consume the candidate without loading a run. |
| `AcknowledgementUnknown` | Return the same owner to the table and report `Indeterminate` after the bounded identical retry. |
| `UnavailableBeforeSubmission` | Return the owner to the table; the exact command remains retryable. |
| `CorruptPhysicalState` before exhaustive classification | Return the owner to the table and fail closed; operator repair or process loss decides its later fate. |
| append-time `InvalidPhysicalCommand`, `IdempotencyConflict`, or immutable `MAX_RUN_BYTES`/reservation `Capacity` | Consume the owner into a permanent typed rejection because the identical command can never become valid. |
| completed stale classification (`Already*`, `NoLongerSelected`, `Conflict`, semantic `InvalidHistory`, or permanent rejection) | Transfer or consume according to that exhaustive semantic fate. |

Load-time capacity is not append-time capacity: if it prevents stale classification, the owner
remains retained. Exact synchronization and container spelling are implementation details, but an
unbounded map, post-await insertion window, cloneable retry token, or owner exposed above Runtime
is forbidden.

Cancellation safety also covers the work around append awaits. The phases are exhaustive:

1. Before the preparation append commits, cancellation during deterministic State preparation,
   fact loading, or submission discards only the uncommitted candidate and releases its unused
   permit; no provider call exists.
2. Once an inserted preparation mints `CommittedCall`, cancellation before provider ingress or
   while its future is in flight consumes that call authority, releases the permit, and leaves only
   the already durable preparation. There is no retained-call table or inferred re-entry; later
   progress follows the ratified cold-recovery rule.
3. Once provider ingress returns accepted Outcome or BlockedIntegrity, Runtime performs bounded
   synchronous interpretation/qualification and installs `TypedConclusion` in acknowledgement
   custody before the next cancellation point. Cancellation after that handoff returns the exact
   conclusion lease and its still-transferred permit to the table. An Unresolved result instead
   consumes call authority, releases the permit, and leaves only the durable preparation.

Start or resume first resolves every pending owner relevant to that run before selecting new work.
One bounded resolution attempt reissues the exact borrowed append through Store's
idempotency-first transaction, reloads stored bytes when classification requires it, asks Journal
to qualify them, and folds them with the same Runtime reducer. If the result remains
unknown, Runtime returns `RunError::Indeterminate { run_id }` and retains the owner; it does not
fabricate a head or execute new State/provider work. Read remains callback-free and may report only
the currently durable `RunView`; it neither resolves nor exposes pending ownership.

Owner-specific promotion remains strict:

- an inserted preparation consumes `PreparedAccess<S, C>` into `CommittedCall<S, C>`;
- a preparation later found after acknowledgement loss never recreates direct-new provider-entry
  authority and folds as a durable preparation, allowing only the declared Read replacement or
  Effect parking rules;
- an inserted conclusion releases the retained typed successor only after its durable frame is
  qualified; a command-identical `Existing` also proves that frame durable, but Runtime folds the
  current head before further execution and uses the retained successor hot only if an authoritative
  current-head load proves the receipt is still current;
- an inserted admission releases its retained typed `C0` only after the exact genesis and Program
  closure are qualified; command-identical `Existing` follows the same current-head rule rather than
  treating an old genesis receipt as the current run; and
- an inserted or found-identical configuration append promotes the retained typed revision under
  its exact Runtime registration from `PendingConfiguration<C>` into
  `ProvenConfiguration<C> { value, canonical envelope, ConfigurationRef }`.

Process death discards process-local owners. Cold recovery derives no provider-entry authority from
history: Pure may be recomputed, Read may replace only within its declared bound, and an unresolved
entry-once Effect remains parked. A trusted process shutdown may explicitly discard its entire
private table; there is no per-owner Application or transport disposal endpoint.

### 4.8 One Runtime-owned durable run view

Runtime's callback-free reducer projects every qualified durable history into one
transport-independent contract:

```text
RunView {
  run_id,
  head_sequence,
  head_digest,
  state: Runnable | Waiting | Succeeded(RetainedValueView) | Failed(RetainedValueView),
}

RetainedValueView {
  contract_ref,
  value_ref,
  canonical,
}
```

Every `RunView` has a real qualified genesis and head. `Runnable` means the durable Program head
permits another State, including a work-bound stop. `Waiting` means retained history currently
permits no immediate action. `Succeeded` and `Failed` carry only the exact terminal value and
contract proven by the retained Program and history; a nonterminal Match or recoverable path is
still `Runnable`.

Start, resume, and read return this same view whenever a qualified durable view is the result.
Absence, indeterminate acknowledgement, conflict, invalid history, assembly incompatibility,
missing semantic dependencies, capacity, and infrastructure failure remain distinct redaction-safe
errors rather than view variants. App does not inspect `RunFrame`, infer status, or install a
terminal projector.

Status, terminal result, trace, audit, replay, and export are callback-free projections over the
same Journal-qualified history and Runtime reducer. Inspection projections may use Runtime's exact
value/capability codecs and structural Match metadata, but it invokes no State implementation,
adapter, provider, configuration authoring, or other ambient IO.

## 5. Target Journal design

### 5.1 One valid-by-construction codec boundary

Journal exposes two conceptual directions:

```text
trusted Runtime projection
  -> Journal structural encoder
  -> EncodedRunFrame { private canonical frame/object bytes, sealed publication attachment }
stored run bytes -> strict decode/qualification -> JournalHistory
```

`EncodedRunFrame` and `JournalHistory` have private fields. `RunFrame` below is the canonical wire
record inside that codec, not a second public construction DTO. There is no workflow in which callers
construct a partially checked public DTO, call `validate`, add more fields, and ask Store to check
it again.

Journal owns only intrinsic durable rules:

- canonical JSON and content hashing;
- float-free hashed structures;
- frame/object/per-run bounds;
- record-family shape;
- exact content-addressed object closure: every referenced closure-local object is present and no
  unreferenced object is admitted;
- run ID and dense frame sequence;
- recursive run head;
- configuration-revision and fact-index/log proof structure; and
- strict portable encoding/decoding.

Runtime owns all meaning that requires the admitted Program: current context, occurrence
reachability, State/capability contract association, preparation-to-conclusion legality, Match
selection, and terminality.

Genesis qualification is split without duplication. Journal proves that sequence one is the
unique `RunAdmitted`, its closure contains exactly one canonical `mfm.program` object, the recorded
`program_ref` names that object, and every closure-local reference is present and content-correct.
The configuration reference is an external durable dependency, not an object Journal pretends
belongs to the genesis closure. Source references are intrinsic opaque members of the admitted
allow-list: Journal proves their bounded sorted/unique representation, while Runtime correlates
fact requests and selected provenance against them; they do not imply another object-loading
protocol. Program strictly decodes the retained Program object and proves its graph. Runtime
compares the Program entry point with `RunAdmitted`, proves the admitted context contract and
complete Program/assembly association, and validates the external configuration through
`QualifiedRunDependencies`. Hot admission persists that exact already-validated Program; cold
resume and portable inspection never rebuild it from current authoring code.

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
  previous_head,
  record,
  object_closure,
}
```

Exact field names and whether an absent optional reference is omitted from canonical JSON remain
codec details. No scope, epoch, tenant, append request ID, conclusion-assigned fact-publication
sequence, record ordinal, or duplicated Program-derived metadata is part of the semantic frame.
`FactSelection.frontier: FactHead` remains canonical as the immutable global fact snapshot observed
by a preparation; none of its fields is the conclusion's database-assigned publication coordinate.
Genesis has no predecessor; every later frame records the exact previous recursive head.

`RunAdmitted.source_refs` is a bounded, sorted, unique vector. Every source in a capability's fact
request must be a member of that admitted vector. A selected fact must match the exact requested
source and subject and must retain producer run, producer frame sequence/head, Program/proposal-set
reference, value reference, and canonical value evidence. Runtime rejects subsets, duplicates,
unadmitted sources, or provenance that merely names a plausible value without its producer proof.

## 6. Target Store and PostgreSQL design

### 6.1 Store is a mechanical port

Store accepts the sealed encoded append projection from Journal and returns retained bytes,
bounded physical snapshots, or mechanical receipts. It does not recanonicalize or reopen Journal
proofs. It may enforce physical byte ceilings, SQL-safe integer ranges, row ordering, key/head
correlation, and transaction completeness, but it does not inspect the Program or reconstruct
domain values.

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
| content-addressed objects | primary key `object_ref`; exact bytes are immutable |
| per-run object membership/charge | primary key `(run_id, object_ref)`; immutable logical byte charge |
| run append requests | primary key `(run_id, append_request_id)`; immutable command digest and receipt |
| run heads | primary key `run_id` |
| active conclusion reservations | primary key `(run_id, reservation_key)`; exact maximum and aggregate agree with run head |
| fact publication headers | primary key global `publication_sequence`; unique `(run_id, run_sequence)`; complete previous/resulting heads, descriptor/update-set digests, and producer locator |
| immutable fact-index nodes | primary key `node_digest`; exact canonical sparse-Merkle leaf/branch bytes are immutable and historical roots remain reachable |
| immutable fact-log nodes | primary key `node_digest`; exact canonical append-only-Merkle leaf/branch bytes are immutable and historical roots remain provable |
| fact head | one singleton `FactHead { publication_sequence, index_root_digest, log_root_digest }` row |
| configuration revisions | primary key global `revision_sequence`; immutable envelope/routing key plus previous/head digest; secondary index `(routing_key, revision_sequence DESC)` |
| configuration append requests | primary key `append_request_id`; immutable command digest and receipt |
| configuration head | one singleton `ConfigurationHead { revision_sequence, head_digest }` row |
| EVM nonce domains | `(sender_id, nonce_domain_id)` |
| EVM nonce operations | `(sender_id, nonce_domain_id, operation_key)` |

Run heads retain the exact recursive head/current sequence plus cumulative `total_bytes` and
`reserved_bytes`, so append does not rescan history. Active reservation and per-run object-membership
rows are mandatory physical metadata and change under the same append transaction. None of this
metadata changes Program semantics.

Fact log and sparse-index hashes use the fixed recurrences specified in Section 2.4. The
singleton names both latest digests; historical publication headers and immutable nodes remain
available for exact old-root proofs and are not garbage-collected while this format is supported.
Configuration revisions similarly persist `previous_head_digest` and compute
the following SHA-256/JCS recurrence:

```text
configuration_routing_digest = H(JCS {
  domain: "mfm.configuration.route.v1",
  instance_id,
  contract_ref,
  schema_ref
})
configuration_genesis_digest = H(JCS {
  domain: "mfm.configuration.head.empty.v1"
})
head_digest = H(JCS {
  domain: "mfm.configuration.revision.v1",
  revision_sequence,
  previous_head_digest,
  envelope_ref,
  routing_key
})
```

The first revision is sequence one and uses exactly `configuration_genesis_digest` as predecessor;
an absent configuration stream has no singleton head row. The singleton points at the latest row;
any historical `ConfigurationHead` is
verified against its exact revision row before latest-by-instance-and-contract lookup. A forged sequence/digest
pair is corruption, never an alternate snapshot. Canonical goldens freeze the routing-key and
reference representation.

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

Exact-head CAS does not fence an old process across an in-place database rollback that recreates a
previously observed head. Such restore is therefore an explicit operational stop-the-world event:
operators stop every Runtime/Store peer, restore the database, and start fresh processes against
the restored authority. Crash/restart against an unrolled-back database remains supported. No API
may imply that removing epoch rotation preserves live-peer restore fencing.

### 6.4 Fact publication without frame rebinding

The current conclusion protocol asks Store to obtain a fact publication coordinate, insert that
coordinate into the conclusion, rebuild the hashed frame, rotate/rebind append metadata, and retry
if the global fact frontier changed. This gives Store semantic mutation authority and makes global
concurrency alter canonical run bytes.

The target conclusion contains only the content-addressed fact proposal-set reference. PostgreSQL
allocates publication order in the same transaction:

```text
1. serialize `(run_id, append_request_id)`, then look it up after any waiter completes
2. return its original receipt only if the complete append is identical; reject conflicting reuse
3. for a new request, serialize the `run_id` head key even when absent, then lock/read and compare
   the run head
4. for a non-empty proposal set, lock the singleton fact head
5. allocate publication_sequence = current + 1; Journal mechanically batch-applies every sealed
   route update to the previous index root, encodes the publication descriptor, appends its leaf to
   the prior Merkle log, and returns the new immutable nodes, header, and exact resulting `FactHead`
6. compute per-run first-reference charge and validate/apply None/Open/Replace/Consume accounting
7. insert the immutable run frame, objects, per-run object membership, sparse-index/log nodes, and
   publication header
8. update active reservation rows and advance run/fact heads and byte/reservation aggregates
9. insert the append-request command digest with the final immutable AppendReceipt
10. commit and return that receipt
```

An idempotent retry uses the same `(run_id, append_request_id)`, finds the same frame and fact root,
and returns the same receipt. Frame and publication either both commit or both roll back.
For a frame without a proposal set, steps 4--5 and the publication/index writes are absent, but
the reservation, run, object-membership, head, and append-request writes remain one transaction.

This deletes:

- `PreparedConclusion::bind_fact_publication` and rebind logic;
- fact-rebind ordinals and rotated append IDs;
- `FactFrontierChanged` conclusion retries;
- fact publication coordinates from Journal hashing;
- Store-side coordinate validation; and
- `fact_frontier` as an append precondition.

Runtime reads a structural fact snapshot and performs semantic fact selection. The selection records
the complete exact `FactHead`--publication sequence, append-only log root, and authenticated
index root--through which it was made and the exact provenance of selected values. Later
publications do not invalidate it. With one database authority,
`FactSelectionFrontier.stream_ref` disappears; the retained `FactHead` is the one snapshot anchor.

The Journal encoder seals `None | NonEmptyFactPublicationAttachment` into `EncodedRunFrame`. A
non-empty attachment can be constructed only when the conclusion names the same proposal-set
reference and that object is present and structurally non-empty in the frame closure. It cannot be
replaced independently after encoding. Its bounded all-and-only update templates are derived from
those same proposal objects, reject duplicate `(source_ref, subject_ref)` routes, and are covered by
the encoded-frame identity. Store consumes this correlation mechanically; it never derives fact
meaning or decides whether a proposal is usable.

Store returns the one bounded membership/non-membership multiproof for Runtime's exact route query
at either the newly captured head or a retained selection's recorded `FactHead`. Journal verifies
the head/query/result correlation, complete fixed-depth sparse-Merkle proof, key preimages, canonical
node hashes/root, frontier-header inclusion, and each deduplicated publication-header inclusion.
For every producer dependency edge, Journal also verifies the canonical append-only-log consistency
witness from the older retained frontier to the producer publication's previous `FactHead`. Runtime
then loads every present leaf's producer prefix through its recorded
head; it never falls back to an older leaf when the latest one is absent, malformed, or semantically
invalid. Journal qualifies those frames and referenced canonical objects. Runtime creates one
private `ProvenFactSnapshot`: it correlates the request, captured head, leaves, proof, and producer
histories and verifies the admitted sorted/unique source manifest, request-source membership, exact
source and subject, producer `RunId`, producer frame sequence/head, Program/proposal-set ordinal/value
references, canonical value bytes, request compatibility, and the capability's exact fact behavior
before selecting values. An authenticated absent route is `NotActionable`, not permission to scan
the publication log.

Producer proof is dependency-closed. If a producer prefix itself retained prior-fact selections,
the portable/Store-backed proof bundle includes their exact dependencies recursively through the
needed producer frame. Every dependency edge has a proven earlier prefix of the same global fact log,
and every publication sequence strictly dominates all fact frontiers on which its producer prefix
depends, so Runtime rejects cycles, non-prefix/non-decreasing edges, and deduplicates the bounded
run/head DAG. Structural
producer evidence alone is insufficient when semantic validity of that producer conclusion depends
on an earlier selection.

`MAX_FACT_DEPENDENCY_NODES`, `MAX_FACT_DEPENDENCY_DEPTH`, and
`MAX_FACT_DEPENDENCY_BYTES` are immutable Journal/protocol constants, not mutable Runtime tuning.
Runtime enforces both each selection closure and the accumulated dependency closure of the consumer
run before appending `StatePrepared`; cold/portable qualification applies the same constants. A
history produced by the current format therefore cannot become invalid merely because a restarted
process chose smaller operational limits. Runtime construction must provision at least the protocol
proof ceiling; per-call concurrency/provider/State-start limits remain separate.

`PreparedAccess<S, C>` retains that snapshot proof and the selected values. The selection's complete
`FactHead` is an immutable observed frontier; it is distinct from, and does not reintroduce, the
database-assigned publication coordinate into a canonical conclusion.

### 6.5 Configuration

Configuration uses one global append-only revision stream and singleton head. Runtime is its sole
typed owner, Journal owns the opaque canonical revision-envelope codec, and Store loads/appends that
envelope mechanically.

The hot publication path is:

```text
ConfigurationInstanceId + typed C
  -> Runtime exact codec canonicalizes/validates C
  -> PendingConfiguration<C> { instance, value, value_ref, exact registration }
  -> Journal encodes ConfigurationEnvelope {
       instance/contract/schema routing key, value_ref, exact object closure
     }
  -> Store.append_configuration
  -> Inserted or exact Existing ConfigurationAppendReceipt {
       ConfigurationHead, ConfigurationRef
     }
  -> Store exact-loads the assigned ConfigurationRef
  -> Journal qualifies its revision header, envelope, and exact closure
  -> ProvenConfiguration<C> { value, envelope, ConfigurationRef }
```

The database-assigned revision sequence is not available in `PendingConfiguration<C>` and is not
part of the content-addressed envelope. `ConfigurationHead { revision_sequence, head_digest }` is
the physical global CAS state. `ConfigurationRef { revision_sequence, envelope_ref,
revision_head_digest }` identifies one immutable typed revision and is created only from an inserted
or exact-existing receipt. Its Journal-qualified `ConfigurationRevisionHeader` binds those fields
to the opaque routing key and predecessor digest; `envelope_ref` names the complete canonical
revision envelope. These types are not interchangeable.

On exact load, Store retrieves by the full `ConfigurationRef`, Journal qualifies its exact revision
header, envelope, and object closure once, and Runtime's exact registration decodes/validates `C`
into a new process-local `ProvenConfiguration<C>`. The same header accompanies an offline portable
bundle, so Store-backed and portable inspection prove the identical sequence-to-envelope binding.
For latest-by-instance-and-type load, Runtime captures a `ConfigurationHead`, derives the opaque
instance/contract/schema routing key from the supplied `ConfigurationInstanceId` and exact `C`
registration, asks Store for the greatest matching `ConfigurationRef` at or below that head, and
then performs the same exact load. A later revision of another instance or configuration contract
cannot masquerade as the requested latest `C`. Completeness of that online greatest-matching-row
query is a trusted mechanical Store/backend obligation covered by Memory/PostgreSQL parity tests;
the current threat model does not claim a cryptographic non-omission proof against a Byzantine
backend. Portable inspection proves exact retained revisions and never claims that one was latest.

Configuration append is one analogous idempotency-first transaction: serialize the append-request
ID, then look up and compare the complete request digest after any waiter; serialize the global
`ConfigurationHead` key even when its row is absent; lock/read and compare that head; insert the
envelope/object closure; have Journal structurally encode the assigned
`ConfigurationRevisionHeader`; insert that header/revision row (whose secondary index serves
latest-by-instance-and-contract lookup); insert the append-request digest and immutable receipt; advance the
singleton head; and commit. Envelope, objects, revision header, request receipt, and head all
commit or all roll back. The same-ID loser observes the winner's committed receipt or conflicting
digest rather than misclassifying the advanced head as `Stale`; two distinct first publications
also serialize against the absent-head key.

The conceptual high-level Runtime API is:

```text
publish_configuration<C>(instance_id, expected: Absent | Present(ConfigurationHead), append_request_id, C)
  -> Result<ProvenConfiguration<C>, ConfigurationError>
load_configuration<C>(ConfigurationRef)
  -> Result<ProvenConfiguration<C>, ConfigurationError>
load_latest_configuration<C>(instance_id, through: Absent | Present(ConfigurationHead))
  -> Result<Absent | Present(ProvenConfiguration<C>), ConfigurationError>
```

`publish` reserves pending-owner capacity before encoding/submission. `Inserted` and exact
`Existing` retain the lease while Store exact-loads the assigned revision and Journal qualifies
the header, envelope, and complete closure; only then may Runtime mint `ProvenConfiguration<C>`.
Load, capacity, corruption, or infrastructure failure before that qualification returns the owner
to custody. Unknown acknowledgement or
caller cancellation retains the exact `PendingConfiguration<C>` under the configuration append
identity. A repeated high-level `publish` with the same append ID, expected physical head, exact
instance/contract, and canonical `C` first resolves that private owner by reissuing its same
borrowed command through Store's idempotency-first path; it never constructs a second owner.
Different caller arguments return a pending-request mismatch without consuming or replacing the
resident owner. A structurally valid stored command with the same ID but a different digest is
`IdempotencyConflict`; a malformed digest/receipt correlation is `CorruptPhysicalState`, not a
conflict. Inserted/exact-existing promotes only after the exact qualification above; stale returns a typed configuration
conflict, pre-submission unavailability or corruption before exhaustive classification retains it,
and repeated unknown retains it. The Section 4.7 custody table, including its configuration-specific
`Existing` and `Stale` rows, applies: cancellation and transient load/qualification failures return
the lease, while append-time invalid command,
idempotency conflict, or immutable envelope/format capacity consumes it into a permanent typed
rejection. Thus retry is caller-driven without exposing an owner token or a second resolution
endpoint. After process loss, an identical
high-level retry may qualify Store's exact existing envelope into a fresh proof; it cannot recover
or fabricate the lost affine owner. App may select and borrow an already-proven configuration for
an entry point but cannot decode retained bytes or append a revision directly.

Admission persists the immutable `ConfigurationRef`. A newer global revision does not invalidate an
admitted revision and does not require Store to requalify it, subject to the unresolved currentness
choice above.

## 7. Target execution flows

### 7.1 New run

```text
typed domain C0
  -> Runtime constructs/retains ProvenValue<C0>
  -> Runtime validates the exact Program into ExecutableProgram
  -> genesis closure retains exactly that canonical mfm.program object
  -> Runtime reserves acknowledgement-custody capacity
  -> Journal encodes RunAdmitted
  -> Store.append_run
       Inserted
         -> qualify exact receipt/encoded genesis -> promote retained C0
       Existing for the byte-identical command
         -> qualify exact receipt/encoded genesis
         -> load/qualify/fold Current before execution or view
       Stale
         -> load/qualify/fold -> AlreadyAdmittedSame | AdmissionConflict | InvalidHistory
       AcknowledgementUnknown
         -> retain PendingAdmission; bounded identical retry or return Indeterminate
       UnavailableBeforeSubmission or CorruptPhysicalState
         -> retain PendingAdmission; return fail-closed retryable/corruption error
       permanent append StoreError
         -> consume PendingAdmission into typed rejection
  -> durable-success classification only
       -> advance_until_stable chooses and starts immediately actionable States
       -> Runtime returns the durable RunView
```

Only an exact durable qualification releases the admission successor. `Inserted` may continue from
its newly linearized head. `Existing` returns the original immutable receipt before head comparison,
so Runtime must obtain and fold `Current` unless an authoritative load separately proves that receipt
still current; it cannot re-enter callbacks from an old genesis receipt. A stale same-genesis race
likewise continues from the qualified durable fold, not an assumed local head. Conflict or invalid
history does not promote. Store never interprets `C0`, substitutes a Program, or selects the first
State.

### 7.2 Hot Pure State

```text
ExecutionCursor::ReadyPure<S>
  -> Runtime reserves acknowledgement-custody capacity
  -> Runtime invokes S with only S::Input (or a borrow)
  -> ProposedStateOutcome<S::Output, S::Failure>
  -> Runtime qualifies and retains the exact output/failure ProvenValue
  -> Journal encodes StateConcluded::Pure
  -> Store.append_run
       Inserted
         -> qualify exact durable frame -> promote retained typed successor
       byte-identical Existing
         -> qualify the exact durable frame
         -> load/qualify/fold Current before further execution or view
       Stale
         -> load/qualify/fold -> AlreadyConcludedSame | Conflict | InvalidHistory
       AcknowledgementUnknown
         -> retain PendingConclusion; bounded identical retry or return Indeterminate
       UnavailableBeforeSubmission or CorruptPhysicalState
         -> retain PendingConclusion; return fail-closed retryable/corruption error
       permanent append StoreError
         -> consume PendingConclusion into typed rejection
  -> durable-success classification only
       -> Runtime reducer chooses the next State or terminal result
```

No result is public and no successor is promoted before an exact durable-success branch. An
`Existing` receipt proves its candidate durable but not current; Runtime never invokes another State
from that historical receipt. There is no erase -> Store qualify -> `SelectedRun` -> Runtime
downcast loop.

### 7.3 Hot Access State

```text
ExecutionCursor::ReadyAccess<S, C>
  -> Runtime reserves one custody permit that can survive provider return
  -> Runtime invokes typed preparation with only S::Input (or a borrow)
       PreparationError
         -> release unused permit; return reviewed RunError with durable head unchanged
       success
         -> typed preparation produces C::Intent and fact request
  -> Runtime selects facts from one complete immutable authenticated Store snapshot
       authenticated required route absent / NotActionable
         -> release unused permit; return unchanged durable RunView::Runnable
       invalid or incomplete proof
         -> release unused permit; fail closed without append or provider entry
       actionable selection
         -> continue
  -> PreparedAccess<S, C>
  -> Journal encodes StatePrepared
  -> Store.append_run preparation
       Inserted
         -> transfer the permit and mint CommittedCall<S, C>
       Existing or Stale
         -> no call; load/qualify/fold durable history into waiting/replacement/conflict fate
       AcknowledgementUnknown
         -> retain PreparedAccess; no call; bounded identical retry or return Indeterminate
       UnavailableBeforeSubmission or CorruptPhysicalState
         -> retain PreparedAccess; no call; return fail-closed retryable/corruption error
       permanent append StoreError
         -> consume PreparedAccess into typed rejection; no call
  -> only the Inserted preparation branch continues
       -> typed adapter/capability ingress consumes CommittedCall<S, C>
       -> AccessResolution<S, C>
            Outcome(accepted evidence)
              -> ordinary typed State interpretation
              -> ProposedStateOutcome<S::Output, S::Failure>
              -> build typed conclusion
            BlockedIntegrity(accepted integrity evidence)
              -> do not invoke the ordinary State interpreter
              -> build the registered contract-fixed S::Failure conclusion
            Unresolved(classification)
              -> consume call authority; append no conclusion; release permit
              -> Effect: durable Waiting under ratified recovery policy
              -> Read: Runnable only for a permitted bounded replacement, otherwise Waiting
       -> for Outcome or BlockedIntegrity only
            -> Runtime qualifies one typed conclusion and correlated fact-proposal owner
            -> install it in acknowledgement custody before any further await
            -> Journal encodes coordinate-free StateConcluded::Access
            -> Store.append_run conclusion
                 Inserted
                   -> qualify exact durable frame/publication receipt
                   -> promote retained typed successor
                 byte-identical Existing
                   -> qualify the exact durable frame/publication receipt
                   -> load/qualify/fold Current before further execution or view
                 Stale
                   -> load/qualify/fold -> AlreadyConcludedSame | NoLongerSelected | Conflict |
                      InvalidHistory
                 AcknowledgementUnknown
                   -> retain TypedConclusion; bounded identical retry or return Indeterminate
                 UnavailableBeforeSubmission or CorruptPhysicalState
                   -> retain TypedConclusion; return fail-closed retryable/corruption error
                 permanent append StoreError
                   -> consume TypedConclusion into typed rejection
```

The flow does not fall through from any non-`Inserted` preparation outcome to provider entry. A
durable preparation found after acknowledgement loss supplies history, never the original
`CommittedCall`. The provider-to-conclusion interval transfers the same custody permit, so an
unknown conclusion acknowledgement cannot encounter a new table-capacity failure. Cancellation
before the preparation commit discards only the uncommitted candidate. Cancellation after an
inserted preparation but before/during provider ingress consumes call authority and releases the
permit while leaving that durable preparation. After accepted Outcome/BlockedIntegrity is
synchronously secured as `TypedConclusion`, cancellation returns its lease and permit to custody;
Unresolved instead releases both without a conclusion. No unresolved or integrity-blocked result
is silently treated as ordinary evidence.

### 7.4 Cold resume

```text
Runtime.resume(run_id)
  -> resolve relevant private pending owner once, or return Indeterminate
  -> Store.load_run(run_id, Current)
  -> PostgreSQL returns complete stored rows/bytes through one captured head
  -> Journal strictly decodes one JournalHistory
  -> Program decoder validates retained Program
  -> Runtime assembly associates stable declarations with concrete drivers
  -> Store/Journal/Runtime build exact configuration and fact/producer dependencies
  -> Runtime reducer folds the history and QualifiedRunDependencies once
  -> if execution is requested, RegisteredState.start reifies the selected input once
  -> ExecutionCursor::Ready* / waiting / parked / terminal
```

Store returns no semantic selection. PostgreSQL knows no State types.

### 7.5 Callback-free replay/status

```text
portable bundle or Store-loaded consumer/dependency bytes
  -> Journal qualification of consumer, configuration, and producer histories
  -> Runtime dependency correlation and pure reducer/inspection
  -> status, trace, terminal result, or inconsistency
```

The inspection path never invokes State or provider callbacks. It is the same inherently
callback-free semantic fold used by cold resume; live cursor materialization happens afterward,
not through a second reducer or an inspection-mode branch.

Semantic inspection requires the retained Program plus compatible semantic registrations from a
`RuntimeAssembly` because that assembly owns the exact value/capability codecs; it requires no
live adapter. Journal's portable semantic bundle contains the consumer history, genesis Program,
each exact configuration revision header and envelope, and, when facts were selected, the
authenticated index proof at the recorded `FactHead` plus the dependency-closed producer histories
required by `QualifiedRunDependencies`. Those physical proof artifacts establish the retained
selection snapshot but never become a conclusion-assigned field or alter semantic run hashing. A
run-only bundle with missing
dependencies produces `MissingDependencies`, not a best-effort `RunView`.

### 7.6 One process-facing contract

The conceptual public surface is:

```text
Runtime.start<T>(...)            -> Result<RunView, RunError>
Runtime.resume(run_id)           -> Result<RunView, RunError>
Runtime.read(run_id)             -> Result<RunView, RunError>
RuntimeAssembly.inspect(portable_bundle) -> Result<RunView, RunError>
```

Exact Rust naming is non-normative. `start` and `resume` share `advance_until_stable`; `read` and
`inspect` execute no State or provider work. A durable result returns `RunView`. Absence,
acknowledgement indeterminacy, conflict, invalid history, missing dependencies, incompatible
assembly, capacity, and infrastructure failure remain distinct errors. No method exposes a
one-State step, cursor, append owner, resolver token, or Store-selected action.

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
- fact stream references and tenant/scope/epoch configuration partitioning; the only retained
  configuration partition is the explicit domain/composition `ConfigurationInstanceId`;
- App's fixed-tenant facade and CLI/REST request or output fields;
- Program derivation/catalog inputs that include tenant identity; and
- tenant/scope/epoch inputs, columns, predicates, and hash components in EVM nonce/effect identity.

Global EVM sender/domain/operation identities, nonce-domain rows, operation rows, uniqueness, and
atomic nonce semantics remain. This cutover rekeys them away from removed authority namespaces; it
does not delete the nonce subsystem or weaken its replay/idempotency contract.

The generic English word “scope” may remain where it describes a lexical or domain concept, but no
Store authority namespace or `StoreScopeId` equivalent may survive under a new name.

Removing tenant does not permit secrets or private user data to become cross-run facts. Secret-free
persisted-surface rules remain unchanged. Product isolation, if ever required, must be designed as
an explicit deployment/access-control boundary rather than smuggled back as an identifier in every
core type.

## 9. Package and API cutover

### 9.1 `mfm-program`

- retain the immutable `State | Match` document and compile-time typed State contracts;
- persist one exact failure contract for every State, using the reserved Never contract for
  infallible States, and persist the complete Access association needed by `StateExecutionKey`;
- strictly decode retained Program bytes and validate graph shape and stable contract continuity;
- remove erased value/capability execution callbacks and catalog branding;
- leave Match tag/payload projection to Runtime's pure schema-derived value registrations;
- expose no general public erase/downcast workflow; and
- leave concrete implementation association to Runtime assembly.

### 9.2 `mfm-runtime`

- own the sole reducer after moving it out of Store;
- own `ExecutableProgram`, hot execution, cold resume, semantic inspection, typed configuration,
  and fact selection;
- derive exact instance/contract/schema configuration routing from an explicit
  `ConfigurationInstanceId`, never ambient Store identity;
- replace erased option bags with affine typed cursor states;
- own the sole private heterogeneous registry, complete `StateExecutionKey`,
  `RegisteredState.start`, and one `start_typed` downcast;
- own pure schema-derived Match projection and exact dependency-qualified folding;
- retain direct-new `CommittedCall` gating;
- own bounded `advance_until_stable`, the private pending-append table, and Runtime-derived
  `RunView`; expose typed configuration publish/exact-load/latest-load and same-request retry; and
- return no cursor/step/owner protocol to App.

### 9.3 `mfm-journal`

- own strict canonical frame/history and portable codecs;
- own sealed run-frame and configuration-envelope codecs plus portable semantic dependency bundles;
- make constructed/decoded frames valid by construction;
- remove identity and redundant record fields;
- remove `FactPublication` from conclusions; and
- expose persisted representations, not semantic Runtime selections.

### 9.4 `mfm-store`

- reduce to the mechanical Store contract and Memory implementation;
- remove Program/capability/catalog/reducer dependencies;
- remove open brands, port splits, semantic owner types, and duplicated validators;
- return complete-through-head bytes, authenticated fact-index proofs, exact/latest configuration bytes,
  and mechanical receipts only; and
- keep backend conformance focused on physical row validity, atomicity, CAS, idempotency, limits,
  and unknown acknowledgement.

### 9.5 PostgreSQL adapters

- implement Store directly rather than a second broad backend SPI;
- use the global schema and no active identity;
- allocate fact/configuration sequences transactionally;
- keep SQL row structs private; and
- perform no Program or capability validation.

### 9.6 App and transports

- remove tenant-scoped construction and request fields;
- generate or accept one global `RunId` for admission/retry;
- call Runtime for start, resume, read, and inspection and consume only `RunView`/reviewed errors;
- delete App's suspended map, one-State drive and resolution methods, frame-derived status, and
  matches over public Runtime lifecycle variants; and
- expose no Runtime lifecycle internals to current or future transports.

App is lifecycle-thin composition/dispatch: it may choose a registered entry point and retain or
borrow a Runtime-issued proven configuration, but it never constructs/decodes semantic proof or
owns execution, append, or lifecycle authority. CLI and HTTP are transport-only: they parse, call
the App/Runtime facade, and render the core-owned view/error contract.

This core cutover does not introduce the generic `ApplicationBuilder::entry_point` DSL or remove
the current EVM/Portfolio dispatch by itself. App may temporarily retain that domain composition
while its execution side moves entirely to Runtime's high-level contract. The deferred Followups
RFC owns generic registration, domain-dependency removal, and functional thin transports after
this core is complete.

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
QualifiedValue::try_downcast public flow
QualifiedValue cross-layer flow

ErasedValue cross-layer alias
DynamicOutcome
DynamicPreparationFailure
DynamicResolution option bag
DynamicCommit
erased DynamicPrepared / DynamicCall result flow
DynamicPure / DynamicAccess adapter layer
DynamicStateRegistration
TypedPrepared / TypedCall islands inside the erased protocol
PreparedExecution / OpenedPreparationCommit wrapper pair
QualifiedRecordedEvidence forwarding wrapper
AdmissionInput public dynamic wrapper
RunSession.selected
RunSession.latest erased slot
RunSession.drive State-at-a-time method
public RunSession coordination owner
parallel SpawnStep / ResumeStep / RuntimeStep lifecycle algebras
old public Runtime spawn/resume step surface
Parked / Terminal public lifecycle variants
App-owned SuspendedRun coordination
App suspended owner map
public PendingConclusion / SuspendedRun resolver protocol
AdmitRunRequest / AdmitRunResponse lifecycle wrappers
DriveResponse
PublicRunView
Application::drive one-State command
finish_runtime_step
resolve_suspended_run
retain_suspended
status_from_frames

StoreBrand
SemanticStore
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
tenant/scope/epoch components of EVM nonce/effect identity
```

The following vocabulary illustrates the principal replacements; it is neither a required public
type list nor a ceiling on private affine proof types:

```text
Runtime: RuntimeAssembly, ExecutableProgram, ProvenValue<T>, private ErasedProvenValue,
         StateExecutionKey, RegisteredState, StateStart, ExecutionCursor,
         QualifiedRunDependencies, ReducedRun, private PendingAppend custody, RunView
Journal: EncodedRunFrame, JournalHistory, EncodedConfigurationEnvelope,
         portable semantic bundle codec
Store: Store, RunAppend, AppendOutcome, AppendReceipt
Configuration: ConfigurationHead, immutable ConfigurationRef, PendingConfiguration<C>,
               ProvenConfiguration<C>
```

## 11. Security and failure semantics

The simplification preserves the existing high-risk invariants:

- run streams remain append-only;
- each append remains all-or-nothing;
- manifests, values, facts, outputs, and Journal objects remain content-addressed;
- structured hashes use canonical, float-free JSON;
- secrets never enter manifests, events, objects, facts, snapshots, outputs, or error details;
- provider IO occurs only through explicit adapters/capabilities;
- admission and recovery use exactly the retained content-addressed Program, never a current-code
  substitute;
- fact selection preserves exact bounded snapshot and producer provenance;
- no provider entry occurs before a newly inserted matching preparation;
- an unknown acknowledgement never mints provider-entry authority;
- private pending ownership conveys no durable authority and disappears on process loss;
- no successful result becomes public before its conclusion is durable or found identical; and
- replay performs no ambient IO or callback execution.

Removing scope/epoch/tenant removes no cryptographic or authorization boundary because those IDs do
not authenticate callers or encrypt rows. Deployment access control remains the responsibility of
the trusted embedding and PostgreSQL deployment boundary. It deliberately removes live-peer restore
fencing, so the stop-all-peers restore procedure in Section 6.3 is part of the safety contract.

PostgreSQL corruption or hostile row mutation is still treated as untrusted byte ingress on load.
Journal and Runtime fail closed with redaction-safe typed errors. Process-local typed proof is not a
substitute for retained-data validation after a restart.

The authenticated fact index/log assumes the trusted compiled Journal/Store transition code in this
architecture. Retained `FactHead` values are the proof anchors: membership, non-membership,
inclusion, and consistency witnesses detect omission, substitution, corruption, and cross-branch
mixing relative to those roots. They do not authenticate a Byzantine writer or add an external
signature. Malicious-writer resistance would require a separately approved validity/signing
contract, not silent expansion of this cutover.

## 12. Tests and verification contract

### 12.1 Compile-time and unit proof tests

- typed Runtime output reaches Journal encoding without Store requalification;
- no supported API can pair an arbitrary contract ref with an unrelated value;
- `ProvenValue`/`ErasedProvenValue` construction, erase, and downcast authority are not public;
- Pure and Access both enter through `RegisteredState.start` and one `start_typed` downcast;
- complete-key lookup includes exact failure and Access mode/fact/attempt metadata, has no partial
  or implementation-only fallback, exact collisions fail assembly, and distinct generic
  instantiations sharing one implementation reference remain distinguishable;
- every State persists an exact failure contract, including the reserved Never contract;
- occurrence bindings select a directly typed adapter inside monomorphic `StateStart`, with no
  erased adapter/downcast or duplicate State start;
- a schema-derived Match projector accepts every declared tag, rejects unknown/non-exhaustive tag
  tables and wrong payload contracts, and produces the exact canonical payload/value reference
  without invoking a State, adapter, provider, or authoring callback;
- contract/schema/registration/`TypeId` mismatch fails without panic and before callbacks;
- typed input, intent, evidence, and outcome remain correlated through preparation/conclusion;
- Access preparation error, authenticated `NotActionable`, ordinary outcome, integrity block,
  unresolved response, and cancellation before/during/after provider entry have exhaustive typed
  fates and conserve the single custody permit;
- a non-Clone typed value moves exactly once through hot entry and failure/Match routing;
- only `Inserted` can consume `PreparedAccess<S, C>` into `CommittedCall<S, C>`;
- `Existing`, `Stale`, and unknown acknowledgement cannot mint a call;
- unknown acknowledgement, retryable definite no-commit, or incomplete stale/existing
  classification retains the exact affine admission, preparation, conclusion, or configuration
  owner, and cancellation returns its lease; and
- hot advancement does not decode/reify Runtime's own just-produced output.

Use compile-fail tests where ownership, visibility, or consuming APIs are the proof mechanism.

### 12.2 Journal tests

- canonical fixtures for all three record families;
- exact-bound and bound-plus-one frame/object/history/configuration-envelope cases;
- non-canonical JSON, floats, hash mismatch, object collision, missing closure, sequence gap, and
  wrong recursive-head rejection;
- unreferenced extra frame/configuration objects are rejected as strictly as missing objects;
- exact genesis Program closure/reference checks and later-frame predecessor checks;
- configuration-envelope contract/schema routing-key, object-closure, and digest checks;
- configuration-revision-header sequence/predecessor/envelope/head-digest checks, including a forged
  historical head;
- fact genesis roots, route-key preimage, sparse leaf/node/update, publication descriptor/header,
  append-only log inclusion/consistency, and membership/non-membership multiproof goldens;
- wrong sparse/log root, predecessor/frontier, route order/duplication, proposal update set,
  publication inclusion, cross-branch consistency, omitted/extra result, duplicate/unused proof node,
  noncanonical default node, or alternate proof ordering fails structurally;
- portable semantic bundles require and round-trip exact configuration/fact/producer dependencies;
- one validation pass per decoded history;
- no identity or conclusion-assigned fact-publication-coordinate fields in canonical output while
  `FactSelection.frontier: FactHead` remains canonical; hostile frontier/root mismatch fails; and
- portable round trips through Journal only.

### 12.3 Runtime semantic tests

- Pure and Access hot paths preserve typed context;
- cold resume reifies retained values once and selects the same next State;
- `State | Match` sequential/fail-fast behavior;
- zero-State roots and values referenced only by Match declarations resolve explicit value
  registrations; cold resume/read/inspect agree on the valid Program's terminal result;
- preparation/conclusion race classification after reload, including exact retry, same semantic
  conclusion, replacement, conflict, and idempotency conflict;
- admission exact-ID retry, different-ID identical genesis, conflicting genesis, concurrent start,
  stale, cancellation, and repeated-unknown classifications;
- start and resume share one bounded progression loop, drain multiple actionable States, and stop
  only at a durable boundary before the next State start;
- every operational `RuntimeLimits` dimension accepts its exact bound and rejects or yields safely
  at bound plus one, including State starts, pending owners, provider ingress, active progression,
  and deterministic work; immutable Journal/configuration/fact proof format ceilings are tested
  separately;
- start, resume, read, and callback-free inspection agree on `RunView`, including runnable,
  waiting, exact terminal values, and routed nonterminal failures;
- retained-Program inspection succeeds without any live adapter and rejects a current-code Program
  substitute or incomplete portable dependency bundle;
- absence, acknowledgement indeterminacy, conflict, invalid history, missing dependencies,
  incompatible assembly, capacity, and infrastructure error remain distinct;
- cancellation at every append await leaves any affine owner in the bounded Runtime table;
- cancellation before preparation commit releases an unused permit; cancellation after an inserted
  preparation but before/during provider ingress consumes call authority and releases the permit;
  accepted provider output reaches typed-conclusion custody before another await, after which
  cancellation returns both its owner lease and transferred permit;
- fault injection before submission and after transaction submission but before acknowledgement
  distinguishes definite no-commit from unknown and preserves the same borrowed command/owner;
- fault injection after command-identical `Existing` or `Stale` but before current-run load,
  dependency qualification, or fold returns the exact owner lease to custody;
- the pending table supports several owner kinds/append IDs for one run, reserves capacity before
  work, transfers an Access permit through call/conclusion, resolves before new work, and never
  exposes a resolver token above Runtime;
- unknown preparation causes zero provider entry, and process loss cannot recreate a call;
- authenticated fact absence appends no preparation and returns the unchanged durable runnable
  view; integrity blocks never invoke ordinary State interpretation; unresolved Effect/Read
  results project only their mode-permitted waiting/replacement fate;
- a retained Read preparation permits only its declared bounded fresh replacement, while an
  entry-once Effect remains parked after owner loss;
- cold Read replacement and conclusion derive the exact persisted reservation key from run and
  preparation sequence without loading a semantic reservation owner;
- exact RunId admission identity distinguishes same-genesis retry, conflicting genesis, and a new
  independent repetition according to the ratified policy; and
- retained `RunAdmitted.entry_point` mismatch with the retained Program fails in Runtime, not
  Journal;
- authenticated fact dependencies reject omitted/reordered/index-tampered matches, wrong frontier
  digest, unadmitted sources, wrong subjects, hostile producer sequence/head/Program/configuration/
  proposal/value provenance, missing transitive producer history, non-decreasing frontiers, cycles,
  and every dependency-DAG bound plus one;
- an invalid latest route leaf fails closed and never falls back to an older publication;
- assembly mismatch fails before append or callbacks.

### 12.4 Store conformance tests

Run identical Memory and PostgreSQL tests for:

- exact-head CAS;
- dense per-run sequence;
- per-run append-request idempotency;
- request-key serialization and idempotency re-read before head comparison, including overlapping
  identical/conflicting IDs and an identical retry after later appends;
- durable command-digest equality over target, expected head, complete frame/object/publication
  attachment, reservation instruction, and receipt; each unequal field is conflicting reuse;
- atomic frame/object/membership/head/reservation/fact-header/index/log/
  append-request-digest/receipt writes;
- inserted/existing/stale/unknown outcomes;
- pre-submission and post-commit/pre-ack fault injection proves no-write versus
  acknowledgement-unknown behavior, and an identical retry after the latter returns the original
  durable receipt;
- run target/sequence/predecessor/expected-head correlation;
- complete-through-captured-head run loads and exact historical producer-prefix loads, with capacity
  rather than truncation;
- fact snapshot proofs capture/verify one frontier and return exactly one bounded result per query
  key, without head/query changes, omitted matches, cross-branch substitution, or false finality;
- sparse-index and append-only-log roots/nodes match exactly between Memory and PostgreSQL for the
  same canonical batch, historical roots remain provable, missing reachable nodes are corruption,
  and proof size is unaffected by unrelated publication count;
- configuration physical-head load, latest-by-instance-and-contract-through-head, exact `ConfigurationRef`
  load, append, exact retry, conflict, stale, and unknown outcomes;
- aggregate byte/reservation conservation, deduplicated object charging, Open/Replace/Consume,
  deterministic reservation-key correlation/restart, late-old consume rejection, stale no-op, and
  exact-retry no-op; and
- concurrent processes writing the same and different `RunId`s, including distinct first writers
  racing an absent run head.

No conformance test may require a Program catalog or `SelectedRun`.

### 12.5 PostgreSQL fact/configuration tests

- concurrent conclusions allocate at most one dense global fact coordinate and one exact
  index/log transition each;
- an identical retry returns the original coordinate;
- run frame, fact publication header, sparse/log nodes, head, append-request digest, and receipt roll
  back together;
- sealed publication attachments cannot disagree with the conclusion or object closure;
- duplicate routes or a partial/all-but-one multi-route update fail, and every node/header/head write
  rolls back with the run append;
- malformed physical fact ordering, head correlation, producer provenance, and snapshot bounds fail
  at their assigned Store/Journal/Runtime boundaries;
- later fact publications do not invalidate an earlier immutable selection;
- configuration revisions use one dense global head and idempotent append ID;
- overlapping identical configuration append IDs return `Inserted` then `Existing`, conflicting
  IDs fail without an incomplete request row, and distinct first publications serialize against
  the absent global head;
- configuration append request rows preserve command digest and immutable receipt across later
  revisions; hostile envelope routing/digest/head rows fail at their assigned boundary;
- envelope/object/revision/request-receipt/head configuration writes are all-or-nothing, and
  forged historical configuration-head sequence/digest pairs fail;
- Runtime is the only typed configuration constructor/decoder and Store never returns `C`;
- configuration acknowledgement cancellation/repeated unknown retain private custody, identical
  high-level publish resolves it, and conflicting retry arguments fail;
- latest-by-instance-and-contract selection does not return the latest revision of another
  instance or contract;
- admission remains bound to an older exact configuration revision after a newer one commits;
- database restart preserves all runs without identity activation/rotation;
- restore procedures require all old peers stopped and fresh processes afterward; no test or API
  claims live-peer ABA fencing without epochs;
- EVM sender/domain/operation nonce rows and uniqueness remain after tenant/scope/epoch rekeying;
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

This RFC is documentation only. After the six Material uncertainties are ratified, implementation
uses two ordered coherent code cutovers. Each includes every affected producer, consumer, test,
fixture, `docs/design.md`, `docs/architecture.md`, crate document, and deletion; verification and
documentation are not cleanup work.

1. **`cut over global journal and persistence contracts`**

   In one Program/domain/Journal/Store/Memory/PostgreSQL/Runtime/App/replay/configuration/fact/
   EVM-nonce cutover, introduce exact persisted State failure/Access associations, the private
   valid-by-construction lean frame/history/configuration-envelope/portable-bundle codecs, and the
   explicit predecessor; introduce mechanical reservation instructions beside Journal-sealed
   conclusion/publication attachments; allocate fact
   coordinates atomically without frame rebinding; make fact/configuration streams global with the
   ratified explicit configuration-instance routing; remove scope/epoch/tenant and identity
   rotation; rewrite baseline schemas and fixtures; and delete every
   old field, reader, hash, key, duplicated structural validator superseded by Journal, identity
   retry, and fact-rebinding retry protocol. Store may still own the existing semantic reducer, fact
   selection, and configuration
   qualification at this coherent intermediate point and is the temporary consumer of the new
   Journal-sealed payloads. Every Runtime/App/replay consumer is updated to that one persisted
   contract; there is one structural qualification path and no dual schema or legacy decoder.

2. **`move the complete semantic proof path into runtime`**

   In one inseparable Program/Runtime/Store/Memory/PostgreSQL/App/replay cutover, add the exact
   Runtime value/capability registrations, `StateExecutionKey`, `RegisteredState.start`,
   `ProvenValue`, `ExecutableProgram`, affine cursor, sole reducer, typed configuration and fact
   selection, bounded progression, private acknowledgement custody, `RunView`, cold resume, and
   callback-free inspection. Simultaneously replace Store with its mechanical API, migrate App to
   start/resume/read, delete `mfm-replay`, and delete Program
   execution callbacks, Store reduction/selection/semantic owners, cross-layer erased values,
   public lifecycle algebras, and App suspension/status logic. No intermediate commit may leave no
   semantic owner or two semantic owners.

For each cutover, run the narrow scope-selected checks first and expand as required by
`docs/build-and-verification.md`. After both are complete and narrower failures are resolved, record
the LOC/public-surface/dependency/absence evidence and run the composed CI gate once. If a public
cutover cannot compile halfway, its deletion and replacement belong in the same commit. Do not
maintain an erased and typed route, Store and Runtime reducers, old and new identity schemas, or
independent replay reducers in parallel. Do not begin the later Followups RFC until this RFC's
acceptance criteria pass.

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
3. `ExecutableProgram` resolves every declaration, exact failure/Access association, Match
   projector, and semantic binding exactly; live adapter availability is additionally required for
   admission and actual Access entry, never callback-free inspection.
4. Complete `StateExecutionKey` lookup has no implementation-only fallback; Pure and Access both
   enter through `RegisteredState.start` and one private `start_typed` downcast.
5. One private Runtime registry handoff is the only execution-layer type erasure.
6. A hot typed value is encoded once and never requalified by Store.
7. Each loaded consumer/dependency snapshot crosses exactly one Journal structural qualification
   and one Runtime semantic fold before use; an independent later load may create a new proof.
8. Start and resume share one bounded `advance_until_stable`; start, resume, and read expose one
   Runtime-derived `RunView` and no State-step lifecycle algebra.
9. Acknowledgement owners stay in one bounded, cancellation-safe, private Runtime table; App and
   Store never receive semantic retry authority, and process loss cannot mint provider entry.
10. Journal frames are valid by construction and have no public repeated validation choreography.
11. Store exposes only mechanical persistence operations and outcomes.
12. PostgreSQL atomically appends run frames, sealed global fact publication headers, and the exact
    sparse-index/append-only-log transition without modifying canonical conclusion bytes.
13. Direct-new typed gating remains the only path to `CommittedCall<S, C>`.
14. `StoreScopeId`, `StoreEpoch`, `TenantScopeId`, and equivalent ambient authority namespaces are
    absent from Rust APIs, JSON, hashes, SQL, transports, facts, configuration, replay, and EVM
    nonce keys; an explicitly supplied domain `ConfigurationInstanceId` is not authority identity.
15. `RunId` alone identifies a run within the database authority, and concurrent processes
    linearize through exact-head CAS; documented restore requires all pre-restore peers stopped.
16. Configuration and facts each use one global append-only PostgreSQL sequence; configuration
    latest lookup is partitioned by the ratified explicit instance/contract route; physical
    `ConfigurationHead` is distinct from immutable `ConfigurationRef`; Runtime is their sole
    semantic qualification/selection owner and Store handles only bytes and physical metadata.
17. Portable decoding lives in Journal and semantic replay/status uses Runtime's sole reducer with
    a compatible Runtime assembly; exact configuration revision headers; bounded authenticated fact
    membership, non-membership, inclusion, and branch-consistency proofs; a dependency-closed
    producer DAG; and no live callbacks.
18. The complete deletion checklist has no compatibility aliases or forwarding wrappers.
19. Authoritative design/architecture documents, fixtures, READMEs, and transport schemas describe
    only the new design.
20. Backend conformance, Runtime semantic, Journal canonical, PostgreSQL transaction, ownership,
    and absence tests pass under the repository's scope-selected verification policy.
21. Production Rust LOC and public coordination surface are net smaller than before the cutover.

The intended end state can be summarized in one sentence:

> Runtime decides and retains typed meaning, Journal proves bytes, and PostgreSQL proves atomic
> persistence; no layer erases another layer's proof merely to reconstruct it later.
