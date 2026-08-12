# RFC: refactor to a single byte-ingress trust boundary

Status: accepted platform target; implementation pending three bounded preflight proofs

Relationship: this RFC supersedes `rfc_single_trust_boundary.md` and is the normative architecture
for the complete MFM platform cutover. Current code and authoritative docs describe the
implementation being replaced where they conflict with this target. Implementation must leave one
current contract and no compatibility paths.

---

## Decision

MFM has one rule for immutable data trust:

> Bytes are protocol-authenticated where applicable, bounded, strictly decoded, canonicalized, and
> semantically validated exactly once when they enter the process trust base. Successful ingress
> returns an opaque immutable value carrying that evidence. Crossing an internal crate or module
> boundary does not erase it, and no downstream layer reconstructs or revalidates the same fact.

This is a data-ingress rule, not an application access-control system. MFM does not authenticate end
users, manage caller credentials, evaluate roles, ACLs, or grants, or keep capability bindings
live-revocable. Exact-head comparison, durability acknowledgement, redaction, secret lifetime,
adapter protocol authenticity, and provider factual trust are different propositions with explicit
owners.

MFM is reachable only behind a trusted embedding/deployment boundary. That owner admits callers,
controls network and process exposure, and selects a tenant-scoped facade. The credential-free REST
service must not be exposed directly to untrusted callers. Public exposure is a separate product
requirement and requires an access-control design outside this RFC.

The run state-machine journal has exactly three record families:

```text
RunAdmitted
StatePrepared
StateConcluded
```

`RunAdmitted` is always the sole genesis and is committed separately. A zero-state Program becomes
terminal by reducing admission. `StatePrepared` is access-only: it represents one exact Read or
Effect attempt and gates the supported live invocation path. Pure states do not create a
preparation. `StateConcluded` atomically records the domain outcome, facts, and outputs of one state
occurrence, plus accepted capability evidence for Access. Terminality is reducer-derived; there is
no `RunClosed` record.

The final visible Program algebra is only `State | Match`. Execution is deterministic, sequential,
and fail-fast:

```text
one admission value C0
  -> State 1(C0) -> C1
  -> State 2(C1) -> C2
  -> ...
  -> nonterminal State N(Cn-1) -> Cn
  -> terminal State T(Cn)
  -> declared root result
```

Every operation has exactly one domain-owned typed admission value, and that value is `C0`.
Domain/application planning constructs `C0`; Runtime and Store only qualify, persist, and transport
it. `C0` is not a list of independently bound roots, and Runtime and Store never assemble, merge,
split, reorder, or interpret its domain fields. A child operation likewise consumes one exact
current value as its singular fragment-entry context; pure expansion substitutes that nominal value
unchanged, and the child returns one successor value to the caller continuation. When a child's
entry contract differs from the caller's current context, an explicit caller-owned State must first
construct the wrapper; expansion performs no runtime value transformation.

Each `Cn` is an ordinary bounded, secret-free, valid-by-representation domain `MfmValue`. It is the
complete successor context chosen by the State implementation, normally retaining admitted input,
prior domain results needed later, current and remaining work, and caller correlation. Runtime,
Store, and Program do not define a generic run-context container, search history for State code, or
interpret how the domain accumulates data. Every successful nonterminal State returns the complete
successor context. A terminal State of any mode may instead return the exact declared root result;
for Read or Effect this is the accepted, interpreted typed output in `StateConcluded::Access`.
Runtime releases no root result until Store has durably committed that conclusion or qualified an
identical recorded conclusion. A zero-state Program means the expanded sequential control form is
empty. It is certifiable only when its declared root result is the exact identity `C0`
contract/value, so reducing `RunAdmitted` returns that same content identity without a callback.
Every nonempty successful path, including a selected Match arm, must end in a terminal State; a
Match-only or empty-arm terminal path is invalid. Any admission-only operation requiring
transformation must contain an explicit Pure terminal State.

For one durable run prefix there is at most one reachable actionable State and one selected
unresolved preparation. Each affine worker owner contains at most one `CommittedCall` or one
`PendingConclusion`, and each directly committed preparation releases at most one call. Competing
workers may race the same sequential occurrence; exact-head compare-and-append and reducer
selection decide which durable successor survives. Read and absorbing recovery attempts may
therefore overlap without creating a parallel Program construct. A declared failure route is
selected immediately and later normal States are not evaluated. `FanOut` is deleted and is not
replaced by
`Collect`, `Gather`, `Join`, lanes, barriers, `state_many`, an ambient output map, or another
parallel or multi-result workflow construct. Parallel execution requires a separate future RFC.

The two execution paths are:

```text
Pure:
  selected occurrence
    -> Runtime invokes StateImplementation.evaluate(input)
         -> ProposedStateOutcome
    -> Store qualifies the proposal into a Pure-scoped PreparedConclusion
    -> Runtime combines it with the inert session continuation
         -> secret-free Pure PendingConclusion
    -> Store commits StateConcluded::Pure
    -> reducer advances

Read / Effect:
  selected occurrence
    -> Runtime invokes StateImplementation.prepare(input)
         -> canonical intent
    -> PreparedExecution { StatePrepared append, inert continuation }
    -> consuming Store commit
    -> direct-new branch only constructs CommittedCall
    -> Runtime invokes StateImplementation.execute(CommittedCall)
         -> state orchestrates bound adapter entry
         -> qualified adapter performs response ingress
         -> state interprets accepted capability evidence
         -> conclusive AccessHandlerResolution
    -> Store qualifies the resolution into an Access-scoped PreparedConclusion
    -> Runtime combines it with the inert session continuation
         -> secret-free Access PendingConclusion
    -> Store commits StateConcluded::Access
    -> reducer advances
```

There is no durable response-observation phase. Once a response has crossed adapter ingress and the
state has interpreted it, Runtime retains one process-local `PendingConclusion` independently of
the initiating request and retries only the conclusion write. Ordinary request cancellation and
graceful drain do not discard that owner. If the process dies or the Runtime owner is explicitly
force-destroyed before `StateConcluded` is durable, the trustworthy response may be lost. Durable
history then contains only `StatePrepared`. This is an accepted availability tradeoff, not evidence
that entry did or did not occur.

The framework does not claim to sandbox arbitrary Rust code:

> `Pure`, `Read<C>`, and `Effect<C>` classify authority supplied through MFM's supported path.
> Application state implementations, adapters, and their dependencies are trusted process code. A
> malicious or incorrect implementation can capture ambient clients, spawn detached work,
> misclassify an Effect as Read, invoke a client outside the committed-call path, or retry
> internally. MFM cannot detect or prevent those violations.

Within that trust model, the simplified design proves:

1. No MFM-mediated live operation begins before its exact `StatePrepared` is durably committed
   under the admitted Store durability profile.
2. Only the consuming direct-new commit branch can construct the matching `CommittedCall`.
3. One directly committed preparation releases at most one framework invocation.
4. A missing conclusion claims neither that external entry occurred nor that it did not occur.
5. Store selects at most one `StateConcluded` for a state occurrence, across all retry attempts.
6. Replay uses only durable records and performs no state callback or live I/O.
7. Reduction selects at most one actionable State. A direct State continuation consumes the exact
   context concluded by its predecessor; a Match arm consumes the exact declared variant payload
   qualified from that concluded closed sum.

No MFM-generated successful output or definite provider result becomes public before the matching
`StateConcluded` is directly durable or found identical in qualified recorded history.

## Material uncertainties

The architecture and product choices are settled. The implementation supports only the two current
entry points, `mfm.portfolio/snapshot@1` and `mfm.evm/submit-transaction@1`; a future product need
must justify a future RFC rather than leave a dormant branch in this cutover.

The settled rulings are:

1. **U1 — session retention:** the caller-owned affine session retains the latest qualified typed
   cumulative context. Dropping it makes the next resume fold the complete bounded prefix. There is
   no global retention, LRU, suffix protocol, checkpoint, structural sharing, compaction, or cold-
   resume SLO.
2. **U2 — Effect attention:** absent. Callers retain run ids. Delete the projection, column, index,
   backend method, DTO/API, CLI/REST/operator surface, bounds, documentation, and tests. There is no
   scheduler or inventory replacement.
3. **U3 — capability evidence:** inventory only capabilities reachable from the two current entry
   points. Each owns one strict, secret-free intent/evidence contract and exact call binding.
   Authenticated JSON-RPC results are provider assertions, not independent chain truth. Raw provider
   material and secrets are discarded. An Effect is `EntryOnce` unless durable absorption for the
   full recovery horizon and post-state-convergent evidence are proven.
4. **U4 — EVM activation:** submission identity is tenant plus wallet nonce domain plus bounded
   non-secret idempotency key. Activation uses a fresh logical Store scope/writer epoch, wallet
   domain, and sender; those identities may use the same PostgreSQL service. There is no sender-
   reuse drain or migration path.
5. **U5 — immutable binding evidence:** retain one content-addressed, secret-free descriptor with
   only the exact State/capability/adapter implementation identities, physical target/route, Effect
   domain, and public signer key-instance identity needed for callback-free replay and exact process
   association. Delete release, generation, promotion, revocation, and currentness lineage.
6. **U6 — binding replacement:** rotate the Store scope or writer epoch. Old nonterminal runs are
   read/replay-only. Do not rebind them or retain old live assemblies.
7. **U7 — configuration history:** retain the existing 16 MiB per-revision ceiling and add fixed
   experimental limits of 1,024 revisions and 64 MiB cumulative canonical bytes per stream. Enforce
   all three identically at every ingress/load/write path. There is no configuration-load SLO.
8. **U8 — typed erasure:** Program owns canonical bytes, exact contract identity, catalog brand, and
   one safely erased owned typed value. Runtime privately and fallibly downcasts under that brand;
   no unchecked/public downcast, unsafe code, public erasure API, or duplicate decode path exists.
   A `Send` future never captures the `!Send + !Sync` keystore; its dedicated owner retains it, and
   the future may carry only owned prepared data and a bounded `Send` command handle.
9. **U9 — capacity:** retain one complete frame per append and the existing outer ceilings: 32 MiB
   per frame; 65,536 frames, 1,048,576 reachable objects, and 512 MiB of canonical frame bytes per
   run. The MVP portfolio admits at most 64 total EVM sources. Reads and proven absorbing Effects
   admit at most three total attempts, including the initial attempt; `EntryOnce` admits exactly
   one. Derive secondary context/object/fact/database limits from the two production contracts
   rather than exposing independent knobs. Before external entry, reserve the complete maximum
   mandatory conclusion, including facts and projections. The EVM preparation durably fixes the
   deterministic transaction identity before broadcast.
10. **U10 — PostgreSQL topology:** promise primary crash/restart durability only, not primary-host
    loss, failover, replica, quorum, or multi-primary survival. Multiple qualified processes may use
    the same Store scope/epoch and are linearized by exact-head CAS; the epoch is not a process
    lock.
11. **U11 — cumulative-context ABI:** freeze complete context tables only for the two current entry
    points. Every successful nonterminal State returns the complete typed successor; Match selects
    one exact complete variant payload; failure carries prior context only in an explicit domain
    failure value; and EVM carries the opaque typed Portfolio continuation unchanged. The handoff is
    consuming and supports non-`Clone` `MfmValue`; no bounded-clone alternative or untyped context
    map exists.

Three narrow implementation proofs remain:

1. **Capability and binding inventory (U3/U5).** The exact reachable evidence variants, absorption
   claims, and immutable descriptor fields have not yet been mapped. Missing one could overstate
   provider truth, duplicate an Effect, or resume under the wrong binding. Check in one table for
   the two current entry points; default every unproven Effect to `EntryOnce` and delete every
   unmapped temporal field.
2. **Typed ABI compile proof (U8/U11).** The safe erased representation, consuming non-`Clone`
   handoff, EVM/Portfolio continuation, affine call/result future, cancellation, and keystore/`Send`
   split have not yet compiled together. Failure changes private Runtime mechanics. Prove this in a
   disposable spike, record the final type/context table, and stop rather than add a fallback if it
   fails.
3. **MVP capacity envelope (U1/U7/U9).** The exact context/conclusion limits derived beneath the
   fixed outer ceilings have not yet been measured. A wrong value may reject one intended maximum
   workflow or fail to reserve a mandatory post-entry conclusion. Record one maximum fixture for
   each current entry point plus exact-bound/bound-plus-one and reservation tests. There is no
   latency acceptance threshold.

No other material uncertainty remains.

## 1. Scope and incompatible cutover

This RFC owns one cutover across typed Program construction, durable State declarations,
process-local State implementations, capability evidence, run journaling and reduction, Runtime
sessions, access execution, facts, projections, replay, PostgreSQL, tenant-scoped application
facades, portable export, EVM identity, and configuration history.

It deliberately excludes:

- production provider topology, key custody, endpoint authentication, and binary assembly;
- a public background scheduler or workflow-specific lifecycle;
- compatibility with current Program, run-journal, portable, or EVM bytes;
- rollback resistance after every independent memory of a later head is lost; and
- sandboxing or static analysis of linked Rust code.

The “exactly three families” rule applies to the append-only run state-machine stream. Configuration
revisions, tenant fact routes, and portable artifacts are separate data families and do not become
run records.

Every run append contains exactly one semantic run record plus its append-atomic object, fact,
publication, and projection closure. There is no adjacent closure record and no multi-record path
that can cross derived terminality inside one batch.

The journal candidate, assigned-record, commit, and recursive `JournalHead` hash algorithms and
their domain separators remain unchanged. Record bodies, logical keys, Program documents, portable
format, and EVM identities change incompatibly. Deployment starts with fresh schema identities,
`StoreScopeId`, writer epoch, portable format, and affected wallet/domain activation. Old tags and
fields are rejected; no decoder, migration, alias, or fallback survives.

## 2. Trust model and ingress

### 2.1 Process trust base

The process TCB includes compiled MFM code and dependencies, state implementations, adapters,
transports, signers, storage implementations, the Rust toolchain, process assembly, and the trusted
embedding/deployment supervisor.

The embedding constructs each application facade with one `TenantScopeId`. Public calls accept
neither credentials nor a tenant override. A process serving several tenants exposes separately
constructed facades. An out-of-partition run id is `RunNotFound`; bytes returned inside one tenant
partition whose `RunAdmitted` tenant disagrees are invalid history, not hidden as absence.

Program catalogs, Runtime assemblies, State implementations, qualified adapters, bindings, and
their concrete dependencies are immutable for one composed-process lifetime. Replacement means
stop intake; drain every pending conclusion to a terminal Store disposition; drain or
conservatively park every prepared execution, committed call, and live provider call; then drop the
assembly and construct a new one. This is an operational drain discipline, not a global
single-writer-process safety mechanism: independently composed workers may overlap and their Store
commands still linearize through the same exact-head protocol.
Explicitly forcing destruction of a `PendingConclusion` accepts the documented response-loss window
and leaves only the conservative durable state already present.

Untrusted executable plugins are outside this model and require a separate process, WASM sandbox,
or another enforceable isolation boundary.

### 2.2 Byte ingress

Ingress sites include caller and command input, configuration sources and rows, Program documents,
complete retained prefixes and found append attempts, RPC/provider responses, imported portable
closures, and database projections.

Each ingress owner performs only the checks relevant to its protocol:

```text
size and work bounds
  -> framing and strict decode
  -> canonical float-free representation
  -> intrinsic value invariants and content identity
  -> protocol authentication and request/call binding, where applicable
  -> opaque domain value
```

For provider material, the adapter also binds the exact committed call, canonical intent,
capability, provider/target/signer, protocol version, and response correlation before producing
`AcceptedEvidence<C>`. Raw provider bytes and diagnostics never reach state interpretation or a
persisted surface. Valid ingress proves only the capability's declared evidence proposition, not
factual truth beyond its TCB.

Contextual checks remain at their owners:

| Proposition | Owner |
| --- | --- |
| fixed facade tenant | application/Store lookup |
| latest head at one snapshot | backend load |
| exact predecessor and writer epoch | append transaction |
| current tenant fact publication head | conclusion transaction when facts publish |
| preparation durability | qualified backend before direct-new call construction |
| selected unresolved preparation | Store conclusion reducer |
| adapter nonce/key/transaction semantics | qualified adapter |
| public redaction and secret exclusion | DTO/render boundary |

### 2.3 PostgreSQL authority and non-guarantees

Within one admitted Store identity and writer epoch, MFM trusts PostgreSQL and its sealed writer
path to report outcomes truthfully, preserve immutable committed bytes atomically, enforce exact-
head compare-and-append and dense fact publication, and retain commits across the admitted
durability profile. The epoch qualifies persisted lineage and append preconditions; it does not
grant mutually exclusive process liveness. Qualified workers may race against the same Store
scope/epoch, and only exact-head/direct-new results create successor or invocation authority.

`NewlyCommitted` means the exact transaction crossed that durability point. The selected profile
survives a crash/restart of the admitted primary; Store qualification rejects weaker effective
settings. A stronger host-loss or failover claim is outside this design and requires a future RFC.
A weakening setting or writer-epoch change invalidates the Store rather than changing the meaning
of direct-new.

A normal process restart may retain the epoch and resume from qualified durable history. A
legitimate database restore or binding replacement rotates Store identity or epoch; old
nonterminal runs become read/replay-only. After every independent later-head anchor is lost, MFM
cannot distinguish a self-consistent rollback from a Store that never advanced. An actively lying
database or administrator is outside this threat model, and a post-commit readback against the same
authority does not strengthen it.

## 3. Program, State, implementation, and assembly

### 3.1 One Program authority

There is exactly one executable semantic representation:

```text
ProgramDocument   bounded, strict, serializable data; never execution authority
Program           opaque, immutable, process-local callback-free authority
```

Typed DSL construction and hostile `ProgramDocument` ingress converge on one normalized sequential
control-form invariant owner. Hot construction does not serialize and reparse its result. Cold
ingress performs byte-specific checks once. A `ProgramRef` is an address, not authority, and exact
content equality cannot substitute a value from another in-process catalog instance.

The final visible declaration algebra is exactly:

```text
Declaration = State | Match
```

Child operations and reusable fragments remain authoring abstractions, but pure expansion removes
their boundaries before `Program` exists. Configuration specialization, capability lowering,
policy/failure wrapping, child substitution, and normalization likewise finish before Program
construction. No authored or expanded `FanOut`, lane, join, barrier, or multi-result declaration
survives. Resume consumes the persisted final sequential control form and never reruns expansion
under current code or configuration.

Admission cardinality is singular by representation across authored, expanded, and certified
Program forms, entry-point contracts, admission commands, and `RunAdmitted`. Every operation
declares exactly one nominal `C0` contract and receives exactly one typed `Value<C0>` constructed
by domain/application planning. There is no repeated root-declaration API, ordered root list,
`initial_bindings` collection, or ordered child-input bijection. A child authoring call consumes
one exact current `Value<Cn>` as its singular fragment-entry value; expansion substitutes it
unchanged and returns one successor value. `C0` names the admitted root value, not every nested
fragment entry. A different child entry contract requires an explicit State-produced wrapper.
Zero-state Programs still admit exactly one `C0` and may terminate only by selecting that exact
value as the root identity result.

The final control form uses one minimal callback-free `SequentialControlAddress` derived from the
declaration ordinal plus enclosing Match arm tags. It addresses State occurrences and Match
selectors only; it has no lane, fragment, lexical-slot, producer, origin, or generic structural-
path variant. Program certification, reducer, journal occurrence derivation, and replay use that one
strict address and reject every old structural-path identity.

Certification validates declaration order, exact nominal context-contract continuity, Match
exhaustiveness and arm convergence, failure routing, all finite bounds, implementation closure, and
one total root result on every successful terminal path, independent of whether the terminal State
is Pure, Read, or Effect. `Match` is deterministic structured choice over a closed-sum complete
context. The reducer selects and qualifies exactly one declared variant payload; that payload is
itself the complete cumulative context and exact content identity received by the arm's first
State. Only that arm advances, and every continuing arm must produce the exact cumulative-context
contract required by the shared continuation. No State receives both an outer selector value and
an unrelated projected payload, and Match is not a multi-result merge.

The admission-derived zero-state exception exists only when the expanded control form contains no
State or Match declaration and the root result is exact `C0` identity. Every nonempty successful
path must reach a terminal State. Certification rejects a Match-only path or a selected empty arm
that would otherwise attempt to become terminal without a conclusion.

`ProgramCatalog` owns callback-free declarations, codecs, pure expansion, sequential control-form
construction, bounds, and hostile ingress. `RuntimeAssembly` is constructed afterward from that
exact catalog instance and owns the immutable process-local implementation registry and private
Runtime brand. Store and Replay receive only callback-free Program/catalog authority.

### 3.2 Durable State and capability contracts

A durable State declaration remains Program data. It owns:

- input, output, failure, fact, and maximum-conclusion contracts;
- exactly one `Pure`, `Read<C>`, or `Effect<C>` classification;
- immutable State implementation identity and execution-binding identity;
- for access, the capability intent/evidence ABI, effect domain, entry discipline, fact mode, and
  recovery budget; and
- callback-free rules needed to verify replacement and absorption from durable history.

For every successful nonterminal occurrence, `S::Output` is the complete domain-owned successor
context. A cumulative context is a bounded, secret-free `MfmValue`, valid by representation. Its
nominal type may contain admitted domain input, prior interpreted domain outputs needed later,
current stage, completed results, remaining work, and a caller continuation/correlation. The exact
shape belongs to the operation or reusable domain fragment, not the kernel. A terminal State of
any mode may instead return the exact declared root result. For Access, that result remains a typed
State output and becomes public only through the durable, qualified `StateConcluded::Access` path.

The predecessor-to-successor contract is exact: across a direct State edge, State N receives the
typed value and content identity concluded by State N-1. When a declared `Match` intervenes, its
callback-free reducer projection qualifies the selected closed-sum variant payload, and that exact
payload contract/content identity becomes the arm's complete input. Every State returns the whole
typed value required by its continuation. The State implementation decides what to retain and how
to interpret domain data. Runtime, Store, and Program neither merge results nor synthesize a generic
context.

Conceptually:

```rust
trait State {
    type Input: MfmValue;
    type Output: MfmValue;
    type Failure: FailureValue;
    type Execution: Execution; // Pure | Read<C> | Effect<C>
    type Lowering: StateLowering;

    fn maximum_conclusion() -> Result<ConclusionCapacityBound, ProgramBuildError>;
}

trait AccessCapabilityContract {
    type Mode: AccessMode; // ReadMode | EffectMode<EntryOnce | EntryAbsorbing<N>>
    type Intent: MfmValue;
    type Evidence: AccessEvidenceValue;
    type Facts: FactSelectionMode;

    fn total_attempt_bound() -> NonZeroU16;
    fn prior_fact_selection(
        intent: &Self::Intent,
    ) -> Result<Option<FactSelectionRequest>, FactSelectionContractError>;
    fn absorption_identity(
        intent: &Self::Intent,
    ) -> Result<Option<AbsorptionIdentity>, CapabilityContractError>;
    fn bind_evidence(
        intent: &Self::Intent,
        evidence: &Self::Evidence,
    ) -> Result<(), EvidenceBindingError>;
}
```

Intent and evidence are projected from the durable capability contract, not chosen freely by a
live implementation. This lets Store reserve bounds, qualify cold history, and reduce replay
without invoking state code.

One capability type has exactly one sealed Access mode. `Read<C>` requires `C::Mode = ReadMode`;
`Effect<C>` requires `EffectMode<_>`. A protocol used under both semantics has two nominal
capability types sharing private codec/transport helpers, never one dual-mode type.

`AccessEvidenceValue` is a kernel-owned sealed blanket marker over valid-by-representation
`MfmValue`; downstream crates never implement that marker manually.
Domain crates define an ordinary finite enum with bounded members, checked decoding, no unknown/raw
provider escape variant, and the normal value derive. Program catalog registration qualifies its
exact contract/codec/type association; only adapter ingress can turn a qualified `C::Evidence` into
call-correlated `AcceptedEvidence<C>`.

For an absorbing Effect, the capability additionally defines a callback-free absorption-identity
projection. It may be the canonical intent's content identity or a capability-owned subvalue; the
kernel does not require a second universal `StableOperationKey` field.

`total_attempt_bound` is one for `EntryOnce` and at most three for a Read or proven absorbing
Effect, including the initial attempt. Catalog finalization rejects zero or larger bounds.

Provider-affecting prior facts must already be explicit in `S::Input`. Program expansion models
their acquisition as an earlier Read State whose complete successor context contains the reviewed
fact interpretation consumed by this State. The post-commit fact selection projected from intent
may support interpretation only and cannot change intent, target, request bytes, signer, binding,
or effect domain.

### 3.3 Process-local State implementation

`StateImplementation<S>` is the one process-local live concept. It is opaque and associated once
with the durable State/implementation/binding identities during assembly. Runtime may erase it
privately; public associated handler or future types are unnecessary.

Pure and access implementations have different contracts:

```text
Pure:
  evaluate(&S::Input) -> ProposedStateOutcome<S::Output, S::Failure>

Read<C> / Effect<C>:
  prepare(&S::Input) -> Result<C::Intent, PreparationError>
  execute(CommittedCall<S, C>) -> Future<Output = AccessHandlerResolution<S, C>>
```

These signatures describe responsibilities; private wrappers provide the settled consuming input-
owner handoff through Pure evaluation, Access accepted/unresolved handling, and explicit failure.
`MfmValue` does not imply `Clone`, and the implementation has no bounded-clone alternative.

`AccessHandlerResolution<S, C>` is a public opaque, private-field value because it appears in the
constructor closure signature; it is not a public enum and has no free constructor. Only consuming,
call-bound methods on `AcceptedOutcomeAccess`, `AcceptedIntegrityAccess`, or `UnresolvedAccess` can
create it. Public `AccessResolution` is matchable, but those payloads have private fields and no
constructors; only bound adapter ingress supplies them. Runtime erases the closure/future privately
into its exhaustive `HandlerResolution`, so no associated handler or future type appears on
`StateImplementation`.

`prepare` is deterministic, secret-free, and performs no ambient I/O. It constructs the exact
canonical intent that `StatePrepared` fixes. There is no unrelated process-only preparation whose
behavior can diverge from the durable intent.

Pure `evaluate` is likewise deterministic and secret-free and derives its result solely from the
exact `S::Input`. Its supported path exposes no ambient I/O, clock, randomness, history, or reader.
Captured ambient access remains a trusted-code violation under the stated TCB, not a supported
dependency.

`PreparationError` is a bounded, redacted drive error, not a State outcome. It appends no record,
constructs no `PreparedExecution` or `CommittedCall`, and returns the still-ready active session to
the caller. Runtime does not spin or automatically retry it; a later explicit drive may try the
same ready occurrence again under the same admitted Program.

`execute` owns post-commit orchestration, consumes the call's bound-adapter entry method, interprets
accepted typed evidence, and proposes the state outcome and facts/outputs. A process-local
implementation may physically capture the same `Arc` used by assembly, and State and adapter may be
the same concrete object, but no captured handle is an argument to entry. The exact qualified
adapter and binding witness are sealed into `CommittedCall`. This does not relax immutable binding,
committed-call consumption, adapter ingress, secret handling, or entry-classification obligations.

A State implementation's supported constructor and callback arguments expose only its exact
`S::Input` and the capability authority sealed into `CommittedCall`; they expose no `RunHistory`,
`QualifiedRun`, Store, reducer, journal reader, arbitrary output map, or Runtime history callback.
Domain crate dependency rules keep those authorities out of ordinary State code. The cumulative
context may contain reviewed secret-free interpretations of prior State results, but never
run-record envelopes, append coordinates, Store/reducer authority, raw evidence from an unrelated
State, provider diagnostics, secrets, credentials, or values outside the declared input contract.
Runtime does not search history on a State's behalf. As elsewhere in this RFC, malicious trusted
Rust that captures an ambient handle is a TCB violation, not something the type signature sandboxes.

A deliberate recovery State receives prior cumulative context only when the domain's declared
failure route carries that context explicitly. Default fail-fast propagation carries only the
typed failure and prevents every later normal State from evaluation, preparation, and invocation.

The load-bearing rule is:

> MFM's supported provider-entering method consumes the exact `CommittedCall`; a prepared operation
> is not callable before its exact durable preparation directly commits.

### 3.4 Assembly and adapter ownership

Assembly owns pools, transports, signers, keystores, actors, secret ingress, endpoint and target
qualification, the immutable descriptor-to-implementation association, readiness, drain, shutdown,
and replacement.

A qualified adapter owns:

- concrete clients and secret-bearing internals;
- translation from canonical intent to the external operation;
- exact provider, target, signer, binding, and effect-domain association;
- bounded decoding, protocol authentication, and request/call correlation;
- stable-key transmission and external atomicity behavior;
- SQL transaction, nonce, and permanent operation-key rules where applicable;
- capability-specific definite-pre-entry versus possible-entry classification; and
- destruction or redaction of raw provider diagnostics.

The supported adapter entry consumes a call-scoped token from `CommittedCall` and derives provider
request bytes from the committed canonical intent. It cannot accept handler-substituted request
bytes. A clone of the adapter is never invocation authority.

## 4. Capability evidence and handler resolution

`C::Evidence` is a bounded, canonical, strict, secret-free capability-owned sum. It represents the
definite results the state is allowed to interpret, including as applicable:

- returned provider success;
- returned provider rejection;
- accepted safe failure;
- capability-specific definite-pre-entry evidence; and
- accepted integrity-blocking evidence that grants no retry authority.

Capability construction preserves uninhabited no-failure cases and any success-only or other
disposition rule that makes illegal state outcomes unrepresentable. The old split settlement API
must not be replaced by a broad freely constructible evidence/outcome pair.

Adapter ingress alone constructs call-correlated `AcceptedEvidence<C>`. A generic operational error
enum can never claim definite pre-entry for an `EntryOnce` Effect. Malformed, unauthenticated,
unbound, or ambiguously correlated provider material produces no accepted evidence.

These hot and cold authorities are distinct. `AcceptedEvidence<C>` is a one-use, call-correlated
adapter-ingress product. The journal stores canonical `C::Evidence`. Callback-free retained-record
ingress may construct only `QualifiedRecordedEvidence<C>` bound to the referenced preparation; it
can never recreate hot accepted evidence or invocation authority.

Execution returns a closed result, not `Result<Conclusion, HandlerError>`:

```text
HandlerResolution =
    Concluded {
        accepted capability evidence,
        state outcome,
        facts and output objects,
    }
  | Unresolved {
        call-bound prepared successor,
        redacted operational classification,
    }
```

`Concluded` is constructed only through the exact call-correlation owner. The
private `Unresolved` wrapper retains the exact active successor and call correlation but no invoker
or external-I/O method; its classification alone is not an owner and cannot be supplied by state
code. It is process-local, is never persisted, and authorizes no retry by itself. Provider success,
returned rejection, accepted safe failure, and accepted integrity blocking are definite and must
enter the synchronous conclusion handoff. Cancellation, caught panic, malformed or unbound
response, transport ambiguity, possible entry, or loss of a trustworthy result before that handoff
leaves only `StatePrepared`. After handoff, cancellation and caught panic retain the
`PendingConclusion`; only process/explicit owner loss may discard it.

Facts remain success-owned unless a domain contract deliberately changes that rule. A typed state
failure publishes no facts. Integrity blocking publishes no domain facts or outputs.

## 5. The three-family run journal

### 5.1 Record algebra and logical keys

Conceptually:

```text
RunRecord =
    RunAdmitted(RunAdmitted)
  | StatePrepared(StatePrepared)
  | StateConcluded(StateConcluded)

RecordLogicalKey =
    Admission(run)
  | StatePreparation(occurrence, preparation ordinal)
  | StateConclusion(occurrence)
```

`RunAdmitted` fixes the run id and tenant, final Program, exactly one admitted domain value as the
exact initial cumulative context `C0`, configuration, source dependencies, Store/catalog identities,
and every admission bound. It contains no root list or binding collection. Domain/application
planning constructs `C0`; Runtime and Store preserve its exact nominal contract and content
identity without interpreting its fields. `RunAdmitted` is always the first and only genesis
record. Admission is never batched with the first State.

`StatePrepared` represents one exact logical Read or Effect attempt:

```text
StatePrepared {
    occurrence,
    canonical_intent,
    execution_binding,
    replaces: optional selected preparation reference,
    reserved_conclusion_capacity,
    preparation-time fact selection fixation when applicable,
}
```

The exact occurrence's Program declaration and selected predecessor supply the immutable typed
cumulative input and its content identity, access mode, capability, entry discipline, evidence
contract, effect domain, and attempt budget. Store supplies coordinates, ordinal, append identity,
and the assigned typed `StatePreparationRef`; state code constructs none of them.

For the EVM broadcast capability, the canonical intent fixes the deterministic transaction identity
before `StatePrepared` becomes durable and before provider entry. Secret signed bytes remain outside
the journal.

`StateConcluded` is a strict sum:

```text
Pure {
    occurrence,
    outcome,
    facts_and_outputs,
}

Access {
    occurrence,
    preparation_ref,
    accepted_fact_selection when applicable,
    accepted_capability_evidence,
    outcome,
    facts_and_outputs,
}
```

For accepted integrity blocking, the closed capability evidence contains its bounded stable
redacted code. The Program/capability's fixed callback-free disposition derives the typed failure
`outcome` and an empty `facts_and_outputs` closure before this uniform Access conclusion is
qualified. It grants no retry authority, invokes no additional State interpretation/routing
callback, and cannot carry cumulative context into recovery; cold reduction invokes no callback at
all. A capability needing domain interpretation or context-carrying recovery uses an ordinary
accepted evidence variant handled by State code instead.

`accepted_fact_selection` is the callback-free selected response plus completeness attestation
bound to the exact preparation request and frontier. It is absent for capabilities without prior
facts and is distinct from the conclusion-time fact-publication coordinate.

Its logical key is the state occurrence, never the preparation. This is what enforces at most one
conclusion across retry attempts. Fact objects and publication routes share the conclusion append.
Every successful nonterminal `StateConcluded` retains the complete successor cumulative context in
its output closure. It is not a delta, lane result, generic output-map entry, or implicit reference
to earlier history. A successful terminal `StateConcluded` instead retains the exact declared root
result, regardless of whether its variant is Pure or Access.

Keep these identities distinct:

1. physical `AppendRequestId`, which identifies one exact predecessor-bound append attempt;
2. assigned `StatePreparationRef`, which identifies one journal access attempt; and
3. the capability-defined external absorption identity, which makes re-entry converge.

### 5.2 Reducer transitions and selection

The callback-free reducer admits only:

```text
Unadmitted
  -> Admitted / terminal-zero-state

Ready(occurrence)
  -> Concluded(Pure)
  -> Prepared(p0 selected)                    // Read or Effect

Prepared(pi selected)
  -> Concluded(pi)
  -> Prepared(pi+1 selected, pi superseded)   // Read or EntryAbsorbing only

Concluded(occurrence)
  -> exactly one next State/Match cursor step or terminal result
```

The reducer owns one deterministic cursor through the sequential `State | Match` control form. At
every valid prefix it selects zero or one actionable occurrence. `Match` reads its exact selector
from the current cumulative context, selects one arm deterministically, and requires that every
continuing arm return the same nominal context contract. A typed failure follows its fixed failure
route immediately; later normal declarations are unreachable and cannot be evaluated or prepared.

For a direct State continuation, the next occurrence input must be the exact complete output and
content identity of the selected predecessor conclusion. For Match, the reducer binds the parent
closed-sum context, deterministic `SequentialControlAddress`, declared arm tag/payload selector,
and exact qualified payload contract/content identity; that payload is the arm's complete input.
Qualification rejects a missing, truncated, reordered, foreign, wrong-contract, or extra context
value. There are no lane walkers, nested lane cursors, active-action collections, completion
barriers, join synthesis, or completion-order reconciliation.

A replacement names exactly the selected unresolved parent and increments the Store-assigned
ordinal without gaps. Branches, cycles, forward references, returning to an older selection, and
replacement after conclusion are invalid.

Read replacement preserves the exact occurrence, immutable cumulative input and content identity,
capability, canonical intent, execution binding, and effect domain. Every replacement consumes the
Program-declared total-attempt budget including the initial attempt. A capability-owned freshness
coordinate, such as a later fact frontier, may change only when it is explicitly separate from
provider semantics. If
selected facts can change the provider operation, the selection must already be fixed in State
input or be mechanically projected from Program/input before `prepare`; it becomes part of the
canonical intent and `StatePrepared`. The post-commit fact continuation is interpretation-only.

`EntryOnce` forbids replacement. `EntryAbsorbing` replacement additionally preserves the exact
absorption identity and remains within the total-entry budget, including the initial attempt.

Conclusion classification precedence is mode-aware:

1. a Pure candidate with the same occurrence, outcome, and canonical object/fact closure, or an
   Access candidate with the same occurrence, preparation ref, optional accepted fact selection,
   capability evidence, outcome, and canonical closure is `AlreadyConcludedSame`;
2. an Access candidate naming a preparation provably superseded in that occurrence is
   `NoLongerSelected`;
3. an already-durable different conclusion for the Pure occurrence or the same selected Access
   preparation is `Conflict`;
4. a foreign, absent, cross-run, or cross-occurrence preparation reference is invalid correlation
   or invalid history. A Pure occurrence lacking a durable conclusion must remain ready; any
   retained history where it does not is invalid rather than a semantic disposition.

Byte-equal outcomes from different preparations are not duplicate conclusions.

### 5.3 Terminality and capacity

Terminality is derived after every semantic record. A root cannot become terminal while any
reachable occurrence has a selected unresolved preparation. Once terminal, every further
`StatePrepared` or `StateConcluded` is illegal; cold ingress rejects a trailing record. A database
`closed` field may remain only as an append-atomic checked projection.

A zero-state admission has an empty expanded control form, uses its exact admitted `C0` contract/
content identity as the root result, and updates the terminal projection in the same transaction as
`RunAdmitted`; it creates no second root object or callback-derived projection. Certification
rejects a different root contract/value, any Match-only control form, or a selected empty Match arm
without a terminal State.

For a nonzero run, terminality is mode-neutral: the reducer derives it only after the selected
terminal State concludes with the exact declared root-result contract and no reachable
continuation. A Pure root uses `StateConcluded::Pure`; a Read or Effect root uses
`StateConcluded::Access`. A context-shaped value is legal as a root when it is the declared root
contract; an output is rejected only when its contract is wrong or a continuation remains.

The sole selected unresolved preparation reserves one complete maximum legal conclusion against run
bytes, object counts and bytes, evidence/outcome variants, facts, dense publication rows, indexes,
accepted prior-fact response/attestation when applicable, projections, batch totals, and database
parameter/frame limits. Replacement atomically transfers the selected predecessor's logical
reservation; conclusion consumes it; a superseded attempt has no independent legal-conclusion
reserve. No conclusion may exceed its reservation.

Capacity also bounds `C0`, every complete `Cn`, State occurrences, sources/collections, total
canonical run bytes, and cold-fold work. Retaining `C1, C2, ... Cn` may cost `O(n^2)` canonical
bytes when each context contains accumulated prior data. This is accepted for current workloads.
The RFC adds no structural sharing, content-reference accumulator, context checkpoint, compaction,
or collection optimization.

### 5.4 One event, reducer, binder, and prefix commitment

Locally typed proposals and qualified retained records converge on one internal semantic event:

```text
typed proposal ---------------------------> ResolvedEvent
hostile persisted record -> qualification -> ResolvedEvent

apply(previous, ResolvedEvent)
  -> PendingAppend { record draft, unbound successor, obligations }

bind local AppendContext
  -> PreparedAppend { exact frame, successor, indexes, projections }

bind retained frame fixation
  -> QualifiedRun
```

There is no intent-versus-recorded reducer, local serialization/requalification, successor
comparison typestate, or second semantic implementation. Cold ingress still checks the complete
retained envelope and absence of surplus records or objects once.

The recursive journal head continues to commit the exact predecessor, Store identity/epoch,
append request, candidate digest, assigned record, and object closure. A deserialized head alone is
not semantic evidence; only opaque `QualifiedRun` establishes a qualified prefix.

## 6. Execution authority and Store coordination

### 6.1 Admission coordination

Admission consumes one owner-bound typed Program/`C0`/configuration/source product and appends
`RunAdmitted` alone against an absent predecessor. Concurrent spawn or resume workers may race.
Direct-new returns the exact admitted session or reducer-derived zero-state terminal result. Raw
`Found` is reserved for lookup of the same physical append id: found-identical promotes no local
successor and may resume only from qualified recorded history; found-different conflicts. A
distinct-id genesis loser receives raw `StaleHead`; Store qualifies the existing `RunAdmitted` and
compares the semantic admission candidate without physical coordinates. Exact-same returns the
recorded active/terminal history, different conflicts, and malformed/invalid history fails
integrity qualification. No local mutex or process lease substitutes for append-id idempotency and
exact-head comparison.

An unknown acknowledgement retains the exact admission append and owner. Resolution
serializes on the same physical identity: found-identical qualifies recorded history; proven
absence with the genesis precondition still current may resubmit, and only that resubmission's
direct-new branch advances. If another physical id commits genesis before that resubmission, raw
`StaleHead` consumes the suspended owner through the same semantic admission comparison: exact-same
returns qualified recorded history, different conflicts, and invalid history fails integrity
qualification. It does not resubmit again. Repeated unknown retains the quarantine. No admission
branch invokes State, adapter, or provider code. This is why admission stays separate from the
first State.

### 6.2 Prepared execution and direct-new authority

For an access occurrence, Runtime combines the Store-prepared append with an inert implementation
continuation and its affine session shell:

```text
PreparedExecution {
    exact StatePrepared append,
    inert implementation continuation,
    Runtime session continuation,
}
```

It has no execute or adapter-entry method. The consuming coordinator calls the owner-bound Store
commit. The backend's raw `NewlyCommitted` value never escapes as transferable proof. Only the
same coordinator frame's direct-new branch combines Store's exact committed-preparation token with
the still-owned continuation to construct `CommittedCall`.

`CommittedCall` is private-field, non-`Clone`, non-Serde, and binds at least:

- exact catalog instance, Runtime assembly brand, Store identity/epoch, durability profile, and
  exact committed Store head;
- tenant, run, Program, occurrence identity/`SequentialControlAddress`, exact immutable cumulative
  input plus its nominal contract and content identity, access mode, and capability;
- canonical intent, State implementation, qualified adapter, execution binding, provider/target/
  signer identities, and effect domain;
- assigned preparation ref, ordinal, committed head, and replacement relation;
- one process-local call-correlation identity and Store-minted conclusion correlation; and
- when declared, the exact one-use prior-run fact continuation.

Within that same affine worker owner there is no independently usable `RunSession` beside a live
`CommittedCall` or its pending conclusion. The active committed successor moves through the call-
correlation owner and returns to that worker's session only after unresolved execution or
conclusion disposition. Other workers may retain independently qualified, potentially stale
sessions with no authority derived from this call.

The official adapter entry consumes the call token once. Private fields and non-cloneability are
supplemented by process-local per-call correlation carried through adapter ingress, so a shared
same-capability adapter cannot substitute a completion retained from another call.
The consuming entry method accepts no caller-supplied adapter, binding, request, target, signer, or
provider argument: it moves the exact qualified adapter and private binding witness already sealed
into `CommittedCall` and derives request bytes from the committed intent.

### 6.3 Preparation commit outcomes and ambiguity

The mechanical backend reports only:

```text
NewlyCommitted | Found(raw attempt bytes) | StaleHead | AcknowledgementUnknown
```

Store qualifies and binds that result to the exact preparation. The semantic branches are:

| Branch | Invocation authority |
| --- | --- |
| direct-new | exactly one matching `CommittedCall` |
| found exact same / `ExistingSame` | none; resume only from qualified history |
| found different, invalid, or over capacity | none; fail closed |
| stale head, terminal run, another preparation selected, or conflict | none |
| retryable failure known to precede commit | retain exact inert `PreparedExecution`; none yet |
| acknowledgement unknown | quarantine exact inert `PreparedExecution`; none yet |

Acknowledgement resolution uses one serializing same-identity operation. A snapshot miss alone is
not proof of absence while the original transaction may still commit.

- found same destroys the pending implementation continuation and invokes zero times;
- found different or invalid fails closed;
- proven absent while the exact reducer precondition remains current--either an unprepared first
  attempt or the same selected replacement parent--may retry or rebind the same
  `PreparedExecution`;
- retry the same physical append identity only when the entire predecessor/fact-precondition/
  candidate command is unchanged; any rebind changing predecessor, fact fixation/precondition, or
  canonical frame first proves absence or definite precommit failure, retains the semantic owner,
  and mints a fresh physical append id;
- only that resubmission's Store-qualified direct-new
  `DirectlyCommitted(CommittedPreparation)` semantic branch may construct `CommittedCall`;
- a later selected preparation, terminality, or stale semantic action invokes zero times and
  returns its typed qualified-history/rebind disposition;
- repeated ambiguity retains the same quarantined owner; only explicit owner destruction or
  process loss drops it; and
- cold history never reconstructs the lost implementation continuation.

### 6.4 Accepted execution and `PendingConclusion`

The call-correlation chain is:

```text
CommittedCall
  -> qualified adapter call
  -> AcceptedEvidence<C>
  -> state interpretation
  -> Store-qualified PreparedConclusion
  -> Runtime-owned PendingConclusion
```

State code supplies only typed evidence interpretation and state/fact/output proposals. Store
constructs the callback-free canonical `PreparedConclusion`: record proposal,
preparation/occurrence correlation, the original cumulative input's nominal contract/content
identity, object closure, capacity discharge, and physical-append owner.
Runtime combines it with the inert session continuation into affine `PendingConclusion`; there is
no independently usable session beside it within that same affine worker owner. Other workers may
hold separately qualified, potentially stale sessions, but cannot derive authority from this
pending owner. Store never owns Runtime brand/dispatch state or hot accepted evidence.
`PendingConclusion` is secret-free, non-Serde, and contains no adapter, raw response, invoker,
implementation callback, scanner, or means to perform provider/fact/state-callback I/O. Runtime's
coordinator may use its inner owner only for the required Store conclusion-commit and
acknowledgement-recovery I/O.

The coordinate-free canonical conclusion excludes predecessor, sequence, record hash, append id,
publication coordinate, and physical new-object deltas. A physical append binds that same semantic
conclusion to one exact predecessor and fact coordinate. Rebinding therefore uses a new physical
append identity only after absence of the preceding identity has been established.

Runtime's consuming coordinator exposes this generic semantic result after moving the inner
`PreparedConclusion` through Store:

```text
Committed(active direct successor)
AlreadyConcludedSame(qualified recorded history)
NoLongerSelected(qualified latest history)  // Access only
Conflict(qualified latest history, typed conflict)
AcknowledgementUnknown(PendingConclusion)
```

Retryable Store failures and permanent Store, epoch, durability, capacity, or integrity failures
travel on a separate typed error channel. Every error returns the same `PendingConclusion`; only an
explicit supervisor force-destruction may discard it under the documented response-loss policy.
There is no undefined semantic conclusion sink.

Access conclusion qualification must match the same cumulative input identity bound through the
selected `StatePrepared` and `CommittedCall`; Pure qualification matches the ready occurrence's
selected predecessor context. Rebinding never widens, refreshes, substitutes, or reinterprets that
input.

A same-run head change cannot preserve the same ready/selected conclusion precondition in this
three-family sequential model. Runtime consumes the outer owner, qualifies the new head, and must
classify it as the same or conflicting conclusion, another preparation selected, terminality, or
invalid history; it never rebinds the conclusion across that head. A Pure occurrence lacking its
conclusion must still be ready. Only the
independent conclusion-time fact-publication frontier may move while the run occurrence/selection
remains current. In that case Runtime moves the inner `PreparedConclusion` through Store, verifies
Store/epoch/tenant/run/Program, fact dependency, exact capacity, and the same Access selection or
Pure-ready occurrence, then binds a new physical append/fact coordinate after the prior attempt is
proven absent or definitely precommit. No branch re-executes State logic, the adapter, response
ingress, interpretation, or a fact scan.

An ambiguous conclusion acknowledgement first resolves its exact physical append identity:

- found same: resume from qualified recorded history; do not republish facts;
- retained physical append proven absent but the same canonical conclusion already recorded under
  another physical append id: `AlreadyConcludedSame`; do not republish facts;
- proven absent with the same Access selection or Pure ready occurrence: retry or rebind the same
  conclusion;
- replacement selected for Access: `NoLongerSelected`;
- different durable conclusion: `Conflict`;
- Pure occurrence no longer ready with no durable conclusion: invalid history;
- repeated ambiguity or retryable failure: retain the same `PendingConclusion`; and
- explicit owner loss: cold history contains either the durable conclusion or the prior reducer
  state--a ready Pure occurrence or the selected Access preparation.

### 6.5 CAS worker linearization

MFM adds no per-run execution guard, `Busy` lifecycle, backend-wide Runtime-process lease, or
cross-process takeover protocol. A worker independently qualifies one exact prefix and consumes
its affine local session into a candidate. Another worker may qualify the same prefix. The Store's
route-scoped append-id lookup, exact-head compare-and-append, callback-free reducer, and semantic
disposition are the shared linearization boundary. The writer epoch remains an immutable persisted
identity and append precondition, not a live mutex.

At any durable head the reducer exposes at most one actionable occurrence and one selected
unresolved preparation. Local affinity ensures that one worker owner contains at most one live
call or pending conclusion, while the direct-new rule ensures that one preparation releases at
most one framework invocation. It does not claim process-global exclusion. A slow Read or
absorbing attempt may remain live while another worker commits a permitted replacement; an old
result is accepted only if its preparation is still selected. `EntryAbsorbing` must cover this
full overlap horizon. This worker contention is not a Program scheduler, lane, or multi-result
workflow construct.

The required races linearize as follows:

- concurrent identical admissions: one direct-new genesis wins; a distinct-id `StaleHead` loser
  qualifies the existing admission and returns recorded history if semantically same or conflict
  if different; same-id `Found` remains acknowledgement/idempotency recovery and likewise resumes
  only from qualified history;
- preparation versus preparation at one head: one direct-new winner may invoke; every loser invokes
  zero times and may propose a later replacement only after requalifying the reducer-selected head;
- conclusion versus another conclusion for the same selected preparation: one append wins; the
  other becomes identical or conflict;
- conclusion versus replacement: a conclusion winner closes the occurrence; a replacement winner
  supersedes the old preparation and makes its result `NoLongerSelected`;
- conclusion versus another same-run append: classify the new head as same/conflicting conclusion,
  another preparation selected, terminal, or invalid; never rebind across it;
- conclusion versus independent fact-publication movement: rebind only after proving the same
  Access selection or Pure-ready occurrence;
- terminal-producing conclusion versus any stale candidate: the terminal winner commits and the
  stale candidate cannot rebind; and
- in-flight same-capability calls, whether on different runs or different permitted attempts of one
  run, cannot transpose Runtime shells, preparation refs, adapter results, fact continuations, or
  conclusion owners.

## 7. Recovery and entry semantics

Recovery is selected by the callback-free reducer. A State implementation may choose only among
actions exposed by its capability contract:

- **Read:** may append a fresh bounded replacement with the exact canonical intent and
  immutable binding. A separately declared freshness coordinate may advance without changing that
  operation.
- **EntryOnce:** an unresolved selected preparation parks indefinitely. No timeout, generic error,
  operator attention, process loss, or binding change proves non-entry. A future operator action
  with independent meaning requires a specifically named event and a new journal design.
- **EntryAbsorbing:** may append a bounded replacement only with the exact canonical intent,
  binding, effect domain, and capability-derived absorption identity.
- **Late result:** a result for a superseded preparation is `NoLongerSelected` and never settles the
  occurrence.
- **Cold history:** can select recovery but never reconstructs execution authority for an old
  preparation.

`EntryAbsorbing` asserts more than the presence of a key-shaped field:

1. repeated external entry is actually absorbed;
2. the adapter transmits or enforces the mechanism;
3. absorption lasts for the entire recovery horizon, including a slow prior attempt overlapping a
   replacement and uncertainty across process loss;
   and
4. every accepted result is a function of external post-state rather than whether one exchange was
   first.

`RowsAffected(1)` versus `RowsAffected(0)` is not convergent evidence. Returning the canonical row
after insert-or-select may be. These are capability/adapter TCB obligations and must be reviewed and
tested for every absorbing production operation.

## 8. Facts, sessions, readers, replay, and configuration

### 8.1 Prior-run facts and publication

When a capability declares prior facts, `StatePrepared` fixes the admitted source manifest, bounded
selection request/digest, preparation-time frontier, and relation to the canonical operation. Only
the direct-new preparation commit mints a one-use Store fact continuation, which moves inside
`CommittedCall`. It is bound to the exact Store, tenant, preparation, sources, frontier, and limits,
and may be retained across retryable pre-provider scans. Cold history and adapters cannot mint or
own it.

Selected facts may inform evidence interpretation but cannot change the committed provider
operation, binding, effect domain, or absorption identity. If they must change provider semantics,
selection occurs before preparation and becomes part of canonical intent.

`StateConcluded` retains the accepted selection response and completeness attestation, exact
producer records/heads, selected identities, facts, and output closure needed for callback-free
recomputation. Preparation-time selection frontier and conclusion-time publication frontier are
different coordinates. Later tenant publications do not widen an accepted selection; only a
conclusion publishing facts compares and advances the current dense publication head.

One Store transaction atomically appends the conclusion and object closure, advances the run head
and reducer projections, publishes at most one dense fact route, advances the tenant fact head, and
updates the terminal projection. Rollback is all-or-nothing.

### 8.2 Qualified history and Runtime sessions

`QualifiedRun@H` is opaque, callback-free semantic evidence for one exact prefix.
`ActiveQualifiedRun@H` adds the private Store coordinator. `RunSession@H` adds the exact Runtime
assembly brand and is affine, non-serializable, and non-cloneable.

Purpose readers receive only callback-free evidence. Store exact-head comparison and reducer
qualification classify competing workers and stale/hostile
commands. Direct admission and conclusion commits advance that worker's retained session without
reload. A direct-new preparation commit transfers its successor into that worker's
`CommittedCall` and then `PendingConclusion`; that affine owner contains no second local session.
Other workers may hold independently qualified sessions, but no non-direct-new branch can borrow
or reconstruct this one's call authority.

The conclusion path performs its one required canonical encoding, while the hot session retains the
already-qualified typed latest cumulative context and passes that same object into the next
implementation without a serialize/decode round trip or prefix fold. A fresh cold resume qualifies
and folds the complete bounded durable prefix callback-free, validates every predecessor/context
link, recovers the exact latest concluded context, and binds it as the next immutable input. It
exposes no history capability to State code.

There is no shared semantic LRU, suffix-refresh protocol, cloneable session, or durable reducer
checkpoint. A fresh resume performs one bounded complete-prefix ingress and fold. Session and
ingress work use explicit bounded resource permits; expensive decode/reduction and deterministic
state work do not block the async runtime.

### 8.3 Demand-time qualification and replay

Ordinary Store open qualifies schema, Store identity/epoch, backend channel and durability, writer
role, exact Program catalog, and bounded transaction ability. Multiple Store/Runtime workers may
open against the same admitted writer identity; their mutations share the backend's exact-head and
append-id linearization. Store open does not know Runtime assembly or enumerate retained run/
configuration history. Runtime composition separately validates the exact catalog-to-assembly
association before any session can exist.

A retained run crosses ingress only when resume, read, trace, replay, export, explicit audit, or a
selected fact dependency consumes it. Malformed dormant history fails that operation without
poisoning readiness or unrelated runs. `audit_store` is a separate read-only, fixed-snapshot
diagnostic and creates no execution authority.

Live, recorded, and portable replay all use the same Store qualifier and reducer. Replay cannot
construct a `CommittedCall`, invoke a State implementation, scan a live Store not present in its
closure, contact a provider, or append. Every valid prefix—including an unresolved preparation and
a terminal prefix—replays with zero callbacks and zero live I/O. Replay validates every cumulative-
context link and rejects old FanOut/lane/join Program or history identities; no compatibility
decoder or fallback survives.

### 8.4 Configuration history

Configuration is a separate append-only stream. External sources and retained rows cross one typed
ingress into `ResolvedConfiguration<T>@Head`; locally typed values are canonicalized once without
hostile reparse. Planning consumes the typed value directly.

Configuration writers use their own coherent `PreparedConfigurationAppend`, exact-head commit,
direct-successor promotion, idempotency, and acknowledgement recovery. The three-family run rule
does not rename configuration revisions or collapse the two domains into a generic history API.
Secret-bearing configuration is consumed into live adapter internals and never persists.

Each configuration stream admits at most 1,024 revisions, 64 MiB of cumulative canonical revision
bytes, and 16 MiB for any one revision. Every ingress, selection, audit, and write path enforces the
same limits. No configuration-load latency is promised.

## 9. Application, export, and deployment surface

`Application` is fixed at construction to one deployment-supplied `TenantScopeId`. Operational
methods accept neither credential, principal, grant, policy-decision reference, nor tenant selector.
Delete the internal access-policy model, CLI access-token option, REST bearer handling, policy-only
401/403 errors, persisted policy evidence, and compatibility aliases. Do not replace them with
headers, optional fields, or a default-allow policy.

Portable export becomes one strict, bounded, callback-free structural closure with no principal,
grant, or policy decision. Its complete stream content identity fixes the bundle; old formats are
rejected. Offline qualification is read-only and does not create an App/CLI/REST persistence-import
surface.

EVM submission identity uses tenant, wallet nonce domain, and a bounded non-secret idempotency key.
Delete the authenticated-issuer/policy namespace and activate the changed domain only with the
fresh logical Store scope/writer epoch, wallet domain, and sender required by U4.

Concrete adapters own transports, signers, keystore actors, protocol credentials, wallet/nonce
transactions, and permanent operation keys. Delete generic live binding refresh, release lineage,
revocation, and callable nested resource handles only after every immutable/protocol proposition
has a final owner. Preserve genuine provider protocol authentication, Store epochs, SQL locks,
nonce reservations, transaction permits, and secret controls.

### 9.1 Sequential EVM balance context

EVM balance collection is a declaration-ordered cumulative State chain, not a source selector plus
inner fan-out. The domain owns a bounded nominal context parameterized by one caller-owned
continuation:

```text
EvmBalanceContext<K: MfmValue> {
    admitted source demand, caller continuation K, and bounded opaque result correlation,
    collection/binding identity and next source position,
    current source and stage,
    checked chain identity and anchor work,
    token decimals and balance observations when applicable,
    completed source results,
    remaining source demand,
}
```

`K` is carried unchanged and has no EVM interpretation API. It is never erased bytes,
`QualifiedValue`, a history reference, or a heterogeneous map. Commit 0 must prove the exact
monomorphized value-contract/identity scheme or use a concrete nominal instantiation per caller.
Separately, one bounded `EvmBalanceResultMetadata` carries the caller-owned
collection ordinal and canonical correlation fields required by the existing public EVM result.
EVM may move those fields unchanged into `EvmBalanceCollectionResult` but cannot inspect or derive
Portfolio semantics from them; they are not a serialization of `K`.

For each source the common prefix is chain identity, initial anchor, and asset `Match`. The native
arm executes `ReadNativeBalance`; the ERC-20 arm executes `ReadTokenDecimals` followed by
`ReadTokenBalance`. Both return the same observed cumulative-context contract, after which anchor
confirmation appends one completed source and advances to the next source. Chain identity, initial
anchor, token decimals when applicable, native/token balance, and anchor confirmation are each an
explicit Read-classified State occurrence. Any failure takes its explicit route before a later
State or source is prepared.

There is no Runtime loop declaration. Before `RunAdmitted`, bounded pure planning/child expansion
unrolls one declaration-ordered stage chain for every admitted source. Each stage is a stable State
occurrence with a direct or Match-arm continuation; confirmation proceeds to the statically next
expanded source occurrence. Planning rejects an empty demand or more than 64 total EVM sources.

After exhaustion, a Pure `EvmBalanceConsolidation<K>` consumes the complete context and owns demand
realization, collection/binding consistency, common-anchor validation, duplicate/missing/foreign-
source rejection, declaration ordering, balance construction and mathematics. It returns the
conceptual nominal successor
`EvmBalanceCollectionCompletion<K> { caller_continuation: K, result:
EvmBalanceCollectionResult }`. `EvmBalanceConsolidation` itself constructs that final EVM result;
the wrapper only returns the opaque continuation beside it for reuse. Portfolio's Pure resume
consumes that one value. A standalone EVM root uses a concrete standalone continuation and adds a
caller-owned Pure projection that unwraps the already-produced result. The U8/U11 preflight records
the exact consuming ownership and strict schema identities; `EvmBalanceResultMetadata` allocates
every public field once. Current public canonical bytes remain unchanged. This is one typed
successor, not a generic multi-result product.

### 9.2 Sequential Portfolio context

Portfolio owns a bounded cumulative snapshot context containing its admitted input, validated
routing/configuration, current collection position, completed EVM collections, remaining collection
demand, and final work. A nominal `PortfolioContinuation` owns that complete context and is valid by
representation: admitted demand equals completed plus the current collection plus the remaining
collections, with dense ordinal, declaration order, and binding consistency. Before each child
call, a Portfolio-owned Pure entry State consumes the current Portfolio context and constructs the
complete nominal `EvmBalanceContext<PortfolioContinuation>` wrapper, including the opaque
continuation and EVM-owned source/result metadata. The EVM fragment consumes that exact singular
entry value; pure expansion does not synthesize or transform it. EVM returns the continuation
unchanged with one `EvmBalanceCollectionResult`; a Portfolio-owned Pure resume State validates the
continuation, appends the result's collection, and advances to the next collection. EVM never
interprets Portfolio semantics.

Pure expansion likewise unrolls one EVM collection fragment and Portfolio resume State per admitted
collection. The sequential Program contains no runtime collection loop, selector cursor, or dynamic
scheduler.

A final Pure Portfolio consolidation consumes the complete accumulated context and owns extraction,
cross-collection validation, totals, and public-output construction. Preserve existing public EVM
and Portfolio result semantics. Delete the current
caller-context JSON echo/decode, source/collection selectors, lane inputs/cursors/results, nested
fan-out authoring, joins, and fan-out-specific failure mapping. A collection failure terminates or
takes one explicit Portfolio domain recovery route before any later collection is prepared.

## 10. API and deletion cutover

### 10.1 Program and package ownership

Retain one opaque `Program`, one serializable `ProgramDocument`, one callback-free catalog, one
Runtime assembly, one Store reducer, and one replay qualifier.

Move the normalized compiler/document work into `mfm-program`, live implementation assembly into
`mfm-runtime`, and callback-free retained semantics into `mfm-store`. Delete superseded
`mfm-spec`, `mfm-certify`, and empty authority-seal packaging after their real obligations have
moved. Final dependency direction is Runtime to Store; Store never depends on Runtime.

The final Program document exposes only State and Match declarations. Authored child operations are
purely expanded away; no expanded fragment boundary survives. Delete all authored/expanded FanOut,
lane, join, lexical slot/producer/origin graph, profile, schema, decoder, and builder surfaces. The
sequential pipeline carries only the current typed `Value<C>`; Match consumes that exact value and
does not retain an ambient binding/provenance map. Do not replace them with `Collect`, `Gather`,
workflow `Join`, lanes, barriers, `state_many`, an ambient output map, or a generic cumulative-
context type.

Delete the old plural admission and child-binding contract in the same Program cut, including
`OperationBuilder::input`, `AuthoredStructuredProgram::input_roots`,
`ExpandedStructuredProgram::input_roots`, normalized `input_root_slot_refs`, admission
`initial_values`, journal/Store `initial_bindings`, plural entry-contract input refs,
`Value::bind_child`, `ChildInputBinding`, `FragmentInputBinding`, and ordered child/capability/
policy input-bijection machinery. Fresh Program, entry-point, admission, and `RunAdmitted` forms
expose exactly one `C0` contract/value and reject the old plural fields and identities without a
compatibility decoder.

### 10.2 State and access API

Introduce or retain in final form:

- callback-free State declarations and State bindings;
- capability-owned canonical intent, strict accepted evidence, entry/absorption, fact, and capacity
  contracts;
- process-local `StateImplementation<S>` and `QualifiedAdapter<S, C>`;
- private `PreparedExecution -> CommittedCall` transition;
- call-correlated accepted evidence;
- one Runtime-owned `PendingConclusion` path around a Store-owned `PreparedConclusion`; and
- one-use preparation-bound prior-fact continuation.

Delete rather than rename or alias:

- `StateTransitionCommitted`;
- `ExternalAccessAuthorized` and `ExternalAccessReserved`;
- `ExternalAccessObserved` and `ObservationOutcome`;
- journal `RunClosed` and all four superseded logical-key families;
- `PreparedReservationAppend`, `CommittedReservation`, `ReadyToInvoke`, and reservation-named
  Runtime reducer states;
- Runtime-phase `AcceptedAccessResponse`, `ReadCompletion`, and `EffectCompletion` wrappers used to
  return a response through Runtime;
- split request, returned-settlement, and safe-failure-settlement callback APIs;
- `ObservationWriteContinuation`, `PreparedObservationAppend`, `RetryObservationPreparation`,
  `ObservationRebaseInput`, `UnresolvedObservationAppend`, observation commit dispositions, and
  observation-specific `SuspendedRun` variants;
- `PreparedDrive::Observation`, `CommittedDrive::Observation`, separate post-observation scheduling,
  and cold observation settlement;
- authorization/reservation/observation terminology in fact attestations where it denotes the old
  run protocol; and
- all old tags, schemas, fields, projections, SQL assumptions, goldens, fixtures, decoders, aliases,
  feature escape hatches, and fallback paths.

Domain-specific wallet/nonce reservations, provider observations, and protocol authorization names
remain where they describe real external concepts.

### 10.3 Store and PostgreSQL

Keep exact-head append atomicity, writer epoch, qualified durability, complete-frame bounds,
found-attempt ingress, acknowledgement ambiguity, dense fact publication, content addressing, and
Memory/PostgreSQL conformance.

Cut the PostgreSQL run schema directly to the three-family strict frame and occurrence-level
conclusion uniqueness. Defense-in-depth unique constraints may cover admission, preparation ordinal,
and occurrence conclusion, but the reducer remains authoritative. Remove object-row decomposition
and target-currentness tables only as specified by their separate ownership cut; do not make the
three-family change coexist with a five-family reader or projection.

## 11. Authoritative documentation changes

Implementation rewrites `docs/design.md` and `docs/architecture.md` with their owning code commits.
They must describe:

- the single-ingress rule and honest TCB boundary;
- opaque Program authority and immutable process assembly;
- durable State declarations versus process-local implementations and qualified adapters;
- separate admission, Pure direct conclusion, and access-only preparation;
- direct-new-only `CommittedCall` construction;
- the accepted process-local response-loss window;
- atomic `StateConcluded`, occurrence-level uniqueness, selected-preparation recovery, and derived
  terminality;
- State/Match-only sequential control, exact cumulative-context continuity, and one-preparation
  maximum-conclusion capacity;
- singular domain-planned `C0`, exact one-context child expansion, and rejection of the plural
  root/binding wire;
- CAS-linearized same-run worker races with no per-run guard or global Runtime-process lease;
- mode-neutral terminal root production, with Access output released only after durable qualified
  conclusion;
- sequential EVM balance and Portfolio cumulative contexts and their Pure consolidations;
- callback-free demand qualification, facts, replay, and projections; and
- PostgreSQL durability and rollback limitations.

Update `docs/run-execution.md`, `docs/known-gaps.md`, persisted/public surface documentation,
Program/Capabilities/Store/Runtime/Replay/App crate READMEs, EVM transaction/routing docs, and
CLI/REST references in the same commits as their contracts. Delete obsolete credential,
observation-lifecycle, currentness, and compatibility documentation rather than labeling it legacy.

## 12. Verification plan

### 12.1 Program, API, and type boundaries

Compile-fail and API tests prove:

- `Program`, `QualifiedRun`, `RunSession`, `PreparedExecution`, `CommittedCall`,
  `PendingConclusion`, Store tokens, fact continuations, and Runtime brands cannot be field-
  constructed, deserialized, or cloned where affinity matters;
- Pure/Read/Effect implementation registration cannot cross modes or capability ABIs;
- intent and evidence project only from the durable capability contract;
- no public or feature-gated provider-entering method works without a committed call;
- no callback-free Store or Replay value can invoke even a Pure implementation; and
- supported State implementation constructors/callbacks expose no run history,
  Store/reducer/journal authority, ambient output map, or context outside `S::Input`, and ordinary
  domain crates cannot import those owners;
- the strict final Program declaration algebra contains only State/Match, with authored children
  expanded away;
- exactly one domain-planned typed `C0` crosses every authored/expanded/certified entry contract,
  admission command, and `RunAdmitted`; repeated roots and ordered child-input bindings are
  unrepresentable, and negative tests reject `OperationBuilder::input`,
  `AuthoredStructuredProgram::input_roots`, `ExpandedStructuredProgram::input_roots`,
  `initial_bindings`, and the other deleted plural carriers; and
- old five-family and FanOut/lane/join/Collect types, packages, feature bridges, aliases, and
  decoder surfaces are absent.

### 12.2 Journal and reducer

Golden and hostile tests cover:

- exactly the three record families and three logical keys; old tags and fields reject;
- separate admission, one-record batches, zero-state terminality as the exact admitted `C0`
  contract/content identity, rejection of any different value/contract or extra root object, and no
  `RunClosed`;
- same-id admission `Found` recovery versus distinct-id genesis `StaleHead` races, whose qualified
  existing admission classifies semantic same/different/invalid without callbacks, including
  unknown -> proven-absent resubmission -> `StaleHead`;
- every Program/input/intent/binding/replacement/evidence/outcome/fact/object/capacity mutation;
- linear replacement ordinals, exact parent selection, no branch/cycle/gap, and mode-specific
  replacement rules;
- one conclusion per occurrence across preparations, exact duplicate idempotency,
  Access `NoLongerSelected`, Pure same/conflict classification, and invalid correlations;
- every valid prefix exposes at most one actionable occurrence and rejects preparation of a later
  occurrence while the current one is ready or prepared;
- exact predecessor-to-successor context continuity, including rejection of stale, truncated,
  reordered, foreign, wrong-contract, or extra cumulative values;
- Match exhaustiveness, selected-arm determinism, exact continuation-context convergence, and zero
  callbacks for unselected arms;
- zero-state admission is terminal; nonzero terminal Pure, terminal Read, and terminal Effect
  States each produce an exact declared root result; a mismatched root contract or remaining
  continuation rejects; terminal Access acknowledgement-unknown -> found-same returns the recorded
  root without re-entry; and no Access result is public before its durable qualified conclusion;
- zero-state means an empty expanded control form with exact `C0` identity; Match-only and selected
  empty-arm terminal paths reject rather than synthesizing a root without `StateConcluded`;
- one selected-preparation reservation, cumulative-context/conclusion/run exact bounds, and each
  independent bound-plus-one; and
- rejection of every state record after terminality, including hostile retained suffixes.

Direct advancement and fresh complete-prefix ingress must produce equal reducer state, indexes,
fact dependencies, capacity accounting, and terminality at every prefix.

### 12.3 Preparation and invocation

Tests prove:

- provider count is zero before durable preparation; direct-new creates exactly one eligible call,
  provider entry remains zero or one, and the consuming fixture observes one when it reaches the
  official adapter-entry method;
- `StateImplementation::execute` count is at most one for the consuming direct-new owner; every
  found-same/different, stale, terminal, conflict, invalid input/bytes, other-preparation-selected,
  known-precommit retryable, capacity, epoch/durability loss, fact-precondition loss, ambiguous,
  and cold-history branch has execute and provider counts zero;
- ambiguity resolution covers found-same/different/invalid, proven-absent direct-new,
  stale/terminal/other-preparation-selected, repeated unknown, explicit drop, and cold restart;
- hostile/adversarial same-head preparation commands yield one direct-new committed-call owner for
  that head, while every losing command has zero execute/provider entry;
- concurrent spawn/spawn, spawn/resume, and resume/resume workers may qualify and speculatively
  evaluate/prepare the same sequential occurrence, but identical admission and preparation races
  produce one direct-new admission append and, for a preparation race, one call authority; every
  non-new preparation branch has zero execute/provider entry, and stale workers resume or
  reprepare only from newly qualified history;
- Memory and PostgreSQL allow multiple qualified workers/processes for one Store scope/epoch and
  produce identical CAS dispositions; no `Busy`, execution-guard, exclusive-open, or live-writer-
  process-lease API exists;
- failure immediately follows its declared route and every later normal State has zero evaluation,
  preparation, execute, and provider counts;
- affine execution owners, accepted wrappers, fact continuations, and pending conclusions cannot be
  serialized, deserialized, copied, defaulted, cloned, or consumed twice;
- the U8/U11 fixture moves a non-`Clone` cumulative `MfmValue` exactly once through Pure, ordinary
  Access Outcome/State-failure, Match, and EVM/Portfolio continuation paths without implicit
  duplication;
- same-type transposition attempts across Store, run, occurrence, input, intent, preparation,
  implementation, adapter, binding, domain, fact continuation, and call correlation fail; and
- cancellation or panic before provider entry does not detach a framework-owned provider future.

### 12.4 Evidence, conclusion, and response loss

The closed result matrix covers:

- provider success, returned rejection, accepted safe failure, definite-pre-entry evidence, and
  accepted integrity blocking produce a conclusion;
- cancellation/caught panic before the consuming conclusion handoff, malformed/unbound/
  unauthenticated response, transport ambiguity, possible entry, and lost trustworthy result leave
  only `StatePrepared`; cancellation/caught panic after handoff retain `PendingConclusion`;
- generic errors cannot mint definite-pre-entry or `EntryOnce` retry authority;
- hot and cold `BlockedIntegrity` take the same Program/capability-certified failure route with no
  additional State interpretation/routing callback (and zero cold callbacks), outputs, facts, or
  context recovery;
- raw responses, provider diagnostics, and wrong request/route/target/signer/binding never reach
  state interpretation or persistence; and
- each direct-new call owner performs accepted-evidence ingress and State interpretation at most
  once; each `PendingConclusion` canonicalizes once; conclusion recovery repeats neither; and each
  consuming Runtime owner returns at most one public result. Competing workers may independently
  perform their own speculative Pure evaluation or directly authorized Access path.

Conclusion recovery tests cover direct commit, identical existing conclusion, different conclusion,
late result, replacement race, same-run-head rebind rejection, fact-publication rebind,
acknowledgement found/invalid-or-over-limit/absent/alternate-physical-same/repeated-unknown,
explicit owner loss,
same-type pending-conclusion transposition, and process restart. Provider invocation, ingress, and
interpretation counters remain unchanged throughout recovery.

For Access, process loss is injected after provider return, ingress, interpretation, synchronous
conclusion handoff, and pending-owner construction. Cancellation/panic before the consuming handoff
may leave only preparation; cancellation immediately after handoff must retain/recover the owner.
During conclusion commit and after acknowledgement, only process death or explicit force-
destruction may lose it. Pure process-loss points cover evaluation, proposal construction,
canonicalization, handoff, and conclusion commit; post-handoff cancellation also retains its owner.
Replay sees the conclusion or the mode's prior prefix and never fabricates an entry claim.

### 12.5 Recovery, facts, replay, and secrets

Tests cover:

- Read bounded replacement and exact canonical intent preservation;
- `EntryOnce` parking with no second preparation;
- `EntryAbsorbing` exact intent/binding/domain/identity/input preservation, total-entry budget, an
  old slow operation or pending result overlapping a permitted replacement, including after
  owner/process loss, full retention horizon, and convergent post-state evidence;
- direct-new-only, one-use, non-transposable prior-fact continuation; fixed preparation selection
  frontier; dense negative completeness; and atomic conclusion/fact publication;
- explicit upstream Read/cumulative-input modeling for provider-affecting facts and rejection of any
  hidden pre-prepare or post-commit intent-changing fact path;
- across a direct State edge, State N receives the exact complete context concluded by State N-1;
  across Match, the selected arm receives the exact qualified complete variant payload; Pure and
  Access advance one chain; early data remains available late; hot and cold selection produce
  identical next context and intent, with one conclusion encoding but no hot serialization/redecode
  round trip;
- replacement preserves the exact cumulative input contract, content identity, and typed value;
- a positive recovery route receives prior context only because its domain failure value carries
  it explicitly; ordinary failure has no ambient history path;
- duplicate conclusion does not republish facts and failed/integrity conclusions publish none;
- every replay prefix performs zero State, adapter, scanner, provider, or other live callbacks;
- every replay prefix derives the same next sequential cursor and exact qualified cumulative input
  as direct reduction, while stale/foreign/truncated/inserted/reordered/stage-rewound/skipped/extra
  contexts fail replay qualification;
- malformed dormant history is isolated to its consumer; and
- secret canaries never appear in Program/configuration, `RunAdmitted`, admission context/source
  manifests, any canonical journal frame, preparation, evidence, pending/concluded values, newly
  reachable object/output closure, facts, portable export, public DTOs, errors, traces, or debug
  formatting.

### 12.6 PostgreSQL, application, and performance

Memory/PostgreSQL conformance and PostgreSQL-specific tests cover exact-head contention, transaction
rollback at every conclusion/publication statement, acknowledgement ambiguity, primary crash/
restart durability before provider entry, weak durability rejection, writer-epoch mismatch,
same-epoch multi-process CAS races, schema and SQL inventory, tenant-first lookup, hostile rows,
derived terminal projection, and old-schema rejection.

Application/transport tests retain fixed-tenant isolation, credential/policy deletion, strict
portable closure, EVM identity, redacted errors, and non-interactive behavior. EVM/Portfolio tests
prove declaration-ordered sequential sources and collections, exact per-source State order,
fail-fast suppression of all later work, opaque typed continuation round-trip, anchor/duplicate/
binding/mathematics behavior, and canonical public-result equivalence.

Benchmarks measure maximum Program ingress, `C0` and maximum `Cn` construction, every cumulative-
context append, frame/conclusion/run construction and ingress, maximum hot-session advancement,
maximum cold resume, exact source/collection/occurrence/context/object/database bounds, every +1,
fact closure, configuration history, and one-shot transport resume. The hot path performs the one
required canonical conclusion encoding; handing the retained typed output to the next State adds no
serialize/decode round trip, historical read, or fold.

## 13. Logical implementation sequence

Each commit includes its contracts, tests, schemas, fixtures, and documentation. No old/new public
API pair, decoder, reducer, or fallback survives a cutover commit.

0. **`resolve prepared-call execution gates`**

   Record the three bounded preflight artifacts: the two-entry-point capability/binding inventory,
   final cumulative-context/type table plus U8/U11 compile-spike result, and U1/U7/U9 MVP capacity
   envelope. Record the settled activation, dormant-run, and primary-crash/restart rules. Inventory
   and delete the rejected FanOut/Collect and Effect-attention branches from both documents.

1. **`scope application facades by tenant`**

   Cut App, CLI, REST, and EVM identity to fixed-tenant, credential-free structural contracts.
   Delete policy persistence and old identities atomically; disable pre-cutover portable export/
   import until final structural v5 lands with three-family frames in Commit 4.

2. **`make program and reduction sequential by construction`**

   In one compiling cut, land opaque Program/value/catalog/compiler authority, State/Match-only
   Program documents, cumulative context contracts, callback-free State declarations, and the
   minimal sequential cursor in every schema-coupled consumer. Cut singular `C0` through Program,
   entry contracts, App admission commands, the current coherent `RunAdmitted`, and Store
   qualification/compiler/reducer in this same commit; retain no length-one plural container.
   Delete FanOut/lanes/joins from Program/spec/certification plus directly coupled Journal origins,
   Store/Runtime walkers, EVM/Portfolio authoring, schemas, fixtures, and UI tests. Authored child
   fragments expand away.
   The Commit-0 U3/U5 inventory and U8/U11 proof fix immutable descriptors and final cumulative
   Input/Output/Failure value identities plus callback implementations under the retained access
   ABI. Retain no compatibility Program variant or cursor. Commit 4 replaces any intermediate
   access-State Program identity while preserving those cumulative value identities.

3. **`make store ingress and appends one semantic path`**

   Invert Store/Runtime dependencies and move the already-current sequential cursor under final
   callback-free Store ownership with one reducer/binder path over the unchanged pre-cutover
   backend/schema, singular preparation capacity, typed Facts response ownership, configuration
   reads, and planning prerequisites. Retain only the one coherent current access protocol. The
   complete-frame SPI and final PostgreSQL v8 land in Commit 4.

4. **`execute committed preparations and conclude states atomically`**

   One inseparable vertical cut across Program/Capabilities, Journal, Store, Runtime,
   State/adapter registration, facts, projections, replay/export v5, complete-frame Store SPI,
   Memory/PostgreSQL v8, App, and the sequential EVM/Portfolio live execution ABI and
   registration using the cumulative value identities fixed in Commit 2, plus goldens, tests, and
   documentation. Introduce the three families, `PreparedExecution`, `CommittedCall`, capability
   evidence, and `PendingConclusion`; delete the complete five-family protocol and reject every old
   journal, portable, and persistence identity
   in this commit. Old FanOut-capable Program identities were rejected in Commit 2.

5. **`qualify retained history only when consumed`**

   Remove eager readiness scans, unify live/read/replay/export/audit/fact consumers behind the one
   demand qualifier, and retain only scope-local prefix reuse.

6. **`make configuration writes one semantic path`**

   Complete the independent affine configuration writer cut with direct successor promotion and no
   reload, echo, cache, or compatibility reader.

The final audit reports product LOC/public-type reductions and proves that no FanOut/lane/join/
Collect workflow type, tag, branch, test, schema, current-design document, or compatibility identity
survives. This RFC, its implementation plan, and the scanner manifest may retain those names only as
explicit deletion/rejection history; legitimate ordinary domain/tooling words follow the scoped
allowlist.

Run focused verification for each affected boundary. On the final tree, run `nix run .#ci` once;
do not immediately precede it with redundant `.#check`, `.#test`, and `.#test-db` runs.

## 14. Acceptance criteria

The cutover is complete only when:

- the run journal and every strict decoder expose exactly `RunAdmitted`, `StatePrepared`, and
  `StateConcluded`;
- admission is a separate sole genesis, zero-state terminality is derived, and no run record
  appends after terminality;
- Pure produces no preparation; Store contention and ambiguity retain its already-computed pending
  conclusion without rerunning the callback;
- every access intent/evidence contract is callback-free Program/capability data and every live
  implementation/binding association is immutable and exact;
- the supported provider-entering path is unreachable until the exact preparation directly commits;
- only that direct-new branch constructs one exact affine `CommittedCall`; every non-new,
  indeterminate, stale, invalid, and cold branch invokes zero times;
- a preparation-acknowledgement quarantine can invoke only after proven absence and a later direct-
  new resubmission;
- call correlation resists same-capability substitution and the adapter derives request bytes from
  the committed canonical intent;
- provider material crosses one bounded, authenticated, request/call-bound ingress into strict
  capability evidence, with no raw diagnostic persistence;
- definite results conclude, unresolved operational outcomes remain ephemeral, and generic errors
  grant no `EntryOnce` retry authority;
- each result's one secret-free `PendingConclusion` owns its conclusion retries, physical-id
  recovery, fact-frontier-only rebinding, and acknowledgement ambiguity without provider/fact/
  state-callback I/O or repeated interpretation; only conclusion Store I/O remains;
- loss of an Access owner before conclusion durability yields only `StatePrepared`; loss of a Pure
  owner yields the prior ready/admitted prefix; no successful result escapes before conclusion
  durability;
- Store admits at most one conclusion per occurrence across all preparations and correctly
  distinguishes identical, superseded, conflicting, and invalid results;
- the final Program algebra is exactly State/Match, every valid prefix has at most one actionable
  occurrence, and no FanOut/Collect/parallel workflow identity or compatibility decoder survives;
- every operation admits exactly one domain-planned typed value as `C0`; Program, entry,
  admission, journal, and child-expansion contracts contain no root list, initial-binding list, or
  ordered input bijection;
- only an empty expanded control form may terminate from admission by returning exact `C0`;
  Match-only and selected empty-arm terminal paths reject, while every nonempty successful path
  ends in a terminal State;
- each successful nonterminal conclusion commits the complete successor cumulative context; a
  direct next State receives that exact value, while Match qualifies the exact declared variant
  payload as its selected arm's complete input;
- Match arms converge on the exact continuation context, failures prevent later normal work, and
  recovery sees prior context only through an explicit domain failure route;
- Read and absorbing replacement preserve their exact contracts and budgets; `EntryOnce` parks;
- every absorbing capability demonstrates actual absorption, full-horizon retention, stable-key
  enforcement, and convergent post-state evidence;
- the sole selected preparation reserves one maximum conclusion and cumulative context,
  conclusion, source/occurrence, total-run, object, and backend bounds reject before entry;
- facts and outputs publish atomically with conclusion, duplicate conclusions never republish, and
  replay recomputes them callback-free;
- hot advancement canonically encodes the conclusion once but passes the already-typed latest
  context without a serialization/redecode round trip; cold resume recovers the identical next
  context callback-free, and replay invokes no callbacks;
- competing workers linearize only through qualified exact-head Store operations: the durable head
  exposes one action/selection, one preparation releases at most one call, one affine worker owner
  contains at most one call/pending conclusion, and permitted Read/absorbing attempts may overlap;
- Store readiness opens no dormant history, while explicit consumers reject malformed selected
  history before returning authority or output;
- PostgreSQL qualifies the admitted durability profile and writer epoch before any direct-new call
  can exist; multiple same-epoch workers use CAS without a process lease, while legitimate restore
  rotates identity/epoch and does not append to the old run;
- fixed-tenant App, credential/policy deletion, strict portable export, EVM identity, configuration,
  redaction, and secret contracts remain satisfied;
- sequential EVM sources and Portfolio collections preserve their public results, ordering,
  anchors, duplicate checks, mathematics, and typed opaque continuation boundary;
- every observation/reservation/transition/closure-era type, tag, logical key, projection, SQL
  assumption, fixture, alias, feature escape hatch, and fallback is absent; and
- final documentation describes only this design and the final report lists all verification,
  public/dependency/LOC deltas, deployment gates, and known limitations.

When these conditions hold, durable preparation gates external entry, atomic conclusion owns state
meaning, and missing conclusions remain deliberately neutral. MFM keeps the strongest safety claim
without carrying a Runtime-wide durable response-observation lifecycle.
