# RFC: refactor to a single byte-ingress trust boundary

Status: accepted platform target; implementation pending the owner rulings listed below

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

The two execution paths are:

```text
Pure:
  selected occurrence
    -> deterministic implementation
    -> secret-free Pure PendingConclusion
    -> Store commits StateConcluded::Pure
    -> reducer advances

Read / Effect:
  selected occurrence
    -> StateImplementation.prepare(input)
    -> PreparedExecution { StatePrepared append, inert continuation }
    -> consuming Store commit
    -> direct-new branch only constructs CommittedCall
    -> StateImplementation.execute(CommittedCall)
    -> qualified adapter ingress
    -> state interprets accepted capability evidence
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

No MFM-generated successful output or definite provider result becomes public before the matching
`StateConcluded` is directly durable or found identical in qualified recorded history.

## Material uncertainties

The response-loss policy, separate admission, access-only preparation, `EntryOnce` parking,
bounded `EntryAbsorbing` recovery, exactly one semantic run record per append, catalog-qualified
domain evidence, explicit upstream States for provider-affecting facts, and one access mode per
capability type are settled. The following implementation gates remain.

1. **Session retention and cold-resume latency**

   - **Choice:** an executor retains one affine Runtime session across steps; a genuinely new resume
     folds the complete bounded prefix. There is no LRU, suffix protocol, or reducer checkpoint.
   - **Why uncertain:** the maximum supported cold-resume cost and caller retention policy have not
     been fixed.
   - **If wrong:** stateless one-action traffic may miss product latency targets.
   - **Resolution:** fix the maximum prefix, session-retention contract, and either a cold-resume
     SLO or explicit no-SLO ruling; benchmark the bound before implementation acceptance.

2. **Effect-attention inventory**

   - **Choice:** retain an append-atomic `needs_effect_attention` projection and bounded listing only
     if operators must discover parked Effects without knowing run ids.
   - **Why uncertain:** known-run recovery may be the only required product surface.
   - **If wrong:** MFM either carries an unused durable/public surface or cannot discover abandoned
     Effects proactively.
   - **Resolution:** choose enabled or absent before freezing PostgreSQL schema. Land or delete the
     column, partial index, DTO/API, bounds, docs, and tests as one slice.

3. **Production capability evidence and factual trust**

   - **Choice:** every capability owns one strict, secret-free evidence sum and states what adapter
     ingress proves. A configured provider remains part of the capability TCB unless the contract
     explicitly requires cryptographic, quorum, or independent evidence.
   - **Why uncertain:** current contracts sometimes use “verified” for both protocol validity and
     factual truth.
   - **If wrong:** authenticated but false provider data may be overstated, or an unavailable proof
     contract may be imposed accidentally.
   - **Resolution:** inventory every production capability's intent, response ingress,
     authentication, correlation, definite-pre-entry evidence, integrity evidence, factual-trust
     assumption, and convergence obligation.

4. **EVM sender/domain cutover**

   - **Choice:** derive submission identity from tenant, wallet nonce domain, and a non-secret
     idempotency key, under fresh run-store and wallet-domain identities and preferably a fresh
     sender.
   - **Why uncertain:** externally retained incomplete submissions and sender-reuse obligations have
     not been proven absent.
   - **If wrong:** a new identity domain can collide with or strand old nonce/effect progress.
   - **Resolution:** use a fresh sender or produce an auditable drain, terminality, pending-nonce,
     and exclusive-control proof before activation.

5. **Cold immutable binding evidence**

   - **Choice:** persist only immutable, secret-free descriptors needed for callback-free replay and
     exact process association; delete current-release and revocation lineage.
   - **Why uncertain:** some current certificate fields may encode a legitimate immutable restart or
     audit proposition despite currentness-oriented names.
   - **If wrong:** deletion can make retained history uninterpretable, while careless retention can
     recreate the rejected live-currentness system.
   - **Resolution:** map every certificate field and consumer to a surviving descriptor/ingress
     proposition or an explicit deletion rationale before the binding cutover.

6. **Dormant runs after binding replacement**

   - **Choice:** a retained Program resumes only under an assembly satisfying its exact immutable
     binding. A changed binding never silently replans or reinterprets it.
   - **Why uncertain:** the deployment policy for nonterminal dormant runs is not fixed.
   - **If wrong:** ordinary reconfiguration may strand valid runs or execute them under a binding
     they never fixed.
   - **Resolution:** choose drain-to-terminal, retain the old immutable assembly, or mark those runs
     read/replay-only. Any rebinding protocol requires a separate RFC.

7. **Total configuration-history bounds**

   - **Choice:** every configuration stream has maximum revision-count and cumulative canonical-byte
     bounds in addition to the per-revision bound.
   - **Why uncertain:** the concrete limits and worst-case load target are not fixed.
   - **If wrong:** one valid selected stream can monopolize memory and CPU.
   - **Resolution:** set both limits and enforce them identically in memory, PostgreSQL, import,
     audit, and tests.

8. **Erased typed intent/evidence representation**

   - **Choice:** Program owns an opaque canonical-bytes, exact-contract, erased-typed-value product
     that crosses Program, Store, and Runtime under one exact catalog instance.
   - **Why uncertain:** the final representation must cover hostile ingress, aggregate values,
     access evidence, direct downcast, and `Send` movement without a duplicate decode escape hatch.
   - **If wrong:** crate ownership or private erasure mechanics must change.
   - **Resolution:** prove the final representation and affine `CommittedCall` future in a disposable
     compile spike before the vertical execution cut.

9. **Capacity and attempt bounds**

   - **Choice:** one bounded complete frame is stored per append; every selected unresolved
     preparation reserves the full maximum legal conclusion. Program/capability contracts also
     bound total Read and absorbing attempts, including the initial attempt.
   - **Why uncertain:** concrete frame, conclusion, run-prefix, object, fact, projection, and attempt
     limits are not fixed.
   - **If wrong:** a legal conclusion may not fit after external entry, or valid workloads may be
     rejected.
   - **Resolution:** set and benchmark all bounds together. Reservation accounting must cover the
     aggregate liability of concurrent fan-out occurrences.

10. **PostgreSQL failure-domain claim**

    - **Choice:** the initial profile promises primary crash/restart durability, not primary-host
      loss.
    - **Why uncertain:** an out-of-tree deployment may advertise synchronous replica or quorum
      survival.
    - **If wrong:** Runtime could enter a provider after an acknowledgement weaker than the product
      claim.
    - **Resolution:** confirm the local profile or name and qualify the exact stronger synchronous
      topology before implementation.

Apart from these product, deployment, and numeric gates, no material architectural uncertainty
remains.

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
assembly and construct a new one. Explicitly forcing destruction of a `PendingConclusion` accepts
the documented response-loss window and leaves only the conservative durable state already
present.

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
durability profile.

`NewlyCommitted` means the exact transaction crossed that durability point. The minimum selected
profile survives a crash/restart of the admitted primary; Store qualification rejects weaker
effective settings. A stronger host-loss claim requires qualification of its named synchronous
topology. A weakening setting or writer-epoch change invalidates the Store rather than changing the
meaning of direct-new.

A legitimate restore rotates Store identity or epoch. After every independent later-head anchor is
lost, MFM cannot distinguish a self-consistent rollback from a Store that never advanced. An
actively lying database or administrator is outside this threat model, and a post-commit readback
against the same authority does not strengthen it.

## 3. Program, State, implementation, and assembly

### 3.1 One Program authority

There is exactly one executable semantic representation:

```text
ProgramDocument   bounded, strict, serializable data; never execution authority
Program           opaque, immutable, process-local callback-free authority
```

Typed DSL construction and hostile `ProgramDocument` ingress converge on one normalized-graph
invariant owner. Hot construction does not serialize and reparse its result. Cold ingress performs
byte-specific checks once. A `ProgramRef` is an address, not authority, and exact content equality
cannot substitute a value from another in-process catalog instance.

Operation expansion remains pure and visible in the final graph: child substitution,
configuration specialization, capability lowering, policy/failure wrapping, fan-out, and
normalization all occur before `Program` exists. Resume uses the persisted final graph and never
reruns expansion under current code or configuration.

`ProgramCatalog` owns callback-free declarations, codecs, expansion, graph construction, bounds,
and hostile ingress. `RuntimeAssembly` is constructed afterward from that exact catalog instance
and owns the immutable process-local implementation registry and private Runtime brand. Store and
Replay receive only callback-free Program/catalog authority.

### 3.2 Durable State and capability contracts

A durable State declaration remains Program data. It owns:

- input, output, failure, fact, and maximum-conclusion contracts;
- exactly one `Pure`, `Read<C>`, or `Effect<C>` classification;
- immutable State implementation identity and execution-binding identity;
- for access, the capability intent/evidence ABI, effect domain, entry discipline, fact mode, and
  recovery budget; and
- callback-free rules needed to verify replacement and absorption from durable history.

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

Provider-affecting prior facts must already be explicit in `S::Input`. Program expansion models
their acquisition as an earlier Read state whose output is lexically fed into this State. The
post-commit fact selection projected from intent may support interpretation only and cannot change
intent, target, request bytes, signer, binding, or effect domain.

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
  | BlockedIntegrity {
        accepted capability-specific integrity evidence,
        stable redacted code,
    }
  | Unresolved {
        call-bound prepared successor,
        redacted operational classification,
    }
```

`Concluded` and `BlockedIntegrity` are constructed only through the exact call-correlation owner.
The private `Unresolved` wrapper retains the exact active successor and call correlation but no
invoker or external-I/O method; its classification alone is not an owner and cannot be supplied by
state code. It is process-local, is never persisted, and authorizes no retry by itself. Provider
success, returned rejection, accepted safe failure, and accepted integrity blocking are definite
and must enter the synchronous conclusion handoff. Cancellation, caught panic, malformed or unbound
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

`RunAdmitted` fixes the run id and tenant, final Program, root input, context, configuration,
source dependencies, Store/catalog identities, and every admission bound. It is always the first
and only genesis record. Admission is never batched with the first state.

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

The exact occurrence's Program declaration supplies the immutable input, access mode, capability,
entry discipline, evidence contract, effect domain, and attempt budget. Store supplies coordinates,
ordinal, append identity, and the assigned typed `StatePreparationRef`; state code constructs none
of them.

`StateConcluded` is a strict outer sum with a nested Access payload:

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
    conclusion,
}

AccessConclusion =
    Outcome {
        accepted_capability_evidence,
        outcome,
        facts_and_outputs,
    }
  | BlockedIntegrity {
        accepted_capability_integrity_evidence,
        stable_redacted_code,
    }
}
```

`BlockedIntegrity` has explicit reducer semantics: it concludes the occurrence as an integrity
failure, publishes no domain output or facts, grants no retry authority, and participates in
terminal outcome derivation through the Program's fixed failure routing.

`accepted_fact_selection` is the callback-free selected response plus completeness attestation
bound to the exact preparation request and frontier. It is absent for capabilities without prior
facts and is distinct from the conclusion-time fact-publication coordinate.

Its logical key is the state occurrence, never the preparation. This is what enforces at most one
conclusion across retry attempts. Fact objects and publication routes share the conclusion append.

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
  -> no further record for that occurrence
```

A replacement names exactly the selected unresolved parent and increments the Store-assigned
ordinal without gaps. Branches, cycles, forward references, returning to an older selection, and
replacement after conclusion are invalid.

Read replacement preserves the exact occurrence, immutable input, capability, canonical intent,
execution binding, and effect domain, and every replacement consumes the Program-declared total-
attempt budget including the initial attempt. A capability-owned freshness coordinate, such as a
later fact frontier, may change only when it is explicitly separate from provider semantics. If
selected facts can change the provider operation, the selection must already be fixed in State
input or be mechanically projected from Program/input before `prepare`; it becomes part of the
canonical intent and `StatePrepared`. The post-commit fact continuation is interpretation-only.

`EntryOnce` forbids replacement. `EntryAbsorbing` replacement additionally preserves the exact
absorption identity and remains within the total-entry budget, including the initial attempt.

Conclusion classification precedence is mode-aware:

1. a Pure candidate with the same occurrence, outcome, and canonical object/fact closure; an Access
   `Outcome` with the same occurrence, preparation ref, optional accepted fact selection,
   capability evidence, outcome, and canonical closure; or an Access `BlockedIntegrity` with the
   same occurrence, preparation ref, optional accepted fact selection, integrity evidence, and
   stable code is `AlreadyConcludedSame`;
2. an Access candidate naming a preparation provably superseded in that occurrence is
   `NoLongerSelected`;
3. an already-durable different conclusion for the Pure occurrence or the same selected Access
   preparation is `Conflict`;
4. a retained Pure candidate with no durable conclusion whose occurrence is no longer ready is
   `NoLongerReady`; and
5. a foreign, absent, cross-run, or cross-occurrence preparation reference is invalid correlation
   or invalid history.

Byte-equal outcomes from different preparations are not duplicate conclusions.

### 5.3 Terminality and capacity

Terminality is derived after every semantic record. A root cannot become terminal while any
reachable occurrence has a selected unresolved preparation. Once terminal, every further
`StatePrepared` or `StateConcluded` is illegal; cold ingress rejects a trailing record. A database
`closed` field may remain only as an append-atomic checked projection.

A zero-state admission retains every newly reachable root-result object and updates the terminal
projection in the same transaction as `RunAdmitted`.

Every selected unresolved preparation reserves the complete maximum legal conclusion against run
bytes, object counts and bytes, evidence/outcome variants, facts, dense publication rows, indexes,
accepted prior-fact response/attestation when applicable, projections, batch totals, and database
parameter/frame limits. Concurrent fan-out occurrences
reserve their aggregate liability. Replacement atomically transfers the selected predecessor's
logical reservation; conclusion consumes it; a superseded attempt has no independent legal-
conclusion reserve. No conclusion may exceed its reservation.

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

Admission consumes one owner-bound typed Program/root/input/context/configuration/source product and
appends `RunAdmitted` alone against an absent predecessor. Direct-new returns the exact admitted
session or reducer-derived zero-state terminal result. Found-identical promotes no local successor
and resumes only from qualified recorded history; found-different conflicts.

An unknown acknowledgement retains the exact admission append and owner. Resolution serializes on
the same physical identity: found-identical qualifies recorded history; proven absence with the
genesis precondition still current may resubmit, and only that resubmission's direct-new branch
advances; repeated unknown retains the quarantine. No admission branch invokes State, adapter, or
provider code. This is why admission stays separate from the first State.

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

- exact catalog instance, Runtime assembly brand, Store identity/epoch, and durability profile;
- tenant, run, Program, occurrence/path, immutable input, access mode, and capability;
- canonical intent, State implementation, qualified adapter, execution binding, provider/target/
  signer identities, and effect domain;
- assigned preparation ref, ordinal, committed head, and replacement relation;
- one process-local call-correlation identity and Store-minted conclusion correlation; and
- when declared, the exact one-use prior-run fact continuation.

There is no independently usable `RunSession` beside a live `CommittedCall` or its pending
conclusion. The active committed successor moves through the call-correlation owner and returns to a
session only after unresolved execution or conclusion disposition.

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
preparation/occurrence correlation, object closure, capacity discharge, and physical-append owner.
Runtime combines it with the inert session continuation into affine `PendingConclusion`; there is
no independently usable session beside it. Store never owns Runtime brand/dispatch state or hot
accepted evidence. `PendingConclusion` is secret-free, non-Serde, and contains no adapter, raw
response, invoker, implementation callback, scanner, or means to perform provider/fact/state-
callback I/O. Runtime's coordinator may use its inner owner only for the required Store
conclusion-commit and acknowledgement-recovery I/O.

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
NoLongerReady(qualified latest history)     // Pure only
Conflict(qualified latest history, typed conflict)
AcknowledgementUnknown(PendingConclusion)
```

Retryable Store failures and permanent Store, epoch, durability, capacity, or integrity failures
travel on a separate typed error channel. Every error returns the same `PendingConclusion`; only an
explicit supervisor force-destruction may discard it under the documented response-loss policy.
There is no undefined semantic conclusion sink.

For a stale run head or conclusion-time fact-publication frontier, Runtime consumes the outer owner,
moves its inner `PreparedConclusion` through Store, and receives a new inner owner or qualified
history before reconstructing the outer result. Common Store/epoch/tenant/run/Program,
nonterminality, fact-dependency, and exact-capacity checks must pass. Access additionally requires
its exact preparation to remain selected and unresolved; Pure requires its exact occurrence to
remain ready. Only then may Store bind a new physical append. No branch re-executes State logic,
the adapter, response ingress, interpretation, or a fact scan.

An ambiguous conclusion acknowledgement first resolves its exact physical append identity:

- found same: resume from qualified recorded history; do not republish facts;
- retained physical append proven absent but the same canonical conclusion already recorded under
  another physical append id: `AlreadyConcludedSame`; do not republish facts;
- proven absent with the same Access selection or Pure ready occurrence: retry or rebind the same
  conclusion;
- replacement selected for Access: `NoLongerSelected`;
- different durable conclusion: `Conflict`;
- Pure occurrence no longer ready with no durable conclusion: `NoLongerReady`;
- repeated ambiguity or retryable failure: retain the same `PendingConclusion`; and
- explicit owner loss: cold history contains either the durable conclusion or the prior reducer
  state--a ready Pure occurrence or the selected Access preparation.

### 6.5 Concurrency linearization

The required races linearize as follows:

- preparation versus preparation: one direct-new winner may invoke; a loser invokes zero times and
  can create a later replacement only through reducer recovery rules;
- conclusion versus another conclusion for the same selected preparation: one append wins; the
  other becomes identical or conflict;
- conclusion versus replacement: a conclusion winner closes the occurrence; a replacement winner
  supersedes the old preparation and makes its result `NoLongerSelected`;
- conclusion versus unrelated run-head or fact-publication movement: rebind only after proving the
  same Access selection or Pure-ready occurrence;
- terminal-producing conclusion versus any stale candidate: the terminal winner commits and the
  stale candidate cannot rebind; and
- two in-flight same-capability calls cannot transpose Runtime shells, preparation refs, adapter
  results, fact continuations, or conclusion owners.

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
3. absorption lasts for the entire overlap and recovery horizon; and
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
updates terminal and optional attention projections. Rollback is all-or-nothing.

### 8.2 Qualified history and Runtime sessions

`QualifiedRun@H` is opaque, callback-free semantic evidence for one exact prefix.
`ActiveQualifiedRun@H` adds the private Store coordinator. `RunSession@H` adds the exact Runtime
assembly brand and is affine, non-serializable, and non-cloneable.

Purpose readers receive only callback-free evidence. Multiple workers may resume the same head and
race; exact-head append selects one successor. Direct admission and conclusion commits advance the
retained session without reload. A direct-new preparation commit transfers its successor into
`CommittedCall` and then `PendingConclusion`; there is no parallel session.

There is no shared semantic LRU, suffix-refresh protocol, cloneable session, or durable reducer
checkpoint. A fresh resume performs one bounded complete-prefix ingress and fold. Session and
ingress work use explicit bounded resource permits; expensive decode/reduction and deterministic
state work do not block the async runtime.

### 8.3 Demand-time qualification and replay

Ordinary Store open qualifies schema, Store identity/epoch, backend channel and durability, writer
role, exact Program catalog, and bounded transaction ability. It does not know Runtime assembly or
enumerate retained run/configuration history. Runtime composition separately validates the exact
catalog-to-assembly association before any session can exist.

A retained run crosses ingress only when resume, read, trace, replay, export, explicit audit, or a
selected fact dependency consumes it. Malformed dormant history fails that operation without
poisoning readiness or unrelated runs. `audit_store` is a separate read-only, fixed-snapshot
diagnostic and creates no execution authority.

Live, recorded, and portable replay all use the same Store qualifier and reducer. Replay cannot
construct a `CommittedCall`, invoke a State implementation, scan a live Store not present in its
closure, contact a provider, or append. Every valid prefix—including an unresolved preparation and
a terminal prefix—replays with zero callbacks and zero live I/O.

### 8.4 Configuration history

Configuration is a separate append-only stream. External sources and retained rows cross one typed
ingress into `ResolvedConfiguration<T>@Head`; locally typed values are canonicalized once without
hostile reparse. Planning consumes the typed value directly.

Configuration writers use their own coherent `PreparedConfigurationAppend`, exact-head commit,
direct-successor promotion, idempotency, and acknowledgement recovery. The three-family run rule
does not rename configuration revisions or collapse the two domains into a generic history API.
Secret-bearing configuration is consumed into live adapter internals and never persists.

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
Delete the authenticated-issuer/policy namespace and activate the changed domain only under the
fresh Store/wallet/sender gate in Material uncertainties.

Concrete adapters own transports, signers, keystore actors, protocol credentials, wallet/nonce
transactions, and permanent operation keys. Delete generic live binding refresh, release lineage,
revocation, and callable nested resource handles only after every immutable/protocol proposition
has a final owner. Preserve genuine provider protocol authentication, Store epochs, SQL locks,
nonce reservations, transaction permits, and secret controls.

## 10. API and deletion cutover

### 10.1 Program and package ownership

Retain one opaque `Program`, one serializable `ProgramDocument`, one callback-free catalog, one
Runtime assembly, one Store reducer, and one replay qualifier.

Move the normalized compiler/document work into `mfm-program`, live implementation assembly into
`mfm-runtime`, and callback-free retained semantics into `mfm-store`. Delete superseded
`mfm-spec`, `mfm-certify`, and empty authority-seal packaging after their real obligations have
moved. Final dependency direction is Runtime to Store; Store never depends on Runtime.

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
- aggregate maximum-conclusion capacity;
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
- old five-family types, packages, feature bridges, aliases, and decoder surfaces are absent.

### 12.2 Journal and reducer

Golden and hostile tests cover:

- exactly the three record families and three logical keys; old tags and fields reject;
- separate admission, one-record batches, zero-state terminality, and no `RunClosed`;
- every Program/input/intent/binding/replacement/evidence/outcome/fact/object/capacity mutation;
- linear replacement ordinals, exact parent selection, no branch/cycle/gap, and mode-specific
  replacement rules;
- one conclusion per occurrence across preparations, exact duplicate idempotency,
  Access `NoLongerSelected`, Pure `NoLongerReady`, conflicts, and invalid
  correlations;
- aggregate capacity reservation, transfer, discharge, exact bound, and bound-plus-one; and
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
- ambiguity resolution covers found-same/different/invalid, proven-absent direct-new, stale/terminal/
  other-preparation-selected, repeated unknown, explicit drop, and cold restart;
- two same-head workers yield one committed-call owner;
- affine execution owners, accepted wrappers, fact continuations, and pending conclusions cannot be
  serialized, deserialized, copied, defaulted, cloned, or consumed twice;
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
- raw responses, provider diagnostics, and wrong request/route/target/signer/binding never reach
  state interpretation or persistence; and
- accepted evidence, interpretation, conclusion canonicalization, and public result each occur at
  most once.

Conclusion recovery tests cover direct commit, identical existing conclusion, different conclusion,
late result, replacement race, stale-head rebind, fact-publication movement, acknowledgement found/
invalid-or-over-limit/absent/alternate-physical-same/repeated-unknown, explicit owner loss,
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
- `EntryAbsorbing` exact intent/binding/domain/identity preservation, total-entry budget, slow-old-
  call race, full retention horizon, and convergent post-state evidence;
- direct-new-only, one-use, non-transposable prior-fact continuation; fixed preparation selection
  frontier; dense negative completeness; and atomic conclusion/fact publication;
- explicit upstream Read/lexical-input modeling for provider-affecting facts and rejection of any
  hidden pre-prepare or post-commit intent-changing fact path;
- duplicate conclusion does not republish facts and failed/integrity conclusions publish none;
- every replay prefix performs zero State, adapter, scanner, provider, or other live callbacks;
- malformed dormant history is isolated to its consumer; and
- secret canaries never appear in Program/configuration, `RunAdmitted`, admission context/source
  manifests, any canonical journal frame, preparation, evidence, pending/concluded values, newly
  reachable object/output closure, facts, portable export, public DTOs, errors, traces, or debug
  formatting.

### 12.6 PostgreSQL, application, and performance

Memory/PostgreSQL conformance and PostgreSQL-specific tests cover exact-head contention, transaction
rollback at every conclusion/publication statement, acknowledgement ambiguity, primary crash/
restart durability before provider entry, weak durability rejection, writer-epoch loss, schema and
SQL inventory, tenant-first lookup, hostile rows, derived terminal projection, and old-schema
rejection.

Application/transport tests retain fixed-tenant isolation, credential/policy deletion, strict
portable closure, EVM identity, redacted errors, and non-interactive behavior.

Benchmarks measure maximum Program ingress, frame construction/ingress, cold run resume, retained
session steps, fact closure, configuration history, maximum conclusion, and one-shot transport
resume. The hot direct path performs zero historical reads/folds after initial resume.

## 13. Logical implementation sequence

Each commit includes its contracts, tests, schemas, fixtures, and documentation. No old/new public
API pair, decoder, reducer, or fallback survives a cutover commit.

0. **`resolve prepared-call execution gates`**

   Record the remaining Material uncertainties, numeric bounds, evidence/absorption audit, binding
   disposition, deployment activation, and compile/benchmark spike results. Delete unchosen
   branches from the plan.

1. **`scope application facades by tenant`**

   Cut App, CLI, REST, and EVM identity to fixed-tenant, credential-free structural contracts.
   Delete policy persistence and old identities atomically; disable pre-cutover portable export/
   import until final structural v5 lands with three-family frames in Commit 4.

2. **`make program and durable state semantics valid by construction`**

   Land opaque Program/value/catalog/compiler authority and callback-free State declarations. The
   Commit-0 U5/U8 ruling fixes the exact immutable descriptor subset retained here; final execution-
   specific Intent/Evidence declarations land in Commit 4. Add no compatibility adapter or second
   binding owner.

3. **`make store ingress and appends one semantic path`**

   Invert Store/Runtime dependencies and land the callback-free Store ownership, one reducer/binder
   path over the unchanged pre-cutover backend/schema, typed Facts response ownership,
   configuration reads, and planning prerequisites. Retain only the one coherent current access
   protocol. The complete-frame SPI, attention schema, and final PostgreSQL v8 all land in Commit 4.

4. **`execute committed preparations and conclude states atomically`**

   One inseparable vertical cut across Program/Capabilities, Journal, Store, Runtime,
   State/adapter registration, facts, projections, replay/export v5, complete-frame Store SPI,
   Memory/PostgreSQL v8 and attention, App, EVM/live implementations, schemas, goldens, tests, and
   documentation. Introduce the three families, `PreparedExecution`, `CommittedCall`, capability
   evidence, and `PendingConclusion`; delete the complete five-family protocol and compatibility
   paths in this commit.

5. **`qualify retained history only when consumed`**

   Remove eager readiness scans, unify live/read/replay/export/audit/fact consumers behind the one
   demand qualifier, and retain only scope-local prefix reuse.

6. **`make configuration writes one semantic path`**

   Complete the independent affine configuration writer cut with direct successor promotion and no
   reload, echo, cache, or compatibility reader.

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
- one secret-free `PendingConclusion` owns all conclusion retries, rebases, fact-frontier movement,
  and acknowledgement ambiguity without provider/fact/state-callback I/O or repeated
  interpretation; only conclusion Store I/O remains;
- loss of an Access owner before conclusion durability yields only `StatePrepared`; loss of a Pure
  owner yields the prior ready/admitted prefix; no successful result escapes before conclusion
  durability;
- Store admits at most one conclusion per occurrence across all preparations and correctly
  distinguishes identical, superseded, conflicting, and invalid results;
- Read and absorbing replacement preserve their exact contracts and budgets; `EntryOnce` parks;
- every absorbing capability demonstrates actual absorption, full-horizon retention, stable-key
  enforcement, and convergent post-state evidence;
- every selected preparation reserves one maximum conclusion and concurrent reservations cannot
  overbook run or backend capacity;
- facts and outputs publish atomically with conclusion, duplicate conclusions never republish, and
  replay recomputes them callback-free;
- `QualifiedRun` remains callback-free, `RunSession` remains affine/Runtime-branded, and no parallel
  session exists beside a live call or pending conclusion;
- Store readiness opens no dormant history, while explicit consumers reject malformed selected
  history before returning authority or output;
- PostgreSQL qualifies the admitted durability profile and writer epoch before any direct-new call
  can exist, and legitimate restore rotates identity/epoch;
- fixed-tenant App, credential/policy deletion, strict portable export, EVM identity, configuration,
  redaction, and secret contracts remain satisfied;
- every observation/reservation/transition/closure-era type, tag, logical key, projection, SQL
  assumption, fixture, alias, feature escape hatch, and fallback is absent; and
- final documentation describes only this design and the final report lists all verification,
  public/dependency/LOC deltas, deployment gates, and known limitations.

When these conditions hold, durable preparation gates external entry, atomic conclusion owns state
meaning, and missing conclusions remain deliberately neutral. MFM keeps the strongest safety claim
without carrying a Runtime-wide durable response-observation lifecycle.
