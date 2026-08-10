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
authorization, current leases and fences, compare-and-swap, durability acknowledgement, redaction,
and secret lifetime are different propositions with explicit owners.

The target design has a small set of load-bearing values and no generic validation layers between
them:

```text
ProgramDocument bytes -- one ingress --> Program

configuration source/row bytes -- one ingress --> ConfigurationValue / VerifiedConfiguration@G

raw run bytes -- one ingress + fold --> VerifiedRun@H

typed proposal -----------------------> ResolvedEvent
qualified record bytes ---------------> ResolvedEvent

VerifiedRun@H + ResolvedEvent
    -> PreparedAppend { batch, successor: PreparedSuccessor@H' }
    -> exact-head CAS
    -> CommitAcknowledged -> VerifiedRun@H'
```

`Program` is valid by construction. `VerifiedRun@H` is semantic authority for one exact journal
prefix. `ConfigurationValue` and `VerifiedConfiguration@G` carry the analogous immutable
configuration evidence. `ResolvedEvent` is the sole run-semantic input. `PreparedAppend` binds the
bytes and not-yet-authoritative successor that were derived together. Only the direct
`NewlyCommitted` outcome establishes the fact needed to promote the successor to `VerifiedRun`. It
does not cause the process to distrust and reconstruct its own values.

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

These decisions require owner rulings before implementation begins.

1. **Database durability and historical immutability**

   - **Choice:** the qualified backend and configured PostgreSQL service are trusted to report
     transaction outcomes truthfully and to preserve committed batch/object rows immutably under
     the admitted roles. Loaded bytes remain hostile until ingress; an actively lying DBA is not in
     scope.
   - **Why uncertain:** the repository does not state whether durability means transaction
     acknowledgement, WAL fsync, or synchronous replica quorum, and it uses stronger hostile-row
     language than this operational assumption.
   - **If wrong:** neither a cache nor repeated replay can establish durable availability against
     the database owner. An effect could be authorized from process-held evidence whose backing
     bytes are no longer recoverable.
   - **Resolution:** publish and qualify the PostgreSQL roles, channel, `fsync`,
     `synchronous_commit`, and replication contract. If DBA-level rewriting is in scope, add an
     independently authenticated monotonic witness or immutable content store.

2. **Demand-time versus boot-time history qualification**

   - **Choice:** store readiness verifies infrastructure, schema, identity, and writer authority;
     each run crosses the byte-ingress boundary lazily on first use. The separate `audit_store`
     operator action scrubs all history.
   - **Why uncertain:** the current design blocks store opening until every visible run and index is
     audited, then discards the resulting per-run evidence.
   - **If wrong:** removing the eager sweep changes readiness from “all retained history is valid”
     to “the service can safely validate history when requested.”
   - **Resolution:** obtain the product readiness decision. A required global guarantee over an
     unbounded store needs a durable authenticated global root; repeatedly scanning and forgetting
     does not provide bounded continuous evidence.

3. **Live revocation latency**

   - **Choice:** program and process bindings are immutable for one process lifetime. Any authority
     that promises live revocation uses one explicit lease or fence checked by its owner at use.
   - **Why uncertain:** acceptable signer, route, provider, and writer revocation latency is not
     specified.
   - **If wrong:** either stale authority remains active too long or arbitrary currentness checks
     reappear throughout the core.
   - **Resolution:** specify a latency for each revocable authority and choose process-lifetime
     admission, a renewable lease, or supervisor-enforced termination.

4. **Persisted program shape, audit evidence, and external consumers**

   - **Choice:** replace the authored/expanded/certified public authority stack with one normalized
     `ProgramDocument` and deploy it only in a fresh store namespace with a new store identity.
     Expansion/policy traces that do not affect execution are not retained as parallel authority.
     Old and new program histories never coexist under one store identity; no compatibility decoder
     or in-place migration is kept.
   - **Why uncertain:** repository policy permits the reset, but it is not yet proven that an
     external auditor needs only the normalized graph and exact catalog identities rather than the
     authored source and derivation trace. Out-of-tree histories and interchange consumers have not
     been inventoried either.
   - **If wrong:** an audit use case may lose required derivation evidence, or external histories
     and exports may become unreadable.
   - **Resolution:** inventory every persisted-program consumer and state the audit question each
     field answers. Retain source or trace only when a named consumer cannot derive its required
     answer from the normalized graph. If compatibility is required, stop and create a
     repository-wide policy rather than adding an ad hoc legacy path.

5. **Effect-attention completeness**

   - **Choice:** the Effect-attention route is a rebuildable index over qualified run histories,
     not semantic authority. The separate `audit_store` operator action detects omissions. The
     tenant fact frontier and its dense publication sequence are different: they remain
     append-atomic, load-bearing evidence because prior-run fact selection depends on proving
     negative completeness.
   - **Why uncertain:** the product contract does not say whether ordinary serving must discover
     every abandoned Effect immediately, even when that run is otherwise cold.
   - **If wrong:** a missing attention route could delay recovery until the run is opened or the
     operator audit runs.
   - **Resolution:** obtain the recovery-latency requirement. If proactive global completeness is a
     serving guarantee, add a separately named append-atomic attention frontier; do not turn an
     ordinary derived route into implicit authority.

6. **Provider truth**

   - **Choice:** a capability either names a qualified provider as part of its TCB or explicitly
     requires cryptographic/quorum evidence. Boundary validation proves protocol and authenticity,
     not that a provider's factual claim is true.
   - **Why uncertain:** current contracts sometimes use “verified” for both meanings.
   - **If wrong:** a well-formed, authenticated but false RPC response may be treated as objective
     chain truth.
   - **Resolution:** classify trust for every production capability and narrow its returned type to
     the evidence actually established.

7. **Read classification**

   - **Choice:** `Read<C>` is a trusted semantic assertion that invoking `C` does not intentionally
     mutate or consume externally meaningful state.
   - **Why uncertain:** Rust cannot inspect an adapter's behavior, and Read retry/fan-out may invoke
     it more than once.
   - **If wrong:** a falsely labelled Read can duplicate a hidden mutation.
   - **Resolution:** review each production Read adapter and document the classification. Code that
     cannot satisfy it must be an Effect.

8. **Cache capacity and adversarial churn**

   - **Choice:** use a per-store weighted LRU with hard total-weight and entry-count limits, plus
     same-run refresh singleflight and per-tenant admission/concurrency limits. One entry's weight
     includes canonical retained bytes, decoded objects, reducer/index entries, and fixed overheads.
   - **Why uncertain:** production resident-size ratios and hot-set distribution have not been
     measured; the current 512 MiB canonical-history bound is not a heap bound.
   - **If wrong:** an authorized tenant can alternate large runs, force eviction/cold replay, or
     exceed the intended resident-memory budget.
   - **Resolution:** measure representative maximum runs, define conservative per-component weights
     and hard constants, then exercise churn and allocation counters before enabling the cache in
     production.

---

## 1. Scope

This RFC owns one cutover across typed program construction, state capability declarations,
configuration history, journal reduction, verified loading, append acknowledgement, and internal
read projections.

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
- database batches, objects, projections, idempotency lookups, and previously unseen suffixes;
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

### 2.3 Checks that are not byte validation

| Proposition | Owner | When checked |
| --- | --- | --- |
| Caller may act for tenant | Application access policy | Each public call |
| Journal head is current | Backend snapshot / CAS | Each load or append |
| Store writer and tenant fact frontier are current | Qualified backend fence / CAS | At append |
| Capability resource is current | Resource lease/fence owner | Authorization or adapter entry |
| Append is durably acknowledged | Qualified backend | Before releasing any MFM-mediated invoker |
| Nonce/operation key is unique | Resource authority transaction | At reservation/mutation |
| Public output contains no secret | Public DTO/render boundary | Before emission |

These checks remain because their facts are contextual or can change. They must be named after the
property they establish, not hidden behind a generic `validate` layer.

### 2.4 Disposition of current check classes

| Check class | Target disposition |
| --- | --- |
| External byte bounds, decode, canonicalization, authentication | Keep at the owning ingress |
| Private constructor and global invariant check | Keep once at construction |
| CAS, lease, fence, revocation, and caller policy | Keep at the owning authority |
| Redaction, zeroization, and secret sink control | Keep |
| Same-process serialize/decode/re-certify | Delete |
| Requalifying a locally compiled append | Delete |
| Intent-versus-recorded duplicate reduction | Eliminate structurally |
| Positive backend echo and comparison | Delete |
| Post-commit reload and field-by-field self-check | Delete |
| Separate full verification for each purpose reader | Delete |

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
Live process assembly, invocation thunks, and their private permit transition move into
`mfm-runtime`. Remaining non-authoritative helpers move to their `mfm-program` or `mfm-runtime`
owner, and the superseded `mfm-certify` crate/authority wrappers are deleted rather than retained as
a facade.

`Program` has private fields, no `Deserialize`, no public struct literal, and no unchecked public
constructor. It is cheaply shared through `Arc`. A `ProgramRef` is an address, not semantic or
invocation authority: persisted records carry the ref, the callback-free catalog alone resolves it
to the exact `Arc<Program>`, and semantic consumers accept that resolved value. They never accept a
document or bare ref and then pretend it is already valid.

`ProgramRef` is the domain-separated hash of the complete canonical `ProgramDocument`. Cache keys
also include the frozen registry fingerprint, so equal nominal component names under different
assemblies cannot alias.

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
`RuntimeAssemblyBuilder`, `ProcessRegistry`, `ErasedInvocationThunk`, `PreparedAccess`, and
`CommittedAccess` are all defined in the same `mfm-runtime` crate; no friend-crate visibility or
reversed dependency is required. `Program` binds their immutable secret-free identities, never
their live handles. Store and offline replay receive
the callback-free catalog only; Runtime receives the live registry. Both products carry the same
private registry identity established once at assembly, without duplicating program-validation
logic. Runtime alone receives the pairing seal, registry, and private invocation thunks.

Runtime resolves a program semantically through `ProgramCatalog` and resolves its implementation
identities separately through `ProcessRegistry` only at execution. Cloning `Arc<Program>` therefore
cannot clone an invoker or grant the store/offline tooling access to a callback.

This replaces the separate authoring certifier, persisted verifier, borrowed entry certifier,
duplicated verification snapshots, and later root/document equality checks. The catalog may cache
`Program` by exact `ProgramRef`; nominal entry-point identity alone is never a cache key.

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
program occurrence, process-registry identity, adapter implementation, physical binding/lineage,
durable authorization intent, one affine live lease/fence token when required, and an existential
one-use invocation. Only the data-only authorization intent and secret-free currentness-requirement
metadata cross the injected `QualifiedHistoryPort`. That port is an opaque store product paired
through `RuntimeAssemblySeal`; application code cannot install a raw/forged port into Runtime.

The history port returns `NewlyCommitted` only for the direct commit. A crate-private Runtime
transition consumes that result together with the still-private `PreparedAccess` and produces
`CommittedAccess`; neither constructor nor invocation handle is public under any Cargo feature.
The store never constructs or returns the invocation package. The erased dispatcher preserves the
same tuple when selecting the registered adapter.

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
thunk only by consuming `CommittedAccess`. Trusted adapter code may still call its own internal
client, but MFM exposes no invocation path that bypasses the committed package.

The framework does not create a second nested capability system for adapter internals. Callable
signer/resource process handles are deleted; concrete adapters own their transports, signers, and
resource clients as TCB internals. Secret-free dependency identities, resource lineage, and live
lease/fence obligations remain represented for audit and currentness. `Read<C>` classifies the
behavior of the whole trusted adapter, including those internals; a false Read classification is
the TCB failure described in Material uncertainty 7.

A cached `VerifiedRun` never freezes live revocation. Every revocable capability supplies an exact
current lease/fence token, checked by its resource owner at authorization or immediately at adapter
entry according to the published latency contract. The semantic cache does not store that changing
proposition as permanent evidence.

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

cold/suffix ingress:
    qualify the complete retained envelope
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
    -> VerifiedRun
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

Given a previously qualified `VerifiedRun@H10`, canonical encoding, and SHA-256 collision and
second-preimage resistance, H10 establishes:

- exact prefix identity and append order;
- predecessor continuity to genesis;
- exact record payloads and coordinates;
- exact identities of every retained object and exact bytes when those objects entered;
- the represented program, configuration, values, facts, access requests, and observations; and
- equality between the process's reduced state and the prefix from which it was established.

A public deserialized `JournalHead` establishes none of this by itself. Authority comes from the
opaque `VerifiedRun` that records how the head was established.

### 6.3 Non-guarantees

The chain does not prove:

- semantic legality before the prefix has crossed ingress;
- that the presented head is latest;
- which of two otherwise valid successors is accepted; the exact-head CAS selects one;
- authorship, because the hashes are not signatures or MACs;
- absence of a valid rollback after the trusted anchor is lost;
- continued availability of historical records or object bytes;
- provider truth or real-world effect occurrence; or
- arbitrary callback machine-code behavior beyond represented implementation identities.

This is hash-chain integrity, not blockchain consensus, finality, or replication.

---

## 7. `VerifiedRun@H` is derived authority

`VerifiedRun` is opaque, non-serializable, and scoped to one opened store identity, writer epoch,
program-catalog fingerprint, and physical-verification assembly. It contains the exact head,
program, qualified immutable context, reduced state, retained-object index, fact dependencies, and
capacity accounting required to continue.

It has exactly three constructors:

1. Full cold prefix ingress and fold.
2. Qualification and fold of a suffix that extends another `VerifiedRun`.
3. A `PreparedSuccessor` promoted only by the direct `NewlyCommitted` exact-head outcome.

The shared store cache carries this derived authority. It is not a “zero-authority accelerator.”

### 7.1 Warm load

One cache is shared by the writer and all purpose readers. Entries are keyed and scoped by store
identity/epoch, tenant, run id, and exact `JournalHead`. The conceptual algorithm is:

| Backend observation | Behavior |
| --- | --- |
| No cached run | Load the bounded complete prefix, qualify/fold once, then install |
| Exact cached head | Reuse the cached `VerifiedRun`; load no prefix batches |
| Strict extension | Load only batches after the cached head, qualify/fold them once, then install |
| Run absent while cached | Fail closed as observed removal/rollback |
| Lower sequence | Fail closed as observed rollback |
| Same sequence, different digest | Fail closed as an observed fork/rewrite |
| Higher head with a gap or wrong predecessor | Fail closed as invalid history |
| Malformed suffix | Preserve the old proof, install nothing, and fail |

The backend reads its indexed head first inside one repeatable-read snapshot. That query establishes
the current observed fixation. It does not re-establish semantic validity.

Eviction or restart loses process-local evidence. The next access is a cold ingress. A
self-consistent database rollback after that loss remains undetectable without an external witness,
matching the current absent-witness contract.

### 7.2 Retention and availability

A process-held `VerifiedRun@H` remains semantically valid for H. MFM does not continuously reread
every historical byte to prove availability beneath H.

Consequently:

- an unchanged-head warm action may continue without loading old batches;
- a fresh process, cache miss, historical replay, or export fails if required bytes are absent;
- an observed regression or fork fails while the process remembers the later head; and
- a direct `NewlyCommitted` authorization means the configured backend acknowledged that atomic
  append,
  not that Runtime reread the entire prefix immediately before effect entry.

If the product requires complete prefix reconstructibility immediately before every effect, it
needs stronger storage or an authenticated availability witness. Replaying the same mutable store
is not such a witness.

### 7.3 Resource behavior

The cache stores an `Arc<VerifiedRun>`, so lookup clones only the `Arc`. State advancement may keep
a measured bounded copy initially; eliminating growing-map copies is a separate representation
optimization and a prerequisite only for a stronger asymptotic claim. The cache uses the hard
weighted-LRU, entry-count, and per-tenant limits from Material uncertainty 8. Every cold load and
warm suffix refresh for the same run is singleflighted, so concurrent callers cannot refold the same
suffix independently.

This RFC claims elimination of repeated verification, not a global Theta(N) bound. The enforceable
cost property is:

> Each successfully admitted `(run, head)` suffix batch is folded at most once while the cache
> evidence that admitted it remains resident.

Malformed or unavailable suffixes may be fetched and rejected again, and eviction deliberately
turns a later access into a new cold ingress. The guarantee is about retained successful evidence,
not a permanent negative cache.

Any linear asymptotic claim additionally requires removal or measurement of growing map clones,
eviction behavior, multi-process duplication, and producer-closure work.

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
    projection/index deltas,
    secret-free currentness-requirement metadata
}
```

Every field is derived from the same event, predecessor, and qualified `AppendContext`. Independent
callers cannot supply a batch, successor, and projection that merely happen to be wrapped together.

The seal proves exact record/object projection (including absence of surplus objects), assigned
record/head binding, direct object/value/first-seen/fact-scan index extension, and all stable
retained physical obligations. It carries the exact lease/fence obligations to their named use
boundary as durable requirement metadata only; it does not own the live lease/token or prove a
changing resource current forever. The single affine live token remains inside Runtime's
`PreparedAccess` and moves only into `CommittedAccess`. The append constructor is private and cannot
mix a batch, successor, or index plan from different preparations.

The backend interprets no record-family semantics. It owns only:

- transaction atomicity;
- exact-head CAS and row-count checks;
- store-writer and tenant-fact-frontier currentness;
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

| Store disposition | Successor/cache | Releases a new authorization invoker |
| --- | --- | --- |
| `NewlyCommitted` | Publish successor monotonically; no reload | Only for an authorization append |
| `Found` -> `ExistingSame` | Compare once; do not install candidate | Never |
| `Found` -> `AppendConflict` | Fail closed; install nothing | Never |
| `Found` -> invalid/capacity error | Fail closed; install nothing | Never |
| `StaleHead` | Drop successor; later fold winning suffix | Never |
| `AcknowledgementUnknown` | Install nothing; resolve exact identity | Never |

`NewlyCommitted` is payloadless because the positive value would only echo the sealed candidate the
backend consumed. A found stored attempt remains real database ingress: its retained bytes are
qualified and compared once against the candidate because it may describe a historical append
followed by later heads. Equal bytes establish idempotency only; they do not establish that the
attempt is still the current run head.

Cache publication is monotonic. If the resident entry is the prepared predecessor, publish the
successor. If another caller already installed a verified descendant of that successor, keep the
descendant. A directly committed authorization may release its private permit under that descendant
only when the descendant's reduced state proves the exact authorization remains pending,
unobserved, unsuperseded, and otherwise entry-eligible. Otherwise discard the permit. A remembered
incompatible head is an integrity incident: publish nothing and release no permit. A delayed commit
response can never overwrite a later rollback/fork anchor.

The table describes new authority release, not whether earlier work happened. An authorization
append's non-new outcome invokes zero times. An observation append occurs after an invocation, so
its non-new outcome releases no new invoker and causes no additional invocation.

### 8.3 Durable external access

The MFM-mediated access bracket remains:

```text
PreparedAccess<K>                       // private existential typed invocation
  -> commit ExternalAccessAuthorized
  -> CommittedAccess<K>                  // only from direct NewlyCommitted
  -> consume once into exact adapter
  -> validate external response once
  -> commit ExternalAccessObserved
  -> settle state
```

Runtime never passes the adapter or invoker to the state callback. The affine authorization binds
the exact capability, access kind, request, program occurrence, physical binding, and committed
head. The single affine live lease/fence token moves from `PreparedAccess` into `CommittedAccess`;
the append carries only its durable requirement metadata. When the resource contract promises
adapter-entry freshness, its owner checks the token after commit and immediately before consumption.
Revocation between commit and entry drops the permit and invokes zero times; the committed
authorization record remains valid history.

Only direct `NewlyCommitted` lets Runtime promote and consume the private package. Runtime invokes
without reloading history; publication of the semantic successor follows the independent monotonic
cache rule above.

The post-authorization full reload and field-by-field comparison are deleted. They do not strengthen
the database commit and only reconstruct a value the process just created.

---

## 9. Readers, facts, startup, and configuration

### 9.1 One verified loader

Public read, trace, audit, replay, export, and Effect-attention surfaces use the same shared
`VerifiedRun` loader. They may retain distinct output DTOs and explicit caller-policy checks to
prevent accidental disclosure, but they do not each qualify and reduce the same prefix.

An outbound export that needs raw retained bytes may still load content not resident in
`VerifiedRun`; those bytes cross database/object ingress and are content-checked once before the
export sink emits them. An imported bundle crosses import ingress. Producing a different output
purpose does not cause another semantic replay.

### 9.2 Facts

Fact verification closure fixes selected producer prefixes at exact producer heads through the
publication routes, and the completed response commits the exact selected evidence. Once that
producer prefix has been verified for the exact head, it is reusable. A suffix that introduces a
new authorization or completed selection verifies only that new producer dependency. A cache keyed
by producer run and exact head may share it across consumer runs.

Offline import/export closure remains self-contained: its supplied producer bytes are a new ingress
and cannot be satisfied by an unrelated process cache entry that is absent from the bundle.

The tenant fact frontier and its dense publication sequence remain append-atomic evidence. Their
continuity proves that the prior-run selector did not silently omit an eligible publication; each
route fixes the exact producer record and head. A gap, missing route, wrong producer binding, or
frontier CAS mismatch is therefore an integrity/currentness failure, not a disposable-index miss.
This load-bearing negative-completeness chain is distinct from per-run semantic projections and
Effect-attention routes, which may be rebuilt from qualified history.

The frontier establishes a changing cross-run fact, but it is not a reason to revalidate every
already-retained producer prefix.

### 9.3 Lazy startup

Ordinary store open verifies:

- schema and store identity;
- backend channel and durability profile;
- writer fence/epoch;
- registry and process assembly; and
- ability to execute bounded head/suffix transactions.

It does not scan every run, discard all per-run results, and then replay them again on first use.
`audit_store` is a separate read-only operator action that scans every visible run, configuration
history, load-bearing fact frontier, and Effect-attention table in one fixed repeatable-read
snapshot, with explicit progress and corruption reporting. It derives the complete expected route
set from every exact qualified run head and compares both directions, detecting missing, surplus,
orphaned, duplicate, stale-head, and wrong-binding rows. It does not run as a readiness fallback,
grant invocation authority, or create persistent proof after its result is discarded.

`StructuredStoreMaintenance::rebuild_effect_attention` is the sole repair owner. It rebuilds the
complete derived route set from that fixed snapshot, then publishes under a fence and CAS over every
fixed source head/table generation. A concurrent append/index change loses CAS and restarts from a
new snapshot. Repair never edits journal or fact-frontier evidence. The audit and repair
contracts/tests are distinct from ordinary store opening.

### 9.4 Configuration history follows the same ingress rule

Configuration is not an exception to this RFC merely because its history is smaller. External
configuration files, environment streams, API payloads, and retained configuration rows are bytes
until their owning ingress returns an opaque `ConfigurationValue` or
`VerifiedConfiguration@ConfigurationHead`. A locally constructed typed value already satisfies the
same intrinsic invariants and is not serialized and reparsed for reassurance.

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
CAS and returns the same payloadless `NewlyCommitted | Found(raw) | StaleHead |
AcknowledgementUnknown` protocol used for run appends. Only direct `NewlyCommitted` promotes the
retained successor. `Found(raw)` crosses configuration ingress once and becomes either
`ExistingSame`, a typed conflict, or an invalid/capacity error; no positive backend echo is compared
by a second owner.

Configuration readers and writers share a bounded cache of `VerifiedConfiguration` by store
identity/epoch, exact configuration coordinate, and head. A cold load qualifies and folds the full
predecessor chain once; a warm load reuses the exact head or qualifies only a strict suffix.
Direct commit publication is monotonic and never replaces a resident descendant. Eviction causes a
new cold ingress. Selecting whether a revision is currently active is a changing head/policy
proposition and remains an explicit query; it is not another validation of immutable revision
bytes.

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
  prepared/committed invocation package structurally owns every relation they currently protect.

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
- complete-prefix verification for every purpose read; and
- the eager whole-store semantic-open sweep from ordinary readiness.

Keep:

- strict cold/suffix qualification;
- exact content/object closure for newly entered bytes;
- direct extension of the prepared successor's object/value/first-seen/fact-scan indexes from the
  resolved typed artifacts, rejecting surplus objects and ignored record fields at cold ingress;
- exact-head CAS and atomic append;
- one exact stored-attempt ingress comparison against the already-built candidate;
- ambiguous acknowledgement recovery;
- backend-owned writer/fact-frontier currentness plus resource-owner leases/fences; and
- separate redaction-safe public projections.

### 10.4 Configuration

Replace configuration writer/reader/open-audit verification paths with one configuration ingress,
opaque `VerifiedConfiguration`, private `PreparedConfigurationAppend`, payloadless positive commit,
and one shared bounded cache. Delete local revision revalidation, positive echo comparison,
per-reader prefix verification, and eager configuration scanning from store readiness. Retain
strict external source/row ingress, exact predecessor/content binding, head CAS, idempotency,
ambiguous acknowledgement recovery, and the separate `audit_store` scan.

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
- Describe `VerifiedRun@H`, exact-head/suffix loading, and derived cache authority.
- Apply the same cold-once/suffix-once rule and commit protocol to configuration history.
- Replace mandatory authorization reread with direct positive commit acknowledgement and retained
  successor.
- State in-process regression detection and the unchanged cross-restart rollback limitation.
- Classify derived run projections/attention routes separately from the load-bearing dense fact
  frontier/publication routes, with explicit completeness owners.

### `docs/architecture.md`

- Update the runtime diagram and Program/State/Store responsibility rows.
- Move the complete callback-free program compiler into `mfm-program`; move live process assembly
  and invocation thunks into `mfm-runtime`, deleting superseded `mfm-certify` authority wrappers.
- Define `Pure | Read<C> | Effect<C>` as framework-issued capability classification.
- Remove claims that ordinary Rust closure purity is type-enforced.
- Remove comparison-token and complete-prefix-per-load architecture.
- Show one registry assembly with least-authority views, one run loader, one reducer, one
  configuration loader, and shared purpose projections.

### `docs/run-execution.md`

- Change admission from app certification plus store recertification to trusted `Program`
  construction plus document persistence.
- Describe one resolved transition and locally retained successor.
- Describe cold full verification and warm suffix continuation.
- Describe configuration hot construction, cold ingress, and suffix continuation.
- Make positive append acknowledgement payloadless.
- Preserve authorization-before-invocation and observation-before-settlement.

### Other documentation

Update the READMEs for program, capabilities, certification, store, runtime, app, and replay, plus
`docs/persisted-public-surfaces.md`, including the golden-frozen journal/configuration formats and
fresh store-identity program cutover. Updating `docs/known-gaps.md` is mandatory: it must record
lazy readiness, the separate `audit_store` and derived-attention rebuild actions, in-process
remembered rollback/fork detection, and the unchanged inability to detect a self-consistent
rollback after every outside anchor is lost.

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
- no public MFM adapter invocation entry exists outside the private committed invocation package;
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
- Production suffix folding equals a fresh cold fold at every prefix.
- No intent/recorded dual reducer remains for a test corpus to reconcile.

### 12.3 Cache and concurrency

- Unchanged cached head performs one head snapshot query, zero batch/object loads, and zero reducer
  calls.
- A foreign valid suffix qualifies and folds each new batch once.
- Wrong predecessor, gap, malformed object, regression, and fork fail without replacing the trusted
  entry; the older proof cannot authorize work against the newly claimed backend head.
- Eviction performs a cold load.
- Concurrent cold loads and warm refreshes use one flight per run.
- Cache entries cannot cross store identity, writer epoch, run, tenant, or program-catalog identity.
- A raw/deserialized `JournalHead` cannot construct or install `VerifiedRun`.
- `PreparedSuccessor` cannot be used as current authority before commit.
- A delayed `NewlyCommitted` response never overwrites a resident verified descendant; an
  authorization under that descendant releases its exact permit only when still pending,
  unobserved, unsuperseded, and entry-eligible. Observed/superseded/ineligible and incompatible
  descendants release no permit.
- Capacity tests measure resident weight as well as canonical bytes and object/batch counts.
- Cached totals plus a suffix preserve every capacity limit and the two-successor authorization
  reserve.
- Interior historical loss with unchanged head leaves a warm verified value usable under the chosen
  availability contract, while a fresh cold process rejects the missing history.
- Cache eviction/restart loses the rollback witness and accepts an otherwise valid restored prefix,
  matching the documented limitation.
- A mismatched non-head derived index reports `DerivedIndexInconsistent` and blocks the indexed
  operation until rebuild; it does not relabel valid journal bytes as corrupt. A head that does not
  match its loaded chain remains invalid history.

### 12.4 Append and external access

- `NewlyCommitted` publishes the retained successor monotonically with zero history reloads.
- An authorization `Found` classified as `ExistingSame` after later successors neither rewinds the
  cache nor releases an invoker.
- Authorization-append `StaleHead`, `AcknowledgementUnknown`, invalid found bytes, capacity failure,
  and typed `AppendConflict` invoke zero times; no non-authorization append releases a new invoker.
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
- A permit cannot be paired with another same-`C` request, run, occurrence, process registry,
  adapter implementation, physical binding, or resource-lineage head.
- The special prior-run fact-scan permit remains store-minted, one-use, and direct-new-commit-only.
- A cached run does not bypass a live capability lease/fence check at the declared boundary.
- For a capability promising adapter-entry freshness, revocation after authorization commit but
  before entry causes zero adapter invocations and discards the unconsumed committed permit.
- Observation is committed before state settlement succeeds.
- PostgreSQL snapshot, atomicity, rollback, CAS contention, numeric suffix ordering, malformed-row,
  and fresh-process continuation tests remain.

### 12.5 Facts and purpose projections

- A new suffix fact dependency is verified once.
- A previously verified exact producer head is reused.
- Changed producer heads, cycles, tenant boundaries, dense-publication gaps, missing routes, and
  frontier mismatches fail.
- Authorization retained in the cached prefix plus a completed fact observation in the suffix does
  not rescan older fact barriers.
- Offline export cannot borrow producer closure from the process cache.
- Public/trace/audit/replay/export views share one verified load while preserving output redaction.
- Ordinary readiness performs no history scan; `audit_store` finds dormant run, configuration, and
  fact-frontier corruption without granting invocation authority.
- In one fixed snapshot, `audit_store` reports missing, surplus, orphaned, duplicate, stale-head,
  and wrong-binding Effect-attention rows in both comparison directions.
- The sole maintenance owner rebuilds from exact verified heads without changing journal/fact
  evidence. A rebuild racing a run append or attention-table mutation loses its fence/CAS and
  restarts; it cannot publish a stale route.

### 12.6 Configuration history

- Typed/local configuration construction increments no ingress-validation counter.
- Hostile file/environment/API and database revisions are bounded and admitted exactly once.
- Secret-bearing configuration produces only a non-serializable live handle; secret canaries never
  appear in revision bytes, history, exports, or diagnostics.
- Cold and suffix folds produce the same exact configuration head/value at every revision.
- Same-head warm reads load no historical revision; concurrent refresh is singleflighted; eviction
  performs a new cold ingress.
- `NewlyCommitted` publishes the retained prepared successor monotonically with no echo/reload.
- Found-same, conflict, stale, and unknown acknowledgement behavior preserves idempotency and never
  installs an uncommitted successor.
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

5. **`resolve and seal each append through one event`**

   Introduce `ResolvedEvent`, `PendingAppend`, `AppendContext`, full `RetainedBatchFixation`, and
   structurally inseparable `PreparedAppend { batch, successor, index plan }` for every event family
   in one cutover. Introduce Runtime-private prepared/committed invocation packages and qualified
   history-port outcome handling, payloadless `NewlyCommitted`, and monotonic successor publication.
   In the same commit delete both reducer branches, semantic equality/comparison typestates, local
   requalification, backend semantic echoes, public/feature-gated authorization minting,
   `ExpectedAuthorization` shadows, and Runtime's reload. The old guard is removed only when its
   projection, binding, obligation, and ordering jobs have structural replacements in this commit.

6. **`expose bounded journal head and segment loading`**

   Unify the existing complete-snapshot and exact-prefix backend APIs behind one bounded
   current-head plus segment contract. Its `after = None` arm preserves cold complete loading, and
   its `after = Some(H)` arm returns a suffix. Preserve historical exact-prefix loads and all
   fixation goldens; there is no journal hash/schema version or in-store chain reset.

7. **`cold-verify once and fold verified suffixes`**

   Add the shared bounded/singleflight cache, switch run writer/readers together, publish direct
   successors monotonically, and fail closed on observed regression/fork. Distinguish derived
   projections/attention routes from the dense fact frontier, remove warm complete-prefix replay and
   independent projection authority, introduce lazy readiness plus `audit_store` and the sole
   Effect-attention rebuild owner, and preserve historical exact-head loads for cold ingress,
   export/replay, and producer fact closure.

8. **`make configuration history single-ingress`**

   Introduce opaque configuration values/evidence and private prepared successors, switch writer
   and readers together to cold/suffix qualification, make positive acknowledgement payloadless and
   publication monotonic, and retarget `audit_store` to the new loader. Preserve exact-head
   CAS/idempotency/ambiguity and the revision wire/hash format; delete local revision revalidation,
   echo comparison, purpose-specific replay, and configuration scanning from ordinary readiness.

9. **`drive and project from one verified run`**

   Move tenant checking into the initial verified load, return the resulting head, delete app
   pre/post-drive loads, share fact evidence and all purpose projections, and remove the remaining
   duplicate verification types and tests.

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
- The changed program-document schema starts under a fresh store identity while `RunAdmitted` and
  journal fixation formats remain golden-frozen.
- Every state has exactly one type-level `Pure`, `Read<C>`, or `Effect<C>` declaration.
- MFM gives state callbacks no transport, signer, store, or generic live capability.
- Adapter-owned dependencies remain adapter TCB internals rather than callable nested framework
  capabilities; their identities/lineage remain represented, and Read classifies whole-adapter
  behavior.
- Cached semantic evidence never bypasses live resource currentness, including revocation between
  authorization commit and adapter entry.
- The documentation does not claim arbitrary Rust closures are sandboxed or provably pure.
- Intent and recorded bytes converge before one reducer; no dual semantic implementation remains.
- The current journal hash format is frozen by golden tests and documented as an exact-prefix
  commitment.
- A warm same-head load performs no historical qualification or reduction.
- Each successfully admitted suffix batch is folded at most once while its cache evidence remains
  resident.
- A directly `NewlyCommitted` append publishes its prepared successor monotonically without a
  reload and never overwrites a remembered descendant.
- Only a directly `NewlyCommitted` authorization can release its candidate's invoker.
- No consumer-callable authorization mint exists under the production feature graph, and exact
  same-`C` request/run/occurrence/registry/adapter/binding/lineage substitution is impossible.
- Authorization-append found-same, stale, unknown, invalid, capacity, conflict, regression, and fork
  paths release no invoker; non-authorization failures release no new invoker or cause an additional
  invocation.
- Observation recovery never recreates a consumed permit; Effect entry and refresh ordinals retain
  their separate typed contracts.
- Configuration history has the same local-construction/cold-ingress/prepared-commit discipline and
  no independent writer/reader/readiness replay paths.
- The dense fact frontier remains load-bearing completeness evidence; ordinary projections and
  Effect-attention routes remain derived under one named audit/rebuild owner.
- Purpose readers share verified evidence rather than replaying the same run.
- Intrinsic validators have moved to opaque construction/ingress; changing facts have explicitly
  named authority owners; redundant shadows are deleted.
- Safe-failure values are capability-specific valid types; Effect-only completion variants, keyed
  entry/bounds, refresh evidence, resource lineage, dependency identities, and the reserved fact
  scanner remain represented.
- The final report measures product-code and public-type deltas and itemizes every new concept or
  net addition; no increase is justified merely as scaffolding for a later cleanup.

When these conditions hold, MFM will rely on the properties it already built: Rust construction,
content identity, an exact recursive journal commitment, atomic CAS, and typed access authority.
It will stop paying every internal layer to distrust the preceding one.
