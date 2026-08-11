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
> returns an opaque immutable value that carries that evidence. Crossing an internal crate or module
> boundary does not erase it, and no downstream layer reconstructs or revalidates the same fact.

This is a data-ingress rule, not an application access-control system. MFM does not authenticate end
users, accept or manage caller credentials, evaluate roles, ACLs, or grants, or keep capability
bindings live-revocable.
Exact-head comparison, durability acknowledgement, redaction, secret lifetime, and adapter protocol
authenticity are different propositions with explicit owners.

MFM is reachable only behind a trusted embedding/deployment boundary. That owner admits callers,
controls network and process exposure, and selects the tenant-scoped facade. The credential-free
REST service must not be exposed directly to untrusted callers. Public/untrusted exposure would be
a new product requirement and requires an access-control design outside this RFC.

The target design has a small set of load-bearing values and no generic validation layers between
them:

```text
ProgramDocument bytes -- one ingress --> Program

configuration source/row bytes -- one ingress --> ResolvedConfiguration<T>@G

raw run prefix -- one cold ingress + fold --> QualifiedRun@H

QualifiedRun@H + RuntimeAssemblyBrand --> RunSession@H

typed proposal -----------------------> ResolvedEvent
qualified record bytes ---------------> ResolvedEvent

RunSession@H + ResolvedEvent
    -> PreparedAppend { batch, successor: PreparedSuccessor@H' }
    -> private PreparedDrive::{Plain, Access, Observation}
       // each variant owns the Runtime session shell + Store append + its exact continuation
    -> RuntimeCommitCoordinator::commit(PreparedDrive)
       -> backend exact-head compare-and-append
       -> direct backend NewlyCommitted
       -> private CommittedDrive::{Plain, Access, Observation}
```

`CommittedDrive::Plain` and `CommittedDrive::Observation` own only the rebuilt advanced session.
`CommittedDrive::Access` owns only the matching `ReadyToInvoke`: there is no parallel advanced
`RunSession` while provider entry is possible. That ready value owns Runtime's
`SessionContinuation`, Store's exact `ObservationWriteContinuation<K, C>` containing the direct-new
active successor, and the inaccessible one-use invocation. No other backend outcome constructs a
committed variant.

New-run spawn follows the same preparation/commit path from locally typed admission inputs and an
absent predecessor. It does not cold-load unrelated retained runs; only explicitly selected
program, configuration, or prior-run fact evidence is a dependency of that admission.

`Program` is valid by construction. `QualifiedRun@H` is callback-free semantic evidence for one
exact journal prefix. Only Runtime can combine it with the exact assembly brand to create an affine
`RunSession@H`; purpose readers never receive drive authority. Multiple workers may construct
sessions, but exact-head compare-and-append selects one successor and only that call's direct
commit branch releases its retained one-use invocation. The backend's payloadless
`NewlyCommitted` is never exposed as a free proof token. `ResolvedConfiguration<T>@G`
carries the analogous typed configuration evidence. `ResolvedEvent` is the sole run-semantic input.
`PreparedAppend` binds the bytes and not-yet-authoritative successor that were derived together. A
positive commit does not cause the process to distrust and reconstruct its own values.

External access uses journal reservation, not security authorization:

```text
PreparedDrive::Access {
    SessionContinuation,
    append: PreparedReservationAppend<K, C>,
    inaccessible one-use invocation
}
    -> RuntimeCommitCoordinator::commit
    -> direct backend NewlyCommitted
    -> ReadyToInvoke {
           SessionContinuation,
           ObservationWriteContinuation<K, C>,
           inaccessible one-use invocation
       }
    -> invoke_once -> response ingress -> AcceptedAccessResponse
    -> PreparedDrive::Observation {
           SessionContinuation,
           PreparedObservationAppend
       }
    -> direct commit ExternalAccessObserved
    -> rebuild RunSession -> select and settle the recorded observation
```

`ExternalAccessReserved` means only that MFM durably reserved one exact journal attempt before
invocation. It does not reserve a provider resource, grant permission, or claim that invocation
occurred. Qualified cold history and a `Found` attempt classified as `ExistingSame` can prove that
the reservation exists but can never reconstruct `ReadyToInvoke`; stale and indeterminate outcomes
do not prove existence.

An **exact-head compare-and-append** atomically requires the current head to equal the prepared
predecessor, inserts the immutable batch, updates its append-atomic projections, and advances the
head. Initial admission requires the predecessor to be absent. This is deliberately not called
compare-and-swap: no journal entry is replaced.

The framework does not claim to sandbox arbitrary Rust code:

> `Pure`, `Read<C>`, and `Effect<C>` classify authority supplied by MFM. `Pure` means that the
> callback receives no MFM-provided live capability. Every external operation mediated by MFM is
> statically declared as `Read<C>` or `Effect<C>` and enters through the journaled Runtime protocol.
> Application callbacks, adapters, and their dependencies are trusted process code. Direct ambient
> I/O inside them is a programming violation that MFM cannot detect.

This narrower guarantee is both honest and strong enough to make incorrect framework-issued
capability wiring unrepresentable.

---

## Material uncertainties

The PostgreSQL and readiness choices are settled: PostgreSQL is the durability/current-head
authority within one admitted store epoch, and ordinary readiness never qualifies dormant history.
These remaining choices require owner rulings before implementation begins.

1. **Outstanding non-idempotent Effect recovery**

   - **Choice:** an unobserved `EntryOnce` reservation parks as unresolved and requires operator
     attention. Only an explicitly duplicate-absorbing Effect may reserve a bounded new ordinal
     under its capability-specific idempotency/absorption contract.
   - **Why uncertain:** the product may expect automatic cross-process continuation after a hard
     crash even when the provider exposes no status or idempotency evidence.
   - **If wrong:** automatic recovery would require a generic provider fence or could duplicate a
     real-world Effect, contradicting this product decision.
   - **Resolution:** confirm that ordinary non-idempotent Effects use manual attention after an
     unobserved reservation; otherwise require a capability-specific provider proof instead of a
     generic Runtime lease.

2. **Session ownership and cold-resume latency**

   - **Choice:** an active executor owns one Runtime-branded affine `RunSession`; there is no global
     run-state LRU, cross-process run lease, or persisted semantic checkpoint. A genuinely new
     resume qualifies and folds the bounded complete prefix once.
   - **Why uncertain:** the current public one-action API encourages independent
     `drive_once(run_id)` calls, and worst-supported cold-resume time has not been measured.
   - **If wrong:** stateless calls routed across workers may repeatedly replay a large prefix and
     miss the product latency target.
   - **Resolution:** define how long an executor may retain a session and benchmark maximum
     supported histories. Consider a separately designed checkpoint only if an explicit cold-resume
     SLO is missed.

3. **Effect-attention inventory**

   - **Choice:** when tenant-wide discovery is a product requirement, the append transaction updates
     a per-run `needs_effect_attention` projection and PostgreSQL partial index atomically with the
     journal head. It is snapshot-complete routing under the trusted-database contract, never
     execution authority.
   - **Why uncertain:** the product may require only recovery of a known run, in which case a global
     inventory API is unnecessary.
   - **If wrong:** MFM either retains an unused global surface or cannot proactively find an
     abandoned Effect without enumerating histories.
   - **Resolution:** decide whether operators must discover unresolved Effects without knowing run
     ids. If enabled, land one complete Store/PostgreSQL slice: append-atomic projection column,
     partial index, one-snapshot listing API/DTO, explicit maximum materialized count and byte
     bounds, and conformance tests. If absent, delete that whole slice; do not keep only its schema
     or only its API.

4. **Provider factual trust**

   - **Choice:** typed configuration binds capability `C` to a concrete `ProviderBinding<C>` that is
     part of that capability's TCB unless `C` explicitly requires cryptographic or quorum evidence.
     Response ingress proves protocol, authenticity, and request binding, not factual truth.
   - **Why uncertain:** current contracts sometimes use “verified” for both propositions.
   - **If wrong:** a well-formed, authenticated but false response may be treated as objective fact.
   - **Resolution:** classify the factual-trust requirement of every production capability and
     narrow its returned type to the evidence actually established.

5. **EVM sender/domain cutover**

   - **Choice:** remove the authenticated-issuer layer and derive the new submission intent from
     `TenantScopeId`, wallet nonce domain, and `SubmissionIdempotencyKey`. Activate it only in a
     fresh wallet/store domain; reuse a physical sender only after the old process is drained and
     every prior allocation/effect is terminal with its pending nonce reconciled.
   - **Why uncertain:** the repository has no proven inventory of externally retained incomplete
     submissions or sender reuse obligations.
   - **If wrong:** a new identity domain could collide with or strand old nonce/effect progress.
   - **Resolution:** prove terminal old state and exclusive sender control, or require a fresh
     sender for the new domain. The repository audit found no non-policy issuer-namespace owner, and
     this RFC deliberately retains none.

6. **Cold binding evidence**

   - **Choice:** persisted programs and reservations retain only the immutable secret-free binding
     descriptors/refs needed for callback-free replay and exact matching. Process construction
     deterministically qualifies those refs to live adapter handles once; no current-release
     history or live binding certificate survives.
   - **Why uncertain:** current physical-release certificates may encode a non-currentness
     proposition used by an offline or restart consumer that has not yet been identified.
   - **If wrong:** deleting the certificate wholesale could make a legitimate historical binding
     impossible to interpret or could silently drop a real immutable invariant.
   - **Resolution:** inventory every certificate field and consumer before the process-binding
     cutover. Move each surviving immutable proposition into the binding descriptor/ingress and
     delete everything whose only owner is hot replacement, lineage, or revocation.

7. **Dormant runs across binding replacement**

   - **Choice:** a retained program resumes only in an assembly satisfying its exact immutable
     binding descriptors. Rebuilding with a changed binding does not rewrite or silently replan the
     old program; those runs must already be terminal, continue under an assembly for the old
     binding, or remain read/replay-only.
   - **Why uncertain:** the product has not stated whether nonterminal dormant runs must continue
     after an operator replaces a provider/adapter binding.
   - **If wrong:** the static-binding cutover could strand valid nonterminal runs during ordinary
     deployment reconfiguration.
   - **Resolution:** define the deployment drain contract and inventory nonterminal-run retention.
     If rebinding old runs is required, design an explicit versioned capability-specific handoff or
     state migration separately; do not reintroduce generic live currentness.

8. **Total configuration-history bound**

   - **Choice:** every configuration stream has an explicit maximum revision count and cumulative
     canonical-byte bound in addition to the existing per-revision bound.
   - **Why uncertain:** demand-time qualification is not a work bound when a selected valid chain
     can grow without limit.
   - **If wrong:** one selected configuration may monopolize memory and CPU without violating a row
     bound.
   - **Resolution:** the configuration owner fixes both limits and a worst-case load target, then
     enforces them identically in memory, PostgreSQL, import, audit, and tests.

9. **Erased typed-value representation**

   - **Choice:** `mfm-program` owns an opaque canonical-bytes + exact-contract + erased-typed-value
     product that crosses Program, Store, and Runtime only under one exact in-process catalog
     instance.
   - **Why uncertain:** the current API has not demonstrated that this product covers aggregate
     structured values, cold ingress, provider outcomes, and `Send` movement without a lifetime or
     duplicate-decode escape hatch.
   - **If wrong:** the Program/value ownership split must change before removing posterior typed
     decoding.
   - **Resolution:** prove the final representation in a disposable compile spike, including
     hostile ingress and Runtime downcast, before landing it with its first production consumer.

10. **Maximum canonical complete-batch frame**

    - **Choice:** PostgreSQL stores one canonical complete-batch frame, bounded before allocation,
      ingress, and write.
    - **Why uncertain:** the existing envelope limit excludes separately stored objects while the
      whole-run limit is too broad to select the new per-append maximum automatically.
    - **If wrong:** a low limit rejects required appends; a high limit permits pathological rows and
      transient allocation.
    - **Resolution:** set and benchmark the frame limit together with the maximum retained prefix,
      then apply it to memory, PostgreSQL, import, duplicate-attempt ingress, and candidate building.

11. **PostgreSQL failure-domain claim**

    - **Choice:** the initial qualified profile promises primary crash/restart durability, not
      survival of primary-host loss.
    - **Why uncertain:** an out-of-tree deployment may already advertise synchronous-replica or
      quorum survival.
    - **If wrong:** Runtime may invoke after an acknowledgement weaker than the published failure
      domain.
    - **Resolution:** the deployment owner confirms the local profile or names and qualifies the
      exact stronger synchronous topology before implementation.

---

## 1. Scope

This RFC owns one cutover across typed program construction, state capability declarations,
configuration history, journal reduction, run spawning/resumption and session ownership, append
acknowledgement, tenant-scoped application facades, portable export/EVM identity cutovers, and
internal read projections.

It deliberately excludes:

- production provider topology, key custody, endpoint authentication, and binary assembly;
- a public background scheduler or product workflow redesign;
- compatibility with the current persisted program representation;
- rollback resistance after all independent memory of a later head is lost; and
- sandboxing or static analysis of arbitrary linked Rust code.

Those are separate product or deployment decisions. Mixing them into this refactor would make the
single-ingress rule depend on unrelated authority choices.

The journal hash algorithm and candidate/record/commit domain separators remain unchanged, and
`RunAdmitted` keeps its tenant-partition shape. Renaming `ExternalAccessAuthorized` to
`ExternalAccessReserved` changes the record schema and therefore record/commit bytes. Program,
portable-export, and EVM submission schemas also change. Deployment initializes a fresh namespace,
`StoreScopeId`, writer epoch, portable format, and affected wallet/domain activation; it never
mixes, rewrites, or decodes the old forms. New golden vectors freeze the one current schema. The
configuration revision content/predecessor format remains unchanged.

---

## 2. Trust model

### 2.1 The process trust base

The process TCB includes:

- compiled MFM crates and their dependencies;
- registered application state callbacks;
- concrete adapters, signers, transports, and storage implementations;
- the Rust compiler/toolchain used to build them;
- process assembly that selects the exact registry, configuration sources, and immutable bindings;
  and
- the trusted embedding/deployment supervisor that controls caller admission, exposure, process
  lifetime, and tenant-facade selection.

The trusted embedding constructs each application facade with one `TenantScopeId`. Public calls on
that facade accept neither credentials nor a tenant selector; every admission, lookup, fact query,
attention query, replay, and export is scoped by the captured tenant. A process serving several
tenants exposes separately constructed facades, and the trusted embedding selects among them.
A run id absent from the facade's tenant partition yields `RunNotFound`, never an authentication or
grant decision. If the backend nevertheless returns a retained row for that partition whose
`RunAdmitted.tenant_scope_id` disagrees, byte ingress reports `InvalidHistory`; an imported bundle
with inconsistent tenant closure is an invalid import. Corruption is not hidden as absence.

`ProgramCatalog`, `ProcessRegistry`, capability adapters, provider bindings, and signer/transport
dependencies are immutable after the composed process is exposed. Replacing any binding means the
embedding stops accepting work, drains the complete reservation/invocation/observation/settlement
bracket defined in section 4.4, drops the composed process, qualifies new configuration once, and
constructs a new process. A forced stop can leave ordinary possible-entry journal evidence; MFM
does not infer that an invocation failed to enter merely because the old process disappeared. There
is no generic Runtime API to replace, revoke, lease, refresh, or reselect a binding in place.

A persisted program may resume only under an assembly that satisfies its exact immutable binding
descriptors. A different binding creates a different program/assembly contract; rebuilding the
process does not silently reinterpret existing program bytes.

A bug or malicious dependency inside that TCB can violate MFM's contract. Re-serializing and
revalidating its output inside the same process does not create isolation from it.

Untrusted plugins or user-supplied executable code are not supported by this model. They require a
separate process, WASM sandbox, or another enforceable isolation boundary.

### 2.2 Byte ingress

The following are byte-ingress sites:

- caller request bodies and command input;
- configuration files, environment streams, and configuration-history rows;
- complete persisted prefixes loaded by explicit resume/replay/export, database projections, and
  bounded stored attempts returned for idempotency recovery;
- RPC and provider responses;
- imported portable exports and offline replay closures; and
- any serialized program document entering from outside its construction call chain.

Each ingress owner performs the checks relevant to its protocol exactly once:

```text
size bound
  -> framing / strict decode
  -> canonical representation
  -> intrinsic value invariants
  -> content identity
  -> protocol authentication and request/response binding, where applicable
  -> opaque domain value
```

The opaque value is then trusted for those immutable properties. It is not converted back to a
primitive or document and revalidated by another internal layer.

For an RPC/provider response, ingress also binds the response to the exact request, capability,
provider identity, and protocol version before returning `C::Returned` or `C::SafeFailure`.
Planning-time provider configuration does not validate a later response. Conversely, a valid
response value proves the admitted protocol proposition, not factual truth beyond the provider's
declared TCB/evidence contract.

### 2.3 PostgreSQL authority

Within one admitted store identity and epoch, MFM trusts the configured PostgreSQL service and its
sealed writer path to:

- report transaction outcomes truthfully;
- preserve committed journal/configuration bytes atomically and immutably;
- enforce exact-head compare-and-append plus the dense fact-frontier barrier/publication contract;
- retain committed history across the exact failure domain named by the admitted
  `DurabilityProfile`; and
- exclude mutation by roles other than the admitted writer contract.

`NewlyCommitted` means more than SQL statement success: the exact transaction has crossed the
durability point required by that profile. Every production profile must at least survive a crash
and restart of the admitted PostgreSQL primary. Store qualification therefore rejects `fsync =
off`, `synchronous_commit = off`, or any weaker effective session/server setting. If the product
claims survival of primary-host loss, the profile additionally requires and qualifies the named
synchronous replica/quorum acknowledgement. Runtime creates no `ReadyToInvoke` under a weaker
profile. A settings change that weakens the qualified profile invalidates the store epoch rather
than silently changing `NewlyCommitted` semantics.

An actively lying DBA or storage service is outside this threat model. If that adversary is added,
neither immediate readback nor repeated replay against the same service supplies an independent
witness.

The coordinator consumes `NewlyCommitted` for the exact sealed append to advance its already-proved
in-process successor. It causes no readback. A restore or replacement starts a new store
identity/epoch; a self-consistent older database is never silently presented as a continuation of
the same epoch.

On a later process start or explicit resume, retained rows are nevertheless bytes, because Rust
construction evidence did not survive serialization. They cross ingress and fold once before
becoming `QualifiedRun`; Runtime must still bind its exact immutable assembly brand before sealing
a `RunSession`. This is reconstruction of lost process evidence, not posterior skepticism about the
value that originally produced the rows.

### 2.4 Checks that are not byte validation

| Proposition | Owner | When checked |
| --- | --- | --- |
| Run is in the facade's fixed tenant | Tenant-scoped lookup/ingress | Admission/load |
| Journal head is current at that snapshot | Backend head read | Resume/latest-read |
| Prepared append extends the current journal head | Exact-head compare-and-append | At commit |
| Store writer epoch is current | Writer-epoch fence check | At append |
| Required fact frontier is current | Exact-frontier operation | Dependent append |
| Append is durably acknowledged | Qualified backend | Before releasing any MFM-mediated invoker |
| Nonce/operation key is unique | Domain adapter transaction | At reservation/mutation |
| Public output contains no secret | Public DTO/render boundary | Before emission |

These checks remain because their facts are contextual or can change. They must be named after the
property they establish, not hidden behind a generic `validate` layer.

### 2.5 Disposition of current check classes

| Check class | Target disposition |
| --- | --- |
| External byte bounds, decode, canonicalization, protocol authentication | Keep at owning ingress |
| Private constructor and global invariant check | Keep once at construction |
| Head/frontier, writer epoch, domain atomicity | Keep at the owning coordinator |
| Caller credentials, principals, grants, and policy decisions | Delete |
| Generic capability live-currentness/revocation | Delete; bindings are immutable |
| Redaction, zeroization, and secret sink control | Keep |
| Same-process serialize/decode/re-certify | Delete |
| Requalifying a locally compiled append | Delete |
| Intent-versus-recorded duplicate reduction | Eliminate structurally |
| Positive backend echo and comparison | Delete |
| Post-commit reload and field-by-field self-check | Delete |
| Purpose-specific verifier implementations or repeated folds within one operation | Delete |

---

## 3. Program validity is a representation invariant

### 3.1 One execution authority

There is exactly one representation accepted as execution authority:

```text
ProgramDocument   serializable, bounded, strict data; never execution authority
Program           opaque, immutable, process-local executable authority
```

The typed DSL may use opaque, private-field, non-deserializable `ProgramCandidate` and
`ProgramFragment` values while authoring and expanding recipes. They are construction IR, not a
third authority: store, reducer, and Runtime cannot consume them. The normalized graph and its
complete callback-free compiler live in `mfm-program`. That compiler owns recipe
expansion/lowering, normalized-graph and program-profile validation, manifest and document
construction, schema/lexical indexes, and the private graph-invariant pass.

Operation expansion remains a required pure construction stage. The authority collapse does not
collapse the compiler pipeline:

```text
OperationTemplate
  + ResolvedConfiguration<T>@G
  + secret-free capability/provider binding descriptions
  + frozen planning profile
    -> private OperationPlan / ProgramCandidate
    -> recursive child-operation substitution
    -> configuration-driven specialization and fan-out
    -> abstract operation/state lowering
    -> pre/proceed/post and failure-state injection
    -> normalization + one final-graph invariant pass
    -> Arc<Program>
    -> canonical ProgramDocument projection
```

Expansion may introduce more operations, states, matches, and fan-outs. Only normalized states are
executable after completion; stable semantic paths may retain construction provenance for tracing.
Every injected state and every configuration-dependent execution choice is visible in the final
graph. Planning consumes the already-ingressed typed configuration directly and performs no
ambient I/O. Live provider handles, protocol credentials, and keystore secrets remain in
`ProcessRegistry`; the program contains only secret-free binding identities. Live process
assembly, invocation thunks, and their private reservation-to-invocation transition move into
`mfm-runtime`. Remaining non-authoritative helpers move to their `mfm-program` or `mfm-runtime`
owner, and the superseded `mfm-certify` crate/authority wrappers are deleted rather than retained as
a facade.

`Program` has private fields, no `Deserialize`, no public struct literal, and no unchecked public
constructor. It is cheaply shared through `Arc`. A `ProgramRef` is an address, not semantic or
invocation authority: persisted records carry the ref, while typed finish or hostile-document
ingress under the exact callback-free catalog returns an `Arc<Program>` retained by the admission/
qualified run. Semantic consumers accept that resolved value. They never accept a document or bare
ref and then pretend it is already valid.

`ProgramRef` is the domain-separated hash of the complete canonical `ProgramDocument`. Every
`Program` also carries its exact in-process catalog-instance brand; persisted fingerprint equality
cannot substitute a Program/value/binding from a different catalog instance.

`Program` retains the normalized executable graph, canonical document/reference, value-schema and
lexical-slot indexes, entry signature and program-profile coverage, and permitted
semantic/implementation support envelope. The refactor removes duplicated proof authorities, not
the data Runtime and replay actually need.

Only two source-specific paths construct `Program`:

```text
typed DSL finish under a frozen ProgramCatalog ---------------------> Arc<Program>

hostile ProgramDocument -> strict ProgramIngress under same catalog -> Arc<Program>
```

Both paths converge on the same normalized-graph invariant owner. Source-specific work remains
source-specific: Rust typing prevents some DSL invalidity before `finish`, while cold ingress alone
performs byte bounds, framing, canonicality, and content-identity checks. The DSL path does not
serialize its result and call the hostile decoder. The ingress path does not receive a privileged
flag that skips normalized-graph checks.

The precise correctness-by-design claim is “if it is representable as `Program`, it is valid,” not
“every partially authored Rust value is already a complete program.” `finish` is the one fallible
global closure/bounds pass; after it returns, no posterior layer repeats that proposition.

### 3.2 What `Program` existence proves

Construction proves, for one exact frozen registry/profile identity:

- entry-point input, output, and failure signatures;
- declaration order and lexical dominance;
- exact state input/output/failure types;
- one `Pure`, `Read<C>`, or `Effect<C>` execution/access declaration per state;
- capability request/returned/safe-failure ABI;
- satisfaction of the normalized executable graph's frozen entry/program-profile language;
- exhaustive Match and bounded FanOut;
- failure routing and one total root outcome;
- unique identities and complete graph/component closure;
- every graph/profile construction, depth, and count bound plus an explicit canonical-document byte
  bound owned by ingress/frozen program profile; and
- exact secret-free implementation/catalog identities needed for restart and audit.

These facts cannot become false while the immutable `Program` and its frozen catalog fingerprint
live. There is no `validate_program(&Program)` API.

### 3.3 One program document

The persisted `ProgramDocument` contains one canonical normalized executable graph plus the exact
entry, profile, catalog, and implementation identities required to reconstruct it. It does not
persist three mutually checked authored, expanded, and certified authorities plus proof objects
whose only consumer recomputes them.

Program serialization is a projection from `Program`, not a constructor for it. Cold database or
import ingress verifies the document's exact content identity, graph invariants, component
resolution, and frozen catalog binding once, then returns the same `Program` type used by hot
authoring.

Resume consumes that exact final document. It does not rerun expansion using current configuration,
current compiler behavior, or a newly selected planning profile. Compiler and configuration changes
affect only newly constructed programs; an admitted run remains fixed to its normalized program and
configuration revision.

That proves a proposition about the normalized graph that exists. It does not prove the historical
claim that an omitted authored graph passed through a particular expansion implementation. If a
named audit consumer needs authored source or a derivation trace, retain it as content-addressed,
non-executable metadata. It never becomes a second validity authority and Runtime never needs it to
execute the normalized graph.

The current `AuthoredStructuredProgram` public-field/deserialization surface and multi-object
`CertifiedProgramDocument` representation are replaced. Old bytes are rejected under the fresh
store-identity cutover.

### 3.4 Catalog and process-registry ownership

Two ordered builders own different propositions and produce non-overlapping products:

```text
ProgramCatalogBuilder::finish()
    -> ProgramCatalog       // callback-free construction and hostile-document ingress

RuntimeAssemblyBuilder::new(clone of that exact ProgramCatalog instance)::finish()
    -> RuntimeAssembly {
           ProcessRegistry,       // private, non-cloneable live callback/adapter authority
           RuntimeAssemblyBrand   // private session/process identity
       }
```

`ProgramCatalog` owns:

- typed state and capability registrations;
- frozen entry profiles and construction bounds;
- complete callback-free recipe expansion/lowering, program-profile/graph validation,
  manifest/index/document construction, and hot DSL completion; and
- hostile `ProgramDocument` ingress.

The callback-free catalog must be finished first. `RuntimeAssemblyBuilder` cannot build or replace
its catalog, and fingerprint/content equality cannot substitute a separately built catalog
instance. Runtime-private `ProcessRegistry` owns the exact callbacks and adapters selected for
invocation.
`RuntimeAssemblyBuilder`, `ProcessRegistry`, `ErasedInvocationThunk`, the mode-specific private
prepared-access drives, and `ReadyToInvoke` are all defined in `mfm-runtime`. `Program` binds
immutable secret-free identities, never live handles. Store and offline replay receive the
callback-free catalog only. Runtime assembly compares each live registration to an exact
catalog-issued branded handle once; it does not revalidate Program semantics. Multiple immutable
composed processes may use clones of the same
catalog, but each has a distinct Runtime brand and registry. The trusted embedding and adapter code
remain TCB; an internal marker/seal cannot prove that callback code honestly implements a claimed
descriptor.

Runtime consumes the exact `Arc<Program>` already retained by admission or qualified history and
uses the shared `ProgramCatalog` instance only for branded typed association. It resolves live
implementation identities separately through `ProcessRegistry` only at execution. There is no
`resolve(ProgramRef)` path that treats an address as authority. Cloning `Arc<Program>` therefore
cannot clone an invoker or give store/offline tooling access to a callback.

This replaces the separate authoring certifier, persisted verifier, borrowed entry certifier,
duplicated verification snapshots, and later root/document equality checks. The finished catalog
and assembly are immutable bounded products, never Program caches. Hot admission and each qualified
cold run retain their returned `Arc<Program>`; neither builder, Store, Runtime, App, nor a purpose
reader owns a global Program resolve/interning map.

---

## 4. Framework capability correctness

### 4.1 One access mode per state

The target state contract is conceptually:

```rust
trait State {
    type Input: StructuredValue;
    type Output: StructuredValue;
    type Failure: FailureValue;
    type Execution: Execution;
}
```

`Execution` is sealed; its only implementations are `Pure`, `Read<C>`, and `Effect<C>`. Reusing the
existing associated-type name avoids a repository-wide terminology-only rename. Mode-specific
constructor `impl` blocks use the corresponding associated-type equality bound, such as
`S: State<Execution = Read<C>>`.

`Read<C>` and `Effect<C>` derive `Request`, `Returned`, and `SafeFailure` directly from `C`. Those
types are removed from `State`; there is no second declaration to compare through `TypeId` at
registry construction.

Each capability owns invariant-safe opaque request/returned values and a capability-specific
safe-failure sum. A broad domain failure enum is not accepted and then checked for an allowed
subset on every call; the EVM submission capabilities receive narrow wrappers/sums in this
cutover. Every relation needed to bind a response to its request or make the observation legal is
checked once at response ingress before `AcceptedAccessResponse` may consume Store's exact
`ObservationWriteContinuation` to prepare `ExternalAccessObserved`. Settlement may interpret that
already-valid observation against the state input; it cannot discover a capability relation whose
failure would invalidate the durable observation.

The unrelated capability-lowering vocabulary currently named `State::Capability`, `Direct`, and
`RequiresCapability` is renamed to `Lowering` or `Expansion`. It must not compete with live-access
capabilities for the same word.

### 4.2 Mode-indexed implementations

The public callback enum is replaced by an opaque `StateImplementation<S>` with mode-specific
constructors:

```text
StateImplementation<S: State<Execution = Pure>>::pure(...)
StateImplementation<S: State<Execution = Read<C>>>::read(...)
StateImplementation<S: State<Execution = Effect<C>>>::effect(...)
```

Consequently it is unrepresentable to:

- register Effect callbacks for a Read state;
- bind callbacks whose ABI differs from that projected by `S::Execution`;
- associate a nominally typed `C2` adapter with a hot `Read<C1>` registration;
- attach access callbacks to Pure;
- disagree about request, returned, or safe-failure types;
- place Effect in a FanOut rule that admits only Pure/Read; or
- invoke an adapter without the exact directly committed reservation package.

Type erasure preserves the mode as a private sealed callable enum:

```text
ErasedStateImplementation = Pure(PureHandle) | Read(ReadHandle) | Effect(EffectHandle)
```

Runtime callback dispatch matches that enum exhaustively rather than exposing every mode method on
one erased trait and returning `Option` for the wrong variant. Registry assembly performs one exact
catalog/process association check for mode, capability identity, and implementation identity; cold
document substitution is an ingress error. Registration derives canonical contracts from
associated types and nominal identities rather than accepting caller-authored complete descriptors
and comparing them back to the types on every use.

### 4.3 What callbacks receive

State callbacks receive only a value frame and, during settlement, one typed observation. Returned
and safe-failure settlement remain different functions; Runtime-only outcomes such as unknown
Effect entry are not smuggled into the state failure ABI:

```text
Pure       (&Input)                         -> Outcome
Read<C>    request(&Input)                  -> C::Request
           settle_returned(&Input, &C::Returned)       -> Outcome
           settle_safe_failure(&Input, &C::SafeFailure) -> Outcome
Effect<C>  request(&Input)                  -> C::Request
           settle_returned(&Input, &C::Returned)       -> Outcome
           settle_safe_failure(&Input, &C::SafeFailure) -> Outcome
```

They never receive a transport, HTTP client, filesystem, clock, store, signer, adapter, invoker,
registry, or generic capability bag from MFM. Composite behavior requiring multiple framework
capabilities expands into multiple visible states rather than hiding accesses inside one callback.

### 4.4 Invocation authority and adapter trust

Dynamic programs require internal type erasure, but erasure is not invocation authority.
Store owns the crate-private callback-free `PreparedReservationAppend<K, C>`. It is the sole owner
of the exact Read/Effect mode and typed request plus the state-input, tenant/run, program occurrence,
immutable execution binding, attempt ordinal/id, append identity, and reservation fixation.
Runtime does not copy those fields into an `AccessFixation` or expected-authorization shadow.
Runtime instead owns a mode-specific private prepared access drive containing exactly a
`SessionContinuation`, that Store package, and the inaccessible existential one-use invocation.
Neither package contains a caller credential, principal, grant, resource lease, freshness
requirement, revocation generation, or provider-entry permit.

A private constructor inseparably moves those three owners into `PreparedDrive::Access`.
`RuntimeCommitCoordinator::commit` consumes that whole package and invokes the Store-owned append's
consuming commit method; Runtime never supplies a second history port or independently selected
predecessor. The append is already bound to the exact callback-free catalog, Store coordinator,
and store epoch. A raw mechanical backend cannot mint it. The backend's payloadless
`NewlyCommitted` outcome remains inside that one consuming Store/coordinator frame and is never
returned as a proof value that another in-flight preparation could use.

On that direct branch, Store returns the exact `CommittedReservation<K, C>`. Consuming it yields the
assigned `reservation_ref`, Store's `ObservationWriteContinuation<K, C>` owning the direct-new
`ActiveQualifiedRun`, and, for the prior-facts mode, the one-use fact-scan continuation. Runtime
moves those values, its `SessionContinuation`, and the same inaccessible invocation into the sole
`CommittedDrive::Access { ready: ReadyToInvoke }` value. It does not also construct or retain an
advanced `RunSession`. `ReadyToInvoke` is private, non-`Clone`, non-serializable, and alone can
consume the captured thunk once.

Every definite non-new, stale, or invalid reservation branch destroys the inaccessible invocation
and returns no ready package. A typed transient Store retry or fact-frontier reprepare branch
instead preserves the exact Store preparation, Runtime `SessionContinuation`, and inaccessible
invocation in one private suspension; resolving that suspension performs one bounded preparation
or commit unit and still cannot invoke unless that exact retry reaches its own direct-new branch.
`AcknowledgementUnknown` similarly pairs Store's affine `UnresolvedReservationAppend<K, C>` with
the same Runtime continuation and invocation. Its bounded resolver consumes that package.
`Found`/`ExistingSame` destroys it without invocation; a proven absent append at the unchanged
predecessor may resubmit it, and only that retry's direct new branch can produce
`CommittedDrive::Access`. Continued ambiguity retains the quarantine for another explicit bounded
resolution or drops it without invocation. Cold history never reconstructs it. There is no
intermediate entry typestate because no post-commit generic freshness decision exists. None of
these constructors or handles is public under any Cargo feature, and neither Store nor the backend
constructs or returns an invocation package. Qualified cold history and `Found -> ExistingSame`
can prove that a reservation record exists but can never reconstruct `ReadyToInvoke`. `StaleHead`
and `AcknowledgementUnknown` prove neither existence nor absence and likewise cannot construct it.

Every durable attempt-identity field and attempt-id preimage remains. The cutover deletes the copied
`ExpectedAuthorization` shadow and repeated comparison only after the private
`PreparedDrive::Access -> CommittedDrive::Access` transition structurally owns every relation they
currently protect. It preserves Effect entry mode, ambiguity and absorption contracts, semantic
adapter dependency, access-fault contract, and the store-minted one-use prior-run fact-scan
continuation.
The non-zero `MAX_ENTRIES` continues to bound only capability-declared duplicate-absorbing
re-entry; generic refresh and supersession ordinals disappear.

Public unqualified `ReadAdapterInvoker`/`EffectAdapterInvoker` entry points and any `invoke(None)`
fallback are deleted. Adapter registration captures the concrete adapter value or closure into a
private `ErasedInvocationThunk` inside the immutable `ProcessRegistry`; the current public
cross-crate `Qualified*PhysicalBinding::invoke_authorized` surface disappears. Runtime can reach the
thunk only by consuming `ReadyToInvoke`. Trusted adapter code may still call its own internal
client, but MFM exposes no invocation path that bypasses the directly committed package.

Process assembly still performs one exact resolution from every immutable secret-free binding
descriptor to its adapter, provider/route, signer, and Effect domain. A missing, ambiguous, or
mismatched component rejects assembly before any facade is exposed. The cutover deletes only
latest/current generation selection and recurring checks; it does not weaken this one-time exact
binding proposition.

The framework does not create a second nested capability system for adapter internals. Callable
signer/resource process handles are deleted; concrete adapters own transports, signers, and
resource clients as TCB internals. Secret-free immutable binding and dependency identities remain
represented for journal fixation and cold binding checks. `Read<C>` classifies the behavior of the
whole trusted adapter, including those internals. This is a reviewed TCB assertion: because Read may
be retried or fanned out, code that may consume or mutate externally meaningful state must be
`Effect<C>`.

Binding replacement has no hot Runtime protocol. The embedding stops intake and drains every
unresolved/retry reservation package, `ReadyToInvoke`, provider call, `AcceptedAccessResponse`,
Store-owned `PreparedObservationAppend`, `RetryObservationPreparation`, `ObservationRebaseInput`,
or `UnresolvedObservationAppend` paired with Runtime's `SessionContinuation`, and resulting
settlement until each access has a durable observation/settlement or a deliberate durable terminal
park. Only then does it drop the old
assembly and construct a newly qualified process. If it forces shutdown instead, any committed but
not durably observed access remains ordinary conservative possible-entry evidence. A later process
may repeat a Read under its retry contract; an `EntryOnce` Effect parks; and an `EntryAbsorbing`
Effect may use a new ordinal only when its capability-specific contract and the exact immutable
binding/effect domain permit it.

### 4.5 Honest limitation

Ordinary Rust closures can capture their own clients, globals, clocks, RNGs, task spawners, or file
handles. MFM cannot prove otherwise. Such code is forbidden by repository policy because it bypasses
journaling and deterministic replay, but the framework does not add runtime validation pretending
to detect it.

A concrete adapter is equally trusted to honor Read versus Effect. The label governs retry,
fan-out, and ambiguity behavior, so deliberately mislabelling a mutating adapter as Read is a
TCB-critical implementation bug.

---

## 5. One semantic event and one reducer

The current writer reduces an intent, compiles it, requalifies its own bytes, reduces the recorded
form, and compares the two successors. This exists because there are two semantic implementations.

The target has one semantic input, one reducer, and one deterministic binding stage:

```text
typed proposal ---------------------------> construct ResolvedEvent
hostile persisted record -> qualification -> construct ResolvedEvent

apply(previous state, ResolvedEvent)
    -> PendingAppend { record draft, unbound successor, obligations }

local write:
    qualify AppendContext {
        store identity/epoch,
        predecessor,
        append request id,
        observed tenant fact frontier/coordinate,
        retained object context
    }
    -> deterministic compile/bind(PendingAppend, AppendContext)
    -> PreparedAppend { exact batch, bound PreparedSuccessor, exact index plan }

persisted-prefix ingress:
    for each batch, qualify the complete retained envelope
    -> ResolvedEvent + RetainedBatchFixation {
        store identity/epoch,
        predecessor,
        append request id,
        tenant fact coordinate,
        every assigned record including adjacent RunClosed,
        exact object set,
        head
    }
    -> the same apply step
    -> bind/check PendingAppend against RetainedBatchFixation exactly once
    -> after the complete fold, QualifiedRun
```

`ResolvedEvent` owns every semantic field on which state advancement depends. `PendingAppend` owns
the one reducer result before record coordinates exist. `AppendContext` supplies all changing
append-time inputs; none is read ambiently by the compiler. Binding adds only physical coordinates,
hashes, and index mutations mechanically derived from the event, context, and predecessor. Its
persisted record and new objects are projections of those inputs; the successor cannot depend on a
parallel semantic field absent from the record. Persisted ingress keeps one boundary-owned exact
projection/binding check over the full `RetainedBatchFixation`, so no surplus object, adjacent
record, envelope field, or otherwise ignored field can evade qualification. It does not introduce
a second semantic reducer.

This deletes:

- `QualifiedEvent::{Intent, Recorded}`;
- parallel intent/recorded reducer branches;
- `PendingSemanticStep::semantic_eq`;
- `ComparisonPassed` and `ComparedReduction`;
- local `qualify_recorded_successor` after `PreparedAppend` makes the same relations structural;
- decomposition of a just-compiled batch back into a semantic event; and
- reduce -> compile -> requalify -> reduce -> compare on every append.

An encode/ingress/apply equivalence property test remains valuable. It tests the one representation
law; it does not serve as a substitute for two implementations.

---

## 6. Journal heads are exact-prefix commitments

### 6.1 Keep the existing hash format

The current journal already implements the required recursive chain. Let `CJ` be the bounded,
float-free canonical JSON encoder.

```text
D_i = SHA256(
    "mfm.structured-candidate.v1\0"
    || CJ({
        run_id,
        expected_head,
        append_request_id,
        tenant_fact_coordinate,
        records,
        objects
    })
)

R_ij = SHA256(
    "mfm.structured-record.v1\0"
    || CJ({ run_id, run_sequence, ordinal, record })
)

K_i = SHA256(
    "mfm.structured-commit.v1\0"
    || CJ({
        store_scope_id,
        store_epoch,
        predecessor: H_(i-1),
        append_request_id,
        tenant_fact_coordinate,
        candidate_digest: D_i,
        record_refs: [
            { run_id, run_sequence, ordinal, record_hash: R_ij }...
        ],
        object_refs
    })
)

H_i = JournalHead { run_sequence: i, commit_digest: K_i }
```

The implemented preimages live in `crates/kernel/journal/src/structured.rs:1363-1413` and
`:1518-1535`; their domain-separated derivations are at `:1640-1677`.

Because `K_i`'s preimage contains the complete preceding `JournalHead`, while `H_i` contains the
new sequence and `K_i` digest:

```text
H10 commits batch 10 and H9
H9  commits batch 9  and H8
...
H1  commits genesis
```

The admission record binds the program, configuration/context/routing roots, and initial values.
Transition records bind input, observation, outcome, facts, and before/after semantic-state digests.
Access records bind Read/Effect, request, capability/adapter/binding identities, and observation.
Content references bind the canonical bytes introduced by each batch.

Rehashing the entire previous state at every append is unnecessary: the previous head is already
its recursive commitment. `RunSemanticStateDigest` remains the narrower digest of reduced semantic
state; it is not renamed or confused with the complete physical-prefix commitment.

### 6.2 What a trusted head establishes

Given a previously established `QualifiedRun@H10`, canonical encoding, and SHA-256 collision and
second-preimage resistance, H10 establishes:

- exact prefix identity and append order;
- predecessor continuity to genesis;
- exact record payloads and coordinates;
- exact identities of every retained object and exact bytes when those objects entered;
- the represented program, configuration, values, facts, access requests, and observations; and
- equality between the process's reduced state and the prefix from which it was established.

A public deserialized `JournalHead` establishes none of this by itself. Semantic evidence comes
from opaque `QualifiedRun`; callback execution additionally requires the exact immutable Runtime
assembly brand described below.

### 6.3 Non-guarantees

The chain does not prove:

- semantic legality before the prefix has crossed ingress;
- that the presented head is latest;
- which of two otherwise valid successors is accepted; exact-head compare-and-append admits at most
  one;
- authorship, because the hashes are not signatures or MACs;
- absence of a valid rollback after the trusted anchor is lost;
- continued availability of historical records or object bytes;
- provider truth or real-world effect occurrence; or
- arbitrary callback machine-code behavior beyond represented implementation identities.

This is hash-chain integrity, not blockchain consensus, finality, or replication.

---

## 7. Qualified history and Runtime-branded drive authority

`QualifiedRun@H` is opaque read-only callback-free semantic evidence, scoped to one store identity/epoch,
tenant, run, program-catalog fingerprint, and exact `JournalHead`. It owns the `Program`, qualified
immutable context, reduced state, retained-object index, fact dependencies, outstanding access
state, and cumulative capacity accounting required to project or continue that prefix.

`ActiveQualifiedRun@H` is the separate affine writer-capable continuation. It owns one
`QualifiedRun@H` plus the exact private history coordinator and is minted only by the qualified
Runtime history port on admission/resume or by direct promotion of that coordinator's own committed
successor. Callback-free readers, replay, export, and audit never receive this type.

`RunSession@H` is an opaque, non-serializable, non-cloneable Runtime drive package:

```text
RunSession@H {
    active: ActiveQualifiedRun@H,
    assembly: private immutable RuntimeAssemblyBrand,
}
```

Purpose readers may borrow callback-free `QualifiedRun` internally and return a DTO. They cannot
construct a writer-capable active run or `RunSession`, extract the assembly brand, reach
`ProcessRegistry`, or invoke even a Pure callback. Runtime creates a session only from its qualified
history port's active run under the exact catalog and its private `RuntimeAssemblyBrand`. Every
callback dispatch requires that brand structurally;
Store's `PreparedReservationAppend<K, C>` owns the exact immutable adapter/binding and attempt
fixation for Read/Effect, while Runtime's paired typed drive adds only the one-use invocation.

`QualifiedRun` can arise only from complete-prefix ingress/fold or a local prepared successor after
direct commit. `ActiveQualifiedRun` can arise only from the exact qualified writer port or its
direct committed successor. `RunSession` can arise only from Runtime-branded spawn/resume or direct
advancement of the same session. A raw/deserialized run id, head, `ProgramRef`, reducer snapshot,
read-only `QualifiedRun`, or callback-free qualified value cannot create drive authority.

### 7.1 Active continuation, not a cache or global lock

The Runtime core exposes an explicit session lifecycle. An in-process executor qualifies once, then
owns its affine branded continuation across deterministic drive steps until terminal completion, an
external wait, or drop. Driving consumes and returns the session, or mutably advances it under
equivalent exclusive Rust ownership. The proof is not thrown away after each internal step.

There is no shared semantic run-state LRU, warm-head lookup, suffix-refresh protocol, eviction
policy, process-wide semantic run map, or catalog-local Program cache in this target. Those
mechanisms only compensate for a stateless API that repeatedly opens the same run. Ordinary `Arc`
sharing inside one admission/qualified run carries no run currentness or invocation authority.

Non-cloneability prevents duplicating one in-process continuation; it does not pretend to establish
global exclusive run ownership. Multiple workers may independently resume the same H and construct
separate branded sessions. Their prepared successors race at exact-head compare-and-append. At most
one append is newly committed; its direct caller alone advances its retained successor and, for an
access reservation, transfers that successor into the sole Store `ObservationWriteContinuation`
inside `ReadyToInvoke`. It does not also receive a session. Every loser consumes or drops its stale
owner, installs nothing, invokes nothing, and must explicitly resume before further work. No
run-driver lease, generation, transfer, expiry, or revocation protocol exists.

### 7.2 Direct advancement and durability

The direct committed Plain/Observation branch of the consuming coordinator rebuilds its owned
`RunSession@H'` from the retained successor with zero history reads, decoding, or reduction. The
direct reservation branch instead moves that same kind of Store-owned active successor into
`ObservationWriteContinuation` inside `ReadyToInvoke`; no parallel session exists until the
observation path resolves. The compare-and-append transaction checked the store writer epoch and
exact predecessor, and PostgreSQL acknowledged the qualified durability point for the exact sealed
bytes. Asking it to echo those bytes cannot strengthen that claim.

`StaleHead`, a typed append-precondition failure, and `AcknowledgementUnknown` install no successor
and release no invoker. Resolving the exact append identity may allow a later explicit resume, but
it never reconstructs a lost one-use invocation package. Only a direct committed branch that still
owns the original or quarantined preparation can create one.

A fresh process, explicit replay/export, or new resume fails if required historical bytes are
missing or malformed. The active session is not required to reread its prefix before every Effect:
within one epoch, PostgreSQL immutability and durability are trusted operational facts. A legitimate
database restore rotates store identity/epoch. Presenting an older state under the same epoch is a
storage-contract violation; without an independent witness MFM cannot detect such a lie after all
later process evidence is gone.

### 7.3 Performance boundary

No cache or durable semantic checkpoint is required for correctness. A checkpoint would add a
second persisted representation, codec/version contract, atomic repair rules, and potentially
quadratic write amplification as reduced state grows. This RFC does not introduce one.

The enforceable hot-path property is:

> After one cold resume, every directly committed step retains the same affine successor ownership
> without reloading or refolding any retained batch. Plain/Observation commits rebuild the session;
> a reservation commit moves the successor into `ReadyToInvoke` until observation resolution.

Cold resume remains bounded by the existing history limits and may replay the complete prefix.
Hard limits on concurrent active sessions and ingress work remain ordinary resource controls. A
stronger cold-resume latency guarantee, compact checkpoint, or global asymptotic claim requires
measurement and a separate design; it is not hidden inside an LRU.

---

## 8. Append, acknowledgement, and invocation

### 8.1 Prepared append

Applying one `ResolvedEvent` produces `PendingAppend`; deterministic assignment, projection, stable
binding discharge, and exact index enumeration then seal one private coherent value:

```text
PreparedAppend {
    batch,
    successor: PreparedSuccessor@H_next,
    projection/index deltas,
    optional needs_effect_attention delta when that product surface is enabled,
    store identity/epoch + expected predecessor + fact-frontier preconditions
}
```

Every field is derived from the same event, predecessor, and qualified `AppendContext`. Independent
callers cannot supply a batch, successor, and projection that merely happen to be wrapped together.

The seal proves exact record/object projection (including absence of surplus objects), assigned
record/head binding, direct object/value/first-seen/fact-scan index extension, and exact immutable
program/configuration/adapter/binding fixations. An access preparation separately retains its
inaccessible invocation only in Runtime's typed access drive; Store's
`PreparedReservationAppend<K, C>` remains callback-free and solely owns the reservation fixation.
Neither the journal batch nor any Store append contains a callable. The append constructor is
private and cannot mix a batch, successor, or index plan from different preparations. Runtime's
private pairing constructor then consumes the exact `SessionContinuation`, owner-bound Store
append, and mode-specific live invocation or observation continuation into the corresponding
`PreparedDrive` variant; callers cannot omit or transpose any of them between commits.

The backend interprets no record-family semantics. It owns only:

- transaction atomicity;
- exact-head compare-and-append and affected-row checks;
- append-atomic exact replacement of the run-head projection and, when enabled, its attention
  projection;
- store-writer epoch and tenant-fact-frontier preconditions;
- append identity and idempotency lookup;
- database-native parameter/frame constraints; and
- ambiguous-acknowledgement classification.

Semantic append capacity, retained-history totals, and the two-successor access-reservation reserve
are proved while constructing `PreparedAppend`. Adapter-specific nonce locks, operation keys, and
transactions remain inside the owning adapter when their domain contract requires them; the run
backend does not absorb them into a generic capability-freshness model.

Database constraints may redundantly protect mechanical row relationships. The backend does not
re-run program qualification, reduction, object semantic closure, or cross-field facts already
made structural in `PreparedAppend`.

### 8.2 Outcomes

```text
BackendAppendOutcome =
    NewlyCommitted
  | Found(StoredAttemptBytes)
  | StaleHead
  | AcknowledgementUnknown
```

This raw SPI disposition is consumed inside the Store-owned commit operation on the exact prepared
append, which is called from the one Runtime coordinator frame that still owns its session and live
continuation. Backend implementations and conformance tests necessarily construct/observe the raw
enum, but it is not semantic evidence, is never accepted by a successor-promotion or invocation
API, and is not returned to Runtime code. The Store operation returns only an append-bound qualified
disposition.

The backend is deliberately not a semantic equality oracle. `Found` returns a bounded raw retained
attempt. The store ingress owner qualifies it and is the sole classifier of
`ExistingSame | AppendConflict | InvalidHistory | CapacityExceeded`; only a well-formed unequal
attempt becomes `AppendConflict`. The coordinator does not ask the backend to classify it and then
repeat the comparison.

`StaleHead` means only that the exact-head comparison failed. Writer-epoch expiry,
fact-frontier mismatch, capacity, and projection inconsistency remain distinct typed failures; they
are not relabelled as a stale journal head.

| Coordinator branch | Returned affine owner | May create `ReadyToInvoke` |
| --- | --- | --- |
| `NewlyCommitted`, Plain/Observation | Rebuilt advanced `RunSession`; no reload | Never |
| `NewlyCommitted`, reservation | Only `ReadyToInvoke`, owning Runtime shell plus Store active successor | Only for that exact direct reservation |
| `Found` -> `ExistingSame` | Do not promote local candidate; an observation resumes only from qualified stored history | Never |
| `Found` -> `AppendConflict` | Fail closed; install nothing | Never |
| `Found` -> invalid/capacity error | Fail closed; install nothing | Never |
| `StaleHead`, Plain/reservation | Consume the stale owner; explicit resume is required | Never |
| `StaleHead`, observation | Runtime shell plus Store-owned `ObservationRebaseInput` | Never |
| transient retry/fact-frontier reprepare | Runtime shell plus exact Store retry/prepared package | Never |
| `AcknowledgementUnknown` | Runtime shell plus exact Store unresolved package | Never |

The unknown branch may retain only a private Runtime suspension pairing its
`SessionContinuation` with Store's exact affine `UnresolvedAppend`,
`UnresolvedReservationAppend`, or `UnresolvedObservationAppend`; it never retains a separately
advanced session or free invoker. Exact resolution consumes that pair: an existing same append
releases nothing; a proven absent append at the unchanged predecessor may retry the same owned
preparation, and only that retry's direct `NewlyCommitted` reservation branch can create
`ReadyToInvoke`. Dropping or losing the quarantine leaves cold recovery conservative.

`NewlyCommitted` is payloadless because the positive value would only echo the sealed candidate the
backend consumed. The coordinator still owns the consumed `PreparedDrive`, so payloadless does not
mean unbound. A found stored attempt remains real database ingress: its retained bytes are
qualified and compared once against the candidate because it may describe a historical append
followed by later heads. Equal bytes establish idempotency only; they do not establish that the
attempt is still the current run head.

Two workers may prepare from the same predecessor. Exact-head compare-and-append admits at most one
successor. Only the direct branch of the coordinator call that consumed the corresponding
`PreparedDrive::Access` can construct `ReadyToInvoke`; a raw outcome cannot be transposed from
another append. Every losing or indeterminate path installs nothing and invokes zero times. An
observation append occurs after an invocation, so its non-new outcome releases no new invoker and
causes no additional invocation.

### 8.3 Durable external access

The MFM-mediated access bracket remains:

```text
PreparedDrive::Access {
    SessionContinuation,
    PreparedReservationAppend<K, C>,
    inaccessible one-use invocation
}
  -> RuntimeCommitCoordinator::commit
  -> direct backend NewlyCommitted
  -> ReadyToInvoke {
         SessionContinuation,
         ObservationWriteContinuation<K, C>, // owns ActiveQualifiedRun@H_reserved
         inaccessible one-use invocation
     }                                       // no parallel RunSession
  -> consume once into exact adapter/provider I/O
  -> validate external response once
  -> AcceptedAccessResponse
  -> PreparedDrive::Observation {
         SessionContinuation,
         PreparedObservationAppend           // Store owns qualified response event
     }
  -> direct commit ExternalAccessObserved
  -> RunSession@H_observed -> select and settle the recorded observation
```

Runtime never passes the adapter or invoker to the state callback. The affine package binds the
exact capability, access kind, request, state-input fixation, run, program occurrence, immutable
assembly/registry, adapter implementation, binding identity, attempt ordinal, reservation record,
and committed head. Store's reservation package is the sole owner of the callback-free fixation;
Runtime owns it transitively instead of copying the tuple. The consuming coordinator's direct
branch consumes `CommittedReservation<K, C>` and moves its
`ObservationWriteContinuation<K, C>`, Runtime's exact `SessionContinuation`, and the
still-inaccessible invocation into `ReadyToInvoke`. Consuming that once performs provider I/O.
Successful response ingress moves the same owners into `AcceptedAccessResponse`, which alone may
consume the Store continuation to produce `PreparedObservationAppend`. No generic resource owner
is consulted between journal commit and provider I/O, and there is never both a post-call response
owner and an independently usable run session.

No other call can perform that move. `Found`/`ExistingSame`, a cold resume, a replay, a stale
predecessor, an append conflict, or an ambiguous acknowledgement can never recreate the thunk or
invocation package. The post-reservation full reload and field-by-field comparison are deleted:
they only reconstructed a value the process had just created and could not make an
indeterminate/non-new outcome safe to invoke.

Observation append has an explicit no-reinvocation recovery path with one response owner. Once
response ingress succeeds, Store's consuming `ObservationWriteContinuation<K, C>` derives the
qualified `ExternalAccessObserved` event and its rebase evidence together. From then until a
terminal disposition, exactly one of these Store-owned affine packages contains that response:

- `PreparedObservationAppend` while the first or rebased append is ready/committing;
- `RetryObservationPreparation` while a transient preparation or fact-frontier reprepare is
  required;
- `ObservationRebaseInput` after a definite stale-head result; or
- `UnresolvedObservationAppend` while acknowledgement is ambiguous.

Runtime never duplicates the response tuple into a second post-call shadow type. It pairs the
one current Store package only with the original `SessionContinuation` inside opaque
`SuspendedRun`/`PreparedDrive::Observation` ownership. There is no independently usable
`RunSession` beside that pair and no API that accepts a caller-supplied latest run.

One explicit `SuspendedRun::resolve` performs at most one Store retry, ambiguity lookup/resubmit, or
reducer-owned rebase unit. A stale observation package consumes itself to load and qualify the
latest selected prefix through its own Store coordinator and then returns exactly one branch:

```text
ObservationRebaseInput::rebase()
  -> Prepared(PreparedObservationAppend)       // same attempt remains selected and unobserved
  | AlreadyRecorded(ActiveQualifiedRun)         // exact observation is durable
  | NoLongerSelected(ActiveQualifiedRun)        // retry/absorption selected another attempt
  | Conflict                                    // different observation exists for that attempt
  | Retryable { input: ObservationRebaseInput, error }
  | Failed(error)
```

Rebase checks the exact reservation/attempt tuple and never calls an adapter. The reservation's
logical key admits at most one observation. If a rebased append also loses the head comparison,
Store returns the moved qualified response in another `ObservationRebaseInput`, again paired only
with the Runtime shell. There is no hidden retry loop. `AlreadyRecorded` discards the duplicate
local event and rebuilds the session from the returned qualified active run so settlement selects
the recorded value. `NoLongerSelected` consumes the late event without state settlement and returns
the exact latest active run; the journal already contains the retry/absorption closure that made it
non-selected. `Conflict` fails closed. None of these paths invokes again or revalidates the
already-ingressed response bytes.

`AcknowledgementUnknown` for an observation does not discard that evidence or settle state. Store
returns the exact affine `UnresolvedObservationAppend`; Runtime quarantines it beside the same
`SessionContinuation` and resolves its append identity in one explicit bounded unit:

- if the exact same observation was committed, Store qualifies the recorded history, discards the
  duplicate local event, and returns the active run from which Runtime selects settlement;
- if the append is proven absent and its predecessor is still current, the same Store package may
  retry once, and only its direct new branch may rebuild a session and settle;
- if the append is absent but the head advanced, Store returns the unchanged qualified response as
  `ObservationRebaseInput` for a later explicit bounded rebase unit;
- repeated ambiguity remains in `UnresolvedObservationAppend` under the normal work bound; and
- different found bytes, invalid history, or an observation conflict fail closed.

Every nonterminal branch preserves the already-ingressed response in exactly one Store package,
invokes zero times, and either reaches one durable selected observation before settlement or a
typed non-settling disposition.

An unobserved reservation is a blocking journal state, not evidence that entry did or did not
occur. A fresh process never invokes that ordinal. A Read may create a new ordinal because Read's
published semantics permit repetition. An `EntryOnce` Effect remains `PossibleEntry` and requires
manual attention. An `EntryAbsorbing` Effect may close/reassert a bounded new ordinal only under its
explicit duplicate-absorption/idempotency contract and only for the exact same immutable binding
and effect domain; otherwise it also parks. No timeout, process replacement, or binding change may
record “did not enter.”

Because there is no live lease, a new Read or `EntryAbsorbing` ordinal may race a slow still-live
`ReadyToInvoke` from the older reservation. That is legal only because repetition or duplicate
absorption is the capability's explicit semantic contract. The race may consume the bounded attempt
budget and leave the run parked; the framework promises safety, not automatic liveness. `EntryOnce`
never takes this path.

This protocol is deterministic journal coordination, not authentication or authorization. It
ensures durable intent before invocation, at most one release of the exact prepared attempt,
the selected observation before its settlement, and conservative recovery without a generic
live-resource control plane.

---

## 9. Readers, facts, startup, and configuration

### 9.1 One demand-triggered ingress

Resume/drive and per-run read, trace, replay, and export requests use the same bounded
prefix-ingress and reducer implementation. An active executor passes its `RunSession` directly; a
genuinely separate read-only request constructs callback-free `QualifiedRun`, projects the requested
DTO, and drops the evidence. It never receives a Runtime assembly brand. Distinct public DTOs and
the facade's fixed tenant partition remain because they own disclosure boundaries, not history
validity.

If tenant-wide Effect recovery discovery is enabled, its listing reads only the append-atomic
routing projection. It cannot execute or settle a run. Any recovery action selected from that list
must explicitly resume the named run through the same ingress before acting.

An outbound export that needs raw retained bytes loads those bytes through database/object ingress
and content-checks them once before the export sink emits them. An imported bundle crosses import
ingress. No purpose owns a parallel semantic verifier.

### 9.2 Facts

Fact verification closure fixes selected producer prefixes at exact producer heads through the
publication routes, and the completed response commits the exact selected evidence. Once that
producer prefix has been qualified for the exact head within a consuming session/load scope, it is
reusable in that scope. A later explicit resume reconstructs required producer evidence from bytes;
there is no process-global semantic cache.

Only a consumer that explicitly selects prior-run evidence qualifies the selected producer prefix
and closure. Creating or driving a run with no such dependency does not open unrelated producer
runs.

Offline import/export closure remains self-contained: its supplied producer bytes are a new ingress
and cannot borrow evidence absent from the bundle.

The tenant fact frontier and its dense publication sequence remain append-atomic evidence. Their
continuity proves that the prior-run selector did not silently omit an eligible publication; each
route fixes the exact producer record and head. A gap, missing route, wrong producer binding, or
exact-frontier precondition mismatch is therefore an integrity/currentness failure, not a
disposable-index miss.
This load-bearing negative-completeness chain is distinct from per-run semantic projections and
Effect-attention routing, which never creates execution authority.

The frontier establishes a changing cross-run fact, but it is not a reason to revalidate every
already-retained producer prefix.

When tenant-wide Effect recovery discovery is part of the product, the reducer-derived
`needs_effect_attention` Boolean is carried by `PreparedAppend`. PostgreSQL updates it in the same
transaction that appends the batch and advances the run head; a partial index supplies a
snapshot-complete list under the trusted-database contract. The listing materializes at most the
owner-fixed count and canonical-byte bounds in that one snapshot and fails explicitly at `max + 1`;
it is never an unbounded enumeration. A second attention frontier, boot audit, and hot-path
reconstruction are unnecessary. If the product does not expose global discovery, the projection,
partial index, DTO/API, bounds, and tests are deleted together.

### 9.3 Demand-time history qualification

Ordinary store open verifies:

- schema and store identity;
- backend channel and durability profile;
- writer epoch;
- registry and process assembly; and
- ability to execute bounded snapshot loads and exact-head compare-and-append transactions.

Startup does not enumerate, load, qualify, or reduce retained run or configuration history. A
malformed dormant run therefore does not prevent store opening, service readiness, or admission and
execution of an unrelated new run.

A retained run crosses byte ingress only when an operation actually consumes that history: resume
or drive, a per-run read or trace, replay, export, explicit audit, or selection as another run's
producer/fact dependency. Qualification failure rejects that consuming operation before it receives
qualified evidence or drive authority, emits output, or releases an invoker. It does not poison
store readiness or unrelated runs.

Starting a new run qualifies its own inputs and only the historical evidence it explicitly selects.
A malformed selected program document, configuration revision, producer prefix, or fact closure
rejects that admission. Malformed unrelated retained history cannot.

`audit_store` is a separate read-only operator action that deliberately qualifies all visible run
and configuration histories plus the dense fact frontier in one fixed repeatable-read snapshot. It
reports corruption with explicit progress. Its result is diagnostic: it does not become readiness
authority, create invocation authority, repair history, or persist a proof after the result is
discarded.

### 9.4 Configuration history follows the same ingress rule

Configuration is not an exception to this RFC merely because its history is smaller. External
configuration files, environment streams, API payloads, and retained configuration rows are bytes
until their owning ingress returns an opaque `ResolvedConfiguration<T>@ConfigurationHead`. A locally
constructed typed value already satisfies the same intrinsic invariants and is not serialized and
reparsed for reassurance. Program planning consumes this typed value directly.

Persisted `ConfigurationValue` remains secret-free. At process assembly, secret-bearing input
crosses its configuration/keystore ingress and is consumed into concrete immutable
adapter/provider/signer internals owned by `ProcessRegistry`; no generic callable signer/resource
process handle is created. Secret material is never projected into a configuration revision, journal
record, fact, export, or diagnostic and retains its domain-specific zeroization/redaction lifetime.

A concrete adapter may renew a protocol token internally only when renewal preserves the exact MFM
binding descriptor and provider/effect identity. That is provider-protocol implementation inside
the adapter TCB, not an MFM lease/currentness API. Changing the binding descriptor, provider
identity, signer authority, or effect domain requires stop/drain/drop/rebuild.

The writer constructs one private coherent value:

```text
PreparedConfigurationAppend {
    revision,
    successor: PreparedConfigurationSuccessor@ConfigurationHead_next
}
```

The revision binds its exact predecessor and content reference.
`ConfigurationCommitCoordinator::commit` consumes `PreparedConfigurationAppend`, calls the backend
exact-head compare-and-append, and keeps the payloadless `NewlyCommitted | Found(raw) | StaleHead |
AcknowledgementUnknown` outcome internal to that call. Only its direct `NewlyCommitted` branch
promotes the owned retained successor. `Found(raw)` crosses configuration ingress once and becomes
either `ExistingSame`, a typed conflict, or an invalid/capacity error; no positive backend echo is
compared by a second owner and no raw outcome can advance another preparation.
`AcknowledgementUnknown` may quarantine that exact affine prepared append for bounded resolution;
found-same promotes nothing, while a proven absent append may resubmit the same owned preparation.

The active configuration owner retains the locally prepared successor after direct commit; there is
no shared semantic configuration cache. A later selection for planning, resume, replay, export, or
audit qualifies the bounded predecessor chain once. A malformed selected revision rejects that
consumer, while a dormant malformed revision does not block startup or a new run that does not
select it. Selecting whether a revision is currently active is a changing head/activation
proposition and remains an explicit query; it is not another validation of immutable revision
bytes.

The typed planner/admission bridge first deletes reader-specific replay, cache, and suffix-selection
paths in favor of the one bounded independent ingress. The subsequent demand/open cut deletes eager
configuration-open audit before configuration writer semantics change. The final configuration
writer cut deletes writer-side full-prefix reload, repeated
`ValidatedConfigurationAppend::from_object` on locally created revisions, and positive echo
comparison. This does not require a generic history framework: run semantics and configuration
semantics retain distinct domain types while obeying the same ingress law.

---

## 10. API and deletion cutover

### 10.1 Tenant-scoped application and policy deletion

The target application surface is construction-scoped:

```text
Application::new(TenantScopeId, qualified process/store assembly) -> Application

application.admit(...)
application.resume(run_id)
application.read(run_id)
application.replay(run_id)
application.export(run_id)
```

If the owner retains tenant-wide Effect recovery discovery, the same facade additionally exposes
`application.list_effect_attention(...)`; otherwise neither that method nor its projection exists.

No public call accepts a credential, authenticated principal, grant, policy-decision reference, or
tenant selector. The facade supplies its captured tenant to every backend operation. Admission
persists that same tenant partition; a retained run or recursive source outside it is not visible
through the facade. A trusted multi-tenant embedding constructs multiple facades and selects one
outside MFM.

Delete the application access-control model completely:

- `SecretCredential`, `MAX_SECRET_CREDENTIAL_BYTES`, and `SecretCredentialError`;
- `ApplicationAccessPolicy`, `ApplicationAccessGrant`, `AccessTarget`, `AuthorizedTenant`, and
  `AccessPolicyError`;
- authenticated-principal identities, policy-decision references, grant typestates,
  `AuthorizedRunCall`, `AuthorizedAdmissionCall`, `run_grant`, and per-call policy evaluation;
- recursive export-source policy callbacks;
- `AuthenticationRequired`, `GrantDenied`, `SourceRunExportDenied`, and policy-only
  `Unauthorized`/`Forbidden` error classes;
- the CLI `--access-token-file` option, token reader/support module, and every credential argument;
  and
- REST bearer/`Authorization` parsing, credential extraction, and policy-only 401/403 mappings.

Do not replace those surfaces with tenant headers, CLI tenant flags, inert optional fields, or a
default-allow policy implementation. PostgreSQL credentials, keystore secrets, provider protocol
authentication/signatures, and response-authenticity checks remain because they protect different
boundaries.

Persisted/exported policy evidence is also deleted:

- `AuthorizedExportClosure` becomes structural `ExportClosure` and contains no principal, grant,
  decision, or policy reference;
- `PortableAuthorizationDecision`, `export_decisions`, grant constants, and decision validation
  disappear;
- structural helpers named `authorized_sources`, `authorized_source_prefixes`, and
  `with_authorized_sources` become neutral source-closure/source-prefix terms;
- recursive export still proves a bounded, acyclic, same-tenant, exact-head source closure;
- the redundant `authorized_closure_digest` disappears because the portable stream's complete
  `ContentRef` already fixes the exact closure bytes; and
- portable export advances to one new format version with no legacy decoder.

`RunAdmitted` retains `TenantScopeId` as domain partition/provenance and gains no security identity.
EVM submission removes `authenticated_principal_id`, `AuthenticatedIntentIssuerId`, and the
policy-only issuer namespace layer, including `IntentIssuerPreimage`,
`derive_authenticated_intent_issuer_id`, and `issuer_namespace_contract_ref`.
`EvmCallerSubmissionToken` becomes
`SubmissionIdempotencyKey`. The new `SubmissionIntentId` is domain-separated over
`TenantScopeId`, wallet nonce domain, and `SubmissionIdempotencyKey`; that key is an ordinary
non-secret idempotency input, not a credential. All affected value/hash
domains and stored wallet shapes cut over under a fresh activation with no compatibility reader;
an old physical sender is reused only after its previous process is drained and nonce/effect state
is proven terminal and reconciled.

### 10.2 Program and certification

Replace with one current API:

- opaque `Program`;
- serializable `ProgramDocument`;
- private-field, non-deserializable `ProgramCandidate`/`ProgramFragment` authoring IR that no
  execution consumer accepts;
- one callback-free `ProgramCatalogBuilder::finish` stage and one subsequent
  `RuntimeAssemblyBuilder::new(clone of that exact ProgramCatalog instance)::finish` stage that
  produces an immutable assembly with a private non-cloneable `ProcessRegistry`; and
- no Program cache/intern/`ProgramRef` resolver in either builder or any downstream owner;
  admissions and qualified runs retain their own `Arc<Program>`.

Delete or absorb:

- public/deserializable executable authored graph types;
- separate `CertifiedProgram` authority naming;
- `AdmissionCertificationRegistry`;
- `AdmissionVerificationRegistry`;
- `EntryPointCertifier` and `AdmissionVerifier` wrappers;
- the `mfm-certify` crate after its callback-free compiler moves to `mfm-program` and live assembly
  moves to `mfm-runtime`;
- self-verification of a document just constructed locally; and
- exact document reconstruction/equality on the hot admission path.

### 10.3 State and capabilities

Delete or replace:

- `State::{Request, Returned, SafeFailure}`;
- `Execution::access_type_ids`;
- publicly selectable `StructuredStateCallbacks` variants;
- all-modes `ErasedStateCallbacks`/state handles, replaced by the sealed Pure/Read/Effect callable
  enum;
- public unqualified `ReadAdapterInvoker::invoke` / `EffectAdapterInvoker::invoke` surfaces and the
  certification `invoke(None)` smoke fallback, plus public
  `Qualified*PhysicalBinding::invoke_authorized` access;
- `ReadCapabilityImplementation` / `EffectCapabilityImplementation`, their erased validator
  wrappers and `ProcessHandle` capability variants, and repeated request/completion validation calls
  after opaque values and narrow failure sums own those invariants;
- registration-time callback-kind and ABI `TypeId` repairs;
- wrong-variant `Option` callback dispatch;
- the older parallel `EffectSpec`/`CapabilitySpec`/`CapabilitySet`/role algebra, its trybuild suite,
  the otherwise-unused `SigningCapability` bridge and Cargo dependency, and identities/tests owned
  only by that obsolete algebra;
- callable signer/resource process handles, while retaining secret-free immutable semantic
  dependency and binding identities;
- `NoRefresh`, `NoRefreshEvidence`, `Refreshable`, `EffectRefreshMode`,
  `EffectCapabilityContract::Refresh`, the refresh-evidence parameter and
  `SupersededBeforeEntry` variant of `EffectAdapterCompletion`, and every refresh-evidence
  validator;
- `RuntimeEffectCapability::RefreshBinding`, `RuntimeEffectRefreshBinding`, `NoRefreshBinding`,
  `RefreshableBinding`, `RuntimeResourceAuthority`, and refresh-only Resource components;
- persisted `StructuredEffectRefreshContract`, `RetainedPhysicalReleaseTrust` and its seal, public
  `EvmPhysicalBindingRelease*`, dynamic current-binding selection, release histories,
  minimum/stable resource-lineage heads, `PhysicalBindingSupersession`, generation-guarded
  signer/provider wrappers, and generic per-call target-currentness checks;
- `PhysicalBindingAuthorization`, `SemanticObligation::PhysicalAuthorization`, and
  `PhysicalObligationChecker::{verify_retained_authorization, verify_current_authorization}`;
- capability lease/fence requirement metadata, tokens, epochs, entry-freshness callbacks, and every
  public or private hot-replacement/revocation API;
- feature-gated public invocation-package constructors, replaced by one crate-private Runtime
  transition; and
- the copied `ExpectedAuthorization` shadow and repeated runtime comparisons only after the private
  `PreparedDrive::Access -> CommittedDrive::Access` package structurally owns every relation they
  currently protect.

Retain:

- one exact capability per state;
- `Read<C>` and `Effect<C>` request/return/failure contracts;
- opaque invariant-safe capability values, capability-specific safe-failure sums, and the one
  request/response relational check at response ingress before observation construction;
- Effect entry, ambiguity, and absorption semantics;
- distinct Read/Effect completion types, including Effect-only entry dispositions;
- safe-failure disposition types;
- FanOut's type-level Effect exclusion;
- affine reservation and invocation, including dynamic erasure and all durable exact-attempt
  identity fields;
- exact adapter/signer/resource dependency identities and the reserved one-use fact scanner;
- one-time immutable descriptor-to-adapter/provider/route/signer/effect-domain resolution and
  assembly-time mismatch rejection; and
- panic containment and redaction-safe fault conversion.

Existing domain `valid_*` functions are not blindly deleted. Intrinsic unary invariants move to
private constructors/custom deserialization, request-response relations move to the one adapter
response ingress before observation construction, and state-specific interpretation remains at
settlement. Operation-specific nonce locks, SQL transaction guards, and idempotency keys remain
inside the adapter that owns their domain semantics. They do not surface as a generic Runtime
capability-currentness framework.

### 10.4 Store and runtime

The journal/API terminology cutover is exact:

- `ExternalAccessAuthorized` becomes `ExternalAccessReserved`;
- `RecordLogicalKey::Authorization` becomes `RecordLogicalKey::AccessReservation`;
- `AccessAuthorizationProposal` becomes `AccessReservationProposal`;
- `QualifiedRuntimeIntent::Authorization`, `PrimaryIntent::Authorization`, and
  `AuthorizationIntent` are absorbed by `ResolvedEvent::AccessReservation`, with no alias or second
  intent representation;
- every access `authorization_ref` becomes `reservation_ref`;
- authorization barriers/frontiers become access-reservation barriers/frontiers;
- public/certified/committed authorization wrappers disappear into the private
  `PreparedDrive::Access -> CommittedDrive::Access` transition;
- reducer `AuthorizationEntry` becomes `ReservationEntry`; Runtime `Authorized<K, V>` is replaced by
  the private erased `ReadyToInvoke` path with a nominal typed inner; and live EVM `AuthorizedCallOrigin` /
  `AuthorizedProviderCall` become reservation-named coordination wrappers;
- response ingress returns one consuming Runtime `AcceptedAccessResponse` that still owns the exact
  Store `ObservationWriteContinuation`; preparing the observation consumes both, after which no
  Runtime response shadow remains and the response moves only through Store-owned
  `PreparedObservationAppend`, `RetryObservationPreparation`, `ObservationRebaseInput`, or
  `UnresolvedObservationAppend`, always paired only with Runtime's `SessionContinuation`; and
- `ExternalAccessObserved` remains the observation record name.

The five journal families are therefore `RunAdmitted`, `StateTransitionCommitted`,
`ExternalAccessReserved`, `ExternalAccessObserved`, and `RunClosed`. Old enum tags, aliases,
decoders, schemas, and fact-mode spellings are not retained.

Delete:

- local intent qualification that reparses trusted domain values;
- re-decoding/requalifying just-authored objects to reconstruct their index;
- `qualify_recorded_successor` on local batches;
- dual reducer branches and comparison typestates;
- positive committed-batch echo/comparison;
- Runtime's post-reservation reload and recheck;
- application pre-drive and post-drive verified loads;
- `drive_once(run_id)` as the Runtime-core ownership API, replaced by explicit spawn/resume and an
  affine caller-owned session;
- purpose-specific verification implementations and repeated loads within one session/request;
- stale-head/idempotency recovery that silently reloads and continues; a pre-invocation stale owner
  requires explicit resume, while a post-invocation response permits only one explicit bounded
  `SuspendedRun::resolve` unit over its Store-owned package; and
- the eager whole-store semantic-open sweep from ordinary readiness.

Keep:

- strict demand-triggered complete-prefix qualification for explicit resume/read/replay/export;
- callback-free `QualifiedRun` for semantic projections and Runtime-branded `RunSession` for
  execution;
- exact content/object closure for newly entered bytes;
- direct extension of the prepared successor's object/value/first-seen/fact-scan indexes from the
  resolved typed artifacts, rejecting surplus objects and ignored record fields at cold ingress;
- affine successor ownership: Plain/Observation direct commits rebuild `RunSession`, while a direct
  reservation holds the active successor only in `ReadyToInvoke`'s Store continuation;
- atomic exact-head compare-and-append;
- one exact stored-attempt ingress comparison against the already-built candidate;
- ambiguous acknowledgement recovery;
- backend-owned writer-epoch/fact-frontier currentness; and
- separate redaction-safe public projections.

Do not introduce a global semantic run cache, suffix-refresh protocol, cloneable session, or durable
reducer checkpoint in this cutover.

### 10.5 Configuration

Replace configuration writer/reader/open-audit verification paths with one typed configuration
ingress, `ResolvedConfiguration<T>`, private `PreparedConfigurationAppend`, and payloadless positive
commit. The read-side ingress/evidence/planner bridge must land before App/Runtime typed admission;
it deletes purpose-specific replay, shared configuration-cache plans, and suffix selection. Delete
eager configuration scanning from store readiness next with the common demand/open cut. The final
writer cut makes the active writer retain and directly promote its prepared successor and deletes
local writer revision revalidation and positive echo comparison. An independent selection always
performs one demand-triggered complete-history ingress. Retain strict external source/row ingress,
exact predecessor/content binding, exact-head compare-and-append, idempotency, ambiguous
acknowledgement recovery, and the separate diagnostic `audit_store` scan.

---

## 11. Authoritative contract changes

Implementation changes the following documents in the same commits as their code:

### `docs/design.md`

- Define the one byte-ingress rule and separate it from currentness, authority, and durability.
- Replace absolute type-enforced no-ambient-I/O language with the enforceable MFM-capability rule
  while retaining no ambient I/O as a coding obligation.
- Replace authored/expanded/certified execution authority with `ProgramDocument` and opaque
  `Program`.
- Keep `RunAdmitted` tenant-partition semantics while cutting the renamed access-reservation record
  and new program-document schema into a fresh store identity with new golden vectors.
- Define one deployment-supplied tenant per application facade and delete caller authentication,
  grants, policy decisions, and credential-bearing calls.
- State that caller admission/network exposure belongs to the trusted embedding and that the
  credential-free REST service is not a public trust boundary.
- Separate callback-free program/catalog authority from Runtime's live process registry.
- Make the process registry and every adapter/binding immutable for process lifetime; replacement is
  stop, drain, drop, and rebuild, with no generic live-currentness or revocation model.
- Replace the three-layer write comparison with `ResolvedEvent` and one reducer.
- Distinguish callback-free `QualifiedRun@H` from Runtime-branded affine `RunSession@H`; describe
  demand-triggered full resume, racing staleable sessions, exact-head compare-and-append, and direct
  successor advancement.
- State that spawning a new run opens no unrelated retained history and malformed dormant history
  is isolated to operations that actually consume it.
- State that no shared run/configuration semantic cache or durable reducer checkpoint exists.
- Apply the same local-construction/direct-commit versus independent-ingress distinction to
  configuration history.
- Replace mandatory post-reservation reread with direct positive commit acknowledgement,
  with the retained successor owned inside `ReadyToInvoke`'s Store observation continuation.
- State the trusted PostgreSQL epoch/immutability contract and the unchanged inability to prove
  rollback freshness after every independent anchor is lost.
- Define the minimum `DurabilityProfile` that `NewlyCommitted` must satisfy before provider entry
  and require store qualification to reject weaker PostgreSQL settings.
- Classify the append-atomic Effect-attention projection separately from execution authority and
  from the load-bearing dense fact frontier/publication routes.

### `docs/architecture.md`

- Update the runtime diagram and Program/State/Store responsibility rows.
- Move the complete callback-free program compiler into `mfm-program`; move live process assembly
  and invocation thunks into `mfm-runtime`, deleting superseded `mfm-certify` authority wrappers.
- Define `Pure | Read<C> | Effect<C>` as framework-issued capability classification.
- Remove claims that ordinary Rust closure purity is type-enforced.
- Remove comparison-token and complete-prefix-per-load architecture.
- Show one immutable registry assembly with least-authority views, private Runtime-branded affine
  sessions that may race at an exact head, one reducer, and purpose projections borrowing
  callback-free qualified evidence rather than acquiring callback authority.
- Delete the application access-control components and generic capability refresh/resource-authority
  components instead of leaving empty architecture boxes.

### `docs/run-execution.md`

- Change admission from app certification plus store recertification to trusted `Program`
  construction plus document persistence.
- Describe one resolved transition and locally retained successor.
- Distinguish new-run spawn from retained-run resume; new admission opens only explicitly selected
  historical dependencies.
- Describe full-prefix resume once, repeated direct affine successor ownership (including the
  reservation successor held inside `ReadyToInvoke` rather than a parallel session), and explicit
  resume after the session is dropped or loses the head comparison.
- Describe direct-new-commit-only creation of `ReadyToInvoke`, affine single consumption, and why
  cold/non-new outcomes cannot invoke.
- Describe the blocking recovery state for unobserved Effects, including `EntryOnce` manual
  attention and capability-specific bounded `EntryAbsorbing` reassertion.
- Describe typed configuration construction, direct writer promotion, and independent cold
  selection without a cache.
- Make positive append acknowledgement payloadless.
- Preserve reservation-before-invocation and observation-before-settlement.

### Other documentation

Update the READMEs for program, capabilities, certification, store, runtime, app, replay, CLI, and
REST, plus `docs/persisted-public-surfaces.md` and the EVM submission/wallet contracts. Document the
portable vNext, EVM identity-domain reset, renamed journal record, unchanged hash algorithm, new
goldens, and fresh store/wallet activations. Updating `docs/known-gaps.md` is mandatory: it must
record demand-time history qualification and failure isolation, the separate diagnostic
`audit_store`,
the absence of a durable semantic checkpoint, the PostgreSQL epoch/restore assumption, and the
unchanged inability to detect a self-consistent rollback after every outside anchor is lost. Delete
credential, principal, grant, policy-decision, live-refresh, and revocation documentation rather
than marking it legacy.

---

## 12. Verification plan

### 12.1 Tenant facade and persisted policy deletion

- Facade construction fixes exactly one `TenantScopeId`; no public application call accepts a
  credential or tenant override.
- Two facades sharing process/store internals can admit, read, drive, trace, replay, and export only
  within their own partitions. If tenant-wide Effect discovery is retained, listing is scoped by
  the same rule. Cross-partition run ids and recursive sources return the partition-local
  `RunNotFound` disposition, never a grant denial.
- A backend row returned inside one tenant lookup with a different `RunAdmitted` tenant fails as
  `InvalidHistory`, and a portable bundle with inconsistent tenant closure fails import; neither is
  downgraded to not-found.
- REST succeeds without an `Authorization` header, arbitrary headers cannot select a tenant, and no
  policy-only 401/403 response remains.
- CLI exposes no access-token option or credential file reader and does not replace it with a tenant
  selector.
- The portable vNext stream contains no principal, grant, or decision field; structural closure
  mutation is still caught by the whole-stream `ContentRef`, and the old format is rejected.
- `RunAdmitted` retains only tenant partition/provenance, not a principal or policy result.
- EVM submission identity is derived exactly from tenant, wallet nonce domain, and
  `SubmissionIdempotencyKey`; changing any component changes the vNext identity, and no principal or
  issuer-policy layer remains.
- No `EvmCallerSubmissionToken`, `IntentIssuerPreimage`, authenticated-issuer derivation, or
  policy-only issuer-namespace symbol remains; no old
  authorized-source helper name remains in replay/store APIs.
- Activation uses fresh run-store and wallet-domain identities. Reusing a physical sender requires
  an auditable proof that the old process drained, every prior allocation and Effect is terminal,
  the pending nonce is reconciled, and sender control is exclusive; otherwise the activation uses a
  fresh sender.
- Public API/error-schema and source-tree checks prove that the deleted access-policy types,
  credential plumbing, authenticated call wrappers, and policy-decision persistence have no
  compatibility aliases or inert replacements.

### 12.2 Program and capability construction

Compile-fail tests prove:

- `Program` cannot be field-constructed or deserialized;
- Pure cannot register Read/Effect callbacks;
- callbacks whose ABI differs from `S::Execution` cannot register;
- a hot `Read<C1>` registration cannot nominally associate a `C2` adapter or an Effect adapter;
- Effect cannot register Read callbacks;
- request, returned, and safe-failure ABI come only from `C`;
- Effect remains forbidden in FanOut;
- erased Pure/Read/Effect dispatch cannot select a wrong-mode method;
- no public MFM adapter invocation entry exists outside private `ReadyToInvoke`;
- no consumer can mint that package under the actual production dependency/feature graph; and
- the affine reservation-to-invocation package cannot be cloned or consumed twice.

API/source/schema-contract checks also prove that `PhysicalBindingAuthorization`,
physical-authorization semantic obligations/checker methods, every named refresh mode/evidence/
binding/resource/release family in section 10.3, current-binding selection, and the other deleted
generic currentness types have no definition, compatibility alias, persisted field, or call site.
Before deletion, a checked inventory maps every current physical-release certificate field and
consumer either to the exact surviving immutable binding descriptor/ingress proposition or to an
explicit deletion rationale. The cutover cannot land with an unmapped field or consumer.

Behavior tests prove:

- both construction paths converge on one private normalized-graph invariant owner, while
  source-specific typing and byte ingress perform only their own checks;
- typed construction and hostile document ingress produce equal `ProgramRef`, canonical
  `ProgramDocument`, normalized graph, schema/lexical indexes, and registry fingerprint;
- a bare `ProgramRef` cannot drive reduction or invocation, and the callback-free catalog view
  cannot invoke;
- cold ingress proves the normalized graph's frozen program-profile predicate; omitted source/trace
  metadata cannot create execution authority;
- hostile access-label, capability, component, and graph substitutions fail at ingress;
- assembly resolves every immutable descriptor to the exact adapter/provider/route/signer/effect
  domain once; missing/ambiguous/mismatched components reject before facade construction, while
  later calls increment no latest/current-generation check counter;
- recursive child-operation/state expansion, configuration specialization, fan-out, and
  pre/proceed/post/failure injection remain present and visible in the final normalized graph;
- planning consumes `ResolvedConfiguration<T>` without a second decode/validation pass;
- resume reconstructs the persisted final graph without rerunning expansion under current
  configuration/compiler behavior;
- the new Program document opens only in the fresh store namespace/identity and old/new histories
  cannot mix;
- hot Program admission invokes no document verifier; and
- each supported dynamic program shape remains within its frozen profile.

No test claims that an arbitrary Rust closure cannot perform ambient I/O.

### 12.3 Event and journal

- Golden `RunAdmitted`, new `ExternalAccessReserved`, H1/H2, portable vNext, EVM identity, and
  configuration-revision vectors freeze the one current wire shapes. Hash algorithm/domain
  separators that this RFC does not replace are checked independently from changed record payloads.
- Canonical fact-request/response goldens contain only access-reservation frontier terminology; no
  old authorization mode string remains accepted.
- Exact source/API absence checks reject `ExternalAccessAuthorized`, access `authorization_ref`,
  `RecordLogicalKey::Authorization`, `QualifiedRuntimeIntent::Authorization`,
  `PrimaryIntent::Authorization`, `AccessAuthorizationProposal`, `AuthorizationIntent`,
  `ExpectedAuthorization`, `CertifiedAccessAuthorization`, and `CommittedAccessAuthorization`;
  the strict decoder rejects the retired record tag and reference field. These checks are
  deliberately exact so provider protocol authentication terminology may remain.
- Path-scoped absence checks also reject coordination-only `AuthorizationEntry`, Runtime
  `Authorized<K, V>`, `AuthorizedCallOrigin`, and `AuthorizedProviderCall`, while explicitly
  retaining provider-protocol `EvmRpcAuthorization` and `EvmRpcInventoryFinishAuthorization`.
- Mutation of every predecessor/program/config/input/output/fact/request/observation/object field
  changes or invalidates the terminal commitment.
- Typed construction -> encode -> cold ingress produces the same `ResolvedEvent`; binding under the
  same complete fixation yields the same reduced state and head, while only the local side has a
  pre-commit `PreparedSuccessor`.
- Substitution of any store identity/epoch, predecessor, append id, tenant coordinate/frontier, or
  retained object in `AppendContext`/`RetainedBatchFixation` fails.
- Cold event ingress rejects every altered derived/assigned field and every surplus introduced
  object.
- A private constructor cannot pair a batch with a different prepared successor, obligation set,
  or index plan.
- Full resume at every valid prefix produces the same state as direct local advancement through
  those commits.
- No intent/recorded dual reducer remains for a test corpus to reconcile.

### 12.4 Session ownership, demand qualification, and concurrency

- `QualifiedRun` and `RunSession` cannot be deserialized or publicly constructed; `RunSession`
  cannot be cloned or used across store identity, epoch, tenant, run, program catalog, or private
  Runtime assembly identity.
- A callback-free purpose reader cannot invoke a Pure callback or seal its `QualifiedRun` into
  `RunSession`.
- Runtime rejects substitution of a qualified prefix/program from another exact catalog instance,
  `RuntimeAssemblyBrand`, or `ProcessRegistry` before any callback dispatch.
- A replacement assembly with a changed immutable binding cannot seal or drive an old program;
  reassembling the exact retained binding can resume it.
- A raw/deserialized `JournalHead` and an uncommitted `PreparedSuccessor` cannot construct qualified
  evidence or advance a session.
- Service/store startup performs zero retained-run and configuration-history reads.
- Spawning a new run performs zero unrelated retained-history queries.
- A malformed dormant run does not block startup or an unrelated spawn; explicitly resuming,
  reading, replaying, or exporting that run fails before typed authority or output is returned.
- A new run fails on a malformed program/configuration/producer/fact dependency it explicitly
  selects, while an otherwise equivalent run without that dependency does not open it.
- Explicit valid resume loads and folds the complete bounded prefix exactly once.
- Repeated directly committed steps on one session perform zero historical loads and reducer
  replays after the initial resume.
- Direct local advancement and a fresh full resume at the resulting head produce equal reduced
  state, object indexes, fact dependencies, and capacity accounting.
- Concurrent resumes may both create branded sessions at H. Exact-head compare-and-append admits
  only one prepared successor; a Plain/Observation winner rebuilds a session, while an access winner
  receives only `ReadyToInvoke` with that successor inside its Store continuation. Every stale or
  precondition-losing owner installs nothing, invokes zero times, and must resume explicitly.
- Dropping a session or restarting causes the next continuation to perform a new complete ingress.
- No global run-state map/LRU, suffix-refresh path, session serialization, or durable semantic
  checkpoint is written or consulted.
- Active-session and ingress-work limits reject excess work explicitly rather than silently evicting
  another execution's authority.
- A legitimate PostgreSQL restore uses a new store identity/epoch; same-epoch historical mutation is
  tested as a storage-contract violation.
- Store qualification rejects a durability profile that can return commit success before the
  access-reservation WAL is durable across its admitted failure domain, including `fsync = off` or
  `synchronous_commit = off`; stronger host-loss claims require their named synchronous quorum.

### 12.5 Append and external access

- The consuming coordinator's direct Plain/Observation `NewlyCommitted` branch rebuilds its exact
  session, while a reservation branch transfers the exact successor into
  `ReadyToInvoke`'s Store observation continuation, with zero head query, history read, decode, or
  reducer replay; the raw backend outcome never escapes.
- Before that branch, no type can reach the invocation thunk. Its owned
  `PreparedDrive::Access` is the only constructor input of `ReadyToInvoke`.
- Two in-flight appends with payloadless backend outcomes cannot transpose Runtime
  `SessionContinuation`s, Store `PreparedReservationAppend`s/successors/index plans, or invocation
  thunks; private constructors and coordinator API shape make such a test fail to compile.
- An access-reservation `Found` classified as `ExistingSame` installs no candidate session and
  releases no invoker.
- Access-reservation `StaleHead`, `AcknowledgementUnknown`, invalid found bytes, capacity failure,
  and typed `AppendConflict` invoke zero times; no unrelated append releases an invoker.
- `Found` under the same append identity with well-formed different bytes returns `AppendConflict`
  after exactly one stored-attempt comparison.
- `AcknowledgementUnknown` quarantines the exact preparation and installs/invokes nothing. If the
  original append exists, resolution returns found-same and invokes zero; if it is proven absent at
  the same predecessor, resubmitting that quarantine can return direct new and invokes exactly once;
  dropping it or cold recovery invokes zero.
- Every framework invocation has a distinct directly committed `ExternalAccessReserved`, and its
  `ReadyToInvoke` is consumed at most once; a crash may leave zero invocations.
- Observation recovery may retain the same `reservation_ref` but never recreates or reuses its
  consumed invocation package and never invokes that ordinal again.
- An observation that loses the head comparison performs at most one reducer rebase per explicit
  `SuspendedRun::resolve`. If another head race occurs, Store returns the same qualified response
  event in a new `ObservationRebaseInput` paired with the Runtime shell for another explicit,
  work-bounded resolution; there is no hidden retry loop. Still-selected appends,
  already-recorded idempotency, no-longer-selected late response, and conflicting-observation
  branches are distinct and invoke zero times.
- Observation `AcknowledgementUnknown` quarantines Store's exact
  `UnresolvedObservationAppend` beside the original `SessionContinuation` and settles nothing.
  Tests cover exact found-same followed by qualified settlement, proven-absent direct retry,
  absent-plus-advanced-head rebase, repeated unknown, and conflict/invalid failure; every branch
  invokes zero times and validates response bytes zero additional times.
- Only a direct committed selected observation can settle state. Already-recorded settlement comes
  from ordinary qualified resume, a late non-selected response never settles, and a different
  observation for the same attempt fails closed.
- `ReadyToInvoke` cannot be paired with another same-`C` request, state input, run, occurrence,
  process registry, adapter implementation, immutable binding, attempt ordinal, or reservation
  record.
- The special prior-run fact-scan continuation remains store-minted, one-use, and
  direct-new-commit-only.
- A cold process cannot invoke an outstanding reservation. Read repetition uses a new ordinal;
  `EntryOnce` Effect recovery parks as `PossibleEntry`; and bounded `EntryAbsorbing` reassertion
  requires its explicit duplicate-absorption contract plus the exact same immutable binding/effect
  domain.
- A slow old `ReadyToInvoke` racing a new Read/`EntryAbsorbing` ordinal may cause both distinct
  attempts to invoke, but only under that capability's repetition/absorption contract. The attempt
  budget remains bounded and exhaustion parks; `EntryOnce` never creates the second ordinal.
- Two workers racing from one head yield one direct reservation winner/`ReadyToInvoke`; every
  `ExistingSame`, stale, conflict, invalid, or acknowledgement-unknown path invokes zero times.
- The immutable `ProcessRegistry` exposes no replace, revoke, lease, refresh, current-binding, or
  provider-entry-freshness API. Stop/drain/rebuild is tested at the embedding lifecycle boundary;
  it resolves or deliberately parks every ambiguous reservation/observation package, and forced
  shutdown never manufactures a “did not enter” disposition.
- Same-binding protocol-token renewal remains private adapter behavior and cannot change the MFM
  binding/provider/effect identity; any such identity change requires process rebuild.
- Every provider success/failure response crosses one bounded adapter ingress into opaque
  `C::Returned`/`C::SafeFailure`; settlement performs no second wire/schema validation.
- A response that fails capability/request binding is rejected before
  `AcceptedAccessResponse` can consume `ObservationWriteContinuation` and therefore appends no
  observation. Settlement only interprets an already-valid observation against state input; it
  cannot invalidate the persisted capability relation.
- Observation is committed before state settlement succeeds.
- PostgreSQL snapshot, atomicity, restore/epoch, exact-head compare-and-append contention, numeric
  ordering, malformed-row, and fresh-process continuation tests remain.

### 12.6 Facts and purpose projections

- A selected fact dependency is qualified once within the consuming session/load scope.
- A previously qualified exact producer head is reused within that scope; an independent resume is
  a fresh ingress.
- Changed producer heads, cycles, tenant boundaries, dense-publication gaps, missing routes, and
  frontier mismatches fail.
- A run with no selected prior-run dependency opens no unrelated producer history.
- Offline export cannot borrow producer closure from process memory that is absent from the bundle.
- Public/trace/replay/export projections derived during one session cause no second semantic fold
  while preserving output redaction.
- Ordinary readiness performs no history scan; `audit_store` reports dormant run, configuration,
  and fact-frontier corruption without changing readiness or creating invocation authority.
- When Effect-attention inventory is enabled, append, head advance, and
  `needs_effect_attention` update are atomic; the partial index is snapshot-complete under the
  PostgreSQL contract, and one-snapshot listing fails explicitly before exceeding the fixed
  materialized count or canonical-byte bound.
- When enabled, Effect-attention listing invokes zero callbacks. Any selected recovery action
  explicitly resumes and qualifies the named run before acting.

### 12.7 Configuration history

- Typed/local configuration construction increments no ingress-validation counter.
- Hostile file/environment/API and database revisions are bounded and admitted exactly once.
- Secret-bearing configuration is consumed at assembly only into concrete immutable
  adapter/provider/signer internals; secret canaries never appear in revision bytes, history,
  exports, diagnostics, or a generic callable resource handle.
- Full independent configuration ingress produces the same exact head/value as direct local writer
  advancement at every revision.
- The consuming configuration coordinator's direct `NewlyCommitted` branch advances only its owned
  prepared successor with no echo/readback; concurrent raw outcomes cannot be transposed.
- Found-same, conflict, stale, and unknown acknowledgement behavior preserves idempotency and never
  installs an uncommitted successor.
- A later independent configuration selection performs one complete bounded ingress; no shared
  configuration cache, suffix refresh, or checkpoint is consulted.
- Malformed dormant configuration does not block readiness or a new run that does not select it;
  selecting it fails before planning receives `ResolvedConfiguration<T>`.
- A changing active-revision selection is checked as head/activation currentness, not by
  revalidating the selected immutable value.
- Wire/schema ingress acceptance and a state's contextual interpretation/settlement are tested as
  different propositions; the latter is not mislabeled duplicate byte validation.

The performance acceptance criterion is measured work and load counts, not an unsupported global
asymptotic claim. Before implementation, the product owner fixes the maximum supported retained
prefix and a cold-resume latency SLO, or explicitly records that no latency SLO exists beyond
bounded completion. A benchmark of that maximum prefix must satisfy the ruling; a miss blocks this
no-checkpoint target and requires a separate checkpoint decision.

---

## 13. Logical commit sequence

Every commit updates affected contracts, tests, fixtures, and public documentation. No old/new API
pair, compatibility decoder, or fallback survives its cutover commit.

1. **`scope application facades by tenant`**

   Give `Application` one construction-time `TenantScopeId` and cut the app, CLI, REST, backend
   facade calls, public errors, tests, and documentation to credential-free tenant-scoped APIs.
   Delete policy/principal/grant/decision persistence in the same commit: introduce portable vNext
   structural closure, direct EVM submission identity, and fresh export, run-store, and wallet
   identities. Remove every old field, decoder, error, policy callback, and compatibility alias
   together. Stage but do not activate the fresh identities until the complete migration train
   passes; reuse a sender only when the auditable drain/terminal/reconciled/exclusive-control gate
   passes.

2. **`make program and state semantics valid by construction`**

   Land the final capability/State/lowering ABI, valid-by-representation values, typed expansion,
   `ProgramDocument`, opaque branded `Program`, and the ordered two-stage construction surface:
   callback-free `ProgramCatalogBuilder::finish`, followed by
   `RuntimeAssemblyBuilder::new` with that exact catalog instance. The latter produces one immutable
   `RuntimeAssembly` with its private non-cloneable registry and distinct Runtime brand. Move the
   sole compiler into `mfm-program`, delete Spec and all duplicate Program authorities, and leave
   Certify only as the one temporary live physical-currentness owner until step 4. Introduce no
   old-ABI catalog, parallel compiler, `ProgramRef` resolver, or Program cache/intern map in either
   stage or any downstream crate.

3. **`make store appends one semantic path`**

   Move callback-free history semantics to Store; split read-only `QualifiedRun` from affine active
   continuation; introduce one `ResolvedEvent`/reducer/binder/prepared successor path; delete dual
   reductions, semantic comparisons, local requalification, and positive echoes. Cut Memory/
   PostgreSQL to complete-frame append mechanics, exact durability/epoch, and a fresh schema
   identity. Land the U3 choice as one indivisible slice in that schema cut: when enabled, include
   the append-atomic column, partial index, bounded snapshot-complete listing DTO/API, limits, and
   backend/conformance tests; when absent, include none of them. Do not stage only the projection or
   only the public API. In the same infrastructure cut, migrate the configuration backend off
   target-authority/validated-append seals; the sole read-side semantic ingress and admission bridge
   land in step 4, while writer promotion completes in step 6. Retain exactly one old access
   coordination wire until step 4.

4. **`reserve external access through affine run sessions`**

   Rename the journal to reservation terminology and land exact Program/fact binding schemas,
   affine `RunSession`/prepared drive/direct successor advancement, request-bound fact mode and
   completions, `ReadyToInvoke`, response ingress, observation ambiguity/rebase, and exhaustive
   owner-carrying errors. Before cutting App/Runtime spawn, also land the read-side configuration and
   admission chain it consumes: one bounded independent configuration-history ingress returning
   `ResolvedConfiguration<C>`, consuming `ConfigurationAdmissionEvidence<C>`, the published typed
   entry-point catalog and configuration-planner registry, and the owner-bound typed admission
   input passed to Runtime. Delete purpose-specific configuration replay, shared cache, and suffix
   selection paths with that bridge; the old writer may continue emitting the unchanged revision
   wire mechanically until step 6. Replace App `drive_once` with typed planner/admission plus
   session/executor ownership only after this chain exists. Freeze immutable bindings and delete
   live refresh/release/revocation/currentness, old proof wrappers, `mfm-certify`, and
   `mfm-authority-seal` only after the U6 proposition map and every structural replacement is
   present.

5. **`qualify retained history only when consumed`**

   Unify bounded complete-prefix resume, read, replay, export, audit, and producer-closure ingress
   behind the one event/reducer path. In this demand/open cut, remove both the eager run-history and
   eager configuration-history readiness scans: ordinary startup and unrelated new-run spawn read
   and qualify neither retained family. This readiness cut lands before step 6 changes configuration
   writer commit semantics. Purpose projections borrow callback-free evidence from an active session
   or explicitly qualify an ephemeral `QualifiedRun`; the dense fact frontier remains load-bearing.
   Consume the already-settled Effect-attention projection only if tenant-wide discovery is a
   product surface. Add diagnostic `audit_store`; add no shared semantic cache, suffix-refresh
   protocol, attention rebuild authority, or reducer checkpoint.

6. **`make configuration commits one prepared-successor path`**

   Complete only the writer-side cut: introduce private prepared configuration successors and make
   a consuming configuration coordinator keep the raw backend outcome bound and advance its active
   owner with no readback. Preserve exact-head compare-and-append, idempotency, acknowledgement
   ambiguity, and the revision wire/hash format; delete local writer revision revalidation and echo
   comparison. The sole read-side `ResolvedConfiguration<C>`/planner/admission path already landed
   in step 4, and ordinary readiness already stopped scanning configuration history in step 5; do
   not add a second reader, cache/suffix path, or readiness scan here.

Do not defer documentation or the test that establishes a replacement invariant until after the
old guard has been deleted.

---

## 14. Acceptance criteria

The refactor is complete only when all of the following are true:

- Deployment contracts make the trusted embedding responsible for caller admission and exposure;
  credential-free REST is never presented as safe for direct untrusted/public reachability.
- `Application` is fixed to one deployment-supplied tenant; public calls accept no caller
  credential, principal, grant, policy-decision reference, or tenant override.
- The app, CLI, REST, errors, tests, and documentation contain no internal end-user authentication
  or authorization model. Multiple tenants are exposed only through separately constructed facades.
- Portable export contains only the bounded structural same-tenant closure and one whole-stream
  content fixation; no principal/grant/decision evidence or compatibility decoder remains.
- EVM submission intent is derived directly from tenant, wallet nonce domain, and submission
  idempotency key under the new domain; no authenticated-issuer layer remains.
- The EVM cutover uses fresh run-store and wallet-domain identities and either a fresh physical
  sender or auditable proof of complete old-process drain, terminal allocations/Effects, reconciled
  pending nonce, and exclusive sender control.
- No public-field or deserializable value is accepted as execution authority; only opaque `Program`
  is.
- A bare `ProgramRef` is never authority, and callback-free store/replay code cannot invoke.
- A hot typed program is globally checked once and is never recertified by store or Runtime.
- Cold/imported program bytes pass one ingress and produce an equivalent `Arc<Program>` with the
  same reference, normalized graph, indexes, and catalog identity.
- Pure recursive operation expansion, configuration specialization, and injected
  pre/proceed/post/failure states remain part of construction and are completely represented in the
  final normalized `Program`; resume never reruns them.
- Changed program, access-reservation, portable, and EVM schemas start under their declared fresh
  identities/activations. `RunAdmitted` retains tenant partitioning, the recursive journal hash
  algorithm remains an exact-prefix commitment, and new golden vectors freeze the one current wire.
- Every state has exactly one type-level `Pure`, `Read<C>`, or `Effect<C>` declaration.
- MFM gives state callbacks no transport, signer, store, or generic live capability.
- Adapter-owned dependencies remain adapter TCB internals rather than callable nested framework
  capabilities; their immutable secret-free identities remain represented, and Read classifies
  whole-adapter behavior.
- Every deleted physical-release certificate field and consumer is accounted for by a checked
  inventory mapping it to a surviving immutable descriptor/ingress proposition or an explicit
  deletion rationale; no unmapped or inert currentness evidence remains.
- `ProgramCatalog`, `ProcessRegistry`, adapters, provider bindings, and signer/transports are
  immutable for a composed process lifetime. No generic replace/revoke/lease/refresh/entry-freshness
  API, metadata, persisted disposition, or test remains; binding replacement is stop, drain, drop,
  and rebuild.
- A changed binding never silently reinterprets a retained program; the exact old binding assembly,
  terminal/read-only disposition, or a separately specified migration is required.
- The documentation does not claim arbitrary Rust closures are sandboxed or provably pure.
- Intent and recorded bytes converge before one reducer; no dual semantic implementation remains.
- The current journal hash format is frozen by golden tests and documented as an exact-prefix
  commitment.
- `QualifiedRun` is callback-free semantic evidence. `RunSession` is opaque, Runtime-assembly
  branded, affine, non-serializable, and non-cloneable; no purpose reader, raw head,
  foreign assembly, or uncommitted successor can construct or advance drive authority.
- Store/service startup qualifies no retained run or configuration history.
- Spawning a new run opens no unrelated history and succeeds despite a malformed dormant run;
  consuming a malformed selected dependency fails locally before typed authority is returned.
- One explicit resume folds one complete bounded prefix once. Every consuming coordinator's direct
  committed branch then retains its exact affine successor with zero head query, reload, decode, or
  replay: Plain/Observation rebuild a session, while reservation places the active successor only
  inside `ReadyToInvoke`'s Store continuation.
- Multiple sessions may race from one head; exact-head compare-and-append advances only the direct
  winner, while every stale/non-new/indeterminate session installs nothing and invokes zero times.
- No shared run/configuration semantic cache, LRU, suffix-refresh protocol, or durable reducer
  checkpoint exists.
- The owner has fixed the maximum supported retained prefix and either a cold-resume latency SLO or
  an explicit no-latency-SLO ruling; the maximum-prefix benchmark satisfies that contract before
  the no-checkpoint session design is accepted.
- Only a consuming `PreparedDrive::Access` coordinator call whose exact
  `ExternalAccessReserved` append is directly new can create `ReadyToInvoke`; the raw backend
  outcome never escapes or pairs with another preparation, and no cold/non-new outcome can invoke.
- No consumer-callable invocation mint exists under the production feature graph, and exact
  same-`C` request/state-input/run/occurrence/registry/adapter/binding/attempt/reservation
  substitution is impossible.
- Reservation found-same, stale, unknown, invalid, capacity, conflict, regression, and fork paths
  release no invoker; unrelated append failures release no new invoker or cause an additional
  invocation.
- Observation recovery never recreates a consumed `ReadyToInvoke`. A cold unobserved `EntryOnce`
  Effect parks; a duplicate-absorbing reassertion uses a new bounded ordinal only under the exact
  capability and immutable binding/effect-domain contract.
- A stale Store-owned `ObservationRebaseInput`, paired only with Runtime's original
  `SessionContinuation`, can perform one explicit bounded rebase unit, never reinvokes, admits at
  most one stored observation per attempt, and distinguishes selected, already-recorded,
  no-longer-selected, and conflicting outcomes. No caller-supplied or independently resumed session
  can be transposed into that recovery.
- Every external invocation is reserved durably first, and every returned response crosses byte
  ingress once. Only a selected observation may settle state, and that exact observation is
  committed before settlement; a late non-selected response never settles.
- Configuration history has the same local-construction/cold-ingress/prepared-commit discipline and
  no independent writer/reader/readiness replay paths. Its consuming commit coordinator also keeps
  payloadless backend outcomes bound to the exact prepared configuration successor.
- The dense fact frontier remains load-bearing completeness evidence. If tenant-wide Effect
  discovery remains, its Boolean projection and partial index update append-atomically but never
  create execution authority.
- Purpose readers borrow callback-free evidence from one active session or explicitly qualify one
  ephemeral `QualifiedRun`; no purpose can drive or owns another semantic verifier.
- Provider response bytes cross one adapter ingress into opaque capability values; downstream code
  does not confuse protocol validity with factual truth beyond the capability's provider/evidence
  contract.
- PostgreSQL's truthful outcome, immutable-row, durability, sealed-writer, and store-epoch contract
  is explicit; `NewlyCommitted` satisfies the qualified crash-durability profile before any provider
  entry, and legitimate restore rotates the identity/epoch.
- Intrinsic validators have moved to opaque construction/ingress; contextual head/frontier and
  domain transaction checks have explicit owners; redundant shadows are deleted.
- Safe-failure values are capability-specific valid types; Effect entry/ambiguity/absorption,
  dependency identities, domain operation keys, and the reserved fact scanner remain represented
  without generic refresh/resource-lineage machinery.
- The final report measures product-code and public-type deltas and itemizes every new concept or
  net addition; no increase is justified merely as scaffolding for a later cleanup.

When these conditions hold, MFM will rely on the properties it already built: Rust construction,
content identity, an exact recursive journal commitment, atomic exact-head compare-and-append, and
typed one-use invocation coordination. It will stop paying every internal layer to distrust the
preceding one.
