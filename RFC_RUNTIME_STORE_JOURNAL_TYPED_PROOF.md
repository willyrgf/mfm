# RFC: establish the core Runtime, Journal, and Store proof path

Status: architecture review complete; implementation planning blocked on one product-scope choice

This RFC is the clean-slate target for the MFM core proof path. It replaces the conflicting runtime,
storage, replay, configuration, fact, tenant, scope, and writer-epoch contracts in
RFC_REFACTOR_SINGLE_TRUST_BOUNDARY.md, docs/design.md, and docs/architecture.md. Implementation must
update those authoritative documents in the same cutover so the repository ends with one current
contract.

There is no compatibility period, legacy decoder, dual schema, or fallback execution path.

Relationship to `RFC_FOLLOWUPS_FROM_REFACT_RUNTIME.md`: this RFC adopts only that document's
durable failure product—typed failure propagation/handlers lower to ordinary State continuations,
and entry points have exact root success/failure contracts. It deliberately does not import the
deferred Operation/Application DSL, State-only Program proposal, or FailureNext::ByVariant. The
follow-up remains non-authoritative rebase source, not implementation input, and must later rebase
its authoring layer onto this retained State-or-Match graph. This RFC freezes the durable lowering target only; its
implementation does not add OperationExpansion or a failure-policy authoring DSL.

## Decision

The target has four semantic owners:

| Owner | Sole responsibility |
| --- | --- |
| Program | The immutable State-or-Match graph, exact root success/failure contracts, typed outcome continuations, and exact persisted value, State, capability, and binding associations |
| Runtime | Program association, the sole semantic reducer, exact registered State entry, typed execution, graph-bounded caller-driven progression, and RunView |
| Journal | Exact canonical run-frame wire, content addressing, structural qualification, recursive heads, frame-local object closure, and fixed format limits |
| Store | Mechanical complete loads, exact-head compare-and-append, idempotent physical equality, actual capacity accounting, and atomic PostgreSQL or Memory persistence |

Application and transports parse requests, supply explicit identities, call Runtime, and render
reviewed outputs. They own no execution session, pending append, recovery token, reducer, or retained
frame interpretation.

One PostgreSQL run database is one run-history authority. RunId is its complete durable run
namespace. The core contains no tenant, Store scope, writer epoch, active deployment identity,
identity rotation, or replacement incarnation concept.

Facts are not part of this target. No current admitted capability consumes prior-run facts, so the
fact selection/publication/proof system is deleted rather than rebuilt speculatively.

Independently published configuration is not part of this target. Domain material required to
reproduce a run is retained in the Program or typed admitted context `C0`; deployment clients,
credentials, and other live bindings remain process-local in RuntimeAssembly and adapters. The
MfmConfig contract, configuration streams, latest lookup, and configuration persistence are
deleted.

Portable bundles and offline inspection are not part of this target. No current core consumer
requires a second complete-history ingress or export format. Runtime reads and qualifies durable
history only through Store. A future portability feature requires a consumer-driven RFC rather
than reserving another wire surface now.

The recommended product scope retains Portfolio snapshots and their EVM Read capabilities but
retires the in-process EOA transaction-submission entry point. The current split nonce-reservation,
signing, broadcast, and status flow has no durable owner that can prevent a reserved nonce gap after
process/provider failure. This RFC does not disguise that missing authority as an operational
assumption. The same cutover deletes generic Effect: nonce reservation and broadcast are its only
production consumers, and retaining synthetic-only mutation semantics would constrain the future
transaction authority without a current need. The core execution algebra is Pure or externally
observational Read.
If submission remains a milestone requirement, its durable transaction authority and Runtime
interaction are a separate prerequisite RFC as described under Material uncertainties.

Runtime futures are cancellation-safe without a cancellation concept. Dropping start or resume
loses only volatile Pure/Read work: Pure recomputes, a Read may be observed again, and a possibly
committed Store append is resolved by the next complete load. Runtime adds no cancellation token,
cleanup branch, detached completion task, pending-result custody, or Runtime-owned semaphore.
Synchronous State work is awaited through spawn_blocking only to protect the async executor.
Adapters and Store own waits, timeouts, pools, and explicit results while their future remains live.
After process termination, durable history is the only recovery authority.

## Material uncertainties

1. **EVM transaction submission and Effect milestone scope.** The recommended target retires
   `mfm.evm/submit-transaction@1`, deletes its now-unconsumed generic Effect mode, and keeps
   Portfolio snapshots plus their EVM Reads. This is uncertain because App, repository
   documentation, and integration tests currently advertise the in-process submission entry point
   even though no CLI/REST route or production composition uses it. If removal is unacceptable,
   the nonce-only design in the prior draft is unsafe: a crash or rejection after allocating nonce
   N can strand the sender while higher nonces continue to be issued. Keeping submission therefore
   requires a separate durable per-sender transaction authority/outbox that owns nonce allocation
   through exact signed-byte custody, retransmission, reconciliation, and higher-nonce fencing; it
   must also define its Runtime mutation contract rather than inherit synthetic one-entry/permanent-
   parking behavior. Sender-retirement wording or nonce-receipt recovery is not sufficient. Resolve
   by obtaining explicit product-owner approval either to defer the entry point and Effect in this
   cutover or to fund that separate outbox RFC before implementation planning.

All other choices and validation results are frozen:

- dropping start/resume is safety-neutral and loses only volatile Pure/Read work; Runtime has no
  cancellation API, cleanup branch, detached completion, or Runtime-owned semaphore;
- Application/composition may bound top-level concurrency, while Adapter and Store own live-IO
  wait control;
- the execution algebra is Pure or externally observational Read; EffectMode and every
  mutating-provider rule are
  absent until a real transaction-authority consumer defines them;
- blocking State/codec callbacks are trusted, bounded-input, pure, and terminating;
- facts, independently published configuration, portable bundles, tenant, scope, epoch, identity
  rotation, and database incarnation are absent;
- every frame carries all canonical objects directly named by its record; there is no
  history-dependent first-reference dictionary, object-count limit, or object reservation;
- every frame-embedded Program/value object is at most 8,388,608 canonical bytes, so the largest
  three-object Read conclusion is representable without per-State sizing;
- exact qualified record equality uses the closed record fields and content references directly;
  there is no separate record digest;
- Program persists only opaque binding_ref; sealed domain constructors derive it from secret-free
  public descriptors, and trusted composition's pairing of an opaque client with that public target
  is part of the adapter TCB;
- trace and adapter-audit DTOs are not core Runtime APIs;
- independent execution uses a fresh caller-supplied RunId;
- the current CLI and REST binaries have no run-progression route, and the current in-process
  App-derived RunId path is replaced rather than preserved. Any future start route accepts and
  echoes an explicit RunId before execution; exact admission retry reuses that RunId and the same
  Program/context;
- supported PostgreSQL recovery is one monotonic locally durable timeline. Writable rewind,
  acknowledged-commit-losing failover, and writable historical clones are outside the safety
  claim.

## 1. Problem situation

### 1.1 Internal layers discard typed proof

The domain surface begins with exact associations:

- State input, output, and failure types;
- Read capability intent and evidence;
- typed ProposedStateOutcome;
- typed values that already passed canonical qualification.

The current implementation repeatedly turns those associations into contract references, canonical
strings, TypeId, Box of Any, option bags, catalog brands, Store brands, and backend DTOs. Later
layers compare the same pieces and downcast them again.

That is useful only at a real trust or heterogeneous dispatch boundary. Inside one trusted process
it creates more invalid combinations, validators, translations, public types, and future change
sites than the proof requires.

### 1.2 Store became the semantic state machine

Store currently opens under identity brands, qualifies Journal history, retains and reifies the
Program, reduces State-or-Match, selects the next action, qualifies configuration and facts,
constructs semantic owners, and then maps those owners onto a second backend vocabulary.

The resulting cold path is backwards:

~~~text
PostgreSQL rows
  -> backend DTOs
  -> Store qualification
  -> Store Program reduction and next-action selection
  -> Runtime reconstruction
  -> State execution
~~~

Persistence should return retained bytes. Runtime should interpret those bytes under the retained
Program and decide what runs next.

### 1.3 Journal mixes wire data with process-local proof

Journal records must be canonical and append-only, but open DTO constructors and repeated validate
methods make every consumer re-decide whether the same bytes are structurally valid. Construction,
Store append, Store load, replay, and Runtime each repeat parts of the same validation.

Journal needs two opaque directions instead:

~~~text
trusted typed inputs -> Journal encoding -> EncodedRunFrame
untrusted retained bytes -> Journal decoding -> JournalHistory
~~~

Store must not reopen either proof.

### 1.4 Duplicate registries describe one association

Program currently owns value/capability reifiers and TypeId associations while Runtime owns State
and adapter registrations. Store uses the first registry before Runtime uses the second.

Only two representations are necessary:

- stable contract and implementation associations in the persisted Program; and
- one trusted RuntimeAssembly associating them with concrete Rust implementations.

### 1.5 Runtime lifecycle leaks into Application

Application currently drives one State at a time and retains suspended Runtime/Store owners. It
infers status from frames and participates in acknowledgement recovery.

Runtime already owns all information needed to advance or inspect a run. Application should see
only start, resume, read, RunView, and reviewed errors.

### 1.6 Unused protocols dominate the core

The current fact surface and the proposed authenticated fact redesign introduce selection modes,
proposal sets, publication coordinates, global heads, Merkle structures, dependency DAGs, proof
limits, global serialization, and portable dependency material. Every reachable production
capability uses no prior facts and every current success proposes an empty set.

The target removes that protocol. A future consumer-driven fact RFC may introduce a new clean
format only after defining its real key, multiplicity, freshness, retention, threat model, and
operational envelope.

The current independently published configuration protocol similarly adds a second value
lifecycle, latest-versus-exact lookup, CAS, revisions, SQL, retry behavior, and admission coupling.
No retained domain value needs that lifecycle: reproducible policy belongs in Program or `C0`, and
live deployment material belongs in assembly/adapters. The target deletes the protocol instead of
preserving a unit or empty configuration.

Portable bundles add another complete-history wire, decode budget, and semantic ingress solely for
an unused export/inspection surface. Store-backed Runtime read is the one supported inspection
path. Portability may return only with a concrete external consumer and its own threat and
transport contract.

### 1.7 Persisted identities model authorities outside the product

StoreScopeId, StoreEpoch, TenantScopeId, and StructuredStoreIdentity flow through frames, hashes,
SQL, facts, configuration, App, and replay even though the selected product has one database
authority and no in-core tenant isolation model.

Their removal is a semantic deletion, not a rename. The admitted authority is one no-loss monotonic
database timeline. Writable rewind or clone is unsupported even after old processes stop; the core
does not pretend an in-database identifier can fence a history that was itself rolled back.

## 2. Architecture and dependency direction

### 2.1 Target dependency graph

| Crate/layer | May depend on |
| --- | --- |
| Values/ids | foundational parsing, schema, and canonical primitives only |
| Program | Values/ids and capability contracts |
| Capabilities | Values/ids |
| Journal | Values/ids canonical primitives and its own record types |
| Store trait | ids and opaque Journal representations |
| Memory/PostgreSQL | Store trait, Journal's sealed transfer types, and storage-driver primitives |
| Runtime | Values, Program, Capabilities, Journal, and Store trait |
| Domain operations/States | Values, Program, and Capabilities; never Store |
| Adapters | Capability/Runtime ingress plus reusable transports/signers |
| App/transports | Runtime plus explicitly composed domains/adapters |

Program does not depend on Runtime or Store. Journal depends only on stable ids, canonical/value
representation, and its own record types. Store depends on stable ids and opaque Journal
representations, never on Program, State, capabilities, domains, or Runtime selection types.
Runtime receives retained bytes only from Store, passes them through Journal structural decoding,
and then performs semantic reduction.

The concrete Memory and mfm-storage-postgres implementations may depend directly on mfm-journal for
EncodedRunFrame and StoredRunBytes; that edge already points downward and requires no mfm-store
reexport or raw-byte adapter.

### 2.2 Validation ownership

Validation remains where trust changes:

| Proposition | Owner and result |
| --- | --- |
| Transport request is bounded and syntactically valid | App/transport typed request |
| A value is canonical, float-free, contract-bound, and content-addressed | Runtime value registration yields its one private proven-value representation |
| A Program document is canonical and structurally valid | Program decoder yields Program |
| A Program is completely supported by one process assembly | Runtime yields ExecutableProgram |
| A frame is exact canonical target wire | Journal yields an opaque qualified type |
| A retained run is a complete structurally valid hash chain | Journal yields JournalHistory |
| A history is semantically valid under its retained Program | Runtime fold yields a private reduced state or RunView |
| Provider evidence matches the exact qualified Read intent | Typed adapter plus capability bind |
| The expected durable head remains current | Store transaction |
| A persisted row set matches its canonical bytes and physical metadata | Store load plus Journal qualification |

The following repeated validations disappear:

- Store reifying Program values before Runtime does so again;
- Store reducing the Program before Runtime reconstructs the selected State;
- backend DTO layers reopening Journal frames;
- replay maintaining another reducer;
- catalog and Store brands standing in for exact type association;
- process-local identity fields rechecked in every owner;
- fact completeness and publication checks for an unused protocol; and
- App inspecting frames to recover Runtime status.

### 2.3 Concepts that remain intentionally private

Runtime may use small private enums or local variables for fold state and one active State driver.
Their exact names are not architecture. They must not become public lifecycle algebras, persisted
owners, Store inputs, App tokens, or another erasure protocol.

Here, private means its construction/lifecycle authority is sealed; opaque means its representation
and inspection surface are hidden. A type required in a cross-crate signature may be public with
private fields and the exact checked constructor assigned by this RFC. In particular,
StoredRunBytes deliberately has a public checked Store-implementation constructor but no raw getter.
The RFC does not require a crate merge or another erased wrapper merely to hide representation.

There is no Runtime pending-owner table of any kind.

## 3. Program, values, and Runtime assembly

### 3.1 State failure contract and Never

The core State contract is:

~~~text
State {
  Input: MfmValue
  Output: MfmValue
  Failure: MfmValue
  stable State implementation identity
}
~~~

Failure no longer implements a framework FailureValue constructor. Infrastructure, capacity,
assembly, and Store errors remain Runtime errors; the framework does not add default variants to a
domain failure contract.

The framework owns one uninhabited Never type. Its exact semantic identity is namespace
`mfm.kernel`, name `never`, version `1` (human shorthand `mfm.kernel/never@1`, not a persisted
display string), with the existing
Sha256JcsV1 semantic digest over the UTF-8 bytes `semantic:mfm.kernel:never:1`. Its schema name is
`mfm.kernel.never`, schema version is `1`, and its complete value shape is the existing externally
tagged Enum shape with zero variants. A hand-written framework registration uses the existing
framework-value descriptor path; the exact semantic identity, canonical SchemaIdentity bytes,
derived SchemaId, and nominal contract ContentRef are frozen by goldens.

The resulting exact persisted identities are:

~~~text
semantic:mfm.kernel:never:1:sha256-jcs-v1:
527c198ca1e6170ddf8966dd86ebd418b62226f14f6ef34d7b7ddd7b91a36835

schema:mfm.kernel.never:1:sha256-jcs-v1:
00e01bfecb60f575bba12886041598ad758074b704806a8af2c036172c01a4d8

nominal content digest content:sha256-v1:
804c7a33a2bc23a692444fcc2833f71d96f8315e6529523a7f884e13e6559927
~~~

The exact canonical SchemaIdentity bytes whose digest appears above are:

~~~json
{"canonicalization":"sha256-jcs-v1","encoding":{"kind":"canonical_json","shape":{"kind":"enum","tagging":{"kind":"external"},"variants":[]}},"persisted_surface":{"numbers":"no_floats","secrets":"no_secrets"},"schema_kind":"value","schema_name":"mfm.kernel.never","schema_version":"1","semantic_type_id":"semantic:mfm.kernel:never:1:sha256-jcs-v1:527c198ca1e6170ddf8966dd86ebd418b62226f14f6ef34d7b7ddd7b91a36835","versioning":"manual_version"}
~~~

That exact top-level framework registration is the only path that admits the empty descriptor;
generic, foreign, and nested empty Enum descriptors remain invalid. Every canonical value fails
validation against Never, and an empty Rust enum with an always-failing Deserialize implementation
has no encoder input. SchemaIdentity validation has one exact root exception for this complete
identity with external tagging and an empty variant vector; ordinary SchemaShape validation still
rejects every foreign or nested empty Enum. RuntimeAssembly installs its value registration
automatically; no new SchemaShape variant or generic allow-empty flag exists.

mfm-program owns and exports the Rust Never type beside State, PureState, and ReadState. mfm-values
owns only the exact reserved root SchemaIdentity validation exception and does not export a second
Never/schema wrapper. No other crate redefines or compatibility-reexports it.

- Pure and Read States may use Never as their exact failure contract when their typed implementation
  can produce only success.
- A retained failure outcome for any State whose failure contract is Never is invalid history.
- Program permits Never only as a State failure or root-failure contract. It rejects Never as
  admitted context, State input/output, root success, or any Match selector contract.
  RuntimeAssembly installs no Match projection for it.

Read capabilities carry every durable provider classification, including an integrity block, in
their own closed Evidence type. The ordinary typed State interpreter maps that evidence to its
declared success or failure; Runtime owns no framework integrity-failure callback or default
failure variant.

The remaining capability contract is Read-only:

~~~text
ReadCapabilityContract {
  Intent: MfmValue
  Evidence: MfmValue
  contract_id() -> mfm_capabilities::Result<StableId>
  bind_evidence(&Intent, &Evidence) -> mfm_capabilities::Result<()>
}
~~~

The public trait is `Send + Sync + 'static` and lives in mfm-capabilities. That crate retains one
redaction-safe `CapabilityError = InvalidContract | EvidenceBinding` plus its Result alias.
`contract_id` is the sole stable identity input. A State or capability identity failure while
checked declaration authoring derives implementation references maps to
ProgramError::InvalidContract. The same static failure during assembly registration or Program
association maps to RuntimeError::IncompatibleAssembly. A hot bind failure is an unexpected trusted
adapter/capability violation: Runtime appends nothing, leaves the durable State Runnable, and
returns Internal. The same bind failure in retained history is InvalidHistory. There is no mode
validation, evidence marker, attempt bound, fact hook, or second capability error surface.

There is no AccessMode, ReadMode marker, EffectMode, Facts associated type, or separate evidence
marker trait. The stable capability association plus the explicit intent/evidence contracts in each
Read declaration freeze the complete ABI.

State behavior is owned beside the State contract in mfm-program rather than by public Runtime
callback-holder wrappers or domain-local duplicate traits. The exact public shape is:

~~~text
ProposedStateOutcome<O, F> =
    Success { output: O }
  | Failure { failure: F }

PureState: State {
  evaluate(Input) -> ProposedStateOutcome<Output, Failure>
}

ReadState<C: ReadCapabilityContract>: State {
  prepare(&Input) -> Result<C::Intent, ReadPreparationError>
  interpret(Input, &C::Evidence) -> ProposedStateOutcome<Output, Failure>
}
~~~

`ProposedStateOutcome` has exactly those two variants and no fact or metadata field.
`ReadPreparationError` is a zero-detail, redaction-safe unit error for an unexpected failure of
trusted deterministic preparation. It maps to RuntimeError::Internal, enters no adapter, appends
nothing, and leaves the durable State Runnable. An expected local/domain rejection must be modeled
by an ordinary preceding Pure guard or by accepted typed Evidence and the Read State's ordinary
Failure outcome, not by ReadPreparationError. A finalized assembly admits exactly one Pure-or-Read
behavior registration for one State implementation reference; implementing both traits does not
create two selectable meanings for the same reference.

### 3.2 Persisted Program associations

Program retains one strict immutable State-or-Match document with:

- one entry point;
- one exact admitted-context contract;
- one exact root-success contract;
- one exact root-failure contract; and
- its bounded declarations.

Every State declaration persists:

- stable State implementation reference;
- exact input, output, and non-optional failure contracts;
- Pure or Read kind;
- for Read, the exact capability, intent, and evidence contracts plus an opaque content-addressed
  adapter binding whose domain-owned descriptor commits every required public target identity;
- one optional success successor index; and
- one optional common failure successor index.

Absence of a success successor means terminal success and requires the State output contract to
equal the Program root-success contract. Absence of a failure successor means terminal failure and
requires the State failure contract to equal the Program root-failure contract. Never is the sole
exception: it has no failure successor because no failure value exists. A present success successor
may name a later reachable State or Match whose exact input/selector contract equals the complete
output contract. A present failure successor must name a later reachable State whose exact input
contract equals the complete failure contract; it cannot target Match directly.

A failure successor receives the complete unchanged failure value: the same contract, content
reference, and canonical bytes. A typed mapper such as Portfolio's MapEvmBalanceFailure is an
ordinary Pure State. Its own failure may be the Program root failure, while a future real recovery
may produce its declared output and rejoin ordinary success. Runtime owns no mapping callback or
special failure record.

A future variant handler can first use an ordinary Pure router State whose input is that complete
failure and whose output is a closed route sum carrying the complete failure in each arm, then use
the already-retained Match declaration. No variant-specific failure edge is needed.

`C0` is the domain-owned typed admitted-context value and the first complete State context, not a
transport request wrapper. Every secret-free domain input needed to reproduce this run but not fixed
by the Program is carried in `C0`. A zero-State Program is permitted only when its admitted-context
and root-success contracts are identical and its root-failure contract is Never.

Program validates graph shape, reachability from declaration index zero, success and failure
contract continuity, root terminality, Match arm syntax/sorted uniqueness, Never reachability, and
fixed graph/size limits. Every edge must be in bounds and strictly greater than its source index, so
every reachable outcome path is acyclic and terminates without a separate cycle protocol. Program
cannot prove Match schema exhaustiveness without the registered value descriptor. Program
performs no State execution, provider IO, Store access, TypeId lookup, or erased value reification.

The later Operation authoring RFC must lower lexical success, Propagate, and typed failure handlers
into these existing declarations and edges. Operations and policies remain authoring-only and never
enter Runtime, Journal, or Store. This core cut does not add State-only Program bytes,
FailureNext::ByVariant, or another variant-routing protocol: no current operation requires them, and
the existing common mapper preserves the current Portfolio contract.

ProgramCatalog, catalog branding, public value reifiers, erased capability callbacks, and
process-local catalog identity are deleted.

The two retained implementation-reference derivations stay byte-for-byte at version 1 so this
cutover does not invent another identity descriptor:

~~~text
state_implementation_ref::<S>() = ContentRef {
  schema: SchemaId("mfm.state-implementation", "1", Sha256JcsV1, 32 zero octets),
  digest: SHA-256(UTF-8(S::state_id()))
}

capability_contract_ref::<C>() = ContentRef {
  schema: SchemaId("mfm.capability-contract", "1", Sha256JcsV1, 32 zero octets),
  digest: SHA-256(UTF-8(C::contract_id()))
}
~~~

State and capability ID construction remains strict before hashing. Concrete preimage/reference
goldens freeze both derivations. Neither preimage has a prefix, separator, newline, JCS envelope, or
associated ABI. The explicit input/output/failure and intent/evidence contract
references in Program are still mandatory: these v1 implementation references alone do not commit
the associated value ABI, so removing the explicit contracts would permit same-ID ABI drift.

Reference fixtures include exact State ID `mfm.test.identity/state@1` mapping to content digest
`content:sha256-v1:fab16c95f74ec061a9482105a20937043dd820e983a35ae63d235d69e53c540a`
and exact capability ID `mfm.test.identity/read@1` mapping to
`content:sha256-v1:70c931a7ae3e52ce943296347e6a3f0ab04425cfb8a390428145ff34753ced54`.

### 3.2.1 Exact Program wire

The clean cut assigns this document persisted-contract schema identity
`mfm-program-document@2`; version 1 has no decoder. `Program` is the sole public checked,
private-field type produced by trusted construction or strict decode. `ProgramDocumentV2` below is
the normative wire projection, not a second public DTO, ingress wrapper, or independently valid
type. Its exact canonical projection is:

~~~text
ProgramDocumentV2 {
  entry_point_id: EntryPointId,
  admitted_context_contract_ref,
  root_success_contract_ref,
  root_failure_contract_ref,
  declarations: [Declaration]
}

Declaration =
    { kind: "state", value: StateDeclaration }
  | { kind: "match", value: MatchDeclaration }

StateDeclaration {
  state_implementation_ref,
  input_contract_ref,
  output_contract_ref,
  failure_contract_ref,
  execution,
  next_index: null | u16,
  failure_next_index: null | u16
}

Execution =
    { kind: "pure" }
  | {
      kind: "read",
      capability_contract_ref,
      intent_contract_ref,
      evidence_contract_ref,
      binding_ref
    }

MatchDeclaration {
  selector_contract_ref,
  variants: [MatchVariant]
}

MatchVariant {
  tag,
  entry_index: u16
}
~~~

All shown fields are mandatory; null is encoded literally and no field is omitted. JCS sorts object
keys. Match variants are strictly increasing and unique by the stable tag's raw ASCII/UTF-8 byte
sequence; there is no locale, case folding, or numeric comparison. Declaration array position is the sole
control identity and is content-identity-bearing; trusted construction preserves its deterministic
topological order, and a different valid sibling order is a different Program. No address field,
sorting comparator, or topological canonicalizer exists. `failure_contract_ref` is never null.
`binding_ref` is a strict content reference to the adapter domain's canonical, secret-free binding
descriptor; the descriptor itself is not a Program field or Journal closure object.

`entry_point_id` is the exact bounded semantic EntryPointId used by Application dispatch, not a
control index. Application verifies its selected dispatch EntryPointId equals this field before
calling Runtime; Runtime receives no second selector, and RunAdmitted does not duplicate it. For a
non-empty Program, declaration index zero is the execution entry. Every
success, failure, and Match-arm target is an in-bounds u16 index strictly greater than its source;
every declaration must be reachable from zero. The valid empty zero-State identity Program has no
control entry and succeeds directly from C0 under the rule above.

Every Match arm entry_index names a later reachable State. ExecutableProgram association obtains the
payload contract for that exact tag from the registered selector descriptor and requires it to equal
the target State's input contract. Neither payload nor continuation contract is persisted again: the
selector descriptor and target State already declare them.

The sole cross-crate authoring/ingress paths are a full-validating
`Program::new(entry_point_id, admitted_context_contract_ref, root_success_contract_ref,
root_failure_contract_ref, declarations) -> Result<Program, ProgramError>` and strict
`Program::decode_canonical(&[u8]) -> Result<Program, ProgramError>`. The public authoring types have
private fields except for the closed `Declaration = State(StateDeclaration) | Match(MatchDeclaration)`
sum. Their exact public checked construction surface is:

~~~text
Execution::pure()
Execution::read(capability_contract_ref, intent_contract_ref, evidence_contract_ref, binding_ref)
StateDeclaration::new(
  state_implementation_ref,
  input_contract_ref,
  output_contract_ref,
  failure_contract_ref,
  execution,
  next_index,
  failure_next_index
) -> Result<StateDeclaration, ProgramError>
MatchVariant::new(tag: StableId, entry_index: u16) -> MatchVariant
MatchDeclaration::new(selector_contract_ref, variants)
  -> Result<MatchDeclaration, ProgramError>
~~~

These constructors accept exact invariant-bearing references rather than generic S/C/B types.
Trusted authoring derives State/capability references through the retained public v1 helpers and a
domain binding reference through that descriptor's checked content-ref method; full Program
validation and later RuntimeAssembly association reject a mismatched combination. The live adapter
builder still never accepts a caller-asserted binding ref. Program supplies read-only canonical
bytes and its ordinary ContentRef to Journal; there is no ProgramBuilder lifecycle, ProgramIngress,
ProgramDocument value, or ProgramRef wrapper.

MatchDeclaration::new sorts arms with the exact raw-byte comparator and rejects duplicates;
decode_canonical rejects an out-of-order retained arm vector rather than normalizing retained bytes.
Declaration array order is never normalized.

`ProgramError` is one private-detail, redaction-safe public enum with exactly `Canonical`,
`InvalidContract`, and `Capacity`. Canonical covers failure to decode the exact canonical wire;
InvalidContract covers invalid identities, declarations, graph shape, and contract relationships;
Capacity covers only the fixed Program count and byte ceilings. The old catalog- and
value-registration-specific variants are absent.

The old `root_contract_ref`, `terminal`, `maximum_conclusion_bytes`, `total_attempt_bound`,
`fact_selection_required`, `effect_domain`, and separate optional `execution_binding` fields are
absent. Pure carries no binding fields. Program admits at most 65,536 declarations and at most
65,535 States, so every target fits u16 and genesis plus every State conclusion on a path fits the
fixed frame-count envelope. The 256-arm Match limit, strict unknown-field rejection, and no-float
rules remain. Program construction and retained decode both enforce the 8 MiB Program-object ceiling
from Section 5.5.

### 3.3 One Runtime assembly

RuntimeAssemblyBuilder owns:

- one exact value codec per stable value contract, schema, and Rust type association;
- one exact State implementation registration per semantic State association;
- one exact typed adapter callback per private capability/binding-ref association;
- one pure schema-derived Match projection for each registered closed-sum value.

Its sole public contribution methods are:

~~~text
RuntimeAssemblyBuilder::new() -> RuntimeAssemblyBuilder
register_value<T: MfmValue>() -> Result<(), RuntimeError>
register_pure<S: PureState>() -> Result<(), RuntimeError>
register_read<S: ReadState<C>, C: ReadCapabilityContract>() -> Result<(), RuntimeError>

register_adapter<C, B, F>(binding: B, callback: F) -> Result<(), RuntimeError>
where
  C: ReadCapabilityContract,
  B: MfmValue,
  F: for<'a> Fn(&'a C::Intent)
       -> Pin<Box<dyn Future<Output = Result<C::Evidence, ReadUnavailable>> + Send + 'a>>
     + Send + Sync + 'static

finish(self) -> Result<RuntimeAssembly, RuntimeError>
~~~

Every builder/finalization rejection uses RuntimeError::IncompatibleAssembly with any diagnostic
retained privately; registration never returns a run-state error such as Absent, Capacity, or
Indeterminate.

`register_pure` automatically registers or exact-deduplicates S::Input, S::Output, and S::Failure.
`register_read` does the same and also registers or exact-deduplicates C::Intent and C::Evidence.
`register_value` remains only for otherwise-unreferenced roots, Match contracts, and zero-State
Programs; Never is automatic. Exact duplicate registrations are no-ops, while a stable identity
paired with a different schema/Rust association is rejected. This idempotence applies to value and
static State contributions only. Every duplicate adapter key `(capability_contract_ref,
binding_ref)` is rejected because two live callbacks have no meaningful equality relation. There is
no public PureImplementation, ReadImplementation, State-driver, codec-registration bundle, or
callback-holder wrapper.

The binding descriptor B passes through the same bounded, known-secret-marker-rejecting MfmValue
qualification path before Runtime derives and retains the private adapter association. It is not
auto-registered as a run-value codec and is never a Journal object; there is no second looser
binding hash path. Registering one State reference as both Pure and Read, or against two different
capability signatures, is an inconsistent State association and is rejected.

Adapter registration receives the validated canonical domain descriptor, not a caller-asserted
binding_ref. Runtime derives its content reference internally and keys the callback by the exact
capability/ref pair. The supported EVM live constructor passes EvmPhysicalTarget itself and a
callback that captures the paired provider handle. Trusted composition remains responsible for
that opaque handle/target pairing as stated below; accepting a raw ref plus callback is not a
supported assembly path.

Assembly finalization rejects duplicate adapter keys and every inconsistent association after the
idempotent value/State contribution rules above. Associating a retained
Program produces ExecutableProgram only when every reachable declaration has one exact compatible
registration. The private split is fixed:

- State implementation reference selects one reusable State driver; association then compares its
  exact Input/Output/Failure contracts, Pure-or-Read kind, and, for Read, exact S/C
  capability/intent/evidence signature; and
- the private tuple of capability contract and binding_ref selects one adapter callback whose
  registered C intent/evidence signature and complete immutable domain binding descriptor must
  match that declaration.

Declaration index, successor/root route, binding, and physical target are deliberately not part of the
reusable State-driver identity. State is deliberately not part of adapter identity. There is no
partial-key fallback and no registration per route. ExecutableProgram privately
pre-resolves both references for each declaration, so the fold performs no lookup and no public
composite key type is needed.

Runtime-private ExecutableProgram owns one Arc to the finalized immutable assembly inner plus compact pre-resolved
indices/handles for its codecs, drivers, adapter callbacks, and Match
projections. It borrows no builder or caller stack. Its construction is the sole assembly
compatibility check; fold never receives RuntimeAssembly again or performs a second
identity/compatibility path.

For every Match, ExecutableProgram association compares the retained selector contract and arms
with the exact registered closed-sum descriptor/tagging profile, proves exhaustive/no-unknown arms
and payload contracts, and pre-resolves the one pure structural projection. Program alone does not
claim that registry-dependent proof.

`binding_ref` content-addresses the complete immutable domain binding descriptor. For surviving EVM
Reads, it is directly the content ref of the domain-owned, secret-free
`EvmPhysicalTarget { chain_id, endpoint_ref }`; there is no one-variant EvmLiveBinding wrapper. The
target requires nonzero u64 chain_id. It is an MfmValue with semantic namespace `mfm.evm`, name
`physical-target`, version `1`, schema name `mfm.evm-physical-target`, and schema version `1`; its
exact externally visible shorthand is `mfm.evm/physical-target@1`. A canonical descriptor/value/ref
golden freezes those identities, the exact `chain_id`/`endpoint_ref` struct fields, and strict
unknown-field rejection. Credentials and client handles are never descriptor material.

Trusted composition constructs each validated EvmPhysicalTarget, passes the same target values to
Portfolio plan_snapshot, and registers the matching live provider under the derived target ref.
EvmPhysicalTarget's sole definition moves from mfm-evm-live to the mfm-evm domain crate, so domain
planning never depends on live IO. The planner input is sorted-unique by chain_id and rejects a
duplicate chain even when the endpoint differs. It places the selected ref in every corresponding
Read declaration and typed C0 route and needs no EvmBalanceBindings/BindingDescriptor wrapper. The sealed live adapter
constructor derives the ref itself and captures the client; it never accepts a caller-asserted ref
as proof. Before provider IO the callback checks the typed intent chain/route against its captured
target/ref. An opaque provider client cannot self-attest its network route, so trusted composition's
pairing of that handle with the public target is an explicit TCB assertion. One target ref may serve
several capability registrations. A semantic target-schema change changes the ref. Whether the
immutable assembly uses one or several private maps is an implementation detail; it exposes one
association path and one final consistency check.

Planner and Runtime derive the target ref through the same MfmValue canonical qualification/content
addressing rule; EvmPhysicalTarget introduces no ad hoc hash schema.
`EvmPhysicalTarget::binding_ref(&self) -> Result<ContentRef, EvmDomainError>` exposes that derived ref
for authoring, but no constructor or registration accepts one from the caller.

### 3.4 One private heterogeneous driver boundary

The retained Program selects a concrete Rust State at runtime, so one private object-safe dispatch
is unavoidable. RuntimeAssembly stores one private `RegisteredState` driver per resolved State
association. That single driver boundary owns the whole selected State attempt:

- decode or downcast the exact input;
- prepare or evaluate `S`;
- for Read, qualify the prepared intent, lend it to the exact registered non-mutating adapter, and
  retain the typed State input and intent across that await;
- interpret and qualify the exact typed outcome;
- encode and append the resulting Journal frame;
- on each Inserted, extend and validate the borrowed private fold accumulator with the exact frame
  before performing any dependent action; and
- return only a small non-generic durable disposition to the outer progression loop.

RegisteredState.start receives a private mutable driver context borrowing the one accumulator and
Store. That context exposes append-and-fold_next, reload-required, and stop operations only inside
Runtime; it is not public, persisted, retained after the call, or a second reducer. A Read driver
therefore performs one observation and, when resolved, appends one fused conclusion while its typed
input, intent, evidence, and outcome remain correlated inside the monomorphic future. No typed value
or owner is smuggled through an erased driver result.

The registered adapter callback has this exact stable-Rust shape (modulo private aliases):

~~~text
dyn for<'a> Fn(&'a C::Intent)
  -> BoxFuture<'a, Result<C::Evidence, ReadUnavailable>>
  + Send + Sync
~~~

`BoxFuture<'a, T>` is a private alias for a pinned boxed `Future<Output = T> + Send + 'a`.
`ReadUnavailable` is the Runtime-owned public zero-detail unit error required by cross-crate live
adapters and is redaction-safe. The callback therefore borrows the exact typed
intent while Runtime retains its private qualification proof, only for the lifetime of its future,
and returns either one typed `C::Evidence` or ReadUnavailable.
The callback captures its validated binding descriptor and live handles; it receives no RunId,
Store position, append authority, public call token, or generic retry token. Runtime retains the
intent and its canonical proof across the await, applies the registered pure `C::bind_evidence`,
runs the ordinary State interpreter for every accepted evidence variant, and persists that same
intent with the evidence and outcome. Transient provider correlation stays private to the adapter.
Runtime adds no call-id hash, StableId, or persisted correlation field.

The driver always receives the same private qualified canonical-value representation. A value may
originate from complete Journal qualification, from a locally constructed frame after Store returns
Inserted, or from the existing pure Match payload projection, but its contract, content reference,
and canonical bytes pass through the same qualifier and `RegisteredState.start`. There is no
HotValue, DerivedValue, second decoder, or parallel hot lifecycle.

Box of Any is not prohibited inside this one private driver boundary. Any second supported
erase/downcast workflow is prohibited.

### 3.5 No independent configuration lifecycle

The core has no MfmConfig trait, ProvenConfiguration owner, ConfigurationKey, mutable latest
configuration, or configuration registry. Existing authoring structures named Config are migrated
according to what they mean:

- secret-free material required to interpret every run using one Program is persisted in that
  Program;
- secret-free material selected independently for one run is part of typed `C0`; and
- clients, credentials, connection material, and other live bindings remain process-local in
  RuntimeAssembly or adapters; the exact secret-free physical target identity required by an
  adapter association is already committed by Program's binding_ref.

Concretely, EvmConfig is submission-only and is deleted outright with that recommended retired
entry point; no empty or renamed replacement remains. PortfolioConfig becomes an ordinary
process-local snapshot-authoring input, loses its MfmValue/MfmConfig derives, and is never passed to
Runtime, Journal, or Store. `plan_snapshot` validates it and consumes its selected public material
and exact EvmPhysicalTarget inputs as
`Result<(Program, PortfolioSnapshotInput), PortfolioError>`: the exact Program followed by typed
`C0`, with no PortfolioAdmissionPlan wrapper or `source_refs`. The old `snapshot_closure_document`,
App `application_catalog`, and `validate_runtime_closure` paths are deleted; RuntimeAssembly
associates each real Program instead of validating a dummy catalog closure. Resume and read
therefore need no authoring Config structure.

Runtime never consults ambient or latest configuration while starting, resuming, or reading a
run. A retained Program plus `C0` is the complete durable admission input.

## 4. Runtime design

### 4.1 Sole semantic reducer

Runtime owns one pure fold over:

- an ExecutableProgram;
- one JournalHistory.

The fold yields one private state:

- Runnable with declaration index, its pre-resolved State driver, and the current qualified value;
- Succeeded with the exact retained Program-root success value;
- Failed with the exact retained Program-root failure value; or
- an explicit semantic inconsistency.

The reducer validates:

- genesis/Program/context association and declaration index zero as the non-empty Program entry;
- exactly one genesis, one RunId throughout the chain, and equality with the requested RunId;
- State-or-Match declaration selection order;
- success/failure successor and root-terminal contract continuity;
- each Pure or Read conclusion belonging to the State selected immediately before it;
- evidence and outcome contracts plus the registered pure C::bind_evidence relation for the exact
  retained intent/evidence pair;
- Match selector/payload continuity; and
- terminality.

On a successful conclusion the fold follows the declaration's success successor or proves the
Program root-success terminal. On a failure conclusion it follows the common failure successor with
the complete unchanged qualified failure or proves the Program root-failure terminal. A child
failure with a mapper successor is therefore Runnable, not Failed. An accepted integrity failure
is an ordinary interpreter outcome and uses that same declared failure edge; Runtime has no
integrity-specific route or projection.

Match uses the one registered schema projection to extract and qualify the selected canonical
payload ephemerally. It creates no Journal record or object. Both Match projection and failure-edge
handoff feed the same qualified-value representation used by ordinary cold State input.

It does not execute State preparation/evaluation/interpretation, call an adapter, perform provider
IO, or re-author a Program.

Execution, resume, and read all use this reducer. There is no Store reducer or replay reducer.

### 4.2 One atomic Read observation

A Read performs no externally mutating action and needs no durable preparation authority. Runtime
derives and qualifies its intent locally, invokes the exact registered adapter once in that
top-level progression call, and either:

- receives ReadUnavailable, appends nothing, and returns RuntimeError::Unavailable; or
- receives typed evidence, binds and interprets it, and appends one fused Read conclusion against
  the exact head that selected that State.

The fused conclusion carries the exact intent, evidence, and outcome. Cold fold therefore repeats
the pure intent/evidence binding without re-running a provider or trusting a process-local call ID.
Concurrent calls may make duplicate non-mutating observations and obtain different evidence. The
first exact-head conclusion inserted is the durable result; every stale caller reloads and follows
that winner. A later start or resume may observe the same Runnable State again. There is no
persisted attempt count, replacement relation, preparation record, Waiting state, call token, or
adapter-entry authority.

### 4.3 One graph-bounded caller-driven progression loop

start first validates/associates the supplied Program and C0, encodes genesis, and calls append
directly; it performs no admission pre-read. Genesis Inserted initializes the private accumulator and
fold_next, while NotInserted takes the complete reload/full-fold path. resume begins with one
complete load, Journal qualification, and full fold under the retained Program. The
advance_until_stable loop then repeatedly:

1. selects and enters one RegisteredState at a time;
2. lends that driver the private driver context for mechanical append; and
3. within the still-live monomorphic driver future, applies the same fold_next transition after
   each Inserted using the exact locally held encoded frame.

NotInserted discards the accumulator and performs one complete current reload and full fold.
Indeterminate returns without local advancement. Cold resume/read always begins with the complete
load/full-fold path. There is no reload after a known Inserted, no second hot reducer, and no history
scan per State; progressing a run is linear in the frames actually loaded or appended.

Runtime stops at:

- terminal success or declared failure;
- an indeterminate append;
- invalid history;
- incompatible assembly;
- capacity; or
- infrastructure failure.

Runtime owns no active-progression, CPU-job, planning-job, or provider-ingress semaphore.
Application/composition may impose one operational bound on simultaneous top-level Runtime calls.
Adapter clients and Store connection pools independently bound their own live IO. None of those
resource policies is a Runtime semantic owner or persisted contract.

There is no separate per-call State-start counter or yield setting. Program acyclicity,
MAX_RUN_FRAMES, and MAX_RUN_BYTES bound successful progression; async IO awaits and spawn_blocking
keep the executor cooperative. One ReadUnavailable error ends that top-level call, so Runtime never
immediately loops on a failing provider. read may observe Runnable. A successful start/resume
continues until terminal success/failure; its nonterminal stops are errors.

There is no scheduler, timer, background run progression, process-wide writer lease, process-local
per-run lock, pending append table, resolver queue, retained recovery token, cancellation token, or
detached completion task.

### 4.4 Cancellation-safe progression and live-IO wait ownership

Dropping a start or resume future is supported and safety-neutral. It may discard volatile Pure
work, a non-mutating provider observation, or a locally interpreted outcome, but it creates no
external mutation authority. If a Store append was in flight, its one frame may or may not have
committed atomically; the next complete load resolves the durable prefix. A later exact start or
resume continues from that prefix and may safely repeat a Pure computation or Read observation.

Runtime therefore needs no cancellation token, cleanup method, detached completion task, or
cancellation-specific durable state. Application may race a top-level future against its own
deadline or stop polling after disconnect; cancellation is merely absence of further progress.
Runtime itself owns no timeout policy. Wait control exists only at live-IO boundaries:

- an Adapter owns provider/client deadlines, connection limits, protocol acknowledgement, and the
  exact mapping to typed evidence or ReadUnavailable;
- Store owns database/pool deadlines and the exact mapping to Unavailable or Indeterminate; and
- Application/composition may bound simultaneous top-level Runtime calls without becoming an
  execution lifecycle owner.

An IO timeout is therefore an Adapter or Store result, not a Runtime lifecycle. ReadUnavailable
appends nothing and maps to RuntimeError::Unavailable, leaving the durable run Runnable. Store append
Unavailable proves the candidate definitely did not commit; load Unavailable means no complete
snapshot was returned. Indeterminate means COMMIT
may have reached PostgreSQL and its outcome is unavailable; it is resolved only by exact retry
inside Store or a later durable load.

After process termination Runtime uses only durable history. There is no volatile owner or command
to recover: a missing Pure/Read conclusion is recomputed or re-observed, while a present conclusion
is folded.

### 4.5 Blocking work contract

State preparation, Pure evaluation, Read interpretation, value
qualification, and substantial canonical work run through spawn_blocking when they may block the
async executor. Runtime immediately awaits the JoinHandle before using the result.

The blocking closure contains only trusted, bounded-input, pure, terminating synchronous work. It
owns no Store, Adapter, provider client, async runtime handle, or append authority; it never invokes
block_on. Store append and adapter/provider Read remain ordinary async IO after the blocking result
is observed. If the outer Runtime future is dropped, an already-running blocking closure may finish,
but its result is discarded and it cannot perform dependent IO or append. Runtime adds no CPU
semaphore or blocking-job timeout. A callback that does not terminate violates its trusted
registration contract; Runtime does not attempt to interrupt it.

### 4.6 Admission and conclusion races

Admission's genesis frame derives expected head Absent.

- Inserted commits the exact genesis.
- NotInserted causes Runtime to load the complete current run. The exact same genesis resumes it; a
  valid different genesis under the same RunId is AdmissionConflict; malformed history is
  InvalidHistory.
- Indeterminate retains nothing. The caller repeats exact start inputs or later reads the RunId.

Conclusion uses the exact selected head. NotInserted means the same frame already exists or another
valid frame won that head/progressed beyond it. Runtime discards its local Pure/Read result, reloads
the complete durable run, and follows the winner. Invalid retained history still fails closed.
There is no conclusion Conflict or private no-longer-selected lifecycle.

### 4.7 RunView and public Runtime surface

RunView is a private-field, non-Serde trusted-core type:

~~~text
RunView {
  run_id,
  head_sequence,
  head_digest,
  state:
      Runnable
    | Succeeded(RetainedValueView)
    | Failed(RetainedValueView)
}

RetainedValueView {
  contract_ref,
  value_ref,
  canonical
}
~~~

Every RunView names one real qualified durable head. It is a snapshot: a transaction whose
acknowledgement was previously indeterminate may commit after an earlier view was captured.
Succeeded and Failed are emitted only after the fold proves the retained value against the
Program's root-success or root-failure contract respectively. Intermediate mapped failures never
appear as terminal RunView values.

The core process-facing surface is:

~~~text
start(run_id, Program, typed_c0) -> Result<RunView, RuntimeError>
resume(run_id) -> Result<RunView, RuntimeError>
read(run_id) -> Result<RunView, RuntimeError>

RuntimeError =
    Absent
  | AdmissionConflict
  | Indeterminate
  | InvalidHistory
  | IncompatibleAssembly
  | Capacity
  | Unavailable
  | Internal
~~~

The operation/domain layer deterministically constructs Program and `C0`; Runtime does not own an
entry-point planner registry. start verifies declaration index zero for a non-empty Program or the
zero-State identity, plus the validated Program's exact `C0` contract before admission; it receives
no separate entry selector. start and resume may execute
State/adapter/provider Read work. read does not.

RuntimeError is the one typed, redaction-safe error surface for all three methods; it has no public
driver diagnostics or arbitrary message field. Implementations may retain source chains privately.
There are no per-method error enums or result DTOs, and errors never become RunView states.

The mapping is normative:

| Source | RuntimeError |
| --- | --- |
| resume/read of absent RunId | Absent |
| different valid genesis under one RunId | AdmissionConflict |
| Store append COMMIT ambiguity | Indeterminate |
| Store physical corruption, retained fixed-limit violation, Journal/Program decode failure, or semantic retained-history violation | InvalidHistory |
| retained C::bind_evidence failure | InvalidHistory |
| duplicate/inconsistent RuntimeAssembly contribution or finalization | IncompatibleAssembly |
| RuntimeAssembly Read capability contract_id/registration failure | IncompatibleAssembly |
| valid Program unsupported or inconsistently associated by the finalized assembly | IncompatibleAssembly |
| local admission/outcome/frame construction or append capacity rejection | Capacity |
| Read adapter/provider or definite Store load/append unavailability | Unavailable |
| trusted supplied Program/typed-C0 contract mismatch before admission | Internal |
| hot C::bind_evidence failure after typed adapter return | Internal |
| unexpected trusted driver, callback, join, or codec failure after redaction | Internal |

StoreOpenError belongs to concrete production PostgreSQL Store opening and is resolved before a
Runtime is constructed; it is not another start/resume/read error surface. A later pool-connection
readiness rejection maps to StoreError::Unavailable as specified in Section 6.5 and requires Store
reconstruction.

Trace, adapter audit, raw frame inspection, and a separate replay response are not core APIs. A
future required trace or audit must be a pure Runtime-owned projection over the same qualified
history, never another reducer or Store port.

## 5. Journal design

### 5.1 Canonical wire rules

All hashed structured data uses the repository canonical JSON implementation with JCS-style
semantics:

- UTF-8 canonical JSON only;
- no floats;
- no duplicate or unknown object fields;
- exact tagged variants;
- no serializer-dependent optional-field omission;
- arrays in their specified canonical order;
- bounded strings, arrays, maps, and nesting;
- canonical bytes must equal strict decode and re-encode; and
- construction inputs include only exact content references and canonical values supplied by the
  Runtime-qualified path.

Secret exclusion is a layered input contract, not a semantic property discoverable by a generic
codec. Domain types and adapters keep secrets outside MfmValue inputs; Runtime value qualification
rejects the repository's mechanically identifiable secret markers before Journal construction;
and Journal never logs canonical values or includes them in diagnostics. An arbitrary secret
encoded as an ordinary string is not claimed to be detectable by Journal.

Protocol counters are non-negative JSON integers bounded by the fixed limits below. Domain values
retain the canonical representation fixed by their MfmValue schemas.

Journal public construction and strict decoding produce opaque valid types. Open public DTO
construction followed by validate choreography is deleted.

### 5.2 Run frame wire

The exact target shape is:

~~~text
RunFrameV1 {
  domain: "mfm.run.frame.v1",
  run_id,
  run_sequence,
  previous_head_digest: null | digest,
  record,
  objects: sorted-unique [
    {
      content_ref,
      canonical: raw canonical no-float JSON value
    }
  ]
}
~~~

Exact record tags and fields are:

~~~text
RunAdmitted {
  kind: "run_admitted",
  program_ref: content_ref,
  admitted_context: content_ref
}

StateConcludedPure {
  kind: "state_concluded_pure",
  outcome
}

StateConcludedRead {
  kind: "state_concluded_read",
  intent: content_ref,
  evidence: content_ref,
  outcome
}

Outcome =
    { kind: "success", value: content_ref }
  | { kind: "failure", value: content_ref }
~~~

Program supplies the selected State, input/output/failure contracts, capability, and binding. None
of that material is duplicated in the semantic frame. The recursive predecessor binds each
conclusion to the State selected by the preceding durable prefix; no occurrence, preparation
sequence, or call ID is persisted. The fixed run-object and non-payload-envelope bounds in Section
5.5 prove every valid Read intent/evidence/outcome triple fits one conclusion frame. There is no
per-State maximum-conclusion field or recursive schema-size algebra.

An accepted integrity block is an ordinary Read conclusion whose capability-owned evidence variant
and State interpreter produce the exact declared failure outcome. Runtime has no special integrity
record or projection.

### 5.3 Exact frame-local object closure

Let R_i be the set of data-object content references directly named by record i and C_i be the
object map embedded in the same frame. The complete rule is:

~~~text
keys(C_i) = R_i
~~~

Direct data-object references are:

- Program and admitted-context objects for RunAdmitted;
- outcome object for StateConcludedPure; and
- intent, evidence, and outcome objects for StateConcludedRead.

Stable contract and implementation references are external identities, not closure objects. There
is no implicit transitive object graph.

Every direct content reference has exactly one object in that frame whose nested canonical value
hashes to the reference. Missing, duplicate, conflicting, out-of-order, or unreferenced objects are
invalid. A reference used by a later frame is embedded again in that later frame; cross-frame
deduplication is deliberately not a wire invariant.

The objects array is sorted by complete content reference. Because RunFrameV1 names at most three
direct objects, frame count and canonical-byte limits already bound object processing. There is no
first-reference dictionary, cumulative object-count limit, object slot reservation, or object
accounting in Store. JournalHistory exposes only the qualified record and frame-local objects needed
by Runtime; it has no cumulative object-store contract. Repeated bytes count fully toward
MAX_RUN_BYTES, which is the accepted cost of deleting cross-frame dictionary state.

### 5.4 Recursive run head and exact record equality

The frame head is:

~~~text
run_head_digest = SHA-256(JCS(RunFrameV1))
~~~

The domain field prevents cross-format reuse. The frame does not contain its own digest.

Genesis requires:

- sequence one;
- previous_head_digest null; and
- RunAdmitted.

Every later frame requires the exact previous sequence plus one and the previous frame digest.

EncodedRunFrame and qualified history expose sealed read-only sequence/head projections required by
Runtime and Store implementations. There is no separate RunPosition public type, wire field, or
Store command/result authority.

Value references already commit to exact canonical objects. The immutable target frame row and
exact canonical frame bytes therefore define physical idempotency without another digest, persisted
identity, or public proof type. Runtime compares supplied and retained genesis only when classifying
admission after NotInserted.

### 5.5 Fixed run limits

The clean format freezes:

| Resource | Bound |
| --- | ---: |
| One canonical frame | 25,231,360 bytes |
| One frame-embedded Program/value object | 8,388,608 bytes |
| Conservative non-object frame-envelope allowance | 65,536 bytes |
| Frames per run | 65,536 |
| Canonical frame bytes per run | 536,870,912 bytes |

The corresponding constants are MAX_FRAME_BYTES, MAX_RUN_OBJECT_CANONICAL_BYTES,
MAX_FRAME_NON_PAYLOAD_ENVELOPE, MAX_RUN_FRAMES, and MAX_RUN_BYTES. The Program decoder uses the same
8,388,608-byte ceiling. mfm-values owns and exports the one
MAX_RUN_OBJECT_CANONICAL_BYTES constant; Program and Journal import it without duplicate constants
or reexports. Journal alone owns MAX_FRAME_BYTES, MAX_FRAME_NON_PAYLOAD_ENVELOPE, MAX_RUN_FRAMES,
and MAX_RUN_BYTES. Journal construction and strict retained decoding enforce the object bound.

For this proof, measured non-object envelope bytes equal complete canonical frame length minus the
sum of the raw canonical lengths of objects embedded in its objects array. Construction and strict
decoding require that measurement to be at most MAX_FRAME_NON_PAYLOAD_ENVELOPE. The allowance is a
conservative theorem bound, not a promise that an otherwise valid frame can attain every byte up to
65,536 independently of its field limits.

Every record has at most three direct objects, and all non-object fields, identities, references,
tags, counters, and wrappers have fixed bounds. Goldens with every such field at
its maximum, including worst-case canonical string escaping and counter digit widths, prove:

~~~text
MAX_FRAME_BYTES
  = 3 * MAX_RUN_OBJECT_CANONICAL_BYTES + MAX_FRAME_NON_PAYLOAD_ENVELOPE
  = 25,231,360
~~~

Thus every accepted Read intent, evidence (including a capability-owned integrity-block variant),
and success/failure outcome is representable in one frame. Frame bytes already contain every frame-local
object value, so objects are charged exactly where their bytes occur and never require separate
cumulative accounting.

Changing a limit requires another deliberate persisted-format cutover.

## 6. Store and PostgreSQL design

### 6.1 Mechanical Store surface

The target Store operations are:

~~~text
load_run(run_id)
  -> Result<Option<StoredRunBytes>, StoreError>

append_run(&EncodedRunFrame)
  -> Result<Inserted | NotInserted, StoreError>
~~~

Those are the only Store trait methods. Store is Send + Sync and object-safe; Runtime holds one
`Arc<dyn Store>`. Its async methods return boxed Send futures borrowing `self` for the call lifetime.
They do not require a `'static` future and do not add an async-trait dependency or a generic Store
parameter to every Runtime/App type.

StoredRunBytes contains only one atomically captured complete prefix as ordered opaque frame bytes.
It does not claim canonical or semantic validity; Journal derives and checks sequence and final head
from those bytes. Store returns a complete value or an error, never truncation. Every implementation
supports the same fixed format ceilings; there is no lower peer-specific mutation or read envelope.
Capacity is append-only: retained rows claiming a total beyond the fixed format are
CorruptPhysicalState, not a large valid history.

mfm-journal owns StoredRunBytes, because mfm-store already depends on Journal's sealed frame types
and reversing that ownership would create either a dependency cycle or a public raw-history getter.
StoredRunBytes is a public opaque, invariant-checked, explicitly unqualified transfer type required
by the Store trait signature. Its public fallible constructor accepts ordered frame-byte buffers for
Store implementations and checks only raw frame-count, per-frame, and cumulative-byte ceilings
before retaining a private vector. Its fields are private and it exposes no frame/bytes iterator or
inspection accessor. `JournalHistory::qualify` consumes it and performs the only
structural/canonical/history qualification. Runtime cannot build semantic history by reading raw
bytes around Journal. Sealed construction applies to EncodedRunFrame and qualified JournalHistory,
not to this cross-backend transfer constructor.

Concrete PostgreSQL open returns `Result<PostgresStore, StoreOpenError>`, where
`StoreOpenError = Incompatible | Unavailable`; Incompatible covers a wrong schema baseline or unsafe
required durability setting, and Unavailable covers failure to establish that observation. Memory
construction has no readiness protocol. No ready/check method is retained on dyn Store, and opening
has no ambiguous-mutation result.

StoreOpenError belongs to and is exported only by the concrete mfm-storage-postgres implementation,
not by the backend-neutral mfm-store trait crate. The generic Store surface has no opening or
readiness vocabulary.

PostgreSQL begins `REPEATABLE READ READ ONLY` before its first snapshot statement. Within that one
snapshot it:

1. captures the current head joined to its immutable frame;
2. for an absent head, rejects any orphan frame before returning Absent;
3. rejects a recorded total over the fixed limit as CorruptPhysicalState before allocating;
4. checks row count, minimum/maximum sequence, and summed octet length against head sequence and
   total bytes;
5. fetches every frame in sequence order; and
6. checks the row RunId/sequence, per-frame length, recomputed frame digest, and accumulated length.

A gap, duplicate, orphan, wrong digest, or inconsistent head/accounting row is
CorruptPhysicalState. Only then does Store return bytes for Journal canonical qualification and
predecessor recurrence. A concurrent append is wholly before or wholly after the captured prefix,
never a truncated mix.

Store does not decode Program, reduce State-or-Match, select facts, reify domain values, or
construct adapter authority.

### 6.2 Append input, results, and errors

EncodedRunFrame supplies sealed read-only projections of its RunId, candidate sequence,
predecessor, head digest, and exact canonical bytes. Genesis derives expected head
Absent; every other candidate derives expected sequence minus one plus previous_head_digest. No
caller can supply a second expected position, reservation instruction, command digest, or byte
length.

~~~text
AppendResult =
    Inserted
  | NotInserted

StoreError =
    Capacity
  | CorruptPhysicalState
  | Unavailable
  | Indeterminate
~~~

Meanings are normative:

- Inserted proves this exact frame committed through this acknowledged invocation.
- NotInserted proves this invocation wrote nothing because the exact frame already exists or the
  candidate-derived predecessor is no longer current.
- append-only Capacity proves this invocation wrote nothing.
- CorruptPhysicalState means retained rows or metadata violate the physical contract and fails
  closed; an append may return it only before candidate mutation or after proven rollback, so this
  invocation wrote nothing.
- Unavailable means the requested Store operation could not complete. For append it is allowed only
  when Store proves the candidate did not commit: failure before COMMIT submission, confirmed
  rollback, or a definite COMMIT rejection. For load it covers unavailable connection/snapshot/query
  completion and has no mutation implication.
- Indeterminate is allowed only when COMMIT may have reached PostgreSQL and its outcome is
  unavailable. Read operations never return it.

AcknowledgementUnknown is not an AppendResult. Exact-target-versus-predecessor-mismatch and result
positions are redundant because every NotInserted requires the same complete reload. InvalidPhysicalCommand is
unrepresentable after Journal sealing.

### 6.3 Idempotency and PostgreSQL transaction order

PostgreSQL serializes all appends for one RunId, including absent genesis, with one transaction
advisory lock. The exact transaction is:

1. begin `READ COMMITTED READ WRITE` and set local synchronous_commit to on;
2. compute `SHA-256(JCS { domain: "mfm.store.run-lock.v1", run_id })`, interpret its first eight
   octets as a big-endian two's-complement i64, and call the one-argument
   `pg_advisory_xact_lock(bigint)` before any target/head read; collisions may only over-serialize;
3. read the current head with a left join to its immutable frame and read the candidate target row;
   when no head exists, also run an indexed EXISTS-any-frame-for-RunId query under the same lock;
4. validate observed physical state before classifying the candidate: a missing head permits no
   frame for that RunId and therefore requires that EXISTS result to be false; a present head must
   join its exact row and have a locally valid sequence, recomputed digest, and bounded total; any
   observed target must have the requested key, self-consistent bytes/digest, and a sequence no
   later than the head; and a candidate sequence at or before the head must have a target row.
   Violation is CorruptPhysicalState;
5. if the target row has exact frame bytes, return NotInserted;
6. otherwise compare the candidate-derived predecessor with the validated current head and return
   NotInserted without mutation when they differ;
7. validate actual frame, frame-count, and cumulative-byte limits from the sealed candidate;
8. insert the immutable frame row;
9. insert/update the run head and cumulative bytes; and
10. commit atomically.

The primary-key row `(run_id, run_sequence)` plus exact frame bytes is the receipt. Exact retry
remains NotInserted after later frames. A different candidate for the same predecessor races the
head and becomes NotInserted. No command digest/index or separate command/receipt row exists. Memory implements
the same ordering and outcomes with an in-process per-RunId critical section. It completes every
fallible validation and required allocation before mutation, then publishes frame, head, and total
together; no returned-error path exists after its first mutation. At most one caller observes
Inserted for one exact frame.

### 6.4 Actual capacity accounting

Every candidate eligible for insertion uses checked/subtraction-form arithmetic to require the
sealed frame length at most MAX_FRAME_BYTES, its candidate sequence within MAX_RUN_FRAMES, and
current total bytes at most `MAX_RUN_BYTES - frame_length`. NotInserted bypasses these insertion
checks because it adds no frame or bytes.

Store accounts only the exact candidate. There is no future-conclusion preflight, durable
reservation, liability row, counter, instruction, per-State maximum, release, transfer, or
underflow path.

### 6.5 PostgreSQL run schema and durability

The logical schema contains:

| Table | Key and retained material |
| --- | --- |
| run_frames | primary key (run_id, run_sequence); canonical frame bytes and derived head digest |
| run_heads | primary key run_id; head sequence and cumulative octet length, with a non-cascading foreign key to its immutable frame |

The static schema-baseline marker remains. Sequence, identifier/digest, frame-length, total, and
foreign-key constraints are physical defenses. Predecessor and byte length are not duplicated
columns: the frame seals the predecessor and PostgreSQL supplies octet_length. The head digest is
read by joining its frame. Runtime deployment may grant only SELECT/INSERT/UPDATE needed by these
transactions, but this RFC adds no trigger or database-role proof layer.

The rewritten `crates/storages/postgres/migrations/0001_*.sql` is the static
operator/provisioning artifact. Production Store code never executes DDL and exposes no public
migrate/install method. Same-crate database tests may use a private cfg(test) fresh-schema helper;
there is no test-support export.

All authority tables are permanent/logged. Concrete production open rejects a wrong schema
baseline, pg_is_in_recovery(), fsync or full_page_writes disabled, or an unlogged authority table.
Every append sets synchronous_commit on. The admitted durability claim is acknowledged local WAL
flush followed by primary process/host crash and restart, assuming storage honors flushes.
Asynchronous-replica failover that may lose acknowledged commits, PITR rollback, writable backup
clones, media loss, and lying storage are unsupported.

The sole production constructor is
`PostgresStore::connect(database_url: &str) -> Result<PostgresStore, StoreOpenError>`. It returns a
usable Store handle only after those checks succeed; there is no public PgPool-taking/unchecked
constructor, durability-profile argument, broad concrete error, or post-open readiness method. The
pool applies the same gate before admitting every newly established physical connection, including
reconnect after database restart. In-place schema or durability-prerequisite changes while an
established connection remains usable are unsupported and require Store reconstruction.

If that gate finds incompatibility while establishing a replacement pool connection after Runtime
already exists, the pool rejects the connection and the interrupted Store operation returns
StoreError::Unavailable; for append, the candidate has not been submitted. The concrete Store may
retain the incompatibility source privately for operators, but StoreError gains no second
Incompatible variant and Runtime exposes no environment diagnostic. The production Store must be
reconstructed before use can resume.

Memory is the ephemeral atomic conformance backend and makes no post-process-restart durability
claim.

### 6.6 Global schema deletion

The target schema has no:

- tenant, Store scope, writer epoch, active identity, rotation, or database incarnation tables;
- global object or per-run object-membership tables;
- append-command/digest, reservation, append-request, or separate receipt tables;
- configuration revision, head, request, or receipt tables;
- fact head, publication, sparse-tree, CT-log, proof, or node tables; or
- any global sequencing table.

Schema version is a static compatibility check, not a writer authority.

## 7. Execution flows

### 7.1 New run

~~~text
operation/domain layer deterministically constructs Program and typed C0
  -> explicit fresh RunId
  -> Program validation and Runtime association
  -> qualify C0 against Program's exact admitted-context contract
  -> use declaration index zero (or the zero-State identity)
  -> Journal encode genesis with complete frame-local Program/C0 objects
  -> Store append sealed genesis, whose predecessor encodes Absent

Inserted
  -> extend the private accumulator with that exact accepted frame
  -> fold_next without reloading
  -> continue progression

NotInserted
  -> complete load/fold current run
  -> continue exact run, AdmissionConflict, or InvalidHistory

Indeterminate
  -> retain nothing
  -> caller repeats exact start or reads RunId
~~~

Core never derives RunId. Every independent execution uses a new caller-supplied value. Retry
reuses the exact RunId and admission inputs.

### 7.2 Hot Pure State

~~~text
Runnable Pure State
  -> exact RegisteredState.start
  -> spawn_blocking evaluation
  -> ProposedStateOutcome<Output, Failure>
  -> Runtime value qualification
  -> Journal encode Pure conclusion
  -> Store append exact head

Inserted
  -> extend the private accumulator with that exact accepted frame
  -> fold_next without reloading

NotInserted
  -> complete load/fold durable current run

Indeterminate
  -> retain nothing
  -> later resume recomputes if conclusion is absent
~~~

Pure evaluation must be deterministic and perform no ambient IO.

### 7.3 Hot Read State

~~~text
Runnable Read State
  -> exact RegisteredState.start
  -> spawn_blocking preparation
  -> Result<C::Intent, ReadPreparationError>

ReadPreparationError
  -> append nothing
  -> return RuntimeError::Internal

typed intent
  -> qualify exact typed intent
  -> typed borrowed-intent adapter ingress
  -> Result<C::Evidence, ReadUnavailable>

ReadUnavailable
  -> append nothing
  -> return RuntimeError::Unavailable

typed evidence
  -> pure C::bind_evidence(intent, evidence)
  -> spawn_blocking ordinary interpretation
  -> typed outcome
~~~

For accepted evidence:

~~~text
typed intent/evidence/outcome qualification
  -> Journal encode one fused Read conclusion
  -> Store append exact selected head

Inserted
  -> extend the private accumulator with that exact accepted frame
  -> fold_next without reloading

NotInserted
  -> discard local observation
  -> complete load/fold durable winner

Indeterminate
  -> retain nothing
  -> later history reveals the committed conclusion or the same Runnable State
~~~

One invocation enters a selected Read at most once before either obtaining evidence or returning
ReadUnavailable. A later start/resume may safely make another non-mutating observation when no
conclusion is durable.

### 7.4 Resume and read

resume:

~~~text
Store complete current load
  -> JournalHistory
  -> retained Program decode
  -> Runtime association
  -> Runtime fold
  -> advance_until_stable
~~~

read performs the same load, qualification, and fold but starts no State, adapter, or provider work.
The fold may invoke only the registered pure schema projection and C::bind_evidence semantic
qualifiers already required above; it invokes no Operation/authoring callback or ambient IO.

There is no pending-owner resolution step.

## 8. Facts are deleted

The cutover deletes:

- the mfm-facts crate;
- FactSelectionMode, NoPriorFacts, PriorRunFacts, and the capability Facts associated type;
- prior_fact_selection callbacks;
- FactSelectionRequest, FactSelection, FactValue, FactProposalSet, provenance, completeness, and
  frontier types;
- facts from ProposedStateOutcome;
- source refs used only as fact allow-lists;
- fact fields from Program and every Journal record;
- Store fact query/publication ports;
- publication attachments and coordinate allocation;
- fact-head CAS and rebinding;
- sparse Merkle and CT log structures;
- producer dependency DAGs and portable fact proofs;
- fact limits, schema, SQL, migrations, fixtures, and tests; and
- facts dependencies from every surviving crate.

The target does not retain empty placeholder fields, NoPriorFacts markers, reserved tables, or a
generic extension hook.

Facts may return only through a separate future RFC with a reachable consumer and a new deliberate
persisted contract.

## 9. Identity and restore contract

The cutover deletes StoreScopeId, StoreEpoch, TenantScopeId, StructuredStoreIdentity, Store opening
brands, active-identity rows, rotation methods, identity retries, and every related field from:

- Program and Journal;
- hashes and canonical fixtures;
- Store APIs and backend rows;
- Runtime values and errors;
- App/transports.

No replacement identifier may provide the same function under another name.

One run database is one run authority and RunId is global within it. Deployment credentials and
network exposure are external security boundaries.

The authority claim covers one monotonic database timeline that never loses its acknowledged rows.
Stopping old peers is necessary but is not sufficient to make a writable rollback the continuation
of that same RunId history: it can forget an acknowledged conclusion and create a divergent fork.

Writable rollback restore, point-in-time recovery behind acknowledged state, data-loss failover, and
writable backup clones are unsupported. Historical copies are read-only and non-authoritative. A
new writable timeline must receive fresh external authority and must never resume RunIds from the
old timeline when callers could confuse the two histories. The core intentionally adds no
in-database incarnation or fence that would itself roll back with the database.

## 10. Package and API cutover

### 10.1 mfm-ids, mfm-values, and derive

- add only the exact reserved root SchemaIdentity exception needed by mfm-program's Never and reject
  every attempted Never value, without exporting another Never type or generic empty-enum switch;
- retain strict canonical, schema, no-float, bounded-input, and content-reference rules;
- own/export the sole MAX_RUN_OBJECT_CANONICAL_BYTES constant and enforce it at run-object
  construction; mfm-program and mfm-journal import it without duplicate definitions/reexports;
- reject mechanically identifiable secret markers during persistable value qualification without
  claiming semantic secret discovery;
- delete MfmConfig, ValidatedConfig, ProvenConfiguration, and the MfmConfig derive;
- remove facts-related value/schema exports;
- delete unused PublicOutputDescriptor/StateInput/OperationOutput/PublicOutputs traits, their derive
  macros and SchemaKind variants, plus FieldSegment/FieldPath support; every surviving State/root
  value uses only MfmValue and exact Program contract refs;
- delete unused CapabilityKind/CapabilityVersion ID families; Read capabilities retain only their
  strict stable contract ID and capability_contract_ref derivation; and
- update derives and compile-fail coverage.

### 10.2 mfm-capabilities

- replace AccessCapabilityContract with the exact ReadCapabilityContract trait from Section 3.1;
- retain only Intent, Evidence, contract_id, bind_evidence, CapabilityError, and its Result alias;
- delete AccessMode, ReadMode, EffectMode, attempt bounds, fact selection, evidence markers,
  QualifiedRecordedEvidence, ProposedStateOutcome, and every mutation/retry/call-correlation owner;
  and
- depend only on mfm-values and mfm-ids plus the existing error support.

### 10.3 mfm-program

- retain the immutable State-or-Match document;
- own and export Never, State, PureState, ReadState, the two-variant ProposedStateOutcome, and zero-detail
  ReadPreparationError; remove domain-local behavior traits and public Runtime implementation
  wrappers;
- expose only ProgramError::Canonical/InvalidContract/Capacity for checked Program construction and
  decode, with private diagnostics and no catalog/value-registration variants;
- persist exact root success and root failure contracts;
- make every failure contract non-optional;
- represent State success and failure continuations as optional forward indices, where absence means the
  corresponding exact root result except that absence for Never is unreachable, and delete the
  redundant terminal boolean;
- reject a present failure successor for Never;
- require every successor input to equal the complete predecessor result contract;
- retain the current single common failure successor used by Portfolio failure mapping;
- add no persisted variant-specific failure router;
- persist complete Read association;
- remove the Program configuration contract and migrate durable policy into Program or `C0`;
- remove FailureValue and State::integrity_failure;
- remove Program execution catalogs, reifiers, TypeId maps, brands, and erased callbacks;
- remove fact behavior; and
- expose no supported erase/downcast workflow.

The deferred Operation authoring RFC must lower typed propagation and handlers into this retained
graph. It does not cause a second Program format, delete Match, or add FailureNext::ByVariant.

### 10.4 mfm-runtime

- own RuntimeAssembly, Program association, the sole reducer, typed State drivers, graph-bounded
  progression, and RunView;
- expose only register_value/register_pure/register_read/register_adapter contributions, with
  automatic exact codec registration/deduplication and no public implementation-holder type;
- keep one private object-safe State driver and one qualified canonical value representation; add no
  HotValue fast-path type;
- extend and fold a private hot accumulator after Inserted, reloading after NotInserted, cold entry,
  or acknowledgement ambiguity;
- run blocking deterministic callbacks outside the async executor and immediately await them;
- retain no pending append or conclusion owner;
- own no cancellation API, timeout policy, detached completion task, or Runtime semaphore;
- perform no background progression; and
- expose no public State-by-State lifecycle algebra.

### 10.5 mfm-journal

- own only the exact RunFrameV1 codec;
- own strict construction/decoding, recursive run heads, exact frame-local object closure, and
  fixed frame/run/object/envelope limits;
- own public sealed EncodedRunFrame and qualified JournalHistory plus public
  opaque/invariant-checked unqualified StoredRunBytes, where EncodedRunFrame exposes Store's
  required read-only exact-byte projection while StoredRunBytes/JournalHistory expose no raw-history
  getter; and
- remove open DTO validation choreography, configuration and portable codecs, record digests,
  first-reference state, identity fields, request ids, facts, and publication rebinding.

### 10.6 mfm-store

- expose only complete load_run and append_run over opaque sealed frames;
- make the sole concrete PostgreSQL connect constructor perform the mandatory
  compatibility/durability checks before returning a usable handle, with no public migration,
  pool-taking/unchecked constructor, durability profile, broad concrete error, post-open method, or
  dyn-Store readiness method;
- implement identical Memory/PostgreSQL append semantics;
- derive sequence, predecessor, and target identity from the candidate frame; use the immutable
  `(run_id, sequence)` row plus exact bytes as the receipt;
- account only each exact candidate's actual frame/count/byte cost, with no preparation preflight or
  persisted reservation;
- own database/pool wait control; append classifies Unavailable only when commit is definitely
  absent and Indeterminate only when COMMIT may have happened, while load Unavailable means no
  complete snapshot;
- use one read-only repeatable snapshot for multi-statement loads and a read-committed,
  synchronous-commit append transaction whose first state-observing action is the per-RunId
  transaction advisory lock;
- own no Program, reducer, State, capability, fact, configuration, portability, or replay
  semantics; and
- delete semantic Store facades, selection owners, brands, duplicated backend DTOs, and split ports.

### 10.7 Adapters and live IO

- own provider/client deadlines, connection limits, and protocol acknowledgement;
- return one capability-owned typed Evidence value or redaction-safe ReadUnavailable;
- perform only externally observational, duplicate-safe Reads; any protocol-level retry remains
  inside that one borrowed-intent callback and its deadline;
- expose no cancellation token or generic retry token to Runtime;
- accept only evidence and outcomes within the universal run-object envelope; and
- encode provider rejection, safe failure, and integrity block in the capability's closed Evidence
  contract for ordinary State interpretation; a retryable unavailability appends no record.

### 10.8 EVM product scope

Subject to the one Material uncertainty, the cutover:

- retires `mfm.evm/submit-transaction@1` without reassigning its stable ID;
- retains Portfolio snapshot and the EVM Read capabilities they use;
- moves EvmReadEvidence::IntegrityBlocked into the ordinary shared Read interpreter path so it still
  produces the exact EvmBalanceFailure::IntegrityBlocked value before Portfolio failure mapping;
- deletes EVM submission selectors, requests, progress/results/failures, planning and binding
  bundles, nonce reservation, candidate derivation, broadcast, receipt/finality/canonical-inclusion
  States, and submission-status Read contracts;
- deletes WalletNonceAuthority, nonce/broadcast adapter handles and registrations, signer integration
  from live EVM, and the mfm-storage-evm-postgres crate, migration, workspace edges, and fixtures;
- removes Broadcast and PossibleEntry from surviving EvmProviderResponse plus every submission-only
  EvmAdapterError branch, while retaining only Read evidence classifications;
- removes TransactionReceipt, FinalizedHead, and CanonicalInclusionBlock from the surviving
  EvmReadIntent subject wire; removes Receipt, FinalizedHead, and CanonicalBlock from EvmReadValue;
  and deletes ReadCapabilityFamily::SubmissionStatus plus the EvmCapability<3> implementation and
  stable IDs, while retaining the Portfolio Read families;
- removes submission dispatch and composition from App and every submission claim from current
  READMEs, design/architecture, inventories, known-gaps, and contract-freeze documents; and
- retains mfm-signing and mfm-keystore as independent platform primitives while removing their
  submission-only App/live-EVM dependency edges.

No disabled entry point, compatibility decoder, placeholder nonce type, private unsupported
submission assembly, generic Effect mode, or synthetic mutation fixture remains.

### 10.9 App and transports

- accept an explicit fresh RunId on every future start route and echo it before execution so exact
  admission retry is possible;
- call only Runtime start/resume/read APIs;
- may stop polling start/resume on timeout or disconnect because ordinary future drop is
  safety-neutral; a later request continues from the durable prefix;
- optionally apply one coarse operational bound to simultaneous top-level Runtime calls;
- render RunView and reviewed redaction-safe errors;
- retain no RunSession, SuspendedRun, pending owner, or frame-derived status;
- remove fixed-tenant facade state;
- remove current trace/audit/replay endpoints unless separately reintroduced under a reviewed
  transport contract;
- delete the unused generic parse_admission_json/MAX_ADMISSION_BYTES surface with the old
  Admit/Drive API; a future real start transport owns its typed request parser and bound; and
- update CLI/REST documentation and fixtures in the same cutover.

The current CLI and REST inventory contains no run-progression route. This is therefore a frozen
contract for a later start/resume transport, not a hidden compatibility obligation for an existing
endpoint.

### 10.10 Replay

Delete mfm-replay, its workspace membership, dependencies, DTOs, and reducer. Runtime read is the
one supported Store-backed semantic inspection path; no portable replacement remains.

## 11. Complete deletion checklist

The cutover is incomplete while any of these supported concepts remains:

~~~text
ProgramCatalog
ProgramCatalogBuilder
ProgramCatalogInner
application_catalog
snapshot_closure_document
validate_runtime_closure
PortfolioAdmissionPlan
ProgramIngress
ProgramRef wrapper
public ProgramDocument/wire DTO separate from checked Program
ValueAssociation
CapabilityAssociation
ReifyValue
public Program canonical_value helper
public/program-owned QualifiedValue
public QualifiedTypedValue
QualifiedTypedValue::erase
public try_downcast
catalog brand
ProgramError::InvalidValue
ProgramError::InvalidCatalog

PublicOutputDescriptor
StateInput marker trait/derive
OperationOutput marker trait/derive
PublicOutputs marker trait/derive
SchemaKind::StateInput/OperationOutput/PublicOutput
FieldSegment/FieldPath
CapabilityKind/CapabilityKindKind
CapabilityVersion/CapabilityVersionKind

DynamicOutcome
DynamicPreparationFailure
DynamicStateRegistration
DynamicPrepared
DynamicCommit
DynamicCall
DynamicResolution
AccessMode
ReadMode marker
EffectMode
EffectKind
EffectVersion
Effect kind/version ID types
AccessCapabilityContract
AccessEvidenceValue marker
mfm-capabilities ProposedStateOutcome export (sole type moves to mfm-program)
PureImplementation
ReadImplementation
AccessImplementation
EvmPureState/EvmAccessState/PortfolioPureState duplicate behavior traits
AccessResolution
AcceptedOutcomeAccess
AcceptedIntegrityAccess
UnresolvedAccess
UnresolvedClassification
ReadResolution
ReadObservation
BlockedIntegrityProjection
PreparedExecution
PreparedAccess
PreparedRead
CommittedCall
CommittedRead
PreparationRef
QualifiedRecordedEvidence
AccessIngressFuture
public `'static` BoxFuture<T> alias (only the private borrowing alias remains)
OpenedPreparationCommit
QualifiedAdapter
Runtime-owned PreparationError
Read attempt/replacement counter or relation
RunView Waiting state
HotValue
public/composite StateExecutionKey
separate Runtime/adapter call_id
RunSession
SuspendedRun
ParkedRun
ParkReason
PendingConclusion
AdmissionInput
standalone AdmissionFailure/AdmissionConflict lifecycle types
ResumeFailure
TerminalRun
RuntimeLimits
RuntimeError::Conflict
NoLongerSelected
SpawnStep
ResumeStep
RuntimeStep
PendingAppend
PendingWork
PendingKey
acknowledgement lease
completion/finalization cell
pending-owner limit or permit
resolver token
Runtime cancellation API/token/state
Runtime active-session/CPU/planning/ingress semaphores
max_active_sessions
max_cpu_jobs
max_planning_jobs
max_ingress_jobs
active_sessions
cpu_jobs
planning_jobs
ingress_jobs
Runtime semaphore acquire helpers and OwnedSemaphorePermit fields
Runtime timeout policy
Runtime detached completion task/finalizer

StoreBrand
StructuredStore
OpenedStructuredStore
StoreParts
QualifiedHistoryPort
HistoryReader
StoreAuditPort
QualifiedRun
RunReducer
RunSelection
RunAction
SelectedRun
PreparedAdmission
PreparedConclusion
SelectedConclusion
FactContinuation
SemanticStore
replay_terminality
retained_program
validate_prefix
AccessActionMode
AccessPreparationOutcome
AdmissionOutcome
SelectedConclusionOutcome
SelectedConclusionPreparationOutcome

StructuredStoreBackend
MemoryStructuredBackend
dyn-Store check_ready/readiness method
mfm-store ReadyError/StoreOpenError exports (StoreOpenError moves to mfm-storage-postgres)
mfm-storage-postgres DurabilityProfile
mfm-storage-postgres broad PostgresError/Result alias
public PostgresStore::from_pool
public PostgresStore::rotate_identity
public PostgresStore::check_ready/profile
public PostgresStore::migrate/install schema method
StoredRunBytes captured head sequence/digest
StoreWorkLimits
RawHistoryLoadLimit
BackendFuture/BackendResult/BackendError aliases
BackendAppendCommand
BackendAppendOutcome
AppendDisposition
AppendResult Existing/Stale variants
RunAppend
public RunPosition
Current/Exact load selector
run listing/pagination API
RawRunPrefix
RawFrameBytes
RawFactPublication
RawFactSnapshot
append request row
configuration request row
separate receipt row
AppendRequestId
IdempotencyConflict
AcknowledgementUnknown result
UnavailableBeforeSubmission
InvalidPhysicalCommand
AlreadyConcludedSame
expected_position field
append command digest or digest index

FailureValue
optional State failure contract
Program root_contract_ref v1
SequentialControlAddress and any replacement public StateAddress
declaration address, next_address, failure_next_address, and Match entry_address fields
declaration-address sorting/map, root inference, and cycle DFS
State terminal
State maximum_conclusion_bytes
State/Read total_attempt_bound
Execution fact_selection_required
Execution effect_domain
separate optional execution_binding field
AccessBindingV2 and mfm-access-binding@2 wire
public BindingDescriptor and mfm.execution-binding@1 wire
EvmBalanceBindings and any one-variant live-binding wrapper
EvmAdapterBinding
EvmLiveAssembly contribution wrapper
live-owned EvmPhysicalTarget definition (the sole type moves to mfm-evm)
RunAdmitted entry_point field
StatePrepared/state_prepared record
PreparationMode
State conclusion occurrence field
StateConcludedAccess/state_concluded_access record
State conclusion preparation_sequence field
MatchVariant payload_contract_ref/continuation_contract_ref fields
public/open RunFrame/RunRecord/RunAdmitted/StatePrepared/StateConcluded DTOs
public/open StateOutcome/ImmutableObject/ValueRef/RecordLogicalKey DTOs
open Journal DTO constructors plus validate choreography

mfm-facts
FactSelectionMode
NoPriorFacts
PriorRunFacts
FactSelection
FactProposalSet
FactHead
FactPublication
fact publication
fact proof
fact SQL

MfmConfig
ValidatedConfig
ProvenConfiguration
ConfigurationKey
ConfigurationPosition
ConfigurationRef
ConfigurationRevision
ConfigurationAppend
ConfigurationHeadProjection
ConfigurationStore
ResolvedConfiguration
ResolvedConfigurationHead
ConfigurationWriteSession
PreparedConfigurationAppend
SuspendedConfigurationAppend
ConfigurationAppendCommand
ConfigurationCommitOutcome
ConfigurationAppendDisposition
BackendConfigurationOutcome
RawConfigurationRevision
configuration publication/load API
configuration revision/head/request/receipt SQL
Program configuration contract
configuration envelope middleman
duplicated configuration schema ref
MAX_CONFIGURATION_REVISIONS
MAX_CONFIGURATION_REVISION_BYTES
MAX_CONFIGURATION_STREAM_BYTES

PortableRunBundle
PortableRun
ExportedRun
export_run
portable export/inspection API
portable decoder or semantic ingress

record_digest
separate semantic record hash/proof type

per-State or schema-recursive conclusion-size proof
Open/Replace/Consume reservation instruction
ReservationKey
run_reservations
reserved_bytes
reserved_frames
durable conclusion reservation/accounting
MAX_STATE_CONCLUSION_BYTES
old MAX_RUN_FRAME_BYTES constant

first-reference object closure/dictionary
MAX_RUN_OBJECTS
object_count
reserved_objects
object-slot reservation/accounting

StoreScopeId
StoreEpoch
TenantScopeId
StructuredStoreIdentity
active identity
identity rotation
database incarnation

global object table
per-run object membership

mfm-replay
separate replay reducer
App suspended map
App frame-derived status
AdmitRunRequest without explicit RunId
parse_admission_json/MAX_ADMISSION_BYTES
AdmitRunResponse
DriveResponse
App-owned RunStatus/PublicRunView
ReplayResponse
TraceResponse/TraceRecord
AccessAuditResponse/AccessAuditEntry
ExportedRun response DTO
mfm.evm/submit-transaction@1 entry point
EVM_SUBMIT_TRANSACTION_ENTRY_POINT_ID
EvmConfig
EvmTransactionTarget
EvmSubmission* public types
NonceReservationIntent/NonceReservationEvidence
BroadcastIntent/BroadcastEvidence
plan_submission
submission_closure_documents
wallet_nonce_effect_domain
public_signer_key_instance_ref
evm_live_adapter_implementation_ref
PostgresWalletNonce*/WalletNonceDomain* public exports
EvmProviderResponse::Broadcast/PossibleEntry variants
submission-only EvmAdapterError variants
EvmReadSubject::TransactionReceipt/FinalizedHead/CanonicalInclusionBlock variants
EvmReadValue::Receipt/FinalizedHead/CanonicalBlock variants
ReadCapabilityFamily::SubmissionStatus and EvmCapability<3> implementation/IDs
EVM submission request/progress/output/failure and plan/binding surface
EVM nonce-reservation/candidate/broadcast/status States and capabilities
WalletNonceAuthority and every nonce-domain/operation/activation type
mfm-storage-evm-postgres and nonce SQL
live-EVM signer/broadcast/nonce adapter integration
App EVM-submission dispatch and composition
SINGLE_TRUST_BOUNDARY_CUTOVER_MANIFEST.md prior-target inventory
IMPL_PLAN_RFC_REFACTOR_SINGLE_TRUST_BOUNDARY.log prior-target handoff record
~~~

Generic words such as state, scope, tenant, fact, trace, or audit may remain in unrelated prose or
domain concepts. A responsibility explicitly transferred above survives only at its one named
target—for example reduction in Runtime and exact-row idempotency in Store. Responsibilities listed
as absent, rather than transferred, may not survive under a new name.

The replacement negative scan is concept- and public-export-based, not only a literal identifier
grep. It inventories symbol/reexport roots, stable/schema IDs, wire tags/fields, SQL tables/columns,
manifest/dependency edges, fixtures, and current documentation claims. Renaming one listed owner or
leaving an opaque compatibility wrapper fails the cutover.

Production LOC, exported type count, dependency edges, and files required to add a State must all
decrease.

## 12. Security and failure semantics

- Secrets never enter Program, `C0`, Journal frames, RunView, outputs, Store physical metadata, SQL
  keys, logs, or error details. Domain types and adapters keep secret material outside persistable
  MfmValue values; Runtime qualification rejects known secret markers before Journal construction.
  Journal makes no dishonest claim to discover an arbitrary secret encoded as an ordinary string.
- Hashed structured values contain no floats.
- Journal and value canonicalization use fixed domain-separated SHA-256 recurrences and strict
  canonical bytes.
- Content hashes prove byte correlation and detect corruption; they are not external signatures
  and do not claim Byzantine database authorship.
- Runtime State logic performs no ambient IO. Provider, network, filesystem, signer, and storage IO
  cross explicit adapters or Store.
- Every registered capability is an externally observational, duplicate-safe Read. Its closed typed
  Evidence owns durable provider rejection, safe-failure, and integrity-block classifications.
- One selected Read is entered at most once per start/resume invocation. ReadUnavailable writes
  nothing; a later invocation may safely observe it again.
- No success is exposed before the durable frame retaining the exact terminal value is observed:
  genesis for a zero-State Program, or a conclusion otherwise.
- Every Store failure for which COMMIT may have reached PostgreSQL and the candidate may have
  committed is Indeterminate.
- Dropping start/resume at any await is safety-neutral: it adds no candidate frame, or the single
  in-flight append may commit atomically; every already-acknowledged durable prefix remains, and the
  next complete load resolves the in-flight candidate. Runtime owns no cancellation mechanism.
- spawn_blocking owns only synchronous pure work, is immediately awaited, and contains no IO or
  persistence authority.
- Store append is atomic per frame and exact-head linearized.
- Frame rows are immutable; exact historical retry returns NotInserted and writes nothing.
- Process termination loses only volatile Pure/Read work; restart trusts only durable history.
- MAX_RUN_OBJECT_CANONICAL_BYTES plus the fixed non-payload envelope proves every accepted Read
  intent/evidence/outcome triple fits one frame.
- Only a no-loss monotonic PostgreSQL timeline has persistence authority. Writable rewind, data-loss
  failover, and writable historical clones are unsupported even after old writers stop.
- Public errors are reviewed and redaction-safe.

## 13. Tests and verification contract

### 13.1 Program, value, and compile-time proofs

- ProgramDocumentV2 goldens freeze every field/tag, literal null successor fields, canonical order,
  schema identity, and rejection of inline binding, physical-target, adapter-implementation, and
  every removed v1 field; Program is the sole public complete-Program construction/decode type and
  the wire projection is not a second public DTO;
- declaration-array order is identity-bearing; deterministic constructors preserve authored
  topological order and two valid sibling orders intentionally produce different Program refs;
- entry_point_id uses exact EntryPointId grammar, non-empty root is index zero, and empty declaration
  identity plus root State and root Match cases are covered;
- the checked target-field parser/constructor accepts integer 65,535 and rejects 65,536, while
  whole-Program root-zero, out-of-bounds/self/back-edge, unreachable, branch/rejoin, and
  common-failure-handler fixtures stay within the independent 8 MiB Program ceiling and prove the
  forward-index graph contract;
- Program rejects missing/extra/incompatible input, output, failure, Read, and Match contracts;
- checked Program construction/decode exposes only ProgramError::Canonical/InvalidContract/Capacity;
  a State/capability identity failure during checked declaration reference derivation is
  InvalidContract, while the same static failure during RuntimeAssembly registration/association is
  IncompatibleAssembly;
- public declaration-authoring compile tests cover only the exact checked constructors from Section
  3.2.1; they accept derived invariant-bearing refs, while complete decode remains Program-only;
- terminal success and failure must match the exact Program root success and failure contracts;
- State success/failure successor inputs must match the complete corresponding result contract;
- a failure successor targeting Match directly is rejected; variant routing requires the ordinary
  Pure-router-plus-Match lowering;
- Program rejects malformed/duplicate Match arms, and ExecutableProgram association rejects
  missing, unknown, non-exhaustive, wrong-payload, or wrong-tagging-profile arms against the exact
  registered descriptor;
- Match tag-order goldens cover a prefix and punctuation/digit boundary under the exact raw-byte
  comparator;
- each Match arm must target a State whose input equals the registered selector descriptor's payload
  contract for that tag; no separate payload- or continuation-contract field exists;
- the current EVM-child failure route reaches the Portfolio mapper and terminates only as exact
  PortfolioSnapshotFailure;
- zero-State identity is accepted only when C0 equals root success and root failure is Never;
- Never has no constructible or decodable value;
- only the exact top-level `mfm.kernel/never@1` registration admits an externally tagged Enum
  descriptor with zero variants; goldens freeze its semantic digest, canonical SchemaIdentity,
  SchemaId, and nominal contract ref, while a variant-bearing Never descriptor, foreign/nested empty
  Enum, or attempted Never payload fails;
- a present failure successor for Never is rejected;
- Never in admitted-context, input, output, root-success, or Match contracts is rejected and no
  Match projection is registered for it;
- Pure and Read registration with Never succeeds when the typed implementation can return only
  success;
- state_implementation_ref and capability_contract_ref v1 preimage/reference goldens remain exact,
  while explicit State and capability-associated value contracts reject same-ID ABI drift;
- ReadCapabilityContract compile tests expose only Intent/Evidence/contract_id/bind_evidence plus
  CapabilityError::InvalidContract/EvidenceBinding; no mode, attempt, fact, evidence-marker, or
  qualified-recorded-evidence API remains;
- PureState, ReadState, ProposedStateOutcome, and ReadPreparationError live in mfm-program;
  Runtime exposes no Pure/Read implementation-holder or domain-local duplicate behavior trait, and
  builder compile tests cover only register_value/register_pure/register_read/register_adapter;
- exact repeated value/State contributions are idempotent, an inconsistent stable association and
  every duplicate capability/binding-ref adapter key are rejected, and the adapter method's HRTB
  signature compiles on stable Rust;
- every pre-cutover EvmConfig field is retired as submission-only, while every PortfolioConfig field
  is classified as Program, typed C0, or process-local binding; changing retained secret-free policy
  changes the corresponding Program/C0 golden, while replacing credentials or a client handle for
  the same public physical target does not change durable bytes;
- plan_snapshot returns only the exact `(Program, PortfolioSnapshotInput)` pair, and no admission
  wrapper, source_refs, dummy closure document, or application catalog/closure validator remains;
- cold resume/read succeeds after the authoring Config values have been dropped;
- persistable value qualification rejects every repository-defined secret marker, and no test
  claims arbitrary-string secret detection;
- the Read adapter HRTB borrows exact typed intent only for its future lifetime and exposes no
  public call/preparation/append token;
- adapter binding registration uses the ordinary bounded/known-secret-marker-rejecting MfmValue
  qualification and rejects a marked-secret descriptor before deriving binding_ref;
- no public erase/downcast or catalog branding path compiles; and
- obsolete StateInput/OperationOutput/PublicOutputs schema/derive families, FieldPath support, and
  CapabilityKind/CapabilityVersion identities have no surviving export or consumer.

### 13.2 Journal goldens

Golden fixtures freeze:

- every RunFrameV1 record and outcome variant;
- null genesis predecessor and non-genesis predecessor representation;
- canonical field names, tagged variants, number representation, and object order;
- nested raw canonical object representation;
- repeated cross-frame references with complete frame-local objects;
- the run-head recurrence;
- exact run/object limits and their independent bound-plus-one behavior;
- an exact 8 MiB run object and rejection at one byte over;
- a fixture with every non-payload field at its maximum whose measured overhead remains within the
  65,536-byte proof allowance; and
- every three-object frame remaining at or below the derived MAX_FRAME_BYTES, with an over-bound raw
  frame rejected before semantic decoding.

Negative fixtures cover:

- non-canonical bytes, floats, duplicate/unknown fields, and invalid UTF-8;
- wrong sequence/predecessor/head/domain, a frame RunId differing from its Store row, and a chain
  RunId differing from genesis or the requested RunId;
- missing, duplicate, conflicting, out-of-order, and extra frame-local closure objects;
- bad object content digest or contract/schema binding;
- invalid record tag/shape and genesis kind, including every removed preparation/Access record and
  occurrence/preparation-sequence field;
- failure under Never.

### 13.3 Runtime semantic tests

- start/resume/read return the same RunView/RuntimeError contract and expose no driver diagnostic;
- a fixed-limit violation found in retained history is InvalidHistory, while the corresponding
  local construction/append violation is Capacity; a supplied trusted Program/typed-C0
  contract mismatch returns Internal before append;
- hot and cold fold produce the same RunView;
- Store-backed read invokes no State, adapter, or provider callback;
- hot typed adapter ingress borrows the exact typed intent while Runtime retains its private
  qualification proof, and Runtime applies C::bind_evidence before accepting evidence;
- cold fold repeats only the pure C::bind_evidence(intent, evidence) relation and rejects a
  mismatched retained Read conclusion as InvalidHistory without a persisted call ID or second
  callback; the equivalent hot bind failure appends nothing and returns Internal;
- Match reduction uses the one schema-derived projection, and hot/cold folds produce the identical
  selected tag, canonical payload bytes, contract, and derived content ref without reserialization;
- private dispatch rejects partial State/capability/binding-ref associations while exposing no
  composite registry key;
- one State implementation registration is reusable across routes, one EvmPhysicalTarget ref may
  serve several exact S/C registrations, and duplicate private capability/binding-ref
  registration is rejected;
- the sealed EVM live adapter constructor derives its ref from the validated public target and captures the
  trusted-composition provider handle; endpoint or descriptor-schema changes alter the ref,
  credential/client-handle replacement for the same public identity does not, and a caller-asserted
  ref is never accepted as proof;
- the `mfm.evm-physical-target@1` canonical golden freezes exact chain_id/endpoint_ref fields and
  exact `mfm.evm/physical-target@1` semantic identity, and the derived binding ref; it rejects zero
  chain_id, duplicate planner targets for one chain, any extra field, every old binding-wrapper
  shape, and any caller-asserted ref;
- restart with the same descriptor recreates the same ref; missing or wrong S/C/ref registration is
  IncompatibleAssembly before any external call;
- binding_ref is an external assembly association, not a Journal closure object, and changing it
  changes Program identity and the recursively committed genesis/head;
- every Inserted frame is folded by local exact extension without a Store reload; NotInserted and
  cold entry take the complete-load path;
- success and failure terminal views always carry the Program's exact root result contract;
- a cold EVM child failure reaches MapEvmBalanceFailure with its contract, reference, and canonical
  bytes unchanged;
- the mapper's failure becomes exact root PortfolioSnapshotFailure and suppresses every later normal
  State; EvmReadEvidence::IntegrityBlocked reaches the same exact typed route through the ordinary
  shared interpreter;
- a small synthetic recovery handler proves child success and handler success rejoin the same next
  State under identical hot and cold folding;
- ReadUnavailable appends no frame, maps to RuntimeError::Unavailable, and leaves the same State
  Runnable for a later invocation;
- ReadPreparationError appends no frame and maps to RuntimeError::Internal, while expected domain
  rejection flows only through typed Evidence and ProposedStateOutcome::Failure;
- one start/resume invocation enters a selected Read adapter at most once;
- concurrent Read observations may overlap, but exact-head append selects one durable fused
  conclusion and every NotInserted caller reloads that winner;
- a fused Read conclusion retains the exact intent, evidence, and typed-outcome canonical objects;
- Pure work safely recomputes after an indeterminate conclusion later found absent;
- zero-State and nonzero-State tests prove no RunView success appears before the durable genesis or
  conclusion frame, respectively, that retains the exact terminal value;
- an earlier RunView remains a valid captured snapshot if a formerly indeterminate transaction
  commits later.

### 13.4 Store conformance

The same suite runs against Memory and PostgreSQL:

- Store remains object-safe behind Arc<dyn Store> with borrowing boxed futures;
- absent-genesis race;
- exact target-row/frame retry before and after later head advancement;
- different-frame same-head race and occupied-target corruption;
- historical exact retry returns NotInserted;
- a stale predecessor returns the same NotInserted and performs no mutation;
- Inserted writes immutable frame, head, and actual-byte accounting atomically;
- rollback leaves no partial frame/head state;
- Memory fault injection proves that every fallible step precedes its one frame/head/total publish
  and that every returned error leaves all three unchanged;
- run-byte and frame-count exact-bound/bound-plus-one behavior;
- NotInserted bypasses insertion-capacity checks and leaves head/total unchanged;
- frame-local object bytes are charged exactly once as part of their containing frame;
- complete current loads from one snapshot under concurrent append;
- gaps, wrong heads/digests/totals, and over-format-limit histories return CorruptPhysicalState;
- absent head with either a candidate-target or noncandidate orphan frame is rejected before
  genesis insertion;

PostgreSQL-only integration and fault-injection tests additionally prove:

- PostgreSQL load begins REPEATABLE READ READ ONLY before its first snapshot read;
- PostgreSQL append acquires the transaction advisory lock before target/head reads and forces
  synchronous_commit=on;
- advisory-lock key goldens freeze the JCS domain, SHA-256 truncation, big-endian signed mapping,
  and one-argument PostgreSQL call; independent processes serialize the same RunId;
- PostgreSQL open rejects recovery mode, disabled fsync/full_page_writes, non-logged authority
  tables, and the wrong static schema baseline as Incompatible, while failure to establish the
  observation is Unavailable;
- no usable production PostgreSQL Store handle can be constructed before those checks succeed, and
  dyn Store has no redundant readiness method;
- pool reconnect after PostgreSQL restart repeats the gate before admitting the new connection;
  a failed reconnect gate rejects that connection and returns StoreError::Unavailable before append
  submission, while in-place prerequisite changes require Store reconstruction;
- every database/pool availability failure known not to have committed returns Unavailable and
  performs no write;
- only injected COMMIT acknowledgement ambiguity returns Indeterminate;
- later load distinguishes committed from rolled-back indeterminate cases; and
- no AppendRequestId, command digest/index, request/receipt row, reservation instruction/table/
  counter, Exact selector, pagination API, object table, membership table, configuration table,
  identity, or fact table exists.

### 13.5 Cancellation, blocking, and live-IO tests

Boundary-focused tests and repository checks cover:

- Runtime exposes no cancellation token, timeout wrapper, detached completion task, or semaphore;
- spawn_blocking closures contain only synchronous deterministic work and are awaited before
  provider or Store IO;
- dropping the outer future while spawn_blocking is running may let only that pure closure finish;
  its result is discarded and it owns no IO or append authority;
- the exact public adapter shape is a stable-Rust higher-ranked borrowed-intent callback returning a
  boxed Send future of typed Evidence or zero-detail ReadUnavailable;
- adapter deadline/transport unavailability returns ReadUnavailable, appends no frame, and maps to
  RuntimeError::Unavailable;
- Store failures definitely before commit return Unavailable and COMMIT ambiguity returns
  Indeterminate;
- dropping start/resume before or during a provider Read leaves no external mutation and a later
  invocation may observe the same durable Runnable State;
- dropping before conclusion append adds no candidate frame; dropping during append may leave the
  one candidate committed or absent, all earlier durable frames remain, and later complete load
  resolves it;
- conclusion post-commit acknowledgement loss is later observed durably;
- conclusion rollback leaves the same Read Runnable and safely re-observable;
- Pure conclusion commit/rollback ambiguity is safely folded/recomputed;
- process termination at every Read boundary has the same durable-prefix behavior; and
- an optional composition-level top-call bound does not enter Runtime proof or persisted state.

### 13.6 EVM product-scope tests

- the exact production entry-point inventory retains Portfolio and contains no EVM submission;
- App rejects the retired `mfm.evm/submit-transaction@1` ID before run admission;
- Portfolio native/token Reads, hot/cold fold, and child-failure mapping remain unchanged;
- live EVM assembly requires only Read bindings and has no nonce-authority or signer dependency;
- no submission State/capability/contract ID, selector, request/progress/result, binding, fixture,
  migration, SQL table, or production registration survives;
- surviving EvmProviderResponse, EvmReadIntent, EvmReadValue, and EvmAdapterError wires contain no
  broadcast/possible-entry/receipt/finality/canonical-inclusion or submission-only branch;
- mfm-storage-evm-postgres is absent from the workspace, manifests, and lockfile; and
- current product documentation advertises no EVM submission path and records that a future path
  requires a durable transaction authority rather than nonce-only allocation.

### 13.7 Absence and complexity evidence

Record:

- workspace dependency graph before and after;
- exported production types before and after;
- net production LOC before and after;
- files/registrations required to add one Pure and one Read State;
- Store production LOC removed by semantic/fact/identity/request/object deletion;
- one canonical Journal construction/decoding path;
- one Runtime reducer;
- one Runtime assembly registry;
- zero Runtime semaphore/cancellation/detached-completion paths;
- one sealed-frame append path and no Store command algebra or durable reservation path; and
- repository searches proving every concept in the deletion checklist is absent from supported
  core APIs, wire, hashes, schema, App, and transports.

Verification follows docs/build-and-verification.md. Run narrow owner-specific checks first,
expand only for affected boundaries, and run the composed CI gate once after narrower failures are
resolved.

## 14. Ordered implementation commits

### 14.1 remove unsafe evm transaction submission

After explicit resolution of the Material uncertainty in favor of deferral, remove the complete
submission path in one coherent EVM-domain/live-adapter/App/storage/workspace/test/documentation
commit:

- retire the submission entry-point and every submission-only State, capability, typed value,
  planner, binding, dispatcher, adapter handle, and fixture, including EvmConfig with no empty
  replacement;
- delete WalletNonceAuthority, signer/broadcast integration from live EVM, and
  mfm-storage-evm-postgres with its migration and dependency edges;
- delete all six `docs/contracts/evm-portfolio/evm-submission-*.json` contracts plus the submission
  branches of capacity-app and EVM integration tests/tasks, and delete the submission-only
  `docs/evm-transactions.md`;
- retain and verify Portfolio plus its EVM Reads and keep standalone signing/keystore primitives;
- rewrite the advertised product entry-point inventory and document the durable transaction
  authority required before submission may return; and
- retain no disabled entry point, compatibility type, schema remnant, or fallback.

This product-scope deletion is independently coherent and precedes the core cutover so that core
planning contains no nonce-only protocol mistaken for complete transaction authority. Shared
generic Effect machinery temporarily has no production registration or consumer only
because its deletion is inseparable from the RunFrame/Runtime fused-Read cut in Commit 14.2; that
second commit deletes the complete core Effect surface rather than preserving it as an extension
point.

### 14.2 establish the typed runtime journal and store proof path

In one inseparable workspace-wide cutover, including ids, values/derive, Program, capabilities,
domains/live adapters, Journal, Store/Memory/PostgreSQL, Runtime, App, CLI/REST, workspace manifests,
Cargo/Nix task wiring, tests, and documentation:

- install mandatory exact State failures, zero-variant Never, exact root success/failure contracts,
  common failure continuations, complete Read associations, and the retained State-or-Match graph;
- install RunFrameV1, recursive heads, frame-local closure, the universal 8 MiB run-object envelope,
  and all canonical goldens;
- add RuntimeAssembly, private exact dispatch, the sole hot/cold reducer, local fold_next,
  fused Read conclusions, borrowed-intent adapters, cancellation-safe progression, and RunView;
- reduce Store directly to load_run/append_run plus checked concrete PostgreSQL open, natural
  target-row idempotency, complete snapshot loads, exact-head append, and actual capacity accounting;
- make PortfolioConfig a process-local authoring input, have its planner return only the exact
  Program and typed C0, and keep live bindings in assembly/adapters;
- migrate App/transports to Runtime APIs and the explicit-RunId contract;
- delete the complete configuration, fact, portable/replay, identity, command/receipt, reservation,
  global-object, Store-reducer, Effect/preparation/call-owner, public lifecycle,
  cancellation/semaphore, and old catalog/reifier surfaces named by this RFC;
- replace the superseded cutover manifest/negative scan with this RFC's absence inventory and remove
  the prior implementation progress log from implementation inputs;
- rewrite baseline schemas and fixtures with no legacy reader or compatibility adapter; and
- delete the one-time `docs/preflight/capability-binding-inventory.md`,
  `docs/preflight/capacity-envelope.md`, and
  `docs/preflight/cumulative-context-abi.md` artifacts because their surviving contracts live in
  this RFC plus executable goldens/tests;
- rewrite `README.md`, `docs/design.md`, `docs/architecture.md`,
  `docs/build-and-verification.md`, `docs/run-execution.md`,
  `docs/persisted-public-surfaces.md`, and `docs/known-gaps.md`; rewrite
  `docs/evm-rpc-routing.md` for direct EvmPhysicalTarget Read bindings and
  `docs/evm-portfolio-contract-freeze.md` for Portfolio-only scope; delete the speculative
  `docs/btc-rpc-routing.md` note; rewrite
  `docs/portfolio-snapshot.md` for direct target refs, implicit declaration indices, and the retained
  failure mapper; update affected crate and binary READMEs;
- reset surviving Portfolio/core goldens and update `nixfied.nix` capacity-app/runtime/store/envelope
  tasks plus negative-scan/CI edges, workspace manifests, Cargo.lock, and core migrations; and
- delete `SINGLE_TRUST_BOUNDARY_CUTOVER_MANIFEST.md` and
  `IMPL_PLAN_RFC_REFACTOR_SINGLE_TRUST_BOUNDARY.log`; add one newly named
  `RUNTIME_STORE_JOURNAL_CUTOVER_MANIFEST.md` derived from Section 11; rewrite
  `scripts/check-cutover-manifest.sh` in place to consume it; and keep the existing negative-scan
  task name while updating its docs/CI edges and scope-selected verification in that same commit.

The current Store reducer and exported SuspendedRun/PreparedAdmission/PendingConclusion lifecycle
cannot coexist coherently with the final no-owner semantics. They therefore disappear in this same
cutover rather than being adapted to a temporary RunAppend surface. At the commit boundary Runtime
is the one semantic owner and Store is purely mechanical; no commit leaves zero or two reducers.

Commit subjects are lower case exactly as shown by the section titles.

## 15. Rejected alternatives

### Split Store files without changing ownership

Rejected. Navigation improves while the same reducers, validators, DTOs, registries, and future
change sites remain.

### Keep Store reduction

Rejected. Runtime would still execute a persistence layer's semantic decision and inspection would
still require Store semantics.

### Remove every erasure

Rejected. A retained heterogeneous Program selects different Rust State families at runtime. One
private exact-key registry dispatch is unavoidable.

### Delete failure continuations or expose child failures directly

Rejected. The supported Portfolio operation already routes each EVM child failure through
MapEvmBalanceFailure and promises PortfolioSnapshotFailure at its root. One optional common failure
successor preserves that current product contract and is directly reducible from retained bytes.

### Import State-only Operation lowering or FailureNext::ByVariant now

Rejected. The current Program needs Match, the current mapper needs only one common failure edge,
and no supported consumer needs variant-specific recovery. Importing the deferred Operation DSL
would reopen unrelated authoring and Application choices. That RFC must lower its typed policies
into this one retained graph and use an ordinary Pure router plus Match if a real future consumer
needs variant routing.

### Keep MFM-owned EOA submission with nonce allocation alone

Rejected. A committed nonce reservation does not guarantee that the exact transaction using that
nonce reaches or remains in the chain. Process loss, signing/provider failure, definite broadcast
rejection, ambiguous submission, or transaction eviction can strand nonce N while later work uses
N+1. Recovering only the reservation receipt leaves the later gap unchanged, and “retire the
sender” is not enforceable across crashes and peers without another durable fence. A supported
future EOA submission path requires one durable per-sender transaction authority/outbox that owns
allocation, exact signed bytes, retransmission/reconciliation, and higher-nonce fencing. That is a
separate product RFC, not an addition to this core cutover.

### Keep generic Effect or its preparation protocol for Read

Rejected. No production Effect consumer survives the submission cut, and a non-mutating Read needs
no durable authority before provider entry. StatePrepared, attempt/replacement accounting,
CommittedCall, Waiting, and future-conclusion preflight would add a second frame and lifecycle to
every successful Read solely to preserve synthetic mutation semantics. A fused Read conclusion plus
exact-head append proves the retained observation; a future mutating consumer must use its real
durable command/outbox contract instead of restoring this generic protocol.

### Add Runtime cancellation or pending-result custody

Rejected. Ordinary Future drop is already safety-neutral for Pure/Read execution. Runtime
cancellation tokens, timeout branches, owner tables, retry leases, and cancellation-specific states
would add a second lifecycle without preserving another result or authority.

### Put provider interpretation or append in a detached finalizer

Rejected. A detached finalizer creates another lifecycle owner without solving process termination.
Synchronous pure work is awaited through spawn_blocking; provider and Store work remains awaited
live IO under Adapter and Store policy.

### Wrap State execution and Store append inside spawn_blocking

Rejected. spawn_blocking protects the async executor from synchronous CPU work; it is not a wait or
durability owner. Store append is async live IO and must not be driven with block_on or a blocking
database client inside the closure. An already-running pure closure may finish after outer Future
drop, but it cannot perform dependent IO or append.

### Keep random append request ids

Rejected. The immutable natural `(run_id, sequence)` target row plus exact canonical frame bytes
already provides exact historical idempotency. Separate request identities add conflict states and
tables without proving another property.

### Keep a separate command/receipt table

Rejected. Every successful command creates exactly one immutable frame. That row can retain the
exact accepted bytes and derived head. A command digest, expected-position field, instruction, or
separate receipt duplicates data already sealed by the frame.

### Keep durable conclusion reservations

Rejected. Pure and Read each append one complete conclusion only after it exists. Store accounts the
actual candidate atomically; there is no earlier durable preparation or future liability to reserve.
Rows, counters, keys, and Open/Replace/Consume transitions would add state without preserving
another reachable obligation.

### Derive a different conclusion maximum for every State schema

Rejected. A recursive canonical-JSON maximum calculator is a second schema algebra and a large
trusted surface. One 8 MiB run-object ceiling plus a frozen 65,536-byte non-payload envelope proves
every three-object Read conclusion fits while retaining ample margin for all current objects.

### Reload and refold the complete run after every Inserted

Rejected. That makes a hot run quadratic in retained frames. Runtime already holds the exact frame
whose Inserted result it observed; extending one private accumulator and calling fold_next preserves
the same proof. Full reload remains mandatory after NotInserted, cold entry, and ambiguity.

### Add a HotValue cache or second derived-value representation

Rejected. It creates separate hot and cold semantic paths before measurement demonstrates a need.
The one qualified canonical value representation is sufficient.

### Keep a first-reference object dictionary

Rejected. Cross-frame deduplication makes frame construction and qualification depend on a
cumulative object map and adds object-count/reservation arithmetic. RunFrameV1 names at most three
direct objects. Re-embedding every directly named object makes closure local and lets the existing
canonical-byte limit account for the complete cost.

### Keep a separate semantic record digest

Rejected. The closed qualified record fields and their content references already define exact
semantic equality. Another digest adds a golden contract and identity without avoiding any
comparison or persistence requirement.

### Store objects globally

Rejected. Exact frame-local objects make retained frames self-contained and give one byte-accounting
model. Physical deduplication is not a semantic requirement.

### Retain facts as empty extension points

Rejected. Empty markers, fields, crates, tables, and capability modes preserve concepts and future
change sites without a consumer. A future fact system must be designed from its real contract.

### Keep independently published configuration

Rejected. Reproducible secret-free policy belongs in Program or typed `C0`; deployment clients and
credentials belong in assembly/adapters. A configuration stream adds a second value lifecycle,
latest lookup, CAS, revisions, SQL, retry semantics, and admission coupling without a current
consumer that needs independent publication.

### Keep portable export and offline inspection

Rejected. No current core consumer needs a second complete-history ingress or export format.
Runtime read over Store is the one semantic inspection path. A future portability feature must
start from a concrete external consumer and cannot reserve codecs or Runtime entry points now.

### Keep tenant, scope, epoch, or a renamed incarnation

Rejected. They do not belong to the selected one-database core authority. Restore fencing is an
operator/deployment concern outside this contract, and an incarnation stored inside a rolled-back
database rolls back with it. The selected deployment contract forbids writable rewind.

### Keep mfm-replay or public trace/audit lifecycle DTOs

Rejected. Runtime owns the only semantic fold. Future projections must reuse it rather than retain
another reducer or Store port.

## 16. Acceptance criteria

Implementation is accepted only when:

1. Program is the sole public checked ingress and persists exact root success/failure contracts plus
   one exact input, output, mandatory failure, optional forward-index success/failure successors,
   and complete Read association for every State; declaration zero is the non-empty root and no
   public address or second wire DTO remains; ProgramError is exactly
   Canonical/InvalidContract/Capacity.
2. Every terminal and successor edge validates against the complete corresponding result contract;
   the EVM-child-to-Portfolio failure mapper remains reachable in hot and cold reduction.
3. Pure and Read Never registration works, Never's exact `mfm.kernel/never@1` semantic/schema/
   contract identities match goldens, it is usable only as a failure contract with no successor,
   and FailureValue is absent.
4. RuntimeAssembly is the only concrete value/State/capability registry; its public contribution
   surface is register_value/register_pure/register_read/register_adapter, State behavior uses the
   one mfm-program PureState/ReadState traits, and no public implementation holder, composite route
   key, or supported erase/downcast path exists.
5. Exactly one private object-safe State-driver boundary and one qualified canonical value path
   exist; HotValue, preparation/call tokens, and a second integrity projection are absent.
6. Runtime owns the only semantic reducer used by execution and read, and hot Inserted frames use
   local fold_next rather than complete reload/refold.
7. Store is object-safe and exposes only load_run and append_run; mfm-journal owns the public
   opaque/invariant-checked StoredRunBytes transfer type with its checked constructor and no raw
   getter, concrete PostgreSQL open/reconnect performs mandatory readiness checks before admitting a
   connection, and Store depends on no Program, State, capability, domain, fact, configuration,
   portability, or Runtime-selection type.
8. ProgramDocumentV2 order, RunFrameV1, frame-local closure, run-head recurrence, the 8 MiB object
   ceiling, 65,536-byte envelope allowance, 25,231,360-byte frame maximum, and every fixed run limit
   match goldens.
9. RunFrameV1 has only admission, Pure-conclusion, and fused Read-conclusion records; entry-point,
   occurrence, preparation, attempt, replacement, and preparation-sequence fields are absent.
10. Every frame embeds exactly the canonical objects directly named by its record; first-reference
    state, a separate record/command digest, object-count limits, and object reservations are absent.
11. The immutable natural target row plus exact frame bytes provides historical idempotency;
    AppendResult is only Inserted or NotInserted, and request/command/receipt/result-position metadata
    is absent.
12. PostgreSQL and Memory have the same absent-head, exact-retry, competing-head, actual-capacity,
    and atomic append behavior, with no preparation preflight or durable reservation state.
13. PostgreSQL uses one repeatable read-only snapshot per complete load and synchronous-commit,
    advisory-locked append transactions; readiness enforces the frozen durability prerequisites.
14. Append returns Unavailable only when commit is definitely absent and Indeterminate only when
    COMMIT may have happened; load Unavailable returns no partial snapshot and has no mutation
    implication.
15. A Read adapter borrows only exact typed intent while Runtime retains its private qualification
    proof and returns typed Evidence or ReadUnavailable; Runtime binds and ordinarily interprets every evidence variant, then appends one fused
    intent/evidence/outcome conclusion.
16. ReadPreparationError maps to Internal and ReadUnavailable maps to Unavailable; both write
    nothing, one invocation enters a selected Read at most once, and concurrent Read observations
    select one exact-head durable winner while others reload it.
17. Dropping start/resume at every await is safety-neutral; Runtime retains no pending owner, lease,
    permit, completion cell, resolver, cancellation/timeout API, detached task, or semaphore.
18. spawn_blocking work is synchronously pure, contains no block_on, Adapter, Store, or persistence
    authority, and a closure finishing after outer Future drop cannot perform dependent IO.
19. start, resume, and read use the same Runtime-derived RunView, and Succeeded/Failed always match
    the retained Program root contract.
20. App/transports own no Runtime lifecycle or frame interpretation, verify selected EntryPointId
    equals Program.entry_point_id before start, may safely drop a top-level future, and keep any
    concurrency bound outside Runtime semantics.
21. A future start transport accepts and echoes explicit RunId before execution; exact admission
    retry reuses that RunId and identical inputs.
22. mfm-replay, portable bundles, export, offline inspection, and every replacement semantic ingress
    are deleted.
23. MfmConfig, configuration publication/load, Program configuration association, configuration
   wire, and configuration SQL are deleted; EvmConfig is retired as submission-only, every former
   PortfolioConfig field is classified as Program, typed C0, or process-local binding, and cold
   execution needs no authoring Config value; plan_snapshot returns only Program plus typed C0 and
   no dummy closure/catalog validation path remains.
24. mfm-facts and every fact selection/publication/proof/storage concept are deleted.
25. tenant, scope, epoch, identity rotation, and database incarnation are absent from APIs, wire,
    hashes, SQL, App, transports, and EVM.
26. AppendRequestId, command/receipt rows, reservations, global objects/membership, and global
    configuration head/sequence are absent.
27. The retired EVM submission entry point and every nonce/broadcast/status domain, adapter, App,
    storage, schema, fixture, and dependency surface are absent; Portfolio and its EVM Reads remain,
    and generic Effect/preparation semantics are deleted.
28. Supported persistence uses one no-loss monotonic timeline; writable rewind, data-loss failover,
    old-run resumption on a fresh authority, and writable historical clones are rejected.
29. Secret-bearing types remain outside persistable values, known secret markers fail before
    Journal construction, and Journal does not claim arbitrary-string secret detection.
30. docs/design.md, docs/architecture.md, transport/operator documentation, crate READMEs, schemas,
    and fixtures describe only this current design.
31. Production LOC, exported types, dependency edges, and files required to add a State decrease.
32. Scope-selected checks and final composed CI pass under docs/build-and-verification.md.
33. State/capability implementation-reference v1 preimages and the domain-owned
    `mfm.evm-physical-target@1` descriptor match exact goldens; no generic/live binding wrapper or
    raw caller-asserted binding-ref registration survives.
34. mfm-capabilities exports only the exact ReadCapabilityContract, CapabilityError, and Result
    surface; Program reference-derivation, assembly association, and hot/cold binding failures map
    to ProgramError::InvalidContract, IncompatibleAssembly, Internal, and InvalidHistory in their
    respective contexts, and no old mode/fact/attempt/evidence-marker or capability-owned outcome
    wrapper remains.
