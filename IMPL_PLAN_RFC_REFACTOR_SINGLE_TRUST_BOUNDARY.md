# Implementation plan: refactor MFM to a single byte-ingress trust boundary

Status: implementation handoff; blocked only by the owner rulings in
[Material uncertainties](#material-uncertainties-and-preflight-gates)

Normative architecture: [`RFC_REFACTOR_SINGLE_TRUST_BOUNDARY.md`](RFC_REFACTOR_SINGLE_TRUST_BOUNDARY.md)

Audience: engineer-agent implementing the complete MFM platform cutover

---

## 0. Authority, mandate, and completion rule

`RFC_REFACTOR_SINGLE_TRUST_BOUNDARY.md` is the accepted target architecture for MFM. It is the new
source of truth for the platform as a whole. Current code, tests, migrations, fixtures, READMEs,
`docs/design.md`, `docs/architecture.md`, and the Git history of the superseded
`rfc_single_trust_boundary.md` describe the system that is being replaced. Where they conflict with
the accepted RFC, they are evidence for the cutover and deletion scope; they are not constraints on
the target.

This file is the normative implementation decomposition of that RFC. It fixes package ownership,
the sensitive Rust APIs, the order in which old guards may be removed, the required tests, and the
logical commit boundaries. It may make the RFC more concrete but may not weaken or reinterpret an
RFC invariant. If an implementation detail here is discovered to conflict with the RFC, stop,
amend both documents deliberately, and obtain an architect review before continuing.

The engineer has complete authority to remove, rename, move, split, merge, or rewrite any code,
crate, public API, schema, migration, fixture, test, task, feature, or document needed to reach the
target. In particular:

- breaking Rust and wire changes are expected;
- old persisted histories are rejected under the fresh-identity cutovers specified here;
- `mfm-certify`, `mfm-spec`, obsolete authority-seal bridges, the application access-control model,
  and the generic live-currentness model are deletion targets, not compatibility surfaces;
- no alias, deprecated wrapper, old decoder, dual write, fallback, optional legacy path, feature-
  gated escape hatch, or inert proof type may survive a cutover commit; and
- existing LOC and abstraction boundaries have no preservation value by themselves.

The permission to refactor is not permission to weaken the properties the RFC explicitly retains.
The exact journal hash chain, append atomicity, exact-head compare-and-append, dense fact frontier,
durable-reservation-before-invocation, observation-before-selected-settlement, content addressing,
tenant partitioning, secret exclusion, EVM nonce/effect safety, and provider protocol
authentication remain load-bearing.

Implementation is complete only when:

1. every acceptance criterion in the RFC and this plan passes;
2. every deleted type and wire spelling is absent from the production feature graph and strict
   decoders reject its old representation;
3. the authoritative contracts have been rewritten to describe only the new design;
4. the workspace and Nix task graph contain no deleted package or compatibility edge;
5. the owner rulings below are recorded and their chosen branches are the only branches in code;
6. the final report includes public-type, dependency, and product-LOC deltas plus every deliberate
   net-new concept; and
7. each logical commit is a coherent current design, with its contracts and tests updated in the
   same commit.

Do not implement this as a long-lived stack of compatibility layers. Temporary compile failures in
the working tree are acceptable while constructing one commit; a committed half-cutover is not.

Commits 1–6 are one non-deployable migration train. Each checkout must compile, pass its scoped
tests, and run only against an ephemeral fresh identity matching that checkout, but no intermediate
schema/domain identity may be activated in a retained deployment. Production activation occurs
once, after Commit 6 and final verification, with the final run-store, Program, journal,
configuration, portable, wallet-domain, and sender identities. This avoids inventing five
successive compatibility migrations while keeping every logical commit reviewable and coherent.

## 1. First-principles target

The implementation has five ownership rules.

1. **Program owns executable semantics.** `mfm-program` is the only owner of the normalized,
   callback-free executable graph and its hostile-byte ingress. A `Program` is valid by
   representation; a `ProgramDocument` and `ProgramRef` are not execution authority.
2. **Store owns retained semantic evidence.** `mfm-store` owns run/configuration byte ingress, the
   sole event reducer, deterministic append binding, `QualifiedRun`, prepared successors, raw
   backend contracts, and callback-free projections.
3. **Runtime owns live execution authority.** `mfm-runtime` owns immutable process assembly,
   callbacks/adapters, the private assembly brand, affine `RunSession`, exact prepared-drive
   pairing, commit coordination, and the one-use transition to provider invocation.
4. **Application owns a tenant-scoped product facade, not access control.** The trusted embedding
   constructs one facade per `TenantScopeId`; MFM accepts no caller credential, principal, role,
   grant, or tenant override.
5. **PostgreSQL owns durable ordering within one admitted epoch.** It preserves immutable bytes and
   returns truthful exact-head append dispositions under a qualified durability profile. It does
   not repeat program/reducer semantics or return a semantic proof.

The hot path must reduce to:

```text
spawn/resume -> ActiveQualifiedRun -> Runtime-branded RunSession@H

RunSession@H + typed action
  -> ResolvedEvent
  -> one reducer
  -> one deterministic binder
  -> PreparedDrive { session continuation, append owning predecessor@H, exact live continuation }
  -> one consuming commit coordinator
  -> exact-head compare-and-append
  -> direct NewlyCommitted
  -> session@H_next [+ ReadyToInvoke]
```

There is no full-prefix reload, deserialize/revalidate, second reducer, semantic comparison, or
positive committed-batch echo on that direct branch.

## 2. Target package and dependency graph

The current `mfm-store -> mfm-runtime -> mfm-certify` direction forces public proof wrappers,
feature-gated constructors, and an artificial `RuntimeHistoryPort` abstraction. Invert it.

```text
mfm-capabilities       capability contracts and entry/ambiguity semantics
        |
        v
mfm-program            typed DSL, pure expansion/compiler, ProgramDocument,
        |               opaque Program, ProgramIngress, ProgramCatalog
        |
        +--------------------------+
        |                          |
        v                          v
mfm-journal                    mfm-store
canonical records/hash         ingress, reducer, QualifiedRun,
                               PreparedAppend, backend contracts,
                               configuration, facts, read projections
                                      |
                                      v
                                mfm-runtime
                                ProcessRegistry, assembly brand,
                                RunSession, PreparedDrive,
                                ReadyToInvoke, state execution
                                      |
                                      v
                                  mfm-app
                                  tenant-scoped facade
```

Storage implementations depend on `mfm-store` and implement only its mechanical backend traits.
Replay depends on `mfm-store` and `mfm-program`, never Runtime. Domain crates depend on Program and
Capabilities for typed declarations; live adapter crates depend on Runtime only where they
register live implementations. Binaries depend on App and remain transport wrappers.

Required graph changes:

- remove `mfm-store`'s dependency on `mfm-runtime`;
- add direct `mfm-store -> mfm-capabilities` and `mfm-store -> mfm-program` dependencies because
  Store's typed reservation/observation products name the sealed Read/Effect/fact-mode contracts
  and its hostile ingress resolves the exact Program catalog; these edges remain acyclic because
  Capabilities depends only on Values/IDs and Program depends on Capabilities;
- add `mfm-runtime -> mfm-store`;
- move `RuntimeHistoryPort`, `VerifiedRunView`, raw append disposition ownership, reducer commands,
  and callback-free identity types to their Store/Program owners or delete them;
- remove the `mfm-runtime/store-authority` feature and every feature-expanded hidden constructor it
  enables;
- absorb the one normalized document/graph surface from `mfm-spec` into `mfm-program`, then delete
  `crates/kernel/spec`, its workspace member, dependencies, tasks, docs, and test fixtures;
- move the callback-free compiler/expansion and live assembly responsibilities out of
  `mfm-certify`, then delete `crates/kernel/certify`, its workspace member, dependencies, tasks,
  docs, feature bridges, and test fixtures;
- replace the remaining load-bearing `mfm-authority-seal` uses with opaque qualified products and
  private constructors in their owning crates, then delete `crates/kernel/authority-seal`, its
  workspace member, feature bridges, dependencies, tasks, docs, and fixtures. Public empty marker
  traits are packaging conventions, not an authority boundary. Implementable backend/provider
  traits are explicit TCB injection seams selected by trusted composition; and
- remove direct `mfm-certify`/`mfm-spec` dependencies from app, replay, store, runtime, PostgreSQL,
  EVM, portfolio, live EVM, integration tests, and all dev-dependency graphs.

No new generic “kernel authority” crate replaces the three deleted crates. Shared callback-free wire
types live with Program or Journal; retained-history types live with Store; live handles live with
Runtime.

## Material uncertainties and preflight gates

These are implementation gates, not reasons to preserve the current design. Record each ruling in
the RFC and this plan before the first code cutover that depends on it. Delete the unchosen branch.

### U1. Unobserved `EntryOnce` recovery

- **Target assumption:** an unobserved `EntryOnce` reservation parks as `PossibleEntry` and requires
  manual/operator disposition. Only `EntryAbsorbing<MAX>` may reserve a new bounded ordinal under
  its exact duplicate-absorption contract and immutable effect domain.
- **Why material:** automatic failover without provider evidence can duplicate a real-world effect.
- **If wrong:** the product needs a capability-specific status/idempotency proof or a separately
  designed provider fence; exact-head journaling alone cannot prove an old thunk did not enter.
- **Gate:** product owner signs off manual attention for ordinary non-idempotent effects. Add the
  cold-resume parking and no-reinvoke tests before deleting current recovery/currentness guards.

### U2. Session retention and cold-resume latency

- **Target assumption:** one executor may retain an affine session across steps; a genuinely new
  resume folds the complete bounded prefix. There is no LRU, suffix protocol, or checkpoint.
- **Why material:** stateless `drive_once(run_id)` traffic across workers may repeatedly replay a
  maximum-sized run.
- **If wrong:** the product may miss its latency target, but an unmeasured cache would only hide the
  ownership decision.
- **Gate:** name the maximum retained-history bound, how long a caller/worker may retain a session,
  and either a cold-resume latency SLO or an explicit no-SLO ruling. Benchmark the maximum supported
  prefix in the Nix shell. A checkpoint requires a separate accepted design if the SLO is missed.

### U3. Tenant-wide Effect-attention inventory

- **Target assumption:** keep the append-atomic Boolean projection and listing API only if operators
  must discover abandoned effects without knowing their run ids.
- **Why material:** without that requirement, the API, column, and partial index are unused product
  surface; with it, known-run resume is insufficient for discovery.
- **If wrong:** an absent inventory strands discoverability, while an unnecessary inventory adds a
  durable projection and public operator surface with no product owner.
- **Gate:** product owner chooses `enabled` or `absent`. When enabled, make the projection
  snapshot-complete under one PostgreSQL snapshot, fix a maximum materialized inventory count and
  byte bound, and qualify a selected run before action. When absent, delete the method, DTOs,
  projection, migration/index, docs, and tests together.

### U4. Provider factual trust

- **Target assumption:** each configured `ProviderBinding<C>` is part of `C`'s TCB unless the
  capability contract explicitly requires a cryptographic proof, quorum, or independent evidence.
- **Why material:** strict decoding and request binding prove protocol validity, not the truth of a
  balance, nonce, receipt, or chain view.
- **If wrong:** MFM may present an authenticated but false provider statement as an independently
  established fact, or impose a proof contract the provider cannot satisfy.
- **Gate:** produce a table for every production Read/Effect capability: provider owner, response
  ingress, authenticity check, request/response relation, factual-trust assumption, and returned
  evidence type. Narrow misleading “verified” language and types before deleting duplicated
  validators.

### U5. EVM identity/sender cutover

- **Target assumption:** there is no retained non-policy issuer namespace. New intent identity is
  over tenant, wallet nonce domain, and `SubmissionIdempotencyKey`; deployment uses fresh run-store
  and wallet-domain identities and preferably a fresh sender.
- **Why material:** old incomplete allocations/effects or a reused sender can collide with or strand
  progress after the hash-domain reset.
- **If wrong:** the fresh intent domain can duplicate, collide with, or make unrecoverable an old
  sender allocation/effect that is still live outside the new Store identity.
- **Gate:** require either a fresh sender or an auditable artifact proving old-process drain, every
  prior allocation/effect terminal, pending nonce reconciled, and exclusive sender control. Do not
  activate the new run-store/wallet identities without that artifact.

### U6. Cold physical-binding evidence

- **Target assumption:** retain only immutable, secret-free descriptor fields needed to interpret
  history and resolve an exact process binding; delete all release/currentness lineage.
- **Why material:** a current certificate field may encode a legitimate immutable restart/audit
  proposition despite its current name.
- **If wrong:** wholesale deletion can make retained history uninterpretable, while retaining a
  currentness field under a neutral name silently recreates the rejected authority layer.
- **Gate:** before the vertical access cutover, check in a field/consumer inventory mapping every
  physical certificate field to either (a) a target descriptor/ingress proposition and exact new
  owner or (b) a deletion rationale. A source/API/schema assertion must fail if an unmapped family
  remains. The inventory must explicitly separate provider inventory/challenge/signature evidence
  that authenticates byte ingress from release/current-head promotion evidence that is deleted. It
  must also decide whether current `durable_generation_ref` is immutable key-instance identity; if
  not, derive a new `key_instance_ref` from immutable signer metadata rather than renaming a
  freshness field.

### U7. Dormant nonterminal runs after binding replacement

- **Target assumption:** a retained program resumes only under an assembly satisfying its exact
  immutable binding descriptors. A changed binding does not reinterpret or replan it.
- **Why material:** ordinary deployment reconfiguration could otherwise strand a nonterminal run.
- **If wrong:** a deployment can lose the only executable assembly for a valid retained run or
  silently reinterpret that run under a binding it never fixed.
- **Gate:** deployment owner chooses one explicit policy: drain all such runs; keep an old immutable
  assembly available; or make them read/replay-only. Any migration/rebinding protocol is a separate
  RFC and must not recreate generic live currentness.

### U8. Total configuration-history bound

- **Target assumption:** every configuration stream has an explicit maximum revision count and
  cumulative canonical-byte bound in addition to the existing 16 MiB per-revision bound.
- **Why material:** demand-time qualification is not a work bound if a valid selected chain may be
  arbitrarily long.
- **If wrong:** a selected configuration can monopolize memory/CPU even though each row is bounded.
- **Gate:** the configuration/product owner fixes both numbers and the expected worst-case load
  time before the configuration commit. Enforce them identically in memory, PostgreSQL, import,
  audit, and test fixtures.

### U9. Erased typed-value representation

- **Target assumption:** canonical bytes plus exact contract identity and an
  `Arc<dyn Any + Send + Sync>`-style erased typed value owned by `mfm-program` can cross
  Program/Store/Runtime under one exact in-process catalog brand without lifetime or thread-safety
  escape hatches.
- **Why material:** this representation is the mechanism that removes hot serialize/decode cycles;
  an invalid shape would push duplicate byte validation back into callback dispatch/settlement.
- **If wrong:** the Program/value ownership split must be revised before the vertical cutover.
- **Gate:** land a compile-only prototype covering local construction, hostile ingress, Store
  retention, Runtime downcast, direct provider response, cold resume, and `Send` movement. It must
  expose no public unchecked downcast or free constructor outside the catalog-owned checked APIs.

### U10. Maximum canonical complete-batch frame

- **Target assumption:** PostgreSQL persists one canonical complete-batch frame and Store enforces
  one explicit maximum frame size before allocation/write.
- **Why material:** the current 32 MiB envelope limit excludes separately stored objects, while the
  whole-run limit is 512 MiB; neither is automatically the correct single-row bound.
- **If wrong:** too small rejects currently required appends; too large permits pathological rows
  and transient allocations.
- **Gate:** set the maximum append frame together with U2's maximum prefix and latency/memory SLO,
  benchmark it, and apply it to memory, PostgreSQL, import, Found ingress, and candidate building.
  Do not preserve object decomposition merely to avoid the choice.

### U11. PostgreSQL failure-domain claim

- **Target assumption:** the current product promises primary crash/restart durability, not survival
  of primary-host loss; therefore the one implemented profile is `PrimaryCrashRestart`.
- **Why material:** an out-of-tree deployment may already advertise synchronous-replica or quorum
  survival.
- **If wrong:** Runtime could invoke after an acknowledgement weaker than the published failure
  domain.
- **Gate:** deployment owner confirms the claim. If host-loss survival is required, name and
  qualify the exact synchronous topology before implementation; do not silently generalize the
  local profile.

Apart from these eleven rulings/gates, no material architectural uncertainty remains. The package
inversion, deletion of internal access control/live revocation, demand-time qualification,
payloadless direct commit, and fresh incompatible wire cutovers are settled.

## 3. Program, state, and process APIs

The signatures below are normative at the ownership and visibility level. Ordinary naming may be
adjusted during implementation only when the resulting API has fewer concepts and preserves the
same construction constraints. Do not make a private field public merely to make a cross-crate
cutover easier.

### 3.1 Capability contracts

`mfm-capabilities` keeps only the access ABI and Effect entry/ambiguity rules:

```rust
pub trait ReadCapabilityContract: Send + Sync + 'static {
    type Request: MfmValue;
    type Returned: MfmValue;
    type SafeFailure: SafeFailureValue;
    type Facts: FactSelectionMode;

    fn contract_id() -> Result<StableId, CapabilityContractError>;
    fn bind_returned(
        request: &Self::Request,
        returned: &Self::Returned,
    ) -> Result<(), CapabilityResponseError>;
    fn bind_safe_failure(
        request: &Self::Request,
        failure: &Self::SafeFailure,
    ) -> Result<(), CapabilityResponseError>;
}

pub trait EffectCapabilityContract: Send + Sync + 'static {
    type Request: MfmValue;
    type Returned: MfmValue;
    type SafeFailure: SafeFailureValue;
    type Entry: EffectEntryModeFor<Self::Request>;
    type Facts: FactSelectionMode;

    fn contract_id() -> Result<StableId, CapabilityContractError>;
    fn bind_returned(
        request: &Self::Request,
        returned: &Self::Returned,
    ) -> Result<(), CapabilityResponseError>;
    fn bind_safe_failure(
        request: &Self::Request,
        failure: &Self::SafeFailure,
    ) -> Result<(), CapabilityResponseError>;
}

pub enum EntryOnce {}
pub struct EntryAbsorbing<const MAX_ENTRIES: u16>;

#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValuePresence {
    Inhabited,
    Uninhabited,
}

#[doc(hidden)]
pub struct IntrinsicTypedValue {
    // Private contract, canonical Arc<str>, ContentRef, and Arc<dyn Any + Send + Sync>.
}

impl IntrinsicTypedValue {
    #[doc(hidden)]
    pub fn contract_ref(&self) -> &ValueContractRef;
    #[doc(hidden)]
    pub fn content_ref(&self) -> &ContentRef;
    #[doc(hidden)]
    pub fn canonical_json(&self) -> &str;
    #[doc(hidden)]
    pub fn into_parts(
        self,
    ) -> (
        ValueContractRef,
        ContentRef,
        Arc<str>,
        Arc<dyn Any + Send + Sync>,
    );
}

// Owned by mfm-values and re-used by Capabilities and Program.
pub trait SafeFailureValue:
    private::SafeFailureValueSealed + Send + Sync + 'static
{
    #[doc(hidden)]
    fn presence() -> ValuePresence;
    #[doc(hidden)]
    fn into_intrinsic(self) -> Result<IntrinsicTypedValue, IntrinsicValueError>;
}

pub enum NoSafeFailure {}

pub trait EntryKeyed: MfmValue {
    type EntryKey: MfmValue + Eq;
    fn entry_key(&self) -> Self::EntryKey;
}

pub trait EffectEntryMode: private::EffectEntryModeSealed + Send + Sync + 'static {}
pub trait EffectEntryModeFor<R: MfmValue>:
    EffectEntryMode + private::EffectEntryModeForSealed<R>
{}

pub trait FactSelectionMode: private::FactSelectionModeSealed + Send + Sync + 'static {}
pub struct NoPriorFacts;
pub struct PriorRunFacts;
```

`EntryOnce` implements the private parameterized relation for every request. The catalog implements
`EffectEntryModeFor<R>` for `EntryAbsorbing<MAX_ENTRIES>` only when `R: EntryKeyed`, and rejects
`MAX_ENTRIES == 0` during its sole finalization before a Program can exist. Downstream crates cannot
implement either sealed trait or invent another mode. Preserve the exact request-derived entry key
and non-zero bounded absorption semantics; delete only refresh/revocation, not ambiguity recovery.
Add compile-fail/build-fail tests for a downstream custom mode, unkeyed absorbing request, and zero
maximum.

`NoPriorFacts` and `PriorRunFacts` are likewise the only fact modes. The platform has one concrete
prior-run selection protocol, so a speculative query-protocol generic is not introduced. These
markers classify whether that framework-issued capability exists by type, not by an unchecked
`Option`.

`SafeFailureValue` is sealed in `mfm-values`. Every ordinary `MfmValue` receives the inhabited
implementation; `NoSafeFailure` receives the only uninhabited implementation and deliberately has
no lexical schema, decoder, persisted slot, or constructor. `ValuePresence` and
`IntrinsicTypedValue` are public-but-doc-hidden lower-layer bridge products with private fields.
The inhabited implementation canonicalizes the already-valid Rust value once and moves that value
into an `Arc<dyn Any + Send + Sync>`; the uninhabited implementation is an exhaustive match on
`self`. Their consuming accessors are usable by `mfm-program` but no downstream crate can construct
either product or implement the sealed trait. This is a cycle-free Rust API, not prose-level
specialization or a `TypeId` branch.

`mfm-runtime` owns the live, request-bound completion packages:

```rust
pub struct ReadCompletion<C: ReadCapabilityContract> {
    call: private::ReadCallToken,
    inner: private::ReadCompletionKind<C>,
}

pub struct EffectCompletion<C: EffectCapabilityContract> {
    call: private::EffectCallToken,
    inner: private::EffectCompletionKind<C>,
}
```

The Runtime-private inners retain only Returned/SafeFailure and Effect-only EntryUnknown. Only the
request-bound `ReadAdapterCall<C>` and `EffectAdapterCall<C>` methods in
§5.3 construct them. They must not
contain refresh evidence, supersession, a capability lease, a resource generation, or a generic
“current” disposition. `EntryUnknown` remains Effect-
only. A capability with no safe failure uses `NoSafeFailure`, not a broad enum plus a runtime subset
validator or fake lexical value.

Delete from `mfm-capabilities`:

- `NoRefreshEvidence`, `EffectRefreshMode`, `NoRefresh`, `Refreshable`, and every refresh-evidence
  generic or validator;
- `ResourceAuthorityContract` and generic resource-currentness contracts;
- public `ReadAdapterInvoker` and `EffectAdapterInvoker` traits;
- `EffectSpec`, `EffectDescriptor`, `EffectClass`, role specs, `CapabilitySpec`,
  `CapabilityDescriptor`, `CapabilitySet`, tuple mutation-authority counting, and their sealed/UI
  machinery. The repository reference audit found no owner independent of the deleted parallel
  algebra; and
- validators whose only job was to compare a caller-authored descriptor with associated Rust
  types. Intrinsic request/return invariants move to opaque value construction/ingress.

### 3.2 State declaration and mode-indexed callbacks

The public state declaration in `mfm-program` is exactly one ABI source:

```rust
pub trait State: Send + Sync + 'static {
    type Input: StructuredValue;
    type Output: StructuredValue;
    type Failure: FailureValue;
    type Execution: Execution;
    type SafeFailureDisposition:
        SafeFailureDisposition<Self::Execution, Self::Failure>;
    type Lowering: StateLowering;

    fn state_id() -> Result<StableId, ProgramBuildError>;
    fn fact_slots() -> Result<Vec<FactSlot>> { Ok(Vec::new()) }
}

pub trait SafeFailureDisposition<E: Execution, F: FailureValue>:
    private::SafeFailureDispositionSealed<E, F> + Send + Sync + 'static
{
    type SafeFailureProposal<O: StructuredValue>;

    fn into_outcome<O: StructuredValue>(
        proposal: Self::SafeFailureProposal<O>,
    ) -> ProposedStateOutcome<O, F>;
}

// Also owned by mfm-values; Program re-exports it for State declarations.
pub trait FailureValue: private::FailureValueSealed + Send + Sync + 'static {
    #[doc(hidden)]
    fn presence() -> ValuePresence;
    #[doc(hidden)]
    fn into_intrinsic(self) -> Result<IntrinsicTypedValue, IntrinsicValueError>;
}
pub trait InhabitedFailureValue: FailureValue + MfmValue {}
pub enum Never {}

pub trait Execution: private::ExecutionSealed + Send + Sync + 'static {}

pub struct Pure;
pub struct Read<C: ReadCapabilityContract>(PhantomData<fn() -> C>);
pub struct Effect<C: EffectCapabilityContract>(PhantomData<fn() -> C>);

pub trait StateLowering: private::StateLoweringSealed + Send + Sync + 'static {}
pub struct DirectLowering;
pub struct ExpandWith<E: StateExpansion>(PhantomData<fn() -> E>);

pub struct StateExpansionScope<'scope, S: State> {
    // Private scoped graph builder plus an ExpansionValue<S::Input>.
}

pub struct ExpansionValue<'scope, T: Send + Sync + 'static> {
    // Private typed endpoint handle with invariant candidate lifetime; no bytes or Serde.
}

pub struct ExpandedState<'scope, S: State> {
    // Private candidate-scoped substitution token proving Input/Output/Failure ABI for S.
}

pub trait StateExpansion: Sized + Send + Sync + 'static {
    type AbstractState: State<Lowering = ExpandWith<Self>>;

    fn expansion_id() -> Result<StableId, ProgramBuildError>;
}

pub struct StateExpansionImplementation<E: StateExpansion> {
    // Private Arc of one pure higher-ranked recipe; may capture catalog-issued binding handles.
}

impl<E: StateExpansion> StateExpansionImplementation<E> {
    pub fn new(
        expand: impl for<'scope> Fn(
                StateExpansionScope<'scope, E::AbstractState>,
            ) -> Result<ExpandedState<'scope, E::AbstractState>, ProgramBuildError>
            + Send
            + Sync
            + 'static,
    ) -> Self;
}

impl<'scope, S: State> StateExpansionScope<'scope, S> {
    pub fn input(&self) -> ExpansionValue<'scope, S::Input>;

    pub fn state<T, B>(
        &mut self,
        binding: &B,
        input: ExpansionValue<'scope, T::Input>,
    ) -> Result<ExpandedState<'scope, T>, ProgramBuildError>
    where
        T: State,
        B: ProgramStateBinding<T>;

    pub fn finish_fallible(
        self,
        output: ExpansionValue<'scope, S::Output>,
        failure: ExpansionValue<'scope, S::Failure>,
    ) -> Result<ExpandedState<'scope, S>, ProgramBuildError>
    where
        S::Failure: InhabitedFailureValue;
}

impl<'scope, S: State> ExpandedState<'scope, S> {
    pub fn into_fallible_endpoints(
        self,
    ) -> (
        ExpansionValue<'scope, S::Output>,
        ExpansionValue<'scope, S::Failure>,
    )
    where
        S::Failure: InhabitedFailureValue;
}

impl<'scope, S> StateExpansionScope<'scope, S>
where
    S: State<Failure = Never>,
{
    pub fn finish_infallible(
        self,
        output: ExpansionValue<'scope, S::Output>,
    ) -> Result<ExpandedState<'scope, S>, ProgramBuildError>;
}

impl<'scope, S> ExpandedState<'scope, S>
where
    S: State<Failure = Never>,
{
    pub fn into_infallible_output(self) -> ExpansionValue<'scope, S::Output>;
}
```

Only those three types implement the sealed `Execution` trait. `State::{Request, Returned,
SafeFailure}` and `Execution::access_type_ids` disappear. Request/response ABI is projected from
`S::Execution` and `C`; Runtime never receives two type declarations to compare with `TypeId`.

`SafeFailureDisposition` remains because it makes safe-failure settlement incapable of producing
returned-only invalid-evidence outcomes and can require success-only handling. `Lowering` remains
because pure operation expansion may replace an abstract state with a larger concrete graph. These
are distinct semantic relations, not duplicate request/response capability declarations.
Its sealed `into_outcome` method is the only erasure bridge: the success-only implementation wraps
its output as success, the may-fail implementation carries an already-typed output/failure outcome,
and the no-safe-failure/Pure relation is uninhabited. Runtime never switches on a disposition id or
recreates the deleted `StateSettlement`.

The current authoring meaning of `State::Capability`, `Capability`, `Direct`, and
`RequiresCapability` is renamed to `Lowering`, `StateLowering`, `DirectLowering`, and `ExpandWith`
in the same commit. It is pure compiler elaboration, not live capability access.

`StateExpansion` declares only the abstract ABI and stable recipe identity. The concrete
`StateExpansionImplementation<E>` is registered into the callback-free catalog and captures the
exact cloneable `ProgramStateBinding<T>` handles it needs; the higher-ranked scope prevents those
handles' endpoints from escaping or crossing candidates. This avoids an impossible static
`E::expand` that would have no way to obtain builder-issued bindings. Expansion implementations are
pure deterministic compiler recipes and receive no Runtime adapter, Store, filesystem, or network
capability.

Live callback construction belongs to `mfm-runtime`:

```rust
pub struct StateImplementation<S: State<Lowering = DirectLowering>> {
    inner: private::TypedStateImplementation<S>,
}

impl<S> StateImplementation<S>
where
    S: State<Execution = Pure, Lowering = DirectLowering>,
{
    pub fn pure(
        binding: StateImplementationBinding<S>,
        invoke: impl Fn(&S::Input) -> ProposedStateOutcome<S::Output, S::Failure>
            + Send
            + Sync
            + 'static,
    ) -> Self;
}

impl<S, C> StateImplementation<S>
where
    S: State<Execution = Read<C>, Lowering = DirectLowering>,
    C: ReadCapabilityContract,
{
    pub fn read(
        binding: StateImplementationBinding<S>,
        request: impl Fn(&S::Input) -> C::Request + Send + Sync + 'static,
        settle_returned: impl Fn(&S::Input, &C::Returned)
                -> ProposedStateOutcome<S::Output, S::Failure>
            + Send
            + Sync
            + 'static,
        settle_safe_failure: impl Fn(&S::Input, &C::SafeFailure)
                -> <S::SafeFailureDisposition as SafeFailureDisposition<
                    S::Execution,
                    S::Failure,
                >>::SafeFailureProposal<S::Output>
            + Send
            + Sync
            + 'static,
    ) -> Self;
}

impl<S, C> StateImplementation<S>
where
    S: State<Execution = Effect<C>, Lowering = DirectLowering>,
    C: EffectCapabilityContract,
{
    pub fn effect(
        binding: StateImplementationBinding<S>,
        request: impl Fn(&S::Input) -> C::Request + Send + Sync + 'static,
        settle_returned: impl Fn(&S::Input, &C::Returned)
                -> ProposedStateOutcome<S::Output, S::Failure>
            + Send
            + Sync
            + 'static,
        settle_safe_failure: impl Fn(&S::Input, &C::SafeFailure)
                -> <S::SafeFailureDisposition as SafeFailureDisposition<
                    S::Execution,
                    S::Failure,
                >>::SafeFailureProposal<S::Output>
            + Send
            + Sync
            + 'static,
    ) -> Self;
}
```

If the consuming domain needs an infallible convenience constructor, give it a distinct Rust name
such as `read_infallible`/`effect_infallible` constrained to `SafeFailure = NoSafeFailure`; inherent
methods cannot be overloaded by a where-clause. Do not pass `Option<settle_safe_failure>`. Keep
the returned settlement total over an already-ingressed, request-bound `C::Returned`; there is
no `StateSettlement::InvalidEvidence` posterior validator. If a returned value is illegal for the
request, refine `C::Returned` or reject it in `C::bind_returned` before observation commit. Keep the
typed safe-failure handling policy, failure mapping, panic containment, and redaction-safe callback
faults. Remove `StateSettlement`, `InvalidEvidence`, `StructuredStateCallbacks`, its public enum variants,
wrong-variant `Option` dispatch, mode/kind self-comparisons, and public erased callback traits.

Runtime erases a registered implementation only into this private exhaustive representation:

```rust
enum ErasedStateImplementation {
    Pure(PureHandle),
    Read(ReadHandle),
    Effect(EffectHandle),
}
```

No erased handle exposes all three modes. The `Read`/`Effect` variants carry their exact capability
and value-codec identities. The only exhaustive match lives at Runtime dispatch.

Required compile-fail tests:

- Read callbacks cannot register for an Effect state or vice versa;
- Pure cannot receive request/settlement callbacks;
- `C2` request/returned/safe-failure callbacks cannot register for `Read<C1>` or `Effect<C1>`;
- Effect cannot enter a FanOut policy that permits only Pure/Read;
- an `ExpandWith<E>` state cannot register a live `StateImplementation`, and an expansion cannot
  substitute endpoints with the wrong abstract Input/Output/Failure ABI;
- `StateImplementation` internals and erased handles cannot be constructed, cloned into another
  mode, serialized, or extracted; and
- there is no callable adapter API in `mfm-capabilities` or `mfm-program`.

### 3.3 `ProgramDocument`, `Program`, compiler, and catalog

`mfm-program` replaces the public Authored/Expanded/Certified authority graph with:

```rust
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProgramDocument {
    // Private, bounded normalized graph and exact frozen identities.
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ProgramRef(ContentRef);

pub struct Program {
    // Private normalized graph, indexes, document/ref, and catalog fingerprint.
}

#[derive(Clone)]
pub struct ProgramCatalog(Arc<ProgramCatalogInner>);

pub struct ProgramCatalogBuilder {
    // Private callback-free type/schema/component declarations and frozen profile.
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StateImplementationDescriptor {
    implementation_ref: ContentRef,
}

pub trait ImmutableBindingDescriptor: MfmValue {}

#[derive(Clone)]
pub struct ImmutableBindingObject {
    // Private strict bytes/refs/typed descriptor and exact ProgramCatalogBuilder brand.
}

pub struct StateImplementationBinding<S: State<Lowering = DirectLowering>> {
    // Private state contract/implementation refs, catalog brand, and S witness.
}

pub struct StateExpansionBinding<E: StateExpansion> {
    // Private abstract-state/expansion identities and catalog brand; no live implementation ref.
}

pub struct ProgramCandidate<
    Input: StructuredValue,
    Output: StructuredValue,
    Failure: FailureValue,
> {
    // Private non-Serde authoring/expansion IR.
}

pub struct TypedEntryProfile<
    Input: StructuredValue,
    Output: StructuredValue,
    Failure: FailureValue,
> {
    // Private catalog brand plus exact root input/output/failure/profile identities.
}

pub struct TypedProgram<
    Input: StructuredValue,
    Output: StructuredValue,
    Failure: FailureValue,
> {
    // Private Arc<Program>, exact catalog brand, and root ABI witnesses.
}

pub struct OperationScope<'scope, Input, Output, Failure>
where
    Input: StructuredValue,
    Output: StructuredValue,
    Failure: FailureValue,
{
    // Private catalog-branded graph builder and root ABI; never escapes the authoring closure.
}

pub struct OperationValue<'scope, T: Send + Sync + 'static> {
    // Private invariant endpoint handle branded to exactly one OperationScope lifetime.
}

pub struct OperationState<'scope, S: State> {
    // Private state token with typed output/failure endpoints in the same scope.
}

pub struct CompletedOperation<'scope, Input, Output, Failure>
where
    Input: StructuredValue,
    Output: StructuredValue,
    Failure: FailureValue,
{
    // Private closed root token; consumed by ProgramCatalog::author_candidate.
}

pub struct ProgramIngress<'a> {
    catalog: &'a ProgramCatalog,
    limits: ProgramIngressLimits,
}

#[derive(Clone)]
pub struct QualifiedValue {
    // Private canonical bytes, exact contract, typed object, and in-process catalog brand.
}

pub struct QualifiedTypedValue<T: StructuredValue> {
    value: QualifiedValue,
    _type: PhantomData<fn() -> T>,
}

pub struct QualifiedFailure<F: FailureValue> {
    value: QualifiedValue,
    _type: PhantomData<fn() -> F>,
}

pub struct QualifiedSafeFailure<F: SafeFailureValue> {
    value: QualifiedValue,
    _type: PhantomData<fn() -> F>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ExecutionBindingRef(ContentRef);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum ExecutionBindingDescriptor {
    Read(ExecutionBindingBody),
    Effect(ExecutionBindingBody),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionBindingBody {
    state_contract_ref: ContentRef,
    state_implementation_ref: ContentRef,
    capability_contract_ref: ContentRef,
    capability_implementation_ref: ContentRef,
    adapter_contract_ref: ContentRef,
    adapter_implementation_ref: ContentRef,
    binding_object_ref: ContentRef,
}

pub struct ReadExecutionBinding<S, C>
where
    S: State<Execution = Read<C>, Lowering = DirectLowering>,
    C: ReadCapabilityContract,
{
    // Private ref, descriptor, catalog brand, and mode/C witness.
}

pub struct EffectExecutionBinding<S, C>
where
    S: State<Execution = Effect<C>, Lowering = DirectLowering>,
    C: EffectCapabilityContract,
{
    // Private ref, descriptor, catalog brand, and mode/C witness.
}

pub trait ProgramStateBinding<S: State>:
    private::ProgramStateBindingSealed<S> + Send + Sync
{
    fn execution_binding_ref(&self) -> Option<&ExecutionBindingRef>;
}
```

The required public surface is:

```rust
impl ProgramCatalogBuilder {
    pub fn new(profile: FrozenProgramProfile) -> Self;
    pub fn register_entry_profile<
        Input: StructuredValue,
        Output: StructuredValue,
        Failure: FailureValue,
    >(
        &mut self,
        descriptor: EntryProfileDescriptor,
    ) -> Result<TypedEntryProfile<Input, Output, Failure>, CatalogBuildError>;
    pub fn register_state<S: State<Lowering = DirectLowering>>(
        &mut self,
        descriptor: StateImplementationDescriptor,
    ) -> Result<StateImplementationBinding<S>, CatalogBuildError>;

    pub fn register_expansion<E: StateExpansion>(
        &mut self,
        implementation: StateExpansionImplementation<E>,
    ) -> Result<StateExpansionBinding<E>, CatalogBuildError>;

    pub fn qualify_binding_object<T: ImmutableBindingDescriptor>(
        &mut self,
        value: T,
    ) -> Result<ImmutableBindingObject, BindingDescriptorError>;
    pub fn register_read<C: ReadCapabilityContract>(
        &mut self,
    ) -> Result<&mut Self, CatalogBuildError>;
    pub fn register_effect<C: EffectCapabilityContract>(
        &mut self,
    ) -> Result<&mut Self, CatalogBuildError>;

    pub fn register_read_binding<S, C>(
        &mut self,
        state: &StateImplementationBinding<S>,
        descriptor: AccessAdapterBindingDescriptor,
    ) -> Result<ReadExecutionBinding<S, C>, CatalogBuildError>
    where
        S: State<Execution = Read<C>, Lowering = DirectLowering>,
        C: ReadCapabilityContract;

    pub fn register_effect_binding<S, C>(
        &mut self,
        state: &StateImplementationBinding<S>,
        descriptor: AccessAdapterBindingDescriptor,
    ) -> Result<EffectExecutionBinding<S, C>, CatalogBuildError>
    where
        S: State<Execution = Effect<C>, Lowering = DirectLowering>,
        C: EffectCapabilityContract;

    pub fn binding_descriptor(
        &mut self,
        capability_implementation_ref: ContentRef,
        adapter_contract_ref: ContentRef,
        adapter_implementation_ref: ContentRef,
        binding_object: ImmutableBindingObject,
    ) -> Result<AccessAdapterBindingDescriptor, BindingDescriptorError>;

    pub fn finish(self) -> Result<ProgramCatalog, CatalogBuildError>;
}

#[derive(Clone)]
pub struct AccessAdapterBindingDescriptor {
    // Private strict capability/adapter implementation and binding-object identities.
}

impl StateImplementationDescriptor {
    pub fn new(implementation_ref: ContentRef) -> Result<Self, BindingDescriptorError>;
}

impl StateImplementationDescriptor {
    pub fn implementation_ref(&self) -> &ContentRef;
}

impl AccessAdapterBindingDescriptor {
    pub fn binding_object(&self) -> &ImmutableBindingObject;
    pub fn content_ref(&self) -> &ContentRef;
}

impl ImmutableBindingObject {
    pub fn schema_ref(&self) -> &SchemaId;
    pub fn content_ref(&self) -> &ContentRef;
    pub fn canonical_json(&self) -> &str;
}

impl ProgramCatalog {
    pub fn author_candidate<Input, Output, Failure>(
        &self,
        entry: &TypedEntryProfile<Input, Output, Failure>,
        bounds: ProgramConstructionBounds,
        build: impl for<'scope> FnOnce(
            OperationScope<'scope, Input, Output, Failure>,
        ) -> Result<CompletedOperation<'scope, Input, Output, Failure>, ProgramBuildError>,
    ) -> Result<ProgramCandidate<Input, Output, Failure>, ProgramBuildError>
    where
        Input: StructuredValue,
        Output: StructuredValue,
        Failure: FailureValue;

    pub fn finish<
        Input: StructuredValue,
        Output: StructuredValue,
        Failure: FailureValue,
    >(
        &self,
        candidate: ProgramCandidate<Input, Output, Failure>,
    ) -> Result<TypedProgram<Input, Output, Failure>, ProgramBuildError>;

    pub fn ingress(&self, limits: ProgramIngressLimits) -> ProgramIngress<'_>;

    pub fn fingerprint(&self) -> &ProgramCatalogFingerprint;

    pub fn shares_catalog_instance(&self, other: &Self) -> bool;
    pub fn owns_program(&self, program: &Program) -> bool;
    pub fn owns_value(&self, value: &QualifiedValue) -> bool;
    pub fn owns_state_binding<S: State<Lowering = DirectLowering>>(
        &self,
        binding: &StateImplementationBinding<S>,
    ) -> bool;
    pub fn owns_read_binding<S, C>(&self, binding: &ReadExecutionBinding<S, C>) -> bool
    where
        S: State<Execution = Read<C>, Lowering = DirectLowering>,
        C: ReadCapabilityContract;
    pub fn owns_effect_binding<S, C>(&self, binding: &EffectExecutionBinding<S, C>) -> bool
    where
        S: State<Execution = Effect<C>, Lowering = DirectLowering>,
        C: EffectCapabilityContract;

    pub fn qualify_typed<T: StructuredValue>(
        &self,
        value: T,
    ) -> Result<QualifiedTypedValue<T>, ValueQualificationError>;

    pub fn qualify_failure<F: FailureValue>(
        &self,
        failure: F,
    ) -> Result<QualifiedFailure<F>, ValueQualificationError>;

    pub fn qualify_safe_failure<F: SafeFailureValue>(
        &self,
        failure: F,
    ) -> Result<QualifiedSafeFailure<F>, ValueQualificationError>;

    pub fn ingress_value(
        &self,
        contract: &ValueContractRef,
        expected_content_ref: &ContentRef,
        canonical_bytes: &[u8],
    ) -> Result<QualifiedValue, ValueIngressError>;

    pub fn ingress_source_typed<T: StructuredValue>(
        &self,
        source_bytes: &[u8],
        limits: SourceValueIngressLimits,
    ) -> Result<QualifiedTypedValue<T>, ValueIngressError>;

    pub fn downcast_value<'a, T: StructuredValue>(
        &self,
        value: &'a QualifiedValue,
    ) -> Result<&'a T, ValueTypeError>;

    pub fn downcast_failure<'a, F: FailureValue>(
        &self,
        value: &'a QualifiedValue,
    ) -> Result<&'a F, ValueTypeError>;

    pub fn downcast_safe_failure<'a, F: SafeFailureValue>(
        &self,
        value: &'a QualifiedValue,
    ) -> Result<&'a F, ValueTypeError>;
}

impl QualifiedValue {
    pub fn contract_ref(&self) -> &ValueContractRef;
    pub fn content_ref(&self) -> &ContentRef;
    pub fn canonical_json(&self) -> &str;
}

impl<T: StructuredValue> QualifiedTypedValue<T> {
    pub fn erased(&self) -> &QualifiedValue;
    pub fn into_erased(self) -> QualifiedValue;
}

impl<T: StructuredValue> Clone for QualifiedTypedValue<T> {
    fn clone(&self) -> Self; // Manual: clones only the inner Arc, no T: Clone bound.
}

impl<F: FailureValue> QualifiedFailure<F> {
    pub fn erased(&self) -> &QualifiedValue;
    pub fn into_erased(self) -> QualifiedValue;
}

impl<F: SafeFailureValue> QualifiedSafeFailure<F> {
    pub fn erased(&self) -> &QualifiedValue;
    pub fn into_erased(self) -> QualifiedValue;
}

impl<F: SafeFailureValue> Clone for QualifiedSafeFailure<F> {
    fn clone(&self) -> Self; // Manual: inhabited values share the inner Arc; no F: Clone bound.
}

impl<'scope, Input, Output, Failure> OperationScope<'scope, Input, Output, Failure>
where
    Input: StructuredValue,
    Output: StructuredValue,
    Failure: FailureValue,
{
    pub fn input(&self) -> OperationValue<'scope, Input>;

    pub fn state<S, B>(
        &mut self,
        path: StatePath,
        binding: &B,
        input: OperationValue<'scope, S::Input>,
    ) -> Result<OperationState<'scope, S>, ProgramBuildError>
    where
        S: State,
        B: ProgramStateBinding<S>;

    pub fn finish_fallible(
        self,
        output: OperationValue<'scope, Output>,
        failure: OperationValue<'scope, Failure>,
    ) -> Result<CompletedOperation<'scope, Input, Output, Failure>, ProgramBuildError>
    where
        Failure: InhabitedFailureValue;
}

impl<'scope, Input, Output> OperationScope<'scope, Input, Output, Never>
where
    Input: StructuredValue,
    Output: StructuredValue,
{
    pub fn finish_infallible(
        self,
        output: OperationValue<'scope, Output>,
    ) -> Result<CompletedOperation<'scope, Input, Output, Never>, ProgramBuildError>;
}

impl<'scope, S: State> OperationState<'scope, S> {
    pub fn into_fallible_endpoints(
        self,
    ) -> (
        OperationValue<'scope, S::Output>,
        OperationValue<'scope, S::Failure>,
    )
    where
        S::Failure: InhabitedFailureValue;
}

impl<'scope, S> OperationState<'scope, S>
where
    S: State<Failure = Never>,
{
    pub fn into_infallible_output(self) -> OperationValue<'scope, S::Output>;
}

impl ProgramIngress<'_> {
    pub fn decode(
        &self,
        expected: &ProgramRef,
        canonical_bytes: &[u8],
    ) -> Result<Arc<Program>, ProgramIngressError>;
}

impl Program {
    pub fn reference(&self) -> &ProgramRef;
    pub fn document(&self) -> &ProgramDocument;
    pub fn catalog_fingerprint(&self) -> &ProgramCatalogFingerprint;
    // Narrow callback-free graph/index queries required by Store and Runtime.
}

impl<Input, Output, Failure> TypedProgram<Input, Output, Failure>
where
    Input: StructuredValue,
    Output: StructuredValue,
    Failure: FailureValue,
{
    pub fn as_program(&self) -> &Program;
    pub fn reference(&self) -> &ProgramRef;
    pub fn into_program(self) -> Arc<Program>;
}

impl<Input, Output, Failure> Clone for TypedProgram<Input, Output, Failure>
where
    Input: StructuredValue,
    Output: StructuredValue,
    Failure: FailureValue,
{
    fn clone(&self) -> Self; // Manual: clones only the private Arc; no ABI-type Clone bounds.
}

impl<Input, Output, Failure> Clone for TypedEntryProfile<Input, Output, Failure>
where
    Input: StructuredValue,
    Output: StructuredValue,
    Failure: FailureValue,
{
    fn clone(&self) -> Self; // Manual immutable branded-handle clone; no ABI-type Clone bounds.
}
```

`Program` has no `Deserialize`, public fields, public literal, unchecked constructor, or
`validate_program` function. `ProgramDocument` is freely serializable data but cannot be executed.
`ProgramRef` is an address and never executes without decoding through the exact catalog. The
catalog is an immutable bounded codec/type/descriptor registry, not an interior-mutable Program
cache: it has no `resolve(ProgramRef)` or unbounded interning map. Hot admission retains its returned
`Arc<Program>`; each cold qualified run owns the `Arc<Program>` decoded from its retained admission.
The fingerprint binds persisted semantics; it is not an in-process brand. Every `Program` carries
the private exact catalog-instance brand, `shares_catalog_instance` uses private `Arc` identity,
and admission, Runtime assembly, Store evidence, and typed-value downcasts require that exact
shared instance.

`ingress_source_typed` is the caller/configuration-planner source boundary: it bounds and strictly
decodes through `T`'s checked constructor, rejects duplicate/unknown/trailing input, canonicalizes
and addresses once, and returns the exact catalog-branded typed value without requiring a caller-
supplied digest. `ingress_value(expected_ref, ..)` remains the retained-byte boundary. Neither App
nor a planner reparses the source token.

`ProgramCatalogBuilder::finish` produces only a callback-free, immutable codec/compiler catalog.
That standalone product is legitimate for Store, Replay, import/export, and offline audit and
cannot mint a session or invocation. Runtime assembly is a separate live TCB composition step, not
a second Program validator. Builder-qualified binding objects and handles carry the exact private
catalog-instance brand, so an object qualified by one builder cannot be used under another.

The `owns_*` methods are the Program-owned exact-instance comparisons Runtime/Store call once while
consuming a live registration, Program admission, or locally typed value. None returns a reusable
proof or falls back to content/fingerprint equality. Add two-catalog, byte-identical transposition
tests. These one-time dynamic-erasure associations are not repeated semantic or byte validation.

The sealed `ProgramStateBinding<S>` is implemented only by `StateImplementationBinding<S>` when
`S::Execution = Pure` and `S::Lowering = DirectLowering`, by `ReadExecutionBinding<S, C>` and
`EffectExecutionBinding<S, C>` for the corresponding direct access modes, and by
`StateExpansionBinding<E>` for `E::AbstractState`. Thus every DSL state occurrence records its
exact implementation or expansion selection, and an access occurrence additionally records the
mode- and state-specific execution binding. A same-`C` binding for `S1` cannot be attached to `S2`.
Runtime registers the same cloneable secret-free direct handles against live callbacks; expansion
handles are compiler-only, are eliminated from the normalized final graph, and can never register
a callback. The handles are identities, not callable authority.

`StateExpansionBinding<E>` fixes `E::expansion_id()` and the exact abstract-state contract. The
scoped builder alone mints `ExpansionValue<T>` and `ExpandedState<S>`; their generic endpoints make
wrong output/failure substitution fail to compile instead of requiring a posterior endpoint-kind
comparison. The compiler consumes the final token and eliminates the abstract occurrence before
constructing `Program`.

The cloneable catalog handles use manual `Clone` implementations over their internal `Arc` and
`PhantomData<fn() -> ...>`. They impose no `S: Clone`, `C: Clone`, or `E: Clone` bound. Add
compile-pass fixtures with deliberately non-Clone marker types, alongside compile-fail fixtures
that cannot detach or reconstruct the handles.

Catalog construction rejects one persisted state/value/capability/binding contract identity mapped
to two Rust `TypeId`s, codecs, modes, or implementation descriptors. It also rejects one Rust type
registered under conflicting identities. This one-to-one registry invariant is checked at builder
insertion/finalization and never repaired during dispatch.

`qualify_failure` is the sealed erasure bridge for `State::Failure`. For an inhabited failure type,
its `mfm-values` implementation produces the same private `IntrinsicTypedValue` as ordinary typed
qualification; Program verifies the registered contract and adds its exact catalog brand. For the
uninhabited `Never`, `into_intrinsic(self)` is an exhaustive match. `downcast_failure` checks the
catalog brand and contract, then uses the retained `Any`; asking it to downcast an uninhabited type
returns `ValueTypeError::UninhabitedContract`. Runtime does not use specialization, a `TypeId`
branch, serialize/decode fallback, fake lexical schema, or a free constructor.

`qualify_safe_failure` is the identical sealed bridge for capability safe failure: inhabited
`MfmValue` implementations produce the normal intrinsic token, while `NoSafeFailure` is an
exhaustive match and produces no value/schema. Provide the capabilities-owned
`bind_no_safe_failure(&NoSafeFailure) -> !` helper/default delegation so no-safe-failure contracts
need no fake validator; do not use `TypeId` or an optional codec.
`downcast_safe_failure` is its inverse for committed/cold observation evidence: inhabited types use
the catalog's exact retained `Any` witness, while an uninhabited type returns
`ValueTypeError::UninhabitedContract`. It never decodes bytes again.

Both `ProgramCatalog::finish` and `ProgramIngress::decode` call one private
`normalize_and_construct_program` owner for final graph closure, bounds, profile, schema, lexical,
and identity invariants. Only hostile ingress performs byte/framing/canonicality/content-ref checks;
the hot path never serializes and decodes its own candidate. Add test-only counters proving one
global pass at finish/ingress and zero posterior validations.

Operation expansion is retained, not flattened away. Domain planning receives an already-ingressed
typed `ResolvedConfiguration<T>` and constructs a private `ProgramCandidate` through the DSL. The
pure compiler still performs child substitution, configuration specialization/fan-out, abstract
operation/state lowering, pre/proceed/post/failure injection, normalization, and the one final-
graph pass. Resume reads the final document and never reruns expansion. Any authored source or
derivation trace retained for auditing is content-addressed metadata with no Program constructor.

`OperationScope` is the sole mutable root-graph authoring owner. Its higher-ranked closure lifetime
brands every `OperationValue` and `OperationState`; a token from another operation or expansion
scope cannot type-check. `ProgramCandidate` is immutable after that closure closes and has no
`state`, edge-mutation, or raw endpoint API. `author_candidate` consumes only a typed closed root,
then `ProgramCatalog::finish` returns `TypedProgram<Input, Output, Failure>`. Add compile-fail
fixtures for cross-scope input/output/failure use and compile-pass coverage for fan-out/join and
infallible roots.

`mfm-program` does not depend on Store's concrete `ResolvedConfiguration<C>`. The App/domain
planner borrows `ValidatedConfig<C>` (or a narrow callback-free planning view) from that resolved
value and supplies ordinary typed planning inputs to the DSL before `ProgramCatalog::finish`.
This keeps configuration ingress with Store/Values and pure expansion with Program without a
dependency cycle or a second deserialization boundary.

Absorb these `mfm-spec` responsibilities into private/document modules in `mfm-program`:

- normalized graph/document DTOs and their strict codecs;
- execution/capability/state/operation manifest records;
- frozen profile and graph-bound definitions; and
- schema/lexical/component identity projections.

Absorb these `mfm-certify` responsibilities into `mfm-program`:

- recipe expansion/lowering;
- normalized graph/profile verification;
- manifest/document construction;
- program/schema/lexical indexes; and
- callback-free catalog association.

Move live registrations, erased callables, and assembly matching to Runtime. Delete every authored,
expanded, certified, admission-certification, verification-snapshot, entry-certifier, and borrowed-
certifier public authority type. Store and Replay accept only `Arc<Program>` resolved by the exact
catalog.

### 3.4 Immutable process assembly and adapter registration

Runtime assembly has one public builder and one opaque finished value:

```rust
pub struct RuntimeAssemblyBuilder {
    catalog: ProgramCatalog,
    // Private live typed registrations under construction.
}

pub struct RuntimeAssembly {
    catalog: ProgramCatalog,
    registry: ProcessRegistry,       // private and non-Clone
    brand: RuntimeAssemblyBrand,     // private, non-Serde; brands sessions to this process
}

pub struct ReadAdapterImplementation<C: ReadCapabilityContract> {
    // Private descriptor ref plus one mode-specific typed invocation package.
}

pub struct EffectAdapterImplementation<C: EffectCapabilityContract> {
    // Private descriptor ref plus one mode-specific typed invocation package.
}

impl<C> ReadAdapterImplementation<C>
where
    C: ReadCapabilityContract<Facts = NoPriorFacts>,
{
    pub fn without_prior_facts<F, Fut>(
        descriptor: AccessAdapterBindingDescriptor,
        invoke: F,
    ) -> Result<Self, AdapterImplementationError>
    where
        F: Fn(ReadAdapterCall<C>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<ReadCompletion<C>, AdapterCallError>> + Send + 'static;
}

impl<C> ReadAdapterImplementation<C>
where
    C: ReadCapabilityContract<Facts = PriorRunFacts>,
{
    pub fn with_prior_facts<Q, F, Fut>(
        descriptor: AccessAdapterBindingDescriptor,
        fact_request: Q,
        invoke: F,
    ) -> Result<Self, AdapterImplementationError>
    where
        Q: Fn(&C::Request) -> Result<FactSelectionRequest, FactRequestError>
            + Send
            + Sync
            + 'static,
        F: Fn(ReadAdapterCallAfterFacts<C>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<ReadCompletion<C>, AdapterCallError>> + Send + 'static;
}

impl<C> EffectAdapterImplementation<C>
where
    C: EffectCapabilityContract<Facts = NoPriorFacts>,
{
    pub fn without_prior_facts<F, Fut>(
        descriptor: AccessAdapterBindingDescriptor,
        invoke: F,
    ) -> Result<Self, AdapterImplementationError>
    where
        F: Fn(EffectAdapterCall<C>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<EffectCompletion<C>, AdapterCallError>> + Send + 'static;
}

impl<C> EffectAdapterImplementation<C>
where
    C: EffectCapabilityContract<Facts = PriorRunFacts>,
{
    pub fn with_prior_facts<Q, F, Fut>(
        descriptor: AccessAdapterBindingDescriptor,
        fact_request: Q,
        invoke: F,
    ) -> Result<Self, AdapterImplementationError>
    where
        Q: Fn(&C::Request) -> Result<FactSelectionRequest, FactRequestError>
            + Send
            + Sync
            + 'static,
        F: Fn(EffectAdapterCallAfterFacts<C>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<EffectCompletion<C>, AdapterCallError>> + Send + 'static;
}

impl RuntimeAssemblyBuilder {
    pub fn new(catalog: ProgramCatalog) -> Self;

    pub fn implement_state<S: State<Lowering = DirectLowering>>(
        &mut self,
        implementation: StateImplementation<S>,
    ) -> Result<&mut Self, AssemblyError>;

    pub fn bind_read<S, C>(
        &mut self,
        binding: ReadExecutionBinding<S, C>,
        implementation: ReadAdapterImplementation<C>,
    ) -> Result<&mut Self, AssemblyError>
    where
        S: State<Execution = Read<C>, Lowering = DirectLowering>,
        C: ReadCapabilityContract;

    pub fn bind_effect<S, C>(
        &mut self,
        binding: EffectExecutionBinding<S, C>,
        implementation: EffectAdapterImplementation<C>,
    ) -> Result<&mut Self, AssemblyError>
    where
        S: State<Execution = Effect<C>, Lowering = DirectLowering>,
        C: EffectCapabilityContract;

    pub fn finish(self) -> Result<RuntimeAssembly, AssemblyError>;
}
```

`ReadAdapterImplementation<C>` and `EffectAdapterImplementation<C>` are opaque, non-Serde live TCB
injection products. Their mode-specific constructors bind the exact catalog-created immutable
descriptor to the typed callback (and, when required, the fact-request callback) before assembly.
`RuntimeAssemblyBuilder::finish` compares that retained descriptor ref with the one inside the
state-specific execution binding; a caller cannot accidentally register a same-`C` adapter under a
different binding without rejection. The constructor does not prove an arbitrary callback is
honest: trusted composition's selected adapter implementation remains part of the TCB, as stated in
U4. The product prevents decomposition/reassociation and catches descriptor/configuration mistakes;
it is not a sandbox or attestation of linked code.

`ReadAdapterCall<C>` and `EffectAdapterCall<C>` have private fields and getters for the exact typed
request. They are separate nominal types because
Rust permits one `C` to implement both capability traits; overlapping inherent impls on one generic
call type would not compile. Application code cannot construct either call. The invocation closure
is captured immediately into a private `ErasedInvocationThunk`; no invoker, provider client,
signer, or generic resource handle is returned from registration. Concrete adapters own their
transports, signers, token refresh, and domain transaction machinery internally.

For prior-fact capabilities, Runtime—not adapter code—calls the registered deterministic
`fact_request`, drives the affine Store scanner through explicit retry/suspension, and invokes the
provider only after `FactScanStep::Selected`. The provider receives an
`*AdapterCallAfterFacts<C>` containing the exact typed request, selected response, and private call
identity; it never owns the scanner. This prevents an ordinary adapter `Result` or cancellation
from destroying the direct-commit-only continuation before selection.

All registered state callbacks and `fact_request` callbacks are synchronous typed functions, not
permission to block a Tokio worker. Runtime moves the owning typed action/request and callback handle
onto the bounded CPU executor specified in §5.1. The job borrows typed inputs only after the move,
catches panic there, and returns the still-affine action plus its typed proposal/request. Provider
response relation binding and canonical qualification use the same executor through the asynchronous
per-call completion methods in §5.3. The live provider future itself remains async I/O and is never
detached merely to catch a panic.

`ProgramCatalogBuilder` and `RuntimeAssemblyBuilder` own deliberately different propositions. The
first freezes callback-free semantics/codecs. The second associates one immutable set of live TCB
callbacks/adapters with that exact catalog instance. Runtime has no constructor from a raw registry,
and its builder cannot change the catalog. Multiple independently composed processes may use clones
of the same callback-free catalog; each receives a distinct Runtime brand and immutable registry.
That is deployment composition, not duplicated Program validation or hot replacement.

`finish` is the single association check. It requires exactly one implementation for every live
catalog component, exact mode/capability/request ABI, exact immutable binding descriptor, and exact
declared provider/route/signer/effect-domain identity. Missing, surplus, ambiguous, or mismatched
registrations reject assembly. This is exact association of immutable descriptors with selected TCB
products, not behavioral proof of callback code. `RuntimeAssembly`, `ProcessRegistry`, and the brand expose no
`replace`, `revoke`, `refresh`, `lease`, `current`, `generation`, or callable lookup API.

The Program fixes one `ExecutionBindingRef` per access state. `binding_object_ref` names the exact
capability-specific strict, secret-free immutable descriptor for provider/route policy, signer,
wallet store/domain, or Effect domain. Do not copy those identities into multiple journal fields.
Put the cross-layer address newtypes `ProgramRef`, `ProgramCatalogFingerprint`,
`ExecutionBindingRef`, `RecordRef`, `JournalHead`, `TenantFactFrontier`, `TypedValueRef`, and their
contained address/digest newtypes in `mfm-ids`. Content-address wrappers have strict checked public
conversion to/from the underlying `ContentRef`; structured coordinates have private fields and
checked constructors/Serde. They are serialized addresses, not authority, so constructing one does
not qualify what it names. Program and Journal still exclusively derive their respective semantic
preimages and resolve/check what those addresses name. Do not add hidden unchecked constructors,
feature-gated friendship, or a Journal→Runtime dependency.

Signing retains one immutable descriptor, not a generation fence:

```rust
pub struct SignerBindingDescriptor {
    signer_ref: SignerRef,
    provider_implementation_ref: ContentRef,
    algorithm: SigningAlgorithm,
    profile: SigningProfile,
    public_identity: PublicSignerIdentity,
    key_instance_ref: ContentRef,
}

impl SignerBindingDescriptor {
    pub fn new(
        signer_ref: SignerRef,
        provider_implementation_ref: ContentRef,
        algorithm: SigningAlgorithm,
        profile: SigningProfile,
        public_identity: PublicSignerIdentity,
    ) -> Result<Self, SignerBindingError>;

    pub fn key_instance_ref(&self) -> &ContentRef;
    pub fn content_ref(&self) -> Result<ContentRef, SignerBindingError>;
}
```

Secret key material is live adapter state and never enters this value. The constructor derives
`key_instance_ref` internally from one domain-separated canonical preimage over `signer_ref`, the
provider implementation, algorithm, profile, and qualified public signer identity. It never accepts
an independently supplied ref that could be paired with the wrong public key. The exact concrete
adapter factory compares the key identity reported by its keystore/HSM ingress with this descriptor
before constructing `EffectAdapterImplementation<C>`. `key_instance_ref` fixes immutable key
identity; it is not a latest generation or revocation claim.

The concrete keystore bridge must respect the existing `Keystore: !Send + !Sync` contract. Trusted
composition starts one bounded dedicated OS-thread actor from checked path/configuration and a
zeroizing credential. That thread constructs, unlocks, uses, locks, and drops `Keystore` entirely on
the same thread. A cloneable `Send + Sync` handle carried by the immutable EVM adapter sends bounded
concrete signing commands and receives one-shot results; it exposes neither `Keystore` nor raw key
material. Queue saturation is a typed pre-entry provider fault, actor panic/exit fails closed, and
process drain stops intake, completes or rejects queued commands, joins the thread, and zeroizes on
drop. Do not use `Arc<Mutex<Keystore>>`, add an unsafe `Send`/`Sync` implementation, or move an open
keystore through `spawn_blocking`. Keep compile assertions for `!Send`/`!Sync` and tests for bounded
queueing, shutdown/drain, panic, credential redaction, AAD anti-swap binding, and zeroization.

Binding replacement is operational: stop facade intake, drain or terminally park the complete
reservation/invocation/observation bracket, drop the assembly, construct and qualify a new one. A
protocol token may renew inside the same concrete adapter only if its immutable MFM descriptor,
provider identity, signer identity, and effect domain do not change.

## 4. Store semantic core and backend APIs

### 4.1 One qualified value path

The one-ingress rule applies to value payloads, not only programs and batches. The opaque
cross-crate `QualifiedValue` in §3.3 is owned by `mfm-program`, the common owner of the exact type
catalog and codecs. Store retains those values in a private callback-free index:

```rust
struct QualifiedValueIndex {
    by_typed_ref: BTreeMap<(ValueContractRef, ContentRef), QualifiedValue>,
    canonical_by_content: BTreeMap<ContentRef, Arc<str>>, // Optional byte dedup only.
}
```

Cold ingress strictly decodes a value once through the exact Program catalog codec, checks its
content reference and intrinsic invariants, and stores both canonical bytes and its erased typed
value. Locally produced callback/configuration/provider values are already typed; their constructor
canonicalizes and content-addresses once, then moves the typed value into the prepared successor.
Runtime settlement calls only `ProgramCatalog::downcast_value` under the exact shared in-process
catalog instance and Runtime assembly brand. A matching persisted catalog fingerprint is necessary
for ingress but is not sufficient for an in-process downcast. Runtime must not serialize and
deserialize a just-produced request, response, state input, output, failure, or fact to recover its
type.

The erased `Any` value is never serialized and is not part of hash identity. Its canonical bytes
are. A mismatch between a value contract and the private downcast is an internal construction fault,
not a reason to retry decoding. Add counters proving one cold decode and zero hot re-decodes.

The hot-path premise requires actual valid-by-representation Rust values; current `MfmValue`
membership alone is not enough. Before cutover, inventory every retained state/capability/fact/
configuration/domain value and make every invalid state unrepresentable: private fields, checked
constructors, checked custom `Deserialize`, bounded collections/strings, and opaque semantic
newtypes. Primitive/full-domain structs may document why every bit pattern is valid. The derive may
generate those delegations but may not bless public-field DTOs. `qualify_typed` therefore performs
one canonicalization/content-addressing operation and no semantic validator. Hostile deserialization
uses the same checked constructors once. Add source/UI tests preventing local invalid struct
literals plus hostile Serde tests for every former validator family. If an inventoried type cannot
meet this rule, stop and amend the RFC rather than hiding a validator in a posterior layer.

Typed evidence is keyed by the full `(ValueContractRef, ContentRef)`/`TypedValueRef`, never content
bytes alone. Identical canonical bytes may inhabit two distinct nominal contracts; byte dedup may
share only the canonical `Arc<str>`, not the typed object or downcast witness. Test both insertion
orders for same bytes under two contracts.

`QualifiedValue` is public only as an opaque Program-to-Store-to-Runtime SPI product. Its fields,
brand, typed object, and constructors stay private to Program; it has no Serde implementation or
unchecked downcast. Do not add a generic “validated value” hierarchy whose users can forge
provenance or repeatedly convert between representations.

### 4.2 Semantic values and the sole reducer

`mfm-store` owns these opaque values:

```rust
pub struct QualifiedRun(Arc<QualifiedRunInner>);

pub struct ActiveQualifiedRun {
    owner: Arc<HistoryCoordinatorInner>,
    run: QualifiedRun,
    // Private, non-Clone, non-Serde; only QualifiedHistoryPort/direct commit mints it.
}

pub enum PlainEvent {}
pub enum AccessReservationEvent {}
pub enum AccessObservationEvent {}

pub trait EventKind: private::EventKindSealed + Send + Sync + 'static {}

pub struct ResolvedEvent<K: EventKind> {
    inner: ResolvedEventKind,
    _kind: PhantomData<fn() -> K>,
}

pub struct ObservationRebaseInput {
    // Store-owned callback-free fixation; private and non-Serde.
}

struct PendingAppend<K: EventKind> {
    record_draft: RecordDraft,
    successor: UnboundSuccessor,
    obligations: SemanticObligations,
    _kind: PhantomData<fn() -> K>,
}

struct RebindSeed<K: EventKind> {
    // Resolved event + unbound successor + invariant predecessor/capacity evidence; no callback.
    _kind: PhantomData<fn() -> K>,
}

#[must_use]
pub struct PreparedAppend<K: EventKind> {
    owner: Arc<HistoryCoordinatorInner>,
    predecessor: Option<PreparedPredecessorEvidence>,
    command: BackendAppendCommand,
    successor: PreparedSuccessor,
    fixation: AppendFixation,
    index_plan: AppendIndexPlan,
    rebind: RebindSeed<K>,
    _kind: PhantomData<fn() -> K>,
}

#[must_use]
pub enum ReadReservation {}
pub enum EffectReservation {}

pub trait ReservationMode<C>:
    private::ReservationModeSealed<C> + Send + Sync + 'static
{
    type Request: MfmValue;
    type Facts: FactSelectionMode;
}

impl<C: ReadCapabilityContract> ReservationMode<C> for ReadReservation {
    type Request = C::Request;
    type Facts = C::Facts;
}

impl<C: EffectCapabilityContract> ReservationMode<C> for EffectReservation {
    type Request = C::Request;
    type Facts = C::Facts;
}

pub struct PreparedReservationAppend<K, C>
where
    K: ReservationMode<C>,
{
    append: PreparedAppend<AccessReservationEvent>,
    request: QualifiedTypedValue<K::Request>,
    // Exact Store-derived reservation fixation; private and never copied into Runtime.
    _mode: PhantomData<fn() -> (K, C)>,
}

pub enum ReservationPrepareDisposition<K, C>
where
    K: ReservationMode<C>,
{
    Prepared(PreparedReservationAppend<K, C>),
    Retryable {
        preparation: RetryReservationPreparation<K, C>,
        error: RetryablePrepareError,
    },
    ResumeRequired {
        run_id: RunId,
        cause: ResumeCause,
    },
    Failed(TerminalPrepareError),
}

pub struct RetryReservationPreparation<K, C>
where
    K: ReservationMode<C>,
{
    // Owner-bound selected action + resolved request/event; private, non-Clone.
    _mode: PhantomData<fn() -> (K, C)>,
}

#[must_use]
pub struct PreparedObservationAppend {
    append: PreparedAppend<AccessObservationEvent>,
    rebase: ObservationRebaseInput,
}

pub enum StorePrepareDisposition<K: EventKind> {
    Prepared(PreparedAppend<K>),
    Retryable {
        preparation: RetryAppendPreparation<K>,
        error: RetryablePrepareError,
    },
    ResumeRequired {
        run_id: RunId,
        cause: ResumeCause,
    },
    Failed(TerminalPrepareError),
}

pub enum ObservationPrepareDisposition {
    Prepared(PreparedObservationAppend),
    Retryable {
        preparation: RetryObservationPreparation,
        error: RetryablePrepareError,
    },
    RebaseRequired {
        input: ObservationRebaseInput,
        cause: ResumeCause,
    },
    Failed(TerminalPrepareError),
}

pub struct RetryAppendPreparation<K: EventKind> {
    // Owner-bound predecessor + already-resolved event/value; private, non-Clone.
}

pub struct RetryObservationPreparation {
    // Owner-bound predecessor + response event + rebase input; private, non-Clone.
}

impl<K: EventKind> RetryAppendPreparation<K> {
    pub async fn retry(self) -> StorePrepareDisposition<K>;
}

impl<K, C> RetryReservationPreparation<K, C>
where
    K: ReservationMode<C>,
{
    pub async fn retry(self) -> ReservationPrepareDisposition<K, C>;
}

impl RetryObservationPreparation {
    pub async fn retry(self) -> ObservationPrepareDisposition;
}
```

`QualifiedRun` is callback-free and deliberately non-Serde. It contains exact store
identity/epoch, tenant, run id, head,
`Arc<Program>`, admitted immutable context, reduced state/frontier, complete retained object and
typed-value indexes, fact closure, access-reservation/observation state, and cumulative capacity.
Its heavy immutable components may use private `Arc`s, but evidence ownership is explicit. Its
fields and constructors are private. Its public methods are narrow immutable identifiers plus
projection entry points; it exposes no reducer state for callers to edit.

`QualifiedRun` carries no backend/coordinator and cannot prepare or commit anything; HistoryReader,
Replay, export, and audit receive only this type. It may share immutable evidence internally by
`Arc`. `ActiveQualifiedRun` is the separate affine writer-capable continuation: it owns one
`QualifiedRun` plus the exact private history coordinator and is minted only by Runtime's port
resume/admission/direct-successor path. Runtime sessions own only `ActiveQualifiedRun`.

The Runtime boundary is one consuming selected-action typestate, not a cloneable action proof,
reducer internals, or a second scheduler:

```rust
pub enum SelectedRunAction {
    Pure(SelectedPureAction),
    Read(SelectedReadAction),
    Effect(SelectedEffectAction),
    UnobservedReadReservation(SelectedReadReservationAction),
    UnobservedEffectReservation(SelectedEffectReservationAction),
    SettleReadObservation(SelectedReadSettlementAction),
    SettleEffectObservation(SelectedEffectSettlementAction),
    Waiting(SelectedWaitingRun),
    Closed(SelectedClosedRun),
}

pub struct StateActionView<'a> {
    pub state_contract_ref: &'a ContentRef,
    pub input: &'a QualifiedValue,
}

pub struct AccessActionView<'a> {
    pub state: StateActionView<'a>,
    pub execution_binding_ref: &'a ExecutionBindingRef,
    pub recovery: AccessRecoveryView<'a>,
}

pub struct TypedPureAction<S: State<Execution = Pure, Lowering = DirectLowering>> {
    // SelectedPureAction plus exact S/catalog witness.
}

pub struct TypedReadAction<S, C> {
    // SelectedReadAction plus exact S/C/execution-binding witness.
}

pub struct TypedEffectAction<S, C> {
    // SelectedEffectAction plus exact S/C/execution-binding witness.
}

pub struct TypedReadReservationAction<S, C> {
    // Selected unobserved reservation plus exact S/C/reservation witness.
}

pub struct TypedEffectReservationAction<S, C> {
    // Selected unobserved reservation plus exact S/C/reservation witness.
}

pub struct TypedReadSettlementAction<S, C> {
    // Selected durable observation plus exact S/C/outcome witness.
}

pub struct TypedEffectSettlementAction<S, C> {
    // Selected durable observation plus exact S/C/outcome witness.
}

impl ActiveQualifiedRun {
    pub fn select(self) -> SelectedRunAction;
    pub fn evidence(&self) -> &QualifiedRun;
}

impl SelectedPureAction {
    pub fn view(&self) -> StateActionView<'_>;
    pub fn bind<S>(
        self,
        binding: &StateImplementationBinding<S>,
    ) -> Result<TypedPureAction<S>, ActionBindingError>
    where
        S: State<Execution = Pure, Lowering = DirectLowering>;
}

impl SelectedReadAction {
    pub fn view(&self) -> AccessActionView<'_>;
    pub fn bind<S, C>(
        self,
        binding: &ReadExecutionBinding<S, C>,
    ) -> Result<TypedReadAction<S, C>, ActionBindingError>
    where
        S: State<Execution = Read<C>, Lowering = DirectLowering>,
        C: ReadCapabilityContract;
}

impl SelectedEffectAction {
    pub fn view(&self) -> AccessActionView<'_>;
    pub fn bind<S, C>(
        self,
        binding: &EffectExecutionBinding<S, C>,
    ) -> Result<TypedEffectAction<S, C>, ActionBindingError>
    where
        S: State<Execution = Effect<C>, Lowering = DirectLowering>,
        C: EffectCapabilityContract;
}

impl SelectedWaitingRun {
    pub fn view(&self) -> WaitView<'_>;
    pub fn into_active(self) -> ActiveQualifiedRun;
}

impl SelectedClosedRun {
    pub fn run_id(&self) -> &RunId;
    pub fn view(&self) -> ClosedView<'_>;
    pub fn into_terminal_evidence(self) -> ClosedRunEvidence;
}
```

Every selected action privately owns the exact `ActiveQualifiedRun`, head, occurrence,
input, and mode fixation. It is non-Clone/non-Serde and has no constructor. Runtime first borrows
the view only to choose its already-resolved dispatch; consuming `bind` associates the dynamic
selection with the exact catalog-issued typed handle once. The resulting typed action exposes
`&S::Input` through the already-retained typed value and owns the only proposal methods below.
There is no `ActionRef`, copied correlation digest, separately supplied predecessor, or posterior
same-run comparison.

The only state transition is:

```rust
fn apply<K: EventKind>(
    previous: Option<&QualifiedRun>,
    event: ResolvedEvent<K>,
) -> Result<PendingAppend<K>, SemanticError>;

fn bind_append<K: EventKind>(
    pending: PendingAppend<K>,
    context: AppendContext,
) -> Result<PreparedAppend<K>, PrepareAppendError>;

fn bind_retained<K: EventKind>(
    previous: Option<&QualifiedRun>,
    pending: PendingAppend<K>,
    retained: RetainedBatchFixation,
) -> Result<QualifiedRun, HistoryIngressError>;
```

These functions are private. `AppendContext` explicitly contains store identity/epoch, expected
predecessor, append request id, tenant fact coordinate/frontier, existing object set/capacity, and
the enabled attention-projection contract. It performs no ambient query. `RetainedBatchFixation`
contains the complete bounded retained envelope: every assigned record including adjacent close,
every exact object, append id, predecessor/head, fact coordinate, store identity, and index-relevant
field. Both binders consume the same `PendingAppend`; retained ingress additionally checks exact
projection and absence of surplus/ignored bytes.

Local source-specific proposal construction is available only by consuming the selected typed
action. Representative signatures:

```rust
impl<S> TypedPureAction<S>
where
    S: State<Execution = Pure, Lowering = DirectLowering>,
{
    pub fn input(&self) -> &S::Input;
    pub async fn prepare_transition(
        self,
        outcome: ProposedStateOutcome<S::Output, S::Failure>,
        append_id: AppendRequestId,
    ) -> StorePrepareDisposition<PlainEvent>;
}

impl<S, C> TypedReadAction<S, C>
where
    S: State<Execution = Read<C>, Lowering = DirectLowering>,
    C: ReadCapabilityContract,
{
    pub fn input(&self) -> &S::Input;
    pub async fn prepare_reservation(
        self,
        request: QualifiedTypedValue<C::Request>,
        append_id: AppendRequestId,
    ) -> ReservationPrepareDisposition<ReadReservation, C>;
}

impl<S, C> TypedEffectAction<S, C>
where
    S: State<Execution = Effect<C>, Lowering = DirectLowering>,
    C: EffectCapabilityContract,
{
    pub fn input(&self) -> &S::Input;
    pub async fn prepare_reservation(
        self,
        request: QualifiedTypedValue<C::Request>,
        append_id: AppendRequestId,
    ) -> ReservationPrepareDisposition<EffectReservation, C>;
}

impl SelectedReadReservationAction {
    pub fn view(&self) -> AccessActionView<'_>;
    pub fn bind<S, C>(
        self,
        binding: &ReadExecutionBinding<S, C>,
    ) -> Result<TypedReadReservationAction<S, C>, ActionBindingError>
    where
        S: State<Execution = Read<C>, Lowering = DirectLowering>,
        C: ReadCapabilityContract;
}

impl<S, C> TypedReadReservationAction<S, C>
where
    S: State<Execution = Read<C>, Lowering = DirectLowering>,
    C: ReadCapabilityContract,
{
    pub fn input(&self) -> &S::Input;
    pub fn request(&self) -> &C::Request;
    pub async fn prepare_repeat(
        self,
        append_id: AppendRequestId,
    ) -> ReservationPrepareDisposition<ReadReservation, C>;
}

impl SelectedEffectReservationAction {
    pub fn view(&self) -> AccessActionView<'_>;
    pub fn bind<S, C>(
        self,
        binding: &EffectExecutionBinding<S, C>,
    ) -> Result<TypedEffectReservationAction<S, C>, ActionBindingError>
    where
        S: State<Execution = Effect<C>, Lowering = DirectLowering>,
        C: EffectCapabilityContract;
}

impl<S, C> TypedEffectReservationAction<S, C>
where
    S: State<Execution = Effect<C>, Lowering = DirectLowering>,
    C: EffectCapabilityContract,
{
    pub fn input(&self) -> &S::Input;
    pub fn request(&self) -> &C::Request;
}

impl<S, C> TypedEffectReservationAction<S, C>
where
    S: State<Execution = Effect<C>, Lowering = DirectLowering>,
    C: EffectCapabilityContract<Entry = EntryOnce>,
{
    pub fn park_possible_entry(self) -> ActiveQualifiedRun;
}

impl<S, C, const MAX: u16> TypedEffectReservationAction<S, C>
where
    S: State<Execution = Effect<C>, Lowering = DirectLowering>,
    C: EffectCapabilityContract<Entry = EntryAbsorbing<MAX>>,
{
    pub async fn prepare_reassert(
        self,
        append_id: AppendRequestId,
    ) -> ReservationPrepareDisposition<EffectReservation, C>;
}

impl SelectedReadSettlementAction {
    pub fn view(&self) -> AccessActionView<'_>;
    pub fn bind<S, C>(
        self,
        binding: &ReadExecutionBinding<S, C>,
    ) -> Result<TypedReadSettlementAction<S, C>, ActionBindingError>
    where
        S: State<Execution = Read<C>, Lowering = DirectLowering>,
        C: ReadCapabilityContract;
}

impl SelectedEffectSettlementAction {
    pub fn view(&self) -> AccessActionView<'_>;
    pub fn bind<S, C>(
        self,
        binding: &EffectExecutionBinding<S, C>,
    ) -> Result<TypedEffectSettlementAction<S, C>, ActionBindingError>
    where
        S: State<Execution = Effect<C>, Lowering = DirectLowering>,
        C: EffectCapabilityContract;
}

pub enum TypedReadObservation<'a, C: ReadCapabilityContract> {
    Returned(&'a C::Returned),
    SafeFailure(&'a C::SafeFailure),
}

pub enum TypedEffectObservation<'a, C: EffectCapabilityContract> {
    Returned(&'a C::Returned),
    SafeFailure(&'a C::SafeFailure),
}

impl<S, C> TypedReadSettlementAction<S, C>
where
    S: State<Execution = Read<C>, Lowering = DirectLowering>,
    C: ReadCapabilityContract,
{
    pub fn input(&self) -> &S::Input;
    pub fn observation(&self) -> TypedReadObservation<'_, C>;
    pub async fn prepare_settlement(
        self,
        outcome: ProposedStateOutcome<S::Output, S::Failure>,
        append_id: AppendRequestId,
    ) -> StorePrepareDisposition<PlainEvent>;
}

impl<S, C> TypedEffectSettlementAction<S, C>
where
    S: State<Execution = Effect<C>, Lowering = DirectLowering>,
    C: EffectCapabilityContract,
{
    pub fn input(&self) -> &S::Input;
    pub fn observation(&self) -> TypedEffectObservation<'_, C>;
    pub async fn prepare_settlement(
        self,
        outcome: ProposedStateOutcome<S::Output, S::Failure>,
        append_id: AppendRequestId,
    ) -> StorePrepareDisposition<PlainEvent>;
}

impl<K: EventKind> PreparedAppend<K> {
    pub async fn commit(self) -> StoreCommitDisposition<K>;
}

impl PreparedObservationAppend {
    pub async fn commit(self) -> ObservationCommitDisposition;
}
```

The retained bytes for an outstanding reservation are identical whether Runtime still owns a hot
in-memory bracket or the process crashed before observing one. The direct-new hot bracket never
reselects history: its typed `ObservationWriteContinuation<K,C>` moves through
`ReadyToInvoke`/`AcceptedAccessResponse` and directly prepares the observation. A cold resume has no
such continuation and Store therefore selects `Unobserved*Reservation` with recovery methods only.
Read may consume it into `prepare_repeat`; `EntryOnce` can only park as `PossibleEntry`; and
`EntryAbsorbing<MAX>` can only consume it into the bounded same-request/effect-domain reassertion
path. Cold history cannot call an observation constructor, and no `Option<response>` or hidden
callback distinguishes the cases.

Once an observation is durable, the sole selector yields `Settle*Observation`, both after direct
commit and after cold resume. The typed observation view downcasts through the exact catalog once
without decoding, Runtime calls the relevant returned/safe-failure settlement callback, and the
selected action alone prepares the following state transition. A crash between observation commit
and settlement therefore resumes the same action rather than a special replay path.

`ObservationPrepareDisposition::RebaseRequired` consumes the known-stale preparation into its
owner-bound `ObservationRebaseInput`, including the qualified response event; Runtime moves it
directly into `SuspendedRun`. `RetryObservationPreparation` is reserved for a transient prepare or
fact-frontier rebind that can retry without loading a newer head. Every post-invocation
pre-observation nonterminal branch preserves the appropriate package.
There is no ownerless `ResumeRequired` branch that can discard a response or permit reinvocation.

The typed action methods qualify a locally typed outcome/request once through their retained exact
catalog and internally create the one `ResolvedEvent`; they do not deserialize it. Runtime may
manually clone a `QualifiedTypedValue<C::Request>` (without a `C: Clone` bound) so the immutable
Arc-backed typed value is shared by Store preparation and the exact invocation thunk. That shares
one value identity, not two validity proofs. Hostile committed records enter through private
record-family qualifiers that produce the same `ResolvedEvent`; there is no public
`ResolvedEvent` literal, proposal DTO, raw-record constructor, or Boolean mode. Observation methods
mint event and rebase input together into `PreparedObservationAppend`, which keeps them structurally
together through direct, stale, and ambiguous outcomes.

Delete `QualifiedEvent::{Intent, Recorded}`, `QualifiedIntentEvent`, `PrimaryIntent`,
`AuthorizationIntent`, parallel `reduce_*_intent`/`reduce_recorded_*` branches,
`PendingSemanticStep::semantic_eq`, `ComparisonPassed`, `ComparedReduction`, local
`qualify_recorded_successor`, just-authored object-index reconstruction, and any comparison between
two reducer results. The encode/cold-ingress equivalence corpus remains a test of one law, not a
production branch.

### 4.3 Demand ingress and the opaque Runtime port

Store has one open path over one composite backend object. Trusted composition supplies the exact
expected identity, Program catalog instance, and work limits; `open` qualifies that same object
once and derives every port from one private coordinator. There is no history-only,
configuration-only, fact-only, audit-only, or already-qualified constructor:

```rust
#[derive(Clone, Debug)]
pub struct StoreWorkLimits {
    pub max_concurrent_prefix_ingress: NonZeroUsize,
    pub max_prefix_frames: NonZeroU64,
    pub max_prefix_canonical_bytes: NonZeroU64,
    pub max_configuration_revisions: NonZeroU64,
    pub max_configuration_canonical_bytes: NonZeroU64,
    pub max_fact_publications_per_read: NonZeroU64,
    pub max_fact_canonical_bytes_per_read: NonZeroU64,
    // Present only in the enabled U3 design.
    pub effect_attention_inventory: EffectAttentionInventoryLimit,
}

pub struct StructuredStore;

impl StructuredStore {
    pub async fn open(
        backend: Arc<dyn StructuredStoreBackend>,
        expected_identity: StructuredStoreIdentity,
        catalog: ProgramCatalog,
        limits: StoreWorkLimits,
    ) -> Result<OpenedStructuredStore, StoreOpenError>;
}

pub struct QualifiedHistoryPort {
    // Non-Clone opaque owner of backend, catalog, Store epoch, and shared ingress limiter.
}

pub struct AdmissionMaterialScope<Input, Output, Failure>
where
    Input: StructuredValue,
    Output: StructuredValue,
    Failure: FailureValue,
{
    // Exact Store/catalog/tenant/entry-profile owner and admission-material bounds; private.
}

pub struct AdmissionContextCandidate {
    // Private valid-by-representation immutable context values minted by AdmissionMaterialScope.
}

pub struct FactSourceManifestCandidate {
    // Private bounded source Program/head rules minted by AdmissionMaterialScope.
}

pub struct TypedAdmissionInput<C, Input, Output, Failure>
where
    C: MfmConfig,
    Input: StructuredValue,
    Output: StructuredValue,
    Failure: FailureValue,
{
    // Exact owner, tenant, typed Program/root, context, config evidence, and fact rules.
}

#[derive(Clone)]
pub struct HistoryReader {
    // Callback-free demand reader over the same qualified backend/catalog.
}

pub struct OpenedStructuredStore {
    runtime_port: QualifiedHistoryPort,
    reader: HistoryReader,
    configuration: ConfigurationStore,
    audit: StoreAuditPort,
}

impl QualifiedHistoryPort {
    #[doc(hidden)]
    pub fn admission_material_scope<Input, Output, Failure>(
        &self,
        tenant: &TenantScopeId,
        entry: &TypedEntryProfile<Input, Output, Failure>,
    ) -> Result<AdmissionMaterialScope<Input, Output, Failure>, AdmissionMaterialError>
    where
        Input: StructuredValue,
        Output: StructuredValue,
        Failure: FailureValue;
}

impl<Input, Output, Failure> AdmissionMaterialScope<Input, Output, Failure>
where
    Input: StructuredValue,
    Output: StructuredValue,
    Failure: FailureValue,
{
    #[doc(hidden)]
    pub fn context<I>(
        &self,
        values: I,
    ) -> Result<AdmissionContextCandidate, AdmissionContextError>
    where
        I: IntoIterator<Item = QualifiedValue>;

    #[doc(hidden)]
    pub fn fact_sources<I>(
        &self,
        rules: I,
    ) -> Result<FactSourceManifestCandidate, FactSourceManifestError>
    where
        I: IntoIterator<Item = PriorRunFactSourceRule>;
}

impl OpenedStructuredStore {
    pub fn into_parts(self) -> StoreParts;
}

pub struct StoreParts {
    runtime_port: QualifiedHistoryPort,
    reader: HistoryReader,
    configuration: ConfigurationStore,
    audit: StoreAuditPort,
    // One private coordinator/catalog brand retained across the consuming process split.
}

pub struct StoreAuditPort {
    // Opaque, non-Clone owner of the combined backend snapshot capability.
}
```

`StoreParts` is an opaque, non-`Clone` process-composition product. It has no public field, per-port
getter, constructor, tuple conversion, or `Deserialize` implementation. `ProcessComposition`
accepts it whole and its `qualify` implementation uses one doc-hidden consuming Store-to-App SPI to
split the four capabilities exactly once; every output retains the same private coordinator/catalog
brand, which Runtime and process composition compare by `Arc::ptr_eq`. The split cannot accept or
replace a separately supplied port, so two Store opens cannot be mixed into one composed process.
Only `StructuredStore::open` constructs `QualifiedHistoryPort`; there is no public constructor that
turns a raw backend implementation, one of its purpose subtraits, or
a raw qualification result directly into semantic/write authority. `open` checks that
`backend.identity()` equals `expected_identity`, qualifies schema/channel/writer role and
`PrimaryCrashRestart`, fixes the exact `ProgramCatalog` instance, and installs one shared limits and
CPU-work coordinator before returning anything. A mismatch destroys the candidate and returns
`StoreOpenError`; no partially opened port escapes. Mechanical backend traits are intentionally
public TCB injection seams and any crate may implement them; an implementation becomes active only
when trusted composition selects the one composite object and Store qualifies it. Opacity protects
semantic products, not the implementability of the SPI. Memory and PostgreSQL conformance backends
pass the same `open` path and cannot mix identities or subtrait implementations from different
objects; PostgreSQL alone is the production backend that additionally freezes the
`PrimaryCrashRestart` deployment proposition described in §4.5.

Store open requires `StoreWorkLimits`. `QualifiedHistoryPort`, every cloned `HistoryReader`,
configuration selection, prior-run fact selection, import, and `audit_store` share one process-local
ingress-work limiter. A complete-prefix fold must acquire one non-cloneable work slot before its
first allocation/read and retain it through decode/reduction; exhaustion returns typed
`IngressWorkCapacityExceeded` immediately. Per-operation frame/byte bounds are checked separately.
There is no wait queue that can grow without bound, eviction, or hidden retry. Dropping or
completing the scope releases the slot.

When U3 is enabled, the one-shot attention limit is fixed in `StoreWorkLimits` at open and the
tenant facade cannot weaken or enlarge it per call. When U3 is absent, that field and its type are
deleted with the rest of the attention surface.

After async I/O has fetched bounded owned frames, framing/JCS decoding and a potentially maximum-
prefix reducer fold run on the platform's bounded blocking/CPU executor, never a Tokio worker.
Move the owned bytes, exact catalog, and ingress-work slot into that job; map panic to a typed
redacted integrity fault. Cancellation of the awaiting task does not release capacity until the
CPU job actually exits and drops its slot. Add an event-loop responsiveness regression at the
maximum supported prefix/configuration/audit page and a saturation test proving the blocking pool
and ingress limiter remain bounded.

Runtime-facing methods are intentionally narrow:

```rust
impl QualifiedHistoryPort {
    pub fn identity(&self) -> &StructuredStoreIdentity;
    pub fn catalog_fingerprint(&self) -> &ProgramCatalogFingerprint;
    pub fn shares_catalog_instance(&self, catalog: &ProgramCatalog) -> bool;

    pub async fn resume(
        &self,
        tenant: &TenantScopeId,
        run_id: &RunId,
    ) -> Result<ActiveQualifiedRun, ResumeError>;

    pub fn begin_admission<C, Input, Output, Failure>(
        &self,
        tenant: &TenantScopeId,
        program: TypedProgram<Input, Output, Failure>,
        root_input: QualifiedTypedValue<Input>,
        context: AdmissionContextCandidate,
        configuration: ConfigurationAdmissionEvidence<C>,
        fact_sources: FactSourceManifestCandidate,
    ) -> Result<TypedAdmissionInput<C, Input, Output, Failure>, AdmissionInputError>
    where
        C: MfmConfig,
        Input: StructuredValue,
        Output: StructuredValue,
        Failure: FailureValue;
}

impl<C, Input, Output, Failure> TypedAdmissionInput<C, Input, Output, Failure>
where
    C: MfmConfig,
    Input: StructuredValue,
    Output: StructuredValue,
    Failure: FailureValue,
{
    pub async fn prepare(
        self,
        append_id: AppendRequestId,
    ) -> StorePrepareDisposition<PlainEvent>;
}
```

`begin_admission` is the only mint. It checks `owns_program`, exact root/context contracts,
tenant-scoped configuration/fact fixations, all input bounds, and captures the same private Store
owner; its data fields have no public literal. `TypedAdmissionInput::prepare` constructs its event
internally and fetches only changing facts
required by `AppendContext`, and requires an absent predecessor. Every resumed/direct-successor
`ActiveQualifiedRun` retains the exact private Store coordinator; consuming selected-action methods fetch
their own required fact frontier and call the same reducer/binder without accepting a separate port
or predecessor. No cross-port action preparation API exists. `resume` performs one bounded complete-
prefix load and fold.

The Store-qualified disposition is not the raw backend enum:

```rust
pub enum StoreCommitDisposition<K: EventKind> {
    DirectlyCommitted(CommittedAppend<K>),
    ExistingSame(ExistingAttempt<K>),
    StaleHead,
    AcknowledgementUnknown(UnresolvedAppend<K>),
    Retryable {
        append: PreparedAppend<K>,
        error: RetryableStoreError,
    },
    FactFrontierChanged {
        preparation: RetryAppendPreparation<K>,
    },
    Failed(TerminalStoreError),
}

pub enum ObservationCommitDisposition {
    DirectlyCommitted(CommittedAppend<AccessObservationEvent>),
    ExistingSame(ExistingAttempt<AccessObservationEvent>),
    StaleHead(ObservationRebaseInput),
    AcknowledgementUnknown(UnresolvedObservationAppend),
    Retryable {
        append: PreparedObservationAppend,
        error: RetryableStoreError,
    },
    FactFrontierChanged {
        preparation: RetryObservationPreparation,
    },
    Failed(TerminalStoreError),
}

pub enum ReservationCommitDisposition<K, C>
where
    K: ReservationMode<C>,
{
    DirectlyCommitted(CommittedReservation<K, C>),
    ExistingSame(ExistingAttempt<AccessReservationEvent>),
    StaleHead,
    AcknowledgementUnknown(UnresolvedReservationAppend<K, C>),
    Retryable {
        append: PreparedReservationAppend<K, C>,
        error: RetryableStoreError,
    },
    FactFrontierChanged {
        preparation: RetryReservationPreparation<K, C>,
    },
    Failed(TerminalStoreError),
}

pub struct CommittedAppend<K: EventKind> {
    // The exact PreparedAppend's callback-free successor and fixation; private.
    _kind: PhantomData<fn() -> K>,
}

pub struct CommittedReservation<K, C>
where
    K: ReservationMode<C>,
{
    // Direct-new successor, assigned reservation ref/head, and exact mode/C resources; private.
    _mode: PhantomData<fn() -> (K, C)>,
}

pub struct ObservationWriteContinuation<K, C>
where
    K: ReservationMode<C>,
{
    // Owns the direct-new ActiveQualifiedRun and exact reservation/request/binding fixation.
    _mode: PhantomData<fn() -> (K, C)>,
}

pub struct ExistingAttempt<K: EventKind> {
    // Qualified equality evidence only; no successor promotion or invocation authority.
    _kind: PhantomData<fn() -> K>,
}

#[must_use]
pub struct UnresolvedAppend<K: EventKind> {
    // The same owner-bound affine preparation and append identity; private, non-Clone, non-Serde.
    _kind: PhantomData<fn() -> K>,
}

#[must_use]
pub struct UnresolvedObservationAppend {
    // Exact owner-bound append + rebase input after unknown acknowledgement.
}

#[must_use]
pub struct UnresolvedReservationAppend<K, C>
where
    K: ReservationMode<C>,
{
    // Exact owner-bound reservation append after unknown acknowledgement.
    _mode: PhantomData<fn() -> (K, C)>,
}

impl<K: EventKind> UnresolvedAppend<K> {
    pub async fn resolve(self) -> StoreCommitDisposition<K>;
}


impl UnresolvedObservationAppend {
    pub async fn resolve(self) -> ObservationCommitDisposition;
}

impl<K, C> UnresolvedReservationAppend<K, C>
where
    K: ReservationMode<C>,
{
    pub async fn resolve(self) -> ReservationCommitDisposition<K, C>;
}

impl<K, C> PreparedReservationAppend<K, C>
where
    K: ReservationMode<C>,
{
    pub fn request(&self) -> &K::Request;
    pub async fn commit(self) -> ReservationCommitDisposition<K, C>;
}

impl CommittedAppend<PlainEvent> {
    pub fn into_successor(self) -> ActiveQualifiedRun;
}

impl CommittedAppend<AccessObservationEvent> {
    pub fn into_successor(self) -> ActiveQualifiedRun;
}

impl<K, C> CommittedReservation<K, C>
where
    K: ReservationMode<C, Facts = NoPriorFacts>,
{
    pub fn into_no_facts(
        self,
    ) -> (RecordRef, ObservationWriteContinuation<K, C>);
}

impl<K, C> CommittedReservation<K, C>
where
    K: ReservationMode<C, Facts = PriorRunFacts>,
{
    pub fn into_prior_facts(
        self,
    ) -> (
        RecordRef,
        ObservationWriteContinuation<K, C>,
        PriorRunFactScanContinuation,
    );
}

impl<C: ReadCapabilityContract> ObservationWriteContinuation<ReadReservation, C> {
    #[doc(hidden)]
    pub async fn prepare_returned(
        self,
        value: QualifiedTypedValue<C::Returned>,
        append_id: AppendRequestId,
    ) -> ObservationPrepareDisposition;

    #[doc(hidden)]
    pub async fn prepare_safe_failure(
        self,
        value: QualifiedSafeFailure<C::SafeFailure>,
        append_id: AppendRequestId,
    ) -> ObservationPrepareDisposition;

    #[doc(hidden)]
    pub fn into_active(self) -> ActiveQualifiedRun;
}

impl<C: EffectCapabilityContract> ObservationWriteContinuation<EffectReservation, C> {
    #[doc(hidden)]
    pub async fn prepare_returned(
        self,
        value: QualifiedTypedValue<C::Returned>,
        append_id: AppendRequestId,
    ) -> ObservationPrepareDisposition;

    #[doc(hidden)]
    pub async fn prepare_safe_failure(
        self,
        value: QualifiedSafeFailure<C::SafeFailure>,
        append_id: AppendRequestId,
    ) -> ObservationPrepareDisposition;

    #[doc(hidden)]
    pub async fn prepare_entry_unknown(
        self,
        code: AccessFaultCode,
        append_id: AppendRequestId,
    ) -> ObservationPrepareDisposition;

    #[doc(hidden)]
    pub fn into_active(self) -> ActiveQualifiedRun;
}

pub struct PriorRunFactScanContinuation {
    // Exact reservation/store/tenant/source bounds; private, non-Clone, non-Serde.
}

impl PriorRunFactScanContinuation {
    pub async fn scan(
        self,
        request: FactSelectionRequest,
    ) -> FactScanStep;
}

pub enum FactScanStep {
    Selected(FactSelectionReadResponse),
    Retryable {
        continuation: PriorRunFactScanContinuation,
        error: RetryableFactScanError,
    },
    Failed(TerminalFactScanError),
}
```

`PreparedAppend::rebind` retains the already-resolved event and unbound successor needed to refresh
only a changing append context; it contains no callback or provider thunk. A fact-frontier change
therefore returns `FactFrontierChanged` with the exact owner and rebinds once without rerunning state logic or
decoding values. `StoreEpochChanged` and `DurabilityProfileLost` are typed terminal/reopen errors,
because the preparation belongs to a Store contract that no longer exists. `ProjectionMismatch`
is fail-closed integrity error. None is collapsed into `StaleHead` or a generic owner-dropping
backend error.

`PreparedAppend` captures the exact private `Arc<HistoryCoordinatorInner>` that created it, so
`commit(self)` cannot target another Store/epoch/backend. Its direct branch consumes that same
value into `CommittedAppend`; unknown consumes it into `UnresolvedAppend`. `CommittedAppend`
offers one consuming callback-free successor extraction used by Runtime's private coordinator. No
API accepts a unit `NewlyCommitted` plus a separately supplied successor, and there is no
`promote_uncommitted_successor` method. `ExistingAttempt` deliberately cannot produce a successor.

Each prepared append privately retains the callback-free predecessor evidence used by `apply`
(`None` only for genesis). On `Found(raw)`, Store first performs bounded framing/canonical ingress.
Exact candidate equality becomes `ExistingSame`. A well-formed unequal candidate is reduced and
bound against that retained predecessor once to classify `AppendConflict`; bad predecessor,
Program, value, projection, capacity, or surplus material is `InvalidHistory`/capacity failure.
This rare duplicate/conflict branch performs no prefix query and cannot promote either successor.

Reservation preparation and commit stay nominal over `C::Facts`; there is no wrong-variant resource
enum or `Option` scanner. The specialized direct-new extraction returns the assigned reservation
ref and one `ObservationWriteContinuation`; the prior-facts form additionally returns exactly one
`PriorRunFactScanContinuation`. Store mints both only while consuming that direct-new reservation
append. Runtime moves them into the matching `ReadyToInvoke` and nominal adapter call; Found, cold
ingress, or another event kind cannot obtain either. The scan continuation is one-use and fixes
Store, tenant, reservation, admitted source manifest, frontier, and bounds.

`ObservationWriteContinuation` owns the successor `ActiveQualifiedRun`, so Runtime never holds a
separate session that could be transposed beside it. It is moved through `ReadyToInvoke` and the
private request-bound `AcceptedAccessResponse`; only that response's Runtime-internal methods call
the doc-hidden Store SPI above with the already-qualified outcome. A historical selected
reservation exposes recovery only and cannot mint an observation. The SPI is public solely because
Runtime is a downstream crate; trusted composition does not export it, and the private Runtime call
token remains the framework's proof that provider ingress actually completed. This is an explicit
TCB seam, not a claim that Rust visibility authenticates arbitrary downstream crates.

An observation direct commit returns only its successor `ActiveQualifiedRun`. The sole selector on
that successor yields `SettleReadObservation` or `SettleEffectObservation`, exactly as a cold resume
after observation commit does. Runtime binds that selected action to `S,C`, borrows its already-
qualified exhaustive outcome, runs the matching settlement callback, and consumes the action into
the next plain transition. There is no direct-commit-only observation evidence path and no special
cold settlement reducer. `EntryUnknown` follows its declared Effect recovery/terminal path.
Framing, strict decode, protocol authentication, or request/response binding failure occurs before
an observation proposal exists and therefore cannot be smuggled into a fake persisted value or
fault observation.

`FactScanStep` preserves the one-use scan continuation on every retryable Store failure. Runtime's
private typed ready owner carries that continuation, the typed fact request, observation writer,
session shell, and still-unentered provider package together; the adapter call does not exist until
`Selected`. `Failed` is a reviewed terminal/fail-closed classification. No plain `Result` may drop
the scanner, and no adapter-facing type can own it.

Rust has no workspace-private visibility. These Store-to-Runtime methods are public only because
Runtime is a downstream crate. Their safety does not rely on a public empty marker trait: all
inputs/outputs have private constructors, the port is an opaque product of Store open, and only
Runtime owns an invocation continuation. Document them as the Runtime SPI and do not re-export them
from App or binaries.

`HistoryReader` uses the same private complete-prefix function:

```rust
impl HistoryReader {
    pub async fn qualify_run(
        &self,
        tenant: &TenantScopeId,
        run_id: &RunId,
    ) -> Result<QualifiedRun, HistoryReadError>;

    pub async fn read_public(... ) -> Result<PublicRunEvidence, HistoryReadError>;
    pub async fn read_trace(... ) -> Result<TraceRunEvidence, HistoryReadError>;
    pub async fn read_access_audit(... ) -> Result<AccessAuditEvidence, HistoryReadError>;
    pub async fn prepare_export(... ) -> Result<ExportRunEvidence, HistoryReadError>;
}

impl StoreAuditPort {
    pub async fn audit_store(
        &mut self,
        request: StoreAuditRequest,
    ) -> Result<StoreAuditReport, StoreAuditError>;
}
```

Every consuming async Store transition returns an exhaustive owner-carrying disposition, not a
generic `Result` whose error can drop affine state. `Retryable` returns the exact preparation;
acknowledgement uncertainty returns the exact unresolved package; only `Failed` is a reviewed
terminal classification that may destroy it. The same rule applies to preparation, observation
rebase, configuration writes, and App mapping. Inject a transient failure at each layer and assert
the identical owner can be retried without reconstructing bytes or invoking again.

DTO methods may call `qualify_run` internally and project immediately; they do not implement their
own verifier. Within one application request that needs several projections, qualify once and
borrow the same `QualifiedRun`. A run id not found inside the facade tenant maps to `RunNotFound`.
A row actually returned for that partition whose `RunAdmitted.tenant_scope_id` differs is
`InvalidHistory`, not hidden as absence.

`HistoryReader` has no whole-store audit method. Only the distinct non-cloneable `StoreAuditPort`
can begin the combined snapshot; its exclusive mutable borrow serializes operator audits without
making it run execution authority or risking an owner-dropping async result.

Ordinary `open`/`check_ready` qualifies schema, store identity/epoch, backend channel and
durability, writer role, exact catalog instance/fingerprint, and the ability to execute bounded
snapshot and exact-head transactions. It performs zero run/configuration enumeration.
`audit_store` is an explicit fixed-snapshot diagnostic with progress; its report creates no cached
evidence or Runtime authority.

### 4.4 Mechanical backend SPI

Concrete storage crates implement public object-safe mechanical subtraits owned by `mfm-store`,
then expose exactly one object implementing their composite. All async methods use the same future
alias and the same exhaustive operational error vocabulary:

```rust
pub type BackendFuture<'a, T> = Pin<
    Box<dyn Future<Output = Result<T, BackendOperationError>> + Send + 'a>,
>;

#[derive(Debug, thiserror::Error)]
pub enum BackendOperationError {
    #[error("backend operation is retryable")]
    Retryable(#[source] RetryableBackendError),
    #[error("store writer epoch changed")]
    StoreEpochChanged,
    #[error("primary-crash/restart durability is no longer qualified")]
    DurabilityProfileLost,
    #[error("tenant fact frontier changed")]
    FactFrontierChanged,
    #[error("stored mechanical projection does not match its canonical frame")]
    ProjectionMismatch,
    #[error("backend operation failed")]
    Fatal(#[source] FatalBackendError),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BackendRetryClass {
    ConnectionUnavailable,
    PoolCapacity,
    SerializationConflict,
    TransactionAborted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BackendFatalClass {
    SchemaContract,
    ConstraintContract,
    CorruptBackendState,
    BackendInvariant,
}

pub struct RetryableBackendError {
    class: BackendRetryClass,
    source: Box<dyn Error + Send + Sync + 'static>,
}

pub struct FatalBackendError {
    class: BackendFatalClass,
    source: Box<dyn Error + Send + Sync + 'static>,
}

impl RetryableBackendError {
    pub fn from_source<E>(class: BackendRetryClass, source: E) -> Self
    where
        E: Error + Send + Sync + 'static;

    pub fn class(&self) -> BackendRetryClass;
}

impl FatalBackendError {
    pub fn from_source<E>(class: BackendFatalClass, source: E) -> Self
    where
        E: Error + Send + Sync + 'static;

    pub fn class(&self) -> BackendFatalClass;
}

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

    // This method exists only in the enabled U3 design.
    fn load_effect_attention_inventory<'a>(
        &'a self,
        tenant: &'a TenantScopeId,
        limit: EffectAttentionInventoryLimit,
    ) -> BackendFuture<'a, BackendEffectAttentionOutcome>;
}

pub trait StructuredConfigurationBackend: Send + Sync + 'static {
    fn load_configuration_stream<'a>(
        &'a self,
        tenant: &'a TenantScopeId,
        key: &'a ConfigurationKey,
        selection: &'a ConfigurationSelection,
        limit: RawConfigurationLoadLimit,
    ) -> BackendFuture<'a, Option<RawConfigurationStream>>;

    fn compare_and_append_configuration<'a>(
        &'a self,
        command: &'a BackendConfigurationAppend,
    ) -> BackendFuture<'a, BackendConfigurationAppendOutcome>;
}

pub trait StructuredFactBackend: Send + Sync + 'static {
    fn load_tenant_fact_frontier<'a>(
        &'a self,
        tenant: &'a TenantScopeId,
    ) -> BackendFuture<'a, RawTenantFactFrontier>;

    fn load_fact_publication_page<'a>(
        &'a self,
        tenant: &'a TenantScopeId,
        after: Option<&'a TenantFactSequence>,
        limit: RawFactPageLimit,
    ) -> BackendFuture<'a, RawFactPublicationPage>;
}

pub trait StructuredStoreBackend:
    StructuredHistoryBackend
    + StructuredConfigurationBackend
    + StructuredFactBackend
    + StoreAuditBackend
    + Send
    + Sync
    + 'static
{
    fn identity(&self) -> &StructuredStoreIdentity;

    fn check_ready<'a>(
        &'a self,
        expected_identity: &'a StructuredStoreIdentity,
    ) -> BackendFuture<'a, ()>;
}

pub enum BackendAppendOutcome {
    NewlyCommitted,
    Found(StoredAttemptBytes),
    StaleHead,
    AcknowledgementUnknown,
}

pub enum BackendConfigurationAppendOutcome {
    NewlyCommitted,
    Found(StoredConfigurationAttemptBytes),
    StaleHead,
    AcknowledgementUnknown,
}

// This enum exists only in the enabled U3 design.
pub enum BackendEffectAttentionOutcome {
    Complete(RawEffectAttentionInventory),
    CapacityExceeded,
}
```

Implement `Debug`, `Display`, and `Error` for the two wrapper errors manually: public formatting
contains only the reviewed class, while `source()` preserves the concrete cause for internal error
chains. Constructors accept no caller-provided display string, and no database URL, SQL parameter,
frame bytes, or credential may cross a public/transport error mapping.

Do not add a blanket composite implementation that can stitch purpose-specific trait objects
together. The concrete Memory or PostgreSQL backend implements `StructuredStoreBackend` for its
single struct, so one `identity()` and `check_ready()` govern every history, configuration, fact,
attention (when enabled), and audit operation. `check_ready` performs no enumeration and returns no
portable proof; definite success is trusted only inside that invocation of
`StructuredStore::open`.

`BackendOperationError` is exhaustive; do not add a top-level `Other`/boxed-arbitrary-error variant
or an IO string escape hatch. The classed wrappers may retain their redacted source as shown. Store
maps `Retryable` to the relevant owner-carrying retry disposition,
`StoreEpochChanged`/`DurabilityProfileLost` to reopen-required terminal errors,
`FactFrontierChanged` to re-preparation with the retained owner, and `ProjectionMismatch`/`Fatal`
to reviewed fail-closed terminal errors. An append may return `AcknowledgementUnknown` only when
the backend sent `COMMIT` and cannot determine whether that exact transaction committed. Connection
loss before `COMMIT`, a definite rollback, serialization failure, pool exhaustion, or read failure
is `BackendOperationError::Retryable`; no non-append operation and no pre-commit failure can report
acknowledgement uncertainty.

The raw DTOs have private fields but public bounded mechanical constructors and borrowed getters,
because PostgreSQL, Memory, and downstream conformance backends must be implementable without a
Store feature or hidden seal. These constructors check only integer conversion, coordinate shape,
per-frame size, page cardinality, and cumulative byte bounds. They do **not** decode JCS, validate a
Program/value, reduce history, verify content refs, compare projections to frame contents, or mint
qualified evidence:

```rust
pub struct RawRunFrame {
    sequence: RunSequence,
    append_id: AppendRequestId,
    head_commit_digest: ContentDigest,
    committed_batch_bytes: Box<[u8]>,
}

impl RawRunFrame {
    pub fn try_from_parts(
        sequence: RunSequence,
        append_id: AppendRequestId,
        head_commit_digest: ContentDigest,
        committed_batch_bytes: impl Into<Box<[u8]>>,
        max_frame_bytes: NonZeroU64,
    ) -> Result<Self, RawBackendDtoError>;

    pub fn sequence(&self) -> RunSequence;
    pub fn append_id(&self) -> &AppendRequestId;
    pub fn head_commit_digest(&self) -> &ContentDigest;
    pub fn committed_batch_bytes(&self) -> &[u8];
}

pub struct RawRunPrefix {
    tenant: TenantScopeId,
    run_id: RunId,
    reported_head: JournalHead,
    frames: Box<[RawRunFrame]>,
}

impl RawRunPrefix {
    pub fn try_from_parts(
        tenant: TenantScopeId,
        run_id: RunId,
        reported_head: JournalHead,
        frames: Vec<RawRunFrame>,
        limit: RawHistoryLoadLimit,
    ) -> Result<Self, RawBackendDtoError>;

    pub fn tenant(&self) -> &TenantScopeId;
    pub fn run_id(&self) -> &RunId;
    pub fn reported_head(&self) -> &JournalHead;
    pub fn frames(&self) -> &[RawRunFrame];
}

pub struct StoredAttemptBytes {
    tenant: TenantScopeId,
    run_id: RunId,
    frame: RawRunFrame,
}

impl StoredAttemptBytes {
    pub fn try_from_parts(
        tenant: TenantScopeId,
        run_id: RunId,
        frame: RawRunFrame,
        limit: RawHistoryLoadLimit,
    ) -> Result<Self, RawBackendDtoError>;

    pub fn tenant(&self) -> &TenantScopeId;
    pub fn run_id(&self) -> &RunId;
    pub fn frame(&self) -> &RawRunFrame;
}

pub struct RawConfigurationRevision {
    sequence: ConfigurationSequence,
    append_id: ConfigurationAppendId,
    head: ConfigurationHead,
    canonical_revision_bytes: Box<[u8]>,
}

impl RawConfigurationRevision {
    pub fn try_from_parts(
        sequence: ConfigurationSequence,
        append_id: ConfigurationAppendId,
        head: ConfigurationHead,
        canonical_revision_bytes: impl Into<Box<[u8]>>,
        max_revision_bytes: NonZeroU64,
    ) -> Result<Self, RawBackendDtoError>;

    pub fn sequence(&self) -> ConfigurationSequence;
    pub fn append_id(&self) -> &ConfigurationAppendId;
    pub fn head(&self) -> &ConfigurationHead;
    pub fn canonical_revision_bytes(&self) -> &[u8];
}

pub struct RawConfigurationStream {
    tenant: TenantScopeId,
    key: ConfigurationKey,
    selected_head: ConfigurationHead,
    revisions: Box<[RawConfigurationRevision]>,
}

impl RawConfigurationStream {
    pub fn try_from_parts(
        tenant: TenantScopeId,
        key: ConfigurationKey,
        selected_head: ConfigurationHead,
        revisions: Vec<RawConfigurationRevision>,
        limit: RawConfigurationLoadLimit,
    ) -> Result<Self, RawBackendDtoError>;

    pub fn tenant(&self) -> &TenantScopeId;
    pub fn key(&self) -> &ConfigurationKey;
    pub fn selected_head(&self) -> &ConfigurationHead;
    pub fn revisions(&self) -> &[RawConfigurationRevision];
}

pub struct StoredConfigurationAttemptBytes {
    tenant: TenantScopeId,
    key: ConfigurationKey,
    revision: RawConfigurationRevision,
}

impl StoredConfigurationAttemptBytes {
    pub fn try_from_parts(
        tenant: TenantScopeId,
        key: ConfigurationKey,
        revision: RawConfigurationRevision,
        limit: RawConfigurationLoadLimit,
    ) -> Result<Self, RawBackendDtoError>;

    pub fn tenant(&self) -> &TenantScopeId;
    pub fn key(&self) -> &ConfigurationKey;
    pub fn revision(&self) -> &RawConfigurationRevision;
}

pub struct RawFactPublication {
    tenant: TenantScopeId,
    sequence: TenantFactSequence,
    head: TenantFactHead,
    canonical_publication_bytes: Box<[u8]>,
}

impl RawFactPublication {
    pub fn try_from_parts(
        tenant: TenantScopeId,
        sequence: TenantFactSequence,
        head: TenantFactHead,
        canonical_publication_bytes: impl Into<Box<[u8]>>,
        max_publication_bytes: NonZeroU64,
    ) -> Result<Self, RawBackendDtoError>;

    pub fn tenant(&self) -> &TenantScopeId;
    pub fn sequence(&self) -> TenantFactSequence;
    pub fn head(&self) -> &TenantFactHead;
    pub fn canonical_publication_bytes(&self) -> &[u8];
}

pub struct RawFactPublicationPage {
    reported_frontier: RawTenantFactFrontier,
    publications: Box<[RawFactPublication]>,
    has_more: bool,
}

impl RawFactPublicationPage {
    pub fn try_from_parts(
        reported_frontier: RawTenantFactFrontier,
        publications: Vec<RawFactPublication>,
        has_more: bool,
        limit: RawFactPageLimit,
    ) -> Result<Self, RawBackendDtoError>;

    pub fn reported_frontier(&self) -> &RawTenantFactFrontier;
    pub fn publications(&self) -> &[RawFactPublication];
    pub fn has_more(&self) -> bool;
}

pub struct RawTenantFactFrontier {
    tenant: TenantScopeId,
    sequence: TenantFactSequence,
    head: TenantFactHead,
}

impl RawTenantFactFrontier {
    pub fn try_from_parts(
        tenant: TenantScopeId,
        sequence: TenantFactSequence,
        head: TenantFactHead,
    ) -> Result<Self, RawBackendDtoError>;

    pub fn tenant(&self) -> &TenantScopeId;
    pub fn sequence(&self) -> TenantFactSequence;
    pub fn head(&self) -> &TenantFactHead;
}

// These three types exist only in the enabled U3 design.
pub struct EffectAttentionInventoryLimit {
    max_items: NonZeroU64,
    max_route_bytes: NonZeroU64,
}

impl EffectAttentionInventoryLimit {
    pub fn try_new(
        max_items: NonZeroU64,
        max_route_bytes: NonZeroU64,
    ) -> Result<Self, RawBackendDtoError>;
    pub fn max_items(&self) -> NonZeroU64;
    pub fn max_route_bytes(&self) -> NonZeroU64;
}

pub struct RawEffectAttentionEntry {
    tenant: TenantScopeId,
    run_id: RunId,
    head_sequence: RunSequence,
    head_digest: ContentDigest,
}

impl RawEffectAttentionEntry {
    pub fn try_from_parts(
        tenant: TenantScopeId,
        run_id: RunId,
        head_sequence: RunSequence,
        head_digest: ContentDigest,
    ) -> Result<Self, RawBackendDtoError>;

    pub fn tenant(&self) -> &TenantScopeId;
    pub fn run_id(&self) -> &RunId;
    pub fn head_sequence(&self) -> RunSequence;
    pub fn head_digest(&self) -> &ContentDigest;
}

pub struct RawEffectAttentionInventory {
    tenant: TenantScopeId,
    entries: Box<[RawEffectAttentionEntry]>,
}

impl RawEffectAttentionInventory {
    pub fn try_from_parts(
        tenant: TenantScopeId,
        entries: Vec<RawEffectAttentionEntry>,
        limit: EffectAttentionInventoryLimit,
    ) -> Result<Self, RawBackendDtoError>;

    pub fn tenant(&self) -> &TenantScopeId;
    pub fn entries(&self) -> &[RawEffectAttentionEntry];
}
```

`RawHistoryLoadLimit`, `RawConfigurationLoadLimit`, and `RawFactPageLimit` have
checked public constructors/getters and use `NonZeroU64`; downstream backends never guess Store's
bounds. `RawBackendDtoError` reports only the violated mechanical bound/coordinate class and never
includes frame bytes. The audit page and, when U3 is enabled, attention DTOs follow this identical
private-field/checked-constructor/getter rule.

In the enabled U3 design, `RawEffectAttentionInventory::try_from_parts` enforces total cardinality
and route-byte bounds; the limit constructor also rejects a cardinality for which the sentinel
`max_items + 1` cannot be represented by the backend query API. The backend method is one bounded
operation, never a page API: PostgreSQL
opens one read-only transaction/snapshot, scans the partial index in canonical `run_id` order with a
`max_items + 1` sentinel, tracks canonical route bytes with checked arithmetic, and returns a
payloadless `BackendEffectAttentionOutcome::CapacityExceeded` if either bound would be exceeded.
Store maps that disposition to its typed public capacity error. It never returns a truncated
`Complete` result.
Memory captures one immutable snapshot and applies the same algorithm. Store then qualifies each
chosen run on demand; the raw inventory itself is not run authority. In the absent U3 design,
delete the method, limit, DTOs, command getter/field, column, index, and tests together.

`BackendAppendCommand` is opaque semantically but exposes the bounded mechanical getters a SQL
implementation needs: store/epoch, tenant/run, expected predecessor or genesis absence, append id,
canonical batch frame, exact projection/index deltas, fact-frontier precondition/delta, and the
conditional attention Boolean. The backend cannot alter or reconstruct them. `StoredAttemptBytes`
is a bounded raw canonical batch/envelope plus the minimum coordinates required for Store ingress.

```rust
impl BackendAppendCommand {
    pub fn store_identity(&self) -> &StructuredStoreIdentity;
    pub fn tenant(&self) -> &TenantScopeId;
    pub fn run_id(&self) -> &RunId;
    pub fn expected_head(&self) -> ExpectedRunHead<'_>;
    pub fn append_id(&self) -> &AppendRequestId;
    pub fn run_sequence(&self) -> RunSequence;
    pub fn head_commit_digest(&self) -> &ContentDigest;
    pub fn committed_batch_bytes(&self) -> &[u8];
    pub fn projection_delta(&self) -> &BackendProjectionDelta;
    pub fn fact_frontier_precondition(&self) -> Option<&TenantFactHead>;
    pub fn fact_publication(&self) -> Option<&BackendFactPublicationDelta>;
    // Present only in the enabled U3 design; absent means the method and field do not exist.
    pub fn needs_effect_attention(&self) -> bool;
}

impl BackendConfigurationAppend {
    pub fn store_identity(&self) -> &StructuredStoreIdentity;
    pub fn tenant(&self) -> &TenantScopeId;
    pub fn key(&self) -> &ConfigurationKey;
    pub fn expected_head(&self) -> ExpectedConfigurationHead<'_>;
    pub fn append_id(&self) -> &ConfigurationAppendId;
    pub fn sequence(&self) -> ConfigurationSequence;
    pub fn head(&self) -> &ConfigurationHead;
    pub fn canonical_revision_bytes(&self) -> &[u8];
}
```

The command constructors remain Store-private. All returned views borrow the exact non-Clone
command; no getter returns an owned frame, delta, or identity that could be edited and resubmitted.

The backend borrows the exact non-Clone command so Store still owns the command/successor after an
unknown acknowledgement and can quarantine or resubmit the identical package without cloning or
reconstruction. Under one tenant/run/epoch transaction lock, `compare_and_append` first proves the
exact `(StoreScopeId, StoreEpoch, TenantScopeId, RunId)` route, then checks append identity within
that same route and returns `Found(raw)` when retained; only when absent does it compare the exact
head and append. It must never query `RunId` or `(RunId, AppendRequestId)` without tenant and Store
identity predicates. A same-spelled run/append tuple under another tenant is therefore
indistinguishable from absence and can never produce `Found`, bytes, head, or a different error.
There is no separate normal-path `lookup_attempt`: it would add a time-of-check/time-of-use seam.
Unknown resolution simply re-borrows the quarantined command; the atomic operation returns Found if
the first attempt committed in that exact route, direct New if it was absent and the head is
unchanged, or Stale if it was absent and the head advanced.

`load_complete_prefix` applies the same Store-scope/epoch/tenant/run predicate to both head and
batch reads. An exact-partition miss returns `None` without probing another tenant; if rows returned
inside that predicate disagree with their raw route fields, Store fails closed as invalid history.
The backend must not perform a preliminary run-id-only existence check because even changing
`None` into a distinct integrity/not-found classification would leak cross-tenant existence.

The raw enum is an SPI disposition, never semantic proof. It is consumed lexically inside the
owner-bound `PreparedAppend::commit`: direct new promotes the owned prepared successor; found bytes
cross strict retained-attempt ingress exactly once; stale destroys the candidate; unknown
quarantines it. Backend implementations and conformance tests necessarily construct/observe the
raw value, but no API accepts it as promotion evidence. Runtime never receives a `CommittedBatch`
echo or a free raw `NewlyCommitted` value.

PostgreSQL's transaction owns only:

- exact predecessor equality (or genesis absence);
- append id uniqueness and raw found-attempt retrieval;
- atomic immutable batch insertion plus head/index/fact-frontier/conditional-attention update;
- Store epoch and affected-row checks;
- database parameter/frame limits; and
- acknowledgement ambiguity.

It does not re-run program ingress, reducer semantics, retained-object closure, projection equality,
or state/fact/access-field coherence already sealed by `PreparedAppend`. Database constraints may
still reject mechanically impossible rows.

Run append transaction order is normative:

1. begin the writable transaction under the managed role/search path;
2. assert transaction-effective durability;
3. lock Store identity and compare scope/Store epoch, retaining the lock through commit;
4. acquire the canonical advisory lock derived from Store scope, tenant, and run, then select the
   exact tenant-scoped `run_history_heads` route `FOR UPDATE` (or prove exact-partition genesis
   absence); a batch without its route/head is `ProjectionMismatch`;
5. query `(store_scope_id, store_epoch, tenant_scope_id, run_id, append_request_id)` and return raw
   `Found` before predecessor/head comparison when present;
6. when no attempt exists, compare the command's exact head to the already-locked route head (or
   its exact-partition genesis absence);
7. check only mechanical tenant/projection consistency;
8. lock/check the dense fact frontier when the append requires it;
9. insert the canonical batch frame;
10. advance the exact run head and conditional attention projection;
11. insert fact publication and advance the dense fact head when required; and
12. commit, mapping a definite success to payloadless New and an uncertain acknowledgement to
    Unknown.

Delete the current append-time retained-history capacity scan. `PreparedAppend` already carries
cumulative capacity evidence. Cold prefix loading still preflights count and total byte bounds
before fetching frames. Configuration append uses the analogous append-id/head transaction and
never loads the complete configuration history: lock/prove the exact
`(store scope, writer epoch, tenant, configuration key)` route first, query the append id only
inside that route, then compare the already-locked exact head. Fact frontier and audit queries are
likewise tenant-scoped unless the explicit non-tenant operator snapshot owns them. No backend
method may infer a tenant from a run id, configuration key, append id, or decoded frame.

The fresh PostgreSQL baseline stores one bounded canonical complete-batch `BYTEA` frame per
immutable append row plus only the mechanical columns needed for exact-head, append-id, tenant fact,
and conditional attention routing. The repository audit found no independent production query
owner for normalized batch-object rows. Delete `StoredBatchEnvelope`, `StoredObjectRow`,
`run_history_batch_objects`, object-row SQL catalog/ACL entries, loaders/groupers/reconstruction, and
their dedicated tests. `StoredAttemptBytes` is the exact frame originally committed; object lookup
within qualified evidence uses the in-memory index built once at prefix ingress.

The exact fresh baseline contract identifier is
`mfm.structured-run-history-postgres.v8`. Its run-history shape is:

```sql
CREATE TABLE run_history_batches (
    store_scope_id          TEXT NOT NULL,
    store_epoch             NUMERIC(20, 0) NOT NULL,
    tenant_scope_id         TEXT NOT NULL,
    run_id                  TEXT NOT NULL,
    run_sequence            NUMERIC(20, 0) NOT NULL,
    append_request_id       TEXT NOT NULL,
    head_commit_digest      TEXT NOT NULL,
    committed_batch_bytes   BYTEA NOT NULL,
    PRIMARY KEY (
        store_scope_id, store_epoch, tenant_scope_id, run_id, run_sequence
    ),
    UNIQUE (
        store_scope_id, store_epoch, tenant_scope_id, run_id, append_request_id
    ),
    UNIQUE (
        store_scope_id, store_epoch, tenant_scope_id, run_id,
        run_sequence, head_commit_digest
    )
);

CREATE TABLE run_history_heads (
    store_scope_id          TEXT NOT NULL,
    store_epoch             NUMERIC(20, 0) NOT NULL,
    tenant_scope_id         TEXT NOT NULL,
    run_id                  TEXT NOT NULL,
    head_sequence           NUMERIC(20, 0) NOT NULL,
    head_commit_digest      TEXT NOT NULL,
    -- Include this column only when U3 is enabled.
    needs_effect_attention  BOOLEAN NOT NULL,
    PRIMARY KEY (store_scope_id, store_epoch, tenant_scope_id, run_id),
    FOREIGN KEY (
        store_scope_id, store_epoch, tenant_scope_id, run_id,
        head_sequence, head_commit_digest
    ) REFERENCES run_history_batches (
        store_scope_id, store_epoch, tenant_scope_id, run_id,
        run_sequence, head_commit_digest
    ) DEFERRABLE INITIALLY DEFERRED
);

-- Include this index only when U3 is enabled.
CREATE INDEX run_history_heads_effect_attention
    ON run_history_heads (
        store_scope_id, store_epoch, tenant_scope_id, run_id
    )
    WHERE needs_effect_attention;
```

If U3 is absent, omit the Boolean and partial index rather than making the column nullable or
hard-coding `FALSE`. The head row is the exact route proof and mutable mechanical projection; the
batch rows remain immutable. PostgreSQL queries use every leading identity/tenant column shown
above. Store scope/epoch remain writer-lineage coordinates, not a replacement for tenant
partitioning. The selected head foreign key is deferred only so one transaction can insert the
immutable batch and advance/insert its head; it is checked before commit.

Resolve U3 before freezing the v8 catalog hash: the comments in the illustrative DDL mark the one
line/index deletion point, not two accepted v8 layouts, a feature flag, or runtime negotiation.
Exactly the chosen relation shape is the sole v8 schema contract.

Keep a duplicated column only when a concrete mechanical query/constraint owns it, and compare it
to decoded frame content once at ingress. Remove candidate/predecessor/object-count columns whose
only consumer reconstructs or rechecks the frame. `configuration_revisions` analogously keeps its
route/sequence/append/content-ref columns plus canonical revision `BYTEA`; remove duplicated
predecessor columns/self-FK because predecessor is in the unchanged strict revision wire and
current-head comparison owns append ordering.

Delete the `target_authority` table, migration data, catalog entries, ACLs, schema validator, and
query code. Store target key and role names are derived from the qualified schema/catalog, not a
live authority row. `TargetBinding` retains only database/schema/OID, `StoreScopeId`, `StoreEpoch`,
derived roles, channel facts, and `QualifiedPostgresDurability`. Delete `PhysicalTargetIdentity`,
`fence_generation`, `release_epoch`, `current_incarnation_ref`, `validate_target`, target hashing,
and exported target-currentness fixations.

Rewrite the destructive pre-production `migrations/0001_store.sql`, bump the schema contract and
catalog hashes, and require a fresh schema/store scope/epoch. Qualification accepts exactly
`mfm.structured-run-history-postgres.v8` and rejects
`mfm.structured-run-history-postgres.v7`. Do not add an ALTER migration, a v7 reader, a fallback
decoder, `StoredBatchEnvelope`, `StoredObjectRow`, `run_history_batch_objects`, or either legacy
object-row insert/load query.

### 4.5 PostgreSQL qualification and restore contract

PostgreSQL backend construction and `check_ready` must produce and freeze a
`QualifiedPostgresDurability` inside that one backend object before `StructuredStore::open` may
return its ports:

```rust
pub struct QualifiedPostgresDurability {
    store_scope_id: StoreScopeId,
    store_epoch: StoreEpoch,
}
```

The type itself means exactly the `PrimaryCrashRestart` contract; do not introduce a public one-
variant `DurabilityProfile` enum. `StoreEpoch` is the admitted writer epoch shared by all workers
of this composed Store. Qualification verifies
permanent/logged tables, `fsync = on`, `full_page_writes = on`, and transaction-effective
`synchronous_commit != off`, plus the selected channel/role/search path/schema identity and
exclusive writer epoch. Do not claim replica/quorum survival. A direct `NewlyCommitted` is defined
to survive primary crash/restart before Runtime may invoke. If another profile is later required,
add a separately tested profile rather than weakening this one.

These checks follow PostgreSQL's documented local-durability semantics: `fsync` and
`full_page_writes` protect crash recovery, and every non-`off` `synchronous_commit` mode waits for
local WAL flush. Keep the implementation and qualification tests aligned with the official
[write-ahead-log settings](https://www.postgresql.org/docs/current/runtime-config-wal.html), not a
hard-coded assumption about a particular deployment's default values.

Within one store epoch, PostgreSQL is trusted to preserve admitted immutable rows, report the
current head truthfully, and return truthful append outcomes. Legitimate restore/rollback rotates
`StoreScopeId` or `StoreEpoch`; reusing an epoch after restoration is a deployment-contract
violation. No post-commit SELECT can strengthen a lying or non-durable backend, so none remains.

`StoreEpoch` remains only the admitted writer epoch: it fences all writers opened as one composed
Store and is rechecked inside every write transaction. It is not an authorization epoch, cache
generation, release/currentness token, history schema version, or per-worker lease. All workers for
one live Store must use the same value. A legitimate restore deliberately installs a fresh writer
epoch (and a fresh scope when the restore changes the logical Store); `StructuredStore::open`
rejects a mismatch before any port is exposed.

Each append transaction rechecks the store scope/epoch and effective durability. Return typed
`StoreEpochChanged`, `DurabilityProfileLost`, and `FactFrontierChanged`; none is
`StaleHead`. Preserve PostgreSQL login roles/grants, DB credentials, TLS or protected-local-channel
requirements, affected-row checks, fact-frontier compare/update, and adapter-owned SQL locks or
idempotency tables. Delete generic physical-target generation/release/current-incarnation checks
whose sole owner was hot capability replacement.

`audit_store` requires one real snapshot across run histories, configuration histories, and the
dense fact chain:

```rust
pub trait StoreAuditSnapshot: Send {
    fn next_run_page(
        &mut self,
        limit: PageLimit,
    ) -> BackendFuture<'_, AuditPage<RawRunPrefix>>;

    fn next_configuration_page(
        &mut self,
        limit: PageLimit,
    ) -> BackendFuture<'_, AuditPage<RawConfigurationStream>>;

    fn next_fact_page(
        &mut self,
        limit: PageLimit,
    ) -> BackendFuture<'_, AuditPage<RawFactPublication>>;
}

pub trait StoreAuditBackend: Send + Sync {
    fn begin_audit_snapshot(
        &self,
    ) -> BackendFuture<'_, Box<dyn StoreAuditSnapshot>>;
}
```

`AuditPage<T>` also has private fields and this public mechanical API:

```rust
impl<T> AuditPage<T> {
    pub fn try_from_parts(
        items: Vec<T>,
        complete: bool,
        limit: PageLimit,
    ) -> Result<Self, RawBackendDtoError>;

    pub fn items(&self) -> &[T];
    pub fn is_complete(&self) -> bool;
}
```

The constructor checks page cardinality and rejects an empty incomplete page; Store qualifies every
raw item. `PageLimit` has a checked public `NonZeroU64` constructor/getter. The snapshot object, not
the caller or DTO, privately owns each source's monotonic keyset cursor. This is the same
implementable mechanical SPI contract as the history/configuration/fact DTOs, not a hidden
friend-crate API.

This is the object-safe mechanical adapter installed while opening a concrete backend. It is public
and implementable but confers no authority until trusted Store composition selects/qualifies the
backend and returns an opaque `StoreAuditPort`. `StoreAuditSnapshot` is non-`Clone`, owns one backend
snapshot/transaction for its entire lifetime, and advances bounded keyset cursors. PostgreSQL
implements it with one qualification-role repeatable-read transaction. Memory captures one
immutable snapshot. Do not reopen a transaction per page/source or compose independent purpose
readers that silently observe different snapshots.

PostgreSQL `check_ready` validates the exact v8 catalog, all v8 relation/column/index/constraint
identities, roles/search path/channel, Store scope/writer epoch, and
`QualifiedPostgresDurability` before returning. It explicitly rejects the v7 catalog, any
`run_history_batch_objects`/object-row query expectation, a missing or malformed
`run_history_heads` foreign key, and (according to the U3 ruling) either a missing exact partial
index or any surviving attention column/index. It executes catalog/settings checks only; run,
configuration, fact, and attention reads remain zero at open.

## 5. Runtime session, commit, and external-access APIs

### 5.1 Public affine execution lifecycle

`mfm-runtime` exposes one session-oriented core API:

```rust
#[derive(Clone, Debug)]
pub struct RuntimeLimits {
    pub max_active_sessions: NonZeroUsize,
    pub max_concurrent_cpu_jobs: NonZeroUsize,
    pub max_concurrent_planning_jobs: NonZeroUsize,
}

#[derive(Clone)]
pub struct Runtime(Arc<RuntimeInner>);

type RuntimeFuture<T> = Pin<Box<dyn Future<Output = T> + Send + 'static>>;

trait ErasedAdmissionInput: Send {
    fn prepare(
        self: Box<Self>,
        continuation: AdmissionContinuation,
        append_id: AppendRequestId,
    ) -> RuntimeFuture<PreparationDecision>;
}

pub struct AdmissionInput {
    runtime: Arc<RuntimeInner>,
    run_id: RunId,
    inner: Box<dyn ErasedAdmissionInput>,
    // Private, non-Clone, non-Serde; exact typed Store input remains behind the erased owner.
}

struct AdmissionContinuation {
    runtime: Arc<RuntimeInner>,
    dispatch: Arc<ResolvedProgramBindings>,
    active_slot: ActiveSessionSlot,
    run_id: RunId,
    // Affine genesis shell; no active Store run exists until direct commit.
}

pub struct RunSession {
    runtime: Arc<RuntimeInner>,
    qualified: ActiveQualifiedRun,
    dispatch: Arc<ResolvedProgramBindings>,
    active_slot: ActiveSessionSlot,
    // No Clone, Serialize, Deserialize, Default, or public constructor.
}

struct SessionContinuation {
    runtime: Arc<RuntimeInner>,
    dispatch: Arc<ResolvedProgramBindings>,
    active_slot: ActiveSessionSlot,
    run_id: RunId,
    // No QualifiedRun; private, non-Clone, non-Serde.
}

pub struct SuspendedRun {
    runtime: Arc<RuntimeInner>,
    run_id: RunId,
    inner: SuspendedRunInner,
    not_sync: PhantomData<Cell<()>>,
    // No Clone, Serialize, Deserialize, or public constructor.
}

enum SuspendedRunInner {
    AmbiguousCommit(UnresolvedPreparedDrive),
    CommitRetry(PreparedDrive),
    PreparationRetry(UnresolvedRuntimePreparation),
    FactScanRetry(ReadyToInvoke),
    ObservationRebase(PendingObservationRecovery),
}

enum UnresolvedRuntimePreparation {
    Plain {
        base: DriveBase,
        preparation: RetryAppendPreparation<PlainEvent>,
    },
    Access(Box<dyn ErasedRetryAccessPreparation>),
    Observation {
        continuation: SessionContinuation,
        preparation: RetryObservationPreparation,
    },
}

struct PendingObservationRecovery {
    continuation: SessionContinuation,
    rebase: ObservationRebaseInput,
}

pub enum RuntimeStep {
    Advanced {
        session: RunSession,
        outcome: StepOutcome,
    },
    Waiting {
        session: RunSession,
        reason: WaitReason,
    },
    Closed {
        run_id: RunId,
        outcome: PublicTerminalOutcome,
    },
    ResumeRequired {
        run_id: RunId,
        cause: ResumeCause,
    },
    Suspended {
        run: SuspendedRun,
        cause: SuspensionCause,
    },
    Retryable {
        session: RunSession,
        error: RetryableRuntimeError,
    },
    Failed {
        run_id: RunId,
        error: TerminalRuntimeError,
    },
}

pub enum SpawnStep {
    Started {
        session: RunSession,
        outcome: StepOutcome,
    },
    Existing {
        run_id: RunId,
    },
    Suspended {
        run: SuspendedRun,
        cause: SuspensionCause,
    },
    Retryable {
        admission: AdmissionInput,
        error: RetryableRuntimeError,
    },
    Rejected {
        error: SpawnError,
    },
}

impl Runtime {
    pub fn assemble(
        assembly: RuntimeAssembly,
        history: QualifiedHistoryPort,
        limits: RuntimeLimits,
    ) -> Result<Self, RuntimeAssemblyError>;

    pub fn prepare_admission<C, Input, Output, Failure>(
        &self,
        tenant: &TenantScopeId,
        program: TypedProgram<Input, Output, Failure>,
        root_input: QualifiedTypedValue<Input>,
        context: AdmissionContextCandidate,
        configuration: ConfigurationAdmissionEvidence<C>,
        fact_sources: FactSourceManifestCandidate,
    ) -> Result<AdmissionInput, AdmissionInputError>
    where
        C: MfmConfig,
        Input: StructuredValue,
        Output: StructuredValue,
        Failure: FailureValue;

    #[doc(hidden)]
    pub fn admission_material_scope<Input, Output, Failure>(
        &self,
        tenant: &TenantScopeId,
        entry: &TypedEntryProfile<Input, Output, Failure>,
    ) -> Result<AdmissionMaterialScope<Input, Output, Failure>, AdmissionMaterialError>
    where
        Input: StructuredValue,
        Output: StructuredValue,
        Failure: FailureValue;

    pub async fn resume(
        &self,
        tenant: &TenantScopeId,
        run_id: &RunId,
    ) -> Result<RunSession, RuntimeError>;

}

impl AdmissionInput {
    pub fn run_id(&self) -> &RunId;
    pub async fn spawn(self) -> SpawnStep;
}

impl RunSession {
    pub fn run_id(&self) -> &RunId;
    pub fn journal_head(&self) -> &JournalHead;
    pub fn status(&self) -> SessionStatus;
    pub async fn drive(self) -> RuntimeStep;
}

impl SuspendedRun {
    pub fn run_id(&self) -> &RunId;
    pub async fn resolve(self) -> RuntimeStep;
}
```

`RunSession` is deliberately `Send + Sync`: shared immutable inspection is harmless and every
state-changing operation consumes the value. `SuspendedRun` is `Send` but explicitly `!Sync` via
its private `Cell` marker because it owns unresolved `FnOnce` continuations. Compile assertions
freeze both choices so backend field changes cannot alter them accidentally. `RuntimeFuture<T>` is
the private object-erasure ABI, so every erased owner is `Send + 'static`. Add compile assertions
that `AdmissionInput::spawn`, `RunSession::drive`, `SuspendedRun::resolve`,
`ReadyToInvoke::invoke_once`, observation preparation, and every erased implementation produce
`Send` futures; do not silently relax the alias to a local future.

`RuntimeInner` owns one bounded `RuntimeCpuExecutor` gated by
`RuntimeLimits::max_concurrent_cpu_jobs`. Pure execution, Read/Effect request construction,
returned/safe-failure settlement, prior-fact request construction, capability response binding, and
typed canonical qualification never run on Tokio workers. Runtime moves the owning typed action or
per-call package into a blocking job, borrows its input/observation only inside that job, catches
panic to a redacted typed fault, and returns the same affine owner beside the typed result. Awaiting
tasks may wait only behind the already-bounded active-session population; no independent unbounded
job queue is created. Cancellation does not release the CPU permit until the running job exits and
drops its owner. Add maximum-value/graph responsiveness, saturation, panic, and cancellation tests.

The composed process also retains a bounded planning CPU executor gated by
`RuntimeLimits::max_concurrent_planning_jobs`. Demand-time App configuration selection, planner
execution, operation expansion, configuration/capability injection, and `Program` finish move the
unchanged `AdmitRunRequest`, selected `ResolvedConfiguration`, and planner into one owned job; none
runs on a Tokio worker. A callback panic is caught and redacted at the App boundary. Capacity
failure returns one opaque `PendingAdmissionInner::Planning` owner containing the unchanged request,
fixed selector result, exact selected `ResolvedConfiguration` once loaded, admission-material
scope, and erased typed planner. Transport code can retry without reparsing, reselecting or
reloading configuration, or partially rebuilding a Program. The executor has no unbounded queue,
and cancellation retains its permit until the job exits and drops or returns the affine owner. Add
maximum-profile/graph responsiveness, saturation, panic, cancellation, and owner-preservation
tests.

`spawn` and `resume` acquire one non-cloneable `ActiveSessionSlot` before returning or preparing an
active owner. `RunSession`, ambiguous admission/access/observation packages, and stale-observation
recovery carry that same slot through every move. Close, definite fail-closed termination, or drop
releases it. Exhaustion returns typed `ActiveSessionCapacityExceeded`; it never evicts, serializes,
or silently drops another session. The slot is process-local resource accounting, not a run lock,
authorization lease, or cross-worker ownership claim.

`Runtime::prepare_admission` is the one final mint from typed application planning to a spawnable
Store product. It verifies the exact Runtime/catalog brand, then immediately calls its retained
non-cloneable
`QualifiedHistoryPort::begin_admission<C,Input,Output,Failure>` and erases that already-associated
Store product behind `AdmissionInput`; App never needs the consumed port. The input owns its Runtime
`Arc`, derived `RunId`, and an erased Store product that already fixes tenant,
`TypedProgram<Input,Output,Failure>`, matching
`QualifiedTypedValue<Input>`, selected `ConfigurationAdmissionEvidence<C>`, qualified context, and
fact source manifest. It has no credential/principal fields and cannot be submitted to another
Runtime. `AdmissionInput::spawn` is owner-bound. If a direct admission commits, it returns
`SpawnStep::Started` with the genesis session. Existing-same returns `Existing`; a terminal
preparation returns `Rejected`; an indeterminate commit retains its exact owner in `Suspended`.
Active-session or other pre-destruction capacity failure returns `Retryable { admission, .. }`
without reparsing, reloading configuration, or replanning. None promotes the local successor on a
non-new branch.

`Runtime::admission_material_scope` is the narrow checked cross-crate bridge needed before that
final mint. It forwards to the retained port's Store-owned constructor and returns only a
callback-free, non-Clone scope branded to the exact tenant, catalog, Store, entry profile, and
bounds. App can ask that scope to construct context/fact candidates but cannot inspect its brand,
construct either candidate's fields, prepare an append, or recover the retained history port. This
is a mechanically public `#[doc(hidden)]` SPI because Rust has no workspace-private friendship; it
confers no live execution or write authority.

`Runtime::assemble` consumes both halves into one immutable `Arc<RuntimeInner>` and requires the
exact shared in-process Program catalog instance in addition to its persisted fingerprint, Store
identity/epoch contract, and assembly identity. There is no constructor accepting an independently
implemented history trait or a raw ProcessRegistry. A callback-free `QualifiedRun` becomes a
`RunSession` only inside `spawn`, `resume`, or direct advancement and only after matching the exact
assembly brand. Purpose readers never receive that brand. Sessions and suspensions retain the same
private Runtime `Arc` and put their consuming `drive`/`resolve` methods on themselves; passing an
owner to a foreign Runtime is therefore not an API operation.

`RunSession::drive` consumes its session. Every branch either returns its exact advanced/unchanged owner,
consumes it into `SuspendedRun`, or discards it and requires explicit resume. It never leaves an
invalid public shell. `SuspendedRunInner::AmbiguousCommit` owns an exact unknown append package;
`ObservationRebase` owns an already-ingressed response after a definite stale observation append
and performs no provider call. `FactScanRetry` owns the complete pre-provider `ReadyToInvoke` after
a retryable Store fact scan; resolving it performs one more scan unit and can invoke the provider
only if that unit returns `Selected`. `SuspendedRun::resolve` performs one bounded
lookup/resubmit/rebase/scan unit; repeated ambiguity, fact-read failure, or head contention returns
another `SuspendedRun`.
There is no hidden unbounded retry loop.

At action selection Runtime destructures a session once into `SessionContinuation` plus its sole
`ActiveQualifiedRun`; the run moves into Store's selected action/preparation, while the shell retains only
Runtime/dispatch/slot identity. A direct `CommittedAppend` supplies the sole successor run and
rebuilds the session with that shell. Unknown/retry suspension retains shell plus Store's affine
preparation. Found/stale/terminal branches drop the shell or require resume as specified. Runtime
never clones a predecessor active run merely to keep a session-shaped wrapper alive.

The ownership protocols are exhaustive enums, not `#[non_exhaustive]`: adding a branch must force
App to decide where the affine owner goes. A retryable drive error returns `Retryable` with the same
session. A retryable resolution error returns `Suspended` with the same logical suspension. Only a
reviewed terminal/fail-closed classification may return `Failed` without an owner. Async task
cancellation can still destroy an in-memory package; documentation treats that as conservative
loss, never as proof that an external Effect did not enter.

Delete Runtime core `drive_once(run_id)`, `load_verified` before every action, optional-previous
proof paths, cloneable verified-run facades, and automatic stale-head reload/continue. Do not add a
global session map. A transport may explicitly implement a one-shot `resume -> drive` composition
and accept the measured cold cost.

### 5.2 Private prepared-drive pairing

The sensitive Runtime values and constructors are crate-private:

```rust
enum PreparedDrive {
    Plain {
        base: DriveBase,
        append: PreparedAppend<PlainEvent>,
    },
    Access(Box<dyn ErasedPreparedAccessDrive>),
    Observation {
        continuation: SessionContinuation,
        append: PreparedObservationAppend,
    },
}

enum DriveBase {
    Admission(AdmissionContinuation),
    Session(SessionContinuation),
}

enum CommittedDrive {
    Plain {
        session: RunSession,
    },
    Access {
        ready: ReadyToInvoke,
    },
    Observation {
        session: RunSession,
    },
}

enum UnresolvedPreparedDrive {
    Plain {
        base: DriveBase,
        append: UnresolvedAppend<PlainEvent>,
    },
    Access(Box<dyn ErasedUnresolvedAccessDrive>),
    Observation {
        continuation: SessionContinuation,
        append: UnresolvedObservationAppend,
    },
}

struct RuntimeCommitCoordinator;

trait ErasedPreparedAccessDrive: Send {
    fn commit(self: Box<Self>) -> RuntimeFuture<CommitDecision>;
}

trait ErasedUnresolvedAccessDrive: Send {
    fn resolve(self: Box<Self>) -> RuntimeFuture<CommitDecision>;
}

trait ErasedRetryAccessPreparation: Send {
    fn retry(self: Box<Self>) -> RuntimeFuture<PreparationDecision>;
}

impl RuntimeCommitCoordinator {
    async fn commit(
        &self,
        prepared: PreparedDrive,
    ) -> CommitDecision;
}
```

`CommitDecision` is an exhaustive private ownership enum mirroring every direct, found, stale,
unknown, retryable, and terminal Store result. A Store commit retry moves the entire reconstructed
`PreparedDrive` into `SuspendedRunInner::CommitRetry`; it cannot reconstruct a `RunSession` while
the exact prepared append still owns the predecessor. Preparation, observation rebase, and unknown
resolution likewise return their exact `SuspendedRun`. `RuntimeStep::Retryable { session, .. }` is
reserved for an error detected before Runtime destructures the session or selects an action. Only
a typed terminal arm contains no owner. The consuming coordinator has no generic `Err` escape hatch
that could drop a session, invocation thunk, or qualified response event.

Only private constructors create each `PreparedDrive` variant. They consume the predecessor
session, Store-prepared append, and the exact continuation derived from the same action. They
perform no posterior shadow-proof check: one Runtime action function creates the live continuation, resolves the
Store event, prepares it, and immediately moves both into the private variant before returning.
No separately callable constructor accepts arbitrary pieces.

The coordinator destructures exactly one variant, calls the append's owner-bound consuming
`commit`, and retains the matching continuation in that stack frame. On
`DirectlyCommitted`, the plain and observation branches consume the callback-free
`CommittedAppend`, rebuild a session from its exact successor, and construct the matching
`CommittedDrive`. The reservation branch instead consumes `CommittedReservation` into the one
`ReadyToInvoke`; its Store observation writer retains the sole active successor until observation
preparation or conservative invocation-fault recovery, so no simultaneous `RunSession` exists. A
genesis direct commit combines the successor with its `AdmissionContinuation`; that concrete affine
shell replaces the undefined notion of an admission “brand.” `ExistingSame`, stale-head, and
terminal access branches destroy the inaccessible invocation thunk and promote nothing. On unknown
it moves Store's `UnresolvedAppend`, the same session brand, and the same continuation into private
`UnresolvedPreparedDrive`, surfaced only as
`SuspendedRunInner::AmbiguousCommit`. A definite stale observation append instead moves Store's
already-qualified response/rebase package and the Runtime session shell into
`SuspendedRunInner::ObservationRebase`; it never enters the ambiguous-commit path.
The Store `Retryable { append, error }` branch moves that exact append and the still-retained
Runtime continuation back into the corresponding `PreparedDrive`, then into
`SuspendedRunInner::CommitRetry`; resolving it makes one owner-bound retry and never replans or
reinvokes. `FactFrontierChanged` is different from both: plain preparation retains
`DriveBase + RetryAppendPreparation`; reservation preparation retains
`SessionContinuation + RetryReservationPreparation<K,C> +` the same typed prepared invocation
behind `ErasedRetryAccessPreparation`; observation preparation retains
`SessionContinuation + RetryObservationPreparation`. One resolve rebinds only Store's changing fact
context. It does not rerun a state callback, rebuild a request, rescan provider facts, or invoke a
provider. A later direct reservation commit can therefore mint `ReadyToInvoke` from the exact
retained invocation package.

For crate ownership, `PreparedAppend::commit` is the Store-owned consuming subroutine of this
coordinator: it alone consumes the lower backend's raw unit/bytes outcome and returns an append-bound
`CommittedAppend` or `UnresolvedAppend`. This is the concrete meaning of the RFC's “raw outcome
remains inside the consuming coordinator.” Forcing the raw unit across the crate boundary would
require a public uncommitted-successor promotion API; the append-bound Store disposition avoids
that hole. Runtime still owns the outer session/continuation pairing and is the sole constructor of
`CommittedDrive`/`ReadyToInvoke`.

No `NewlyCommitted` enum, Boolean, record ref, head, `CommittedAppend`, or found-same result is
accepted by a `ReadyToInvoke` constructor. This consuming match is the only mint. Add a unit test
inside Runtime that prepares two concurrent access drives and proves no outcome/successor/
continuation transposition API exists; supplement it with compile-fail fixtures for every private
constructor.

### 5.3 Reservation-to-invocation types

Runtime's private access chain is:

```rust
struct TypedPreparedAccessNoFacts<K, C>
where
    K: ReservationMode<C, Facts = NoPriorFacts>,
{
    continuation: SessionContinuation,
    append: PreparedReservationAppend<K, C>,
    invocation: TypedNoFactsInvocation<K, C>,
}

struct TypedPreparedAccessPriorFacts<K, C>
where
    K: ReservationMode<C, Facts = PriorRunFacts>,
{
    continuation: SessionContinuation,
    append: PreparedReservationAppend<K, C>,
    invocation: TypedPriorFactsInvocation<K, C>,
}

struct ReadyToInvoke {
    inner: Box<dyn ErasedReadyAccess>,
}

struct TypedReadyAccessNoFacts<K, C>
where
    K: ReservationMode<C, Facts = NoPriorFacts>,
{
    continuation: SessionContinuation,
    observation: ObservationWriteContinuation<K, C>,
    reservation_ref: RecordRef,
    invocation: TypedNoFactsInvocation<K, C>,
}

struct TypedReadyAccessNeedsFactRequest<K, C>
where
    K: ReservationMode<C, Facts = PriorRunFacts>,
{
    continuation: SessionContinuation,
    observation: ObservationWriteContinuation<K, C>,
    reservation_ref: RecordRef,
    invocation: TypedPriorFactsInvocation<K, C>,
    scan: PriorRunFactScanContinuation,
}

struct TypedReadyAccessFactScan<K, C>
where
    K: ReservationMode<C, Facts = PriorRunFacts>,
{
    continuation: SessionContinuation,
    observation: ObservationWriteContinuation<K, C>,
    reservation_ref: RecordRef,
    invocation: TypedPriorFactsProvider<K, C>,
    request: FactSelectionRequest,
    scan: PriorRunFactScanContinuation,
}

struct CallIdentity {
    // Zero-data allocation identity; private and never serialized.
}

struct AcceptedAccessResponse {
    inner: Box<dyn ErasedAcceptedAccessResponse>,
}

trait ErasedReadyAccess: Send {
    fn invoke_once(self: Box<Self>) -> RuntimeFuture<AccessInvocationStep>;
}

trait ErasedAcceptedAccessResponse: Send {
    fn prepare_observation(
        self: Box<Self>,
        append_id: AppendRequestId,
    ) -> RuntimeFuture<ObservationPreparationStep>;
}

enum ObservationPreparationStep {
    Prepared(PreparedDrive),
    Suspended {
        run: SuspendedRun,
        cause: SuspensionCause,
    },
    Failed {
        run_id: RunId,
        error: TerminalRuntimeError,
    },
}

impl ReadyToInvoke {
    async fn invoke_once(self) -> AccessInvocationStep;
}

enum AccessInvocationStep {
    Accepted(AcceptedAccessResponse),
    Suspended {
        run: SuspendedRun,
        cause: SuspensionCause,
    },
    Faulted {
        session: RunSession,
        fault: AccessInvocationFault,
    },
}
```

The Store-owned reservation package is the sole owner of the exact Read/Effect kind, request and
state-input refs, tenant/run, Program occurrence/path, `ExecutionBindingRef`, semantic call id,
attempt ordinal/id, and append identity. Runtime does not copy those fields into an
`AccessFixation` shadow. The execution binding transitively fixes the exact capability/adapter
implementations and immutable provider/route/signer/effect-domain descriptor. It contains no
principal, grant, policy decision, live generation, lease, fence, currentness metadata, or
provider-entry permit.

Each concrete `TypedPreparedAccessNoFacts<K,C>` or `TypedPreparedAccessPriorFacts<K,C>` owns a
private typed invocation package captured from the immutable ProcessRegistry and the exact typed
request. The package is not a caller-reconstructible raw closure. `K` is exactly `ReadReservation`
or `EffectReservation`; `C` and the fact mode remain nominal through preparation, unknown
resolution, direct commit, `ObservationWriteContinuation<K,C>`, and accepted response. Runtime
uses distinct `TypedReadyAccessNoFacts`, `TypedReadyAccessNeedsFactRequest`, and
`TypedReadyAccessFactScan` self types. Extraction therefore uses ordinary non-overlapping impls;
there is no optional scanner, kind test, `TypeId`, or `invoke(None)` case. Runtime erases only the
whole already-associated drive behind a private object-safe consuming trait so it can store dynamic
Programs; erasure never exposes or re-associates its pieces.

The private action function constructs the matching thunk while it still owns the typed selected
action, then immediately moves it beside the inseparable Store
`PreparedReservationAppend<K,C>` into the typed drive. The journal sees only its callback-free
`ExternalAccessReserved` event. Only the coordinator's direct-new branch moves the thunk,
`SessionContinuation`, Store-owned successor/observation continuation, and fact scanner into
`ReadyToInvoke`; Found/ExistingSame, cold history, Replay, StaleHead, conflict, capacity failure,
and acknowledgement ambiguity cannot.

`ReadyToInvoke` owns the whole active bracket; no separate `RunSession` exists while provider entry
is possible. For a prior-facts access, Runtime first executes the registered `fact_request`
callback once on its bounded CPU executor. It keeps the resulting `FactSelectionRequest` and owns
`PriorRunFactScanContinuation`; the adapter owns neither. Runtime then performs one Store scan. A
`FactScanStep::Retryable` rebuilds the exact typed ready owner inside
`SuspendedRunInner::FactScanRetry`, and one `SuspendedRun::resolve` performs exactly one further
scan by passing a clone of the retained immutable typed request; it never reruns `fact_request` or
decodes that request again. `FactScanStep::Selected` alone constructs the matching
`*AdapterCallAfterFacts` and permits provider entry. A terminal scan failure performs no provider
entry and either reconstructs the exact reservation session from
`ObservationWriteContinuation::into_active()` plus `SessionContinuation`, or fails closed under a
reviewed terminal classification; it never drops a live scanner behind an adapter `Result`.

On any other pre-entry invocation fault, Runtime likewise consumes
`ObservationWriteContinuation::into_active()` and the retained `SessionContinuation` to return the
exact reservation session. On accepted ingress it moves both into `AcceptedAccessResponse`, which
alone can prepare the observation. Cancellation or drop conservatively abandons the in-memory
bracket and leaves the durable reservation unresolved.

Adapter completion must be request-bound before Store can prepare an observation. Make completion
construction flow through the nonconstructible per-call object:

```rust
impl<C: ReadCapabilityContract<Facts = NoPriorFacts>> ReadAdapterCall<C> {
    pub fn request(&self) -> &C::Request;
    pub async fn returned(self, value: C::Returned)
        -> Result<ReadCompletion<C>, ResponseIngressError>;
    pub async fn safe_failure(self, value: C::SafeFailure)
        -> Result<ReadCompletion<C>, ResponseIngressError>;
}

impl<C: EffectCapabilityContract<Facts = NoPriorFacts>> EffectAdapterCall<C> {
    pub fn request(&self) -> &C::Request;
    pub async fn returned(self, value: C::Returned)
        -> Result<EffectCompletion<C>, ResponseIngressError>;
    pub async fn safe_failure(self, value: C::SafeFailure)
        -> Result<EffectCompletion<C>, ResponseIngressError>;
    pub fn entry_unknown(self, code: AccessFaultCode) -> EffectCompletion<C>;
}

impl<C> ReadAdapterCallAfterFacts<C>
where
    C: ReadCapabilityContract<Facts = PriorRunFacts>,
{
    pub fn request(&self) -> &C::Request;
    pub fn facts(&self) -> &FactSelectionReadResponse;
    pub async fn returned(self, value: C::Returned)
        -> Result<ReadCompletion<C>, ResponseIngressError>;
    pub async fn safe_failure(self, value: C::SafeFailure)
        -> Result<ReadCompletion<C>, ResponseIngressError>;
}

impl<C> EffectAdapterCallAfterFacts<C>
where
    C: EffectCapabilityContract<Facts = PriorRunFacts>,
{
    pub fn request(&self) -> &C::Request;
    pub fn facts(&self) -> &FactSelectionReadResponse;
    pub async fn returned(self, value: C::Returned)
        -> Result<EffectCompletion<C>, ResponseIngressError>;
    pub async fn safe_failure(self, value: C::SafeFailure)
        -> Result<EffectCompletion<C>, ResponseIngressError>;
    pub fn entry_unknown(self, code: AccessFaultCode) -> EffectCompletion<C>;
}
```

`ReadCompletion`/`EffectCompletion` have private inners and no public literal. Immediately before
provider entry, the invocation frame allocates an `Arc<CallIdentity>`, retains one clone, and moves
the other into the nonconstructible call object. A completion consumes that call object and carries
the same `Arc` back. Runtime performs exactly one `Arc::ptr_eq` against the retained identity before
accepting the completion; mismatch is a redacted adapter-protocol fault and cannot become an
observation. Nominal privacy and non-`Clone` alone are not claimed to prove correlation: an
`Fn + Sync` adapter could stash an old same-`C` completion and return it for another invocation.
The private allocation identity closes that substitution hole without becoming semantic
revalidation. It is never serialized, hashed, logged, exposed through a constructor, or used as
journal identity. All call, after-facts, and completion types remain non-`Clone`, non-`Copy`, and
non-Serde.

The consuming `returned` and `safe_failure` methods are async because response binding and
canonical qualification are bounded CPU work. They move the call object, typed outcome, and a
Runtime CPU-executor handle into one owned bounded job. That job invokes the capability's sole
request/response relation owner, canonicalizes and qualifies the accepted typed result exactly
once, and returns the completion with its `Arc<CallIdentity>`. An adapter whose transport returns
bytes still owns framing limits, strict decode, and external protocol authentication before calling
these methods. Decode, protocol-authentication, relational, panic, or executor-saturation rejection
returns a redacted `AdapterCallError`/`ResponseIngressError` to the invocation frame and cannot be
converted into an observation. A relation whose failure would make the durable observation illegal
may not be deferred to state settlement.

`invoke_once` contains both provider panic sites without adding a public `UnwindSafe` bound. It
catches synchronous call-construction panic with
`catch_unwind(AssertUnwindSafe(|| adapter(call)))`, then catches a panic while polling the returned
future with `AssertUnwindSafe(future).catch_unwind().await`. It does not `tokio::spawn` or otherwise
detach the provider future merely to catch panic, because caller cancellation must not leave an
unowned provider entry running. After the future returns, the invocation frame checks
`Arc::ptr_eq`, enters the provider no more than once, and returns either a fully ingressed
`AcceptedAccessResponse` or redacted `AccessInvocationFault`. A fault never becomes
`ExternalAccessObserved`: Runtime returns the already advanced reservation session in its
conservative Read retry or Effect possible-entry state. A returned provider response is never
discarded before byte ingress, even when later rebase determines that the attempt is no longer
selected.

Only Runtime matches Store's owner-carrying `FactScanStep`. `Retryable` keeps the exact scan
continuation, typed fact request, observation writer, session continuation, and invocation package
inside `ReadyToInvoke` and its `FactScanRetry` suspension. `Selected` moves the Store response into
the after-facts call, whose `facts()` getter borrows it, and only then calls the provider. `Failed`
performs no provider entry and follows the explicit owner-recovery/fail-closed path above. There are
no adapter-facing `*CallWithFacts` or `*FactScanStep` types, and no adapter `Result` branch can own
or destroy the scanner.

Delete `CertifiedAccessAuthorization`, `CommittedAccessAuthorization`, `ExpectedAuthorization`,
`Authorized<K, V>`, `AuthorizationEntry`, `AuthorizedCallOrigin`, `AuthorizedProviderCall`, public
physical-binding invokers, and any `invoke(None)`/test fallback only in the same vertical change that
lands this chain. Rename legitimate coordination wrappers to reservation terminology; do not remove
unrelated provider-protocol types such as RPC authentication/signature proofs.

### 5.4 Observation commit, rebase, and ambiguity

After invocation, Runtime owns one `AcceptedAccessResponse`: the typed provider outcome, exact
`SessionContinuation`, and the same typed Store `ObservationWriteContinuation<K,C>` that came from
the direct reservation commit. Its private erased prepare method consumes that whole package and
calls the exact mode/C Store continuation; it does not reselect the outstanding reservation or
reconstruct journal coordinates. Store binds the response, derives `ExternalAccessObserved`, and
then owns that qualified event exactly once: in `PreparedObservationAppend`, in the
`RetryObservationPreparation` returned when only the fact frontier changed, or in
`ObservationRebaseInput` after a stale predecessor. Runtime never retains a second response/event
package beside those Store owners. Its `PreparedDrive::Observation` is only
`SessionContinuation + PreparedObservationAppend`; its preparation-retry suspension is only
`SessionContinuation + RetryObservationPreparation`; and its rebase suspension is only
`SessionContinuation + ObservationRebaseInput`.

Map `ObservationPrepareDisposition` exhaustively: `Prepared` becomes
`PreparedDrive::Observation`, `Retryable` becomes
`UnresolvedRuntimePreparation::Observation`, `RebaseRequired` becomes
`PendingObservationRecovery`, and `Failed` is terminal. Map observation commit
`FactFrontierChanged` back to the same preparation-retry owner plus the retained continuation. Its
retry rebinds only Store's changing fact context and does not rerun the state callback, fact-request
callback, fact scan, provider, response relation, or canonicalization. A later `Prepared` commit
uses the one moved event. Direct commit returns a successor session only. Runtime selects that successor's
`SettleReadObservation`/`SettleEffectObservation` action and uses the same typed settlement path as
cold resume.

When observation exact-head comparison loses, keep the already-ingressed response in an opaque
`SuspendedRun`; do not turn it into ordinary `ResumeRequired`. One explicit
`SuspendedRun::resolve` consumes Store's rebase owner, which loads the latest selected prefix under
the same coordinator and performs one reducer-owned check:

```rust
pub enum ObservationRebase {
    Prepared(PreparedObservationAppend),
    AlreadyRecorded(ActiveQualifiedRun),
    NoLongerSelected(ActiveQualifiedRun),
    Conflict,
    Retryable {
        input: ObservationRebaseInput,
        error: RetryableObservationRebaseError,
    },
    Failed(TerminalObservationRebaseError),
}

impl ObservationRebaseInput {
    pub async fn rebase(self) -> ObservationRebase;
}
```

`ObservationRebaseInput` is defined by Store, has private fields and constructors, is non-Serde,
and owns the qualified response event plus exact private Store coordinator/run/tenant identity but
no Runtime callback or invocation handle. It cannot be submitted to a different port or paired
with a caller-supplied latest run. `Prepared` moves that same event into the rebased append and
attempts one commit. If stale again, the disposition returns the moved rebase input in another
opaque suspension. `AlreadyRecorded` discards the duplicate event and settles from the recorded
observation. `NoLongerSelected` consumes the late response event without settlement. Both branches
return the exact latest `ActiveQualifiedRun` already loaded by rebase, so Runtime rebuilds the
session with its retained shell instead of doing a second cold fold. `Conflict` fails closed. None
invokes an adapter.

Observation `AcknowledgementUnknown` quarantines the exact `PreparedDrive::Observation`. One
bounded resolution unit has these branches:

- found exact same observation: qualify it, discard the duplicate local event, and resume
  settlement from retained history;
- proven absent at unchanged predecessor: retry the same prepared observation once;
- proven absent with an advanced head: recover the unchanged qualified event and run one explicit
  rebase;
- repeated unknown: return a new `SuspendedRun`;
- different bytes, invalid retained bytes, capacity failure, or conflict: fail closed.

The reservation logical key and Store reducer admit at most one observation per attempt. A late
non-selected response never settles. Settlement consumes the typed accepted value from qualified
evidence; it does not decode or validate it again.

### 5.5 Crash and concurrency behavior

Two sessions may be resumed independently from the same head. Non-cloneability prevents accidental
duplication of one Rust continuation, not global ownership. Exact-head compare-and-append selects
one successor. A direct access winner alone receives `ReadyToInvoke`; all other branches invoke
zero times.

For a prior-facts winner, a retryable fact scan remains pre-entry: the exact scanner and invocation
owner stay in `FactScanRetry`, and each explicit resolution performs one Store read. Concurrent fact
publication can change the selected response between retry attempts, but the already-built typed
fact request and durable reservation remain fixed. No adapter future is constructed until Store
returns `Selected`; a retry, terminal scan failure, cancellation, or dropped suspension therefore
invokes the provider zero times. Once the after-facts adapter future is constructed, ordinary Read
or Effect entry semantics apply and Runtime never repeats it to repair a scan or response-ingress
failure.

An unobserved reservation in cold history never recreates a thunk:

- Read may reserve a new ordinal because repetition is its declared semantic contract;
- `EntryOnce` becomes `PossibleEntry` and parks under U1;
- `EntryAbsorbing<MAX>` may close/reassert a new ordinal only under its exact duplicate-absorption
  and immutable binding/effect-domain contract; and
- a changed process/binding, timeout, or shutdown never proves that entry did not occur.

Without a generic live fence, a slow live Read or duplicate-absorbing Effect may race the later
ordinal. That is legal only under that capability's repetition/absorption promise and may exhaust
the bounded attempt budget. Preserve tests for this race and conservative liveness disposition.

## 6. Journal, facts, and configuration cutovers

### 6.1 One current reservation wire

The new journal schema has exactly five record families:

```rust
pub enum RunRecord {
    RunAdmitted(RunAdmitted),
    StateTransitionCommitted(StateTransitionCommitted),
    ExternalAccessReserved(ExternalAccessReserved),
    ExternalAccessObserved(ExternalAccessObserved),
    RunClosed(RunClosed),
}

pub enum RecordLogicalKey {
    Admission { run_id: RunId },
    Transition { occurrence_id: OccurrenceId },
    AccessReservation { access_attempt_id: AccessAttemptId },
    Observation { access_attempt_id: AccessAttemptId },
    Closure { run_id: RunId },
}
```

The reservation record fixes at least:

```rust
pub struct ExternalAccessReserved {
    pub access_attempt_id: AccessAttemptId,
    pub attempt_ordinal: u64,
    pub occurrence_id: OccurrenceId,
    pub occurrence_path_ref: ContentRef,
    pub semantic_call_id: SemanticCallId,
    pub state_input_ref: LexicalValueRef,
    pub access_kind: AccessKind,
    pub semantic_head: SemanticHead,
    pub execution_binding_ref: ExecutionBindingRef,
    pub request: TypedValueRef,
    pub request_digest: RequestDigest,
}

pub struct ExternalAccessObserved {
    pub reservation_ref: RecordRef,
    pub access_attempt_id: AccessAttemptId,
    pub outcome: ObservationOutcome,
}

pub enum ObservationOutcome {
    Returned { value: TypedValueRef },
    SafeFailure { value: TypedValueRef },
    EntryUnknown { fault_code: StableId },
}
```

The record envelope/admission and commit fixation already bind run, tenant, store scope, and epoch;
do not duplicate them in this body. `execution_binding_ref` commits the exact descriptor described
in §3.4, including capability/adapter implementations and its content-addressed binding object. The
U6 inventory may add a genuinely immutable proposition inside that binding object; it may not add a
latest/current/release-lineage proposition or copy the same identity into parallel record fields.
`admitted_routing_policy_ref` is absorbed into the binding object and removed as a duplicate.

The same vNext cut normalizes Program/fact references instead of retaining Certified/physical
shadows:

```rust
pub struct RunAdmitted {
    pub program_ref: ProgramRef,
    // existing tenant/config/context/input/fact-source fixations under their new strict names
}

pub struct PriorRunFactSourceRule {
    pub program_refs: Box<[ProgramRef]>,
    // existing bounded source predicates
}

// Selected response material uses mfm_facts::SelectedPriorRunFact and
// mfm_facts::FactSelectionAttestation exactly as defined in §6.2.
```

Rename `SemanticStatePreimage.certified_program_ref` and every other semantic preimage/source field
to the exact `program_ref`/`program_refs` form. Delete
`PriorRunFactScannerBindingCertificate`, schema `mfm.prior-run-fact-scanner-binding`, its physical-
binding ref, and `AdmissionMaterialRefs.stable_resource_lineage_contract_refs`; the Program's
`ExecutionBindingRef`, direct-new fact-scan continuation, reservation ref, dense frontier, and
tenant/source rules already own their independent propositions. Reject literal old field/schema
spellings and freeze new source-rule, selected-fact, attestation, admission, and semantic-state
goldens.

`AccessAttemptId` is derived under a new exact preimage from run, occurrence/path, semantic head,
request digest, execution binding, and ordinal. Do not repeat fields whose containing record/head
already fixes them unless the preimage requires explicit domain separation.

Delete from records, preimages, reducers, projections, and exports:

- `minimum_lineage_head_ref`;
- `stable_resource_lineage_contract_ref`;
- physical release/current generation fields;
- `SupersededBeforeEntry`; and
- principal/grant/policy-decision evidence.

Rename every coordination spelling in the same wire cutover:

| Old | New |
| --- | --- |
| `ExternalAccessAuthorized` | `ExternalAccessReserved` |
| `RecordLogicalKey::Authorization` | `RecordLogicalKey::AccessReservation` |
| `AccessAuthorizationProposal` | `AccessReservationProposal` |
| `authorization_ref` | `reservation_ref` |
| `AuthorizationEntry` | `ReservationEntry` |
| authorization barrier/frontier | access-reservation barrier/frontier |
| live EVM authorized call origin | reservation call origin |

Do not globally ban the English word “authorization”: provider protocol authentication and
PostgreSQL roles remain legitimate. Use path-scoped source/API assertions for the deleted MFM
coordination symbols.

The candidate, assigned-record, commit, and `JournalHead` algorithms and their existing domain
separators remain unchanged. The changed record bytes produce new hashes under a fresh store scope/
epoch; old tags, fields, fact-mode spellings, schemas, aliases, and decoders are rejected. Freeze
new golden vectors for every record family, candidate, record ref, commit digest, attempt identity,
and predecessor mutation.

### 6.2 Dense fact completeness

Keep the tenant fact head and dense publication sequence as append-atomic authority. Rename
`CompleteThroughAuthorizationFrontier` and its wire spelling to
`CompleteThroughAccessReservationFrontier`. Every publication transaction locks/checks the expected
tenant fact head, inserts the next dense route, and advances the head atomically with its run
append. A mismatch is `FactFrontierChanged`, never `StaleHead`.

A hot query reads the authoritative `tenant_fact_heads` row. It does not repeatedly load the latest
route merely to compare it back to the head. Route/head continuity, exact producer record/head,
missing routes, and gaps are checked when the selected producer closure crosses ingress or during
`audit_store`.

One consuming selection scope may retain an ephemeral callback-free `QualifiedPrefixSet` so the
same exact producer prefix is not reloaded inside that request/session. It is dropped with the
scope; it is not an LRU or process-global authority. The reserved one-use fact scanner remains
affine and bound to the exact access attempt, but rename “permit” to “fact-scan continuation.”

Settle the dependency direction before moving the response. `mfm-facts` must remain independent of
Journal, so move the address-only `RecordRef`, `JournalHead`, `TenantFactFrontier`, and
`TypedValueRef` (and the digest newtypes contained by them) to `mfm-ids` beside `ProgramRef` and
`ExecutionBindingRef`. They retain strict checked Serde/conversion and their current hash/preimage
algorithms; `mfm-journal` still owns construction of assigned records, heads, and fact-publication
coordinates. Constructing one of these serializable addresses does not qualify what it names.
`mfm-facts` then owns `SelectedPriorRunFact`, `FactSelectionAttestation`,
`FactSelectionQueryResult`, and the one actual typed response shape; Journal depends on Facts for
those value DTOs and does not define a shadow:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct FactQueryOrdinal(u8); // 0..=127; private field

impl FactQueryOrdinal {
    pub fn try_from_index(index: usize) -> Result<Self, FactSelectionResponseError>;
    pub const fn index(self) -> usize;
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct FactSelectionRequestDigest(ContentDigest); // private checked digest address

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum FactSelectionResponseError {
    #[error("fact selection response exceeds frozen bounds")]
    BoundExceeded,
    #[error("fact selection response has an invalid canonical value or reference")]
    InvalidValue,
    #[error("fact selection result does not match its authored query")]
    QueryMismatch,
    #[error("fact selection result order is invalid")]
    InvalidOrder,
    #[error("fact selection attestation does not match the reservation frontier")]
    AttestationMismatch,
}

impl FactSelectionRequest {
    pub fn request_digest(&self) -> Result<FactSelectionRequestDigest, FactError>;
    pub fn query_ref(
        &self,
        ordinal: FactQueryOrdinal,
    ) -> Result<ContentRef, FactSelectionResponseError>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FactSelectionCompleteness {
    CompleteThroughAccessReservationFrontier,
}

pub struct FactSelectionAttestation {
    reservation_ref: RecordRef,
    access_reservation_frontier: TenantFactFrontier,
    observed_tenant_fact_head: TenantFactFrontier,
}

impl FactSelectionAttestation {
    pub fn complete(
        reservation_ref: RecordRef,
        access_reservation_frontier: TenantFactFrontier,
        observed_tenant_fact_head: TenantFactFrontier,
    ) -> Result<Self, FactSelectionResponseError>;

    pub fn reservation_ref(&self) -> &RecordRef;
    pub fn access_reservation_frontier(&self) -> &TenantFactFrontier;
    pub fn observed_tenant_fact_head(&self) -> &TenantFactFrontier;
}

pub struct SelectedPriorRunFact {
    // Private Program/record/value addresses plus bounded canonical subject/response/claim bytes.
}

impl SelectedPriorRunFact {
    #[allow(clippy::too_many_arguments)]
    pub fn try_from_parts(
        publication_frontier: TenantFactFrontier,
        producer_record_ref: RecordRef,
        producer_program_ref: ProgramRef,
        producer_entry_point_operation_id: StableId,
        emission_ordinal: u32,
        descriptor_ref: ContentRef,
        subject: TypedValueRef,
        response: TypedValueRef,
        claim_ref: ContentRef,
        content_identity: FactContentIdentityDigest,
        fact_identity: FactLogicalIdentityDigest,
        subject_canonical_json: impl Into<Box<str>>,
        response_canonical_json: impl Into<Box<str>>,
        claim_canonical_json: impl Into<Box<str>>,
    ) -> Result<Self, FactSelectionResponseError>;

    pub fn publication_frontier(&self) -> &TenantFactFrontier;
    pub fn producer_record_ref(&self) -> &RecordRef;
    pub fn producer_program_ref(&self) -> &ProgramRef;
    pub fn descriptor_ref(&self) -> &ContentRef;
    pub fn subject(&self) -> &TypedValueRef;
    pub fn response(&self) -> &TypedValueRef;
    pub fn claim_ref(&self) -> &ContentRef;
    pub fn content_identity(&self) -> &FactContentIdentityDigest;
    pub fn fact_identity(&self) -> &FactLogicalIdentityDigest;
    // Borrowed getters also expose the entry/ordinal and three canonical strings.
}

pub struct FactSelectionReadResponse {
    completeness: FactSelectionCompleteness,
    attestation: FactSelectionAttestation,
    request_digest: FactSelectionRequestDigest,
    query_results: Box<[FactSelectionQueryResult]>,
}

pub struct FactSelectionQueryResult {
    ordinal: FactQueryOrdinal,
    query_ref: ContentRef,
    selected: Box<[SelectedPriorRunFact]>,
}

impl FactSelectionQueryResult {
    pub fn for_request_query(
        request: &FactSelectionRequest,
        ordinal: FactQueryOrdinal,
        selected: Vec<SelectedPriorRunFact>,
    ) -> Result<Self, FactSelectionResponseError>;

    pub const fn ordinal(&self) -> FactQueryOrdinal;
    pub fn query_ref(&self) -> &ContentRef;
    pub fn selected(&self) -> &[SelectedPriorRunFact];
}

impl FactSelectionReadResponse {
    pub fn from_complete_scan(
        request: &FactSelectionRequest,
        attestation: FactSelectionAttestation,
        query_results: Vec<FactSelectionQueryResult>,
    ) -> Result<Self, FactSelectionResponseError>;

    pub const fn completeness(&self) -> FactSelectionCompleteness;
    pub fn attestation(&self) -> &FactSelectionAttestation;
    pub fn request_digest(&self) -> &FactSelectionRequestDigest;
    pub fn query_results(&self) -> &[FactSelectionQueryResult];

    pub fn validate_for_reservation(
        &self,
        request: &FactSelectionRequest,
        expected_reservation: &RecordRef,
        expected_frontier: &TenantFactFrontier,
    ) -> Result<(), FactSelectionResponseError>;
}
```

All fields are private. `SelectedPriorRunFact::try_from_parts` bounds and strictly validates each
canonical string, checks its typed value/claim content references, and recomputes the selected
content/logical identities; it never accepts caller-supplied bytes and inconsistent refs as one
value. `FactSelectionAttestation::complete` requires exact equality of the
captured reservation frontier and the observed tenant fact head, including store scope, epoch,
tenant, and dense order. `FactSelectionQueryResult::for_request_query` derives `query_ref` from the
exact authored query instead of accepting it from the caller; it rejects an ordinal absent from the
request, a nonmatching fact, a duplicate fact identity, more values than that query's `limit`, or
values outside the query's deterministic ordering/tie-break. `from_complete_scan` derives the
request digest, requires exactly one result for every request query in contiguous authored ordinal
order, rejects missing/surplus/reordered/duplicate query groups, enforces the request's aggregate
selected-result and response-byte bounds plus the absolute maxima, and fixes the sole completeness
variant. Checked `Deserialize` applies the same intrinsic shape/order/absolute-bound rules;
`validate_for_reservation` is the one contextual request/reservation/frontier relation check at
Store ingress. No caller-supplied digest, query ref, completeness enum, or unchecked wire struct can
enter the qualified response.

Store constructs this value once from qualified producer evidence and returns it directly through
the request-bound fact call. Runtime retains that same typed value and settlement never parses an
embedded payload. Delete `FactSelectionReadResponse.canonical_response_base64url`,
`canonical_response_json`, `from_canonical_json`, the parallel Journal-owned
`PriorRunFactSelectionResponse`, `PriorRunFactQueryResult`, `PriorRunFactScanAttestation`,
`PriorRunFactCompletenessMode`, and their old schema/domain/goldens. There is no separate
`PriorRunFactScanResult` wrapper and no JSON/base64 encode-decode bridge between Facts, Journal,
Store, Program, or Runtime. Add one-ingress counters and migrate `mfm-facts` tests around the former
response wrapper plus Store scanner/Program value consumers.

Facts tests must cover ordinals 0 and 127, 128 query groups, zero/129 groups, wrong/repeated/skipped/
reordered ordinal, wrong derived query ref, per-query and aggregate selection overflow, duplicate or
misordered selected identities, response-byte overflow, attestation store/epoch/tenant/head mismatch,
and checked-Deserialize parity. Store tests cover dense negative completeness, exact-frontier
mismatch, wrong producer head, missing/gapped route, response/request/reservation binding, source
closure, same-scope reuse, and new-resume re-ingress. Freeze canonical request, selected-fact,
attestation, query-result, response, and digest goldens under fresh access-reservation schema ids.

### 6.3 Conditional Effect-attention projection

Resolve U3 before editing the schema.

When enabled, `PreparedAppend` contains the reducer-derived `needs_effect_attention` Boolean and the
PostgreSQL transaction updates `run_history_heads.needs_effect_attention` atomically with the head.
A partial index supplies one snapshot-complete tenant listing. A route hit is not run authority;
Runtime explicitly resumes and qualifies the selected run before recovery.

When absent, remove the Boolean, partial index, list method, DTOs, CLI/REST surface, known-gaps
contract, and tests in the same commit. Do not leave an optional field or disabled runtime branch.
No second attention frontier, startup rebuild, or semantic cache exists in either ruling.

### 6.4 Configuration types and commit coordinator

Configuration remains a distinct domain with the same trust discipline:

```rust
pub struct ResolvedConfiguration<C: MfmConfig> {
    value: Arc<ValidatedConfig<C>>,
    key: ConfigurationKey,
    head: ConfigurationHead,
    store: StructuredStoreIdentity,
}

#[derive(Clone)]
pub struct ConfigurationStore(Arc<ConfigurationCoordinatorInner>);

pub struct ConfigurationAdmissionEvidence<C: MfmConfig> {
    // Private exact store/key/head/content contract refs; no detached typed value.
}

pub struct ConfigurationWriteSession<C: MfmConfig> {
    owner: Arc<ConfigurationCoordinatorInner>,
    current: Option<ResolvedConfiguration<C>>,
    // Non-Clone active owner for this key/head.
}

pub struct PreparedConfigurationAppend<C: MfmConfig> {
    owner: Arc<ConfigurationCoordinatorInner>,
    command: BackendConfigurationAppend,
    successor: PreparedConfigurationSuccessor<C>,
}

pub struct SuspendedConfigurationAppend<C: MfmConfig> {
    // Exact owner Arc plus affine preparation after unknown acknowledgement.
}
```

Required public API:

```rust
impl ConfigurationStore {
    pub async fn load<C: MfmConfig>(
        &self,
        key: &ConfigurationKey,
        head: ConfigurationSelection,
    ) -> Result<ResolvedConfiguration<C>, ConfigurationLoadError>;

    pub async fn begin_write<C: MfmConfig>(
        &self,
        key: &ConfigurationKey,
    ) -> Result<ConfigurationWriteSession<C>, ConfigurationLoadError>;
}

impl<C: MfmConfig> ConfigurationWriteSession<C> {
    pub fn current(&self) -> Option<&ResolvedConfiguration<C>>;
    pub fn into_current(self) -> Option<ResolvedConfiguration<C>>;
    pub fn prepare(
        self,
        next: ValidatedConfig<C>,
        append_id: ConfigurationAppendId,
    ) -> ConfigurationPrepareStep<C>;
}

impl<C: MfmConfig> PreparedConfigurationAppend<C> {
    pub async fn commit(self) -> ConfigurationCommitStep<C>;
}

impl<C: MfmConfig> SuspendedConfigurationAppend<C> {
    pub async fn resolve(self) -> ConfigurationCommitStep<C>;
}

impl<C: MfmConfig> ResolvedConfiguration<C> {
    pub fn value(&self) -> &ValidatedConfig<C>;
    pub fn key(&self) -> &ConfigurationKey;
    pub fn head(&self) -> &ConfigurationHead;
    pub fn store_identity(&self) -> &StructuredStoreIdentity;
    pub fn content_ref(&self) -> &ContentRef;
    pub fn into_admission_evidence(self) -> ConfigurationAdmissionEvidence<C>;
}

pub enum ConfigurationCommitStep<C: MfmConfig> {
    DirectlyCommitted(ConfigurationWriteSession<C>),
    ExistingSame,
    StaleHead,
    AcknowledgementUnknown(SuspendedConfigurationAppend<C>),
    Retryable {
        owner: ConfigurationRetryOwner<C>,
        error: RetryableConfigurationError,
    },
    Failed(TerminalConfigurationError),
}

pub enum ConfigurationPrepareStep<C: MfmConfig> {
    Prepared(PreparedConfigurationAppend<C>),
    Rejected {
        session: ConfigurationWriteSession<C>,
        error: ConfigurationPrepareError,
    },
}

pub enum ConfigurationRetryOwner<C: MfmConfig> {
    Prepared(PreparedConfigurationAppend<C>),
    Suspended(SuspendedConfigurationAppend<C>),
}

impl<C: MfmConfig> ValidatedConfig<C> {
    pub fn from_typed(value: C) -> Result<Self, ConfigurationValueError>;
    pub fn from_source(
        source_bytes: &[u8],
        limits: ConfigValueIngressLimits,
    ) -> Result<Self, ConfigValueIngressError>;
    pub fn from_retained(
        expected_content_ref: &ContentRef,
        canonical_bytes: &[u8],
        limits: ConfigValueIngressLimits,
    ) -> Result<Self, ConfigValueIngressError>;

    pub fn value(&self) -> &C;
    pub fn canonical_json(&self) -> &str;
    pub fn content_ref(&self) -> &ContentRef;
}
```

Reuse `mfm-values::{MfmConfig, ValidatedConfig<C>}` instead of creating a parallel configuration
value/schema abstraction. `mfm-values` also owns the callback-free static `C` contract,
`ConfigValueIngressLimits`, and `ConfigValueIngressError`, avoiding a Store→Values dependency
cycle or an illegal foreign inherent impl. There is no `ConfigurationIngress` wrapper or
`ConfigurationStore::ingress` method that repeats this proposition. Redesign the value to own
`(C, Arc<str>, ContentRef)`.
`from_typed` canonicalizes and addresses an already invariant-safe `C` once without decoding it;
`from_source` bounds, strictly decodes/validates, canonicalizes, and computes its ref;
`from_retained` performs the same one private decode path and additionally checks the persisted
expected ref. No public `Option<ContentRef>` conflates those ingresses. Store wraps/maps errors and
binds the value to key/head/epoch in `ResolvedConfiguration`; the value itself claims no Store
authority. Neither a resolved value nor writer duplicates bytes or recomputes the ref. `load` folds
the complete bounded selected chain once.

Every write/session/prepared/suspended product carries the exact private
`Arc<ConfigurationCoordinatorInner>`. Preparation is a consuming method on
`ConfigurationWriteSession`; commit and resolution are consuming methods on their owner-bound
packages. Cross-store, cross-epoch, or cross-contract transposition is therefore not an operation,
and no posterior owner-id comparison repairs it. `ConfigurationWriteSession<C>` is non-Clone, so
two local prepared successors cannot be forked out of one active owner. `prepare` binds exact
predecessor/content/revision and retains the typed successor. The configuration backend borrows the
exact non-Clone command while its package remains owned, just like run history. Direct new alone
moves out its successor into the next active session, so several direct writes do not reload their
own history. Found raw bytes ingress and compare once; existing-same promotes nothing and a further
write requires explicit `begin_write`. Unknown retains the same owner-bound preparation for one
bounded explicit resolution unit. No append path loads the full history.

Secret-bearing inputs are consumed at process assembly into concrete immutable adapter/provider/
signer internals; only secret-free configuration values/references persist. Preserve bounded input,
redaction, zeroization, keystore AAD/anti-swap binding, and constant-time comparisons. No generic
issuer/resource handle, live generation, or lease emerges from configuration.

Delete the current writer full-prefix reload, repeated `ValidatedConfigurationAppend::from_object`
on local revisions, positive backend echo, reader-specific replay, shared cache/suffix ideas, and
ordinary readiness scan. Current-active selection remains a changing explicit query; it is not
immutable-byte revalidation.

The valid-by-representation migration also deletes the current semantic
`MfmConfig::validate`/`ConfigError` hook and the old `ValidatedConfig::new` validator wrapper.
Checked constructors and checked `Deserialize` own intrinsic validity; the three constructors shown
above own only typed canonicalization or one hostile byte ingress. Do not retain both mechanisms.

## 7. Tenant facade, transport, export, and EVM identity

### 7.1 Tenant-scoped Application API

Application composition owns one typed, callback-free entry/planning registry:

```rust
#[derive(Clone)]
pub struct PublishedEntryPointCatalog(Arc<PublishedEntryPointCatalogInner>);

pub struct PublishedEntryPointCatalogBuilder {
    catalog: ProgramCatalog,
    // Private bounded entry ids, source decoders, root ABIs, and planner descriptors.
}

#[derive(Clone)]
pub struct ConfigurationPlannerRegistry(Arc<ConfigurationPlannerRegistryInner>);

pub struct ConfigurationPlannerRegistryBuilder {
    entries: PublishedEntryPointCatalogBuilder,
    // Private typed planner implementations before one consuming finish.
}

pub struct PublishedEntryPoint {
    // Private stable entry id, request limits, config contract, entry profile, and planner ref.
}

/// A planner's tenant-free choice within its registered configuration family.
pub struct ConfigurationTargetSelection {
    target_id: StableId,
    head: ConfigurationSelection,
}

impl ConfigurationTargetSelection {
    pub fn new(target_id: StableId, head: ConfigurationSelection) -> Self;
    pub fn target_id(&self) -> &StableId;
    pub fn head(&self) -> &ConfigurationSelection;
}

pub struct EntryPlanningCall<'a, Request, C, Input, Output, Failure>
where
    Request: StructuredValue,
    C: MfmConfig,
    Input: StructuredValue,
    Output: StructuredValue,
    Failure: FailureValue,
{
    configuration: ResolvedConfiguration<C>,
    materials: AdmissionMaterialScope<Input, Output, Failure>,
    // Borrowed request plus exact catalog/profile/planner brand; all other fields private.
}

pub struct PlannedAdmission<C, Input, Output, Failure>
where
    C: MfmConfig,
    Input: StructuredValue,
    Output: StructuredValue,
    Failure: FailureValue,
{
    program: TypedProgram<Input, Output, Failure>,
    root_input: QualifiedTypedValue<Input>,
    configuration: ConfigurationAdmissionEvidence<C>,
    context: AdmissionContextCandidate,
    fact_sources: FactSourceManifestCandidate,
}

type AppFuture<T> = Pin<Box<dyn Future<Output = T> + Send + 'static>>;

trait ErasedAdmissionPlanningRetry: Send {
    fn retry(
        self: Box<Self>,
        process: Arc<ComposedMfmProcess>,
    ) -> AppFuture<AdmissionPlanningStep>;
}

enum AdmissionPlanningStep {
    Prepared(AdmissionInput),
    Retryable {
        planning: Box<dyn ErasedAdmissionPlanningRetry>,
        error: PublicRetryableError,
    },
    Rejected(PublicError),
}

impl ConfigurationPlannerRegistryBuilder {
    pub fn new(catalog: ProgramCatalog) -> Self;

    pub fn register<Request, C, Input, Output, Failure>(
        &mut self,
        descriptor: EntryPlannerDescriptor,
        entry: TypedEntryProfile<Input, Output, Failure>,
        select_configuration: impl Fn(&Request) -> ConfigurationTargetSelection
            + Send
            + Sync
            + 'static,
        plan: impl for<'a> Fn(
                EntryPlanningCall<'a, Request, C, Input, Output, Failure>,
            ) -> Result<PlannedAdmission<C, Input, Output, Failure>, PlanningError>
            + Send
            + Sync
            + 'static,
    ) -> Result<(), PlannerRegistrationError>
    where
        Request: StructuredValue,
        C: MfmConfig,
        Input: StructuredValue,
        Output: StructuredValue,
        Failure: FailureValue;

    pub fn finish(
        self,
    ) -> Result<(PublishedEntryPointCatalog, ConfigurationPlannerRegistry), PlannerBuildError>;
}

impl PublishedEntryPointCatalog {
    pub fn entries(&self) -> &[PublishedEntryPoint];
    pub fn get(&self, id: &EntryPointId) -> Option<&PublishedEntryPoint>;
}

impl<'a, Request, C, Input, Output, Failure>
    EntryPlanningCall<'a, Request, C, Input, Output, Failure>
where
    Request: StructuredValue,
    C: MfmConfig,
    Input: StructuredValue,
    Output: StructuredValue,
    Failure: FailureValue,
{
    pub fn request(&self) -> &Request;
    pub fn configuration(&self) -> &ValidatedConfig<C>;
    pub fn catalog(&self) -> &ProgramCatalog;
    pub fn entry_profile(&self) -> &TypedEntryProfile<Input, Output, Failure>;

    pub fn context<I>(
        &self,
        values: I,
    ) -> Result<AdmissionContextCandidate, AdmissionContextError>
    where
        I: IntoIterator<Item = QualifiedValue>;

    pub fn fact_sources<I>(
        &self,
        rules: I,
    ) -> Result<FactSourceManifestCandidate, FactSourceManifestError>
    where
        I: IntoIterator<Item = PriorRunFactSourceRule>;

    pub fn finish(
        self,
        program: TypedProgram<Input, Output, Failure>,
        root_input: QualifiedTypedValue<Input>,
        context: AdmissionContextCandidate,
        fact_sources: FactSourceManifestCandidate,
    ) -> Result<PlannedAdmission<C, Input, Output, Failure>, PlanningError>;
}

impl ConfigurationPlannerRegistry {
    async fn prepare_admission(
        &self,
        runtime: &Runtime,
        configurations: &ConfigurationStore,
        tenant: &TenantScopeId,
        request: AdmitRunRequest,
    ) -> AdmissionPlanningStep;
}
```

The published entry's private erased decoder invokes
`ProgramCatalog::ingress_source_typed<Request>` exactly once at CLI/REST ingress and stores the
typed request inside `AdmitRunRequest`. It invokes the registered pure selector once and stores its
target/head result in that same opaque request; planning and retry never invoke the selector again.
The selector can return only a target id and head selection; it cannot name a tenant, Store scope,
or entry operation. `prepare_admission` constructs the full `ConfigurationKey` from the stored
target/head plus the fixed facade tenant, the opened Store scope, and the registered entry
operation, then cold-qualifies exactly that one configuration. It moves the result into
`EntryPlanningCall` together with the exact `AdmissionMaterialScope` obtained through Runtime for
that tenant/entry, then invokes the matching typed planner with borrowed request/config getters. No
request field, planner callback, or public constructor can replace any injected partition or scope
component.

`EntryPlanningCall::{context, fact_sources}` are the only planner-facing candidate constructors.
They use the call's exact Store-owned `AdmissionMaterialScope`, catalog, tenant, entry profile, and
fixed admission-material bounds. The App methods are typed delegations to the scope's sole checked
Store-owned mints; App never writes either candidate's private fields. `context` accepts
only catalog-branded `QualifiedValue`s, rejects a foreign catalog, duplicate slot, wrong contract,
or count/aggregate-byte overflow, and does not encode/decode a value. `fact_sources` accepts only
valid-by-representation `PriorRunFactSourceRule`s, canonicalizes order once, and rejects duplicate
producer classes and rule/reference/byte overflow. Neither candidate has a public field, raw-byte
constructor, `Deserialize`, or constructor detached from an exact scope retained by an
`EntryPlanningCall`.
`EntryPlanningCall::finish(self, ..)` consumes that exact resolved value into
`ConfigurationAdmissionEvidence<C>`, checks the typed Program/root/candidates against the same
catalog/profile once, and calls
`Runtime::prepare_admission` with the still-typed Program/root pair. Its private erased registry is
only dynamic dispatch over registrations whose complete `(Request, C, Input, Output, Failure)` ABI
and catalog brand were fixed by `register`; it does not use `TypeId`, public `Any`, or
serialize/redecode. Before `AdmissionInput` exists, a capacity retry erases the exact typed request,
selected `ResolvedConfiguration`, material scope, and planner into
`ErasedAdmissionPlanningRetry`; after `AdmissionInput` exists, Runtime returns that exact input.
Neither retry reconstructs an earlier phase.

Composition validates entry id uniqueness, exact catalog instance, entry profile, request/config
contracts, planner implementation identity, and complete Runtime binding coverage once. It cannot
validate future config-derived `ProgramRef`s; each planner constructs that actual Program under the
same catalog and admission records its resulting ref. `PublishedEntryPointCatalog` remains in the
composed process because transports need its exact source decoder and bounds.

`Application` is a cloneable facade over one immutable composed process and one fixed tenant:

```rust
#[derive(Clone)]
pub struct Application {
    tenant: TenantScopeId,
    store_scope_id: StoreScopeId,
    process: Arc<ComposedMfmProcess>,
}

pub struct RunExecutor {
    // Same composed Runtime + affine RunSession; private, non-Clone, non-Serde.
}

pub struct SuspendedExecution {
    // Same composed Runtime + SuspendedRun; private, non-Clone, non-Serde.
}

pub struct PendingAdmission {
    process: Arc<ComposedMfmProcess>,
    inner: PendingAdmissionInner,
    // Private, non-Clone, non-Serde owner of one exact pre-spawn or spawn phase.
}

enum PendingAdmissionInner {
    Planning(Box<dyn ErasedAdmissionPlanningRetry>),
    Spawning(AdmissionInput),
}

pub enum DriveRunResult {
    Advanced {
        executor: RunExecutor,
        response: DriveResponse,
    },
    Waiting {
        executor: RunExecutor,
        response: DriveResponse,
    },
    Closed {
        response: DriveResponse,
    },
    ResumeRequired {
        run_id: RunId,
        cause: PublicResumeCause,
    },
    Suspended {
        execution: SuspendedExecution,
        response: DriveResponse,
    },
    Retryable {
        executor: RunExecutor,
        error: PublicRetryableError,
    },
    Failed {
        run_id: RunId,
        error: PublicTerminalError,
    },
}

pub enum SpawnRunResult {
    Started {
        executor: RunExecutor,
        response: SpawnResponse,
    },
    Existing {
        run_id: RunId,
    },
    Suspended {
        execution: SuspendedExecution,
        response: SpawnResponse,
    },
    Retryable {
        admission: PendingAdmission,
        error: PublicRetryableError,
    },
    Rejected {
        error: PublicError,
    },
}
```

The public facade is:

```rust
impl Application {
    // Called only by trusted process/deployment composition.
    pub fn for_tenant(
        process: Arc<ComposedMfmProcess>,
        tenant: TenantScopeId,
    ) -> Result<Self, ApplicationAssemblyError>;

    pub fn tenant_scope_id(&self) -> &TenantScopeId;
    pub fn entry_points(&self) -> &PublishedEntryPointCatalog;
    pub fn decode_admit_json(
        &self,
        bytes: &[u8],
        limits: &TransportIngressLimits,
    ) -> Result<AdmitRunRequest, RequestIngressError>;
    pub fn decode_admit_cli(
        &self,
        args: ParsedRunArguments,
        limits: &TransportIngressLimits,
    ) -> Result<AdmitRunRequest, RequestIngressError>;
    pub async fn check_ready(&self) -> Result<(), PublicError>;

    pub async fn spawn_run(
        &self,
        request: AdmitRunRequest,
    ) -> SpawnRunResult;

    pub async fn resume_run(
        &self,
        run_id: RunId,
    ) -> Result<RunExecutor, PublicError>;

    pub async fn read_public_run(
        &self,
        run_id: RunId,
    ) -> Result<PublicRunView, PublicError>;

    pub async fn replay_run(
        &self,
        run_id: RunId,
        request: ReplayRequest,
    ) -> Result<ReplayResponse, PublicError>;

    pub async fn read_transition_trace(
        &self,
        run_id: RunId,
        page: PageRequest,
    ) -> Result<TransitionTracePage, PublicError>;

    pub async fn read_access_audit(
        &self,
        run_id: RunId,
        page: PageRequest,
    ) -> Result<AccessAuditPage, PublicError>;

    pub async fn export_run(
        &self,
        run_id: RunId,
        request: ExportRequest,
    ) -> Result<ExportedRun, PublicError>;

}

impl RunExecutor {
    pub fn run_id(&self) -> &RunId;
    pub async fn drive(self) -> DriveRunResult;
}

impl SuspendedExecution {
    pub fn run_id(&self) -> &RunId;
    pub async fn resolve(self) -> DriveRunResult;
}

impl PendingAdmission {
    pub async fn retry(self) -> SpawnRunResult;
}
```

The public ownership enums are exhaustive and map Runtime variants one-for-one. A retryable
drive returns the same executor plus a redaction-safe error; an outer `Err` must never discard an
affine owner. `SpawnRunResult` distinguishes direct start, existing-run explicit resume, ambiguous
admission suspension, owner-carrying retry, and terminal pre-owner rejection. `PendingAdmission`
has two private exhaustive phases. `Planning` owns the exact typed request, fixed selector result,
selected `ResolvedConfiguration` once loaded, material scope, and erased planner when the bounded
planning executor had no capacity; its retry resumes that same phase and invokes the planner at
most once. `Spawning` owns the already-created `AdmissionInput` returned by Runtime capacity retry
and submits only that input again. Neither phase reparses caller bytes, reruns the selector,
reloads an already-selected configuration, or reconstructs an earlier owner. `Application` maps
private `AdmissionPlanningStep::Retryable` to the first phase and `SpawnStep::Retryable` to the
second; `PendingAdmission::retry` maps a newly prepared input through the same spawn outcome
function before returning. No branch installs a locally prepared genesis on found-same.

Caller bytes/arguments are a real ingress even though caller authentication is deleted. App owns
opaque operational DTOs with private fields and checked constructors; transports own framing and
size limits:

```rust
#[derive(Clone, Debug)]
pub struct TransportIngressLimits {
    pub max_json_body_bytes: NonZeroUsize,
    pub max_cli_value_bytes: NonZeroUsize,
    pub max_import_bytes: NonZeroU64,
}

pub struct AdmitRunRequest {
    // Private catalog brand, checked entry point, idempotency key, tenant-free target/head
    // selection, and bounded typed input.
}
```

REST bounds the body before allocation/parsing, rejects duplicate/unknown fields and trailing data,
strictly decodes once, and computes canonical/content identity only for fields that enter a hash.
CLI bounds every file/string/stdin source before conversion and calls the same domain constructors.
Neither schema contains tenant, credential, principal, grant, or policy fields. App consumes the
opaque DTO and never reparses it. The transport obtains discovery metadata through
`Application::entry_points` and must obtain the operational request through that same
Application's decoder; there is no free `AdmitRunRequest` decoder taking an arbitrary catalog.
The request retains the exact composed-process/catalog brand. A request decoded by another
composed process is rejected before configuration lookup, while facades over the same process may
apply their own fixed tenant to the same tenant-free request. Apply the same pattern to
resume/read/replay/export and configuration source requests; do not create one generic unbounded
JSON envelope. Portable offline qualification is the separate callback-free §7.3 byte API and has
no App/CLI/REST request surface. Tests cover
oversize, duplicate/unknown field, invalid UTF-8/lexical values, noncanonical hashed input,
trailing bytes, CLI/REST equivalence, cross-catalog request transposition, two fixed-tenant facades
selecting distinct configuration streams from the same target choice, and counters proving zero
App reparses. A backend row for another tenant is rejected before the planner callback receives a
configuration value. Compile-pass coverage constructs context/fact candidates through a real
Store-minted scope from downstream App, while compile-fail coverage rejects candidate literals,
foreign-scope transposition, and direct access to the retained port. Saturation tests exercise both
`PendingAdmission` phases and prove the planning phase retains the exact resolved configuration and
invokes the selector/load/planner at most once, while the spawning phase submits only the same
`AdmissionInput`.

When U3 is enabled, add only:

```rust
pub async fn list_effect_entry_attention(
    &self,
) -> Result<EffectEntryAttentionInventory, PublicError>;
```

The enabled U3 ruling must set a maximum inventory cardinality. PostgreSQL materializes the bounded
inventory from one snapshot; exceeding the published bound is a capacity error, not an incomplete
page. Do not claim one-snapshot completeness across independently issued page requests.

Whole-store diagnostics do not belong on a tenant facade. Trusted composition may separately
retain an opaque operator surface:

```rust
pub struct StoreOperator {
    audit: StoreAuditPort,
}

impl StoreOperator {
    pub async fn audit_store(
        &mut self,
        request: StoreAuditRequest,
    ) -> Result<StoreAuditReport, OperatorError>;
}
```

It is never reachable from `Application`, CLI/REST tenant routes, or a `RunExecutor` unless the
embedding deliberately exposes a separate operator process/API. Its audit is unpartitioned by
design and creates no execution authority.

After trusted `for_tenant` construction, no operational facade call accepts `TenantScopeId`,
credential, access token, principal, grant, or policy decision. The concrete composed process
always supplies the facade's stored tenant to planning, Store, and Runtime.
Two facades may share the same composed process/backend, but a run outside one facade's query
partition is `RunNotFound`. If PostgreSQL returns bytes for that partition whose admitted tenant
differs, return redaction-safe `InvalidHistory`.

`ComposedMfmProcess` is minted only by this explicit trusted composition API:

```rust
pub struct ProcessComposition {
    runtime_assembly: RuntimeAssembly,
    store: StoreParts,
    entry_points: PublishedEntryPointCatalog,
    configuration_planners: ConfigurationPlannerRegistry,
    runtime_limits: RuntimeLimits,
}

pub struct ComposedMfmProcess {
    runtime: Runtime,
    reader: HistoryReader,
    configuration: ConfigurationStore,
    entry_points: PublishedEntryPointCatalog,
    planners: ConfigurationPlannerRegistry,
}

pub struct ComposedProcessParts {
    pub process: Arc<ComposedMfmProcess>,
    pub operator: StoreOperator,
}

impl ProcessComposition {
    pub fn new(
        runtime_assembly: RuntimeAssembly,
        store: StoreParts,
        entry_points: PublishedEntryPointCatalog,
        configuration_planners: ConfigurationPlannerRegistry,
        runtime_limits: RuntimeLimits,
    ) -> Self;

    pub fn qualify(self) -> Result<ComposedProcessParts, ProcessCompositionError>;
}
```

`qualify` consumes the one `QualifiedHistoryPort` into Runtime, retains clones of the callback-free
`HistoryReader` and `ConfigurationStore`, and gives the distinct `StoreAuditPort` to a
`StoreOperator` only
to trusted embedding code. It checks the exact Program-catalog instance, Store epoch, immutable
cross-product identities, and complete association of every published entry's typed request/config/
root ABI, entry profile, and planner implementation before returning the shareable process. Future
configuration-derived Program refs do not exist at composition and are checked when that planner
finishes its typed Program. Composition does not reopen or
revalidate RuntimeAssembly callbacks/bindings; `RuntimeAssemblyBuilder::finish` already owns that
proposition. No second constructor or raw registry/port injection path exists.
`for_tenant` does not requalify those process-global facts; it only fixes domain partitioning. If a
deployment needs multiple tenants, it constructs separate facades over the same immutable `Arc`.
MFM never selects a facade from request credentials or headers.

Delete `drive_once` from the core and App public APIs. CLI/REST can offer a one-action transport by
calling `resume_run`, consuming `RunExecutor::drive`, rendering the result, and dropping a returned
executor. Document that this deliberately performs a cold resume per request. A background worker
or embedding that needs efficient continuation owns `RunExecutor` across steps.

### 7.2 Complete application access-control deletion

Delete, with no replacement shim:

- `crates/app/src/access.rs`;
- `SecretCredential`, `MAX_SECRET_CREDENTIAL_BYTES`, `ApplicationAccessPolicy`,
  `ApplicationAccessGrant`, app `AccessTarget`, `AuthorizedTenant`, `AccessPolicyError`;
- the `Application.policy` field and constructor parameter;
- every `authorize_run`, `run_grant`, `AuthorizedRunCall`, `AuthorizedAdmissionCall`, principal,
  decision ref, recursive source-policy callback, and policy-bearing backend signature;
- public `AuthenticationRequired`, `GrantDenied`, and `SourceRunExportDenied` errors and
  `ErrorClass::{Unauthorized, Forbidden}`. The repository reference audit found no non-policy
  owner;
- CLI `--access-token-file`, `support/access.rs`, credential reads, help text, JSON fixtures, and
  tests;
- REST bearer parsing, `Authorization` header extraction, maximum bearer size, 401/403 mappings,
  OpenAPI/README claims, and tests; and
- every credential/principal/grant/policy-decision field in fixtures, mocks, integration helpers,
  exports, and domain requests.

Do not remove PostgreSQL login roles/grants, database/keystore/provider credentials, RPC protocol
authorization headers, provider challenge/signature verification, or secret redaction. Those
protect actual external protocol boundaries.

Deployment documentation must state the exposure precondition verbatim: the embedding admits
callers and controls network/process reachability; the credential-free REST server is unsafe for
direct untrusted/public exposure. Adding public exposure is a new access-control product design,
not a reason to leave dead policy types.

### 7.3 Portable export vNext

Portable export becomes a structural, same-tenant, callback-free bundle. Introduce one strict v5
wire and reject v4. Store owns the already-qualified, data-only `ExportClosure`; Replay owns only
the v5 framing/encoding/decoding and deterministic replay projection:

```rust
pub struct ExportClosure {
    // Private root/source complete frames and exact structural fixations/routes.
}

pub struct ExportRunFixation {
    // Private run/tenant/store/epoch, exact journal/semantic heads, complete frame bytes,
    // and direct source ids. Data-only and non-Clone.
}

pub struct ExportFactRoute {
    // Private dense frontier, exact producer record, and exact producer head addresses.
}

impl ExportRunEvidence {
    #[doc(hidden)]
    pub fn into_export_closure(self) -> ExportClosure;
}

impl ExportClosure {
    pub fn root(&self) -> &ExportRunFixation;
    pub fn sources(&self) -> &[ExportRunFixation];
    pub fn fact_frontiers(&self) -> &[TenantFactFrontier];
    pub fn fact_routes(&self) -> &[ExportFactRoute];
}

impl ExportRunFixation {
    pub fn run_id(&self) -> &RunId;
    pub fn tenant_scope_id(&self) -> &TenantScopeId;
    pub fn store_scope_id(&self) -> &StoreScopeId;
    pub fn store_epoch(&self) -> StoreEpoch;
    pub fn journal_head(&self) -> &JournalHead;
    pub fn semantic_head(&self) -> &SemanticHead;
    pub fn batch_frames(&self) -> impl Iterator<Item = &[u8]>;
    pub fn direct_source_run_ids(&self) -> &[RunId];
}

impl ExportFactRoute {
    pub fn frontier(&self) -> &TenantFactFrontier;
    pub fn producer_record(&self) -> &RecordRef;
    pub fn producer_head(&self) -> &JournalHead;
}

pub struct PortableRunExport {
    // Private decoded v5 header + exact structural closure + frames + terminal fixation.
}

#[derive(Clone, Debug)]
pub struct PortableExportLimits {
    // Private non-zero values, each capped by its frozen absolute maximum.
}

impl PortableExportLimits {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        max_stream_bytes: NonZeroU64,
        max_record_bytes: NonZeroU64,
        max_records: NonZeroU64,
        max_batches: NonZeroU64,
        max_source_runs: NonZeroU64,
        max_fact_routes: NonZeroU64,
        max_closure_depth: NonZeroU32,
    ) -> Result<Self, PortableLimitError>;

    // One const borrowed/value getter for each limit.
}

pub struct EncodedPortableRunExport {
    bytes: Box<[u8]>,
    content_ref: ContentRef,
}

impl EncodedPortableRunExport {
    pub fn as_bytes(&self) -> &[u8];
    pub fn content_ref(&self) -> &ContentRef;
    pub fn into_bytes(self) -> Box<[u8]>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PortableExportError {
    #[error("portable export evidence is inconsistent")]
    InvalidEvidence,
    #[error("portable export exceeds frozen bounds")]
    TooLarge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PortableQualificationError {
    #[error("portable stream exceeds frozen bounds")]
    TooLarge,
    #[error("portable stream content reference does not match")]
    StreamRefMismatch,
    #[error("portable stream is not strict v5")]
    InvalidWire,
    #[error("portable history is invalid")]
    InvalidHistory,
}

impl PortableRunExport {
    pub fn encode_v5(
        evidence: ExportRunEvidence,
        limits: &PortableExportLimits,
    ) -> Result<EncodedPortableRunExport, PortableExportError>;

    pub async fn qualify_offline_v5(
        bytes: &[u8],
        expected_stream_ref: &ContentRef,
        portable_limits: &PortableExportLimits,
        store_limits: &StoreWorkLimits,
        catalog: &ProgramCatalog,
    ) -> Result<StructuredReplayResult, PortableQualificationError>;
}

// mfm-store: the one callback-free semantic ingress used by the method above.
pub struct RawOfflineFactRoute {
    frontier: TenantFactFrontier,
    producer_transition: RecordRef,
    producer_head: JournalHead,
}

impl RawOfflineFactRoute {
    pub fn try_from_parts(
        frontier: TenantFactFrontier,
        producer_transition: RecordRef,
        producer_head: JournalHead,
    ) -> Result<Self, RawBackendDtoError>;

    pub fn frontier(&self) -> &TenantFactFrontier;
    pub fn producer_transition(&self) -> &RecordRef;
    pub fn producer_head(&self) -> &JournalHead;
}

pub struct OfflineHistoryClosure {
    // Private root RawRunPrefix, bounded source prefixes, and exact raw fact routes.
}

impl OfflineHistoryClosure {
    pub fn try_from_parts(
        root: RawRunPrefix,
        sources: Vec<RawRunPrefix>,
        fact_routes: Vec<RawOfflineFactRoute>,
        limits: &StoreWorkLimits,
    ) -> Result<Self, RawBackendDtoError>;
}

pub async fn qualify_offline_history_closure(
    closure: OfflineHistoryClosure,
    catalog: &ProgramCatalog,
    limits: &StoreWorkLimits,
) -> Result<QualifiedRun, HistoryIngressError>;
```

`PortableRunExport::encode_v5` consumes `ExportRunEvidence`; there is no `&ExportRunEvidence`
encoder and no reusable borrowed encoder view. The crate seam is one consuming
`ExportRunEvidence::into_export_closure(self) -> ExportClosure` transfer. `ExportClosure` has private
fields, no constructor, no `Clone`, and only the owned/borrowed structural accessors needed by the
Replay encoder. It contains canonical complete frames and addresses, never `QualifiedRun`, a
Runtime brand, callback, credential, principal, or policy result. This explicit data transfer
replaces `ExportEncoderConsumerSeal`; it is not represented as an authority marker.

The limit owner has checked construction/Deserialize that rejects zeroes, values above frozen
absolute maxima, and inconsistent subordinate limits; the public-field sketch above is descriptive,
not permission for unchecked struct literals. `RawOfflineFactRoute` and `OfflineHistoryClosure`
constructors enforce only mechanical coordinate/count/byte bounds; only the Store qualifier assigns
semantic meaning. Encoding walks the consumed closure once, enforces
the bounds before every allocation/write, emits deterministic canonical JSON Lines, and derives the
returned `ContentRef` from those exact bytes in the same pass. Freeze
`application/vnd.mfm.structured-run-export-stream.v5`,
`mfm.structured-portable-run-export-stream.v5`, every v5 record schema id, and the terminal
fixation domain; do not reuse a v4 schema/domain literal.

`qualify_offline_v5` first checks the total byte bound, derives the v5 stream `ContentRef` from the
exact supplied bytes, and compares it with `expected_stream_ref` **before parsing a record**. It then
strictly decodes one bounded record at a time, rejects unknown/duplicate fields, noncanonical JSON,
trailing material, wrong media/schema/version, and every count/frame/depth overflow, and converts
the complete bundle into `OfflineHistoryClosure`. Replay performs no semantic verification: it
calls Store's `qualify_offline_history_closure`, which uses the same Program ingress, full-frame
ingress, reducer, binder, dense-fact closure checks, and request-bound fact-response validation as
live demand qualification. The Store qualifier retains one call-scoped `QualifiedPrefixSet`, never
loads missing evidence from a live backend, returns only the qualified root `QualifiedRun`, and
Replay borrows that value to produce `StructuredReplayResult` before dropping it.

Delete `AuthorizedExportClosure`, `PortableAuthorizationDecision`, `PORTABLE_EXPORT_GRANT`,
`export_decisions`, authenticated principal, root/source decisions,
`validate_export_decisions`, and `authorized_closure_digest`. The complete stream `ContentRef`
already fixes the exact bundle; no independent consumer owns the old policy-derived digest.

Rename `authorized_sources`, `authorized_source_prefixes`, and `with_authorized_sources` to
`sources`, `source_prefixes`, and `with_source_closure`. Preserve strict bounds, reachability,
acyclicity, exact source cutoffs/heads, tenant/store/epoch equality, fact closure, content
fixations, and rejection of surplus/omitted/substituted sources. A cross-tenant closure is invalid
export/offline-qualification structure, not denial.

Bump media type, stream/record schema ids, terminal seal literal, and every changed digest domain.
There is no v4 enum variant, decoder, migration, or compatibility test. Replace policy-decision
tamper tests with structural omission/substitution/cycle/fixation/tamper tests.

This is offline qualification, not persistent import. Do not add `Application::import*`, a Store
writer/import command, an import database table, CLI command, REST route, OpenAPI operation, or an
`Import*` public DTO/error. The only production entry point is the bounded callback-free method
above; it neither appends nor mutates Store state. A future persistence feature would require its
own product contract and is outside this RFC.

Replacement tests cover consuming evidence exactly once, byte-for-byte deterministic v5 output,
the returned whole-stream ref, byte/record/frame/batch/source/route/depth bounds, and v4/media/
schema/domain rejection. Offline tests assert a wrong expected ref fails before the first decode or
Store fold, every supplied frame enters once, the root and every reachable source qualify once,
omitted/surplus/substituted/reordered/cyclic/cross-tenant/cross-store/cross-epoch routes fail, no
live backend read or append occurs, and the result equals replay projected from the same live
`QualifiedRun`. Compile/source-surface tests prove `ExportRunEvidence` is non-Clone, no borrowed
encoder view/trust snapshot/verifier survives, and App/CLI/REST/OpenAPI expose no import.

### 7.4 EVM submission identity

The transport request contains a bounded non-secret idempotency key, not a principal:

```rust
pub struct SubmissionIdempotencyKey(Box<str>); // private field

impl SubmissionIdempotencyKey {
    pub const MAX_UTF8_BYTES: usize = 256;
    pub fn new(value: impl Into<Box<str>>) -> Result<Self, SubmissionIdempotencyKeyError>;
    pub fn as_str(&self) -> &str;
}

pub struct EvmSubmissionRequest {
    tenant_scope_id: TenantScopeId,       // injected by the facade
    wallet_nonce_domain: WalletNonceDomain,
    idempotency_key: SubmissionIdempotencyKey,
    // Existing transaction semantics, all private/invariant-safe.
}

pub fn derive_submission_intent_id(
    tenant: &TenantScopeId,
    nonce_domain: &WalletNonceDomain,
    key: &SubmissionIdempotencyKey,
) -> SubmissionIntentId;
```

The vNext preimage is a domain-separated, canonical, float-free structure containing exactly those
three identity fields. `new` accepts valid UTF-8 that is nonempty, at most 256 bytes, and contains
no Unicode whitespace; checked `Deserialize` and the MFM schema delegate to that constructor. Freeze
a new schema/domain and use the same constructor in CLI/REST. Rename `EvmCallerSubmissionToken` to
`SubmissionIdempotencyKey`. Delete `EVM_CALLER_SUBMISSION_TOKEN_MAX_BYTES`,
`caller_submission_token`, `caller-submission-token`, schema
`mfm.evm.caller_submission_token`, its validator/getter, and CLI flag/help/JSON spellings, plus
`AuthenticatedIntentIssuerId`, `IntentIssuerPreimage`,
`derive_authenticated_intent_issuer_id`, `issuer_namespace_contract_ref`,
`authenticated_principal_id`, `from_authorized`, and the `mfm.evm.intent-issuer.v1` domain. Bump the
submission-intent and submission-semantics domains plus every affected MFM value schema, nested
state contract, Program ref, request/value ref, reservation key, candidate key, completion key, and
golden vector. There is no old decoder.

The public `AdmitRunRequest` does not accept tenant; Application injects its fixed scope when it
constructs the domain request. The same tenant + wallet nonce domain + idempotency key converges
across retries/runs; a different tenant does not. The key is ordinary request identity and must not
be documented or handled as a secret credential.

Activate the changed identity only under fresh run-store and wallet-domain identities. U5 is a hard
deployment gate: use a fresh sender unless the auditable drain/terminal/reconciled-nonce/exclusive-
control proof exists. Old JSON wallet rows and journals are not reinterpreted or rewritten.

The fresh wire cut also collapses wallet completion/recovery shadows. `CompletedWalletNonce` owns
one private typed terminal/recovery closure with checked constructors; `CanonicalTerminalOutcome`
projects the public result on demand from that typed value. Delete editable
`recovery_closure: String`, duplicated `canonical_public_result`, and every decode-mutate-reencode-
revalidate cycle around them. Persist one canonical projection of the typed owner at the database
boundary and decode it once on cold ingress. Migrate live EVM and EVM-PostgreSQL consumers and
replace string-mutation tests with typed constructor, canonical projection, and tamper/golden tests.

## 8. Complete codebase cutover and deletion inventory

This inventory is a minimum, not a whitelist. Run the negative source/API/schema scans in §10 after
each vertical move and delete newly discovered shadows. A current name may be retained only when a
named target owner and invariant below requires it.

### 8.1 Workspace, packages, and features

| Current surface | Target action |
| --- | --- |
| `crates/kernel/spec` / `mfm-spec` | Move the one normalized strict document model into `mfm-program`; delete package, member, docs, tests, tasks, and dependencies. |
| `crates/kernel/certify` / `mfm-certify` | Move pure compiler work to Program and live assembly work to Runtime; delete package, member, docs, UI suite, tasks, features, and dependencies. |
| `crates/kernel/authority-seal` / `mfm-authority-seal` | Replace real uses with opaque owner products/private constructors; delete all public marker traits, package, member, tasks, and edges. |
| `mfm-runtime/store-authority` and `mfm-certify/runtime-authority` | Delete features and every `#[cfg]` constructor/test. No successor feature. |
| `mfm-store -> mfm-runtime` | Remove. Move callback-free history commands/cursor/errors/identity to Store. |
| `mfm-runtime -> mfm-store` | Add as the one direction for concrete qualified evidence/prepared appends. |
| optional `mfm-program` in Store | Make non-optional; Store always resolves exact Programs during ingress. |
| Store/Runtime mixed integration tests | Move live callback/session tests to Runtime; leave Store callback-free qualification/backend tests. |

Update root `Cargo.toml`, every affected crate/dev manifest, `Cargo.lock`, metadata/layer assertions,
`nixfied.nix`, generated task inventory, deny/source manifests, and crate READMEs in the same
cutover. No deleted package may appear even as a dev-dependency.

Delete `mfm-authority-seal` only after mapping every current marker, not by assuming every use is
revocation:

| Current marker | Final owner/replacement |
| --- | --- |
| `RuntimeHistoryPortSeal` | opaque `QualifiedHistoryPort` plus `ActiveQualifiedRun`; no marker |
| `PhysicalBindingVerifierSeal` | U6 immutable descriptor/assembly ingress, or deletion when it expressed currentness only |
| `RetainedPhysicalReleaseTrustSeal` | delete with physical release/currentness |
| `StoreLineageTrustSeal` | strict Store/import structural ingress products |
| `ExportEncoderConsumerSeal` | delete; consuming `ExportRunEvidence::into_export_closure -> PortableRunExport::encode_v5` data transfer, no marker |
| `WalletNonceAuthoritySeal` | concrete wallet-domain transaction SPI captured inside the immutable EVM adapter |
| `DeploymentCredentialSinkSeal` | opaque PostgreSQL credential-admission product; preserve secret handling, no empty marker |
| `DeploymentCredentialBrokerSeal` | same concrete credential broker/sink composition |
| `RuntimeAssemblyConsumerSeal` | `ProcessComposition::qualify` |
| `ValidatedAppendConsumerSeal` | owner-bound `PreparedAppend`/`BackendAppendCommand` borrowed getters; configuration equivalent lands in the same infrastructure cut |

The reference/invariant inventory in U6 records the concrete consumer replacing each surviving
seam. A public implementable empty trait never becomes evidence merely because its name contains
“seal.”

### 8.2 Program, Spec, Certify, and Capabilities

In `crates/kernel/program/src/structured.rs`:

- replace `OperationBuilder::finish -> AuthoredStructuredProgram` with the scoped
  `ProgramCatalog::author_candidate -> ProgramCandidate` path, followed by
  `ProgramCatalog::finish -> TypedProgram<Input, Output, Failure>`;
- delete `Execution::access_type_ids`;
- delete `RuntimeReadCapability`, `RuntimeEffectCapability`, `RuntimeEffectRefreshBinding`,
  `NoRefreshBinding`, `RefreshableBinding`, and generic entry-binding repair wrappers where
  `EffectEntryModeFor<Request>` can derive the relation directly;
- delete `RuntimeReadAdapter`, `RuntimeEffectAdapter`, `RuntimeSigner`, and
  `RuntimeResourceAuthority` public invocation/resource traits;
- rename `CapabilityExpansion`, authoring `Capability`, `Direct`, and `RequiresCapability` to the
  chosen lowering vocabulary;
- delete `State::{Request, Returned, SafeFailure}` and any duplicate request ABI bounds;
- delete `StateFrame`; its only field is `&S::Input`, and the target callback signature carries that
  exact borrow without another authority wrapper;
- delete public `StructuredStateCallbacks` and wrong-variant methods; and
- keep the authoring DSL, failure routes, Match, FanOut, child operation expansion, injection, and
  stable semantic paths, retargeted to private IR.

From `crates/kernel/spec/src/structured.rs`, move only normalized graph/document DTOs, strict
codecs, frozen bounds/profile, and secret-free manifest identities into private/public data modules
of Program. Delete public `Authored*`, `Expanded*`, `CertifiedProgram*`, expansion/policy proof
objects, `StructuredEffectRefreshContract`, generic Resource component/schema fields, old object
tags, and every decoder for the three-authority format.

From `crates/kernel/certify/src/structured.rs`, move:

- expansion/lowering, normalization, profile/graph checks, manifest/document construction,
  schema/lexical indexes → `mfm-program`; and
- typed callback erasure, process maps, binding capture, panic/fault conversion → `mfm-runtime`.

Then delete at least:

- `CertifiedProgram`, `ProgramRegistryBuilder`, `CertifiedProgramRegistry`;
- `AdmissionCertificationRegistry`, `AdmissionVerificationRegistry`, `EntryPointCertifier`,
  `AdmissionVerifier`;
- `CertifiedProcessRegistry`, `RuntimeAssemblyToken`, `QualifiedComponentIdentity` proof wrappers;
- `QualifiedPhysicalBinding<K>`, Read/Effect aliases/sources, `AccessTargetSelection`,
  `PhysicalBindingSelection`, `ErasedBoundAccessInvocation`, bound invocation wrappers;
- `CertifiedAccessAuthorization`, `ExpectedAuthorization`, and prior proof snapshots;
- process `Signer`/`Resource` handles and all current-binding/release/refresh selection; and
- the old certify trybuild suite. Recreate only still-meaningful nonforgeability tests in Program,
  Store, or Runtime.

In `crates/kernel/capabilities/src/lib.rs`, apply the §3.1 deletion list. Also remove
`ComponentFuture`, `ReadAdapterCompletion`, `EffectAdapterCompletion`,
`EffectContractCompletion`, `ReadCapabilityImplementation`, `EffectCapabilityImplementation`,
`ReadAdapterInvoker`, `EffectAdapterInvoker`, `BoundedComponentContract`,
`BoundedComponentInvoker`, `SignerContract`, `ResourceAuthorityContract`, generic signer/resource
contract plumbing, and the corresponding tuple/role UI fixtures. If
`crates/signing` still declares a capability type whose only purpose was nested callable signing,
delete it; concrete EVM adapters own signing internally.

### 8.3 Journal, Store, Runtime, and Replay

In `crates/kernel/runtime/src/history`:

- move surviving callback-free proposals, `ActionableState`, `ProgramCursor`,
  `StructuredFrontier`, `EffectEntrySubject`, Store identities, and Store error categories to
  `mfm-store`;
- delete `RuntimeHistoryPort`, `VerifiedRunView`, `HistoryAppendOutcome`,
  `StructuredAppendAttempt`, `CommittedAccessAuthorization`, proof re-exports, `port.rs`,
  `proofs.rs`, and obsolete identity/command wrappers; and
- replace authorization proposal names with the typed Store `ResolvedEvent` constructors.

In `crates/kernel/runtime/src/structured.rs`:

- replace `Runtime<P>`, `admit_run`, `drive_once`, pre-drive load, `Prepared<K,V>`,
  `Authorized<K,V>`, `InvokedObservation<K,V>`, `authorize`, `invoke_authorized`, and
  `commit_invoked_observation` with §5;
- delete post-reservation `load_verified` and every field-by-field `ExpectedAuthorization`
  comparison;
- replace `RuntimeProcessRegistry` with private immutable `ProcessRegistry` plus one resolved
  dispatch map per session;
- preserve deterministic action selection, panic containment, redacted error attribution,
  Effect ambiguity/absorption, and settlement semantics; and
- ensure no production Cargo feature exports an invocation constructor.

In `crates/kernel/store/src/structured`:

| File/surface | Target |
| --- | --- |
| `adapter.rs` | Delete; it exists to implement the old Runtime-owned port. |
| `reducer.rs` | One `ResolvedEventData`/`apply`; rename Reservation entry; delete intent/recorded branches, refresh/supersession, `semantic_eq`. |
| `compiler.rs` | Rewrite/rename to `binder.rs`; deterministic assignment/projection only. Delete preview/comparison types. |
| `qualification.rs` | Complete-prefix and found-attempt ingress using `ProgramCatalog`; delete hot successor qualification and all physical currentness checkers. |
| `obligations.rs` | Delete after prior-run fact requirements become explicit reducer/binder data and immutable binding checks move to Program/Runtime assembly. |
| `coordinator.rs` | Consuming `PreparedAppend` commit/resolution; delete prelookup, local requalification, second reduction, echo comparison, stale reload. |
| `validated_append.rs` | Delete; replace with private inseparable `PreparedAppend` and opaque mechanical `BackendAppendCommand`. |
| `backend.rs` | Raw bounded frames, borrowed payloadless compare-and-append, typed changing-precondition errors, audit snapshot SPI. |
| `semantic_open.rs` | Infrastructure-only open; demand qualifier and explicit audit. Delete ordinary eager sweep. |
| `purpose.rs` | Project from borrowed `QualifiedRun`; emit consuming `ExportRunEvidence`/`ExportClosure`; delete recorded/offline verifier evidence and borrowed encoder views. |
| `fact_scan.rs` | Scope memo to ephemeral `QualifiedPrefixSet`; rename fact-scan continuation; no global cache. |
| `configuration.rs` | §6.4 API; no hidden retry loop, reload, echo, or eager scan. |
| `mod.rs` | Rename `VerifiedStructuredRun` to opaque `QualifiedRun`; remove authorization/currentness getters. |

Explicitly delete `ProgramVerificationRegistry`, `PhysicalBindingAuthorization`,
`PhysicalBindingSupersession`, `PhysicalObligationChecker::{verify_retained_authorization,
verify_current_authorization}`, `SemanticObligation::{PhysicalAuthorization,
PhysicalSupersession}`, `ObligationDischargeScope::{RetainedOnly,RetainedAndCurrent}` where no
nonphysical owner remains, `QualifiedRecordedEvent`, `QualifiedIntentEvent`,
`PendingSemanticStep`, `ComparisonPassed`, `ComparedReduction`, and `FinalizedReduction` wrappers.
Also delete `OfflineRunClosure`, `OfflineVerifiedRun`, `RecordedRunEvidence`, `ReplayRunReader`,
`load_for_recorded_verify`, `ExportEncoderView`, `ExportEncoderSource`, `with_encoder_view`, and
`verify_offline_run_closure`. Their replacements are, respectively, bounded
`OfflineHistoryClosure`, Store's one `qualify_offline_history_closure -> QualifiedRun`, borrowed
replay projection from `QualifiedRun`, and the consuming `ExportRunEvidence -> ExportClosure -> v5`
path. Do not retain a purpose-specific semantic verifier under a new name.

In `crates/kernel/replay`:

- consume Store `QualifiedRun`/purpose projections; delete any independent program/history
  verifier and every `mfm-certify` edge;
- retain replay-result strict decoding and deterministic projection, not history authority;
- make `PortableRunExport::encode_v5` consume `ExportRunEvidence`, and delete
  `ReplayEncoderConsumer`, `ReplayTrustSnapshot`, `RetainedPhysicalReleaseTrust`,
  `StoreLineageTrust`, `PortableRunExport::{verify_offline,verify_with_trust}`, and every callback/
  trust-snapshot encoder or verifier path;
- feed bounded offline v5 frames/source prefixes through Store's same ingress/fold and use one
  qualification-call-scoped producer memo only; and
- never fill an omitted bundle dependency from live Store evidence or expose a persistence/import
  operation.

### 8.4 Application, CLI, REST, portable, and EVM

Apply §7.2 to `crates/app/src/access.rs`, `lib.rs`, `application.rs`, `errors.rs`,
`production_structured.rs`, all app tests/helpers, `bin/cli`, and `bin/rest-api`.

In `crates/kernel/replay/src/portable.rs`, delete the current policy/trust/view/verifier model and
introduce the evidence-consuming v5 encode plus bounded offline qualification API in §7.3. In Store
purpose/test support rename every `authorized_*source*` helper to structural closure terminology.
Delete any App/CLI/REST/OpenAPI “import” route, command, DTO, or error discovered during the cut;
none has a target replacement.

In `crates/domains/evm/src/submission.rs`, `wallet_authority.rs`,
`submission_process.rs`, App injection, EVM PostgreSQL JSON values, tests, and goldens, apply §7.4.
Delete exact old names `EvmCallerSubmissionToken`, `AuthenticatedIntentIssuerId`,
`IntentIssuerPreimage`, `derive_authenticated_intent_issuer_id`,
`issuer_namespace_contract_ref`, `authenticated_principal_id`, and `from_authorized`.

### 8.5 Generic live-currentness deletion

Delete these families vertically, after U6 maps immutable survivors:

- Capabilities: `NoRefreshEvidence`, refresh modes/bindings/evidence, Resource authority, and
  `SupersededBeforeEntry`;
- Program/old Spec: `RuntimeEffectCapability::RefreshBinding`,
  `RuntimeEffectRefreshBinding`, `NoRefreshBinding`, `RefreshableBinding`,
  `RuntimeResourceAuthority`, `StructuredEffectRefreshContract`, and resource-only manifests;
- Certify/Store: dynamic `current_binding`, supersession head, minimum lineage, retained/current
  physical authorization, `PhysicalBindingAuthorization`,
  `PhysicalObligationChecker::{verify_retained_authorization, verify_current_authorization}`,
  `SemanticObligation::PhysicalAuthorization`, `RetainedPhysicalReleaseTrust`, physical obligation
  checker, release trust, and current-release lookup;
- Runtime/Journal: refreshable cursor leaf, stable-resource/minimum-lineage fields, superseded
  outcome, release certificates, and every associated hash/preimage field;
- live EVM: `physical_release.rs`, release histories, current-release selectors, per-access consume
  permits, `EvmBroadcastResource`, `WalletNonceAuthorityResource`, and no-op resource invokers;
- Signing/Keystore: `GenerationGuardedDeterministicSigningProvider`,
  `SigningGenerationGuard`, `SigningGenerationGuardFuture`,
  `VerifiedGenerationGuardedSignerBinding`, `QualifiedReadSigningProvider`,
  `GenerationGuardedSignerDescriptor`, `sign_guarded`, `verify_current_and_exclusive`,
  `durable_generation_ref`, `fence_attestation_ref`, `direct_sign_exclusion_ref`, the
  `mfm.signing.generation-guarded-signer-descriptor` schema, durable-generation/fence/currentness
  wrappers, per-sign generation reads, and `SigningGenerationGuardError`/related tests. Preserve
  immutable key identity, secret qualification, signing
  algorithms, AAD, zeroization, and deterministic/tamper tests; and
- generic PostgreSQL target authority/currentness as specified in §4.5.

EVM routing follows the same U6 split. Retain one strict immutable route/catalog descriptor and
provider challenge/signature evidence only where it authenticates bytes at assembly ingress.
Delete append-only “current routing catalog” histories/heads/promotions,
`EvmRoutingGenerationDescriptor`, current-generation selectors,
`provider_fence_head_ref`, deployment assembly lease/current-public-lineage fields,
`current_public_lineage_head`, signer generation/fence/direct-sign fields, and release-history
checks in `domains/evm/chain_registry.rs`, `storages/evm-postgres/provider.rs`, and
`tests/wallet-authority-provider`. Rewrite `DeploymentAssemblyBinding` to exact immutable refs.
The provider inventory challenge/finish authorization protocol survives because it authenticates
external protocol bytes; it does not become live MFM currentness.

Do **not** mechanically delete every type called lease, fence, permit, or authorization. Preserve:

- Store epoch and exact-head/fact-frontier transaction preconditions;
- EVM PostgreSQL transaction/reservation/snapshot leases that scope a DB lock or atomic operation;
- wallet nonce locks, permanent operation keys, and provider/domain idempotency;
- provider RPC request authentication, inventory challenge/signature/finish proofs that establish
  byte-source authenticity; and
- database roles/grants and credentials.

Rename a preserved coordination type only when its old name falsely implies the deleted app policy;
do not blur real protocol authentication.

### 8.6 PostgreSQL baseline and test backend

Implement §4.4–§4.5 in:

- `crates/storages/postgres/src/{structured,configuration,session,schema,transaction,roles,sql_catalog}.rs`;
- `crates/storages/postgres/migrations/0001_store.sql`;
- Store's memory backend; and
- backend conformance suites.

Memory and PostgreSQL must have identical disposition semantics and bounds. Neither returns a
positive payload echo. Both return exact stored raw bytes for duplicate append identity. Both keep
unknown acknowledgement injectable in tests without claiming commit/absence. PostgreSQL alone
qualifies its channel, schema, roles, epoch, and durability.

### 8.7 Contract and documentation rewrite

Update documentation with the code that changes each owner:

- `docs/design.md`: make the RFC's Program, byte ingress, journal, Store, Runtime session,
  reservation/observation, PostgreSQL, facts, configuration, tenant, and recovery invariants the
  authoritative current contract; delete the former auth/currentness/dual-proof text;
- `docs/architecture.md`: replace the crate graph and placement taxonomy, including deletion of
  Spec/Certify/Authority-seal and the Store→Runtime inversion;
- `docs/run-execution.md`: show spawn/resume/session, prepared commit, access reservation,
  observation rebase/ambiguity, and Effect recovery;
- `docs/known-gaps.md`: demand-time corruption discovery, no cross-restart rollback witness,
  manual `EntryOnce`, cold-resume SLO ruling, dormant binding behavior, and provider factual trust;
- persisted/public-surface documentation: new Program, journal, portable v5, EVM, configuration,
  and PostgreSQL identities with explicit rejection of old bytes;
- root and all affected crate/binary READMEs: current APIs only, tenant-scoped trusted exposure,
  transport one-shot cost, no credentials; and
- `docs/build-and-verification.md`/Nix task inventory only where package/task ids actually change.

The superseded `rfc_single_trust_boundary.md` is deleted with this accepted RFC/plan handoff; Git
history is the archival copy. The final `docs/design.md` and `docs/architecture.md` must cite the
accepted RFC as the decision record but must stand alone as the current product contract.

## 9. Ordered implementation and logical commits

The sequence below is mandatory unless a new architect review demonstrates a smaller coherent
cut. Subjects are lower case. Every commit includes its code, contracts, schemas, fixtures, and
replacement tests. Commits 1–6 are one intentionally non-deployable migration train: do not activate
the fresh production Store/wallet identities until Commit 6 and the final gates pass. Each checkout
still has exactly one active internal design and must compile under its scoped verification; no
commit may carry parallel old/new public APIs, decoders, reducers, or runtime protocols.

### Commit 0 — `resolve single-ingress implementation gates`

This is an architecture/deployment decision commit and must land before implementation.

Required outputs:

1. record the cutover-base commit id and owner rulings for U1–U11 directly in the RFC and this plan;
   remove conditional target
   branches after a ruling;
2. record the maximum batch frame, maximum run prefix, maximum configuration revisions/bytes,
   active-session retention contract, and cold-resume/load SLO or explicit no-SLO;
3. add the production-capability provider/evidence table required by U4;
4. add the checked physical certificate/consumer disposition table required by U6, including the
   final `ExecutionBindingDescriptor`, provider protocol-auth evidence, and signer
   `key_instance_ref` mapping;
5. record the dormant-run deployment policy and complete access-bracket drain procedure;
6. record EVM fresh schema/store/wallet/sender activation or the signed sender-reuse proof
   requirements;
7. confirm `PrimaryCrashRestart` or specify/qualify the stronger exact database topology;
8. run and discard a target-faithful typed-value/affine-API compile spike and a disposable full-
   frame/prefix benchmark prototype; record only the resulting API decision, provisional limits,
   harness specification, and measurements; and
9. settle U3 so subsequent APIs and schema contain exactly one attention design.

Do not add placeholder production abstractions or commit the spike. Commit 3 reruns the frame/
prefix benchmark against the real backend and may only tighten within the already published bound;
expanding a bound or missing its SLO requires an owner/RFC amendment.

Coherence condition: after this commit the plan has one implementable target and no unresolved
owner choice affecting a public type, wire, schema, or recovery behavior.

Verification: validate changed links/claims and recorded disposable-spike output, then
`git diff --check`. This docs/decision commit does not retain or gate a production Rust prototype.

### Commit 1 — `scope application facades by tenant`

Make the entire policy/persistence cut atomically:

1. introduce fixed-tenant `Application`, credential-free backend calls, and partition-first Store/
   PostgreSQL lookup;
2. remove App policy/credential/principal/grant types, typestates, errors, tests, and constructors;
3. remove CLI token-file support and REST bearer/401/403 behavior; document trusted exposure;
4. replace portable authorization closure with strict structural v5 and delete the v4 decoder;
5. cut EVM request/intent identity to tenant + nonce domain + `SubmissionIdempotencyKey`, including
   every nested schema/hash/golden and wallet row;
6. stage and validate the fresh run-store/wallet/sender identity plan without production
   activation; and
7. update App/CLI/REST/Replay/EVM/EVM-PostgreSQL docs and public examples.

Do not rename runtime journal coordination in this commit unless required by the persisted EVM or
portable wire; it is removed atomically with its replacement in Commit 4. The result is still a
coherent system: “authorization” there means the old journal reservation protocol only, while the
application access-control model is wholly absent.

Replacement tests:

- fixed facade tenant cannot be overridden through any public call/header/CLI argument;
- two facades sharing a backend cannot read/drive/replay/trace/audit/export each other's run;
- out-of-partition lookup is NotFound and returned mismatched admission bytes are InvalidHistory;
- REST works without Authorization and cannot choose a tenant;
- CLI help/JSON contains no token option or secret;
- portable v5 consuming encode and same-tenant offline qualification pass; wrong whole-stream ref,
  bounds, tampering/omission/cycle/cross-tenant/old v4 fail before projection as applicable, and no
  App/CLI/REST/OpenAPI persistence import exists;
- no principal/grant/decision field exists in emitted bytes; and
- EVM same-scope idempotency converges, different tenants diverge, old issuer bytes fail, and the
  staged activation preconditions are exercised without production activation.

Focused verification in the Nix shell: format; check/test `mfm-app`, `mfm-replay`, `mfm-evm`,
`mfm-evm-postgres`, CLI, REST, and affected integration targets; run the affected DB-backed tests
and schema/golden checks. Run `cargo-metadata-contract` if manifests change in this commit.

### Commit 2 — `make program and state semantics valid by construction`

Land the final callback-free semantic ABI and compiler together; neither can compile against a
temporary form of the other:

1. move `ProgramRef`, `ProgramCatalogFingerprint`, `ExecutionBindingRef`, `RecordRef`,
   `JournalHead`, `TenantFactFrontier`, `TypedValueRef`, and their contained address/digest newtypes
   to `mfm-ids`; simplify capability contracts including safe-failure, fact mode, and non-zero keyed
   Effect entry modes; and migrate every domain declaration without changing hash preimages;
2. replace `State` request duplication with `Execution`/`Lowering`, typed expansion scopes, final
   root ProgramCandidate path, and mode-indexed State implementations;
3. complete the valid-by-representation value inventory and checked-Deserialize cut, including
   `QualifiedValue`/typed/failure/safe-failure erased bridges and the final U8-bounded
   `ValidatedConfig<C>::{from_typed, from_source, from_retained}` constructors;
4. absorb normalized document/profile/manifest types and the sole expansion/compiler
   implementation into `mfm-program`;
5. introduce `ProgramDocument`, opaque branded `Program`, immutable callback-free catalog,
   Program/value ingress, execution-binding descriptors, and one final invariant function;
6. introduce immutable Runtime assembly/state/adapter registration against that exact catalog,
   while retaining the current journal coordination protocol until Commit 4;
7. migrate Store, Replay, domains, and the still-live physical binding portion of Certify to
   `Arc<Program>` and the final State/capability ABI;
8. delete every authored/expanded/certified executable authority, old Program decoder, and
   `mfm-spec`; and
9. shrink `mfm-certify` to one coherent live physical-binding/currentness owner only. It must not
   compile, certify, wrap, or revalidate a Program.

There is one compiler and one Program authority at this commit. Certify's temporary remaining job is
not a compatibility Program facade; Commit 4 deletes that separate live-currentness owner.

Replacement tests:

- hot finish and cold ingress yield the same ref/document/graph/index/fingerprint;
- one final invariant counter, zero hot byte-ingress calls, and zero posterior Program validation;
- hostile bounds/canonical/ref/catalog/graph substitutions fail once at ingress;
- child expansion, configuration specialization, fan-out, injected pre/proceed/post/failure states,
  tracing provenance, and resume-without-reexpansion pass;
- Program cannot be field-constructed/deserialized and ProgramDocument/Ref cannot execute; and
- wrong mode/fact/entry/expansion/value construction fails at compile/ingress, catalog brands cannot
  transpose, and `rg`/metadata prove no `mfm-spec`, old object tags, or old decoder remains; and
- Runtime assembly rejects missing, surplus, ambiguous, wrong-mode, wrong-catalog, and mismatched
  state/adapter/binding registrations, while its finished handles remain non-Clone where required.

Focused verification: format; Program/derive/canonical/value/domain/replay/store package checks and
tests; affected Program UI/golden suites; migrate the assembly cases currently owned by
`crates/kernel/certify/src/structured/process_handle_tests.rs` to the Runtime assembly suite in this
commit; rewrite `tests/integration/tests/cargo_metadata_contract.rs`; then run
`cargo-metadata-contract` and the smallest cross-crate check that covers every migrated consumer.

### Commit 3 — `make store ingress and appends one semantic path`

Cut Store mechanics and semantic ownership without yet changing the external-access wire:

1. move callback-free cursor/commands/errors/identity/reducer ownership to Store, remove
   Store→Runtime, and add Runtime→Store;
2. split read-only `QualifiedRun` from affine `ActiveQualifiedRun`; introduce selected actions,
   one `ResolvedEvent`/`apply`, `PendingAppend`, one binder, and owner-carrying prepare/commit errors;
3. delete dual Intent/Recorded reducers, semantic comparisons, local requalification, echo
   comparison, and friend feature bridges;
4. seal prepared appends/successors and cut Memory/PostgreSQL run storage to borrowed complete-frame
   New/Found/Stale/Unknown with the real U10 bound;
5. rewrite the PostgreSQL baseline run tables, delete object decomposition and `target_authority`,
   qualify `PrimaryCrashRestart`, and rerun the target full-frame/prefix benchmark;
6. land the one read-side configuration ingress now: `ConfigurationStore::load<C>`,
   `ResolvedConfiguration<C>`, and `ConfigurationAdmissionEvidence<C>` use Commit 2's strict
   `ValidatedConfig<C>` ingress and the ruled U8 total bounds; route selected reads and the
   temporarily retained readiness caller through that same qualifier, with no independent reader
   replay, cache, suffix validator, or second evidence constructor;
7. land the callback-free `PublishedEntryPointCatalog`, tenant-free
   `ConfigurationTargetSelection`, `ConfigurationPlannerRegistry` registrations,
   `EntryPlanningCall`, and the sole call-bound context/fact candidate constructors. This commit
   builds and tests planning products, but does not add the final Runtime/App spawn path;
8. migrate the configuration writer backend mechanically to its concrete owner-bound borrowed
   command and remove `ValidatedAppendConsumerSeal`/target-authority use now; retain the current
   single active writer protocol until Commit 6 rather than introducing a temporary second writer;
9. implement the complete chosen U3 attention slice—schema column/index or total deletion,
   backend command/projection, DTO/API, and atomic tests—in this commit;
10. use Commit 2's shared `mfm-ids` fact response addresses; make Facts own the checked selected-
   fact, attestation, ordinal/query-result, digest, and response types; retain dense fact authority;
   and delete Journal's duplicate response plus the embedded JSON/base64 loop; and
11. preserve the current external-access coordination record/currentness owner as the one active
   protocol only until Commit 4.

This commit is coherent because hot and retained events already use one reducer/binder and one
backend frame; only the security-misnamed access wire/live-currentness protocol remains to be
replaced. No second Store path survives.

Required replacement test groups:

- compile-fail Program/PreparedAppend/QualifiedRun-vs-ActiveQualifiedRun privacy;
- one reducer hot/cold equivalence and mutation of every retained fixation/surplus field;
- dense fact frontier/scanner binding, exact request/query/result/attestation relation and bounds,
  Facts/Journal duplicate-response negatives, and distinct changing-precondition errors;
- PostgreSQL crash/restart durability, Store epoch, raw Found, full-frame bounds, atomic head/fact/
  conditional-attention behavior, old schema rejection, and Memory/PostgreSQL conformance;
- configuration source/retained one-ingress reads, U8 overflow, exact selected evidence, foreign
  tenant/store/entry rejection before planner invocation, tenant-free target selection, candidate
  bounds/catalog privacy, and writer mechanical command parity without its affine cut; and
- real maximum-prefix/frame benchmark plus event-loop responsiveness.

Update Store/PostgreSQL/Facts/configuration mechanical contracts, architecture graph, persisted
batch surfaces, and U3 surface here.

Focused verification: iterate with package/target filters and backend conformance in the Nix shell.
Before committing, run formatting, affected package/UI/doctests, metadata contract, SQL inventory/
offline SQLx, Store backend conformance, structured-history/configuration PostgreSQL qualification,
fact tests, and model-check when its graph changed. The named configuration owners are
`crates/kernel/store/src/structured/configuration.rs` and the configuration cases in
`crates/storages/postgres/tests/structured_history.rs`. Do not substitute final `.#ci` language for
this exact scoped revision.

### Commit 4 — `reserve external access through affine run sessions`

This is the inseparable access/runtime/currentness vertical cut:

1. freeze the U6 immutable binding mapping and cut Journal/fact attestations to Program refs,
   `ExecutionBindingRef`, `ExternalAccessReserved`, reservation refs/keys/frontiers, and new strict
   preimages/goldens;
2. land `RunSession`, `SessionContinuation`, selected-action dispatch, `PreparedDrive`, exhaustive
   owner-carrying preparation/commit/rebase, `SuspendedRun`, and zero-read direct advancement;
3. land typed fact-mode adapter calls and
   `PreparedReservationAppend -> ReadyToInvoke -> AcceptedAccessResponse ->
   PreparedObservationAppend`, with exact response ingress, ambiguity, stale rebase, and
   observation-before-settlement;
4. connect Commit 3's resolved configuration and planning products to the one
   `Runtime::prepare_admission`/`AdmissionInput` bridge, then land `ProcessComposition`, the
   catalog-bound Application decoders, fixed-tenant spawn/resume, and exhaustive
   `RunExecutor`/suspension outcomes while deleting App/backend `drive_once`;
5. replace EVM/wallet/balance/portfolio/signing/keystore/provider assembly with immutable binding
   objects and preserve only protocol authentication/domain transactions;
6. delete refresh/release/revocation/currentness, physical authorization shadows, old access wire,
   `mfm-certify`, all remaining `mfm-authority-seal` markers/package, feature constructors, tasks,
   schemas, tests, and decoders; and
7. add the full path-scoped negative inventory.

The commit is not ready while an old guard is merely bypassed. Its real ordering, projection,
binding, ambiguity, or secret-boundary job must exist in the final owner or the deletion stops.

Tests cover mode/fact/entry UI, direct-new zero-read/invoke, all
non-new zero-invoke branches, two-worker races, same-C token transposition, reservation and
observation unknown/stale/rebase matrices, transient owner preservation, EntryOnce/manual and
EntryAbsorbing/slow-live recovery, dormant binding/drain behavior, signer/keystore security, and
every deleted wire/API/schema negative. Application integration additionally covers catalog-bound
decode, cross-process request rejection before configuration lookup, same-process/two-tenant
selector injection, candidate-constructor privacy/bounds, and exact owner preservation across
admission retry. Rewrite `tests/integration/tests/transport_surface_contract.rs` and the affected
`crates/app/tests/application_privacy_ui.rs` cases in this commit; neither may continue to require
`drive_once` or an auth-only facade.

Update design/architecture/run-execution, persisted surfaces, EVM routing/transactions, Runtime/
Journal/Live/Signing/Keystore docs, transport executor APIs, and deployment drain docs here.
Verification is the affected Runtime/Journal/App/EVM/live/signing/keystore UI/unit/integration/DB
set plus metadata/SQL/model gates; the final workspace CI remains Commit 6.

### Commit 5 — `qualify retained history only when consumed`

1. remove the ordinary startup/eager run and configuration readiness enumeration here—not in
   Commit 3 or Commit 6—and leave Store open/readiness infrastructure-only;
2. unify Runtime resume, public/trace/access-audit reads, Replay, export, offline portable
   qualification, explicit audit, and selected fact producers behind the one complete-prefix
   qualifier;
3. retain only request/session-scoped `QualifiedPrefixSet` reuse;
4. make `audit_store` use one combined fixed Store snapshot with bounded progress;
5. integrate the already-landed U3 listing (when enabled) through demand-time qualification; and
6. delete semantic-open eager sweep, the configuration readiness scan and its pagination helpers,
   purpose verifiers, global/suffix/cache/checkpoint remnants, and stale automatic continuation.

Tests must prove zero dormant loads at startup/unrelated spawn, malformed dormant isolation,
selected malformed failure before output/authority, exact producer closure behavior, same-scope
reuse/new-scope re-ingress, audit snapshot consistency, and the chosen attention contract.

Focused verification: Store/Runtime/Replay/App tests plus the structured-history PostgreSQL and
fact/attention targets. Name `crates/storages/postgres/src/qualification.rs`, Store qualification
tests, and readiness/startup integration tests as owners of the removed eager behavior. Update
readiness, known-gaps, reader/export, and operator-audit docs here.

### Commit 6 — `make configuration writes one semantic path`

1. add non-Clone `ConfigurationWriteSession<C>`, its prepared successor, borrowed backend append,
   owner-carrying retry, and affine unknown resolution on top of Commit 3's one resolved read;
2. have `begin_write` obtain its current value through that same qualifier, and have direct commit
   advance the retained typed successor without a reload or readback;
3. route typed/source/retained successors through Commit 2's strict `ValidatedConfig<C>`
   constructors exactly once and enforce the same U8 totals on write preparation;
4. store canonical revision frames in the fresh PostgreSQL baseline and remove duplicated
   predecessor/self-FK/reconstruction surfaces, building on Commit 3's already-owner-bound backend;
5. remove writer prefix reload, hidden retry loop, local revalidation, echo comparison, writer-only
   reconstruction, and every writer cache/suffix path; and
6. update Store/App/PostgreSQL/configuration docs, schemas, fixtures, and public examples.

Tests must cover local typed construction, one ingress for each source/retained successor, total
bounds, exact predecessor/content, direct commit zero readback, Found/conflict/stale/unknown
resolution, retry-owner preservation, writer-session non-clone/transposition, secret exclusion,
and Memory/PostgreSQL parity. Retarget
`crates/kernel/store/tests/ui/fail/validated_configuration_append_cannot_be_forged.rs` to the final
writer owner and finish the configuration cases in
`crates/storages/postgres/tests/structured_history.rs`; do not move startup/readiness assertions
out of Commit 5.

Focused verification: Values/Store/App/PostgreSQL configuration tests, SQL inventory/schema checks,
and affected DB qualification.

### Final verification and handoff

After Commit 6 and only after targeted failures are resolved:

1. run `nix run .#model-check` if the task graph changed and it has not run on the final revision;
2. run `nix run .#ci` exactly once on the final tree. Do not immediately precede it with redundant
   `.#check`, `.#test`, and `.#test-db` runs;
3. run `git diff --check <cutover-base>...HEAD` using Commit 0's recorded base, also having run
   plain `git diff --check` before each commit, and inspect `git status --short`;
4. generate the public API/dependency/LOC delta report in §11; and
5. confirm every commit subject is lower case and each commit independently checks out as one
   coherent design; and
6. only then activate the fresh production Store/wallet identities under the U5 deployment plan.

Report every command run, result, and any gate not run. Direct Cargo/Rust commands always use the
default Nix development shell.

## 10. Verification specification

### 10.1 Test-only evidence counters

Add narrow `cfg(test)` counters at the actual owners; do not add production observability state that
changes authority. Tests must be able to assert:

- Program final invariant passes, hostile Program ingress calls, and expansion calls;
- value canonicalizations, cold typed decodes, and Runtime typed downcasts;
- backend prefix/frame reads, retained batch decodes, reducer applications, and binder calls;
- post-commit head/prefix reads;
- provider invocation and response-ingress relation calls;
- prior-run producer prefix folds;
- configuration source/row ingress, revision folds, append-time history reads, and readbacks; and
- startup run/configuration enumeration.

Counters are keyed/scoped per test Store/Runtime so parallel tests do not race. They prove absence of
work only in tests; they are not runtime proofs.

### 10.2 Boundary test matrix

| Boundary | Required positive evidence | Required hostile/negative evidence |
| --- | --- | --- |
| typed Program finish | one final invariant pass; no byte ingress; exact Program/ref/index | duplicate ids, incomplete graph, bound/profile failure cannot create Program |
| Program bytes | strict bounded canonical decode once; same Program as hot | unknown/missing/surplus fields, wrong ref/catalog/profile/schema/graph rejected |
| local typed value | encode/content-address once; typed object retained under exact catalog instance | wrong contract or equal-fingerprint foreign catalog cannot downcast or enter event |
| cold value | strict decode/intrinsic check once; settlement uses retained typed value | bad canonical bytes/ref/schema/type/catalog brand rejected before QualifiedRun |
| run prefix | each exact frame ingressed/folded once; exact head and object closure | mutate every envelope/record/object/ref/predecessor/index duplicate; add surplus object/record |
| direct append | owned prepared successor advances with zero read/decode/fold | no echo, no independent batch/successor/index pairing constructor |
| found attempt | raw frame ingressed and compared once | valid unequal = conflict; malformed/over-limit = invalid/capacity; no promotion/invocation |
| provider response | protocol/auth/request relation once before pending observation | mismatched request, invalid signature/schema/bounds cannot become observation |
| observation settlement | direct selected commit settles from retained typed value | stale/unknown/conflict/non-selected never settle or reinvoke |
| configuration | local zero ingress; cold each revision once; direct zero readback | bounds/predecessor/ref/Found/stale/unknown/malformed dormant matrix |
| portable export/offline qualification | consuming structural v5 encode; expected whole-stream ref checked before bounded decode; Store sole qualifier | old v4, wrong stream ref, omitted/substituted/cyclic/cross-tenant/surplus/tampered/over-limit closure rejected; no persistence/API import |
| EVM identity | vNext tenant/domain/key convergence rules | principal/issuer/old domain bytes and unsafe activation rejected |
| process assembly | exact Program/catalog/binding/implementation resolution once | missing/surplus/wrong mode/provider/route/signer/effect domain rejects facade exposure |

### 10.3 Append and access disposition matrix

Every row requires assertions for session ownership, successor installation, provider calls, and
retained quarantine:

| Disposition | Successor/session | Invocation | Recovery owner |
| --- | --- | --- | --- |
| direct New, plain | exact prepared successor installed | n/a | returned session |
| direct New, reservation | exact successor retained in the Store observation writer; no `RunSession` exists yet | exactly one `ReadyToInvoke` becomes eligible | private ready package with the same `SessionContinuation` |
| direct New, observation | exact successor installed into a rebuilt session | zero additional calls | returned session; sole selector yields settlement |
| Found exact same | local successor not installed | zero | explicit qualified resume if needed |
| Found valid different | none; fail conflict | zero | none |
| Found invalid/over-limit | none; fail closed | zero | none |
| StaleHead | stale session consumed | zero | explicit resume |
| StoreEpochChanged | none | zero | reopen/reassemble Store |
| DurabilityProfileLost | none | zero | operator repair/new epoch |
| FactFrontierChanged | none installed | zero | exact reprepare continuation; no callback, scan, or provider rerun |
| ProjectionMismatch | none; integrity failure | zero | operator diagnosis |
| Unknown | none installed | zero | exact affine `SuspendedRun` |
| unknown retry → Found | none installed | zero | qualified resume |
| unknown retry → direct New, plain/observation | exact retained successor installed into a rebuilt session | zero | returned session |
| unknown retry → direct New, reservation | exact successor retained in the Store observation writer; no `RunSession` exists yet | exactly one `ReadyToInvoke` becomes eligible | private ready package with the same `SessionContinuation` |
| unknown retry → Stale | none | zero | explicit resume/rebase |
| repeated Unknown | none | zero | same non-Clone suspension returned |

Prepare two or more same-head actions and attempt every request/input/run/occurrence/Program/
assembly/adapter/binding/effect-domain/ordinal substitution. Only one exact direct winner may
advance; no API accepts a foreign raw result or successor.

Observation recovery tests must cover direct commit, stale→Prepared, repeated stale,
AlreadyRecorded, NoLongerSelected, Conflict, unknown→found-same, unknown→absent-unchanged,
unknown→absent-advanced, repeated unknown, invalid found bytes, drop/cancellation, and process
restart. Provider invocation count remains one and response-ingress count remains one throughout.

### 10.4 Type/API compile-fail suite

Use Trybuild or the repository's current compile-fail owner. Prove that downstream code cannot:

- construct, mutate, deserialize, or invoke `Program`;
- treat `ProgramDocument` or `ProgramRef` as `Program`;
- call a wrong `StateImplementation<S>` constructor or register wrong capability/value ABI;
- use Effect in the Read/Pure-only FanOut contract;
- construct/clone/serialize an `AdmissionMaterialScope`, construct an admission candidate without
  that exact scope, or transpose a candidate/scope across Store, tenant, catalog, or entry profile;
- construct/clone/serialize `QualifiedRun`, `RunSession`, `SuspendedRun`, configuration write/
  suspended sessions, `PreparedAppend`, `CommittedAppend`, `UnresolvedAppend`,
  `PreparedReservationAppend`, `ReadyToInvoke`, `ObservationWriteContinuation`,
  `ProcessRegistry`, or assembly brand;
- import a public adapter invoker, invoke a registered binding, or reach an `invoke(None)` path;
- construct a `CommittedAppend` or promote an uncommitted prepared successor;
- use a plain/observation commit disposition to create access invocation;
- use a fact-scan continuation without the direct committed reservation path; or
- enable a Cargo feature that exposes any constructor/mint above.

Also test that callback-free Store/Replay products cannot invoke even Pure, and a foreign Runtime
assembly cannot brand or drive a qualified run.

### 10.5 Exact negative source, metadata, and wire checks

Add maintained checks scoped to relevant paths. They should fail on the following retired symbols
or spellings (case/Serde variants included) while allowing genuine provider-protocol auth names.
The production/current-contract scanner excludes exactly this accepted RFC, this implementation
plan, and its own banned-token manifest because those three files intentionally inventory retired
spellings. Hostile wire fixtures either assemble the spelling from fragments or use an explicit
fixture-only allowlist. Do not exempt whole `tests/`, fixture, or documentation trees.

**Application policy:**

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
```

**EVM issuer:**

```text
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

**Journal coordination:**

```text
ExternalAccessAuthorized
AccessAuthorizationProposal
AuthorizationIntent
RecordLogicalKey::Authorization
PrimaryIntent::Authorization
ExpectedAuthorization
AuthorizationEntry
CertifiedAccessAuthorization
CommittedAccessAuthorization
AuthorizedCallOrigin
AuthorizedProviderCall
QualifiedRuntimeIntent::Authorization
Authorized<K, V>
```

Check access-record `authorization_ref` only in Journal/Store/Runtime/Replay/live-EVM coordination
paths; do not reject unrelated provider protocol authorization types.

**Generic live currentness:**

```text
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
SigningGenerationGuardFuture
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
```

Add inventory-generated checks for every U6 field/family, public `EvmPhysicalBindingRelease*`,
effect refresh evidence variant, current-release selector, and signer/provider generation wrapper;
the fixed literal list above is not permission for an unlisted shadow to survive.

**Old capability/callback algebra:**

```text
ComponentFuture
ReadAdapterCompletion
EffectAdapterCompletion
EffectContractCompletion
ReadCapabilityImplementation
EffectCapabilityImplementation
ReadAdapterInvoker
EffectAdapterInvoker
BoundedComponentContract
BoundedComponentInvoker
SignerContract
ResourceAuthorityContract
ErasedStateCallbacks
ProcessHandle
SigningCapability
```

**Authority-seal markers:**

```text
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
```

**Program/fact old wire:**

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
CompleteThroughAuthorizationFrontier
canonical_response_base64url
canonical_response_json
FactSelectionReadResponse::from_canonical_json
stable_resource_lineage_contract_refs
```

**Portable verifier/view/import shadows:**

```text
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
strict_decode   (as a production public method)
Application::import
import_run
ImportRunRequest
ImportRunResult
```

**Packages/features/core API:**

```text
mfm-spec
mfm-certify
mfm-authority-seal
runtime-authority
store-authority
RuntimeHistoryPort
VerifiedRunView
drive_once   (under kernel Runtime and App public API)
run_history_batch_objects
target_authority
```

Metadata must prove no direct/dev/feature edge and no Nix task references a deleted package. Strict
wire tests must reject the old Program document families, `external_access_authorized`, access
`authorization_ref`, old fact frontier spelling, portable v4, old EVM issuer values/domains, and
the old PostgreSQL schema contract. Avoid a blanket textual ban on “authorization,” “lease,” or
“credential.”

Named migration owners that must not be missed:

- rewrite `tests/integration/tests/cargo_metadata_contract.rs` and its `nixfied.nix` task to assert
  Runtime→Store and absence of Spec/Certify/Authority-seal;
- delete/replace `access_error_contract.rs` and its explicit integration `Cargo.toml` target;
- invert `transport_surface_contract.rs` assertions that currently require `drive_once`, forbid
  resume, freeze credential routes/CLI, or name writer fences;
- update `run_identity_contract.rs` tenant wording and replace the current
  `legacy_surface_contract.rs` scanner with a scoped scanner that includes `tests/`, fixtures, and
  documentation instead of skipping them;
- rewrite `crates/app/tests/application_privacy_ui.rs` and delete its auth/currentness fixtures;
- replace Journal `structured_tests.rs`/`structured_record_families.rs`, Facts response tests,
  PostgreSQL fact/config/history tests, and `tests/wallet-authority-provider` goldens at their owning
  commits; and
- split `crates/kernel/store/tests/structured_runtime_causal.rs` into callback-free Store and live
  Runtime owners; replace `structured_runtime.rs`, `api_surface.rs`, and
  `structured_history_qualification.rs` rather than preserving their old `StructuredRuntime`,
  seal, certification, or offline-verifier contracts; and
- replace `crates/kernel/store/tests/structured_history_qualification.rs` offline verifier cases and
  `crates/kernel/store/tests/export_source_closure_unit.rs` borrowed-view cases with the Store sole-
  qualifier and consuming-export contracts; replace `tests/integration/tests/replay_wire_contract.rs`
  with v5 expected-stream-ref/bounds/no-import coverage; and
- update root `README.md`, `docs/persisted-public-surfaces.md`, `docs/btc-rpc-routing.md`,
  `docs/evm-rpc-routing.md`, `docs/evm-transactions.md`, `docs/portfolio-snapshot.md`,
  `crates/kernel/facts/README.md`, `crates/kernel/store/README.md`,
  `crates/kernel/replay/README.md`, all affected crate/binary READMEs, and CLI/REST examples
  explicitly. Replay and transport docs must say offline qualification is read-only and no
  persistent/App/CLI/REST import exists.

### 10.6 PostgreSQL and crash tests

Backend conformance runs against Memory and PostgreSQL. PostgreSQL-specific coverage includes:

- the only public Store open path accepts one `Arc<dyn StructuredStoreBackend>`, expected identity,
  exact catalog instance, and limits, then yields history/reader/configuration/audit ports sharing
  that one coordinator; identity/catalog mismatch or any failed subtrait readiness check yields no
  partial port; compile-fail tests reject a `StoreParts` literal, per-port extraction/replacement,
  and construction from two opens, while a hostile doc-hidden split attempt fails the exact private
  coordinator brand check;
- object-safety/implementability tests for the composite history/configuration/fact/audit backend,
  `BackendFuture`, every borrowed command getter, and public mechanical raw DTO
  `try_from_parts`/getter API; count, byte, coordinate, and overflow negatives must fail before
  Store ingress without exposing payload bytes in errors;
- exhaustive fault injection mapping for every `BackendOperationError` variant at read, prepare,
  append, fact, configuration, and audit sites; only a connection loss after `COMMIT` may return
  `AcknowledgementUnknown`, while every pre-commit/definite-rollback failure preserves the exact
  retry owner;
- schema/catalog/role/channel qualification with no run/config scan;
- exact `mfm.structured-run-history-postgres.v8` catalog and fresh
  `run_history_heads`/`run_history_batches` columns, composite keys, deferred selected-head foreign
  key, queries, grants, and (when enabled) attention partial index; v7, normalized object rows,
  `StoredBatchEnvelope`, `StoredObjectRow`, and legacy object-row insert/load SQL are absent and
  rejected;
- logged table, primary status, `fsync`, `full_page_writes`, and effective
  `synchronous_commit` qualification plus weak-setting rejection;
- transaction-local durability pin and typed loss after a setting/epoch change;
- raw full-frame exact round trip and duplicated mechanical-column mismatch ingress;
- route proof/lock before append-id lookup, then tenant-and-Store-scoped `Found` before head
  comparison in one lock scope; seed identical run/append ids under two tenants and prove neither
  bytes, Found, head, nor error classification crosses the requested tenant;
- atomic batch/head/fact publication/frontier/conditional-attention transaction and injected
  rollback at every statement;
- when U3 is enabled, one-snapshot canonical-order inventory with exact-bound success,
  `max_items + 1` cardinality rejection, byte-bound rejection, concurrent head flips that never
  yield a mixed snapshot, and qualification of a selected run before action; when absent, compile/
  catalog/source assertions prove the method, DTOs, column, partial index, and SQL are all absent;
- definite commit followed by actual primary crash/restart retains the reservation before the test
  provider is allowed to enter;
- uncertain acknowledgement resolution matrix using the same borrowed command;
- legitimate restore rotates Store identity/writer epoch, all workers for one composed Store share
  one epoch, and no test treats it as an auth/currentness/cache token; and
- old schema, object table assumptions, target-authority row, and same-epoch restored deployment
  are rejected by qualification.

The PostgreSQL source/catalog/migration negative scan is scoped so journal predecessor fields are
not false positives, but it rejects these exact retired storage spellings in production paths:

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
has_effect_entry_attention
```

Intentionally hostile old-schema fixtures may assemble banned literals from fragments or live in
an explicit scanner allowlist; production SQL, schema inventory, migration catalog, qualification,
and code must contain none of them.

Same-epoch self-consistent rollback after all independent memory is lost remains outside the stated
threat model; tests/docs must not claim the hash chain detects it.

### 10.7 Secrets and external protocol boundaries

Retain or strengthen tests proving no password, token, mnemonic, private key, raw signing seed, or
provider credential appears in Program documents, configuration revisions, journal frames, facts,
portable exports, public DTOs, CLI/REST output, errors, tracing, or debug formatting. Keep bounded
secret ingress, zeroization, constant-time comparison, keystore AAD anti-swap, and corruption/
tamper coverage.

Separately prove provider response/challenge/signature/request binding still rejects mutation and
wrong route/target. Those tests must use “protocol authentication” language, not resurrect app
caller authorization or live MFM binding currentness.

### 10.8 Performance acceptance

After U2/U8/U10 set numeric contracts, benchmark:

- maximum canonical Program ingress and hot finish;
- maximum batch construction/append/frame ingress;
- maximum supported run cold resume and memory high-water mark;
- multiple directly committed steps in one retained session;
- worst selected prior-run fact closure;
- maximum configuration chain ingress; and
- one-shot transport `resume -> drive` versus retained `RunExecutor`.

Acceptance is the recorded threshold, not an unqualified asymptotic claim. The direct hot path must
show exactly zero prefix reads/folds after the initial resume. Report allocations/peak resident
memory as well as canonical bytes. Do not add a cache/checkpoint as a benchmark workaround.

### 10.9 Documentation verification

Check every changed intra-repository link, Rust snippet against the final public API, CLI/REST
example against current help/schema, SQL/table claim against the catalog, and Nix command/task id
against `nixfied.nix`. Run `git diff --check`. Documentation tests are part of the owning commit,
not a final cleanup batch.

## 11. Engineer-agent handoff and final report

The implementing engineer should treat this plan as a sequence of outcomes, not a request to
preserve current file shapes. Before each logical commit, restate its deletion boundary and the old
guards whose jobs must be structurally replaced. When an unexpected current consumer appears:

1. identify the exact proposition it consumes;
2. place a real immutable/temporal/domain proposition with the target owner;
3. update the RFC/plan if that changes architecture; and
4. delete the old path rather than maintaining both.

The final handoff report must contain:

- owner rulings U1–U11 and links to their tests/benchmarks/inventories;
- the final crate dependency graph and list of deleted packages/features;
- public Rust API inventory before/after, highlighting every new opaque/affine type;
- persisted identity/schema versions and deployment activation steps;
- product-code LOC and public-type counts before/after, excluding generated/vendor artifacts;
- every deliberate net-new concept with its one owner and why an existing concept could not do the
  job;
- exact test/gate commands and results;
- any unrun gate and reason;
- known limitations: trusted embedding exposure, provider factual trust, manual EntryOnce recovery,
  cold-resume bound, and no rollback proof after anchor loss; and
- confirmation that old RFC/code/docs terminology and compatibility paths are absent.

Success is not “the new types exist.” Success is that the old internal distrust machinery, policy
system, currentness control plane, duplicated serialized authorities, dual reducers, and hot
revalidation paths no longer exist—and every retained invariant has one obvious owner.
