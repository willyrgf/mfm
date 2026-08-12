# Implementation plan: refactor MFM to a single byte-ingress trust boundary

Status: implementation handoff; blocked only by the owner rulings in
[Material uncertainties and preflight gates](#material-uncertainties-and-preflight-gates)

Normative architecture:
[`RFC_REFACTOR_SINGLE_TRUST_BOUNDARY.md`](RFC_REFACTOR_SINGLE_TRUST_BOUNDARY.md)

Audience: engineer-agent implementing the complete MFM platform cutover

---

## 0. Authority, mandate, and completion rule

The RFC is the accepted platform architecture. Current code, tests, migrations, fixtures, READMEs,
`docs/design.md`, and `docs/architecture.md` describe the implementation being replaced where they
conflict with it. This plan fixes package ownership, sensitive Rust boundaries, deletion order,
verification, and logical commits. If implementation discovers a conflict with the RFC, stop,
amend both documents deliberately, and obtain architect review before continuing.

The engineer may break and rewrite Rust APIs, schemas, migrations, fixtures, tasks, and persisted
formats. Old bytes have no compatibility claim. Specifically:

- the five-family run protocol is a deletion target, not a compatibility source;
- FanOut and every lane/join/parallel workflow surface are deletion targets and have no Collect,
  Gather, barrier, `state_many`, output-map, or other multi-result replacement;
- `mfm-spec`, `mfm-certify`, obsolete authority-seal bridges, application access control, and
  generic live-currentness are deletion targets after their real obligations move;
- no alias, old decoder, dual write, fallback, optional legacy branch, or feature-gated authority
  constructor may survive a cutover commit; and
- existing LOC and public-type boundaries have no preservation value by themselves.

The refactor must retain exact-prefix hashing, append atomicity, exact-head comparison, writer
epoch and admitted durability, content addressing, dense fact completeness, direct-new-only live
call authority, occurrence-level conclusion uniqueness, secret exclusion, adapter protocol
authentication, EVM nonce/effect safety, and callback-free replay.

Implementation is complete only when:

1. every RFC and plan acceptance criterion passes;
2. the run journal has exactly `RunAdmitted`, `StatePrepared`, and `StateConcluded`;
3. every five-family type, tag, projection, SQL assumption, fixture, alias, and decoder is absent;
4. the final Program declaration algebra is exactly State/Match, every run advances through one
   sequential cumulative context from exactly one domain-planned `C0`, and every plural-root,
   ordered child-input, FanOut/lane/join/Collect workflow identity is absent;
5. the authoritative docs describe only the new design;
6. the remaining owner rulings are recorded and unchosen branches deleted;
7. each logical commit is one coherent current design with tests and docs; and
8. the final report includes public API, dependency, product-LOC, persisted-identity, and
   verification deltas.

The implementation commits form one non-deployable migration train. Each checkout must compile and
run only against its matching ephemeral fresh identity, but no intermediate schema/domain identity
is activated in a retained deployment. Production activates only after the final commit and gates.

## 1. First-principles target and package graph

The final ownership rules are:

1. **Program owns callback-free executable semantics.** `mfm-program` owns normalized Program data,
   the State/Match-only sequential control form, durable State/capability and cumulative-context
   contracts, value codecs, pure child expansion, and hostile Program ingress.
2. **Store owns durable semantic evidence.** `mfm-store` owns run/configuration byte ingress, the
   sole sequential cursor/reducer/binder, exact cumulative-context continuity, the sole selected
   preparation, singular capacity accounting, exact-head commits, `QualifiedRun`, the secret-free
   `PreparedConclusion` append owner, facts, and callback-free projections.
3. **Runtime owns live execution authority.** `mfm-runtime` owns immutable implementation assembly,
   private erasure, affine worker sessions retaining the latest typed context,
   `PreparedExecution`, direct-new construction of `CommittedCall`, call correlation, the outer
   affine `PendingConclusion`, and execution containment. It owns no scheduler, lane set, per-run
   mutex, or history API for State code.
4. **Adapters own provider entry and ingress.** A qualified adapter consumes a committed call,
   derives provider bytes from its canonical intent, performs the concrete operation, and returns
   only bounded call-bound capability evidence or an unresolved classification.
5. **Application owns a fixed-tenant facade, not caller access control.** Trusted embedding selects
   one facade per tenant.
6. **PostgreSQL owns durable physical ordering within one admitted Store epoch.** It returns
   truthful mechanical dispositions and does not rerun Program/reducer semantics.
7. **Domains own cumulative context semantics.** Each operation defines its bounded, secret-free
   admitted `C0`; operations and reusable fragments define their nominal fragment-entry/successor
   values and consolidations. State implementations alone decide what interpreted domain data to
   retain; no kernel-owned heterogeneous context exists.

The target graph is:

```text
mfm-values + mfm-ids
        |
        v
mfm-capabilities       intent/evidence, fact mode, entry/absorption contracts
        |
        v
mfm-program            DSL, pure compiler, State declarations, ProgramDocument,
        |               opaque Program, ProgramCatalog, binding descriptors
        |
        +--------------------------+
        |                          |
        v                          v
mfm-journal                    mfm-store
three strict records           ingress, reducer/binder, QualifiedRun,
and hash chain                 selected preparation, PreparedConclusion,
                               facts/configuration/projections/backend SPI
                                      |
                                      v
                                mfm-runtime
                                RuntimeAssembly, StateImplementation,
                                RunSession, PreparedExecution,
                                CommittedCall, call containment
                                      |
                                      v
                                  mfm-app
                                  fixed-tenant facade
```

Storage implementations depend on Store's mechanical SPI. Replay depends on Program and Store,
never Runtime. Domain crates depend on Program/Capabilities. Live crates depend on Runtime only for
implementation/adapter registration. Binaries consume App.

Required dependency changes:

- remove `mfm-store -> mfm-runtime` and add `mfm-runtime -> mfm-store`;
- make Store depend directly on Program/Capabilities for callback-free contracts;
- move normalized document/compiler work from `mfm-spec`/`mfm-certify` into Program;
- move live registrations and erasure from Certify into Runtime;
- move callback-free cursor, reducer, and history-port responsibilities into Store;
- replace real authority-seal uses with opaque owner products/private constructors, then delete the
  package and feature bridges; and
- remove every direct/dev/Nix task edge to the deleted packages.

No new generic authority crate replaces them.

The final hot paths are:

```text
admission:
  one domain-planned typed C0 -> RunAdmitted append -> worker session or terminal zero-state result

Pure:
  sole selected occurrence -> exact predecessor Cn -> Runtime invokes evaluate(Cn)
  -> ProposedStateOutcome -> Store Pure-scoped PreparedConclusion
  -> Runtime Pure-scoped PendingConclusion
  -> exact-head conclusion commit containing complete Cn+1 or exact root result -> session/terminal

Read / Effect:
  sole selected occurrence -> exact predecessor Cn
    -> Runtime invokes prepare(Cn) -> canonical intent
  -> PreparedExecution
  -> direct-new StatePrepared commit -> CommittedCall
  -> Runtime invokes StateImplementation.execute(CommittedCall)
       -> bound adapter entry and qualified ingress
       -> state evidence interpretation
       -> conclusive AccessHandlerResolution
  -> Store Access-scoped PreparedConclusion -> Runtime Access-scoped PendingConclusion
  -> exact-head selected conclusion commit containing complete Cn+1 or exact root result
  -> session/terminal
```

After the one required canonical frame encoding, no direct branch performs a local serialize-to-
decode round trip, full-prefix reload, second reducer, semantic comparison, or positive frame echo.

The visible Program declaration algebra is only `State | Match`. Child operations/fragments are
authoring abstractions eliminated by pure expansion. Each nonterminal success replaces the current
typed `Value<Cn>` with the exact complete `Value<Cn+1>`; Match selects one exhaustive arm and every
continuing arm returns the exact common context contract. Failure takes its declared route
immediately. Every operation has exactly one domain-owned typed admission value `C0`, constructed
by domain/application planning. Runtime and Store qualify, persist, and transport that value but do
not assemble or interpret it. `C0` names the admitted run root. A nested child consumes one
singular fragment-entry `Value<Cn>` and returns one successor after expansion; expansion substitutes
that exact nominal value unchanged. If the child entry contract differs, an explicit caller-owned
State constructs the wrapper before the child call.

For one durable run prefix there is at most one actionable occurrence and selected preparation.
Each affine worker owner contains at most one `CommittedCall` or `PendingConclusion`; competing
workers may race, and Store CAS selects the durable successor. Every successful nonterminal State
returns the complete next context. A terminal Pure, Read, or Effect State may return the exact
declared root result, which Runtime exposes only after its conclusion is durable and qualified.
The zero-state exception means the expanded sequential control form is empty. Certification
requires the root contract/value to be the exact admitted `C0`, and reducing `RunAdmitted` returns
that same content identity without a callback. Every nonempty successful path, including a selected
Match arm, ends in a terminal State. Match-only or selected empty-arm terminal paths reject. A
different admission projection requires an explicit Pure terminal State.

## Material uncertainties and preflight gates

These are implementation gates. Record each ruling in the RFC and this plan before the first code
cut depending on it, and delete the unchosen branch.

Already settled: one semantic run record per append; domain-owned `MfmValue` evidence qualified by
the catalog; provider-affecting facts represented by an explicit upstream State and `S::Input`; one
sealed Read-or-Effect mode per nominal capability type; State/Match-only structured control;
sequential fail-fast execution; singular domain-planned `C0`; domain-owned complete cumulative
contexts; CAS-linearized worker races without a Runtime process lease; mode-neutral terminal roots;
and no FanOut/Collect replacement.

### U1. Session retention, cumulative-context hot advancement, and cold-resume performance

- **Target:** retain the exact latest `QualifiedTypedValue<Cn>` inside the affine session. Hot
  conclusion advancement performs its one required canonical conclusion encoding, then hands the
  already-typed `Cn` to the next State with no serialize/decode round trip or prefix fold. Cold
  resume qualifies/folds the complete bounded prefix, validates every context link, and binds the
  latest concluded context. There is no LRU, suffix protocol, context checkpoint, structural
  sharing, or compaction.
- **Why/consequence:** maximum context size, caller retention, and cold-resume SLO are unknown; a
  wrong bound makes valid stateless traffic or accepted `O(n^2)` retention miss latency/memory
  targets.
- **Gate:** fix maximum context and prefix frames/bytes/objects, active-session retention, and
  either a cold-resume SLO or explicit no-SLO. Benchmark maximum hot advancement and full cold
  resume at U9's worst case in the Nix shell.
- **Failure action:** fail Commit 0 and raise bounded resources, relax the SLO, or reduce admitted
  workload. Checkpointing, compaction, sharing, or collection optimization requires a future RFC.

### U2. Effect-attention inventory

- **Target:** keep the append-atomic projection/listing only if operators need tenant-wide discovery
  without a run id.
- **Why/consequence:** the discovery product requirement is unconfirmed; the wrong choice either
  carries an unused schema/API surface or makes parked Effects undiscoverable by inventory.
- **Gate:** choose enabled or absent. Enabled requires one fixed-snapshot bounded listing, exact
  column/partial index, DTO/API, and qualification-before-action. Absent deletes that complete
  slice.

### U3. Production capability evidence and factual trust

- **Target:** every Read/Effect capability owns one strict `Intent`, one strict closed `Evidence`
  sum, request/call binding, entry classification, and factual-trust statement.
- **Why/consequence:** production providers prove different facts; an incomplete contract can
  overstate provider truth, convergence, or definite pre-entry authority.
- **Gate:** check in a table covering every production capability: provider owner, intent, raw
  ingress, authentication, request/call correlation, returned/rejection/safe-failure variants,
  definite-pre-entry evidence, integrity evidence, factual-trust assumption, fact mode, and public
  evidence type, plus evidence-to-reviewed-domain-context projection and raw-evidence/secret
  retention disposition.
- **Absorbing gate:** additionally prove stable-key enforcement, actual absorption, full recovery-
  horizon retention, and post-state-convergent evidence.

### U4. EVM identity and sender activation

- **Target:** submission identity is tenant + wallet nonce domain + non-secret idempotency key;
  activation uses fresh Store/wallet identities and preferably a fresh sender.
- **Why/consequence:** retained old-process nonce/effect state has not been disproved; wrong reuse
  can collide with or strand external submissions.
- **Gate:** use a fresh sender or produce an auditable old-process drain, terminal allocation/
  Effect inventory, pending-nonce reconciliation, and exclusive-control artifact.

### U5. Immutable binding evidence

- **Target:** retain only immutable, secret-free binding/ingress propositions; delete live release,
  promotion, generation, and revocation lineage.
- **Why/consequence:** current certificate fields mix immutable evidence with freshness; deleting
  the wrong field can make history uninterpretable, while retaining it recreates live currentness.
- **Gate:** map every certificate field and consumer to one final descriptor/ingress owner or an
  explicit deletion rationale. Separate provider protocol authentication from currentness.
- **Signer gate:** determine immutable key-instance identity from public signer metadata rather than
  renaming a freshness field.

### U6. Dormant runs after binding replacement

- **Target:** retained Programs run only under their exact immutable binding.
- **Why/consequence:** deployment treatment of nonterminal dormant runs is not chosen; silent
  rebinding changes admitted semantics, while dropping old assemblies may strand runs.
- **Gate:** deployment chooses drain-to-terminal, retain the old assembly, or read/replay-only.
  Rebinding/migration is a separate RFC.

### U7. Configuration-history bounds

- **Target:** maximum revisions and cumulative canonical bytes in addition to per-revision bounds.
- **Why/consequence:** concrete limits and load targets are unknown; weak limits permit one valid
  stream to monopolize memory/CPU, while low limits reject intended workloads.
- **Gate:** fix both limits and worst-case load time; enforce identically in Memory, PostgreSQL,
  import, audit, selection, and write preparation.

### U8. Erased typed intent/evidence and affine futures

- **Target:** Program-owned canonical bytes + exact contract + erased typed object under one catalog
  brand; Runtime-private `Send` affine call/result erasure.
- **Why/consequence:** the final Rust representation is not compiled yet; a failed shape changes
  private crate mechanics and could tempt an unsafe public downcast or duplicate decode path.
- **Gate:** a disposable compile spike must cover hot typed `Cn -> Cn+1` handoff without a Serde
  round trip (while canonically encoding the conclusion once), cold latest-context binding, Match-
  arm variant qualification/convergence, cumulative-context transposition/foreign insertion, the
  EVM/Portfolio opaque typed continuation, consuming input ownership through Access/failure paths,
  Store retention, Runtime downcast, provider evidence, `CommittedCall`, prior-fact substates,
  cancellation, and `Send`. Expose no unchecked public downcast or free constructor.

### U9. Cumulative-context, frame, conclusion, run, source/occurrence, and attempt bounds

- **Target:** one canonical complete frame per append; the sole selected unresolved preparation owns
  one maximum-conclusion reservation. Retaining complete `C1..Cn` may cost `O(n^2)` canonical bytes
  and is accepted. The deleted FanOut limit `4,096` supplies no source-count rationale.
- **Why/consequence:** numeric workload/backend limits are unknown; under-reservation can strand an
  entered Effect, while low bounds reject legal cumulative workloads.
- **Gate:** fix `C0`/maximum `Cn`, source/collection/occurrence counts, context/frame/conclusion/run
  bytes, objects/facts/projections/database parameters, Read attempts, and absorbing total entries.
  `MAX_TOTAL_ENTRIES` includes the initial attempt and must be nonzero.
- **Benchmark:** initial/maximum context construction, every cumulative-context append, maximum hot
  advancement, maximum cold resume, every exact bound, and each independent +1. Backend overflow
  may only enlarge a bounded contract or reduce workload; never introduce FanOut, Collect,
  structural sharing, or checkpoints.

### U10. PostgreSQL durability topology

- **Target:** `PrimaryCrashRestart` unless the deployment names a stronger synchronous topology. A
  writer epoch is an immutable persisted Store/run identity and append precondition, not a live
  lock. Multiple qualified workers/processes may use the same Store scope and epoch; exact-head
  compare-and-append, direct-new call construction, and occurrence-level conclusion uniqueness
  linearize their commands. Scope/epoch rotation remains restore/new identity and follows U6; it
  does not append to the old run.
- **Why/consequence:** deployments may advertise host-loss survival; entering after a weaker
  acknowledgement would violate that product durability claim.
- **Gate:** confirm the durability claim or qualify the exact stronger synchronous topology, then
  test its actual failure domain and same-epoch multi-process CAS behavior. Cross-process exclusive
  execution is outside this RFC and requires a separate design if ever needed.

### U11. Concrete cumulative-context ABI

- **Target:** each operation owns one nominal admitted `C0`, and each domain owns the operation's
  later context/final-result types. Every successful nonterminal State returns the complete
  successor context. Every operation has one singular domain-planned `C0` contract/value across
  authored, expanded, certified, entry, admission, and journal forms;
  child expansion substitutes one current context rather than an ordered input bijection. Match
  qualifies one closed-sum variant payload as the selected arm's exact complete input, every
  continuing arm returns the exact continuation contract, recovery receives prior context only
  through an explicit domain failure value, and EVM carries an opaque typed caller continuation
  unchanged. `MfmValue` does not imply `Clone`: the
  spike must choose a private consuming input-owner handoff through Pure and ordinary Access
  Outcome/State-failure paths, or deliberately add and bound an explicit cloning contract;
  callback-free integrity blocking needs no such handoff.
- **Why/consequence:** exact production State-by-State schemas and the generic/monomorphized EVM-to-
  Portfolio boundary are not compiled; a wrong choice could create an untyped map, lose early data,
  let EVM interpret Portfolio state, or force another persisted cut.
- **Gate:** check in the complete `C0 -> State input/output -> final result` table for every
  production operation, including Match variant projection/convergence, failure routes, EVM stages,
  Portfolio resume, consuming-input/clone choice, nonduplicative public-result field allocation,
  and strict schema/contract identities. Compile the typed continuation; if the generic derive is
  impossible, use concrete nominal caller instantiations, never erased bytes or history handles.

Apart from these gates, the architecture is fixed: access-only preparation, direct-new-only
`CommittedCall`, process-local response loss, capability-owned evidence, Runtime-owned
`PendingConclusion` around Store-owned `PreparedConclusion`, occurrence-level conclusion
uniqueness, CAS-linearized worker races, singular `C0`, State/Match-only sequential cumulative
execution, and three-family replay.

## 2. Program, State, capability, and assembly contracts

### 2.1 Program and qualified values

`mfm-program` exposes only:

```rust
pub struct ProgramDocument { /* private strict normalized State/Match control data */ }
pub struct ProgramRef(ContentRef);
pub struct Program { /* private sequential control form, indexes, document/ref, catalog brand */ }
#[derive(Clone)]
pub struct ProgramCatalog(Arc<ProgramCatalogInner>);
pub struct ProgramCatalogBuilder { /* callback-free declarations */ }
pub struct ProgramIngress<'a> { /* exact catalog + limits */ }

#[derive(Clone)]
pub struct QualifiedValue {
    /* canonical bytes, exact contract, erased typed object, exact catalog brand */
}
pub struct QualifiedTypedValue<T> { /* private typed witness */ }
```

`Program` has no public fields, `Deserialize`, unchecked constructor, or posterior validator.
`ProgramDocument` and `ProgramRef` never execute. Typed finish and hostile ingress converge on one
private normalized sequential-control constructor; only ingress performs byte-specific work.
Catalog equality for typed downcast and Runtime/Store association is exact in-process instance
identity, not just a persisted fingerprint.

The visible final declaration enum is exactly `State | Match`. Complete pure expansion removes child
operation/fragment boundaries and performs configuration specialization, abstract State lowering,
pre/proceed/post/failure injection, Match normalization, bounds, and one total root outcome before
Program construction. Resume consumes the final document without expansion. Strict ingress rejects
authored/expanded FanOut, lane, join, retained Fragment, and every compatibility form.

Every operation has exactly one domain-owned typed admission value, and that value is `C0`.
Domain/application planning constructs it before admission. Runtime and Store only qualify,
persist, and transport its exact nominal contract/content identity; they never assemble, merge,
split, reorder, or interpret its fields. Singularity is by representation across authored,
expanded, and certified Program, entry-point policy, admission command, and `RunAdmitted`. The
final APIs contain no repeated root declaration, root-order vector, initial-binding collection, or
ordered child-input bijection. A zero-state Program has an empty expanded control form, still admits
one `C0`, and may terminate only by selecting that exact value as its identity root result.

The normalized form contains one minimal callback-free `SequentialControlAddress` derived from
declaration ordinal plus enclosing Match arm tags. It addresses State occurrences and Match
selectors only. It has no lane, fragment, lexical slot/producer/origin, or generic structural-path
variant. Program certification, reducer, journal occurrence derivation, and replay share this strict
address; all old path/ref identities reject.

Program certification proves that every successful terminal path returns exactly one value of the
declared root-result contract and has no continuation, independent of whether its terminal State
is Pure, Read, or Effect. For Access, Runtime may expose that typed root value only after Store has
durably committed or qualified the matching `StateConcluded::Access`. A context-shaped value is
not inherently unfinished; reject it only if its contract differs from the declared root or a
continuation remains.

Only an empty expanded control form may use admission-derived zero-state terminality. Every
nonempty successful path must conclude a terminal State. Certification rejects a Match-only path or
a selected empty arm that would otherwise end without `StateConcluded`.

The authoring pipeline carries one current typed `Value<C>`. A State consumes that exact value and
replaces it with `Value<S::Output>`. `Match` selects from a closed-sum complete context; the reducer
qualifies exactly one declared variant payload, which is itself the complete cumulative context and
exact content identity consumed by that arm's first State. Every continuing arm returns one exact
common nominal contract. A nested-field decision not exposed by the context's closed sum is produced
by an explicit Pure State. There is no second outer-selector argument or arbitrary prior-output
lookup.

A child-operation authoring abstraction accepts only one exact current `Value<Cn>` as its singular
fragment-entry value. Pure expansion substitutes it unchanged and returns one successor value to
the caller continuation; it never converts the caller context into a different nominal child
wrapper. An explicit caller-owned State must construct such a wrapper first. An earlier, foreign,
missing, extra, or wrong-contract child value is rejected.

All retained domain values must be valid by representation. Private fields, checked constructors,
checked `Deserialize`, bounded collections, and nominal newtypes replace posterior semantic
validators. `QualifiedValue` retains canonical bytes and the typed object; hot Runtime paths encode
each required canonical value once and never serialize then redecode just-produced input, intent,
evidence, outcome, failure, or fact values for typed handoff.

Each operation/domain fragment owns its cumulative context types as ordinary bounded, secret-free
`MfmValue`s. No kernel `CumulativeContext`, heterogeneous map, output registry, history reference,
or content-reference accumulator is added. Successive full contexts may create `O(n^2)` canonical
retention; this is deliberate and bounded.

Typed evidence is keyed by `(ValueContractRef, ContentRef)`, never bytes alone. Identical bytes
under two nominal contracts may share only canonical byte storage, not a downcast witness.

### 2.2 Durable State and capability ABI

The public callback-free ABI is conceptually:

```rust
pub trait State: Send + Sync + 'static {
    type Input: MfmValue;
    type Output: MfmValue;
    type Failure: FailureValue;
    type Execution: Execution;
    type Lowering: StateLowering;

    fn state_id() -> Result<StableId, ProgramBuildError>;
    fn fact_slots() -> Result<Vec<FactSlot>> { Ok(Vec::new()) }
    fn maximum_conclusion() -> Result<ConclusionCapacityBound, ProgramBuildError>;
}

pub trait Execution: private::ExecutionSealed + Send + Sync + 'static {}
pub struct Pure;
pub struct Read<C: AccessCapabilityContract>(PhantomData<fn() -> C>);
pub struct Effect<C: AccessCapabilityContract>(PhantomData<fn() -> C>);

pub trait AccessCapabilityContract: Send + Sync + 'static {
    type Mode: AccessMode;
    type Intent: MfmValue;
    type Evidence: AccessEvidenceValue;
    type Facts: FactSelectionMode;

    fn contract_id() -> Result<StableId, CapabilityContractError>;
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

pub trait AccessMode: private::AccessModeSealed + Send + Sync + 'static {}
pub struct ReadMode;
pub struct EffectMode<E: EffectEntryMode>(PhantomData<fn() -> E>);
pub trait EffectEntryMode: private::EffectEntryModeSealed + Send + Sync + 'static {}
pub enum EntryOnce {}
pub struct EntryAbsorbing<const MAX_TOTAL_ENTRIES: u16>;
pub struct NoPriorFacts;
pub struct PriorRunFacts;
```

Only the sealed execution/access modes above exist. `Read<C>` is catalog-valid only for
`C::Mode = ReadMode`; `Effect<C>` only for `EffectMode<_>`. One capability type cannot serve both
modes. A shared provider protocol uses two nominal capability types with distinct contract
identities and private shared codec/transport helpers.

`EntryAbsorbing<MAX>` is valid only when the callback-free contract returns an absorption identity
and its evidence is post-state convergent. Catalog finalization rejects zero total entries and a
missing/present identity that disagrees with the mode. `total_attempt_bound` includes the initial
attempt; it is one for `EntryOnce`, the declared MAX for absorbing Effects, and the declared finite
Read bound. `State::maximum_conclusion`, evidence/object contract bounds, and fact/projection bounds
produce the exact reservation.

`AccessEvidenceValue` is a kernel-owned sealed blanket marker over valid-by-representation
`MfmValue + Send + Sync + 'static`; domain crates do not implement it manually. A production
capability selects one ordinary domain-owned finite enum with private/bounded members, checked
construction/decoding, no unknown/raw-string/provider-bytes escape variant, and the normal value
derive. Program catalog registration qualifies its exact contract/codec/type association. Each
capability therefore chooses one closed sum, not a broad domain error plus a runtime subset check.
Its checked decoding and catalog association preserve:

- returned success and returned rejection distinctions;
- accepted safe failures and uninhabited no-failure contracts;
- capability-specific definite-pre-entry evidence;
- accepted integrity-blocking evidence that grants no retry; and
- any success-only or other evidence-to-state-outcome disposition.

Keep hot and cold evidence authorities separate. Adapter ingress alone constructs the affine,
one-use, call-correlated `AcceptedEvidence<C>`. The journal stores canonical `C::Evidence`, and
callback-free retained-record ingress constructs only `QualifiedRecordedEvidence<C>` bound to the
referenced `StatePrepared`. Cold Store/replay cannot reconstruct hot accepted evidence or call
authority.

Do not retain both the old split settlement policy and a freely constructible new evidence/outcome
pair. Move each load-bearing disposition into evidence/proposal constructors and delete the old
callback-era wrapper only after its invalid combinations are impossible.

`prior_fact_selection` is a pure mechanical projection of Program/input-fixed canonical intent.
Post-commit selected facts may affect only evidence interpretation. Any fact that changes intent,
target, request bytes, signer, binding, or effect domain must be acquired by an explicit earlier
Read State and included in the complete cumulative `S::Input` before `prepare`; there is no hidden
pre-prepare fact callback or projection.

State lowering/expansion remains distinct from live access. Rename the authoring vocabulary away
from generic “Capability” where necessary; no expansion type becomes a live provider handle.

For each successful nonterminal State, `S::Output` is the complete cumulative successor context.
It carries all admitted/prior/current/remaining domain data needed by later States. Terminal Pure
consolidation/public-projection States may return the final domain/public result, but Pure is not a
kernel requirement: any terminal State mode may return the exact declared root result. Certification
requires the exact selected predecessor output, nominal contract, and continuation continuity--not
merely compatible bytes or Rust shape. Across a direct State edge the input is the predecessor's
exact output. Across Match it is the callback-free, content-qualified selected closed-sum variant
payload specified by the Program; there is no second outer-selector input.

### 2.3 State conclusions

Use one strict proposal algebra. Only the State outcome is freely constructible; the opaque access
proposal name below is conceptual—the consuming methods return the public opaque handler result
that privately contains it:

```rust
pub enum ProposedStateOutcome<O, F> {
    Success {
        output: O,
        facts: FactProposalSet,
    },
    Failure {
        failure: F,
    },
}

pub struct AccessConclusionProposal<S: State, C: AccessCapabilityContract> {
    /* private call correlation + disposition-specific evidence + payload */
}

impl<S: State, C: AccessCapabilityContract> AcceptedOutcomeAccess<S, C> {
    pub fn conclude(
        self,
        outcome: ProposedStateOutcome<S::Output, S::Failure>,
    ) -> AccessHandlerResolution<S, C>; // contains AccessConclusionProposal<S, C>
}

impl<S: State, C: AccessCapabilityContract> AcceptedIntegrityAccess<S, C> {
    pub fn conclude_blocked(self) -> AccessHandlerResolution<S, C>;
}
```

On a normal success `O` is the complete successor context unless this is a certified terminal
State, in which case it is the exact declared root-result contract.
The State implementation alone chooses and constructs that domain value. A failure is fail-fast by
default and carries no context implicitly. If a recovery State needs prior context, its declared
domain `F` embeds the exact bounded context explicitly and the certified failure route names that
contract; no kernel failure-context envelope is introduced.

Adapter ingress uses the qualified evidence variant disposition and its contract-fixed stable code
to return exactly one private-
constructor `AcceptedOutcomeAccess` or `AcceptedIntegrityAccess`; callers cannot turn one into the
other or pair evidence with another call. Failure owns no facts; integrity block owns no domain
output or facts. Exact generic spelling may differ, but these consuming authorities and illegal-
pair exclusions may not.

### 2.4 Callback-free bindings and live assembly

Reserve `StateImplementation` for the live object. Callback-free Program/catalog types use:

```rust
pub struct StateImplementationRef(ContentRef);
pub struct StateBinding<S: State> { /* private catalog witness */ }
pub struct ExecutionBindingRef(ContentRef);
pub struct ExecutionBindingDescriptor {
    /* state/capability/adapter implementation refs + immutable binding object */
}
pub struct ReadExecutionBinding<S, C> { /* private mode/type witness */ }
pub struct EffectExecutionBinding<S, C> { /* private mode/type witness */ }
```

The descriptor fixes the State implementation, capability implementation, adapter contract and
implementation, provider/route/signer or analogous immutable identities, binding object, and Effect
domain without secrets or live handles. A same-capability binding for another State is not
substitutable.

Program catalog finalization rejects duplicate/conflicting Rust types, contract identities,
codecs, modes, implementation refs, or binding descriptors. It emits cloneable callback-free
handles only; those handles are identities, not invocation authority.

Runtime assembly is:

```rust
pub struct RuntimeAssemblyBuilder { /* exact ProgramCatalog + live registrations */ }
pub struct RuntimeAssembly {
    catalog: ProgramCatalog,
    registry: ProcessRegistry,      // private, non-Clone
    brand: RuntimeAssemblyBrand,    // private, non-Serde
}

pub struct StateImplementation<S: State> {
    inner: private::TypedStateImplementation<S>,
}

pub struct QualifiedAdapter<S, C> {
    /* exact ExecutionBindingRef + concrete secret-bearing implementation */
}
```

Mode-specific constructors accept only the matching `StateBinding`/execution binding:

```text
Pure:
  evaluate(&S::Input) -> ProposedStateOutcome<S::Output, S::Failure>

Read<C> / Effect<C>:
  prepare(&S::Input) -> Result<C::Intent, PreparationError>
  execute(CommittedCall<S, C>) -> Future<Output = AccessHandlerResolution<S, C>>
```

These signatures describe callback responsibilities, not a settled input-ownership ABI. U11 must
compile the no-blanket-`Clone` path: the affine action retains the exact typed input while
`prepare` borrows it, and Pure evaluation or the accepted/unresolved Access wrapper supplies the
one consuming handoff needed to build a complete successor or explicit failure context. If that
cannot be made sound and ergonomic, U11 may deliberately require a bounded explicit clone contract;
implementation must not assume `MfmValue: Clone`.

The public constructor may take async closures or a typed implementation object; it must not expose
associated handler/future types or an all-modes erased public trait. Runtime erases only into a
private exhaustive `Pure | Read | Effect` enum.

`AccessHandlerResolution<S, C>` is a public opaque, private-field return value solely because it
appears in the generic constructor closure. It is not an enum and has no free constructor. Only
consuming methods on exact call-bound `AcceptedOutcomeAccess<S, C>`,
`AcceptedIntegrityAccess<S, C>`, and `UnresolvedAccess<S, C>` construct it; Runtime privately erases
it into `HandlerResolution<S, C>`. The constructor is generic over the closure/future, so
`StateImplementation` exposes no associated handler or future type.

`PreparationError` is bounded and redacted. It appends nothing, mints no execution owner, returns
the same ready active session in a typed drive disposition, and is never automatically retried by
one `drive` call. A later explicit drive may attempt the same occurrence again.

Pure `evaluate` is deterministic, secret-free, and derives its result solely from exact `S::Input`.
Its supported constructor/callback path exposes no ambient I/O, clock, randomness, history, or
reader. Capturing such a handle is a trusted-code violation, not a supported dependency. Production
Pure fixtures assert the same qualified input yields identical canonical outcome/failure bytes and
that all reviewed dependencies are present in the typed input.

`RuntimeAssemblyBuilder::finish` associates every exact callback-free State descriptor with
exactly one live `StateImplementation<S>`. Every Read/Effect execution binding additionally maps to
one exact qualified adapter; Pure has none. Missing, surplus, ambiguous, wrong-mode, wrong-catalog,
wrong-provider, wrong-signer, or wrong-effect-domain registrations reject before a facade exists.
The finished assembly exposes no replace, refresh, revoke, lease, current-generation, or callable
lookup API.

Concrete adapters own clients, transports, credentials, signers, keystore actors, transaction/
nonce machinery, stable-key transmission, raw response ingress, and entry classification. The
supported provider-entering method requires and consumes the exact committed-call token. A captured
`Arc<QualifiedAdapter<S, C>>` is safe to clone because it is inert without that token.

Keep the keystore `!Send + !Sync`. If assembly uses an actor, construct and retain the keystore on
one bounded dedicated OS thread, expose only bounded concrete signing commands, drain and join on
shutdown, and preserve AAD, zeroization, corruption, and redaction tests. Do not add unsafe
`Send`/`Sync` or `Arc<Mutex<Keystore>>`.

Supported State implementation constructors/callbacks receive only exact typed `S::Input` (and for
Access, the exact `CommittedCall`). Those APIs expose no `RunHistory`, `QualifiedRun`, Store,
reducer/cursor, journal reader, ambient output map, or arbitrary earlier output; ordinary domain
crate dependency rules keep those owners unavailable. Runtime passes the latest qualified
predecessor context and never searches history for a callback. Context values may retain reviewed
secret-free domain interpretations, never raw unrelated evidence, envelopes, coordinates, secrets,
credentials, or bearer authority. Malicious trusted Rust capturing an otherwise available handle
remains a TCB violation, not a sandbox guarantee.

## 3. Journal and Store semantic core

### 3.1 Strict run wire

`mfm-journal` defines exactly:

```rust
pub enum RunRecord {
    RunAdmitted(RunAdmitted),
    StatePrepared(StatePrepared),
    StateConcluded(StateConcluded),
}

pub enum RecordLogicalKey {
    Admission { run_id: RunId },
    StatePreparation {
        occurrence_id: OccurrenceId,
        preparation_ordinal: PreparationOrdinal,
    },
    StateConclusion { occurrence_id: OccurrenceId },
}
```

Every run append contains exactly one `RunRecord`. Objects, facts, routes, and projections are part
of that append's atomic closure but are not extra run records.

`RunAdmitted` retains the final Program ref, tenant, exactly one admitted domain value as the exact
initial cumulative context `C0`, selected configuration, fact-source manifest, Store/catalog
coordinates, invocation identity, and admission bounds. It has no root/value list or binding
collection. Planning constructs `C0`; Runtime and Store preserve its exact nominal contract and
content identity without interpreting its fields. It is always a separate genesis append.

`StatePrepared` has the minimal durable body:

```rust
pub struct StatePrepared {
    occurrence_id: OccurrenceId,
    canonical_intent: TypedValueRef,
    execution_binding: ExecutionBindingRef,
    replaces: Option<StatePreparationRef>,
    reserved_conclusion_capacity: ConclusionCapacityReservation,
    fact_selection: Option<PreparedFactSelection>,
}
```

`FactSelectionRequest` is the callback-free capability-level request mechanically projected from
canonical intent. Store combines it with the admitted source manifest and preparation-time frontier
to construct secret-free `PreparedFactSelection`; it is not a post-commit State callback result.
Direct-new commit uses that fixation to mint the optional one-use fact continuation.

Program/reduction supplies the immutable cumulative input's exact nominal contract/content identity/
typed value, mode, capability, evidence contract, entry discipline, effect domain, absorption rule,
and attempt bounds. Store assigns the preparation ordinal, record coordinates, append identity, and
`StatePreparationRef`. State/Runtime do not copy or author them.

Do not add a second `AccessAttemptId` unless it proves a proposition not already established by the
assigned preparation ref. Keep physical append id, journal preparation ref, and capability
absorption identity distinct.

The conclusion wire is:

```rust
pub enum StateConcluded {
    Pure {
        occurrence_id: OccurrenceId,
        outcome: StateOutcomeRef,
        facts_and_outputs: ConclusionObjectClosure,
    },
    Access {
        occurrence_id: OccurrenceId,
        preparation_ref: StatePreparationRef,
        accepted_fact_selection: Option<AcceptedFactSelectionRef>,
        accepted_capability_evidence: TypedValueRef,
        outcome: StateOutcomeRef,
        facts_and_outputs: ConclusionObjectClosure,
    },
}
```

Exact field factoring may avoid redundant refs, but the outer `Pure | Access` sum and uniform Access
payload are normative. The logical conclusion key is the occurrence.
`AcceptedFactSelectionRef` binds the selected response and completeness attestation to the exact
preparation/request/frontier; it is absent for capabilities without prior facts.

Accepted integrity blocking is a closed capability-evidence variant containing its bounded stable
redacted code. Its Program/capability-certified callback-free disposition derives the typed failure
outcome and empty closure for the same uniform Access record. It grants no retry and invokes no
additional State interpretation; cold reduction invokes no State code. It cannot carry prior
context to recovery. A capability needing State interpretation or a context-carrying recovery route
uses an ordinary accepted evidence variant and persists its interpreted typed State failure instead.

Every successful nonterminal outcome's closure contains the complete successor cumulative context.
It is never an implicit delta, lane result, ambient output-map element, or instruction to retrieve
earlier values. A direct State edge fixes the next input to that exact contract/content identity.
A Match edge additionally binds the parent closed-sum context, deterministic
`SequentialControlAddress`, declared arm tag/payload selector, and exact qualified payload that
becomes its selected arm's complete input.

A successful terminal outcome of any mode instead contains the exact declared root result. Pure
uses `StateConcluded::Pure`; Read/Effect uses `StateConcluded::Access` with its selected preparation
and accepted evidence. Store derives terminality only when that exact root contract concludes with
no reachable continuation, and Runtime returns no Access root before the recorded conclusion is
durable and qualified.

The changed records, Program refs, fact attestations, portable format, EVM values, and Store schema
use fresh strict identities/goldens. Keep the existing candidate, assigned-record, commit, and
recursive head algorithms/domain separators; old record tags and fields reject.

### 3.2 Qualified run and reducer lattice

Store owns:

```rust
pub struct QualifiedRun(Arc<QualifiedRunInner>);       // callback-free, cloneable evidence
pub struct ActiveQualifiedRun { /* Store owner + run; affine */ }

pub enum SelectedRunAction {
    Pure(SelectedPureAction),
    Access(SelectedAccessAction),
    UnresolvedRead(SelectedReadPreparation),
    UnresolvedEffect(SelectedEffectPreparation),
    Terminal(SelectedTerminalRun),
}
```

Every selected action owns the exact active run, head, occurrence, input, Program/catalog witness,
and reducer state. Runtime may borrow a view to choose dispatch, then must consume the action into
the exact typed binding. There is no cloneable action ref or caller-supplied predecessor.

`QualifiedRun` carries one deterministic cursor through the sequential State/Match control form.
Every valid prefix exposes zero or one selected action—never a collection. Match reads its exact
closed-sum selector from the current cumulative context, chooses one exhaustive arm, and requires
every continuing arm to produce the exact common context contract. Nonselected arms produce no
action. A failure selects its declared route immediately; every later normal declaration is
unreachable.

The reducer transitions are exactly:

```text
Unadmitted -> Admitted or terminal-zero-state
Ready      -> Pure conclusion
Ready      -> preparation p0 selected                 // access only
Prepared i -> conclusion i
Prepared i -> preparation i+1 selected, i superseded // Read/absorbing only
Concluded  -> no later record for that occurrence
Terminal   -> no later run record
```

A successful conclusion advances a direct State edge only when its complete output is the exact
next occurrence input. Across Match, the reducer qualifies the exact declared variant payload from
that concluded closed sum as the arm input. Retained qualification rejects a wrong parent context,
sequential address, arm tag/payload selector or payload, stale earlier output, same-shaped foreign
nominal value, wrong content identity, truncation, insertion, reorder, contract substitution, stage
rewind/skip, and surplus context. There are no lane walkers, nested cursors, actionable collections,
unresolved barriers, completion-order reconciliation, or join synthesis.

Replacement rules:

- `replaces` names exactly the selected unresolved preparation;
- Store ordinal increments without gap; no branches, cycles, or old selection restoration;
- occurrence, immutable cumulative input contract/content/typed value, capability, canonical
  intent, binding, and effect domain are equal;
- a declared Read freshness coordinate may change only outside provider semantics;
- every Read replacement consumes its Program-declared total-attempt budget, including the initial
  attempt;
- `EntryOnce` has no replacement;
- `EntryAbsorbing` also preserves absorption identity and stays below total-entry budget; and
- no replacement is legal after occurrence conclusion or derived terminality.

Terminality is checked after every record. It cannot be derived while a reachable occurrence has a
selected unresolved preparation. Cold ingress rejects any trailing suffix. The database terminal
field is projection only.

Zero-state admission is the only no-State terminal path. Its expanded control form is empty, and it
is valid only when the exact admitted `C0` contract/content identity is the declared root result. A
Match-only path or selected empty arm cannot become terminal. For a nonzero Program, the terminal
State may be Pure, Read, or Effect; its output must equal the declared root-result contract and
leave no continuation. A context-shaped root is valid when that is the declared result.

Conclusion disposition precedence is mode-aware:

1. same Pure occurrence/outcome/canonical closure, or same Access occurrence/preparation/optional
   fact selection/evidence/outcome/canonical closure: `AlreadyConcludedSame`;
2. a provably superseded same-occurrence Access preparation: `NoLongerSelected`;
3. an already-durable different conclusion for the Pure occurrence or same selected Access
   preparation: `Conflict`;
4. foreign, missing, cross-run, or cross-occurrence ref: invalid correlation/history. A Pure
   occurrence lacking a durable conclusion must remain ready; any history where it does not is
   invalid rather than a semantic disposition.

Byte-equal domain output under another preparation is not idempotency.

### 3.3 One event, reducer, and binder

Store has one private semantic path:

```rust
fn apply<K: EventKind>(
    previous: Option<&QualifiedRun>,
    event: ResolvedEvent<K>,
) -> Result<PendingAppend<K>, SemanticError>;

fn bind_local<K: EventKind>(
    pending: PendingAppend<K>,
    context: AppendContext,
) -> Result<PreparedAppend<K>, PrepareAppendError>;

fn bind_retained<K: EventKind>(
    previous: Option<&QualifiedRun>,
    pending: PendingAppend<K>,
    fixation: RetainedFrameFixation,
) -> Result<QualifiedRun, HistoryIngressError>;
```

Event kinds are admission, State preparation, and State conclusion only. Typed local proposals and
strict retained records produce the same `ResolvedEvent`. `AppendContext` explicitly supplies Store
identity/epoch, predecessor, append request id, fact coordinates, object context, capacity, and
projection contract. No binder reads ambient state.

Delete dual Intent/Recorded reducers, semantic equality/comparison typestates, local
`qualify_recorded_successor`, just-authored decode/index reconstruction, and positive echo
comparison. Keep encode/cold-ingress equivalence as a property test of the one law.

### 3.4 Capacity accounting

`QualifiedRun` carries cumulative committed use and at most one reserved liability. A
`ConclusionCapacityReservation` is derived from the Program State/capability maximum and includes:

- largest legal evidence and State outcome variant;
- newly reachable canonical object bytes/counts without assuming deduplication;
- maximum output/failure and success-owned facts;
- accepted prior-fact response and completeness attestation when applicable;
- dense publication row and tenant-head change;
- run indexes, terminal and optional attention projections;
- complete-frame/candidate/record bytes and batch count; and
- backend parameter/count limits.

Preparing the sole selected access atomically adds one liability before a call can exist.
Replacement transfers the selected predecessor's liability while accounting for its new record
bytes. Conclusion consumes the liability. Superseded preparations own no conclusion capacity.
Exact-bound passes; bound-plus-one rejects before external entry.

Admission and certification also bound `C0`, maximum `Cn`, source/collection/occurrence counts,
total context/conclusion/run bytes, objects, facts, projections, cold-fold work, and backend
parameters. The sum of complete cumulative conclusions may be `O(n^2)`; accounting assumes no
structural sharing or deduplication optimization. The value `4,096` survives only if U9 justifies it
as a product workload limit, never because FanOut once admitted that many lanes.

Pure has no pre-entry reservation. Store must preflight its declared maximum at the selected head
before callback dispatch, retain the actual callback proposal in `PendingConclusion`, and never
rerun the callback merely because the head or fact-publication frontier moved. A later race that
makes the retained proposal impossible to fit produces the typed capacity/terminal disposition; it
does not discard and recompute the proposal within the live owner.

### 3.5 Store ports and demand ingress

Trusted composition opens one composite backend with expected identity, exact Program catalog, and
work limits:

```rust
pub struct StructuredStore;
pub struct OpenedStructuredStore;
pub struct QualifiedHistoryPort; // non-Clone Runtime owner
#[derive(Clone)]
pub struct HistoryReader;
pub struct ConfigurationStore;
pub struct StoreAuditPort;        // non-Clone operator owner

impl StructuredStore {
    pub async fn open(
        backend: Arc<dyn StructuredStoreBackend>,
        expected_identity: StructuredStoreIdentity,
        catalog: ProgramCatalog,
        limits: StoreWorkLimits,
    ) -> Result<OpenedStructuredStore, StoreOpenError>;
}
```

The consuming split returns ports sharing one exact private coordinator/catalog brand for that
open. Callers cannot construct ports, mix branded parts from different opens, or turn a backend
trait object into semantic authority. Multiple qualified opens, Runtime assemblies, workers, and
processes may target the same admitted Memory/PostgreSQL writer identity. They retain independent
process-local brands and all mutations share the backend's append-id lookup and exact-head CAS;
none receives process-global execution authority. Runtime `resume` returns `ActiveQualifiedRun`;
readers return only callback-free evidence or purpose DTOs.

Store open checks schema, identity/epoch, channel, writer role, durability, catalog, and bounded
transaction ability. It performs zero run/configuration enumeration. Complete-prefix ingress uses
one shared bounded CPU-work/ingress limiter and enforces frame/count/byte bounds before allocation.
Cancellation does not release a CPU permit until the blocking job exits.

Purpose readers, Replay/export, fact producer loads, and explicit audit call the same complete-
prefix qualifier. `audit_store` alone uses one fixed snapshot across runs, configuration, and dense
fact routes; its report creates no cache or drive authority.

## 4. Mechanical backend and PostgreSQL cutover

### 4.1 Backend SPI

Concrete Memory/PostgreSQL backends implement one composite mechanical object:

```rust
pub trait StructuredHistoryBackend: Send + Sync + 'static {
    fn load_complete_prefix<'a>(
        &'a self,
        tenant: &'a TenantScopeId,
        run: &'a RunId,
        limit: RawHistoryLoadLimit,
    ) -> BackendFuture<'a, Option<RawRunPrefix>>;

    fn compare_and_append<'a>(
        &'a self,
        command: &'a BackendAppendCommand,
    ) -> BackendFuture<'a, BackendAppendOutcome>;
}

pub enum BackendAppendOutcome {
    NewlyCommitted,
    Found(StoredAttemptBytes),
    StaleHead,
    AcknowledgementUnknown,
}
```

Configuration, fact, and audit subtraits remain domain-specific. Do not add a blanket adapter that
stitches unrelated trait objects into one Store. Raw DTOs have private fields plus checked
mechanical constructors/getters for coordinates/counts/bytes; they never decode canonical JSON,
qualify Program/value semantics, reduce, or mint authority.

`BackendAppendCommand` is non-Clone and exposes borrowed mechanical getters for exact Store/tenant/
run route, expected predecessor or genesis absence, append id, one canonical frame, head digest,
projection deltas, fact precondition/publication, and optional attention projection. Store retains
the exact command across acknowledgement ambiguity and re-borrows it; the backend never returns a
positive payload echo.

Operational errors are exhaustive and redaction-safe: retryable before-commit failures,
Store-epoch change, durability loss, fact-frontier change, projection mismatch, and fatal backend
contract failure remain distinct. Only loss after transaction submission with unknown outcome is
`AcknowledgementUnknown`.

### 4.2 Append transaction

The PostgreSQL append transaction:

1. begins under the managed role/search path and asserts effective durability;
2. locks and checks Store scope/epoch through commit;
3. locks the exact Store/tenant/run route or proves partition-scoped genesis absence;
4. looks up append identity inside that same route and returns raw `Found` before predecessor
   comparison;
5. compares the exact predecessor when no attempt exists;
6. checks mechanical route/projection consistency;
7. locks/checks the tenant fact head when a conclusion publishes facts;
8. inserts one immutable canonical frame;
9. advances run head and reducer-derived terminal/optional attention projection;
10. inserts at most one dense fact publication and advances its head; and
11. commits, mapping definite success to payloadless `NewlyCommitted` and uncertain acknowledgement
    to `AcknowledgementUnknown`.

Every statement is one transaction; rollback exposes none of the frame, objects, head, conclusion,
facts, or projections. Duplicate conclusion found-same does not publish again.

Acknowledgement resolution reissues the same append identity under the same serializing route lock.
A snapshot miss outside that operation is not proof of absence. If rebinding a semantic conclusion
after a proven absent physical attempt, Store first reduces the latest prefix and assigns a new
physical append id bound to that predecessor.

### 4.3 Fresh PostgreSQL baseline

Use one bounded canonical complete-frame `BYTEA` row per run append plus only mechanical columns
needed for route, sequence, append-id idempotency, selected head, fact routing, and the chosen
attention projection. Keep the target contract id
`mfm.structured-run-history-postgres.v8`; v8 now means the final three-family schema because the
superseded v8 proposal was never deployed.

At minimum:

```sql
CREATE TABLE run_history_batches (
    store_scope_id         TEXT NOT NULL,
    store_epoch            NUMERIC(20, 0) NOT NULL,
    tenant_scope_id        TEXT NOT NULL,
    run_id                 TEXT NOT NULL,
    run_sequence           NUMERIC(20, 0) NOT NULL,
    append_request_id      TEXT NOT NULL,
    head_commit_digest     TEXT NOT NULL,
    committed_batch_bytes  BYTEA NOT NULL,
    PRIMARY KEY (store_scope_id, store_epoch, tenant_scope_id, run_id, run_sequence),
    UNIQUE (store_scope_id, store_epoch, tenant_scope_id, run_id, append_request_id)
);

CREATE TABLE run_history_heads (
    store_scope_id         TEXT NOT NULL,
    store_epoch            NUMERIC(20, 0) NOT NULL,
    tenant_scope_id        TEXT NOT NULL,
    run_id                 TEXT NOT NULL,
    head_sequence          NUMERIC(20, 0) NOT NULL,
    head_commit_digest     TEXT NOT NULL,
    closed                 BOOLEAN NOT NULL,
    -- Include needs_effect_attention only under the enabled U2 ruling.
    PRIMARY KEY (store_scope_id, store_epoch, tenant_scope_id, run_id)
);
```

The migration/schema owner must add the exact selected-head foreign key, catalog identities,
constraints, indexes, roles, and grants required by the final query inventory. `closed` and optional
attention are checked projections, never event authority. Defense-in-depth projection/uniqueness
for occurrence preparations/conclusions may be added only if derived atomically from Store's sealed
command; the reducer remains authoritative.

Delete normalized object-row reconstruction and duplicated candidate/predecessor columns whose sole
consumer rechecks the frame. Delete `target_authority` and generic physical-currentness columns only
after U5 maps any immutable survivor. Rewrite the destructive pre-release baseline; add no ALTER
migration or old reader.

`check_ready` validates the exact v8 catalog, route keys, selected-head constraint, roles, channel,
Store epoch, and admitted durability, but reads no run/configuration history. It rejects the
superseded pre-cutover schema contract and every five-family projection assumption.

### 4.4 Durability and restore

The initial `QualifiedPostgresDurability` means exactly primary crash/restart survival. Qualify
logged tables, primary status, `fsync = on`, `full_page_writes = on`, effective
`synchronous_commit != off`, role/search path/channel, schema identity, and writer epoch. Each
append rechecks epoch and transaction-effective durability. The epoch is a persisted lineage and
CAS precondition, not a live-process lease.

A direct-new preparation result is not returned until the exact `StatePrepared` frame is durable
under this profile. A normal restart may resume the same epoch from qualified durable history;
multiple live workers/processes use the same exact-head protocol. Legitimate restore rotates scope
or epoch, cannot append to an old run, and follows U6's dormant-run disposition. No test or
document may claim the hash chain detects same-epoch self-consistent rollback after every outside
anchor is lost.

## 5. Runtime lifecycle and execution ownership

### 5.1 Sessions and exhaustive owner-carrying outcomes

Runtime exposes:

```rust
#[derive(Clone)]
pub struct Runtime(Arc<RuntimeInner>);
pub struct AdmissionInput { /* Runtime-minted owner-bound typed Store admission */ }
pub struct ResumeInput { /* owner-bound run selection under exact Store/Runtime */ }
pub struct RunSession {
    /* Runtime brand + ActiveQualifiedRun + latest qualified typed context */
}
pub struct SuspendedRun { /* exact affine retry/unknown owner; !Sync */ }

impl AdmissionInput {
    pub async fn spawn(self) -> SpawnStep;
}

impl Runtime {
    pub async fn resume(&self, input: ResumeInput) -> ResumeStep;
}

impl RunSession {
    pub async fn drive(self) -> RuntimeStep;
}

impl SuspendedRun {
    pub async fn resolve(self) -> RuntimeStep;
}

pub enum SpawnStep {
    Active(RunSession),
    Terminal(TerminalRun),
    Suspended(SuspendedRun),       // retained admission acknowledgement/retry owner
    Conflict(AdmissionConflict),
    Failed(AdmissionFailure),      // no append could have committed
}

pub enum ResumeStep {
    Active(RunSession),
    Terminal(TerminalRun),
    Parked(ParkedRun),
    Failed(ResumeFailure),
}

pub enum RuntimeStep {
    Advanced(RunSession),
    Terminal(TerminalRun),
    PreparationRejected {
        session: RunSession,
        error: PreparationError,
    },
    Unresolved {
        session: RunSession,       // selected preparation remains durable
        classification: UnresolvedClassification,
    },
    Parked {
        session: RunSession,
        reason: ParkReason,
    },
    Suspended(SuspendedRun),       // exact preparation/conclusion owner retained
    Conflict {
        history: QualifiedRun,
        error: RunConflict,
    },
    Failed {
        history: QualifiedRun,
        error: TerminalRuntimeError,
    },
}
```

Privately, `SuspendedRun` is a strict sum of admission input/append, `PreparedExecution`, or
`PendingConclusion`; each variant retains its exact physical acknowledgement state and inert
continuation. It contains no cold-mint path. `TerminalRun` and every qualified-history failure are
callback-free evidence, not live sessions. Exact final public names may vary, but no disposition or
owner fate may be omitted from these sums.

`RunSession` is non-Clone/non-Serde and every changing operation consumes it. Runtime owns bounded
active-session, deterministic CPU-job, planning-job, and ingress limits. Pure evaluation, intent
construction, evidence binding/canonicalization, and interpretation run on bounded CPU work, not a
Tokio worker. Provider futures remain async and are never detached to catch panic.

Independent workers may hold affine sessions qualified from the same run prefix. Each changing
operation consumes only its own owner, and no local owner can be cloned or split. The Store—not a
Runtime registry—linearizes their mutations through route-scoped append-id lookup, exact-head
compare-and-append, and the reducer's closed semantic dispositions. A stale worker receives
qualified history or an explicit rebind/recovery owner; it never attaches a successor or call from
another worker. Runtime exposes no `Busy`, per-run guard, exclusive-open, or writer-process-lease
lifecycle.

Each session retains the exact latest `QualifiedTypedValue<Cn>` already produced by the
reducer/binder. Direct conclusion advancement performs the required canonical conclusion encoding
once, then passes that retained typed object to the next State without a serialize/decode round
trip or historical read/fold. Cold resume performs one bounded callback-free complete-prefix
qualification, validates every context edge, and binds the latest concluded value to the next
occurrence. Runtime exposes neither the prefix nor a history accessor to State code.

Every public/private async ownership protocol uses exhaustive enums. Retryable and unknown branches
return the exact owner. A generic `Err` cannot silently drop a session, inert execution, fact
continuation, accepted result, or pending conclusion. A pending conclusion may be dropped only by
the explicit supervisor force-destruction operation under the documented response-loss policy;
ordinary terminal/error classification cannot do so.

The selected active run moves out of the session into the current action. Admission/conclusion
direct commit supplies the sole successor that rebuilds the shell. A direct-new preparation commit
moves that successor into `CommittedCall`. There is never a second session beside a live call or
pending conclusion within that same affine worker owner. Other workers may retain independently
qualified, potentially stale sessions without authority derived from this call.

### 5.2 Admission and Pure

Admission input is minted only from exact typed Program/root/configuration/context/fact-source
evidence under one Store/catalog/tenant owner. Concurrent workers may spawn or resume the same run.
Spawn commits `RunAdmitted` alone against an absent predecessor. Direct-new returns a session or
zero-state terminal result; found-same promotes nothing and requires qualified history; found-
different conflicts; unknown retains the exact admission append and worker owner.

The admission coordinator classifies direct-new, found-identical, found-different, retryable known-
precommit, distinct-id stale genesis, and acknowledgement-unknown distinctly. Raw `Found` is
same-physical-id lookup only. For a distinct-id `StaleHead`, Store qualifies the existing
`RunAdmitted` and compares the semantic admission candidate excluding physical coordinates:
same returns recorded Active/Terminal, different is `AdmissionConflict`, and invalid history is an
integrity failure. Unknown resolution serializes on the same physical id: found-identical returns
qualified recorded history, proven absence with the genesis precondition still current permits
exact resubmission, and repeated unknown retains the same owner. Only a later direct-new
resubmission advances. If another physical id commits genesis before that resubmission, its raw
`StaleHead` consumes the suspended owner through the same semantic comparison: same returns
qualified Active/Terminal, different conflicts, and invalid history fails integrity qualification;
it does not resubmit again. Every branch invokes zero State/adapter/provider callbacks.

Domain/application planning constructs exactly one bounded initial cumulative context `C0` before
admission. Runtime qualifies that one typed object; direct-new retains it in the new session, while
cold found-same qualification reconstructs the same nominal contract/content identity from durable
evidence. Runtime and Store never synthesize a context from multiple inputs.

For Pure:

1. Runtime binds the selected Pure action and exact predecessor cumulative context to its exact
   `StateImplementation<S>`;
2. it moves the action through bounded panic-contained evaluation once;
3. Store qualifies the typed proposal into `PreparedConclusion`, and Runtime constructs a Pure-
   scoped `PendingConclusion` around it and the inert session continuation;
4. same-run head contention classifies the recorded successor without reevaluation, while
   fact-frontier contention and acknowledgement recovery move that same pending owner; and
5. a successful nonterminal conclusion installs its complete typed successor context in the
   rebuilt worker session; direct/recorded terminal conclusion returns the final result.

Once one worker has constructed `PendingConclusion`, no contention or recovery branch for that
owner reruns the Pure callback. Another worker may independently evaluate the same ready occurrence;
Store selects one durable conclusion. A callback panic creates no record.

### 5.3 Prepared execution

For access, Runtime passes the exact selected cumulative input to deterministic
`prepare(&input) -> C::Intent`, qualifies the already-valid typed intent once, and asks the selected
Store action to prepare `StatePrepared`. Store captures only callback-free semantics. Runtime
immediately pairs the resulting owner with its exact inert
implementation continuation and session shell:

```rust
struct PreparedExecution<S, C> {
    continuation: SessionContinuation,
    append: PreparationAppendState<C>,
    implementation: InertImplementationContinuation<S, C>,
}

enum PreparationAppendState<C> {
    Prepared(PreparedStatePreparation<C>),
    AcknowledgementUnknown(UnresolvedPreparationAppend<C>),
    Rebindable(RebindableStatePreparation<C>),
}
```

The concrete type may be privately erased for dynamic Programs. It has no execute/provider method,
is non-Clone/non-Serde, and is bound to one Runtime/Store/catalog/occurrence.

The Runtime commit coordinator consumes the whole value and calls the append's owner-bound Store
commit. The backend reports only its raw mechanical SPI. Store qualifies that outcome and returns
one closed semantic disposition:

```text
DirectlyCommitted(CommittedPreparation)
ExistingSame(ActiveQualifiedRun)
Terminal(ActiveQualifiedRun)
SelectedOtherPreparation(ActiveQualifiedRun)
Conflict(ActiveQualifiedRun, PreparationConflict)
AcknowledgementUnknown(UnresolvedPreparationAppend)
Retryable(PreparedStatePreparation)
FactPreconditionChanged(RebindableStatePreparation)
```

or one typed terminal operational error:

```text
InvalidCorrelation(ActiveQualifiedRun, terminal error)
CapacityRejected(ActiveQualifiedRun, terminal error)
StoreEpochChanged(QualifiedRun, terminal error; reopen required)
DurabilityProfileLost(QualifiedRun, terminal error; reopen required)
ProjectionMismatch(QualifiedRun, terminal integrity error)
```

Store returns only Store-owned append/rebind values or qualified history. Runtime's same consuming
frame still holds the inert implementation/session continuation and rewraps the Store product when
recovery is safe; terminal paths return the qualified callback-free history and explicitly destroy
the inert live continuation. There is no payloadless generic failure branch.

Raw backend `StaleHead` is not a semantic rebind disposition. Store qualifies the new sequential
head and classifies it as `ExistingSame`, `Terminal`, `SelectedOtherPreparation`, `Conflict`, or an
invalid correlation/integrity error. A same-run append cannot leave the original preparation action
current. `RebindableStatePreparation` is reserved for a definitely precommit fact-precondition
change that preserves the run predecessor and every canonical provider-semantic field.

Only the coordinator's direct-new arm still owns both `CommittedPreparation` and the exact inert
continuation. It consumes them into `CommittedCall`. No API accepts a free `NewlyCommitted`, record
ref, head, Boolean, or committed Store token with a separately supplied continuation.

On acknowledgement unknown, quarantine the entire `PreparedExecution`. Resolution:

- found same destroys the implementation continuation and returns no call;
- found different/invalid fails closed;
- proven absent while the exact reducer precondition remains current--either an unprepared first
  attempt or the same selected replacement parent--retries/rebinds the same owner;
- retry the same physical append id only when the complete predecessor/fact-precondition/candidate
  command is unchanged; any rebind changing predecessor, fact fixation/precondition, or canonical
  frame requires proven absence or definite precommit failure, the same semantic owner, and a
  fresh physical append id;
- only that retry's own direct-new result creates a call;
- `SelectedOtherPreparation`, terminality, stale action, repeated unknown, drop, or cold restart
  creates no call.

### 5.4 `CommittedCall` and prior facts

`CommittedCall<S, C>` is public only as an opaque Runtime-to-domain implementation input. It has
private fields, no constructor, `Clone`, `Copy`, `Serialize`, `Deserialize`, or `Default`, and its
provider-entering operation consumes it.

It binds:

- exact Runtime/catalog/assembly/Store identity, epoch, durability, tenant, run, Program, and
  committed head;
- occurrence identity/`SequentialControlAddress`, exact cumulative input contract/content/typed
  value, mode, capability, intent, State implementation, and execution binding;
- qualified adapter/provider/target/signer/effect-domain association;
- preparation ref, ordinal, replacement parent, and reserved conclusion capacity;
- one process-local call-correlation identity plus Store-minted conclusion correlation; and
- exact preparation-bound fact continuation where `C::Facts = PriorRunFacts`.

The supported flow is conceptually:

```rust
match call.invoke_bound_adapter().await {
    AccessResolution::Outcome(accepted) => {
        accepted.conclude_with_input(|input, evidence| interpret(input, evidence))
    }
    AccessResolution::BlockedIntegrity(accepted) => accepted.conclude_blocked(),
    AccessResolution::Unresolved(unresolved) => unresolved.finish(),
}
```

The consuming method names are conceptual until U11 fixes the input-owner ABI. The invariant is
that the call-correlated outcome/unresolved wrapper, not a clone requirement or history lookup,
owns the exact input needed for the successor or explicit State-failure proposal. Integrity blocking
is different: the Program/capability contract fixes one callback-free, failure-only route from its
evidence and stable code. It invokes no State callback and cannot carry cumulative context into a
recovery State. A capability needing domain interpretation or context-carrying recovery represents
the accepted integrity result in its ordinary Outcome evidence sum instead.

The public `AccessResolution<S, C>` is a strict enum only so downstream implementation code can
match it. Its `AcceptedOutcomeAccess`, `AcceptedIntegrityAccess`, and `UnresolvedAccess` payloads
have private fields and no constructors; only the bound adapter ingress can supply them. Their
consuming methods are the only constructors for opaque `AccessHandlerResolution<S, C>`.

These accepted outcome/integrity and unresolved wrappers are private-constructor, call-correlated
ephemeral values, not Runtime phases or durable records. An accepted wrapper cannot be paired with
another call or raw evidence. It offers only its disposition-specific conclusion constructor, not a
generic “safe abandon” conversion. Panic/process loss can still destroy it within the accepted loss
window.
The caller supplies no adapter, binding, request, target, signer, or provider argument. The
consuming method moves the exact qualified adapter and private binding witness already sealed into
`CommittedCall`, and derives request bytes from its canonical intent before provider entry.

For prior facts, the direct-new preparation commit alone mints the one-use Store continuation. The
call may move it through private attempt-local substates across retryable scans before provider
entry. It fixes Store, tenant, preparation, source manifest, selection request/digest, frontier, and
limits.
The adapter never owns a scanner, and cold history cannot recreate it. A failed/retryable scan
constructs no provider future. Selected facts cannot alter the committed provider intent/binding/
domain/absorption identity.

Per-call correlation flows from `CommittedCall` through adapter entry, ingress, accepted
evidence, and conclusion. Runtime checks exact identity before accepting the result. This prevents a
shared adapter from returning a stashed same-capability completion for another invocation.

Contain synchronous adapter-call construction and future-poll panic without a public
`UnwindSafe` bound and without spawning/detaching the provider future. Cancellation may lose the
owner; it never becomes proof of non-entry.

### 5.5 Closed handler resolution

Runtime's private closed result is:

```rust
enum HandlerResolution<S, C> {
    Concluded(CallBoundConclusion<S, C>), // uniform evidence + outcome + closure
    Unresolved(CallBoundUnresolved<S, C>),
}
```

It is not `Result`. `Concluded` can arise only from call-correlated accepted evidence and typed
proposal constructors. `CallBoundUnresolved` retains the exact active prepared successor and
call correlation, plus a bounded, stable, redaction-safe `UnresolvedClassification`, but contains
no invoker or external-I/O method. The classification alone is not constructible by state code,
persisted, or reducer input. Runtime consumes the wrapper to restore the live session at the
already-committed preparation; losing it falls within the explicit Access owner-loss window.

The definite matrix is normative:

| Adapter/handler result | Durable action |
| --- | --- |
| provider success | conclude |
| returned provider rejection | conclude |
| accepted safe failure | conclude under capability disposition |
| accepted capability-specific integrity block | conclude blocked, no retry authority |
| definite pre-entry evidence | capability-defined conclusion; never generic enum |
| malformed/unauthenticated/unbound response | unresolved preparation |
| transport acknowledgement ambiguous / possible entry | unresolved preparation |
| panic/cancellation before synchronous conclusion handoff | unresolved preparation |
| post-handoff panic/cancellation | Runtime supervisor retains `PendingConclusion` |
| process/explicit owner loss | durable conclusion or prior neutral prefix |
| no trustworthy result | unresolved preparation |

No ordinary `?` path may convert a definite returned rejection into unresolved state. A capability
with no safe failure remains uninhabited rather than accepting a broad enum and validating a subset.

### 5.6 Runtime-owned pending conclusion with a Store append owner

For Access, Runtime consumes its call-bound accepted wrapper and asks Store to qualify the
persistable `C::Evidence`, typed outcome, facts/outputs, selected preparation, optional accepted
fact response/attestation, and closure into a secret-free `PreparedConclusion`. For Pure, Store
qualifies the exact ready occurrence and typed proposal the same way. Runtime then combines that
Store owner with the already-held inert session continuation into:

```rust
#[must_use]
pub struct PendingConclusion {
    /* Runtime brand, inert session continuation, Store PreparedConclusion */
}

struct PreparedConclusion {
    /* canonical semantic conclusion,
       occurrence/(optional preparation)/original cumulative-input correlation,
       object closure, capacity discharge, exact state below */
    state: PreparedConclusionState,
}

enum PreparedConclusionState {
    Ready(PreparedConclusionAppend),
    AcknowledgementUnknown(UnresolvedConclusionAppend),
    Rebindable(RebindableConclusionAppend),
}
```

Within that same affine worker owner there is no independently usable `RunSession` beside
`PendingConclusion`; the session continuation is inert and private inside it. Other workers may
hold separately qualified, potentially stale sessions but cannot derive authority from this owner.
Store never depends on Runtime brands, dispatch, active-slot accounting, or the hot
`AcceptedEvidence<C>` wrapper. `PendingConclusion` contains no adapter, implementation, provider
future, raw response, scanner, secret, or provider/fact/state-callback I/O method. Runtime's
coordinator may use its inner owner only for Store conclusion-commit and acknowledgement-recovery
I/O. It is non-Clone/non-Serde and is the only outer owner moved through prepare, commit,
same-run-head classification, fact-frontier rebinding, and acknowledgement resolution for that
worker.

Store's private inner conclusion operation returns the following callback-free result. Runtime
never exposes it directly; it retains the inert continuation while moving only the inner
`PreparedConclusion` through Store:

```rust
enum InnerConclusionCommitStep {
    Committed(ActiveQualifiedRun),
    AlreadyConcludedSame(ActiveQualifiedRun),
    NoLongerSelected(ActiveQualifiedRun), // Access only
    Conflict {
        history: ActiveQualifiedRun,
        error: ConclusionConflict,
    },
    AcknowledgementUnknown(UnresolvedConclusionAppend),
}

pub enum InnerConclusionCommitFailure {
    Retryable {
        prepared: PreparedConclusion,
        error: RetryableConclusionError,
    },
    Permanent {
        prepared: PreparedConclusion,
        error: PermanentConclusionError, // Store/epoch/durability/capacity/integrity
    },
}

impl PendingConclusion {
    pub async fn advance(self) -> RuntimeStep;
}
```

The Runtime coordinator consumes the outer owner, temporarily retains its brand/inert continuation,
calls the private Store-inner operation, and rebuilds `RunSession`/`TerminalRun` on durable/history
results or rewraps the returned inner owner as `SuspendedRun`. The closed private state makes only
the appropriate commit, resolve, or rebind operation reachable. Every returned
`PreparedConclusion` includes the exact ready/unknown/rebindable physical state; all retryable and
permanent errors therefore rebuild the same pending owner. An explicit supervisor operation
outside this API may force-destroy it under the documented response-loss policy; an ordinary error
cannot.
No observation-specific Runtime lifecycle or public command hierarchy is introduced.

For Access, Store qualifies the conclusion against the exact cumulative input contract/content
identity fixed by the selected preparation and carried through `CommittedCall`. For Pure, it checks
the ready occurrence's selected predecessor context. Fact-coordinate rebinding and acknowledgement
recovery retain that identity and may not widen, refresh, substitute, or reinterpret it.

A same-run head change consumes the owner and loads one latest qualified snapshot. Because no
unrelated run record exists, Store must classify that head as the same/conflicting conclusion,
another preparation selected, terminality, or invalid history; it never rebinds the conclusion
across the changed head. A Pure occurrence lacking its conclusion must still be ready. Only an
independent fact-publication
precondition may change while the same run occurrence/selection remains current. Store then checks
Store/epoch/tenant/run/Program, fact dependency, exact capacity, and the same selected Access
preparation or Pure-ready occurrence, and uses a fresh physical append identity only after proving
the prior attempt absent or definitely precommit. It never reruns State logic, fact selection,
adapter, ingress, or interpretation.

Unknown resolution checks the exact current physical append id first:

- found same -> qualify recorded history; no republish;
- retained append absent but the same canonical conclusion recorded under another physical id ->
  `AlreadyConcludedSame`; no republish;
- proven absent + same Access selection or Pure ready occurrence -> retry/rebind the same canonical
  conclusion;
- superseded Access preparation -> `NoLongerSelected`;
- different durable conclusion -> `Conflict`;
- Pure occurrence no longer ready with no durable conclusion -> invalid history;
- repeated unknown/retryable -> return same owner; and
- explicit owner/process loss -> later cold history is either concluded or the prior reducer state:
  ready occurrence for Pure, selected preparation for Access.

### 5.7 Crash and race behavior

Freeze these linearizations in code/tests:

- concurrent admissions with the same physical id use `Found` for idempotency/acknowledgement
  recovery; distinct-id genesis races yield one direct-new and `StaleHead` losers, which qualify the
  existing admission and return recorded history when semantically same or conflict when different;
  unknown owners retain their exact physical attempt;
- concurrent workers may qualify and speculatively evaluate/prepare the same sequential occurrence;
  same-head preparation commands yield one direct-new call authority and every non-new branch has
  zero `execute` and provider entry;
- conclusion versus same-preparation conclusion: one physical append, other same/conflict;
- conclusion versus replacement: conclusion closes, or replacement supersedes and late result is
  `NoLongerSelected`;
- conclusion versus another same-run append: classify as same/conflicting conclusion, another
  preparation selected, terminal, or invalid; never rebind across it;
- conclusion versus independent fact-frontier movement: rebind only after the same Access
  selection or Pure-ready occurrence is proven;
- terminal conclusion versus stale preparation/conclusion: terminal wins; stale cannot rebind;
- unknown preparation found-same never calls; unknown proven absent may call only after a later
  direct-new resubmission; and
- all conclusion recovery has stable provider/ingress/interpretation counters.

The reducer exposes one action/selected preparation per durable prefix, but there is no
process-global live-owner limit. A permitted Read or EntryAbsorbing replacement may commit while an
older call or pending conclusion remains process-local; the older conclusion is accepted only if
its preparation is still selected. EntryOnce still has no replacement.

Cancellation, panic, or owner loss before the state consumes its accepted wrapper into the
synchronous conclusion handoff may discard an accepted response and leave Access history prepared.
That consuming handoff atomically transfers correlation/outcome ownership to Runtime, which builds
`PendingConclusion` before cancellation can observe a free value. Thereafter ordinary request/task
cancellation and graceful drain move the owner to Runtime's conclusion-recovery supervisor; they do
not discard it. Process death, OOM that terminates the process, or explicit supervisor force-
destruction may still lose the result. Durable history remains neutral. No successful or definite
result crosses the public boundary before a durable/qualified conclusion.

## 6. Recovery, facts, and configuration

### 6.1 Recovery actions

Cold reducer selection offers only:

- Read: bounded replacement preserving exact cumulative input contract/content/typed value,
  canonical intent, and binding;
- EntryOnce: park indefinitely, with no second attempt;
- EntryAbsorbing: bounded replacement preserving exact cumulative input, intent, binding, effect
  domain, and derived absorption identity;
- late result: `NoLongerSelected`; and
- terminal/waiting: no executable action.

State code cannot reclassify EntryOnce, enlarge a budget, change canonical intent or stable
absorption identity, substitute binding/domain, or hide a semantic retry after possible entry.

For Read, any allowed freshness coordinate is explicit and separate. For absorbing Effects,
`MAX_TOTAL_ENTRIES` includes the initial entry. Read and EntryAbsorbing attempts may overlap across
workers or processes, including a slow old operation continuing while a replacement enters.
Absorption must cover that full overlap and recovery horizon; an old result becomes
`NoLongerSelected` after replacement, and the budget may exhaust and park. EntryOnce cannot create
that overlap through MFM because it has no replacement.

Recovery State code receives prior cumulative context only when its declared domain failure value
carries that exact bounded context. Default propagation is failure-only. No recovery action obtains
ambient history or asks Runtime to reconstruct an earlier context for it.

Manual/operator attention is a projection and human workflow, not a journal mutation. A future
cancellation/expiry/abandonment/operator-termination event requires a specifically named RFC with
outstanding-effect semantics.

### 6.2 Prior-run facts

`mfm-facts` remains journal-independent and owns the strict request, selected fact, query result,
response, completeness, and attestation value types. Address-only refs live in `mfm-ids`.

Rename preparation-related fields consistently:

- `reservation_ref` -> `preparation_ref`;
- `CompleteThroughAccessReservationFrontier` ->
  `CompleteThroughStatePreparationFrontier`;
- request/attestation validators bind the exact `StatePreparationRef` and preparation-time
  frontier.

Delete Journal's duplicated fact-response representation and embedded canonical JSON/base64
roundtrip.
Store constructs the typed response once from qualified producer histories. The strict response
fixes request digest, authored query order, exact selected identities, producer Program/record/head,
canonical subject/response/claim values, and completeness through the captured frontier.

`StateConcluded::Access` retains the accepted fact response/attestation needed for replay. Facts
emitted by the State publish in that same conclusion transaction. Keep preparation selection
frontier and conclusion publication frontier distinct.

### 6.3 Effect-attention projection

Resolve U2 before schema freeze. If enabled, derive attention from selected unresolved Effect
preparations and update it atomically with every head. Listing is one bounded canonical-order
snapshot and creates no run/call authority; selected action requires ordinary qualified resume.

If absent, delete column, partial index, backend method, limits, DTO/API, CLI/REST/operator surface,
docs, and tests together. Do not keep optional or hard-coded-false remnants.

### 6.4 Configuration

Configuration retains its distinct API:

```rust
pub struct ResolvedConfiguration<C: MfmConfig> { /* value, key, head, Store */ }
pub struct ConfigurationWriteSession<C: MfmConfig> { /* affine current owner */ }
pub struct PreparedConfigurationAppend<C: MfmConfig> { /* command + successor */ }
pub struct SuspendedConfigurationAppend<C: MfmConfig> { /* unknown owner */ }
```

`ValidatedConfig<C>` owns typed, source, and retained construction with checked bounds,
canonicalization, and content identity. Local typed construction does not decode; external/row
bytes decode exactly once. `ResolvedConfiguration` adds key/head/Store evidence and supplies a
non-detached admission product.

Writers consume one non-Clone session into a prepared successor. Direct-new advances it with zero
readback; found raw bytes ingress once; stale/conflict/unknown promote nothing; unknown retains the
exact owner. No append loads full history. Independent selection performs one complete bounded
ingress. Ordinary readiness does not scan configuration history.

Configuration is not a fourth run record family. Keep its wire/hash and exact-head domain distinct.
Its final backend append port accepts only a borrowed mechanical command assembled inside Store;
backends cannot mint semantic validity. Land that command, Memory/PostgreSQL parity, exhaustive
fault mapping, and the retargeted `validated_configuration_append_cannot_be_forged.rs` test no later
than Commit 4, before deleting the authority-seal crate. Commit 6 changes only affine writer
promotion and removes remaining reload/readback paths.

## 7. Application, portable export, and EVM cutovers

### 7.1 Fixed-tenant application

Trusted composition consumes one exact Runtime assembly, Store parts product, entry/planning
registries, and Runtime limits into an immutable process. `Application::for_tenant` fixes the
tenant. Multiple qualified processes or assemblies may target the same admitted Store identity;
their immutable catalog/binding associations must match their admitted Programs, and their writes
linearize through Store CAS rather than a shared process registry. Public methods include entry
discovery, admission decode/spawn, resume/drive, public read, replay, trace, access audit, and
export; none accepts a credential or tenant override.

`RunExecutor`, `SuspendedExecution`, and `PendingAdmission` are affine owner wrappers over Runtime
owners. Retryable public enums return the same owner. Delete core/App `drive_once`; one-shot
transports explicitly compose resume then one drive and accept cold-resume cost.

Request bytes remain real ingress. App-owned opaque DTOs have private fields and checked
constructors; REST bounds before allocation, strictly decodes once, and rejects duplicate/unknown/
trailing input. CLI bounds files/stdin/arguments and uses the same domain constructors. No request
contains tenant, credential, principal, grant, or policy fields.

Delete the application access policy, credential/principal/grant typestates, token-file CLI,
bearer/Authorization REST plumbing, policy-only errors/statuses, policy decision persistence, and
recursive source-policy callbacks. Preserve database/provider credentials and protocol
authentication.

### 7.2 Portable export

Use one strict bounded v5 structural stream. Store constructs a consuming non-Clone
`ExportRunEvidence`/`ExportClosure` containing exact three-family frames, same-tenant source
closure,
dense fact routes, Store/epoch fixation, and no Runtime authority or policy evidence. Replay encodes
once and returns whole-stream `ContentRef`.

Offline qualification checks total bound and expected stream ref before record parsing, strictly
decodes v5 only, builds one bounded offline closure, and calls Store's same callback-free qualifier.
It never fills missing sources from live Store and performs no append. App/CLI/REST expose no
persistent import operation.

Delete v4, policy decisions, principals/grants, trust-snapshot callbacks, borrowed encoder views,
parallel offline verifier, and compatibility decoder. Replace observation-era access audit fields
with preparation/supersession/conclusion/integrity status derived from the three-family reducer.

### 7.3 EVM identity, sequential balances, and live adapters

Rename the bounded non-secret caller token to `SubmissionIdempotencyKey`. Derive vNext submission
intent from tenant, wallet nonce domain, and that key. Delete authenticated principal/issuer
namespace fields, constructors, domains, schemas, and decoders. Activate only under U4.

EVM State implementations prepare canonical intents, capture qualified adapters, and execute only
from `CommittedCall`. Adapters retain JSON-RPC authentication, exact request/response binding,
wallet nonce transactions, permanent operation keys, signed bytes, broadcast entry classification,
and convergent status/result evidence. Raw RPC/provider bytes never become capability evidence
without bounded authenticated ingress.

Collapse editable wallet completion/recovery string shadows into one typed terminal/recovery value
with one canonical persisted projection and one cold decode. Keep SQL nonce locks, candidate
activation, provider mutation attestations, and callback-free proof verification where they own
real domain invariants.

Replace EVM balance lane selection and aggregation with a nominal cumulative context:

```rust
struct EvmBalanceContext<K: MfmValue> {
    /* admitted demand + opaque K + bounded opaque result metadata,
       collection/binding, next source,
       current source/stage, chain/anchor/decimal/balance work,
       completed sources, remaining demand */
}

struct EvmBalanceCollectionCompletion<K: MfmValue> {
    caller_continuation: K,
    result: EvmBalanceCollectionResult,
}
```

Private construction/decoding proves that admitted demand equals completed identities plus the
current source, when present, plus remaining demand; position, order, binding, and stage payload
agree. `K` is monomorphized into an exact nominal contract and has no EVM accessor. It is not bytes,
`QualifiedValue`, a reference, or a map.

One bounded `EvmBalanceResultMetadata` (final name fixed by U11) separately carries the caller-owned
collection ordinal and canonical correlation fields required by the existing public EVM result.
EVM may move them unchanged into `EvmBalanceCollectionResult`, but cannot inspect or derive
Portfolio semantics from them; they are not serialized `K` and are allocated exactly once.

The per-source common prefix is `CheckChainIdentity -> ReadInitialAnchor -> Match asset`. The
native arm executes `ReadNativeBalance`; the ERC-20 arm executes `ReadTokenDecimals ->
ReadTokenBalance`. Both return the same observed cumulative-context contract, after which
`ConfirmBalanceAnchor` appends the interpreted completed-source result and advances. A failure
prevents all later State/source work. Chain identity, initial anchor, optional token decimals,
native/token balance, and anchor confirmation are each explicit Read-classified State occurrences;
none is folded into Runtime, Match, or consolidation. Pure `EvmBalanceConsolidation` owns demand/
binding realization, common anchor, duplicate/missing/foreign source rejection, declaration order,
balance construction/mathematics, and returns the nominal `EvmBalanceCollectionCompletion<K>`.
Portfolio
consumes that one typed successor. `EvmBalanceConsolidation` itself constructs the final
`EvmBalanceCollectionResult`; the wrapper only returns opaque `K` beside it. A standalone EVM root
uses a concrete standalone continuation, and its caller-owned Pure projection unwraps the already-
produced result. U11 fixes the exact ownership, caller-correlation representation, and schema names,
allocates every field once, and preserves current public canonical bytes unless it proves and
approves a redundant-field deletion. Only reviewed domain interpretations survive in context; raw
capability evidence does not.

Before `RunAdmitted`, bounded pure planning/child expansion unrolls one declaration-ordered stage
chain per admitted source. Confirmation continues to the statically next expanded source
occurrence; there is no Runtime loop, selector cursor, or dynamic source scheduler.

### 7.4 Sequential Portfolio context

`PortfolioSnapshotContext` contains admitted selector/config/routing, validated selection, next
collection position, completed EVM collections, remaining demand, and final work. The nominal
`PortfolioContinuation` privately owns that complete context and is valid by representation:
admitted collection demand equals completed plus current plus remaining collections, with dense
ordinal, declaration order, selection, and binding consistency. A Portfolio-owned Pure entry State
consumes the current Portfolio context and constructs the complete nominal
`EvmBalanceContext<PortfolioContinuation>` for the selected collection, including the opaque
continuation and EVM-owned source/result metadata. The EVM child consumes that exact singular
fragment-entry value; expansion performs no runtime wrapper transformation. EVM carries the
continuation unchanged and returns it with one `EvmBalanceCollectionResult`; a Portfolio-owned Pure
resume State validates the continuation, appends the result's collection, and advances. The current
Portfolio JSON `caller_context` encode/decode path is deleted; U11 allocates the retained public
caller-correlation field without making EVM interpret Portfolio semantics.

Pure expansion also unrolls one EVM fragment plus Portfolio resume State per admitted collection.
The final Program has no runtime collection loop, collection selector cursor, or dynamic scheduler.

After all collections, Pure `PortfolioConsolidation` owns extraction, validation, totals, and
`PortfolioPublicOutputs`. Preserve the existing EVM collection and Portfolio public wire semantics
unless U11 proves an existing field redundant. Retain an explicit EVM-to-Portfolio failure mapping
only if it is part of the domain failure contract; delete lane-local scopes and collect-all
semantics. A failed collection prevents every later collection preparation/provider call.

## 8. Complete deletion and migration inventory

This is a minimum inventory. Run scoped source/API/schema scans after each vertical change and add
newly discovered shadows.

### 8.1 Packages, features, and dependencies

Delete after responsibility transfer:

- `crates/kernel/spec` / `mfm-spec`;
- `crates/kernel/certify` / `mfm-certify`;
- `crates/kernel/authority-seal` / `mfm-authority-seal`;
- `mfm-runtime/store-authority`, `mfm-certify/runtime-authority`, and every hidden constructor;
- Store-to-Runtime dependency and old Runtime history adapter; and
- all workspace, Cargo.lock, metadata/layer, Nix task, docs, fixture, and dev-dependency references.

Map real seal obligations first: Runtime history -> opaque Store port/active run; export ->
consuming evidence transfer; wallet mutation -> concrete adapter/domain transaction; credential
ingress -> opaque provider product; Runtime composition -> consuming process builder; backend write
-> borrowed sealed command. Empty implementable marker traits do not survive as “authority.”

### 8.2 Five-family execution protocol

Delete in the one vertical execution commit:

- `StateTransitionCommitted`, `ExternalAccessAuthorized`, `ExternalAccessReserved`,
  `ExternalAccessObserved`, journal `RunClosed`, `ObservationOutcome`, and their tags/schemas;
- transition, authorization/reservation, observation, and closure logical keys;
- `AccessReservationEvent`, `AccessObservationEvent`, `ReadReservation`, `EffectReservation`, and
  run-journal reservation entries/frontiers;
- `PreparedReservationAppend`, `CommittedReservation`, `UnresolvedReservationAppend`, reservation
  prepare/commit dispositions, and `ReadyToInvoke`;
- Runtime-phase `AcceptedAccessResponse`, `ErasedAcceptedAccessResponse`, `ReadCompletion`,
  `EffectCompletion`, and wrappers used only to shuttle results through Runtime;
- split request/returned/safe-failure settlement callbacks and selected observation settlement
  actions;
- `ObservationWriteContinuation`, `PreparedObservationAppend`, `RetryObservationPreparation`,
  `ObservationRebaseInput`, `ObservationRebase`, `UnresolvedObservationAppend`, observation
  prepare/commit dispositions, and observation-specific suspensions;
- `PreparedDrive::Observation`, `CommittedDrive::Observation`, `PendingObservationRecovery`,
  post-observation scheduling, and cold observation settlement;
- `CompleteThroughAccessReservationFrontier` and fact `reservation_ref` where they refer to the run
  protocol; and
- every five-family projection, access-audit/portable field, SQL assumption, golden, fixture,
  decoder, feature constructor, alias, fallback, and compatibility test.

Domain wallet/nonce reservation, ordinary provider observations, and protocol authorization remain
when they describe real external concepts. Negative scans must be path-scoped.

### 8.3 Sequential Program, multi-root, and parallel workflow algebra

Delete in the State/Match-only Program cut:

- the multi-root admission contract: repeated `OperationBuilder::input`,
  `AuthoredStructuredProgram::input_roots`, `ExpandedStructuredProgram::input_roots`,
  `NormalizedStructuredProgram::input_root_slot_refs`, `StructuredAdmissionCommand::initial_values`,
  app `PreparedAdmission::initial_values`, `RunAdmitted::initial_bindings`, and Store qualification/
  compiler/reducer `initial_bindings`;
- plural entry and State input encodings whose only purpose is the old binding graph:
  `StructuredEntryPointContract::input_contract_refs`,
  `QualifiedStructuredEntryPointPolicy::public_input_contract_refs`,
  `StructuredPublicContractRefs::input_contract_refs`, `AuthoredStateCall::inputs`, and
  `ExpandedStateBinding::inputs`, `PendingState::inputs`, and
  `SemanticStatePreimage::live_bindings`; replace them with exact singular `C0`/`S::Input` fields
  and fresh strict identities rather than accepting length-one vectors;
- the ordered child-input contract: `Value::bind_child`, `ChildInputBinding`,
  `FragmentInputBinding`, `AuthoredOperationCall::input_bindings`,
  `PendingChild::input_bindings`, `ExpandedFragment::input_bindings`, `initial_bindings(...)`, and
  child/capability/policy root/boundary-bijection substitution machinery and errors; child
  authoring consumes only the current `Value<Cn>` and expansion substitutes it as the exact
  singular fragment-entry value;
- old authored/expanded Program, entry-point, policy, certified-root, admission, and journal wire
  identities containing `input_roots`, `input_root_slot_refs`, plural `input_contract_refs`,
  `initial_values`, or `initial_bindings`; strict ingress rejects zero/multiple-root and plural
  encodings with no compatibility decoder; and

- `FanOutBuilder`, `FanOutResults`, `FanOutVisitor`, `AllowsFanOut`, `FanOutOneRemaining`,
  `FanOutAtLimit`, and all `.fan_out`/lane authoring methods and policies;
- `AuthoredFanOut`, `AuthoredFanOutLane`, `ExpandedFanOut`, `ExpandedFanOutLane`, and authored/
  expanded declaration variants, tags, schemas, codecs, fixtures, and document identities;
- `ExpandedFragment`, `ExpandedDeclaration::Fragment`, `FragmentInputBinding`, and every expanded
  fragment boundary retained in the final Program; fragments remain authoring-only and pure
  expansion must erase them;
- `LaneOutcome`, `LaneOutcomeContract`, `FanOutJoinContract`, `NonEmptyFanOutJoin`, lane/join value
  definitions, contract-ref helpers, canonical JSON helpers, and persisted object handlers;
- the old slot/provenance graph: `LexicalProducer`, `ExpandedLexicalProducer`, `LexicalSlot`,
  `ExpandedLexicalSlot`, `LexicalValueRef`, `StructuralValueOrigin`, their path/slot refs, maps,
  definitions and wire identities, plus `SlotBindings`, `ReducedRunState::bindings`,
  `UnboundSuccessor::bindings`, `Engine::bindings`, `typed_binding`, `require_binding`, and
  `merge_bindings`. The sequential authoring pipeline carries one current typed `Value<C>`; Match
  uses that exact value rather than an ambient lexical binding graph;
- `LaneCursor`, `ProgramCursor::InFanOut`, `WorkCursor::InFanOut`, `walk_fan_out`, `bind_lane`,
  `collect_lane_actions`, nested/actionable lane collections, completion barriers, and join
  synthesis;
- `StructuredFrontier::Actions(Vec<ActionableState>)`, `frontier_actions`, and
  `collect_work_actions`; replace them with one singular selected-action frontier and update Runtime
  `structured.rs` plus Store `purpose.rs` in the same cut;
- `MAX_STRUCTURED_LANES`, `MAX_FAN_OUT_DEPTH`, `max_lanes`, `max_fan_out_depth`, fan-out depth/count
  profiles, effect/nesting policies, lane-local failures, collect-all behavior, and concurrent
  conclusion reservation; and
- old strict Program/path/slot/profile identities whose shape admitted FanOut or retained expanded
  fragments;
- fan-out-specific terminality, cold-ingress qualification, portable closure, and structural audit
  rules. Final Program/history bytes receive fresh State/Match-only identities; no old decoder
  remains.

Do not introduce `CollectBuilder`, `GatherBuilder`, `state_many`, authored/expanded workflow
Collect/Gather/Join variants, lanes, workflow barriers, an ambient output map, a generic
heterogeneous run context, or structural-sharing/checkpoint optimization. Exact negative scans are
path-scoped: iterator `.collect()`, domain `*Collection`, SQL joins, actor shutdown joins, fact-
selection barriers, fact producers, and backend concurrency remain legitimate.

Delete EVM/Portfolio fan-out construction and data:

- `EVM_BALANCE_PROGRAM_LANE_LIMIT`, `EvmBalanceLaneInput`, `EvmBalanceLaneCursor`,
  `EvmBalanceLaneResult`, `BalanceLaneJoin`, `PortfolioCollectionJoin`;
- first/next selector, input-exhaustion, and lane-aggregation States and their schemas/ids;
- `author_evm_balance_lane_selection`, `author_evm_balance_fan_out`, its mapped-scope helper,
  `structured_portfolio_lane_inputs` and its input helper, `aggregate_lanes`,
  `AggregatePortfolioCollectionsState`, `aggregate_portfolio_collections`, grouped lanes, and
  caller-context JSON echo/decode including `PortfolioSnapshotInput::canonical_json` and
  `PortfolioSnapshotInput::decode`;
- old non-cumulative `BootstrapBalanceWork`, `AnchoredBalanceWork`, `TokenBalanceWork`, and
  `ObservedBalanceWork` schemas, unless Commit 0 explicitly proves and renames a value as one
  private stage payload inside the cumulative context; and
- `balance-fan-out`, `portfolio-network-fan-out`, `evm-source-fan-out`, lane/source labels, and
  fan-out-specific failure mapping.

Replace only the surviving domain semantics with the §7.3/§7.4 cumulative States and Pure
consolidations. Preserve `EvmBalanceCollectionResult` and ordinary collection terminology.

### 8.4 Generic live-currentness and policy

After U5 maps immutable survivors, delete generic refresh modes/evidence, resource-currentness
contracts, current physical binding selection, release histories/promotions, minimum lineage,
supersession-before-entry, generation-guarded signer/provider wrappers, per-call target-currentness,
target-authority table, and no-op resource invokers.

Preserve Store writer epoch/durability and exact-head/fact transaction
preconditions, provider protocol
authentication and inventory proofs, immutable target/signer identity, wallet nonce locks,
permanent operation keys, SQL transaction permits, database roles/grants, keystore qualification,
AAD, zeroization, and constant-time comparisons.

Delete application credentials, policy types, principals, grants, decision refs, policy-only public
errors, CLI token files, REST bearer logic, portable policy evidence, and authenticated EVM issuer
identity. Do not replace them with inert optional fields.

### 8.5 Store/Runtime file ownership

Target Store changes:

| Current surface | Final action |
| --- | --- |
| Runtime history adapter | delete; Store owns opaque ports/actions |
| reducer | three events, selected preparation/replacement/conclusion lattice |
| compiler/comparison | one binder; delete comparison typestates and local requalification |
| qualification | one complete-prefix/found-attempt ingress; no callback/currentness verifier |
| obligation wrapper | keep fact/capacity/binding obligations directly; delete wrapper |
| coordinator | owner-bound prepared append and generic pending conclusion recovery |
| validated append seals | replace with private prepared append + borrowed backend command |
| purpose readers | borrow `QualifiedRun`; audit or consume export evidence |
| fact scanner | preparation-bound continuation and scope-local producer memo |
| configuration | distinct affine typed writer/reader |
| semantic open | infrastructure-only open plus explicit audit; no eager history sweep |

Target Runtime changes:

- replace old `Prepared`/`Authorized`/observation/settlement lifecycle with selected action,
  `PreparedExecution`, `CommittedCall`, `HandlerResolution`, Runtime `PendingConclusion`, and
  Store `PreparedConclusion`;
- retain immutable process registry, Runtime brand, deterministic selection, panic containment,
  redacted error attribution, bounded work, and session ownership;
- delete pre/post-drive verified reloads, automatic stale continuation, cloneable verified-run
  facades, and public invocation-package constructors; and
- ensure no production feature can mint or enter a provider without a committed call.

### 8.6 Documentation and contract inventory

Update with owning commits:

- `docs/design.md`, `docs/architecture.md`, and `docs/run-execution.md`;
- `docs/known-gaps.md` and persisted/public surface documentation;
- root, Program, Capabilities, Store, Runtime, Facts, Replay, App, signing, keystore, EVM, CLI, and
  REST READMEs;
- EVM routing/transaction and portfolio docs;
- PostgreSQL migration/catalog/SQL inventory docs; and
- metadata/layer/Nix task contracts where packages/tasks change.

The final authoritative docs must stand alone. Git history is the archive; do not retain old
protocol descriptions as legacy appendices.

### 8.7 Exact negative manifest and named migration owners

Maintain one path-scoped scanner manifest, including exact Serde snake-case spellings and SQL
identifiers. Commit 0 must enumerate every match in the cutover-base tree, assign every survivor or
deletion, and check the complete resulting literal/path manifest into the repository; category
examples are not a substitute. It scans production, tests, fixtures, and documentation; it excludes
only this RFC, this plan, and the scanner manifest itself. Hostile fixtures assemble banned literals
from fragments or use exact fixture-only allowlists. The mandatory seed includes:

**Superseded run lifecycle and callback algebra**

```text
StateTransitionCommitted
ExternalAccessAuthorized
ExternalAccessReserved
ExternalAccessObserved
RunClosed
ObservationOutcome
AccessAuthorizationProposal
AccessReservationEvent
AccessObservationEvent
PreparedReservationAppend
CommittedReservation
UnresolvedReservationAppend
ReadyToInvoke
AcceptedAccessResponse
ErasedAcceptedAccessResponse
ReadCompletion
EffectCompletion
SafeFailureDisposition
AuthorizationIntent
PrimaryIntent::Authorization
ExpectedAuthorization
AuthorizationEntry
CertifiedAccessAuthorization
CommittedAccessAuthorization
AuthorizedCallOrigin
AuthorizedProviderCall
QualifiedRuntimeIntent::Authorization
Authorized<K, V>
ObservationWriteContinuation
PreparedObservationAppend
RetryObservationPreparation
ObservationRebaseInput
ObservationRebase
UnresolvedObservationAppend
PendingObservationRecovery
RecordLogicalKey::Authorization
RecordLogicalKey::Reservation
RecordLogicalKey::Observation
RecordLogicalKey::Transition
RecordLogicalKey::Closure
external_access_authorized
external_access_reserved
external_access_observed
state_transition_committed
run_closed
authorization_ref
observation_ref
reservation_ref
```

**Old live callbacks, currentness, and authority seals**

```text
ComponentFuture
ReadAdapterCompletion
EffectAdapterCompletion
EffectContractCompletion
ReadCapabilityImplementation
EffectCapabilityImplementation
ReadAdapterInvoker
EffectAdapterInvoker
ErasedStateCallbacks
ProcessHandle
BoundedComponentContract
BoundedComponentInvoker
SignerContract
ResourceAuthorityContract
SigningCapability
RuntimeHistoryPortSeal
PhysicalBindingVerifierSeal
RetainedPhysicalReleaseTrustSeal
StoreLineageTrustSeal
ExportEncoderConsumerSeal
WalletNonceAuthoritySeal
DeploymentCredentialSinkSeal
DeploymentCredentialBrokerSeal
RuntimeAssemblyConsumerSeal
ValidatedAppendConsumerSeal
NoRefreshEvidence
EffectRefreshMode
EffectCapabilityContract::Refresh
RuntimeEffectRefreshBinding
NoRefreshBinding
RefreshableBinding
RuntimeResourceAuthority
StructuredEffectRefreshContract
RetainedPhysicalReleaseTrust
PhysicalBindingAuthorization
PhysicalBindingSupersession
PhysicalObligationChecker
SemanticObligation::PhysicalAuthorization
SupersededBeforeEntry
minimum_lineage_head_ref
stable_resource_lineage_contract_ref
GenerationGuardedDeterministicSigningProvider
SigningGenerationGuard
VerifiedGenerationGuardedSignerBinding
QualifiedReadSigningProvider
GenerationGuardedSignerDescriptor
sign_guarded
verify_current_and_exclusive
is_read_attestation_eligible
verify_read_attestation_qualification
durable_generation_ref
fence_attestation_ref
direct_sign_exclusion_ref
SigningGenerationGuardError
GenerationMismatch
GenerationGuardUnavailable
GenerationFenced
DirectSigningOverlap
ReadAttestationIneligible
mfm.signing.generation-guarded-signer-descriptor
EvmRoutingGenerationDescriptor
provider_fence_head_ref
current_public_lineage_head
QualifiedWriterProcessLease
RunBusy
ExecutionGuardRegistry
SpawnStep::Busy
ResumeStep::Busy
```

**Application policy and EVM issuer identity**

```text
SecretCredential
SecretCredentialError
MAX_SECRET_CREDENTIAL_BYTES
ApplicationAccessPolicy
ApplicationAccessGrant
AccessTarget
AuthorizedTenant
AccessPolicyError
AuthorizedRunCall
AuthorizedAdmissionCall
AuthorizedExportDecision
RunGrantMarker
run_grant
authenticated_principal_id
authorization_decision_ref
PortableAuthorizationDecision
AuthorizedExportClosure
authorized_closure_digest
PORTABLE_EXPORT_GRANT
export_decisions
validate_export_decisions
from_authorized_export_closure
authorized_sources
authorized_source_prefixes
with_authorized_sources
access_token_file
--access-token-file
bearer_credential
MAX_BEARER_BYTES
MAX_ACCESS_TOKEN_BYTES
AuthenticationRequired
GrantDenied
SourceRunExportDenied
ErrorClass::Unauthorized
ErrorClass::Forbidden
EvmCallerSubmissionToken
EVM_CALLER_SUBMISSION_TOKEN_MAX_BYTES
caller_submission_token
caller-submission-token
mfm.evm.caller_submission_token
AuthenticatedIntentIssuerId
IntentIssuerPreimage
derive_authenticated_intent_issuer_id
issuer_namespace_contract_ref
mfm.evm.intent-issuer.v1
from_authorized
```

**Program, fact, portable, package, and feature shadows**

```text
certified_program_ref
certified_program_refs
producer_certified_program_ref
PriorRunFactScannerBindingCertificate
mfm.prior-run-fact-scanner-binding
structured.prior_run_fact_scanner_binding
PriorRunFactSelectionResponse
PriorRunFactQueryResult
PriorRunFactScanAttestation
PriorRunFactCompletenessMode
CompleteThroughAccessReservationFrontier
CompleteThroughAuthorizationFrontier
canonical_response_base64url
canonical_response_json
FactSelectionReadResponse::from_canonical_json
stable_resource_lineage_contract_refs
ReplayEncoderConsumer
ReplayTrustSnapshot
RetainedPhysicalReleaseTrust
StoreLineageTrust
OfflineRunClosure
OfflineVerifiedRun
RecordedRunEvidence
ReplayRunReader
load_for_recorded_verify
ExportEncoderView
ExportEncoderSource
with_encoder_view
verify_offline_run_closure
PortableRunExport::verify_offline
verify_with_trust
strict_decode
Application::import
import_run
ImportRunRequest
ImportRunResult
mfm-spec
mfm-certify
mfm-authority-seal
runtime-authority
store-authority
RuntimeHistoryPort
VerifiedRunView
drive_once
```

**Plural admission and ordered child-input bindings**

```text
OperationBuilder::input
AuthoredStructuredProgram::input_roots
ExpandedStructuredProgram::input_roots
NormalizedStructuredProgram::input_root_slot_refs
input_root_slot_refs
StructuredAdmissionCommand::initial_values
PreparedAdmission::initial_values
RunAdmitted::initial_bindings
StructuredEntryPointContract::input_contract_refs
QualifiedStructuredEntryPointPolicy::public_input_contract_refs
StructuredPublicContractRefs::input_contract_refs
QualifiedAdmission::initial_bindings
AuthoredStateCall::inputs
ExpandedStateBinding::inputs
PendingState::inputs
SemanticStatePreimage::live_bindings
Value::bind_child
ChildInputBinding
FragmentInputBinding
AuthoredOperationCall::input_bindings
PendingChild::input_bindings
ExpandedFragment::input_bindings
initial_bindings
child_root_id
child_contract_ref
SlotBindings
ReducedRunState::bindings
UnboundSuccessor::bindings
Engine::bindings
typed_binding
require_binding
merge_bindings
exact ordered input-root bijection
mfm.structured-entry-point-contract
mfm.qualified-structured-entry-point-policy
mfm.certified-program-root
```

The manifest uses path-qualified checks for generic field/function spellings such as
`input_contract_refs`, `initial_values`, `initial_bindings`, and `inputs`; unrelated domain
collections are not banned. Commit 0 adds the exact old versions of the authored/expanded Program,
certified-root, admission, and journal identities that encode these plural fields.

**FanOut, lane, join, and rejected workflow replacements**

```text
FanOut
AuthoredFanOut
AuthoredFanOutLane
ExpandedFanOut
ExpandedFanOutLane
AuthoredDeclaration::FanOut
ExpandedDeclaration::FanOut
ExpandedFragment
ExpandedDeclaration::Fragment
FragmentInputBinding
FanOutBuilder
FanOutResults
FanOutVisitor
AllowsFanOut
FanOutOneRemaining
FanOutAtLimit
LanePolicy
LaneOutcome
LaneOutcomeContract
FanOutJoinContract
NonEmptyFanOutJoin
StructuredValueDefinition::NonEmptyFanOutJoin
StructuralPathSegment::FanOutLane
StructuralValueOrigin::FanOutLane
LexicalProducer::LaneOutcome
LexicalProducer::FanOutJoin
ExpandedLexicalProducer::LaneOutcome
ExpandedLexicalProducer::FanOutJoin
LexicalProducer::FragmentInput
LexicalProducer::FragmentBoundary
ExpandedLexicalProducer::FragmentInput
ExpandedLexicalProducer::FragmentBoundary
LexicalProducer
ExpandedLexicalProducer
LexicalSlot
ExpandedLexicalSlot
LexicalValueRef
StructuralValueOrigin
LaneCursor
ProgramCursor::InFanOut
WorkCursor::InFanOut
walk_fan_out
bind_lane
collect_lane_actions
StructuredFrontier::Actions
frontier_actions
collect_work_actions
MAX_STRUCTURED_LANES
MAX_FAN_OUT_DEPTH
max_lanes
max_fan_out_depth
fan_out_join_contract_ref
fan_out_join_contract_canonical_json
lane_outcome_contract_ref
lane_outcome_contract_canonical_json
non_empty_fan_out_join
declaration_ordered_lane_slots
structured.lane_outcome_contract
structured.fan_out_join_contract
LANE_OUTCOME_CONTRACT_OBJECT_TYPE
FAN_OUT_JOIN_CONTRACT_OBJECT_TYPE
mfm.spec.lane-outcome-contract
mfm.spec.fan-out-join-contract
fan_out
fan_out_lane
lane_outcome
fan_out_join
fragment_input
fragment_boundary
ExpandedDeclaration kind=fragment
CollectBuilder
GatherBuilder
state_many
mfm.spec.expanded-lexical-slot-definition
mfm.structured-lexical-slot
mfm.spec.expanded-lexical-producer
mfm.spec.expanded-path-ref
mfm.spec.expanded-slot-ref
mfm.authored-structured-program
mfm.expanded-structured-program
mfm.spec.structural-path-segment
mfm.structured-path
mfm.spec.expanded-structural-path-definition
mfm.structured-expansion-profile
mfm.spec.certified-structural-bounds
```

Also reject authored/expanded workflow `Collect`, `Gather`, `Join`, lane, or barrier declaration
variants and every exact old Serde spelling found by Commit 0. The scanner must not ban ordinary
`.collect()`, domain collections, SQL/OS-thread joins, or synchronization/fact-selection barriers.

**EVM and Portfolio fan-out identities**

```text
EVM_BALANCE_PROGRAM_LANE_LIMIT
EvmBalanceLaneInput
EvmBalanceOperationInput
EvmBalanceLaneCursor
EvmBalanceLaneResult
SelectFirstBalanceLaneState
SelectNextBalanceLaneState
AssertBalanceInputExhaustedState
AggregateBalanceLanesState
AggregatePortfolioCollectionsState
BalanceLaneJoin
PortfolioCollectionJoin
author_evm_balance_lane_selection
author_evm_balance_fan_out
author_evm_balance_fan_out_in_mapped_scope
structured_portfolio_lane_inputs
structured_portfolio_lane_inputs_from_input
aggregate_lanes
aggregate_portfolio_collections
PortfolioSnapshotInput::canonical_json
PortfolioSnapshotInput::decode
BootstrapBalanceWork
AnchoredBalanceWork
TokenBalanceWork
ObservedBalanceWork
mfm.evm.structured_balance_lane_input
mfm.evm.structured_balance_lane_cursor
mfm.evm.structured_balance_lane_result
mfm.evm.structured_balance_operation_input
mfm.evm.structured_bootstrap_balance_work
mfm.evm.structured_anchored_balance_work
mfm.evm.structured_token_balance_work
mfm.evm.structured_observed_balance_work
mfm.evm.state/structured-select-first-balance-lane
mfm.evm.state/structured-select-next-balance-lane
mfm.evm.state/structured-assert-balance-input-exhausted
mfm.evm.state/structured-aggregate-balance-lanes
select-balance-lane
assert-balance-input-exhausted
balance-source
balance-fan-out
portfolio-network/
portfolio-network-fan-out
evm-source-fan-out
```

Commit 0 inventories and replaces the enclosing FanOut-capable Program/path/slot/profile identities,
including current authored/expanded Program documents, structural path/lexical producer definitions,
expansion profile, and certified bounds. Fresh final identities admit only State/Match.

**Superseded PostgreSQL/catalog shapes**

```text
mfm.structured-run-history-postgres.v7
StoredBatchEnvelope
StoredObjectRow
run_history_batch_objects
structured_insert_object_rows
insert_object_rows
load_object_rows
load_object_rows_through
group_object_rows
batch_envelope_json
run_history_batches.candidate_digest
run_history_batches.predecessor_sequence
run_history_batches.predecessor_commit_digest
target_authority
has_effect_entry_attention
```

The scanner supplements, rather than replaces, Rust API/Trybuild and strict wire/schema tests. It
must inventory-generate additions for every U5 binding/currentness field and every newly discovered
alias, feature, SQL view, DTO, or fixture shadow. Do not blanket-ban ordinary domain words such as
authorization, observation, reservation, credential, closed, lease, or fence.

Named migration owners that must be rewritten or deleted at their owning commit include:

- `tests/integration/tests/cargo_metadata_contract.rs`, its `nixfied.nix` task, and layer metadata;
- `access_error_contract.rs`, `transport_surface_contract.rs`, `run_identity_contract.rs`, and
  `legacy_surface_contract.rs` (the replacement scanner must include tests/fixtures/docs);
- `crates/app/tests/application_privacy_ui.rs` and its auth/currentness Trybuild fixtures;
- Journal `structured_tests.rs` and `structured_record_families.rs`; Facts response tests; Store
  `structured_runtime_causal.rs`, `structured_runtime.rs`, `api_surface.rs`,
  `structured_history_qualification.rs`, and `export_source_closure_unit.rs`;
- Memory/PostgreSQL fact, configuration, history, crash, schema, and SQL-inventory suites;
- `tests/integration/tests/replay_wire_contract.rs`, portable v4/import fixtures, and
  `tests/wallet-authority-provider` goldens;
- `validated_configuration_append_cannot_be_forged.rs`, retargeted to the final borrowed command
  and affine writer owner; and
- root/persisted-surface/routing/transaction/portfolio docs, every affected crate/binary README,
  and CLI/REST schemas/examples.

Singular-`C0` owners specifically include Program `structured.rs`, Spec `structured.rs`, Certify
`structured.rs`, Runtime history `commands.rs`, Journal `structured.rs`, Store
`qualification.rs`/`compiler.rs`/`reducer.rs`, App `production_structured.rs`, EVM
`structured_balance.rs` and `submission_expansion.rs`, and Portfolio `structured.rs`. Replace the
Program multi-root authoring/normalization tests, Certify's exact-root-bijection tests, journal
`initial_bindings` fixtures, and the forged-child-input Trybuild pair with singular construction,
exact-current-context child substitution, hostile plural-wire rejection, and hot/cold `C0`
identity tests.

FanOut-specific named owners additionally include:

- Program/spec/certification structured sources and tests; Journal structural origin; Runtime
  history cursor; Store reducer, qualification, test support, causal and source-closure tests;
- EVM `structured_balance`, exports/submission tests, live registration, and README; Portfolio
  structured authoring, operation tests, README, and `docs/portfolio-snapshot.md`;
- root README, `docs/design.md`, `docs/architecture.md`, `docs/run-execution.md`, and
  `docs/persisted-public-surfaces.md`; and
- PostgreSQL structured-history/profile fixtures and the application production expansion profile.

Delete rather than rewrite the `effect_in_fan_out`, `proceed_in_fan_out`, and
`third_level_fan_out` Trybuild source/`.stderr` pairs, plus FanOut-only nonempty/depth, lane-origin,
join-contract, completion-permutation, nested/hidden-Effect, and collect-all tests. Mixed nominal/
failure fixtures retain only sequentially relevant coverage.

## 9. Ordered implementation and logical commits

Every subject is lower case. Every commit includes code, contracts, schemas, fixtures, tests, and
docs. Commits form one migration train and activate no retained production identity until final
verification.

### Commit 0 — `resolve prepared-call execution gates`

Record U1–U11 decisions, cutover-base commit, the complete production `C0 -> Cn -> final result`
table, Match convergence and explicit failure-context routes, the EVM/Portfolio typed continuation,
singular admission/child context ABI, CAS worker-race contract, numeric limits, cumulative-context
benchmarks/spike conclusions,
capability evidence/absorption table, immutable binding disposition, dormant-run policy, EVM
activation, and PostgreSQL topology. Check in the exhaustive §8.7 cutover manifest, including
plural admission/child bindings, rejected guard/lease/Busy surfaces, FanOut/Collect, and every
other deletion category; remove every conditional/rejected design branch.

Do not commit disposable spike code or placeholder abstractions. Verification: links/claims,
recorded measurements, and `git diff --check`.

### Commit 1 — `scope application facades by tenant`

Atomically:

1. fix `Application` to one tenant and enforce partition-first Store/PostgreSQL lookup;
2. delete caller policy/credential/principal/grant APIs and persistence;
3. delete CLI token-file and REST bearer/401/403 behavior; document trusted exposure;
4. disable the pre-cutover portable export/import surface after deleting policy evidence; final
   structural v5 is introduced only with three-family frames in Commit 4;
5. cut EVM identity to tenant + nonce domain + idempotency key; and
6. stage fresh Store/wallet/sender activation without deploying it.

This commit may retain the one current five-family execution protocol; it introduces no second
one. Verify affected App/Replay/EVM/binaries, DB schemas/goldens, metadata if manifests change, and
fixed-tenant isolation.

### Commit 2 — `make program and reduction sequential by construction`

In one compiling schema cut, land opaque Program/value/catalog/compiler authority, State/Match-only
documents, singular `C0`, cumulative-context contracts, mode-neutral terminal-root certification,
typed single-context child expansion, and
callback-free State declarations. Install the minimal sequential cursor in every schema-coupled
Journal/Store/Runtime consumer and migrate EVM/Portfolio authoring to cumulative contexts. Delete
FanOut/lanes/joins, expanded fragment variants, profiles, schemas, walkers, UI fixtures, and old
Program identities across Spec/Program/Certify/Journal/Store/Runtime/domains; absorb Spec and
callback-free Certify work, delete `mfm-spec`, and retain no compatibility Program/cursor.
The singular-`C0` cut also updates entry-point contracts, App admission commands, Runtime admission
owners, the current coherent `RunAdmitted` field/schema, and Store qualification/compiler/reducer;
no length-one `input_roots`, `initial_values`, or `initial_bindings` container survives this commit.
Commit 4 replaces that coherent five-family `RunAdmitted` with the final three-family identity while
preserving the singular `C0` value/contract.
This commit lands the final EVM/Portfolio cumulative Input/Output/Failure value schemas, authoring,
and callback implementations under the still-current access ABI; those cumulative value identities
do not change again in Commit 4.

The final execution-specific Intent/Evidence portion remains in Commit 4. The Commit-0 U5/U8/U11
rulings freeze descriptors and context identities. Verify State/Match-only hostile ingress,
context/Match continuity, hot/cold Program equivalence, one invariant pass, expansion, value
construction, catalog transposition, sequential EVM/Portfolio authoring, UI deletion, metadata, and
affected dependents.

### Commit 3 — `make store ingress and appends one semantic path`

Land prerequisites that remain coherent under the already-sequential current execution protocol:

1. invert Store/Runtime dependency and move callback-free semantics to Store;
2. move the already-current sequential cursor under Store ownership and introduce
   `QualifiedRun`/`ActiveQualifiedRun`, one reducer/binder, singular reservation, and owner-bound
   appends;
3. delete dual reducers, semantic comparison, local requalification, and positive echo;
4. retain the unchanged pre-cutover mechanical backend/schema behind that one semantic owner;
5. land typed configuration read ingress and planning products;
6. make Facts own its strict typed selection response and retain dense completeness; and
7. retain exactly the one old access protocol until Commit 4, with no new observation abstractions.

Commit 0 must have proven these exact prerequisites leave the one five-family protocol coherent.
Anything that did not pass that proof is already assigned to Commit 4; implementation does not
make this choice conditionally. The complete-frame Store/backend SPI, Memory/PostgreSQL storage,
U2 attention slice, final v8 schema, selected-head constraint, record families, and projections all
land only in Commit 4, so no interim storage bridge or second v8 cut exists.

Verify reducer/binder hot-cold equivalence over the retained backend, facts, configuration reads,
metadata, and maximum-prefix semantic benchmarks.

### Commit 4 — `execute committed preparations and conclude states atomically`

This is one inseparable vertical cut across:

- final State/Capability Intent/Evidence ABI and State implementation registration;
- final access-State Program identities, rejecting any intermediate access-contract identity from
  Commit 2 while preserving its cumulative Input/Output/Failure value identities;
- Journal three-family wire, logical keys, refs, schemas, and goldens;
- Store selected-preparation reducer, capacity, facts, purpose projections, and
  `PreparedConclusion`;
- Runtime `PreparedExecution -> CommittedCall`, call correlation, adapter ingress, closed result,
  outer `PendingConclusion`, owner-carrying preparation/conclusion recovery, and sessions;
- Facts attestations and direct-new one-use continuation;
- Replay/access audit/portable three-family projections;
- final strict portable v5 encode/verify with no v4 decoder or persistent import;
- complete-frame Store/backend SPI and Memory/PostgreSQL v8 schema, transactions, terminal/U2-
  attention projections, durability/crash qualification, bounds, and hostile checks;
- App, EVM, live adapters, wallet/signing/keystore assembly, fixtures, and integration tests;
- final sequential EVM/Portfolio live execution registration/ABI, adapters, and public-result
  equivalence using the cumulative value identities already fixed in Commit 2;
- the final borrowed configuration backend command plus Memory/PostgreSQL parity and forgery tests
  required before authority-seal deletion;
- deletion of remaining Certify/authority-seal/currentness surfaces; and
- design, architecture, execution, persisted-surface, crate, and deployment docs.

Do not commit while an old durable safety check is bypassed rather than replaced, while a five-
family reader/writer remains, or while any compatibility journal/portable/persistence tag/alias/
decoder exists.
Old FanOut-capable Program identities were already rejected in Commit 2.

Verify the full preparation, invocation, evidence, conclusion, recovery, capacity, fact, replay,
secret, binding, PostgreSQL, EVM, and negative-source matrices in §10.

### Commit 5 — `qualify retained history only when consumed`

Remove eager run/configuration readiness enumeration. Unify Runtime resume, public/trace/access-
audit reads, Replay/export/offline qualification, explicit audit, and selected fact producers behind
the one complete-prefix qualifier. Retain only request/session-scoped prefix reuse. Integrate the
chosen attention listing through ordinary qualified resume.

Verify zero startup/unrelated loads, malformed dormant isolation, selected dependency failure,
scope-local reuse, fresh-scope re-ingress, fixed-snapshot audit, and no cache/checkpoint remnants.

### Commit 6 — `make configuration writes one semantic path`

Complete the distinct affine configuration writer over the borrowed backend command already landed
in Commit 4: typed session, prepared successor, direct-new promotion, found/stale/conflict/unknown
owner handling, total bounds, and no reload/readback/cache/suffix path.

Verify typed/source/retained construction, exact predecessor/content, bounds, direct zero readback,
ambiguity, transposition, secret exclusion, and Memory/PostgreSQL parity.

### Final verification and activation

After focused failures are resolved:

1. run `nix run .#model-check` on the final revision if the task graph changed;
2. run `nix run .#ci` exactly once; do not immediately precede it with redundant component gates;
3. run plain `git diff --check` before each commit and range diff-check from Commit 0's base;
4. inspect clean status and commit subjects;
5. generate the final delta/report in §11; and
6. prove the final exact §8.7 cutover manifest empty outside its scoped rejection/allowlist cases,
   including plural-C0, guard/lease/Busy, and FanOut/Collect categories, and report product LOC/
   public-type reductions; and
7. only then activate fresh Store/wallet/domain identities under U4/U10.

## 10. Verification specification

### 10.1 Test-only counters and model harness

Add scoped `cfg(test)` counters at actual owners for:

- Program invariant passes, ingress, and expansion;
- cumulative-context construction, hot typed handoff, cold decode/bind, and Runtime downcast;
- cumulative-context ownership moves/clones selected by U11, with no implicit duplication;
- prefix/frame reads, record decode, reducer, binder, and post-commit reads;
- Pure evaluation and access intent preparation;
- `StateImplementation::execute`, provider entry, adapter ingress, evidence binding, State
  interpretation, and conclusion
  canonicalization;
- prior-fact request/scan/producer folds and fact publication;
- configuration ingress/folds/append-time reads/readbacks; and
- startup run/configuration enumeration;
- selected actionable occurrences, competing-worker candidates, direct-new winners, per-owner
  session/call/pending high-water, and permitted replacement overlap; and
- ordered EVM source/stage and Portfolio collection traces.

Counters are per test Store/Runtime and do not become production authority. Add a small model/
property harness enumerating admission, Pure, preparation, replacement, conclusion, competing-head
classification, fact movement, unknown acknowledgement, terminality, cumulative context links,
Match choice, fail-fast routing, and hostile late results.

### 10.2 Type and API exclusions

Trybuild/API tests prove downstream code cannot:

- construct, mutate, deserialize, or execute `Program`/`ProgramRef`/`ProgramDocument` incorrectly;
- register Pure/Read/Effect callbacks or adapters under the wrong mode/capability/binding;
- use one nominal capability type as both Read and Effect, manually implement the evidence marker,
  or bypass exact catalog evidence registration;
- construct/mutate/deserialize `QualifiedRun`, or construct/clone `ActiveQualifiedRun`,
  `RunSession`, `PreparedExecution`, `CommittedCall`, accepted call wrappers,
  `PendingConclusion`, fact continuation, or Runtime brand; callback-free `QualifiedRun` alone may
  be cloned;
- serialize, deserialize, copy, default, reuse, or consume twice any affine execution owner,
  accepted wrapper, pending conclusion, or fact continuation;
- extract a live invoker/adapter call from Program, Store, Replay, or assembly;
- construct `CommittedCall` from a Store token, raw backend outcome, found-same history, cold
  preparation, head, ref, or another call;
- construct a free accepted evidence/conclusion pair or transpose correlation;
- construct `StatePrepared`, `StateConcluded`, record envelopes, occurrence/preparation refs,
  append ids, or publication coordinates from State implementation code; implementations return
  only typed intent, outcome, and fact proposals;
- obtain `RunHistory`, `QualifiedRun`, Store/reducer/cursor/journal readers, an ambient output map,
  or arbitrary prior values through supported State constructors/callbacks; ordinary domain crate
  dependencies expose none of those owners (captured ambient handles remain a trusted-code
  violation, not a sandbox claim);
- construct an authored/expanded FanOut/Collect/Gather/workflow Join declaration or any retained/
  expanded Fragment, lane, or barrier; call `.fan_out`/`state_many`; or use an old Program/profile/
  schema identity (authored reusable fragments remain valid before pure expansion);
- declare a second admission root; construct an authored/expanded/certified/admission/journal form
  with zero or multiple roots; or supply an ordered child-input binding collection;
- promote an uncommitted successor or split one affine worker owner into a second session beside
  its call/conclusion owner; or
- enable a Cargo feature exposing any authority mint.

Use the existing Trybuild owner and serialized Nextest group; update checked `.stderr` fixtures.

Assembly behavioral tests build one exact-complete catalog/implementation/adapter association and
then independently inject missing, surplus, ambiguous, wrong-mode, wrong-catalog, wrong-provider,
wrong-target, wrong-signer, wrong-binding, and wrong-effect-domain registrations. Every invalid
`RuntimeAssemblyBuilder::finish` fails before facade/session/call authority exists and leaves all
State/adapter/provider counters zero.

### 10.3 Wire, reducer, and capacity matrix

Journal unit/integration and Store tests cover:

- exactly three strict families/keys and one semantic record per append;
- separate admission and H1/H2/H3 goldens with no adjacent closure;
- admission direct-new, found-same, found-different, unknown -> found-same, and serially proven-
  absent retry, with zero State/adapter/provider callbacks on every branch;
- distinct-physical-id admission races: one direct-new; `StaleHead` losers semantically compare
  the qualified existing `RunAdmitted` and return recorded Active/Terminal when exact-same,
  conflict when different, and integrity failure when invalid; raw `Found` remains same-id only;
- unknown -> proven-absent resubmission -> `StaleHead` performs that same semantic comparison once,
  returns qualified history/conflict/integrity as appropriate, and neither calls back nor resubmits;
- zero-state admission terminality returns the exact admitted `C0` contract/content identity;
  a different same-contract value, different contract, or extra root object rejects;
- zero-state requires an empty expanded control form; Match-only and selected empty-arm terminal
  paths reject, while every nonempty successful path concludes a terminal State;
- strict rejection of every old tag/field/schema and compatibility alias;
- strict final Program ingress with only State/Match; child fragments expanded away; every old
  FanOut/lane/join/retained-Fragment identity rejected;
- exactly one domain-planned typed `C0` across authored, expanded, certified, entry-point,
  admission-command, and `RunAdmitted` forms; old plural fields, zero/multiple roots, repeated
  builder input, and ordered child-input bindings reject;
- typed builder construction produces exactly one `Value<C0>`; child authoring accepts only the
  current singular fragment-entry `Value<Cn>`, expansion binds that exact nominal value unchanged,
  and the child returns one successor; stale, foreign, missing, extra, or wrong-contract child
  values reject, and a different child wrapper requires an explicit caller-owned State;
- Runtime/Store round-trip the planning-owned `C0` without assembling or interpreting it; hot
  admission and cold found-same qualification recover the same contract/content/typed value and
  derive the same first State input and canonical intent;
- certification accepts zero-state admission terminality and exact terminal Pure, terminal Read,
  and terminal Effect root results, but rejects a mismatched root contract or any remaining
  continuation; terminal Read/Effect acknowledgement-unknown -> found-same returns the recorded
  root without re-entry, and terminal Access output remains unavailable publicly before its durable
  qualified conclusion;
- mutation of every run/Program/input/intent/binding/replacement/capacity/evidence/outcome/fact/
  object field and complete prefix commitment;
- hot encode -> cold ingress equivalence at every prefix;
- selected replacement parent, no gap/branch/cycle, exact equality fields, mode/budget rules;
- occurrence-level conclusion uniqueness across two or more preparations;
- exact duplicate, superseded late, same-selected conflict, Pure conflict, and invalid correlation;
- terminal record rejection on hot preparation, hot conclusion, a same-run-head rebind attempt, and
  hostile cold suffix;
- at most one actionable occurrence at every prefix and rejection of a future occurrence while the
  current occurrence is ready or prepared;
- across direct State edges, State N input equals the exact nominal contract/content/typed value
  concluded by State N-1; stale, same-shaped foreign, truncated, inserted, reordered, stage-rewound/
  skipped, and extra context rejects;
- Match exhaustiveness, exact declared variant-payload qualification, continuation-contract
  convergence, selected-arm complete context, and no action/callback for unselected arms; and
- a non-`Clone` cumulative `MfmValue` fixture moved exactly once through Pure, ordinary Access
  Outcome/State-failure paths, Match, and EVM/Portfolio continuation with move-count/no-duplication
  assertions, or equivalent exact tests for U11's deliberately selected bounded-clone contract;
- one preparation reservation plus exact source/collection/occurrence/context/conclusion/run/
  object/database bounds and every independent +1.

### 10.4 Preparation disposition matrix

Every row asserts successor/session ownership and retained owner. A direct-new row mints exactly
one eligible `CommittedCall`; provider entry remains zero or one and becomes one only when the
consuming adapter-entry method reaches invocation.
The consuming direct-new fixture also asserts `StateImplementation::execute <= 1`; every non-new,
ambiguity-resolution, conclusion-recovery, and cold-history row asserts execute count zero.

| Preparation result | Call count | Owner/result |
| --- | --- | --- |
| direct-new | one call becomes eligible | exact `CommittedCall`; no session |
| raw `Found` -> `ExistingSame` | zero | no local promotion; qualified resume |
| raw `Found` -> different/invalid/over-limit | zero | fail closed |
| validation failure | zero | typed error; no append or automatic retry |
| Stale/terminal/replaced | zero | explicit resume/recovery selection |
| retryable known pre-commit | zero | exact inert `PreparedExecution` |
| fact precondition changed | zero | reviewed exact rebindable preparation owner only |
| Store epoch changed | zero | fail closed and reopen; no retained call authority |
| durability profile lost | zero | fail closed and reopen; no retained call authority |
| projection mismatch | zero | integrity failure; no retained call authority |
| acknowledgement unknown | zero | exact quarantined `PreparedExecution` |
| unknown -> raw `Found` -> `ExistingSame` | zero | destroy continuation, qualified resume |
| unknown -> absent -> direct-new | one eligible call | same owner becomes exact call |
| unknown -> proven absent -> stale/other preparation selected | zero | no call |
| repeated acknowledgement unknown | zero | same quarantined owner retained |
| drop/process loss/cold restart | zero | old authority lost; cold recovery may replace |

Race concurrent spawn/spawn, spawn/resume, and resume/resume workers in one and two processes.
Identical admission yields one direct-new genesis; same-id contenders recover through `Found`, and
distinct-id contenders qualify `StaleHead` before returning the recorded history. A different
candidate conflicts. Same-head preparations yield one direct-new call authority while every
non-new worker has zero execute/provider entry. Pure evaluation and deterministic `prepare` may
each occur once per speculative worker; they are not invocation authority. Multiple opens and
assemblies over the same qualified Memory/PostgreSQL writer identity are accepted and produce the
same CAS dispositions without a `Busy` or live-process-lease path. A different-run control still
advances. At the Store/adversarial layer, prepare two hostile same-head values and attempt same-type
substitution of Store, epoch, tenant, run, Program, occurrence, cumulative input contract/content/
typed value, intent, State implementation, adapter, binding, provider, signer, effect domain,
preparation ref, fact continuation, session shell, and raw backend disposition. Only the exact
direct-new pairing may call.

PostgreSQL crash tests prove the durable `StatePrepared` frame survives primary restart before the
test provider enters, same-epoch recovery resumes from qualified history, and weak durability or an
epoch mismatch cannot produce a direct-new call.

### 10.5 Adapter ingress and handler matrix

Positive/negative tests prove:

- provider success, returned rejection, accepted safe failure, capability definite-pre-entry, and
  accepted integrity block all take a definite conclusion path;
- hot/cold integrity blocking follows the same certified callback-free failure-only route with zero
  State interpretation, output/fact publication, or implicit recovery context;
- malformed, oversize, unauthenticated, wrong-version, wrong-request/call, wrong provider/route/
  target/signer/binding, ambiguous transport, possible entry, and panic/cancellation before the
  consuming conclusion handoff leave only preparation; caught panic/cancellation after handoff
  retain the pending owner;
- a generic operational error cannot construct definite-pre-entry evidence or authorize EntryOnce
  retry;
- evidence is canonicalized/bound once and State interpretation does not decode raw bytes;
- shared adapters cannot return a stashed completion for another same-capability call;
- adapter request bytes derive from committed intent and stable key, not handler replacement bytes;
- call-construction and future-poll panic are contained without detached work; and
- provider diagnostics are destroyed/redacted before errors, traces, or persistence.

Use compile-time and live same-type transposition tests; privacy alone is insufficient.

### 10.6 Pending conclusion and ambiguity matrix

Common Pure and Access rows are:

| Conclusion result | Re-execute/re-enter/re-interpret | Durable/session result |
| --- | --- | --- |
| direct commit | zero | exact advanced session/terminal |
| already same | zero | qualified recorded session/terminal; no republish |
| different conclusion | zero | fail-closed conflict |
| another same-run append | zero | qualify/classify; never rebind across the changed head |
| fact publication changed | zero | same owner rebinds fact coordinate only |
| acknowledgement unknown | zero | same owner quarantined |
| unknown -> found same | zero | recorded result; no republish |
| unknown -> id absent/other append same | zero | `AlreadyConcludedSame`; no republish |
| unknown -> absent/precondition current | zero | retry/rebind same semantic conclusion |
| unknown -> invalid/over-limit | zero | `Permanent`; same owner; no publication/result |
| repeated unknown/retryable | zero | same owner retained |
| Access owner/process loss | zero | cold history is conclusion or preparation only |
| Pure owner/process loss | zero | cold history is conclusion or prior ready/admitted prefix |

For `another same-run append`, qualification must return the exact same/conflicting conclusion,
another selected preparation, terminal state, or invalid-history classification. None permits
conclusion rebinding across that changed run head.

Mode-specific rows are:

| Mode-specific conclusion result | Re-execute/re-enter/re-interpret | Durable/session result |
| --- | --- | --- |
| Access superseded preparation | zero | `NoLongerSelected` latest session |
| Access unknown -> replacement/different | zero | `NoLongerSelected`/conflict |
| Pure existing same/different conclusion | zero | recorded result/conflict |
| Pure no conclusion but not ready | zero | invalid-history failure |

For Access, inject process loss after provider return, ingress, interpretation, synchronous
conclusion handoff, and pending-owner construction. Inject cancellation/panic before the consuming
handoff and prove prepared history remains; inject cancellation immediately after the handoff and
prove supervisor retention/recovery. During commit, after COMMIT-before-ack, and after
acknowledgement, only process death or explicit force-destruction may produce owner loss. For Pure,
inject process loss after evaluation, proposal construction, canonicalization, and conclusion
handoff; cancellation after handoff must likewise retain the owner. Replay exposes the conclusion
or the mode's prior prefix. Public successful output or definite provider result never escapes
early.

Snapshot every applicable provider, ingress, interpretation, canonical-conclusion, and fact-scan
counter when `PendingConclusion` is minted; no counter may increase during conclusion recovery.
Pure provider/ingress counters remain zero. Recovery of one `PendingConclusion` never reevaluates
Pure, while independent competing workers may each speculatively evaluate it once before Store
selects one durable conclusion. Capability and retryable pre-entry fact-scan cardinalities are
asserted separately.

Prepare two same-type `PendingConclusion` owners and try to swap physical append outcomes/ids,
semantic conclusions, active successors/session shells, fact plans, and acknowledgement
quarantines. Neither owner may advance or publish the other's conclusion.

### 10.7 Recovery, facts, replay, and terminality

Recovery tests cover:

- Read exact-input/exact-operation bounded attempts and explicit freshness coordinate;
- EntryOnce parking and absence of any second preparation/action;
- Read and EntryAbsorbing permitted replacement overlap across same-process and cross-process
  workers, exact cumulative input/intent/binding/domain/identity, total-entry bounds, late-old
  `NoLongerSelected`, adapter stable-key transmission, full overlap/recovery horizon, and convergent
  evidence; EntryOnce has no MFM replacement overlap;
- replacement changes the physical append id and `StatePreparationRef` while preserving canonical
  intent and absorption identity; independent mutation proves the three identities are not
  interchangeable, and source/API checks reject a duplicate universal `StableOperationKey` field;
- explicit rejection of non-convergent `RowsAffected(1)`/`RowsAffected(0)`-style evidence;
- late old result before/after newer conclusion -> `NoLongerSelected`;
- direct-new-only one-use prior-fact continuation, retryable scan before provider, and no cold mint;
- provider-affecting fact dependency is represented by an explicit prior Read State and included in
  the complete predecessor `S::Input`; catalog/API tests reject a hidden pre-prepare fact callback
  or a post-commit fact changing intent/target/request/signer/binding/domain;
- fixed preparation selection frontier, dense route completeness, producer head/source binding,
  selected response/attestation persistence, and separate publication frontier;
- atomic conclusion + objects + facts + dense route + heads + terminal/attention projection, with
  rollback at every SQL statement;
- duplicate conclusion does not publish twice; State failure/integrity block publishes no facts;
- Pure and Access conclusions advance the same cumulative chain; a direct State receives the exact
  predecessor context and a Match arm receives the exact qualified complete variant payload; early
  data remains available to late States; hot/cold resume selects the same next context and intent
  after one conclusion encoding and with no hot Serde round trip;
- explicit domain failure-context recovery round-trips the prior context; default failure exposes
  no history/context and leaves every later normal State's evaluate/prepare/execute/provider
  counters at zero;
- every valid replay prefix derives the same next sequential cursor and exact qualified cumulative
  input as direct reduction with zero State, adapter, scanner, provider, signer, or live callback;
- replay/portable qualification rejects stale/foreign/truncated/inserted/reordered/stage-rewound/
  skipped/extra contexts and contract/content substitutions; and
- no semantic append after derived terminality in Memory, PostgreSQL, direct advancement, a
  same-run-head rebind attempt, and hostile retained input.

### 10.8 PostgreSQL conformance and hostile schema

Memory/PostgreSQL conformance covers raw complete frames, route-scoped Found-before-head,
exact-head races, unknown acknowledgement, Store/fact changing preconditions, bounds, and
single/multiple composite Store opens.

The composite-open authority suite proves each public open takes one backend, expected Store
identity, exact catalog instance, and limits, then yields all ports under one private coordinator
brand. Trybuild/hostile cases reject port literals, per-port replacement/extraction, mixing branded
parts across opens, raw-backend-to-semantic conversion, partial-port escape after failed readiness,
and foreign coordinator/catalog brands. Memory and PostgreSQL tests accept simultaneous qualified
opens/Runtime assemblies over the same backend/store writer identity and prove their competing
commands receive identical CAS dispositions without cross-brand transposition.

Backend SPI contract tests prove object safety and implementation from an external test crate;
`BackendFuture` lifetime/Send bounds; every borrowed command getter; and bounded public raw DTO
constructors/getters. Count, byte, coordinate, and overflow errors fail before Store ingress and
never include payload bytes. Exhaustive injected faults cover every history, append, fact,
configuration, and audit call site: only loss after possible transaction submission is
`AcknowledgementUnknown`; all known pre-commit or definite-rollback failures return the exact
retry/rebind owner or the named terminal classification.

PostgreSQL-specific coverage includes:

- v8 exact catalog, one-frame tables, selected-head constraint, roles/grants/query inventory, and
  chosen attention shape;
- zero history/configuration reads during open;
- Store/tenant/run partition predicates on every lookup, with identical run/append ids under two
  tenants and no cross-partition information leak;
- transaction rollback at each frame/head/object/fact/projection statement;
- borrowed run and configuration command parity across Memory/PostgreSQL plus forgery/transposition
  rejection before authority-seal deletion;
- logged table/primary/`fsync`/`full_page_writes`/effective `synchronous_commit`, epoch, and writer-
  role qualification;
- epoch/durability loss after open;
- actual primary crash/restart after preparation and conclusion commit with same-epoch qualified
  resume;
- two independent same-epoch processes racing admission, preparation, replacement, and conclusion
  through exact-head CAS, with only direct-new preparation winners entering providers;
- legitimate database restore with new identity/epoch and no old-run continuation, plus rejection
  of a restored deployment that reuses the old identity/epoch;
- hostile frame/mechanical-column/projection mismatch; and
- absence/rejection of the superseded pre-cutover schema, normalized object rows,
  target-currentness schema, and every five-family tag/projection/query assumption.

### 10.9 Application, portable, EVM, and secrets

Retain tests for fixed-tenant facade isolation; partition mismatch invalidity; credential/policy/
token/bearer deletion; strict request ingress; owner-carrying admission/executor retry; trusted REST
exposure docs; and no public tenant override.

Portable v5 tests cover consuming evidence once, deterministic bytes/ref, all bounds, three-family
frames, source/fact closure, omission/substitution/cycle/cross-tenant/cross-Store/cross-epoch,
expected ref before decode, v4 rejection, zero live reads/callbacks, and no persistence import.

EVM tests cover new identity convergence/divergence, old issuer/domain rejection, fresh activation
gate, nonce/idempotency, exact broadcast entry classification, wallet terminal value tamper, and
absorbing evidence convergence. Balance tests log exact declaration/source order--chain identity,
anchor, then native balance or token decimals/token balance, then confirmation--and inject failure
at every stage with no later preparation/call. They inspect every complete context, Match-arm
convergence, consolidation anchor/duplicate/binding/order/mathematics, and exact standalone public-
result contract/content/canonical-byte equivalence, including ordinal and caller correlation unless
U11 deliberately proves and approves a public-field deletion.

Portfolio tests prove exact collection order, the Pure entry State constructs the complete EVM
wrapper before each child call, expansion passes it unchanged, one EVM collection completes before
the next begins, failure suppresses later collections, the opaque typed continuation returns byte/
content-identically, EVM is semantically independent of it, final Pure consolidation runs, and the
canonical public output is equivalent.

Secret canaries cover Program/configuration, `RunAdmitted`, `C0..Cn`, hot/cold session contexts,
explicit failure context, EVM/Portfolio contexts and consolidation inputs, admission source
manifests,
`StatePrepared`, committed-call debug, accepted evidence, `PendingConclusion`, `StateConcluded`,
canonical journal frames, newly reachable object/output closure, facts, portable bytes, DTOs,
CLI/REST output, errors, traces, and debug formatting. Retain bounded secret ingress, zeroization,
constant-time comparison, keystore AAD anti-swap, and corruption tests.

### 10.10 Negative source, metadata, and documentation checks

Execute the checked exhaustive §8.7 path/literal manifest, including every cutover-base addition,
exact Serde spelling, SQL identifier, deleted package/feature, old Program authority, policy/
currentness family, portable v4/import surface, EVM issuer field/domain, superseded pre-cutover
PostgreSQL contract, and obsolete Nix task. Exclude only the RFC, this plan, and the scanner's own
manifest; hostile fixtures assemble banned literals or use explicit per-fixture allowlists.

Do not globally ban ordinary words such as authorization, observation, reservation, credential,
lease, fence, closed, collect, collection, join, lane, barrier, producer, graph, or concurrent.
Provider protocol authorization, Read observations, wallet reservations, database credentials,
transaction leases, public closed status, domain collections, iterator collection, SQL/thread joins,
test/fact barriers, fact producers, dependency graphs, and cross-run/backend concurrency may remain.

Check every changed repository link, Rust/API snippet, CLI/REST example, SQL claim, and Nix task id.
Run `git diff --check`.

### 10.11 Performance acceptance

After U1/U7/U9 fix limits, benchmark maximum Program finish/ingress, `C0`/maximum `Cn`
construction, every cumulative-context append, frame/conclusion/run build/append/ingress, maximum
hot advancement, maximum cold resume and memory high-water, exact source/collection/occurrence/
context/object/database bounds and every +1, one selected-preparation conclusion reservation,
worst fact closure,
configuration chain, and one-shot resume/drive versus retained executor.

Acceptance is the recorded threshold. Hot conclusion advancement performs one required canonical
record encoding; passing its retained typed context to the next State adds no serialize/decode
round trip, prefix read, or fold. Direct admission advancement and preparation-to-call construction
likewise add no avoidable round trip/read/fold. Do not add a cache, checkpoint, sharing scheme,
FanOut, or Collect as a benchmark workaround.

## 11. Engineer handoff and final report

Before each logical commit, restate its deletion boundary and the old guards whose jobs move. When
an unexpected consumer appears:

1. identify the exact proposition it consumes;
2. place that immutable, temporal, protocol, or domain proposition with one target owner;
3. amend RFC/plan if ownership or architecture changes; and
4. delete the old path rather than preserve both.

The final report contains:

- U1–U11 rulings and links to tests, benchmarks, and inventories;
- final crate graph and deleted packages/features;
- public Rust API before/after, highlighting each opaque/affine type;
- final record/schema/portable/EVM/configuration identities and activation steps;
- product-code LOC and public-type counts before/after;
- every net-new concept, its one owner, and why an existing concept could not own it;
- exact verification commands/results and every unrun gate/reason;
- known limitations: trusted embedding, provider factual trust, response-loss window, EntryOnce
  parking, cold-resume bound, immutable binding replacement, and absent rollback witness; and
- confirmation that no five-family, FanOut/lane/join/Collect workflow definition, use, or identity
  survives outside explicit rejection/deletion manifests, with narrow allowance for legitimate
  ordinary domain/tooling words and with product LOC/public-type reductions.

Success is not that new names exist. Success is that durable preparation gates the one supported
entry path, one atomic conclusion owns State meaning, missing conclusions remain neutral, replay is
callback-free, execution is sequential over domain-owned cumulative contexts, and the superseded
observation and FanOut lifecycles no longer exist.
