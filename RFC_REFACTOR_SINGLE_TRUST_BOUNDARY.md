# RFC: refactor to a single byte-ingress trust boundary

Status: draft for architectural review

Relationship: this is the proposed replacement for `rfc_single_trust_boundary.md`. The old RFC
remains review material until this proposal is accepted; implementation must leave only one current
contract and must not retain compatibility paths between them.

---

## Decision

MFM has one rule for immutable data trust:

> Bytes are authenticated where applicable, bounded, strictly decoded, canonicalized, and
> semantically validated exactly once when they enter the process trust base. Successful ingress
> returns an opaque immutable value that carries that evidence. Crossing an internal crate or module
> boundary does not erase it, and no downstream layer reconstructs or revalidates the same fact.

This is a data-ingress rule, not a claim that one process owns every form of authority. Caller
authorization, current leases and fences, exact-head comparison, durability acknowledgement,
redaction, and secret lifetime are different propositions with explicit owners.

The target design has a small set of load-bearing values and no generic validation layers between
them:

```text
ProgramDocument bytes -- one ingress --> Program

configuration source/row bytes -- one ingress --> ResolvedConfiguration<T>@G

raw run prefix -- one cold ingress + fold --> QualifiedRun@H

QualifiedRun@H + RuntimeAssemblyBrand + RunDriverPermit --> RunSession@H

typed proposal -----------------------> ResolvedEvent
qualified record bytes ---------------> ResolvedEvent

RunSession@H + ResolvedEvent
    -> PreparedAppend { batch, successor: PreparedSuccessor@H' }
    -> exact-head compare-and-append
    -> NewlyCommitted -> RunSession@H'
```

New-run spawn follows the same preparation/commit path from locally typed admission inputs and an
absent predecessor. It does not cold-load unrelated retained runs; only explicitly selected
program, configuration, or prior-run fact evidence is a dependency of that admission.

`Program` is valid by construction. `QualifiedRun@H` is callback-free semantic evidence for one
exact journal prefix. Only Runtime can combine it with the exact assembly brand and a fenced,
affine per-run driver permit to create `RunSession@H`; purpose readers never receive drive
authority. `ResolvedConfiguration<T>@G` carries the analogous typed configuration evidence.
`ResolvedEvent` is the sole run-semantic input. `PreparedAppend` binds the bytes and
not-yet-authoritative successor that were derived together. Only the direct `NewlyCommitted`
outcome advances the session to that successor. It does not cause the process to distrust and
reconstruct its own values.

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

1. **Session ownership and cold-resume latency**

   - **Choice:** an active driver owns one Runtime-branded affine `RunSession` and its externally
     fenced per-run permit; there is no global run-state LRU or persisted semantic checkpoint. A
     genuinely new resume qualifies and folds the bounded complete prefix once, then must acquire
     that permit before it becomes drive authority.
   - **Why uncertain:** the current public one-action API encourages independent
     `drive_once(run_id)` calls, and worst-supported cold-resume time has not been measured.
   - **If wrong:** stateless calls routed across workers may repeatedly replay a large prefix and
     miss the product latency target.
   - **Resolution:** define how long a driver may retain a session and benchmark maximum supported
     histories. Consider a separately designed checkpoint only if an explicit cold-resume SLO is
     missed.

2. **Revocation completion semantics**

   - **Choice:** driver/resource revocation prevents new provider entries after its linearization
     point; work admitted before that point may drain.
   - **Why uncertain:** the product does not say whether “revoked” also means every previously
     admitted call has completed.
   - **If wrong:** revocation may report success while pre-cutover work remains in flight.
   - **Resolution:** define whether each revocation API waits for admitted work to drain.

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
     ids. Keep the atomic projection only for that contract.

4. **Provider factual trust**

   - **Choice:** typed configuration binds capability `C` to a concrete `ProviderBinding<C>` that is
     part of that capability's TCB unless `C` explicitly requires cryptographic or quorum evidence.
     Response ingress proves protocol, authenticity, and request binding, not factual truth.
   - **Why uncertain:** current contracts sometimes use “verified” for both propositions.
   - **If wrong:** a well-formed, authenticated but false response may be treated as objective fact.
   - **Resolution:** classify the factual-trust requirement of every production capability and
     narrow its returned type to the evidence actually established.

---

## 1. Scope

This RFC owns one cutover across typed program construction, state capability declarations,
configuration history, journal reduction, run spawning/resumption and session ownership, append
acknowledgement, and internal read projections.

It deliberately excludes:

- production provider topology, key custody, endpoint authentication, and binary assembly;
- a public background scheduler or product workflow redesign;
- compatibility with the current persisted program representation;
- rollback resistance after all independent memory of a later head is lost; and
- sandboxing or static analysis of arbitrary linked Rust code.

Those are separate product or deployment decisions. Mixing them into this refactor would make the
single-ingress rule depend on unrelated authority choices.

The current journal hash algorithm and `RunAdmitted` serialization remain unchanged. The new
program-document schema changes program content references, so deployment initializes a fresh
namespace, `StoreScopeId`, and writer epoch rather than mixing or rewriting chains in place. This is
a store-identity cutover, not a journal hash-version or in-store chain reset. Candidate, record, and
commit digest domains remain byte-for-byte unchanged. The configuration revision
content/predecessor format also remains unchanged; its qualification and commit ownership change,
not its wire identity.

---

## 2. Trust model

### 2.1 The process trust base

The process TCB includes:

- compiled MFM crates and their dependencies;
- registered application state callbacks;
- concrete adapters, signers, transports, and storage implementations;
- the Rust compiler/toolchain used to build them; and
- process assembly that selects the exact registry and configuration issuers.

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
  -> authentication and request/response binding, where applicable
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
synchronous replica/quorum acknowledgement. MFM releases no provider entry permit under a weaker
profile. A settings change that weakens the qualified profile invalidates the store epoch rather
than silently changing `NewlyCommitted` semantics.

An actively lying DBA or storage service is outside this threat model. If that adversary is added,
neither immediate readback nor repeated replay against the same service supplies an independent
witness.

`NewlyCommitted` for the exact sealed append advances the already-proved in-process successor. It
causes no readback. A restore or replacement starts a new store identity/epoch; a self-consistent
older database is never silently presented as a continuation of the same epoch.

The store writer authority also owns one cross-process, fenced driver permit per active run. Permit
acquisition/revocation and writer-epoch transfer are linearized by that authority. The permit is
carried by `RunSession`, checked by every append, and consumed or rechecked by the MFM access-entry
owner immediately before provider I/O. Thus a separately reconstructed semantic prefix is not drive
authority, and a fenced former driver cannot use a previously committed authorization after a new
driver wins.

The entry operation is explicit: `RunDriverAuthority::enter(generation)` either returns one affine
`DriverEntryPermit` and registers that call as admitted, or rejects before provider I/O. Driver
transfer/revocation serializes with `enter`; after transfer wins, no old generation can enter. What
transfer waits for already-admitted calls is the revocation-completion choice in Material
uncertainty 2. This may require a lightweight currentness interaction with the authority, but it
never reloads or revalidates journal history.

On a later process start or explicit resume, retained rows are nevertheless bytes, because Rust
construction evidence did not survive serialization. They cross ingress and fold once before
becoming `QualifiedRun`; Runtime must still bind its exact assembly brand and acquire the driver
permit before sealing a `RunSession`. This is reconstruction of lost process evidence, not
posterior skepticism about the value that originally produced the rows.

### 2.4 Checks that are not byte validation

| Proposition | Owner | When checked |
| --- | --- | --- |
| Caller may act for tenant | Application access policy | Each public call |
| Observed journal head is current at that snapshot | Backend snapshot head read | At resume or latest-read |
| Prepared append extends the current journal head | Exact-head compare-and-append | At commit |
| Store writer epoch is current | Writer-epoch fence check | At append |
| Caller owns this run's drive authority | Run-driver permit owner | At session seal, append, and access entry |
| Required tenant fact frontier is current | Exact-frontier barrier or compare-and-publish | At dependent append |
| Capability resource is current | Resource lease/fence owner | Authorization or adapter entry |
| Append is durably acknowledged | Qualified backend | Before releasing any MFM-mediated invoker |
| Nonce/operation key is unique | Resource authority transaction | At reservation/mutation |
| Public output contains no secret | Public DTO/render boundary | Before emission |

These checks remain because their facts are contextual or can change. They must be named after the
property they establish, not hidden behind a generic `validate` layer.

### 2.5 Disposition of current check classes

| Check class | Target disposition |
| --- | --- |
| External byte bounds, decode, canonicalization, authentication | Keep at the owning ingress |
| Private constructor and global invariant check | Keep once at construction |
| Head/frontier concurrency, leases, fences, revocation, caller policy | Keep at the owning authority |
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
expansion/lowering, normalized-graph and policy validation, manifest and document construction,
schema/lexical indexes, and the private graph-invariant pass.

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
ambient I/O. Live provider handles, credentials, and secrets remain in `ProcessRegistry`; the
program contains only secret-free binding identities.
Live process assembly, invocation thunks, and their private permit transition move into
`mfm-runtime`. Remaining non-authoritative helpers move to their `mfm-program` or `mfm-runtime`
owner, and the superseded `mfm-certify` crate/authority wrappers are deleted rather than retained as
a facade.

`Program` has private fields, no `Deserialize`, no public struct literal, and no unchecked public
constructor. It is cheaply shared through `Arc`. A `ProgramRef` is an address, not semantic or
invocation authority: persisted records carry the ref, the callback-free catalog alone resolves it
to the exact `Arc<Program>`, and semantic consumers accept that resolved value. They never accept a
document or bare ref and then pretend it is already valid.

`ProgramRef` is the domain-separated hash of the complete canonical `ProgramDocument`. Any
catalog-local interning key also includes the frozen registry fingerprint, so equal nominal
component names under different assemblies cannot alias.

`Program` retains the normalized executable graph, canonical document/reference, value-schema and
lexical-slot indexes, entry signature/profile/policy coverage, and permitted
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
- satisfaction of the normalized executable graph's frozen entry/profile/policy language;
- exhaustive Match and bounded FanOut;
- failure routing and one total root outcome;
- unique identities and complete graph/component closure;
- every graph/profile construction, depth, and count bound plus an explicit canonical-document byte
  bound owned by ingress/frozen entry policy; and
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

### 3.4 Registry ownership

One registry assembly owns the invariant implementation and produces two non-overlapping products:

```text
RuntimeAssemblyBuilder::finish()
    -> ProgramCatalog       // callback-free construction and hostile-document ingress
    -> ProcessRegistry      // non-cloneable live callback/adapter authority for Runtime
    -> RuntimeAssemblySeal  // private proof that the two views came from one assembly
```

`ProgramCatalog` owns:

- typed state and capability registrations;
- frozen entry profiles and construction bounds;
- complete callback-free recipe expansion/lowering, policy/graph validation, manifest/index/document
  construction, and hot DSL completion; and
- hostile `ProgramDocument` ingress.

Runtime-private `ProcessRegistry` owns the exact callbacks and adapters selected for invocation.
`RuntimeAssemblyBuilder`, `ProcessRegistry`, `ErasedInvocationThunk`, `PreparedAccess`,
`CommittedAccessPendingEntry`, and `EnteredAccess` are all defined in the same `mfm-runtime` crate;
no friend-crate visibility or reversed dependency is required. `Program` binds their immutable
secret-free identities, never their live handles. Store and offline replay receive
the callback-free catalog only; Runtime receives the live registry. Both products carry the same
private registry identity established once at assembly, without duplicating program-validation
logic. Runtime alone receives the pairing seal, registry, and private invocation thunks.

Runtime resolves a program semantically through `ProgramCatalog` and resolves its implementation
identities separately through `ProcessRegistry` only at execution. Cloning `Arc<Program>` therefore
cannot clone an invoker or grant the store/offline tooling access to a callback.

This replaces the separate authoring certifier, persisted verifier, borrowed entry certifier,
duplicated verification snapshots, and later root/document equality checks. The catalog may intern
`Program` by exact `ProgramRef`; nominal entry-point identity alone is never an identity key.

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
cutover. Relations that inherently join a response to its request remain one check at response
ingress or settlement, because that is a new contextual proposition rather than revalidation of
either value in isolation.

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
- place Effect in a FanOut policy that admits only Pure/Read; or
- invoke an adapter without the exact typed authorization.

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

Dynamic programs require internal type erasure, but erasure is not permission. `mfm-runtime` owns a
crate-private `PreparedAccess` containing the exact access kind, capability identity, request, run,
program occurrence, private Runtime assembly brand, driver generation, process-registry identity,
adapter implementation, physical binding/lineage, durable authorization intent, driver/resource
entry requirements, and an inaccessible existential one-use invocation. It contains no entry
permit. Only the data-only authorization intent and secret-free currentness-requirement metadata cross the injected
`QualifiedHistoryPort`. That port is an opaque store product paired through `RuntimeAssemblySeal`;
application code cannot install a raw/forged port into Runtime.

The history port returns `NewlyCommitted` only for the direct commit. A crate-private Runtime
transition consumes that result together with the still-private `PreparedAccess` and produces
`CommittedAccessPendingEntry`. Runtime then asks the driver/resource owners to enter. Only their
positive post-commit outcomes construct `EnteredAccess`, which alone contains the affine entry
permits and can reach the invocation thunk. None of these constructors or handles is public under
any Cargo feature. The store never constructs or returns the invocation package. The erased
dispatcher preserves the same tuple when selecting the registered adapter.

Every durable authorization field and attempt-id preimage remains. The cutover deletes only the
copied `ExpectedAuthorization` shadow and repeated comparison after the private package makes that
mismatch unrepresentable. It also preserves Effect refresh mode/evidence, resource lineage, entry
mode and key contract, semantic adapter dependency, access-fault contract, and the store-minted
one-use prior-run fact-scan permit. The non-zero `MAX_ENTRIES` bounds `EntryUnknown` re-entry;
`EffectRefreshMode` and its evidence govern `SupersededBeforeEntry` refresh/attempt ordinals on a
separate axis.

Public unqualified `ReadAdapterInvoker`/`EffectAdapterInvoker` entry points and any `invoke(None)`
fallback are deleted. Adapter registration captures the concrete adapter value/closure immediately
into a non-cloneable private `ErasedInvocationThunk` inside `ProcessRegistry`; the current public
cross-crate `Qualified*PhysicalBinding::invoke_authorized` surface disappears. Runtime can reach the
thunk only by consuming `EnteredAccess`. Trusted adapter code may still call its own internal
client, but MFM exposes no invocation path that bypasses the committed package.

The framework does not create a second nested capability system for adapter internals. Callable
signer/resource process handles are deleted; concrete adapters own their transports, signers, and
resource clients as TCB internals. Secret-free dependency identities, resource lineage, and live
lease/fence obligations remain represented for audit and currentness. `Read<C>` classifies the
behavior of the whole trusted adapter, including those internals. This is a reviewed TCB assertion:
because Read may be retried or fanned out, code that may consume or mutate externally meaningful
state must be `Effect<C>`.

An active `RunSession` never freezes live revocation. Program and ordinary process bindings are
immutable for one process lifetime. When a capability promises live revocation, its resource owner
serializes revocation against consumption of the affine entry permit immediately before provider
I/O or key access. If revocation linearizes first, entry is refused; if permit consumption
linearizes first, the already-admitted call follows the published drain contract. Historical
program/run validity never substitutes for that changing entry decision.

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
from opaque `QualifiedRun`; drive authority additionally requires the Runtime brand and live
driver permit described below.

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

`QualifiedRun@H` is opaque callback-free semantic evidence, scoped to one store identity/epoch,
tenant, run, program-catalog fingerprint, and exact `JournalHead`. It owns the `Program`, qualified
immutable context, reduced state, retained-object index, fact dependencies, outstanding access
state, and cumulative capacity accounting required to project or continue that prefix.

`RunSession@H` is an opaque, non-serializable, non-cloneable Runtime drive package:

```text
RunSession@H {
    qualified: QualifiedRun@H,
    assembly: private RuntimeAssemblyBrand,
    driver: affine fenced RunDriverPermit,
}
```

Purpose readers may borrow callback-free `QualifiedRun` internally and return a DTO. They cannot
seal a `RunSession`, extract the assembly brand, reach `ProcessRegistry`, or invoke even a Pure
callback. Runtime seals a session only after proving that the program/catalog identity belongs to
its exact private `RuntimeAssemblySeal` and after acquiring the per-run permit. Every callback
dispatch requires that brand structurally; `PreparedAccess` adds the exact adapter/binding identity
for Read/Effect.

`QualifiedRun` can arise only from complete-prefix ingress/fold or a local prepared successor after
direct commit. `RunSession` can arise only from Runtime-branded spawn/resume or direct advancement
of the same session. A raw/deserialized run id, head, `ProgramRef`, reducer snapshot, or
callback-free qualified value cannot create drive authority.

### 7.1 Active ownership, not a cache

The Runtime core exposes an explicit session lifecycle. An in-process driver qualifies and acquires
once, then owns the affine branded session across deterministic drive steps until terminal
completion, an external wait, or explicit release. Driving consumes and returns the session, or
mutably advances it under equivalent exclusive ownership. The proof is not thrown away after each
internal step.

There is no shared semantic run-state LRU, warm-head lookup, suffix-refresh protocol, eviction policy, or
process-wide semantic run map in this target. Those mechanisms only compensate for a stateless API
that repeatedly opens the same run. Catalog-local `Program` interning by exact `ProgramRef` is
unrelated: it shares immutable executable structure and carries no run currentness or invocation
authority.

Multiple callers may independently obtain callback-free qualified evidence, but the fenced
run-driver owner mints at most one live drive permit for a run/store epoch. Dropping or releasing a
session ends that permit and deliberately loses its process continuation evidence. A later driver
performs explicit cold resume and acquires a new generation. Writer failover/revocation and provider
entry serialize against that generation, so an old session cannot append or enter an MFM capability
after the new generation wins.

### 7.2 Direct advancement and durability

A direct `NewlyCommitted` result advances `RunSession@H` to its retained successor at H' with zero
history reads, decoding, or reduction. The compare-and-append transaction checked the session's
writer epoch and driver generation, and PostgreSQL acknowledged the qualified durability point for
the exact sealed bytes. Asking it to echo those bytes cannot strengthen that claim.

`StaleHead`, a typed append-precondition failure, and `AcknowledgementUnknown` install no successor.
The unknown outcome releases no Effect invoker until exact append identity is resolved. Resolution
may allow a later explicit resume, but it never manufactures the direct one-use invocation permit
that only the original `NewlyCommitted` authorization owns.

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

> After one cold resume, every directly committed step advances the same active session without
> reloading or refolding any retained batch.

Cold resume remains bounded by the existing history limits and may replay the complete prefix.
Hard limits on concurrent active sessions and ingress work remain ordinary resource controls. A
stronger cold-resume latency guarantee, compact checkpoint, or global asymptotic claim requires
measurement and a separate design; it is not hidden inside an LRU.

---

## 8. Append, acknowledgement, and invocation

### 8.1 Prepared append

Applying one `ResolvedEvent` produces `PendingAppend`; deterministic assignment, projection, stable
physical-obligation discharge, and exact enumeration of changing-currentness requirement metadata
then seal one private coherent value:

```text
PreparedAppend {
    batch,
    successor: PreparedSuccessor@H_next,
    projection/index deltas including needs_effect_attention,
    writer epoch + run-driver generation,
    secret-free resource-currentness requirement metadata
}
```

Every field is derived from the same event, predecessor, and qualified `AppendContext`. Independent
callers cannot supply a batch, successor, and projection that merely happen to be wrapped together.

The seal proves exact record/object projection (including absence of surplus objects), assigned
record/head binding, direct object/value/first-seen/fact-scan index extension, and all stable
retained physical obligations. It carries the exact lease/fence obligations to their named use
boundary as durable requirement metadata only; it does not own the live lease/token or prove a
changing resource current forever. `PreparedAccess` carries only owner identities, generations,
requirements, and its inaccessible invocation before commit. Driver/resource entry permits can be
minted only after direct `NewlyCommitted` and exist only in `EnteredAccess`. The append constructor
is private and cannot mix a batch, successor, or index plan from different preparations.

The backend interprets no record-family semantics. It owns only:

- transaction atomicity;
- exact-head compare-and-append and affected-row checks;
- append-atomic exact replacement of the run-head/attention projection;
- store-writer/run-driver fences and tenant-fact-frontier preconditions;
- append identity and idempotency lookup;
- database-native parameter/frame constraints; and
- ambiguous-acknowledgement classification.

Semantic append capacity, retained-history totals, and the two-successor authorization reserve are
proved while constructing `PreparedAppend`. Resource-specific currentness remains with the named
resource authority or physical-obligation checker; the run backend does not absorb it.

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

The backend is deliberately not a semantic equality oracle. `Found` returns a bounded raw retained
attempt. The store ingress owner qualifies it and is the sole classifier of
`ExistingSame | AppendConflict | InvalidHistory | CapacityExceeded`; only a well-formed unequal
attempt becomes `AppendConflict`. The coordinator does not ask the backend to classify it and then
repeat the comparison.

`StaleHead` means only that the exact-head comparison failed. Writer-fence expiry,
fact-frontier mismatch, capacity, and projection inconsistency remain distinct typed failures; they
are not relabelled as a stale journal head.

| Store disposition | Active session | May create `CommittedAccessPendingEntry` |
| --- | --- | --- |
| `NewlyCommitted` | Advance the consumed predecessor session; no reload | Only for an authorization append |
| `Found` -> `ExistingSame` | Compare once; do not install candidate | Never |
| `Found` -> `AppendConflict` | Fail closed; install nothing | Never |
| `Found` -> invalid/capacity error | Fail closed; install nothing | Never |
| `StaleHead` | Consume the stale session; explicit resume is required | Never |
| `AcknowledgementUnknown` | Install nothing; resolve exact identity | Never |

`NewlyCommitted` is payloadless because the positive value would only echo the sealed candidate the
backend consumed. A found stored attempt remains real database ingress: its retained bytes are
qualified and compared once against the candidate because it may describe a historical append
followed by later heads. Equal bytes establish idempotency only; they do not establish that the
attempt is still the current run head.

There is no unnamed deployment serializer or shared publication race: preparing an append requires
the private live `RunDriverPermit`, and the store transaction checks its exact generation. The
cross-process permit owner cannot mint a second live generation until revocation/expiry of the
first has linearized. Exact-head compare-and-append still protects journal ordering; the driver
fence separately proves who may continue or release an invoker.

The table describes progression toward new entry authority, not whether earlier work happened.
`NewlyCommitted` alone is still insufficient for provider I/O: the post-commit driver/resource
entry transition must produce `EnteredAccess`. An authorization append's non-new outcome calls
neither `enter` owner and invokes zero times. An observation append occurs after an invocation, so
its non-new outcome releases no new invoker and causes no additional invocation.

### 8.3 Durable external access

The MFM-mediated access bracket remains:

```text
PreparedAccess<K>                       // requirements + inaccessible invocation; no entry permit
  -> commit ExternalAccessAuthorized
  -> CommittedAccessPendingEntry<K>      // only from direct NewlyCommitted
  -> driver.enter(generation) + resource.enter(requirement)
  -> EnteredAccess<K>                    // owns affine entry permits
  -> consume once into exact adapter/provider I/O
  -> validate external response once
  -> commit ExternalAccessObserved
  -> settle state
```

Runtime never passes the adapter or invoker to the state callback. The affine authorization binds
the exact capability, access kind, request, program occurrence, physical binding, and committed
head. `PreparedAccess` binds the session's assembly brand, driver generation, and resource
requirement but cannot enter. Direct commit moves those facts and the still-inaccessible invocation
into `CommittedAccessPendingEntry`. Only then, immediately before provider I/O, do the named owners
serialize entry against driver transfer and resource revocation and mint the affine permits inside
`EnteredAccess`.

If resource revocation wins while the run driver remains current, Runtime invokes zero times and
commits `SupersededBeforeEntry`. If the run-driver generation itself is lost, the former driver
invokes zero times and cannot append; the successor driver resumes the durable pending authorization
and applies the conservative recovery protocol. A crash before a known pre-entry disposition is
durable remains `PossibleEntry`; the absence of provider entry cannot be inferred from authorization
alone.

Only direct `NewlyCommitted` lets Runtime create the pending-entry package; only `EnteredAccess`
lets it consume the invocation. Runtime performs no history reload and advances the active session
directly.

The post-authorization full reload and field-by-field comparison are deleted. They do not strengthen
the database commit and only reconstruct a value the process just created.

---

## 9. Readers, facts, startup, and configuration

### 9.1 One demand-triggered ingress

Resume/drive and per-run read, trace, replay, and export requests use the same bounded prefix-ingress
and reducer implementation. An active driver passes its `RunSession` directly; a genuinely separate
read-only request constructs callback-free `QualifiedRun`, projects the requested DTO, and drops the
evidence. It never acquires a driver permit or Runtime assembly brand. Distinct public DTOs and
caller-policy checks remain because they own disclosure, not history validity.

Effect-attention listing reads only the append-atomic routing projection. It cannot execute or
settle a run. Any recovery action selected from that list must explicitly resume the named run
through the same ingress before acting.

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
Effect-attention routing, which never grants execution authority.

The frontier establishes a changing cross-run fact, but it is not a reason to revalidate every
already-retained producer prefix.

When tenant-wide Effect recovery discovery is part of the product, the reducer-derived
`needs_effect_attention` Boolean is carried by `PreparedAppend`. PostgreSQL updates it in the same
transaction that appends the batch and advances the run head; a partial index supplies a
snapshot-complete list under the trusted-database contract. A second attention frontier, boot audit,
and hot-path reconstruction are unnecessary. If the product does not expose global discovery, the
projection and API are deleted together.

### 9.3 Demand-time history qualification

Ordinary store open verifies:

- schema and store identity;
- backend channel and durability profile;
- writer fence/epoch;
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
authority, grant invocation authority, repair history, or persist a proof after the result is
discarded.

### 9.4 Configuration history follows the same ingress rule

Configuration is not an exception to this RFC merely because its history is smaller. External
configuration files, environment streams, API payloads, and retained configuration rows are bytes
until their owning ingress returns an opaque `ResolvedConfiguration<T>@ConfigurationHead`. A locally
constructed typed value already satisfies the same intrinsic invariants and is not serialized and
reparsed for reassurance. Program planning consumes this typed value directly.

Persisted `ConfigurationValue` remains secret-free. Secret-bearing input is admitted into a
non-serializable issuer/resource handle with explicit lifetime and redaction; it is never projected
into a configuration revision, journal record, fact, export, or diagnostic.

The writer constructs one private coherent value:

```text
PreparedConfigurationAppend {
    revision,
    successor: PreparedConfigurationSuccessor@ConfigurationHead_next
}
```

The revision binds its exact predecessor and content reference. The backend performs an exact-head
compare-and-append and returns the same payloadless `NewlyCommitted | Found(raw) | StaleHead |
AcknowledgementUnknown` protocol used for run appends. Only direct `NewlyCommitted` promotes the
retained successor. `Found(raw)` crosses configuration ingress once and becomes either
`ExistingSame`, a typed conflict, or an invalid/capacity error; no positive backend echo is compared
by a second owner.

The active configuration owner retains the locally prepared successor after direct commit; there is
no shared semantic configuration cache. A later selection for planning, resume, replay, export, or
audit qualifies the bounded predecessor chain once. A malformed selected revision rejects that
consumer, while a dormant malformed revision does not block startup or a new run that does not
select it. Selecting whether a revision is currently active is a changing head/policy proposition
and remains an explicit query; it is not another validation of immutable revision bytes.

The current writer-side full-prefix reload, repeated `ValidatedConfigurationAppend::from_object`
on locally created revisions, positive echo comparison, reader-specific replay, and eager
configuration-open audit are deleted in the same cutover. This does not require a generic history
framework: run semantics and configuration semantics retain distinct domain types while obeying
the same ingress law.

---

## 10. API and deletion cutover

### 10.1 Program and certification

Replace with one current API:

- opaque `Program`;
- serializable `ProgramDocument`;
- private-field, non-deserializable `ProgramCandidate`/`ProgramFragment` authoring IR that no
  execution consumer accepts;
- one registry assembly producing callback-free `ProgramCatalog` and non-cloneable
  `ProcessRegistry` views;
- exact `ProgramRef` caching.

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

### 10.2 State and capabilities

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
- callable signer/resource process handles, while retaining secret-free semantic dependency
  identities and lineage;
- feature-gated public authorization constructors, replaced by one crate-private Runtime transition;
  and
- the copied `ExpectedAuthorization` shadow and repeated runtime comparisons only after the private
  `PreparedAccess -> CommittedAccessPendingEntry -> EnteredAccess` package structurally owns every
  relation they currently protect.

Retain:

- one exact capability per state;
- `Read<C>` and `Effect<C>` request/return/failure contracts;
- opaque invariant-safe capability values, capability-specific safe-failure sums, and the one
  request/response relational check at adapter ingress/settlement;
- Effect entry, ambiguity, absorption, and refresh semantics;
- distinct Read/Effect completion types, including Effect-only entry dispositions;
- safe-failure disposition types;
- FanOut's type-level Effect exclusion; and
- affine authorization and invocation, including dynamic erasure and all durable authorization
  identity fields;
- exact adapter/signer/resource dependency identities and the reserved one-use fact scanner; and
- panic containment and redaction-safe fault conversion.

Existing domain `valid_*` functions are not blindly deleted. Intrinsic unary invariants move to
private constructors/custom deserialization, request-response relations move to the one adapter
ingress or settlement, and mutable facts become named lease/fence checks.

### 10.3 Store and runtime

Delete:

- local intent qualification that reparses trusted domain values;
- re-decoding/requalifying just-authored objects to reconstruct their index;
- `qualify_recorded_successor` on local batches;
- dual reducer branches and comparison typestates;
- positive committed-batch echo/comparison;
- Runtime's post-authorization reload and recheck;
- application pre-drive and post-drive verified loads;
- `drive_once(run_id)` as the Runtime-core ownership API, replaced by explicit spawn/resume and an
  affine caller-owned session;
- purpose-specific verification implementations and repeated loads within one session/request;
- stale-head/idempotency recovery that silently reloads and continues instead of requiring explicit
  resume; and
- the eager whole-store semantic-open sweep from ordinary readiness.

Keep:

- strict demand-triggered complete-prefix qualification for explicit resume/read/replay/export;
- callback-free `QualifiedRun` for semantic projections and Runtime-branded, driver-fenced
  `RunSession` for execution;
- exact content/object closure for newly entered bytes;
- direct extension of the prepared successor's object/value/first-seen/fact-scan indexes from the
  resolved typed artifacts, rejecting surplus objects and ignored record fields at cold ingress;
- affine `RunSession` ownership and direct successor advancement;
- atomic exact-head compare-and-append;
- one exact stored-attempt ingress comparison against the already-built candidate;
- ambiguous acknowledgement recovery;
- backend-owned writer/run-driver/fact-frontier currentness plus resource-owner leases/fences; and
- separate redaction-safe public projections.

Do not introduce a global semantic run cache, suffix-refresh protocol, cloneable session, or durable
reducer checkpoint in this cutover.

### 10.4 Configuration

Replace configuration writer/reader/open-audit verification paths with one typed configuration
ingress, `ResolvedConfiguration<T>`, private `PreparedConfigurationAppend`, and payloadless positive
commit. The active writer retains and directly promotes its prepared successor; an independent
selection performs one demand-triggered complete-history ingress. Delete local revision
revalidation, positive echo comparison, purpose-specific replay, shared configuration-cache plans,
suffix refresh, and eager configuration scanning from store readiness. Retain strict external
source/row ingress, exact predecessor/content binding, exact-head compare-and-append, idempotency,
ambiguous acknowledgement recovery, and the separate diagnostic `audit_store` scan.

---

## 11. Authoritative contract changes

Implementation changes the following documents in the same commits as their code:

### `docs/design.md`

- Define the one byte-ingress rule and separate it from currentness, authority, and durability.
- Replace absolute type-enforced no-ambient-I/O language with the enforceable MFM-capability rule
  while retaining no ambient I/O as a coding obligation.
- Replace authored/expanded/certified execution authority with `ProgramDocument` and opaque
  `Program`.
- Freeze `RunAdmitted`/journal serialization while requiring a fresh store identity for the new
  program-document schema.
- Separate callback-free program/catalog authority from Runtime's live process registry.
- Replace the three-layer write comparison with `ResolvedEvent` and one reducer.
- Distinguish callback-free `QualifiedRun@H` from Runtime-branded affine `RunSession@H`; describe
  demand-triggered full resume, the cross-process run-driver fence, exact-head compare-and-append,
  and direct successor advancement.
- State that spawning a new run opens no unrelated retained history and malformed dormant history
  is isolated to operations that actually consume it.
- State that no shared run/configuration semantic cache or durable reducer checkpoint exists.
- Apply the same local-construction/direct-commit versus independent-ingress distinction to
  configuration history.
- Replace mandatory authorization reread with direct positive commit acknowledgement and retained
  successor.
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
- Show one registry assembly with least-authority views, one privately branded driver owning one
  fenced affine run session, one reducer, and purpose projections borrowing callback-free qualified
  evidence rather than acquiring drive authority.

### `docs/run-execution.md`

- Change admission from app certification plus store recertification to trusted `Program`
  construction plus document persistence.
- Describe one resolved transition and locally retained successor.
- Distinguish new-run spawn from retained-run resume; new admission opens only explicitly selected
  historical dependencies.
- Describe full-prefix resume once, repeated direct advancement of the active session, and explicit
  resume after the session is dropped or loses the head comparison.
- Describe run-driver acquisition/failover and the entry-fence rule preventing an old driver from
  invoking after its generation loses.
- Describe typed configuration construction, direct writer promotion, and independent cold
  selection without a cache.
- Make positive append acknowledgement payloadless.
- Preserve authorization-before-invocation and observation-before-settlement.

### Other documentation

Update the READMEs for program, capabilities, certification, store, runtime, app, and replay, plus
`docs/persisted-public-surfaces.md`, including the golden-frozen journal/configuration formats and
fresh store-identity program cutover. Updating `docs/known-gaps.md` is mandatory: it must record
demand-time history qualification and failure isolation, the separate diagnostic `audit_store`,
the absence of a durable semantic checkpoint, the PostgreSQL epoch/restore assumption, and the
unchanged inability to detect a self-consistent rollback after every outside anchor is lost.

---

## 12. Verification plan

### 12.1 Program and capability construction

Compile-fail tests prove:

- `Program` cannot be field-constructed or deserialized;
- Pure cannot register Read/Effect callbacks;
- callbacks whose ABI differs from `S::Execution` cannot register;
- a hot `Read<C1>` registration cannot nominally associate a `C2` adapter or an Effect adapter;
- Effect cannot register Read callbacks;
- request, returned, and safe-failure ABI come only from `C`;
- Effect remains forbidden in FanOut;
- erased Pure/Read/Effect dispatch cannot select a wrong-mode method;
- no public MFM adapter invocation entry exists outside private `EnteredAccess`;
- no consumer can mint that package under the actual production dependency/feature graph; and
- affine authorization cannot be cloned or consumed twice.

Behavior tests prove:

- both construction paths converge on one private normalized-graph invariant owner, while
  source-specific typing and byte ingress perform only their own checks;
- typed construction and hostile document ingress produce equal `ProgramRef`, canonical
  `ProgramDocument`, normalized graph, schema/lexical indexes, and registry fingerprint;
- a bare `ProgramRef` cannot drive reduction or invocation, and the callback-free catalog view
  cannot invoke;
- cold ingress proves the normalized graph's frozen policy predicate; omitted source/trace metadata
  cannot grant authority;
- hostile access-label, capability, component, and graph substitutions fail at ingress;
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

### 12.2 Event and journal

- Golden `RunAdmitted`, H1/H2, and configuration-revision vectors freeze the unchanged wire and hash
  preimages before the refactor begins.
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

### 12.3 Session ownership, demand qualification, and concurrency

- `QualifiedRun` and `RunSession` cannot be deserialized or publicly constructed; `RunSession`
  cannot be cloned or used across store identity, epoch, tenant, run, program catalog, or private
  Runtime assembly identity.
- A callback-free purpose reader cannot acquire a driver permit, invoke a Pure callback, or seal its
  `QualifiedRun` into `RunSession`.
- Runtime rejects substitution of a qualified prefix/program from another
  `RuntimeAssemblySeal`/`ProcessRegistry` before any callback dispatch.
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
- Concurrent drive-resume attempts may both qualify bytes, but the cross-process owner mints only
  one live `RunDriverPermit`; a fenced old generation cannot append or enter an MFM capability.
- Exact-head compare-and-append still admits only one prepared successor for a predecessor; a stale
  or precondition-losing session installs nothing and must resume explicitly.
- Dropping a session or restarting causes the next continuation to perform a new complete ingress.
- No global run-state map/LRU, suffix-refresh path, session serialization, or durable semantic
  checkpoint is written or consulted.
- Active-session and ingress-work limits reject excess work explicitly rather than silently evicting
  another execution's authority.
- A legitimate PostgreSQL restore uses a new store identity/epoch; same-epoch historical mutation is
  tested as a storage-contract violation.
- Store qualification rejects a durability profile that can return commit success before the
  authorization WAL is durable across its admitted failure domain, including `fsync = off` or
  `synchronous_commit = off`; stronger host-loss claims require their named synchronous quorum.

### 12.4 Append and external access

- `NewlyCommitted` advances the exact consumed session with zero head query, history read, decode,
  or reducer replay.
- Before direct `NewlyCommitted`, Runtime calls neither `RunDriverAuthority::enter` nor the resource
  entry owner, and no type can reach the invocation thunk.
- Direct `NewlyCommitted` creates only `CommittedAccessPendingEntry`; provider I/O remains
  impossible until both post-commit entry operations succeed and construct `EnteredAccess`.
- An authorization `Found` classified as `ExistingSame` installs no candidate session and releases
  no invoker.
- Authorization-append `StaleHead`, `AcknowledgementUnknown`, invalid found bytes, capacity failure,
  and typed `AppendConflict` call no entry owner and invoke zero times; no non-authorization append
  releases a new invoker.
- `Found` under the same append identity with well-formed different bytes returns `AppendConflict`
  after exactly one stored-attempt comparison.
- `AcknowledgementUnknown` followed by exact retry leaves one append and never duplicates
  invocation.
- Every framework invocation has a distinct, directly acknowledged `NewlyCommitted` authorization,
  and its affine permit is consumed at most once; a crash may leave zero invocations.
- Observation recovery may retain the same authorization reference but never recreates or reuses
  its consumed invocation permit and never invokes that attempt again.
- A later Effect invocation requires a new committed attempt ordinal. `MAX_ENTRIES` bounds
  entry-unknown re-entry; refresh evidence governs supersession ordinals separately.
- `EnteredAccess` permits cannot be paired with another same-`C` request, run, occurrence, process registry,
  adapter implementation, physical binding, or resource-lineage head.
- The special prior-run fact-scan permit remains store-minted, one-use, and direct-new-commit-only.
- A retained session does not bypass a live capability lease/fence check at the declared entry
  boundary.
- For a capability promising adapter-entry freshness, revocation after authorization commit but
  before entry causes zero adapter invocations and durably records `SupersededBeforeEntry` when the
  process observes that pre-entry result.
- Run-driver transfer/fencing after authorization commit but before entry causes zero adapter
  invocations; the old driver cannot append an observation or disposition, and the new driver sees
  the durable pending authorization through conservative recovery.
- Every provider success/failure response crosses one bounded adapter ingress into opaque
  `C::Returned`/`C::SafeFailure`; settlement performs no second wire/schema validation.
- Observation is committed before state settlement succeeds.
- PostgreSQL snapshot, atomicity, restore/epoch, exact-head compare-and-append contention, numeric
  ordering, malformed-row, and fresh-process continuation tests remain.

### 12.5 Facts and purpose projections

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
  and fact-frontier corruption without changing readiness or granting invocation authority.
- When Effect-attention inventory is enabled, append, head advance, and
  `needs_effect_attention` update are atomic; the partial index is snapshot-complete under the
  PostgreSQL contract.
- Effect-attention listing invokes zero callbacks. Any selected recovery action explicitly resumes
  and qualifies the named run before acting.

### 12.6 Configuration history

- Typed/local configuration construction increments no ingress-validation counter.
- Hostile file/environment/API and database revisions are bounded and admitted exactly once.
- Secret-bearing configuration produces only a non-serializable live handle; secret canaries never
  appear in revision bytes, history, exports, or diagnostics.
- Full independent configuration ingress produces the same exact head/value as direct local writer
  advancement at every revision.
- `NewlyCommitted` advances the active configuration owner with no echo or readback.
- Found-same, conflict, stale, and unknown acknowledgement behavior preserves idempotency and never
  installs an uncommitted successor.
- A later independent configuration selection performs one complete bounded ingress; no shared
  configuration cache, suffix refresh, or checkpoint is consulted.
- Malformed dormant configuration does not block readiness or a new run that does not select it;
  selecting it fails before planning receives `ResolvedConfiguration<T>`.
- A changing active-revision selection is checked as head/policy currentness, not by revalidating
  the selected immutable value.
- Wire/schema ingress acceptance and a state's contextual interpretation/settlement are tested as
  different propositions; the latter is not mislabeled duplicate byte validation.

The performance acceptance criterion is measured work and load counts, not an unsupported global
asymptotic claim.

---

## 13. Logical commit sequence

Every commit updates affected contracts, tests, fixtures, and public documentation. No old/new API
pair, compatibility decoder, or fallback survives its cutover commit.

1. **`freeze current run and configuration fixation formats`**

   Add golden `RunAdmitted`, candidate, record, commit, H1/H2, and configuration-revision vectors
   before moving any producer. Document that the following work preserves those exact wire/hash
   formats and uses a fresh store identity for the new program-document schema.

2. **`relocate the callback-free program compiler`**

   Move recipe expansion/lowering, policy and normalized-graph validation, manifest/document
   construction, and schema/lexical indexes into `mfm-program` behind the current public API. Prove
   byte-for-byte output equality; introduce no second compiler and change no persisted bytes.

3. **`make state execution mode-safe without wire changes`**

   Derive callback ABI from `Pure | Read<C> | Effect<C>`, introduce mode-indexed typed and erased
   callable representations, move live `ProcessRegistry`/invocation-thunk ownership into
   `mfm-runtime` behind the current API, and preserve every existing canonical capability/state
   contract byte. Delete callback/ABI repair, unqualified invokers/test fallbacks, obsolete
   capability-set/signing algebra, and callable signer/resource process handles. Keep generic
   capability validators, their erased handles, durable authorization fields, and
   `ExpectedAuthorization` until their replacements land in later commits.

4. **`make program and capability values valid by construction`**

   Introduce opaque capability request/returned values and capability-specific safe-failure sums,
   then introduce `ProgramDocument`, opaque `Program`, callback-free `ProgramCatalog`, and the
   assembly seal around the already Runtime-private `ProcessRegistry`. Cut every
   producer/consumer/export/fixture to the one API, delete the replaced generic capability
   validators/erased handles, old program/certifier/registry authority surfaces, and the now-empty
   `mfm-certify` boundary. Initialize all wire-changing types only in a fresh store
   namespace/identity. Keep no decoder or in-place migration.

5. **`resolve appends and advance affine run sessions`**

   Introduce `ResolvedEvent`, `PendingAppend`, `AppendContext`, full `RetainedBatchFixation`, and
   structurally inseparable `PreparedAppend { batch, successor, index plan }` for every event family
   in one cutover. Introduce callback-free `QualifiedRun`, Runtime-branded non-cloneable
   `RunSession`, the cross-process fenced `RunDriverPermit`, explicit spawn/resume, direct session
   advancement, Runtime-private `PreparedAccess -> CommittedAccessPendingEntry -> EnteredAccess`
   invocation typestate, qualified history-port outcome handling, and payloadless
   `NewlyCommitted`. State and qualify the PostgreSQL
   exact-head/minimum-durability/epoch contract here. In the same commit delete both reducer branches,
   semantic equality/comparison typestates, local requalification, backend semantic echoes,
   `drive_once(run_id)` core ownership, app/Runtime post-commit reloads, stale-head automatic
   continuation, public/feature-gated authorization minting, and `ExpectedAuthorization` shadows.
   The old guard is removed only when its projection, binding, obligation, ordering, and session
   ownership jobs have structural replacements in this commit.

6. **`qualify retained history only when consumed`**

   Unify bounded complete-prefix resume, read, replay, export, audit, and producer-closure ingress
   behind the one event/reducer path. Make ordinary startup and unrelated new-run spawn perform zero
   retained-history scans. Cut purpose projections to borrow callback-free evidence from an active
   session or explicitly qualify an ephemeral `QualifiedRun`, keep the dense fact frontier
   load-bearing, and make Effect-attention an
   append-atomic Boolean projection if tenant-wide discovery remains a product surface. Add the
   diagnostic `audit_store`; add no shared semantic cache, suffix-refresh protocol, attention
   rebuild authority, or reducer checkpoint.

7. **`make configuration history single-ingress without a cache`**

   Introduce `ResolvedConfiguration<T>` and private prepared successors, make direct
   `NewlyCommitted` advance the active configuration owner with no readback, and make every
   independent selection one bounded complete-history ingress. Preserve exact-head
   compare-and-append, idempotency, acknowledgement ambiguity, and the revision wire/hash format;
   delete local revision revalidation, echo comparison, purpose-specific replay, cache/suffix plans,
   and configuration scanning from ordinary readiness.

Do not defer documentation or the test that establishes a replacement invariant until after the
old guard has been deleted.

---

## 14. Acceptance criteria

The refactor is complete only when all of the following are true:

- No public-field or deserializable value is accepted as execution authority; only opaque `Program`
  is.
- A bare `ProgramRef` is never authority, and callback-free store/replay code cannot invoke.
- A hot typed program is globally checked once and is never recertified by store or Runtime.
- Cold/imported program bytes pass one ingress and produce an equivalent `Arc<Program>` with the
  same reference, normalized graph, indexes, and catalog identity.
- Pure recursive operation expansion, configuration specialization, and injected
  pre/proceed/post/failure states remain part of construction and are completely represented in the
  final normalized `Program`; resume never reruns them.
- The changed program-document schema starts under a fresh store identity while `RunAdmitted` and
  journal fixation formats remain golden-frozen.
- Every state has exactly one type-level `Pure`, `Read<C>`, or `Effect<C>` declaration.
- MFM gives state callbacks no transport, signer, store, or generic live capability.
- Adapter-owned dependencies remain adapter TCB internals rather than callable nested framework
  capabilities; their identities/lineage remain represented, and Read classifies whole-adapter
  behavior.
- Retained session evidence never bypasses live resource currentness, including revocation between
  authorization commit and adapter entry; a known pre-entry rejection is durably closed as
  `SupersededBeforeEntry`.
- The documentation does not claim arbitrary Rust closures are sandboxed or provably pure.
- Intent and recorded bytes converge before one reducer; no dual semantic implementation remains.
- The current journal hash format is frozen by golden tests and documented as an exact-prefix
  commitment.
- `QualifiedRun` is callback-free semantic evidence. `RunSession` is opaque, Runtime-assembly
  branded, driver-fenced, affine, non-serializable, and non-cloneable; no purpose reader, raw head,
  foreign assembly, or uncommitted successor can construct or advance drive authority.
- Store/service startup qualifies no retained run or configuration history.
- Spawning a new run opens no unrelated history and succeeds despite a malformed dormant run;
  consuming a malformed selected dependency fails locally before typed authority is returned.
- One explicit resume folds one complete bounded prefix once. Every directly `NewlyCommitted`
  append then advances that exact active session with zero head query, reload, decode, or replay.
- At most one cross-process `RunDriverPermit` generation is live per run; an old generation cannot
  append or enter an MFM capability after transfer/revocation wins.
- No shared run/configuration semantic cache, LRU, suffix-refresh protocol, or durable reducer
  checkpoint exists.
- Only a directly `NewlyCommitted` authorization can create `CommittedAccessPendingEntry`; only
  successful post-commit driver/resource entry can create `EnteredAccess` and reach the candidate's
  invoker.
- No consumer-callable authorization mint exists under the production feature graph, and exact
  same-`C` request/run/occurrence/registry/adapter/binding/lineage substitution is impossible.
- Authorization-append found-same, stale, unknown, invalid, capacity, conflict, regression, and fork
  paths call no entry owner and release no invoker; non-authorization failures release no new
  invoker or cause an additional invocation.
- Observation recovery never recreates a consumed permit; Effect entry and refresh ordinals retain
  their separate typed contracts.
- Configuration history has the same local-construction/cold-ingress/prepared-commit discipline and
  no independent writer/reader/readiness replay paths.
- The dense fact frontier remains load-bearing completeness evidence. If tenant-wide Effect
  discovery remains, its Boolean projection and partial index update append-atomically but never
  grant execution authority.
- Purpose readers borrow callback-free evidence from one active session or explicitly qualify one
  ephemeral `QualifiedRun`; no purpose can drive or owns another semantic verifier.
- Provider response bytes cross one adapter ingress into opaque capability values; downstream code
  does not confuse protocol validity with factual truth beyond the capability's provider/evidence
  contract.
- PostgreSQL's truthful outcome, immutable-row, durability, writer-lineage, and store-epoch contract
  is explicit; `NewlyCommitted` satisfies the qualified crash-durability profile before any provider
  entry, and legitimate restore rotates the identity/epoch.
- Intrinsic validators have moved to opaque construction/ingress; changing facts have explicitly
  named authority owners; redundant shadows are deleted.
- Safe-failure values are capability-specific valid types; Effect-only completion variants, keyed
  entry/bounds, refresh evidence, resource lineage, dependency identities, and the reserved fact
  scanner remain represented.
- The final report measures product-code and public-type deltas and itemizes every new concept or
  net addition; no increase is justified merely as scaffolding for a later cleanup.

When these conditions hold, MFM will rely on the properties it already built: Rust construction,
content identity, an exact recursive journal commitment, atomic exact-head compare-and-append, and
typed access authority. It will stop paying every internal layer to distrust the preceding one.
